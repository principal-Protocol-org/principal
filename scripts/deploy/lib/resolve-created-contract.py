#!/usr/bin/env python3
"""Resolve a Soroban transaction's actual created-contract address, read-only.

`InvokeHostFunctionResult`'s success case in the classic transaction result is a SHA-256 hash of
an internal pre-image, not the return value itself -- decoding it directly (e.g. from Horizon's
or stellar.expert's result_xdr) does NOT give the created contract's address, and will silently
produce a plausible-looking but wrong 32-byte value. The real return value -- including, for a
CreateContract host function, the new contract's own address -- lives in the transaction's
Soroban metadata (`resultMetaXdr`), only available from Soroban RPC's `getTransaction`, decoded as
`TransactionMeta` and read from `.v4.soroban_meta.return_value` (or `.v3...` on an older ledger).

For a single-contract CreateContract call, return_value is `{"address": "C..."}` directly. For a
factory-style call that creates several contracts in one invocation (e.g. this project's
MarketFactory.create_tokens/create_engine/create_pool_and_escrow), return_value is instead a named
map of `{symbol: {"address": "C..."}}` entries -- this script prints that whole map as JSON in
that case, since there is no single address to return.

Usage: resolve-created-contract.py <tx-hash> [--rpc-url URL]
Prints the resolved address (or a JSON map, for a multi-contract-creating call) to stdout.
Exits non-zero, with nothing on stdout, if the transaction is not found, did not succeed, or its
return value isn't a contract address / map of contract addresses.
"""
import json
import subprocess
import sys
import urllib.request

DEFAULT_RPC = "https://soroban-testnet.stellar.org"


def get_transaction(rpc_url: str, tx_hash: str) -> dict:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": "getTransaction", "params": {"hash": tx_hash}}
    ).encode()
    req = urllib.request.Request(
        rpc_url,
        data=payload,
        headers={"Content-Type": "application/json", "User-Agent": "curl/8.0"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def decode_transaction_meta(meta_xdr: str) -> dict:
    proc = subprocess.run(
        ["stellar", "xdr", "decode", "--type", "TransactionMeta", "--output", "json"],
        input=meta_xdr,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"stellar xdr decode failed: {proc.stderr.strip()}")
    return json.loads(proc.stdout)


def extract_return_value(meta: dict):
    for version_key in ("v4", "v3"):
        try:
            return meta[version_key]["soroban_meta"]["return_value"]
        except (KeyError, TypeError):
            continue
    raise RuntimeError("no v3/v4 soroban_meta.return_value in this transaction's meta")


def main() -> int:
    args = sys.argv[1:]
    rpc_url = DEFAULT_RPC
    if "--rpc-url" in args:
        i = args.index("--rpc-url")
        rpc_url = args[i + 1]
        del args[i : i + 2]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    tx_hash = args[0]

    try:
        result = get_transaction(rpc_url, tx_hash)
    except Exception as exc:  # noqa: BLE001 -- surface any network/JSON error to the caller
        print(f"getTransaction failed: {exc}", file=sys.stderr)
        return 1

    res = result.get("result", {})
    if res.get("status") != "SUCCESS":
        print(f"transaction not found or not successful: status={res.get('status')}", file=sys.stderr)
        return 1

    try:
        meta = decode_transaction_meta(res["resultMetaXdr"])
        rv = extract_return_value(meta)
    except Exception as exc:  # noqa: BLE001
        print(f"could not extract return value: {exc}", file=sys.stderr)
        return 1

    if isinstance(rv, dict) and "address" in rv:
        print(rv["address"])
        return 0
    if isinstance(rv, dict) and "map" in rv:
        out = {}
        for entry in rv["map"]:
            key = entry.get("key", {}).get("symbol")
            val = entry.get("val", {})
            if key is not None and isinstance(val, dict) and "address" in val:
                out[key] = val["address"]
        if out:
            print(json.dumps(out))
            return 0

    print(f"return value is not a contract address or address map: {json.dumps(rv)[:200]}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
