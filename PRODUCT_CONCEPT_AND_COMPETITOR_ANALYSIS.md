# Principal Protocol — Product Concept and Competitor Positioning

SCF #44 resubmission material. Every figure below is fact-checked against the live codebase and verified external sources (Spectra's own Q2 2026 report, Stellar RWA issuer data, Blend/Pendle category history) — not restated from the original submission.

---

## 1. Product

Principal Protocol is Soroban-native yield-tokenization infrastructure for Stellar's regulated real-world assets. It splits a yield-bearing RWA — starting with Ondo's USDY — into two independently tradable instruments:

- **Principal Token (PT)** — a fixed-value claim redeemable at par at a stated maturity.
- **Yield Token (YT)** — captures all variable yield generated between issuance and maturity.

This is the same PT/YT primitive established by Pendle and adopted across DeFi, applied specifically to Stellar's own compliance-gated tokenized treasuries rather than as a generalized, permissionless yield-source abstraction. It gives Stellar users three things that don't currently exist for any regulated RWA on the network: lock in a fixed yield, sell future yield upfront for liquidity, or take a directional position on RWA yield rates.

---

## 2. Markets

- **USDY (Ondo)** — live on Stellar today, KYC/whitelist-gated, clawback-enabled at the issuer level. First and current market.
- **BENJI (Franklin Templeton, $650M+ on Stellar)** and **USTBL (Spiko, $563M on Stellar)** — expansion path; the same asset-agnostic `SYWrapper`/`PrincipalManager` pair extends to either with no new contract code, only new deployments.
- **PT as collateral (Blend, ~$80M TVL)** — an adjacent market. Blend's permissionless custom-collateral pools are Stellar's analog to Aave/Morpho/Euler, where PT-as-collateral is the single most validated growth mechanism in the PT/YT category (Pendle's PT collateral crossed $4.6B on Aave after one cap raise). Currently inaccessible to any RWA-backed PT anywhere, because permissioned assets can't be safely liquidated in an open market — see the Liquidation Adapter in §4.

---

## 3. Competitive Differentiation

SCF #44 feedback named two direct competitors. Both are independently verified below — this isn't a general market survey.

### Spectra
Confirmed via Spectra's own Q2 2026 ecosystem report (Certora-audited, June 2026): Spectra is building a cross-chain bridge that brings its *existing* EVM Principal Tokens (XRP/Flare yield, crvUSD vaults, liquid staking) to Stellar as its first non-EVM environment — not a native Soroban RWA-splitting protocol. Nothing in that report mentions RWAs, compliance, KYC, or eligibility gating.

- **Different asset universe** — general DeFi yield vs. Stellar's regulated treasuries.
- **Different trust model** — bridged settlement back to EVM logic vs. native Soroban end to end.
- **Different compliance posture** — none in Spectra's stated plans vs. two-layer compliance (SAC authorization inheritance + `Permissioning`) enforced today at mint, redemption, deposit, withdrawal, and PT/YT transfer, plus a dedicated compliance-recovery contract (§4).
- **Different timeline** — their bridge is "will soon be available"; eight of ten Principal Protocol contracts are built and tested today, including the compliance-recovery contract described in §4 ([PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md)).

### YieldBack.Cash
A sponsor-funded bond-coupon pool (SCF #38, funded) — depositors get fixed income while a sponsor pre-funds the coupon. Not a PT/YT splitting protocol: no principal/yield separation, no two-sided market, no asset-agnostic extension without a new sponsor relationship per market. Principal Protocol mints PT and YT directly from the underlying asset's own yield — no sponsor required.

### Adjacent RWA infrastructure (not direct competitors)
Centrifuge (asset financing), Maple and Goldfinch (credit marketplaces), and Ondo (the underlying asset issuer, not a competing splitter) operate in adjacent categories, not PT/YT splitting. Included for completeness only — not part of the differentiation argument, since this isn't the comparison reviewers raised.

---

## 4. Original Features

Four mechanisms exist only because the underlying collateral is regulated — none are meaningful in a permissionless-collateral protocol like Spectra's:

1. **Two-layer compliance, inherited live from the issuer** — built and tested across `SYWrapper`, `PrincipalManager`, `PTToken`, and `YTToken` today. Every check runs `underlying_SAC.authorized(account)` — a real, public, no-auth-required Stellar Asset Contract function, read live from the actual issuer — as a mandatory floor, plus `Permissioning` as an optional narrowing layer. This means Principal never maintains a separate compliance registry that could drift out of sync with the issuer's own decisions: if the issuer deauthorizes a wallet on the underlying asset itself, every Principal-derived instrument reflects that immediately.
2. **Compliance recovery via `RecoveryEscrow`** (built and tested for SY; PT/YT seizure built, post-maturity unwind pending the same PrincipalManager↔token wiring as item 3 below) — lets the issuer recover a specific deauthorized account's value after their RWA has been split into SY, PT, and YT, without penalizing other depositors in the shared pool, and without `RecoveryEscrow` ever holding an admin key of its own — it re-authenticates the issuer's real, live SAC `admin()` on every call.
3. **Compliant Liquidation Adapter** (proposed, feasibility-verified against the current codebase) — lets PT-RWA serve as real collateral in a lending market like Blend without a permissioned token ever reaching an unapproved liquidator.
4. **Asymmetric PT/YT permissioning** (built and tested) — distinct eligibility policy for the protected-principal claim vs. the speculative-yield claim, using `Permissioning`'s existing per-asset allow-list layered on top of the SAC compliance floor.

---

## 5. Response to SCF #44 Feedback

| Reviewer feedback | Response |
|---|---|
| No differentiation from Spectra/YieldBack.Cash | §3 — both verified and directly addressed |
| No traction; ask too large for a market-size pitch | Ask cut from $142,000 to **$112,000**, scoped to wiring the remaining contracts together + one live USDY testnet market — a checkable, on-chain milestone (§6) |
| Submission metadata (1 person) doesn't match described team (4–5) | Form-accuracy fix, handled directly in the SCF application — not a doc issue |
| Academic team, no execution track record | Point to what's shipped: 8/10 contracts built and unit-tested, including the compliance-recovery contract in §4 ([PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md)), documented threat model ([SECURITY.md](SECURITY.md)), deployment runbook ([DEPLOYMENT.md](DEPLOYMENT.md)) |

---

## 6. Ask

**$112,000** (down from $142,000) — wire `PrincipalManager` to actually call the now-built `SYWrapper`, `PTToken`, and `YTToken` contracts (closing the internal-balance-map gap noted in [PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md)'s Known Limitations), build the remaining `MarketPool` and `Router` contracts, complete the `RecoveryEscrow` finalize path for PT/YT, and ship one live USDY testnet market exercising the full flow end to end. The Liquidation Adapter (§4) is the remaining differentiation-proving deliverable inside that scope — `RecoveryEscrow` and asymmetric PT/YT permissioning are already built. Full third-party audit and mainnet deployment are deferred to a follow-on round, to be requested once there's real testnet usage to point to.
