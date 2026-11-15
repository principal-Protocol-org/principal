# Tranche 2 design-decision justifications

Dated 15 November 2026. This explains two places where the deployed contracts' actual behavior
reads differently from the original requirement's own wording, from the real source, with line
citations. It does not change any deployed code, and it does not accept anything on SCF's behalf
-- where SCF's own acceptance of an interpretation is actually needed, that is stated plainly as
an open question for the project owner or issuer, not answered here.

All citations below are pinned to commit
[`e30c4761ba058ace455108ade263b49bf53dee44`](https://github.com/principal-Protocol-org/principal/commit/e30c4761ba058ace455108ade263b49bf53dee44)
on `main`, the current revision as of this writing. Both decisions concern logic confirmed
unchanged, line for line in its actual policy, since the commits genuinely deployed to Testnet:
`git diff` against
[`665bb6760372d30307a1a035da24e51776f727e8`](https://github.com/principal-Protocol-org/principal/commit/665bb6760372d30307a1a035da24e51776f727e8)
(the official deployment's own recorded source) shows `contracts/sy_wrapper/src/lib.rs` and
`contracts/router/src/lib.rs` byte-identical to the current revision, and
`OracleAdapter.set_reference_value`'s decrease/deviation policy unchanged in its actual conditions
-- the only difference in that function since `665bb67` is an added internal boolean parameter to
a private helper (`reject`, from a later monitoring-integrity fix) and a new, separate function
appended after it; neither touches what is described below. The TR2 deployment's own commit,
[`aae6cf81346a51f600393e4267be45e65524ae4e`](https://github.com/principal-Protocol-org/principal/commit/aae6cf81346a51f600393e4267be45e65524ae4e),
sits between these two points in history and carries the identical policy either way.

## Decision A: SY deposits during oracle staleness

**What the original requirement says.** Minting and deposits should stop when no fresh oracle
price is available.

**What is demonstrated, from source, as opposed to interpreted below:**

1. **The deposit path that remains available.** `SYWrapper.deposit`
   ([`contracts/sy_wrapper/src/lib.rs#L207-L238`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/sy_wrapper/src/lib.rs#L207-L238))
   checks that the caller isn't paused, is authorized on the underlying SAC, is permitted under
   Principal's own compliance layer, that the deposit amount is positive, that the resulting
   shares clear the caller's own slippage floor, and that the deposit doesn't exceed the
   per-address cap or trip RiskControl's circuit breaker. It does not call `OracleAdapter` at any
   point, directly or indirectly, and can be called by anyone (a wallet, a script, another
   contract) whether or not `OracleAdapter.is_available()` is currently `true`.

2. **What that path actually calculates, and why it needs no oracle.** The shares minted come
   from `underlying_to_shares`
   ([`contracts/sy_wrapper/src/lib.rs#L598-L609`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/sy_wrapper/src/lib.rs#L598-L609)),
   which reads `SYWrapperContract::exchange_rate`
   ([`#L426-L443`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/sy_wrapper/src/lib.rs#L426-L443)
   -- line range approximate past L435, read the function in full at that anchor). That rate is
   purely `TotalUnderlying / TotalShares`, both values this same contract's own storage, updated
   only by this contract's own deposit/withdraw/yield-reporting calls. It is a statement about SY
   itself as a rebasing wrapper over the underlying asset, not a statement about the PT/YT fixed
   rate market, and it is not derived from `OracleAdapter` in any way. There is, correspondingly,
   no `DataKey::Oracle` or any oracle reference anywhere in `sy_wrapper/src/lib.rs` at all --
   confirmed by searching the file, not merely by its absence from the functions read above.

3. **Which downstream operations do require a fresh oracle.** `PrincipalManager.mint`
   ([`contracts/principal_manager/src/lib.rs#L340-L344`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/principal_manager/src/lib.rs#L340-L344))
   calls `Self::assert_oracle_fresh(&env)` as its very first substantive check, before even
   validating the amount. That helper
   ([`#L861-L870`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/principal_manager/src/lib.rs#L861-L870))
   calls `OracleAdapter.is_available()` and panics `Error::OracleStale` if it reads `false`. PT/YT
   minting -- the operation that actually depends on the oracle's rate for its own accounting
   (see Decision B below) -- is the operation this requirement's "minting" half gates, and it is
   gated correctly.

4. **Whether `Router.wrap_and_mint` reverts atomically when minting rejects a stale oracle.**
   `wrap_and_mint`
   ([`contracts/router/src/lib.rs#L483-L500`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/router/src/lib.rs#L483-L500))
   calls `SYClient::new(...).deposit(...)` and then, in the same function, `ManagerClient::new(...).mint(...)`
   -- plain cross-contract calls, not the `try_` variants that would catch a panic. If `mint`
   panics `OracleStale`, that panic propagates up through the host's normal unwind, aborting the
   whole transaction; Soroban transactions have no partial-commit, so the `deposit` call's own
   storage writes (shares credited, the deposit cap counter advanced, the RiskControl volume
   report) are rolled back along with it. Demonstrated by reading the call shape, not run against
   a live stale-oracle scenario in this pass -- the existing test suite already exercises this
   exact path (`contracts/principal_manager/src/test.rs`'s `oracle_stale_blocks_mint` and this
   project's own integration tests cover the combined flow; this document does not re-run them,
   it cites what they already establish about the code's structure). **A bare `SYWrapper.deposit`
   call, made directly rather than through `wrap_and_mint`, is not affected by any of this** --
   it has nothing to revert on, because it never touches the oracle.

**Interpretation, separated from the above.** The risks that remain when direct SY deposits are
allowed during staleness:

- A user who calls `SYWrapper.deposit` directly (bypassing the Router) accumulates SY shares that
  cannot yet be tokenized into PT/YT, since `mint` stays blocked until the oracle recovers. This
  carries no pricing risk at the SY layer itself, because SY's own exchange rate doesn't depend on
  the oracle -- but it does mean exposure measured in SY (not yet PT/YT) can keep growing during
  exactly the kind of incident (a stale or unavailable price feed) that might warrant pausing
  activity more broadly, not narrowly.
- `RiskControl.check_deposit` is still called and still consumes circuit-breaker volume capacity
  during a stale-oracle period, since `deposit` doesn't check oracle freshness before calling it.
  A sequence of direct deposits during an outage could consume meaningful circuit-breaker headroom
  that would otherwise be available once the oracle recovers and minting resumes -- a capacity
  question, not a pricing exploit.
- A literal reading of "deposits should stop" is not satisfied: deposits do not stop. Whether this
  matters in practice depends on whether the requirement meant "every deposit call" or "every
  operation that leads to PT/YT exposure," which the existing wording does not itself resolve.

**Whether frontend restrictions could enforce this for direct contract callers:** no. A frontend
can refuse to construct a deposit transaction while it believes the oracle is stale, but it has no
way to prevent a different caller -- a script, another contract, a developer using the Stellar CLI
directly -- from calling `SYWrapper.deposit` regardless of what any frontend does or doesn't show.
Enforcing this for every possible caller, not just the dApp's own UI, would require a change in the
deployed contract itself (see point 10).

**Why the existing behavior might be an intentional design choice, not an oversight.** SY is
structured as a yield-bearing wrapper whose own correctness (shares in proportion to underlying,
net of accrued yield already reported into it) does not depend on the PT/YT market's price at
all; gating it on that market's oracle would be a stricter rule than SY's own accounting needs,
and would block an operation (joining the SY pool) that doesn't itself create the kind of
price-dependent exposure the requirement's own next clause (minting) is actually protecting
against. Read this way, "minting and deposits should stop" plausibly meant "the operations that
turn an oracle-stale situation into a priced position" -- which is exactly what `mint` is and
`deposit` alone is not. This is this document's own plausible reading of the requirement, offered
because the question asks for it -- it is not a record of anyone having approved this
interpretation.

**Whether the deployed behavior satisfies the original wording literally:** no. "Minting and
deposits should stop" is two clauses; only the first is enforced in the deployed code. Minting is
correctly, verifiably gated; direct SY deposits are not gated at all.

**The precise exception SCF would need to accept, if this is to be treated as acceptable as
deployed:** that "deposits" in the original requirement is read as "deposits that result in PT/YT
minting" (the `wrap_and_mint` path, which is in fact correctly blocked), not literally every call
that can move the underlying asset into SY, and that a bare SY-only deposit carrying no PT/YT
exposure is outside what that clause was meant to restrict. **This is a question for the project
owner or the issuer to answer, together with SCF -- it is not something this document, or anyone
executing it, can decide on SCF's behalf.**

**What a deployed-code change would need to look like for literal compliance (described only, not
implemented, per this task's own scope):** `SYWrapper` currently has no oracle reference of any
kind in its own storage -- confirmed above, this is not merely a missing check but a missing piece
of state. Literal compliance would require: adding an oracle address to `SYWrapper`'s
`initialize`/storage (a new `DataKey::Oracle` or equivalent, set once at construction, the same
shape `PrincipalManager` already uses); calling `OracleAdapter.is_available()` and reverting
`OracleStale` at the top of `deposit`, mirroring `assert_oracle_fresh`'s own pattern exactly; and
deciding, as a product question, whether `withdraw` should be gated the same way or deliberately
left open (an outflow carries a different risk profile than an inflow, and the original
requirement's own wording only names "minting and deposits," not withdrawals). None of this was
implemented in the course of answering this question.

## Decision B: rejection of every oracle decrease

**What the original requirement says.** OracleAdapter should have a price-deviation guard
rejecting updates that move more than ±10% from the previous value.

**What is demonstrated, from source:**

1. **The exact implemented policy.** `OracleAdapter.set_reference_value`
   ([`contracts/oracle_adapter/src/lib.rs#L355-L435`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/oracle_adapter/src/lib.rs#L355-L435),
   the specific branch at
   [`#L415-L429`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/oracle_adapter/src/lib.rs#L415-L429))
   rejects **any** submitted value strictly less than the currently stored price, unconditionally
   -- `value < current_price` alone is sufficient to reject it as `"decrease"`, with no deviation
   percentage computed or consulted at all for that branch. Only when the submitted value is
   greater than or equal to the current one does the ±`max_deviation_bps` check
   ([`#L419-L428`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/oracle_adapter/src/lib.rs#L419-L428),
   default 1000 bps = 10% in this deployment's own configuration) run at all, and it bounds only
   how far the price can rise in one submission, never how far it can fall, because a fall of any
   size is already rejected one branch earlier. This is a strictly asymmetric rule -- "no decrease
   at all, up to +10% increase" -- not the symmetric "±10%" band the requirement's own wording
   describes.

2. **Which PT/YT accounting assumptions depend on this.** `YTToken`'s own module doc
   ([`contracts/yt_token/src/lib.rs#L25-L29`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/yt_token/src/lib.rs#L25-L29))
   states the yield index as `G = INDEX_SCALE * SCALE / rate` -- the reciprocal of the reference
   rate -- and a holder's pending yield as `N * (G_s - G) / INDEX_SCALE`. If `rate` ever decreased,
   `G` would increase, making `G_s - G` negative for anyone already holding YT at the time of that
   decrease -- a negative pending-yield figure, which this accounting model has no representation
   for (YT is designed to accrue only upward, reflecting only the underlying's actual yield
   accumulation, never a loss of principal). The non-decreasing-reference-value invariant is not
   incidental to this design; the yield index's own correctness is defined in terms of it.

3. **What would happen if decreases were accepted without changing the accounting model.** Per
   the point above: a holder's `pending_yield` calculation could go negative, and nothing in the
   current `yt_token`/`principal_manager` code path is written to handle that case --
   `settle`/`claim_yield` assume a non-negative result. Accepting a decrease today, without also
   redesigning this accounting, would either need a new floor/clamp (changing the accrual
   semantics, to decide what happens to yield "already accrued" against a rate that then fell) or
   would risk an arithmetic or logical inconsistency in yield settlement. This was not tested
   against a live decreased-rate scenario in this pass -- the statement above follows from reading
   the accrual formula, not from forcing a decrease through a patched build.

4. **What happens operationally if the real underlying reference value genuinely falls.** The
   stored price simply does not move: every submission below it is rejected as `"decrease"`
   (`reject`'s own bookkeeping records this, but does not change `Price`/`Timestamp`). Because
   `Timestamp` also does not advance on a rejected submission, the oracle's own staleness clock
   keeps running against the last *accepted* timestamp; once `max_stale_seconds` elapses with no
   accepted update, `is_available()`
   ([`#L609-L616`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/oracle_adapter/src/lib.rs#L609-L616))
   starts reading `false`, which then blocks minting (Decision A, point 3) through
   `assert_oracle_fresh`. In other words: a genuine decline in the real rate does not get silently
   absorbed into a wrong, stuck price that users could exploit -- it eventually manifests as a
   stale/unavailable oracle, which blocks the operation that depends on it. This is a real,
   demonstrated consequence of the rejection rule, not a hypothetical.

5. **Whether the policy can leave the oracle stuck stale, or operations blocked, with no in-protocol
   remedy.** Searching every mutating function on `OracleAdapter`
   ([`contracts/oracle_adapter/src/lib.rs`](https://github.com/principal-Protocol-org/principal/blob/e30c4761ba058ace455108ade263b49bf53dee44/contracts/oracle_adapter/src/lib.rs),
   `initialize`, `set_sources`, `set_safety_params`, `set_reference_value`, `sync_from_feed`,
   `transfer_admin`) confirms there is no admin-only override that can force the stored `Price`
   downward once set -- not even the admin can submit a decrease through any path. The only ways
   out of a "stuck high, now stale" state are (a) for the real rate to recover back above the
   stuck value, at which point a normal submission is accepted again, or (b) deploying an entirely
   new `OracleAdapter` instance and, because `PrincipalManager`/`MarketPool`/`YTToken` bind their
   oracle address immutably at `initialize` with no setter (confirmed in an earlier round of this
   same evidence work, independently, by an external review), recreating the whole market around
   it. Yes, this policy can leave operations blocked with no narrower fix available than that.

6. **Which asset assumptions make this policy appropriate, and its limits.** The requirement's own
   context (Tranche 3 Deliverable 3) names the target asset as USDY, a yield-bearing stablecoin
   whose reference rate is expected to be monotonically non-decreasing by the asset's own design
   (it accrues yield; it is not expected to mark down in the way a floating-price asset would). For
   an asset with that property, "reject every decrease" and "reject it because real declines
   shouldn't happen" are nearly the same policy in practice, and the asymmetry costs little. The
   limitation is exactly that asset-specific assumption: for a different underlying whose fair
   value can legitimately fall (almost anything other than an accruing yield-bearing instrument),
   this same policy would convert every ordinary downward price movement into a stale,
   blocked-from-minting oracle, not a correctly-updated lower price -- which would not be a safety
   feature, it would be an availability failure masquerading as one.

7. **Why this is stricter than a symmetric ±10% policy.** A symmetric policy would let the price
   fall by up to 10% per submission, same as it can rise. This implementation lets it fall by
   exactly 0% per submission (any amount of decrease is rejected, not just one exceeding 10%) while
   still allowing a rise of up to 10%. It is strictly more restrictive on the downside than the
   literal wording describes, not merely differently calibrated.

**Whether SCF must explicitly accept that interpretation:** yes, if "±10% deviation guard" is read
to mean a policy that is supposed to be symmetric, the deployed code does not implement that --
it implements a stricter, asymmetric rule for a specific, stated reason (point 2 above, the
non-decreasing accounting model). **Whether that reason is an acceptable substitute for literal
compliance is a judgment call for the project owner, the issuer, and SCF together, not something
resolved by this document.**

**Proposed acceptance wording, offered for SCF's own consideration, not as an accepted decision:**
*"OracleAdapter's deviation guard is accepted as implemented -- rejecting any decrease outright and
bounding increases at the configured ±10% -- on the basis that PT/YT settlement correctness
depends on a non-decreasing reference value (see YTToken's own yield-index design), and that the
initially targeted USDY market's own expected rate behavior (monotonically non-decreasing by
design) makes this asymmetry low-cost in practice. This acceptance is scoped to assets with that
same non-decreasing-rate property; a future market on an asset whose fair value can legitimately
fall would need either a different policy or an explicit, separate risk acceptance before launch."*

## Questions that need the project owner's or issuer's own answer, not supplied here

- Is "deposits" in Tranche 2 Deliverable 1's requirement meant to cover every `SYWrapper.deposit`
  call, or only ones that lead to PT/YT minting? This decides whether Decision A's current
  behavior is accepted as-is, or whether the code change described in Decision A's final point
  needs to be built before this deliverable can be called complete against its own literal wording.
- Does the project intend to launch Principal against any underlying asset whose fair value can
  legitimately decline (not a monotonically-accruing instrument like USDY)? If yes, Decision B's
  policy needs to be revisited for that asset specifically before launch, not merely accepted as
  written for USDY.
- If `SYWrapper.deposit` is gated on oracle freshness per Decision A's described fix, should
  `SYWrapper.withdraw` be gated the same way, left open, or decided separately? The original
  requirement's own wording does not mention withdrawals at all.

## What this document does not do

It does not implement either described code change. It does not constitute SCF's acceptance of
either interpretation -- the proposed wording above is a draft for SCF's own consideration, not a
record of approval. It does not revisit or change the stale-oracle deposit behavior or any other
protocol functionality, consistent with this task's own stated scope boundary.
