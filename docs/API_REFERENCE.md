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
never negative.

| Function | Auth | What it does / needs | Errors |
|---|---|---|---|
| `initialize(admin)` | – | One-time setup. | `AlreadyInitialized`(1) |
| `set_reference_value(caller, value, timestamp)` | `caller` = admin | Publishes `value` at `timestamp`. Needs `value > 0`, `value ≥` the stored value, and `stored timestamp < timestamp ≤ ledger time` (a future-dated timestamp is rejected so the feed cannot be bricked). | `Unauthorized`(2), `InvalidValue`(3), `TimestampTooOld`(4), `ValueDecreased`(6), `TimestampInFuture`(7) |
| `get_reference_value()` → `i128` | – | Last published value. | `NotInitialized`(5) if none |
| `get_reference_timestamp()` → `u64` | – | Timestamp of the last value. | `NotInitialized`(5) |
| `is_fresh(max_stale_seconds)` → `bool` | – | `true` iff `ledger_time − stored_timestamp ≤ max_stale_seconds`. A stored timestamp *ahead* of the ledger clock is stale. Consumers use 3 600 s. | – |
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
| all other functions | | identical to `PTToken`, except that `mint`/`burn`/`transfer`/`transfer_from`/`seize` also settle each affected account's pending yield *before* the balance moves | error codes differ: `OracleStale`=10, `NotAuthorizedOnSac`=11, `IssuerMismatch`=12, `RecoveryEscrowAlreadySet`=13, `NotRecoveryEscrow`=14 |

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
| `add_liquidity(from, pt_desired, sy_desired, min_lp_out)` → `(pt_used, sy_used, lp)` | `from` | First deposit sets the opening price (SY value must not exceed PT) and locks 1 000 LP; later deposits are pro-rata (`ceil` on the amounts taken), surplus stays with the caller. | `InvalidInitialRatio`(15), `MinimumLiquidity`(21), `ZeroAmount`(7), `SlippageExceeded`(8), `Expired`(6), `Paused`(5) |
| `add_liquidity_single_sy(from, sy_in, min_lp_out)` → `(pt_used, sy_used, lp)` | `from` | Swaps just enough SY into PT (normal fee) to deposit both in the post-swap ratio; rounding leftovers are refunded. Needs a seeded pool. | as above, `InsufficientLiquidity`(9) on an empty pool |
| `remove_liquidity(from, to, lp, min_pt_out, min_sy_out)` → `(pt, sy)` | `from` | Pro-rata exit, also after maturity, and without an oracle. The pool pause blocks it while the market is live but never after maturity. | `InsufficientLpBalance`(10), `ZeroAmount`(7), `SlippageExceeded`(8), `Paused`(5), compliance errors |
| `lp_balance(a)`, `lp_total_supply()` | – | | – |
| `transfer_lp(from, to, amount)` | `from` | Moves LP; **both** sides compliant. | `InsufficientLpBalance`(10), `ZeroAmount`(7), compliance errors, `Paused`(5) |
| `seize_lp(caller, account, amount)` / `redeem_seized_lp(caller, lp)` | the configured escrow | Forced LP transfer to the escrow / burn of the escrow's own LP for PT + SY sent to the escrow. Work while paused and after maturity. | `NotRecoveryEscrow`(18), `ZeroAmount`(7), `InsufficientLpBalance`(10) |
| `claim_protocol_fees()` / `claim_creator_fees()` → SY | **anyone** | As `PrincipalManager`. | `NothingToClaim`(19) |
| `accrued_fees()` → `(protocol, creator)` | – | | – |
| `reserves()` → `(pt, sy)`, `pool_state()` → `PoolState` | – | Internal reserves (not token balances). | |
| `pt_price()` | – | Spot price of 1 PT in SY value at `SCALE`; `SCALE` at maturity. | `InsufficientLiquidity`(9) if unseeded |
| `implied_rate()` | – | Fixed annualized yield at the spot price, at `SCALE`; 0 at maturity. | |
| `time_exponent()` | – | Curve exponent `a` at 1e18; `1e18` at maturity. | |
| `quote_sy_for_pt(sy_in)`, `quote_pt_for_sy(pt_in)`, `quote_buy_exact_pt(pt_out)` → `Quote{amount_out, amount_in, fee_shares}` | – | Exact price a trade would get right now. | as the swaps |
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
| `swap_sy_for_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` → `(yt_out, sy_back)` | `from` | **Flash-mint:** mint PT + YT from `sy_in`, sell the PT into the pool, keep the YT and the SY proceeds. `min_yt_out` guards the YT minted; `max_net_cost` guards the pool price (`sy_in − sy_back`), reverting the pool's `SlippageExceeded`(8) if the PT fetches too little. |
| `swap_yt_for_sy(from, pool, yt_in, min_sy_out, deadline)` | `from` | **Flash-redeem** via the pool. |
| `add_liquidity`, `add_liquidity_single_sy`, `remove_liquidity` | `from` | Pool LP operations. |
| `recombine(from, pool, amount, min_sy_out, deadline)` | `from` | PT + YT → SY before maturity. |
| `redeem_at_maturity(from, pool, pt_amount, yt_amount)` | `from` | PT and/or YT → underlying after maturity. |
| `claim_yield(from, pool)` | `from` | Accrued YT yield → underlying. |

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
| OracleAdapter | `ref_set` → (value, timestamp); `adm_xfer` → (from, to) |
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
