# Reproducible build recipe

This is the recipe referenced by `docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md`: steps that
rebuild the exact WASM bytes installed on-chain, so anyone can verify the deployed contract
code against the source without trusting us.

## Root cause of the 2026-11-22 non-reproducibility report

The dApp team reported rebuilding commit `3af9388` and getting 0 of 12 WASM hashes
matching the deployed bytes, with Permissioning's 12-byte diff traced to two functions emitted
in reverse order — i.e. identical source, different machine-code layout.

Verified root cause: **the Rust toolchain was never pinned**, anywhere.

- `Cargo.lock` *is* committed (dependency versions were already reproducible).
- No `rust-toolchain.toml` existed at the repo root.
- CI (`.github/workflows/ci.yml`) builds against `dtolnay/rust-toolchain@stable`, i.e.
  whatever "stable" resolves to on the day the workflow runs — it floats too.

So the only reason any one machine reproduces the deployed bytes is if it happens to already
have the exact rustc/LLVM build used at deploy time (`rustc 1.94.1`, bundled LLVM 21.1.8)
installed. A different rustc point release — even with identical source and identical
dependency versions — changes LLVM's codegen-unit internals enough to reorder emitted
functions. That reordering is exactly the "two functions swapped" symptom reported.

**Fix:** `rust-toolchain.toml` added at the repository root, pinning
`channel = "1.94.1"` with `targets = ["wasm32v1-none", "wasm32-unknown-unknown"]`. Any
machine with `rustup` installed will now auto-switch to this exact toolchain the moment it
runs a cargo command inside the repo — no manual version matching required.

## Verification performed

Rebuilt commit `3af9388` from a clean `git worktree` (isolated from any already-built
`target/` directory) using the documented recipe, after confirming `rustc --version`
resolved to `1.94.1 (e408947bf 2026-03-25)` via the toolchain pin:

```
git worktree add /tmp/repro-check 3af9388
cd /tmp/repro-check
cargo build --workspace --release --target wasm32v1-none
sha256sum target/wasm32v1-none/release/*.wasm
```

Result: **12/12 WASM artifacts matched the deployed hashes exactly**, byte for byte,
including `principal_permissioning.wasm`:

| Contract | SHA-256 (rebuilt) | Matches deployed? |
|---|---|---|
| `principal_manager.wasm` | `e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115` | yes |
| `principal_market_config.wasm` | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` | yes |
| `principal_market_factory.wasm` | `6c21fc0e78f3064f67ae30924b8f9ca0223fbb61a20ac640cceca7b152456759` | yes |
| `principal_market_pool.wasm` | `52285a25bbf372d0e5d22ba565b76af6a7e832422df83b6a4809de8adf3aa112` | yes |
| `principal_oracle_adapter.wasm` | `8581bbba82bb57f5c5c9c00adf98b1691ad20821840e55a67ad8d7ecace0c1b0` | yes |
| `principal_permissioning.wasm` | `4437468dbbda6d86a115f188de877135b0b5a400b69ad7e9444cf269c54f969c` | yes |
| `principal_pt_token.wasm` | `db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2` | yes |
| `principal_recovery_escrow.wasm` | `dadea71fcfb84adcfb4691f472551f490cf440c5aaae5c6b5e5d4bb6f61441e8` | yes |
| `principal_risk_control.wasm` | `ac75508d24c5457de7db1106bf8b5e9b62ea436f3dfd1a309fda5138110c2526` | yes |
| `principal_router.wasm` | `2a0997317651edb0f7e2bd753570b03a88462aa6d216c14b06ebec246bb3c1c2` | yes |
| `principal_sy_wrapper.wasm` | `be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a` | yes |
| `principal_yt_token.wasm` | `c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654` | yes |

These are the same 12 hashes already recorded in `docs/MAINNET_DEPLOYMENT_EVIDENCE.md` §8
(captured at source commit `4b5bc2b`) and cross-checked against each deployed contract's
on-chain WASM hash at upload time. No contract source changed between `4b5bc2b` and
`3af9388` (that range only added self-service tooling scripts), so the two commits are
expected — and now confirmed — to produce identical bytes.

## How to reproduce this yourself

1. Install `rustup` if you don't already have it (https://rustup.rs).
2. `git clone` this repository (or `git worktree add` a given commit) and `cd` into it.
3. Run `rustc --version` — because of the committed `rust-toolchain.toml`, `rustup` will
   transparently download and switch to `1.94.1 (e408947bf 2026-03-25)` if you don't already
   have it installed. Confirm the version string matches exactly before proceeding.
4. `cargo build --workspace --release --target wasm32v1-none`
5. `sha256sum target/wasm32v1-none/release/*.wasm`
6. Compare against the table above (or the equivalent table in
   `docs/MAINNET_DEPLOYMENT_EVIDENCE.md` §8). Every hash should match exactly.

## Residual known limitation

`rust-toolchain.toml` pins the *compiler*. It does not pin the host OS/architecture. The
verification above was performed on `x86_64-unknown-linux-gnu`; the Rust/LLVM wasm backend is
not known to vary output by host platform for this target, but this has only been confirmed
on Linux x86_64 so far — if a future rebuild on a different host (e.g. macOS
`aarch64-apple-darwin`) ever produces a mismatch despite an identical pinned rustc version,
that would be new information worth recording here rather than assuming the recipe above is
incomplete.
