//! Cross-contract integration tests for the Principal Protocol.
//!
//! Unlike each contract's own unit tests, the tests under `tests/` deploy the *entire* stack
//! (oracle, permissioning, risk control, market config, SY wrapper, PT, YT, principal manager,
//! market pool, router, recovery escrow) together, wired exactly as `docs/DEPLOYMENT.md` describes,
//! and drive it through realistic multi-step flows: deposit, tokenize, trade, provide liquidity,
//! claim yield, settle at maturity, recover a flagged account, rotate admins, trip the circuit
//! breaker. The shared deployment lives in [`stack`].

#![cfg_attr(target_family = "wasm", no_std)]
#![cfg(not(target_family = "wasm"))]

pub mod stack;
