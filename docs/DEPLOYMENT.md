# Deployment Guide

## Overview

All eleven of the protocol's Soroban contracts are implemented. They are initialized in dependency order
because later contracts reference earlier ones by address. This guide is the CLI form of
TECHNICAL_SPECIFICATION.md §17, which `contracts/integration_tests/src/stack.rs` executes verbatim in tests.

```
 1. OracleAdapter
 2. Permissioning
 3. RiskControl
 4. SYWrapper          (underlying, Permissioning)            + wire RiskControl
 5. PTToken, YTToken   (Permissioning, underlying, maturity; YTToken also OracleAdapter)
 6. MarketConfig       (underlying, maturity, fees, protocol admin, treasury)
 7. PrincipalManager   (SYWrapper, PT, YT, Oracle, Permissioning, underlying, maturity, MarketConfig) + wire RiskControl, set_minter
 8. MarketPool         (PrincipalManager, time stretch)
 9. RecoveryEscrow     (underlying, SYWrapper, PT, YT, PrincipalManager, MarketPool)  + set_recovery_escrow ×4
10. Router             (admin)                                 + register_market(pool)
11. Compliance standing for every protocol contract that holds value
```

**Every `initialize` that takes an `admin` (steps 4–8) requires the underlying's issuer authority to sign the
transaction** — for a Stellar asset the SAC's live `admin()`, for a SEP-57 token an account holding the
operator role (`--source` must be that key, not a generic deployer key). This is the market-creation gate. `RecoveryEscrow` has no admin of its own
and can be initialized by anyone, since it only validates that its contracts share one underlying and
re-derives all authority live at call time. The `Router` holds nothing and needs no compliance standing.

### Key roles: `ISSUER_KEY` vs `ADMIN_KEY`

Found worth separating during a dApp-integration review: the examples below (and
`scripts/create_market_via_factory.sh` before this review) use one key, `admin`, for everything —
convenient for a single deployer, but it means anything that needs *any* admin power on this
protocol (e.g. a faucet that needs to authorize new users' trustlines) ends up needing full
control over every contract, since they're the same key. Two distinct roles exist in the contracts
already; this guide now names them separately.

- **`ISSUER_KEY`** — the underlying asset's real, live issuer authority (for a Stellar SAC,
  `admin()`; for SEP-57, the operator role). **Hard contract requirement, not a script
  convention**: `SYWrapper`, `PTToken`, `YTToken`, `MarketConfig`, `PrincipalManager` and
  `MarketPool` all check their `admin` argument against the underlying's real issuer at
  `initialize` and revert `IssuerMismatch` otherwise (`principal_compliance::is_authority`) — a
  deliberate guard against a lower-trust admin key fabricating a market for an asset it does not
  actually issue. This means **market creation itself cannot be delegated away from the real
  issuer** — steps 4–9's `initialize`/`set_recovery_escrow` calls, and `MarketFactory`'s
  `create_tokens`/`create_engine`/`create_pool_and_escrow` (which just relay `issuer` into these
  same six contracts), all need `ISSUER_KEY`. `MarketConfig.set_fees` also checks the issuer live
  (`assert_issuer`), not `protocol_admin` — fee tuning stays an issuer action too. `ISSUER_KEY` is
  also the only key that can ever call the underlying SAC's own `set_authorized` (step 11) — a
  protocol-level Stellar restriction that has nothing to do with this protocol's own admin model.
- **`ADMIN_KEY`** — a separate, non-issuer key for every admin action that does *not* carry an
  `IssuerMismatch` check: `OracleAdapter` (steps 1, plus `set_sources`/`set_reference_value`/
  `set_safety_params`/`set_value_bounds` afterward), `Permissioning` (step 2, plus
  `grant_account`/`grant_asset`/`revoke_*`), `RiskControl` (step 3, plus `add_consumer`/
  `set_cb_limit`/pausing), `Router` (step 10, plus `register_market`/`transfer_admin`), and
  `MarketConfig`'s `protocol_admin`-gated calls (`set_protocol_share`, `set_treasury`,
  `transfer_protocol_admin` — distinct from `set_fees`, above). None of these ever check against
  the underlying's issuer, so `ADMIN_KEY` can safely be a different account, rotated independently,
  without touching market creation or compliance at all.

**What this does and does not fix.** A faucet that only needs to *fund* test accounts with the
underlying needs neither key — a funded account with a trustline is enough. A faucet that also
needs to *authorize new trustlines* (true whenever the asset is `AUTH_REQUIRED`, as this protocol's
Testnet deployments are) genuinely needs `ISSUER_KEY` for that one call, and — because market
creation is also gated on the same real issuer, by design, not by choice of script — handing a
tool `ISSUER_KEY` for trustline authorization still leaves it *able* to create markets too, as an
inherent property of the Stellar asset model, not something `ADMIN_KEY`'s introduction removes.
The mitigation available today is operational (scope what the faucet tool's own code ever calls,
monitor its key's activity), not cryptographic. Fully separating "can authorize trustlines" from
"can create markets" would need a contract change — a `market_creator` parameter checked
independently of the SAC issuer in the six `IssuerMismatch`-gated contracts — which is a larger,
deliberate change to this protocol's trust model and is **not** made here; flagged as a follow-up
if the dApp team's faucet design needs it.

`PrincipalManager` mints and burns real SEP-41 PT/YT, which is why `PTToken`/`YTToken` (step 5) are deployed
*before* it and given their minter afterwards, and why the protocol contracts need the standing in step 11.

## Build WASM artifacts

```bash
cargo build --workspace --release --target wasm32v1-none
```

Artifacts:

| Contract | WASM path |
|---|---|
| OracleAdapter | `target/wasm32v1-none/release/principal_oracle_adapter.wasm` |
| Permissioning | `target/wasm32v1-none/release/principal_permissioning.wasm` |
| SYWrapper | `target/wasm32v1-none/release/principal_sy_wrapper.wasm` |
| PrincipalManager | `target/wasm32v1-none/release/principal_manager.wasm` |
| RiskControl | `target/wasm32v1-none/release/principal_risk_control.wasm` |
| PTToken | `target/wasm32v1-none/release/principal_pt_token.wasm` |
| YTToken | `target/wasm32v1-none/release/principal_yt_token.wasm` |
| RecoveryEscrow | `target/wasm32v1-none/release/principal_recovery_escrow.wasm` |
| MarketConfig | `target/wasm32v1-none/release/principal_market_config.wasm` |
| MarketPool | `target/wasm32v1-none/release/principal_market_pool.wasm` |
| Router | `target/wasm32v1-none/release/principal_router.wasm` |

`wasm32-unknown-unknown` no longer builds with Soroban SDK 26 on current Rust; use `wasm32v1-none`
(`rustup target add wasm32v1-none`). Every artifact is under the 128 KiB limit.

## Testnet deployment

Set your network and both identities once (see "Key roles" above — `admin` below is `ISSUER_KEY`;
a separate `protocol_admin` identity is `ADMIN_KEY`, used for steps 1, 2, 3 and 10):

```bash
stellar network add testnet \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015"

stellar keys generate admin --network testnet          # ISSUER_KEY — must be the underlying's real issuer
stellar keys generate protocol_admin --network testnet # ADMIN_KEY — anything else; never checked against the issuer
stellar keys address admin           # note this address for initialization
stellar keys address protocol_admin  # note this address too
```

### 1. OracleAdapter

Uses `ADMIN_KEY` (`protocol_admin` below) — `initialize` takes an arbitrary admin, no issuer check.

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_oracle_adapter.wasm \
  --source protocol_admin --network testnet \
  --alias oracle_adapter

stellar contract invoke --id oracle_adapter \
  --source protocol_admin --network testnet \
  -- initialize --admin $(stellar keys address protocol_admin)

# Submit an initial reference value now -- YTToken.initialize (step 5) reads this live as its
# yield-index genesis rate and requires it to be fresh, and PrincipalManager.mint (step 6)
# requires freshness too. Replace 10_000_000 (1.00) and the timestamp with the real current
# rate and Unix time.
stellar contract invoke --id oracle_adapter --source protocol_admin --network testnet \
  -- set_reference_value --caller $(stellar keys address protocol_admin) \
     --value 10000000 --timestamp $(date +%s)
```

Note: once a value is submitted, every later submission must be `>=` the currently stored value (`set_reference_value` reverts `ValueDecreased` otherwise) — USDY's per-unit value is expected to only appreciate, and the PT/YT settlement math depends on that.

### 2. Permissioning

Uses `ADMIN_KEY` — `initialize` takes an arbitrary admin, no issuer check. `grant_account`/
`grant_asset`/`revoke_*` in step 11 below use this same key, not the issuer.

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_permissioning.wasm \
  --source protocol_admin --network testnet \
  --alias permissioning

stellar contract invoke --id permissioning \
  --source protocol_admin --network testnet \
  -- initialize --admin $(stellar keys address protocol_admin)
```

### 3. RiskControl

Uses `ADMIN_KEY` — `initialize` takes an arbitrary admin, no issuer check. Its own `add_consumer`
(steps 4 and 7 below) uses this same key too, even though those steps are otherwise issuer-signed
for the contract being wired *into* RiskControl.

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_risk_control.wasm \
  --source protocol_admin --network testnet \
  --alias risk_control

# cb_limit = 0 disables the circuit breaker; set to a non-zero value to enable
stellar contract invoke --id risk_control \
  --source protocol_admin --network testnet \
  -- initialize --admin $(stellar keys address protocol_admin) --cb-limit 0
```

### 4. SYWrapper

Replace `<USDY_CONTRACT_ID>` with the USDY Stellar Asset Contract address on testnet. **`--source` and `--admin` must be the underlying SAC's real, live admin key** — `initialize` reads `underlying.admin()` and reverts `IssuerMismatch` if it doesn't match, and also requires that admin's `require_auth()`.

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_sy_wrapper.wasm \
  --source admin --network testnet \
  --alias sy_wrapper

stellar contract invoke --id sy_wrapper \
  --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --underlying <USDY_CONTRACT_ID> \
     --permissioning $(stellar contract id alias permissioning --network testnet)
```

Wire the circuit breaker (after RiskControl exists) and, if wanted, a per-address cap:

```bash
stellar contract invoke --id sy_wrapper --source admin --network testnet \
  -- set_risk_control --admin $(stellar keys address admin) \
     --risk_control $(stellar contract id alias risk_control --network testnet)
stellar contract invoke --id risk_control --source protocol_admin --network testnet \
  -- add_consumer --caller $(stellar keys address protocol_admin) \
     --consumer $(stellar contract id alias sy_wrapper --network testnet)
```

### 5. PTToken and YTToken

Replace `<MATURITY_UNIX_TS>` with the desired maturity Unix timestamp (e.g. `1767225600` for 2026-01-01 00:00 UTC). These must be deployed *before* `PrincipalManager`, which now requires both addresses at initialization. Same issuer-admin requirement as step 4. `YTToken.initialize` additionally requires the oracle (step 1) to already have a fresh reference value set — it reads that value live as its yield-index genesis rate rather than hardcoding it, and reverts `OracleStale` if the oracle isn't fresh at this moment.

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_pt_token.wasm \
  --source admin --network testnet \
  --alias pt_token

stellar contract invoke --id pt_token \
  --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --permissioning $(stellar contract id alias permissioning --network testnet) \
     --underlying <USDY_CONTRACT_ID> \
     --maturity <MATURITY_UNIX_TS> \
     --name "Principal Token USDY" --symbol "PT-USDY" --decimals 7
# (no minter yet -- two-phase init; set_minter is called in step 6 below)

stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_yt_token.wasm \
  --source admin --network testnet \
  --alias yt_token

stellar contract invoke --id yt_token \
  --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --permissioning $(stellar contract id alias permissioning --network testnet) \
     --underlying <USDY_CONTRACT_ID> \
     --oracle $(stellar contract id alias oracle_adapter --network testnet) \
     --maturity <MATURITY_UNIX_TS> \
     --name "Yield Token USDY" --symbol "YT-USDY" --decimals 7
```

### 6. MarketConfig

Per-market fees and split. The issuer authority signs `initialize`; fees are capped (tokenization ≤ 100 bps,
YT ≤ 5 000 bps, swap tier ≤ 500 bps). The example values below are 5 bps, 10 %, 0.1 %, and a 20 % protocol
share.

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_market_config.wasm \
  --source admin --network testnet --alias market_config

stellar contract invoke --id market_config --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --underlying <USDY_CONTRACT_ID> \
     --maturity <MATURITY_UNIX_TS> \
     --protocol_admin <PRINCIPAL_ADMIN_ADDRESS> \
     --treasury <PRINCIPAL_TREASURY_ADDRESS> \
     --tokenization_fee_bps 5 --yt_fee_bps 1000 --swap_fee_tier_bps 10 \
     --protocol_share_bps 2000
```

The issuer retunes fees later with `set_fees`; Principal moves its share/treasury with `set_protocol_share` /
`set_treasury`. For a SEP-57 underlying the creator's 80 % goes to an explicit payee (`set_creator_payee`).

### 7. PrincipalManager

Same issuer-authority requirement as step 4. `initialize` cross-checks that `sy_wrapper`/`pt_token`/`yt_token`
report the same `underlying_address()` and `permissioning_address()`, that `pt_token`/`yt_token`/`market_config`
report the same `maturity()`, that `market_config.underlying()` matches, and that `yt_token.oracle_address()`
matches `--oracle` — it reverts `TopologyMismatch` if the earlier steps used inconsistent values.

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_manager.wasm \
  --source admin --network testnet --alias principal_manager

stellar contract invoke --id principal_manager --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --sy_wrapper    $(stellar contract id alias sy_wrapper --network testnet) \
     --pt_token      $(stellar contract id alias pt_token --network testnet) \
     --yt_token      $(stellar contract id alias yt_token --network testnet) \
     --oracle        $(stellar contract id alias oracle_adapter --network testnet) \
     --permissioning $(stellar contract id alias permissioning --network testnet) \
     --underlying    <USDY_CONTRACT_ID> \
     --maturity      <MATURITY_UNIX_TS> \
     --market_config $(stellar contract id alias market_config --network testnet)

# Register PrincipalManager as the minter on both tokens (one-time):
for t in pt_token yt_token; do
  stellar contract invoke --id $t --source admin --network testnet \
    -- set_minter --admin $(stellar keys address admin) \
       --minter $(stellar contract id alias principal_manager --network testnet)
done

# Wire the circuit breaker: PrincipalManager reports every mint to RiskControl, and must be a consumer.
stellar contract invoke --id principal_manager --source admin --network testnet \
  -- set_risk_control --admin $(stellar keys address admin) \
     --risk_control $(stellar contract id alias risk_control --network testnet)
stellar contract invoke --id risk_control --source protocol_admin --network testnet \
  -- add_consumer --caller $(stellar keys address protocol_admin) \
     --consumer $(stellar contract id alias principal_manager --network testnet)
```

(Do the same `set_risk_control` + `add_consumer` for `sy_wrapper` right after step 4. A wired wrapper or
manager that is *not* a registered consumer reverts every deposit/mint — the breaker fails closed.)

### 8. MarketPool

The issuer authority signs `initialize`. Everything else is read from the manager, so the pool cannot be wired
to mismatched contracts. `--time_stretch_years` (1–20) sets how gently the curve flattens; the market's
remaining life must be under 75 % of it (a 4-year stretch supports maturities up to 3 years).

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_market_pool.wasm \
  --source admin --network testnet --alias market_pool

stellar contract invoke --id market_pool --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin) \
     --principal_manager $(stellar contract id alias principal_manager --network testnet) \
     --time_stretch_years 4
```

### 9. RecoveryEscrow

No admin of its own — `initialize` verifies that `SYWrapper`, `PTToken`, `YTToken`, `PrincipalManager` and
`MarketPool` all report the same `underlying_address()` (`PositionUnderlyingMismatch` otherwise). Deploy it after
step 8.

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_recovery_escrow.wasm \
  --source admin --network testnet --alias recovery_escrow

stellar contract invoke --id recovery_escrow --source admin --network testnet \
  -- initialize --underlying <USDY_CONTRACT_ID> \
     --sy_wrapper        $(stellar contract id alias sy_wrapper --network testnet) \
     --pt_token          $(stellar contract id alias pt_token --network testnet) \
     --yt_token          $(stellar contract id alias yt_token --network testnet) \
     --principal_manager $(stellar contract id alias principal_manager --network testnet) \
     --market_pool       $(stellar contract id alias market_pool --network testnet)

# Wire it into each position contract — one-time, admin-gated, issuer-signed:
for c in sy_wrapper pt_token yt_token market_pool; do
  stellar contract invoke --id $c --source admin --network testnet \
    -- set_recovery_escrow --admin $(stellar keys address admin) \
       --escrow $(stellar contract id alias recovery_escrow --network testnet)
done
```

### 10. Router

Uses `ADMIN_KEY` — `initialize` takes an arbitrary admin, no issuer check.

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_router.wasm \
  --source protocol_admin --network testnet --alias router
stellar contract invoke --id router --source protocol_admin --network testnet -- initialize --admin $(stellar keys address protocol_admin)
stellar contract invoke --id router --source protocol_admin --network testnet \
  -- register_market --caller $(stellar keys address protocol_admin) \
     --pool $(stellar contract id alias market_pool --network testnet)
```

`register_market` cross-checks the pool against its manager and tokens. The Router holds nothing and needs no
standing on the underlying.

### 11. Compliance standing for the protocol contracts

Every contract that ever *holds* SY, PT, YT or the underlying is a participant like any other and needs **both**
compliance layers. Underlying-level authorization (`set_authorized`) is `ISSUER_KEY` only — the one Stellar
protocol-level restriction nothing here can delegate around. `Permissioning.grant_account`/`grant_asset` use
`ADMIN_KEY` (`protocol_admin`), since Permissioning was initialized with that key in step 2, not the issuer.

| Address | Underlying-level authorization | `Permissioning.grant_account` | `grant_asset` for PT **and** YT |
|---|---|---|---|
| `sy_wrapper` (custodies the underlying) | yes | yes | – |
| `principal_manager` (custodies SY) | yes | yes | – |
| `market_pool` (holds PT, SY) | yes | yes | yes |
| `recovery_escrow` (holds seized positions and recovered underlying) | yes | yes | yes |
| fee payees: treasury and the creator | yes | yes | – |
| `router` | **none** | – | – |

```bash
# Stellar asset / SEP-8 (SAC) -- ISSUER_KEY, no substitute:
stellar contract invoke --id <USDY_CONTRACT_ID> --source admin --network testnet \
  -- set_authorized --id <CONTRACT_ADDRESS> --authorize true
# Permissioning -- ADMIN_KEY:
stellar contract invoke --id permissioning --source protocol_admin --network testnet \
  -- grant_account --caller $(stellar keys address protocol_admin) --account <CONTRACT_ADDRESS>
stellar contract invoke --id permissioning --source protocol_admin --network testnet \
  -- grant_asset --caller $(stellar keys address protocol_admin) --account <CONTRACT_ADDRESS> \
     --asset $(stellar contract id alias pt_token --network testnet)      # and again for yt_token
```

**SEP-8 issuers:** the standing above must be *persistent*. The SEP-8 authorize→pay→deauthorize sandwich
cannot wrap a single-operation Soroban call, so a holder (or protocol contract) that is authorized only inside
an approved classic transaction is unauthorized to Principal and every operation reverts `NotAuthorizedOnSac`.
**SEP-57 issuers:** give each address above a verified identity with the token's `IdentityVerifier` (and leave
it unfrozen) instead of `set_authorized`; without it the first deposit reverts inside the token's own transfer.
Details: [COMPLIANCE_ARCHITECTURE.md](COMPLIANCE_ARCHITECTURE.md).

## Lessons from the 4 October 2026 Testnet run

The run is recorded in [TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md](TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md), section 6. Three points change the procedure above.

- Escrow wiring is one-time. `set_recovery_escrow` on SYWrapper, PT, YT and MarketPool reverts `RecoveryEscrowAlreadySet` once set, and PrincipalManager stores its SYWrapper address at initialization. Choose the escrow before first use. A replacement escrow means a new SYWrapper and a full redeploy of every dependent contract.
- Choose maturity with the whole run in mind. Maturity is fixed at initialization for PT, YT, MarketConfig and PrincipalManager. Each setup step takes tens of seconds on Testnet, so allow well over an hour between the first deployment and the first mint.
- Timestamps come from the ledger. The oracle rejects a timestamp later than the ledger's close time. Use the latest ledger close time minus a few seconds, not the local clock.

## Post-deployment checklist

- [ ] Grant admin key to a multisig or hardware key before mainnet.
- [ ] Set a non-zero `cb_limit` (and, if wanted, `set_asset_limit` for the underlying) on RiskControl appropriate for initial TVL; confirm the window (`get_window_ledgers`, default 17 280 ≈ 24 h).
- [ ] Register at least one pauser with `risk_control invoke -- add_pauser`.
- [ ] Confirm `SYWrapper` and `PrincipalManager` are registered consumers (`is_consumer`) **and** wired (`risk_control()` returns the RiskControl) — otherwise deposits/mints revert.
- [ ] Set the per-address deposit cap on `SYWrapper` if the market wants one (`set_deposit_cap`).
- [ ] Set the initial reference value on OracleAdapter and confirm a relay keeps it fresh (< 3 600 s).
- [ ] Confirm fees on `MarketConfig` and that the treasury and creator payees are compliant (fee claims transfer SY to them).
- [ ] Give every protocol contract its compliance standing (step 11) and grant at least one test account.
- [ ] Run a full deposit → mint → swap → redeem cycle on testnet; call `settle_all()` at maturity.
- [ ] Rehearse a recovery: deauthorize a test account, `seize_all_positions`, `finalize_record`.

## Key rotation

Admin keys can be rotated on any contract with `transfer_admin`:

```bash
stellar contract invoke --id <CONTRACT_ID> \
  --source current_admin --network testnet \
  -- transfer_admin \
     --current-admin $(stellar keys address current_admin) \
     --new-admin <NEW_ADMIN_ADDRESS>
```

## Emergency pause

Any registered pauser can trip the global pause, which blocks every `SYWrapper.deposit` and `PrincipalManager.mint` (they call RiskControl). Each of `SYWrapper`, `PrincipalManager` and `MarketPool` also has its own admin `set_paused`. Recovery (`RecoveryEscrow`) keeps working while paused.

```bash
stellar contract invoke --id risk_control \
  --source pauser_key --network testnet \
  -- pause --caller $(stellar keys address pauser_key)
```

Only the admin can unpause — RiskControl's own admin, `ADMIN_KEY`, not the issuer:

```bash
stellar contract invoke --id risk_control \
  --source protocol_admin --network testnet \
  -- unpause --caller $(stellar keys address protocol_admin)
```

## Testnet manifest for `principal-dApp`

[`testnet.json`](../testnet.json) (repo root) collects the current Testnet addresses and wasm
hashes a dApp integration needs: the shared infrastructure, all 12 contracts' wasm hashes, the
Router's live function list, the test asset, and every current market's full address set.

**Status as of 2026-11-12: schema confirmed, no longer a guess.** It now conforms to
`principal-dApp`'s real `manifestSchema` (`schemaVersion: 2`), fetched directly from
`principal-Protocol-org/dapp`'s `testnet` branch at `lib/protocol/manifest.ts` (that repo's own
`config/manifests/testnet.json` was read too, as a shape reference). Checked against the schema's
actual regexes and `validateManifest`'s cross-field rules (every market's `contracts.router`/
`contracts.oracle` must equal `shared.router`/`shared.oracle`; no duplicate `marketId`; etc.), not
just eyeballed. Two consequences of that cross-field rule, worth noting:

- The file lists only the one market currently registered on the current shared `Router`
  instance (`principal-testnet-2026-11-12-m6`, ~100 days to maturity — comfortably past the
  dApp team's "3+ months" ask). The schema has no concept of a market belonging to a *different*,
  now-superseded shared `Router` instance (a new `Router` instance is deployed every time its own
  wasm changes, since Soroban code can't be upgraded in place) — including one here with a
  mismatched `contracts.router` would fail `validateManifest` outright, not just look stale. Prior
  rounds' now-matured markets (on their own, now-superseded Router instances) are documented in
  `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §9–§11 instead of carried in this file.
- `sourceCommit` is now `9509529693f630cb269ea1992249123642774ca9`, the commit that actually
  contains the fixes deployed in `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §11 (updated
  after that commit landed, since `testnet.json` was necessarily written against the working tree
  just before it).

**Known to be stale as of commit `5a6727e` (not yet redeployed)**: that commit added four
read-only quote functions (`MarketPool.quote_add_liquidity_single_sy`/`quote_lp_exit`,
`Router.quote_add_liquidity_underlying`/`quote_lp_to_underlying`) closing the last open
dApp-reported gap ("no quote for the two new USDY LP functions") — see
`docs/DAPP_SMART_CONTRACT_INTEGRATION_GUIDE.md` §8's closing addendum. `MarketPool`'s and
`Router`'s wasm both changed again, so `testnet.json`'s `wasmHashes.pool`/`wasmHashes.router`
(and Market 6's own `pool`/`router` addresses, still on the pre-this-commit wasm) do not yet
reflect it. Not redeployed in the same pass deliberately: both additions are pure, already
exact-equality-tested read-only views with no auth-tree or cross-call complexity (unlike the
auth-drift and compliance bugs earlier rounds needed live Testnet proof for specifically because
a Rust test cannot reproduce Soroban's own two-phase simulate/sign/submit behavior) — a live
redeploy would add infrastructure churn without adding assurance beyond what the exact-equality
regression test (which did catch a real bug in `quote_lp_exit`'s first draft) already gives.
Fold into the next redeployment that happens for another reason, rather than redeploying solely
for this.

Regenerate or hand-edit this file after any redeployment; nothing currently automates keeping it
in sync.

## Mainnet

Same steps as testnet with:

```bash
--network mainnet
--rpc-url https://mainnet.sorobanrpc.com
--network-passphrase "Public Global Stellar Network ; September 2015"
```

Ensure the WASM hash is verified onchain before granting admin authority.
