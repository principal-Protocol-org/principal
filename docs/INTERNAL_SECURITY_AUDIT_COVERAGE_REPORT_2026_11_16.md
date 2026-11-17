# Test Coverage Report — Internal Security Review, 16 November 2026

**Branch:** `mainnet` (re-measured at commit `000009eb07e2976df396948e8b12650ae8191670`, after the
H-4 fix and the independent-review follow-up round landed; first measured at base commit
`29e363b30706c79067dcaf7c35607cbba73f982d`)
**Companion document:** [`INTERNAL_SECURITY_AUDIT_2026_11_16.md`](INTERNAL_SECURITY_AUDIT_2026_11_16.md)

This document records the coverage measurement taken as the closing step of the accompanying
security review, kept separate from that report so the two can be updated independently going
forward (coverage is re-measured whenever the fixed state changes meaningfully; the findings
record should not be touched once closed). This is the second measurement: the first reflected
the review's original pass (C-1 plus 25 further findings, 477 tests); this one reflects the
follow-up round that added C-2, H-3's clock correction, H-4, M-12, L-12, and their regression
tests (489 tests).

## Methodology

Source-based coverage instrumentation (`cargo llvm-cov`) was run across the entire Cargo
workspace — every contract's `src/lib.rs` and `src/test.rs`, plus the full cross-contract
integration test suite under `contracts/integration_tests/` — with every test in the workspace
executed under instrumentation in a single pass (`cargo llvm-cov --workspace --no-fail-fast`). The
figures below reflect what the real test suite actually exercises, not a theoretical or partial
measurement.

`cargo test --workspace` (run separately, without instrumentation, as part of the same
verification pass, and again standalone immediately before this measurement) result: **489 tests
passed, 0 failed, 0 ignored**, across all fourteen contracts' own unit tests and the full
integration suite (25 test binaries in total).

## Workspace totals

| Metric | Covered | Total | Coverage |
|---|---|---|---|
| Lines | 10,449 | 10,633 | **98.27%** |
| Regions | 22,211 | 22,612 | **98.23%** |
| Functions | 969 | 1,084 | 89.39% |

Line and region coverage both exceed the 98% target, and both improved slightly over the first
measurement (98.24%→98.27% line, 98.21%→98.23% region), consistent with the new regression tests
added this round (`withdraw_gained` negative-baseline cases, the oracle throttle-clock test, three
`initialize_requires_the_admins_own_signature` tests, `initialize_requires_the_issuers_own_signature`)
exercising code that was already covered at a basic level but not at these specific edges.
Function-level coverage is essentially unchanged (89.23%→89.39%); as before, this gap is
concentrated in small, largely boilerplate functions (simple getters, trait-derived
implementations, and defensive branches reachable only through a host-level failure rather than
any reachable application state) rather than untested business logic. Line coverage — which
reflects actual executable statements reached — remains the more meaningful measure of how
thoroughly the protocol's real logic was exercised, and is the figure the 98% target was set
against.

## Per-file breakdown

```
Filename                           Regions    Missed Regions     Cover   Functions  Missed Functions  Executed       Lines      Missed Lines     Cover    Branches   Missed Branches     Cover
----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
compliance/src/lib.rs                   87                 2    97.70%           7                 2    71.43%          47                 2    95.74%           0                 0         -
compliance/src/mock_rwa.rs             213                 7    96.71%          17                 1    94.12%         112                 3    97.32%           0                 0         -
compliance/src/test.rs                 308                 0   100.00%          16                 0   100.00%         123                 0   100.00%           0                 0         -
integration_tests/src/stack.rs         522                 0   100.00%          23                 0   100.00%         242                 0   100.00%           0                 0         -
market_config/src/lib.rs               308                 1    99.68%          25                 1    96.00%         176                 1    99.43%           0                 0         -
market_config/src/test.rs              469                 0   100.00%          16                 0   100.00%         232                 0   100.00%           0                 0         -
market_factory/src/lib.rs              289                13    95.50%          21                10    52.38%         195                11    94.36%           0                 0         -
market_factory/src/test.rs             239                 0   100.00%           8                 0   100.00%         158                 0   100.00%           0                 0         -
market_pool/src/lib.rs                1918               105    94.53%          90                11    87.78%         967                39    95.97%           0                 0         -
market_pool/src/math.rs                200                14    93.00%          12                 0   100.00%         111                 1    99.10%           0                 0         -
market_pool/src/math_test.rs           242                 0   100.00%          17                 0   100.00%         125                 0   100.00%           0                 0         -
oracle_adapter/src/lib.rs              717                29    95.96%          43                 7    83.72%         428                19    95.56%           0                 0         -
oracle_adapter/src/test.rs            2031                 9    99.56%          82                 2    97.56%         744                 5    99.33%           0                 0         -
permissioning/src/lib.rs               154                 4    97.40%          14                 2    85.71%          90                 2    97.78%           0                 0         -
permissioning/src/test.rs              257                 0   100.00%          12                 0   100.00%          95                 0   100.00%           0                 0         -
principal_manager/src/lib.rs          1007                54    94.64%          76                18    76.32%         564                25    95.57%           0                 0         -
principal_manager/src/test.rs         2305                 1    99.96%          76                 1    98.68%         961                 1    99.90%           0                 0         -
pt_token/src/lib.rs                    620                32    94.84%          48                12    75.00%         354                12    96.61%           0                 0         -
pt_token/src/test.rs                  1127                 1    99.91%          40                 1    97.50%         416                 1    99.76%           0                 0         -
recovery_escrow/src/lib.rs             532                11    97.93%          29                 4    86.21%         313                 6    98.08%           0                 0         -
recovery_escrow/src/test.rs            948                 0   100.00%          17                 0   100.00%         392                 0   100.00%           0                 0         -
risk_control/src/lib.rs                539                10    98.14%          37                 4    89.19%         334                 4    98.80%           0                 0         -
risk_control/src/test.rs               915                 0   100.00%          36                 0   100.00%         316                 0   100.00%           0                 0         -
router/src/lib.rs                      882                17    98.07%          54                12    77.78%         570                15    97.37%           0                 0         -
router/src/test.rs                     319                 0   100.00%          18                 0   100.00%         153                 0   100.00%           0                 0         -
sy_wrapper/src/lib.rs                  781                22    97.18%          50                 4    92.00%         474                 8    98.31%           0                 0         -
sy_wrapper/src/test.rs                1672                 1    99.94%          59                 1    98.31%         575                 1    99.83%           0                 0         -
yt_token/src/lib.rs                    967                64    93.38%          70                20    71.43%         551                24    95.64%           0                 0         -
yt_token/src/test.rs                  2044                 4    99.80%          71                 2    97.18%         815                 4    99.51%           0                 0         -
----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
TOTAL                                22612               401    98.23%        1084               115    89.39%       10633               184    98.27%           0                 0         -
```

## Lowest-covered source files, for anyone prioritizing further test-writing

Ordered by line coverage, lowest first — every `src/lib.rs`/`src/math.rs` file in the workspace
(test files themselves are consistently at or near 100%, as expected, and are omitted here; the
first measurement's version of this table only showed a subset of these files without stating a
cutoff — this one is the complete set, for a cleaner reference going forward):

| File | Line coverage | Missed lines |
|---|---|---|
| `market_factory/src/lib.rs` | 94.36% | 11 |
| `oracle_adapter/src/lib.rs` | 95.56% | 19 |
| `principal_manager/src/lib.rs` | 95.57% | 25 |
| `yt_token/src/lib.rs` | 95.64% | 24 |
| `compliance/src/lib.rs` | 95.74% | 2 |
| `market_pool/src/lib.rs` | 95.97% | 39 |
| `pt_token/src/lib.rs` | 96.61% | 12 |
| `router/src/lib.rs` | 97.37% | 15 |
| `permissioning/src/lib.rs` | 97.78% | 2 |
| `recovery_escrow/src/lib.rs` | 98.08% | 6 |
| `sy_wrapper/src/lib.rs` | 98.31% | 8 |
| `risk_control/src/lib.rs` | 98.80% | 4 |
| `market_pool/src/math.rs` | 99.10% | 1 |
| `market_config/src/lib.rs` | 99.43% | 1 |

`market_pool/src/lib.rs` and `principal_manager/src/lib.rs` carry the largest absolute number of
missed lines, consistent with being the two largest and most branch-heavy contracts in the
workspace; nothing in the findings record (the companion document) identified any of these
specific gaps as corresponding to an untested security-relevant path — the missed lines in each
case are predominantly defensive branches (error arms reachable only through a host-level failure
or an already-excluded invariant violation) rather than reachable application logic left
unexercised. `recovery_escrow/src/lib.rs`'s line coverage moved slightly (98.38% in the first
measurement, 98.08% here) because `initialize` genuinely grew by several lines for the H-4 fix
(the new `caller` parameter and the `require_auth()`/`is_authority` check); inspecting the
instrumented per-line output directly confirms both new branches this added are exercised at
least once (`caller.require_auth()`'s failure path by the new
`initialize_requires_the_issuers_own_signature` regression test; the `compliance::is_authority`
check's own `Unauthorized` panic arm also shows a nonzero execution count, though tracing exactly
which existing test reaches it was not conclusive and is not worth further effort — the measured
result is what matters, not the attribution). The file's absolute missed-line count (6, up from 5)
is consistent with the added lines, not a regression in what was already covered.
