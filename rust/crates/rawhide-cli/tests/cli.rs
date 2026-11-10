//! CLI contract tests.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

#[test]
fn capacity_default_geometry() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.args(["capacity", "--width", "2048", "--height", "1536"]);
    command
        .assert()
        .success()
        .stdout(
            predicate::str::contains("samples: 9437184").and(predicate::str::contains(
                "capacity (1 LSB plane(s)): 1,152.0 KB",
            )),
        );
}

#[test]
fn capacity_cfa_with_frames() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "capacity",
        "--width",
        "2048",
        "--height",
        "1536",
        "--mode",
        "cfa",
        "--raw-frames",
        "2",
        "--lsb-planes",
        "4",
    ]);
    command
        .assert()
        .success()
        .stdout(
            predicate::str::contains("samples: 6291456").and(predicate::str::contains(
                "capacity (4 LSB plane(s) x 2 frames): 3,072.0 KB",
            )),
        );
}

#[test]
fn capacity_linear_four_planes() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "capacity",
        "--width",
        "2048",
        "--height",
        "1536",
        "--lsb-planes",
        "4",
    ]);
    command
        .assert()
        .success()
        .stdout(
            predicate::str::contains("samples: 9437184").and(predicate::str::contains(
                "capacity (4 LSB plane(s)): 4,608.0 KB",
            )),
        );
}

#[test]
fn help_mentions_the_project() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.arg("--help");
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("RawHide embeds a framed payload"));
}

#[test]
fn version_is_reported() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.arg("--version");
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("rawhide 0.1.0"));
}

#[test]
fn invalid_plane_count_is_rejected_by_clap() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.args(["capacity", "--lsb-planes", "17"]);
    command.assert().failure().stderr(
        predicate::str::contains("17 is not in 1..=16")
            .or(predicate::str::contains("invalid value")),
    );
}

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .canonicalize()
        .expect("fixtures directory")
}

fn fixture(name: &str) -> std::path::PathBuf {
    fixtures().join("dng").join(name)
}

fn payload(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join("payloads").join(name)).expect("payload fixture")
}

#[test]
fn decode_linear_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("recovered.bin");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("linear_256x192_16bit_1plane.dng").to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    command.assert().success();
    assert_eq!(std::fs::read(&output).unwrap(), payload("payload_3000.bin"));
}

#[test]
fn decode_keyed_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("recovered.bin");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("linear_128x96_16bit_4planes_keyed.dng")
            .to_str()
            .unwrap(),
        "--key",
        "fixture-key",
        "-o",
        output.to_str().unwrap(),
    ]);
    command.assert().success();
    assert_eq!(std::fs::read(&output).unwrap(), payload("payload_1000.bin"));
}

#[test]
fn decode_split_set_by_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("recovered.bin");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("split0002.dng").to_str().unwrap(),
        fixture("split0001.dng").to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    command
        .assert()
        .success()
        .stderr(predicate::str::contains("no chunk totals"));
    assert_eq!(std::fs::read(&output).unwrap(), payload("payload_5000.bin"));
}

#[test]
fn decode_wrong_key_fails() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("recovered.bin");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("linear_128x96_16bit_4planes_keyed.dng")
            .to_str()
            .unwrap(),
        "--key",
        "wrong",
        "-o",
        output.to_str().unwrap(),
    ]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("no decodable payload"));
}

#[test]
fn decode_max_bytes_fails_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("recovered.bin");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("linear_256x192_16bit_1plane.dng").to_str().unwrap(),
        "--max-bytes",
        "1k",
        "-o",
        output.to_str().unwrap(),
    ]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("exceeds max_bytes"));
    assert!(!output.exists());
    let temp = output.with_file_name(format!(
        ".{}.rawhide.tmp",
        output.file_name().unwrap().to_str().unwrap()
    ));
    assert!(!temp.exists());
}

#[test]
fn decode_failure_after_streaming_removes_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("as-directory");
    std::fs::create_dir(&output).unwrap();

    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "decode",
        "-i",
        fixture("linear_256x192_16bit_1plane.dng").to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    command.assert().failure();
    assert!(!dir.path().join(".as-directory.rawhide.tmp").exists());
}

#[test]
fn encode_then_decode_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("roundtrip.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_3000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
    ]);
    encode
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote"))
        .stdout(predicate::str::contains("payload"));

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_keyed_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("keyed.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--key",
        "spike-key",
        "--lsb-planes",
        "2",
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
        "--key",
        "spike-key",
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_capacity_failure_is_clear() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("too-big.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");
    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--lsb-planes",
        "1",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("exceeds capacity"));
    assert!(!encoded.exists());
}

#[test]
fn encode_cfa_14bit_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("cfa.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--mode",
        "cfa",
        "--bit-depth",
        "14",
        "--lsb-planes",
        "2",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_multiple_raw_frames_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("frames.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--raw-frames",
        "3",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_split_set_with_page_numbers_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("vol.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--split-file",
        "3000",
        "--split-file-metadata-seq",
        "PageNumber",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    encode.assert().success();

    let album = dir.path().join("payload_5000");
    let mut chunks = std::fs::read_dir(&album)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    chunks.sort();
    assert_eq!(chunks.len(), 2);

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        chunks[1].to_str().unwrap(),
        chunks[0].to_str().unwrap(),
        "--split-file-metadata-seq",
        "PageNumber",
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode
        .assert()
        .success()
        .stderr(predicate::str::contains("no chunk totals").not());
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_auto_selects_planes_and_reports_config() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("auto.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
    ]);
    encode
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "auto-config (mode, bit_depth, lsb_planes)",
        ))
        .stdout(predicate::str::contains("5 LSB plane(s)"))
        .stdout(predicate::str::contains("heavily degrade"));
}

#[test]
fn encode_auto_size_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("sized.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--auto-size",
    ]);
    encode
        .assert()
        .success()
        .stdout(predicate::str::contains("auto-config"))
        .stdout(predicate::str::contains("auto-size"));

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_auto_size_rejects_explicit_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("sized.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--auto-size",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("--auto-size cannot be combined"));
}

#[test]
fn encode_camera_profile_and_image_time_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("canon.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--camera-profile",
        "canon_eos_80d",
        "--image-time",
        "2021:06:05 14:30:09",
        "--width",
        "128",
        "--height",
        "96",
    ]);
    encode.assert().success().stdout(predicate::str::contains(
        "auto-config (mode, bit_depth, lsb_planes)",
    ));

    let bytes = std::fs::read(&encoded).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("Canon EOS 80D"));
    assert!(text.contains("2021:06:05 14:30:09"));

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_rejects_unknown_camera_profile() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("missing.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--camera-profile",
        "not_a_camera",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown camera profile"));
}

#[test]
fn encode_custom_profile_uses_suggested_mode() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("custom.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--camera-profile",
        "custom_pixel_xl",
        "--width",
        "128",
        "--height",
        "96",
        "--seed",
        "4",
    ]);
    encode
        .assert()
        .success()
        .stdout(predicate::str::contains(" cfa 16-bit"));
}

#[cfg(unix)]
#[test]
fn encode_rfc3339_image_time_keeps_local_offset() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("time.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.env("TZ", "Asia/Tokyo").args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--image-time",
        "2021-06-05T14:30:09+09:00",
    ]);
    encode.assert().success();

    let text = String::from_utf8_lossy(&std::fs::read(&encoded).unwrap()).into_owned();
    assert!(text.contains("2021:06:05 14:30:09"));
    assert!(text.contains("+09:00"));
}

#[test]
fn encode_rejects_unknown_level_policy() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("policy.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--level-policy",
        "bogus",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("possible values"));
}

fn gps_coordinates(path: &std::path::Path) -> (Vec<[u32; 2]>, Vec<[u32; 2]>) {
    let file = tiff_reader::TiffFile::open(path).unwrap();
    let pointer = file.ifd(0).unwrap().tag(34_853).unwrap();
    let offset = match &pointer.value {
        tiff_reader::TagValue::Long(values) => values[0],
        other => panic!("unexpected GPS pointer {other:?}"),
    };
    let gps = file.read_ifd_at_offset(u64::from(offset)).unwrap();
    let rational = |tag: u16| match &gps.tag(tag).unwrap().value {
        tiff_reader::TagValue::Rational(values) => values.clone(),
        other => panic!("unexpected GPS value {other:?}"),
    };
    (rational(2), rational(4))
}

#[test]
fn split_set_shares_one_gps_location() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("vol.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--camera-profile",
        "custom_pixel_xl",
        "--split-file",
        "3000",
        "--width",
        "128",
        "--height",
        "96",
        "--level-policy",
        "strict",
        "--seed",
        "7",
    ]);
    encode.assert().success();

    let album = dir.path().join("payload_5000");
    let first = album.join("vol0001.dng");
    let second = album.join("vol0002.dng");
    assert_eq!(gps_coordinates(&first), gps_coordinates(&second));
}

#[test]
fn encode_rejects_out_of_range_gps_radius() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("radius.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--gps-radius",
        "30",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("gps radius must be between"));
}

#[test]
fn encode_accepts_a_valid_gps_radius() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("radius.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--camera-profile",
        "custom_pixel_xl",
        "--gps-radius",
        "5",
        "--width",
        "64",
        "--height",
        "48",
        "--seed",
        "2",
    ]);
    encode.assert().success();
    assert_eq!(gps_coordinates(&encoded).0.len(), 3);
}

#[test]
fn encode_adobe_deflate_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("deflate.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--compression",
        "adobe_deflate",
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_rejects_deflate_for_packed_depths() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("deflate.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--bit-depth",
        "12",
        "--compression",
        "adobe_deflate",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires bit depth 8 or 16"));
}

#[test]
fn gen_tiff_writes_a_plain_tiff() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("cover.tif");
    let mut command = cargo_bin_cmd!("rawhide");
    command.args([
        "gen-tiff",
        "-o",
        output.to_str().unwrap(),
        "--width",
        "16",
        "--height",
        "12",
        "--seed",
        "1",
    ]);
    command.assert().success();
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(b"II*\0"));
}

#[test]
fn encode_deflate_mode_and_depth_matrix_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");
    for (mode, depth) in [("linear", 8), ("cfa", 8), ("cfa", 16)] {
        let encoded = dir.path().join(format!("{mode}-{depth}.dng"));
        let recovered = dir.path().join(format!("{mode}-{depth}.bin"));
        let mut encode = cargo_bin_cmd!("rawhide");
        encode.args([
            "encode",
            "-i",
            payload_path.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
            "--width",
            "300",
            "--height",
            "257",
            "--mode",
            mode,
            "--bit-depth",
            &depth.to_string(),
            "--compression",
            "adobe_deflate",
            "--seed",
            "3",
        ]);
        encode.assert().success();

        let mut decode = cargo_bin_cmd!("rawhide");
        decode.args([
            "decode",
            "-i",
            encoded.to_str().unwrap(),
            "-o",
            recovered.to_str().unwrap(),
        ]);
        decode.assert().success();
        assert_eq!(
            std::fs::read(&recovered).unwrap(),
            std::fs::read(&payload_path).unwrap()
        );
    }
}

#[test]
fn encode_deflate_multi_frame_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("frames.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "300",
        "--height",
        "257",
        "--raw-frames",
        "2",
        "--compression",
        "adobe_deflate",
        "--seed",
        "3",
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_with_file_thumbnail_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let thumbnail = dir.path().join("thumb.tif");
    rawhide_core::write_cover_tiff(&thumbnail, 40, 30, Some(1), "thumb").unwrap();
    let encoded = dir.path().join("file-thumb.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--thumbnail",
        thumbnail.to_str().unwrap(),
    ]);
    encode.assert().success();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn encode_rejects_missing_thumbnail_file() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("missing-thumb.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--thumbnail",
        dir.path().join("missing.png").to_str().unwrap(),
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown thumbnail source"));
}

#[test]
fn encode_writes_debug_metadata_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("debug.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--debug-metadata-output",
        "--seed",
        "3",
    ]);
    encode.assert().success();

    let sidecar = dir.path().join("debug.json");
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(value["schema"], "stegodng-debug-metadata");
    assert_eq!(value["summary"]["width"], 64);
    assert_eq!(value["summary"]["height"], 48);
    assert!(value["metadata"]["datetime"].as_str().is_some());
    assert!(value["profile"]["slug"].as_str().is_some());
    let ifd0 = value["file"]["ifd0"].as_array().unwrap();
    assert!(ifd0.iter().any(|tag| tag["tag"] == 256));
    assert_eq!(when_name(ifd0, 254).as_deref(), Some("NewSubfileType"));
    let sub_ifds = ifd0.iter().find(|tag| tag["tag"] == 330).expect("SubIFDs");
    assert_eq!(sub_ifds["type"], "IFD");
    assert!(sub_ifds["value"].is_u64());
    assert!(value["file"]["subifds"].as_array().is_some());
    assert_eq!(
        value["cover"]["frames"][0]["shape"],
        serde_json::json!([48, 64, 3])
    );
    assert_eq!(value["metadata"]["lens_spec"].as_array().unwrap().len(), 8);
    assert!(value["profile"]["iso_choices"].is_null());
    assert!(value["profile"]["lenses"].is_null());
    assert!(value["profile"]["dng_version"]["crc32"].as_str().is_some());
    assert_eq!(value["profile"]["dng_version"]["bytes"], 4);
    assert!(value["cover"]["frames"][0]["crc32"].as_str().is_some());
    assert!(value["encode"]["payload"]["frame_crc32"].as_str().is_some());
    assert!(value["thumbnail"]["crc32"].as_str().is_some());
}

fn when_name(tags: &[serde_json::Value], code: u16) -> Option<String> {
    tags.iter()
        .find(|tag| tag["tag"] == code)
        .and_then(|tag| tag["name"].as_str().map(str::to_owned))
}

#[test]
fn split_set_writes_one_debug_sidecar_per_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("vol.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--split-file",
        "3000",
        "--debug-metadata-output",
        "--seed",
        "3",
    ]);
    encode.assert().success().stdout(
        predicate::str::contains("thumbnail: ")
            .and(predicate::str::contains("debug: "))
            .and(predicate::str::contains("profile: gfx_100"))
            .and(predicate::str::contains("meta: ")),
    );

    let album = dir.path().join("payload_5000");
    let mut seqs = Vec::new();
    for seq in [1_u32, 2] {
        let sidecar = album.join(format!("vol{seq:04}.json"));
        assert!(sidecar.is_file(), "missing {}", sidecar.display());
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(value["summary"]["chunk_seq"], seq);
        assert_eq!(value["summary"]["chunk_total"], 2);
        seqs.push(value["summary"]["chunk_seq"].as_u64().unwrap());
    }
    seqs.sort_unstable();
    assert_eq!(seqs, [1, 2]);
}

#[test]
fn debug_sidecar_does_not_overwrite_json_output() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("image.json");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--debug-metadata-output",
    ]);
    encode.assert().success();
    assert!(encoded.is_file());
    assert!(dir.path().join("image.dng.json").is_file());
    let bytes = std::fs::read(&encoded).unwrap();
    assert!(bytes.starts_with(b"II*\0"));
}

#[test]
fn bigtiff_output_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("big.dng");
    let recovered = dir.path().join("recovered.bin");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--bigtiff",
        "--seed",
        "3",
    ]);
    encode.assert().success();
    assert!(std::fs::read(&encoded).unwrap().starts_with(b"II+\0"));

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        recovered.to_str().unwrap(),
    ]);
    decode.assert().success();
    assert_eq!(
        std::fs::read(&recovered).unwrap(),
        std::fs::read(&payload_path).unwrap()
    );
}

#[test]
fn no_randomize_produces_deterministic_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");
    let mut stamps = Vec::new();
    for index in 0..2 {
        let encoded = dir.path().join(format!("det{index}.dng"));
        let mut encode = cargo_bin_cmd!("rawhide");
        encode.args([
            "encode",
            "-i",
            payload_path.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
            "--width",
            "64",
            "--height",
            "48",
            "--no-randomize",
            "--debug-metadata-output",
        ]);
        encode.assert().success();
        let sidecar = dir.path().join(format!("det{index}.json"));
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(sidecar).unwrap()).unwrap();
        stamps.push(value["metadata"]["datetime"].as_str().unwrap().to_owned());
    }
    assert_eq!(stamps[0], stamps[1]);
}

#[test]
fn no_randomize_split_sets_are_deterministic_per_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");
    let fixed_id = "00000000000000000000000000000001";
    let mut runs = Vec::new();
    let mut metadata = Vec::new();
    for run in 0..2 {
        let output_dir = dir.path().join(format!("run{run}"));
        std::fs::create_dir_all(&output_dir).unwrap();
        let encoded = output_dir.join("vol.dng");
        let mut encode = cargo_bin_cmd!("rawhide");
        encode.args([
            "encode",
            "-i",
            payload_path.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
            "--width",
            "128",
            "--height",
            "96",
            "--split-file",
            "3000",
            "--split-file-id",
            fixed_id,
            "--no-randomize",
            "--debug-metadata-output",
        ]);
        encode.assert().success();
        let album = output_dir.join("payload_5000");
        let mut chunk_bytes = Vec::new();
        let mut chunk_meta = Vec::new();
        for seq in [1_u32, 2] {
            chunk_bytes.push(std::fs::read(album.join(format!("vol{seq:04}.dng"))).unwrap());
            let sidecar: serde_json::Value = serde_json::from_slice(
                &std::fs::read(album.join(format!("vol{seq:04}.json"))).unwrap(),
            )
            .unwrap();
            chunk_meta.push(sidecar["metadata"].clone());
        }
        runs.push(chunk_bytes);
        metadata.push(chunk_meta);
    }
    assert_eq!(runs[0], runs[1]);
    assert_eq!(metadata[0], metadata[1]);
    assert_ne!(metadata[0][0], metadata[0][1]);
}

#[test]
fn progress_flags_are_accepted_and_mutually_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("progress.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");
    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--progress",
    ]);
    let output = encode.output().unwrap();
    assert!(output.status.success());
    assert!(
        !output.stderr.is_empty(),
        "--progress must force stderr output"
    );

    let mut quiet = cargo_bin_cmd!("rawhide");
    quiet.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        dir.path().join("quiet.dng").to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--no-progress",
    ]);
    let output = quiet.output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "--no-progress must stay silent");

    let mut conflict = cargo_bin_cmd!("rawhide");
    conflict.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--progress",
        "--no-progress",
    ]);
    conflict.assert().failure();

    let mut decode = cargo_bin_cmd!("rawhide");
    decode.args([
        "decode",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        dir.path().join("out.bin").to_str().unwrap(),
        "--no-progress",
    ]);
    decode.assert().success();
}

#[test]
fn debug_sidecar_inventory_covers_gps_exif_and_large_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    for bigtiff in [false, true] {
        let encoded = dir.path().join(format!("custom-{bigtiff}.dng"));
        let mut encode = cargo_bin_cmd!("rawhide");
        encode.args([
            "encode",
            "-i",
            payload_path.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
            "--camera-profile",
            "custom_pixel_xl",
            "--level-policy",
            "strict",
            "--width",
            "128",
            "--height",
            "96",
            "--debug-metadata-output",
            "--seed",
            "4",
        ]);
        if bigtiff {
            encode.arg("--bigtiff");
        }
        encode.assert().success();
        let sidecar: serde_json::Value =
            serde_json::from_slice(&std::fs::read(encoded.with_extension("json")).unwrap())
                .unwrap();
        assert_ne!(sidecar["file"]["ifd0"].as_array().unwrap().len(), 0);
        assert_ne!(sidecar["file"]["subifds"].as_array().unwrap().len(), 0);
        assert!(sidecar["file"]["exif"].as_array().is_some());
        assert!(sidecar["file"]["gps"].as_array().is_some());
        assert!(sidecar["file"]["preview_jpeg"]["crc32"].as_str().is_some());

        let ifd0 = sidecar["file"]["ifd0"].as_array().unwrap();
        let sub_ifds = ifd0.iter().find(|tag| tag["tag"] == 330).expect("SubIFDs");
        assert_eq!(sub_ifds["type"], if bigtiff { "IFD8" } else { "LONG" });
        assert!(sub_ifds["value"].is_u64());

        let raw_tags = sidecar["file"]["subifds"][0].as_array().unwrap();
        assert_eq!(
            raw_tags
                .iter()
                .find(|tag| tag["tag"] == 322)
                .expect("TileWidth")["value"],
            256
        );
        assert_eq!(
            raw_tags
                .iter()
                .find(|tag| tag["tag"] == 323)
                .expect("TileLength")["value"],
            256
        );
        assert!(raw_tags.iter().any(|tag| tag["tag"] == 50829));
        assert!(raw_tags.iter().any(|tag| tag["tag"] == 51125));
        assert!(
            raw_tags
                .iter()
                .any(|tag| tag["tag"] == 50780 && tag["name"] == "BestQualityScale")
        );
        let digest = ifd0
            .iter()
            .find(|tag| tag["tag"] == 51111)
            .expect("NewRawImageDigest");
        assert_eq!(digest["value"].as_str().unwrap().len(), 32);

        assert_eq!(
            sidecar["cover"]["frames"][0]["shape"],
            serde_json::json!([96, 128])
        );
        assert_eq!(sidecar["metadata"]["fidelity_layout"], false);
        assert_eq!(sidecar["metadata"]["emit_lens"], false);
        assert_eq!(sidecar["metadata"]["emit_serials"], false);
        assert!(sidecar["metadata"]["exif_allow"].is_array());
        assert_eq!(sidecar["metadata"]["photo_cfa_pattern"]["bytes"], 8);
        assert!(sidecar["metadata"]["makernote"]["bytes"].as_u64().is_some());
        assert_eq!(
            sidecar["metadata"]["lens_spec"].as_array().unwrap().len(),
            8
        );
    }

    let encoded = dir.path().join("xmp.dng");
    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--debug-metadata-output",
    ]);
    encode.assert().success();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(encoded.with_extension("json")).unwrap()).unwrap();
    let tags = sidecar["file"]["ifd0"].as_array().unwrap();
    let xmp = tags.iter().find(|tag| tag["tag"] == 700).expect("XMP tag");
    assert_eq!(xmp["value"]["bytes"], 12_288);
    assert!(xmp["value"]["crc32"].as_str().is_some());
}

#[test]
fn encode_reports_thumbnail_profile_and_meta_lines() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("lines.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--seed",
        "3",
        "--thumbnail",
        "synthetic",
        "--debug-metadata-output",
    ]);
    encode.assert().success().stdout(
        predicate::str::contains("thumbnail: ")
            .and(predicate::str::contains(": synthetic-gradient"))
            .and(predicate::str::contains(
                "profile: gfx_100 (Fujifilm GFX 100)",
            ))
            .and(predicate::str::contains("meta: "))
            .and(predicate::str::contains("debug: ")),
    );
}

#[test]
fn split_album_accepts_positive_flag_and_single_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("single.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--seed",
        "5",
        "--thumbnail",
        "synthetic",
        "--split-file",
        "20k",
        "--split-file-album",
    ]);
    encode.assert().success().stdout(
        predicate::str::contains("thumbnail: ")
            .and(predicate::str::contains("profile: gfx_100"))
            .and(predicate::str::contains("meta: ")),
    );
    assert!(dir.path().join("payload_1000").join("single.dng").is_file());
    assert!(!encoded.exists());
}

#[test]
fn split_rejects_non_ascii_id_without_leaving_album() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("vol.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--seed",
        "5",
        "--thumbnail",
        "synthetic",
        "--split-file",
        "3000",
        "--split-file-id",
        "é",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-empty ASCII"));
    assert!(!dir.path().join("payload_5000").exists());
}

#[test]
fn split_failure_removes_created_album() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("out.dng");
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "128",
        "--height",
        "96",
        "--seed",
        "5",
        // A real file that is not an image: the config succeeds, the album
        // is created, and the failure happens inside the first write.
        "--thumbnail",
        payload_path.to_str().unwrap(),
        "--split-file",
        "3000",
    ]);
    encode
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot decode thumbnail"));
    assert!(!dir.path().join("payload_5000").exists());
}

#[test]
fn debug_sidecar_records_drawn_metadata_seed() {
    let dir = tempfile::tempdir().unwrap();
    let encoded = dir.path().join("unseeded.dng");
    let payload_path = fixtures().join("payloads").join("payload_1000.bin");

    let mut encode = cargo_bin_cmd!("rawhide");
    encode.args([
        "encode",
        "-i",
        payload_path.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--width",
        "64",
        "--height",
        "48",
        "--thumbnail",
        "synthetic",
        "--debug-metadata-output",
    ]);
    encode.assert().success();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(encoded.with_extension("json")).unwrap()).unwrap();
    assert!(sidecar["encode"]["options"]["seed"].is_null());
    assert_eq!(sidecar["cover"]["seed"], 0);
    assert!(sidecar["cover"]["metadata_seed"].as_u64().is_some());
}

#[test]
fn unseeded_split_chunks_draw_fresh_seeds() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = fixtures().join("payloads").join("payload_5000.bin");
    let mut seeds = Vec::new();

    for name in ["a", "b"] {
        let encoded = dir.path().join(name).join("vol.dng");
        std::fs::create_dir_all(encoded.parent().unwrap()).unwrap();
        let mut encode = cargo_bin_cmd!("rawhide");
        encode.args([
            "encode",
            "-i",
            payload_path.to_str().unwrap(),
            "-o",
            encoded.to_str().unwrap(),
            "--width",
            "128",
            "--height",
            "96",
            "--thumbnail",
            "synthetic",
            "--split-file",
            "3000",
            "--debug-metadata-output",
        ]);
        encode.assert().success();
        let chunk = encoded
            .parent()
            .unwrap()
            .join("payload_5000")
            .join("vol0001.dng");
        let sidecar: serde_json::Value =
            serde_json::from_slice(&std::fs::read(chunk.with_extension("json")).unwrap()).unwrap();
        seeds.push((
            sidecar["cover"]["seed"].as_u64().unwrap(),
            sidecar["cover"]["metadata_seed"].as_u64().unwrap(),
        ));
    }
    assert_ne!(seeds[0], seeds[1]);
}
