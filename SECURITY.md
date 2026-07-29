# Security Controls and Emergency Procedures

## 1. Threat model

| Threat | Impact | Mitigation |
|---|---|---|
| Malicious oracle price | Wrong settlement; PT/YT over/under-redeemed | `require_auth` on price setter; freshness window checked at mint, redeem, and `YTToken.update_yield_index`; multi-source feed not yet implemented |
| Unauthorized mint | Inflation of PT/YT supply | `require_auth()` on all entrypoints; permissioning check before mint; `PTToken`/`YTToken` mint restricted to a single registered minter, locked by `set_minter` |
| Deauthorized SAC holder retains a Principal position | An investor the issuer has deauthorized on the underlying asset still holds or moves SY/PT/YT | `underlying_SAC.authorized(account)` — the mandatory floor, read live from the actual Stellar Asset Contract — is checked on both sides of every deposit, withdraw, mint, redeem, and PT/YT transfer, in addition to `Permissioning`; see §6 |
| Market stood up over an issuer's objection | A third party deploys a Principal market against a regulated asset without the issuer's involvement | `initialize` on `SYWrapper`/`PrincipalManager`/`PTToken`/`YTToken` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()` — reverts `IssuerMismatch` otherwise |
| Permissioning bypass | Ineligible user holds, moves, or redeems PT/YT | Checked at `SYWrapper` deposit/withdraw, `PrincipalManager` mint/redeem, and `PTToken`/`YTToken` transfer/transfer_from — on both the sending and receiving side, not only the recipient, so a revoked account is frozen rather than merely blocked from new positions |
| Front-running compliance action | Flagged account cashes out or dumps its position before the issuer can act | Both-sides authorization/eligibility checks (above) mean a deauthorized or revoked account cannot self-withdraw or transfer out; `RecoveryEscrow.seize_*` additionally requires the target to already be deauthorized on the underlying SAC, so it can't be used against an account that hasn't actually been flagged |
| Replay across maturities | Wrong redemption mapping | Each issuance has unique `maturity_timestamp`; maturity check on every redeem |
| Flash deposit attack | Circuit breaker drained | Rolling 24h window circuit breaker in RiskControl — logic implemented and tested, not yet cross-contract wired into `SYWrapper`/`PrincipalManager` |
| Admin key compromise | Protocol takeover scoped to that contract | Single-call `transfer_admin`, requires the current admin's signature; recommend multisig for production. `seize()` on `SYWrapper`/`PTToken`/`YTToken` is restricted to the one configured `RecoveryEscrow` address — a compromised token-contract admin key alone cannot seize a balance, since `RecoveryEscrow` itself re-derives authority from the underlying SAC's real, live `admin()`, not from any key it stores |
| Rogue RecoveryEscrow substitution | An attacker points a market's `seize()` trust at an escrow they control | `set_recovery_escrow` is a one-time, admin-gated setter on each of `SYWrapper`/`PTToken`/`YTToken` — redirecting seizure authority requires that market's own real admin, and can only ever be done once |
| Reentrancy | State corruption | Checks-effects-interactions in `SYWrapper`: internal state is updated before the external `token::Client::transfer` call, on both `deposit` and `withdraw` |
| Integer overflow | Incorrect accounting | Soroban `i128` arithmetic; `overflow-checks = true` in release profile |

## 2. Per-contract security properties

### OracleAdapter

- Only the stored admin may call `set_reference_value`. The caller must pass their address explicitly and call `require_auth()` — Soroban's auth model verifies the signature.
- Timestamps are monotonically increasing: a new price with a timestamp ≤ the stored timestamp is rejected with `TimestampTooOld`.
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

- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()`, the same market-creation gate as `SYWrapper`.
- `mint` is blocked after maturity (`assert_not_mature`). `redeem` is blocked before maturity (`assert_mature`). These checks use `env.ledger().timestamp()` — not caller-supplied values.
- Oracle freshness is verified at redemption time. A stale oracle blocks settlement until the feed is updated.
- `underlying_SAC.authorized(account)` and `Permissioning.is_allowed(account)` are both checked on `mint` and `redeem` — closing a gap where a deauthorized or revoked account could previously still redeem for the underlying asset after being flagged.
- `mint` calls `SYWrapper.transfer` to take real custody of the caller's SY shares, then `PTToken.mint`/`YTToken.mint` to credit real, holdable balances. `redeem` calls `PTToken.burn`/`YTToken.burn` and releases real underlying via `SYWrapper.withdraw`, so the token-level protections below (§PTToken/YTToken, §SYWrapper) are reachable through the normal mint/redeem flow, not only by calling the token contracts directly.
- PT and YT balances live in `PTToken`/`YTToken`'s own storage, not duplicated in `PrincipalManager` — there is no shared or secondary counter that could drift out of sync or be manipulated by burning one token to inflate the other.
- YT redemption does not compute its own payout: it calls `YTToken.update_yield_index`/`burn`/`claim_yield` and forwards whatever that settles to. `YTToken.claim_yield` is separately, publicly callable by any holder at any time, so having `PrincipalManager` also compute and pay an independent amount would create a double-payment path for the same accrued yield — see `redeem_yt_does_not_double_pay_yield_already_claimed_directly`.
- PT redemption keeps its own `pt_amount * SCALE / final_rate` formula; there is no analogous independent PT-side payer to conflict with, and YT yield is floored at zero regardless: if `final_rate <= SCALE`, YT holders receive nothing but PT holders are unaffected.
- `PrincipalManager`'s own contract address must itself be SAC-authorized and Permissioning-granted before deployment is usable — it is now a genuine SY holder between mint and redemption, and self-authorizes as itself (a Soroban contract can `require_auth()` as its own address when it is the invoking contract) the same way `RecoveryEscrow` does when unwrapping a seizure.

### PTToken / YTToken

- `initialize` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()`, the same market-creation gate as `SYWrapper`.
- `transfer` and `transfer_from` check compliance on **both** `from` and `to` — checking only the recipient would let a deauthorized or revoked holder freely move its position to any still-eligible party before being frozen.
- Each side is checked against both layers: `underlying_SAC.authorized(account)` (the mandatory floor) and `Permissioning` — the coarse, account-level `is_allowed(account)` gate, and `is_allowed_for_asset(account, own_contract_address)`, a per-token gate that lets PT and YT carry independent eligibility policies for the same market.
- `mint` and `burn` are restricted to a single registered minter, set exactly once via `set_minter` (reverts `MinterAlreadySet` on a second call); both revert `MinterNotSet` if called before a minter is registered. `burn` itself has no compliance check — it only removes value and never redirects it to a new party, so there's nothing to gate.
- `seize()` is restricted to the one address configured via `set_recovery_escrow` (one-time, admin-gated). Same trust model as `SYWrapper.seize()` above — the token contract only checks "is the caller my configured escrow," not the issuer's identity or the target's deauthorization status.
- `YTToken.update_yield_index()` is permissionless but requires the oracle to be fresh (`is_fresh(MAX_ORACLE_STALENESS_SECS)`, matching `PrincipalManager`'s own freshness discipline) and is a no-op if the rate hasn't increased since the last recorded high-water mark, so YT can never accrue negative yield.
- Every balance-changing operation (mint, burn, transfer in, transfer out, **and `seize`**) settles the affected account's pending yield **before** the balance changes, against the current index, then advances that account's snapshot. Without this, a buyer (or an escrow receiving a seized balance) could retroactively receive yield accrued before it held the position, or a seller (or a seized holder) could lose yield already earned.

### RecoveryEscrow

- Holds **no admin key of its own**. Every `seize_*` call re-derives authority by reading `underlying_SAC.admin()` live and requiring that address's `require_auth()` — if the issuer rotates their admin key, the new key is authoritative immediately, with nothing stored in this contract to become stale or need updating.
- Also requires the target account to already fail `underlying_SAC.authorized(account)` — reverts `TargetStillAuthorized` otherwise. Compliance recovery can only be used against an account the issuer has actually deauthorized, never merely because it holds a balance.
- Single point of verification: `SYWrapper`, `PTToken`, and `YTToken` do not each re-implement issuer-identity and deauthorization checks — they trust calls from their own configured `RecoveryEscrow` address, and all of the actual authentication logic lives once here, shared across all three.
- `seize_sy` seizes and immediately unwraps (via `SYWrapper.withdraw(from=self, to=self)`) in the same call, since SY has no maturity — the escrow ends the call holding raw underlying, ready for the issuer's native SAC clawback. `seize_pt` and `seize_yt` seize a real balance but do not yet finalize it (redeem at maturity, unwrap toward underlying) — that requires `PrincipalManager` to call the real `PTToken`/`YTToken` contracts, which it doesn't do for any caller yet (see PROOF_OF_CONCEPT.md's Known Limitations).
- `initialize` verifies that `SYWrapper`, `PTToken`, and `YTToken` all report the same `underlying_address()` (reverts `PositionUnderlyingMismatch` otherwise), preventing an escrow from being accidentally or maliciously wired to contracts for different underlying assets.

### RiskControl

- Pausers can pause but **cannot** unpause. Unpause requires the admin. This prevents a compromised pauser from cycling the pause to allow specific transactions.
- The circuit breaker window resets automatically after `CB_WINDOW_SECS` (86400 s = 24 hours). The limit is set at initialization; changes require admin auth and emit an event.
- Setting `cb_limit = 0` disables the circuit breaker. This must only be done intentionally — document the reason in the admin governance log.

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

Effect: all `check_deposit` calls revert. SYWrapper and PrincipalManager must call `check_deposit` before processing operations.

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

# PT/YT: seizes the real balance into RecoveryEscrow; does not yet finalize (see PROOF_OF_CONCEPT.md)
stellar contract invoke --id recovery_escrow --source issuer_admin \
  -- seize_pt --caller <issuer_admin_address> --account <flagged_account> --amount <amount>
```

`seize_sy`/`seize_pt`/`seize_yt` all revert `Unauthorized` unless `caller` is the underlying SAC's real, live `admin()`, and `TargetStillAuthorized` unless the target is already deauthorized on that SAC. There is no separate compliance-signer role to configure — authority is always exactly the issuer's own SAC admin key, whatever it currently is.

## 5. Access control matrix

| Action | OracleAdapter | Permissioning | SYWrapper | PrincipalManager | RiskControl | PTToken / YTToken | RecoveryEscrow |
|---|---|---|---|---|---|---|---|
| Initialize | deployer (once) | deployer (once) | underlying SAC's real admin (once) | underlying SAC's real admin (once) | deployer (once) | underlying SAC's real admin (once) | anyone, once (validates position contracts share one underlying) |
| Set reference value | admin | — | — | — | — | — | — |
| Grant/revoke account | — | admin | — | — | — | — | — |
| Deposit | — | — | SAC-authorized + eligible account only | — | — | — | — |
| Withdraw | — | — | SAC-authorized + eligible sender and recipient | — | — | — | — |
| Set recovery escrow | — | — | admin (once) | — | — | admin (once) | — |
| Seize (compliance recovery) | — | — | configured RecoveryEscrow only | — | — | configured RecoveryEscrow only | underlying SAC's real, live admin; target must already be SAC-deauthorized |
| Mint PT/YT | — | — | — | permitted user | — | registered minter only, recipient must be SAC-authorized + eligible | — |
| Burn PT/YT | — | — | — | — | — | registered minter only, no compliance check | — |
| Transfer PT/YT | — | — | — | — | — | SAC-authorized + eligible sender and recipient (account + per-asset) | — |
| Redeem PT/YT | — | — | — | SAC-authorized + eligible PT/YT holder (post-maturity) | — | — | — |
| Claim yield (YT) | — | — | — | — | — | holder only (`require_auth`) | — |
| Pause | — | — | admin | admin | admin or pauser | — | — |
| Unpause | — | — | admin | admin | admin only | — | — |
| Transfer admin | admin | admin | admin | admin | admin | admin | n/a — no admin key |

## 6. Permissioning and compliance

Compliance is enforced through **two layers**, checked independently on every affected account:

- `underlying_SAC.authorized(account)` — the mandatory floor. Real, public, no-auth-required Stellar Asset Contract functions (`authorized`, `admin`) are read live from the actual issuer, so there is exactly one source of truth: if the issuer deauthorizes a wallet on the underlying asset itself, every Principal contract reflects that immediately, with no separate registry that could drift out of sync. This closes the compliance bypass that would otherwise exist if Principal only checked its own, separate registry — an underlying asset's own restrictions are never something the protocol needs to "mirror" and could get out of sync on.
- `Permissioning.is_allowed(account)` — an optional, Principal-specific additional layer, on top of the SAC floor, never a replacement for it. It narrows within what the SAC already permits; it cannot loosen it, since both checks must independently pass. Checked on both sides of every transfer-like operation, not only the recipient, so a flagged account is frozen rather than merely blocked from acquiring new positions.
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

Done, at the unit-test level (101 tests across eight contracts):

- [x] Unit tests for arithmetic edge cases (zero amounts, exact-limit deposits, insufficient balance/allowance).
- [x] Oracle failure scenarios: stale price blocks redemption (`PrincipalManager`) and blocks yield-index advancement (`YTToken`).
- [x] Authorization-inheritance violations: deauthorized-on-SAC accounts blocked from deposit, withdraw, mint, redeem, and PT/YT transfer on both sides; a deauthorized account cannot front-run seizure by self-withdrawing.
- [x] Permissioning violations: unauthorized mint reverts; revoked accounts blocked from redeem, deposit, withdraw, and PT/YT transfer on both sides.
- [x] Market-creation gating: `initialize` rejects an admin that doesn't match the underlying SAC's real, live `admin()`.
- [x] Compliance-recovery correctness: `seize()` on `SYWrapper`/`PTToken`/`YTToken` only affects the flagged account's own balance and requires the caller to be the one configured `RecoveryEscrow`; `RecoveryEscrow.seize_*` requires the caller to be the underlying SAC's real admin and the target to already be SAC-deauthorized; `seize_sy` unwraps to underlying in the same call; `YTToken.seize` settles both sides' pending yield before moving the balance.
- [x] Yield-accounting correctness: late buyers don't retroactively receive prior yield; transfers and seizures settle both sides before the balance moves.
- [x] Cross-contract integration: `PrincipalManager.mint`/`redeem` tested against real `SYWrapper`, `PTToken`, and `YTToken` deployments (not mocks) — real SY custody transfer, real PT/YT mint and burn, real underlying release, and a regression test confirming YT yield isn't paid twice through `YTToken.claim_yield`'s independent entrypoint and `PrincipalManager.redeem`.
- [x] Circuit breaker trip and window-reset tests.
- [x] Admin rotation tests on all seven admin-bearing contracts.

Still outstanding before mainnet deployment:

- [ ] `RecoveryEscrow.finalize_pt`/`finalize_yt` and their test coverage — `RecoveryEscrow` doesn't yet know `PrincipalManager`'s address, so it can't call `redeem` on a seized position.
- [ ] Cross-contract `RiskControl` wiring and associated integration tests.
- [ ] `MarketPool` and `Router` test coverage once those contracts exist.
- [ ] Third-party security audit with full access to source and test suite.
