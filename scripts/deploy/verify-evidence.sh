#!/usr/bin/env bash
# Read-only deployment-evidence verifier. Unlike verify-deployment.sh (current contract identity,
# topology, and operational health against ONE manifest's own schema), this checks the evidence
# itself: does each recorded deploy_tx really create the contract id the manifest claims, is the
# on-chain wasm hash what the manifest says, and is the evidence actually complete. No blockchain
# writes anywhere in this script -- every call is a public read (Horizon/Soroban RPC history,
# `stellar contract fetch`).
#
# Supports two manifest formats, detected automatically:
#   - "tr2":      deployments/testnet/deployment-manifest.json's own schema (snake_case,
#                 contracts.<name>.{id,wasm_sha256,deploy_tx,wasm_path})
#   - "official":  deployments/testnet/official-dapp/deployment-manifest.json's schema
#                 (camelCase-ish, contracts.<name>.{id,wasm_sha256,deploy_tx})
# A manifest in neither shape is rejected rather than guessed at.
#
# Usage: ./scripts/deploy/verify-evidence.sh [path/to/manifest.json]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOLVER="$SCRIPT_DIR/lib/resolve-created-contract.py"

MANIFEST="${1:-deployments/testnet/deployment-manifest.json}"
[ -f "$MANIFEST" ] || { echo "no such manifest: $MANIFEST" >&2; exit 2; }

PASS_COUNT=0
FAIL_COUNT=0
WARN_COUNT=0
check() { # <label> <cmd...>
  local label="$1"; shift
  if "$@" >/tmp/verify-evidence.out 2>&1; then
    echo "PASS  $label"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL  $label"
    sed 's/^/      /' /tmp/verify-evidence.out
    FAIL_COUNT=$((FAIL_COUNT + 1))
  fi
}
warn() { echo "WARN  $1"; WARN_COUNT=$((WARN_COUNT + 1)); }

if jq -e '.contracts_source_commit' "$MANIFEST" >/dev/null 2>&1; then
  FORMAT=official
elif jq -e '.source_commit' "$MANIFEST" >/dev/null 2>&1; then
  FORMAT=tr2
else
  echo "cannot detect manifest format (neither .contracts_source_commit nor .source_commit present) -- refusing to guess" >&2
  exit 2
fi
echo "== Verifying evidence in $MANIFEST (detected format: $FORMAT) =="
echo

# --------------------------------------------------------------- 1. Historical deployment provenance
echo "-- 1. Historical deployment provenance (does each recorded deploy_tx really create this contract id?) --"
NAMES="$(jq -r '.contracts | keys[]' "$MANIFEST")"
TOTAL=0
WITH_DEPLOY_TX=0
for name in $NAMES; do
  TOTAL=$((TOTAL + 1))
  id="$(jq -r --arg n "$name" '.contracts[$n].id // empty' "$MANIFEST")"
  deploy_tx="$(jq -r --arg n "$name" '.contracts[$n].deploy_tx // empty' "$MANIFEST")"
  [ -z "$id" ] && continue
  if [ -z "$deploy_tx" ] || [ "$deploy_tx" = "null" ]; then
    warn "$name: no deploy_tx on record -- cannot verify provenance (evidence gap, not a failure of this check)"
    continue
  fi
  WITH_DEPLOY_TX=$((WITH_DEPLOY_TX + 1))
  check "$name: deploy_tx $deploy_tx creates $id" bash -c "
    resolved=\$(python3 '$RESOLVER' '$deploy_tx' 2>/dev/null)
    [ \"\$resolved\" = '$id' ]
  "
done
MARKET_CREATION_STEPS="$(jq -c '.evidence_reconciliation.market_creation_tx_verified // [] | .[]' "$MANIFEST" 2>/dev/null || true)"
if [ -n "$MARKET_CREATION_STEPS" ]; then
  # Process substitution, not a pipe: a pipe would run this loop in a subshell, silently
  # discarding every update to WITH_DEPLOY_TX/PASS_COUNT/FAIL_COUNT made inside it.
  while IFS= read -r step; do
    tx="$(echo "$step" | jq -r '.tx')"
    step_name="$(echo "$step" | jq -r '.step')"
    while IFS=$'\t' read -r cname caddr; do
      WITH_DEPLOY_TX=$((WITH_DEPLOY_TX + 1))
      check "$cname (via $step_name, tx $tx) creates $caddr" bash -c "
        resolved=\$(python3 '$RESOLVER' '$tx' 2>/dev/null)
        echo \"\$resolved\" | jq -e --arg c '$cname' --arg a '$caddr' 'if type==\"object\" then .[\$c]==\$a else .==\$a end' >/dev/null 2>&1 \
          || echo \"\$resolved\" | grep -q \"$caddr\"
      "
    done < <(echo "$step" | jq -r '.creates | to_entries[] | "\(.key)\t\(.value)"')
  done < <(echo "$MARKET_CREATION_STEPS")
fi
echo "   ($WITH_DEPLOY_TX of $TOTAL contracts have a deploy_tx or verified factory-creation mapping)"

# --------------------------------------------------------------- 2. Current on-chain identity/code
echo
echo "-- 2. Current on-chain code (does the live contract's wasm hash match the manifest?) --"
for name in $NAMES; do
  id="$(jq -r --arg n "$name" '.contracts[$n].id // empty' "$MANIFEST")"
  manifest_hash="$(jq -r --arg n "$name" '.contracts[$n].wasm_sha256 // empty' "$MANIFEST")"
  [ -z "$id" ] || [ -z "$manifest_hash" ] && continue
  [ "$manifest_hash" = "built-in (Stellar Asset Contract, not a Principal-source contract)" ] && {
    warn "$name: built-in SAC, no wasm hash to check"
    continue
  }
  check "$name on-chain code hash matches manifest ($id)" bash -c "
    f=\$(mktemp)
    stellar contract fetch --id '$id' --network testnet --out-file \"\$f\" >/dev/null 2>&1 || exit 1
    h=\$(sha256sum \"\$f\" | awk '{print \$1}')
    rm -f \"\$f\"
    [ \"\$h\" = '$manifest_hash' ]
  "
done

# --------------------------------------------------------------- 3. Evidence completeness
echo
echo "-- 3. Evidence completeness --"
if [ "$WITH_DEPLOY_TX" = "$TOTAL" ]; then
  echo "PASS  every contract has a deploy_tx or a verified factory-creation mapping on record"
  PASS_COUNT=$((PASS_COUNT + 1))
else
  echo "WARN  $((TOTAL - WITH_DEPLOY_TX)) of $TOTAL contract(s) still have neither a deploy_tx nor a factory-creation mapping on record (see 'historical deployment provenance' above for which)"
  WARN_COUNT=$((WARN_COUNT + 1))
fi

# --------------------------------------------------------------- 4. Source/build attestation
echo
echo "-- 4. Source/build attestation --"
if [ "$FORMAT" = official ]; then
  ATTESTED="$(jq -r '.source_to_byte_reproducibility.attempted // false' "$MANIFEST")"
  if [ "$ATTESTED" = "true" ]; then
    RESULT="$(jq -r '.source_to_byte_reproducibility.result // "unknown"' "$MANIFEST")"
    echo "PASS  source_to_byte_reproducibility attestation present: $RESULT"
    PASS_COUNT=$((PASS_COUNT + 1))
    echo "      (this is a recorded attestation, not re-run live by this script -- re-run the isolated"
    echo "       git-archive build yourself to reproduce it; see the closure report for the exact commands)"
  else
    warn "no source_to_byte_reproducibility attestation recorded in this manifest"
  fi
else
  warn "tr2 format manifest carries no source/build attestation field yet -- see docs/TRANCHE_2_DEPLOYMENT_EVIDENCE_CLOSURE.md for the ad hoc reproducibility result instead"
fi

echo
echo "== $PASS_COUNT passed, $FAIL_COUNT failed, $WARN_COUNT warned (evidence checks only -- this script makes no claim about current operational health; see verify-deployment.sh for that, TR2 only) =="
[ "$FAIL_COUNT" -eq 0 ]
