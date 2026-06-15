# Principal Protocol

**Yield tokenization infrastructure for Stellar RWAs**

Principal Protocol is a Soroban-native protocol that splits yield-bearing assets into two independently tradable instruments: a **Principal Token (PT)** that delivers a fixed, predictable return at maturity, and a **Yield Token (YT)** that captures all variable yield generated between issuance and maturity.

The first supported market targets **Ondo USDY on Stellar** — a tokenized US Treasury-backed note. The architecture is designed to support any Stellar yield-bearing asset.

---

## Why Principal Protocol

Stellar already hosts significant tokenized real-world assets — USDY (Ondo), BENJI (Franklin Templeton), USTBL (Spiko) and others — but users currently have no infrastructure to:

- **Lock in a fixed yield** from variable-yield RWA assets.
- **Sell future yield upfront** for immediate liquidity.
- **Express a directional view** on future RWA yield rates.
- **Hedge interest-rate risk** by separating principal and yield exposure.

Principal Protocol fills this gap by creating a dedicated fixed-income and yield market layer on top of Stellar's existing RWA ecosystem.

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
  OracleAdapter provides final USDY/USDC rate
  PT holders → receive principal in USDY
  YT holders → receive accumulated yield in USDY
```

**Example:** A user deposits 100 USDC worth of USDY with a 3-month maturity. They receive PT-USDY (worth 100 USDC at maturity) and YT-USDY (capturing the yield). If USDY yields 4% annualized, the YT holder receives ~1 USDC of yield over the period, while the PT holder always receives 100 USDC of value at maturity regardless of rate movements.

---

## Protocol Architecture

The protocol is composed of nine Soroban contracts organized in three layers:

### Infrastructure layer (shared across all markets)

| Contract | Role |
|---|---|
| `OracleAdapter` | USDY/USDC reference value with freshness controls |
| `Permissioning` | Account and asset eligibility registry for permissioned RWAs |
| `RiskControl` | Global pause, multi-pauser roles, rolling 24h circuit breaker |

### Tokenization layer (per underlying asset)

| Contract | Role |
|---|---|
| `SYWrapper` | Wraps the underlying asset into standardized SY shares; exchange rate grows if the underlying rebases, otherwise value is tracked through the oracle |
| `PrincipalManager` | Mints PT + YT from SY shares; settles both at maturity |

### Market layer (per maturity date)

| Contract | Role |
|---|---|
| `PTToken` | Standalone SEP-41 Principal Token |
| `YTToken` | Standalone SEP-41 Yield Token with claimable yield |
| `MarketPool` | Yield-curve AMM for PT ↔ SY trading (time-aware, no LP impermanent loss from time decay) |
| `Router` | Single-transaction orchestration: wrap, mint, swap, recombine, redeem |

---

## Key Design Properties

**Fixed-income from variable yield** — PT holders receive a known value at maturity regardless of whether the underlying USDY yield increases or decreases. PT behaves like a zero-coupon bond on the underlying position.

**Yield market** — YT gives direct, capital-efficient exposure to future yield. Buying YT is economically equivalent to a leveraged long position on the underlying asset's yield rate.

**Time-aware AMM** — `MarketPool` uses a constant-power-sum invariant parameterized by time to maturity. The curve automatically shifts so PT converges to par at expiry, eliminating the structural impermanent loss that would occur in a standard AMM.

**Single liquidity pool** — PT and YT both trade through a single PT/SY pool. YT trading is routed through a flash-mint pattern, avoiding pool fragmentation and concentrating LP capital.

**Permissioned by design** — Eligibility constraints from USDY are preserved across all derived instruments. PT, YT, and SY transfers check the `Permissioning` registry so the protocol cannot be used as a compliance bypass for restricted participants.

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

## Repository Layout

```
contracts/
  oracle_adapter/        — USDY/USDC reference value oracle
  permissioning/         — account and asset eligibility registry
  risk_control/          — pause, pauser roles, rolling circuit breaker
  sy_wrapper/            — standardized yield wrapper (SY-USDY)
  principal_manager/     — tokenization engine: mints/burns PT and YT

Cargo.toml               — workspace (Soroban SDK 26.x, Rust 2021)
TECHNICAL_SPECIFICATION.md — full protocol spec, AMM math, settlement, storage
ARCHITECTURE.md          — contract diagrams, sequence flows, deployment order
PROOF_OF_CONCEPT.md      — current POC scope, what is built, how to run it
SECURITY.md              — threat model, per-contract security properties
DEPLOYMENT.md            — Stellar CLI deployment guide
AUDIT_REVIEW.md          — security findings tracker
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

See [PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md) for the current POC scope and test instructions.  
See [DEPLOYMENT.md](DEPLOYMENT.md) for testnet and mainnet deployment.

---

## Documentation

| Document | Contents |
|---|---|
| [TECHNICAL_SPECIFICATION.md](TECHNICAL_SPECIFICATION.md) | Full protocol spec: all nine contracts, AMM invariant, settlement math, fee structure, storage design, error codes, constants |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Contract interaction diagrams, sequence flows (mint, swap, redeem, flash-mint YT), AMM curve, deployment order |
| [PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md) | Current POC: five implemented contracts, what they demonstrate, test coverage, build instructions |
| [SECURITY.md](SECURITY.md) | Threat model, per-contract security properties, incident response |
| [DEPLOYMENT.md](DEPLOYMENT.md) | Step-by-step Stellar CLI deployment for testnet and mainnet |
| [AUDIT_REVIEW.md](AUDIT_REVIEW.md) | Security findings, status tracking, open items |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development workflow, code style, PR checklist |

---

## Development Roadmap

### Phase 1 — Proof of Concept (complete)
Five foundational contracts implemented and tested on Soroban:
OracleAdapter, Permissioning, RiskControl, SYWrapper, PrincipalManager.
See [PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md) for full details.

### Phase 2 — Full Protocol
- Standalone SEP-41 `PTToken` and `YTToken` with transfer permissioning and yield claiming.
- `MarketPool` — yield-curve AMM with time-aware invariant, built-in implied-rate oracle, and LP fee distribution.
- `Router` — single-transaction user flows including flash-mint YT and flash-redeem YT patterns.
- Fee governance with timelock, protocol treasury accumulation.
- Full integration test suite and third-party security audit.
- Testnet launch with one USDY market and one maturity date.
- Mainnet v1 deployment.

---

## License

Apache 2.0
