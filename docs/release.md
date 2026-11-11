# Release process

Releases are automated by [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Merging to `main` produces a beta pre-release, and the first merge on a new
version adds the official release at the same time.

## Channels

| trigger | what is published |
|---|---|
| push to `main` | beta pre-release `vX.Y.Z-beta.<run>`, one per workflow run |
| push to `main`, first time the version has no tag | official release `vX.Y.Z` (same commit as the beta) |
| push of a `v*` tag | official release for that tag; the tag must equal the workspace version |
| manual `workflow_dispatch` | build only: binaries are kept as workflow artifacts, nothing is published |

The workflow reads the version from `[workspace.package]` in `Cargo.toml`. On a
`main` push it checks whether `vX.Y.Z` already exists; if it does, only the beta
is published. That means the official release is created exactly once per version,
even when several commits land before the next bump.

Betas are ordinary GitHub pre-releases: they are never marked "latest" and do
not move the release badge. Each completed run gets its own tag
(`v0.1.0-beta.42`, numbered by workflow run), so a build can be traced back to
the commit that produced it. Delete old pre-releases and tags when they are no
longer useful.

Merges that arrive while a release is already building are queued, and only the
newest queued run survives: a burst of rapid merges can skip the beta for an
intermediate commit. That commit is still contained in the next beta, so do not
re-run an old workflow to backfill it. Explicit `v*` tag pushes are not affected.

Both channels build the same matrix:

- Linux x86-64, ARM64 (aarch64) and ARMv7 hard-float,
- macOS Intel and Apple Silicon,
- Windows x86-64 and ARM64,

plus `SHA256SUMS`, verified by `cargo test --locked --workspace --all-features`
before anything is published. The release is only created after every build
succeeds. If a build fails, the tag is not created and the next `main` push
retries.

> The beta binaries report the base version (`rawoxide --version` prints the
> workspace version, without the `-beta.N` suffix). Use the release name or
> `SHA256SUMS` to tell betas apart.

## Cutting a release

1. Update `[workspace.package].version` in `Cargo.toml`; both crates inherit it.
2. Run `cargo check --workspace` and commit the updated `Cargo.lock` with the
   bump. The release builds use `--locked`, so a stale lock file fails them.
3. Move the `Unreleased` entries in `CHANGELOG.md` under the new version.
4. Run the local gates:
   ```bash
   just ci
   # Optional interoperability gate, needs a reference checkout:
   RAWHIDE_ORACLE=./RawHide just cross-decode
   ```
5. Merge to `main` and wait for the `Release` workflow.

The merge publishes the beta `vX.Y.Z-beta.<run>` and, because `vX.Y.Z` does not
exist yet, the official `vX.Y.Z` release. Later merges on the same version only
publish betas.

You can still release from a tag explicitly (for example from a maintenance
branch):

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

The tag must match the workspace version or the workflow fails before building.

## If a release goes wrong

- Bad code in a just-merged version: bump the patch version and merge again.
  Tags are immutable from the workflow's point of view, so the version must
  change.
- Bad assets on a release: if the publish job failed, choose **Re-run failed
  jobs**, not **Re-run all jobs**. A full re-run repeats `prepare`, which sees
  the tag the first attempt already created and therefore skips the official
  publish; re-running the failed jobs reuses the original plan and updates the
  release in place, keeping its tag. If the job succeeded but the assets are
  wrong, delete the release **and** its tag, then re-run the workflow: the tag
  is the only thing that marks a version as released.
- Unwanted beta: delete the pre-release and its tag on GitHub. The next `main`
  push still works because beta tags are not used for the official check.
- Official release created by mistake before the version was final: delete the
  release and the `vX.Y.Z` tag. The next `main` push republishes it.

## Verify a download

```bash
sha256sum -c SHA256SUMS
```

On macOS use `shasum -a 256 -c SHA256SUMS`.

## Build notes

The Linux ARM binaries are cross-compiled with `cargo-zigbuild` and a pinned Zig
toolchain. `rawoxide-linux-armv7` targets ARMv7-A with hard-float; older
distributions such as Bullseye may report missing GLIBC symbol versions. The
macOS and Windows ARM64 binaries are built natively on ARM64 runners.

The macOS binaries are currently unsigned/unnotarized, so Gatekeeper may require
an explicit override. Windows binaries are not code-signed either. Treat these
as release-policy limitations until signing credentials are available.

## Release notes

The workflow generates notes from the tag history. Official releases compare
against the previous stable tag, so betas do not split the changelog. Add
migration notes for any CLI or on-disk behavior change, especially split
markers, BigTIFF output, metadata tag changes or new minimum Rust versions.
