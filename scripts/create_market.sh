#!/usr/bin/env bash
# Deploy and wire up one new Principal Protocol market.
#
# There is no on-chain factory contract in this codebase (verified: no contract
# calls env.deployer(); Router only *registers* pools that were already deployed
# elsewhere — see docs/DAPP_INTEGRATION_AND_MARKET_CREATION.md). This script is
# the off-chain equivalent: it runs the same 11-step sequence DEPLOYMENT.md
# documents by hand (identical to contracts/integration_tests/src/stack.rs),
# as one idempotent, parametrized job your backend can call.
#
# The caller's `--source` key MUST be the underlying asset's live issuer
# authority (for a classic/SEP-8 asset, the SAC's current admin() — for a
# SEP-57 RWAToken, an account holding the operator role). Every `initialize`
# re-derives and checks this on-chain; a market cannot be created over the
# issuer's objection no matter who runs this script.
#
# Reusable shared infrastructure (OracleAdapter, Permissioning, RiskControl)
# is NOT tied to any one market's maturity. Pass existing ids via
# ORACLE_ID / PERMISSIONING_ID / RISK_CONTROL_ID to reuse them across markets;
# leave them unset to deploy fresh ones. SYWrapper, by contrast, binds
# one-time to a RecoveryEscrow (set_recovery_escrow reverts if called twice),
# so a NEW SYWrapper is always deployed per market — never reuse one from an
# earlier market.
#
# Usage:
#   ISSUER_KEY=admin \
#   UNDERLYING_ID=CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L \
#   MATURITY_TS=1798761600 \
#   PROTOCOL_ADMIN_ADDR=G... TREASURY_ADDR=G... \
#   ./create_market.sh
#
# Optional overrides: ORACLE_ID, PERMISSIONING_ID, RISK_CONTROL_ID,
# ROUTER_ID (reuse an existing Router instead of deploying one),
# TOKENIZATION_FEE_BPS (default 5), YT_FEE_BPS (default 1000),
# SWAP_FEE_TIER_BPS (default 10), PROTOCOL_SHARE_BPS (default 2000),
# TIME_STRETCH_YEARS (default 4), WASM_DIR (default target/wasm32v1-none/release),
# ALIAS_PREFIX (default m_<unix time>), NETWORK (default testnet).
set -euo pipefail

: "${ISSUER_KEY:?set ISSUER_KEY to the stellar CLI identity for the underlying's issuer authority}"
: "${UNDERLYING_ID:?set UNDERLYING_ID to the underlying asset's contract id}"
: "${MATURITY_TS:?set MATURITY_TS to the market's maturity, a future Unix timestamp}"
: "${PROTOCOL_ADMIN_ADDR:?set PROTOCOL_ADMIN_ADDR (Principal's MarketConfig protocol_admin)}"
: "${TREASURY_ADDR:?set TREASURY_ADDR (fee recipient address, must hold compliance standing)}"

NETWORK="${NETWORK:-testnet}"
WASM_DIR="${WASM_DIR:-target/wasm32v1-none/release}"
ALIAS_PREFIX="${ALIAS_PREFIX:-m_$(date +%s)}"
TOKENIZATION_FEE_BPS="${TOKENIZATION_FEE_BPS:-5}"
YT_FEE_BPS="${YT_FEE_BPS:-1000}"
SWAP_FEE_TIER_BPS="${SWAP_FEE_TIER_BPS:-10}"
PROTOCOL_SHARE_BPS="${PROTOCOL_SHARE_BPS:-2000}"
TIME_STRETCH_YEARS="${TIME_STRETCH_YEARS:-4}"

ISSUER_ADDR="$(stellar keys address "$ISSUER_KEY")"
NOW="$(date +%s)"
if [ "$MATURITY_TS" -le "$NOW" ]; then
  echo "ERROR: MATURITY_TS ($MATURITY_TS) is not in the future (now=$NOW)." >&2
  exit 1
fi

inv() {
  local id="$1"; shift
  stellar contract invoke --id "$id" --source "$ISSUER_KEY" --network "$NETWORK" -- "$@"
}
alias_id() { stellar contract id alias "$1" --network "$NETWORK"; }

echo "== Market '$ALIAS_PREFIX' — issuer $ISSUER_ADDR — maturity $MATURITY_TS =="

# --- Steps 1-3: shared infrastructure (reused if already provided) ---------
if [ -z "${ORACLE_ID:-}" ]; then
  echo "-- Deploying OracleAdapter --"
  stellar contract deploy --wasm "$WASM_DIR/principal_oracle_adapter.wasm" \
    --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_oracle"
  inv "$(alias_id "${ALIAS_PREFIX}_oracle")" initialize --admin "$ISSUER_ADDR"
  inv "$(alias_id "${ALIAS_PREFIX}_oracle")" set_reference_value \
    --caller "$ISSUER_ADDR" --value 10000000 --timestamp "$(date +%s)"
  ORACLE_ID="$(alias_id "${ALIAS_PREFIX}_oracle")"
else
  echo "-- Reusing OracleAdapter $ORACLE_ID -- refreshing its reference value --"
  inv "$ORACLE_ID" set_reference_value --caller "$ISSUER_ADDR" --value 10000000 --timestamp "$(date +%s)"
fi

if [ -z "${PERMISSIONING_ID:-}" ]; then
  echo "-- Deploying Permissioning --"
  stellar contract deploy --wasm "$WASM_DIR/principal_permissioning.wasm" \
    --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_permissioning"
  inv "$(alias_id "${ALIAS_PREFIX}_permissioning")" initialize --admin "$ISSUER_ADDR"
  PERMISSIONING_ID="$(alias_id "${ALIAS_PREFIX}_permissioning")"
else
  echo "-- Reusing Permissioning $PERMISSIONING_ID --"
fi

if [ -z "${RISK_CONTROL_ID:-}" ]; then
  echo "-- Deploying RiskControl --"
  stellar contract deploy --wasm "$WASM_DIR/principal_risk_control.wasm" \
    --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_risk_control"
  inv "$(alias_id "${ALIAS_PREFIX}_risk_control")" initialize --admin "$ISSUER_ADDR" --cb-limit 0
  RISK_CONTROL_ID="$(alias_id "${ALIAS_PREFIX}_risk_control")"
else
  echo "-- Reusing RiskControl $RISK_CONTROL_ID --"
fi

# --- Step 4: SYWrapper (always fresh per market — see header note) ---------
echo "-- Deploying SYWrapper (fresh — never reused across markets) --"
stellar contract deploy --wasm "$WASM_DIR/principal_sy_wrapper.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_sy_wrapper"
SY_ID="$(alias_id "${ALIAS_PREFIX}_sy_wrapper")"
inv "$SY_ID" initialize --admin "$ISSUER_ADDR" --underlying "$UNDERLYING_ID" --permissioning "$PERMISSIONING_ID"
inv "$SY_ID" set_risk_control --admin "$ISSUER_ADDR" --risk_control "$RISK_CONTROL_ID"
inv "$RISK_CONTROL_ID" add_consumer --caller "$ISSUER_ADDR" --consumer "$SY_ID"

# --- Step 5: PTToken, YTToken -----------------------------------------------
echo "-- Deploying PTToken / YTToken --"
stellar contract deploy --wasm "$WASM_DIR/principal_pt_token.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_pt_token"
PT_ID="$(alias_id "${ALIAS_PREFIX}_pt_token")"
inv "$PT_ID" initialize --admin "$ISSUER_ADDR" --permissioning "$PERMISSIONING_ID" \
  --underlying "$UNDERLYING_ID" --maturity "$MATURITY_TS" \
  --name "Principal Token" --symbol "PT" --decimals 7

stellar contract deploy --wasm "$WASM_DIR/principal_yt_token.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_yt_token"
YT_ID="$(alias_id "${ALIAS_PREFIX}_yt_token")"
inv "$YT_ID" initialize --admin "$ISSUER_ADDR" --permissioning "$PERMISSIONING_ID" \
  --underlying "$UNDERLYING_ID" --oracle "$ORACLE_ID" --maturity "$MATURITY_TS" \
  --name "Yield Token" --symbol "YT" --decimals 7

# --- Step 6: MarketConfig ---------------------------------------------------
echo "-- Deploying MarketConfig --"
stellar contract deploy --wasm "$WASM_DIR/principal_market_config.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_market_config"
MC_ID="$(alias_id "${ALIAS_PREFIX}_market_config")"
inv "$MC_ID" initialize --admin "$ISSUER_ADDR" --underlying "$UNDERLYING_ID" --maturity "$MATURITY_TS" \
  --protocol_admin "$PROTOCOL_ADMIN_ADDR" --treasury "$TREASURY_ADDR" \
  --tokenization_fee_bps "$TOKENIZATION_FEE_BPS" --yt_fee_bps "$YT_FEE_BPS" \
  --swap_fee_tier_bps "$SWAP_FEE_TIER_BPS" --protocol_share_bps "$PROTOCOL_SHARE_BPS"

# --- Step 7: PrincipalManager ------------------------------------------------
echo "-- Deploying PrincipalManager --"
stellar contract deploy --wasm "$WASM_DIR/principal_manager.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_principal_manager"
PM_ID="$(alias_id "${ALIAS_PREFIX}_principal_manager")"
inv "$PM_ID" initialize --admin "$ISSUER_ADDR" --sy_wrapper "$SY_ID" --pt_token "$PT_ID" --yt_token "$YT_ID" \
  --oracle "$ORACLE_ID" --permissioning "$PERMISSIONING_ID" --underlying "$UNDERLYING_ID" \
  --maturity "$MATURITY_TS" --market_config "$MC_ID"

inv "$PT_ID" set_minter --admin "$ISSUER_ADDR" --minter "$PM_ID"
inv "$YT_ID" set_minter --admin "$ISSUER_ADDR" --minter "$PM_ID"
inv "$PM_ID" set_risk_control --admin "$ISSUER_ADDR" --risk_control "$RISK_CONTROL_ID"
inv "$RISK_CONTROL_ID" add_consumer --caller "$ISSUER_ADDR" --consumer "$PM_ID"

# --- Step 8: MarketPool ------------------------------------------------------
echo "-- Deploying MarketPool --"
stellar contract deploy --wasm "$WASM_DIR/principal_market_pool.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_market_pool"
POOL_ID="$(alias_id "${ALIAS_PREFIX}_market_pool")"
inv "$POOL_ID" initialize --admin "$ISSUER_ADDR" --principal_manager "$PM_ID" --time_stretch_years "$TIME_STRETCH_YEARS"

# --- Step 9: RecoveryEscrow --------------------------------------------------
echo "-- Deploying RecoveryEscrow --"
stellar contract deploy --wasm "$WASM_DIR/principal_recovery_escrow.wasm" \
  --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_recovery_escrow"
ESCROW_ID="$(alias_id "${ALIAS_PREFIX}_recovery_escrow")"
inv "$ESCROW_ID" initialize --underlying "$UNDERLYING_ID" --sy_wrapper "$SY_ID" --pt_token "$PT_ID" \
  --yt_token "$YT_ID" --principal_manager "$PM_ID" --market_pool "$POOL_ID"

for c in "$SY_ID" "$PT_ID" "$YT_ID" "$POOL_ID"; do
  inv "$c" set_recovery_escrow --admin "$ISSUER_ADDR" --escrow "$ESCROW_ID"
done

# --- Step 10: Router ----------------------------------------------------------
if [ -z "${ROUTER_ID:-}" ]; then
  echo "-- Deploying Router --"
  stellar contract deploy --wasm "$WASM_DIR/principal_router.wasm" \
    --source "$ISSUER_KEY" --network "$NETWORK" --alias "${ALIAS_PREFIX}_router"
  ROUTER_ID="$(alias_id "${ALIAS_PREFIX}_router")"
  inv "$ROUTER_ID" initialize --admin "$ISSUER_ADDR"
else
  echo "-- Reusing Router $ROUTER_ID --"
fi
inv "$ROUTER_ID" register_market --caller "$ISSUER_ADDR" --pool "$POOL_ID"

echo
echo "== Market deployed. Addresses: =="
cat <<EOF
UNDERLYING_ID=$UNDERLYING_ID
ORACLE_ID=$ORACLE_ID
PERMISSIONING_ID=$PERMISSIONING_ID
RISK_CONTROL_ID=$RISK_CONTROL_ID
SY_ID=$SY_ID
PT_ID=$PT_ID
YT_ID=$YT_ID
MARKET_CONFIG_ID=$MC_ID
PRINCIPAL_MANAGER_ID=$PM_ID
MARKET_POOL_ID=$POOL_ID
RECOVERY_ESCROW_ID=$ESCROW_ID
ROUTER_ID=$ROUTER_ID
EOF
echo
echo "REMAINING STEP (not yet done by this script — step 11 of DEPLOYMENT.md):"
echo "grant compliance standing (underlying authorization + Permissioning"
echo "grant_account + grant_asset for PT and YT) to: $SY_ID, $PM_ID, $POOL_ID,"
echo "$ESCROW_ID, $TREASURY_ADDR, and the market creator's payee address."
echo "Nothing in this market can hold a balance until that is done."
