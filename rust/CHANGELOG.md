# Changelog

All notable changes to the Rust port are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
semantic versioning.

## [Unreleased]

### Added

- Cargo workspace with `rawhide-core` and `rawhide-cli`.
- Payload framing (`DNGS` magic, version/flags, u64 length, CRC32, optional
  SHA-256 counter keystream) with golden vectors from the Python implementation.
- LSB codec with the Python plane/sample mapping for `u8` and `u16` samples.
- Capacity and KiB formatting helpers matching the Python CLI.
- `rawhide capacity` CLI command.
- Read-only DNG support: `DngReader` / `RawFrame`, raw `SubIFD`s, tiled and
  stripped raws, packed 10/12/14-bit samples, uncompressed and Deflate blocks,
  multiple frames, and split markers (`ImageUniqueID`, `ImageNumber`,
  `PageNumber`, `ImageDescription`).
- `decode_files` and `rawhide decode`: 1-16 plane auto-detection, strict
  explicit planes, `--key`, `--max-bytes`, split metadata/filename ordering and
  partial-chunk warnings. Validated against Python-generated DNG fixtures.
- Writer (`core::writer`, `rawhide encode`): classic TIFF, tiled raw pages,
  `LinearRaw`/`CFA`, 8/10/12/14/16-bit, baseline JPEG preview, optional
  key/planes. Python, `tifffile`, `exiv2`, `tiffdump` and `rawpy` accept the
  output.
- Multiple raw frames (1-8) per DNG, each a full-resolution `SubIFD`, with
  per-frame seeded synthetic covers and payload striping across frames.
- Split encoding (`--split-file`, `--split-file-metadata-{id,seq}`,
  `--split-file-id`, `--no-split-file-album`) with `ImageUniqueID`/`ImageNumber`,
  `PageNumber` totals and `ImageDescription` sequence markers, DCF-style chunk
  names and the Python album layout. Python decodes Rust split sets and vice
  versa.
- Typed TIFF layer (`core::tiff`): `TagValue`/`Entry`/`IfdBuilder` with inline
  and out-of-line values, multi-IFD offsets and classic IFD serialization;
  writer rebuilt on it, including `DOUBLE` values for `NoiseProfile`.
- `core::sizing`: `AutoSizer` parity (presets, `--auto-size`, 4:3 completion,
  plane ceilings, KB-formatted errors, risk warnings).
- `core::profile` + `core::profiles`: typed `CameraProfile`, fidelity and depth
  helpers, and all 307 Python `profile.json` files embedded in the binary
  (plus the three Adobe blobs), with lazy loading and Python-style fallbacks.
- `core::metadata`: seeded metadata randomizer (lens/ISO/exposure/serials/
  timestamps/as-shot neutral/noise) honouring per-profile pools and fidelity
  ranges; `--image-time` pinning.
- Writer metadata: per-profile `Make`/`Model`/`Software`/calibration matrices,
  EXIF camera/lens/exposure block, `AsShotNeutral`, `BaselineExposure` and
  `NoiseProfile`. DNG colour/profile tags live in IFD0 (not the raw SubIFD),
  fidelity tag-presence flags/allow-lists are honoured, and `--level-policy`
  changes emitted white/black levels, synthetic-cover range, CFA pattern,
  black-level repeat dims and crop/bayer-split tags.
- CLI: `--auto-size`, auto-selected LSB planes, `--camera-profile`,
  `--level-policy`, `--image-time` (keywords and RFC 3339 with local offset),
  `auto-config`/risk warning output and profile `suggested_mode`.
- Tooling: rustfmt, clippy, nextest, llvm-cov, cargo-deny, cargo-audit, insta,
  proptest, criterion, `just`, CI workflow, roadmap/architecture/testing docs.
