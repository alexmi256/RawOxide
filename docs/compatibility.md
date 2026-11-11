# Compatibility

RawOxide is a Rust implementation of the RawHide concept. The two tools share a
payload and DNG format so that files can move between them, while each keeps its
own internal implementation.

## Contract

| level | guarantee |
|---|---|
| payload wire format | Same `DNGS` frame, CRC, length/flag layout and SHA-256 keystream. |
| LSB mapping | Same plane/sample assignment for `u8` and `u16` samples. |
| split sets | Same shared-ID and sequence metadata fields, naming and ordering semantics. |
| DNG readability | Output is a valid DNG accepted by ordinary raw readers. |
| decode interoperability | A file written by either implementation decodes in the other. |
| seed stability | Stable within RawOxide for a given seed and pinned time. Not shared with RawHide. |

Compatibility is **outcome-compatible**, not byte-identical. RNG draw order,
IFD entry order, JPEG encoder bytes, Lanczos implementation, profile storage and
metadata pool order are native to RawOxide. None of them affect whether a payload
is recovered or whether a DNG is valid.

## Feature status

| area | status | notes |
|---|---|---|
| capacity | complete | preset and auto geometry, 1–16 planes, 1–8 frames |
| encode | complete | 8/10/12/14/16-bit, linear/CFA, tiled raw, JPEG preview |
| decode | complete | auto plane detection, streaming output, split sets |
| camera profiles | complete | 307 profiles + Adobe assets embedded |
| metadata | complete | seeded pools, EXIF/DNG tags, maker note, per-run date anchor |
| GPS | complete | land sampling, radius, UTC timestamps, split-shared location |
| digests/opcodes | complete | raw image digest, unique ID, gain map and profile opcode lists |
| compression | complete | none and Adobe Deflate with predictor 2 |
| BigTIFF | complete | automatic above classic limits, or forced |
| split sets | complete | album layout, DCF names, sequence/ID markers, 9999-chunk guard |
| debug sidecar | complete | options, seeds, profile snapshot, IFD inventory, CRCs |
| `gen-tiff` | complete | plain uncompressed 8-bit RGB TIFF |
| external DNG decode | limited | uncompressed/Deflate RawOxide-style layouts only |
| parallel encoding | planned | current encoder is single-threaded |
| fuzzing | planned | no fuzz targets committed yet |

## Known boundaries

- **Debug sidecar schema.** The JSON sidecar keeps the stable schema id
  `stegodng-debug-metadata` so existing consumers continue to recognise it.
- **Thumbnail default.** RawOxide defaults `--thumbnail` to `synthetic` so encode
  works offline and reproducibly. Pass `--thumbnail random` for a fetched
  Commons photo.
- **BigTIFF EXIF pointers.** RawOxide writes 8-byte `LONG8`/`IFD8` EXIF and GPS
  pointers in BigTIFF and reads them back. BigTIFF files from other writers that
  store an 8-byte offset inside a 4-byte `LONG` field are outside the reader
  scope when the pointed IFD lies past 4 GiB; the raw payload path is unaffected.
- **External raw layouts.** Decode accepts uncompressed and Deflate raw blocks,
  strips or tiles, packed 10/12/14-bit samples and multiple frames. DNGs from
  other writers using other compressions or sample layouts are not accepted.
- **Opcode ids.** Gain-map opcode lists and profile opcode lists are generated
  and fingerprinted. Other opcode ids are not interpreted.
- **`gen-tiff`.** Uses a minimal built-in writer and does not reproduce another
  writer's software tag or TIFF metadata byte for byte.

## Checking interoperability

The repository includes an optional developer script that encodes with one tool
and decodes with the other, covering linear, CFA, multi-frame and split output:

```bash
# Needs a RawHide/ checkout at the repository root and its venv.
RAWHIDE_ORACLE=./RawHide just cross-decode
```

The Rust test suite itself is hermetic and does not require the reference
implementation: committed DNG fixtures and golden vectors cover the decode and
framing paths.

## Stability

The `DNGS` frame version and the split metadata semantics are stable. New frame
versions are identified by the version byte; readers reject unknown versions
instead of guessing. Changes that would break existing payloads or split sets
require a new format version and a release note.
