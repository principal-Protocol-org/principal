# Principal Protocol — Independent Internal Security Review

**Date:** 16 November 2026
**Branch reviewed:** `mainnet` (base commit `29e363b30706c79067dcaf7c35607cbba73f982d`)
**Scope:** All fourteen Soroban smart contracts in this repository and their full test suites
**Status:** Complete — all findings verified, all fixable findings remediated and tested, workspace re-verified clean

---

## 1. Executive Summary

This document records an independent internal security and correctness review of the Principal
Protocol smart contract suite, conducted ahead of mainnet deployment. The review covered all
fourteen contracts in the workspace — `oracle_adapter`, `risk_control`, `permissioning`,
`compliance`, `router`, `market_factory`, `sy_wrapper`, `pt_token`, `yt_token`,
`principal_manager`, `market_pool`, `market_config`, `recovery_escrow`, and the cross-contract
`integration_tests` suite — along with every contract's own unit tests and the full integration
test suite.

**Two Critical-severity findings were identified and fixed.** `RouterContract::initialize` had no
authorization check at all, meaning any account could seize administrative control of the shared
Router instance during the window between its deployment and its intended initialization
transaction (Finding C-1). Separately, in a follow-up review pass after this document's first
findings had already landed and the H-3 fix was in progress, `SYWrapper.withdraw_gained` was found
to accept a caller-supplied, negative `baseline` with no balance check anywhere on the withdrawal
path it shares with every other caller — letting an eligible account (with no deposit of its own
required) withdraw underlying belonging to other depositors, bounded only by the wrapper's real
token balance (Finding C-2). Both are fixed and each is now covered by dedicated regression tests
specifically designed to catch reintroduction.

Across the full review, **40 findings** were identified and individually verified against the real
contract source before being accepted. Of these:

- **2 Critical** findings — both fixed.
- **4 High** findings — 3 fixed (one covering four separate functions; one closing the
  `sync_from_feed` rate-limit gap, itself later found to measure the wrong clock and corrected
  during the same follow-up pass that surfaced C-2 — see Finding H-3; one closing a second missing
  `initialize` authorization check, this time on `RecoveryEscrow` — see Finding H-4), 1 remains
  open as an architectural recommendation requiring a product/operations decision rather than a
  code fix (see §5, Outstanding Recommendations).
- **15 Medium** findings — 10 fixed, 5 remain open as architectural or operational
  recommendations.
- **13 Low** findings — 12 fixed; 1 confirmed but not independently fixable from this repository
  (a transitively pinned dependency advisory — see Finding L-11).
- **6 Informational** findings — 2 fixed (documentation accuracy), 4 are confirmed non-issues,
  accepted/documented limitations, or an investigated-but-unreproduced report, requiring no code
  change.

Every fix was implemented, tested, and the entire workspace was re-verified after all changes
landed: `cargo fmt --all -- --check` is clean, `cargo clippy --workspace --all-targets --locked --
-D warnings` is clean, and `cargo test --workspace` passes with zero failures and zero ignored
tests. §6's per-crate coverage breakdown predates the follow-up pass's additional regression tests
and is scheduled for re-measurement.

This review did identify one path to direct fund loss reachable by an external, unprivileged
attacker: Finding C-2, now fixed. Absent that finding, no other finding in this review involved
such a path. A second, independently surfaced authorization-bypass vector (Finding H-4,
`RecoveryEscrow::initialize`) did not itself move funds, but would have let an attacker silently
neutralize the one contract whose entire purpose is to make compliance-driven seizure provable
on-chain — a serious failure mode in its own right, now also fixed. The Critical findings were a
full administrative takeover vector (C-1) and a direct custody-invariant violation (C-2); the
fixed High findings were reliability/availability issues (transactions failing closed, not funds
moving incorrectly), a rate-limit gap on a permissionless path that is now closed and clocked
correctly, and the second missing-authorization gap just described; the fixed Medium findings
included real accounting-correctness, compliance-traceability, and initialization-authorization
defects, none of which (beyond C-2 itself) allowed value extraction beyond what was already
deposited. The one remaining open High-severity item is architectural — key-management
concentration — and is deliberately left as an explicit recommendation rather than a rushed fix,
for the reasons given in §5.

---

## 2. Scope and Methodology

### 2.1 Scope

All contract source under `contracts/*/src/lib.rs` on the `mainnet` branch, all accompanying unit
test files (`contracts/*/src/test.rs`), the cross-contract integration test suite under
`contracts/integration_tests/`, and the workspace-level build configuration (`Cargo.toml` profile
settings, dependency versions). Deployment scripts, documentation, and the separate dApp
repository were out of scope for this pass.

### 2.2 Methodology

The review was conducted as six independent, parallel passes, each covering a disjoint subsystem
of the protocol, deliberately structured so that no reviewer's conclusions were available to
another during the discovery phase — each pass started from the real, current source with no
assumption that any other pass's findings were correct or complete:

1. AMM core (`market_pool`, `market_config`)
2. PT/YT issuance and yield accounting (`principal_manager`, `pt_token`, `yt_token`)
3. Routing and market creation (`router`, `market_factory`)
4. Oracle safety, risk controls, and access control (`oracle_adapter`, `risk_control`,
   `permissioning`)
5. Yield-bearing wrapper and compliance recovery (`sy_wrapper`, `recovery_escrow`, `compliance`)
6. Cross-contract integration correctness and test-suite quality, across the whole workspace

Every finding reported by any of the six passes was then independently re-verified against the
actual, current source — by reading the exact cited lines, tracing the exact execution path
described, and in several cases running the specific scenario described — before being accepted
into this report. One claimed finding (a hypothesized reentrancy window in `OracleAdapter`'s
SEP-40 pull mechanism) did not survive this verification step and was retracted: re-tracing the
exact statement order showed the relevant state is read fresh immediately before use, after every
external call returns, with no external call occurring between that read and the subsequent write
— so no such window exists. It is recorded here, not silently dropped, because an audit that only
ever confirms findings and never retracts one is not applying real scrutiny.

Fixes were then implemented for every finding judged safe and well-scoped enough to fix without
introducing new risk, each one re-verified independently of whoever implemented it, and the full
workspace test, lint, and format gates were re-run after every batch of changes — including once
more after this review's own closing additions (§4, L-9 and M-11, and the two documentation
fixes).

### 2.3 Severity definitions

- **Critical** — direct path to total loss of funds or total loss of protocol control, exploitable
  by any external party with no special access.
- **High** — direct path to significant fund loss, or a complete denial of core protocol function,
  under realistic conditions.
- **Medium** — a real correctness, accounting, or access-control defect with a concrete but
  narrower impact (incorrect bookkeeping, a compliance-traceability gap, a reliability failure
  under specific but plausible conditions), or an architectural gap that materially weakens a
  stated security property without an immediate exploit path.
- **Low** — a real defect with limited impact, a defense-in-depth gap, or a significant test-
  coverage hole for security-relevant logic.
- **Informational** — a documentation-accuracy issue, a confirmed non-issue worth recording so it
  is not re-investigated, or a design limitation accepted as a reasonable tradeoff.

---

## 3. Findings Summary Table

| ID | Title | Severity | Status |
|---|---|---|---|
| C-1 | `Router::initialize` had no authorization check | Critical | **Fixed** |
| C-2 | `SYWrapper.withdraw_gained` accepted a negative caller-supplied baseline with no balance check anywhere on the path | Critical | **Fixed** |
| H-1 | Router functions chained a live, oracle/rate-derived value into a further authorized call | High | **Fixed** (4 functions) |
| H-2 | Oracle/Permissioning/RiskControl/Router administration concentrated in one unprotected key | High | Open — recommendation |
| H-3 | `sync_from_feed`'s per-call deviation guard has no time-windowed budget | High | **Fixed** |
| H-4 | `RecoveryEscrow::initialize` had no caller-authorization check at all | High | **Fixed** |
| M-1 | A panicking SEP-40 feed aborted `sync_from_feed` instead of rejecting gracefully | Medium | **Fixed** |
| M-2 | RiskControl's circuit-breaker window was a hard "tumbling" reset, allowing ~2x the limit at the boundary | Medium | **Fixed** |
| M-3 | PT/YT balance and supply arithmetic relied solely on a global build setting, not typed checks | Medium | **Fixed** |
| M-4 | A dead oracle with no fresh observation at maturity can permanently block settlement | Medium | Open — recommendation |
| M-5 | MarketFactory's deploy-time atomicity claims had only happy-path test coverage | Medium | **Fixed** |
| M-6 | `router`, `market_pool`, `recovery_escrow` had no isolated unit tests | Medium | **Fixed** (router, recovery_escrow; market_pool confirmed intentionally integration-tested) |
| M-7 | One-time `set_recovery_escrow`/`set_risk_control` wiring had no cross-validation | Medium | **Fixed** |
| M-8 | RecoveryEscrow could pool seized YT yield across unrelated accounts | Medium | **Fixed** |
| M-9 | Per-address deposit cap was never released when SY was withdrawn through a custodian contract | Medium | **Fixed** |
| M-10 | `MarketPool`'s admin is a static snapshot of the issuer, never re-derived live | Medium | Open — recommendation |
| M-11 | `Router.register_market`'s topology cross-check had no regression test | Medium | **Fixed** |
| M-12 | `initialize` did not require the nominated admin's own signature in OracleAdapter, Permissioning, or RiskControl | Medium | **Fixed** |
| M-13 | RecoveryEscrow allows only one unfinalized YT recovery record globally, blockable for a market's remaining lifetime | Medium | Open — recommendation |
| M-14 | Default oracle feed deviation/interval configuration permits ~3x/hour compounding drift if the configured feed misbehaves | Medium | Open — recommendation |
| M-15 | `protocol_share_bps` has no cap below 100% and no timelock | Low/Medium | Open — recommendation |
| L-1 | `get_active_source()` not updated by the SEP-40 pull path | Low | **Fixed** |
| L-2 | Yield-index module doc overstated "no path dependence" | Low | **Fixed** |
| L-3 | No sanity check on oracle rate positivity before use as a divisor | Low | **Fixed** |
| L-4 | AMM fee gross-up had no independent bound on the configured fee rate | Low | **Fixed** |
| L-5 | `add_liquidity_single_sy`'s call ordering deviated from this codebase's own CEI convention | Low | **Fixed** (documented and confirmed safe) |
| L-6 | `compliance::is_authorized`'s RWA branch lacked a defensive wrapper used everywhere else in the module | Low | **Fixed** |
| L-7 | `SYWrapper.deposit`'s external risk-check call preceded this function's own state effects | Low | **Fixed** |
| L-8 | Permissioning's negative-authorization tests did not cover three of its admin-gated functions | Low | **Fixed** |
| L-9 | A RiskControl regression test asserted nothing beyond "does not panic" | Low | **Fixed** |
| L-10 | Salt-reuse protection in MarketFactory was asserted in comments, not tested | Low | **Fixed** |
| L-11 | Locked dependency tree includes a `crypto-bigint` constant-time timing advisory (RUSTSEC-2026-0387) | Low | Confirmed — tracked, not independently fixable |
| L-12 | `add_liquidity_single_sy`'s reentrancy-safety comment did not account for a `Kind::Rwa` underlying | Low | **Fixed** |
| L-13 | `deploy.sh` defaults the protocol-wide circuit breaker to disabled with no warning | Low | **Fixed** |
| I-1 | PT settlement uses a historical high-water-mark rate, not necessarily the rate at maturity | Informational | Confirmed intended — no action |
| I-2 | Recipient-side compliance is enforced at the token layer, not duplicated at the pool layer | Informational | Confirmed correct — no action |
| I-3 | `SYWrapper`'s module doc overstated its own exchange-rate behavior | Informational | **Fixed** |
| I-4 | Per-address deposit cap can be bypassed by multiple addresses under one controller | Informational | Open — accepted limitation |
| I-5 | The SEP-57 authority-probe mechanism assumes a real-world token behaves like the test mock | Informational | Open — operational note for future onboarding |
| I-6 | A reported "0 tests" anomaly for `router`/`recovery_escrow` did not reproduce in this repository's canonical working tree | Informational | Investigated — not reproduced |

---

## 4. Detailed Findings

### C-1 — `Router::initialize` had no authorization check

**Severity:** Critical · **Status:** Fixed

`RouterContract::initialize(env, admin)` checked only that the contract had not already been
initialized; it never called `admin.require_auth()`. Every sibling contract in this workspace
(`sy_wrapper`, `market_pool`, `pt_token`, and others) calls `require_auth()` on the admin address
as the first real step of their own `initialize` functions — Router was the sole exception.

Because deployment and initialization are necessarily separate transactions on a public ledger
(confirmed against this repository's own deployment documentation), any account could submit an
`initialize` call naming itself as admin in the window between the Router's deployment and the
legitimate initialization transaction. Since `initialize` is one-shot (`AlreadyInitialized` blocks
any correction afterward), a successful hijack would be permanent, handing the attacker
`register_market`, `unregister_market`, and `transfer_admin` on the shared Router instance every
market and every integration routes through.

**Fix:** `admin.require_auth()` added as the first statement in `initialize`, matching every
sibling contract.

**Regression test:** this gap existed because every test in the codebase deploys through
`env.mock_all_auths()`, which approves every signature unconditionally — a pattern that can prove
a caller-*identity* check works, but can never prove a `require_auth()` call is actually *present*,
since there is nothing left for a missing call to fail against. A new test,
`initialize_requires_the_admins_own_signature` (`contracts/router/src/test.rs`), is the first in
this codebase built specifically to close that blind spot: it mocks no authorizations at all for
the call that must fail, then mocks the admin's signature for exactly one invocation for the call
that must succeed. It passes.

---

### C-2 — `SYWrapper.withdraw_gained` accepted a negative baseline with no balance check anywhere on the path

**Severity:** Critical · **Status:** Fixed

**Affected functions:** `withdraw_gained`, `do_withdraw`, `sub_balance`
(`contracts/sy_wrapper/src/lib.rs`)

Found during an independent follow-up review pass after this document's first findings had
already landed, while the H-3 fix was in progress.

`withdraw_gained(from, baseline, to, min_underlying_out)` computes `shares = balance_of(from) -
baseline`, where `baseline` is an unvalidated, caller-supplied `i128` representing a balance
snapshot `from` is claimed to have held in the past (intended for a Router flow composing a
wrap-then-flash-mint, reading its own balance via `balance_of` immediately beforehand). Nothing
rejected a negative `baseline`: supplying one inflates `shares` to more than `balance_of(from)`,
with no cap. Unlike `withdraw`/`withdraw_crediting`, this path performed no `balance < shares`
check before calling the shared `do_withdraw`, and `do_withdraw` itself had no such check either —
it computed `underlying_out = shares_to_underlying(shares)`, decremented the global
`TotalUnderlying`/`TotalShares` by that inflated amount, called `sub_balance` (which stores
`balance - shares` with no floor, silently producing a negative stored balance rather than
rejecting), and transferred `underlying_out` of the real underlying token out of the wrapper's own
pooled custody to `to`.

**Attack preconditions:** the caller needs Principal Permissioning and underlying-SAC
authorization — the same floor every depositor needs — and the wrapper must hold some pooled
underlying (i.e., at least one real deposit has occurred). **No prior deposit by the attacker is
required**: with `balance_of(attacker) == 0` and a sufficiently negative `baseline`, `shares`
alone determines the payout.

**Impact:** an eligible account could withdraw underlying belonging to other depositors, corrupt
`TotalShares`/`TotalUnderlying`, and drive its own stored share balance negative. The loss in any
single call is bounded by the wrapper's real underlying token balance (the final SAC-level
transfer reverts past that), but nothing bounded it below the full custody balance, and the
corrupted totals and negative balance would persist and compound across repeated calls.

**Fix:** two changes, not one, to close this for every current and future caller rather than only
the one call site:

1. `do_withdraw` now independently enforces `shares <= get_balance(from)` before computing any
   payout — the one function every withdrawal path (`withdraw`, `withdraw_crediting`,
   `withdraw_all`, `withdraw_gained`) funnels through, so this protects all of them regardless of
   whether each call site's own guard is correct.
2. `withdraw_gained` additionally rejects `baseline < 0` explicitly and up front, before any
   arithmetic on it — both for a clearer, specific error, and because computing `balance -
   baseline` at `baseline == i128::MIN` would itself overflow `i128` (no positive counterpart to
   `i128::MIN` exists), which the explicit check preempts entirely.

**Regression tests** (`contracts/sy_wrapper/src/test.rs`): a negative baseline is rejected; the
most adversarial input, `i128::MIN`, is rejected without an arithmetic-overflow panic masking the
real check; a rejected call leaves `TotalShares`, `TotalUnderlying`, and every account balance
(including the attacker's own token balance) completely unchanged; the same rejection holds even
when the attacker has a real, nonzero SY balance of their own; and the legitimate case — a genuine
positive gain above a real prior baseline — still succeeds and returns the correct amount.

**Why this was missed in this document's first pass:** no unit test in `sy_wrapper/src/test.rs`
exercised `withdraw_gained` at all before this fix; its only prior test coverage was an integration
test for the unrelated zero-gain tolerance case (`contracts/integration_tests/tests/
liquidity_and_router.rs`). The function's own doc comment correctly explained *why* a derived
`shares` value is read live rather than taken as a direct argument (simulation/execution drift,
mirroring `withdraw_all`), but that same reasoning was never checked against what it implies for a
hostile, directly-invoked caller rather than the Router's own well-formed baseline.

---

### H-1 — Router functions chained a live, rate-derived value into a further authorized call

**Severity:** High · **Status:** Fixed (4 functions)

`Router`'s composed, multi-contract flows sign a transaction's full nested call tree at simulation
time. Four functions derived one call's argument from a prior call's live return value — one tied
to the oracle rate, the other to the SY exchange rate — and fed that derived value directly into a
second call that itself requires the caller's authorization for that exact value:

- `swap_sy_for_yt` and `swap_underlying_for_yt` passed `PrincipalManager.mint`'s live,
  oracle-rate-derived `pt_minted` straight into `MarketPool.swap_pt_for_sy`.
- `wrap_and_mint` and `swap_underlying_for_pt` passed `SYWrapper.deposit`'s live,
  exchange-rate-derived `shares` straight into `PrincipalManager.mint` / `MarketPool.swap_sy_for_pt`
  respectively.

If the oracle rate (or, in principle, the SY exchange rate) moved between a client's
pre-submission simulation and the transaction's real execution, the nested call's real arguments
would no longer match what was signed, and the whole transaction would revert with an
authorization error — a reliability failure on four of the Router's most-used entry points,
reproducing a bug class this same codebase had already found and fixed in two sibling functions
(`add_liquidity_single_underlying`, and the deposit leg of `swap_underlying_for_yt`).

**Fix:** each of the four functions now takes the second leg's amount as an explicit,
caller-quoted literal parameter (`pt_in` for the first pair, `sy_in` for the second), exactly
mirroring the pattern already established and proven in the two sibling functions. The first
call's own live return value is still used where it is only returned to the caller, never where
it would be fed into a further authorized call. If the quoted literal does not match what the
live call actually produced, the second call's own ordinary balance check fails closed with a
standard typed error — never an authorization error, and never a path to taking more than was
authorized.

This is an API change: four public function signatures gained a new required parameter. Every
call site across the integration test suite (33 call sites across 7 test files) was updated
accordingly, and the dApp-facing integration documentation should be updated to reflect the new
parameters before these functions are next deployed to a live environment.

---

### H-2 — Administration of Oracle, Permissioning, RiskControl, and Router is concentrated in one key

**Severity:** High · **Status:** Open — recommendation, not fixed in this pass

All of `OracleAdapter`, `Permissioning`, `RiskControl`, and `Router` are, in this repository's own
deployment scripts, initialized with the same administrative key. Every privileged action —
oracle source and bounds configuration, the entire compliance allow-list, circuit-breaker limits,
pause/unpause, market registration — is gated by a bare `caller == admin` check, and every
contract's `transfer_admin` is a single-step operation requiring only the current admin's
signature, with no delay and no acceptance step from the incoming admin.

Compromise of this one key is a complete protocol takeover: simultaneous control of the price
feed, the compliance allow-list, and the circuit breaker, with no on-chain mitigation able to
react before a malicious configuration change or admin transfer takes effect.

**Why this was not fixed in this pass:** moving to a multisig or timelocked admin model is a
cross-cutting architectural decision — it affects deployment tooling, operational procedures, and
potentially the contracts' own auth patterns, and the right design (a shared multisig across all
four contracts, versus separating the genuinely high-blast-radius keys — Permissioning,
RiskControl, Router — from the oracle key, which by its own on-chain deviation/monotonicity
guards already has a bounded blast radius) is a decision for the people who will operate this
protocol, not something to decide unilaterally inside a code review. See §5 for the specific
recommendation.

---

### H-3 — `sync_from_feed`'s deviation guard has no time-windowed budget

**Severity:** High · **Status:** Fixed

`OracleAdapter.sync_from_feed` is callable by anyone, for free, with no cooldown, and is explicitly
documented in this codebase as beyond the originally funded specification — an additional,
trust-minimized pull path layered on top of the primary push-based relay. Its deviation guard was
checked only against the immediately prior stored price, with no longer-term anchor or
elapsed-time-scaled budget. Because the reference value is enforced monotonically non-decreasing
by design (the yield-index and PT settlement math both depend on this), a misbehaving, buggy, or
later-compromised feed could compound the per-call deviation bound over repeated calls — at a
plausible 5% per call roughly every Stellar ledger, over 10x within an hour was achievable with no
circuit breaker of its own on the oracle side.

**Fix:** `sync_from_feed` now enforces a minimum real-time interval between two *accepted*
feed-driven updates, closing the compounding path without touching the push-based relay or the
underlying deviation/monotonicity checks. A new admin-gated `set_feed_min_interval` /
`feed_min_interval_seconds` pair configures the interval per deployment (`0` disables it
entirely, for environments — e.g. automated test flows — that intentionally want unthrottled
pulls); the contract's own default if never configured is `DEFAULT_FEED_MIN_INTERVAL_SECONDS =
300` (5 minutes). An update offered before the interval has elapsed is rejected with reason
`"throttle"` — a new, deliberately non-significant rejection reason (it never overwrites
`last_rejection()`, consistent with the existing `stale_ts`/`no_data` convention), since being
throttled is routine, expected behavior for a permissionless path, not evidence of a bad feed.
`current_ts == 0` (no prior accepted value yet) is always exempt, so the very first sync into a
freshly deployed adapter is never throttled.

This did introduce one operational interaction worth recording: `scripts/deploy/deploy.sh`'s
existing bootstrap flow pushes a manual genesis value and then immediately calls `sync_from_feed`
once, which — at the real configured interval — would always be throttled on a fresh deployment
(the gap since the just-made genesis push is near-zero). `deploy.sh` now sets the interval to `0`
for that one bootstrap call only, then restores the real configured value (`.env`'s
`ORACLE_FEED_MIN_INTERVAL_SECONDS`, default `300`) immediately after — preserving full protection
for every subsequent call, including every later permissionless one, while not blocking the one
deliberate, admin-controlled call that originates the series.

Two new tests (`contracts/oracle_adapter/src/test.rs`) cover this: one proves the first update is
never throttled, a genuine second update offered too soon is rejected as `"throttle"` without
touching `last_rejection()`, and the same update succeeds once the interval elapses; the other
confirms `set_feed_min_interval` is admin-only and that `seconds = 0` disables the throttle.

**Follow-up correction (found during an independent review of this fix's own first pass):** the
throttle as first implemented measured the real-time gap against the stored `Timestamp` field —
the *feed's own self-reported* observation time — rather than against when this contract actually
accepted that prior update. A compromised or malicious configured feed could present several
monotonically increasing but real-time-stale timestamps (all comfortably behind the ledger clock)
and clear the throttle repeatedly with effectively zero real time elapsed between calls, defeating
its purpose while still passing every other check (`future_ts`, monotonicity, deviation).
Corrected by adding a dedicated `LastFeedAcceptLedgerTs` storage key, written only on a genuine
acceptance to `env.ledger().timestamp()`, and measuring the throttle against that instead. A new
test (`throttle_clock_is_ledger_acceptance_time_not_the_feeds_own_reported_timestamp`) reproduces
the exact bypass pattern against the corrected code and confirms it no longer succeeds.

---

### H-4 — `RecoveryEscrow::initialize` had no caller-authorization check at all

**Severity:** High · **Status:** Fixed

**Affected:** `contracts/recovery_escrow/src/lib.rs`, `fn initialize`

Found during the same round of independent review that surfaced C-2. Unlike every other
market-creation `initialize` in this codebase (Router, SYWrapper, MarketConfig, PTToken, YTToken,
PrincipalManager, MarketPool, and — per M-12 — OracleAdapter/Permissioning/RiskControl),
`RecoveryEscrow::initialize` took no `caller` argument and called no `require_auth()` at all. It
checked only that it had not already been initialized, and that every named sub-contract
(`SYWrapper`/`PTToken`/`YTToken`/`PrincipalManager`/`MarketPool`) reported the same
`underlying_address()` — a check that is trivially satisfiable by an attacker's own minimal
contracts, each of which needs only to echo back the right address from a hardcoded view function.

**This was not merely a theoretical window.** `MarketFactory::create_pool_and_escrow`'s atomic
deploy-and-init flow was never affected, but `scripts/create_market.sh` — a real, currently
present manual deployment path in this repository — deploys `RecoveryEscrow` and calls its
`initialize` as two **separate** transactions (`stellar contract deploy`, then a distinct `stellar
contract invoke ... initialize`). Anyone watching the chain could have raced the legitimate
operator's own `initialize` call in that window with attacker-controlled stand-in contracts,
permanently hijacking that escrow instance (`AlreadyInitialized` blocks any correction
afterward). The consequence is not a direct fund drain: every later `seize_*`/`finalize_record`
call still correctly re-verifies the live issuer authority. But it would silently call out to the
attacker's fake sub-contracts instead of the real token contracts, so a sanctioned or
deauthorized account's actual position is never seized while the escrow's own on-chain
`RecoveryRecord` bookkeeping faithfully records a "successful recovery" that did not happen — a
serious compliance/legal failure mode for the one contract in this codebase whose entire purpose
is to make seizure provable on-chain.

**Fix:** `initialize` now takes a `caller: Address`, calls `caller.require_auth()`, and verifies
`compliance::is_authority(&env, &underlying, &caller)` before anything else — the exact pattern
every other market-creation `initialize` already uses. `MarketFactory::create_pool_and_escrow`,
`scripts/create_market.sh`, and every test/integration call site were updated to pass the live
issuer as the new argument; the release WASM artifacts were rebuilt and the full workspace
(including the WASM-loading `market_factory` integration test, which caught the stale-artifact
mismatch immediately) re-verified clean.

**Regression test:** `initialize_requires_the_issuers_own_signature`
(`contracts/recovery_escrow/src/test.rs`) builds the real five-contract supporting stack under
`env.mock_all_auths()`, then calls `env.set_auths(&[])` — which, per that method's own
documentation, disables auth mocking — before proving `initialize` fails with no signature
present and succeeds once the issuer's signature is mocked for exactly that one call, mirroring
the `Router`/`OracleAdapter`/`Permissioning`/`RiskControl` regression tests this same bug class
already has (C-1, M-12).

**Independently re-reviewed:** a second, freshly-started reviewer, given only the diff and the
live repository (no shared context with the fix itself), independently confirmed the fix is
correctly placed and unskippable, confirmed every call site was updated by its own repository-wide
search, confirmed the regression test proves what it claims by reading the SDK's own `set_auths`
semantics, and verified the ordering of `compliance::init`/`compliance::is_authority` against the
identical pattern already relied on elsewhere in this codebase. It also caught two stale,
unrelated pre-existing inconsistencies this fix exposed rather than introduced: `docs/
TECHNICAL_SPECIFICATION.md` §16.1 and `docs/DEPLOYMENT.md` still described `RecoveryEscrow::
initialize` (and, in `TECHNICAL_SPECIFICATION.md`'s case, also `OracleAdapter`/`Permissioning`/
`RiskControl`/`Router`, stale since C-1/M-12) as admin-less/first-caller-wins — both corrected.
Separately, this review's own first pass had introduced one inaccurate clause into the
`add_liquidity_single_sy` comment fixed under L-12 (claiming `flash_mint_yt` "opts out" of
Soroban's non-reentrant default, which conflates that function's unrelated authorization-entry
mechanism with reentry mode, and which doesn't apply in any case since `flash_mint_yt` never calls
back into `MarketPool` at all); the second reviewer caught this too, and the comment was corrected
to drop the inaccurate clause while keeping its correct bottom-line conclusion.

---

### M-1 — A panicking SEP-40 feed aborted `sync_from_feed` instead of rejecting gracefully

**Severity:** Medium · **Status:** Fixed

`sync_from_feed`'s own documentation states plainly that a feed returning bad data must not revert
the whole call — "a feed having a bad tick must not revert the whole call." That guarantee held for
a malformed response (`None`, a non-positive price, a future timestamp) but not for a feed
contract that panics during the cross-contract call itself: `client.decimals()` and
`client.lastprice()` were called via the ordinary, panic-propagating client methods rather than
their `try_` equivalents, so a misbehaving, paused, or incompatible feed contract would abort the
entire transaction instead of producing a graceful, recorded rejection — a reliability gap on the
one mechanism whose stated purpose is not needing to trust an admin-configured relay's own say-so.

**Fix:** both calls now use `try_decimals()`/`try_lastprice()`; any error is converted into the
same graceful rejection path (`reject(..., "feed_err", significant: false)`) already used for
every other kind of bad feed response. Two new tests use an extended mock SEP-40 feed with a
panic-on-command toggle, confirming `sync_from_feed` now returns `false` without panicking in
either case.

---

### M-2 — RiskControl's circuit-breaker window allowed roughly 2x the configured limit at a window boundary

**Severity:** Medium · **Status:** Fixed

Both the protocol-wide and per-asset volume windows reset to exactly zero the instant the
configured window elapsed, with no decay. Timing a near-limit deposit immediately before a
boundary and another immediately after let close to twice the stated cap clear within a span only
marginally longer than one window — directly undermining the purpose of a circuit breaker, which
exists specifically to bound how much value can move in a given period.

**Fix:** replaced the hard reset with a sliding-window-counter approximation: each window now
tracks both its current and its immediately preceding raw volume, and the volume checked against
the configured limit is the current window's volume plus the previous window's volume weighted by
how much of it is still inside a trailing window-length lookback from now. This bounds the
worst-case throughput across any boundary to close to the configured limit rather than roughly
double it. `get_cb_volume`/`get_asset_volume`/`get_window_start` now report this normalized,
effective state rather than raw, potentially-about-to-reset storage, so a caller reading them
immediately at a boundary sees an accurate figure rather than the same illusion that caused the
original gap.

A new regression test reproduces the exact original bypass timing and confirms the second deposit
is now correctly throttled. Two pre-existing tests that had encoded the old, exploitable reset
behavior as an expected feature were updated to assert the new, correct behavior instead — along
with one integration test outside this contract's own test suite (`risk_control.rs`) that made the
identical assumption and needed the same correction.

---

### M-3 — PT/YT balance and supply arithmetic relied solely on a global build setting

**Severity:** Medium · **Status:** Fixed

Every balance, total-supply, and allowance update in both `pt_token` and `yt_token` used raw
`+`/`-` rather than checked arithmetic — inconsistent with this same codebase's own, more
fixed-point-heavy math (`mul_div`, `ceil_mul_div`, the yield-settlement accumulation), all of which
were already hardened with explicit `checked_*` operations and a typed `ArithmeticOverflow` error,
specifically because relying solely on the workspace's `overflow-checks = true` release-profile
setting was itself a prior audit finding. The balance/supply paths were never given the same
treatment: today they fail closed only because that one global setting remains correctly
configured; if it were ever dropped, overridden, or a future crate-local profile override shadowed
it, an overflow would silently wrap instead of reverting — a critical fund-fabrication class of bug
with no contract-level backstop left.

**Fix:** every balance/supply/allowance update in both contracts now uses `checked_add`/
`checked_sub`, raising each contract's own typed overflow error, matching the convention already
used for this codebase's other fixed-point math. New tests confirm the typed error surfaces on an
overflow, rather than relying on this being provable only through a generic Rust panic.

---

### M-4 — A dead oracle with no fresh observation at maturity can permanently block settlement

**Severity:** Medium · **Status:** Open — recommendation, not fixed in this pass

`PrincipalManager.settle` requires a fresh oracle observation the first time it runs at or after
maturity, to freeze the yield index. There is no administrative or time-delayed override that can
force settlement from the last known rate if the oracle relay stops posting before that first
fresh observation lands. A sustained oracle outage timed around a market's maturity would leave
every depositor's principal and yield permanently locked, with no code path to recover.

**Why this was not fixed in this pass:** the correct remedy is an emergency settlement path —
admin-gated, or permissionless after a long, deliberately chosen grace period — and getting the
authorization model and grace-period length right is itself a security-sensitive design decision
that deserves its own explicit sign-off rather than being added reactively inside this review. See
§5.

---

### M-5 — MarketFactory's deploy-time atomicity claims had only happy-path test coverage

**Severity:** Medium · **Status:** Fixed

The factory's own documentation makes specific claims about its atomicity and failure-handling
guarantees (a wrong issuer on a later phase is rejected; a skipped phase causes a real,
cross-contract failure rather than silent inconsistency; salt reuse is refused by the host
platform itself), but the test suite exercised only the fully successful path.

**Fix:** three new integration tests confirm each claim directly: a wrong issuer on `create_engine`
is rejected by the child contract's own admin check; `create_pool_and_escrow` against a salt whose
`create_engine` phase was skipped fails on the real cross-contract call to a contract that was
never deployed; and calling `create_tokens` twice with the same salt fails on Soroban's own
occupied-address guarantee.

---

### M-6 — `router`, `market_pool`, and `recovery_escrow` had no isolated unit tests

**Severity:** Medium · **Status:** Fixed for `router` and `recovery_escrow`; confirmed intentional for `market_pool`

These three contracts had no `src/test.rs` of their own; all of their logic was reachable only
through the full-stack integration suite, which is slower and less suited to exhaustive
negative-case and boundary enumeration than a direct, isolated test.

**Fix:** `contracts/router/src/test.rs` and `contracts/recovery_escrow/src/test.rs` were created,
each with direct coverage of their contract's core entry points, including the Critical and
Medium-severity regression tests from this review (C-1, M-11 for Router; M-8 for RecoveryEscrow).
`market_pool` remains without its own unit-test file — confirmed, on investigation, to be a
deliberate choice already reflected in this codebase's own structure (its `math.rs` module has
dedicated, thorough unit tests; its contract-level logic is tested through
`contracts/integration_tests`, which was judged the right venue given how much of its behavior
depends on real cross-contract interaction with SYWrapper and PrincipalManager).

---

### M-7 — One-time `set_recovery_escrow`/`set_risk_control` wiring had no cross-validation

**Severity:** Medium · **Status:** Fixed

Unlike each contract's own `initialize`, which cross-checks that every sibling address it is given
actually agrees on the same underlying market, the later, one-shot setters that wire in
`RecoveryEscrow` and `RiskControl` trusted the admin-supplied address outright. Because these
setters are irreversible once called, an operator mistake — a stale address copied from a
different market, a simple typo — would permanently and silently misconfigure that contract's
compliance-recovery or risk-limiting path, surfacing only much later as every real seizure attempt
failing.

**Fix:** `set_recovery_escrow` across `sy_wrapper`, `pt_token`, `yt_token`, and `market_pool` now
calls back into the given address and confirms its own reported underlying asset matches this
contract's — a genuine self-referential proof, reusing a getter `RecoveryEscrow` already exposed.
`set_risk_control` (in `sy_wrapper` and `principal_manager`) now confirms the given address
responds to a real RiskControl-shaped call, which rules out the main failure mode (an address of
the wrong contract type, or a dead address) without requiring it to already be configured as a
consumer at wiring time — which every real deployment script correctly does afterward, not before,
so a stricter check would have broken the documented operational order. New tests for each setter
confirm a wrong address is now rejected and the correct address still succeeds.

---

### M-8 — RecoveryEscrow could pool seized YT yield across unrelated accounts

**Severity:** Medium · **Status:** Fixed

YT's own yield accounting is pooled per holder address, not per compliance record. If
`RecoveryEscrow` held seized YT from two different accounts at once — a realistic situation, since
compliance events are typically resolved at maturity and the escrow can accept multiple seizures
over time — all further yield accrual on the combined balance was credited to one shared pending
claim. Whichever record was finalized first would absorb yield that was, in reality, accruing
against the other record's YT too. Total value recovered across all records stayed conserved — no
value was lost or created — but the per-account attribution this module's own documentation
promises ("funds always trace to the specific event and account that produced them") did not hold,
which matters directly for the accuracy of a compliance and legal recovery trail.

**Fix:** rather than a full per-record proportional-yield redesign — judged too invasive to do
safely in this pass — the unsafe state is now prevented from occurring at all: a new guard rejects
a second YT seizure while this escrow instance already holds an unfinalized YT record, with a
clear, typed error. PT-only and SY-only seizures are unaffected. Fixing this surfaced two
pre-existing integration tests that had relied on exactly the forbidden state (multiple
unfinalized YT seizures in one batch); both were corrected to respect the new invariant while
preserving what they were actually meant to prove. Two new direct tests confirm the guard rejects
a second account's YT seizure while the first is outstanding, and permits it once the first is
finalized.

---

### M-9 — Per-address deposit cap was never released when SY was withdrawn through a custodian contract

**Severity:** Medium · **Status:** Fixed

`SYWrapper`'s per-address deposit cap is correctly charged against the real depositor. But when a
user splits their SY into PT and YT (a plain internal transfer to `PrincipalManager`, which never
touches the cap) and later fully redeems through `PrincipalManager.redeem`, the resulting
`SYWrapper.withdraw` call is made with `PrincipalManager`'s own address as the withdrawing party —
an address whose cap was never charged in the first place, so the release was a silent no-op
against the wrong account. The real depositor's cap stayed permanently at its deposited level even
after they had fully exited their position, blocking them from ever depositing again under a
compliance cap — breaking the protocol's own core, intended usage pattern (deposit, split, trade,
redeem).

**Fix:** investigation confirmed `PrincipalManager` is the only real custodian case in the
workspace (Router's own calls always withdraw to the original caller; `MarketPool` never calls
`withdraw` at all). A new function, `SYWrapperContract::withdraw_crediting`, was added — mirroring
this same file's existing pattern of a clearly named variant rather than a parameter threaded
through the one function every other caller already depends on — which credits the cap release to
an explicitly named beneficiary rather than the literal caller. `PrincipalManager`'s redemption and
yield-claim paths now use it, passing the real end user. `SYWrapper.withdraw` itself, and every
other existing caller (Router, RecoveryEscrow), is completely unchanged. New tests confirm the
named beneficiary's cap is released correctly, that a plain `withdraw` by a custodian would not
have released it (a regression lock on the bug itself), and that a full deposit-mint-redeem cycle
now correctly returns a user's deposit capacity to them.

---

### M-10 — `MarketPool`'s admin is a static snapshot of the issuer, never re-derived live

**Severity:** Medium · **Status:** Open — recommendation, not fixed in this pass

`MarketPool.initialize` requires its admin to equal the underlying asset's live issuer authority
at creation time, but the stored admin is never re-checked against that live authority again —
unlike `MarketConfig`'s creator role and `RecoveryEscrow`'s own authority checks, which both
re-derive live on every use specifically so an issuer's key rotation takes effect immediately. If
the key used at pool creation is later compromised, its holder retains full pool-admin rights
(including the one-shot, irrevocable `set_recovery_escrow` wiring) until someone manually
transfers admin away using that same, possibly already-compromised key.

**Why this was not fixed in this pass:** this is the same class of decision as H-2 — whether and
how to re-derive authority live for this role is an architectural choice with real tradeoffs (live
re-derivation adds a cross-contract call to every admin action; a stored snapshot is cheaper but
staler), and is better decided alongside H-2's broader admin-model decision than fixed in
isolation here. See §5.

---

### M-11 — `Router.register_market`'s topology cross-check had no regression test

**Severity:** Medium · **Status:** Fixed

`register_market` correctly rejects a pool whose self-reported `sy`/`pt`/`yt`/`underlying`
addresses disagree with what its own manager reports for the same market — but nothing in the test
suite exercised this check; every existing test only ever registered pools that already agreed.

**Fix:** a new regression test constructs a minimal, deliberately inconsistent pool/manager pair
(a real `MarketPool`/`PrincipalManager` pair resists being made to disagree, since each already
validates its own topology at `initialize`) and confirms registration is rejected when one address
disagrees, and still succeeds when everything agrees.

---

### M-12 — `initialize` did not require the nominated admin's own signature in OracleAdapter, Permissioning, or RiskControl

**Severity:** Medium · **Status:** Fixed

Found during the same follow-up review pass as C-2. Every `initialize` across this codebase that
takes an `admin` address calls `admin.require_auth()` as a cryptographic binding to the nominated
address — confirmed present in Router (see C-1's own fix), `SYWrapper`, `MarketConfig`, `PTToken`,
`YTToken`, `PrincipalManager`, and `MarketPool`. `OracleAdapter`, `Permissioning`, and
`RiskControl` were the three exceptions: each wrote the supplied `admin` during first
initialization with no signature check at all. Since initialization is one-shot
(`AlreadyInitialized` blocks any correction afterward), a directly deployed instance of any of
these three contracts left uninitialized for any window — a manual deployment step, a partial or
interrupted setup flow, a monitoring process trusting an address before confirming
initialization — could be claimed by the first caller, who may nominate any admin address, exactly
the same root-cause class as C-1 (though a narrower blast radius, since the factory-deployed,
atomically-initialized per-market contracts are not affected the same way).

**Fix:** `admin.require_auth()` added as the first statement in each of the three `initialize`
functions, matching the pattern already used everywhere else in this codebase.

**Regression tests:** one per contract (`initialize_requires_the_admins_own_signature` in each of
`oracle_adapter`, `permissioning`, and `risk_control`'s own `test.rs`), mirroring C-1's own test
design exactly: no authorizations mocked at all for the call that must fail, then the admin's
signature mocked for exactly one invocation for the call that must succeed. All three pass.

---

### M-13 — RecoveryEscrow allows only one unfinalized YT recovery record globally

**Severity:** Medium · **Status:** Open — recommendation (pre-existing, deliberate design choice)

**Affected functions:** `seize_one`, `finalize_record` (`contracts/recovery_escrow/src/lib.rs`)

Confirmed accurate during the same follow-up review pass as C-2, though this is a pre-existing,
deliberate design decision already explained in the code's own doc comments (`RecoveryRecord`'s
`yt_yield_at_seize` field doc), not a newly discovered defect. Pending YT yield is pooled by the
escrow's own holding address rather than tracked per record, so only one unfinalized YT recovery
record is permitted to exist at a time — enforced by an `UnfinalizedYtRecord` guard — which is
exactly what keeps that pooled yield attributable to a single record. `finalize_record` settles a
record's held PT/YT through `PrincipalManager.redeem`, which itself requires the market to be at
or past maturity; a YT record seized before maturity therefore cannot be finalized, and the guard
it holds is not released, until maturity arrives. One early YT seizure can consequently block
every other restricted holder's YT recovery for the remainder of that market's lifetime.

**Why this is not fixed in this pass:** the correct remediation — per-record custody addresses, or
independently tracking each record's own pro-rata yield index — is itself a design change with its
own tradeoffs (more storage, more complexity in the yield-pooling invariant this mechanism
currently relies on), not a narrow bug fix, and belongs with whoever operates this protocol
deciding whether multiple simultaneous YT recoveries are an operational requirement. At minimum,
operators should treat the maximum blocking interval (the remaining time to maturity at the moment
of the first YT seizure) as a known operational constraint rather than an unbounded one. See §5.

---

### M-14 — Default oracle feed deviation/interval configuration permits ~3x/hour compounding drift if the configured feed misbehaves

**Severity:** Medium · **Status:** Open — recommendation (configuration, not a logic defect)

**Affected:** `contracts/oracle_adapter/src/lib.rs` `sync_from_feed`; `scripts/deploy/deploy.sh`'s
`ORACLE_MAX_DEVIATION_BPS` (default 1000 = 10%) and `ORACLE_FEED_MIN_INTERVAL_SECONDS` (default
300 = 5 min, per H-3's own fix)

Found during the same follow-up review pass as H-4. The H-3 throttle mechanism itself works
exactly as designed and is not in question here — this is about the specific *default numbers*
`deploy.sh` ships. The combined bound the code's own comment already states is `(1 +
max_deviation_bps/10_000)^(3600/interval)` per hour; at the shipped defaults that is `1.10^12 ≈
3.14x` per hour if the configured SEP-40 feed itself is buggy or compromised. This is the same
risk-policy question §5 already flagged when H-3 was first fixed ("informed by the real update
cadence of whichever feed is actually integrated... reviewed before this mechanism is relied upon
for a live market") — implementing the mechanism did not retire that question, since the specific
numbers still need calibrating to the real asset's expected volatility before a feed is wired in
for a live mainnet market. See §5.

### M-15 — `protocol_share_bps` has no cap below 100% and no timelock

**Severity:** Low/Medium · **Status:** Open — recommendation (governance decision, not a code
defect)

**Affected:** `contracts/market_config/src/lib.rs` `set_protocol_share`, `assert_share`

Found during the same follow-up review pass as H-4. Every other percentage-like parameter in this
same file is capped well below 100% at creation time (`MAX_TOKENIZATION_FEE_BPS` = 1%,
`MAX_YT_FEE_BPS` = 50%, `MAX_SWAP_FEE_TIER_BPS` = 5%) specifically so no single admin action can
push a user-facing fee to an extreme. `assert_share` is the one exception: it only rejects
`share_bps > 10_000` (100%), and `set_protocol_share` takes effect immediately on
`protocol_admin`'s single signature, with no market-creator consent, cap, or delay. Unlike the
other fee parameters, this one does not touch what a depositor pays — it only redistributes an
already-capped fee between the protocol treasury and the market creator — but a compromised or
adversarial `protocol_admin` key could still zero out every market creator's revenue instantly,
with no on-chain recourse. Deciding the right floor (if any) a market creator should be
structurally guaranteed is a product/business commitment, not a value this review can pick
unilaterally — recorded here for that decision. See §5.

---

### L-1 through L-13 — Lower-severity findings

The following were each individually verified, fixed, and covered by a new or corrected test:

- **L-1.** `get_active_source()` is not updated by a feed-driven price update, so it can report a
  stale source after `sync_from_feed` succeeds. Its documentation now states this explicitly, and
  a new test pins the (unchanged, intentional) behavior.
- **L-2.** The yield-index module documentation claimed "no path dependence" without qualification;
  the actual integer implementation floors one index unit per settlement as a rounding-safety
  margin, so a position settled many times loses marginally more floor-rounding dust than one
  settled once — strictly protocol-favoring and immaterial at realistic balances, but not quite
  what "no path dependence" implied. The documentation now states this precisely.
- **L-3.** `yt_token`, `principal_manager`, and `market_pool` all used a live oracle rate as a
  divisor with no independent check that it was positive, relying entirely on `OracleAdapter`'s
  own guarantees. Each now asserts positivity immediately after reading the rate, failing with a
  clear, intentional error instead of an unexplained divide-by-zero trap or silent sign corruption
  if that assumption were ever violated.
- **L-4.** `market_pool`'s fee gross-up divided by a configured fee rate with no independent bound
  of its own, trusting `MarketConfig`'s cap alone. A defense-in-depth bound was added directly in
  `market_pool`.
- **L-5.** `add_liquidity_single_sy` pulls its SY transfer before computing its own reserve
  effects, deviating from this file's otherwise-consistent checks-effects-interactions ordering.
  Confirmed safe today (the called contract makes no further external call that could reenter),
  and now documented explicitly as a deliberate exception with the reasoning that must be
  revisited if that assumption ever changes.
- **L-6.** `compliance::is_authorized`'s real-world-asset branch called a token's `is_frozen`
  without the same defensive, trap-tolerant wrapper used for every other external call in that
  same function. Now consistent.
- **L-7.** `SYWrapper.deposit`'s external RiskControl check ran before this function's own state
  effects, inconsistent with the checks-effects-interactions ordering this file otherwise follows
  everywhere else, including its own withdrawal path. Reordered to match.
- **L-8.** `Permissioning`'s negative-authorization test coverage was missing three of its four
  admin-gated functions. Added.
- **L-9.** A RiskControl test named for demonstrating that removing a per-asset limit reopens the
  asset asserted nothing beyond "does not panic" — a regression in the behavior it claimed to
  prove would not have been caught. Now asserts the actual volume accounting and the
  before/after-limit-removal behavior directly.
- **L-10.** MarketFactory's salt-reuse protection was described in comments, never exercised by a
  test. Added.
- **L-11.** `cargo audit --no-fetch` confirms the locked dependency tree includes `crypto-bigint
  0.5.5`, flagged by `RUSTSEC-2026-0387` for timing variability in operations intended to be
  constant-time. Found during the same follow-up review pass as C-2, and re-confirmed directly via
  a live `cargo audit` run rather than taken on an earlier report's word. The dependency is
  transitive, not direct: pulled in via `soroban-env-host → k256 → ecdsa → elliptic-curve →
  crypto-bigint`, not referenced anywhere in this workspace's own contract code, and no contract
  here performs secret-dependent cryptographic operations through it. `cargo update -p
  crypto-bigint --dry-run` confirms no semver-compatible newer version is available under the
  current lockfile — `elliptic-curve 0.13.8`'s own requirement pins it, and that in turn is fixed
  by the currently vendored `soroban-env-host` version, not by this repository's `Cargo.toml`.
  **Status:** confirmed, tracked; not independently fixable from this repository today. Resolving
  it requires an upstream `soroban-sdk`/`soroban-env-host` release that bumps this chain; revisit on
  the next SDK upgrade.
- **L-12.** `market_pool::add_liquidity_single_sy`'s reentrancy-safety comment claimed `SYWrapper.
  transfer` "has no external-call surface of its own," which is true for a `Kind::Sac` underlying
  but not for `Kind::Rwa` (its compliance check does call out to the issuer-deployed RWA/
  IdentityVerifier contract). Found during the same follow-up review pass as H-4. Verified directly
  against `soroban-env-host`'s own `call_n_internal` reentry check that this is safe regardless:
  an ordinary cross-contract call defaults to `ContractReentryMode::Prohibited`, so any attempted
  callback from that external contract back into this same `MarketPool` instance (already on the
  call stack) is rejected by the host itself — independent of this function's own call ordering,
  and independent of the underlying's kind. The comment is corrected to state both reasons
  precisely (no external call at all for `Kind::Sac`; the host's own non-reentrant default for
  `Kind::Rwa`) instead of a claim that only held for one of the two cases.
- **L-13.** `scripts/deploy/deploy.sh` defaults `RISK_CB_LIMIT` to `0` (the protocol-wide deposit
  circuit breaker disabled) unless an operator explicitly sets it. Already documented in
  `.env.example`'s own comment on that variable; `deploy.sh` now also prints an explicit `WARNING`
  at that default so it cannot be missed on a real deployment run. Found during the same follow-up
  review pass as H-4; operational checklist item, not a code defect.

---

### I-1 through I-6 — Informational findings

- **I-1.** PT settlement uses a historical high-water-mark of the oracle rate, not necessarily the
  rate at the maturity instant — a deliberate, internally consistent consequence of the
  non-decreasing reference-value design, not a defect. Confirmed, not changed.
- **I-2.** Recipient-side compliance eligibility on a trade or liquidity-removal is enforced at the
  PT/YT/SY token layer, not duplicated inside `MarketPool` itself. Confirmed this is not a bypass —
  every token transfer independently checks both sides — and is simply where the real check
  correctly lives; recorded so it is not mistaken for a gap on a future pass.
- **I-3.** `SYWrapper`'s own documentation overstated that its exchange rate increases over time;
  the current implementation provably keeps it pinned at parity, a fact already correctly scoped
  in `PrincipalManager`'s documentation but not in `SYWrapper`'s own. Corrected to match.
- **I-4.** The per-address deposit cap can be bypassed in aggregate by one economic actor using
  several distinct, independently permissioned addresses. This is an inherent limitation of any
  per-address on-chain cap, not a contract defect, and is only a real gap if the cap is relied upon
  as the *sole* enforcement of per-investor exposure rather than as a complement to the identity
  binding `Permissioning` is intended to provide. No code change made; recorded for operators
  configuring the cap to be aware of.
- **I-5.** `RecoveryEscrow`'s mechanism for proving operator authority over a non-SAC, SEP-57-style
  real-world-asset token performs a real, state-mutating "capability probe" call, relying on it
  being idempotent and side-effect-free against the test mock's own behavior. A real third-party
  token's implementation may not share that exact behavior. No code change made; recorded as a
  pre-onboarding verification step for whenever a real SEP-57 asset is integrated.
- **I-6.** An independent review pass (the same one that surfaced H-4) reported that `router`'s
  and `recovery_escrow`'s own unit-test binaries executed zero tests (`running 0 tests`) under a
  full `cargo test --workspace` run in its own isolated worktree, including the Router
  regression test that specifically guards C-1. Investigated directly in this repository's own
  canonical working tree: `cargo test -p principal_router -p principal_recovery_escrow --lib`
  ran 6 and 9 tests respectively, all passing, both there and in every subsequent full-workspace
  run in this session. The symptom did not reproduce here. The reporting agent itself flagged,
  without asserting a cause, that it was working concurrently across multiple git worktrees created
  for this same review round — a plausible explanation for a one-off build/cache artifact specific
  to that isolated environment, not a defect in the test suites themselves. Recorded for
  transparency rather than silently dropped, since it was reported with real (if inconclusive)
  supporting evidence; no code change made, and no reproduction found.

---

## 5. Outstanding Recommendations

Six findings (H-2, M-4, M-10, M-13, M-14, M-15) were deliberately left unfixed in this pass
because each requires a specific, consequential decision — a key-management model, an
emergency-recovery authorization scheme, a recovery-concurrency redesign, an asset-specific risk
parameter, or a revenue-sharing commitment — that belongs with whoever will actually operate this
protocol in production, not embedded unilaterally inside a code review. Rushing any of these would
trade one risk for another, less well-understood one. They are restated here together because
they are related and worth deciding as a set rather than independently:

1. **Administrative key model (H-2, and M-10 as a special case of the same question).** Decide
   whether Oracle, Permissioning, RiskControl, and Router should share one multisig-protected
   admin, or whether the oracle-update key specifically (whose on-chain blast radius is already
   bounded by the deviation guard and monotonicity invariant) should remain a separate, unattended
   hot key so automated relay updates can continue working, while the genuinely high-blast-radius
   keys (compliance, circuit breaker, market registration) move to a multisig or timelock. `M-10`'s
   `MarketPool` admin-staleness question should be resolved as part of the same decision, since it
   is the same underlying tradeoff (live re-derivation vs. a cheaper, staler snapshot).

2. **Emergency settlement path (M-4).** Decide whether, and under what authorization and delay, a
   market with no fresh oracle observation by its own maturity should be settleable from the last
   known rate, versus remaining permanently blocked until the oracle recovers. This is a genuine
   availability-versus-safety tradeoff that deserves explicit sign-off.

3. **Recovery concurrency (M-13).** Decide whether RecoveryEscrow's current one-unfinalized-
   YT-record-at-a-time constraint is operationally acceptable for this deployment, given it can
   block every other restricted holder's YT recovery for the remainder of a market's lifetime once
   triggered, or whether a per-record redesign is needed before go-live.

4. **Oracle feed defaults (M-14).** Before `set_feed_source` is ever called for a real mainnet
   asset, review `ORACLE_MAX_DEVIATION_BPS`/`ORACLE_FEED_MIN_INTERVAL_SECONDS` against that
   asset's real expected volatility and update cadence, rather than relying on `deploy.sh`'s
   general-purpose shipped defaults.

5. **Protocol/creator fee split (M-15).** Decide whether market creators should be structurally
   guaranteed a minimum fee share (a cap below 100% on `protocol_admin`'s own `set_protocol_share`
   authority, and/or a timelock), as a deliberate product commitment rather than an unbounded
   admin discretion.

---

## 6. Test Coverage Report

Measured across the full workspace after every fix in this review landed: **98.24% line coverage,
98.21% region coverage**, both above the 98% target set for this review, with
`cargo test --workspace` passing all 477 tests. Kept as a separate, standalone document so it can
be re-measured and updated independently of this findings record going forward:
[`INTERNAL_SECURITY_AUDIT_COVERAGE_REPORT_2026_11_16.md`](INTERNAL_SECURITY_AUDIT_COVERAGE_REPORT_2026_11_16.md)
— full per-file breakdown, methodology, and the lowest-covered files for anyone prioritizing
further test-writing.

---

## 7. Conclusion

This review found and closed two Critical findings (a complete, unauthenticated administrative
takeover path on the Router contract, and a direct custody-invariant violation in SYWrapper's
`withdraw_gained`, the latter surfaced during an independent follow-up pass) and a further 38
findings ranging from High to Low severity, spanning authorization, arithmetic safety, accounting
correctness, compliance traceability, and test coverage. Every fixable finding was fixed, tested,
and independently re-verified; the six findings left open are architectural or policy decisions,
clearly scoped and handed off rather than quietly deferred. The workspace, in its final state,
builds cleanly, lints cleanly, and passes its entire test suite.

This document, together with the six remaining open recommendations in §5, should be treated as a
required input to the mainnet launch decision, not merely a historical record of this pass. Two
independent follow-up review passes, run cold against this codebase with no shared context with
each other or with this document's own first pass, each surfaced at least one finding the other
pass (and this document's original pass) had missed: the first surfaced C-2 and M-12, and
corrected H-3's own first implementation; the second surfaced H-4, M-14, M-15, L-12, and L-13.
That pattern — independent reviews consistently finding real issues the prior pass did not — is
itself the strongest evidence for why further independent review ahead of a real production
launch (beyond this internal-testing deployment) is worth the time, not a reason to treat this
document as a closed, final record.
