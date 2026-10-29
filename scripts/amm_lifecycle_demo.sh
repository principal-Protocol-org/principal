#!/usr/bin/env bash
# Drive one market through a full real lifecycle: deposit, mint, add liquidity, swap, settle,
# redeem. This is the exact sequence run live against Stellar Testnet on 29 October 2026 on a
# market created by MarketFactory (docs/MARKET_FACTORY.md), generalized to take any market's
# addresses as parameters.
#
# Each step prints the transaction it submitted (stellar CLI prints the tx hash and events on
# success). Run with USER_KEY already funded with the underlying asset, authorized, and granted
# Permissioning standing for this market's PT/YT (grant_user-equivalent -- see docs/DEPLOYMENT.md
# §11); this script does not do that for you, the same way it doesn't grant compliance standing to
# the contracts themselves (docs/MARKET_FACTORY.md explains why that's a deliberate, separate step).
#
# Usage:
#   SY_ID=... PT_ID=... YT_ID=... MANAGER_ID=... POOL_ID=... ORACLE_ID=... \
#   USER_KEY=my_test_user DEPOSIT_AMOUNT=1000000000 \
#   ./scripts/amm_lifecycle_demo.sh
#
# Optional: NETWORK (default testnet), LIQUIDITY_SY_AMOUNT (default: half of DEPOSIT_AMOUNT),
# SWAP_PT_AMOUNT (default: 50000000 = 5 units at 7 decimals), ORACLE_VALUE / ORACLE_CALLER to
# refresh the oracle before mint/settle (skip if a relay already keeps it fresh), DO_SETTLE=0 to
# stop before settlement (e.g. because maturity hasn't passed yet).
set -euo pipefail

: "${SY_ID:?set SY_ID}"
: "${PT_ID:?set PT_ID}"
: "${YT_ID:?set YT_ID}"
: "${MANAGER_ID:?set MANAGER_ID}"
: "${POOL_ID:?set POOL_ID}"
: "${USER_KEY:?set USER_KEY to a stellar CLI identity already funded, authorized and granted on this market}"
: "${DEPOSIT_AMOUNT:?set DEPOSIT_AMOUNT (raw underlying units, e.g. 1000000000 = 100 units at 7 decimals)}"

NETWORK="${NETWORK:-testnet}"
LIQUIDITY_SY_AMOUNT="${LIQUIDITY_SY_AMOUNT:-$((DEPOSIT_AMOUNT / 2))}"
SWAP_PT_AMOUNT="${SWAP_PT_AMOUNT:-50000000}"
DO_SETTLE="${DO_SETTLE:-1}"

USER_ADDR="$(stellar keys address "$USER_KEY")"
# A couple of these calls (observed live on swap_pt_for_sy, right after add_liquidity) occasionally
# need more resource-fee headroom than the CLI's own simulated estimate, failing at submission
# with InsufficientRefundableFee even though simulation itself succeeded -- this is the Soroban
# resource fee, not the classic network fee, so `--inclusion-fee` does nothing for it (tried that
# first; it still failed). `--resource-fee` is the flag that actually overrides it. 5,000,000
# stroops (0.5 XLM) was enough every time this was hit; override with RESOURCE_FEE if needed.
inv() {
  stellar contract invoke --id "$1" --source "$USER_KEY" --network "$NETWORK" \
    --resource-fee "${RESOURCE_FEE:-5000000}" -- "${@:2}"
}
# `stellar contract invoke` prints a bare i128/u64 scalar as a JSON string (e.g. "500000000",
# quotes included), since i128 doesn't fit JS's number type. Strip the quotes before reusing such
# a value as another CLI argument or in arithmetic, or the next call fails with "Expected type
# i128 ... but received: '"500000000"'" (hit live while first testing this script).
raw() { printf '%s' "$1" | tr -d '"'; }

if [ -n "${ORACLE_ID:-}" ] && [ -n "${ORACLE_CALLER:-}" ] && [ -n "${ORACLE_VALUE:-}" ]; then
  echo "-- Refreshing oracle (ORACLE_ID set) --"
  NOW="$(($(date +%s) - 2))"
  stellar contract invoke --id "$ORACLE_ID" --source "$USER_KEY" --network "$NETWORK" \
    -- set_reference_value --caller "$ORACLE_CALLER" --value "$ORACLE_VALUE" --timestamp "$NOW"
fi

echo "-- 1/5 sy.deposit ($DEPOSIT_AMOUNT) --"
SHARES="$(raw "$(inv "$SY_ID" deposit --from "$USER_ADDR" --amount "$DEPOSIT_AMOUNT" --min_shares_out 0)")"
echo "shares minted: $SHARES"

echo "-- 2/5 pm.mint --"
MINT_RESULT="$(inv "$MANAGER_ID" mint --from "$USER_ADDR" --sy_shares "$SHARES")"
echo "$MINT_RESULT"

echo "-- 3/5 sy.deposit (liquidity leg: $LIQUIDITY_SY_AMOUNT) + pool.add_liquidity --"
LIQ_SHARES="$(raw "$(inv "$SY_ID" deposit --from "$USER_ADDR" --amount "$LIQUIDITY_SY_AMOUNT" --min_shares_out 0)")"
PT_FOR_LIQUIDITY="$LIQ_SHARES"
inv "$POOL_ID" add_liquidity --from "$USER_ADDR" \
  --pt_desired "$PT_FOR_LIQUIDITY" --sy_desired "$((LIQ_SHARES * 8 / 10))" --min_lp_out 0

echo "-- 4/5 pool.swap_pt_for_sy ($SWAP_PT_AMOUNT) --"
inv "$POOL_ID" swap_pt_for_sy --from "$USER_ADDR" --to "$USER_ADDR" \
  --pt_in "$SWAP_PT_AMOUNT" --min_sy_out 0

if [ "$DO_SETTLE" = "1" ]; then
  echo "-- 5/5 pm.settle_all + pm.redeem (only valid after maturity) --"
  inv "$MANAGER_ID" settle_all
  REMAINING_PT="$(raw "$(inv "$PT_ID" balance --account "$USER_ADDR")")"
  REMAINING_YT="$(raw "$(inv "$YT_ID" balance --account "$USER_ADDR")")"
  echo "redeeming pt=$REMAINING_PT yt=$REMAINING_YT"
  inv "$MANAGER_ID" redeem --from "$USER_ADDR" --pt_amount "$REMAINING_PT" --yt_amount "$REMAINING_YT"
else
  echo "DO_SETTLE=0 -- stopping before settle_all/redeem (maturity presumably hasn't passed)."
fi

echo "== Lifecycle demo complete =="
