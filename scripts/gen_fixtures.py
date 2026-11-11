#!/usr/bin/env python3
"""Generate the committed fixtures with the reference implementation.

The fixtures are small, deterministic DNGs plus JSON golden vectors. They are
reviewed like source and committed so the Rust tests stay hermetic (no Python,
NumPy or network needed at test time).

Usage:
    python3 scripts/gen_fixtures.py [--force]

The script re-execs itself with the reference checkout's venv if the current
interpreter lacks NumPy/Pillow/tifffile. Set RAWHIDE_ORACLE to point at the
checkout; it defaults to `RawHide/` in the repository root.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent.parent / "fixtures"
PROJECT_ROOT = FIXTURES.parent
PYTHON_PROJECT = Path(os.environ.get("RAWHIDE_ORACLE", PROJECT_ROOT / "RawHide"))


def ensure_oracle_interpreter() -> None:
    """Re-exec with the reference checkout's venv when available."""
    venv_python = PYTHON_PROJECT / ".venv" / "bin" / "python"
    venv_prefix = str(PYTHON_PROJECT / ".venv")
    if venv_python.is_file() and sys.prefix != venv_prefix:
        os.execv(str(venv_python), [str(venv_python), __file__, *sys.argv[1:]])
    try:
        import numpy  # noqa: F401
        import PIL  # noqa: F401
        import tifffile  # noqa: F401
    except ImportError:
        raise SystemExit(
            "The Python oracle dependencies are missing. Run this script with "
            f"{venv_python} (or point RAWHIDE_ORACLE at a prepared checkout)."
        )


ensure_oracle_interpreter()
sys.path.insert(0, str(PYTHON_PROJECT))

import numpy  # noqa: E402
import PIL  # noqa: E402
import tifffile  # noqa: E402

import stego_dng  # noqa: E402
import stegodng  # noqa: E402
from stegodng.codec import LsbCodec, format_kb, format_kb_hi, format_kb_lo  # noqa: E402
from stegodng.framing import PayloadFrame  # noqa: E402


def deterministic_bytes(length: int, salt: int = 0) -> bytes:
    """A repeatable, reasonably incompressible payload."""
    return bytes(((index * 37 + salt * 11 + 17) & 0xFF) for index in range(length))


def write_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def dng(
    name: str,
    payload: bytes,
    *,
    width: int,
    height: int,
    seed: int,
    **kwargs: object,
) -> Path:
    output = FIXTURES / "dng" / name
    output.parent.mkdir(parents=True, exist_ok=True)
    stego_dng.encode(
        payload,
        str(output),
        width=width,
        height=height,
        seed=seed,
        thumbnail="synthetic",
        progress=False,
        **kwargs,
    )
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--force",
        action="store_true",
        help="overwrite existing fixtures (review the diff before committing)",
    )
    args = parser.parse_args()

    FIXTURES.mkdir(parents=True, exist_ok=True)
    if (FIXTURES / "manifest.json").exists() and not args.force:
        raise SystemExit(
            "Fixtures already exist; pass --force to regenerate them. "
            f"Manifest: {FIXTURES / 'manifest.json'}"
        )

    payload_1000 = deterministic_bytes(1000, salt=1)
    payload_3000 = deterministic_bytes(3000, salt=2)
    payload_5000 = deterministic_bytes(5000, salt=3)
    payload_8000 = deterministic_bytes(8000, salt=4)
    for name, data in {
        "payload_1000.bin": payload_1000,
        "payload_3000.bin": payload_3000,
        "payload_5000.bin": payload_5000,
        "payload_8000.bin": payload_8000,
    }.items():
        write_bytes(FIXTURES / "payloads" / name, data)

    dng(
        "linear_256x192_16bit_1plane.dng",
        payload_3000,
        width=256,
        height=192,
        seed=7,
        lsb_planes=1,
        bit_depth=16,
        mode="linear",
    )
    dng(
        "linear_128x96_8bit_deflate.dng",
        payload_1000,
        width=128,
        height=96,
        seed=8,
        lsb_planes=1,
        bit_depth=8,
        mode="linear",
        compression="adobe_deflate",
    )
    dng(
        "cfa_128x96_14bit_2planes.dng",
        payload_1000,
        width=128,
        height=96,
        seed=9,
        lsb_planes=2,
        bit_depth=14,
        mode="cfa",
    )
    dng(
        "linear_128x96_16bit_4planes_keyed.dng",
        payload_1000,
        width=128,
        height=96,
        seed=10,
        lsb_planes=4,
        bit_depth=16,
        mode="linear",
        key=b"fixture-key",
    )
    dng(
        "linear_128x96_16bit_3frames.dng",
        payload_8000,
        width=128,
        height=96,
        seed=11,
        lsb_planes=1,
        bit_depth=16,
        mode="linear",
        frames=3,
    )

    split_infos = stego_dng.encode_split(
        payload_5000,
        str(FIXTURES / "dng" / "split.dng"),
        3000,
        width=128,
        height=96,
        seed=12,
        thumbnail="synthetic",
        progress=False,
        split_id="fixture-split-0001",
        split_id_field="ImageUniqueID",
        split_seq_field="ImageNumber",
    )
    if len(split_infos) != 2:
        raise SystemExit(f"expected a 2-chunk split fixture, got {len(split_infos)}")

    key = b"secret"
    frame = PayloadFrame.pack(b"rawoxide-golden", key)
    write_json(
        FIXTURES / "golden" / "framing.json",
        {
            "payload_hex": b"rawoxide-golden".hex(),
            "key_hex": key.hex(),
            "frame_keyed_hex": frame.hex(),
            "frame_plain_hex": PayloadFrame.pack(b"rawoxide-golden", None).hex(),
            "frame_empty_hex": PayloadFrame.pack(b"", None).hex(),
            "keystream_0_64_hex": PayloadFrame._keystream_range(key, 0, 64).hex(),
            "header_len": PayloadFrame.HEADER_LEN,
        },
    )

    capacity_cases = []
    for n_samples, planes, frames in [
        (9_437_184, 1, 1),
        (9_437_184, 4, 1),
        (9_437_184, 1, 2),
        (3_145_728, 2, 1),
        (1_179_630, 1, 1),
    ]:
        capacity = stego_dng.capacity_bytes_total(n_samples, planes, frames)
        capacity_cases.append(
            {
                "n_samples_per_frame": n_samples,
                "lsb_planes": planes,
                "frames": frames,
                "capacity": capacity,
                "format_kb": format_kb(capacity),
                "format_kb_hi": format_kb_hi(capacity),
                "format_kb_lo": format_kb_lo(capacity),
            }
        )
    write_json(FIXTURES / "golden" / "capacity.json", {"cases": capacity_cases})

    write_json(
        FIXTURES / "golden" / "environment.json",
        {
            "python": sys.version,
            "stegodng": getattr(stegodng, "__version__", None),
            "numpy": numpy.__version__,
            "pillow": PIL.__version__,
            "tifffile": tifffile.__version__,
            "command": "python3 scripts/gen_fixtures.py --force",
        },
    )

    cover = numpy.array(
        [((index * 257 + 3) & 0xFFFF) for index in range(128)],
        dtype=numpy.uint16,
    )
    codec_frame = PayloadFrame.pack(b"abc", None)
    embedded = LsbCodec.stripe_frames(
        [cover.copy()], codec_frame, 2, progress=False
    )[0]
    write_json(
        FIXTURES / "golden" / "codec.json",
        {
            "cover": [int(value) for value in cover],
            "frame_hex": codec_frame.hex(),
            "lsb_planes": 2,
            "embedded": [int(value) for value in embedded],
        },
    )

    manifest = []
    for path in sorted(FIXTURES.rglob("*")):
        if not path.is_file() or path.name in {"manifest.json", "README.md"}:
            continue
        data = path.read_bytes()
        manifest.append(
            {
                "path": path.relative_to(FIXTURES).as_posix(),
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    write_json(
        FIXTURES / "manifest.json",
        {
            "generated_by": "RawHide Python reference (stegodng)",
            "note": "Regenerate with scripts/gen_fixtures.py --force; review the diff.",
            "files": manifest,
        },
    )

    total = sum(item["bytes"] for item in manifest)
    print(f"wrote {len(manifest)} fixtures ({total / 1024:.1f} KiB) under {FIXTURES}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
