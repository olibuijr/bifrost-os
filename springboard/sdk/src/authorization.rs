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

use bifrost_ui::access_control::RightStore;

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
}
