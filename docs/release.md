# Release checklist

## Before tagging

1. Update the workspace version in `Cargo.toml`; both crates inherit it.
2. Run the local gates:
   ```bash
   just ci
   # Optional interoperability gate, needs a reference checkout:
   RAWHIDE_ORACLE=./RawHide just cross-decode
   ```
3. Confirm the working tree is clean and the version in `Cargo.lock` matches.
4. Merge to the release branch and wait for CI to pass.

## Publish

Push a tag matching the workspace version:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

`.github/workflows/release.yml` then:

- runs `cargo test --locked --workspace --all-features`,
- builds `rawoxide` for Linux (x86-64, ARM64 and ARMv7 hard-float), macOS
  (Intel and Apple Silicon) and Windows (x86-64 and ARM64),
- stages platform names:
  - `rawoxide-linux-x86_64`
  - `rawoxide-linux-aarch64`
  - `rawoxide-linux-armv7`
  - `rawoxide-macos-x86_64`
  - `rawoxide-macos-aarch64`
  - `rawoxide-windows-x86_64.exe`
  - `rawoxide-windows-aarch64.exe`
- generates `SHA256SUMS` and attaches the binaries plus checksums to the GitHub
  release.

The Linux ARM binaries are cross-compiled with the GNU toolchains
(`aarch64-linux-gnu-gcc` and `arm-linux-gnueabihf-gcc`) and the matching target
libc headers. The `rawoxide-linux-armv7` build targets ARMv7-A with hard-float.
They are linked against the CI cross sysroot and require a recent glibc; older
releases such as Bullseye may report missing GLIBC symbol versions. The macOS
and Windows ARM64 binaries are built natively on ARM64 runners.

A `workflow_dispatch` run builds and uploads artifacts without publishing.

## Verify a download

```bash
sha256sum -c SHA256SUMS
```

On macOS use `shasum -a 256 -c SHA256SUMS`.

The macOS binaries are currently unsigned/unnotarized, so Gatekeeper may require
an explicit override. Windows binaries are not code-signed either. Treat these
as release-policy limitations until signing credentials are available.

## Release notes

The workflow generates release notes from the tag history. Add migration notes
for any CLI or on-disk behavior change, especially split markers, BigTIFF
output, metadata tag changes or new minimum Rust versions.
