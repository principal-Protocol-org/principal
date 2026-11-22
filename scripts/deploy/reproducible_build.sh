#!/usr/bin/env bash
# Builds every contract so the output WASM is byte-identical regardless of which machine,
# home directory, or checkout path built it. See deployments/mainnet/reproducible-build.md
# for why this is necessary: rustc embeds the absolute source and $CARGO_HOME paths into the
# compiled output (via file!()/#[track_caller], independent of `strip`/toolchain version), so
# two machines building the identical source at different filesystem paths get different bytes
# unless those paths are normalized.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

cargo_home="${CARGO_HOME:-$HOME/.cargo}"

export RUSTFLAGS="--remap-path-prefix=${cargo_home}=/cargo-home --remap-path-prefix=${repo_root}=/build ${RUSTFLAGS:-}"

echo "repo root:   $repo_root" >&2
echo "CARGO_HOME:  $cargo_home" >&2
echo "RUSTFLAGS:   $RUSTFLAGS" >&2
rustc --version >&2

cargo build --workspace --release --target wasm32v1-none --locked

echo "" >&2
echo "WASM artifacts:" >&2
sha256sum target/wasm32v1-none/release/*.wasm
