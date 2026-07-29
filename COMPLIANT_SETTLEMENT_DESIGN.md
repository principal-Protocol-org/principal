# Compliant Settlement Design — PT/YT Tokens and Permissioning Extensions

Scope for this branch (`feature/phase2-compliant-pt-yt-settlement` — the branch name predates this file's rename):

1. `PTToken` — standalone SEP-41 Principal Token. **Implemented.**
2. `YTToken` — standalone SEP-41 Yield Token with claimable yield. **Implemented.**
3. `Permissioning` checks extended to `SYWrapper.deposit`/`withdraw` and `PrincipalManager.redeem`. **Implemented.**
4. **Authorization inheritance** — `SYWrapper`, `PrincipalManager`, `PTToken`, `YTToken` all check `underlying_SAC.authorized(account)` live, in addition to `Permissioning`. **Implemented.**
5. **Market-creation gating** — `initialize` on `SYWrapper`, `PrincipalManager`, `PTToken`, `YTToken` requires `admin` to equal the underlying SAC's actual `admin()`. **Implemented.**
6. **`RecoveryEscrow`** — replaces the original `SYWrapper.remediate()` with `seize_sy`/`seize_pt`/`seize_yt`, authenticated against the SAC's live `admin()` instead of a separate protocol admin key. **Implemented, including `finalize_pt`/`finalize_yt` (post-maturity unwind of a seized PT/YT position) — see §3.**
7. **Wiring `PrincipalManager` to actually call `SYWrapper`/`PTToken`/`YTToken`** — `mint` takes real SY custody and mints real PT/YT; `redeem` burns real PT/YT and releases real underlying. **Implemented — see §2.1.**
8. `LiquidationAdapter` — a distinct mechanism from `RecoveryEscrow` (see §4); still design-only. **Not implemented.**

Out of scope for this branch: `MarketPool`, `Router`. Each gets its own branch once this lands, per TECHNICAL_SPECIFICATION.md §6/§7 sequencing.

**Design provenance:** items 4–6 replace this document's original design after reviewing `Principal_compliance.pdf`, an external design note proposing that Principal inherit the underlying Stellar Asset Contract's own `authorized()`/`admin()` functions directly, rather than relying solely on a separate, protocol-managed `Permissioning` registry. Both functions were verified as real, public, no-auth-required SAC interface functions (confirmed against Stellar's own documentation) before any code was written against them. The result is strictly more capable than the version it replaces: it closes an operational gap where Principal's own `Permissioning` registry could drift out of sync with the issuer's actual authorization decisions, and it ties compliance-recovery authority to the real issuer's key instead of a separate Principal-controlled admin. `Permissioning` is kept as an *additional*, optional layer rather than removed — a pure SAC `authorized()` check can't express Principal-specific narrowing like asymmetric PT/YT eligibility (§1).

---

## 0. Pre-commit self-audit — three findings, all fixed

Before this branch was pushed, a second pass reviewed the diff specifically for the failure modes a Stellar/Soroban auditor would check first: authorization bypass and checks-effects-interactions. Three real issues, all fixed and covered by new regression tests:

1. **`SYWrapper.deposit` violated this repo's own documented security invariant.** `SECURITY.md`'s SYWrapper section explicitly claims "all internal state... is updated **before** the external `token::Client::transfer` call. This prevents reentrancy." The actual code did the opposite — external transfer first, state update after — leaving a window where a malicious `underlying` token contract could reenter `deposit` while `total_underlying`/`total_shares` still reflected pre-call values. Fixed: state now updates before the external call, matching `withdraw`'s existing (correct) ordering and the documented claim. Since Soroban transactions are atomic, reordering introduces no new failure mode — a failed transfer still reverts everything.
2. **`SYWrapper.withdraw` only checked the recipient's (`to`) eligibility, not the withdrawer's (`from`).** This meant a flagged account could self-withdraw the instant it suspected `remediate()` was coming, cashing out before compliance action landed — defeating the entire point of Clawback Propagation. Fixed: `withdraw` now checks both `from` and `to`. Regression test: `revoked_holder_cannot_front_run_remediation_by_self_withdrawing`.
3. **`PTToken`/`YTToken` transfer and `transfer_from` only checked the recipient (`to`), not the sender (`from`).** Same front-running problem one level up: a revoked holder could freely move PT/YT to any still-eligible party before being frozen, which also directly contradicted the resubmission doc's claim that "eligibility controls stay enforced as the position moves through the protocol." Fixed: both sides checked on every transfer path. Regression tests: `revoked_holder_cannot_dump_pt_before_remediation`, `revoked_holder_cannot_dump_yt_before_remediation`.

All three share a pattern: checking only the *receiving* side of a transfer is the default instinct (it's what stops ineligible parties from acquiring the instrument), but it silently permits an already-eligible-then-revoked holder to move value out before any compliance action reaches them. A revoked account needs to be frozen on both sides, not just blocked from new destinations.

## 0.1 Second, stricter pass — one severe finding, one medium

Requested explicitly as a follow-up: a second, stricter audit of the same diff, assuming the first pass had already caught the obvious issues. It hadn't caught everything.

1. **Severe — `SYWrapper.remediate()` never checked that its target account was actually revoked.** The function required the caller to be admin and required `shares` not to exceed the account's own balance, but nothing tied it to Permissioning's revocation state at all. As written, a legitimate admin key — or an attacker who compromised it — could call `remediate()` against *any* depositor, including one who had never been flagged, and walk away with their SY balance. The existing test (`remediate_burns_only_flagged_account_share`) didn't catch this because it never actually revoked the account it remediated; it happened to pass for the wrong reason. Fixed: added `assert_revoked`, which requires `Permissioning.is_allowed(account) == false` before `remediate` will act. This also has a structural benefit beyond closing the hole: revoking an account is Permissioning's admin action, remediating it is SYWrapper's — potentially different keys/roles — so a single compromised key can no longer both flag and drain an account unilaterally. Regression test: `remediate_requires_prior_revocation`. The pre-existing tests were updated to actually revoke their target before remediating, since that's the realistic sequence the fix now requires.

2. **Medium — `YTToken.update_yield_index()` consumed the oracle rate without checking freshness.** Every other oracle-consuming path in this codebase (`PrincipalManager.mint`/`redeem`) calls `is_fresh()` before trusting a rate; this one didn't, so a stalled oracle relay could leave a rate on record indefinitely and the index would keep being computed against it as if current. Fixed: added the same `MAX_ORACLE_STALENESS_SECS` (3600s) check used elsewhere, panicking with `OracleStale` otherwise. Regression test: `update_yield_index_blocked_by_stale_oracle`. (This also surfaced a test-fixture gap: the existing yield tests advanced the oracle's recorded timestamp without advancing the ledger clock, which — now that freshness is actually checked — would have made `is_fresh` see the oracle timestamp as being *ahead* of the ledger and correctly report stale. Fixed by advancing `env.ledger()` alongside the oracle timestamp in those tests, matching the pattern `PrincipalManager`'s tests already used.)

Both findings share a lesson: the first pass fixed *who can act on an account*, but not *whether the system-of-record actually agrees that account is in the state the action assumes*. A privileged caller acting on the wrong target, or acting on stale information, are both authorization gaps even when the caller itself is legitimate.

---

## 1. PTToken / YTToken — deviation from the original spec, and why

TECHNICAL_SPECIFICATION.md §6 originally specified `transfer` enforcing `Permissioning.is_allowed(to)` — the flat, account-level allow-list already used by `PrincipalManager.mint`.

This implementation uses `Permissioning.is_allowed_for_asset(to, asset_key)` instead, where `asset_key` is the token contract's own address, set at `initialize`. This is a deliberate change, not an oversight:

- It's what makes **asymmetric PT/YT permissioning** possible — PT and YT can carry different eligibility policies (e.g., PT distributed more broadly as a protected-principal note, YT restricted to a narrower speculative audience), which is one of the four originality claims in the resubmission doc.
- It costs nothing new: `Permissioning.is_allowed_for_asset` already exists and is already keyed by `(account, asset: Address)` — see `contracts/permissioning/src/lib.rs:92-117`. Passing the token's own address as the asset key is a wiring decision, not new infrastructure.
- An admin who wants PT and YT to behave identically (symmetric policy) simply grants the same account for both token addresses. Nothing forces divergence; it just becomes possible.

`PTToken` and `YTToken` both also fall back to `Permissioning.is_allowed(to)` (the coarse, protocol-wide gate already used by `mint`) as a first check — an account globally revoked can't hold either instrument regardless of per-asset grants. Per-asset eligibility narrows *within* that broader gate; it doesn't bypass it.

### 1.1 PTToken interface

```rust
fn initialize(env, admin: Address, permissioning: Address, underlying: Address, maturity: u64,
              name: String, symbol: String, decimals: u32);
// No minter parameter here at all — matches the two-phase init in TECHNICAL_SPECIFICATION.md
// §6.2 to break the PrincipalManager <-> PTToken circular dependency. set_minter() registers
// the minter separately, once PrincipalManager exists. `admin` must equal `underlying`'s real,
// live SAC admin() (§2's market-creation gate).

fn set_minter(env, admin: Address, minter: Address);   // one-time, reverts MinterAlreadySet if already set
fn set_recovery_escrow(env, admin: Address, escrow: Address);  // one-time, same pattern (§3)

// SEP-41
fn transfer(env, from: Address, to: Address, amount: i128);
fn transfer_from(env, spender: Address, from: Address, to: Address, amount: i128);
fn approve(env, from: Address, spender: Address, amount: i128, expiration_ledger: u32);
fn allowance(env, from: Address, spender: Address) -> i128;
fn balance(env, account: Address) -> i128;
fn decimals(env) -> u32;
fn name(env) -> String;
fn symbol(env) -> String;

// Minter-only
fn mint(env, to: Address, amount: i128);   // caller must equal registered minter
fn burn(env, from: Address, amount: i128); // caller must equal registered minter -- see §1.4
                                            // for why there is no holder-callable burn at all

// Compliance recovery (§3)
fn seize(env, caller: Address, account: Address, amount: i128) -> i128;

// Views
fn total_supply(env) -> i128;
fn maturity(env) -> u64;
fn minter(env) -> Address;
fn recovery_escrow(env) -> Address;
fn underlying_address(env) -> Address;
```

Storage: `Admin`, `Minter` (absent until `set_minter`), `Permissioning`, `Underlying`, `RecoveryEscrow` (absent until `set_recovery_escrow`), `Maturity`, `Name`, `Symbol`, `Decimals`, `TotalSupply` in `instance`; `Balance(Address)`, `Allowance(Address, Address)` in `persistent` with TTL extension on every touch (same `BALANCE_TTL_LEDGERS` constant used elsewhere in the codebase).

Errors: `AlreadyInitialized`, `Unauthorized`, `NotInitialized`, `ZeroAmount`, `InsufficientBalance`, `InsufficientAllowance`, `PermissionDenied`, `MinterAlreadySet`, `MinterNotSet`, `NotAuthorizedOnSac`, `IssuerMismatch`, `RecoveryEscrowAlreadySet`, `NotRecoveryEscrow`.

### 1.2 YTToken interface

Same as PTToken (including no minter parameter in `initialize`), plus the yield-index mechanism from TECHNICAL_SPECIFICATION.md §5.5:

```rust
fn initialize(env, admin, permissioning, underlying, oracle, maturity, name, symbol, decimals);
fn set_minter(env, admin, minter);
fn set_recovery_escrow(env, admin, escrow);

// compliance recovery (§3) -- settles both sides' pending yield before moving the balance
fn seize(env, caller: Address, account: Address, amount: i128) -> i128;

// yield accrual
fn update_yield_index(env);              // permissionless — requires fresh oracle, advances index
fn claim_yield(env, from: Address) -> i128;
fn accrued_yield_index(env) -> i128;
fn last_claimed_index(env, account: Address) -> i128;
fn pending_claim(env, account: Address) -> i128;   // settled-but-unclaimed amount
fn recovery_escrow(env) -> Address;
fn underlying_address(env) -> Address;
```

`claim_yield` settles the caller (see §1.3), then returns and zeroes their accumulated `PendingClaim`. `PrincipalManager.redeem` now calls this directly as the authoritative payer of YT yield at redemption (rather than computing an independent amount itself) specifically because `claim_yield` is also independently, publicly callable by any holder at any time — see `contracts/principal_manager/src/lib.rs`'s module doc comment for the double-payment reasoning, and `redeem_yt_does_not_double_pay_yield_already_claimed_directly` for the regression test.

Errors: same set as PTToken, plus `OracleStale` (`update_yield_index` called when the oracle isn't fresh).

### 1.3 Yield-accounting correctness (settle-before-mutate, and why the index is multiplicative)

Every balance-changing path (`mint`, `burn`, `transfer`, `transfer_from`, `seize`) calls an internal `settle(account)` for each affected account **before** its balance changes: it computes pending yield against the current global factor, adds that to the account's `PendingClaim`, and advances `LastClaimedIndex` to the current factor. Skipping this step entirely is a standard reward-accounting bug class — a buyer could otherwise receive yield accrued before they held the position (buying in right before a large factor update), or a seller could lose yield already earned (transferring out right after one). Both are covered by regression tests (`late_buyer_does_not_receive_prior_yield`, `transfer_settles_both_sides`).

**The factor itself is multiplicative, not additive — this was a real bug, found and fixed during a post-implementation audit.** `update_yield_index` advances a global factor `F` (starts at `SCALE`, only ever decreases) as `F = F * last_rate / now_rate`, and `settle` computes an account's pending yield as `balance * (F_last_settle - F_now) / F_last_settle`. This telescopes exactly to `balance * (rate_now - rate_at_settle) / rate_now` — the same formula `PrincipalManager` uses for PT redemption — regardless of how many times `update_yield_index` was called in between.

The first version of this contract accumulated yield *additively* instead: `index += (rate_now - rate_last) * SCALE / rate_now`, summed across every call, with `settle` computing `balance * (index_now - index_last_settle) / SCALE`. That sum is a Riemann approximation of `ln(rate_final/rate_genesis)`, which is provably `≥` the correct `(rate_final - rate_genesis)/rate_final` once there is more than one intermediate step, and the gap widens with every additional call. Because `PrincipalManager.redeem` treats this contract's `claim_yield` as the sole authoritative payer of YT yield (§2.1), that overstatement was a genuine solvency bug, not just an accounting nicety: aggregate PT + YT redemptions for a market could exceed the underlying value actually held by `PrincipalManager`, and since `update_yield_index` is permissionless with no rate limit, it was actively triggerable, not merely a passive drift under normal operation. Verified numerically: a 90-day market with daily updates and 30% total appreciation produced a 13.5% overstatement on the YT side (3.1% aggregate shortfall) under the additive formula; the multiplicative formula reduces the identical scenario to a 0.0004% floor-rounding residual. Regression test: `yield_is_path_independent_across_many_intermediate_updates`.

### 1.4 Why custom SEP-41 contracts, not Stellar Asset Contracts (SACs)

SY, PT, and YT (and, when built, LP) are each a plain Soroban contract implementing SEP-41, not a Stellar Asset Contract. This is deliberate: a SAC's native `burn` is callable by any holder against their own balance, with no way for the issuing contract to intercept or refuse it. If PT (or SY, or LP) were a SAC, any holder could burn their own position directly, bypassing `PrincipalManager`/`SYWrapper` entirely.

That matters here specifically because these tokens aren't just bearer balances — they're claims priced against pooled state (`SYWrapper`'s `total_underlying`/`total_shares` exchange rate, `PrincipalManager`'s PT/YT notional split, `YTToken`'s global yield index) and, once `MarketPool` exists, an AMM curve. An uncontrolled burn removes value from circulation without the corresponding accounting step (releasing the matching underlying, updating pool reserves, settling accrued yield) ever running, which desyncs total supply from the value actually backing it and can misprice every other holder's position. Pendle enforces the same restriction on its own PT/YT for the same reason: yield-bearing derivative positions can't be burned outside the protocol's own accounting path.

Every burn path in this codebase already reflects that: `PTToken.burn`/`YTToken.burn`/`SYWrapper` have no holder-facing burn at all — `burn` is minter-only (`PrincipalManager`, once wired) or reachable only via `withdraw` (`SYWrapper`, which burns shares and releases the matching underlying in the same call, never one without the other). There is no `from.require_auth()`-gated self-burn on any of these contracts.

---

## 2. Compliance checks — two mandatory layers, plus market-creation gating

`PrincipalManager.redeem()` and `SYWrapper.deposit`/`withdraw` originally had no `Permissioning` check at all (only `mint()` did). That gap is closed, and a second, independent layer was added on top:

- **`Permissioning`** (Principal-specific, optional narrowing) — `SYWrapper.initialize` takes a `permissioning: Address` parameter; `deposit`/`withdraw`/`mint`/`redeem`/token `transfer` all call `assert_permitted`.
- **SAC authorization inheritance** (mandatory floor, inherited live from the issuer) — every one of those same functions also calls `assert_sac_authorized`, which queries `underlying_SAC.authorized(account)` directly. Both `authorized(id: Address) -> bool` and `admin(env) -> Address` are real, public, no-auth-required functions on Soroban's built-in Stellar Asset Contract interface (`StellarAssetInterface`) — confirmed against Stellar's own documentation before implementation, not assumed. Unrestricted (non-`AUTH_REQUIRED`) assets return `authorized() == true` for every address by default, so this degrades gracefully for assets that don't use Stellar's authorization flags at all.
- **Market creation** — `initialize` on `SYWrapper`, `PrincipalManager`, `PTToken`, and `YTToken` now requires `admin` to equal `underlying_SAC.admin()` (read live) and requires `admin.require_auth()`. A third party cannot stand up a market — not even a new maturity on an already-integrated asset — without the actual issuer's signature. Read live, not cached: if the issuer rotates their SAC admin key, the new key is authoritative for any *future* market creation immediately, with nothing to update on already-deployed contracts (existing contracts keep whatever `admin` they were initialized with; only `initialize` itself checks the match).

Why both compliance layers, not just one: a pure `authorized()` check can express "can this address hold the underlying asset at all" but nothing more specific — it can't express PT-vs-YT asymmetric policy (§1), since the SAC has no concept of Principal's derivative instruments. `Permissioning` narrows within that floor; it can never loosen it, since both checks must pass.

---

## 3. Compliance recovery — `seize` and `RecoveryEscrow`

The native clawback function of a Stellar Asset only applies to the underlying asset's own balance. It cannot reach SY, PT, or YT positions directly, since those are separate Soroban positions. The original version of this design (a `SYWrapper.remediate()` function, admin-authorized by a separate Principal-controlled key) has been replaced by a dedicated `RecoveryEscrow` contract plus a `seize` function on each of `SYWrapper`, `PTToken`, and `YTToken`:

```rust
// On SYWrapper, PTToken, YTToken:
fn seize(env, caller: Address, account: Address, amount: i128) -> i128;
fn set_recovery_escrow(env, admin: Address, escrow: Address);  // one-time

// On RecoveryEscrow:
fn seize_sy(env, caller: Address, account: Address, shares: i128) -> i128;
fn seize_pt(env, caller: Address, account: Address, amount: i128) -> i128;
fn seize_yt(env, caller: Address, account: Address, amount: i128) -> i128;
fn finalize_pt(env, caller: Address, pt_amount: i128) -> i128;  // post-maturity unwind of a seized PT position
fn finalize_yt(env, caller: Address, yt_amount: i128) -> i128;  // same, for YT
```

**Split responsibility, deliberately.** `SYWrapper.seize`/`PTToken.seize`/`YTToken.seize` do almost nothing on their own: each checks that `caller` equals its own configured `RecoveryEscrow` address (set once via `set_recovery_escrow`, same admin-gated one-time pattern as `set_minter`), then moves the balance — a forced transfer, not a burn. None of them authenticate the issuer or check the target's compliance state themselves. All of that lives once, in `RecoveryEscrow`:

- `caller` must equal `underlying_SAC.admin()`, read live on every call — not a separate Principal-controlled admin key, and not cached from market-creation time.
- `account` must already be deauthorized on the SAC (`!underlying_SAC.authorized(account)`) — the same defense-in-depth principle as the original `assert_revoked` check (§0.1), re-anchored to the real source of truth instead of Principal's own `Permissioning`. This is what stops `seize` from being a generic drain: the issuer's admin key alone isn't sufficient, they also have to have actually deauthorized the target first.
- `RecoveryEscrow` has **no admin key of its own** — every check is a live read against the underlying SAC. If the issuer rotates their admin key, the new key is authoritative here immediately.

**SY unwinds immediately.** `seize_sy` seizes the balance, then in the same call unwraps it via a normal `SYWrapper.withdraw(from=escrow, to=escrow)` — the escrow is pre-authorized on both compliance layers at market setup, same as any other legitimate holder — leaving the escrow holding raw underlying, ready for the issuer's native SAC `clawback`. No separate finalize step, since SY has no maturity.

**PT/YT seizure and finalization both work now.** `seize_pt`/`seize_yt` move a flagged account's real `PTToken`/`YTToken` balance to the escrow. `finalize_pt`/`finalize_yt` complete the unwind at or after maturity: they call `PrincipalManager.redeem(from=escrow_address, ...)` on the escrow's own already-seized balance, which burns it and pays the resulting underlying back to the escrow via `SYWrapper.withdraw` — the same outcome `seize_sy` reaches immediately, just gated on maturity the way any PT/YT redemption is. `RecoveryEscrow.initialize` now takes a fifth `principal_manager: Address` parameter, and its consistency check verifies that contract also reports the same underlying as the other three.

`finalize_yt` has one subtlety `finalize_pt` doesn't: `PrincipalManager.redeem` calls `YTToken.claim_yield(from=escrow)` two call frames below `finalize_yt` (`RecoveryEscrow -> PrincipalManager -> YTToken`), and `claim_yield` requires that `from` address's own authorization. A contract's self-authorization only automatically covers calls it makes *directly*; reaching one frame further requires explicitly pre-declaring that sub-invocation via `env.authorize_as_current_contract` before calling `redeem`. `finalize_pt` doesn't need this because `PTToken.burn`/`YTToken.burn` are both minter-gated (`PrincipalManager`'s own direct self-authorization, one frame, handled automatically), not `from`-gated.

### 2.1 `PrincipalManager` now takes real custody -- `mint`/`redeem` call the real contracts

`mint` transfers the caller's SY shares into `PrincipalManager`'s own custody via `SYWrapper.transfer` (a new SEP-41-style function added specifically for this -- SYWrapper previously had no share-transfer capability at all, only `deposit`/`withdraw`), then mints real `PTToken`/`YTToken` balances. `redeem` burns those real balances and releases real underlying via `SYWrapper.withdraw`, self-authorizing as `PrincipalManager`'s own contract address the same way `RecoveryEscrow` does when unwrapping a seizure (§3). `PrincipalManager`'s own address is therefore now a genuine SY holder between mint and redemption, and must itself be SAC-authorized and Permissioning-granted for that reason (see DEPLOYMENT.md).

PT redemption keeps `PrincipalManager`'s own `pt_amount * SCALE / final_rate` formula (verified correct against the underlying economics; there is no independent PT-side payer to conflict with). YT redemption does not: it now calls `YTToken.update_yield_index()` then `burn()` then `claim_yield()` and treats the returned amount as authoritative, instead of computing a second, independent payout the way an earlier version of this contract did. The reason is `claim_yield` is a separate, publicly callable entrypoint (`from.require_auth()` only) backed by its own index -- if `redeem()` also computed and paid its own amount, a holder could receive yield twice for the same accrued period through the two different paths. See `contracts/principal_manager/src/lib.rs`'s module doc comment and the `redeem_yt_does_not_double_pay_yield_already_claimed_directly` regression test.

SY-share custody is converted to/from underlying amounts via `SYWrapper.exchange_rate()` -- a *different* rate from the Oracle's USDC-per-underlying price feed used for the PT/YT notional split. For a price-appreciating asset like USDY, where holding the token doesn't itself grow the wallet balance, `SYWrapper`'s exchange rate stays at 1.0 and this conversion is a no-op; for a balance-rebasing asset it would not be, and reconciling the two rate concepts for that case is out of scope until a second asset type is actually onboarded.

**This is not `LiquidationAdapter`.** `RecoveryEscrow` is for issuer-initiated compliance recovery — the SAC admin reclaiming value from a deauthorized holder. `LiquidationAdapter` (§4) is for third-party lending markets liquidating an under-collateralized borrower. Different caller, different trigger, different destination for the seized value. Both need PT to be movable without the holder's live authorization, but they solve unrelated problems.

---

## 4. LiquidationAdapter — design only, not implemented this pass

Interface sketch, per the feasibility audit in the resubmission doc:

```rust
fn initialize(env, admin: Address, principal_manager: Address, market_pool: Address, settlement_asset: Address);
fn liquidate(env, caller: Address, pt_amount: i128, min_settlement_out: i128) -> i128;
```

- `caller` is expected to be a lending pool contract (e.g., a Blend pool) that already custodies the borrower's PT balance under its own address, and self-authorizes the call to `LiquidationAdapter` via `authorize_as_current_contract` — the adapter never needs the original borrower's signature.
- `LiquidationAdapter`'s own address must be granted permanent `Permissioning` eligibility by the issuer/admin — a one-time approval, same category as approving `SYWrapper` itself.
- Internally: redeems (post-maturity) or recombines/swaps (pre-maturity, once `MarketPool` exists) the seized PT into the underlying, then pays the caller out in `settlement_asset` (plain USDC, not the permissioned RWA) at the oracle rate minus a liquidation bonus.

**Why this isn't implemented in this branch:** the pre-maturity path depends on `MarketPool`, which doesn't exist yet, and the exact calling convention depends on which Blend pool type this integrates with — a decision that belongs to whoever owns the Blend relationship, not something to guess at in contract code. Building against an assumed interface now risks a rewrite once that's settled. Tracked as a follow-up branch once `MarketPool`/`Router` land and the Blend integration point is confirmed.
