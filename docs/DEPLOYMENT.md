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

Set your network and identity once:

```bash
stellar network add testnet \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015"

stellar keys generate admin --network testnet
stellar keys address admin   # note this address for initialization
```

### 1. OracleAdapter

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_oracle_adapter.wasm \
  --source admin --network testnet \
  --alias oracle_adapter

stellar contract invoke --id oracle_adapter \
  --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin)

# Submit an initial reference value now -- YTToken.initialize (step 5) reads this live as its
# yield-index genesis rate and requires it to be fresh, and PrincipalManager.mint (step 6)
# requires freshness too. Replace 10_000_000 (1.00) and the timestamp with the real current
# rate and Unix time.
stellar contract invoke --id oracle_adapter --source admin --network testnet \
  -- set_reference_value --caller $(stellar keys address admin) \
     --value 10000000 --timestamp $(date +%s)
```

Note: once a value is submitted, every later submission must be `>=` the currently stored value (`set_reference_value` reverts `ValueDecreased` otherwise) — USDY's per-unit value is expected to only appreciate, and the PT/YT settlement math depends on that.

### 2. Permissioning

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_permissioning.wasm \
  --source admin --network testnet \
  --alias permissioning

stellar contract invoke --id permissioning \
  --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin)
```

### 3. RiskControl

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/principal_risk_control.wasm \
  --source admin --network testnet \
  --alias risk_control

# cb_limit = 0 disables the circuit breaker; set to a non-zero value to enable
stellar contract invoke --id risk_control \
  --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin) --cb-limit 0
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
stellar contract invoke --id risk_control --source admin --network testnet \
  -- add_consumer --caller $(stellar keys address admin) \
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
stellar contract invoke --id risk_control --source admin --network testnet \
  -- add_consumer --caller $(stellar keys address admin) \
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

```bash
stellar contract deploy --wasm target/wasm32v1-none/release/principal_router.wasm \
  --source admin --network testnet --alias router
stellar contract invoke --id router --source admin --network testnet -- initialize --admin $(stellar keys address admin)
stellar contract invoke --id router --source admin --network testnet \
  -- register_market --caller $(stellar keys address admin) \
     --pool $(stellar contract id alias market_pool --network testnet)
```

`register_market` cross-checks the pool against its manager and tokens. The Router holds nothing and needs no
standing on the underlying.

### 11. Compliance standing for the protocol contracts

Every contract that ever *holds* SY, PT, YT or the underlying is a participant like any other and needs **both**
compliance layers. Do this for each address below, from the issuer / Permissioning admin:

| Address | Underlying-level authorization | `Permissioning.grant_account` | `grant_asset` for PT **and** YT |
|---|---|---|---|
| `sy_wrapper` (custodies the underlying) | yes | yes | – |
| `principal_manager` (custodies SY) | yes | yes | – |
| `market_pool` (holds PT, SY) | yes | yes | yes |
| `recovery_escrow` (holds seized positions and recovered underlying) | yes | yes | yes |
| fee payees: treasury and the creator | yes | yes | – |
| `router` | **none** | – | – |

```bash
# Stellar asset / SEP-8 (SAC):
stellar contract invoke --id <USDY_CONTRACT_ID> --source admin --network testnet \
  -- set_authorized --id <CONTRACT_ADDRESS> --authorize true
stellar contract invoke --id permissioning --source admin --network testnet \
  -- grant_account --caller $(stellar keys address admin) --account <CONTRACT_ADDRESS>
stellar contract invoke --id permissioning --source admin --network testnet \
  -- grant_asset --caller $(stellar keys address admin) --account <CONTRACT_ADDRESS> \
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

Only the admin can unpause:

```bash
stellar contract invoke --id risk_control \
  --source admin --network testnet \
  -- unpause --caller $(stellar keys address admin)
```

## Mainnet

Same steps as testnet with:

```bash
--network mainnet
--rpc-url https://mainnet.sorobanrpc.com
--network-passphrase "Public Global Stellar Network ; September 2015"
```

Ensure the WASM hash is verified onchain before granting admin authority.
