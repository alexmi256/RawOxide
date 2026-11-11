# Release checklist

## Before tagging

1. Update the workspace version in `Cargo.toml`; both crates inherit it.
2. Run the local gates:
   ```bash
   just ci
   just cross-decode
   cargo deny check
   ```
3. Confirm the working tree is clean and the version in `Cargo.lock` matches.
4. Merge to the release branch and wait for CI to pass.

## Publish

Push a tag matching the workspace version:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

`.github/workflows/rust-release.yml` then:

- runs `cargo test --locked --workspace --all-features`,
- builds `rawhide` for Linux, macOS and Windows,
- stages platform names:
  - `rawhide-linux-x86_64`
  - `rawhide-macos`
  - `rawhide-windows-x86_64.exe`
- generates `SHA256SUMS` and attaches the binaries plus checksums to the GitHub
  release.

A `workflow_dispatch` run builds and uploads artifacts without publishing.

## Verify a download

```bash
sha256sum -c SHA256SUMS
```

On macOS use `shasum -a 256 -c SHA256SUMS`.

The macOS binary is currently unsigned/unnotarized, so Gatekeeper may require
an explicit override. Windows binaries are not code-signed either. Treat these
as release-policy limitations until signing credentials are available.

## Release notes

The workflow generates release notes from the tag history. Add migration notes
for any CLI or on-disk behavior change, especially split markers, BigTIFF
output, metadata tag changes or new minimum Rust versions.
