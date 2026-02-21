use crate::{Subscription, SubscriptionStatus, SubscriptionVault, SubscriptionVaultClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, IntoVal};

#[test]
fn test_init_and_struct() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);
}

#[test]
fn test_charge_subscription_auth() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);

    // Test authorized call
    env.mock_all_auths();
    client.charge_subscription(&0);
}

#[test]
#[should_panic] // Soroban panic on require_auth failure
fn test_charge_subscription_unauthorized() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);

    let non_admin = Address::generate(&env);

    // Mock auth for the non_admin address
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "charge_subscription",
            args: (0u32,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.charge_subscription(&0);
}

#[test]
fn test_charge_subscription_admin() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);

    // Mock auth for the admin address
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "charge_subscription",
            args: (0u32,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.charge_subscription(&0);
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
