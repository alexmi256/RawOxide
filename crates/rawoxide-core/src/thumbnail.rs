//! Preview picture sources for the IFD0 JPEG thumbnail.
//!
//! `synthetic` uses the built-in gradient, `random` tries Wikimedia Commons
//! and falls back to deterministic noise, and a filesystem path is decoded,
//! center-cropped to the preview aspect and Lanczos-resized.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use image::imageops::FilterType;
use thiserror::Error;

use crate::cover::synthetic_rgb;
use crate::metadata::Rng64;

const RANDOM_URL: &str = "https://commons.wikimedia.org/wiki/Special:Random/Image";
const FILEPATH_URL: &str = "https://commons.wikimedia.org/wiki/Special:FilePath/";
const USER_AGENT: &str = "ImageSteg-DNG/1.0 (random-thumbnail fetcher)";
const MAX_DOWNLOAD: u64 = 50 * 1024 * 1024;
const ALLOWED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

/// Preview picture source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewSource<'a> {
    /// Built-in gradient.
    #[default]
    Synthetic,
    /// Commons photo, with offline noise fallback.
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

    /// Commons fetch failed.
    #[error("cannot fetch thumbnail: {0}")]
    Network(String),

    /// Image is too large for a preview buffer.
    #[error("thumbnail is too large")]
    TooLarge,
}

/// Prepared preview pixels plus the human-readable source label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailImage {
    /// Interleaved RGB8 pixels, `width * height * 3` bytes.
    pub pixels: Vec<u8>,
    /// Resolved source label (`synthetic-gradient`, `commons:<file>`, ...).
    pub label: String,
}

/// Prepare RGB8 pixels for the preview.
///
/// # Errors
///
/// Returns [`ThumbnailError`] for invalid dimensions, unreadable/undecodable
/// files, or oversized buffers. `Random` falls back to deterministic noise on
/// fetch/decode failures, but can still return [`ThumbnailError::TooLarge`].
pub fn rgb(
    source: PreviewSource<'_>,
    width: u32,
    height: u32,
    seed: Option<u64>,
) -> Result<Vec<u8>, ThumbnailError> {
    rgb_with_label(source, width, height, seed).map(|image| image.pixels)
}

/// Prepare preview pixels and report the resolved source label.
///
/// # Errors
///
/// See [`rgb`].
pub fn rgb_with_label(
    source: PreviewSource<'_>,
    width: u32,
    height: u32,
    seed: Option<u64>,
) -> Result<ThumbnailImage, ThumbnailError> {
    if width == 0 || height == 0 {
        return Err(ThumbnailError::InvalidGeometry { width, height });
    }
    match source {
        PreviewSource::Synthetic => {
            let pixels = synthetic_rgb(
                usize::try_from(width).unwrap_or(usize::MAX),
                usize::try_from(height).unwrap_or(usize::MAX),
                seed,
            )
            .ok_or(ThumbnailError::TooLarge)?;
            Ok(ThumbnailImage {
                pixels,
                label: "synthetic-gradient".to_owned(),
            })
        }
        PreviewSource::Random => {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build();
            random_rgb_with_label(width, height, seed, |target_width| {
                fetch_commons(&agent, target_width)
            })
        }
        PreviewSource::File(path) => {
            let label = path.display().to_string();
            let data = std::fs::read(path).map_err(|source| ThumbnailError::Io {
                path: label.clone(),
                source,
            })?;
            Ok(ThumbnailImage {
                pixels: prepare_bytes(&data, &label, width, height)?,
                label: format!("file:{label}"),
            })
        }
    }
}

fn random_rgb_with_label<F>(
    width: u32,
    height: u32,
    seed: Option<u64>,
    mut fetch: F,
) -> Result<ThumbnailImage, ThumbnailError>
where
    F: FnMut(u32) -> Result<(Vec<u8>, String), ThumbnailError>,
{
    for _ in 0..3 {
        if let Ok((data, source)) = fetch(width)
            && let Ok(pixels) = prepare_bytes(&data, &source, width, height)
        {
            return Ok(ThumbnailImage {
                pixels,
                label: format!("commons:{source}"),
            });
        }
    }
    Ok(ThumbnailImage {
        pixels: noise_rgb(width, height, seed)?,
        label: "synthetic-noise-fallback".to_owned(),
    })
}

/// Deterministic offline noise preview.
///
/// # Errors
///
/// Returns [`ThumbnailError::TooLarge`] when the buffer size overflows.
#[allow(clippy::cast_possible_truncation)]
pub fn noise_rgb(width: u32, height: u32, seed: Option<u64>) -> Result<Vec<u8>, ThumbnailError> {
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

fn fetch_commons(agent: &ureq::Agent, width: u32) -> Result<(Vec<u8>, String), ThumbnailError> {
    let response = agent
        .get(RANDOM_URL)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| ThumbnailError::Network(error.to_string()))?;
    let filename = parse_commons_target(response.get_url())?;
    let url = filepath_url(&filename, width);
    let response = agent
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| ThumbnailError::Network(error.to_string()))?;
    let mut data = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD)
        .read_to_end(&mut data)
        .map_err(|error| ThumbnailError::Network(error.to_string()))?;
    if data.len() as u64 >= MAX_DOWNLOAD {
        return Err(ThumbnailError::Network(
            "download exceeds size limit".to_owned(),
        ));
    }
    if data.len() < 1024 {
        return Err(ThumbnailError::Network("empty download".to_owned()));
    }
    Ok((data, filename))
}

fn parse_commons_target(final_url: &str) -> Result<String, ThumbnailError> {
    let marker = "/wiki/File:";
    let path = final_url
        .split(marker)
        .nth(1)
        .ok_or_else(|| ThumbnailError::Network(format!("unexpected redirect {final_url:?}")))?;
    let raw_name = path.split(['?', '#']).next().unwrap_or(path);
    let filename = urlencoding::decode(raw_name)
        .map_err(|error| ThumbnailError::Network(error.to_string()))?
        .into_owned();
    let extension = Path::new(&filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(ThumbnailError::Network(format!(
            "unsupported media type {extension:?}"
        )));
    }
    Ok(filename)
}

fn filepath_url(filename: &str, width: u32) -> String {
    let target_width = width.clamp(64, 4096);
    format!(
        "{FILEPATH_URL}{}?width={target_width}",
        urlencoding::encode(filename)
    )
}

#[allow(clippy::cast_possible_truncation)]
fn prepare_bytes(
    data: &[u8],
    label: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ThumbnailError> {
    let decoded = image::load_from_memory(data).map_err(|error| ThumbnailError::Decode {
        path: label.to_owned(),
        error,
    })?;
    let image = if decoded.color().has_alpha() {
        let rgba = decoded.to_rgba8();
        image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let alpha = u16::from(pixel[3]);
            let blend =
                |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
            image::Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])])
        })
    } else {
        decoded.to_rgb8()
    };
    let (source_width, source_height) = image.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err(ThumbnailError::Decode {
            path: label.to_owned(),
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
    let cropped = image::imageops::crop_imm(&image, x, y, crop_width, crop_height).to_image();
    let resized = image::imageops::resize(&cropped, width, height, FilterType::Lanczos3);
    Ok(resized.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_uses_fetched_image_when_available() {
        let image = image::RgbImage::from_fn(20, 10, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 20) as u8, 90])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let pixels = random_rgb_with_label(8, 6, None, |_| Ok((bytes.clone(), "test".to_owned())))
            .unwrap()
            .pixels;
        assert_eq!(pixels, prepare_bytes(&bytes, "test", 8, 6).unwrap());
        let labeled =
            random_rgb_with_label(8, 6, None, |_| Ok((bytes.clone(), "Photo.JPG".to_owned())))
                .unwrap();
        assert_eq!(labeled.label, "commons:Photo.JPG");
        assert_eq!(labeled.pixels, pixels);
    }

    #[test]
    fn commons_target_parsing_handles_names_queries_and_extensions() {
        assert_eq!(
            parse_commons_target("https://commons.wikimedia.org/wiki/File:My%20Photo.JPG?x=1")
                .unwrap(),
            "My Photo.JPG"
        );
        assert_eq!(
            parse_commons_target("https://commons.wikimedia.org/wiki/File:%C3%84.png#frag")
                .unwrap(),
            "Ä.png"
        );
        assert!(parse_commons_target("https://commons.wikimedia.org/wiki/Category:x.jpg").is_err());
        assert!(parse_commons_target("https://commons.wikimedia.org/wiki/File:map.svg").is_err());
        assert!(parse_commons_target("https://commons.wikimedia.org/wiki/File:noext").is_err());
        assert_eq!(
            filepath_url("My Photo.JPG", 128),
            "https://commons.wikimedia.org/wiki/Special:FilePath/My%20Photo.JPG?width=128"
        );
        assert_eq!(
            filepath_url("x.png", 1),
            "https://commons.wikimedia.org/wiki/Special:FilePath/x.png?width=64"
        );
        assert_eq!(
            filepath_url("x.png", 10_000),
            "https://commons.wikimedia.org/wiki/Special:FilePath/x.png?width=4096"
        );
    }

    #[test]
    fn random_retries_three_times_then_falls_back() {
        let mut attempts = 0;
        let pixels = random_rgb_with_label(8, 6, Some(3), |_| {
            attempts += 1;
            Err(ThumbnailError::Network("offline".to_owned()))
        })
        .unwrap();
        assert_eq!(attempts, 3);
        assert_eq!(pixels.pixels, noise_rgb(8, 6, Some(3)).unwrap());
        assert_eq!(pixels.label, "synthetic-noise-fallback");
    }

    #[test]
    fn transparent_images_composite_over_white() {
        let rgba = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let pixels = prepare_bytes(&bytes, "transparent", 4, 4).unwrap();
        assert!(pixels.iter().all(|value| *value == 255));
    }

    #[test]
    fn random_falls_back_to_noise_when_fetching_fails() {
        let pixels = random_rgb_with_label(8, 6, Some(9), |_| {
            Err(ThumbnailError::Network("offline".to_owned()))
        })
        .unwrap();
        assert_eq!(pixels.pixels, noise_rgb(8, 6, Some(9)).unwrap());
        assert_eq!(pixels.label, "synthetic-noise-fallback");
    }
}
