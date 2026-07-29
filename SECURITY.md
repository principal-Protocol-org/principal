# Security Controls and Emergency Procedures

## 1. Threat model

| Threat | Impact | Mitigation |
|---|---|---|
| Malicious oracle price | Wrong settlement; PT/YT over/under-redeemed | `require_auth` on price setter; freshness window checked at mint, redeem, and `YTToken.update_yield_index`; multi-source feed not yet implemented |
| Unauthorized mint | Inflation of PT/YT supply | `require_auth()` on all entrypoints; permissioning check before mint; `PTToken`/`YTToken` mint restricted to a single registered minter, locked by `set_minter` |
| Permissioning bypass | Ineligible user holds, moves, or redeems PT/YT | Checked at `SYWrapper` deposit/withdraw, `PrincipalManager` mint/redeem, and `PTToken`/`YTToken` transfer/transfer_from — on both the sending and receiving side, not only the recipient, so a revoked account is frozen rather than merely blocked from new positions |
| Front-running compliance action | Flagged account cashes out or dumps its position before an admin can act | Both-sides eligibility checks (above) mean a revoked account cannot self-withdraw or transfer out; `SYWrapper.remediate()` additionally requires the target to already be revoked, so it can't be used against an account that hasn't been flagged |
| Replay across maturities | Wrong redemption mapping | Each issuance has unique `maturity_timestamp`; maturity check on every redeem |
| Flash deposit attack | Circuit breaker drained | Rolling 24h window circuit breaker in RiskControl — logic implemented and tested, not yet cross-contract wired into `SYWrapper`/`PrincipalManager` |
| Admin key compromise | Protocol takeover scoped to that contract | Single-call `transfer_admin`, requires the current admin's signature; recommend multisig for production. `SYWrapper.remediate()` requires the target account to already be revoked by `Permissioning`'s admin — a compromised `SYWrapper` admin key alone cannot drain an arbitrary account without `Permissioning`'s admin having also flagged it, so a single compromised key isn't sufficient for that specific action |
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
- `deposit` checks `Permissioning.is_allowed(from)`; `withdraw` checks **both** `from` and `to` — checking only the recipient would let a revoked account self-withdraw before any compliance action reached it.
- `remediate()` — the compliance-recovery path — requires the caller to be admin **and** requires the target account to already be revoked (`is_allowed(account) == false`). It cannot act on an account Permissioning still considers eligible, regardless of who calls it, and it can burn at most that account's own balance, so other depositors' shares are never affected.
- The exchange rate is derived from `total_underlying / total_shares` — it cannot be directly written. An attacker cannot set an arbitrary rate.
- Pause flag blocks both deposits and withdrawals. Only admin can unpause.
- Zero-amount deposits and withdrawals are rejected.
- Withdrawal checks that `balance >= shares` before proceeding, preventing underflow.

### PrincipalManager

- `mint` is blocked after maturity (`assert_not_mature`). `redeem` is blocked before maturity (`assert_mature`). These checks use `env.ledger().timestamp()` — not caller-supplied values.
- Oracle freshness is verified at redemption time. A stale oracle blocks settlement until the feed is updated.
- Permissioning is checked on both `mint` and `redeem` — closing a gap where a revoked account could previously still redeem for the underlying asset after being flagged.
- PT and YT balances use separate persistent storage keys — there is no shared counter that could be manipulated by burning one token to inflate the other.
- YT yield is floored at zero: if `final_rate <= SCALE`, YT holders receive nothing but PT holders are unaffected.
- Does not yet call `SYWrapper`, `PTToken`, or `YTToken` — mint/redeem operate on internal balance maps. This means the token-level protections below aren't yet reachable through `PrincipalManager`'s own mint/redeem flow (see PROOF_OF_CONCEPT.md's Known Limitations).

### PTToken / YTToken

- `transfer` and `transfer_from` check eligibility on **both** `from` and `to` — checking only the recipient would let a revoked holder freely move its position to any still-eligible party before being frozen.
- Each check is two-layered: the coarse, account-level `Permissioning.is_allowed(account)` gate, and `Permissioning.is_allowed_for_asset(account, own_contract_address)` — a per-token gate that lets PT and YT carry independent eligibility policies for the same market.
- `mint` and `burn` are restricted to a single registered minter, set exactly once via `set_minter` (reverts `MinterAlreadySet` on a second call); both revert `MinterNotSet` if called before a minter is registered. `burn` itself has no eligibility check — it only removes value and never redirects it to a new party, so there's nothing to gate.
- `YTToken.update_yield_index()` is permissionless but requires the oracle to be fresh (`is_fresh(MAX_ORACLE_STALENESS_SECS)`, matching `PrincipalManager`'s own freshness discipline) and is a no-op if the rate hasn't increased since the last recorded high-water mark, so YT can never accrue negative yield.
- Every balance-changing operation (mint, burn, transfer in, transfer out) settles the affected account's pending yield **before** the balance changes, against the current index, then advances that account's snapshot. Without this, a buyer could retroactively receive yield accrued before they held the position, or a seller could lose yield already earned by transferring out.

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

All seven implemented contracts implement `transfer_admin(current_admin, new_admin)` as a single call — the current admin authorizes and the new admin takes effect immediately, with no separate acceptance step. Use this to rotate to a multisig or hardware key:

```bash
stellar contract invoke --id <contract_id> \
  -- transfer_admin \
     --current-admin <old_admin> \
     --new-admin <new_multisig_address>
```

### Compliance recovery (remediate)

```bash
# Requires: caller is SYWrapper's admin, AND account is already revoked in Permissioning
stellar contract invoke --id sy_wrapper \
  -- remediate \
     --caller <admin_address> \
     --account <flagged_account> \
     --shares <amount>
```

Reverts `AccountNotRevoked` if the target hasn't actually been revoked — revoke via `Permissioning.revoke_account` first. In production, `SYWrapper`'s admin role for this action is expected to be an issuer-authorized compliance signer, not the routine protocol admin key.

## 5. Access control matrix

| Action | OracleAdapter | Permissioning | SYWrapper | PrincipalManager | RiskControl | PTToken / YTToken |
|---|---|---|---|---|---|---|
| Initialize | deployer (once) | deployer (once) | deployer (once) | deployer (once) | deployer (once) | deployer (once) |
| Set reference value | admin | — | — | — | — | — |
| Grant/revoke account | — | admin | — | — | — | — |
| Deposit | — | — | eligible account only | — | — | — |
| Withdraw | — | — | eligible sender and eligible recipient | — | — | — |
| Remediate (compliance recovery) | — | — | admin, target must already be revoked | — | — | — |
| Mint PT/YT | — | — | — | permitted user | — | registered minter only, recipient must be eligible |
| Burn PT/YT | — | — | — | — | — | registered minter only, no eligibility check |
| Transfer PT/YT | — | — | — | — | — | eligible sender and eligible recipient (account + per-asset) |
| Redeem PT/YT | — | — | — | eligible PT/YT holder (post-maturity) | — | — |
| Claim yield (YT) | — | — | — | — | — | holder only (`require_auth`) |
| Pause | — | — | admin | admin | admin or pauser | — |
| Unpause | — | — | admin | admin | admin only | — |
| Transfer admin | admin | admin | admin | admin | admin | admin |

## 6. Permissioning and compliance

- `Permissioning.is_allowed(account)` must return `true` for every participant that deposits, withdraws, mints, holds, transfers, or redeems SY, PT, or YT — checked on both sides of every transfer-like operation, not only the recipient, so a revoked account is frozen rather than merely blocked from acquiring new positions.
- `Permissioning.is_allowed_for_asset(account, asset)` provides finer-grained per-asset gating. `PTToken` and `YTToken` both check this against their own contract address, so an admin can grant an account access to PT without granting YT for the same market, or vice versa.
- Eligibility entries in persistent storage expire after `ELIGIBILITY_TTL_LEDGERS` (≈ 30 days). Issuers must refresh entries for active participants before expiry.
- If an underlying asset (e.g. USDY) is permissioned by its issuer, the permissioning contract must mirror those restrictions. The protocol does not create a compliance bypass.
- `SYWrapper.remediate()` gives issuers a way to recover a specific revoked account's value without a native Stellar Asset Contract clawback against the pooled reserve, which would otherwise haircut every other depositor along with the flagged account.

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

Done, at the unit-test level (72 tests across seven contracts):

- [x] Unit tests for arithmetic edge cases (zero amounts, exact-limit deposits, insufficient balance/allowance).
- [x] Oracle failure scenarios: stale price blocks redemption (`PrincipalManager`) and blocks yield-index advancement (`YTToken`).
- [x] Permissioning violations: unauthorized mint reverts; revoked accounts blocked from redeem, deposit, withdraw, and PT/YT transfer on both sides.
- [x] Compliance-recovery correctness: `remediate()` only affects the flagged account's own balance, requires prior revocation, requires admin.
- [x] Yield-accounting correctness: late buyers don't retroactively receive prior yield; transfers settle both sides before the balance moves.
- [x] Circuit breaker trip and window-reset tests.
- [x] Admin rotation tests on all seven implemented contracts.

Still outstanding before mainnet deployment:

- [ ] Integration tests for cross-contract flows once `PrincipalManager` actually calls `SYWrapper`/`PTToken`/`YTToken` (not yet wired — see PROOF_OF_CONCEPT.md's Known Limitations).
- [ ] Cross-contract `RiskControl` wiring and associated integration tests.
- [ ] `MarketPool` and `Router` test coverage once those contracts exist.
- [ ] Third-party security audit with full access to source and test suite.
