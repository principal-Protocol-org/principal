#!/usr/bin/env bash
# Builds every contract so the output WASM is byte-identical regardless of which machine,
# home directory, or checkout path built it. See deployments/mainnet/reproducible-build.md
# for why this is necessary: rustc embeds the absolute source and $CARGO_HOME paths into the
# compiled output (via file!()/#[track_caller], independent of `strip`/toolchain version), so
# two machines building the identical source at different filesystem paths get different bytes
# unless those paths are normalized.
#
# The host OS/architecture is also part of the recipe, not just the toolchain version: measured
# directly, rustc 1.94.1 on macOS arm64 and on Linux arm64 both produce different bytes than
# x86_64 Linux for this target, even for the two contracts that embed no path at all. Run this
# only on x86_64-unknown-linux-gnu (CI's ubuntu-latest runners already are; locally, use
# `docker run --rm -v "$PWD:/src" -w /src --platform linux/amd64 rust:1.94.1-slim ...` or
# equivalent if you're not already on that host).
#
# REMAP_CARGO_HOME_TO / REMAP_CHECKOUT_TO override what the real paths get normalized to.
# Defaults produce fresh, machine-independent bytes for a NEW build you're about to deploy.
# To instead verify today's already-live contracts, which were built before this script existed
# and have no remapping applied, set REMAP_CARGO_HOME_TO to the literal path recorded in their
# bytes (recoverable from the deployed wasm itself -- see reproducible-build.md).
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

host="$(rustc -vV | awk -F': ' '/^host:/ {print $2}')"
if [ "$host" != "x86_64-unknown-linux-gnu" ]; then
  echo "error: this recipe only reproduces known-good bytes on x86_64-unknown-linux-gnu (got: $host)." >&2
  echo "       build inside a pinned linux/amd64 container instead -- see reproducible-build.md." >&2
  exit 1
fi

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
remap_cargo_home_to="${REMAP_CARGO_HOME_TO:-/cargo-home}"
remap_checkout_to="${REMAP_CHECKOUT_TO:-/build}"

export RUSTFLAGS="--remap-path-prefix=${cargo_home}=${remap_cargo_home_to} --remap-path-prefix=${repo_root}=${remap_checkout_to} ${RUSTFLAGS:-}"

echo "host:        $host" >&2
echo "repo root:   $repo_root (remapped to $remap_checkout_to)" >&2
echo "CARGO_HOME:  $cargo_home (remapped to $remap_cargo_home_to)" >&2
echo "RUSTFLAGS:   $RUSTFLAGS" >&2
rustc --version >&2

cargo build --workspace --release --target wasm32v1-none --locked

echo "" >&2
echo "WASM artifacts:" >&2
sha256sum target/wasm32v1-none/release/*.wasm
