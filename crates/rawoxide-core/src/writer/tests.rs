//! Writer unit tests.

use super::cover::{synthetic_cover, synthetic_cover_band};
use super::encode::{apply_predictor, encode_tile_band, encode_tiles, tile_row_bytes};
use super::entries::select_bigtiff;
use super::{Compression, RawMode};

#[test]
fn layout_selection_uses_raw_bytes_and_force() {
    assert!(!select_bigtiff(3_500_000_000, 0, false));
    assert!(select_bigtiff(3_500_000_001, 0, false));
    assert!(select_bigtiff(3_000_000_000, 500_000_001, false));
    assert!(select_bigtiff(1, 1, true));
}

#[test]
fn banded_cover_matches_full_cover() {
    let full = synthetic_cover(300, 200, RawMode::Linear, 16, 7, None, 0);
    let channels = 3;
    let mut joined = Vec::new();
    for y_start in (0..200).step_by(96) {
        let rows = (200 - y_start).min(96);
        joined.extend_from_slice(&synthetic_cover_band(
            300,
            200,
            y_start,
            rows,
            RawMode::Linear,
            16,
            7,
            None,
            0,
        ));
    }
    assert_eq!(full.len(), joined.len());
    assert_eq!(full, joined);
    assert_eq!(channels, 3);
}

#[test]
fn banded_tile_encoding_matches_full_encoding() {
    let width: usize = 300;
    let height: usize = 200;
    let tile_width: usize = 128;
    let tile_height: usize = 96;
    let across = width.div_ceil(tile_width);
    let down = height.div_ceil(tile_height);
    let cover = synthetic_cover(width, height, RawMode::Linear, 16, 3, None, 0);
    let row_bytes = tile_row_bytes(tile_width, 3, 16).unwrap();
    for compression in [Compression::None, Compression::AdobeDeflate] {
        let expected = encode_tiles(
            &cover,
            width,
            height,
            RawMode::Linear,
            16,
            tile_width,
            tile_height,
            across,
            down,
            row_bytes,
            compression,
        )
        .unwrap();
        let mut actual = Vec::new();
        for tile_y in 0..down {
            let y_start = tile_y * tile_height;
            let rows = (height - y_start).min(tile_height);
            let band = &cover[y_start * width * 3..(y_start + rows) * width * 3];
            for tile_x in 0..across {
                actual.push(
                    encode_tile_band(
                        band,
                        width,
                        rows,
                        3,
                        16,
                        tile_x,
                        tile_width,
                        tile_height,
                        row_bytes,
                        compression,
                    )
                    .unwrap(),
                );
            }
        }
        assert_eq!(expected, actual);
    }
}

#[test]
fn predictor_uses_the_component_stride() {
    let mut eight = vec![10, 20, 30, 40, 50, 60, 70, 80, 90];
    apply_predictor(&mut eight, 8, 3);
    assert_eq!(eight, vec![10, 20, 30, 30, 30, 30, 30, 30, 30]);

    let mut sixteen: Vec<u8> = [10_u16, 20, 30, 40, 50, 60]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    apply_predictor(&mut sixteen, 16, 3);
    let expected: Vec<u8> = [10_u16, 20, 30, 30, 30, 30]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    assert_eq!(sixteen, expected);
}
