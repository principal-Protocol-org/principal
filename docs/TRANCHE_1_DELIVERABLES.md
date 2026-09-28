# Tranche 1 (MVP) — Deliverables, Evidence and Verification

Status as of 2026-09-28. This is the acceptance document for Tranche 1: for each deliverable it states
what was built, where the code is, which tests prove it, and how to re-run the proof. Nothing here is
self-reported: every claim names a test or CI job that fails if it stops being true.

**Reproduce everything**

```bash
cargo build --release --target wasm32v1-none -p principal_market_pool   # once, for the CPU-budget test
cargo test --workspace                       # 328 tests, ~45 s
cargo llvm-cov --workspace --ignore-filename-regex '(/test\.rs|_test\.rs|/tests/|integration_tests|mock_rwa)' --summary-only
cargo build --workspace --release --target wasm32v1-none   # all 11 contracts
```

In CI the same steps run on every pull request (`.github/workflows/ci.yml`: `lint`, `wasm`, `test`,
`coverage`, `audit`), with the coverage report uploaded as a build artifact and printed in the job
summary. Documentation is built and published by `.github/workflows/docs.yml`.

## Summary

| | |
|---|---|
| Contracts | **11** (was 8): + `MarketConfig`, `MarketPool` (AMM), `Router`, and the shared `principal_compliance` adapter crate (not a contract) |
| Internal audit | 5-reviewer pass after the initial build; 2 High + 9 Medium/Low findings fixed, rest documented — see [TRANCHE_1_AUDIT.md](TRANCHE_1_AUDIT.md) |
| Rust | 14.3 k lines (was 7.2 k); all new code covered below |
| Tests | **328**: 217 unit + 111 cross-contract integration, all passing |
| Coverage (production code) | **97.7 % of lines** (3 810 / 3 901), 96.7 % of regions; every file ≥ 93.9 % |
| Contract sizes | all under Soroban's 128 KiB limit; largest `market_pool` 66 KB |
| AMM cost | swap ≈ 4.6 M CPU instructions (4.6 % of the 100 M limit), measured on the real WASM |
| Static checks | `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` clean |

## Response to the review

| Review point | Response | Where |
|---|---|---|
| *D1's ">95 % coverage, all tests pass" criterion is already met at 98.5 % and does not test the deliverable. Replace it with functional AMM assertions: time-decay curve, PT convergence to par, flash-mint and flash-redeem.* | **Replaced** — see [Deliverable 1 success criteria](#deliverable-1-success-criteria-revised). Coverage remains as a CI floor (95 %), not as the acceptance test. | below |
| *Address SEP-57 (T-REX): compliance reads the SAC's `authorized()`, so a SEP-57 `RWAToken` has no SAC admin to attach to.* | **Fixed in code, not only described.** A compliance adapter detects an RWA underlying and maps it onto the same two questions (may this account hold it? who is the issuer authority?) using `is_frozen` + `IdentityVerifier` and an operator-role capability probe. Every recovery scenario runs over both a SAC and a SEP-57 token. | [COMPLIANCE_ARCHITECTURE.md §3](COMPLIANCE_ARCHITECTURE.md#3-sep-57-t-rex-rwa-tokens-why-inheritance-broke-and-how-it-is-restored), `contracts/compliance`, `recovery.rs` |
| *Address SEP-8 (Regulated Assets, Final): PT/YT inheritance on SEP-8 issuers must be described.* | **Described precisely, including what cannot be inherited** (the authorize→pay→deauthorize sandwich cannot wrap a single-operation Soroban transaction; off-chain approval criteria are invisible to contracts) and what a SEP-8 issuer must do. Tested. | [COMPLIANCE_ARCHITECTURE.md §2](COMPLIANCE_ARCHITECTURE.md#2-sep-8-regulated-assets-what-ptyt-inherit-and-what-they-cannot), `sep8_style_asset_needs_persistent_authorization_at_rest` |
| *Every commit since 16 August has been docs-only; restart shipping code.* | This change set is code: +7.0 k lines of Rust across 11 contracts, plus CI. It also found and fixed three real defects in code that was already "done" (see [Defects found](#defects-found-while-building-this)). | this document |

---

## Deliverable 1 — Core Yield Tokenization Contracts

Budget $23,200 · Weeks 1–3.

### What was built

| Component | Requirement | Implementation | Proof |
|---|---|---|---|
| **SYWrapper** | tracks each depositor's share via an exchange rate | `exchange_rate = total_underlying × SCALE / total_shares` | unit tests `deposit_and_exchange_rate`, `withdraw_returns_underlying` |
| | slippage-protected deposit/withdraw | `deposit(from, amount, min_shares_out)`, `withdraw(from, shares, to, min_underlying_out)` → `SlippageExceeded` | `slippage_boundary_min_equals_exact_output_passes_and_one_more_reverts` |
| | per-address deposit cap | `set_deposit_cap`, `net_deposited(account)` (net of withdrawals) | `deposit_cap_boundary_exactly_at_the_cap_passes_and_one_unit_over_reverts` |
| | operations gated by the underlying's Stellar compliance rules | `principal_compliance::is_authorized` on both sides of every deposit/withdraw/transfer | `compliance_matrix_deauthorized_accounts_are_blocked_on_every_position_type` (SAC **and** SEP-57) |
| **PrincipalManager** | two-phase initialization that resolves the circular dependency | PT/YT deployed first → `PrincipalManager.initialize` (topology-checked: same underlying, permissioning, maturity, oracle, config) → one-time `set_minter` | `initialize_rejects_mismatched_{underlying,permissioning,maturity,oracle}` |
| | per-user entry-rate tracking at mint | every account snapshots the YT index at its own mint/transfer (`LastClaimedIndex`), so it earns only from when it held the position | `late_minter_does_not_receive_prior_yield`, `multi_user_late_mint_does_not_dilute_early_holder_yield` |
| | yield accounting | `claim_yield` (pre-maturity, paid in the same call) and the YT leg of `redeem` | `allowance_transfer_and_mid_life_claim_then_redeem`, `pt_plus_yt_claims_never_exceed_the_deposited_shares_at_any_mint_rate` |
| | maturity settlement | `settle_all()` freezes one settlement rate for PT and YT; yield stops at maturity | `yt_stops_accruing_at_maturity_and_pt_uses_the_frozen_rate`, `settle_all_requires_maturity_and_a_fresh_oracle` |
| | (extra) recombination | `recombine` PT + YT → SY before maturity | `recombine_returns_sy_at_the_current_rate_…` |
| **PT** | standard SEP-41, mint/burn only by PrincipalManager, `seize` for compliance recovery | `contracts/pt_token` | 29 unit tests; `only_the_manager_can_mint_and_burn_pt_and_yt_…` |
| **YT** | accruing yield index, claim anytime, SEP-41, same seize | `contracts/yt_token`; index `G = 1e12·SCALE/rate` | 36 unit tests incl. solvency at any mint rate |
| **AMM trading layer** | PT/SY market, PT swaps, liquidity deposit/withdraw, LP positions, compliance inherited | `contracts/market_pool` — time-aware Yield Space curve; LP ledger with compliance-gated `transfer_lp` and `seize_lp` | `amm.rs` (22), `liquidity_and_router.rs` (22) |
| **Router** | coordinates wrapping, PT/YT issuance, trading, liquidity, redemption | `contracts/router` — stateless, acts as the user, registry, deadline + min-out on every flow | `liquidity_and_router.rs`: `router_*` |
| **Market configuration** | the SAC's *current* administrator controls market creation, maturity and fees; tokenization fee (e.g. 5 bps), YT fee (e.g. 10 %), swap Fee Tier (e.g. 0.1 %); `Trading Fee = Fee Tier × Days to Maturity / 365`; protocol share configurable, initially 20 % / 80 % to the creator | `contracts/market_config`; fee accrual in `PrincipalManager` and `MarketPool`; permissionless `claim_protocol_fees` / `claim_creator_fees` pay the treasury and the **live** `SAC.admin()` | `only_the_live_sac_admin_may_set_fees`, `swap_fee_follows_fee_tier_times_days_to_maturity_over_365`, `swap_fees_split_twenty_eighty_…`, `creator_fee_share_follows_a_sac_admin_rotation`, `tokenization_fee_is_withheld_accrued_and_claimable_…` |

### Deliverable 1 success criteria (revised)

*Replaces "all contracts compile and pass their full unit and integration test suites with test coverage
exceeding 95 %", which the codebase already satisfied and which says nothing about whether the AMM
works.* The deliverable is accepted when **all** of the following functional assertions hold, each proven
by a named test running against the real contracts:

| # | Functional assertion | Test(s) | What is asserted |
|---|---|---|---|
| **1** | **Time-decay yield curve.** With no trades at all, PT's price rises monotonically toward par as maturity approaches, following the closed form `(x/y)^(τ/S)`, while the reserves never move. | `time_decay_curve_pt_price_rises_monotonically_toward_par_with_no_trades`; guide example 5 (`doc_examples.rs`) | Price strictly increases at every 15-day sample over 180 days, stays below 1 before maturity, matches `0.95^(τ/4y)` to 2·10⁻⁶ at each step, the exponent rises toward 1, and `reserves()` is byte-identical at the end. Implied fixed rate stays ≈ 1.27 %. |
| **2** | **PT converges to par at maturity.** | `pt_converges_to_par_at_maturity`; `sweeping_the_oracle_rate_keeps_the_curve_in_value_units` | One minute before maturity: price within 10⁻⁶ of 1, exponent > 0.999999, 10 SY buys 10 PT to within 100 raw units. At maturity: swaps revert `Expired`, the exponent is exactly 1, LPs exit, and **PT redeems 1:1 at par** through `PrincipalManager`. Repeated at oracle rates 1.00 / 1.01 / 1.05 / 1.10 (value-unit correctness). |
| **3** | **Flash-mint path** (buy YT). | `flash_mint_buys_yt_by_minting_and_selling_the_pt_in_one_transaction`; `flash_mint_enforces_min_yt_out_and_the_deadline` | 100 SY → exactly 100 YT to the buyer, 0 PT retained, SY back = the pool's quote for the minted PT, PT and YT supplies grow together, `min_yt_out` and `deadline` (boundary inclusive) enforced. |
| **4** | **Flash-redeem path** (sell YT). | `flash_redeem_sells_yt_for_sy_by_recombining_with_pool_pt`; `flash_mint_then_flash_redeem_round_trip_returns_the_capital_minus_impact`; `flash_redeem_reverts_when_the_yt_cannot_cover_the_pt_it_consumes`; `flash_redeem_min_out_is_a_hard_floor_at_the_exact_payout` | Payout = recombined SY − the curve price of the PT consumed, exactly; pool PT falls by exactly `yt_in`, SY reserve rises by the net cost, PT and YT supplies burn together; a full mint→redeem round trip never returns more than it started with; near maturity (YT ≈ worthless) the call **reverts atomically** with nothing moved. |
| 5 | The curve's arithmetic is correct and safe. | `math_test.rs` (11); `invariant_never_decreases_across_a_pseudo_random_swap_sequence`; `fee_free_round_trips_never_profit_at_any_pool_size` | `ln`/`exp`/`pow`/`solve` match an `f64` reference across 20 orders of magnitude; `k` never falls; no profitable round trip from 10⁸ to 10¹⁷ raw units. |
| 6 | The AMM fits Soroban's resource limits. | `budget.rs` (real WASM) | Every operation < 100 M CPU instructions; a swap is ~4.6 M. |
| 7 | Solvency: PT + YT never claim more than the SY held. | `deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent`; YT `pt_plus_yt_claims_never_exceed_…` | After a full multi-user lifecycle with fees, every holder is paid and `PrincipalManager` holds only rounding dust. |

Coverage stays as a **CI floor** (95 % lines, `MIN_LINE_COVERAGE` in `ci.yml`) to catch silent erosion;
it is not an acceptance criterion.

---

## Deliverable 2 — Risk & Compliance Contracts

Budget $15,200 · Weeks 3–4.

| Component | Requirement | Implementation | Proof |
|---|---|---|---|
| **RiskControl** | consumer registration, rolling-window limit, pause switch | `add_consumer`, `check_deposit`, `pause`/`unpause` (admin-only unpause) | 29 unit tests |
| | **wired directly into SYWrapper's deposit and PrincipalManager's mint** | both call `RiskControl.check_deposit` inside their own entrypoint | `an_over_limit_deposit_reverts_automatically_with_no_manual_call`, `an_over_limit_mint_reverts_automatically` |
| | **ledger sequence numbers, not wall-clock** | `DEFAULT_WINDOW_LEDGERS` = 17 280; `env.ledger().sequence()` | `window_is_ledger_sequence_based_and_ignores_wall_clock`, `window_resets_after_exactly_window_ledgers` |
| | **per-asset limit beside the protocol-wide one** | `set_asset_limit`; both checked before either is written | `per_asset_limit_is_enforced_on_chain_alongside_the_protocol_limit`, `per_asset_limit_is_checked_alongside_the_protocol_limit` |
| **RecoveryEscrow** | re-checks the underlying's current administrator on every call | no stored key; `principal_compliance::is_authority` live | `sac_admin_rotation_moves_recovery_authority_immediately_with_nothing_to_update` |
| | seizes SY, PT, YT **and LP** | `seize_sy`/`seize_pt`/`seize_yt`/`seize_lp` | `recovery.rs` |
| | SY converts to the underlying at once; PT/YT held fully backed until maturity, then settled for native clawback | `seize_sy` unwraps in the same call; `finalize_record` | `seize_sy_unwraps_at_once_…`, `seize_pt_and_yt_hold_until_maturity_then_finalize_onto_the_same_record`, `recovered_underlying_can_be_clawed_back_natively_by_the_issuer` |
| | **batch seizure** for several accounts in one transaction | `seize_batch` (≤ 10 accounts, all-or-nothing), `seize_all_positions` | `batch_seizure_recovers_several_accounts_in_one_transaction_with_a_record_each`, `a_batch_is_all_or_nothing_and_size_bounded`, `seize_all_positions_sweeps_every_position_type_…` |
| | **per-account record** so recovered funds trace to the recovery event | `RecoveryRecord` per account per event (ledger, time, amounts, underlying recovered); `account_records`, `finalize_record` writes results back | `several_recovery_events_for_one_account_each_get_their_own_record` |

**Success criteria — all met.** A deposit or mint over the limit reverts automatically with no separate
call (`an_over_limit_*`); ledger-sequence windows and per-asset limits are active on-chain
(`window_is_ledger_sequence_*`, `per_asset_limit_is_enforced_*`); authorization and administrator controls
are inherited from the underlying — for a classic asset via the SAC, for a SEP-57 token via the adapter
(`compliance_matrix_*`, run over both); and recovery correctly handles SY, PT, YT and LP
(`recovery.rs`, run over both, 15 tests).

---

## Deliverable 3 — Test Suite + Technical Documentation

Budget $7,600 · Weeks 4–5.

| Item | Delivered | Where |
|---|---|---|
| **Edge cases: error conditions** | most error paths (all of `edge_cases.rs`, `recovery.rs`, `risk_control.rs`, `liquidity_and_router.rs`, the oracle/PM/YT audit regressions) assert the *specific* `try_*` error code; some pre-audit unit tests still use `#[should_panic]` and are being migrated (tracked, not blocking) | across all suites |
| **Edge cases: boundary values** | zero / negative / `i128::MIN` on every entry point; maximum amounts up to `i128::MAX` leave state untouched; a 10-billion-token position round-trips; exact thresholds for the deposit cap, slippage, circuit breaker, maturity (second-exact), oracle freshness (3 600 vs 3 601 s), allowance expiry, deadline | `edge_cases.rs` |
| **Edge cases: authorization** | every admin-only function rejects a non-admin; escrow-only `seize` rejects even the admin; mint/burn need the manager; a representative set of state-changing calls (deposit/withdraw/transfer, mint, admin setters, a pool swap and LP exit, a Router swap) fails with *no signatures at all* (`mock_auths(&[])`) in `edge_cases.rs::state_changing_calls_need_the_callers_own_signature`; permissionless calls (`update_yield_index`, fee claims) work with none and cannot redirect value | `edge_cases.rs` |
| **Full lifecycle integration** | deposit → wrap → PT/YT issuance → trading → liquidity → yield claiming → settlement → redemption → fee claims, with a solvency check | `full_lifecycle.rs::deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent` |
| **Complete compliance-recovery cycle** | flag → seize (SY, PT/YT, LP, batch) → finalize → native clawback, over a SAC and a SEP-57 token | `recovery.rs` |
| **Documentation: function reference** | what each function does, needs, and what can go wrong | [API_REFERENCE.md](API_REFERENCE.md) |
| **Documentation: yield math, fees, compliance with real numbers** | worked examples, each asserted by a test | [YIELD_MATH_AND_FEES.md](YIELD_MATH_AND_FEES.md) (+ `doc_examples.rs`), [AMM_DESIGN.md](AMM_DESIGN.md), [COMPLIANCE_ARCHITECTURE.md](COMPLIANCE_ARCHITECTURE.md) |
| **CI/CD** | tests, lint, WASM build + size gate, coverage report, audit on every PR | `.github/workflows/ci.yml` |
| **Docs published** | mdBook site built on every PR, deployed to GitHub Pages from `main` | `.github/workflows/docs.yml`, `book.toml`, `docs/SUMMARY.md` |

### Tranche completion criteria (from the roadmap)

| Criterion | Status |
|---|---|
| Contracts merged to `main` under a tagged release | **Pending** — code is complete and tested; the merge and `v*` tag are the maintainers' step. CI runs on `v*` tags. |
| Test suite passing in public CI with a coverage report published | Workflows added (`.github/workflows/ci.yml`, `docs.yml`); confirm on the first pushed pull request — not yet independently observed on GitHub. Locally: 328 / 328 pass, coverage 97.7 %. |
| RiskControl demonstrably blocks an oversized deposit in an integration test | ✅ `an_over_limit_deposit_reverts_automatically_with_no_manual_call` |
| Documentation published in the repository | ✅ `docs/` + published site |

---

## Defects found while building this

Writing the acceptance tests found real defects in code that was already marked done. All are fixed and
regression-tested.

1. **Insolvency at any mint rate other than 1.0.** `YTToken` scaled yield by a percentage of *notional*,
   but yield accrues on the *underlying* the position holds. For a position minted at rate 1.05 and
   settled at 1.10, PT + YT claimed **100.23 SY against 100 SY of custody**. Every earlier test ran at rate
   1.0, where the two coincide. Fix: the index is now the closed form `1/rate`, which telescopes to
   exactly `N / r₀` per position for any `r₀` (also removes path-dependence and 1e-7 rounding drift; the
   factor is now 1e12-precise and rounded against the YT holder). Tests: `pt_plus_yt_claims_never_exceed_…`,
   `deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent`.
2. **AMM rounding profit at scale.** A one-unit safety margin was not enough once pools are large:
   fixed-point error scales with reserve size and a fee-free round trip on a 1e17-unit pool *profited* by
   6 units. Fix: a magnitude-scaled pad (`1 + u/1e14`) in the curve solver. Test:
   `fee_free_round_trips_never_profit_at_any_pool_size`.
3. **Recovery blocked by the pause.** `RecoveryEscrow.seize_sy` seized fine while the wrapper was paused,
   but its immediate unwrap went through `withdraw`, which the pause blocked — defeating the stated intent
   that recovery survive an incident pause. Fix: the configured escrow is exempt from the SY pause when it
   is a party. Test: `seizure_works_while_the_market_is_paused`.

Also corrected: the documented build target (`wasm32-unknown-unknown`) does not build with the pinned SDK
on current Rust — `wasm32v1-none` is required.

## Deviations, design decisions and open items

Stated so nothing is discovered later.

* **Swap fees do not reach LPs.** The specified split routes 20 % of each swap fee to Principal and 80 % to
  the market creator; LPs earn PT convergence only. One field in `MarketConfig.split` adds an LP share if
  wanted. ([AMM_DESIGN.md §4](AMM_DESIGN.md))
* **The circuit breaker counts each entry point**, so a `wrap_and_mint` of 100 consumes 200 of the window.
  This follows "wire the breaker into SYWrapper deposit and PrincipalManager mint" literally; a single
  intake point would need an origin marker.
* **Per-user entry rate** is realised as a per-account snapshot of the YT index rather than a stored
  `InitialRate(addr)`; it is equivalent for accounting and additionally correct across transfers.
* **`SYWrapper.exchange_rate` is 1.0 in practice** (it tracks deposits and withdrawals, not rebases);
  appreciation is carried by the oracle. A rebasing underlying would need the two rates reconciled.
* **Router registry is admin-only** (protocol admin), by design: it is an anti-phishing allow-list, not
  market creation, which stays gated on the underlying's issuer authority.
* **SEP-57 is a Draft** (v0.4.0). The adapter isolates every assumption in one crate; see the limits in
  [COMPLIANCE_ARCHITECTURE.md §3](COMPLIANCE_ARCHITECTURE.md).
* **Not built (outside Tranche 1):** fee-change timelock, implied-rate TWAP oracle, `LiquidationAdapter`,
  a third-party audit, deployment scripts for the new contracts (steps documented in
  [DEPLOYMENT.md](DEPLOYMENT.md)).
* **Test-snapshot files** for the new crates and the integration suite are git-ignored (regenerated on
  every run); the pre-existing per-contract snapshot directories remain tracked.
