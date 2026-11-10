//! Read-only DNG/TIFF access.
//!
//! This is the Rust counterpart of the Python `DngContainer` read side. It
//! deliberately uses `tiff-reader` for the low-level TIFF/BigTIFF, tile/strip,
//! Deflate and bit-packed-sample work, and adds only the DNG-specific pieces:
//!
//! - finding raw `SubIFD`s (`LinearRaw` 34892 / `CFA` 32803);
//! - normalising 8-bit and bit-packed samples to `u16` for the LSB codec;
//! - reading the split chunk fields (`ImageUniqueID`, `ImageNumber`,
//!   `PageNumber`, `ImageDescription`).
//!
//! `DngReader::raw_frames` still returns full frames for compatibility. The
//! decode path uses `RawBandReader` to consume tile rows/strips in raster order,
//! so it does not need to keep the full raster resident.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use thiserror::Error;
use tiff_reader::{Ifd, TagValue, TiffFile};

use crate::split::{SplitError, SplitIdField, SplitMarkers, SplitSeqField, validate_fields};

/// IFD0 tag pointing at the EXIF Sub-IFD.
pub const TAG_EXIF_IFD: u16 = 34665;
/// IFD0 tag pointing at the GPS IFD.
pub const TAG_GPS_IFD: u16 = 34853;
/// IFD0 `ImageDescription`.
pub const TAG_IMAGE_DESCRIPTION: u16 = 270;
/// IFD0 `PageNumber`.
pub const TAG_PAGE_NUMBER: u16 = 297;
/// EXIF `ImageUniqueID`.
pub const TAG_IMAGE_UNIQUE_ID: u16 = 42016;
/// EXIF `ImageNumber`.
pub const TAG_IMAGE_NUMBER: u16 = 37393;
/// DNG `LinearRaw` photometric interpretation.
pub const PHOTOMETRIC_LINEAR_RAW: u16 = 34892;
/// DNG `CFA` photometric interpretation.
pub const PHOTOMETRIC_CFA: u16 = 32803;
/// Upper bound on decoded samples per frame (about 4 GiB of `u16` samples).
const MAX_FRAME_SAMPLES: usize = 1 << 31;
/// Upper bound on one compressed/uncompressed raw block.
const MAX_BLOCK_BYTES: usize = 512 * 1024 * 1024;

/// Errors from opening or reading a DNG.
#[derive(Debug, Error)]
pub enum DngError {
    /// The file could not be opened or parsed as TIFF.
    #[error("cannot open {path}: {source}")]
    Open {
        /// Input path.
        path: PathBuf,
        /// Underlying TIFF error.
        source: tiff_reader::TiffError,
    },

    /// A TIFF operation failed after the file was opened.
    #[error("TIFF error in {path}: {source}")]
    Tiff {
        /// Input path.
        path: PathBuf,
        /// Underlying TIFF error.
        source: tiff_reader::TiffError,
    },

    /// No `SubIFD` with a raw photometric interpretation was found.
    #[error("no raw SubIFD image found - not a stego DNG?")]
    NoRawSubIfd,

    /// The raw layout is not one `RawHide` writes or reads.
    #[error("unsupported raw layout: {detail}")]
    UnsupportedLayout {
        /// Human-readable detail.
        detail: String,
    },

    /// A tag has the wrong TIFF type for the requested value.
    #[error("tag {tag} has an unsupported value/type: {detail}")]
    InvalidTag {
        /// TIFF tag code.
        tag: u16,
        /// Human-readable detail.
        detail: String,
    },

    /// The TIFF header/IFD structure is malformed.
    #[error("invalid TIFF structure in {path}: {detail}")]
    InvalidFile {
        /// Input path.
        path: PathBuf,
        /// Human-readable detail.
        detail: String,
    },

    /// The EXIF IFD pointer is not a usable offset.
    #[error("tag {tag} is not an inline offset")]
    InvalidIfdPointer {
        /// TIFF tag code.
        tag: u16,
    },

    /// Decoded sample count does not match the declared geometry.
    #[error("decoded {actual} samples, expected {expected}")]
    SampleCountMismatch {
        /// Expected number of samples.
        expected: usize,
        /// Actual number of samples.
        actual: usize,
    },

    /// A split-field error (unknown field, unparseable sequence, etc.).
    #[error(transparent)]
    Split(#[from] SplitError),
}

/// One raw image frame, normalised for the LSB codec.
///
/// `u8` samples are widened to `u16` (values 0..=255); their low bits are
/// unchanged, which is all the codec reads. 10/12/14-bit packed samples are
/// already returned by `tiff-reader` in a 16-bit container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// Samples per pixel: 3 for `linear`, 1 for `cfa`.
    pub samples_per_pixel: usize,
    /// Declared bits per sample (8/10/12/14/16).
    pub bits_per_sample: u16,
    /// Interleaved samples in C order.
    pub samples: Vec<u16>,
}

impl RawFrame {
    /// Number of samples in the frame.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

/// Per-frame raw geometry shared by the full-frame and streaming readers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RawFrameInfo {
    width: usize,
    height: usize,
    samples_per_pixel: usize,
    bits_per_sample: u16,
    sample_count: usize,
    little_endian: bool,
    compression: u16,
    predictor: u16,
}

impl RawFrameInfo {
    /// Number of interleaved samples in the frame.
    pub(crate) const fn sample_count(self) -> usize {
        self.sample_count
    }
}

/// One horizontal band of decoded raw samples.
pub(crate) struct RawBand {
    y_start: usize,
    #[allow(dead_code)]
    rows: usize,
    pub(crate) samples: Vec<u16>,
}

enum FrameLayout {
    Tiles {
        tile_width: usize,
        tile_height: usize,
        across: usize,
        offsets: Vec<u64>,
        byte_counts: Vec<u64>,
    },
    Strips {
        rows_per_strip: usize,
        offsets: Vec<u64>,
        byte_counts: Vec<u64>,
    },
}

enum BandPlan {
    Tiles {
        tile_width: usize,
        tile_height: usize,
        rows: usize,
        offsets: Vec<u64>,
        byte_counts: Vec<u64>,
    },
    Strip {
        y_start: usize,
        rows: usize,
        offset: u64,
        byte_count: u64,
    },
}

/// Streaming reader for one raw frame, yielding tile rows or strips.
pub(crate) struct RawBandReader<'a> {
    reader: &'a DngReader,
    info: RawFrameInfo,
    layout: FrameLayout,
    file: File,
    next_y: usize,
}

impl RawBandReader<'_> {
    /// Next band in raster order, or `None` after the last row.
    ///
    /// # Errors
    ///
    /// Returns a TIFF/layout error for malformed blocks.
    pub(crate) fn next_band(&mut self) -> Result<Option<RawBand>, DngError> {
        if self.next_y >= self.info.height {
            return Ok(None);
        }
        let next_y = self.next_y;
        let plan = match &self.layout {
            FrameLayout::Tiles {
                tile_width,
                tile_height,
                across,
                offsets,
                byte_counts,
            } => {
                let tile_y = next_y / tile_height;
                let rows = (self.info.height - next_y).min(*tile_height);
                let start =
                    tile_y
                        .checked_mul(*across)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "tile row index overflows usize".to_owned(),
                        })?;
                let end =
                    start
                        .checked_add(*across)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "tile row range overflows usize".to_owned(),
                        })?;
                let row_offsets =
                    offsets
                        .get(start..end)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "missing tile offsets in row".to_owned(),
                        })?;
                let row_counts =
                    byte_counts
                        .get(start..end)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "missing tile byte counts in row".to_owned(),
                        })?;
                BandPlan::Tiles {
                    tile_width: *tile_width,
                    tile_height: *tile_height,
                    rows,
                    offsets: row_offsets.to_vec(),
                    byte_counts: row_counts.to_vec(),
                }
            }
            FrameLayout::Strips {
                rows_per_strip,
                offsets,
                byte_counts,
            } => {
                let index = next_y / rows_per_strip;
                let y_start = index.checked_mul(*rows_per_strip).ok_or_else(|| {
                    DngError::UnsupportedLayout {
                        detail: "strip row index overflows usize".to_owned(),
                    }
                })?;
                let rows = (self.info.height - y_start).min(*rows_per_strip);
                let offset = *offsets
                    .get(index)
                    .ok_or_else(|| DngError::UnsupportedLayout {
                        detail: "missing strip offset".to_owned(),
                    })?;
                let byte_count =
                    *byte_counts
                        .get(index)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "missing strip byte count".to_owned(),
                        })?;
                BandPlan::Strip {
                    y_start,
                    rows,
                    offset,
                    byte_count,
                }
            }
        };
        self.read_plan(plan).map(Some)
    }

    fn read_plan(&mut self, plan: BandPlan) -> Result<RawBand, DngError> {
        let info = self.info;
        match plan {
            BandPlan::Tiles {
                tile_width,
                tile_height,
                rows,
                offsets,
                byte_counts,
            } => {
                let y_start = self.next_y;
                let samples = info
                    .width
                    .checked_mul(rows)
                    .and_then(|pixels| pixels.checked_mul(info.samples_per_pixel))
                    .ok_or_else(|| DngError::UnsupportedLayout {
                        detail: "band sample count overflows usize".to_owned(),
                    })?;
                let mut band = vec![0_u16; samples];
                let row_bytes =
                    row_bytes(tile_width, info.samples_per_pixel, info.bits_per_sample)?;
                let expected_bytes = row_bytes.checked_mul(tile_height).ok_or_else(|| {
                    DngError::UnsupportedLayout {
                        detail: "tile byte count overflows usize".to_owned(),
                    }
                })?;
                for (tile_x, (&offset, &byte_count)) in
                    offsets.iter().zip(byte_counts.iter()).enumerate()
                {
                    let raw = self.reader.read_block(
                        &mut self.file,
                        offset,
                        byte_count,
                        info.compression,
                        expected_bytes,
                    )?;
                    let block = decode_block(
                        &raw,
                        tile_width,
                        tile_height,
                        info.samples_per_pixel,
                        info.bits_per_sample,
                        info.little_endian,
                        info.predictor,
                    )?;
                    copy_block(
                        &mut band,
                        info.width,
                        rows,
                        info.samples_per_pixel,
                        tile_width,
                        tile_height,
                        tile_x * tile_width,
                        0,
                        &block,
                    );
                }
                self.next_y = y_start + rows;
                Ok(RawBand {
                    y_start,
                    rows,
                    samples: band,
                })
            }
            BandPlan::Strip {
                y_start,
                rows,
                offset,
                byte_count,
            } => {
                let row_bytes =
                    row_bytes(info.width, info.samples_per_pixel, info.bits_per_sample)?;
                let expected_bytes =
                    row_bytes
                        .checked_mul(rows)
                        .ok_or_else(|| DngError::UnsupportedLayout {
                            detail: "strip byte count overflows usize".to_owned(),
                        })?;
                let raw = self.reader.read_block(
                    &mut self.file,
                    offset,
                    byte_count,
                    info.compression,
                    expected_bytes,
                )?;
                let samples = decode_block(
                    &raw,
                    info.width,
                    rows,
                    info.samples_per_pixel,
                    info.bits_per_sample,
                    info.little_endian,
                    info.predictor,
                )?;
                self.next_y = y_start + rows;
                Ok(RawBand {
                    y_start,
                    rows,
                    samples,
                })
            }
        }
    }
}

/// A read-only DNG handle.
pub struct DngReader {
    path: PathBuf,
    file: TiffFile,
}

impl fmt::Debug for DngReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DngReader")
            .field("path", &self.path)
            .field("ifd_count", &self.file.ifd_count())
            .finish()
    }
}

impl DngReader {
    /// Open a DNG/TIFF file.
    ///
    /// # Errors
    ///
    /// Returns [`DngError::Open`] when the file cannot be opened or parsed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DngError> {
        let path = path.as_ref().to_path_buf();
        let file = TiffFile::open(&path).map_err(|source| DngError::Open {
            path: path.clone(),
            source,
        })?;
        Ok(Self { path, file })
    }

    /// Input path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The primary IFD (preview/metadata directory).
    ///
    /// # Errors
    ///
    /// Returns a TIFF error if IFD0 cannot be read.
    pub fn ifd0(&self) -> Result<&Ifd, DngError> {
        self.ifd(0)
    }

    /// Raw `SubIFD` offsets from IFD0, in file order.
    ///
    /// Unlike `tiff-reader`'s tag decoding, this understands classic TIFF
    /// type 13 (`IFD`) and `BigTIFF` type 18 (`IFD8`).
    ///
    /// # Errors
    ///
    /// Returns a TIFF/layout error for malformed files.
    pub fn sub_ifd_offsets(&self) -> Result<Vec<u64>, DngError> {
        self.read_sub_ifd_offsets()
    }

    /// Raw `SubIFD` directories in file order.
    ///
    /// # Errors
    ///
    /// Returns a TIFF/layout error for malformed files.
    pub fn raw_ifds(&self) -> Result<Vec<Ifd>, DngError> {
        let mut out = Vec::new();
        for offset in self.read_sub_ifd_offsets()? {
            let ifd = self.read_ifd(offset)?;
            if is_raw(&ifd) {
                out.push(ifd);
            }
        }
        Ok(out)
    }

    /// Raw `SubIFD` frames in file order.
    ///
    /// # Errors
    ///
    /// Returns [`DngError::NoRawSubIfd`] when no raw image is present, or a
    /// TIFF/layout error for malformed or unsupported files.
    pub fn raw_frames(&self) -> Result<Vec<RawFrame>, DngError> {
        let ifds = self.raw_ifd_list()?;
        if ifds.is_empty() {
            return Err(DngError::NoRawSubIfd);
        }
        ifds.iter().map(|ifd| self.read_frame(ifd)).collect()
    }

    /// Per-frame geometry in file order.
    pub(crate) fn raw_frame_infos(&self) -> Result<Vec<RawFrameInfo>, DngError> {
        let ifds = self.raw_ifd_list()?;
        if ifds.is_empty() {
            return Err(DngError::NoRawSubIfd);
        }
        ifds.iter().map(|ifd| self.frame_info(ifd)).collect()
    }

    /// Streaming reader for raw frame `index`.
    pub(crate) fn band_reader(&self, index: usize) -> Result<RawBandReader<'_>, DngError> {
        let ifds = self.raw_ifd_list()?;
        let ifd = ifds.get(index).ok_or_else(|| DngError::UnsupportedLayout {
            detail: format!("raw frame index {index} out of range"),
        })?;
        self.band_reader_for_ifd(ifd)
    }

    fn raw_ifd_list(&self) -> Result<Vec<Ifd>, DngError> {
        let mut ifds = Vec::new();
        for offset in self.read_sub_ifd_offsets()? {
            let ifd = self.read_ifd(offset)?;
            if is_raw(&ifd) {
                ifds.push(ifd);
            }
        }
        if ifds.is_empty() {
            // Fallback for a plain multi-page TIFF: top-level IFDs after IFD0.
            for index in 1..self.file.ifd_count() {
                let ifd = self.ifd(index)?;
                if is_raw(ifd) {
                    ifds.push(ifd.clone());
                }
            }
        }
        Ok(ifds)
    }

    /// Read split markers using the configured fields.
    ///
    /// # Errors
    ///
    /// Returns [`DngError::Split`] for unknown/colliding fields or an
    /// unparseable sequence value, and a TIFF error if the EXIF IFD is
    /// malformed.
    pub fn split_markers(
        &self,
        id_field: SplitIdField,
        seq_field: SplitSeqField,
    ) -> Result<SplitMarkers, DngError> {
        validate_fields(id_field, seq_field)?;
        let ifd0 = self.ifd(0)?;
        let exif = self.exif_ifd()?;
        let mut markers = SplitMarkers::default();

        match id_field {
            SplitIdField::ImageUniqueId => {
                if let Some(exif) = &exif
                    && let Some(value) = exif.tag(TAG_IMAGE_UNIQUE_ID)
                {
                    markers.id_present = true;
                    markers.id = Some(ascii_value(TAG_IMAGE_UNIQUE_ID, &value.value)?);
                }
            }
            SplitIdField::ImageDescription => {
                if let Some(value) = ifd0.tag(TAG_IMAGE_DESCRIPTION) {
                    markers.id_present = true;
                    markers.id = Some(ascii_value(TAG_IMAGE_DESCRIPTION, &value.value)?);
                }
            }
            SplitIdField::None => {}
        }

        match seq_field {
            SplitSeqField::ImageNumber => {
                if let Some(exif) = &exif
                    && let Some(value) = exif.tag(TAG_IMAGE_NUMBER)
                {
                    markers.seq_present = true;
                    markers.seq = Some(u32_value(TAG_IMAGE_NUMBER, &value.value)?);
                }
            }
            SplitSeqField::PageNumber => {
                if let Some(value) = ifd0.tag(TAG_PAGE_NUMBER) {
                    markers.seq_present = true;
                    let Some(values) = value.value.as_u16_slice() else {
                        return Err(DngError::InvalidTag {
                            tag: TAG_PAGE_NUMBER,
                            detail: "expected SHORT sequence+total".to_owned(),
                        });
                    };
                    if values.len() != 2 {
                        return Err(DngError::InvalidTag {
                            tag: TAG_PAGE_NUMBER,
                            detail: format!("expected two SHORT values, got {}", values.len()),
                        });
                    }
                    markers.seq = Some(u32::from(values[0]));
                    markers.total = Some(u32::from(values[1]));
                }
            }
            SplitSeqField::ImageDescription => {
                if let Some(value) = ifd0.tag(TAG_IMAGE_DESCRIPTION) {
                    markers.seq_present = true;
                    let text = ascii_value(TAG_IMAGE_DESCRIPTION, &value.value)?;
                    let (seq, total) = parse_sequence_text(&text)?;
                    markers.seq = Some(seq);
                    markers.total = Some(total);
                }
            }
            SplitSeqField::None => {}
        }

        Ok(markers)
    }

    /// The EXIF Sub-IFD, when present.
    ///
    /// # Errors
    ///
    /// Returns a TIFF/layout error for a malformed EXIF IFD or pointer.
    pub fn exif_ifd(&self) -> Result<Option<Ifd>, DngError> {
        let ifd0 = self.ifd(0)?;
        let Some(tag) = ifd0.tag(TAG_EXIF_IFD) else {
            return Ok(None);
        };
        let offset = tag
            .value
            .as_u64()
            .ok_or(DngError::InvalidIfdPointer { tag: TAG_EXIF_IFD })?;
        Ok(Some(self.read_ifd(offset)?))
    }

    /// The GPS Sub-IFD, when present.
    ///
    /// # Errors
    ///
    /// Returns a TIFF/layout error for a malformed GPS IFD or pointer.
    pub fn gps_ifd(&self) -> Result<Option<Ifd>, DngError> {
        let ifd0 = self.ifd(0)?;
        let Some(tag) = ifd0.tag(TAG_GPS_IFD) else {
            return Ok(None);
        };
        let offset = tag
            .value
            .as_u64()
            .ok_or(DngError::InvalidIfdPointer { tag: TAG_GPS_IFD })?;
        Ok(Some(self.read_ifd(offset)?))
    }

    fn read_sub_ifd_offsets(&self) -> Result<Vec<u64>, DngError> {
        let invalid = |detail: String| DngError::InvalidFile {
            path: self.path.clone(),
            detail,
        };
        let mut file =
            File::open(&self.path).map_err(|error| invalid(format!("cannot open: {error}")))?;
        let mut header = [0_u8; 16];
        file.read_exact(&mut header)
            .map_err(|error| invalid(format!("short TIFF header: {error}")))?;

        let little = match &header[..2] {
            b"II" => true,
            b"MM" => false,
            _ => return Err(invalid("not a little/big-endian TIFF".to_owned())),
        };
        let version = read_u16(&header[2..4], little);
        let (bigtiff, ifd0_offset) = match version {
            42 => (false, u64::from(read_u32(&header[4..8], little))),
            43 => (true, read_u64(&header[8..16], little)),
            other => {
                return Err(invalid(format!("unsupported TIFF version {other}")));
            }
        };

        file.seek(SeekFrom::Start(ifd0_offset))
            .map_err(|error| invalid(format!("cannot seek to IFD0: {error}")))?;
        let (entry_count, entry_size, field_size) = if bigtiff {
            let mut count_bytes = [0_u8; 8];
            file.read_exact(&mut count_bytes)
                .map_err(|error| invalid(format!("short BigTIFF IFD0: {error}")))?;
            (read_u64(&count_bytes, little), 20, 8)
        } else {
            let mut count_bytes = [0_u8; 2];
            file.read_exact(&mut count_bytes)
                .map_err(|error| invalid(format!("short TIFF IFD0: {error}")))?;
            (u64::from(read_u16(&count_bytes, little)), 12, 4)
        };
        let entry_count = usize::try_from(entry_count)
            .map_err(|_| invalid("IFD entry count does not fit in memory".to_owned()))?;
        if entry_count > 4096 {
            return Err(invalid(format!(
                "implausible IFD0 entry count {entry_count}"
            )));
        }

        let mut entries = vec![0_u8; entry_count * entry_size];
        file.read_exact(&mut entries)
            .map_err(|error| invalid(format!("short IFD0 entries: {error}")))?;

        for entry in entries.chunks_exact(entry_size) {
            if read_u16(&entry[0..2], little) != 330 {
                continue;
            }
            let type_code = read_u16(&entry[2..4], little);
            let count = if bigtiff {
                read_u64(&entry[4..12], little)
            } else {
                u64::from(read_u32(&entry[4..8], little))
            };
            let element_size: usize = match type_code {
                13 | 4 => 4,
                18 | 16 if bigtiff => 8,
                other => {
                    return Err(invalid(format!(
                        "SubIFDs tag has unsupported TIFF type {other}"
                    )));
                }
            };
            let count = usize::try_from(count)
                .map_err(|_| invalid("SubIFDs count does not fit in memory".to_owned()))?;
            if count > 4096 {
                return Err(invalid(format!("implausible SubIFDs count {count}")));
            }
            let total = count
                .checked_mul(element_size)
                .ok_or_else(|| invalid("SubIFDs byte count overflows".to_owned()))?;

            let raw = if total <= field_size {
                entry[entry_size - field_size..].to_vec()
            } else {
                let offset = if bigtiff {
                    read_u64(&entry[entry_size - 8..], little)
                } else {
                    u64::from(read_u32(&entry[entry_size - 4..], little))
                };
                let mut value = vec![0_u8; total];
                file.seek(SeekFrom::Start(offset))
                    .map_err(|error| invalid(format!("cannot seek to SubIFDs: {error}")))?;
                file.read_exact(&mut value)
                    .map_err(|error| invalid(format!("short SubIFDs array: {error}")))?;
                value
            };

            let mut offsets = Vec::with_capacity(count);
            for index in 0..count {
                let start = index * element_size;
                let offset = if element_size == 8 {
                    read_u64(&raw[start..start + 8], little)
                } else {
                    u64::from(read_u32(&raw[start..start + 4], little))
                };
                offsets.push(offset);
            }
            return Ok(offsets);
        }

        Ok(Vec::new())
    }

    fn ifd(&self, index: usize) -> Result<&Ifd, DngError> {
        self.file.ifd(index).map_err(|source| DngError::Tiff {
            path: self.path.clone(),
            source,
        })
    }

    fn read_ifd(&self, offset: u64) -> Result<Ifd, DngError> {
        self.file
            .read_ifd_at_offset(offset)
            .map_err(|source| DngError::Tiff {
                path: self.path.clone(),
                source,
            })
    }

    fn read_frame(&self, ifd: &Ifd) -> Result<RawFrame, DngError> {
        let info = self.frame_info(ifd)?;
        let mut bands = self.band_reader_for_ifd(ifd)?;
        let mut samples = vec![0_u16; info.sample_count()];
        while let Some(band) = bands.next_band()? {
            let start = band
                .y_start
                .checked_mul(info.width)
                .and_then(|pixels| pixels.checked_mul(info.samples_per_pixel))
                .ok_or_else(|| DngError::UnsupportedLayout {
                    detail: "band offset overflows usize".to_owned(),
                })?;
            let end = start.checked_add(band.samples.len()).ok_or_else(|| {
                DngError::UnsupportedLayout {
                    detail: "band end overflows usize".to_owned(),
                }
            })?;
            let target =
                samples
                    .get_mut(start..end)
                    .ok_or_else(|| DngError::UnsupportedLayout {
                        detail: "band lies outside the frame".to_owned(),
                    })?;
            target.copy_from_slice(&band.samples);
        }
        Ok(RawFrame {
            width: info.width,
            height: info.height,
            samples_per_pixel: info.samples_per_pixel,
            bits_per_sample: info.bits_per_sample,
            samples,
        })
    }

    fn frame_info(&self, ifd: &Ifd) -> Result<RawFrameInfo, DngError> {
        let width = usize::try_from(ifd.width()).map_err(|_| DngError::UnsupportedLayout {
            detail: "width does not fit usize".to_owned(),
        })?;
        let height = usize::try_from(ifd.height()).map_err(|_| DngError::UnsupportedLayout {
            detail: "height does not fit usize".to_owned(),
        })?;
        let samples_per_pixel = usize::from(ifd.samples_per_pixel());
        if !matches!(samples_per_pixel, 1 | 3) {
            return Err(DngError::UnsupportedLayout {
                detail: format!(
                    "samples_per_pixel={samples_per_pixel} (expected 1 for CFA or 3 for linear)"
                ),
            });
        }

        let bits_values = ifd.bits_per_sample().map_err(|source| DngError::Tiff {
            path: self.path.clone(),
            source,
        })?;
        let bits = *bits_values
            .first()
            .ok_or_else(|| DngError::UnsupportedLayout {
                detail: "missing BitsPerSample".to_owned(),
            })?;
        if !bits_values.iter().all(|&value| value == bits) {
            return Err(DngError::UnsupportedLayout {
                detail: format!("mixed BitsPerSample values: {bits_values:?}"),
            });
        }
        if !matches!(bits, 8 | 10 | 12 | 14 | 16) {
            return Err(DngError::UnsupportedLayout {
                detail: format!("bits_per_sample={bits}"),
            });
        }

        let sample_format = Self::tag_u16(ifd, 339)?.unwrap_or(1);
        if sample_format != 1 {
            return Err(DngError::UnsupportedLayout {
                detail: format!("sample_format={sample_format} (expected unsigned integer)"),
            });
        }
        let planar_configuration = Self::tag_u16(ifd, 284)?.unwrap_or(1);
        if planar_configuration != 1 {
            return Err(DngError::UnsupportedLayout {
                detail: format!("planar_configuration={planar_configuration} (expected chunky)"),
            });
        }
        let fill_order = Self::tag_u16(ifd, 266)?.unwrap_or(1);
        if fill_order != 1 {
            return Err(DngError::UnsupportedLayout {
                detail: format!("fill_order={fill_order} (expected MSB-to-LSB)"),
            });
        }

        let compression = ifd.compression();
        if !matches!(compression, 1 | 8 | 32_946) {
            return Err(DngError::UnsupportedLayout {
                detail: format!("compression={compression} (expected none or Deflate)"),
            });
        }
        let predictor = Self::tag_u16(ifd, 317)?.unwrap_or(1);
        if !matches!(predictor, 1 | 2) {
            return Err(DngError::UnsupportedLayout {
                detail: format!("predictor={predictor}"),
            });
        }
        if !matches!(bits, 8 | 16) && predictor != 1 {
            return Err(DngError::UnsupportedLayout {
                detail: format!(
                    "predictor {predictor} is not supported for {bits}-bit packed samples"
                ),
            });
        }

        let sample_count = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(samples_per_pixel))
            .ok_or_else(|| DngError::UnsupportedLayout {
                detail: "image sample count overflows usize".to_owned(),
            })?;
        if sample_count > MAX_FRAME_SAMPLES {
            return Err(DngError::UnsupportedLayout {
                detail: format!("frame has {sample_count} samples (limit {MAX_FRAME_SAMPLES})"),
            });
        }

        Ok(RawFrameInfo {
            width,
            height,
            samples_per_pixel,
            bits_per_sample: bits,
            sample_count,
            little_endian: matches!(self.file.byte_order(), tiff_reader::ByteOrder::LittleEndian),
            compression,
            predictor,
        })
    }

    fn frame_layout(ifd: &Ifd, info: &RawFrameInfo) -> Result<FrameLayout, DngError> {
        let tiles = (
            Self::tag_u64_vec(ifd, 324)?,
            Self::tag_u64_vec(ifd, 325)?,
            Self::tag_u32(ifd, 322)?,
            Self::tag_u32(ifd, 323)?,
        );
        if let (Some(offsets), Some(byte_counts), Some(tile_width), Some(tile_height)) = tiles {
            let tile_width =
                usize::try_from(tile_width).map_err(|_| DngError::UnsupportedLayout {
                    detail: "tile width does not fit usize".to_owned(),
                })?;
            let tile_height =
                usize::try_from(tile_height).map_err(|_| DngError::UnsupportedLayout {
                    detail: "tile height does not fit usize".to_owned(),
                })?;
            if tile_width == 0 || tile_height == 0 {
                return Err(DngError::UnsupportedLayout {
                    detail: "zero tile dimension".to_owned(),
                });
            }
            let across = info.width.div_ceil(tile_width);
            let down = info.height.div_ceil(tile_height);
            let expected_count =
                across
                    .checked_mul(down)
                    .ok_or_else(|| DngError::UnsupportedLayout {
                        detail: "tile count overflows usize".to_owned(),
                    })?;
            if offsets.len() < expected_count || byte_counts.len() < expected_count {
                return Err(DngError::UnsupportedLayout {
                    detail: format!(
                        "expected {expected_count} tiles, got {} offsets and {} byte counts",
                        offsets.len(),
                        byte_counts.len()
                    ),
                });
            }
            return Ok(FrameLayout::Tiles {
                tile_width,
                tile_height,
                across,
                offsets,
                byte_counts,
            });
        }

        let offsets = Self::tag_u64_vec(ifd, 273)?.ok_or_else(|| DngError::UnsupportedLayout {
            detail: "missing StripOffsets and TileOffsets".to_owned(),
        })?;
        let byte_counts =
            Self::tag_u64_vec(ifd, 279)?.ok_or_else(|| DngError::UnsupportedLayout {
                detail: "missing StripByteCounts and TileByteCounts".to_owned(),
            })?;
        let rows_per_strip = Self::tag_u32(ifd, 278)?.map_or(info.height, |rows| {
            usize::try_from(rows).unwrap_or(info.height)
        });
        if rows_per_strip == 0 {
            return Err(DngError::UnsupportedLayout {
                detail: "RowsPerStrip is zero".to_owned(),
            });
        }
        let strip_count = info.height.div_ceil(rows_per_strip);
        if offsets.len() < strip_count || byte_counts.len() < strip_count {
            return Err(DngError::UnsupportedLayout {
                detail: format!(
                    "expected {strip_count} strips, got {} offsets and {} byte counts",
                    offsets.len(),
                    byte_counts.len()
                ),
            });
        }
        Ok(FrameLayout::Strips {
            rows_per_strip,
            offsets,
            byte_counts,
        })
    }

    fn band_reader_for_ifd(&self, ifd: &Ifd) -> Result<RawBandReader<'_>, DngError> {
        let info = self.frame_info(ifd)?;
        let layout = Self::frame_layout(ifd, &info)?;
        let file = File::open(&self.path).map_err(|error| DngError::InvalidFile {
            path: self.path.clone(),
            detail: format!("cannot open raw data: {error}"),
        })?;
        Ok(RawBandReader {
            reader: self,
            info,
            layout,
            file,
            next_y: 0,
        })
    }

    fn read_block(
        &self,
        file: &mut File,
        offset: u64,
        byte_count: u64,
        compression: u16,
        expected_bytes: usize,
    ) -> Result<Vec<u8>, DngError> {
        let invalid = |detail: String| DngError::InvalidFile {
            path: self.path.clone(),
            detail,
        };
        let byte_count = usize::try_from(byte_count)
            .map_err(|_| invalid("block byte count does not fit in memory".to_owned()))?;
        if byte_count > MAX_BLOCK_BYTES {
            return Err(invalid(format!(
                "raw block is {byte_count} bytes (limit {MAX_BLOCK_BYTES})"
            )));
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| invalid(format!("cannot seek to raw block: {error}")))?;
        let mut data = vec![0_u8; byte_count];
        file.read_exact(&mut data)
            .map_err(|error| invalid(format!("short raw block: {error}")))?;

        let data = match compression {
            1 => data,
            8 | 32_946 => {
                let mut decoder = flate2::read::ZlibDecoder::new(data.as_slice());
                let mut decoded = Vec::with_capacity(expected_bytes);
                decoder
                    .read_to_end(&mut decoded)
                    .map_err(|error| invalid(format!("Deflate decode failed: {error}")))?;
                decoded
            }
            other => {
                return Err(DngError::UnsupportedLayout {
                    detail: format!("compression={other}"),
                });
            }
        };

        if data.len() < expected_bytes {
            return Err(invalid(format!(
                "block decoded to {} bytes, expected {expected_bytes}",
                data.len()
            )));
        }
        let mut data = data;
        data.truncate(expected_bytes);
        Ok(data)
    }

    fn tag_u16(ifd: &Ifd, tag: u16) -> Result<Option<u16>, DngError> {
        ifd.tag(tag)
            .map(|value| {
                value.value.as_u16().ok_or_else(|| DngError::InvalidTag {
                    tag,
                    detail: "expected a single SHORT".to_owned(),
                })
            })
            .transpose()
    }

    fn tag_u32(ifd: &Ifd, tag: u16) -> Result<Option<u32>, DngError> {
        ifd.tag(tag)
            .map(|value| {
                value.value.as_u32().ok_or_else(|| DngError::InvalidTag {
                    tag,
                    detail: "expected a single SHORT/LONG".to_owned(),
                })
            })
            .transpose()
    }

    fn tag_u64_vec(ifd: &Ifd, tag: u16) -> Result<Option<Vec<u64>>, DngError> {
        ifd.tag(tag)
            .map(|value| {
                value
                    .value
                    .as_u64_vec()
                    .ok_or_else(|| DngError::InvalidTag {
                        tag,
                        detail: "expected an array of SHORT/LONG/LONG8 values".to_owned(),
                    })
            })
            .transpose()
    }
}

fn row_bytes(width: usize, samples_per_pixel: usize, bits: u16) -> Result<usize, DngError> {
    let byte_width = match bits {
        8 | 10 | 12 | 14 | 16 => usize::from(bits),
        other => {
            return Err(DngError::UnsupportedLayout {
                detail: format!("bits_per_sample={other}"),
            });
        }
    };
    let samples =
        width
            .checked_mul(samples_per_pixel)
            .ok_or_else(|| DngError::UnsupportedLayout {
                detail: "row sample count overflows usize".to_owned(),
            })?;
    if matches!(bits, 8 | 16) {
        samples
            .checked_mul(byte_width / 8)
            .ok_or_else(|| DngError::UnsupportedLayout {
                detail: "row byte count overflows usize".to_owned(),
            })
    } else {
        samples
            .checked_mul(byte_width)
            .and_then(|value| value.checked_add(7))
            .map(|value| value / 8)
            .ok_or_else(|| DngError::UnsupportedLayout {
                detail: "row bit count overflows usize".to_owned(),
            })
    }
}

fn decode_block(
    raw: &[u8],
    width: usize,
    rows: usize,
    samples_per_pixel: usize,
    bits: u16,
    little_endian: bool,
    predictor: u16,
) -> Result<Vec<u16>, DngError> {
    let sample_count = width
        .checked_mul(rows)
        .and_then(|pixels| pixels.checked_mul(samples_per_pixel))
        .ok_or_else(|| DngError::UnsupportedLayout {
            detail: "block sample count overflows usize".to_owned(),
        })?;

    let mut samples: Vec<u16> = match bits {
        8 => raw
            .iter()
            .take(sample_count)
            .map(|&value| u16::from(value))
            .collect(),
        16 => {
            let needed =
                sample_count
                    .checked_mul(2)
                    .ok_or_else(|| DngError::UnsupportedLayout {
                        detail: "block byte count overflows usize".to_owned(),
                    })?;
            if raw.len() < needed {
                return Err(DngError::UnsupportedLayout {
                    detail: format!("short 16-bit block: {} < {needed}", raw.len()),
                });
            }
            let (pairs, _remainder) = raw[..needed].as_chunks::<2>();
            pairs
                .iter()
                .map(|pair| {
                    if little_endian {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect()
        }
        10 | 12 | 14 => unpack_packed(raw, width, rows, samples_per_pixel, bits)?,
        other => {
            return Err(DngError::UnsupportedLayout {
                detail: format!("bits_per_sample={other}"),
            });
        }
    };

    if samples.len() < sample_count {
        return Err(DngError::UnsupportedLayout {
            detail: format!("short block: {} < {sample_count} samples", samples.len()),
        });
    }
    samples.truncate(sample_count);

    if predictor == 2 {
        if !matches!(bits, 8 | 16) {
            return Err(DngError::UnsupportedLayout {
                detail: format!("predictor 2 with {bits}-bit packed samples"),
            });
        }
        let row_samples = width * samples_per_pixel;
        for row in 0..rows {
            let base = row * row_samples;
            for index in samples_per_pixel..row_samples {
                let current = base + index;
                samples[current] =
                    samples[current].wrapping_add(samples[current - samples_per_pixel]);
            }
        }
    }

    Ok(samples)
}

fn unpack_packed(
    raw: &[u8],
    width: usize,
    rows: usize,
    samples_per_pixel: usize,
    bits: u16,
) -> Result<Vec<u16>, DngError> {
    let bits_usize = usize::from(bits);
    let row_bytes = row_bytes(width, samples_per_pixel, bits)?;
    let needed = row_bytes
        .checked_mul(rows)
        .ok_or_else(|| DngError::UnsupportedLayout {
            detail: "packed block byte count overflows usize".to_owned(),
        })?;
    if raw.len() < needed {
        return Err(DngError::UnsupportedLayout {
            detail: format!("short packed block: {} < {needed}", raw.len()),
        });
    }

    let row_samples = width * samples_per_pixel;
    let mut out = Vec::with_capacity(row_samples * rows);
    for row in 0..rows {
        let base = row * row_bytes;
        for sample in 0..row_samples {
            let bit_offset = sample * bits_usize;
            let mut value = 0_u16;
            for bit in 0..bits_usize {
                let index = bit_offset + bit;
                let byte = raw[base + index / 8];
                let bit_value = (byte >> (7 - (index % 8))) & 1;
                value = (value << 1) | u16::from(bit_value);
            }
            out.push(value);
        }
    }
    Ok(out)
}

fn copy_block(
    target: &mut [u16],
    width: usize,
    height: usize,
    samples_per_pixel: usize,
    block_width: usize,
    block_height: usize,
    x0: usize,
    y0: usize,
    block: &[u16],
) {
    for y in 0..block_height {
        let target_y = y0 + y;
        if target_y >= height {
            break;
        }
        for x in 0..block_width {
            let target_x = x0 + x;
            if target_x >= width {
                break;
            }
            for channel in 0..samples_per_pixel {
                target[(target_y * width + target_x) * samples_per_pixel + channel] =
                    block[(y * block_width + x) * samples_per_pixel + channel];
            }
        }
    }
}

fn is_raw(ifd: &Ifd) -> bool {
    matches!(
        ifd.photometric_interpretation(),
        Some(PHOTOMETRIC_LINEAR_RAW | PHOTOMETRIC_CFA)
    )
}

fn read_u16(bytes: &[u8], little: bool) -> u16 {
    let array: [u8; 2] = bytes.try_into().expect("2-byte slice");
    if little {
        u16::from_le_bytes(array)
    } else {
        u16::from_be_bytes(array)
    }
}

fn read_u32(bytes: &[u8], little: bool) -> u32 {
    let array: [u8; 4] = bytes.try_into().expect("4-byte slice");
    if little {
        u32::from_le_bytes(array)
    } else {
        u32::from_be_bytes(array)
    }
}

fn read_u64(bytes: &[u8], little: bool) -> u64 {
    let array: [u8; 8] = bytes.try_into().expect("8-byte slice");
    if little {
        u64::from_le_bytes(array)
    } else {
        u64::from_be_bytes(array)
    }
}

fn ascii_value(tag: u16, value: &TagValue) -> Result<String, DngError> {
    value
        .as_str()
        .map(|text| text.trim_end_matches('\u{0}').to_owned())
        .ok_or_else(|| DngError::InvalidTag {
            tag,
            detail: "expected ASCII".to_owned(),
        })
}

fn u32_value(tag: u16, value: &TagValue) -> Result<u32, DngError> {
    value.as_u32().ok_or_else(|| DngError::InvalidTag {
        tag,
        detail: "expected a single SHORT/LONG".to_owned(),
    })
}

fn parse_sequence_text(text: &str) -> Result<(u32, u32), DngError> {
    let (sequence, total) = text.split_once('/').ok_or_else(|| {
        DngError::Split(SplitError::InvalidSequence {
            value: text.to_owned(),
        })
    })?;
    let sequence = sequence.trim().parse().map_err(|_| {
        DngError::Split(SplitError::InvalidSequence {
            value: text.to_owned(),
        })
    })?;
    let total = total.trim().parse().map_err(|_| {
        DngError::Split(SplitError::InvalidSequence {
            value: text.to_owned(),
        })
    })?;
    Ok((sequence, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_text_parses() {
        assert_eq!(parse_sequence_text("0003/0012").unwrap(), (3, 12));
        assert!(parse_sequence_text("3 of 12").is_err());
    }

    #[test]
    fn endian_helpers_work() {
        assert_eq!(read_u16(&[0x34, 0x12], true), 0x1234);
        assert_eq!(read_u16(&[0x12, 0x34], false), 0x1234);
        assert_eq!(read_u32(&[0x78, 0x56, 0x34, 0x12], true), 0x1234_5678);
        assert_eq!(read_u32(&[0x12, 0x34, 0x56, 0x78], false), 0x1234_5678);
        assert_eq!(
            read_u64(&[8, 7, 6, 5, 4, 3, 2, 1], true),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn row_bytes_cover_container_and_packed_layouts() {
        assert_eq!(row_bytes(4, 3, 16).unwrap(), 24);
        assert_eq!(row_bytes(4, 3, 8).unwrap(), 12);
        assert_eq!(row_bytes(4, 1, 14).unwrap(), 7);
        assert_eq!(row_bytes(4, 1, 12).unwrap(), 6);
        assert_eq!(row_bytes(4, 1, 10).unwrap(), 5);
        assert!(row_bytes(4, 1, 32).is_err());
    }

    #[test]
    fn unpacks_12_bit_samples_msb_first() {
        let raw = [0xAB, 0xC1, 0x23];
        let samples = unpack_packed(&raw, 2, 1, 1, 12).unwrap();
        assert_eq!(samples, vec![0xABC, 0x123]);
        assert!(unpack_packed(&raw[..2], 2, 1, 1, 12).is_err());
    }

    #[test]
    fn decoder_undoes_horizontal_predictor() {
        let eight_bit = decode_block(&[1, 2, 3, 4], 2, 2, 1, 8, true, 2).unwrap();
        assert_eq!(eight_bit, vec![1, 3, 3, 7]);

        let sixteen_bit = [1_u16, 2, 3, 4]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let decoded = decode_block(&sixteen_bit, 2, 2, 1, 16, true, 2).unwrap();
        assert_eq!(decoded, vec![1, 3, 3, 7]);
    }

    #[test]
    fn copy_block_crops_at_image_edges() {
        let mut target = vec![0_u16; 4 * 3];
        let block = vec![1_u16, 2, 3, 4];
        copy_block(&mut target, 4, 3, 1, 2, 2, 3, 2, &block);
        // Only the top-left block sample lands inside the 4x3 target at (3, 2).
        assert_eq!(target[2 * 4 + 3], 1);
        assert_eq!(target.iter().filter(|&&value| value != 0).count(), 1);
    }

    #[test]
    fn tag_value_helpers_handle_expected_types() {
        let ascii = TagValue::Ascii("abc\0".to_owned());
        assert_eq!(ascii_value(270, &ascii).unwrap(), "abc");
        assert!(ascii_value(270, &TagValue::Long(vec![1])).is_err());
        assert_eq!(u32_value(37393, &TagValue::Long(vec![7])).unwrap(), 7);
        assert!(u32_value(37393, &TagValue::Ascii("x".to_owned())).is_err());
    }

    #[test]
    fn reads_classic_sub_ifd_offsets() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II*\0");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&330_u16.to_le_bytes());
        bytes.extend_from_slice(&13_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&26_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&100_u32.to_le_bytes());
        bytes.extend_from_slice(&200_u32.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subifd.tif");
        std::fs::write(&path, bytes).unwrap();
        let reader = DngReader::open(&path).unwrap();
        assert_eq!(reader.read_sub_ifd_offsets().unwrap(), vec![100, 200]);
    }

    #[test]
    fn reads_inline_sub_ifd_offset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II*\0");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&330_u16.to_le_bytes());
        bytes.extend_from_slice(&13_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&100_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inline.tif");
        std::fs::write(&path, bytes).unwrap();
        let reader = DngReader::open(&path).unwrap();
        assert_eq!(reader.read_sub_ifd_offsets().unwrap(), vec![100]);
    }

    #[test]
    fn reads_bigtiff_sub_ifd_offsets() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II+\0");
        bytes.extend_from_slice(&8_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.extend_from_slice(&330_u16.to_le_bytes());
        bytes.extend_from_slice(&18_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u64.to_le_bytes());
        bytes.extend_from_slice(&52_u64.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&100_u64.to_le_bytes());
        bytes.extend_from_slice(&200_u64.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.tif");
        std::fs::write(&path, bytes).unwrap();
        let reader = DngReader::open(&path).unwrap();
        assert_eq!(reader.read_sub_ifd_offsets().unwrap(), vec![100, 200]);
    }

    #[test]
    fn rejects_unsupported_sub_ifd_type() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II*\0");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&330_u16.to_le_bytes());
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&100_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("badtype.tif");
        std::fs::write(&path, bytes).unwrap();
        let reader = DngReader::open(&path).unwrap();
        assert!(reader.read_sub_ifd_offsets().is_err());
    }

    #[test]
    fn rejects_short_16bit_and_packed_predictor_blocks() {
        assert!(decode_block(&[0], 1, 1, 1, 16, true, 1).is_err());
        assert!(decode_block(&[0, 0], 1, 1, 1, 12, true, 2).is_err());
    }

    #[test]
    fn rejects_bad_tiff_header_for_sub_ifds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.tif");
        std::fs::write(&path, b"XX").unwrap();
        let reader = DngReader::open(&path);
        assert!(reader.is_err());
    }

    #[test]
    fn band_reader_reproduces_full_frames() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dng");
        for name in [
            "linear_256x192_16bit_1plane.dng",
            "linear_128x96_8bit_deflate.dng",
            "cfa_128x96_14bit_2planes.dng",
            "linear_128x96_16bit_4planes_keyed.dng",
            "linear_128x96_16bit_3frames.dng",
            "split0001.dng",
        ] {
            let reader = DngReader::open(fixtures.join(name)).unwrap();
            let frames = reader.raw_frames().unwrap();
            for (index, frame) in frames.iter().enumerate() {
                let mut bands = reader.band_reader(index).unwrap();
                let mut joined = Vec::new();
                let mut expected_y = 0_usize;
                while let Some(band) = bands.next_band().unwrap() {
                    assert_eq!(band.y_start, expected_y);
                    assert_ne!(band.samples.len(), 0);
                    expected_y += band.rows;
                    joined.extend_from_slice(&band.samples);
                }
                assert_eq!(expected_y, frame.height);
                assert_eq!(joined, frame.samples, "{name} frame {index}");
            }
        }
    }

    #[test]
    fn band_reader_handles_strip_layouts() {
        fn entry(bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let width = 4_usize;
        let height = 3_usize;
        let sub_ifd = 26_u32;
        let strip_offset = sub_ifd + 2 + 11 * 12 + 4;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II");
        bytes.extend_from_slice(&42_u16.to_le_bytes());
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&330_u16.to_le_bytes());
        bytes.extend_from_slice(&13_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&sub_ifd.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        bytes.extend_from_slice(&11_u16.to_le_bytes());
        entry(&mut bytes, 256, 4, 1, width as u32);
        entry(&mut bytes, 257, 4, 1, height as u32);
        entry(&mut bytes, 258, 3, 1, 16);
        entry(&mut bytes, 259, 3, 1, 1);
        entry(&mut bytes, 262, 3, 1, u32::from(PHOTOMETRIC_LINEAR_RAW));
        entry(&mut bytes, 273, 4, 1, strip_offset);
        entry(&mut bytes, 277, 3, 1, 1);
        entry(&mut bytes, 278, 4, 1, height as u32);
        entry(&mut bytes, 279, 4, 1, (width * height * 2) as u32);
        entry(&mut bytes, 284, 3, 1, 1);
        entry(&mut bytes, 339, 3, 1, 1);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for sample in 0..width * height {
            bytes.extend_from_slice(&(sample as u16).to_le_bytes());
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("strip.dng");
        std::fs::write(&path, bytes).unwrap();
        let reader = DngReader::open(&path).unwrap();
        let frames = reader.raw_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].samples, (0..12).collect::<Vec<u16>>());
        let mut bands = reader.band_reader(0).unwrap();
        let band = bands.next_band().unwrap().unwrap();
        assert_eq!(band.samples, frames[0].samples);
        assert!(bands.next_band().unwrap().is_none());
    }
}
