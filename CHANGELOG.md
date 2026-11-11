# Changelog

All notable changes to RawOxide are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
semantic versioning.

## [Unreleased]

### Added

- Payload framing (`DNGS` magic, version/flags, 64-bit length, CRC32 and an
  optional SHA-256 counter keystream) plus the LSB plane codec for `u8`/`u16`
  samples.
- `capacity`, `encode`, `decode` and `gen-tiff` commands, including
  auto-selected geometry and LSB planes, split sets, progress output and
  JSON debug sidecars.
- Streaming encode and decode paths: payloads are read in 64 KiB chunks and raw
  bands are written/read incrementally, so CLI memory is independent of payload
  size.
- Tiled DNG writer for `LinearRaw`/`CFA` raws at 8/10/12/14/16-bit, with a
  baseline JPEG preview, classic TIFF and BigTIFF, multiple raw frames (1–8),
  optional Adobe Deflate tiles and split markers.
- Embedded registry of 307 camera profiles plus Adobe lookup blobs, driving
  make/model, lens/exposure pools, calibration, levels, crop geometry, preview
  size and tag conventions.
- Seeded EXIF/DNG metadata randomizer with per-run date anchoring,
  `--image-time` pinning, `--no-randomize`, GPS land sampling and UTC
  timestamps, maker-note envelopes, DNG raw digests and gain-map/profile opcode
  lists.
- Preview sources: synthetic gradient, Wikimedia Commons with offline fallback,
  and local images with centre-crop + Lanczos resize.
- `--debug-metadata-output` sidecar with invocation, resolved options, profile
  snapshot, drawn seeds, per-frame CRCs and a named IFD/tag inventory.
- Fixture and golden-vector test suite, property tests, CLI assertions,
  criterion benchmarks and coverage configuration.
- CI for Linux, macOS and Windows, an MSRV check, `cargo-deny`, `cargo-audit`,
  rustdoc, and a tag-triggered release workflow that publishes per-platform
  binaries with `SHA256SUMS`.

### Changed

- Project renamed to **RawOxide**; binaries and crates are now `rawoxide`,
  `rawoxide-cli` and `rawoxide-core`.
- The Rust workspace moved from `rust/` to the repository root.
- User documentation was restructured around what RawOxide does: quick-start
  scenarios, installation, building, CLI reference, advanced features and
  format documentation.
