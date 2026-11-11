# Rust port plan

**Status:** active; Phases 0–5 are implemented, with Phase 6 hardening partly done
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

## 3. Current state (Phases 0–5)

Implemented:

- Cargo workspace (`rawhide-core`, `rawhide-cli`), MIT license, MSRV 1.99.1,
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
- Writer (`core::writer`, `rawhide encode`): classic TIFF and BigTIFF, tiled
  `LinearRaw`/`CFA` at 8/10/12/14/16-bit, JPEG preview, 1-8 raw frames,
  profile tile geometry, crop `ActiveArea`/`DefaultUserCrop`, split markers,
  embedded streaming payload and seeded EXIF/DNG metadata.
- All 307 Python camera profiles plus Adobe assets embedded; metadata pools,
  GPS, raw digests, MakerNote, GainMap `OpcodeList2`, profile `OpcodeList3` and
  Adobe tag conventions are emitted.
- Split encoding: album layout for multi- and single-chunk sets, DCF names,
  shared UUID, `ImageNumber`/`PageNumber`/`ImageDescription` sequencing,
  fresh per-chunk seeds for unseeded runs, ASCII split-id validation and a
  9999-chunk guard.
- Debug sidecar (`--debug-metadata-output`): Python-shaped summary, encode
  options, payload/cover/thumbnail records, full profile snapshot, randomized
  metadata, resolved thumbnail label/CRC, per-frame CRCs, preview JPEG hash and
  named IFD tag inventory.
- Tests: unit, property (`proptest`), CLI (`assert_cmd`), fixture integration,
  criterion benchmark, and an automated Python ↔ Rust cross-decode script
  (`just cross-decode`).
- Tooling: rustfmt, clippy (`-D warnings`), nextest, llvm-cov, cargo-deny,
  cargo-audit, `just`, GitHub Actions, coverage config, fuzzing/mutation plans.

Remaining compatibility/tooling gaps are tracked in
[porting-matrix.md](porting-matrix.md); Rust-produced BigTIFF pointers are
covered by the sidecar inventory test, and forced-BigTIFF split decoding is
covered separately.

---

## 4. Architecture

```text
rawhide-cli
  ├── clap parsing, progress, user-facing messages, exit codes
  ├── sidecar       Python-shaped JSON debug document + tag inventory
  └── rawhide-core
        ├── framing       DNGS header + keystream
        ├── codec         LSB mapping, capacity, KiB helpers
        ├── tiff/         reader + writer + tag/IFD builders
        ├── profile       embedded camera profiles + level math
        ├── metadata      seeded EXIF randomization, per-run date anchor
        ├── geo           tz city/land sampler
        ├── cover         synthetic raw covers
        ├── thumbnail     synthetic/Commons/offline-noise/file preview + label
        ├── sizing        auto-fit search
        ├── split         chunk naming/sequencing + id/chunk-count validation
        ├── digests       DNG SDK digests
        ├── opcodes       GainMap/OpcodeList2; other ids out of scope (same as Python)
        ├── makernote     opaque HDRP envelopes
        └── writer        tiled DNG output, split markers, sidecar statistics
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
panic. Coverage is ~88% lines overall.

Known scope boundary: `tiff-reader` parses an EXIF IFD pointer (tag 34665) as a
4-byte LONG, while Python's BigTIFF writer stores an 8-byte offset field in a
LONG entry. The Rust writer uses `LONG8`/`IFD8` pointers, which the Rust reader
handles; EXIF/GPS pointer inventory is covered by the sidecar BigTIFF test, and
split-marker decode by a forced-BigTIFF round trip. Python BigTIFF files
whose EXIF IFD lies past 4 GiB are outside the current decode scope; raw payload
decode is unaffected.

### Phase 2 — Write DNG (core writer done; streaming/compression done)

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
- Banded single-frame writes and incremental 256-row digest computation.
- Streaming payload I/O from a file range in 64 KiB chunks, keeping encode memory
  independent of payload size; decode releases raw frames and reuses the
  extracted buffer for the payload.
- Adobe profile tags, Commons thumbnails and the debug sidecar inventory.

Writer parity note:

- GainMap is the only opcode id the Python reference fingerprints, and the Rust
  writer matches that; other opcode ids are not a parity gap.

**Acceptance:** Python decodes Rust output across the L1 matrix (including
multi-frame and split sets); `rawpy` accepts it; `tiffdump` shows the expected
skeleton; peak RSS < 64 MiB for the 256×192/2048×1536 cases.

### Phase 3 — Profiles and metadata (done)

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

**Acceptance:** profile list/resolve matches Python; sidecar snapshots match the
documented schema; digests reproduce the Python values for fixed raw/crop/opcode
inputs; GainMap round-trips through Python.

### Phase 4 — Metadata, GPS and time (done)

- Deterministic native RNG (explicitly seeded per stream/band).
- Lens/ISO/exposure/serial pools, fidelity ranges, software/orientation pools.
- `zones.bin` parsing, land sampling, elevation, offsets; tzdata via
  `chrono-tz`.
- `--image-time`: random, input birth time (`created()` + Unix `ctime`
  fallback), input mtime, now, plus explicit wall-clock/RFC 3339 values; UTC/
  offset consistency.
- Fidelity rolling date windows use one anchor per encode run so split chunks
  share one window and wall-clock movement cannot shift seeded metadata.

**Acceptance met:** seeded metadata is stable; GPS fixes are on land near a tz
principal location; pinned times produce matching EXIF/GPS values.

### Phase 5 — CLI parity and thumbnails (done)

- All encode/decode/capacity/gen-tiff flags, split album layout (including the
  positive `--split-file-album` form), metadata field selection, `--auto-size`,
  `--level-policy`, `--debug-metadata-output`, `--progress/--no-progress`,
  `--gps-radius`.
- Thumbnail sources: synthetic, user file, Wikimedia Commons with bounded
  retries and offline fallback; image decode, center crop, Lanczos3, RGB→YCbCr,
  resolved source labels.
- KiB formatting plus the `auto-config`/`warning:`/`note:`/`error:` lines,
  including `thumbnail:`, `debug:`, `profile:` and `meta:` output.

Remaining differences: the Rust CLI defaults `--thumbnail` to `synthetic`
(Python defaults to `random`). Progress bars are implemented but use a simpler
single-bar presentation than Python's nested tqdm bars.

### Phase 6 — Hardening, performance, release (partly done)

- Streaming encode/decode, banded tiles and incremental digests are done.
- Automated Python ↔ Rust cross-decode runs in CI (`scripts/cross_decode.sh`).
- MSRV CI and a tag-triggered release workflow with checksums are implemented.
- Still open: parallel tile generation, `cargo-fuzz` targets/Miri and benchmark
  comparison against Python.

**Acceptance:** fuzz targets run clean for a nightly budget; benchmarks meet
the §0 targets; Linux/macOS/Windows artifacts build in CI.

Total estimate: **~8–12 dev-weeks**. The implementation used a custom writer
rather than `tiff-writer` (see ADR 0003); this history is kept for reference.

---

## 6. Technical decisions

### TIFF/DNG stack

- **Write:** a custom classic-TIFF/BigTIFF writer in `core::writer`, not
  `tiff-writer`. `tiff-writer` 0.11 was evaluated and rejected for this output
  because it cannot express `SubIFDs`, `LinearRaw`/`CFA` or packed
  10/12/14-bit samples; see ADR 0003. `rawler`/`dnglab` (LGPL-2.1) remains
  reference-only under the MIT license.
- **Read:** `tiff-reader` 0.11 for IFD/tag and tile/strip parsing, with a
  custom raw-block reader in `dng.rs`. Raw blocks currently cover uncompressed
  and Deflate data; the crate supports more compressions but the DNG raw path
  does not use them.
- **Preview:** one baseline JPEG strip, YCbCr 4:2:0, quality 75; no
  lossless-JPEG raw encoder needed (raw is uncompressed/Deflate only).

### RNG and determinism

- No NumPy/Python seed parity. The implementation uses `Rng64` (`SplitMix64`)
  instances: the cover seed also drives the preview fallback; the metadata seed
  initializes separate location and Makernote instances. For split sets one
  chunk seed drives cover and metadata, while a separate location seed can be
  shared across the set. The chunk-set UUID uses `uuid` v4 unless overridden.
- Per-frame and per-chunk seeds are deterministic when a seed is supplied;
  unseeded split chunks and unseeded split GPS draws use fresh entropy.
- Golden vectors are generated by the Rust implementation and pinned; Python
  fixtures test *decode* compatibility, not seed parity.
- No `HashMap` iteration in write paths; use `BTreeMap`/sorted keys.
- Parallel tile generation is not implemented yet.

### Streaming and memory

- Cover generation, embedding and tile writing share one band/tile buffer.
- Encode reads payload file ranges in 64 KiB chunks; in-memory callers borrow
  the payload slice instead of copying it into a framed buffer.
- Decode streams tile-row/strip bands from `RawBandReader` into a payload sink
  and stops once the declared payload is complete; the full raster is never
  resident. `decode_files` keeps the `Vec` API, while
  `decode_files_to_writer` and the CLI write payload bytes directly (CLI to a
  temp file, renamed on success), so CLI decode memory is near constant. Strip
  rows larger than a payload still decode one strip at a time rather than the
  whole frame.
- Target peak RSS: < 64 MiB for encode at multi-gigabyte payload sizes; CLI
  decode is near constant (band + sink buffers), while the in-memory
  `decode_files` API remains one payload-sized allocation.

### Profiles/assets

- Embed `profile.json` + Adobe blobs + `zones.bin` through the generated
  `profiles_generated.rs` `include_str!`/`include_bytes!` table; no runtime
  files.
- Parse profile JSON tolerantly: unknown keys are ignored, enum/range values
  are validated, and one corrupt profile must not break the rest.

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
- CLI/JSON assertions (`assert_cmd` + `serde_json`) for sidecars and output;
- committed Python-generated DNG fixtures for cross-decode;
- bidirectional Python ↔ Rust encode/decode checks via `just cross-decode`;
- written tag-inventory comparisons for reference profiles in the cross-decode gate;
- manual consumer checks with `tiffdump`, `exiv2`, `rawpy`;
- `cargo llvm-cov` coverage (patch target 80%);
- fuzzing for untrusted parsers (planned, no targets committed yet);
- criterion benchmarks for codec/tile paths.

---

## 8. Risks

| risk | impact | mitigation |
|---|---|---|
| Python BigTIFF EXIF pointer layout is unsupported | split markers in Python files >4 GiB | Rust writer uses LONG8 and is tested; Python files are out of scope |
| External DNG compression/layout support is narrower than Python | cannot decode some third-party DNGs | keep decode scope documented; extend `dng.rs` only if required |
| JPEG preview differences | visual/fingerprint parity | preserve YCbCr 4:2:0 + tags; don't compare JPEG bytes; solid-red regression test |
| Python oracle nondeterminism (`date_bounds` + `now`) | flaky metadata fixtures | Rust uses a per-run anchor; pin image time in fixtures |
| Huge-file memory | OOM | banded/streamed design; RSS regression tests |
| tzdata version drift | GPS offsets differ | embed and pin tzdata; document update policy |
| Python CLI/sidecar drift | stale parity claims | cross-decode CI + sidecar key/CRC checks |

---

## 9. Decisions and open questions

Resolved:

1. Keep MIT; no `rawler` adoption (ADR 0004).
2. CI covers Linux, Windows and macOS, with an MSRV check; a tag-triggered
   release workflow uploads release binaries.
3. No PyO3 shim; the Python CLI remains the specification/oracle.
4. Preview fetching is Commons-only, matching Python.
5. `--progress` follows the tri-state TTY/force/disable behaviour and reports
   row/byte percentages; a single bar is used per encode/decode file rather than
   Python's nested tqdm bars.

Still open:

1. Whether decoding arbitrary third-party DNGs (broader compression/layout
   support) belongs in v1 or a later reader track.

---

## 10. References

- [Feasibility study](../../RawHide/native-port-feasibility.md)
- [Python README](../../RawHide/README.md), [AGENTS.md](../../RawHide/AGENTS.md)
- DNG 1.6 / TIFF 6.0 specifications
- `tiff-reader` 0.11 docs: <https://docs.rs/tiff-reader>
- ADRs: [0001](adr/0001-format-compatibility.md),
  [0002](adr/0002-rng-policy.md), [0003](adr/0003-tiff-stack.md),
  [0004](adr/0004-license-policy.md)
