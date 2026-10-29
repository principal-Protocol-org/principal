#!/usr/bin/env bash
# Create one new Principal Protocol market through MarketFactoryContract: three atomic
# transactions (create_tokens, create_engine, create_pool_and_escrow), then the two wiring
# calls and the compliance grants the factory deliberately leaves to the caller.
#
# This formalizes the exact sequence verified live on Stellar Testnet on 29 October 2026
# (docs/MARKET_FACTORY.md) into a reusable, parametrized script, mirroring
# scripts/create_market.sh's fully-manual flow but using the factory for the seven per-market
# contracts.
#
# Requires: stellar CLI, jq, and the wasm hashes of the seven per-market contracts already
# uploaded to the target network (run `stellar contract upload --wasm <file>` once per contract,
# or reuse hashes already on-chain if byte-identical code is already deployed there).
#
# Usage:
#   FACTORY_ID=... ISSUER_KEY=admin UNDERLYING_ID=... MATURITY_TS=$(date -d '+90 days' +%s) \
#   PERMISSIONING_ID=... ORACLE_ID=... RISK_CONTROL_ID=... ROUTER_ID=... \
#   PROTOCOL_ADMIN_ADDR=... TREASURY_ADDR=... \
#   SY_WASM=... PT_WASM=... YT_WASM=... CONFIG_WASM=... MANAGER_WASM=... POOL_WASM=... ESCROW_WASM=... \
#   ./scripts/create_market_via_factory.sh
#
# Optional: SALT (64 hex chars, default: random), PT_NAME, PT_SYMBOL, YT_NAME, YT_SYMBOL
# (defaults below), DECIMALS (default 7), TOKENIZATION_FEE_BPS (5), YT_FEE_BPS (1000),
# SWAP_FEE_TIER_BPS (10), PROTOCOL_SHARE_BPS (2000), TIME_STRETCH_YEARS (4), NETWORK (testnet),
# SKIP_WIRING=1 to stop after create_pool_and_escrow (e.g. when RiskControl/Router are owned by a
# different signer who must run their own add_consumer/register_market afterward).
# Set ORACLE_CALLER + ORACLE_VALUE (and ORACLE_ID) to refresh the oracle immediately before
# create_tokens, if this signer also controls it and a relay isn't already keeping it fresh --
# YTToken.initialize requires a reference value younger than 3600s and reverts OracleStale
# otherwise.
set -euo pipefail

: "${FACTORY_ID:?set FACTORY_ID to the deployed MarketFactoryContract id}"
: "${ISSUER_KEY:?set ISSUER_KEY to the stellar CLI identity for the live issuer authority of the underlying asset}"
: "${UNDERLYING_ID:?set UNDERLYING_ID}"
: "${MATURITY_TS:?set MATURITY_TS to a future Unix timestamp}"
: "${PERMISSIONING_ID:?set PERMISSIONING_ID}"
: "${ORACLE_ID:?set ORACLE_ID}"
: "${RISK_CONTROL_ID:?set RISK_CONTROL_ID}"
: "${PROTOCOL_ADMIN_ADDR:?set PROTOCOL_ADMIN_ADDR}"
: "${TREASURY_ADDR:?set TREASURY_ADDR}"
: "${SY_WASM:?set SY_WASM (sy_wrapper wasm hash)}"
: "${PT_WASM:?set PT_WASM (pt_token wasm hash)}"
: "${YT_WASM:?set YT_WASM (yt_token wasm hash)}"
: "${CONFIG_WASM:?set CONFIG_WASM (market_config wasm hash)}"
: "${MANAGER_WASM:?set MANAGER_WASM (principal_manager wasm hash)}"
: "${POOL_WASM:?set POOL_WASM (market_pool wasm hash)}"
: "${ESCROW_WASM:?set ESCROW_WASM (recovery_escrow wasm hash)}"

NETWORK="${NETWORK:-testnet}"
PT_NAME="${PT_NAME:-Principal Token}"
PT_SYMBOL="${PT_SYMBOL:-PT}"
YT_NAME="${YT_NAME:-Yield Token}"
YT_SYMBOL="${YT_SYMBOL:-YT}"
DECIMALS="${DECIMALS:-7}"
TOKENIZATION_FEE_BPS="${TOKENIZATION_FEE_BPS:-5}"
YT_FEE_BPS="${YT_FEE_BPS:-1000}"
SWAP_FEE_TIER_BPS="${SWAP_FEE_TIER_BPS:-10}"
PROTOCOL_SHARE_BPS="${PROTOCOL_SHARE_BPS:-2000}"
TIME_STRETCH_YEARS="${TIME_STRETCH_YEARS:-4}"
SALT="${SALT:-$(openssl rand -hex 32 2>/dev/null || printf '%064d' "$(date +%s%N)")}"

ISSUER_ADDR="$(stellar keys address "$ISSUER_KEY")"
NOW="$(date +%s)"
if [ "$MATURITY_TS" -le "$NOW" ]; then
  echo "ERROR: MATURITY_TS ($MATURITY_TS) is not in the future (now=$NOW)." >&2
  exit 1
fi

# Set RESOURCE_FEE (stroops) if a call fails with InsufficientRefundableFee -- an occasional gap
# between the CLI's simulated Soroban resource-fee estimate and what submission actually needs,
# seen live on scripts/amm_lifecycle_demo.sh's swap call against this same network (note:
# `--inclusion-fee` does NOT fix this -- that pads the classic network fee, not the Soroban
# resource fee; `--resource-fee` is the flag that overrides the simulated one). Unset by default
# since the three create_* calls ran clean without it when this script was last verified end to
# end.
RESOURCE_FEE_ARGS=()
if [ -n "${RESOURCE_FEE:-}" ]; then RESOURCE_FEE_ARGS=(--resource-fee "$RESOURCE_FEE"); fi
inv() { stellar contract invoke --id "$FACTORY_ID" --source "$ISSUER_KEY" --network "$NETWORK" "${RESOURCE_FEE_ARGS[@]}" -- "$@"; }
field() { python3 -c "import sys,json; print(json.load(sys.stdin)['$1'])"; }

echo "== MarketFactory $FACTORY_ID -- salt $SALT -- maturity $MATURITY_TS =="

# YTToken.initialize (inside create_tokens) reads the oracle's current reference value as its
# genesis rate and requires it to be fresh (< 3600s old) -- refresh it here if the caller also
# controls the oracle. If a relay already keeps it fresh, or this signer has no oracle write
# authority, leave ORACLE_CALLER/ORACLE_VALUE unset and ensure freshness yourself beforehand;
# create_tokens will revert OracleStale (YTToken error 10) otherwise.
if [ -n "${ORACLE_ID:-}" ] && [ -n "${ORACLE_CALLER:-}" ] && [ -n "${ORACLE_VALUE:-}" ]; then
  echo "-- Refreshing oracle before create_tokens --"
  stellar contract invoke --id "$ORACLE_ID" --source "$ISSUER_KEY" --network "$NETWORK" \
    -- set_reference_value --caller "$ORACLE_CALLER" --value "$ORACLE_VALUE" --timestamp "$(($(date +%s) - 2))"
fi

echo "-- predict_market (preview, no auth, no deploy) --"
PREDICTED="$(inv predict_market --salt "$SALT")"
echo "$PREDICTED"

echo "-- Phase 1/3: create_tokens --"
TOKENS="$(inv create_tokens \
  --issuer "$ISSUER_ADDR" --salt "$SALT" \
  --wasm "{\"sy_wrapper\":\"$SY_WASM\",\"pt_token\":\"$PT_WASM\",\"yt_token\":\"$YT_WASM\"}" \
  --params "{\"underlying\":\"$UNDERLYING_ID\",\"maturity\":$MATURITY_TS,\"permissioning\":\"$PERMISSIONING_ID\",\"oracle\":\"$ORACLE_ID\",\"pt_name\":\"$PT_NAME\",\"pt_symbol\":\"$PT_SYMBOL\",\"yt_name\":\"$YT_NAME\",\"yt_symbol\":\"$YT_SYMBOL\",\"decimals\":$DECIMALS}")"
echo "$TOKENS"
SY_ADDR="$(echo "$TOKENS" | field sy_wrapper)"
PT_ADDR="$(echo "$TOKENS" | field pt_token)"
YT_ADDR="$(echo "$TOKENS" | field yt_token)"

echo "-- Phase 2/3: create_engine --"
ENGINE="$(inv create_engine \
  --issuer "$ISSUER_ADDR" --salt "$SALT" \
  --wasm "{\"market_config\":\"$CONFIG_WASM\",\"principal_manager\":\"$MANAGER_WASM\"}" \
  --tokens "{\"sy_wrapper\":\"$SY_ADDR\",\"pt_token\":\"$PT_ADDR\",\"yt_token\":\"$YT_ADDR\"}" \
  --params "{\"underlying\":\"$UNDERLYING_ID\",\"maturity\":$MATURITY_TS,\"permissioning\":\"$PERMISSIONING_ID\",\"oracle\":\"$ORACLE_ID\",\"risk_control\":\"$RISK_CONTROL_ID\",\"protocol_admin\":\"$PROTOCOL_ADMIN_ADDR\",\"treasury\":\"$TREASURY_ADDR\",\"tokenization_fee_bps\":$TOKENIZATION_FEE_BPS,\"yt_fee_bps\":$YT_FEE_BPS,\"swap_fee_tier_bps\":$SWAP_FEE_TIER_BPS,\"protocol_share_bps\":$PROTOCOL_SHARE_BPS}")"
echo "$ENGINE"
CONFIG_ADDR="$(echo "$ENGINE" | field market_config)"
MANAGER_ADDR="$(echo "$ENGINE" | field principal_manager)"

echo "-- Phase 3/3: create_pool_and_escrow --"
POOLESC="$(inv create_pool_and_escrow \
  --issuer "$ISSUER_ADDR" --salt "$SALT" \
  --wasm "{\"market_pool\":\"$POOL_WASM\",\"recovery_escrow\":\"$ESCROW_WASM\"}" \
  --tokens "{\"sy_wrapper\":\"$SY_ADDR\",\"pt_token\":\"$PT_ADDR\",\"yt_token\":\"$YT_ADDR\"}" \
  --engine "{\"market_config\":\"$CONFIG_ADDR\",\"principal_manager\":\"$MANAGER_ADDR\"}" \
  --params "{\"underlying\":\"$UNDERLYING_ID\",\"time_stretch_years\":$TIME_STRETCH_YEARS}")"
echo "$POOLESC"
POOL_ADDR="$(echo "$POOLESC" | field market_pool)"
ESCROW_ADDR="$(echo "$POOLESC" | field recovery_escrow)"

if [ "${SKIP_WIRING:-0}" != "1" ]; then
  echo "-- Wiring: risk_control.add_consumer x2, router.register_market --"
  stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ISSUER_KEY" --network "$NETWORK" \
    -- add_consumer --caller "$ISSUER_ADDR" --consumer "$SY_ADDR"
  stellar contract invoke --id "$RISK_CONTROL_ID" --source "$ISSUER_KEY" --network "$NETWORK" \
    -- add_consumer --caller "$ISSUER_ADDR" --consumer "$MANAGER_ADDR"
  if [ -n "${ROUTER_ID:-}" ]; then
    stellar contract invoke --id "$ROUTER_ID" --source "$ISSUER_KEY" --network "$NETWORK" \
      -- register_market --caller "$ISSUER_ADDR" --pool "$POOL_ADDR"
  else
    echo "ROUTER_ID not set -- skipping router.register_market."
  fi
else
  echo "SKIP_WIRING=1 -- not calling risk_control.add_consumer or router.register_market."
  echo "A different signer (RiskControl/Router's own admin) must run those before the market can take deposits."
fi

cat <<EOF

== Market created ==
SALT=$SALT
SY_ADDR=$SY_ADDR
PT_ADDR=$PT_ADDR
YT_ADDR=$YT_ADDR
CONFIG_ADDR=$CONFIG_ADDR
MANAGER_ADDR=$MANAGER_ADDR
POOL_ADDR=$POOL_ADDR
ESCROW_ADDR=$ESCROW_ADDR

REMAINING STEP (not automated): grant compliance standing (underlying authorization +
Permissioning grant_account + grant_asset for PT/YT where applicable) to $SY_ADDR, $MANAGER_ADDR,
$POOL_ADDR, $ESCROW_ADDR, $TREASURY_ADDR, and the creator payee -- see docs/DEPLOYMENT.md §11 and
docs/MARKET_FACTORY.md. Nothing in this market can hold a balance until that is done.
EOF
