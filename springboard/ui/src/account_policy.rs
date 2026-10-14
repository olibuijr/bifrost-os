//! Account restriction policy: which apps an account may open and which features it may use. A policy is data that
//! the shell asks before it launches an app or offers a feature. An account with no entry is unrestricted. Nothing
//! here reads or writes an account database.

use std::collections::{BTreeMap, BTreeSet};

/// A feature an account policy can switch off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    Purchases,
    Camera,
    Microphone,
    Location,
    ContentSharing,
}

/// Why a request is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// An app or account identifier is empty after trimming.
    InvalidId,
    /// The app is on the account's block list.
    AppBlocked(String),
    /// The feature is switched off for the account.
    FeatureDisabled(Feature),
}

/// Failures of a policy edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// An app or account identifier is empty.
    EmptyId,
}

/// The restrictions on one account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    blocked_apps: BTreeSet<String>,
    disabled: BTreeSet<Feature>,
}

impl Policy {
    /// A policy with no restrictions.
    pub fn unrestricted() -> Policy {
        Policy::default()
    }

    /// Adds an app to the block list. Blocking an app twice has no further effect.
    pub fn block_app(&mut self, app_id: &str) -> Result<(), PolicyError> {
        let app_id = non_empty(app_id)?;
        self.blocked_apps.insert(app_id.to_string());
        Ok(())
    }

    /// Removes an app from the block list. Returns whether it was blocked.
    pub fn unblock_app(&mut self, app_id: &str) -> bool {
        self.blocked_apps.remove(app_id.trim())
    }

    /// Switches a feature off for the account.
    pub fn disable(&mut self, feature: Feature) {
        self.disabled.insert(feature);
    }

    /// Switches a feature back on for the account.
    pub fn enable(&mut self, feature: Feature) {
        self.disabled.remove(&feature);
    }

    /// Checks whether the account may open the app.
    pub fn check_app(&self, app_id: &str) -> Result<(), Denial> {
        let app_id = app_id.trim();
        if app_id.is_empty() {
            return Err(Denial::InvalidId);
        }
        if self.blocked_apps.contains(app_id) {
            Err(Denial::AppBlocked(app_id.to_string()))
        } else {
            Ok(())
        }
    }

    /// Checks whether the account may use the feature.
    pub fn check_feature(&self, feature: Feature) -> Result<(), Denial> {
        if self.disabled.contains(&feature) {
            Err(Denial::FeatureDisabled(feature))
        } else {
            Ok(())
        }
    }
}

/// Policies for every account the shell manages, keyed by account identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicyTable {
    accounts: BTreeMap<String, Policy>,
}

impl PolicyTable {
    /// An empty table: every account is unrestricted.
    pub fn new() -> PolicyTable {
        PolicyTable::default()
    }

    /// Stores the policy for an account, replacing any earlier one.
    pub fn set(&mut self, account_id: &str, policy: Policy) -> Result<(), PolicyError> {
        let account_id = non_empty(account_id)?;
        self.accounts.insert(account_id.to_string(), policy);
        Ok(())
    }

    /// Removes an account's policy, so the account is unrestricted again. Returns the removed policy.
    pub fn remove(&mut self, account_id: &str) -> Option<Policy> {
        self.accounts.remove(account_id.trim())
    }

    /// The policy stored for an account, if it has one.
    pub fn policy(&self, account_id: &str) -> Option<&Policy> {
        self.accounts.get(account_id.trim())
    }

    /// Checks an app launch for an account. An account with no policy may open any app.
    pub fn check_app(&self, account_id: &str, app_id: &str) -> Result<(), Denial> {
        if account_id.trim().is_empty() {
            return Err(Denial::InvalidId);
        }
        self.policy(account_id).map_or_else(|| Policy::unrestricted().check_app(app_id), |policy| policy.check_app(app_id))
    }

    /// Checks a feature for an account. An account with no policy may use any feature.
    pub fn check_feature(&self, account_id: &str, feature: Feature) -> Result<(), Denial> {
        if account_id.trim().is_empty() {
            return Err(Denial::InvalidId);
        }
        self.policy(account_id).map_or(Ok(()), |policy| policy.check_feature(feature))
    }
}

fn non_empty(id: &str) -> Result<&str, PolicyError> {
    let id = id.trim();
    if id.is_empty() {
        Err(PolicyError::EmptyId)
    } else {
        Ok(id)
    }
}
