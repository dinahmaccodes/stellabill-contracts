use crate::{
    Subscription, SubscriptionEntry, SubscriptionStatus, SubscriptionVault,
    SubscriptionVaultClient,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

// ─── helpers ────────────────────────────────────────────────────────────────

/// Spin up a fresh environment with auth mocked and deploy the contract.
/// Returns `(env, contract_id)`. Callers build `SubscriptionVaultClient::new(&env, &id)` locally
/// to avoid lifetime issues.
fn setup() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    (env, contract_id)
}

// ─── existing tests (preserved) ─────────────────────────────────────────────

#[test]
fn test_init_and_struct() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);
}

#[test]
fn test_subscription_struct() {
    let env = Env::default();
    let sub = Subscription {
        subscriber: Address::generate(&env),
        merchant: Address::generate(&env),
        amount: 10_000_0000, // 10 USDC (7 decimals stored as i128)
        interval_seconds: 30 * 24 * 60 * 60, // 30 days
        last_payment_timestamp: 0,
        status: SubscriptionStatus::Active,
        prepaid_balance: 50_000_0000,
        usage_enabled: false,
    };
    assert_eq!(sub.status, SubscriptionStatus::Active);
}

// ─── get_subscriptions_by_subscriber ────────────────────────────────────────

/// A subscriber that has never created any subscription returns an empty list.
#[test]
fn test_view_by_subscriber_zero_subscriptions() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let unknown = Address::generate(&env);
    let result = client.get_subscriptions_by_subscriber(&unknown, &0, &0);
    assert_eq!(result.len(), 0, "unknown subscriber should return empty list");
}

/// A subscriber with exactly one subscription gets that single entry with correct fields.
#[test]
fn test_view_by_subscriber_one_subscription() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let id = client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);

    let entries = client.get_subscriptions_by_subscriber(&subscriber, &0, &0);
    assert_eq!(entries.len(), 1);
    let entry: SubscriptionEntry = entries.get(0).unwrap();
    assert_eq!(entry.id, id);
    assert_eq!(entry.subscription.subscriber, subscriber);
    assert_eq!(entry.subscription.merchant, merchant);
    assert_eq!(entry.subscription.amount, 1_000_000);
    assert_eq!(entry.subscription.status, SubscriptionStatus::Active);
    assert_eq!(entry.subscription.usage_enabled, false);
}

/// A subscriber with several subscriptions gets all of them in ascending-ID order.
#[test]
fn test_view_by_subscriber_many_subscriptions() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let mut expected_ids: Vec<u32> = Vec::new(&env);
    for i in 0..5u32 {
        let id = client.create_subscription(
            &subscriber,
            &merchant,
            &((i as i128 + 1) * 1_000_000),
            &86_400,
            &false,
        );
        expected_ids.push_back(id);
    }

    let entries = client.get_subscriptions_by_subscriber(&subscriber, &0, &0);
    assert_eq!(entries.len(), 5);
    for i in 0..5u32 {
        let entry: SubscriptionEntry = entries.get(i).unwrap();
        assert_eq!(entry.id, expected_ids.get(i).unwrap(), "id at position {i} mismatch");
        assert_eq!(
            entry.subscription.amount,
            (i as i128 + 1) * 1_000_000,
            "amount at position {i} mismatch"
        );
    }
}

/// Subscriptions from different subscribers are stored in isolated indices;
/// subscriber A cannot see subscriber B's subscriptions and vice-versa.
#[test]
fn test_view_by_subscriber_isolation() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let sub_a = Address::generate(&env);
    let sub_b = Address::generate(&env);
    let merchant = Address::generate(&env);

    client.create_subscription(&sub_a, &merchant, &1_000_000, &86_400, &false);
    client.create_subscription(&sub_a, &merchant, &2_000_000, &86_400, &false);
    client.create_subscription(&sub_b, &merchant, &3_000_000, &86_400, &false);

    let a_entries = client.get_subscriptions_by_subscriber(&sub_a, &0, &0);
    let b_entries = client.get_subscriptions_by_subscriber(&sub_b, &0, &0);

    assert_eq!(a_entries.len(), 2, "sub_a should have 2 subscriptions");
    assert_eq!(b_entries.len(), 1, "sub_b should have 1 subscription");

    for i in 0..a_entries.len() {
        assert_eq!(
            a_entries.get(i).unwrap().subscription.subscriber,
            sub_a,
            "entry {i} in sub_a list must belong to sub_a"
        );
    }
    assert_eq!(
        b_entries.get(0).unwrap().subscription.subscriber,
        sub_b,
        "sub_b's entry must belong to sub_b"
    );
}

/// `start` and `limit` correctly window the result set (standard offset/limit pagination).
#[test]
fn test_view_by_subscriber_pagination() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    for _ in 0..10u32 {
        client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);
    }

    // First page: items 0–2
    let page1 = client.get_subscriptions_by_subscriber(&subscriber, &0, &3);
    assert_eq!(page1.len(), 3);
    assert_eq!(page1.get(0).unwrap().id, 0);
    assert_eq!(page1.get(2).unwrap().id, 2);

    // Second page: items 3–5
    let page2 = client.get_subscriptions_by_subscriber(&subscriber, &3, &3);
    assert_eq!(page2.len(), 3);
    assert_eq!(page2.get(0).unwrap().id, 3);
    assert_eq!(page2.get(2).unwrap().id, 5);

    // Final partial page: only item 9 remains
    let page_last = client.get_subscriptions_by_subscriber(&subscriber, &9, &3);
    assert_eq!(page_last.len(), 1);
    assert_eq!(page_last.get(0).unwrap().id, 9);
}

/// `start` offset equal to or beyond the total subscription count returns an empty list gracefully.
#[test]
fn test_view_by_subscriber_start_beyond_total() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);

    // start=5 but only 1 subscription exists
    let result = client.get_subscriptions_by_subscriber(&subscriber, &5, &10);
    assert_eq!(result.len(), 0, "start beyond total must return empty list");

    // start == total (exact boundary)
    let result2 = client.get_subscriptions_by_subscriber(&subscriber, &1, &10);
    assert_eq!(result2.len(), 0, "start == total must return empty list");
}

/// An address that has never interacted with the contract returns an empty list without error.
#[test]
fn test_view_by_subscriber_missing_subscriber() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let never_subscribed = Address::generate(&env);
    let result = client.get_subscriptions_by_subscriber(&never_subscribed, &0, &100);
    assert_eq!(result.len(), 0);
}

/// `limit = 0` is the "return-all" sentinel; it returns every subscription regardless of count.
#[test]
fn test_view_by_subscriber_limit_zero_returns_all() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    for _ in 0..7u32 {
        client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);
    }

    let all = client.get_subscriptions_by_subscriber(&subscriber, &0, &0);
    assert_eq!(all.len(), 7, "limit=0 must return all 7 subscriptions");
}

/// Paginating through a large number of subscriptions collects every entry exactly once,
/// in the correct ascending-ID order, and terminates cleanly.
#[test]
fn test_view_by_subscriber_large_count_pagination() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let total: u32 = 20;
    for _ in 0..total {
        client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);
    }

    let page_size: u32 = 5;
    let mut collected: Vec<u32> = Vec::new(&env);
    let mut cursor: u32 = 0;
    loop {
        let page = client.get_subscriptions_by_subscriber(&subscriber, &cursor, &page_size);
        let len = page.len();
        for j in 0..len {
            collected.push_back(page.get(j).unwrap().id);
        }
        cursor += len;
        if len < page_size {
            break;
        }
    }

    assert_eq!(collected.len(), total, "paginating must yield all {total} entries");
    // Verify strict ascending order
    for i in 0..total {
        assert_eq!(collected.get(i).unwrap(), i, "entry at position {i} must have id {i}");
    }
}

/// Subscriptions for two subscribers are interleaved in global ID space; each subscriber's view
/// must return only their own entries in ascending ID order.
#[test]
fn test_view_by_subscriber_ordering_with_interleaved_subscriptions() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let sub_a = Address::generate(&env);
    let sub_b = Address::generate(&env);
    let merchant = Address::generate(&env);

    // Interleave: A(id=0), B(id=1), A(id=2), B(id=3), A(id=4)
    let id_a0 = client.create_subscription(&sub_a, &merchant, &1_000_000, &86_400, &false);
    let id_b0 = client.create_subscription(&sub_b, &merchant, &2_000_000, &86_400, &false);
    let id_a1 = client.create_subscription(&sub_a, &merchant, &3_000_000, &86_400, &false);
    let id_b1 = client.create_subscription(&sub_b, &merchant, &4_000_000, &86_400, &false);
    let id_a2 = client.create_subscription(&sub_a, &merchant, &5_000_000, &86_400, &false);

    let a_entries = client.get_subscriptions_by_subscriber(&sub_a, &0, &0);
    assert_eq!(a_entries.len(), 3);
    assert_eq!(a_entries.get(0).unwrap().id, id_a0);
    assert_eq!(a_entries.get(1).unwrap().id, id_a1);
    assert_eq!(a_entries.get(2).unwrap().id, id_a2);

    let b_entries = client.get_subscriptions_by_subscriber(&sub_b, &0, &0);
    assert_eq!(b_entries.len(), 2);
    assert_eq!(b_entries.get(0).unwrap().id, id_b0);
    assert_eq!(b_entries.get(1).unwrap().id, id_b1);
}

/// `limit` larger than the remaining items returns only the items that exist (no panic, no padding).
#[test]
fn test_view_by_subscriber_limit_beyond_remaining() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    for _ in 0..3u32 {
        client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);
    }

    // start=1, limit=100 → only 2 items remain
    let result = client.get_subscriptions_by_subscriber(&subscriber, &1, &100);
    assert_eq!(result.len(), 2, "only the 2 remaining items should be returned");
    assert_eq!(result.get(0).unwrap().id, 1);
    assert_eq!(result.get(1).unwrap().id, 2);
}

/// A single page exactly matching the total returns all items and a follow-up call returns empty.
#[test]
fn test_view_by_subscriber_exact_page_boundary() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    for _ in 0..4u32 {
        client.create_subscription(&subscriber, &merchant, &1_000_000, &86_400, &false);
    }

    let page = client.get_subscriptions_by_subscriber(&subscriber, &0, &4);
    assert_eq!(page.len(), 4);

    // Next cursor = 4, which equals total → must return empty
    let next = client.get_subscriptions_by_subscriber(&subscriber, &4, &4);
    assert_eq!(next.len(), 0);
}

/// `get_subscription` returns `Error::NotFound` for an ID that does not exist.
#[test]
fn test_get_subscription_not_found() {
    let (env, contract_id) = setup();
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    client.init(&Address::generate(&env), &Address::generate(&env));

    let result = client.try_get_subscription(&9999);
    assert!(result.is_err(), "non-existent subscription_id must return NotFound");
}
