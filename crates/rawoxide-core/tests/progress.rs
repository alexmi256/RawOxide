//! Progress-callback contract tests for the writer and decoder.

use std::path::PathBuf;

use rawoxide_core::{
    DecodeOptions, PreviewSource, RawMode, SplitIdField, SplitSeqField, WriterOptions,
    decode_files_to_writer_with_progress, write_dng, write_dng_from_file_with_progress,
};

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 37 % 251) as u8).collect()
}

#[test]
fn writer_reports_rows_for_single_and_multi_frame_files() {
    let dir = tempfile::tempdir().unwrap();
    let payload_path = dir.path().join("payload.bin");
    let body = payload(100);
    std::fs::write(&payload_path, &body).unwrap();

    for frames in [1_u8, 2] {
        let output = dir.path().join(format!("frames-{frames}.dng"));
        let options = WriterOptions {
            width: 64,
            height: 48,
            mode: RawMode::Linear,
            frames,
            seed: 3,
            preview: PreviewSource::Synthetic,
            ..WriterOptions::default()
        };
        let mut events = Vec::new();
        write_dng_from_file_with_progress(
            &output,
            &payload_path,
            0,
            u64::try_from(body.len()).unwrap(),
            &options,
            |done, total| events.push((done, total)),
        )
        .unwrap();

        let total = u64::from(frames) * 48;
        assert_eq!(events.first(), Some(&(0, total)));
        assert_eq!(events.last(), Some(&(total, total)));
        assert!(
            events.windows(2).all(|pair| pair[0].0 <= pair[1].0),
            "{events:?}"
        );
        assert!(events.iter().all(|(_, seen)| *seen == total));
    }
}

#[test]
fn decoder_reports_per_file_bytes_and_resets_for_each_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let body = payload(300);
    let first = dir.path().join("vol0001.dng");
    let second = dir.path().join("vol0002.dng");
    let options_for = |seq: u32, total: u32| WriterOptions {
        width: 64,
        height: 48,
        mode: RawMode::Linear,
        seed: 3,
        split_id: Some("progress-set"),
        split_seq: Some(seq),
        split_total: Some(total),
        split_id_field: SplitIdField::ImageUniqueId,
        split_seq_field: SplitSeqField::PageNumber,
        preview: PreviewSource::Synthetic,
        ..WriterOptions::default()
    };
    write_dng(&first, &body[..200], &options_for(1, 2)).unwrap();
    write_dng(&second, &body[200..], &options_for(2, 2)).unwrap();

    let mut decoded = Vec::new();
    let mut events = Vec::new();
    let result = decode_files_to_writer_with_progress(
        &[first, second],
        &DecodeOptions {
            split_seq_field: SplitSeqField::PageNumber,
            ..DecodeOptions::default()
        },
        &mut decoded,
        |index, count, done, total| events.push((index, count, done, total)),
    )
    .unwrap();

    assert_eq!(decoded, body);
    assert_eq!(result.payload_len, 300);
    assert_eq!(events.first(), Some(&(0, 2, 0, 200)));
    assert!(events.contains(&(0, 2, 200, 200)), "{events:?}");
    assert!(events.contains(&(1, 2, 0, 100)), "{events:?}");
    assert_eq!(events.last(), Some(&(1, 2, 100, 100)));
    for pair in events.windows(2) {
        if pair[0].0 == pair[1].0 {
            assert!(pair[0].2 <= pair[1].2, "{events:?}");
        }
    }
}

#[test]
fn single_file_decode_reports_zero_then_final_total() {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("single.dng");
    let body = payload(100);
    write_dng(
        &path,
        &body,
        &WriterOptions {
            width: 64,
            height: 48,
            mode: RawMode::Linear,
            seed: 3,
            preview: PreviewSource::Synthetic,
            ..WriterOptions::default()
        },
    )
    .unwrap();

    let mut decoded = Vec::new();
    let mut events = Vec::new();
    decode_files_to_writer_with_progress(
        &[path],
        &DecodeOptions::default(),
        &mut decoded,
        |index, count, done, total| events.push((index, count, done, total)),
    )
    .unwrap();
    assert_eq!(decoded, body);
    assert_eq!(events.first(), Some(&(0, 1, 0, 100)));
    assert_eq!(events.last(), Some(&(0, 1, 100, 100)));
}
