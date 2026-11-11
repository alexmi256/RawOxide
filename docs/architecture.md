# Architecture

## Workspace

RawOxide is a Cargo workspace with one library and one binary crate:

```text
crates/rawoxide-core      library: codecs, DNG I/O, profiles, metadata
  framing                 DNGS frame header + SHA-256 keystream
  codec                   LSB mapping, capacity, KiB formatting
  tiff                    typed tag values + classic/BigTIFF IFD builder
  dng                     DNG/raw-block reader
  profile/profiles        embedded camera registry, depth/level/fidelity math
  metadata                seeded EXIF/DNG fields + per-run date anchor
  geo                     embedded tz-city/land table + GPS sampling
  cover                   synthetic raw covers
  thumbnail               synthetic/Commons/offline-noise/file previews
  sizing                  auto-config / smallest fitting geometry
  split                   chunk naming, sequencing, id/count validation
  digests                 DNG SDK raw-image / unique-ID hashes
  opcodes                 gain-map and profile opcode lists
  writer                  tiled DNG writer, split markers, sidecar statistics

crates/rawoxide-cli       clap CLI, progress, errors, exit codes
  commands                capacity, decode, gen-tiff
  encode                  encode orchestration and split sets
  sidecar                 JSON debug document + tag inventory
```

Dependencies point one way: `rawoxide-cli → rawoxide-core`. Core never reads
process arguments, never installs a global logger and performs no network I/O
unless a feature explicitly asks for it (the optional Commons preview source).

## Encode data flow

```text
payload file range
  → streaming frame header + CRC (64 KiB chunks, optional XOR)
  → cover generation (bands, deterministic per-band RNG)
  → LSB embed in place in the same band buffer
  → TIFF writer (tiles, compression, preview, EXIF/GPS)
  → post-write patch/link only where the writer cannot express a structure
```

Only one payload chunk, one band and one tile buffer are live at a time. The
payload is read from disk as needed; a multi-gigabyte payload is never
materialized beside the frame. The embedding pass re-hashes the source file and
fails closed if it changes mid-write.

## Decode data flow

```text
DNG file
  → TIFF reader parses IFD0 + SubIFDs (bounded offsets/counts)
  → RawBandReader yields tile rows/strips in raster order
  → header probe (first ≤144 samples), then LSB extract into a payload sink
  → stop reading once the declared payload is complete
  → incremental CRC verification (wrong key fails closed)
  → payload Vec for the library API, or a writer for the streaming CLI path
```

Only one band, one decoded block and a small sink buffer are live at a time.
`decode_files` returns a payload-sized `Vec` for embedding; `decode_files_to_writer`
streams bytes to a caller-provided writer. The CLI uses the streaming form with
a temporary file in the destination directory and renames it only after the CRC
passes.

Auto-detection tries plane counts 1–16 and accepts the first frame that passes
magic and CRC. `--max-bytes` is checked before allocating the payload buffer.

## Error model

- Library: `thiserror` enums per module (`FrameError`, `CodecError`,
  `TiffError`, …). Callers can distinguish "no magic, try the next plane" from a
  capacity rejection.
- CLI: `anyhow::Result` at the boundary, `error:` prefix on stderr, exit code 1.
- User-facing sizes use binary KiB/MiB/GiB formatting.
- Panics are bugs. Malformed input must return an error.

## Determinism

- Seeded output is stable within RawOxide and is not tied to another
  implementation's RNG.
- RNG streams are explicit and named: cover, metadata, thumbnail, location,
  split. A stream is not shared between fields whose draw order could change.
- Rolling date windows use one anchor per encode run, so split chunks share a
  window and a wall-clock tick cannot shift seeded metadata.
- No `HashMap` iteration in write paths. Use `BTreeMap` or sorted keys.
- Parallel work, when added, must seed each unit and write results in index
  order so the byte stream is independent of scheduling.

## Memory and concurrency

- Encode and decode are single-threaded; correctness and bounded memory come
  first.
- Cover generation, embedding and tile writing share one band buffer.
- Raw frames are generated, embedded, encoded and written one at a time.
- The SDK image digest and per-frame debug CRCs are computed incrementally, so
  neither all frames nor a full raster are retained.
- Decode releases raw frames before unpacking and reuses the extracted buffer.
- `unsafe` is forbidden in core. If mmap becomes necessary, it must be isolated
  behind a small module with a documented safety contract and an ADR.

## Dependency policy

- Prefer pure-Rust, MIT/Apache-compatible dependencies; a default build needs no
  C toolchain.
- `tiff-reader` is pinned exactly (`=0.11.x`) until its API settles.
- `cargo-deny` enforces licenses, advisories and sources; `cargo-audit` runs in
  CI.
- Rust 1.99.1 is the minimum toolchain (edition 2024), checked separately in CI.

## DNG reader and `tiff-reader`

`tiff-reader` 0.11 parses IFDs, tags, EXIF pointers and tile/strip tables, but
rejects DNG `LinearRaw`/`CFA` in its colour-model validation and lacks classic
TIFF type 13 (`IFD`) for `SubIFDs`. `rawoxide-core::dng` therefore uses the crate
for structure, parses `SubIFDs` offsets itself (classic type 13 and BigTIFF type
18), and reads raw blocks itself: tiles/strips, Deflate, predictor 2 and packed
samples. This is isolated behind `DngReader`, so replacing or patching the crate
later does not affect the decode or CLI layers. See ADR 0003.

## Writer

The writer lives in `rawoxide-core::writer` and builds on `rawoxide-core::tiff`
(typed tag values + classic/BigTIFF IFD builder). It emits:

- tiled `LinearRaw`/`CFA` raw pages at 8/10/12/14/16-bit;
- a baseline JPEG preview;
- DNG calibration and colour tags, optionally Adobe profile tags;
- 1–8 full-resolution raw frames with payload striping;
- split markers in IFD0/EXIF;
- a profile/EXIF metadata block, optional GPS and maker note;
- optional Deflate-compressed tiles and BigTIFF offsets.

Profile `fidelity.raw.tile` drives tile geometry, and cropped CFA outputs carry
`ActiveArea`/`DefaultUserCrop`. Frames are written one at a time and banded by
tile row.

## Debug sidecar

`--debug-metadata-output` writes a JSON sidecar beside the output, replacing
its extension with `.json` (or appending `.dng.json` when the output already
ends in `.json`). It has a stable schema:

```text
schema, schema_version, generated_at, tool, invocation, summary,
encode (options, payload, output, capacity, split),
profile (full static snapshot, binary blobs hashed),
cover, thumbnail, metadata, file
```

`file` contains `ifd0`, `subifds`, optional `exif`/`gps` arrays and a
`preview_jpeg` record (`bytes`, `crc32`, `strips`). Each tag has `tag`, `name`,
`type`, `count` and `value`. Values longer than 256 encoded bytes become
`{"bytes": <length>, "crc32": "<8 hex digits>"}` so XMP, opcode lists and offset
tables do not bloat the document.

The document records the drawn metadata/cover seeds, per-frame embedded-raw
CRCs, the framed payload CRC, and the resolved thumbnail label and RGB CRC, so a
run can be audited or reproduced.

## File-format contract

The wire format is described in [`format.md`](format.md). The DNG structure is
defined by the TIFF 6.0 and DNG 1.6 specifications plus the tag sets in
`writer/entries.rs` and the profile assets. Tag inventory is validated with
consumer tools in the test workflow.
