//! Authorization rights for apps: ask for named rights before a privileged operation.
//!
//! The model is `bifrost_ui::access_control::RightStore`. An app calls [`Authorization::obtain`] with the rights it
//! needs. A right that is already valid is returned at once. A missing right is granted only when the app allows
//! interaction and the presence the caller reports satisfies user presence (a passcode or biometry). A refused
//! request grants nothing. [`Authorization::invalidate_credentials`] drops every right this object holds.
//!
//! The rights live in this object, in the app. The shell does not yet enforce them, and the manifest permission is
//! still the only grant that the shell enforces at launch (`springboard/src/bare/app_policy.rs`).
//!
//! ```ignore
//! use bifrost_app::authorization::{Authorization, ObtainFlags, Presence};
//! let mut auth = Authorization::new();
//! let presence = Presence { passcode: true, ..Presence::default() };
//! let granted = auth.obtain(&["disk.format"], ObtainFlags { interaction_allowed: true }, now_secs, Some(&presence), Some(300))?;
//! ```

pub use bifrost_ui::access_control::{ObtainError, ObtainFlags, Presence, RightState};
pub use bifrost_ui::auth_prompt::{AuthPrompt, PromptError, PromptPhase, DEFAULT_MAX_ATTEMPTS};

use bifrost_ui::access_control::{Error as RightError, RightStore};

/// Why a password prompt did not grant its rights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptGrantError {
    /// The request named no rights.
    NoRights,
    /// The prompt has not succeeded. Carries the phase it ended in (cancelled, locked out, fallback chosen, or still
    /// prompting).
    NotAuthorized(PromptPhase),
    /// A right name is not valid, or the store is full. Nothing was granted.
    Store(RightError),
}

/// The rights one app holds. Times are seconds since any fixed epoch the caller uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Authorization {
    store: RightStore,
}

impl Authorization {
    /// An object that holds no rights.
    pub fn new() -> Authorization {
        Authorization::default()
    }

    /// Authorizes the named rights and returns them in the order asked. See `RightStore::obtain`.
    pub fn obtain(
        &mut self,
        rights: &[&str],
        flags: ObtainFlags,
        now: u64,
        presence: Option<&Presence>,
        valid_for: Option<u64>,
    ) -> Result<Vec<String>, ObtainError> {
        self.store.obtain(rights, flags, now, presence, valid_for)
    }

    /// Grants the named rights after the user passed a device-owner prompt (see [`AuthPrompt`]). Only a prompt that
    /// succeeded grants anything. The grant is all or nothing: every name is checked on a copy of the store, which
    /// replaces this one only when every grant succeeds.
    pub fn grant_after_prompt(&mut self, prompt: &AuthPrompt, rights: &[&str], now: u64, valid_for: Option<u64>) -> Result<Vec<String>, PromptGrantError> {
        if rights.is_empty() {
            return Err(PromptGrantError::NoRights);
        }
        if *prompt.phase() != PromptPhase::Succeeded {
            return Err(PromptGrantError::NotAuthorized(prompt.phase().clone()));
        }
        let mut next = self.store.clone();
        for right in rights {
            next.grant(right, now, valid_for).map_err(PromptGrantError::Store)?;
        }
        self.store = next;
        Ok(rights.iter().map(|r| r.to_string()).collect())
    }

    /// The state of one right at `now`.
    pub fn state(&self, right: &str, now: u64) -> RightState {
        self.store.state(right, now)
    }

    /// Drops every right this object holds. Returns how many were dropped.
    pub fn invalidate_credentials(&mut self) -> usize {
        self.store.invalidate_credentials()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passcode() -> Presence {
        Presence { passcode: true, ..Presence::default() }
    }

    fn interactive() -> ObtainFlags {
        ObtainFlags { interaction_allowed: true }
    }

    #[test]
    fn a_new_object_holds_no_rights() {
        let auth = Authorization::new();
        assert_eq!(auth.state("disk.format", 0), RightState::Unknown);
    }

    #[test]
    fn a_missing_right_is_refused_without_interaction() {
        let mut auth = Authorization::new();
        let got = auth.obtain(&["disk.format"], ObtainFlags::default(), 0, Some(&passcode()), None);
        assert_eq!(got, Err(ObtainError::NeedsInteraction("disk.format".to_string())));
        assert_eq!(auth.state("disk.format", 0), RightState::Unknown);
    }

    #[test]
    fn a_passcode_grants_the_right_and_it_expires_after_its_lifetime() {
        let mut auth = Authorization::new();
        let got = auth.obtain(&["disk.format"], interactive(), 100, Some(&passcode()), Some(300));
        assert_eq!(got, Ok(vec!["disk.format".to_string()]));
        assert_eq!(auth.state("disk.format", 399), RightState::Valid, "299 s after the grant");
        assert_eq!(auth.state("disk.format", 400), RightState::Expired, "300 s after the grant");
    }

    #[test]
    fn a_valid_right_is_obtained_again_without_presence() {
        let mut auth = Authorization::new();
        auth.obtain(&["disk.format"], interactive(), 0, Some(&passcode()), None).unwrap();
        let again = auth.obtain(&["disk.format"], ObtainFlags::default(), 5000, None, None);
        assert_eq!(again, Ok(vec!["disk.format".to_string()]));
    }

    #[test]
    fn invalidating_credentials_forgets_every_right() {
        let mut auth = Authorization::new();
        auth.obtain(&["disk.format", "net.admin"], interactive(), 0, Some(&passcode()), None).unwrap();
        assert_eq!(auth.invalidate_credentials(), 2);
        assert_eq!(auth.state("disk.format", 1), RightState::Unknown);
        let after = auth.obtain(&["disk.format"], ObtainFlags::default(), 1, None, None);
        assert_eq!(after, Err(ObtainError::NeedsInteraction("disk.format".to_string())));
    }

    fn passed_prompt() -> AuthPrompt {
        let mut prompt = AuthPrompt::new("Install Calc", false, DEFAULT_MAX_ATTEMPTS).unwrap();
        prompt.submit(true).unwrap();
        prompt
    }

    #[test]
    fn a_passed_prompt_grants_its_rights_for_their_lifetime() {
        let mut auth = Authorization::new();
        let got = auth.grant_after_prompt(&passed_prompt(), &["disk.format", "net.admin"], 100, Some(300));
        assert_eq!(got, Ok(vec!["disk.format".to_string(), "net.admin".to_string()]));
        assert_eq!(auth.state("net.admin", 399), RightState::Valid);
        assert_eq!(auth.state("net.admin", 400), RightState::Expired);
    }

    #[test]
    fn a_prompt_that_did_not_succeed_grants_nothing() {
        let mut auth = Authorization::new();
        let mut cancelled = AuthPrompt::new("Install Calc", true, DEFAULT_MAX_ATTEMPTS).unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(auth.grant_after_prompt(&cancelled, &["disk.format"], 0, None), Err(PromptGrantError::NotAuthorized(PromptPhase::Cancelled)));

        let mut locked = AuthPrompt::new("Install Calc", false, 1).unwrap();
        locked.submit(false).unwrap();
        assert_eq!(auth.grant_after_prompt(&locked, &["disk.format"], 0, None), Err(PromptGrantError::NotAuthorized(PromptPhase::LockedOut)));
        assert_eq!(auth.state("disk.format", 0), RightState::Unknown);
    }

    #[test]
    fn a_bad_right_name_grants_none_of_the_rights() {
        let mut auth = Authorization::new();
        let got = auth.grant_after_prompt(&passed_prompt(), &["disk.format", "Bad Name"], 0, None);
        assert!(matches!(got, Err(PromptGrantError::Store(_))));
        assert_eq!(auth.state("disk.format", 0), RightState::Unknown, "the valid name is not kept either");
        assert_eq!(auth.grant_after_prompt(&passed_prompt(), &[], 0, None), Err(PromptGrantError::NoRights));
    }
}
