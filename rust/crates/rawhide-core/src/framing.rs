//! Payload framing: magic, version, flags, length and CRC32.
//!
//! The framed stream is what is embedded in (and recovered from) the LSB
//! planes. With a key, the header and body are XOR-masked with a SHA-256
//! counter keystream. This is confidentiality against casual inspection, not
//! authenticated encryption; a wrong key fails closed on the magic/CRC check.
//!
//! The wire format matches the Python reference:
//!
//! ```text
//! offset  size  field
//! 0       4     magic "DNGS"
//! 4       1     version (1)
//! 5       1     flags (bit 0 = keyed)
//! 6       8     payload length, big-endian u64
//! 14      4     CRC32 of the plaintext payload, big-endian u32
//! 18      N     payload (XOR-masked when keyed)
//! ```

use sha2::{Digest, Sha256};

use crate::error::FrameError;

/// Parsed fixed-size frame header.
///
/// `payload_len` is the declared plaintext payload length; the total frame is
/// [`PayloadFrame::HEADER_LEN`] plus that many bytes. `crc32` covers the
/// plaintext payload, not the XOR-masked body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Frame format version.
    pub version: u8,
    /// Header flags (bit 0 = keyed).
    pub flags: u8,
    /// Declared plaintext payload length in bytes.
    pub payload_len: u64,
    /// CRC32 of the plaintext payload.
    pub crc32: u32,
}

impl FrameHeader {
    /// Whether the frame was written with a key.
    #[must_use]
    pub const fn is_keyed(self) -> bool {
        self.flags & PayloadFrame::FLAG_KEYED != 0
    }
}

/// Payload framing helpers.
#[derive(Debug, Clone, Copy, Default)]
pub struct PayloadFrame;

impl PayloadFrame {
    /// Frame magic bytes.
    pub const MAGIC: [u8; 4] = *b"DNGS";
    /// Frame format version understood by this implementation.
    pub const VERSION: u8 = 1;
    /// Size of the fixed header in bytes.
    pub const HEADER_LEN: usize = 18;
    /// Header flag set when a key was used.
    pub const FLAG_KEYED: u8 = 0x01;

    /// Chunk size for streaming XOR, mirroring the Python implementation.
    const XOR_CHUNK: usize = 4 << 20;

    /// Frame `payload`, optionally masking it with `key`.
    ///
    /// The returned bytes are exactly what should be striped into the raw
    /// samples.
    #[must_use]
    pub fn pack(payload: &[u8], key: Option<&[u8]>) -> Vec<u8> {
        let keyed = key.is_some_and(|key| !key.is_empty());
        let header = Self::masked_header(
            u64::try_from(payload.len()).unwrap_or(u64::MAX),
            crc32fast::hash(payload),
            key,
        );

        let mut framed = Vec::with_capacity(Self::HEADER_LEN + payload.len());
        framed.extend_from_slice(&header);
        if let Some(key) = key.filter(|_| keyed) {
            for (index, chunk) in payload.chunks(Self::XOR_CHUNK).enumerate() {
                let offset = (index * Self::XOR_CHUNK) as u64;
                let keystream = Self::keystream_range(key, offset, chunk.len());
                framed.extend(chunk.iter().zip(keystream).map(|(byte, mask)| byte ^ mask));
            }
        } else {
            framed.extend_from_slice(payload);
        }
        framed
    }

    /// Masked frame header for a payload length and CRC.
    pub(crate) fn masked_header(
        payload_len: u64,
        crc32: u32,
        key: Option<&[u8]>,
    ) -> [u8; Self::HEADER_LEN] {
        let keyed = key.is_some_and(|key| !key.is_empty());
        let mut header = [0_u8; Self::HEADER_LEN];
        header[..4].copy_from_slice(&Self::MAGIC);
        header[4] = Self::VERSION;
        header[5] = if keyed { Self::FLAG_KEYED } else { 0 };
        header[6..14].copy_from_slice(&payload_len.to_be_bytes());
        header[14..18].copy_from_slice(&crc32.to_be_bytes());
        if let Some(key) = key.filter(|_| keyed) {
            let masked = Self::xor_data(&header, key);
            header.copy_from_slice(&masked);
        }
        header
    }

    /// Parse the fixed frame header without requiring the body.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] for a short header, bad magic or an unsupported
    /// version.
    pub fn parse_header(stream: &[u8], key: Option<&[u8]>) -> Result<FrameHeader, FrameError> {
        if stream.len() < Self::HEADER_LEN {
            return Err(FrameError::HeaderTooShort {
                need: Self::HEADER_LEN,
                len: stream.len(),
            });
        }

        let mut header = [0_u8; Self::HEADER_LEN];
        header.copy_from_slice(&stream[..Self::HEADER_LEN]);
        if let Some(key) = key.filter(|key| !key.is_empty()) {
            let unmasked = Self::xor_data(&header, key);
            header.copy_from_slice(&unmasked);
        }

        if header[..4] != Self::MAGIC {
            return Err(FrameError::BadMagic);
        }
        let version = header[4];
        if version != Self::VERSION {
            return Err(FrameError::UnsupportedVersion(version));
        }

        let mut len_bytes = [0_u8; 8];
        len_bytes.copy_from_slice(&header[6..14]);
        let mut crc_bytes = [0_u8; 4];
        crc_bytes.copy_from_slice(&header[14..18]);

        Ok(FrameHeader {
            version,
            flags: header[5],
            payload_len: u64::from_be_bytes(len_bytes),
            crc32: u32::from_be_bytes(crc_bytes),
        })
    }

    /// Unpack a framed stream, returning `(payload, total_frame_len)`.
    ///
    /// `total_frame_len` is the number of bytes the frame occupies, which lets
    /// callers ignore trailing bytes (other frames or padding).
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] when the header is truncated, the magic or
    /// version is wrong, the declared payload is truncated, or the CRC32 does
    /// not match.
    pub fn unpack(stream: &[u8], key: Option<&[u8]>) -> Result<(Vec<u8>, usize), FrameError> {
        let header = Self::parse_header(stream, key)?;
        let total = usize::try_from(header.payload_len)
            .ok()
            .and_then(|len| len.checked_add(Self::HEADER_LEN))
            .ok_or(FrameError::LengthOverflow {
                declared: header.payload_len,
            })?;
        if stream.len() < total {
            return Err(FrameError::Truncated {
                need: total,
                have: stream.len(),
            });
        }

        let body = if let Some(key) = key.filter(|key| !key.is_empty()) {
            Self::xor_data(&stream[Self::HEADER_LEN..total], key)
        } else {
            stream[Self::HEADER_LEN..total].to_vec()
        };

        if crc32fast::hash(&body) != header.crc32 {
            return Err(FrameError::CrcMismatch);
        }
        Ok((body, total))
    }

    /// Unpack an owned framed stream, reusing its buffer for the payload.
    ///
    /// Unlike [`Self::unpack`], this does not allocate a second payload buffer:
    /// the header is dropped by shifting the body left in place.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] under the same conditions as [`Self::unpack`].
    pub fn unpack_owned(
        mut stream: Vec<u8>,
        key: Option<&[u8]>,
    ) -> Result<(Vec<u8>, usize), FrameError> {
        let header = Self::parse_header(&stream, key)?;
        let total = usize::try_from(header.payload_len)
            .ok()
            .and_then(|len| len.checked_add(Self::HEADER_LEN))
            .ok_or(FrameError::LengthOverflow {
                declared: header.payload_len,
            })?;
        if stream.len() < total {
            return Err(FrameError::Truncated {
                need: total,
                have: stream.len(),
            });
        }

        if let Some(key) = key.filter(|key| !key.is_empty()) {
            Self::xor_in_place(&mut stream[Self::HEADER_LEN..total], key);
        }
        if crc32fast::hash(&stream[Self::HEADER_LEN..total]) != header.crc32 {
            return Err(FrameError::CrcMismatch);
        }
        stream.copy_within(Self::HEADER_LEN..total, 0);
        stream.truncate(total - Self::HEADER_LEN);
        Ok((stream, total))
    }

    fn xor_in_place(data: &mut [u8], key: &[u8]) {
        for (index, chunk) in data.chunks_mut(Self::XOR_CHUNK).enumerate() {
            let offset = (index * Self::XOR_CHUNK) as u64;
            let keystream = Self::keystream_range(key, offset, chunk.len());
            for (byte, mask) in chunk.iter_mut().zip(keystream) {
                *byte ^= mask;
            }
        }
    }

    /// SHA-256 counter keystream slice `[offset, offset + length)`.
    ///
    /// The counter is a big-endian `u64` appended to the key, matching the
    /// Python reference byte-for-byte.
    #[must_use]
    pub fn keystream_range(key: &[u8], offset: u64, length: usize) -> Vec<u8> {
        if length == 0 {
            return Vec::new();
        }
        let first_counter = offset / 32;
        let skip = usize::try_from(offset % 32).expect("offset % 32 always fits in usize");
        let need = skip + length;
        let hash_count = need.div_ceil(32);
        let mut material = Vec::with_capacity(hash_count * 32);

        for counter in first_counter..first_counter + hash_count as u64 {
            let mut hasher = Sha256::new();
            hasher.update(key);
            hasher.update(counter.to_be_bytes());
            material.extend_from_slice(&hasher.finalize());
        }

        material[skip..skip + length].to_vec()
    }

    fn xor_data(data: &[u8], key: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for (index, chunk) in data.chunks(Self::XOR_CHUNK).enumerate() {
            let offset = (index * Self::XOR_CHUNK) as u64;
            let keystream = Self::keystream_range(key, offset, chunk.len());
            out.extend(chunk.iter().zip(keystream).map(|(byte, mask)| byte ^ mask));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN_KEYED_FRAME: &str =
        "450640131939666cb76e22aae90510311e7273297028715c0341d0014ece8c65";
    const GOLDEN_KEYSTREAM_64: &str = concat!(
        "014807401838666cb76e22aae90b3b0a22959ae2fb40621b1d09e862eb9653e5",
        "fa47b9e67455721b1993d175081173b9d142cdb299f3656010bf2b864cbd7db1"
    );
    const GOLDEN_PLAIN_FRAME: &str =
        "444e47530100000000000000000e2b3b3ce7726177686964652d676f6c64656e";
    const GOLDEN_EMPTY_FRAME: &str = "444e47530100000000000000000000000000";

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(out, "{byte:02x}").expect("writing to String cannot fail");
        }
        out
    }

    #[test]
    fn unkeyed_roundtrip() {
        let payload = b"hello rawhide";
        let frame = PayloadFrame::pack(payload, None);
        assert_eq!(frame.len(), PayloadFrame::HEADER_LEN + payload.len());
        let (recovered, total) = PayloadFrame::unpack(&frame, None).unwrap();
        assert_eq!(recovered, payload);
        assert_eq!(total, frame.len());
    }

    #[test]
    fn keyed_roundtrip() {
        let payload = b"hello rawhide";
        let key = b"secret";
        let frame = PayloadFrame::pack(payload, Some(key));
        let (recovered, total) = PayloadFrame::unpack(&frame, Some(key)).unwrap();
        assert_eq!(recovered, payload);
        assert_eq!(total, frame.len());
    }

    #[test]
    fn keyed_frame_matches_python_golden_vector() {
        let frame = PayloadFrame::pack(b"rawhide-golden", Some(b"secret"));
        assert_eq!(hex(&frame), GOLDEN_KEYED_FRAME);
    }

    #[test]
    fn plain_frame_matches_python_golden_vector() {
        assert_eq!(
            hex(&PayloadFrame::pack(b"rawhide-golden", None)),
            GOLDEN_PLAIN_FRAME
        );
        assert_eq!(hex(&PayloadFrame::pack(b"", None)), GOLDEN_EMPTY_FRAME);
    }

    #[test]
    fn keystream_matches_python_golden_vector() {
        let stream = PayloadFrame::keystream_range(b"secret", 0, 64);
        assert_eq!(hex(&stream), GOLDEN_KEYSTREAM_64);
    }

    #[test]
    fn keystream_offset_is_a_slice_of_the_prefix() {
        let prefix = PayloadFrame::keystream_range(b"secret", 0, 100);
        let slice = PayloadFrame::keystream_range(b"secret", 13, 40);
        assert_eq!(slice, prefix[13..53]);
    }

    #[test]
    fn parse_header_matches_pack() {
        let frame = PayloadFrame::pack(b"header", Some(b"secret"));
        let header = PayloadFrame::parse_header(&frame, Some(b"secret")).unwrap();
        assert_eq!(header.version, PayloadFrame::VERSION);
        assert!(header.is_keyed());
        assert_eq!(header.payload_len, 6);
        assert_eq!(header.crc32, crc32fast::hash(b"header"));
    }

    #[test]
    fn parse_header_rejects_short_input() {
        assert_eq!(
            PayloadFrame::parse_header(&[0_u8; 17], None).unwrap_err(),
            FrameError::HeaderTooShort {
                need: PayloadFrame::HEADER_LEN,
                len: 17,
            }
        );
    }

    #[test]
    fn unpack_ignores_trailing_bytes() {
        let payload = b"abc";
        let mut frame = PayloadFrame::pack(payload, None);
        frame.extend_from_slice(b"trailing");
        let (recovered, total) = PayloadFrame::unpack(&frame, None).unwrap();
        assert_eq!(recovered, payload);
        assert_eq!(total, PayloadFrame::HEADER_LEN + payload.len());
    }

    #[test]
    fn rejects_short_header() {
        let err = PayloadFrame::unpack(&[0_u8; 3], None).unwrap_err();
        assert_eq!(
            err,
            FrameError::HeaderTooShort {
                need: PayloadFrame::HEADER_LEN,
                len: 3
            }
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let frame = PayloadFrame::pack(b"abc", None);
        let mut corrupted = frame;
        corrupted[0] = b'X';
        assert_eq!(
            PayloadFrame::unpack(&corrupted, None).unwrap_err(),
            FrameError::BadMagic
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        let frame = PayloadFrame::pack(b"abc", None);
        let mut corrupted = frame;
        corrupted[4] = 99;
        assert_eq!(
            PayloadFrame::unpack(&corrupted, None).unwrap_err(),
            FrameError::UnsupportedVersion(99)
        );
    }

    #[test]
    fn rejects_truncated_payload() {
        let frame = PayloadFrame::pack(b"abcdef", None);
        let truncated = &frame[..frame.len() - 1];
        assert_eq!(
            PayloadFrame::unpack(truncated, None).unwrap_err(),
            FrameError::Truncated {
                need: frame.len(),
                have: truncated.len()
            }
        );
    }

    #[test]
    fn rejects_crc_mismatch() {
        let mut frame = PayloadFrame::pack(b"abcdef", None);
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert_eq!(
            PayloadFrame::unpack(&frame, None).unwrap_err(),
            FrameError::CrcMismatch
        );
    }

    #[test]
    fn wrong_key_fails_closed() {
        let frame = PayloadFrame::pack(b"abcdef", Some(b"correct"));
        assert_eq!(
            PayloadFrame::unpack(&frame, Some(b"wrong")).unwrap_err(),
            FrameError::BadMagic
        );
    }

    #[test]
    fn owned_unpack_matches_borrowed_unpack() {
        let payload: Vec<u8> = (0..(4 << 20) + 37)
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();
        for key in [None, Some(&b"stream-key"[..])] {
            let frame = PayloadFrame::pack(&payload, key);
            let (borrowed, borrowed_len) = PayloadFrame::unpack(&frame, key).unwrap();
            let (owned, owned_len) = PayloadFrame::unpack_owned(frame.clone(), key).unwrap();
            assert_eq!(borrowed_len, owned_len);
            assert_eq!(borrowed, owned);
            assert_eq!(owned, payload);
        }
    }
}
