# Contributing to Principal Protocol

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| Rust | stable (≥ 1.79) | Contract compilation |
| `wasm32-unknown-unknown` target | — | WASM builds |
| Stellar CLI | ≥ 22.0 | Deploy and invoke contracts |
| `cargo-test` | bundled | Unit tests |

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked stellar-cli
```

## Repository layout

```
contracts/
  oracle_adapter/     — reference-value oracle with freshness and admin controls
  permissioning/      — account and asset eligibility registry
  sy_wrapper/         — yield wrapper: holds underlying, mints SY shares
  principal_manager/  — splits SY shares into PT + YT; settles at maturity
  risk_control/       — global pause flag and rolling circuit breaker
```

Each contract is an independent crate with its own `Cargo.toml`.

## Building

```bash
# All contracts (native, for tests)
cargo build

# All contracts (WASM, for deployment)
cargo build --target wasm32-unknown-unknown --release
```

WASM artifacts land in `target/wasm32-unknown-unknown/release/*.wasm`.

## Testing

```bash
# All workspace tests
cargo test

# Single contract
cargo test -p principal_oracle_adapter
cargo test -p principal_permissioning
cargo test -p principal_sy_wrapper
cargo test -p principal_manager
cargo test -p principal_risk_control

# Verbose output
cargo test -- --nocapture
```

Tests use `env.mock_all_auths()` to bypass auth in unit tests. Integration tests that validate the full auth flow should be added in a separate `tests/` crate per contract.

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
4. Write unit tests in the same file under `#[cfg(test)]`.

## Pull request checklist

- [ ] `cargo fmt --all` clean
- [ ] `cargo clippy --all -- -D warnings` clean
- [ ] `cargo test` passes
- [ ] New storage keys documented in `TECHNICAL_SPECIFICATION.md`
- [ ] Security implications noted in PR description
- [ ] Events emitted for all state changes
