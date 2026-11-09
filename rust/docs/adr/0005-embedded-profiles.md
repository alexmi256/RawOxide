# ADR 0005 — Embed the camera profile registry in the binary

## Context

The Python package ships 307 `profile.json` files plus three Adobe lookup blobs
under `stegodng/profiles/` (about 1.5 MiB). The Rust port must remain a single
self-contained binary with no Python runtime and no install-time data directory.
The registry is static seed data: it changes only when the Python registry is
regenerated.

## Decision

Copy the registry into `rust/assets/profiles/` and compile it into
`rawhide-core`:

- `scripts/embed_profiles.py` copies `profile.json` and `.bin` files and
  regenerates `crates/rawhide-core/src/profiles_generated.rs`, a table of
  `include_str!`/`include_bytes!` entries keyed by slug/relative path.
- `core::profiles` parses entries lazily into a `OnceLock<BTreeMap<...>>`,
  skipping corrupt entries exactly like the Python registry.
- The generated table is committed; regenerating is a source change and is
  reviewed like any other.

The three Adobe blobs are embedded but not yet consumed; the Adobe writer path
lands with the fidelity/Adobe work.

## Consequences

- One binary, no runtime data directory, and the registry cannot drift from the
  commit that built the binary.
- Binary size grows by roughly the asset size (1.5 MiB before compression).
- Profile updates require re-running the script and rebuilding; this is
  acceptable for static reference data.
- Keeping the assets as plain files (rather than a generated Rust literal)
  preserves reviewability and the one-to-one mapping with the Python registry.
