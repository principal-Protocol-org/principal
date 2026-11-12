# Router Quotes for Underlying-Denominated Flows

Closes the one remaining gap in the underlying-denominated trading work (§4.1 of
`docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`): previewing `swap_underlying_for_pt`,
`swap_pt_for_underlying`, `swap_underlying_for_yt`, `swap_yt_for_underlying`, or
`add_liquidity_keep_yt` meant calling `MarketPool`'s own SY-denominated quote and separately
calling `SYWrapper.exchange_rate()` to convert by hand. Five new, read-only `Router` functions
compose the two calls into one answer, each in the same units the real call's caller actually
pays or receives.

## 1. The five functions

| Function | Mirrors | Returns |
|---|---|---|
| `quote_underlying_for_pt(pool, underlying_amount)` | `swap_underlying_for_pt` | `Quote{amount_out, amount_in, fee_shares}` — `amount_out` is PT; nothing on the output side needs converting, only the input (`underlying_amount` → SY shares) feeds into the pool quote |
| `quote_pt_for_underlying(pool, pt_in)` | `swap_pt_for_underlying` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` — both converted to underlying |
| `quote_underlying_for_yt(pool, underlying_amount)` | `swap_underlying_for_yt` | `UnderlyingFlashMintQuote{yt_out, underlying_back, mint_fee_underlying, swap_fee_underlying}` |
| `quote_yt_for_underlying(pool, yt_in)` | `swap_yt_for_underlying` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` |
| `quote_add_liquidity_keep_yt(pool, underlying_amount)` | `add_liquidity_keep_yt` | `KeepYtQuote{mint_amount, pt_desired, sy_desired, lp_estimate, yt_out, mint_fee_underlying}` |

Each is pure composition: a `MarketPool` quote this protocol already shipped and verified
(`quote_sy_for_pt`, `quote_pt_for_sy`, `quote_yt_for_sy`, `quote_sy_for_yt`) plus one
`SYWrapper.exchange_rate()` read, applying the exact same `underlying * RATE_SCALE / rate` /
`shares * rate / RATE_SCALE` formula `SYWrapper.deposit`/`withdraw_all` themselves use. No new
on-chain math was invented for this change.

`quote_add_liquidity_keep_yt` returns `mint_amount`, `pt_desired` and `sy_desired` to pass
directly into the real call's own arguments of the same names, plus `lp_estimate` (a preview of
`add_liquidity`'s own `lp_minted`) and `yt_out` from the mint leg.

**Update, a later integration review**: this quote originally derived `mint_amount` from
`MarketPool.zap_swap_amount` — the wrong formula. `zap_swap_amount` solves for the ratio
`add_liquidity_single_sy`'s zap needs (PT *bought* from the pool at the curve price); Keep YT's PT
is *minted* 1:1 (minus the tokenization fee) at the oracle rate instead, a different ratio
entirely, so reusing it generally left SY stranded in the subsequent `add_liquidity` call rather
than fully used. Fixed by a new `MarketPool.keep_yt_split`, which bisects for the `mint_amount`
that actually balances minted PT against remaining SY in the pool's raw reserve ratio — the ratio
`add_liquidity`'s own proportional deposit targets. The real call (`add_liquidity_keep_yt`) was
fixed the same way at the same time, and for an independent reason: it used to derive
`sy_remaining` from `deposit`'s live-exchange-rate-dependent output and `pt_desired` from
`mint`'s live-oracle-rate-dependent output, then pass both straight into `add_liquidity`'s own
authorized call — exactly the `Error(Auth, InvalidAction)` drift class `swap_pt_for_underlying`
hit (§2 below describes the mechanism for `SYWrapper`; the same mechanism applies to any
live-state-derived value, including `PrincipalManager.mint`'s oracle-rate-dependent notional).
Both `mint_amount` and the now-separate `pt_desired`/`sy_desired` are literal, pre-quoted
arguments on the real call today, with no such drift to go stale.

`lp_estimate` previews `add_liquidity`'s own `lp_minted` via the identical
`(pt_desired * total_lp / pt_reserve).min(sy_desired * total_lp / sy_reserve)` formula, reading
`reserves()`/`lp_total_supply()` the same way the real call's own internals do. Like every quote
in this protocol, it is exact only if reserves/`total_lp` do not move between this read and the
real call — see `contracts/integration_tests/tests/liquidity_and_router.rs`'s
`router_quotes_for_underlying_denominated_flows_match_the_real_outcome_exactly`, which asserts
`lp_estimate` against the real `lp_minted` with no tolerance, in a sequence with no intervening
trade.

## 2. Exactness, proven, not just tested

Every other quote in this protocol (`MarketPool`'s own, `docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md`
§5.4) carries the caveat that real time passing between the quote and the real call can shift the
answer by rounding dust, because `MarketPool`'s curve depends on `seconds_to_maturity`. These five
Router quotes carry no such caveat for their `SYWrapper` leg, and that claim is provable, not just
empirically true in the numbers this change happened to test with.

**Claim**: under the current `SYWrapper` contract, `exchange_rate()` is invariant at exactly
`RATE_SCALE` (1.0) for the entire lifetime of any instance.

**Why**: `exchange_rate()` reads two stored integers, `TotalUnderlying` and `TotalShares`, computed
as `TotalUnderlying * RATE_SCALE / TotalShares` (or `RATE_SCALE` itself, before the first deposit).
Exactly three functions ever write either field: `deposit`, `do_withdraw` (backing both `withdraw`
and `withdraw_all`), and `seize` — and `seize` explicitly does not (`seize`'s own doc comment: "a
pure forced transfer, not a burn... `TotalShares`/`TotalUnderlying` are unaffected"). `deposit`
computes `shares = amount * RATE_SCALE / rate` *before* mutating state, then sets
`TotalUnderlying += amount` and `TotalShares += shares`. Starting from the bootstrap case
(`TotalShares == 0` -> `shares = amount`, `rate` defined as `RATE_SCALE`), every subsequent
deposit at `rate == RATE_SCALE` computes `shares = amount * RATE_SCALE / RATE_SCALE = amount`
exactly — no remainder, ever, because `RATE_SCALE` divides `amount * RATE_SCALE` exactly for any
integer `amount`. So `TotalUnderlying` and `TotalShares` increase by the *same* amount on every
deposit, preserving the 1:1 ratio exactly. The identical argument holds for `do_withdraw`'s
`underlying_out = shares * rate / RATE_SCALE` at `rate == RATE_SCALE`. By induction, the ratio
can never move away from exactly 1.0 once it starts there — and there is no fourth function (no
rebase, no sync, no harvest) that pulls in yield independent of a deposit to move it in the first
place. (A worked counterexample confirms the general case *would* matter if the ratio ever started
off a clean multiple of `RATE_SCALE`: starting from `TotalUnderlying = 1000`,
`TotalShares = 999` gives `rate_A = 10,010,010`; depositing `7` gives `shares = 6`
(`70,000,000 / 10,010,010` floored), and the *new* rate,
`1007 * 10,000,000 / 1005 = 10,019,900`, differs from `rate_A` — this is exactly the scenario that
would make a quote reading the rate once, for two conversions straddling that deposit, diverge
from a real call that reads it twice. It cannot happen today only because nothing in this codebase
can ever get `SYWrapper`'s ratio away from exactly 1.0 to begin with.)

**Consequence**: `quote_underlying_for_pt` and `quote_add_liquidity_keep_yt` each only ever read
`exchange_rate()` once and use it for a single conversion, so this question does not apply to them
regardless. `quote_pt_for_underlying` and `quote_yt_for_underlying` each correspond to a real call
with exactly one `SYWrapper` state mutation (one `withdraw_gained` -- `withdraw_all` originally,
switched for the unrelated reason in the "Update" below -- nothing before it), so there is
only ever one rate to read, and the quote and the real call necessarily read the same stored
integers. `quote_underlying_for_yt` is the only one of the five whose real call
(`swap_underlying_for_yt`) has *two* separate `SYWrapper` reads straddling a mutation (`deposit`,
then later `withdraw_gained`) — and per the proof above, those two reads are provably identical
under the current contract, so the quote (which reads the rate once and reuses it) still matches
exactly. (The `withdraw_all` -> `withdraw_gained` switch changes nothing about this argument: both
read the same `exchange_rate()`-derived formula, just starting from a different balance.)

**Update, a later dApp-integration review**: `swap_pt_for_underlying`, `swap_yt_for_underlying` and
`swap_underlying_for_yt` no longer use `withdraw_all` at all -- they read a `baseline` before the
mutating step and use `SYWrapper.withdraw_gained` instead, so a caller who already holds unrelated
SY is no longer rejected outright (the earlier `withdraw_all`-based versions required `from` to
hold *zero* pre-existing SY, which failed routinely in practice). This has no effect on the
exactness argument above or on `quote_pt_for_underlying`/`quote_yt_for_underlying`/
`quote_underlying_for_yt`'s own correctness: `baseline` is read once per call, at a point before
any `SYWrapper` mutation, and does not change how many times `exchange_rate()` itself is read or
what it is read against.

**This is a forward-looking note, not a live bug**: if `SYWrapper` is ever extended with a
rebase/sync capability (to track a genuinely rebasing underlying's *live* balance rather than its
own bookkeeping total), the induction above breaks, and `quote_underlying_for_yt` specifically
would need to either read the rate twice (mirroring the real call) or be re-audited against
whatever the new mechanism does. Flagged here so it isn't rediscovered the hard way later.

## 3. Test coverage

`contracts/integration_tests/tests/liquidity_and_router.rs`:
`router_quotes_for_underlying_denominated_flows_match_the_real_outcome_exactly` quotes each of the
five flows, then runs the real call, and asserts exact equality on every comparable field
(`amount_out`, `yt_out`, `underlying_back`, and now `lp_estimate` against `lp_minted`) — not an
approximate/tolerance check. A separate assertion confirms `quote_add_liquidity_keep_yt`'s
`mint_amount + sy_desired` accounts for the full wrapped amount. `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace` all
pass clean.

## 4. What this does not cover (by choice)

No fresh Testnet deployment was made to verify these five quotes live. Unlike the earlier
underlying-denominated Router functions and the `MarketPool` YT quotes (both verified live earlier
this session), this change adds no new state-changing logic and no new rounding formula — it is a
read-only composition of two primitives (`MarketPool`'s quotes, `SYWrapper.exchange_rate()`) each
already verified live independently, combined with the exactness proof in §2, which covers every
possible input under the current contract rather than the handful of values a live run would have
sampled. A live deployment would need a fresh `Router` instance (the currently-deployed one predates
these functions and Soroban contract code cannot be upgraded in place), re-registering an existing
market into it, for no additional assurance over what §2's proof and §3's exact-equality test
already give.
