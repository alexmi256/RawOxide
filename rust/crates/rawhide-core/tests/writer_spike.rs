//! Writer-spike tests: Rust output must be readable by the Rust reader and
//! structurally valid for the Python/tifffile/rawpy toolchain.
//!
//! The Python cross-check is run in CI/manual verification because the Rust
//! tests stay hermetic; see `docs/testing.md`.

use std::path::Path;

use rawhide_core::{
    DecodeOptions, DngReader, RawMode, WriteError, WriterOptions, decode_files, write_dng,
    write_spike_dng,
};

fn payload() -> Vec<u8> {
    (0..3000)
        .map(|index| ((index * 37 + 17) & 0xFF) as u8)
        .collect()
}

#[test]
fn writes_a_decodable_linear_raw_dng() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spike.dng");
    let info = write_spike_dng(&path, &payload(), 256, 192, 1, None).unwrap();
    assert_eq!(info.width, 256);
    assert_eq!(info.height, 192);
    assert_eq!(info.payload_len, 3000);
    assert!(info.size > 0);

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"II*\0");

    let reader = DngReader::open(&path).unwrap();
    let frames = reader.raw_frames().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].width, 256);
    assert_eq!(frames[0].height, 192);
    assert_eq!(frames[0].samples_per_pixel, 3);
    assert_eq!(frames[0].bits_per_sample, 16);

    let outcome = decode_files(&[path], &DecodeOptions::default()).unwrap();
    assert_eq!(outcome.payload, payload());
}

#[test]
fn writes_a_keyed_multi_plane_dng() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keyed.dng");
    write_spike_dng(&path, &payload(), 128, 96, 4, Some(b"test-key")).unwrap();

    let options = DecodeOptions {
        key: Some(b"test-key"),
        lsb_planes: Some(4),
        ..DecodeOptions::default()
    };
    let outcome = decode_files(&[path], &options).unwrap();
    assert_eq!(outcome.payload, payload());
}

#[test]
fn rejects_oversized_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("too-big.dng");
    let error = write_spike_dng(&path, &vec![0_u8; 100_000], 64, 48, 1, None).unwrap_err();
    assert!(matches!(error, WriteError::Capacity { .. }));
    assert!(!path.exists());
}

#[test]
fn rejects_zero_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zero.dng");
    let error = write_spike_dng(&path, b"x", 0, 48, 1, None).unwrap_err();
    assert!(matches!(error, WriteError::InvalidGeometry { .. }));
}

#[test]
fn output_contains_expected_tiff_markers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("markers.dng");
    write_spike_dng(&path, b"markers", 64, 48, 1, None).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // Classic little-endian TIFF header; IFD0 lives at the end of the file.
    assert_eq!(&bytes[..4], b"II*\0");
    // SubIFDs tag (330) is present in the IFD0 directory.
    assert!(
        bytes
            .windows(2)
            .any(|window| window == 330_u16.to_le_bytes())
    );
    assert!(Path::new(&path).is_file());
}

#[test]
fn writer_matrix_roundtrips_all_modes_and_depths() {
    let payload: Vec<u8> = (0..200).map(|index| (index * 13) as u8).collect();
    let dir = tempfile::tempdir().unwrap();

    for mode in [RawMode::Linear, RawMode::Cfa] {
        for bit_depth in [8_u16, 10, 12, 14, 16] {
            let path = dir
                .path()
                .join(format!("{}_{bit_depth}.dng", mode.as_str()));
            let options = WriterOptions {
                width: 50,
                height: 30,
                mode,
                bit_depth,
                lsb_planes: 2,
                key: None,
                tile_width: 32,
                tile_height: 24,
                ..WriterOptions::default()
            };
            write_dng(&path, &payload, &options)
                .unwrap_or_else(|error| panic!("{} bit={bit_depth}: {error}", mode.as_str()));

            let reader = DngReader::open(&path).unwrap();
            let frames = reader.raw_frames().unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].width, 50);
            assert_eq!(frames[0].height, 30);
            assert_eq!(frames[0].samples_per_pixel, mode.channels());
            assert_eq!(frames[0].bits_per_sample, bit_depth);

            let decoded = decode_files(&[path], &DecodeOptions::default()).unwrap();
            assert_eq!(
                decoded.payload,
                payload,
                "{} bit={bit_depth}",
                mode.as_str()
            );
        }
    }
}

#[test]
fn rejects_invalid_depth_and_planes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.dng");
    let mut options = WriterOptions {
        width: 64,
        height: 48,
        bit_depth: 9,
        ..WriterOptions::default()
    };
    assert!(matches!(
        write_dng(&path, b"x", &options),
        Err(WriteError::InvalidBitDepth(9))
    ));

    options.bit_depth = 8;
    options.lsb_planes = 9;
    assert!(matches!(
        write_dng(&path, b"x", &options),
        Err(WriteError::InvalidPlanes { .. })
    ));
}

#[test]
fn writer_handles_multiple_raw_frames() {
    let payload: Vec<u8> = (0..1000).map(|index| (index * 17) as u8).collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frames.dng");
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Linear,
        bit_depth: 16,
        lsb_planes: 1,
        frames: 3,
        seed: 42,
        key: None,
        profile: None,
        metadata: None,
        level_policy: rawhide_core::LevelPolicy::Profile,
        split_id: None,
        split_seq: None,
        split_total: None,
        split_id_field: rawhide_core::SplitIdField::default(),
        split_seq_field: rawhide_core::SplitSeqField::default(),
        tile_width: 64,
        tile_height: 48,
    };
    write_dng(&path, &payload, &options).unwrap();
    let reader = DngReader::open(&path).unwrap();
    assert_eq!(reader.raw_frames().unwrap().len(), 3);
    let outcome = decode_files(&[path], &DecodeOptions::default()).unwrap();
    assert_eq!(outcome.payload, payload);
}

#[test]
fn writer_emits_split_markers_and_links_exif() {
    let payload = vec![7_u8; 1000];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split.dng");
    let options = WriterOptions {
        width: 128,
        height: 96,
        split_id: Some("rust-split-set"),
        split_seq: Some(1),
        split_total: Some(2),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let reader = DngReader::open(&path).unwrap();
    let markers = reader
        .split_markers(
            rawhide_core::SplitIdField::ImageUniqueId,
            rawhide_core::SplitSeqField::ImageNumber,
        )
        .unwrap();
    assert!(markers.id_present);
    assert!(markers.seq_present);
    assert_eq!(markers.id.as_deref(), Some("rust-split-set"));
    assert_eq!(markers.seq, Some(1));
    assert_eq!(markers.total, None);

    assert_eq!(
        decode_files(&[path], &DecodeOptions::default())
            .unwrap()
            .payload,
        payload
    );
}

#[test]
fn writer_emits_page_number_totals() {
    let payload = vec![9_u8; 1000];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.dng");
    let options = WriterOptions {
        width: 128,
        height: 96,
        split_id: Some("rust-split-set"),
        split_seq: Some(2),
        split_total: Some(3),
        split_seq_field: rawhide_core::SplitSeqField::PageNumber,
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let reader = DngReader::open(&path).unwrap();
    let markers = reader
        .split_markers(
            rawhide_core::SplitIdField::ImageUniqueId,
            rawhide_core::SplitSeqField::PageNumber,
        )
        .unwrap();
    assert_eq!(markers.seq, Some(2));
    assert_eq!(markers.total, Some(3));
}

#[test]
fn writer_embeds_profile_and_randomized_metadata() {
    let payload = vec![3_u8; 500];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("canon.dng");
    let profile = rawhide_core::profiles::get_profile("canon_eos_80d").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(12), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("Canon EOS 80D"));
    assert!(text.contains("Canon"));
    assert!(text.contains(&metadata.datetime));
    assert!(text.contains(&metadata.body_serial));
    assert!(text.contains(&metadata.lens_serial));
    assert!(!text.contains("Rust Spike DNG"));
    assert_eq!(
        decode_files(&[path], &DecodeOptions::default())
            .unwrap()
            .payload,
        payload
    );
}

#[test]
fn dng_tags_live_in_ifd0_not_raw_subifd() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ifd0.dng");
    let profile = rawhide_core::profiles::get_profile("canon_eos_80d").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(1), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    for tag in [50_706_u16, 50_707, 50_708, 50_721, 50_722, 50_728, 50_730] {
        assert!(ifd0.tag(tag).is_some(), "missing IFD0 tag {tag}");
    }
    let raw = file
        .read_ifd_at_offset(sub_ifd_offset_from_file(&path))
        .unwrap();
    for tag in [50_706_u16, 50_707, 50_708, 50_721, 50_722, 50_728, 50_730] {
        assert!(raw.tag(tag).is_none(), "tag {tag} leaked into raw SubIFD");
    }
    assert_eq!(
        ifd0.tag(50_735).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Ascii(
            metadata.camera_serial.clone()
        ))
    );
    let packet = match ifd0.tag(700).map(|tag| &tag.value) {
        Some(tiff_reader::TagValue::Byte(bytes)) => bytes,
        other => panic!("unexpected XMP value {other:?}"),
    };
    assert_eq!(packet.len(), 12_288);
    assert!(packet.starts_with(b"<?xpacket begin"));
    let text = String::from_utf8_lossy(packet);
    assert!(text.contains("<xmp:Rating>0</xmp:Rating>"));
    let end = b"<?xpacket end=\"w\"?>\n";
    let end_at = packet
        .windows(end.len())
        .position(|window| window == end)
        .expect("XMP end marker");
    assert!(
        packet[end_at + end.len()..]
            .iter()
            .all(|byte| *byte == b' ')
    );
}

#[test]
fn strict_level_policy_changes_emitted_white_and_black() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("strict.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(1), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: rawhide_core::RawMode::Cfa,
        profile: Some(profile),
        metadata: Some(&metadata),
        level_policy: rawhide_core::LevelPolicy::Strict,
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    let raw = file
        .read_ifd_at_offset(sub_ifd_offset_from_file(&path))
        .unwrap();
    assert_eq!(
        raw.tag(50_717).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Long(vec![16_368]))
    );
    assert_eq!(
        raw.tag(50_714).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Long(vec![1020, 1016, 1016, 1020]))
    );
    assert!(raw.tag(50_713).is_some());
    assert_eq!(
        raw.tag(33_422).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Byte(vec![2, 1, 1, 0]))
    );
    assert!(matches!(
        raw.tag(51_041).map(|tag| &tag.value),
        Some(tiff_reader::TagValue::Double(values)) if values.len() == 6
    ));
    let max_sample = DngReader::open(&path)
        .unwrap()
        .raw_frames()
        .unwrap()
        .iter()
        .flat_map(|frame| frame.samples.iter().copied())
        .max()
        .unwrap();
    assert!(
        max_sample <= 16_368,
        "strict sample {max_sample} above white"
    );
    assert!(ifd0.tag(700).is_none(), "fidelity emit_xmp=false must win");
    assert!(ifd0.tag(50_739).is_some(), "ShadowScale missing");
    assert!(
        ifd0.tag(50_740).is_none(),
        "DNGPrivateData should not be written"
    );
    assert!(matches!(
        ifd0.tag(50_728).map(|tag| &tag.value),
        Some(tiff_reader::TagValue::Rational(values))
            if values.len() == 3 && values[1] == [10_000, 10_000]
    ));
}

#[test]
fn fidelity_tag_flags_and_allowlist_are_honoured() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custom.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(4), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: rawhide_core::RawMode::Cfa,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let exif = file.ifd(0).unwrap().tag(34_665).unwrap();
    let offset = match &exif.value {
        tiff_reader::TagValue::Long(values) => values[0],
        other => panic!("unexpected Exif IFD pointer {other:?}"),
    };
    let exif_ifd = file.read_ifd_at_offset(u64::from(offset)).unwrap();
    for tag in [42_034_u16, 42_035, 42_036, 42_037, 42_033, 34_850, 36_880] {
        assert!(exif_ifd.tag(tag).is_none(), "tag {tag} should be filtered");
    }
    assert!(file.ifd(0).unwrap().tag(700).is_none());
    assert!(exif_ifd.tag(37_382).is_some());
    assert!(exif_ifd.tag(41_996).is_some());
}

fn sub_ifd_offset_from_file(path: &std::path::Path) -> u64 {
    let bytes = std::fs::read(path).unwrap();
    let ifd_offset = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let count = u16::from_le_bytes([bytes[ifd_offset], bytes[ifd_offset + 1]]) as usize;
    for index in 0..count {
        let entry = ifd_offset + 2 + index * 12;
        let tag = u16::from_le_bytes([bytes[entry], bytes[entry + 1]]);
        if tag == 330 {
            let value = &bytes[entry + 8..entry + 12];
            return u64::from(u32::from_le_bytes([value[0], value[1], value[2], value[3]]));
        }
    }
    panic!("no SubIFDs tag");
}

#[test]
fn linear_mode_ignores_crop_margin_like_python() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("linear-crop.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Linear,
        profile: Some(profile),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let raw = file
        .read_ifd_at_offset(sub_ifd_offset_from_file(&path))
        .unwrap();
    assert_eq!(
        raw.tag(50_719).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Rational(vec![[0, 1], [0, 1]]))
    );
    assert_eq!(
        raw.tag(50_720).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Rational(vec![[128, 1], [96, 1]]))
    );
}

#[test]
fn cfa_crop_margin_that_leaves_no_image_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crop.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let options = WriterOptions {
        width: 16,
        height: 16,
        mode: RawMode::Cfa,
        profile: Some(profile),
        ..WriterOptions::default()
    };
    let error = write_dng(&path, &[1_u8; 16], &options).unwrap_err();
    assert!(matches!(error, WriteError::InvalidCrop { .. }), "{error}");
    assert!(!path.exists());
}

#[test]
fn writer_emits_gps_ifd_for_gps_profiles() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gps.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(4), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Cfa,
        frames: 2,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    assert_eq!(
        DngReader::open(&path).unwrap().raw_frames().unwrap().len(),
        2
    );
    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let pointer = file.ifd(0).unwrap().tag(34_853).expect("GPSTag");
    let offset = match &pointer.value {
        tiff_reader::TagValue::Long(values) => values[0],
        other => panic!("unexpected GPS pointer {other:?}"),
    };
    let gps = file.read_ifd_at_offset(u64::from(offset)).unwrap();
    for tag in [0_u16, 1, 2, 3, 4, 5, 6, 7, 11, 27, 29] {
        assert!(gps.tag(tag).is_some(), "missing GPS tag {tag}");
    }
}

#[test]
fn gps_ifd_is_valid_when_exif_is_absent() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gps-no-exif.dng");
    let mut profile = rawhide_core::profiles::get_profile("custom_pixel_xl")
        .unwrap()
        .clone();
    profile.fidelity.as_mut().unwrap().exif_allow = Some(Vec::new());
    let metadata = rawhide_core::MetadataRandomizer::new(Some(4), Some(&profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Cfa,
        profile: Some(&profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    assert!(ifd0.tag(34_665).is_none(), "EXIF should be filtered out");
    let pointer = ifd0.tag(34_853).expect("GPSTag");
    let offset = match &pointer.value {
        tiff_reader::TagValue::Long(values) => values[0],
        other => panic!("unexpected GPS pointer {other:?}"),
    };
    let gps = file.read_ifd_at_offset(u64::from(offset)).unwrap();
    assert!(gps.tag(2).is_some());
    assert!(gps.tag(29).is_some());
}

#[test]
fn raw_digest_tags_are_written_for_fidelity_profiles() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("digests.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(4), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Cfa,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    for tag in [51_111_u16, 50_781] {
        let value = ifd0.tag(tag).unwrap();
        assert!(matches!(&value.value, tiff_reader::TagValue::Byte(bytes) if bytes.len() == 16));
    }
}

#[test]
fn makernote_is_written_when_the_profile_requests_it() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("makernote.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let metadata = rawhide_core::MetadataRandomizer::new(Some(4), Some(profile)).randomize(None);
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Cfa,
        profile: Some(profile),
        metadata: Some(&metadata),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();

    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    assert_eq!(
        ifd0.tag(50_741).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Short(vec![1]))
    );
    let exif_pointer = ifd0.tag(34_665).unwrap();
    let offset = match &exif_pointer.value {
        tiff_reader::TagValue::Long(values) => values[0],
        other => panic!("unexpected Exif IFD pointer {other:?}"),
    };
    let exif = file.read_ifd_at_offset(u64::from(offset)).unwrap();
    let value = exif.tag(37_500).unwrap();
    match &value.value {
        tiff_reader::TagValue::Undefined(bytes) => {
            assert!((2640..=2757).contains(&bytes.len()));
            assert!(bytes.starts_with(b"HDRP"));
        }
        other => panic!("unexpected MakerNote value {other:?}"),
    }
}

#[test]
fn multi_frame_files_omit_raw_digest_tags() {
    let payload = vec![5_u8; 200];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frames-digests.dng");
    let profile = rawhide_core::profiles::get_profile("custom_pixel_xl").unwrap();
    let options = WriterOptions {
        width: 128,
        height: 96,
        mode: RawMode::Cfa,
        frames: 2,
        profile: Some(profile),
        ..WriterOptions::default()
    };
    write_dng(&path, &payload, &options).unwrap();
    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd0 = file.ifd(0).unwrap();
    assert!(ifd0.tag(51_111).is_none());
    assert!(ifd0.tag(50_781).is_none());
}
