//! End-to-end checks for the in-memory framing + LSB codec pipeline.
//!
//! The TIFF layer is not wired in yet, so these tests exercise exactly the
//! bytes that will later be striped into raw samples.

use rawoxide_core::{LsbCodec, PayloadFrame};

fn framed_roundtrip<S: rawoxide_core::codec::Sample + std::fmt::Debug + PartialEq>(
    cover: &mut [S],
    payload: &[u8],
    key: Option<&[u8]>,
    planes: u8,
) {
    let frame = PayloadFrame::pack(payload, key);
    let codec = LsbCodec::new(planes).unwrap();

    codec.embed(cover, &frame).unwrap();
    let recovered = codec.extract(cover, frame.len()).unwrap();
    assert_eq!(recovered, frame);

    let (unpacked, total) = PayloadFrame::unpack(&recovered, key).unwrap();
    assert_eq!(unpacked, payload);
    assert_eq!(total, frame.len());
}

#[test]
fn u16_linear_roundtrip() {
    let payload = b"the quick brown fox jumps over the lazy dog";
    let mut cover = vec![0_u16; 4096];
    framed_roundtrip(&mut cover, payload, None, 1);
}

#[test]
fn u16_keyed_roundtrip() {
    let payload = b"keyed payload";
    let mut cover = vec![0_u16; 4096];
    framed_roundtrip(
        &mut cover,
        payload,
        Some(b"correct horse battery staple"),
        2,
    );
}

#[test]
fn u8_cfa_roundtrip() {
    let payload = b"cfa payload with several planes";
    let mut cover = vec![0_u8; 4096];
    framed_roundtrip(&mut cover, payload, None, 4);
}

#[test]
fn maximum_plane_count_roundtrip() {
    let payload = b"16 planes uses the whole sample";
    let mut cover = vec![0_u16; 1024];
    framed_roundtrip(&mut cover, payload, None, 16);
}

#[test]
fn extract_rejects_oversized_request() {
    let cover = vec![0_u16; 16];
    let codec = LsbCodec::new(1).unwrap();
    assert!(codec.extract(&cover, 3).is_err());
    assert!(codec.extract(&cover, 2).is_ok());
}

#[test]
fn capacity_is_reported_in_kilobytes() {
    assert_eq!(rawoxide_core::format_kb(1_179_630), "1,152.0 KB");
    assert_eq!(rawoxide_core::format_kb_hi(1_179_630), "1,152.0 KB");
    assert_eq!(rawoxide_core::format_kb_lo(1_179_630), "1,151.9 KB");
}
