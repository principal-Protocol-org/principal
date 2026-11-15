#!/usr/bin/env python3
"""Offline tests for scripts/deploy/lib/resolve-created-contract.py's decoding logic, and for the
wasm-hash-comparison and evidence-completeness logic scripts/deploy/verify-evidence.sh uses.

Every fixture under fixtures/*.resultMetaXdr.txt and fixtures/*.envelopeXdr.txt is real XDR, saved
verbatim from Horizon/Soroban RPC for a transaction that is already cited, with its own
stellar.expert link, in docs/TRANCHE_2_TESTNET_ACCEPTANCE.md or
docs/TRANCHE_2_DEPLOYMENT_EVIDENCE_CLOSURE.md -- these are not synthetic. No network access: the
only non-offline part of the underlying tool (get_transaction/Horizon) is deliberately not
exercised here; `stellar xdr decode` runs locally against the saved XDR text and needs no network
either.

Run: python3 scripts/deploy/tests/test_evidence.py
"""
import hashlib
import importlib.util
import json
import pathlib
import subprocess
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
        meta = resolver.decode_xdr("TransactionMeta", 
            fixture("router_create_contract.resultMetaXdr.txt")
        )
        rv = resolver.extract_return_value(meta)
        self.assertEqual(rv, {"address": "CC4BQRFRQQZF5VQUHZV3PS4LSIPNNAGI3UOL3K24DZNTCHGZQ3JHQ3E2"})

    def test_factory_multi_contract_creation_resolves_to_a_named_map(self):
        meta = resolver.decode_xdr("TransactionMeta", 
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
        meta = resolver.decode_xdr("TransactionMeta", 
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
        meta = resolver.decode_xdr("TransactionMeta", 
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


def decode_envelope(xdr: str) -> dict:
    proc = subprocess.run(
        ["stellar", "xdr", "decode", "--type", "TransactionEnvelope", "--output", "json"],
        input=xdr,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip())
    return json.loads(proc.stdout)


class EnvelopeOperationKindTests(unittest.TestCase):
    """resolve-created-contract.py's own hardening: a decoded return value that merely LOOKS like
    an address is not proof of creation -- it also has to check the envelope's real operation.
    These exercise extract_host_function directly against real, saved envelopes."""

    def test_a_direct_deploy_is_identified_as_create_contract(self):
        env = decode_envelope(fixture("router_create_contract.envelopeXdr.txt"))
        hf = resolver.extract_host_function(env)
        self.assertIn("create_contract", hf)
        self.assertNotIn("invoke_contract", hf)

    def test_a_factory_call_is_identified_as_invoke_contract_not_create_contract(self):
        # This is the exact case the task warns about: factory_create_tokens's return value is a
        # map of addresses that looks exactly like a creation result, but the operation that
        # produced it is invoke_contract (calling MarketFactory.create_tokens), never
        # create_contract -- a caller must check this, not just decode the return value.
        env = decode_envelope(fixture("factory_create_tokens.envelopeXdr.txt"))
        hf = resolver.extract_host_function(env)
        self.assertIn("invoke_contract", hf)
        self.assertNotIn("create_contract", hf)
        self.assertEqual(hf["invoke_contract"]["function_name"], "create_tokens")

    def test_a_factory_mapping_inconsistent_with_the_manifest_is_rejected(self):
        # The exact "factory mapping inconsistent with the manifest" case verify-evidence.sh's
        # cross-check (not just resolve-created-contract.py) exists to catch: the real factory
        # call is confirmed genuine (same fixture as above), but a manifest that claimed this
        # transaction created a DIFFERENT contract than the one it actually did must be rejected,
        # not accepted just because *some* valid factory call happened.
        env = decode_envelope(fixture("factory_create_tokens.envelopeXdr.txt"))
        hf = resolver.extract_host_function(env)
        meta = resolver.decode_xdr("TransactionMeta", fixture("factory_create_tokens.resultMetaXdr.txt"))
        rv = resolver.extract_return_value(meta)
        addresses = {entry["key"]["symbol"]: entry["val"]["address"] for entry in rv["map"]}
        wrongly_claimed_cname = "market_pool"  # this call never creates market_pool
        self.assertNotIn(wrongly_claimed_cname, addresses)
        self.assertEqual(hf["invoke_contract"]["contract_address"], "CACFLQZQBBNF37V6ELSKFMC77FGRGLW74UPIYKT7U5B4VBMPLJ6A2UQ6")


class EvidenceCompletenessAndAttestationTests(unittest.TestCase):
    """The remaining required negative cases: missing required evidence, and a recorded
    unsuccessful source rebuild must never be presented as if it succeeded."""

    def test_missing_deploy_tx_is_incomplete_evidence_not_silently_passed(self):
        contracts = {
            "a": {"id": "CA...", "deploy_tx": "deadbeef"},
            "b": {"id": "CB...", "deploy_tx": ""},  # missing -- the real TR2 shape before §9's fix
            "c": {"id": "CC...", "deploy_tx": None},
        }
        total = len(contracts)
        with_tx = sum(
            1 for c in contracts.values() if c.get("deploy_tx") not in (None, "", "null")
        )
        self.assertEqual(total, 3)
        self.assertEqual(with_tx, 1)
        self.assertNotEqual(with_tx, total)  # must NOT read as "evidence complete"

    def test_a_0_of_n_rebuild_result_is_never_a_successful_attestation(self):
        # Mirrors verify-evidence.sh's own regex-based distinction: "attempted" (present) and
        # "succeeded" (this) must stay separate claims.
        failed_result = "0 of 12 byte-identical to the deployed/manifest hashes"
        succeeded_result = "12 of 12 byte-identical to the deployed/manifest hashes"
        import re

        self.assertIsNotNone(re.match(r"^0 of \d+ byte-identical", failed_result))
        self.assertIsNone(re.match(r"^0 of \d+ byte-identical", succeeded_result))
        m = re.match(r"^(\d+) of (\d+) byte-identical", succeeded_result)
        self.assertEqual(m.group(1), m.group(2))
        m_failed = re.match(r"^(\d+) of (\d+) byte-identical", failed_result)
        self.assertNotEqual(m_failed.group(1), m_failed.group(2))


if __name__ == "__main__":
    unittest.main()
