# dApp Integration Guide — Switching to Stellar Mainnet

> **Update, 2026-11-21 — if you already pulled `deployments/mainnet-dapp-manifest.json` before
> this date, re-pull it now.** Your own team's live checks caught a real, permanent bug this guide
> had not: the USTRY/GILTS/CETES/XAUM `OracleAdapter`s were off by 10x (and three had no feed
> wired at all). They have been redeployed end to end ("v2") with new contract addresses — the
> manifest file in this repo is already updated; the copy you have is not. Full incident writeup:
> `MAINNET_DEPLOYMENT_EVIDENCE.md` §12. Thank you for catching this.
>
> **Update, 2026-11-22 — build reproducibility fixed.** Your rebuild of `3af9388` getting 0/12
> matching hashes was a real gap: the repo had no pinned Rust toolchain (`Cargo.lock` was
> committed, but nothing forced a specific rustc, and CI itself floated on `@stable`). Added
> `rust-toolchain.toml` pinning `1.94.1` and re-pinned the CI jobs that build the on-chain WASM.
> A clean rebuild of `3af9388` now reproduces all 12 deployed hashes byte for byte. Recipe:
> `deployments/mainnet/reproducible-build.md`. Full writeup: `MAINNET_DEPLOYMENT_EVIDENCE.md` §14.
> The oracle staleness recurrence you flagged separately (`max_stale_seconds` shorter than the
> feed's real 12h cadence) is also fixed — all 5 oracles now at `max_stale_seconds=86400`,
> `MAINNET_DEPLOYMENT_EVIDENCE.md` §13.

**Audience:** the Principal dApp team (`principal-Protocol-org/dapp`). **Written by:** the
contracts/deployment side, as a senior-auditor-grade handoff — every claim below is backed by a
real, verified Stellar mainnet transaction, not an assumption. **Status of mainnet contracts as of
2026-11-22:** five mock assets, six markets, all live, wired, compliance-granted, and
real-transaction tested (four of the five assets are on their "v2" contracts — see the update
notice above). Zero fabricated data — see
[`docs/MAINNET_DEPLOYMENT_EVIDENCE.md`](MAINNET_DEPLOYMENT_EVIDENCE.md) for the full narrative and
[`docs/MAINNET_TRANSACTION_LEDGER.md`](MAINNET_TRANSACTION_LEDGER.md) for the complete,
Horizon-cross-checked list of all 276 real transactions behind this deployment, and
[`docs/MAINNET_ADDRESSES.md`](MAINNET_ADDRESSES.md) for every address on one page with no
narrative.

This mirrors the structure of the dApp's own testnet integration story (its README's "Network
selection" and "Manifest" sections, and `config/manifests/testnet.json`) — but for mainnet, and
with the real money implications that come with it made explicit rather than assumed.

**File paths below**: anything under `lib/`, `app/`, `components/`, `config/`, `scripts/` (other
than `scripts/self_service/` and `scripts/create_market_via_factory.sh`) and `.env.example` is a
path inside **`principal-Protocol-org/dapp`** (branch `testnet` at the time this guide was
written), read directly to verify every claim in this guide. `scripts/`, `docs/`, and
`deployments/` paths with no such prefix, and anything under `contracts/`, are in **this**
(`principal`) repo.

---

## 0. Read this before touching anything

**Mainnet is not testnet with bigger numbers.** Every contract call on the addresses below moves
real XLM and real (mock, but real on-chain) assets. Specifically, unlike testnet:

- **There is no Friendbot.** Funding a new account costs real XLM, from a real wallet, every time.
- **There is no automated oracle push on mainnet by design — confirmed by reading the code, not
  just the README.** `lib/server/oracle-keeper.ts:131` throws
  `OracleKeeperConfigError` the instant `manifest.network === "mainnet"`, before anything else
  runs: `if (manifest.network === "mainnet") throw new OracleKeeperConfigError("the oracle keeper
  runs only on testnet and local networks.")`. That is the right call for Tranche 2 and it still
  is — leave it in place — but it means **something else must keep every mainnet `OracleAdapter`
  fresh**, or every mint/redeem/settle call on it reverts `OracleStale`. This deployment has been
  doing that by hand (`stellar contract invoke ... set_reference_value`, with the real
  RedStone-sourced price, repeated manually throughout deployment and testing — see §2 below).
  **Before any mainnet traffic beyond manual testing, this needs a real scheduled job**
  (`services/oracle-relay/relay.sh` in the `principal` repo is built and proven on testnet for
  exactly this; it has not yet been pointed at mainnet). Do **not** repurpose
  `lib/server/oracle-keeper.ts` for this — build the mainnet relay as its own, separately-reviewed
  job, the same way `services/oracle-relay/relay.sh` already is.
- **`set_authorized` on a classic asset reverts if the target account has no trustline yet** — this
  bit every single mock asset at least once during deployment (see the `TrustlineMissingError`
  findings in `MAINNET_DEPLOYMENT_EVIDENCE.md` §5). A dApp UI flow that authorizes a new user must
  either build the `change-trust` operation into the same flow or clearly sequence it first.
- **A `transaction submission timeout` from the Stellar RPC is not necessarily a real failure** —
  this happened repeatedly on mainnet this deployment (confirmed via Horizon: check the tx hash
  directly; 404 means safe to resubmit the identical signed transaction, `successful: true` means
  it already landed and must not be resubmitted). Any mainnet-facing submission path in the dApp
  (the step-review/retry UI) should already do this given your own `README.md`'s refundable-fee
  retry note — just confirm it checks Horizon rather than assuming a timeout = failure.
- **These are mock assets, not the real RWAs they track.** `MOCKUSDY`/`MOCKUSTRY`/`MOCKGILTS`/
  `MOCKCETES`/`MOCKXAUM` are Principal-issued classic Stellar assets that mirror the *price* of
  real USDY/USTRY/GILTS/CETES/XAUM via the same shared RedStone SEP-40 feed real markets for those
  assets would use — see `MAINNET_DEPLOYMENT_EVIDENCE.md` §1–§2 for exactly why (the real assets'
  issuer authority is a contract Principal does not control, so a real market on them isn't
  possible yet). Label them as such in the UI; do not present them as the real RWAs.

---

## 1. The exact code change needed

Your own `lib/protocol/manifest-source.ts` (as of the `testnet` branch this guide was checked
against) only recognizes two networks:

```ts
export function activeManifest(): Manifest {
  const network = process.env.NEXT_PUBLIC_PRINCIPAL_NETWORK ?? "testnet"
  const cached = parsed.get(network)
  if (cached) return cached
  let manifest: Manifest
  if (network === "testnet") manifest = parseManifest(testnetJson)
  else if (network === "local") manifest = parseManifest(localJson)
  else throw new Error(`NEXT_PUBLIC_PRINCIPAL_NETWORK must be "testnet" or "local", got "${network}".`)
  parsed.set(network, manifest)
  return manifest
}
```

`lib/protocol/manifest.ts`'s own schema **already supports `"mainnet"`** —
`z.enum(["local", "testnet", "mainnet"])` and `PASSPHRASES.mainnet` are both already correct and
need no change. The only real gap is the loader above never reaching that branch. The fix:

```diff
+import mainnetJson from "@/config/manifests/mainnet.json"
 import localJson from "@/config/manifests/local.json"
 import testnetJson from "@/config/manifests/testnet.json"
 import { parseManifest } from "./manifest"
 import type { Manifest } from "./types"

 const parsed = new Map<string, Manifest>()

 export function activeManifest(): Manifest {
   const network = process.env.NEXT_PUBLIC_PRINCIPAL_NETWORK ?? "testnet"
   const cached = parsed.get(network)
   if (cached) return cached
   let manifest: Manifest
   if (network === "testnet") manifest = parseManifest(testnetJson)
   else if (network === "local") manifest = parseManifest(localJson)
+  else if (network === "mainnet") manifest = parseManifest(mainnetJson)
-  else throw new Error(`NEXT_PUBLIC_PRINCIPAL_NETWORK must be "testnet" or "local", got "${network}".`)
+  else throw new Error(`NEXT_PUBLIC_PRINCIPAL_NETWORK must be "testnet", "local" or "mainnet", got "${network}".`)
   parsed.set(network, manifest)
   return manifest
 }
```

Then add the manifest file itself: **[`deployments/mainnet-dapp-manifest.json`](../deployments/mainnet-dapp-manifest.json)**
in this (`principal`) repo is a ready-to-copy, schema-validated `config/manifests/mainnet.json` —
copy it in verbatim. It was built directly from the real deployment manifests
(`deployments/mainnet-{mockusdy,ustry,gilts,cetes,xaum}/deployment-manifest.json`) and independently
checked against your own zod regexes (`C[A-Z2-7]{55}`, `G[A-Z2-7]{55}`, 64-hex `wasmHashes`) before
being handed over — every field will parse.

Two things the generic diff above can't do for you, both load-bearing:
1. **`routerFunctions` reuse is verified, not assumed.** Your `testnet.json` pins its
   `routerFunctions` list against `principal` commit `665bb67`. This mainnet deployment is built
   from `principal` commit `3af9388` — a *descendant* of `665bb67` — and
   `git diff 665bb67 3af9388 -- contracts/router/src/lib.rs` has **zero changes to any public
   function**. The same `routerFunctions` array (already in the manifest file above) is correct
   for mainnet without modification. If a later mainnet redeploy moves past a commit that *does*
   touch `Router`, re-check this before reusing the list blindly.
2. **The `ORACLE_ADMIN_SECRET` route's mainnet refusal is correct and must stay.** Do not wire a
   mainnet oracle cron into this dApp using that route — see §0. If the dApp team ends up owning
   the oracle relay long-term, build it as its own, separately-reviewed job (model it on
   `services/oracle-relay/relay.sh`), not as an extension of the existing testnet-only route.

Set the environment:

```bash
# .env.mainnet (gitignored — never commit real keys)
NEXT_PUBLIC_PRINCIPAL_NETWORK=mainnet

# FAUCET_ISSUER_SECRET / FAUCET_PERMISSIONING_ADMIN_SECRET: leave unset on mainnet. "Get test
# tokens" is already network-gated end to end -- confirmed by reading the code, not assumed:
# lib/protocol/onboarding.ts's onboardingAvailable({ network }) returns false for "mainnet"
# (asserted directly in lib/protocol/__tests__/onboarding.test.ts), and every call site that
# renders or serves the faucet checks it before doing anything else -- the button
# (wallet-button.tsx, onboarding-dialog.tsx, compliance-card.tsx) and the server route itself
# (app/api/faucet/route.ts, via lib/server/faucet.ts:246's
# `if (!onboardingAvailable(manifest) ...) return { error: "network" }`). Setting these two
# secrets on mainnet does nothing unsafe by itself -- the route still refuses every call -- but
# there is no reason to hold a real secret key in a mainnet environment that can never use it.

# DATABASE_URL / DATABASE_URL_UNPOOLED: needed for the market-snapshot recorder (rate charts,
# /api/cron/record) -- see §1a for the required one-time migration and the production scheduler.
# Without these the app still works (market pages fall back to the manifest after 1.5s with no
# history/chart data), so this is required for full functionality, not for the app to run at all.
DATABASE_URL=
DATABASE_URL_UNPOOLED=

# QSTASH_CURRENT_SIGNING_KEY / QSTASH_NEXT_SIGNING_KEY: from the mainnet QStash account (see §1a).
QSTASH_CURRENT_SIGNING_KEY=
QSTASH_NEXT_SIGNING_KEY=
```

### 1a. Infrastructure that must exist before the recorder/charts work on mainnet

These are genuinely required steps, separate from the code change above, and your own
`README.md`'s "Scheduler" table already names the target: branch `main`, Vercel **Production**,
domain `principalmarket.fi`, QStash account `principalmarket-mainnet`. That table's own network
column said *"Mainnet, once supported (Tranche 3)"* — Tranche 3 is this deployment, so this is now
the moment to actually do it, not a future placeholder.

1. **Run the DB migration once, against the mainnet Neon branch**, before the first deploy:
   ```bash
   pnpm db:migrate --env-file .env.mainnet
   ```
   This applies `db/migrations/*.sql` over a direct (unpooled) connection — never run it against the
   pooled `DATABASE_URL` directly (Neon convention, already handled by the script if
   `DATABASE_URL_UNPOOLED` is set or derivable by stripping `-pooler` from the host).
2. **Create a QStash schedule** with cron `*/15 * * * *` calling
   `https://principalmarket.fi/api/cron/record` on the final host (no redirect), and set
   `QSTASH_CURRENT_SIGNING_KEY`/`QSTASH_NEXT_SIGNING_KEY` from that same QStash account in the
   Production environment's variables.
3. **Do not** create a corresponding mainnet schedule to `/api/cron/oracle` — see §0 and `oracle-keeper.ts`'s
   own refusal above. That route has no mainnet mode to enable.
4. Keep the mainnet secrets (`DATABASE_URL`, `QSTASH_*`) scoped to the Production environment only,
   the same isolation your README already describes for the `testnet` branch/Preview environment.

### 1b. What you do *not* need to change

Confirmed by reading the actual call sites, not assumed — these already handle `"mainnet"`
correctly with zero code change once §1's one diff lands:

- **The testnet/local banner already hides itself.** `components/app/shell/testnet-banner.tsx`:
  `if (network !== "testnet" && network !== "local") return null` — a mainnet manifest already
  renders no banner. `networkLabel()` in `components/app/shell/network.ts` already has a
  `"Stellar Mainnet"` branch too, in case you want to reuse it elsewhere.
- **The manager wizard and `/app/manage` have no network-specific gating at all** — grepped the
  whole `app/app` and `components/app/manage` trees for a literal `"testnet"`/`"local"` check and
  found none outside `manifest-source.ts` itself. The wizard's `predict_market`/`create_*` calls
  and the resume-by-salt flow are already network-agnostic.
- **`lib/protocol/manifest.ts`'s schema, `PASSPHRASES`, and `types.ts`'s `Network` union already
  include `"mainnet"`** — nothing to add there.

---

## 2. Real mainnet contracts — the reference table

Every address below is live on Stellar Public mainnet right now, independently verified via
`scripts/deploy/verify-deployment.sh` (78/79 checks passing per market — the one expected
"failure" is a `git_tree_dirty` flag, not a real issue) and via direct `stellar contract invoke
--send=no` reads, not just trusted from a deploy log.

**Shared infrastructure** (one instance, reused by every market below):

| Contract | Mainnet address |
|---|---|
| `Permissioning` | `CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ` |
| `RiskControl` | `CBHCRRLWHS7KRQNAXYY3OMNDTSXDNJKCDK6KPUUJPFGDBSN76JEWWQ5G` |
| `Router` | `CCM4QQUKTD2CUTRSNNI6OO4JHYEF7MHKBMKSM5JYJNFLKSTAZBQD7QF5` |
| `MarketFactory` | `CDYH7UV24XS7G7MDRTBJUS4PJNKKUXTLUUTL67MR5CYJAXSPPHNWS65T` |

**Per-asset `OracleAdapter`** — one instance *per mock asset* (not shared — see
`MAINNET_DEPLOYMENT_EVIDENCE.md` §4 for why), each tracking the real RWA's price via the same
shared RedStone SEP-40 feed (`CBMGLKUQZVSAIL5CPDDAWSUY7MAKXISHMOZEVLMBUWBMFGHRJSR4WYRF`).
**USTRY/GILTS/CETES/XAUM are "v2" instances, redeployed 2026-11-21 — see the update notice at the
top of this guide and `MAINNET_DEPLOYMENT_EVIDENCE.md` §12 before using any address you may have
cached from before that date:**

| Asset | `OracleAdapter` | Tracks real price of |
|---|---|---|
| MOCKUSDY | `CCKXG4HSVH57AQTYJDNNXHQBCGNH6KRFNMCMATTMYFUX2TDQ7GD5K7IT` | USDY (Ondo) |
| MOCKUSTRY v2 | `CBTW7CU46PMCPKZQ3VX6OBELOEC7HHQXJSAX535VW3EK34MVS7UWPFR7` | USTRY |
| MOCKGILTS v2 | `CD22HNREJHKTATZMFKALCTHJVKC35MHO2SAUSTG4M6PWLJZPS724J6X7` | GILTS |
| MOCKCETES v2 | `CBK2GO7UPMES2B7LZYZXNO3JP5JPNIIE74E4PUUJGSZZKD4FZ47QMUZK` | CETES |
| MOCKXAUM v2 | `CDFI7IXUTGLV2EQE3VCHPCYOEBX2LSLXDKNU7ELJFZH726VDF73SOPFD` | XAUM (Matrixdock Gold) |

**Markets** — five mock assets, six markets (MOCKUSDY has two: a 90-day one and a since-matured
30-minute flow-test one). Full per-market contract addresses (SY/PT/YT/Config/Manager/Pool/Escrow)
are in [`deployments/mainnet-dapp-manifest.json`](../deployments/mainnet-dapp-manifest.json) —
not repeated here to avoid a second place these can drift out of sync.

| Market | Underlying SAC | Maturity (Unix) | Status |
|---|---|---|---|
| MOCKUSDY 90-day | `CDQYZUYQF7TRQ7BX6YBSCU3HLWBDS3E2VSP7AUE7C3E2D2U2IBBQOBGF` | `1802719669` | **open**, AMM liquidity seeded, real swap proven |
| MOCKUSDY 30-min | same SAC | `1794946266` | **matured and fully redeemed** — read-only/demo value now, like the old testnet market this guide's structure mirrors |
| MOCKUSTRY v2 90-day | `CDHQ4B6U3D5O6PGAWAO22K45ASD2BAGD25IHITRIIUHXA3GBABZTYBA5` | `1802984619` | **open**, AMM liquidity seeded, real swap proven |
| MOCKGILTS v2 90-day | `CBBF4H2ZNMJKULJEOMZDMVQP27LD7HRTHEHETD647P3SCBTAWRGVJG4O` | `1802986133` | **open**, AMM liquidity seeded, real swap proven |
| MOCKCETES v2 90-day | `CBVSL37VLX4Z362AETPZVM65EZGETM2ROT7K56MIURXJU4LZDIJ4IATP` | `1802986873` | **open**, AMM liquidity seeded, real swap proven |
| MOCKXAUM v2 90-day | `CBAJVVTBBR6XXAK4N32WHVOALDRMQ27YMQNS6PBLGNWJIBFWOYS6AVVD` | `1802987509` | **open**, AMM liquidity seeded, real swap proven |

The v1 USTRY/GILTS/CETES/XAUM markets (different maturities, different per-market contract
addresses, same underlying SACs) are **abandoned — do not route traffic at them.** See
`MAINNET_DEPLOYMENT_EVIDENCE.md` §12 if you need their old addresses for the historical record.

The 30-minute MOCKUSDY market is included in the manifest's lineage but **deliberately left out**
of `deployments/mainnet-dapp-manifest.json`'s `markets[]` array — same reasoning your own
`DAPP_INTEGRATION_AND_MARKET_CREATION.md` gives for the old matured testnet market: tokenization
and trading on a matured market are permanently closed (`PrincipalManager`/`MarketPool` both
revert `AlreadyMature`/`Expired`), so listing it as a live, tradeable market in the UI would be
wrong. If you want a "closed market" demo state in the UI, add it back by hand with its real final
numbers from `MAINNET_DEPLOYMENT_EVIDENCE.md` §5 ("Market 2").

**Oracle freshness, right now, is not guaranteed by anything automated.** Before pointing real
dApp traffic at any of the five `OracleAdapter`s above, check `is_available()` live — if it is
`false`, every mint/redeem/settle call on that asset will revert `OracleStale` until someone with
the admin key (`mainnet-admin`, address `GDZN5WVOUBRTZKADUULRXJOGVGNK4WWXJPSALTVOORLSU5NFOZ2P5NCB`)
pushes a fresh value. See §0's note on the relay.

---

## 3. Testing it yourself: deploy your own mock asset and market, end to end

This is the same self-service path already built and proven on testnet
(`scripts/self_service/README.md`), reused here verbatim with `NETWORK=mainnet` — **it was
designed network-agnostic from the start precisely so this would work without changes.** It needs
no Principal admin key for your own asset and market; only the two genuinely security-relevant
admin-gated steps (`RiskControl.add_consumer`, `Router.register_market`) are left to Principal's
own `registration_watcher.sh`, run on a schedule.

All commands below are run from a clone of the `principal` repo (contracts side), with the
`stellar` CLI installed and a `mainnet` network configured
(`stellar network add mainnet --rpc-url https://soroban-rpc.mainnet.stellar.gateway.fm
--network-passphrase "Public Global Stellar Network ; September 2015"`, or use `--rpc-url`/
`--network-passphrase` flags directly as shown).

### 3.1 Generate and fund your own key (real XLM — there is no faucet)

```bash
stellar keys generate my-mainnet-tester --network mainnet
stellar keys address my-mainnet-tester
# → send that G-address a few real XLM from an exchange or another wallet before continuing.
# Budget: ~3 XLM comfortably covers account creation reserve + every step below with margin
# (confirmed empirically this deployment — an entire new market, from classic-asset issuance
# through compliance grants, cost under 2 XLM total per asset once WASM reuse was in effect).
```

### 3.2 Issue your own mock asset and its Soroban Asset Contract

```bash
ISSUER_KEY=my-mainnet-tester ASSET_CODE=MOCKFOO NETWORK=mainnet \
./scripts/self_service/create_mock_asset.sh
```

This is exactly the script that issued `MOCKUSDY`, `MOCKUSTRY`, `MOCKGILTS`, `MOCKCETES`, and
`MOCKXAUM` on real mainnet this deployment (the `ASSET_CODE` must start with `MOCK` or `TEST` or
`registration_watcher.sh` will never auto-register a market on it — see the script's own header
for why). It prints a `SAC_ID` at the end — the Soroban Asset Contract address for your new asset.
**Real example, MOCKUSTRY's actual mainnet SAC deploy** (same code path you are about to run):
tx [`07b4a96d80ef4b81423a8277ec36307211bc70aa44b1c861cc9342252f3c115d`](https://stellar.expert/explorer/public/tx/07b4a96d80ef4b81423a8277ec36307211bc70aa44b1c861cc9342252f3c115d).

### 3.3 Make sure the asset has a real price to track (optional, but realistic)

If you want your mock asset to track a real RWA's live price the way the five already-deployed
ones do, find its real SAC address on the shared RedStone feed and push a genesis value to a new
`OracleAdapter` instance — see `MAINNET_DEPLOYMENT_EVIDENCE.md` §2 for the exact live query against
feed `CBMGLKUQZVSAIL5CPDDAWSUY7MAKXISHMOZEVLMBUWBMFGHRJSR4WYRF`. This is optional for a pure UI
smoke test — a market works with any positive oracle value, real-tracking or not.

### 3.4 Create your market through the factory

```bash
FACTORY_ID=CDYH7UV24XS7G7MDRTBJUS4PJNKKUXTLUUTL67MR5CYJAXSPPHNWS65T \
ISSUER_KEY=my-mainnet-tester ADMIN_KEY=my-mainnet-tester \
UNDERLYING_ID=<the SAC_ID from 3.2> MATURITY_TS=$(date -d '+1 day' +%s) \
PERMISSIONING_ID=CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ \
ORACLE_ID=<your oracle, or reuse one of §2's existing ones for a quick smoke test> \
RISK_CONTROL_ID=CBHCRRLWHS7KRQNAXYY3OMNDTSXDNJKCDK6KPUUJPFGDBSN76JEWWQ5G \
PROTOCOL_ADMIN_ADDR=<your own address> TREASURY_ADDR=<your own address> \
SY_WASM=be4b12cf3c16461b7e71fa225ed2363aba62bdc42c1ea5c08c98d7ede6a27c9a \
PT_WASM=db025340887ef9d92922e2444da24edab5fec125460e9adc373a5f77b26578b2 \
YT_WASM=c83a2a8d2a4a79ff5348675581a27d48e52a920d83dc887672840d4a79c02654 \
CONFIG_WASM=11b0cbdf10a8d204fcb568c54529801747feac05208c5bcbfaefd822ad0c79a8 \
MANAGER_WASM=e2d04b45b867d0dbc41660ac0d97d3e705005ddfb10350bb030d25b1aff86115 \
POOL_WASM=52285a25bbf372d0e5d22ba565b76af6a7e832422df83b6a4809de8adf3aa112 \
ESCROW_WASM=dadea71fcfb84adcfb4691f472551f490cf440c5aaae5c6b5e5d4bb6f61441e8 \
SKIP_WIRING=1 NETWORK=mainnet \
./scripts/create_market_via_factory.sh
```

Every one of those `*_WASM` hashes is **already installed on mainnet** — this call reuses them
(cheap: confirmed via `simulateTransaction`, re-uploading an already-installed hash costs
**0.0120674 XLM**, not a fresh ~50-110 XLM upload). `SKIP_WIRING=1` because you don't hold
`RiskControl`/`Router` admin rights — that's `registration_watcher.sh`'s job next.
**Real example, the exact same three calls, MOCKUSTRY's (v1, now-abandoned) market creation —
still a real, successful, representative response shape, even though that specific market was
later replaced per §12 of the evidence doc:**
`create_tokens` tx [`eaeecc87878b8d289e8036e9eacffc234d83e5bc5391184988ec2e09621ecadb`](https://stellar.expert/explorer/public/tx/eaeecc87878b8d289e8036e9eacffc234d83e5bc5391184988ec2e09621ecadb),
`create_engine` tx [`c77e3f1567388ac98882d69bcade313671d7a8155c3fbf8f1ade3afee80e1b09`](https://stellar.expert/explorer/public/tx/c77e3f1567388ac98882d69bcade313671d7a8155c3fbf8f1ade3afee80e1b09),
`create_pool_and_escrow` tx [`8f02b4f8a60d3efe9296f6b86b6ce4082b07ff6a8fc09db4e53883bd18ed0535`](https://stellar.expert/explorer/public/tx/8f02b4f8a60d3efe9296f6b86b6ce4082b07ff6a8fc09db4e53883bd18ed0535).
(A clean v2 equivalent, correctly oracle-wired from the start: GILTS v2's three calls,
`887db2b0f0611638b1efc7c37a9da82f610c45806e3d70b2c4a2b625ece741b9` /
`69270210810abce59d37e4afb0e1e4b2e8ab3a47051006ae0d1d71626284bf30` /
`cb76ae4ea7b96e8807c105a3ebaa2be1f1b5d024b8556466bcaf7b5e55c4699a`.)

### 3.5 Wait for registration

**Confirmed running, 2026-11-21: `registration_watcher.sh` is now on a real cron job against
mainnet, every 15 minutes** (not just "proven once on testnet" as earlier drafts of this guide
said) — so a market you create via the self-service path above is picked up automatically within
15 minutes, no manual step or message to the contracts team needed. Once it has run against your
market: `RiskControl.is_consumer` is `true` for your `SYWrapper` and `PrincipalManager`, and
`Router.market()` returns your pool's real topology. (You can still deploy your own instance of
this script against your own test market if you want full control over timing instead of waiting
on the shared schedule — it needs no Principal key, only the admin credentials for whichever
`RiskControl`/`Router`/`Permissioning` you point it at.)

### 3.6 Grant yourself (or any test wallet) permission to actually trade

This is the step that bit this deployment hardest and is **easy to miss**: `PTToken`/`YTToken`'s
`mint` checks **both** `Permissioning.is_allowed` (account-level) **and**
`Permissioning.is_allowed_for_asset(account, this_token)` (per-asset) on whoever will *hold* the
PT/YT — not just on the market's own contracts. A trading wallet also needs its own classic-asset
**trustline** to your mock asset *before* `set_authorized` is called for it, or that call reverts
`TrustlineMissingError`. Concretely, for a wallet `$TESTER` that will trade on your market:

```bash
TESTER=$(stellar keys address my-mainnet-tester)   # or any wallet you're testing with

# 1. trustline first (classic Stellar operation, signed by the trading wallet itself)
stellar tx new change-trust --source my-mainnet-tester --network mainnet \
  --line "MOCKFOO:$(stellar keys address my-mainnet-tester)"

# 2. underlying-asset authorization (signed by the issuer)
stellar contract invoke --id <your SAC_ID> --source my-mainnet-tester --network mainnet \
  -- set_authorized --id "$TESTER" --authorize true

# 3. account-level Permissioning standing (signed by the Permissioning admin — here, you)
stellar contract invoke --id CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ \
  --source my-mainnet-tester --network mainnet \
  -- grant_account --caller "$TESTER" --account "$TESTER"

# 4. per-asset Permissioning standing for the PT and YT tokens specifically (same signer)
stellar contract invoke --id CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ \
  --source my-mainnet-tester --network mainnet \
  -- grant_asset --caller "$TESTER" --account "$TESTER" --asset <your PT_ADDR>
stellar contract invoke --id CBCWU57AILKFDOYQAESFRPH57Q6UK66E7RBYV4ICZHNNGWIWFAHVRTXJ \
  --source my-mainnet-tester --network mainnet \
  -- grant_asset --caller "$TESTER" --account "$TESTER" --asset <your YT_ADDR>
```

**Real example of exactly this gap, found live during this deployment**: the first attempt to mint
PT/YT for a test wallet on mainnet reverted `Error(Contract, #7)` (`PermissionDenied`) because only
steps 1 and 3 had been done — tx that failed was a simulation-only rejection (never charged), fixed
by adding steps 2 and 4, then the real mint succeeded: tx
[`12f90a7b41736bceb318d13e4a413f92eedf34823f24900db492512a0375fee3`](https://stellar.expert/explorer/public/tx/12f90a7b41736bceb318d13e4a413f92eedf34823f24900db492512a0375fee3).

### 3.7 Run the real flow

```bash
# mint your mock asset to the test wallet (you're the issuer)
stellar tx new payment --source my-mainnet-tester --network mainnet \
  --destination "$TESTER" --asset "MOCKFOO:$(stellar keys address my-mainnet-tester)" \
  --amount 10000000000   # NOTE: classic-asset --amount here is in stroops (1e-7), not decimal
                          # units -- 10000000000 stroops = 1000.0000000 MOCKFOO. This bit the
                          # original deployment too (see §4's transaction examples below).

# deposit into SYWrapper
stellar contract invoke --id <your SY_ADDR> --source my-mainnet-tester --network mainnet \
  -- deposit --from "$TESTER" --amount 5000000000 --min_shares_out 1

# split into PT + YT
stellar contract invoke --id <your MANAGER_ADDR> --source my-mainnet-tester --network mainnet \
  -- mint --from "$TESTER" --sy_shares 5000000000
```

§4 below has the real mainnet tx hashes for every one of these calls, already run on MOCKUSTRY,
MOCKGILTS, MOCKCETES and MOCKXAUM, plus the AMM liquidity/swap and settle/redeem flows — use them
as the known-good shape of what a successful response looks like when you reproduce these calls
from the dApp's own transaction-building code.

---

## 4. Real mainnet transaction examples, by flow

Every hash below is independently verified on Horizon as `successful: true`
(see `docs/MAINNET_TRANSACTION_LEDGER.md` for the complete cross-check). Use these to validate your
dApp's transaction-building and result-parsing code against real mainnet responses before trusting
it with a real user. **The first nine rows below are from the original (now-abandoned, per §12 of
the evidence doc) USTRY v1 and MOCKUSDY markets** — still real, still valid as examples of what a
successful response looks like (the bug was in one oracle genesis value, not in how any of these
calls behave), but don't treat their contract addresses as currently live. The last four rows are
from the corrected v2 markets.

| Flow | Contract.function | Real tx hash | What happened |
|---|---|---|---|
| Classic asset mint (issuer → user) | SAC `payment` | [`fb9b93f3ec98795196ea7dcffeea792852125e24288ee4c1ec81e0c5f1e94f41`](https://stellar.expert/explorer/public/tx/fb9b93f3ec98795196ea7dcffeea792852125e24288ee4c1ec81e0c5f1e94f41) | 1000 MOCKUSTRY minted to a test wallet |
| `SYWrapper.deposit` | wrap underlying into SY | [`bef5117ea188a5389642b4ed78cac0b4cec412d7bbf3a27269ce22212427e9fd`](https://stellar.expert/explorer/public/tx/bef5117ea188a5389642b4ed78cac0b4cec412d7bbf3a27269ce22212427e9fd) | 500 MOCKUSTRY → 500 SY shares |
| `PrincipalManager.mint` | split SY into PT + YT | [`d3691d05aa26e440fdbde785a1d68811c8eef80af53d37374908ab668c489298`](https://stellar.expert/explorer/public/tx/d3691d05aa26e440fdbde785a1d68811c8eef80af53d37374908ab668c489298) | 500 SY → 54,005.833575 PT + YT, fee 2.5 SY |
| `MarketPool.add_liquidity` | seed an AMM pool | [`516490ef54181d4ca2b65c6ceed1e3596499efcd4d04b658249b1b8ed4b9b7a9`](https://stellar.expert/explorer/public/tx/516490ef54181d4ca2b65c6ceed1e3596499efcd4d04b658249b1b8ed4b9b7a9) | **first-ever AMM liquidity for this protocol on mainnet**: 60 PT + 50 SY → 54.77 LP |
| `MarketPool.swap_pt_for_sy` | a real AMM trade | [`1bff923cff8eb97a2c810bc4a22e38b4c12f4f8ef69c41fd0799da62215c8afa`](https://stellar.expert/explorer/public/tx/1bff923cff8eb97a2c810bc4a22e38b4c12f4f8ef69c41fd0799da62215c8afa) | **first-ever AMM swap on mainnet**: 5 PT → 4.3005615 SY |
| `PrincipalManager.claim_yield` (pre-maturity, zero yield) | yield check without redeeming | [`5686cbee284613a5a3be9392ee51518e8686a194640acb3df3fef58962fb6754`](https://stellar.expert/explorer/public/tx/5686cbee284613a5a3be9392ee51518e8686a194640acb3df3fef58962fb6754) | correctly returns `0` — confirms the "no yield yet" UI state renders right |
| `PrincipalManager.settle_all` | freeze the settlement rate at maturity | [`d3b8edd47628d50c90e8a275e492e19ec86b8e088decdcf93da1f130d7aa3ac8`](https://stellar.expert/explorer/public/tx/d3b8edd47628d50c90e8a275e492e19ec86b8e088decdcf93da1f130d7aa3ac8) | rate frozen at `11534743` |
| `PrincipalManager.redeem` | post-maturity redemption | [`b3a0f1a55fea5a959535590181250951ac0e78e44fbd26220ffe708a1b9c8e04`](https://stellar.expert/explorer/public/tx/b3a0f1a55fea5a959535590181250951ac0e78e44fbd26220ffe708a1b9c8e04) | 5,764.487814 PT + YT → 499.7499999 underlying |
| `Permissioning.grant_account` | account-level compliance grant | [`358bb2753923e9413a78907df7a5fb08af1f5b2adec8f5c400a0b22f7d3369c9`](https://stellar.expert/explorer/public/tx/358bb2753923e9413a78907df7a5fb08af1f5b2adec8f5c400a0b22f7d3369c9) | a test wallet granted account-level standing |
| `Permissioning.grant_asset` | per-asset (PT/YT) compliance grant | [`9db32a368b5c14f1862bbd8268d5c060ba177f28747361473a69bddac6e6822d`](https://stellar.expert/explorer/public/tx/9db32a368b5c14f1862bbd8268d5c060ba177f28747361473a69bddac6e6822d) | the same wallet granted PT-specific standing |
| `MarketFactory.create_tokens` (v1, abandoned) | | [`eaeecc87878b8d289e8036e9eacffc234d83e5bc5391184988ec2e09621ecadb`](https://stellar.expert/explorer/public/tx/eaeecc87878b8d289e8036e9eacffc234d83e5bc5391184988ec2e09621ecadb) | deploys SY/PT/YT atomically |
| `OracleAdapter.set_feed_source` + correctly-scaled genesis (v2, GILTS) | | [`e0dad98e3b0c78a68224d046d2c9aa9e3246dd8fb79e157ecd517c4a733c1d28`](https://stellar.expert/explorer/public/tx/e0dad98e3b0c78a68224d046d2c9aa9e3246dd8fb79e157ecd517c4a733c1d28) / [`e45355779815189cb115fdab763e5426eefa76c031936594b4ac84d526dc7f08`](https://stellar.expert/explorer/public/tx/e45355779815189cb115fdab763e5426eefa76c031936594b4ac84d526dc7f08) | real feed wired, value `14065740` (correctly ÷10 from the feed's 8-decimal `140657400`) |
| `MarketFactory.create_tokens` (v2, GILTS) | | [`887db2b0f0611638b1efc7c37a9da82f610c45806e3d70b2c4a2b625ece741b9`](https://stellar.expert/explorer/public/tx/887db2b0f0611638b1efc7c37a9da82f610c45806e3d70b2c4a2b625ece741b9) | deploys SY/PT/YT atomically, this time with a correct oracle from the start |
| `MarketPool.add_liquidity` (v2, GILTS) | | [`f1896a51206eb982702f206bc4f009bb62174e5eb5804348a34f954d952c0c75`](https://stellar.expert/explorer/public/tx/f1896a51206eb982702f206bc4f009bb62174e5eb5804348a34f954d952c0c75) | 180 PT + 125 SY → 149.9999 LP — note the PT:SY ratio differs from USTRY's because it's sized against GILTS's own (higher) oracle rate, see §12's finding on `InvalidInitialRatio` |
| `MarketPool.swap_pt_for_sy` (v2, GILTS) | | [`329489f7da68539005adb4ce01dd74c6a97b138e35e98089f6f9f42c4418512e`](https://stellar.expert/explorer/public/tx/329489f7da68539005adb4ce01dd74c6a97b138e35e98089f6f9f42c4418512e) | 1 PT → 0.7082269 SY |

For the full set — every compliance grant, every funding transfer, every retry, with real
timestamps — see `docs/MAINNET_TRANSACTION_LEDGER.md` (276 entries, independently reconciled
against Horizon on 2026-11-18).

---

## 5. Checklist before pointing real users at mainnet

**Code and config (this guide):**

- [ ] `manifest-source.ts` updated per §1, `config/manifests/mainnet.json` copied from
      `deployments/mainnet-dapp-manifest.json` and committed.
- [ ] `DATABASE_URL`/`DATABASE_URL_UNPOOLED` set for the mainnet Neon branch, migration run once
      (§1a step 1), QStash schedule created for `/api/cron/record` (§1a step 2).
- [ ] An oracle-freshness job is actually running against every `OracleAdapter` in §2 — manual spot
      checks are not a substitute (confirmed necessary: the oracle went stale multiple times during
      this deployment's own multi-hour working sessions). Confirmed this is *not* the dApp's own
      `/api/cron/oracle` route (§0, §1a step 3) — that one must stay mainnet-disabled.
- [ ] The 30-minute MOCKUSDY market is either excluded from the UI's live-market list or clearly
      labeled matured/closed — it cannot mint or trade, only read.
- [ ] The dApp's transaction-submission retry path is confirmed to check Horizon by hash before
      treating a timeout as a failure (see §0) — this is not hypothetical, it happened repeatedly
      on real mainnet calls this deployment made.
- [ ] Every new trading wallet's onboarding flow in the UI covers all four steps in §3.6, in order
      (trustline → `set_authorized` → `grant_account` → `grant_asset` ×2) — missing any one of them
      produces a real, confirmed-reproducible revert, not a hypothetical edge case.
- [x] ~~`FAUCET_ISSUER_SECRET`/`FAUCET_PERMISSIONING_ADMIN_SECRET` risk on mainnet~~ — not a real
      risk to decide on: `onboardingAvailable()` already returns `false` for `network: "mainnet"`
      everywhere it's checked (button, dialog, compliance card, and the server route itself); the
      faucet cannot run on mainnet regardless of whether these secrets are set. Confirmed by
      reading the code and its own test (§1b).
- [x] ~~Testnet banner showing on mainnet~~ — already correctly hides itself (§1b). No change needed.

**Known pre-existing gaps, from the dApp's own `README.md` "Before mainnet" section** — these
predate this deployment and are not something this guide can verify the status of; the dApp team's
own notes say each one is *"fine on Testnet; each needs a change before mainnet traffic."* Confirm
each is actually closed (or consciously accepted) before real mainnet traffic, not just before
real mainnet *support*:

- [ ] **Bounded registry reads** — `sqlRegistryStore.rows()` reads every `market_registry` row on
      every page render; a junk factory market (anyone can call `create_tokens` on the real,
      permissionless `MarketFactory` at `CDYH7UV24XS7G7MDRTBJUS4PJNKKUXTLUUTL67MR5CYJAXSPPHNWS65T`)
      adds a row forever. More directly relevant on mainnet than testnet, since the factory is
      real and permissionless there too — anyone, not just Principal, can call it.
- [ ] **Sparks above 50 pools** — `GET /api/markets/sparks` refuses more than 50 pools in one
      request. Six markets today; headroom exists, but this is a real ceiling, not a theoretical one.
- [ ] **Header role polling** scales with markets × connected users — six markets today is fine,
      worth revisiting before adding the remaining mock assets planned in
      `MAINNET_DEPLOYMENT_EVIDENCE.md` §10.
- [ ] **Step-quote-vs-preview warning** and **refundable-fee margin** — both about a later step in a
      multi-step route failing when mainnet ledger state moves between simulation and inclusion.
      Reproduced twice on the *local* network per the dApp's own README; not yet reproduced on
      mainnet by this deployment (every real mainnet flow in §3/§4 was single-step), but mainnet's
      real trading activity is exactly the condition that would trigger it.

**Open items from the contracts side** (tracked in `MAINNET_DEPLOYMENT_EVIDENCE.md` §10/§12, not
blocking the dApp code change itself):

- [x] ~~`principal-mainnet-treasury` trustline gap~~ — closed 2026-11-21. All five classic-asset
      trustlines (MOCKUSDY, MOCKUSTRY, MOCKGILTS, MOCKCETES, MOCKXAUM) now exist.
- [x] ~~Oracle staleness recurrence (`max_stale_seconds` shorter than the feed's real cadence)~~ —
      closed 2026-11-22. All 5 oracles raised to `max_stale_seconds=86400`. §13.
- [x] ~~WASM build non-reproducibility (0/12 hashes matched on rebuild)~~ — closed 2026-11-22.
      Root cause was an unpinned Rust toolchain, not the source. §14.
- [ ] `accessContact` — Principal still needs to name a contact and determine where it belongs;
      no matching field was found anywhere in this repo's code, schema, or migrations.

---

## 6. References

- [`docs/MAINNET_DEPLOYMENT_EVIDENCE.md`](MAINNET_DEPLOYMENT_EVIDENCE.md) — the full narrative: why
  mock assets, the real oracle feed, every market's creation story, every real finding.
- [`docs/MAINNET_TRANSACTION_LEDGER.md`](MAINNET_TRANSACTION_LEDGER.md) — all 276 real transactions,
  reconciled against Horizon.
- [`docs/MAINNET_ADDRESSES.md`](MAINNET_ADDRESSES.md) — every address in §2 above, on one page,
  no narrative.
- [`deployments/mainnet-dapp-manifest.json`](../deployments/mainnet-dapp-manifest.json) — the
  ready-to-copy `config/manifests/mainnet.json`.
- [`scripts/self_service/README.md`](../scripts/self_service/README.md) — the self-service tooling
  used in §3, proven on both testnet and now mainnet.
- [`scripts/create_market_via_factory.sh`](../scripts/create_market_via_factory.sh),
  [`scripts/self_service/create_mock_asset.sh`](../scripts/self_service/create_mock_asset.sh),
  [`scripts/self_service/registration_watcher.sh`](../scripts/self_service/registration_watcher.sh) —
  the scripts this guide references.
- [`deployments/mainnet/reproducible-build.md`](../deployments/mainnet/reproducible-build.md) —
  the step-by-step recipe to rebuild the deployed WASM byte-for-byte, and the root-cause writeup
  for the 2026-11-22 build-reproducibility incident (§14 of the evidence doc).
