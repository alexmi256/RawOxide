#!/usr/bin/env python3
"""Compare the written-tag inventory of two Rust/Python debug sidecars."""

from __future__ import annotations

import json
import sys


def inventory(path: str) -> dict[str, object]:
    with open(path, encoding="utf-8") as handle:
        document = json.load(handle)
    file = document["file"]
    result: dict[str, object] = {}
    for part in ("ifd0", "exif", "gps"):
        result[part] = {int(tag["tag"]) for tag in file.get(part, ())}
    result["subifds"] = [
        {int(tag["tag"]) for tag in ifd}
        for ifd in file.get("subifds", ())
    ]
    return result


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: tag_inventory_parity.py RUST.json PYTHON.json")
    rust = inventory(sys.argv[1])
    python = inventory(sys.argv[2])
    failures = 0

    for part in ("ifd0", "exif", "gps"):
        for tag in sorted(rust[part] ^ python[part]):
            side = "rust-only" if tag in rust[part] else "python-only"
            print(f"{part} tag {tag}: {side}")
            failures += 1

    rust_subifds = rust["subifds"]
    python_subifds = python["subifds"]
    if len(rust_subifds) != len(python_subifds):
        print(f"subifd count: rust={len(rust_subifds)} python={len(python_subifds)}")
        failures += 1
    for index, (rust_ifd, python_ifd) in enumerate(
        zip(rust_subifds, python_subifds)
    ):
        for tag in sorted(rust_ifd ^ python_ifd):
            side = "rust-only" if tag in rust_ifd else "python-only"
            print(f"subifd {index} tag {tag}: {side}")
            failures += 1

    if failures:
        return 1
    print("tag inventory matches")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
