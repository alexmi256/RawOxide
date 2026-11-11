# ADR 0002 — Native deterministic RNG

**Status:** accepted
**Date:** 2026-11-08

## Context

Cover pixels, metadata and GPS locations are seeded. The values must be
reproducible for a given seed, must stay stable across dependency updates, and
must not depend on hash-map iteration or scheduling.

## Decision

Use explicit native RNG instances with separate purposes:

| seed / RNG | purpose |
|---|---|
| `cover` seed | synthetic raw samples; also drives the synthetic/noise preview fallback |
| `metadata` seed | EXIF values and serials; initializes separate `location` and `makernote` `Rng64` instances |
| `location` seed | shared GPS sampling for split sets; a plain encode uses the metadata randomizer's own location instance |
| split chunk seed | per-chunk cover + metadata seed; the chunk-set UUID uses `uuid` v4 unless overridden |

Implementation: local `SplitMix64` (`Rng64`) instances seeded with the caller
seed or fresh entropy. For split sets the chunk seed drives both cover and
metadata; unseeded split chunks draw a fresh seed each. The split UUID is not an
RNG stream. Never share one RNG instance between fields whose draw order could
change.

Rules:

- Per-frame and per-chunk seeds keep runs reproducible when a seed is supplied.
- Parallel tile generation, when added, must seed each band/tile so output is
  independent of scheduling.
- No `HashMap` iteration in write paths; use `BTreeMap` or sorted keys.
- Record the effective cover/metadata/location seeds in the debug sidecar so a
  run is reproducible.
- Pin golden hashes for RawOxide output.

## Consequences

- Reproducibility tests are fast and stable across dependency updates.
- Output is not expected to match another implementation's RNG stream.
- If cross-tool seed parity is ever required, it becomes a separate
  compatibility mode with its own ADR and is explicitly best-effort.
