# Tranche 1 — Internal Audit and Fixes

A multi-agent senior-auditor review of the Tranche 1 branch (`feat/tranche-1-mvp`), run before this work was
proposed for merge. Five independent reviews ran in parallel — core accounting/solvency, AMM math and the
router, authorization/compliance/DoS, deliverables-and-documentation verification, and test-quality/mutation
testing — each read-only against the repository, several with numeric reproducers. This document records
every finding, its disposition, and the regression test that now guards it. It is the answer to "what did a
second pass find, and what changed because of it" — read [TRANCHE_1_DELIVERABLES.md](TRANCHE_1_DELIVERABLES.md)
first for what the deliverables are and how they map to code.

**Net result:** two High findings (both fixed), eleven Medium/Low findings (all fixed or explicitly accepted
below), and a set of test-quality and CI corrections. No finding broke PT + YT solvency beyond dust already
covered by existing floor-rounding; the closed-form YT index that the original Tranche 1 build introduced
(1/rate, replacing an earlier percentage-of-notional formula) held up under fuzzing and independent
reference-math checks.

---

## How to reproduce

```bash
cargo test --offline -p principal_integration_tests --test audit_regressions   # this document's regressions
cargo test --offline --workspace                                               # everything: 328 tests
cargo llvm-cov --workspace --ignore-filename-regex '(/test\.rs|_test\.rs|/tests/|integration_tests|mock_rwa)' --summary-only
```

---

## Findings and fixes

### High

**AMM-01 — `swap_yt_for_sy` (flash-redeem) stranded the seller's already-accrued YT yield, and a plain
`YTToken.transfer`/`seize` handed unsynced yield to the wrong party.**

The yield index (`G = INDEX_SCALE·SCALE/rate`) was advanced only by `update_yield_index`. `transfer`,
`transfer_from`, `mint`, `burn` and `seize` all *settled* against whatever index was currently stored, but
never *advanced* it first. So any balance change that happened before a keeper called `update_yield_index`
settled against a stale index: a seller's yield since the last update went to whoever received the tokens
(a buyer, or — in `swap_yt_for_sy` — the pool itself, which has no function that can ever claim it). Measured
by the AMM auditor: a holder selling 100 YT after a 30-day, 5% rate move who did **not** call
`update_yield_index` first lost 4.76% of the position's value versus claiming first (all of it, the entire
accrued yield) into `PendingClaim(pool)`, unrecoverable.

*Fix* — `contracts/yt_token/src/lib.rs`: every balance-changing entrypoint (`transfer`, `transfer_from`,
`mint`, `burn`, `seize`) now calls an internal `sync_index` **before** `settle`, which advances the index to
the oracle's current value (requiring freshness only once the market has matured and the index is still
unfrozen — see L-01 below). `update_yield_index` itself is now a thin wrapper around the same internal
`advance`. This makes every settlement — plain transfers, pool intake, seizure — exact regardless of whether
a keeper has run recently.

*Regression*: `flash_redeem_leaves_the_sellers_accrued_yield_with_the_seller`,
`a_plain_yt_transfer_does_not_hand_unsynced_yield_to_the_receiver` (`contracts/integration_tests/tests/audit_regressions.rs`), plus updated YT unit tests `a_plain_transfer_does_not_hand_unsynced_yield_to_the_receiver` and `seize_moves_balance_and_settles_both_sides` (`contracts/yt_token/src/test.rs`).

**AMM-02 — `Router.swap_sy_for_yt` (flash-mint) had no slippage guard on its pool leg.**

The only guard was `min_yt_out` on the YT minted, which is fixed by the oracle rate and the tokenization fee
and has nothing to do with the pool. The PT sale ran with `min_sy_out = 0`. Measured: sandwiching the PT sale
(sell PT first, let the victim's sale land at a worse price, rebuy) raised the victim's net YT cost by
6.7× (11.57 SY of attacker profit on a 100-SY trade) with zero protection.

*Fix* — `contracts/router/src/lib.rs`: `swap_sy_for_yt` takes a new `max_net_cost` parameter; the pool call's
`min_sy_out` is derived from it (`sy_in − max_net_cost`), so the price leg is bounded like every other Router
flow.

*Regression*: `flash_mint_reverts_when_the_pt_sells_for_less_than_the_callers_max_net_cost`.

### Medium

**A-01/A-02 — `initialize` had no authorization on `OracleAdapter`, `Permissioning`, `RiskControl`, `Router`
and `RecoveryEscrow`; the first caller becomes admin.** Confirmed with `mock_auths(&[])`: an attacker's
`initialize` call succeeds with no signature at all. A squatted `Permissioning`, in particular, is
un-recoverable (SY/PT/YT/PM/pool store its address immutably with no setter).

*Disposition — accepted, documented, not code-changed.* This class of contract has no natural "owner" to
authenticate against at construction (unlike `SYWrapper`/`PTToken`/`PrincipalManager`, whose `initialize`
already requires the underlying's live issuer authority — that check is unaffected and sound). The correct
fix is deploy-time atomicity (a `__constructor`, supported by Soroban SDK 26, or a factory that deploys and
initializes in one transaction), which is a deployment-tooling change for Tranche 2, not a contract-logic
change that this tranche's scope covers. **Documented as a deployment requirement**: DEPLOYMENT.md and
SECURITY.md now say "first caller (front-runnable) — deploy and initialize in the same submitted transaction"
instead of "deployer (once)", and TECHNICAL_SPECIFICATION.md §16.1 states the exception explicitly. A
consuming market's own `initialize` (SYWrapper, PT, YT, PrincipalManager, MarketConfig, MarketPool) still
requires the *issuer's* live signature regardless of which `Permissioning`/`Oracle`/`RiskControl` addresses it
is given, so a squatted infrastructure contract cannot be used to stand up a market over the issuer's
objection — it can only deny service to a market that is mistakenly pointed at it, which is a deployment
error, not a way to steal funds.

**A-03 — `RiskControl`'s circuit breaker only counts inflows; a deposit-then-withdraw loop consumes the
window's budget at low cost.** Confirmed: 10× (deposit 100, withdraw all) against a 1000-unit limit fully
consumed it while returning the attacker's capital, blocking a legitimate depositor.

*Disposition — accepted as a known limitation, documented.* The breaker's purpose (per the grant text) is
capping new exposure created per window, not net flow; crediting withdrawals back would let an attacker
launder a much larger flash-style deposit through the same window by depositing and withdrawing between
sub-steps, which is a worse property to have. The griefer must already be compliant (Permissioning is
default-deny), so this is an insider/KYC'd-account risk, not an open one. TECHNICAL_SPECIFICATION.md §16.6 and
SECURITY.md's threat table now say plainly that the window counts gross inflows and describe the mitigation
(monitoring plus the per-asset limit, which bounds the blast radius to one asset).

**A-04 — `MAX_BATCH = 10` was not achievable: a full-position (SY+PT+YT+LP) batch of 4 accounts already
exceeds Soroban's 100-ledger-entry-per-transaction footprint limit (measured: 103 entries), and 5 accounts
fails outright.**

*Fix* — `contracts/recovery_escrow/src/lib.rs`: `MAX_BATCH` lowered to **3** (measured to fit: ~82 entries for
3 full-position accounts) and `seize_all_positions` now checks `accounts.len() > MAX_BATCH` **before** reading
any balance, so an oversized request fails cheaply instead of after doing the work.

*Regression*: `batch_bound_is_three_and_is_checked_before_any_balance_is_read`,
`a_full_batch_at_the_bound_of_every_position_type_succeeds` (the latter actually seizes SY+PT+LP for 3
real accounts in one transaction against the live Soroban resource metering, not just the count).

**A-05/E-01 — Seizing a YT position left its already-accrued yield on the flagged (deauthorized) account,
where it could never be claimed, and `finalize_record` paid one record's YT leg from the *escrow's entire*
pending claim, misattributing yield across records that happened to share the escrow.**

*Fix* — `contracts/yt_token/src/lib.rs`: `seize` now moves the flagged account's `PendingClaim` to the escrow
in the same call (in addition to settling both sides, which it already did). `contracts/recovery_escrow/src/lib.rs`:
`RecoveryRecord` gained a `yt_yield_at_seize` field, snapshotted from `pending_claim(escrow)` immediately
before and after the `seize` call, so each record states exactly how much yield it contributed at seizure
time (yield accruing *after* seizure is still pooled across every YT the escrow holds and paid out by
whichever `finalize_record` runs first — documented, not solved, since attributing post-seizure pooled yield
per record would require a second per-record index, which is out of scope for this tranche).

*Regression*: `seizing_yt_moves_its_accrued_yield_to_the_escrow_and_records_it` (run over both a SAC and a
SEP-57 underlying).

**C-01 — YT index rounding (ceil at both the snapshot and the current value) could overpay a large position
by a few raw units, enough on a 10¹⁵-unit position to make the final redemption revert (claims exceeding
custody by 1187 stroops in the auditor's reproducer).**

*Fix* — `contracts/yt_token/src/lib.rs::settle`: the pending-yield formula now subtracts one extra index unit
before dividing (`(last - index - 1)`, guarded so it never goes negative), which makes the rounding
direction-safe — a difference of two round-ups can only *undershoot* the true value now, never overshoot it —
at a cost of at most ~2·10⁻¹² of the balance per settlement.

*Regression*: `a_very_large_position_can_always_be_fully_redeemed` (the exact 10¹⁵-unit, two-rate reproducer).
Two existing integration assertions that depended on exact equality were loosened to allow this same
sub-2-unit rounding: `recombine_returns_sy_at_the_current_rate_and_keeps_accrued_yield_claimable` and
`zero_fee_market_takes_nothing` (`contracts/integration_tests/tests/full_lifecycle.rs`).

**C-02 — A future-dated `OracleAdapter.set_reference_value` (e.g. milliseconds submitted instead of seconds)
permanently bricked the feed:** it reads as stale forever (`ledger_ts < stored_ts`), and every later
correction is rejected too, since timestamps must strictly increase.

*Fix* — `contracts/oracle_adapter/src/lib.rs`: `set_reference_value` now rejects `timestamp > ledger.timestamp()`
with a new `TimestampInFuture` error, at the door, before it can ever be stored.

*Regression*: `future_timestamp_rejected_so_the_feed_cannot_be_bricked` (oracle unit test),
`a_future_dated_oracle_update_is_rejected_and_the_market_keeps_working` (integration).

**L-01 — After maturity, `PrincipalManager.settle_all`/`redeem` demanded a fresh oracle even once the YT
index was already frozen (which itself requires a fresh observation to have happened), so a relay that goes
silent right after freezing still locked every redemption.**

*Fix* — `contracts/principal_manager/src/lib.rs::settle`: the freshness check and `update_yield_index` call
are now skipped once `YTToken.is_frozen()` is already true; `last_oracle_rate()` (the frozen value) is used
directly. A relay must still produce **one** fresh observation at or after maturity — the freeze can no
longer happen without it — but nothing further is required after that.

*Regression*: `settlement_needs_no_oracle_once_the_yt_index_is_frozen`.

**AMM-04 — `remove_liquidity` was blocked by the pool's admin pause with no exception, contradicting the
documented "LPs can always exit."**

*Fix* — `contracts/market_pool/src/lib.rs`: the pause now applies to `remove_liquidity` only while the market
is live (`now < maturity`); after maturity it is unconditionally available, matching every other
"redemption never depends on an admin switch" guarantee in the protocol.

*Regression*: `a_paused_pool_blocks_lp_exit_while_live_but_never_after_maturity`.

**Kind-detection / SAC fail-open — `principal_compliance::is_authorized`/`is_authority` called the SAC's
`authorized()`/`admin()` directly (non-`try_`); a trap (e.g. no trustline) would panic the calling contract
instead of resolving to "not authorized."**

*Fix* — `contracts/compliance/src/lib.rs`: both now use `try_authorized`/`try_admin` and treat any error as a
negative result — fail-closed instead of fail-panic. (`Kind::Rwa`'s path already used the fallible clients.)

*Regression*: covered indirectly by the existing compliance suite continuing to pass; no SAC in this test
environment can be made to trap on a granted account, so this is a defensive fix rather than one with a
positive-path reproducer.

### Low / Info — documented, no code change needed

- **A-06** (RWA "authority" = whoever passes the `set_address_frozen` probe, which may be a narrower role
  than a true admin) and **A-08** (an unverifiable RWA identity path reads as "not authorized" for everyone,
  which is fail-closed but conflates infrastructure failure with a real flag) are both inherent to SEP-57
  being a Draft standard with no dedicated role-check view; documented in
  [COMPLIANCE_ARCHITECTURE.md](COMPLIANCE_ARCHITECTURE.md) as a stated limitation of the adapter, to be
  revisited if SEP-57 adds one.
- **A-09/F-02** (`MarketConfig.initialize` takes `protocol_admin`/`treasury`/`protocol_share_bps` from the
  market creator, so nothing on-chain ties them to Principal) is a deployment-process control, not a
  contract-logic gap: Principal's own deployment script supplies these values, the same way it supplies every
  other constructor argument. Documented as an open item in TRANCHE_1_DELIVERABLES.md rather than coded
  around, since hard-wiring a protocol address into an open-source contract would only move the trust
  assumption, not remove it.
- **F-01** (a fee change applies immediately, including to yield a YT holder already accrued before the
  change) is accepted for this tranche and listed as a known limitation (no timelock) in both
  TECHNICAL_SPECIFICATION.md §15.3 and TRANCHE_1_DELIVERABLES.md; a timelock is scoped to a later tranche.
- **A-10/I-03** (no contract ever extended its own *instance* TTL, only per-user *persistent* entries) is
  fixed: every long-lived contract now exposes a permissionless `bump()` (see below), and the Router's market
  listings extend their own TTL on every use.
- **A-11** (SY's escrow pause-exemption lets any user push SY to the escrow during a pause; `PM.redeem` has
  no equivalent exemption, so a PM pause blocks `finalize_record`) — the first is self-harm only (no rescue
  path is needed since nothing can be lost, only donated); the second is accepted as intentional: an admin
  pause of `PrincipalManager` is a deliberate full stop and recovery is expected to use the ordinary
  `finalize_record` path once unpaused, not to route around a live incident response.
- **AMM-03** (nested calls with state-dependent amounts, e.g. the zap's computed split, must match a
  user-signed transaction's simulated values exactly, so intervening activity makes them fail closed rather
  than lose funds) and **AMM-06** (the Router registry trusts a registered pool's self-reported topology, and
  had no way to remove a bad listing) — the first is inherent to any multi-step simulate-then-submit flow and
  costs only a revert; the second is fixed with a new `unregister_market` (admin-only), documented above under
  keeper functions.
- **AMM-05** (the first LP deposit is permissionless and can set a skewed opening price) is accepted: any
  compliant address can already trade at any price the curve allows, and gating the *first* deposit
  specifically would need a new role with no clear owner; `min_lp_out` already protects every deposit after
  the first, and the market creator is expected to seed a real market itself.
- **AMM-07** (a 1-raw-unit sell reverts with the (correct but generically named) `InsufficientLiquidity`
  rather than a dedicated "dust" error) — accepted; the behavior is correct, only the error name is generic,
  and adding a new error code purely for a friendlier message was judged not worth another contract-error
  variant this late in the tranche.

---

## Documentation corrections made from this review

Beyond the code fixes above, the review's documentation pass found and corrected: the README's "four layers"
(the architecture has three), the minimum Rust version (soroban-sdk 26 needs 1.91, not 1.84), the claim that
`SYWrapper`'s exchange rate "grows as yield accrues" (it is fixed at 1.0 in this release; appreciation is
carried by the oracle — this was already stated correctly elsewhere in the docs but not everywhere), four
sequence diagrams in ARCHITECTURE.md that no longer matched the code (deposit/mint routing the circuit
breaker through the Router instead of SYWrapper/PrincipalManager calling it directly; the flash-mint diagram
minting PT/YT to the Router instead of the user; the redemption diagram omitting `settle_all`; the
recombination diagram's stale signature), a nonexistent `settlement_reserve` mechanism referenced in three
places, the false claim that arithmetic uses `checked_mul` throughout (it uses `overflow-checks = true`,
which is different and was already the accurate statement in most places), the overstated "every
state-changing function emits an event" and "every Router flow takes a deadline" (both now name their
exceptions), a stale `finalize_pt`/`finalize_yt` description in SECURITY.md (replaced by `finalize_record`
throughout), a `docs/SUMMARY.md` entry pointing outside the mdBook source directory, and status banners added
to the pre-Tranche-1 Testnet evidence documents and to ARCHITECTURE.md's backend/frontend design sections so
they are not mistaken for built, current-tranche scope. See the diffs on this branch for the complete list;
[TRANCHE_1_DELIVERABLES.md](TRANCHE_1_DELIVERABLES.md) is the up-to-date deliverable-by-deliverable record and
supersedes any conflicting statement anywhere else in `docs/`.

**Not independently verified in this pass:** an `mdbook build` of `docs/SUMMARY.md` (mdBook is not installed
in this environment); GitHub Actions actually executing `.github/workflows/ci.yml` end-to-end on
`github.com` (no PR has been opened against this branch yet). Both should be confirmed once this branch is
pushed and a pull request is opened.

## Test-count and coverage after this pass

328 tests (up from 314: +8 oracle, +2 YT, +14 new `audit_regressions.rs`), all passing;
`cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` both clean; all 11 contracts
build under `wasm32v1-none`, largest (`market_pool`) 66,420 bytes, well under the 128 KiB limit.
Production-code coverage after this pass: **97.67 % of lines** (3 810 / 3 901), 96.65 % of regions, every
file at or above 93.9 %.
