use soroban_sdk::{
    testutils::{Address as _, IssuerFlags},
    token, Address, Env,
};

use principal_permissioning::{PermissioningContract, PermissioningContractClient};
use principal_risk_control::{RiskControlContract, RiskControlContractClient};

use super::{Error, SYWrapperContract, SYWrapperContractClient, RATE_SCALE};

/// Deploy a minimal mock SAC for testing. `admin` becomes the SAC's real admin, matching
/// how `initialize` now requires SYWrapper's own admin to equal `underlying.admin()`. The
/// issuer's `RevocableFlag` is set so tests can simulate deauthorization
/// (`set_authorized(_, false)`) -- without it, deauthorizing panics with "issuer does not
/// have AUTH_REVOCABLE set" instead of exercising the check under test.
fn deploy_token(env: &Env, admin: &Address) -> Address {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.address()
}

struct Fixture {
    env: Env,
    client: SYWrapperContractClient<'static>,
    admin: Address,
    underlying: Address,
    perm: PermissioningContractClient<'static>,
    perm_admin: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let underlying = deploy_token(&env, &admin);

    let perm_id = env.register_contract(None, PermissioningContract);
    let perm = PermissioningContractClient::new(&env, &perm_id);
    let perm_admin = Address::generate(&env);
    perm.initialize(&perm_admin);

    let wrapper_id = env.register_contract(None, SYWrapperContract);
    let client = SYWrapperContractClient::new(&env, &wrapper_id);
    // admin == underlying's real SAC admin, satisfying the new issuer-match requirement.
    client.initialize(&admin, &underlying, &perm_id);

    Fixture {
        env,
        client,
        admin,
        underlying,
        perm,
        perm_admin,
    }
}

/// Grants both layers: Permissioning (Principal-specific) and SAC authorization (the
/// mandatory floor inherited from the issuer). Most tests want both cleared.
fn grant(f: &Fixture, user: &Address) {
    f.perm.grant_account(&f.perm_admin, user);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(user, &true);
}

fn mint(env: &Env, token: &Address, _admin: &Address, to: &Address, amount: i128) {
    let tok = token::StellarAssetClient::new(env, token);
    tok.mint(to, &amount);
}

/// A minimal RecoveryEscrow-*shaped* contract exposing only `underlying_address()` -- enough to
/// exercise `set_recovery_escrow`'s self-referential cross-check without standing up a full
/// RecoveryEscrow (which itself requires PT/YT/PrincipalManager/MarketPool all deployed and
/// wired together). Used in place of a bare `Address::generate` wherever a test needs a
/// plausible escrow address that the new cross-check will actually call into.
mod mock_escrow {
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

    #[contracttype]
    enum DataKey {
        Underlying,
    }

    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn init(env: Env, underlying: Address) {
            env.storage()
                .instance()
                .set(&DataKey::Underlying, &underlying);
        }

        pub fn underlying_address(env: Env) -> Address {
            env.storage().instance().get(&DataKey::Underlying).unwrap()
        }
    }
}

/// Deploy a `mock_escrow` reporting `underlying` as its own underlying address.
fn deploy_escrow_mock(env: &Env, underlying: &Address) -> Address {
    let id = env.register_contract(None, mock_escrow::MockEscrow);
    mock_escrow::MockEscrowClient::new(env, &id).init(underlying);
    id
}

#[test]
#[should_panic]
fn initialize_rejects_admin_not_matching_sac_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let real_sac_admin = Address::generate(&env);
    let underlying = deploy_token(&env, &real_sac_admin);
    let perm_id = env.register_contract(None, PermissioningContract);
    PermissioningContractClient::new(&env, &perm_id).initialize(&real_sac_admin);

    let impostor = Address::generate(&env);
    let wrapper_id = env.register_contract(None, SYWrapperContract);
    let client = SYWrapperContractClient::new(&env, &wrapper_id);
    // impostor is not the underlying SAC's admin -- market creation must be rejected.
    client.initialize(&impostor, &underlying, &perm_id);
}

#[test]
fn deposit_and_exchange_rate() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);
    // First depositor: shares == underlying (1:1).
    assert_eq!(shares, 1_000_000_000);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE);
    assert_eq!(f.client.balance_of(&user), 1_000_000_000);
}

#[test]
#[should_panic]
fn deposit_without_permissioning_grant_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    // SAC-authorized but never granted in Principal's own Permissioning layer.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &true);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    f.client.deposit(&user, &1_000_000_000_i128, &0);
}

#[test]
#[should_panic]
fn deposit_without_sac_authorization_panics() {
    // Granted in Principal's own Permissioning, but explicitly deauthorized on the
    // underlying SAC itself (e.g. the issuer never cleared them, or revoked them) -- the
    // mandatory floor inherited from the issuer must still block this regardless of
    // Principal's own Permissioning state. (A freshly-registered test SAC defaults every
    // address to authorized=true, matching real unrestricted-asset semantics, so this test
    // explicitly deauthorizes rather than relying on an unset default.)
    let f = setup();
    let user = Address::generate(&f.env);
    f.perm.grant_account(&f.perm_admin, &user);
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    f.client.deposit(&user, &1_000_000_000_i128, &0);
}

#[test]
#[should_panic]
fn deauthorized_on_sac_cannot_front_run_seizure_by_self_withdrawing() {
    // If withdraw only checked `to`, an investor the issuer just deauthorized on the SAC
    // could cash out the instant they suspected a seizure was coming.
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    // Issuer deauthorizes the account directly on the underlying SAC.
    token::StellarAssetClient::new(&f.env, &f.underlying).set_authorized(&user, &false);
    f.client.withdraw(&user, &shares, &user, &0);
}

#[test]
fn withdraw_returns_underlying() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

    let shares = f.client.deposit(&user, &500_000_000_i128, &0);
    let out = f.client.withdraw(&user, &shares, &user, &0);
    assert_eq!(out, 500_000_000);
    assert_eq!(f.client.total_shares(), 0);
}

#[test]
#[should_panic]
fn withdraw_to_unpermitted_recipient_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

    let shares = f.client.deposit(&user, &500_000_000_i128, &0);
    f.client.withdraw(&user, &shares, &stranger, &0); // stranger never granted
}

#[test]
#[should_panic]
fn withdraw_more_than_balance_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.deposit(&user, &100_000_000_i128, &0);
    f.client.withdraw(&user, &200_000_000_i128, &user, &0);
}

/// Regression: a negative `baseline` previously inflated `shares = balance_of(from) - baseline`
/// to more than `from` ever held, with no check anywhere on the path rejecting it -- an eligible
/// account with *zero* SY of its own could withdraw other depositors' pooled underlying (bounded
/// only by the wrapper's real token balance), corrupt `TotalShares`/`TotalUnderlying`, and drive
/// its own stored balance negative. Found during a follow-up review after this contract's own
/// first audit pass; `withdraw_gained` must reject `baseline < 0` outright.
#[test]
#[should_panic]
fn withdraw_gained_rejects_a_negative_baseline() {
    let f = setup();
    // A victim deposits first, so there is real pooled underlying for an exploit to target.
    let victim = Address::generate(&f.env);
    grant(&f, &victim);
    mint(&f.env, &f.underlying, &f.admin, &victim, 1_000_000_000);
    f.client.deposit(&victim, &1_000_000_000_i128, &0);

    // The attacker holds no SY of their own at all.
    let attacker = Address::generate(&f.env);
    grant(&f, &attacker);
    f.client
        .withdraw_gained(&attacker, &-500_000_000_i128, &attacker, &0);
}

/// Same as above at `i128::MIN`, the most adversarial input: the fix must reject it outright
/// before any arithmetic on it is attempted (computing `balance - baseline` at this value would
/// itself overflow `i128`, since `i128` cannot represent `-i128::MIN`).
#[test]
#[should_panic]
fn withdraw_gained_rejects_i128_min_baseline_without_overflow() {
    let f = setup();
    let victim = Address::generate(&f.env);
    grant(&f, &victim);
    mint(&f.env, &f.underlying, &f.admin, &victim, 1_000_000_000);
    f.client.deposit(&victim, &1_000_000_000_i128, &0);

    let attacker = Address::generate(&f.env);
    grant(&f, &attacker);
    f.client
        .withdraw_gained(&attacker, &i128::MIN, &attacker, &0);
}

/// A negative-baseline call must leave every piece of state untouched, not merely panic --
/// otherwise a partial-effects bug could still leak value even while the call itself reverts.
/// Soroban rolls back all storage writes on a panicking invocation, but this pins that guarantee
/// specifically for this path rather than relying on the runtime's rollback semantics alone.
#[test]
fn withdraw_gained_negative_baseline_leaves_state_unchanged() {
    let f = setup();
    let victim = Address::generate(&f.env);
    grant(&f, &victim);
    mint(&f.env, &f.underlying, &f.admin, &victim, 1_000_000_000);
    f.client.deposit(&victim, &1_000_000_000_i128, &0);

    let attacker = Address::generate(&f.env);
    grant(&f, &attacker);
    assert_eq!(
        f.client
            .try_withdraw_gained(&attacker, &-500_000_000_i128, &attacker, &0)
            .err(),
        Some(Ok(soroban_sdk::Error::from_contract_error(
            Error::InsufficientShares as u32
        )))
    );

    assert_eq!(f.client.total_shares(), 1_000_000_000);
    assert_eq!(f.client.total_underlying(), 1_000_000_000);
    assert_eq!(f.client.balance_of(&victim), 1_000_000_000);
    assert_eq!(f.client.balance_of(&attacker), 0);
    assert_eq!(
        token::Client::new(&f.env, &f.underlying).balance(&attacker),
        0
    );
}

/// Defense-in-depth: even granting the attacker a small real SY balance of their own, a baseline
/// negative enough to claim more than that balance must still be rejected -- the bug was not
/// merely "zero-balance attacker", it was "`shares` can exceed balance at all".
#[test]
#[should_panic]
fn withdraw_gained_rejects_negative_baseline_even_with_a_real_balance() {
    let f = setup();
    let victim = Address::generate(&f.env);
    grant(&f, &victim);
    mint(&f.env, &f.underlying, &f.admin, &victim, 1_000_000_000);
    f.client.deposit(&victim, &1_000_000_000_i128, &0);

    let attacker = Address::generate(&f.env);
    grant(&f, &attacker);
    mint(&f.env, &f.underlying, &f.admin, &attacker, 10_000_000);
    f.client.deposit(&attacker, &10_000_000_i128, &0);

    // shares = 10_000_000 - (-500_000_000) = 510_000_000, far above the attacker's real balance.
    f.client
        .withdraw_gained(&attacker, &-500_000_000_i128, &attacker, &0);
}

/// The legitimate case must keep working: a genuine positive gain above a real prior baseline is
/// still withdrawable.
#[test]
fn withdraw_gained_accepts_a_legitimate_positive_gain() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    let baseline = f.client.deposit(&user, &1_000_000_000_i128, &0);

    mint(&f.env, &f.underlying, &f.admin, &user, 200_000_000);
    f.client.deposit(&user, &200_000_000_i128, &0);

    let out = f.client.withdraw_gained(&user, &baseline, &user, &0);
    assert_eq!(out, 200_000_000);
    assert_eq!(f.client.balance_of(&user), baseline);
}

/// `withdraw_all` exists so a caller (in practice, a Router function chaining a curve-derived
/// swap payout straight into a withdrawal) never needs to pass an exact `shares` amount that
/// could drift between simulation and execution -- see its own doc comment for why. This pins
/// its basic behavior: it takes the caller's whole balance, not a chosen amount.
#[test]
fn withdraw_all_takes_the_entire_balance() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);

    f.client.deposit(&user, &500_000_000_i128, &0);
    let out = f.client.withdraw_all(&user, &user, &0);
    assert_eq!(out, 500_000_000);
    assert_eq!(f.client.total_shares(), 0);
    assert_eq!(f.client.balance_of(&user), 0);
}

#[test]
#[should_panic]
fn withdraw_all_with_zero_balance_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.withdraw_all(&user, &user, &0); // never deposited -- zero balance
}

#[test]
#[should_panic]
fn withdraw_all_to_unpermitted_recipient_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    f.client.deposit(&user, &500_000_000_i128, &0);
    f.client.withdraw_all(&user, &stranger, &0); // stranger never granted
}

#[test]
#[should_panic]
fn deposit_while_paused_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.set_paused(&f.admin, &true);
    f.client.deposit(&user, &100_000_000_i128, &0);
}

#[test]
fn unpause_re_enables_deposits() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 200_000_000);

    f.client.set_paused(&f.admin, &true);
    f.client.set_paused(&f.admin, &false); // unpause
    let shares = f.client.deposit(&user, &200_000_000_i128, &0);
    assert!(shares > 0);
}

#[test]
fn exchange_rate_stays_at_inception_rate() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE); // 1:1 at inception

    let user2 = Address::generate(&f.env);
    grant(&f, &user2);
    mint(&f.env, &f.underlying, &f.admin, &user2, 500_000_000);
    f.client.deposit(&user2, &500_000_000_i128, &0);
    assert_eq!(f.client.exchange_rate(), RATE_SCALE); // still 1:1

    assert_eq!(f.client.total_underlying(), 1_500_000_000);
    assert_eq!(f.client.total_shares(), 1_500_000_000);
    let _ = shares;
}

#[test]
#[should_panic]
fn zero_deposit_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    f.client.deposit(&user, &0_i128, &0);
}

#[test]
fn balance_of_returns_correct_shares() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 300_000_000);
    f.client.deposit(&user, &300_000_000_i128, &0);
    assert_eq!(f.client.balance_of(&user), 300_000_000);
}

#[test]
fn admin_transfer() {
    let f = setup();
    let new_admin = Address::generate(&f.env);
    f.client.transfer_admin(&f.admin, &new_admin);
    assert_eq!(f.client.get_admin(), new_admin);
}

// --- share transfer ---

#[test]
fn transfer_moves_shares_between_eligible_accounts() {
    let f = setup();
    let user = Address::generate(&f.env);
    let recipient = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &recipient);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    let moved = f.client.transfer(&user, &recipient, &shares);
    assert_eq!(moved, shares);
    assert_eq!(f.client.balance_of(&user), 0);
    assert_eq!(f.client.balance_of(&recipient), shares);
    // Pure balance move: total shares/underlying unaffected.
    assert_eq!(f.client.total_shares(), shares);
}

#[test]
#[should_panic]
fn transfer_to_unpermitted_recipient_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 500_000_000);
    let shares = f.client.deposit(&user, &500_000_000_i128, &0);

    f.client.transfer(&user, &stranger, &shares); // stranger never granted
}

#[test]
#[should_panic]
fn transfer_more_than_balance_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    let recipient = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &recipient);
    mint(&f.env, &f.underlying, &f.admin, &user, 100_000_000);
    f.client.deposit(&user, &100_000_000_i128, &0);

    f.client.transfer(&user, &recipient, &200_000_000_i128);
}

// --- seize (compliance recovery) ---

#[test]
fn set_recovery_escrow_once() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    assert_eq!(f.client.recovery_escrow(), escrow);
}

#[test]
#[should_panic]
fn set_recovery_escrow_twice_panics() {
    let f = setup();
    let escrow1 = deploy_escrow_mock(&f.env, &f.underlying);
    let escrow2 = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow1);
    // The second call panics on `RecoveryEscrowAlreadySet` before it ever calls back into
    // `escrow2`, so `escrow2` can stay a bare, non-contract address here.
    f.client.set_recovery_escrow(&f.admin, &escrow2);
}

#[test]
#[should_panic]
fn set_recovery_escrow_rejects_an_escrow_configured_for_a_different_underlying() {
    // Found during audit review: this wiring is one-shot and irreversible, so an admin mistake
    // (a stale escrow address copied from a different market) previously had no cross-check at
    // all. This mock reports a *different* underlying than this wrapper's own -- mirroring
    // `RecoveryEscrow::initialize`'s own cross-check of each position's `underlying_address()`,
    // the wrapper must now reject it.
    let f = setup();
    let other_underlying = Address::generate(&f.env);
    let wrong_escrow = deploy_escrow_mock(&f.env, &other_underlying);
    f.client.set_recovery_escrow(&f.admin, &wrong_escrow);
}

#[test]
fn seize_moves_balance_to_escrow_without_holder_auth() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    let innocent = Address::generate(&f.env);
    grant(&f, &bad_actor);
    grant(&f, &innocent);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 1_000_000_000);
    mint(&f.env, &f.underlying, &f.admin, &innocent, 1_000_000_000);
    f.client.deposit(&bad_actor, &1_000_000_000_i128, &0);
    f.client.deposit(&innocent, &1_000_000_000_i128, &0);

    let seized = f.client.seize(&escrow, &bad_actor, &1_000_000_000_i128);
    assert_eq!(seized, 1_000_000_000);

    // Balance moved to the escrow; total shares unaffected (forced transfer, not a burn);
    // innocent depositor untouched.
    assert_eq!(f.client.balance_of(&bad_actor), 0);
    assert_eq!(f.client.balance_of(&escrow), 1_000_000_000);
    assert_eq!(f.client.balance_of(&innocent), 1_000_000_000);
    assert_eq!(f.client.total_shares(), 2_000_000_000);
}

#[test]
#[should_panic]
fn seize_requires_configured_escrow_caller() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    let impostor = Address::generate(&f.env);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&impostor, &bad_actor, &500_000_000_i128);
}

#[test]
#[should_panic]
fn seize_cannot_exceed_target_balance() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&escrow, &bad_actor, &600_000_000_i128);
}

#[test]
fn seize_works_while_paused() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.set_paused(&f.admin, &true);
    let seized = f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
    assert_eq!(seized, 500_000_000);
}

#[test]
fn escrow_can_unwrap_seized_sy_via_normal_withdraw() {
    // End-to-end: seize, then the escrow -- pre-authorized on both layers, same as any
    // legitimate holder -- unwraps into raw underlying via the ordinary withdraw path.
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    grant(&f, &escrow); // issuer + Principal both pre-authorize the escrow

    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    mint(&f.env, &f.underlying, &f.admin, &bad_actor, 500_000_000);
    f.client.deposit(&bad_actor, &500_000_000_i128, &0);

    f.client.seize(&escrow, &bad_actor, &500_000_000_i128);
    let unwrapped = f.client.withdraw(&escrow, &500_000_000_i128, &escrow, &0);
    assert_eq!(unwrapped, 500_000_000);

    let underlying_client = token::Client::new(&f.env, &f.underlying);
    assert_eq!(underlying_client.balance(&escrow), 500_000_000);
}

#[test]
#[should_panic]
fn double_initialize_panics() {
    let f = setup();
    f.client
        .initialize(&f.admin, &f.underlying, &f.perm.address);
}

#[test]
#[should_panic]
fn seize_rejects_zero_shares() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    let bad_actor = Address::generate(&f.env);
    grant(&f, &bad_actor);
    f.client.seize(&escrow, &bad_actor, &0_i128);
}

#[test]
fn exchange_rate_before_any_deposit_is_rate_scale() {
    let f = setup();
    assert_eq!(f.client.exchange_rate(), RATE_SCALE);
    assert_eq!(f.client.total_underlying(), 0);
    assert_eq!(f.client.total_shares(), 0);
}

#[test]
#[should_panic]
fn non_admin_cannot_set_paused() {
    let f = setup();
    let impostor = Address::generate(&f.env);
    f.client.set_paused(&impostor, &true);
}

#[test]
fn admin_transfer_and_getters() {
    let f = setup();
    let escrow = deploy_escrow_mock(&f.env, &f.underlying);
    f.client.set_recovery_escrow(&f.admin, &escrow);
    assert_eq!(f.client.get_admin(), f.admin);
    assert_eq!(f.client.underlying_address(), f.underlying);
    assert_eq!(f.client.permissioning_address(), f.perm.address);
    assert_eq!(f.client.recovery_escrow(), escrow);

    let new_admin = Address::generate(&f.env);
    f.client.transfer_admin(&f.admin, &new_admin);
    assert_eq!(f.client.get_admin(), new_admin);
}

// ====================================================================== slippage / cap / risk-control
// Coverage gap found during an audit review: none of these three documented security controls
// (front-running protection, per-address exposure limits, the protocol-wide circuit breaker) had
// any test at all -- every deposit/withdraw call elsewhere in this file passes `0` for the min-out
// parameter, and `set_deposit_cap`/`set_risk_control` were never exercised.

#[test]
fn deposit_slippage_exact_minimum_passes_one_more_reverts() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);

    // 1:1 at the first deposit, so the exact minimum is the deposited amount itself.
    assert_eq!(
        f.client
            .try_deposit(&user, &1_000_000_000_i128, &1_000_000_000_i128)
            .err(),
        None
    );
}

#[test]
#[should_panic]
fn deposit_slippage_one_above_the_real_output_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    f.client
        .deposit(&user, &1_000_000_000_i128, &1_000_000_001_i128);
}

#[test]
fn withdraw_slippage_exact_minimum_passes_one_more_reverts() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);

    assert_eq!(
        f.client
            .try_withdraw(&user, &shares, &user, &1_000_000_000_i128)
            .err(),
        None
    );
}

#[test]
#[should_panic]
fn withdraw_slippage_one_above_the_real_output_panics() {
    let f = setup();
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    let shares = f.client.deposit(&user, &1_000_000_000_i128, &0);
    f.client
        .withdraw(&user, &shares, &user, &1_000_000_001_i128);
}

#[test]
fn deposit_cap_boundary_exactly_at_the_cap_passes_and_one_more_reverts() {
    let f = setup();
    f.client.set_deposit_cap(&f.admin, &1_000_i128);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 2_000);

    assert_eq!(f.client.deposit(&user, &1_000_i128, &0), 1_000);
    assert_eq!(
        f.client.try_deposit(&user, &1_i128, &0).err(),
        Some(Ok(soroban_sdk::Error::from_contract_error(
            Error::DepositCapExceeded as u32
        )))
    );
}

#[test]
fn deposit_cap_is_released_on_withdraw_so_a_fresh_deposit_up_to_the_cap_succeeds_again() {
    let f = setup();
    f.client.set_deposit_cap(&f.admin, &1_000_i128);
    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 3_000);

    f.client.deposit(&user, &1_000_i128, &0);
    assert_eq!(
        f.client.try_deposit(&user, &1_i128, &0).err(),
        Some(Ok(soroban_sdk::Error::from_contract_error(
            Error::DepositCapExceeded as u32
        )))
    );

    // Withdrawing releases net-deposited headroom, not just the cap check itself.
    f.client.withdraw(&user, &500_i128, &user, &0);
    assert_eq!(f.client.deposit(&user, &500_i128, &0), 500);
    // Back at the cap again.
    assert_eq!(
        f.client.try_deposit(&user, &1_i128, &0).err(),
        Some(Ok(soroban_sdk::Error::from_contract_error(
            Error::DepositCapExceeded as u32
        )))
    );
}

// ======================================================================== withdraw_crediting
// Found during audit review: a custodian contract (e.g. `PrincipalManager`) that holds a user's
// SY in its own name between taking custody and unwinding it later must release the user's own
// deposit cap, not its own -- plain `withdraw` always credits `from`, which is the custodian's
// own address in that case, a no-op against an address the cap was never charged on. These
// confirm `withdraw_crediting` actually credits `beneficiary`, independent of both `from` and
// `to`.

#[test]
fn withdraw_crediting_releases_the_named_beneficiarys_cap_not_froms() {
    let f = setup();
    f.client.set_deposit_cap(&f.admin, &1_000_i128);

    // `user` deposits (charging its own cap), then moves its SY into `custodian`'s own name via
    // a plain transfer -- mirroring `PrincipalManager.mint`, which never touches the cap.
    let user = Address::generate(&f.env);
    let custodian = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &custodian);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000);
    let shares = f.client.deposit(&user, &1_000_i128, &0);
    f.client.transfer(&user, &custodian, &shares);

    // `user` is now capped out even though it holds no SY at all anymore.
    assert_eq!(f.client.net_deposited(&user), 1_000);
    assert_eq!(
        f.client.try_deposit(&user, &1_i128, &0).err(),
        Some(Ok(soroban_sdk::Error::from_contract_error(
            Error::DepositCapExceeded as u32
        )))
    );

    // `custodian` unwinds the custodied SY back to `user`, naming `user` as the beneficiary of
    // the cap release -- exactly what `PrincipalManager.redeem`/`pay_yield` do.
    f.client
        .withdraw_crediting(&custodian, &shares, &user, &0, &user);

    // `user`'s cap is released, not `custodian`'s (which was never charged in the first place).
    assert_eq!(f.client.net_deposited(&user), 0);
    assert_eq!(f.client.net_deposited(&custodian), 0);
    assert_eq!(f.client.deposit(&user, &1_000_i128, &0), 1_000);
}

#[test]
fn plain_withdraw_by_a_custodian_would_not_have_released_the_real_depositors_cap() {
    // Locks in the bug `withdraw_crediting` exists to fix: if a custodian uses plain `withdraw`
    // instead (crediting itself, `from`), the real depositor's cap stays stuck even though it
    // now holds no SY at all.
    let f = setup();
    f.client.set_deposit_cap(&f.admin, &1_000_i128);
    let user = Address::generate(&f.env);
    let custodian = Address::generate(&f.env);
    grant(&f, &user);
    grant(&f, &custodian);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000);
    let shares = f.client.deposit(&user, &1_000_i128, &0);
    f.client.transfer(&user, &custodian, &shares);

    f.client.withdraw(&custodian, &shares, &user, &0);

    assert_eq!(f.client.net_deposited(&user), 1_000);
    assert!(f.client.try_deposit(&user, &1_i128, &0).is_err());
}

fn setup_risk_control(f: &Fixture) -> RiskControlContractClient<'static> {
    let rc_id = f.env.register_contract(None, RiskControlContract);
    let rc = RiskControlContractClient::new(&f.env, &rc_id);
    rc.initialize(&f.admin, &(10_000 * 1_000_000_000_i128));
    rc
}

#[test]
#[should_panic]
fn set_risk_control_rejects_an_address_that_is_not_a_risk_control_contract() {
    // Found during audit review: `set_risk_control` wired in the admin-supplied address with
    // no cross-check at all. `f.perm.address` is a real deployed contract, but not
    // RiskControl-shaped: it has no `is_consumer` entry point, so the cross-check call traps
    // instead of silently wiring in a copy-pasted address of the wrong contract type.
    let f = setup();
    f.client.set_risk_control(&f.admin, &f.perm.address);
}

#[test]
fn set_risk_control_accepts_a_real_risk_control_contract() {
    let f = setup();
    let rc = setup_risk_control(&f);
    f.client.set_risk_control(&f.admin, &rc.address);
    assert_eq!(f.client.risk_control(), Some(rc.address));
}

#[test]
fn a_wired_risk_control_that_is_not_a_registered_consumer_fails_closed() {
    // "a wired wrapper that is not a registered consumer reverts" -- documented in
    // `docs/SECURITY.md`; this is the only place that property was actually exercised.
    let f = setup();
    let rc = setup_risk_control(&f);
    f.client.set_risk_control(&f.admin, &rc.address);
    // Deliberately never registered as a consumer.

    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 1_000_000_000);
    assert!(f
        .client
        .try_deposit(&user, &1_000_000_000_i128, &0)
        .is_err());
}

#[test]
fn risk_control_blocks_a_deposit_that_would_exceed_the_shared_limit() {
    let f = setup();
    let rc = setup_risk_control(&f);
    f.client.set_risk_control(&f.admin, &rc.address);
    rc.add_consumer(&f.admin, &f.client.address);
    rc.set_cb_limit(&f.admin, &1_000_i128);

    let user = Address::generate(&f.env);
    grant(&f, &user);
    mint(&f.env, &f.underlying, &f.admin, &user, 2_000);

    assert_eq!(f.client.deposit(&user, &1_000_i128, &0), 1_000);
    assert_eq!(rc.get_cb_volume(), 1_000);
    // One more unit trips the shared breaker -- and the deposit itself reverts, atomically, with
    // no separate manual call needed.
    assert!(f.client.try_deposit(&user, &1_i128, &0).is_err());
    assert_eq!(f.client.balance_of(&user), 1_000);
    assert_eq!(rc.get_cb_volume(), 1_000);
}
