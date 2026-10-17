//! The `local_auth` service: `LAContext` evaluation backed by the account password, typed into the shell's own sheet.
//!
//! Methods (params are a JSON object):
//! - `can_evaluate {policy}` returns `{ok}` or `{ok: false, error}`. The biometrics policy is always
//!   `biometry_not_available`; `device_owner_authentication` needs an account password (a login backend).
//! - `evaluate {policy, reason, cancel?}` returns `{ok: true}` when the person typed the right password, or
//!   `{ok: false, error}` otherwise. The password never reaches the app: the shell draws a password sheet with
//!   `reason` (see `shell/auth_sheet.rs`) and checks what the person types through the login backend. The first call
//!   for an app queues the sheet and answers `{ok: false, error: "pending", pending: true}`; the app calls again and
//!   gets the answer once the person has answered the sheet. `cancel: true` answers `user_cancel` without a sheet.
//! - `device_state {}` returns `{ok, biometry_type, passcode_set}` (see `device_state_reply`).
//! - `answer {app, password?, cancel?}` is the shell's own call that records what the sheet found. An app cannot make
//!   it: an app's id is never empty, and only the shell calls with an empty id.
//!
//! The password is verified by the login backend, read-only. Five failures in a row lock the service per app for the
//! retry time the backend reports, or 30 seconds; a locked attempt is `authentication_failed` without asking the
//! backend. Error names match `bifrost_app::local_auth::LAError::wire`.
use super::{Shell, SystemService};
use bifrost_core::biometry as bio;
use bifrost_core::json::Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

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

/// A request for the password sheet: the app that asked and the reason the sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub app: String,
    pub reason: String,
}

/// Requests waiting behind the sheet on screen. A request past this is refused; the app asks again later.
pub const MAX_QUEUED: usize = 8;

/// What the password sheet works from: the requests in turn, the one on screen and the answers apps have not taken.
/// Every service call and every shell tick runs on the shell thread, so one thread-local copy is the whole state.
#[derive(Default)]
struct SheetState {
    queue: VecDeque<Request>,
    shown: Option<Request>,
    answers: BTreeMap<String, Result<(), Failure>>,
}

thread_local! {
    static SHEET: RefCell<SheetState> = RefCell::new(SheetState::default());
}

/// Queues a sheet for `app`. Returns false when the queue is full.
pub fn queue(app: &str, reason: &str) -> bool {
    SHEET.with(|s| {
        let mut s = s.borrow_mut();
        if s.queue.len() >= MAX_QUEUED {
            return false;
        }
        s.queue.push_back(Request { app: app.into(), reason: reason.into() });
        true
    })
}

/// Whether `app` has a sheet queued or on screen, so its next call waits instead of queuing another.
pub fn is_waiting(app: &str) -> bool {
    SHEET.with(|s| {
        let s = s.borrow();
        s.queue.iter().any(|r| r.app == app) || s.shown.as_ref().is_some_and(|r| r.app == app)
    })
}

/// The next request the shell should draw, when no sheet is on screen. The request then counts as shown.
pub fn take_request() -> Option<Request> {
    SHEET.with(|s| {
        let mut s = s.borrow_mut();
        if s.shown.is_some() {
            return None;
        }
        let next = s.queue.pop_front()?;
        s.shown = Some(next.clone());
        Some(next)
    })
}

/// Records the sheet's answer for `app` and takes it off the screen. The app's next call returns the answer once.
pub fn finish(app: &str, answer: Result<(), Failure>) {
    SHEET.with(|s| {
        let mut s = s.borrow_mut();
        if s.shown.as_ref().is_some_and(|r| r.app == app) {
            s.shown = None;
        }
        s.answers.insert(app.into(), answer);
    })
}

/// The answer the sheet recorded for `app`, if the app has not taken it yet.
pub fn take_result(app: &str) -> Option<Result<(), Failure>> {
    SHEET.with(|s| s.borrow_mut().answers.remove(app))
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

/// Wrong passwords one sheet allows before the app gets `authentication_failed`.
pub const SHEET_ATTEMPTS: u32 = 3;

/// What the sheet does after one answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A wrong password with tries left: the sheet stays open for another try.
    Retry { attempts_left: u32 },
    /// The answer is final: the app gets it (success, cancel, lockout or the last wrong password).
    Done,
}

/// Decides the step after an answer. `failure` is `None` for the right password, and `attempt` counts the wrong
/// passwords this sheet already had (0 on the first answer). Only a wrong password retries; a lockout or a
/// cancel ends the sheet at once.
pub fn next_step(failure: Option<Failure>, attempt: u32) -> Step {
    match failure {
        Some(Failure::AuthenticationFailed) if attempt + 1 < SHEET_ATTEMPTS => Step::Retry { attempts_left: SHEET_ATTEMPTS - attempt - 1 },
        _ => Step::Done,
    }
}

/// The reply while the sheet is up: the app calls again.
fn pending_reply() -> Json {
    Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str("pending".into())), ("pending".into(), Json::Bool(true))])
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

/// The wire name of a biometry type, as `device_state` reports it.
pub fn biometry_wire(kind: bio::BiometryType) -> &'static str {
    match kind {
        bio::BiometryType::None => "none",
        bio::BiometryType::Touch => "touch_id",
        bio::BiometryType::Face => "face_id",
        bio::BiometryType::Optic => "optic_id",
    }
}

/// The `device_state` reply: the biometry type this device has and whether a passcode is set.
pub fn device_state_reply(password_set: bool) -> Json {
    let facts = device_facts(password_set);
    Json::Obj(vec![
        ("ok".into(), Json::Bool(true)),
        ("biometry_type".into(), Json::Str(biometry_wire(facts.biometry).into())),
        ("passcode_set".into(), Json::Bool(facts.passcode_set)),
    ])
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
                if let Some(answer) = take_result(app_id) {
                    return Ok(reply(answer));
                }
                if !is_waiting(app_id) {
                    let reason = params.get("reason").and_then(Json::as_str).unwrap_or("");
                    if !queue(app_id, reason) {
                        return Ok(reply(Err(Failure::SystemCancel)));
                    }
                }
                Ok(pending_reply())
            }
            "answer" => {
                // only the shell answers the sheet: an app's id is never empty
                if !app_id.is_empty() {
                    return Err("local_auth: answer is the shell's".into());
                }
                let app = params.get("app").and_then(Json::as_str).unwrap_or("").to_string();
                let attempt = match params.get("attempt") {
                    Some(Json::Num(n)) if *n >= 0.0 => *n as u32,
                    _ => 0,
                };
                let answer = if matches!(params.get("cancel"), Some(Json::Bool(true))) {
                    Err(Failure::UserCancel)
                } else {
                    let password = params.get("password").and_then(Json::as_str).unwrap_or("").to_string();
                    let backend = shell.login_backend.clone();
                    self.verify(&app, t, || match backend {
                        None => Check::NoPassword,
                        Some(b) => match b.authenticate(&password) {
                            Reply::Ok(_) => Check::Right,
                            Reply::Wrong { retry_secs } | Reply::LockedOut { retry_secs } => Check::Wrong { retry_secs },
                            Reply::NoPassword => Check::NoPassword,
                            Reply::Unavailable => Check::Unavailable,
                        },
                    })
                };
                // a locked app ends the sheet at once: another try would not reach the backend
                let locked = self.fails.get(&app).is_some_and(|(n, until)| *n >= MAX_FAILURES && t < *until);
                let step = if locked { Step::Done } else { next_step(answer.err(), attempt) };
                match step {
                    Step::Retry { attempts_left } => Ok(Json::Obj(vec![
                        ("ok".into(), Json::Bool(false)),
                        ("retry".into(), Json::Bool(true)),
                        ("attempts_left".into(), Json::Num(f64::from(attempts_left))),
                    ])),
                    Step::Done => {
                        finish(&app, answer);
                        Ok(Json::Obj(vec![("ok".into(), Json::Bool(true)), ("retry".into(), Json::Bool(false))]))
                    }
                }
            }
            "device_state" => Ok(device_state_reply(shell.login_backend.is_some())),
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
    fn a_request_waits_for_the_sheet_and_its_answer_is_taken_once() {
        assert!(queue("app.sheet", "Reveal the password"));
        assert!(is_waiting("app.sheet"));
        let shown = take_request().expect("the first request is shown");
        assert_eq!(shown, Request { app: "app.sheet".into(), reason: "Reveal the password".into() });
        assert!(take_request().is_none(), "one sheet at a time");
        assert!(is_waiting("app.sheet"), "the app still waits while its sheet is on screen");
        finish("app.sheet", Err(Failure::UserCancel));
        assert!(!is_waiting("app.sheet"));
        assert_eq!(take_result("app.sheet"), Some(Err(Failure::UserCancel)));
        assert_eq!(take_result("app.sheet"), None);
    }

    #[test]
    fn a_wrong_password_retries_until_the_last_try() {
        assert_eq!(next_step(Some(Failure::AuthenticationFailed), 0), Step::Retry { attempts_left: 2 });
        assert_eq!(next_step(Some(Failure::AuthenticationFailed), 1), Step::Retry { attempts_left: 1 });
        assert_eq!(next_step(Some(Failure::AuthenticationFailed), 2), Step::Done, "the third wrong password ends the sheet");
        assert_eq!(SHEET_ATTEMPTS, 3);
    }

    #[test]
    fn a_right_password_a_cancel_or_a_lockout_never_retries() {
        assert_eq!(next_step(None, 0), Step::Done);
        assert_eq!(next_step(Some(Failure::UserCancel), 0), Step::Done);
        assert_eq!(next_step(Some(Failure::SystemCancel), 0), Step::Done);
    }

    #[test]
    fn the_pending_reply_says_the_app_should_call_again() {
        let r = pending_reply();
        assert_eq!(r.get("pending"), Some(&Json::Bool(true)));
        assert_eq!(r.get("error").and_then(Json::as_str), Some("pending"));
    }

    #[test]
    fn device_state_reports_no_biometry_and_the_passcode() {
        let reply = device_state_reply(true);
        assert_eq!(reply.get("biometry_type").and_then(Json::as_str), Some("none"));
        assert_eq!(reply.get("passcode_set"), Some(&Json::Bool(true)));
        assert_eq!(device_state_reply(false).get("passcode_set"), Some(&Json::Bool(false)));
        assert_eq!(biometry_wire(bio::BiometryType::Face), "face_id");
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
