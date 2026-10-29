# Dapp Smart Contract Integration Guide: Market Creation and AMM

**Audience:** dapp/backend engineers integrating against Principal Protocol's Soroban contracts.
**Status:** every transaction cited in this document was submitted to Stellar Testnet on 29 October
2026 and is linked by hash; every function signature was read directly from the contract source at
`contracts/*/src/lib.rs` in this commit. Nothing here is inferred from documentation alone.

This guide covers the two things not fully covered end-to-end elsewhere: **how to create a new
market** (`MarketFactoryContract`) and **how to integrate the AMM** (`MarketPool`, via `Router`),
with a complete worked example and the exact scripts used to produce it. For the full protocol
architecture, settlement math, and error model, see `docs/TECHNICAL_SPECIFICATION.md`,
`docs/YIELD_MATH_AND_FEES.md`, and `docs/MARKET_FACTORY.md` — this document does not repeat them,
it shows how to *call* them.

**A note on paths used below:** `contracts/*` and `scripts/*` paths are in *this* repository
(`principal`). `src/lib/*.ts`, `src/types.ts` and `src/config/markets.ts` paths in §2–§4 refer to
`principal-dApp`, the companion frontend repository this guide's TypeScript integration patterns
are grounded in — a separate codebase, not part of this repository, read directly while writing
this guide for accuracy rather than guessed at.

---

## 1. Architecture in one table

| Layer | Contract | Role |
|---|---|---|
| Infrastructure (shared) | `OracleAdapter` | monotonic reference-value feed, freshness-checked |
| | `Permissioning` | optional per-market eligibility, narrower than the underlying |
| | `RiskControl` | pause, pausers, circuit breaker |
| | `MarketFactory` | stateless, atomic per-market deployment (new; this guide, §3) |
| Tokenization (per asset) | `SYWrapper` | underlying ↔ SY shares |
| | `PrincipalManager` | SY → PT+YT, fees, settlement, redemption |
| | `MarketConfig` | per-market fees and payees |
| | `RecoveryEscrow` | compliance recovery (seize/unwind) |
| Market (per maturity) | `PTToken`, `YTToken` | SEP-41-style tokens |
| | `MarketPool` | the AMM (this guide, §5) |
| | `Router` | stateless one-transaction user flows |

A full market is 10 contracts (7 per-market + 3 shared) plus the shared `Router`. `principal_compliance` is a library, not a deployed contract.

---

## 2. How a dapp actually talks to these contracts

This section describes the pattern already implemented in the companion dapp
(`principal-dApp`, `src/lib/stellar.ts`/`protocol.ts`/`transactions.ts`) — reuse it rather than
inventing another one. The lifecycle for every write call is:

1. **Build** an unsigned transaction: one `Contract(contractId).call(functionName, ...args)`
   operation via `@stellar/stellar-sdk`'s `TransactionBuilder`.
2. **Prepare** it: `server.prepareTransaction(tx)` — this runs Soroban RPC simulation and attaches
   the resource footprint, auth entries, and resource fee. Read `minResourceFee` /
   `simulation.transactionFee` here, before asking anyone to sign.
3. **Review**: show the human everything simulation produced (call tree, fee, diagnostics,
   footprint) — never sign off blind data.
4. **Sign**: hand the *exact same* unsigned XDR to the wallet (Freighter). After signing, verify
   the signed envelope's body hash equals the unsigned one's and that the signature belongs to the
   expected account — a wallet is not trusted to return what it was asked to sign without
   checking.
5. **Submit**: `server.sendTransaction(signedTx)`, then poll `server.getTransaction(hash)` until a
   terminal status.
6. **Read-only calls** (quotes, balances, state) skip steps 3–5 entirely: simulate with a null
   source account and read `result.retval` — no signature, no submission, no fee.

```ts
// Shape already defined in principal-dApp/src/lib/stellar.ts — reuse these types directly.
type ScArg =
  | { type: "address"; value: string }
  | { type: "i128"; value: string | number | bigint }
  | { type: "u32"; value: number }
  | { type: "u64"; value: string | number | bigint }
  | { type: "bool"; value: boolean }
  | { type: "symbol"; value: string }
  | { type: "string"; value: string };

type ContractInvocation = { contractId: string; functionName: string; args: ScArg[] };
```

**One Soroban-specific gotcha that will bite a naive CLI wrapper** (found live while writing this
guide): `stellar contract invoke` prints a scalar `i128`/`u64` return value as a JSON string with
the quotes included — `"500000000"`, not `500000000` — because i128 doesn't fit JS's number type.
Feed that string straight into the next `--amount "$VALUE"` argument and the CLI rejects it:
`Expected type i128 ... but received: '"500000000"'`. Strip the quotes before reusing any CLI
output as the next call's input (`scripts/amm_lifecycle_demo.sh`'s `raw()` helper does this). The
JS/TS SDK does not have this problem — `scValToNative` already returns a real `bigint`.

A second gotcha, also found live: an `InsufficientRefundableFee` submission failure (simulation
succeeds, submission doesn't) is a **Soroban resource-fee** shortfall, not a network-fee one.
`stellar contract invoke --inclusion-fee <n>` does **not** fix it — that pads the classic network
fee. `--resource-fee <n>` is the flag that overrides the simulated resource fee; it fixed a
real `swap_pt_for_sy` failure immediately after an `add_liquidity` call in the same session (§5.3).
In the JS SDK, this is `prepareTransaction`'s own simulated `sorobanData.resourceFee` — pad it
directly if you see the same error rather than guessing which fee needs raising.

---

## 3. Creating a market: `MarketFactoryContract`

Full design rationale and the Testnet evidence for *why* this is three calls, not one, is in
`docs/MARKET_FACTORY.md` — read that once. This section is the practical "how," re-verified live
for this guide with a fourth market.

### 3.1 Preconditions

- The underlying asset exists (a SAC or SEP-57 token) and its issuer authority is the one signing
  every call below.
- Shared infrastructure already exists: `OracleAdapter`, `Permissioning`, `RiskControl`, `Router`.
  These are reused across markets, never redeployed per market.
- The oracle has a **fresh** reference value (< 3600s old) at the moment `create_tokens` runs —
  `YTToken.initialize` reads it live as the yield-index genesis rate and reverts `OracleStale`
  otherwise. This bit the first live run of this exact guide's example (§3.4, finding 1).
- Each of the seven per-market contracts' Wasm is already uploaded (`stellar contract upload`, or
  `Deployer.upload_contract_wasm` from inside another contract) — the factory takes hashes, not
  bytecode.

### 3.2 `predict_market` — see the addresses before anything is signed

```bash
stellar contract invoke --id "$FACTORY_ID" --source "$ISSUER_KEY" --network testnet \
  --send=no -- predict_market --salt "$SALT"
```

Pure computation, no auth, no deploy. Returns the seven addresses `create_tokens` /
`create_engine` / `create_pool_and_escrow` will produce for this exact `salt`. Use this to render
a "you are about to create these contracts" confirmation screen before the issuer signs anything.

### 3.3 The three creation calls

| Call | Deploys | Signer |
|---|---|---|
| `create_tokens(issuer, salt, TokenWasmHashes, TokenParams)` | `SYWrapper`, `PTToken`, `YTToken` | issuer only |
| `create_engine(issuer, salt, EngineWasmHashes, tokens, EngineParams)` | `MarketConfig`, `PrincipalManager` (+ wires minter, risk control) | issuer only |
| `create_pool_and_escrow(issuer, salt, PoolEscrowWasmHashes, tokens, engine, PoolEscrowParams)` | `MarketPool`, `RecoveryEscrow` (+ wires recovery escrow) | issuer only |

`TokenParams`: `underlying, maturity, permissioning, oracle, pt_name, pt_symbol, yt_name, yt_symbol, decimals`.
`EngineParams`: `underlying, maturity, permissioning, oracle, risk_control, protocol_admin, treasury, tokenization_fee_bps, yt_fee_bps, swap_fee_tier_bps, protocol_share_bps`.
`PoolEscrowParams`: `underlying, time_stretch_years`.

The **same `salt`** (32 bytes, hex) goes into all three calls, and the `tokens`/`engine` structs
returned by the earlier calls are passed forward unchanged into the later ones.

After the third call, two steps remain — deliberately left out of the factory (different signer,
and a human eligibility decision, respectively):

```bash
stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ISSUER_KEY" --network testnet \
  -- add_consumer --caller "$ISSUER_ADDR" --consumer "$SY_ADDR"
stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ISSUER_KEY" --network testnet \
  -- add_consumer --caller "$ISSUER_ADDR" --consumer "$MANAGER_ADDR"
stellar contract invoke --id "$ROUTER_ID" --source "$ISSUER_KEY" --network testnet \
  -- register_market --caller "$ISSUER_ADDR" --pool "$POOL_ADDR"
```

Then compliance standing (`docs/DEPLOYMENT.md` §11) for every address that custodies value: `SYWrapper`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow`, the treasury, and the creator payee.

### 3.4 Real example — market created live for this guide

Run with [`scripts/create_market_via_factory.sh`](../scripts/create_market_via_factory.sh) against the live Testnet factory `CCQQPZ63JQP63X2OP5REATX46CQMO7GTDRQBUHZYPGSMSBOHZII577ZZ`, reusing the shared infrastructure from `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`:

```bash
FACTORY_ID=CCQQPZ63JQP63X2OP5REATX46CQMO7GTDRQBUHZYPGSMSBOHZII577ZZ \
ISSUER_KEY=sta-testnet-deployer \
UNDERLYING_ID=CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L \
MATURITY_TS=$(($(date +%s) + 1800)) \
PERMISSIONING_ID=CDX64AORDY5FMDPQH633WIMNH5NBABW34UIXEDIS5E3IFWU7QFQ6YMCF \
ORACLE_ID=CAW53CGHBDS3DS6JJ7NOWSTCJCKNN663FEEN3WOYWJKMDOFAALZIXCRI \
ORACLE_CALLER=GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB \
ORACLE_VALUE=10500000 \
RISK_CONTROL_ID=CCQKY4S4TC5G3LZIT37I5IBSSDVEZIIVPZSMCJVSOTASGKC6XKTZKLYR \
ROUTER_ID=CARTAP66PCU6GJJ4MGWATZM54YIT4RF7WNTPCA2KHMQD6W23NNU7KC2U \
PROTOCOL_ADMIN_ADDR=GAZHOIUXZGGEW3CSTE2EDVMM6FX65EBN3JW3J5AV225WC5QCHLCHYZO6 \
TREASURY_ADDR=GASQCEPSJJFWY5ITMQGNOVXQOGIQK2CBG6KBMHT7EMHEMSP7MHMYYAGR \
SY_WASM=a3d6b0640a08639da5450a1f5d52cd73a8b5d5c5a1b1cdd7724d8f3dc55e261d \
PT_WASM=44fd470c791b36c597d96ccebbaf3ed4c0bb216449f089f4ed3e0b0a981b168a \
YT_WASM=ab62a8eafdf9dd0e407c45a4b9fef3db2ab449ea1c3445e2a1bc29ef9d32f3e5 \
CONFIG_WASM=11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8 \
MANAGER_WASM=ba4537b3e2bfb5a602cdd475776b40f418d112af28ef43513b5bcf305d2ead26 \
POOL_WASM=ca8f32c5861724f5d32cac15ff8efbb0d76eea085453269699d9b0afc78c171a \
ESCROW_WASM=a4d46832ef28f9f4ee30ce64dd2115c2e2dbbfb86eb1f6ad985debedd9b33590 \
PT_NAME="Principal Token F5" PT_SYMBOL="PT-F5" YT_NAME="Yield Token F5" YT_SYMBOL="YT-F5" \
./scripts/create_market_via_factory.sh
```

The oracle refresh the script ran first (precondition, §3.1) is
[dfa5de15...](https://stellar.expert/explorer/testnet/tx/dfa5de159caae0e2cc7e9cba330e2c5c78da11b700a585fb2ae9c84d4ca7e482).

Result — salt `5d636a6b776a141e0998261811adaf4ba5c7a846dd43ee83816e010646c97815`, maturity `1793300108`:

| Contract | Address | Transaction |
|---|---|---|
| SYWrapper | `CCPB3QQPAPQRF6BL257XJ5THB6HFWZCSV5GKZ63SSD56DRHU4ZDU3NI6` | [7ca23c4f...](https://stellar.expert/explorer/testnet/tx/7ca23c4f6eb554da8d2d37fb98d7e2cd1a84c654ebd4981610e8baf605b8c16f) (`create_tokens`) |
| PTToken | `CAY4SA3DC2D76QP52E7ICE3U3OYGJX26LE6MSJVXLAMFI3XFFKD74ABA` | same tx |
| YTToken | `CBGIXGV2PL7ECJT5EFWULMTFLG5TT3YML33JULCAWVVDKERS7PMBAI7Y` | same tx |
| MarketConfig | `CA476QAORIFCBP2WIMAZ75EAXARPQBRCIIUQ6JH6GEGZD33UTO2QZQWR` | [866aec49...](https://stellar.expert/explorer/testnet/tx/866aec49899bd2c9486c0078b0aba5bd6d1343baad94216306f4556f768e2925) (`create_engine`) |
| PrincipalManager | `CAC53MLBMELUS4JKHFS2HNAKXONAV2SKU2V7Z4ZFJVNJDLRHQ23OW7M6` | same tx |
| MarketPool | `CCNWLZFDPQ556C7OZEUW6577FSRVCW5LLV4JAJOLB5HW5OGVC263TVPO` | [105726a3...](https://stellar.expert/explorer/testnet/tx/105726a3d95019c11e4045ffb5b40ef3cf358636a1e85689184152df0ba3a491) (`create_pool_and_escrow`) |
| RecoveryEscrow | `CB55EFJDMU525Y7HEPX7S5XKV5VTA6VYZNPTYWPTFE7BFLCQPE3SR32W` | same tx |

Wiring: `add_consumer(SY)` [c7372a70...](https://stellar.expert/explorer/testnet/tx/c7372a70b5430ec68fe4cd43ee16ee7d26ccd9ae6fee15bc783e20230ac06c8b), `add_consumer(Manager)` [245dae50...](https://stellar.expert/explorer/testnet/tx/245dae50ac2abef8dba4e118702707f9c9ce77dcf5088f774e4e1c43ef546a24), `register_market` [fc6f5782...](https://stellar.expert/explorer/testnet/tx/fc6f5782932badcc95d84c273d660ffd65b7f576dfefd0ec2957710e62b72a38).

**Two real failures hit and fixed while producing this exact example** (kept here, not smoothed over, because they are exactly what a first-time integrator will also hit):

1. **First attempt reverted `OracleStale`** (YTToken error 10) — the script didn't refresh the
   oracle before `create_tokens`, and the last-known reference value was already older than 3600s.
   Fixed by adding the `ORACLE_CALLER`/`ORACLE_VALUE` refresh step shown above, which must run
   *before* `create_tokens`, not after.
2. **A reused `salt` from the failed attempt could not be reused** — Soroban refuses to redeploy a
   contract at an address that already has code, exactly as designed (§3.6). The fix was simply a
   fresh salt, not a code change — this is the host-level guarantee the factory relies on, not a
   bug.

### 3.5 TypeScript sketch for `principal-dApp`

The companion dapp's `MarketManifest`/`ContractMap` types (`src/types.ts`) and its one registered
market (`src/config/markets.ts`) predate the factory and have **no slot for it and no market
creation action**. Extending them is a type-level, additive change — sketched here, not applied,
since it's a separate repository with its own release process:

```ts
// Extends ContractMap (src/types.ts) with the one new shared contract a registry entry needs:
export type ContractMap = {
  // ...existing fields unchanged...
  factory: string; // new: MarketFactoryContract, shared across markets like router/oracle/riskControl
};

// New transaction-builder functions alongside principalInvocationForIntent (src/lib/protocol.ts):
export function createTokensInvocation(factory: string, issuer: string, salt: string, wasm: TokenWasmHashes, params: TokenParams): ContractInvocation {
  return {
    contractId: factory,
    functionName: "create_tokens",
    args: [
      { type: "address", value: issuer },
      { type: "bytes32" /* add to ScArg union */, value: salt },
      /* wasm and params marshal to ScVal maps -- see §3.3's field names */
    ],
  };
}
// ...create_engine / create_pool_and_escrow follow the same shape, threading `tokens`/`engine`
// forward exactly as scripts/create_market_via_factory.sh does.
```

Note `ScArg`'s union (`src/lib/stellar.ts`) has no `bytes32`/struct-map variant today — both are
needed for `salt` and the `TokenWasmHashes`/`*Params` structs, which `nativeToScVal` can produce
from a plain JS object once the field is added. This is the only structural gap; the rest of the
build/prepare/sign/submit lifecycle in §2 applies unchanged.

### 3.6 Determinism and salts

Each per-market contract's address is `derive(FactoryAddress, sha256(salt ++ tag))` for a small
fixed `tag` per contract kind — not derived from the issuer or underlying. Reusing a `salt` that
already deployed something fails outright (a second `create_contract_with_constructor` at an
occupied address reverts at the host level); pick a fresh salt per market, e.g.
`sha256(underlying ++ maturity ++ nonce)`.

---

## 4. Core user flows (via `Router`)

These are **already implemented** in `principal-dApp/src/lib/protocol.ts`
(`principalInvocationForIntent`) — this table is the reference for what each one calls on-chain,
not new code.

| Dapp action | Router call | Downstream |
|---|---|---|
| `wrap_and_mint` | `Router.wrap_and_mint(from, pool, amount, min_pt_out, deadline)` | `SYWrapper.deposit` → `PrincipalManager.mint` |
| `swap_sy_for_pt` | `Router.swap_sy_for_pt(from, pool, sy_in, min_pt_out, deadline)` | `MarketPool.swap_sy_for_pt` |
| `swap_pt_for_sy` | `Router.swap_pt_for_sy(from, pool, pt_in, min_sy_out, deadline)` | `MarketPool.swap_pt_for_sy` |
| `swap_sy_for_yt` (flash-mint) | `Router.swap_sy_for_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` | `PrincipalManager.mint` → `MarketPool.swap_pt_for_sy` |
| `swap_yt_for_sy` (flash-redeem) | `Router.swap_yt_for_sy(from, pool, yt_in, min_sy_out, deadline)` | `MarketPool.swap_yt_for_sy` |
| `add_liquidity` | `Router.add_liquidity(from, pool, pt_in, sy_in, min_lp_out, deadline)` | `MarketPool.add_liquidity` |
| `add_liquidity_single_sy` | `Router.add_liquidity_single_sy(from, pool, sy_in, min_lp_out, deadline)` | `MarketPool.add_liquidity_single_sy` |
| `remove_liquidity` | `Router.remove_liquidity(from, pool, lp_in, min_pt_out, min_sy_out, deadline)` | `MarketPool.remove_liquidity` |
| `recombine` | `Router.recombine(from, pool, amount, min_sy_out, deadline)` | `PrincipalManager.recombine` |
| `claim_yield` | `Router.claim_yield(from, pool)` | `PrincipalManager.claim_yield` |
| `redeem_at_maturity` | `Router.redeem_at_maturity(from, pool, pt_amount, yt_amount)` | `PrincipalManager.redeem` |

`claim_yield` and `redeem_at_maturity` take no `deadline` — correct, not an omission:
`PrincipalManager.redeem`/`claim_yield` have no slippage window to bound (settlement is already
rate-frozen, or the call is balance-only).

The Router never custodies funds and is never itself compliance-gated; every downstream check runs
against `from`, the real user, so a deauthorized user gains nothing by routing through it.

---

## 5. AMM integration: `MarketPool` direct interface

`Router` wraps these for normal users; call `MarketPool` directly for liquidity management,
protocol/creator fee claims, or pricing that `Router` doesn't expose.

### 5.1 Pricing (read-only, no auth, no fee)

| Function | Returns |
|---|---|
| `pt_price()` | SY per PT, at `SCALE = 1e7` |
| `implied_rate()` | annualized rate implied by the current price, at `SCALE = 1e7` |
| `time_exponent()` | the curve's `a` exponent at WAD (`1e18`); `1e18` at maturity |
| `quote_sy_for_pt(sy_in)` | `Quote { amount_out, fee_shares }` — buying PT with `sy_in` SY |
| `quote_pt_for_sy(pt_in)` | `Quote { amount_out, fee_shares }` — selling `pt_in` PT |
| `quote_buy_exact_pt(pt_out)` | `Quote { amount_in, fee_shares }` — SY needed for an exact PT amount |
| `zap_swap_amount(sy_in)` | how much of a single-sided SY deposit converts to PT |
| `reserves()` | `(pt_reserve, sy_reserve)` |
| `pool_state()`, `maturity()`, `is_paused()` | self-explanatory |

`principal-dApp/src/lib/quotes.ts` wires `quote_sy_for_pt`/`quote_pt_for_sy` today (the two
swap-quote actions); `quote_buy_exact_pt`, `zap_swap_amount`, `pt_price`, and `implied_rate` are
not wired yet — direct pricing/exact-output flows and a market-overview "current rate" display
would need them added, following the exact pattern `fetchMarketQuote` already uses.

Live example against the market from §3.4 (`scripts/quote_market.sh`):

```bash
$ ./scripts/quote_market.sh CCNWLZFDPQ556C7OZEUW6577FSRVCW5LLV4JAJOLB5HW5OGVC263TVPO v11-carol 10000000 20000000
pt_price (SY per PT, SCALE=1e7): "9999912"
implied_rate (annualized, SCALE=1e7): "2396518"
reserves (pt_reserve, sy_reserve): ["770000000","281204038"]
maturity: 1793300108
is_paused: false
quote_sy_for_pt(10000000): {"amount_in":"10000000","amount_out":"10500087","fee_shares":"1"}
quote_pt_for_sy(20000000): {"amount_in":"0","amount_out":"19047442","fee_shares":"1"}
```

### 5.2 Direct swap and liquidity functions (require `from.require_auth()`)

| Function | Signature |
|---|---|
| `swap_sy_for_pt` | `(from, to, sy_in, min_pt_out) -> i128` |
| `swap_pt_for_sy` | `(from, to, pt_in, min_sy_out) -> i128` |
| `swap_yt_for_sy` | `(from, to, yt_in, min_sy_out) -> i128` (flash-redeem: recombines with the pool's own PT) |
| `add_liquidity` | `(from, pt_desired, sy_desired, min_lp_out) -> (pt_used, sy_used, lp_minted)` |
| `add_liquidity_single_sy` | `(from, sy_in, min_lp_out) -> (pt_used, sy_used, lp_minted)` |
| `remove_liquidity` | `(from, to, lp_in, min_pt_out, min_sy_out) -> (pt_out, sy_out)` |
| `claim_protocol_fees`, `claim_creator_fees` | `() -> i128`, permissionless, pay the fixed payees from `MarketConfig` |

`add_liquidity`'s first-ever call on an empty pool additionally requires
`sy_desired * rate / SCALE <= pt_desired` (`InvalidInitialRatio`, error 15, otherwise) and that
`isqrt(pt_desired * sy_desired) > MINIMUM_LIQUIDITY`. Hit live while preparing this guide, against
a pool at a 1.05 oracle rate: `add_liquidity(pt_desired=500000000, sy_desired=500000000, ...)`
reverted `InvalidInitialRatio` (`500,000,000 × 1.05 = 525,000,000 > 500,000,000`); lowering to
`sy_desired=400000000` succeeded (`400,000,000 × 1.05 = 420,000,000 ≤ 500,000,000`).

### 5.3 Real example — the full AMM lifecycle, live

Before this runs, every address that custodies this market's value (`SYWrapper`,
`PrincipalManager`, `MarketPool`, `RecoveryEscrow`) and the test user (Carol, an existing
Testnet account already authorized on the underlying) were granted compliance standing per §3.3's
checklist and `docs/DEPLOYMENT.md` §11 — eleven transactions total, each the same
`set_authorized`/`grant_account`/`grant_asset` pattern already shown in §3.3, not repeated here.

Run with [`scripts/amm_lifecycle_demo.sh`](../scripts/amm_lifecycle_demo.sh) against the market
from §3.4, as Carol:

```bash
SY_ID=CCPB3QQPAPQRF6BL257XJ5THB6HFWZCSV5GKZ63SSD56DRHU4ZDU3NI6 \
PT_ID=CAY4SA3DC2D76QP52E7ICE3U3OYGJX26LE6MSJVXLAMFI3XFFKD74ABA \
YT_ID=CBGIXGV2PL7ECJT5EFWULMTFLG5TT3YML33JULCAWVVDKERS7PMBAI7Y \
MANAGER_ID=CAC53MLBMELUS4JKHFS2HNAKXONAV2SKU2V7Z4ZFJVNJDLRHQ23OW7M6 \
POOL_ID=CCNWLZFDPQ556C7OZEUW6577FSRVCW5LLV4JAJOLB5HW5OGVC263TVPO \
USER_KEY=v11-carol DEPOSIT_AMOUNT=300000000 DO_SETTLE=0 \
./scripts/amm_lifecycle_demo.sh
```

| Step | Call | Result | Transaction |
|---|---|---|---|
| 1 | `sy.deposit(300 STA)` | 300,000,000 SY shares | [78aaf3fe...](https://stellar.expert/explorer/testnet/tx/78aaf3fee202373851ebd8188e30e27649cc33559f965980204221495c99554e) |
| 2 | `pm.mint(300,000,000 SY)` | 314,842,500 PT, 314,842,500 YT, 150,000 SY fee (0.05%) | [e6d6b731...](https://stellar.expert/explorer/testnet/tx/e6d6b731614f0ee5719addbdd5bd202fe20d8606cc00088f17af95ce1b287270) |
| 3a | `sy.deposit(150 STA)` (liquidity leg) | 150,000,000 SY shares | [2d18aca9...](https://stellar.expert/explorer/testnet/tx/2d18aca91755c58b56d19073ee31cd0ac7ce4232a80bff4d309705699fae758a) |
| 3b | `pool.add_liquidity(150,000,000 PT, ...)` | 150,000,000 PT + 68,504,731 SY used, 107,880,472 LP minted | [4d611b62...](https://stellar.expert/explorer/testnet/tx/4d611b62c70e63974420b379d25e906d8879640ec27a75e6e68f5ee423857c88) |
| 4 | `pool.swap_pt_for_sy(50,000,000 PT)` | 47,618,669 SY received | [06c114d4...](https://stellar.expert/explorer/testnet/tx/06c114d495615c2d99396cc17b61abd377d28f2c9f9b2f6e420034cb058cb7bc) |

Settlement and redemption (step 5, `DO_SETTLE=1`, only valid once `now >= maturity`) use the
identical `PrincipalManager.settle_all()` / `.redeem(from, pt_amount, yt_amount)` calls already
proven live on an earlier market created by this same factory in `docs/MARKET_FACTORY.md`'s
"Live Testnet proof" table (`settle_all` froze the rate at 10,500,000; `redeem` paid out
475,690,476 raw STA from the PT leg, 0 from the YT leg since the rate never moved between mint and
settlement) — the mechanics are identical regardless of which market or which deployment path
produced it.

**A real failure hit and fixed while producing this example:** the first attempt at step 4 failed
at submission with `InsufficientRefundableFee` (§2's second gotcha) — simulation had just run
`add_liquidity`, which writes new LP ledger entries, and the following `swap_pt_for_sy` call's own
simulated resource fee came in short of what submission actually needed. Fixed by setting
`--resource-fee` (not `--inclusion-fee`, which does nothing for this) to 5,000,000 stroops in the
script's `inv()` helper; confirmed by retrying the exact same call with that flag and watching it
succeed.

---

## 6. Error reference (master table)

Every contract's `#[contracterror]` enum, read from source. A dapp's error decoder should map
`(contractId, code)` to this table rather than guessing from the bare integer, since codes are
**not** unique across contracts (e.g. code 10 means `RecoveryEscrowAlreadySet` on `PTToken` but
`NothingToSeize` on `RecoveryEscrow`).

| Contract | Code | Error |
|---|---|---|
| OracleAdapter | 1–7 | AlreadyInitialized, Unauthorized, InvalidValue, TimestampTooOld, NotInitialized, ValueDecreased, TimestampInFuture |
| Permissioning | 1–3 | AlreadyInitialized, Unauthorized, NotInitialized |
| RiskControl | 1–13 | AlreadyInitialized, Unauthorized, NotInitialized, Paused, CircuitBreakerTripped, NotPauser, AlreadyPauser, NotConsumer, AlreadyConsumer, ZeroAmount, AssetLimitTripped, InvalidLimit, InvalidWindow |
| SYWrapper | 1–15 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, InsufficientShares, Paused, ArithmeticOverflow, PermissionDenied, NotAuthorizedOnSac, RecoveryEscrowAlreadySet, NotRecoveryEscrow, IssuerMismatch, SlippageExceeded, DepositCapExceeded, InvalidCap |
| PTToken | 1–13 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, InsufficientBalance, InsufficientAllowance, PermissionDenied, MinterAlreadySet, MinterNotSet, NotAuthorizedOnSac, IssuerMismatch, RecoveryEscrowAlreadySet, NotRecoveryEscrow |
| YTToken | 1–14 | same as PTToken, plus 10=OracleStale (shifting NotAuthorizedOnSac→11, IssuerMismatch→12, RecoveryEscrowAlreadySet→13, NotRecoveryEscrow→14) |
| MarketConfig | 1–7 | AlreadyInitialized, NotInitialized, Unauthorized, IssuerMismatch, FeeTooHigh, InvalidShare, NotApplicable |
| PrincipalManager | 1–15 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, NotMature, AlreadyMature, OracleStale, (8 unused), Paused, PermissionDenied, NotAuthorizedOnSac, IssuerMismatch, TopologyMismatch, NothingToClaim, ArithmeticOverflow |
| MarketPool | 1–21 | AlreadyInitialized, NotInitialized, Unauthorized, IssuerMismatch, Paused, Expired, ZeroAmount, SlippageExceeded, InsufficientLiquidity, InsufficientLpBalance, PermissionDenied, NotAuthorizedOnSac, OracleStale, MaturityTooFar, InvalidInitialRatio, MathError, RecoveryEscrowAlreadySet, NotRecoveryEscrow, NothingToClaim, InvalidStretch, MinimumLiquidity |
| Router | 1–8 | AlreadyInitialized, NotInitialized, Unauthorized, MarketNotRegistered, DeadlineExpired, SlippageExceeded, TopologyMismatch, AlreadyRegistered |
| RecoveryEscrow | 1–11 | AlreadyInitialized, NotInitialized, Unauthorized, TargetStillAuthorized, ZeroAmount, PositionUnderlyingMismatch, BatchTooLarge, RecordNotFound, AlreadyFinalized, NothingToSeize, NothingToFinalize |

Two codes an integrator will hit often, confirmed live in this guide: `MarketPool` error 6
(`Expired`) and `PrincipalManager` error 6 (`AlreadyMature`) both mean "this market's maturity has
passed" — expected behavior once `now >= maturity`, not a bug (see
`docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §2 for the live reproduction).

---

## 7. Scripts in this repository

| Script | Purpose |
|---|---|
| `scripts/create_market_via_factory.sh` | the three factory calls + wiring, parametrized (§3.4) |
| `scripts/amm_lifecycle_demo.sh` | deposit → mint → liquidity → swap → (optionally) settle → redeem (§5.3) |
| `scripts/quote_market.sh` | read-only AMM pricing, no signing key needed (§5.1) |
| `scripts/create_market.sh` | the fully-manual, pre-factory 11-step deployment (kept as a fallback / reference) |
| `scripts/check_market_status.sh` | live probe for whether a given market is open or matured |

---

## 8. Self-audit

This document was reviewed against the live network, not just reread. Specifically:

- Every function signature in §3–§5 was read from `contracts/*/src/lib.rs` in this commit, not
  from memory or from other docs — cross-checked against the actual `stellar contract invoke`
  calls that succeeded below.
- Every transaction hash in §3.4 and §5.3 was produced by actually running the cited script against
  Stellar Testnet while writing this guide (not reused from an earlier, different run), with the
  resulting contract addresses re-queried afterward to confirm they match what the script printed.
- Three real bugs were found and fixed in the process, not just in the prose:
  1. `scripts/create_market.sh` and the first draft of `scripts/create_market_via_factory.sh` had
     apostrophes inside `${VAR:?message}` parameter expansions, which breaks bash's parser even
     inside double quotes. The broken file (`create_market_via_factory.sh`, one such line) failed
     outright; `create_market.sh` (four such lines) happened to still parse because the apostrophe
     count was even, which is not a property anyone should rely on. Both are fixed; verified with
     `bash -n` on every script in `scripts/` afterward.
  2. `scripts/amm_lifecycle_demo.sh` fed a quoted JSON string (`"500000000"`) from one
     `stellar contract invoke` call straight into the next call's `i128` argument, which the CLI
     rejects. Fixed with a `raw()` helper; verified by rerunning the full script after the fix.
  3. The same script's swap step failed at submission with `InsufficientRefundableFee` after a
     preceding `add_liquidity` call. `--inclusion-fee` (tried first) did not fix it;
     `--resource-fee` did. Verified by retrying the exact failing call manually with each flag.
- `cargo fmt --all -- --check` and `cargo clippy -p principal_market_factory --all-targets --locked -- -D warnings` were not re-run for this document (it adds no Rust code), but `bash -n` was run on every modified/added script.
- What this guide does **not** verify live: `claim_yield`, `recombine`, the Router's flash-mint/flash-redeem paths (`swap_sy_for_yt`/`swap_yt_for_sy`), and `add_liquidity_single_sy`/`remove_liquidity` — these are documented from source (§4–§5) and are exercised in the repository's own Rust integration tests (`contracts/integration_tests/tests/`, `contracts/market_factory/tests/create_market.rs`), but not independently re-run against Testnet while writing this specific document. Treat §4's table as source-accurate and test-covered, not independently Testnet-verified the way §3.4/§5.3 are.
- `principal-dApp`'s source was read directly (`src/lib/*.ts`, `src/types.ts`,
  `src/config/markets.ts`) to ground §2–§4 and the §3.5 gap analysis in what that codebase actually
  does today, not a guess at what a "typical" dapp might do. Its one registered market
  (`sta-testnet-2026-10-04-m4`) is the market already shown matured/closed in
  `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §2 — that dapp has no live market to trade against
  until its registry is pointed at a fresh one (e.g. §3.4's) and extended with factory support
  (§3.5).

---

## 9. References

- `docs/MARKET_FACTORY.md` — factory design, the single-call design's real Testnet failure, API
- `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` — the matured-market investigation this guide
  continues from
- `docs/AMM_DESIGN.md` — the curve math, fixed-point arithmetic, rounding rules
- `docs/YIELD_MATH_AND_FEES.md` — settlement math with asserted test numbers
- `docs/API_REFERENCE.md` — every function of every contract, independent of this guide's examples
- `docs/DEPLOYMENT.md` — the full manual deployment procedure, §11 compliance-standing checklist
- `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` — the shared infrastructure this guide reuses
