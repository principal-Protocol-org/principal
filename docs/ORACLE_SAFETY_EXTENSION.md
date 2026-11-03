# OracleAdapter Safety Extension

Closes the contract-side half of Tranche 2 Deliverable 1 (`docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md`
§5, §14.1): primary/fallback source configuration, a deviation guard, and an explicit
"is a usable price available" signal for every oracle-consuming contract. Everything else that
deliverable needs — the scripted deployment pipeline, the public Testnet dApp, the live E2E
suite — is tooling and frontend work outside this document's scope; this covers only the contract
change and its live verification.

## 1. What was there before

`contracts/oracle_adapter` (Tranche 1) was a single-writer feed: `initialize(admin)`,
`set_reference_value(caller, value, timestamp)`, `is_fresh(max_stale_seconds)`. It enforced a
monotonically increasing timestamp and non-decreasing value — required because the entire PT/YT
settlement model assumes the reference rate never falls (`docs/SECURITY.md` §1, §2) — but had no
concept of multiple sources, no deviation guard, and no way for a caller to ask "is this feed
currently usable" without first picking its own staleness threshold.

## 2. What this extension adds

| Addition | Why |
|---|---|
| `PrimarySource`/`FallbackSource`/`ActiveSource`, set via `set_sources` | The funded text requires a primary plus an optional fallback, each a deterministic `BytesN<32>` ID rather than a free-form string on-chain |
| `MaxStaleSeconds`/`MaxDeviationBps`, set at `initialize` and updatable via `set_safety_params` | Centralizes the staleness policy in the oracle itself — previously duplicated as a `MAX_ORACLE_STALENESS_SECS` constant in three separate consumer contracts (`MarketPool`, `PrincipalManager`, `YTToken`), each of which could, in principle, drift out of sync |
| `is_available()` | `is_fresh` against the oracle's own configured threshold, so a consumer needs no constant of its own |
| `assert_available()` | A one-hop panic-propagating check (`OracleUnavailable`) for a caller that wants a crisp revert instead of reading a boolean |
| A deviation guard: reject an increase of more than `max_deviation_bps` | The funded text's ±10% guard against a compromised or malfunctioning feed pushing a large upward jump in one tick. Decreases are already caught first by the existing `ValueDecreased` check, so this only ever fires on an outsized increase |
| `last_rejection()` / `LastRejectedValue`/`Timestamp`/`Reason` / `ref_rej` event | A public dashboard needs to show "why was the last update rejected" from a contract read, not from a private relay's own log (`docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md` §10.5) |
| A fallback that may only refresh the timestamp, never the value | The funded text's "recommended conservative policy" §10.3: primary is the sole source of truth for price; fallback exists only to keep the feed from reading stale when primary is briefly unavailable |

New errors, appended after the existing seven rather than renumbering them (`AlreadyInitialized`=1
… `TimestampInFuture`=7, unchanged): `OracleUnavailable`=8, `DeviationTooHigh`=9,
`InvalidSource`=10, `InvalidDeviation`=11, `InvalidStaleness`=12, `FallbackNotConfigured`=13.

## 3. Hard failures vs. graceful rejections — and why both exist

This is the one design decision in this extension that isn't a direct transcription of the spec,
because Soroban makes the spec's literal ask ("store why this was rejected, then abort") impossible
in a single call: a panicking invocation reverts *every* storage write and discards *every* event
from that invocation, so there is no way to both revert and persist a rejection record in the same
transaction.

Two classes of bad `set_reference_value` call are handled differently as a result:

- **Malformed or misconfigured calls panic** — `Unauthorized`, `InvalidValue` (non-positive),
  `TimestampInFuture`, `InvalidSource`, `FallbackNotConfigured`, and the config-time
  `InvalidStaleness`/`InvalidDeviation` on `initialize`/`set_safety_params`. These indicate a
  broken caller, not a legitimate price tick, so they revert outright exactly like every other
  malformed call in this codebase.
- **A syntactically valid submission from a recognized source that policy simply rejects does not
  panic** — `ValueDecreased`, `TimestampTooOld`, `DeviationTooHigh`. `set_reference_value` returns
  `false`, leaves `Price`/`Timestamp` untouched, and records the attempt in
  `LastRejectedValue`/`LastRejectedTimestamp`/`LastRejectReason` (readable via `last_rejection()`)
  plus a `ref_rej` event. The transaction itself still **succeeds** — a routine rejected tick (a
  feed's transient bad read, say) should not look like a broken transaction in monitoring, and the
  only way a dashboard can read "last rejection reason" from a contract instead of a private
  relay's database is if the rejection is actually committed on-chain.

**This is the one behavior genuinely worth flagging to anyone scripting against this contract**: a
`stellar contract invoke` call that submits a value violating the deviation guard, the monotonic
check, or the timestamp-freshness check will print "✅ Transaction submitted successfully!" even
though the price was *not* updated. A human operator or an automated relay must check the
**return value**, not just transaction success, to know whether their submission actually landed.
§4's live evidence demonstrates this directly: the rejected-deviation call below shows a successful
transaction whose own return value is `false`.

A caller that wants a hard revert instead can call `assert_available()` beforehand, or simulate
`set_reference_value` first and inspect its return value before submitting for real.

## 4. Live Testnet evidence

A fresh `OracleAdapter` instance was deployed with the new wasm (hash below), initialized with the
production-recommended defaults (`max_stale_seconds = 3600`, `max_deviation_bps = 1000`, i.e. 10%),
and given one deterministic primary source ID computed exactly per §5.2.2's convention:

```text
preimage:  principal-oracle-source:testnet:STA:operator:manual
source_id: da9d5436f1bc0a2e8211d7a57158501e1c1cd8589fc9e04e1ca286917dc14e3b  (sha256 of the preimage)
```

| Contract | New WASM SHA-256 |
|---|---|
| OracleAdapter | `c21408bd5f75e31ad553d8a9f9531ced1e0bbd216ebc660ebdc4b0baf759547c` |
| PrincipalManager | `fd68e76429b1cae6c5167eb363584afd0a2530a0770547040d4c85c80f05757e` |
| MarketPool | `a98ad679cd01d837db2a03327a406ab31d68efdf4694783a6fb4d616c207bfa0` |
| YTToken | `65cf62c272c4d9acdd886f7036960072df6e9495e917252acee4f61aa8ea1170` |

`PrincipalManager`, `MarketPool` and `YTToken` all changed too — not because their own logic
changed, but because each one's minimal `OracleInterface` (the cross-contract trait it calls
through) now declares `is_available()` instead of `is_fresh(max_stale_seconds)`, removing each
contract's own hardcoded `MAX_ORACLE_STALENESS_SECS` constant in favor of the oracle's own
centrally configured threshold. `SYWrapper`, `PTToken`, `MarketConfig` and `RecoveryEscrow` are
unchanged and reuse their existing uploaded WASM.

OracleAdapter instance: `CA6AF46CNMNOV5RMDADCPTU5PSMZRQXM4G5TXJQ4YWRKPFAB67NLIQZN`.

| Step | Call | Result | Transaction |
|---|---|---|---|
| 1 | `initialize(admin, 3600, 1000)` | OK | [ca03ea57...](https://stellar.expert/explorer/testnet/tx/ca03ea57645a68ce54737a612e8921402dd9e365abb96e9c8ad3dbfd517b60af) |
| 2 | `set_sources(admin, da9d5436…, None)` | OK, `src_set` event | [595333ae...](https://stellar.expert/explorer/testnet/tx/595333aed32cf637ba93d07ea1f0763d94937000788aa4a4e4c3c03ab7d29d1b) |
| 3 | `set_reference_value(admin, da9d5436…, 10000000, now)` — genesis | `true`, `ref_set` event | [e85c2253...](https://stellar.expert/explorer/testnet/tx/e85c22537dae3da05c8a4fa343b4211f0424dd5e25d4a3a1f738939226667759) |
| 4 | `set_reference_value(…, 12000000, …)` — a 20% jump against a 10% cap | **`false`**, `Price` unchanged at `10000000`, `ref_rej` event with reason `deviate` | [25b0710e...](https://stellar.expert/explorer/testnet/tx/25b0710eb5cd11054653cbc81b3358b8c98f0f89735c71304be1d5e4fe9d1a6a) |
| 5 | `set_reference_value(…, 10500000, …)` — a 5% jump, inside the cap | `true`, `Price` now `10500000` | [f9a64ce5...](https://stellar.expert/explorer/testnet/tx/f9a64ce5611a5f42534065a72a1627cd1006f094aaa095305c0196f21ba1cf1e) |

Step 4's `last_rejection()` read back exactly what was recorded:
`{"reason":"deviate","timestamp":1793697416,"value":"12000000"}` — a contract read, not a log file.

**Availability blocking a real consumer flow, end to end**, on a market created through
`MarketFactoryContract` using the four new WASM hashes above plus the unchanged
`SYWrapper`/`PTToken`/`MarketConfig`/`RecoveryEscrow`:

| Step | Call | Result | Transaction |
|---|---|---|---|
| 6 | `PrincipalManager.mint` with the oracle fresh | succeeds, 10,494,750 PT/YT minted | [cc75cf56...](https://stellar.expert/explorer/testnet/tx/cc75cf56795157ee26e0a27b5bf43f3484a393febac3147194b9ef7dd889b986) |
| 7 | `set_safety_params(admin, 2, 1000)` — narrow the window to 2s for this demo | OK | [cfd49f60...](https://stellar.expert/explorer/testnet/tx/cfd49f60705948d9a35fce34f4f4f2a6ac3957968bf715f35c4829bb2b926a29) |
| 8 | `PrincipalManager.mint` once the stored value is more than 2s old | reverts `OracleStale` (PrincipalManager error 7) | simulation failure, no transaction — see diagnostic trace below |
| 9 | `set_safety_params(admin, 3600, 1000)` then a fresh `set_reference_value` | OK | [493ae23a...](https://stellar.expert/explorer/testnet/tx/493ae23a2153663036f6625e5d06ccada6322160607ac2947be267a00abd6ac9), [101c6781...](https://stellar.expert/explorer/testnet/tx/101c6781a71d1a460a9c5c7a0bfd5e542202c0a66c3080d6b52a5f0fa7847f09) |
| 10 | `PrincipalManager.mint` again, oracle fresh again | succeeds, 5,247,375 PT/YT minted | [427bf580...](https://stellar.expert/explorer/testnet/tx/427bf5800fb11cd3b7c27fb4b9c8cc145e817123543896714845ffa4b25ab597) |

Step 8's diagnostic trace shows the exact cross-contract chain this extension is meant to produce
— `PrincipalManager` calling the oracle's own `is_available()`, getting `false`, and raising its
own, unchanged `OracleStale` error:

```text
[fn_call] PrincipalManager.mint
[fn_call] OracleAdapter.is_available -> false
[error]   PrincipalManager: "failing with contract error", 7   (OracleStale)
```

Step 7's 2-second window (rather than waiting out the real 3600s production default) is a
deliberate shortcut for this demonstration only — step 9 restores the production defaults
immediately afterward. The staleness-blocks-mint mechanism itself is unchanged by this shortcut;
only the wait time is compressed.

**Why no single-step live demonstration of the fallback path**: the "may only refresh the
timestamp" behavior is deterministic logic with no network-timing dependency, and is covered
exhaustively by `contracts/oracle_adapter/src/test.rs`'s `fallback_*` tests (same-value timestamp
refresh accepted, any value mismatch rejected even within the deviation bound, a non-newer
timestamp rejected). Live evidence would add a second source ID and two more transactions without
exercising any mechanism the deviation-guard and availability evidence above don't already cover
live.

## 5. Test coverage

`contracts/oracle_adapter/src/test.rs`: 35 unit tests — initialize/`set_safety_params` parameter
validation, source configuration and views, unrecognized-source handling (both "wrong source
given a configured fallback" and "nothing configured at all", which get different errors),
primary-source submission (accept / graceful-reject on decrease, stale timestamp, deviation; hard
panic on invalid value, future timestamp, wrong caller), the deviation guard's boundary and its
exemption for the very first value, the fallback's timestamp-only-refresh policy, and
`is_available`/`assert_available`.

Every existing consumer contract's own oracle-staleness tests continue to pass unchanged after
switching from `is_fresh(constant)` to `is_available()`:
`contracts/principal_manager/src/test.rs` (`oracle_stale_blocks_mint`, `oracle_stale_blocks_redeem`),
`contracts/yt_token/src/test.rs` (`initialize_rejects_stale_oracle`,
`update_yield_index_blocked_by_stale_oracle`), and the shared-stack integration suite
(`contracts/integration_tests/tests/edge_cases.rs`'s
`oracle_freshness_boundary_exactly_at_the_window_is_fresh_one_second_later_is_stale`,
`full_lifecycle.rs`'s `settle_all_requires_maturity_and_a_fresh_oracle`,
`liquidity_and_router.rs`'s `a_stale_oracle_blocks_swaps_but_never_blocks_an_lp_exit`). The shared
test harness (`contracts/integration_tests/src/stack.rs`) configures every market's oracle with a
365-day staleness window and a 100% deviation allowance by default, generous enough that no
existing test's oracle move trips either new guard by accident; the three tests above that
specifically exercise the 3,600-second production boundary narrow their own market's oracle via
`set_safety_params` first.

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and
`cargo test --workspace` all pass clean.

## 6. Self-audit

- **Found and fixed during review, before this was ever run live**: the "no source matches"
  branch originally always raised `FallbackNotConfigured` whenever the submitted source wasn't the
  primary, including when *no* source had been configured at all (`primary_source` itself `None`)
  — a case where `InvalidSource` is the accurate error, since `FallbackNotConfigured` presupposes a
  real, configured primary. Fixed to check `primary.is_some()` first; a new test
  (`unrecognized_source_panics_invalid_source_when_nothing_is_configured_at_all`) locks in the
  corrected behavior alongside the existing one for the "primary configured, no fallback" case.
- **Deliberately not wired into the three existing consumers**: `assert_available()` (the
  panic-propagating form of `is_available()`) exists because the funded text explicitly asks for
  an `OracleUnavailable` error, but none of `MarketPool`/`PrincipalManager`/`YTToken` call it —
  each keeps raising its own pre-existing, already-tested, already-documented `OracleStale` error
  instead. Renaming or renumbering that error in three places to match the spec's vocabulary
  exactly would be unrelated churn with no behavioral benefit; this doc and `API_REFERENCE.md` both
  record that this protocol's `OracleStale` *is* what the funded text calls "the oracle is
  unavailable" at the consumer level, so the correspondence is documented rather than silently
  assumed.
- **The deviation guard's ceiling is hard-capped at 100% (`max_deviation_bps` ≤ `BPS_SCALE` =
  10,000)** in `initialize`/`set_safety_params`, matching this codebase's existing pattern of
  hard-capping percentage-like parameters (`SECURITY.md` §1: tokenization fee ≤1%, YT fee ≤50%,
  swap tier ≤5%). One existing integration test
  (`edge_cases.rs::remaining_slippage_and_dust_guards_on_the_pool_and_manager`) previously jumped
  the shared test market's oracle rate 3x (200%) in one step to force a recombine dust case; that
  move is now impossible in one call under any valid configuration, by design, so the test was
  changed to a 10% bump — the only property it actually needed was "rate > SCALE", not any
  particular magnitude.
- **Deviation arithmetic** (`(value - current_price) * BPS_SCALE / current_price`) uses `i128`
  with this codebase's workspace-wide `overflow-checks = true` release profile setting, so an
  adversarial or malformed value large enough to overflow the intermediate product would panic
  (abort the transaction) rather than silently wrap into a wrong, under-protective accepted value
  — a fail-closed outcome consistent with how every other fixed-point calculation in this codebase
  already relies on the same release-profile setting.
- **What this doc does not verify live**: the fallback source's timestamp-only-refresh policy
  (§4 explains why — it has no network-timing dependency, and is covered exhaustively by unit
  tests instead) and a literal 3,600-second real-time wait for the production staleness window
  (§4 step 7 substitutes a 2-second window to demonstrate the same mechanism in well under a
  minute; the mechanism is identical, only the wait time is compressed).

## 7. What is still open

This closes the *contract* portion of Tranche 2 Deliverable 1 and the prerequisite Tranche 3
Deliverable 3 depends on. Still outstanding, and out of scope for this change: the scripted
deployment pipeline (`scripts/deploy/deploy.sh`, `verify-deployment.sh`, the manifest schema), the
public Testnet dApp, the live E2E suite, the Tranche 2/3 acceptance documents, and (Tranche 3) the
RedStone oracle-relay service and its monitoring/alerting — all tooling and frontend work the
funded spec maps to separate work packages (`docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md` §14.2–
§14.7), none of which touch contract code.
