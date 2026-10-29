# MarketFactory: Atomic Market Creation

`contracts/market_factory` adds an on-chain factory for Principal Protocol markets. It answers the
gap identified in `docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md` §1 and the deploy-time-atomicity
finding in `docs/TRANCHE_1_AUDIT.md`: before this contract, creating a market meant an operator
running roughly fifteen separate CLI transactions by hand (`docs/DEPLOYMENT.md`), with no atomicity
between them — exactly what produced the "superseded" half-wired markets in the 4 October 2026
Testnet run (`docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §3).

## Design

`MarketFactoryContract` is **stateless**: no admin, no `initialize`, no storage of its own. Every
deploy and wire-up it performs is gated by the *child* contract's own compliance check (reading the
underlying's live issuer authority), exactly as when those calls are made by hand — the factory
does not loosen that gate. Wasm hashes are passed as call arguments rather than stored in an
admin-settable registry, so there is no new mutable, front-runnable state to squat.

Each per-market contract is deployed with `env.deployer().with_current_contract(salt)` from
already-uploaded Wasm (`Deployer::upload_contract_wasm` or `stellar contract upload`), then
initialized and wired via ordinary cross-contract calls to the generated addresses — all inside one
Soroban transaction per phase. `predict_market(salt)` derives the same addresses with no deployment,
so a dapp can show them in a confirmation screen before the issuer signs anything.

### Three phases, not one

The original design was a single `create_market` call deploying and wiring all seven contracts
(`SYWrapper`, `PTToken`, `YTToken`, `MarketConfig`, `PrincipalManager`, `MarketPool`,
`RecoveryEscrow`) atomically. **This was tested against real Stellar Testnet and failed**: it is not
a theoretical concern or a test-harness artifact.

- Factory `CB2Z4QGKDFCVSB3SD5XQKGIOCL4KDNXEJQMHDTGK2ZBTJNR4FHRRKOCH` (single-call design, since
  superseded). A `create_market` invocation ran every deploy and wiring step correctly and computed
  the right result, but the *overall* invocation then failed host-side metering:
  `Error(Budget, ExceededLimit)` / `"Memory(OutOfBoundsGrowth)"`, reported while deploying the sixth
  of seven contracts (`MarketPool`). Because Soroban transactions are all-or-nothing, the entire
  transaction reverted — nothing was left on-chain. This happened on a from-scratch native SDK test
  too (67,027,032 bytes against a 41,943,040-byte ceiling), but the Testnet run is what confirms it's
  a property of a real transaction's resource budget, not an artifact of one long-lived test `Env`
  holding many contracts' Wasm instances resident at once.
- The fix: split `create_market` into three calls, each still fully atomic, each deploying and
  wiring roughly a third of the contracts. Factory
  `CCQQPZ63JQP63X2OP5REATX46CQMO7GTDRQBUHZYPGSMSBOHZII577ZZ` (current) ran all three phases
  successfully against the same real network, same underlying asset, same shared infrastructure:

| Phase | Deploys | Transaction |
|---|---|---|
| `create_tokens` | SYWrapper, PTToken, YTToken | [5897b204...](https://stellar.expert/explorer/testnet/tx/5897b2045b49c14bbe99dffcbc4c9da6e0fbba056babea04bc32e0ddf40a72ff) |
| `create_engine` | MarketConfig, PrincipalManager + minter/risk-control wiring | [aebba716...](https://stellar.expert/explorer/testnet/tx/aebba71605f91618d62187c51a4e885f9edd7c9fb3cc59fc8edba19ad9a4e434) |
| `create_pool_and_escrow` | MarketPool, RecoveryEscrow + recovery-escrow wiring | [20c27947...](https://stellar.expert/explorer/testnet/tx/20c27947d50b42d13043bb6b7eafa426dc51d86f58fa02b092f9d79ba85a09d8) |

Three transactions is still a large reduction from the ~15-20 run by hand today, and each of the
three remains internally atomic — a failure partway through any one phase rolls back that whole
phase, never leaving a half-wired contract on-chain.

### What the factory deliberately does not do

- **Does not deploy or own shared infrastructure** (`OracleAdapter`, `Permissioning`, `RiskControl`,
  `Router`). Those aren't maturity-bound and are meant to be reused across markets — pass their
  existing addresses in.
- **Does not call `RiskControl.add_consumer` or `Router.register_market`.** Both are gated by *that*
  contract's own stored admin, generally a different key from the issuer signing `create_*`. Keeping
  them out means every `create_*` call needs only the issuer's signature. The gap is harmless by
  design: an unregistered consumer makes `SYWrapper.deposit` / `PrincipalManager.mint` revert closed,
  never open.
- **Does not grant compliance standing.** Deciding who is eligible to hold the asset is the issuer's
  judgment call, not something a script should make silently.

The caller finishes those three steps (`docs/DEPLOYMENT.md` §11) right after `create_pool_and_escrow`
returns, exactly as with the pre-factory manual flow.

## Live Testnet proof (29 October 2026)

Using the existing shared infrastructure and underlying asset from the 4 October 2026 run
(`docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`) — `OracleAdapter`, `Permissioning`, `RiskControl`,
`Router`, and the STA SAC — a brand-new, third market was created entirely through the factory, then
actually used:

| Step | Result | Transaction |
|---|---|---|
| `create_tokens` | SY `CDMCDBBI...`, PT `CA3EGZQD...`, YT `CA7XZ6HR...` | [5897b204...](https://stellar.expert/explorer/testnet/tx/5897b2045b49c14bbe99dffcbc4c9da6e0fbba056babea04bc32e0ddf40a72ff) |
| `create_engine` | MarketConfig `CDQS6NKR...`, PrincipalManager `CCJWVPSO...` | [aebba716...](https://stellar.expert/explorer/testnet/tx/aebba71605f91618d62187c51a4e885f9edd7c9fb3cc59fc8edba19ad9a4e434) |
| `create_pool_and_escrow` | MarketPool `CB2QD6D3...`, RecoveryEscrow `CCMZZM2C...` | [20c27947...](https://stellar.expert/explorer/testnet/tx/20c27947d50b42d13043bb6b7eafa426dc51d86f58fa02b092f9d79ba85a09d8) |
| `risk_control.add_consumer` ×2, `router.register_market` | wired into shared infra | [ad7e3455...](https://stellar.expert/explorer/testnet/tx/ad7e3455f1dd474cdddde5ea3b0d67de0f87abde9a8573cefa2d9e27cfa38e76), [e825256e...](https://stellar.expert/explorer/testnet/tx/e825256e3d2d70edfb8617a26cef063de1e17509fad96762e0360142d209b636), [b88ce19a...](https://stellar.expert/explorer/testnet/tx/b88ce19aab1a6078b1921909f527eaf9d4e5471b752a131c272e7f0fd7ab0979) |
| Compliance standing granted to SY/PrincipalManager/Pool/RecoveryEscrow + Carol | all succeeded | (eleven grants; see the factory's own event log for each) |
| `sy.deposit` (100 STA) | 1,000,000,000 SY shares | [06e1aecf...](https://stellar.expert/explorer/testnet/tx/06e1aecf8788b0064f4ec6ed8f053cad68f1976f91c213f5734301bb0363e978) |
| `pm.mint` | 1,049,475,000 PT and YT (0.05% fee, 1.05 oracle rate) | [573bb764...](https://stellar.expert/explorer/testnet/tx/573bb764f929d7d080bf963fa7a8007d2449605bf19643c7232782fcf4624ddb) |
| `pool.add_liquidity` (500 PT, 400 SY) | 447,212,595 LP | (same session) |
| `pool.swap_pt_for_sy` (50 PT) | 47,618,889 SY received | [002fd27b...](https://stellar.expert/explorer/testnet/tx/002fd27b969c2b0bb4fcaf40666a2d5072369243a3e679c1e413d049738bcc9e) |
| `pm.settle_all` (after the 30-minute maturity passed) | rate frozen at 10,500,000 | [a2333bb7...](https://stellar.expert/explorer/testnet/tx/a2333bb738d5fa624da29f4144f9e42b7537f8c76a496d209b5b72fcf4712330) |
| `pm.redeem` (499,475,000 PT, 1,049,475,000 YT) | 475,690,476 raw STA from PT, 0 from YT | [b8c29dcb...](https://stellar.expert/explorer/testnet/tx/b8c29dcb7ab5e8b2663ac5cbce84d096aadfb202bf27e29dd919a4a896032396) |

The YT leg paid 0 correctly, not as a bug: the oracle rate never moved between mint (1.05) and
settlement (1.05), and the yield formula `N_yt × (1/r_settle − 1/r_now)` is exactly zero when those
two rates are equal. This market's maturity was deliberately short (30 minutes, matching the 4
October run's rationale) specifically so the full lifecycle — deposit, mint, add liquidity, swap,
settle, redeem — could be exercised live in one sitting, on a market the factory created from
nothing. Every step above ran against the real compiled contracts on Stellar Testnet; none of it was
simulated or asserted from the local test alone.

## API

See `contracts/market_factory/src/lib.rs` for full doc comments. Summary:

- `predict_market(salt) -> MarketContracts` — pure, no auth, no deployment.
- `create_tokens(issuer, salt, TokenWasmHashes, TokenParams) -> MarketTokens`
- `create_engine(issuer, salt, EngineWasmHashes, MarketTokens, EngineParams) -> MarketEngine`
- `create_pool_and_escrow(issuer, salt, PoolEscrowWasmHashes, MarketTokens, MarketEngine, PoolEscrowParams) -> MarketPoolAndEscrow`

The same `salt` must be passed to all three calls for one market. `issuer` must be the underlying's
live issuer authority and must sign every call.

## Tests

- `contracts/market_factory/src/test.rs` — pure-logic unit tests (address determinism, the auth
  gate); no Wasm build needed.
- `contracts/market_factory/tests/create_market.rs` — full cross-contract proof, against the real
  compiled contracts: all three phases, then a genuine deposit → mint → add-liquidity → swap →
  settle → redeem on the resulting market, plus proof market #1 (from the same test's `Stack`
  fixture) is unaffected. Needs `cargo build --workspace --release --target wasm32v1-none` first;
  skips otherwise.
