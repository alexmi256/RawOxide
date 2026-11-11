//! Raw image digests written by Adobe-style DNG writers.
//!
//! `NewRawImageDigest` (51111) is the DNG SDK fast image digest: 256x256
//! tiles in row-major order, each hashed in planar pixel order and
//! little-endian byte order, then the concatenated 16-byte tile digests are
//! hashed again.
//!
//! `RawDataUniqueID` (50781) hashes that digest together with the unique
//! camera model, crop size/origin and user-crop rationals. Opcode fingerprints
//! are included when the caller supplies them.

use md5::{Digest, Md5};

/// Tile size used by the DNG SDK fast digest.
pub const DIGEST_TILE: usize = 256;

/// New raw image digest tag.
pub const NEW_RAW_IMAGE_DIGEST_TAG: u16 = 51_111;

/// Raw data unique ID tag.
pub const RAW_DATA_UNIQUE_ID_TAG: u16 = 50_781;

/// Compute `NewRawImageDigest` for interleaved samples.
///
/// `samples` is `width * height * channels` in C order; values are hashed as
/// `u8` when `bit_depth <= 8`, otherwise as little-endian `u16`, matching the
/// stored DNG sample type.
///
/// # Panics
///
/// Panics when `samples.len()` is not exactly
/// `width * height * channels`, instead of hashing zero-filled data.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn new_raw_image_digest(
    samples: &[u16],
    width: usize,
    height: usize,
    channels: usize,
    bit_depth: u16,
) -> [u8; 16] {
    assert_eq!(
        samples.len(),
        width.saturating_mul(height).saturating_mul(channels),
        "raw image sample buffer has the wrong length"
    );
    let mut outer = Md5::new();
    let step = DIGEST_TILE;
    for top in (0..height).step_by(step) {
        let bottom = (top + step).min(height);
        for left in (0..width).step_by(step) {
            let right = (left + step).min(width);
            let mut tile = Md5::new();
            for channel in 0..channels {
                for y in top..bottom {
                    for x in left..right {
                        let index = (y * width + x) * channels + channel;
                        let value = samples[index];
                        if bit_depth <= 8 {
                            tile.update([value as u8]);
                        } else {
                            tile.update(value.to_le_bytes());
                        }
                    }
                }
            }
            outer.update(tile.finalize());
        }
    }
    outer.finalize().into()
}

/// Incremental `NewRawImageDigest` builder.
///
/// Feed rows with [`Self::update_band`] in order, then call [`Self::finish`].
/// The result matches [`new_raw_image_digest`].
#[derive(Debug)]
pub struct RawImageDigest {
    width: usize,
    height: usize,
    channels: usize,
    bit_depth: u16,
    outer: Md5,
    buffers: Vec<Vec<Vec<u8>>>,
    block_start: usize,
    next_row: usize,
}

impl RawImageDigest {
    /// Create a digest builder for an interleaved image.
    #[must_use]
    pub fn new(width: usize, height: usize, channels: usize, bit_depth: u16) -> Self {
        let tile_columns = width.div_ceil(DIGEST_TILE);
        Self {
            width,
            height,
            channels,
            bit_depth,
            outer: Md5::new(),
            buffers: (0..tile_columns)
                .map(|_| (0..channels).map(|_| Vec::new()).collect())
                .collect(),
            block_start: 0,
            next_row: 0,
        }
    }

    /// Hash a horizontal band starting at `y_start`.
    ///
    /// # Panics
    ///
    /// Panics when `band` is shorter than `width * rows * channels`.
    #[allow(clippy::cast_possible_truncation)]
    pub fn update_band(&mut self, y_start: usize, rows: usize, band: &[u16]) {
        assert_eq!(y_start, self.next_row, "digest bands must be contiguous");
        assert!(
            y_start
                .checked_add(rows)
                .is_some_and(|end| end <= self.height),
            "digest band extends past the image height"
        );
        let needed = self
            .width
            .checked_mul(rows)
            .and_then(|samples| samples.checked_mul(self.channels))
            .expect("digest band geometry overflows");
        assert!(
            band.len() >= needed,
            "digest band is shorter than its geometry"
        );
        self.next_row += rows;
        for local_y in 0..rows {
            let y = y_start + local_y;
            if y >= self.block_start + DIGEST_TILE {
                self.finish_block();
                self.block_start += DIGEST_TILE;
            }
            for tile_index in 0..self.buffers.len() {
                let left = tile_index * DIGEST_TILE;
                let right = (left + DIGEST_TILE).min(self.width);
                for channel in 0..self.channels {
                    let buffer = &mut self.buffers[tile_index][channel];
                    for x in left..right {
                        let value = band[(local_y * self.width + x) * self.channels + channel];
                        if self.bit_depth <= 8 {
                            buffer.push(value as u8);
                        } else {
                            buffer.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
        }
    }

    fn finish_block(&mut self) {
        for tile in &mut self.buffers {
            let mut context = Md5::new();
            for buffer in tile.iter() {
                context.update(buffer);
            }
            self.outer.update(context.finalize());
            for buffer in tile.iter_mut() {
                buffer.clear();
            }
        }
    }

    /// Finalize the digest.
    ///
    /// # Panics
    ///
    /// Panics if the hashed rows do not cover the declared height.
    #[must_use]
    pub fn finish(mut self) -> [u8; 16] {
        assert_eq!(
            self.next_row, self.height,
            "digest rows do not cover the image height"
        );
        if self.height == 0 {
            return self.outer.finalize().into();
        }
        self.finish_block();
        self.outer.finalize().into()
    }
}

/// Compute `RawDataUniqueID`.
///
/// Crop arguments are `(numerator, denominator)` in DNG tag order. Non-empty
/// opcode lists contribute their DNG SDK fingerprints in
/// `(OpcodeList1, OpcodeList2, OpcodeList3)` order.
///
/// # Errors
///
/// Returns [`crate::opcodes::OpcodeError`] when an opcode list is malformed.
pub fn raw_data_unique_id(
    digest: &[u8; 16],
    unique_camera_model: &str,
    crop_size: [(u32, u32); 2],
    crop_origin: [(u32, u32); 2],
    opcode_lists: [Option<&[u8]>; 3],
) -> Result<[u8; 16], crate::opcodes::OpcodeError> {
    let mut printer = Md5::new();
    printer.update(digest);
    printer.update(unique_camera_model.as_bytes());
    for (numerator, denominator) in [
        crop_size[0],
        crop_size[1],
        crop_origin[0],
        crop_origin[1],
        (0, 1),
        (0, 1),
        (1, 1),
        (1, 1),
    ] {
        printer.update(numerator.to_le_bytes());
        printer.update(denominator.to_le_bytes());
    }
    for blob in opcode_lists
        .into_iter()
        .flatten()
        .filter(|blob| !blob.is_empty())
    {
        printer.update(crate::opcodes::opcode_list_fingerprint(blob)?);
    }
    Ok(printer.finalize().into())
}
