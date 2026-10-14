//! Parity sdk-rest-08: headless tests of the failure capture and app sessions in `bifrost_app::exception_handler`, the
//! scheduler model in `bifrost_app::schedulers` and the WebDriver wire model in `bifrost_app::webdriver`. The tests use
//! temporary directories, in-memory values and short-lived queues. None of them opens a display or a device, and none
//! depends on the length of a delay.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use bifrost_app::dispatch::DispatchQueue;
use bifrost_app::exception_handler::{
    crash_records, end_session, handling, start_session, Decision, ExceptionHandler, ExceptionHandlerDelegate, Failure,
    FailureKind,
};
use bifrost_app::schedulers::{ImmediateScheduler, Scheduler};
use bifrost_app::webdriver::{
    element_reference, json_string, ErrorCode, Locator, Strategy, Timeouts, WebDriverError, ELEMENT_KEY, MAX_TIMEOUT_MS,
};

// --- failure capture ----------------------------------------------------------------------------------------------------

/// Refuses every logging decision and approves every handling decision.
struct RefuseLog;

impl ExceptionHandlerDelegate for RefuseLog {
    fn should_log(&self, _failure: &Failure, _mask: u32) -> bool {
        false
    }
}

fn logging_handler() -> ExceptionHandler {
    let mut handler = ExceptionHandler::new();
    handler.set_handling_mask(handling::LOG_UNCAUGHT);
    handler
}

#[test]
fn a_monitored_failure_is_appended_to_the_crash_log_in_the_shell_format() {
    let dir = tempfile::tempdir().unwrap();
    let failure = Failure::new(FailureKind::Uncaught, "boom", Some("src/a.rs:3:9"));
    let decision = logging_handler().capture(&failure, dir.path()).unwrap();
    assert_eq!(decision, Decision { handle: false, log: true, halt: false });
    let text = fs::read_to_string(dir.path().join("crash.log")).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.ends_with("': uncaught failure: boom at src/a.rs:3:9\n"), "text was {text:?}");
}

#[test]
fn a_captured_failure_reads_back_with_the_same_signature() {
    let dir = tempfile::tempdir().unwrap();
    let failure = Failure::new(FailureKind::Uncaught, "boom", Some("src/a.rs:3:9"));
    logging_handler().capture(&failure, dir.path()).unwrap();
    let records = crash_records(dir.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].signature, "uncaught failure: boom at src/a.rs:3:9");
}

#[test]
fn a_failure_outside_the_mask_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let failure = Failure::new(FailureKind::Uncaught, "boom", None);
    let decision = ExceptionHandler::new().capture(&failure, dir.path()).unwrap();
    assert_eq!(decision, Decision { handle: false, log: false, halt: false });
    assert!(!dir.path().join("crash.log").exists());
}

#[test]
fn a_handling_bit_alone_decides_without_writing_a_log_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut handler = ExceptionHandler::new();
    handler.set_handling_mask(handling::HANDLE_UNCAUGHT);
    let failure = Failure::new(FailureKind::Uncaught, "boom", None);
    assert_eq!(handler.capture(&failure, dir.path()).unwrap(), Decision { handle: true, log: false, halt: false });
    assert!(!dir.path().join("crash.log").exists());
}

#[test]
fn a_delegate_that_refuses_logging_keeps_the_failure_out_of_the_crash_log() {
    let dir = tempfile::tempdir().unwrap();
    let mut handler = logging_handler();
    handler.set_delegate(Some(Box::new(RefuseLog)));
    let failure = Failure::new(FailureKind::Uncaught, "boom", None);
    assert!(!handler.capture(&failure, dir.path()).unwrap().log);
    assert!(!dir.path().join("crash.log").exists());
}

#[test]
fn a_failure_without_a_location_logs_its_kind_and_message() {
    let failure = Failure::new(FailureKind::Other, "x", None);
    assert_eq!(failure.log_line(), "other failure: x");
}

#[test]
fn a_message_with_a_line_break_stays_on_one_log_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut handler = ExceptionHandler::new();
    handler.set_handling_mask(handling::LOG_TOP_LEVEL);
    handler.capture(&Failure::new(FailureKind::TopLevel, "a\nb", None), dir.path()).unwrap();
    let text = fs::read_to_string(dir.path().join("crash.log")).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.ends_with("': top level failure: a | b\n"), "text was {text:?}");
}

#[test]
fn a_capture_into_a_missing_directory_reports_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let failure = Failure::new(FailureKind::Uncaught, "boom", None);
    assert!(logging_handler().capture(&failure, &missing).is_err());
}

#[test]
fn a_capture_into_a_directory_that_is_not_utf8_reports_the_error() {
    let bytes: &[u8] = b"/tmp/sdk-rest-08-\xff";
    let dir = Path::new(std::ffi::OsStr::from_bytes(bytes));
    let failure = Failure::new(FailureKind::Uncaught, "boom", None);
    let err = logging_handler().capture(&failure, dir).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// --- app sessions: unclean exit at the next start -------------------------------------------------------------------------

#[test]
fn a_session_that_ended_with_end_session_is_clean_at_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(start_session(dir.path()).unwrap(), None);
    end_session(dir.path(), 0, "quit").unwrap();
    assert_eq!(start_session(dir.path()).unwrap(), None);
}

#[test]
fn a_session_without_end_session_is_unclean_at_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    start_session(dir.path()).unwrap();
    let note = start_session(dir.path()).unwrap().expect("the earlier session never ended");
    assert!(note.contains("unclean"), "note was {note:?}");
}

#[test]
fn end_session_records_the_status_and_reason() {
    let dir = tempfile::tempdir().unwrap();
    end_session(dir.path(), 4, "restore").unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("last_exit")).unwrap(), "4 restore\n");
}

#[test]
fn a_session_directory_that_is_not_utf8_reports_the_error() {
    let bytes: &[u8] = b"/tmp/sdk-rest-08-\xff";
    let dir = Path::new(std::ffi::OsStr::from_bytes(bytes));
    assert_eq!(start_session(dir).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(end_session(dir, 0, "quit").unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
}

// --- schedulers -----------------------------------------------------------------------------------------------------------

#[test]
fn an_immediate_scheduler_runs_work_before_schedule_returns() {
    let ran = Arc::new(AtomicUsize::new(0));
    let seen = ran.clone();
    ImmediateScheduler.schedule(move || {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[test]
fn an_immediate_scheduler_runs_delayed_work_at_once() {
    let ran = Arc::new(AtomicBool::new(false));
    let seen = ran.clone();
    ImmediateScheduler.schedule_after(Duration::from_secs(3600), Duration::ZERO, move || seen.store(true, Ordering::SeqCst));
    assert!(ran.load(Ordering::SeqCst), "the immediate scheduler does not wait out the delay");
}

#[test]
fn a_serial_queue_scheduler_runs_work_in_the_order_it_was_scheduled() {
    let queue = DispatchQueue::serial("sdk-rest-08 order");
    let (tx, rx) = mpsc::channel();
    for n in 1..=3 {
        let tx = tx.clone();
        queue.schedule(move || tx.send(n).unwrap());
    }
    let got: Vec<i32> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
    assert_eq!(got, vec![1, 2, 3]);
}

#[test]
fn delayed_work_on_a_queue_scheduler_still_runs() {
    let queue = DispatchQueue::serial("sdk-rest-08 delay");
    let (tx, rx) = mpsc::channel();
    queue.schedule_after(Duration::from_millis(20), Duration::from_millis(5), move || tx.send("late").unwrap());
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("late"));
}

/// Schedules two increments through any scheduler, so the trait is the only thing the code depends on.
fn increment_twice<S: Scheduler>(scheduler: &S, counter: &Arc<AtomicUsize>) {
    for _ in 0..2 {
        let c = counter.clone();
        scheduler.schedule(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
    }
}

#[test]
fn any_scheduler_is_used_through_the_trait() {
    let counter = Arc::new(AtomicUsize::new(0));
    increment_twice(&ImmediateScheduler, &counter);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

// --- WebDriver: error codes and statuses ----------------------------------------------------------------------------------

#[test]
fn every_error_code_has_a_wire_name_that_maps_back_to_itself() {
    for code in ErrorCode::ALL {
        assert_eq!(ErrorCode::from_wire(code.wire_name()), code);
    }
}

#[test]
fn an_error_name_the_specification_does_not_define_maps_to_unknown_error() {
    assert_eq!(ErrorCode::from_wire("no such unicorn"), ErrorCode::UnknownError);
}

#[test]
fn a_missing_element_is_a_404_no_such_element() {
    assert_eq!(ErrorCode::NoSuchElement.wire_name(), "no such element");
    assert_eq!(ErrorCode::NoSuchElement.http_status(), 404);
}

#[test]
fn a_script_timeout_is_a_408_and_an_unknown_method_is_a_405() {
    assert_eq!(ErrorCode::ScriptTimeout.http_status(), 408);
    assert_eq!(ErrorCode::UnknownMethod.http_status(), 405);
}

#[test]
fn a_protocol_error_takes_its_code_and_message_from_the_wire() {
    let err = WebDriverError::from_wire("stale element reference", "element is detached");
    assert_eq!(err.code, ErrorCode::StaleElementReference);
    assert_eq!(err.to_string(), "stale element reference: element is detached");
}

#[test]
fn an_error_without_a_message_prints_only_its_code() {
    assert_eq!(WebDriverError::new(ErrorCode::Timeout, "").to_string(), "timeout");
}

// --- WebDriver: locators, timeouts and element references -----------------------------------------------------------------

#[test]
fn a_css_locator_is_sent_with_its_strategy_name() {
    let locator = Locator::new(Strategy::CssSelector, "a.nav > li").unwrap();
    assert_eq!(locator.to_json(), r#"{"using":"css selector","value":"a.nav > li"}"#);
    assert_eq!(locator.strategy(), Strategy::CssSelector);
    assert_eq!(locator.value(), "a.nav > li");
}

#[test]
fn a_blank_locator_value_is_an_invalid_argument() {
    assert_eq!(Locator::new(Strategy::XPath, "   ").unwrap_err().code, ErrorCode::InvalidArgument);
}

#[test]
fn a_locator_value_with_a_control_character_is_an_invalid_argument() {
    assert_eq!(Locator::new(Strategy::LinkText, "Save\u{7}").unwrap_err().code, ErrorCode::InvalidArgument);
}

#[test]
fn a_tag_name_with_whitespace_is_an_invalid_selector() {
    assert_eq!(Locator::new(Strategy::TagName, "div span").unwrap_err().code, ErrorCode::InvalidSelector);
}

#[test]
fn a_tag_name_without_whitespace_is_accepted() {
    assert!(Locator::new(Strategy::TagName, "button").is_ok());
}

#[test]
fn quotes_in_a_locator_value_are_escaped_in_its_body() {
    let locator = Locator::new(Strategy::XPath, r#"//a[@title="Save"]"#).unwrap();
    assert_eq!(locator.to_json(), r#"{"using":"xpath","value":"//a[@title=\"Save\"]"}"#);
}

#[test]
fn json_strings_escape_backslashes_and_control_characters() {
    assert_eq!(json_string("a\\b\nc\u{1}"), r#""a\\b\nc\u0001""#);
}

#[test]
fn the_default_timeouts_are_the_specification_values() {
    assert_eq!(Timeouts::default(), Timeouts { script_ms: Some(30_000), page_load_ms: 300_000, implicit_ms: 0 });
}

#[test]
fn timeouts_are_sent_as_one_body_with_null_for_no_script_timeout() {
    let timeouts = Timeouts::new(None, 5_000, 250).unwrap();
    assert_eq!(timeouts.to_json(), r#"{"script":null,"pageLoad":5000,"implicit":250}"#);
}

#[test]
fn a_timeout_above_the_largest_javascript_integer_is_an_invalid_argument() {
    assert_eq!(MAX_TIMEOUT_MS, 9_007_199_254_740_991);
    assert_eq!(Timeouts::new(Some(MAX_TIMEOUT_MS + 1), 1, 0).unwrap_err().code, ErrorCode::InvalidArgument);
    assert!(Timeouts::new(Some(MAX_TIMEOUT_MS), 1, 0).is_ok());
}

#[test]
fn an_element_reference_uses_the_specification_key() {
    assert_eq!(ELEMENT_KEY, "element-6066-11e4-a52e-4f735466cecf");
    assert_eq!(element_reference("f.1").unwrap(), r#"{"element-6066-11e4-a52e-4f735466cecf":"f.1"}"#);
}

#[test]
fn a_blank_element_id_is_an_invalid_argument() {
    assert_eq!(element_reference("  ").unwrap_err().code, ErrorCode::InvalidArgument);
}
