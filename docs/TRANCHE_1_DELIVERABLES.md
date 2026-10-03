# Tranche 1 (MVP): Deliverables, Evidence and Verification

Status as of 4 October 2026. The current contracts are deployed to Stellar Testnet and exercised on-chain; see the on-chain sections of Deliverables 1 and 2. This is the acceptance record for Tranche 1. For each deliverable it gives the original funded description and success criteria word for word, then the evidence that each success criterion is met, with full links to source code, tests, documents and Stellar Testnet transactions.

Reviewed commit: [6cf257afb588](https://github.com/principal-Protocol-org/principal/commit/6cf257afb588c63d694f2262d8b289d4809be8ee) on branch `feat/tranche-1-mvp`. Every source link below is pinned to this commit, so it does not move when the branch changes.

## How to reproduce the checks

```bash
cargo build --workspace --release --target wasm32v1-none   # builds all 11 contracts
cargo test --workspace                                      # 328 tests
cargo llvm-cov --workspace --ignore-filename-regex '(/test\.rs|_test\.rs|/tests/|integration_tests|mock_rwa)' --summary-only
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The same steps run on every pull request in [ci.yml](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/.github/workflows/ci.yml). The documentation site is built and deployed by [docs.yml](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/.github/workflows/docs.yml).

## Results of the re-check on 3 October 2026

| Check | Result |
|---|---|
| Tests, whole workspace | 328 passed, 0 failed |
| Line coverage, production code | 97.67% (3,810 of 3,901 lines) |
| Lowest single file, line coverage | `compliance` at 95.65%; every other file is higher |
| Region coverage, production code | 96.65% |
| Testnet transaction hashes cited in the two evidence files | 65 unique, all report a successful result on the public Testnet ledger |
| Testnet contract addresses cited | 17, each returns a creation record from the Stellar Expert API |
| Testnet account addresses cited | 8, each exists on Horizon |
| Source and document paths cited in the Word review | all exist at commit `6cf257a` |

Not checked in this pass: the GitHub Actions run on GitHub's own runners, and the live GitHub Pages site. Neither has been observed yet, because no pull request has been opened from this branch.

## Summary

| Item | Value |
|---|---|
| Contracts | 11 (was 8): adds `MarketConfig`, `MarketPool` (AMM), `Router`, and the shared `principal_compliance` adapter crate (a library, not a contract) |
| Internal audit | Five-reviewer pass on this commit. 2 High and 11 Medium/Low findings. All High findings fixed. See [TRANCHE_1_AUDIT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TRANCHE_1_AUDIT.md) for the disposition of each finding. |
| Rust | About 14.3k lines (was 7.2k) |
| Contract sizes | All under Soroban's 128 KiB limit. Largest `market_pool` at about 66 KB. |
| AMM cost | A swap uses about 4.6M CPU instructions, against a 100M limit, measured on the real WASM. |
| Static checks | `cargo fmt --check` and `cargo clippy -D warnings` pass. |

---

## Deliverable 1: Core Yield Tokenization Contracts

Budget: $23,200. Weeks 1–3. 186 hours.

### Original description (as funded, verbatim)

> Complete the core yield tokenization and trading contracts:
>
> SYWrapper — wraps the underlying yield-bearing asset and tracks each depositor's share via an exchange rate. Main features: slippage-protected deposit/withdraw, a per-address deposit cap, and operations gated by the underlying asset's Stellar compliance rules.
>
> PrincipalManager — coordinates the split between PT and YT. Main features: two-phase initialization that resolves the circular dependency with the token contracts, per-user entry-rate tracking at mint, yield accounting, and maturity settlement.
>
> PT (Principal Token) — a fixed-value claim redeemable at par at maturity. Main features: a standard SEP-41 token, mintable/burnable only by PrincipalManager, plus a seize function reserved for compliance recovery.
>
> YT (Yield Token) — captures the variable yield generated between issuance and maturity. Main features: an accruing yield index letting holders claim earned yield anytime before maturity, plus the same SEP-41 base and compliance-recovery seize function as PT.
>
> AMM trading layer — implements the PT/SY liquidity market, PT swaps, liquidity deposits and withdrawals, and LP positions. Compliance controls inherited from the underlying Stellar Asset also apply to trading and LP positions.
>
> Router — coordinates the user flows across wrapping, PT/YT issuance, trading, liquidity operations, and redemption.
>
> Market configuration — the current administrator of the underlying SAC controls market creation and configures the market maturity and fees. Each market supports a configurable tokenization fee (for example, 5 bps), YT fee (for example, 10%), and swap Fee Tier (for example, 0.1%). Swap fees decrease as maturity approaches following Trading Fee = Fee Tier x Days to Maturity / 365. Principal's protocol share is also configurable and is initially set at 20% of the tokenization, YT, and swap fees collected by each market, with the remaining 80% allocated to the market creator, i.e. the current administrator of the underlying SAC.

### Success criteria (as funded, verbatim)

> All contracts compile and pass their full unit and integration test suites with test coverage exceeding 95%.

**Reviewer's note.** An earlier build of this branch already exceeded 95% coverage while the AMM was materially unfinished. Coverage alone cannot show that the trading system works. The review therefore keeps 95% as a floor that must hold, and adds four functional tests that the AMM must pass. This replacement criterion was proposed in the funder's review of the earlier draft.

Replacement functional criteria, each shown by a named test:

| # | Behaviour | Test | What the test checks, in plain words |
|---|---|---|---|
| 1 | Time-decay price curve | `time_decay_curve_pt_price_rises_monotonically_toward_par_with_no_trades` ([source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L62)) | With no trades, PT's price rises at every 15-day sample over 180 days, stays below par before maturity, and the pool's reserves do not change. |
| 2 | PT converges to par at maturity | `pt_converges_to_par_at_maturity` ([source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L92)) | One minute before maturity the price is within a millionth of par. At maturity, trading stops and PT redeems one-for-one through PrincipalManager. |
| 3 | Flash-mint (buy YT in one transaction) | `flash_mint_buys_yt_by_minting_and_selling_the_pt_in_one_transaction` ([source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L457)) | The buyer deposits SY, the protocol mints PT and YT, the PT is sold to the pool, and the buyer ends with only YT. Companion test `flash_mint_enforces_min_yt_out_and_the_deadline` checks the minimum-output and deadline guards. |
| 4 | Flash-redeem (sell YT in one transaction) | `flash_redeem_sells_yt_for_sy_by_recombining_with_pool_pt` ([source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L525)) | The seller's YT is combined with PT from the pool's reserve, and the seller receives the difference, atomically. Companion tests cover the round trip, a revert when YT cannot cover the PT it consumes, and the exact minimum-output floor. |

### Evidence and proof

**Compile.** The eleven contracts build for `wasm32v1-none`. This was confirmed in the earlier audit session. The build target is `wasm32v1-none` rather than `wasm32-unknown-unknown`, which does not build with soroban-sdk 26 on current Rust. The target change is recorded under [Defects found](#defects-found-while-building-this).

**Tests pass.** 328 tests passed on 3 October 2026 with `cargo test --workspace`. The number includes unit tests inside each contract and the cross-contract integration tests in [contracts/integration_tests/tests/](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/).

**Coverage above 95%.** 97.67% of production lines are covered. The per-file lowest is [compliance](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/compliance/src/lib.rs) at 95.65%. The coverage exclusion rule is the one in the command above.

**Components.**

SYWrapper:
- Exchange-rate accounting: [sy_wrapper/src/lib.rs lines 345–361](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L345-L361).
- Slippage-protected deposit, which reverts with `SlippageExceeded`: [lines 207–249](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L207-L249). Slippage-protected withdraw: [lines 276–317](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L276-L317).
- Per-address deposit cap: [lines 396–436](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L396-L436).
- Compliance gate on every transfer: [lines 628–636](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L628-L636).
- Test `deposit_cap_boundary_exactly_at_the_cap_passes_and_one_unit_over_reverts`: deposits exactly the cap (must succeed), then one base unit more (must be rejected). It checks the exact edge, not a value well inside the limit. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L176).
- Test `slippage_boundary_min_equals_exact_output_passes_and_one_more_reverts`: a minimum-output setting equal to the amount received succeeds, and one unit higher is rejected. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L209).

PrincipalManager:
- Two-phase setup. In plain words: PT and YT are deployed first with no minter. PrincipalManager is then registered as the only minter. This breaks the deadlock in which each contract needs the other's address before either can be configured. `initialize` is at [principal_manager/src/lib.rs lines 290–341](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/lib.rs#L290-L341). `set_minter` on YT is at [yt_token/src/lib.rs lines 252–258](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/yt_token/src/lib.rs#L252-L258), and it reverts `MinterAlreadySet` on a second call.
- Test `initialize_rejects_mismatched_underlying`: PrincipalManager refuses to start if PT or YT uses a different underlying asset. Sibling tests cover mismatched permissioning, maturity and oracle. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/test.rs#L594).
- Per-user entry rate: each account stores the yield index at its last mint or transfer, so it earns only from the time it held the position. Code: [yt_token/src/lib.rs lines 677–712](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/yt_token/src/lib.rs#L677-L712).
- Test `late_minter_does_not_receive_prior_yield`: an account that mints after yield has accrued gets none of that yield. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/test.rs#L430).
- Test `multi_user_late_mint_does_not_dilute_early_holder_yield`: a later minter at a higher rate does not reduce the early holder's accrued yield. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/full_lifecycle.rs#L210).
- Mint: [principal_manager/src/lib.rs lines 343–421](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/lib.rs#L343-L421). Maturity settlement, where `settle_all` freezes one rate and `redeem` pays out: [lines 461–530](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/lib.rs#L461-L530).
- Test `deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent`: a full run with several users, trades and fees, from deposit to redemption. At the end every holder is paid, and PrincipalManager holds only rounding dust. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/full_lifecycle.rs#L20).

PT and YT tokens:
- SEP-41 interface: [pt_token/src/lib.rs lines 182–295](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/pt_token/src/lib.rs#L182-L295).
- Mint and burn, callable only by PrincipalManager: [lines 295–349](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/pt_token/src/lib.rs#L295-L349).
- `seize`, callable only by the configured RecoveryEscrow: [lines 350–380](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/pt_token/src/lib.rs#L350-L380).
- YT yield index, `update_yield_index` and `claim_yield`: [yt_token/src/lib.rs lines 430–577](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/yt_token/src/lib.rs#L430-L577).
- Test `escrow_only_seize_functions_reject_everyone_else_including_the_admin`: `seize` refuses every caller except the escrow, including the token's own admin. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L403).
- Test `pt_plus_yt_claims_never_exceed_the_deposited_shares_at_any_mint_rate`: however a position was minted, PT and YT together never claim more underlying than was deposited. This is the solvency property. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/yt_token/src/test.rs#L822).

AMM trading layer (MarketPool):
- PT swaps: [market_pool/src/lib.rs lines 279–341](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/market_pool/src/lib.rs#L279-L341).
- Liquidity add and remove: [lines 389–542](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/market_pool/src/lib.rs#L389-L542).
- LP ledger with compliance gate and recovery support: [lines 543–589](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/market_pool/src/lib.rs#L543-L589).
- Test `deauthorized_holders_cannot_trade_or_provide_liquidity`: an account the issuer has deauthorised cannot swap or add liquidity. It runs over both a classic Stellar asset and a SEP-57 token. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/liquidity_and_router.rs#L261).

Router:
- `wrap_and_mint`: [router/src/lib.rs lines 261–309](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/router/src/lib.rs#L261-L309).
- `swap_sy_for_yt` and `swap_yt_for_sy`, the flash-mint and flash-redeem entry points: [lines 335–391](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/router/src/lib.rs#L335-L391).
- `redeem_at_maturity`: [lines 456–463](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/router/src/lib.rs#L456-L463).
- Test `router_acts_as_the_user_so_a_deauthorized_user_gains_nothing_by_using_it`: the Router holds no funds and acts for the caller, so using it cannot get around a compliance block. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/liquidity_and_router.rs#L594).

Market configuration and fees:
- `initialize` and `set_fees`, callable only by the underlying asset's current administrator, read live: [market_config/src/lib.rs lines 105–188](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/market_config/src/lib.rs#L105-L188).
- Swap fee formula, Fee Tier × Days to Maturity ÷ 365: [lines 269–277](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/market_config/src/lib.rs#L269-L277).
- Test `example_5_the_swap_fee_schedule` checks the documented fee table against the formula at six maturities: 365, 180, 90, 30, 7 and 1 days. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/doc_examples.rs#L122).
- Test `swap_fees_split_twenty_eighty_and_are_claimable_by_treasury_and_creator`: swap fees split 20% to the treasury and 80% to the creator, and each party can claim its share. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L407).
- Test `creator_fee_share_follows_a_sac_admin_rotation`: if the asset's administrator changes, the creator share goes to the new administrator. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/amm.rs#L439).

### On-chain evidence (current contracts, Stellar Testnet, 4 October 2026)

The current contracts were deployed to Stellar Testnet on 4 October 2026, from source identical to commit 6cf257a, and exercised end to end. The full record, with every contract address, account, and transaction, is in [TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md](https://github.com/principal-Protocol-org/principal/blob/e985b3ecfb3192705c5896274b820e31c2ccbaed/docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md). All 159 unique transaction hashes were checked on Horizon and all report success.

For Deliverable 1, the on-chain record shows these points:
- Deposit into SYWrapper and mint of PT and YT by PrincipalManager, with the 0.05% tokenization fee visible in the balances (Alice minted 4,997,500,000 raw PT and YT from a 5,000,000,000 raw deposit).
- Liquidity added and removed, PT swapped for SY, and SY swapped for PT on the pool.
- Flash-mint through the Router (SY to YT in one transaction).
- Mid-life yield claim after the oracle moved from 1.00 to 1.05.
- Settlement at maturity by `settle_all`, followed by redemption of both PT and YT.
- Trading refused after maturity, shown by a simulation error with no transaction.

Transactions for the PT and YT mint, the flash-mint, the claim and the redemption are in the evidence record, section 7, and each one links to the Stellar Expert page for that transaction. Flash-redeem on-chain is not in this run; it is covered by the named tests above.

### Assessment

Met in full. The compile, test and coverage criteria are each shown with a fresh result from this review. The four functional AMM behaviours are each shown by a named test that passes.

Two residual points are documented by the project, not found in this review:
- Under the specified 20/80 fee split, liquidity providers earn no swap fees. [AMM_DESIGN.md §4](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/AMM_DESIGN.md) records this as a deliberate reading of the funded split. A one-field change is identified if the intended economics differ.
- SYWrapper's exchange rate is fixed at 1.0. That is correct for assets whose appreciation is carried by the oracle rate. It has not been exercised with a balance-rebasing underlying, and the project states this limit plainly.

---

## Deliverable 2: Risk and Compliance Contracts

Budget: $15,200. Weeks 3–4. 122 hours.

### Original description (as funded, verbatim)

> Implement the Risk & Compliance Contracts:
>
> RiskControl — an automated circuit breaker limiting how much can be deposited across the protocol in a rolling window. Main features: consumer registration, a rolling-window limit checked directly during protocol operations, and a pause switch. This deliverable wires the breaker directly into SYWrapper's deposit and PrincipalManager's mint calls, switches the rolling window from wall-clock time to Stellar ledger sequence numbers, and adds a per-asset limit alongside the existing protocol-wide one.
>
> RecoveryEscrow — allows the underlying SAC administrator to execute compliance recovery without affecting other users. It re-checks the underlying asset's current administrator on every call and can seize SY, PT, YT, and LP positions. SY can be converted back into the underlying asset immediately, while PT/YT positions remain fully backed in escrow until maturity and are then settled back into the underlying asset for native clawback. This deliverable adds batch seizure for handling multiple accounts in one transaction and a per-account record so recovered funds trace back to the specific recovery event that produced them.

### Success criteria (as funded, verbatim)

> All contracts compile and pass their full unit and integration test suites with test coverage exceeding 95%.
>
> A deposit or mint that would exceed the circuit-breaker limit reverts automatically with no separate manual call required; ledger-sequence-based window and per-asset limits are active on-chain; authorization and administrator controls are inherited from the underlying SAC; and recovery flows correctly handle SY, PT, YT, and LP positions.

### Evidence and proof

**Criterion 1: compile, test and coverage.** Shared with Deliverable 1. 328 of 328 tests passed on 3 October 2026. RiskControl's line coverage is 99.28%, and RecoveryEscrow's is 98.63%. RiskControl has 29 unit tests in its own crate. RecoveryEscrow has no unit tests of its own by design: each of its calls depends on the real SY, PT, YT and pool contracts, so it is tested through the cross-contract suite, with 15 tests in [recovery.rs](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs).

**Criterion 2a: a deposit or mint over the limit reverts on its own.**
- `check_deposit`, the rolling-window limit check: [risk_control/src/lib.rs lines 233–302](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/risk_control/src/lib.rs#L233-L302).
- SYWrapper's deposit calls RiskControl inside its own entry point: [sy_wrapper/src/lib.rs lines 233–241](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/sy_wrapper/src/lib.rs#L233-L241).
- PrincipalManager's mint calls RiskControl inside its own entry point: [principal_manager/src/lib.rs lines 365–381](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/principal_manager/src/lib.rs#L365-L381).
- Consumer registration, so that only a registered contract may call the breaker: [risk_control/src/lib.rs lines 190–206](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/risk_control/src/lib.rs#L190-L206). Pause switch, where pausers can pause and only the admin can unpause: [lines 118–147](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/risk_control/src/lib.rs#L118-L147).
- Test `an_over_limit_deposit_reverts_automatically_with_no_manual_call`: a deposit that pushes the total over the limit is rejected by the deposit itself. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/risk_control.rs#L27).
- Test `an_over_limit_mint_reverts_automatically`: the same, for a PT/YT mint. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/risk_control.rs#L55).

**Criterion 2b: the window is measured in ledger sequence numbers, not wall-clock time.**
- `DEFAULT_WINDOW_LEDGERS` is 17,280 ledgers, about 24 hours at Stellar's roughly 5-second ledger close: [risk_control/src/lib.rs lines 24–33](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/risk_control/src/lib.rs#L24-L33).
- Test `window_is_ledger_sequence_based_and_ignores_wall_clock`: moving the wall clock forward a year without new ledgers does not reopen the window. It reopens only when the ledger sequence reaches the boundary. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/risk_control.rs#L89).

**Criterion 2c: a per-asset limit, active on-chain alongside the protocol-wide limit.**
- `set_asset_limit`. Both limits are checked before either is written, so a trip leaves no partial state: [risk_control/src/lib.rs lines 318–330](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/risk_control/src/lib.rs#L318-L330).
- Test `per_asset_limit_is_enforced_on_chain_alongside_the_protocol_limit`: the per-asset limit rejects a deposit even when the protocol-wide limit still has room. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/risk_control.rs#L109).

**Criterion 2d: authorization and administrator controls inherited from the underlying asset.**
- The compliance adapter answers "may this account hold the asset" and "who is the issuer", for both a classic Stellar Asset Contract and a SEP-57 token: [compliance/src/lib.rs lines 67–140](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/compliance/src/lib.rs#L67-L140).
- RecoveryEscrow re-checks the underlying's current administrator on every call and stores no administrator key: [recovery_escrow/src/lib.rs lines 528–534](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L528-L534).
- Test `compliance_matrix_deauthorized_accounts_are_blocked_on_every_position_type`: a deauthorised account is blocked from every position type (SY, PT, YT, LP), checked for both asset models. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L503).
- Test `sac_admin_rotation_moves_recovery_authority_immediately_with_nothing_to_update`: when the asset's administrator changes, recovery authority moves at once. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L469).
- Design rationale and the limits of each asset model: [COMPLIANCE_ARCHITECTURE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/COMPLIANCE_ARCHITECTURE.md).

**Criterion 2e: recovery handles SY, PT, YT and LP positions.**
- `seize_sy` converts seized SY back to the underlying at once: [recovery_escrow/src/lib.rs lines 243–249](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L243-L249).
- `seize_pt` and `seize_yt` hold positions fully backed in escrow until maturity: [lines 250–264](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L250-L264).
- `seize_lp` seizes LP positions. This goes beyond the original eight-contract design, because the AMM did not exist then: [lines 265–273](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L265-L273).
- `finalize_record` settles held PT and YT into the underlying at maturity, ready for native clawback: [lines 316–350](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L316-L350).
- Test `seize_sy_unwraps_at_once_writes_a_record_and_touches_nobody_else`: seizing SY unwraps it at once, writes a record, and leaves other accounts alone. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L47).
- Test `seize_pt_and_yt_hold_until_maturity_then_finalize_onto_the_same_record`. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L98).
- Test `seize_lp_burns_it_unwraps_the_sy_leg_and_holds_the_pt_leg`. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L195).
- Test `recovered_underlying_can_be_clawed_back_natively_by_the_issuer`: the underlying recovered into escrow can be taken back by the issuer through the asset's own clawback. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L82).
- The three seize tests each run twice, once per asset model, through the helper `each_kind` at [recovery.rs line 16](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L16).

**Batch seizure and per-account records.**
- `seize_batch` and `seize_all_positions` are all-or-nothing. If one account is still authorised, the whole transaction reverts: [recovery_escrow/src/lib.rs lines 274–315](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L274-L315).
- Batch size is limited to 3 accounts. The original value of 10 exceeded Soroban's 100-ledger-entry transaction limit. The audit lowered it to the measured maximum: [lines 56–61](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L56-L61).
- `RecoveryRecord`, one per seizure event, with ledger, time, every position seized, and the YT yield recovered with it: [lines 161–186](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/recovery_escrow/src/lib.rs#L161-L186).
- Test `batch_seizure_recovers_several_accounts_in_one_transaction_with_a_record_each`. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs#L257).
- Test `a_full_batch_at_the_bound_of_every_position_type_succeeds`: a batch of the maximum size, with every position type, succeeds under Soroban's real resource limits. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/audit_regressions.rs#L211).

### On-chain evidence (current contracts, Stellar Testnet, 4 October 2026)

The current RecoveryEscrow and RiskControl were exercised on Testnet on 4 October 2026. The full record is in [TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md](https://github.com/principal-Protocol-org/principal/blob/e985b3ecfb3192705c5896274b820e31c2ccbaed/docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md).

What the on-chain record shows:
- A single batch transaction, `seize_all_positions`, moved a deauthorised account's PT and YT into the escrow and wrote record 0.
- `finalize_record` at maturity settled the seized positions into the escrow: 2,855,714,285 raw units from the PT leg and 128,507,142 raw units from the YT leg, with 142,785,714 raw units of yield attributed to the seizure.
- RiskControl's protocol-wide volume is cumulative on-chain (27,200,000,000 raw units). SYWrapper and PrincipalManager call it from inside their own entry points, so the deposits and mints in this run were counted without any separate call.

What the on-chain record does not show, stated plainly:
- A refused deposit over the limit. No such transaction exists in this run. The automatic revert is covered by the named tests above.
- The ledger-sequence window reopening. The window is 17,280 ledgers and the run lasted far less than that.
- A native clawback of the recovered STA. The STA asset has clawback disabled (`auth_clawback_enabled: false`), so this step cannot be run on this asset as configured. The recovered amount sits in the escrow.

### Assessment

Met in full on the code and tests. The automatic-revert, ledger-sequence, per-asset, compliance-inheritance and recovery criteria are each shown by named tests that pass on this commit.

One design limit is disclosed rather than hidden: the circuit breaker counts gross inflows, so an already-compliant account that repeatedly deposits and withdraws can use up a window's budget. That costs the account its own capital and fees. The audit judged it not a fund-safety issue, because Permissioning is default-deny. See [TRANCHE_1_AUDIT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TRANCHE_1_AUDIT.md).

On-chain, the Testnet record supports the recovery mechanism but not the current breaker. Neither the automatic wiring nor the ledger-sequence window has been shown on a network yet. A fresh Testnet run of the current build is a Tranche 2 task.

---

## Deliverable 3: Test Suite and Technical Documentation

Budget: $7,600. Weeks 4–5. 76 hours.

### Original description (as funded, verbatim)

> Deliverables 1 and 2 already verify each contract works correctly under normal use. This deliverable adds the edge cases:
>
> Edge-case testing — error conditions, boundary values (zero amounts, maximum amounts, exact-threshold values), and authorization checks (who's allowed to call what, and what happens when they're not) across the protocol contracts, plus a full lifecycle integration test from deposit through PT/YT issuance, trading, yield claiming, and maturity settlement, together with a complete compliance-recovery cycle.
>
> Documentation — a reference explaining what each contract function does, what it needs, and what can go wrong, plus a guide to the yield math, market fees, and compliance architecture with real number examples, so other developers can build on top of the protocol without reading the code itself.
>
> CI/CD — every test runs automatically on every pull request, with a coverage report published alongside it, so a regression can't reach the merged codebase unnoticed.

### Success criteria (as funded, verbatim)

> Tests pass in CI, and documentation is published.

### Evidence and proof

**Criterion A: tests pass in CI.**
- Tests pass locally on this commit: 328 of 328, re-run 3 October 2026.
- The CI workflow is [ci.yml](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/.github/workflows/ci.yml). It runs formatting, linting, the WASM build with a size check, the full test suite, the coverage report and a dependency audit on every pull request.
- The same steps were reproduced locally against the same commit. `cargo fmt --check`, the clippy command, the WASM build, `cargo test --workspace`, `cargo llvm-cov` and `cargo audit` each passed.
- **Not yet shown:** a CI run on GitHub's runners. That needs a pull request from this branch: [open it here](https://github.com/principal-Protocol-org/principal/pull/new/feat/tranche-1-mvp).

**Criterion B: documentation is published.**
- Documentation is committed under [docs/](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/), with 17 files.
- The site is built with mdBook from [SUMMARY.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/SUMMARY.md) and [book.toml](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/book.toml). `mdbook build` completed with no errors on this commit.
- The publish workflow is [docs.yml](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/.github/workflows/docs.yml). It deploys to GitHub Pages on pushes to `main`.
- **Not yet shown:** the live GitHub Pages site. It is not deployed until the branch reaches `main`.

**Edge-case testing (funded item).** Error conditions and boundary values are in [edge_cases.rs](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs). Key tests:
- `every_admin_only_function_rejects_a_caller_that_is_not_the_admin`: every admin-only function refuses a non-admin caller. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L328).
- `state_changing_calls_need_the_callers_own_signature`: deposit, withdraw, transfer, mint, admin setters, a pool swap, an LP exit and a Router swap all fail when no signature is present. The test removes every mocked authorisation to show this. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L450).
- `escrow_only_seize_functions_reject_everyone_else_including_the_admin`. [Source](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/edge_cases.rs#L403).

**Full lifecycle (funded item).** One continuous test runs every stage in the description against the real contracts, not mocks. The stages are deposit, PT and YT issuance including the tokenization fee, trading through the pool and Router, a mid-life yield claim with the YT fee withheld, permissionless settlement at maturity, redemption for every participant, LP exit, and fee claims. It ends with a solvency check. Test: `deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent`, [full_lifecycle.rs line 20](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/full_lifecycle.rs#L20).

**Compliance-recovery cycle (funded item).** Flag, seize, finalise at maturity, then native clawback, over both asset models. Covered by the 15 tests in [recovery.rs](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/recovery.rs), listed under Deliverable 2.

**Documentation (funded item).**
- Function reference, covering all eleven contracts: [API_REFERENCE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/API_REFERENCE.md).
- Yield math, fees, and worked numeric examples: [YIELD_MATH_AND_FEES.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/YIELD_MATH_AND_FEES.md). Every worked number is asserted by a passing test in [doc_examples.rs](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/doc_examples.rs), so a change that moves a number fails the test rather than leaving the guide stale.
- Compliance architecture, including the SEP-8 and SEP-57 points: [COMPLIANCE_ARCHITECTURE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/COMPLIANCE_ARCHITECTURE.md).
- AMM design, including a survey of comparable Stellar and Soroban projects: [AMM_DESIGN.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/AMM_DESIGN.md).
- Specification, architecture and deployment runbook: [TECHNICAL_SPECIFICATION.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TECHNICAL_SPECIFICATION.md), [ARCHITECTURE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/ARCHITECTURE.md), [DEPLOYMENT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/DEPLOYMENT.md).

### Assessment

Partly evidenced, and this review does not round it up. The test suite and documentation content satisfy the funded description. The success criterion has two parts still open:
1. The tests have passed locally, but not yet in CI on GitHub's runners, because no pull request has been opened.
2. The documentation is committed, but the live GitHub Pages site has not been checked.

Opening the pull request and checking the published site would close this deliverable.

---

## Tranche completion criteria (from the roadmap)

| Criterion | Status |
|---|---|
| Contracts merged to `main` under a tagged release | **Pending.** Code is complete and tested. The merge and `v*` tag are the maintainers' step. |
| Test suite passing in public CI with a coverage report published | **Partly evidenced.** Workflows are committed. 328 of 328 pass locally, and coverage is 97.67%. CI on GitHub has not yet been observed. |
| RiskControl demonstrably blocks an oversized deposit in an integration test | Met. [`an_over_limit_deposit_reverts_automatically_with_no_manual_call`](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/contracts/integration_tests/tests/risk_control.rs#L27) |
| Documentation published in the repository | Met in the repository. The live GitHub Pages site has not been checked. |

---

## Defects found while building this

Writing the acceptance tests found real defects in code that was already marked done. All are fixed and covered by regression tests.

1. **Insolvency at any mint rate other than 1.0.** `YTToken` scaled yield by a percentage of notional, but yield accrues on the underlying the position holds. A position minted at rate 1.05 and settled at 1.10 claimed 100.23 SY against 100 SY of custody. The fix changes the index to the closed form `1/rate`, which telescopes to exactly `N / r₀` per position for any starting rate. Tests: `pt_plus_yt_claims_never_exceed_the_deposited_shares_at_any_mint_rate` and `deposit_tokenize_trade_claim_settle_redeem_with_fees_stays_solvent`.
2. **AMM rounding profit at scale.** A one-unit safety margin was too small for large pools. A fee-free round trip on a 1e17-unit pool profited by 6 units. The fix is a pad that scales with reserve size, in the curve solver. Test: `fee_free_round_trips_never_profit_at_any_pool_size`.
3. **Recovery blocked by the pause.** `RecoveryEscrow.seize_sy` unwrapped through `withdraw`, which the pause blocked. This defeated the intent that recovery survive an incident pause. The fix exempts the configured escrow from the SY pause when it is a party. Test: `seizure_works_while_the_market_is_paused`.
4. **Build target.** The documented target `wasm32-unknown-unknown` does not build with the pinned SDK on current Rust. `wasm32v1-none` is required.

## Deviations, design decisions and open items

- **Swap fees do not reach LPs.** The specified split routes 20% of each swap fee to Principal and 80% to the creator. LPs earn PT convergence only. One field in `MarketConfig.split` adds an LP share if wanted. [AMM_DESIGN.md §4](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/AMM_DESIGN.md).
- **The circuit breaker counts each entry point.** A `wrap_and_mint` of 100 uses 200 of the window. This follows "wire the breaker into SYWrapper deposit and PrincipalManager mint" literally. A single intake point would need an origin marker.
- **Per-user entry rate** is a per-account snapshot of the YT index, not a stored initial rate. It is equivalent for accounting and also correct across transfers.
- **`SYWrapper.exchange_rate` is 1.0 in practice.** It tracks deposits and withdrawals, not rebases. A rebasing underlying would need the two rates reconciled.
- **Router registry is admin-only**, by design. It is an allow-list against phishing, not market creation, which stays gated on the underlying's issuer authority.
- **SEP-57 is a Draft (v0.4.0).** The adapter isolates every assumption in one crate. See [COMPLIANCE_ARCHITECTURE.md §3](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/COMPLIANCE_ARCHITECTURE.md).
- **Not built, outside Tranche 1:** fee-change timelock, implied-rate TWAP oracle, `LiquidationAdapter`, a third-party audit, and deployment scripts for the new contracts. [DEPLOYMENT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/DEPLOYMENT.md) documents the steps.
- **Documentation defect in the earlier evidence file.** [TESTNET_DEPLOYMENT_EVIDENCE.md §6.1](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TESTNET_DEPLOYMENT_EVIDENCE.md) has a garbled paragraph about Alice's STA balance. The transactions are correct, but the prose should be rewritten. This is not yet fixed.
- **Test-snapshot files** for the new crates and the integration suite are git-ignored and regenerated on each run.
