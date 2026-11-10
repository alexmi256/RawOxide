//! Decode payloads from DNG files.
//!
//! Mirrors the Python decode policy:
//! - explicit `lsb_planes` is strict;
//! - otherwise try 1..=16 and accept the first frame that passes magic + CRC;
//! - `max_bytes` is checked before allocating the declared payload;
//! - one file returns its chunk (with a warning when split markers are present);
//! - several files are ordered/verified via split metadata or 4-digit names.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::codec::{format_kb_hi, format_kb_lo};
use crate::dng::{DngError, DngReader, RawFrame};
use crate::error::{CodecError, FrameError};
use crate::framing::PayloadFrame;
use crate::split::{
    SplitError, SplitIdField, SplitSeqField, order_by_markers, order_by_names, validate_fields,
};

/// Decode options for one or more chunk files.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeOptions<'a> {
    /// Optional XOR keystream key.
    pub key: Option<&'a [u8]>,
    /// Explicit plane count; `None` auto-detects 1..=16.
    pub lsb_planes: Option<u8>,
    /// Per-file declared-payload cap.
    pub max_bytes: Option<u64>,
    /// Field holding the shared UUID.
    pub split_id_field: SplitIdField,
    /// Field holding the chunk sequence.
    pub split_seq_field: SplitSeqField,
}

/// Result of decoding a file set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeOutcome {
    /// Recovered payload bytes, chunks concatenated in order.
    pub payload: Vec<u8>,
    /// Non-fatal notices for the CLI (partial chunk, missing totals, ...).
    pub warnings: Vec<String>,
    /// Chunk files in the order actually used.
    pub ordered_paths: Vec<PathBuf>,
}

/// Decode errors.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// No input files were given.
    #[error("no input files given")]
    NoInput,

    /// The DNG reader failed.
    #[error(transparent)]
    Dng(#[from] DngError),

    /// Split metadata/ordering failed.
    #[error(transparent)]
    Split(#[from] SplitError),

    /// Framing failed after extraction.
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// The LSB codec rejected the request.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// `lsb_planes` is outside 1..=16.
    #[error("lsb_planes must be 1-16 (got {0})")]
    InvalidPlanes(u8),

    /// The declared payload exceeds the per-file safety cap.
    #[error(
        "declared payload {declared} exceeds max_bytes limit {max} (pass --max-bytes larger than {max})",
        declared = format_kb_hi(*declared),
        max = format_kb_lo(*max)
    )]
    MaxBytesExceeded {
        /// Declared payload length.
        declared: u64,
        /// Configured cap.
        max: u64,
    },

    /// The declared payload does not fit in the image's LSB slots.
    #[error("declared payload exceeds image capacity")]
    CapacityExceeded,

    /// No plane count produced a valid frame.
    #[error("{detail}")]
    NoPayload {
        /// Diagnostic detail.
        detail: String,
    },

    /// The supplied files mix marker/no-marker states.
    #[error("inconsistent split metadata: {detail}")]
    InconsistentSplitMetadata {
        /// Human-readable detail.
        detail: String,
    },
}

/// Decode one file or a chunk set.
///
/// # Errors
///
/// Returns [`DecodeError`] for unreadable files, inconsistent split metadata,
/// an exceeded `max_bytes`/capacity limit, or no valid frame.
pub fn decode_files(
    paths: &[PathBuf],
    options: &DecodeOptions<'_>,
) -> Result<DecodeOutcome, DecodeError> {
    if paths.is_empty() {
        return Err(DecodeError::NoInput);
    }
    validate_fields(options.split_id_field, options.split_seq_field)?;

    let mut markers = Vec::with_capacity(paths.len());
    for path in paths {
        let reader = DngReader::open(path)?;
        markers.push(reader.split_markers(options.split_id_field, options.split_seq_field)?);
    }

    if paths.len() == 1 {
        let payload = decode_one(&paths[0], options)?;
        let mut warnings = Vec::new();
        let marker = &markers[0];
        if marker.seq_present || marker.id_present {
            warnings.push(format!(
                "{} carries split metadata (chunk {} of {}): decoding one chunk returns partial data; pass every chunk file to restore the whole payload.",
                paths[0].display(),
                marker
                    .seq
                    .map_or_else(|| "?".to_owned(), |seq| format!("{seq:04}")),
                marker
                    .total
                    .map_or_else(|| "?".to_owned(), |total| format!("{total:04}"))
            ));
        }
        return Ok(DecodeOutcome {
            payload,
            warnings,
            ordered_paths: paths.to_vec(),
        });
    }

    let id_active = options.split_id_field.is_active();
    let seq_active = options.split_seq_field.is_active();
    let all_have = markers
        .iter()
        .all(|marker| (!id_active || marker.id_present) && (!seq_active || marker.seq_present));
    let any_have = markers
        .iter()
        .any(|marker| (id_active && marker.id_present) || (seq_active && marker.seq_present));

    let ordered = if all_have {
        order_by_markers(paths, &markers, id_active, seq_active)?
    } else if !any_have {
        order_by_names(paths)?
    } else {
        return Err(DecodeError::InconsistentSplitMetadata {
            detail: "some files carry chunk markers and some do not; pass a consistent set"
                .to_owned(),
        });
    };

    let mut payload = Vec::new();
    for path in &ordered {
        let chunk = decode_one(path, options).map_err(|error| DecodeError::NoPayload {
            detail: format!("{}: {error}", path.display()),
        })?;
        payload.extend_from_slice(&chunk);
    }

    let mut warnings = Vec::new();
    if seq_active && ordered.len() > 1 && markers.iter().all(|marker| marker.total.is_none()) {
        warnings.push(format!(
            "no chunk totals found in metadata ({} carries sequence numbers only); assuming the given files are the complete set.",
            options.split_seq_field
        ));
    }

    Ok(DecodeOutcome {
        payload,
        warnings,
        ordered_paths: ordered,
    })
}

fn decode_one(path: &Path, options: &DecodeOptions<'_>) -> Result<Vec<u8>, DecodeError> {
    if let Some(planes) = options.lsb_planes
        && !(1..=16).contains(&planes)
    {
        return Err(DecodeError::InvalidPlanes(planes));
    }

    let reader = DngReader::open(path)?;
    let mut frames = reader.raw_frames()?;
    let candidates: Vec<u8> = options
        .lsb_planes
        .map_or_else(|| (1..=16).collect(), |planes| vec![planes]);

    let mut last_error = None;
    let mut limit_error = None;
    for planes in candidates {
        match decode_with_planes(&mut frames, options.key, planes, options.max_bytes) {
            Ok(payload) => return Ok(payload),
            Err(AttemptError::Limit(error)) => {
                limit_error.get_or_insert(error);
            }
            Err(AttemptError::Other(error)) => {
                last_error = Some(error.to_string());
            }
        }
    }

    if let Some(error) = limit_error {
        return Err(error);
    }
    let detail = if options.lsb_planes.is_some() {
        "magic not found - wrong key, wrong LSB depth/mode or no payload".to_owned()
    } else {
        format!(
            "no decodable payload at 1-16 LSB planes - wrong key or no payload (last error: {})",
            last_error.unwrap_or_else(|| "no valid frame".to_owned())
        )
    };
    Err(DecodeError::NoPayload { detail })
}

#[derive(Debug)]
enum AttemptError {
    Limit(DecodeError),
    Other(DecodeError),
}

fn decode_with_planes(
    frames: &mut Vec<RawFrame>,
    key: Option<&[u8]>,
    planes: u8,
    max_bytes: Option<u64>,
) -> Result<Vec<u8>, AttemptError> {
    let header_bytes =
        extract_bytes(frames.as_slice(), planes, PayloadFrame::HEADER_LEN).map_err(|error| {
            match error {
                DecodeError::CapacityExceeded => AttemptError::Limit(error),
                other => AttemptError::Other(other),
            }
        })?;

    let header = match PayloadFrame::parse_header(&header_bytes, key) {
        Ok(header) => header,
        Err(FrameError::BadMagic) => {
            return Err(AttemptError::Other(DecodeError::NoPayload {
                detail: "no magic".to_owned(),
            }));
        }
        Err(error) => {
            return Err(AttemptError::Other(DecodeError::Frame(error)));
        }
    };

    if let Some(max) = max_bytes
        && header.payload_len > max
    {
        return Err(AttemptError::Limit(DecodeError::MaxBytesExceeded {
            declared: header.payload_len,
            max,
        }));
    }

    let total_bytes = usize::try_from(header.payload_len)
        .ok()
        .and_then(|payload| payload.checked_add(PayloadFrame::HEADER_LEN))
        .ok_or_else(|| {
            AttemptError::Limit(DecodeError::NoPayload {
                detail: "declared payload length does not fit in memory".to_owned(),
            })
        })?;

    let total_bits = total_bytes
        .checked_mul(8)
        .ok_or(AttemptError::Limit(DecodeError::CapacityExceeded))?;
    let total_slots = frames.iter().fold(0_u64, |total, frame| {
        total.saturating_add(
            u64::try_from(frame.sample_count())
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(planes)),
        )
    });
    if u64::try_from(total_bits).unwrap_or(u64::MAX) > total_slots {
        return Err(AttemptError::Limit(DecodeError::CapacityExceeded));
    }

    let stream =
        extract_bytes(frames.as_slice(), planes, total_bytes).map_err(|error| match error {
            DecodeError::CapacityExceeded => AttemptError::Limit(error),
            other => AttemptError::Other(other),
        })?;
    let (payload, _) = PayloadFrame::unpack_owned(stream, key)
        .map_err(|error| AttemptError::Other(DecodeError::Frame(error)))?;
    // Extraction already peaked with both allocations live; releasing the raw
    // samples here keeps the peak unchanged while preserving plane fallback.
    frames.clear();
    Ok(payload)
}

fn extract_bytes(frames: &[RawFrame], planes: u8, n_bytes: usize) -> Result<Vec<u8>, DecodeError> {
    if !(1..=16).contains(&planes) {
        return Err(DecodeError::InvalidPlanes(planes));
    }
    let n_bits = n_bytes
        .checked_mul(8)
        .ok_or(DecodeError::CapacityExceeded)?;
    let planes_usize = usize::from(planes);
    let total_slots = frames.iter().fold(0_usize, |total, frame| {
        total.saturating_add(frame.sample_count().saturating_mul(planes_usize))
    });
    if n_bits > total_slots {
        return Err(DecodeError::CapacityExceeded);
    }

    let mut out = vec![0_u8; n_bytes];
    let mut global_bit = 0_usize;
    for frame in frames {
        if global_bit >= n_bits {
            break;
        }
        let frame_slots = frame.sample_count().saturating_mul(planes_usize);
        let take = (n_bits - global_bit).min(frame_slots);
        for local in 0..take {
            let sample = frame.samples[local / planes_usize];
            let plane = u8::try_from(local % planes_usize).expect("plane index fits in u8");
            let bit = (sample >> plane) & 1;
            let index = global_bit + local;
            out[index / 8] |= u8::try_from(bit).expect("LSB is 0 or 1") << (7 - (index % 8));
        }
        global_bit += take;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(samples: Vec<u16>) -> RawFrame {
        RawFrame {
            width: samples.len(),
            height: 1,
            samples_per_pixel: 1,
            bits_per_sample: 16,
            samples,
        }
    }

    #[test]
    fn extract_matches_codec_mapping() {
        let frame = frame(vec![0b0000, 0b0000, 0b0000, 0b0000]);
        let bytes = extract_bytes(&[frame], 2, 1).unwrap();
        assert_eq!(bytes, vec![0]);
    }

    #[test]
    fn extract_spans_frames() {
        let first = frame(vec![1, 0, 0, 0]);
        let second = frame(vec![0, 1, 0, 1]);
        // 8 bits total: 1000 0101 -> 0x85.
        let bytes = extract_bytes(&[first, second], 1, 1).unwrap();
        assert_eq!(bytes, vec![0b1000_0101]);
    }

    #[test]
    fn rejects_insufficient_capacity() {
        let frame = frame(vec![0]);
        assert!(matches!(
            extract_bytes(&[frame], 1, 1),
            Err(DecodeError::CapacityExceeded)
        ));
    }

    #[test]
    fn extract_with_two_planes_uses_plane_then_sample_order() {
        let frame = frame(vec![0b01, 0b10, 0b00, 0b11]);
        let bytes = extract_bytes(&[frame], 2, 1).unwrap();
        assert_eq!(bytes, vec![0b1001_0011]);
    }

    #[test]
    fn decode_with_planes_accepts_a_synthetic_frame() {
        let payload = b"synthetic payload";
        let frame_bytes = PayloadFrame::pack(payload, None);
        let mut samples = vec![0_u16; frame_bytes.len() * 8];
        crate::codec::embed_frame(&mut samples, &frame_bytes, 1).unwrap();
        let raw = RawFrame {
            width: samples.len(),
            height: 1,
            samples_per_pixel: 1,
            bits_per_sample: 16,
            samples,
        };
        let decoded = decode_with_planes(&mut vec![raw], None, 1, None).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn decode_with_planes_reports_no_magic_and_limits() {
        let raw = RawFrame {
            width: 200,
            height: 1,
            samples_per_pixel: 1,
            bits_per_sample: 16,
            samples: vec![0; 200],
        };
        assert!(matches!(
            decode_with_planes(&mut vec![raw], None, 1, None),
            Err(AttemptError::Other(DecodeError::NoPayload { .. }))
        ));

        let payload = b"x";
        let frame_bytes = PayloadFrame::pack(payload, None);
        let mut samples = vec![0_u16; frame_bytes.len() * 8];
        crate::codec::embed_frame(&mut samples, &frame_bytes, 1).unwrap();
        let raw = RawFrame {
            width: samples.len(),
            height: 1,
            samples_per_pixel: 1,
            bits_per_sample: 16,
            samples,
        };
        assert!(matches!(
            decode_with_planes(&mut vec![raw], None, 1, Some(0)),
            Err(AttemptError::Limit(DecodeError::MaxBytesExceeded { .. }))
        ));
    }
}
