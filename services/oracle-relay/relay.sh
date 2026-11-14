#!/usr/bin/env bash
# One-shot oracle relay: fetches a SEP-40 feed's current price (e.g. RedStone's USDY/USD feed),
# validates it, and submits it to OracleAdapter.set_reference_value -- the primary, spec-
# compliant mainnet oracle integration path (docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md §10.2).
# Meant to be invoked on a schedule (cron, a systemd timer, CI) rather than run as a
# long-lived daemon itself -- this repo's scope is contracts and deployment scripts, not a
# production monitoring/alerting service; see this directory's README for what Tranche 3
# Deliverable 3's full monitoring/alerting requirement (spec §10.4) still needs beyond this.
#
# Usage:
#   RELAY_FEED_ID=<SEP-40 feed contract> RELAY_FEED_ASSET_KIND=Other RELAY_FEED_ASSET_VALUE=USDY \
#   ORACLE_ID=<OracleAdapter contract> ORACLE_CALLER_KEY=<stellar keys identity> \
#   RELAY_PROVIDER=RedStone RELAY_FEED_LABEL=USDY/USD-SEP40 \
#   ./services/oracle-relay/relay.sh
#
# Every attempted update is appended to audit-log.jsonl (one JSON object per line) regardless of
# outcome -- submitted, rejected, or skipped -- per spec §10.2's audit-log requirement. This is
# part of oracle acceptance evidence, not just a debugging aid; do not delete or truncate it.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

NETWORK="${NETWORK:-testnet}"
RELAY_STATE_FILE="${RELAY_STATE_FILE:-./relay-state.json}"
AUDIT_LOG="${AUDIT_LOG:-./audit-log.jsonl}"
RELAY_MAX_SOURCE_STALE_SECONDS="${RELAY_MAX_SOURCE_STALE_SECONDS:-900}" # 15 min, independent of OracleAdapter's own 3600s
RELAY_RETRIES="${RELAY_RETRIES:-3}"

: "${RELAY_FEED_ID:?set RELAY_FEED_ID (the SEP-40 feed contract to read)}"
: "${RELAY_FEED_ASSET_KIND:?set RELAY_FEED_ASSET_KIND (Stellar or Other)}"
: "${RELAY_FEED_ASSET_VALUE:?set RELAY_FEED_ASSET_VALUE (the matching address or symbol)}"
: "${ORACLE_ID:?set ORACLE_ID (the OracleAdapter contract to update)}"
: "${ORACLE_CALLER_KEY:?set ORACLE_CALLER_KEY (a stellar keys identity configured as a source on ORACLE_ID)}"
: "${RELAY_PROVIDER:?set RELAY_PROVIDER, e.g. RedStone (used in the deterministic source-id preimage)}"
: "${RELAY_FEED_LABEL:?set RELAY_FEED_LABEL, e.g. USDY/USD-SEP40 (used in the deterministic source-id preimage)}"
: "${RELAY_ASSET_CODE:?set RELAY_ASSET_CODE, e.g. USDY (used in the deterministic source-id preimage)}"
command -v jq >/dev/null 2>&1 || { echo "jq is required." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }

PROTOCOL_DECIMALS=7
CALLER_ADDR="$(stellar keys address "$ORACLE_CALLER_KEY")"

# Deterministic source id, exact preimage convention from spec §5.2.2 / docs/ORACLE_SAFETY_EXTENSION.md:
#   source_id = sha256("principal-oracle-source:<network>:<asset>:<provider>:<feed>")
PREIMAGE="principal-oracle-source:${NETWORK}:${RELAY_ASSET_CODE}:${RELAY_PROVIDER}:${RELAY_FEED_LABEL}"
SOURCE_ID="$(printf '%s' "$PREIMAGE" | sha256sum | awk '{print $1}')"

log_attempt() { # <decision> <reason> <source_value> <scaled_value> <source_ts> <ledger_ts> <prev_value> <tx>
  jq -n \
    --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg preimage "$PREIMAGE" --arg source_id "$SOURCE_ID" \
    --arg decision "$1" --arg reason "$2" \
    --arg raw "$3" --arg scaled "$4" --arg source_ts "$5" --arg ledger_ts "$6" --arg prev "$7" --arg tx "$8" \
    '{at: $at, source_preimage: $preimage, source_id: $source_id, decision: $decision, reason: $reason,
      raw_source_value: $raw, scaled_value: $scaled, source_timestamp: ($source_ts | tonumber? // null),
      ledger_timestamp: ($ledger_ts | tonumber? // null), previous_onchain_value: ($prev | tonumber? // null),
      tx: ($tx | select(. != "") // null)}' >>"$AUDIT_LOG"
}

retry() { # <description> <command...>
  local desc="$1"; shift
  local n=0
  until "$@"; do
    n=$((n + 1))
    if [ "$n" -ge "$RELAY_RETRIES" ]; then
      echo "giving up on $desc after $n attempts" >&2
      return 1
    fi
    echo "retrying $desc (attempt $((n + 1))/$RELAY_RETRIES)..." >&2
    sleep $((n * 2))
  done
}

view() { stellar contract invoke --id "$1" --source "$ORACLE_CALLER_KEY" --network "$NETWORK" --send=no -- "${@:2}"; }
raw_json() { printf '%s' "$1" | tr -d '\n'; }

# -------------------------------------------------------------------------- 1. fetch the feed
FEED_RESULT="$(retry "fetch lastprice" view "$RELAY_FEED_ID" lastprice --asset "{\"$RELAY_FEED_ASSET_KIND\":\"$RELAY_FEED_ASSET_VALUE\"}")" \
  || { log_attempt skipped feed_fetch_failed "" "" "" "" "" ""; exit 1; }
if [ "$(raw_json "$FEED_RESULT")" = "null" ]; then
  echo "feed returned no data for this asset" >&2
  log_attempt rejected no_data "" "" "" "" "" ""
  exit 0
fi
SOURCE_VALUE="$(echo "$FEED_RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin)['price'])")"
SOURCE_TS="$(echo "$FEED_RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin)['timestamp'])")"
FEED_DECIMALS="$(retry "fetch decimals" view "$RELAY_FEED_ID" decimals | tr -d '"')"

# ------------------------------------------------------------ 2. replay / staleness (local)
LAST_SOURCE_TS=0
if [ -f "$RELAY_STATE_FILE" ]; then
  LAST_SOURCE_TS="$(jq -r '.last_source_timestamp // 0' "$RELAY_STATE_FILE")"
fi
if [ "$SOURCE_TS" -le "$LAST_SOURCE_TS" ]; then
  echo "source timestamp $SOURCE_TS is not newer than the last accepted $LAST_SOURCE_TS -- replay or no new data" >&2
  log_attempt rejected replayed_or_no_new_data "$SOURCE_VALUE" "" "$SOURCE_TS" "" "" ""
  exit 0
fi
NOW="$(date +%s)"
if [ $((NOW - SOURCE_TS)) -gt "$RELAY_MAX_SOURCE_STALE_SECONDS" ]; then
  echo "source data is $((NOW - SOURCE_TS))s old, over the relay's own ${RELAY_MAX_SOURCE_STALE_SECONDS}s threshold" >&2
  log_attempt rejected source_stale "$SOURCE_VALUE" "" "$SOURCE_TS" "" "" ""
  exit 0
fi

# ---------------------------------------------------------- 3. rescale to protocol SCALE=1e7
SCALED_VALUE="$(python3 -c "
source, decimals, target = $SOURCE_VALUE, $FEED_DECIMALS, $PROTOCOL_DECIMALS
if decimals == target:
    print(source)
elif decimals < target:
    print(source * 10 ** (target - decimals))
else:
    print(source // 10 ** (decimals - target))
")"
if [ "$SCALED_VALUE" -le 0 ]; then
  echo "rescaled value is non-positive ($SCALED_VALUE)" >&2
  log_attempt rejected bad_price "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "" "" ""
  exit 0
fi

# ------------------------------------------------- 4. compare against the on-chain value, local
CURRENT_VALUE="$(view "$ORACLE_ID" get_reference_value | tr -d '"')"
MAX_DEVIATION_BPS="$(view "$ORACLE_ID" max_deviation_bps | tr -d '"')"
LEDGER_TS="$(stellar rpc get-latest-ledger --rpc-url "${RPC_URL:-https://soroban-testnet.stellar.org}" 2>/dev/null \
  | python3 -c "import sys,json; print(json.load(sys.stdin).get('ledgerCloseTime', $NOW))" 2>/dev/null || echo "$NOW")"

if [ "$SCALED_VALUE" -lt "$CURRENT_VALUE" ]; then
  echo "scaled value $SCALED_VALUE is below the current on-chain value $CURRENT_VALUE -- would be rejected as ValueDecreased on-chain anyway, not submitting" >&2
  log_attempt rejected decrease "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "$LEDGER_TS" "$CURRENT_VALUE" ""
  exit 0
fi
if [ "$CURRENT_VALUE" -gt 0 ]; then
  DEVIATION_BPS="$(python3 -c "print(($SCALED_VALUE - $CURRENT_VALUE) * 10000 // $CURRENT_VALUE)")"
  if [ "$DEVIATION_BPS" -gt "$MAX_DEVIATION_BPS" ]; then
    echo "deviation ${DEVIATION_BPS}bps exceeds max ${MAX_DEVIATION_BPS}bps -- not submitting" >&2
    log_attempt rejected deviate "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "$LEDGER_TS" "$CURRENT_VALUE" ""
    exit 0
  fi
fi
if [ "$LEDGER_TS" -lt "$SOURCE_TS" ]; then
  # Use ledger time, never later than it -- spec §10.2's "use ledger close time or a timestamp
  # not later than ledger time."
  SUBMIT_TS="$LEDGER_TS"
else
  SUBMIT_TS="$SOURCE_TS"
fi

# -------------------------------------------------------------------- 5. submit, with retries
echo "submitting: source=$SOURCE_VALUE (decimals=$FEED_DECIMALS) -> scaled=$SCALED_VALUE @ $SUBMIT_TS (current=$CURRENT_VALUE)"
ERR_FILE="$(mktemp)"
OUT_FILE="$(mktemp)"
if ! retry "set_reference_value" bash -c \
  "stellar contract invoke --id '$ORACLE_ID' --source '$ORACLE_CALLER_KEY' --network '$NETWORK' \
    -- set_reference_value --caller '$CALLER_ADDR' --source '$SOURCE_ID' \
       --value '$SCALED_VALUE' --timestamp '$SUBMIT_TS' >'$OUT_FILE' 2>'$ERR_FILE'"
then
  log_attempt skipped submission_failed "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "$LEDGER_TS" "$CURRENT_VALUE" ""
  cat "$ERR_FILE" >&2
  rm -f "$ERR_FILE" "$OUT_FILE"
  exit 1
fi
TX_HASH="$(grep -oE 'Signing transaction: [0-9a-f]+' "$ERR_FILE" | awk '{print $3}' || true)"
rm -f "$ERR_FILE"

# A policy rejection (decrease/deviate/stale on-chain) is a *successful* transaction that returns
# false, not a reverted call -- the retry above only sees the CLI's own exit code, which is 0
# either way. Without decoding the actual return value, a rejection got logged as "submitted ok"
# and, worse, replay-state still advanced to this source timestamp, so a later, corrected
# resubmission of the exact same tick would be silently dropped as already-seen. Found live
# reviewing this call site -- never reproduced against a real feed in this pass (see report).
SET_RESULT="$(tr -d '"[:space:]' <"$OUT_FILE")"
rm -f "$OUT_FILE"
if [ "$SET_RESULT" != "true" ]; then
  echo "oracle rejected this update on-chain (set_reference_value returned '$SET_RESULT', tx=$TX_HASH) -- not advancing replay state" >&2
  log_attempt skipped policy_rejected "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "$LEDGER_TS" "$CURRENT_VALUE" "$TX_HASH"
  jq -n --arg tx "$TX_HASH" --arg scaled "$SCALED_VALUE" --arg prev "$CURRENT_VALUE" \
    '{submitted: false, tx: $tx, attempted_value: ($scaled | tonumber), previous_value: ($prev | tonumber)}'
  exit 1
fi

jq -n --arg ts "$SOURCE_TS" '{last_source_timestamp: ($ts | tonumber)}' >"$RELAY_STATE_FILE"
log_attempt submitted ok "$SOURCE_VALUE" "$SCALED_VALUE" "$SOURCE_TS" "$LEDGER_TS" "$CURRENT_VALUE" "$TX_HASH"

jq -n --arg tx "$TX_HASH" --arg scaled "$SCALED_VALUE" --arg prev "$CURRENT_VALUE" --arg dev "${DEVIATION_BPS:-0}" \
  '{submitted: true, tx: $tx, new_value: ($scaled | tonumber), previous_value: ($prev | tonumber),
    deviation_bps: ($dev | tonumber)}'
