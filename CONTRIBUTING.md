# Contributing

Thanks for helping with RawOxide. The project is a Rust DNG steganography
toolkit; the [README](README.md) describes the user-facing behaviour and
[`docs/format.md`](docs/format.md) the on-disk format.

## Setup

```bash
# Rust 1.99.1+; rustup is recommended.
rustup component add rustfmt clippy llvm-tools-preview
cargo install --locked cargo-nextest cargo-llvm-cov cargo-deny cargo-audit
```

`just` is optional but every common task has a recipe (`just --list`). Tests are
hermetic and use the committed fixtures under `fixtures/`; no network or
external tools are required.

## Before opening a pull request

```bash
just fmt
just lint
just test
just test-doc
just deny
just audit
just doc
```

Or run the full local CI:

```bash
just ci
```

For changes that touch the codec, writer or a parser, also run:

```bash
just coverage
just bench        # when performance-sensitive
just mutants      # optional, slow
```

If you have a checkout of the original RawHide implementation, the optional
interoperability gate should pass too:

```bash
RAWHIDE_ORACLE=./RawHide just cross-decode
```

## Code style

- `rustfmt` is authoritative; do not hand-format around it.
- Clippy runs with `-D warnings` and the workspace lint set (`pedantic`,
  `nursery`, `cargo`). If a lint is genuinely wrong for a local case, use a
  narrowly scoped `#[allow(...)]` with a comment, not a crate-wide allow.
- `unsafe` is forbidden in `rawoxide-core`. If a future feature truly needs it
  (for example mmap), isolate it behind a small, documented module and open a
  discussion first.
- Public items need rustdoc. `RUSTDOCFLAGS="-D warnings"` is enforced in CI.
- Errors: use `thiserror` in the library and `anyhow` only at the CLI boundary.
  Never panic on user input or malformed files; return an error with context.
- Determinism: no `HashMap` iteration in file-writing paths. Use `BTreeMap` or
  sort keys. Keep seeded output stable within this implementation.

## Tests

- Unit tests live next to the code; integration tests in `tests/`.
- Property tests cover codecs/parsers; CLI/JSON assertions (`assert_cmd` +
  `serde_json`) cover output and sidecars; golden fixtures cover decode and the
  wire format.
- Add a test for every bug fix. For format changes, add a round-trip and, when
  relevant, an interoperability fixture.
- Keep tests hermetic: no network, no writes outside `tempfile` directories.

## Commit messages

Use imperative, scoped subjects, for example:

```text
codec: fix plane mapping for odd sample counts
tiff: write tiled LinearRaw SubIFDs
ci: add llvm-cov coverage job
```

Keep the body focused on *why*; the diff shows *what*.

## License

By contributing you agree that your contribution is licensed under the
project's MIT license.
