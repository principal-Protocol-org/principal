# Principal Protocol

**A Soroban-native yield tokenization protocol for regulated RWAs on Stellar, with a native compliance layer.**

Principal Protocol splits a regulated, yield-bearing real-world asset into two independently tradable instruments: a **Principal Token (PT)** that delivers a fixed, predictable return at maturity, and a **Yield Token (YT)** that captures all variable yield generated between issuance and maturity. Every derived position — SY, PT, YT, and LP — inherits its compliance controls directly from the underlying asset's own Stellar Asset Contract (SAC), so regulated RWAs stay regulated all the way through the protocol, not just at the point of deposit.

The first supported market targets **Ondo USDY on Stellar** — a tokenized US Treasury-backed note. The architecture is designed to support any regulated Stellar RWA.

---

## Why Principal Protocol

Stellar already hosts significant tokenized real-world assets — USDY (Ondo), BENJI (Franklin Templeton), USTBL (Spiko) and others — but users currently have no infrastructure to:

- **Lock in a fixed yield** from variable-yield RWA assets.
- **Sell future yield upfront** for immediate liquidity.
- **Express a directional view** on future RWA yield rates.
- **Hedge interest-rate risk** by separating principal and yield exposure.

Principal Protocol fills this gap by creating a dedicated fixed-income and yield market layer on top of Stellar's existing RWA ecosystem.

---

## Market Opportunity & Originality

Stellar's tokenized RWA ecosystem has already grown past $3.2B, led by issuers such as Ondo, Franklin Templeton, and Spiko, with DTC-tokenized Treasury bills, bonds, and notes expected on Stellar in the first half of 2027. Despite that growth, Stellar still has no dedicated fixed-income or yield-trading infrastructure for these assets — no way to lock in a fixed yield, sell future yield upfront, or take a directional position on future rates.

Most regulated RWAs on Stellar are issued as Stellar Assets (including SEP-8 regulated assets), with compliance enforced natively through their Stellar Asset Contract (SAC) — authorization and clawback the issuer already manages. Newer issuers use SEP-57 (T-REX) tokens, which have no SAC at all; Principal supports both (see [Compliance](#compliance-is-inherited-directly-from-the-underlying-asset)). That compliance model doesn't automatically extend to a PT/YT market built on top of the asset: without inheriting it, an ineligible wallet could hold or trade PT/YT for an asset it isn't authorized to hold, and an issuer clawback of the underlying wouldn't reach the corresponding PT/YT position. Principal closes this gap with native compliance inheritance rather than a separate, Principal-managed allow-list that could drift out of sync with the issuer's own decisions — see [Compliance inheritance](#key-design-properties) below. This supports the different compliance models Stellar RWA issuers actually use: money-market funds such as BENJI rely on both authorization and clawback, while tokenized notes such as USDY have permissionless secondary transfers but retain clawback as a key issuer control. Principal preserves whichever controls are active, for both classes.

The yield-tokenization model itself is proven — Pendle, the category-defining PT/YT protocol, has passed $1B in TVL across EVM chains. Principal brings the same primitive to Stellar, purpose-built for the compliance requirements regulated RWAs carry, which a permissionless-collateral design was never built to handle.

---

## How it works

```
User deposits USDY
        │
        ▼
   SYWrapper  ──────────────── issues SY-USDY shares
   (standardized yield          (exchange rate grows as
    wrapper)                     yield accrues)
        │
        ▼
PrincipalManager  ─────────── splits SY shares into:
(tokenization engine)
        │
        ├──── PT-USDY  ── fixed principal claim, redeemable at maturity
        │                  (zero-coupon bond on yield)
        │
        └──── YT-USDY  ── all yield generated until maturity
                           (decays to zero at expiry)

At maturity:
  OracleAdapter provides final USDY/USD rate (RedStone SEP-40 feed)
  PT holders → receive principal in USDY
  YT holders → receive accumulated yield in USDY
```

**Example:** A user deposits 100 USDC worth of USDY with a 3-month maturity. They receive PT-USDY (worth 100 USDC at maturity) and YT-USDY (capturing the yield). If USDY yields 4% annualized, the YT holder receives ~1 USDC of yield over the period, while the PT holder always receives 100 USDC of value at maturity regardless of rate movements.

---

## Protocol Architecture

The protocol is composed of **eleven Soroban contracts** organized in four layers, plus one shared library crate (`principal_compliance`). See [docs/TECHNICAL_SPECIFICATION.md](docs/TECHNICAL_SPECIFICATION.md) for the full spec and [docs/API_REFERENCE.md](docs/API_REFERENCE.md) for every function.

### Infrastructure layer (shared across all markets)

| Contract | Role |
|---|---|
| `OracleAdapter` | Monotonic reference-value feed (USDC per underlying) with freshness checks. For the USDY market, fed from the RedStone USDY/USD SEP-40 feed by a relay. |
| `Permissioning` | An optional, narrower eligibility configuration surface — administered by the market's operator, never a Principal-controlled registry (see below) |
| `RiskControl` | Global pause, multi-pauser roles, and a circuit breaker on deposit volume: a protocol-wide **and** a per-asset limit over a window counted in **ledger sequence numbers**. Wired directly into `SYWrapper.deposit` and `PrincipalManager.mint`, so an over-limit call reverts by itself. |
| `principal_compliance` (library) | The adapter every contract uses to ask "may this account hold the underlying?" and "who is the issuer authority?" — for a classic/SEP-8 asset via its SAC, for a SEP-57 RWA token via frozen-flag + identity verifier + operator role. |

### Tokenization layer (per underlying asset)

| Contract | Role |
|---|---|
| `SYWrapper` | Wraps the underlying into standardized SY shares. Slippage-protected `deposit`/`withdraw`, a per-address deposit cap, and operations gated by the underlying's own compliance rules on both sides. `seize()` lets the configured `RecoveryEscrow` recover a deauthorized account's balance (also while paused). |
| `PrincipalManager` | Mints real PT + YT from a user's real SY (taking custody), charges the tokenization and YT fees, `recombine`s PT + YT before maturity, freezes one settlement rate with the permissionless `settle_all()`, and redeems both at maturity. |
| `MarketConfig` | Per-market configuration: maturity, **tokenization fee**, **YT fee**, **swap Fee Tier**, and the protocol/creator fee split (initially 20 % / 80 %). Fees are set by the underlying's *current* issuer authority, read live; the creator's 80 % is paid to that same authority. |
| `RecoveryEscrow` | The compliance-recovery component. Re-checks the issuer authority live on every call (no key of its own) and seizes SY, PT, YT **and LP**; batch seizure for several accounts in one transaction; a per-account `RecoveryRecord` traces recovered funds to the event that produced them. |

### Market layer (per maturity)

| Contract | Role |
|---|---|
| `PTToken` | SEP-41 Principal Token; both-sides compliance on every transfer; `seize` for recovery. |
| `YTToken` | SEP-41 Yield Token with a `1/rate` yield index (exactly solvent at any mint rate), claimable any time, frozen at maturity; same gating and `seize`. |
| `MarketPool` | The time-aware PT/SY yield-curve AMM (constant power sum `x^a + y^a = k`, `a = 1 − τ/S`): PT converges to par at maturity with no time-decay impermanent loss. Swaps, proportional and single-sided liquidity, compliance-gated LP positions, the flash-redeem YT path, and swap fees that decay as `Fee Tier × Days to Maturity / 365`. |
| `Router` | Stateless single-transaction flows — wrap-and-mint, swaps, **flash-mint** YT, flash-redeem YT, liquidity, recombine, redeem — each with a `deadline` and `min_out`. Acts as the user, holds nothing. |

`PrincipalManager` mints and burns through the real `PTToken`/`YTToken` contracts — PT and YT are genuine SEP-41 balances, holdable in any wallet. Compliance recovery covers SY, PT, YT and LP end to end.

---

## Key Design Properties

**Fixed-income from variable yield** — PT holders receive a known value at maturity regardless of whether the underlying USDY yield increases or decreases. PT behaves like a zero-coupon bond on the underlying position.

**Yield market** — YT gives direct, capital-efficient exposure to future yield. Buying YT is economically equivalent to a leveraged long position on the underlying asset's yield rate.

**Time-aware AMM** — `MarketPool` uses a constant-power-sum invariant parameterized by time to maturity. The curve automatically shifts so PT converges to par at expiry, eliminating the structural impermanent loss that would occur in a standard AMM. Proven by tests, not asserted: with no trades at all, PT's price climbs from 0.99376 to 0.99997 over 179 days following the closed form, and reaches par at maturity ([docs/AMM_DESIGN.md](docs/AMM_DESIGN.md)).

**Single liquidity pool** — PT and YT both trade through a single PT/SY pool. Buying YT is a flash-mint (mint PT + YT, sell the PT into the pool); selling YT is a flash-redeem (the pool recombines the YT with its own PT), each atomic in one transaction — no pool fragmentation, no loan, and the pool can never be left short.

### Compliance is inherited directly from the underlying asset

The current issuer authority of the underlying asset controls market creation and every compliance right in that market, including its maturity and fee parameters (see [Business Model](#business-model)). Compliance controls applied to SY, PT, YT and LP positions are inherited directly from those active on the underlying: if the underlying requires authorization, that requirement applies to every derived position; if it imposes none, Principal adds no restriction of its own by default. This is enforced at the contract level, not by convention — every market contract's `initialize` requires the caller to be the underlying's issuer authority, read live, so a market can only be created with the issuer's real participation.

| Underlying | "May this account hold it?" | "Who is the issuer authority?" |
|---|---|---|
| Classic Stellar asset, incl. **SEP-8** regulated assets (via the SAC) | `SAC.authorized(account)` | `SAC.admin()` |
| **SEP-57 (T-REX)** `RWAToken` (no SAC) | not frozen **and** `IdentityVerifier.verify_identity` succeeds | the `operator` role, proven by an idempotent capability probe |

Two things follow that an integrator must know, both detailed in [docs/COMPLIANCE_ARCHITECTURE.md](docs/COMPLIANCE_ARCHITECTURE.md): a **SEP-8** issuer's authorize→pay→deauthorize "sandwich" cannot wrap a single-operation Soroban call, so holders must be left *persistently* authorized once approved (and off-chain approval criteria are not visible to contracts); and for **SEP-57** only the frozen flag and identity are evaluated on PT/YT/LP transfers, with the token's Compliance modules running when SY is deposited or withdrawn.

`Permissioning` gives the market's operator an optional, narrower configuration surface on top of that floor — for example, distinct eligibility for PT versus YT. It can only narrow eligibility, never loosen it below what the underlying already allows.

`RecoveryEscrow` is the central compliance-recovery component. If the underlying's real, current issuer authority deauthorizes an account, they can recover that account's derived positions without affecting any other depositor: SY is reconverted into the underlying asset immediately, since it carries no maturity; LP is burned for its PT + SY (SY leg reconverted at once); PT and YT remain fully backed inside `RecoveryEscrow` until maturity, then settle into the underlying so the issuer can execute their native clawback. Several accounts can be recovered in one transaction, and every event leaves a per-account record.

**Asset-agnostic** — The SYWrapper and PrincipalManager are designed for any Stellar yield-bearing asset, classic or SEP-57. USDY is the first market; the same contracts extend to BENJI, USTBL, or any future RWA.

**Stellar-native** — All contracts use Soroban storage tiers (`instance` / `persistent`), `require_auth()`, `#[contracttype]` typed keys, `#[contracterror]` typed errors, and SEP-41 for tokens.

---

## User Flows

### Buy PT (fixed income)
```
USDY → Router.swap_sy_for_pt   (wrap → SY → MarketPool)
Redeem at maturity: PT → principal value in USDY, at the frozen settlement rate
```

### Buy YT (yield exposure) — flash-mint
```
SY → Router.swap_sy_for_yt: mint PT + YT, sell the PT into the pool, keep YT + SY proceeds
Claim yield any time (PrincipalManager.claim_yield) or redeem all at maturity
```

### Sell YT — flash-redeem
```
YT → MarketPool.swap_yt_for_sy: pool recombines the YT with its own PT, pays you the difference
```

### Provide liquidity
```
PT + SY (or SY alone) → MarketPool → LP position
PT converges to par as maturity nears; no time-decay impermanent loss
```

### Full exit before maturity
```
PT + YT (equal amounts) → PrincipalManager.recombine() → SY → USDY
```

---

## Settlement Mathematics

All arithmetic uses fixed-point with `SCALE = 10_000_000` (10^7). Oracle rates are stored at this scale: 1.03 USDC per underlying = `10_300_000`. Minting `n` SY at rate `r₀` (after the tokenization fee) creates `N = n × r₀` notional of PT and of YT.

```
PT holder receives:  floor(N_pt × SCALE / final_rate)                    underlying
YT holder accrues:   N_yt × (1/r_settle − 1/r_now)                       underlying, any time
```

`final_rate` is the single rate frozen by `settle_all()` at maturity. The YT index is the closed form `1/rate`, so over a position's life PT + YT claims telescope to exactly `N / r₀ = n` — the SY deposited — for **any** mint rate and any number of oracle updates. (Worked example: 100 SY minted at 1.05 and settled at 1.10 pay 95.4545 to the PT and 4.5454 to the YT: total 100.) Yield stops accruing at maturity; rounding always favors the protocol. A full guide with real numbers is in [docs/YIELD_MATH_AND_FEES.md](docs/YIELD_MATH_AND_FEES.md).

---

## Business Model

Each Principal market — one per underlying asset and maturity — carries three configurable fees, all set by the underlying's issuer authority creating the market:

| Fee | Charged on | Example |
|---|---|---|
| Tokenization fee | Underlying tokenized into PT/YT | 5 bps |
| YT fee | Yield accrued by YT holders | 10% |
| Swap fee | Each PT trade, decreasing as maturity approaches: `Fee Tier × Days to Maturity / 365` | 0.1% Fee Tier |

Principal's protocol share of these fees is itself configurable, initially set at 20% — the remaining 80% goes to the market creator, i.e. the underlying's current issuer authority (for a Stellar asset, `SAC.admin()`, read at payout time). All three fees are accrued in SY inside `PrincipalManager` and `MarketPool` and paid out by permissionless `claim_protocol_fees` / `claim_creator_fees`; the payees are fixed by the market's `MarketConfig`, so calling them cannot redirect value. Caps: tokenization ≤ 1 %, YT fee ≤ 50 %, swap Fee Tier ≤ 5 %.

Go-to-market starts with a single USDY market, targeting USDY holders and treasuries seeking fixed yield, and vault managers and DeFi funds seeking exposure to future rates. The same fee structure extends to every additional maturity and RWA issuer as adoption grows, so multiple markets generate fees in parallel.

---

## Repository Layout

```
contracts/
  compliance/            — principal_compliance: SAC / SEP-8 / SEP-57 adapter (library) + test RWA token
  oracle_adapter/        — monotonic reference-value oracle
  permissioning/         — optional, admin-controlled eligibility configuration
  risk_control/          — pause, pauser roles, ledger-sequence circuit breaker (global + per-asset)
  market_config/         — per-market fees, fee split, swap-fee schedule
  sy_wrapper/            — standardized yield wrapper (SY); slippage, deposit cap, seize
  principal_manager/     — tokenization engine: mint, recombine, settle_all, redeem, fees
  pt_token/              — SEP-41 Principal Token
  yt_token/              — SEP-41 Yield Token with the 1/rate index
  market_pool/           — the PT/SY yield-curve AMM (+ fixed-point math in src/math.rs)
  router/                — stateless one-transaction user flows
  recovery_escrow/       — seize SY/PT/YT/LP, batch, per-account records
  integration_tests/     — full-stack fixture (src/stack.rs) and the cross-contract test suite

Cargo.toml               — workspace (Soroban SDK 26.x, Rust 2021)
.github/workflows/       — ci.yml (lint, wasm, tests, coverage, audit), docs.yml (publish docs)

docs/
  TRANCHE_1_DELIVERABLES.md  — acceptance document: deliverable → code → test → how to verify
  API_REFERENCE.md           — every function: what it does, needs, and what can go wrong
  YIELD_MATH_AND_FEES.md     — yield math, fees and compliance with real, test-asserted numbers
  COMPLIANCE_ARCHITECTURE.md — SAC, SEP-8 and SEP-57 handling, recovery
  AMM_DESIGN.md              — curve, arithmetic, rounding, research on existing Stellar projects
  TECHNICAL_SPECIFICATION.md — full protocol spec
  ARCHITECTURE.md            — contract diagrams, sequence flows, deployment order
  SECURITY.md                — threat model, per-contract security properties
  DEPLOYMENT.md              — Stellar CLI deployment guide
  CONTRIBUTING.md            — development workflow, code style, PR checklist
  PROOF_OF_CONCEPT.md, COMPLIANT_SETTLEMENT_DESIGN.md, TESTNET_*.md — history and evidence
```

---

## Quick Start

**Requirements:** Rust stable (≥ 1.84), the `wasm32v1-none` target, Stellar CLI ≥ 22.0. (`wasm32-unknown-unknown` no longer builds with Soroban SDK 26 on current Rust.)

```bash
# Add the WASM target (once)
rustup target add wasm32v1-none

# Run everything: unit + cross-contract integration tests
cargo test --workspace

# Coverage of production code (unit-test modules and the test crate excluded)
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --ignore-filename-regex '(/test\.rs|_test\.rs|/tests/|integration_tests|mock_rwa)' --summary-only

# Build all WASM artifacts
cargo build --workspace --release --target wasm32v1-none
```

WASM artifacts are produced in `target/wasm32v1-none/release/`. Build the pool once before `cargo test` to also run the on-WASM CPU-budget test.

See [docs/TRANCHE_1_DELIVERABLES.md](docs/TRANCHE_1_DELIVERABLES.md) for the acceptance evidence, [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for deployment.

---

## Documentation

| Document | Contents |
|---|---|
| [docs/TRANCHE_1_DELIVERABLES.md](docs/TRANCHE_1_DELIVERABLES.md) | Tranche 1 acceptance: each deliverable mapped to code and to the tests that prove it; the functional AMM success criteria; defects found; open items |
| [docs/API_REFERENCE.md](docs/API_REFERENCE.md) | Every function of every contract: what it does, what it needs, what can go wrong; events and types |
| [docs/YIELD_MATH_AND_FEES.md](docs/YIELD_MATH_AND_FEES.md) | The yield index, settlement, market fees, AMM pricing and compliance, with real numbers asserted by tests |
| [docs/COMPLIANCE_ARCHITECTURE.md](docs/COMPLIANCE_ARCHITECTURE.md) | Inheritance for SAC, **SEP-8** and **SEP-57** underlyings; what each cannot express; recovery |
| [docs/AMM_DESIGN.md](docs/AMM_DESIGN.md) | The yield-curve AMM: survey of existing Stellar projects, curve, fixed-point math, rounding, resource use |
| [docs/TECHNICAL_SPECIFICATION.md](docs/TECHNICAL_SPECIFICATION.md) | Full protocol spec: all contracts, settlement math, fee structure, storage, error codes, constants |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Contract interaction diagrams, sequence flows, deployment order |
| [docs/PROOF_OF_CONCEPT.md](docs/PROOF_OF_CONCEPT.md) | Implementation history and Testnet evidence |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model, per-contract security properties, incident response |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Step-by-step Stellar CLI deployment |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, code style, PR checklist |

---

## License

Apache 2.0
