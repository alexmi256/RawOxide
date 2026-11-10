//! DNG writer: tiled `LinearRaw`/`CFA` raws, JPEG preview, multiple frames,
//! EXIF/GPS/profile metadata, split markers, Deflate and classic/`BigTIFF`
//! output.

use std::io::{BufWriter, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;
use thiserror::Error;

use crate::codec::{capacity_bytes_total, format_kb_hi, format_kb_lo};
use crate::error::CodecError;
use crate::metadata::Metadata;
use crate::profile::{CameraProfile, LevelPolicy};
use crate::split::{SplitIdField, SplitSeqField, validate_fields, validate_split_id};
use crate::thumbnail::{PreviewSource, ThumbnailError, rgb_with_label as thumbnail_rgb_with_label};
use crate::tiff::{Entry, IfdBuilder, TagType};

mod cover;
mod encode;
mod entries;
mod payload;

use self::cover::CoverGenerator;
use self::encode::{embed_band, encode_tile_band, tile_row_bytes, update_frame_crc};
use self::entries::{
    TAG_NEW_RAW_IMAGE_DIGEST, TAG_RAW_DATA_UNIQUE_ID, build_ifd, crop_rect, gps_entries,
    metadata_entries, preview_entries, raw_entries, select_bigtiff, split_entries,
};
use self::payload::{FileSource, PayloadBitReader, PayloadSource, SliceSource};

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
    /// Force `BigTIFF` even for small files (tests/diagnostics).
    pub force_bigtiff: bool,
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
    /// Collect sidecar statistics (per-frame and framed-payload CRCs).
    pub debug_stats: bool,
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
            force_bigtiff: false,
            split_id: None,
            split_seq: None,
            split_total: None,
            split_id_field: SplitIdField::default(),
            split_seq_field: SplitSeqField::default(),
            tile_width: 128,
            tile_height: 96,
            debug_stats: false,
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

    /// Payload bytes changed while the file was being written.
    #[error("payload changed during write (CRC mismatch); retry once the file is stable")]
    PayloadChanged,

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
    /// Whether the output is `BigTIFF`.
    pub bigtiff: bool,
    /// Resolved preview source label.
    pub thumbnail_label: String,
    /// Preview width in pixels.
    pub thumbnail_width: usize,
    /// Preview height in pixels.
    pub thumbnail_height: usize,
    /// CRC32 of the prepared RGB8 preview pixels.
    pub thumbnail_crc32: u32,
    /// CRC32 of each embedded raw frame (after LSB embedding), when debug
    /// statistics were requested.
    pub frame_crc32s: Vec<u32>,
    /// CRC32 of the framed payload (header + body, masked when keyed), when
    /// debug statistics were requested.
    pub payload_frame_crc32: Option<u32>,
    /// Effective white level used for the cover, when a fidelity profile
    /// declares one.
    pub white_level: Option<u32>,
    /// Cover headroom reserved above the payload bits.
    pub headroom: u32,
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

struct TempGuard<'a> {
    path: &'a Path,
    armed: bool,
}

impl Drop for TempGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(self.path);
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
    let mut source = SliceSource { data: payload };
    write_dng_with(path, &mut source, options, None)
}

/// Write a DNG from a byte range of a payload file.
///
/// The payload is streamed in small chunks and is never materialized in
/// memory, which keeps peak memory independent of the payload size.
///
/// # Errors
///
/// See [`write_dng`], plus I/O errors when the file or range cannot be read.
pub fn write_dng_from_file(
    path: impl AsRef<Path>,
    payload_path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    options: &WriterOptions<'_>,
) -> Result<WriteInfo, WriteError> {
    let mut source = FileSource::open(payload_path, offset, len)?;
    write_dng_with(path, &mut source, options, None)
}

/// Write a DNG from a payload file while reporting progress.
///
/// `progress(done_rows, total_rows)` is called before the first row and after
/// each tile row has been generated, embedded and written.
///
/// # Errors
///
/// See [`write_dng_from_file`].
pub fn write_dng_from_file_with_progress<F>(
    path: impl AsRef<Path>,
    payload_path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    options: &WriterOptions<'_>,
    mut progress: F,
) -> Result<WriteInfo, WriteError>
where
    F: FnMut(u64, u64),
{
    let mut source = FileSource::open(payload_path, offset, len)?;
    write_dng_with(
        path,
        &mut source,
        options,
        Some(&mut progress as &mut dyn FnMut(u64, u64)),
    )
}

fn write_dng_with(
    path: impl AsRef<Path>,
    source: &mut dyn PayloadSource,
    options: &WriterOptions<'_>,
    mut progress: Option<&mut dyn FnMut(u64, u64)>,
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
        force_bigtiff,
        split_id,
        split_seq,
        split_total,
        split_id_field,
        split_seq_field,
        tile_width,
        tile_height,
        debug_stats,
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
    if let Some(id) = split_id {
        validate_split_id(id)?;
    }
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
    let payload_len = source.len();
    let payload_len_usize = usize::try_from(payload_len).map_err(|_| WriteError::TooLarge)?;
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
    let (preview_width, preview_height) = preview_size(profile, width, height);
    let thumbnail = thumbnail_rgb_with_label(
        preview,
        u32::try_from(preview_width).unwrap_or(u32::MAX),
        u32::try_from(preview_height).unwrap_or(u32::MAX),
        Some(seed),
    )?;
    let thumbnail_crc32 = crc32fast::hash(&thumbnail.pixels);
    let preview_rgb = thumbnail.pixels;
    let preview = encode_jpeg(&preview_rgb, preview_width, preview_height)?;
    let bytes_per_sample = if bit_depth <= 8 { 1_u64 } else { 2 };
    let uncompressed_bytes = u64::try_from(sample_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(frames))
        .saturating_mul(bytes_per_sample);
    let big = select_bigtiff(
        uncompressed_bytes,
        u64::try_from(preview.len()).unwrap_or(u64::MAX),
        force_bigtiff,
    );
    let preview_offset = if big { 16_u64 } else { 8_u64 };
    let mut raw_cursor = preview_offset
        .checked_add(u64::try_from(preview.len()).map_err(|_| WriteError::TooLarge)?)
        .ok_or(WriteError::TooLarge)?;

    let mut frame = PayloadBitReader::new(source, key, debug_stats)?;
    let tile_row_bytes = tile_row_bytes(tile_width, channels, bit_depth)?;
    let across = width.div_ceil(tile_width);
    let down = height.div_ceil(tile_height);
    let tile_count = across.checked_mul(down).ok_or(WriteError::TooLarge)?;
    let opcode2 = if mode == RawMode::Cfa {
        profile
            .and_then(|profile| profile.gainmap.as_ref())
            .map(|gainmap| crate::opcodes::build_opcode_list2(width, height, Some(seed), gainmap))
    } else {
        None
    };
    let digest_enabled = frames == 1
        && profile.is_some_and(|profile| {
            profile
                .fidelity
                .as_ref()
                .is_some_and(|fidelity| fidelity.raw_digests)
        });
    let mut raw_digests = None;
    let mut frame_tiles: Vec<(Vec<u64>, Vec<u64>)> = Vec::with_capacity(usize::from(frames));
    let mut frame_crc32s = Vec::with_capacity(usize::from(frames));

    let output_path = path.as_ref();
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output.dng");
    let temp_path = output_path.with_file_name(format!(".{file_name}.rawhide.tmp"));
    let mut guard = TempGuard {
        path: &temp_path,
        armed: true,
    };
    let mut file = BufWriter::new(std::fs::File::create(&temp_path)?);
    if big {
        file.write_all(b"II")?;
        file.write_all(&43_u16.to_le_bytes())?;
        file.write_all(&8_u16.to_le_bytes())?;
        file.write_all(&0_u16.to_le_bytes())?;
        file.write_all(&0_u64.to_le_bytes())?;
    } else {
        file.write_all(b"II*\0")?;
        file.write_all(&0_u32.to_le_bytes())?;
    }
    file.write_all(&preview)?;

    let total_rows = u64::from(frames).saturating_mul(u64::try_from(height).unwrap_or(u64::MAX));
    let mut rows_done = 0_u64;
    if let Some(callback) = progress.as_deref_mut() {
        callback(0, total_rows);
    }

    for frame_index in 0..usize::from(frames) {
        let frame_seed = seed.wrapping_add(u64::try_from(frame_index).unwrap_or(u64::MAX));
        let mut tile_offsets = Vec::with_capacity(tile_count);
        let mut tile_byte_counts = Vec::with_capacity(tile_count);
        let mut digest_hasher = if digest_enabled && frame_index == 0 {
            Some(crate::digests::RawImageDigest::new(
                width, height, channels, bit_depth,
            ))
        } else {
            None
        };
        let cover = CoverGenerator::new(
            width,
            height,
            mode,
            bit_depth,
            frame_seed,
            cover_white,
            headroom,
        );
        let mut band = Vec::new();
        let mut frame_crc = debug_stats.then(crc32fast::Hasher::new);
        for tile_y in 0..down {
            let y_start = tile_y * tile_height;
            let rows = (height - y_start).min(tile_height);
            band.resize(width * rows * channels, 0);
            cover.fill_band(y_start, rows, &mut band);
            embed_band(
                &mut band,
                width,
                height,
                y_start,
                rows,
                channels,
                lsb_planes,
                &mut frame,
                u64::try_from(frame_index).unwrap_or(u64::MAX),
            )?;
            if let Some(hasher) = &mut frame_crc {
                update_frame_crc(hasher, &band, bit_depth);
            }
            if let Some(hasher) = &mut digest_hasher {
                hasher.update_band(y_start, rows, &band);
            }
            for tile_x in 0..across {
                let tile = encode_tile_band(
                    &band,
                    width,
                    rows,
                    channels,
                    bit_depth,
                    tile_x,
                    tile_width,
                    tile_height,
                    tile_row_bytes,
                    compression,
                )?;
                tile_offsets.push(raw_cursor);
                tile_byte_counts.push(u64::try_from(tile.len()).unwrap_or(u64::MAX));
                file.write_all(&tile)?;
                raw_cursor = raw_cursor
                    .checked_add(u64::try_from(tile.len()).unwrap_or(u64::MAX))
                    .ok_or(WriteError::TooLarge)?;
            }
            rows_done = rows_done.saturating_add(u64::try_from(rows).unwrap_or(u64::MAX));
            if let Some(callback) = progress.as_deref_mut() {
                callback(rows_done, total_rows);
            }
        }
        if let Some(hasher) = digest_hasher {
            let profile = profile.expect("digest selection requires a profile");
            let (origin, size) = crop_rect(profile, mode, width, height);
            let digest = hasher.finish();
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
                if opcode2.is_some() {
                    [None, opcode2.as_deref(), None]
                } else {
                    [None, None, Some(profile.opcode_list3.as_slice())]
                },
            )?;
            raw_digests = Some((digest, unique));
        }
        frame_tiles.push((tile_offsets, tile_byte_counts));
        if let Some(hasher) = frame_crc {
            frame_crc32s.push(hasher.finalize());
        }
    }
    let payload_frame_crc32 = frame.frame_crc32();
    frame.finish()?;
    let ifd0_offset = raw_cursor;

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

    let placeholder_exif: Option<u64> = if exif_entries.is_empty() {
        None
    } else {
        Some(0)
    };
    let placeholder_gps: Option<u64> = gps_entries.as_ref().map(|_| 0);
    let zero_sub_ifds = vec![0_u64; usize::from(frames)];
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
        big,
    );
    let ifd0_size = build_ifd(
        &IfdBuilder::from_entries(ifd0_placeholder),
        ifd0_offset,
        big,
    )?
    .len();

    let raw_template_offsets = vec![0_u64; tile_count];
    let raw_template_counts = vec![0_u64; tile_count];
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
        big,
    );
    let raw_ifd_size = build_ifd(&IfdBuilder::from_entries(raw_ifd_template), 0, big)?.len();

    let raw_ifd_start = ifd0_offset
        .checked_add(u64::try_from(ifd0_size).map_err(|_| WriteError::TooLarge)?)
        .ok_or(WriteError::TooLarge)?;
    let mut raw_ifd_offsets = Vec::with_capacity(usize::from(frames));
    for frame_index in 0..usize::from(frames) {
        let offset = raw_ifd_start
            .checked_add(
                u64::try_from(frame_index.saturating_mul(raw_ifd_size))
                    .map_err(|_| WriteError::TooLarge)?,
            )
            .ok_or(WriteError::TooLarge)?;
        raw_ifd_offsets.push(offset);
    }
    let exif_template_size = if exif_entries.is_empty() {
        0
    } else {
        build_ifd(&IfdBuilder::from_entries(exif_entries.clone()), 0, big)?.len()
    };
    let exif_offset = if exif_entries.is_empty() {
        None
    } else {
        let offset = raw_ifd_start
            .checked_add(
                u64::try_from(usize::from(frames).saturating_mul(raw_ifd_size))
                    .map_err(|_| WriteError::TooLarge)?,
            )
            .ok_or(WriteError::TooLarge)?;
        Some(offset)
    };
    let raw_end = raw_ifd_start
        .checked_add(
            u64::try_from(usize::from(frames).saturating_mul(raw_ifd_size))
                .map_err(|_| WriteError::TooLarge)?,
        )
        .ok_or(WriteError::TooLarge)?;
    let gps_offset = if gps_entries.is_some() {
        Some(if let Some(exif_start) = exif_offset {
            exif_start
                .checked_add(u64::try_from(exif_template_size).map_err(|_| WriteError::TooLarge)?)
                .ok_or(WriteError::TooLarge)?
        } else {
            raw_end
        })
    } else {
        None
    };

    let ifd0 = build_ifd(
        &IfdBuilder::from_entries(preview_entries(
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
            big,
        )),
        ifd0_offset,
        big,
    )?;

    let mut raw_ifds = Vec::with_capacity(usize::from(frames));
    for (frame_index, (tile_offsets, tile_byte_counts)) in frame_tiles.iter().enumerate() {
        let entries = raw_entries(
            width,
            height,
            mode,
            bit_depth,
            tile_width,
            tile_height,
            tile_offsets,
            tile_byte_counts,
            profile,
            metadata,
            level_policy,
            compression,
            opcode2.as_deref(),
            big,
        );
        raw_ifds.push(build_ifd(
            &IfdBuilder::from_entries(entries),
            raw_ifd_offsets[frame_index],
            big,
        )?);
    }

    let exif_ifd = if let Some(offset) = exif_offset {
        Some(build_ifd(
            &IfdBuilder::from_entries(exif_entries),
            offset,
            big,
        )?)
    } else {
        None
    };
    let gps_ifd = match (gps_offset, gps_entries) {
        (Some(offset), Some(entries)) => {
            Some(build_ifd(&IfdBuilder::from_entries(entries), offset, big)?)
        }
        _ => None,
    };

    file.flush()?;
    let offset_field = if big { 8_u64 } else { 4_u64 };
    file.seek(SeekFrom::Start(offset_field))?;
    if big {
        file.write_all(&ifd0_offset.to_le_bytes())?;
    } else {
        file.write_all(
            &u32::try_from(ifd0_offset)
                .map_err(|_| WriteError::TooLarge)?
                .to_le_bytes(),
        )?;
    }
    file.seek(SeekFrom::End(0))?;
    file.write_all(&ifd0)?;
    for ifd in &raw_ifds {
        file.write_all(ifd)?;
    }
    if let Some(ifd) = &exif_ifd {
        file.write_all(ifd)?;
    }
    if let Some(ifd) = &gps_ifd {
        file.write_all(ifd)?;
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&temp_path, output_path)?;
    guard.armed = false;
    let size = std::fs::metadata(output_path)?.len();
    Ok(WriteInfo {
        path: path.as_ref().to_path_buf(),
        size,
        capacity,
        payload_len: payload_len_usize,
        width,
        height,
        mode,
        bit_depth,
        lsb_planes,
        frames,
        bigtiff: big,
        thumbnail_label: thumbnail.label,
        thumbnail_width: preview_width,
        thumbnail_height: preview_height,
        thumbnail_crc32,
        frame_crc32s,
        payload_frame_crc32,
        white_level: cover_white,
        headroom,
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
mod tests;
