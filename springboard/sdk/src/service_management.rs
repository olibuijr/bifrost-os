//! Login items (`SMAppService`): `SMAppService.mainApp` registers the app to start when the user's session starts. The
//! shell keeps the list (`Settings > General > Login Items` shows and removes entries) and launches the apps in it
//! after the desktop is up. Needs no manifest permission: the user sees and controls the list.
//!
//! `status()` is synchronous like and reads the last status the shell reported; the shell reports it once when the
//! app starts and after every `register`, `unregister` and `refresh`, which also arrives in `App::login_item_status`.

pub use crate::proto::LoginStatus as Status;
use crate::proto::{LoginOp, ToShell};
use crate::Ctx;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

static LAST: AtomicU8 = AtomicU8::new(0);

/// `SMAppService`; only `mainApp` exists (BifrOSt has no embedded login-item helpers, launch agents or daemons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SMAppService(());

impl SMAppService {
    /// `SMAppService.mainApp`.
    pub fn main_app() -> Self {
        SMAppService(())
    }
    /// `register()`: the answer arrives as `App::login_item_status` with the returned request id.
    pub fn register(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Register)
    }
    /// `unregister()`.
    pub fn unregister(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Unregister)
    }
    /// Asks the shell for the current status again (it can change in Settings).
    pub fn refresh(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Query)
    }
    /// `status`: the last status the shell reported (`NotRegistered` until the first report).
    pub fn status(&self) -> Status {
        Status::from_wire(LAST.load(Ordering::Relaxed)).unwrap_or(Status::NotRegistered)
    }
}

fn send(ctx: &Ctx, op: LoginOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::LoginItem { request, op });
    request
}

pub(crate) fn mirror(status: Status) {
    LAST.store(status as u8, Ordering::Relaxed);
}

/// The error domain of [`ServiceError::to_error`].
pub const ERROR_DOMAIN: &str = "bifrost.service-management";

/// The errors a registration or removal can report (the cases of the Service Management Errors reference).
/// The reference lists the cases and their meaning but not their numeric values, so [`ServiceError::code`] numbers
/// them 1 to 11 in declaration order. That numbering is stable for this build only, and apps should compare cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// The app is already registered.
    AlreadyRegistered,
    /// The authorization requested failed.
    AuthorizationFailure,
    /// An internal failure has occurred.
    InternalFailure,
    /// The app's property list is invalid.
    InvalidPlist,
    /// The app's code signature does not meet the requirements for the operation.
    InvalidSignature,
    /// The job must be enabled first.
    JobMustBeEnabled,
    /// The system cannot find the specified job.
    JobNotFound,
    /// The job's property list was not found.
    JobPlistNotFound,
    /// The user denied the app's launch request.
    LaunchDeniedByUser,
    /// The service needed for the operation is unavailable or no longer accepts requests.
    ServiceUnavailable,
    /// The path does not exist, or the helper tool at it is not valid.
    ToolNotValid,
}

const ALL_ERRORS: [ServiceError; 11] = [
    ServiceError::AlreadyRegistered,
    ServiceError::AuthorizationFailure,
    ServiceError::InternalFailure,
    ServiceError::InvalidPlist,
    ServiceError::InvalidSignature,
    ServiceError::JobMustBeEnabled,
    ServiceError::JobNotFound,
    ServiceError::JobPlistNotFound,
    ServiceError::LaunchDeniedByUser,
    ServiceError::ServiceUnavailable,
    ServiceError::ToolNotValid,
];

impl ServiceError {
    /// Every case, in declaration order.
    pub fn all() -> &'static [ServiceError] {
        &ALL_ERRORS
    }

    /// The stable number of this case, 1 to 11 (see the type's note on numbering).
    pub fn code(self) -> i64 {
        ALL_ERRORS.iter().position(|e| *e == self).map_or(0, |i| i as i64 + 1)
    }

    /// The case with this number, or `None` for a number outside 1 to 11.
    pub fn from_code(code: i64) -> Option<ServiceError> {
        if code < 1 {
            return None;
        }
        ALL_ERRORS.get((code - 1) as usize).copied()
    }

    /// The case as an [`AppError`] in [`ERROR_DOMAIN`], with its text as the description.
    pub fn to_error(self) -> crate::error::AppError {
        crate::error::AppError::new(ERROR_DOMAIN, self.code()).with_description(self.to_string())
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServiceError::AlreadyRegistered => "The app is already registered.",
            ServiceError::AuthorizationFailure => "The authorization requested failed.",
            ServiceError::InternalFailure => "An internal failure has occurred.",
            ServiceError::InvalidPlist => "The app's property list is invalid.",
            ServiceError::InvalidSignature => "The app's code signature does not meet the requirements for the operation.",
            ServiceError::JobMustBeEnabled => "The job must be enabled first.",
            ServiceError::JobNotFound => "The system cannot find the specified job.",
            ServiceError::JobPlistNotFound => "The job's property list was not found.",
            ServiceError::LaunchDeniedByUser => "The user denied the app's launch request.",
            ServiceError::ServiceUnavailable => "The service needed for the operation is unavailable.",
            ServiceError::ToolNotValid => "The path does not exist, or the helper tool at it is not valid.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_error_codes_are_one_to_eleven_and_round_trip() {
        assert_eq!(ServiceError::all().len(), 11);
        assert_eq!(ServiceError::AlreadyRegistered.code(), 1);
        assert_eq!(ServiceError::JobNotFound.code(), 7);
        assert_eq!(ServiceError::ToolNotValid.code(), 11);
        for e in ServiceError::all() {
            assert_eq!(ServiceError::from_code(e.code()), Some(*e));
        }
        assert_eq!(ServiceError::from_code(0), None);
        assert_eq!(ServiceError::from_code(12), None);
        assert_eq!(ServiceError::from_code(-1), None);
    }

    #[test]
    fn to_error_carries_domain_code_and_text() {
        let e = ServiceError::LaunchDeniedByUser.to_error();
        assert!(e.is(ERROR_DOMAIN, 9));
        assert!(!e.is(ERROR_DOMAIN, 8));
        assert_eq!(e.localized_description(), "The user denied the app's launch request.");
    }

    #[test]
    fn every_case_has_a_distinct_text() {
        let texts: Vec<String> = ServiceError::all().iter().map(|e| e.to_string()).collect();
        for (i, a) in texts.iter().enumerate() {
            assert!(!a.is_empty());
            for b in &texts[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn display_text_matches_the_reference_wording() {
        assert_eq!(ServiceError::AlreadyRegistered.to_string(), "The app is already registered.");
        assert_eq!(ServiceError::InvalidSignature.to_string(), "The app's code signature does not meet the requirements for the operation.");
    }

    #[test]
    fn the_domain_is_the_bifrost_service_domain() {
        assert_eq!(ServiceError::InternalFailure.to_error().domain(), "bifrost.service-management");
        assert_eq!(ServiceError::InternalFailure.to_error().code(), 3);
    }

    #[test]
    fn calls_go_to_the_shell_and_status_follows_the_report() {
        let ctx = crate::runtime::test_ctx();
        let s = SMAppService::main_app();
        assert_eq!(s.status(), Status::NotRegistered);
        let a = s.register(&ctx);
        let b = s.refresh(&ctx);
        let c = s.unregister(&ctx);
        assert_eq!(
            ctx.take_sent(),
            [ToShell::LoginItem { request: a, op: LoginOp::Register }, ToShell::LoginItem { request: b, op: LoginOp::Query }, ToShell::LoginItem { request: c, op: LoginOp::Unregister }]
        );
        mirror(Status::Enabled);
        assert_eq!(s.status(), Status::Enabled);
        mirror(Status::NotRegistered);
    }
}
