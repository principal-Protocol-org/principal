# Security Controls and Emergency Procedures

## 1. Threat model

| Threat | Impact | Mitigation |
|---|---|---|
| Malicious oracle price | Wrong settlement; PT/YT over/under-redeemed | `require_auth` on price setter; freshness window checked at mint, recombine, first settlement/redeem, every AMM swap, and `YTToken.update_yield_index`; multi-source feed not yet implemented |
| Unauthorized mint | Inflation of PT/YT supply | `require_auth()` on all entrypoints; permissioning check before mint; `PTToken`/`YTToken` mint restricted to a single registered minter, locked by `set_minter` |
| Deauthorized SAC holder retains a Principal position | An investor the issuer has deauthorized on the underlying asset still holds or moves SY/PT/YT and LP | `underlying_SAC.authorized(account)` — the mandatory floor, read live from the actual Stellar Asset Contract — is checked on both sides of every deposit, withdraw, mint, redeem, and PT/YT transfer, in addition to `Permissioning`; see §6 |
| Market stood up over an issuer's objection | A third party deploys a Principal market against a regulated asset without the issuer's involvement | `initialize` on `SYWrapper`/`PrincipalManager`/`PTToken`/`YTToken` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()` — reverts `IssuerMismatch` otherwise |
| Permissioning bypass | Ineligible user holds, moves, or redeems PT/YT | Checked at `SYWrapper` deposit/withdraw, `PrincipalManager` mint/redeem, and `PTToken`/`YTToken` transfer/transfer_from — on both the sending and receiving side, not only the recipient, so a revoked account is frozen rather than merely blocked from new positions |
| Front-running compliance action | Flagged account cashes out or dumps its position before the issuer can act | Both-sides authorization/eligibility checks (above) mean a deauthorized or revoked account cannot self-withdraw or transfer out; `RecoveryEscrow.seize_*` additionally requires the target to already be deauthorized on the underlying SAC, so it can't be used against an account that hasn't actually been flagged |
| Replay across maturities | Wrong redemption mapping | Each issuance has unique `maturity_timestamp`; maturity check on every redeem |
| Flash deposit attack | Circuit breaker drained | Ledger-sequence-window circuit breaker in RiskControl (protocol-wide and per-asset), called from inside `SYWrapper.deposit` and `PrincipalManager.mint` themselves — an over-limit call reverts atomically |
| Admin key compromise | Protocol takeover scoped to that contract | Single-call `transfer_admin`, requires the current admin's signature; recommend multisig for production. `seize()` on `SYWrapper`/`PTToken`/`YTToken` is restricted to the one configured `RecoveryEscrow` address — a compromised token-contract admin key alone cannot seize a balance, since `RecoveryEscrow` itself re-derives authority from the underlying SAC's real, live `admin()`, not from any key it stores |
| Rogue RecoveryEscrow substitution | An attacker points a market's `seize()` trust at an escrow they control | `set_recovery_escrow` is a one-time, admin-gated setter on each of `SYWrapper`/`PTToken`/`YTToken` — redirecting seizure authority requires that market's own real admin, and can only ever be done once |
| Reentrancy | State corruption | Checks-effects-interactions in `SYWrapper`: internal state is updated before the external `token::Client::transfer` call, on both `deposit` and `withdraw` |
| Integer overflow | Incorrect accounting | Soroban `i128` arithmetic; `overflow-checks = true` in release profile |
| YT yield accounting exceeds custody | Aggregate PT + YT claims exceed the SY actually held (once at any mint rate ≠ 1.0 — 100.23 SY of claims against 100 held at 1.05 → 1.10; earlier, additively across many oracle steps) | `YTToken`'s index is the closed form `G = 1e12·SCALE/rate` (rounded up): PT + YT telescope to exactly the shares deposited for any mint rate and any number of updates (`pt_plus_yt_claims_never_exceed_the_deposited_shares_at_any_mint_rate`, lifecycle solvency test) |
| Circuit-breaker griefing | Anyone calling `RiskControl.check_deposit` directly for an arbitrary amount, exhausting the day's budget and blocking real depositors, once this contract is wired into a real deposit path | `check_deposit` requires `caller` to be a registered consumer — found and fixed during the same audit, before any real deposit path called this contract |
| `RiskControl.check_deposit` accepts non-positive amounts | A registered consumer passing a negative amount could reduce the recorded circuit-breaker volume | `check_deposit` reverts `ZeroAmount` for `amount <= 0` — found and fixed during a follow-up audit |
| YT genesis baseline doesn't match a live market | `YTToken.initialize` hardcoding `LastOracleRate = SCALE` regardless of the real oracle value, combined with `PrincipalManager.mint` never advancing the index, could let a mint settle against a stale baseline and later receive credit for a rate movement that happened before that YT existed | `YTToken.initialize` now reads the genesis rate live from the oracle (reverting `OracleStale` if it isn't fresh) and `PrincipalManager.mint` calls `YTToken.update_yield_index()` before crediting the new balance, so a fresh mint's own snapshot is always the current factor — found and fixed during a follow-up audit |
| Mint against a stale oracle | `PrincipalManager.redeem` checked oracle freshness but `mint` didn't, so PT/YT could be minted against a stale rate | `PrincipalManager.mint` now calls `assert_oracle_fresh` before reading the rate, matching `redeem` — found and fixed during a follow-up audit |
| Direct `YTToken.claim_yield` footgun | `claim_yield` used to authorize on the holder (`from`), making it a public entrypoint that settled and zeroed a pending claim without ever transferring underlying — a holder calling it directly (instead of through `PrincipalManager`) would permanently forfeit that claim | `claim_yield` is now minter-gated, the same as `mint`/`burn`; `PrincipalManager.claim_yield` is the only path that settles through it, and pays the result out via `SYWrapper.withdraw` in the same call — found and fixed during a follow-up audit |
| Incoherent market topology | `PrincipalManager.initialize` accepted arbitrary `sy_wrapper`/`pt_token`/`yt_token` addresses with no cross-checks, so a deployment mistake could pair PT/YT from one market with SY custody, permissioning, or oracle assumptions from another | `initialize` now verifies all three share the configured `underlying`, `permissioning`, and (for PT/YT) `maturity`, and that `yt_token`'s oracle matches — reverts `TopologyMismatch` otherwise; found and fixed during a follow-up audit |
| Oracle value decreases silently accepted | `OracleAdapter.set_reference_value` enforced increasing timestamps but not non-decreasing values, while the entire PT/YT settlement model (`YTToken`'s never-negative yield, PT's `pt_amount * SCALE / final_rate` redemption formula) silently assumes the reference rate never falls | `set_reference_value` now reverts `ValueDecreased` if the new value is below the current one (equal values still allowed) — found and fixed during a follow-up audit |
| SEP-57 underlying has no SAC | `authorized()`/`admin()` do not exist, so inheritance and market-creation gating would fail | `principal_compliance` adapter: holders must be not-frozen and identity-verified; issuer authority is proven by an idempotent operator-role capability probe; runs the same recovery suite as a SAC (COMPLIANCE_ARCHITECTURE.md §3) |
| SEP-8 approval sandwich cannot wrap a Soroban call | A holder authorized only inside an approved classic transaction would be treated as authorized | Principal reads the flag as it stands when the contract runs; such holders are *unauthorized* and revert `NotAuthorizedOnSac` (fail closed); issuers must leave approved holders persistently authorized (COMPLIANCE_ARCHITECTURE.md §2) |
| AMM rounding leak | Fixed-point error scales with pool size; a fee-free round trip on a 1e17-unit pool profited by 6 units with a one-unit margin | `MarketPool` pads every solved reserve by `1 + u/1e14` units against the trader; `fee_free_round_trips_never_profit_at_any_pool_size` (1e8–1e17) |
| Pool donation / LP inflation | Tokens sent to the pool skew LP pricing (first-depositor attack) | LP is priced from internal reserves, never live balances; `MINIMUM_LIQUIDITY` locked by the first deposit; `a_donation_to_the_pool_cannot_skew_lp_pricing` |
| Recovery blocked by an operational pause | The unwrap step of `seize_sy` needs `SYWrapper.withdraw`, which the pause blocks | The configured escrow is exempt from `SYWrapper`'s pause when it is a party; pool `seize_lp`/`redeem_seized_lp` bypass the pause (`seizure_works_while_the_market_is_paused`) |
| Confiscatory fee configuration | A compromised issuer key sets punitive fees, or the creator zeroes Principal's share | Hard caps (tokenization ≤ 1 %, YT ≤ 50 %, swap tier ≤ 5 %); the protocol share belongs to `protocol_admin`, not the creator; fee claims are permissionless and pay fixed payees |
| Router as a phishing / confused-deputy surface | A user is steered into an arbitrary contract, or the Router is used to bypass compliance | Every flow needs a registered pool (admin, topology-checked); the Router acts as the user and holds nothing, so every downstream check runs against the real user; `router_acts_as_the_user_so_a_deauthorized_user_gains_nothing_by_using_it` |

## 2. Per-contract security properties

### OracleAdapter

- Only the stored admin may call `set_reference_value`. The caller must pass their address explicitly and call `require_auth()` — Soroban's auth model verifies the signature.
- Timestamps are monotonically increasing: a new price with a timestamp ≤ the stored timestamp is rejected with `TimestampTooOld`.
- Values are monotonically non-decreasing: a new value below the currently stored one is rejected with `ValueDecreased` (equal values are allowed — a same-price heartbeat refresh is not a decrease). This is enforced because the entire PT/YT settlement model already, silently, assumes it: `YTToken.update_yield_index` only ever accrues on a rate increase, and PT's redemption formula would release more underlying than was ever deposited if the rate could fall below its value at issuance.
- `is_fresh` uses `env.ledger().timestamp()` — the ledger clock — not a caller-supplied value, preventing time manipulation.
- Admin transfers emit an on-chain event and require the current admin to authorize.

### Permissioning

- All write operations (`grant_account`, `revoke_account`, `grant_asset`, `revoke_asset`, `grant_accounts`) require the caller to match the stored admin and call `require_auth()`.
- Eligibility entries use `persistent()` storage with a 30-day TTL. Entries that are not refreshed expire and default to `false` (deny), providing automatic revocation for inactive participants.
- Batch `grant_accounts` is guarded by the same admin check as single grants — no privilege escalation from batching.

### SYWrapper

- Follows checks-effects-interactions: all internal state (`total_underlying`, `total_shares`, `Balance`) is updated **before** the external `token::Client::transfer` call, on both `deposit` and `withdraw`. This prevents reentrancy from manipulating invariants.
- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()` — only the underlying asset's real issuer admin can stand up a market on it (reverts `IssuerMismatch` otherwise).
- `deposit` checks `underlying_SAC.authorized(from)` and `Permissioning.is_allowed(from)`; `withdraw` and `transfer` check **both** layers on **both** `from` and `to` — checking only the recipient would let a deauthorized or revoked account self-withdraw (or self-receive) before any compliance action reached it.
- `transfer` is a plain internal balance move between two compliant accounts — no change to `total_underlying`/`total_shares`, no external token call, so it carries none of `deposit`/`withdraw`'s reentrancy surface. It exists specifically so `PrincipalManager.mint` can take custody of a caller's shares.
- `seize()` — the compliance-recovery path — is restricted to the one address configured via `set_recovery_escrow` (a one-time, admin-gated setter). `SYWrapper` itself does not authenticate the issuer or check deauthorization; it only checks "is the caller my configured escrow," trusting `RecoveryEscrow` to have done that verification (see the `RecoveryEscrow` section below). It moves at most the target account's own balance, so other depositors' shares are never affected, and it is a forced transfer to the caller (not a burn), leaving the seized value recoverable rather than destroyed.
- The exchange rate is derived from `total_underlying / total_shares` — it cannot be directly written. An attacker cannot set an arbitrary rate.
- Pause flag blocks deposits, withdrawals and transfers for ordinary users; `seize()` still works while paused, and the configured `RecoveryEscrow` is exempt from the pause when it is a party to a `withdraw`/`transfer`, so a recovery's unwrap step is never blockable by the switch that halts ordinary activity.
- `deposit`/`withdraw` are slippage-protected (`min_shares_out`, `min_underlying_out`); a per-address cap on net deposits (`set_deposit_cap`) bounds any one address; once `set_risk_control` is called every `deposit` reports its amount to `RiskControl.check_deposit` inside the same transaction (a wired wrapper that is not a registered consumer reverts — fail closed).
- Compliance is routed through `principal_compliance` (SAC `authorized()` for classic/SEP-8 assets; frozen + identity for SEP-57).
- Zero-amount deposits and withdrawals are rejected.
- Withdrawal checks that `balance >= shares` before proceeding, preventing underflow.

### PrincipalManager

- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()`, the same market-creation gate as `SYWrapper`. It additionally verifies that `sy_wrapper`, `pt_token`, and `yt_token` all report the same `underlying_address()` and `permissioning_address()`, that `pt_token`/`yt_token`'s `maturity()` matches the `maturity` parameter, and that `yt_token.oracle_address()` matches `oracle` — reverting `TopologyMismatch` otherwise. Without this, a deployment mistake could pair PT/YT from one market with SY custody, permissioning, or oracle assumptions from another.
- `mint` is blocked after maturity (`assert_not_mature`). `redeem` is blocked before maturity (`assert_mature`). These checks use `env.ledger().timestamp()` — not caller-supplied values.
- Oracle freshness is verified at mint, recombine, claim and the first settlement (`assert_oracle_fresh`). `settle_all()` (permissionless, after maturity) freezes one settlement rate for PT and YT; after it, redemption needs no oracle, so a stale feed cannot trap holders.
- The tokenization fee is withheld in SY at mint and the YT fee from every yield payout; both accrue 20/80 (from `MarketConfig`) and are paid only to configuration-fixed payees by permissionless claims. `mint` reports the tokenized underlying value to `RiskControl` when wired. `recombine` returns SY at the current rate and leaves accrued yield claimable.
- `underlying_SAC.authorized(account)` and `Permissioning.is_allowed(account)` are both checked on `mint` and `redeem` — closing a gap where a deauthorized or revoked account could previously still redeem for the underlying asset after being flagged.
- `mint` calls `YTToken.update_yield_index()` before crediting the new YT balance, then `SYWrapper.transfer` to take real custody of the caller's SY shares, then `PTToken.mint`/`YTToken.mint` to credit real, holdable balances. Bringing the index current before the credit means a fresh mint's own yield snapshot always starts at the just-updated factor — it can never retroactively receive credit for a rate movement that happened before it existed. `redeem` calls `PTToken.burn`/`YTToken.burn` and releases real underlying via `SYWrapper.withdraw`, so the token-level protections below (§PTToken/YTToken, §SYWrapper) are reachable through the normal mint/redeem flow, not only by calling the token contracts directly.
- PT and YT balances live in `PTToken`/`YTToken`'s own storage, not duplicated in `PrincipalManager` — there is no shared or secondary counter that could drift out of sync or be manipulated by burning one token to inflate the other.
- YT redemption does not compute its own payout: it calls `YTToken.update_yield_index`/`burn`/`claim_yield` and forwards whatever that settles to. `YTToken.claim_yield` is minter-gated — only `PrincipalManager` can call it — so this is the only path that can ever settle a claim, closing the double-payment surface a separately-callable public entrypoint would otherwise create.
- `claim_yield(from)` lets a holder collect accrued yield without redeeming (burning) their YT position or waiting for maturity — it brings the index current, claims through `YTToken` as the registered minter, and pays the result out via `SYWrapper.withdraw` in the same call, so a settled claim can never go unpaid. A later `redeem` correctly pays nothing further for yield already claimed this way — see `redeem_yt_does_not_double_pay_yield_already_claimed_via_claim_yield`.
- PT redemption is `pt_amount * SCALE / settled_rate` (floor); YT yield accrues only when the rate rises above a position's entry rate, so PT principal is always protected. PT + YT claims sum to exactly the SY deposited for any mint rate.
- `PrincipalManager`'s own contract address must itself be SAC-authorized and Permissioning-granted before deployment is usable — it is now a genuine SY holder between mint and redemption, and self-authorizes as itself (a Soroban contract can `require_auth()` as its own address when it is the invoking contract) the same way `RecoveryEscrow` does when unwrapping a seizure.

### PTToken / YTToken

- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()`, the same market-creation gate as `SYWrapper`. `YTToken.initialize` additionally reads its genesis yield-index baseline (`LastOracleRate`) live from the oracle rather than hardcoding it to `SCALE`, and requires the oracle to be fresh at that moment (`OracleStale` otherwise) — a market created when the real rate is already above `SCALE` no longer baselines against a value the real rate never actually was.
- `transfer` and `transfer_from` check compliance on **both** `from` and `to` — checking only the recipient would let a deauthorized or revoked holder freely move its position to any still-eligible party before being frozen.
- Each side is checked against both layers: `underlying_SAC.authorized(account)` (the mandatory floor) and `Permissioning` — the coarse, account-level `is_allowed(account)` gate, and `is_allowed_for_asset(account, own_contract_address)`, a per-token gate that lets PT and YT carry independent eligibility policies for the same market.
- `mint` and `burn` are restricted to a single registered minter, set exactly once via `set_minter` (reverts `MinterAlreadySet` on a second call); both revert `MinterNotSet` if called before a minter is registered. `burn` itself has no compliance check — it only removes value and never redirects it to a new party, so there's nothing to gate. `YTToken.claim_yield` is gated the same way (`caller` must be the registered minter) — it used to authorize on the holder (`from`) instead, making it a public entrypoint that could settle and zero a pending claim with no underlying ever transferred; see `PrincipalManager`'s own `claim_yield`, which is now the only path that reaches it.
- `seize()` is restricted to the one address configured via `set_recovery_escrow` (one-time, admin-gated). Same trust model as `SYWrapper.seize()` above — the token contract only checks "is the caller my configured escrow," not the issuer's identity or the target's deauthorization status.
- `YTToken.update_yield_index()` is permissionless but requires the oracle to be fresh (`is_fresh(MAX_ORACLE_STALENESS_SECS)`, matching `PrincipalManager`'s own freshness discipline) and is a no-op if the rate hasn't increased since the last recorded high-water mark, so YT can never accrue negative yield.
- Every balance-changing operation (mint, burn, transfer in, transfer out, **and `seize`**) settles the affected account's pending yield **before** the balance changes, against the current index, then advances that account's snapshot. Without this, a buyer (or an escrow receiving a seized balance) could retroactively receive yield accrued before it held the position, or a seller (or a seized holder) could lose yield already earned.

### RecoveryEscrow

- Holds **no admin key of its own**. Every `seize_*`/`finalize_record` call re-derives authority through the compliance adapter (`SAC.admin()` live, or the SEP-57 operator probe) and requires that address's `require_auth()` — if the issuer rotates their admin key, the new key is authoritative immediately, with nothing stored in this contract to become stale or need updating.
- Also requires the target account to already fail the underlying's authorization check — reverts `TargetStillAuthorized` otherwise. Compliance recovery can only be used against an account the issuer has actually deauthorized, never merely because it holds a balance.
- Single point of verification: `SYWrapper`, `PTToken`, and `YTToken` do not each re-implement issuer-identity and deauthorization checks — they trust calls from their own configured `RecoveryEscrow` address, and all of the actual authentication logic lives once here, shared across all three.
- `seize_sy` seizes and immediately unwraps (via `SYWrapper.withdraw(from=self, to=self)`) in the same call, since SY has no maturity — the escrow ends the call holding raw underlying, ready for the issuer's native SAC clawback. `seize_pt`/`seize_yt` seize a real balance; `finalize_pt`/`finalize_yt` complete the unwind at or after maturity by calling `PrincipalManager.redeem(from=self, ...)`, which burns the escrow's own seized balance and pays the resulting underlying back to the escrow the same way `seize_sy` does immediately. `finalize_yt` used to additionally need `env.authorize_as_current_contract` before invoking `redeem`, since the old holder-gated `YTToken.claim_yield(from=self)` sat two call frames below `finalize_yt` (`RecoveryEscrow -> PrincipalManager -> YTToken`) and a contract's ordinary self-authorization only covers calls it makes directly. Now that `claim_yield` is minter-gated instead (authorized on `PrincipalManager`'s own address, the contract that actually calls it directly, one frame up), that workaround is no longer needed — `finalize_pt` never needed it either, since `PTToken.burn`/`YTToken.burn` were already minter-gated the same way.
- `initialize` verifies that `SYWrapper`, `PTToken`, and `YTToken` all report the same `underlying_address()` (reverts `PositionUnderlyingMismatch` otherwise), preventing an escrow from being accidentally or maliciously wired to contracts for different underlying assets.
- Covers SY, PT, YT **and LP** (`seize_lp` burns the LP in the pool: SY leg unwrapped at once, PT leg held). `seize_batch` (≤ 10 accounts) and `seize_all_positions` are all-or-nothing and authenticate the issuer once but check every target individually. Each event writes an immutable `RecoveryRecord`; `finalize_record` (once per record) settles the held PT/YT after maturity and writes the underlying onto that record. `finalize_pt`/`finalize_yt` no longer exist: unattributed finalization would break record traceability.

### MarketConfig

- `initialize` and `set_fees` require the underlying's **live** issuer authority (adapter check) — the creator, never a key stored here. Hard caps: tokenization ≤ 100 bps, YT ≤ 5 000 bps, swap tier ≤ 500 bps (effective swap fee ≤ 10 %).
- Principal's share (`protocol_share_bps`), the treasury and `protocol_admin` belong to `protocol_admin`; the creator cannot change them. `split` floors the protocol share so protocol + creator always equals the fee.
- The creator payout is `SAC.admin()` read at payout time (a rotation redirects it automatically) or, for a SEP-57 token, an explicit payee only the operator can set.
- **No timelock** on fee changes (events only). Recommended production setup: multisig issuer key.

### MarketPool

- `initialize` requires the issuer authority; wiring is read from the manager, so mismatched contracts are impossible by construction. The curve exponent must stay ≥ 0.25 (`MaturityTooFar`).
- State-changing trades need a fresh oracle, an unpaused pool and `now < maturity`; liquidity *removal* needs none, so a stale feed or a maturity can never trap LPs.
- Every trade, liquidity operation and LP transfer requires the caller (and any LP recipient) to be compliant on the underlying and `Permissioning`; the pool's own address needs standing on both layers plus per-asset PT/YT grants.
- Effects are written before token transfers (checks-effects-interactions). Reserves are internal state; balances always equal reserves plus accrued fees (asserted in tests).
- Fixed-point arithmetic is integer only (`WAD = 1e18`), verified against an `f64` reference, with a magnitude-scaled rounding pad against the trader; `k` never decreases across swaps.
- `swap_yt_for_sy` (flash-redeem) is one atomic call: it reverts `InsufficientLiquidity` if the recombined SY does not cover the PT's price, so it cannot take value from the pool.
- `seize_lp`/`redeem_seized_lp` are restricted to the one configured escrow (one-time setter) and work while paused and after maturity.
- Spot `pt_price()`/`implied_rate()` are manipulable within a block and must not be used as an external oracle; no TWAP is built.
- Swap fees accrue to protocol/creator buckets only — LPs receive none by design of the specified split.

### Router

- Holds no funds and no state about funds; needs no standing on the underlying. Always acts as the user (`from.require_auth()`), so every downstream compliance check is evaluated against the real user.
- Every flow requires a registered pool; registration is admin-only and cross-checks the pool against its manager and tokens (`TopologyMismatch`).
- `deadline` (`DeadlineExpired`) and `min_*_out` (`SlippageExceeded`) on every flow that has an output.

### principal_compliance (library)

- `init` detects `Kind::Sac` (answers `admin()`) or `Kind::Rwa` once, storing only the kind; every answer is read live.
- `Kind::Rwa` authorization fails closed: a frozen address, a missing identity verifier, or a reverting `verify_identity` all mean *not authorized*.
- `Kind::Rwa` authority uses an idempotent `set_address_frozen(caller, current, caller)` probe; it depends on the token enforcing RBAC on `operator`. SEP-57 is a Draft; all assumptions live in one crate.

### RiskControl

- Pausers can pause but **cannot** unpause. Unpause requires the admin. This prevents a compromised pauser from cycling the pause to allow specific transactions.
- The circuit-breaker window is counted in **ledger sequence numbers** (`DEFAULT_WINDOW_LEDGERS` = 17 280 ≈ 24 h; `set_window_ledgers`), not wall-clock time, and resets automatically. A protocol-wide limit and a per-asset limit are both checked before either is written, so a trip leaves no partial state. Limits/window changes require admin auth and emit an event.
- Setting `cb_limit = 0` disables the circuit breaker. This must only be done intentionally — document the reason in the admin governance log.
- `check_deposit` requires `caller` to be a registered consumer (`add_consumer`/`remove_consumer`, admin-gated, same one-time-per-call-site pattern as `add_pauser`). Without this, anyone could call `check_deposit` directly with an arbitrary amount to exhaust a day's circuit-breaker budget and block every legitimate depositor at zero cost beyond a transaction fee — found and fixed during a post-implementation audit, before this contract was ever wired into a real deposit path.

## 3. Oracle security

### Minimum requirements for production

- The reference value feed must be signed by the asset issuer (Ondo) or a multi-party oracle network.
- Enforce `max_stale_seconds ≤ 3600` (1 hour). The current constant in PrincipalManager is `3600`.
- Record `value`, `timestamp`, and `source_id` on-chain for post-mortem analysis.
- If the oracle fails or goes stale, `RiskControl.pause()` must be triggered before maturity settlement is allowed.

### Oracle failure response

1. Monitor `OracleAdapter` for staleness (timestamp delta > threshold).
2. If stale: registered pauser calls `RiskControl.pause()` immediately.
3. Admin investigates oracle feed; updates `OracleAdapter` once feed is restored.
4. Admin calls `RiskControl.unpause()` after confirming price validity.

## 4. Emergency controls

### Pause

```bash
# Any registered pauser can pause immediately
stellar contract invoke --id risk_control \
  -- pause --caller <pauser_address>
```

Effect: all `check_deposit` calls revert, so every `SYWrapper.deposit` and `PrincipalManager.mint` reverts (`Paused`). `SYWrapper`, `PrincipalManager` and `MarketPool` also have their own admin `set_paused`. Recovery keeps working while paused.

### Unpause

```bash
# Only admin can unpause
stellar contract invoke --id risk_control \
  -- unpause --caller <admin_address>
```

### Circuit breaker

The circuit breaker limits cumulative deposit volume within a window of ledger sequence numbers (default 17 280 ≈ 24 h), protocol-wide (`CircuitBreakerTripped`) and per asset (`AssetLimitTripped`). Volume is counted at each entry point, so a `wrap_and_mint` counts at both `SYWrapper.deposit` and `PrincipalManager.mint`. The window resets automatically; the admin adjusts `set_cb_limit`, `set_asset_limit` and `set_window_ledgers`.

### Admin transfer (all contracts)

`OracleAdapter`, `Permissioning`, `RiskControl`, `SYWrapper`, `PrincipalManager`, `MarketPool` and `Router` implement `transfer_admin(current_admin, new_admin)` (and `MarketConfig` `transfer_protocol_admin`) as a single call; `PTToken`/`YTToken` keep their setup-only admin, and `RecoveryEscrow` has no admin. The current admin authorizes and the new admin takes effect immediately, with no separate acceptance step — the current admin authorizes and the new admin takes effect immediately, with no separate acceptance step. Use this to rotate to a multisig or hardware key:

```bash
stellar contract invoke --id <contract_id> \
  -- transfer_admin \
     --current-admin <old_admin> \
     --new-admin <new_multisig_address>
```

Note that `RecoveryEscrow` needs no equivalent — it re-derives all authority from the underlying SAC's own admin key on every call, so rotating the issuer's SAC admin key rotates `RecoveryEscrow`'s effective authority automatically.

### Compliance recovery (seize via RecoveryEscrow)

The issuer's own SAC admin key must first deauthorize the target on the underlying asset (this is what `RecoveryEscrow` checks before allowing any seizure):

```bash
# 1. Issuer deauthorizes the target on the underlying SAC (outside Principal's contracts)
stellar contract invoke --id <underlying_sac> --source issuer_admin \
  -- set_authorized --id <flagged_account> --authorize false

# 2. Issuer calls RecoveryEscrow, which seizes and immediately unwraps SY toward the underlying
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- seize_sy \
     --caller <issuer_admin_address> \
     --account <flagged_account> \
     --shares <amount>

# PT/YT/LP: seizes the real balance into RecoveryEscrow (LP is burned for PT + SY; SY leg unwrapped)
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- seize_pt --caller <issuer_admin_address> --account <flagged_account> --amount <amount>

# Several accounts, every position type, in one transaction (all-or-nothing, <= 10 accounts):
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- seize_all_positions --caller <issuer_admin_address> --accounts '["<acct1>","<acct2>"]'

# 3. At or after maturity, settle what a record still holds (PT/YT) into the underlying:
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- finalize_record --caller <issuer_admin_address> --id <record_id>
```

`seize_*` and `finalize_record` revert `Unauthorized` unless `caller` is the underlying's live issuer authority (`SAC.admin()`, or the SEP-57 operator role); `seize_*` additionally revert `TargetStillAuthorized` unless the target is already deauthorized. Every seizure writes a `RecoveryRecord` (`account_records(account)`, `get_record(id)`); `finalize_record` writes the underlying recovered onto the same record and can run once per record. The issuer then finishes with the underlying's native mechanism (SAC `clawback` of the escrow's balance).

## 5. Access control matrix

*(Columns cover the original contracts; for `MarketConfig`, `MarketPool` and `Router` see the per-contract sections and [API_REFERENCE.md](API_REFERENCE.md), whose "Auth" column is authoritative. `finalize_pt`/`finalize_yt` below are now `finalize_record`.)*

| Action | OracleAdapter | Permissioning | SYWrapper | PrincipalManager | RiskControl | PTToken / YTToken | RecoveryEscrow |
|---|---|---|---|---|---|---|---|
| Initialize | deployer (once) | deployer (once) | underlying SAC's real admin (once) | underlying SAC's real admin (once); validates SY/PT/YT share one underlying, permissioning, maturity, and oracle | deployer (once) | underlying SAC's real admin (once) | anyone, once (validates position contracts share one underlying) |
| Set reference value | admin | — | — | — | — | — | — |
| Grant/revoke account | — | admin | — | — | — | — | — |
| Deposit | — | — | SAC-authorized + eligible account only | — | — | — | — |
| Withdraw | — | — | SAC-authorized + eligible sender and recipient | — | — | — | — |
| Set recovery escrow | — | — | admin (once) | — | — | admin (once) | — |
| Seize (compliance recovery) | — | — | configured RecoveryEscrow only | — | — | configured RecoveryEscrow only | underlying SAC's real, live admin; target must already be SAC-deauthorized |
| Finalize seized PT/YT | — | — | — | — | — | — | underlying SAC's real, live admin; acts only on the escrow's own balance, post-maturity |
| Mint PT/YT | — | — | — | permitted user | — | registered minter only, recipient must be SAC-authorized + eligible | — |
| Burn PT/YT | — | — | — | — | — | registered minter only, no compliance check | — |
| Transfer PT/YT | — | — | — | — | — | SAC-authorized + eligible sender and recipient (account + per-asset) | — |
| Redeem PT/YT | — | — | — | SAC-authorized + eligible PT/YT holder (post-maturity) | — | — | — |
| Claim yield (YT) | — | — | — | holder, via `PrincipalManager.claim_yield` (`require_auth`) | — | registered minter only | — |
| Pause | — | — | admin | admin | admin or pauser | — | — |
| Unpause | — | — | admin | admin | admin only | — | — |
| Transfer admin | admin | admin | admin | admin | admin | admin | n/a — no admin key |

## 6. Permissioning and compliance

Compliance is inherited directly from the underlying asset (SAC for classic/SEP-8 assets, frozen + identity for SEP-57 — see COMPLIANCE_ARCHITECTURE.md) — this is the mechanism the protocol depends on to function at all. An optional, admin-controlled narrowing layer (`Permissioning`) can additionally be configured on top of it, but the protocol works correctly with SAC inheritance alone:

- `underlying_SAC.authorized(account)` — the mandatory floor. Real, public, no-auth-required Stellar Asset Contract functions (`authorized`, `admin`) are read live from the actual issuer, so there is exactly one source of truth: if the issuer deauthorizes a wallet on the underlying asset itself, every Principal contract reflects that immediately, with no separate registry that could drift out of sync. This closes the compliance bypass that would otherwise exist if Principal only checked its own, separate registry — an underlying asset's own restrictions are never something the protocol needs to "mirror" and could get out of sync on.
- `Permissioning.is_allowed(account)` — an optional, narrower configuration surface on top of the SAC floor, administered by the same underlying-SAC administrator who controls market creation — not a separate Principal-managed registry, and never a replacement for the SAC floor. It narrows within what the SAC already permits; it cannot loosen it, since both checks must independently pass, and it adds no restriction of its own when the SAC imposes none. Checked on both sides of every transfer-like operation, not only the recipient, so a flagged account is frozen rather than merely blocked from acquiring new positions.
- `Permissioning.is_allowed_for_asset(account, asset)` provides finer-grained per-asset gating. `PTToken` and `YTToken` both check this against their own contract address, so an admin can grant an account access to PT without granting YT for the same market, or vice versa — something the SAC's own `authorized()` cannot express, since it has no concept of Principal's derivative instruments.
- Eligibility entries in `Permissioning`'s persistent storage expire after `ELIGIBILITY_TTL_LEDGERS` (≈ 30 days). Issuers must refresh entries for active participants before expiry. (`underlying_SAC.authorized()` has no such TTL — it reflects the issuer's current state directly.)
- Market creation itself requires the underlying SAC's real, live `admin()` to authorize `initialize` — no third party can stand up a Principal market on a regulated asset without that asset's actual issuer.
- `RecoveryEscrow.seize_*` gives issuers a way to recover a specific deauthorized account's value without a native Stellar Asset Contract clawback against the pooled reserve, which would otherwise haircut every other depositor along with the flagged account.

## 7. Governance and upgrade model

### v1 policy

- Core contract logic is immutable (no upgrade entrypoint in v1).
- Only parameters (oracle admin, permissioning entries, circuit breaker limit, fee rates) can be changed via existing admin entrypoints.
- All admin entrypoints require `require_auth()` and emit on-chain events.

### Recommended production setup

- Replace single admin keys with a 2-of-3 or 3-of-5 multisig before mainnet.
- Apply a 24–72 hour timelock to parameter changes that affect settlement math or fee rates.
- Maintain a separate guardian key with pauser role for emergency use.

## 8. Settlement accounting safety

- All arithmetic uses `i128` fixed-point with `SCALE = 10_000_000`.
- PT redemption uses integer division (floor). Residual rounding goes to `settlement_reserve` (to be implemented in production).
- YT yield is `max(0, (final_rate - SCALE) * yt_amount / SCALE)`. The floor at zero ensures PT holders are made whole before YT holders receive anything.
- Overflow is blocked at the Rust level (`overflow-checks = true` in the release profile).

## 9. Testing

314 tests (203 unit, 111 cross-contract integration) run on every pull request; production-code line coverage is 97.5 %. The security-relevant properties and the tests that prove them are listed per deliverable in [TRANCHE_1_DELIVERABLES.md](TRANCHE_1_DELIVERABLES.md); the highlights:

- [x] Arithmetic and boundary edge cases on every entry point: zero, negative, `i128::MIN`/`MAX`, exact thresholds (`edge_cases.rs`).
- [x] Authorization matrix: every admin-only function rejects a non-admin; escrow-only `seize` rejects even the admin; every state-changing call fails with **no signatures**; permissionless calls cannot redirect value.
- [x] Compliance inheritance on both a SAC and a SEP-57 underlying: deauthorized accounts blocked on SY, PT, YT, LP, on both sides; SEP-8 at-rest authorization semantics.
- [x] Recovery: SY, PT/YT with maturity finalization, LP, batch, `seize_all_positions`, records, native clawback, paused-market recovery, live authority rotation.
- [x] Solvency: PT + YT claims never exceed custody at any mint rate; a full multi-user lifecycle with fees leaves only rounding dust.
- [x] AMM: closed-form time decay, par at maturity, invariant monotonicity, no profitable round trip 1e8–1e17, donation resistance, flash-mint and flash-redeem, rate sweep 1.00–1.10, CPU budget on real WASM.
- [x] Circuit breaker wired into `SYWrapper.deposit` and `PrincipalManager.mint`; ledger-sequence window; per-asset limit; fail-closed on de-registration.
- [x] Oracle: monotonic values and timestamps; stale-feed behaviour at every consumer; freshness boundary (3 600 vs 3 601 s).
- [x] Admin rotation across all admin-bearing contracts; old keys lose every power.

Still outstanding before mainnet:

- [ ] Third-party security audit with full access to source and test suite (see TRANCHE_1_AUDIT.md for the internal review).
- [ ] Fee-change timelock; implied-rate TWAP oracle; multisig admin keys.
