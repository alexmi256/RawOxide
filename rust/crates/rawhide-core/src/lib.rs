//! Core library for `RawHide`.
//!
//! This crate is the pure-Rust home for the payload framing, LSB codec and
//! (eventually) the TIFF/DNG container logic. It deliberately has no CLI or
//! network dependencies so it can be embedded in other tools and tested in
//! isolation.
//!
//! The port targets **format compatibility** with the Python reference
//! implementation, not byte-identical output. See `docs/port-plan.md` for the
//! compatibility contract and the staged implementation plan.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod codec;
pub mod cover;
pub mod decode;
pub mod digests;
pub mod dng;
pub mod error;
pub mod framing;
pub mod geo;
pub mod metadata;
pub mod opcodes;
pub mod profile;
pub mod profiles;
pub mod sizing;
pub mod split;
pub mod thumbnail;
pub mod tiff;
pub mod writer;

pub use codec::{
    LsbCodec, capacity_bytes, capacity_bytes_total, format_kb, format_kb_hi, format_kb_lo,
};
pub use cover::{CoverError, write_cover_tiff};
pub use decode::{
    DecodeError, DecodeOptions, DecodeOutcome, DecodeStreamOutcome, decode_files,
    decode_files_to_writer, decode_files_to_writer_with_progress,
};
pub use digests::{new_raw_image_digest, raw_data_unique_id};
pub use dng::{DngError, DngReader, RawFrame};
pub use error::{CodecError, Error, FrameError, Result};
pub use framing::{FrameHeader, PayloadFrame};
pub use geo::{GeoError, Location, Zone, sample_location};
pub use metadata::{ImageTime, Metadata, MetadataRandomizer, Rng64, randomize_metadata};
pub use opcodes::{GainMap, OpcodeError};
pub use profile::{CameraProfile, FidelityConfig, LevelPolicy, ProfileError};
pub use sizing::{
    RecommendOptions, RecommendedConfig, SizingError, SizingMode, max_config, recommend,
    risk_warnings,
};
pub use split::{
    MAX_CHUNKS, SplitError, SplitIdField, SplitMarkers, SplitSeqField, chunk_bounds, chunk_path,
    order_by_markers, order_by_names, parse_chunk_name, parse_size, split_album_dir,
    validate_fields, validate_split_id,
};
pub use thumbnail::{
    PreviewSource, ThumbnailError, ThumbnailImage, rgb as thumbnail_rgb,
    rgb_with_label as thumbnail_rgb_with_label,
};
pub use writer::{
    Compression, RawMode, WriteError, WriteInfo, WriterOptions, write_dng, write_dng_from_file,
    write_dng_from_file_with_progress, write_spike_dng,
};
