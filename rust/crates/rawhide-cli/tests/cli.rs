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
fn gen_tiff_scaffold_fails_clearly() {
    let mut command = cargo_bin_cmd!("rawhide");
    command.args(["gen-tiff", "-o", "cover.tif"]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("not implemented"))
        .stderr(predicate::str::contains("port-plan.md"));
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
