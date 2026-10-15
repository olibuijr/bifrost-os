//! Login items, launch agents and launch daemons through the service handles, the service error values, and the
//! bundle lookups for embedded helpers and job property lists.
//!
//! Headless: every call runs against an in-memory context (`Ctx::testing`), the bundle file lists are plain slices and
//! the font is read from the assets folder. No shell, display or device is involved, and no time is read.

use bifrost_app::error::AppError;
use bifrost_app::kit::{Font, Ui};
use bifrost_app::proto::{Configure, LoginOp, LoginStatus, ToShell};
use bifrost_app::service_bundle::{
    find_login_item, helper_from_entry, helper_path_ok, helper_status, locate_job_plist, HelperItem, JobKind, LAUNCH_AGENTS_DIR,
    LAUNCH_DAEMONS_DIR, LOGIN_ITEMS_DIR,
};
use bifrost_app::service_management::{
    error_for_reply, open_system_settings_login_items, set_login_item_enabled, status_for_legacy_plist, LegacyLoginItem,
    SMAppService, ServiceError, Status, ERROR_DOMAIN,
};
use bifrost_app::Ctx;
use std::cell::Cell;
use std::rc::Rc;

fn ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

#[test]
fn error_values_follow_the_reference_numbering() {
    let expected = [
        (ServiceError::InternalFailure, 1),
        (ServiceError::InvalidSignature, 2),
        (ServiceError::AuthorizationFailure, 3),
        (ServiceError::ToolNotValid, 4),
        (ServiceError::JobNotFound, 5),
        (ServiceError::ServiceUnavailable, 6),
        (ServiceError::JobPlistNotFound, 7),
        (ServiceError::JobMustBeEnabled, 8),
        (ServiceError::InvalidPlist, 9),
        (ServiceError::LaunchDeniedByUser, 10),
        (ServiceError::AlreadyRegistered, 11),
    ];
    assert_eq!(ServiceError::ALL.len(), 11);
    for (error, code) in expected {
        assert_eq!(error.code(), code, "{error:?}");
        assert_eq!(ServiceError::from_code(code), Some(error));
    }
    assert_eq!(ServiceError::from_code(0), None);
    assert_eq!(ServiceError::from_code(12), None);
}

#[test]
fn error_values_carry_the_service_error_domain() {
    assert_eq!(ERROR_DOMAIN, "SMAppServiceErrorDomain");
    let error: AppError = ServiceError::JobNotFound.to_error();
    assert_eq!(error.domain(), "SMAppServiceErrorDomain");
    assert_eq!(error.code(), 5);
    assert!(error.is(ERROR_DOMAIN, 5));
    assert!(!error.is(ERROR_DOMAIN, 10));
    assert_eq!(ServiceError::LaunchDeniedByUser.to_error().localized_description(), "The user denied the app's launch request.");
}

#[test]
fn every_error_value_has_its_own_text() {
    let texts: std::collections::BTreeSet<String> = ServiceError::all().iter().map(|e| e.to_string()).collect();
    assert_eq!(texts.len(), ServiceError::all().len());
    assert_eq!(ServiceError::AlreadyRegistered.to_string(), "The application is already registered.");
    assert_eq!(ServiceError::JobPlistNotFound.to_string(), "The job's property list was not found in the app bundle.");
}

#[test]
fn a_refused_register_reply_maps_to_its_error_value() {
    assert_eq!(error_for_reply(LoginOp::Register, LoginStatus::NotRegistered), Some(ServiceError::LaunchDeniedByUser));
    assert_eq!(error_for_reply(LoginOp::Register, LoginStatus::NotFound), Some(ServiceError::JobNotFound));
    assert_eq!(error_for_reply(LoginOp::Register, LoginStatus::Enabled), None);
    assert_eq!(error_for_reply(LoginOp::Unregister, LoginStatus::NotRegistered), None);
}

#[test]
fn an_unknown_login_item_refuses_register_and_unregister_without_a_request() {
    let ctx = ctx();
    let item = SMAppService::login_item("com.example.helper");
    assert_eq!(item.status(), Status::NotFound);
    assert_eq!(item.try_register(&ctx), Err(ServiceError::JobNotFound));
    assert_eq!(item.try_unregister(&ctx), Err(ServiceError::JobNotFound));
    assert_eq!(item.register(&ctx), 0);
    assert_eq!(item.unregister(&ctx), 0);
    assert!(ctx.take_sent().is_empty());
}

#[test]
fn a_job_name_without_the_plist_suffix_is_invalid_before_the_bundle_is_read() {
    let ctx = ctx();
    assert_eq!(SMAppService::agent("com.example.agent").try_register(&ctx), Err(ServiceError::InvalidPlist));
    assert_eq!(SMAppService::daemon("dir/job.plist").try_register(&ctx), Err(ServiceError::InvalidPlist));
    assert_eq!(SMAppService::agent(".plist").try_unregister(&ctx), Err(ServiceError::InvalidPlist));
    assert!(ctx.take_sent().is_empty());
}

#[test]
fn a_valid_job_name_is_not_found_because_no_job_ships() {
    let ctx = ctx();
    assert_eq!(SMAppService::agent("com.example.agent.plist").try_register(&ctx), Err(ServiceError::JobPlistNotFound));
    assert_eq!(SMAppService::daemon("com.example.daemon.plist").try_unregister(&ctx), Err(ServiceError::JobPlistNotFound));
    assert_eq!(SMAppService::agent("com.example.agent.plist").status(), Status::NotFound);
    assert_eq!(SMAppService::daemon("com.example.daemon.plist").status(), Status::NotFound);
    assert_eq!(SMAppService::daemon("x.plist").refresh(&ctx), 0);
    assert!(ctx.take_sent().is_empty());
}

#[test]
fn unregister_with_reports_a_refused_call_at_once() {
    let ctx = ctx();
    let seen: Rc<Cell<Option<Option<ServiceError>>>> = Rc::new(Cell::new(None));
    let sink = seen.clone();
    let request = SMAppService::login_item("com.example.helper").unregister_with(&ctx, move |e| sink.set(Some(e)));
    assert_eq!(request, 0);
    assert_eq!(seen.get(), Some(Some(ServiceError::JobNotFound)));
    assert!(ctx.take_sent().is_empty());
}

#[test]
fn the_deprecated_enable_call_is_refused_without_a_request() {
    let ctx = ctx();
    assert_eq!(set_login_item_enabled(&ctx, "com.example.helper", true), Err(ServiceError::JobNotFound));
    assert_eq!(set_login_item_enabled(&ctx, "com.example.helper", false), Err(ServiceError::JobNotFound));
    assert!(ctx.take_sent().is_empty());
}

#[test]
fn open_settings_sends_one_open_pane_request() {
    let ctx = ctx();
    let id = open_system_settings_login_items(&ctx);
    assert_eq!(ctx.take_sent(), [ToShell::LoginItem { request: id, op: LoginOp::OpenSettings }]);
}

#[test]
fn legacy_plist_status_follows_the_shell_list() {
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
fn legacy_plist_paths_must_be_absolute_plist_paths_without_parent_steps() {
    let known = vec![LegacyLoginItem { plist_path: "helper.plist".into(), enabled: true }];
    assert_eq!(status_for_legacy_plist("helper.plist", &known), Status::NotFound, "relative path");
    assert_eq!(status_for_legacy_plist("/a/../b.plist", &known), Status::NotFound, "parent component");
    assert_eq!(status_for_legacy_plist("/a/helper.txt", &known), Status::NotFound, "wrong suffix");
    assert_eq!(status_for_legacy_plist("/.plist", &known), Status::NotFound, "no name before the suffix");
}

#[test]
fn bundle_directories_are_the_reference_layout() {
    assert_eq!(LOGIN_ITEMS_DIR, "Contents/Library/LoginItems");
    assert_eq!(LAUNCH_AGENTS_DIR, "Contents/Library/LaunchAgents");
    assert_eq!(LAUNCH_DAEMONS_DIR, "Contents/Library/LaunchDaemons");
    assert_eq!(JobKind::Agent.dir(), LAUNCH_AGENTS_DIR);
    assert_eq!(JobKind::Daemon.dir(), LAUNCH_DAEMONS_DIR);
}

#[test]
fn an_embedded_helper_must_be_an_app_directly_in_the_login_item_directory() {
    assert!(helper_path_ok("Contents/Library/LoginItems/Updater.app"));
    assert!(!helper_path_ok("Contents/Library/LoginItems/Updater"), "no app suffix");
    assert!(!helper_path_ok("Contents/Library/LoginItems/nested/Updater.app"), "nested directory");
    assert!(!helper_path_ok("Contents/Library/LoginItems/../Updater.app"), "parent step");
    assert!(!helper_path_ok("Contents/Library/LaunchAgents/Updater.app"), "other directory");
}

#[test]
fn a_helper_entry_needs_a_path_and_a_non_empty_identifier() {
    let item = helper_from_entry("Contents/Library/LoginItems/Updater.app", Some(" org.example.updater ")).unwrap();
    assert_eq!(item.bundle_id, "org.example.updater");
    assert_eq!(item.path, "Contents/Library/LoginItems/Updater.app");
    assert_eq!(helper_from_entry("Contents/Library/LoginItems/Updater.app", None), None);
    assert_eq!(helper_from_entry("Contents/Library/LoginItems/Updater.app", Some("  ")), None);
    assert_eq!(helper_from_entry("Contents/Resources/Updater.app", Some("org.example.updater")), None);
}

fn helpers() -> Vec<HelperItem> {
    ["org.example.updater", "org.example.sync"]
        .iter()
        .map(|id| helper_from_entry(&format!("Contents/Library/LoginItems/{id}.app"), Some(*id)).unwrap())
        .collect()
}

#[test]
fn a_login_item_identifier_finds_its_embedded_helper() {
    let items = helpers();
    assert_eq!(find_login_item("org.example.sync", &items).unwrap().path, "Contents/Library/LoginItems/org.example.sync.app");
    assert_eq!(find_login_item("org.example.missing", &items), Err(ServiceError::JobNotFound));
    assert_eq!(find_login_item("", &items), Err(ServiceError::JobNotFound));
    assert_eq!(find_login_item("org.example/updater", &items), Err(ServiceError::JobNotFound));
    assert_eq!(find_login_item("org.example.updater", &[]), Err(ServiceError::JobNotFound));
}

#[test]
fn a_helper_reports_enabled_only_when_the_user_switched_it_on() {
    let items = helpers();
    assert_eq!(helper_status("org.example.gone", &items, &["org.example.updater"]), Status::NotFound);
    assert_eq!(helper_status("org.example.updater", &items, &["org.example.updater"]), Status::Enabled);
    assert_eq!(helper_status("org.example.updater", &items, &[]), Status::NotRegistered);
    assert_eq!(helper_status("org.example.updater", &items, &["org.example.other"]), Status::NotRegistered);
}

#[test]
fn a_job_plist_is_found_only_in_its_own_directory() {
    let files = ["Contents/Library/LaunchAgents/com.example.agent.plist", "Contents/Library/LaunchDaemons/com.example.daemon.plist"];
    assert_eq!(
        locate_job_plist(JobKind::Agent, "com.example.agent.plist", &files),
        Ok("Contents/Library/LaunchAgents/com.example.agent.plist".to_string())
    );
    assert_eq!(
        locate_job_plist(JobKind::Daemon, "com.example.daemon.plist", &files),
        Ok("Contents/Library/LaunchDaemons/com.example.daemon.plist".to_string())
    );
    assert_eq!(locate_job_plist(JobKind::Daemon, "com.example.agent.plist", &files), Err(ServiceError::JobPlistNotFound));
    assert_eq!(locate_job_plist(JobKind::Agent, "com.example.gone.plist", &files), Err(ServiceError::JobPlistNotFound));
}

#[test]
fn a_job_plist_name_is_checked_before_the_bundle_is_searched() {
    let files = ["Contents/Library/LaunchAgents/com.example.agent"];
    assert_eq!(locate_job_plist(JobKind::Agent, "com.example.agent", &files), Err(ServiceError::InvalidPlist));
    assert_eq!(locate_job_plist(JobKind::Agent, "dir/job.plist", &files), Err(ServiceError::InvalidPlist));
    assert_eq!(locate_job_plist(JobKind::Daemon, ".plist", &files), Err(ServiceError::InvalidPlist));
}
