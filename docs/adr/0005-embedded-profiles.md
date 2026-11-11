# ADR 0005 — Embed the camera profile registry in the binary

**Status:** accepted
**Date:** 2026-11-08

## Context

RawOxide ships 307 `profile.json` files plus three Adobe lookup blobs (about
1.5 MiB) that describe camera make/model, lens and exposure pools, levels,
calibration, crop geometry, preview geometry and tag conventions. The registry
is static seed data: it changes only when the registry is refreshed.

A runtime data directory would add an installation step, make the binary
non-portable and allow the registry to drift from the binary that uses it.

## Decision

Copy the registry into `assets/profiles/` and compile it into `rawoxide-core`:

- `scripts/embed_profiles.py` copies `profile.json` and `.bin` files and
  regenerates `crates/rawoxide-core/src/profiles_generated.rs`, a table of
  `include_str!`/`include_bytes!` entries keyed by slug and relative path.
- `core::profiles` parses entries lazily into a `OnceLock<BTreeMap<...>>`,
  skipping corrupt entries instead of failing the registry.
- The generated table is committed; refreshing the registry is a source change
  and is reviewed like any other.
- The three Adobe blobs are embedded and consumed by the fidelity/Adobe writer
  path: forward matrices, profile names, calibration signatures, copyright,
  embed policy, HueSat and LookTable data.

## Consequences

- One binary, no runtime data directory, no registry drift.
- Binary size grows by roughly the asset size (1.5 MiB before compression).
- Profile updates require re-running the script and rebuilding; acceptable for
  static reference data.
- Keeping the assets as plain files preserves reviewability and a one-to-one
  mapping with each camera's source data.
