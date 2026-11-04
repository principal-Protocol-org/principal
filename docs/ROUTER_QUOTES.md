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
| `quote_add_liquidity_keep_yt(pool, underlying_amount)` | `add_liquidity_keep_yt` | `KeepYtQuote{mint_amount, sy_remaining, yt_out, mint_fee_underlying}` |

Each is pure composition: a `MarketPool` quote this protocol already shipped and verified
(`quote_sy_for_pt`, `quote_pt_for_sy`, `quote_yt_for_sy`, `quote_sy_for_yt`, `zap_swap_amount`) plus
one `SYWrapper.exchange_rate()` read, applying the exact same
`underlying * RATE_SCALE / rate` / `shares * rate / RATE_SCALE` formula `SYWrapper.deposit`/
`withdraw_all` themselves use. No new on-chain math was invented for this change.

`quote_add_liquidity_keep_yt` returns the `mint_amount` to pass directly into the real call's own
`mint_amount` argument — the thing a caller actually needed `exchange_rate()` for before this
change — plus `yt_out` from the mint leg. It does **not** predict `add_liquidity`'s own
`pt_used`/`sy_used`/`lp_minted`: no quote for that exists anywhere in this protocol, not even for
the plain `add_liquidity`/`add_liquidity_single_sy` calls it composes with, since that is ordinary
proportional-ratio arithmetic against reserves that can move before the real call lands regardless
of what any quote says.

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
with exactly one `SYWrapper` state mutation (one `withdraw_all`, nothing before it), so there is
only ever one rate to read, and the quote and the real call necessarily read the same stored
integers. `quote_underlying_for_yt` is the only one of the five whose real call
(`swap_underlying_for_yt`) has *two* separate `SYWrapper` reads straddling a mutation (`deposit`,
then later `withdraw_all`) — and per the proof above, those two reads are provably identical under
the current contract, so the quote (which reads the rate once and reuses it) still matches exactly.

**This is a forward-looking note, not a live bug**: if `SYWrapper` is ever extended with a
rebase/sync capability (to track a genuinely rebasing underlying's *live* balance rather than its
own bookkeeping total), the induction above breaks, and `quote_underlying_for_yt` specifically
would need to either read the rate twice (mirroring the real call) or be re-audited against
whatever the new mechanism does. Flagged here so it isn't rediscovered the hard way later.

## 3. Test coverage

`contracts/integration_tests/tests/liquidity_and_router.rs`:
`router_quotes_for_underlying_denominated_flows_match_the_real_outcome_exactly` quotes each of the
five flows, then runs the real call, and asserts exact equality on every comparable field
(`amount_out`, `yt_out`, `underlying_back`) — not an approximate/tolerance check. A separate
assertion confirms `quote_add_liquidity_keep_yt`'s `mint_amount + sy_remaining` accounts for the
full wrapped amount. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked
-- -D warnings`, and `cargo test --workspace` all pass clean.

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
