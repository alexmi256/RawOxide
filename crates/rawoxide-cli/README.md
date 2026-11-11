# rawoxide-cli

The `rawoxide` command-line interface: `capacity`, `encode`, `decode` and
`gen-tiff`, including split chunk sets, camera-profile selection, preview
sources, progress output and the optional JSON debug sidecar.

The CLI owns argument parsing, progress presentation, user-facing messages and
exit codes; the encoding, decoding and metadata logic lives in `rawoxide-core`.

See the workspace [README](../../README.md),
[format reference](../../docs/format.md) and
[architecture](../../docs/architecture.md).
