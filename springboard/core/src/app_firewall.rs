//! Per-app decisions for incoming connections: the application firewall's answer to "may this app accept network
//! connections from outside?" The first time an app listens for incoming connections, there is no decision and the
//! caller asks the user. The answer is then remembered per app: [`Decision::Allow`] or [`Decision::Deny`].
//!
//! The model reads no state from the system and changes no firewall rule. The caller stores the map (or rebuilds it
//! from its own store), asks [`AppFirewall::check`] before it lets an app accept a connection, and applies the rule
//! that the decision implies. Outgoing connections are not covered here.

use std::collections::BTreeMap;

/// Longest app id the model accepts, in bytes.
pub const MAX_APP_ID_LEN: usize = 256;

/// What the firewall does with incoming connections for one app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The app may accept incoming connections.
    Allow,
    /// The app may not accept incoming connections.
    Deny,
}

/// What the caller must do for an app that wants to accept an incoming connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// A remembered decision allows it.
    Allowed,
    /// A remembered decision refuses it.
    Refused,
    /// No decision is remembered yet, so the user must be asked first.
    AskUser,
}

/// Why a decision could not be recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirewallError {
    /// The app id is blank, too long, or has a control character.
    InvalidAppId,
}

/// The remembered decisions, one per app id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppFirewall {
    decisions: BTreeMap<String, Decision>,
}

fn valid_app_id(app: &str) -> bool {
    !app.trim().is_empty() && app.len() <= MAX_APP_ID_LEN && !app.chars().any(|c| c.is_control())
}

impl AppFirewall {
    pub fn new() -> AppFirewall {
        AppFirewall::default()
    }

    /// Remembers the user's answer for `app`, replacing an earlier one.
    pub fn record(&mut self, app: &str, decision: Decision) -> Result<(), FirewallError> {
        if !valid_app_id(app) {
            return Err(FirewallError::InvalidAppId);
        }
        self.decisions.insert(app.to_string(), decision);
        Ok(())
    }

    /// What to do when `app` wants to accept an incoming connection. An app with no remembered decision, or with a
    /// blank or invalid id, is asked about rather than allowed.
    pub fn check(&self, app: &str) -> Check {
        match self.decisions.get(app) {
            Some(Decision::Allow) => Check::Allowed,
            Some(Decision::Deny) => Check::Refused,
            None => Check::AskUser,
        }
    }

    /// Forgets the remembered decision for `app`, so the user is asked again. Returns whether one was removed.
    pub fn forget(&mut self, app: &str) -> bool {
        self.decisions.remove(app).is_some()
    }

    /// The apps with a remembered decision, in name order.
    pub fn apps(&self) -> Vec<(&str, Decision)> {
        self.decisions.iter().map(|(app, d)| (app.as_str(), *d)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_with_no_decision_is_asked_about() {
        let fw = AppFirewall::new();
        assert_eq!(fw.check("Chat"), Check::AskUser);
    }

    #[test]
    fn a_remembered_allow_or_deny_is_applied_without_asking() {
        let mut fw = AppFirewall::new();
        fw.record("Chat", Decision::Allow).unwrap();
        fw.record("Game", Decision::Deny).unwrap();
        assert_eq!(fw.check("Chat"), Check::Allowed);
        assert_eq!(fw.check("Game"), Check::Refused);
    }

    #[test]
    fn a_new_answer_replaces_the_old_one() {
        let mut fw = AppFirewall::new();
        fw.record("Chat", Decision::Allow).unwrap();
        fw.record("Chat", Decision::Deny).unwrap();
        assert_eq!(fw.check("Chat"), Check::Refused);
        assert_eq!(fw.apps(), vec![("Chat", Decision::Deny)]);
    }

    #[test]
    fn forgetting_an_app_asks_again() {
        let mut fw = AppFirewall::new();
        fw.record("Chat", Decision::Allow).unwrap();
        assert!(fw.forget("Chat"));
        assert!(!fw.forget("Chat"));
        assert_eq!(fw.check("Chat"), Check::AskUser);
    }

    #[test]
    fn blank_long_or_control_app_ids_are_refused_and_not_remembered() {
        let mut fw = AppFirewall::new();
        assert_eq!(fw.record("  ", Decision::Allow), Err(FirewallError::InvalidAppId));
        assert_eq!(fw.record("Ch\nat", Decision::Allow), Err(FirewallError::InvalidAppId));
        let long = "a".repeat(MAX_APP_ID_LEN + 1);
        assert_eq!(fw.record(&long, Decision::Allow), Err(FirewallError::InvalidAppId));
        assert!(fw.apps().is_empty());
        assert_eq!(fw.check(""), Check::AskUser);
    }

    #[test]
    fn apps_are_listed_in_name_order() {
        let mut fw = AppFirewall::new();
        fw.record("Zed", Decision::Deny).unwrap();
        fw.record("Alpha", Decision::Allow).unwrap();
        assert_eq!(fw.apps(), vec![("Alpha", Decision::Allow), ("Zed", Decision::Deny)]);
    }
}
