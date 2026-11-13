#!/usr/bin/env bash
# Post-deploy verification for scripts/deploy/deploy.sh's output. Implements
# docs/TRANCHE_2_3_TECHNICAL_SPECIFICATION.md §5.6's checklist: asserts the expected initial
# state of everything in deployment-manifest.json, not just "did deploy.sh exit 0." Read-only
# throughout (every check is a --send=no simulation, a plain view call, or a local hash
# comparison); never signs or submits anything, so it is safe to run as often as wanted,
# including as a recurring health check against an already-live deployment.
#
# Usage: ./scripts/deploy/verify-deployment.sh [path/to/deployment-manifest.json]
# Exit code is nonzero if any check fails; a PASS/FAIL line is printed per check.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

MANIFEST="${1:-${MANIFEST:-deployments/testnet/deployment-manifest.json}}"
[ -f "$MANIFEST" ] || { echo "No manifest at $MANIFEST -- run scripts/deploy/deploy.sh first." >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq is required." >&2; exit 1; }

NETWORK="$(jq -r '.network' "$MANIFEST")"
PROBE_SOURCE="${PROBE_SOURCE:-}"
[ -z "$PROBE_SOURCE" ] && { PROBE_SOURCE="verify-deploy-probe"; stellar keys generate "$PROBE_SOURCE" --no-fund >/dev/null 2>&1 || true; }
export NETWORK PROBE_SOURCE # needed by view()/eq()/alive() once exported as functions, below

PASS_COUNT=0
FAIL_COUNT=0
FAILED=0

check() { # <description> <command...>
  local desc="$1"
  shift
  if "$@" >/tmp/verify_out.$$ 2>&1; then
    echo "PASS  $desc"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL  $desc"
    sed 's/^/      /' /tmp/verify_out.$$
    FAIL_COUNT=$((FAIL_COUNT + 1))
    FAILED=1
  fi
  rm -f /tmp/verify_out.$$
}

view() { # <contract_id> <fn> <args...>  -> raw stdout (quotes stripped)
  stellar contract invoke --id "$1" --source "$PROBE_SOURCE" --network "$NETWORK" --send=no \
    -- "${@:2}" 2>/dev/null | tr -d '"'
}

eq() { [ "$1" = "$2" ]; } # readable assertion helper for `check`

# Probe fn defaults to get_admin, but MarketConfig/RecoveryEscrow/MarketFactory have no admin of
# their own (RecoveryEscrow "has no admin of its own," docs/DEPLOYMENT.md; MarketConfig's
# admin-shaped actions are all protocol_admin/issuer-gated setters, not a getter; MarketFactory is
# fully stateless) -- get_admin on any of them isn't just "fails," it's not a real function, found
# live on the first run of this script (every one of those three read as dead/unresponsive).
alive() { local id="$1" fn="${2:-get_admin}"; [ -n "$id" ] && [ "$id" != "null" ] && view "$id" "$fn" >/dev/null 2>&1; }

# `check ... bash -c "...view...eq..."` below runs in a fresh, non-interactive bash that does not
# inherit this script's shell functions unless they are exported -- without this, `view` and `eq`
# both silently fell through to whatever binaries of the same name are on PATH (a `/usr/bin/view`
# symlink to vim's read-only mode, and nothing at all for `eq`), producing bogus FAILs with Vim's
# own startup banner as the "error output." Found live on the first run of this script.
export -f view eq alive

contract_id() { jq -r --arg n "$1" '.contracts[$n].id // empty' "$MANIFEST"; }
contract_hash() { jq -r --arg n "$1" '.contracts[$n].wasm_sha256 // empty' "$MANIFEST"; }

wasm_hash_matches_chain() { # <contract_id> <expected_hash>
  local id="$1" expected="$2" tmp
  tmp="$(mktemp)"
  stellar contract fetch --id "$id" --out-file "$tmp" --network "$NETWORK" >/dev/null 2>&1 || return 1
  local actual
  actual="$(sha256sum "$tmp" | awk '{print $1}')"
  rm -f "$tmp"
  [ "$actual" = "$expected" ]
}

wasm_hash_matches_local_build() { # <name>
  local name="$1" path
  path="$(jq -r --arg n "$name" '.contracts[$n].wasm_path // empty' "$MANIFEST")"
  [ -f "$path" ] || return 1
  [ "$(sha256sum "$path" | awk '{print $1}')" = "$(contract_hash "$name")" ]
}

echo "== Verifying $MANIFEST (network=$NETWORK) =="
echo

echo "-- git_tree_dirty (spec §4.4: must be false for an acceptance deployment) --"
DIRTY="$(jq -r '.git_tree_dirty' "$MANIFEST")"
if [ "$DIRTY" = "false" ]; then
  echo "PASS  git_tree_dirty is false"
  PASS_COUNT=$((PASS_COUNT + 1))
else
  echo "FAIL  git_tree_dirty is true -- not acceptance-ready (label and justify explicitly if this is intentional)"
  FAIL_COUNT=$((FAIL_COUNT + 1))
  FAILED=1
fi

echo
echo "-- Shared infrastructure --"
for name in oracle_adapter permissioning risk_control router market_factory; do
  ID="$(contract_id "$name")"
  [ -z "$ID" ] && { echo "SKIP  $name: not in manifest"; continue; }
  # market_factory is stateless and has no get_admin -- predict_market is a pure, no-auth preview
  # call that exists on every instance, so it doubles as a liveness probe here.
  if [ "$name" = market_factory ]; then
    check "$name ($ID) exists on $NETWORK and responds" bash -c \
      "view '$ID' predict_market --salt '0000000000000000000000000000000000000000000000000000000000000000' >/dev/null"
  else
    check "$name ($ID) exists on $NETWORK and responds" alive "$ID"
  fi
  HASH="$(contract_hash "$name")"
  if [ -n "$HASH" ]; then
    check "$name on-chain code hash matches manifest" wasm_hash_matches_chain "$ID" "$HASH"
    check "$name manifest hash matches local build artifact" wasm_hash_matches_local_build "$name"
  fi
done

ORACLE_ID="$(contract_id oracle_adapter)"
PERM_ID="$(contract_id permissioning)"
RISK_ID="$(contract_id risk_control)"
ROUTER_ID="$(contract_id router)"

if [ -n "$ORACLE_ID" ]; then
  check "oracle_adapter.is_available() == true" bash -c "eq \"\$(view '$ORACLE_ID' is_available)\" true"
  REF_VALUE="$(view "$ORACLE_ID" get_reference_value)"
  echo "      (get_reference_value() = $REF_VALUE -- compare by hand against .env's ORACLE_INITIAL_VALUE if this is a fresh deployment)"
fi
if [ -n "$PERM_ID" ]; then
  EXPECTED_ADMIN="$(jq -r --arg n permissioning '.contracts[$n].admin_address // empty' "$MANIFEST")"
  if [ -n "$EXPECTED_ADMIN" ]; then
    check "permissioning.get_admin() == manifest admin_address" bash -c \
      "eq \"\$(view '$PERM_ID' get_admin)\" '$EXPECTED_ADMIN'"
  fi
fi
if [ -n "$RISK_ID" ]; then
  check "risk_control responds to get_cb_limit" bash -c "view '$RISK_ID' get_cb_limit >/dev/null"
  check "risk_control responds to get_window_ledgers" bash -c "view '$RISK_ID' get_window_ledgers >/dev/null"
fi

echo
echo "-- Market topology (latest market in manifest) --"
MARKET_ID="$(jq -r '.market.market_id // empty' "$MANIFEST")"
if [ -z "$MARKET_ID" ]; then
  echo "SKIP  no market recorded yet"
else
  SY="$(contract_id sy_wrapper)"; PT="$(contract_id pt_token)"; YT="$(contract_id yt_token)"
  CONFIG="$(contract_id market_config)"; MANAGER="$(contract_id principal_manager)"
  POOL="$(contract_id market_pool)"; ESCROW="$(contract_id recovery_escrow)"
  MATURITY="$(jq -r '.market.maturity' "$MANIFEST")"
  echo "market_id=$MARKET_ID pool=$POOL maturity=$MATURITY"

  # Config and Escrow have no get_admin either (Config's admin-shaped calls are setters, not a
  # getter; Escrow deliberately has no admin of its own) -- probe each with a getter that
  # actually exists on it instead of the generic default.
  for n_a in "SY:$SY:get_admin" "PT:$PT:get_admin" "YT:$YT:get_admin" "Config:$CONFIG:underlying" \
             "Manager:$MANAGER:get_admin" "Pool:$POOL:get_admin" "Escrow:$ESCROW:underlying_address"; do
    label="${n_a%%:*}"; rest="${n_a#*:}"; addr="${rest%%:*}"; fn="${rest#*:}"
    check "$label ($addr) exists on $NETWORK and responds" alive "$addr" "$fn"
  done

  check "SYWrapper.underlying_address() matches manifest underlying" bash -c \
    "eq \"\$(view '$SY' underlying_address)\" '$(jq -r '.underlying.sac' "$MANIFEST")'"
  check "SYWrapper.permissioning_address() matches manifest permissioning" bash -c \
    "eq \"\$(view '$SY' permissioning_address)\" '$PERM_ID'"

  for tok_a in "PT:$PT" "YT:$YT"; do
    tok="${tok_a%%:*}"; addr="${tok_a#*:}"
    check "$tok.underlying_address() matches manifest underlying" bash -c \
      "eq \"\$(view '$addr' underlying_address)\" '$(jq -r '.underlying.sac' "$MANIFEST")'"
    check "$tok.permissioning_address() matches manifest permissioning" bash -c \
      "eq \"\$(view '$addr' permissioning_address)\" '$PERM_ID'"
    check "$tok.maturity() matches manifest maturity" bash -c \
      "eq \"\$(view '$addr' maturity)\" '$MATURITY'"
    check "$tok.minter() == PrincipalManager" bash -c "eq \"\$(view '$addr' minter)\" '$MANAGER'"
    check "$tok.recovery_escrow() matches manifest" bash -c \
      "eq \"\$(view '$addr' recovery_escrow)\" '$ESCROW'"
  done

  check "PrincipalManager topology: sy_wrapper_address" bash -c "eq \"\$(view '$MANAGER' sy_wrapper_address)\" '$SY'"
  check "PrincipalManager topology: pt_address" bash -c "eq \"\$(view '$MANAGER' pt_address)\" '$PT'"
  check "PrincipalManager topology: yt_address" bash -c "eq \"\$(view '$MANAGER' yt_address)\" '$YT'"
  check "PrincipalManager topology: oracle_address" bash -c "eq \"\$(view '$MANAGER' oracle_address)\" '$ORACLE_ID'"
  check "PrincipalManager topology: config_address" bash -c "eq \"\$(view '$MANAGER' config_address)\" '$CONFIG'"

  check "MarketPool topology: manager_address" bash -c "eq \"\$(view '$POOL' manager_address)\" '$MANAGER'"
  check "MarketPool topology: sy_address" bash -c "eq \"\$(view '$POOL' sy_address)\" '$SY'"
  check "MarketPool topology: pt_address" bash -c "eq \"\$(view '$POOL' pt_address)\" '$PT'"
  check "MarketPool topology: yt_address" bash -c "eq \"\$(view '$POOL' yt_address)\" '$YT'"
  check "MarketPool topology: config_address" bash -c "eq \"\$(view '$POOL' config_address)\" '$CONFIG'"
  check "MarketPool.maturity() matches manifest" bash -c "eq \"\$(view '$POOL' maturity)\" '$MATURITY'"
  check "MarketPool.recovery_escrow() matches manifest" bash -c "eq \"\$(view '$POOL' recovery_escrow)\" '$ESCROW'"

  check "RecoveryEscrow.underlying_address() matches manifest underlying" bash -c \
    "eq \"\$(view '$ESCROW' underlying_address)\" '$(jq -r '.underlying.sac' "$MANIFEST")'"

  if [ -n "$ROUTER_ID" ]; then
    check "Router has registered the pool (market().manager == PrincipalManager)" bash -c \
      "stellar contract invoke --id '$ROUTER_ID' --source '$PROBE_SOURCE' --network '$NETWORK' --send=no -- market --pool '$POOL' 2>/dev/null | grep -q '$MANAGER'"
  fi

  if [ -n "$RISK_ID" ]; then
    check "RiskControl.is_consumer(SYWrapper) == true" bash -c "eq \"\$(view '$RISK_ID' is_consumer --account '$SY')\" true"
    check "RiskControl.is_consumer(PrincipalManager) == true" bash -c "eq \"\$(view '$RISK_ID' is_consumer --account '$MANAGER')\" true"
  fi

  check "MarketPool is not paused" bash -c "eq \"\$(view '$POOL' is_paused)\" false"
  NOW="$(date +%s)"
  if [ "$NOW" -lt "$MATURITY" ]; then
    echo "PASS  market has not matured yet ($(( (MATURITY - NOW) / 86400 ))d remaining)"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL  market has already matured ($(( (NOW - MATURITY) / 86400 ))d ago)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
    FAILED=1
  fi

  if [ -n "$PERM_ID" ]; then
    for n_a in "SY:$SY" "Manager:$MANAGER" "Pool:$POOL" "Escrow:$ESCROW"; do
      label="${n_a%%:*}"; addr="${n_a#*:}"
      check "$label has Permissioning account-level standing" bash -c \
        "eq \"\$(view '$PERM_ID' is_allowed --account '$addr')\" true"
    done
    for n_a in "Pool:$POOL" "Escrow:$ESCROW"; do
      label="${n_a%%:*}"; addr="${n_a#*:}"
      check "$label has PT per-asset standing" bash -c \
        "eq \"\$(view '$PERM_ID' is_allowed_for_asset --account '$addr' --asset '$PT')\" true"
      check "$label has YT per-asset standing" bash -c \
        "eq \"\$(view '$PERM_ID' is_allowed_for_asset --account '$addr' --asset '$YT')\" true"
    done
  fi
fi

echo
echo "== $PASS_COUNT passed, $FAIL_COUNT failed =="
exit $FAILED
