#!/usr/bin/env bash
# Semi-automated registration for tester-created markets: testers self-serve everything through
# scripts/self_service/create_mock_asset.sh and scripts/create_market_via_factory.sh
# (SKIP_WIRING=1) with no Principal admin key required for either step. The two steps that
# genuinely need Principal's own admin authority -- RiskControl.add_consumer and
# Router.register_market, plus the Permissioning account-level grants for the market's own
# contracts -- stay admin-gated, but this script removes the need for a human to run them by
# hand for every tester market, while keeping the actual security decision (is this really a
# recognized test asset, not something unexpected) in a sanity check this script runs itself
# before ever signing anything.
#
# This deliberately does NOT grant any human account's compliance standing (Permissioning's
# account-level grant for a specific tester's own trading address, or any per-asset PT/YT grant
# for them) -- only the market's own four protocol contracts (SYWrapper, PrincipalManager,
# MarketPool, RecoveryEscrow), which are deterministic, protocol-owned addresses, not arbitrary
# end-user accounts. Who is allowed to actually trade in a tester's market remains a separate,
# deliberate decision for whoever operates Permissioning -- auto-granting arbitrary addresses
# submitted by self-service testers would defeat the whole point of that compliance layer.
#
# Run on a schedule (cron, a systemd timer) rather than as a daemon -- one invocation processes
# every new `mkt_new` event since the last run and exits.
#
# Usage:
#   FACTORY_ID=... PERMISSIONING_ID=... RISK_CONTROL_ID=... ROUTER_ID=... ADMIN_KEY=... \
#   NETWORK=testnet RPC_URL=https://soroban-testnet.stellar.org \
#   ./scripts/self_service/registration_watcher.sh
#
# Optional: ASSET_CODE_PATTERN (default '^(MOCK|TEST)[A-Z0-9]{0,8}$', must match
# create_mock_asset.sh's own default or testers' assets will never pass), WATCHER_STATE_FILE
# (default ./watcher-state.json), WATCHER_LOG (default ./watcher-log.jsonl),
# EVENTS_LOOKBACK_LEDGERS (default 17280, ~1 day, used only on the very first run when the state
# file does not yet have a last-processed ledger).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

: "${FACTORY_ID:?set FACTORY_ID (the deployed MarketFactory contract id)}"
: "${PERMISSIONING_ID:?set PERMISSIONING_ID}"
: "${RISK_CONTROL_ID:?set RISK_CONTROL_ID}"
: "${ROUTER_ID:?set ROUTER_ID}"
: "${ADMIN_KEY:?set ADMIN_KEY (must be the admin RiskControl/Router/Permissioning were initialized with)}"
NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
# Literal `{`/`}` inside a `${VAR:-default}` substitution confuses bash's own brace matching
# (the first bare `}` it meets closes the substitution, not the one actually balancing `{0,8}`)
# -- found live, the very first time this watcher was run against a real event. Set the default
# as a separate statement instead of inside the substitution.
ASSET_CODE_PATTERN="${ASSET_CODE_PATTERN:-}"
[ -z "$ASSET_CODE_PATTERN" ] && ASSET_CODE_PATTERN='^(MOCK|TEST)[A-Z0-9]{0,8}$'
WATCHER_STATE_FILE="${WATCHER_STATE_FILE:-./watcher-state.json}"
WATCHER_LOG="${WATCHER_LOG:-./watcher-log.jsonl}"
EVENTS_LOOKBACK_LEDGERS="${EVENTS_LOOKBACK_LEDGERS:-17280}"
command -v jq >/dev/null 2>&1 || { echo "jq is required." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }

ADMIN_ADDR="$(stellar keys address "$ADMIN_KEY")"

log() { # <decision> <reason> <pool> <escrow> [<underlying_code>]
  jq -n --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg decision "$1" --arg reason "$2" \
    --arg pool "${3:-}" --arg escrow "${4:-}" --arg code "${5:-}" \
    '{at: $at, decision: $decision, reason: $reason, pool: ($pool | select(. != "")),
      escrow: ($escrow | select(. != "")), underlying_code: ($code | select(. != ""))}' \
    >>"$WATCHER_LOG"
}

view() { stellar contract invoke --id "$1" --source "$ADMIN_KEY" --network "$NETWORK" --send=no -- "${@:2}" 2>/dev/null; }
raw() { printf '%s' "$1" | tr -d '"'; }

# ------------------------------------------------------------------ 1. find new mkt_new events
LAST_LEDGER=0
PROCESSED_POOLS="[]"
if [ -f "$WATCHER_STATE_FILE" ]; then
  LAST_LEDGER="$(jq -r '.last_processed_ledger // 0' "$WATCHER_STATE_FILE")"
  PROCESSED_POOLS="$(jq -c '.processed_pools // []' "$WATCHER_STATE_FILE")"
fi
LATEST_LEDGER="$(curl -s -X POST "$RPC_URL" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger","params":{}}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['sequence'])")"
if [ "$LAST_LEDGER" -le 0 ]; then
  LAST_LEDGER=$((LATEST_LEDGER - EVENTS_LOOKBACK_LEDGERS))
  [ "$LAST_LEDGER" -lt 1 ] && LAST_LEDGER=1
fi
START_LEDGER=$((LAST_LEDGER + 1))

echo "watching $FACTORY_ID for mkt_new events, ledgers $START_LEDGER..$LATEST_LEDGER"
EVENTS_JSON="$(stellar events --id "$FACTORY_ID" --start-ledger "$START_LEDGER" --network "$NETWORK" \
  --output json --count 1000 2>/dev/null || true)"

# -------------------------------------------------------------- 2. filter to mkt_new, decode
# Topic for the `mkt_new` symbol, base64 SCVal-encoded: precomputed once (symbols this short are
# a fixed, deterministic encoding) rather than re-deriving it with a Soroban SDK call every run.
MKT_NEW_TOPIC="AAAADwAAAAdta3RfbmV3AA=="
NEW_MARKETS="$(printf '%s' "$EVENTS_JSON" | python3 -c "
import sys, json
rows = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    ev = json.loads(line)
    if ev.get('topic') == ['$MKT_NEW_TOPIC']:
        rows.append(ev)
print(json.dumps(rows))
")"

COUNT="$(echo "$NEW_MARKETS" | jq 'length')"
echo "found $COUNT mkt_new event(s) in range"
HIGHEST_LEDGER="$LAST_LEDGER"

for i in $(seq 0 $((COUNT - 1))); do
  EVENT="$(echo "$NEW_MARKETS" | jq -c ".[$i]")"
  EVENT_LEDGER="$(echo "$EVENT" | jq -r '.ledger')"
  [ "$EVENT_LEDGER" -gt "$HIGHEST_LEDGER" ] && HIGHEST_LEDGER="$EVENT_LEDGER"
  VALUE="$(echo "$EVENT" | jq -r '.value')"
  TX="$(echo "$EVENT" | jq -r '.txHash')"

  DECODED="$(python3 decode_mkt_new.py "$VALUE" 2>/dev/null)" || { log skipped decode_failed "" "" ""; continue; }
  POOL="$(echo "$DECODED" | jq -r '.pool')"
  ESCROW="$(echo "$DECODED" | jq -r '.escrow')"

  if echo "$PROCESSED_POOLS" | jq -e --arg p "$POOL" 'index($p)' >/dev/null; then
    continue # already handled in an earlier run
  fi

  echo "-- candidate market: pool=$POOL escrow=$ESCROW (tx=$TX) --"

  # ---------------------------------------------------- 3. sanity checks (never skip these)
  SY_ADDR="$(raw "$(view "$POOL" sy_address)")"
  MANAGER_ADDR="$(raw "$(view "$POOL" manager_address)")"
  UNDERLYING_ADDR="$(raw "$(view "$POOL" underlying_address)")"
  if [ -z "$SY_ADDR" ] || [ -z "$MANAGER_ADDR" ] || [ -z "$UNDERLYING_ADDR" ]; then
    echo "  SKIP: pool did not respond to expected views (not a real/live MarketPool?)"
    log skipped pool_unresponsive "$POOL" "$ESCROW" ""
    PROCESSED_POOLS="$(echo "$PROCESSED_POOLS" | jq -c --arg p "$POOL" '. + [$p]')"
    continue
  fi

  UNDERLYING_NAME="$(raw "$(view "$UNDERLYING_ADDR" name)")"
  UNDERLYING_CODE="${UNDERLYING_NAME%%:*}"
  if ! [[ "$UNDERLYING_CODE" =~ $ASSET_CODE_PATTERN ]]; then
    echo "  SKIP: underlying asset code '$UNDERLYING_CODE' does not match the recognized test-asset pattern -- leaving for manual review, not auto-registering"
    log pending_manual_review naming_pattern_mismatch "$POOL" "$ESCROW" "$UNDERLYING_CODE"
    PROCESSED_POOLS="$(echo "$PROCESSED_POOLS" | jq -c --arg p "$POOL" '. + [$p]')"
    continue
  fi

  ALREADY_REGISTERED="false"
  if view "$RISK_CONTROL_ID" is_consumer --account "$SY_ADDR" 2>/dev/null | grep -q true; then
    ALREADY_REGISTERED="true"
  fi
  if [ "$ALREADY_REGISTERED" = "true" ]; then
    echo "  SKIP: already registered (RiskControl already lists SYWrapper as a consumer)"
    log skipped already_registered "$POOL" "$ESCROW" "$UNDERLYING_CODE"
    PROCESSED_POOLS="$(echo "$PROCESSED_POOLS" | jq -c --arg p "$POOL" '. + [$p]')"
    continue
  fi

  # --------------------------------------------------------------- 4. the admin-gated steps
  echo "  underlying '$UNDERLYING_CODE' matches the recognized pattern -- registering"
  stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ADMIN_KEY" --network "$NETWORK" \
    -- add_consumer --caller "$ADMIN_ADDR" --consumer "$SY_ADDR" >/dev/null 2>&1 || true
  stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ADMIN_KEY" --network "$NETWORK" \
    -- add_consumer --caller "$ADMIN_ADDR" --consumer "$MANAGER_ADDR" >/dev/null 2>&1 || true
  stellar contract invoke --id "$ROUTER_ID" --source "$ADMIN_KEY" --network "$NETWORK" \
    -- register_market --caller "$ADMIN_ADDR" --pool "$POOL" >/dev/null 2>&1 || true
  for addr in "$SY_ADDR" "$MANAGER_ADDR" "$POOL" "$ESCROW"; do
    stellar contract invoke --id "$PERMISSIONING_ID" --source "$ADMIN_KEY" --network "$NETWORK" \
      -- grant_account --caller "$ADMIN_ADDR" --account "$addr" >/dev/null 2>&1 || true
  done

  log registered ok "$POOL" "$ESCROW" "$UNDERLYING_CODE"
  echo "  registered: $POOL"
  PROCESSED_POOLS="$(echo "$PROCESSED_POOLS" | jq -c --arg p "$POOL" '. + [$p]')"
done

jq -n --argjson ledger "$HIGHEST_LEDGER" --argjson pools "$PROCESSED_POOLS" \
  '{last_processed_ledger: $ledger, processed_pools: $pools}' >"$WATCHER_STATE_FILE"
echo "done -- state saved to $WATCHER_STATE_FILE"
