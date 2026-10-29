# Dapp Integration: Market Creation and Current Testnet Status

> **Update:** a market-creation factory now exists — `contracts/market_factory`, deployed and
> exercised live on Testnet. Section 1 below is kept as the record of the gap as it stood, but see
> [docs/MARKET_FACTORY.md](MARKET_FACTORY.md) for the current, correct answer and a worked example
> on real Testnet transactions.

This note answers two questions a dapp developer needs settled before building against Principal Protocol:

1. **Is there a factory contract that lets an RWA issuer open a market from the dapp UI?** At the
   time this note was first written, no (see below). **As of 29 October 2026, yes** — see
   [docs/MARKET_FACTORY.md](MARKET_FACTORY.md).
2. **Is the deployed Testnet market closed?** Yes — verified live, right now, not just from old deployment notes.

It then gives the integration path that actually works today, with runnable scripts.

---

## 1. There was no market-creation factory (historical — see the update above)

Checked directly against the contract source, not just the docs, as of the original Tranche 1 code:

```bash
grep -rn "deployer()\|ContractIdPreimage\|Deployer" contracts/*/src/*.rs
# no matches anywhere in the workspace
```

At that point, no contract in this codebase called Soroban's `env.deployer()` (the primitive a real on-chain factory uses to deploy + initialize child contracts atomically). `Router` was the closest thing to a registry, but read its own functions:

```rust
pub fn register_market(env: Env, caller: Address, pool: Address) -> MarketInfo
pub fn unregister_market(env: Env, caller: Address, pool: Address)
pub fn is_registered(env: Env, pool: Address) -> bool
pub fn market(env: Env, pool: Address) -> MarketInfo
```

`register_market` takes the address of a **pool that already exists** and cross-checks its wiring; it does not deploy anything. `docs/AMM_DESIGN.md` says this in plain words: *"we register markets by pool rather than deploying pairs from a factory."*

`docs/TRANCHE_1_AUDIT.md` (finding on deploy-time atomicity) independently confirmed this was a known, intentional gap for that tranche:

> *"The correct fix is deploy-time atomicity (a `__constructor`... or a factory that deploys and initializes in one transaction), which is a deployment-tooling change for Tranche 2, not a contract-logic change that this tranche's scope covers."*

That gap is now closed: `contracts/market_factory` does exactly this, deployed and proven on real Testnet transactions. It took three calls rather than one to stay inside a real transaction's resource budget — the single-call version was tried first and genuinely failed on-chain, not just in a test. Full detail, including every transaction hash, is in [docs/MARKET_FACTORY.md](MARKET_FACTORY.md).

**What "creating a market" actually is today:** deploying eight contracts (`SYWrapper`, `PTToken`, `YTToken`, `MarketConfig`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow`, and registering on `Router`) in a fixed dependency order, each `initialize` call signed by the underlying asset's **live issuer authority** (the SAC's current `admin()`, or for a SEP-57 token, the `operator` role). That issuer-signature requirement is the real access-control gate — it is already as strong as a factory's gate would be, just not atomic and not wrapped in one contract call. The full 11-step sequence is documented in `docs/DEPLOYMENT.md` and executed verbatim by the test fixture at `contracts/integration_tests/src/stack.rs`.

**What this means for your dapp:** your backend (whatever service holds or routes the issuer's signing authority) is the factory, right now. Section 3 gives you that as a single script instead of eleven manual CLI steps, so you can call it from your deployment pipeline today and swap it for a real on-chain factory in Tranche 2 without changing anything your frontend talks to (the deployed contract addresses and ABI are identical either way).

---

## 2. The deployed Testnet market — verified closed, live, today

Someone told you the deployed market is closed. That is correct, and it is not a guess — three independent live checks against Stellar Testnet confirm it, run as part of preparing this note:

**a. Maturity has passed.** The "final market" from the 4 October 2026 Testnet run uses `MarketPool` at `CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR`:

```bash
$ stellar contract invoke --id CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR \
    --source-account GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB \
    --rpc-url https://soroban-testnet.stellar.org \
    --network-passphrase "Test SDF Network ; September 2015" --send=no -- maturity
1791056732
```

Unix `1791056732` is roughly **2026-10-04, 20 minutes after deployment** — the market was deliberately given a 20-minute maturity window so the full lifecycle (deposit → mint → trade → yield → compliance recovery → settlement → redemption) could be exercised quickly on Testnet. That was the point of that run; it was never meant to stay open. Today's date is well past that.

**b. The pool itself refuses to trade, live, right now:**

```bash
$ stellar contract invoke --id CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR \
    --source v11-carol --rpc-url https://soroban-testnet.stellar.org \
    --network-passphrase "Test SDF Network ; September 2015" --send=no \
    -- swap_pt_for_sy --from GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5 \
       --to GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5 --pt_in 100 --min_sy_out 0
❌ error: transaction simulation failed: HostError: Error(Contract, #6)
```

`MarketPool` error `6` is `Expired` (`contracts/market_pool/src/lib.rs`, the `Error` enum). This matches `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §4's note that a post-maturity swap was refused the same way during the original run — the behavior is stable, reproducible, and not paused-by-accident.

**c. New PT/YT issuance is closed too, same live check against `PrincipalManager` (`CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27`):**

```bash
$ stellar contract invoke --id CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27 \
    --source v11-carol --rpc-url https://soroban-testnet.stellar.org \
    --network-passphrase "Test SDF Network ; September 2015" --send=no \
    -- mint --from GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5 --sy_shares 100
❌ error: transaction simulation failed: HostError: Error(Contract, #6)
```

`PrincipalManager` error `6` is `AlreadyMature` (`contracts/principal_manager/src/lib.rs`). This is **not** the global pause (`is_paused` on the pool reads `false` — checked live, not inferred).

**One nuance that matters for how you read "closed":** `SYWrapper` (`CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT`) has no maturity field at all — wrapping the underlying into SY shares still works on this contract today:

```bash
$ stellar contract invoke --id CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT \
    --source v11-carol ... --send=no \
    -- deposit --from GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5 --amount 1 --min_shares_out 0
📅 ... Success ...
"1"
```

So "the market is closed" is precise, not approximate: **tokenization (`mint`) and trading (all `MarketPool` swaps/liquidity ops) are closed for this specific market** because its maturity passed; the shared underlying-wrapping step (`SYWrapper.deposit`) isn't gated by maturity at all, it's just pointless to call on its own once `mint` is closed, since there is nothing useful to do with the resulting SY in *this* market.

A reusable version of checks (a)–(c), parametrized for any market, is in [`scripts/check_market_status.sh`](../scripts/check_market_status.sh) — point it at any `MarketPool`/`PrincipalManager` pair and it tells you, live, whether that specific market is open.

---

## 3. What to actually build against

### 3a. If you want to demo/display the matured Testnet market

Its final on-chain state (balances, the recovery record, fees paid out) is permanent and still readable with plain view calls — nothing about "closed" stops you from reading history. Use the addresses and figures in `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §3 and §5 for a read-only demo UI (balances, recovered-position record, supply). Do not route write calls (`mint`, swaps, `deposit` into this specific `PrincipalManager`) at it — they're gated as shown above.

### 3b. If you need a live, tradeable market (the real integration path)

**Preferred: use the on-chain factory.** `contracts/market_factory` now does this atomically (three
calls instead of fifteen-plus manual steps, each call internally all-or-nothing) and has been
exercised on real Testnet — see [docs/MARKET_FACTORY.md](MARKET_FACTORY.md) for the API and the
transaction hashes. The script-based path below still works and remains useful if you want full
manual control over every step, but the factory is the shorter, safer default now.

Either way, stand up a new market with a future maturity, reusing the shared infrastructure that isn't maturity-bound:

| Component | Reuse or redeploy? |
|---|---|
| `OracleAdapter` | Reuse — not market-specific, just needs a fresh value pushed before use |
| `Permissioning` | Reuse, if the same eligibility rules apply |
| `RiskControl` | Reuse, if one shared circuit breaker budget is acceptable across markets |
| `SYWrapper` | **Always redeploy.** It binds one-time to a `RecoveryEscrow` (`set_recovery_escrow` reverts `RecoveryEscrowAlreadySet` on a second call) — this is exactly finding #1 from the 4 October run that bit the original deployment when the second market tried to reuse the first market's `SYWrapper` |
| `PTToken`, `YTToken`, `MarketConfig`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow` | Always redeploy — one set per maturity |
| `Router` | Reuse — just call `register_market` again for the new pool |

[`scripts/create_market.sh`](../scripts/create_market.sh) runs that whole sequence as one parametrized job (the same 11 steps `docs/DEPLOYMENT.md` documents by hand), signed by the issuer key your backend controls:

```bash
ISSUER_KEY=my_issuer_identity \
UNDERLYING_ID=<the underlying SAC or SEP-57 token contract id> \
MATURITY_TS=$(date -d '+90 days' +%s) \
PROTOCOL_ADMIN_ADDR=<Principal's protocol admin address> \
TREASURY_ADDR=<Principal's fee treasury address> \
ORACLE_ID=<existing oracle, optional> \
PERMISSIONING_ID=<existing permissioning, optional> \
RISK_CONTROL_ID=<existing risk control, optional> \
ROUTER_ID=<existing router, optional> \
./scripts/create_market.sh
```

It prints every deployed contract address at the end, plus a reminder for the one step it deliberately does **not** automate: granting compliance standing (§11 of `DEPLOYMENT.md`) to every address that will hold a balance (`SYWrapper`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow`, the treasury, the market creator's payee). That step requires the issuer's judgment about who is actually eligible to hold the asset — a script should not make that call silently, and the issuer signature alone can't prove eligibility the way a real KYC/allow-list decision can.

### 3c. Wiring this into a UI "Create Market" button

Since the issuer's live signature is the real gate, a "Create Market" button in your dapp has to either:
- run inside a backend service that custodies (or co-signs with) the issuer's key, calling the factory's three `create_*` entrypoints (or `create_market.sh`'s manual equivalent) server-side and returning the new addresses to the frontend, or
- build those same operations as unsigned transaction envelopes and hand them to the issuer to sign one by one in their own wallet (Freighter, etc.) — slower, but keeps the issuer key off your servers.

Either way, the factory's `predict_market(salt)` lets you show the issuer the addresses their signature is about to create *before* they sign — useful for the confirmation screen on that button.

Either way, no change to the frontend's read/write ABI is needed once Tranche 2 ships a real atomic factory — only the deployment step changes, the contract interfaces your dapp already calls (`SYWrapper.deposit`, `PrincipalManager.mint`, `MarketPool.swap_*`, `Router.*`) stay the same.

---

## 4. Checklist carried over from the actual Testnet run (don't relearn these the hard way)

From `docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md` §6 and `docs/DEPLOYMENT.md`:

- [ ] Pick maturity generously. Each deploy step takes tens of seconds on Testnet; budget well over an hour between first deploy and first mint, and far more in a real launch.
- [ ] Deploy a **new** `SYWrapper` per market — never reuse one that already has a `RecoveryEscrow` set.
- [ ] Oracle freshness is enforced on every mint/redeem/settle (must be < 3,600s old) — a relay needs to keep pushing values for the market's whole life, not just at setup.
- [ ] Fee payees (`treasury`, the creator payee) need `Permissioning` standing *before* `claim_protocol_fees` / `claim_creator_fees` will succeed — this specifically failed with error 8 on the first attempt of the real run.
- [ ] Every contract that custodies value (`SYWrapper`, `PrincipalManager`, `MarketPool`, `RecoveryEscrow`) needs both underlying-level authorization *and* `Permissioning.grant_account` (+ `grant_asset` for PT/YT where listed in §11 of `DEPLOYMENT.md`) — nothing moves until this is done.
- [ ] Timestamps passed to the oracle must not be ahead of the ledger's close time (`TimestampInFuture`, error 7) — derive them from the latest ledger close time, not the local clock.

---

## 5. References

- [`docs/DEPLOYMENT.md`](DEPLOYMENT.md) — the full manual procedure these scripts automate
- [`docs/TECHNICAL_SPECIFICATION.md`](TECHNICAL_SPECIFICATION.md) §17 — the canonical deployment-order spec
- [`docs/TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md`](TESTNET_CURRENT_DEPLOYMENT_EVIDENCE.md) — the run this note's live checks reproduce
- [`docs/TRANCHE_1_AUDIT.md`](TRANCHE_1_AUDIT.md) — the audit finding that names deploy-time atomicity / a factory as Tranche 2 scope
- [`scripts/create_market.sh`](../scripts/create_market.sh), [`scripts/check_market_status.sh`](../scripts/check_market_status.sh) — the scripts this note references
