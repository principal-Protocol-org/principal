# Principal Protocol Dapp Technical Specification

Version: 1.0  
Date: 12 October 2026  
Audience: frontend engineers, backend engineers, Stellar/Soroban integrators, auditors, and operators  
Scope: dapp and off-chain services that integrate with the current Tranche 1 Principal Protocol contracts

> **Addendum (29 October 2026):** `contracts/market_factory` landed and was verified end to end on
> Testnet — see [docs/MARKET_FACTORY.md](MARKET_FACTORY.md). Market creation now has an on-chain
> path (three atomic calls instead of an off-chain script running ~15 manual transactions); §5, §6.4
> and §8.4 below are updated accordingly, and §21 records this as a changelog entry.

---

## 1. Executive summary

Principal Protocol is a Soroban-native fixed-income and yield-trading protocol for regulated Stellar RWAs. The smart contracts split a yield-bearing underlying asset into:

- SY: standardized yield shares backed by the underlying asset.
- PT: principal token, redeemable at maturity for principal value.
- YT: yield token, accruing the variable yield until maturity.
- LP: MarketPool liquidity positions over the PT/SY AMM.

The dapp must not treat these contracts as generic tokens. It must preserve the protocol's compliance inheritance, oracle freshness model, maturity lifecycle, one-time deployment wiring, Soroban simulation requirements, and issuer recovery path. The correct product is therefore a trading and portfolio app plus an operator console, backed by a browser transaction-building module, optional backend APIs, event indexer, oracle/keeper jobs, and a registry of supported markets.

This specification is derived from:

- Contract code under `contracts/*/src`.
- Existing protocol docs: `TECHNICAL_SPECIFICATION.md`, `API_REFERENCE.md`, `ARCHITECTURE.md`, `SECURITY.md`, `COMPLIANCE_ARCHITECTURE.md`, `AMM_DESIGN.md`, `DEPLOYMENT.md`, and `TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`.
- Claude project memory for this repository, including audit notes, deployment proof, and tranche review material.
- The current testnet deployment evidence dated 4 October 2026.

The dapp must default to the Router for normal user actions, use direct contract calls only when the Router has no equivalent, simulate every Soroban transaction before asking the wallet to sign, and surface preflight failures before the user sees a wallet prompt.

---

## 2. Current on-chain reference deployment

This is a historical reference deployment as of 12 October 2026, made on Stellar Testnet on 4 October 2026 from contract source identical to commit `6cf257a`. The local repository HEAD at the time this dapp spec was prepared is `5c706b9`; the deployment evidence states that contract source remained identical after the reviewed commit and only documentation changed.

This section is input evidence for dapp design, not the source of truth for future acceptance. Current acceptance evidence must come from `deployments/<network>/deployment-manifest.json`, `deployments/<network>/mainnet-deployment-manifest.json`, and the tranche acceptance documents.

Network:

| Field | Value |
|---|---|
| Network | Stellar Testnet |
| Passphrase | `Test SDF Network ; September 2015` |
| RPC | `https://soroban-testnet.stellar.org` |
| Underlying | STA classic asset, used as a regulated testnet stand-in for USDY |
| Underlying SAC | `CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L` |

Final market contracts:

| Component | Contract ID |
|---|---|
| OracleAdapter | `CAW53CGHBDS3DS6JJ7NOWSTCJCKNN663FEEN3WOYWJKMDOFAALZIXCRI` |
| Permissioning | `CDX64AORDY5FMDPQH633WIMNH5NBABW34UIXEDIS5E3IFWU7QFQ6YMCF` |
| RiskControl | `CCQKY4S4TC5G3LZIT37I5IBSSDVEZIIVPZSMCJVSOTASGKC6XKTZKLYR` |
| SYWrapper | `CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT` |
| PTToken | `CCOC5N37SVJ7OVG5ZM2U4SO4BOCAXWEOI7235U6333X2H7Q3WYM3QMRD` |
| YTToken | `CAULNVSPNAQWKVV2YKIPQC2NUGPVMJ7SS2PPVKFCKCLUN2AE5QLFELHD` |
| MarketConfig | `CBKWPYFGNISVQSDLSQCDJIOMXODZTJ22UV7J7HHZRCBYEWM6AQXMEQOI` |
| PrincipalManager | `CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27` |
| MarketPool | `CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR` |
| RecoveryEscrow | `CD23JCOPNIT26KAEVQI3EKEAUZ7RS2M5IDW7MTDZ6B7O3OAGOWU5ANBG` |
| Router | `CARTAP66PCU6GJJ4MGWATZM54YIT4RF7WNTPCA2KHMQD6W23NNU7KC2U` |

The run exercised 159 successful transactions covering deployment, initialization, deposits, mints, AMM trades, Router flash-mint YT, mid-life yield claim, compliance recovery, settlement, redemption, LP exit, fee claims, and final recovery settlement.

Product lessons from the deployment that must become dapp preflight checks:

- Recovery escrow wiring is one-time. A market cannot casually replace `SYWrapper`, PT, YT, or pool recovery authority after first wiring.
- Maturity is fixed on PT, YT, MarketConfig, and PrincipalManager. Short maturities can expire during setup.
- Fee recipients must pass underlying compliance and Permissioning before fee claims can succeed.
- A stale oracle blocks mint, recombine, swaps, first settlement, and unfrozen YT index updates.
- The oracle rejects future timestamps and decreasing values.
- PT and YT redemption amounts may be unequal by design.
- Testnet STA had `auth_clawback_enabled: false`, so the final native clawback step was not possible in that deployment even though recovery settlement succeeded.

---

## 3. Dapp scope

The dapp has two product surfaces:

1. User app:
   - Connect Stellar wallet.
   - Discover markets by underlying and maturity.
   - View fixed yield, variable yield, pool depth, oracle freshness, maturity state, and compliance eligibility.
   - Wrap underlying into SY and mint PT/YT.
   - Trade PT and YT through the PT/SY pool.
   - Provide and remove liquidity.
   - Recombine PT+YT before maturity.
   - Claim YT yield.
   - Redeem PT/YT at or after maturity.
   - View portfolio balances, accrued yield, transaction history, and recovery status.

2. Operator and issuer console:
   - View deployment wiring and topology health.
   - Manage Permissioning grants.
   - Monitor RiskControl pause and circuit-breaker state.
   - Monitor oracle freshness and submit oracle updates where configured.
   - Register markets in Router.
   - Claim protocol and creator fees.
   - Execute compliance recovery through RecoveryEscrow after the issuer has deauthorized a target.
   - Run storage TTL keepers.

Out of scope for the first dapp release:

- Contract upgrades. The current contracts are immutable.
- A custom wallet or custody service.
- On-chain price oracle for PT/YT. Pool spot price and implied rate are manipulable trading state, not external oracle data.
- SEP-8 approval-server orchestration around Soroban calls. SEP-8 holders must be persistently authorized by the issuer before using Principal.
- PT-as-collateral integrations with lending markets such as Blend. This is a strong future growth path but needs liquidation and compliance design that is not part of the Tranche 1 contracts.

### 3.1 Product positioning for the dapp

The first production-grade market should be USDY because the protocol is designed around regulated, appreciating RWA assets on Stellar. The same dapp architecture should support additional markets such as BENJI and USTBL by adding registry entries and deploying fresh contract sets; no frontend architecture should hard-code USDY-only assumptions beyond copy and featured-market ordering.

The dapp should make the three core user jobs obvious:

- Fixed yield: buy or mint PT and hold to maturity.
- Yield exposure: buy or mint YT and claim variable yield.
- Liquidity: provide PT/SY depth and monitor LP exposure.

Future product tracks should be isolated behind feature flags:

- PT collateral in Blend or other lending markets.
- Multi-maturity ladder views.
- Institutional/issuer onboarding portal for new RWA markets.
- Portfolio reporting for treasuries and compliance teams.

### 3.2 Evidence baseline

The Tranche 1 review material records:

- 328 of 328 tests passing on the reviewed commit.
- 97.67 percent production-code line coverage.
- mdBook documentation build passing.
- GitHub Actions CI passing for the reviewed commit.
- 159 successful Stellar Testnet transactions in the current 4 October 2026 deployment.

The dapp release process should preserve that standard by requiring its own frontend/backend test suite, testnet smoke evidence, and a documented transaction checklist before mainnet enablement.

### 3.3 Grant acceptance evidence matrix

The dapp must produce evidence that maps directly to SCF Tranche 2 and Tranche 3 funded criteria.

| Funded area | Required dapp feature | Acceptance evidence |
|---|---|---|
| Tranche 2 dApp public URL | Reviewer-accessible Testnet app | Public URL, commit, build hash, manifest version, screenshots, and environment label |
| Wallet connection | Freighter connect and network/passphrase validation | Screenshot/video, wallet state log, wrong-network rejection proof |
| Compliance status | Underlying trustline/authorization and Permissioning checks | Account address, Horizon/SAC read proof, Permissioning contract read proof |
| Deposit and mint | Primary `Router.wrap_and_mint` flow | Browser screenshot, unsigned/signed XDR hashes, transaction hash, post-state PT/YT/SY reads |
| Trading | PT and YT buy/sell through Router | Quote evidence, slippage settings, transaction hashes, post-trade pool reserve reads |
| Liquidity | Add/remove liquidity | Transaction hashes, LP balance reads, pool reserve reads |
| Yield and maturity | Claim yield, settle, redeem | Oracle freshness proof, transaction hashes, settled-rate/read proof, balance deltas |
| Transaction history | Event/indexer-backed history | Indexed-through ledger, raw event reference, UI screenshot |
| Invalid action blocking | Compliance, oracle, pause, maturity, slippage, and risk-limit failures | Simulation output or UI blocked-state screenshot with reason |
| Tranche 3 dashboard | No-wallet Mainnet monitoring | Public URL, metric source table, raw contract returns, ledger/timestamp, cache/stale status |
| Tranche 3 oracle monitoring | RedStone/SEP-40 source status and alerting | Source payload hash, scaled value, update tx, freshness state, alert test, blocked-operation proof |

---

## 4. System architecture

Recommended stack:

| Layer | Responsibilities |
|---|---|
| Frontend | Wallet connection, market UI, portfolio UI, admin UI, transaction review, signatures |
| Frontend transaction engine | Tranche 2 baseline builder, simulation, XDR hash display, wallet signing, submission, polling |
| Backend API | Registry, market data, optional transaction builder, optional submission relay, error decoding |
| Indexer | RPC event ingestion, normalized market/portfolio database, WebSocket fanout |
| Keeper jobs | Oracle relay, settlement trigger, storage TTL bump, health checks, optional fee claim monitor |
| Data stores | PostgreSQL for normalized state and event history; Redis for short-lived quote/cache data |
| Stellar integrations | Stellar RPC for Soroban simulation/state/events; Horizon for classic accounts, balances, trust lines, and fee stats |

The canonical Tranche 2 path is browser-side transaction construction, simulation, signing, submission, and polling. A backend transaction builder may be added later for reliability or Mainnet operations, but it is optional and must return unsigned XDR that the frontend verifies before signing.

The backend never holds user private keys. If a backend builder is introduced, user transactions are built unsigned, simulated, returned to the frontend, signed by the wallet, then submitted by either the frontend or backend and tracked by the indexer.

Admin-only automation requires separate signer infrastructure. For production, privileged signers should be controlled by multisig, hardware custody, or an HSM-backed policy service. Testnet can use local Stellar CLI identities only for development.

---

## 5. Market registry

The dapp must have an explicit registry. Never infer a market from arbitrary user-provided contract IDs.

Suggested registry record:

```json
{
  "market_id": "sta-testnet-2026-10-04-m4",
  "network": "testnet",
  "network_passphrase": "Test SDF Network ; September 2015",
  "rpc_url": "https://soroban-testnet.stellar.org",
  "horizon_url": "https://horizon-testnet.stellar.org",
  "underlying": {
    "asset_code": "STA",
    "issuer": "GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB",
    "sac": "CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L",
    "decimals": 7,
    "compliance_kind": "sac"
  },
  "contracts": {
    "oracle": "CAW53CGHBDS3DS6JJ7NOWSTCJCKNN663FEEN3WOYWJKMDOFAALZIXCRI",
    "permissioning": "CDX64AORDY5FMDPQH633WIMNH5NBABW34UIXEDIS5E3IFWU7QFQ6YMCF",
    "risk_control": "CCQKY4S4TC5G3LZIT37I5IBSSDVEZIIVPZSMCJVSOTASGKC6XKTZKLYR",
    "sy": "CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT",
    "pt": "CCOC5N37SVJ7OVG5ZM2U4SO4BOCAXWEOI7235U6333X2H7Q3WYM3QMRD",
    "yt": "CAULNVSPNAQWKVV2YKIPQC2NUGPVMJ7SS2PPVKFCKCLUN2AE5QLFELHD",
    "config": "CBKWPYFGNISVQSDLSQCDJIOMXODZTJ22UV7J7HHZRCBYEWM6AQXMEQOI",
    "manager": "CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27",
    "pool": "CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR",
    "recovery_escrow": "CD23JCOPNIT26KAEVQI3EKEAUZ7RS2M5IDW7MTDZ6B7O3OAGOWU5ANBG",
    "router": "CARTAP66PCU6GJJ4MGWATZM54YIT4RF7WNTPCA2KHMQD6W23NNU7KC2U"
  },
  "maturity": 1791056747,
  "token_decimals": 7,
  "status": "matured"
}
```

Registry validation job:

- `Router.is_registered(pool)` is true.
- `Router.market(pool)` resolves the expected manager, SY, PT, YT, and underlying.
- `MarketPool.manager_address()` equals registry manager.
- `PrincipalManager.sy_wrapper_address()`, `pt_address()`, `yt_address()`, `oracle_address()`, `permissioning_address()`, `config_address()`, and `underlying_address()` match registry.
- PT/YT `minter()` equals PrincipalManager.
- SY/PT/YT/Pool `recovery_escrow()` equals RecoveryEscrow.
- MarketConfig underlying and maturity match registry.
- Oracle is initialized and fresh enough for active market actions.
- RiskControl has SYWrapper and PrincipalManager registered as consumers.

If any validation fails, show the market as `unsafe_registry_mismatch` and disable user actions.

### 5.1 Adding a new registry entry

A new entry is produced by calling `MarketFactoryContract` (`contracts/market_factory`), not by
deploying contracts by hand. The factory is stateless and issuer-signed; see
[docs/MARKET_FACTORY.md](MARKET_FACTORY.md) for the full API and Testnet evidence. At a high level:

1. `predict_market(salt)` — pure, no auth — to show the issuer the seven addresses their signature
   is about to create, before they sign anything.
2. `create_tokens(issuer, salt, TokenWasmHashes, TokenParams)` → `SYWrapper`, `PTToken`, `YTToken`.
3. `create_engine(issuer, salt, EngineWasmHashes, tokens, EngineParams)` → `MarketConfig`,
   `PrincipalManager`, with the minter and circuit-breaker wiring done.
4. `create_pool_and_escrow(issuer, salt, PoolEscrowWasmHashes, tokens, engine, PoolEscrowParams)` →
   `MarketPool`, `RecoveryEscrow`, with the recovery-escrow wiring done.
5. The dapp (not the factory) then calls `RiskControl.add_consumer` for SY and PrincipalManager,
   `Router.register_market` for the pool, and grants compliance standing to every address that
   custodies value — the same three steps the pre-factory manual flow always required, deliberately
   left out of the factory because they're gated by a different signer and a human eligibility
   decision respectively.

Each of the three `create_*` calls is its own transaction and is internally atomic; splitting that
way was required after the original single-transaction design was tried on real Testnet and failed
on a host-level resource limit (`docs/MARKET_FACTORY.md`'s "Three phases, not one" section). Only
once step 5 completes should the registry record's `status` move from `pending` to `active`.

---

## 6. Frontend information architecture

### 6.1 Market list

Each row/card should show:

- Underlying asset and maturity.
- Status: pre-maturity, matured-unsettled, settled, expired/no-trading, paused, oracle-stale, registry-invalid.
- Fixed APY from `MarketPool.implied_rate()`.
- PT price from `MarketPool.pt_price()`.
- Oracle rate and freshness from OracleAdapter.
- Pool reserves and liquidity.
- Tokenization fee, YT fee, swap fee tier from MarketConfig.
- User eligibility badge when wallet is connected.

Disable action entry points when:

- Market is unregistered.
- Registry topology validation fails.
- Wallet network/passphrase mismatches.
- User has no trustline to the underlying classic asset.
- User is not authorized on the underlying or not Permissioning-granted.
- RiskControl or contract pause blocks the action.
- Oracle is stale for the intended action.

### 6.2 Market detail

Tabs:

- Overview: maturity timeline, rate, PT price, implied APY, pool depth, TVL, fees.
- Trade: buy/sell PT, buy/sell YT.
- Mint/Redeem: wrap-and-mint, recombine, claim yield, redeem at maturity.
- Liquidity: add/remove liquidity, single-sided SY deposit.
- Portfolio: connected account balances and pending actions.
- Activity: swaps, mints, claims, liquidity, redemptions, recovery events.
- Risk/Compliance: eligibility, oracle freshness, pause state, circuit-breaker volume.

### 6.3 Portfolio

Show per account and market:

- Underlying balance from Horizon/SAC.
- SY balance from `SYWrapper.balance_of`.
- PT balance from `PTToken.balance`.
- YT balance from `YTToken.balance`.
- Pending YT claim from `YTToken.pending_claim`.
- LP balance from `MarketPool.lp_balance`.
- Claimable matured redemption preview.
- Recovery records from `RecoveryEscrow.account_records` and `get_record`.
- Allowances for PT/YT if direct token transfer flows are later added.

### 6.4 Operator console

Separate privileged routes:

- Market creation: `predict_market` preview, then the three `create_*` calls (§5.1), then the
  finish-wiring step (RiskControl consumers, Router registration, compliance grants) that moves a
  pending registry entry to `active`.
- Oracle: latest value, timestamp, freshness, next relay status, monotonicity check.
- Permissioning: account grants and per-token grants.
- RiskControl: pause/unpause, pausers, consumers, protocol limit, per-asset limit, window, current volumes.
- MarketConfig: fee rates, protocol share, treasury, creator payee for SEP-57.
- Recovery: deauthorized account check, full-position preview, seize requests, finalization.
- Deployment health: topology, one-time wiring, Router registration, contract standing on underlying and Permissioning.
- Keepers: TTL bump status, settlement readiness, fee claim opportunities.

Operator-console access control:

- Public users can read only public protocol state.
- Operator read-only role can view health, indexed history, and alert status.
- Protocol admin role can build Permissioning, RiskControl, fee, Router, and pause transactions.
- Issuer authority signer role is required for RecoveryEscrow seizure/finalization workflows.
- Oracle operator role can build or submit oracle update transactions according to the configured signer policy.
- Mainnet admin actions should support external signing or multisig handoff rather than requiring a local browser hot key.
- Every privileged action must write an audit record with role, signer address, action, unsigned XDR hash, signed XDR hash when available, transaction hash, timestamp, and result.

---

## 7. User flows and contract calls

All ordinary user flows should use Router where possible. The Router acts as the user and holds no funds, so downstream compliance checks still evaluate the real user.

### 7.1 Wrap and mint PT/YT

Contract path:

`Router.wrap_and_mint(from, pool, amount, min_pt_out, deadline)`

Nested path:

1. `SYWrapper.deposit(from, amount, min_shares_out=0)`.
2. `PrincipalManager.mint(from, sy_shares)`.
3. PT and YT are minted to `from`.

Preflight:

- User has enough underlying and XLM.
- User trustline exists and is authorized.
- User passes Permissioning account-level gate.
- User passes per-asset gates for PT and YT.
- Oracle fresh.
- Now is before maturity.
- SYWrapper and PrincipalManager are not paused.
- RiskControl is not paused and has enough remaining volume.
- Amount is above zero and produces nonzero notional after tokenization fee.

UI output:

- PT minted.
- YT minted.
- Fee shares.
- Effective notional and tokenization fee.

### 7.2 Buy PT with SY

Contract path:

`Router.swap_sy_for_pt(from, pool, sy_in, min_pt_out, deadline)`

Quote source:

`MarketPool.quote_sy_for_pt(sy_in)`

Preflight:

- User has SY balance.
- Pool is seeded.
- Market not matured.
- Oracle fresh.
- Pool not paused.
- User compliant.
- Slippage minimum set from quote and user tolerance.

### 7.3 Sell PT for SY

Contract path:

`Router.swap_pt_for_sy(from, pool, pt_in, min_sy_out, deadline)`

Quote source:

`MarketPool.quote_pt_for_sy(pt_in)`

Preflight:

- User has PT balance.
- Same market, oracle, pause, liquidity, and compliance checks as buy PT.

### 7.4 Buy YT with SY

Contract path:

`Router.swap_sy_for_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)`

Economic path:

1. Mint PT + YT from user SY.
2. Sell the minted PT into the pool.
3. User keeps YT and receives any SY back.

Preflight:

- Same as mint plus swap.
- Use a simulation-backed quote because this path composes mint fees, oracle rate, and pool output.
- `min_yt_out` protects YT minted.
- `max_net_cost` protects the PT sale leg.

UI output:

- YT received.
- SY spent net.
- SY returned.
- Implied YT price.

### 7.5 Sell YT for SY

Contract path:

`Router.swap_yt_for_sy(from, pool, yt_in, min_sy_out, deadline)`

Nested path:

`MarketPool.swap_yt_for_sy` uses the pool's PT reserve to recombine against the user's YT and pays the remaining SY after curve price and fee.

Preflight:

- User has YT balance.
- Pool has enough PT reserve.
- Oracle fresh and market live.
- User compliant.

### 7.6 Add liquidity

Two-sided:

`Router.add_liquidity(from, pool, pt_desired, sy_desired, min_lp_out, deadline)`

Single-sided SY:

`Router.add_liquidity_single_sy(from, pool, sy_in, min_lp_out, deadline)`

Preflight:

- Market before maturity.
- Pool not paused.
- User has balances.
- User compliant.
- For first liquidity, initial ratio must satisfy pool rule: SY value must not exceed PT value.
- `min_lp_out` from simulation and slippage tolerance.

### 7.7 Remove liquidity

Contract path:

`Router.remove_liquidity(from, pool, lp, min_pt_out, min_sy_out, deadline)`

Important:

- Liquidity removal is allowed after maturity and does not need an oracle.
- Pool pause blocks LP exit while live but not after maturity.

Preflight:

- User has LP.
- Recipient compliant.
- Slippage limits from pool state.

### 7.8 Recombine before maturity

Contract path:

`Router.recombine(from, pool, amount, min_sy_out, deadline)`

Effect:

Burns equal PT and YT and returns SY at the current oracle rate. Any YT yield already accrued remains claimable through `claim_yield`.

Preflight:

- User has at least `amount` PT and YT.
- Before maturity.
- Oracle fresh.
- User compliant.
- PrincipalManager not paused.

### 7.9 Claim YT yield

Contract path:

`Router.claim_yield(from, pool)`

Direct underlying payment is handled by PrincipalManager. Do not call `YTToken.claim_yield` from the dapp. It is minter-gated by design and exists only for PrincipalManager.

Preflight:

- User has YT or pending claim.
- Oracle fresh unless YT index is already frozen/settled.
- User compliant.
- PrincipalManager not paused.

### 7.10 Redeem at maturity

Contract path:

`Router.redeem_at_maturity(from, pool, pt_amount, yt_amount)`

Effect:

Redeems any mix of PT and YT after maturity. PT and YT amounts do not have to be equal.

Preflight:

- Ledger timestamp is at or after maturity.
- At least one amount is nonzero.
- User has balances.
- If unsettled, oracle fresh for implicit settlement.
- If already settled, stale oracle must not block redemption.
- User compliant.

### 7.11 Settle market

Contract path:

`PrincipalManager.settle_all()`

This is permissionless and should be exposed as a keeper action, not a primary user button except when a connected user is attempting redemption and the market is still unsettled.

Preflight:

- Now at or after maturity.
- If YT not frozen, oracle fresh.

---

## 8. Application API and transaction-builder specification

Tranche 2 baseline: browser-side transaction construction and simulation. The frontend may call read APIs and indexer APIs, but it does not need a backend builder to satisfy Testnet dApp acceptance.

Optional backend builder: if implemented, it must follow the same lifecycle and safety checks as the browser builder. It must never hold user private keys, and the frontend must reject any signed XDR whose hash differs from the reviewed unsigned XDR.

### 8.1 Public market endpoints

```text
GET /api/v1/markets
GET /api/v1/markets/:market_id
GET /api/v1/markets/:market_id/state
GET /api/v1/markets/:market_id/quotes/sy-for-pt?amount=
GET /api/v1/markets/:market_id/quotes/pt-for-sy?amount=
GET /api/v1/markets/:market_id/quotes/sy-for-yt?amount=
GET /api/v1/markets/:market_id/quotes/yt-for-sy?amount=
GET /api/v1/markets/:market_id/depth?side=&steps=
GET /api/v1/markets/:market_id/activity
```

Quote endpoints must include:

- Raw input and output.
- Human decimal values.
- Fee amount.
- Price impact.
- Oracle rate used.
- Ledger sequence and timestamp.
- Staleness/valid-until hint.
- Whether simulation is recommended before display. For composed flows, the answer should always be yes.

### 8.2 Portfolio endpoints

```text
GET /api/v1/accounts/:address/eligibility
GET /api/v1/accounts/:address/portfolio
GET /api/v1/accounts/:address/markets/:market_id
GET /api/v1/accounts/:address/activity
GET /api/v1/accounts/:address/recovery-records
```

Eligibility response:

```json
{
  "address": "G...",
  "market_id": "sta-testnet-2026-10-04-m4",
  "underlying_authorized": true,
  "permissioning_account": true,
  "permissioning_pt": true,
  "permissioning_yt": true,
  "has_underlying_trustline": true,
  "can_wrap": true,
  "can_hold_sy": true,
  "can_hold_pt": true,
  "can_hold_yt": true,
  "reasons": []
}
```

### 8.3 Transaction builder interface

All transaction builders, whether browser modules or optional backend endpoints, return unsigned XDR and a human-readable summary.

```text
POST /api/v1/tx/wrap-and-mint
POST /api/v1/tx/unwrap
POST /api/v1/tx/swap-sy-for-pt
POST /api/v1/tx/swap-pt-for-sy
POST /api/v1/tx/swap-sy-for-yt
POST /api/v1/tx/swap-yt-for-sy
POST /api/v1/tx/add-liquidity
POST /api/v1/tx/add-liquidity-single-sy
POST /api/v1/tx/remove-liquidity
POST /api/v1/tx/recombine
POST /api/v1/tx/redeem-at-maturity
POST /api/v1/tx/claim-yield
POST /api/v1/tx/settle-all
POST /api/v1/tx/submit
GET  /api/v1/tx/:hash
```

Builder request common fields:

```json
{
  "source": "G...",
  "market_id": "sta-testnet-2026-10-04-m4",
  "amount": "10000000",
  "slippage_bps": 50,
  "deadline_seconds": 300,
  "client_request_id": "uuid"
}
```

Builder response:

```json
{
  "network_passphrase": "Test SDF Network ; September 2015",
  "unsigned_xdr": "...",
  "unsigned_xdr_hash": "sha256:...",
  "source_account": "G...",
  "source_sequence": "123456789",
  "latest_ledger": 123456,
  "deadline": 1791056000,
  "call_tree": [
    "Router.swap_sy_for_pt",
    "MarketPool.swap_sy_for_pt"
  ],
  "expected_signers": ["G..."],
  "soroban_auth_entries": [],
  "footprint_restore_required": false,
  "simulation": {
    "ok": true,
    "latest_ledger": 123456,
    "min_resource_fee": "123456",
    "transaction_data_hash": "sha256:...",
    "return_value": {},
    "diagnostics": []
  },
  "summary": {
    "action": "swap_sy_for_pt",
    "market_id": "sta-testnet-2026-10-04-m4",
    "inputs": [],
    "expected_outputs": [],
    "max_slippage_bps": 50,
    "deadline": 1791056000
  },
  "risk": {
    "oracle_fresh": true,
    "market_live": true,
    "user_eligible": true,
    "pool_registered": true
  },
  "post_sign_verification": {
    "required": true,
    "signed_xdr_hash_must_match_reviewed_payload": true
  }
}
```

Submission endpoint:

- Accepts signed XDR.
- Verifies passphrase and source account match the pending request where possible.
- Submits through Stellar RPC.
- Polls `getTransaction` until success, failure, or timeout.
- Returns decoded contract return values and normalized events.

Idempotency:

- `client_request_id` prevents duplicate build records.
- `signed_xdr` hash prevents duplicate submission.
- If a transaction status is unknown, keep polling by hash before rebuilding.

### 8.4 Admin endpoints

```text
POST /api/v1/admin/markets/predict
POST /api/v1/admin/markets/create-tokens
POST /api/v1/admin/markets/create-engine
POST /api/v1/admin/markets/create-pool-and-escrow
POST /api/v1/admin/markets/finish-wiring
POST /api/v1/admin/permissioning/grant-account
POST /api/v1/admin/permissioning/revoke-account
POST /api/v1/admin/permissioning/grant-asset
POST /api/v1/admin/permissioning/revoke-asset
POST /api/v1/admin/risk/pause
POST /api/v1/admin/risk/unpause
POST /api/v1/admin/risk/set-limits
POST /api/v1/admin/oracle/set-reference-value
POST /api/v1/admin/recovery/seize-all-positions
POST /api/v1/admin/recovery/finalize-record
POST /api/v1/admin/fees/claim-protocol
POST /api/v1/admin/fees/claim-creator
```

The four `markets/*` creation endpoints build unsigned XDR for the corresponding
`MarketFactoryContract` call (§5.1); `finish-wiring` builds the `RiskControl.add_consumer` ×2,
`Router.register_market`, and compliance-grant transactions that follow. `predict` is read-only and
needs no signer.

Admin endpoints should normally build transactions for an admin wallet, not submit with a server-held key, except for an explicitly configured oracle relay or keeper.

Recovery endpoints must require issuer authority signer context. Protocol admin authorization alone is insufficient unless the same signer is also the underlying issuer authority accepted by the contracts.

---

## 9. Transaction lifecycle

Every user transaction follows this lifecycle. "Builder" means the browser-side builder for Tranche 2, or the optional backend builder if the project explicitly enables one.

1. Frontend creates an intent.
2. Builder validates market registry and user eligibility.
3. Builder loads account sequence and balances.
4. Builder builds contract invocation XDR.
5. Builder simulates via Stellar RPC.
6. Builder applies simulation result: footprint, auth entries, resource fee, and result preview.
7. Builder returns unsigned XDR, hash, call tree, and summary.
8. Frontend asks wallet to sign with exact network passphrase.
9. Frontend verifies signed XDR hash/call content still matches the reviewed transaction.
10. Frontend or backend submits via RPC.
11. Submitter polls transaction status.
12. Backend decodes return value and events.
13. Indexer updates normalized state.
14. Frontend updates optimistically only after submission, and finalizes after confirmed success.

Critical implementation details:

- Never ask the wallet to sign before simulation.
- Never reuse an unsigned XDR after sequence number, deadline, resource fee, or ledger footprint has gone stale.
- Treat simulation as advisory. State can change before inclusion.
- Rebuild and re-sign on sequence errors, expired deadlines, stale footprint, or archived entries.
- If simulation reports archived entries, present a restore-footprint flow, submit the restore transaction first, then rebuild and re-simulate the original action.
- If restoration fails, stop and show the archived-entry reason instead of asking the wallet to sign the original action.
- Deadline should default to 300 seconds for user trades and be configurable.
- Use integer raw amounts end to end; format decimals only in UI.

Wallet state machine:

| State | Required behavior |
|---|---|
| Wallet not installed | Show install/connect guidance and keep actions disabled |
| Wallet locked or unavailable | Ask user to unlock/retry; do not build signable XDR |
| Connected | Show account, network, and passphrase |
| Wrong network | Disable signing and show expected network/passphrase |
| Account changed | Clear pending unsigned XDR and rebuild with the new account |
| Network changed | Clear session-sensitive state and revalidate registry/network |
| User rejects signature | Mark action cancelled; do not auto-resubmit |
| Wallet returns changed XDR | Reject submission unless post-sign verification proves semantic equivalence |
| Pending transaction timeout | Continue hash polling and show unknown/pending status before allowing rebuild |
| Disconnected | Clear account-specific balances, eligibility, and pending signing state |

---

## 10. Indexer design

The indexer consumes Soroban events through Stellar RPC `getEvents`, stores raw event XDR, decodes normalized records, and pushes market/account updates through WebSocket.

Minimum tables:

- `markets`
- `market_contracts`
- `market_state_snapshots`
- `oracle_updates`
- `permissioning_events`
- `risk_events`
- `user_balances`
- `pt_yt_transfers`
- `sy_transfers`
- `lp_transfers`
- `mints`
- `recombines`
- `redemptions`
- `yield_claims`
- `swaps`
- `liquidity_events`
- `fee_claims`
- `recovery_records`
- `transactions`
- `raw_events`

Event topics to decode:

| Contract | Topics |
|---|---|
| OracleAdapter | `ref_set`, `adm_xfer` |
| Permissioning | `acc_grant`, `acc_rev`, `acc_batch`, `ast_grant`, `ast_rev`, `adm_xfer` |
| RiskControl | `paused`, `add_psr`, `rm_psr`, `add_cons`, `rm_cons`, `cb_limit`, `ast_limit`, `cb_window`, `adm_xfer` |
| MarketConfig | `mkt_new`, `fees_set`, `payee_set`, `share_set`, `treas_set`, `padm_xfer` |
| SYWrapper | `deposit`, `withdraw`, `sy_xfer`, `seize`, `paused`, `cap_set`, `rc_set`, `esc_set`, `adm_xfer` |
| PTToken/YTToken | `mint`, `burn`, `transfer`, `approve`, `seize`, `min_set`, `esc_set`; YT also `idx_up`, `frozen`, `claim` |
| PrincipalManager | `mint`, `recombine`, `redeem`, `yt_claim`, `settled`, `fee_claim`, `paused`, `rc_set`, `adm_xfer` |
| MarketPool | `buy_pt`, `sell_pt`, `sell_yt`, `add_liq`, `zap_liq`, `rem_liq`, `lp_xfer`, `lp_seize`, `fee_claim`, `paused`, `esc_set`, `adm_xfer` |
| Router | `mkt_reg`, `adm_xfer` |
| RecoveryEscrow | `recovery`, `finalize` |

Important audit note:

PT/YT events intentionally deviate from canonical SEP-41 event shape in the current contracts. The indexer must decode the contract-specific topics above and should not assume wallet/indexer support for standard SEP-41 transfer events until those events are migrated.

Reconciliation:

- Periodically read on-chain balances/views and compare against indexed state.
- Reconcile PT, YT, SY, and LP balances after every transaction involving the connected user.
- Reconcile pool reserves with `MarketPool.reserves()` and `pool_state()`.
- Store the ledger sequence for every state snapshot.

Operational requirements:

- Store ingestion cursor by network, contract, start ledger, latest processed ledger, and RPC cursor token where provided.
- Backfill from the deployment ledger recorded in the manifest.
- Use idempotent event keys: `network + transaction_hash + operation_index + event_index`.
- Handle RPC pagination and retry without duplicating normalized rows.
- Expose indexed-through ledger and lag seconds to the frontend.
- Mark history and compliance feeds stale when indexer lag exceeds the configured threshold.
- Provide a documented re-index procedure from a ledger checkpoint.
- WebSocket clients must reconnect with last-seen ledger and fetch missed updates over HTTP.

---

## 11. Data and math rules

Constants:

| Constant | Value |
|---|---:|
| `SCALE` | 10,000,000 |
| `FEE_SCALE` | 1,000,000,000,000 |
| `INDEX_SCALE` | 1,000,000,000,000 |
| `WAD` | 1,000,000,000,000,000,000 |
| Oracle max staleness | 3,600 seconds |
| Default RiskControl window | 17,280 ledgers |
| Tokenization fee cap | 100 bps |
| YT fee cap | 5,000 bps |
| Swap fee tier cap | 500 bps |
| Minimum liquidity | 1,000 raw LP |

UI formatting:

- Use raw `i128` strings in API payloads.
- Use `decimals = 7` for the current PT/YT/SY style amounts unless registry says otherwise.
- Never use floating point for transaction values.
- Use decimal libraries for display and analytics.
- Show both raw and formatted amounts in admin/audit views.

Core formulas for display:

```text
PT redemption underlying = floor(pt_amount * SCALE / settled_rate)
YT pending yield = balance * (last_claimed_index - current_index) / INDEX_SCALE
Trading fee = swap_fee_tier * days_to_maturity / 365
Protocol fee share = fee * protocol_share_bps / 10000
Creator fee share = fee - protocol_fee
```

The dapp may display these formulas as previews, but contract simulation is authoritative for transaction output.

---

## 12. Compliance and permissioning requirements

Principal's compliance model is inherited from the underlying asset and optionally narrowed by Permissioning.

For a SAC/classic/SEP-8 asset:

- Holder eligibility is `SAC.authorized(account)`.
- Issuer authority is `SAC.admin()`.

For a SEP-57 RWA token:

- Holder eligibility is not frozen and identity-verified.
- Issuer authority is proven by the operator capability probe.

Dapp requirements:

- Show the user's underlying authorization and Permissioning status before every action.
- Block actions that would fail compliance.
- Check both account-level and per-asset Permissioning for PT/YT.
- Show that SEP-8 sandwich authorization is not enough for Soroban calls; users must be persistently authorized at rest.
- For admin recovery, require the target to already be deauthorized/frozen before building a RecoveryEscrow call.
- Protocol contract addresses that hold value must themselves have compliance standing; include this in deployment health.

Recovery flow:

1. Issuer deauthorizes/freezes target on underlying.
2. Admin console verifies target is no longer authorized.
3. Console reads full SY/PT/YT/LP balances.
4. Console builds `RecoveryEscrow.seize_all_positions(caller, accounts)`.
5. After maturity, console builds `RecoveryEscrow.finalize_record(caller, id)` for records holding PT/YT.
6. Issuer completes underlying-native clawback/forced-transfer where available.

Do not expose RecoveryEscrow calls to ordinary users.

---

## 13. Oracle and keeper requirements

OracleAdapter stores a monotonic reference value: USDC per underlying scaled by `SCALE`.

Tranche 2 changes the OracleAdapter ABI by adding source-aware updates. The dapp must version its generated contract clients and registry entries:

- `oracle_abi_version = 1` means legacy `set_reference_value(caller, value, timestamp)`.
- `oracle_abi_version = 2` means source-aware `set_reference_value(caller, source, value, timestamp)`.
- The registry/manifest must declare the ABI version per market.
- Generated clients and error decoders must be regenerated and checked into the app package for the deployed ABI.
- The dapp must refuse to build oracle admin transactions if the ABI version in the registry does not match the generated client.
- Acceptance evidence must include one successful source-aware oracle update and one decoded rejection using the deployed ABI.

Oracle relay responsibilities:

- Fetch signed/reference source for the underlying.
- For USDY Mainnet, fetch the configured RedStone USDY/USD SEP-40 source named in the manifest.
- Verify source authenticity using the exact feed contract/API, signer set, message format, and timestamp semantics recorded in the manifest.
- Enforce replay protection by rejecting source payloads older than the last accepted source timestamp or already submitted payload hash.
- Record feed decimals and convert deterministically to `SCALE = 10_000_000` using integer arithmetic and documented rounding.
- Validate value is positive.
- Validate value is greater than or equal to current on-chain value.
- Use a timestamp at or before latest ledger close time. Do not use a local clock that can be ahead of the ledger.
- Submit `set_reference_value(caller, source, value, timestamp)` for ABI version 2 deployments, or the legacy three-argument function only when the manifest explicitly declares ABI version 1.
- Alert if the feed approaches 3,600 seconds stale.

Keeper jobs:

| Job | Cadence | Action |
|---|---:|---|
| Oracle freshness monitor | every ledger or minute | alert and/or submit heartbeat |
| Settlement keeper | after maturity | call `PrincipalManager.settle_all()` once fresh oracle exists |
| TTL bump keeper | daily or weekly | call `bump()` on long-lived contracts |
| Permissioning TTL monitor | daily | identify grants approaching expiry |
| Fee monitor | daily | detect claimable protocol/creator fees |
| Registry validator | every deploy/change | topology checks listed in section 5 |

Maturity rule:

- Before settlement, a fresh oracle is needed to freeze the YT index and settlement rate.
- After settlement, redemption must not depend on oracle freshness.

---

## 14. Risk controls and pause handling

RiskControl is wired into SYWrapper deposits and PrincipalManager mints. A wrap-and-mint flow reports volume at both entry points.

Production policy: treat the current double-reporting behavior as intentional unless contracts are explicitly changed and re-audited. Production circuit-breaker limits must be calibrated so the primary `Router.wrap_and_mint` path has usable capacity after both `SYWrapper.deposit` and `PrincipalManager.mint` report volume. The dashboard must label whether displayed remaining capacity is raw RiskControl capacity or effective wrap-and-mint capacity after double-reporting.

Dapp behavior:

- Read `RiskControl.is_paused()`, `get_cb_limit()`, `get_cb_volume()`, `get_asset_limit(asset)`, `get_asset_volume(asset)`, and `get_window_ledgers()`.
- Estimate whether the user's intended deposit/mint will exceed limits.
- Explain that final enforcement is on-chain and simulation is authoritative.
- Disable deposit/mint when paused.
- Do not disable RecoveryEscrow because recovery is intentionally available while ordinary operations are paused.

Contract-level pauses:

- SYWrapper pause affects deposit, withdraw, transfer, except configured recovery paths.
- PrincipalManager pause affects mint, recombine, redeem, claim yield.
- MarketPool pause affects live trading and LP operations, but removal after maturity is allowed.
- RiskControl pause blocks `check_deposit`, therefore deposits and mints.
- PrincipalManager must expose a live `is_paused()` view for the dashboard and dapp gating. If the deployed ABI lacks this view, Tranche 2/3 implementation must add it before Mainnet dashboard acceptance.

---

## 15. Error decoding

The backend must map contract errors to user-safe messages and developer diagnostics. Because Router composes calls, a Router transaction may surface a downstream error from SYWrapper, PrincipalManager, MarketPool, PTToken, YTToken, RiskControl, or Permissioning.

Minimum user-facing categories:

- Not eligible: underlying authorization or Permissioning failure.
- Insufficient balance.
- Insufficient allowance, if direct token flows are used.
- Slippage exceeded.
- Deadline expired.
- Oracle stale.
- Market expired or not mature.
- Market not registered.
- Pool not seeded or insufficient liquidity.
- Paused.
- Circuit breaker tripped.
- Registry/topology mismatch.
- Recovery target still authorized.
- Nothing to claim/finalize.
- Wallet not installed, locked, disconnected, wrong network, account changed, or signature rejected.
- Insufficient XLM reserve or fee balance.
- Trustline missing.
- Underlying issuer authorization missing.
- Permissioning grant expired.
- Stale quote or stale simulation.
- RPC outage or RPC rate limit.
- Archived ledger entry requires restore.
- Pending transaction timed out or final status unknown.

Diagnostics should include:

- Source contract.
- Raw error code.
- Simulated transaction hash or request ID.
- Ledger sequence.
- Contract call path where known.

---

## 16. Security and audit requirements for the dapp

### 16.1 Trust boundaries

- User wallet signs all user transactions.
- Browser builder is the Tranche 2 baseline for user transactions.
- Optional backend builder/submission services cannot change signed XDR and must be verified by XDR hash/call summary before signing.
- Registry controls which contracts the UI will call.
- Indexer data is a cache, not source of truth.
- Oracle relay/admin signers are privileged.
- Recovery UI is privileged and issuer-controlled.

### 16.2 Required controls

- Strict market allow-list.
- Topology validation before enabling a market.
- Exact network passphrase display before wallet signing.
- Simulation before signing.
- Human-readable transaction summary.
- Slippage and deadline on every user flow that supports them.
- Integer-only amount handling.
- XDR tamper checks between build and sign where wallet APIs allow.
- Rate limiting and idempotency on submission.
- Secondary RPC fallback.
- Event reconciliation against contract views.
- Admin action audit log.
- Alerts for stale oracle, paused markets, risk limit exhaustion, keeper failures, and registry mismatch.

### 16.3 Known protocol assumptions the dapp must respect

- Oracle value is monotonic. Negative yield/loss is not supported by the current PT/YT economics.
- PT/YT direct holder burn is not exposed; burns are minter-only.
- `YTToken.claim_yield` is not a public payment path. Use PrincipalManager/Router.
- MarketPool spot price is not an oracle.
- RecoveryEscrow is legal/compliance tooling, not a liquidation module.
- Permissioning entries expire and must be refreshed.
- Instance storage TTL must be bumped for long-dated markets.
- Fee changes are not timelocked at contract level; the dapp should show fee-change history and current fees prominently.

---

## 17. Testing plan

### 17.1 Unit tests

Backend:

- Registry validation.
- Amount parsing/formatting.
- Error decoding.
- Quote response shaping.
- Deadline/slippage generation.
- Idempotent submission records.
- Optional backend-builder XDR hash and call-tree verification.
- Indexer cursor, pagination, duplicate-event, and re-index logic.

Frontend:

- Wallet network mismatch.
- Wallet not installed, locked, rejected signature, account changed, and network changed states.
- Eligibility states.
- Disabled action reasons.
- Transaction summary rendering.
- Matured/settled/unsettled market states.
- Restore-footprint prompt and retry flow.
- Dashboard no-wallet rendering.
- RPC outage/stale-indexer/error states.
- Responsive layouts, keyboard navigation, and decimal input validation.

### 17.2 Integration tests against local/simulated Soroban

- Wrap and mint.
- Buy/sell PT.
- Buy/sell YT.
- Add/remove liquidity.
- Claim yield.
- Recombine.
- Settle and redeem.
- Stale oracle blocks active flows.
- Permissioning denial blocks user before wallet prompt.
- RiskControl over-limit simulation returns structured error.
- Recovery flow for a deauthorized account.

### 17.3 Testnet smoke tests

Run on a short-dated market:

- Deploy and registry validation.
- Grant user and protocol contract standing.
- User wrap-and-mint.
- Pool seed and swaps.
- Router flash-mint YT.
- Oracle update and claim yield.
- Maturity settlement.
- Redeem PT/YT.
- Fee claim.
- Compliance recovery and finalization.

Use the 4 October 2026 deployment evidence as the baseline checklist, with one improvement: use a maturity long enough for manual setup and smoke testing.

Browser E2E must cover each funded flow through the public dapp or a production-equivalent preview build: connect, compliance display, wrap-and-mint, PT trade, YT trade, liquidity add/remove, claim, settle/redeem, transaction history, and at least three blocked invalid actions.

---

## 18. Release checklist

Before enabling a market in production:

- Contracts built from audited commit.
- WASM hashes recorded.
- Frontend build commit, artifact hash, and manifest version recorded.
- Mainnet and Testnet environments separated by distinct manifests, network passphrases, and deployment URLs.
- Runtime config cannot silently switch networks without visible environment labeling.
- Registry topology validation passes.
- Router registration verified.
- PT/YT minters set exactly once.
- RecoveryEscrow wired exactly once on SY/PT/YT/Pool.
- Protocol contracts holding value are authorized on underlying and Permissioning-granted.
- Treasury and creator fee recipients are eligible.
- Oracle relay live and monitored.
- RiskControl consumers set for SYWrapper and PrincipalManager.
- Permissioning grant refresh process defined.
- TTL bump keeper active.
- Indexer caught up from deployment ledger.
- Frontend points to the correct network passphrase.
- Admin signers are production-grade, not local test keys.
- Emergency runbook reviewed.
- CSP/security headers configured.
- Source-map publication policy approved.
- Public healthcheck URL available.
- Cache invalidation and rollback procedure documented.

---

## 19. Open implementation decisions

These are product/engineering decisions outside the current contracts:

- Which wallet APIs to support first: Freighter only, or Freighter plus SEP-7/mobile deep links.
- Whether backend submits signed user XDR or frontend submits directly and backend only tracks.
- Whether market data API computes depth from contract math locally or from repeated on-chain quote simulations.
- Whether admin actions are built for external multisig signing or submitted by a controlled operator service.
- Oracle source for production USDY and signer governance.
- Hosting and failover providers for Stellar RPC/Horizon.
- Whether to expose direct PT/YT transfers in the UI. If exposed, enforce compliance preflight for both sender and recipient.

---

## 20. Minimal build order for the dapp team

1. Registry and topology validator.
2. Read-only market state and portfolio.
3. Transaction builder for wrap-and-mint, claim-yield, redeem.
4. Trading quotes and PT/YT swaps.
5. Liquidity flows.
6. Indexer and activity feeds.
7. Oracle/settlement/TTL keepers.
8. Admin console for Permissioning, RiskControl, fees, and recovery.
9. Admin console for market creation (`MarketFactoryContract`, §5.1, §6.4, §8.4).
10. Production hardening: signer governance, monitoring, reconciliation, and incident runbooks.

The dapp should launch read-only first against the documented testnet deployment, then enable signing flow by flow after each one has a simulation-backed preflight and indexed reconciliation.

---

## 21. Multi-agent audit resolution

This specification was re-audited from three angles: full-stack dapp architecture, Stellar/Soroban contract integration, and SCF acceptance evidence. The following issues were resolved in this version:

| Finding | Resolution |
|---|---|
| Transaction-builder conflict between dapp and tranche specs | Browser-side build/simulate/sign/submit is now the Tranche 2 baseline; backend builder is optional and constrained by XDR hash verification |
| Wallet flow under-specified | Added wallet state machine covering missing wallet, locked wallet, wrong network, account/network changes, rejection, disconnect, and pending timeout |
| Archived Soroban entries not handled | Added restore-footprint flow and failure behavior |
| Builder response missing audit fields | Added unsigned XDR hash, source sequence, latest ledger, deadline, call tree, auth entries, simulation hash, and post-sign verification |
| Indexer operational model incomplete | Added cursor/checkpoint, idempotent event keys, pagination, lag, re-index, and WebSocket reconnect requirements |
| Operator console lacked RBAC | Added public/read-only/protocol-admin/issuer-authority/oracle-operator role boundaries and audit logging |
| Oracle ABI/versioning ambiguous | Added `oracle_abi_version` handling and source-aware update requirements |
| RedStone/SEP-40 trust model vague | Added feed/API, signer/auth, timestamp, replay, decimals, and integer conversion requirements |
| RiskControl double-reporting unclear | Added production policy and dashboard labeling for effective wrap-and-mint capacity |
| PrincipalManager pause view missing | Made `PrincipalManager.is_paused()` mandatory for Mainnet dashboard acceptance |
| Acceptance evidence not mapped in dapp spec | Added grant acceptance evidence matrix tied to funded Tranche 2/3 flows |

### 21.1 Addendum: market creation (29 October 2026)

| Finding | Resolution |
|---|---|
| No specified path for creating a new market; `contracts/market_factory` did not exist when this spec was first written | Added §5.1 (registry entry creation via `MarketFactoryContract`), a Market creation route in §6.4, the `markets/*` admin endpoints in §8.4, and a build-order item in §20. The factory's own design, the real Testnet resource-limit finding that forced a three-call split, and full transaction evidence are in [docs/MARKET_FACTORY.md](MARKET_FACTORY.md), not duplicated here |
