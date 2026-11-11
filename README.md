# RawOxide

[![CI](https://github.com/alexmi256/RawOxide/actions/workflows/ci.yml/badge.svg)](https://github.com/alexmi256/RawOxide/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust 1.99.1+](https://img.shields.io/badge/rust-1.99.1%2B-orange)

RawOxide hides arbitrary files inside DNG raw sensor data. It writes a valid,
camera-realistic DNG whose least-significant bits carry a framed payload, then
recovers that payload later — with no sidecar, no extra metadata field and no
obvious change to how the image looks.

RawOxide is a native Rust implementation of the [RawHide](https://github.com/alexmi256/RawHide)
concept. It shares the payload framing and DNG conventions of that project, so
the two can exchange files, but it runs as a single self-contained binary with
no Python, NumPy, Pillow or `tifffile` at runtime.

> **Status:** `0.1.x`, usable end to end. Encode and decode are implemented for
> every documented mode. Interfaces and on-disk defaults may still change
> before `1.0`.

## TL;DR

| I want to… | Command |
|---|---|
| Check how much a frame can hold | `rawoxide capacity --width 2048 --height 1536` |
| Hide a file in a DNG | `rawoxide encode -i secret.bin -o photo.dng` |
| Recover it | `rawoxide decode -i photo.dng -o secret.bin` |
| Hide a large file across several DNGs | `rawoxide encode -i big.bin -o out/vol.dng --split-file 100M` |
| Recover a split set | `rawoxide decode -i out/big/vol0001.dng out/big/vol0002.dng -o big.bin` |
| Use a passphrase | `rawoxide encode -i secret.bin -o photo.dng --key "correct horse"` |
| Make the smallest possible file | `rawoxide encode -i secret.bin -o photo.dng --auto-size` |
| Make it look like a specific camera | `rawoxide encode -i secret.bin -o photo.dng --camera-profile canon_eos_80d` |
| Reproduce a previous run exactly | `rawoxide encode -i secret.bin -o photo.dng --seed 42 --image-time "2025:06:01 12:00:00"` |
| Inspect what was written | `rawoxide encode -i secret.bin -o photo.dng --debug-metadata-output` |

## What RawOxide does

- **Any payload type.** The input is treated as opaque bytes; a JPEG, ZIP,
  database export or another DNG are equally fine.
- **Valid DNG output.** Files carry a raw `SubIFD` (`LinearRaw` or `CFA`), a JPEG
  preview, EXIF, optional GPS, calibration tags, noise profile and optional
  maker note. Mainstream raw readers open the result.
- **Camera-realistic metadata.** 307 embedded camera profiles supply make/model,
  lens and exposure pools, serial formats, black/white levels, calibration
  matrices, crop geometry, GPS behaviour and profile-specific tag conventions.
- **Bounded memory.** Encode streams the payload in 64 KiB chunks and writes
  banded tiles; decode streams raw bands into the output file. Multi-gigabyte
  payloads do not require the payload to be resident in memory.
- **Deterministic when seeded.** Cover pixels, metadata and GPS draws come from
  explicit named RNG streams. The same seed and pinned capture time reproduce a
  run; the effective seeds are recorded in the debug sidecar.
- **Split sets.** A payload can be spread over DCF-style numbered DNG chunks with
  a shared set ID and sequence markers, so a directory of files behaves as one
  payload.
- **Classic TIFF or BigTIFF.** Output switches automatically when classic offsets
  would overflow, or when you ask for it.
- **No unsafe code.** `unsafe` is forbidden in the core library, and malformed
  input is expected to return an error rather than panic.

## Requirements and installation

### Prebuilt binaries

When a release is published, platform binaries and `SHA256SUMS` are attached to
the [GitHub release](https://github.com/alexmi256/RawOxide/releases):

| platform | asset |
|---|---|
| Linux x86-64 | `rawoxide-linux-x86_64` |
| Linux ARM64 (aarch64) | `rawoxide-linux-aarch64` |
| Linux ARMv7 (32-bit, hard-float) | `rawoxide-linux-armv7` |
| macOS Apple Silicon (ARM64) | `rawoxide-macos-aarch64` |
| macOS Intel (x86-64) | `rawoxide-macos-x86_64` |
| Windows x86-64 | `rawoxide-windows-x86_64.exe` |
| Windows ARM64 | `rawoxide-windows-aarch64.exe` |

```bash
chmod +x rawoxide-linux-x86_64
./rawoxide-linux-x86_64 --help
```

### From source

Building requires **Rust 1.99.1 or newer**; the workspace uses edition 2024.
Install it with [rustup](https://rustup.rs) if you do not already have it.

```bash
git clone https://github.com/alexmi256/RawOxide.git
cd RawOxide

# Build an optimised binary at target/release/rawoxide
cargo build --release -p rawoxide-cli

# Or install it into ~/.cargo/bin
cargo install --path crates/rawoxide-cli
```

To cross-compile an ARM Linux binary, install the matching GNU cross toolchain
and target libc headers (for example `gcc-aarch64-linux-gnu` with
`libc6-dev-arm64-cross`, or `gcc-arm-linux-gnueabihf` with
`libc6-dev-armhf-cross`, on Debian/Ubuntu) and run:

```bash
just build-arm64  # aarch64, 64-bit
just build-arm32  # armv7 hard-float, 32-bit
```

The binaries are written to `target/aarch64-unknown-linux-gnu/release/rawoxide`
and `target/armv7-unknown-linux-gnueabihf/release/rawoxide`.

`just` is optional but convenient; `just --list` shows the recipes.

## Quick start

Generate a payload and hide it:

```bash
rawoxide encode -i secret.bin -o photo.dng
```

Typical output:

```text
auto-config (mode, bit_depth, width, height, lsb_planes): 2048x1536 linear 16-bit, 1 LSB plane(s), capacity 1,152.0 KB for 2.9 KB input
wrote photo.dng (18,718.8 KB)
payload 2.9 KB / capacity 1,152.0 KB
thumbnail: photo.dng: synthetic-gradient
profile: gfx_100 (Fujifilm GFX 100)
meta: 2021:09:30 15:31:40 ISO125 GF110mmF2 R LM WR S/N 92A90F9C
```

The thumbnail, profile and metadata lines reflect the resolved run; the drawn
values change with the seed unless `--seed` is supplied.

Recover it:

```bash
rawoxide decode -i photo.dng -o recovered.bin
# recovered 2.9 KB -> recovered.bin
```

Check that the bytes match:

```bash
cmp secret.bin recovered.bin && echo "round trip ok"
```

You can also open the file with an ordinary DNG reader to confirm it is a
well-formed raw image.

## Building and testing

| task | command |
|---|---|
| check | `cargo check --workspace --all-targets` |
| build (debug) | `cargo build -p rawoxide-cli` |
| build (release) | `cargo build --release -p rawoxide-cli` |
| format | `cargo fmt --all` |
| lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| tests | `cargo nextest run --workspace --all-features` or `cargo test --workspace` |
| doc tests | `cargo test --workspace --doc` |
| docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` |
| coverage | `cargo llvm-cov nextest --workspace --all-features --lcov --output-path lcov.info` |

The workspace declares stable Rust plus `rustfmt`, `clippy` and
`llvm-tools-preview` in `rust-toolchain.toml`. CI runs the same checks on
Linux, macOS and Windows, together with 32-bit and 64-bit ARM cross-compilation
checks, native Intel macOS and Windows ARM64 builds, an MSRV build,
`cargo-deny`, `cargo-audit` and rustdoc.

The committed fixtures under `fixtures/` make the tests hermetic: no network,
no external tools and no reference implementation are needed to run the suite.

## CLI overview

```text
rawoxide <COMMAND>

Commands:
  capacity  Print the sample count and payload capacity for a geometry
  encode    Embed a payload into a new DNG
  decode    Extract a payload from one or more DNG files
  gen-tiff  Generate a plain TIFF cover
```

Run `rawoxide <command> --help` for the authoritative list. Global `-v`/`-vv`
increase log verbosity; logs go to stderr.

### `capacity`

Reports the raw sample count and the payload capacity of a geometry.

| option | default | meaning |
|---|---|---|
| `--width` | `2048` | raw frame width in pixels |
| `--height` | `1536` | raw frame height in pixels |
| `--mode` | `linear` | `linear` (3 samples/pixel) or `cfa` (1 sample/pixel) |
| `--lsb-planes` | `1` | bits used per sample, 1–16 |
| `--raw-frames` | `1` | full-resolution frames sharing the file, 1–8 |

Capacity is `samples × planes / 8` minus the 18-byte frame header.

### `encode`

| option | default | meaning |
|---|---|---|
| `-i, --input` | required | payload file to embed |
| `-o, --output` | `<input>.dng` | output path |
| `--width`, `--height` | auto/preset | raw frame geometry |
| `--auto-size` | off | compute minimal non-preset dimensions for the payload |
| `--mode` | profile suggestion, then `linear` | `linear` or `cfa` raw layout |
| `--bit-depth` | `16` | packed sample depth: 8, 10, 12, 14 or 16 |
| `--lsb-planes` | smallest fitting | LSB planes per sample, 1–16 |
| `--raw-frames` | `1` | full-resolution raw frames in one file, 1–8 |
| `--seed` | random | seed for cover, metadata and thumbnail draws |
| `--camera-profile` | `gfx_100` | profile slug, optionally `slug-subprofile` |
| `--level-policy` | `profile` | `profile`, `capacity` or `strict` |
| `--compression` | `none` | raw tile compression: `none` or `adobe_deflate` |
| `--thumbnail` | `synthetic` | `synthetic`, `random` or an image path |
| `--image-time` | `random` | capture time source, see below |
| `--gps-radius` | `25` | GPS sampling radius in km, 0–25 |
| `--no-randomize` | off | deterministic metadata instead of random draws |
| `--key` | none | passphrase for the XOR keystream |
| `--bigtiff` | auto | force 64-bit BigTIFF output |
| `--split-file` | off | split payload into chunks of at most `SIZE` |
| `--split-file-metadata-id` | `ImageUniqueID` | shared set ID field |
| `--split-file-metadata-seq` | `ImageNumber` | chunk sequence field |
| `--split-file-id` | generated | override the shared set ID |
| `--split-file-album` / `--no-split-file-album` | album | group chunks in a directory |
| `--debug-metadata-output` | off | write a JSON sidecar beside the output (`photo.json` for `photo.dng`) |
| `--progress` / `--no-progress` | TTY-aware | force or suppress the progress bar |

`SIZE` accepts plain bytes or `K`/`M`/`G` suffixes (binary units), for example
`512K`, `100M` or `2G`.

`--image-time` accepts:

- `random` — draw a plausible time from the profile;
- `now` — current local time;
- `input-created` — payload file birth time, falling back to Unix `ctime`;
- `input-modified` — payload file modification time;
- an explicit `YYYY:MM:DD HH:MM:SS`, `YYYY-MM-DDTHH:MM:SS`, or RFC 3339 value.

If `--width`/`--height` are omitted, RawOxide chooses a camera-like preset that
fits the payload. `--auto-size` instead searches for the smallest geometry,
which produces a smaller but potentially less typical file.

### `decode`

| option | default | meaning |
|---|---|---|
| `-i, --input` | required | one DNG, or several chunk files |
| `-o, --output` | required | recovered payload path |
| `--key` | none | passphrase used at encode time |
| `--lsb-planes` | auto-detect | planes used at encode time, 1–16 |
| `--max-bytes` | unlimited | safety cap on the declared payload per file |
| `--split-file-metadata-id` | `ImageUniqueID` | shared set ID field |
| `--split-file-metadata-seq` | `ImageNumber` | chunk sequence field |
| `--progress` / `--no-progress` | TTY-aware | force or suppress the progress bar |

Without `--lsb-planes`, decode tries plane counts 1–16 and accepts the first
that passes the frame magic and CRC. Split chunks can be supplied in any order
when the sequence field is present; filename ordering is used otherwise. Output
is written to a temporary file beside the destination and renamed only after
the payload passes its CRC.

### `gen-tiff`

Writes a plain uncompressed 8-bit RGB TIFF with a synthetic gradient cover. It
is useful for testing or for producing a simple cover image.

| option | default |
|---|---|
| `-o, --output` | required |
| `--width` | `2048` |
| `--height` | `1536` |
| `--seed` | random |

## Advanced features

### Camera profiles

Camera profiles drive geometry, raw layout, bit depth, metadata pools, level
policy and GPS behaviour. The registry is embedded in the binary; profiles live
in `assets/profiles/<slug>/profile.json`. Select a profile with
`--camera-profile canon_eos_80d`; profiles with alternative geometries also
accept a sub-profile suffix, for example `--camera-profile canon_eos_80d-native`.

### Auto sizing and planes

With no geometry flags, the encoder picks the smallest preset that fits the
payload and normally asks for the fewest LSB planes that will hold it. Explicit
`--width`, `--height` and `--lsb-planes` override that. `--auto-size` bypasses
the presets and finds the smallest exact geometry; it cannot be combined with
`--width`, `--height` or a sub-profile, and the CLI warns when the result is not
a normal camera size.

### Raw layouts and depths

`linear` stores three samples per pixel (demosaiced RGB); `cfa` stores one
sample per pixel (Bayer mosaic). Supported packed depths are 8, 10, 12, 14 and
16 bits. `adobe_deflate` compression is available for 8- and 16-bit output.

### Level policy

`--level-policy` controls how much of the sample range the synthetic cover uses:

- `profile` (default) uses the selected camera profile's `raw.level_policy`;
- `capacity` keeps the full container range, maximising capacity;
- `strict` honours the profile's white level and plane ceiling, which looks most
  plausible but can reduce capacity.

The policy also affects emitted white/black levels, the synthetic-cover range,
CFA pattern tags and black-level repeat dimensions.

### Multiple raw frames

`--raw-frames N` writes 1–8 full-resolution raw `SubIFD`s and stripes the
payload across them. This is useful for burst-style covers and increases the
available capacity without changing the pixel dimensions.

### Split sets

`--split-file SIZE` cuts the payload into chunks, writes DCF-style names
(`vol0001.dng`, `vol0002.dng`, …), and records the set membership in tag
metadata. By default chunks go into an album directory named after the input
file; `--no-split-file-album` writes them beside the output instead.

Choose the sequence field deliberately:

- `PageNumber` stores `(current, total)` and fails closed on a missing or extra
  chunk. It is the safest choice for new sets.
- `ImageNumber` stores only the current number, so missing chunks are not
  detected.
- `ImageDescription` stores `"NNNN/MMMM"` and can be used for either purpose,
  but the same field cannot be selected for both the ID and the sequence.

The shared set ID is validated as non-empty ASCII, and sets are limited to 9999
chunks by the four-digit naming convention.

### BigTIFF

Output is classic TIFF until the raw data would exceed classic offsets, then
RawOxide switches to BigTIFF automatically. Use `--bigtiff` to force it for
small files. BigTIFF is valid DNG 1.6 and is understood by `tifffile` and the
Rust reader; some older tools (notably `exiv2`) cannot parse it. RawOxide emits
a warning when it writes BigTIFF.

### Previews

`--thumbnail synthetic` generates an offline gradient/noise preview and is the
default. `--thumbnail random` fetches a random Wikimedia Commons image (with a
deterministic offline-noise fallback). A path selects a local image, which is
centre-cropped and resized with Lanczos filtering. Profile preview dimensions
are respected.

### Metadata, time and GPS

Metadata is drawn from the selected profile: camera make/model, lens, exposure,
ISO, serials, software, orientation and DNG colour tags. `--no-randomize` uses a
deterministic seed; `--seed` makes a random run reproducible. `--image-time`
pins the capture timestamp, and `--gps-radius` controls how far the sampled
location may move from the profile's principal city. Profiles that emit GPS get
a dedicated GPS IFD with UTC timestamps consistent with the local capture time.

### Debug sidecar

`--debug-metadata-output` writes a JSON document next to each output file:

```bash
rawoxide encode -i secret.bin -o photo.dng --debug-metadata-output
# photo.dng + photo.json
```

The sidecar replaces the output extension with `.json`. If the output already
ends in `.json`, `.dng.json` is appended instead.

The sidecar records the invocation, resolved options, payload and capacity,
cover and preview details, the profile snapshot, the drawn metadata, effective
seeds, per-frame CRCs and a named inventory of every written IFD tag. It is the
easiest way to audit or reproduce a run.

### Keys and threat model

`--key` applies a SHA-256 counter XOR keystream to the framed payload. It
prevents casual inspection of the LSB planes, but it is **not authenticated
encryption**: a wrong key fails closed on the magic/CRC, while an attacker who
can modify the file can also modify the payload. Use a real encryption tool
(age, GPG) before embedding if you need authenticity or strong confidentiality.

## How it works

The payload is wrapped in an 18-byte frame:

```text
magic "DNGS" | version | flags | payload length (u64 BE) | CRC32 (u32 BE) | payload
```

Frame bits are written to LSB plane `i % planes` of sample `i / planes`, spread
across the raw frames. The DNG writer emits tiled `LinearRaw`/`CFA` pages, a
baseline JPEG preview, `SubIFD`s, EXIF/GPS IFDs and the profile's DNG/Adobe tags.
Decode reads raw bands, probes the header, extracts only as many bytes as the
declared payload needs and verifies the CRC before publishing the output.

See [`docs/format.md`](docs/format.md) for the byte-level format and
[`docs/architecture.md`](docs/architecture.md) for the module layout, data flow
and memory rules.

## Compatibility

RawOxide implements the same payload framing and DNG conventions as the original
RawHide tool, so files written by either implementation decode in the other.
Compatibility is at the level of outcomes — payload bytes, split semantics and
DNG readability — not byte-identical files. RNG streams, IFD ordering, JPEG
bytes and metadata pool order are native to RawOxide.

Decode accepts the uncompressed and Deflate raw layouts RawOxide writes,
including packed 10/12/14-bit samples and multiple frames. DNGs from other
writers that use different compressions or sample layouts are outside the
current decode scope; encode output is the primary input.

See [`docs/compatibility.md`](docs/compatibility.md) for the exact contract and
known boundaries.

## Performance and robustness

- Encode streams the payload in 64 KiB chunks and processes one tile band at a
  time; peak memory is bounded by the band and writer state, not payload size.
- Decode writes directly to a temporary output file as bands are read, so CLI
  decode memory stays near constant even for multi-gigabyte payloads.
- `--max-bytes` is enforced before allocating a declared payload.
- Malformed TIFF/IFD/framing data returns an error; panics are treated as bugs.
- `unsafe` is forbidden in the core crate; dependencies are audited in CI.

## Documentation

| document | contents |
|---|---|
| [`docs/format.md`](docs/format.md) | payload frame, LSB mapping, DNG layout, split markers |
| [`docs/architecture.md`](docs/architecture.md) | workspace, data flow, memory and dependency rules |
| [`docs/compatibility.md`](docs/compatibility.md) | interoperability contract and known boundaries |
| [`docs/testing.md`](docs/testing.md) | test pyramid, fixtures, cross-decode and consumer checks |
| [`docs/release.md`](docs/release.md) | release checklist and artifact names |
| [`docs/adr/`](docs/adr/) | architecture decision records |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | setup, style and pull-request gates |
| [`SECURITY.md`](SECURITY.md) | vulnerability reporting and threat model |

## Contributing

Issues and pull requests are welcome. Run `just ci` (or the individual commands
in [Building and testing](#building-and-testing)) before opening a pull
request, and add a test for every behaviour change. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for details.

## License

MIT. See [`LICENSE`](LICENSE).
