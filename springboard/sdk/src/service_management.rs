//! Login items (`SMAppService`): `SMAppService.mainApp` registers the app to start when the user's session starts. The
//! shell keeps the list (`Settings > General > Login Items` shows and removes entries) and launches the apps in it
//! after the desktop is up. Needs no manifest permission: the user sees and controls the list.
//!
//! `status()` is synchronous like the reference and reads the last status the shell reported; the shell reports it once
//! when the app starts and after every `register`, `unregister` and `refresh`, which also arrives in
//! `App::login_item_status`.
//!
//! The other service kinds (`loginItem(identifier:)`, `agent(plistName:)`, `daemon(plistName:)`) exist as handles so apps
//! get the reference's answers: their status is `NotFound`, because no helper bundle, launch agent or launch daemon ships
//! in an app bundle, and `try_register` / `try_unregister` return the matching `ServiceError` without sending anything
//! to the shell.

pub use crate::proto::LoginStatus as Status;
use crate::proto::{LoginOp, ToShell};
use crate::Ctx;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

static LAST: AtomicU8 = AtomicU8::new(0);

/// The errors `SMAppService` reports, one per constant of the official "Service Management Errors" list. The
/// documentation gives no numeric codes, so the variants carry none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    AlreadyRegistered,
    AuthorizationFailure,
    InternalFailure,
    InvalidPlist,
    InvalidSignature,
    JobMustBeEnabled,
    JobNotFound,
    JobPlistNotFound,
    LaunchDeniedByUser,
    ServiceUnavailable,
    ToolNotValid,
}

impl ServiceError {
    /// Every variant, in the order of the official list.
    pub const ALL: [ServiceError; 11] = [
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
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServiceError::AlreadyRegistered => "The application is already registered.",
            ServiceError::AuthorizationFailure => "The authorization requested failed.",
            ServiceError::InternalFailure => "An internal failure has occurred.",
            ServiceError::InvalidPlist => "The app's property list is invalid.",
            ServiceError::InvalidSignature => "The app's code signature doesn't meet the requirements to perform the operation.",
            ServiceError::JobMustBeEnabled => "The job must be enabled first.",
            ServiceError::JobNotFound => "The system can't find the specified job.",
            ServiceError::JobPlistNotFound => "The job's property list was not found in the app bundle.",
            ServiceError::LaunchDeniedByUser => "The user denied the app's launch request.",
            ServiceError::ServiceUnavailable => "The service necessary to perform this operation is unavailable or is no longer accepting requests.",
            ServiceError::ToolNotValid => "The specified path doesn't exist or the helper tool at the specified path isn't valid.",
        })
    }
}

impl std::error::Error for ServiceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    MainApp,
    LoginItem(String),
    Agent(String),
    Daemon(String),
}

/// `SMAppService`: a handle on one service. `main_app()` is the only kind the shell can register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SMAppService(Kind);

impl SMAppService {
    /// `SMAppService.mainApp`.
    pub fn main_app() -> Self {
        SMAppService(Kind::MainApp)
    }
    /// `loginItem(identifier:)`: a helper login item by bundle identifier. No helper bundles ship, so it is never found.
    pub fn login_item(identifier: &str) -> Self {
        SMAppService(Kind::LoginItem(identifier.to_string()))
    }
    /// `agent(plistName:)`: a launch agent by property list name. No launch agent ships, so it is never found.
    pub fn agent(plist_name: &str) -> Self {
        SMAppService(Kind::Agent(plist_name.to_string()))
    }
    /// `daemon(plistName:)`: a launch daemon by property list name. No launch daemon ships, so it is never found.
    pub fn daemon(plist_name: &str) -> Self {
        SMAppService(Kind::Daemon(plist_name.to_string()))
    }
    /// `register()`: the answer arrives as `App::login_item_status` with the returned request id. Returns 0 when no
    /// request was sent (a service other than the main app); `try_register` says why.
    pub fn register(&self, ctx: &Ctx) -> u32 {
        self.try_register(ctx).unwrap_or(0)
    }
    /// `register()` as a `Result`: `Ok` with the request id for the main app, otherwise the error the reference gives.
    pub fn try_register(&self, ctx: &Ctx) -> Result<u32, ServiceError> {
        self.precheck()?;
        Ok(send(ctx, LoginOp::Register))
    }
    /// `unregister()`. Returns 0 when no request was sent; `try_unregister` says why.
    pub fn unregister(&self, ctx: &Ctx) -> u32 {
        self.try_unregister(ctx).unwrap_or(0)
    }
    /// `unregister()` as a `Result`, like `try_register`.
    pub fn try_unregister(&self, ctx: &Ctx) -> Result<u32, ServiceError> {
        self.precheck()?;
        Ok(send(ctx, LoginOp::Unregister))
    }
    /// Asks the shell for the current status again (it can change in Settings). Only the main app has a shell status;
    /// for the other kinds nothing is sent and 0 is returned.
    pub fn refresh(&self, ctx: &Ctx) -> u32 {
        match self.0 {
            Kind::MainApp => send(ctx, LoginOp::Query),
            _ => 0,
        }
    }
    /// `status`: the last status the shell reported for the main app (`NotRegistered` until the first report).
    /// Every other kind is `NotFound`.
    pub fn status(&self) -> Status {
        match self.0 {
            Kind::MainApp => Status::from_wire(LAST.load(Ordering::Relaxed)).unwrap_or(Status::NotRegistered),
            _ => Status::NotFound,
        }
    }
    /// The error a register or unregister call gets before anything reaches the shell.
    fn precheck(&self) -> Result<(), ServiceError> {
        match &self.0 {
            Kind::MainApp => Ok(()),
            Kind::LoginItem(_) => Err(ServiceError::JobNotFound),
            Kind::Agent(name) | Kind::Daemon(name) => {
                if valid_plist_name(name) {
                    Err(ServiceError::JobPlistNotFound)
                } else {
                    Err(ServiceError::InvalidPlist)
                }
            }
        }
    }
}

/// A property list name: no path separator, and a `.plist` suffix with something before it.
fn valid_plist_name(name: &str) -> bool {
    !name.contains('/') && name.len() > ".plist".len() && name.ends_with(".plist")
}

fn send(ctx: &Ctx, op: LoginOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::LoginItem { request, op });
    request
}

pub(crate) fn mirror(status: Status) {
    LAST.store(status as u8, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn main_app_try_register_returns_the_request_id_it_sent() {
        let ctx = crate::runtime::test_ctx();
        let id = SMAppService::main_app().try_register(&ctx).unwrap();
        assert_eq!(ctx.take_sent(), [ToShell::LoginItem { request: id, op: LoginOp::Register }]);
    }

    #[test]
    fn other_kinds_report_not_found_and_do_not_read_the_shell_status() {
        assert_eq!(SMAppService::login_item("com.example.helper").status(), Status::NotFound);
        assert_eq!(SMAppService::agent("com.example.agent.plist").status(), Status::NotFound);
        assert_eq!(SMAppService::daemon("com.example.daemon.plist").status(), Status::NotFound);
        assert_eq!(SMAppService::agent("not-a-plist").status(), Status::NotFound);
    }

    #[test]
    fn login_item_register_fails_with_job_not_found_and_sends_nothing() {
        let ctx = crate::runtime::test_ctx();
        let s = SMAppService::login_item("com.example.helper");
        assert_eq!(s.try_register(&ctx), Err(ServiceError::JobNotFound));
        assert_eq!(s.try_unregister(&ctx), Err(ServiceError::JobNotFound));
        assert_eq!(s.register(&ctx), 0);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn agent_and_daemon_names_are_checked_before_the_bundle_lookup() {
        let ctx = crate::runtime::test_ctx();
        assert_eq!(SMAppService::agent("com.example.agent.plist").try_register(&ctx), Err(ServiceError::JobPlistNotFound));
        assert_eq!(SMAppService::daemon("com.example.daemon.plist").try_unregister(&ctx), Err(ServiceError::JobPlistNotFound));
        assert_eq!(SMAppService::agent("com.example.agent").try_register(&ctx), Err(ServiceError::InvalidPlist));
        assert_eq!(SMAppService::daemon("dir/job.plist").try_register(&ctx), Err(ServiceError::InvalidPlist));
        assert_eq!(SMAppService::agent(".plist").try_register(&ctx), Err(ServiceError::InvalidPlist));
        assert_eq!(SMAppService::agent("").try_register(&ctx), Err(ServiceError::InvalidPlist));
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn refresh_for_other_kinds_sends_nothing() {
        let ctx = crate::runtime::test_ctx();
        assert_eq!(SMAppService::daemon("x.plist").refresh(&ctx), 0);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn service_errors_have_the_official_texts_and_are_all_distinct() {
        assert_eq!(ServiceError::ALL.len(), 11);
        assert_eq!(ServiceError::JobNotFound.to_string(), "The system can't find the specified job.");
        assert_eq!(ServiceError::LaunchDeniedByUser.to_string(), "The user denied the app's launch request.");
        let texts: std::collections::BTreeSet<String> = ServiceError::ALL.iter().map(|e| e.to_string()).collect();
        assert_eq!(texts.len(), 11);
    }
}
