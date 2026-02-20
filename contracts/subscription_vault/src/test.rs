use crate::{Error, Subscription, SubscriptionStatus, SubscriptionVault, SubscriptionVaultClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{Address, Env};

const INTERVAL: u64 = 30 * 24 * 60 * 60; // 30 days
const T0: u64 = 1_700_000_000;

/// Helper: register contract, init, and create one Active subscription at timestamp T0.
fn setup(env: &Env, interval: u64) -> (SubscriptionVaultClient, u32) {
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(env, &contract_id);

    let token = Address::generate(env);
    let admin = Address::generate(env);
    client.init(&token, &admin);

    let subscriber = Address::generate(env);
    let merchant = Address::generate(env);

    env.ledger().set_timestamp(T0);
    let id = client.create_subscription(&subscriber, &merchant, &10_000_000i128, &interval, &false);
    (client, id)
}

#[test]
fn test_init_and_struct() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);
    // TODO: add create_subscription test with mock token
}

#[test]
fn test_subscription_struct() {
    let env = Env::default();
    let sub = Subscription {
        subscriber: Address::generate(&env),
        merchant: Address::generate(&env),
        amount: 10_000_0000, // 10 USDC (6 decimals)
        interval_seconds: 30 * 24 * 60 * 60, // 30 days
        last_payment_timestamp: 0,
        status: SubscriptionStatus::Active,
        prepaid_balance: 50_000_0000,
        usage_enabled: false,
    };
    assert_eq!(sub.status, SubscriptionStatus::Active);
}

// -- Billing interval enforcement tests --------------------------------------

/// Just-before: charge 1 second before the interval elapses.
/// Must reject with IntervalNotElapsed and leave storage untouched.
#[test]
fn test_charge_rejected_before_interval() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, INTERVAL);

    // 1 second too early.
    env.ledger().set_timestamp(T0 + INTERVAL - 1);

    let res = client.try_charge_subscription(&id);
    assert_eq!(res, Err(Ok(Error::IntervalNotElapsed)));

    // Storage unchanged — last_payment_timestamp still equals creation time.
    let sub = client.get_subscription(&id);
    assert_eq!(sub.last_payment_timestamp, T0);
}

/// Exact boundary: charge at exactly last_payment_timestamp + interval_seconds.
/// Must succeed and advance last_payment_timestamp.
#[test]
fn test_charge_succeeds_at_exact_interval() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, INTERVAL);

    env.ledger().set_timestamp(T0 + INTERVAL);
    client.charge_subscription(&id);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.last_payment_timestamp, T0 + INTERVAL);
}

/// After interval: charge well past the interval boundary.
/// Must succeed and set last_payment_timestamp to the current ledger time.
#[test]
fn test_charge_succeeds_after_interval() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, INTERVAL);

    let charge_time = T0 + 2 * INTERVAL;
    env.ledger().set_timestamp(charge_time);
    client.charge_subscription(&id);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.last_payment_timestamp, charge_time);
}

// -- Edge cases: boundary timestamps & repeated calls ------------------------
//
// Assumptions about ledger time monotonicity:
//   Soroban ledger timestamps are set by validators and are expected to be
//   non-decreasing across ledger closes (~5-6 s on mainnet). The contract
//   does NOT assume strict monotonicity — it only requires
//   `now >= last_payment_timestamp + interval_seconds`. If a validator were
//   to produce a timestamp equal to the previous ledger's (same second), the
//   charge would simply be rejected as the interval cannot have elapsed in
//   zero additional seconds. The contract never relies on `now > previous_now`.

/// Same-timestamp retry: a second charge at the identical timestamp that
/// succeeded must be rejected because 0 seconds < interval_seconds.
#[test]
fn test_immediate_retry_at_same_timestamp_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, INTERVAL);

    let t1 = T0 + INTERVAL;
    env.ledger().set_timestamp(t1);
    client.charge_subscription(&id);

    // Retry at the same timestamp — must fail, storage stays at t1.
    let res = client.try_charge_subscription(&id);
    assert_eq!(res, Err(Ok(Error::IntervalNotElapsed)));

    let sub = client.get_subscription(&id);
    assert_eq!(sub.last_payment_timestamp, t1);
}

/// Repeated charges across 6 consecutive intervals.
/// Verifies the sliding-window reset works correctly over many cycles.
#[test]
fn test_repeated_charges_across_many_intervals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, INTERVAL);

    for i in 1..=6u64 {
        let charge_time = T0 + i * INTERVAL;
        env.ledger().set_timestamp(charge_time);
        client.charge_subscription(&id);

        let sub = client.get_subscription(&id);
        assert_eq!(sub.last_payment_timestamp, charge_time);
    }

    // One more attempt without advancing time — must fail.
    let res = client.try_charge_subscription(&id);
    assert_eq!(res, Err(Ok(Error::IntervalNotElapsed)));
}

/// Minimum interval (1 second): charge at creation time must fail,
/// charge 1 second later must succeed.
#[test]
fn test_one_second_interval_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup(&env, 1);

    // At creation time — 0 seconds elapsed, interval is 1 s → too early.
    env.ledger().set_timestamp(T0);
    let res = client.try_charge_subscription(&id);
    assert_eq!(res, Err(Ok(Error::IntervalNotElapsed)));

    // Exactly 1 second later — boundary, should succeed.
    env.ledger().set_timestamp(T0 + 1);
    client.charge_subscription(&id);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.last_payment_timestamp, T0 + 1);
}

// -- Usage-based charge tests ------------------------------------------------

const PREPAID: i128 = 50_000_000; // 50 USDC

/// Helper: create a subscription with `usage_enabled = true` and a known
/// `prepaid_balance` by writing directly to storage after creation.
fn setup_usage(env: &Env) -> (SubscriptionVaultClient, u32) {
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(env, &contract_id);

    let token = Address::generate(env);
    let admin = Address::generate(env);
    client.init(&token, &admin);

    let subscriber = Address::generate(env);
    let merchant = Address::generate(env);

    env.ledger().set_timestamp(T0);
    let id = client.create_subscription(
        &subscriber,
        &merchant,
        &10_000_000i128,
        &INTERVAL,
        &true, // usage_enabled
    );

    // Seed prepaid balance by writing the subscription back with funds.
    let mut sub = client.get_subscription(&id);
    sub.prepaid_balance = PREPAID;
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&id, &sub);
    });

    (client, id)
}

/// Successful usage charge: debits prepaid_balance by the requested amount.
#[test]
fn test_usage_charge_debits_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup_usage(&env);

    client.charge_usage(&id, &10_000_000i128);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.prepaid_balance, PREPAID - 10_000_000);
    assert_eq!(sub.status, SubscriptionStatus::Active);
}

/// Draining the balance to zero transitions status to InsufficientBalance.
#[test]
fn test_usage_charge_drains_balance_to_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup_usage(&env);

    client.charge_usage(&id, &PREPAID);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.prepaid_balance, 0);
    assert_eq!(sub.status, SubscriptionStatus::InsufficientBalance);
}

/// Rejected when usage_enabled is false.
#[test]
fn test_usage_charge_rejected_when_disabled() {
    let env = Env::default();
    env.mock_all_auths();
    // Use the regular setup helper which creates usage_enabled = false.
    let (client, id) = setup(&env, INTERVAL);

    let res = client.try_charge_usage(&id, &1_000_000i128);
    assert_eq!(res, Err(Ok(Error::UsageNotEnabled)));
}

/// Rejected when usage_amount exceeds prepaid_balance.
#[test]
fn test_usage_charge_rejected_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup_usage(&env);

    let res = client.try_charge_usage(&id, &(PREPAID + 1));
    assert_eq!(res, Err(Ok(Error::InsufficientPrepaidBalance)));

    // Balance unchanged.
    let sub = client.get_subscription(&id);
    assert_eq!(sub.prepaid_balance, PREPAID);
}

/// Rejected when usage_amount is zero or negative.
#[test]
fn test_usage_charge_rejected_invalid_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, id) = setup_usage(&env);

    let res_zero = client.try_charge_usage(&id, &0i128);
    assert_eq!(res_zero, Err(Ok(Error::InvalidAmount)));

    let res_neg = client.try_charge_usage(&id, &(-1i128));
    assert_eq!(res_neg, Err(Ok(Error::InvalidAmount)));

    // Balance unchanged.
    let sub = client.get_subscription(&id);
    assert_eq!(sub.prepaid_balance, PREPAID);
}
