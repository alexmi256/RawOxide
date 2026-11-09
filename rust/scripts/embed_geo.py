#!/usr/bin/env python3
"""Copy the Python geo table into `assets/geo/zones.bin`.

Usage:
    python3 scripts/embed_geo.py [--python-root ../RawHide]

The table is static public-domain tz data; re-run this when the Python asset
changes and commit the result. The script refuses blobs without the expected
`STGEO1` magic.
"""

from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "assets" / "geo" / "zones.bin"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-root", type=Path, default=ROOT.parent / "RawHide")
    args = parser.parse_args()
    source = args.python_root / "stegodng" / "geo" / "zones.bin"
    if not source.is_file():
        parser.error(f"no geo data at {source}")

    import zlib

    blob = zlib.decompress(source.read_bytes())
    if not blob.startswith(b"STGEO1"):
        parser.error(f"unexpected geo header in {source}")
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_bytes(source.read_bytes())
    print(f"copied {source} -> {TARGET} ({TARGET.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
