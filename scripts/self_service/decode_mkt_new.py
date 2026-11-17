#!/usr/bin/env python3
"""Decode a MarketFactory `mkt_new` event's XDR payload into (salt, market_pool, recovery_escrow).

`mkt_new` is published as `((salt, pool, escrow))` -- see
contracts/market_factory/src/lib.rs's own `create_pool_and_escrow`. Used by
registration_watcher.sh, which gets each event's raw base64 "value" field from
`stellar events --output json` and pipes it through this script rather than hand-parsing XDR in
bash. Requires the `stellar_sdk` Python package (`pip install stellar-sdk`).

Usage: decode_mkt_new.py <base64-scval>
Prints one JSON object: {"salt": "<64 hex chars>", "pool": "C...", "escrow": "C..."}
"""
import json
import sys

from stellar_sdk import scval
from stellar_sdk.xdr import SCVal


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: decode_mkt_new.py <base64-scval>", file=sys.stderr)
        return 2
    sc_val = SCVal.from_xdr(sys.argv[1])
    if sc_val.vec is None or sc_val.vec.sc_vec is None:
        print("error: expected an SCV_VEC value", file=sys.stderr)
        return 1
    items = list(sc_val.vec.sc_vec)
    if len(items) != 3:
        print(f"error: expected 3 elements (salt, pool, escrow), got {len(items)}", file=sys.stderr)
        return 1
    salt_item, pool_item, escrow_item = items
    salt = salt_item.bytes.sc_bytes.hex() if salt_item.bytes else None
    pool = scval.from_address(pool_item).address if pool_item.address else None
    escrow = scval.from_address(escrow_item).address if escrow_item.address else None
    if salt is None or pool is None or escrow is None:
        print("error: unexpected element types -- not a mkt_new payload?", file=sys.stderr)
        return 1
    print(json.dumps({"salt": salt, "pool": pool, "escrow": escrow}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
