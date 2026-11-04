# AMM Design Review: Our Curve, Sidereal's Curve, and the Flash-Swap-for-YT Decision

Requested as a senior-auditor review of two open items from the dApp developers' thread: whether
to build a true flash swap for YT (removing the capital-up-front requirement on
`swap_underlying_for_yt`), and whether to adopt or reuse Sidereal's AMM contracts
(`github.com/sidereal-tech/contracts`), a colleague's suggestion. Both questions turn out to have
the same answer once the two codebases are actually compared: **build the flash swap ourselves, on
our own curve; do not adopt or port any of Sidereal's contracts.**

## 1. Method

Sidereal's public contracts repo was cloned and read directly — not summarized from its README.
It ships its own internal audit (`docs/audit/2026-08-internal-audit.md`, dated 2026-08-18) and a
live-user-simulation findings log (`findings.md`), both far more detailed and more damaging than
anything a fresh outside read would have found independently; this review leans on them as primary
source, cross-checked against the actual source lines they cite, not taken on faith. Our own AMM
(`contracts/market_pool`) was re-checked against the specific failure classes Sidereal's audit
documents, not just reread in isolation — the comparison is the point.

## 2. What Sidereal's own audit says about Sidereal

Quoting their repo's own README: *"Live on Stellar mainnet since 2026-07-11. Not professionally
audited; the contracts are immutable, so treat the deployment as early and unaudited."* Their
internal audit (same repo, `docs/audit/2026-08-internal-audit.md`) is more specific:

| Area | Their own verdict |
|---|---|
| V1 AMM (live on mainnet, immutable) | **"Not safe to seed with third-party liquidity."** |
| V2 vault + strategy seam (not yet on mainnet) | **"Not safe to deploy."** |

Two **CRITICAL** findings, neither fixable on the already-deployed contract:

- **C1** — the AMM's curve prices PT face (asset-denominated) against raw SY *shares* with no
  index conversion (`amm/src/lib.rs:799`, `let total_asset = state.total_sy;` — a variable named
  for assets, holding shares). At the SY wrapper's live rate, this is a riskless arbitrage against
  LPs whenever `e < R` (R = the wrapper's *cumulative* appreciation since its own inception, not
  the market's own term) — measured by their own bit-exact model at **2–7.5% of pool reserves per
  trade** depending on how far the rate has moved. Fixed in the source tree we read (asset-
  denominated on both sides now), but the live mainnet bytecode is immutable and still has it.
- **C2** — the untested V2 vault's exchange rate can be inflated by an ordinary token transfer to
  the strategy contract (no deposit call, no auth needed) — a textbook first-depositor/donation
  exploit. Their own worked numeric example: an attacker nets +249.99 on a 500 outlay, leaving the
  victim with ~250 of a 1000 deposit. Fixed in the tree we read; was never on mainnet.

Six **HIGH** findings, the two most relevant to the question actually asked here:

- **H1** — both YT "flash routes" (the exact feature this review was asked to evaluate for reuse)
  break once the SY rate exceeds roughly 0.5% cumulative accrual: selling YT reverts
  `InsufficientLiquidity` for every input, and *buying* YT's solver can return 0.0000001 YT while
  keeping 100% of the buyer's SY — their own words: *"a 100% loss of principal is not
  suboptimality."* This is the direct consequence of C1, in the exact code path we would have
  wanted to borrow.
- **H4** — a YT holder can self-transfer to a fresh address during a rate dip and double-claim
  already-paid yield; reproduced on-chain against their own integration harness, with their own
  stated invariant (`escrow must cover PT+YT claims`) failing the assertion. Root cause: a
  per-holder checkpoint keyed directly to the raw oracle rate, with a fresh recipient initialized
  to *whatever the rate currently reads* rather than `max(current, sender's prior checkpoint)` —
  and their underlying yield source (Blend v2) can legitimately lower its rate when socializing bad
  debt, so "the rate only goes up" (asserted in their own code comments and tests) is false for
  them, not just unlikely.

Plus H2 (SY-in swaps can donate up to 84.7% of a trade's value to LPs with no refund, independent
of C1), H3 (their TWAP — marketed as manipulation-resistant lending collateral — can be set to an
arbitrary value by one atomic trade-and-reverse for ~0.1% of pool value), M5 (no fee anywhere in
the protocol, bolted on post-launch, after the AMM shipped irreversibly), M6 (the YT routes'
advertised 0.3% fee is actually 1.6–5%+ in practice, charged on leveraged notional instead of the
real SY risked), and M8 (their SY token isn't SEP-41 conformant and emits no events — permanent,
since it's immutable). None of this is a hostile outside read: it is their own documented,
dated, line-cited internal audit.

## 3. The same failure classes, checked against our own AMM

| Failure class (Sidereal's own finding) | Our status |
|---|---|
| **C1** — curve mixes SY shares with asset-unit face, leaking value once the rate moves off 1.0 | **Not present, and defended by a named regression test.** `contracts/integration_tests/tests/amm.rs::sweeping_the_oracle_rate_keeps_the_curve_in_value_units` sweeps the oracle rate across 1.00/1.01/1.05/1.10 and asserts identical curve economics at every rate — its own comment: *"A comparable protocol once mixed raw SY shares with asset units in its curve, which leaked value once the SY rate moved off 1.0."* `MarketPool::value_of(sy_reserve, rate)` converts SY shares to value units inside the curve on every call that touches the curve (`calc_buy_pt`, `calc_sell_pt`, `calc_buy_exact_pt`), not only on the flash routes. |
| **H1** — YT flash routes break above ~0.5% accrual | **Not present.** `swap_sy_for_yt`/`swap_yt_for_sy` were run live on Testnet against a 1.05 oracle rate this session (`docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §4.1, §5.4) and in the Rust suite at the same rates `sweeping_the_oracle_rate_keeps_the_curve_in_value_units` checks, with no special-casing needed. |
| **H4** — per-holder checkpoint keyed to raw rate; fresh recipient starts at the dipped rate, enabling a double-claim | **Not present, for two independent reasons.** (1) Our `OracleAdapter.set_reference_value` hard-rejects any value below the currently stored one (`ValueDecreased`) — the "rate can fall" precondition that triggers their bug cannot arise in our oracle layer at all, by construction, not by assumption. (2) Even defensively, `YTToken::advance` only ever updates the global yield index `if now_rate > last_rate` — a second, independent gate. Our yield accounting is a single global monotone index (`YieldIndex`), not a per-holder rate checkpoint, so there is no "this holder's own checkpoint" state for a fresh address to under-initialize in the first place. |
| **H2** — unspent swap budget silently donated to LPs instead of refunded | **Partially relevant, worth a explicit test when the new flash-buy-YT path is built** (§5). Our existing `swap_sy_for_pt`/`swap_pt_for_sy` are exact-input with no independent liquidity cap that could leave a budget unspent the way Sidereal's `exchange_rate >= WAD` floor does, so this specific shape doesn't apply today — but the new pool-funded flash-buy-YT route (§5) introduces a similar "buyer pays a budget, pool fills what the curve allows" shape for the first time in our protocol and must get this right from the start. |
| **M6** — fee charged on leveraged notional instead of real SY risked, contradicting the advertised rate | **Applies directly to the design in §5** — called out there. |
| **M5** — no fee decided before an immutable deploy | **Not applicable.** Our fee model (tokenization fee, YT fee, swap fee tier, protocol share) has existed since Tranche 1, is configured at market creation, and nothing here proposes shipping a fee-less AMM. |

This is not "our code happened to avoid bugs nobody thought to check for." `sweeping_the_oracle_rate_keeps_the_curve_in_value_units`'s own comment shows the C1-class failure was a known, named risk this codebase was already defending against before this review started.

## 4. Decision: do not adopt or port Sidereal's AMM or vault contracts

Both halves of the thing being evaluated — the AMM and the SY-vault seam it would need to pair
with — are rated **"not safe to seed with third-party liquidity"** / **"not safe to deploy"** by
Sidereal's own internal audit, with two CRITICAL and six HIGH findings, one of them already live
with real funds on an immutable mainnet contract that cannot be patched. The source tree's *fixed*
state (what's in the clone, not what's on their mainnet) is better, but still carries the open M8
(SEP-41 non-conformance, permanent) and several open LOW items, and has never had a professional
third-party audit — the same stage our own protocol is at, except our internal audit
(`docs/TRANCHE_1_AUDIT.md`) found two High findings, both fixed, with no unfixed High or Critical
remaining and no live-mainnet exposure to anything found.

There is no version of "take their AMM" that doesn't mean importing a contract whose own authors
call it unsafe for third-party liquidity, over a contract that has already been checked against
the specific way theirs failed and found not to have the same defect. The flash-YT mechanism this
review was actually asked to evaluate is also the mechanism their own audit shows breaking
completely (H1) once any realistic amount of yield has accrued — adopting it would mean importing
the exact feature that doesn't work, instead of building the same capability on a curve already
proven not to have that failure mode.

## 5. Decision: build the true flash swap for YT, on our own AMM

**Built, tested, and verified live — see §5.5 for the implementation record and evidence.** The
design below is kept as written (it is what was actually built), followed by what changed during
implementation and the live results.

The capital-advance limitation this review was asked about is real and worth removing — but the
*mechanism* Sidereal uses to remove it (not their specific code, which has H1/H2/M6) is a sound,
reusable pattern, and Soroban supports it cleanly:

**The pattern.** A contract can author its own narrow, argument-pinned authorization for calls it
makes using its own balances (`env.authorize_as_current_contract`, matched against
`AuthorizedInvocation`s with no wildcard sub-invocations), then invoke another contract using funds
it already custodies — no borrowed capital, no external loan, just the pool spending its own
reserves inside the same atomic transaction as the trade that will immediately recoup them. Their
`flash_split`/`flash_recombine` helpers (`amm/src/lib.rs:1463-1509`) are a clean, minimal
demonstration of exactly this, confirmed sound by their own audit's "Verified sound" section
(*"Every `authorize_self_call` is argument-pinned with empty `sub_invocations`... an EOA cannot
forge it"*) — the mechanism is fine; C1/H1 are bugs in what it's wired up to, not in the mechanism
itself.

**Applied to our protocol**, a true `MarketPool.flash_mint_yt(from, sy_in, min_yt_out, max_net_cost)`
(or an equivalent Router composition) would:

1. Use the pool's **own** SY reserves — not `from`'s balance — to fund the mint leg's `sy_shares`
   argument to `PrincipalManager.mint`, self-authorized the same way `SYWrapper`/`PrincipalManager`
   already self-authorize their own nested calls elsewhere in this codebase (and the way Sidereal's
   `flash_split` does it more explicitly, with narrower, argument-pinned scope worth matching).
2. Keep the minted PT in the pool's own reserves (equivalent to the pool having just bought that
   PT — our `reconcile`/reserve-update pattern from `swap_pt_for_sy` already does this bookkeeping
   step for a sale; this reuses it rather than reinventing it).
3. Charge `from` only the **net cost** (SY advanced minus the PT's curve value, i.e. what today's
   `swap_sy_for_yt` already *reports* as `sy_in - sy_back`) up front, not the full notional.
4. Transfer only `yt_out` to `from`.

This removes the "hold 45x leverage capital" requirement `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`
§4.1 currently documents as a limitation, without touching the curve math this review just
confirmed is already correct across the oracle-rate range that broke Sidereal's version.

**What this review flags as requirements for that build, learned directly from Sidereal's
findings rather than discovered independently**:

- **Test the new path across the same oracle-rate sweep** `sweeping_the_oracle_rate_keeps_the_curve_in_value_units` already runs for every other swap — this is exactly the dimension Sidereal's own property suite missed (pinned to rate = 1.0, "where the defect vanishes identically," their own words) and should be the first thing the new function's test suite does, not an afterthought.
- **Fail closed on a capped fill, never return dust while keeping the trader's funds** (H1's buy-side bug) — any solver-style sizing in the new function must revert `SlippageExceeded`/`InsufficientLiquidity` rather than settle for less than quoted while still taking payment.
- **Refund, never donate, any unspent budget** (H2) — if the pool cannot fully fund the requested `sy_in`'s worth of mint within its own liquidity, the unused portion must return to `from`, not silently accrue to LP reserves as an invisible, unbounded donation.
- **Charge the swap fee on the real SY the curve leg prices, not on the minted PT's leveraged face** (M6) — our existing `fee_rate` application inside `calc_sell_pt`/`calc_buy_exact_pt` already does this correctly (fee is computed on `gross`/`net_shares`, SY-denominated quantities, never on raw PT face), so this is a matter of reusing those same helpers for the new pool-funded leg rather than inventing a new fee computation.

### 5.5 What actually got built, and the bug found while building it

`MarketPool.flash_mint_yt(from, to, sy_in, min_yt_out, max_net_cost) -> (yt_out, net_cost)` plus
two Router wrappers, `flash_mint_yt` (SY-denominated) and `flash_mint_yt_from_underlying` (wraps
only a caller-chosen `underlying_budget`, not the full notional, into SY first). The three
requirements above all held: the new path is covered by its own rate sweep
(`true_flash_mint_stays_correct_across_a_rate_sweep`), a capped/short fill fails closed
(`InsufficientLiquidity`/`SlippageExceeded`, never a short payout), and the fee is computed by
reusing `calc_sell_pt`'s existing SY-denominated fee math unchanged.

**A real, live bug was found and fixed during implementation — not in the curve, in the capital
flow itself.** The first version pulled `from`'s authorized transfer as the mid-execution-computed
`net_cost` directly. Reproduced live on Testnet: the transaction reverted
`InvokeHostFunction(Trapped)` at *submission*, not simulation — the exact `Error(Auth,
InvalidAction)` staleness class already documented in
`docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §2's third gotcha and fixed there for
`add_liquidity_single_sy`. `net_cost` depends on `calc_sell_pt`, which reads `seconds_to_maturity`
live, so the value Soroban had `from` pre-authorize during simulation no longer matched what real
execution computed moments later. Fixed the same way `add_liquidity_single_sy` was: `from`
authorizes transferring the full, *fixed* `max_net_cost` instead (a plain literal with nothing to
go stale), and the function refunds `max_net_cost - net_cost` once the real cost is known — the
same net effect on both balances, reordered into two fixed-amount transfers instead of one derived
one. The full accounting proof is in `flash_mint_yt`'s own doc comment in
`contracts/market_pool/src/lib.rs`.

A second, separate mechanism had to be gotten right for the self-authorized mint call itself:
Soroban's "a contract's direct calls are always considered authorized" only covers the *immediate*
hop. `MarketPool` calling `PrincipalManager.mint(this, sy_in)` is fine on its own, but `mint` in
turn calls `SYWrapper.transfer(this, manager, sy_in)` on `PrincipalManager`'s own frame — two hops
from `MarketPool`'s, which self-authorization does not reach. This also reverted
(`Error(Auth, InvalidAction)`, "encountered unauthorized call for a contract earlier in the call
stack") until `flash_mint_yt` added one explicit `env.authorize_as_current_contract` entry for
exactly that nested call — confirmed against the Soroban SDK's own documented three-contract
example (`soroban-sdk-26.0.1/src/env.rs`), which states only the deepest call needs an entry, not
every hop in between.

**Live evidence** (Testnet, a fresh market, pool seeded 100,000,000 PT / 80,000,000 SY, 1.05
oracle rate): a trader holding only 5,000,000 SY successfully tokenized a 50,000,000 SY notional —
ten times her own balance — receiving 52,473,750 YT for a net cost of 3,213,999 SY (quoted
3,214,008; the nine-unit difference is the same quote-vs-execution ledger-close drift already
documented and accepted for `quote_sy_for_yt` in §5.4 of the integration guide, not a new issue).
Both Router wrappers were verified the same way, including `flash_mint_yt_from_underlying` wrapping
only a 2,000,000-underlying budget against a 10,000,000 SY notional. Every refund matched the
`max_net_cost - net_cost` arithmetic exactly in each run.

## 6. What this review did not do

No code was copied from Sidereal's repository into this one. The clone lives outside this repo (a
scratch directory) and nothing in it was imported, adapted, or referenced by path from any file
here — this document describes what was *read*, for the architectural pattern and the failure
history, not what was reused. Sidereal's own license (Apache-2.0) would permit reuse of their
code; the recommendation not to is a risk judgment about *that specific code's* audit history, not
a licensing constraint.
