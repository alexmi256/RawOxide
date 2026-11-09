# ADR 0002 — Native deterministic RNG, no Python/NumPy parity

**Status:** accepted
**Date:** 2026-11-08

## Context

Cover pixels, metadata and GPS locations are seeded. The Python implementation
uses NumPy `default_rng` (PCG64) for covers and Python `random.Random`
(MT19937) for metadata. Reproducing those streams exactly in Rust would be a
fragile side project (see ADR 0001) and is not needed for decode compatibility.

## Decision

Use an explicit native RNG with named, independent streams:

| stream | purpose |
|---|---|
| `cover` | synthetic raw samples (per band/frame) |
| `metadata` | EXIF values and serials |
| `thumbnail` | synthetic/noise fallback |
| `location` | GPS city/land sampling |
| `split` | split-set UUID/auxiliary draws |

Preferred implementation: `rand_chacha::ChaCha8Rng` or `rand_pcg::Pcg64`, seeded
with a documented derivation (e.g. `seed ^ domain_constant`). Never share one
stream between fields whose draw order could change.

Rules:

- Per-band/per-tile seeds so parallel output is scheduling-independent.
- No `HashMap` iteration in write paths; use `BTreeMap` or sorted keys.
- Record the effective seed/stream in the debug sidecar so a native run is
  reproducible.
- Pin golden hashes for Rust output; use Python fixtures only for decode tests.

## Consequences

- Rust output differs from Python for the same seed. This is documented.
- Reproducibility tests are faster and stable across dependency updates.
- If seed parity is ever required, it becomes a separate compatibility mode
  with its own ADR and is explicitly best-effort.
