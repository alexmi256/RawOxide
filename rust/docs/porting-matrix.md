# Porting matrix

Status: ✅ done · 🚧 partial · ⬜ planned · 🧪 needs oracle fixtures

| Python module | Rust target | Status | Notes / acceptance |
|---|---|---|---|
| `stegodng/framing.py` | `core::framing` | ✅ | Python golden vectors pinned; keyed/unkeyed round-trip |
| `stegodng/codec.py` | `core::codec` | ✅ | `u8`/`u16` mapping, per-frame and banded writes |
| `stegodng/codec.py` formatters | `core::codec::{format_kb*}` | ✅ | Python Ki KiB output pinned |
| `stegodng/profile.py` | `core::profile` | ✅ | Constants, depth/black/white helpers, fidelity policies |
| `stegodng/metadata.py` | `core::metadata` | ✅ | Seeded pools/fields, profile ranges, GPS sampling |
| `stegodng/geo/` | `core::geo` | ✅ | `zones.bin` embedded, land sampling, tz offsets |
| `stegodng/filetime.py` | CLI `resolve_image_time` | ✅ | birth time (`created`/`statx`), Unix `ctime` fallback, mtime, now, explicit values |
| `stegodng/cover.py` | `core::cover` / `writer` | ✅ | synthetic raw covers; plain TIFF gen-tiff |
| `stegodng/container.py` | `core::dng` reader | ✅ | `tiff-reader` tags/IFDs + custom raw block reader; tile/strip, Deflate, packed, BigTIFF |
| `stegodng/container.py` | `core::writer` | ✅ | classic TIFF + BigTIFF, tiled raw, LinearRaw/CFA, 8/10/12/14/16-bit, JPEG preview, 1-8 frames, profile tile geometry, crop ActiveArea, EXIF split markers; Python/tifffile classic+BigTIFF, exiv2/rawpy classic only |
| `stegodng/digests.py` | `core::digests` | ✅ | tile MD5 + RawDataUniqueID incl. GainMap fingerprints |
| `stegodng/fidelity.py` | `core::profile` | ✅ | metadata/raw/Adobe profile fields, `raw.tile`, crop `ActiveArea`, Adobe tag conventions |
| `stegodng/makernote.py` | `core::profile` / `core::metadata` | ✅ | HDRP envelope parsed, seeded generation, writer tags |
| `stegodng/opcodes.py` | `core::opcodes` | ✅ | GainMap + profile OpcodeList3; other opcode ids are also unsupported in the Python reference |
| `stegodng/sizing.py` | `core::sizing` | ✅ | Presets, auto-size, plane ceilings, KB errors, warnings |
| `stegodng/split.py` | `core::split` | ✅ | field parsing, sizes, filename/metadata ordering, chunk bounds/naming, album layout, ASCII id and 9999-chunk validation |
| `stegodng/stego.py` | `core::decode` / `core::writer` | ✅ | decode, writer, split encode, banded streaming, incremental digests, sidecar stats |
| `stegodng/thumbnails.py` | `core::thumbnail` | ✅ | Commons/synthetic/offline-noise/file + Lanczos + resolved labels |
| `stegodng/cli.py` | `cli` | ✅ | all commands/flags, album layout, thumbnail/profile/meta/debug output, BigTIFF warnings |
| `stegodng/profiles/__init__.py` | `core::profiles` | ✅ | 307 JSON + 3 blobs embedded; lazy load, longest-slug resolve |
| `stegodng/debugmeta.py` | `cli::sidecar` | ✅ | Python-shaped summary/options/payload/cover/thumbnail/profile/metadata + named IFD/preview inventory |
| `stegodng/progress.py` | `cli` | ✅ | TTY-aware progress bars for encode/decode, split chunk messages and row/byte percentages |

## CLI flags checklist

| flag family | status |
|---|---|
| `capacity` | ✅ |
| `encode`: input/output/width/height/mode/bit-depth/lsb-planes/key | ✅ (writer path) |
| `encode`: seed/thumbnail | ✅ (Commons/synthetic/offline-noise/file thumbnails) |
| `encode`: auto-size/frames | ✅ (`--auto-size`, auto planes, `--raw-frames 1-8`) |
| `encode`: profiles/subprofiles/level-policy | ✅ |
| `encode`: compression/split/album/metadata fields | ✅ (none/adobe_deflate, split/album/metadata, positive and negative album flags) |
| `encode`: BigTIFF | ✅ (`--bigtiff`, automatic above 3.5 GB) |
| `encode`: image-time/GPS/debug sidecar/progress | ✅ (sidecar, `--no-randomize`, progress flags; spinner only) |
| `decode`: input set/key/planes/max-bytes/split fields | ✅ |
| `decode`: progress | ✅ |
| `gen-tiff` | ✅ (plain RGB TIFF; minimal writer, documented metadata/output differences) |

## Required oracle fixtures

- ✅ `linear_256x192_16bit_1plane.dng`
- ✅ `linear_128x96_8bit_deflate.dng`
- ✅ `cfa_128x96_14bit_2planes.dng`
- ✅ `linear_128x96_16bit_4planes_keyed.dng`
- ✅ `linear_128x96_16bit_3frames.dng`
- ✅ `split0001.dng` / `split0002.dng`
- ⬜ `gps_custom_pixel_xl.dng` (add a fixed-image-time oracle fixture; the per-run date anchor is implemented)
- ✅ framing/codec/capacity JSON golden vectors

Update this table as modules land. A module is only ✅ when its behaviour is
covered by tests, Python cross-decode works where interoperability matters, and
consumer tools accept the result. Structural/byte-level parity with Python is
not required; implementation freedom is recorded in ADR 0001.

## Known differences

- `--thumbnail` defaults to `synthetic` (offline-safe) instead of Python's
  `random`; pass `--thumbnail random` for the Commons behaviour.
- Progress uses one bar with row/byte percentages and a chunk message; Python
  uses nested per-file/embedding bars.
- The Rust writer uses 8-byte `LONG8` EXIF/GPS pointers in BigTIFF and the Rust
  reader handles them. Python's BigTIFF writer stores an 8-byte offset in a
  4-byte `LONG` field; such files past 4 GiB are outside the Rust decode scope.
- Decode accepts the uncompressed and Deflate raw layouts RawHide writes.
  External DNGs using other compressions or sample layouts are outside the
  current decode scope.
- `gen-tiff` uses the custom writer and reports output size; Python uses
  `tifffile` with the default profile's software string.
- The decode no-totals advisory uses `warning:` where Python uses `note:`; the
  sidecar `tool` block reports the Rust target rather than Python dependency
  versions.
- `just cross-decode` (also a CI job) runs encode/decode in both directions for
  linear, CFA, multi-frame and split output.
