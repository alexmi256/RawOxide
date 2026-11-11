# Fixtures

Committed, deterministic artefacts used by the test suite. They let the tests
run without external tools, network access or a reference implementation.

Regenerate:

```bash
python3 scripts/gen_fixtures.py --force
# or: just fixtures
```

The generator reads from a checkout of the original RawHide implementation
(`./RawHide` by default; set `RAWHIDE_ORACLE` to override) and re-execs into
its virtualenv when one is present. Review the diff before
committing. `manifest.json` records sizes, SHA-256 hashes and generation
provenance; `golden/environment.json` records the tool versions.

## Contents

| path | purpose |
|---|---|
| `payloads/*.bin` | deterministic plaintext payloads used by the DNG fixtures |
| `dng/linear_256x192_16bit_1plane.dng` | default path, 16-bit linear, 1 plane |
| `dng/linear_128x96_8bit_deflate.dng` | 8-bit linear, Adobe Deflate |
| `dng/cfa_128x96_14bit_2planes.dng` | packed 14-bit CFA, 2 planes |
| `dng/linear_128x96_16bit_4planes_keyed.dng` | keyed payload, 4 planes |
| `dng/linear_128x96_16bit_3frames.dng` | three raw SubIFDs |
| `dng/split0001.dng`, `dng/split0002.dng` | split set with shared UUID |
| `golden/framing.json` | framing and keystream vectors |
| `golden/codec.json` | LSB embed mapping (cover before/after) |
| `golden/capacity.json` | capacity and size-formatting vectors |
| `golden/environment.json` | tool versions used to generate the set |

## Rules

- Do not hand-edit a DNG or payload.
- Do not add large fixtures; keep the set under a few MiB.
- If a generator change alters output intentionally, regenerate with `--force`
  and call it out in the pull request.
- Fixtures are committed explicitly; the reference checkout's own ignore rules do
  not apply here.
