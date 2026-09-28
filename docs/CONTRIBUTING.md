# Contributing to Principal Protocol

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| Rust | stable (≥ 1.84) | Contract compilation |
| `wasm32v1-none` target | — | WASM builds |
| Stellar CLI | ≥ 22.0 | Deploy and invoke contracts |
| `cargo-test` | bundled | Unit tests |

```bash
rustup target add wasm32v1-none
cargo install --locked stellar-cli
```

## Repository layout

```
contracts/
  compliance/         — principal_compliance: SAC / SEP-8 / SEP-57 adapter (library) + test RWA token
  oracle_adapter/     — reference-value oracle with freshness and admin controls
  permissioning/      — account and asset eligibility registry
  risk_control/       — pause and the ledger-sequence circuit breaker (global + per-asset)
  market_config/      — per-market fees, fee split, swap-fee schedule
  sy_wrapper/         — yield wrapper: holds underlying, mints SY shares
  principal_manager/  — mint, recombine, settle_all, redeem, fees
  pt_token/           — SEP-41 Principal Token
  yt_token/           — SEP-41 Yield Token with the 1/rate index
  market_pool/        — PT/SY yield-curve AMM (fixed-point math in src/math.rs)
  router/             — stateless one-transaction flows
  recovery_escrow/    — seize SY/PT/YT/LP, batch, records
  integration_tests/  — full-stack fixture (src/stack.rs) and cross-contract tests
```

Each contract is an independent crate with its own `Cargo.toml`. Unit tests live in `src/test.rs`
(`#[cfg(test)] mod test;`) so coverage can exclude them; cross-contract tests live in
`contracts/integration_tests/tests/`.

## Building

```bash
# All contracts (native, for tests)
cargo build

# All contracts (WASM, for deployment)
cargo build --target wasm32v1-none --release
```

WASM artifacts land in `target/wasm32v1-none/release/*.wasm`.

## Testing

```bash
# Everything (unit + integration)
cargo test --workspace

# Build the pool WASM first if you want the on-WASM CPU-budget test to run (it skips otherwise)
cargo build --release --target wasm32v1-none -p principal_market_pool

# One crate / one suite
cargo test -p principal_sy_wrapper
cargo test -p principal_integration_tests --test amm

# Production-code coverage (what CI reports)
cargo llvm-cov --workspace --ignore-filename-regex '(/test\.rs|_test\.rs|/tests/|integration_tests|mock_rwa)' --summary-only
```

Unit tests use `env.mock_all_auths()`; the integration suite additionally drops the mocks
(`env.mock_auths(&[])`) to prove that calls fail without signatures. Assert **specific** error codes with
`try_*` (`err_code(...)` in `stack.rs`), not bare `should_panic`. If you change a number quoted in
`docs/YIELD_MATH_AND_FEES.md`, `doc_examples.rs` will fail until the guide is updated with it.

## Code style

* `cargo fmt --all` before every commit.
* `cargo clippy --all -- -D warnings` must pass with zero warnings.
* No `unwrap()` on external inputs — use `panic_with_error!` with a typed `#[contracterror]` value.
* Storage keys must be variants of a `#[contracttype]` enum, never raw strings.
* Use `instance()` storage for contract configuration; `persistent()` for per-user data.
* Emit an event for every state-changing operation.

## Adding a new contract

1. Create `contracts/<name>/Cargo.toml` with `crate-type = ["cdylib", "rlib"]` and `soroban-sdk` as both a dependency and dev-dependency (with `features = ["testutils"]`).
2. Add `"contracts/<name>"` to the workspace `members` list in the root `Cargo.toml`.
3. Define `#[contracterror]` and `#[contracttype]` enums before the contract struct.
4. Write unit tests in `src/test.rs` (`#[cfg(test)] mod test;` in `lib.rs`) and cross-contract tests under `contracts/integration_tests/tests/`.

## Pull request checklist

- [ ] `cargo fmt --all` clean
- [ ] `cargo clippy --all -- -D warnings` clean
- [ ] `cargo test --workspace` passes and coverage stays ≥ 95 %
- [ ] New storage keys documented in `TECHNICAL_SPECIFICATION.md`
- [ ] Security implications noted in PR description
- [ ] Events emitted for all state changes
