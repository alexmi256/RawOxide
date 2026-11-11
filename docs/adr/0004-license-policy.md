# ADR 0004 — License: MIT by default; revisit if a copyleft DNG library is adopted

**Status:** accepted
**Date:** 2026-11-08

## Context

RawOxide is MIT-licensed. Some capable DNG implementations use copyleft
licenses: `rawler`/`dnglab` are LGPL-2.1 and the `dng` crate is AGPL-3.0.
Adopting one would change the distribution obligations of the whole project,
not just its source availability. "Non-commercial" is a description of how a
project is run, not a license term: both LGPL and AGPL permit commercial use.

## Decision

- RawOxide is **MIT**.
- Default dependencies are MIT/Apache/BSD-compatible, enforced by `cargo-deny`.
- `tiff-reader` (MIT/Apache) is used for TIFF parsing; the custom writer is this
  project's own MIT code. No copyleft dependency is needed for the default
  build.
- `rawler`/`dnglab`/`dng` are **reference implementations** only.
- If a future writer/reader requirement makes a copyleft library necessary,
  open a new ADR and relicense the whole project deliberately (AGPL-3.0 or
  LGPL-2.1/3.0), with legal review and updated `LICENSE`, metadata and
  third-party notices. Relicensing is a project-wide decision, not a local
  dependency change.

## Consequences

- The project stays usable by downstream MIT/Apache projects.
- `cargo-deny` blocks accidental copyleft dependencies.
- If the license changes later, all contributors' notices and the provenance of
  the original RawHide concept must be preserved.
