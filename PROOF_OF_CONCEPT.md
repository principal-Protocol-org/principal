# Principal Protocol — Proof of Concept

This document describes the current state of the Principal Protocol implementation: five Soroban smart contracts that form the foundational layer of the protocol and demonstrate the core yield-tokenization mechanics on Stellar.

---

## Scope

The POC implements the entire infrastructure and tokenization layer of the protocol:

| Contract | Crate | Status |
|---|---|---|
| `OracleAdapter` | `principal_oracle_adapter` | Complete |
| `Permissioning` | `principal_permissioning` | Complete |
| `RiskControl` | `principal_risk_control` | Complete |
| `SYWrapper` | `principal_sy_wrapper` | Complete |
| `PrincipalManager` | `principal_manager` | Complete |

The market layer (`PTToken`, `YTToken`, `MarketPool`, `Router`) is the Phase 2 scope and is not part of this POC. PT and YT balances are tracked internally within `PrincipalManager` rather than as standalone SEP-41 tokens.

---

## What the POC Demonstrates

### 1. Standardized yield wrapping

`SYWrapper` accepts any Stellar yield-bearing asset, holds it, and issues SY shares at a rolling exchange rate. As yield accrues in the underlying, the exchange rate grows, so each SY share is worth progressively more underlying over time. This is the foundational accounting primitive for the entire protocol.

### 2. Principal and yield tokenization

`PrincipalManager` accepts SY shares and splits them into equal PT and YT amounts based on the oracle reference rate at the time of minting. This demonstrates the core economic mechanism: a single position with variable yield is separated into a fixed principal claim and a future yield claim.

### 3. Deterministic maturity settlement

At or after the maturity timestamp, `PrincipalManager.redeem()` computes and distributes USDY to PT and YT holders using the final oracle rate. The settlement formula uses fixed-point arithmetic with floor rounding, ensuring deterministic and auditable outcomes.

### 4. Oracle integration

`OracleAdapter` stores an admin-submitted USDY/USDC reference value with monotonic timestamp enforcement and freshness checks. This is the trust anchor for minting and redemption pricing.

### 5. Permissioned compliance flow

`Permissioning` enforces per-account and per-asset eligibility, checked by `PrincipalManager` on every mint and redemption. This preserves the compliance constraints of the underlying USDY asset across all derived instruments.

### 6. Risk controls

`RiskControl` implements a global pause with multi-pauser roles and a rolling 24-hour circuit breaker on deposit volume. These are independent safety layers that can halt the protocol in response to oracle failures, market anomalies, or operational incidents.

---

## Contract Interfaces

### OracleAdapter

```rust
fn initialize(env: Env, admin: Address)
fn set_reference_value(env: Env, caller: Address, value: i128, timestamp: u64)
fn get_reference_value(env: Env) -> i128
fn get_reference_timestamp(env: Env) -> u64
fn is_fresh(env: Env, max_stale_seconds: u64) -> bool
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

`value` is scaled by `RATE_SCALE = 10_000_000`. A value of `10_300_000` represents 1.03 USDC per USDY.

### Permissioning

```rust
fn initialize(env: Env, admin: Address)
fn grant_account(env: Env, caller: Address, account: Address)
fn revoke_account(env: Env, caller: Address, account: Address)
fn grant_accounts(env: Env, caller: Address, accounts: Vec<Address>)
fn grant_asset(env: Env, caller: Address, account: Address, asset: Address)
fn revoke_asset(env: Env, caller: Address, account: Address, asset: Address)
fn is_allowed(env: Env, account: Address) -> bool
fn is_allowed_for_asset(env: Env, account: Address, asset: Address) -> bool
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

### SYWrapper

```rust
fn initialize(env: Env, admin: Address, underlying: Address)
fn deposit(env: Env, from: Address, amount: i128) -> i128       // returns shares minted
fn withdraw(env: Env, from: Address, shares: i128, to: Address) -> i128  // returns underlying
fn exchange_rate(env: Env) -> i128                               // scaled ×10⁷
fn total_underlying(env: Env) -> i128
fn total_shares(env: Env) -> i128
fn balance_of(env: Env, account: Address) -> i128
fn underlying_address(env: Env) -> Address
fn set_paused(env: Env, caller: Address, paused: bool)
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

### PrincipalManager

```rust
fn initialize(env: Env, admin: Address, sy_wrapper: Address,
              oracle: Address, permissioning: Address, maturity: u64)
fn mint(env: Env, from: Address, sy_shares: i128) -> MintResult
fn redeem(env: Env, from: Address, pt_amount: i128, yt_amount: i128) -> RedeemResult
fn pt_balance(env: Env, account: Address) -> i128
fn yt_balance(env: Env, account: Address) -> i128
fn total_pt(env: Env) -> i128
fn total_yt(env: Env) -> i128
fn maturity(env: Env) -> u64
fn is_mature(env: Env) -> bool
fn set_paused(env: Env, caller: Address, paused: bool)
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address

// Return types
struct MintResult   { pt_minted: i128, yt_minted: i128 }
struct RedeemResult { underlying_from_pt: i128, underlying_from_yt: i128 }
```

### RiskControl

```rust
fn initialize(env: Env, admin: Address, cb_limit: i128)
fn pause(env: Env, caller: Address)
fn unpause(env: Env, caller: Address)
fn is_paused(env: Env) -> bool
fn add_pauser(env: Env, caller: Address, pauser: Address)
fn remove_pauser(env: Env, caller: Address, pauser: Address)
fn check_deposit(env: Env, amount: i128)
fn set_cb_limit(env: Env, caller: Address, new_limit: i128)
fn get_cb_limit(env: Env) -> i128
fn get_cb_volume(env: Env) -> i128
fn transfer_admin(env: Env, current_admin: Address, new_admin: Address)
fn get_admin(env: Env) -> Address
```

---

## Settlement Formula

All arithmetic uses `i128` with `SCALE = 10_000_000` (10^7). `RATE_SCALE` is an alias for the same value. Oracle rates are stored in these same units: 1.03 USDC per underlying = `10_300_000`.

Mint (stores `initial_rate` per user for later settlement):
```
initial_rate          = OracleAdapter.get_reference_value()  // e.g. 10_300_000
notional              = sy_shares * initial_rate / SCALE
PT_minted             = notional
YT_minted             = notional
initial_rate_s[user]  = initial_rate                         // stored for YT settlement
```

Redeem (at maturity, given `final_rate` from oracle and per-user `initial_rate`):
```
// PT: redeem principal USDC value → convert to underlying at final rate
usdy_from_pt = floor(pt_amount * SCALE / final_rate)

// YT: redeem yield accrued above initial rate → convert to underlying at final rate
yield_delta  = max(0, final_rate - initial_rate_s[user])
usdy_from_yt = floor(yt_amount * yield_delta / final_rate)
```

Using `initial_rate` (not `SCALE`) in `yield_delta` ensures YT captures only yield accrued since this user's mint, regardless of what the oracle rate was at protocol inception.

---

## Constants

| Constant | Value | Contract | Meaning |
|---|---|---|---|
| `SCALE` | `10_000_000` | All contracts | Universal fixed-point denominator (10^7). `RATE_SCALE` is a deprecated alias for the same value. |
| `ELIGIBILITY_TTL_LEDGERS` | `518_400` | Permissioning | ~30 days at 5 s/ledger |
| `CB_WINDOW_SECS` | `86_400` | RiskControl | 24-hour circuit breaker window |
| `MAX_ORACLE_STALENESS_SECS` | `3_600` | PrincipalManager | 1-hour freshness at redemption |

---

## Error Codes

| Contract | Code | Error | Trigger |
|---|---|---|---|
| OracleAdapter | 1 | `AlreadyInitialized` | `initialize` called twice |
| OracleAdapter | 2 | `Unauthorized` | caller ≠ admin |
| OracleAdapter | 3 | `InvalidValue` | value ≤ 0 |
| OracleAdapter | 4 | `TimestampTooOld` | new timestamp ≤ stored |
| OracleAdapter | 5 | `NotInitialized` | read before initialize |
| Permissioning | 1 | `AlreadyInitialized` | initialize called twice |
| Permissioning | 2 | `Unauthorized` | caller ≠ admin |
| Permissioning | 3 | `NotInitialized` | read before initialize |
| SYWrapper | 1 | `AlreadyInitialized` | initialize called twice |
| SYWrapper | 2 | `Unauthorized` | caller ≠ admin |
| SYWrapper | 3 | `NotInitialized` | read before initialize |
| SYWrapper | 4 | `ZeroAmount` | deposit or withdraw ≤ 0 |
| SYWrapper | 5 | `InsufficientShares` | withdraw > balance |
| SYWrapper | 6 | `Paused` | operation while paused |
| SYWrapper | 7 | `ArithmeticOverflow` | fixed-point overflow |
| PrincipalManager | 1 | `AlreadyInitialized` | initialize called twice |
| PrincipalManager | 2 | `Unauthorized` | caller ≠ admin |
| PrincipalManager | 3 | `NotInitialized` | read before initialize |
| PrincipalManager | 4 | `ZeroAmount` | amount ≤ 0 |
| PrincipalManager | 5 | `NotMature` | redeem before maturity |
| PrincipalManager | 6 | `AlreadyMature` | mint after maturity |
| PrincipalManager | 7 | `OracleStale` | oracle too old at redemption |
| PrincipalManager | 8 | `InsufficientBalance` | redeem > PT or YT balance |
| PrincipalManager | 9 | `Paused` | operation while paused |
| PrincipalManager | 10 | `PermissionDenied` | user not in allow-list |
| RiskControl | 1 | `AlreadyInitialized` | initialize called twice |
| RiskControl | 2 | `Unauthorized` | caller ≠ admin |
| RiskControl | 3 | `NotInitialized` | read before initialize |
| RiskControl | 4 | `Paused` | check_deposit while paused |
| RiskControl | 5 | `CircuitBreakerTripped` | deposit exceeds rolling limit |
| RiskControl | 6 | `NotPauser` | pause called by non-pauser |
| RiskControl | 7 | `AlreadyPauser` | add_pauser for existing pauser |

---

## Test Coverage

Each contract has a unit test suite using `soroban_sdk::testutils`. Tests cover:

**OracleAdapter**
- Initialization and double-init guard
- Reference value update and retrieval
- Monotonic timestamp enforcement (reject stale timestamps)
- Freshness check (`is_fresh` with varying staleness thresholds)
- Unauthorized update attempt
- Admin transfer

**Permissioning**
- Account grant and revoke
- Asset-level grant and revoke
- Batch `grant_accounts`
- `is_allowed` and `is_allowed_for_asset` return values
- Unauthorized grant attempt
- Admin transfer

**SYWrapper**
- Deposit and share minting
- Exchange rate calculation at inception and after yield accrual simulation
- Withdrawal and underlying return
- Insufficient share balance rejection
- Pause and unpause behavior
- Admin transfer

**PrincipalManager**
- Mint PT and YT from SY shares
- Notional calculation at multiple oracle rates
- Redeem at maturity — PT and YT separately
- Redeem before maturity rejection (`NotMature`)
- Mint after maturity rejection (`AlreadyMature`)
- Oracle staleness rejection at redemption
- Permission check rejection
- Pause behavior
- Admin transfer

**RiskControl**
- Pause and unpause
- Pauser role add and remove
- Non-pauser `pause` rejection
- Non-admin `unpause` rejection
- Circuit breaker trip on volume excess
- Circuit breaker window reset after 24 hours
- Disabled circuit breaker (cb_limit = 0)
- Admin transfer

---

## Build and Run

```bash
# Prerequisites
rustup target add wasm32-unknown-unknown

# Run all tests
cargo test

# Build WASM artifacts
cargo build --target wasm32-unknown-unknown --release
```

WASM files are produced in `target/wasm32-unknown-unknown/release/`:
- `principal_oracle_adapter.wasm`
- `principal_permissioning.wasm`
- `principal_risk_control.wasm`
- `principal_sy_wrapper.wasm`
- `principal_manager.wasm`

---

## Deployment Order (POC)

```bash
# 1. OracleAdapter
stellar contract deploy --wasm target/.../principal_oracle_adapter.wasm \
  --source admin --network testnet --alias oracle_adapter
stellar contract invoke --id oracle_adapter --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS>

# 2. Permissioning
stellar contract deploy --wasm target/.../principal_permissioning.wasm \
  --source admin --network testnet --alias permissioning
stellar contract invoke --id permissioning --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS>

# 3. RiskControl
stellar contract deploy --wasm target/.../principal_risk_control.wasm \
  --source admin --network testnet --alias risk_control
stellar contract invoke --id risk_control --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --cb-limit 0

# 4. SYWrapper
stellar contract deploy --wasm target/.../principal_sy_wrapper.wasm \
  --source admin --network testnet --alias sy_wrapper
stellar contract invoke --id sy_wrapper --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --underlying <USDY_CONTRACT_ADDRESS>

# 5. PrincipalManager
stellar contract deploy --wasm target/.../principal_manager.wasm \
  --source admin --network testnet --alias principal_manager
stellar contract invoke --id principal_manager --source admin --network testnet \
  -- initialize \
     --admin <ADMIN_ADDRESS> \
     --sy-wrapper <SY_WRAPPER_ADDRESS> \
     --oracle <ORACLE_ADAPTER_ADDRESS> \
     --permissioning <PERMISSIONING_ADDRESS> \
     --maturity <UNIX_TIMESTAMP>
     # UNIX_TIMESTAMP = current time + maturity duration in seconds
     # Example for 3-month market: $(date -d "+90 days" +%s)
     # Example for 6-month market: $(date -d "+180 days" +%s)
```

See [DEPLOYMENT.md](DEPLOYMENT.md) for the complete guide including network configuration and post-deployment verification.

---

## Testnet Deployment

All five POC contracts have been deployed and initialised on **Stellar Testnet** (June 2026).
The deployment demonstrates the full infrastructure + tokenization layer of the protocol executing real on-chain transactions.

### Deployed Contract Addresses

| Contract | Address | Explorer |
|---|---|---|
| OracleAdapter | `CDJSHBEULGIFN6PS7VDEWBTWWDBFPLMU75K2YOCJVLDQS5YKQROG36NL` | [view](https://stellar.expert/explorer/testnet/contract/CDJSHBEULGIFN6PS7VDEWBTWWDBFPLMU75K2YOCJVLDQS5YKQROG36NL) |
| Permissioning | `CBLSJAM7M32NDMRMEWOADEONC563DNZE2Y2JDVGCDIQ7ZJ53HZPH2GA6` | [view](https://stellar.expert/explorer/testnet/contract/CBLSJAM7M32NDMRMEWOADEONC563DNZE2Y2JDVGCDIQ7ZJ53HZPH2GA6) |
| RiskControl | `CCBDWAHYF5MBHOR4LQJ7XQQQQJFTYMMHXGQC2GBBRV3HTRBU7UASOBJY` | [view](https://stellar.expert/explorer/testnet/contract/CCBDWAHYF5MBHOR4LQJ7XQQQQJFTYMMHXGQC2GBBRV3HTRBU7UASOBJY) |
| Mock USDY (SAC) | `CAS53AG5G3XHKHPGJQRYEB2SEAYAHZNRFZZ57WKMVBC54RZIAMULNHIL` | [view](https://stellar.expert/explorer/testnet/contract/CAS53AG5G3XHKHPGJQRYEB2SEAYAHZNRFZZ57WKMVBC54RZIAMULNHIL) |
| SYWrapper | `CC25AC7YDW32PSC4UNAT33LXG4E6IR3I3HRWHUGJSFOS5OTL7MZRZMLO` | [view](https://stellar.expert/explorer/testnet/contract/CC25AC7YDW32PSC4UNAT33LXG4E6IR3I3HRWHUGJSFOS5OTL7MZRZMLO) |
| PrincipalManager | `CCWPPNCPJMEHBJ2P4SKHZMW3JFN3ACGQTJHJYSN5NPHKZC4ZD2CVUVDH` | [view](https://stellar.expert/explorer/testnet/contract/CCWPPNCPJMEHBJ2P4SKHZMW3JFN3ACGQTJHJYSN5NPHKZC4ZD2CVUVDH) |

**Admin / Deployer:** `GB2HC2NLXR7LHKXGS2IZL4F5LZVQVKRBKCWONQQW4WIYUXDILHORWQPZ`  
**Market maturity:** 1789135669 (11 September 2026, 90-day market)  
**Mock USDY:** a Stellar Asset Contract issued by the admin address, used in place of the real Ondo USDY token for testnet demonstration.

> All amounts in the contracts use `SCALE = 10_000_000` (10^7). An amount of `1_000_000_000` represents 100 tokens; a rate of `10_300_000` represents 1.03 USDC per underlying.

---

### Phase A — Contract Deployment

Each contract is deployed in two Soroban transactions: one to upload the WASM binary to the ledger (pay for code storage), and one to instantiate the contract from that WASM hash.

#### OracleAdapter
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [ef4c396…](https://stellar.expert/explorer/testnet/tx/ef4c396512da4b0fbcf59c610d5cc8015fc2f6063d9b679ab520b0fe8881de07) | Uploads `principal_oracle_adapter.wasm` (15 KB) |
| Deploy contract | [994b94a…](https://stellar.expert/explorer/testnet/tx/994b94aa7272719bedcd70b8faa74d58e50cfe1d588db35422fd9c0d6355b799) | Instantiates contract at `CDJSHE…` |

#### Permissioning
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [5f51a46…](https://stellar.expert/explorer/testnet/tx/5f51a4681a64d45aa3b9b4760b1344655657437251793d770c0ffb16edb69450) | Uploads `principal_permissioning.wasm` (12 KB) |
| Deploy contract | [c2dbefb…](https://stellar.expert/explorer/testnet/tx/c2dbefb03f29abf026fb523b82911837c6e8d11dcdb24b8fe7f950e7d4a6b94d) | Instantiates contract at `CBLSJA…` |

#### RiskControl
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [27b117e…](https://stellar.expert/explorer/testnet/tx/27b117eb7a424d133e38747bcf721132a8d3a36e363f43bcbf0b27d8b3d7b617) | Uploads `principal_risk_control.wasm` (18 KB) |
| Deploy contract | [559c16a…](https://stellar.expert/explorer/testnet/tx/559c16aba8e228d577168aa553408d21a6dca963d204ca7ebdc8482f4329ac1f) | Instantiates contract at `CCBDWA…` |

#### Mock USDY (SAC)
| Step | Transaction | Description |
|---|---|---|
| Deploy SAC | [e47e623…](https://stellar.expert/explorer/testnet/tx/e47e62308e515af41589068fb62b3664b02c52c8b715ff24c4c97bec22026ccc) | Wraps native Stellar asset `USDY:admin` as a Soroban token contract (SAC) |

#### SYWrapper
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [d2ed531…](https://stellar.expert/explorer/testnet/tx/d2ed531f3cdb80d831b58b1126a206f348c58a5c5e88efe835707a4f0e2f4c1b) | Uploads `principal_sy_wrapper.wasm` (17 KB) |
| Deploy contract | [5e6960b…](https://stellar.expert/explorer/testnet/tx/5e6960b5671cb702d2f37d33838fce791d4589c8874398116754132c13ccf469) | Instantiates contract at `CC25AC…` |

#### PrincipalManager
| Step | Transaction | Description |
|---|---|---|
| Upload WASM | [2514f9d…](https://stellar.expert/explorer/testnet/tx/2514f9d602d5a10449c5223b02dd984c642c5b2fc3b94ecb8b82c23e70612d61) | Uploads `principal_manager.wasm` (28 KB) |
| Deploy contract | [b79d485…](https://stellar.expert/explorer/testnet/tx/b79d485cbfe56b4d88fcbdd0a4bce73469af7f408ba174e4a250dbb8f112d0bd) | Instantiates contract at `CCWPPN…` |

---

### Phase B — Initialization

After deployment every contract is one-time-initialized to set the admin and register its dependencies.
Initialization is a separate transaction from deployment so that each contract's address is known before the next contract is configured.

**TX-01 · OracleAdapter.initialize**
```
stellar contract invoke --id CDJSHE... -- initialize --admin GB2HC2...
```
Transaction: [dd172cc…](https://stellar.expert/explorer/testnet/tx/dd172cc15d366947ea275fcde8bfbce2bc8716deb6465a3fc8c2e7fee4d2d0f2)

Sets the admin key that is authorized to update the USDY/USDC reference rate. After this call the oracle is live but has no price — any `get_reference_value` call would revert until a price is submitted.

---

**TX-02 · OracleAdapter.set_reference_value**
```
stellar contract invoke --id CDJSHE... \
  -- set_reference_value --caller GB2HC2... --value 10300000 --timestamp <unix>
```
Transaction: [6354d3f…](https://stellar.expert/explorer/testnet/tx/6354d3f93ac98d3844ea929ed07b21c1e59189ff39cb7c3658f0a153802f48bc)

Submits the first USDY/USDC reference rate: **1.03 USDC per USDY** (`value = 10_300_000`, scaled by 10^7). The monotonic timestamp guard ensures no relay can submit an older rate and replay a stale price. After this transaction `is_fresh(3600)` returns `true`.

---

**TX-03 · Permissioning.initialize**
```
stellar contract invoke --id CBLSJA... -- initialize --admin GB2HC2...
```
Transaction: [cc63716…](https://stellar.expert/explorer/testnet/tx/cc63716a67fb1a1f87c0bede8559d4483fb0634814bdd5d968920256eac06e45)

Initializes the eligibility registry. No accounts are allowed yet — every `is_allowed` call returns `false` until the admin explicitly grants access.

---

**TX-04 · RiskControl.initialize**
```
stellar contract invoke --id CCBDWA... -- initialize --admin GB2HC2... --cb-limit 0
```
Transaction: [2fe9fcb…](https://stellar.expert/explorer/testnet/tx/2fe9fcb9ca4e6134c6ab2b9ab724562d835403a3c1a14fda136808275aa83825)

Initializes the global pause with `cb_limit = 0` (circuit breaker disabled). Protocol is unpaused; any deposit volume is accepted. The circuit breaker can be enabled later with `set_cb_limit`.

---

**TX-05 · SYWrapper.initialize**
```
stellar contract invoke --id CC25AC... \
  -- initialize --admin GB2HC2... --underlying CAS53A...
```
Transaction: [6616f7a…](https://stellar.expert/explorer/testnet/tx/6616f7a3e1033f1036575babee7b6de9b090d3082594191d24bf2409fa7e91b6)

Registers the mock USDY SAC (`CAS53A…`) as the underlying asset. From this point SYWrapper can accept USDY deposits. The initial exchange rate is 1:1 (`10_000_000`).

---

**TX-06 · Permissioning.grant_account**
```
stellar contract invoke --id CBLSJA... \
  -- grant_account --caller GB2HC2... --account GB2HC2...
```
Transaction: [67a31da…](https://stellar.expert/explorer/testnet/tx/67a31da87754a2acf94ef383dd4a4cc78aade6e69aca100387d61f403ec6e809)

Adds the admin address to the eligibility allow-list. After this call `is_allowed(GB2HC2...)` returns `true`. Any account not in this list will be rejected by `PrincipalManager.mint()` with `PermissionDenied`.

---

**TX-07 · PrincipalManager.initialize**
```
stellar contract invoke --id CCWPPN... \
  -- initialize \
     --admin GB2HC2... \
     --sy-wrapper CC25AC... \
     --oracle CDJSHE... \
     --permissioning CBLSJA... \
     --maturity 1789135669
```
Transaction: [c9ee150…](https://stellar.expert/explorer/testnet/tx/c9ee1506c5536665cbcd3787cc4d5494d8b474c62c6e752a2fdd252eb7b0d0af)

Ties all infrastructure contracts together into one market. The maturity timestamp **1789135669** corresponds to **11 September 2026** — a 90-day market from deployment date. Before this timestamp `mint()` is open and `redeem()` reverts; after it the positions reverse.

---

### Phase C — Protocol Transactions

With all contracts initialized, the core yield-tokenization flow is executed on-chain.

**TX-08 · SYWrapper.deposit — wrap 100 USDY into SY shares**
```
stellar contract invoke --id CC25AC... \
  -- deposit --from GB2HC2... --amount 1000000000
```
Transaction: [045d054…](https://stellar.expert/explorer/testnet/tx/045d054704adc3b43acacd4992c5eb5012b9f35e82302633c9a16604e8e842c0)

The user transfers **100 USDY** (`1_000_000_000` in 10^7 units) to the SYWrapper. The contract:
1. Calls `token::transfer(user → SYWrapper, 100 USDY)` — an actual on-chain SEP-41 token transfer.
2. Computes shares at the current exchange rate (1:1 at inception → 100 shares).
3. Credits `1_000_000_000` SY-USDY shares to the user's persistent storage slot.

**On-chain state after TX-08:**
```
total_underlying  = 1_000_000_000   (100 USDY held by SYWrapper)
total_shares      = 1_000_000_000   (100 SY-USDY issued)
exchange_rate     = 10_000_000      (1.0 — 1 USDY per SY share)
deployer balance  = 1_000_000_000   (100 SY-USDY shares)
```

---

**TX-09 · PrincipalManager.mint — split 50 SY-USDY into PT + YT**
```
stellar contract invoke --id CCWPPN... \
  -- mint --from GB2HC2... --sy-shares 500000000
```
Transaction: [289ceb6…](https://stellar.expert/explorer/testnet/tx/289ceb63d66a21396673d12ef6b89b4ac38798229832c9c871ee9c43b1d80a69)

The user submits **50 SY-USDY shares** (`500_000_000`) for tokenization. The contract:
1. Calls `OracleAdapter.get_reference_value()` → receives `10_300_000` (1.03 USDC/USDY).
2. Verifies the user is on the Permissioning allow-list via `PermClient.is_allowed()`.
3. Stores `initial_rate = 10_300_000` for this user (used at YT settlement).
4. Computes notional: `500_000_000 × 10_300_000 / 10_000_000 = 515_000_000` (51.5 USDC).
5. Credits `515_000_000` PT and `515_000_000` YT to the user's persistent storage.

**On-chain state after TX-09:**
```
PT balance (deployer) = 515_000_000   (51.5 USDC notional)
YT balance (deployer) = 515_000_000   (51.5 USDC notional)
total_PT              = 515_000_000
total_YT              = 515_000_000
is_mature             = false          (matures 11 Sep 2026)
```

The notional is 51.5 USDC, not 50: the oracle rate of 1.03 USDC/USDY means each USDY share is worth slightly more than 1 USDC, so 50 USDY × 1.03 = 51.5 USDC of PT+YT.

---

**TX-10 · RiskControl.set_cb_limit — enable 24h deposit circuit breaker**
```
stellar contract invoke --id CCBDWA... \
  -- set_cb_limit --caller GB2HC2... --new-limit 500000000
```
Transaction: [3ccb389…](https://stellar.expert/explorer/testnet/tx/3ccb3895b0dde4177255eb6ac0ed37ac5cfb50272081ad8c063097d4c98141b9)

Sets the rolling 24-hour deposit limit to **50 USDY** (`500_000_000`). Any call to `check_deposit` that would push cumulative volume over this limit within a 24-hour window reverts with `CircuitBreakerTripped`. The window resets automatically after `CB_WINDOW_SECS = 86_400` seconds.

---

**TX-11 · RiskControl.check_deposit — record a 10 USDY deposit against the circuit breaker**
```
stellar contract invoke --id CCBDWA... \
  -- check_deposit --amount 100000000
```
Transaction: [ab1d8bc…](https://stellar.expert/explorer/testnet/tx/ab1d8bced4dcc94b107505b8944c95ed83792c3e2052759d8b47054139d7b69a)

Records a **10 USDY** deposit event (`100_000_000`) against the circuit breaker window. After this call `get_cb_volume()` returns `100_000_000`. A subsequent call with more than `400_000_000` (40 USDY) within the same 24-hour window would trip the breaker and revert.

---

### What the Testnet Deployment Proves

| Claim | Evidence |
|---|---|
| Soroban contracts compile and deploy | 5 contracts live on testnet with verified WASM hashes |
| Oracle price feed works | TX-02 sets 1.03 rate; `is_fresh` returns `true` |
| Permissioning enforces eligibility | TX-06 grants account; PrincipalManager checks it on every mint |
| SYWrapper wraps yield-bearing tokens | TX-08 executes a real SEP-41 `transfer` on-chain |
| PT+YT minting uses oracle rate | TX-09 produces 51.5 USDC notional from 50 USDY × 1.03 |
| Per-user `initial_rate` is stored | Visible in TX-09 ledger state; used at maturity settlement |
| Circuit breaker tracks volume | TX-11 increments `cb_volume`; limit enforced in-window |
| All contracts are interoperable | TX-09 makes live cross-contract calls to OracleAdapter and Permissioning |

---

## Known POC Limitations

The following items are intentional scope boundaries for the POC and will be addressed in Phase 2:

1. **Internal PT/YT balances** — PT and YT are tracked as mappings inside `PrincipalManager`, not as standalone SEP-41 tokens. They cannot be held in external wallets or traded on DEX pools without Phase 2 contracts.

2. **No AMM** — `MarketPool` is not implemented. There is no on-chain market for PT or YT trading in the POC.

3. **No Router** — Users interact with each contract individually. Single-transaction flows (wrap + mint, swap, recombine) require Phase 2 Router integration.

4. **No yield claiming** — YT holders cannot claim accrued yield before maturity in the POC. The full YT yield streaming and claiming mechanism is a Phase 2 feature.

5. **No recombination** — PT + YT → SY recombination before maturity is planned for Phase 2.

6. **Single oracle submitter** — The POC uses a single admin-controlled oracle. Multi-source aggregation and quorum oracle are Phase 2 infrastructure.

7. **Cross-contract wiring of RiskControl** — In the POC, `RiskControl.check_deposit` is invoked directly by the caller (e.g. a test harness or admin script) rather than being called automatically by `SYWrapper` and `PrincipalManager` as cross-contract calls. The risk control logic and interface are fully implemented and tested; Phase 2 wires them into the deposit and mint call paths so no user can bypass the circuit breaker by calling `SYWrapper` directly. The POC focuses on proving the logic correctness of each contract in isolation; end-to-end cross-contract integration is the Phase 2 scope.

8. **SY share transfer not dispatched** — `PrincipalManager.mint()` records the SY share deposit internally but does not call `SYWrapper.transfer()` to move shares into the contract. Similarly, `redeem()` computes and returns the underlying amounts due but does not dispatch the token transfer to the caller. Both transfers are Phase 2 milestones once the Router contract is available to orchestrate multi-step flows in a single transaction.

9. **Single `initial_rate` per user** — `PrincipalManager` stores one oracle rate per user address at the time of their first mint. If a user mints multiple times across different oracle rates, all their YT is settled using the rate from the first mint. Production will track rates per mint batch to ensure each YT unit is settled against the rate at the time it was created.
