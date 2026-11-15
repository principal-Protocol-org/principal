#!/usr/bin/env python3
"""Resolve a Soroban transaction's actual created-contract address, read-only.

`InvokeHostFunctionResult`'s success case in the classic transaction result is a SHA-256 hash of
an internal pre-image, not the return value itself -- decoding it directly (e.g. from Horizon's
or stellar.expert's result_xdr) does NOT give the created contract's address, and will silently
produce a plausible-looking but wrong 32-byte value. The real return value -- including, for a
CreateContract host function, the new contract's own address -- lives in the transaction's
Soroban metadata (`resultMetaXdr`), only available from Soroban RPC's `getTransaction`, decoded as
`TransactionMeta` and read from `.v4.soroban_meta.return_value` (or `.v3...` on an older ledger).

A decoded return value that merely LOOKS like a contract address (or a map of them) is not proof
that this transaction created that contract: any successful call can return an address-shaped
value for unrelated reasons (a getter, a prediction/simulation-style view, a cross-contract call
that happens to hand back something stored elsewhere). To actually prove creation, this script
also decodes the transaction's `envelopeXdr` and checks what operation was really submitted:

  - "create_contract": the operation's own host function is `invoke_host_function.host_function
    .create_contract` -- a direct CreateContract call. This is the only shape for which a bare
    returned address proves creation on its own.
  - "invoke_contract": the operation is `invoke_host_function.host_function.invoke_contract`,
    calling a named function on a known contract (e.g. this project's own MarketFactory calls,
    which create several child contracts in one transaction and return a map of their addresses).
    A returned map alone is not enough here either: the caller must also confirm the invoked
    contract address and function name are the ones actually expected to create something, via
    --expect-kind invoke_contract --expect-contract/--expect-function below -- this script will
    not do that inference on its own.

Usage:
  resolve-created-contract.py <tx-hash> --expect-kind create_contract [--expect-wasm <sha256>] [--rpc-url URL]
  resolve-created-contract.py <tx-hash> --expect-kind invoke_contract --expect-contract <C...> --expect-function <name>[,<name>...] [--rpc-url URL]

Prints the resolved address (or a JSON map, for a multi-contract-creating call) to stdout.
Exits non-zero, with nothing on stdout, if the transaction is not found, did not succeed, its
envelope's real operation does not match --expect-kind/--expect-contract/--expect-function, its
create_contract executable wasm does not match --expect-wasm (when given), or its return value
isn't a contract address / map of contract addresses.
"""
import json
import subprocess
import sys
import urllib.request

DEFAULT_RPC = "https://soroban-testnet.stellar.org"


def rpc_call(rpc_url: str, method: str, params: dict) -> dict:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(
        rpc_url,
        data=payload,
        headers={"Content-Type": "application/json", "User-Agent": "curl/8.0"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def decode_xdr(xdr_type: str, value: str) -> dict:
    proc = subprocess.run(
        ["stellar", "xdr", "decode", "--type", xdr_type, "--output", "json"],
        input=value,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"stellar xdr decode --type {xdr_type} failed: {proc.stderr.strip()}")
    return json.loads(proc.stdout)


def extract_return_value(meta: dict):
    for version_key in ("v4", "v3"):
        try:
            return meta[version_key]["soroban_meta"]["return_value"]
        except (KeyError, TypeError):
            continue
    raise RuntimeError("no v3/v4 soroban_meta.return_value in this transaction's meta")


def extract_host_function(envelope: dict) -> dict:
    """Returns the single operation's invoke_host_function.host_function body. Refuses a
    multi-operation transaction outright rather than guessing which operation matters -- every
    deployment/factory call this project makes is a single-operation transaction, and a
    multi-operation one is exactly the kind of transaction this script should not be trusted to
    interpret automatically."""
    ops = envelope["tx"]["tx"]["operations"]
    if len(ops) != 1:
        raise RuntimeError(f"expected exactly 1 operation, found {len(ops)} -- refusing to guess which one")
    body = ops[0]["body"]
    if "invoke_host_function" not in body:
        raise RuntimeError(f"operation is not invoke_host_function (got: {list(body.keys())})")
    return body["invoke_host_function"]["host_function"]


def main() -> int:
    args = sys.argv[1:]
    rpc_url = DEFAULT_RPC
    expect_kind = None
    expect_wasm = None
    expect_contract = None
    expect_functions = None
    for flag, var in (
        ("--rpc-url", "rpc_url"),
        ("--expect-kind", "expect_kind"),
        ("--expect-wasm", "expect_wasm"),
        ("--expect-contract", "expect_contract"),
        ("--expect-function", "expect_functions"),
    ):
        if flag in args:
            i = args.index(flag)
            val = args[i + 1]
            del args[i : i + 2]
            if var == "rpc_url":
                rpc_url = val
            elif var == "expect_kind":
                expect_kind = val
            elif var == "expect_wasm":
                expect_wasm = val
            elif var == "expect_contract":
                expect_contract = val
            elif var == "expect_functions":
                expect_functions = set(val.split(","))
    if len(args) != 1 or expect_kind not in ("create_contract", "invoke_contract"):
        print(__doc__, file=sys.stderr)
        return 2
    if expect_kind == "invoke_contract" and (not expect_contract or not expect_functions):
        print("--expect-kind invoke_contract requires --expect-contract and --expect-function", file=sys.stderr)
        return 2
    tx_hash = args[0]

    try:
        result = rpc_call(rpc_url, "getTransaction", {"hash": tx_hash}).get("result", {})
    except Exception as exc:  # noqa: BLE001 -- surface any network/JSON error to the caller
        print(f"getTransaction failed: {exc}", file=sys.stderr)
        return 1

    if result.get("status") != "SUCCESS":
        print(f"transaction not found or not successful: status={result.get('status')}", file=sys.stderr)
        return 1

    try:
        host_function = extract_host_function(decode_xdr("TransactionEnvelope", result["envelopeXdr"]))
    except Exception as exc:  # noqa: BLE001
        print(f"could not inspect envelope's real operation: {exc}", file=sys.stderr)
        return 1

    if expect_kind == "create_contract":
        cc = host_function.get("create_contract")
        if cc is None:
            print(
                f"expected a create_contract operation, but this transaction's real operation is {list(host_function.keys())} -- "
                "a returned address here would NOT prove this transaction created anything",
                file=sys.stderr,
            )
            return 1
        actual_wasm = cc.get("executable", {}).get("wasm")
        if expect_wasm and actual_wasm != expect_wasm:
            print(
                f"create_contract operation deploys wasm {actual_wasm}, not the expected {expect_wasm} -- "
                "this is not the deployment transaction for that artifact",
                file=sys.stderr,
            )
            return 1
    else:  # invoke_contract
        ic = host_function.get("invoke_contract")
        if ic is None:
            print(
                f"expected an invoke_contract operation, but this transaction's real operation is {list(host_function.keys())}",
                file=sys.stderr,
            )
            return 1
        if ic.get("contract_address") != expect_contract:
            print(
                f"invoke_contract operation targets {ic.get('contract_address')}, not the expected factory {expect_contract} -- "
                "a map of addresses returned here would not be a verified factory-creation mapping",
                file=sys.stderr,
            )
            return 1
        if ic.get("function_name") not in expect_functions:
            print(
                f"invoke_contract operation calls '{ic.get('function_name')}', not one of the expected creation "
                f"functions {sorted(expect_functions)} -- this transaction is not a verified creation call",
                file=sys.stderr,
            )
            return 1

    try:
        meta = decode_xdr("TransactionMeta", result["resultMetaXdr"])
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
