# API Reference

What every public function of every Principal contract does, what it needs, and what can go wrong.
Written so an integrator can build on the protocol without reading the code.

**Conventions**

* Amounts are `i128` raw integers at `SCALE = 10_000_000` (1e7). Rates are "USDC per underlying ×
  SCALE". Timestamps are ledger seconds; `*_ledger` values are ledger sequence numbers.
* **Auth** is what the transaction must be signed by (`require_auth`). "admin" means the address stored
  as that contract's admin; "issuer authority" means the underlying's *live* authority (`SAC.admin()`, or
  the SEP-57 operator role — see [COMPLIANCE_ARCHITECTURE.md](COMPLIANCE_ARCHITECTURE.md)).
* **"Compliant"** means: the account is authorized on the underlying **and** passes `Permissioning`
  (`is_allowed`; PT/YT additionally `is_allowed_for_asset` for the token's own address). Wherever a
  function says *both sides*, the sender and the recipient must each be compliant.
* **Errors** are contract errors (`Error(Contract, #n)`). A cross-contract call that reverts surfaces
  the *callee's* code, so `#5` from `Router.swap_sy_for_pt` may be the pool's, the token's or the
  wrapper's; the tables list the code each contract itself raises.
* A `try_*` variant of every function exists on the generated clients and returns the error instead of
  trapping.
* Most state-changing functions emit an event (appendix A); `initialize`, `check_deposit`, `bump` and the Router flows do not.

Contents: [OracleAdapter](#oracleadapter) · [Permissioning](#permissioning) ·
[RiskControl](#riskcontrol) · [MarketConfig](#marketconfig) · [SYWrapper](#sywrapper) ·
[PTToken](#pttoken) · [YTToken](#yttoken) · [PrincipalManager](#principalmanager) ·
[MarketPool](#marketpool) · [Router](#router) · [RecoveryEscrow](#recoveryescrow) ·
[Appendix A: events](#appendix-a-events) · [Appendix B: shared types](#appendix-b-types)

---

## OracleAdapter

The reference-value feed: USDC value of one underlying token, `SCALE`d, with a timestamp. Single-writer,
**monotonic**: value and timestamp may never go backwards, because the settlement math assumes yield is
never negative. Extended (`docs/ORACLE_SAFETY_EXTENSION.md`) with a primary/optional-fallback source
pair, a deviation guard, and centralized staleness policy — a **breaking ABI change** from the
Tranche 1 version (`initialize` and `set_reference_value` both take new required arguments), so a
market using this version needs a freshly deployed `OracleAdapter` instance, not a retrofit of one
already live.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, max_stale_seconds, max_deviation_bps)` | – | One-time setup. `max_stale_seconds` and `max_deviation_bps` become this feed's own policy (see `is_available`); both must be nonzero and `max_deviation_bps ≤ 10_000` (100%). Sources are configured separately via `set_sources`. | `AlreadyInitialized`(1), `InvalidStaleness`(12), `InvalidDeviation`(11) |
| `set_sources(caller, primary_source, fallback_source)` | `caller` = admin | Configures the primary and optional fallback source IDs (each a deterministic `BytesN<32>`, e.g. `sha256("principal-oracle-source:<network>:<asset>:<provider>:<feed>")`). `fallback_source: None` removes any existing fallback. | `Unauthorized`(2) |
| `set_safety_params(caller, max_stale_seconds, max_deviation_bps)` | `caller` = admin | Updates the staleness/deviation policy without touching sources. Same validation as `initialize`. | `Unauthorized`(2), `InvalidStaleness`(12), `InvalidDeviation`(11) |
| `set_value_bounds(caller, min_value, max_value)` | `caller` = admin | Configures an optional sane absolute range checked against **every** accepted value, including the first-ever submission (otherwise exempt from the deviation guard, since there is nothing yet to deviate from). Unset by default — no bound until this is called once. Exists to catch a relay decimal/scale bug (e.g. an 18-decimal raw value where the feed expects 8) *before* it is ever stored: once such a value were accepted, every later correct submission would read as a `decrease` and be rejected forever, since nothing can move the stored value back down. Found during an audit review. Also rejects a range that would exclude the **currently stored** price, if one exists — a later audit review found that, without this, an admin re-tightening or miscalculating the bounds could brick the feed from the other direction, no relay bug needed. | `Unauthorized`(2), `InvalidValue`(3) if `min_value ≤ 0`, `max_value ≤ min_value`, or the range excludes the price already on record |
| `value_bounds()` → `Option<(i128, i128)>` | – | The configured `(min_value, max_value)`, or `None` if `set_value_bounds` has never been called. | – |
| `set_reference_value(caller, source, value, timestamp)` → `bool` | `caller` = admin | Publishes `value` at `timestamp` from `source`, which must be the configured primary or fallback. Malformed calls (non-positive value, future timestamp, unrecognized source) **panic**. A syntactically valid submission policy rejects (outside the configured value bounds, a value decrease, a non-newer timestamp, a >`max_deviation_bps` increase — including one that would overflow the deviation calculation itself, treated as "deviation far exceeds threshold" — or a fallback trying to move the value) **returns `false`** instead of panicking — `Price`/`Timestamp` stay untouched, and the attempt is recorded in `last_rejection()` plus a `ref_rej` event, because Soroban cannot both revert and persist a record in the same call. A caller must check the **return value**, not just transaction success, to know whether a submission landed — see `docs/ORACLE_SAFETY_EXTENSION.md` §3. | `Unauthorized`(2), `InvalidValue`(3), `TimestampInFuture`(7), `InvalidSource`(10), `FallbackNotConfigured`(13) |
| `get_reference_value()` → `i128` | – | Last published value. | `NotInitialized`(5) if none |
| `get_reference_timestamp()` → `u64` | – | Timestamp of the last value. | `NotInitialized`(5) |
| `is_fresh(max_stale_seconds)` → `bool` | – | `true` iff `ledger_time − stored_timestamp ≤ max_stale_seconds`, against a **caller-supplied** threshold. A stored timestamp *ahead* of the ledger clock is stale. | – |
| `is_available()` → `bool` | – | `is_fresh` against this feed's **own configured** `max_stale_seconds` — no threshold needed from the caller. This is what `MarketPool`/`PrincipalManager`/`YTToken` call instead of `is_fresh` as of this extension; each still raises its own pre-existing `OracleStale` error when this returns `false` (this protocol's name for what this extension calls "unavailable"). | – |
| `assert_available()` | – | Panics `OracleUnavailable`(8) unless `is_available()` is true. Not called by any consumer in this codebase today — infrastructure for a caller that wants a crisp revert in one hop rather than reading the boolean itself. | `OracleUnavailable`(8) |
| `get_primary_source()`/`get_fallback_source()`/`get_active_source()` → `Option<BytesN<32>>` | – | The configured sources, and which one the current `Price` last came from (`None` before the first accepted update). | – |
| `max_stale_seconds()` → `u64` / `max_deviation_bps()` → `u32` | – | The feed's current policy. | `NotInitialized`(5) |
| `last_rejection()` → `Option<OracleRejection>` | – | `{ value, timestamp, reason }` for the most recent policy-rejected submission (`reason` is one of the short symbols `decrease`, `stale_ts`, `deviate`, `fb_value`, `oob`), or `None` if nothing has ever been rejected. | – |
| `transfer_admin(current_admin, new_admin)` | current admin | Single-step admin handover. | `Unauthorized`(2) |
| `get_admin()` → `Address` | – | | `NotInitialized`(5) |

## Permissioning

An **optional** allow-list the market administrator may run. It can only *narrow* eligibility on top of
the underlying's own authorization; it never widens it.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin)` | – | One-time. | `AlreadyInitialized`(1) |
| `grant_account(caller, account)` / `revoke_account(...)` | admin | Sets the account-level flag. Entries live ~30 days of ledgers and are extended on write. | `Unauthorized`(2) |
| `grant_accounts(caller, accounts)` | admin | Batch grant in one transaction. | `Unauthorized`(2) |
| `is_allowed(account)` → `bool` | – | Account-level flag (default `false`). | – |
| `grant_asset(caller, account, asset)` / `revoke_asset(...)` | admin | Per-asset flag; `asset` is a PT/YT token address, so PT and YT can have different audiences. | `Unauthorized`(2) |
| `is_allowed_for_asset(account, asset)` → `bool` | – | | – |
| `transfer_admin`, `get_admin` | admin | As above. | `Unauthorized`(2), `NotInitialized`(3) |

## RiskControl

The protocol circuit breaker and pause switch. `SYWrapper.deposit` and `PrincipalManager.mint` call
`check_deposit` **inside the same transaction**, so an over-limit deposit or mint reverts by itself.

* **Windows are ledger-sequence based** (default `DEFAULT_WINDOW_LEDGERS` = 17 280 ≈ 24 h at ~5 s), so
  wall-clock manipulation cannot stretch them. A window starts at the first counted deposit after the
  previous one lapsed and resets `window_ledgers` ledgers later.
* **Two limits**, both checked before either is written (a trip leaves no partial state): the
  protocol-wide `cb_limit` across all assets, and an optional per-asset limit. `0` disables a limit.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, cb_limit)` | – | One-time; window = default. | `AlreadyInitialized`(1) |
| `check_deposit(caller, asset, amount)` | `caller` = a **registered consumer** | Counts `amount` of `asset` against both windows. Needs: registered consumer, not paused, `amount > 0`, protocol total ≤ `cb_limit`, asset total ≤ its limit. | `NotConsumer`(8), `Paused`(4), `ZeroAmount`(10), `CircuitBreakerTripped`(5), `AssetLimitTripped`(11) |
| `pause(caller)` | admin **or** a pauser | Global pause. | `NotPauser`(6) |
| `unpause(caller)` | admin only | Pausers cannot unpause (prevents pause/unpause cycling). | `Unauthorized`(2) |
| `is_paused()` | – | | – |
| `add_pauser` / `remove_pauser(caller, pauser)` | admin | | `Unauthorized`(2), `AlreadyPauser`(7) |
| `is_pauser(account)` | – | admin counts as a pauser. | – |
| `add_consumer` / `remove_consumer(caller, consumer)` | admin | Who may call `check_deposit`. Removing a consumer makes its deposits **fail closed**. | `Unauthorized`(2), `AlreadyConsumer`(9) |
| `is_consumer(account)` | – | | – |
| `set_cb_limit(caller, limit)` | admin | Protocol-wide limit; `0` disables. | `Unauthorized`(2), `InvalidLimit`(12) if negative |
| `set_asset_limit(caller, asset, limit)` | admin | Per-asset limit; `0` disables. | `Unauthorized`(2), `InvalidLimit`(12) |
| `set_window_ledgers(caller, ledgers)` | admin | Window length in ledgers (> 0). | `Unauthorized`(2), `InvalidWindow`(13) |
| `get_cb_limit`, `get_asset_limit(asset)`, `get_window_ledgers`, `get_window_start` | – | | – |
| `get_cb_volume()`, `get_asset_volume(asset)` | – | Volume in the *current* window (`0` once it lapsed). | – |
| `transfer_admin`, `get_admin` | admin | | `Unauthorized`(2), `NotInitialized`(3) |

## MarketConfig

Per-market configuration and fee-split rules. One instance per (underlying, maturity).

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, underlying, maturity, protocol_admin, treasury, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps, protocol_share_bps)` | `admin` = issuer authority | Creates the config. Fees must be within caps: tokenization ≤ 100 bps, YT ≤ 5 000 bps, swap tier ≤ 500 bps; share ≤ 10 000 bps. Detects the underlying's compliance kind. | `AlreadyInitialized`(1), `IssuerMismatch`(4), `FeeTooHigh`(5), `InvalidShare`(6) |
| `set_fees(caller, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps)` | issuer authority, **live** | Retunes the three fees (same caps). Takes effect immediately for later operations. | `Unauthorized`(3), `FeeTooHigh`(5) |
| `set_creator_payee(caller, payee)` | issuer authority | Repoints the creator's payout — **only for a SEP-57 underlying**, which has no `admin()` to read. | `Unauthorized`(3), `NotApplicable`(7) for a SAC |
| `set_protocol_share(caller, bps)` | `protocol_admin` | Principal's cut (initially 2 000 = 20 %). Deliberately *not* settable by the creator. | `Unauthorized`(3), `InvalidShare`(6) |
| `set_treasury(caller, treasury)`, `transfer_protocol_admin(caller, new_admin)` | `protocol_admin` | | `Unauthorized`(3) |
| `creator()` → `Address` | – | The underlying's **current** `admin()` for a SAC (live); the stored payee for an RWA. | `NotInitialized`(2) |
| `split(fee)` → `(protocol, creator)` | – | `protocol = fee × share / 10 000` (floored); `creator = fee − protocol`. Sums to `fee`. | `NotInitialized`(2) |
| `swap_fee_rate(seconds_to_maturity)` → `i128` | – | **Trading Fee = Fee Tier × Days to Maturity / 365**, at `FEE_SCALE = 1e12`, capped at 10 %. | `NotInitialized`(2) |
| `underlying`, `maturity`, `tokenization_fee_bps`, `yt_fee_bps`, `swap_fee_tier_bps`, `protocol_share_bps`, `treasury`, `protocol_admin` | – | Views. | `NotInitialized`(2) |

## SYWrapper

Wraps the underlying into SY shares, one pool of underlying per market. `exchange_rate = total_underlying
× SCALE / total_shares` (1.0 at inception and in practice for an appreciating asset).

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, underlying, permissioning)` | `admin` = issuer authority | One-time market-creation gate. | `AlreadyInitialized`(1), `IssuerMismatch`(12) |
| `deposit(from, amount, min_shares_out)` → shares | `from` | Pulls `amount` of underlying, mints shares at the current rate. Needs: not paused, `from` compliant, `amount > 0`, `shares ≥ min_shares_out`, `from`'s net deposits + `amount` ≤ the per-address cap (if set), and — once `set_risk_control` was called — `RiskControl.check_deposit` passing. Checks-effects-interactions. | `Paused`(6), `NotAuthorizedOnSac`(9), `PermissionDenied`(8), `ZeroAmount`(4), `SlippageExceeded`(13), `DepositCapExceeded`(14), `ArithmeticOverflow`(7); RiskControl's `Paused`(4) / `CircuitBreakerTripped`(5) / `AssetLimitTripped`(11) / `NotConsumer`(8) |
| `withdraw(from, shares, to, min_underlying_out)` → underlying | `from` | Burns shares, sends underlying to `to`. Needs: not paused (the recovery escrow is exempt), **both** `from` and `to` compliant (a flagged account cannot cash out ahead of a seizure), `shares > 0`, balance, `out ≥ min_underlying_out`. Credits the amount back against `from`'s cap. | `Paused`(6), `NotAuthorizedOnSac`(9), `PermissionDenied`(8), `ZeroAmount`(4), `InsufficientShares`(5), `SlippageExceeded`(13) |
| `transfer(from, to, amount)` → amount | `from` | Moves SY shares (no external call). Both sides compliant; not paused unless the escrow is a party. | as above |
| `withdraw_all(from, to, min_underlying_out)` → underlying | `from` | Like `withdraw`, but burns `from`'s **entire** SY balance instead of taking `shares` as an argument — reads the balance at execution time so a caller composing this after a curve-derived amount (e.g. a swap's payout) has nothing drift-prone left to authorize. No longer used by any Router flow (see `withdraw_gained`), but kept as a standalone primitive. See `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §2's third gotcha. | as `withdraw` |
| `withdraw_gained(from, baseline, to, min_underlying_out)` → underlying | `from` | Like `withdraw_all`, but withdraws only the SY gained *since* `baseline` (a balance the caller read, via `balance_of`, right before whatever produced the gain) rather than the entire balance — so pre-existing SY above `baseline` is left untouched. Used by `Router.swap_pt_for_underlying`/`swap_yt_for_underlying`/`swap_underlying_for_yt`/`flash_mint_yt_from_underlying` — a dApp-integration review found the `withdraw_all`-based versions failed routinely for any caller who already held unrelated SY. Tolerates a zero (or negative) gain gracefully, returning `0`, **unless** `min_underlying_out > 0`, in which case it still reverts `SlippageExceeded` — found needed after a zero/exact-cost flash-mint budget reverted `ZeroAmount` outright. | as `withdraw`, plus `SlippageExceeded`(13) on a zero gain with a nonzero `min_underlying_out` |
| `exchange_rate()`, `total_underlying()`, `total_shares()`, `balance_of(a)` | – | Views. | – |
| `net_deposited(account)`, `deposit_cap()` | – | Underlying an address has deposited net of withdrawals (floored at 0), and the cap (`0` = none). | – |
| `set_deposit_cap(admin, cap)` | admin | Per-address cap in underlying units; `0` disables. Does not claw back existing balances. | `Unauthorized`(2), `InvalidCap`(15) |
| `set_risk_control(admin, risk_control)` | admin | Wires the breaker. The wrapper must also be a registered consumer there or every deposit reverts. | `Unauthorized`(2) |
| `set_paused(caller, paused)`, `transfer_admin`, `get_admin`, `is_paused`, `risk_control`, `underlying_address`, `permissioning_address` | admin / – | | `Unauthorized`(2), `NotInitialized`(3) |
| `set_recovery_escrow(admin, escrow)` | admin | **One-time** wiring of the only address allowed to `seize`. | `Unauthorized`(2), `RecoveryEscrowAlreadySet`(10) |
| `seize(caller, account, shares)` → shares | `caller` = the configured escrow | Forced transfer of SY from `account` to the escrow, without `account`'s consent; works while paused. | `NotRecoveryEscrow`(11), `ZeroAmount`(4), `InsufficientShares`(5) |
| `recovery_escrow()` | – | | `NotRecoveryEscrow`(11) if unset |

## PTToken

Standalone SEP-41 Principal Token. Mintable/burnable **only** by the registered minter
(`PrincipalManager`); there is no holder-callable burn. Every transfer path checks compliance on both
sides.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, permissioning, underlying, maturity, name, symbol, decimals)` | `admin` = issuer authority | One-time. No minter yet (two-phase init). | `AlreadyInitialized`(1), `IssuerMismatch`(11) |
| `set_minter(admin, minter)` | admin | **One-time.** Resolves the PT ⇄ manager circular dependency. | `Unauthorized`(2), `MinterAlreadySet`(8) |
| `set_recovery_escrow(admin, escrow)` | admin | One-time. | `Unauthorized`(2), `RecoveryEscrowAlreadySet`(12) |
| `transfer(from, to, amount)` | `from` | SEP-41 transfer; both sides compliant. | `ZeroAmount`(4), `NotAuthorizedOnSac`(10), `PermissionDenied`(7), `InsufficientBalance`(5) |
| `transfer_from(spender, from, to, amount)` | `spender` | Spends allowance; unexpired (`expiration_ledger ≥` current sequence) and sufficient. | `InsufficientAllowance`(6) + the above |
| `approve(from, spender, amount, expiration_ledger)` | `from` | `amount ≥ 0` (0 revokes). | `ZeroAmount`(4) if negative |
| `allowance`, `balance`, `decimals`, `name`, `symbol`, `total_supply`, `maturity`, `minter`, `get_admin`, `recovery_escrow`, `underlying_address`, `permissioning_address` | – | Views. | `MinterNotSet`(9), `NotRecoveryEscrow`(13), `NotInitialized`(3) |
| `mint(to, amount)` | the **minter** | Creates PT to a compliant `to`. | `MinterNotSet`(9), `ZeroAmount`(4), `NotAuthorizedOnSac`(10), `PermissionDenied`(7) |
| `burn(from, amount)` | the minter | Destroys PT; no compliance check (only removes value). | `MinterNotSet`(9), `ZeroAmount`(4), `InsufficientBalance`(5) |
| `seize(caller, account, amount)` | the configured escrow | Forced transfer to the escrow. | `NotRecoveryEscrow`(13), `ZeroAmount`(4), `InsufficientBalance`(5) |

## YTToken

As `PTToken`, plus continuous yield accrual (index `G = INDEX_SCALE × SCALE / rate`, see
[YIELD_MATH_AND_FEES.md](YIELD_MATH_AND_FEES.md)). `initialize` reads its genesis rate live from the
oracle (reverting if stale). Additional/different functions:

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, permissioning, underlying, oracle, maturity, name, symbol, decimals)` | `admin` = issuer authority | One-time; baselines the index at the live oracle rate. | `AlreadyInitialized`(1), `IssuerMismatch`(12), `OracleStale`(10) |
| `update_yield_index()` | **anyone** | Advances the index to the current oracle rate (only if it rose). Every balance change (`mint`, `burn`, `transfer`, `transfer_from`, `seize`) also syncs the index first, so yield is never attributed against a stale index; after maturity an unfrozen index needs a fresh oracle. The first call at/after maturity advances it a last time and **freezes** it; later calls are no-ops. Needs a fresh oracle unless frozen. | `OracleStale`(10) |
| `claim_yield(caller, from)` → i128 | `caller` = the **minter** | Settles `from` and returns/zeroes their pending yield (underlying units). Only `PrincipalManager` can reach it, and it pays in the same call — a holder cannot burn a claim without being paid. | `Unauthorized`(2), `MinterNotSet`(9) |
| `pending_claim(account)`, `accrued_yield_index()`, `last_claimed_index(account)`, `is_frozen()`, `last_oracle_rate()`, `oracle_address()` | – | Views. `last_oracle_rate` is the market's settlement rate once frozen. | – |
| all other functions | | identical to `PTToken`, except that `mint`/`burn`/`transfer`/`transfer_from`/`seize` also settle each affected account's pending yield *before* the balance moves | error codes differ: `OracleStale`=10, `NotAuthorizedOnSac`=11, `IssuerMismatch`=12, `RecoveryEscrowAlreadySet`=13, `NotRecoveryEscrow`=14, `ArithmeticOverflow`=15 (the pending-yield calculation, on an overflowing balance) |

## PrincipalManager

The tokenization engine. Takes custody of SY, mints/burns real PT and YT, charges the market fees,
freezes the settlement rate and redeems.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, sy_wrapper, pt_token, yt_token, oracle, permissioning, underlying, maturity, market_config)` | `admin` = issuer authority | One-time. **Topology-checked**: SY/PT/YT/config must share the underlying; SY/PT/YT the permissioning; PT/YT/config the maturity; YT the oracle. | `AlreadyInitialized`(1), `IssuerMismatch`(12), `TopologyMismatch`(13) |
| `mint(from, sy_shares)` → `MintResult{pt_minted, yt_minted, fee_shares}` | `from` | Splits SY into equal PT + YT. Needs: not paused, before maturity, fresh oracle, `sy_shares > 0`, `from` compliant, `RiskControl` passing (if wired; counts the underlying value), and a non-zero notional after the tokenization fee. Takes custody of *all* `sy_shares` (fee included), brings the YT index current first, mints. | `Paused`(9), `AlreadyMature`(6), `OracleStale`(7), `ZeroAmount`(4), `NotAuthorizedOnSac`(11), `PermissionDenied`(10) + RiskControl's |
| `recombine(from, amount)` → shares | `from` | Before maturity: burns `amount` PT and `amount` YT and returns `amount × SCALE / rate` SY at the current rate. Accrued YT yield stays claimable. | `Paused`(9), `AlreadyMature`(6), `OracleStale`(7), `ZeroAmount`(4), compliance errors, `InsufficientBalance` from the tokens |
| `settle_all()` → rate | **anyone** | At/after maturity: advances (and freezes) the YT index and records the frozen settlement rate; if the index is already frozen no oracle is needed. Idempotent. Not blocked by the pause. | `NotMature`(5), `OracleStale`(7) |
| `redeem(from, pt_amount, yt_amount)` → `RedeemResult{underlying_from_pt, underlying_from_yt}` | `from` | After maturity, any mix of PT and YT (either may be 0, not both). PT pays `pt × SCALE / settled_rate`; YT pays its remaining yield less the YT fee. Settles implicitly if nobody called `settle_all`. After settlement no fresh oracle is needed. | `Paused`(9), `NotMature`(5), `ZeroAmount`(4), `OracleStale`(7) (first redeem only), compliance errors |
| `claim_yield(from)` → underlying paid | `from` | Pays accrued YT yield (less the YT fee) without burning YT, before or after maturity. | `Paused`(9), `OracleStale`(7) (unless settled), compliance errors |
| `claim_protocol_fees()` / `claim_creator_fees()` → SY shares | **anyone** | Pays the accrued fee bucket, in SY, to `MarketConfig.treasury()` / `MarketConfig.creator()` (live issuer authority). Payee is fixed by configuration — calling cannot redirect. The payee must be compliant. | `NothingToClaim`(14) |
| `set_risk_control(admin, rc)` | admin | Wires the breaker (the manager must be a registered consumer). | `Unauthorized`(2) |
| `set_paused(caller, paused)`, `transfer_admin`, `get_admin` | admin | | `Unauthorized`(2) |
| `is_paused()` | – | Read-only counterpart to `set_paused`; before this existed a dApp could only infer the paused state indirectly, by attempting a call and catching `Error::Paused`. | – |
| views: `pt_balance`, `yt_balance`, `total_pt`, `total_yt`, `maturity`, `is_mature`, `settled_rate` (`Option`), `accrued_fees` (`(protocol, creator)`), `risk_control`, and the wiring addresses `underlying_address`, `sy_wrapper_address`, `pt_address`, `yt_address`, `oracle_address`, `permissioning_address`, `config_address` | – | | `NotInitialized`(3) |

## MarketPool

The PT/SY yield-curve AMM (see [AMM_DESIGN.md](AMM_DESIGN.md)). **Compliance:** every trade, liquidity
operation and LP transfer requires the caller (and any LP recipient) to be compliant; the pool's own
address must be authorized, Permissioning-granted, and per-asset granted for PT and YT.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin, principal_manager, time_stretch_years)` | `admin` = issuer authority | Reads underlying, SY, PT, YT, oracle, config, permissioning and maturity from the manager. `time_stretch_years` in 1..=20; remaining life must be < 75 % of it. | `AlreadyInitialized`(1), `IssuerMismatch`(4), `InvalidStretch`(20), `MaturityTooFar`(14) |
| `swap_sy_for_pt(from, to, sy_in, min_pt_out)` → PT out | `from` | Buy PT with SY. Needs: not paused, before maturity, fresh oracle, seeded pool, `from` compliant, `pt_out ≥ min_pt_out`. Fee (in SY, on the input) accrues to the fee buckets. | `Paused`(5), `Expired`(6), `OracleStale`(13), `InsufficientLiquidity`(9), `ZeroAmount`(7), `SlippageExceeded`(8), `NotAuthorizedOnSac`(12), `PermissionDenied`(11) |
| `swap_pt_for_sy(from, to, pt_in, min_sy_out)` → SY out (after fee) | `from` | Sell PT. Same gates; fee is taken from the SY output. | as above |
| `swap_yt_for_sy(from, to, yt_in, min_sy_out)` → SY out | `from` | **Flash-redeem:** takes `yt_in` YT, recombines it with `yt_in` PT from the pool's own reserve, keeps the curve price of that PT plus fee, pays `to` the rest. Reverts if the recombined SY does not cover the price. Atomic. | as above; `InsufficientLiquidity`(9) when YT is worth too little |
| `flash_mint_yt(from, to, sy_in, min_yt_out, max_net_cost)` → `(yt_out, net_cost)` | `from` | **True flash-mint:** funds the mint leg from the pool's *own* SY reserves, keeps the PT the mint produces, sends `to` only the YT. `from` authorizes transferring `max_net_cost` (a fixed literal, not the mid-execution-derived `net_cost` — see the function's own doc comment for why), and any unused portion is refunded. Verified live on Testnet minting against a notional larger than the caller's own SY balance. | as above; `InsufficientLiquidity`(9) |
| `add_liquidity(from, pt_desired, sy_desired, min_lp_out)` → `(pt_used, sy_used, lp)` | `from` | First deposit sets the opening price (SY value must not exceed PT) and locks 1 000 LP; later deposits are pro-rata (`ceil` on the amounts taken), surplus stays with the caller. | `InvalidInitialRatio`(15), `MinimumLiquidity`(21), `ZeroAmount`(7), `SlippageExceeded`(8), `Expired`(6), `Paused`(5) |
| `add_liquidity_single_sy(from, sy_in, min_lp_out)` → `(pt_used, sy_used, lp)` | `from` | Swaps just enough SY into PT (normal fee) to deposit both in the post-swap ratio. Leftover SY is refunded to `from`; leftover PT is **not** — it is folded back into the pool's own reserve instead, never transferred. A dApp-integration review found the PT refund reverted `PermissionDenied` for any caller lacking PTToken's per-asset grant (checked even for a single raw unit), which a caller using this "LP only" zap specifically to avoid holding PT has no reason to have requested — observed live as a roughly 1-in-12 failure rate. Needs a seeded pool. | as above, `InsufficientLiquidity`(9) on an empty pool |
| `quote_add_liquidity_single_sy(sy_in)` → `(pt_used, sy_used, lp)` | – | Previews `add_liquidity_single_sy`'s own return exactly (same `zap_split`/`calc_buy_pt` math, read-only). Found missing in a dApp-integration review. | `ZeroAmount`(7), `InsufficientLiquidity`(9) on an empty pool |
| `remove_liquidity(from, to, lp, min_pt_out, min_sy_out)` → `(pt, sy)` | `from` | Pro-rata exit, also after maturity, and without an oracle. The pool pause blocks it while the market is live but never after maturity. | `InsufficientLpBalance`(10), `ZeroAmount`(7), `SlippageExceeded`(8), `Paused`(5), compliance errors |
| `quote_lp_exit(lp_in)` → `(pt_out, sy_out, pt_sold_for_sy, swap_fee_shares)` | – | Previews `remove_liquidity`'s proportional split for `lp_in`, then selling the PT leg — correctly priced against the reserves *as they stand after* that removal, not before it (a first version got this wrong). Backs `Router.quote_lp_to_underlying`. Last two fields are `0` if `pt_out` rounds to `0`. | `ZeroAmount`(7), `InsufficientLiquidity`(9) on an empty pool |
| `swap_all_pt_for_sy(from, to, min_sy_out)` → SY out | `from` | Sells `from`'s **entire** PT balance, reading it at execution time instead of taking it as an argument — same reason `SYWrapper.withdraw_all` exists. No longer used by any Router flow (see `swap_gained_pt_for_sy`), but kept as a standalone primitive. Otherwise identical to `swap_pt_for_sy`. | as `swap_pt_for_sy` |
| `swap_gained_pt_for_sy(from, to, baseline, min_sy_out)` → SY out | `from` | Like `swap_all_pt_for_sy`, but sells only the PT gained *since* `baseline` — the PT equivalent of `SYWrapper.withdraw_gained`, for the same reason. Used by `Router.remove_liquidity_to_underlying`, so pre-existing, unrelated PT survives untouched. Tolerates a zero (or negative) gain gracefully, returning `0`, unless `min_sy_out > 0`, in which case it reverts `SlippageExceeded`. | as `swap_pt_for_sy`, plus `SlippageExceeded`(8) on a zero gain with a nonzero `min_sy_out` |
| `keep_yt_split(sy_shares)` | – | How much of `sy_shares` `Router.add_liquidity_keep_yt` should *mint* (not swap) so the minted PT and remaining SY land in the pool's raw reserve ratio. **Not** `zap_swap_amount`: that targets the curve-swap ratio (`add_liquidity_single_sy`'s zap), which an integration review found recommends the wrong split for minted PT and leaves SY stranded. | `ZeroAmount`(7), `InsufficientLiquidity`(9) on an empty pool |
| `lp_balance(a)`, `lp_total_supply()` | – | | – |
| `transfer_lp(from, to, amount)` | `from` | Moves LP; **both** sides compliant. | `InsufficientLpBalance`(10), `ZeroAmount`(7), compliance errors, `Paused`(5) |
| `seize_lp(caller, account, amount)` / `redeem_seized_lp(caller, lp)` | the configured escrow | Forced LP transfer to the escrow / burn of the escrow's own LP for PT + SY sent to the escrow. Work while paused and after maturity. | `NotRecoveryEscrow`(18), `ZeroAmount`(7), `InsufficientLpBalance`(10) |
| `claim_protocol_fees()` / `claim_creator_fees()` → SY | **anyone** | As `PrincipalManager`. | `NothingToClaim`(19) |
| `accrued_fees()` → `(protocol, creator)` | – | | – |
| `reserves()` → `(pt, sy)`, `pool_state()` → `PoolState` | – | Internal reserves (not token balances). | |
| `pt_price()` | – | Spot price of 1 PT in SY value at `SCALE`; `SCALE` at maturity. | `InsufficientLiquidity`(9) if unseeded |
| `implied_rate()` | – | Fixed annualized yield at the spot price, at `SCALE`; 0 at maturity. | |
| `time_exponent()` | – | Curve exponent `a` at 1e18; `1e18` at maturity. | |
| `quote_sy_for_pt(sy_in)`, `quote_pt_for_sy(pt_in)`, `quote_buy_exact_pt(pt_out)`, `quote_yt_for_sy(yt_in)` → `Quote{amount_out, amount_in, fee_shares}` | – | Exact price a trade would get right now; `quote_yt_for_sy` previews `swap_yt_for_sy`. | as the swaps |
| `quote_sy_for_yt(sy_in)` → `FlashMintQuote{yt_out, sy_back, mint_fee_shares, swap_fee_shares}` | – | Previews both `swap_sy_for_yt` (the Router composition) and `flash_mint_yt` — `sy_in - sy_back` is `flash_mint_yt`'s `net_cost`. | `ZeroAmount`(7) |
| `zap_swap_amount(sy_in)` | – | How much of `sy_in` a single-sided deposit would swap. | |
| `set_paused`, `transfer_admin`, `get_admin`, `is_paused` | admin | | `Unauthorized`(3) |
| `set_recovery_escrow(admin, escrow)`, `recovery_escrow()` | admin | One-time. | `RecoveryEscrowAlreadySet`(17) |
| views: `maturity`, `manager_address`, `sy_address`, `pt_address`, `yt_address`, `underlying_address`, `config_address` | – | | `NotInitialized`(2) |

## Router

Stateless coordinator. Holds no funds, needs no standing on the underlying, and always acts **as the
user**, so every downstream check is evaluated against the real user. A market is named by its pool and
must be registered.

Every user function except `redeem_at_maturity` and `claim_yield` takes a `deadline` (ledger timestamp; revert `DeadlineExpired` if `now > deadline`)
and, where there is an output, a `min_*_out` (revert `SlippageExceeded`). Downstream errors surface with
their own codes.

| Function | Auth | What it does |
|---|---|---|
| `initialize(admin)` | – | One-time (`AlreadyInitialized`(1)). |
| `register_market(caller, pool)` → `MarketInfo` | router admin | Resolves manager/SY/PT/YT/underlying from the pool and cross-checks them (`TopologyMismatch`(7), `AlreadyRegistered`(8), `Unauthorized`(3)). |
| `unregister_market(caller, pool)` | router admin | Removes a listing (`MarketNotRegistered`(4) if absent). |
| `is_registered(pool)`, `market(pool)`, `get_admin`, `transfer_admin` | – / admin | `market` reverts `MarketNotRegistered`(4). |
| `wrap_and_mint(from, pool, amount, min_pt_out, deadline)` → `MintResult` | `from` | underlying → SY → PT + YT. |
| `unwrap(from, pool, shares, min_underlying_out, deadline)` | `from` | SY → underlying. |
| `swap_sy_for_pt` / `swap_pt_for_sy(from, pool, amount, min_out, deadline)` | `from` | Pool trades. |
| `swap_underlying_for_pt(from, pool, underlying_amount, min_pt_out, deadline)` → PT | `from` | underlying → SY (`SYWrapper.deposit`, unguarded) → PT (`MarketPool.swap_sy_for_pt`, guarded by `min_pt_out`) — one signature for a caller who holds the underlying, not SY. |
| `swap_pt_for_underlying(from, pool, pt_in, min_underlying_out, deadline)` → underlying | `from` | PT → SY (`MarketPool.swap_pt_for_sy`) → underlying (`SYWrapper.withdraw_gained`, with a `baseline` read right before the swap). Works regardless of any pre-existing SY `from` already holds — an earlier version used `withdraw_all` with a zero-pre-existing-SY precondition instead, which a dApp-integration review found failed routinely for any caller who simply already held SY. |
| `swap_sy_for_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` → `(yt_out, sy_back)` | `from` | **Flash-mint:** mint PT + YT from `sy_in`, sell the PT into the pool, keep the YT and the SY proceeds. `min_yt_out` guards the YT minted; `max_net_cost` guards the pool price (`sy_in − sy_back`), reverting the pool's `SlippageExceeded`(8) if the PT fetches too little. |
| `swap_underlying_for_yt(from, pool, underlying_amount, sy_in, min_yt_out, max_net_cost, deadline)` → `(yt_out, underlying_back)` | `from` | Same flash-mint as `swap_sy_for_yt`, wrapping `underlying_amount` first and unwrapping the proceeds back to underlying at the end (`withdraw_gained`, baseline read before the wrap). Not a true flash loan: `from` needs the full `underlying_amount` in hand up front. `sy_in` is a caller-supplied literal (quote `SYWrapper.exchange_rate()` off-chain), **not** derived from the wrap's own result — an integration review found that chaining it caused the same `Error(Auth, InvalidAction)` drift `swap_pt_for_underlying` once hit. Works regardless of any pre-existing SY `from` already holds, same as `swap_pt_for_underlying`. |
| `flash_mint_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` → `(yt_out, net_cost)` | `from` | Forwards to `MarketPool.flash_mint_yt`: a **true flash swap**, unlike `swap_sy_for_yt`/`swap_underlying_for_yt` above. `from` only needs `max_net_cost` in SY, not the full `sy_in` notional — the pool funds the rest from its own reserves. Verified live tokenizing a notional larger than the caller's SY balance. |
| `flash_mint_yt_from_underlying(from, pool, sy_in, min_yt_out, max_net_cost, underlying_budget, min_underlying_back, deadline)` → `(yt_out, net_cost, underlying_back)` | `from` | Same true flash-mint, wrapping only `underlying_budget` of underlying (not the full notional) into SY first. `max_net_cost` is SY-denominated and independent of `underlying_budget` — see `docs/ROUTER_QUOTES.md` §2 for why the two are not simply convertible in general, even though they are numerically equal today. The surplus between `underlying_budget` and the real cost is unwrapped back to underlying before returning (`SYWrapper.withdraw_gained`, guarded by `min_underlying_back`), **not** left as SY — an integration review found leaving it as SY cost an extra signature before any further same-transaction call using `withdraw_all`. No zero-pre-existing-SY precondition. |
| `swap_yt_for_sy(from, pool, yt_in, min_sy_out, deadline)` | `from` | **Flash-redeem** via the pool. |
| `swap_yt_for_underlying(from, pool, yt_in, min_underlying_out, deadline)` → underlying | `from` | Same flash-redeem as `swap_yt_for_sy`, unwrapping the SY proceeds to underlying (`withdraw_gained`). Works regardless of any pre-existing SY `from` already holds, same as `swap_pt_for_underlying`. |
| `add_liquidity`, `add_liquidity_single_sy`, `remove_liquidity` | `from` | Pool LP operations. |
| `add_liquidity_single_underlying(from, pool, underlying_amount, sy_in, min_lp_out, deadline)` → `(pt_used, sy_used, lp)` | `from` | Underlying → SY (unguarded) → single-sided LP deposit (`add_liquidity_single_sy`), one signature for a holder of the underlying rather than SY — found missing in an integration review. `sy_in` is a caller-supplied literal (quote `SYWrapper.exchange_rate()` off-chain), not derived from the wrap, for the same `Error(Auth, InvalidAction)` reason as `swap_underlying_for_yt`'s `sy_in`. |
| `remove_liquidity_to_underlying(from, pool, lp_in, min_underlying_out, deadline)` → underlying | `from` | LP → PT + SY (`remove_liquidity`) → sell the PT leg (`MarketPool.swap_gained_pt_for_sy`) → unwrap (`SYWrapper.withdraw_gained`), one signature — found missing in an integration review (previously 2-3 separate calls). Both legs sweep only the gain since a baseline read before `remove_liquidity`, so pre-existing PT and/or SY `from` already holds survive untouched — an earlier version used the full-sweep primitives with a zero-pre-existing-balance precondition instead, which a dApp-integration review found failed routinely (the "LP only" zap and Keep YT deposits can themselves leave small remainders, and simply having bought PT on an earlier trade is enough to be "holding PT"). |
| `add_liquidity_keep_yt(from, pool, underlying_amount, mint_amount, pt_desired, sy_desired, min_lp_out, deadline)` → `KeepYtResult{pt_used, sy_used, lp_minted, yt_minted}` | `from` | Pendle-style "keep YT" deposit: wraps `underlying_amount`, mints PT+YT from `mint_amount` of it, deposits `pt_desired`/`sy_desired` as a normal proportional `add_liquidity`, and lets the caller keep the YT instead of selling it. No swap leg, so the only guard is `min_lp_out`. All three of `mint_amount`/`pt_desired`/`sy_desired` are caller-supplied literals (quoted beforehand via `quote_add_liquidity_keep_yt`, which uses `MarketPool.keep_yt_split`) — **never** derived mid-execution: an earlier version derived `pt_desired` from `mint`'s own live-oracle-rate-dependent output and `sy_desired` from the wrap's live-exchange-rate-dependent output, which an integration review found reverted `Error(Auth, InvalidAction)` the same way `swap_pt_for_underlying` once did. |
| `recombine(from, pool, amount, min_sy_out, deadline)` | `from` | PT + YT → SY before maturity. |
| `redeem_at_maturity(from, pool, pt_amount, yt_amount)` | `from` | PT and/or YT → underlying after maturity. |
| `claim_yield(from, pool)` | `from` | Accrued YT yield → underlying. |

### Router quotes (no auth, no fee)

Preview any of the five underlying-denominated functions above without needing a separate
`SYWrapper.exchange_rate()` call and manual conversion: each composes `MarketPool`'s own
SY-denominated quote with the exchange rate, replicating the real call's exact rounding.

| Function | Returns | Mirrors |
|---|---|---|
| `quote_underlying_for_pt(pool, underlying_amount)` | `Quote{amount_out, amount_in, fee_shares}` — `amount_out` is PT, unaffected by the wrap leg; `fee_shares` is the AMM fee in SY | `swap_underlying_for_pt` |
| `quote_pt_for_underlying(pool, pt_in)` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` — both converted to underlying | `swap_pt_for_underlying` |
| `quote_underlying_for_yt(pool, underlying_amount)` | `UnderlyingFlashMintQuote{yt_out, underlying_back, mint_fee_underlying, swap_fee_underlying}` | `swap_underlying_for_yt` |
| `quote_yt_for_underlying(pool, yt_in)` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` | `swap_yt_for_underlying` |
| `quote_add_liquidity_keep_yt(pool, underlying_amount)` | `KeepYtQuote{mint_amount, pt_desired, sy_desired, lp_estimate, yt_out, mint_fee_underlying}` — `mint_amount`/`pt_desired`/`sy_desired` are the exact values to pass as `add_liquidity_keep_yt`'s own arguments of the same names | `add_liquidity_keep_yt` end to end, including a preview of `add_liquidity`'s own `lp_minted` (`lp_estimate`) — exact only if reserves/`total_lp` do not move between this read and the real call, like every quote in this protocol |
| `quote_add_liquidity_underlying(pool, underlying_amount)` | `SingleSidedUnderlyingQuote{sy_in, pt_used, sy_used, lp_minted}` — `sy_in` is the exact value to pass as `add_liquidity_single_underlying`'s own `sy_in` argument | `add_liquidity_single_underlying` end to end (composes `MarketPool.quote_add_liquidity_single_sy`). Named without "single" in it — the full name with a `quote_` prefix exceeds Soroban's 32-byte function-name limit. Found missing in a dApp-integration review. |
| `quote_lp_to_underlying(pool, lp_in)` | `RemoveLiquidityUnderlyingQuote{pt_out, sy_out, swap_fee_shares, underlying_out}` | `remove_liquidity_to_underlying` end to end (composes `MarketPool.quote_lp_exit`). Named without "remove"/"to" in it for the same 32-byte limit reason. Found missing in the same review. |

These are provably exact against the real executed outcome under the current `SYWrapper`, not
just exact in the test numbers tried: `SYWrapper.exchange_rate()` can only ever equal
`RATE_SCALE` (1.0) for the lifetime of any instance of the current contract — nothing in
`deposit`/`withdraw`/`seize` can move `total_underlying`/`total_shares` out of the exact 1:1 ratio
the first deposit establishes, and there is no rebase/sync entrypoint that pulls in yield
independent of a deposit. `quote_underlying_for_yt` is the one function here with two
rate-dependent conversions straddling a state-mutating `deposit` inside the real call
(`swap_underlying_for_yt`'s own deposit, then its later `withdraw_all`) — if a future version of
`SYWrapper` ever gains a mechanism to move its own share rate within a single transaction, this
quote (which reads the rate once and reuses it for both conversions) would need rechecking against
the real call's two separate reads. See `docs/ROUTER_QUOTES.md` for the full exactness argument and
the regression test that proves it.

## RecoveryEscrow

Compliance recovery for SY, PT, YT and LP. **No admin key**: every entrypoint re-checks the issuer
authority live. A seizure needs `caller` = issuer authority *and* the target already deauthorized on the
underlying. Each event writes a [`RecoveryRecord`](#appendix-b-types).

| Function | What it does | Errors |
|---|---|---|
| `initialize(underlying, sy_wrapper, pt_token, yt_token, principal_manager, market_pool)` | One-time; every contract must report the same underlying. | `AlreadyInitialized`(1), `PositionUnderlyingMismatch`(6) |
| `seize_sy(caller, account, shares)` → underlying | Seize SY and **unwrap at once**; the escrow holds raw underlying ready for the issuer's native clawback. | `Unauthorized`(3), `TargetStillAuthorized`(4), `ZeroAmount`(5) |
| `seize_pt(caller, account, amount)`, `seize_yt(...)` | Seize into escrow, held fully backed until maturity. | as above |
| `seize_lp(caller, account, amount)` → `(pt_received, underlying_from_sy_leg)` | Seize LP, burn it in the pool, unwrap the SY leg at once, hold the PT leg. | as above |
| `seize_batch(caller, requests)` → record ids | Up to `MAX_BATCH` = 3 accounts in one transaction (bounded by the 100-entry ledger footprint with every position type held); one record each; **all-or-nothing** (one still-authorized target reverts the whole batch). Each `SeizeRequest` names per-position amounts; `0` skips a position. | `BatchTooLarge`(7), `ZeroAmount`(5) for an empty batch or an empty request, `TargetStillAuthorized`(4) |
| `seize_all_positions(caller, accounts)` → record ids | Reads each account's full SY/PT/YT/LP balances and seizes them all; accounts with nothing are skipped. | `NothingToSeize`(10) |
| `finalize_record(caller, id)` → `(underlying_from_pt, underlying_from_yt)` | At/after maturity, redeems the PT (`pt_amount + lp_pt`) and YT a record still holds and writes the underlying onto that record. Once per record. | `Unauthorized`(3), `RecordNotFound`(8), `AlreadyFinalized`(9), `NothingToFinalize`(11) for a pure-SY record; `NotMature` from the manager |
| `get_record(id)`, `account_records(account)` → ids oldest first, `record_count()`, `underlying_address()` | Views. | `RecordNotFound`(8) |

---

## Appendix A: events

| Contract | Topic → payload |
|---|---|
| OracleAdapter | `ref_set` → (value, timestamp); `ref_rej` → (value, timestamp, reason); `src_set` → (primary_source, fallback_source); `adm_xfer` → (from, to) |
| Permissioning | `acc_grant`/`acc_rev` → (caller, account); `acc_batch` → caller; `ast_grant`/`ast_rev` → (caller, account, asset); `adm_xfer` |
| RiskControl | `paused` → (caller, bool); `add_psr`/`rm_psr` → (caller, pauser); `add_cons`/`rm_cons` → (caller, consumer); `cb_limit` → (caller, limit); `ast_limit` → (caller, asset, limit); `cb_window` → (caller, ledgers); `adm_xfer` |
| MarketConfig | `mkt_new` → (underlying, maturity, tok_bps, yt_bps, tier_bps, share_bps); `fees_set` → (caller, tok, yt, tier); `payee_set`; `share_set`; `treas_set`; `padm_xfer` |
| SYWrapper | `deposit` → (from, amount, shares); `withdraw` → (from, shares, underlying); `sy_xfer` → (from, to, amount); `seize` → (escrow, account, shares); `paused`; `cap_set`; `rc_set`; `esc_set`; `adm_xfer` |
| PTToken / YTToken | `mint` → (to, amount); `burn` → (from, amount); `transfer` → (from, to, amount); `approve`; `seize` → (escrow, account, amount); `min_set`; `esc_set`. YT also: `idx_up` → (index, rate); `frozen` → rate; `claim` → (from, amount) |
| PrincipalManager | `mint` → (from, shares, notional, fee_shares); `recombine` → (from, amount, shares); `redeem` → (from, pt, yt, underlying_pt, underlying_yt); `yt_claim` → (from, gross, paid); `settled` → rate; `fee_claim` → (payee, amount, is_protocol); `paused`; `rc_set`; `adm_xfer` |
| MarketPool | `buy_pt`/`sell_pt`/`sell_yt` → (from, to, in, out, fee); `add_liq` → (from, pt, sy, lp); `zap_liq`; `rem_liq` → (from, to, lp, pt, sy); `lp_xfer`; `lp_seize`; `fee_claim`; `paused`; `esc_set`; `adm_xfer` |
| Router | `mkt_reg` → pool; `adm_xfer` |
| RecoveryEscrow | `recovery` → (caller, id, account, sy, pt, yt, lp); `finalize` → (id, account, underlying_pt, underlying_yt) |

## Appendix B: types

```rust
MintResult      { pt_minted, yt_minted, fee_shares }                       // PrincipalManager / Router
RedeemResult    { underlying_from_pt, underlying_from_yt }
PoolState       { pt_reserve, sy_reserve, total_lp, rate, exponent /*1e18*/, fee_rate /*1e12*/, seconds_to_maturity }
Quote           { amount_out, amount_in, fee_shares }
MarketInfo      { pool, manager, sy, pt, yt, underlying }                  // Router
SeizeRequest    { account, sy_shares, pt_amount, yt_amount, lp_amount }    // 0 = skip that position
RecoveryRecord  { id, account, ledger, timestamp,
                  sy_shares, underlying_from_sy,
                  lp_amount, lp_pt, lp_sy_shares, underlying_from_lp,
                  pt_amount, yt_amount, yt_yield_at_seize,
                  finalized, underlying_from_pt, underlying_from_yt }
```

Constants: `SCALE` = 1e7 · `FEE_SCALE` = 1e12 · `INDEX_SCALE` = 1e12 · `WAD` = 1e18 ·
`DEFAULT_WINDOW_LEDGERS` = 17 280 · `MAX_BATCH` = 3 · `MINIMUM_LIQUIDITY` = 1 000 ·
oracle staleness window = 3 600 s · fee caps: tokenization 100 bps, YT 5 000 bps, swap tier 500 bps.

## Keeper functions

Every long-lived contract (`SYWrapper`, `PTToken`, `YTToken`, `PrincipalManager`, `MarketPool`, `Router`, `MarketConfig`, `RecoveryEscrow`) exposes a permissionless `bump()` that extends its *instance* storage (admin, config, reserves, totals, settlement rate, fee buckets) by ~30 days. Soroban does not extend instance TTL on ordinary calls, so a long-dated market needs a keeper to call it (or to restore an archived instance). Per-user persistent entries are extended on every write; `Permissioning` entries only by `grant_*` (re-grant before expiry).
