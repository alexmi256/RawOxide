//! Decode tests against the Python-generated DNG fixtures.
//!
//! These are the Phase 1 acceptance tests: every fixture must decode to its
//! original payload, with auto-detected planes, keys, split ordering and the
//! safety limits matching the Python implementation.

use std::path::{Path, PathBuf};

use rawhide_core::{
    DecodeError, DecodeOptions, DngReader, SplitIdField, SplitSeqField, decode_files,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .canonicalize()
        .expect("fixtures directory")
}

fn fixture(name: &str) -> PathBuf {
    fixtures().join("dng").join(name)
}

fn payload(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join("payloads").join(name)).expect("payload fixture")
}

fn decode_one(name: &str, options: &DecodeOptions<'_>) -> Vec<u8> {
    decode_files(&[fixture(name)], options)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
        .payload
}

#[test]
fn decodes_linear_16bit_auto_planes() {
    let decoded = decode_one("linear_256x192_16bit_1plane.dng", &DecodeOptions::default());
    assert_eq!(decoded, payload("payload_3000.bin"));
}

#[test]
fn decodes_8bit_deflate() {
    let decoded = decode_one("linear_128x96_8bit_deflate.dng", &DecodeOptions::default());
    assert_eq!(decoded, payload("payload_1000.bin"));
}

#[test]
fn decodes_cfa_14bit_packed() {
    let decoded = decode_one("cfa_128x96_14bit_2planes.dng", &DecodeOptions::default());
    assert_eq!(decoded, payload("payload_1000.bin"));
}

#[test]
fn decodes_keyed_payload() {
    let options = DecodeOptions {
        key: Some(b"fixture-key"),
        lsb_planes: Some(4),
        ..DecodeOptions::default()
    };
    let decoded = decode_one("linear_128x96_16bit_4planes_keyed.dng", &options);
    assert_eq!(decoded, payload("payload_1000.bin"));
}

#[test]
fn wrong_key_fails_closed() {
    let options = DecodeOptions {
        key: Some(b"wrong-key"),
        ..DecodeOptions::default()
    };
    let error = decode_files(
        &[fixture("linear_128x96_16bit_4planes_keyed.dng")],
        &options,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no decodable payload"));
}

#[test]
fn decodes_three_raw_frames() {
    let decoded = decode_one("linear_128x96_16bit_3frames.dng", &DecodeOptions::default());
    assert_eq!(decoded, payload("payload_8000.bin"));
}

#[test]
fn decodes_split_set_in_metadata_order() {
    let paths = vec![fixture("split0002.dng"), fixture("split0001.dng")];
    let outcome = decode_files(&paths, &DecodeOptions::default()).unwrap();
    assert_eq!(outcome.payload, payload("payload_5000.bin"));
    assert_eq!(outcome.ordered_paths[0], fixture("split0001.dng"));
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("no chunk totals"))
    );
}

#[test]
fn explicit_plane_count_is_strict() {
    let options = DecodeOptions {
        lsb_planes: Some(2),
        ..DecodeOptions::default()
    };
    let error = decode_files(&[fixture("linear_256x192_16bit_1plane.dng")], &options).unwrap_err();
    assert!(error.to_string().contains("magic not found"));
}

#[test]
fn max_bytes_is_checked_before_payload_allocation() {
    let options = DecodeOptions {
        max_bytes: Some(1024),
        ..DecodeOptions::default()
    };
    let error = decode_files(&[fixture("linear_256x192_16bit_1plane.dng")], &options).unwrap_err();
    match error {
        DecodeError::MaxBytesExceeded { declared, max } => {
            assert_eq!(declared, 3000);
            assert_eq!(max, 1024);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn raw_frames_expose_geometry_and_samples() {
    let reader = DngReader::open(fixture("linear_256x192_16bit_1plane.dng")).unwrap();
    let frames = reader.raw_frames().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].width, 256);
    assert_eq!(frames[0].height, 192);
    assert_eq!(frames[0].samples_per_pixel, 3);
    assert_eq!(frames[0].bits_per_sample, 16);
    assert_eq!(frames[0].sample_count(), 256 * 192 * 3);

    let reader = DngReader::open(fixture("cfa_128x96_14bit_2planes.dng")).unwrap();
    let frames = reader.raw_frames().unwrap();
    assert_eq!(frames[0].samples_per_pixel, 1);
    assert_eq!(frames[0].bits_per_sample, 14);
    assert_eq!(frames[0].sample_count(), 128 * 96);

    let reader = DngReader::open(fixture("linear_128x96_16bit_3frames.dng")).unwrap();
    assert_eq!(reader.raw_frames().unwrap().len(), 3);
}

#[test]
fn split_markers_match_python_defaults() {
    let reader = DngReader::open(fixture("split0001.dng")).unwrap();
    let markers = reader
        .split_markers(SplitIdField::ImageUniqueId, SplitSeqField::ImageNumber)
        .unwrap();
    assert!(markers.id_present);
    assert!(markers.seq_present);
    assert_eq!(markers.id.as_deref(), Some("fixture-split-0001"));
    assert_eq!(markers.seq, Some(1));
    assert_eq!(markers.total, None);

    let plain = DngReader::open(fixture("linear_256x192_16bit_1plane.dng")).unwrap();
    let markers = plain
        .split_markers(SplitIdField::ImageUniqueId, SplitSeqField::ImageNumber)
        .unwrap();
    assert!(!markers.id_present);
    assert!(!markers.seq_present);
}

#[test]
fn empty_input_is_rejected() {
    let error = decode_files(&[], &DecodeOptions::default()).unwrap_err();
    assert!(matches!(error, DecodeError::NoInput));
}
