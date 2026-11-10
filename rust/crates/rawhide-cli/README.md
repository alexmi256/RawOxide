# rawhide-cli

The `rawhide` command-line interface: `capacity`, `encode`, `decode` and
`gen-tiff`, including split chunk sets, camera-profile selection, preview
sources, progress output and the optional JSON debug sidecar.

The CLI owns argument parsing, progress presentation, user-facing messages and
exit codes; the encoding, decoding and metadata logic lives in `rawhide-core`.

See the workspace [README](../../README.md),
[port plan](../../docs/port-plan.md) and
[porting matrix](../../docs/porting-matrix.md).
