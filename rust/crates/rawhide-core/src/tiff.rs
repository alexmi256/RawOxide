//! Minimal classic-TIFF and `BigTIFF` building blocks.
//!
//! This is the reusable layer under [`crate::writer`]: typed tag values, IFD
//! entries and little-endian directory serializers. It deliberately does not
//! know about DNG, previews or payloads.
//!
//! [`IfdBuilder::build`] emits classic directories; [`IfdBuilder::build_big`]
//! emits 8-byte counts/offsets and `IFD8`/`LONG8` values.

use thiserror::Error;

/// TIFF field type codes used by the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagType {
    /// 8-bit unsigned integer.
    Byte,
    /// NUL-terminated ASCII.
    Ascii,
    /// 16-bit unsigned integer.
    Short,
    /// 32-bit unsigned integer.
    Long,
    /// Two 32-bit unsigned integers.
    Rational,
    /// Opaque bytes.
    Undefined,
    /// Two 32-bit signed integers.
    SRational,
    /// IEEE 754 single-precision float.
    Float,
    /// IEEE 754 double.
    Double,
    /// 32-bit IFD offset.
    Ifd,
    /// 64-bit unsigned integer (`BigTIFF`).
    Long8,
    /// 64-bit IFD offset (`BigTIFF`).
    Ifd8,
}

impl TagType {
    /// TIFF type code.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Ascii => 2,
            Self::Short => 3,
            Self::Long => 4,
            Self::Rational => 5,
            Self::Undefined => 7,
            Self::SRational => 10,
            Self::Float => 11,
            Self::Double => 12,
            Self::Ifd => 13,
            Self::Long8 => 16,
            Self::Ifd8 => 18,
        }
    }

    /// Size of one value element in bytes.
    #[must_use]
    pub const fn element_size(self) -> usize {
        match self {
            Self::Byte | Self::Ascii | Self::Undefined => 1,
            Self::Short => 2,
            Self::Long | Self::Ifd | Self::Float => 4,
            Self::Rational | Self::SRational | Self::Double | Self::Long8 | Self::Ifd8 => 8,
        }
    }
}

/// Typed tag value.
#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    /// 8-bit values.
    Byte(Vec<u8>),
    /// ASCII text (the NUL terminator is added on encode).
    Ascii(String),
    /// 16-bit values.
    Short(Vec<u16>),
    /// 32-bit values.
    Long(Vec<u32>),
    /// Unsigned rationals.
    Rational(Vec<(u32, u32)>),
    /// Opaque bytes.
    Undefined(Vec<u8>),
    /// Signed rationals.
    SRational(Vec<(i32, i32)>),
    /// IEEE 754 single-precision floats.
    Float(Vec<f32>),
    /// IEEE 754 doubles.
    Double(Vec<f64>),
    /// IFD offsets (one or more child directories).
    Ifd(Vec<u32>),
    /// 64-bit unsigned values (`BigTIFF`).
    Long8(Vec<u64>),
    /// 64-bit IFD offsets (`BigTIFF`).
    Ifd8(Vec<u64>),
}

impl TagValue {
    /// Field type for this value.
    #[must_use]
    pub const fn tag_type(&self) -> TagType {
        match self {
            Self::Byte(_) => TagType::Byte,
            Self::Ascii(_) => TagType::Ascii,
            Self::Short(_) => TagType::Short,
            Self::Long(_) => TagType::Long,
            Self::Rational(_) => TagType::Rational,
            Self::Undefined(_) => TagType::Undefined,
            Self::SRational(_) => TagType::SRational,
            Self::Float(_) => TagType::Float,
            Self::Double(_) => TagType::Double,
            Self::Ifd(_) => TagType::Ifd,
            Self::Long8(_) => TagType::Long8,
            Self::Ifd8(_) => TagType::Ifd8,
        }
    }

    /// TIFF count field.
    ///
    /// ASCII counts include the trailing NUL.
    #[must_use]
    #[allow(clippy::match_same_arms)] // Arms have different vector element types.
    pub const fn count(&self) -> usize {
        match self {
            Self::Byte(values) => values.len(),
            Self::Ascii(text) => text.len() + 1,
            Self::Short(values) => values.len(),
            Self::Long(values) => values.len(),
            Self::Rational(values) => values.len(),
            Self::Undefined(values) => values.len(),
            Self::SRational(values) => values.len(),
            Self::Float(values) => values.len(),
            Self::Double(values) => values.len(),
            Self::Ifd(values) => values.len(),
            Self::Long8(values) | Self::Ifd8(values) => values.len(),
        }
    }

    /// Encode the value bytes in little-endian order.
    #[must_use]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        match self {
            Self::Byte(values) | Self::Undefined(values) => values.clone(),
            Self::Ascii(text) => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                bytes
            }
            Self::Short(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 2);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes
            }
            Self::Long(values) | Self::Ifd(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 4);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes
            }
            Self::Long8(values) | Self::Ifd8(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes
            }
            Self::Rational(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for (numerator, denominator) in values {
                    bytes.extend_from_slice(&numerator.to_le_bytes());
                    bytes.extend_from_slice(&denominator.to_le_bytes());
                }
                bytes
            }
            Self::SRational(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for (numerator, denominator) in values {
                    bytes.extend_from_slice(&numerator.to_le_bytes());
                    bytes.extend_from_slice(&denominator.to_le_bytes());
                }
                bytes
            }
            Self::Float(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 4);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes
            }
            Self::Double(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes
            }
        }
    }
}

/// One IFD directory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// TIFF tag code.
    pub tag: u16,
    /// Typed value.
    pub value: TagValue,
}

impl Entry {
    /// Create an entry from a typed value.
    #[must_use]
    pub const fn new(tag: u16, value: TagValue) -> Self {
        Self { tag, value }
    }

    /// SHORT array entry.
    #[must_use]
    pub fn shorts(tag: u16, values: &[u16]) -> Self {
        Self::new(tag, TagValue::Short(values.to_vec()))
    }

    /// LONG array entry.
    #[must_use]
    pub fn longs(tag: u16, values: &[u32]) -> Self {
        Self::new(tag, TagValue::Long(values.to_vec()))
    }

    /// RATIONAL array entry.
    #[must_use]
    pub fn rationals(tag: u16, values: &[(u32, u32)]) -> Self {
        Self::new(tag, TagValue::Rational(values.to_vec()))
    }

    /// SRATIONAL array entry.
    #[must_use]
    pub fn srationals(tag: u16, values: &[(i32, i32)]) -> Self {
        Self::new(tag, TagValue::SRational(values.to_vec()))
    }

    /// IEEE 754 float array entry.
    #[must_use]
    pub fn floats(tag: u16, values: &[f32]) -> Self {
        Self::new(tag, TagValue::Float(values.to_vec()))
    }

    /// IEEE 754 double array entry.
    #[must_use]
    pub fn doubles(tag: u16, values: &[f64]) -> Self {
        Self::new(tag, TagValue::Double(values.to_vec()))
    }

    /// NUL-terminated ASCII entry.
    #[must_use]
    pub fn ascii(tag: u16, text: &str) -> Self {
        Self::new(tag, TagValue::Ascii(text.to_owned()))
    }

    /// BYTE or UNDEFINED entry.
    ///
    /// # Panics
    ///
    /// Panics when `dtype` is not [`TagType::Byte`] or
    /// [`TagType::Undefined`]; this is a programmer error in the writer.
    #[must_use]
    pub fn bytes(tag: u16, dtype: TagType, bytes: Vec<u8>) -> Self {
        match dtype {
            TagType::Byte => Self::new(tag, TagValue::Byte(bytes)),
            TagType::Undefined => Self::new(tag, TagValue::Undefined(bytes)),
            _ => panic!("Entry::bytes only supports Byte or Undefined"),
        }
    }

    /// One child-IFD offset.
    #[must_use]
    pub fn ifd(tag: u16, offset: u32) -> Self {
        Self::new(tag, TagValue::Ifd(vec![offset]))
    }

    /// Multiple child-IFD offsets (`SubIFDs` with N frames).
    #[must_use]
    pub fn ifds(tag: u16, offsets: &[u32]) -> Self {
        Self::new(tag, TagValue::Ifd(offsets.to_vec()))
    }

    /// One 64-bit integer value (`BigTIFF`).
    #[must_use]
    pub fn long8(tag: u16, value: u64) -> Self {
        Self::new(tag, TagValue::Long8(vec![value]))
    }

    /// Multiple 64-bit child-IFD offsets (`BigTIFF`).
    #[must_use]
    pub fn ifds8(tag: u16, offsets: &[u64]) -> Self {
        Self::new(tag, TagValue::Ifd8(offsets.to_vec()))
    }

    /// Multiple 64-bit integers (`BigTIFF`).
    #[must_use]
    pub fn long8s(tag: u16, values: &[u64]) -> Self {
        Self::new(tag, TagValue::Long8(values.to_vec()))
    }
}

/// Minimal classic little-endian IFD builder.
#[derive(Debug, Clone, Default)]
pub struct IfdBuilder {
    entries: Vec<Entry>,
}

impl IfdBuilder {
    /// Empty directory.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Build from an entry list.
    #[must_use]
    pub const fn from_entries(entries: Vec<Entry>) -> Self {
        Self { entries }
    }

    /// Append one entry.
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Append many entries.
    pub fn extend(&mut self, entries: impl IntoIterator<Item = Entry>) {
        self.entries.extend(entries);
    }

    /// Entries, sorted by tag.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Serialize the directory at `ifd_offset`, appending out-of-line values.
    ///
    /// # Errors
    ///
    /// Returns [`Error`](enum@Error) when the entry/count/offset arithmetic exceeds classic
    /// TIFF limits.
    pub fn build(&self, ifd_offset: u32) -> Result<Vec<u8>, Error> {
        let mut sorted: Vec<&Entry> = self.entries.iter().collect();
        sorted.sort_by_key(|entry| entry.tag);

        let entry_count =
            u16::try_from(sorted.len()).map_err(|_| Error::TooManyEntries(sorted.len()))?;
        let entry_bytes = u32::try_from(sorted.len())
            .map_err(|_| Error::TooManyEntries(sorted.len()))?
            .checked_mul(12)
            .ok_or(Error::OffsetOverflow)?;
        let data_start = ifd_offset
            .checked_add(2)
            .and_then(|value| value.checked_add(entry_bytes))
            .and_then(|value| value.checked_add(4))
            .ok_or(Error::OffsetOverflow)?;

        let mut directory = Vec::new();
        directory.extend_from_slice(&entry_count.to_le_bytes());
        let mut data = Vec::new();

        for entry in sorted {
            if matches!(entry.value.tag_type(), TagType::Long8 | TagType::Ifd8) {
                return Err(Error::UnsupportedBigType(entry.tag));
            }
            let encoded = entry.value.to_le_bytes();
            let count = u32::try_from(entry.value.count()).map_err(|_| Error::CountOverflow)?;
            directory.extend_from_slice(&entry.tag.to_le_bytes());
            directory.extend_from_slice(&entry.value.tag_type().code().to_le_bytes());
            directory.extend_from_slice(&count.to_le_bytes());

            if encoded.len() <= 4 {
                let mut field = [0_u8; 4];
                field[..encoded.len()].copy_from_slice(&encoded);
                directory.extend_from_slice(&field);
            } else {
                let data_len = u32::try_from(data.len()).map_err(|_| Error::OffsetOverflow)?;
                let offset = data_start
                    .checked_add(data_len)
                    .ok_or(Error::OffsetOverflow)?;
                directory.extend_from_slice(&offset.to_le_bytes());
                data.extend_from_slice(&encoded);
            }
        }

        directory.extend_from_slice(&0_u32.to_le_bytes());
        directory.extend_from_slice(&data);
        Ok(directory)
    }
}

impl IfdBuilder {
    /// Serialize a `BigTIFF` directory at `ifd_offset`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`](enum@Error) for entry/count/offset overflows.
    pub fn build_big(&self, ifd_offset: u64) -> Result<Vec<u8>, Error> {
        let mut sorted: Vec<&Entry> = self.entries.iter().collect();
        sorted.sort_by_key(|entry| entry.tag);
        let entry_count =
            u64::try_from(sorted.len()).map_err(|_| Error::TooManyEntries(sorted.len()))?;
        let entry_bytes = u64::try_from(sorted.len())
            .map_err(|_| Error::TooManyEntries(sorted.len()))?
            .checked_mul(20)
            .ok_or(Error::BigOffsetOverflow)?;
        let data_start = ifd_offset
            .checked_add(8)
            .and_then(|value| value.checked_add(entry_bytes))
            .and_then(|value| value.checked_add(8))
            .ok_or(Error::BigOffsetOverflow)?;
        let mut directory = Vec::new();
        directory.extend_from_slice(&entry_count.to_le_bytes());
        let mut data = Vec::new();
        for entry in sorted {
            let encoded = entry.value.to_le_bytes();
            let count = u64::try_from(entry.value.count()).map_err(|_| Error::CountOverflow)?;
            directory.extend_from_slice(&entry.tag.to_le_bytes());
            directory.extend_from_slice(&entry.value.tag_type().code().to_le_bytes());
            directory.extend_from_slice(&count.to_le_bytes());
            if encoded.len() <= 8 {
                let mut field = [0_u8; 8];
                field[..encoded.len()].copy_from_slice(&encoded);
                directory.extend_from_slice(&field);
            } else {
                let data_len = u64::try_from(data.len()).map_err(|_| Error::BigOffsetOverflow)?;
                let offset = data_start
                    .checked_add(data_len)
                    .ok_or(Error::BigOffsetOverflow)?;
                directory.extend_from_slice(&offset.to_le_bytes());
                data.extend_from_slice(&encoded);
            }
        }
        directory.extend_from_slice(&0_u64.to_le_bytes());
        directory.extend_from_slice(&data);
        Ok(directory)
    }
}

/// TIFF builder errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// Too many entries for a classic TIFF directory.
    #[error("IFD has {0} entries; classic TIFF supports at most 65535")]
    TooManyEntries(usize),

    /// A value count does not fit the 32-bit count field.
    #[error("tag value count exceeds 32 bits")]
    CountOverflow,

    /// A file offset does not fit the 32-bit classic TIFF field.
    #[error("classic TIFF offset overflow (file would exceed 4 GiB)")]
    OffsetOverflow,

    /// A `BigTIFF` offset overflowed 64 bits.
    #[error("BigTIFF offset/count arithmetic overflow")]
    BigOffsetOverflow,

    /// Classic TIFF cannot encode a 64-bit value.
    #[error("classic TIFF cannot encode 64-bit tag {0}")]
    UnsupportedBigType(u16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_count_includes_nul_and_inline_values_are_padded() {
        let value = TagValue::Ascii("abc".to_owned());
        assert_eq!(value.count(), 4);
        assert_eq!(value.to_le_bytes(), b"abc\0");

        let builder =
            IfdBuilder::from_entries(vec![Entry::shorts(50, &[1, 2]), Entry::ascii(100, "abc")]);
        let bytes = builder.build(8).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 2);
        // Entries are sorted by tag: 50 then 100.
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 50);
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 100);
        // ASCII "abc\0" fits inline in the 4-byte value field.
        assert_eq!(&bytes[22..26], b"abc\0");
    }

    #[test]
    fn out_of_line_values_get_absolute_offsets() {
        let builder = IfdBuilder::from_entries(vec![Entry::rationals(200, &[(1, 2)])]);
        let bytes = builder.build(100).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 1);
        // Count, type, count, offset.
        let offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(offset, 100 + 2 + 12 + 4);
        assert_eq!(&bytes[18..26], &[1, 0, 0, 0, 2, 0, 0, 0]);
    }

    #[test]
    fn bigtiff_uses_8_byte_values_and_offsets() {
        let builder = IfdBuilder::from_entries(vec![
            Entry::shorts(50, &[1, 2]),
            Entry::ascii(100, "abcdefgh"),
        ]);
        let bytes = builder.build_big(100).unwrap();
        assert_eq!(u64::from_le_bytes(bytes[0..8].try_into().unwrap()), 2);
        // Entry 1: tag 50, SHORT, count 2, value inline in 8 bytes.
        assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 50);
        assert_eq!(&bytes[20..24], &[1, 0, 2, 0]);
        // Entry 2: ASCII count 9 > 8, so the value is out-of-line.
        assert_eq!(u16::from_le_bytes(bytes[28..30].try_into().unwrap()), 100);
        let offset = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        assert_eq!(offset, 100 + 8 + 40 + 8);
        assert_eq!(&bytes[56..65], b"abcdefgh\0");
        // Next-IFD field occupies 8 bytes after the entries.
        assert_eq!(&bytes[48..56], &[0; 8]);
    }

    #[test]
    fn multiple_ifd_offsets_are_encoded_as_ifd_type() {
        let entry = Entry::ifds(330, &[100, 200]);
        assert_eq!(entry.value.tag_type(), TagType::Ifd);
        assert_eq!(entry.value.count(), 2);
        assert_eq!(entry.value.to_le_bytes(), [100, 0, 0, 0, 200, 0, 0, 0]);
    }
}
