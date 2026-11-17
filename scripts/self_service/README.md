# Self-service tester tooling

Lets an external tester deploy their own mock asset and create their own market with no
Principal admin key involved in either step, while keeping the two genuinely security-relevant
admin-gated actions (`RiskControl.add_consumer`, `Router.register_market`) under Principal's own
control — removed from being a manual, per-tester chore via `registration_watcher.sh`, not
removed as a security boundary. See each script's own header comment for full usage details;
this file is the end-to-end walkthrough.

## For a tester

1. **Generate and fund your own key** (testnet: `--fund` via Friendbot is free; mainnet: fund it
   yourself with real XLM — this tooling never moves funds on your behalf):
   ```bash
   stellar keys generate my-tester-key --network testnet --fund
   ```
2. **Create your own mock asset.** The asset code must start with `MOCK` or `TEST` (the default
   pattern both this script and the watcher use) or your market will never be auto-registered:
   ```bash
   ISSUER_KEY=my-tester-key ASSET_CODE=MOCKFOO NETWORK=testnet \
   ./scripts/self_service/create_mock_asset.sh
   ```
   Prints a `SAC_ID` — the Soroban Asset Contract address for your new asset.
3. **Make sure the shared `OracleAdapter` has a value fresher than its own `max_stale_seconds`**
   (ask whoever operates it, or check `is_available()` yourself) — `YTToken.initialize` reverts
   `OracleStale` otherwise. You cannot push to it yourself (it is Principal's own admin-gated
   shared infrastructure); this is one of the few places self-service genuinely depends on
   Principal keeping it running.
4. **Create your market**, using `scripts/create_market_via_factory.sh` with `SKIP_WIRING=1`
   (you do not have `RiskControl`/`Router` admin rights, and the factory does not need them for
   the three `create_*` calls — only your own asset's issuer authority, which you already have):
   ```bash
   FACTORY_ID=<the deployed MarketFactory> \
   ISSUER_KEY=my-tester-key ADMIN_KEY=my-tester-key \
   UNDERLYING_ID=<your SAC_ID> MATURITY_TS=$(date -d '+90 days' +%s) \
   PERMISSIONING_ID=<shared> ORACLE_ID=<shared> RISK_CONTROL_ID=<shared> \
   PROTOCOL_ADMIN_ADDR=<your own address> TREASURY_ADDR=<your own address> \
   SY_WASM=... PT_WASM=... YT_WASM=... CONFIG_WASM=... MANAGER_WASM=... POOL_WASM=... ESCROW_WASM=... \
   SKIP_WIRING=1 NETWORK=testnet \
   ./scripts/create_market_via_factory.sh
   ```
   (`ADMIN_KEY` is only used for the wiring calls this skips — pass anything; it is never
   invoked. The seven `*_WASM` hashes are whatever Principal's own shared deployment already
   uploaded — reuse them, no re-upload needed, since the per-market contract code is identical
   for every market.)
5. **Wait for the next watcher run** (Principal runs this on a schedule — ask how often). Once
   it has run, your market can accept deposits.

Granting *your own trading account's* Permissioning standing (so you personally can deposit/mint/
trade, as opposed to the market's own contracts being wired up) is a separate, deliberate step
Principal still does by hand per real tester — this tooling does not and should not auto-approve
arbitrary human accounts.

## For Principal's own ops: `registration_watcher.sh`

Run on a schedule (cron, a systemd timer):

```bash
FACTORY_ID=<...> PERMISSIONING_ID=<...> RISK_CONTROL_ID=<...> ROUTER_ID=<...> \
ADMIN_KEY=<the admin RiskControl/Router/Permissioning were initialized with> \
NETWORK=testnet RPC_URL=https://soroban-testnet.stellar.org \
./scripts/self_service/registration_watcher.sh
```

Each run: scans `MarketFactory`'s own `mkt_new` events since the last run (`watcher-state.json`
tracks the cursor), decodes each one's `(salt, pool, escrow)` payload
(`decode_mkt_new.py`, needs the `stellar_sdk` Python package), and for every pool not already
processed: reads its real on-chain topology (never trusts the event data as the final word),
resolves the underlying asset's real classic-asset code, and checks it against
`ASSET_CODE_PATTERN`. A match gets `RiskControl.add_consumer` (SY + PrincipalManager),
`Router.register_market`, and `Permissioning.grant_account` for the market's own four contracts
(SY/Manager/Pool/Escrow — never for any human account). A mismatch is logged as
`pending_manual_review` and left alone, not silently dropped — `watcher-log.jsonl` is this tool's
audit trail, mirroring `services/oracle-relay`'s own `audit-log.jsonl` convention; keep it, don't
truncate it.

## Proven end-to-end on testnet, 2026-11-17

A full self-service round-trip was run for real against the fresh testnet deployment recorded in
`docs/TESTNET_MOCKUSDY_DEPLOYMENT_EVIDENCE_2026_11_17.md` §7 — a freshly generated tester key
created its own `MOCKFOO` asset and market with zero Principal signatures, and
`registration_watcher.sh` discovered and correctly auto-registered it (confirmed independently
on-chain afterward: `RiskControl.is_consumer` true for both SY and Manager, `Router.market()`
returns the correct topology, `Permissioning.is_allowed` true for the pool). See that document
for the full transaction record, plus two real bugs this exercise found and fixed in these
scripts (a bash parameter-expansion gotcha with literal `{}` in a default value, and an oracle
staleness issue on a market created well after the shared `OracleAdapter`'s last push).
