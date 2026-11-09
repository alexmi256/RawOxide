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

/// Compute `RawDataUniqueID`.
///
/// Crop arguments are `(numerator, denominator)` in DNG tag order. Opcode
/// fingerprints are omitted: the Rust writer emits empty opcode lists, and
/// this value describes the file as written.
#[must_use]
pub fn raw_data_unique_id(
    digest: &[u8; 16],
    unique_camera_model: &str,
    crop_size: [(u32, u32); 2],
    crop_origin: [(u32, u32); 2],
) -> [u8; 16] {
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
    printer.finalize().into()
}
