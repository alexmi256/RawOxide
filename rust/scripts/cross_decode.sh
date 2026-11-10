#!/usr/bin/env bash
# Cross-decode gate: files written by either implementation must decode in
# the other. Runs offline (synthetic previews) and uses only committed
# fixtures. CI calls this after building the CLI.
set -euo pipefail

rust_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "$rust_root/.." && pwd)"
python_project="$repo_root/RawHide"
payload_dir="$rust_root/fixtures/payloads"
rawhide="${RAWHIDE_BIN:-$rust_root/target/debug/rawhide}"

if [[ -x "$python_project/.venv/bin/python" ]]; then
    python="${PYTHON:-$python_project/.venv/bin/python}"
else
    python="${PYTHON:-python3}"
fi

if [[ ! -x "$rawhide" ]]; then
    echo "missing $rawhide; run 'cargo build -p rawhide-cli' first" >&2
    exit 1
fi

tmp="$(mktemp -d "${TMPDIR:-/tmp}/rawhide-cross.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

run_python() {
    (cd "$python_project" && "$python" stego_dng.py "$@")
}

check() {
    local expected="$1"
    local actual="$2"
    cmp "$expected" "$actual"
    echo "ok: $(basename "$actual")"
}

echo "== Rust encode -> Python decode =="
payload="$payload_dir/payload_3000.bin"
"$rawhide" encode -i "$payload" -o "$tmp/rust.dng" \
    --width 128 --height 96 --seed 5 --thumbnail synthetic
run_python decode -i "$tmp/rust.dng" -o "$tmp/from-rust.bin" --no-progress
check "$payload" "$tmp/from-rust.bin"

echo "== Python encode -> Rust decode =="
run_python encode -i "$payload" -o "$tmp/python.dng" \
    --width 128 --height 96 --seed 5 --thumbnail synthetic --no-progress
"$rawhide" decode -i "$tmp/python.dng" -o "$tmp/from-python.bin" --no-progress
check "$payload" "$tmp/from-python.bin"

echo "== Rust CFA profile -> Python decode =="
"$rawhide" encode -i "$payload" -o "$tmp/rust-cfa.dng" \
    --camera-profile custom_pixel_xl --seed 7 --thumbnail synthetic
run_python decode -i "$tmp/rust-cfa.dng" -o "$tmp/from-rust-cfa.bin" --no-progress
check "$payload" "$tmp/from-rust-cfa.bin"

echo "== Python CFA profile -> Rust decode =="
run_python encode -i "$payload" -o "$tmp/python-cfa.dng" \
    --camera-profile custom_pixel_xl --seed 7 --thumbnail synthetic --no-progress
"$rawhide" decode -i "$tmp/python-cfa.dng" -o "$tmp/from-python-cfa.bin" --no-progress
check "$payload" "$tmp/from-python-cfa.bin"

echo "== Rust multi-frame -> Python decode =="
"$rawhide" encode -i "$payload" -o "$tmp/rust-frames.dng" \
    --width 256 --height 192 --raw-frames 3 --seed 9 --thumbnail synthetic
run_python decode -i "$tmp/rust-frames.dng" -o "$tmp/from-rust-frames.bin" --no-progress
check "$payload" "$tmp/from-rust-frames.bin"

echo "== Python multi-frame -> Rust decode =="
run_python encode -i "$payload" -o "$tmp/python-frames.dng" \
    --width 256 --height 192 --raw-frames 3 --seed 9 --thumbnail synthetic --no-progress
"$rawhide" decode -i "$tmp/python-frames.dng" -o "$tmp/from-python-frames.bin" --no-progress
check "$payload" "$tmp/from-python-frames.bin"

echo "== Rust split set -> Python decode =="
split_payload="$payload_dir/payload_5000.bin"
"$rawhide" encode -i "$split_payload" -o "$tmp/rust-split/vol.dng" \
    --width 128 --height 96 --seed 11 --split-file 3000 \
    --split-file-metadata-seq PageNumber --thumbnail synthetic
mapfile -t rust_chunks < <(find "$tmp/rust-split" -name 'vol*.dng' | sort)
run_python decode -i "${rust_chunks[@]}" \
    --split-file-metadata-seq PageNumber -o "$tmp/from-rust-split.bin" --no-progress
check "$split_payload" "$tmp/from-rust-split.bin"

echo "== Python split set -> Rust decode =="
run_python encode -i "$split_payload" -o "$tmp/python-split/vol.dng" \
    --width 128 --height 96 --seed 11 --split-file 3000 \
    --split-file-metadata-seq PageNumber --thumbnail synthetic --no-progress
mapfile -t python_chunks < <(find "$tmp/python-split" -name 'vol*.dng' | sort)
"$rawhide" decode -i "${python_chunks[@]}" \
    --split-file-metadata-seq PageNumber -o "$tmp/from-python-split.bin" --no-progress
check "$split_payload" "$tmp/from-python-split.bin"

echo "cross-decode matrix passed"
