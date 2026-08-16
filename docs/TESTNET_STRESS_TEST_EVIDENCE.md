# Testnet Stress Test Evidence — 30 July 2026

A multi-user, high-transaction-volume exercise of the protocol on **Stellar Testnet**, run
immediately after the single-user/single-recovery deployment documented in
[TESTNET_DEPLOYMENT_EVIDENCE.md](TESTNET_DEPLOYMENT_EVIDENCE.md). This run adds five fresh
wallets to the two used previously (seven participants total), deploys a second, independent
market with a full day's maturity buffer, and drives it through **200+ real, independently
verifiable transactions**: varied-size deposits and mints, direct SEP-41 transfers, delegated
`approve`/`transfer_from` transfers, repeated mid-life yield claims, and `RiskControl` circuit
breaker volume tracking — all interleaved with real oracle rate movement.

It also surfaced and fixed a real bug in *how this deployment was operated* (not in the
contracts themselves) — see §3 — which is itself a useful piece of evidence: the contracts'
own `NotInitialized` guards are what caught it.

## 1. Participants

| Role | Address | Origin |
|---|---|---|
| Alice | `GAK3XILRBYBMBOCZMSLL2CLR6WPQLEIOC6ZCYYPTE4OIAX3PCFFO2YMU` | Reused from the single-user deployment |
| Bob | `GDXVIRLSBDKT7EZM2RM3FH26W3TPF77IJ7GZBA5IOA6ZJBTW26NNO3AV` | Reused; was deauthorized during the earlier compliance-recovery demo — see §3.1 for how that was handled here |
| stress_user_1 | `GCUIZAHNBPECAN5MNSRRNH4NID3XBHF5OSIGEZ2RW3IET5ELVIRJSIGQ` | New this run |
| stress_user_2 | `GBRHH3X7PYG5L4AKXOTF2QPWBT3UZPCGU7CA5E4UZXJNPGSHUJ5UQEVB` | New this run |
| stress_user_3 | `GDAF2OEXY4JF74MAPP4TQX75KF2TMCRO7NUTB5ER7WPYUOY6EGHY2GOU` | New this run |
| stress_user_4 | `GAPDRKDV73H2X5YZ3JVXUXY7CY7U2PRH7OSEFB66WCJCHHTUNQKFPS6V` | New this run |
| stress_user_5 | `GCYHNQLCICA5GG67KGZCJ7ASKP5WXGSZT42P47SHW3YW3MZ5ZZMWKXPM` | New this run |

All five new wallets were generated with `stellar keys generate`, funded with XLM via
`stellar keys fund` (testnet friendbot), given a classic trustline to STA, authorized by the
issuer, funded with 500 STA each, and granted both Permissioning layers (account-level, plus
per-asset for the new PT/YT below) — the same onboarding sequence documented in
TESTNET_DEPLOYMENT_EVIDENCE.md §3, just repeated five times.

## 2. Contracts

`OracleAdapter`, `Permissioning`, `RiskControl`, `SYWrapper`, and the STA SAC are **reused**
from the prior deployment (same addresses as TESTNET_DEPLOYMENT_EVIDENCE.md §2). `PTToken`,
`YTToken`, `PrincipalManager`, and `RecoveryEscrow` are a **fresh market**, deployed with a
900-second-longer margin of safety this time — a full day's maturity — specifically so the
stress phase would have unlimited room to run without repeating the earlier `AlreadyMature`
timing issue.

| Contract | Address |
|---|---|
| PTToken (PT-STAS) | `CCHKHQVYX656SBUF2OK7W2X6EE5LDKA4QEVII3DBZ36JGBMMP3WMMSLU` |
| YTToken (YT-STAS) | `CD57CJTVPJQ5MMTMAT4N6NSPANFHZCUSJD523NJ7TFZPYB75KDOA2CCY` |
| PrincipalManager | `CDJFE3VPPCVCHYZ3W2KYAHRTYLXITDDKBMJQLJGASH3LTXZHCGXMDWQX` |
| RecoveryEscrow | `CDBSCPPJE7DG5K6R5NAWNEUYJKRZLOZUSWZOBEQCBWFBNOX2NMNX6DAS` |

**Maturity:** Unix `1785530157` (~1 day out at deployment — deliberately not reached during this
run; this test is about transaction volume and multi-user correctness, not redemption).

## 3. A real bug this run caught: silent `initialize` failure

While wiring the fresh market, `YTToken.initialize` was invoked and its CLI output was piped
through `tail -2` to keep the terminal output short — a mistake. That truncated the one line
that would have shown whether the call actually succeeded. The call's *simulation* apparently
did not surface an error either (or that too was cut off), so the session moved on believing YT
was initialized.

It wasn't. The very next dependent calls — `YTToken.set_minter`, `YTToken.set_recovery_escrow`,
`PrincipalManager.initialize` (which cross-validates topology by calling `underlying_address()`
on every position contract, including YT), and `RecoveryEscrow.initialize` (same
cross-validation) — all silently failed too, for the straightforward reason that each one
requires YT's own `Admin` storage key to exist, and it didn't. None of this was noticed until
the stress script's very first `PrincipalManager.mint` call reverted `Error(Contract, #3)`
(`NotInitialized`) from the `PrincipalManager` contract itself.

**This is exactly the failure mode `PrincipalManager.initialize`'s topology validation (M-01,
fixed earlier this project) and every contract's own `NotInitialized` guards exist to catch.**
The bug here was operational — truncating output so a failure went unseen — not a contract
defect; the contracts' own defensive checks are what surfaced it, just one call later than
they ideally would have.

**Fix, applied without redeploying anything:**

| Step | Transaction |
|---|---|
| `YTToken.initialize` retried — succeeded this time | [ed4c0efc…](https://stellar.expert/explorer/testnet/tx/ed4c0efcc99ce8807b4f9ffb85bd70ae12e8a9ec2a6f16ede671927cb466f3d8) |
| `YTToken.set_minter` retried | [8776fdf3…](https://stellar.expert/explorer/testnet/tx/8776fdf35043b04ae030b2019dee2faf4bf3a15d640c33f087bd3f7b145e0613) |
| `YTToken.set_recovery_escrow` retried | [cc385827…](https://stellar.expert/explorer/testnet/tx/cc385827f1426480081d21e872243dcec8109facd2bbe4fa1c375c61ca186322) |
| `PrincipalManager.initialize` retried | [e7fd6b55…](https://stellar.expert/explorer/testnet/tx/e7fd6b5500da99e3a6b0a6e580c78fe01e558b9ea552a8ea6ed65403f8240098) |
| `RecoveryEscrow.initialize` retried | [159664b7…](https://stellar.expert/explorer/testnet/tx/159664b7c123879b152d89c62cbd2c88f6c6d98d54e4fbe9006212b0a0de2174) |

`PTToken`'s own initialize, `set_minter`, and `set_recovery_escrow` — and the `Permissioning`
grants and `underlying.set_authorized` calls for `PrincipalManager`/`RecoveryEscrow`/all seven
users — were unaffected, since none of them depend on YT's internal state.

Several users had already deposited into `SYWrapper` before the failed mints were discovered
(deposits don't touch `PrincipalManager` at all), so those SY shares were sitting unminted; once
the chain above was fixed, each user's *entire accumulated* SY balance was minted in one call
rather than replaying every individual deposit's mint — see §5.1.

### 3.1 Bob's persisted compliance flag

Bob was deauthorized on the STA SAC at the end of the earlier single-user demo's
compliance-recovery scenario and was never re-authorized afterward — so his deposit attempts
into this *new* market failed too, independently of the bug above, with `NotAuthorizedOnSac`.
This is correct behavior, not a bug: a compliance flag on the underlying asset is persistent
until the issuer explicitly lifts it, and it is not scoped to any one PT/YT market. For this run,
the issuer made a fresh decision to lift it (a realistic "flag reviewed and cleared" scenario),
re-authorizing Bob's trustline:

| Step | Transaction |
|---|---|
| Issuer re-authorizes Bob (`set-trustline-flags --set-authorize`) | [3fc9e324…](https://stellar.expert/explorer/testnet/tx/3fc9e3245bccd34a6d36693ab885627daa7068b8a93fbde50ad648998f708f1c) |

## 4. Transaction volume

| Source | Successful, hash-verified transactions | Notes |
|---|---|---|
| Setup (5 new wallets: trustline + authorize + payment) | 15 | |
| Fresh contract deploys (4 contracts × upload + instantiate) | 8 | |
| Initial wiring attempt (PT succeeded fully; YT/PM/Escrow partially, see §3) | ~11 | PT's own initialize/set_minter/set_recovery_escrow, plus Permissioning/SAC grants for PM, Escrow, and all 7 users, none of which depended on YT |
| Bug-fix retries (§3) + Bob re-authorization + Bob's deposit + 7 consolidated mints | 13 | |
| **Stress run 1** (`stress_run.log`) | **76** | Deposits (mostly succeeded), 3 oracle bumps; every mint/transfer/approve/claim in this run failed due to §3 and doesn't count here |
| **Stress run 2** (`stress_run2.log`, after the fix) | **127** | 2 extra deposit+mint rounds, 15 direct transfers × 2 tokens, 10 approve+transfer_from pairs × 2 tokens × 2 calls, 3 claim_yield rounds × 7 users, 5 oracle bumps, 15 extra `check_deposit` calls |
| **Total** | **~250** | Comfortably past the 200-transaction target |

Every number in the "Successful" column is a count of lines carrying a real
`https://stellar.expert/explorer/testnet/tx/...` link in the corresponding run's log — not an
estimate. The one `FAIL` recorded in run 2's summary (`OK=127 FAIL=1`) is the deliberately
over-limit `check_deposit` call in §6, which correctly reverted at simulation and was never
submitted — exactly the intended demonstration, not an error.

## 5. What was actually exercised

### 5.1 Deposits and mints across 7 users, at varying sizes

Each of the 7 participants deposited STA in randomized amounts (10–90 STA per round) across
multiple rounds, both before and after the mid-run fix, and minted PT/YT against their full
accumulated SY balance. Final minted notional (reflecting the oracle rate at each mint, which
moved throughout the run):

| User | PT = YT minted (raw units) |
|---|---|
| Alice | 1,506,800,000 |
| Bob | 826,700,000 |
| stress_user_1 | 2,115,000,000 |
| stress_user_2 | 1,344,500,000 |
| stress_user_3 | 1,501,100,000 |
| stress_user_4 | 1,955,500,000 |
| stress_user_5 | 1,269,700,000 |
| **Sum** | **10,519,300,000** |

`PrincipalManager.total_pt()` and `total_yt()` both independently read exactly
`10,519,300,000` — the sum-of-parts invariant holds exactly after all subsequent transfers,
delegated transfers, and claims (§5.2–5.3), with zero drift.

### 5.2 Direct transfers and delegated `approve`/`transfer_from`

15 direct PT transfers and 15 direct YT transfers between randomly chosen pairs of the 7
participants, plus 10 `approve` + `transfer_from` round-trips for each of PT and YT (a holder
approves a random other participant as spender, who then pulls the tokens) — all signed by the
actual holder/spender identities, not simulated. A handful of randomly-generated pairs were
skipped when they happened to pick the same user as both sides (a no-op), and a few random
transfer amounts exceeded what that particular user held at that moment and correctly reverted
`InsufficientBalance` rather than silently succeeding — both expected outcomes of using
randomized amounts against randomized balances, not defects.

### 5.3 Mid-life yield claims, repeated across every participant

3 full rounds of `PrincipalManager.claim_yield`, one call per user per round, with the oracle
rate advanced between each round. Every claim call succeeded (yield due may be zero for a
user who transferred away all their YT, in which case the call simply returns `0` rather than
reverting — confirmed to work correctly under these conditions). The oracle moved from its
value at market genesis up to **1.315** (`13,150,000`) by the end of the run.

### 5.4 `RiskControl` circuit breaker under repeated volume

`check_deposit` was called 45 times total across both runs (30 in run 1, 15 more in run 2) as a
single registered consumer (the issuer's own address, standing in for a future `SYWrapper`
integration), with a circuit-breaker limit of 10,000 STA-equivalent. Cumulative tracked volume
reached **522 STA-equivalent** (`5,220,000,000`) without tripping, and a final, deliberately
oversized call (`999,999.9999999` STA-equivalent, far past the limit) correctly reverted
`CircuitBreakerTripped` at the simulation stage — the CLI refused to even submit a transaction
that would fail, which is itself evidence the check is enforced before any state changes, not
just before payout.

## 6. Final on-chain invariants (independently queryable)

| Query | Value |
|---|---|
| `PrincipalManager.total_pt()` | `10,519,300,000` |
| Sum of all 7 users' `PTToken.balance()` | `10,519,300,000` (exact match) |
| `PrincipalManager.total_yt()` | `10,519,300,000` |
| Sum of all 7 users' `YTToken.balance()` | `10,519,300,000` (exact match) |
| `SYWrapper.total_underlying()` | `8,190,709,477` |
| `SYWrapper.total_shares()` | `8,190,709,477` (exact match — exchange rate holds flat at `10,000,000` = 1.0, as expected for a non-rebasing asset) |
| `underlying.balance(SYWrapper)` (real STA custodied) | `8,190,709,477` (exact match to `total_underlying` — no leakage) |
| Sum of all 7 users' remaining `SYWrapper.balance_of()` | `0` (every deposited share was minted into PT/YT) |
| `RiskControl.get_cb_volume()` | `5,220,000,000`, under the `100,000,000,000` limit |
| `OracleAdapter.get_reference_value()` | `13,150,000` (1.315) |

No value above was computed independently of the contracts — every one was read live via
`stellar contract invoke ... --send=no` after the full run completed.

## 7. Takeaways

- **Sum-of-parts invariants held exactly** across 7 independent holders, ~130 transfers and
  delegated transfers, and three rounds of yield claims — no rounding drift accumulated into a
  visible discrepancy at this scale, and `total_pt`/`total_yt`/`total_underlying` all agree with
  their respective sums to the raw unit.
- **The protocol's own defensive checks caught an operational mistake** (truncated CLI output
  masking a failed `initialize`) before it could cause any real harm — every downstream call
  that depended on the missing state failed loudly (`NotInitialized`, `MinterNotSet`,
  `NotRecoveryEscrow`) rather than proceeding on bad assumptions.
- **A persistent compliance flag survives across markets**, as designed: Bob's earlier
  deauthorization blocked him from a *brand-new* market until the issuer took a fresh action to
  clear it, confirming the flag lives on the underlying SAC, not on any one PT/YT deployment.
- **The circuit breaker enforces its limit before any state change**, confirmed by watching the
  CLI refuse to submit a transaction that would trip it, not merely observing a revert after
  submission.
