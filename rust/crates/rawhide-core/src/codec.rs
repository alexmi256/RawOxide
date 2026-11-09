//! LSB steganography codec for raw samples.
//!
//! Bit `i` of the framed payload goes to plane `i % lsb_planes` of sample
//! `i / lsb_planes`, spreading the payload over all tiles/channels. The mapping
//! is identical to the Python implementation, so files written by either tool
//! can be decoded by the other.
//!
//! The current implementation works on an in-memory slice. The production
//! encoder will call the same mapping from a banded/tiled streaming pipeline;
//! keeping the mapping in one place is deliberate.

use crate::error::CodecError;
use crate::framing::PayloadFrame;

/// A raw sample type that can carry LSB planes.
///
/// Implemented for `u8` (8-bit raws) and `u16` (10/12/14/16-bit raws stored in
/// a 16-bit container).
pub trait Sample: Copy {
    /// Number of bits in one sample.
    const BITS: u8;

    /// Return `self` with `plane` set to `bit` (0 or 1).
    #[must_use]
    fn with_lsb(self, plane: u8, bit: u8) -> Self;

    /// Return the value of `plane`.
    fn lsb(self, plane: u8) -> u8;
}

impl Sample for u8 {
    // `Self` is the sample type; the width is deliberately a plain u8.
    #[allow(clippy::use_self)]
    const BITS: u8 = 8;

    fn with_lsb(self, plane: u8, bit: u8) -> Self {
        (self & !(1_u8 << plane)) | ((bit & 1) << plane)
    }

    fn lsb(self, plane: u8) -> u8 {
        (self >> plane) & 1
    }
}

impl Sample for u16 {
    const BITS: u8 = 16;

    fn with_lsb(self, plane: u8, bit: u8) -> Self {
        (self & !(1_u16 << plane)) | (Self::from(bit & 1) << plane)
    }

    fn lsb(self, plane: u8) -> u8 {
        ((self >> plane) & 1) as u8
    }
}

/// LSB embed/extract codec over raw sample slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LsbCodec {
    lsb_planes: u8,
}

impl LsbCodec {
    /// Create a codec using `lsb_planes` (1-16).
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidPlanes`] when `lsb_planes` is 0 or greater
    /// than 16.
    pub fn new(lsb_planes: u8) -> Result<Self, CodecError> {
        if !(1..=16).contains(&lsb_planes) {
            return Err(CodecError::InvalidPlanes(lsb_planes));
        }
        Ok(Self { lsb_planes })
    }

    /// Number of LSB planes used by this codec.
    #[must_use]
    pub const fn lsb_planes(self) -> u8 {
        self.lsb_planes
    }

    /// Payload capacity in bytes for `n_samples` samples.
    #[must_use]
    pub fn capacity(self, n_samples: u64) -> u64 {
        capacity_bytes(n_samples, self.lsb_planes)
    }

    /// Embed a framed payload into `cover` in place.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::PayloadTooLarge`] when the frame does not fit, or
    /// [`CodecError::PlanesExceedSampleBits`] when the sample type is too
    /// narrow for the configured plane count.
    pub fn embed<S: Sample>(self, cover: &mut [S], frame: &[u8]) -> Result<(), CodecError> {
        embed_frame(cover, frame, self.lsb_planes)
    }

    /// Extract `n_bytes` bytes of a framed payload from `samples`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::RequestTooLarge`] when the request exceeds the
    /// sample capacity.
    pub fn extract<S: Sample>(self, samples: &[S], n_bytes: usize) -> Result<Vec<u8>, CodecError> {
        extract_frame(samples, n_bytes, self.lsb_planes)
    }
}

/// Payload capacity in bytes for one frame of `n_samples` samples.
///
/// Uses saturating subtraction: a frame header always costs
/// [`PayloadFrame::HEADER_LEN`] bytes, so tiny images report zero capacity
/// instead of a negative number.
#[must_use]
pub fn capacity_bytes(n_samples: u64, lsb_planes: u8) -> u64 {
    (u128::from(n_samples) * u128::from(lsb_planes) / 8)
        .saturating_sub(u128::from(PayloadFrame::HEADER_LEN as u64))
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Total payload capacity in bytes across `frames` equal frames.
///
/// The frame header is charged once for the whole bitstream, matching the
/// Python `capacity_bytes_total` helper.
#[must_use]
pub fn capacity_bytes_total(n_samples_per_frame: u64, lsb_planes: u8, frames: u64) -> u64 {
    (u128::from(n_samples_per_frame) * u128::from(lsb_planes) * u128::from(frames) / 8)
        .saturating_sub(u128::from(PayloadFrame::HEADER_LEN as u64))
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Embed unpacked bits (0/1) into `cover` in place.
///
/// # Errors
///
/// Returns [`CodecError::InvalidBit`] if a value other than 0 or 1 is present,
/// or [`CodecError::PayloadTooLarge`] when the bits do not fit.
pub fn embed_bits<S: Sample>(
    cover: &mut [S],
    bits: &[u8],
    lsb_planes: u8,
) -> Result<(), CodecError> {
    validate_planes::<S>(lsb_planes)?;
    let capacity = u64::try_from(cover.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(lsb_planes));
    let need = u64::try_from(bits.len()).unwrap_or(u64::MAX);
    if need > capacity {
        return Err(CodecError::PayloadTooLarge { need, capacity });
    }

    let planes = usize::from(lsb_planes);
    if let Some(&invalid) = bits.iter().find(|&&bit| bit > 1) {
        return Err(CodecError::InvalidBit(invalid));
    }
    for (index, &bit) in bits.iter().enumerate() {
        let sample_index = index / planes;
        let plane = u8::try_from(index % planes).expect("plane index fits in u8");
        cover[sample_index] = cover[sample_index].with_lsb(plane, bit);
    }
    Ok(())
}

/// Extract `n_bits` LSB values (0/1) from `samples` in embed order.
///
/// # Errors
///
/// Returns [`CodecError::RequestTooLarge`] when the request exceeds the sample
/// capacity.
pub fn extract_bits<S: Sample>(
    samples: &[S],
    n_bits: usize,
    lsb_planes: u8,
) -> Result<Vec<u8>, CodecError> {
    validate_planes::<S>(lsb_planes)?;
    let capacity = u64::try_from(samples.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(lsb_planes));
    let need = u64::try_from(n_bits).unwrap_or(u64::MAX);
    if need > capacity {
        return Err(CodecError::RequestTooLarge { need, capacity });
    }

    let planes = usize::from(lsb_planes);
    let mut bits = Vec::with_capacity(n_bits);
    for index in 0..n_bits {
        let sample_index = index / planes;
        let plane = u8::try_from(index % planes).expect("plane index fits in u8");
        bits.push(samples[sample_index].lsb(plane));
    }
    Ok(bits)
}

/// Embed a framed byte stream directly, without materialising eight bits per
/// byte.
///
/// # Errors
///
/// See [`embed_bits`].
pub fn embed_frame<S: Sample>(
    cover: &mut [S],
    frame: &[u8],
    lsb_planes: u8,
) -> Result<(), CodecError> {
    validate_planes::<S>(lsb_planes)?;
    let capacity = u64::try_from(cover.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(lsb_planes));
    let need = u64::try_from(frame.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(8);
    if need > capacity {
        return Err(CodecError::PayloadTooLarge { need, capacity });
    }

    let planes = usize::from(lsb_planes);
    for (byte_index, &byte) in frame.iter().enumerate() {
        for bit_index in 0..8 {
            let index = byte_index * 8 + bit_index;
            let bit = (byte >> (7 - bit_index)) & 1;
            let sample_index = index / planes;
            let plane = u8::try_from(index % planes).expect("plane index fits in u8");
            cover[sample_index] = cover[sample_index].with_lsb(plane, bit);
        }
    }
    Ok(())
}

/// Extract `n_bytes` framed bytes from `samples`.
///
/// # Errors
///
/// See [`extract_bits`].
pub fn extract_frame<S: Sample>(
    samples: &[S],
    n_bytes: usize,
    lsb_planes: u8,
) -> Result<Vec<u8>, CodecError> {
    let n_bits = n_bytes
        .checked_mul(8)
        .ok_or_else(|| CodecError::RequestTooLarge {
            need: u64::MAX,
            capacity: u64::try_from(samples.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(lsb_planes)),
        })?;
    let bits = extract_bits(samples, n_bits, lsb_planes)?;
    let mut out = vec![0_u8; n_bytes];
    let (chunks, _remainder) = bits.as_chunks::<8>();
    for (byte_index, chunk) in chunks.iter().enumerate() {
        let mut byte = 0_u8;
        for (bit_index, &bit) in chunk.iter().enumerate() {
            byte |= bit << (7 - bit_index);
        }
        out[byte_index] = byte;
    }
    Ok(out)
}

fn validate_planes<S: Sample>(lsb_planes: u8) -> Result<(), CodecError> {
    if !(1..=16).contains(&lsb_planes) {
        return Err(CodecError::InvalidPlanes(lsb_planes));
    }
    if lsb_planes > S::BITS {
        return Err(CodecError::PlanesExceedSampleBits {
            planes: lsb_planes,
            bits: S::BITS,
        });
    }
    Ok(())
}

/// Format a byte count as kilobytes with one decimal and thousands separators.
#[must_use]
pub fn format_kb(n: u64) -> String {
    // n / 1024 rounded to one decimal, using integer arithmetic to stay exact.
    let tenths = (u128::from(n) * 10 + 512) / 1024;
    format_tenths(tenths)
}

/// Like [`format_kb`] but rounded up (payload sizes in errors).
#[must_use]
pub fn format_kb_hi(n: u64) -> String {
    let tenths = (u128::from(n) * 10).div_ceil(1024);
    format_tenths(tenths)
}

/// Like [`format_kb`] but rounded down (capacities in errors).
#[must_use]
pub fn format_kb_lo(n: u64) -> String {
    let tenths = u128::from(n) * 10 / 1024;
    format_tenths(tenths)
}

fn format_tenths(tenths: u128) -> String {
    let whole = tenths / 10;
    let fraction = tenths % 10;
    format!("{}.{fraction} KB", with_thousands(whole))
}

fn with_thousands(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_matches_reference_values() {
        assert_eq!(capacity_bytes(9_437_184, 1), 1_179_630);
        assert_eq!(capacity_bytes(9_437_184, 4), 4_718_574);
        assert_eq!(capacity_bytes_total(9_437_184, 4, 1), 4_718_574);
        assert_eq!(capacity_bytes_total(9_437_184, 1, 2), 2_359_278);
        // Tiny images report zero rather than a negative capacity.
        assert_eq!(capacity_bytes(16, 1), 0);
    }

    #[test]
    fn formatting_matches_python_reference() {
        assert_eq!(format_kb(0), "0.0 KB");
        assert_eq!(format_kb(1), "0.0 KB");
        assert_eq!(format_kb(1023), "1.0 KB");
        assert_eq!(format_kb(1025), "1.0 KB");
        assert_eq!(format_kb(1_179_630), "1,152.0 KB");
        assert_eq!(format_kb(4_718_592), "4,608.0 KB");
        assert_eq!(format_kb(19_903_600), "19,437.1 KB");
        assert_eq!(format_kb_hi(19_903_600), "19,437.2 KB");
        assert_eq!(format_kb_lo(19_903_600), "19,437.1 KB");
        assert_eq!(format_kb_lo(1_179_630), "1,151.9 KB");
        assert_eq!(format_kb_hi(1), "0.1 KB");
    }

    #[test]
    fn u16_roundtrip_single_plane() {
        let mut cover = vec![0_u16; 256];
        let payload = b"RawHide";
        let frame = PayloadFrame::pack(payload, None);
        let codec = LsbCodec::new(1).unwrap();
        codec.embed(&mut cover, &frame).unwrap();
        let recovered = codec.extract(&cover, frame.len()).unwrap();
        assert_eq!(recovered, frame);
        assert_eq!(PayloadFrame::unpack(&recovered, None).unwrap().0, payload);
    }

    #[test]
    fn u8_roundtrip_multiple_planes() {
        let mut cover = vec![0_u8; 256];
        let frame = PayloadFrame::pack(b"cfa mode", None);
        let codec = LsbCodec::new(4).unwrap();
        codec.embed(&mut cover, &frame).unwrap();
        let recovered = codec.extract(&cover, frame.len()).unwrap();
        assert_eq!(recovered, frame);
    }

    #[test]
    fn u16_supports_sixteen_planes() {
        let mut cover = vec![0_u16; 64];
        let frame = PayloadFrame::pack(b"16-plane payload", None);
        let codec = LsbCodec::new(16).unwrap();
        codec.embed(&mut cover, &frame).unwrap();
        assert_eq!(codec.extract(&cover, frame.len()).unwrap(), frame);
    }

    #[test]
    fn mapping_is_plane_then_sample() {
        // Bit i goes to plane i % 2 of sample i / 2.
        let mut cover = vec![0_u16; 4];
        let bits = [1_u8, 0, 1, 1, 0, 1, 0, 0];
        embed_bits(&mut cover, &bits, 2).unwrap();
        assert_eq!(cover, [0b01, 0b11, 0b10, 0b00]);
        assert_eq!(extract_bits(&cover, bits.len(), 2).unwrap(), bits);
    }

    #[test]
    fn rejects_invalid_plane_counts() {
        assert_eq!(LsbCodec::new(0).unwrap_err(), CodecError::InvalidPlanes(0));
        assert_eq!(
            LsbCodec::new(17).unwrap_err(),
            CodecError::InvalidPlanes(17)
        );
        let mut cover = [0_u8; 8];
        assert_eq!(
            embed_bits(&mut cover, &[0], 9).unwrap_err(),
            CodecError::PlanesExceedSampleBits { planes: 9, bits: 8 }
        );
    }

    #[test]
    fn rejects_oversized_payload() {
        let mut cover = [0_u16; 2];
        let frame = vec![0_u8; 3];
        let err = embed_frame(&mut cover, &frame, 1).unwrap_err();
        assert_eq!(
            err,
            CodecError::PayloadTooLarge {
                need: 24,
                capacity: 2
            }
        );
    }

    #[test]
    fn rejects_non_binary_bits() {
        let mut cover = [0_u16; 8];
        assert_eq!(
            embed_bits(&mut cover, &[2], 1).unwrap_err(),
            CodecError::InvalidBit(2)
        );
    }

    #[test]
    fn empty_frame_roundtrips() {
        let mut cover = [0_u16; 8];
        let codec = LsbCodec::new(1).unwrap();
        codec.embed(&mut cover, &[]).unwrap();
        assert_eq!(codec.extract(&cover, 0).unwrap(), Vec::<u8>::new());
    }
}
