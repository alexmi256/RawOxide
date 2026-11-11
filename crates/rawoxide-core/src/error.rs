//! Error types shared by the core library.
//!
//! Module-specific errors stay separate so callers can react to the precise
//! failure (for example, `decode` tries each LSB-plane count and treats a bad
//! magic as "try the next one"). The top-level [`Error`](enum@Error) exists for APIs that
//! do not need that distinction.

use thiserror::Error;

/// Errors produced while packing or unpacking a payload frame.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    /// The stream is shorter than the fixed frame header.
    #[error("bitstream too short for header (need {need} bytes, got {len})")]
    HeaderTooShort {
        /// Number of bytes required.
        need: usize,
        /// Number of bytes available.
        len: usize,
    },

    /// The DNGS magic bytes were not found.
    #[error("magic not found - wrong key, wrong LSB depth or no payload")]
    BadMagic,

    /// The frame version is not supported by this build.
    #[error("unsupported frame version {0}")]
    UnsupportedVersion(u8),

    /// The declared payload length does not fit in the available stream.
    #[error("truncated payload: need {need} bytes, have {have}")]
    Truncated {
        /// Total frame length (header + payload) declared by the header.
        need: usize,
        /// Number of bytes actually available.
        have: usize,
    },

    /// The declared payload length cannot be represented on this platform.
    #[error("declared payload length {declared} does not fit in memory")]
    LengthOverflow {
        /// Declared payload length in bytes.
        declared: u64,
    },

    /// The payload body does not match its CRC32.
    #[error("CRC32 mismatch - corrupt payload or wrong key")]
    CrcMismatch,
}

/// Errors produced by the LSB codec.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    /// `lsb_planes` must be between 1 and 16.
    #[error("lsb_planes must be 1-16 (got {0})")]
    InvalidPlanes(u8),

    /// The requested planes do not fit in the sample type (for example, 16
    /// planes on `u8` samples).
    #[error("lsb_planes ({planes}) exceeds sample width ({bits} bits)")]
    PlanesExceedSampleBits {
        /// Requested number of planes.
        planes: u8,
        /// Sample width in bits.
        bits: u8,
    },

    /// The payload needs more bits than the cover can hold.
    #[error("payload too large: need {need} bits, capacity is {capacity} bits")]
    PayloadTooLarge {
        /// Required number of bits.
        need: u64,
        /// Available number of bits.
        capacity: u64,
    },

    /// The request needs more bits than the raw image can provide.
    #[error("request too large: need {need} bits, capacity is {capacity} bits")]
    RequestTooLarge {
        /// Required number of bits.
        need: u64,
        /// Available number of bits.
        capacity: u64,
    },

    /// A bit value other than 0 or 1 was supplied.
    #[error("bit values must be 0 or 1 (got {0})")]
    InvalidBit(u8),
}

/// Top-level error type for APIs that do not need module-specific errors.
#[derive(Debug, Error)]
pub enum Error {
    /// A payload frame could not be packed or unpacked.
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// The LSB codec rejected the request.
    #[error(transparent)]
    Codec(#[from] CodecError),
}

/// Convenience result type used by the core library.
pub type Result<T> = std::result::Result<T, Error>;
