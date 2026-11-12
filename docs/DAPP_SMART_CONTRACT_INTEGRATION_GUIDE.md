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

A third gotcha, the most important one for anyone *writing new Router-style composed flows*
rather than just calling the existing ones: **never chain a `MarketPool` swap's payout into a
second call that needs fresh `from.require_auth()` on that exact amount.** `MarketPool`'s price
depends on `seconds_to_maturity`, read live from the ledger, so the exact payout can differ by a
few raw units between a client's pre-submission simulation and real execution moments later.
Soroban's pre-signed authorization for the second call must match its real arguments exactly, so
passing the swap's output as that call's amount reverts `Error(Auth, InvalidAction)` — not because
anything about the trade was wrong, but because the authorization baked in at simulation time is
already stale by the time the transaction lands. This broke a first version of
`Router.swap_pt_for_underlying` (swap PT→SY, then `SYWrapper.withdraw(from, sy_out, ...)`) live on
Testnet; §4.1 has the full story, the fix (`SYWrapper.withdraw_all`, which takes no amount argument
at all — it reads the caller's balance at execution instead), and the live transactions proving it.
The same risk applies to *any* value a contract computes mid-transaction from time-dependent state
(not just this protocol's curve) that gets reused as a further authorized argument — if you hit
`Error(Auth, InvalidAction)` on a composed call that worked in isolation, suspect this before
anything else.

A fourth gotcha, found live while producing `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md`'s
feature campaign: **the built-in Stellar Asset Contract and this protocol's own `PTToken`/
`YTToken`/`SYWrapper` name their balance-query parameter differently.** The SAC's `balance`
function takes `id`; this protocol's own tokens take `account`. Passing the wrong name is a
client-side argument error, not a contract revert, and — unlike most mistakes here — it fails
*silently* into an empty/undefined result rather than an error a caller is likely to check for,
which then poisons whatever arithmetic consumes it (observed directly: an empty balance read used
in a sell-half calculation). Always confirm which contract you're querying before reusing a
balance-reading helper across both.

A fifth gotcha, same source: **a flow that both mints *and* provides liquidity in one script needs
two separate SY deposits, not one.** `PrincipalManager.mint` takes custody of the *entire*
`sy_shares` argument passed to it; depositing once and minting with the full resulting balance
leaves nothing for an immediately-following `add_liquidity`/`add_liquidity_single_sy` call, which
then reverts `InsufficientShares` (SYWrapper) or `InsufficientLiquidity` (MarketPool, on an
unseeded pool) — not a contract bug, exactly the correct response to spending the same SY twice,
but an easy trap when composing a demo or a setup script. Budget a dedicated deposit for the
mint leg and a separate one for the liquidity leg, in that order — see
`docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §6 steps 2–5 for the corrected pattern.

A sixth gotcha, found live while producing §9 of that same evidence doc: **the underlying asset's
compliance authorization is global across every market, not per-market.** Deauthorizing a user
(the first step of a compliance-recovery `seize`) flips the Stellar Asset Contract's
`set_authorized` flag for that account on the underlying asset itself — it is not scoped to the
market that triggered it. A user recovered/seized on one market stays deauthorized on every other
market built on the same underlying asset until an issuer explicitly re-authorizes them; a dApp
that reuses one test/demo underlying across many markets needs to track this centrally (one
authorization flag per account, not one per market-membership row) rather than assuming recovery
on market A leaves a user untouched on market B.

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
| `swap_underlying_for_pt` *(new, §4.1)* | `Router.swap_underlying_for_pt(from, pool, underlying_amount, min_pt_out, deadline)` | `SYWrapper.deposit` → `MarketPool.swap_sy_for_pt` |
| `swap_pt_for_sy` | `Router.swap_pt_for_sy(from, pool, pt_in, min_sy_out, deadline)` | `MarketPool.swap_pt_for_sy` |
| `swap_pt_for_underlying` *(new, §4.1)* | `Router.swap_pt_for_underlying(from, pool, pt_in, min_underlying_out, deadline)` | `MarketPool.swap_pt_for_sy` → `SYWrapper.withdraw_gained` |
| `swap_sy_for_yt` (flash-mint, capital up front) | `Router.swap_sy_for_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` | `PrincipalManager.mint` → `MarketPool.swap_pt_for_sy` |
| `swap_underlying_for_yt` *(new, §4.1)* | `Router.swap_underlying_for_yt(from, pool, underlying_amount, sy_in, min_yt_out, max_net_cost, deadline)` | `SYWrapper.deposit` → `PrincipalManager.mint` → `MarketPool.swap_pt_for_sy` → `SYWrapper.withdraw_gained` |
| `flash_mint_yt` (**true flash-mint**) *(new, §4.3)* | `Router.flash_mint_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` | `MarketPool.flash_mint_yt` |
| `flash_mint_yt_from_underlying` *(new, §4.3)* | `Router.flash_mint_yt_from_underlying(from, pool, sy_in, min_yt_out, max_net_cost, underlying_budget, min_underlying_back, deadline)` | `SYWrapper.deposit` → `MarketPool.flash_mint_yt` → `SYWrapper.withdraw_gained` |
| `swap_yt_for_sy` (flash-redeem) | `Router.swap_yt_for_sy(from, pool, yt_in, min_sy_out, deadline)` | `MarketPool.swap_yt_for_sy` |
| `swap_yt_for_underlying` *(new, §4.1)* | `Router.swap_yt_for_underlying(from, pool, yt_in, min_underlying_out, deadline)` | `MarketPool.swap_yt_for_sy` → `SYWrapper.withdraw_gained` |
| `add_liquidity` | `Router.add_liquidity(from, pool, pt_in, sy_in, min_lp_out, deadline)` | `MarketPool.add_liquidity` |
| `add_liquidity_single_sy` | `Router.add_liquidity_single_sy(from, pool, sy_in, min_lp_out, deadline)` | `MarketPool.add_liquidity_single_sy` |
| `add_liquidity_single_underlying` *(new)* | `Router.add_liquidity_single_underlying(from, pool, underlying_amount, sy_in, min_lp_out, deadline)` | `SYWrapper.deposit` → `MarketPool.add_liquidity_single_sy` |
| `add_liquidity_keep_yt` *(new, §4.1)* | `Router.add_liquidity_keep_yt(from, pool, underlying_amount, mint_amount, pt_desired, sy_desired, min_lp_out, deadline)` | `SYWrapper.deposit` → `PrincipalManager.mint` → `MarketPool.add_liquidity` |
| `remove_liquidity` | `Router.remove_liquidity(from, pool, lp_in, min_pt_out, min_sy_out, deadline)` | `MarketPool.remove_liquidity` |
| `remove_liquidity_to_underlying` *(new)* | `Router.remove_liquidity_to_underlying(from, pool, lp_in, min_underlying_out, deadline)` | `MarketPool.remove_liquidity` → `MarketPool.swap_gained_pt_for_sy` → `SYWrapper.withdraw_gained` |
| `recombine` | `Router.recombine(from, pool, amount, min_sy_out, deadline)` | `PrincipalManager.recombine` |
| `claim_yield` | `Router.claim_yield(from, pool)` | `PrincipalManager.claim_yield` |
| `redeem_at_maturity` | `Router.redeem_at_maturity(from, pool, pt_amount, yt_amount)` | `PrincipalManager.redeem` |

`claim_yield` and `redeem_at_maturity` take no `deadline` — correct, not an omission:
`PrincipalManager.redeem`/`claim_yield` have no slippage window to bound (settlement is already
rate-frozen, or the call is balance-only).

The Router never custodies funds and is never itself compliance-gated; every downstream check runs
against `from`, the real user, so a deauthorized user gains nothing by routing through it.

### 4.1 Underlying-denominated trading and "keep YT" liquidity

Added directly in response to the dApp team's integration feedback: today's dApp has to chain 2–3
wallet signatures to pay in the yield-bearing underlying (USDY, BENJI) — wrap, then swap — because
the original Router only took raw underlying in `wrap_and_mint`; every other function needed SY,
PT or YT already in hand. The five rows marked *(new)* above close that gap, plus the
`add_liquidity_single_sy` fix below, which was independently reported as failing outright.

**`swap_underlying_for_pt` / `swap_underlying_for_yt` do not remove the capital requirement
`swap_sy_for_yt` already had.** `swap_underlying_for_yt` still needs `from` to hold the *full*
`underlying_amount` being tokenized up front, not just the eventual net cost — buying YT at 45×
leverage to spend 100 USDY still means holding ~4,500 USDY during the call. The Router sequences
existing calls inside one atomic transaction; it does not advance capital. **This limitation is
now optional, not fundamental**: §4.3 adds `flash_mint_yt`/`flash_mint_yt_from_underlying`, a true
flash swap where the pool fronts the capital, built after this guide was first written. Both forms
stay available — `swap_sy_for_yt`/`swap_underlying_for_yt` are unchanged.

**The real bug, found live while adding these:** a first version of `swap_pt_for_underlying`
(`MarketPool.swap_pt_for_sy` then `SYWrapper.withdraw(from, sy_out, from, min_underlying_out)`,
`sy_out` being the swap's own curve-derived payout) reverted at submission on real Testnet — not
at simulation, which succeeded — with a bare `InvokeHostFunction(Trapped)`. §2's third gotcha
explains the mechanism: `sy_out` is computed from `seconds_to_maturity`, which ticks forward
between a client's local simulation and real on-chain execution, so the pre-signed authorization
for `withdraw`'s exact `sy_out` argument no longer matched by the time the transaction landed.

The fix, applied to `swap_pt_for_underlying`, `swap_yt_for_underlying`, and the final leg of
`swap_underlying_for_yt`: a new `SYWrapper.withdraw_all(from, to, min_underlying_out)` that takes
**no** amount argument at all — it reads `from`'s current SY balance live, inside its own
execution, rather than taking a pre-computed one as input. With no curve-derived value left in the
authorized invocation, there is nothing to go stale. The tradeoff: it withdraws *everything*, so
these three functions now revert `PreexistingSyBalance` (Router error 9) if `from` already holds
any SY before the call — a precondition the dApp should check (or just satisfy, by never leaving a
user mid-flow with stray SY) rather than something a human needs to manually clear.

`add_liquidity_keep_yt` had the same class of problem one step earlier: it originally computed the
mint/liquidity split by calling `MarketPool.zap_swap_amount` *inside* its own execution and feeding
that curve-derived value straight into `PrincipalManager.mint`'s authorized `sy_shares` argument —
same staleness risk, different call. The fix there is different because `mint` has no "take
everything" equivalent that would make sense (it's a partial split, not a full withdrawal):
`mint_amount` is now a plain, caller-supplied argument, quoted off-chain before building the
transaction — exactly how every other `min_*_out` in this Router is already handled — and passed
in as a literal, signed value that cannot drift.

**Update, a second dApp-integration review round, after the above had already shipped.** Three
further issues, all fixed in the same pass:
1. `add_liquidity_keep_yt` still had *two* derived values feeding `add_liquidity`'s own authorized
   call: `sy_remaining` (from `deposit`'s live-exchange-rate-dependent output) and
   `minted.pt_minted` (from `mint`'s live-oracle-rate-dependent notional) — the fix above only
   covered `mint`'s own `sy_shares` argument, not these two. Fixed by making `pt_desired` and
   `sy_desired` plain, caller-supplied arguments too (see the new signature below) — quoted via
   `quote_add_liquidity_keep_yt`, which does the exact same quoting a caller would otherwise do by
   hand. `swap_underlying_for_yt` had the identical problem (`sy_in` derived from `deposit`'s
   output, fed into `mint`'s authorized call) and is fixed the same way: `sy_in` is now a plain
   argument too.
2. `quote_add_liquidity_keep_yt`'s `mint_amount` was quoted via `MarketPool.zap_swap_amount` —
   the wrong formula for this flow. `zap_swap_amount` solves for *buying* PT from the pool at the
   curve price (`add_liquidity_single_sy`'s zap); Keep YT *mints* PT 1:1 (minus the tokenization
   fee) at the oracle rate instead, a different ratio, so the quoted split generally left SY
   stranded in the following `add_liquidity` call. Fixed with a new `MarketPool.keep_yt_split`
   that solves for the correct ratio directly — see `docs/ROUTER_QUOTES.md` for the full fix and
   the new `lp_estimate` field this quote now also returns.
3. `flash_mint_yt_from_underlying` left any unused wrap budget as plain SY in the caller's
   balance — which then blocked any further same-transaction call using `withdraw_all` (every
   other `_for_underlying` flow), reverting `PreexistingSyBalance` against the caller's own
   leftover surplus and costing an extra signature just to clear it. Fixed with a new
   `SYWrapper.withdraw_gained(from, baseline, to, min_underlying_out)`, which withdraws only the
   SY gained since a `baseline` the Router reads immediately before the wrap leg — so the surplus
   now comes back as underlying automatically, and (unlike `withdraw_all`) a caller who already
   holds unrelated SY does not need to clear it first.

Two gaps also closed in the same pass, reported alongside the above:
`add_liquidity_single_underlying` (wrap, then `add_liquidity_single_sy`, one signature — before
this, single-sided LP from the underlying needed a separate `deposit` call first) and
`remove_liquidity_to_underlying` (`remove_liquidity`, then sell the PT leg, then unwrap — one
signature in place of 2-3; see the "Update" below for how the sell/unwrap legs actually work
today). See the updated function table and `docs/API_REFERENCE.md`'s Router section for both
signatures.

**Update, a third dApp-integration review round, after the team studied this round's commits.**
Two real bugs, both found by the dApp team reading the actual contract code rather than taking an
earlier "can't reproduce" answer at face value, plus one real gap in `withdraw_gained`'s adoption:

1. **`add_liquidity_single_sy`'s PT refund reverted `PermissionDenied` for any caller lacking
   PTToken's per-asset grant** — the real cause behind the "1 in 12" failure report two rounds
   back. The earlier "can't reproduce it, the refund looks unconditional" answer was simply wrong:
   the refund *is* conditional (`if pt_refund > 0`), and `PTToken.transfer` checks the
   *recipient's* per-asset grant even for a single raw unit — a caller using this "LP only" zap
   specifically to avoid ever holding PT directly has no reason to have requested one. Fixed by
   **not** transferring the PT dust at all: it is folded back into the pool's own reserve instead
   (`pt1 + pt_used + pt_refund`, algebraically `pt_reserve - pt_used` — exactly as if only
   `pt_used` of the "bought" PT ever left the reserve ledger). The SY refund is untouched by this
   fix: SY carries no per-asset grant, only an account-level one, so it never had this risk. The
   same fix automatically covers `add_liquidity_single_underlying`, which composes this function.
2. **`withdraw_gained` existed but was only wired into one Router flow** (`flash_mint_yt_from_underlying`)
   while `swap_pt_for_underlying`, `swap_yt_for_underlying`, `swap_underlying_for_yt` and
   `remove_liquidity_to_underlying` still used the full-sweep primitives
   (`withdraw_all`/`swap_all_pt_for_sy`) behind a zero-pre-existing-balance precondition. The dApp
   team found this failed routinely in practice: the "LP only" zap and Keep YT deposits can
   themselves leave small PT/SY remainders in a wallet, and simply having bought PT on some
   earlier, unrelated trade is enough to be "holding PT" by definition — none of which has
   anything to do with the balance a composed call like `swap_pt_for_underlying` actually needs to
   unwrap. All four functions now read a `baseline` before their mutating step and sweep only the
   gain on top of it (`withdraw_gained`, and a new PT equivalent, `MarketPool.swap_gained_pt_for_sy`),
   so none of them need the zero-pre-existing-balance precondition anymore — Router errors 9/10
   (`PreexistingSyBalance`/`PreexistingPtBalance`) are gone (never returned, removed from the enum,
   left as a gap in the numbering).
3. **New defect found in `withdraw_gained` itself while fixing the above**:
   `flash_mint_yt_from_underlying` reverted `ZeroAmount` whenever the wrap produced no surplus over
   the real net cost — including the common case of `underlying_budget = 0` (the whole cost
   covered by SY the buyer already held) — forcing a caller to always over-budget by a margin as
   a workaround. Fixed at the source: `withdraw_gained`/`swap_gained_pt_for_sy` now tolerate a
   zero (or negative) gain gracefully, returning `0`, but still enforce a nonzero `min_*_out` as a
   `SlippageExceeded` revert — a zero gain is only silently fine if the caller did not actually
   require getting something back.

Not actioned, by the reporter's own framing: no deeper contract-level separation between "can
authorize trustlines" and "can create a market" (both gated on the real issuer by design) is
planned for Tranche 2 testnet — scoping what the faucet's own code calls is enough for now, with
the contract-level separation flagged as a mainnet consideration in `docs/DEPLOYMENT.md`'s "Key
roles" section. The previous round's `testnet.json` was also reported unusable (wrong field names,
missing wasm hashes, both markets already matured) — see `docs/DEPLOYMENT.md`'s "Testnet
manifest" section for the current status.

Live evidence for all five functions plus the `add_liquidity_single_sy` fix, each confirmed on a
fresh Testnet market created purely to reproduce and then fix these exact failures (predates the
"Update" above; see `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §10 for this round's live
evidence):

| Function | Result | Transaction |
|---|---|---|
| `add_liquidity_single_sy` (fixed) | 500,000,000 SY in, 3,535 refunded, 243,429,586 LP minted | [094e8b99...](https://stellar.expert/explorer/testnet/tx/094e8b9904dcd6b316bdccf7a73506ef0e76981521a2cedb17181c2caf51fe6e) |
| `swap_underlying_for_pt` | 500,000,000 raw underlying → 524,992,691 PT | [ea0d0125...](https://stellar.expert/explorer/testnet/tx/ea0d0125c161adae5fb2672822d22fb37b6dfb8800d1eeffc155f1377cdbbdf2) |
| `swap_pt_for_underlying` (fixed) | 100,000,000 PT → 95,236,204 raw underlying | [61bbc1d8...](https://stellar.expert/explorer/testnet/tx/61bbc1d8cf93d92e9b0b3cda1dd37a6b23f13a81cad218f44db9e10b9386f955) |
| `swap_underlying_for_yt` (fixed final leg) | 300,000,000 underlying → 314,842,500 YT, 299,838,073 underlying change | live, Testnet |
| `swap_yt_for_underlying` (fixed) | 50,000,000 YT → 2,707 raw underlying | live, Testnet |
| `add_liquidity_keep_yt` (fixed) | 100,000,000 underlying → 45,526,182 LP + 87,873,672 YT kept | [37045f59...](https://stellar.expert/explorer/testnet/tx/37045f59aa7a69f13fb39e86feff3dd2587f0deafb63c227d753c8265932e1a1) |

The exact commands, representative of all five (each just swaps the function name and args per
§4's table). Updated to the current signatures (`add_liquidity_keep_yt` now also takes
`pt_desired`/`sy_desired`; `quote_add_liquidity_keep_yt` now does the whole off-chain quote in one
call instead of a bare `zap_swap_amount`):

```bash
# swap_underlying_for_pt -- note --resource-fee: see §2's second gotcha.
stellar contract invoke --id "$ROUTER_ID" --source v11-carol --network testnet --resource-fee 5000000 \
  -- swap_underlying_for_pt --from "$CAROL" --pool "$POOL_ADDR" \
     --underlying_amount 500000000 --min_pt_out 0 --deadline 9999999999

# add_liquidity_keep_yt -- mint_amount/pt_desired/sy_desired are all quoted off-chain first, via
# quote_add_liquidity_keep_yt (which itself calls MarketPool.keep_yt_split -- see §4.2/ROUTER_QUOTES.md).
Q=$(stellar contract invoke --id "$ROUTER_ID" --source v11-carol --network testnet \
  --send=no -- quote_add_liquidity_keep_yt --pool "$POOL_ADDR" --underlying_amount 100000000)
MINT_AMOUNT=$(echo "$Q" | python3 -c 'import json,sys; print(json.load(sys.stdin)["mint_amount"])')
PT_DESIRED=$(echo "$Q" | python3 -c 'import json,sys; print(json.load(sys.stdin)["pt_desired"])')
SY_DESIRED=$(echo "$Q" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sy_desired"])')
stellar contract invoke --id "$ROUTER_ID" --source v11-carol --network testnet --resource-fee 5000000 \
  -- add_liquidity_keep_yt --from "$CAROL" --pool "$POOL_ADDR" \
     --underlying_amount 100000000 --mint_amount "$MINT_AMOUNT" \
     --pt_desired "$PT_DESIRED" --sy_desired "$SY_DESIRED" --min_lp_out 0 --deadline 9999999999
```

### 4.2 Quotes for the five underlying-denominated flows

Previewing any of the five functions above used to mean calling `MarketPool`'s own SY-denominated
quote and separately calling `SYWrapper.exchange_rate()` to convert by hand. Five new, read-only
`Router` functions compose both calls into one answer, each in the units the real call's caller
actually experiences — `quote_add_liquidity_keep_yt` is exactly the `Q=$(...)` call the
`add_liquidity_keep_yt` example above uses:

| Function | Mirrors | Returns |
|---|---|---|
| `quote_underlying_for_pt(pool, underlying_amount)` | `swap_underlying_for_pt` | `Quote{amount_out, amount_in, fee_shares}` (`amount_out` is PT) |
| `quote_pt_for_underlying(pool, pt_in)` | `swap_pt_for_underlying` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` |
| `quote_underlying_for_yt(pool, underlying_amount)` | `swap_underlying_for_yt` | `UnderlyingFlashMintQuote{yt_out, underlying_back, mint_fee_underlying, swap_fee_underlying}` |
| `quote_yt_for_underlying(pool, yt_in)` | `swap_yt_for_underlying` | `UnderlyingQuote{amount_out, amount_in, fee_underlying}` |
| `quote_add_liquidity_keep_yt(pool, underlying_amount)` | `add_liquidity_keep_yt`, end to end | `KeepYtQuote{mint_amount, pt_desired, sy_desired, lp_estimate, yt_out, mint_fee_underlying}` — `mint_amount`/`pt_desired`/`sy_desired` are exactly the values to pass as the real call's own arguments of the same names; `lp_estimate` previews `lp_minted` |
| `quote_add_liquidity_underlying(pool, underlying_amount)` *(new)* | `add_liquidity_single_underlying`, end to end | `SingleSidedUnderlyingQuote{sy_in, pt_used, sy_used, lp_minted}` — `sy_in` is exactly the value to pass as the real call's own `sy_in` argument. Named without "single" in it — the full name with the `quote_` prefix exceeds Soroban's 32-byte function-name limit. |
| `quote_lp_to_underlying(pool, lp_in)` *(new)* | `remove_liquidity_to_underlying`, end to end | `RemoveLiquidityUnderlyingQuote{pt_out, sy_out, swap_fee_shares, underlying_out}` — backed by a new `MarketPool.quote_lp_exit`, which prices the removal's PT leg against the reserves *after* the removal, not before it (a first version got this backwards and was caught by the exact-equality regression test, not inspection). |

Both found missing in a dApp-integration review ("no quote for the two new USDY LP functions") —
full story: `docs/ROUTER_QUOTES.md`'s "Update."

Full design, the exactness argument for why these need no "quote vs. live execution" drift caveat
of their own (beyond the usual seconds-to-maturity one every `MarketPool` quote already carries),
and the regression test proving it: `docs/ROUTER_QUOTES.md`.

### 4.3 True flash-mint for YT

Closes the capital-advance limitation §4.1 flags for `swap_sy_for_yt`/`swap_underlying_for_yt`:
`MarketPool.flash_mint_yt` funds the mint leg from the pool's **own** SY reserves instead of the
caller's, keeps the minted PT in reserves (exactly as if the pool had just bought it), and sends
the caller only the YT. The caller authorizes transferring `max_net_cost`, not the full `sy_in`
notional — verified live tokenizing a 50,000,000-SY-share notional for a caller holding only
5,000,000 SY. `quote_sy_for_yt` (§4.2/§5.1) already previews this exactly: its `sy_in - sy_back`
*is* `flash_mint_yt`'s `net_cost`.

| Dapp action | Router call | Downstream |
|---|---|---|
| `flash_mint_yt` | `Router.flash_mint_yt(from, pool, sy_in, min_yt_out, max_net_cost, deadline)` | `MarketPool.flash_mint_yt` |
| `flash_mint_yt_from_underlying` | `Router.flash_mint_yt_from_underlying(from, pool, sy_in, min_yt_out, max_net_cost, underlying_budget, min_underlying_back, deadline)` → `(yt_out, net_cost, underlying_back)` | `SYWrapper.deposit` → `MarketPool.flash_mint_yt` → `SYWrapper.withdraw_gained` |

`flash_mint_yt_from_underlying` wraps only `underlying_budget` of underlying into SY — not the full
notional, since that would defeat the point — so `underlying_budget` only needs to cover
`max_net_cost` SY shares at the live `SYWrapper.exchange_rate()`, a plain off-chain conversion like
every other `_for_underlying` caller already does. Any surplus is unwrapped back to underlying
before the call returns (`SYWrapper.withdraw_gained`, guarded by `min_underlying_back`) — **not**
left as SY. An earlier version left it as SY in the caller's own balance; a dApp-integration
review found this cost an extra signature before any further same-transaction call that, at the
time, still used the full-sweep `withdraw_all` (every other `_for_underlying` flow now also uses
`withdraw_gained` instead — see §4.1's third "Update" — so this is no longer an issue regardless).
`withdraw_gained` has no zero-pre-existing-SY precondition either way — a caller who already holds
unrelated SY does not need to clear it first — and, as of the third dApp-integration review round,
also tolerates a zero (or negative) gain gracefully rather than reverting `ZeroAmount`.

**Two real bugs found live while building this, both in the capital-advance mechanism itself, not
the curve:**

1. A first version transferred the mid-execution-computed `net_cost` directly as the caller's
   authorized amount. Reverted `InvokeHostFunction(Trapped)` at submission — the same
   `Error(Auth, InvalidAction)` staleness class as §2's third gotcha and the `add_liquidity_single_sy`
   fix in §5.2: `net_cost` depends on the live curve, so the pre-signed amount drifted by the time
   the transaction landed. Fixed the same way: the caller authorizes the full, fixed `max_net_cost`
   instead, and the function refunds the unused remainder once the real cost is known.
2. Soroban's "a contract's direct calls are always authorized" only covers the *immediate* hop.
   `MarketPool` calling `PrincipalManager.mint` is fine on its own, but `mint` in turn calls
   `SYWrapper.transfer` on *its own* frame — one hop further than self-authorization reaches.
   Reverted `Error(Auth, InvalidAction)` ("a contract earlier in the call stack") until
   `flash_mint_yt` added one explicit `env.authorize_as_current_contract` entry for that nested
   call.

Full accounting proof, the Sidereal mechanism this borrowed, and the live transaction evidence:
`docs/AMM_FLASH_SWAP_AND_SIDEREAL_REVIEW.md` §5.

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
| `quote_yt_for_sy(yt_in)` *(new, §5.4)* | `Quote { amount_out, fee_shares }` — flash-redeem preview: selling `yt_in` YT |
| `quote_sy_for_yt(sy_in)` *(new, §5.4)* | `FlashMintQuote { yt_out, sy_back, mint_fee_shares, swap_fee_shares }` — flash-mint preview: buying YT with `sy_in` SY |
| `zap_swap_amount(sy_in)` | how much of a single-sided SY deposit converts to PT (for `add_liquidity_single_sy`'s zap) |
| `keep_yt_split(sy_shares)` *(new)* | how much of `sy_shares` `Router.add_liquidity_keep_yt` should *mint* instead — a different ratio than `zap_swap_amount` above, since this leg mints PT at the oracle rate rather than buying it at the curve price |
| `quote_add_liquidity_single_sy(sy_in)` *(new)* | previews `add_liquidity_single_sy`'s own `(pt_used, sy_used, lp)` exactly |
| `quote_lp_exit(lp_in)` *(new)* | previews `remove_liquidity`'s split for `lp_in` plus selling the PT leg, correctly priced against the reserves *after* the removal |
| `reserves()` | `(pt_reserve, sy_reserve)` |
| `pool_state()`, `maturity()`, `is_paused()` | self-explanatory |

`principal-dApp/src/lib/quotes.ts` wires `quote_sy_for_pt`/`quote_pt_for_sy` today (the two
swap-quote actions); `quote_buy_exact_pt`, `quote_yt_for_sy`, `quote_sy_for_yt`, `zap_swap_amount`,
`pt_price`, and `implied_rate` are not wired yet — direct pricing/exact-output flows, the YT-trade
preview closed in §5.4, and a market-overview "current rate" display would need them added,
following the exact pattern `fetchMarketQuote` already uses.

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
| `swap_all_pt_for_sy` | `(from, to, min_sy_out) -> i128` — like `swap_pt_for_sy`, but sells `from`'s **entire** PT balance, read at execution time instead of taken as an argument. No longer used by any Router flow (see `swap_gained_pt_for_sy`), kept as a standalone primitive. |
| `swap_gained_pt_for_sy` *(new)* | `(from, to, baseline, min_sy_out) -> i128` — like `swap_all_pt_for_sy`, but sells only the PT gained *since* `baseline`, so pre-existing, unrelated PT survives untouched. Used by `Router.remove_liquidity_to_underlying`. Tolerates a zero (or negative) gain, returning `0`, unless `min_sy_out > 0` (then `SlippageExceeded`). |
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

**`add_liquidity_single_sy` was reported broken on real Testnet** (reproduced and confirmed: the
dApp team's own repro showed a signed amount of `9999979445` against an executed amount of
`9999979436` — nine raw units apart, enough to invalidate the signature) and has since been fixed.
Root cause: it used to transfer `split + sy_used` from the caller, both derived from the pool's
time-dependent curve state, so the amount Soroban authorized at simulation time could differ from
what real execution computed moments later — §2's third gotcha, manifesting *inside* `MarketPool`
itself rather than through Router composition. The fix takes the caller's full, fixed `sy_in` up
front instead and refunds whatever goes unused; confirmed live on Testnet
(`add_liquidity_single_sy(sy_in=500000000, ...)` → 500,000,000 taken, 3,535 refunded, 243,429,586
LP minted — [tx](https://stellar.expert/explorer/testnet/tx/094e8b9904dcd6b316bdccf7a73506ef0e76981521a2cedb17181c2caf51fe6e)).
The return values and economics are unchanged; only the internal transfer sequence is different,
so no existing integration needs to change anything about how it reads this function's outputs.

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

**Step 5 — settlement and redemption, on this exact market, once its maturity passed:**

| Step | Call | Result | Transaction |
|---|---|---|---|
| 5a | `pm.settle_all()` | rate frozen at 10,500,000 | [d8a04fbd...](https://stellar.expert/explorer/testnet/tx/d8a04fbdcbdf037d317f2e5e3ef138696595fd4a5d1603d62e76462d513cb13c) |
| 5b | `pm.redeem(594,317,500 PT, 1,364,317,500 YT)` | 566,016,666 raw STA from the PT leg, 0 from YT | [8dcb9659...](https://stellar.expert/explorer/testnet/tx/8dcb96599da4cab10507a0f76745fbdc56ca015ee86639b106535df754a3ea77) |

Step 5b's amounts are larger than step 2's 314,842,500 because this redeemed Carol's *entire*
accumulated position on this market — including PT/YT minted during an earlier, failed dry run of
this same script against this same market (before the `InsufficientRefundableFee` fix below), not
only the clean run shown in steps 1–4. The YT leg paying exactly 0 is correct, not a bug: the
oracle rate never moved between any of those mints (1.05) and settlement (1.05), and
`N_yt × (1/r_settle − 1/r_now)` is exactly zero when the two rates are equal — the identical result
already shown on a different market in `docs/MARKET_FACTORY.md`'s "Live Testnet proof" table, now
reconfirmed on this one.

**A real failure hit and fixed while producing this example:** the first attempt at step 4 failed
at submission with `InsufficientRefundableFee` (§2's second gotcha) — simulation had just run
`add_liquidity`, which writes new LP ledger entries, and the following `swap_pt_for_sy` call's own
simulated resource fee came in short of what submission actually needed. Fixed by setting
`--resource-fee` (not `--inclusion-fee`, which does nothing for this) to 5,000,000 stroops in the
script's `inv()` helper; confirmed by retrying the exact same call with that flag and watching it
succeed.

### 5.4 Pricing YT trades: closing the "RouterStatic" gap

Bastien's reply in the dApp developers' thread asked for the Pendle-style `RouterStatic`
pattern — a read-only preview of slippage, resulting APY, amount received, and fees, shown to the
user *before* they sign — and pointed out Principal had nothing equivalent for YT. That's
confirmed by `principal-dApp/src/lib/quotes.ts`'s own comment: *"Until a dedicated quote is wired
for this action, reuse the same conservative 'no usable quote -> 0' fallback as min_yt_out"* — the
dapp was quoting PT trades (`quote_sy_for_pt`/`quote_pt_for_sy`) but had no way to quote either YT
flow (`swap_sy_for_yt`'s flash-mint or `swap_yt_for_sy`'s flash-redeem) at all.

Two new, read-only `MarketPool` functions close this:

- **`quote_yt_for_sy(yt_in) -> Quote`** previews `swap_yt_for_sy` (flash-redeem): recombining
  `yt_in` YT with `yt_in` PT pulled from the pool's own reserve, at the live oracle rate, minus the
  curve-priced cost of pulling that PT. Replicates `swap_yt_for_sy`'s math exactly — same
  `calc_buy_exact_pt` call, same `recombined = yt_in * SCALE / rate` formula `PrincipalManager.recombine`
  itself uses — rather than calling the real swap and discarding its effects, since the quote must
  stay read-only (no `require_auth`, no state change).
- **`quote_sy_for_yt(sy_in) -> FlashMintQuote`** previews `swap_sy_for_yt` (flash-mint):
  tokenizing `sy_in` SY into PT+YT at the live oracle rate and `MarketConfig`'s tokenization fee,
  then selling the minted PT back into the same pool at the live curve price. `FlashMintQuote`
  reports the mint fee and the swap fee separately (`mint_fee_shares`, `swap_fee_shares`), not just
  a net number, so a dapp can show the user both legs' cost the way Bastien's request asked for.
  Replicates `PrincipalManager.mint`'s exact ceil-rounded-fee, floor-rounded-notional formula
  in-contract rather than calling it.

**Both reverts match their real swap exactly, not just their happy path** — proven live, not just
in the Rust test suite (`contracts/integration_tests/tests/amm.rs`,
`quote_yt_for_sy_reverts_the_same_way_the_real_swap_does`): a YT position priced at near-zero (rate
above 1.0 and the curve close enough to par that recombining can't cover the PT it must buy back)
makes `quote_yt_for_sy` revert `InsufficientLiquidity` (pool error 9) exactly like `swap_yt_for_sy`
itself would — confirmed on a fresh market (`CDRDFQ7IHM6K6II7BDWLQCF4L3UWG6S4GTAPRB7XKBIOOS3J6YH7O367`,
30-minute maturity, 1.05 oracle rate): both `quote_yt_for_sy(1000000)` and the real
`Router.swap_yt_for_sy(1000000, ...)` reverted with the identical contract error, at simulation, so
no transaction hash exists for either (correct — neither should have submitted).

**Getting a genuinely profitable flash-redeem to demonstrate the happy path took two more fresh
markets, and that itself is a real finding worth recording.** A 1.05 oracle rate shrinks
`recombine`'s payout by dividing by 1.05, while a shallow discount (PT seeded at a 0.95 SY/PT
ratio) only lowers the cost of buying back the PT by a few percent — not enough margin to clear
that division. A second market seeded with the same 0.95 ratio but a 180-day maturity hit the same
wall (`quote_yt_for_sy` and the real swap both reverted `InsufficientLiquidity` again, for the same
reason, just confirmed twice). Only a third market, seeded at a steeper 0.80 PT/SY ratio
(`add_liquidity(pt_desired=100000000, sy_desired=80000000, ...)`), left enough margin for a small
trade to clear it. This is not a quote bug — it is the real economics `PrincipalManager.recombine`
enforces, and matches the already-existing Rust regression
`flash_redeem_reverts_when_the_yt_cannot_cover_the_pt_it_consumes` — but it means a dapp's YT-sell
UI should expect `quote_yt_for_sy` to legitimately return "no, you'd lose money" close to maturity
whenever the oracle rate has moved enough, and should surface that rather than treat it as an
error state.

Live evidence, all three calls on market `CCXMGKRUMRHXJUQ66HN4Y6U2LKWYRLUQO75OIROWOYNXL6OE2KS32NI2`
(180-day maturity, seeded 100,000,000 PT / 80,000,000 SY, oracle rate 1.05):

| Call | Quoted | Real result | Transaction |
|---|---|---|---|
| `quote_yt_for_sy(1000000)` then `Router.swap_yt_for_sy(1000000, ...)` | `{"amount_out":"18544","fee_shares":"461"}` | `18544` SY paid out | [0a1d5918...](https://stellar.expert/explorer/testnet/tx/0a1d5918674e12ef6f3ea5c6a701ca0bb1c808ce90e757aeda1eb91d3663f1a3) |
| `quote_sy_for_yt(15000000)` then `Router.swap_sy_for_yt(15000000, ...)` | `{"yt_out":"15742125","sy_back":"14399757",...}` | `yt_out=15742125`, `sy_back=14399758` | [5ae51df2...](https://stellar.expert/explorer/testnet/tx/5ae51df2fd07b5ec21fc9dbadfc20a3d4df6732bbd1362646fcfe6cc7a1c4a11) |

`quote_yt_for_sy`'s payout matched the real swap bit-for-bit. `quote_sy_for_yt`'s `yt_out` (the
mint leg, priced off the oracle rate, which does not move between two calls seconds apart) also
matched exactly; `sy_back` (the sell-PT leg, priced off the time-decaying curve) was one raw unit
off — the quote was simulated a few seconds, and one ledger close, before the real swap executed,
and `seconds_to_maturity` ticked down by that much in between. This is the same class of
quote-vs-execution drift as §2's third gotcha, not a flaw in the quote math: the Rust test suite's
`quote_sy_for_yt_matches_the_real_flash_mint_outcome_exactly` asserts a bit-for-bit match because
there the quote and the swap execute inside the same simulated ledger, with no real time passing
between them — a guarantee only a single atomic call (not two separate client round-trips) can
give on live Testnet. A dapp quoting a flash-mint SY-back amount should treat it as indicative
down to the last one or two raw units, exactly like every other `min_*_out` parameter in this
Router, rather than assume byte-for-byte equality with whatever ledger the swap lands in.

```bash
# Both quotes, read-only, no signature needed beyond a throwaway simulation account:
./scripts/quote_market.sh "$POOL_ID" v11-carol 0 0 1000000 15000000
```

---

## 6. Error reference (master table)

Every contract's `#[contracterror]` enum, read from source. A dapp's error decoder should map
`(contractId, code)` to this table rather than guessing from the bare integer, since codes are
**not** unique across contracts (e.g. code 10 means `RecoveryEscrowAlreadySet` on `PTToken` but
`NothingToSeize` on `RecoveryEscrow`).

| Contract | Code | Error |
|---|---|---|
| OracleAdapter (markets created with the §3.4/§4.1/§5.4 WASM hashes — Tranche 1 ABI) | 1–7 | AlreadyInitialized, Unauthorized, InvalidValue, TimestampTooOld, NotInitialized, ValueDecreased, TimestampInFuture |
| OracleAdapter (markets created with the extended oracle WASM, `docs/ORACLE_SAFETY_EXTENSION.md`) | 1–13 | same 1–7, plus OracleUnavailable(8), DeviationTooHigh(9), InvalidSource(10), InvalidDeviation(11), InvalidStaleness(12), FallbackNotConfigured(13) — **and** `initialize`/`set_reference_value` take new required arguments, a breaking ABI change requiring a freshly deployed OracleAdapter instance, not a retrofit |
| Permissioning | 1–3 | AlreadyInitialized, Unauthorized, NotInitialized |
| RiskControl | 1–13 | AlreadyInitialized, Unauthorized, NotInitialized, Paused, CircuitBreakerTripped, NotPauser, AlreadyPauser, NotConsumer, AlreadyConsumer, ZeroAmount, AssetLimitTripped, InvalidLimit, InvalidWindow |
| SYWrapper | 1–15 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, InsufficientShares, Paused, ArithmeticOverflow, PermissionDenied, NotAuthorizedOnSac, RecoveryEscrowAlreadySet, NotRecoveryEscrow, IssuerMismatch, SlippageExceeded, DepositCapExceeded, InvalidCap |
| PTToken | 1–13 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, InsufficientBalance, InsufficientAllowance, PermissionDenied, MinterAlreadySet, MinterNotSet, NotAuthorizedOnSac, IssuerMismatch, RecoveryEscrowAlreadySet, NotRecoveryEscrow |
| YTToken | 1–15 | same as PTToken, plus 10=OracleStale (shifting NotAuthorizedOnSac→11, IssuerMismatch→12, RecoveryEscrowAlreadySet→13, NotRecoveryEscrow→14), plus 15=ArithmeticOverflow (the pending-yield calculation, on an overflowing balance) |
| MarketConfig | 1–7 | AlreadyInitialized, NotInitialized, Unauthorized, IssuerMismatch, FeeTooHigh, InvalidShare, NotApplicable |
| PrincipalManager | 1–15 | AlreadyInitialized, Unauthorized, NotInitialized, ZeroAmount, NotMature, AlreadyMature, OracleStale, (8 unused), Paused, PermissionDenied, NotAuthorizedOnSac, IssuerMismatch, TopologyMismatch, NothingToClaim, ArithmeticOverflow |
| MarketPool | 1–21 | AlreadyInitialized, NotInitialized, Unauthorized, IssuerMismatch, Paused, Expired, ZeroAmount, SlippageExceeded, InsufficientLiquidity, InsufficientLpBalance, PermissionDenied, NotAuthorizedOnSac, OracleStale, MaturityTooFar, InvalidInitialRatio, MathError, RecoveryEscrowAlreadySet, NotRecoveryEscrow, NothingToClaim, InvalidStretch, MinimumLiquidity |
| Router | 1–8 | AlreadyInitialized, NotInitialized, Unauthorized, MarketNotRegistered, DeadlineExpired, SlippageExceeded, TopologyMismatch, AlreadyRegistered. Codes 9/10 (`PreexistingSyBalance`/`PreexistingPtBalance`) existed briefly and are now unused/removed: a later dApp-integration review found the functions that returned them (`swap_pt_for_underlying`/`swap_yt_for_underlying`/`swap_underlying_for_yt`/`remove_liquidity_to_underlying`) failed routinely for any caller who simply already held PT or SY, and switched them to a baseline/gain-based sweep (`withdraw_gained`/`swap_gained_pt_for_sy`) that no longer needs a zero-pre-existing-balance precondition at all. |
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
| `scripts/quote_market.sh` | read-only AMM pricing, no signing key needed, including the YT quotes (§5.1, §5.4) |
| `scripts/create_market.sh` | the fully-manual, pre-factory 11-step deployment (kept as a fallback / reference) |
| `scripts/check_market_status.sh` | live probe for whether a given market is open or matured |

---

## 8. Self-audit

This document was reviewed against the live network, not just reread. Specifically:

- Every function signature in §3–§5 was read from `contracts/*/src/lib.rs` in this commit, not
  from memory or from other docs — cross-checked against the actual `stellar contract invoke`
  calls that succeeded below.
- Every transaction hash in §3.4, §4.1 and §5.3 was produced by actually running the cited script
  or command against Stellar Testnet while writing this guide (not reused from an earlier,
  different run), with the resulting contract addresses re-queried afterward to confirm they match
  what was printed.
- Five real bugs were found and fixed in the process, not just in the prose:
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
  4. `MarketPool.add_liquidity_single_sy` reverted `Error(Auth, InvalidAction)` on real Testnet
     (dApp team's own report, with a concrete signed-vs-executed amount mismatch) — fixed by
     taking the caller's full `sy_in` up front instead of a curve-derived amount, confirmed live
     post-fix (§5.2).
  5. A first version of `Router.swap_pt_for_underlying` reverted `InvokeHostFunction(Trapped)` at
     submission (simulation succeeded) for the same underlying reason as #4, but through Router
     composition rather than inside `MarketPool` — fixed with a new `SYWrapper.withdraw_all` and,
     for `add_liquidity_keep_yt`, a caller-supplied `mint_amount` instead of a mid-execution
     `zap_swap_amount` call (§2's third gotcha, §4.1).
- For the contract-level changes (fixes #4 and #5, plus the five new Router functions,
  `SYWrapper.withdraw_all`, and the two new `MarketPool` quote functions in §5.4): `cargo fmt --all
  -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test
  --workspace` were all run clean after every change, not just once at the end — including new
  regression tests (`single_sided_deposit_leaves_no_residual_sy_stranded_in_the_pool`,
  `withdraw_all_takes_the_entire_balance` and its two `#[should_panic]` siblings,
  `router_underlying_denominated_flows_and_keep_yt_work_end_to_end`,
  `quote_yt_for_sy_matches_the_real_flash_redeem_payout_exactly`,
  `quote_yt_for_sy_reverts_the_same_way_the_real_swap_does`, and
  `quote_sy_for_yt_matches_the_real_flash_mint_outcome_exactly`). `bash -n` was run on every
  modified/added script.
- §5.4's two quote functions needed three fresh markets to verify live, not one: the first two
  (30-minute and 180-day maturity, both seeded at a 0.95 PT/SY ratio) could only demonstrate
  `quote_yt_for_sy`'s revert path — a genuinely profitable flash-redeem needed a third, seeded at a
  steeper 0.80 ratio, to leave enough margin against the 1.05 oracle rate. That took longer than
  expected and is recorded as a finding in §5.4 itself, not smoothed over here. The one-raw-unit
  gap between `quote_sy_for_yt`'s quoted `sy_back` and the real swap's (§5.4) is real, reproduced
  exactly once, and is attributed to the ledger close between the two separate calls — not
  re-attempted dozens of times to see if it is consistent, since the mechanism (§2's third gotcha)
  already explains it and the Rust test suite proves the underlying formula matches bit-for-bit
  when no time passes between quote and execution.
- §4.2's five Router-level quotes were deliberately **not** verified against a fresh live
  deployment (every other change in this document was) — the reasoning, not just the choice, is in
  `docs/ROUTER_QUOTES.md` §4: each is a read-only composition of two primitives already verified
  live independently (`MarketPool`'s quotes, `SYWrapper.exchange_rate()`), and the same doc's §2
  proves exactness by induction over every input the current `SYWrapper` contract can ever produce,
  not just the handful of values a live run would have sampled. The Rust regression
  (`router_quotes_for_underlying_denominated_flows_match_the_real_outcome_exactly`, alongside the
  existing `router_underlying_denominated_flows_and_keep_yt_work_end_to_end`) asserts exact
  equality against the real executed outcome, not an approximate check.
- `settle_all`/`redeem` (§5.3 step 5) were added after this market's maturity actually passed
  during the writing of this guide (not simulated, not assumed) — settled at 10,500,000, redeemed
  594,317,500 PT and 1,364,317,500 YT for 566,016,666 raw STA, 0 from the YT leg.
- `claim_yield`, `remove_liquidity` (full exit), `add_liquidity_single_sy`, direct
  `MarketPool.flash_mint_yt`, and both `Router.flash_mint_yt`/`flash_mint_yt_from_underlying` and
  `Router.add_liquidity_keep_yt` **have since been run live** against a fresh Testnet deployment —
  `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §4 and §6 have the full transaction-by-transaction
  record, including a real settle/redeem cycle and a full compliance-recovery
  (deauthorize → seize → finalize) cycle. `recombine` and the deadline/slippage guards *specifically
  tripping* (as opposed to the calls themselves succeeding) remain verified only via the repository's
  own Rust integration tests (`contracts/integration_tests/tests/`,
  `contracts/market_factory/tests/create_market.rs`), not independently re-run against Testnet.
- `principal-dApp`'s source was read directly (`src/lib/*.ts`, `src/types.ts`,
  `src/config/markets.ts`) to ground §2–§4 and the §3.5 gap analysis in what that codebase actually
  does today, not a guess at what a "typical" dapp might do. Its one registered market
  (`sta-testnet-2026-10-04-m4`) is the market already shown matured/closed in
  `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §2 — that dapp has no live market to trade against
  until its registry is pointed at a fresh one (e.g. §3.4's) and extended with factory support
  (§3.5), and its `TransactionIntent`/`protocol.ts` would need the five new actions from §4.1 added
  following the exact pattern already there for the existing ones.

**Addendum: a second dApp-integration review round**, after the dApp team studied the commits
from the round above and reported what still didn't work. Five real gaps confirmed against the
code as it stood (not taken on the report's word alone) and fixed, all documented inline in §4.1
and §5.2 above: `add_liquidity_keep_yt`'s and `swap_underlying_for_yt`'s remaining
`Error(Auth, InvalidAction)` exposure; `quote_add_liquidity_keep_yt`'s wrong-split formula;
`flash_mint_yt_from_underlying` stranding its surplus as SY; and no single-signature path for
single-sided-underlying liquidity add/remove. One claim (a refund-logic failure in
`add_liquidity_single_sy`, "1 of 12" in their testing) could not be reproduced by reading the
current code at the time — **this turned out to be a mistake on this project's side, corrected by
the dApp team in the next round below, not a dead end**: the refund genuinely is conditional, and
that condition is exactly where the bug lives. `cargo fmt --all -- --check`, `cargo clippy
--workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace` all pass clean,
and the new drift fixes were verified live, not just via the Rust test suite — see
`docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` §10.

**Addendum: a third dApp-integration review round**, after the dApp team read the actual contract
code (not just the test suite) and corrected this project's own prior analysis. Full narrative:
§4.1's "Update, a third dApp-integration review round" above. In short: `add_liquidity_single_sy`'s
PT refund reverted `PermissionDenied` for any caller lacking PTToken's per-asset grant (the real
mechanism behind the "1 in 12" report), fixed by not transferring the dust at all;
`withdraw_gained` existed but was wired into only one of five flows that needed it, so the other
four still rejected any caller holding pre-existing PT/SY, fixed by applying it everywhere plus a
new PT equivalent (`MarketPool.swap_gained_pt_for_sy`); and both gain-sweep primitives now
tolerate a zero/negative gain gracefully instead of reverting `ZeroAmount`, closing a defect
`flash_mint_yt_from_underlying` had whenever its budget left no surplus. New regression tests for
all three: `single_sided_deposit_succeeds_for_a_caller_with_no_pt_asset_grant`,
`underlying_denominated_router_flows_tolerate_pre_existing_pt_and_sy`,
`flash_mint_yt_from_underlying_tolerates_a_budget_with_no_surplus`,
`gained_sweeps_still_enforce_slippage_on_a_zero_gain`
(`contracts/integration_tests/tests/liquidity_and_router.rs`). Full test/clippy/fmt gate clean.

**Addendum: closing out the dApp team's reports.** The one remaining item from their feedback --
"no quote for the two new USDY LP functions," listed as minor -- is now closed too:
`quote_add_liquidity_underlying` and `quote_lp_to_underlying` (§4.2), backed by two new
`MarketPool` views, `quote_add_liquidity_single_sy` and `quote_lp_exit` (§5.1). Building
`quote_lp_exit` surfaced a real bug in its own first draft, caught by this doc's own
exact-equality testing discipline rather than by inspection: pricing the removal's PT leg against
the reserves *before* `remove_liquidity` ran, when the real sequence necessarily sells that PT
*after* the removal has already shrunk them. Fixed by having `quote_lp_exit` construct the
post-removal `PoolState` explicitly. Everything the dApp team identified across all three review
rounds is now either fixed, explicitly deferred by their own prioritization (post-trade price
impact in quotes, the PT/LP-as-collateral oracle), or settled by their own recommendation (no
contract-level issuer/admin separation for Tranche 2 testnet).

---

## 9. References

- `docs/AMM_FLASH_SWAP_AND_SIDEREAL_REVIEW.md` — the comparative AMM audit behind §4.3's true
  flash-mint, the two capital-flow bugs found and fixed while building it, and the live Testnet
  evidence
- `docs/ORACLE_SAFETY_EXTENSION.md` — the primary/fallback source, deviation guard, and
  availability-signaling extension to `OracleAdapter`, with live Testnet evidence and its own
  breaking-ABI migration note
- `docs/ROUTER_QUOTES.md` — the five Router-level quotes for the underlying-denominated flows in
  §4.1, and the proof that `SYWrapper.exchange_rate()` cannot drift within one of these calls under
  the current contract
- `docs/MARKET_FACTORY.md` — factory design, the single-call design's real Testnet failure, API
- `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` — the matured-market investigation this guide
  continues from
- `docs/AMM_DESIGN.md` — the curve math, fixed-point arithmetic, rounding rules
- `docs/YIELD_MATH_AND_FEES.md` — settlement math with asserted test numbers
- `docs/API_REFERENCE.md` — every function of every contract, independent of this guide's examples
- `docs/DEPLOYMENT.md` — the full manual deployment procedure, §11 compliance-standing checklist
- `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` — the shared infrastructure this guide reuses
- `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md` — a from-scratch redeployment and full feature
  campaign run after the 4 November 2026 audit, covering every flow in §4–§5 plus the extended
  `OracleAdapter`, `RiskControl`'s circuit breaker, and a full compliance-recovery and
  settle/redeem cycle, with six of the gotchas in §2 above found live while producing it (its §9
  also redeploys and live-verifies the one Medium from the later final closing-pass audit,
  `docs/AUDIT_2026_11_05_FINAL_REVIEW.md`)
