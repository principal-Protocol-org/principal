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

Most regulated RWAs on Stellar are issued as Stellar Assets, with compliance enforced natively through their Stellar Asset Contract (SAC) — authorization and clawback the issuer already manages. That compliance model doesn't automatically extend to a PT/YT market built on top of the asset: without inheriting it, an ineligible wallet could hold or trade PT/YT for an asset it isn't authorized to hold, and an issuer clawback of the underlying wouldn't reach the corresponding PT/YT position. Principal closes this gap with native compliance inheritance rather than a separate, Principal-managed allow-list that could drift out of sync with the issuer's own decisions — see [Compliance inheritance](#key-design-properties) below. This supports the different compliance models Stellar RWA issuers actually use: money-market funds such as BENJI rely on both authorization and clawback, while tokenized notes such as USDY have permissionless secondary transfers but retain clawback as a key issuer control. Principal preserves whichever controls are active, for both classes.

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

The protocol is composed of ten Soroban contracts organized in four layers (see [docs/TECHNICAL_SPECIFICATION.md](docs/TECHNICAL_SPECIFICATION.md) for the full spec).

### Infrastructure layer (shared across all markets)

| Contract | Role |
|---|---|
| `OracleAdapter` | Reference-value feed with primary/fallback source, freshness, and deviation checks. For the USDY market, the primary source is the RedStone USDY/USD SEP-40 feed. |
| `Permissioning` | An optional, narrower eligibility configuration surface — administered by the same operator as the rest of the market, not a separate Principal-controlled registry (see below) |
| `RiskControl` | Global pause, multi-pauser roles, rolling 24h circuit breaker |

### Tokenization layer (per underlying asset)

| Contract | Role |
|---|---|
| `SYWrapper` | Wraps the underlying asset into standardized SY shares; exchange rate grows if the underlying rebases, otherwise value is tracked through the oracle. Deposit, withdraw, and transfer inherit the underlying SAC's own `authorized()` as the mandatory compliance floor. `seize()` lets a configured `RecoveryEscrow` forcibly recover a deauthorized account's balance. |
| `PrincipalManager` | Mints real PT + YT from a user's real SY shares (taking custody via `SYWrapper.transfer`); redeems both at maturity by burning them and releasing real underlying via `SYWrapper.withdraw`. Mint and redeem inherit the same SAC-authorization floor. |
| `RecoveryEscrow` | The central compliance-recovery component. Authenticates the underlying SAC's real, current administrator (read live) and orchestrates `seize` across `SYWrapper`/`PTToken`/`YTToken` — and, once built, LP positions — see [Compliance below](#compliance-is-inherited-directly-from-the-underlying-sac). |

### Market layer (per maturity date)

| Contract | Role |
|---|---|
| `PTToken` | Standalone SEP-41 Principal Token, already implemented and exercised on Testnet. Transfers inherit the underlying SAC's `authorized()` floor on both sender and recipient. |
| `YTToken` | Standalone SEP-41 Yield Token with continuous yield accrual and claiming, already implemented and exercised on Testnet, gated the same way as PTToken. |
| `MarketPool` | Yield-curve AMM for PT ↔ SY trading (time-aware, no LP impermanent loss from time decay) — the next core component to implement, alongside `Router` |
| `Router` | Single-transaction orchestration: wrap, mint, swap, recombine, redeem, and liquidity operations |

`PrincipalManager` mints and burns through the real `PTToken`/`YTToken` contracts — PT and YT are genuine SEP-41 balances, holdable in any wallet, not tracked in `PrincipalManager`'s own storage. Compliance recovery covers SY, PT, and YT end to end today, including `RecoveryEscrow.finalize_pt`/`finalize_yt`, and extends to LP positions once `MarketPool` ships.

---

## Key Design Properties

**Fixed-income from variable yield** — PT holders receive a known value at maturity regardless of whether the underlying USDY yield increases or decreases. PT behaves like a zero-coupon bond on the underlying position.

**Yield market** — YT gives direct, capital-efficient exposure to future yield. Buying YT is economically equivalent to a leveraged long position on the underlying asset's yield rate.

**Time-aware AMM** — `MarketPool` uses a constant-power-sum invariant parameterized by time to maturity. The curve automatically shifts so PT converges to par at expiry, eliminating the structural impermanent loss that would occur in a standard AMM.

**Single liquidity pool** — PT and YT both trade through a single PT/SY pool. YT trading is routed through a flash-mint pattern, avoiding pool fragmentation and concentrating LP capital.

### Compliance is inherited directly from the underlying SAC

The current administrator of the underlying Stellar Asset Contract (SAC) controls market creation and every compliance right in that market, including its maturity and fee parameters (see [Business Model](#business-model)). Compliance controls applied to SY, PT, YT, and LP positions are inherited directly from those active on the underlying asset: if the SAC requires authorization, that requirement applies to every derived position; if the SAC imposes no such requirement, Principal adds no restriction of its own by default. This is enforced at the contract level, not just by convention — `initialize` on `SYWrapper`, `PrincipalManager`, `PTToken`, and `YTToken` each require `admin == underlying_SAC.admin()` (read live), so a market can only be created with the issuer's real participation.

`Permissioning` gives that same administrator an optional, narrower configuration surface on top of the SAC floor — for example, distinct eligibility for PT versus YT. It's deployed and administered by the same operator as the rest of the market, not a separate Principal-managed registry, and it can only narrow eligibility, never loosen it below what the SAC already allows.

`RecoveryEscrow` is the central compliance-recovery component. If the underlying SAC's real, current administrator deauthorizes an account, they can recover that account's derived positions without affecting any other depositor: SY is reconverted into the underlying asset immediately, since it carries no maturity; PT and YT remain fully backed inside `RecoveryEscrow` until maturity, then settle into the underlying asset so the issuer can execute their native SAC clawback. The same recovery path extends to LP positions once `MarketPool` ships.

**Asset-agnostic** — The SYWrapper and PrincipalManager are designed for any Stellar yield-bearing asset. USDY is the first market; the same contracts extend to BENJI, USTBL, or any future RWA.

**Stellar-native** — All contracts use Soroban storage tiers (`instance` / `persistent`), `require_auth()`, `#[contracttype]` typed keys, `#[contracterror]` typed errors, and SEP-41 for tokens.

---

## User Flows

### Buy PT (fixed income)
```
USDY → SYWrapper → SY shares → MarketPool (swap SY for PT)
Redeem at maturity: PT → principal value in USDY
```

### Buy YT (yield exposure)
```
USDY → Router (flash-mint) → YT-USDY
Claim yield incrementally or redeem all at maturity
```

### Provide liquidity
```
PT + SY → MarketPool → LP tokens
Earn swap fees; no time-decay impermanent loss
```

### Full exit before maturity
```
PT + YT (equal amounts) → PrincipalManager.recombine() → SY → USDY
```

---

## Settlement Mathematics

All arithmetic uses fixed-point with `SCALE = 10_000_000` (10^7). Oracle rates are stored at this scale: 1.03 USDC per underlying = `10_300_000`. Let `final_rate` be the oracle value at redemption and `initial_rate` the value stored at each user's mint time.

```
PT holder receives:  floor(pt_amount * SCALE / final_rate)          underlying tokens
YT holder receives:  floor(yt_amount * max(0, final_rate - initial_rate) / final_rate)  underlying tokens
```

`pt_amount` and `yt_amount` are in USDC-notional units at SCALE. Dividing by `final_rate` (also at SCALE) converts back to underlying token units. `initial_rate` is per-user and ensures YT captures only yield accrued since that user's mint. If `final_rate ≤ initial_rate` (no yield), YT holders receive zero — PT principal is always protected. Settlement uses floor rounding; rounding residuals accumulate in a protocol-governed reserve.

---

## Business Model

Each Principal market — one per underlying asset and maturity — carries three configurable fees, all set by the RWA issuer creating the market:

| Fee | Charged on | Example |
|---|---|---|
| Tokenization fee | Underlying tokenized into PT/YT | 5 bps |
| YT fee | Yield accrued by YT holders | 10% |
| Swap fee | Each PT trade, decreasing as maturity approaches: `Fee Tier × Days to Maturity / 365` | 0.1% Fee Tier |

Principal's protocol share of these fees is itself configurable, initially set at 20% — the remaining 80% goes to the market creator, i.e. the underlying SAC's current administrator.

Go-to-market starts with a single USDY market, targeting USDY holders and treasuries seeking fixed yield, and vault managers and DeFi funds seeking exposure to future rates. The same fee structure extends to every additional maturity and RWA issuer as adoption grows, so multiple markets generate fees in parallel.

---

## Repository Layout

```
contracts/
  oracle_adapter/        — reference value oracle (RedStone USDY/USD SEP-40 feed for the USDY market)
  permissioning/         — optional, admin-controlled eligibility configuration (see README's compliance section)
  risk_control/          — pause, pauser roles, rolling circuit breaker
  sy_wrapper/            — standardized yield wrapper (SY-USDY); seize() for compliance recovery
  principal_manager/     — tokenization engine: mints/burns PT and YT internally
  pt_token/              — standalone SEP-41 Principal Token
  yt_token/              — standalone SEP-41 Yield Token with yield accrual/claiming
  recovery_escrow/       — authenticates the issuer's SAC admin; orchestrates seize across SY/PT/YT

Cargo.toml               — workspace (Soroban SDK 26.x, Rust 2021)

docs/
  TECHNICAL_SPECIFICATION.md — full protocol spec, AMM math, settlement, storage
  ARCHITECTURE.md            — contract diagrams, sequence flows, deployment order
  PROOF_OF_CONCEPT.md        — implemented scope, what is built, how to run it
  SECURITY.md                — threat model, per-contract security properties
  DEPLOYMENT.md              — Stellar CLI deployment guide
  CONTRIBUTING.md            — development workflow, code style, PR checklist
```

---

## Quick Start

**Requirements:** Rust stable ≥ 1.79, `wasm32-unknown-unknown` target, Stellar CLI ≥ 22.0.

```bash
# Add WASM target (once)
rustup target add wasm32-unknown-unknown

# Run all unit tests
cargo test

# Build all WASM artifacts
cargo build --target wasm32-unknown-unknown --release
```

WASM artifacts are produced in `target/wasm32-unknown-unknown/release/`.

See [docs/PROOF_OF_CONCEPT.md](docs/PROOF_OF_CONCEPT.md) for the current implemented scope and test instructions.  
See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for testnet and mainnet deployment.

---

## Documentation

| Document | Contents |
|---|---|
| [docs/TECHNICAL_SPECIFICATION.md](docs/TECHNICAL_SPECIFICATION.md) | Full protocol spec: all ten contracts, AMM invariant, settlement math, fee structure, storage design, error codes, constants |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Contract interaction diagrams, sequence flows (mint, swap, redeem, flash-mint YT), AMM curve, deployment order |
| [docs/PROOF_OF_CONCEPT.md](docs/PROOF_OF_CONCEPT.md) | Eight implemented contracts, what they demonstrate, test coverage, build instructions |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model, per-contract security properties, incident response |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Step-by-step Stellar CLI deployment for testnet and mainnet |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, code style, PR checklist |

---

## License

Apache 2.0
