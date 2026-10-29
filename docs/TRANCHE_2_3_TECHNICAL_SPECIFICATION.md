# Tranche 2 and 3 Technical Specification

Version: 1.0  
Date: 12 October 2026  
Project: Principal Protocol  
Scope: technical work required to deliver SCF #45 Tranche 2 and Tranche 3  
Audience: full-stack engineers, Soroban engineers, DevOps, reviewers, auditors, and operators

---

## 1. Purpose

This document is the build specification for the remaining funded work after Tranche 1. It translates the Tranche 2 and Tranche 3 deliverables into concrete implementation tasks, repository artifacts, contract changes, dApp features, deployment procedures, CI jobs, monitoring, and acceptance evidence.

Tranche 1 delivered the core Soroban protocol contracts: `OracleAdapter`, `Permissioning`, `RiskControl`, `MarketConfig`, `SYWrapper`, `PrincipalManager`, `PTToken`, `YTToken`, `MarketPool`, `Router`, `RecoveryEscrow`, and the shared `principal_compliance` library. Tranche 2 and Tranche 3 turn that protocol into a reviewer-usable Testnet product and then a production Mainnet deployment.

Source material:

- The SCF #45 Tranche 2 and Tranche 3 deliverable text pasted into this task.
- `TRANCHE_MILESTONE_ROADMAP.docx`.
- `PRODUCT_CONCEPT_AND_COMPETITOR_ANALYSIS.docx`.
- Existing repository docs: `TECHNICAL_SPECIFICATION.md`, `API_REFERENCE.md`, `ARCHITECTURE.md`, `SECURITY.md`, `DEPLOYMENT.md`, `COMPLIANCE_ARCHITECTURE.md`, `AMM_DESIGN.md`, and `TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`.
- Current contract code in `contracts/*/src`.
- Claude/Codex project memory and the current 4 October 2026 Testnet deployment evidence.

---

## 2. Tranche summary

| Tranche | Duration | Budget | Objective |
|---|---:|---:|---|
| Tranche 2: Testnet | Weeks 6-12 | $40,000 | Scripted Testnet deployment, oracle safety extensions, public Testnet dApp, live E2E tests, and reviewer walkthrough |
| Tranche 3: Mainnet | Weeks 13-16 | $30,000 | Production Mainnet deployment, public monitoring dashboard, production oracle integration and alerting |

Tranche 2 deliverables:

| Deliverable | Budget | Hours | Duration |
|---|---:|---:|---|
| Testnet Deployment + Oracle Safety | $16,000 | 128 | Weeks 6-7 |
| dApp on Testnet | $18,000 | 180 | Weeks 7-11 |
| End-to-End Tests + Testnet Walkthrough | $6,000 | 60 | Weeks 10-12 |

Tranche 3 deliverables:

| Deliverable | Budget | Hours | Duration |
|---|---:|---:|---|
| Mainnet Deployment + Production Configuration | $15,000 | 120 | Weeks 13-14 |
| dApp Dashboard + Protocol Monitoring | $10,000 | 100 | Weeks 13-16 |
| Mainnet Oracle Integration + Monitoring | $5,000 | 50 | Weeks 13-16 |

---

## 3. Repository deliverables

The implementation should add these top-level areas without disturbing Tranche 1 contract docs:

```text
scripts/
  deploy/
    deploy.sh
    verify-deployment.sh
    lib.sh
    env.example
    manifests/
      testnet-deployment-manifest.example.json
      mainnet-deployment-manifest.example.json
  oracle/
    submit-reference-value.ts
    redstone-usdy-source.ts
    monitor-oracle.ts
  e2e/
    run-live-testnet-flow.ts

apps/
  web/
    package.json
    src/
      app/
      components/
      lib/stellar/
      lib/protocol/
      lib/indexer/
      lib/oracle/
      lib/format/
      config/

services/
  indexer/
  oracle-relay/
  monitor/

deployments/
  testnet/
    deployment-manifest.json
    smoke-test-report.md
  mainnet/
    mainnet-deployment-manifest.json
    smoke-test-report.md
    reproducible-build.md

docs/
  TRANCHE_2_TESTNET_WALKTHROUGH.md
  TRANCHE_2_TESTNET_ACCEPTANCE.md
  TRANCHE_3_MAINNET_ACCEPTANCE.md
  MAINNET_RUNBOOK.md
```

If the team chooses a single app/service package instead of separate `apps/` and `services/`, the same logical modules still need to exist: web UI, transaction builder, event indexer, oracle relay, monitor/alerting, deployment scripts, and E2E tests.

---

## 4. Shared technical architecture

### 4.1 Runtime components

| Component | Tranche 2 | Tranche 3 |
|---|---|---|
| Soroban contracts | Deployed to Testnet by script | Deployed to Mainnet by same script |
| Contract registry | Testnet manifest-backed | Mainnet manifest-backed |
| Web dApp | Public Testnet URL | Mainnet URL with dashboard and monitoring |
| Wallet | Freighter via `@stellar/freighter-api` | Freighter plus same signing path |
| Transaction builder | Simulate, assemble, return/sign/submit XDR | Same with Mainnet network/passphrase |
| Event indexer | Required for transaction history and E2E visibility | Required for compliance-activity dashboard |
| Oracle updates | Manual/scripted updates for Testnet | Production RedStone USDY/USD SEP-40 integration |
| Monitoring | Testnet status and CI evidence | Production alerting and dashboard status |

### 4.2 Data sources

| Data | Source | Notes |
|---|---|---|
| Contract state | Stellar RPC `getLedgerEntries` / generated contract clients | Authoritative for current protocol state |
| Contract events | Stellar RPC `getEvents` | Source for activity feeds and compliance history |
| Classic account/trustline state | Horizon `/accounts/{address}` | User asset balance, trustline, auth flags, sequence |
| Transaction submission/status | Stellar RPC `sendTransaction`, `getTransaction` | Required for Soroban result metadata |
| Oracle source | Testnet script; Mainnet RedStone SEP-40 | Mainnet source must be monitored and replay-safe |
| dApp registry | `deployment-manifest.json` | Must be checked against on-chain topology |

### 4.3 Reviewer evidence rule

Every funded deliverable must produce evidence that an independent reviewer can verify without trusting a private machine:

- Contract deployment evidence must include Stellar explorer links or Horizon/RPC transaction hashes.
- Build evidence must include source commit, build command, artifact hash, and on-chain WASM/code hash comparison.
- dApp evidence must include a public URL, screenshots, and transaction hashes for each completed flow.
- Dashboard evidence must map each displayed value to either a contract read or an indexed event query.
- Oracle evidence must include the source value, converted scaled value, submission transaction, freshness state, and any rejection/alert test.

Any deliverable evidence that depends on a local terminal only is supporting evidence, not acceptance evidence.

### 4.4 Non-acceptance blocker conditions

A tranche deliverable is not acceptance-ready if any blocker below is present, even when the feature appears to work locally:

| Area | Blocker |
|---|---|
| Deployment manifest | Contract IDs, WASM hashes, transaction IDs, admin addresses, or production parameters are missing, unverifiable, or inconsistent with on-chain state |
| Reproducible build | A reviewer cannot rebuild the WASM artifacts from the tagged source and compare them to on-chain code hashes or upload transaction hashes |
| Scripted deployment | Deployment depends on undocumented manual steps that are not recorded in the manifest or acceptance evidence |
| Oracle safety | Stale, unavailable, decreasing, or over-deviation oracle updates do not fail with explicit errors and recorded evidence |
| Oracle source identity | Primary/fallback source IDs are not deterministic, not documented in the manifest, or cannot be mapped from dashboard display to on-chain source hash |
| dApp signing | The dApp asks a wallet to sign before simulation, signs against the wrong network/passphrase, or cannot show the exact XDR hash being signed |
| User flows | Funded flows require private tooling instead of the public dApp, except for documented operator-only setup such as issuer authorization |
| E2E evidence | Tests do not publish transaction hashes, run IDs, screenshots/logs, or enough account context for a reviewer to reproduce the flow |
| Mainnet deployment | Mainnet launch uses a dirty source tree, untagged artifacts, hot-key admin custody, or undisclosed contract addresses |
| Mainnet smoke test | Smoke-test deposit/mint cannot be verified by transaction hash and contract state reads |
| Dashboard | A displayed current-state metric is cached/indexed without labeling source ledger, or has no contract/source mapping |
| Compliance activity | RecoveryEscrow or Permissioning history is shown without indexed transaction hashes and ledger timestamps |
| Alerting | Oracle stale/unavailable alerts are configured but not tested, or alert ownership/escalation is undocumented |

Known limitations are acceptable only when the acceptance document labels them clearly and explains why they do not block the funded success criteria.

### 4.5 Network configuration

Testnet:

```text
STELLAR_NETWORK=testnet
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
SOROBAN_RPC_URL=https://soroban-testnet.stellar.org
HORIZON_URL=https://horizon-testnet.stellar.org
```

Mainnet:

```text
STELLAR_NETWORK=mainnet
NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
SOROBAN_RPC_URL=<production RPC endpoint>
HORIZON_URL=https://horizon.stellar.org
```

The dApp must refuse to sign when the wallet network and market registry network do not match.

---

## 5. Tranche 2 Deliverable 1: Testnet Deployment + Oracle Safety

### 5.1 Funded objective

Deploy the complete protocol contract set to Stellar Testnet using a scripted, repeatable pipeline. Extend `OracleAdapter` with the safety features required for public deployment:

- Configurable primary and fallback price sources.
- Automatic protocol-wide block when no fresh price is available, surfacing `OracleUnavailable` on affected mint/deposit flows.
- Deviation guard rejecting updates that move more than +/-10 percent from the previous value.

Ongoing Testnet oracle freshness is maintained by manual/scripted updates, not a dedicated background service.

### 5.2 Required contract work

Current `OracleAdapter` supports admin-submitted monotonic values and freshness checks. Tranche 2 must extend it while preserving the existing monotonic PT/YT settlement invariant.

#### 5.2.1 OracleAdapter storage

Add instance storage:

```rust
enum DataKey {
    Admin,
    Price,
    Timestamp,
    PrimarySource,
    FallbackSource,
    ActiveSource,
    MaxStaleSeconds,
    MaxDeviationBps,
    LastRejectedValue,
    LastRejectedTimestamp,
    LastRejectReason,
}
```

Recommended defaults:

| Setting | Value |
|---|---:|
| `MaxStaleSeconds` | 3,600 |
| `MaxDeviationBps` | 1,000 (10 percent) |

The current monotonic non-decreasing `value >= current_price` rule must remain unless the protocol economics are redesigned for negative yield. Tranche 2 should not introduce negative oracle movement support.

#### 5.2.2 OracleAdapter public API

Add or adjust functions:

```rust
initialize(admin, max_stale_seconds, max_deviation_bps)
set_sources(caller, primary_source, fallback_source)
set_safety_params(caller, max_stale_seconds, max_deviation_bps)
set_reference_value(caller, source, value, timestamp)
get_primary_source() -> Option<BytesN<32>>
get_fallback_source() -> Option<BytesN<32>>
get_active_source() -> Option<BytesN<32>>
max_stale_seconds() -> u64
max_deviation_bps() -> u32
is_available() -> bool
last_rejection() -> Option<OracleRejection>
```

If the source is off-chain rather than an on-chain contract, represent it as a `BytesN<32>` source ID. Source IDs must be deterministic:

```text
source_id = sha256("principal-oracle-source:<network>:<asset>:<provider>:<feed>")
```

Example preimage:

```text
principal-oracle-source:mainnet:USDY:RedStone:USDY/USD-SEP40
```

The manifest must store both the human-readable preimage and the 32-byte hash. Events store the hash; dashboards resolve it through the manifest. Do not store arbitrary free-form strings on-chain.

This is a breaking ABI change from the Tranche 1-style `set_reference_value(caller, value, timestamp)` function. Acceptance requires:

- Manifest field `oracle_abi_version = 2`.
- Regenerated contract clients for the dApp, deployment scripts, oracle relay, and tests.
- Updated error decoder tables.
- A migration note in `TRANCHE_2_TESTNET_ACCEPTANCE.md` explaining that Tranche 2 deployments use the source-aware oracle ABI.
- Evidence that every oracle-consuming path and every oracle admin/update script calls the deployed ABI, not the legacy function.

#### 5.2.3 New errors

Add:

```rust
OracleUnavailable
DeviationTooHigh
InvalidSource
InvalidDeviation
InvalidStaleness
FallbackNotConfigured
```

Existing errors must remain stable where possible:

- `AlreadyInitialized`
- `Unauthorized`
- `InvalidValue`
- `TimestampTooOld`
- `NotInitialized`
- `ValueDecreased`
- `TimestampInFuture`

If error numbers change, update `API_REFERENCE.md`, `SECURITY.md`, dApp error decoding, and E2E assertions in the same PR.

Auditor requirement: new errors must be appended after existing error numbers where possible. Renumbering existing errors breaks dApp decoders, tests, and reviewer runbooks, so any renumbering requires a migration note in the acceptance document.

#### 5.2.4 Deviation guard

For an existing price `p` and proposed `v`:

```text
abs(v - p) * 10_000 / p <= max_deviation_bps
```

Because the oracle is monotonic today, `v < p` is rejected first by `ValueDecreased`. The deviation guard still protects against large upward jumps.

On rejection:

- Do not update `Price` or `Timestamp`.
- Store `LastRejectedValue`, `LastRejectedTimestamp`, and reason.
- Emit `ref_rej`.
- The dashboard must surface the rejection.

#### 5.2.5 Availability guard

Affected protocol operations already call `OracleAdapter.is_fresh(3600)` or equivalent through PrincipalManager/YTToken/MarketPool. Tranche 2 must make the public failure mode explicit:

- `PrincipalManager.mint`, `recombine`, `claim_yield`, first `settle_all`/implicit settlement must fail when oracle unavailable.
- `MarketPool` swaps must fail when oracle unavailable.
- `YTToken.update_yield_index` must fail when oracle unavailable unless already frozen.

The grant text says `OracleUnavailable` on mint/deposit. `SYWrapper.deposit` does not currently depend on the oracle. The required Tranche 2 behavior is therefore:

- `Router.wrap_and_mint` must fail atomically when the oracle is unavailable, leaving neither deposited SY nor minted PT/YT.
- Direct `PrincipalManager.mint` must fail when the oracle is unavailable.
- Standalone `SYWrapper.deposit` may remain oracle-independent, but the dApp must label it as "wrap only" and must not present it as the funded deposit-and-mint flow.

Do not add an oracle dependency to `SYWrapper` unless a later architecture decision explicitly chooses to block standalone wrapping too. Adding that dependency expands the blast radius of oracle outages and should not be done casually.

Expected unavailable-oracle behavior:

| Path | Expected failure |
|---|---|
| `PrincipalManager.mint` | `OracleUnavailable` before minting PT/YT |
| `PrincipalManager.recombine` | `OracleUnavailable` before burning PT/YT or returning SY |
| `PrincipalManager.claim_yield` | `OracleUnavailable` unless YT index is already frozen/settled |
| `PrincipalManager.settle_all` | `OracleUnavailable` if settlement needs a fresh rate |
| `MarketPool` live swaps | `OracleUnavailable` before reserve movement |
| `YTToken.update_yield_index` | `OracleUnavailable` unless already frozen |
| `Router.wrap_and_mint` | Atomic failure with no retained SY deposit and no PT/YT mint |
| `Router` swap/recombine/claim paths | Propagated user-safe oracle unavailable error from the downstream consumer |

Tests and dApp error decoding must assert these path-specific outcomes.

### 5.3 Deployment pipeline

Create `scripts/deploy/deploy.sh`.

Inputs:

- Single `.env` file.
- Target network: `testnet`.
- Source identity name or key reference.
- Underlying SAC address.
- Maturity timestamp, with first market set to about 90 days.
- Fee parameters.
- RiskControl limits.
- Oracle initial value and timestamp.

Required `.env` keys:

```bash
NETWORK=testnet
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
RPC_URL=https://soroban-testnet.stellar.org
HORIZON_URL=https://horizon-testnet.stellar.org
STELLAR_SOURCE_ACCOUNT=admin
UNDERLYING_SAC=
ISSUER_ADMIN_ADDRESS=
PROTOCOL_ADMIN_ADDRESS=
TREASURY_ADDRESS=
MARKET_MATURITY_UNIX=
TOKENIZATION_FEE_BPS=5
YT_FEE_BPS=1000
SWAP_FEE_TIER_BPS=10
PROTOCOL_SHARE_BPS=2000
RISK_CB_LIMIT=
RISK_ASSET_LIMIT=
RISK_WINDOW_LEDGERS=17280
ORACLE_INITIAL_VALUE=
ORACLE_INITIAL_TIMESTAMP=
ORACLE_MAX_STALE_SECONDS=3600
ORACLE_MAX_DEVIATION_BPS=1000
TIME_STRETCH_YEARS=2
```

Script responsibilities:

1. Validate required env vars.
2. Build WASM or verify existing build artifacts.
3. Compute SHA-256 for every WASM.
4. Deploy all eleven contracts in dependency order.
5. Initialize each contract.
6. Set the initial oracle reference value.
7. Wire RiskControl into SYWrapper and PrincipalManager.
8. Set PT/YT minters.
9. Deploy and wire RecoveryEscrow once to SYWrapper, PTToken, YTToken, and MarketPool.
10. Deploy Router and register MarketPool.
11. Apply compliance standing for protocol contracts where the test asset requires it.
12. Grant Permissioning standing for protocol contracts and initial test accounts.
13. Write `deployments/testnet/deployment-manifest.json`.
14. Write a transaction log for every deploy and init step.

RiskControl volume policy:

- The current primary `Router.wrap_and_mint` path reports volume through both `SYWrapper.deposit` and `PrincipalManager.mint`.
- Treat this as intentional double-reporting unless a later audited contract change makes the atomic flow count once.
- Configure Testnet and Mainnet circuit-breaker limits with this effective capacity in mind.
- The deployment manifest must state whether the market uses `double_reported_wrap_and_mint_capacity = true`.
- Dashboard capacity displays must show both raw remaining RiskControl capacity and effective wrap-and-mint capacity when double-reporting is active.

### 5.4 Idempotent deployment behavior

The deployment script must be safely repeatable:

- If a contract alias/address exists in the manifest, do not redeploy it unless `FORCE_REDEPLOY=true`.
- If initialization already happened, verify expected state instead of retrying and failing.
- If one-time setters (`set_minter`, `set_recovery_escrow`) are already set, verify the target address.
- If a prior run failed mid-way, resume from the next missing step.
- Never overwrite a successful manifest without making a timestamped backup.

### 5.5 Deployment manifest schema

`deployment-manifest.json`:

```json
{
  "schema_version": "1.0",
  "network": "testnet",
  "network_passphrase": "Test SDF Network ; September 2015",
  "rpc_url": "https://soroban-testnet.stellar.org",
  "horizon_url": "https://horizon-testnet.stellar.org",
  "source_commit": "git-sha",
  "built_at": "iso-8601",
  "wasm_target": "wasm32v1-none",
  "underlying": {
    "kind": "sac",
    "asset_code": "STA",
    "issuer": "G...",
    "sac": "C...",
    "decimals": 7
  },
  "market": {
    "market_id": "sta-testnet-90d-001",
    "maturity": 1790000000,
    "tokenization_fee_bps": 5,
    "yt_fee_bps": 1000,
    "swap_fee_tier_bps": 10,
    "protocol_share_bps": 2000
  },
  "contracts": {
    "oracle_adapter": {
      "id": "C...",
      "wasm_sha256": "...",
      "deploy_tx": "...",
      "init_tx": "..."
    }
  },
  "wiring": {
    "router_registered_pool": true,
    "pt_minter": "C...",
    "yt_minter": "C...",
    "recovery_escrow": "C...",
    "risk_consumers": ["C...", "C..."]
  },
  "transactions": []
}
```

Every contract entry must include:

- Contract ID.
- WASM path.
- WASM SHA-256.
- Deploy transaction hash.
- Initialize transaction hash.
- Admin/source address.
- Important post-init transaction hashes.

Manifest audit fields:

- `release_tag`
- `git_tree_dirty` (`false` for acceptance deployments)
- `rustc_version`
- `stellar_cli_version`
- `soroban_sdk_version`
- `build_container_image` or host build description
- `contract_code_hash_on_chain` where available from RPC/ledger entries
- `verified_at_ledger`
- `verified_by_script_version`

### 5.6 Post-deploy verification script

Create `scripts/deploy/verify-deployment.sh`.

It must assert:

- All manifest contract IDs exist on Stellar Testnet.
- WASM hashes match local build artifacts.
- `OracleAdapter.get_reference_value()` returns initial value.
- `OracleAdapter.is_available()` / `is_fresh()` returns true.
- `Permissioning.get_admin()` equals expected admin.
- `RiskControl` limits/window match `.env`.
- `RiskControl.is_consumer(SYWrapper)` and `is_consumer(PrincipalManager)` are true.
- `SYWrapper.underlying_address()` and `permissioning_address()` match manifest.
- PT/YT underlying, permissioning, maturity, and minter match manifest.
- PrincipalManager topology matches SY/PT/YT/oracle/config.
- MarketPool manager, SY, PT, YT, config, and maturity match manifest.
- RecoveryEscrow underlying matches all positions.
- SY/PT/YT/Pool recovery escrow points to RecoveryEscrow.
- Router has registered the pool and `Router.market(pool)` matches manifest.
- Protocol contracts have required compliance standing and Permissioning grants.
- Contract code hashes or deploy ledger entries match the manifest's WASM hashes.
- `git_tree_dirty` is false for acceptance deployments.

### 5.7 Testnet acceptance evidence

Publish `docs/TRANCHE_2_TESTNET_ACCEPTANCE.md` with:

- Link to deployment manifest.
- Every contract address.
- Every deployment/init/wiring transaction hash.
- WASM SHA-256 table.
- Verification script output.
- Oracle guard tests: fresh update accepted, stale unavailable state detected, >10 percent deviation rejected.
- Full PT/YT/AMM flow transaction hashes.
- Compliance recovery transaction hashes.

---

## 6. Tranche 2 Deliverable 2: dApp on Testnet

### 6.1 Funded objective

Ship a React dApp at a public Testnet URL where a reviewer can connect Freighter, deposit a test asset, receive PT/YT, trade through the market, provide liquidity, claim yield, redeem, and view positions.

### 6.2 Recommended stack

The repository currently has no frontend package. Add:

```text
apps/web
```

Recommended choices:

- React + Vite or Next.js.
- TypeScript.
- `@stellar/stellar-sdk`.
- `@stellar/freighter-api`.
- TanStack Query for RPC/cache state.
- Zod or equivalent for manifest validation.
- Playwright for browser E2E.
- Lightweight CSS system or existing design system.

The public URL can be Vercel, Netlify, Cloudflare Pages, or similar. It must not require credentials.

Auditor note: the dApp may be static, but transaction construction must still be deterministic and inspectable. If transactions are built in-browser, the signed XDR summary must be generated from the exact XDR sent to Freighter. If a backend builder is introduced, the frontend must display the builder output and reject any signed XDR whose hash differs from the reviewed XDR.

### 6.3 Wallet and network

Required:

- Connect Freighter.
- Read public key.
- Read/validate network where wallet API allows.
- Persist session locally.
- Provide disconnect.
- Refuse signing when the market registry network/passphrase differs from the wallet/network.
- Show XDR summary before signing.

### 6.4 Market page

Display:

- Market ID.
- Underlying asset.
- Maturity date.
- Time-to-maturity countdown.
- Current oracle rate.
- Oracle freshness/availability.
- Total PT outstanding from `PrincipalManager.total_pt()` and/or PT `total_supply()`.
- Total YT outstanding from `PrincipalManager.total_yt()` and/or YT `total_supply()`.
- Pool reserves and liquidity from `MarketPool.reserves()` / `pool_state()`.
- PT price from `MarketPool.pt_price()`.
- Implied fixed rate from `MarketPool.implied_rate()`.
- Fee settings from `MarketConfig`.
- RiskControl pause and volume state.

### 6.5 Compliance status

For connected wallet:

- Underlying SAC trustline exists.
- Underlying SAC authorization is true.
- `Permissioning.is_allowed(account)` is true.
- `Permissioning.is_allowed_for_asset(account, pt)` and `yt` are true where needed.
- For SEP-57 markets, account is identity-verified and not frozen.

The dApp must show specific failed reasons before asking the wallet to sign.

### 6.6 User panels

#### Deposit panel

For standalone SY deposit:

- Input underlying amount.
- Preview SY shares from `SYWrapper.exchange_rate()`.
- `min_shares_out` from slippage setting.
- Build `SYWrapper.deposit` only if standalone SY deposit is supported in UI.

For main user journey, prefer `Router.wrap_and_mint`.

#### Mint panel

For existing SY:

- Input SY shares.
- Preview PT/YT from oracle rate and tokenization fee.
- Build `PrincipalManager.mint` or Router equivalent.

#### Wrap and mint panel

Primary flow:

```text
Router.wrap_and_mint(from, pool, amount, min_pt_out, deadline)
```

Output:

- PT minted.
- YT minted.
- Fee shares.
- Transaction hash.

#### Trading panel

Flows:

```text
Router.swap_sy_for_pt
Router.swap_pt_for_sy
Router.swap_sy_for_yt
Router.swap_yt_for_sy
```

Quote and preflight:

- Use pool quote functions for simple PT trades.
- Use simulation-backed quote for YT flash-mint/flash-redeem.
- Include slippage controls.
- Include deadline.
- Surface oracle stale, expired market, unseeded pool, and insufficient liquidity.

#### Liquidity panel

Flows:

```text
Router.add_liquidity
Router.add_liquidity_single_sy
Router.remove_liquidity
```

Show:

- User PT/SY/LP balances.
- Expected LP minted/burned.
- Pool share.
- Minimum outputs.

#### Yield claim and redeem panels

Flows:

```text
Router.claim_yield
Router.recombine
Router.redeem_at_maturity
PrincipalManager.settle_all (keeper/helper)
```

Rules:

- Claim yield requires fresh oracle unless settled/frozen.
- Redeem at maturity can accept unequal PT and YT amounts.
- If market is mature but not settled, UI should show "settlement required" and trigger or include `settle_all`.
- After settlement, stale oracle must not block redemption.

### 6.7 Position dashboard

Show for connected wallet:

- Underlying balance.
- SY balance.
- PT balance.
- YT balance.
- LP balance.
- Pending YT claim.
- Current oracle rate.
- Initial/entry-rate information where available from indexer events.
- Projected fixed return from PT at maturity.
- Trade/liquidity history.

Because per-position entry rate is not exposed as a simple contract view, reconstruct "locked initial_rate" from mint and transfer events where possible. If reconstruction is incomplete, label the value as indexed/estimated and keep claim/redeem previews simulation-backed.

### 6.8 Transaction history

Use protocol events indexed from Stellar RPC:

- Deposits.
- Mints.
- Transfers.
- Swaps.
- Liquidity add/remove.
- Yield claims.
- Recombines.
- Redemptions.
- Recovery events.
- Oracle updates.

For each event:

- Ledger sequence.
- Timestamp.
- Transaction hash.
- Contract.
- Event type.
- Raw amounts.
- Formatted amounts.

### 6.9 Transaction builder

Baseline for Tranche 2 is browser-side transaction construction and simulation. A backend builder can be added later, but it is not required for acceptance. Browser-side construction keeps the Testnet dApp simpler to review and avoids introducing a server trust dependency before Mainnet.

Required lifecycle:

1. Build intent.
2. Resolve market manifest.
3. Load account sequence.
4. Construct Soroban invocation.
5. Simulate with Stellar RPC.
6. Apply footprint/auth/resource fee.
7. Return unsigned XDR and summary.
8. Ask Freighter to sign.
9. Submit signed XDR.
10. Poll until final.
11. Decode result and events.

Never ask Freighter to sign before simulation.

Security invariant: the dApp must compute and display a hash of the XDR that the user is asked to sign. If the XDR is rebuilt after simulation because sequence, fee, footprint, or deadline changed, the old summary must be discarded and a new wallet prompt must be shown.

### 6.10 Testnet dApp acceptance

To satisfy Deliverable 2:

- Public URL is live.
- Freighter connect works.
- Test account can see compliance status.
- Atomic `Router.wrap_and_mint` works.
- PT/YT balances update.
- PT/YT trading works.
- Liquidity deposit/withdraw works.
- Yield claim works after oracle update.
- Maturity redeem works after settlement.
- Transaction history shows events.
- Interface blocks known invalid actions: missing trustline, missing Permissioning, stale oracle, matured market trades, paused state, slippage, deadline.

---

## 7. Tranche 2 Deliverable 3: E2E Tests + Testnet Walkthrough

### 7.1 Funded objective

Add an automated suite exercising the full protocol flow against live Testnet contracts and publish a step-by-step walkthrough with screenshots for reviewers.

### 7.2 Live Testnet E2E suite

Add CI job:

```text
e2e-testnet
```

It should run on:

- Pull requests, either on a scheduled/manual mode if Testnet funding/keys are sensitive.
- Nightly schedule.
- Manual `workflow_dispatch`.

Required secrets:

- Testnet source key(s).
- Testnet issuer/admin key for test asset, if test creates fresh accounts/grants.
- RPC URL.
- Horizon URL.
- Deployment manifest path or JSON.

### 7.3 E2E scenario

The suite must cover:

1. Faucet funding or funded test accounts.
2. Trustline setup for the test asset.
3. SAC authorization / Permissioning grant.
4. Oracle fresh update.
5. Underlying deposit.
6. PT/YT issuance.
7. PT/YT transfers.
8. PT trade.
9. YT trade through Router.
10. Liquidity add.
11. Liquidity remove.
12. Yield claim after oracle rate update.
13. Maturity settlement.
14. Redemption.
15. Compliance recovery: deauthorize target, `seize_all_positions`, `finalize_record`.

Because a 90-day market cannot mature inside a normal CI run, create one of these test modes:

- `live-long-market`: exercises all pre-maturity flows against the 90-day public market.
- `live-short-market`: deploys or uses a short-lived test market for settlement/redeem/recovery finalization.

The public reviewer walkthrough should use the 90-day market for ordinary flows and a separately labeled short-lived market for maturity-only proof if needed.

Live E2E isolation rules:

- Each CI run uses fresh funded accounts or an account pool with per-run sequence locking.
- Each CI run writes a run ID into logs and acceptance artifacts.
- Shared long-market tests must avoid consuming scarce balances or changing admin state unexpectedly.
- Short-market tests must deploy a dedicated market or use a dedicated E2E market, not the reviewer-facing 90-day market.
- Recovery tests must target dedicated accounts only; never deauthorize a reviewer/demo account.
- Faucet failures must not be the only funding path; maintain a funded Testnet account pool for CI.

### 7.4 Assertions

Every E2E step must assert:

- Transaction success.
- Expected contract return value.
- Expected event observed.
- User balances changed as expected.
- Indexed state catches up to ledger.
- UI, if included in browser test, shows the new state.

### 7.5 Walkthrough document

Create:

```text
docs/TRANCHE_2_TESTNET_WALKTHROUGH.md
```

It must include:

- Public dApp URL.
- Testnet network/passphrase.
- Freighter setup instructions.
- Test account funding instructions.
- Underlying trustline/compliance approval instructions.
- Step-by-step screenshots:
  - Connect wallet.
  - View market.
  - Atomic `Router.wrap_and_mint`.
  - Trade PT.
  - Trade/buy YT.
  - Add/remove liquidity.
  - Claim yield.
  - Redeem after maturity or short-market demo.
  - View transaction history.
- Links to transaction hashes for the reference run.
- Troubleshooting: stale oracle, missing trustline, not authorized, insufficient XLM reserve, wrong network.

### 7.6 Acceptance evidence

Create:

```text
docs/TRANCHE_2_TESTNET_ACCEPTANCE.md
```

Required contents:

- CI run links for live E2E.
- Deployment manifest link.
- Public dApp URL.
- Walkthrough link.
- Reference run transaction table.
- Known limitations and whether they block grant acceptance.

---

## 8. Tranche 3 Deliverable 1: Mainnet Deployment + Production Configuration

### 8.1 Funded objective

Deploy the complete protocol contract set to Stellar Mainnet using the Tranche 2 scripted pipeline, adapted for production. Publish `mainnet-deployment-manifest.json`, configure production RiskControl/oracle/maturity/fees/compliance, publish reproducible build instructions, switch the dApp to Mainnet contracts, and run a small smoke-test deposit and mint.

### 8.2 Production deployment policy

Mainnet deployment requires a release tag.

Required gates before deployment:

- Contracts and deployment scripts merged.
- Test suite passing.
- Testnet E2E passing.
- Tranche 2 acceptance doc complete.
- WASM artifacts built from clean source.
- Release tag created.
- Admin/issuer/protocol keys confirmed.
- Multisig or hardware custody ready where applicable.
- Emergency pause procedure rehearsed.

### 8.3 Reproducible build

Create:

```text
deployments/mainnet/reproducible-build.md
```

It must include:

- Git commit and tag.
- `Cargo.lock` SHA-256.
- `rust-toolchain.toml` SHA-256, or explicit Rust toolchain version if no toolchain file exists.
- Build container image digest, or host OS/toolchain description if a container is not used.
- Rust version.
- Stellar CLI version.
- Target: `wasm32v1-none`.
- Optimizer flags and profile settings.
- Build command:

```bash
cargo build --workspace --release --target wasm32v1-none
```

- SHA-256 command.
- Table of local WASM hashes.
- Table of on-chain WASM hashes or deploy transaction references.
- Confirmation that each contract is below Soroban's contract size limit.

Build reproducibility must compare the local artifact hash to the ledger's installed contract code hash or to the upload transaction's WASM hash. Comparing only local files to the manifest is insufficient.

### 8.4 Mainnet manifest

`deployments/mainnet/mainnet-deployment-manifest.json` must mirror the Testnet manifest and include:

- Network/passphrase.
- Source commit/tag.
- Contract addresses.
- WASM SHA-256 hashes.
- Deployment transaction IDs.
- Init/wiring transaction IDs.
- Production parameters:
  - RiskControl limits.
  - Market maturity.
  - Fee rates.
  - Oracle settings.
  - Underlying SAC and issuer admin.
  - Protocol admin and treasury.
  - Permissioning policy.
- Smoke-test transaction hashes.

### 8.5 Production parameters

RiskControl:

- Set nonzero protocol-wide circuit-breaker limit.
- Set per-asset limit for first underlying.
- Confirm window length.
- Register pausers.
- Register SYWrapper and PrincipalManager as consumers.
- Document whether wrap-and-mint capacity is double-reported and calibrate limits accordingly.

Oracle:

- Configure production source: RedStone USDY/USD SEP-40 for initial USDY market.
- Set max staleness.
- Set max deviation.
- Submit initial value.
- Confirm `is_available` / `is_fresh`.

Market:

- Underlying SAC: production USDY SAC.
- Maturity: first live market date.
- Fees: tokenization, YT fee, swap fee tier, protocol share.
- MarketPool time stretch.
- Router registration.

Compliance:

- Issuer admin signs market creation.
- RecoveryEscrow operations require the underlying issuer authority signer, not merely the protocol admin.
- Protocol contracts that hold value are authorized/verified on the underlying.
- Protocol contracts receive Permissioning grants.
- Treasury and creator fee recipients are eligible.
- Initial user for smoke test is eligible.

### 8.6 Smoke test

Run with a small amount:

1. Confirm user eligibility.
2. Confirm the user has issuer approval and enough underlying asset balance.
3. Submit or verify a fresh oracle value.
4. Submit one atomic `Router.wrap_and_mint` transaction.
5. Confirm PT/YT balances.
6. Optionally make a tiny PT trade if liquidity is seeded and approved by launch policy.
7. Record transaction hashes.
8. Confirm dashboard reflects the change.

Smoke test must not depend on a private UI session. It should be verifiable by transaction hash and contract state reads.

Smoke-test evidence must include:

- Launch account.
- Raw and formatted amount.
- `Router.wrap_and_mint` transaction hash and explorer link.
- Pre/post underlying, SY, PT, and YT balances.
- Oracle value/timestamp used.
- RiskControl pre/post volume and remaining capacity.
- Dashboard screenshot or metric proof showing the Mainnet state update.
- If separate deposit/mint calls are also tested, label them secondary and do not use them as the primary success proof.

Mainnet smoke-test guardrails:

- Use a pre-approved launch account with explicit issuer/compliance standing.
- Use the smallest meaningful amount above contract dust/rounding thresholds.
- Do not run recovery or deauthorization tests on Mainnet as part of grant acceptance.
- Do not rely on unpublished private transaction hashes; publish explorer links.
- Include a rollback/response note for what operators will do if smoke test simulation fails.

### 8.7 Acceptance criteria

Tranche 3 Deliverable 1 is complete when:

- All production contracts are live on Mainnet.
- `mainnet-deployment-manifest.json` is committed.
- Reproducible build instructions are committed.
- On-chain addresses and WASM hashes are verified.
- Initial market configuration is active.
- Smoke-test deposit and mint succeed.
- Smoke-test success is proven by atomic `Router.wrap_and_mint` evidence and post-state reads.
- Mainnet dApp points to Mainnet manifest.

---

## 9. Tranche 3 Deliverable 2: dApp Dashboard + Protocol Monitoring

### 9.1 Funded objective

Build a public, real-time view into Mainnet protocol state directly into the Mainnet dApp. It must require no wallet, credentials, or special tooling.

Every current-state figure should be read live from contracts. Compliance-activity history can use a small indexer because historical RecoveryEscrow and Permissioning events are not fully queryable from current contract state alone.

Dashboard data-fetch architecture:

- Current-state widgets should read through a small API proxy or generated client layer that records RPC endpoint, ledger, and raw result for each widget.
- Browser-direct RPC reads are acceptable only if rate limits, CORS, fallback endpoints, and raw-result inspection are handled explicitly.
- Default refresh cadence: 15-30 seconds for health/safety/oracle widgets, 60-120 seconds for slower protocol aggregates.
- Cache TTL must be shorter than the visible refresh cadence and displayed when stale.
- Each widget needs loading, stale, unavailable, and partial-indexer states.
- RPC fallback must not silently mix values from different ledgers without labeling the ledger mismatch.

### 9.2 Dashboard pages

Add public routes:

```text
/dashboard
/dashboard/markets
/dashboard/markets/:market_id
/dashboard/oracle
/dashboard/risk
/dashboard/compliance
/dashboard/transactions
```

### 9.3 Protocol health

Show:

- Total value locked.
- Outstanding PT supply per market.
- Outstanding YT supply per market.
- PT/SY pool liquidity.
- Pool reserves.
- LP total supply.
- Pending yield claimable per connected/viewed position where an account is supplied.
- Market maturity and settlement status.
- Fee buckets accrued in PrincipalManager and MarketPool.

Metric definitions:

- TVL in underlying units: `SYWrapper.total_underlying()`, plus any raw underlying held in RecoveryEscrow if the dashboard includes recovered funds in protocol custody. The dashboard must label whether recovered escrow balances are included.
- TVL in USD/reference units: `underlying_units * OracleAdapter.get_reference_value() / SCALE`.
- PT supply: PT token `total_supply()` and/or `PrincipalManager.total_pt()`, with any difference explained.
- YT supply: YT token `total_supply()` and/or `PrincipalManager.total_yt()`, with any difference explained.
- Pool liquidity: `MarketPool.reserves()` valued using the current oracle rate for SY notional and raw PT notional.
- Pending yield per position: computed only for indexed/current holders. If holder enumeration is incomplete, the dashboard must label aggregate pending yield as "indexed holders only" and provide the indexer coverage ledger.
- Public no-wallet dashboards should show aggregate pending yield only when indexed-holder coverage is complete enough to label the value honestly. Otherwise expose pending yield as an address lookup and label aggregate values as estimated/indexed coverage.

Contract sources:

- `SYWrapper.total_underlying()`
- `SYWrapper.total_shares()`
- `PrincipalManager.total_pt()`
- `PrincipalManager.total_yt()`
- `PrincipalManager.settled_rate()`
- `PrincipalManager.accrued_fees()`
- `MarketPool.reserves()`
- `MarketPool.pool_state()`
- `MarketPool.lp_total_supply()`
- `MarketPool.accrued_fees()`
- PT/YT `total_supply()`
- `YTToken.pending_claim(account)` for account-specific view

### 9.4 Safety status

Show:

- RiskControl protocol circuit-breaker volume.
- Remaining protocol capacity.
- Per-asset volume and remaining capacity.
- RiskControl pause state.
- RiskControl window start and length.
- SYWrapper pause state.
- PrincipalManager pause state.
- MarketPool pause state.
- Oracle current value.
- Oracle timestamp.
- Oracle freshness/availability.
- Last oracle deviation rejection.
- Feed service health from Deliverable 3.

Contract/service sources:

- `RiskControl.get_cb_limit()`
- `RiskControl.get_cb_volume()`
- `RiskControl.get_asset_limit(asset)`
- `RiskControl.get_asset_volume(asset)`
- `RiskControl.get_window_ledgers()`
- `RiskControl.get_window_start()`
- `RiskControl.is_paused()`
- `SYWrapper.is_paused()`
- `PrincipalManager.is_paused()`; this view is mandatory for Mainnet dashboard acceptance.
- `MarketPool.is_paused()`
- `OracleAdapter.get_reference_value()`
- `OracleAdapter.get_reference_timestamp()`
- `OracleAdapter.is_fresh(max)`
- New OracleAdapter rejection/status views from Tranche 2.

If a pause view is missing from any contract needed by the dashboard, Tranche 2/3 implementation must add the smallest possible read-only view in that contract rather than rely permanently on event reconstruction. Event reconstruction is acceptable as a temporary bridge for historical context, but the dashboard's current safety status must prefer a live contract read.

### 9.5 Compliance activity

Show:

- RecoveryEscrow seizures.
- RecoveryEscrow finalizations.
- Account recovered.
- Record ID.
- Amounts seized by position type.
- Underlying recovered.
- Transaction hash.
- Ledger/time.
- Current underlying SAC administrator.
- Permissioning grant/revoke activity.
- Aggregate KYC/eligibility counts where available from indexed Permissioning events.

Current-state contract sources:

- `RecoveryEscrow.record_count()`
- `RecoveryEscrow.get_record(id)`
- `RecoveryEscrow.account_records(account)`
- `RecoveryEscrow.underlying_address()`
- Underlying SAC `admin()`
- `Permissioning.is_allowed(account)`
- `Permissioning.is_allowed_for_asset(account, asset)`

Historical source:

- Indexer over `RecoveryEscrow` `recovery` / `finalize` events.
- Indexer over Permissioning events.

### 9.6 Dashboard validation

Each dashboard metric must have:

- Label.
- Value.
- Source contract/function or indexer source.
- Contract ID.
- Function arguments.
- Raw returned value.
- Formatted displayed value.
- Last ledger sequence.
- Timestamp.
- RPC endpoint or indexer coverage ledger.
- Cache/stale status.
- Link to explorer where relevant.

For reviewer confidence, include an "How to verify" drawer that maps each figure to a contract read.

Dashboard failure modes:

- If RPC is stale, show the last ledger and mark data stale.
- If indexer is behind, show the indexed-through ledger and do not mix it silently with newer contract reads.
- If oracle is stale, keep displaying the last accepted value but mark affected operations blocked.
- If contract reads fail, show "unavailable" rather than cached green status.

### 9.7 Acceptance criteria

Deliverable 2 is complete when:

- Dashboard and monitoring views are live at a public Mainnet URL.
- No wallet is required for dashboard read-only views.
- Protocol health values reflect Mainnet contract state.
- Safety status surfaces RiskControl, pause, and oracle status.
- Compliance activity shows RecoveryEscrow and relevant compliance events.
- Every displayed figure has a source and can be independently confirmed.

---

## 10. Tranche 3 Deliverable 3: Mainnet Oracle Integration + Monitoring

### 10.1 Funded objective

Integrate the production oracle source used by the first live market. For the initial USDY market, the primary source is the RedStone USDY/USD SEP-40 feed. `OracleAdapter` remains the protocol contract responsible for the exchange rate, freshness, fallback, and deviation checks.

### 10.2 Oracle relay service

Add:

```text
services/oracle-relay
```

Responsibilities:

- Fetch RedStone USDY/USD SEP-40 value.
- Validate source authenticity according to the exact RedStone/SEP-40 source configured in the manifest.
- Record the feed contract/API endpoint, source identifier, authorized signer set or verification method, decimals, message format, and timestamp semantics.
- Reject replayed source payload hashes and payloads older than the last accepted source timestamp.
- Reject source data older than the configured source staleness threshold even if the on-chain oracle is still fresh.
- Convert value to `SCALE = 10_000_000`.
- Document integer conversion and rounding from feed decimals to `SCALE = 10_000_000`.
- Compare against current on-chain value.
- Reject local submission if value is lower than current on-chain value.
- Reject local submission if deviation is above configured threshold.
- Use ledger close time or a timestamp not later than ledger time.
- Submit `OracleAdapter.set_reference_value`.
- Retry transient RPC failures safely.
- Emit logs/metrics.

The relay must persist an append-only local or database audit log for every attempted update:

- Source payload identifier/hash.
- Raw source value.
- Scaled value.
- Source timestamp.
- Ledger timestamp used.
- Previous on-chain value.
- Decision: submitted, rejected, or skipped.
- Reason.
- Transaction hash, if submitted.

This audit log is part of oracle acceptance evidence.

### 10.3 Fallback behavior

The funded text requires primary and fallback sources. The service and OracleAdapter must define:

- Primary source: RedStone USDY/USD SEP-40.
- Fallback source: operator-approved backup source or previous accepted value heartbeat, depending on risk policy.
- When fallback is allowed.
- How fallback use is recorded.
- Whether fallback can update value or only refresh timestamp.

Recommended conservative policy:

- Primary source updates value.
- Fallback may refresh timestamp only when value matches the last accepted value, unless explicitly configured by admin.
- Any fallback value movement requires the same monotonic and deviation checks.
- Dashboard labels active source.

Fallback must not hide primary-source failure. If fallback is used, the dashboard and alerts must still show that the primary source is unavailable or rejected.

### 10.4 Monitoring

Metrics:

- Last source fetch timestamp.
- Last successful on-chain update timestamp.
- On-chain oracle timestamp.
- Staleness seconds.
- Current value.
- Current source.
- Rejected update count.
- Last rejection reason.
- RPC submission failures.
- Feed fetch failures.
- Deviation from previous accepted value.

Alerts:

- Feed unavailable.
- On-chain oracle older than warning threshold.
- On-chain oracle older than max staleness.
- Deviation rejection.
- Value decrease attempt.
- RPC submission failure after retries.
- Signer balance low.
- Signer authorization failure.

Alert channels:

- At minimum: email or Discord/Telegram/Slack webhook.
- Production recommended: PagerDuty/Opsgenie or equivalent for stale feed.

Alert acceptance evidence must include alert threshold, channel, owner, test timestamp, emitted alert payload or screenshot, and resolution status.

### 10.5 Dashboard integration

Deliverable 2 dashboard must display:

- Current oracle value.
- Current oracle timestamp.
- Fresh/stale/unavailable status.
- Primary/fallback source status.
- Last successful update transaction hash.
- Last rejection reason, if any.
- Time until affected protocol operations will block.

### 10.6 Protocol operation blocking

Acceptance must demonstrate:

- When oracle is fresh, mint/trade/claim/settle paths work.
- When oracle is stale or unavailable, affected operations fail as designed.
- After a fresh update, affected operations resume.

This can be demonstrated on Testnet before Mainnet and with a safe Mainnet read/simulation where no harmful transaction is submitted.

Mainnet blocking evidence should use simulation where possible. Do not intentionally stale the production oracle just to prove blocking on Mainnet; use Testnet for destructive/staleness drills and Mainnet simulation or historical stale-window evidence for acceptance.

### 10.7 Acceptance criteria

Deliverable 3 is complete when:

- Production oracle source is integrated.
- OracleAdapter receives live Mainnet updates.
- Dashboard shows current price and freshness status.
- Stale/unavailable prices block affected protocol operations.
- Automated alerting is active and tested.

---

## 11. Cross-tranche CI/CD

### 11.1 Existing contract CI

Continue running:

```bash
cargo test --workspace
cargo build --workspace --release --target wasm32v1-none
mdbook build
```

### 11.2 New CI jobs

Add:

| Job | Purpose |
|---|---|
| `deploy-dry-run` | Validate scripts, env parsing, manifest schema without submitting |
| `manifest-validate` | Validate deployment manifests against JSON schema |
| `web-lint` | Frontend lint/typecheck |
| `web-test` | Frontend unit/component tests |
| `e2e-local` | Browser tests against mocked/local config |
| `e2e-testnet-live` | Live Testnet flow against deployed contracts |
| `oracle-service-test` | Oracle source conversion, deviation guard, alert simulation |
| `docs-build` | mdBook including new walkthrough/acceptance docs |

Live Testnet CI may be `workflow_dispatch` if secrets or network costs make every-PR execution impractical. If so, the acceptance document must include the latest successful run link.

---

## 12. Security requirements

### 12.1 Key management

Testnet:

- Local keys acceptable.
- Secrets must still not be committed.

Mainnet:

- Deployment/admin keys must use multisig, hardware custody, or controlled signer service.
- Oracle signer must have limited operational scope.
- Treasury address must be verified.
- Pauser role must be separate from unpause/admin authority.

### 12.2 Frontend safety

- Hard-code no arbitrary contract IDs from URL params.
- Load markets only from signed/committed manifest.
- Display network passphrase.
- Simulate before signing.
- Show transaction summary.
- Use deadlines and min-out parameters.
- Decode errors safely.
- Do not expose RecoveryEscrow to ordinary users.

### 12.3 Deployment safety

- Back up manifest before re-run.
- Verify one-time wiring before first market use.
- Confirm maturity is far enough in the future.
- Confirm fee recipients are compliant.
- Confirm oracle timestamp is not in the future.
- Confirm RiskControl consumers are registered and wired.

### 12.4 Oracle safety

- Enforce monotonic non-decreasing reference value.
- Enforce max deviation.
- Enforce max staleness.
- Store and expose rejection history.
- Alert before protocol operations become blocked.

---

## 13. Acceptance checklist

### 13.1 Tranche 2 Deliverable 1

- [ ] `deploy.sh` exists and reads one `.env`.
- [ ] Deployment is repeatable/idempotent.
- [ ] `deployment-manifest.json` is generated.
- [ ] WASM hashes are recorded.
- [ ] Deployment transaction IDs are recorded.
- [ ] Post-deploy verification script passes.
- [ ] Oracle safety params are configured.
- [ ] +/-10 percent deviation rejection is demonstrated.
- [ ] Oracle unavailable/stale state blocks affected flows.
- [ ] RiskControl limits are configured.
- [ ] 90-day first Testnet market is configured.
- [ ] Compliance and RecoveryEscrow are active on-chain.
- [ ] Negative cases are demonstrated: Permissioning denial, SAC deauthorization denial, RiskControl pause, RiskControl limit trip, RecoveryEscrow unauthorized caller rejection, and recovery target still authorized rejection.

### 13.2 Tranche 2 Deliverable 2

- [ ] Public Testnet dApp URL is live.
- [ ] Freighter wallet connects.
- [ ] Market page shows required live fields.
- [ ] Compliance status is shown.
- [ ] Atomic `Router.wrap_and_mint` works.
- [ ] PT/YT previews work.
- [ ] PT/YT trading works.
- [ ] Liquidity add/remove works.
- [ ] Position dashboard works.
- [ ] Yield claim works.
- [ ] Redeem panel works.
- [ ] Transaction history from events works.
- [ ] UI blocks compliance/oracle/maturity restricted actions.

### 13.3 Tranche 2 Deliverable 3

- [ ] Live Testnet E2E suite exists.
- [ ] E2E suite runs in CI or documented manual CI dispatch.
- [ ] E2E covers funding, deposit, mint, transfer, trading, liquidity, yield claim, maturity settlement, and recovery.
- [ ] Walkthrough with screenshots is published.
- [ ] Independent reviewer can reproduce the flow.

### 13.4 Tranche 3 Deliverable 1

- [ ] Mainnet deployment manifest is published.
- [ ] All production contracts are live.
- [ ] WASM hashes are verified.
- [ ] Reproducible build instructions are published.
- [ ] Production RiskControl params are set.
- [ ] Production oracle config is set.
- [ ] First market maturity/fees are set.
- [ ] Compliance config is active.
- [ ] dApp points to Mainnet contracts.
- [ ] Smoke-test deposit and mint succeed.
- [ ] Smoke test uses atomic `Router.wrap_and_mint` as primary evidence.

### 13.5 Tranche 3 Deliverable 2

- [ ] Public dashboard is live.
- [ ] No wallet required for read-only monitoring.
- [ ] TVL, PT/YT supply, liquidity, and pending yield views work.
- [ ] RiskControl status works.
- [ ] Pause status works.
- [ ] Oracle freshness and rejection status work.
- [ ] RecoveryEscrow activity feed works.
- [ ] Current SAC admin and compliance status are visible.
- [ ] Values can be independently confirmed against contracts.

### 13.6 Tranche 3 Deliverable 3

- [ ] RedStone USDY/USD SEP-40 integration is implemented.
- [ ] Oracle relay submits production updates.
- [ ] Current price and freshness show on dashboard.
- [ ] Stale/unavailable prices block affected operations.
- [ ] Alerting is active.
- [ ] Alert test evidence is published.

---

## 14. Engineering work packages

Each work package should be a reviewable PR or small PR sequence. A reviewer should be able to map every PR to one funded deliverable.

### 14.1 Contract and oracle-safety package

Deliverable mapping: Tranche 2 Deliverable 1, Tranche 3 Deliverable 3.

Tasks:

- Extend `OracleAdapter` storage, errors, events, and views.
- Add source IDs as deterministic `BytesN<32>` values derived from `sha256("principal-oracle-source:<network>:<asset>:<provider>:<feed>")`.
- Add deviation and availability checks.
- Update oracle-consuming contracts or adapters so unavailable/stale oracle failures surface clearly.
- Update `API_REFERENCE.md`, `SECURITY.md`, and `TECHNICAL_SPECIFICATION.md`.
- Add unit tests for source config, stale/unavailable state, deviation rejection, monotonic rejection, future timestamp rejection, and fallback behavior.
- Add integration tests proving affected mint/trade/claim/settle flows fail when oracle is unavailable.

Done when:

- `cargo test --workspace` passes.
- Error decoding docs are updated.
- The dApp can read oracle status and last rejection from contract views.

### 14.2 Deployment automation package

Deliverable mapping: Tranche 2 Deliverable 1, Tranche 3 Deliverable 1.

Tasks:

- Add `scripts/deploy/deploy.sh`.
- Add `scripts/deploy/verify-deployment.sh`.
- Add `scripts/deploy/env.example`.
- Add manifest JSON schema and examples.
- Add dry-run mode.
- Add idempotent resume behavior.
- Add SHA-256 generation.
- Add deployment transaction capture.
- Add topology verification for every contract.

Done when:

- A fresh Testnet deployment can be created from one `.env`.
- Re-running the script verifies existing state without corrupting the manifest.
- Verification script exits nonzero on any topology mismatch.

### 14.3 Web dApp package

Deliverable mapping: Tranche 2 Deliverable 2.

Baseline decision: implement the web app as `apps/web` using React, TypeScript, Vite, `@stellar/stellar-sdk`, `@stellar/freighter-api`, TanStack Query, Zod, and Playwright. This keeps the app simple to deploy as static assets while leaving room for a backend builder service later.

Tasks:

- Add manifest loader and validator.
- Add wallet/session/network module.
- Add Stellar RPC/Horizon clients.
- Add contract client wrappers.
- Implement browser-side transaction construction, simulation, XDR hash display, signing, submission, and polling as the Tranche 2 baseline.
- Treat backend transaction builder as optional; if added, require unsigned XDR hash verification and post-sign hash/call checks.
- Add market page.
- Add compliance status panel.
- Add wrap-and-mint, trade, liquidity, claim, recombine, redeem panels.
- Add transaction summary modal.
- Add signed XDR submission and polling.
- Add transaction history from indexer/events.
- Add disabled-state reasons for compliance, oracle, maturity, pause, slippage, and liquidity failures.
- Add wallet state handling for not installed, locked, connected, wrong network, account/network changed, user rejection, disconnect, and retry.
- Add restore-footprint handling for archived Soroban entries.

Done when:

- Public Testnet URL is live.
- A reviewer can complete all funded flows through the browser.
- Browser tests cover connect, market display, every funded user flow, key wallet failure states, invalid-action blocking, and at least one live Testnet transaction path.

### 14.4 Indexer and transaction-history package

Deliverable mapping: Tranche 2 Deliverable 2, Tranche 3 Deliverable 2.

Tasks:

- Add event ingestion from Stellar RPC `getEvents`.
- Store raw event XDR.
- Decode normalized protocol events.
- Store transaction hash, ledger, timestamp, contract, event topic, account, market, and raw amounts.
- Store cursor/checkpoint state, indexed-through ledger, idempotent event keys, and RPC pagination metadata.
- Expose read APIs for user activity, market activity, compliance activity, and oracle updates.
- Add reconciliation jobs that compare indexed state against contract views.
- Document re-indexing from deployment ledger or checkpoint.

Done when:

- dApp transaction history updates after live Testnet transactions.
- Dashboard compliance activity can be rendered from indexed events.
- Re-indexing from a ledger checkpoint is documented and tested.

### 14.5 Live E2E and walkthrough package

Deliverable mapping: Tranche 2 Deliverable 3.

Tasks:

- Add `scripts/e2e/run-live-testnet-flow.ts`.
- Add CI workflow for live Testnet E2E.
- Add separate long-market and short-market modes.
- Capture transaction hashes and screenshots.
- Publish `TRANCHE_2_TESTNET_WALKTHROUGH.md`.
- Publish `TRANCHE_2_TESTNET_ACCEPTANCE.md`.

Done when:

- Latest E2E CI run is linked in the acceptance doc.
- Independent reviewer can reproduce the walkthrough.

### 14.6 Mainnet launch package

Deliverable mapping: Tranche 3 Deliverable 1.

Tasks:

- Freeze release tag.
- Build reproducible WASM artifacts.
- Publish `deployments/mainnet/reproducible-build.md`.
- Run Mainnet deployment script with production `.env`.
- Publish `mainnet-deployment-manifest.json`.
- Configure RiskControl, oracle, fees, maturity, compliance standing, pausers, treasury, and Router.
- Run atomic `Router.wrap_and_mint` smoke test.
- Switch dApp manifest to Mainnet.
- Publish `TRANCHE_3_MAINNET_ACCEPTANCE.md`.

Done when:

- Mainnet contract IDs and smoke-test transaction hash are public.
- Local rebuild checksums match the manifest.

### 14.7 Dashboard and monitoring package

Deliverable mapping: Tranche 3 Deliverable 2 and 3.

Tasks:

- Add public dashboard routes.
- Add live contract reads for protocol health.
- Add live RiskControl and pause status.
- Add oracle status and source status.
- Add compliance activity feed.
- Add RedStone oracle relay service.
- Add alerting and alert test.
- Add dashboard "How to verify" source mapping.

Done when:

- Public dashboard requires no wallet.
- Every displayed current-state metric has a contract source and ledger timestamp.
- Oracle stale/unavailable alert has been tested and recorded.

---

## 15. Acceptance evidence templates

### 15.1 Testnet acceptance document template

`docs/TRANCHE_2_TESTNET_ACCEPTANCE.md` should use this structure:

````markdown
# Tranche 2 Testnet Acceptance

Date:
Commit:
Git tree clean:
Public dApp URL:
Network:
RPC URL:
Horizon URL:
Manifest:
Manifest validation output:

## Blocker Disposition

| Blocker from spec section 4.4 | Status | Evidence link |
|---|---|---|

## Deliverable 1: Testnet Deployment + Oracle Safety

| Contract | Address | WASM SHA-256 | On-chain code hash / upload tx | Deploy Tx | Init Tx | Explorer |
|---|---|---|---|---|---|---|

Verification command:

```bash
scripts/deploy/verify-deployment.sh deployments/testnet/deployment-manifest.json
```

Oracle safety evidence:

| Check | Tx / Simulation | Raw result/error | Expected result | Evidence |
|---|---|---|---|---|

Negative safety evidence:

| Case | Tx / Simulation | Expected rejection | Evidence |
|---|---|---|---|
| Permissioning denial | | | |
| SAC deauthorization denial | | | |
| RiskControl pause | | | |
| RiskControl limit trip | | | |
| RecoveryEscrow unauthorized caller | | | |
| Recovery target still authorized | | | |

## Deliverable 2: dApp on Testnet

| Browser flow | Public URL / screenshot | Tx hash / simulation | Post-state proof | Result |
|---|---|---|---|---|
| Connect wallet | | | | |
| Compliance status | | | | |
| Wrap and mint | | | | |
| PT trade | | | | |
| YT trade | | | | |
| Add liquidity | | | | |
| Remove liquidity | | | | |
| Claim yield | | | | |
| Settle/redeem | | | | |
| Transaction history | | | | |
| Invalid-action blocking | | | | |

## Deliverable 3: E2E + Walkthrough

CI run:
Walkthrough:
Reference account:
Run ID:
Screenshots:
Transaction table:
Reviewer reproduction command:
Known limitations:
````

### 15.2 Mainnet acceptance document template

`docs/TRANCHE_3_MAINNET_ACCEPTANCE.md` should use this structure:

````markdown
# Tranche 3 Mainnet Acceptance

Date:
Release tag:
Commit:
Git tree clean:
Public dApp URL:
Dashboard URL:
Mainnet manifest:
RPC URL:
Horizon URL:
Manifest validation output:

## Blocker Disposition

| Blocker from spec section 4.4 | Status | Evidence link |
|---|---|---|

## Deliverable 1: Mainnet Deployment + Production Configuration

| Contract | Address | WASM SHA-256 | On-chain code hash / upload tx | Deploy Tx | Init Tx | Explorer |
|---|---|---|---|---|---|---|

Reproducible build:

Reproducible build evidence:

| Artifact | Local SHA-256 | On-chain/upload hash | Match | Command |
|---|---|---|---|---|

Smoke-test evidence:

| Field | Value |
|---|---|
| Launch account | |
| Raw amount | |
| Formatted amount | |
| Atomic `Router.wrap_and_mint` tx | |
| Explorer link | |
| Pre/post underlying balance | |
| Pre/post SY balance | |
| Pre/post PT balance | |
| Pre/post YT balance | |
| Oracle value/timestamp | |
| RiskControl pre/post volume | |
| Dashboard proof | |

## Deliverable 2: dApp Dashboard + Protocol Monitoring

| Dashboard metric | Contract ID / source | Function + params | Raw value | Display value | Ledger | RPC/indexer coverage | Cache/stale status | Verified |
|---|---|---|---|---|---:|---|---|---|

## Deliverable 3: Mainnet Oracle Integration + Monitoring

Oracle source:
Oracle source ID preimage:
Oracle source ID hash:
Feed contract/API:
Signer/auth verification:
Feed decimals:
Conversion rule:
Last update tx:
Freshness:

Oracle update evidence:

| Source payload hash | Raw value | Scaled value | Source timestamp | On-chain timestamp | Tx | Result |
|---|---:|---:|---|---|---|---|

Blocking evidence:

| Path | Testnet tx / Mainnet simulation | Expected error | Result |
|---|---|---|---|

Alert test:

| Threshold | Channel | Owner | Test timestamp | Payload/screenshot | Resolution |
|---|---|---|---|---|---|

Known limitations:
````

### 15.3 Deployment manifest validation command

Every acceptance document should include:

```bash
scripts/deploy/verify-deployment.sh deployments/<network>/<manifest>.json
```

and the command output should be pasted or linked.

---

## 16. Baseline implementation decisions

The following defaults remove ambiguity for implementation. They may be changed only by an explicit architecture decision record.

| Decision | Baseline |
|---|---|
| Frontend framework | React + Vite + TypeScript |
| Wallet | Freighter first |
| Transaction building | Browser builds and simulates initially; backend builder can be added if needed for reliability |
| Manifest source | Committed JSON manifest loaded by the app at build/deploy time |
| Indexer storage | PostgreSQL for normalized rows, raw event XDR retained |
| E2E frequency | Manual `workflow_dispatch` plus nightly schedule; required before acceptance |
| Testnet maturity model | 90-day public market plus short-lived E2E settlement market |
| Mainnet first market | USDY unless issuer readiness or compliance constraints require a smaller production asset |
| Oracle primary | RedStone USDY/USD SEP-40 for USDY |
| Oracle fallback | Admin-approved fallback; heartbeat-only unless explicitly configured to move value |
| Alerting | Webhook plus email minimum; pager escalation recommended for production |
| Mainnet admin custody | Multisig or hardware-backed signer; no local hot-key admin for production |

Any deviation from these baselines must be documented in `docs/ARCHITECTURE_DECISIONS.md` or an equivalent ADR file and linked from the relevant acceptance document.

---

## 17. Reviewer commands

Reviewers should be able to run:

```bash
mdbook build
cargo test --workspace
cargo build --workspace --release --target wasm32v1-none
scripts/deploy/verify-deployment.sh deployments/testnet/deployment-manifest.json
scripts/deploy/verify-deployment.sh deployments/mainnet/mainnet-deployment-manifest.json
```

For frontend work:

```bash
cd apps/web
npm install
npm run typecheck
npm run test
npm run build
```

For live E2E, the exact command should be recorded in the acceptance document because it depends on secrets and target deployment:

```bash
npm run e2e:testnet
```

---

## 18. Senior auditor review

This section records the main audit concerns found while preparing this specification and the controls added to close them. These are specification-level findings; implementation PRs still require code review, tests, and deployment verification.

| Finding | Risk | Specification control |
|---|---|---|
| Evidence could rely on local terminal output | Reviewer cannot independently verify deliverables | Section 4.3 requires public URLs, transaction hashes, contract reads, source commits, hashes, and explorer/RPC-verifiable evidence |
| Acceptance failure modes were implicit | A feature could appear complete while missing a funded criterion | Section 4.4 defines non-acceptance blocker conditions across deployment, dApp, oracle, dashboard, and alerting |
| Oracle source identity could be ambiguous | Dashboard/provider label may not match on-chain source used for accounting | Section 5.2.2 requires deterministic source IDs with manifest preimages and on-chain `BytesN<32>` hashes |
| `deposit` wording could be interpreted as blocking all SY wrapping | Adding oracle dependency to `SYWrapper` would unnecessarily expand outage blast radius | Section 5.2.5 defines `Router.wrap_and_mint` and direct mint as oracle-blocked while allowing standalone `SYWrapper.deposit` to remain oracle-independent if labeled correctly |
| Existing error numbers could drift | dApp error decoding, tests, and runbooks could break silently | Section 5.2.3 requires append-only errors where possible and migration notes if renumbering occurs |
| Reproducible build evidence could compare only local files | Mainnet WASM verification would not prove deployed code matches source | Sections 5.5, 5.6, and 8.3 require on-chain code hash or upload transaction hash comparison |
| Live Testnet E2E could mutate shared state | Reviewer/demo accounts or the long-lived market could be disrupted by CI | Section 7.3 requires fresh/dedicated accounts, run IDs, account pools, short-market isolation, and dedicated recovery targets |
| Mainnet smoke testing could create avoidable operational risk | Grant evidence could require dangerous production deauthorization/recovery drills | Section 8.6 restricts Mainnet smoke testing to small approved deposit/mint flows and excludes recovery/deauth drills |
| Dashboard metrics could mix live and indexed data silently | Public monitoring could show misleading protocol health | Sections 9.3 and 9.6 require source mapping, ledger timestamps, stale labels, and indexer coverage disclosure |
| Missing pause views could force fragile event reconstruction | Current safety status could be wrong after missed historical events | Section 9.4 requires live read-only pause views where needed |
| Fallback oracle could mask primary failure | Operators and users might see "fresh" while the production feed is down | Section 10.3 requires dashboard/alerts to continue surfacing primary-source failure when fallback is used |
| Mainnet stale-oracle proof could be unsafe | Acceptance testing could intentionally degrade production availability | Section 10.6 requires Mainnet simulation or Testnet drills instead of intentionally staling production oracle |
| Oracle relay decisions could be untraceable | Price updates/rejections could not be audited after the fact | Section 10.2 requires an append-only oracle audit log for every attempted update |
| Dapp transaction architecture could split between browser and backend builders | Engineers could implement two inconsistent signing paths | Sections 6.9 and 14.3 make browser-side build/simulate/sign/submit the Tranche 2 baseline and backend builder optional |
| Wallet and archived-entry failures were underspecified | Users could be pushed into unsafe signing/retry flows | Dapp spec sections 8-9 define XDR hash verification, wallet state handling, and restore-footprint behavior |
| Acceptance templates were thinner than acceptance rules | Reviewer evidence could omit contract reads, screenshots, source hashes, or blocker disposition | Section 15 now requires blocker disposition, raw state proofs, screenshots, transaction hashes, dashboard raw values, oracle payload evidence, and alert ownership |
| RedStone/SEP-40 production verification was too vague | Oracle relay could accept unauthenticated or replayed source data | Section 10.2 now requires exact feed/API, signer/auth verification, decimals, timestamp semantics, replay rejection, source staleness, and conversion rules |
| RiskControl double-reporting was not explicit | Main wrap-and-mint capacity could be misconfigured in production | Sections 5.3, 8.5, and dashboard requirements document double-reporting and effective-capacity labeling |
| PrincipalManager pause status required a live view | Dashboard safety state could depend on fragile event reconstruction | Section 9.4 makes `PrincipalManager.is_paused()` mandatory for Mainnet dashboard acceptance |
| Mainnet smoke test evidence could be ambiguous | Separate deposit/mint calls could pass without proving the funded atomic user path | Sections 8.6, 8.7, and 15.2 require atomic `Router.wrap_and_mint` transaction evidence and post-state reads |

Implementation auditors should reject PRs that weaken these controls without an explicit architecture decision record and matching update to acceptance criteria.

## 19. Implementation order

Recommended sequence:

1. Add manifest schema and registry validation.
2. Extend OracleAdapter and tests.
3. Build deployment scripts and post-deploy verifier.
4. Run fresh Testnet deployment.
5. Build web dApp read-only market state.
6. Add wallet signing and transaction builder.
7. Enable user flows one by one: wrap/mint, trade, liquidity, claim, redeem.
8. Add event indexer and transaction history.
9. Add live Testnet E2E.
10. Publish walkthrough and Tranche 2 acceptance doc.
11. Harden deployment scripts for Mainnet.
12. Integrate RedStone oracle relay and alerts.
13. Deploy Mainnet.
14. Switch dApp to Mainnet and add public dashboard.
15. Publish Mainnet acceptance evidence.

---

## 20. Remaining decisions before coding

These decisions are intentionally narrow and operational. They should be recorded in short architecture decision records before implementation starts:

- Production RPC provider and fallback provider.
- Public hosting provider.
- Final alerting channel and escalation owner.
- Exact RedStone SEP-40 integration package/API.
- Mainnet issuer-admin signing workflow with the USDY issuer.
- Initial production RiskControl limits.
- First market maturity date and launch liquidity plan.
