# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Contact the
maintainer privately (the address in `Cargo.toml`/the repository profile) with:

- the affected version/commit,
- a minimal reproducer or malformed file,
- the impact you observed (panic, OOM, out-of-bounds read, etc.).

You will get an acknowledgement and a coordinated disclosure timeline.

## Scope and threat model

RawOxide processes **untrusted DNG/TIFF files** during decode. The decoder must:

- bound all allocations from declared lengths before allocating,
- validate IFD offsets/counts and block offsets against the file size,
- never panic on malformed input (return an error instead),
- respect `--max-bytes` before allocating a payload,
- avoid unbounded decompression (Deflate/LZW output limits per block).

`unsafe` is forbidden in `rawoxide-core`. Dependencies are audited in CI with
`cargo-deny` and `cargo-audit`. Fuzzing the TIFF/IFD parser and the framing
decoder is planned (see `docs/testing.md`); until then, treat unknown files as
hostile and run decode in a constrained process if that matters to you.

## Encryption caveat

The `--key` option is a SHA-256 counter XOR keystream. It provides
confidentiality against casual LSB inspection, **not** authenticated
encryption. A wrong key fails closed on the magic/CRC, but an attacker who can
modify the file can also modify the payload. Use a real encryption tool
(age/GPG) before embedding if the payload needs authenticity or strong
confidentiality.
