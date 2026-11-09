# Porting matrix

Status: ✅ done · 🚧 partial · ⬜ planned · 🧪 needs oracle fixtures

| Python module | Rust target | Status | Notes / acceptance |
|---|---|---|---|
| `stegodng/framing.py` | `core::framing` | ✅ | Python golden vectors pinned; keyed/unkeyed round-trip |
| `stegodng/codec.py` | `core::codec` | 🚧 | In-memory `u8`/`u16` mapping done; multi-frame streaming planned |
| `stegodng/codec.py` formatters | `core::codec::{format_kb*}` | ✅ | Python Ki KiB output pinned |
| `stegodng/profile.py` | `core::profile` | ✅ | Constants, depth/black/white helpers, fidelity policies |
| `stegodng/metadata.py` | `core::metadata` | ✅ | Seeded pools/fields, profile ranges, GPS sampling |
| `stegodng/geo/` | `core::geo` | ✅ | `zones.bin` embedded, land sampling, tz offsets |
| `stegodng/filetime.py` | `core::filetime` | ⬜ | birth time (`statx`/`created`), mtime, now |
| `stegodng/cover.py` | `core::cover` / `writer` | 🚧 | synthetic raw covers; plain TIFF gen-tiff |
| `stegodng/container.py` | `core::dng` reader | ✅ | `tiff-reader` tags/IFDs + custom raw block reader; tile/strip, Deflate, packed, BigTIFF |
| `stegodng/container.py` | `core::writer` | 🚧 | classic TIFF, tiled raw, LinearRaw/CFA, 8/10/12/14/16-bit, JPEG preview, 1-8 frames, EXIF split markers; Python/rawpy accepted |
| `stegodng/digests.py` | `core::digests` | ✅ | tile MD5 + RawDataUniqueID incl. GainMap fingerprints |
| `stegodng/fidelity.py` | `core::fidelity` | ⬜ | serde config + validation |
| `stegodng/makernote.py` | `core::profile` / `core::metadata` | ✅ | HDRP envelope parsed, seeded generation, writer tags |
| `stegodng/opcodes.py` | `core::opcodes` | 🚧 | GainMap build/parse/fingerprint; other opcode ids unsupported |
| `stegodng/sizing.py` | `core::sizing` | ✅ | Presets, auto-size, plane ceilings, KB errors, warnings |
| `stegodng/split.py` | `core::split` | ✅ | field parsing, sizes, filename/metadata ordering, chunk bounds/naming, album layout |
| `stegodng/stego.py` | `core::decode` / `core::writer` | 🚧 | decode orchestration, writer and split encode done; metadata/streaming planned |
| `stegodng/thumbnails.py` | `core::thumbnail` | ⬜ | Commons/file/synthetic, Lanczos, YCbCr |
| `stegodng/cli.py` | `cli` | 🚧 | `capacity`, `decode` and spike `encode` done; `gen-tiff` stub |
| `stegodng/profiles/__init__.py` | `core::profiles` | ✅ | 307 JSON + 3 blobs embedded; lazy load, longest-slug resolve |
| `stegodng/debugmeta.py` | `core::debug` | ⬜ | JSON sidecar schema |
| `stegodng/progress.py` | `cli::progress` | ⬜ | indicatif tri-state |

## CLI flags checklist

| flag family | status |
|---|---|
| `capacity` | ✅ |
| `encode`: input/output/width/height/mode/bit-depth/lsb-planes/key | ✅ (writer path) |
| `encode`: seed/thumbnail | 🚧 (seed used for covers/frames/metadata; thumbnail still synthetic) |
| `encode`: auto-size/frames | ✅ (`--auto-size`, auto planes, `--raw-frames 1-8`) |
| `encode`: profiles/subprofiles/level-policy | ✅ |
| `encode`: compression/split/album/metadata fields | ✅ (none/adobe_deflate, split/album/metadata) |
| `encode`: image-time/GPS/debug sidecar/progress | 🚧 (`--image-time` and GPS done; debug/progress pending) |
| `decode`: input set/key/planes/max-bytes/split fields | ✅ |
| `decode`: progress | ⬜ |
| `gen-tiff` | ✅ |

## Required oracle fixtures

- ✅ `linear_256x192_16bit_1plane.dng`
- ✅ `linear_128x96_8bit_deflate.dng`
- ✅ `cfa_128x96_14bit_2planes.dng`
- ✅ `linear_128x96_16bit_4planes_keyed.dng`
- ✅ `linear_128x96_16bit_3frames.dng`
- ✅ `split0001.dng` / `split0002.dng`
- ⬜ `gps_custom_pixel_xl.dng` (after fixing `date_bounds` or pinning image time)
- ✅ framing/codec/capacity JSON golden vectors

Update this table as modules land. A module is only ✅ when its behaviour is
covered by tests, Python cross-decode works where interoperability matters, and
consumer tools accept the result. Structural/byte-level parity with Python is
not required; implementation freedom is recorded in ADR 0001.
