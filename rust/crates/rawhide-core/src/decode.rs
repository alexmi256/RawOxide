//! Decode payloads from DNG files.
//!
//! Mirrors the Python decode policy:
//! - explicit `lsb_planes` is strict;
//! - otherwise try 1..=16 and accept the first frame that passes magic + CRC;
//! - `max_bytes` is checked before allocating the declared payload;
//! - one file returns its chunk (with a warning when split markers are present);
//! - several files are ordered/verified via split metadata or 4-digit names.

use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::codec::{format_kb_hi, format_kb_lo};
#[cfg(test)]
use crate::dng::RawFrame;
use crate::dng::{DngError, DngReader};
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

/// Streaming decode result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeStreamOutcome {
    /// Total payload bytes written.
    pub payload_len: u64,
    /// Non-fatal notices for the CLI.
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

    /// Writing the recovered payload failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

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
    let plan = plan_files(paths, options)?;
    if plan.ordered.len() == 1 {
        let payload = decode_one(&plan.ordered[0], options)?;
        return Ok(DecodeOutcome {
            payload,
            warnings: plan.warnings,
            ordered_paths: plan.ordered,
        });
    }

    let mut payload = Vec::new();
    for path in &plan.ordered {
        let mut chunk =
            decode_one(path, options).map_err(|error| per_file_error(path, error, true))?;
        payload.append(&mut chunk);
    }
    Ok(DecodeOutcome {
        payload,
        warnings: plan.warnings,
        ordered_paths: plan.ordered,
    })
}

/// Decode one file or a chunk set directly into a writer.
///
/// Unlike [`decode_files`], this never materializes the payload in memory: each
/// chunk is streamed into `writer` as it is extracted. On error, bytes already
/// written remain the caller's responsibility (the CLI writes to a temp file).
///
/// # Errors
///
/// Returns [`DecodeError`] under the same conditions as [`decode_files`], plus
/// I/O errors from `writer`.
pub fn decode_files_to_writer<W: Write>(
    paths: &[PathBuf],
    options: &DecodeOptions<'_>,
    writer: &mut W,
) -> Result<DecodeStreamOutcome, DecodeError> {
    decode_files_to_writer_impl(paths, options, writer, None)
}

/// Decode one file or a chunk set directly into a writer, reporting progress.
///
/// `progress(file_index, file_count, bytes_written, file_payload_len)` is
/// called with zero before each file and after each payload flush. File indices
/// are zero-based.
///
/// # Errors
///
/// See [`decode_files_to_writer`].
pub fn decode_files_to_writer_with_progress<W, F>(
    paths: &[PathBuf],
    options: &DecodeOptions<'_>,
    writer: &mut W,
    mut progress: F,
) -> Result<DecodeStreamOutcome, DecodeError>
where
    W: Write,
    F: FnMut(usize, usize, u64, u64),
{
    decode_files_to_writer_impl(
        paths,
        options,
        writer,
        Some(&mut progress as &mut dyn FnMut(usize, usize, u64, u64)),
    )
}

fn decode_files_to_writer_impl<W: Write>(
    paths: &[PathBuf],
    options: &DecodeOptions<'_>,
    writer: &mut W,
    mut progress: Option<&mut dyn FnMut(usize, usize, u64, u64)>,
) -> Result<DecodeStreamOutcome, DecodeError> {
    let plan = plan_files(paths, options)?;
    let multiple = plan.ordered.len() > 1;
    let file_count = plan.ordered.len();
    let mut payload_len = 0_u64;
    for (index, path) in plan.ordered.iter().enumerate() {
        let progress_ref = progress
            .as_deref_mut()
            .map(|callback| callback as &mut dyn FnMut(usize, usize, u64, u64));
        let len = decode_one_into(path, options, writer, index, file_count, progress_ref)
            .map_err(|error| per_file_error(path, error, multiple))?;
        payload_len = payload_len
            .checked_add(len)
            .ok_or(DecodeError::CapacityExceeded)?;
    }
    Ok(DecodeStreamOutcome {
        payload_len,
        warnings: plan.warnings,
        ordered_paths: plan.ordered,
    })
}

struct DecodePlan {
    ordered: Vec<PathBuf>,
    warnings: Vec<String>,
}

fn plan_files(paths: &[PathBuf], options: &DecodeOptions<'_>) -> Result<DecodePlan, DecodeError> {
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
        return Ok(DecodePlan {
            ordered: paths.to_vec(),
            warnings,
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

    let mut warnings = Vec::new();
    if seq_active && ordered.len() > 1 && markers.iter().all(|marker| marker.total.is_none()) {
        warnings.push(format!(
            "no chunk totals found in metadata ({} carries sequence numbers only); assuming the given files are the complete set.",
            options.split_seq_field
        ));
    }

    Ok(DecodePlan { ordered, warnings })
}

fn per_file_error(path: &Path, error: DecodeError, multiple: bool) -> DecodeError {
    if multiple {
        DecodeError::NoPayload {
            detail: format!("{}: {error}", path.display()),
        }
    } else {
        error
    }
}

fn decode_one(path: &Path, options: &DecodeOptions<'_>) -> Result<Vec<u8>, DecodeError> {
    let mut payload = Vec::new();
    let mut target = VecTarget(&mut payload);
    decode_one_target(path, options, &mut target, 0, 1, None)?;
    Ok(payload)
}

fn decode_one_into<W: Write>(
    path: &Path,
    options: &DecodeOptions<'_>,
    writer: &mut W,
    file_index: usize,
    file_count: usize,
    progress: Option<&mut dyn FnMut(usize, usize, u64, u64)>,
) -> Result<u64, DecodeError> {
    let mut target = WriterTarget(writer);
    decode_one_target(path, options, &mut target, file_index, file_count, progress)
}

trait PayloadTarget {
    fn reserve(&mut self, len: usize);
    fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}

struct VecTarget<'a>(&'a mut Vec<u8>);

impl PayloadTarget for VecTarget<'_> {
    fn reserve(&mut self, len: usize) {
        self.0.reserve(len);
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

struct WriterTarget<'a, W: Write>(&'a mut W);

impl<W: Write> PayloadTarget for WriterTarget<'_, W> {
    fn reserve(&mut self, _len: usize) {}

    fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.0.write_all(bytes)
    }
}

const SINK_BUFFER: usize = 64 * 1024;

struct PayloadSink<'t, 'k, 'p, T: PayloadTarget> {
    target: &'t mut T,
    key: Option<&'k [u8]>,
    hasher: crc32fast::Hasher,
    buffer: Vec<u8>,
    written: u64,
    bit: u8,
    bits: u8,
    file_index: usize,
    file_count: usize,
    expected_len: u64,
    progress: Option<&'p mut dyn FnMut(usize, usize, u64, u64)>,
}

impl<'t, 'k, 'p, T: PayloadTarget> PayloadSink<'t, 'k, 'p, T> {
    fn new(
        target: &'t mut T,
        key: Option<&'k [u8]>,
        payload_len: usize,
        file_index: usize,
        file_count: usize,
        mut progress: Option<&'p mut dyn FnMut(usize, usize, u64, u64)>,
    ) -> Self {
        target.reserve(payload_len);
        let expected_len = u64::try_from(payload_len).unwrap_or(u64::MAX);
        if let Some(callback) = progress.as_deref_mut() {
            callback(file_index, file_count, 0, expected_len);
        }
        Self {
            target,
            key: key.filter(|key| !key.is_empty()),
            hasher: crc32fast::Hasher::new(),
            buffer: Vec::with_capacity(SINK_BUFFER),
            written: 0,
            bit: 0,
            bits: 0,
            file_index,
            file_count,
            expected_len,
            progress,
        }
    }

    fn push_bit(&mut self, bit: u8) -> Result<(), DecodeError> {
        self.bit = (self.bit << 1) | (bit & 1);
        self.bits += 1;
        if self.bits == 8 {
            self.buffer.push(self.bit);
            self.bit = 0;
            self.bits = 0;
            if self.buffer.len() >= SINK_BUFFER {
                self.flush()?;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DecodeError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        if let Some(key) = self.key {
            let mask = PayloadFrame::keystream_range(key, self.written, self.buffer.len());
            for (byte, mask) in self.buffer.iter_mut().zip(mask) {
                *byte ^= mask;
            }
        }
        self.hasher.update(&self.buffer);
        self.target.write_bytes(&self.buffer)?;
        self.written += u64::try_from(self.buffer.len()).unwrap_or(u64::MAX);
        self.buffer.clear();
        if let Some(callback) = self.progress.as_deref_mut() {
            callback(
                self.file_index,
                self.file_count,
                self.written,
                self.expected_len,
            );
        }
        Ok(())
    }

    fn finish(mut self, expected_crc: u32, expected_len: u64) -> Result<(), DecodeError> {
        if self.bits != 0 {
            return Err(DecodeError::NoPayload {
                detail: "payload is not byte-aligned".to_owned(),
            });
        }
        self.flush()?;
        if self.written != expected_len {
            return Err(DecodeError::NoPayload {
                detail: format!(
                    "decoded payload is {} bytes, expected {expected_len}",
                    self.written
                ),
            });
        }
        if self.hasher.finalize() != expected_crc {
            return Err(DecodeError::Frame(FrameError::CrcMismatch));
        }
        Ok(())
    }
}

fn decode_one_target<T: PayloadTarget>(
    path: &Path,
    options: &DecodeOptions<'_>,
    target: &mut T,
    file_index: usize,
    file_count: usize,
    progress: Option<&mut dyn FnMut(usize, usize, u64, u64)>,
) -> Result<u64, DecodeError> {
    if let Some(planes) = options.lsb_planes
        && !(1..=16).contains(&planes)
    {
        return Err(DecodeError::InvalidPlanes(planes));
    }

    let reader = DngReader::open(path)?;
    let infos = reader.raw_frame_infos()?;
    if infos.is_empty() {
        return Err(DecodeError::Dng(DngError::NoRawSubIfd));
    }
    let candidates: Vec<u8> = options
        .lsb_planes
        .map_or_else(|| (1..=16).collect(), |planes| vec![planes]);

    // The header is at most 144 bits and may span frames in small multi-frame
    // files. Accumulate whole bands across frames, keeping the current reader
    // positioned for the payload fill so no band is decoded twice.
    let header_samples = match options.lsb_planes {
        Some(planes) => {
            usize::try_from(HEADER_BITS.div_ceil(u64::from(planes))).unwrap_or(usize::MAX)
        }
        // Auto-detection must still be able to test the 1-plane candidate.
        None => usize::try_from(HEADER_BITS).unwrap_or(usize::MAX),
    };
    let mut prefix = Vec::new();
    let mut current_frame = 0_usize;
    let mut current_reader = if infos.is_empty() {
        None
    } else {
        Some(reader.band_reader(0)?)
    };
    while prefix.len() < header_samples {
        let next = match current_reader.as_mut() {
            Some(bands) => bands.next_band()?,
            None => break,
        };
        if let Some(band) = next {
            prefix.extend_from_slice(&band.samples);
        } else {
            current_frame += 1;
            current_reader = if current_frame < infos.len() {
                Some(reader.band_reader(current_frame)?)
            } else {
                None
            };
        }
    }

    let mut last_error = None;
    let mut chosen = None;
    for planes in candidates {
        let Some(header_bytes) = extract_prefix_bytes(&prefix, planes, PayloadFrame::HEADER_LEN)
        else {
            continue;
        };
        match PayloadFrame::parse_header(&header_bytes, options.key) {
            Ok(header) => {
                chosen = Some((planes, header));
                break;
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    let Some((planes, header)) = chosen else {
        return Err(no_payload_error(options.lsb_planes, last_error));
    };

    if let Some(max) = options.max_bytes
        && header.payload_len > max
    {
        return Err(DecodeError::MaxBytesExceeded {
            declared: header.payload_len,
            max,
        });
    }

    let payload_len = usize::try_from(header.payload_len).map_err(|_| DecodeError::NoPayload {
        detail: "declared payload length does not fit in memory".to_owned(),
    })?;
    let total_bytes = payload_len
        .checked_add(PayloadFrame::HEADER_LEN)
        .ok_or(DecodeError::CapacityExceeded)?;
    let total_bits = u64::try_from(total_bytes)
        .ok()
        .and_then(|bytes| bytes.checked_mul(8))
        .ok_or(DecodeError::CapacityExceeded)?;
    let total_slots = infos.iter().fold(0_u64, |total, info| {
        total.saturating_add(
            u64::try_from(info.sample_count())
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(planes)),
        )
    });
    if total_bits > total_slots {
        return Err(DecodeError::CapacityExceeded);
    }

    let mut sink = PayloadSink::new(
        target,
        options.key,
        payload_len,
        file_index,
        file_count,
        progress,
    );
    let mut framed_bit = 0_u64;

    fill_payload_sink(&mut sink, &prefix, planes, &mut framed_bit, total_bits)?;
    if framed_bit < total_bits
        && let Some(mut bands) = current_reader
    {
        while let Some(band) = bands.next_band()? {
            fill_payload_sink(
                &mut sink,
                &band.samples,
                planes,
                &mut framed_bit,
                total_bits,
            )?;
            if framed_bit >= total_bits {
                break;
            }
        }
    }

    for frame_index in current_frame + 1..infos.len() {
        if framed_bit >= total_bits {
            break;
        }
        let mut bands = reader.band_reader(frame_index)?;
        while framed_bit < total_bits {
            let Some(band) = bands.next_band()? else {
                break;
            };
            fill_payload_sink(
                &mut sink,
                &band.samples,
                planes,
                &mut framed_bit,
                total_bits,
            )?;
        }
    }

    if framed_bit < total_bits {
        return Err(DecodeError::CapacityExceeded);
    }

    sink.finish(header.crc32, header.payload_len)?;
    Ok(header.payload_len)
}

const HEADER_BITS: u64 = PayloadFrame::HEADER_LEN as u64 * 8;

fn extract_prefix_bytes(samples: &[u16], planes: u8, n_bytes: usize) -> Option<Vec<u8>> {
    let planes = usize::from(planes);
    let n_bits = n_bytes.checked_mul(8)?;
    if samples.len().checked_mul(planes)? < n_bits {
        return None;
    }
    let mut out = vec![0_u8; n_bytes];
    for bit_index in 0..n_bits {
        let sample = samples[bit_index / planes];
        let plane = bit_index % planes;
        let bit = (sample >> plane) & 1;
        out[bit_index / 8] |= u8::try_from(bit).unwrap_or(0) << (7 - (bit_index % 8));
    }
    Some(out)
}

fn fill_payload_sink<T: PayloadTarget>(
    sink: &mut PayloadSink<'_, '_, '_, T>,
    samples: &[u16],
    planes: u8,
    framed_bit: &mut u64,
    total_bits: u64,
) -> Result<(), DecodeError> {
    let planes = usize::from(planes);
    for &sample in samples {
        for plane in 0..planes {
            if *framed_bit >= total_bits {
                return Ok(());
            }
            if *framed_bit >= HEADER_BITS {
                sink.push_bit(u8::try_from((sample >> plane) & 1).unwrap_or(0))?;
            }
            *framed_bit += 1;
        }
    }
    Ok(())
}

fn no_payload_error(explicit: Option<u8>, last_error: Option<String>) -> DecodeError {
    let detail = last_error.unwrap_or_else(|| "no valid frame".to_owned());
    if let Some(planes) = explicit {
        DecodeError::NoPayload {
            detail: format!(
                "magic not found at {planes} LSB plane(s) - wrong key, wrong LSB depth/mode or no payload ({detail})"
            ),
        }
    } else {
        DecodeError::NoPayload {
            detail: format!(
                "no decodable payload at 1-16 LSB planes - wrong key or no payload (last error: {detail})"
            ),
        }
    }
}

#[cfg(test)]
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
    fn streaming_fill_matches_framed_payload() {
        let body = b"streaming body bits across planes";
        for key in [None, Some(&b"stream-key"[..])] {
            let framed = PayloadFrame::pack(body, key);
            for planes in [1_u8, 2, 3, 7, 8, 16] {
                let mut samples = vec![0_u16; framed.len() * 8 / usize::from(planes) + 1];
                crate::codec::embed_frame(&mut samples, &framed, planes).unwrap();
                let mut payload = Vec::new();
                {
                    let mut target = VecTarget(&mut payload);
                    let mut sink = PayloadSink::new(&mut target, key, body.len(), 0, 1, None);
                    let mut framed_bit = 0_u64;
                    fill_payload_sink(
                        &mut sink,
                        &samples,
                        planes,
                        &mut framed_bit,
                        u64::try_from(framed.len() * 8).unwrap(),
                    )
                    .unwrap();
                    sink.finish(crc32fast::hash(body), u64::try_from(body.len()).unwrap())
                        .unwrap();
                }
                assert_eq!(payload, body, "keyed={key:?} planes={planes}");
            }
        }
    }
}
