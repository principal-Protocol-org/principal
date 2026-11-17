# Mainnet Deployment Evidence — Tranche 3 Internal Testing

**Purpose:** this document is the single source of truth for every real Stellar mainnet address,
contract, and transaction created for Tranche 3's internal-testing deployment. Kept separate from
the security review documents (`INTERNAL_SECURITY_AUDIT_2026_11_16.md` and its coverage report) so
each can be updated independently — this one grows with every deployment action; those are closed
findings records.

**Status (2026-11-18): Markets 1 AND 2 (MOCKUSDY) plus a new MOCKUSTRY market are all COMPLETE AND
LIVE on mainnet, with real end-to-end transaction testing performed on all three.** Shared
infrastructure (§4), the 90-day MOCKUSDY market (§5 Market 1), the 30-minute MOCKUSDY flow-test
market (§5 Market 2), and the 90-day MOCKUSTRY market (§5) are all fully deployed, wired,
compliance-granted, and independently verified (78/79 real checks pass on each — the one
"failure" is the expected `git_tree_dirty` flag). Real mint→deposit→split→trade cycles were run
against all three markets with real signatures — see §5's "Real transaction testing"
subsection for the full record. Mainnet deployment had paused twice along the way on funding (current
mainnet Soroban WASM-upload resource fees, ~0.0012-0.0015 XLM/byte confirmed on two independent
RPC endpoints, made the real cost far higher than first assumed) — each pause left nothing
half-built, and every upload proved fully reusable once funding resumed. Real costs landed a
consistent ~13% below every `simulateTransaction` quote across all 9 WASM uploads this
deployment needed. **≈78.29 XLM remains** across the three working keys — likely enough for a
second (30-minute) market reusing the same already-uploaded contracts (see §10). The
self-service tooling, oracle relay, and full lifecycle proof are already proven out on testnet
(see the `TESTNET_MOCKUSDY_*` document) and apply identically here now that mainnet has a real
market to point them at.

Entries below are added only after the corresponding on-chain transaction has actually been
submitted and its hash confirmed — nothing in this document is planned-but-not-yet-done unless
explicitly marked `PLANNED, NOT YET EXECUTED`.

**Network:** Stellar mainnet (`Public Global Stellar Network ; September 2015`)
**Source commit (contracts):** `4b5bc2b` and onward — see each section for the exact commit the
deployed WASM was built from.

---

## 1. Why mock assets

Principal cannot unilaterally create a market for a real RWA token it does not control: market
creation requires the caller to be the real token's own issuer authority
(`compliance::is_authority`), which for a real asset like Ondo's USDY is Ondo's own key, not
Principal's. For internal testing and reviewer demos on mainnet, Principal instead issues its own
**mock** classic Stellar assets that track a **real** asset's live price via the same real SEP-40
oracle feed the real asset uses — the contracts and oracle-wiring logic under test are identical
to what a real-asset market would use; only the underlying token itself is one Principal controls
end-to-end for testing purposes.

## 2. Real oracle feed verification (live, re-confirmed before use)

Re-verified live on 2026-11-17, directly against the real RedStone SEP-40 feed contract on
mainnet — not taken from any earlier session's notes.

- **Feed contract:** `CBMGLKUQZVSAIL5CPDDAWSUY7MAKXISHMOZEVLMBUWBMFGHRJSR4WYRF` (shared, multi-asset).
- **Feed `decimals()`:** `8`.
- **`assets()`** returns 21 `Sep40Asset::Stellar(...)` entries plus one `Other` entry. Identified
  by calling each returned SAC address's own `name()` view (format `<CODE>:<ISSUER>`):

  | Code | Real SAC address (Stellar contract) | Real issuer (G-address) |
  |---|---|---|
  | USDY | `CB3YA656OYIHU57657I5KGSBRHE5I3OZU4VFC22PYAOANFZHEWNYGAGP` | `GAJMPX5NBOG6TQFPQGRABJEEB2YE7RFRLUKJDZAZGAD5GFX4J7TADAZ6` |
  | USTRY | `CBLV4ATSIWU67CFSQU2NVRKINQIKUZ2ODSZBUJTJ43VJVRSBTZYOPNUR` | `GCRYUGD5NVARGXT56XEZI5CIFCQETYHAPQQTHO2O3IQZTHDH4LATMYWC` |
  | CETES | `CAL6ER2TI6CTRAY6BFXWNWA7WTYXUXTQCHUBCIBU5O6KM3HJFG6Z6VXV` | `GCRYUGD5NVARGXT56XEZI5CIFCQETYHAPQQTHO2O3IQZTHDH4LATMYWC` |
  | GILTS | `CCQNEBGX5RJQGNLQ25C67DZLOSNM4NFQBANUS3ASALF2X552BSIQ6XCE` | `GCRYUGD5NVARGXT56XEZI5CIFCQETYHAPQQTHO2O3IQZTHDH4LATMYWC` |
  | USDC | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` | `GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` |
  | (others) | EURC, TESOURO, KTB, MXNe, EUROB, PYUSD, Solv BTC, xSolvBTC, "Matrixdock Gold" (likely XAUm — not yet confirmed), native XLM, plus 2 not yet identified, plus one `Other:"earnUSDC_PPS"` entry | — |

  Only USDY, USTRY, CETES, GILTS were cross-checked against this project's own earlier candidate
  list; the rest are recorded here only because they came back from the same live query, not
  because they are planned mock-asset targets.

- **USDY live price (queried this session):** raw `lastprice()` on the feed for
  `{"Stellar":"CB3YA656OYIHU57657I5KGSBRHE5I3OZU4VFC22PYAOANFZHEWNYGAGP"}` returned
  `price = 115347432` at the feed's 8 decimals (**≈ $1.15347432**), `timestamp = 1794912290`
  (consistent with "now" at the time of this check — not stale).
- **USDY real SAC `admin()`:** `CCD3RTQ4YBRLNPIVPT7LQUGT7VXZFIYYOG7ISGOCFMHUT6C6QT6MMBCH` — a
  **contract address**, not a plain account, confirming USDY's real issuer authority is itself a
  smart contract Principal does not control (the whole reason this deployment uses a mock asset
  instead of the real one). This address is never used by Principal for anything; it is recorded
  here only as evidence that market creation against the real USDY is genuinely not possible for
  Principal, matching the `compliance::is_authority` analysis in the security review.

## 3. Mock assets

### MOCKUSDY (tracks real USDY / Ondo Finance)

**Issuer key:** `mainnet-mockusdy-issuer` (local `stellar keys` alias) → `GB3SOEMQVYBLMHPV3AK64PV4FJTOMQ2XUJLOZSVDJLP2LT42EU3XLE4P`
— newly generated for this deployment, dedicated solely to this asset.

**Funding:**
- `create-account` from treasury (`GBUVAHYJNCXZWUK3NIOVJ6EYQRSXMVT2XOUEASS3VAG4EWIXXWYSVSXW`), 5 XLM starting balance —
  tx [`6d6513dfe9db2b932b7b2374cb8f86add800ad07ed947f61a005a3c11fa0f256`](https://stellar.expert/explorer/public/tx/6d6513dfe9db2b932b7b2374cb8f86add800ad07ed947f61a005a3c11fa0f256) — confirmed successful.

**Issuer flags set** (`set-options`), matching real USDY's own observed flags exactly
(`auth_required: false, auth_revocable: true, auth_clawback_enabled: true`) —
tx [`a65d5bdafee7da783f72a03e30d2b6c368a3189350e60e23439e8175feacdc80`](https://stellar.expert/explorer/public/tx/a65d5bdafee7da783f72a03e30d2b6c368a3189350e60e23439e8175feacdc80) —
confirmed successful. Live-verified on the account directly:
`{auth_required: false, auth_revocable: true, auth_immutable: false, auth_clawback_enabled: true}`.

**Classic asset code:** `MOCKUSDY`, issued by the account above (no separate "create asset"
transaction exists on Stellar — the asset is defined by code+issuer; the SAC below is what makes
it usable by the Soroban contracts).

**SAC (Soroban Asset Contract) address:** `CDQYZUYQF7TRQ7BX6YBSCU3HLWBDS3E2VSP7AUE7C3E2D2U2IBBQOBGF`
— deployed via `stellar contract asset deploy --asset "MOCKUSDY:GB3SOEMQVYBLMHPV3AK64PV4FJTOMQ2XUJLOZSVDJLP2LT42EU3XLE4P"` —
tx [`33f9393647837881e3bbda15c4bfbf7b12d02beaa20987c0bc2c73e9625a2aeb`](https://stellar.expert/explorer/public/tx/33f9393647837881e3bbda15c4bfbf7b12d02beaa20987c0bc2c73e9625a2aeb) —
confirmed successful. Live-verified: `admin()` = the issuer address above, `name()` =
`"MOCKUSDY:GB3SOEMQVYBLMHPV3AK64PV4FJTOMQ2XUJLOZSVDJLP2LT42EU3XLE4P"`, `decimals()` = `7`.

**Price source:** this asset has no price of its own — `OracleAdapter`'s SEP-40 pull path is
wired to the real RedStone feed contract (§2 above), queried using the **real USDY SAC address**
(`CB3YA656OYIHU57657I5KGSBRHE5I3OZU4VFC22PYAOANFZHEWNYGAGP`) as the lookup key, not MOCKUSDY's own
SAC — see §4 for the exact wiring once the shared OracleAdapter is deployed.

### Key roles for this deployment

| Role | Alias | Address | Notes |
|---|---|---|---|
| `ISSUER_KEY` | `mainnet-mockusdy-issuer` | `GB3SOEMQVYBLMHPV3AK64PV4FJTOMQ2XUJLOZSVDJLP2LT42EU3XLE4P` | real issuer of the MOCKUSDY classic asset; signs every issuer-gated `initialize` |
| `ADMIN_KEY` | `mainnet-admin` | `GDZN5WVOUBRTZKADUULRXJOGVGNK4WWXJPSALTVOORLSU5NFOZ2P5NCB` | admin of OracleAdapter/Permissioning/RiskControl/Router; also used as `PROTOCOL_ADMIN_ADDRESS` for this internal-testing deployment (consistent with H-2's open key-concentration recommendation — not the model a real production launch should use) |
| `TREASURY_ADDRESS` | `principal-mainnet-treasury` | `GBUVAHYJNCXZWUK3NIOVJ6EYQRSXMVT2XOUEASS3VAG4EWIXXWYSVSXW` | receives Principal's protocol fee share |
| internal test accounts | `mainnet-test-a`/`b`/`c` | `GCOMBIYWZ3HIMAS7FLFERQMVZIDVIMFZAAV7A2KMCDV3RAKP5NL5R5BM` / `GAVGI2TA5S4PDPYLD5GDQIALIOIBXP44VL65LEEVQ4YKB6RBOZ7WFVS6` / `GDHF73TQO774WWC2W46CIYUZDZUB372OIQHZ5KYWKQCFUHEIJPEX2BQ3` | pre-granted compliance standing for internal flow testing |

### Funding and trustline transactions (all confirmed successful on Horizon)

| Action | From → To | Amount | Tx hash |
|---|---|---|---|
| create-account | treasury → mockusdy-issuer | 5 XLM | `6d6513dfe9db2b932b7b2374cb8f86add800ad07ed947f61a005a3c11fa0f256` |
| payment (top-up) | treasury → mainnet-admin | 5 XLM | `56a51e645955e55006471fe897e8db8b7d7c834dc7fec3c352a9f8f502d3101b` |
| set-options (revocable+clawback) | mockusdy-issuer | — | `a65d5bdafee7da783f72a03e30d2b6c368a3189350e60e23439e8175feacdc80` |
| contract asset deploy (SAC) | mockusdy-issuer | — | `33f9393647837881e3bbda15c4bfbf7b12d02beaa20987c0bc2c73e9625a2aeb` |
| change-trust (MOCKUSDY) | treasury | — | `a1ca431f93d7f81b5b08cc36fdb800f483ec1885ee64b514943c99620b019be5` |
| payment (top-up, for trustline reserve) | treasury → mainnet-test-a | 3 XLM | `c69c82764dea39ca703f5fa22fdceb1e2d65e928dee5d02edab2f636c0fb2641` |
| payment (top-up, for trustline reserve) | treasury → mainnet-test-b | 3 XLM | `0840bed66951cd7d2c06d3ba50db77ba7d2e3d10d31008a99508bf2a542cd1db` |
| payment (top-up, for trustline reserve) | treasury → mainnet-test-c | 3 XLM | `7d8e5bb71d3b3dbc79be3175572d1747093dcd0330d18def8fd25851b4f926ba` |
| change-trust (MOCKUSDY) | mainnet-test-a | — | `0d0686a472c00d0306ed5147e67672a21bfd480ce19565e73ae7925b12be437f` |
| change-trust (MOCKUSDY) | mainnet-test-b | — | `e89a1062e33034388c7b97eee7d778a97d1dc05e58c879f19c8f9c383fd6477a` |
| change-trust (MOCKUSDY) | mainnet-test-c | — | `061be28d1b506fa4a64729fe038fcbd8a70c1bd4a433e59e69ccb9d8bfa13bda` |

All four MOCKUSDY trustlines live-verified via Horizon: `is_authorized: true`, `is_clawback_enabled: true`, balance `0` (pre-mint).

**First deployment attempt failed with `TxInsufficientBalance`** on the very first WASM upload
(`oracle_adapter`, 43,205 bytes) — mainnet Soroban resource/rent fees for a contract-code upload
are materially higher than the 5 XLM originally funded to each working key covered. No funds were
lost (confirmed via Horizon: `mainnet-admin`'s balance was unchanged after the failed attempt).
`deploy.sh` itself has a related robustness gap worth fixing separately: `upload_wasm`'s failure
inside a `hash="$(upload_wasm ...)"` command substitution did not trigger `set -e` the way a
plain command failure would, so the script kept running with an empty contract id and produced a
cascade of confusing downstream errors instead of stopping cleanly at the real failure. Not fixed
in this deployment round; tracked as a follow-up to `scripts/deploy/deploy.sh`.

**Top-up funding (confirmed successful):**

| From → To | Amount | Tx hash |
|---|---|---|
| treasury → mainnet-admin | 20 XLM | `6665b7f861c377ec2cb6734964d483658849a46556bdb56f2befe5ea4abb2e29` |
| treasury → mainnet-mockusdy-issuer | 20 XLM | `2dc9289490058ddddc45ce8a0746de687ca3dfe8191b0845722c7e6636e6b4fa` |

Balances immediately before the retried deployment: `mainnet-admin` 26.6576618 XLM,
`mainnet-mockusdy-issuer` 24.9176668 XLM, `principal-mainnet-treasury` 44.7001300 XLM.

### CRITICAL FINDING: mainnet Soroban WASM-upload resource fees are far higher than assumed — deployment paused

**`permissioning` (11,308 bytes) deployed successfully** — `oracle_adapter`'s upload then failed
again with `TxInsufficientBalance` even at 26.66 XLM funded. Queried the RPC's
`simulateTransaction` directly for `oracle_adapter`'s upload (43,205 bytes, not yet successfully
uploaded) to get the exact number rather than guessing: **`minResourceFee` = `534563659` stroops =
53.4563659 XLM, for this one WASM upload alone** (not counting deploy+init). Cross-checked against
`permissioning`'s own actual cost (`mainnet-admin` dropped 26.6576618 → 10.6898814 XLM, i.e.
~15.97 XLM, for its upload+deploy+init combined) — the fee scales roughly linearly with WASM byte
size (~0.0012 XLM/byte), consistent between the two data points.

**Contracts deployed so far:**

| Contract | ID | Deploy tx | Init tx | Cost |
|---|---|---|---|---|
| `permissioning` | `CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ` | [`e061c1f781c76f14277c92c539b8ad585ae2f5c392d0c8f99fdcef4f58003f37`](https://stellar.expert/explorer/public/tx/e061c1f781c76f14277c92c539b8ad585ae2f5c392d0c8f99fdcef4f58003f37) | [`d92625d1c63ae551d716e70aa1fcf1ffb8ca454a2392a8402f571f626608cd64`](https://stellar.expert/explorer/public/tx/d92625d1c63ae551d716e70aa1fcf1ffb8ca454a2392a8402f571f626608cd64) | ~15.97 XLM |

**Projected total cost, extrapolating linearly from these two real data points
(~0.0012 XLM/byte) across every contract this plan still needs to upload:**

| Scope | Total WASM bytes | Rough projected cost |
|---|---|---|
| Remaining shared infra (oracle_adapter, risk_control, router, market_factory) | ~167,282 | **~206 XLM** |
| First market's 7 per-market contracts (sy_wrapper, pt_token, yt_token, market_config, principal_manager, market_pool, recovery_escrow) | ~293,311 | **~363 XLM** |
| **Total for just the first (90-day) market, on top of what's already spent** | ~460,593 | **~570 XLM** |

A second market for the same asset (the 30-minute market) would likely cost much less for the
per-market contract *uploads* specifically if Soroban deduplicates identical WASM by hash (not yet
confirmed) — but would still incur fresh deploy+init fees for every new contract instance.

**This is roughly 7–10x the entire treasury balance available for this deployment** (started at
83.7 XLM; ~60.3 XLM remains across the three working keys as of this point). **Deployment paused
here — no further transactions submitted — pending a decision on how to proceed.** Nothing beyond
`permissioning`'s own ~16 XLM has been lost; the rest of the attempted transactions failed at
submission with no funds debited (confirmed via Horizon balance checks after each failure).

## 4. Shared protocol infrastructure

**Status: COMPLETE** as of 2026-11-17, resumed after the funding pause (§3.1 above), using
`scripts/deploy/deploy.sh`'s new `STOP_AFTER_SHARED_INFRA=true` flag (added this round,
backward-compatible, default off) to stop cleanly before the market phase. The run itself hit
the same two issues already on file from the first attempt — a `TxInsufficientBalance`-adjacent
client timeout on `router`'s and `market_factory`'s uploads, and the pre-existing `deploy.sh`
manifest-misattribution bug this document already flagged (§3.1) — both worked around by hand
(resubmitting the identical already-signed transaction, which succeeded the second time; the
manifest was corrected directly). Full verification:
`MANIFEST=deployments/mainnet-mockusdy/deployment-manifest.json
./scripts/deploy/verify-deployment.sh` → **19 passed, 1 failed** (the expected `git_tree_dirty`
flag only).

| Contract | ID | Key transactions |
|---|---|---|
| `OracleAdapter` | `CCKXG4HSVH57AQTYJDNNXHQBCGNH6KRFNMCMATTMYFUX2TDQ7GD5K7IT` | deploy `7fbf805d54ef08a332dd58a620606ca82dd36d25b31070fa9f9eff11ee1c8ed5`, init `a8e39609fb856d5818fb178f28a56c0c95d0141f816e5216190536866452241d` |
| `Permissioning` | `CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ` | (from the first, pre-pause attempt — see §3.1) |
| `RiskControl` | `CBHCRRLWHS7KRQNAXYY3OMNDTSXDNJKCDK6KPUUJPFGDBSN76JEWWQ5G` | deploy `05e79d4da2dc06e7a25e0b381923e886926802ecb1a09d2ea991856a0b9e9b40`, init `f4274c9a83bb0c2653a7685fbae310877ad3de3fa44123c7fc0911bf700691c3` |
| `Router` | `CCM4QQUKTD2CUTRSNNI6OO4JHYEF7MHKBMKSM5JYJNFLKSTAZBQD7QF5` | upload `d9cd44116ca70b67a11d1503d9f3c352c70c4926495edf23de4d8008efccfa4b`, deploy `cc24f84c48a703a68f0f16373250a1a4ddf388df2cb88494168144812f620921`, init `4cc9172b0a24c4f566bee43193bf98a613e6f65422e0f7139e61128d3d6614dc` |
| `MarketFactory` (stateless) | `CDYH7UV24XS7G7MDRTBJUS4PJNKKUXTLUUTL67MR5CYJAXSPPHNWS65T` | upload `67363c8020a4609f9bf4808f2dd8faabd6d0b45ea0049dc71b25f390a9469e85`, deploy `7ab8f3ca627588608ca54d70a7232ecffa0714e3c9294f09b90f284331beb7af` |

**Oracle genesis + real SEP-40 feed wiring** (done by hand after the automated run stopped
short of this step, since the mid-run failures above prevented `deploy.sh` from reaching it):
manual genesis push of `11,534,743` (real USDY price re-checked live at this point — unchanged
from §2/§3's earlier reading) — tx `f186c699affbc6267112a166cb37b0c4bb611433ce598ea2e2820ac148d19434`;
`set_feed_source` to the real RedStone feed / real USDY SAC — tx
`cfc891dea1cec242f442c9b585a9b69fa0b64842331bc4671a4023559f1e9d1d`; the zero-then-restore
throttle bootstrap (`set_feed_min_interval` → 0, `sync_from_feed`, `set_feed_min_interval` → 300)
— tx `ef2cf87e51f316877a2dd982c8ddcee141bd67f26f99b8d54386a6d6eca87ae7`,
`9a78fad9a8e48609e178d564db2ecf1bb1f40c144913b551f81e6e28e745d9b0` (correctly rejected
`stale_ts` — the real feed's own last tick is older than this genesis push, exactly as seen on
testnet §6; the manual value remains in effect), `e49b979a04f4b915970b085adf7e7f57bd5bbf01bd8a39429eeb0ef6b7568360`.

### Real cost data from this round (useful for the next phase's budgeting)

Two transactions genuinely never landed on-chain the first time (`transaction submission
timeout` — confirmed via a 404 on Horizon, unlike an earlier payment this document records that
*had* actually succeeded despite the same client-side symptom) and were resubmitted identically,
succeeding the second time with no double charge. Real charged fees for the two contracts fully
measured this round came in **~13% below** the pre-upload `simulateTransaction` quote (e.g.
`risk_control`'s combined deploy+init cost only `0.076 XLM`, not the several XLM a flat per-byte
model would suggest) — the per-byte upload quote is a conservative ceiling, not the exact charge.
Balances before this round: `mainnet-admin` 240.6898814, `mainnet-mockusdy-issuer` 24.9176668,
`principal-mainnet-treasury` (unchanged at the time) XLM. After completing all of shared infra
(including the two retried uploads and the full oracle bootstrap): `mainnet-admin` 60.0045044,
`mainnet-mockusdy-issuer` 24.9176668 (untouched this round), `principal-mainnet-treasury`
186.3511731 XLM — **≈ 271 XLM remaining across all three keys**, consumed **≈ 181 XLM** to finish
all of shared infrastructure (lower than the ~215 XLM upper-bound estimate given when funding
was requested).

### `scripts/deploy/deploy.sh` changes made this round

- Added `STOP_AFTER_SHARED_INFRA=true` (default off, fully backward-compatible): exits cleanly
  right after shared infrastructure is deployed and recorded, before the market-creation phase.
  Added specifically to let this deployment be funded and executed in two safe, independently
  budgeted stages rather than risking a stall mid-market-creation again.

## 5. Markets

### Market 1 — 90-day reviewer market — **STATUS: COMPLETE AND LIVE**

Completed in full on 2026-11-17 after a final funding round. All 7 per-market contracts were
uploaded (each balance-checked against the real fee before proceeding), the market was created
atomically through `MarketFactory`, wired, and every compliance grant applied. **Full
verification: `MANIFEST=deployments/mainnet-mockusdy/deployment-manifest.json
./scripts/deploy/verify-deployment.sh` → 78 passed, 1 failed** (the expected `git_tree_dirty`
flag only — every real check passes).

**WASM uploads** — all 7 landed at the same ~13.0-13.1% real-cost-below-quote discount observed
on every single upload this entire deployment (9 for 9, counting §4's two):

| Contract | WASM hash | Upload tx | Quoted (XLM) | Real cost (XLM) |
|---|---|---|---|---|
| `sy_wrapper` | `be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a` | `abe8ed52023709c790716f02688a8e3363e3a5b895e53b8faa7fd1cb43e6d174` | 49.9493112 | 43.4346278 |
| `pt_token` | `db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2` | `5e90ef46a93fc20f9150c9d3c95e1129397738d07fe006ffa086f4a130c7233a` | 42.1388872 | 36.6427374 |
| `yt_token` | `c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654` | `233514e0ebab444e40ce11e3845ffa4b5285cac3ec94c5fde1bee581aebb5268` | 60.7313451 | 52.8103563 |
| `market_config` | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` | `22432394de18898744edc16571eedea3ad42612042b4e8786c620bea50f3e5df` | 34.1468408 | 29.6930028 |
| `principal_manager` | `e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115` | `3dcaca908ae2dceb2200679c05d2a0c0b1a3e0ad4f8fc33465d5a80eb5dd52a2` | 64.3810413 | 55.9840685 |
| `market_pool` | `52285a25bbf372d0e5d22ba565b76af6a7e832422df83b6a4809de8adf3aa112` | `773f9908fe82d1043e0e5a25929501cd04b9cc4334f4c69f8f131b86315ec9e4` | 108.5089046 | 94.3568661 |
| `recovery_escrow` | `dadea71fcfb84adcfb4691f472551f490cf440c5aaae5c6b5e5d4bb6f61441e8` | `84cfb2485b906f6601c3e4b827e6c49f3ab6b35837ea7683bfe9800f7070e2f0` | 55.8143835 | 48.5346951 |

All signed by `ISSUER_KEY` (`mainnet-mockusdy-issuer`), matching `deploy.sh`'s own convention and
the contract requirement that `ISSUER_KEY` be the real issuer authority relayed into these
contracts' `initialize` calls. The oracle's genesis value needed a fresh push immediately before
the factory calls — it had aged past `max_stale_seconds` (3600s) again since §4, reverting
`YTToken::OracleStale` (error #10) on a `create_tokens` simulation. Refreshed via
`set_reference_value` (tx `2b2b3783462c5bb1d4d36ae68c30c27853c93a1615ce41e6e343a94ac55f6f9e`) —
a direct, real demonstration of why the oracle relay needs to actually run on a schedule in any
live deployment, not just once at genesis.

**`MarketFactory.create_tokens` was measured live before committing to it: only 0.3665618
XLM** — confirming the factory's `create_*` calls are cheap (they deploy already-uploaded code
into new instances, not new WASM), resolving the one open cost-uncertainty flagged earlier in
this document.

**Deployed contracts:**

| Contract | ID |
|---|---|
| `SYWrapper` | `CCY5P6CCHIPX7ND3HSIHFLD57756WWILI4UQLSZ2UIF5CVGYN6XJGJ7Q` |
| `PTToken` (`PT-MUSDY`) | `CASF4U23SQ6XPCKPVNQ273OM3ILZRJFNHTHW6KGZRXQD4QK65PWWLEFE` |
| `YTToken` (`YT-MUSDY`) | `CCQ2VU23X5GY7ZBTYNKWGLXRQVODSNSCNJVOLXG7NHLEXT37FDAKKPGC` |
| `MarketConfig` | `CAV62P3JRTBVBOCCZ4GW3X7AAW4NJ35LP7KVM5TY6Q6OXROMOKI4WAQL` |
| `PrincipalManager` | `CBBKUZJWZDA7YLA6GHSXVLV62QJ6DLKCO4GXUWF43FXLPN5S3XVNUKCC` |
| `MarketPool` | `CAON3FAQU5W6SGZFWOM22GONNKZEIN4AGVTKMCWCC6PI43QWB27MGW4I` |
| `RecoveryEscrow` | `CBHHRTTRTXQM3OFDOBL6QORP6JS5RVIRXSF5B3C4TMOVSIQBDKKA67SJ` |

Maturity: `1802719669` (90 days from creation). `market_id` = `mainnet-CDQYZU-CCY5P6CC`.

**Factory + wiring transactions:**

| Step | Tx hash |
|---|---|
| `create_tokens` | `f6300538c4967d298e102dd86d17b37c20270e347590c934a536d9f660668fd1` |
| `create_engine` | `802e13bf669b923efeeb5ebc3b444460e6e72a912ea7d548c3b7d04991b75071` |
| `create_pool_and_escrow` | `d0599af33679a18ac1304ebd5c66b886bef9717b951be8a532eb1015ed1e1078` |
| `risk_control.add_consumer(SYWrapper)` | `2e1715687a5e5fcf0983f80f9785af7c29b06e44917f5a1cbd3c89bdc0e7abbc` |
| `risk_control.add_consumer(PrincipalManager)` | `a1ae25fb627589ad5c636e630c35edd74009b20bbd73ffc193e74b1d91dc359a` |
| `router.register_market` | `66739eb5acd1cfc255f7b44667a9de19e3deec92231506fc6e97d28c278d8282` |

**Compliance grants**: account-level standing (SAC `set_authorized` + `Permissioning.
grant_account`) for `SYWrapper`/`PrincipalManager`/`MarketPool`/`RecoveryEscrow`/treasury, plus
per-asset PT/YT grants for `MarketPool`/`RecoveryEscrow`/treasury — all applied and spot-checked
directly on-chain afterward (`authorized()`, `is_allowed()`, `is_allowed_for_asset()` all
confirmed `true`), not just trusted from the invoke calls' own exit codes.

**Total real cost for this market's 7 uploads: 361.456354 XLM** (vs. 415.6707137 XLM quoted —
13.04% discount, consistent with every other upload this session).
Combined with §4's shared infrastructure, **this is the first fully live, verified Principal
Protocol market on Stellar mainnet.**

Balances after completing this market: `mainnet-admin` 4.5996396, `mainnet-mockusdy-issuer`
67.3421048, `principal-mainnet-treasury` 6.3511431 XLM — **≈78.29 XLM remaining**, enough to
likely cover a second (30-minute) market for the same asset, since it reuses all 7 already-
uploaded WASM hashes and only needs cheap deploy+init+wiring+grant calls (see §10).

### Market 2 — 30-minute internal flow-test market — **STATUS: COMPLETE AND LIVE**

Created directly via `scripts/create_market_via_factory.sh` (not `deploy.sh`), reusing all 7
WASM hashes already uploaded for Market 1 — no new uploads, only cheap `deploy_from_hash` +
`initialize` calls for 7 new contract instances plus the usual factory wiring. Underlying is the
same `MOCKUSDY` SAC (`CDQYZUYQF7TRQ7BX6YBSCU3HLWBDS3E2VSP7AUE7C3E2D2U2IBBQOBGF`) as Market 1 — this
market exists purely to exercise a real, short-lived maturity→settlement→redemption cycle on
mainnet, mirroring the proof already done on testnet
(`TESTNET_MOCKUSDY_DEPLOYMENT_EVIDENCE_2026_11_17.md` §5 "Market 2").

Maturity: `1794946266` (≈30 minutes from creation). `market_id` = `mainnet-CDQYZU-CA74VH`.
Salt: `d5cf4691e6c18b70ec6abad0cef0d7ae8f91c3f9adb7f441ea32c63e3101a7e4`.

**Deployed contracts:**

| Contract | ID |
|---|---|
| `SYWrapper` | `CA74VHFDP5G6TQK274OW7S7UUDDBUASXTJUYZ5H2RAJVNJJKYJOY6S55` |
| `PTToken` (`PT-MU30M`) | `CAKNYWZCBZZML3Y7MZVFVVUAIDVB6EV6MR2MMDPDBYIGFE56ONMKR6JW` |
| `YTToken` (`YT-MU30M`) | `CCT6H57QP2YYDAES6OPTWJJOKI5SGLFFMK2G2TXGS4UMPJNCSPLYJWXS` |
| `MarketConfig` | `CAYOF3S5NBBK4XVHHFIKJQPWDZSMYX7OZ26MWLPQAT2EKWPVL44VN336` |
| `PrincipalManager` | `CB7VVJJNU44HPGOEVLH5D7E4BVAHUP5MAFXZUVC4RS34F5TSXSUUV3SV` |
| `MarketPool` | `CC3HM3YGRMJ4QHHZQCF2Q7XVMO3MIAJGZNBEYTOQSSGE76HCP25MMULV` |
| `RecoveryEscrow` | `CAET3XI5BM75DOABUEG5RTTNXXYADLTTM7EHTC2IJEDU5DR4BL4CVAGW` |

**Factory + wiring transactions:**

| Step | Tx hash |
|---|---|
| `create_tokens` | `9ef56e0fecfda733cfab952127b82b0c1332bd0fad8923c8a0ae7b305f50fd8b` |
| `create_engine` | `5afc1deab6787694cca25c7880d956fcc0ec3150e595d6b8e667ae57f52b9c78` |
| `create_pool_and_escrow` | `698240cba79b7f6b3f31eac3956a29dbae13b91a7cf48bce0729b8e29b834524` |
| `risk_control.add_consumer(SYWrapper)` | `b7e4a1d0736facb67b5d496dbe582f11a5005c413a9637cc9467bd2afd1778a2` |
| `risk_control.add_consumer(PrincipalManager)` | `430b3c8d1c7b48509b1d3956bfce380111eb001e934d995e7841f74b90c1cfdc` |
| `router.register_market` | `715cdb37837ce0bb00bed7531cffbfa2ca3387ff32924f50cd5ecb0a674d3591` |

**Compliance grants** (account-level `grant_account` for SY/Manager/Pool/Escrow, plus per-asset
`grant_asset` for Pool/Escrow on PT/YT): 6 transactions,
`207a7aa6c9a6d0def8d01a6a3b2cd909d61a56ca1fa7eba22f0cebbc1461603e` through
`32a010f8eb1049a240a5eb40ebe99346298116f33c92234e9b8fa023e4e70d43` (see the manifest's
`transactions` array, steps `market2:compliance_grant_1..6`, for the exact mapping). The oracle
needed a second genesis refresh immediately before this market's `create_tokens` (same
`OracleStale` pattern as Market 1 — real elapsed time between funding/upload steps keeps outrunning
`max_stale_seconds`).

### Real transaction testing (both markets) — 2026-11-17/18

The first real end-to-end flows run against Principal Protocol on Stellar mainnet. All with a
fresh test account (`mainnet-test-a`,
`GCOMBIYWZ3HIMAS7FLFERQMVZIDVIMFZAAV7A2KMCDV3RAKP5NL5R5BM`), minted real MOCKUSDY, real
signatures, real on-chain state — nothing simulated or assumed.

**Permission gap found and fixed live:** `PTToken::mint`/`YTToken::mint` each call
`Permissioning.is_allowed` (account-level) **and** `is_allowed_for_asset(account, this_token)`
(per-asset) on the *recipient*, not just on the market's own contracts. Granting only the market's
four contracts (as the factory-adjacent compliance step does) is not sufficient for a human trading
account to ever hold PT/YT — it also needs its own per-asset grant for every PT/YT token it will
hold. First attempt at `PrincipalManager.mint` on Market 2 reverted `Error(Contract, #7)`
(`PermissionDenied`, confirmed by reading `contracts/pt_token/src/lib.rs`'s `Error` enum) until
`mainnet-test-a` was separately `grant_asset`'d for `PT2`/`YT2` (and later `PT1`/`YT1`). Not a bug —
this is the two-layer compliance design working as intended — but worth recording: **every real
trading account needs its own per-asset grant, separate from the market's own contract-level
grants**, and the self-service tooling's README already says this explicitly (§6) for exactly this
reason.

| # | Action | Market | Tx hash | Result |
|---|---|---|---|---|
| 1 | `Permissioning.grant_account(test-a)` | — | `358bb2753923e9413a78907df7a5fb08af1f5b2adec8f5c400a0b22f7d3369c9` | granted |
| 2 | Mint 1000 MOCKUSDY to test-a | — | `2add04a0d80106971726147966a9d85d561beff8b75cedffe418b3b366ba83a4` | 1000.0000000 MOCKUSDY (a first attempt at `--amount 1000` sent 0.0001000 — the CLI's classic-asset `--amount` is in stroops, not decimal units; corrected) |
| 3 | `SYWrapper2.deposit(500 MOCKUSDY)` | 2 | `f3fcba50d1f2605cbe153ef9454f7119ed931d5ca0ecbea63c727d2adaea5726` | 500 SY shares minted |
| 4 | `Permissioning.grant_asset(test-a, PT2)` | 2 | `9db32a368b5c14f1862bbd8268d5c060ba177f28747361473a69bddac6e6822d` | granted (fix for the gap above) |
| 5 | `Permissioning.grant_asset(test-a, YT2)` | 2 | `a89a068720391c4ba3b623d91bb32a27b7650cc6364db3d05f9f2071b01159ee` | granted |
| 6 | `PrincipalManager2.mint(500 SY)` | 2 | `12f90a7b41736bceb318d13e4a413f92eedf34823f24900db492512a0375fee3` | 5,764.487814 PT2 + 5,764.487814 YT2 minted, fee 2.5 SY withheld |
| 7 | `PrincipalManager2.claim_yield(test-a)` (pre-maturity) | 2 | `5686cbee284613a5a3be9392ee51518e8686a194640acb3df3fef58962fb6754` | `0` — correct, no yield has accrued yet (oracle price unchanged, seconds since mint) |
| 8 | `Permissioning.grant_asset(test-a, PT1)` | 1 | `fc64092f20768394e142a64bbba1a4a89add3622af79a71ffff4fa1be6a237fc` | granted |
| 9 | `Permissioning.grant_asset(test-a, YT1)` | 1 | `71dbd9e12b298ba3c8990abcb1faa74bf16cb5d5008a38a2ba5ccb1d8360fe48` | granted |
| 10 | `SYWrapper1.deposit(200 MOCKUSDY)` | 1 | `972727f72064f9acd6955339a84c544157bcb5ebc113c19ee3cd25a325ef0776` | 200 SY shares minted |
| 11 | `PrincipalManager1.mint(100 SY)` | 1 | `ddbdd13683ffd9b93509d9c223556aaa04d57c2d0870195db304580b8e2b4906` | 1,152.897562 PT1 + 1,152.897562 YT1 minted, fee 0.5 SY withheld |
| 12 | `MarketPool1.add_liquidity(60 PT, 50 SY)` | 1 | `516490ef54181d4ca2b65c6ceed1e3596499efcd4d04b658249b1b8ed4b9b7a9` | **first AMM liquidity ever seeded for this protocol on mainnet** — 54.7721557 LP minted |
| 13 | `MarketPool1.swap_pt_for_sy(5 PT)` | 1 | `1bff923cff8eb97a2c810bc4a22e38b4c12f4f8ef69c41fd0799da62215c8afa` | **first real AMM swap on mainnet** — 4.3005615 SY out, fee 0.0010605 SY |

Every result above was read back from the real transaction's own emitted events (`sy_xfer`,
`mint`, `add_liq`, `sell_pt`), not assumed from a successful exit code.

Market 2's maturity (`1794946266`) was tracked live. Once it passed:

| # | Action | Tx hash | Result |
|---|---|---|---|
| 14 | `PrincipalManager2.settle_all()` (1st attempt) | `6def077f8f172846ab4488964153e33ae87fa904ad09119933181081542a457b` | `transaction submission timeout` — confirmed via Horizon 404 that it never actually landed, so safely resubmitted identically |
| 15 | `PrincipalManager2.settle_all()` (resubmit) | `d3b8edd47628d50c90e8a275e492e19ec86b8e088decdcf93da1f130d7aa3ac8` | settlement rate frozen at `11534743` |
| 16 | `PrincipalManager2.redeem(test-a, 5764.487814 PT2, 5764.487814 YT2)` | `b3a0f1a55fea5a959535590181250951ac0e78e44fbd26220ffe708a1b9c8e04` | `499.7499999` MOCKUSDY from the PT leg, `0` from the YT leg (no yield had accrued — consistent with step 7's pre-maturity `claim_yield` also returning `0`, since the real USDY price never moved during this market's 30-minute life) |

Final `mainnet-test-a` MOCKUSDY balance: `799.7500999` (1000 minted − 500 into Market 2 − 200 into
Market 1 + 499.7499999 redeemed back from Market 2 ≈ matches, modulo the tiny tokenization fee and
the original 0.0001 MOCKUSDY stroops/decimal mix-up in step 2).

**This is the first complete, real, on-chain Principal Protocol market lifecycle — deposit → split
into PT/YT → mature → settle → redeem — proven end to end on Stellar mainnet**, mirroring the
equivalent testnet proof exactly, with every step read back from real emitted contract events.

### MOCKUSTRY market — second mock asset — **STATUS: COMPLETE AND LIVE** — 2026-11-17/18

First of the remaining mock assets (USTRY, GILTS, CETES, XAUM) from the original Tranche 3 plan.
Tracks real USTRY (Ondo's tokenized short-term treasuries, per §2's RedStone feed lookup) the same
way MOCKUSDY tracks real USDY: own classic asset + SAC, same issuer flags
(`auth_revocable`/`auth_clawback_enabled`), priced via a **new, dedicated `OracleAdapter`
instance** keyed to the real USTRY SAC (`CBLV4ATSIWU67CFSQU2NVRKINQIKUZ2ODSZBUJTJ43VJVRSBTZYOPNUR`).

**Correction confirmed by this round's real costs**: contrary to §10's earlier (now-corrected)
estimate, a new mock asset's `OracleAdapter` does **not** need a new WASM upload — the code is
already installed on mainnet, so only a cheap new *instance* (`deploy_from_hash` + `initialize`)
is needed. All 7 per-market contracts likewise reused their already-uploaded hashes from Market 1.
`Permissioning`/`RiskControl`/`Router`/`MarketFactory` were reused as-is from §4 — never
redeployed; only the fresh manifest (`deployments/mainnet-ustry/deployment-manifest.json`) was
pre-populated with their existing IDs so `deploy.sh`'s own idempotency check would detect and
reuse them rather than creating duplicates.

**Funding**: treasury topped up by 20 XLM from `mainnet-mockusdy-issuer`'s surplus (payment tx
`f9fc5e03e80629410e93fe727df4e4b0ab2bb906abac6c8e403c7679bcc80a7d`) — that issuer never needed
anywhere near its original funding once uploads turned out to be mostly reuse — then a new
`mainnet-ustry-issuer` (`GDM2Q2F5KC2ECQZ5C3TIPTM7UTZJ775G4UZCLLGYULZUMBI6VEQERK4K`) was
created and funded with 3 XLM from treasury (tx
`b22bfdc532c9ff02ac394a2ff163a47f764e99376669081790dc30a1ad298cda`), flags set to match real
USTRY's issuer (tx `201b32a7aada1a2b9e8c084de5ea458c6970558b95840534e0e4a25bc4e8f025`,
live-verified `auth_revocable: true, auth_clawback_enabled: true`), and the SAC deployed (tx
`07b4a96d80ef4b81423a8277ec36307211bc70aa44b1c861cc9342252f3c115d`) →
**`MOCKUSTRY` SAC: `CDHQ4B6U3D5O6PGAWAO22K45ASD2BAGD25IHITRIIUHXA3GBABZTYBA5`**.

**Real incident, found and recovered live**: `scripts/deploy/deploy.sh` was run via
`ENV_FILE=.env.mainnet-ustry`, which deployed a fresh `OracleAdapter` instance
(`CBHAWS6B4YCNRLYRP2HUP77P4IRP3PZYPSYVPV4OMZNU2ZNKSAQQ2GXU`, genesis value `108065700` = real
USTRY price) and then called `MarketFactory.create_tokens` successfully — but the shell process
running `deploy.sh` was killed (host-side, exit 137) before `create_engine`/
`create_pool_and_escrow` could run, leaving a real, orphaned `SYWrapper`/`PTToken`/`YTToken` triple
on mainnet with no `MarketConfig`/`PrincipalManager`/`MarketPool`/`RecoveryEscrow` and nothing in
the manifest. **Recovered without creating a second, duplicate market**: `MarketFactory`'s
`create_tokens` event (`mkt_tok`) was queried directly (`stellar events --id <factory>`,
decoded with the same `stellar_sdk` approach as `scripts/self_service/decode_mkt_new.py`) to
recover the exact `salt` (`1d998d8b76f8afe73b3f773a89890805e5313ed42f0ccb8ef8d0002b4901d6b7`) and
`sy_wrapper` address already created; `predict_market` with that salt confirmed the deterministic
addresses for the remaining five contracts; the real `maturity`/`decimals` were read back live
from the already-initialized `PTToken` rather than guessed. `create_engine` and
`create_pool_and_escrow` were then called by hand with that exact salt, completing the same market
`create_tokens` had already started — verified by `predict_market` returning the same addresses
both before and after. **Lesson for future asset rounds**: run `deploy.sh` with a longer
foreground timeout (or `run_in_background`) for the per-market phase specifically, and recover via
the factory's own events (deterministic salts make this always possible) rather than ever
re-running `create_tokens` with a fresh random salt, which would silently create a second,
unrelated, equally-real set of contracts.

**Deployed contracts:**

| Contract | ID |
|---|---|
| `OracleAdapter` (USTRY-specific) | `CBHAWS6B4YCNRLYRP2HUP77P4IRP3PZYPSYVPV4OMZNU2ZNKSAQQ2GXU` |
| `SYWrapper` | `CA4EMPWQ3NBUHRA75DXCGRTSGROD6YNFFPXUEEKVC262P5GKK6EPVTSX` |
| `PTToken` | `CAYXXH7ZY54CENRIJFDGYIBIJJDNUOV2TUNFKO6QVUZYYDFGQGXI4HH7` |
| `YTToken` | `CCUDPECIHOJK2RLFIDF7K2NM4H77V3JUAUZODU6OMT5QWVVYUMXXVKZG` |
| `MarketConfig` | `CBWIVLMDZCV7USXM7TVUF4KW6HJUVDXMXMOKIVCVHEC2DN6GTPHFSYHP` |
| `PrincipalManager` | `CAB6LHML2KZH4SRV5NLTDFIX5PLM6F7R54WFOTLK5XXJ6TVDL7MNRD44` |
| `MarketPool` | `CBJGWQ7BJ7AND2MNG4RCA626PPX4RTDKLJGNG6UVXFTOYF6GGQ7T5GZ6` |
| `RecoveryEscrow` | `CC5AFSYUTYVYYLZID64UE7L74TW2JMONZGOAWDGPBQSWA6ANEOOII2RE` |

Maturity: `1802724716` (90 days). `market_id` = `mainnet-CDHQ4B-CA4EMPWQ`. PT/YT token names were
left at their generic defaults (`"Principal Token"`/`"PT"`, `"Yield Token"`/`"YT"`) since
`PT_NAME`/`PT_SYMBOL`/`YT_NAME`/`YT_SYMBOL` weren't set before `create_tokens` ran — cosmetic only,
immutable now, harmless.

**Verification**: `MANIFEST=deployments/mainnet-ustry/deployment-manifest.json
./scripts/deploy/verify-deployment.sh` → **78 passed, 1 failed** (expected `git_tree_dirty` only).

**Real smoke test** (same `mainnet-test-a` account, after establishing its own `MOCKUSTRY`
trustline — tx `8981e8a4976262855aed9c98666622dfe900b00e2016b03efa038f1937b75545` — and a
`set_authorized` retry once that trustline existed, tx
`90c24dffed12b1edba1e289fb76043dd14dcb726c77468d28d2b535437ccaa05`): minted 1000 MOCKUSTRY (tx
`fb9b93f3ec98795196ea7dcffeea792852125e24288ee4c1ec81e0c5f1e94f41`), deposited 500 into
`SYWrapper` (tx `bef5117ea188a5389642b4ed78cac0b4cec412d7bbf3a27269ce22212427e9fd`), minted PT/YT
from the resulting 500 SY (tx `d3691d05aa26e440fdbde785a1d68811c8eef80af53d37374908ab668c489298`,
first attempt timed out but 404'd on Horizon and was safely resubmitted) → `54005.833575` PT and
YT minted, fee `2.5` SY withheld.

Balances after this round: `mainnet-admin` 3.5252852, `mainnet-ustry-issuer` 1.8018105,
`mainnet-mockusdy-issuer` 46.2218571, `principal-mainnet-treasury` ≈20.35 XLM — comfortably enough
for GILTS, CETES, and XAUM to follow the same pattern.

### MOCKGILTS market — third mock asset — **STATUS: COMPLETE AND LIVE** — 2026-11-18

Same pattern as MOCKUSTRY, done cleanly this time (manual phase-by-phase factory calls from the
start, applying both lessons from MOCKUSTRY: no monolithic `deploy.sh` run subject to a host-side
kill, and `mainnet-test-a`'s classic-asset trustline established *before* its `set_authorized`
call, not after). Tracks real GILTS (per §2's RedStone feed lookup,
`CCQNEBGX5RJQGNLQ25C67DZLOSNM4NFQBANUS3ASALF2X552BSIQ6XCE`, real price `141972700` at the feed's
8-decimal scale). Zero transaction timeouts this round.

**Setup**: new issuer `mainnet-gilts-issuer`
(`GA5KXEPU7FLWKNG255WKYSYQ5U3G7WHFKD7HTTK2DR37KYJJ7V6GOR3G`) funded with 3 XLM from treasury (tx
`6c34224e2d22892fcc44bc7dd1953a613cff9e418334823aa00cbcd0730cf1d7`), flags set to match the real
GILTS issuer (tx `b7ede1c47a17e53178c68f0049fbe244ced2d681ce44a6ff00279f4c3dcccd79`, confirmed
`auth_revocable: true, auth_clawback_enabled: true`), SAC deployed (tx
`ca114f96670387e3741016db4a451dbb6d2dde4dbb5479ebebee2614b9d6fe6b`) →
**`MOCKGILTS` SAC: `CBBF4H2ZNMJKULJEOMZDMVQP27LD7HRTHEHETD647P3SCBTAWRGVJG4O`**. New
`OracleAdapter` instance deployed from the already-installed hash (tx
`60143de045ced4f69741ad4ca4133d6aa387c6c677aba5383f404e73deb274ff`, init tx
`0a3febe495a1e2656e097a8a498e04ba05780302090f71342fe72706c15f26bd`) →
**`CDZS34JFW7D24MONUKQDVJZKGBPOMIEVEWQQAHTEG3LYR2VFI4XNFNSZ`**, genesis value `141972700` pushed
(tx `ed8b732c22702b8772ee270da7e4e20e9fb775e322d79329cf9287b2c5c3c421`).

**Deployed contracts** (PT/YT given real names this time —
`"Principal Token MOCKGILTS"`/`"PT-MGILTS"`, `"Yield Token MOCKGILTS"`/`"YT-MGILTS"`):

| Contract | ID |
|---|---|
| `OracleAdapter` (GILTS-specific) | `CDZS34JFW7D24MONUKQDVJZKGBPOMIEVEWQQAHTEG3LYR2VFI4XNFNSZ` |
| `SYWrapper` | `CDXXRJJS2HFJBYF344ZGIC4DDRC7HAM4TJBTOM3ISFWE42KNRVAM3ZBQ` |
| `PTToken` (`PT-MGILTS`) | `CCHWW2AZ7JJQKMVNB2XFJ4SKZCCMSANLVBUZLGRCT46SVENVWD4B4LAN` |
| `YTToken` (`YT-MGILTS`) | `CADNGBDYZIRG2L6PPRSER6DSMIJFFAAIV25KE4F5KZIO7QGAF4LLIGXZ` |
| `MarketConfig` | `CCG7TK57WPPBJ2OP3YJLCGK6HOANGUPNVFAFBXJHJSWNRSDOKKZFH2VO` |
| `PrincipalManager` | `CBOJIJBR477VTPSE6VB7W4ZEQZKNJ3NEVI4FPORAEC4OMXQBIGLBMUJG` |
| `MarketPool` | `CDWVSTG3UZXZOEF2FGKJXFZFGEYEO3HUH7IFZNHOEFSQYZ575GAUYQH4` |
| `RecoveryEscrow` | `CAOYAMW4FFHLL7BYBMDLLWIN4LIRZFZA2Z4ZUNDBYCROC2U7JJMC2CUK` |

Maturity: `1802727891` (90 days). `market_id` = `mainnet-CBBF4H-CDXXRJJS`. Salt
`485b2d22039af0f6ddcc7dddd63e470aca222aeed8c4f537856d7e546f17713d`.

**Factory + wiring**: `create_tokens` tx
`5b4e44362c4e8a57fd9f088a5799015a65fe926eaf5bcd8111139f841ab48ced`, `create_engine` tx
`876e6b39a9e45afb6f6e828f31b725667e81236a5f3107eaf4e6e74ac4ac52d2`,
`create_pool_and_escrow` tx `f4b70aa5a5143b4cea46080d9db8b7a8b1364376fcfb26198ce00adbdcedb1d7`,
`risk_control.add_consumer` ×2 (tx `32d0f018fb32a8e93deefc73c4cdeedfc5fb2f82cda488f9bc3a1e1e86467107`,
`5f927ebdc77cd4f029eb9b5e551c09253ed8b5e3ad4287e5ebdc00b06fb6af85`), `router.register_market` tx
`14de3a4a963f42bdfeed171e072249733385c3db1399cd7b07f96895049d6f9f`.

**Verification**: `MANIFEST=deployments/mainnet-gilts/deployment-manifest.json
./scripts/deploy/verify-deployment.sh` → **78 passed, 1 failed** (expected `git_tree_dirty`).

**Real smoke test**: `mainnet-test-a` established its `MOCKGILTS` trustline first (tx
`e3758479c2ac652c12f7615625ae435a098483a842eaadeddc0b99002979171d`), then compliance grants
landed clean on the first attempt — minted 1000 MOCKGILTS (tx
`ef6e4ed5f9f5e7bd9f6f66c586285c3ae6296aa63ac11cd9fdb22868de8c37b6`), deposited 500 into
`SYWrapper` (tx `ce0a3dda55836e5fa84f5c30ecbd0f4539e1bc2af7d3d655ecee57e156d63927`), minted PT/YT
from the resulting 500 SY (tx `3536b1d5483e71bacdd72534e1cb8b709ea558e2f000cafffac60abd6aab0670`)
→ `70950.856825` PT and YT minted, fee `2.5` SY withheld.

Balances after this round: `mainnet-admin` topped up to `7.9884018` XLM (5 XLM transferred in from
`mainnet-mockusdy-issuer`'s surplus, tx
`d11e29aba04c6b67d9a4bf706effd21d1c3cbf28d3601ed9295f8c2b19fd5c09`), `mainnet-gilts-issuer`
`1.8330390` XLM — comfortably enough for CETES and XAUM to follow.

## 6. Self-service tester tooling

*(filled in once built)*

## 7. Oracle relay / alerting backend

*(filled in once built)*

## 8. Reproducible build evidence

**Source commit:** `4b5bc2b` (`mainnet` branch, matches the state fully covered by the
independent multi-agent security review and its H-4 follow-up round).

**Build command:** `cargo build --workspace --release --target wasm32v1-none`

**Toolchain:** `rustc 1.94.1 (e408947bf 2026-03-25)`, `cargo 1.94.1 (29ea6fb6a 2026-03-24)`,
`stellar 26.0.0 (60f7458e7ecffddf2f2d91dc6d0d2db4fab03ecc)`.

**WASM artifacts (all under the repository's 128 KiB size gate):**

| Contract | Size (bytes) | SHA-256 |
|---|---|---|
| `principal_manager.wasm` | 47,549 | `e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115` |
| `principal_market_config.wasm` | 22,639 | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` |
| `principal_market_factory.wasm` | 24,973 | `6c21fc0e78f3064f67ae30924b8f9ca0223fbb61a20ac640cceca7b152456759` |
| `principal_market_pool.wasm` | 88,042 | `52285a25bbf372d0e5d22ba565b76af6a7e832422df83b6a4809de8adf3aa112` |
| `principal_oracle_adapter.wasm` | 43,205 | `8581bbba82bb57f5c5c9c00adf98b1691ad20821840e55a67ad8d7ecace0c1b0` |
| `principal_permissioning.wasm` | 11,308 | `4437468dbbda6d86a115f188de877135b0b5a400b69ad7e9444cf269c54f969c` |
| `principal_pt_token.wasm` | 29,678 | `db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2` |
| `principal_recovery_escrow.wasm` | 41,773 | `dadea71fcfb84adcfb4691f472551f490cf440c5aaae5c6b5e5d4bb6f61441e8` |
| `principal_risk_control.wasm` | 24,453 | `ac75508d24c5457de7db1106bf8b5e9b62ea436f3dfd1a309fda5138110c2526` |
| `principal_router.wasm` | 74,651 | `2a0997317651edb0f7e2bd753570b03a88462aa6d216c14b06ebec246bb3c1c2` |
| `principal_sy_wrapper.wasm` | 41,540 | `be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a` |
| `principal_yt_token.wasm` | 44,088 | `c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654` |

These are the exact artifacts uploaded in §3/§4/§5 below — each deployed contract's on-chain WASM
hash (captured at upload time) is cross-checked against this table.

## 9. dApp handoff

*(filled in once contract addresses are finalized and handed off)*

## 10. Resume checklist (next steps now that three markets are live and tested)

**Shared infrastructure (§4), MOCKUSDY Markets 1 & 2, MOCKUSTRY's market, and a full real
deposit→split→AMM/trade→mature→settle→redeem cycle (§5) are all done.** Next steps, in priority
order:

1. **GILTS, CETES, XAUM remain** from the original plan. Confirmed cost model (see MOCKUSTRY's
   write-up in §5): no genuinely new WASM upload needed for any of them — `oracle_adapter` and
   all 7 per-market WASMs are already installed on mainnet; each asset only needs new, cheap
   *instances* plus its own classic asset/SAC. Real prices already captured live from the RedStone
   feed (§2 cross-reference): GILTS `141972700`, CETES `6469400` (feed's 8-decimal scale). XAUM's
   real token (`CC2RBGYNCFBCVENIDL5BFBWPH4OUZM2UA3OD2K2N54GLMWCC4KWPVAGO`, symbol `XAUM`, 9
   decimals) is a native Soroban token, not a classic-asset SAC like the others (confirmed: it has
   no `admin()`) — doesn't change Principal's own mock-asset approach (still issue
   `MOCKXAUM` as a classic asset + SAC the usual way), only means there's no real G-account issuer
   to mirror flags from; use the same `auth_revocable`/`auth_clawback_enabled` pair as the others
   for consistency. Real XAUM price: `386575800000` (8-decimal scale, ≈$3865.76).
2. **Operational lesson from MOCKUSTRY's near-miss** (its own write-up in §5 has the full story):
   when running `scripts/deploy/deploy.sh` for a new asset, give the per-market phase a long
   foreground timeout or run it with `run_in_background` — it was killed mid-flight this round,
   leaving `create_tokens`-only orphaned contracts that had to be recovered by hand via the
   factory's own `mkt_tok` event (salt + `predict_market`). This recovery path always works
   (salts are caller-chosen and deterministic), so a kill is recoverable, but it costs real time;
   better to avoid it. Never re-run `create_tokens` with a fresh random salt to "retry" after a
   partial failure — check the factory's events for an existing `mkt_tok`/`mkt_new` first.
3. Self-service tester tooling (`scripts/self_service/`) and the oracle relay
   (`services/oracle-relay/relay.sh`) are already built and proven on testnet, and the manual
   grant/mint/deposit/mint/settle/redeem steps they automate are now also proven by hand on
   mainnet (§5's testing tables) — both apply directly to this deployment now; neither needs to be
   rebuilt, only pointed at the mainnet contract IDs in §4/§5 and run on a real schedule (the
   oracle going stale repeatedly across Market 1, Market 2, and MOCKUSTRY's creation is a direct,
   repeated demonstration of why the relay needs to actually run, not just exist).
4. Reproducible-build sign-off and dApp handoff (§8/§9) can now reference three real, live,
   fully-exercised mainnet markets' addresses.
5. **Permission-gap finding to carry forward**: a human trading account needs its own
   `Permissioning.grant_asset` for every PT/YT token it will hold, separate from the market's own
   contract-level grants — found live during MOCKUSDY's testing round (§5), and hit again
   identically for MOCKUSTRY (test-a's `set_authorized` also needs its own classic-asset trustline
   established first, a related but distinct gap — found live there too). Any future testing
   checklist or self-service doc should call both out explicitly as required steps for a *new
   human tester* against a *new asset*, not just a new market on an already-tested asset.
6. `deploy.sh`'s known `upload_wasm`/`deploy_from_hash` `set -e`-blind-spot bug (a real failure
   inside a `$(...)` substitution doesn't stop the script cleanly, and a mere submission timeout
   isn't distinguished from a real failure) is still open — still worth fixing before relying on
   `deploy.sh` fully unattended for the next asset.
