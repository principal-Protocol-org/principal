# Tranche 2 Deliverable 1 — Testnet Acceptance Evidence

Status: this documents the first live, end-to-end exercise of the new `scripts/deploy/deploy.sh` /
`scripts/deploy/verify-deployment.sh` pipeline and the extended, source-aware `OracleAdapter`
against Stellar Testnet, per `docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md` §5.7. It supersedes
nothing — the PT/YT/AMM feature campaign and compliance-recovery cycle this deployment's *contract
code* is identical to were already proven live in
[`docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md`](TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md); this
document's own new evidence is the deployment pipeline itself, its idempotency, the oracle safety
guards (push and the new pull path), and a live cross-contract proof of the new SEP-40 pull
mechanism. Everything below is from commands actually run this session; nothing is fabricated, and
every gap found is stated plainly rather than hidden.

**Working tree status at the time of this deployment**: dirty (`git_tree_dirty: true` in the
manifest, confirmed `FAIL` by `verify-deployment.sh`) — this phase's own code (the OracleAdapter
SEP-40 extension, the deployment scripts themselves, this document) was not yet committed when
these commands ran, since live-testing had to happen before anything could be committed as
working. **This is the one documented, expected non-acceptance condition per spec §4.4** and must
be resolved (by committing this work) before this deployment is cited as a clean acceptance
artifact; it does not reflect anything wrong with the deployment itself, every other check against
it passed.

**Update**: a second live pass (§9) closed every other gap found after this document was first
written — three found by an external review of this exact commit, two found independently while
acting on that review. `verify-deployment.sh` now reports **78 passed, 1 failed**, and the single
remaining `FAIL` is still only `git_tree_dirty` (expected, for the same reason as above — this
pass's own fixes are themselves uncommitted while being proven live). Everything below this point
is the original write-up and is kept as-is for its own historical accuracy; §9 is the new
evidence, added without altering what came before it.

## 1. Oracle ABI migration note (spec §5.2.2)

`OracleAdapter.set_reference_value` is the breaking, source-aware ABI
(`set_reference_value(caller, source, value, timestamp)`, not Tranche 1's
`set_reference_value(caller, value, timestamp)`). `oracle_abi_version: 2` is recorded in the
manifest. Every script in this deployment — `scripts/deploy/deploy.sh`,
`scripts/create_market_via_factory.sh`, `services/oracle-relay/relay.sh`, and the Rust test suite
— calls this ABI exclusively; nothing in this repository still calls the Tranche 1 two-argument
form. `docs/API_REFERENCE.md`/`docs/SECURITY.md`'s error tables were not found to need changes:
this round appended new errors (`OracleUnavailable = 8` onward, confirmed live in §4 below) after
the existing 1–7, per the auditor requirement in spec §5.2.3 — no renumbering occurred.

## 2. Deployment manifest and addresses

Manifest: [`deployments/testnet/deployment-manifest.json`](../deployments/testnet/deployment-manifest.json).

- Network: `testnet`, passphrase `Test SDF Network ; September 2015`, RPC
  `https://soroban-testnet.stellar.org`.
- Source commit at build time: `665bb6760372d30307a1a035da24e51776f727e8` (the tree was dirty on
  top of this commit — see the status note above).
- `rustc 1.94.1 (e408947bf 2026-03-25)`, `stellar 26.0.0 (60f7458e7ecffddf2f2d91dc6d0d2db4fab03ecc)`,
  `soroban-sdk 26.0.1`, target `wasm32v1-none`.
- Underlying asset: a fresh classic asset, code `TR2`, issued by a dedicated Testnet-only account
  (`tranche2-issuer`), `AUTH_REQUIRED` + `AUTH_REVOCABLE` both set
  ([`1c68939d…`](https://stellar.expert/explorer/testnet/tx/1c68939d7898876e89def5961d8deaf4b7205dd0802d6e9ab12549aa8c028390)),
  wrapped as a Stellar Asset Contract
  ([`156bfef5…`](https://stellar.expert/explorer/testnet/tx/156bfef5ade9e965e528414beaf74e8d1dfbeb4a5d9023349c790d500d3d14eb)):
  `CD2A2J7YWN4QME2KUHLFWNBWJUHGY7IP5RNYY47Y625XIL6OFHK25374`.
- Accounts: issuer `GDJ6UBFYQLWPHHOCRSHPBPW74NL2Q5ZUWKSEIOVKSYSK2JRE4PPGLZWT`
  (`tranche2-issuer`), admin `GBTZNAOBEIRAZPN6BKD7SZO2L3DGAQQZ6ZDP2YVGGZPO3K2E7YBSDFFF`
  (`tranche2-admin`, also `PROTOCOL_ADMIN_ADDRESS`), treasury
  `GBPDLO73W2BQJ3F755ZZZKVFKP2FPVEIFW5THYANGDWEYJ5QQYDCMGET` (`tranche2-treasury`). All three are
  Testnet-only keys with no mainnet counterpart.

### Shared infrastructure

| Contract | Address | Deploy tx | Init tx |
|---|---|---|---|
| OracleAdapter | `CCP6U5FLJW6ZUUFLJOQLQ2SYRVA2ENQVGZ5YMRRJTJIBMFOCOI7PJP6P` | not captured — see §6 finding 5 | [`1955050f…`](https://stellar.expert/explorer/testnet/tx/1955050f4dc388219616bca2beefbd75fa1b0dc3f65b7c93e4f67223fc541b45) |
| Permissioning | `CD7PNCOQNXWCDMKB2SPWWH5M5V4CBWW5XTP6KCXSU4ZT2Q4HBVIAHMFQ` | not captured — see §6 finding 5 | [`e9ed37b9…`](https://stellar.expert/explorer/testnet/tx/e9ed37b9a1a348798da90fd2a54d71fe126275d786ee77c5a0f385327d37784d) |
| RiskControl | `CCQ4WK3JFD3I7TQ73JS5IYBSGGALVWEH2AVOKKK3OBGTVV6G6ZC4CF5I` | not captured — see §6 finding 5 | [`f0ba774a…`](https://stellar.expert/explorer/testnet/tx/f0ba774a87336c8bd5993ae3dc800e35c50161150dc908487ba32e3d3054e0e0) |
| Router | `CBYXKUV3DNIRC3DQ7LU257C373EBOLRFJCN3F65JP2LT35NJL47Y5DVZ` | not captured — see §6 finding 5 | [`c6d5b869…`](https://stellar.expert/explorer/testnet/tx/c6d5b8691c9b5e7c3f08cd614cadf9776ca416f637c01d593a56ebc7468c5b1c) |
| MarketFactory | `CACFLQZQBBNF37V6ELSKFMC77FGRGLW74UPIYKT7U5B4VBMPLJ6A2UQ6` | not captured — see §6 finding 5 | stateless, no `initialize` |

`RISK_CB_LIMIT=0` (circuit breaker disabled for this Testnet round, per `.env`),
`RISK_WINDOW_LEDGERS=17280` (the on-chain default, left unset).

Note on the five "not captured" deploy-tx cells above: these five contracts were first deployed by
an earlier, buggy run of the script (finding 5 below), before the `LAST_TX_FILE` fix existed, so
their original deploy transactions were genuinely lost, not merely unlogged. Every run after the
fix correctly reused them (idempotency, finding 4) rather than redeploying, so there was no later
deploy call to capture a hash from either — `deploy_tx` for these five will stay empty in this
manifest until a deliberate `FORCE_REDEPLOY=true` run replaces them, which the fix itself was only
verified against in isolation (a throwaway contract, hash `5f6017b5…`, cited in finding 5), not
against these specific five.

### Market (`testnet-CD2A2J-CDO7S4YP`, maturity `1802381857`, ≈90 days out at creation)

*(Renamed from `testnet-CD2A2J-89d-CDO7S4YP` in §9.4's fix below — the original id embedded a
live days-to-maturity count that changed on every later re-run of `deploy.sh` even though nothing
about the market itself changed; the stable id above is what the manifest and the live chain
agree on as of this document's last update.)*

Created through `MarketFactoryContract` (`scripts/create_market_via_factory.sh`), salt
`8dfa421b2b38fdd572d2e2fc99f25ded3a1aa7d8ca0bea76d3df9360e566b67b`:

| Contract | Address |
|---|---|
| SYWrapper | `CDO7S4YPV36NFC5CHMSBGUVRZSKHPHIX35RBL33R4RLALVUK4YQSG4S5` |
| PTToken | `CADCXJF7DTZ5T2W7QRIJWBI5Q5LVGRTHW33F2SRAGMQKO3MIRDQIJBWG` |
| YTToken | `CBCLSNK5N4SDNLTE7KXV52T6AQLPANSGH2UHUZ4K2TBW7TMIUWMY3O6J` |
| MarketConfig | `CAY2UW3LM7KTHSUZJKCTVQPTWP74V3O64IMN5ZQWSTDBQUWAPDOJYCWN` |
| PrincipalManager | `CB6TD5FT3UP5OMWFIKVXO72O4IVXB72JAC3KQOBTBJ2FGGW7YWI4JI2K` |
| MarketPool | `CDFKUEQHX32XOHV6K2VHYUDEKZFHRCRSNVNH3KBYBBVEQM2KM2CFEGCY` |
| RecoveryEscrow | `CDGBQF54N66HEZGEFW7ZIU57DEEHCAWGPDGU6SCYUSXV5TBSFDATZW3X` |

Fees: `tokenization_fee_bps=5`, `yt_fee_bps=1000`, `swap_fee_tier_bps=10`, `protocol_share_bps=2000`,
`time_stretch_years=2`. `double_reported_wrap_and_mint_capacity: true` (spec §5.3's RiskControl
volume-policy note, recorded as required).

Market-creation and wiring transactions (via `create_market_via_factory.sh`; not yet individually
logged into the manifest's own `transactions[]` — see §6 finding 3):

| Step | Transaction |
|---|---|
| `factory.create_tokens` | [`5fa11388…`](https://stellar.expert/explorer/testnet/tx/5fa113887a4ca2f70e35f92c1fc32ba58833ee7418526df5fb64f3c0b7dc3094) |
| `factory.create_engine` | [`27f3a2dc…`](https://stellar.expert/explorer/testnet/tx/27f3a2dc4db00216e802a46a8040a9c87819b6beab12cc60bca3c6f23590c9e2) |
| `factory.create_pool_and_escrow` | [`6d84844d…`](https://stellar.expert/explorer/testnet/tx/6d84844d00b806f783f85696d4f9241b04f13d86e057b1b6cc46ed39fa7becb0) |
| `risk_control.add_consumer(SYWrapper)` | [`3c7be0b0…`](https://stellar.expert/explorer/testnet/tx/3c7be0b061e68d0cd1732ee2a7ea31ea1f64d0af4639f1a5adfa0a3ca2c46400) |
| `risk_control.add_consumer(PrincipalManager)` | [`0f3a22e9…`](https://stellar.expert/explorer/testnet/tx/0f3a22e991b1f6ada97d728655551ce406d9c93c7298df6d07e9461aa9a71a67) |
| `router.register_market` | [`f7cd11a3…`](https://stellar.expert/explorer/testnet/tx/f7cd11a356cd3108dad4f32de06353cb839e941d310d8577dfec0abdadf8a814) |

Compliance grants for this market's contracts, treasury, and recovery escrow — every one of these
16 calls is now individually logged in the manifest's `transactions[]` (a gap found and fixed this
round; see §6 finding 5):

| Account | `set_authorized`/`grant_account` | `grant_asset` (PT) | `grant_asset` (YT) |
|---|---|---|---|
| SYWrapper | [`e79f4f06…`](https://stellar.expert/explorer/testnet/tx/e79f4f06653894713cdc02e504940d460cc5682bb6470b0c4b006c64975c5842) / [`356d2923…`](https://stellar.expert/explorer/testnet/tx/356d292364e1a91fb6e5391d5fb1380668be7a01386cc333fb6b876712ba1929) | – | – |
| PrincipalManager | [`187a4127…`](https://stellar.expert/explorer/testnet/tx/187a412771b8d0069da7712b66fdc23982c5981d06846b06e72664fc0b541418) / [`c37adc86…`](https://stellar.expert/explorer/testnet/tx/c37adc868e87b771b6ccb8da04d6263af60bf13b593b955e8f17538c919bd35c) | – | – |
| MarketPool | [`0095bede…`](https://stellar.expert/explorer/testnet/tx/0095bedecd0736f10ac808725073fda64fb59adff3144165f535b8d829706403) / [`274cf936…`](https://stellar.expert/explorer/testnet/tx/274cf936b48e7a02687297e798b21c57836c9faebc2584cedb7213c05fb46c8f) | [`bbebaab9…`](https://stellar.expert/explorer/testnet/tx/bbebaab9ae57c765d97f361541590d7cd40e4a0285e83d24095e76280aefdbce) | [`e5621ccb…`](https://stellar.expert/explorer/testnet/tx/e5621ccb001dfa508b8482077a60b61e041a9df789cedca7bfe0bedc73d6c273) |
| RecoveryEscrow | [`0b97c7da…`](https://stellar.expert/explorer/testnet/tx/0b97c7da20cec7ce466696c86b4e1c961de8575b819a299cae44eac649d3cda6) / [`ff680653…`](https://stellar.expert/explorer/testnet/tx/ff680653bcf279880f5a120d692a4797a103b419c9f7235129e305088019f576) | [`51f52156…`](https://stellar.expert/explorer/testnet/tx/51f5215660334fe9ea63d72a48ac8eacebfbc2a1e3d07dfae4b6444f3976488a) | [`e589d914…`](https://stellar.expert/explorer/testnet/tx/e589d914ad7cbf30c64d319f4d4059af538383442dde6d60ecabdc4f10928012) |
| Treasury | [`41073ba7…`](https://stellar.expert/explorer/testnet/tx/41073ba7de41d98b1baf190234ce2c257b987e5ca58413df8184f6affca3bb3b) / [`03159b8b…`](https://stellar.expert/explorer/testnet/tx/03159b8bfc593625ccbe4a3ada16317a91e3c4f80d61b9a8549fbaddc2374113) | [`3efc1850…`](https://stellar.expert/explorer/testnet/tx/3efc18508bf3166566dbc816c6a12ef398f3dacc22b5d65cb129e97d0da57a7b) | [`faf010fe…`](https://stellar.expert/explorer/testnet/tx/faf010fee9430e61d4ef8002e34ad31ad81dcc1d55d3a99ee9b3cbce1a6658af) |

Oracle genesis push:
[`74b26c56…`](https://stellar.expert/explorer/testnet/tx/74b26c561190309c3781d4447d4ebc0974dbda0fa8975d73d203ec443e3f5aed)
(`10,000,000` @ scale `1e7`, source = 32 zero bytes, i.e. `sha256` preimage not yet assigned to a
real relay identity — this round used a placeholder manual-push source, documented further in §5).

## 3. WASM SHA-256 table

All twelve contracts, built fresh this session from the dirty tree described above
(`cargo build --workspace --release --target wasm32v1-none`):

| Contract | SHA-256 |
|---|---|
| `principal_oracle_adapter.wasm` | `f5c8a129c5aad53aff733b325a9949926a4fd4abf185a4dd0aec53476bf45253` |
| `principal_permissioning.wasm` | `3bc08a9e21f12da4a9727c4923d664953974d3700a2fe60f8662cddf0eb9356b` |
| `principal_risk_control.wasm` | `fcac847ea311297a27abed2629c24fe56b6963fb899e880557a6d01bfde7ada1` |
| `principal_router.wasm` | `708c8a283c242b7b8071dd939aa6612aaa804a78e99d0378e27c26a56284132e` |
| `principal_market_factory.wasm` | `409228b6b6cb077eea65effa50840d9ded1547636998138e725404ede8818a5b` |
| `principal_sy_wrapper.wasm` | `394217638cd1b20b59dafb43bcd6fa76e3c9db860a89c54bc2cc9a9fc1c98e32` |
| `principal_pt_token.wasm` | `44fd470c791b36c597d96ccebbaf3ed4c0bb216449f089f4ed3e0b0a981b168a` |
| `principal_yt_token.wasm` | `3fe7f27d38365ee8e933cd86dcd943a7330c2ba6a2c476717ac6b299fe283e89` |
| `principal_market_config.wasm` | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` |
| `principal_manager.wasm` | `819ae7ba616bc078369fe2b6e9a984b9b788fbb43dc62501494241b5e1b6f584` |
| `principal_market_pool.wasm` | `caafcbfbcbfff646303f7da75ba707a48add01e79620f11a80a21cecf7e2cb4e` |
| `principal_recovery_escrow.wasm` | `a4d46832ef28f9f4ee30ce64dd2115c2e2dbbfb86eb1f6ad985debedd9b33590` |

At the time this section was first written, `verify-deployment.sh` only independently confirmed
5 of these 12 against the on-chain code hash and the manifest's own recorded `wasm_sha256` (§4) —
an inaccuracy in this very claim, caught by an external review and fixed in §9.3/§9.5: the script
now checks all twelve, and all twelve now genuinely pass.

Separately, and real evidence in its own right: a clean rebuild from this same commit (§9) produced
**byte-identical** hashes to every one of the twelve above, with no exception — confirming this
dirty-tree build was already reproducible, not just internally consistent.

## 4. Verification script output

`./scripts/deploy/verify-deployment.sh` against the manifest above, latest run:

```
== Verifying deployments/testnet/deployment-manifest.json (network=testnet) ==

-- git_tree_dirty (spec §4.4: must be false for an acceptance deployment) --
FAIL  git_tree_dirty is true -- not acceptance-ready (label and justify explicitly if this is intentional)

-- Shared infrastructure --
PASS  oracle_adapter (CCP6U5FLJW6ZUUFLJOQLQ2SYRVA2ENQVGZ5YMRRJTJIBMFOCOI7PJP6P) exists on testnet and responds
PASS  oracle_adapter on-chain code hash matches manifest
PASS  oracle_adapter manifest hash matches local build artifact
PASS  permissioning (CD7PNCOQNXWCDMKB2SPWWH5M5V4CBWW5XTP6KCXSU4ZT2Q4HBVIAHMFQ) exists on testnet and responds
PASS  permissioning on-chain code hash matches manifest
PASS  permissioning manifest hash matches local build artifact
PASS  risk_control (CCQ4WK3JFD3I7TQ73JS5IYBSGGALVWEH2AVOKKK3OBGTVV6G6ZC4CF5I) exists on testnet and responds
PASS  risk_control on-chain code hash matches manifest
PASS  risk_control manifest hash matches local build artifact
PASS  router (CBYXKUV3DNIRC3DQ7LU257C373EBOLRFJCN3F65JP2LT35NJL47Y5DVZ) exists on testnet and responds
PASS  router on-chain code hash matches manifest
PASS  router manifest hash matches local build artifact
PASS  market_factory (CACFLQZQBBNF37V6ELSKFMC77FGRGLW74UPIYKT7U5B4VBMPLJ6A2UQ6) exists on testnet and responds
PASS  market_factory on-chain code hash matches manifest
PASS  market_factory manifest hash matches local build artifact
PASS  oracle_adapter.is_available() == true
      (get_reference_value() = 11500000 -- compare by hand against .env's ORACLE_INITIAL_VALUE if this is a fresh deployment)
PASS  permissioning.get_admin() == manifest admin_address
PASS  risk_control responds to get_cb_limit
PASS  risk_control responds to get_window_ledgers

-- Market topology (latest market in manifest) --
market_id=testnet-CD2A2J-89d-CDO7S4YP pool=CDFKUEQHX32XOHV6K2VHYUDEKZFHRCRSNVNH3KBYBBVEQM2KM2CFEGCY maturity=1802381857
PASS  SY (CDO7S4YPV36NFC5CHMSBGUVRZSKHPHIX35RBL33R4RLALVUK4YQSG4S5) exists on testnet and responds
PASS  PT (CADCXJF7DTZ5T2W7QRIJWBI5Q5LVGRTHW33F2SRAGMQKO3MIRDQIJBWG) exists on testnet and responds
PASS  YT (CBCLSNK5N4SDNLTE7KXV52T6AQLPANSGH2UHUZ4K2TBW7TMIUWMY3O6J) exists on testnet and responds
PASS  Config (CAY2UW3LM7KTHSUZJKCTVQPTWP74V3O64IMN5ZQWSTDBQUWAPDOJYCWN) exists on testnet and responds
PASS  Manager (CB6TD5FT3UP5OMWFIKVXO72O4IVXB72JAC3KQOBTBJ2FGGW7YWI4JI2K) exists on testnet and responds
PASS  Pool (CDFKUEQHX32XOHV6K2VHYUDEKZFHRCRSNVNH3KBYBBVEQM2KM2CFEGCY) exists on testnet and responds
PASS  Escrow (CDGBQF54N66HEZGEFW7ZIU57DEEHCAWGPDGU6SCYUSXV5TBSFDATZW3X) exists on testnet and responds
PASS  SYWrapper.underlying_address() matches manifest underlying
PASS  SYWrapper.permissioning_address() matches manifest permissioning
PASS  PT.underlying_address() matches manifest underlying
PASS  PT.permissioning_address() matches manifest permissioning
PASS  PT.maturity() matches manifest maturity
PASS  PT.minter() == PrincipalManager
PASS  PT.recovery_escrow() matches manifest
PASS  YT.underlying_address() matches manifest underlying
PASS  YT.permissioning_address() matches manifest permissioning
PASS  YT.maturity() matches manifest maturity
PASS  YT.minter() == PrincipalManager
PASS  YT.recovery_escrow() matches manifest
PASS  PrincipalManager topology: sy_wrapper_address
PASS  PrincipalManager topology: pt_address
PASS  PrincipalManager topology: yt_address
PASS  PrincipalManager topology: oracle_address
PASS  PrincipalManager topology: config_address
PASS  MarketPool topology: manager_address
PASS  MarketPool topology: sy_address
PASS  MarketPool topology: pt_address
PASS  MarketPool topology: yt_address
PASS  MarketPool topology: config_address
PASS  MarketPool.maturity() matches manifest
PASS  MarketPool.recovery_escrow() matches manifest
PASS  RecoveryEscrow.underlying_address() matches manifest underlying
PASS  Router has registered the pool (market().manager == PrincipalManager)
PASS  RiskControl.is_consumer(SYWrapper) == true
PASS  RiskControl.is_consumer(PrincipalManager) == true
PASS  MarketPool is not paused
PASS  market has not matured yet (89d remaining)
PASS  SY has Permissioning account-level standing
PASS  Manager has Permissioning account-level standing
PASS  Pool has Permissioning account-level standing
PASS  Escrow has Permissioning account-level standing
PASS  Pool has PT per-asset standing
PASS  Pool has YT per-asset standing
PASS  Escrow has PT per-asset standing
PASS  Escrow has YT per-asset standing

== 64 passed, 1 failed ==
```

64/65 checks pass; the single `FAIL` is the expected, documented `git_tree_dirty` condition from
the status note above, not a defect in the deployment.

*(This was this script's first-ever run and only hash-checked 5 of the 12 deployed contracts —
see §9.3. §9.5 has the later, 78-check run after that gap and others were closed.)*

## 5. Oracle safety guards, demonstrated live

All of the following are real transactions against the live `OracleAdapter` instance above
(`CCP6U5FLJW6ZUUFLJOQLQ2SYRVA2ENQVGZ5YMRRJTJIBMFOCOI7PJP6P`), run in sequence this session, in
addition to the 56 passing Rust unit tests already covering every branch of this logic
in-process (`contracts/oracle_adapter/src/test.rs`).

### 5.1 Push path (`set_reference_value`, the spec's primary ABI)

| Check | Result | Transaction |
|---|---|---|
| Fresh update accepted (`11,500,000`, +6.5% from the prior value, within the 10% bound) | accepted, `ref_set` emitted | [`157d9beb…`](https://stellar.expert/explorer/testnet/tx/157d9beb934e89962710a4ceb7af75175a9f9c853450492d680b73b62c44cca3) |
| >10% deviation rejected (`13,500,000`, +17.4%) | **not reverted** — call succeeded, returned `false`; value stayed at `11,500,000`; `ref_rej` emitted with `reason: "deviate"`; `last_rejection()` read back `{"reason":"deviate","timestamp":1794606978,"value":"13500000"}` | [`2d189f03…`](https://stellar.expert/explorer/testnet/tx/2d189f03e9a87dd26f987a5e0bfe4359f11d6d7b7496eced98597b699b62bcd8) |
| Stale/unavailable state detected | `is_available()` read `true` under the normal 3600s window; `set_safety_params(max_stale_seconds=1, ...)` deterministically made the existing value stale; `is_available()` immediately read `false`; `assert_available()` simulation reverted `Error(Contract, #8)` = `OracleUnavailable` | safety-params tx [`7ab72841…`](https://stellar.expert/explorer/testnet/tx/7ab7284151f4b89e9d7d193b2ff3b4a2e32317f95e0e6c9cce42e51681b7de46); `assert_available` was a `--send=no` simulation (no tx, by design — it never submits) |
| Oracle restored to normal operating state | `set_safety_params` back to `max_stale_seconds=3600`/`max_deviation_bps=1000` (matching `.env`), value refreshed to the same `11,500,000` | [`e130b157…`](https://stellar.expert/explorer/testnet/tx/e130b157ed9ea9c7c07c6b733c6f3b7032c0c7447c212515df077c673a75acf5), [`93a3f980…`](https://stellar.expert/explorer/testnet/tx/93a3f980a6cf298d8e9eaffa5caef37a1db3046663c2176fa6350393a6f71352) |

Note on `OracleUnavailable`'s error number: confirmed live as `#8`, consistent with spec §5.2.3's
instruction to append new errors after the existing 1–7 (`AlreadyInitialized`, `Unauthorized`,
`InvalidValue`, `TimestampTooOld`, `NotInitialized`, `ValueDecreased`, `TimestampInFuture`) rather
than renumber.

### 5.2 Pull path (`set_feed_source`/`sync_from_feed`) — beyond the funded spec

This mechanism (§5.2/§10.2 note in `OracleAdapter`'s own module doc: an *additional*, optional
on-chain pull, kept alongside the spec's primary off-chain relay path per an explicit "keep both"
decision) had 13 passing unit tests but had never been exercised against a real cross-contract
call. No real RedStone Testnet feed address exists yet to test against directly, so a minimal,
spec-shaped SEP-40 mock (`decimals()`/`lastprice(asset)`, matching `contracts/oracle_adapter/src/test.rs`'s
own `MockSep40Feed` field-for-field) was built as a standalone crate and deployed live:
`CBF7CKVZAPBAEW7QAGESW52IWURLANACQG7JVGI74U4LVLEUIVXGGIQA`
([deploy](https://stellar.expert/explorer/testnet/tx/1d5b1557c491014c077bd42d0207170888adfc82f365a88afb1975fa8ec444dc)).
It is not part of the Principal workspace — a throwaway live-testing tool, not product code.

| Check | Result | Transaction |
|---|---|---|
| `set_feed_source` wiring | accepted, `feed_set` emitted | [`70e8e0be…`](https://stellar.expert/explorer/testnet/tx/70e8e0be5fe710a177a929a5052c47f785771a61590fc3591c93e67266528955) |
| `sync_from_feed`, rescale-down accept (feed `decimals()=8`, price `100,000,000` → protocol scale `10,000,000`, same value as the prior push — "value may stay equal" is allowed) | accepted, `last_update_was_feed() == true` | [`09b687f7…`](https://stellar.expert/explorer/testnet/tx/09b687f780e2908d8d783f12d989b6a5afcbee5e5ca04f09851f58544748feaf) |
| `sync_from_feed`, genuine increase within bound (feed `108,000,000` → `10,800,000`, +8%) | accepted | [`39f33f56…`](https://stellar.expert/explorer/testnet/tx/39f33f5682d4cd2f2a92c0d1a379358cc22ccfbe7b937f8131afb7d35f03e461) |
| `sync_from_feed`, >10% deviation (feed `125,000,000` → `12,500,000`, +15.7% from `10,800,000`) | **not reverted** — returned `false`; value stayed at `10,800,000`; `ref_rej` emitted with `reason: "deviate"`; `last_rejection()` confirmed it | [`0eff8aae…`](https://stellar.expert/explorer/testnet/tx/0eff8aae6fa20c728c8e93c765b789ff45735619f823deca175a9717a933155f) |

This is live, direct proof that the rescale/accept/reject pipeline works correctly against a real
cross-contract SEP-40 call, not just the in-process Rust tests. It also resolved the one
implementation uncertainty flagged in the code before this round: the CLI's JSON encoding for a
`Sep40Asset` argument is standard double-quoted JSON (`{"Other":"USDY"}`), confirmed by direct
trial — **not** the unquoted `{Other:hello}` shape the CLI's own `--help` text for `set_price`
shows as an example (that literal form was tried first and the CLI itself rejected it,
`Unknown case {Other:USDY} for Sep40Asset`). `deploy.sh`'s existing
`ASSET_JSON="{\"$ORACLE_FEED_ASSET_KIND\":\"$ORACLE_FEED_ASSET_VALUE\"}"` already produced the
correct shape; no code change was needed there, only this confirmation.

## 6. Findings from the first live runs, and what was fixed

The deployment pipeline (`scripts/deploy/deploy.sh`, `scripts/deploy/verify-deployment.sh`) had
never been run before this session — every item below is a real bug found by running it against
Testnet, not a hypothetical review comment. All are fixed in the versions of the scripts this
document describes.

1. **`deploy_or_reuse`'s status `echo` lines corrupted every shared-infra contract ID.** Every
   caller assigns this function's result via `X_ID="$(deploy_or_reuse ...)"`, which captures *all*
   of the function's stdout, not just its final `printf`. The function's own
   `echo "$name: deploying fresh"`/`echo "$name: reusing ..."` lines were on stdout too, so
   `ORACLE_ID` etc. ended up holding multi-line garbage (`"oracle_adapter: deploying fresh\nCCP6U5..."`),
   which every subsequent `--id "$ORACLE_ID"` call then rejected. Compounding this,
   `invoke()`/`upload_wasm()`/`deploy_from_hash()` captured the real CLI error to a tempfile but
   only ever read it for the `"Signing transaction: ..."` line, then deleted it — so the actual
   failure died completely silently. Fixed: the status lines now go to `>&2`; the three wrappers
   now print the real stderr before letting the script exit on failure.
2. **The per-market crate `principal_manager`'s own wasm filename doesn't follow the
   `principal_<name>.wasm` pattern every other per-market crate does** (its package is already
   named `principal_manager`, so the artifact is `principal_manager.wasm`, not
   `principal_principal_manager.wasm`). The deploy script's upload loop assumed the uniform
   pattern and failed outright the first time it reached this contract. Fixed with a small
   `wasm_filename_for()` helper special-casing this one name.
3. **`set_authorized` on a classic G-account with no existing trustline reverts the SAC's own
   `TrustlineMissingError` (`Error(Contract, #13)`)** — true of `TREASURY_ADDRESS` the first time
   this script runs against a brand-new asset, since a fresh account has never touched it. A
   contract address never hits this (the SAC creates its balance entry lazily), so this is
   specific to G-accounts, and only that account's own key can fix it (`stellar tx new
   change-trust`) — this script cannot do it on anyone's behalf. Fixed: the compliance loop now
   detects this specific failure and prints the exact remediation command instead of a bare,
   opaque contract error; also documented in `docs/DEPLOYMENT.md` §11.
4. **The per-market section had no idempotency check at all** — re-running `deploy.sh` against an
   unchanged `.env` created a second, fully redundant market every time (confirmed live: the first
   re-run produced a second market with a new salt while correctly reusing all five shared-infra
   contracts). This directly contradicted the task's own idempotency requirement and spec §5.4's
   "verify, don't retry" rule for one-time setters. Fixed: the market section now checks
   `sy_wrapper`'s liveness first and reuses the existing market's six contract addresses unless
   `FORCE_REDEPLOY=true`; `market_id` is now derived from `SY_ADDR` rather than a wall-clock
   timestamp, so it stays stable across reuse; a reused market's `maturity` is now read back live
   from `PT.maturity()` rather than recomputed from `.env`'s "now + 90 days" default (which would
   otherwise silently overwrite the manifest with a fictitious maturity on every re-run). Proven:
   two consecutive full runs after this fix produced byte-identical shared-infra and market
   contract sets (diffed the manifests; only `built_at`/`transactions` differed), confirmed via
   `deploy_tx`/`deploy_run5.log`–`deploy_run7.log` in this session's working notes.
5. **Every `deploy_tx` in the manifest came back empty, and no compliance-grant transaction was
   logged at all.** Root cause for the first: `upload_wasm`/`deploy_from_hash` are always called
   as `x="$(wrapper ...)"` — a command substitution, which always forks a subshell — and the
   `LAST_TX=...` assignment meant to report the signed transaction's hash back to the caller was
   made *inside* that subshell, so it vanished the instant the subshell exited; a plain
   `invoke ... >/dev/null` call (no subshell) never hit this, which is why every `init_tx` was
   correctly populated the whole time while every `deploy_tx` was not. Fixed with a tiny file-based
   side channel (`LAST_TX_FILE`, read back via a new `read_last_tx` helper after any
   subshell-wrapped call) — verified in isolation against a real `stellar contract deploy` call
   (captured a real, previously-lost tx hash: `5f6017b5db7ab64e12b15ce605e628d6ef4beb30e1a4682dc99fe0e2ef59faca`).
   The second gap (compliance grants) was simply a missing `manifest_log_tx` call at each site;
   fixed, and all 16 resulting hashes are in §2's table above, logged for real on the final run.
6. **`verify-deployment.sh`'s `bash -c "...view...eq..."` checks all silently ran the wrong
   program.** `view`/`eq`/`alive` are shell functions defined in the parent script; a fresh,
   non-interactive `bash -c` subshell does not inherit them unless exported, so `view` fell
   through to whatever `/usr/bin/view` resolves to on this machine — a symlink to Vim's read-only
   mode — and `eq` resolved to nothing at all. Every check using this pattern (about two dozen)
   failed with Vim's own startup banner as the "error output," not a real assertion failure.
   Fixed: `export -f view eq alive` plus `export NETWORK PROBE_SOURCE` (the functions' own free
   variables) right after they're defined.
7. **`alive()`'s hardcoded `get_admin` probe doesn't exist on `market_factory` (stateless),
   `market_config`, or `recovery_escrow`** (the latter two have no admin of their own by design —
   see `docs/DEPLOYMENT.md` "Overview"). Every one of these three read as dead on the first run,
   not because anything was actually wrong, but because the check itself called a function that
   never existed on them. Fixed: `alive()` now takes an optional probe function name; the three
   call sites above pass a real getter each contract actually has (`predict_market` for the
   factory, `underlying`/`underlying_address` for Config/Escrow) instead of the default.

After all seven fixes, the pipeline was re-run clean (§4), idempotency was proven with two
back-to-back full runs producing no new contracts, and the oracle guards and SEP-40 pull mechanism
were proven live (§5).

## 7. PT/YT/AMM flows and compliance recovery

The protocol's *contract code* backing this deployment's PT/YT/AMM logic and compliance-recovery
path is unchanged from the full feature campaign already proven live end to end in
[`docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md`](TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md) §4
(full market lifecycle: deposit/mint, liquidity, every swap direction, both true flash-mints for
YT, oracle fallback/deviation/bounds rejections, a RiskControl circuit-breaker trip, a full
deauthorize → seize → finalize compliance-recovery cycle, and settle/redeem/fee-claim) and §6
(the Router's plain and underlying-funded flash-mints, `add_liquidity_keep_yt`). That document's
own transaction table is the live evidence for those flows; re-running the entire campaign against
this round's fresh market would only re-prove logic this phase did not touch, so it was not
repeated here — this document's own new live evidence (§5, §6) is deliberately scoped to what
*did* change this round: the deployment pipeline itself, its idempotency, the oracle safety guards
on both the push and new pull paths, and the SEP-40 cross-contract proof.

## 8. What was not verified live, and why

- **The real RedStone USDY/USD SEP-40 feed.** No confirmed Testnet (or mainnet) contract address
  exists for it yet (per this session's own research — RedStone's SEP-40 offering launched after
  this session's knowledge cutoff window started tracking it, and no address was found in its
  current public docs). `sync_from_feed` was proven live against a spec-shaped mock instead (§5.2)
  — correct proof of the mechanism, not of RedStone's specific feed. `ORACLE_FEED_ADDRESS` in
  `.env.example` must not be filled with a guessed address; confirm it from RedStone's own current
  docs immediately before any real use, as `services/oracle-relay/README.md` already warns.
- **`services/oracle-relay/relay.sh` against a real RedStone feed.** It *was* run live this round
  against the same mock feed (§9.6, proving the policy-rejection return-value fix end to end with
  a real transaction), which is correct proof of the script's own logic; proof against the actual
  RedStone feed is still blocked on the same missing address as `sync_from_feed` above, and running
  it on an actual schedule (cron/systemd/CI) remains out of scope for this deliverable (spec §10.2
  is a Tranche 3 item).
- **A per-market-creation transaction log inside the manifest's own `transactions[]` array.** The
  six hashes in §2's market-creation table came from this session's own run logs
  (`create_market_via_factory.sh`'s inherited stderr), not from the manifest file itself — that
  script doesn't yet write its own transaction log into the manifest the way `deploy.sh`'s
  shared-infra and compliance steps now do (finding 5 above fixed the latter two, not this one).
  Flagged as a real, scoped gap rather than silently left out: a future round should either teach
  `create_market_via_factory.sh` to accept the same `manifest_log_tx`-style callback, or have
  `deploy.sh` parse the hashes out of its captured output the same way it already parses the
  contract addresses.

## 9. Second live pass: six more gaps closed, found by an external review plus self-review

An external review of this document's own commit (the dApp integration team, reading this exact
code) found three real issues; reviewing their findings against the actual source surfaced two
more, and the dApp team's own §6 note about `relay.sh` pointed at a sixth. All six are fixed below
and proven against live Testnet, not just reasoned about. `deploy.sh`/`verify-deployment.sh` were
re-run against the *same* TR2 deployment throughout — every shared-infra contract and the market
itself were reused, not redeployed; only the specific setter/push calls below are new transactions.

### 9.1 `git_tree_dirty` would never clear, even after a clean commit

Four files unrelated to this deployment (`AUDIT_REPORT.md`, three `.docx` files) sit untracked at
the repo root; `deploy.sh`'s `git status --porcelain` check counts them regardless of their
relevance. Fixed without touching, moving, or taking any position on those files: their exact
names were added to `.git/info/exclude` (local-only, uncommitted — not `.gitignore`, which is
shared and shouldn't carry a policy opinion about files outside this task's scope). Confirmed:
`git status --porcelain` now prints nothing in an otherwise-clean tree.

### 9.2 Oracle genesis push would have been rejected as a decrease, silently

Found independently while acting on the review above, not reported by it. `deploy.sh`'s oracle
genesis block only pushes when `is_available()` reads false, using the static `.env`
`ORACLE_INITIAL_VALUE` (`10,000,000`) with no regard for whatever value is already stored. The
TR2 oracle's actual stored value was `11,500,000` (confirmed live, `is_available()==false` — well
past the 3600s staleness window since the first pass); pushing `10,000,000` is a decrease, which
`set_reference_value` rejects gracefully (`reason: "decrease"`, returns `false`, does not panic)
— and the script never checked that return value, so it would have logged the tx as a success
while leaving the oracle unavailable.

Fixed: before submitting, the script now reads the oracle's current (stale) value and clamps the
genesis value to `max(ORACLE_INITIAL_VALUE, current_value)`, then caps it at
`current_value * (1 + max_deviation_bps/10000)` so it can't trip the deviation guard either. It
then reads the actual decoded boolean `set_reference_value` returns and aborts loudly if it isn't
`true`, instead of trusting that a submitted transaction means an accepted value. The same
check was added to the (currently unexercised, since no `ORACLE_FEED_ADDRESS` is set this round)
`sync_from_feed` call in the optional SEP-40-wiring branch.

Live proof: re-running `deploy.sh` printed
`oracle_adapter: ORACLE_INITIAL_VALUE=10000000 would be rejected against the existing stale value
11500000 (decrease, or exceeds max_deviation_bps=1000) -- using 11500000 instead`, then submitted
`11,500,000` (an equal-value push, explicitly allowed) and received `true` back —
[`d3c62bea…`](https://stellar.expert/explorer/testnet/tx/d3c62beade870fdaf5a1866b66686eff98a0314935fd1a96f77d508e14ed6078).
`is_available()` reads `true` again afterward.

### 9.3 `verify-deployment.sh` only hash-checked 5 of the 12 contracts

Found by the review, confirmed by reading the script: its wasm-hash loop iterated only
`oracle_adapter permissioning risk_control router market_factory`, never the seven per-market
contracts — so §3's "`verify-deployment.sh` independently confirmed every one of these" was not
literally true. Fixed by adding the identical `wasm_hash_matches_chain`/
`wasm_hash_matches_local_build` pair, looped over `sy_wrapper pt_token yt_token market_config
principal_manager market_pool recovery_escrow`, inside the market-topology section where their
contract ids are already resolved. All 14 new checks pass live (§9.5's output).

### 9.4 `MARKET_ID` embedded a live day-count, so it changed on every re-run

Found by the review: `MARKET_ID` was built from
`$(( (MARKET_MATURITY_UNIX - $(date +%s)) / 86400 ))d`, a function of *when* the script happens
to run, not of the market itself — so the same, unchanged, reused market got a new id
(`testnet-CD2A2J-89d-CDO7S4YP` → a lower number the next day, and so on) every single re-run,
breaking any doc or external record that had cited the old one. Fixed by dropping the day-count
entirely: `MARKET_ID="${NETWORK}-${UNDERLYING_SAC:0:6}-${SY_ADDR:0:8}"`, which depends only on
the underlying asset and the market's own SYWrapper address — both fixed for this market's
lifetime. This is a one-time rename: the market itself, every address, and its maturity are
unchanged; only the label changed, from `testnet-CD2A2J-89d-CDO7S4YP` to
`testnet-CD2A2J-CDO7S4YP` (confirmed live in the manifest and by `verify-deployment.sh`, §9.5).

### 9.5 RiskControl limits were `0` (circuit breaker disabled) — not a real demonstration

Tranche 2 Deliverable 1 explicitly requires "RiskControl limits" as part of initial
configuration; `RISK_CB_LIMIT=0` (the live-test default, which disables the breaker entirely)
doesn't demonstrate that. `RiskControl` already exposes live setters for both values
(`set_cb_limit`, `set_asset_limit` — the latter was already called conditionally by `deploy.sh`;
only the former was missing), so no redeploy was needed. Fixed: `deploy.sh` now reads the
current `get_cb_limit()` and calls `set_cb_limit` if it differs from `.env`'s `RISK_CB_LIMIT`,
mirroring the existing `set_window_ledgers` pattern. `.env` was updated to
`RISK_CB_LIMIT=50000000000` and `RISK_ASSET_LIMIT=50000000000` (5,000 units of the 7-decimal
underlying — the same order of magnitude as the largest single deposits in the prior full
feature campaign, `docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md`), applied live:

| Call | Transaction |
|---|---|
| `risk_control.set_cb_limit(50000000000)` | [`fc989ea9…`](https://stellar.expert/explorer/testnet/tx/fc989ea90aee7fc7834662d0c0bfbbfe0138a74e9cb8a4732b5920ce6316b36d) |
| `risk_control.set_asset_limit(TR2 SAC, 50000000000)` | [`e699525e…`](https://stellar.expert/explorer/testnet/tx/e699525e785888b927a5a7e08cffa63338ff16264faae9d16a3045da0a1177e4) |

Confirmed live afterward: `get_cb_limit()` and `get_asset_limit(TR2 SAC)` both read back
`50000000000`.

After 9.1–9.5, `deploy.sh` was re-run (confirmed idempotent: every shared-infra contract and the
market were reused, zero new deploys) and `verify-deployment.sh` was re-run against the result:

```
== 78 passed, 1 failed ==
```

The single `FAIL` is still `git_tree_dirty` — expected, since this pass's own fixes are
themselves uncommitted while being proven live (same documented condition as §1/§4). Every other
check, including the 14 new hash checks from §9.3, passes.

### 9.6 `relay.sh` logged a policy rejection as `"submitted ok"`

The dApp team's own §6 note, not something this round went looking for independently: `relay.sh`'s
submit step only checked whether the CLI invocation itself succeeded (the transaction was
included), via its exit code — but a graceful policy rejection (decrease/deviate/stale) is a
*successful* transaction that returns `false`, not a revert, so the CLI's own exit code is 0
either way. The old code logged every included transaction as `log_attempt submitted ok ...` and
advanced `relay-state.json`'s replay-guard timestamp regardless, which would have silently hidden
a real rejection from the audit log and could have caused a later, genuinely corrected
resubmission of the same source tick to be wrongly treated as already-seen.

Fixed: the submit step now captures the invocation's actual decoded stdout (the real `true`/
`false` the contract returned, not just the CLI's exit code) and, on `false`, logs
`decision: "skipped", reason: "policy_rejected"` instead of `"submitted ok"`, does **not** write
`relay-state.json`, and exits nonzero.

Proven live, not just by inspection — no real RedStone feed exists to test against, so this
reused the same mock SEP-40 feed §5.2 proved `sync_from_feed` against
(`CBF7CKVZAPBAEW7QAGESW52IWURLANACQG7JVGI74U4LVLEUIVXGGIQA`). The oracle's `primary_source` was
temporarily repointed (`set_sources`) at the exact source-id hash `relay.sh` itself derives from
its own preimage convention, the mock feed was given a price equal to the oracle's current value
at a timestamp equal to its already-stored timestamp (deliberately replaying the exact tick
`set_reference_value`'s own `timestamp <= current_ts` check exists to catch), and `relay.sh` was
run pointed at it with `relay-state.json` reset (so the relay's own *local* replay guard would not
pre-empt the on-chain call — the point was to reach the on-chain check, not stop short of it):

```
submitting: source=115000000 (decimals=8) -> scaled=11500000 @ 1794613443 (current=11500000)
oracle rejected this update on-chain (set_reference_value returned 'false', tx=59451eaf5e0749897185c241c5899020e9f07a55b61f2182efea7675b2119317) -- not advancing replay state
```

Transaction: [`59451eaf…`](https://stellar.expert/explorer/testnet/tx/59451eaf5e0749897185c241c5899020e9f07a55b61f2182efea7675b2119317)
(a real, included, successful transaction that returned `false`). `last_rejection()` on the
oracle read back `{"reason":"stale_ts","timestamp":1794613443,"value":"11500000"}` immediately
after, confirming the exact rejection reason. `relay-state.json` was not written (confirmed by
its absence afterward), and `audit-log.jsonl` recorded `"decision": "skipped", "reason":
"policy_rejected"` — the fix working exactly as intended, not the old `"submitted ok"`.

The oracle's `primary_source` was restored to the documented zero-byte placeholder immediately
after this test
([`0c92432d…`](https://stellar.expert/explorer/testnet/tx/0c92432d24f6933bc8a65f66e9994274e4dcd61578c0f64a037f0c24ec730bc3)),
so §2's existing description of the genesis push's source configuration remains accurate; the
stored price/timestamp were never at risk during this test, since `set_sources` only changes
which source id is accepted, not the value already on record.

## 10. A real finding from a dedicated security review, fixed after §9's live pass

Separately from the dApp-integration review in §9, a focused security review was commissioned
specifically against the `sync_from_feed`/`set_feed_source` diff (this being new, beyond-spec
on-chain surface the funded spec's own enumerated error/API lists never covered). It first raised
a reentrancy claim against `sync_from_feed` — re-checked line-by-line against the actual source
before acting on it, and retracted: `current_price`/`current_ts` are read fresh *after* both
external `Sep40Client` calls (`decimals()`, `lastprice()`) return, with no further external call
between that read and the storage write, so no TOCTOU window exists there. No fix was needed or
made for that claim.

It raised a second, confirmed-real issue: `reject()` unconditionally overwrote the single
`LastRejectedValue`/`LastRejectedTimestamp`/`LastRejectReason` slot regardless of the rejection's
severity, and `sync_from_feed` is permissionless (no caller, no `require_auth`) — so anyone, for
free, at any time, could call it with no new data (a routine `stale_ts`/`no_data` rejection) right
after a genuine `deviate`/`decrease` attempt, overwriting `last_rejection()`'s view back to
something benign. The `ref_rej` event log already recorded every rejection regardless, so this was
a monitoring-integrity gap in the single-slot view, not a missing audit trail — but Tranche 3
Deliverable 3 explicitly requires "automated alerting when the feed becomes stale or unavailable,"
and `last_rejection()` is the obvious, cheap thing such alerting would poll.

Fixed: `reject()` now takes an explicit `significant: bool`; only `oob`, `decrease`, `deviate`,
and `fb_value` (the reasons that reflect an actually-attempted, disallowed value change) update
the slot. Routine reasons (`stale_ts`, `no_data`, `bad_price`, `rescale`, `future_ts`) still emit
`ref_rej` but leave the slot untouched. A new regression test,
`a_routine_rejection_cannot_mask_a_prior_significant_one`, reproduces the exact masking scenario
end to end (a genuine `deviate` rejection, then an immediate permissionless `stale_ts` call) and
asserts `last_rejection()` still reads `deviate` afterward. All 57 of `oracle_adapter`'s own tests
pass, `cargo fmt`/`clippy --workspace -- -D warnings`/`cargo test --workspace` are clean.

This fix landed in source *after* §9's live redeploy pass finished, so the TR2 OracleAdapter
instance deployed on-chain (`CCP6U5FLJW6ZUUFLJOQLQ2SYRVA2ENQVGZ5YMRRJTJIBMFOCOI7PJP6P`) still runs
the pre-fix bytecode — §9.3's "all twelve wasm artifacts rebuild byte-identical to the manifest"
claim is only true of the source as it stood at that pass's build, not of `HEAD` from this point
on. Deliberately not force-redeployed to pick this up: `PrincipalManager`/`MarketPool`/`YTToken`
bind their `OracleAdapter` address at `initialize` with no setter (confirmed by the same external
review, independently, in their own §4 note), so changing it for this already-created market would
mean redeploying the entire market, not just one contract — a disruptive, low-value trade for a
few-line change to a monitoring-integrity view, on a deployment that already fully served its
purpose (proving the pipeline, idempotency, and a live `sync_from_feed` cross-contract call). The
fix is proven at the same bar most contract-logic changes in this project are proven at before a
live redeploy — the Rust test suite — and will be included whenever this contract is next actually
redeployed for real (the next Testnet round, or Tranche 3's mainnet deployment).

This also revises this document's own earlier framing of `sync_from_feed` as needing a full,
unscoped, unbudgeted security review before being considered for any mainnet role: one has now
been done. Its one real, confirmed finding is fixed and tested above; the reentrancy claim did not
hold up. This is a meaningfully stronger position than "beyond spec, defer indefinitely" — though
still one internal review pass, not an independent third-party audit, which remains the right bar
to clear before a mainnet asset actually relies on this mechanism.
