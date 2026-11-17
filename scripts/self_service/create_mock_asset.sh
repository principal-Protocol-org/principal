#!/usr/bin/env bash
# Fully self-service: a tester issues their own mock Stellar classic asset and deploys its SAC,
# with no Principal involvement and no admin key of ours required. Anyone can issue a classic
# Stellar asset and wrap it as a Soroban Asset Contract -- this script just automates the exact
# steps docs/MAINNET_DEPLOYMENT_EVIDENCE.md and docs/TESTNET_MOCKUSDY_DEPLOYMENT_EVIDENCE_2026_11_17.md
# did by hand for MOCKUSDY, for any tester to repeat for their own mock asset.
#
# The asset CODE is deliberately restricted to a recognized naming pattern (default: starts with
# MOCK or TEST) -- not because this script enforces anything on-chain (it can't; a tester
# controls their own issuer key and could run the underlying `stellar` commands directly with any
# code they want), but because scripts/self_service/registration_watcher.sh independently
# re-checks this same pattern against the real on-chain asset code before auto-registering any
# market built on it. A market for an asset that doesn't match is not rejected by this script,
# but it will not be auto-registered later -- it will sit pending for manual review. This script
# enforces the pattern up front purely so a tester gets that feedback immediately instead of
# finding out only after building a whole market.
#
# Usage:
#   ISSUER_KEY=my-test-issuer ASSET_CODE=MOCKFOO NETWORK=testnet \
#   ./scripts/self_service/create_mock_asset.sh
#
# ISSUER_KEY must already exist as a `stellar keys` identity (generate + fund it yourself first,
# e.g. `stellar keys generate my-test-issuer --network testnet --fund`). This script does not
# create or fund keys -- that is a tester's own responsibility, and on mainnet involves real
# money this script should never be trusted to move on anyone's behalf.
#
# Optional: ASSET_CODE_PATTERN (default '^(MOCK|TEST)[A-Z0-9]{0,8}$'), REVOCABLE (default true),
# CLAWBACK_ENABLED (default true) -- matching real USDY's own observed mainnet flags; set either
# to "false" to skip that flag if your test scenario specifically needs to exercise its absence.
set -euo pipefail

: "${ISSUER_KEY:?set ISSUER_KEY to a stellar CLI identity you control}"
: "${ASSET_CODE:?set ASSET_CODE, e.g. MOCKFOO (must match ASSET_CODE_PATTERN)}"
NETWORK="${NETWORK:-testnet}"
# Literal `{`/`}` inside a `${VAR:-default}` substitution confuses bash's own brace matching
# (the first bare `}` it meets closes the substitution, not the one actually balancing `{0,8}`)
# -- found live while building registration_watcher.sh's identical pattern default. Set the
# default as a separate statement instead of inside the substitution.
ASSET_CODE_PATTERN="${ASSET_CODE_PATTERN:-}"
[ -z "$ASSET_CODE_PATTERN" ] && ASSET_CODE_PATTERN='^(MOCK|TEST)[A-Z0-9]{0,8}$'
REVOCABLE="${REVOCABLE:-true}"
CLAWBACK_ENABLED="${CLAWBACK_ENABLED:-true}"

if ! [[ "$ASSET_CODE" =~ $ASSET_CODE_PATTERN ]]; then
  echo "ERROR: ASSET_CODE '$ASSET_CODE' does not match the recognized test-asset pattern ($ASSET_CODE_PATTERN)." >&2
  echo "  A market built on an asset that doesn't match will not be auto-registered by" >&2
  echo "  registration_watcher.sh later -- pick a code matching the pattern, or get it widened" >&2
  echo "  first if you have a real reason to need a different one." >&2
  exit 1
fi

ISSUER_ADDR="$(stellar keys address "$ISSUER_KEY")"
echo "== Creating $ASSET_CODE, issuer $ISSUER_ADDR, network $NETWORK =="

FLAGS=()
[ "$REVOCABLE" = "true" ] && FLAGS+=(--set-revocable)
[ "$CLAWBACK_ENABLED" = "true" ] && FLAGS+=(--set-clawback-enabled)
if [ "${#FLAGS[@]}" -gt 0 ]; then
  echo "-- setting issuer flags: ${FLAGS[*]} --"
  stellar tx new set-options --source "$ISSUER_KEY" "${FLAGS[@]}" --network "$NETWORK"
fi

echo "-- deploying SAC --"
SAC_ID="$(stellar contract asset deploy --asset "${ASSET_CODE}:${ISSUER_ADDR}" --source "$ISSUER_KEY" --network "$NETWORK" | tail -1)"

echo
echo "== Done =="
echo "ASSET_CODE=$ASSET_CODE"
echo "ISSUER_ADDR=$ISSUER_ADDR"
echo "SAC_ID=$SAC_ID"
echo
echo "Next: establish a trustline from any account that needs to hold $ASSET_CODE directly as a"
echo "classic asset (e.g. a treasury or test account you control) with:"
echo "  stellar tx new change-trust --source <that-account> --line \"${ASSET_CODE}:${ISSUER_ADDR}\" --network $NETWORK"
echo "Then create a market for it with scripts/create_market_via_factory.sh (SKIP_WIRING=1 -- you"
echo "do not have RiskControl/Router admin rights; see that script's own usage comment and"
echo "scripts/self_service/README.md), using UNDERLYING_ID=\$SAC_ID and ISSUER_KEY=$ISSUER_KEY."
