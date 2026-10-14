//! Parity sdk-tests-01: headless tests of the shell's models for shortcuts, the local account, the login window and its
//! authentication limits, crash and diagnostics bundles, and the photo slideshow. Each test runs the model in memory:
//! no display is drawn, no helper is called and no device is opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use springboard::privd::auth::{lockout_secs, parse_fails, parse_hint, AuthError, HINT_MAX, MIN_PASSWORD};
use springboard::shell::account::{parse as parse_account, password_change_problem, password_status_needs_set};
use springboard::shell::diag::{bundle, current_ma, date_text, delete_all, max_temp, parse_line, parse_log, parse_mem, post, signature, upload_url};
use springboard::shell::intents::{parse_reply as parse_intent_reply, request_line, request_line_with, ManifestIntent};
use springboard::shell::login::{autologin_name, parse_lock_message, parse_reply, power_effect, AccountInfo, Act, Backend, Login, Reply, HINT_AFTER, MESSAGE_MAX, SHAKE_MS};
use springboard::shell::photos::library::Photo;
use springboard::shell::photos::slideshow::{ken_burns, Slideshow, CONTROLS_MS, FADE_MS, SLIDE_MS};

// -- shortcuts in Search -----------------------------------------------------------------------------------------------

fn shortcut(params: &[&str]) -> ManifestIntent {
    ManifestIntent {
        id: "new-note".into(),
        title: "New Note".into(),
        shortcut: true,
        params: params.iter().map(|p| p.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn headless_only_parameterless_shortcuts_are_listed_in_search() {
    assert!(shortcut(&[]).runnable());
    assert!(shortcut(&["due:date?"]).runnable(), "an optional parameter does not block the shortcut");
    assert!(!shortcut(&["text:string"]).runnable(), "a required parameter needs a prompt the shell does not have");
    let mut hidden = shortcut(&[]);
    hidden.shortcut = false;
    assert!(!hidden.runnable(), "an intent not flagged as a shortcut stays out of Search");
}

#[test]
fn headless_a_shortcut_needs_a_title_and_a_plain_id() {
    let mut untitled = shortcut(&[]);
    untitled.title.clear();
    assert!(!untitled.runnable());
    for id in ["", "a\"b", "run now", "x/../y"] {
        let mut bad = shortcut(&[]);
        bad.id = id.into();
        assert!(!bad.runnable(), "id {id:?} is not a safe method name");
    }
    let mut dashed = shortcut(&[]);
    dashed.id = "open_note-2".into();
    assert!(dashed.runnable());
}

#[test]
fn headless_a_shortcut_request_names_the_intent_method() {
    assert_eq!(request_line("new-note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
}

#[test]
fn headless_string_parameters_are_escaped_in_the_request() {
    assert_eq!(
        request_line_with("create-note", &[("text", "a \"quote\"\nline\\")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\nline\\\\\"}}\n"
    );
    assert_eq!(
        request_line_with("look-up", &[("word", "cat"), ("lang", "is")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\",\"lang\":\"is\"}}\n"
    );
}

#[test]
fn headless_an_intent_reply_yields_its_dialog_or_its_error() {
    assert_eq!(parse_intent_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#), Ok(Some("Created".into())));
    assert_eq!(parse_intent_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#), Ok(None));
    assert_eq!(
        parse_intent_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#),
        Err("no intent x".into())
    );
    assert!(parse_intent_reply("nonsense").is_err());
}

// -- local account -----------------------------------------------------------------------------------------------------

#[test]
fn headless_the_installer_record_gives_the_full_name_and_the_user_name() {
    let a = parse_account("full_name=Ada Lovelace\nusername=ada\n").unwrap();
    assert_eq!((a.full_name.as_str(), a.username.as_str()), ("Ada Lovelace", "ada"));
}

#[test]
fn headless_an_account_without_a_full_name_uses_the_user_name() {
    let a = parse_account("username=ada\nfull_name=\n").unwrap();
    assert_eq!(a.full_name, "ada");
    assert!(parse_account("full_name=Ada Lovelace\n").is_none(), "a record without a user name is no account");
}

#[test]
fn headless_an_unset_or_locked_password_asks_for_one_at_first_run() {
    assert!(password_status_needs_set("ada NP 2026-10-01 0 99999 7 -1"));
    assert!(password_status_needs_set("ada L 2026-10-01 0 99999 7 -1"));
    assert!(password_status_needs_set("ada LK 2026-10-01 0 99999 7 -1"));
    assert!(!password_status_needs_set("ada P 2026-10-01 0 99999 7 -1"));
    assert!(!password_status_needs_set(""));
}

#[test]
fn headless_a_password_change_is_checked_before_the_helper_sees_it() {
    assert_eq!(password_change_problem("old-pass", "short", "short"), Some("The new password needs at least 8 characters."));
    assert_eq!(password_change_problem("old-pass", "a:bcdefgh", "a:bcdefgh"), Some("The password cannot contain a colon or a line break."));
    assert_eq!(password_change_problem("old-pass", "longenough", "longenugh"), Some("The two passwords do not match."));
    assert_eq!(password_change_problem("longenough", "longenough", "longenough"), Some("Choose a password that is different from the current one."));
    assert_eq!(password_change_problem("old-pass", "longenough", "longenough"), None);
}

// -- login window and its authentication limits ------------------------------------------------------------------------

struct Helper(Option<AccountInfo>);

impl Backend for Helper {
    fn authenticate(&self, _password: &str) -> Reply {
        Reply::Unavailable
    }
    fn account(&self) -> Option<AccountInfo> {
        self.0.clone()
    }
}

fn account(full_name: &str, hint: &str, has_password: bool, locked_out_secs: u64) -> AccountInfo {
    AccountInfo { full_name: full_name.into(), username: "ada".into(), has_password, locked_out_secs, password_hint: hint.into() }
}

fn login_for(info: Option<AccountInfo>) -> Login {
    Login::new(Arc::new(Helper(info)), "Fallback Name", 1200, 800, 0.0)
}

#[test]
fn headless_the_login_window_takes_its_name_and_hint_from_the_helper() {
    let l = login_for(Some(account("Ada Lovelace", "my cat", true, 0)));
    assert_eq!(l.name, "Ada Lovelace");
    assert_eq!(l.username, "ada");
    assert_eq!(l.hint, "my cat");
    assert_eq!(l.msg, springboard::shell::login::Msg::None);
    assert_eq!(login_for(Some(account("", "", true, 0))).name, "Fallback Name", "a blank name keeps the fallback");
}

#[test]
fn headless_a_wrong_password_shakes_and_says_so() {
    let mut l = login_for(Some(account("Ada", "", true, 0)));
    l.apply(Reply::Wrong { retry_secs: 0 }, 1000.0);
    assert_eq!(l.message(1000.0), "Incorrect password");
    assert!(l.shaking(1000.0));
    assert!(!l.shaking(1000.0 + SHAKE_MS), "the shake ends after SHAKE_MS");
    assert!(l.shake_phase(1000.0).abs() < 1e-6, "the shake starts at rest");
}

#[test]
fn headless_the_hint_appears_after_three_wrong_passwords() {
    let mut l = login_for(Some(account("Ada", "my cat", true, 0)));
    for _ in 0..HINT_AFTER - 1 {
        l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
    }
    assert_eq!(l.hint_line(), None);
    l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
    assert_eq!(l.hint_line().as_deref(), Some("Hint: my cat"));
}

#[test]
fn headless_a_lockout_counts_down_in_minutes_then_seconds() {
    let mut l = login_for(Some(account("Ada", "", true, 0)));
    l.apply(Reply::Wrong { retry_secs: 60 }, 1000.0);
    assert!(l.locked(1000.0));
    assert_eq!(l.secs_left(1000.0), 60);
    assert_eq!(l.message(1000.0), "Try again in 1 minute");
    assert_eq!(l.message(31_000.0), "Try again in 30 seconds");
    assert!(!l.locked(61_000.0), "the lock ends exactly at the retry time");
    assert_eq!(l.message(61_000.0), "");
}

#[test]
fn headless_a_long_lockout_is_shown_in_minutes() {
    let mut l = login_for(Some(account("Ada", "", true, 0)));
    l.apply(Reply::LockedOut { retry_secs: 120 }, 0.0);
    assert_eq!(l.message(0.0), "Try again in 2 minutes");
    let mut short = login_for(Some(account("Ada", "", true, 0)));
    short.apply(Reply::LockedOut { retry_secs: 0 }, 0.0);
    assert_eq!(short.secs_left(0.0), 1, "a lockout is at least one second");
}

#[test]
fn headless_an_account_locked_at_start_stays_locked_for_its_remaining_time() {
    let l = login_for(Some(account("Ada", "", true, 30)));
    assert!(l.locked(0.0));
    assert_eq!(l.secs_left(0.0), 30);
}

#[test]
fn headless_an_account_without_a_password_says_so() {
    let l = login_for(Some(account("Ada", "", false, 0)));
    assert_eq!(l.message(0.0), "No password is set for this account");
}

#[test]
fn headless_an_unreachable_helper_shows_the_unavailable_line_and_no_hint() {
    let l = login_for(None);
    assert_eq!(l.message(0.0), "Login service unavailable");
    assert_eq!(l.hint_line(), None);
}

#[test]
fn headless_a_correct_password_returns_the_session_token_and_clears_the_count() {
    let mut l = login_for(Some(account("Ada", "my cat", true, 0)));
    for _ in 0..HINT_AFTER {
        l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
    }
    assert!(l.hint_line().is_some());
    assert_eq!(l.apply(Reply::Ok("token-1".into()), 0.0), Some("token-1".into()));
    assert_eq!(l.hint_line(), None, "a success resets the wrong count");
}

#[test]
fn headless_a_helper_answer_frame_is_read_into_a_reply() {
    assert_eq!(parse_reply(r#"{"ok":true,"result":{"token":"abc"}}"#), Reply::Ok("abc".into()));
    assert_eq!(parse_reply(r#"{"ok":false,"error":{"code":"wrong_password","retry_secs":30}}"#), Reply::Wrong { retry_secs: 30 });
    assert_eq!(parse_reply(r#"{"ok":false,"error":{"code":"locked_out","retry_secs":900}}"#), Reply::LockedOut { retry_secs: 900 });
    assert_eq!(parse_reply(r#"{"ok":false,"error":{"code":"no_password"}}"#), Reply::NoPassword);
    assert_eq!(parse_reply(r#"{"ok":false,"error":{"code":"internal"}}"#), Reply::Unavailable, "an unknown code never unlocks");
    assert_eq!(parse_reply("garbage"), Reply::Unavailable);
}

#[test]
fn headless_the_lock_message_is_cleaned_and_capped() {
    assert_eq!(parse_lock_message("message=  Back soon \n").as_deref(), Some("Back soon"));
    assert_eq!(parse_lock_message("message=\u{7}\n"), None, "a blank message is no message");
    assert_eq!(parse_lock_message("other=1\n"), None);
    let long = format!("message={}", "a".repeat(300));
    assert_eq!(parse_lock_message(&long).map(|m| m.len()), Some(MESSAGE_MAX));
}

#[test]
fn headless_the_autologin_file_accepts_only_plain_account_names() {
    assert_eq!(autologin_name("ada\n").as_deref(), Some("ada"));
    assert_eq!(autologin_name("ada.lovelace-2_x").as_deref(), Some("ada.lovelace-2_x"));
    assert_eq!(autologin_name("a b"), None);
    assert_eq!(autologin_name("  \n"), None);
    assert_eq!(autologin_name(&"a".repeat(65)), None);
}

#[test]
fn headless_power_choices_become_effects_only_when_chosen() {
    assert!(power_effect(&Act::None).is_none());
    assert!(power_effect(&Act::Sleep).is_some());
    assert!(power_effect(&Act::Restart).is_some());
    assert!(power_effect(&Act::ShutDown).is_some());
}

#[test]
fn headless_lockout_grows_after_five_wrong_passwords() {
    let secs: Vec<u64> = (0..=8).map(lockout_secs).collect();
    assert_eq!(secs, vec![0, 0, 0, 0, 0, 60, 300, 900, 3600]);
    assert_eq!(lockout_secs(40), 3600, "the longest lockout is an hour");
}

#[test]
fn headless_the_password_hint_is_cleaned_and_capped() {
    assert_eq!(parse_hint("password_hint=my cat\n"), "my cat");
    assert_eq!(parse_hint("password_hint=a\u{7}b\n"), "ab", "control characters are dropped");
    assert_eq!(parse_hint("fails=1\n"), "");
    assert_eq!(parse_hint(&format!("password_hint={}\n", "x".repeat(300))).chars().count(), HINT_MAX);
}

#[test]
fn headless_the_persisted_fail_counter_parses_or_starts_at_zero() {
    assert_eq!(parse_fails("fails=3\nuntil=99"), (3, 99));
    assert_eq!(parse_fails("garbage"), (0, 0));
    assert_eq!(parse_fails("fails=-2\nuntil=5"), (0, 5), "a negative count is no count");
}

#[test]
fn headless_each_protocol_error_has_a_stable_code() {
    assert_eq!(AuthError::WrongPassword { fails: 1, retry_secs: 0 }.code(), "wrong_password");
    assert_eq!(AuthError::LockedOut { retry_secs: 60 }.code(), "locked_out");
    assert_eq!(AuthError::NoPassword.code(), "no_password");
    assert_eq!(AuthError::Unauthorized.code(), "unauthorized");
    assert_eq!(AuthError::Locked.code(), "locked");
    assert_eq!(AuthError::BadParams("x".into()).code(), "bad_params");
    assert_eq!(AuthError::Internal("x".into()).code(), "internal");
    assert_eq!(MIN_PASSWORD, 8);
}

// -- crash reports and the diagnostics bundle --------------------------------------------------------------------------

#[test]
fn headless_a_crash_log_line_gives_its_time_and_message() {
    assert_eq!(parse_line("[1700000000 22:13:20] thread 'main': boom"), Some((1_700_000_000, "boom".into())));
    assert_eq!(parse_line("no bracket here"), None);
}

#[test]
fn headless_a_crash_signature_is_the_panic_place_and_text() {
    assert_eq!(signature("panic: index out of bounds at src/x.rs:12:5"), "src/x.rs:12 index out of bounds");
    assert_eq!(signature("plain message"), "plain message");
    let long = signature(&"w".repeat(70));
    assert_eq!(long.chars().count(), 60);
    assert!(long.ends_with('…'));
}

#[test]
fn headless_a_crash_log_keeps_its_valid_lines_oldest_first() {
    let log = "[100 00:01:40] thread 'a': first\njunk\n[200 00:03:20] thread 'b': panic: oops at src/y.rs:3:1\n";
    let entries = parse_log(log, Path::new("/state/crash.log"));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].unix, 100);
    assert_eq!(entries[1].signature, "src/y.rs:3 oops");
    assert_eq!(entries[1].file, PathBuf::from("/state/crash.log"));
}

#[test]
fn headless_a_crash_date_is_shown_in_the_chosen_zone() {
    assert_eq!(date_text(0, 0), "Jan 1, 00:00");
    assert_eq!(date_text(0, 3600), "Jan 1, 01:00");
    assert_eq!(date_text(31 * 86_400 + 14 * 3600 + 5 * 60, 0), "Feb 1, 14:05");
}

#[test]
fn headless_the_bundle_for_the_developer_lists_each_diagnostic_file_in_order() {
    let text = bundle(Path::new("/nonexistent/bifrost-diagnostics-test"), "0.8.0", 42);
    assert!(text.starts_with("springboard diagnostics\nbuild 0.8.0\ntime 42\n"));
    let at = |name: &str| text.find(&format!("\n== {name} ==\n")).unwrap_or_else(|| panic!("missing section {name}"));
    assert!(at("last_exit") < at("heartbeat"));
    assert!(at("heartbeat") < at("crash.log.old"));
    assert!(at("crash.log.old") < at("crash.log"));
}

#[test]
fn headless_the_bundle_upload_goes_next_to_the_update_manifest_over_https_only() {
    assert_eq!(upload_url("https://bifrost.example/updates/manifest.json", "bundle.txt"), "https://bifrost.example/diag/bundle.txt");
    assert_eq!(upload_url("https://bifrost.example", "b"), "https://bifrost.example/diag/b");
    let refused = post("http://bifrost.example/diag/b", b"x").unwrap_err();
    assert!(refused.to_string().contains("https"));
}

#[test]
fn headless_the_performance_readings_are_converted_and_filtered() {
    assert_eq!(max_temp(&[40_000, 85_500, 200_000]), Some(85.5), "a reading out of range is ignored");
    assert_eq!(max_temp(&[]), None);
    assert_eq!(parse_mem("MemTotal: 2048000 kB\nMemAvailable: 1024000 kB\n"), Some((1000, 2000)));
    assert_eq!(current_ma(-1_500_000), -1500);
}

#[test]
fn headless_clearing_crash_logs_counts_only_the_files_removed() {
    assert_eq!(delete_all(Path::new("/nonexistent/bifrost-diagnostics-test")), 0);
}

// -- photo slideshow ---------------------------------------------------------------------------------------------------

fn show(n: usize) -> Slideshow {
    let photos = (0..n).map(|i| Photo { path: PathBuf::from(format!("/pics/{i}.jpg")), mtime: i as i64, size: 1 }).collect();
    Slideshow::new(photos, 0).unwrap()
}

/// Ticks every 250 ms (the largest step the clock accepts) up to `to_ms`, the way a running shell does.
fn run_to(s: &mut Slideshow, to_ms: u32) {
    s.tick(0.0);
    for k in 1..=(to_ms / 250) {
        s.tick(f64::from(k) * 250.0);
    }
}

#[test]
fn headless_an_empty_selection_has_no_slideshow_and_the_start_is_clamped() {
    assert!(Slideshow::new(vec![], 0).is_none());
    let photos = (0..3).map(|i| Photo { path: PathBuf::from(format!("/p/{i}.jpg")), mtime: 0, size: 1 }).collect();
    assert_eq!(Slideshow::new(photos, 9).unwrap().index, 2);
}

#[test]
fn headless_a_picture_stays_one_slide_length_before_the_next_appears() {
    let mut s = show(2);
    run_to(&mut s, SLIDE_MS as u32 - 250);
    assert_eq!(s.index, 0);
    s.tick(SLIDE_MS);
    assert_eq!(s.index, 1);
    assert_eq!(s.next_index(), 0, "the last picture wraps to the first");
}

#[test]
fn headless_the_slide_length_follows_the_chosen_seconds_and_never_drops_below_two() {
    let mut two = show(2).with_seconds(2);
    run_to(&mut two, 2000);
    assert_eq!(two.index, 1, "two seconds later the second picture is up");
    let mut early = show(2).with_seconds(0);
    run_to(&mut early, 1750);
    assert_eq!(early.index, 0);
    let mut zero = show(2).with_seconds(0);
    run_to(&mut zero, 2000);
    assert_eq!(zero.index, 1, "a request for zero seconds still gets the two-second minimum");
}

#[test]
fn headless_the_cross_fade_overlaps_the_next_picture_in_its_last_second() {
    let mut s = show(3);
    run_to(&mut s, 3500);
    let layers = s.layers();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].0, 0);
    assert!((layers[0].1 - 0.9).abs() < 1e-4, "the current drift runs to 0.9 of its path");
    assert_eq!(layers[0].2, 255);
    assert_eq!(layers[1].0, 1);
    assert!((layers[1].1 - 0.1).abs() < 1e-4);
    assert_eq!(layers[1].2, 127, "half of the one-second fade is half opaque");
}

#[test]
fn headless_before_the_fade_only_the_current_picture_is_drawn() {
    let s = show(3);
    let layers = s.layers();
    assert_eq!(layers.len(), 1);
    assert!((layers[0].1 - (FADE_MS / (SLIDE_MS + FADE_MS)) as f32).abs() < 1e-6);
}

#[test]
fn headless_pausing_freezes_the_show_and_its_pictures() {
    let mut s = show(3);
    run_to(&mut s, 3500);
    let before = s.layers();
    s.toggle_pause();
    assert!(s.paused && s.controls);
    assert!(!s.tick(9999.0), "a paused show asks for no frame");
    assert_eq!(s.layers(), before);
}

#[test]
fn headless_a_step_jumps_one_picture_and_restarts_its_clock() {
    let mut s = show(3);
    run_to(&mut s, 2000);
    s.step(1, 2000.0);
    assert_eq!(s.index, 1);
    assert!((s.layers()[0].1 - 0.2).abs() < 1e-6, "the new picture starts its drift again");
    s.step(-1, 2000.0);
    assert_eq!(s.index, 0);
    s.step(-1, 2000.0);
    assert_eq!(s.index, 2, "stepping back from the first picture wraps to the last");
}

#[test]
fn headless_the_controls_hide_after_their_timeout_unless_paused() {
    let mut s = show(2);
    s.show_controls(0.0);
    assert!(s.controls);
    s.tick(CONTROLS_MS - 1.0);
    assert!(s.controls);
    s.tick(CONTROLS_MS);
    assert!(!s.controls, "the controls hide once the timeout has passed");
    s.toggle_pause();
    s.tick(CONTROLS_MS * 4.0);
    assert!(s.controls, "a paused show keeps its controls");
    s.toggle_controls(CONTROLS_MS * 4.0);
    assert!(!s.controls, "a second tap hides the controls again");
}

#[test]
fn headless_ken_burns_starts_centred_and_anchors_one_edge_per_corner() {
    let fit = (800.0, 600.0);
    let view = (1200.0, 1920.0);
    for index in 0..4 {
        let (x, y, w, h) = ken_burns(index, 0.0, fit, view);
        assert!((x - 200.0).abs() < 1e-3 && (y - 660.0).abs() < 1e-3, "index {index} starts centred");
        assert!((w - 800.0).abs() < 1e-3 && (h - 600.0).abs() < 1e-3);
    }
    let (x, y, w, h) = ken_burns(0, 1.0, fit, view);
    assert!((w - 896.0).abs() < 0.01 && (h - 672.0).abs() < 0.01, "the picture grows to 1.12 times its size");
    assert!((x - 200.0).abs() < 0.01 && (y - 660.0).abs() < 0.01, "index 0 keeps its top left corner");
    let (x1, _, w1, _) = ken_burns(1, 1.0, fit, view);
    assert!((x1 + w1 - 1000.0).abs() < 0.01, "index 1 keeps its right edge where the centred picture ends");
    let (_, y2, _, h2) = ken_burns(2, 1.0, fit, view);
    assert!((y2 + h2 - 1260.0).abs() < 0.01, "index 2 keeps its bottom edge");
}

#[test]
fn headless_ken_burns_clamps_progress_to_the_path() {
    let a = ken_burns(3, 2.0, (800.0, 600.0), (1200.0, 1920.0));
    let b = ken_burns(3, 1.0, (800.0, 600.0), (1200.0, 1920.0));
    assert_eq!(a, b);
}
