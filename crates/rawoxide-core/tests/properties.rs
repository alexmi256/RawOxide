//! Property tests for the framing and LSB codec.
//!
//! These complement the deterministic unit tests: random payloads, keys and
//! plane counts must always round-trip, and any request beyond capacity must
//! fail rather than silently truncate.

use proptest::prelude::*;
use rawoxide_core::{LsbCodec, PayloadFrame};

proptest! {
    #[test]
    fn framing_roundtrips(payload in proptest::collection::vec(any::<u8>(), 0..4096), key in proptest::collection::vec(any::<u8>(), 0..32)) {
        let key = if key.is_empty() { None } else { Some(key.as_slice()) };
        let frame = PayloadFrame::pack(&payload, key);
        let (recovered, total) = PayloadFrame::unpack(&frame, key).unwrap();
        prop_assert_eq!(recovered, payload);
        prop_assert_eq!(total, frame.len());
    }

    #[test]
    fn u16_embed_extract_roundtrips(
        payload in proptest::collection::vec(any::<u8>(), 0..2048),
        planes in 1_u8..=16,
        cover_len in 1_usize..8192,
    ) {
        let frame = PayloadFrame::pack(&payload, None);
        let needed_bits = frame.len().saturating_mul(8);
        prop_assume!(needed_bits <= cover_len * usize::from(planes));
        let mut cover = vec![0_u16; cover_len];
        let codec = LsbCodec::new(planes).unwrap();
        codec.embed(&mut cover, &frame).unwrap();
        let recovered = codec.extract(&cover, frame.len()).unwrap();
        prop_assert_eq!(recovered, frame);
    }

    #[test]
    fn u8_embed_extract_roundtrips(
        payload in proptest::collection::vec(any::<u8>(), 0..1024),
        planes in 1_u8..=8,
        cover_len in 1_usize..4096,
    ) {
        let frame = PayloadFrame::pack(&payload, None);
        let needed_bits = frame.len().saturating_mul(8);
        prop_assume!(needed_bits <= cover_len * usize::from(planes));
        let mut cover = vec![0_u8; cover_len];
        let codec = LsbCodec::new(planes).unwrap();
        codec.embed(&mut cover, &frame).unwrap();
        let recovered = codec.extract(&cover, frame.len()).unwrap();
        prop_assert_eq!(recovered, frame);
    }

    #[test]
    fn oversized_payload_is_rejected(payload in proptest::collection::vec(any::<u8>(), 1..256), planes in 1_u8..=8) {
        let frame = PayloadFrame::pack(&payload, None);
        let needed_bits = frame.len().saturating_mul(8);
        let cover_len = (needed_bits / usize::from(planes)).saturating_sub(1);
        let mut cover = vec![0_u16; cover_len];
        let codec = LsbCodec::new(planes).unwrap();
        let err = codec.embed(&mut cover, &frame).unwrap_err();
        let too_large = matches!(err, rawoxide_core::CodecError::PayloadTooLarge { .. });
        prop_assert!(too_large);
    }
}
