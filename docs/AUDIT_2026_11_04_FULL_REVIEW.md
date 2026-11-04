# Full-Protocol Audit — 4 November 2026

Branch: `feat/tranche-1-mvp`. Reviewer stance: senior Soroban/Stellar smart-contract auditor, using
seven independent review passes (six parallel code/test audits covering every contract directory
under `contracts/`, plus a separate roadmap feature-completeness check) rather than one. This
covers every smart contract, every unit and integration test file, and every doc under `docs/`,
including the three features that shipped earlier the same day (Router composed quotes for the
underlying-denominated flows, the extended `OracleAdapter` multi-source/deviation guard, and the
true flash-mint `MarketPool.flash_mint_yt`).

## Scope split across the seven passes

1. `contracts/market_pool` (AMM core + `math.rs`)
2. `contracts/principal_manager`, `contracts/pt_token`, `contracts/yt_token`
3. `contracts/sy_wrapper`, `contracts/oracle_adapter`
4. `contracts/router`, `contracts/market_factory`, `contracts/market_config`,
   `contracts/recovery_escrow`, `contracts/risk_control`, `contracts/permissioning`,
   `contracts/compliance`
5. `contracts/integration_tests` (test-quality review, mutation-testing mindset)
6. All of `docs/*.md`, cross-checked against current contract source
7. The funded `TRANCHE_MILESTONE_ROADMAP` Tranche 1 feature list, plus disposition of the seven
   findings in the older, untracked `AUDIT_REPORT.md` (dated 2026-07-30, pre-dates Tranche 1)

## Findings fixed

| # | Severity | Finding | Fix |
|---|---|---|---|
| 1 | High | `OracleAdapter.set_reference_value`: no bound on an accepted value, including the first-ever submission (exempt from the deviation guard by design). A relay decimal/scale bug could plant a catastrophically wrong genesis value that, once accepted, makes every later *correct* submission read as a `decrease` and get rejected forever — nothing can move the stored value back down without reopening the exact insolvency path the monotonic-value invariant exists to close. | Added `set_value_bounds(caller, min_value, max_value)` / `value_bounds()`, admin-only, unset by default. Checked on every accepted value, including the genesis-exempt case. New error `ValueOutOfBounds`=14, rejects gracefully (`reason = "oob"`), same as every other policy rejection. |
| 2 | Medium | Same function's deviation-bps calculation (`(value - current_price) * BPS_SCALE / current_price`) used raw `i128` operators; an astronomically large `value` would overflow and panic, discarding the `last_rejection()` record this module's own design otherwise guarantees survives a policy rejection. | Switched to `checked_mul`/`checked_div`; an overflow is now treated as "deviation far exceeds threshold" and routed through the existing graceful `reject(..., "deviate")` path instead of panicking. |
| 3 | Medium/High | `MarketFactory.create_engine`/`create_pool_and_escrow` took `tokens`/`engine` as plain caller-supplied structs with no on-chain check that they were what `salt` actually deployed in the earlier phase. A wrong `salt` or a stale/copy-pasted struct would wire a market's *one-time* minter/circuit-breaker/recovery-escrow references to the wrong, unrelated contracts — irreversibly, since `set_minter` and friends can only be called once. Operator-error class, not externally exploitable (`issuer.require_auth()` still gates every call), but high blast radius and previously undetectable until redemption time. | Both entrypoints now recompute the expected addresses from `salt` (the same derivation `predict_market` uses) and revert `Error::TopologyMismatch`(1) on any mismatch, before any deployment or wiring. |
| 4 | Low | `MarketPool.add_liquidity`'s first-deposit ratio guard (`sy_desired * rate / SCALE > pt_desired`) floored the SY leg's value, under-reporting it by up to 1 part in `SCALE` (1e-7 relative) and letting a ratio a hair under par pass. Self-inflicted by the first LP only; no third party harmed. | Changed to `ceil_div(sy_desired * rate, SCALE)`, rounding in the pool's favor like every other reserve-sizing computation in the contract. |
| 5 | Informational | `YTToken::transfer_from` called `sync_index()` twice in a row (harmless — idempotent — but a copy-paste leftover). | Removed the duplicate call. |
| 6 | Informational (hygiene) | `contracts/recovery_escrow/test_snapshots/` held 16 snapshot files referencing test functions that do not exist anywhere in this crate's history — `RecoveryEscrow` has no unit tests of its own by design (its behavior is only reachable through the real SY/PT/YT/pool contracts, tested in `contracts/integration_tests/tests/recovery.rs`, 18 tests). The orphaned directory made a fully-covered, security-critical contract look untested. | Deleted the directory. |
| 7 | Docs accuracy (7 sub-findings, all High/Medium) | `docs/TECHNICAL_SPECIFICATION.md` §9.2 still showed the pre-extension `OracleAdapter` interface (an actively wrong ABI, not just incomplete); §14's constants table still listed the retired `MAX_ORACLE_STALENESS_SECS`; `docs/SECURITY.md` line 89 and two `docs/ARCHITECTURE.md` sequence-diagram lines still said `is_fresh(3600)` instead of `is_available()`; both OracleAdapter's and Router's error tables in §13 were missing error codes added since; §1/§8.1 didn't mention the 12th contract (`MarketFactoryContract`) or any of the Router functions added since Tranche 1; `docs/YIELD_MATH_AND_FEES.md` §7 didn't mention `flash_mint_yt` as an alternative to the capital-up-front flash-mint it described. | All seven corrected in place; see the diff on this commit. `docs/API_REFERENCE.md`, `docs/ROUTER_QUOTES.md`, `docs/ORACLE_SAFETY_EXTENSION.md`, `docs/AMM_DESIGN.md`, `docs/AMM_FLASH_SWAP_AND_SIDEREAL_REVIEW.md`, and `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` were independently checked line-by-line against current code and found already accurate — no changes needed there. |

## Test-coverage gaps closed

All of the following had **zero** prior coverage (not weak coverage — none at all) for the
property named, confirmed by grep before writing each test:

- **`contracts/oracle_adapter/src/test.rs`**: the new value-bounds check (default-unset, validation,
  admin-gating, rejecting an out-of-range genesis value *and* a subsequent value, accepting a
  value within bounds) and the overflow-safe deviation rejection.
- **`contracts/market_factory/src/test.rs`**: `create_engine` rejecting a `tokens` value that
  doesn't match `salt`, and accepting one that does (so the new check isn't over-strict).
- **`contracts/sy_wrapper/src/test.rs`**: deposit and withdraw slippage boundaries (exact minimum
  passes, one unit more reverts), the deposit cap boundary and its release on withdraw, and
  `RiskControl` integration (a wired-but-unregistered consumer fails closed; a registered
  consumer's limit is enforced and reported).
- **`contracts/principal_manager/src/test.rs`**: `recombine` (round-trip at par, after a rate
  increase returns exactly the claimable shortfall, over-balance reverts, post-maturity reverts),
  `claim_protocol_fees`/`claim_creator_fees` (pay the configured payees, zero the bucket, double-
  claim reverts `NothingToClaim`), and `settle_all` idempotency (a second call after further ledger
  advance returns the same frozen rate, not a fresh one).
- **`contracts/integration_tests`**: the `OracleAdapter` multi-source fallback path (primary goes
  stale, fallback refreshes the timestamp and the market keeps working; both go stale and it
  correctly stops) and the deviation guard (an excessive jump rejects gracefully and the market
  keeps working off the last accepted rate) — both previously had *zero* integration coverage
  despite being a same-day feature addition. `flash_mint_yt`'s relationship to the circuit breaker
  was undocumented either way; a new test proves it **is** gated (it mints through
  `PrincipalManager.mint` internally, which does report to `RiskControl`) — correcting an initial
  audit-pass inference that it was exempt. `Router.flash_mint_yt_from_underlying` now has a
  negative counterpart (`underlying_budget` below the real net cost reverts cleanly, including the
  earlier underlying-to-SY wrap rolling back). The mislabeled
  `router_registration_rejects_a_pool_that_belongs_to_a_different_market` (which, despite its name,
  never tested a rejection — contested by its own code comment) was renamed and rewritten to prove
  what it actually can: `Router` resolves `MarketInfo` straight from the pool, so there is no
  "mismatched pool" scenario to construct, and it does so per-pool, not from a shared cache. The
  admin-only sweep in `edge_cases.rs` now also covers `OracleAdapter.set_sources`/
  `set_safety_params`/`set_value_bounds`. The dust-claim test's assertion was tightened from
  "non-negative" to the exact zero its own name promises, using parameters that actually produce a
  1-raw-unit gross yield (the original parameters produced ~198 raw units, so the claim never was
  the "dust" case the test's name claimed).

Full workspace: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked --
-D warnings`, `cargo test --workspace`, and `cargo build --workspace --release --target
wasm32v1-none` all pass after every fix above, with every contract's compiled Wasm still well
under Soroban's 128 KiB limit (largest: `market_pool` at ~77 KB).

## Reviewed and found sound — no fix needed

Both AMM core math (`market_pool`/`math.rs`) and the YT/PT/SY custody and yield-index logic
(`principal_manager`/`pt_token`/`yt_token`/`sy_wrapper`) were independently re-derived against the
exact failure classes this project's own prior audits (`docs/TRANCHE_1_AUDIT.md`, the earlier
Sidereal comparative review) already named: rounding direction (every division rounds in the
pool's or protocol's favor), the curve's SY-shares-vs-SY-value unit discipline (the exact bug class
that broke a competitor's AMM), `flash_mint_yt`'s nested `authorize_as_current_contract` entry
(pinned to exactly the one nested call that needs it, no broader), reentrancy (none — no callback
surface during custody transfers), and the double-claim/stranded-yield invariants around transfers
and flash-redeem. The `Router`/`MarketConfig`/`RecoveryEscrow`/`RiskControl`/`Permissioning`/
`compliance` layer was checked for authorization-bypass, over-seizure, and fail-open paths and
found consistently fail-closed and consistently routed through `principal_compliance` with no
transfer path that skips it.

## Disposition of `AUDIT_REPORT.md`'s seven findings (2026-07-30, pre-Tranche-1)

All seven — H-01 (YT genesis baseline), H-02 (mint skipping oracle-freshness), H-03 (public
`claim_yield` footgun), M-01 (no topology validation), M-02 (RiskControl not wired),
M-03 (oracle decreases not rejected), L-01 (`RiskControl.check_deposit` accepting non-positive
amounts) — are confirmed **fixed** in current code, each independently re-verified against the
live source rather than taken on the later documents' word. See `AUDIT_REPORT.md` for the original
text; this review found no reason to keep that file's findings open.

## One roadmap gap found — resolved by clarification, not new code

`TRANCHE_MILESTONE_ROADMAP.docx`'s `PrincipalManager` feature list (Tranche 1, Deliverable 1)
includes: *"early PT redemption before maturity at an oracle-derived discount."* No function
named for this exists anywhere in `contracts/principal_manager/src/lib.rs`, and both the project's
own `docs/TRANCHE_1_DELIVERABLES.md` and the independent 2026-10-03 Deliverable Review quote the
funded description with this clause silently dropped, with no disposition recorded anywhere.

Asked to decide whether to implement new redemption logic for this, the conclusion (re-derived
from the actual code, not assumed) is **no new code is needed**: `MarketPool.swap_pt_for_sy` /
`Router.swap_pt_for_underlying` — already live, tested, and audited above with no finding — *is*
this feature. `calc_sell_pt` values the SY leg at the live oracle rate before running it through
the curve, and the curve's exponent is a direct function of time-to-maturity, so the discount a
PT holder receives is anchored to the oracle and shrinks to zero at maturity: exactly "an
oracle-derived discount," computed through the curve (which also reflects real supply and demand)
rather than a separate formula. It needs no matching YT, unlike `recombine` (the one function that
could be mistaken for covering this, and does not: it pays the *current* rate with no discount,
and only works if the caller also holds the matching YT).

A standalone, non-market-priced redemption function was deliberately **not** built: `PrincipalManager`
holds no reserve beyond the SY fungibly backing all outstanding PT+YT in aggregate — there is no
per-position earmarking to draw a "spare" discount from. Any separate formulaic discount not
backed by matched counter-liquidity would either shortchange the YT side still outstanding against
that same notional, or require a second pricing mechanism that must track the curve's own
time-decay math exactly forever — a new, ongoing solvency-bug surface for zero economic benefit
over the mechanism already live. The clarification closing this gap is in `docs/AMM_DESIGN.md` §9.
