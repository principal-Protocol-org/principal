//! Defense-in-depth guards added to `MarketPool` during an audit review:
//!
//! * The oracle rate is read in one place (`rate()`, used by `state()`/`state_fresh()` and
//!   `add_liquidity`'s own direct read) and must be positive, not merely fresh --
//!   `is_available()` attests freshness, not positivity, and a buggy/compromised oracle
//!   reporting a non-positive value while still "available" would otherwise divide-by-zero or
//!   silently corrupt sign throughout every quote/trade.
//! * `MarketConfig.swap_fee_rate` is read in one place (`state()`) and must land in
//!   `[0, FEE_SCALE)`, independent of `MarketConfig`'s own cap on it -- `calc_buy_exact_pt`
//!   divides by `FEE_SCALE - fee_rate`.
//!
//! The real `OracleAdapter` refuses to ever store a non-positive value (see its own module
//! docs), and the real `MarketConfig` caps `fee_rate` well under `FEE_SCALE`, so neither bad
//! input can be produced through the real contracts' own public APIs. These tests swap the
//! pool's configured `Oracle`/`Config` address (both plain, public `DataKey` fields, same
//! technique `audit_regressions.rs` already uses to read TTLs via `Env::as_contract`) for a
//! minimal double that can misbehave on purpose, to prove the pool's own guard -- not the
//! upstream contract's good behavior -- is what stops it.

use principal_integration_tests::stack::*;
use principal_market_pool::{DataKey, Error as PoolError};

mod mock_oracle {
    use soroban_sdk::{contract, contractimpl, contracttype, Env};

    #[contracttype]
    enum DataKey {
        Rate,
        Available,
    }

    /// A settable oracle double, for exercising a non-positive-but-fresh reading -- a state the
    /// real `OracleAdapter` can never reach, since it rejects non-positive values at submission.
    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn init(env: Env, rate: i128, available: bool) {
            env.storage().instance().set(&DataKey::Rate, &rate);
            env.storage()
                .instance()
                .set(&DataKey::Available, &available);
        }

        pub fn get_reference_value(env: Env) -> i128 {
            env.storage().instance().get(&DataKey::Rate).unwrap()
        }

        pub fn is_available(env: Env) -> bool {
            env.storage().instance().get(&DataKey::Available).unwrap()
        }
    }
}

mod mock_fee_config {
    use soroban_sdk::{contract, contractimpl, contracttype, Env};

    #[contracttype]
    enum DataKey {
        FeeRate,
    }

    /// A settable `MarketConfig` double exposing only `swap_fee_rate` -- the one call
    /// `MarketPool::state` makes on the config -- returning whatever out-of-range value a test
    /// sets, something the real `MarketConfig` can never do (it caps `fee_rate` well under
    /// `FEE_SCALE` on `initialize`/`set_fees`).
    #[contract]
    pub struct MockFeeConfig;

    #[contractimpl]
    impl MockFeeConfig {
        pub fn init(env: Env, fee_rate: i128) {
            env.storage().instance().set(&DataKey::FeeRate, &fee_rate);
        }

        pub fn swap_fee_rate(env: Env, _seconds_to_maturity: u64) -> i128 {
            env.storage().instance().get(&DataKey::FeeRate).unwrap()
        }
    }
}

#[test]
fn pool_state_rejects_a_fresh_but_non_positive_oracle_rate() {
    let s = deploy_stack(LONG);

    let mock_oracle_id = s.env.register(mock_oracle::MockOracle, ());
    mock_oracle::MockOracleClient::new(&s.env, &mock_oracle_id).init(&-5_i128, &true);

    // Swap the pool's own configured Oracle for the double -- a plain, public `DataKey` field,
    // not something `MarketPool` exposes a setter for (by design: it is fixed at `initialize`
    // from the manager's own topology).
    s.env.as_contract(&s.pool.address, || {
        s.env
            .storage()
            .instance()
            .set(&DataKey::Oracle, &mock_oracle_id);
    });

    assert_eq!(
        s.pool.try_pool_state().err(),
        err_code(PoolError::OracleStale as u32)
    );
    // Every other quote/trade shares the same single read point, so one representative extra
    // call is enough to confirm the fix is not local to `pool_state` alone.
    assert_eq!(
        s.pool.try_quote_sy_for_pt(&(10 * SCALE)).err(),
        err_code(PoolError::OracleStale as u32)
    );
}

#[test]
fn pool_state_rejects_an_out_of_range_fee_rate_independent_of_market_config() {
    let s = deploy_stack(LONG);

    let mock_config_id = s.env.register(mock_fee_config::MockFeeConfig, ());
    // At or above FEE_SCALE: `calc_buy_exact_pt`'s `FEE_SCALE - fee_rate` divisor would be
    // zero or negative if this ever reached it.
    mock_fee_config::MockFeeConfigClient::new(&s.env, &mock_config_id)
        .init(&principal_market_pool::FEE_SCALE);

    s.env.as_contract(&s.pool.address, || {
        s.env
            .storage()
            .instance()
            .set(&DataKey::Config, &mock_config_id);
    });

    assert_eq!(
        s.pool.try_pool_state().err(),
        err_code(PoolError::MathError as u32)
    );
}
