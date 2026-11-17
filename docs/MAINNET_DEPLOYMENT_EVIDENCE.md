# Mainnet Deployment Evidence — Tranche 3 Internal Testing

**Purpose:** this document is the single source of truth for every real Stellar mainnet address,
contract, and transaction created for Tranche 3's internal-testing deployment. Kept separate from
the security review documents (`INTERNAL_SECURITY_AUDIT_2026_11_16.md` and its coverage report) so
each can be updated independently — this one grows with every deployment action; those are closed
findings records.

**Status (2026-11-17, updated three times): shared infrastructure COMPLETE; market creation IN
PROGRESS, PAUSED AGAIN on funding.** Mainnet deployment first paused after discovering that
current mainnet Soroban WASM-upload resource fees (~0.0012-0.0015 XLM/byte, confirmed on two
independent RPC endpoints) make the full first deployment cost far more than the treasury was
initially funded with. After a treasury top-up (≈452 XLM across the three working keys), shared
infrastructure (§4) was completed in full and verified. After a second top-up and further
spend-down (consolidating idle funds between keys, continuing on existing funds per explicit
user instruction rather than waiting for more), **5 of the first market's 7 per-market contracts
are now uploaded** (§5) — paused again before `recovery_escrow` when the issuer key's remaining
balance (~41 XLM) could no longer safely cover even the cheaper of the two remaining uploads
(quoted 55.81 XLM). **≈52.71 XLM remains** across the three keys; **≈143-165 XLM more** is
needed for the two remaining uploads alone (`market_pool`, `recovery_escrow`), before even
counting the `MarketFactory`'s three `create_*` calls (not yet cost-measured). See §10 (Resume
checklist) for exactly what's needed to finish.

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

### Market 1 — 90-day reviewer market (in progress, paused again on funding)

With shared infrastructure complete (§4), began uploading the 7 per-market WASM contracts —
`scripts/deploy/deploy.sh`'s own per-market loop does these one at a time with `ISSUER_KEY`
before ever calling `create_market_via_factory.sh`, so this is a safe place to pause: every
upload below is independently sunk, idempotent, and fully reusable whenever the rest of the
market is created — nothing is left half-built.

**Uploaded so far** (5 of 7), each balance-checked against the real fee before proceeding to the
next, confirming the same ~13% real-cost-below-simulated-quote pattern first seen in §4 held
consistently across all five (now 7 of 7 contracts total, counting §4's two, at a remarkably
tight 13.0-13.1% discount every single time):

| Contract | WASM hash | Upload tx | Quoted (XLM) | Real cost (XLM) |
|---|---|---|---|---|
| `sy_wrapper` | `be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a` | `abe8ed52023709c790716f02688a8e3363e3a5b895e53b8faa7fd1cb43e6d174` | 49.9493112 | 43.4346278 |
| `pt_token` | `db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2` | `5e90ef46a93fc20f9150c9d3c95e1129397738d07fe006ffa086f4a130c7233a` | 42.1388872 | 36.6427374 |
| `yt_token` | `c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654` | `233514e0ebab444e40ce11e3845ffa4b5285cac3ec94c5fde1bee581aebb5268` | 60.7313451 | 52.8103563 |
| `market_config` | `11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8` | `22432394de18898744edc16571eedea3ad42612042b4e8786c620bea50f3e5df` | 34.1468408 | 29.6930028 |
| `principal_manager` | `e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115` | `3dcaca908ae2dceb2200679c05d2a0c0b1a3e0ad4f8fc33465d5a80eb5dd52a2` | 64.3810413 | 55.9840685 |

All five signed by `ISSUER_KEY` (`mainnet-mockusdy-issuer`), matching `deploy.sh`'s own
convention and the contract requirement that `ISSUER_KEY` be the real issuer authority relayed
into these contracts' `initialize` calls later. Before this round, the admin key's own surplus
(beyond what §4's wiring step needs — confirmed ~1 XLM) was consolidated into the issuer key to
maximize what was available to spend: `mainnet-admin` → `mainnet-mockusdy-issuer`, 55 XLM, tx
confirmed successful; a second, smaller top-up from `principal-mainnet-treasury` failed
(`Underfunded` — asked for slightly more than its own minimum-reserve headroom allowed) and
genuinely did not go through (only the tiny base fee was charged).

**Stopped before `recovery_escrow`**: re-quoted live at exactly 55.8143835 XLM (unchanged from
§5's first pass) — even at the observed ~13% discount (~48.6 XLM real), this exceeds the
41.3528740 XLM remaining on the issuer key at that point, and `market_pool` (quoted 108.5089046,
~94.4 at discount) is far further out of reach. Rather than risk a failed
`TxInsufficientBalance` transaction, stopped here cleanly.

**Still needed for this market**: uploading `market_pool` and `recovery_escrow` (quoted
108.51 + 55.81 = 164.32 XLM, likely ≈143 XLM at the ~13% discount — not yet confirmed for these
two specifically), plus the `MarketFactory`'s three `create_*` calls (cost not yet measured —
each deploys 2-3 new contract instances in one transaction, likely costlier than the small plain
invokes already measured in §4, so budget with real caution rather than assuming it is
negligible), plus wiring/compliance grants (confirmed cheap, ~1 XLM total for ~25 calls).

Balances after this round: `mainnet-admin` 5.0044944, `mainnet-mockusdy-issuer` 41.3528740,
`principal-mainnet-treasury` 6.3511531 XLM — **≈ 52.71 XLM remaining across all three keys**,
short of the ~143-165 XLM still needed for just the two remaining uploads (before even counting
the factory calls). See §10 for the updated resume checklist.

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

## 10. Resume checklist (when mainnet market creation picks back up)

**Shared infrastructure is done** (§4) — nothing further needed there; only the first market
itself remains.

1. **Fund `mainnet-mockusdy-issuer`** (`GB3SOEMQVYBLMHPV3AK64PV4FJTOMQ2XUJLOZSVDJLP2LT42EU3XLE4P`
   — signs every per-market WASM upload *and* the factory's three `create_*` calls) with enough
   XLM for the two still-missing uploads (`market_pool`, `recovery_escrow`: quoted
   108.51 + 55.81 = 164.32 XLM, likely ≈143 XLM at the ~13% real-cost discount observed on every
   single upload so far, seven for seven, in §4 and §5 — not yet confirmed for these two
   specifically) **plus** the three `create_*` calls' own cost, which has **not yet been
   measured** — each deploys 2-3 new contract instances in a single transaction, so budget with
   real caution rather than assuming it is as cheap as a plain invoke. **`mainnet-admin`** (for
   the wiring + compliance grants) only needs ~1 XLM total — it was drawn down to ~5 XLM this
   round precisely because its surplus was moved to the issuer key instead; top it back up only
   if that 5 XLM buffer turns out not to be enough. Balances as of this update: `mainnet-admin`
   5.0044944, `mainnet-mockusdy-issuer` 41.3528740, `principal-mainnet-treasury` 6.3511531 XLM.
   Re-check the live resource fee first (`simulateTransaction` against a built-only upload XDR,
   as done throughout §4/§5) in case network fee conditions have changed.
2. **Re-running `ENV_FILE=.env.mainnet-mockusdy ./scripts/deploy/deploy.sh` will redundantly
   re-upload all five already-uploaded per-market contracts** (`sy_wrapper`, `pt_token`,
   `yt_token`, `market_config`, `principal_manager` — see §5's table) **before** reaching
   `market_pool`/`recovery_escrow` — `deploy.sh`'s per-market loop has no manifest check for
   already-uploaded-but-not-yet-deployed WASM, unlike the shared-infra `deploy_or_reuse`
   function. This is believed harmless, not wasteful: re-uploading byte-identical code that
   already exists on-chain was confirmed idempotent and nearly free when checked against
   `permissioning` in §4 (`0.0036 XLM` to "re-upload" code already present) — but this has **not
   been independently re-confirmed for these specific five contracts**, so budget a small buffer
   for that assumption being wrong before relying on it. The cleaner alternative is uploading
   `market_pool`/`recovery_escrow` by hand first (`stellar contract upload --wasm ... --source
   mainnet-mockusdy-issuer --network mainnet`, same pattern as §5's table), then running
   `scripts/create_market_via_factory.sh` directly with all seven resulting hashes (see that
   script's own usage comment) instead of `deploy.sh`'s market section at all.
3. **A `transaction submission timeout` on any single upload is not necessarily a real
   failure** — confirmed multiple times this round: resubmitting the identical already-signed
   transaction (same source, same sequence number) either confirms the original succeeded, or
   genuinely resubmits if it didn't (check the tx hash on Horizon first either way: a 404 means
   it never landed and is safe to retry; `successful: true` means it already did and must not be
   redone). `deploy.sh` itself does not yet handle this automatically -- the known
   `upload_wasm`/`deploy_from_hash` `set -e`-blind-spot bug (doesn't stop the script cleanly on a
   real failure, and also doesn't retry a mere timeout) is still open; worth fixing properly
   before relying on this script unattended.
4. Once the first market is live, create the second (30-minute) market for the same asset
   directly via `scripts/create_market_via_factory.sh` (not a second `deploy.sh` run, which would
   just reuse the existing market per its own idempotency check) — reusing the already-deployed
   shared infra IDs and the same 7 per-market WASM hashes (no re-upload needed — confirmed cheap
   and correct on testnet §5 "Market 2").
5. Everything else in the Tranche 3 plan (self-service tester tooling, oracle relay/alerting
   backend, reproducible-build sign-off, dApp handoff) is being proven out on testnet in the
   meantime per the user's explicit decision on 2026-11-17 — see the existing `TESTNET_*` evidence
   docs for that work, not this one.
