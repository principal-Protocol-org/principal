# Principal Protocol Mainnet Smart Contract Security Assessment

**Status:** Partial assessment — further independent review and exploit testing required  
**Repository:** `principal`  
**Branch:** `mainnet`  
**Audited commit:** `451b8175e6b5e7142ca901d768436afa89ae6857`  
**Assessment date:** 17 November 2026  

## 1. Executive Summary

This assessment reviewed the latest smart contract source in the local `mainnet` working tree. It did not audit or rely on the older contracts deployed on Stellar Testnet.

The audited baseline is commit `451b8175e6b5e7142ca901d768436afa89ae6857`. The working tree also contained an uncommitted OracleAdapter rate-limit patch and corresponding test/snapshot changes. The source state and patch were preserved under `audits/evidence/`.

One potentially critical custody vulnerability was identified in `SYWrapper.withdraw_gained`: a caller controls the signed `baseline` value, negative baselines are accepted, and the common withdrawal implementation does not independently enforce `shares <= balance`. The resulting arithmetic can request more shares than the caller owns, debit the caller to a negative internal balance, and transfer underlying from pooled custody. This finding is strongly supported by direct code analysis but was not reproduced by an executed deterministic PoC before this report was finalized. Mainnet deployment is therefore **Blocked pending immediate reproduction and remediation**.

The uncommitted OracleAdapter throttle patch also has an implementation weakness: it compares ledger time with the last accepted feed-provided timestamp instead of recording the ledger time at acceptance. A compromised or malfunctioning configured feed can potentially present multiple increasing but sufficiently old timestamps in one ledger and compound the per-update deviation allowance. This requires control or compromise of the configured feed and does not let an ordinary caller forge an honest feed's response. The patch's tests currently fail in two cases.

This is not a completed full audit. The report preserves the verified review work, findings, test results, and limitations for continuation by another reviewer.

## 2. Audit Scope and Methodology

### Source scope

The review covered the production source of these 12 deployable contracts:

1. MarketConfig
2. MarketFactory
3. MarketPool
4. OracleAdapter
5. Permissioning
6. PrincipalManager
7. PTToken
8. RecoveryEscrow
9. RiskControl
10. Router
11. SYWrapper
12. YTToken

The shared `principal_compliance` library was also reviewed. `principal_integration_tests` is a test package rather than a deployable contract.

Approximately 282 public functions and entry points were inventoried across the deployable contracts. The review traced authorization, compliance, custody, tokenization, yield accrual, redemption, AMM trading, liquidity, recovery, risk-control, factory, and oracle paths.

### Version and working-tree state

- Active branch: `mainnet`
- Commit: `451b8175e6b5e7142ca901d768436afa89ae6857`
- Rust compiler observed: `rustc 1.94.1`
- Cargo observed: `cargo 1.94.1`
- Soroban SDK: `26.0.1`
- Soroban environment dependencies resolved to `26.1.3` in `Cargo.lock`
- Release profile enables overflow checks and abort-on-panic behavior.

The local working tree contained an uncommitted OracleAdapter throttle change. Evidence is stored in:

- `audits/evidence/source-state.json`
- `audits/evidence/working-tree.patch`

No existing documentation or production smart contract source was changed by this assessment.

### Methods used

- Manual entry-point and storage review
- Authorization and privileged-role tracing
- Cross-contract call and custody-flow analysis
- PT/YT issuance, yield, recombination, maturity, and redemption analysis
- Compliance inheritance and recovery-path review
- AMM reserve and fee-accounting review
- Oracle freshness, deviation, and timestamp review
- Existing unit and integration test execution
- Release WASM compilation
- Offline dependency advisory scan

### Explicit limitations

- The suspected SYWrapper exploit was not executed as a deterministic PoC before finalization.
- Stateful fuzzing and comprehensive property testing were not completed.
- The complete negative-authorization matrix was not independently re-executed without test authorization mocks.
- No deployed bytecode was assessed because the user explicitly scoped this work to the latest source; the existing Testnet deployment corresponds to an older source version.
- The OracleAdapter local patch is not represented by the audited Git SHA alone and must be identified by the preserved working-tree patch.
- The full test suite stopped at OracleAdapter after two failures, so packages scheduled later in Cargo's run were not demonstrated by this specific command.

## 3. Protocol Architecture and Trust Model

The protocol wraps an underlying SAC or SEP-57-style RWA token into SY shares. PrincipalManager takes custody of SY and mints equal-notional PT and YT. PT represents principal redeemable at the frozen maturity rate. YT uses a reciprocal-rate index to accrue appreciation above its holder's settlement index. MarketPool trades PT against SY and composes PT/YT flash-mint and recombination flows. Router sequences user operations. RecoveryEscrow lets the live issuer authority seize deauthorized derived positions and unwind them toward the underlying. Permissioning adds protocol account and per-instrument allowlists, while the compliance library checks the underlying asset's live authorization state.

The principal trust assumptions are:

- The underlying issuer controls holder authorization, freeze, and clawback policy.
- Oracle administration or the configured external feed supplies a monotonic, correctly scaled reference value.
- Contract administrators correctly wire shared infrastructure and one-time minter/recovery references.
- Keepers maintain instance and persistent storage TTLs or restore archived state.
- Permissioning administrators maintain account and instrument eligibility.
- The targeted financial model assumes a non-decreasing underlying reference value; declining-value assets are unsupported by the settlement design.

## 4. Security Findings Summary

| ID | Severity | Status | Finding |
|---|---:|---|---|
| PRINCIPAL-001 | Critical | Strongly supported; PoC not executed | Negative `withdraw_gained` baseline can bypass the SY balance limit and drain pooled underlying |
| PRINCIPAL-002 | High | Strongly supported; patch-specific | Feed timestamp is incorrectly used as the acceptance throttle clock |
| PRINCIPAL-003 | Medium | Confirmed by test execution | Latest OracleAdapter working tree fails two unit tests after the throttle patch |
| PRINCIPAL-004 | Medium | Design/trust-model risk | Initializers do not consistently require the nominated administrator's authorization |
| PRINCIPAL-005 | Medium | Design/availability risk | Recovery serializes all YT seizures behind one global unfinalized record until maturity |
| PRINCIPAL-006 | Low | Confirmed by dependency scan | Locked dependency tree includes a reported `crypto-bigint` timing advisory |

No finding should be interpreted as proof that unaffected code is secure. PRINCIPAL-001 must be reproduced and fixed before deployment.

## 5. Detailed Security Findings

### PRINCIPAL-001: Negative `withdraw_gained` baseline can bypass the SY balance limit and drain pooled underlying

**Severity:** Critical  
**Status:** Strongly supported but not reproduced  
**Affected contract:** SYWrapper  
**Affected functions:** `withdraw_gained`, `do_withdraw`, `sub_balance`  
**Location:** `contracts/sy_wrapper/src/lib.rs:405`, `:432`, `:740`  

**Root cause**

`withdraw_gained` accepts a caller-supplied signed `i128 baseline` and computes:

```text
shares = balance_of(from) - baseline
```

It does not reject a negative baseline and does not require the baseline to be less than or equal to the current balance. If the caller supplies `baseline = -X`, the computed withdrawal becomes `balance + X`.

Unlike `withdraw` and `withdraw_crediting`, this path does not perform `balance < shares` before calling `do_withdraw`. The common `do_withdraw` implementation also omits an independent balance check. `sub_balance` then writes `balance - shares`, which can create a negative internal balance, while the wrapper transfers the calculated underlying amount from its pooled token custody.

**Attack preconditions**

- The attacker is authorized on the underlying asset.
- The attacker passes protocol Permissioning.
- The wrapper is not paused.
- The wrapper holds underlying deposited by any users.
- The attacker can authorize a direct `withdraw_gained` call for their own address.

The Router normally supplies a legitimate baseline, but `withdraw_gained` is a public contract entry point and can be invoked directly.

**Exploit scenario**

1. Victims deposit underlying into SYWrapper, creating pooled underlying custody.
2. The attacker obtains any eligible account; a positive SY balance may not be required if the selected negative baseline creates positive `shares`.
3. The attacker calls `withdraw_gained(attacker, negative_baseline, attacker, 0)`.
4. `shares = attacker_balance - negative_baseline` exceeds the attacker's balance.
5. `do_withdraw` decreases global totals and transfers pooled underlying to the attacker.
6. `sub_balance` records a negative attacker share balance instead of rejecting the withdrawal.

**Expected secure behavior**

Negative baselines and baselines above the current balance must be rejected. Every withdrawal implementation must independently enforce `0 < shares <= balance(from)`.

**Impact**

If runtime behavior matches the direct code path, an eligible external account can withdraw underlying belonging to other depositors, corrupt totals, and make the wrapper insolvent. The loss is bounded by the wrapper's actual underlying token balance, because the final token transfer reverts beyond available custody.

**Evidence**

- `withdraw_gained` computes the delta from a caller-controlled signed baseline.
- It lacks the balance comparison used by `withdraw` and `withdraw_crediting`.
- `do_withdraw` lacks its own balance comparison.
- `sub_balance` stores the arithmetic result without preventing a negative balance.

**Proof of concept**

Not executed. This must be the first follow-up test. A deterministic test should deposit funds from a victim, grant an attacker eligibility, call `withdraw_gained` with a negative baseline, and assert that the call reverts without changing balances or totals.

**Recommendation**

1. Reject `baseline < 0`.
2. Reject `baseline > current_balance` unless the intended zero-gain behavior explicitly permits it without state change.
3. Calculate the gain with checked subtraction.
4. Add a mandatory `shares <= get_balance(from)` check inside `do_withdraw`, so every current and future caller is protected.
5. Use checked arithmetic for total and balance decrements.

**Regression test**

Test negative baseline, baseline greater than balance, minimum `i128`, zero balance, and a legitimate positive gain. Assert storage and token custody remain unchanged on every rejected case.

**Confidence:** High from code analysis; exploit classification remains pending runtime reproduction.

### PRINCIPAL-002: Feed timestamp is incorrectly used as the acceptance throttle clock

**Severity:** High  
**Status:** Strongly supported; applies to the uncommitted OracleAdapter patch  
**Affected contract:** OracleAdapter  
**Affected function:** `sync_from_feed`  
**Location:** `contracts/oracle_adapter/src/lib.rs:588` and `:617`  

**Root cause**

The patch intends to impose a minimum real-time interval between accepted permissionless feed syncs. It compares:

```text
ledger_timestamp - current_ts < minimum_interval
```

However, `current_ts` is the stored feed-provided price timestamp, and each successful update stores the next feed timestamp in the same `Timestamp` key. The contract does not store the ledger timestamp at which an update was accepted.

If a configured feed returns multiple increasing timestamps that remain at least the minimum interval behind ledger time, multiple calls in the same ledger can pass the throttle. Each may increase the stored price by the maximum per-update deviation, compounding a guard that was intended to be time-bounded.

**Attack preconditions**

- The configured SEP-40 feed is malicious, compromised, or faulty, or its responses can otherwise be controlled.
- Feed timestamps are increasing and accepted as non-future.
- Absolute value bounds either are unset or are broad enough for the walk.
- Downstream operations consume the manipulated reference value.

An ordinary user cannot forge values returned by an honest configured feed merely because `sync_from_feed` is permissionless.

**Impact**

A compromised feed can potentially accelerate upward price manipulation despite the new throttle, affecting PT/YT issuance, AMM pricing, maturity settlement, and solvency. Exact financial impact depends on market liquidity, configured bounds, deviation rate, and timing.

**Recommendation**

Store a separate `LastFeedAcceptanceLedgerTimestamp` set to `env.ledger().timestamp()` only after a successful feed update. Apply the minimum interval against that value. Keep the feed observation timestamp separately for freshness and monotonicity checks.

**Regression test**

In one ledger, configure a mock feed to return several increasing observations with timestamps all sufficiently behind ledger time. Only the first sync must succeed. Advance ledger time to the precise interval boundary and confirm the next update succeeds.

**Confidence:** High from code analysis; deterministic PoC not executed.

### PRINCIPAL-003: Latest OracleAdapter working tree fails two unit tests after the throttle patch

**Severity:** Medium  
**Status:** Confirmed  
**Affected contract:** OracleAdapter  
**Affected tests:** `last_update_was_feed_toggles_correctly_between_push_and_pull`, `sync_from_feed_does_not_touch_active_source`  
**Evidence:** `audits/evidence/latest-tests.log`  

The command `cargo test --workspace --locked --offline` reached OracleAdapter after the earlier packages and integration suites passed. OracleAdapter reported 58 passing tests and two failures. Both expected `sync_from_feed()` to return true but the new default throttle rejected the call.

The failures show that the latest working tree is not release-ready and that the throttle changed valid transition behavior without updating the test setup or intended semantics consistently.

**Recommendation**

Fix the throttle clock as described in PRINCIPAL-002, update tests to advance ledger time only where the policy truly requires it, and rerun the complete workspace suite from start to finish.

### PRINCIPAL-004: Initializers do not consistently require the nominated administrator's authorization

**Severity:** Medium  
**Status:** Design/trust-model risk  
**Affected contracts:** Permissioning, RiskControl; potentially any directly deployed contract whose initialization does not authenticate its nominated authority  
**Locations:** `contracts/permissioning/src/lib.rs:41`, `contracts/risk_control/src/lib.rs:129`  

Permissioning and RiskControl write the supplied `admin` during first initialization without calling `admin.require_auth()`. Because initialization is one-time, an uninitialized directly deployed instance can be claimed by the first caller, who may nominate any admin address.

The factory and deployment scripts may reduce the normal operational window, and a squatted instance should not automatically be treated as an official deployment. Nevertheless, the contracts themselves do not cryptographically bind initialization to the nominated administrator. This is material for manual deployments, partial factory flows, misconfigured deployments, and monitoring that trusts an address before initialization is verified.

**Recommendation**

Require `admin.require_auth()` in every admin-bearing initializer and add negative tests that initialize without the nominated admin's signature. Bind deployment tooling and manifests to expected contract addresses and initialization transactions.

**Confidence:** High for the code behavior; severity depends on deployment procedure and address trust establishment.

### PRINCIPAL-005: Recovery serializes all YT seizures behind one global unfinalized record until maturity

**Severity:** Medium  
**Status:** Design/availability risk  
**Affected contract:** RecoveryEscrow  
**Affected functions:** `seize_one`, `finalize_record`  
**Location:** `contracts/recovery_escrow/src/lib.rs:463`  

RecoveryEscrow intentionally allows only one unfinalized YT recovery record globally because YT pending yield is pooled by the escrow address. A seized YT record cannot be finalized before maturity through the documented path. Therefore, one early YT seizure can block recovery of every other restricted holder's YT for the remaining market lifetime.

This preserves per-record yield attribution but creates a potentially long compliance and operational denial of service. A malicious holder cannot directly trigger seizure, but an incident involving multiple sanctioned or deauthorized accounts cannot be handled promptly.

**Recommendation**

Use per-record custody addresses or account for each record's pro-rata balance and yield index independently. At minimum, document and operationally accept the maximum blocking interval and add an issuer-authorized pre-maturity consolidation or transfer mechanism that preserves attribution.

**Confidence:** High for the serialization behavior; classification is a design risk rather than an unauthorized exploit.

### PRINCIPAL-006: Locked dependency tree includes a reported `crypto-bigint` timing advisory

**Severity:** Low  
**Status:** Confirmed by offline dependency scan  
**Affected dependency:** `crypto-bigint 0.5.5`  
**Advisory:** `RUSTSEC-2026-0387`  

`cargo audit --no-fetch --json` reported one vulnerability concerning timing variability in operations intended to be constant-time. It also reported the unmaintained `paste 1.0.15` package as an informational warning.

No reviewed Principal contract code was shown to perform secret-dependent cryptographic operations through this affected dependency in a way that establishes a practical contract exploit. The issue should still be resolved in the locked dependency chain and assessed against signing or off-chain tooling that may handle secrets.

**Recommendation**

Upgrade the dependency chain to a version using `crypto-bigint >= 0.7.6`, where compatible, and rerun `cargo audit` plus the full contract test and WASM build suite.

## 6. Contract-by-Contract Assessment

### Compliance library

The library detects SAC versus SEP-57-style RWA behavior and reads authorization live. SAC authorization failures and RWA probe failures generally fail closed. The RWA authority check uses an idempotent `set_address_frozen` capability probe, which assumes compatible operator semantics and makes a state-changing cross-contract call for authorization discovery. Real supported SEP-57 implementations should be integration-tested; the repository uses a local mock.

### MarketConfig

Fee caps and protocol-share bounds are enforced. Issuer authority is read live through the compliance adapter, and protocol administration is separate. Arithmetic in `split` uses raw multiplication; configured shares are bounded, but very large fee inputs rely on release overflow checks. No confirmed exploit was established in this pass.

### MarketFactory

The three-phase deterministic factory reduces partial deployment risk within each transaction and validates salt-derived addresses between phases. It intentionally accepts caller-provided WASM hashes and is not an approved-code registry. Users and deployment tooling must independently verify uploaded bytecode. No confirmed exploit was established in this pass.

### MarketPool

The pool maintains internal PT/SY reserves, fee buckets, and LP supply. Existing tests cover invariant monotonicity, round-trip non-profit, donation resistance, reserve/token balance agreement, deadlines, pauses, and oracle guards. Raw arithmetic remains widespread, relying on release overflow checks. A complete independent mathematical proof and high-range fuzzing were not completed.

### OracleAdapter

Push updates are admin-authenticated and enforce source identity, timestamp monotonicity, non-decreasing value, deviation, freshness, and optional absolute bounds. Pull updates are permissionless and read the configured feed. The local throttle patch has PRINCIPAL-002 and currently fails two tests under PRINCIPAL-003.

### Permissioning

Administrative grant, revoke, and transfer functions require the stored admin. Eligibility uses persistent storage with explicit TTL extension and fails closed after expiration or archival until restored. Initialization lacks nominated-admin authorization, as described in PRINCIPAL-004.

### PrincipalManager

The manager custodies SY, mints and burns PT/YT, freezes settlement, pays yield, and accrues fees. Topology checks compare underlying, permissioning, maturity, and oracle wiring. Core user operations require user authorization and compliance. Its safe redemption depends critically on SYWrapper's withdrawal invariant, which PRINCIPAL-001 violates through another public wrapper entry point.

### PTToken

Mint and burn are restricted to a one-time minter, while transfers enforce underlying authorization plus account and per-instrument Permissioning. Recovery seizure is escrow-only. Existing code uses checked balance/supply arithmetic. No confirmed exploit was established in this pass.

### RecoveryEscrow

Seizure requires the live issuer authority and a deauthorized target; the position contracts trust the escrow address. Topology checks compare underlying identity. The global YT-record serialization produces PRINCIPAL-005.

### RiskControl

Consumers must be registered before they can consume deposit capacity. Global and per-asset sliding-window limits use checked arithmetic. Pausers cannot unpause. Initialization lacks nominated-admin authorization, as described in PRINCIPAL-004. The window length and limits remain fully admin-controlled trust assumptions.

### Router

Router markets are admin-registered after topology checks. User flows require the user's authorization and downstream contracts reapply compliance. Deadlines and output/cost limits exist on trading paths. Several composed paths depend on public baseline-based helpers; the Router supplies legitimate baselines, but direct invocation of `SYWrapper.withdraw_gained` creates PRINCIPAL-001.

### SYWrapper

SYWrapper custodies all underlying assets and maintains internal share balances. Deposits, transfers, and withdrawals enforce underlying authorization and Permissioning. The critical balance-invariant gap is detailed in PRINCIPAL-001. The design's exchange rate remains pinned at inception for the currently targeted non-rebasing asset model; support for rebasing or loss-bearing assets is not implemented.

### YTToken

The reciprocal-rate index and per-account settlement logic are designed to avoid retroactive yield assignment and overpayment. Transfers settle both sides; claims are manager-only. Maturity freezes the first fresh at-or-after-maturity observation. The model ignores decreases and therefore depends on a non-decreasing asset value. No independent stateful proof of solvency was completed.

## 7. Economic Security Assessment

The intended economic invariant for a position minted with net shares `S` at rate `R0` and settled at `Rf` is:

```text
PT redemption + YT yield <= S
```

The reciprocal YT index is structured so that ideal real-valued claims telescope to the original net shares. The implementation applies protocol-favoring rounding to YT settlement. Existing integration tests cover large-position redemption, path independence, fee-free round trips, pool invariant behavior, first-depositor controls, donation resistance, and reserve accounting.

These protections do not preserve solvency if the wrapper permits withdrawal beyond a holder's share balance. PRINCIPAL-001 breaks the more fundamental custody invariant:

```text
For every withdrawal: burned shares <= holder balance
```

The protocol also explicitly assumes non-decreasing oracle value. It is unsuitable for assets with negative yield, market-price declines, principal loss, or redemption haircuts unless loss allocation is redesigned across PT and YT.

## 8. Stellar and RWA Compliance Assessment

Derived positions do not assume SAC controls propagate automatically. SY, PT, YT, LP, manager operations, and transfers generally recheck the underlying authorization and Principal Permissioning. RecoveryEscrow supplies a dedicated path for derived-position seizure after the issuer deauthorizes a holder.

Underlying issuer clawback can remove assets held by protocol contracts at the SAC layer. The source does not create an automatic accounting adjustment for an issuer clawback against pooled custody. Such a clawback can leave outstanding SY/PT/YT liabilities undercollateralized; this is an unavoidable issuer trust and operational risk unless explicit loss allocation and reconciliation are implemented.

Code-enforced properties include live authorization checks and escrow-gated forced movement of derived positions. External assumptions include correct issuer behavior, authorization configuration, identity-verifier correctness, legal authority for seizure, and post-recovery native clawback procedures.

## 9. Soroban Security Assessment

The contracts generally use `require_auth` at user and privileged state-changing entry points, and cross-contract calls depend on Soroban authorization trees and atomic rollback. MarketPool explicitly authorizes a nested SY transfer during true flash mint. The runtime forbids direct reentrancy into a contract already on the invocation stack, reducing classic EVM-style reentrancy risk.

Instance and persistent state are subject to TTL and archival. Most contracts expose permissionless `bump` functions for instance state and extend user-entry TTL on writes. Operational keepers are still required for long-dated markets, and dormant user entries may need restoration before interaction. No contract upgrade entry points or storage migration mechanisms were found; upgrades would require deployment replacement or external address migration rather than an in-contract WASM update.

## 10. Adversarial Testing Results

### WASM build

The committed snapshot was built with:

```text
cargo build --workspace --release --locked --offline --target wasm32v1-none
```

The build completed successfully and produced 12 deployable WASM files, all below the repository's 128 KiB size gate. This build represented the committed snapshot, not the later uncommitted OracleAdapter patch.

### Latest working-tree tests

Executed:

```text
cargo test --workspace --locked --offline
```

Before reaching OracleAdapter, the command recorded passing results for:

- Compliance unit tests: 9
- AMM integration tests: 31
- Audit regression tests: 11
- Budget tests: 1
- Documentation examples: 6
- Edge cases: 24
- Full lifecycle: 13
- Liquidity and Router: 32
- Pool guards: 2
- Recovery: 19
- Risk control: 11
- PrincipalManager unit tests: 44
- MarketConfig unit tests: 13
- MarketFactory tests: 9 total across unit and integration tests
- MarketPool unit tests: 15

OracleAdapter then reported 58 passed and 2 failed. Cargo stopped at that package, so this command is not evidence that every remaining package passed in the latest working tree.

### Dependency scan

Executed:

```text
cargo audit --no-fetch --json
```

Result: one reported vulnerability (`RUSTSEC-2026-0387`) and one unmaintained-package warning (`paste`).

### Tests not completed

- Deterministic PoC for PRINCIPAL-001
- Deterministic throttle bypass test for PRINCIPAL-002
- Full workspace rerun after OracleAdapter correction
- Stateful multi-actor fuzzing
- Comprehensive random AMM and PT/YT invariant campaigns with recorded seeds
- Release lint and formatting checks for the final uncommitted working tree

## 11. Deployment and Operational Risks

- Do not deploy while PRINCIPAL-001 remains unresolved.
- The current OracleAdapter patch fails tests and does not enforce its intended real-time throttle correctly.
- Shared administrator keys control permissioning, risk limits, pausing, oracle settings, Router registration, and operational wiring. Use separated roles and multisignature controls where supported.
- Some administrator transfers are one-step and do not require acceptance by the new administrator, creating irreversible typo and key-loss risk.
- One-time minter and recovery-escrow wiring makes configuration mistakes difficult or impossible to correct without redeployment.
- There is no in-contract upgrade or migration mechanism; remediation requires a deliberate deployment and position-migration plan.
- Instance and user-entry TTL operations require monitoring and restoration procedures.
- Oracle absolute value bounds are optional. They should be mandatory in deployment policy.
- A native issuer clawback against protocol custody can create insolvency unless liabilities are reconciled operationally.

## 12. Mainnet Readiness Assessment

**Blocked.**

The code should not be deployed to mainnet until the following actions are completed in priority order:

1. Reproduce and remediate PRINCIPAL-001; add defense-in-depth balance enforcement in `do_withdraw`.
2. Correct the OracleAdapter acceptance-clock design and make every OracleAdapter test pass.
3. Rerun the entire workspace suite, release WASM build, formatting, linting, and dependency audit from the exact release tree.
4. Add deterministic exploit/regression tests for both principal findings.
5. Require authorization in every administrator-bearing initializer.
6. Decide whether global YT recovery serialization is operationally acceptable; redesign if multiple simultaneous recoveries are required.
7. Conduct an independent second review and stateful economic fuzzing after remediation.
8. Produce reproducible WASM hashes from the final clean source tree before any deployment.

## 13. Final Conclusion

The latest `mainnet` source contains substantial authorization, compliance, topology, oracle, rounding, and regression-test protections. However, direct code analysis identified a strongly supported critical custody flaw in the public SY withdrawal helper and a high-risk weakness in the uncommitted oracle throttle. The latest working tree also fails two OracleAdapter tests.

The protocol is not ready for mainnet deployment in the assessed state. The immediate priority is to prevent withdrawals exceeding a holder's SY balance and prove the fix with a deterministic adversarial test. Oracle throttling must then use a contract-controlled acceptance clock, followed by a full clean-tree validation and independent re-audit.

This document is intentionally labeled partial. It preserves actual evidence and does not claim that unexecuted tests passed or that unreviewed attack hypotheses were disproved.
