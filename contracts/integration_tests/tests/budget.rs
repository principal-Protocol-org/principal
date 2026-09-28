//! Resource-limit check: the AMM's fixed-point curve math must fit inside a Soroban transaction's
//! CPU budget when run as the real compiled WASM (native tests do not meter guest instructions).
//!
//! Needs the pool built first:
//! `cargo build --release --target wasm32v1-none -p principal_market_pool`.
//! When the artifact is absent the test skips instead of failing, so plain `cargo test` works on a
//! fresh checkout; CI builds the WASM first and therefore always runs it.

use principal_integration_tests::stack::*;

const POOL_WASM_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32v1-none/release/principal_market_pool.wasm"
);
/// Soroban's per-transaction CPU instruction limit (network config setting).
const CPU_LIMIT: u64 = 100_000_000;

fn cpu(s: &Stack) -> u64 {
    s.env.cost_estimate().budget().cpu_instruction_cost()
}

#[test]
fn amm_operations_fit_the_per_transaction_cpu_budget() {
    let Ok(wasm) = std::fs::read(POOL_WASM_PATH) else {
        eprintln!("skipping: {POOL_WASM_PATH} not built");
        return;
    };
    let s = deploy_with_pool_wasm(Config::with_example_fees(T0 + 180 * DAY), &wasm);
    s.env.cost_estimate().budget().reset_unlimited();
    let (_, lp) = seed_pool(&s, 1_000 * SCALE, 950 * SCALE);
    let trader = s.new_user();
    deposit_sy(&s, &trader, 400 * SCALE);
    let shares = deposit_sy(&s, &trader, 400 * SCALE);
    s.pm.mint(&trader, &shares);
    let pool = s.pool.address.clone();

    let mut report = std::vec::Vec::new();
    let mut measure = |name: &str, f: &dyn Fn()| {
        s.env.cost_estimate().budget().reset_unlimited();
        f();
        let used = cpu(&s);
        let mem = s.env.cost_estimate().budget().memory_bytes_cost();
        report.push((name.to_string(), used));
        eprintln!("  {name}: cpu {used}, mem {mem}");
        assert!(
            used < CPU_LIMIT,
            "{name} used {used} CPU instructions (limit {CPU_LIMIT})"
        );
    };

    measure("swap_sy_for_pt", &|| {
        s.pool.swap_sy_for_pt(&trader, &trader, &(10 * SCALE), &0);
    });
    measure("swap_pt_for_sy", &|| {
        s.pool.swap_pt_for_sy(&trader, &trader, &(10 * SCALE), &0);
    });
    measure("add_liquidity", &|| {
        s.pool
            .add_liquidity(&trader, &(20 * SCALE), &(20 * SCALE), &0);
    });
    measure("add_liquidity_single_sy (18-step bisection)", &|| {
        s.pool.add_liquidity_single_sy(&trader, &(20 * SCALE), &0);
    });
    measure("router.swap_sy_for_yt (flash-mint)", &|| {
        s.router
            .swap_sy_for_yt(&trader, &pool, &(20 * SCALE), &0, &u64::MAX);
    });
    let yt = s.yt.balance(&trader).min(15 * SCALE);
    measure("swap_yt_for_sy (flash-redeem)", &|| {
        s.pool.swap_yt_for_sy(&trader, &trader, &yt, &0);
    });
    measure("remove_liquidity", &|| {
        s.pool
            .remove_liquidity(&lp, &lp, &(s.pool.lp_balance(&lp) / 10), &0, &0);
    });
    for (name, used) in &report {
        eprintln!(
            "{name:<48} {used:>12} CPU instructions ({}% of limit)",
            used * 100 / CPU_LIMIT
        );
    }
}
