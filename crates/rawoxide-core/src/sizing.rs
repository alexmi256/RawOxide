//! Automatic configuration: pick the smallest fitting geometry.
//!
//! Sizing policy: walk preset geometries, or compute an exact-fit 4:3 frame
//! with `auto_size`, choosing
//! the fewest LSB planes that hold the payload. Sizes in errors are reported
//! in kilobytes.

use std::fmt::Write as _;

use thiserror::Error;

use crate::codec::{capacity_bytes, capacity_bytes_total, format_kb, format_kb_hi, format_kb_lo};
use crate::framing::PayloadFrame;
use crate::profile::{
    AUTO_MIN_H, AUTO_MIN_W, MAX_FRAMES, MAX_LSB_PLANES, PIXELSHIFT_H, PIXELSHIFT_W,
    PRESET_GEOMETRIES, SUPPORTED_BIT_DEPTHS, top_planes,
};

/// Raw layout for sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizingMode {
    /// Demosaiced RGB (three samples per pixel).
    #[default]
    Linear,
    /// Bayer mosaic (one sample per pixel).
    Cfa,
}

impl SizingMode {
    /// CLI name (`linear`/`cfa`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Cfa => "cfa",
        }
    }

    /// Parse a CLI mode name.
    ///
    /// # Errors
    ///
    /// Returns [`SizingError::InvalidMode`] for anything else.
    #[allow(clippy::result_large_err)]
    pub fn parse(text: &str) -> Result<Self, SizingError> {
        match text {
            "linear" => Ok(Self::Linear),
            "cfa" => Ok(Self::Cfa),
            _ => Err(SizingError::InvalidMode(text.to_owned())),
        }
    }

    /// Samples per pixel.
    #[must_use]
    pub const fn samples_per_pixel(self) -> u64 {
        match self {
            Self::Linear => 3,
            Self::Cfa => 1,
        }
    }
}

/// Inputs to [`recommend`]; `None` means autodetect.
#[derive(Debug, Clone, Copy)]
pub struct RecommendOptions<'a> {
    /// Explicit width.
    pub width: Option<usize>,
    /// Explicit height.
    pub height: Option<usize>,
    /// Explicit mode (defaults to linear).
    pub mode: Option<SizingMode>,
    /// Explicit bit depth (defaults to 16).
    pub bit_depth: Option<u8>,
    /// Explicit LSB planes (defaults to the smallest fitting count).
    pub lsb_planes: Option<u8>,
    /// Compute minimal dimensions instead of using presets.
    pub auto_size: bool,
    /// Full-resolution raw frames.
    pub frames: u8,
    /// Profile plane ceiling.
    pub max_planes: Option<u8>,
    /// Camera sensor geometries tried first with `auto_size`.
    pub sensor_sizes: &'a [(usize, usize)],
    /// Fill the frame with payload bits before growing the geometry.
    pub prefer_max_planes: bool,
}

impl Default for RecommendOptions<'_> {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            mode: None,
            bit_depth: None,
            lsb_planes: None,
            auto_size: false,
            frames: 1,
            max_planes: None,
            sensor_sizes: &[],
            prefer_max_planes: false,
        }
    }
}

/// Selected configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedConfig {
    /// Frame width.
    pub width: usize,
    /// Frame height.
    pub height: usize,
    /// Raw layout.
    pub mode: SizingMode,
    /// Sample depth.
    pub bit_depth: u8,
    /// LSB planes.
    pub lsb_planes: u8,
    /// Raw frames.
    pub frames: u8,
    /// Payload capacity in bytes.
    pub capacity: u64,
    /// Names of fields chosen automatically.
    pub auto: Vec<&'static str>,
    /// Preset label (`demo`, `sensor`, `auto-size`, ...).
    pub preset: Option<&'static str>,
}

/// Sizing failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SizingError {
    /// Unknown mode name.
    #[error("mode must be 'linear' or 'cfa'")]
    InvalidMode(String),

    /// Unsupported depth.
    #[error("bit_depth must be one of {SUPPORTED_BIT_DEPTHS:?}")]
    InvalidBitDepth(u8),

    /// Planes outside `1..=16`.
    #[error("lsb_planes must be 1-{MAX_LSB_PLANES} (got {0})")]
    InvalidPlanes(u8),

    /// Frames outside `1..=8`.
    #[error("frames must be 1-{MAX_FRAMES} (got {0})")]
    InvalidFrames(u8),

    /// Profile plane ceiling outside `1..=16`.
    #[error("max_planes must be 1-{MAX_LSB_PLANES} (got {0})")]
    InvalidMaxPlanes(u8),

    /// Planes exceed the declared depth.
    #[error(
        "lsb_planes ({planes}) exceeds bit_depth ({depth}); payload values would \
         overflow the declared WhiteLevel. Use --bit-depth 16 (full-range uint16, \
         up to 16 planes) or fewer planes."
    )]
    PlanesExceedDepth {
        /// Requested planes.
        planes: u8,
        /// Requested depth.
        depth: u8,
    },

    /// Planes exceed the profile ceiling.
    #[error(
        "lsb_planes ({planes}) exceeds the profile maximum of {top} at {depth}-bit; \
         use fewer planes or a capacity level policy."
    )]
    PlanesExceedProfile {
        /// Requested planes.
        planes: u8,
        /// Profile ceiling.
        top: u8,
        /// Depth.
        depth: u8,
    },

    /// `auto_size` was combined with explicit dimensions.
    #[error(
        "--auto-size cannot be combined with explicit --width/--height \
         (it computes dimensions itself)"
    )]
    AutoSizeConflict,

    /// Zero or negative geometry.
    #[error("width and height must be positive (got {width}x{height})")]
    NonPositiveGeometry {
        /// Width.
        width: usize,
        /// Height.
        height: usize,
    },

    /// Explicit geometry cannot hold the payload.
    #[error(
        "payload {payload} exceeds capacity {capacity} for {width}x{height} {mode} \
         {depth}-bit with {planes} LSB plane(s) (max at this geometry with {top} \
         planes: {max_here}; largest supported configuration ({max_config}): \
         {global}); {hint}"
    )]
    ExplicitExceeds {
        /// Payload size, formatted.
        payload: String,
        /// Capacity at the request, formatted.
        capacity: String,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// Mode name.
        mode: &'static str,
        /// Depth.
        depth: u8,
        /// Planes.
        planes: u8,
        /// Depth ceiling.
        top: u8,
        /// Capacity at the depth ceiling, formatted.
        max_here: String,
        /// Largest supported config description.
        max_config: String,
        /// Global maximum, formatted.
        global: String,
        /// Remediation hint.
        hint: &'static str,
    },

    /// Capacity is exhausted even with the largest plane count.
    #[error(
        "payload {payload} exceeds capacity {capacity} for {width}x{height} {mode} \
         {depth}-bit even with {top} LSB planes (largest supported configuration \
         ({max_config}): {global}); {hint}"
    )]
    GeometryExceeds {
        /// Payload size, formatted.
        payload: String,
        /// Capacity at the depth ceiling, formatted.
        capacity: String,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// Mode name.
        mode: &'static str,
        /// Depth.
        depth: u8,
        /// Depth ceiling.
        top: u8,
        /// Largest supported config description.
        max_config: String,
        /// Global maximum, formatted.
        global: String,
        /// Remediation hint.
        hint: &'static str,
    },

    /// Nothing fits under the requested constraints.
    #[error(
        "payload {payload} exceeds maximum capacity {global} under the given \
         constraints (largest supported configuration ({max_config}): {absolute}){hint}. \
         Split the input across multiple DNGs."
    )]
    NothingFits {
        /// Payload size, formatted.
        payload: String,
        /// Capacity under constraints, formatted.
        global: String,
        /// Largest supported config description.
        max_config: String,
        /// Absolute maximum, formatted.
        absolute: String,
        /// Additional hint.
        hint: String,
    },
}

const EXPLICIT_HINT: &str = "Use a larger --width/--height, more --lsb-planes, more \
     --raw-frames, leave dimensions unset for auto-sizing, or split the input \
     across multiple DNGs.";

const PRESET_HINT: &str = "Use a larger --width/--height, leave dimensions unset for \
     auto-sizing, more --raw-frames, or split the input across multiple DNGs.";

/// Largest supported configuration description.
#[must_use]
pub fn max_config(mode: SizingMode, bit_depth: u8, frames: u8) -> String {
    let mut text = format!(
        "{PIXELSHIFT_W}x{PIXELSHIFT_H} {} {bit_depth}-bit, {} LSB planes",
        mode.as_str(),
        top_planes(bit_depth)
    );
    if frames > 1 {
        let _ = write!(text, " x {frames} frames");
    }
    text
}

fn capacity(width: usize, height: usize, mode: SizingMode, planes: u8, frames: u8) -> u64 {
    let samples = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(mode.samples_per_pixel());
    capacity_bytes_total(samples, planes, u64::from(frames))
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::result_large_err
)]
fn recommend_inner(
    payload_len: u64,
    options: &RecommendOptions<'_>,
) -> Result<RecommendedConfig, SizingError> {
    let mode_eff = options.mode.unwrap_or_default();
    let depth_eff = options.bit_depth.unwrap_or(16);
    if options.bit_depth.is_some() && !SUPPORTED_BIT_DEPTHS.contains(&depth_eff) {
        return Err(SizingError::InvalidBitDepth(depth_eff));
    }
    if let Some(planes) = options.lsb_planes
        && !(1..=MAX_LSB_PLANES).contains(&planes)
    {
        return Err(SizingError::InvalidPlanes(planes));
    }
    if !(1..=MAX_FRAMES).contains(&options.frames) {
        return Err(SizingError::InvalidFrames(options.frames));
    }
    if let Some(ceiling) = options.max_planes
        && !(1..=MAX_LSB_PLANES).contains(&ceiling)
    {
        return Err(SizingError::InvalidMaxPlanes(ceiling));
    }
    let top = top_planes(depth_eff).min(options.max_planes.unwrap_or(MAX_LSB_PLANES));
    if let Some(planes) = options.lsb_planes {
        if planes > depth_eff {
            return Err(SizingError::PlanesExceedDepth {
                planes,
                depth: depth_eff,
            });
        }
        if planes > top {
            return Err(SizingError::PlanesExceedProfile {
                planes,
                top,
                depth: depth_eff,
            });
        }
    }

    let factor = mode_eff.samples_per_pixel();
    let cap = |width: usize, height: usize, planes: u8| {
        capacity(width, height, mode_eff, planes, options.frames)
    };

    let mut auto: Vec<&'static str> = Vec::new();
    if options.mode.is_none() {
        auto.push("mode");
    }
    if options.bit_depth.is_none() {
        auto.push("bit_depth");
    }

    if let (Some(width), Some(height)) = (options.width, options.height) {
        if options.auto_size {
            return Err(SizingError::AutoSizeConflict);
        }
        if width == 0 || height == 0 {
            return Err(SizingError::NonPositiveGeometry { width, height });
        }
        if let Some(planes) = options.lsb_planes {
            let c = cap(width, height, planes);
            if payload_len <= c {
                return Ok(RecommendedConfig {
                    width,
                    height,
                    mode: mode_eff,
                    bit_depth: depth_eff,
                    lsb_planes: planes,
                    frames: options.frames,
                    capacity: c,
                    auto,
                    preset: None,
                });
            }
            let max_here = cap(width, height, top);
            let global = cap(PIXELSHIFT_W, PIXELSHIFT_H, top);
            return Err(SizingError::ExplicitExceeds {
                payload: format_kb_hi(payload_len),
                capacity: format_kb_lo(c),
                width,
                height,
                mode: mode_eff.as_str(),
                depth: depth_eff,
                planes,
                top,
                max_here: format_kb(max_here),
                max_config: max_config(mode_eff, depth_eff, options.frames),
                global: format_kb(global),
                hint: EXPLICIT_HINT,
            });
        }
        let plane_order: Vec<u8> = if options.prefer_max_planes {
            (1..=top).rev().collect()
        } else {
            (1..=top).collect()
        };
        for p in plane_order {
            let c = cap(width, height, p);
            if payload_len <= c {
                auto.push("lsb_planes");
                return Ok(RecommendedConfig {
                    width,
                    height,
                    mode: mode_eff,
                    bit_depth: depth_eff,
                    lsb_planes: p,
                    frames: options.frames,
                    capacity: c,
                    auto,
                    preset: None,
                });
            }
        }
        let max_here = cap(width, height, top);
        let global = cap(PIXELSHIFT_W, PIXELSHIFT_H, top);
        return Err(SizingError::GeometryExceeds {
            payload: format_kb_hi(payload_len),
            capacity: format_kb_lo(max_here),
            width,
            height,
            mode: mode_eff.as_str(),
            depth: depth_eff,
            top,
            max_config: max_config(mode_eff, depth_eff, options.frames),
            global: format_kb(global),
            hint: PRESET_HINT,
        });
    }

    // One-sided geometry: complete via the 4:3 GFX aspect.
    if options.width.is_some() || options.height.is_some() {
        if options.auto_size {
            return Err(SizingError::AutoSizeConflict);
        }
        let (width, height) = match (options.width, options.height) {
            (Some(width), None) => {
                auto.push("height");
                (width, round_div(width * 3, 4))
            }
            (None, Some(height)) => {
                auto.push("width");
                (round_div(height * 4, 3), height)
            }
            _ => unreachable!("both or neither handled above"),
        };
        if width == 0 || height == 0 {
            return Err(SizingError::NonPositiveGeometry { width, height });
        }
        let mut completed = *options;
        completed.width = Some(width);
        completed.height = Some(height);
        let mut result = recommend_inner(payload_len, &completed)?;
        let mut merged = result.auto;
        let label = *auto.last().expect("one-sided geometry adds a label");
        let position = merged
            .iter()
            .position(|field| *field == "lsb_planes")
            .unwrap_or(merged.len());
        merged.insert(position, label);
        result.auto = merged;
        return Ok(result);
    }

    let planes_list: Vec<u8> = if let Some(planes) = options.lsb_planes {
        vec![planes]
    } else if options.prefer_max_planes {
        (1..=top).rev().collect()
    } else {
        (1..=top).collect()
    };

    if options.auto_size {
        for &(sw, sh) in options.sensor_sizes {
            if sw > PIXELSHIFT_W || sh > PIXELSHIFT_H {
                continue;
            }
            for &p in &planes_list {
                let c = cap(sw, sh, p);
                if payload_len <= c {
                    auto.push("width");
                    auto.push("height");
                    if options.lsb_planes.is_none() {
                        auto.push("lsb_planes");
                    }
                    return Ok(RecommendedConfig {
                        width: sw,
                        height: sh,
                        mode: mode_eff,
                        bit_depth: depth_eff,
                        lsb_planes: p,
                        frames: options.frames,
                        capacity: c,
                        auto,
                        preset: Some("sensor"),
                    });
                }
            }
        }
        let need_bits = payload_len
            .saturating_add(PayloadFrame::HEADER_LEN as u64)
            .saturating_mul(8);
        for &p in &planes_list {
            let frames = u64::from(options.frames);
            let per_frame_bits = (need_bits as f64) / f64::from(p) / frames as f64;
            let need_px = (per_frame_bits / factor as f64).ceil();
            let width = ((need_px * 4.0 / 3.0).sqrt()).ceil();
            let width = width as u64;
            let height = (width as f64 * 3.0 / 4.0).ceil();
            let by_bits = ((need_bits as f64)
                / (frames as f64 * width as f64 * factor as f64 * f64::from(p)))
            .ceil();
            let mut width = width.max(AUTO_MIN_W as u64) as usize;
            let mut height = (height.max(by_bits) as u64).max(AUTO_MIN_H as u64) as usize;
            if width % 2 == 1 {
                width += 1;
            }
            if height % 2 == 1 {
                height += 1;
            }
            let mut fits = false;
            for _ in 0..8 {
                if cap(width, height, p) >= payload_len {
                    fits = true;
                    break;
                }
                height += 2;
            }
            if fits && width <= PIXELSHIFT_W && height <= PIXELSHIFT_H {
                auto.push("width");
                auto.push("height");
                if options.lsb_planes.is_none() {
                    auto.push("lsb_planes");
                }
                return Ok(RecommendedConfig {
                    width,
                    height,
                    mode: mode_eff,
                    bit_depth: depth_eff,
                    lsb_planes: p,
                    frames: options.frames,
                    capacity: cap(width, height, p),
                    auto,
                    preset: Some("auto-size"),
                });
            }
        }
    } else {
        for &(sw, sh, label) in &PRESET_GEOMETRIES {
            for &p in &planes_list {
                let c = cap(sw, sh, p);
                if payload_len <= c {
                    auto.push("width");
                    auto.push("height");
                    if options.lsb_planes.is_none() {
                        auto.push("lsb_planes");
                    }
                    return Ok(RecommendedConfig {
                        width: sw,
                        height: sh,
                        mode: mode_eff,
                        bit_depth: depth_eff,
                        lsb_planes: p,
                        frames: options.frames,
                        capacity: c,
                        auto,
                        preset: Some(label),
                    });
                }
            }
        }
    }

    let planes_max = options.lsb_planes.unwrap_or(top);
    let global = cap(PIXELSHIFT_W, PIXELSHIFT_H, planes_max);
    let absolute = cap(PIXELSHIFT_W, PIXELSHIFT_H, top);
    let mut hint = String::new();
    if options.lsb_planes.is_some_and(|planes| planes < top) {
        let _ = write!(
            hint,
            " (with {top} LSB planes the max would be {})",
            format_kb_lo(absolute)
        );
    }
    if mode_eff == SizingMode::Cfa {
        let linear = capacity_bytes(
            (PIXELSHIFT_W as u64) * (PIXELSHIFT_H as u64) * 3,
            MAX_LSB_PLANES,
        );
        let _ = write!(hint, " (linear mode max would be {})", format_kb_lo(linear));
    }
    if options.auto_size {
        hint.push_str(
            " (--auto-size already uses minimal dimensions; only more planes, more \
             --raw-frames, linear mode, or splitting the input help)",
        );
    }
    Err(SizingError::NothingFits {
        payload: format_kb_hi(payload_len),
        global: format_kb_lo(global),
        max_config: max_config(mode_eff, depth_eff, options.frames),
        absolute: format_kb_lo(absolute),
        hint,
    })
}

fn round_div(numerator: usize, denominator: usize) -> usize {
    // Round halves to even to keep the geometry stable.
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    match (remainder * 2).cmp(&denominator) {
        std::cmp::Ordering::Greater => quotient + 1,
        std::cmp::Ordering::Equal if quotient % 2 == 1 => quotient + 1,
        _ => quotient,
    }
}

/// Choose the smallest configuration that fits `payload_len` bytes.
///
/// # Errors
///
/// Returns [`SizingError`] for invalid options or when nothing fits.
#[allow(clippy::result_large_err)]
pub fn recommend(
    payload_len: u64,
    options: &RecommendOptions<'_>,
) -> Result<RecommendedConfig, SizingError> {
    recommend_inner(payload_len, options)
}

/// High-density option warnings (also see the README detection risks).
#[must_use]
pub fn risk_warnings(
    config: &RecommendedConfig,
    key: bool,
    display_name: Option<&str>,
) -> Vec<String> {
    let label = display_name.unwrap_or("GFX 100 II");
    let mut out = Vec::new();
    let planes = config.lsb_planes;
    let depth = config.bit_depth;
    if planes > 4 {
        if planes >= MAX_LSB_PLANES {
            out.push(format!(
                "warning: {planes} LSB planes leave no cover: every sample bit is \
                 payload, so the raw image renders as noise. Bit-plane statistics \
                 trivially expose this."
            ));
        } else if planes >= 9 {
            out.push(format!(
                "warning: {planes} LSB planes destroy most of the {depth}-bit cover \
                 signal; the raw image will look severely degraded/noise-like."
            ));
        } else {
            out.push(format!(
                "warning: {planes} LSB planes heavily degrade the cover image (only \
                 {} of {depth} bits per sample remain photographic).",
                depth - planes
            ));
        }
    }
    if config.frames > 1 {
        out.push(format!(
            "warning: {} full-resolution raw frames in one still DNG is \
             non-standard (burst/stack-style multi-image TIFFs exist, but no \
             {label} combiner output looks like this) and multiplies the file \
             size ~{}x.",
            config.frames, config.frames
        ));
    }
    if depth < 14 {
        out.push(format!(
            "warning: {depth}-bit is below every {label} option (14/16-bit); the \
             BitsPerSample tag alone marks the file as unusual for this camera story."
        ));
    }
    if !out.is_empty() && !key {
        out.push(
            "warning: high-risk options without --key leave the payload readable to \
             anyone extracting LSBs; consider encrypting."
                .to_owned(),
        );
    }
    out
}
