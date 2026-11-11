//! Streaming payload sources for the writer.
//!
//! The framed payload is read in small chunks while bands are embedded, so a
//! large input file never has to be materialized beside the DNG.

use std::fs::File;
use std::io::{ErrorKind, Read as _, Seek as _, SeekFrom};

use crate::framing::PayloadFrame;

use super::WriteError;

/// Chunk size for framed payload reads.
const CHUNK: usize = 64 * 1024;

/// Random-access payload bytes used by the writer.
pub(super) trait PayloadSource {
    /// Payload length in bytes.
    fn len(&self) -> u64;

    /// CRC32 of the plaintext payload.
    fn crc32(&mut self) -> Result<u32, WriteError>;

    /// Read exactly `out.len()` payload bytes starting at `offset`.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), WriteError>;

    /// Whether repeated reads are guaranteed to observe the same bytes.
    fn is_stable(&self) -> bool {
        true
    }
}

/// In-memory payload slice.
pub(super) struct SliceSource<'a> {
    pub(super) data: &'a [u8],
}

impl PayloadSource for SliceSource<'_> {
    fn len(&self) -> u64 {
        u64::try_from(self.data.len()).unwrap_or(u64::MAX)
    }

    fn crc32(&mut self) -> Result<u32, WriteError> {
        Ok(crc32fast::hash(self.data))
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), WriteError> {
        let start = usize::try_from(offset).map_err(|_| WriteError::TooLarge)?;
        let end = start.checked_add(out.len()).ok_or(WriteError::TooLarge)?;
        let bytes = self
            .data
            .get(start..end)
            .ok_or_else(|| short_payload(offset, out.len()))?;
        out.copy_from_slice(bytes);
        Ok(())
    }
}

/// A byte range in a payload file.
pub(super) struct FileSource {
    file: File,
    start: u64,
    len: u64,
}

impl FileSource {
    pub(super) fn open(
        path: impl AsRef<std::path::Path>,
        start: u64,
        len: u64,
    ) -> Result<Self, WriteError> {
        let file = File::open(path)?;
        let available = file.metadata()?.len();
        let end = start.checked_add(len).ok_or(WriteError::TooLarge)?;
        if end > available {
            return Err(WriteError::Io(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                format!("payload range {start}..{end} exceeds file size {available}"),
            )));
        }
        Ok(Self { file, start, len })
    }
}

impl PayloadSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn crc32(&mut self) -> Result<u32, WriteError> {
        let mut hasher = crc32fast::Hasher::new();
        let mut buffer = vec![0_u8; CHUNK];
        let mut offset = 0_u64;
        while offset < self.len {
            let rows = (self.len - offset).min(CHUNK as u64);
            let take = usize::try_from(rows).unwrap_or(CHUNK);
            self.read_at(offset, &mut buffer[..take])?;
            hasher.update(&buffer[..take]);
            offset += rows;
        }
        Ok(hasher.finalize())
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), WriteError> {
        let position = self.start.checked_add(offset).ok_or(WriteError::TooLarge)?;
        self.file.seek(SeekFrom::Start(position))?;
        self.file.read_exact(out)?;
        Ok(())
    }

    fn is_stable(&self) -> bool {
        false
    }
}

/// Sequential bit reader over the framed header and payload.
pub(super) struct PayloadBitReader<'a> {
    source: &'a mut dyn PayloadSource,
    header: [u8; PayloadFrame::HEADER_LEN],
    key: Option<&'a [u8]>,
    payload_len: u64,
    total_bits: u64,
    buffer: Vec<u8>,
    buffer_start: u64,
    next_index: u64,
    expected_crc: u32,
    hasher: Option<crc32fast::Hasher>,
    frame_hasher: Option<crc32fast::Hasher>,
}

impl<'a> PayloadBitReader<'a> {
    pub(super) fn new(
        source: &'a mut dyn PayloadSource,
        key: Option<&'a [u8]>,
        collect_frame_crc: bool,
    ) -> Result<Self, WriteError> {
        let payload_len = source.len();
        let verify = !source.is_stable();
        let crc = source.crc32()?;
        let header = PayloadFrame::masked_header(payload_len, crc, key);
        let total_bytes = payload_len
            .checked_add(PayloadFrame::HEADER_LEN as u64)
            .ok_or(WriteError::TooLarge)?;
        let total_bits = total_bytes.checked_mul(8).ok_or(WriteError::TooLarge)?;
        Ok(Self {
            source,
            header,
            key: key.filter(|key| !key.is_empty()),
            payload_len,
            total_bits,
            buffer: Vec::new(),
            buffer_start: 0,
            next_index: 0,
            expected_crc: crc,
            hasher: verify.then(crc32fast::Hasher::new),
            frame_hasher: collect_frame_crc.then(crc32fast::Hasher::new),
        })
    }

    /// Position the sequential cursor at `index`.
    pub(super) fn seek(&mut self, index: u64) {
        debug_assert!(index >= self.next_index, "payload cursor is forward-only");
        self.next_index = index;
    }

    /// Next bit, or `None` past the end of the framed payload.
    #[inline]
    pub(super) fn next_bit(&mut self) -> Result<Option<u8>, WriteError> {
        let index = self.next_index;
        if index >= self.total_bits {
            return Ok(None);
        }
        self.next_index = index + 1;
        let byte_index = index >> 3;
        let offset = byte_index.saturating_sub(self.buffer_start);
        if offset >= u64::try_from(self.buffer.len()).unwrap_or(u64::MAX) {
            self.load(byte_index)?;
        }
        let byte = self.buffer[usize::try_from(byte_index - self.buffer_start).unwrap_or(0)];
        Ok(Some((byte >> (7 - (index & 7))) & 1))
    }

    /// CRC32 of the framed bytes read so far (header + masked body), when
    /// collection was requested.
    pub(super) fn frame_crc32(&self) -> Option<u32> {
        self.frame_hasher
            .as_ref()
            .map(|hasher| hasher.clone().finalize())
    }

    /// Verify that a changing file still matches the CRC embedded in the header.
    pub(super) fn finish(self) -> Result<(), WriteError> {
        if let Some(hasher) = self.hasher
            && hasher.finalize() != self.expected_crc
        {
            return Err(WriteError::PayloadChanged);
        }
        Ok(())
    }

    fn load(&mut self, byte_index: u64) -> Result<(), WriteError> {
        let total_bytes = self
            .payload_len
            .checked_add(PayloadFrame::HEADER_LEN as u64)
            .ok_or(WriteError::TooLarge)?;
        let start = byte_index - byte_index % CHUNK as u64;
        let end = start.saturating_add(CHUNK as u64).min(total_bytes);
        let len = usize::try_from(end - start).map_err(|_| WriteError::TooLarge)?;
        self.buffer.resize(len, 0);

        let header_len = PayloadFrame::HEADER_LEN as u64;
        let filled = if start < header_len {
            let header_end = header_len.min(end);
            let count = usize::try_from(header_end - start).unwrap_or(0);
            self.buffer[..count].copy_from_slice(&self.header[start as usize..header_end as usize]);
            count
        } else {
            0
        };
        if filled < len {
            let body_offset = start
                .saturating_add(filled as u64)
                .saturating_sub(header_len);
            self.source
                .read_at(body_offset, &mut self.buffer[filled..])?;
            if let Some(hasher) = &mut self.hasher {
                hasher.update(&self.buffer[filled..]);
            }
        }
        if let Some(key) = self.key {
            let body_start = start.max(header_len);
            if end > body_start {
                let mask_offset = body_start - header_len;
                let count = usize::try_from(end - body_start).unwrap_or(0);
                let mask = PayloadFrame::keystream_range(key, mask_offset, count);
                let target = usize::try_from(body_start - start).unwrap_or(0);
                for (byte, mask) in self.buffer[target..target + count].iter_mut().zip(mask) {
                    *byte ^= mask;
                }
            }
        }
        if let Some(hasher) = &mut self.frame_hasher {
            hasher.update(&self.buffer[..len]);
        }
        self.buffer_start = start;
        Ok(())
    }
}

fn short_payload(offset: u64, len: usize) -> WriteError {
    WriteError::Io(std::io::Error::new(
        ErrorKind::UnexpectedEof,
        format!("payload does not contain {len} bytes at offset {offset}"),
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn streaming_bits_match_the_packed_frame() {
        let payload: Vec<u8> = (0..70_000).map(|index| (index % 251) as u8).collect();
        for key in [None, Some(&b"stream-key"[..])] {
            let packed = PayloadFrame::pack(&payload, key);
            let mut source = SliceSource { data: &payload };
            let mut reader = PayloadBitReader::new(&mut source, key, false).unwrap();
            assert_eq!(reader.total_bits, u64::try_from(packed.len() * 8).unwrap());
            for index in 0..packed.len() * 8 {
                let byte = packed[index / 8];
                let expected = (byte >> (7 - (index % 8))) & 1;
                assert_eq!(reader.next_bit().unwrap(), Some(expected));
            }
            assert_eq!(reader.next_bit().unwrap(), None);
        }
    }

    #[test]
    fn file_source_reads_a_range_and_matches_the_slice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"skip").unwrap();
        file.write_all(b"streamed payload bytes").unwrap();
        drop(file);

        let mut source = FileSource::open(&path, 4, 22).unwrap();
        let mut out = [0_u8; 22];
        source.read_at(0, &mut out).unwrap();
        assert_eq!(&out, b"streamed payload bytes");
        assert_eq!(
            source.crc32().unwrap(),
            crc32fast::hash(b"streamed payload bytes")
        );
    }

    #[test]
    fn file_source_streams_across_chunk_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        let payload: Vec<u8> = (0..70_000).map(|index| (index % 253) as u8).collect();
        std::fs::write(&path, &payload).unwrap();

        for key in [None, Some(&b"file-stream-key"[..])] {
            let packed = PayloadFrame::pack(&payload, key);
            let mut source = FileSource::open(&path, 0, payload.len() as u64).unwrap();
            let mut reader = PayloadBitReader::new(&mut source, key, false).unwrap();
            for index in 0..packed.len() * 8 {
                let byte = packed[index / 8];
                let expected = (byte >> (7 - (index % 8))) & 1;
                assert_eq!(reader.next_bit().unwrap(), Some(expected));
            }
            assert_eq!(reader.next_bit().unwrap(), None);
            reader.finish().unwrap();
        }
    }

    #[test]
    fn changed_file_fails_crc_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        std::fs::write(&path, b"stable payload bytes").unwrap();
        let mut source = FileSource::open(&path, 0, 20).unwrap();
        let mut reader = PayloadBitReader::new(&mut source, None, false).unwrap();

        std::fs::write(&path, b"changed payload byte").unwrap();
        while reader.next_bit().unwrap().is_some() {}
        assert!(matches!(reader.finish(), Err(WriteError::PayloadChanged)));
    }

    #[test]
    fn missing_file_range_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        std::fs::write(&path, b"short").unwrap();
        assert!(FileSource::open(&path, 2, 10).is_err());
    }
}
