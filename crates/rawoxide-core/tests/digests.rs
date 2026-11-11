//! Digest parity with the Python oracle.

use rawoxide_core::digests::{new_raw_image_digest, raw_data_unique_id};

#[test]
fn linear_raw_digest_matches_python() {
    let samples: Vec<u16> = (0..18).collect();
    let digest = new_raw_image_digest(&samples, 3, 2, 3, 16);
    assert_eq!(
        digest,
        [
            0x5e, 0x5d, 0x6c, 0xb4, 0xb0, 0x46, 0xcf, 0xa0, 0xce, 0x4d, 0x53, 0xe1, 0x1e, 0x9b,
            0x2d, 0x9d
        ]
    );
}

#[test]
fn cfa_8bit_digest_matches_python() {
    let samples: Vec<u16> = (0..15).collect();
    let digest = new_raw_image_digest(&samples, 5, 3, 1, 8);
    assert_eq!(
        digest,
        [
            0xe3, 0x71, 0xc9, 0xee, 0x2e, 0x52, 0x6d, 0x42, 0x24, 0xe5, 0x9c, 0xf3, 0xaf, 0xb5,
            0x20, 0x66
        ]
    );
}

#[test]
fn unique_id_matches_python() {
    let digest = new_raw_image_digest(&(0..18).collect::<Vec<u16>>(), 3, 2, 3, 16);
    let uid = raw_data_unique_id(
        &digest,
        "Custom Pixel XL",
        [(112, 1), (80, 1)],
        [(8, 1), (8, 1)],
        [None, None, None],
    )
    .unwrap();
    assert_eq!(
        uid,
        [
            0x39, 0x96, 0xcd, 0xb8, 0x31, 0x69, 0x1e, 0x2b, 0x97, 0x46, 0xd1, 0xdd, 0x6d, 0x31,
            0x42, 0x61
        ]
    );
}

#[test]
fn ten_bit_multitile_digest_matches_python() {
    let width = 300;
    let height = 257;
    let samples: Vec<u16> = (0..width * height)
        .map(|index| (index % 1023) as u16)
        .collect();
    assert_eq!(
        new_raw_image_digest(&samples, width, height, 1, 10),
        [
            0x48, 0x46, 0xdd, 0x81, 0x7a, 0x12, 0xe6, 0xb1, 0xea, 0x11, 0xea, 0x02, 0x5f, 0xa1,
            0x64, 0x81,
        ]
    );
}

#[test]
fn twelve_bit_multitile_digest_matches_python() {
    let width = 300;
    let height = 257;
    let samples: Vec<u16> = (0..width * height)
        .map(|index| (index % 4095) as u16)
        .collect();
    assert_eq!(
        new_raw_image_digest(&samples, width, height, 1, 12),
        [
            0xf0, 0x55, 0x78, 0xb9, 0x9a, 0xdd, 0x33, 0x5e, 0x6e, 0x44, 0x80, 0xd7, 0x98, 0x20,
            0x45, 0x6e,
        ]
    );
}

#[test]
fn fourteen_bit_multitile_digest_matches_python() {
    let width = 300;
    let height = 257;
    let samples: Vec<u16> = (0..width * height)
        .map(|index| (index % 16383) as u16)
        .collect();
    assert_eq!(
        new_raw_image_digest(&samples, width, height, 1, 14),
        [
            0x77, 0xab, 0x56, 0xf4, 0xa0, 0xa6, 0xc0, 0x65, 0xc4, 0x8f, 0x9e, 0x63, 0x98, 0x90,
            0xcd, 0xb7,
        ]
    );
}

#[test]
fn linear_multitile_planar_digest_matches_python() {
    let width = 300;
    let height = 257;
    let samples: Vec<u16> = (0..width * height * 3)
        .map(|index| ((index as u64 * 3) % 4095) as u16)
        .collect();
    assert_eq!(
        new_raw_image_digest(&samples, width, height, 3, 12),
        [
            0xb3, 0x6a, 0xa8, 0x30, 0x33, 0xe7, 0x7b, 0x90, 0x18, 0x60, 0x97, 0x28, 0xb8, 0x49,
            0x1b, 0x07,
        ]
    );
}

#[test]
#[should_panic(expected = "wrong length")]
fn short_sample_buffer_fails_closed() {
    let _ = new_raw_image_digest(&[1, 2, 3], 4, 4, 1, 16);
}

#[test]
fn incremental_digest_matches_one_shot() {
    for (channels, depth) in [(1, 10), (3, 16), (3, 8)] {
        let width = 300;
        for height in [96, 256, 257, 300] {
            for band_rows in [96, 300] {
                let samples: Vec<u16> = (0..width * height * channels)
                    .map(|index| ((index * 7) % 1024) as u16)
                    .collect();
                let expected = new_raw_image_digest(&samples, width, height, channels, depth);
                let mut digest =
                    rawoxide_core::digests::RawImageDigest::new(width, height, channels, depth);
                for y in (0..height).step_by(band_rows) {
                    let rows = (height - y).min(band_rows);
                    let band = &samples[y * width * channels..(y + rows) * width * channels];
                    digest.update_band(y, rows, band);
                }
                assert_eq!(digest.finish(), expected);
            }
        }
    }
}

#[test]
fn incremental_digest_matches_empty_image() {
    let expected = new_raw_image_digest(&[], 64, 0, 1, 16);
    let digest = rawoxide_core::digests::RawImageDigest::new(64, 0, 1, 16).finish();
    assert_eq!(digest, expected);
}

#[test]
#[should_panic(expected = "do not cover")]
fn incremental_digest_rejects_missing_rows() {
    let _ = rawoxide_core::digests::RawImageDigest::new(64, 48, 1, 16).finish();
}

#[test]
#[should_panic(expected = "contiguous")]
fn incremental_digest_rejects_gaps() {
    let mut digest = rawoxide_core::digests::RawImageDigest::new(4, 4, 1, 16);
    let band = vec![0_u16; 8];
    digest.update_band(0, 2, &band);
    digest.update_band(3, 1, &band);
}
