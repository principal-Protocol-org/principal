# Final Closing-Pass Audit — 5 November 2026

Branch: `feat/tranche-1-mvp`, commit `a1038e6` plus this review's own fixes. Reviewer stance: senior
Soroban/Stellar smart-contract auditor. This is the closing pass after
[`docs/AUDIT_2026_11_04_FULL_REVIEW.md`](AUDIT_2026_11_04_FULL_REVIEW.md)'s comprehensive audit,
its gap-closing follow-up fix, and the Testnet redeployment in
[`docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md`](TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md) —
four independent fresh-eyes passes, one per subsystem, each explicitly tasked with scrutinizing
the newest (least-reviewed) changes alongside a full independent re-check of everything else in
scope, rather than re-treading the prior audit's own findings.

## Scope

1. `contracts/market_pool` (AMM core + `math.rs`)
2. `contracts/principal_manager`, `contracts/pt_token`, `contracts/yt_token`
3. `contracts/oracle_adapter`, `contracts/sy_wrapper`
4. `contracts/market_factory`, `contracts/router`, `contracts/market_config`,
   `contracts/recovery_escrow`, `contracts/risk_control`, `contracts/permissioning`,
   `contracts/compliance`

Every contract directory under `contracts/` is covered. Each pass re-ran the relevant `cargo
test` suites (all green beforehand) and, for market_factory, the live-wasm
`tests/create_market.rs` integration test against a real release build.

## Finding fixed

| Severity | Finding | Fix |
|---|---|---|
| Medium | `OracleAdapter.set_value_bounds` never checked the new range against the value already on record. An admin calling it with a range excluding the live price (a plausible mistake: re-tightening bounds, a unit miscalculation, forgetting the live price) would permanently brick the feed — no future submission could ever be both "within bounds" and "not a decrease" — a self-inflicted version of the exact failure class this feature exists to prevent. | `set_value_bounds` now reads the stored price (if any) and rejects a range that excludes it (`Error::InvalidValue`). New test `set_value_bounds_rejects_a_range_excluding_the_current_price` covers both directions and the inclusive boundary. |

## Low/Informational items closed for consistency (not required, done anyway)

- `PrincipalManager::accrue_fee` and `YTToken::seize`'s pending-claim transfer still used raw `+`
  for their accumulator updates, relying entirely on the workspace's `overflow-checks = true`
  profile to fail closed with an untyped panic — the same class of gap the gap-closing pass
  already fixed everywhere else. Both now use `checked_add` and raise this contract's own
  `ArithmeticOverflow` error, for consistency with every other arithmetic site touched that round.
- `Router`'s true flash-mint wrappers (`flash_mint_yt`, `flash_mint_yt_from_underlying`) had no
  dedicated test proving a deauthorized user gains nothing from them specifically — added to the
  existing `router_acts_as_the_user_so_a_deauthorized_user_gains_nothing_by_using_it` test, which
  already covers every other Router entrypoint this way.
- `MarketFactory`'s `TopologyMismatch` error doc comment now states precisely what the check
  proves (salt-arithmetic consistency) versus what actually prevents a skipped phase from causing
  harm (Soroban's own atomic, fail-closed cross-contract call semantics) — found as a naming/scope
  clarity nuance during the audit, not a code defect, but worth being exact about.

## Reviewed and found sound — no fix needed

- **MarketPool + math**: rounding direction, curve unit discipline, `flash_mint_yt`'s pinned
  nested-auth entry, reentrancy, access control, and the `ceil_div` first-deposit-ratio fix were
  all independently re-derived and confirmed correct. Two pre-existing, non-regression
  Informational notes (`zap_split`'s bisection precision at very large inputs — self-correcting via
  refunds, never an overpay; `remove_liquidity`'s lack of a `MIN_RESERVE` floor — matches the
  standard locked-`MINIMUM_LIQUIDITY` pattern, no exploit path).
- **PrincipalManager / PTToken / YTToken**: the `mul_div`/`ceil_mul_div` wiring from the
  gap-closing pass was checked call-site-by-call-site against the original raw expressions (no
  operand transposition), confirmed behaviorally identical on the non-overflow path, and the three
  new overflow-regression tests were independently re-derived arithmetically (not just trusted) to
  confirm they trip the exact claimed overflow site. Solvency, the monotonic yield index,
  settlement boundaries, and SEP-41 conformance re-derived and confirmed sound.
- **OracleAdapter / SYWrapper**: the value-bounds ordering, the first-value exemption still being
  covered, and the deviation-overflow-to-graceful-rejection fix were all confirmed genuinely
  unreachable-by-panic now (re-derived with `i128::MAX`-magnitude worked examples). SYWrapper's
  1:1 exchange-rate invariant, dual-sided compliance gating, and seize/pause-exemption logic
  re-derived and confirmed sound.
- **MarketFactory / Router / MarketConfig / RecoveryEscrow / RiskControl / Permissioning /
  Compliance**: the `deployed_addr` refactor was confirmed functionally identical to the closures
  it replaced (by diff and by two passing tests, one against real compiled Wasm). Whether
  `TopologyMismatch` could be bypassed by skipping a phase entirely was traced through to Soroban's
  own call semantics and confirmed it cannot cause harm (see the doc-comment fix above). Every
  other claim re-derived in this pass — RecoveryEscrow cannot seize more than a target's real
  balance or bypass live deauthorization (both independently bounded by each child contract,
  confirmed by a live `>balance` test), RiskControl fails closed on every edge case including an
  uninitialized/no-consumers state, Permissioning's allow-list sense is applied consistently, and
  no transfer path in this layer bypasses `principal_compliance` — held up with no gaps found.

## Verification

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace` (all green across all 12 contract crates plus
`principal_integration_tests`), and `cargo build --workspace --release --target wasm32v1-none`
(every contract's Wasm still well under Soroban's 128 KiB limit; largest is `market_pool` at
~77 KB) all pass after the fix above.

## Go/no-go

**GO.** No Critical or High findings across any contract. One Medium, fixed and tested. A handful
of Low/Informational items, three closed for consistency, the remainder genuinely pre-existing
design characteristics with no exploit path, not introduced by any change reviewed in this pass.
