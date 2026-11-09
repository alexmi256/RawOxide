//! DNG `OpcodeList2` support, starting with the GainMap opcode.
//!
//! The list is big-endian: a count followed by `(id, version, flags, size,
//! parameters)` records. Each GainMap parameter block carries the DNG 1.3
//! header (area, plane metadata, 30x40 map, spacing) and big-endian `f32`
//! gains. Fingerprints for `RawDataUniqueID` re-emit the list in host
//! (little-endian) order, matching the DNG SDK.

#![allow(
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    clippy::chunks_exact_to_as_chunks,
    clippy::doc_markdown
)]

use thiserror::Error;

use crate::metadata::Rng64;
use crate::profile::GainMapConfig;

/// GainMap opcode id.
pub const GAIN_MAP_OPCODE: u32 = 9;
/// GainMap opcode version.
pub const GAIN_MAP_VERSION: u32 = 0x0103_0000;
/// On-disk gain map points (vertical, horizontal).
pub const GAIN_MAP_POINTS: (usize, usize) = (30, 40);
/// CFA phase order used by the Pixel XL samples: B, G, G, R (BGGR).
pub const GAIN_MAP_PHASES: [(u32, u32); 4] = [(1, 1), (1, 0), (0, 1), (0, 0)];
const HEADER_SIZE: usize = 10 * 4 + 4 * 8 + 4;

/// Opcode parsing/building failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OpcodeError {
    /// The list or parameter block is truncated.
    #[error("opcode list is truncated: {0}")]
    Truncated(String),
    /// Unsupported opcode id.
    #[error("unsupported opcode id {0}")]
    Unsupported(u32),
    /// Unsupported GainMap version.
    #[error("unexpected GainMap version 0x{0:08x}")]
    Version(u32),
    /// Implausible opcode count.
    #[error("implausible opcode count {0}")]
    Count(u32),
    /// Trailing bytes or size mismatch.
    #[error("malformed opcode list: {0}")]
    Malformed(String),
    /// Point count overflows or is implausibly large.
    #[error("implausible GainMap point count")]
    PointCount,
}

/// One parsed GainMap opcode.
#[derive(Debug, Clone, PartialEq)]
pub struct GainMap {
    /// Top row.
    pub top: u32,
    /// Left column.
    pub left: u32,
    /// Bottom row.
    pub bottom: u32,
    /// Right column.
    pub right: u32,
    /// Map plane index.
    pub plane: u32,
    /// Map plane count.
    pub planes: u32,
    /// Row pitch.
    pub row_pitch: u32,
    /// Column pitch.
    pub col_pitch: u32,
    /// Vertical points.
    pub points_v: u32,
    /// Horizontal points.
    pub points_h: u32,
    /// Vertical spacing.
    pub spacing_v: f64,
    /// Horizontal spacing.
    pub spacing_h: f64,
    /// Vertical origin.
    pub origin_v: f64,
    /// Horizontal origin.
    pub origin_h: f64,
    /// Gain map planes.
    pub map_planes: u32,
    /// Row-major gains.
    pub gains: Vec<f64>,
}

/// Build an `OpcodeList2` with four GainMap opcodes.
///
/// # Panics
///
/// Panics if the profile grid is not a valid `4 x V x H` grid.
#[must_use]
pub fn build_opcode_list2(
    width: usize,
    height: usize,
    seed: Option<u64>,
    config: &GainMapConfig,
) -> Vec<u8> {
    let gains = generate_gainmaps(seed, config);
    let mut out = Vec::new();
    out.extend_from_slice(&4_u32.to_be_bytes());
    for (plane, &(top, left)) in GAIN_MAP_PHASES.iter().enumerate() {
        let params = gainmap_params(width, height, top, left, &gains[plane]);
        out.extend_from_slice(&GAIN_MAP_OPCODE.to_be_bytes());
        out.extend_from_slice(&GAIN_MAP_VERSION.to_be_bytes());
        out.extend_from_slice(&0_u32.to_be_bytes());
        out.extend_from_slice(&u32::try_from(params.len()).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&params);
    }
    out
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn bilinear(grid: &[f64], rows: usize, cols: usize, dst_v: usize, dst_h: usize) -> Vec<f64> {
    assert!(rows >= 2 && cols >= 2 && grid.len() == rows * cols);
    let mut out = Vec::with_capacity(dst_v * dst_h);
    for y in 0..dst_v {
        let sy = if dst_v == 1 {
            0.0
        } else {
            y as f64 * (rows - 1) as f64 / (dst_v - 1) as f64
        };
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(rows - 1);
        let wy = sy - y0 as f64;
        for x in 0..dst_h {
            let sx = if dst_h == 1 {
                0.0
            } else {
                x as f64 * (cols - 1) as f64 / (dst_h - 1) as f64
            };
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(cols - 1);
            let wx = sx - x0 as f64;
            let top = grid[y0 * cols + x0] * (1.0 - wx) + grid[y0 * cols + x1] * wx;
            let bottom = grid[y1 * cols + x0] * (1.0 - wx) + grid[y1 * cols + x1] * wx;
            out.push(top * (1.0 - wy) + bottom * wy);
        }
    }
    out
}

fn normal(rng: &mut Rng64) -> f64 {
    let u1 = rng.uniform(f64::MIN_POSITIVE, 1.0);
    let u2 = rng.uniform(0.0, 1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

fn generate_gainmaps(seed: Option<u64>, config: &GainMapConfig) -> Vec<Vec<f64>> {
    let mut rng = Rng64::new(Some(seed.unwrap_or(0)));
    let mut output = Vec::with_capacity(4);
    let (points_v, points_h) = GAIN_MAP_POINTS;
    for plane in 0..4 {
        let base = &config.planes
            [plane * config.rows * config.cols..(plane + 1) * config.rows * config.cols];
        let mut gains = bilinear(base, config.rows, config.cols, points_v, points_h);
        let noise: Vec<f64> = (0..config.rows * config.cols)
            .map(|_| normal(&mut rng))
            .collect();
        let mut noise = bilinear(&noise, config.rows, config.cols, points_v, points_h);
        let mean = noise.iter().sum::<f64>() / noise.len() as f64;
        let variance = noise
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f64>()
            / noise.len() as f64;
        let std = variance.sqrt();
        if std > 0.0 {
            for value in &mut noise {
                *value /= std;
            }
        }
        for (gain, noise) in gains.iter_mut().zip(noise) {
            *gain = (*gain + config.jitter * noise).clamp(config.clip.0, config.clip.1);
        }
        output.push(gains);
    }
    output
}

#[allow(clippy::cast_possible_truncation)]
fn gainmap_params(width: usize, height: usize, top: u32, left: u32, gains: &[f64]) -> Vec<u8> {
    let (points_v, points_h) = GAIN_MAP_POINTS;
    assert_eq!(gains.len(), points_v * points_h);
    let mut out = Vec::with_capacity(HEADER_SIZE + gains.len() * 4);
    for value in [
        top,
        left,
        u32::try_from(height).unwrap_or(u32::MAX),
        u32::try_from(width).unwrap_or(u32::MAX),
        0,
        1,
        2,
        2,
        u32::try_from(points_v).unwrap_or(u32::MAX),
        u32::try_from(points_h).unwrap_or(u32::MAX),
    ] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    let spacing_v = 1.0 / (points_v.saturating_sub(1).max(1) as f64);
    let spacing_h = 1.0 / (points_h.saturating_sub(1).max(1) as f64);
    for value in [spacing_v, spacing_h, 0.0, 0.0] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(&1_u32.to_be_bytes());
    for gain in gains {
        out.extend_from_slice(&(*gain as f32).to_be_bytes());
    }
    out
}

fn u32_at(blob: &[u8], at: usize) -> Result<u32, OpcodeError> {
    let bytes = blob
        .get(at..at + 4)
        .ok_or_else(|| OpcodeError::Truncated("u32 field".to_owned()))?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn f64_at(blob: &[u8], at: usize) -> Result<f64, OpcodeError> {
    let bytes = blob
        .get(at..at + 8)
        .ok_or_else(|| OpcodeError::Truncated("f64 field".to_owned()))?;
    Ok(f64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

/// Parse an `OpcodeList2` into its GainMap records.
///
/// # Errors
///
/// Returns [`OpcodeError`] for truncated, unsupported or malformed lists.
pub fn parse_opcode_list2(blob: &[u8]) -> Result<Vec<GainMap>, OpcodeError> {
    let count = u32_at(blob, 0)?;
    if count > 64 {
        return Err(OpcodeError::Count(count));
    }
    let mut offset = 4;
    let mut out = Vec::new();
    for index in 0..count {
        if offset + 16 > blob.len() {
            return Err(OpcodeError::Truncated(format!("opcode {index} header")));
        }
        let opcode = u32_at(blob, offset)?;
        let version = u32_at(blob, offset + 4)?;
        let size = u32_at(blob, offset + 12)? as usize;
        offset += 16;
        if offset + size > blob.len() {
            return Err(OpcodeError::Truncated(format!("opcode {index} parameters")));
        }
        if opcode != GAIN_MAP_OPCODE {
            return Err(OpcodeError::Unsupported(opcode));
        }
        if version != GAIN_MAP_VERSION {
            return Err(OpcodeError::Version(version));
        }
        let params = &blob[offset..offset + size];
        offset += size;
        if size < HEADER_SIZE {
            return Err(OpcodeError::Truncated(format!("opcode {index} GainMap")));
        }
        let points_v = u32_at(params, 32)? as usize;
        let points_h = u32_at(params, 36)? as usize;
        let map_planes = u32_at(params, 72)? as usize;
        let values = points_v
            .checked_mul(points_h)
            .and_then(|count| count.checked_mul(map_planes))
            .filter(|count| *count <= 16_777_216)
            .ok_or(OpcodeError::PointCount)?;
        let expected = values.checked_mul(4).ok_or(OpcodeError::PointCount)?;
        if size != HEADER_SIZE + expected {
            return Err(OpcodeError::Malformed(format!(
                "opcode {index}: {size} bytes, expected {}",
                HEADER_SIZE + expected
            )));
        }
        let mut gains = Vec::with_capacity(values);
        for chunk in params[HEADER_SIZE..].as_chunks::<4>().0 {
            gains.push(f64::from(f32::from_be_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3],
            ])));
        }
        out.push(GainMap {
            top: u32_at(params, 0)?,
            left: u32_at(params, 4)?,
            bottom: u32_at(params, 8)?,
            right: u32_at(params, 12)?,
            plane: u32_at(params, 16)?,
            planes: u32_at(params, 20)?,
            row_pitch: u32_at(params, 24)?,
            col_pitch: u32_at(params, 28)?,
            points_v: u32_at(params, 32)?,
            points_h: u32_at(params, 36)?,
            spacing_v: f64_at(params, 40)?,
            spacing_h: f64_at(params, 48)?,
            origin_v: f64_at(params, 56)?,
            origin_h: f64_at(params, 64)?,
            map_planes: u32_at(params, 72)?,
            gains,
        });
    }
    if offset != blob.len() {
        return Err(OpcodeError::Malformed(format!(
            "{} trailing bytes",
            blob.len() - offset
        )));
    }
    Ok(out)
}

/// DNG SDK fingerprint stream for `RawDataUniqueID` (little-endian).
///
/// # Errors
///
/// Returns [`OpcodeError`] for malformed or unsupported lists.
pub fn opcode_list_fingerprint(blob: &[u8]) -> Result<Vec<u8>, OpcodeError> {
    let count = u32_at(blob, 0)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 64 {
        return Err(OpcodeError::Count(count));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 4;
    for index in 0..count {
        if offset + 16 > blob.len() {
            return Err(OpcodeError::Truncated(format!("opcode {index} header")));
        }
        let opcode = u32_at(blob, offset)?;
        let version = u32_at(blob, offset + 4)?;
        let flags = u32_at(blob, offset + 8)?;
        let size = u32_at(blob, offset + 12)?;
        offset += 16;
        let size_usize = size as usize;
        if offset + size_usize > blob.len() {
            return Err(OpcodeError::Truncated(format!("opcode {index} parameters")));
        }
        let params = &blob[offset..offset + size_usize];
        offset += size_usize;
        if opcode != GAIN_MAP_OPCODE {
            return Err(OpcodeError::Unsupported(opcode));
        }
        if size_usize < HEADER_SIZE {
            return Err(OpcodeError::Truncated(format!("opcode {index} GainMap")));
        }
        out.extend_from_slice(&opcode.to_le_bytes());
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        for at in (0..40).step_by(4) {
            out.extend_from_slice(&u32_at(params, at)?.to_le_bytes());
        }
        for at in [40, 48, 56, 64] {
            out.extend_from_slice(&f64_at(params, at)?.to_le_bytes());
        }
        out.extend_from_slice(&u32_at(params, 72)?.to_le_bytes());
        let body = &params[HEADER_SIZE..];
        if !body.len().is_multiple_of(4) {
            return Err(OpcodeError::Malformed(
                "GainMap map data is not a whole number of floats".to_owned(),
            ));
        }
        for chunk in body.as_chunks::<4>().0 {
            out.extend_from_slice(
                &f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]).to_le_bytes(),
            );
        }
    }
    if offset != blob.len() {
        return Err(OpcodeError::Malformed(format!(
            "{} trailing bytes",
            blob.len() - offset
        )));
    }
    Ok(out)
}
