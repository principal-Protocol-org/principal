#!/usr/bin/env bash
# Read-only AMM pricing for one market pool: no signing key needed beyond a throwaway source
# account for simulation (view calls don't move funds). Useful for a dapp's quote fetcher or for
# manually checking a price before building a swap transaction.
#
# Usage:
#   ./scripts/quote_market.sh <market_pool_id> [source_identity] [sy_in_for_buy_quote] [pt_in_for_sell_quote]
set -euo pipefail

POOL_ID="${1:?market_pool contract id required}"
SOURCE="${2:-}"
SY_IN="${3:-0}"
PT_IN="${4:-0}"

NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"

if [ -z "$SOURCE" ]; then
  SOURCE="quote-probe-$$"
  stellar keys generate "$SOURCE" --no-fund >/dev/null 2>&1 || true
fi

inv() {
  stellar contract invoke --id "$POOL_ID" --source "$SOURCE" \
    --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" \
    --send=no -- "$@"
}

echo "== Pool $POOL_ID pricing =="
echo "pt_price (SY per PT, SCALE=1e7): $(inv pt_price)"
echo "implied_rate (annualized, SCALE=1e7): $(inv implied_rate)"
echo "time_exponent (WAD=1e18): $(inv time_exponent)"
echo "reserves (pt_reserve, sy_reserve): $(inv reserves)"
echo "maturity: $(inv maturity)"
echo "is_paused: $(inv is_paused)"

if [ "$SY_IN" != "0" ]; then
  echo
  echo "quote_sy_for_pt($SY_IN): $(inv quote_sy_for_pt --sy_in "$SY_IN")"
fi
if [ "$PT_IN" != "0" ]; then
  echo
  echo "quote_pt_for_sy($PT_IN): $(inv quote_pt_for_sy --pt_in "$PT_IN")"
fi
