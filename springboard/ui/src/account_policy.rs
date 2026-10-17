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

impl Feature {
    /// Every feature, in the order the policy file lists them.
    pub const ALL: [Feature; 5] = [
        Feature::Purchases,
        Feature::Camera,
        Feature::Microphone,
        Feature::Location,
        Feature::ContentSharing,
    ];

    /// The name a feature has in the policy file.
    pub fn name(self) -> &'static str {
        match self {
            Feature::Purchases => "purchases",
            Feature::Camera => "camera",
            Feature::Microphone => "microphone",
            Feature::Location => "location",
            Feature::ContentSharing => "content-sharing",
        }
    }

    /// The feature with this policy-file name, if there is one.
    pub fn from_name(name: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|feature| feature.name() == name)
    }
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

    /// The blocked app identifiers, in sorted order.
    pub fn blocked_apps(&self) -> Vec<&str> {
        self.blocked_apps.iter().map(String::as_str).collect()
    }

    /// The switched-off features, in their declaration order.
    pub fn disabled_features(&self) -> Vec<Feature> {
        self.disabled.iter().copied().collect()
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

impl std::fmt::Display for Denial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denial::InvalidId => write!(f, "empty account or app identifier"),
            Denial::AppBlocked(app) => write!(f, "{app} is blocked for this account"),
            Denial::FeatureDisabled(feature) => write!(f, "{} is switched off for this account", feature.name()),
        }
    }
}

impl PolicyTable {
    /// Reads a policy file. Each non-blank line that does not start with `#` is one rule:
    /// `<account> block <app>` or `<account> disable <feature>`, separated by spaces or tabs.
    /// Any other line is an error, so a damaged file cannot silently drop a restriction.
    pub fn parse(text: &str) -> Result<PolicyTable, String> {
        let mut table = PolicyTable::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = number + 1;
            let fields: Vec<&str> = line.split_whitespace().collect();
            let &[account, verb, value] = fields.as_slice() else {
                return Err(format!("policy line {at}: expected three fields"));
            };
            let mut policy = table.policy(account).cloned().unwrap_or_default();
            match verb {
                "block" => policy.block_app(value).map_err(|e| format!("policy line {at}: {e:?}"))?,
                "disable" => match Feature::from_name(value) {
                    Some(feature) => policy.disable(feature),
                    None => return Err(format!("policy line {at}: unknown feature {value}")),
                },
                _ => return Err(format!("policy line {at}: unknown rule {verb}")),
            }
            table.set(account, policy).map_err(|e| format!("policy line {at}: {e:?}"))?;
        }
        Ok(table)
    }

    /// Writes the table in the form `parse` reads, accounts and rules in sorted order.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (account, policy) in &self.accounts {
            for app in &policy.blocked_apps {
                out.push_str(&format!("{account} block {app}\n"));
            }
            for feature in &policy.disabled {
                out.push_str(&format!("{account} disable {}\n", feature.name()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_file_round_trips() {
        let mut table = PolicyTable::new();
        let mut kid = Policy::unrestricted();
        kid.block_app("is.bifrost.browser").unwrap();
        kid.block_app("is.bifrost.store").unwrap();
        kid.disable(Feature::Camera);
        kid.disable(Feature::ContentSharing);
        table.set("kid", kid).unwrap();
        table.set("guest", Policy::unrestricted()).unwrap();
        let text = table.to_text();
        assert_eq!(PolicyTable::parse(&text).unwrap(), table);
        assert_eq!(PolicyTable::parse(&text).unwrap().to_text(), text);
    }

    #[test]
    fn policy_file_skips_comments_and_blank_lines() {
        let table = PolicyTable::parse("# rules\n\nkid\tblock\tis.bifrost.store\n").unwrap();
        assert_eq!(table.check_app("kid", "is.bifrost.store"), Err(Denial::AppBlocked("is.bifrost.store".into())));
        assert_eq!(table.check_app("kid", "is.bifrost.notes"), Ok(()));
    }

    #[test]
    fn policy_file_rejects_damaged_rules() {
        assert!(PolicyTable::parse("kid block").is_err());
        assert!(PolicyTable::parse("kid disable teleport").is_err());
        assert!(PolicyTable::parse("kid allow is.bifrost.store").is_err());
        assert!(PolicyTable::parse("  block is.bifrost.store").is_err());
    }

    #[test]
    fn feature_names_are_unique_and_parse_back() {
        for feature in Feature::ALL {
            assert_eq!(Feature::from_name(feature.name()), Some(feature));
        }
        assert_eq!(Feature::from_name("unknown"), None);
    }

    #[test]
    fn denial_text_names_the_app_or_feature() {
        assert_eq!(Denial::AppBlocked("is.bifrost.store".into()).to_string(), "is.bifrost.store is blocked for this account");
        assert_eq!(Denial::FeatureDisabled(Feature::Camera).to_string(), "camera is switched off for this account");
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
