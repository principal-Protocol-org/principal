# AMM Design — the PT/SY yield-curve pool

`MarketPool` is the PT/SY market for one (underlying, maturity) pair. This document records *why* it
is built the way it is: what existing Stellar/Soroban projects were studied and what was taken from or
rejected about each, the curve and its arithmetic, rounding and safety rules, fees, liquidity, the YT
flash paths, and measured resource use. The function-level reference is in
[API_REFERENCE.md](API_REFERENCE.md); worked numbers are in
[YIELD_MATH_AND_FEES.md](YIELD_MATH_AND_FEES.md).

## 1. What existing Stellar projects taught us

Before committing to a design, the open-source Soroban AMMs and yield-tokenization protocols were
read. What was actually read, and what it changed:

| Project | What was read | Decision it drove |
|---|---|---|
| **Sidereal** (`sidereal-tech/contracts`) — PT/YT/SY protocol with a time-decay AMM, on mainnet | `ARCHITECTURE.md`, `findings.md` (its own post-launch findings list) | **Unit discipline.** Its finding *M6* is a curve that mixed raw SY *shares* with asset-denominated PT face value, leaking value to traders once the SY rate moved off 1.0 — its fixture defaulted to rate 1.0, where shares and assets coincide, so tests missed it. Our curve is evaluated only in notional-value units (`shares × oracle_rate / SCALE`), and the AMM suite **sweeps the rate over {1.00, 1.01, 1.05, 1.10}** (`sweeping_the_oracle_rate_keeps_the_curve_in_value_units`). Also adopted: LP priced from internal reserves with a `MINIMUM_LIQUIDITY` burn (it found no first-depositor donation issue for that reason); a YT yield formula of the form `(R − c)/(c·R)` that telescopes across settlements (ours: the 1/rate index, [§ YT index](YIELD_MATH_AND_FEES.md)); an *escrow-coverage* invariant `custody ≥ PT supply + unclaimed YT yield` (ours: the solvency assertions in `full_lifecycle.rs`); flash-swap YT routes done atomically inside one transaction. Not adopted: Pendle's logit curve (see §2), and fees left in reserves for LPs (the grant terms route swap fees to the protocol and market creator). |
| **Everspan** (`sayweer/everspan`) — PT/SY AMM, splitter, Blend vault, on testnet | README | A plain **constant-product** PT/SY pool prices the fixed rate from the reserve ratio but has **no time decay**: PT does not converge to par by construction, so LPs carry structural loss as maturity approaches. Rejected for that reason. Its explicit two-step "split, then sell the principal" YT purchase is the same shape as our Router flash-mint. |
| **Soroswap** (`soroswap/core`) — factory / pair / router constant-product AMM, audited | README | The **router discipline**: every user call takes a minimum output *and a deadline*; LP tokens live in the pair; `MINIMUM_LIQUIDITY`. All three adopted (`deadline` and `min_*_out` on every Router flow). Its pair/factory/router split maps onto our pool/manager/router, except that we register markets by pool rather than deploying pairs from a factory. |
| **Aquarius** (`AquaToken/soroban-amm`) | repository listing only | Not analysed in detail; noted as the stable-swap / concentrated-liquidity reference should a second pool type be added. |

**Not reviewed:** Comet (Balancer-style weighted pools). Its repository README covers only testing
tooling; no design claims are made from it.

Lessons applied that are not in any single project: prove the arithmetic against an independent
reference, meter it as real WASM (§8), and make rounding conservative in proportion to magnitude (§5).

## 2. The curve

Two families fit the requirement "PT must converge to par at maturity with no time-decay
impermanent loss": Pendle's logit-based curve (used by Sidereal) and the **Yield Space
constant-power-sum** curve. Principal's README has always described the latter; the older
specification text described the former. They were contradictory; this implementation settles on
Yield Space:

```
    x^a + y^a = k            a = 1 − τ / S            0 < a ≤ 1
```

* `x` — the SY reserve **valued in notional**: `shares × oracle_rate / SCALE` (the same rate
  `PrincipalManager` uses to size PT, so one SY of value and one PT of face are directly comparable).
* `y` — the PT reserve.
* `τ` — seconds to maturity; `S` — the pool's *time stretch* (`time_stretch_years`, 1–20 years).
* Marginal price of PT in SY value: `p = (x / y)^(1 − a) = (x / y)^(τ / S)`.

Why this one:

* **Par by construction.** As `τ → 0`, `a → 1`, the invariant collapses to the constant sum
  `x + y = k` and `p → (x/y)^0 = 1` for *any* reserve ratio. No special case, no oracle, no keeper.
* **No time-decay impermanent loss.** LPs hold a fixed basket of PT and SY. The curve sliding to par
  underneath them is pure gain (`example_4` in the guide: untraded reserves, price 0.99376 → 0.99997
  over 179 days, implied rate flat at ~1.27%).
* **Closed form both ways.** Swapping holds `k` fixed, moves one reserve, and recovers the other as
  `(k − p'^a)^(1/a)`. Exact-in *and* exact-out are the same computation, so there is no Newton
  iteration, no convergence failure mode, and the flash-redeem path can price "buy exactly N PT"
  directly. (The logit form needs an iterative solve for one of the two directions.)
* **The opening price is set by the first LP** (the ratio of their deposit), not by a separate rate
  anchor parameter. The first deposit must not price PT above par (`InvalidInitialRatio`).

Bounds: the exponent must stay ≥ 0.25 (so `1/a ≤ 4` and the fixed-point intermediates fit `i128`),
i.e. the market's remaining life must be < 75 % of the stretch — enforced at pool creation
(`MaturityTooFar`). A 4-year stretch supports markets up to 3 years.

Swaps are refused at and after maturity (`Expired`); `remove_liquidity` is always available. After
maturity PT redeems at par through `PrincipalManager`, independent of any pool price.

## 3. Arithmetic (`market_pool/src/math.rs`)

Soroban has no floating point, so the curve needs integer `ln`, `exp` and `pow`:

* **Format:** signed fixed point, `WAD = 1e18`, in `i128`.
* **`ln(x)`:** reduce `x = m·2^k`, `m ∈ [1, 2)`; then `ln m = 2·atanh((m−1)/(m+1))` by its odd series
  (`z < 1/3`, converging by 1/9 per term) plus `k·ln 2`.
* **`exp(x)`:** `x = k·ln 2 + r`, `|r| ≤ ln2/2`, Taylor series for `e^r`, shifted by `2^k`. Flushes to
  zero below `e^-42`; errors above `e^40`.
* **`pow(b, e) = exp(e · ln b)`.**
* **`solve(p, q, p', a)`** works on **ratios to the largest reserve** `u = max(p, q, p')`, so pool size
  never overflows the intermediates: `(q'/u) = ((p/u)^a + (q/u)^a − (p'/u)^a)^(1/a)`, then `q' = u ·
  (q'/u)`. A residual below 1e-12 is treated as a drained reserve (`InsufficientLiquidity`).

**Verified against an independent reference.** The unit tests compare `ln`, `exp`, `pow` and `solve`
with `f64` across 20 orders of magnitude, and check the invariant `p^a + q^a = k` before and after
every solved move (`market_pool/src/math_test.rs`). Measured relative error is a few 1e-17 of the
largest reserve.

## 4. Fees

`Trading Fee = Fee Tier × Days to Maturity / 365`, with days measured to the second, from the market's
`MarketConfig.swap_fee_rate(seconds_to_maturity)` at `FEE_SCALE = 1e12`. It is highest at issuance
and falls linearly to zero at maturity, so arbitrage back to par is free at the end. The rate has 1e-12
resolution: it is accurate to the last day (0.1 % tier, 1 day out = 0.000274 %).

* Charged **in SY shares**: on the SY going in when buying PT; on the SY coming out when selling PT;
  on the SY the pool keeps when buying PT for a flash-redeem.
* **Not added to the reserves.** It accrues to two buckets split by the config's protocol share
  (initially 20 % Principal / 80 % market creator). `claim_protocol_fees` / `claim_creator_fees` are
  permissionless; the payee is read from `MarketConfig` (treasury; the underlying's live issuer
  authority), so calling them cannot redirect value.
* **Open design point:** under this split LPs earn *no* swap fees; their return is PT convergence
  alone. That follows the grant terms literally (20 % protocol / 80 % creator of every swap fee). If
  LP fee income is wanted, add an LP share to `MarketConfig.split` — one field, one contract.

## 5. Rounding and safety rules

1. **Everything rounds against the trader and toward the pool.** Inputs and costs round up, outputs
   round down, at every unit boundary (shares ↔ value, fee).
2. **A magnitude-scaled pad, not a unit of last place.** `solve` returns its result rounded up by
   `1 + u / 1e14` raw units. The first version padded by a single unit; the huge-position test
   (`the_largest_sane_position_round_trips_without_overflow`) then showed a *profitable* fee-free round
   trip of 6 units on a 1e17-unit pool, because fixed-point error scales with pool size. The pad is
   ~100× the worst observed error and costs a trader at most 1e-14 of pool size per swap.
   `fee_free_round_trips_never_profit_at_any_pool_size` now asserts no profit from 1e8 to 1e17 units.
3. **The invariant `k` never decreases** across a pseudo-random 40-swap sequence
   (`invariant_never_decreases_across_a_pseudo_random_swap_sequence`).
4. **Reserves are internal state.** Pricing and LP minting use the pool's own `ReservePT`/`ReserveSY`,
   never a live token balance, so tokens sent straight to the pool change nothing
   (`a_donation_to_the_pool_cannot_skew_lp_pricing`). Test:
   `pool_token_balances_always_match_internal_reserves_plus_fees` checks balances = reserves + fees.
5. **Oracle freshness gates every state-changing trade** (`OracleStale`); liquidity *removal* needs no
   oracle, so a stale feed can never trap LPs.
6. **A reserve may never fall below 1 000 raw units** after a swap.
7. **Checks-effects-interactions.** Reserve and fee state is written before any token moves.

## 6. Liquidity

* `add_liquidity(pt, sy, min_lp)`: the first deposit mints `isqrt(pt·sy) − 1000` LP and permanently
  locks 1000 LP to the pool; later deposits mint `min(pt/PT_res, sy/SY_res) × supply`, take
  `ceil` of each side, and leave the surplus with the caller.
* `add_liquidity_single_sy(sy, min_lp)`: bisects (18 steps, ~2·10⁻⁵ of the input) for the swap size at
  which the remaining SY and the PT bought exactly match the post-swap ratio, buys that much PT at the
  normal fee, deposits both, and refunds rounding leftovers.
* `remove_liquidity` is pro-rata, always available, also after maturity.
* LP is an internal ledger (`lp_balance`, `transfer_lp`), not a SEP-41 token. It is compliance-gated
  on both sides of every transfer and can be seized by the escrow (`seize_lp`).

## 7. YT: flash-mint and flash-redeem

YT is not pooled — one PT/SY pool serves both instruments, so liquidity is not fragmented.

* **Buy YT** — `Router.swap_sy_for_yt`: `PrincipalManager.mint(user, sy_in)` produces PT + YT, then the
  PT is sold into the pool with `swap_pt_for_sy`; the user keeps the YT and the SY proceeds. Both steps
  are calls made *as the user*, in one atomic transaction; there is no loan and the Router holds
  nothing. Net cost = `sy_in − sy_back`.
* **Sell YT** — `MarketPool.swap_yt_for_sy`: the pool takes the user's YT, uses `yt_in` of its **own PT
  reserve** to `recombine` PT + YT into SY, keeps the curve price of that PT (plus fee) and pays the user
  the remainder. Nothing is borrowed across calls, so the pool cannot be left short; if the recombined
  SY does not cover the PT's price the call reverts `InsufficientLiquidity` and nothing moves (near
  maturity, PT ≈ par and YT is worth ≈ 0 — the correct outcome).

  A pool-internal path was chosen over Pendle's callback flash-swap because Soroban forbids contract
  re-entrancy: a pool→Router→pool callback would be rejected by the host.

## 8. Resource use, measured on real WASM

Native tests do not meter guest instructions, so `tests/budget.rs` registers the pool from its
compiled WASM and reads the CPU meter. Soroban's per-transaction limit is 100 000 000 instructions.

| Operation | CPU instructions | % of limit |
|---|---|---|
| `swap_sy_for_pt` | ~4.6 M | 4.6 % |
| `swap_pt_for_sy` | ~4.6 M | 4.6 % |
| `add_liquidity` | ~2.7 M | 2.7 % |
| `add_liquidity_single_sy` (18-step bisection) | ~31.7 M | 32 % |
| `Router.swap_sy_for_yt` (flash-mint) | ~6.9 M | 6.9 % |
| `swap_yt_for_sy` (flash-redeem) | ~6.4 M | 6.4 % |
| `remove_liquidity` | ~2.7 M | 2.7 % |

(Pool guest only; the other contracts in each call add their own, smaller, cost.) Contract sizes
(release, `wasm32v1-none`): `market_pool` 66 KB, `principal_manager` 46 KB, `yt_token` 42 KB,
`recovery_escrow` 38 KB, `router` 34 KB, `sy_wrapper` 33 KB, `pt_token` 28 KB, `market_config` 22 KB —
all under the 128 KB limit. The CI `wasm` job gates on it.

## 9. Known limitations

* **One curve family, one pool per market.** No concentrated liquidity; no second pool type.
* **No implied-rate TWAP oracle** (`MarketPool` exposes spot `pt_price()` / `implied_rate()` only).
  Spot is manipulable within a block by anyone with capital; do not use it as a lending oracle.
* **SY value uses the oracle rate only**, matching `PrincipalManager`; a rebasing underlying whose SY
  exchange rate drifts from 1.0 would need the two rates reconciled (out of scope until onboarded).
* **Swap fees do not reach LPs** (§4).
* **Time stretch is a fixed per-pool parameter**, chosen by the market creator at pool creation.
