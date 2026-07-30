//! Cross-contract integration tests for the Principal Protocol.
//!
//! Unlike each contract's own unit tests (which mostly deploy the handful of dependencies a
//! single contract needs to exercise its own logic), the tests under `tests/` in this crate
//! deploy the *entire* eight-contract stack together and drive it through realistic, multi-step
//! flows the way a real integrator or end user would: deposit, mint, transfer (including
//! allowance-based transfers), claim yield mid-life, redeem at maturity, compliance recovery,
//! admin rotation, and pause/circuit-breaker behavior. This crate has no library code of its
//! own -- it exists purely to host these `tests/*.rs` integration tests as a separate compiled
//! target, matching Cargo's own "integration test" terminology.
