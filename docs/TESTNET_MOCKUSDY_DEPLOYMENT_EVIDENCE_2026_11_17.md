# Testnet MOCKUSDY Deployment Evidence — Tranche 3 Internal Testing

**Purpose:** a fresh, standalone testnet deployment of the Tranche 3 internal-testing plan,
entirely separate from any earlier testnet work in this repository (`TESTNET_DEPLOYMENT_EVIDENCE*.md`,
`TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`, etc., which record different, prior deployments and are
left untouched). Mirrors `docs/MAINNET_DEPLOYMENT_EVIDENCE.md`'s structure, but on testnet — free
to run, no real-money cost, used to prove out the full flow (both markets, self-service tooling,
oracle relay) before mainnet funding is revisited.

**Status:** in progress. Entries are added only after the corresponding on-chain transaction has
actually been submitted and confirmed.

**Network:** Stellar testnet (`Test SDF Network ; September 2015`)
**Source commit (contracts):** `12f9b34` (`mainnet` branch — the contracts under test are
identical to the already fully-reviewed mainnet state; only the deployment target differs).

---

## 1. Why mock assets (same rationale as mainnet)

Principal cannot unilaterally create a market for a real RWA token it does not control. For
testing, Principal issues its own mock classic Stellar asset, MOCKUSDY, intended to track real
USDY's live price. Testnet does not have RedStone's real SEP-40 feed contract (that only exists
on mainnet, since it is a live financial oracle service) — so on testnet, `OracleAdapter` uses its
primary **push-based** path (`set_reference_value`) instead of the SEP-40 pull path, with values
sourced from a small relay script that reads the *real* mainnet feed and republishes it here (see
§6). This is itself a working first draft of the Tranche 3 "oracle relay" deliverable.

## 2. Keys generated for this deployment

All freshly generated for this deployment via `stellar keys generate ... --fund` (testnet
Friendbot, free), dedicated to this run only — not reused from any prior testnet work in this
repository.

| Role | Alias | Address |
|---|---|---|
| `ISSUER_KEY` | `testnet-mockusdy-issuer` | `GB3ZXKFW4Y3GOPPBAK4BD4RHAPSYUKUMRYLMHGJTWYJNQLEHYG6I22LW` |
| `ADMIN_KEY` | `testnet-mockusdy-admin` | `GD7YZSW4OQ2IEWLLXF5ZH267GEVW5QDR73V2XUG6DXPJ3UL5DNWQY5K4` |
| `TREASURY_ADDRESS` | `testnet-mockusdy-treasury` | `GBKOYQRXPAZV5CLQCYP6WWTHWZXVNLPT3LODG5FTOGT7DUS6VZYXOUEZ` |
| internal test account A | `testnet-mockusdy-test-a` | `GBSOJNKD3LSGVX7T362YFX4AZ4EGKTA6TZR7CW7L6ZYL4TTG5SSJFZUX` |
| internal test account B | `testnet-mockusdy-test-b` | `GAYNXRCK3LHVGXX7C3R5X3DILEESK7JUAD7DXX7LIIUQ73GJBKCUN5U6` |
| internal test account C | `testnet-mockusdy-test-c` | `GAUO2RBY23JAX7I42MUFZ6DGRJLBQYH6DLOVLVNMOZR52D457H5QTMYQ` |

All six funded with the standard Friendbot allotment (10,000 XLM each) — real transactions on
testnet, but zero real-world cost.

## 3. Mock asset: MOCKUSDY

**Issuer flags set** (`set-options --set-revocable --set-clawback-enabled`), matching real USDY's
own observed mainnet flags exactly — tx
[`b54b944db7e16390afb63486b9d9cdab820268f1a868584b60a95c22db1694f2`](https://stellar.expert/explorer/testnet/tx/b54b944db7e16390afb63486b9d9cdab820268f1a868584b60a95c22db1694f2).

**SAC (Soroban Asset Contract) address:** `CD4EYJ7LBMNEBJZ3TXO65Z3TVQ67MSFGFGVZVMXDV67BSSV74RWBG3QV`
— deployed via `stellar contract asset deploy --asset "MOCKUSDY:GB3ZXKFW4Y3GOPPBAK4BD4RHAPSYUKUMRYLMHGJTWYJNQLEHYG6I22LW"` —
tx [`ab8965d78893baa8035e04f07b8885be2c14eb62aa5ff86870e9566e491de28b`](https://stellar.expert/explorer/testnet/tx/ab8965d78893baa8035e04f07b8885be2c14eb62aa5ff86870e9566e491de28b).

**Trustlines established** (treasury + three internal test accounts):

| Account | Tx hash |
|---|---|
| `testnet-mockusdy-treasury` | `f6ce05af66c7a14cc8667c123095bb747d0de70fb9c7352100662dbae142f2ef` |
| `testnet-mockusdy-test-a` | `251929213364845db7ce9e23aa80ccc4157f5b0e130977420063cf5411418968` |
| `testnet-mockusdy-test-b` | `c77a4c649d57c5cc993fc056a7d96dfd82175198a1a73f2c11f905b0af008296` |
| `testnet-mockusdy-test-c` | `bbdbadd306a4474d05ae561facce87b5286a4cc9822b7589cdcf915ce1033400` |

## 4. Shared protocol infrastructure

Deployed in one run of `ENV_FILE=.env.testnet-mockusdy ./scripts/deploy/deploy.sh`
(manifest: `deployments/testnet-mockusdy/deployment-manifest.json`).

| Contract | ID | Deploy tx | Init tx |
|---|---|---|---|
| `OracleAdapter` | `CC362JNQUE34J355KT444SYPELO7JWQJZO5GWZPAAI2JHNCKDS4BEXER` | `a7bfe869996eac2def29a225ee75e56a32ba2122a46721a0f107495d76fa694e` | `3e35773f70844af084c9e6d325780a2e0dd069111984570661eeb1a51e67703a` |
| `Permissioning` | `CD6VQ2DDZOSJC25ICJC6CY47LIMBTXCHI7RHMQN7W4JVU7HP2PLO5VM2` | `5a45b825884ad15819f976f252910cb544e761c1003a2a5098849bd43ecca8ff` | `92e18cad23fdb6ed752631de43b46b1d9829ca65fcbfefd0c5903dc2524e66ee` |
| `RiskControl` | `CAWCDKT2GNM4JYSQPYHNWCELAWEWG7D32YSWU3PECXFMXEKUZY6BA3QP` | `007f83818b5ee01e9ab50d75df2022b2256abfdb90fe5505f9c56582214fa121` | `55e093f3aa06187588af7f04658fe911fc405d56c3a3f5744e07e4269a8a2742` |
| `Router` | `CDUR6KBZSYNU6R5UZS2FGHHA5OFLFN55C3QCBVDCRGBKGIS4FZ7VSSQP` | `5271904074dc0a9484e35f0323aeae2cb24a5e95f37ce9dcbe141e14377225c2` | `e64d24f239cf01cae3f5b00bdfadc648f514d9d40460f917d424ebad3739249c` |
| `MarketFactory` (stateless, no init) | `CBADNEHDX27XJITODTHC5AHVHT5PBWJKZZES7FNE7FFKSKGBONLTYDMC` | `75f4fc0974e3f633743c16d6742943a89a43770b4b9932ca4e8e6490ef73b9b7` | — |

**Oracle bootstrap:** manual genesis push of `11534743` (the real USDY price read live from
mainnet at deployment time, rescaled to the protocol's 1e7 scale) via `set_reference_value`. No
SEP-40 feed configured (none exists on testnet) — `is_available()` and
`last_update_was_feed() == false` confirmed by `verify-deployment.sh`.

**Full verification:** `MANIFEST=deployments/testnet-mockusdy/deployment-manifest.json
./scripts/deploy/verify-deployment.sh` → **78 passed, 1 failed** (the one failure is the expected
`git_tree_dirty` flag, since this working tree has uncommitted work in progress — not a real
defect). Every on-chain WASM hash matches both the manifest and this session's local build
artifacts; every topology cross-check, compliance grant, and wiring link passed.

*(Note: `verify-deployment.sh` does not read `ENV_FILE` — only a `MANIFEST` env var or `$1`. Running
it with only `ENV_FILE` set silently falls back to verifying the repository's older, unrelated
`deployments/testnet/deployment-manifest.json` instead. Always pass `MANIFEST=` explicitly.)

## 5. Markets

### Market 1 — 90-day reviewer market

Created atomically via `MarketFactory` (three-phase deterministic deploy), as part of the same
`deploy.sh` run above. `market_id` = `testnet-CD4EYJ-CCIARV55`. Maturity: `1802691992` (now + 90
days at deployment time).

| Contract | ID |
|---|---|
| `SYWrapper` | `CCIARV55BT34TADCG55XHWG7AJYDG5WLWIY26PAV2PFY4I7CMP5KHHZD` |
| `PTToken` | `CBNHGQTMUHSIVO6EKH2P72WOSKVPREKLCRXZ7ENPS4UCLD6GCFZJ42BB` |
| `YTToken` | `CAKLE3GQFICX6AJAGQN5UEWBP2WJGKY7IAP25Q2PACOMEOPEMT5YQ4O2` |
| `MarketConfig` | `CDNW2N2PAFTHMMS3BOZLJ2IP7N3OCSUPVHJVHDTK6UWWHJTULCLINTCP` |
| `PrincipalManager` | `CA3F5OLLDRGNA7QSO72TOFYBDN4RJ2JLVUZN3G6JLLWG7ZMEHWB36RG7` |
| `MarketPool` | `CCL64YMIHZW6TYGLEMH5T7Q2MVGPUSMGBJP7SKUSAEFRAFB6BSMCTSGP` |
| `RecoveryEscrow` | `CANHZ7WOGSRS7Z2KSWDW54733I5P2IMI6ZDMLYAUCUNCBXXMXQ6E5LXQ` |

**Factory transactions:**

| Step | Tx hash |
|---|---|
| `create_tokens` | `26ddf0206aa2ee8f25ddb24df0411163ed2b410831a1aacebb018435c08f5ab9` |
| `create_engine` | `8c0ded2712049b48024b7e30537467b94248f0dddb009acaf045ab338ac9cfbe` |
| `create_pool_and_escrow` | `8d1cb624e9bed386d57971a187f6d40d0b91262fa2ecc076f32895a98a942542` |
| `risk_control.add_consumer(SYWrapper)` | `73611ee72127902c727a47c0384c39eb170bbd836503d80ee87f1968676656f6` |
| `risk_control.add_consumer(PrincipalManager)` | `4cf714f54161a14ead4ad5aae2fd295e34d5f286ec0509ec5fd7ddee27f518ac` |
| `router.register_market` | `0405d3c5ebb27aa6cbb7a16b4b89eb124197702ff81128c57764720b27ec62d9` |

**Compliance grants** (account-level for SY/Manager/Pool/Escrow/treasury/3 test accounts, plus
per-asset PT/YT grants for Pool/Escrow/treasury/3 test accounts) — 20 transactions, all recorded
in the manifest's own `transactions[]` array and all individually `PASS`ed by
`verify-deployment.sh`.

### Market 2 — 30-minute internal flow-test market

Created via `scripts/create_market_via_factory.sh` directly (not a second `deploy.sh` run),
reusing the shared infrastructure above and the same already-uploaded per-market WASM hashes
(byte-identical code, no re-upload needed — confirmed cheap/idempotent on testnet). Maturity set
to `now + 30 minutes` at creation time = `1794918427`.

| Contract | ID |
|---|---|
| `SYWrapper` | `CAJHGQMO5TYQE6MASWSGVYAGJYFOGKMLONJU5MDIWY6LOKNI66QXMQW5` |
| `PTToken` (`PT-MUSDY30`) | `CCJBDS7IBF5D3IKUE5I2Y3C2IEVL326SK7MVVDKMMIVVL4SM7BNJX2GW` |
| `YTToken` (`YT-MUSDY30`) | `CAGCNO4SLFQAW3BVS2OU4Z4TQKATP2MJPFOMFVMF2EUBHGICUHFECQX7` |
| `MarketConfig` | `CARR7HYOJN3EM6HC2NYU2M2KBD3YVQCUELMNTAXMY62DKYMCBPSITYUC` |
| `PrincipalManager` | `CDH2DN5LK5A6URX7ANSHYYAHWBN5VK6WOQWB36P676QH5WLHIXWR6IIY` |
| `MarketPool` | `CDFJBPADQW2LGXHXJE7BGJTNKVHTOAUZSTVQ7ECQBQW2KBBEF25LLSQN` |
| `RecoveryEscrow` | `CBMKL4D5ML2P4D7P5B2UHPGZ3DFMOPW23ZZ5GTCMR347VP5H6RYJBPRR` |

**Factory + wiring transactions:**

| Step | Tx hash |
|---|---|
| `create_tokens` | `9b064fde07cfd209c982974c865da8294267b118c214bc0ccfa556b13f133619` |
| `create_engine` | `7eb8abb6563ce8b3d243811199c7ee90223c50206cf7aaacde2a41208f065296` |
| `create_pool_and_escrow` | `dfba5d93528179726454e8eeb7a70107efc5edb391006e91118c31e6a1bc1561` |
| `risk_control.add_consumer(SYWrapper)` | `3f380a53bfc6246b54e88520c07e7815be7803d79a91e065ebf219dffd05b5ad` |
| `risk_control.add_consumer(PrincipalManager)` | `7fd875b266bb67565459fc002329019375eafcfdee7ec34a698ae97addc81600` |
| `router.register_market` | `9dc3989af8dfdea2897cb1c89e1d120a50422a54ad1b5be5deb6e6916f74d569` |

**Compliance grants:** account-level grants for SY/Manager/Pool/Escrow (4 new contracts) plus
per-asset PT/YT grants for Pool/Escrow/treasury/3 test accounts (6 accounts × 2 assets) — 16
transactions total, all confirmed successful (done by hand via direct `stellar contract invoke`
calls mirroring `deploy.sh`'s own loop, since `create_market_via_factory.sh` deliberately leaves
this step to the caller).

### Market 2 — full lifecycle proof (mint → deposit → mint PT/YT → mature → settle → redeem)

Exercised live, end to end, within this one 30-minute market's real lifetime:

1. **Mint 1,000 MOCKUSDY** to `testnet-mockusdy-test-a` — tx
   `581e85534bf0333cf3bc8b6606a9af4720fbebe451cdd4586a17bc4f9e0ac622`.
2. **Deposit 500 MOCKUSDY** into `SYWrapper` — tx
   `3b484d8a225b026012be03f95572a5b39a7d94499f0576707bf8e60b138e3d36` → `5,000,000,000` SY shares.
3. **Mint PT+YT** from `2,000,000,000` SY shares via `PrincipalManager.mint` — tx
   `91cd685ff7355b86449c1d398e55c2e19d07cf169e37c26eb7318e2a16672f57` → `2,305,795,125` PT and
   `2,305,795,125` YT minted, `1,000,000` withheld as the tokenization fee (5 bps of
   2,000,000,000 = 1,000,000 — matches `TOKENIZATION_FEE_BPS=5` exactly).
4. **Market matures** (30 minutes after creation, `1794918427`).
5. **`settle_all`** (permissionless) — tx
   `1b1be0cbeefb14d0221d9a09c5fe2d73e490a000405a5476d505e8b39fe00f07` → froze the settlement rate
   at `11,534,743` — unchanged from genesis, since the real USDY price had not ticked between
   deployment and maturity (confirmed independently in §6's relay investigation).
6. **`redeem`** (PT=2,305,795,125, YT=2,305,795,125) for `testnet-mockusdy-test-a` — tx
   `e709b73efa065b02643e20ec8b3332e818bc16ccde34f4e9faacd55bdc5f3cc6` → `underlying_from_pt =
   1,998,999,999`, `underlying_from_yt = 0`.

**Economics check:** PT redemption (1,998,999,999) is within 1 raw unit of the 1,999,000,000 net
shares actually minted against (2,000,000,000 minus the 1,000,000 fee) — the 1-unit gap is
protocol-favoring floor-rounding, consistent with the security review's own documented
rounding-direction analysis. YT yield is exactly `0`, correctly matching `YTToken`'s own
"no yield without a rate increase" invariant, since the settlement rate never moved from genesis.
This is a complete, successful proof of the mint → trade-ready → mature → settle → redeem
lifecycle on a freshly deployed market, inside its own real (not simulated) 30-minute lifetime.

## 6. Oracle relay (testnet draft of the Tranche 3 deliverable)

`services/oracle-relay/relay.sh` already existed from earlier session work, implementing the
spec's §10.2 push-based relay design — but had never been run against a genuinely live SEP-40
feed before this session, and assumed the feed and the `OracleAdapter` being updated are on the
same network.

**Two real issues found and fixed while actually using it against RedStone's real mainnet feed:**

1. **Cross-network relaying wasn't supported.** Added `FEED_NETWORK` (defaults to `$NETWORK`,
   so every existing same-network caller is unaffected) so the script can fetch from one network
   (mainnet, where the real feed lives) and submit to another (testnet, where this mock asset's
   `OracleAdapter` lives).
2. **The installed `stellar` CLI cannot pretty-print a real feed's `lastprice()` return value**
   (its `Sep40PriceData` struct) — confirmed live: `stellar contract invoke ... -- lastprice`
   against RedStone's real feed exits 1 with `error: cannot print result ...: Missing Entry
   Sep40PriceData`, even though the simulation itself succeeded and the correctly decoded price
   and timestamp are visible in the CLI's own error text. This did not reproduce against whatever
   mock SEP-40 feed this script's own prior tests used, which is presumably why it went
   undetected until pointed at a real feed. Fixed with a `fetch_lastprice()` helper that tries
   the CLI's normal JSON path first, and falls back to regex-parsing that same error text for
   this one specific, identified failure signature — any other failure still surfaces as a real
   error, not silently swallowed.

**Live run against the real setup**, fetching from mainnet, submitting to this testnet deployment's
`OracleAdapter` (`CC362JNQUE34J355KT444SYPELO7JWQJZO5GWZPAAI2JHNCKDS4BEXER`):

- First attempt (default `RELAY_MAX_SOURCE_STALE_SECONDS=900`): correctly rejected locally as
  `source_stale` — the real feed's last update was ~4,895 seconds (~82 minutes) old. **This is a
  genuine, useful finding, not a relay bug**: RedStone's real USDY feed updates less frequently
  than the relay's own default 15-minute staleness threshold assumes. Consistent with finding
  M-14 in the security review (oracle parameters need per-asset tuning) — the relay's own
  staleness threshold is exactly this kind of parameter, now proven to need the same
  asset-specific calibration as the on-chain throttle.
- Second attempt (`RELAY_MAX_SOURCE_STALE_SECONDS=7200`, i.e. 2h, more realistic for this asset):
  passed the staleness/replay/deviation checks and genuinely submitted to chain — tx
  `50b816a7fdabb9137332569215ed122496c0d62361707216a651c2b05781da27`. Rejected **on-chain**, as
  `stale_ts`: the fetched source timestamp (`1794912290`) was from before this deployment's own
  genesis push (which used a later bootstrap timestamp), so there was nothing newer to accept —
  correct behavior, not a defect, and exactly the kind of gracefully-recorded rejection
  `OracleAdapter`'s own design intends. No real price change had occurred at the real feed
  between this deployment's genesis push and this relay attempt.

**Not yet proven:** an actual *accepted* real-feed update landing on-chain (requires either
waiting for RedStone's real feed to tick forward past this deployment's genesis timestamp and
re-running the relay, or a dedicated test against a feed that updates faster). The mechanism
itself — fetch, validate, rescale, submit, audit-log — is proven correct end to end; only a
genuinely-newer real tick was not available during this session to complete one full accepted
round-trip.

## 7. Self-service tester tooling

**Not yet built.** No existing script in this repository lets an external tester deploy their
own mock asset or create their own market (confirmed by search — nothing under any
`self-service`/`watcher`/`tester`-named path exists yet). This remains open from the original
Tranche 3 plan: a minting script for testers to deploy their own mock asset, plus the
semi-automated `Router.register_market` watcher that auto-registers a new `MarketFactory`-created
market after a topology/naming-pattern sanity check. Scoped as a separate, dedicated piece of
work rather than rushed into this deployment session.

## 8. Reproducible build evidence

Identical source commit and build as the mainnet deployment record
(`docs/MAINNET_DEPLOYMENT_EVIDENCE.md` §8) — the contracts under test are exactly the same;
only the deployment target differs. Source commit `12f9b34`, same toolchain
(`rustc 1.94.1`, `cargo 1.94.1`, `stellar 26.0.0`), same twelve WASM artifacts and SHA-256 hashes.
Not repeated here verbatim to avoid the two records drifting apart — see that document's §8 for
the full table. Every on-chain contract deployed in this testnet run had its WASM hash verified
against this same set by `scripts/deploy/verify-deployment.sh` (§4 above).
