# Yield Math, Market Fees and Compliance — with real numbers

A guide for developers building on Principal without reading the contracts. Every number below is
produced by the real contracts in
[`doc_examples.rs`](../contracts/integration_tests/tests/doc_examples.rs), which asserts each one; if a
contract change moves a number, that test fails and this page must change with it.

Conventions: amounts are raw integers at `SCALE = 10_000_000` (1e7), so `1.0` is `10_000_000` and
`104.9475` is `1_049_475_000`. Rates are "USDC per underlying × SCALE": `1.05` is `10_500_000`.

---

## 1. The moving parts

```
underlying (e.g. USDY)  ──deposit──►  SY shares  ──mint──►  PT  +  YT
       ▲                                  ▲                  │      │
       └─────────── redeem / unwrap ──────┴──── recombine ───┘      │
                                                                     ▼
                                               MarketPool (PT ⇄ SY)   YT yield accrues to the holder
```

* **SY** is a share of the wrapper's pool of underlying. `exchange_rate = total_underlying ×
  SCALE / total_shares`, and it is `1.0` in this release: value growth of an appreciating asset like
  USDY shows up in the **oracle rate**, not in the SY exchange rate.
* **The oracle rate** `r` is the USDC value of one underlying token. It is monotonically
  non-decreasing (`OracleAdapter.set_reference_value` reverts `ValueDecreased`), because the
  settlement math below assumes yield is never negative.
* **PT** is a claim to a fixed *notional* of USDC; **YT** is a claim to the appreciation of the
  underlying that notional is backed by. They are minted 1:1 and can be recombined 1:1.

## 2. Tokenizing: `notional = net_shares × r`

Minting `n` SY shares at oracle rate `r`, with a tokenization fee of `f` bps:

```
fee_shares = ceil(n × f / 10_000)          withheld in PrincipalManager's custody
notional   = (n − fee_shares) × r / SCALE  PT minted = YT minted = notional
```

**Example 1 — 100 SY at rate 1.05.**

| | fee | fee shares | PT minted | YT minted |
|---|---|---|---|---|
| fee-free market | 0 | 0 | **105.0000** | **105.0000** |
| 5 bps tokenization fee | 5 bps | **0.05 SY** | **104.9475** | **104.9475** |

The 0.05 SY fee is split by the market's protocol share (20 %): **0.01 SY** accrues to Principal and
**0.04 SY** to the market creator. Both stay in custody until `claim_protocol_fees` /
`claim_creator_fees`.

## 3. Yield: the `1/rate` index

A YT holder earns the appreciation of the underlying their notional is backed by. `YTToken` keeps one
global index `G = INDEX_SCALE × SCALE / r` (`INDEX_SCALE = 1e12`, rounded up) and, for each account,
the `G` at its last settlement. For `N` YT (notional units) settled at `r_s` and now at `r`:

```
pending (underlying) = N × (1/r_s − 1/r)  =  N × (G_s − G) / INDEX_SCALE
```

Every balance change (mint, burn, transfer either way, seize) settles the affected accounts first, so a
buyer never receives yield from before they held the position and a seller never loses yield already
earned.

**Why `1/r` and not "a percentage of notional".** PT and YT are minted in *notional* (`N = shares ×
r₀`) but yield accrues on the *underlying* the position holds (`N / r`). Summed over a position's life
the claims telescope to exactly what was deposited:

```
PT at maturity        N / r_final
+ YT yield, all steps N × (1/r₀ − 1/r_final)
= N / r₀              = the shares deposited, for ANY mint rate r₀ and ANY number of updates
```

**Example 2 — 100 SY minted at 1.05, settled at 1.10 (fee-free).**

| Leg | Formula | Underlying |
|---|---|---|
| PT (105 notional) | 105 / 1.10 | **95.4545** |
| YT (105 notional) | 105 × (1/1.05 − 1/1.10) | **4.5454** |
| **Total** | | **99.9999… ≈ 100.0000** (1 raw unit of rounding) |

The custody backing them (100 SY) covers the claims exactly. An earlier version scaled yield by a
percentage of notional, which is only equivalent at `r₀ = 1.0`; at `r₀ = 1.05` it claimed 100.23 SY
against 100 held. That is fixed and regression-tested (`pt_plus_yt_claims_never_exceed_the_deposited_
shares_at_any_mint_rate`, and the solvency assertion in `full_lifecycle.rs`).

**Yield stops at maturity.** The first `update_yield_index` at or after maturity advances the index one
last time from the fresh oracle and *freezes* it. `PrincipalManager.settle_all()` (permissionless)
triggers that and records the frozen rate as the single settlement rate every redemption uses, so PT and
YT are always settled against the same number and later oracle moves change nothing.

## 4. The YT fee

`yt_fee_bps` of every yield payout — the mid-life `claim_yield` and the YT leg of `redeem` — is
withheld as SY shares and accrued 20 % / 80 % like the other fees.

**Example 3 — the same position with the documented 10 % YT fee.** 104.9475 YT minted at 1.05; at
1.10 the holder calls `claim_yield`:

```
gross  = 104.9475 × (1/1.05 − 1/1.10)                  ≈ 4.5432 underlying
fee    = 10 % of gross                                  ≈ 0.4543  (stays in custody)
paid   = gross − fee                                    = 4.0889  underlying, to the holder
```

Fee buckets afterwards: protocol **0.1009 SY**, creator **0.4035 SY** (0.05 tokenization + 0.4543 YT,
split 20/80).

## 5. Market fees at a glance

| Fee | Charged on | Example | Set by |
|---|---|---|---|
| Tokenization | SY tokenized at `mint` | 5 bps | the underlying's issuer authority |
| YT | yield paid to a YT holder | 10 % | the issuer authority |
| Swap | each PT⇄SY trade, in SY | Fee Tier 0.1 % × days-to-maturity / 365 | the issuer authority (the tier) |
| Protocol share | of *every* fee above | 20 % (rest 80 % → market creator) | Principal (`protocol_admin`) |

Hard caps (so a compromised key cannot configure a confiscatory market): tokenization ≤ 1 %, YT fee ≤
50 %, swap tier ≤ 5 % (effective swap fee ≤ 10 %). The market creator is the underlying's **current**
issuer authority — for a Stellar asset, `SAC.admin()` read at payout time, so an admin rotation redirects
the creator share with nothing to update.

**Example 4 — the swap-fee schedule** (Fee Tier 0.1 %):

| Days to maturity | 365 | 180 | 90 | 30 | 7 | 1 | 0 |
|---|---|---|---|---|---|---|---|
| Fee rate | 0.1000 % | 0.0493 % | 0.0247 % | 0.00822 % | 0.00192 % | 0.000274 % | 0 |

A 1 000 SY trade at 90 days pays 0.2466 SY of fee; the same trade a week out pays 0.0192 SY.

## 6. The AMM: price, time decay, convergence

The pool trades PT against SY on `x^a + y^a = k`, `a = 1 − τ/S` (details in
[AMM_DESIGN.md](AMM_DESIGN.md)). `x` is the SY reserve valued in notional, `y` the PT reserve, `τ` the
time to maturity, `S` the time stretch (4 years here). The spot price of one PT in SY is
`(x / y)^(τ / S)`.

**Example 5 — a pool of 1 000 PT against SY worth 950, 180 days out (`S` = 4 years).**

* exponent `a = 1 − (180/365)/4 = 0.8767`
* opening price `0.95^0.1233 =` **0.99376** → a fixed yield of `(1/0.99376 − 1) × 365/180 =` **1.27 % a year**
* buying PT with 20 SY returns **20.0648 PT**, paying **0.00986 SY** fee (0.1 % × 180/365 × 20)

**Time decay with no trading at all** — reserves untouched, only the clock moving:

| Day | 0 | 60 | 90 | 120 | 150 | 179 | 1 minute before maturity |
|---|---|---|---|---|---|---|---|
| PT price | 0.99376 | 0.99583 | 0.99687 | 0.99792 | 0.99896 | 0.99997 | **0.9999999** |
| Implied rate | 1.274 % | ≈1.27 % | ≈1.27 % | ≈1.27 % | ≈1.27 % | ≈1.27 % | – |

The price climbs to par by itself and the implied fixed rate stays put — so LPs, who hold a fixed
basket, gain from the convergence instead of losing to it (**no time-decay impermanent loss**). At
maturity the exponent is exactly 1, swaps close, and PT redeems at par through `PrincipalManager`.

## 7. Buying and selling YT (flash-mint / flash-redeem)

YT is not pooled. **Buying YT** is `Router.swap_sy_for_yt`: tokenize the SY into PT + YT, sell the PT into
the pool, keep the YT and the proceeds — in one atomic transaction.

**Example 6 — 100 SY into the pool of Example 5 (fee-free).**

* 100 SY tokenize into **100 PT + 100 YT**
* the 100 PT sell for **98.1284 SY** (this trade is 10 % of the pool, so it moves the price)
* net cost of 100 YT = **1.8716 SY** → **1.87 cents per YT**

**Selling YT** is `MarketPool.swap_yt_for_sy` (flash-redeem): the pool takes the YT, recombines it with
100 PT from its own reserve into 100 SY, keeps the curve price of that PT, and pays the difference —
**1.8716 SY**, so the round trip returns the 100 SY to within 2 raw units. Near maturity the PT costs
~par, YT is worth ~0, and the call reverts rather than take value from the pool.

## 8. Recombination

Before maturity, `PrincipalManager.recombine(amount)` burns `amount` PT and `amount` YT and returns
`amount × SCALE / r` SY at the *current* rate. At `r = 1.25`, 100 PT + 100 YT return **80 SY**; the other
**20 SY** back the yield the YT already accrued, which stays claimable afterwards
(`recombine_returns_sy_at_the_current_rate_and_keeps_accrued_yield_claimable`).

## 9. Compliance in one paragraph

Every SY, PT, YT and LP operation checks that **both** the sender and the recipient are authorized on
the *underlying* — `SAC.authorized()` for a Stellar asset (including a SEP-8 regulated asset), or
not-frozen-and-identity-verified for a SEP-57 RWA token — and, optionally, the market administrator's
`Permissioning` allow-list. Creating a market, retuning fees and recovering an account need the
underlying's live issuer authority. Nothing is copied into a Principal-run list. See
[COMPLIANCE_ARCHITECTURE.md](COMPLIANCE_ARCHITECTURE.md) for the SAC / SEP-8 / SEP-57 details and what
each cannot express.

## 10. Circuit breaker

`SYWrapper.deposit` and `PrincipalManager.mint` each call `RiskControl.check_deposit` **inside the same
transaction**, so a deposit or mint that would exceed the limit reverts by itself. The window is counted in
**ledger sequence numbers** (default 17 280 ledgers ≈ 24 h at ~5 s), not wall-clock time, and a
**per-asset** limit sits beside the protocol-wide one. Volume is counted at each entry point, so a
`wrap_and_mint` of 100 counts 200 (100 at the wrapper, 100 at the manager).
