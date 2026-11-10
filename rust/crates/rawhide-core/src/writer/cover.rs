//! Synthetic raw cover generation.

use super::RawMode;

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
pub(super) fn synthetic_cover(
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    seed: u64,
    white_level: Option<u32>,
    headroom: u32,
) -> Vec<u16> {
    synthetic_cover_band(
        width,
        height,
        0,
        height,
        mode,
        bit_depth,
        seed,
        white_level,
        headroom,
    )
}

/// Generates synthetic raw covers band by band.
///
/// Per-column ramp and sine terms are computed once, so writing a frame costs
/// no trigonometric calls per pixel and reuses one band buffer across tiles.
pub(super) struct CoverGenerator {
    width: usize,
    total_height: usize,
    mode: RawMode,
    channels: usize,
    limit: f32,
    scale: f32,
    height_den: f32,
    gains: [f32; 3],
    columns: Vec<f32>,
}

impl CoverGenerator {
    #[allow(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        clippy::too_many_arguments
    )]
    pub(super) fn new(
        width: usize,
        total_height: usize,
        mode: RawMode,
        bit_depth: u16,
        seed: u64,
        white_level: Option<u32>,
        headroom: u32,
    ) -> Self {
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
        let height_den = total_height.saturating_sub(1).max(1) as f32;
        let phase = (seed % 6_283) as f32 * 0.001;
        let columns = (0..width)
            .map(|x| {
                1400.0 * (x as f32 / width_den)
                    + 500.0
                        * (std::f32::consts::TAU * x as f32 / width.max(1) as f32 * 3.0 + phase)
                            .sin()
            })
            .collect();
        Self {
            width,
            total_height,
            mode,
            channels: mode.channels(),
            limit: limit as f32,
            scale,
            height_den,
            gains: [1.0, 0.94, 0.82],
            columns,
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub(super) fn fill_band(&self, y_start: usize, rows: usize, cover: &mut [u16]) {
        debug_assert_eq!(cover.len(), self.width * rows * self.channels);
        debug_assert!(y_start + rows <= self.total_height);
        for local_y in 0..rows {
            let y = y_start + local_y;
            let y_term = 2600.0 * (y as f32 / self.height_den);
            for (x, &x_term) in self.columns.iter().enumerate() {
                let base = 7000.0 + y_term + x_term;
                let offset = (local_y * self.width + x) * self.channels;
                for channel in 0..self.channels {
                    let gain = if self.mode == RawMode::Cfa {
                        self.gains[match (y % 2, x % 2) {
                            (0, 0) => 0,
                            (0, 1) | (1, 0) => 1,
                            _ => 2,
                        }]
                    } else {
                        self.gains[channel]
                    };
                    let value = (base * gain * self.scale).clamp(0.0, self.limit) as u16;
                    cover[offset + channel] = value;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn synthetic_cover_band(
    width: usize,
    total_height: usize,
    y_start: usize,
    rows: usize,
    mode: RawMode,
    bit_depth: u16,
    seed: u64,
    white_level: Option<u32>,
    headroom: u32,
) -> Vec<u16> {
    let generator = CoverGenerator::new(
        width,
        total_height,
        mode,
        bit_depth,
        seed,
        white_level,
        headroom,
    );
    let mut cover = vec![0_u16; width * rows * mode.channels()];
    generator.fill_band(y_start, rows, &mut cover);
    cover
}
