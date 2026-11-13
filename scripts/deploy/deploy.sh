#!/usr/bin/env bash
# Full, idempotent, .env-driven deployment of the Principal protocol's shared infrastructure
# plus one market, to whichever network .env points at. Implements
# docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md §5.3's deployment pipeline, writing
# deployments/<network>/deployment-manifest.json in the exact shape §5.5 specifies.
#
# Usage:
#   cp .env.example .env   # fill in ISSUER_KEY, ADMIN_KEY, UNDERLYING_SAC, etc.
#   cargo build --workspace --release --target wasm32v1-none
#   ./scripts/deploy/deploy.sh
#   ./scripts/deploy/verify-deployment.sh
#
# Idempotent (§5.4): a contract already recorded in the manifest is reused (verified alive with
# a cheap read call, not just trusted) unless FORCE_REDEPLOY=true. One-time setters already
# applied are verified, not retried. The manifest is backed up (timestamped copy) before every
# overwrite, and a failed run can simply be re-run -- everything already recorded is skipped.
#
# This script calls scripts/create_market_via_factory.sh for the per-market contracts (the
# three create_* factory calls plus wiring) rather than duplicating that logic.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

ENV_FILE="${ENV_FILE:-.env}"
if [ -f "$ENV_FILE" ]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
else
  echo "No $ENV_FILE found -- copy .env.example to $ENV_FILE and fill it in first." >&2
  exit 1
fi

: "${ISSUER_KEY:?set ISSUER_KEY in $ENV_FILE}"
: "${ADMIN_KEY:?set ADMIN_KEY in $ENV_FILE}"
: "${UNDERLYING_SAC:?set UNDERLYING_SAC in $ENV_FILE}"
: "${PROTOCOL_ADMIN_ADDRESS:?set PROTOCOL_ADMIN_ADDRESS in $ENV_FILE}"
: "${TREASURY_ADDRESS:?set TREASURY_ADDRESS in $ENV_FILE}"
command -v jq >/dev/null 2>&1 || { echo "jq is required." >&2; exit 1; }

NETWORK="${NETWORK:-testnet}"
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
MANIFEST="${MANIFEST:-deployments/$NETWORK/deployment-manifest.json}"
WASM_DIR="${WASM_DIR:-target/wasm32v1-none/release}"
ORACLE_MAX_DEVIATION_BPS="${ORACLE_MAX_DEVIATION_BPS:-1000}"
ORACLE_MAX_STALE_SECONDS="${ORACLE_MAX_STALE_SECONDS:-3600}"
ORACLE_INITIAL_VALUE="${ORACLE_INITIAL_VALUE:-10000000}"
ORACLE_INITIAL_TIMESTAMP="${ORACLE_INITIAL_TIMESTAMP:-$(($(date +%s) - 2))}"
RISK_CB_LIMIT="${RISK_CB_LIMIT:-0}"
RISK_WINDOW_LEDGERS="${RISK_WINDOW_LEDGERS:-17280}"
MARKET_MATURITY_UNIX="${MARKET_MATURITY_UNIX:-$(( $(date +%s) + 90 * 86400 ))}"
FORCE_REDEPLOY="${FORCE_REDEPLOY:-false}"

ISSUER_ADDR="$(stellar keys address "$ISSUER_KEY")"
ADMIN_ADDR="$(stellar keys address "$ADMIN_KEY")"
SOURCE_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
GIT_TREE_DIRTY="$([ -z "$(git status --porcelain 2>/dev/null)" ] && echo false || echo true)"

echo "== deploy.sh: network=$NETWORK issuer=$ISSUER_ADDR admin=$ADMIN_ADDR manifest=$MANIFEST =="
[ "$GIT_TREE_DIRTY" = true ] && echo "WARNING: working tree is dirty -- git_tree_dirty=true will be recorded; not acceptable for an acceptance deployment (spec §4.4)." >&2

mkdir -p "$(dirname "$MANIFEST")"

# --------------------------------------------------------------------------- manifest plumbing
if [ -f "$MANIFEST" ]; then
  BACKUP="${MANIFEST}.$(date -u +%Y%m%dT%H%M%SZ).bak"
  cp "$MANIFEST" "$BACKUP"
  echo "Backed up existing manifest to $BACKUP (spec §5.4: never overwrite without a backup)."
else
  jq -n \
    --arg sv "1.0" --arg net "$NETWORK" --arg pass "$NETWORK_PASSPHRASE" \
    --arg rpc "$RPC_URL" --arg hor "$HORIZON_URL" --arg commit "$SOURCE_COMMIT" \
    --arg built "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg target "wasm32v1-none" \
    --argjson dirty "$([ "$GIT_TREE_DIRTY" = true ] && echo true || echo false)" \
    '{schema_version: $sv, network: $net, network_passphrase: $pass, rpc_url: $rpc,
      horizon_url: $hor, source_commit: $commit, built_at: $built, wasm_target: $target,
      git_tree_dirty: $dirty, oracle_abi_version: 2,
      rustc_version: "", stellar_cli_version: "", soroban_sdk_version: "", release_tag: "",
      underlying: {}, market: {}, contracts: {}, wiring: {}, transactions: []}' \
    >"$MANIFEST"
fi

# Audit fields (spec §5.5) that are cheap to fill in now rather than leave blank.
RUSTC_VERSION="$(rustc --version 2>/dev/null || echo unknown)"
STELLAR_CLI_VERSION="$(stellar --version 2>/dev/null | head -1 || echo unknown)"
SOROBAN_SDK_VERSION="$(grep -m1 'soroban-sdk = { version' contracts/oracle_adapter/Cargo.toml 2>/dev/null \
  | sed -E 's/.*version = "([^"]+)".*/\1/' || echo unknown)"
_tmp="$(mktemp)"
jq --arg rustc "$RUSTC_VERSION" --arg cli "$STELLAR_CLI_VERSION" --arg sdk "$SOROBAN_SDK_VERSION" \
   --arg commit "$SOURCE_COMMIT" --argjson dirty "$([ "$GIT_TREE_DIRTY" = true ] && echo true || echo false)" \
   --arg built "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '.rustc_version = $rustc | .stellar_cli_version = $cli | .soroban_sdk_version = $sdk |
   .source_commit = $commit | .git_tree_dirty = $dirty | .built_at = $built' \
  "$MANIFEST" >"$_tmp"
mv "$_tmp" "$MANIFEST"

manifest_contract_field() { jq -r --arg n "$1" --arg f "$2" '.contracts[$n][$f] // empty' "$MANIFEST"; }

manifest_set_contract() { # <name> <id> <wasm_path> <wasm_sha256> <deploy_tx> <init_tx> <admin_address>
  local _tmp
  _tmp="$(mktemp)"
  jq --arg n "$1" --arg id "$2" --arg wp "$3" --arg wh "$4" --arg dt "$5" --arg it "$6" --arg adm "$7" \
    '.contracts[$n] = {id: $id, wasm_path: $wp, wasm_sha256: $wh, deploy_tx: $dt, init_tx: $it, admin_address: $adm}' \
    "$MANIFEST" >"$_tmp"
  mv "$_tmp" "$MANIFEST"
}

manifest_log_tx() { # <step> <tx_hash>
  [ -z "${2:-}" ] && return 0
  local _tmp
  _tmp="$(mktemp)"
  jq --arg step "$1" --arg tx "$2" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '.transactions += [{step: $step, tx: $tx, at: $ts}]' "$MANIFEST" >"$_tmp"
  mv "$_tmp" "$MANIFEST"
}

manifest_set_path() { # <jq path expr, e.g. ".market.market_id"> <value>
  local _tmp
  _tmp="$(mktemp)"
  jq --arg v "$2" "$1 = \$v" "$MANIFEST" >"$_tmp"
  mv "$_tmp" "$MANIFEST"
}

manifest_set_path_num() { # <jq path> <numeric value>
  local _tmp
  _tmp="$(mktemp)"
  jq --argjson v "$2" "$1 = \$v" "$MANIFEST" >"$_tmp"
  mv "$_tmp" "$MANIFEST"
}

# ---------------------------------------------------------------------------- CLI wrappers
# Both wrappers capture the submitted transaction's hash from the CLI's own stderr ("Signing
# transaction: <hash>") -- `stellar contract invoke`/`upload`/`deploy` offer no structured way
# to get it otherwise (confirmed empirically against the installed CLI version).
#
# This has to be a file, not a plain global variable: `upload_wasm`/`deploy_from_hash` are always
# called as `x="$(upload_wasm ...)"` (their actual return value -- a hash or contract id -- has
# to come back as captured stdout), and command substitution always forks a subshell. Any
# `LAST_TX=...` assignment made *inside* that subshell is invisible to the parent shell the
# instant the subshell exits -- so a plain variable here silently stayed empty forever. Found
# live: this is why every `deploy_tx` in the manifest came back "" on the first real run, even
# though every `init_tx` (set from a plain, non-substituted `invoke ... >/dev/null` call, which
# never forks) was correctly populated the whole time.
LAST_TX_FILE="$(mktemp)"
trap 'rm -f "$LAST_TX_FILE"' EXIT
LAST_TX=""

# All three wrappers below: if the underlying `stellar` call fails, print its captured stderr
# before letting `set -e` abort the script -- without this, a failure deep inside one of these
# died completely silently (the stderr was captured to a tempfile and only ever read for its
# "Signing transaction: ..." line, never shown, and `rm -f` ran before the script could die on
# the failing assignment) (found live: the very first real run of this script against Testnet).
invoke() { # <contract_id> <source_key> -- <fn> <args...>
  local id="$1" src="$2"
  shift 2
  local err out
  err="$(mktemp)"
  if ! out="$(stellar contract invoke --id "$id" --source "$src" --network "$NETWORK" "$@" 2>"$err")"; then
    echo "invoke failed: --id $id --source $src -- $*" >&2
    cat "$err" >&2
    rm -f "$err"
    return 1
  fi
  LAST_TX="$(grep -oE 'Signing transaction: [0-9a-f]+' "$err" | awk '{print $3}' || true)"
  printf '%s' "$LAST_TX" >"$LAST_TX_FILE"
  rm -f "$err"
  printf '%s' "$out"
}

upload_wasm() { # <path> <source_key>  -> prints wasm hash
  local wasm="$1" src="$2"
  local err out
  err="$(mktemp)"
  if ! out="$(stellar contract upload --wasm "$wasm" --source "$src" --network "$NETWORK" 2>"$err")"; then
    echo "upload_wasm failed: $wasm" >&2
    cat "$err" >&2
    rm -f "$err"
    return 1
  fi
  LAST_TX="$(grep -oE 'Signing transaction: [0-9a-f]+' "$err" | awk '{print $3}' || true)"
  printf '%s' "$LAST_TX" >"$LAST_TX_FILE"
  rm -f "$err"
  printf '%s' "$out" | tr -d '[:space:]"'
}

deploy_from_hash() { # <wasm_hash> <source_key>  -> prints contract id
  local hash="$1" src="$2"
  local err out
  err="$(mktemp)"
  if ! out="$(stellar contract deploy --wasm-hash "$hash" --source "$src" --network "$NETWORK" 2>"$err")"; then
    echo "deploy_from_hash failed: $hash" >&2
    cat "$err" >&2
    rm -f "$err"
    return 1
  fi
  LAST_TX="$(grep -oE 'Signing transaction: [0-9a-f]+' "$err" | awk '{print $3}' || true)"
  printf '%s' "$LAST_TX" >"$LAST_TX_FILE"
  rm -f "$err"
  printf '%s' "$out" | tr -d '[:space:]"'
}

# Call after any of the three wrappers above was itself invoked via `x="$(wrapper ...)"` (a
# subshell) -- reads the hash back out of the file side channel documented at LAST_TX_FILE above.
read_last_tx() { LAST_TX="$(cat "$LAST_TX_FILE" 2>/dev/null || true)"; }

raw() { printf '%s' "$1" | tr -d '"'; }

# ------------------------------------------------------------------ shared-infra idempotency
is_alive() { stellar contract invoke --id "$1" --source "$ADMIN_KEY" --network "$NETWORK" --send=no -- "$2" >/dev/null 2>&1; }

deploy_or_reuse() { # <name> <crate_filename_stem> <source_key> <init_fn_or_empty> <init_args...>
  local name="$1" stem="$2" src="$3" init_fn="$4"
  shift 4
  local id="$(manifest_contract_field "$name" id)"
  # Every caller assigns this function's output via `X_ID="$(deploy_or_reuse ...)"`, which
  # captures ALL of this function's stdout, not just its final `printf`. A status `echo` here
  # must go to stderr (>&2) or it silently becomes part of the captured contract ID (found live:
  # the very first run corrupted every shared-infra ID this way, since each one is >1 line of
  # garbage once the status line is prepended, which `stellar contract invoke --id` then rejects).
  if [ "$FORCE_REDEPLOY" != "true" ] && [ -n "$id" ] && is_alive "$id" get_admin; then
    echo "$name: reusing $id (already deployed and alive)" >&2
    printf '%s' "$id"
    return 0
  fi
  echo "$name: deploying fresh" >&2
  local wasm_path="$WASM_DIR/principal_${stem}.wasm"
  local hash deploy_tx init_tx=""
  hash="$(upload_wasm "$wasm_path" "$src")"
  id="$(deploy_from_hash "$hash" "$src")"
  read_last_tx
  deploy_tx="$LAST_TX"
  manifest_log_tx "deploy:$name" "$deploy_tx"
  if [ -n "$init_fn" ]; then
    invoke "$id" "$src" -- "$init_fn" "$@" >/dev/null
    init_tx="$LAST_TX"
    manifest_log_tx "init:$name" "$init_tx"
  fi
  manifest_set_contract "$name" "$id" "$wasm_path" "$hash" "$deploy_tx" "$init_tx" "$ADMIN_ADDR"
  printf '%s' "$id"
}

echo
echo "-- Shared infrastructure --"

ORACLE_ID="$(deploy_or_reuse oracle_adapter oracle_adapter "$ADMIN_KEY" initialize \
  --admin "$ADMIN_ADDR" \
  --max_stale_seconds "$ORACLE_MAX_STALE_SECONDS" --max_deviation_bps "$ORACLE_MAX_DEVIATION_BPS")"

PERMISSIONING_ID="$(deploy_or_reuse permissioning permissioning "$ADMIN_KEY" initialize --admin "$ADMIN_ADDR")"

RISK_CONTROL_ID="$(deploy_or_reuse risk_control risk_control "$ADMIN_KEY" initialize \
  --admin "$ADMIN_ADDR" --cb_limit "$RISK_CB_LIMIT")"
if [ "$RISK_WINDOW_LEDGERS" != "17280" ]; then
  CURRENT_WINDOW="$(raw "$(invoke "$RISK_CONTROL_ID" "$ADMIN_KEY" --send=no -- get_window_ledgers)")"
  if [ "$CURRENT_WINDOW" != "$RISK_WINDOW_LEDGERS" ]; then
    invoke "$RISK_CONTROL_ID" "$ADMIN_KEY" -- set_window_ledgers \
      --caller "$ADMIN_ADDR" --ledgers "$RISK_WINDOW_LEDGERS" >/dev/null
    manifest_log_tx "risk_control:set_window_ledgers" "$LAST_TX"
  fi
fi
if [ -n "${RISK_ASSET_LIMIT:-}" ]; then
  invoke "$RISK_CONTROL_ID" "$ADMIN_KEY" -- set_asset_limit \
    --caller "$ADMIN_ADDR" --asset "$UNDERLYING_SAC" --new_limit "$RISK_ASSET_LIMIT" >/dev/null
  manifest_log_tx "risk_control:set_asset_limit" "$LAST_TX"
fi

ROUTER_ID="$(deploy_or_reuse router router "$ADMIN_KEY" initialize --admin "$ADMIN_ADDR")"

FACTORY_ID="$(manifest_contract_field market_factory id)"
if [ "$FORCE_REDEPLOY" != "true" ] && [ -n "$FACTORY_ID" ] && \
   stellar contract invoke --id "$FACTORY_ID" --source "$ADMIN_KEY" --network "$NETWORK" --send=no \
     -- predict_market --salt "$(python3 -c "print('0'*64)")" >/dev/null 2>&1; then
  echo "market_factory: reusing $FACTORY_ID"
else
  echo "market_factory: deploying fresh (stateless -- no initialize)"
  HASH="$(upload_wasm "$WASM_DIR/principal_market_factory.wasm" "$ADMIN_KEY")"
  FACTORY_ID="$(deploy_from_hash "$HASH" "$ADMIN_KEY")"
  read_last_tx
  manifest_log_tx "deploy:market_factory" "$LAST_TX"
  manifest_set_contract market_factory "$FACTORY_ID" "$WASM_DIR/principal_market_factory.wasm" "$HASH" "$LAST_TX" "" "$ADMIN_ADDR"
fi

# Oracle genesis: manual push (always -- PT/YT token initialize below requires a fresh value
# regardless of whether a SEP-40 feed is also configured) plus, optionally, the beyond-spec
# SEP-40 feed wiring described in OracleAdapter's own module doc.
if [ "$(raw "$(invoke "$ORACLE_ID" "$ADMIN_KEY" --send=no -- is_available)")" != "true" ]; then
  echo "oracle_adapter: submitting the manual genesis value ($ORACLE_INITIAL_VALUE @ $ORACLE_INITIAL_TIMESTAMP)"
  ZERO_SOURCE="$(python3 -c "print('0'*64)")"
  invoke "$ORACLE_ID" "$ADMIN_KEY" -- set_sources --caller "$ADMIN_ADDR" \
    --primary_source "$ZERO_SOURCE" --fallback_source null >/dev/null 2>&1 || true
  invoke "$ORACLE_ID" "$ADMIN_KEY" -- set_reference_value \
    --caller "$ADMIN_ADDR" --source "$ZERO_SOURCE" \
    --value "$ORACLE_INITIAL_VALUE" --timestamp "$ORACLE_INITIAL_TIMESTAMP" >/dev/null
  manifest_log_tx "oracle_adapter:set_reference_value" "$LAST_TX"
else
  echo "oracle_adapter: already has a fresh value, leaving the manual push alone"
fi
if [ -n "${ORACLE_FEED_ADDRESS:-}" ]; then
  echo "oracle_adapter: [beyond spec] wiring SEP-40 feed $ORACLE_FEED_ADDRESS"
  : "${ORACLE_FEED_ASSET_KIND:?set ORACLE_FEED_ASSET_KIND alongside ORACLE_FEED_ADDRESS}"
  : "${ORACLE_FEED_ASSET_VALUE:?set ORACLE_FEED_ASSET_VALUE alongside ORACLE_FEED_ADDRESS}"
  ASSET_JSON="{\"$ORACLE_FEED_ASSET_KIND\":\"$ORACLE_FEED_ASSET_VALUE\"}"
  invoke "$ORACLE_ID" "$ADMIN_KEY" -- set_feed_source \
    --caller "$ADMIN_ADDR" --feed "$ORACLE_FEED_ADDRESS" --asset "$ASSET_JSON" >/dev/null
  manifest_log_tx "oracle_adapter:set_feed_source" "$LAST_TX"
  invoke "$ORACLE_ID" "$ADMIN_KEY" -- sync_from_feed >/dev/null
  manifest_log_tx "oracle_adapter:sync_from_feed" "$LAST_TX"
fi

echo
echo "-- Market (via MarketFactory, maturity $MARKET_MATURITY_UNIX) --"

# principal_manager's own crate package name is already "principal_manager" (unlike every other
# per-market crate, named "principal_<thing>" with a bare "<thing>" directory) -- its wasm file is
# "principal_manager.wasm", not "principal_principal_manager.wasm" (found live: the naive
# "principal_${name}.wasm" pattern below is correct for every other name in this loop).
wasm_filename_for() { [ "$1" = principal_manager ] && echo "principal_manager.wasm" || echo "principal_${1}.wasm"; }

# Per-market idempotency (spec §5.4 -- "if initialization already happened ... resume/verify, not
# retry"): this section used to run unconditionally on every invocation, so a second run against
# an unchanged .env silently created a *second*, fully redundant market instead of reusing the
# first one's contracts -- found live on this script's own second test run. SYWrapper is this
# market's own entry point (the per-market contract an admin/compliance key can cheaply read
# without first knowing any of the other six addresses), so its liveness is the one check needed.
UNDERLYING_DECIMALS="${UNDERLYING_DECIMALS:-7}"
EXISTING_SY_ID="$(manifest_contract_field sy_wrapper id)"
if [ "$FORCE_REDEPLOY" != "true" ] && [ -n "$EXISTING_SY_ID" ] && is_alive "$EXISTING_SY_ID" underlying_address; then
  echo "market: reusing the market already in the manifest (sy_wrapper=$EXISTING_SY_ID) -- set FORCE_REDEPLOY=true to create another"
  SY_ADDR="$EXISTING_SY_ID"
  PT_ADDR="$(manifest_contract_field pt_token id)"
  YT_ADDR="$(manifest_contract_field yt_token id)"
  CONFIG_ADDR="$(manifest_contract_field market_config id)"
  MANAGER_ADDR="$(manifest_contract_field principal_manager id)"
  POOL_ADDR="$(manifest_contract_field market_pool id)"
  ESCROW_ADDR="$(manifest_contract_field recovery_escrow id)"
  # A reused market's maturity is fixed forever on-chain (PTToken/YTToken have no setter for it) --
  # MARKET_MATURITY_UNIX, if left blank in .env, defaults earlier to "now + 90 days" and would
  # otherwise overwrite the manifest with a new, wrong value on every single re-run. Read the
  # real one back from chain instead. Found live: the first idempotency re-run recorded a
  # different fictitious maturity than the market it was supposedly describing.
  MARKET_MATURITY_UNIX="$(raw "$(invoke "$PT_ADDR" "$ADMIN_KEY" --send=no -- maturity)")"
else
  for name in sy_wrapper pt_token yt_token market_config principal_manager market_pool recovery_escrow; do
    VAR="$(echo "$name" | tr '[:lower:]' '[:upper:]')_WASM"
    declare "$VAR=$(upload_wasm "$WASM_DIR/$(wasm_filename_for "$name")" "$ISSUER_KEY")"
  done

  MARKET_OUT="$(
    FACTORY_ID="$FACTORY_ID" ISSUER_KEY="$ISSUER_KEY" ADMIN_KEY="$ADMIN_KEY" \
    UNDERLYING_ID="$UNDERLYING_SAC" MATURITY_TS="$MARKET_MATURITY_UNIX" \
    PERMISSIONING_ID="$PERMISSIONING_ID" ORACLE_ID="$ORACLE_ID" RISK_CONTROL_ID="$RISK_CONTROL_ID" \
    ROUTER_ID="$ROUTER_ID" PROTOCOL_ADMIN_ADDR="$PROTOCOL_ADMIN_ADDRESS" TREASURY_ADDR="$TREASURY_ADDRESS" \
    SY_WASM="$SY_WRAPPER_WASM" PT_WASM="$PT_TOKEN_WASM" YT_WASM="$YT_TOKEN_WASM" \
    CONFIG_WASM="$MARKET_CONFIG_WASM" MANAGER_WASM="$PRINCIPAL_MANAGER_WASM" \
    POOL_WASM="$MARKET_POOL_WASM" ESCROW_WASM="$RECOVERY_ESCROW_WASM" \
    DECIMALS="$UNDERLYING_DECIMALS" \
    TOKENIZATION_FEE_BPS="${TOKENIZATION_FEE_BPS:-5}" YT_FEE_BPS="${YT_FEE_BPS:-1000}" \
    SWAP_FEE_TIER_BPS="${SWAP_FEE_TIER_BPS:-10}" PROTOCOL_SHARE_BPS="${PROTOCOL_SHARE_BPS:-2000}" \
    TIME_STRETCH_YEARS="${TIME_STRETCH_YEARS:-2}" NETWORK="$NETWORK" \
    ./scripts/create_market_via_factory.sh
  )"
  echo "$MARKET_OUT"

  SY_ADDR="$(echo "$MARKET_OUT" | grep -oE '^SY_ADDR=.*' | cut -d= -f2)"
  PT_ADDR="$(echo "$MARKET_OUT" | grep -oE '^PT_ADDR=.*' | cut -d= -f2)"
  YT_ADDR="$(echo "$MARKET_OUT" | grep -oE '^YT_ADDR=.*' | cut -d= -f2)"
  CONFIG_ADDR="$(echo "$MARKET_OUT" | grep -oE '^CONFIG_ADDR=.*' | cut -d= -f2)"
  MANAGER_ADDR="$(echo "$MARKET_OUT" | grep -oE '^MANAGER_ADDR=.*' | cut -d= -f2)"
  POOL_ADDR="$(echo "$MARKET_OUT" | grep -oE '^POOL_ADDR=.*' | cut -d= -f2)"
  ESCROW_ADDR="$(echo "$MARKET_OUT" | grep -oE '^ESCROW_ADDR=.*' | cut -d= -f2)"

  for name_addr in "sy_wrapper:$SY_ADDR" "pt_token:$PT_ADDR" "yt_token:$YT_ADDR" \
                   "market_config:$CONFIG_ADDR" "principal_manager:$MANAGER_ADDR" \
                   "market_pool:$POOL_ADDR" "recovery_escrow:$ESCROW_ADDR"; do
    n="${name_addr%%:*}"; a="${name_addr#*:}"
    wasm_var="$(echo "$n" | tr '[:lower:]' '[:upper:]')_WASM"
    manifest_set_contract "$n" "$a" "$WASM_DIR/$(wasm_filename_for "$n")" "${!wasm_var}" "" "" "$ISSUER_ADDR"
  done
fi

echo
echo "-- Compliance grants for the new market's contracts --"
sac() { invoke "$UNDERLYING_SAC" "$ISSUER_KEY" -- "$@"; }
perm() { invoke "$PERMISSIONING_ID" "$ADMIN_KEY" -- "$@"; }

for addr in "$SY_ADDR" "$MANAGER_ADDR" "$POOL_ADDR" "$ESCROW_ADDR" "$TREASURY_ADDRESS" ${EXTRA_COMPLIANCE_ACCOUNTS:-}; do
  # set_authorized on a classic G-account with no existing trustline to the underlying reverts
  # the SAC's own TrustlineMissingError (error #13) -- a contract address (C...) never needs
  # this (the SAC creates its balance entry lazily), but TREASURY_ADDRESS/EXTRA_COMPLIANCE_ACCOUNTS
  # frequently are G-accounts, and the trustline is that account's own `change_trust` operation --
  # this script cannot establish it on anyone else's behalf. Found live on the first real run
  # against a fresh AUTH_REQUIRED asset.
  if ! sac set_authorized --id "$addr" --authorize true >/dev/null; then
    if [[ "$addr" == G* ]]; then
      echo "ERROR: $addr has no trustline to $UNDERLYING_SAC's classic asset yet -- that account must" >&2
      echo "  run its own 'stellar tx new change-trust --source <key-for-$addr> --line <CODE>:<ISSUER_G_ADDR>'" >&2
      echo "  before this script's compliance grants can authorize it." >&2
    fi
    exit 1
  fi
  manifest_log_tx "compliance:set_authorized:$addr" "$LAST_TX"
  perm grant_account --caller "$ADMIN_ADDR" --account "$addr" >/dev/null
  manifest_log_tx "compliance:grant_account:$addr" "$LAST_TX"
done
# Anything that actually holds PT/YT (pool/escrow back LP positions and seized balances; the
# treasury/extra accounts may trade directly) needs the per-asset grant too, not just the
# account-level one -- found live in an earlier round (docs/TESTNET_DEPLOYMENT_EVIDENCE_2026_11_05.md
# §10.1): grant_account alone is not enough for anything that holds PT/YT.
for addr in "$POOL_ADDR" "$ESCROW_ADDR" "$TREASURY_ADDRESS" ${EXTRA_COMPLIANCE_ACCOUNTS:-}; do
  perm grant_asset --caller "$ADMIN_ADDR" --account "$addr" --asset "$PT_ADDR" >/dev/null
  manifest_log_tx "compliance:grant_asset_pt:$addr" "$LAST_TX"
  perm grant_asset --caller "$ADMIN_ADDR" --account "$addr" --asset "$YT_ADDR" >/dev/null
  manifest_log_tx "compliance:grant_asset_yt:$addr" "$LAST_TX"
done

manifest_set_path ".underlying.kind" "sac"
manifest_set_path ".underlying.sac" "$UNDERLYING_SAC"
manifest_set_path_num ".underlying.decimals" "$UNDERLYING_DECIMALS"
# Derived from SY_ADDR, not a wall-clock timestamp, so a reused market (see the per-market
# idempotency check above) keeps the same market_id on every re-run instead of a fresh one each
# time -- found live: the first idempotency test run produced two different market_ids for what
# was supposed to be the same, reused market.
MARKET_ID="${NETWORK}-${UNDERLYING_SAC:0:6}-$(( (MARKET_MATURITY_UNIX - $(date +%s)) / 86400 ))d-${SY_ADDR:0:8}"
manifest_set_path ".market.market_id" "$MARKET_ID"
manifest_set_path_num ".market.maturity" "$MARKET_MATURITY_UNIX"
manifest_set_path_num ".market.tokenization_fee_bps" "${TOKENIZATION_FEE_BPS:-5}"
manifest_set_path_num ".market.yt_fee_bps" "${YT_FEE_BPS:-1000}"
manifest_set_path_num ".market.swap_fee_tier_bps" "${SWAP_FEE_TIER_BPS:-10}"
manifest_set_path_num ".market.protocol_share_bps" "${PROTOCOL_SHARE_BPS:-2000}"
# Router.wrap_and_mint reports volume through both SYWrapper.deposit and PrincipalManager.mint --
# spec §5.3's RiskControl volume policy note, recorded here so dashboard capacity displays know
# to show both raw and effective capacity.
_tmp="$(mktemp)"
jq '.market.double_reported_wrap_and_mint_capacity = true' "$MANIFEST" >"$_tmp"
mv "$_tmp" "$MANIFEST"

_tmp="$(mktemp)"
jq --argjson registered true --arg pt_minter "$MANAGER_ADDR" --arg yt_minter "$MANAGER_ADDR" \
   --arg escrow "$ESCROW_ADDR" --argjson consumers "$(jq -n --arg a "$SY_ADDR" --arg b "$MANAGER_ADDR" '[$a,$b]')" \
  '.wiring = {router_registered_pool: $registered, pt_minter: $pt_minter, yt_minter: $yt_minter,
              recovery_escrow: $escrow, risk_consumers: $consumers}' "$MANIFEST" >"$_tmp"
mv "$_tmp" "$MANIFEST"

echo
echo "== deploy.sh done. market_id=$MARKET_ID. See $MANIFEST. Now run scripts/deploy/verify-deployment.sh. =="
