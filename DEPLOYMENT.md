# Deployment Guide

## Overview

The protocol deploys five independent Soroban contracts. They must be initialized in dependency order because later contracts reference earlier ones by address.

```
1. OracleAdapter
2. Permissioning
3. SYWrapper          (needs: underlying asset address)
4. RiskControl        (standalone)
5. PrincipalManager   (needs: SYWrapper, OracleAdapter, Permissioning, RiskControl)
```

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

Replace `<USDY_CONTRACT_ID>` with the USDY Stellar Asset Contract address on testnet.

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_sy_wrapper.wasm \
  --source admin --network testnet \
  --alias sy_wrapper

stellar contract invoke --id sy_wrapper \
  --source admin --network testnet \
  -- initialize \
     --admin $(stellar keys address admin) \
     --underlying <USDY_CONTRACT_ID>
```

### 5. PrincipalManager

Replace `<MATURITY_UNIX_TS>` with the desired maturity Unix timestamp (e.g. `1767225600` for 2026-01-01 00:00 UTC).

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/principal_manager.wasm \
  --source admin --network testnet \
  --alias principal_manager

stellar contract invoke --id principal_manager \
  --source admin --network testnet \
  -- initialize \
     --admin    $(stellar keys address admin) \
     --sy-wrapper    $(stellar contract id alias sy_wrapper --network testnet) \
     --oracle        $(stellar contract id alias oracle_adapter --network testnet) \
     --permissioning $(stellar contract id alias permissioning --network testnet) \
     --maturity <MATURITY_UNIX_TS>
```

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
