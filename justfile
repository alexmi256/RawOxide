set shell := ["bash", "-euo", "pipefail", "-c"]

oracle_dir := env("RAWHIDE_ORACLE", "RawHide")

default: ci

# Fetch dependencies and print the active toolchain.
setup:
    cargo fetch
    rustc --version
    cargo --version

# Install the developer tools used by CI (user-level; no sudo).
tools:
    cargo install --locked cargo-nextest cargo-llvm-cov cargo-deny cargo-audit cargo-mutants cargo-zigbuild

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

check:
    cargo check --workspace --all-targets --all-features

test:
    cargo nextest run --workspace --all-features

test-doc:
    cargo test --workspace --doc --all-features

test-all: test test-doc

coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${LLVM_PROFDATA:-}" ] && command -v llvm-profdata >/dev/null 2>&1; then
        export LLVM_PROFDATA="$(command -v llvm-profdata)"
        export LLVM_COV="$(command -v llvm-cov)"
    fi
    cargo llvm-cov nextest --workspace --all-features --lcov --output-path lcov.info
    echo "wrote lcov.info"

coverage-html:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${LLVM_PROFDATA:-}" ] && command -v llvm-profdata >/dev/null 2>&1; then
        export LLVM_PROFDATA="$(command -v llvm-profdata)"
        export LLVM_COV="$(command -v llvm-cov)"
    fi
    cargo llvm-cov nextest --workspace --all-features --html
    echo "open target/llvm-cov/html/index.html"

deny:
    cargo deny check

audit:
    cargo audit

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

bench:
    cargo bench -p rawoxide-core

mutants:
    cargo mutants --workspace

machete:
    cargo machete

typos:
    typos

# Regenerate committed golden vectors and DNG fixtures with the Python reference.
fixtures:
    python3 scripts/gen_fixtures.py

# Run the Python reference test suite (developer-only; needs a RawHide/ checkout).
oracle:
    {{oracle_dir}}/.venv/bin/python -m pytest {{oracle_dir}}/tests -q

# Interoperability gate: encode with one implementation, decode with the other.
cross-decode:
    cargo build -p rawoxide-cli
    ./scripts/cross_decode.sh

# Cross-compile the CLI for ARM Linux (needs Zig and cargo-zigbuild).
build-arm64:
    rustup target add aarch64-unknown-linux-gnu
    cargo zigbuild --locked --release -p rawoxide-cli --target aarch64-unknown-linux-gnu

build-arm32:
    rustup target add armv7-unknown-linux-gnueabihf
    cargo zigbuild --locked --release -p rawoxide-cli --target armv7-unknown-linux-gnueabihf

run:
    cargo run -p rawoxide-cli --

ci: fmt-check lint check test test-doc deny audit doc

clean:
    cargo clean
