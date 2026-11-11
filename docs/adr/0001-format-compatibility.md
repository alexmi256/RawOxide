# ADR 0001 — Format compatibility, not byte-identical output

**Status:** accepted
**Date:** 2026-11-08

## Context

RawOxide writes a payload into DNG raw samples. Two readers must be able to
agree on a file:

1. **wire-format compatibility** — another implementation of the same format can
   recover the payload; or
2. **byte-identical output** — the same seed produces exactly the same DNG.

Option 2 would require pinning every implementation detail that contributes to
the bytes: RNG draw order, the JPEG encoder, resampling kernels, IFD ordering,
compression framing and metadata pool order. Those details change as libraries
evolve, and they do not change what the file means or whether the payload is
recoverable.

## Decision

RawOxide guarantees **wire-format compatibility** for the payload frame, LSB
mapping, capacity accounting and split-set semantics. A file written by a
compatible implementation can be decoded here, and RawOxide output can be
decoded there.

Seed stability is a within-implementation guarantee: the same seed and pinned
capture time reproduce a RawOxide run, but the bytes are not promised to match
another implementation.

The writer is free to choose its own IFD layout, tag order, tiling, EXIF/GPS
construction and profile representation as long as the resulting DNG carries the
same information, is accepted by real readers, and round-trips the payload.

## Consequences

- The RNG is native and documented (ADR 0002); it is not copied from another
  implementation.
- Golden vectors are split: committed fixtures prove decode compatibility;
  native tests pin RawOxide output.
- CI can run a cross-decode matrix rather than a cross-language hash comparison.
- Documentation says "same seed → same file within RawOxide", not "same as
  another tool".
