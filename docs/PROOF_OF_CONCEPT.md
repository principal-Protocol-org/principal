# Principal Protocol — Proof of Concept

This document describes the current state of the Principal Protocol implementation: eight Soroban smart contracts that form the infrastructure, tokenization, standalone-token, and compliance-recovery layers of the protocol and demonstrate the core yield-tokenization mechanics on Stellar.

---

## Scope

Eight of ten contracts are implemented:

| Contract | Crate | Status |
|---|---|---|
| `OracleAdapter` | `principal_oracle_adapter` | Complete |
| `Permissioning` | `principal_permissioning` | Complete |
| `RiskControl` | `principal_risk_control` | Complete |
| `SYWrapper` | `principal_sy_wrapper` | Complete |
| `PrincipalManager` | `principal_manager` | Complete |
| `PTToken` | `principal_pt_token` | Complete |
| `YTToken` | `principal_yt_token` | Complete |
| `RecoveryEscrow` | `principal_recovery_escrow` | Complete for all three position types — `seize_sy` unwraps immediately; `seize_pt`/`seize_yt` seize, `finalize_pt`/`finalize_yt` redeem through `PrincipalManager` at or after maturity |

`MarketPool` and `Router` are not yet implemented. `PrincipalManager.mint` takes real custody of the caller's `SYWrapper` shares (via a new `transfer` function added to `SYWrapper` for this) and mints real `PTToken`/`YTToken` balances; `redeem` burns those real balances and releases real underlying via `SYWrapper.withdraw`. PT and YT minted through the protocol are genuine SEP-41 balances, holdable in any wallet. Compliance recovery is complete for all three position types: `RecoveryEscrow.seize_sy` unwraps immediately, and `seize_pt`/`seize_yt` plus `finalize_pt`/`finalize_yt` seize and then redeem a flagged PT/YT position through `PrincipalManager` at or after maturity — see [Known Limitations](#known-limitations) below for what's still outstanding (`MarketPool`, `Router`).

`SYWrapper`, `PrincipalManager`, `PTToken`, and `YTToken` all inherit compliance from the underlying Stellar Asset Contract (`underlying_SAC.authorized()`, checked live) as a mandatory floor beneath `Permissioning`'s optional additional layer, and all gate market creation on the underlying SAC's real, live `admin()`. `RecoveryEscrow` gives the issuer a way to recover a deauthorized account's position without a native SAC clawback haircutting every other holder. See [COMPLIANT_SETTLEMENT_DESIGN.md](COMPLIANT_SETTLEMENT_DESIGN.md) for the full design rationale.

---

## What the POC Demonstrates

### 1. Standardized yield wrapping

`SYWrapper` accepts any Stellar yield-bearing asset, holds it, and issues SY shares at a rolling exchange rate. As yield accrues in the underlying, the exchange rate grows, so each SY share is worth progressively more underlying over time. This is the foundational accounting primitive for the entire protocol.

### 2. Principal and yield tokenization

`PrincipalManager` takes real custody of a user's SY shares (via `SYWrapper.transfer`) and splits them into equal PT and YT amounts based on the oracle reference rate at the time of minting, crediting real `PTToken`/`YTToken` balances the user can hold in any wallet. This demonstrates the core economic mechanism: a single position with variable yield is separated into a fixed principal claim and a future yield claim.

### 3. Deterministic maturity settlement

At or after the maturity timestamp, `PrincipalManager.redeem()` burns real PT/YT balances and releases real USDY to the caller via `SYWrapper.withdraw`, using the final oracle rate. The settlement formula uses fixed-point arithmetic with floor rounding, ensuring deterministic and auditable outcomes.

### 4. Oracle integration

`OracleAdapter` stores an admin-submitted USDY/USDC reference value with monotonic timestamp enforcement and freshness checks. This is the trust anchor for minting and redemption pricing.

### 5. Two-layer compliance: authorization inheritance + Permissioning

Every contract checks two independent layers on every affected account: `underlying_SAC.authorized(account)` (the mandatory floor, read live from the actual Stellar Asset Contract the underlying is issued as — no separate registry that could drift out of sync with the issuer's own decisions) and `Permissioning` (an optional, Principal-specific additional layer that can narrow but never loosen the SAC floor). `Permissioning` is checked by `PrincipalManager` on both mint and redemption, and by `SYWrapper` on both deposit and withdrawal — on every path, both the sending and receiving side are checked, so a revoked or deauthorized account is frozen rather than merely blocked from acquiring new positions. This preserves the compliance constraints of the underlying USDY asset across all derived instruments, and reflects the issuer's own authorization decisions immediately rather than requiring a separate action to mirror them.

### 6. Risk controls

`RiskControl` implements a global pause with multi-pauser roles and a rolling 24-hour circuit breaker on deposit volume. These are independent safety layers that can halt the protocol in response to oracle failures, market anomalies, or operational incidents.

### 7. Standalone PT and YT tokens with per-instrument eligibility

`PTToken` and `YTToken` are full SEP-41 tokens (transfer, transfer_from, approve, allowance) with minter-gated mint/burn, restricted to a single registered minter set once via `set_minter`. Transfers check both the coarse, account-level `Permissioning.is_allowed()` gate and a per-token `is_allowed_for_asset()` check keyed to the token's own contract address — so PT and YT can carry independent eligibility policies for the same underlying market, using allow-list infrastructure that already exists rather than new contracts.

### 8. Continuous yield accrual

`YTToken` implements a global yield index, advanced by the permissionless `update_yield_index()` (gated on oracle freshness) and settled via the minter-gated `claim_yield()` — reachable only through `PrincipalManager.claim_yield()`, which pays the result out in real underlying. Every balance-changing operation — mint, burn, transfer in, transfer out — settles the affected account's pending yield against its balance *before* the change, so a buyer cannot retroactively receive yield accrued before they held the position, and a seller cannot lose yield already earned by transferring out. `PrincipalManager.mint` itself calls `update_yield_index()` before crediting a new YT balance, so a fresh mint's own snapshot always starts current, closing the same gap from the other direction.

### 9. Compliance recovery without collateral damage

`RecoveryEscrow.seize_sy`/`seize_pt`/`seize_yt` let the underlying asset's real issuer (authenticated live via `underlying_SAC.admin()`, not a stored key) recover a single deauthorized account's SY, PT, or YT balance without touching any other holder's share of the pooled reserve. Recovery only acts on accounts the issuer has already deauthorized on the SAC itself (`TargetStillAuthorized` otherwise) — deauthorizing and seizing both trace back to the same issuer admin key, but as two explicit, separate on-chain actions. `SYWrapper`/`PTToken`/`YTToken` each expose a `seize()` that trusts calls only from their own configured `RecoveryEscrow` (set once, admin-gated); all issuer-identity and deauthorization verification lives once in `RecoveryEscrow`, not duplicated per contract. `seize_sy` unwraps to raw underlying in the same call, ready for the issuer's native SAC clawback; `seize_pt`/`seize_yt` seize the real balance, and `finalize_pt`/`finalize_yt` complete the unwind at or after maturity by redeeming it through `PrincipalManager`.

### 10. Market-creation gating

`initialize` on `SYWrapper`, `PrincipalManager`, `PTToken`, and `YTToken` requires `admin == underlying_SAC.admin()` (read live) and `admin.require_auth()` — only the entity that actually controls a regulated asset's authorization and clawback can stand up a Principal market on it. This is checked once, at market creation, using the same public, no-auth-required SAC functions everything else in this section relies on.

---

## Contract Interfaces

### OracleAdapter

```rust
fn initialize(env: Env, admin: Address)
fn set_reference_value(env: Env, caller: Address, value: i128, timestamp: u64)
fn get_reference_value(env: Env) -> i128
fn get_reference_timestamp(env: Env) -> u64
fn is_fresh(env: Env, max_stale_seconds: u64) -> bool
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

`value` is scaled by `RATE_SCALE = 10_000_000`. A value of `10_300_000` represents 1.03 USDC per USDY.

### Permissioning

```rust
fn initialize(env: Env, admin: Address)
fn grant_account(env: Env, caller: Address, account: Address)
fn revoke_account(env: Env, caller: Address, account: Address)
fn grant_accounts(env: Env, caller: Address, accounts: Vec<Address>)
fn grant_asset(env: Env, caller: Address, account: Address, asset: Address)
fn revoke_asset(env: Env, caller: Address, account: Address, asset: Address)
fn is_allowed(env: Env, account: Address) -> bool
fn is_allowed_for_asset(env: Env, account: Address, asset: Address) -> bool
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

### SYWrapper

```rust
fn initialize(env: Env, admin: Address, underlying: Address, permissioning: Address)
              // admin must equal underlying.admin() (read live) and must authorize this call
fn deposit(env: Env, from: Address, amount: i128) -> i128       // returns shares minted
fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128  // returns underlying
fn exchange_rate(env: Env) -> i128                               // scaled ×10⁷
fn total_underlying(env: Env) -> i128
fn total_shares(env: Env) -> i128
fn balance_of(env: Env, account: Address) -> i128
fn underlying_address(env: Env) -> Address
fn set_paused(env: Env, caller: Address, paused: bool)
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address

// Share transfer — what lets PrincipalManager take custody of a user's shares at mint
fn transfer(env: Env, from: Address, to: Address, amount: i128) -> i128

// Compliance recovery — see "Compliance recovery without collateral damage" above
fn set_recovery_escrow(env: Env, admin: Address, escrow: Address)  // one-time; reverts if already set
fn seize(env: Env, caller: Address, account: Address, shares: i128) -> i128  // caller must be the configured escrow
fn recovery_escrow(env: Env) -> Address
```

`deposit` checks `underlying.authorized(from)` and `Permissioning.is_allowed(from)`; `withdraw` and `transfer` check both layers on both `from` and `to`. `transfer` is a plain internal balance move (no `TotalUnderlying`/`TotalShares` change, no external token call) — `PrincipalManager.mint` calls it to move a user's shares into its own custody before minting PT/YT. `seize` requires the caller to be the one address configured via `set_recovery_escrow` — it does not itself authenticate the issuer or check deauthorization; that verification lives once in `RecoveryEscrow` (see below). It moves shares to the caller (a forced transfer, not a burn), works even while the contract is paused, and can move at most the target account's own balance.

### PTToken

```rust
fn initialize(env: Env, admin: Address, permissioning: Address, underlying: Address, maturity: u64,
              name: String, symbol: String, decimals: u32)
              // admin must equal underlying.admin() (read live) and must authorize this call
fn set_minter(env: Env, admin: Address, minter: Address)   // one-time; reverts if already set
fn set_recovery_escrow(env: Env, admin: Address, escrow: Address)  // one-time; reverts if already set

// SEP-41
fn transfer(env: Env, from: Address, to: Address, amount: i128)
fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128)
fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32)
fn allowance(env: Env, from: Address, spender: Address) -> i128
fn balance(env: Env, account: Address) -> i128
fn decimals(env: Env) -> u32
fn name(env: Env) -> String
fn symbol(env: Env) -> String

// Minter-only
fn mint(env: Env, to: Address, amount: i128)
fn burn(env: Env, from: Address, amount: i128)

// Compliance recovery
fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128  // caller must be the configured escrow

// Views
fn total_supply(env: Env) -> i128
fn maturity(env: Env) -> u64
fn minter(env: Env) -> Address
fn get_admin(env: Env) -> Address
fn recovery_escrow(env: Env) -> Address
fn underlying_address(env: Env) -> Address
```

`transfer` and `transfer_from` check both `from` and `to` against `underlying.authorized()` (the mandatory floor) and `Permissioning.is_allowed()` (account-level) plus `is_allowed_for_asset(account, pt_token_address)` (per-instrument). `mint` checks the recipient the same way; `burn` has no compliance check, since it only removes value and never redirects it. `seize` trusts only the configured `RecoveryEscrow` address.

### YTToken

Same shape as `PTToken`, plus yield accrual:

```rust
fn initialize(env: Env, admin: Address, permissioning: Address, underlying: Address, oracle: Address, maturity: u64,
              name: String, symbol: String, decimals: u32)
              // admin must equal underlying.admin() (read live) and must authorize this call
fn set_minter(env: Env, admin: Address, minter: Address)
fn set_recovery_escrow(env: Env, admin: Address, escrow: Address)

// SEP-41 — identical to PTToken

// Minter-only — identical to PTToken

// Compliance recovery — settles both sides' pending yield before moving the balance
fn seize(env: Env, caller: Address, account: Address, amount: i128) -> i128

// Yield accrual
fn update_yield_index(env: Env)                       // permissionless; requires fresh oracle
fn claim_yield(env: Env, caller: Address, from: Address) -> i128  // caller must be the registered minter
fn accrued_yield_index(env: Env) -> i128
fn last_claimed_index(env: Env, account: Address) -> i128
fn pending_claim(env: Env, account: Address) -> i128
fn recovery_escrow(env: Env) -> Address
fn underlying_address(env: Env) -> Address
fn permissioning_address(env: Env) -> Address
fn oracle_address(env: Env) -> Address
```

`update_yield_index` reverts with `OracleStale` if the oracle hasn't been refreshed within `MAX_ORACLE_STALENESS_SECS` — matching the same freshness discipline `PrincipalManager` already applies at mint and redeem. `initialize` reads its genesis yield-index baseline live from the oracle (also requiring freshness) rather than hardcoding it to `SCALE`, so a market created when the real rate is already above `SCALE` doesn't baseline against a value the rate never actually was. `claim_yield` is minter-gated the same way `mint`/`burn` already are — see `PrincipalManager.claim_yield` below for the entrypoint that actually reaches it and pays out real underlying.

### RecoveryEscrow

```rust
fn initialize(env: Env, underlying: Address, sy_wrapper: Address, pt_token: Address, yt_token: Address,
              principal_manager: Address)
              // reverts PositionUnderlyingMismatch unless all four report the same underlying_address()
fn seize_sy(env: Env, caller: Address, account: Address, shares: i128) -> i128
              // caller must equal underlying.admin() (read live); account must fail underlying.authorized()
              // seizes via SYWrapper.seize, then immediately unwraps via SYWrapper.withdraw(from=self, to=self)
fn seize_pt(env: Env, caller: Address, account: Address, amount: i128) -> i128
              // same authorization; seizes via PTToken.seize
fn seize_yt(env: Env, caller: Address, account: Address, amount: i128) -> i128
              // same authorization; seizes via YTToken.seize
fn finalize_pt(env: Env, caller: Address, pt_amount: i128) -> i128
              // caller must equal underlying.admin() (read live); redeems this contract's own
              // already-seized PT balance via PrincipalManager.redeem(from=self, pt_amount, 0)
fn finalize_yt(env: Env, caller: Address, yt_amount: i128) -> i128
              // same, for YT; redeems this contract's own already-seized YT balance via
              // PrincipalManager.redeem(from=self, 0, yt_amount)
fn underlying_address(env: Env) -> Address
```

`RecoveryEscrow` has no admin key of its own — every `seize_*`/`finalize_*` call re-derives authority from the underlying SAC's own live `admin()`. It is the single place that authenticates the issuer and checks target deauthorization; `SYWrapper`/`PTToken`/`YTToken` each just trust calls from their own configured `RecoveryEscrow` address. `finalize_pt`/`finalize_yt` redeem the escrow's own already-seized balance through `PrincipalManager` at or after maturity, releasing underlying the same way `seize_sy` does immediately for SY — no separate deauthorization check runs at finalize time, since the target was already verified at seize time and finalize only ever touches the escrow's own balance.

### PrincipalManager

```rust
fn initialize(env: Env, admin: Address, sy_wrapper: Address, pt_token: Address, yt_token: Address,
              oracle: Address, permissioning: Address, underlying: Address, maturity: u64)
              // admin must equal underlying.admin() (read live) and must authorize this call
              // also validates sy_wrapper/pt_token/yt_token share underlying, permissioning,
              // maturity, and oracle -- reverts TopologyMismatch otherwise
fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult
              // reverts OracleStale if the oracle isn't fresh (same check redeem already has)
fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult
fn claim_yield(env: Env, from: Address) -> i128
              // claims accrued YT yield without redeeming/burning YT or waiting for maturity;
              // pays real underlying via SYWrapper.withdraw
fn pt_balance(env: Env, account: Address) -> i128
fn yt_balance(env: Env, account: Address) -> i128
fn total_pt(env: Env) -> i128
fn total_yt(env: Env) -> i128
fn maturity(env: Env) -> u64
fn is_mature(env: Env) -> bool
fn set_paused(env: Env, caller: Address, paused: bool)
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
fn underlying_address(env: Env) -> Address

// Return types
struct MintResult   { pt_minted: i128, yt_minted: i128 }
struct RedeemResult { underlying_from_pt: i128, underlying_from_yt: i128 }
```

`mint` and `redeem` both check `underlying.authorized(from)` and `Permissioning.is_allowed(from)`. `mint` calls `YTToken.update_yield_index()` before crediting the new YT balance (so a fresh mint's own yield snapshot always starts at the just-updated factor, never retroactively earning a rate movement that happened before it existed), then moves the caller's real SY shares into this contract's own custody via `SYWrapper.transfer`, then mints real balances via `PTToken.mint`/`YTToken.mint` (this contract must already be the registered minter on both). `redeem` burns those real balances (`PTToken.burn`/`YTToken.burn`) and releases real underlying via `SYWrapper.withdraw`, self-authorizing as its own contract address the same way `RecoveryEscrow` does when unwrapping a seizure — which means this contract's own address must itself be SAC-authorized and Permissioning-granted (see DEPLOYMENT.md). PT redemption uses this contract's own `pt_amount * SCALE / final_rate` formula; YT redemption instead calls `YTToken.update_yield_index`/`burn`/`claim_yield` and pays out whatever that settles to, rather than computing an independent amount — `claim_yield` is minter-gated, so `PrincipalManager` is the only caller that can ever reach it, closing the double-payment surface a separately-callable public entrypoint would otherwise create (see `redeem_yt_does_not_double_pay_yield_already_claimed_via_claim_yield` in the test suite). `PrincipalManager.claim_yield` provides the same settle-and-pay mechanism without burning YT or requiring maturity, for holders who just want their accrued yield mid-life.

### RiskControl

```rust
fn initialize(env: Env, admin: Address, cb_limit: i128)
fn pause(env: Env, caller: Address)
fn unpause(env: Env, caller: Address)
fn is_paused(env: Env) -> bool
fn add_pauser(env: Env, caller: Address, pauser: Address)
fn remove_pauser(env: Env, caller: Address, pauser: Address)
fn add_consumer(env: Env, caller: Address, consumer: Address)      // admin-gated
fn remove_consumer(env: Env, caller: Address, consumer: Address)   // admin-gated
fn is_consumer(env: Env, account: Address) -> bool
fn check_deposit(env: Env, caller: Address, amount: i128)   // caller must be a registered consumer
fn set_cb_limit(env: Env, caller: Address, new_limit: i128)
fn get_cb_limit(env: Env) -> i128
fn get_cb_volume(env: Env) -> i128
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

`check_deposit` requires `caller` to be a registered consumer (`add_consumer`, admin-gated, mirroring `add_pauser`) — without this, anyone could call it directly with an arbitrary amount to exhaust a day's circuit-breaker budget and block every legitimate depositor. Found and fixed during a post-implementation audit, before this contract was ever wired into a real deposit path.

---

## Settlement Formula

All arithmetic uses `i128` with `SCALE = 10_000_000` (10^7). `RATE_SCALE` is an alias for the same value. Oracle rates are stored in these same units: 1.03 USDC per underlying = `10_300_000`.

Mint (stores `initial_rate` per user for later settlement):
```
initial_rate          = OracleAdapter.get_reference_value()  // e.g. 10_300_000
notional              = sy_shares * initial_rate / SCALE
PT_minted             = notional
YT_minted             = notional
initial_rate_s[user]  = initial_rate                         // stored for YT settlement
```

Redeem (at maturity, given `final_rate` from oracle and per-user `initial_rate`):
```
// PT: redeem principal USDC value → convert to underlying at final rate
usdy_from_pt = floor(pt_amount * SCALE / final_rate)

// YT: redeem yield accrued above initial rate → convert to underlying at final rate
yield_delta  = max(0, final_rate - initial_rate_s[user])
usdy_from_yt = floor(yt_amount * yield_delta / final_rate)
```

Using `initial_rate` (not `SCALE`) in `yield_delta` ensures YT captures only yield accrued since this user's mint, regardless of what the oracle rate was at protocol inception.

---

## Constants

| Constant | Value | Contract | Meaning |
|---|---|---|---|
| `SCALE` | `10_000_000` | All contracts | Universal fixed-point denominator (10^7). `RATE_SCALE` is a deprecated alias for the same value. |
| `ELIGIBILITY_TTL_LEDGERS` | `518_400` | Permissioning | ~30 days at 5 s/ledger |
| `CB_WINDOW_SECS` | `86_400` | RiskControl | 24-hour circuit breaker window |
| `MAX_ORACLE_STALENESS_SECS` | `3_600` | PrincipalManager, YTToken | 1-hour freshness at redemption and at yield-index advancement |
| `BALANCE_TTL_LEDGERS` | `518_400` | SYWrapper, PrincipalManager, PTToken, YTToken | ~30 days at 5 s/ledger, applied to every persistent per-user entry |

---

## Error Codes

| Contract | Code | Error | Trigger |
|---|---|---|---|
| OracleAdapter | 1 | `AlreadyInitialized` | `initialize` called twice |
| OracleAdapter | 2 | `Unauthorized` | caller ≠ admin |
| OracleAdapter | 3 | `InvalidValue` | value ≤ 0 |
| OracleAdapter | 4 | `TimestampTooOld` | new timestamp ≤ stored |
| OracleAdapter | 5 | `NotInitialized` | read before initialize |
| OracleAdapter | 6 | `ValueDecreased` | new value < currently stored value (equal values still allowed) |
| Permissioning | 1 | `AlreadyInitialized` | initialize called twice |
| Permissioning | 2 | `Unauthorized` | caller ≠ admin |
| Permissioning | 3 | `NotInitialized` | read before initialize |
| SYWrapper | 1 | `AlreadyInitialized` | initialize called twice |
| SYWrapper | 2 | `Unauthorized` | caller ≠ admin |
| SYWrapper | 3 | `NotInitialized` | read before initialize |
| SYWrapper | 4 | `ZeroAmount` | deposit or withdraw ≤ 0 |
| SYWrapper | 5 | `InsufficientShares` | withdraw > balance |
| SYWrapper | 6 | `Paused` | operation while paused |
| SYWrapper | 7 | `ArithmeticOverflow` | fixed-point overflow |
| SYWrapper | 8 | `PermissionDenied` | account not in allow-list |
| SYWrapper | 9 | `NotAuthorizedOnSac` | account fails `underlying.authorized()` |
| SYWrapper | 10 | `RecoveryEscrowAlreadySet` | `set_recovery_escrow` called twice |
| SYWrapper | 11 | `NotRecoveryEscrow` | `seize` called by an address other than the configured escrow |
| SYWrapper | 12 | `IssuerMismatch` | `initialize` called with `admin` ≠ `underlying.admin()` |
| PrincipalManager | 1 | `AlreadyInitialized` | initialize called twice |
| PrincipalManager | 2 | `Unauthorized` | caller ≠ admin |
| PrincipalManager | 3 | `NotInitialized` | read before initialize |
| PrincipalManager | 4 | `ZeroAmount` | amount ≤ 0 |
| PrincipalManager | 5 | `NotMature` | redeem before maturity |
| PrincipalManager | 6 | `AlreadyMature` | mint after maturity |
| PrincipalManager | 7 | `OracleStale` | oracle too old at redemption |
| PrincipalManager | 8 | `InsufficientBalance` | redeem > PT or YT balance |
| PrincipalManager | 9 | `Paused` | operation while paused |
| PrincipalManager | 10 | `PermissionDenied` | user not in allow-list (mint and redeem) |
| PrincipalManager | 11 | `NotAuthorizedOnSac` | account fails `underlying.authorized()` (mint and redeem) |
| PrincipalManager | 12 | `IssuerMismatch` | `initialize` called with `admin` ≠ `underlying.admin()` |
| PrincipalManager | 13 | `TopologyMismatch` | `sy_wrapper`/`pt_token`/`yt_token` don't share `underlying`, `permissioning`, `maturity`, or (for YT) `oracle` at initialize |
| PTToken | 1 | `AlreadyInitialized` | initialize called twice |
| PTToken | 2 | `Unauthorized` | caller ≠ admin |
| PTToken | 3 | `NotInitialized` | read before initialize |
| PTToken | 4 | `ZeroAmount` | amount ≤ 0 |
| PTToken | 5 | `InsufficientBalance` | transfer/burn > balance |
| PTToken | 6 | `InsufficientAllowance` | transfer_from > allowance or allowance expired |
| PTToken | 7 | `PermissionDenied` | account fails account-level or per-asset eligibility |
| PTToken | 8 | `MinterAlreadySet` | `set_minter` called twice |
| PTToken | 9 | `MinterNotSet` | mint/burn before `set_minter` |
| PTToken | 10 | `NotAuthorizedOnSac` | account fails `underlying.authorized()` |
| PTToken | 11 | `IssuerMismatch` | `initialize` called with `admin` ≠ `underlying.admin()` |
| PTToken | 12 | `RecoveryEscrowAlreadySet` | `set_recovery_escrow` called twice |
| PTToken | 13 | `NotRecoveryEscrow` | `seize` called by an address other than the configured escrow |
| YTToken | 1–9 | *(same as PTToken)* | identical error set |
| YTToken | 10 | `OracleStale` | `update_yield_index` called with a stale oracle |
| YTToken | 11 | `NotAuthorizedOnSac` | account fails `underlying.authorized()` |
| YTToken | 12 | `IssuerMismatch` | `initialize` called with `admin` ≠ `underlying.admin()` |
| YTToken | 13 | `RecoveryEscrowAlreadySet` | `set_recovery_escrow` called twice |
| YTToken | 14 | `NotRecoveryEscrow` | `seize` called by an address other than the configured escrow |
| RecoveryEscrow | 1 | `AlreadyInitialized` | initialize called twice |
| RecoveryEscrow | 2 | `NotInitialized` | any `seize_*`/`finalize_*` called before initialize |
| RecoveryEscrow | 3 | `Unauthorized` | caller ≠ `underlying.admin()` (read live) |
| RecoveryEscrow | 4 | `TargetStillAuthorized` | target account is still `authorized()` on the underlying SAC |
| RecoveryEscrow | 5 | `ZeroAmount` | seize/finalize amount or shares ≤ 0 |
| RecoveryEscrow | 6 | `PositionUnderlyingMismatch` | a position contract's `underlying_address()` doesn't match at initialize |
| RiskControl | 1 | `AlreadyInitialized` | initialize called twice |
| RiskControl | 2 | `Unauthorized` | caller ≠ admin |
| RiskControl | 3 | `NotInitialized` | read before initialize |
| RiskControl | 4 | `Paused` | check_deposit while paused |
| RiskControl | 5 | `CircuitBreakerTripped` | deposit exceeds rolling limit |
| RiskControl | 6 | `NotPauser` | pause called by non-pauser |
| RiskControl | 7 | `AlreadyPauser` | add_pauser for existing pauser |
| RiskControl | 8 | `NotConsumer` | check_deposit called by an unregistered caller |
| RiskControl | 9 | `AlreadyConsumer` | add_consumer for an already-registered consumer |
| RiskControl | 10 | `ZeroAmount` | `check_deposit` amount ≤ 0 |

---

## Test Coverage

175 unit tests across eight contracts, plus a 15-test cross-contract integration suite (190 total), using `soroban_sdk::testutils`. Line coverage is 98.5% workspace-wide (`cargo llvm-cov --workspace`); the remaining uncovered lines are `#[contracttype]`/`#[contracterror]` derive-macro artifacts and two `SYWrapper` rounding-edge branches that require an artificially inflated exchange rate to reach and are not exercisable through any real deposit sequence.

**OracleAdapter** (14 tests)
- Initialization and double-init guard
- Reference value update and retrieval
- Monotonic timestamp enforcement (reject stale timestamps)
- Monotonic value enforcement (reject a value decrease; an equal-value resubmission is still allowed)
- Freshness check (`is_fresh` with varying staleness thresholds, and the case where the stored timestamp is somehow ahead of the ledger clock)
- Unauthorized update attempt; unauthorized `transfer_admin` attempt
- Admin transfer

**Permissioning** (7 tests)
- Initialization and double-init guard
- Account grant and revoke
- Asset-level grant and revoke
- Batch `grant_accounts`
- `is_allowed` and `is_allowed_for_asset` return values
- Unauthorized grant attempt
- Admin transfer

**SYWrapper** (29 tests)
- Initialization and double-init guard; rejects an admin that doesn't match the underlying SAC's real, live `admin()`
- Deposit and share minting; rejected when the depositor isn't on the Permissioning allow-list or isn't `authorized()` on the underlying SAC
- A deauthorized-on-SAC account cannot self-withdraw to front-run seizure
- Withdrawal and underlying return; rejected when either the withdrawer or the recipient isn't permitted
- Insufficient share balance rejection
- Pause and unpause behavior; non-admin `set_paused` rejection
- Exchange rate calculation at inception, and before any deposit has ever happened
- Zero-deposit rejection; `balance_of` correctness
- Admin transfer; `get_admin`/`underlying_address`/`permissioning_address`/`recovery_escrow` getters
- `transfer` moves shares between eligible accounts (used by `PrincipalManager.mint` to take custody)
- `transfer` rejected for an unpermitted recipient, or for more than the sender's balance
- `set_recovery_escrow` succeeds once, reverts on a second call
- `seize()` moves the flagged account's balance to the configured escrow without the holder's authorization; rejects a zero-share seizure
- `seize()` requires the caller to be the configured escrow
- `seize()` cannot exceed the target's balance
- `seize()` works while the contract is paused
- The escrow can unwrap seized SY via a normal `withdraw` call

**PrincipalManager** (27 tests)
- Initialization and double-init guard; rejects an admin that doesn't match the underlying SAC's real, live `admin()`
- `initialize` rejects mismatched underlying, permissioning, maturity, or oracle across `sy_wrapper`/`pt_token`/`yt_token`
- Mint PT and YT from SY shares — real custody moves via `SYWrapper.transfer`, real balances credited via `PTToken.mint`/`YTToken.mint`; total supply tracking on mint and redeem; rejects a zero-share mint
- A second mint after a rate movement doesn't retroactively receive the first mint's prior yield
- Redeem at maturity — PT and YT separately, including the zero-yield case; PT redemption releases the actual underlying to the caller; rejects a zero-amount redeem (both legs zero)
- YT redemption's underlying amount matches `YTToken`'s own index-based `claim_yield` result, not an independently computed formula
- `PrincipalManager.claim_yield` pays real underlying ahead of redemption, and a later `redeem()` doesn't pay the same yield again
- Calling `YTToken.claim_yield` directly (not through `PrincipalManager`) is rejected
- Redeem before maturity rejection (`NotMature`); mint after maturity rejection (`AlreadyMature`)
- Oracle staleness rejection at both mint and redemption
- Permission check rejection at mint (unpermissioned user) and at redeem (revoked user)
- A deauthorized-on-SAC account cannot mint or redeem
- Admin transfer; a non-admin cannot call `transfer_admin`

**PTToken** (29 tests)
- Initialization and double-init guard; rejects an admin that doesn't match the underlying SAC's real, live `admin()`
- Mint blocked until `set_minter` is called; `set_minter` cannot be called twice, and a non-admin caller is rejected
- Mint and balance tracking; mint rejected for an account not `authorized()` on the underlying SAC, and for a zero amount
- Transfer between eligible accounts; rejected when the recipient lacks the per-token asset grant, for a zero amount, or for more than the sender's balance
- A revoked or deauthorized-on-SAC holder cannot transfer PT to a still-eligible party before seizure
- `approve` / `transfer_from` delegated transfer; rejected for a negative approval amount, a zero transfer amount, an exceeded or expired allowance, or insufficient underlying balance
- Burn reduces total supply; rejected for a zero amount or insufficient balance
- `seize()` moves the flagged account's balance to the configured escrow without the holder's authorization; rejects a zero amount or an amount exceeding the target's balance
- `seize()` requires the caller to be the configured escrow
- `set_recovery_escrow` cannot be called twice
- `decimals`/`name`/`symbol`/`total_supply`/`maturity`/`minter`/`get_admin`/`recovery_escrow`/`underlying_address`/`permissioning_address` view getters

**YTToken** (35 tests)
- Initialization and double-init guard; rejects an admin that doesn't match the underlying SAC's real, live `admin()`
- `initialize` reverts `OracleStale` when the oracle isn't fresh at market creation
- A market created with the oracle already above `SCALE` doesn't overpay the first mint
- Mint and balance tracking; mint rejected for an account not `authorized()` on the underlying SAC, for a zero amount, or for an account granted account-level but not per-asset Permissioning
- `set_minter`/`set_recovery_escrow` each settable only once, and reject a non-admin caller
- No yield accrual when the oracle rate hasn't increased
- Yield accrues correctly after a rate increase, and is fully claimable
- Yield is path-independent: many small oracle updates between mint and claim produce the same result (within ordinary floor-rounding dust) as one big jump straight to the final rate — the regression test for the multiplicative-index fix (see SECURITY.md)
- A late buyer does not retroactively receive yield accrued before they held the position
- A transfer settles both sides' pending yield before the balance moves; rejected for a zero amount or insufficient balance
- `approve` / `transfer_from` delegated transfer; rejected for a negative approval amount, a zero transfer amount, an exceeded or expired allowance, or insufficient underlying balance
- A revoked or deauthorized-on-SAC holder cannot transfer YT to a still-eligible party before seizure
- `update_yield_index` rejects a stale oracle
- `claim_yield` rejects a non-minter caller
- Burn reduces total supply; rejected for a zero amount or insufficient balance
- `seize()` settles both sides' pending yield before moving the balance; rejects a zero amount or an amount exceeding the target's balance
- `seize()` requires the caller to be the configured escrow
- `decimals`/`name`/`symbol`/`maturity`/`minter`/`get_admin`/`recovery_escrow`/`underlying_address`/`permissioning_address`/`oracle_address`/`last_claimed_index`/`pending_claim` view getters

**RecoveryEscrow** (16 tests)
- Initialization and double-init guard
- `seize_sy` seizes and immediately unwraps to underlying in the same call; rejects a zero-share seizure
- `seize_sy` requires the caller to be the underlying SAC's real, live issuer admin
- `seize_sy` requires the target account to already be deauthorized on the underlying SAC
- `initialize` rejects a position contract, or a `PrincipalManager`, whose `underlying_address()` doesn't match the others
- `seize_pt`/`seize_yt` move the flagged account's real PTToken/YTToken balance to the escrow (minted and deposited through a real `PrincipalManager.mint()` call, not a mock); each rejects a zero-amount seizure
- `finalize_pt`/`finalize_yt` redeem the escrow's own seized balance through a real `PrincipalManager` deployment at or after maturity, releasing real underlying to the escrow; each rejects a zero-amount finalize
- `finalize_pt` requires the caller to be the underlying SAC's real, live issuer admin

**RiskControl** (18 tests)
- Initialization and double-init guard
- Pause and unpause
- Pauser role add and remove
- Non-pauser `pause` rejection
- Non-admin `unpause` rejection
- Circuit breaker trip on volume excess
- Circuit breaker window reset after 24 hours
- Disabled circuit breaker (cb_limit = 0)
- `check_deposit` rejects an unregistered caller, and stops working for a caller whose consumer registration was removed
- `check_deposit` rejects a zero or negative amount
- `add_consumer` rejects a duplicate registration
- Admin transfer

### Integration Tests (`contracts/integration_tests`, `cargo test -p principal_integration_tests`)

Unlike each contract's own unit tests -- which mostly deploy just the handful of dependencies needed to exercise that one contract's own logic -- this crate deploys the entire eight-contract stack together in `tests/common/mod.rs`'s `deploy_stack` helper and drives it through realistic, multi-step flows the way a real integrator would, split across three files:

**`tests/full_lifecycle.rs`** (4 tests)
- Deposit, split into PT + YT, an allowance-based `approve`/`transfer_from` of PT between two users, a mid-life `PrincipalManager.claim_yield` that pays real underlying, and maturity redemption for both users -- checking `PrincipalManager`'s own `pt_balance`/`yt_balance`/`total_pt`/`total_yt`/`maturity`/`is_mature`/`underlying_address` views at each step, not just the token contracts directly
- A second mint after a rate movement doesn't dilute the first minter's already-accrued yield, verified at the `PrincipalManager.claim_yield` level
- `PrincipalManager.set_paused` blocks mint; unpausing restores it

**`tests/compliance_recovery.rs`** (4 tests)
- `seize_sy` unwraps immediately for a flagged depositor, ready for the issuer's native SAC clawback
- Full seize-then-finalize of both PT and YT positions after maturity, verifying the escrow ends up holding real underlying equal to what both `finalize_pt`/`finalize_yt` returned
- Seizure requires the target already deauthorized, and requires the real issuer admin

**`tests/admin_and_risk_control.rs`** (7 tests)
- Admin rotation across every admin-bearing contract (`OracleAdapter`, `Permissioning`, `RiskControl`, `SYWrapper`, `PrincipalManager` -- `PTToken`/`YTToken` have no `transfer_admin`, only a one-time market-creation admin), followed by a real mint proving the market still functions under the new admin; the old admin key no longer works afterward
- Pauser add/remove lifecycle, and that only the admin can unpause
- `RiskControl`'s consumer-registration, circuit-breaker trip, and window-reset behavior exercised from a registered consumer's perspective (standing in for the not-yet-wired `SYWrapper`/`PrincipalManager` call sites)

---

## Build and Run

```bash
# Prerequisites
rustup target add wasm32-unknown-unknown

# Run all tests
cargo test

# Build WASM artifacts
cargo build --target wasm32-unknown-unknown --release
```

WASM files are produced in `target/wasm32-unknown-unknown/release/` (or `target/wasm32v1-none/release/`, depending on Rust toolchain version — see note above):
- `principal_oracle_adapter.wasm`
- `principal_permissioning.wasm`
- `principal_risk_control.wasm`
- `principal_sy_wrapper.wasm`
- `principal_manager.wasm`
- `principal_pt_token.wasm`
- `principal_yt_token.wasm`
- `principal_recovery_escrow.wasm`

---

## Deployment Order

```bash
# 1. OracleAdapter
stellar contract deploy --wasm target/.../principal_oracle_adapter.wasm \
  --source admin --network testnet --alias oracle_adapter
stellar contract invoke --id oracle_adapter --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS>

# 2. Permissioning
stellar contract deploy --wasm target/.../principal_permissioning.wasm \
  --source admin --network testnet --alias permissioning
stellar contract invoke --id permissioning --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS>

# 3. RiskControl
stellar contract deploy --wasm target/.../principal_risk_control.wasm \
  --source admin --network testnet --alias risk_control
stellar contract invoke --id risk_control --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --cb-limit 0

# 4. SYWrapper (initialize now also takes --permissioning; --source must be the
#    underlying SAC's real admin key, or this reverts IssuerMismatch)
stellar contract deploy --wasm target/.../principal_sy_wrapper.wasm \
  --source admin --network testnet --alias sy_wrapper
stellar contract invoke --id sy_wrapper --source admin --network testnet \
  -- initialize \
     --admin <ADMIN_ADDRESS> \
     --underlying <USDY_CONTRACT_ADDRESS> \
     --permissioning <PERMISSIONING_ADDRESS>

# 5. PTToken -- must be deployed before PrincipalManager, which now requires its address at
#    initialize (--source must be the underlying SAC's real admin key)
stellar contract deploy --wasm target/.../principal_pt_token.wasm \
  --source admin --network testnet --alias pt_token
stellar contract invoke --id pt_token --source admin --network testnet \
  -- initialize \
     --admin <ADMIN_ADDRESS> \
     --permissioning <PERMISSIONING_ADDRESS> \
     --underlying <USDY_CONTRACT_ADDRESS> \
     --maturity <UNIX_TIMESTAMP> \
     --name "Principal Token USDY" --symbol "PT-USDY" --decimals 7
     # UNIX_TIMESTAMP = current time + maturity duration in seconds
     # Example for 3-month market: $(date -d "+90 days" +%s)
     # Example for 6-month market: $(date -d "+180 days" +%s)
# (no minter yet -- two-phase init; set_minter is called in step 7 below)

# 6. YTToken (--source must be the underlying SAC's real admin key)
stellar contract deploy --wasm target/.../principal_yt_token.wasm \
  --source admin --network testnet --alias yt_token
stellar contract invoke --id yt_token --source admin --network testnet \
  -- initialize \
     --admin <ADMIN_ADDRESS> \
     --permissioning <PERMISSIONING_ADDRESS> \
     --underlying <USDY_CONTRACT_ADDRESS> \
     --oracle <ORACLE_ADAPTER_ADDRESS> \
     --maturity <UNIX_TIMESTAMP> \
     --name "Yield Token USDY" --symbol "YT-USDY" --decimals 7
# (no minter yet)

# 7. PrincipalManager (--source must also be the underlying SAC's real admin key)
stellar contract deploy --wasm target/.../principal_manager.wasm \
  --source admin --network testnet --alias principal_manager
stellar contract invoke --id principal_manager --source admin --network testnet \
  -- initialize \
     --admin <ADMIN_ADDRESS> \
     --sy-wrapper <SY_WRAPPER_ADDRESS> \
     --pt-token <PT_TOKEN_ADDRESS> \
     --yt-token <YT_TOKEN_ADDRESS> \
     --oracle <ORACLE_ADAPTER_ADDRESS> \
     --permissioning <PERMISSIONING_ADDRESS> \
     --underlying <USDY_CONTRACT_ADDRESS> \
     --maturity <UNIX_TIMESTAMP>

# 7.5 Wire minters, then grant PrincipalManager's own address both compliance layers --
#     it is now a genuine SY holder between mint and redemption, and both sender and
#     recipient on its own SYWrapper.transfer/withdraw calls.
stellar contract invoke --id pt_token --source admin --network testnet \
  -- set_minter --admin <ADMIN_ADDRESS> --minter <PRINCIPAL_MANAGER_ADDRESS>
stellar contract invoke --id yt_token --source admin --network testnet \
  -- set_minter --admin <ADMIN_ADDRESS> --minter <PRINCIPAL_MANAGER_ADDRESS>
stellar contract invoke --id permissioning --source admin --network testnet \
  -- grant_account --caller <ADMIN_ADDRESS> --account <PRINCIPAL_MANAGER_ADDRESS>
stellar contract invoke --id <USDY_CONTRACT_ADDRESS> --source admin --network testnet \
  -- set_authorized --id <PRINCIPAL_MANAGER_ADDRESS> --authorize true

# 8. RecoveryEscrow (no admin of its own; validates all four contracts below
#    share the same underlying, including PrincipalManager)
stellar contract deploy --wasm target/.../principal_recovery_escrow.wasm \
  --source admin --network testnet --alias recovery_escrow
stellar contract invoke --id recovery_escrow --source admin --network testnet \
  -- initialize \
     --underlying <USDY_CONTRACT_ADDRESS> \
     --sy-wrapper <SY_WRAPPER_ADDRESS> \
     --pt-token <PT_TOKEN_ADDRESS> \
     --yt-token <YT_TOKEN_ADDRESS> \
     --principal-manager <PRINCIPAL_MANAGER_ADDRESS>

# Wire the escrow into each token contract (one-time, admin-gated):
stellar contract invoke --id sy_wrapper --source admin --network testnet \
  -- set_recovery_escrow --admin <ADMIN_ADDRESS> --escrow <RECOVERY_ESCROW_ADDRESS>
stellar contract invoke --id pt_token --source admin --network testnet \
  -- set_recovery_escrow --admin <ADMIN_ADDRESS> --escrow <RECOVERY_ESCROW_ADDRESS>
stellar contract invoke --id yt_token --source admin --network testnet \
  -- set_recovery_escrow --admin <ADMIN_ADDRESS> --escrow <RECOVERY_ESCROW_ADDRESS>
```

`PTToken`, `YTToken`, and `RecoveryEscrow` are not part of the live testnet deployment recorded below — that deployment predates all three, and predates the authorization-inheritance, market-creation-gating, and `PrincipalManager` wiring work entirely. The commands above are untested reference templates, not a record of an actual deployment.

See [DEPLOYMENT.md](DEPLOYMENT.md) for the complete guide including network configuration and post-deployment verification.

---

## Current Testnet Deployment — Full Stack, PT/YT, and Compliance Recovery (30 July 2026)

All eight contracts — including `PTToken`, `YTToken`, and `RecoveryEscrow`, none of which existed at the time of the historical deployment recorded further below — are live on **Stellar Testnet**, reflecting the current source after every fix described in this document and in SECURITY.md. This is a real deployment: every address below is a real contract, every transaction hash resolves on [stellar.expert](https://stellar.expert/explorer/testnet), and every balance was moved by the contracts' own logic, not asserted.

Since Ondo's real USDY only exists on Stellar mainnet, testnet uses a purpose-issued classic Stellar asset, **STA**, standing in for it: `AUTH_REQUIRED` and `AUTH_REVOCABLE` are both set on the issuer (matching a real regulated asset), and it is wrapped as a Stellar Asset Contract (SAC) the same way USDY would be. Two independent participants hold STA — Alice and Bob — chosen to demonstrate the two things this protocol is actually for: ordinary continuous-yield tokenization, and issuer-initiated compliance recovery that touches only the flagged account.

### Deployed Contract Addresses

| Contract | Address |
|---|---|
| OracleAdapter | [`CDBIVWBLB6UEOBIWO5HAFYPM7LPMH3GKDRG4UMTVQDEAJRY3OIWJLPUO`](https://stellar.expert/explorer/testnet/contract/CDBIVWBLB6UEOBIWO5HAFYPM7LPMH3GKDRG4UMTVQDEAJRY3OIWJLPUO) |
| Permissioning | [`CBTV5C3GSQKYTAMHOVET7RH25BKSFSAEENTP7SRZEEBD6LKHE24MLXIM`](https://stellar.expert/explorer/testnet/contract/CBTV5C3GSQKYTAMHOVET7RH25BKSFSAEENTP7SRZEEBD6LKHE24MLXIM) |
| RiskControl | [`CCQXRR3SEP7UTSJORW43V2COD4D3HR6FCLZFJSPXBEH7W4WJWY7VTGC3`](https://stellar.expert/explorer/testnet/contract/CCQXRR3SEP7UTSJORW43V2COD4D3HR6FCLZFJSPXBEH7W4WJWY7VTGC3) |
| STA (SAC, stand-in for USDY) | [`CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L`](https://stellar.expert/explorer/testnet/contract/CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L) |
| SYWrapper | [`CA23M3FMEZJL5MHYTCDLSU7NG4MJ5UYAAKIT5QC4W2TO4SDZQ5EX3XMN`](https://stellar.expert/explorer/testnet/contract/CA23M3FMEZJL5MHYTCDLSU7NG4MJ5UYAAKIT5QC4W2TO4SDZQ5EX3XMN) |
| PTToken | [`CDHAJVFKVHJ3NTSVUVPSXEEFZT6LKHLOOQ35KPU7T6Z64XQEEGEE76ML`](https://stellar.expert/explorer/testnet/contract/CDHAJVFKVHJ3NTSVUVPSXEEFZT6LKHLOOQ35KPU7T6Z64XQEEGEE76ML) |
| YTToken | [`CALVCKLXBODNE6AD5KRJY2TWX2WNGQIVIGXUCBOS7AFXC3Q6M5XMA2L2`](https://stellar.expert/explorer/testnet/contract/CALVCKLXBODNE6AD5KRJY2TWX2WNGQIVIGXUCBOS7AFXC3Q6M5XMA2L2) |
| PrincipalManager | [`CDKBWCBFPIAVHYGKT6PUGU6ALELWCWXFC23NM3TYFYH6XLETVVMF3LRP`](https://stellar.expert/explorer/testnet/contract/CDKBWCBFPIAVHYGKT6PUGU6ALELWCWXFC23NM3TYFYH6XLETVVMF3LRP) |
| RecoveryEscrow | [`CCH4DZ6B64IMY7CNINWSFUI46PTL4B266RZ63EZZWHW5AGGAAN5WNSKW`](https://stellar.expert/explorer/testnet/contract/CCH4DZ6B64IMY7CNINWSFUI46PTL4B266RZ63EZZWHW5AGGAAN5WNSKW) |

**Issuer / protocol admin:** `GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB` — the STA issuer's own key, satisfying every contract's market-creation gate (`admin == underlying_SAC.admin()`).
**Alice** (ordinary holder): `GAK3XILRBYBMBOCZMSLL2CLR6WPQLEIOC6ZCYYPTE4OIAX3PCFFO2YMU`
**Bob** (later flagged and recovered): `GDXVIRLSBDKT7EZM2RM3FH26W3TPF77IJ7GZBA5IOA6ZJBTW26NNO3AV`
**Market maturity:** `1785427014` (a short-dated testnet market, reached and redeemed within this same deployment run)

### Setup

| Step | Transaction |
|---|---|
| Issuer sets `AUTH_REQUIRED`+`AUTH_REVOCABLE` on the STA issuing account | [c8722cb3…](https://stellar.expert/explorer/testnet/tx/c8722cb32fbc84fd2398f7eeb85c537e971c0164d588384f226d1e99b63f3fe0) |
| Bob establishes a classic trustline to STA | [ac9e3a7a…](https://stellar.expert/explorer/testnet/tx/ac9e3a7ac6f6d6f525c0b009ae71274dd6119b8e920f9297a3939839c824a5a4) |
| Issuer authorizes Bob's trustline | [794d70a0…](https://stellar.expert/explorer/testnet/tx/794d70a03c4d127a3903234ebae02d6505013c934a93b5d0df1088892f57cc1d) |
| Issuer pays 1,000 STA to Bob | [82dc30b3…](https://stellar.expert/explorer/testnet/tx/82dc30b304eed4b61502ff47aa26936d08fe56090eb2f2e6b0cb2ae0c1589c03) |
| Issuer pays 1,000 STA to Alice | [66f60592…](https://stellar.expert/explorer/testnet/tx/66f60592c6dba8b7c4deddfa53b115fd7e45b70b870eda5d328fe62ce803f663) |
| OracleAdapter.initialize + set_reference_value (genesis 1.00) | [5ef5655f…](https://stellar.expert/explorer/testnet/tx/5ef5655f9d3c465e05b4011a1624e8cf23a994775065e3bcb0827b4fd4b3e4fa), [2073b9d5…](https://stellar.expert/explorer/testnet/tx/2073b9d5dc2cf4a33e69a85012b9da2db6bc60a4729c355538641a811e2b4ba0) |
| Permissioning.initialize | [45bbcbc7…](https://stellar.expert/explorer/testnet/tx/45bbcbc7ecf68f6d82ac35ff047fa0b79757f1efbb2a1f713a83c497f0ddca18) |
| RiskControl.initialize | [a53cbe5a…](https://stellar.expert/explorer/testnet/tx/a53cbe5a9a001b7e03cd48e1cdf6b9216c3d2f8e860bf0694f1e83368211ace4) |
| SYWrapper.initialize | [173d434d…](https://stellar.expert/explorer/testnet/tx/173d434d4f7ae3fcee0f06a67200e06a2aa4b4f4578c4a1c7eb7f991b999e977) |
| PTToken / YTToken / PrincipalManager.initialize, `set_minter` × 2, RecoveryEscrow.initialize, `set_recovery_escrow` × 2 | market-creation and wiring transactions, all under the issuer's key, per [DEPLOYMENT.md](DEPLOYMENT.md)'s order |
| SYWrapper, PrincipalManager, and RecoveryEscrow's own contract addresses each granted `Permissioning.grant_account` and `underlying.set_authorized(id, true)` | required because they custody real STA on users' behalf — a Soroban contract address has no classic trustline, so the SAC's own `set_authorized` (not `set-trustline-flags`, which only accepts classic accounts) is what authorizes it |
| Alice and Bob each granted `grant_account` + `grant_asset` for PT and YT | per-asset eligibility, mirroring PTToken/YTToken's independent-policy design |

One real-world wrinkle worth recording: the first PT/YT/PrincipalManager instances were deployed with a maturity 240 seconds out, and the wiring transactions above alone took longer than that on real testnet (each one is a real ledger close, not a simulated instant test). `PrincipalManager.mint` correctly reverted `AlreadyMature` rather than let a deposit past its own maturity — exactly the check `mint_after_maturity_panics` exercises in the unit tests — so PT, YT, PrincipalManager, and RecoveryEscrow were redeployed with a 900-second maturity and rewired; the addresses above are that corrected deployment. SYWrapper and Alice's SY shares were unaffected and reused as-is, since SYWrapper carries no maturity of its own.

### Thread 1 — Alice: continuous yield accrual, claimed without redeeming

| Step | Transaction | Result |
|---|---|---|
| `SYWrapper.deposit(from=Alice, amount=500 STA)` | [5728686d…](https://stellar.expert/explorer/testnet/tx/5728686d24cca4e4b5c40493b4c40d5a8ee1d21e92696ae0cca9ffaa38e3dd7e) | 5,000,000,000 SY shares (1:1 at inception) |
| `PrincipalManager.mint(from=Alice, sy_shares=5,000,000,000)` | [36860d63…](https://stellar.expert/explorer/testnet/tx/36860d63c31344a7e6ea6035c506a0906b3bf98c8d2a9a30c8f488e0d0795930) | 500 PT + 500 YT minted |
| Oracle rate moves 1.00 → 1.08 (simulated NAV appreciation) | [21e30297…](https://stellar.expert/explorer/testnet/tx/21e30297bf72240beb2e2472cd8f992a21e051d1e0fc6cf8d4064e2fec7f615f) | — |
| **`PrincipalManager.claim_yield(from=Alice)`** — mid-life, no PT/YT burned | [cbacf10c…](https://stellar.expert/explorer/testnet/tx/cbacf10c43ce153a95383d15b27136e7b839a4e922b8d5b632c597d3b7bde8d7) | **37.03705 STA paid to Alice's wallet in real underlying**, verified by her balance moving from 500.5 → 537.53705 STA; her PT and YT balances stayed at 500/500 |
| Oracle rate moves 1.08 → 1.10; maturity reached | [f537ddca…](https://stellar.expert/explorer/testnet/tx/f537ddca936369445bbf83af20bbded9a7a7b1d0a70e03fd816c72b6c53d61b7) | — |
| `PrincipalManager.redeem(from=Alice, pt_amount=500, yt_amount=500)` | [9ae67f42…](https://stellar.expert/explorer/testnet/tx/9ae67f429e2fda7978ae76c72fc5ae02f893dcddc42a8e23415db5de0627f957) | 454.5454545 STA (principal) + 9.0909542 STA (yield accrued since the mid-life claim) |

Alice's final wallet balance: **1,001.1734587 STA** — every unit accounted for across two claims and one redemption, exactly matching `500 (unspent) + 37.03705 (mid-life claim) + 454.5454545 (PT principal) + 9.0909542 (remaining YT yield)`. Her PT and YT balances end at zero; `PrincipalManager.total_pt()`/`total_yt()` both read `0` once every position in the market has been settled.

### Thread 2 — Bob: issuer-initiated compliance recovery, isolated to one account

| Step | Transaction | Result |
|---|---|---|
| `SYWrapper.deposit(from=Bob, amount=300 STA)` | [6a6c1cda…](https://stellar.expert/explorer/testnet/tx/6a6c1cda02b2fdb150fc6868ba9e250c3fdd41520c558783b8d6ffb1e1885ad0) | 3,000,000,000 SY shares |
| `PrincipalManager.mint(from=Bob, sy_shares=3,000,000,000)` | [f7eb96df…](https://stellar.expert/explorer/testnet/tx/f7eb96dff11049b0b16d0cd514fafd86ca21e69ba683ec11005cd65a8bb20ad1) | 300 PT + 300 YT minted |
| Issuer clears Bob's trustline authorization on the classic STA asset (a regulator-style AML/compliance flag — the exact mechanism `underlying_SAC.authorized()` reads live) | [6d78b157…](https://stellar.expert/explorer/testnet/tx/6d78b157f535d84f76b95938210db0ca26decdc31b42bbb96e5f6077acb75f90) | `authorized(Bob)` now reads `false` |
| `RecoveryEscrow.seize_pt(caller=issuer, account=Bob, amount=300)` | [669fa812…](https://stellar.expert/explorer/testnet/tx/669fa812cd038a34e394d23e7857f296e68b243c2ca166b2ae39368d4675a913) | Bob's 300 PT moved to the escrow; reverts unless the caller is the SAC's real, live admin *and* the target is already deauthorized |
| `RecoveryEscrow.seize_yt(caller=issuer, account=Bob, amount=300)` | [474042a5…](https://stellar.expert/explorer/testnet/tx/474042a5fbb7d265b78471b4a21c77882bbfa20bb9d007ff50e7de2c4bbda629) | Bob's 300 YT moved to the escrow |
| `RecoveryEscrow.finalize_pt(caller=issuer, pt_amount=300)` — post-maturity | [fb62bc54…](https://stellar.expert/explorer/testnet/tx/fb62bc549a4e2f3648f7d0d5e12924ad320dfd33c9a829cffc65fdf4f2f04a2f) | Escrow's seized PT redeemed through the real `PrincipalManager`: 272.7272727 STA released to the escrow |
| `RecoveryEscrow.finalize_yt(caller=issuer, yt_amount=300)` | [04293656…](https://stellar.expert/explorer/testnet/tx/04293656f604d50802ec3ed416d25d5231f7d57c2e991768da8caa705fa7aed6) | Escrow's seized YT redeemed the same way: 5.4545725 STA released to the escrow |

The escrow ends the run holding **278.1818452 STA** — real, spendable underlying recovered from a single flagged account — ready for the issuer's native SAC `clawback`, without a native clawback ever touching Bob's own wallet (his un-deposited 700 STA was never involved) and *without pausing or otherwise affecting Alice's market*, which redeemed normally in parallel. This is the property no plain PT/YT-splitting protocol has: a regulator-style action against one holder's position is fully containable to that holder, with the rest of the market provably unaffected — verified here by Alice's redemption succeeding for its full, correct amount in the same deployment where Bob's position was independently seized.

### What This Deployment Proves

| Claim | Evidence |
|---|---|
| All eight contracts deploy and wire together on a real network, not only in the in-process test harness | Every address above resolves on stellar.expert; `PrincipalManager.initialize`'s topology validation (M-01) accepted the real addresses, meaning they do share one underlying/permissioning/maturity/oracle |
| Continuous yield accrual pays real underlying without burning the position | Alice's `claim_yield` transaction shows an actual STA transfer, and her PT/YT balances are unchanged immediately after |
| YT yield already claimed doesn't get paid again at redemption | Alice's redemption's YT leg (9.0909542 STA) reflects only the 1.08→1.10 move *after* her claim, not the full 1.00→1.10 move |
| Compliance recovery reaches only the flagged position | Bob's 700 STA of un-deposited wallet balance was never touched; only his 300-STA PT/YT position was seized and unwound |
| Compliance recovery doesn't require pausing the market | Alice's mint, claim, and redemption all happened in the same deployment as Bob's deauthorization and seizure, with no `set_paused` call anywhere in this run |
| Mint is correctly blocked once a market matures | The first PT/YT/PrincipalManager deployment's `mint` call reverted `AlreadyMature` in real conditions, not just in a test with a manipulated ledger clock |

See [TESTNET_DEPLOYMENT_EVIDENCE.md](TESTNET_DEPLOYMENT_EVIDENCE.md) for the complete, unabridged transaction log — every setup, wiring, and demo call in this deployment, including the superseded first attempt.

---

## Historical Testnet Deployment (superseded)

The deployment below is a historical record from an earlier development stage and is superseded by the full-stack deployment documented above. It predates `PTToken`, `YTToken`, `RecoveryEscrow`, and every compliance fix described elsewhere in this document; it is kept for reference only.

All five originally-implemented contracts have been deployed and initialised on **Stellar Testnet** (June 2026). The deployment demonstrates the infrastructure and tokenization layer of the protocol executing real on-chain transactions.

**This deployment predates `PTToken`, `YTToken`, `RecoveryEscrow`, the wiring that lets `PrincipalManager` actually call them, and every compliance fix described elsewhere in this document — including the original Permissioning integration, and the later authorization-inheritance / market-creation-gating / seize redesign.** Specifically: the deployed `SYWrapper` was initialized without a `permissioning` parameter (that argument didn't exist yet) and with no `underlying`-admin gate on `initialize` and no concept of `underlying_SAC.authorized()` at all, so its deposit/withdraw calls on testnet are not gated the way the current source code requires; and the deployed `PrincipalManager`'s `redeem()` predates both the permissioning check and the SAC-authorization check added since. It also predates `RiskControl.check_deposit`'s consumer-registration requirement (TX-11 below calls the old, unrestricted signature) and `YTToken`'s multiplicative yield index (a different bug entirely, fixed after this deployment). The deployed WASM reflects the source as it existed at deployment time — it does not update itself when the repository changes. Treat the addresses and transactions below as a historical record of that earlier version, not as a live demonstration of the current contract set. A redeployment reflecting the current source, including `PTToken`, `YTToken`, and `RecoveryEscrow`, has not yet been done.

### Deployed Contract Addresses

| Contract | Address | Explorer |
|---|---|---|
| OracleAdapter | `CDJSHBEULGIFN6PS7VDEWBTWWDBFPLMU75K2YOCJVLDQS5YKQROG36NL` | [view](https://stellar.expert/explorer/testnet/contract/CDJSHBEULGIFN6PS7VDEWBTWWDBFPLMU75K2YOCJVLDQS5YKQROG36NL) |
| Permissioning | `CBLSJAM7M32NDMRMEWOADEONC563DNZE2Y2JDVGCDIQ7ZJ53HZPH2GA6` | [view](https://stellar.expert/explorer/testnet/contract/CBLSJAM7M32NDMRMEWOADEONC563DNZE2Y2JDVGCDIQ7ZJ53HZPH2GA6) |
| RiskControl | `CCBDWAHYF5MBHOR4LQJ7XQQQQJFTYMMHXGQC2GBBRV3HTRBU7UASOBJY` | [view](https://stellar.expert/explorer/testnet/contract/CCBDWAHYF5MBHOR4LQJ7XQQQQJFTYMMHXGQC2GBBRV3HTRBU7UASOBJY) |
| Mock USDY (SAC) | `CAS53AG5G3XHKHPGJQRYEB2SEAYAHZNRFZZ57WKMVBC54RZIAMULNHIL` | [view](https://stellar.expert/explorer/testnet/contract/CAS53AG5G3XHKHPGJQRYEB2SEAYAHZNRFZZ57WKMVBC54RZIAMULNHIL) |
| SYWrapper | `CC25AC7YDW32PSC4UNAT33LXG4E6IR3I3HRWHUGJSFOS5OTL7MZRZMLO` | [view](https://stellar.expert/explorer/testnet/contract/CC25AC7YDW32PSC4UNAT33LXG4E6IR3I3HRWHUGJSFOS5OTL7MZRZMLO) |
| PrincipalManager | `CCWPPNCPJMEHBJ2P4SKHZMW3JFN3ACGQTJHJYSN5NPHKZC4ZD2CVUVDH` | [view](https://stellar.expert/explorer/testnet/contract/CCWPPNCPJMEHBJ2P4SKHZMW3JFN3ACGQTJHJYSN5NPHKZC4ZD2CVUVDH) |

**Admin / Deployer:** `GB2HC2NLXR7LHKXGS2IZL4F5LZVQVKRBKCWONQQW4WIYUXDILHORWQPZ`  
**Market maturity:** 1789135669 (11 September 2026, 90-day market)  
**Mock USDY:** a Stellar Asset Contract issued by the admin address, used in place of the real Ondo USDY token for testnet demonstration.

> All amounts in the contracts use `SCALE = 10_000_000` (10^7). An amount of `1_000_000_000` represents 100 tokens; a rate of `10_300_000` represents 1.03 USDC per underlying.

---

### Phase A — Contract Deployment

Each contract is deployed in two Soroban transactions: one to upload the WASM binary to the ledger (pay for code storage), and one to instantiate the contract from that WASM hash.

#### OracleAdapter
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [ef4c396…](https://stellar.expert/explorer/testnet/tx/ef4c396512da4b0fbcf59c610d5cc8015fc2f6063d9b679ab520b0fe8881de07) | Uploads `principal_oracle_adapter.wasm` (15 KB) |
| Deploy contract | [994b94a…](https://stellar.expert/explorer/testnet/tx/994b94aa7272719bedcd70b8faa74d58e50cfe1d588db35422fd9c0d6355b799) | Instantiates contract at `CDJSHE…` |

#### Permissioning
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [5f51a46…](https://stellar.expert/explorer/testnet/tx/5f51a4681a64d45aa3b9b4760b1344655657437251793d770c0ffb16edb69450) | Uploads `principal_permissioning.wasm` (12 KB) |
| Deploy contract | [c2dbefb…](https://stellar.expert/explorer/testnet/tx/c2dbefb03f29abf026fb523b82911837c6e8d11dcdb24b8fe7f950e7d4a6b94d) | Instantiates contract at `CBLSJA…` |

#### RiskControl
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [27b117e…](https://stellar.expert/explorer/testnet/tx/27b117eb7a424d133e38747bcf721132a8d3a36e363f43bcbf0b27d8b3d7b617) | Uploads `principal_risk_control.wasm` (18 KB) |
| Deploy contract | [559c16a…](https://stellar.expert/explorer/testnet/tx/559c16aba8e228d577168aa553408d21a6dca963d204ca7ebdc8482f4329ac1f) | Instantiates contract at `CCBDWA…` |

#### Mock USDY (SAC)
| Step | Transaction | Description |
|---|---|---|
| Deploy SAC | [e47e623…](https://stellar.expert/explorer/testnet/tx/e47e62308e515af41589068fb62b3664b02c52c8b715ff24c4c97bec22026ccc) | Wraps native Stellar asset `USDY:admin` as a Soroban token contract (SAC) |

#### SYWrapper
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [d2ed531…](https://stellar.expert/explorer/testnet/tx/d2ed531f3cdb80d831b58b1126a206f348c58a5c5e88efe835707a4f0e2f4c1b) | Uploads `principal_sy_wrapper.wasm` (17 KB) |
| Deploy contract | [5e6960b…](https://stellar.expert/explorer/testnet/tx/5e6960b5671cb702d2f37d33838fce791d4589c8874398116754132c13ccf469) | Instantiates contract at `CC25AC…` |

#### PrincipalManager
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [2514f9d…](https://stellar.expert/explorer/testnet/tx/2514f9d602d5a10449c5223b02dd984c642c5b2fc3b94ecb8b82c23e70612d61) | Uploads `principal_manager.wasm` (28 KB) |
| Deploy contract | [b79d485…](https://stellar.expert/explorer/testnet/tx/b79d485cbfe56b4d88fcbdd0a4bce73469af7f408ba174e4a250dbb8f112d0bd) | Instantiates contract at `CCWPPN…` |

---

### Phase B — Initialization

After deployment every contract is one-time-initialized to set the admin and register its dependencies.
Initialization is a separate transaction from deployment so that each contract's address is known before the next contract is configured.

**TX-01 · OracleAdapter.initialize**
```
stellar contract invoke --id CDJSHE... -- initialize --admin GB2HC2...
```
Transaction: [dd172cc…](https://stellar.expert/explorer/testnet/tx/dd172cc15d366947ea275fcde8bfbce2bc8716deb6465a3fc8c2e7fee4d2d0f2)

Sets the admin key that is authorized to update the USDY/USDC reference rate. After this call the oracle is live but has no price — any `get_reference_value` call would revert until a price is submitted.

---

**TX-02 · OracleAdapter.set_reference_value**
```
stellar contract invoke --id CDJSHE... \
  -- set_reference_value --caller GB2HC2... --value 10300000 --timestamp <unix>
```
Transaction: [6354d3f…](https://stellar.expert/explorer/testnet/tx/6354d3f93ac98d3844ea929ed07b21c1e59189ff39cb7c3658f0a153802f48bc)

Submits the first USDY/USDC reference rate: **1.03 USDC per USDY** (`value = 10_300_000`, scaled by 10^7). The monotonic timestamp guard ensures no relay can submit an older rate and replay a stale price. After this transaction `is_fresh(3600)` returns `true`.

---

**TX-03 · Permissioning.initialize**
```
stellar contract invoke --id CBLSJA... -- initialize --admin GB2HC2...
```
Transaction: [cc63716…](https://stellar.expert/explorer/testnet/tx/cc63716a67fb1a1f87c0bede8559d4483fb0634814bdd5d968920256eac06e45)

Initializes the eligibility registry. No accounts are allowed yet — every `is_allowed` call returns `false` until the admin explicitly grants access.

---

**TX-04 · RiskControl.initialize**
```
stellar contract invoke --id CCBDWA... -- initialize --admin GB2HC2... --cb-limit 0
```
Transaction: [2fe9fcb…](https://stellar.expert/explorer/testnet/tx/2fe9fcb9ca4e6134c6ab2b9ab724562d835403a3c1a14fda136808275aa83825)

Initializes the global pause with `cb_limit = 0` (circuit breaker disabled). Protocol is unpaused; any deposit volume is accepted. The circuit breaker can be enabled later with `set_cb_limit`.

---

**TX-05 · SYWrapper.initialize**
```
stellar contract invoke --id CC25AC... \
  -- initialize --admin GB2HC2... --underlying CAS53A...
```
Transaction: [6616f7a…](https://stellar.expert/explorer/testnet/tx/6616f7a3e1033f1036575babee7b6de9b090d3082594191d24bf2409fa7e91b6)

Registers the mock USDY SAC (`CAS53A…`) as the underlying asset. From this point SYWrapper can accept USDY deposits. The initial exchange rate is 1:1 (`10_000_000`).

---

**TX-06 · Permissioning.grant_account**
```
stellar contract invoke --id CBLSJA... \
  -- grant_account --caller GB2HC2... --account GB2HC2...
```
Transaction: [67a31da…](https://stellar.expert/explorer/testnet/tx/67a31da87754a2acf94ef383dd4a4cc78aade6e69aca100387d61f403ec6e809)

Adds the admin address to the eligibility allow-list. After this call `is_allowed(GB2HC2...)` returns `true`. Any account not in this list will be rejected by `PrincipalManager.mint()` with `PermissionDenied`.

---

**TX-07 · PrincipalManager.initialize**
```
stellar contract invoke --id CCWPPN... \
  -- initialize \
     --admin GB2HC2... \
     --sy-wrapper CC25AC... \
     --oracle CDJSHE... \
     --permissioning CBLSJA... \
     --maturity 1789135669
```
Transaction: [c9ee150…](https://stellar.expert/explorer/testnet/tx/c9ee1506c5536665cbcd3787cc4d5494d8b474c62c6e752a2fdd252eb7b0d0af)

Ties all infrastructure contracts together into one market. The maturity timestamp **1789135669** corresponds to **11 September 2026** — a 90-day market from deployment date. Before this timestamp `mint()` is open and `redeem()` reverts; after it the positions reverse.

---

### Phase C — Protocol Transactions

With all contracts initialized, the core yield-tokenization flow is executed on-chain.

**TX-08 · SYWrapper.deposit — wrap 100 USDY into SY shares**
```
stellar contract invoke --id CC25AC... \
  -- deposit --from GB2HC2... --amount 1000000000
```
Transaction: [045d054…](https://stellar.expert/explorer/testnet/tx/045d054704adc3b43acacd4992c5eb5012b9f35e82302633c9a16604e8e842c0)

The user transfers **100 USDY** (`1_000_000_000` in 10^7 units) to the SYWrapper. The contract:
1. Calls `token::transfer(user → SYWrapper, 100 USDY)` — an actual on-chain SEP-41 token transfer.
2. Computes shares at the current exchange rate (1:1 at inception → 100 shares).
3. Credits `1_000_000_000` SY-USDY shares to the user's persistent storage slot.

**On-chain state after TX-08:**
```
total_underlying  = 1_000_000_000   (100 USDY held by SYWrapper)
total_shares      = 1_000_000_000   (100 SY-USDY issued)
exchange_rate     = 10_000_000      (1.0 — 1 USDY per SY share)
deployer balance  = 1_000_000_000   (100 SY-USDY shares)
```

---

**TX-09 · PrincipalManager.mint — split 50 SY-USDY into PT + YT**
```
stellar contract invoke --id CCWPPN... \
  -- mint --from GB2HC2... --sy-shares 500000000
```
Transaction: [289ceb6…](https://stellar.expert/explorer/testnet/tx/289ceb63d66a21396673d12ef6b89b4ac38798229832c9c871ee9c43b1d80a69)

The user submits **50 SY-USDY shares** (`500_000_000`) for tokenization. The contract:
1. Calls `OracleAdapter.get_reference_value()` → receives `10_300_000` (1.03 USDC/USDY).
2. Verifies the user is on the Permissioning allow-list via `PermClient.is_allowed()`.
3. Stores `initial_rate = 10_300_000` for this user (used at YT settlement).
4. Computes notional: `500_000_000 × 10_300_000 / 10_000_000 = 515_000_000` (51.5 USDC).
5. Credits `515_000_000` PT and `515_000_000` YT to the user's persistent storage.

**On-chain state after TX-09:**
```
PT balance (deployer) = 515_000_000   (51.5 USDC notional)
YT balance (deployer) = 515_000_000   (51.5 USDC notional)
total_PT              = 515_000_000
total_YT              = 515_000_000
is_mature             = false          (matures 11 Sep 2026)
```

The notional is 51.5 USDC, not 50: the oracle rate of 1.03 USDC/USDY means each USDY share is worth slightly more than 1 USDC, so 50 USDY × 1.03 = 51.5 USDC of PT+YT.

---

**TX-10 · RiskControl.set_cb_limit — enable 24h deposit circuit breaker**
```
stellar contract invoke --id CCBDWA... \
  -- set_cb_limit --caller GB2HC2... --new-limit 500000000
```
Transaction: [3ccb389…](https://stellar.expert/explorer/testnet/tx/3ccb3895b0dde4177255eb6ac0ed37ac5cfb50272081ad8c063097d4c98141b9)

Sets the rolling 24-hour deposit limit to **50 USDY** (`500_000_000`). Any call to `check_deposit` that would push cumulative volume over this limit within a 24-hour window reverts with `CircuitBreakerTripped`. The window resets automatically after `CB_WINDOW_SECS = 86_400` seconds.

---

**TX-11 · RiskControl.check_deposit — record a 10 USDY deposit against the circuit breaker**
```
stellar contract invoke --id CCBDWA... \
  -- check_deposit --amount 100000000
```
Transaction: [ab1d8bc…](https://stellar.expert/explorer/testnet/tx/ab1d8bced4dcc94b107505b8944c95ed83792c3e2052759d8b47054139d7b69a)

Records a **10 USDY** deposit event (`100_000_000`) against the circuit breaker window. After this call `get_cb_volume()` returns `100_000_000`. A subsequent call with more than `400_000_000` (40 USDY) within the same 24-hour window would trip the breaker and revert.

---

### What the Testnet Deployment Proves

This table describes what the historical deployment above proved about the contract versions live at the time — it predates the compliance-gap fixes, PT/YT tokens, and the authorization-inheritance/RecoveryEscrow redesign (see the caveat at the top of this section), so treat it as evidence for the underlying mechanics, not for the current source as a whole.

| Claim | Evidence |
|---|---|
| Soroban contracts compile and deploy | 5 contracts live on testnet with verified WASM hashes |
| Oracle price feed works | TX-02 sets 1.03 rate; `is_fresh` returns `true` |
| Permissioning enforces eligibility | TX-06 grants account; PrincipalManager checks it on every mint |
| SYWrapper wraps yield-bearing tokens | TX-08 executes a real SEP-41 `transfer` on-chain |
| PT+YT minting uses oracle rate | TX-09 produces 51.5 USDC notional from 50 USDY × 1.03 |
| Per-user `initial_rate` is stored | Visible in TX-09 ledger state; used at maturity settlement |
| Circuit breaker tracks volume | TX-11 increments `cb_volume`; limit enforced in-window |
| All contracts are interoperable | TX-09 makes live cross-contract calls to OracleAdapter and Permissioning |

---

## Known Limitations

The following are the current, honest scope boundaries — each is either genuinely outstanding work or a nuance worth being precise about:

1. **No AMM.** `MarketPool` is not implemented. There is no on-chain market for PT or YT trading.

2. **No Router.** Users interact with each contract individually. Single-transaction flows (wrap + mint, swap, recombine) require a `Router` contract that doesn't exist yet.

3. ~~Standalone `claim_yield()` doesn't dispatch a real transfer on its own.~~ **Fixed.** `YTToken.claim_yield` is now minter-gated (only `PrincipalManager` can call it, the same way `mint`/`burn` already were), and `PrincipalManager.claim_yield(from)` is the sole path that reaches it: it brings the index current, settles through `YTToken`, and pays the result out via `SYWrapper.withdraw` in the same call — so a YT holder can now claim accrued yield in underlying *without* redeeming (burning) the position or waiting for maturity, and a settled claim can never go unpaid. `redeem()` and `RecoveryEscrow.finalize_yt` are unaffected, since both already went through `PrincipalManager`.

4. **No recombination.** PT + YT → SY recombination before maturity is not implemented.

5. **Single oracle submitter.** A single admin-controlled oracle is implemented. Multi-source aggregation and quorum oracle are not.

6. **`RiskControl` is not cross-contract-wired.** `RiskControl.check_deposit` is invoked directly by a registered consumer (e.g. a test harness or admin script standing in for `SYWrapper`/`PrincipalManager` today) rather than being called automatically by those contracts. The risk control logic and interface are fully implemented and tested in isolation, including the consumer-registration gate (`add_consumer`/`remove_consumer`) that prevents an arbitrary caller from griefing the circuit breaker directly; wiring `check_deposit` into the actual deposit and mint call paths, and registering `SYWrapper`/`PrincipalManager` as consumers at deployment time, remains outstanding.
