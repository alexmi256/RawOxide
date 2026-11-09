# Rust port plan

**Status:** active; Phase 0 (foundations) and Phase 1 (DNG read/decode) complete
**Audience:** implementers and reviewers
**Reference:** [feasibility study](../../RawHide/native-port-feasibility.md),
Python [README](../../RawHide/README.md) and [AGENTS.md](../../RawHide/AGENTS.md)

---

## 1. Goal and non-goals

### Goal

A self-contained Rust CLI that reproduces RawHide's behaviour with:

- one static binary per platform, no Python/NumPy/Pillow/tifffile at runtime;
- **payload compatibility**: Rust decodes Python output and Python decodes Rust
  output;
- **outcome parity**: same encode/decode capabilities, metadata information,
  profiles, split sets, GPS, debug sidecar, DNG-reader acceptance and sensible
  CLI/error behaviour;
- lower and bounded memory use at multi-gigabyte scale (streaming/banded);
- a clean Rust design. Python is the reference for behaviour, not a structural
  blueprint.

### Implementation freedom

The port may differ from Python in internal structure and still be correct:
IFD entry order, dummy-tag patching, strip/tile choice, EXIF/GPS construction,
tag inventory quirks, JPEG encoder, Lanczos implementation, RNG draw order,
profile storage format, metadata pool order, and error-message wording. We only
preserve what affects the outcome: payload wire format, DNG acceptance, the
information carried in tags, split-set interoperability, and user-facing
capabilities. Tests assert behaviour and cross-decode, not byte/IFD equality.

### Non-goals

- Byte-identical output with Python for the same seed. See
  [ADR 0001](adr/0001-format-compatibility.md).
- Reimplementing every Python helper as a public Rust API. The Rust API is
  designed for the CLI and embedding, not as a line-by-line mirror.
- Lossy compression or JPEG XL raw storage; the Python tool deliberately
  excludes them.
- New formats (ORF/ARW/…) in v1. `rawhide-format-expansion.md` is a separate
  v2 track.

---

## 2. Compatibility contract

| level | requirement | decision |
|---|---|---|
| **L1 wire format** | Either implementation decodes the other's files; split sets, keys, planes, depths and modes interoperate | **Required** |
| **L2 seed parity** | Same seed produces byte-identical DNGs across languages | **Not required** |
| **L3 feature parity** | All documented CLI flags, profiles, sidecar fields and messages | Required, staged |

L1 needs only the framing, LSB mapping, TIFF reader and tag values. It does not
need NumPy/PCG64/ziggurat or Pillow/libjpeg parity. L2 would require reproducing
NumPy `Generator` draws, Python MT19937 `_randbelow` semantics, Pillow Lanczos
and libjpeg-turbo output; NumPy itself does not guarantee `Generator` streams
across versions. The port defines its own deterministic seeds and golden
vectors instead.

Acceptance gate for L1 (run in CI once the TIFF layer lands):

1. Python encode → Rust decode → payload identical.
2. Rust encode → Python decode → payload identical.
3. `tiffdump`/`exiv2`/`rawpy` accept the Rust output and see the expected tags.
4. Rust encode twice with the same seed → identical bytes (offset tags
   excluded).

---

## 3. Current state (Phase 0 + Phase 1)

Implemented:

- Cargo workspace (`rawhide-core`, `rawhide-cli`), MIT license, MSRV 1.88,
  edition 2024.
- `PayloadFrame`: magic/version/flags/length/CRC32, optional SHA-256 counter
  keystream, golden vectors from Python.
- `LsbCodec`: Python plane/sample mapping for `u8`/`u16`, capacity helpers,
  KiB formatting (`format_kb{,_hi,_lo}`).
- `rawhide capacity` with Python-compatible output.
- `DngReader`/`RawFrame`: raw SubIFD discovery, tiled/stripped reads, packed
  10/12/14-bit samples, uncompressed/Deflate blocks, predictor 2, multiple
  frames, and split markers.
- `decode_files` + `rawhide decode`: auto-detect 1-16 planes, strict explicit
  planes, `--key`, `--max-bytes`, split metadata/filename ordering, partial
  chunk warnings. All Python fixtures decode byte-identically.
- Tests: unit, property (`proptest`), CLI (`assert_cmd`), fixture integration,
  criterion benchmark.
- Tooling: rustfmt, clippy (`-D warnings`), nextest, llvm-cov, insta, cargo-deny,
  cargo-audit, `just`, GitHub Actions, coverage config, fuzzing/mutation plans.
- Gitignore from the GitHub Rust template plus coverage/editor entries.

Not implemented yet: DNG writing, profiles, metadata randomization, cover,
thumbnails, split encoding, sidecar, and the `encode`/`gen-tiff` CLI paths. Those
return a clear "not implemented" error.

---

## 4. Architecture

```text
rawhide-cli
  ├── clap parsing, progress, user-facing messages, exit codes
  └── rawhide-core
        ├── framing       DNGS header + keystream
        ├── codec         LSB mapping, capacity, KiB helpers
        ├── tiff/         reader + writer + tag/IFD builders (done)
        ├── profile       embedded camera profiles + level math (done)
        ├── metadata      seeded EXIF randomization (done)
        ├── geo           tz city/land sampler (done)
        ├── cover         synthetic raw covers (in progress)
        ├── thumbnail     synthetic/Commons/offline-noise/file preview
        ├── sizing        auto-fit search (done)
        ├── split         chunk naming/sequencing (done)
        ├── digests       DNG SDK digests (done)
        ├── opcodes       GainMap/OpcodeList (in progress)
        ├── makernote     opaque HDRP envelopes (done)
        └── debug         JSON sidecar (planned)
```

`rawhide-core` has no CLI, network by default, or global state. The CLI owns
I/O policy, progress and error presentation. See [architecture.md](architecture.md)
for data flow and memory/concurrency rules.

---

## 5. Workstreams and milestones

### Phase 0 — Foundations (done)

Workspace, license, tooling, framing, LSB codec, capacity, tests, CI.

**Acceptance:** `just ci` green; golden framing/format vectors match Python.

### Phase 1 — Read DNG (done)

- `tiff-reader` 0.11 parses IFD0/SubIFDs, tag values, tile/strip tables and
  EXIF pointers (BigTIFF included).
- **Implementation note:** `tiff-reader` 0.11 rejects DNG `LinearRaw`/`CFA` in
  its color-model validation and does not decode classic TIFF type 13 (`IFD`),
  which is how `SubIFDs` is stored. We therefore use it for IFD/tag parsing and
  read raw blocks ourselves in `dng.rs`: tile/strip offsets, uncompressed and
  Deflate blocks, predictor 2, packed 10/12/14-bit samples, multiple frames.
  This avoids vendoring the crate; revisit if upstream supports DNG
  photometrics.
- `DngReader`: `raw_frames`, `split_markers`, `exif_ifd`.
- Decode path: header first, auto-detect planes 1–16, enforce `--max-bytes`,
  then extract/unpack; split metadata or filename ordering; partial-chunk
  warning.
- Support uncompressed and Deflate raws; classic and BigTIFF.

**Acceptance met:** all committed Python fixtures decode byte-identically
(linear/cfa, 8/10/14/16-bit, keyed, three frames, Deflate, split sets,
reversed chunk order). Malformed files and wrong keys return errors, never
panic. Coverage is ~81% lines overall.

Known Phase 1 limitation: `tiff-reader` parses an EXIF IFD pointer (tag 34665)
as a 4-byte LONG, while the Python writer stores an 8-byte offset field in
BigTIFF. Split markers in a BigTIFF whose EXIF IFD sits past 4 GiB may therefore
be misread. Raw decode is unaffected; revisit when adding full metadata support
in Phase 3.

### Phase 2 — Write DNG (core writer done; streaming/compression next)

**Result:** a custom classic-TIFF/`BigTIFF` writer in `core::writer` emits tiled raw pages
(`LinearRaw` or `CFA`) at 8/10/12/14/16-bit, a baseline JPEG preview, multiple
full-resolution raw frames (1-8), split markers in IFD0/EXIF and an embedded
payload. Python `stego_dng.decode` recovers the payload for every mode/depth
combination, for multi-frame files and for split chunk sets; `tifffile` sees
`LinearRaw`/`CFA`; `exiv2`/`tiffdump` show the SubIFD/EXIF inventory and
`rawpy.postprocess()` succeeds. This is pinned by `writer_spike`/`cli` tests and
manual Python/rawpy checks.

The writer uses a small custom IFD/tile writer rather than `tiff-writer` because
0.11 does not express `SubIFDs`, `LinearRaw`/`CFA` or packed depths; the custom
writer already produces accepted files and keeps full control over split/EXIF
markers.

Completed in this phase:

- Multiple raw frames (burst/stack style) with per-frame covers and payload
  striping.
- Split encoding: DCF chunk names, optional album directory, shared
  `ImageUniqueID`/`ImageDescription` UUID, `ImageNumber`/`PageNumber`/
  `ImageDescription` sequence markers, `PageNumber` totals.
- EXIF IFD linking (`--split-file-metadata-id`/`-seq`) with the Python field
  names and fail-closed defaults.

Remaining full-writer work:

- Digest-enabled frames still materialize one full frame; other single frames are banded.
- Adobe tag conventions.
- Commons thumbnail fetching; file/synthetic/offline-random previews are done.

**Acceptance:** Python decodes Rust output across the L1 matrix (including
multi-frame and split sets); `rawpy` accepts it; `tiffdump` shows the expected
skeleton; peak RSS < 64 MiB for the 256×192/2048×1536 cases.

### Phase 3 — Profiles and metadata (in progress)

Done:

- Embedded all 307 `profile.json` files and the three Adobe blobs
  (`assets/profiles/`, generated table in `profiles_generated.rs`); lazy
  registry with Python-style fallbacks, corrupt-entry skipping and longest-slug
  subprofile resolution.
- Typed `CameraProfile` + `FidelityConfig` (metadata/raw fields), depth helpers,
  black/white level scaling and strict/capacity policies.
- `AutoSizer` parity and CLI `--auto-size`/auto-planes.
- Seeded metadata randomizer with per-profile pools, fidelity ranges, serial
  shapes and `--image-time`; writer emits the EXIF camera/lens/exposure block,
  `AsShotNeutral`, `BaselineExposure` and `NoiseProfile`. DNG profile tags are
  in IFD0, emitted EXIF tags respect `emit_lens`/`emit_serials`/`exif_allow`, and
  `--level-policy` drives white/black levels and synthetic-cover range, plus
  CFA pattern, crop and bayer tags.

- `core::geo`: embedded `zones.bin`, land-mask sampling with radius shrinking,
  elevation noise and zone-offset filtering; GPS IFD emission for `emit_gps`
  profiles with local offsets and UTC timestamps.

- `core::digests`: DNG SDK tiled `NewRawImageDigest` and `RawDataUniqueID`,
  emitted for single-frame `raw_digests` profiles; verified with
  `tools/digests.py`.

- MakerNote envelopes: `fidelity.makernote` parsing, seeded block generation,
  EXIF tag 37500 and `MakerNoteSafety`.
- Profile `OpcodeList3` emission (tag 51022) alongside GainMap `OpcodeList2`
  (tag 51009), with both included in `RawDataUniqueID` fingerprints.
- GainMap `OpcodeList2`: profile grid parsing, four 30x40 phase maps, big-endian
  opcode encoding, SDK fingerprint stream and writer tag 51009; `RawDataUniqueID`
  includes the fingerprint.

Remaining:

- Other opcode ids and full Adobe IFD entry rewrite/compaction (profile tag
  blobs are implemented; debug sidecar fields/inventory are done).

**Acceptance:** profile list/resolve matches Python; sidecar snapshots match the
documented schema; digests reproduce the Python values for fixed raw/crop/opcode
inputs; GainMap round-trips through Python.

### Phase 4 — Metadata, GPS and time (2–2.5 weeks)

- Deterministic native RNG (explicitly seeded per stream/band).
- Lens/ISO/exposure/serial pools, fidelity ranges, software/orientation pools.
- `zones.bin` parsing, land sampling, elevation, offsets; tzdata via
  `chrono-tz` (pin the embedded version).
- `--image-time`: random, input birth time (`statx`/`created()` + ctime
  fallback), input mtime, now; UTC/offset consistency.
- Fix the Python `date_bounds()`/`now()` reproducibility bug in the oracle
  before pinning metadata fixtures.

**Acceptance:** seeded metadata is stable; GPS fixes are on land near a tz
principal location; pinned times produce matching EXIF/GPS values.

### Phase 5 — CLI parity and thumbnails (2–3 weeks)

- All encode/decode/capacity/gen-tiff flags, split album layout, metadata field
  selection, `--auto-size`, `--level-policy`, `--debug-metadata-output`,
  `--progress/--no-progress`, `--gps-radius`.
- Thumbnail sources: synthetic, user file, Wikimedia Commons with bounded
  retries and offline fallback; image decode, center crop, Lanczos3, RGB→YCbCr.
- Exact KiB formatting and `auto-config`/`warning:`/`note:`/`error:` lines.

**Acceptance:** CLI snapshot tests match documented output; no network in tests
(inject an opener); all Python error messages have a Rust equivalent.

### Phase 6 — Hardening, performance, release (2–3 weeks)

- Streaming decode (only the tiles covering the needed prefix); parallel tile
  compression/generation with deterministic per-band seeds.
- Fuzzing (`cargo-fuzz`: framing, IFD parser, decode); Miri for core;
  mutation testing on critical modules.
- Benchmarks vs Python; memory ceilings; `cargo-dist` release artifacts; docs.

**Acceptance:** fuzz targets run clean for a nightly budget; benchmarks meet
the §0 targets; Linux/macOS/Windows artifacts build in CI.

Total: **~8–12 dev-weeks** with `tiff-writer`/`tiff-reader`; add 2–3 weeks if
the writer has to be built from scratch or `rawler` is adopted.

---

## 6. Technical decisions

### TIFF/DNG stack

- **Write:** `tiff-writer` 0.11 (MIT/Apache): tiles, strips, BigTIFF, Deflate,
  LZW, JPEG 4:2:0, streaming, arbitrary tags. Patch the three DNG gaps
  (SubIFDs, LinearRaw/CFA, packed 10/12/14-bit). Fallback: `tiff`/`tiff-forge`
  plus a custom writer. `rawler`/`dnglab` (LGPL-2.1) is reference-only unless
  the project license changes.
- **Read:** `tiff-reader` 0.11 (same suite): BigTIFF, tiles, SubIFDs,
  bit-packed 1–15 bits, Deflate/LZW/JPEG, parse budgets. Fallback: `tiff`.
- **Preview:** one baseline JPEG strip, YCbCr 4:2:0, quality 75; no
  lossless-JPEG raw encoder needed (raw is uncompressed/Deflate only).

### RNG and determinism

- No NumPy/Python parity. Use an explicit, documented native stream (e.g.
  `rand_chacha::ChaCha8Rng` or `rand_pcg::Pcg64`) with per-stream seeds:
  cover, metadata, thumbnail, location, split.
- Per-band/per-tile seeds so parallel work is byte-stable.
- Golden vectors are generated by the Rust implementation and pinned; Python
  fixtures test *decode* compatibility, not seed parity.
- No `HashMap` iteration in write paths; use `BTreeMap`/sorted keys.

### Streaming and memory

- Cover generation, embedding and tile writing share one band/tile buffer.
- Decode reads only the tiles covering the required bit prefix; a small payload
  in a large DNG must not allocate the raster.
- Payload framing is streamed (CRC/length known before or in a first pass).
- Target peak RSS: < 64 MiB for the 588 MB native case; < 100 MiB for 2.5 GB
  pixel-shift (excluding OS page cache).

### Profiles/assets

- Embed `profile.json` + Adobe blobs + `zones.bin` with `include_dir` or
  `rust-embed`; no runtime files.
- `serde` structs with `deny_unknown_fields`? The Python loader tolerates extra
  keys; match that by ignoring unknown fields, but validate enum/range values.
- Keep the registry tolerant: one bad profile must not break the rest.

### CLI and errors

- `clap` derive; `anyhow` only at `main`.
- `thiserror` in the library; distinguish "no magic / try next plane" from
  "capacity limit" as Python does.
- All user-facing sizes in KiB via `format_kb{,_hi,_lo}`.
- `tracing` to stderr; progress via `indicatif` (TTY-only unless forced).

---

## 7. Testing strategy (summary)

See [testing.md](testing.md). The short version:

- unit tests next to code;
- property tests for framing/codec/parsers;
- snapshot tests (`insta`) for JSON sidecars and CLI output;
- committed Python-generated DNG fixtures for cross-decode;
- consumer checks with `tiffdump`, `exiv2`, `rawpy`;
- `cargo llvm-cov` coverage (patch target 80%);
- fuzzing for untrusted parsers;
- criterion benchmarks for codec/tile paths.

---

## 8. Risks

| risk | impact | mitigation |
|---|---|---|
| `tiff-writer` 0.11 is young (0.11, Nov 2026) | API churn/bugs | pin exact version; keep a thin adapter; fallback to `tiff`/custom; vendor patches if needed |
| SubIFD/packed-depth patches are deeper than expected | schedule | spike first; keep `tiff-forge` as an IFD assembler fallback |
| JPEG preview differences | visual/fingerprint parity | preserve YCbCr 4:2:0 + tags; don't compare JPEG bytes; solid-red regression test |
| Python oracle nondeterminism (`date_bounds` + `now`) | flaky fixtures | fix/pin Python before metadata fixtures; use fixed `--image-time` where needed |
| Huge-file memory | OOM | banded/streamed design; RSS regression tests |
| tzdata version drift | GPS offsets differ | embed and pin tzdata; document update policy |
| License change if `rawler` adopted | project-wide | keep MIT + `tiff-writer` by default; separate ADR if changed |

---

## 9. Open questions

1. Do we keep MIT, or relicense the port to AGPL/LGPL to permit `rawler`? Default:
   MIT + `tiff-writer`; revisit only if the writer spike fails.
2. Target platforms for v1: Linux x86_64 + Windows + macOS (arm64/x86_64)?
3. Do we ship a Python compatibility shim (PyO3) or keep the Python CLI as the
   reference indefinitely?
4. `--thumbnail random` network policy: Commons only, or support a configurable
   URL? Match Python first.
5. Progress bar output on non-TTY and `--progress` semantics: match Python's
   tri-state exactly.

---

## 10. References

- [Feasibility study](../../RawHide/native-port-feasibility.md)
- [Python README](../../RawHide/README.md), [AGENTS.md](../../RawHide/AGENTS.md)
- DNG 1.6 / TIFF 6.0 specifications
- `tiff-writer`/`tiff-reader` 0.11 docs: <https://docs.rs/tiff-writer>,
  <https://docs.rs/tiff-reader>
- ADRs: [0001](adr/0001-format-compatibility.md),
  [0002](adr/0002-rng-policy.md), [0003](adr/0003-tiff-stack.md),
  [0004](adr/0004-license-policy.md)
