#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    NotFound = 404,
    Unauthorized = 401,
    /// Charge attempted before `last_payment_timestamp + interval_seconds`.
    IntervalNotElapsed = 1001,
    /// Subscription is not Active (e.g. Paused, Cancelled).
    NotActive = 1002,
    /// `charge_usage` called on a subscription with `usage_enabled = false`.
    UsageNotEnabled = 1003,
    /// Prepaid balance is too low to cover the requested debit.
    InsufficientPrepaidBalance = 1004,
    /// The supplied amount is not a positive value.
    InvalidAmount = 1005,
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

#[contract]
pub struct SubscriptionVault;

#[contractimpl]
impl SubscriptionVault {
    /// Initialize the contract (e.g. set token and admin). Extend as needed.
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), Error> {
        env.storage().instance().set(&Symbol::new(&env, "token"), &token);
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        Ok(())
    }

    /// Create a new subscription. Caller deposits initial USDC; contract stores agreement.
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
        env.storage().instance().set(&id, &sub);
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

    /// Billing engine (backend) calls this to charge one interval.
    ///
    /// Enforces strict interval timing: the current ledger timestamp must be
    /// >= `last_payment_timestamp + interval_seconds`. If the interval has not
    /// elapsed, returns `Error::IntervalNotElapsed` and leaves storage unchanged.
    /// On success, `last_payment_timestamp` is advanced to the current ledger
    /// timestamp.
    pub fn charge_subscription(env: Env, subscription_id: u32) -> Result<(), Error> {
        // TODO: require_caller admin or authorized billing service

        let mut sub: Subscription = env
            .storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;

        if sub.status != SubscriptionStatus::Active {
            return Err(Error::NotActive);
        }

        let now = env.ledger().timestamp();
        let next_charge_at = sub
            .last_payment_timestamp
            .checked_add(sub.interval_seconds)
            .expect("interval overflow");

        if now < next_charge_at {
            return Err(Error::IntervalNotElapsed);
        }

        sub.last_payment_timestamp = now;

        // TODO: deduct sub.amount from sub.prepaid_balance, transfer to merchant

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Charge a metered usage amount against the subscription's prepaid balance.
    ///
    /// Designed for integration with an **off-chain usage metering service**:
    /// the service measures consumption, then calls this entrypoint with the
    /// computed `usage_amount` to debit the subscriber's vault.
    ///
    /// # Requirements
    ///
    /// * The subscription must be `Active`.
    /// * `usage_enabled` must be `true` on the subscription.
    /// * `usage_amount` must be positive (`> 0`).
    /// * `prepaid_balance` must be >= `usage_amount`.
    ///
    /// # Behaviour
    ///
    /// On success, `prepaid_balance` is reduced by `usage_amount`.  If the
    /// debit drains the balance to zero the subscription transitions to
    /// `InsufficientBalance` status, signalling that no further charges
    /// (interval or usage) can proceed until the subscriber tops up.
    ///
    /// # Errors
    ///
    /// | Variant | Reason |
    /// |---------|--------|
    /// | `NotFound` | Subscription ID does not exist. |
    /// | `NotActive` | Subscription is not `Active`. |
    /// | `UsageNotEnabled` | `usage_enabled` is `false`. |
    /// | `InvalidAmount` | `usage_amount` is zero or negative. |
    /// | `InsufficientPrepaidBalance` | Prepaid balance cannot cover the debit. |
    pub fn charge_usage(
        env: Env,
        subscription_id: u32,
        usage_amount: i128,
    ) -> Result<(), Error> {
        // TODO: require_caller admin or authorized metering service

        let mut sub: Subscription = env
            .storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;

        if sub.status != SubscriptionStatus::Active {
            return Err(Error::NotActive);
        }

        if !sub.usage_enabled {
            return Err(Error::UsageNotEnabled);
        }

        if usage_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if sub.prepaid_balance < usage_amount {
            return Err(Error::InsufficientPrepaidBalance);
        }

        sub.prepaid_balance -= usage_amount;

        // If the vault is now empty, transition to InsufficientBalance so no
        // further charges (interval or usage) can proceed until top-up.
        if sub.prepaid_balance == 0 {
            sub.status = SubscriptionStatus::InsufficientBalance;
        }

        // TODO: transfer usage_amount USDC to merchant

        env.storage().instance().set(&subscription_id, &sub);
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

    /// Read subscription by id (for indexing and UI).
    pub fn get_subscription(env: Env, subscription_id: u32) -> Result<Subscription, Error> {
        env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)
    }

    fn _next_id(env: &Env) -> u32 {
        let key = Symbol::new(env, "next_id");
        let id: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(id + 1));
        id
    }
}

#[cfg(test)]
mod test;
