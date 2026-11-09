# ADR 0001 — Format compatibility, not byte-identical output

**Status:** accepted
**Date:** 2026-11-08

## Context

The Python implementation is the specification. A port could aim for:

1. **wire-format compatibility** — either implementation can decode the other's
   files; or
2. **seed-compatible output** — the same seed produces a byte-identical DNG.

Option 2 requires reproducing NumPy's PCG64 + `SeedSequence` + ziggurat normal
draws, Python's MT19937 `_randbelow` semantics, Pillow's Lanczos kernel and
libjpeg-turbo's JPEG bytes, plus the same tzdata and tifffile packed-bit/Deflate
details. NumPy's own NEP 19 policy does not guarantee `Generator` distribution
streams across versions, so even Python output is only stable for a pinned
dependency set. The maintenance cost is high and the user-facing value is low.

## Decision

The Rust port guarantees **wire-format compatibility (L1)** only. Seed-stable
output is a within-port guarantee. Python output is decoded by Rust and Rust
output by Python; byte-for-byte equality across languages is not promised.

The port also does **not** replicate Python's internal implementation where a
cleaner Rust design gives the same outcome. Examples: dummy-tag patching, exact
IFD entry ordering, tifffile-specific strip/tile workarounds, Adobe tag
inventory parity, Python/NumPy RNG draw order, Pillow's Lanczos/JPEG bytes, and
the exact metadata field pool order. Those are implementation details, not
functionality. Writers may use their own IFD layout, tag order, tiling, EXIF/GPS
construction and profile representation as long as the resulting DNG carries
the same information, is accepted by real readers, and round-trips the payload.

The compatibility contract is therefore: same payload semantics, same DNG
read/write outcomes, same user-facing capabilities; not the same bytes or the
same internal structure.

## Consequences

- Define a native, documented RNG (see ADR 0002); do not copy NumPy/Python draws.
- Golden vectors are split: Python fixtures prove decode compatibility; Rust
  fixtures pin Rust output.
- CI runs a cross-decode matrix, not a cross-language hash comparison.
- User documentation must say "same seed → same file within RawHide", not
  "same as Python".
