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

# A unique scratch dir per invocation (PID + RANDOM), not a fixed /tmp path -- two concurrent runs
# of this script (e.g. one per manifest) used to share a single hardcoded /tmp/verify-evidence.out
# and could corrupt each other's captured output. Cleaned up on exit either way.
SCRATCH="$(mktemp -d "/tmp/verify-evidence.$$.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

PASS_COUNT=0
FAIL_COUNT=0
WARN_COUNT=0
check() { # <label> <cmd...>
  local label="$1"; shift
  local out="$SCRATCH/$(echo "$label" | tr -c 'a-zA-Z0-9' '_' | cut -c1-80).$RANDOM.out"
  if "$@" >"$out" 2>&1; then
    echo "PASS  $label"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL  $label"
    sed 's/^/      /' "$out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
  fi
}
warn() { echo "WARN  $1"; WARN_COUNT=$((WARN_COUNT + 1)); }

# Runs the resolver with --expect-kind create_contract (+ --expect-wasm if given) and succeeds
# only if its resolved address equals $1. A real function, not an inline bash -c string, so
# argument passing (notably an optional --expect-wasm pair) doesn't need fragile array-in-string
# quoting.
resolves_to_create_contract() { # <expected_id> <tx> [wasm_hash]
  local expected="$1" tx="$2" wasm_hash="${3:-}"
  local args=(python3 "$RESOLVER" "$tx" --expect-kind create_contract)
  [ -n "$wasm_hash" ] && args+=(--expect-wasm "$wasm_hash")
  local resolved
  resolved="$("${args[@]}" 2>&1)" || return 1
  [ "$resolved" = "$expected" ]
}

# Runs the resolver with --expect-kind invoke_contract against the known factory, and succeeds
# only if the resolved map's entry for $2 (the contract name) equals $1 (its canonical id).
resolves_to_factory_creation() { # <expected_id> <contract_name> <tx> <factory_id>
  local expected="$1" cname="$2" tx="$3" factory_id="$4"
  local resolved
  resolved="$(python3 "$RESOLVER" "$tx" --expect-kind invoke_contract --expect-contract "$factory_id" \
    --expect-function create_tokens,create_engine,create_pool_and_escrow 2>&1)" || return 1
  echo "$resolved" | jq -e --arg c "$cname" --arg a "$expected" '.[$c]==$a' >/dev/null 2>&1
}

# A built-in Stellar Asset Contract's creation is a different shape entirely: no custom wasm, and
# often (as here) old enough to have fallen out of Soroban RPC's getTransaction retention window
# -- resolve-created-contract.py's RPC-based method cannot reach it at all. Its address is instead
# a deterministic function of (network passphrase, asset), so this checks it two ways, neither of
# which needs RPC: Horizon's own envelope_xdr (full history, no retention window) to confirm the
# real operation is create_contract with executable==stellar_asset and the claimed asset, and a
# pure local recomputation of what that asset's SAC address must be.
resolves_to_sac_creation() { # <expected_id> <tx> <asset_code> <asset_issuer>
  local expected="$1" tx="$2" asset_code="$3" asset_issuer="$4"
  local env_xdr op
  env_xdr="$(curl -sf "https://horizon-testnet.stellar.org/transactions/$tx" | jq -r '.envelope_xdr')" || return 1
  [ -n "$env_xdr" ] && [ "$env_xdr" != "null" ] || return 1
  op="$(echo "$env_xdr" | stellar xdr decode --type TransactionEnvelope --output json 2>/dev/null \
    | jq -c '.tx.tx.operations[0].body.invoke_host_function.host_function.create_contract // empty')" || return 1
  [ -n "$op" ] || return 1
  echo "$op" | jq -e '.executable == "stellar_asset"' >/dev/null 2>&1 || return 1
  echo "$op" | jq -e --arg c "$asset_code" --arg i "$asset_issuer" \
    '.contract_id_preimage.asset.credit_alphanum4.asset_code == $c and .contract_id_preimage.asset.credit_alphanum4.issuer == $i' \
    >/dev/null 2>&1 || return 1
  local recomputed
  recomputed="$(stellar contract id asset --asset "$asset_code:$asset_issuer" --network testnet 2>/dev/null)" || return 1
  [ "$recomputed" = "$expected" ]
}

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
echo "   (verified against the transaction's own decoded envelope operation, not merely a returned"
echo "   address -- see resolve-created-contract.py's own doc for why that distinction matters)"
NAMES="$(jq -r '.contracts | keys[]' "$MANIFEST")"
TOTAL=0
# Associative array: each contract name is marked covered at most once, by exactly one source
# (a direct deploy_tx, or a verified factory mapping) -- so a contract accidentally described by
# both, or by neither, is caught instead of silently double-counted into a false "100% covered".
declare -A COVERED_BY
for name in $NAMES; do
  TOTAL=$((TOTAL + 1))
  COVERED_BY["$name"]=""
  id="$(jq -r --arg n "$name" '.contracts[$n].id // empty' "$MANIFEST")"
  deploy_tx="$(jq -r --arg n "$name" '.contracts[$n].deploy_tx // empty' "$MANIFEST")"
  wasm_hash="$(jq -r --arg n "$name" '.contracts[$n].wasm_sha256 // empty' "$MANIFEST")"
  [ -z "$id" ] && continue
  if [ -z "$deploy_tx" ] || [ "$deploy_tx" = "null" ]; then
    continue
  fi
  if [ "$wasm_hash" = "built-in (Stellar Asset Contract, not a Principal-source contract)" ]; then
    asset_code="$(jq -r --arg n "$name" '.contracts[$n].asset_code // empty' "$MANIFEST")"
    asset_issuer="$(jq -r --arg n "$name" '.contracts[$n].asset_issuer // empty' "$MANIFEST")"
    if [ -n "$asset_code" ] && [ -n "$asset_issuer" ]; then
      check "$name: deploy_tx $deploy_tx creates $id (built-in SAC for $asset_code)" \
        resolves_to_sac_creation "$id" "$deploy_tx" "$asset_code" "$asset_issuer"
      COVERED_BY["$name"]="deploy_tx"
    else
      warn "$name: built-in SAC with a deploy_tx on record, but no asset_code/asset_issuer to verify it against -- cannot check"
    fi
    continue
  fi
  check "$name: deploy_tx $deploy_tx creates $id" resolves_to_create_contract "$id" "$deploy_tx" "$wasm_hash"
  COVERED_BY["$name"]="deploy_tx"
done

MARKET_CREATION_STEPS="$(jq -c '.evidence_reconciliation.market_creation_tx_verified // [] | .[]' "$MANIFEST" 2>/dev/null || true)"
if [ -n "$MARKET_CREATION_STEPS" ]; then
  FACTORY_ID="$(jq -r '.contracts.market_factory.id // empty' "$MANIFEST")"
  # Process substitution, not a pipe: a pipe would run this loop in a subshell, silently
  # discarding every update to COVERED_BY/PASS_COUNT/FAIL_COUNT made inside it.
  while IFS= read -r step; do
    tx="$(echo "$step" | jq -r '.tx')"
    step_name="$(echo "$step" | jq -r '.step')"
    while IFS=$'\t' read -r cname caddr; do
      # Cross-check against the manifest's OWN canonical contracts.<cname>.id, not just against
      # the evidence_reconciliation block's own (possibly independently wrong) copy of the
      # address -- a factory mapping that only agrees with itself is not proof of anything.
      canonical_id="$(jq -r --arg n "$cname" '.contracts[$n].id // empty' "$MANIFEST")"
      if [ -z "$canonical_id" ]; then
        echo "FAIL  $cname (via $step_name): no such contract in this manifest's .contracts -- cannot cross-check"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        continue
      fi
      if [ "$canonical_id" != "$caddr" ]; then
        echo "FAIL  $cname (via $step_name): evidence_reconciliation claims $caddr, but .contracts.$cname.id is $canonical_id -- mismatch"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        continue
      fi
      check "$cname (via $step_name, tx $tx) creates $canonical_id (matches .contracts.$cname.id)" \
        resolves_to_factory_creation "$canonical_id" "$cname" "$tx" "$FACTORY_ID"
      if [ -n "${COVERED_BY[$cname]:-}" ]; then
        echo "FAIL  $cname is covered by BOTH a direct deploy_tx and a factory mapping -- should be exactly one"
        FAIL_COUNT=$((FAIL_COUNT + 1))
      fi
      COVERED_BY["$cname"]="factory:$step_name"
    done < <(echo "$step" | jq -r '.creates | to_entries[] | "\(.key)\t\(.value)"')
  done < <(echo "$MARKET_CREATION_STEPS")
fi

WITH_DEPLOY_TX=0
UNCOVERED=()
for name in $NAMES; do
  if [ -n "${COVERED_BY[$name]:-}" ]; then
    WITH_DEPLOY_TX=$((WITH_DEPLOY_TX + 1))
  else
    UNCOVERED+=("$name")
  fi
done
for name in "${UNCOVERED[@]:-}"; do
  [ -z "$name" ] && continue
  warn "$name: no deploy_tx and no factory-creation mapping on record -- cannot verify provenance (evidence gap, not a failure of this check)"
done
echo "   ($WITH_DEPLOY_TX of $TOTAL contracts have exactly one deploy_tx or verified factory-creation mapping)"

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
if [ "$WITH_DEPLOY_TX" = "$TOTAL" ] && [ "$FAIL_COUNT" -eq 0 ]; then
  echo "PASS  every contract has exactly one deploy_tx or a verified factory-creation mapping on record"
  PASS_COUNT=$((PASS_COUNT + 1))
else
  echo "WARN  $((TOTAL - WITH_DEPLOY_TX)) of $TOTAL contract(s) still have neither a deploy_tx nor a factory-creation mapping on record (see 'historical deployment provenance' above for which)"
  WARN_COUNT=$((WARN_COUNT + 1))
fi

# --------------------------------------------------------------- 3b. Snapshot/source attribution
if [ "$FORMAT" = official ]; then
  echo
  echo "-- 3b. Snapshot checksum and source attribution --"
  SNAPSHOT_FILE="$(jq -r '.source.snapshot_file // empty' "$MANIFEST")"
  CLAIMED_SHA="$(jq -r '.source.snapshot_sha256 // empty' "$MANIFEST")"
  MANIFEST_DIR="$(dirname "$MANIFEST")"
  if [ -n "$SNAPSHOT_FILE" ] && [ -n "$CLAIMED_SHA" ]; then
    check "snapshot $SNAPSHOT_FILE sha256 matches claimed $CLAIMED_SHA" bash -c "
      [ -f '$MANIFEST_DIR/$SNAPSHOT_FILE' ] || exit 1
      actual=\$(sha256sum '$MANIFEST_DIR/$SNAPSHOT_FILE' | awk '{print \$1}')
      [ \"\$actual\" = '$CLAIMED_SHA' ]
    "
  else
    warn "no snapshot_file/snapshot_sha256 claimed in .source -- nothing to check"
  fi
fi

# --------------------------------------------------------------- 4. Source/build attestation
echo
echo "-- 4. Source/build attestation --"
echo "   ('attestation present' and 'source reproduction successful' are reported separately --"
echo "   an attempted-and-failed rebuild is never presented as a pass here)"
if [ "$FORMAT" = official ]; then
  ATTESTED="$(jq -r '.source_to_byte_reproducibility.attempted // false' "$MANIFEST")"
  if [ "$ATTESTED" = "true" ]; then
    RESULT="$(jq -r '.source_to_byte_reproducibility.result // "unknown"' "$MANIFEST")"
    echo "PASS  source_to_byte_reproducibility attestation present: $RESULT"
    PASS_COUNT=$((PASS_COUNT + 1))
    if echo "$RESULT" | grep -qE '^0 of [0-9]+ byte-identical'; then
      warn "source reproduction attempt recorded but UNSUCCESSFUL ($RESULT) -- this is an attestation, not a successful rebuild; on-chain/manifest hash agreement (section 2 above) is a separate, unaffected claim"
    elif echo "$RESULT" | grep -qE '^([0-9]+) of \1 byte-identical'; then
      echo "PASS  source reproduction attempt recorded as fully successful: $RESULT"
      PASS_COUNT=$((PASS_COUNT + 1))
    else
      warn "source reproduction result is neither a full success nor a recognized 0-of-N failure phrasing -- read it directly: $RESULT"
    fi
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
