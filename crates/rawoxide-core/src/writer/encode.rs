//! Band embedding and tile encoding for raw frames.

use std::io::Write as _;

use flate2::write::ZlibEncoder;

#[cfg(test)]
use super::RawMode;
use super::payload::PayloadBitReader;
use super::{Compression, WriteError};

#[allow(clippy::too_many_arguments)]
pub(super) fn embed_band(
    band: &mut [u16],
    width: usize,
    total_height: usize,
    y_start: usize,
    rows: usize,
    channels: usize,
    lsb_planes: u8,
    frame: &mut PayloadBitReader<'_>,
    frame_index: u64,
) -> Result<(), WriteError> {
    let planes = usize::from(lsb_planes);
    let slots_per_frame = width
        .checked_mul(total_height)
        .and_then(|samples| samples.checked_mul(channels))
        .and_then(|samples| samples.checked_mul(planes))
        .ok_or(WriteError::TooLarge)?;
    let frame_offset =
        frame_index.saturating_mul(u64::try_from(slots_per_frame).unwrap_or(u64::MAX));
    let band_start = u64::try_from(y_start.saturating_mul(width).saturating_mul(channels))
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(lsb_planes));
    frame.seek(frame_offset.saturating_add(band_start));
    for local_y in 0..rows {
        for x in 0..width {
            for channel in 0..channels {
                let slot = (local_y * width + x) * channels + channel;
                for plane in 0..planes {
                    let Some(bit) = frame.next_bit()? else {
                        return Ok(());
                    };
                    band[slot] = (band[slot] & !(1_u16 << plane)) | (u16::from(bit) << plane);
                }
            }
        }
    }
    Ok(())
}

/// Feed one embedded raw band into a per-frame CRC.
///
/// The digest hashes the array's storage bytes (`uint8` for 8-bit covers,
/// little-endian `uint16` otherwise); the band is already in raster order.
pub(super) fn update_frame_crc(hasher: &mut crc32fast::Hasher, band: &[u16], bit_depth: u16) {
    const BUFFER: usize = 8192;
    let mut bytes = [0_u8; BUFFER];
    let mut used = 0_usize;
    if bit_depth <= 8 {
        for &sample in band {
            bytes[used] = sample as u8;
            used += 1;
            if used == BUFFER {
                hasher.update(&bytes);
                used = 0;
            }
        }
    } else {
        for &sample in band {
            let pair = sample.to_le_bytes();
            bytes[used] = pair[0];
            bytes[used + 1] = pair[1];
            used += 2;
            if used + 2 > BUFFER {
                hasher.update(&bytes[..used]);
                used = 0;
            }
        }
    }
    hasher.update(&bytes[..used]);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_tile_band(
    band: &[u16],
    width: usize,
    rows: usize,
    channels: usize,
    bit_depth: u16,
    tile_x: usize,
    tile_width: usize,
    tile_height: usize,
    row_bytes: usize,
    compression: Compression,
) -> Result<Vec<u8>, WriteError> {
    let mut tile_data = Vec::with_capacity(row_bytes * tile_height);
    for row in 0..tile_height {
        let mut row_data = vec![0_u8; row_bytes];
        let mut bit_position = 0_usize;
        if row < rows {
            for local_x in 0..tile_width {
                let global_x = tile_x * tile_width + local_x;
                for channel in 0..channels {
                    let value = if global_x < width {
                        band[(row * width + global_x) * channels + channel]
                    } else {
                        0
                    };
                    match bit_depth {
                        8 => {
                            row_data[bit_position / 8] = value as u8;
                            bit_position += 8;
                        }
                        16 => {
                            let byte = bit_position / 8;
                            row_data[byte..byte + 2].copy_from_slice(&value.to_le_bytes());
                            bit_position += 16;
                        }
                        10 | 12 | 14 => {
                            pack_bits(&mut row_data, bit_position, value, bit_depth);
                            bit_position += usize::from(bit_depth);
                        }
                        _ => unreachable!("bit depth validated by caller"),
                    }
                }
            }
        }
        if matches!(compression, Compression::AdobeDeflate) {
            apply_predictor(&mut row_data, bit_depth, channels);
        }
        tile_data.extend_from_slice(&row_data);
    }
    if matches!(compression, Compression::AdobeDeflate) {
        let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tile_data)?;
        Ok(encoder.finish()?)
    } else {
        Ok(tile_data)
    }
}

pub(super) fn tile_row_bytes(
    width: usize,
    channels: usize,
    bit_depth: u16,
) -> Result<usize, WriteError> {
    let samples = width.checked_mul(channels).ok_or(WriteError::TooLarge)?;
    match bit_depth {
        8 => Ok(samples),
        16 => samples.checked_mul(2).ok_or(WriteError::TooLarge),
        10 | 12 | 14 => samples
            .checked_mul(usize::from(bit_depth))
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or(WriteError::TooLarge),
        other => Err(WriteError::InvalidBitDepth(other)),
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn encode_tiles(
    cover: &[u16],
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    tile_width: usize,
    tile_height: usize,
    across: usize,
    down: usize,
    row_bytes: usize,
    compression: Compression,
) -> Result<Vec<Vec<u8>>, WriteError> {
    let channels = mode.channels();
    let mut out = Vec::with_capacity(across * down);
    for tile_y in 0..down {
        for tile_x in 0..across {
            let mut tile_data = Vec::with_capacity(row_bytes * tile_height);
            for row in 0..tile_height {
                let global_y = tile_y * tile_height + row;
                let mut row_data = vec![0_u8; row_bytes];
                let mut bit_position = 0_usize;
                for local_x in 0..tile_width {
                    let global_x = tile_x * tile_width + local_x;
                    for channel in 0..channels {
                        let value = if global_y < height && global_x < width {
                            cover[(global_y * width + global_x) * channels + channel]
                        } else {
                            0
                        };
                        match bit_depth {
                            8 => {
                                row_data[bit_position / 8] = value as u8;
                                bit_position += 8;
                            }
                            16 => {
                                let byte = bit_position / 8;
                                row_data[byte..byte + 2].copy_from_slice(&value.to_le_bytes());
                                bit_position += 16;
                            }
                            10 | 12 | 14 => {
                                pack_bits(&mut row_data, bit_position, value, bit_depth);
                                bit_position += usize::from(bit_depth);
                            }
                            _ => unreachable!("bit depth validated by caller"),
                        }
                    }
                }
                if matches!(compression, Compression::AdobeDeflate) {
                    apply_predictor(&mut row_data, bit_depth, channels);
                }
                tile_data.extend_from_slice(&row_data);
            }
            if matches!(compression, Compression::AdobeDeflate) {
                let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&tile_data)?;
                out.push(encoder.finish()?);
            } else {
                out.push(tile_data);
            }
        }
    }
    Ok(out)
}

pub(super) fn apply_predictor(row: &mut [u8], bit_depth: u16, channels: usize) {
    if bit_depth == 8 {
        for index in (channels..row.len()).rev() {
            row[index] = row[index].wrapping_sub(row[index - channels]);
        }
    } else if bit_depth == 16 {
        let samples = row.len() / 2;
        for index in (channels..samples).rev() {
            let current = u16::from_le_bytes([row[index * 2], row[index * 2 + 1]]);
            let previous =
                u16::from_le_bytes([row[(index - channels) * 2], row[(index - channels) * 2 + 1]]);
            row[index * 2..index * 2 + 2]
                .copy_from_slice(&current.wrapping_sub(previous).to_le_bytes());
        }
    }
}

pub(super) fn pack_bits(target: &mut [u8], bit_offset: usize, value: u16, bit_depth: u16) {
    for bit in 0..usize::from(bit_depth) {
        let index = bit_offset + bit;
        let source_bit = (value >> (bit_depth - 1 - bit as u16)) & 1;
        target[index / 8] |= u8::try_from(source_bit).unwrap_or(0) << (7 - (index % 8));
    }
}
