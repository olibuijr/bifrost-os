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
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

static LAST: AtomicU8 = AtomicU8::new(0);

type Completion = Box<dyn FnOnce(Option<ServiceError>)>;

thread_local! {
    static PENDING: RefCell<HashMap<u32, Completion>> = RefCell::new(HashMap::new());
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
        check_main_app(&self.0, LoginOp::Register, self.status())?;
        Ok(send(ctx, LoginOp::Register))
    }
    /// `unregister()`. Returns 0 when no request was sent; `try_unregister` says why.
    pub fn unregister(&self, ctx: &Ctx) -> u32 {
        self.try_unregister(ctx).unwrap_or(0)
    }
    /// `unregister()` as a `Result`, like `try_register`.
    pub fn try_unregister(&self, ctx: &Ctx) -> Result<u32, ServiceError> {
        self.precheck()?;
        check_main_app(&self.0, LoginOp::Unregister, self.status())?;
        Ok(send(ctx, LoginOp::Unregister))
    }
    /// `unregister(completionHandler:)`: `done(None)` when the item is gone, `done(Some(error))` when not. A call that
    /// fails before reaching the shell (unknown item, no plist) calls `done` at once and returns 0.
    pub fn unregister_with(&self, ctx: &Ctx, done: impl FnOnce(Option<ServiceError>) + 'static) -> u32 {
        match self.try_unregister(ctx) {
            Err(e) => {
                done(Some(e));
                0
            }
            Ok(request) => {
                PENDING.with(|p| p.borrow_mut().insert(request, Box::new(done)));
                request
            }
        }
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

/// `openSystemSettingsLoginItems()`: the shell opens Settings on the Login Items page, where the user approves or removes
/// login items. Returns the request id (the shell answers with the current status).
pub fn open_system_settings_login_items(ctx: &Ctx) -> u32 {
    send(ctx, LoginOp::OpenSettings)
}

/// The errors the main app's `register` and `unregister` give from the last status alone: registering when already
/// registered or waiting for approval is `AlreadyRegistered`; unregistering when not registered is `JobNotFound`.
fn check_main_app(kind: &Kind, op: LoginOp, status: Status) -> Result<(), ServiceError> {
    if *kind != Kind::MainApp {
        return Ok(());
    }
    match (op, status) {
        (LoginOp::Register, Status::Enabled | Status::RequiresApproval) => Err(ServiceError::AlreadyRegistered),
        (LoginOp::Unregister, Status::NotRegistered | Status::NotFound) => Err(ServiceError::JobNotFound),
        _ => Ok(()),
    }
}

/// The error a shell reply to `op` means: a `register` answered `NotRegistered` was refused in Login Items
/// (`LaunchDeniedByUser`), answered `NotFound` it names no item (`JobNotFound`).
pub fn error_for_reply(op: LoginOp, status: Status) -> Option<ServiceError> {
    match (op, status) {
        (LoginOp::Register, Status::NotRegistered) => Some(ServiceError::LaunchDeniedByUser),
        (LoginOp::Register, Status::NotFound) => Some(ServiceError::JobNotFound),
        _ => None,
    }
}

/// Finishes the `unregister_with` call of `request` when the shell's reply arrives.
pub(crate) fn complete(request: u32, status: Status) {
    if let Some(done) = PENDING.with(|p| p.borrow_mut().remove(&request)) {
        done(if status == Status::NotRegistered { None } else { Some(ServiceError::InternalFailure) });
    }
}

/// A property list name: no path separator, and a `.plist` suffix with something before it.
pub(crate) fn valid_plist_name(name: &str) -> bool {
    !name.contains('/') && name.len() > ".plist".len() && name.ends_with(".plist")
}

fn send(ctx: &Ctx, op: LoginOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::LoginItem { request, op });
    request
}

/// A login item an earlier release of the desktop registered with a property list, as the shell's list holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyLoginItem {
    pub plist_path: String,
    pub enabled: bool,
}

/// `statusForLegacyPlist(at:)`: the status of a login item that an earlier release registered with a property list at
/// `path`. The path must be absolute, end in `.plist` and hold no `..` component; any other path is `NotFound`, as is a
/// path the shell does not list. An enabled entry is `Enabled`, a disabled one `NotRegistered`. There is no approval
/// step, so `RequiresApproval` is never returned.
pub fn status_for_legacy_plist(path: &str, known: &[LegacyLoginItem]) -> Status {
    if !valid_legacy_path(path) {
        return Status::NotFound;
    }
    match known.iter().find(|item| item.plist_path == path) {
        Some(item) if item.enabled => Status::Enabled,
        Some(_) => Status::NotRegistered,
        None => Status::NotFound,
    }
}

fn valid_legacy_path(path: &str) -> bool {
    path.starts_with('/') && path.ends_with(".plist") && path.len() > ".plist".len() + 1 && !path.split('/').any(|c| c == "..")
}

/// The deprecated enable-by-identifier call. It toggles a helper login item by identifier, and the helper lives in the
/// app bundle. No helper bundles ship, so the identifier names no item: the call fails with `JobNotFound`, exactly as
/// `SMAppService::login_item(identifier)` does, and nothing is sent to the shell. Apps should use `SMAppService`.
pub fn set_login_item_enabled(ctx: &Ctx, identifier: &str, enabled: bool) -> Result<u32, ServiceError> {
    let handle = SMAppService::login_item(identifier);
    if enabled {
        handle.try_register(ctx)
    } else {
        handle.try_unregister(ctx)
    }
}

pub(crate) fn mirror(status: Status) {
    LAST.store(status as u8, Ordering::Relaxed);
}

/// The error domain of [`ServiceError::to_error`].
pub const ERROR_DOMAIN: &str = "SMAppServiceErrorDomain";

/// The errors a registration or removal can report (the cases of the Service Management Errors reference).
/// [`ServiceError::code`] is the `kSMError*` value of the reference platform, release 27 (`kSMErrorInternalFailure` 1 to
/// `kSMErrorAlreadyRegistered` 11).
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
    /// Every case, in the order of the official list.
    pub const ALL: [ServiceError; 11] = ALL_ERRORS;

    /// Every case, in declaration order.
    pub fn all() -> &'static [ServiceError] {
        &ALL_ERRORS
    }

    /// The `kSMError*` value of this case.
    pub fn code(self) -> i64 {
        match self {
            ServiceError::InternalFailure => 1,
            ServiceError::InvalidSignature => 2,
            ServiceError::AuthorizationFailure => 3,
            ServiceError::ToolNotValid => 4,
            ServiceError::JobNotFound => 5,
            ServiceError::ServiceUnavailable => 6,
            ServiceError::JobPlistNotFound => 7,
            ServiceError::JobMustBeEnabled => 8,
            ServiceError::InvalidPlist => 9,
            ServiceError::LaunchDeniedByUser => 10,
            ServiceError::AlreadyRegistered => 11,
        }
    }

    /// The case with this `kSMError*` value, or `None` for a number outside 1 to 11.
    pub fn from_code(code: i64) -> Option<ServiceError> {
        ALL_ERRORS.iter().copied().find(|e| e.code() == code)
    }

    /// The case as an [`AppError`] in [`ERROR_DOMAIN`], with its text as the description.
    pub fn to_error(self) -> crate::error::AppError {
        crate::error::AppError::new(ERROR_DOMAIN, self.code()).with_description(self.to_string())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Mutex;

    /// The last status is process-wide, so the tests that read or set it run one at a time.
    static GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn registering_twice_is_already_registered_and_sends_nothing() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = crate::runtime::test_ctx();
        for status in [Status::Enabled, Status::RequiresApproval] {
            mirror(status);
            assert_eq!(SMAppService::main_app().try_register(&ctx), Err(ServiceError::AlreadyRegistered));
        }
        mirror(Status::NotRegistered);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn unregistering_an_item_that_is_not_registered_is_job_not_found() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = crate::runtime::test_ctx();
        mirror(Status::NotRegistered);
        assert_eq!(SMAppService::main_app().try_unregister(&ctx), Err(ServiceError::JobNotFound));
        let seen = Rc::new(Cell::new(None));
        let sink = seen.clone();
        assert_eq!(SMAppService::main_app().unregister_with(&ctx, move |e| sink.set(Some(e))), 0);
        assert_eq!(seen.get(), Some(Some(ServiceError::JobNotFound)));
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn unregister_with_completes_when_the_shell_answers() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = crate::runtime::test_ctx();
        mirror(Status::Enabled);
        let seen = Rc::new(Cell::new(None));
        let sink = seen.clone();
        let request = SMAppService::main_app().unregister_with(&ctx, move |e| sink.set(Some(e)));
        assert_eq!(ctx.take_sent(), [ToShell::LoginItem { request, op: LoginOp::Unregister }]);
        assert_eq!(seen.get(), None, "not before the reply");
        complete(request, Status::NotRegistered);
        assert_eq!(seen.get(), Some(None));
        mirror(Status::NotRegistered);
    }

    #[test]
    fn a_register_the_user_refused_is_launch_denied_by_user() {
        assert_eq!(error_for_reply(LoginOp::Register, Status::NotRegistered), Some(ServiceError::LaunchDeniedByUser));
        assert_eq!(error_for_reply(LoginOp::Register, Status::NotFound), Some(ServiceError::JobNotFound));
        assert_eq!(error_for_reply(LoginOp::Register, Status::RequiresApproval), None);
        assert_eq!(error_for_reply(LoginOp::Unregister, Status::NotRegistered), None);
    }

    #[test]
    fn open_system_settings_login_items_sends_the_open_pane_request() {
        let ctx = crate::runtime::test_ctx();
        let id = open_system_settings_login_items(&ctx);
        assert_eq!(ctx.take_sent(), [ToShell::LoginItem { request: id, op: LoginOp::OpenSettings }]);
    }

    #[test]
    fn a_plist_name_without_the_suffix_is_invalid_plist() {
        let ctx = crate::runtime::test_ctx();
        assert_eq!(SMAppService::agent("com.example.agent").try_register(&ctx), Err(ServiceError::InvalidPlist));
    }

    #[test]
    fn service_error_codes_are_one_to_eleven_and_round_trip() {
        assert_eq!(ServiceError::all().len(), 11);
        assert_eq!(ServiceError::InternalFailure.code(), 1);
        assert_eq!(ServiceError::InvalidSignature.code(), 2);
        assert_eq!(ServiceError::AuthorizationFailure.code(), 3);
        assert_eq!(ServiceError::ToolNotValid.code(), 4);
        assert_eq!(ServiceError::JobNotFound.code(), 5);
        assert_eq!(ServiceError::ServiceUnavailable.code(), 6);
        assert_eq!(ServiceError::JobPlistNotFound.code(), 7);
        assert_eq!(ServiceError::JobMustBeEnabled.code(), 8);
        assert_eq!(ServiceError::InvalidPlist.code(), 9);
        assert_eq!(ServiceError::LaunchDeniedByUser.code(), 10);
        assert_eq!(ServiceError::AlreadyRegistered.code(), 11);
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
        assert!(e.is(ERROR_DOMAIN, 10));
        assert!(!e.is(ERROR_DOMAIN, 9));
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
        assert_eq!(ServiceError::AlreadyRegistered.to_string(), "The application is already registered.");
        assert_eq!(ServiceError::InvalidSignature.to_string(), "The app's code signature doesn't meet the requirements to perform the operation.");
    }

    #[test]
    fn the_domain_is_the_reference_service_domain() {
        assert_eq!(ServiceError::InternalFailure.to_error().domain(), "SMAppServiceErrorDomain");
        assert_eq!(ServiceError::InternalFailure.to_error().code(), 1);
    }

    #[test]
    fn calls_go_to_the_shell_and_status_follows_the_report() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        mirror(Status::NotRegistered);
        let ctx = crate::runtime::test_ctx();
        let s = SMAppService::main_app();
        assert_eq!(s.status(), Status::NotRegistered);
        let a = s.register(&ctx);
        let b = s.refresh(&ctx);
        mirror(Status::Enabled);
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        mirror(Status::NotRegistered);
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
    fn legacy_plist_status_needs_an_absolute_plist_path_the_shell_lists() {
        let known = vec![
            LegacyLoginItem { plist_path: "/Library/Launch/helper.plist".into(), enabled: true },
            LegacyLoginItem { plist_path: "/Library/Launch/old.plist".into(), enabled: false },
        ];
        assert_eq!(status_for_legacy_plist("/Library/Launch/helper.plist", &known), Status::Enabled);
        assert_eq!(status_for_legacy_plist("/Library/Launch/old.plist", &known), Status::NotRegistered);
        assert_eq!(status_for_legacy_plist("/Library/Launch/gone.plist", &known), Status::NotFound);
        assert_eq!(status_for_legacy_plist("/Library/Launch/helper.plist", &[]), Status::NotFound);
    }

    #[test]
    fn legacy_plist_paths_that_are_not_plist_paths_are_not_found() {
        let known = vec![LegacyLoginItem { plist_path: "helper.plist".into(), enabled: true }];
        assert_eq!(status_for_legacy_plist("helper.plist", &known), Status::NotFound, "relative path");
        assert_eq!(status_for_legacy_plist("/a/../b.plist", &known), Status::NotFound, "parent component");
        assert_eq!(status_for_legacy_plist("/a/helper.txt", &known), Status::NotFound, "wrong suffix");
        assert_eq!(status_for_legacy_plist("/.plist", &known), Status::NotFound, "no name before the suffix");
    }

    #[test]
    fn the_deprecated_enable_call_fails_with_job_not_found_and_sends_nothing() {
        let ctx = crate::runtime::test_ctx();
        assert_eq!(set_login_item_enabled(&ctx, "com.example.helper", true), Err(ServiceError::JobNotFound));
        assert_eq!(set_login_item_enabled(&ctx, "com.example.helper", false), Err(ServiceError::JobNotFound));
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
