# ADR 0003 — TIFF/DNG stack: `tiff-reader` plus a custom writer

**Status:** accepted
**Date:** 2026-11-08

## Context

RawOxide needs to read and write tiled DNG raws with structures that general
TIFF crates often do not expose:

- `SubIFDs`;
- `LinearRaw` and `CFA` photometric values;
- packed 10/12/14-bit samples;
- arbitrary DNG/Adobe tags;
- classic TIFF and BigTIFF;
- multiple full-resolution frames and split markers.

## Decision

1. **Read** with `tiff-reader` 0.11 (pinned exactly) for IFD/tag parsing, EXIF
   pointers, tile/strip tables and BigTIFF structure.
2. **Write** with a small purpose-built classic TIFF/BigTIFF writer in
   `rawoxide-core::writer`, built on the typed `rawoxide-core::tiff` layer.
3. Read raw blocks in `rawoxide-core::dng` rather than through the crate's
   colour-model path, because `tiff-reader` 0.11 rejects `LinearRaw`/`CFA` in
   that path and lacks classic TIFF type 13 (`IFD`) for `SubIFDs`.
4. Keep both behind `DngReader` and the writer module so either side can be
   replaced without touching decode or CLI layers.
5. Treat `rawler`/`dnglab` and similar full DNG libraries as references only
   under the MIT license (ADR 0004).

## Consequences

- The reader supports DNG `LinearRaw`/`CFA`, packed samples, Deflate with
  predictor 2 and multiple frames without vendoring a crate.
- The writer has full control over `SubIFDs`, photometric values, packed depths,
  split/EXIF markers and BigTIFF pointers.
- `tiff-reader` is pinned exactly; upgrades need a review of its DNG and
  packed-sample behaviour.
- DNG-specific logic stays in this project's adapter modules instead of being
  scattered through the CLI.
- A future upstream that supports DNG photometrics and type 13 could replace
  the custom raw-block reader; the public boundaries do not change.

## History

The initial design evaluated `tiff-writer` 0.11 for the write path. It could not
emit `SubIFDs`, did not expose `LinearRaw`/`CFA`, and only supported
8/16/32/64-bit samples, so the custom writer replaced it before the writer
spike was complete. The read-side limitation of `tiff-reader` was found during
the same spike and led to the custom raw-block reader.
