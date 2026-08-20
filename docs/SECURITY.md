# Security Controls and Emergency Procedures

## 1. Threat model

| Threat | Impact | Mitigation |
|---|---|---|
| Malicious oracle price | Wrong settlement; PT/YT over/under-redeemed | `require_auth` on price setter; freshness window checked at mint, redeem, and `YTToken.update_yield_index`; multi-source feed not yet implemented |
| Unauthorized mint | Inflation of PT/YT supply | `require_auth()` on all entrypoints; permissioning check before mint; `PTToken`/`YTToken` mint restricted to a single registered minter, locked by `set_minter` |
| Deauthorized SAC holder retains a Principal position | An investor the issuer has deauthorized on the underlying asset still holds or moves SY/PT/YT (and LP, once `MarketPool` ships) | `underlying_SAC.authorized(account)` — the mandatory floor, read live from the actual Stellar Asset Contract — is checked on both sides of every deposit, withdraw, mint, redeem, and PT/YT transfer, in addition to `Permissioning`; see §6 |
| Market stood up over an issuer's objection | A third party deploys a Principal market against a regulated asset without the issuer's involvement | `initialize` on `SYWrapper`/`PrincipalManager`/`PTToken`/`YTToken` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()` — reverts `IssuerMismatch` otherwise |
| Permissioning bypass | Ineligible user holds, moves, or redeems PT/YT | Checked at `SYWrapper` deposit/withdraw, `PrincipalManager` mint/redeem, and `PTToken`/`YTToken` transfer/transfer_from — on both the sending and receiving side, not only the recipient, so a revoked account is frozen rather than merely blocked from new positions |
| Front-running compliance action | Flagged account cashes out or dumps its position before the issuer can act | Both-sides authorization/eligibility checks (above) mean a deauthorized or revoked account cannot self-withdraw or transfer out; `RecoveryEscrow.seize_*` additionally requires the target to already be deauthorized on the underlying SAC, so it can't be used against an account that hasn't actually been flagged |
| Replay across maturities | Wrong redemption mapping | Each issuance has unique `maturity_timestamp`; maturity check on every redeem |
| Flash deposit attack | Circuit breaker drained | Rolling 24h window circuit breaker in RiskControl — logic implemented and tested, not yet cross-contract wired into `SYWrapper`/`PrincipalManager` |
| Admin key compromise | Protocol takeover scoped to that contract | Single-call `transfer_admin`, requires the current admin's signature; recommend multisig for production. `seize()` on `SYWrapper`/`PTToken`/`YTToken` is restricted to the one configured `RecoveryEscrow` address — a compromised token-contract admin key alone cannot seize a balance, since `RecoveryEscrow` itself re-derives authority from the underlying SAC's real, live `admin()`, not from any key it stores |
| Rogue RecoveryEscrow substitution | An attacker points a market's `seize()` trust at an escrow they control | `set_recovery_escrow` is a one-time, admin-gated setter on each of `SYWrapper`/`PTToken`/`YTToken` — redirecting seizure authority requires that market's own real admin, and can only ever be done once |
| Reentrancy | State corruption | Checks-effects-interactions in `SYWrapper`: internal state is updated before the external `token::Client::transfer` call, on both `deposit` and `withdraw` |
| Integer overflow | Incorrect accounting | Soroban `i128` arithmetic; `overflow-checks = true` in release profile |
| YT yield-index path-dependence | Aggregate PT + YT redemptions exceed the underlying actually held, once `PrincipalManager.redeem` began treating `YTToken.claim_yield` as authoritative | `YTToken`'s accrual factor is multiplicative (`F = F * last_rate / now_rate`, telescoping exactly to `SCALE * rate_genesis / rate_now`), not additive — found and fixed during a post-implementation audit; the earlier additive formula overstated yield, unboundedly, with every extra `update_yield_index` call. See COMPLIANT_SETTLEMENT_DESIGN.md §1.3 |
| Circuit-breaker griefing | Anyone calling `RiskControl.check_deposit` directly for an arbitrary amount, exhausting the day's budget and blocking real depositors, once this contract is wired into a real deposit path | `check_deposit` requires `caller` to be a registered consumer — found and fixed during the same audit, before any real deposit path called this contract |
| `RiskControl.check_deposit` accepts non-positive amounts | A registered consumer passing a negative amount could reduce the recorded circuit-breaker volume | `check_deposit` reverts `ZeroAmount` for `amount <= 0` — found and fixed during a follow-up audit |
| YT genesis baseline doesn't match a live market | `YTToken.initialize` hardcoding `LastOracleRate = SCALE` regardless of the real oracle value, combined with `PrincipalManager.mint` never advancing the index, could let a mint settle against a stale baseline and later receive credit for a rate movement that happened before that YT existed | `YTToken.initialize` now reads the genesis rate live from the oracle (reverting `OracleStale` if it isn't fresh) and `PrincipalManager.mint` calls `YTToken.update_yield_index()` before crediting the new balance, so a fresh mint's own snapshot is always the current factor — found and fixed during a follow-up audit |
| Mint against a stale oracle | `PrincipalManager.redeem` checked oracle freshness but `mint` didn't, so PT/YT could be minted against a stale rate | `PrincipalManager.mint` now calls `assert_oracle_fresh` before reading the rate, matching `redeem` — found and fixed during a follow-up audit |
| Direct `YTToken.claim_yield` footgun | `claim_yield` used to authorize on the holder (`from`), making it a public entrypoint that settled and zeroed a pending claim without ever transferring underlying — a holder calling it directly (instead of through `PrincipalManager`) would permanently forfeit that claim | `claim_yield` is now minter-gated, the same as `mint`/`burn`; `PrincipalManager.claim_yield` is the only path that settles through it, and pays the result out via `SYWrapper.withdraw` in the same call — found and fixed during a follow-up audit |
| Incoherent market topology | `PrincipalManager.initialize` accepted arbitrary `sy_wrapper`/`pt_token`/`yt_token` addresses with no cross-checks, so a deployment mistake could pair PT/YT from one market with SY custody, permissioning, or oracle assumptions from another | `initialize` now verifies all three share the configured `underlying`, `permissioning`, and (for PT/YT) `maturity`, and that `yt_token`'s oracle matches — reverts `TopologyMismatch` otherwise; found and fixed during a follow-up audit |
| Oracle value decreases silently accepted | `OracleAdapter.set_reference_value` enforced increasing timestamps but not non-decreasing values, while the entire PT/YT settlement model (`YTToken`'s never-negative yield, PT's `pt_amount * SCALE / final_rate` redemption formula) silently assumes the reference rate never falls | `set_reference_value` now reverts `ValueDecreased` if the new value is below the current one (equal values still allowed) — found and fixed during a follow-up audit |

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
- Pause flag blocks deposits and withdrawals; `seize()` intentionally still works while paused, since compliance recovery should not be blockable by the same switch that halts ordinary user activity.
- Zero-amount deposits and withdrawals are rejected.
- Withdrawal checks that `balance >= shares` before proceeding, preventing underflow.

### PrincipalManager

- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()`, the same market-creation gate as `SYWrapper`. It additionally verifies that `sy_wrapper`, `pt_token`, and `yt_token` all report the same `underlying_address()` and `permissioning_address()`, that `pt_token`/`yt_token`'s `maturity()` matches the `maturity` parameter, and that `yt_token.oracle_address()` matches `oracle` — reverting `TopologyMismatch` otherwise. Without this, a deployment mistake could pair PT/YT from one market with SY custody, permissioning, or oracle assumptions from another.
- `mint` is blocked after maturity (`assert_not_mature`). `redeem` is blocked before maturity (`assert_mature`). These checks use `env.ledger().timestamp()` — not caller-supplied values.
- Oracle freshness is verified at both mint and redemption time (`assert_oracle_fresh`). A stale oracle blocks minting and settlement until the feed is updated.
- `underlying_SAC.authorized(account)` and `Permissioning.is_allowed(account)` are both checked on `mint` and `redeem` — closing a gap where a deauthorized or revoked account could previously still redeem for the underlying asset after being flagged.
- `mint` calls `YTToken.update_yield_index()` before crediting the new YT balance, then `SYWrapper.transfer` to take real custody of the caller's SY shares, then `PTToken.mint`/`YTToken.mint` to credit real, holdable balances. Bringing the index current before the credit means a fresh mint's own yield snapshot always starts at the just-updated factor — it can never retroactively receive credit for a rate movement that happened before it existed. `redeem` calls `PTToken.burn`/`YTToken.burn` and releases real underlying via `SYWrapper.withdraw`, so the token-level protections below (§PTToken/YTToken, §SYWrapper) are reachable through the normal mint/redeem flow, not only by calling the token contracts directly.
- PT and YT balances live in `PTToken`/`YTToken`'s own storage, not duplicated in `PrincipalManager` — there is no shared or secondary counter that could drift out of sync or be manipulated by burning one token to inflate the other.
- YT redemption does not compute its own payout: it calls `YTToken.update_yield_index`/`burn`/`claim_yield` and forwards whatever that settles to. `YTToken.claim_yield` is minter-gated — only `PrincipalManager` can call it — so this is the only path that can ever settle a claim, closing the double-payment surface a separately-callable public entrypoint would otherwise create.
- `claim_yield(from)` lets a holder collect accrued yield without redeeming (burning) their YT position or waiting for maturity — it brings the index current, claims through `YTToken` as the registered minter, and pays the result out via `SYWrapper.withdraw` in the same call, so a settled claim can never go unpaid. A later `redeem` correctly pays nothing further for yield already claimed this way — see `redeem_yt_does_not_double_pay_yield_already_claimed_via_claim_yield`.
- PT redemption keeps its own `pt_amount * SCALE / final_rate` formula; there is no analogous independent PT-side payer to conflict with, and YT yield is floored at zero regardless: if `final_rate <= SCALE`, YT holders receive nothing but PT holders are unaffected.
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

- Holds **no admin key of its own**. Every `seize_*` call re-derives authority by reading `underlying_SAC.admin()` live and requiring that address's `require_auth()` — if the issuer rotates their admin key, the new key is authoritative immediately, with nothing stored in this contract to become stale or need updating.
- Also requires the target account to already fail `underlying_SAC.authorized(account)` — reverts `TargetStillAuthorized` otherwise. Compliance recovery can only be used against an account the issuer has actually deauthorized, never merely because it holds a balance.
- Single point of verification: `SYWrapper`, `PTToken`, and `YTToken` do not each re-implement issuer-identity and deauthorization checks — they trust calls from their own configured `RecoveryEscrow` address, and all of the actual authentication logic lives once here, shared across all three.
- `seize_sy` seizes and immediately unwraps (via `SYWrapper.withdraw(from=self, to=self)`) in the same call, since SY has no maturity — the escrow ends the call holding raw underlying, ready for the issuer's native SAC clawback. `seize_pt`/`seize_yt` seize a real balance; `finalize_pt`/`finalize_yt` complete the unwind at or after maturity by calling `PrincipalManager.redeem(from=self, ...)`, which burns the escrow's own seized balance and pays the resulting underlying back to the escrow the same way `seize_sy` does immediately. `finalize_yt` used to additionally need `env.authorize_as_current_contract` before invoking `redeem`, since the old holder-gated `YTToken.claim_yield(from=self)` sat two call frames below `finalize_yt` (`RecoveryEscrow -> PrincipalManager -> YTToken`) and a contract's ordinary self-authorization only covers calls it makes directly. Now that `claim_yield` is minter-gated instead (authorized on `PrincipalManager`'s own address, the contract that actually calls it directly, one frame up), that workaround is no longer needed — `finalize_pt` never needed it either, since `PTToken.burn`/`YTToken.burn` were already minter-gated the same way.
- `initialize` verifies that `SYWrapper`, `PTToken`, and `YTToken` all report the same `underlying_address()` (reverts `PositionUnderlyingMismatch` otherwise), preventing an escrow from being accidentally or maliciously wired to contracts for different underlying assets.
- Covers SY, PT, and YT today. Extending the same seize-now/finalize-at-settlement pattern to LP positions is planned once `MarketPool` is built — no `seize_lp`/`finalize_lp` exists yet, since `MarketPool` doesn't exist yet either.

### RiskControl

- Pausers can pause but **cannot** unpause. Unpause requires the admin. This prevents a compromised pauser from cycling the pause to allow specific transactions.
- The circuit breaker window resets automatically after `CB_WINDOW_SECS` (86400 s = 24 hours). The limit is set at initialization; changes require admin auth and emit an event.
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

Effect: all `check_deposit` calls revert. Once wired in, `SYWrapper` and `PrincipalManager` will each need to be registered as a consumer (`add_consumer`) and must call `check_deposit(caller=self, amount)` before processing operations.

### Unpause

```bash
# Only admin can unpause
stellar contract invoke --id risk_control \
  -- unpause --caller <admin_address>
```

### Circuit breaker

The circuit breaker limits cumulative deposit volume within a 24-hour rolling window. If the limit is exceeded, deposits revert with `CircuitBreakerTripped`. The window resets automatically; the admin can raise the limit with `set_cb_limit`.

### Admin transfer (all contracts)

Seven of the eight implemented contracts (all except `RecoveryEscrow`, which has no admin) implement `transfer_admin(current_admin, new_admin)` as a single call — the current admin authorizes and the new admin takes effect immediately, with no separate acceptance step. Use this to rotate to a multisig or hardware key:

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

# PT/YT: seizes the real balance into RecoveryEscrow
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- seize_pt --caller <issuer_admin_address> --account <flagged_account> --amount <amount>

# 3. Once at or after maturity, finalize the seized PT/YT position: redeems it through
#    PrincipalManager and pays the resulting underlying back to the escrow
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- finalize_pt --caller <issuer_admin_address> --pt-amount <amount>
```

`seize_sy`/`seize_pt`/`seize_yt`/`finalize_pt`/`finalize_yt` all revert `Unauthorized` unless `caller` is the underlying SAC's real, live `admin()`. `seize_*` additionally revert `TargetStillAuthorized` unless the target is already deauthorized on that SAC; `finalize_*` don't repeat that check since they only ever act on the escrow's own already-seized balance, not a third party's. There is no separate compliance-signer role to configure — authority is always exactly the issuer's own SAC admin key, whatever it currently is.

## 5. Access control matrix

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

Compliance is inherited directly from the underlying SAC — this is the mechanism the protocol depends on to function at all. An optional, admin-controlled narrowing layer (`Permissioning`) can additionally be configured on top of it, but the protocol works correctly with SAC inheritance alone:

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

## 9. Testing requirements

Done, at the unit-test level (175 tests across eight contracts) plus a dedicated cross-contract integration suite (15 tests, 190 total):

- [x] Unit tests for arithmetic edge cases (zero amounts, exact-limit deposits, insufficient balance/allowance).
- [x] Oracle failure scenarios: stale price blocks redemption (`PrincipalManager`) and blocks yield-index advancement (`YTToken`).
- [x] Authorization-inheritance violations: deauthorized-on-SAC accounts blocked from deposit, withdraw, mint, redeem, and PT/YT transfer on both sides; a deauthorized account cannot front-run seizure by self-withdrawing.
- [x] Permissioning violations: unauthorized mint reverts; revoked accounts blocked from redeem, deposit, withdraw, and PT/YT transfer on both sides.
- [x] Market-creation gating: `initialize` rejects an admin that doesn't match the underlying SAC's real, live `admin()`.
- [x] Compliance-recovery correctness: `seize()` on `SYWrapper`/`PTToken`/`YTToken` only affects the flagged account's own balance and requires the caller to be the one configured `RecoveryEscrow`; `RecoveryEscrow.seize_*` requires the caller to be the underlying SAC's real admin and the target to already be SAC-deauthorized; `seize_sy` unwraps to underlying in the same call; `YTToken.seize` settles both sides' pending yield before moving the balance.
- [x] Yield-accounting correctness: late buyers don't retroactively receive prior yield; transfers and seizures settle both sides before the balance moves; yield is path-independent across any number of intermediate `update_yield_index` calls (`yield_is_path_independent_across_many_intermediate_updates`).
- [x] Cross-contract integration: `PrincipalManager.mint`/`redeem` tested against real `SYWrapper`, `PTToken`, and `YTToken` deployments (not mocks) — real SY custody transfer, real PT/YT mint and burn, real underlying release, and a regression test confirming `PrincipalManager.claim_yield` and `redeem` don't pay the same accrued yield twice.
- [x] `RecoveryEscrow.finalize_pt`/`finalize_yt` — full seize-then-finalize flow tested against a real `PrincipalManager` deployment (not a mock), and a negative test for a non-issuer caller.
- [x] Circuit breaker trip and window-reset tests, including `RiskControl.check_deposit` rejecting zero and negative amounts.
- [x] Admin rotation tests on all seven admin-bearing contracts.
- [x] YT genesis-baseline correctness: a market created with the oracle already above `SCALE` doesn't overpay the first mint; `YTToken.initialize` reverts on a stale oracle; a second mint after a rate movement doesn't retroactively receive the first mint's prior yield (`late_minter_does_not_receive_prior_yield`).
- [x] Mint-time oracle freshness: a stale oracle blocks `PrincipalManager.mint`, mirroring the existing `redeem` check.
- [x] `YTToken.claim_yield` minter-gating: a non-minter caller is rejected; `PrincipalManager.claim_yield` pays real underlying and a subsequent `redeem` doesn't double-pay.
- [x] `PrincipalManager.initialize` topology validation: mismatched underlying, permissioning, maturity, and oracle each independently rejected.
- [x] `OracleAdapter.set_reference_value` rejects a value decrease; an equal-value resubmission is still allowed.
- [x] Full-stack view/getter and defensive-branch coverage: every admin-bearing contract's `transfer_admin`/`get_admin`, every token's `decimals`/`name`/`symbol`/`maturity`/`minter`/`recovery_escrow`/`underlying_address`/`permissioning_address`, and the zero-amount/insufficient-balance/expired-or-exceeded-allowance/double-initialize/non-admin-caller branches on every entrypoint that has one, are each exercised directly rather than only reachable incidentally through a happy-path flow.
- [x] Cross-contract integration suite (`contracts/integration_tests`, `cargo test -p principal_integration_tests`): deploys all eight contracts together and drives full multi-step flows no single contract's own unit tests exercise end-to-end — deposit/mint/approve+transfer_from/mid-life claim/maturity redemption across two users; seize-and-finalize compliance recovery for SY, PT, and YT through the real `PrincipalManager`; and admin rotation across every admin-bearing contract followed by a real mint to prove the market still functions afterward.
- [x] `RiskControl`'s pause/pauser/consumer/circuit-breaker lifecycle exercised from a registered consumer's perspective (`registered_consumer_check_deposit_trips_and_resets_circuit_breaker`), simulating the shape the future `SYWrapper`/`PrincipalManager` wiring will actually call — the cross-contract *wiring* itself remains outstanding (see below), only the receiving side is now covered.

Still outstanding before mainnet deployment:

- [ ] Cross-contract `RiskControl` wiring into `SYWrapper.deposit`/`PrincipalManager.mint` themselves (the consumer-side behavior it will call is now tested; the calls don't exist yet).
- [ ] `MarketPool` and `Router` test coverage once those contracts exist.
- [ ] Third-party security audit with full access to source and test suite.
