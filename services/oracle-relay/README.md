# Oracle relay

Implements `docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md` §10.2's primary mainnet oracle
integration path: an off-chain script that fetches a SEP-40 feed (e.g. RedStone's USDY/USD
feed), validates and rescales it, and submits it through `OracleAdapter.set_reference_value` —
the same push ABI every other oracle update in this protocol already uses.

## What `relay.sh` does

One invocation = one attempted update, logged to `audit-log.jsonl` regardless of outcome
(submitted, rejected, or skipped — see the script's own `log_attempt` calls for the exact
shape). Run it on a schedule (cron, a systemd timer, a CI job) rather than as a daemon.

Checks performed, in order: replay/no-new-data (source timestamp must be newer than the last
*locally* accepted one, tracked in `relay-state.json`), local staleness (independent of and
tighter than `OracleAdapter`'s own `max_stale_seconds`), rescaling to `SCALE = 1e7`, a local
decrease check and a local deviation check (both mirror, not replace, `OracleAdapter`'s own
on-chain checks — this just avoids wasting a transaction that would revert/reject on-chain
anyway), and using ledger time (never later than it) for the submitted timestamp.

The deterministic source ID follows the exact preimage convention from
`docs/ORACLE_SAFETY_EXTENSION.md` / spec §5.2.2:
`sha256("principal-oracle-source:<network>:<asset>:<provider>:<feed>")`. The script computes
this itself from `RELAY_PROVIDER`/`RELAY_FEED_LABEL`/`RELAY_ASSET_CODE` — record the plaintext
preimage in the deployment manifest or acceptance doc too, since the on-chain `set_sources`
only ever stores the hash and a dashboard needs the preimage to display anything meaningful.

## What this does NOT cover (spec §10.4, §10.5 — out of this repo's scope)

This repo is contracts and deployment scripts; a production monitoring/alerting service and a
dashboard are not. Still needed elsewhere before this is a complete Tranche 3 Deliverable 3:

- Persistent metrics (last fetch/update timestamps, staleness seconds, rejection counts) beyond
  what `audit-log.jsonl` already records per-run — a real metrics store/scrape target.
- Alerting (feed unavailable, oracle aging past a warning threshold, deviation rejections,
  signer balance/authorization failures) to an actual channel (email/Discord/Slack/PagerDuty).
- Dashboard display of current value/freshness/source status/last update tx — this belongs with
  the dApp's own monitoring surface, in that repo, not here.
- A scheduler (cron/systemd timer/CI schedule) actually invoking `relay.sh` on a cadence — this
  script is the unit of work, not the scheduler.

## Before pointing this at a real feed

Do not use a guessed `RELAY_FEED_ID`. Confirm the actual deployed RedStone (or whichever
provider's) SEP-40 feed/adapter contract address from that provider's own current docs or app
immediately before configuring this — addresses recorded in any doc, including this one, can go
stale. `OracleAdapter.set_feed_source`/`sync_from_feed` (an additional, beyond-spec on-chain pull
capability — see `OracleAdapter`'s own module doc) needs the same caution and its own separate
review before any mainnet asset relies on it.
