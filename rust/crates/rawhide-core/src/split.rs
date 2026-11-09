//! Split-file metadata fields, chunk naming and size parsing.
//!
//! The wire names and defaults match the Python implementation:
//! `ImageUniqueID` for the shared chunk UUID and `ImageNumber` for the
//! per-chunk sequence. Decode can fall back to the `out0001.dng` filename
//! convention when the metadata fields are absent.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use thiserror::Error;

/// Metadata field carrying the shared chunk-set UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitIdField {
    /// EXIF `ImageUniqueID` (tag 42016).
    #[default]
    ImageUniqueId,
    /// IFD0 `ImageDescription` (tag 270).
    ImageDescription,
    /// Do not write/read a shared UUID.
    None,
}

impl SplitIdField {
    /// CLI/wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ImageUniqueId => "ImageUniqueID",
            Self::ImageDescription => "ImageDescription",
            Self::None => "none",
        }
    }

    /// Whether the field is in use.
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }
}

impl fmt::Display for SplitIdField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SplitIdField {
    type Err = SplitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ImageUniqueID" => Ok(Self::ImageUniqueId),
            "ImageDescription" => Ok(Self::ImageDescription),
            "none" => Ok(Self::None),
            other => Err(SplitError::UnknownIdField {
                field: other.to_owned(),
            }),
        }
    }
}

/// Metadata field carrying the per-chunk sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitSeqField {
    /// EXIF `ImageNumber` (tag 37393), scalar, no total.
    #[default]
    ImageNumber,
    /// IFD0 `PageNumber` (tag 297), `(sequence, total)`.
    PageNumber,
    /// IFD0 `ImageDescription` (tag 270), `"NNNN/MMMM"`.
    ImageDescription,
    /// Do not write/read a sequence.
    None,
}

impl SplitSeqField {
    /// CLI/wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ImageNumber => "ImageNumber",
            Self::PageNumber => "PageNumber",
            Self::ImageDescription => "ImageDescription",
            Self::None => "none",
        }
    }

    /// Whether the field is in use.
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }
}

impl fmt::Display for SplitSeqField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SplitSeqField {
    type Err = SplitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ImageNumber" => Ok(Self::ImageNumber),
            "PageNumber" => Ok(Self::PageNumber),
            "ImageDescription" => Ok(Self::ImageDescription),
            "none" => Ok(Self::None),
            other => Err(SplitError::UnknownSeqField {
                field: other.to_owned(),
            }),
        }
    }
}

/// Split markers read from one chunk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SplitMarkers {
    /// Shared chunk-set UUID, when present.
    pub id: Option<String>,
    /// Chunk sequence number (1-based), when present.
    pub seq: Option<u32>,
    /// Declared total chunk count, when the field carries one.
    pub total: Option<u32>,
    /// Whether the configured UUID field was present.
    pub id_present: bool,
    /// Whether the configured sequence field was present.
    pub seq_present: bool,
}

/// Errors from split-field parsing, size parsing and chunk ordering.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SplitError {
    /// Unknown `--split-file-metadata-id` value.
    #[error("unknown split id field {field:?}: choose from ImageDescription, ImageUniqueID, none")]
    UnknownIdField {
        /// Invalid field name.
        field: String,
    },

    /// Unknown `--split-file-metadata-seq` value.
    #[error(
        "unknown split seq field {field:?}: choose from ImageDescription, ImageNumber, PageNumber, none"
    )]
    UnknownSeqField {
        /// Invalid field name.
        field: String,
    },

    /// The same metadata field was selected for both UUID and sequence.
    #[error("split id and seq fields collide on {field:?}: pick different fields")]
    FieldCollision {
        /// Colliding field name.
        field: String,
    },

    /// A sequence value could not be parsed.
    #[error("cannot parse chunk sequence from {value:?} (expected 'NNNN/MMMM')")]
    InvalidSequence {
        /// Raw value.
        value: String,
    },

    /// A size string could not be parsed.
    #[error("invalid size {text:?}: expected like 1g, 100m, 500k")]
    InvalidSize {
        /// Raw size text.
        text: String,
    },

    /// A size unit was not recognized.
    #[error("invalid size unit {unit:?} in {text:?}: use k, m or g")]
    InvalidSizeUnit {
        /// Raw size text.
        text: String,
        /// Unit text.
        unit: String,
    },

    /// A size was zero or negative.
    #[error("split size must be positive (got {text:?})")]
    NonPositiveSize {
        /// Raw size text.
        text: String,
    },

    /// A filename has no 4-digit chunk sequence.
    #[error(
        "{path}: no split metadata found in any file, and this name lacks a 4-digit sequence number like out0001.dng"
    )]
    ChunkName {
        /// Offending path.
        path: PathBuf,
    },

    /// Filenames mix stems or suffixes.
    #[error("chunk filenames mix stems/suffixes ({detail}); pass one sequence with a shared stem")]
    MixedNames {
        /// Human-readable detail.
        detail: String,
    },

    /// Duplicate chunk numbers.
    #[error("duplicate chunk numbers {numbers}; pass each chunk once")]
    DuplicateSequence {
        /// Comma-separated numbers.
        numbers: String,
    },

    /// Chunk numbers are not consecutive from 0001.
    #[error("chunk numbers not consecutive (got {got}): missing chunk(s) {missing}")]
    NotConsecutive {
        /// Comma-separated numbers present.
        got: String,
        /// Comma-separated numbers missing.
        missing: String,
    },

    /// A file carries a different chunk-set UUID.
    #[error("chunk files belong to different sets (shared UUIDs differ): {detail}")]
    DifferentSets {
        /// Human-readable detail.
        detail: String,
    },

    /// Chunk metadata declares conflicting totals.
    #[error("conflicting chunk totals: {totals:?}")]
    ConflictingTotals {
        /// Totals found.
        totals: Vec<u32>,
    },

    /// The declared total does not match the supplied files.
    #[error(
        "chunk metadata declares {total} total chunks but {given} files were given; missing chunk(s): {missing}"
    )]
    MissingChunks {
        /// Declared total.
        total: u32,
        /// Number of files supplied.
        given: usize,
        /// Missing numbers.
        missing: String,
    },
}

/// Validate a split-field pair; the same field cannot store both values.
pub fn validate_fields(id_field: SplitIdField, seq_field: SplitSeqField) -> Result<(), SplitError> {
    if id_field == SplitIdField::ImageDescription && seq_field == SplitSeqField::ImageDescription {
        return Err(SplitError::FieldCollision {
            field: "ImageDescription".to_owned(),
        });
    }
    Ok(())
}

/// Parse `1g`, `100m`, `500k`, `2G`, `64KB`, or a bare byte count.
#[allow(clippy::cast_precision_loss)] // Sizes are far below 2^53 in practice.
pub fn parse_size(text: &str) -> Result<u64, SplitError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(SplitError::InvalidSize {
            text: text.to_owned(),
        });
    }

    let number_end = trimmed
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_digit() && *ch != '.')
        .map_or(trimmed.len(), |(index, _)| index);
    let (number_text, unit_text) = trimmed.split_at(number_end);
    if number_text.is_empty() || number_text == "." {
        return Err(SplitError::InvalidSize {
            text: text.to_owned(),
        });
    }
    let number: f64 = number_text.parse().map_err(|_| SplitError::InvalidSize {
        text: text.to_owned(),
    })?;
    let unit = unit_text.trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1_u64,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024 * 1024,
        "g" | "gb" => 1024 * 1024 * 1024,
        _ => {
            return Err(SplitError::InvalidSizeUnit {
                text: text.to_owned(),
                unit,
            });
        }
    };
    let size = number * multiplier as f64;
    if !size.is_finite() || size <= 0.0 {
        return Err(SplitError::NonPositiveSize {
            text: text.to_owned(),
        });
    }
    Ok(size as u64)
}

/// Chunk bounds for a payload and maximum payload bytes per chunk.
///
/// Returns `(sequence, total, start, end)` with 1-based sequence. An empty
/// payload yields one empty chunk, matching Python.
pub fn chunk_bounds(
    payload_len: usize,
    split_size: u64,
) -> Result<Vec<(u32, u32, usize, usize)>, SplitError> {
    if split_size == 0 {
        return Err(SplitError::NonPositiveSize {
            text: "0".to_owned(),
        });
    }
    let payload_len_u64 = u64::try_from(payload_len).unwrap_or(u64::MAX);
    let total_u64 = if payload_len_u64 == 0 {
        1
    } else {
        payload_len_u64.div_ceil(split_size)
    };
    let total = u32::try_from(total_u64).map_err(|_| SplitError::InvalidSize {
        text: format!("{total_u64} chunks exceed the 4-digit sequence limit"),
    })?;
    let chunk = usize::try_from(split_size).unwrap_or(usize::MAX);
    let mut bounds = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for seq in 1..=total {
        let start =
            usize::try_from(u64::from(seq - 1).saturating_mul(split_size)).unwrap_or(usize::MAX);
        if start > payload_len {
            break;
        }
        let end = start.saturating_add(chunk).min(payload_len);
        bounds.push((seq, total, start, end));
    }
    Ok(bounds)
}

/// Insert a DCF-style 4-digit sequence before the extension.
///
/// `out.dng` + 1 -> `out0001.dng`; no-extension names get `name0001`.
#[must_use]
pub fn chunk_path(output: &Path, seq: u32) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("out");
    let suffix = output
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| format!(".{extension}"));
    output.with_file_name(format!("{stem}{seq:04}{suffix}"))
}

/// Directory grouping split chunks: `<output-parent>/<input-stem>`.
#[must_use]
pub fn split_album_dir(output: &Path, input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("album");
    output.parent().unwrap_or_else(|| Path::new("")).join(stem)
}

/// Split a basename into `(stem, number, suffix)`; `None` without 4 digits.
///
/// `out0001.dng` -> `("out", 1, ".dng")`. The suffix must be alphanumeric
/// (the Python regex allows the same).
#[must_use]
pub fn parse_chunk_name(path: &Path) -> Option<(String, u32, String)> {
    let name = path.file_name()?.to_str()?;
    let (base, suffix) = match name.rsplit_once('.') {
        Some((base, extension))
            if !extension.is_empty() && extension.chars().all(|ch| ch.is_ascii_alphanumeric()) =>
        {
            (base, format!(".{extension}"))
        }
        _ => (name, String::new()),
    };

    if base.len() < 4 {
        return None;
    }
    let (stem, digits) = base.split_at(base.len() - 4);
    if !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let number = digits.parse().ok()?;
    Some((stem.to_owned(), number, suffix))
}

/// Order chunk paths by their 4-digit names, requiring 1..N consecutive.
pub fn order_by_names(paths: &[PathBuf]) -> Result<Vec<PathBuf>, SplitError> {
    let mut parsed = Vec::with_capacity(paths.len());
    for path in paths {
        let Some((stem, number, suffix)) = parse_chunk_name(path) else {
            return Err(SplitError::ChunkName { path: path.clone() });
        };
        parsed.push((path.clone(), stem, number, suffix));
    }

    let stems: std::collections::BTreeSet<&str> =
        parsed.iter().map(|(_, stem, _, _)| stem.as_str()).collect();
    let suffixes: std::collections::BTreeSet<&str> = parsed
        .iter()
        .map(|(_, _, _, suffix)| suffix.as_str())
        .collect();
    if stems.len() != 1 || suffixes.len() != 1 {
        let detail = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SplitError::MixedNames { detail });
    }

    let mut numbers: Vec<u32> = parsed.iter().map(|(_, _, number, _)| *number).collect();
    numbers.sort_unstable();
    check_consecutive(&numbers)?;

    let mut ordered: Vec<(u32, PathBuf)> = parsed
        .into_iter()
        .map(|(path, _, number, _)| (number, path))
        .collect();
    ordered.sort_unstable_by_key(|(number, _)| *number);
    Ok(ordered.into_iter().map(|(_, path)| path).collect())
}

/// Order chunks by metadata, enforcing one UUID and 1..N sequences.
///
/// When only the UUID field is active, filenames provide the order (matching
/// the Python implementation).
pub fn order_by_markers(
    paths: &[PathBuf],
    markers: &[SplitMarkers],
    id_active: bool,
    seq_active: bool,
) -> Result<Vec<PathBuf>, SplitError> {
    if paths.len() != markers.len() {
        return Err(SplitError::MixedNames {
            detail: "path/marker count mismatch".to_owned(),
        });
    }

    if id_active {
        let mut ids = std::collections::BTreeSet::new();
        for marker in markers {
            let Some(id) = marker.id.as_deref() else {
                return Err(SplitError::DifferentSets {
                    detail: "a file is missing its chunk-set UUID".to_owned(),
                });
            };
            ids.insert(id);
        }
        if ids.len() != 1 {
            let detail = paths
                .iter()
                .zip(markers)
                .map(|(path, marker)| {
                    format!(
                        "{}: {}",
                        path.display(),
                        marker.id.as_deref().unwrap_or("<missing>")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(SplitError::DifferentSets { detail });
        }
    }

    if !seq_active {
        return order_by_names(paths);
    }

    let mut pairs = Vec::with_capacity(paths.len());
    for (path, marker) in paths.iter().zip(markers) {
        let Some(seq) = marker.seq else {
            return Err(SplitError::NotConsecutive {
                got: "<missing sequence>".to_owned(),
                missing: "<unknown>".to_owned(),
            });
        };
        pairs.push((seq, path.clone(), marker.total));
    }
    pairs.sort_unstable_by_key(|(seq, _, _)| *seq);

    let numbers: Vec<u32> = pairs.iter().map(|(seq, _, _)| *seq).collect();
    check_consecutive(&numbers)?;

    let totals: std::collections::BTreeSet<u32> =
        pairs.iter().filter_map(|(_, _, total)| *total).collect();
    if totals.len() > 1 {
        return Err(SplitError::ConflictingTotals {
            totals: totals.into_iter().collect(),
        });
    }
    if let Some(&total) = totals.iter().next()
        && usize::try_from(total).unwrap_or(usize::MAX) != paths.len()
    {
        let missing = (1..=total)
            .filter(|number| !numbers.contains(number))
            .map(|number| format!("{number:04}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SplitError::MissingChunks {
            total,
            given: paths.len(),
            missing,
        });
    }

    Ok(pairs.into_iter().map(|(_, path, _)| path).collect())
}

fn check_consecutive(numbers: &[u32]) -> Result<(), SplitError> {
    let duplicates: Vec<u32> = numbers
        .windows(2)
        .filter(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
        .collect();
    if !duplicates.is_empty() {
        return Err(SplitError::DuplicateSequence {
            numbers: duplicates
                .iter()
                .map(|number| format!("{number:04}"))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    if numbers.is_empty() {
        return Ok(());
    }
    let expected: Vec<u32> = (1..=u32::try_from(numbers.len()).unwrap_or(u32::MAX)).collect();
    if numbers != expected {
        let missing = expected
            .iter()
            .filter(|number| !numbers.contains(number))
            .map(|number| format!("{number:04}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SplitError::NotConsecutive {
            got: numbers
                .iter()
                .map(|number| format!("{number:04}"))
                .collect::<Vec<_>>()
                .join(", "),
            missing,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_like_python() {
        assert_eq!(parse_size("500k").unwrap(), 500 * 1024);
        assert_eq!(parse_size("100m").unwrap(), 100 * 1024 * 1024);
        assert_eq!(parse_size("1g").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("2G").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("64KB").unwrap(), 64 * 1024);
        assert_eq!(parse_size(" 1024 ").unwrap(), 1024);
        assert!(parse_size("0").is_err());
        assert!(parse_size("bogus").is_err());
    }

    #[test]
    fn parses_chunk_names() {
        assert_eq!(
            parse_chunk_name(Path::new("out0001.dng")),
            Some(("out".to_owned(), 1, ".dng".to_owned()))
        );
        assert_eq!(
            parse_chunk_name(Path::new("album/vol0012.DNG")),
            Some(("vol".to_owned(), 12, ".DNG".to_owned()))
        );
        assert!(parse_chunk_name(Path::new("out.dng")).is_none());
    }

    #[test]
    fn orders_names_and_rejects_gaps() {
        let paths = vec![PathBuf::from("vol0002.dng"), PathBuf::from("vol0001.dng")];
        assert_eq!(
            order_by_names(&paths).unwrap(),
            vec![PathBuf::from("vol0001.dng"), PathBuf::from("vol0002.dng")]
        );
        assert!(
            order_by_names(&[PathBuf::from("vol0001.dng"), PathBuf::from("vol0003.dng")]).is_err()
        );
    }

    fn marker(id: &str, seq: u32, total: Option<u32>) -> SplitMarkers {
        SplitMarkers {
            id: Some(id.to_owned()),
            seq: Some(seq),
            total,
            id_present: true,
            seq_present: true,
        }
    }

    #[test]
    fn field_names_roundtrip_and_validate() {
        for (text, field) in [
            ("ImageUniqueID", SplitIdField::ImageUniqueId),
            ("ImageDescription", SplitIdField::ImageDescription),
            ("none", SplitIdField::None),
        ] {
            assert_eq!(text.parse::<SplitIdField>().unwrap(), field);
            assert_eq!(field.to_string(), text);
        }
        for (text, field) in [
            ("ImageNumber", SplitSeqField::ImageNumber),
            ("PageNumber", SplitSeqField::PageNumber),
            ("ImageDescription", SplitSeqField::ImageDescription),
            ("none", SplitSeqField::None),
        ] {
            assert_eq!(text.parse::<SplitSeqField>().unwrap(), field);
            assert_eq!(field.to_string(), text);
        }
        assert!("bogus".parse::<SplitIdField>().is_err());
        assert!("bogus".parse::<SplitSeqField>().is_err());
        assert!(
            validate_fields(
                SplitIdField::ImageDescription,
                SplitSeqField::ImageDescription
            )
            .is_err()
        );
    }

    #[test]
    fn orders_by_markers_and_totals() {
        let paths = vec![PathBuf::from("vol0002.dng"), PathBuf::from("vol0001.dng")];
        let markers = vec![marker("id", 2, Some(2)), marker("id", 1, Some(2))];
        assert_eq!(
            order_by_markers(&paths, &markers, true, true).unwrap(),
            vec![PathBuf::from("vol0001.dng"), PathBuf::from("vol0002.dng")]
        );

        let conflicting = vec![marker("id", 1, Some(2)), marker("id", 2, Some(3))];
        assert!(matches!(
            order_by_markers(&paths, &conflicting, true, true),
            Err(SplitError::ConflictingTotals { .. })
        ));

        let missing = vec![marker("id", 1, Some(3)), marker("id", 2, Some(3))];
        assert!(matches!(
            order_by_markers(&paths, &missing, true, true),
            Err(SplitError::MissingChunks { .. })
        ));

        let duplicate = vec![marker("id", 1, None), marker("id", 1, None)];
        assert!(matches!(
            order_by_markers(&paths, &duplicate, true, true),
            Err(SplitError::DuplicateSequence { .. })
        ));

        let different = vec![marker("a", 1, None), marker("b", 2, None)];
        assert!(matches!(
            order_by_markers(&paths, &different, true, true),
            Err(SplitError::DifferentSets { .. })
        ));
    }

    #[test]
    fn id_only_falls_back_to_names() {
        let paths = vec![PathBuf::from("vol0002.dng"), PathBuf::from("vol0001.dng")];
        let markers = vec![marker("id", 2, None), marker("id", 1, None)];
        assert_eq!(
            order_by_markers(&paths, &markers, true, false).unwrap(),
            vec![PathBuf::from("vol0001.dng"), PathBuf::from("vol0002.dng")]
        );
    }

    #[test]
    fn rejects_mixed_names_and_duplicates() {
        assert!(matches!(
            order_by_names(&[PathBuf::from("a0001.dng"), PathBuf::from("b0002.dng")]),
            Err(SplitError::MixedNames { .. })
        ));
        assert!(matches!(
            order_by_names(&[PathBuf::from("a0001.dng"), PathBuf::from("a0001.dng")]),
            Err(SplitError::DuplicateSequence { .. })
        ));
    }

    #[test]
    fn parses_decimal_sizes_and_units() {
        assert_eq!(parse_size("1.5k").unwrap(), 1536);
        assert_eq!(parse_size(" 2 mb ").unwrap(), 2 * 1024 * 1024);
        assert!(matches!(
            parse_size("1x"),
            Err(SplitError::InvalidSizeUnit { .. })
        ));
        assert!(matches!(
            parse_size("-1"),
            Err(SplitError::InvalidSize { .. })
        ));
    }
}
