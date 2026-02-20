#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

/// Typed storage keys used throughout the contract.
///
/// Using an enum prevents key collisions and makes storage layout explicit and auditable.
#[contracttype]
pub enum DataKey {
    /// Address of the token (USDC) used for billing.
    Token,
    /// Address of the contract administrator / billing engine.
    Admin,
    /// Monotonically increasing counter used to assign subscription IDs.
    NextId,
    /// Full [`Subscription`] record stored under its assigned `u32` ID.
    Subscription(u32),
    /// Per-subscriber index: maps an [`Address`] to the ordered list of subscription IDs it owns.
    ///
    /// Stored as `Vec<u32>`; items are appended in creation order, so the list is always
    /// sorted ascending by ID.
    SubscriberIndex(Address),
}

#[contracterror]
#[repr(u32)]
pub enum Error {
    NotFound = 404,
    Unauthorized = 401,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    Active = 0,
    Paused = 1,
    Cancelled = 2,
    InsufficientBalance = 3,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    pub subscriber: Address,
    pub merchant: Address,
    pub amount: i128,
    pub interval_seconds: u64,
    pub last_payment_timestamp: u64,
    pub status: SubscriptionStatus,
    pub prepaid_balance: i128,
    pub usage_enabled: bool,
}

/// A subscription record paired with its on-chain ID.
///
/// Returned by [`SubscriptionVault::get_subscriptions_by_subscriber`] so callers
/// always know the ID without needing a separate lookup.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionEntry {
    /// Unique, auto-assigned subscription ID (starts at 0, increments by 1).
    pub id: u32,
    /// Full subscription data.
    pub subscription: Subscription,
}

#[contract]
pub struct SubscriptionVault;

#[contractimpl]
impl SubscriptionVault {
    /// Initialize the contract (e.g. set token and admin). Extend as needed.
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), Error> {
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Create a new subscription. Caller deposits initial USDC; contract stores agreement.
    ///
    /// Also appends the new subscription ID to the subscriber's index so it is
    /// discoverable via [`get_subscriptions_by_subscriber`].
    pub fn create_subscription(
        env: Env,
        subscriber: Address,
        merchant: Address,
        amount: i128,
        interval_seconds: u64,
        usage_enabled: bool,
    ) -> Result<u32, Error> {
        subscriber.require_auth();
        // TODO: transfer initial deposit from subscriber to contract, then store subscription
        let sub = Subscription {
            subscriber: subscriber.clone(),
            merchant,
            amount,
            interval_seconds,
            last_payment_timestamp: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
            prepaid_balance: 0i128, // TODO: set from initial deposit
            usage_enabled,
        };
        let id = Self::_next_id(&env);
        env.storage().instance().set(&DataKey::Subscription(id), &sub);

        // Update subscriber → [subscription IDs] index.
        let index_key = DataKey::SubscriberIndex(subscriber);
        let mut ids: Vec<u32> = env
            .storage()
            .instance()
            .get(&index_key)
            .unwrap_or_else(|| Vec::new(&env));
        ids.push_back(id);
        env.storage().instance().set(&index_key, &ids);

        Ok(id)
    }

    /// Subscriber deposits more USDC into their vault for this subscription.
    pub fn deposit_funds(
        env: Env,
        subscription_id: u32,
        subscriber: Address,
        amount: i128,
    ) -> Result<(), Error> {
        subscriber.require_auth();
        // TODO: transfer USDC from subscriber, increase prepaid_balance for subscription_id
        let _ = (env, subscription_id, amount);
        Ok(())
    }

    /// Billing engine (backend) calls this to charge one interval. Deducts from vault, pays merchant.
    pub fn charge_subscription(_env: Env, _subscription_id: u32) -> Result<(), Error> {
        // TODO: require_caller admin or authorized billing service
        // TODO: load subscription, check interval and balance, transfer to merchant, update last_payment_timestamp and prepaid_balance
        Ok(())
    }

    /// Subscriber or merchant cancels the subscription. Remaining balance can be withdrawn by subscriber.
    pub fn cancel_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();
        // TODO: load subscription, set status Cancelled, allow withdraw of prepaid_balance
        let _ = (env, subscription_id);
        Ok(())
    }

    /// Pause subscription (no charges until resumed).
    pub fn pause_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();
        // TODO: load subscription, set status Paused
        let _ = (env, subscription_id);
        Ok(())
    }

    /// Merchant withdraws accumulated USDC to their wallet.
    pub fn withdraw_merchant_funds(
        _env: Env,
        merchant: Address,
        _amount: i128,
    ) -> Result<(), Error> {
        merchant.require_auth();
        // TODO: deduct from merchant's balance in contract, transfer token to merchant
        Ok(())
    }

    /// Read a single subscription by its ID.
    pub fn get_subscription(env: Env, subscription_id: u32) -> Result<Subscription, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Subscription(subscription_id))
            .ok_or(Error::NotFound)
    }

    /// Return a page of subscriptions owned by `subscriber`, ordered ascending by subscription ID.
    ///
    /// # Parameters
    ///
    /// | Name         | Description |
    /// |--------------|-------------|
    /// | `subscriber` | Address whose subscriptions to retrieve. |
    /// | `start`      | Zero-based offset into the subscriber's full list (cursor for pagination). |
    /// | `limit`      | Maximum entries to return. Pass `0` to retrieve **all** subscriptions. |
    ///
    /// # Returns
    ///
    /// A `Vec<SubscriptionEntry>` in ascending ID order.
    /// Returns an empty vec when the subscriber has no subscriptions or when `start`
    /// is equal to or exceeds the subscriber's total subscription count.
    ///
    /// # Pagination strategy
    ///
    /// Use a simple cursor pattern:
    /// 1. Call `get_subscriptions_by_subscriber(addr, 0, PAGE_SIZE)`.
    /// 2. Record `next_start = start + result.len()`.
    /// 3. If `result.len() < PAGE_SIZE` you have reached the end; otherwise repeat from step 1
    ///    with the updated cursor.
    ///
    /// This pattern is safe when new subscriptions are added during iteration: because IDs are
    /// monotonically increasing, entries that were already returned will never shift position in
    /// the index.
    ///
    /// # Performance
    ///
    /// Performs **one** storage read to load the subscriber's ID list (`O(1)` key lookup), then
    /// **one** storage read per returned entry (`O(limit)` total). The cost is independent of
    /// the subscriber's total subscription count, making pagination cheap even for high-volume
    /// subscribers.
    pub fn get_subscriptions_by_subscriber(
        env: Env,
        subscriber: Address,
        start: u32,
        limit: u32,
    ) -> Vec<SubscriptionEntry> {
        let index_key = DataKey::SubscriberIndex(subscriber);
        let ids: Vec<u32> = env
            .storage()
            .instance()
            .get(&index_key)
            .unwrap_or_else(|| Vec::new(&env));

        let total = ids.len();
        let start_idx = start.min(total);
        let end_idx = if limit == 0 {
            total
        } else {
            start.saturating_add(limit).min(total)
        };

        let mut result: Vec<SubscriptionEntry> = Vec::new(&env);
        let mut i = start_idx;
        while i < end_idx {
            if let Some(sub_id) = ids.get(i) {
                if let Some(sub) = env
                    .storage()
                    .instance()
                    .get::<DataKey, Subscription>(&DataKey::Subscription(sub_id))
                {
                    result.push_back(SubscriptionEntry {
                        id: sub_id,
                        subscription: sub,
                    });
                }
            }
            i += 1;
        }
        result
    }

    // ─── internal helpers ────────────────────────────────────────────────────

    fn _next_id(env: &Env) -> u32 {
        let id: u32 = env.storage().instance().get(&DataKey::NextId).unwrap_or(0);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        id
    }
}

#[cfg(test)]
mod test;
