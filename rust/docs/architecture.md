# Architecture

## Workspace

```text
rawhide-core        library, no CLI/network policy
  framing           DNGS frame + SHA-256 keystream
  profile/profiles  camera registry, depth/level helpers
  metadata          seeded per-file EXIF/DNG fields
  geo               embedded tz-city/land table + GPS sampling
  sizing            auto-config / smallest fitting geometry
  digests           DNG SDK raw image / unique-ID hashes
  codec             LSB mapping, capacity, KiB formatting
  tiff              typed tag values + classic TIFF IFD builder
  dng               read-only DNG/raw block reader
  writer            DNG writer (tiles, LinearRaw/CFA, JPEG preview)
  profile           embedded profiles + level math (planned)
  metadata          seeded EXIF randomization (planned)
  geo               tz city + land sampler (planned)
  cover             synthetic raw covers (planned)
  thumbnail         synthetic/offline-noise/file preview + resize
  sizing            auto-fit search (planned)
  split             chunk naming/sequence
  digests           DNG SDK tile MD5 digests (planned)
  opcodes           GainMap/OpcodeList2 (planned)
  makernote         HDRP envelope (planned)
  debug             JSON sidecar (planned)

rawhide-cli         clap CLI, progress, errors, exit codes
```

Dependencies point one way: `rawhide-cli → rawhide-core`. Core never reads
process arguments, never installs a global logger and never performs network
I/O unless a feature explicitly asks for it.

## Encode data flow (target)

```text
payload file
  → PayloadFrame::pack (streaming CRC/length, optional XOR)
  → cover generation (bands, deterministic per-band RNG)
  → LSB embed (in place, same band buffer)
  → TIFF writer (tiles, compression, preview, EXIF/GPS)
  → post-write patch/link (only where the writer cannot express it)
```

Only one band and one tile buffer are live at a time. The payload is streamed
from disk; a 1 GB payload must not be materialized beside the frame.

## Decode data flow (target)

```text
DNG file
  → TIFF reader parses IFD0 + SubIFDs (bounded offsets/counts)
  → read only the tiles covering the required bit prefix
  → LSB extract (header first, then declared payload length)
  → PayloadFrame::unpack (CRC; wrong key fails closed)
  → payload file
```

Auto-detection tries plane counts 1–16 and accepts the first frame that passes
magic + CRC, matching Python. `--max-bytes` is checked *before* allocating the
payload buffer.

## Error model

- Library: `thiserror` enums per module (`FrameError`, `CodecError`,
  `TiffError`, …). Callers can distinguish "no magic, try next plane" from a
  capacity rejection.
- CLI: `anyhow::Result` at the boundary, `error:` prefix on stderr, exit 1.
  User-facing sizes always use `format_kb*`.
- Panics are bugs. Malformed input must return an error. Fuzz targets enforce
  this for parsers.

## Determinism

- Seeded output is stable **within this implementation**. It is not required to
  match Python byte-for-byte.
- RNG streams are explicit and named: cover, metadata, thumbnail, location,
  split. Do not share a stream across fields whose draw order could change.
- Parallel work uses per-band/per-tile seeds; the final byte stream is
  independent of scheduling.
- No `HashMap` iteration in write paths. Use `BTreeMap`, `IndexMap` with sorted
  insertion, or sort keys.

## Memory and concurrency

- Phase 1–2 are single-threaded to establish correctness.
- Phase 6 adds `rayon` for independent bands/tiles. Determinism is preserved by
  seeding each unit and writing results in index order.
- `unsafe` is forbidden in core. If mmap becomes necessary, isolate it behind a
  small module with a documented safety contract and an ADR.

## Dependency policy

- Prefer pure-Rust, MIT/Apache dependencies; no C toolchain for a default build.
- Pin exact versions for the TIFF stack (`=0.11.x`) until its API settles.
- `cargo-deny` enforces licenses/advisories/sources; `cargo-audit` runs in CI.
- MSRV is 1.88 (edition 2024 + `tiff-writer` requirement). CI checks it when the
  dependency graph allows.

## DNG reader and `tiff-reader`

`tiff-reader` 0.11 parses IFDs/tags, EXIF pointers and tile/strip tables, but
rejects `LinearRaw`/`CFA` in its color-model validation and lacks classic
TIFF type 13 (`IFD`) for `SubIFDs`. `rawhide-core::dng` therefore uses the crate
for structure and reads raw blocks itself (tiles/strips, Deflate, predictor 2,
packed samples). This is isolated behind `DngReader`; replacing or patching the
crate later does not affect the decode/CLI layers. See ADR 0003.

`core::profile` types the Python `CameraProfile`/`FidelityConfig`; `core::profiles`
embeds the 307-file registry at compile time and loads it lazily. `core::sizing`
implements the Python `AutoSizer` contract. `core::metadata` draws reproducible
per-file fields from a local SplitMix64 stream, honouring profile pools and
fidelity ranges; `--image-time` pins the capture timestamp. `core::geo` loads
the embedded `zones.bin` table once (`scripts/embed_geo.py` refreshes it), samples land-only points with a separate
RNG stream and resolves zone offsets through `chrono-tz`; GPS tags are written
to a dedicated GPS IFD. `core::digests` reproduces the DNG SDK fast image
digest (tiled MD5) and `RawDataUniqueID`, written for single-frame profiles
that request raw digests.

The writer lives in `rawhide-core::writer` and builds on `rawhide-core::tiff`
(typed tag values + classic IFD builder): tiled `LinearRaw`/`CFA` raw pages at
8/10/12/14/16-bit, a JPEG preview, DNG calibration tags, 1-8 full-resolution raw
frames, split markers in IFD0/EXIF, a profile/EXIF metadata block and the
payload. Payload bytes are striped across frames in order, and split chunks use
DCF-style names. Frames are generated, embedded, encoded and written one at a time, and
non-digest single frames are banded by tile row, so neither all frames nor a
full raster are retained. Digest-enabled frames still materialize the frame to
compute the SDK digest. Remaining work is Commons thumbnails and Adobe tag
conventions. It was
validated with Python, `tifffile`, `exiv2`, `tiffdump` and `rawpy`, including
multi-frame and split sets.

## File-format contract

The wire format is defined by the Python implementation, not by this document:

- frame header in `framing.rs`;
- LSB mapping in `codec.rs`;
- TIFF/DNG skeleton in `docs/port-plan.md` §Phase 2 and the Python
  `container.py`/`stego.py`;
- tag inventory validated with `tiffdump`/`exiv2`/`rawpy`.

When the Rust writer intentionally differs (for example, a different JPEG
encoder), record it in an ADR and in the porting matrix.
