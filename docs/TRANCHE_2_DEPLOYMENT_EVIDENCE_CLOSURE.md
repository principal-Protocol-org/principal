# Tranche 2 Deliverable 1: deployment evidence closure

Dated 15 November 2026. This closes the deployment-evidence gaps left open across the two
Testnet deployment records this grant's Deliverable 1 touches: the official public dApp
deployment, and the contracts team's own pipeline-proof deployment ("TR2"). Nothing on either
chain was touched to write this document — every transaction below already existed before this
work started; what changed is that each one is now traced back to the contract it created and
cross-checked against what the relevant manifest claims, using only public reads and local,
offline decoding. No contract was deployed, upgraded, or initialized, no oracle value was
pushed, no permission or limit was changed, and no transaction was submitted, in the course of
writing this document.

## The two deployments, and why they are kept apart

The official deployment is what the public dApp, its CI suites, and its end-user walkthrough
actually run against: market `testnet-usdy-1802336344`, built from contracts commit
[`665bb6760372d30307a1a035da24e51776f727e8`](https://github.com/principal-Protocol-org/principal/commit/665bb6760372d30307a1a035da24e51776f727e8)
by the dApp repository's own `scripts/dev-stack/deploy.sh`, deployed by admin account
`GBLPAJ2GVSFUWCUIFMEV4EOV2XIOEWMJRIQ3LOM2Z42TNIY3J6ACB7UA`. Its manifest lives in the dApp
repository, at
[`config/manifests/testnet.json`](https://github.com/principal-Protocol-org/dapp/blob/testnet/config/manifests/testnet.json),
pinned here to the dApp repository's `testnet` branch at commit `0fa85478c2e7180cf0914fd1a7a8608e937cd5de`
as it stood on 15 November 2026 (retrieved via the GitHub API at that exact commit, not the
moving branch reference, specifically so this document's own claims stay reproducible even if
`testnet` moves again).

The contracts team's TR2 deployment is a separate, disposable market that exists only to prove
the scripted deployment pipeline (`scripts/deploy/deploy.sh`,
`scripts/deploy/verify-deployment.sh`) itself end to end on real Testnet — nothing in the public
dApp, its CI, or its walkthrough runs against it, and
[`docs/TRANCHE_2_TESTNET_ACCEPTANCE.md`](TRANCHE_2_TESTNET_ACCEPTANCE.md) says so explicitly in
its own first section ("supersedes nothing"). Its manifest is
[`deployments/testnet/deployment-manifest.json`](../deployments/testnet/deployment-manifest.json)
in this repository, deployed by admin account
`GBTZNAOBEIRAZPN6BKD7SZO2L3DGAQQZ6ZDP2YVGGZPO3K2E7YBSDFFF`. Both deployments share the same
contracts source lineage (TR2's pipeline commit, `aae6cf81346a51f600393e4267be45e65524ae4e`,
descends from `665bb67` with the OracleAdapter SEP-40 pull extension added and nothing else
changed in the other eleven contracts — confirmed below, not assumed), but they are two distinct
sets of live contract instances with two distinct admin keys, and this document never merges
their transaction records or their verification results into one.

## The gaps this closes

Three gaps were open going into this work. First, the official dApp manifest records contract
addresses and wasm hashes but never recorded a single deployment transaction — its own evidence
doc, `TRANCHE_2_EVIDENCE.md`, says plainly that the manifest "does not record the deployment
transactions" and offers only a human-read timestamp for one contract (the Router, "13 November
2026 09:06:44 UTC") as a clue, not proof. Second, the TR2 pipeline manifest had five empty
`deploy_tx` fields for its shared infrastructure (OracleAdapter, Permissioning, RiskControl,
Router, MarketFactory) — lost, per its own acceptance doc, to a bug in an early run of the
deploy script, before it learned to capture a transaction hash out of a subshell correctly.
Third, TR2's own seven market contracts were created through three MarketFactory calls whose
hashes were already written down in the acceptance document's prose, but never consolidated
into the manifest file itself in a form a script could check.

## How the transactions were actually recovered

A transaction's own classic result (`result_xdr`, what Horizon and stellar.expert both show by
default) does **not** contain the address of a contract that `CreateContract` created. The
`InvokeHostFunctionResult` success case there is a SHA-256 hash of an internal pre-image, not the
return value — decoding it directly looks like it worked (it produces a plausible 32-byte value)
but is wrong, and this was confirmed the hard way during this work: an early attempt decoded
twelve `result_xdr` values this way and matched zero of them against any known contract address,
for either deployment. The real return value — the new contract's actual address, for a
`CreateContract` call, or a named map of several addresses for a MarketFactory call that creates
more than one — lives only in the transaction's Soroban metadata, fetched from Soroban RPC's
`getTransaction` (`resultMetaXdr`, not available from Horizon's standard transaction or operation
endpoints at all) and decoded locally with `stellar xdr decode --type TransactionMeta --output
json`, reading `.v4.soroban_meta.return_value` (or `.v3...` on an older ledger).

For each deployment, every `invoke_host_function` operation of type
`HostFunctionTypeHostFunctionTypeCreateContract` submitted by the deployer account in the
relevant time window was found via that account's own Horizon operation history (both are
dedicated keys with their own history, not shared with unrelated activity beyond a couple of
earlier practice/test deployments on the same keys, excluded by timestamp and by not matching
any known address). Each candidate transaction's real return value was then decoded as above and
compared, address for address, against what the relevant manifest already claims. A candidate
whose decoded address did not match any manifest entry would have been reported as unresolved,
not guessed into place by proximity to a known `init_tx` timestamp — none were; every single
`CreateContract` transaction examined for either deployment matched exactly one contract already
on record.

## What was recovered, and what still is not

For the official deployment, all twelve `CreateContract` transactions were found and verified
this way: Router at ledger 5707432 (`2ca519146eca2cb6d359017432cbe2de8fd40eae7f8e362d3d4c484bd43803dc`,
[stellar.expert](https://stellar.expert/explorer/testnet/tx/2ca519146eca2cb6d359017432cbe2de8fd40eae7f8e362d3d4c484bd43803dc)),
matching the evidence doc's own stated time for it exactly; OracleAdapter at ledger 5707342
(`68be08adffa4c4c02d11c50a4719822d5843d7bc0641bb8ca5f9e8b174514f92`); Permissioning at ledger 5707348
(`6c381be195fccd2e289eeb638736f47f0f49ee604e1049c0a782868daa425d0e`); RiskControl at ledger 5707350
(`652844b9e4262322872e8a449ae47ea8ff160f4426dab9da3c54354c1d346648`); SYWrapper at ledger 5707353
(`85f8a91f2bdd2fe6adf2ef7f26cece352cf7940d563fb507964a34b4d2fb5183`); PTToken at ledger 5707408
(`2d5d4409b52f149e64b089deb2195420818a6b4b20fa8a6cf9a3d08b66156035`); YTToken at ledger 5707411
(`bc087931cad49b0a83022bd7b45ae1de1db22587f3935df374a75b1366555f92`); MarketConfig at ledger 5707413
(`ed48bb3765a2125f23b5c2aa87ffe026e9d924772d8a95d4181bd61ffed4df46`); PrincipalManager at ledger 5707416
(`4434237bb38591fb4f9e259fc857e6a622e203fffe0aeb1ee4f3e7d8504b2917`); MarketPool at ledger 5707423
(`70ce1725e4074344df33c01123c3b4f28eb1753f2c21f103ea121bbc328a9bdf`); RecoveryEscrow at ledger 5707425
(`0c67b1e6ebe942e6f8d485f28290a6bffd77327f5797911aa64160d88eab0c69`); MarketFactory at ledger 5707436
(`b5c15f22a374ec1865ca57bfd1caa30385d519e8ba5cc303a1a8221a8ef119bc`). This incidentally settled an
assumption worth stating plainly: the official deployment never actually invokes MarketFactory to
create this market. `scripts/dev-stack/deploy.sh` deploys all twelve contracts directly, one at a
time (the classic eleven-step sequence this project's own `DEPLOYMENT.md` documents, plus one more
step that deploys MarketFactory's wasm without ever calling it) — MarketFactory exists on-chain for
this deployment, correctly, but played no role in creating any of the other eleven. Seven of the
twelve also had a matching `UploadContractWasm` operation recovered the same way, logged in
`deployments/testnet/official-dapp/deployment-manifest.json`; the other five (Permissioning,
RiskControl, PTToken, MarketConfig, RecoveryEscrow) have no upload operation anywhere in this
deployment's own time window at all, most plausibly because that exact wasm hash was already
on-chain from an earlier session on the same deployer key (the account's history goes back to
2 November 2026) and the CLI had nothing left to upload — this is recorded as not recovered, not
guessed at.

For the TR2 deployment, the five missing shared-infrastructure `deploy_tx` values were recovered
the same way, all five clustered in a single early run at 13 November 2026 20:39:54–20:41:14 UTC:
OracleAdapter (`6c0e4550494ac3c704ac271d1f78f694a889ca40fedc0f1b3ab505678b9a28d8`), Permissioning
(`e2b92a798ae4cc238c5ec360bc3d72d16c86f2b4f2ca02d4c6a5447bea29ec31`), RiskControl
(`e972e59be3e56c280ecf646e33f44aa81c8cfbdacdab7077a2cda6f0cd10e010`), Router
(`d24617082427c08bbb11f96666fa98930ea2ed48ce2a7e0a1b40dc1d73075473`), MarketFactory
(`6891882ffaafa0d569672b80d43ffa17ee20cf254f033f84d8922f678b030a97`). These are now written into
`deployments/testnet/deployment-manifest.json` itself, each one explicitly marked
`"deploy_tx_reconstructed": true` and explained in a new `evidence_reconciliation` block in that
same file, rather than silently presented as if the original 13 November run had captured them —
it did not; the manifest's own `built_at`/`source_commit` fields, describing that original run,
are untouched. The three MarketFactory calls that created TR2's seven market contracts were
likewise re-verified against their real decoded return values (not re-trusted from the acceptance
document's prose) and consolidated into that same `evidence_reconciliation` block:
`factory.create_tokens` (`5fa113887a4ca2f70e35f92c1fc32ba58833ee7418526df5fb64f3c0b7dc3094`)
created SYWrapper, PTToken, and YTToken; `factory.create_engine`
(`27f3a2dc4db00216e802a46a8040a9c87819b6beab12cc60bca3c6f23590c9e2`) created MarketConfig and
PrincipalManager; `factory.create_pool_and_escrow`
(`6d84844d00b806f783f85696d4f9241b04f13d86e057b1b6cc46ed39fa7becb0`) created MarketPool and
RecoveryEscrow. All seven match the manifest's own recorded addresses exactly. Between the five
recovered shared-infrastructure transactions and these three factory transactions, every one of
TR2's twelve contracts now has a verified creation transaction on record, either directly or
through its factory call.

## On-chain code, independently re-checked

Separately from transaction provenance, the deployed bytecode itself was re-fetched and re-hashed
for both deployments on 15 November 2026 (`stellar contract fetch --id <address> --network
testnet`, then `sha256sum`, a public read with no signing key). All twelve of the official
deployment's contracts match `testnet.json`'s own `wasmHashes` exactly — this independently
reproduces, rather than merely trusts, the dApp evidence document's own 13 November 2026 finding
of the same result. All twelve of TR2's contracts match `deployment-manifest.json`'s own recorded
hashes exactly, unchanged from the acceptance document's own §4/§9.3 findings.

## Source-to-byte reproducibility, attempted honestly

Matching a manifest's own recorded hash is not the same claim as reproducing that bytecode from
source, and the two were kept separate here, as the task this document closes out requires.
Two isolated builds were made from `git archive` snapshots of each deployment's exact recorded
source commit — never a working tree, never the repository's own checkout — using this
verification's own machine, `rustc 1.94.1 (e408947bf 2026-03-25)`, the same `cargo build
--workspace --release --target wasm32v1-none` command both deployments' own build processes use.

The TR2 commit (`aae6cf8`) reproduced **12 of 12** contracts byte-identically. This machine's own
toolchain version, 1.94.1, is exactly what TR2's own manifest already records as its build
toolchain — a controlled, apples-to-apples comparison, and it succeeded completely.

The official deployment's commit (`665bb67`) reproduced **0 of 12** byte-identically against the
deployed/manifest hashes on this same machine. This is not a new or contradicting finding: the
dApp's own `TRANCHE_2_EVIDENCE.md` (§2.1) already documents that its CI runner's build of the
same commit also got 0 of 12 against the same hashes, attributing it to wasm files that embed the
building machine's own absolute cargo-registry path, and states that matching the official
hashes needed Rust 1.95.0 specifically, which this verification's machine does not have (1.94.1).
That a different toolchain version, on a third machine, also fails to reproduce the official
hashes is consistent with, not a new weakness in, their own already-published finding. One real
data point narrows the explanation, though: this verification's own two isolated builds, run on
the identical machine and toolchain, produced **byte-identical wasm for eleven of the twelve
contracts** between the two commits (every contract except OracleAdapter, which is exactly the
one file `aae6cf8` actually changed relative to `665bb67`). That rules out this machine's own
build path as an automatically-fatal source of non-determinism for this codebase — the same
toolchain, same machine, same eleven contracts, reproduced identically against one deployment's
recorded hash (TR2's) and not the other's (the official one), which points at the Rust version
difference (1.94.1 here vs. the 1.95.0 the dApp's own developer used) as the more likely
remaining factor, on top of the already-documented absolute-path embedding. This was not run
further to a firm conclusion — doing so would mean installing Rust 1.95.0 specifically to test
that one variable in isolation, which this pass did not do — and is reported as a narrowing, not
a closed question.

## What `git_tree_dirty: true` means, and why it was not touched

TR2's manifest has recorded `git_tree_dirty: true` since it was first written, because the
13 November 2026 run that produced it ran against an uncommitted tree on top of `665bb67` — the
scripted pipeline itself was being written and debugged in that same working tree, live, as the
spec's own §4.4 anticipates a deployment might need to do. That fact describes a specific
historical moment and is preserved exactly as it was recorded; it was not changed to `false` just
because a checkout of the repository today happens to be clean, because that would misrepresent
what was actually true of the tree the deployed bytecode came from. Separately, and only as an
addition rather than a replacement: the isolated `git archive` rebuild of `aae6cf8` above is
itself a clean-source attestation, dated 15 November 2026, reproducing the deployed bytecode
exactly from a commit, with no working-tree state involved at all — this is new evidence that
sits alongside the historical `git_tree_dirty: true` flag, not a reason to retire it.

## Reconciling the dApp review's own citations

The dApp's own `docs/scf/TRANCHE_2_REVIEW.md` cites the contracts pipeline specifically at commit
`aae6cf81346a51f600393e4267be45e65524ae4e` ("contracts `main` = `aae6cf8` (14 November 2026)"),
which is accurate as a description of that exact, deliberately pinned commit — the review was
right to pin to an immutable commit rather than a moving branch reference, and its description of
that commit's own state (one failing check, `git_tree_dirty`) matches what that commit's own
acceptance document says. Since then, contracts `main` has moved twice: once to
[`24d980d6fbdffae9bd561a80cc7dcb9f1846fa11`](https://github.com/principal-Protocol-org/principal/commit/24d980d6fbdffae9bd561a80cc7dcb9f1846fa11),
which fixed the six issues an earlier dApp-side review of the pipeline itself had found (the
`git_tree_dirty` precondition, an oracle-genesis push that could silently fail as a decrease, a
wasm-hash check that only covered five of twelve contracts, a market id that drifted with
wall-clock time, a disabled RiskControl circuit breaker, and an oracle relay that logged a
rejected submission as a success) plus one further monitoring-integrity fix from a dedicated
security review (`last_rejection()` could be masked by any permissionless caller); and separately,
on a branch not yet merged to `main` (`fix/testnet-gap-closure-redeploy`, deliberately kept off
`main` so its own fresh set of contract addresses would not land there), which closes the two
gaps this very document also addresses for the official deployment's own equivalent gaps, plus
one more idempotency fix found while proving it. None of this contradicts the dApp review's own
citations — they describe `aae6cf8` correctly — but a reader relying only on the review without
checking whether `main` has moved since would miss that several of the caveats it lists against
that pinned commit are now fixed on `main` itself, and that the two remaining gaps named in that
commit's own acceptance document (the `transactions[]` consolidation and the OracleAdapter
bytecode-source gap from the `last_rejection()` fix) are closed, for the TR2 deployment, by this
document, and separately by the still-unmerged branch for an even fresher instance of that same
deployment.

## Reproducible, read-only verification

Two scripts now check this evidence, kept separate on purpose. `scripts/deploy/verify-deployment.sh`
(unchanged by this work) checks TR2's current contract identity, topology, and operational
health — including oracle freshness, which reads as unavailable right now simply because enough
time has passed since the last push for the 3,600-second staleness window to lapse; that is
today's real operational state, not a defect, and was not refreshed to make a check pass, since
every write is out of scope for this document. `scripts/deploy/verify-evidence.sh` is new: a
read-only evidence verifier that supports either manifest's own schema, auto-detected, and checks
that each recorded `deploy_tx` (or, for a MarketFactory-created contract, its consolidated
factory-creation mapping) really does create the contract id a manifest claims, that the current
on-chain wasm hash matches what the manifest records, how complete the evidence is, and whether
a source/build attestation is on file — using the same `stellar xdr decode`-based method this
document describes, via `scripts/deploy/lib/resolve-created-contract.py`. Run it against either
manifest:

```
./scripts/deploy/verify-evidence.sh deployments/testnet/deployment-manifest.json
./scripts/deploy/verify-evidence.sh deployments/testnet/official-dapp/deployment-manifest.json
```

Both currently report every contract's creation transaction verified, every on-chain wasm hash
matching, and only the genuinely open items as warnings (the official deployment's USDY Stellar
Asset Contract has no `deploy_tx` of its own, since it was created via `stellar contract asset
deploy`, not the `deploy_contract()` upload/deploy pair the other twelve went through). Offline
tests for the underlying decoding logic, including the three negative cases this closure work
was asked to cover specifically — a transaction that created a different contract than claimed,
a wrongly recorded deployment hash, and a wasm hash mismatch — are in
`scripts/deploy/tests/test_evidence.py`, built from real, cited transactions saved as fixtures
rather than synthetic data, and run with no network access at all:

```
python3 scripts/deploy/tests/test_evidence.py -v
```

## What this does not close

The official deployment's five un-recovered upload transactions (Permissioning, RiskControl,
PTToken, MarketConfig, RecoveryEscrow) stay open — most plausibly already-uploaded wasm reused
from an earlier session on the same key, per the explanation above, but that is a plausible
explanation, not a proof, and no further Horizon history beyond this deployment account's own
was searched to try to confirm it. The Rust-version question in source-to-byte reproducibility
above was narrowed, not settled. Nothing about the dApp repository changed, could not change
under this work's own scope, and did not need to: its supplemental deployment evidence —
the official deployment's enriched manifest, with its recovered transactions — now lives in this
repository instead, at `deployments/testnet/official-dapp/`, clearly marked throughout as
reconstructed rather than original, and the dApp repository's own files are exactly as they were
before this work started.
