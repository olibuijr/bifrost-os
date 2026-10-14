//! Parity sdk-rest-07: headless tests for the crash record reader and the cloud identity model. No test touches a file,
//! the network or a device.

use bifrost_app::cloud_identity::{self, Error as IdentityError, UserIdentity};
use bifrost_app::crash_record::{self, CrashEntry, Error as RecordError, LastExit, Panic};

#[test]
fn a_crash_log_line_gives_its_time_thread_and_message() {
    let entry = crash_record::parse_entry("[1700000000 12:00:01] thread 'main': panic: boom at a.rs:3:9").unwrap();
    assert_eq!(
        entry,
        CrashEntry { unix_time: 1700000000, thread: "main".into(), message: "panic: boom at a.rs:3:9".into() }
    );
}

#[test]
fn a_line_break_inside_a_message_stays_as_the_shell_wrote_it() {
    let entry = crash_record::parse_entry("[1 00:00:01] thread 'unnamed': a | b").unwrap();
    assert_eq!(entry.thread, "unnamed");
    assert_eq!(entry.message, "a | b");
}

#[test]
fn a_line_without_a_stamp_is_a_missing_stamp_error() {
    let err = crash_record::parse_entry("thread 'main': boom").unwrap_err();
    assert!(matches!(err, RecordError::MissingStamp(_)));
}

#[test]
fn a_stamp_whose_time_is_not_a_number_is_a_bad_time_error() {
    let err = crash_record::parse_entry("[soon 12:00:01] thread 'main': boom").unwrap_err();
    assert!(matches!(err, RecordError::BadTime(_)));
}

#[test]
fn a_line_without_a_thread_part_is_a_missing_thread_error() {
    let err = crash_record::parse_entry("[1 00:00:01] just text").unwrap_err();
    assert!(matches!(err, RecordError::MissingThread(_)));
}

#[test]
fn a_whole_log_skips_blank_lines_and_keeps_a_bad_line_as_an_error() {
    let text = "[1 00:00:01] thread 'a': first\n\n[2 00:00:02] thread 'a': second\n[3 00:00:0";
    let results = crash_record::parse_log(text);
    assert_eq!(results.len(), 3, "the blank line is skipped, the partial last line is kept");
    assert_eq!(results[0].as_ref().unwrap().message, "first");
    assert_eq!(results[1].as_ref().unwrap().unix_time, 2);
    assert!(results[2].is_err(), "a partly written line must not vanish silently");
}

#[test]
fn a_panic_with_a_location_gives_message_file_line_and_column() {
    assert_eq!(
        crash_record::parse_panic("panic: boom at a.rs:3:9"),
        Some(Panic::At { message: "boom".into(), file: "a.rs".into(), line: 3, column: 9 })
    );
}

#[test]
fn a_panic_message_that_contains_at_keeps_its_own_text() {
    assert_eq!(
        crash_record::parse_panic("panic: index at 2 at src/lib.rs:10:5"),
        Some(Panic::At { message: "index at 2".into(), file: "src/lib.rs".into(), line: 10, column: 5 })
    );
}

#[test]
fn a_panic_without_a_known_location_gives_only_its_message() {
    assert_eq!(
        crash_record::parse_panic("panic: boom at unknown location"),
        Some(Panic::UnknownLocation { message: "boom".into() })
    );
}

#[test]
fn a_message_that_is_not_a_panic_gives_no_panic() {
    assert_eq!(crash_record::parse_panic("app notes quit unexpectedly: exit 1"), None);
    assert_eq!(crash_record::parse_panic("panic: boom at a.rs:x:9"), None, "a non-numeric line is not a location");
    assert_eq!(crash_record::parse_panic("panic: boom at :3:9"), None, "a location without a file is not a location");
}

#[test]
fn last_exit_gives_the_status_and_the_whole_reason() {
    assert_eq!(
        crash_record::parse_last_exit("5 ui loop stalled for 20s\n"),
        Ok(LastExit { status: 5, reason: "ui loop stalled for 20s".into() })
    );
}

#[test]
fn last_exit_without_a_numeric_status_is_a_bad_exit_error() {
    assert!(matches!(crash_record::parse_last_exit("restore"), Err(RecordError::BadExit(_))));
    assert!(matches!(crash_record::parse_last_exit("x restore"), Err(RecordError::BadExit(_))));
    assert!(matches!(crash_record::parse_last_exit(""), Err(RecordError::BadExit(_))));
}

#[test]
fn a_start_after_a_run_without_last_exit_is_an_unclean_run() {
    assert!(crash_record::ended_uncleanly(true, None));
    let exit = LastExit { status: 0, reason: "quit".into() };
    assert!(!crash_record::ended_uncleanly(true, Some(&exit)));
}

#[test]
fn a_first_start_is_not_an_unclean_run() {
    assert!(!crash_record::ended_uncleanly(false, None));
}

#[test]
fn the_record_file_names_match_the_shell_writer() {
    assert_eq!(crash_record::CRASH_LOG_FILE, "crash.log");
    assert_eq!(crash_record::ROTATED_CRASH_LOG_FILE, "crash.log.old");
    assert_eq!(crash_record::LAST_EXIT_FILE, "last_exit");
    assert_eq!(crash_record::LAST_START_FILE, "last_start");
}

fn me_body(email: &str, name: &str, id: &str, admin: bool) -> String {
    format!(
        "{{\"account\":{{\"id\":\"{id}\",\"email\":\"{email}\",\"name\":\"{name}\",\"avatar\":\"\",\"created_at\":1700000000,\"is_admin\":{admin}}},\"session_id\":\"s1\"}}"
    )
}

#[test]
fn a_signed_in_answer_gives_the_users_identity() {
    let body = me_body("ada@example.com", "Ada", "acc-1", true);
    assert_eq!(
        cloud_identity::identity_from_me(200, &body),
        Ok(UserIdentity {
            user_record_name: "acc-1".into(),
            display_name: "Ada".into(),
            lookup_email: "ada@example.com".into(),
            is_admin: true,
        })
    );
}

#[test]
fn an_empty_display_name_is_kept_as_empty() {
    let body = me_body("bo@example.com", "", "acc-2", false);
    let identity = cloud_identity::identity_from_me(200, &body).unwrap();
    assert_eq!(identity.display_name, "");
    assert!(!identity.is_admin);
}

#[test]
fn an_unauthorized_answer_is_not_signed_in() {
    assert_eq!(cloud_identity::identity_from_me(401, "{\"error\":\"unauthorized\"}"), Err(IdentityError::NotSignedIn));
}

#[test]
fn any_other_status_is_an_unexpected_error_with_the_status() {
    assert_eq!(cloud_identity::identity_from_me(500, ""), Err(IdentityError::Unexpected { status: 500 }));
}

#[test]
fn a_body_that_is_not_json_is_malformed() {
    assert!(matches!(cloud_identity::identity_from_me(200, "not json"), Err(IdentityError::Malformed(_))));
}

#[test]
fn an_answer_without_an_account_is_malformed() {
    assert!(matches!(cloud_identity::identity_from_me(200, "{\"session_id\":\"s1\"}"), Err(IdentityError::Malformed(_))));
}

#[test]
fn an_empty_account_id_is_malformed() {
    let body = me_body("ada@example.com", "Ada", "", false);
    assert!(matches!(cloud_identity::identity_from_me(200, &body), Err(IdentityError::Malformed(_))));
}

#[test]
fn an_email_without_an_at_sign_is_malformed() {
    let body = me_body("ada", "Ada", "acc-1", false);
    assert!(matches!(cloud_identity::identity_from_me(200, &body), Err(IdentityError::Malformed(_))));
}

#[test]
fn a_missing_admin_flag_is_malformed() {
    let body = "{\"account\":{\"id\":\"acc-1\",\"email\":\"ada@example.com\",\"name\":\"Ada\"}}";
    assert!(matches!(cloud_identity::identity_from_me(200, body), Err(IdentityError::Malformed(_))));
}

#[test]
fn an_error_message_names_the_status_or_the_reason() {
    assert_eq!(IdentityError::Unexpected { status: 503 }.to_string(), "the cloud service answered with status 503");
    assert_eq!(IdentityError::NotSignedIn.to_string(), "no account is signed in to the cloud service");
}
