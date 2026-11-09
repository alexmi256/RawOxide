# ADR 0004 — License: MIT by default; revisit if `rawler` is adopted

**Status:** accepted
**Date:** 2026-11-08

## Context

The Python project is MIT. The maintainer controls the Rust port's copyright
and can relicense it. `rawler`/`dnglab` are LGPL-2.1; the `dng` crate is
AGPL-3.0. Using those would change the license obligations of the distributed
port, not merely its source availability. "Non-commercial" is a description of
the project's operation, not a license term: both LGPL and AGPL permit
commercial use.

## Decision

- The Rust port is **MIT**.
- Default dependencies are MIT/Apache/BSD-compatible (enforced by
  `cargo-deny`).
- `tiff-writer`/`tiff-reader` (MIT/Apache) are the chosen TIFF stack, so no
  copyleft dependency is needed for the default build.
- `rawler`/`dnglab`/`dng` are **reference implementations** only.
- If the writer spike fails and `rawler` becomes necessary, open a new ADR and
  relicense the whole port deliberately (AGPL-3.0 or LGPL-2.1/3.0), with legal
  review and updated `LICENSE`/metadata/third-party notices. Relicensing is a
  project-wide decision, not a local dependency change.

## Consequences

- The port stays usable by downstream MIT/Apache projects.
- `cargo-deny` blocks accidental copyleft dependencies.
- If the license changes later, all contributors' notices and the Python MIT
  provenance must be preserved.
