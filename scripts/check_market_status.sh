#!/usr/bin/env bash
# Read-only check of whether a deployed Principal market is still open.
#
# A "market" here means one SYWrapper + PrincipalManager + MarketPool triple.
# This script never signs anything that moves funds: `maturity`/`is_paused` are
# plain view calls, and the swap/mint probes are run with --send=no, so they
# are simulated only and nothing is submitted to the network.
#
# Usage:
#   ./check_market_status.sh <market_pool_id> <principal_manager_id> [source_account]
#
# Example (the Tranche 1 Testnet "final market"):
#   ./check_market_status.sh \
#     CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR \
#     CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27 \
#     v11-carol
set -euo pipefail

POOL_ID="${1:?market_pool contract id required}"
MANAGER_ID="${2:?principal_manager contract id required}"
SOURCE="${3:-}"

NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"

if [ -z "$SOURCE" ]; then
  echo "No source account given — generating a throwaway identity for simulation."
  SOURCE="market-status-probe"
  stellar keys generate "$SOURCE" --no-fund >/dev/null 2>&1 || true
fi
PROBE_ADDR="$(stellar keys address "$SOURCE")"

invoke() {
  stellar contract invoke --id "$1" --source "$SOURCE" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" \
    --send=no -- "${@:2}"
}

echo "== Market status: pool=$POOL_ID manager=$MANAGER_ID =="
echo

NOW="$(date +%s)"
MATURITY="$(invoke "$POOL_ID" maturity)"
PAUSED="$(invoke "$POOL_ID" is_paused)"

echo "Current Unix time : $NOW"
echo "Pool maturity     : $MATURITY"
if [ "$NOW" -ge "$MATURITY" ]; then
  echo "Maturity status   : PAST (matured $(( (NOW - MATURITY) / 86400 )) day(s) ago)"
else
  echo "Maturity status   : ACTIVE ($(( (MATURITY - NOW) / 86400 )) day(s) remaining)"
fi
echo "Pool paused flag  : $PAUSED"
echo

echo "-- Live probe: PrincipalManager.mint (simulated, 1 unit, not submitted) --"
if invoke "$MANAGER_ID" mint --from "$PROBE_ADDR" --sy_shares 1 >/tmp/mint_probe.out 2>&1; then
  echo "RESULT: mint is OPEN (simulation succeeded)"
else
  echo "RESULT: mint is CLOSED or failing. Raw error:"
  grep -E "Error\(Contract" /tmp/mint_probe.out || cat /tmp/mint_probe.out
fi
echo

echo "-- Live probe: MarketPool.swap_pt_for_sy (simulated, 1 unit, not submitted) --"
if invoke "$POOL_ID" swap_pt_for_sy --from "$PROBE_ADDR" --to "$PROBE_ADDR" --pt_in 1 --min_sy_out 0 >/tmp/swap_probe.out 2>&1; then
  echo "RESULT: swap is OPEN (simulation succeeded)"
else
  echo "RESULT: swap is CLOSED or failing. Raw error:"
  grep -E "Error\(Contract" /tmp/swap_probe.out || cat /tmp/swap_probe.out
fi
echo
echo "Error code legend: MarketPool #6 = Expired. PrincipalManager #6 = AlreadyMature."
echo "Both mean the market's maturity has passed — this is expected, not a bug,"
echo "once 'now' >= maturity. It does NOT mean the chain, the contracts, or"
echo "SYWrapper deposits are broken: SYWrapper has no maturity gate of its own."
