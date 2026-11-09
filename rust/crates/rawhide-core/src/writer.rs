//! Minimal DNG writer for the Phase 2 spike.
//!
//! Scope: one 16-bit `LinearRaw`/`CFA` `SubIFD`, tiled raw pages, a baseline `JPEG`
//! preview strip, classic little-endian TIFF and a single frame. The full
//! writer will add EXIF/GPS, profiles, split markers, multiple frames, Deflate
//! and `BigTIFF`.
//!
//! The IFD builder is deliberately small and explicit; the full writer will
//! replace it with a reusable tag/`IFD` layer informed by `tiff-writer`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use flate2::write::ZlibEncoder;
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;
use thiserror::Error;

use crate::codec::{capacity_bytes_total, format_kb_hi, format_kb_lo};
use crate::error::CodecError;
use crate::framing::PayloadFrame;
use crate::metadata::Metadata;
use crate::profile::{CameraProfile, LevelPolicy};
use crate::split::{SplitIdField, SplitSeqField, validate_fields};
use crate::thumbnail::{PreviewSource, ThumbnailError, rgb as thumbnail_rgb};
use crate::tiff::{Entry, IfdBuilder, TagType};

/// Raw layout for the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RawMode {
    /// Demosaiced RGB: three samples per pixel.
    #[default]
    Linear,
    /// Bayer mosaic: one sample per pixel (RGGB).
    Cfa,
}

impl RawMode {
    /// Samples per pixel.
    #[must_use]
    pub const fn channels(self) -> usize {
        match self {
            Self::Linear => 3,
            Self::Cfa => 1,
        }
    }

    /// DNG photometric interpretation code.
    #[must_use]
    pub const fn photometric(self) -> u16 {
        match self {
            Self::Linear => 34_892,
            Self::Cfa => 32_803,
        }
    }

    /// CLI-facing name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Cfa => "cfa",
        }
    }
}

/// Writer options.
#[derive(Debug, Clone, Copy)]
pub struct WriterOptions<'a> {
    /// Frame width.
    pub width: usize,
    /// Frame height.
    pub height: usize,
    /// Raw layout.
    pub mode: RawMode,
    /// Bits per sample: 8/10/12/14/16.
    pub bit_depth: u16,
    /// LSB planes (`1..=bit_depth`, max 16).
    pub lsb_planes: u8,
    /// Number of full-resolution raw frames (1-8).
    pub frames: u8,
    /// Seed for per-frame synthetic covers.
    pub seed: u64,
    /// Optional XOR keystream key.
    pub key: Option<&'a [u8]>,
    /// Optional camera profile for calibration/metadata tags.
    pub profile: Option<&'a CameraProfile>,
    /// Optional randomized metadata for EXIF/DNG tags.
    pub metadata: Option<&'a Metadata>,
    /// Fidelity level policy for white/black levels.
    pub level_policy: LevelPolicy,
    /// Raw tile compression.
    pub compression: Compression,
    /// IFD0 preview source.
    pub preview: PreviewSource<'a>,
    /// Optional shared chunk-set UUID.
    pub split_id: Option<&'a str>,
    /// Optional chunk sequence (1-based).
    pub split_seq: Option<u32>,
    /// Optional declared chunk total.
    pub split_total: Option<u32>,
    /// Metadata field for the UUID.
    pub split_id_field: SplitIdField,
    /// Metadata field for the sequence.
    pub split_seq_field: SplitSeqField,
    /// Tile width for the raw page.
    pub tile_width: usize,
    /// Tile height for the raw page.
    pub tile_height: usize,
}

impl Default for WriterOptions<'_> {
    fn default() -> Self {
        Self {
            width: 256,
            height: 192,
            mode: RawMode::Linear,
            bit_depth: 16,
            lsb_planes: 1,
            frames: 1,
            seed: 0,
            key: None,
            profile: None,
            metadata: None,
            level_policy: LevelPolicy::Profile,
            compression: Compression::None,
            preview: PreviewSource::Synthetic,
            split_id: None,
            split_seq: None,
            split_total: None,
            split_id_field: SplitIdField::default(),
            split_seq_field: SplitSeqField::default(),
            tile_width: 128,
            tile_height: 96,
        }
    }
}

/// Writer spike errors.
#[derive(Debug, Error)]
pub enum WriteError {
    /// Payload is larger than the frame's LSB capacity.
    #[error(
        "payload {payload} exceeds capacity {capacity} ({width}x{height} {mode} {bit_depth}-bit, {planes} LSB plane(s))",
        payload = format_kb_hi(*payload),
        capacity = format_kb_lo(*capacity)
    )]
    Capacity {
        /// Payload size.
        payload: u64,
        /// Capacity.
        capacity: u64,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// Mode name.
        mode: &'static str,
        /// Bit depth.
        bit_depth: u16,
        /// Planes.
        planes: u8,
    },

    /// Width/height are zero.
    #[error("width and height must be positive (got {width}x{height})")]
    InvalidGeometry {
        /// Width.
        width: usize,
        /// Height.
        height: usize,
    },

    /// Unsupported bit depth.
    #[error("bit_depth must be one of 8, 10, 12, 14, 16 (got {0})")]
    InvalidBitDepth(u16),

    /// Invalid plane count.
    #[error("lsb_planes ({planes}) must be 1..={limit}")]
    InvalidPlanes {
        /// Planes.
        planes: u8,
        /// Allowed maximum.
        limit: u8,
    },

    /// Compression is not supported for this bit depth.
    #[error("{compression} compression requires bit depth 8 or 16 (got {bit_depth})")]
    UnsupportedCompression {
        /// Requested compression.
        compression: &'static str,
        /// Requested depth.
        bit_depth: u16,
    },

    /// The profile crop margin leaves no image.
    #[error("crop_margin leaves no image in {width}x{height}")]
    InvalidCrop {
        /// Frame width.
        width: usize,
        /// Frame height.
        height: usize,
    },

    /// Tile geometry is zero or too large.
    #[error("tile dimensions must be positive and fit the image (got {width}x{height})")]
    InvalidTiles {
        /// Tile width.
        width: usize,
        /// Tile height.
        height: usize,
    },

    /// Frame count is outside 1..=8.
    #[error("frames must be 1-8 (got {0})")]
    InvalidFrames(u8),

    /// Split markers are incomplete or contradictory.
    #[error("invalid split markers: {0}")]
    InvalidSplit(String),

    /// Split-field validation failed.
    #[error(transparent)]
    Split(#[from] crate::split::SplitError),

    /// The file is too large for classic 32-bit TIFF offsets.
    #[error("spike writer only supports files below 4 GiB")]
    TooLarge,

    /// JPEG encoding failed.
    #[error("JPEG preview encoding failed: {0}")]
    Jpeg(String),

    /// I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// LSB embedding failed.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// TIFF serialization failed.
    #[error(transparent)]
    Tiff(#[from] crate::tiff::Error),

    /// Preview preparation failed.
    #[error(transparent)]
    Thumbnail(#[from] ThumbnailError),

    /// Opcode fingerprinting failed.
    #[error(transparent)]
    Opcode(#[from] crate::opcodes::OpcodeError),
}

/// Information about a written DNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteInfo {
    /// Output path.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Payload capacity in bytes.
    pub capacity: u64,
    /// Embedded payload length.
    pub payload_len: usize,
    /// Frame width.
    pub width: usize,
    /// Frame height.
    pub height: usize,
    /// Raw layout.
    pub mode: RawMode,
    /// Bits per sample.
    pub bit_depth: u16,
    /// LSB planes used.
    pub lsb_planes: u8,
    /// Raw frames written.
    pub frames: u8,
}

const TAG_NEW_SUBFILE_TYPE: u16 = 254;
const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_IMAGE_DESCRIPTION: u16 = 270;
const TAG_MAKERNOTE: u16 = 37500;
const TAG_MAKERNOTE_SAFETY: u16 = 50741;
const TAG_XMP: u16 = 700;
const TAG_MAKE: u16 = 271;
const TAG_MODEL: u16 = 272;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_ORIENTATION: u16 = 274;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_X_RESOLUTION: u16 = 282;
const TAG_Y_RESOLUTION: u16 = 283;
const TAG_PLANAR_CONFIGURATION: u16 = 284;
const TAG_PREDICTOR: u16 = 317;
const TAG_RESOLUTION_UNIT: u16 = 296;
const TAG_PAGE_NUMBER: u16 = 297;
const TAG_SOFTWARE: u16 = 305;
const TAG_DATE_TIME: u16 = 306;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_LENGTH: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_BYTE_COUNTS: u16 = 325;
const TAG_SUB_IFDS: u16 = 330;
const TAG_EXIF_IFD: u16 = 34665;
const TAG_GPS_IFD: u16 = 34853;
const TAG_GPS_VERSION: u16 = 0;
const TAG_GPS_LAT_REF: u16 = 1;
const TAG_GPS_LAT: u16 = 2;
const TAG_GPS_LON_REF: u16 = 3;
const TAG_GPS_LON: u16 = 4;
const TAG_GPS_ALT_REF: u16 = 5;
const TAG_GPS_ALT: u16 = 6;
const TAG_GPS_TIME: u16 = 7;
const TAG_GPS_DOP: u16 = 11;
const TAG_GPS_PROCESSING: u16 = 27;
const TAG_GPS_DATE: u16 = 29;
const TAG_EXPOSURE_TIME: u16 = 33434;
const TAG_FNUMBER: u16 = 33437;
const TAG_EXPOSURE_PROGRAM: u16 = 34850;
const TAG_ISO: u16 = 34855;
const TAG_DATETIME_ORIGINAL: u16 = 36867;
const TAG_DATETIME_DIGITIZED: u16 = 36868;
const TAG_OFFSET_TIME: u16 = 36880;
const TAG_SHUTTER_SPEED: u16 = 37377;
const TAG_APERTURE: u16 = 37378;
const TAG_BRIGHTNESS: u16 = 37379;
const TAG_EXPOSURE_BIAS: u16 = 37380;
const TAG_MAX_APERTURE: u16 = 37381;
const TAG_SUBJECT_DISTANCE: u16 = 37382;
const TAG_METERING: u16 = 37383;
const TAG_LIGHT_SOURCE: u16 = 37384;
const TAG_FLASH: u16 = 37385;
const TAG_FOCAL_LENGTH: u16 = 37386;
const TAG_SUBSEC_TIME: u16 = 37520;
const TAG_SUBSEC_TIME_ORIGINAL: u16 = 37521;
const TAG_SUBSEC_TIME_DIGITIZED: u16 = 37522;
const TAG_SUBJECT_DISTANCE_RANGE: u16 = 41996;
const TAG_FOCAL_35MM: u16 = 41989;
const TAG_BODY_SERIAL: u16 = 42033;
const TAG_LENS_SPEC: u16 = 42034;
const TAG_LENS_MAKE: u16 = 42035;
const TAG_LENS_MODEL: u16 = 42036;
const TAG_LENS_SERIAL: u16 = 42037;
const TAG_AS_SHOT_NEUTRAL: u16 = 50728;
const TAG_BASELINE_EXPOSURE: u16 = 50730;
const TAG_NOISE_PROFILE: u16 = 51041;
const TAG_IMAGE_UNIQUE_ID: u16 = 42016;
const TAG_IMAGE_NUMBER: u16 = 37393;
const TAG_CFA_REPEAT_PATTERN_DIM: u16 = 33_421;
const TAG_CFA_PATTERN: u16 = 33_422;
const TAG_REFERENCE_BLACK_WHITE: u16 = 532;
const TAG_YCBCR_SUBSAMPLING: u16 = 530;
const TAG_DNG_VERSION: u16 = 50706;
const TAG_DNG_BACKWARD_VERSION: u16 = 50707;
const TAG_UNIQUE_CAMERA_MODEL: u16 = 50708;
const TAG_CFA_PLANE_COLOR: u16 = 50710;
const TAG_CFA_LAYOUT: u16 = 50711;
const TAG_BLACK_LEVEL_REPEAT_DIM: u16 = 50713;
const TAG_BLACK_LEVEL: u16 = 50714;
const TAG_WHITE_LEVEL: u16 = 50717;
const TAG_DEFAULT_SCALE: u16 = 50718;
const TAG_DEFAULT_CROP_ORIGIN: u16 = 50719;
const TAG_DEFAULT_CROP_SIZE: u16 = 50720;
const TAG_COLOR_MATRIX1: u16 = 50721;
const TAG_COLOR_MATRIX2: u16 = 50722;
const TAG_BASELINE_NOISE: u16 = 50731;
const TAG_BASELINE_SHARPNESS: u16 = 50732;
const TAG_BAYER_GREEN_SPLIT: u16 = 50733;
const TAG_LINEAR_RESPONSE_LIMIT: u16 = 50734;
const TAG_CAMERA_SERIAL: u16 = 50735;
const TAG_ANTI_ALIAS_STRENGTH: u16 = 50738;
const TAG_SHADOW_SCALE: u16 = 50739;
const TAG_CALIBRATION_ILLUMINANT1: u16 = 50778;
const TAG_CALIBRATION_ILLUMINANT2: u16 = 50779;
const TAG_BEST_QUALITY_SCALE: u16 = 50780;
const TAG_RAW_DATA_UNIQUE_ID: u16 = 50781;
const TAG_NEW_RAW_IMAGE_DIGEST: u16 = 51111;
const TAG_OPCODE_LIST2: u16 = 51009;
const TAG_OPCODE_LIST3: u16 = 51022;

/// Python-compatible static XMP packet (12 KiB, space-padded).
fn xmp_packet() -> Vec<u8> {
    const CORE: &[u8] = b"<?xpacket begin=\"\xef\xbb\xbf\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Adobe XMP Core 5.6-c140 79.160451, 2017/05/06-01:08:21\">\n\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
<rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
<xmp:Rating>0</xmp:Rating>\n\
</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n\
<?xpacket end=\"w\"?>\n";
    let mut packet = CORE.to_vec();
    packet.resize(12_288, b' ');
    packet
}

/// Raw tile compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Uncompressed tiles.
    #[default]
    None,
    /// Adobe Deflate with horizontal predictor 2 (8/16-bit only).
    AdobeDeflate,
}

impl Compression {
    /// CLI name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AdobeDeflate => "adobe_deflate",
        }
    }
}

/// Write a DNG containing `payload` with the given options.
///
/// # Errors
///
/// Returns [`WriteError`] for invalid geometry/depth/planes, an oversized
/// payload, JPEG/I/O failures, or a file that would exceed classic TIFF
/// offsets.
pub fn write_dng(
    path: impl AsRef<Path>,
    payload: &[u8],
    options: &WriterOptions<'_>,
) -> Result<WriteInfo, WriteError> {
    let WriterOptions {
        width,
        height,
        mode,
        bit_depth,
        lsb_planes,
        frames,
        seed,
        key,
        profile,
        metadata,
        level_policy,
        compression,
        preview,
        split_id,
        split_seq,
        split_total,
        split_id_field,
        split_seq_field,
        tile_width,
        tile_height,
    } = *options;

    if width == 0 || height == 0 {
        return Err(WriteError::InvalidGeometry { width, height });
    }
    if !matches!(bit_depth, 8 | 10 | 12 | 14 | 16) {
        return Err(WriteError::InvalidBitDepth(bit_depth));
    }
    if compression != Compression::None && !matches!(bit_depth, 8 | 16) {
        return Err(WriteError::UnsupportedCompression {
            compression: compression.as_str(),
            bit_depth,
        });
    }
    let plane_limit = u8::try_from(bit_depth.min(16)).unwrap_or(16);
    if !(1..=plane_limit).contains(&lsb_planes) {
        return Err(WriteError::InvalidPlanes {
            planes: lsb_planes,
            limit: plane_limit,
        });
    }
    if !(1..=8).contains(&frames) {
        return Err(WriteError::InvalidFrames(frames));
    }
    if tile_width == 0 || tile_height == 0 {
        return Err(WriteError::InvalidTiles {
            width: tile_width,
            height: tile_height,
        });
    }
    if mode == RawMode::Cfa
        && let Some((margin_x, margin_y)) = profile
            .and_then(|profile| profile.fidelity.as_ref())
            .and_then(|fidelity| fidelity.crop_margin)
        && (width <= 2 * margin_x || height <= 2 * margin_y)
    {
        return Err(WriteError::InvalidCrop { width, height });
    }

    let has_split = split_id.is_some() || split_seq.is_some();
    if has_split {
        validate_fields(split_id_field, split_seq_field)?;
        if split_id_field.is_active() && split_id.is_none() {
            return Err(WriteError::InvalidSplit(
                "the selected UUID field has no value".to_owned(),
            ));
        }
        if split_seq_field.is_active() && split_seq.is_none() {
            return Err(WriteError::InvalidSplit(
                "the selected sequence field has no value".to_owned(),
            ));
        }
        if matches!(
            split_seq_field,
            SplitSeqField::PageNumber | SplitSeqField::ImageDescription
        ) && split_total.is_none()
        {
            return Err(WriteError::InvalidSplit(
                "PageNumber/ImageDescription sequences need a total".to_owned(),
            ));
        }
    }

    let channels = mode.channels();
    let sample_count = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(WriteError::TooLarge)?;
    let capacity = capacity_bytes_total(
        u64::try_from(sample_count).unwrap_or(u64::MAX),
        lsb_planes,
        u64::from(frames),
    );
    let payload_len = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    if payload_len > capacity {
        return Err(WriteError::Capacity {
            payload: payload_len,
            capacity,
            width,
            height,
            mode: mode.as_str(),
            bit_depth,
            planes: lsb_planes,
        });
    }

    let depth = u8::try_from(bit_depth).unwrap_or(16);
    let fidelity_white = profile
        .and_then(|profile| profile.fidelity.as_ref())
        .and_then(|fidelity| fidelity.white_level);
    let cover_defaults = CameraProfile::default();
    let cover_profile = profile.unwrap_or(&cover_defaults);
    let cover_white =
        fidelity_white.map(|_| cover_profile.effective_white(depth, level_policy.as_str()));
    let peak = (1_u32 << bit_depth) - 1;
    let headroom = cover_white.map_or(0, |white| {
        if white < peak {
            (1_u32 << lsb_planes) - 1
        } else {
            0
        }
    });
    let mut covers: Vec<Vec<u16>> = (0..frames)
        .map(|frame_index| {
            synthetic_cover(
                width,
                height,
                mode,
                bit_depth,
                seed.wrapping_add(u64::from(frame_index)),
                cover_white,
                headroom,
            )
        })
        .collect();
    let frame_bytes = PayloadFrame::pack(payload, key);
    stripe_embed(&mut covers, &frame_bytes, lsb_planes)?;

    let tile_row_bytes = tile_row_bytes(tile_width, channels, bit_depth)?;
    let across = width.div_ceil(tile_width);
    let down = height.div_ceil(tile_height);
    let tile_count = across.checked_mul(down).ok_or(WriteError::TooLarge)?;
    let encoded_frames: Vec<Vec<Vec<u8>>> = covers
        .iter()
        .map(|cover| {
            encode_tiles(
                cover,
                width,
                height,
                mode,
                bit_depth,
                tile_width,
                tile_height,
                across,
                down,
                tile_row_bytes,
                compression,
            )
        })
        .collect::<Result<_, _>>()?;
    let frame_lens: Vec<usize> = encoded_frames
        .iter()
        .map(|tiles| {
            tiles
                .iter()
                .fold(0_usize, |total, tile| total.saturating_add(tile.len()))
        })
        .collect();
    let total_raw_len = frame_lens
        .iter()
        .try_fold(0_usize, |total, len| total.checked_add(*len))
        .ok_or(WriteError::TooLarge)?;

    let (preview_width, preview_height) = preview_size(profile, width, height);
    let preview_rgb = thumbnail_rgb(
        preview,
        u32::try_from(preview_width).unwrap_or(u32::MAX),
        u32::try_from(preview_height).unwrap_or(u32::MAX),
        Some(seed),
    )?;
    let preview = encode_jpeg(&preview_rgb, preview_width, preview_height)?;
    let preview_offset = 8_u32;
    let raw_offset = preview_offset
        .checked_add(u32::try_from(preview.len()).map_err(|_| WriteError::TooLarge)?)
        .ok_or(WriteError::TooLarge)?;
    let ifd0_offset = raw_offset
        .checked_add(u32::try_from(total_raw_len).map_err(|_| WriteError::TooLarge)?)
        .ok_or(WriteError::TooLarge)?;

    let (mut ifd0_split, mut exif_entries) = split_entries(
        split_id,
        split_seq,
        split_total,
        split_id_field,
        split_seq_field,
    )?;
    let (metadata_ifd0, metadata_exif) = metadata_entries(profile, metadata);
    ifd0_split.extend(metadata_ifd0);
    exif_entries.extend(metadata_exif);
    let opcode2 = if mode == RawMode::Cfa {
        profile
            .and_then(|profile| profile.gainmap.as_ref())
            .map(|gainmap| crate::opcodes::build_opcode_list2(width, height, Some(seed), gainmap))
    } else {
        None
    };
    let raw_digests = if frames == 1
        && profile.is_some_and(|profile| {
            profile
                .fidelity
                .as_ref()
                .is_some_and(|fidelity| fidelity.raw_digests)
        }) {
        let profile = profile.expect("checked above");
        let (origin, size) = crop_rect(profile, mode, width, height);
        let digest =
            crate::digests::new_raw_image_digest(&covers[0], width, height, channels, bit_depth);
        let unique = crate::digests::raw_data_unique_id(
            &digest,
            &profile.unique_camera_model,
            [
                (u32::try_from(size.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(size.1).unwrap_or(u32::MAX), 1),
            ],
            [
                (u32::try_from(origin.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(origin.1).unwrap_or(u32::MAX), 1),
            ],
            opcode2.as_deref(),
        )?;
        Some((digest, unique))
    } else {
        None
    };
    if let Some((digest, unique)) = raw_digests {
        ifd0_split.push(Entry::bytes(
            TAG_NEW_RAW_IMAGE_DIGEST,
            TagType::Byte,
            digest.to_vec(),
        ));
        ifd0_split.push(Entry::bytes(
            TAG_RAW_DATA_UNIQUE_ID,
            TagType::Byte,
            unique.to_vec(),
        ));
    }
    let gps_entries = gps_entries(metadata);

    let zero_sub_ifds = vec![0_u32; usize::from(frames)];
    let placeholder_exif = if exif_entries.is_empty() {
        None
    } else {
        Some(0)
    };
    let placeholder_gps = gps_entries.as_ref().map(|_| 0);
    let ifd0_placeholder = preview_entries(
        preview_width,
        preview_height,
        preview.len(),
        preview_offset,
        &zero_sub_ifds,
        placeholder_exif,
        placeholder_gps,
        profile,
        metadata,
        &ifd0_split,
    );
    let ifd0_size = IfdBuilder::from_entries(ifd0_placeholder)
        .build(ifd0_offset)?
        .len();

    let raw_template_offsets = vec![0_u32; tile_count];
    let raw_template_counts = vec![0_u32; tile_count];
    let raw_ifd_template = raw_entries(
        width,
        height,
        mode,
        bit_depth,
        tile_width,
        tile_height,
        &raw_template_offsets,
        &raw_template_counts,
        profile,
        metadata,
        level_policy,
        compression,
        opcode2.as_deref(),
    );
    let raw_ifd_size = IfdBuilder::from_entries(raw_ifd_template).build(0)?.len();

    let raw_ifd_start = ifd0_offset
        .checked_add(u32::try_from(ifd0_size).map_err(|_| WriteError::TooLarge)?)
        .ok_or(WriteError::TooLarge)?;
    let mut raw_ifd_offsets = Vec::with_capacity(usize::from(frames));
    for frame_index in 0..usize::from(frames) {
        let offset = raw_ifd_start
            .checked_add(
                u32::try_from(frame_index.saturating_mul(raw_ifd_size))
                    .map_err(|_| WriteError::TooLarge)?,
            )
            .ok_or(WriteError::TooLarge)?;
        raw_ifd_offsets.push(offset);
    }
    let exif_template_size = if exif_entries.is_empty() {
        0
    } else {
        IfdBuilder::from_entries(exif_entries.clone())
            .build(0)?
            .len()
    };
    let exif_offset = if exif_entries.is_empty() {
        None
    } else {
        let offset = raw_ifd_start
            .checked_add(
                u32::try_from(usize::from(frames).saturating_mul(raw_ifd_size))
                    .map_err(|_| WriteError::TooLarge)?,
            )
            .ok_or(WriteError::TooLarge)?;
        Some(offset)
    };
    let raw_end = raw_ifd_start
        .checked_add(
            u32::try_from(usize::from(frames).saturating_mul(raw_ifd_size))
                .map_err(|_| WriteError::TooLarge)?,
        )
        .ok_or(WriteError::TooLarge)?;
    let gps_offset = if gps_entries.is_some() {
        Some(if let Some(exif_start) = exif_offset {
            exif_start
                .checked_add(u32::try_from(exif_template_size).map_err(|_| WriteError::TooLarge)?)
                .ok_or(WriteError::TooLarge)?
        } else {
            raw_end
        })
    } else {
        None
    };

    let ifd0 = IfdBuilder::from_entries(preview_entries(
        preview_width,
        preview_height,
        preview.len(),
        preview_offset,
        &raw_ifd_offsets,
        exif_offset,
        gps_offset,
        profile,
        metadata,
        &ifd0_split,
    ))
    .build(ifd0_offset)?;

    let mut raw_data = Vec::with_capacity(total_raw_len);
    let mut raw_ifds = Vec::with_capacity(usize::from(frames));
    let mut frame_start = u64::from(raw_offset);
    for (frame_index, tiles) in encoded_frames.iter().enumerate() {
        let mut tile_offsets = Vec::with_capacity(tile_count);
        let mut tile_byte_counts = Vec::with_capacity(tile_count);
        let mut cursor = frame_start;
        for tile in tiles {
            tile_offsets.push(u32::try_from(cursor).map_err(|_| WriteError::TooLarge)?);
            tile_byte_counts.push(u32::try_from(tile.len()).map_err(|_| WriteError::TooLarge)?);
            raw_data.extend_from_slice(tile);
            cursor = cursor
                .checked_add(u64::try_from(tile.len()).unwrap_or(u64::MAX))
                .ok_or(WriteError::TooLarge)?;
        }
        frame_start = cursor;
        let entries = raw_entries(
            width,
            height,
            mode,
            bit_depth,
            tile_width,
            tile_height,
            &tile_offsets,
            &tile_byte_counts,
            profile,
            metadata,
            level_policy,
            compression,
            opcode2.as_deref(),
        );
        raw_ifds.push(IfdBuilder::from_entries(entries).build(raw_ifd_offsets[frame_index])?);
    }

    let exif_ifd = if let Some(offset) = exif_offset {
        Some(IfdBuilder::from_entries(exif_entries).build(offset)?)
    } else {
        None
    };
    let gps_ifd = match (gps_offset, gps_entries) {
        (Some(offset), Some(entries)) => Some(IfdBuilder::from_entries(entries).build(offset)?),
        _ => None,
    };

    let mut file = Vec::with_capacity(
        ifd0_offset as usize
            + ifd0.len()
            + raw_ifds.iter().map(Vec::len).sum::<usize>()
            + exif_ifd.as_ref().map_or(0, Vec::len)
            + gps_ifd.as_ref().map_or(0, Vec::len),
    );
    file.extend_from_slice(b"II*\0");
    file.extend_from_slice(&ifd0_offset.to_le_bytes());
    file.extend_from_slice(&preview);
    file.extend_from_slice(&raw_data);
    file.extend_from_slice(&ifd0);
    for ifd in &raw_ifds {
        file.extend_from_slice(ifd);
    }
    if let Some(ifd) = &exif_ifd {
        file.extend_from_slice(ifd);
    }
    if let Some(ifd) = &gps_ifd {
        file.extend_from_slice(ifd);
    }

    std::fs::write(path.as_ref(), &file)?;
    let size = std::fs::metadata(path.as_ref())?.len();
    Ok(WriteInfo {
        path: path.as_ref().to_path_buf(),
        size,
        capacity,
        payload_len: payload.len(),
        width,
        height,
        mode,
        bit_depth,
        lsb_planes,
        frames,
    })
}

/// Convenience wrapper for the original 16-bit linear spike.
///
/// # Errors
///
/// See [`write_dng`].
pub fn write_spike_dng(
    path: impl AsRef<Path>,
    payload: &[u8],
    width: usize,
    height: usize,
    lsb_planes: u8,
    key: Option<&[u8]>,
) -> Result<WriteInfo, WriteError> {
    write_dng(
        path,
        payload,
        &WriterOptions {
            width,
            height,
            lsb_planes,
            key,
            ..WriterOptions::default()
        },
    )
}

fn stripe_embed(covers: &mut [Vec<u16>], frame: &[u8], lsb_planes: u8) -> Result<(), WriteError> {
    let total_slots = covers.iter().fold(0_u64, |total, cover| {
        total.saturating_add(
            u64::try_from(cover.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(lsb_planes)),
        )
    });
    let total_bits = u64::try_from(frame.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(8);
    if total_bits > total_slots {
        return Err(WriteError::TooLarge);
    }

    let planes = usize::from(lsb_planes);
    let mut global_bit = 0_usize;
    for cover in covers {
        if global_bit >= frame.len().saturating_mul(8) {
            break;
        }
        let slots = cover.len().saturating_mul(planes);
        let take = slots.min(frame.len().saturating_mul(8) - global_bit);
        for local in 0..take {
            let index = global_bit + local;
            let bit = (frame[index / 8] >> (7 - (index % 8))) & 1;
            let sample = local / planes;
            let plane = u8::try_from(local % planes).unwrap_or(0);
            cover[sample] = (cover[sample] & !(1_u16 << plane)) | (u16::from(bit) << plane);
        }
        global_bit += take;
    }
    Ok(())
}

fn split_entries(
    split_id: Option<&str>,
    split_seq: Option<u32>,
    split_total: Option<u32>,
    id_field: SplitIdField,
    seq_field: SplitSeqField,
) -> Result<(Vec<Entry>, Vec<Entry>), WriteError> {
    let mut ifd0 = Vec::new();
    let mut exif = Vec::new();

    if let Some(id) = split_id {
        match id_field {
            SplitIdField::ImageUniqueId => exif.push(Entry::ascii(TAG_IMAGE_UNIQUE_ID, id)),
            SplitIdField::ImageDescription => {
                ifd0.push(Entry::ascii(TAG_IMAGE_DESCRIPTION, id));
            }
            SplitIdField::None => {}
        }
    }

    if let Some(seq) = split_seq {
        match seq_field {
            SplitSeqField::ImageNumber => {
                exif.push(Entry::longs(TAG_IMAGE_NUMBER, &[seq]));
            }
            SplitSeqField::PageNumber => {
                let total = split_total.ok_or_else(|| {
                    WriteError::InvalidSplit("PageNumber needs a total".to_owned())
                })?;
                ifd0.push(Entry::shorts(
                    TAG_PAGE_NUMBER,
                    &[
                        u16::try_from(seq).unwrap_or(u16::MAX),
                        u16::try_from(total).unwrap_or(u16::MAX),
                    ],
                ));
            }
            SplitSeqField::ImageDescription => {
                let total = split_total.ok_or_else(|| {
                    WriteError::InvalidSplit("ImageDescription sequence needs a total".to_owned())
                })?;
                ifd0.push(Entry::ascii(
                    TAG_IMAGE_DESCRIPTION,
                    &format!("{seq:04}/{total:04}"),
                ));
            }
            SplitSeqField::None => {}
        }
    }

    Ok((ifd0, exif))
}

fn metadata_entries(
    profile: Option<&CameraProfile>,
    metadata: Option<&Metadata>,
) -> (Vec<Entry>, Vec<Entry>) {
    let Some(metadata) = metadata else {
        return (Vec::new(), Vec::new());
    };
    let fidelity = profile.and_then(|profile| profile.fidelity.as_ref());
    let allow = fidelity.and_then(|fidelity| fidelity.exif_allow.as_ref());
    let base_enabled = |tag: u16| allow.is_none_or(|list| list.contains(&u32::from(tag)));
    let extra_enabled = |tag: u16| allow.is_some_and(|list| list.contains(&u32::from(tag)));
    let emit_lens = fidelity.is_none_or(|fidelity| fidelity.emit_lens);
    let emit_serials = fidelity.is_none_or(|fidelity| fidelity.emit_serials);

    let mut ifd0 = Vec::new();
    if emit_serials {
        ifd0.push(Entry::ascii(TAG_CAMERA_SERIAL, &metadata.camera_serial));
    }
    let mut exif = Vec::new();

    let fnumber = if metadata.fnumber.1 == 0 {
        0.0
    } else {
        f64::from(metadata.fnumber.0) / f64::from(metadata.fnumber.1)
    };
    let exposure_seconds = if metadata.exposure_time.1 == 0 {
        0.0
    } else {
        f64::from(metadata.exposure_time.0) / f64::from(metadata.exposure_time.1)
    };
    let shutter = if exposure_seconds > 0.0 {
        (-exposure_seconds.log2() * 100.0).round() as i32
    } else {
        0
    };
    let aperture = if fnumber > 0.0 {
        (2.0 * fnumber.log2() * 100.0).round() as u32
    } else {
        0
    };

    macro_rules! push_base {
        ($tag:expr, $entry:expr) => {
            if base_enabled($tag) {
                exif.push($entry);
            }
        };
    }

    push_base!(
        TAG_EXPOSURE_TIME,
        Entry::rationals(TAG_EXPOSURE_TIME, &[metadata.exposure_time])
    );
    push_base!(
        TAG_FNUMBER,
        Entry::rationals(TAG_FNUMBER, &[metadata.fnumber])
    );
    push_base!(
        TAG_EXPOSURE_PROGRAM,
        Entry::shorts(TAG_EXPOSURE_PROGRAM, &[metadata.exposure_program])
    );
    push_base!(
        TAG_ISO,
        Entry::shorts(TAG_ISO, &[u16::try_from(metadata.iso).unwrap_or(u16::MAX)])
    );
    push_base!(
        TAG_DATETIME_ORIGINAL,
        Entry::ascii(TAG_DATETIME_ORIGINAL, &metadata.datetime)
    );
    push_base!(
        TAG_DATETIME_DIGITIZED,
        Entry::ascii(TAG_DATETIME_DIGITIZED, &metadata.datetime)
    );
    push_base!(
        TAG_OFFSET_TIME,
        Entry::ascii(TAG_OFFSET_TIME, &metadata.offset)
    );
    push_base!(
        TAG_SHUTTER_SPEED,
        Entry::srationals(TAG_SHUTTER_SPEED, &[(shutter, 100)])
    );
    push_base!(
        TAG_APERTURE,
        Entry::rationals(TAG_APERTURE, &[(aperture, 100)])
    );
    push_base!(
        TAG_BRIGHTNESS,
        Entry::srationals(
            TAG_BRIGHTNESS,
            &[(
                metadata.brightness.0 as i32,
                i32::try_from(metadata.brightness.1).unwrap_or(100),
            )]
        )
    );
    push_base!(
        TAG_EXPOSURE_BIAS,
        Entry::srationals(
            TAG_EXPOSURE_BIAS,
            &[(
                metadata.exposure_bias.0,
                i32::try_from(metadata.exposure_bias.1).unwrap_or(100),
            )]
        )
    );
    push_base!(
        TAG_MAX_APERTURE,
        Entry::rationals(TAG_MAX_APERTURE, &[metadata.max_aperture])
    );
    if extra_enabled(TAG_SUBJECT_DISTANCE) && metadata.subject_distance.1 != 0 {
        exif.push(Entry::rationals(
            TAG_SUBJECT_DISTANCE,
            &[metadata.subject_distance],
        ));
    }
    push_base!(
        TAG_METERING,
        Entry::shorts(TAG_METERING, &[metadata.metering])
    );
    push_base!(TAG_LIGHT_SOURCE, Entry::shorts(TAG_LIGHT_SOURCE, &[0]));
    push_base!(TAG_FLASH, Entry::shorts(TAG_FLASH, &[metadata.flash]));
    push_base!(
        TAG_FOCAL_LENGTH,
        Entry::rationals(TAG_FOCAL_LENGTH, &[metadata.focal_rational])
    );
    push_base!(
        TAG_SUBSEC_TIME,
        Entry::ascii(TAG_SUBSEC_TIME, &metadata.subsec)
    );
    push_base!(
        TAG_SUBSEC_TIME_ORIGINAL,
        Entry::ascii(TAG_SUBSEC_TIME_ORIGINAL, &metadata.subsec)
    );
    push_base!(
        TAG_SUBSEC_TIME_DIGITIZED,
        Entry::ascii(TAG_SUBSEC_TIME_DIGITIZED, &metadata.subsec)
    );
    push_base!(
        TAG_FOCAL_35MM,
        Entry::shorts(
            TAG_FOCAL_35MM,
            &[u16::try_from(metadata.focal_35mm).unwrap_or(u16::MAX)]
        )
    );
    if emit_serials {
        push_base!(
            TAG_BODY_SERIAL,
            Entry::ascii(TAG_BODY_SERIAL, &metadata.body_serial)
        );
    }
    if emit_lens {
        push_base!(
            TAG_LENS_SPEC,
            Entry::rationals(TAG_LENS_SPEC, &metadata.lens_spec)
        );
        push_base!(
            TAG_LENS_MAKE,
            Entry::ascii(TAG_LENS_MAKE, &metadata.lens_make)
        );
        push_base!(TAG_LENS_MODEL, Entry::ascii(TAG_LENS_MODEL, &metadata.lens));
        if emit_serials {
            push_base!(
                TAG_LENS_SERIAL,
                Entry::ascii(TAG_LENS_SERIAL, &metadata.lens_serial)
            );
        }
    }
    if extra_enabled(TAG_SUBJECT_DISTANCE_RANGE) && metadata.subject_distance_range != 0 {
        exif.push(Entry::shorts(
            TAG_SUBJECT_DISTANCE_RANGE,
            &[metadata.subject_distance_range],
        ));
    }
    if let Some(makernote) = &metadata.makernote {
        if extra_enabled(TAG_MAKERNOTE) {
            exif.push(Entry::bytes(
                TAG_MAKERNOTE,
                TagType::Undefined,
                makernote.clone(),
            ));
        }
        ifd0.push(Entry::shorts(TAG_MAKERNOTE_SAFETY, &[1]));
    }
    (ifd0, exif)
}

fn gps_entries(metadata: Option<&Metadata>) -> Option<Vec<Entry>> {
    let gps = metadata?.gps.as_ref()?;
    Some(vec![
        Entry::bytes(TAG_GPS_VERSION, TagType::Byte, vec![2, 2, 0, 0]),
        Entry::ascii(TAG_GPS_LAT_REF, &gps.lat_ref.to_string()),
        Entry::rationals(TAG_GPS_LAT, &[(gps.lat.0, 1), (gps.lat.1, 1), gps.lat.2]),
        Entry::ascii(TAG_GPS_LON_REF, &gps.lon_ref.to_string()),
        Entry::rationals(TAG_GPS_LON, &[(gps.lon.0, 1), (gps.lon.1, 1), gps.lon.2]),
        Entry::bytes(
            TAG_GPS_ALT_REF,
            TagType::Byte,
            vec![u8::try_from(gps.altitude_ref).unwrap_or(0)],
        ),
        Entry::rationals(TAG_GPS_ALT, &[(gps.altitude_mm, 1000)]),
        Entry::rationals(
            TAG_GPS_TIME,
            &[(gps.time.0, 1), (gps.time.1, 1), (gps.time.2, 1)],
        ),
        Entry::rationals(TAG_GPS_DOP, &[(gps.dop, 1000)]),
        Entry::bytes(
            TAG_GPS_PROCESSING,
            TagType::Undefined,
            b"ASCII\0\0\0fused".to_vec(),
        ),
        Entry::ascii(TAG_GPS_DATE, &gps.date),
    ])
}

fn preview_entries(
    width: usize,
    height: usize,
    jpeg_len: usize,
    jpeg_offset: u32,
    sub_ifd_offsets: &[u32],
    exif_offset: Option<u32>,
    gps_offset: Option<u32>,
    profile: Option<&CameraProfile>,
    metadata: Option<&Metadata>,
    extra: &[Entry],
) -> Vec<Entry> {
    let make = profile.map_or("RawHide", |profile| profile.make.as_str());
    let model = profile.map_or("Rust Spike DNG", |profile| profile.model.as_str());
    let software = metadata
        .map(|metadata| metadata.software.as_str())
        .filter(|software| !software.is_empty())
        .or_else(|| profile.map(|profile| profile.software.as_str()))
        .unwrap_or("RawHide Rust writer");
    let datetime = metadata.map_or("2026:11:08 12:00:00", |metadata| metadata.datetime.as_str());
    let orientation = metadata.map_or(1, |metadata| metadata.orientation);
    let mut entries = vec![
        Entry::longs(TAG_NEW_SUBFILE_TYPE, &[1]),
        Entry::longs(TAG_IMAGE_WIDTH, &[u32::try_from(width).unwrap_or(u32::MAX)]),
        Entry::longs(
            TAG_IMAGE_LENGTH,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_BITS_PER_SAMPLE, &[8, 8, 8]),
        Entry::shorts(TAG_COMPRESSION, &[7]),
        Entry::shorts(TAG_PHOTOMETRIC, &[6]),
        Entry::ascii(TAG_MAKE, make),
        Entry::ascii(TAG_MODEL, model),
        Entry::longs(TAG_STRIP_OFFSETS, &[jpeg_offset]),
        Entry::shorts(TAG_ORIENTATION, &[orientation]),
        Entry::shorts(TAG_SAMPLES_PER_PIXEL, &[3]),
        Entry::longs(
            TAG_ROWS_PER_STRIP,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::longs(
            TAG_STRIP_BYTE_COUNTS,
            &[u32::try_from(jpeg_len).unwrap_or(u32::MAX)],
        ),
        Entry::rationals(TAG_X_RESOLUTION, &[(72, 1)]),
        Entry::rationals(TAG_Y_RESOLUTION, &[(72, 1)]),
        Entry::shorts(TAG_PLANAR_CONFIGURATION, &[1]),
        Entry::shorts(TAG_RESOLUTION_UNIT, &[2]),
        Entry::ascii(TAG_SOFTWARE, software),
        Entry::ascii(TAG_DATE_TIME, datetime),
        Entry::ifds(TAG_SUB_IFDS, sub_ifd_offsets),
        Entry::shorts(TAG_YCBCR_SUBSAMPLING, &[2, 2]),
        Entry::rationals(
            TAG_REFERENCE_BLACK_WHITE,
            &[(0, 1), (255, 1), (128, 1), (255, 1), (128, 1), (255, 1)],
        ),
    ];
    if let Some(profile) = profile {
        entries.extend(profile_ifd0_entries(profile));
    }
    if let Some(metadata) = metadata {
        entries.push(Entry::rationals(
            TAG_AS_SHOT_NEUTRAL,
            &[
                (
                    u32::try_from(metadata.as_shot_neutral[0].0).unwrap_or(0),
                    metadata.as_shot_neutral[0].1,
                ),
                (
                    u32::try_from(metadata.as_shot_neutral[1].0).unwrap_or(0),
                    metadata.as_shot_neutral[1].1,
                ),
                (
                    u32::try_from(metadata.as_shot_neutral[2].0).unwrap_or(0),
                    metadata.as_shot_neutral[2].1,
                ),
            ],
        ));
        entries.push(Entry::srationals(
            TAG_BASELINE_EXPOSURE,
            &[(
                metadata.baseline_exposure.0,
                i32::try_from(metadata.baseline_exposure.1).unwrap_or(10_000),
            )],
        ));
    }
    let emit_xmp = profile
        .and_then(|profile| profile.fidelity.as_ref())
        .is_none_or(|fidelity| fidelity.emit_xmp);
    if emit_xmp {
        entries.push(Entry::bytes(TAG_XMP, TagType::Byte, xmp_packet()));
    }
    if let Some(offset) = exif_offset {
        entries.push(Entry::longs(TAG_EXIF_IFD, &[offset]));
    }
    if let Some(offset) = gps_offset {
        entries.push(Entry::longs(TAG_GPS_IFD, &[offset]));
    }
    entries.extend(extra.iter().cloned());
    entries
}

fn profile_ifd0_entries(profile: &CameraProfile) -> Vec<Entry> {
    let matrix1 = profile
        .color_matrix1
        .iter()
        .map(|&(numerator, denominator)| {
            (numerator, i32::try_from(denominator).unwrap_or(i32::MAX))
        })
        .collect::<Vec<_>>();
    let matrix2 = profile
        .color_matrix2
        .iter()
        .map(|&(numerator, denominator)| {
            (numerator, i32::try_from(denominator).unwrap_or(i32::MAX))
        })
        .collect::<Vec<_>>();
    vec![
        Entry::bytes(TAG_DNG_VERSION, TagType::Byte, profile.dng_version.to_vec()),
        Entry::bytes(
            TAG_DNG_BACKWARD_VERSION,
            TagType::Byte,
            profile.dng_backward.to_vec(),
        ),
        Entry::ascii(TAG_UNIQUE_CAMERA_MODEL, &profile.unique_camera_model),
        Entry::srationals(TAG_COLOR_MATRIX1, &matrix1),
        Entry::srationals(TAG_COLOR_MATRIX2, &matrix2),
        Entry::shorts(TAG_CALIBRATION_ILLUMINANT1, &[profile.calib_illuminant1]),
        Entry::shorts(TAG_CALIBRATION_ILLUMINANT2, &[profile.calib_illuminant2]),
        Entry::rationals(TAG_BASELINE_NOISE, &[profile.baseline_noise]),
        Entry::rationals(TAG_BASELINE_SHARPNESS, &[profile.baseline_sharpness]),
        Entry::rationals(TAG_LINEAR_RESPONSE_LIMIT, &[profile.linear_response_limit]),
        Entry::rationals(TAG_SHADOW_SCALE, &[profile.shadow_scale]),
    ]
}

#[allow(clippy::too_many_arguments)]
fn raw_entries(
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    tile_width: usize,
    tile_height: usize,
    tile_offsets: &[u32],
    tile_byte_counts: &[u32],
    profile: Option<&CameraProfile>,
    metadata: Option<&Metadata>,
    level_policy: LevelPolicy,
    compression: Compression,
    opcode2: Option<&[u8]>,
) -> Vec<Entry> {
    let channels = mode.channels();
    let defaults = CameraProfile::default();
    let profile = profile.unwrap_or(&defaults);
    let depth = u8::try_from(bit_depth).unwrap_or(16);
    let mask = profile.effective_white(depth, level_policy.as_str());
    let black_values = profile.effective_black_levels(mode.as_str(), depth, level_policy.as_str());
    let cfa_pattern = cfa_pattern_bytes(profile.cfa_pattern.as_deref());
    let (crop_origin, crop_size) = crop_rect(profile, mode, width, height);
    let best_quality = if profile.fidelity.is_some() {
        (1, 1)
    } else {
        (100, 100)
    };
    let mut entries = vec![
        Entry::longs(TAG_NEW_SUBFILE_TYPE, &[0]),
        Entry::longs(TAG_IMAGE_WIDTH, &[u32::try_from(width).unwrap_or(u32::MAX)]),
        Entry::longs(
            TAG_IMAGE_LENGTH,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_BITS_PER_SAMPLE, &vec![bit_depth; channels]),
        Entry::shorts(
            TAG_COMPRESSION,
            &[if compression == Compression::None {
                1
            } else {
                8
            }],
        ),
        Entry::shorts(TAG_PHOTOMETRIC, &[mode.photometric()]),
        Entry::shorts(
            TAG_SAMPLES_PER_PIXEL,
            &[u16::try_from(channels).unwrap_or(u16::MAX)],
        ),
        Entry::shorts(TAG_PLANAR_CONFIGURATION, &[1]),
        Entry::longs(
            TAG_TILE_WIDTH,
            &[u32::try_from(tile_width).unwrap_or(u32::MAX)],
        ),
        Entry::longs(
            TAG_TILE_LENGTH,
            &[u32::try_from(tile_height).unwrap_or(u32::MAX)],
        ),
        Entry::longs(TAG_TILE_OFFSETS, tile_offsets),
        Entry::longs(TAG_TILE_BYTE_COUNTS, tile_byte_counts),
        Entry::longs(TAG_BLACK_LEVEL, &black_values),
        Entry::longs(TAG_WHITE_LEVEL, &vec![mask; channels]),
        Entry::rationals(TAG_DEFAULT_SCALE, &[(1, 1), (1, 1)]),
        Entry::rationals(
            TAG_DEFAULT_CROP_ORIGIN,
            &[
                (u32::try_from(crop_origin.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(crop_origin.1).unwrap_or(u32::MAX), 1),
            ],
        ),
        Entry::rationals(
            TAG_DEFAULT_CROP_SIZE,
            &[
                (u32::try_from(crop_size.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(crop_size.1).unwrap_or(u32::MAX), 1),
            ],
        ),
        Entry::rationals(TAG_ANTI_ALIAS_STRENGTH, &[(100, 100)]),
        Entry::rationals(TAG_BEST_QUALITY_SCALE, &[best_quality]),
    ];
    if let Some(opcode2) = opcode2 {
        entries.push(Entry::bytes(
            TAG_OPCODE_LIST2,
            TagType::Undefined,
            opcode2.to_vec(),
        ));
    } else {
        entries.push(Entry::bytes(
            TAG_OPCODE_LIST3,
            TagType::Undefined,
            vec![0, 0, 0, 0],
        ));
    }
    if matches!(compression, Compression::AdobeDeflate) {
        entries.push(Entry::shorts(TAG_PREDICTOR, &[2]));
    }
    if black_values.len() == 4 {
        entries.push(Entry::shorts(TAG_BLACK_LEVEL_REPEAT_DIM, &[2, 2]));
    }
    if let Some(green) = profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.bayer_green_split)
    {
        entries.push(Entry::longs(TAG_BAYER_GREEN_SPLIT, &[u32::from(green)]));
    }
    if let Some(metadata) = metadata
        && let Some(noise) = metadata.noise_profile
    {
        entries.push(Entry::doubles(TAG_NOISE_PROFILE, &noise));
    }
    if mode == RawMode::Cfa {
        entries.push(Entry::shorts(TAG_CFA_REPEAT_PATTERN_DIM, &[2, 2]));
        entries.push(Entry::bytes(TAG_CFA_PATTERN, TagType::Byte, cfa_pattern));
        entries.push(Entry::shorts(TAG_CFA_PLANE_COLOR, &[0, 1, 2]));
        entries.push(Entry::shorts(TAG_CFA_LAYOUT, &[1]));
    }
    entries
}

fn crop_rect(
    profile: &CameraProfile,
    mode: RawMode,
    width: usize,
    height: usize,
) -> ((usize, usize), (usize, usize)) {
    if mode == RawMode::Cfa {
        profile
            .fidelity
            .as_ref()
            .and_then(|fidelity| fidelity.crop_margin)
            .map_or(((0, 0), (width, height)), |(x, y)| {
                ((x, y), (width - 2 * x, height - 2 * y))
            })
    } else {
        ((0, 0), (width, height))
    }
}

fn cfa_pattern_bytes(pattern: Option<&str>) -> Vec<u8> {
    let color = |channel: u8| match channel {
        b'R' => 0,
        b'B' => 2,
        _ => 1,
    };
    match pattern {
        Some(pattern) if pattern.len() == 4 => pattern.bytes().map(color).collect(),
        _ => vec![0, 1, 1, 2],
    }
}

fn tile_row_bytes(width: usize, channels: usize, bit_depth: u16) -> Result<usize, WriteError> {
    let samples = width.checked_mul(channels).ok_or(WriteError::TooLarge)?;
    match bit_depth {
        8 => Ok(samples),
        16 => samples.checked_mul(2).ok_or(WriteError::TooLarge),
        10 | 12 | 14 => samples
            .checked_mul(usize::from(bit_depth))
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or(WriteError::TooLarge),
        other => Err(WriteError::InvalidBitDepth(other)),
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_tiles(
    cover: &[u16],
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    tile_width: usize,
    tile_height: usize,
    across: usize,
    down: usize,
    row_bytes: usize,
    compression: Compression,
) -> Result<Vec<Vec<u8>>, WriteError> {
    let channels = mode.channels();
    let mut out = Vec::with_capacity(across * down);
    for tile_y in 0..down {
        for tile_x in 0..across {
            let mut tile_data = Vec::with_capacity(row_bytes * tile_height);
            for row in 0..tile_height {
                let global_y = tile_y * tile_height + row;
                let mut row_data = vec![0_u8; row_bytes];
                let mut bit_position = 0_usize;
                for local_x in 0..tile_width {
                    let global_x = tile_x * tile_width + local_x;
                    for channel in 0..channels {
                        let value = if global_y < height && global_x < width {
                            cover[(global_y * width + global_x) * channels + channel]
                        } else {
                            0
                        };
                        match bit_depth {
                            8 => {
                                row_data[bit_position / 8] = value as u8;
                                bit_position += 8;
                            }
                            16 => {
                                let byte = bit_position / 8;
                                row_data[byte..byte + 2].copy_from_slice(&value.to_le_bytes());
                                bit_position += 16;
                            }
                            10 | 12 | 14 => {
                                pack_bits(&mut row_data, bit_position, value, bit_depth);
                                bit_position += usize::from(bit_depth);
                            }
                            _ => unreachable!("bit depth validated by caller"),
                        }
                    }
                }
                if matches!(compression, Compression::AdobeDeflate) {
                    apply_predictor(&mut row_data, bit_depth, channels);
                }
                tile_data.extend_from_slice(&row_data);
            }
            if matches!(compression, Compression::AdobeDeflate) {
                let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&tile_data)?;
                out.push(encoder.finish()?);
            } else {
                out.push(tile_data);
            }
        }
    }
    Ok(out)
}

fn apply_predictor(row: &mut [u8], bit_depth: u16, channels: usize) {
    if bit_depth == 8 {
        for index in (channels..row.len()).rev() {
            row[index] = row[index].wrapping_sub(row[index - channels]);
        }
    } else if bit_depth == 16 {
        let samples = row.len() / 2;
        for index in (channels..samples).rev() {
            let current = u16::from_le_bytes([row[index * 2], row[index * 2 + 1]]);
            let previous =
                u16::from_le_bytes([row[(index - channels) * 2], row[(index - channels) * 2 + 1]]);
            row[index * 2..index * 2 + 2]
                .copy_from_slice(&current.wrapping_sub(previous).to_le_bytes());
        }
    }
}

fn pack_bits(target: &mut [u8], bit_offset: usize, value: u16, bit_depth: u16) {
    for bit in 0..usize::from(bit_depth) {
        let index = bit_offset + bit;
        let source_bit = (value >> (bit_depth - 1 - bit as u16)) & 1;
        target[index / 8] |= u8::try_from(source_bit).unwrap_or(0) << (7 - (index % 8));
    }
}

#[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
fn synthetic_cover(
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    seed: u64,
    white_level: Option<u32>,
    headroom: u32,
) -> Vec<u16> {
    let channels = mode.channels();
    let mask = (1_u32 << bit_depth) - 1;
    let limit = mask
        .min(white_level.unwrap_or(mask))
        .saturating_sub(headroom)
        .max(1);
    let scale = if white_level.is_some() {
        limit as f32 / 12_500.0
    } else {
        limit as f32 / 65_535.0
    };
    let width_den = width.saturating_sub(1).max(1) as f32;
    let height_den = height.saturating_sub(1).max(1) as f32;
    let phase = (seed % 6_283) as f32 * 0.001;
    let gains = [1.0_f32, 0.94, 0.82];
    let mut cover = vec![0_u16; width * height * channels];
    for y in 0..height {
        for x in 0..width {
            let base = 7000.0
                + 2600.0 * (y as f32 / height_den)
                + 1400.0 * (x as f32 / width_den)
                + 500.0
                    * (std::f32::consts::TAU * x as f32 / width.max(1) as f32 * 3.0 + phase).sin();
            for channel in 0..channels {
                let gain = if mode == RawMode::Cfa {
                    gains[match (y % 2, x % 2) {
                        (0, 0) => 0,
                        (0, 1) | (1, 0) => 1,
                        _ => 2,
                    }]
                } else {
                    gains[channel]
                };
                let value = (base * gain * scale).clamp(0.0, limit as f32) as u16;
                cover[(y * width + x) * channels + channel] = value;
            }
        }
    }
    cover
}

fn preview_size(profile: Option<&CameraProfile>, width: usize, height: usize) -> (usize, usize) {
    profile
        .and_then(|profile| profile.fidelity.as_ref())
        .and_then(|fidelity| fidelity.preview_size)
        .unwrap_or_else(|| (width.min(4000), height.min(3000)))
}

fn encode_jpeg(rgb: &[u8], width: usize, height: usize) -> Result<Vec<u8>, WriteError> {
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 75);
    encoder
        .encode(
            rgb,
            u32::try_from(width).unwrap_or(u32::MAX),
            u32::try_from(height).unwrap_or(u32::MAX),
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| WriteError::Jpeg(error.to_string()))?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::apply_predictor;

    #[test]
    fn predictor_uses_the_component_stride() {
        let mut eight = vec![10, 20, 30, 40, 50, 60, 70, 80, 90];
        apply_predictor(&mut eight, 8, 3);
        assert_eq!(eight, vec![10, 20, 30, 30, 30, 30, 30, 30, 30]);

        let mut sixteen: Vec<u8> = [10_u16, 20, 30, 40, 50, 60]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        apply_predictor(&mut sixteen, 16, 3);
        let expected: Vec<u8> = [10_u16, 20, 30, 30, 30, 30]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(sixteen, expected);
    }
}
