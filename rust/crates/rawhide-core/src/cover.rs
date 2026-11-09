//! Plain TIFF cover generation (`gen-tiff`).
//!
//! Produces an uncompressed 8-bit RGB TIFF with a synthetic gradient/noise
//! cover, matching the Python `generate_tiff` helper's role as a
//! testing/comparison image outside the stego container.

#![allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]

use std::path::Path;

use thiserror::Error;

use crate::metadata::Rng64;
use crate::tiff::{Entry, IfdBuilder};

const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_PLANAR_CONFIGURATION: u16 = 284;
const TAG_SOFTWARE: u16 = 305;

/// Cover generation failures.
#[derive(Debug, Error)]
pub enum CoverError {
    /// Zero width or height.
    #[error("width and height must be positive (got {width}x{height})")]
    InvalidGeometry {
        /// Width.
        width: usize,
        /// Height.
        height: usize,
    },

    /// Pixel buffer or classic TIFF offsets do not fit.
    #[error("cover is too large for classic TIFF")]
    TooLarge,

    /// TIFF serialization failed.
    #[error(transparent)]
    Tiff(#[from] crate::tiff::Error),

    /// I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Write a plain 8-bit RGB TIFF cover.
///
/// # Errors
///
/// Returns [`CoverError`] for invalid geometry, oversized images, TIFF
/// serialization or I/O failures.
pub fn write_cover_tiff(
    path: impl AsRef<Path>,
    width: usize,
    height: usize,
    seed: Option<u64>,
    software: &str,
) -> Result<u64, CoverError> {
    if width == 0 || height == 0 {
        return Err(CoverError::InvalidGeometry { width, height });
    }
    let mut rng = Rng64::new(Some(seed.unwrap_or(0)));
    let mut pixels = Vec::with_capacity(
        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(CoverError::TooLarge)?,
    );
    let width_den = width.saturating_sub(1).max(1) as f32;
    let height_den = height.saturating_sub(1).max(1) as f32;
    for y in 0..height {
        for x in 0..width {
            let noise = |rng: &mut Rng64| rng.uniform(-9.0, 9.0) as f32;
            let r = 90.0 + 120.0 * (x as f32 / width_den) + noise(&mut rng);
            let g = 100.0 + 90.0 * (y as f32 / height_den) + noise(&mut rng);
            let b = 160.0 - 70.0 * (x as f32 / width_den) + noise(&mut rng);
            pixels.push(r.clamp(0.0, 255.0) as u8);
            pixels.push(g.clamp(0.0, 255.0) as u8);
            pixels.push(b.clamp(0.0, 255.0) as u8);
        }
    }
    let strip_offset = 8_u32;
    let ifd_offset = u32::try_from(pixels.len().checked_add(8).ok_or(CoverError::TooLarge)?)
        .map_err(|_| CoverError::TooLarge)?;
    let entries = vec![
        Entry::longs(TAG_IMAGE_WIDTH, &[u32::try_from(width).unwrap_or(u32::MAX)]),
        Entry::longs(
            TAG_IMAGE_LENGTH,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_BITS_PER_SAMPLE, &[8, 8, 8]),
        Entry::shorts(TAG_COMPRESSION, &[1]),
        Entry::shorts(TAG_PHOTOMETRIC, &[2]),
        Entry::longs(TAG_STRIP_OFFSETS, &[strip_offset]),
        Entry::shorts(TAG_SAMPLES_PER_PIXEL, &[3]),
        Entry::longs(
            TAG_ROWS_PER_STRIP,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::longs(
            TAG_STRIP_BYTE_COUNTS,
            &[u32::try_from(pixels.len()).map_err(|_| CoverError::TooLarge)?],
        ),
        Entry::shorts(TAG_PLANAR_CONFIGURATION, &[1]),
        Entry::ascii(TAG_SOFTWARE, software),
    ];
    let ifd = IfdBuilder::from_entries(entries).build(ifd_offset)?;
    let mut file = Vec::with_capacity(ifd_offset as usize + ifd.len());
    file.extend_from_slice(b"II*\0");
    file.extend_from_slice(&ifd_offset.to_le_bytes());
    file.extend_from_slice(&pixels);
    file.extend_from_slice(&ifd);
    std::fs::write(path.as_ref(), &file)?;
    Ok(std::fs::metadata(path.as_ref())?.len())
}
