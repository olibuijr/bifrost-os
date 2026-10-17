//! Administrator sign-in for system changes (`IASUtilities` parity: the sheet that asks for an administrator's
//! password before an install, update or account change).
//!
//! Pure: no PAM, no window. The shell asks `SignInSession::check` before it runs a change. When the answer is
//! `Outcome::NeedsSignIn`, the shell shows the sheet and reports the password check with `sign_in`. The PAM check
//! itself stays in `springboard/src/privd/auth.rs`; this module only decides when the sheet is needed.
//!
//! Rules:
//! - A change that needs no administrator (`Change::requires_admin` is false) is always allowed.
//! - A successful sign-in lets the same session make changes for `grace_secs` seconds, so an install that asks for
//!   several steps does not show the sheet again at each step.
//! - `sign_out` ends the grace period at once.

/// The system changes that go through the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    InstallApp,
    RemoveApp,
    UpdateSystem,
    AccountChange,
    SecuritySetting,
    Display,
}

impl Change {
    /// Every change except a display setting needs an administrator.
    pub fn requires_admin(self) -> bool {
        !matches!(self, Change::Display)
    }
}

/// The answer to a check before a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    NeedsSignIn,
}

/// The default grace period after a successful sign-in, in seconds.
pub const DEFAULT_GRACE_SECS: i64 = 300;

#[derive(Clone, Debug)]
pub struct SignInSession {
    grace_secs: i64,
    /// When the last successful sign-in happened, in seconds since the epoch.
    signed_in_at: Option<i64>,
}

impl Default for SignInSession {
    fn default() -> Self {
        Self::with_grace(DEFAULT_GRACE_SECS)
    }
}

impl SignInSession {
    /// A session with a grace period. Negative values are raised to zero (no grace).
    pub fn with_grace(grace_secs: i64) -> Self {
        SignInSession { grace_secs: grace_secs.max(0), signed_in_at: None }
    }

    /// Whether a change may run now.
    pub fn check(&self, change: Change, now: i64) -> Outcome {
        if !change.requires_admin() {
            return Outcome::Allowed;
        }
        match self.signed_in_at {
            Some(at) if now >= at && now - at <= self.grace_secs => Outcome::Allowed,
            _ => Outcome::NeedsSignIn,
        }
    }

    /// Report the result of the password check from the sheet. Returns whether the sign-in succeeded. A wrong
    /// password leaves any earlier grace period as it was.
    pub fn sign_in(&mut self, now: i64, password_ok: bool) -> bool {
        if password_ok {
            self.signed_in_at = Some(now);
        }
        password_ok
    }

    /// End the grace period now.
    pub fn sign_out(&mut self) {
        self.signed_in_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_change_never_asks_for_a_password() {
        let s = SignInSession::default();
        assert_eq!(s.check(Change::Display, 0), Outcome::Allowed);
        assert!(!Change::Display.requires_admin());
        assert!(Change::UpdateSystem.requires_admin());
    }

    #[test]
    fn an_admin_change_asks_until_a_sign_in_succeeds() {
        let mut s = SignInSession::default();
        assert_eq!(s.check(Change::InstallApp, 100), Outcome::NeedsSignIn);
        assert!(!s.sign_in(100, false));
        assert_eq!(s.check(Change::InstallApp, 100), Outcome::NeedsSignIn);
        assert!(s.sign_in(100, true));
        assert_eq!(s.check(Change::InstallApp, 100), Outcome::Allowed);
    }

    #[test]
    fn the_grace_period_covers_later_steps_then_expires() {
        let mut s = SignInSession::with_grace(60);
        s.sign_in(1000, true);
        assert_eq!(s.check(Change::UpdateSystem, 1060), Outcome::Allowed);
        assert_eq!(s.check(Change::RemoveApp, 1061), Outcome::NeedsSignIn);
    }

    #[test]
    fn a_failed_sign_in_keeps_the_earlier_grace_period() {
        let mut s = SignInSession::with_grace(60);
        s.sign_in(1000, true);
        assert!(!s.sign_in(1030, false));
        assert_eq!(s.check(Change::AccountChange, 1050), Outcome::Allowed);
    }

    #[test]
    fn sign_out_ends_the_grace_at_once() {
        let mut s = SignInSession::default();
        s.sign_in(10, true);
        s.sign_out();
        assert_eq!(s.check(Change::SecuritySetting, 11), Outcome::NeedsSignIn);
    }

    #[test]
    fn a_clock_that_went_back_does_not_grant_access() {
        let mut s = SignInSession::with_grace(60);
        s.sign_in(1000, true);
        assert_eq!(s.check(Change::InstallApp, 990), Outcome::NeedsSignIn);
    }

    #[test]
    fn a_negative_grace_means_no_grace() {
        let mut s = SignInSession::with_grace(-5);
        s.sign_in(1000, true);
        assert_eq!(s.check(Change::InstallApp, 1000), Outcome::Allowed);
        assert_eq!(s.check(Change::InstallApp, 1001), Outcome::NeedsSignIn);
    }
}
