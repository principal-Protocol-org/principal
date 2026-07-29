# Deployment Guide

## Overview

Eight of the protocol's ten Soroban contracts are implemented. They must be initialized in dependency order because later contracts reference earlier ones by address. (`MarketPool` and `Router` are not yet implemented and aren't part of this guide.)

```
1. OracleAdapter
2. Permissioning
3. RiskControl        (standalone)
4. SYWrapper          (needs: underlying asset address, Permissioning)
5. PrincipalManager   (needs: underlying asset address, SYWrapper, OracleAdapter, Permissioning, RiskControl)
6. PTToken            (needs: Permissioning, underlying asset address, maturity — not yet called by PrincipalManager)
7. YTToken            (needs: Permissioning, underlying asset address, OracleAdapter, maturity — not yet called by PrincipalManager)
8. RecoveryEscrow     (needs: underlying asset address, SYWrapper, PTToken, YTToken)
```

**Every `initialize` call that takes an `admin` parameter (steps 4–7) requires the underlying asset's real, live SAC `admin()` to sign the transaction** — `--source` must be the issuer's own key, not a generic deployer key. This is the market-creation gate: it ties standing up a Principal market to the entity that actually controls the regulated asset's authorization and clawback. `RecoveryEscrow` (step 8) has no admin of its own and can be initialized by anyone, since it only validates that the three position contracts share one underlying and re-derives all real authority live at call time.

`PrincipalManager` does not yet call `SYWrapper`, `PTToken`, or `YTToken` — it still mints/burns PT and YT against its own internal balance maps. Deploying `PTToken`/`YTToken` today gives you correct, tested standalone contracts, but they aren't reachable through `PrincipalManager`'s mint/redeem flow yet. See PROOF_OF_CONCEPT.md's Known Limitations.

## Build WASM artifacts

```bash
cargo build --target wasm32-unknown-unknown --release
```

Artifacts:

| Contract | WASM path |
|---|---|
| OracleAdapter | `target/wasm32-unknown-unknown/release/principal_oracle_adapter.wasm` |
| Permissioning | `target/wasm32-unknown-unknown/release/principal_permissioning.wasm` |
| SYWrapper | `target/wasm32-unknown-unknown/release/principal_sy_wrapper.wasm` |
| PrincipalManager | `target/wasm32-unknown-unknown/release/principal_manager.wasm` |
| RiskControl | `target/wasm32-unknown-unknown/release/principal_risk_control.wasm` |
| PTToken | `target/wasm32-unknown-unknown/release/principal_pt_token.wasm` |
| YTToken | `target/wasm32-unknown-unknown/release/principal_yt_token.wasm` |
| RecoveryEscrow | `target/wasm32-unknown-unknown/release/principal_recovery_escrow.wasm` |

(Or `target/wasm32v1-none/release/`, depending on Rust toolchain version — `wasm32-unknown-unknown` requires Rust ≤ 1.81; newer toolchains require `wasm32v1-none`.)

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
  --wasm target/wasm32-unknown-unknown/release/principal_oracle_adapter.wasm \
  --source admin --network testnet \
  --alias oracle_adapter

stellar contract invoke --id oracle_adapter \
  --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin)
```

### 2. Permissioning

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_permissioning.wasm \
  --source admin --network testnet \
  --alias permissioning

stellar contract invoke --id permissioning \
  --source admin --network testnet \
  -- initialize --admin $(stellar keys address admin)
```

### 3. RiskControl

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_risk_control.wasm \
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
  --wasm target/wasm32-unknown-unknown/release/principal_sy_wrapper.wasm \
  --source admin --network testnet \
  --alias sy_wrapper

stellar contract invoke --id sy_wrapper \
  --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --underlying <USDY_CONTRACT_ID> \
     --permissioning $(stellar contract id alias permissioning --network testnet)
```

### 5. PrincipalManager

Replace `<MATURITY_UNIX_TS>` with the desired maturity Unix timestamp (e.g. `1767225600` for 2026-01-01 00:00 UTC). Same issuer-admin requirement as step 4.

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_manager.wasm \
  --source admin --network testnet \
  --alias principal_manager

stellar contract invoke --id principal_manager \
  --source admin --network testnet \
  -- initialize \
     --admin    $(stellar keys address admin) \
     --underlying    <USDY_CONTRACT_ID> \
     --sy-wrapper    $(stellar contract id alias sy_wrapper --network testnet) \
     --oracle        $(stellar contract id alias oracle_adapter --network testnet) \
     --permissioning $(stellar contract id alias permissioning --network testnet) \
     --maturity <MATURITY_UNIX_TS>
```

### 6. PTToken and YTToken

Standalone, tested contracts — deploying them is optional at this stage since `PrincipalManager` doesn't call them yet (see Overview above). Use the same `<MATURITY_UNIX_TS>` as step 5 if deploying for the same market. Same issuer-admin requirement as step 4.

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_pt_token.wasm \
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

stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_yt_token.wasm \
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

`set_minter` is not called here because no contract currently mints through `PTToken`/`YTToken` — calling it against `PrincipalManager`'s address today would register a minter that never actually calls `mint`/`burn`.

### 7. RecoveryEscrow

No admin of its own — `initialize` just verifies that `SYWrapper`, `PTToken`, and `YTToken` all report the same `underlying_address()` (reverts `PositionUnderlyingMismatch` otherwise), so any key can deploy and initialize it.

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_recovery_escrow.wasm \
  --source admin --network testnet \
  --alias recovery_escrow

stellar contract invoke --id recovery_escrow \
  --source admin --network testnet \
  -- initialize \
     --underlying <USDY_CONTRACT_ID> \
     --sy-wrapper $(stellar contract id alias sy_wrapper --network testnet) \
     --pt-token   $(stellar contract id alias pt_token --network testnet) \
     --yt-token   $(stellar contract id alias yt_token --network testnet)
```

Then wire it into each token contract — one-time, admin-gated, must be signed by the underlying SAC's real admin (same key as steps 4–6):

```bash
stellar contract invoke --id sy_wrapper --source admin --network testnet \
  -- set_recovery_escrow --admin $(stellar keys address admin) \
     --escrow $(stellar contract id alias recovery_escrow --network testnet)

stellar contract invoke --id pt_token --source admin --network testnet \
  -- set_recovery_escrow --admin $(stellar keys address admin) \
     --escrow $(stellar contract id alias recovery_escrow --network testnet)

stellar contract invoke --id yt_token --source admin --network testnet \
  -- set_recovery_escrow --admin $(stellar keys address admin) \
     --escrow $(stellar contract id alias recovery_escrow --network testnet)
```

Finally, give the escrow standing to actually hold seized value: grant it authorization on the underlying SAC (`set_authorized(id=<recovery_escrow>, authorize=true)`, from the issuer) and, if `Permissioning` is enforced for PT/YT, grant it there too.

## Post-deployment checklist

- [ ] Grant admin key to a multisig or hardware key before mainnet.
- [ ] Set a non-zero `cb_limit` on RiskControl appropriate for initial TVL.
- [ ] Register at least one pauser with `risk_control invoke -- add_pauser`.
- [ ] Set the initial USDY reference value on OracleAdapter.
- [ ] Grant at least one test account in Permissioning and confirm `is_allowed` returns `true`.
- [ ] Run a full deposit → mint → redeem cycle on testnet before mainnet.

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

Any registered pauser can pause all protocol operations:

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
