//! `GainMap` opcode structure and profile parsing.

use rawhide_core::opcodes::{
    GAIN_MAP_PHASES, build_opcode_list2, opcode_list_fingerprint, parse_opcode_list2,
};
use rawhide_core::profiles;

#[test]
fn custom_profile_gainmap_grid_parses() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let gainmap = profile.gainmap.as_ref().expect("gainmap");
    assert_eq!((gainmap.rows, gainmap.cols), (8, 10));
    assert_eq!(gainmap.planes.len(), 4 * 8 * 10);
    assert!((gainmap.jitter - 0.025).abs() < f64::EPSILON);
    assert_eq!(gainmap.clip, (1.0, 4.2));
}

#[test]
fn opcode_list_roundtrips_four_phase_maps() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let gainmap = profile.gainmap.as_ref().unwrap();
    let blob = build_opcode_list2(128, 96, Some(4), gainmap);
    let maps = parse_opcode_list2(&blob).unwrap();
    assert_eq!(maps.len(), 4);
    for (map, phase) in maps.iter().zip(GAIN_MAP_PHASES) {
        assert_eq!((map.top, map.left), phase);
        assert_eq!((map.right, map.bottom), (128, 96));
        assert_eq!((map.points_v, map.points_h), (30, 40));
        assert_eq!(map.gains.len(), 30 * 40);
        assert!(map.gains.iter().all(|gain| (1.0..=4.2).contains(gain)));
    }
    let fingerprint = opcode_list_fingerprint(&blob).unwrap();
    assert_eq!(fingerprint.len(), blob.len());
    assert_eq!(&fingerprint[..4], &4_u32.to_le_bytes());
}

fn minimal_blob() -> Vec<u8> {
    let mut header = Vec::new();
    for value in [0_u32, 0, 2, 2, 0, 1, 2, 2, 2, 2] {
        header.extend_from_slice(&value.to_be_bytes());
    }
    for value in [1.0_f64, 1.0, 0.0, 0.0] {
        header.extend_from_slice(&value.to_be_bytes());
    }
    header.extend_from_slice(&1_u32.to_be_bytes());
    for gain in [1.0_f32, 1.5, 2.0, 2.5] {
        header.extend_from_slice(&gain.to_be_bytes());
    }
    let mut blob = Vec::new();
    blob.extend_from_slice(&1_u32.to_be_bytes());
    for value in [9_u32, 0x0103_0000, 0, u32::try_from(header.len()).unwrap()] {
        blob.extend_from_slice(&value.to_be_bytes());
    }
    blob.extend_from_slice(&header);
    blob
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

#[test]
fn fingerprint_bytes_match_python() {
    let fingerprint = opcode_list_fingerprint(&minimal_blob()).unwrap();
    assert_eq!(
        fingerprint,
        hex(
            "010000000900000000000301000000005c00000000000000000000000200000002000000000000000100000002000000020000000200000002000000000000000000f03f000000000000f03f00000000000000000000000000000000010000000000803f0000c03f0000004000002040"
        )
    );
}

#[test]
fn unique_id_with_opcode_fingerprint_matches_python() {
    let uid = rawhide_core::digests::raw_data_unique_id(
        &[0_u8; 16],
        "Cam",
        [(2, 1), (2, 1)],
        [(0, 1), (0, 1)],
        [None, Some(&minimal_blob()), None],
    )
    .unwrap();
    assert_eq!(
        uid,
        <[u8; 16]>::try_from(hex("e16517ed690a36e692633f94ae1ae804")).unwrap()
    );
}

#[test]
fn malformed_lists_fail_closed() {
    assert!(parse_opcode_list2(&[]).is_err());
    assert!(parse_opcode_list2(&[0, 0, 0, 1]).is_err());
    let mut blob = minimal_blob();
    blob[8..12].copy_from_slice(&0x0200_0000_u32.to_be_bytes());
    assert!(parse_opcode_list2(&blob).is_err());
    let mut blob = minimal_blob();
    blob[4..8].copy_from_slice(&99_u32.to_be_bytes());
    assert!(parse_opcode_list2(&blob).is_err());
    let mut blob = minimal_blob();
    blob.push(0);
    assert!(parse_opcode_list2(&blob).is_err());
    let mut blob = minimal_blob();
    blob[52..56].copy_from_slice(&u32::MAX.to_be_bytes());
    blob[56..60].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse_opcode_list2(&blob).is_err());
}

#[test]
fn fingerprint_rejects_non_float_tail() {
    let mut blob = minimal_blob();
    let size = u32::from_be_bytes(blob[16..20].try_into().unwrap());
    blob[16..20].copy_from_slice(&(size + 1).to_be_bytes());
    blob.push(0);
    assert!(opcode_list_fingerprint(&blob).is_err());
    assert!(
        rawhide_core::digests::raw_data_unique_id(
            &[0_u8; 16],
            "Cam",
            [(2, 1), (2, 1)],
            [(0, 1), (0, 1)],
            [None, Some(&blob), None],
        )
        .is_err()
    );
}
