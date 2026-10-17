//! The `local_auth` service: `LAContext` evaluation backed by the account password.
//!
//! Methods (params are a JSON object):
//! - `can_evaluate {policy}` returns `{ok}` or `{ok: false, error}`. The biometrics policy is always
//!   `biometry_not_available`; `device_owner_authentication` needs an account password (a login backend).
//! - `evaluate {policy, reason, password?, cancel?}` returns `{ok: true}` when the password is right,
//!   `{ok: false, error: "user_cancel"}` when `cancel` is true, and otherwise `{ok: false, error}`. Without a
//!   `password` the reply is `{ok: false, error: "not_interactive", prompt: true}`: the password sheet that would
//!   collect it is not drawn yet, so the caller supplies the password itself (honest stub, see docs/bifrost-ui.md).
//!
//! The password is verified by the login backend, read-only. Five failures in a row lock the service per app for the
//! retry time the backend reports, or 30 seconds; a locked attempt is `authentication_failed` without asking the
//! backend. Error names match `bifrost_app::local_auth::LAError::wire`.
use super::{Shell, SystemService};
use bifrost_core::biometry as bio;
use bifrost_core::json::Json;
use std::collections::BTreeMap;

/// Failures in a row after which an app is locked out.
pub const MAX_FAILURES: u32 = 5;
/// Lockout when the backend gives no retry time.
pub const DEFAULT_LOCKOUT_SECS: f64 = 30.0;

/// The policies an app may ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    DeviceOwner,
    Biometrics,
}

/// The wire names of the errors this service returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    AuthenticationFailed,
    UserCancel,
    SystemCancel,
    PasscodeNotSet,
    BiometryNotAvailable,
    NotInteractive,
}

impl Failure {
    pub fn wire(self) -> &'static str {
        match self {
            Failure::AuthenticationFailed => "authentication_failed",
            Failure::UserCancel => "user_cancel",
            Failure::SystemCancel => "system_cancel",
            Failure::PasscodeNotSet => "passcode_not_set",
            Failure::BiometryNotAvailable => "biometry_not_available",
            Failure::NotInteractive => "not_interactive",
        }
    }
}

/// What checking one password gave.
pub enum Check {
    Right,
    Wrong { retry_secs: u64 },
    Unavailable,
    NoPassword,
}

#[derive(Default)]
pub struct LocalAuth {
    fails: BTreeMap<String, (u32, f64)>,
}

fn reply(r: Result<(), Failure>) -> Json {
    match r {
        Ok(()) => Json::Obj(vec![("ok".into(), Json::Bool(true))]),
        Err(e) => Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str(e.wire().into()))]),
    }
}

fn policy(params: &Json) -> Result<Policy, String> {
    match params.get("policy").and_then(Json::as_str) {
        Some("device_owner_authentication") => Ok(Policy::DeviceOwner),
        Some("device_owner_authentication_with_biometrics") => Ok(Policy::Biometrics),
        other => Err(format!("local_auth: unknown policy {other:?}")),
    }
}

/// The device facts the shell knows: no biometric sensor, and the account password as the passcode.
pub fn device_facts(password_set: bool) -> bio::Device {
    bio::Device { biometry: bio::BiometryType::None, enrolled_sets: Vec::new(), passcode_set: password_set, failures: 0, max_failures: 3 }
}

/// The biometry model's requirement for an app policy: device-owner asks for the default (biometry, then passcode);
/// the biometrics policy asks for biometry alone.
fn requirement(policy: Policy) -> bio::Requirement {
    match policy {
        Policy::DeviceOwner => bio::Policy::DeviceOwner.requirement(),
        Policy::Biometrics => bio::Policy::BiometricsOnly.requirement(),
    }
}

impl LocalAuth {
    /// The policy check without a prompt, decided by the biometry model. `password_set` is whether the device has an
    /// account password.
    pub fn can_evaluate(policy: Policy, password_set: bool) -> Result<(), Failure> {
        match bio::resolve(requirement(policy), &device_facts(password_set), None) {
            Ok(_) => Ok(()),
            Err(bio::Failure::PasscodeNotSet) => Err(Failure::PasscodeNotSet),
            Err(_) => Err(Failure::BiometryNotAvailable),
        }
    }

    /// Verifies one password for `app` at time `t` with `check`, counting failures and locking out after
    /// [`MAX_FAILURES`]. The backend is not asked while the app is locked out.
    pub fn verify(&mut self, app: &str, t: f64, check: impl FnOnce() -> Check) -> Result<(), Failure> {
        let entry = self.fails.entry(app.to_string()).or_insert((0, 0.0));
        if entry.0 >= MAX_FAILURES {
            if t < entry.1 {
                return Err(Failure::AuthenticationFailed);
            }
            *entry = (0, 0.0);
        }
        match check() {
            Check::Right => {
                *entry = (0, 0.0);
                Ok(())
            }
            Check::Wrong { retry_secs } => {
                entry.0 += 1;
                if entry.0 >= MAX_FAILURES {
                    entry.1 = t + if retry_secs > 0 { retry_secs as f64 } else { DEFAULT_LOCKOUT_SECS };
                }
                Err(Failure::AuthenticationFailed)
            }
            Check::Unavailable => Err(Failure::SystemCancel),
            Check::NoPassword => Err(Failure::PasscodeNotSet),
        }
    }
}

impl SystemService for LocalAuth {
    fn name(&self) -> &'static str {
        "local_auth"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        use crate::shell::login::Reply;
        match method {
            "can_evaluate" => Ok(reply(Self::can_evaluate(policy(params)?, shell.login_backend.is_some()))),
            "evaluate" => {
                let p = policy(params)?;
                if let Err(e) = Self::can_evaluate(p, shell.login_backend.is_some()) {
                    return Ok(reply(Err(e)));
                }
                if matches!(params.get("cancel"), Some(Json::Bool(true))) {
                    return Ok(reply(Err(Failure::UserCancel)));
                }
                let Some(password) = params.get("password").and_then(Json::as_str) else {
                    return Ok(Json::Obj(vec![
                        ("ok".into(), Json::Bool(false)),
                        ("error".into(), Json::Str(Failure::NotInteractive.wire().into())),
                        ("prompt".into(), Json::Bool(true)),
                    ]));
                };
                let backend = shell.login_backend.clone();
                let r = self.verify(app_id, t, || match backend {
                    None => Check::NoPassword,
                    Some(b) => match b.authenticate(password) {
                        Reply::Ok(_) => Check::Right,
                        Reply::Wrong { retry_secs } | Reply::LockedOut { retry_secs } => Check::Wrong { retry_secs },
                        Reply::NoPassword => Check::NoPassword,
                        Reply::Unavailable => Check::Unavailable,
                    },
                });
                Ok(reply(r))
            }
            other => Err(format!("local_auth: unknown method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrong() -> Check {
        Check::Wrong { retry_secs: 0 }
    }

    #[test]
    fn can_evaluate_follows_policy_and_password() {
        assert_eq!(LocalAuth::can_evaluate(Policy::Biometrics, true), Err(Failure::BiometryNotAvailable));
        assert_eq!(LocalAuth::can_evaluate(Policy::DeviceOwner, true), Ok(()));
        assert_eq!(LocalAuth::can_evaluate(Policy::DeviceOwner, false), Err(Failure::PasscodeNotSet));
    }

    #[test]
    fn the_biometry_model_decides_the_policy_check() {
        assert_eq!(bio::resolve(requirement(Policy::DeviceOwner), &device_facts(true), None), Ok(bio::Method::Passcode));
        assert_eq!(bio::resolve(requirement(Policy::DeviceOwner), &device_facts(false), None), Err(bio::Failure::PasscodeNotSet));
        assert_eq!(bio::resolve(requirement(Policy::Biometrics), &device_facts(true), None), Err(bio::Failure::BiometryUnavailable));
        assert_eq!(LocalAuth::can_evaluate(Policy::Biometrics, false), Err(Failure::BiometryNotAvailable));
    }

    #[test]
    fn right_password_passes_and_wrong_fails() {
        let mut s = LocalAuth::default();
        assert_eq!(s.verify("a", 0.0, || Check::Right), Ok(()));
        assert_eq!(s.verify("a", 0.0, wrong), Err(Failure::AuthenticationFailed));
    }

    #[test]
    fn five_failures_lock_without_asking_the_backend() {
        let mut s = LocalAuth::default();
        for _ in 0..MAX_FAILURES {
            let _ = s.verify("a", 0.0, wrong);
        }
        assert_eq!(s.verify("a", 1.0, || panic!("asked while locked")), Err(Failure::AuthenticationFailed));
        assert_eq!(s.verify("b", 1.0, || Check::Right), Ok(()), "other apps are not locked");
        assert_eq!(s.verify("a", DEFAULT_LOCKOUT_SECS + 1.0, || Check::Right), Ok(()));
    }

    #[test]
    fn success_resets_the_count() {
        let mut s = LocalAuth::default();
        for _ in 0..MAX_FAILURES - 1 {
            let _ = s.verify("a", 0.0, wrong);
        }
        s.verify("a", 0.0, || Check::Right).unwrap();
        let _ = s.verify("a", 0.0, wrong);
        assert_eq!(s.verify("a", 0.0, || Check::Right), Ok(()));
    }

    #[test]
    fn backend_retry_time_sets_the_lockout() {
        let mut s = LocalAuth::default();
        for _ in 0..MAX_FAILURES {
            let _ = s.verify("a", 0.0, || Check::Wrong { retry_secs: 120 });
        }
        assert_eq!(s.verify("a", 100.0, || panic!("locked")), Err(Failure::AuthenticationFailed));
        assert_eq!(s.verify("a", 121.0, || Check::Right), Ok(()));
    }

    #[test]
    fn unavailable_and_missing_password_map_to_errors() {
        let mut s = LocalAuth::default();
        assert_eq!(s.verify("a", 0.0, || Check::Unavailable), Err(Failure::SystemCancel));
        assert_eq!(s.verify("a", 0.0, || Check::NoPassword), Err(Failure::PasscodeNotSet));
    }

    #[test]
    fn policy_names_parse_and_unknown_is_an_error() {
        let p = |n: &str| Json::Obj(vec![("policy".into(), Json::Str(n.into()))]);
        assert_eq!(policy(&p("device_owner_authentication")), Ok(Policy::DeviceOwner));
        assert_eq!(policy(&p("device_owner_authentication_with_biometrics")), Ok(Policy::Biometrics));
        assert!(policy(&p("x")).is_err());
    }
}
