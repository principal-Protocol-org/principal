# Mainnet Deployment Evidence — Tranche 3 Internal Testing

**Purpose:** this document is the single source of truth for every real Stellar mainnet address,
contract, and transaction created for Tranche 3's internal-testing deployment. Kept separate from
the security review documents (`INTERNAL_SECURITY_AUDIT_2026_11_16.md` and its coverage report) so
each can be updated independently — this one grows with every deployment action; those are closed
findings records.

**Status: PAUSED (2026-11-17).** Mainnet deployment is on hold after discovering that current
mainnet Soroban WASM-upload resource fees (~0.0012 XLM/byte, confirmed on two independent RPC
endpoints) make finishing even the first planned market cost roughly 550-600 XLM beyond what was
already spent — far more than the treasury was funded with. The user chose to pause mainnet
spending and continue the rest of the Tranche 3 plan (self-service tooling, oracle relay backend,
full end-to-end flow proof) on testnet, which is free, and revisit mainnet funding separately
later. **Nothing below this point is abandoned** — the MOCKUSDY issuer, SAC, and `permissioning`
contract deployed here remain live on mainnet and reusable when deployment resumes. See
"Resume checklist" at the end of this document for exactly what's needed to pick this back up.

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

*(filled in as deployment proceeds — OracleAdapter, Permissioning, RiskControl, Router)*

## 5. Markets

*(filled in as deployment proceeds)*

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

## 10. Resume checklist (when mainnet deployment picks back up)

1. **Fund the treasury** (`GBUVAHYJNCXZWUK3NIOVJ6EYQRSXMVT2XOUEASS3VAG4EWIXXWYSVSXW`) with
   enough XLM for the remaining work — budget at least **600 XLM** for the rest of the first
   market (4 remaining shared-infra contracts + 7 per-market contracts), plus headroom for a
   second market and any other mock assets. Re-check the live resource fee first
   (`simulateTransaction` against a built-only upload XDR, as done in §3 above) in case network
   fee conditions have changed since 2026-11-17.
2. **Re-run** `ENV_FILE=.env.mainnet-mockusdy ./scripts/deploy/deploy.sh` from the repository
   root. It is idempotent: `permissioning` (already live) will be detected and reused; the three
   contracts whose upload failed (`oracle_adapter`, `risk_control`, and whatever came after) will
   be retried fresh. The manifest at `deployments/mainnet-mockusdy/deployment-manifest.json`
   already has backups from both prior attempts.
3. Before resuming, fix the robustness gap noted above: `upload_wasm`'s failure inside a
   `hash="$(upload_wasm ...)"` assignment does not stop the script via `set -e`, producing
   confusing cascade errors instead of a clean abort. Worth patching `scripts/deploy/deploy.sh`
   to check `upload_wasm`'s/`deploy_from_hash`'s exit status explicitly before continuing.
4. Once the first market is live, create the second (30-minute) market for the same asset
   directly via `scripts/create_market_via_factory.sh` (not a second `deploy.sh` run, which would
   just reuse the existing market per its own idempotency check) — reusing the already-deployed
   `oracle_adapter`/`permissioning`/`risk_control`/`router`/`market_factory` IDs.
5. Everything else in the Tranche 3 plan (self-service tester tooling, oracle relay/alerting
   backend, reproducible-build sign-off, dApp handoff) is being proven out on testnet in the
   meantime per the user's explicit decision on 2026-11-17 — see the existing `TESTNET_*` evidence
   docs for that work, not this one.
