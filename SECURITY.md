# Security Controls and Emergency Procedures

## 1. Threat model

| Threat | Impact | Mitigation |
|---|---|---|
| Malicious oracle price | Wrong settlement; PT/YT over/under-redeemed | `require_auth` on price setter; freshness window; multi-source feed in production |
| Unauthorized mint | Inflation of PT/YT supply | `require_auth()` on all entrypoints; permissioning check before mint |
| Permissioning bypass | Ineligible user holds PT/YT | Permissioning checked in PrincipalManager at mint and redeem |
| Replay across maturities | Wrong redemption mapping | Each issuance has unique `maturity_timestamp`; maturity check on every redeem |
| Flash deposit attack | Circuit breaker drained | Rolling 24h window circuit breaker in RiskControl |
| Admin key compromise | Full protocol takeover | Two-step `transfer_admin`; recommend multisig for production |
| Reentrancy | State corruption | Checks-effects-interactions pattern in SYWrapper; state updated before external calls |
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

- Follows checks-effects-interactions: all internal state (`total_underlying`, `total_shares`, `Balance`) is updated **before** the external `token::Client::transfer` call. This prevents reentrancy from manipulating invariants.
- The exchange rate is derived from `total_underlying / total_shares` — it cannot be directly written. An attacker cannot set an arbitrary rate.
- Pause flag blocks both deposits and withdrawals. Only admin can unpause.
- Zero-amount deposits and withdrawals are rejected.
- Withdrawal checks that `balance >= shares` before proceeding, preventing underflow.

### PrincipalManager

- `mint` is blocked after maturity (`assert_not_mature`). `redeem` is blocked before maturity (`assert_mature`). These checks use `env.ledger().timestamp()` — not caller-supplied values.
- Oracle freshness is verified at redemption time. A stale oracle blocks settlement until the feed is updated.
- Permissioning is checked for every `mint` call.
- PT and YT balances use separate persistent storage keys — there is no shared counter that could be manipulated by burning one token to inflate the other.
- YT yield is floored at zero: if `final_rate <= SCALE`, YT holders receive nothing but PT holders are unaffected.

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

All five contracts implement `transfer_admin(current_admin, new_admin)`. The current admin must authorize. Use this to rotate to a multisig or hardware key:

```bash
stellar contract invoke --id <contract_id> \
  -- transfer_admin \
     --current-admin <old_admin> \
     --new-admin <new_multisig_address>
```

## 5. Access control matrix

| Action | OracleAdapter | Permissioning | SYWrapper | PrincipalManager | RiskControl |
|---|---|---|---|---|---|
| Initialize | deployer (once) | deployer (once) | deployer (once) | deployer (once) | deployer (once) |
| Set reference value | admin | — | — | — | — |
| Grant/revoke account | — | admin | — | — | — |
| Deposit | — | — | any (if allowed) | — | — |
| Withdraw | — | — | share holder | — | — |
| Mint PT/YT | — | — | — | permitted user | — |
| Redeem PT/YT | — | — | — | PT/YT holder (post-maturity) | — |
| Pause | — | — | admin | admin | admin or pauser |
| Unpause | — | — | admin | admin | admin only |
| Transfer admin | admin | admin | admin | admin | admin |

## 6. Permissioning and compliance

- `Permissioning.is_allowed(account)` must return `true` for every participant that mints, holds, transfers, or redeems PT or YT.
- `Permissioning.is_allowed_for_asset(account, asset)` provides finer-grained per-asset gating when different assets have different eligibility requirements.
- Eligibility entries in persistent storage expire after `ELIGIBILITY_TTL_LEDGERS` (≈ 30 days). Issuers must refresh entries for active participants before expiry.
- If an underlying asset (e.g. USDY) is permissioned by its issuer, the permissioning contract must mirror those restrictions. The protocol does not create a compliance bypass.

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

Before mainnet deployment:

- [ ] Unit tests for all arithmetic edge cases (zero amounts, exact-limit deposits, rounding at large values).
- [ ] Integration tests for oracle failure scenarios (stale price blocks redemption).
- [ ] Integration tests for permissioning violations (unauthorized mint reverts).
- [ ] Circuit breaker trip and window-reset tests.
- [ ] Admin rotation tests on all five contracts.
- [ ] Third-party security audit with full access to source and test suite.
