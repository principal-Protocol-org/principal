# Compliance Architecture — SAC, SEP-8 and SEP-57

Principal's compliance model is **inheritance**: every derived position — SY, PT, YT and LP — may be
held, moved, traded or redeemed only by an account the *underlying asset itself* would let hold the
underlying, and only the underlying's own issuer authority may create a market, retune its fees or
recover a flagged account. Nothing is copied into a Principal-managed list that could drift out of
sync with the issuer's own decisions.

That sentence has to be made concrete for each way a regulated asset can exist on Stellar. This
document covers the three that matter and states, for each, exactly what Principal reads, what it
cannot see, and what the issuer has to do.

| | Classic asset, no flags | **SEP-8 regulated asset** | **SEP-57 (T-REX) RWA token** |
|---|---|---|---|
| What it is | Classic Stellar asset exposed through its Stellar Asset Contract (SAC) | Classic Stellar asset with `AUTH_REQUIRED` + `AUTH_REVOCABLE` and an approval server (SEP-8, *Active*, v1.7.4) | A Soroban contract implementing the `RWAToken` interface, with external Compliance and IdentityVerifier contracts (SEP-57, *Draft*, v0.4.0) |
| Has a SAC? | yes | yes | **no** |
| "May this account hold it?" | `SAC.authorized(a)` (always `true`) | `SAC.authorized(a)` (the trustline / contract-balance authorized flag) | `!is_frozen(a)` **and** `identity_verifier().verify_identity(a)` succeeds |
| "Who is the issuer authority?" | `SAC.admin()` | `SAC.admin()` | an RBAC role held by an `operator` — there is **no `admin()`** |
| Principal's handling | `Kind::Sac` | `Kind::Sac` | `Kind::Rwa` |

All of it lives in one crate, [`principal_compliance`](../contracts/compliance/src/lib.rs). Every
contract routes its checks through it; none of them calls the SAC directly any more.

---

## 1. The adapter

`principal_compliance::init(env, underlying)` runs once, at market creation, in every contract that
takes an `underlying`. It asks the underlying `admin()` — a question only a SAC answers — and stores
`Kind::Sac` or `Kind::Rwa` in the calling contract's own instance storage. Afterwards:

```rust
is_authorized(env, underlying, account) -> bool   // may `account` hold / move the underlying?
is_authority(env, underlying, caller)   -> bool   // is `caller` the issuer's current authority?
authority(env, underlying)              -> Option<Address>   // Some(admin) for a SAC, None for RWA
```

Only the *kind* is cached. Every answer is read live from the issuer's own contract, so a
deauthorization, freeze, key rotation or role change takes effect on the very next call with nothing
to sync.

### `Kind::Sac`

* `is_authorized` → `SAC.authorized(account)`.
* `is_authority`  → `SAC.admin() == caller`.

### `Kind::Rwa` (SEP-57)

* `is_authorized` → `!token.is_frozen(account)` **and** `IdentityVerifier(token.identity_verifier())
  .verify_identity(account)` returns without reverting. `verify_identity` returns nothing and reverts
  when the account is not verified, so Principal calls it through the fallible client and treats any
  failure — including a token with no verifier set — as *not authorized*. It fails closed.
* `is_authority` → SEP-57 deliberately has no `admin()`: "RBAC checks are expected to be enforced on
  the `operator`". So Principal **proves the role instead of reading an address**: it calls
  `token.set_address_frozen(caller, <caller's current frozen flag>, operator = caller)`. The token
  only accepts that from an authorized operator, and because the value written equals the value
  already stored it changes nothing. If the call succeeds, `caller` holds the role; if it reverts,
  they do not. (The caller must have signed the surrounding invocation, as for any privileged call.)

  The probe means an RWA market is created, retuned and recovered by *whoever the token currently
  authorizes*, evaluated live — the exact property the SAC `admin()` read gives a classic asset.

An RWA has no address to pay the creator's 80% fee share to, so `MarketConfig` stores an explicit
`creator_payee` for `Kind::Rwa` markets (initially the operator that created the market, changed with
`set_creator_payee`, which is itself operator-gated). For a SAC the creator is `SAC.admin()`, read at
payout time.

---

## 2. SEP-8 regulated assets: what PT/YT inherit, and what they cannot

SEP-8 describes how an issuer of an `AUTH_REQUIRED` + `AUTH_REVOCABLE` classic asset approves
payments: a wallet submits a transaction to the issuer's **approval server**, which answers
`success`, `revised`, `pending`, `action_required` or `rejected`. An approvable transaction is a
**sandwich** — operations that authorize the client's accounts, the payment itself, then operations
that deauthorize the accounts again — so holders sit *deauthorized at rest* and are authorized only
for the duration of one approved transaction.

### What is inherited

A SEP-8 asset is a classic asset, so it has a SAC and Principal treats it as `Kind::Sac`. Everything
in the table above applies unchanged:

* SY `deposit`/`withdraw`/`transfer`, PT/YT `transfer`/`transfer_from`/`mint`, `PrincipalManager`
  `mint`/`recombine`/`redeem`/`claim_yield`, every `MarketPool` trade and LP operation, and LP
  transfers all require `SAC.authorized(account)` on **both** the sending and the receiving side.
* `MarketConfig` creation, fee retuning and `RecoveryEscrow` recovery require the live `SAC.admin()`.
* `RecoveryEscrow` seizes a deauthorized holder's SY/PT/YT/LP into escrow and unwraps to the
  underlying so the issuer can finish with the SAC's native `clawback` (asset flag
  `AUTH_CLAWBACK_ENABLED`, a separate flag from the two SEP-8 requires; the escrow's balance is the
  clawback target). Test: `recovered_underlying_can_be_clawed_back_natively_by_the_issuer`.

PT and YT are **not** SEP-8 assets themselves. They are Soroban contracts, so there is no trustline
for them and no approval server for them. Their transfers are policed on-chain, at every transfer, by
inheriting the underlying's flag — an investor the issuer has deauthorized can neither send nor
receive PT/YT/LP, and the escrow can recover what they hold.

### What is *not* inherited, and why

1. **The sandwich cannot wrap a Soroban call.** A Soroban transaction contains exactly one
   operation (`InvokeHostFunction`), so an approval server cannot add the authorize / deauthorize
   operations around it, and a `revised` response has nothing to revise. Principal reads the flag as
   it stands *when the contract runs*. A holder who is only ever authorized *inside* a sandwich is,
   to Principal, **unauthorized** and every operation reverts `NotAuthorizedOnSac` (fail closed).
   For a SEP-8 issuer to support Principal, a holder that has passed the issuer's approval (KYC) must
   be left **persistently authorized**; a later revoke then freezes every derived position at once.
   Test: `sep8_style_asset_needs_persistent_authorization_at_rest`.
2. **Off-chain approval criteria are invisible.** SEP-8's `approval_criteria` (per-transaction
   limits, jurisdiction rules, velocity checks) are enforced by the issuer's server at *signing time*
   on classic transactions. That server never sees a Soroban invocation, so those criteria are not
   inherited by SY/PT/YT/LP. The on-chain analogues are the issuer's control of the authorized flag
   (what KYC approval sets), the optional `Permissioning` layer the market administrator runs, and
   the `RiskControl` limits. An issuer whose regime depends on per-transaction off-chain approval must
   express it as one of those, or not list the asset.
3. **Protocol contracts need standing too.** SAC balances held by a *contract* address (`C…`) carry
   their own authorized flag. The issuer must `set_authorized` the SY wrapper, `PrincipalManager`,
   the pool and the escrow (see [DEPLOYMENT.md](DEPLOYMENT.md)). The Router holds nothing and needs
   none.

---

## 3. SEP-57 (T-REX) RWA tokens: why inheritance broke, and how it is restored

SEP-57 defines the standard interface for permissioned real-world-asset tokens on Stellar:

* `RWAToken` — SEP-41 plus `forced_transfer`, `recover_balance`, `set_address_frozen`,
  `freeze_partial_tokens`, `is_frozen`, `pause`, `set_compliance`, `set_identity_verifier`,
  `compliance()`, `identity_verifier()`, `mint`, `burn`. Every privileged function takes an
  `operator: Address`; the token enforces RBAC on it.
* `Compliance` — a modular, hook-based contract (`transferred`, `created`, `destroyed`) that the token
  calls at fixed points; modules may revert to block an operation.
* `IdentityVerifier` — `verify_identity(user)`, which reverts for an unverified account.

Before this change Principal's compliance read `SAC.authorized()` and `SAC.admin()`. An RWA issued as
a SEP-57 token has **no SAC**, so there was nothing to attach a market to: `initialize` could not
establish who the issuer is, and no per-account eligibility could be read. The adapter in §1 closes
that gap — an RWA underlying is detected automatically and mapped onto the same two questions.

### How each Principal function behaves over an RWA underlying

| Principal action | RWA behaviour |
|---|---|
| Create a market (`MarketConfig`, SY, PT, YT, `PrincipalManager`, `MarketPool`) | `admin` must pass the operator capability probe, live |
| Hold / move SY, PT, YT, LP | both sides must be not-frozen **and** identity-verified |
| `SYWrapper.deposit` / `withdraw` (moves the underlying) | the token's own `transfer` runs its Compliance hooks on the SY wrapper and the user; any module that reverts reverts the deposit (fail closed) |
| Retune fees | operator-gated, live |
| Recover a flagged account | operator-gated; target must be frozen or unverified; SY/PT/YT/LP seized into escrow exactly as for a SAC |
| Creator fee payout | explicit `creator_payee` |

The same integration suites run over both kinds
(`contracts/integration_tests/tests/recovery.rs`, `liquidity_and_router.rs`, `edge_cases.rs`): every
recovery scenario — SY, PT/YT with maturity finalization, LP, batch, records, paused-market recovery —
is executed once over a SAC and once over a SEP-57 token backed by a test RWA token
(`principal_compliance::mock_rwa`).

### What an RWA issuer must do

1. Give the SY wrapper, `PrincipalManager`, the pool and the escrow a **verified identity** (and keep
   them unfrozen) — they custody SY/PT/YT/underlying and are participants like any other. The wrapper
   in particular: without it the first deposit reverts inside the token's own transfer.
2. Grant the operator role to whoever should create markets and run recovery.
3. Leave any Compliance modules that should apply to *deposits* installed; they run inside
   `SYWrapper.deposit`/`withdraw` when the underlying moves.

### Limits, stated plainly

* **Only frozen + identity are evaluated on PT/YT/LP transfers.** PT/YT/LP moving between users does
  not move the underlying, so the token's Compliance *module* rules (transfer limits, country
  restrictions, holding periods) do not run for them — only the two checks above. Module rules do run
  when SY is deposited or withdrawn. Partial freezes (`freeze_partial_tokens`) restrict the underlying
  balance, not derived positions.
* **SEP-57 is a Draft** (v0.4.0 at the time of writing). The adapter isolates every SEP-57 assumption
  in one crate (`RwaTokenInterface`, `IdentityVerifierInterface`) so a change to the standard is a
  one-file change.
* **The authority probe emits the token's own events/hooks** for an idempotent `set_address_frozen`,
  and depends on the token honoring RBAC on `operator`. A token whose `set_address_frozen` is
  unrestricted would let anyone pass; that is a token defect, not something an integrator can
  compensate for. A future SEP-57 revision that exposes an explicit role-check view would replace the
  probe.
* `forced_transfer` / `recover_balance` act on the **underlying** balance. They do not move derived
  positions: an account whose underlying was force-moved still holds its PT/YT/LP until it is frozen
  and seized through the escrow.

---

## 4. Optional narrowing: `Permissioning`

Independent of the underlying, a market administrator may run a `Permissioning` contract (per-account
and per-asset allow-lists, e.g. PT open to a wider audience than YT). It can only **narrow**
eligibility: every check requires both the underlying's authorization *and* the allow-list entry,
never either alone. It is not a registry Principal controls.

## 5. Recovery, end to end

```
issuer deauthorizes the account on the underlying          (SAC set_authorized(false) / RWA freeze)
        │
        ▼
RecoveryEscrow.seize_*  / seize_batch / seize_all_positions      caller = live issuer authority
        │   (target must already be deauthorized — the issuer's key alone is not a drain)
        ├── SY  ──► seized, unwrapped at once ─────────────► raw underlying held by the escrow
        ├── LP  ──► seized, burned for PT + SY; SY leg unwrapped at once; PT leg held
        └── PT/YT ─► held fully backed until maturity ──► finalize_record ──► raw underlying
        │
        ▼
one RecoveryRecord per account per event: what was seized, what it yielded, when (ledger + time)
        │
        ▼
issuer finishes with the underlying's native mechanism    (SAC clawback / RWA forced_transfer)
```

The escrow has no key of its own; it re-reads the issuer authority on every call. Seizure is
deliberately available while the market is paused, because a legal freeze is often triggered by the
same incident that justifies an operational pause.
