# ADR 0003 — TIFF/DNG stack: `tiff-writer` + `tiff-reader`, with patches

**Status:** accepted; Phase 1 amended (see below)
**Date:** 2026-11-08

## Context

The Python tool relies on `tifffile` for tiled raw pages, JPEG-compressed YCbCr
strips, packed `BitsPerSample`, Deflate + predictor, BigTIFF, SubIFDs and
arbitrary tags. No native library covers all of that.

Survey results (see the feasibility study §4.4):

- `tiff-writer` 0.11 (MIT/Apache, Nov 2026): tiles, strips, BigTIFF, Deflate,
  LZW, JPEG 4:2:0, streaming, arbitrary tags. Gaps: no SubIFDs, no
  `LinearRaw`/`CFA` photometric enum, no packed 10/12/14-bit.
- `tiff-reader` 0.11: tiles, SubIFDs, bit-packed 1–15 bits, Deflate/LZW/JPEG,
  parse budgets.
- `tiff` 0.11 (image-rs): mature, BigTIFF, tags, strips; no tiled/JPEG writing.
- `tiff-forge` 0.5: arbitrary IFD hierarchy + BigTIFF; caller supplies blocks.
- `rawler`/`dnglab` (LGPL-2.1): capable full DNG writer, camera-RAW-oriented;
  reference only unless the project license changes (ADR 0004).
- `dng` (AGPL-3.0): too restrictive for the default license; not a fit.

## Decision

1. **Read** with `tiff-reader` 0.11 (pinned exact).
2. **Write** with `tiff-writer` 0.11 (pinned exact), behind a small
   `core::tiff` adapter so the dependency can be replaced.
3. Patch/extend the writer for:
   - SubIFD emission (tag 330) and child IFD offsets;
   - `LinearRaw` (34892) and `CFA` (32803) photometric values;
   - packed 10/12/14-bit samples.
4. Link EXIF/GPS IFDs after the main write (as Python does) if the writer does
   not expose child IFD pointers directly.
5. Fallback: `tiff`/`tiff-forge` plus a custom writer if the 0.x API or bugs
   make the adapter untenable.
6. `rawler`/`dnglab` code may be studied as a reference; do not link it under
   the default MIT license.

## Consequences

- MSRV is 1.88 (`tiff-writer` requirement).
- Pin exact versions; audit upgrades. The 0.11 API is young.
- Keep the DNG-specific logic in our adapter, not scattered through the CLI.
- The one-week spike (256×192 LinearRaw + JPEG preview, verified by
  `tifffile`/`rawpy`/`tiffdump`) is the go/no-go gate for this ADR.
- If the spike fails, revisit ADR 0004 (license) before adopting `rawler`.

## Amendment 2026-11-08 — Phase 1 implementation

The read spike exposed two limitations in `tiff-reader` 0.11:

1. Its color-model validation rejects DNG `LinearRaw` (34892) and `CFA`
   (32803), so `raster_layout()`/`read_image_bytes_from_ifd()` cannot be used on
   our raw SubIFDs.
2. Its tag type table has no classic TIFF `IFD` type (13), so the `SubIFDs`
   tag (330) is not read correctly.

We keep `tiff-reader` for IFD/tag parsing, EXIF pointers, tile/strip offset
tables and BigTIFF, and read raw blocks ourselves in `dng.rs`: uncompressed and
Deflate blocks, predictor 2, packed 10/12/14-bit samples, tiles/strips and
multiple frames. This keeps Phase 1 working without vendoring the crate.
Revisit if upstream adds DNG photometrics and type-13 support; the custom
reader is isolated behind `DngReader`.

## Amendment 2026-11-08 — writer spike

`tiff-writer` 0.11 cannot emit `SubIFDs`, does not expose `LinearRaw`/`CFA`
photometric values, and only supports 8/16/32/64-bit samples. The Phase 2 writer
therefore uses a small classic-TIFF writer in `core::writer`: tiled
`LinearRaw`/`CFA` raw pages at 8/10/12/14/16-bit, one baseline JPEG preview
strip, a `SubIFDs` pointer, DNG calibration tags and optional payload embedding.
Python `stego_dng.decode`, `tifffile`, `exiv2`, `tiffdump` and
`rawpy.postprocess()` accept the result for every mode/depth combination.

## Amendment 2026-11-08 — frames and split sets

The custom writer was extended rather than replaced. It now writes 1-8
full-resolution raw frames (one `SubIFD` each, payload striped across them) and
links an EXIF IFD for split markers (`ImageUniqueID`, `ImageNumber`) while
IFD0 carries `PageNumber`/`ImageDescription` sequences. Tag 34665 is written as
a `LONG` offset (not TIFF type 13), matching what readers expect; tag 330
(`SubIFDs`) uses type 13. The split CLI writes DCF-style chunk names in the
Python album layout. Python decodes Rust multi-frame and split outputs, and
`rawpy` accepts the multi-frame file.

Remaining writer work is streaming/banded writes for multi-gigabyte covers,
EXIF/GPS metadata, profiles, optional Deflate and BigTIFF.
