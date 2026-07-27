# Phase 2 Design — Compliant PT/YT Tokens and Settlement Extensions

Scope for this branch (`feature/phase2-compliant-pt-yt-settlement`), matching the deliverables committed to in `PRODUCT_CONCEPT_AND_COMPETITOR_ANALYSIS.md` §4/§6:

1. `PTToken` — standalone SEP-41 Principal Token.
2. `YTToken` — standalone SEP-41 Yield Token with claimable yield.
3. Close the compliance gap found during audit: `Permissioning` checks extended to `SYWrapper.deposit`/`withdraw` and `PrincipalManager.redeem`.
4. Clawback Propagation on `SYWrapper`.
5. `LiquidationAdapter` — interface and design only in this pass (depends on an external Blend integration decision the team hasn't made yet; implementing against an assumed interface would be guesswork).

Out of scope for this branch: `MarketPool`, `Router`, asymmetric-permissioning admin tooling beyond the primitive itself. Each gets its own branch once this lands, per TECHNICAL_SPECIFICATION.md §6/§7 sequencing.

---

## 1. PTToken / YTToken — deviation from the original spec, and why

TECHNICAL_SPECIFICATION.md §6 originally specified `transfer` enforcing `Permissioning.is_allowed(to)` — the flat, account-level allow-list already used by `PrincipalManager.mint`.

This implementation uses `Permissioning.is_allowed_for_asset(to, asset_key)` instead, where `asset_key` is the token contract's own address, set at `initialize`. This is a deliberate change, not an oversight:

- It's what makes **asymmetric PT/YT permissioning** possible — PT and YT can carry different eligibility policies (e.g., PT distributed more broadly as a protected-principal note, YT restricted to a narrower speculative audience), which is one of the four originality claims in the resubmission doc.
- It costs nothing new: `Permissioning.is_allowed_for_asset` already exists and is already keyed by `(account, asset: Address)` — see `contracts/permissioning/src/lib.rs:92-117`. Passing the token's own address as the asset key is a wiring decision, not new infrastructure.
- An admin who wants PT and YT to behave identically (symmetric policy) simply grants the same account for both token addresses. Nothing forces divergence; it just becomes possible.

`PTToken` and `YTToken` both also fall back to `Permissioning.is_allowed(to)` (the coarse, protocol-wide gate already used by `mint`) as a first check — an account globally revoked can't hold either instrument regardless of per-asset grants. Per-asset eligibility narrows *within* that broader gate; it doesn't bypass it.

### 1.1 PTToken interface

```rust
fn initialize(env, admin: Address, minter: Address, permissioning: Address, maturity: u64,
              name: String, symbol: String, decimals: u32);
// `minter` is NOT set here — matches the two-phase init in TECHNICAL_SPECIFICATION.md §6.2 to
// break the PrincipalManager <-> PTToken circular dependency. initialize() takes a placeholder
// and set_minter() locks it in once PrincipalManager exists.

fn set_minter(env, admin: Address, minter: Address);   // one-time, reverts if already set

// SEP-41
fn transfer(env, from: Address, to: Address, amount: i128);
fn transfer_from(env, spender: Address, from: Address, to: Address, amount: i128);
fn approve(env, from: Address, spender: Address, amount: i128, expiration_ledger: u32);
fn allowance(env, from: Address, spender: Address) -> i128;
fn balance(env, account: Address) -> i128;
fn decimals(env) -> u32;
fn name(env) -> String;
fn symbol(env) -> String;

// Minter-only
fn mint(env, to: Address, amount: i128);   // caller must equal registered minter
fn burn(env, from: Address, amount: i128); // caller must equal registered minter

// Views
fn total_supply(env) -> i128;
fn maturity(env) -> u64;
fn minter(env) -> Address;
```

Storage: `Admin`, `Minter` (Option, unset until `set_minter`), `Permissioning`, `Maturity`, `TotalSupply` in `instance`; `Balance(Address)`, `Allowance(Address, Address)` in `persistent` with TTL extension on every touch (same `BALANCE_TTL_LEDGERS` constant used elsewhere in the codebase).

Errors: `AlreadyInitialized`, `Unauthorized`, `NotInitialized`, `MinterAlreadySet`, `ZeroAmount`, `InsufficientBalance`, `InsufficientAllowance`, `PermissionDenied`.

### 1.2 YTToken interface

Same as PTToken, plus the yield-index mechanism from TECHNICAL_SPECIFICATION.md §5.5:

```rust
fn initialize(env, admin, minter_placeholder, permissioning, oracle, maturity, name, symbol, decimals);
fn set_minter(env, admin, minter);

// yield accrual
fn update_yield_index(env);              // permissionless — pulls current oracle rate, advances index
fn claim_yield(env, from: Address) -> i128;
fn accrued_yield_index(env) -> i128;
fn last_claimed_index(env, account: Address) -> i128;
```

`claim_yield` computes `yt_balance[from] * (accrued_yield_index - last_claimed_index[from]) / SCALE`, resets the caller's snapshot, and — in this branch — records the claim as an event rather than dispatching an actual underlying transfer, matching the POC's existing convention in `PrincipalManager` ("computed and returned but not dispatched... Phase 2 integration milestone once Router is available"). Wiring the real transfer is a Router-branch concern, not this one.

---

## 2. Closing the compliance gap (audit finding)

Verified during the SCF resubmission audit: `PrincipalManager.redeem()` and `SYWrapper.deposit`/`withdraw` had no `Permissioning` check — only `mint()` did. This branch closes it:

- `SYWrapper.initialize` gains a `permissioning: Address` parameter. `deposit` and `withdraw` call `assert_permitted(&env, &from)` (deposit) and `assert_permitted(&env, &to)` (withdraw), mirroring the existing `assert_permitted` pattern already in `PrincipalManager`.
- `PrincipalManager.redeem()` calls `assert_permitted(&env, &from)` before paying out, same as `mint()` already does.

This is a breaking change to both contracts' `initialize` signatures (`SYWrapper` gains a parameter) — acceptable pre-mainnet, since nothing is deployed yet per PROOF_OF_CONCEPT.md.

---

## 3. Clawback Propagation

New admin-authorized function on `SYWrapper` (the contract that actually holds pooled underlying reserves):

```rust
fn remediate(env, caller: Address, account: Address, shares: i128) -> i128;
```

- `caller.require_auth()`; `caller` must equal `SYWrapper`'s admin (same `assert_admin` pattern already in the contract) — in production this role is expected to be a compliance-multisig the issuer has authorized, not the day-to-day protocol admin key, but that's an operational/deployment decision, not something the contract can enforce on its own.
- Burns exactly `shares` from `Balance(account)` — never more than that account holds, so co-depositors are structurally unaffected.
- Reduces `TotalShares`/`TotalUnderlying` and transfers the equivalent underlying out to `caller` (the compliance role forwards it to the issuer off-chain, or a future extension could take an explicit `to: Address` — kept as `caller` for this pass to minimize new trust assumptions).
- Emits a `remediate` event `(caller, account, shares, underlying_released)` for audit trail.

This only touches `SYWrapper`'s own internal balance — it does not call Stellar's native SAC clawback op, and it doesn't touch `PrincipalManager`'s PT/YT balances directly (those net out naturally: if `PrincipalManager.redeem` is later called against a remediated account's now-reduced SY entitlement, the existing formulas hold without special-casing). PT/YT-side remediation (burning a flagged account's PT/YT balance directly) is left for the `PrincipalManager`-owning branch once `PTToken`/`YTToken` are live and holding real balances, since right now `PrincipalManager` still tracks PT/YT internally.

---

## 4. LiquidationAdapter — design only, not implemented this pass

Interface sketch, per the feasibility audit in the resubmission doc:

```rust
fn initialize(env, admin: Address, principal_manager: Address, market_pool: Address, settlement_asset: Address);
fn liquidate(env, caller: Address, pt_amount: i128, min_settlement_out: i128) -> i128;
```

- `caller` is expected to be a lending pool contract (e.g., a Blend pool) that already custodies the borrower's PT balance under its own address, and self-authorizes the call to `LiquidationAdapter` via `authorize_as_current_contract` — the adapter never needs the original borrower's signature.
- `LiquidationAdapter`'s own address must be granted permanent `Permissioning` eligibility by the issuer/admin — a one-time approval, same category as approving `SYWrapper` itself.
- Internally: redeems (post-maturity) or recombines/swaps (pre-maturity, once `MarketPool` exists) the seized PT into the underlying, then pays the caller out in `settlement_asset` (plain USDC, not the permissioned RWA) at the oracle rate minus a liquidation bonus.

**Why this isn't implemented in this branch:** the pre-maturity path depends on `MarketPool`, which doesn't exist yet, and the exact calling convention depends on which Blend pool type this integrates with — a decision that belongs to whoever owns the Blend relationship, not something to guess at in contract code. Building against an assumed interface now risks a rewrite once that's settled. Tracked as a follow-up branch once `MarketPool`/`Router` land and the Blend integration point is confirmed.
