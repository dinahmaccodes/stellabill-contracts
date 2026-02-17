use crate::{Subscription, SubscriptionStatus, SubscriptionVault, SubscriptionVaultClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

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
