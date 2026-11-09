//! Preview picture sources for the IFD0 JPEG thumbnail.
//!
//! `synthetic` uses the built-in gradient, `random` uses deterministic noise
//! (the offline fallback of the Python provider; no network access), and a
//! filesystem path is decoded, center-cropped to the preview aspect and
//! Lanczos-resized.

use std::path::Path;

use image::imageops::FilterType;
use thiserror::Error;

use crate::cover::synthetic_rgb;
use crate::metadata::Rng64;

/// Preview picture source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewSource<'a> {
    /// Built-in gradient.
    #[default]
    Synthetic,
    /// Deterministic offline noise.
    Random,
    /// User-supplied image file.
    File(&'a Path),
}

/// Preview preparation failures.
#[derive(Debug, Error)]
pub enum ThumbnailError {
    /// Zero target dimensions.
    #[error("thumbnail dimensions must be positive (got {width}x{height})")]
    InvalidGeometry {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },

    /// Source file could not be read.
    #[error("cannot read thumbnail {path}: {source}")]
    Io {
        /// Source path.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },

    /// Source file could not be decoded.
    #[error("cannot decode thumbnail {path}: {error}")]
    Decode {
        /// Source path.
        path: String,
        /// Underlying error.
        error: image::ImageError,
    },

    /// Image is too large for a preview buffer.
    #[error("thumbnail is too large")]
    TooLarge,
}

/// Prepare RGB8 pixels for the preview.
///
/// # Errors
///
/// Returns [`ThumbnailError`] for invalid dimensions, unreadable/undecodable
/// files, or oversized buffers.
#[allow(clippy::cast_possible_truncation)]
pub fn rgb(
    source: PreviewSource<'_>,
    width: u32,
    height: u32,
    seed: Option<u64>,
) -> Result<Vec<u8>, ThumbnailError> {
    if width == 0 || height == 0 {
        return Err(ThumbnailError::InvalidGeometry { width, height });
    }
    match source {
        PreviewSource::Synthetic => synthetic_rgb(
            usize::try_from(width).unwrap_or(usize::MAX),
            usize::try_from(height).unwrap_or(usize::MAX),
            seed,
        )
        .ok_or(ThumbnailError::TooLarge),
        PreviewSource::Random => {
            let count = (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or(ThumbnailError::TooLarge)?;
            let mut rng = Rng64::new(Some(seed.unwrap_or(0).wrapping_add(31_337)));
            let mut pixels = Vec::with_capacity(count);
            for _ in 0..count {
                pixels.push(rng.below(256) as u8);
            }
            Ok(pixels)
        }
        PreviewSource::File(path) => {
            let label = path.display().to_string();
            let image = image::ImageReader::open(path)
                .map_err(|source| ThumbnailError::Io {
                    path: label.clone(),
                    source,
                })?
                .with_guessed_format()
                .map_err(|source| ThumbnailError::Io {
                    path: label.clone(),
                    source,
                })?
                .decode()
                .map_err(|error| ThumbnailError::Decode {
                    path: label.clone(),
                    error,
                })?
                .to_rgb8();
            let (source_width, source_height) = image.dimensions();
            if source_width == 0 || source_height == 0 {
                return Err(ThumbnailError::Decode {
                    path: label,
                    error: image::ImageError::Limits(image::error::LimitError::from_kind(
                        image::error::LimitErrorKind::DimensionError,
                    )),
                });
            }
            let target = f64::from(width) / f64::from(height);
            let source_aspect = f64::from(source_width) / f64::from(source_height);
            let (crop_width, crop_height) = if source_aspect > target {
                let crop_width = (f64::from(source_height) * target).round() as u32;
                (crop_width.max(1).min(source_width), source_height)
            } else {
                let crop_height = (f64::from(source_width) / target).round() as u32;
                (source_width, crop_height.max(1).min(source_height))
            };
            let x = (source_width - crop_width) / 2;
            let y = (source_height - crop_height) / 2;
            let cropped =
                image::imageops::crop_imm(&image, x, y, crop_width, crop_height).to_image();
            let resized = image::imageops::resize(&cropped, width, height, FilterType::Lanczos3);
            Ok(resized.into_raw())
        }
    }
}
