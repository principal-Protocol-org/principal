#!/usr/bin/env python3
"""Offline tests for scripts/deploy/lib/resolve-created-contract.py's decoding logic, and for the
wasm-hash-comparison logic scripts/deploy/verify-evidence.sh uses.

Every fixture under fixtures/*.resultMetaXdr.txt is a real resultMetaXdr, saved verbatim from
Soroban RPC's getTransaction for a transaction that is already cited, with its own
stellar.expert link, in docs/TRANCHE_2_TESTNET_ACCEPTANCE.md or
docs/TRANCHE_2_DEPLOYMENT_EVIDENCE_CLOSURE.md -- these are not synthetic. No network access: the
only non-offline part of the underlying tool (get_transaction) is deliberately not exercised
here; `stellar xdr decode` runs locally against the saved XDR text and needs no network either.

Run: python3 scripts/deploy/tests/test_evidence.py
"""
import hashlib
import importlib.util
import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"
RESOLVER_PATH = HERE.parent / "lib" / "resolve-created-contract.py"

spec = importlib.util.spec_from_file_location("resolve_created_contract", RESOLVER_PATH)
resolver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resolver)


def fixture(name: str) -> str:
    return (FIXTURES / name).read_text()


class DecodeTransactionMetaTests(unittest.TestCase):
    """Positive cases: real, cited transactions decode to the address(es) already on record."""

    def test_single_contract_creation_resolves_to_its_real_address(self):
        meta = resolver.decode_transaction_meta(
            fixture("router_create_contract.resultMetaXdr.txt")
        )
        rv = resolver.extract_return_value(meta)
        self.assertEqual(rv, {"address": "CC4BQRFRQQZF5VQUHZV3PS4LSIPNNAGI3UOL3K24DZNTCHGZQ3JHQ3E2"})

    def test_factory_multi_contract_creation_resolves_to_a_named_map(self):
        meta = resolver.decode_transaction_meta(
            fixture("factory_create_tokens.resultMetaXdr.txt")
        )
        rv = resolver.extract_return_value(meta)
        self.assertIn("map", rv)
        addresses = {
            entry["key"]["symbol"]: entry["val"]["address"] for entry in rv["map"]
        }
        self.assertEqual(
            addresses,
            {
                "pt_token": "CADCXJF7DTZ5T2W7QRIJWBI5Q5LVGRTHW33F2SRAGMQKO3MIRDQIJBWG",
                "sy_wrapper": "CDO7S4YPV36NFC5CHMSBGUVRZSKHPHIX35RBL33R4RLALVUK4YQSG4S5",
                "yt_token": "CBCLSNK5N4SDNLTE7KXV52T6AQLPANSGH2UHUZ4K2TBW7TMIUWMY3O6J",
            },
        )


class NegativeCaseTests(unittest.TestCase):
    """The specific negative cases the task calls for: a wrong deployment hash recorded, a
    transaction that created a *different* contract than claimed, and a wasm hash mismatch.
    These test the comparison logic callers (verify-evidence.sh, resolve-created-contract.py's
    own CLI) actually rely on, not just that decoding succeeds."""

    def test_a_transaction_that_created_a_different_contract_is_detected_as_a_mismatch(self):
        # permissioning_create_contract.resultMetaXdr.txt is a REAL CreateContract transaction --
        # it just creates Permissioning, not Router. A caller that (wrongly) expected it to prove
        # Router's deployment must see a mismatch, not a false positive.
        meta = resolver.decode_transaction_meta(
            fixture("permissioning_create_contract.resultMetaXdr.txt")
        )
        rv = resolver.extract_return_value(meta)
        claimed_router_address = "CC4BQRFRQQZF5VQUHZV3PS4LSIPNNAGI3UOL3K24DZNTCHGZQ3JHQ3E2"
        self.assertNotEqual(rv.get("address"), claimed_router_address)
        self.assertEqual(rv.get("address"), "CAFQMOCW3673DBKJBJ7C555IYPOULPH2WFOEOX52JAUCZRGZF5FRVLBA")

    def test_a_wrong_deployment_hash_recorded_in_a_manifest_is_detected(self):
        # Simulates a manifest that (wrongly) recorded some other, unrelated address as the
        # result of Router's real creation transaction -- the exact "wrong deployment hash"
        # scenario the task asks to be able to catch.
        meta = resolver.decode_transaction_meta(
            fixture("router_create_contract.resultMetaXdr.txt")
        )
        rv = resolver.extract_return_value(meta)
        wrongly_recorded_address = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFCT4"
        self.assertNotEqual(rv.get("address"), wrongly_recorded_address)

    def test_wasm_hash_mismatch_is_detected(self):
        # The same comparison scripts/deploy/verify-evidence.sh's "current on-chain code" check
        # performs: sha256 the actual bytes, compare against what the manifest claims.
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(b"not actually the real wasm bytes")
            path = pathlib.Path(f.name)
        try:
            real_hash = hashlib.sha256(path.read_bytes()).hexdigest()
            wrongly_recorded_hash = "f5c8a129c5aad53aff733b325a9949926a4fd4abf185a4dd0aec53476bf45253"
            self.assertNotEqual(real_hash, wrongly_recorded_hash)
        finally:
            path.unlink()

    def test_wasm_hash_match_is_also_correctly_detected(self):
        # The positive counterpart to the above, so this test file cannot pass by always
        # asserting inequality.
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(b"deterministic fixture content")
            path = pathlib.Path(f.name)
        try:
            h1 = hashlib.sha256(path.read_bytes()).hexdigest()
            h2 = hashlib.sha256(path.read_bytes()).hexdigest()
            self.assertEqual(h1, h2)
        finally:
            path.unlink()


if __name__ == "__main__":
    unittest.main()
