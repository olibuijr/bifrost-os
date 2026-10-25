//! Parity sdk-tests-10: headless tests of the shell's App Intents listing and request lines (`springboard::shell::intents`,
//! `springboard::shell::media_intents`, the manifest reader in `springboard::shell::apphost`) and of the Screen Time
//! settings (`springboard::shell::screentime::config`).
//!
//! Headless: the manifest text is parsed in memory, request lines are built as strings, and Screen Time runs against
//! in-memory preferences (`Config::memory`). No socket, app process, display or device is involved.

use bifrost_ui::media_session::RemoteCommand;
use springboard::shell::apphost::Manifest;
use springboard::shell::intents::{parse_reply, request_line, request_line_with, socket_path, ManifestIntent};
use springboard::shell::media_intents::{command_for_intent, command_name, manifest_intents, TRANSPORT};
use springboard::shell::screentime::config::{Config, Downtime, Ignore, Reason, IGNORE_SECS, LIMIT_STEPS};
use springboard::shell::AppId;

const UNIX_NOON_DAY_20000: i64 = 20_000 * 86_400 + 12 * 3600;
const UNIX_23H_DAY_20000: i64 = 20_000 * 86_400 + 23 * 3600;

fn listed(id: &str, shortcut: bool, params: &[&str]) -> ManifestIntent {
    ManifestIntent { id: id.into(), title: "New Note".into(), description: String::new(), shortcut, params: params.iter().map(|p| p.to_string()).collect() }
}

// --- intents listed in Search ------------------------------------------------------------------------------------------

#[test]
fn headless_an_intent_flagged_as_a_shortcut_with_no_parameter_is_runnable_from_search() {
    assert!(listed("new-note", true, &[]).runnable());
}

#[test]
fn headless_an_intent_with_only_optional_parameters_is_still_runnable_from_search() {
    assert!(listed("new-note", true, &["due:date?"]).runnable());
}

#[test]
fn headless_an_intent_with_a_required_parameter_is_listed_but_not_run_from_search() {
    assert!(!listed("new-note", true, &["text:string"]).runnable());
    assert!(!listed("new-note", true, &["text:string", "due:date?"]).runnable());
}

#[test]
fn headless_an_intent_that_is_not_a_shortcut_is_not_offered_in_search() {
    assert!(!listed("new-note", false, &[]).runnable());
}

#[test]
fn headless_an_intent_with_an_empty_title_or_an_unsafe_id_is_not_offered() {
    let mut untitled = listed("new-note", true, &[]);
    untitled.title.clear();
    assert!(!untitled.runnable());
    assert!(!listed("", true, &[]).runnable());
    assert!(!listed("a b", true, &[]).runnable(), "an id with a space is refused");
    assert!(!listed("a\"b", true, &[]).runnable(), "an id with a quote is refused");
    assert!(listed("note-2_draft", true, &[]).runnable(), "dashes and underscores are safe");
}

#[test]
fn headless_the_manifest_reader_turns_an_intent_table_into_a_search_entry() {
    let text = "id = \"a.notes\"\nname = \"Notes\"\nexec = \"notes\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\ndescription = \"Creates a note\"\nshortcut = true\nparams = [\"due:date?\"]\n";
    let manifest = Manifest::parse(text).expect("the manifest parses");
    assert_eq!(manifest.intents.len(), 1);
    let intent = &manifest.intents[0];
    assert_eq!(intent.id, "new-note");
    assert_eq!(intent.title, "New Note");
    assert!(intent.shortcut);
    assert_eq!(intent.params, vec!["due:date?"]);
    assert!(intent.runnable());
}

#[test]
fn headless_an_intent_table_that_needs_input_is_read_but_not_runnable() {
    let text = "id = \"a.notes\"\nname = \"Notes\"\nexec = \"notes\"\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nshortcut = true\nparams = [\"text:string\"]\n";
    let manifest = Manifest::parse(text).expect("the manifest parses");
    assert_eq!(manifest.intents.len(), 1);
    assert!(!manifest.intents[0].runnable());
}

#[test]
fn headless_a_request_line_runs_an_intent_with_no_parameters() {
    assert_eq!(request_line("new-note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
}

#[test]
fn headless_a_request_line_carries_string_parameters_with_their_quotes_and_newlines_escaped() {
    assert_eq!(
        request_line_with("create-note", &[("text", "a \"quote\"\nline\\")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\nline\\\\\"}}\n"
    );
    assert_eq!(request_line_with("look-up", &[("word", "cat")]), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\"}}\n");
}

#[test]
fn headless_a_control_character_in_a_parameter_is_escaped_as_a_unicode_escape() {
    assert_eq!(request_line_with("ping", &[("s", "\u{1}\t")]), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.ping\",\"params\":{\"s\":\"\\u0001\\t\"}}\n");
}

#[test]
fn headless_a_reply_yields_the_dialog_text_or_the_error_message() {
    assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#), Ok(Some("Created".into())));
    assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#), Ok(None));
    assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#), Err("no intent x".into()));
    assert!(parse_reply("nonsense").is_err(), "a reply that is not JSON is an error");
}

#[test]
fn headless_an_app_control_socket_lives_in_the_data_folder_named_for_the_app() {
    assert!(socket_path("notes").ends_with("appdata/notes/control.sock"));
}

// --- transport intents from the Now Playing session --------------------------------------------------------------------

#[test]
fn headless_the_transport_intents_are_three_runnable_shortcuts_with_no_parameters() {
    assert_eq!(TRANSPORT.len(), 3);
    let listed = manifest_intents();
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["media-play-pause", "media-next", "media-previous"]);
    assert!(listed.iter().all(|i| i.shortcut && i.params.is_empty() && i.runnable()));
}

#[test]
fn headless_each_transport_intent_sends_its_matching_session_command() {
    assert!(matches!(command_for_intent("media-play-pause"), Some(RemoteCommand::TogglePlayPause)));
    assert!(matches!(command_for_intent("media-next"), Some(RemoteCommand::Next)));
    assert!(matches!(command_for_intent("media-previous"), Some(RemoteCommand::Previous)));
    assert!(command_for_intent("media-stop").is_none(), "an id that is not a transport intent sends nothing");
}

#[test]
fn headless_a_remote_command_has_the_name_the_session_service_uses() {
    assert_eq!(command_name(RemoteCommand::TogglePlayPause), "toggle");
    assert_eq!(command_name(RemoteCommand::Next), "next");
    assert_eq!(command_name(RemoteCommand::Previous), "previous");
    assert_eq!(command_name(RemoteCommand::Play), "play");
    assert_eq!(command_name(RemoteCommand::SeekTo), "seek");
}

// --- Screen Time: limits -----------------------------------------------------------------------------------------------

#[test]
fn headless_a_daily_limit_is_stored_per_app_and_zero_removes_it() {
    let mut c = Config::memory();
    assert_eq!(c.limit_mins(AppId::Browser), None);
    c.set_limit(AppId::Browser, Some(30));
    assert_eq!(c.limit_mins(AppId::Browser), Some(30));
    assert_eq!(c.limit_mins(AppId::Terminal), None, "another app has no limit");
    c.set_limit(AppId::Browser, Some(0));
    assert_eq!(c.limit_mins(AppId::Browser), None, "a zero limit is no limit");
    c.set_limit(AppId::Browser, None);
    assert_eq!(c.limit_mins(AppId::Browser), None);
}

#[test]
fn headless_cycling_a_limit_walks_the_steps_and_wraps_back_to_none() {
    assert_eq!(LIMIT_STEPS, [0, 15, 30, 60, 120, 180, 240]);
    let mut c = Config::memory();
    let steps: Vec<Option<u32>> = (0..7).map(|_| c.cycle_limit(AppId::Browser)).collect();
    assert_eq!(steps, vec![Some(15), Some(30), Some(60), Some(120), Some(180), Some(240), None]);
    assert_eq!(c.cycle_limit(AppId::Browser), Some(15), "the cycle starts again after none");
}

#[test]
fn headless_an_app_is_covered_once_its_use_reaches_the_limit() {
    let mut c = Config::memory();
    c.set_limit(AppId::Browser, Some(30));
    assert_eq!(c.block(AppId::Browser, 1799, UNIX_NOON_DAY_20000, 0), None, "29 minutes 59 seconds is still allowed");
    assert_eq!(c.block(AppId::Browser, 1800, UNIX_NOON_DAY_20000, 0), Some(Reason::Limit));
}

#[test]
fn headless_settings_is_never_blocked_so_its_limits_can_always_be_changed() {
    let mut c = Config::memory();
    c.set_limit(AppId::Settings, Some(1));
    assert_eq!(c.block(AppId::Settings, 100_000, UNIX_NOON_DAY_20000, 0), None);
}

#[test]
fn headless_an_unset_clock_never_blocks_an_app() {
    let mut c = Config::memory();
    c.set_limit(AppId::Browser, Some(1));
    assert_eq!(c.block(AppId::Browser, 100_000, 0, 0), None);
}

// --- Screen Time: Downtime ---------------------------------------------------------------------------------------------

#[test]
fn headless_downtime_is_off_by_default_and_keeps_its_window_within_one_day() {
    let mut c = Config::memory();
    assert_eq!(c.downtime(), Downtime { on: false, start: 22 * 60, end: 7 * 60 });
    c.set_downtime(Downtime { on: true, start: 1500, end: 420 });
    let stored = c.downtime();
    assert!(stored.on);
    assert_eq!(stored.start, 60, "a start past one day wraps to the same minute of the day");
    assert_eq!(stored.end, 420);
}

#[test]
fn headless_downtime_blocks_an_app_it_does_not_allow_inside_its_window() {
    let mut c = Config::memory();
    c.set_downtime(Downtime { on: true, start: 22 * 60, end: 7 * 60 });
    assert_eq!(c.block(AppId::Browser, 0, UNIX_23H_DAY_20000, 0), Some(Reason::Downtime));
    c.set_allowed(AppId::Browser, true);
    assert_eq!(c.block(AppId::Browser, 0, UNIX_23H_DAY_20000, 0), None, "an allowed app keeps working during Downtime");
}

#[test]
fn headless_downtime_does_not_block_outside_its_window() {
    let mut c = Config::memory();
    c.set_downtime(Downtime { on: true, start: 22 * 60, end: 7 * 60 });
    assert_eq!(c.block(AppId::Browser, 0, UNIX_NOON_DAY_20000, 0), None);
}

#[test]
fn headless_settings_is_always_allowed_and_another_app_follows_its_allow_flag() {
    let mut c = Config::memory();
    assert!(c.allowed(AppId::Settings));
    assert!(!c.allowed(AppId::Browser), "an app is not allowed in Downtime until the user allows it");
    c.set_allowed(AppId::Browser, true);
    assert!(c.allowed(AppId::Browser));
    c.set_allowed(AppId::Browser, false);
    assert!(!c.allowed(AppId::Browser));
}

// --- Screen Time: ignore grants ----------------------------------------------------------------------------------------

#[test]
fn headless_a_fifteen_minute_ignore_grant_lifts_the_limit_until_it_runs_out() {
    assert_eq!(IGNORE_SECS, 900);
    let mut c = Config::memory();
    c.set_limit(AppId::Browser, Some(30));
    c.grant(AppId::Browser, Ignore::Fifteen, UNIX_NOON_DAY_20000, 0);
    assert_eq!(c.block(AppId::Browser, 4000, UNIX_NOON_DAY_20000 + 10, 0), None);
    assert_eq!(c.block(AppId::Browser, 4000, UNIX_NOON_DAY_20000 + IGNORE_SECS, 0), Some(Reason::Limit));
}

#[test]
fn headless_a_today_ignore_grant_lasts_until_the_local_day_changes() {
    let mut c = Config::memory();
    c.set_limit(AppId::Browser, Some(30));
    c.grant(AppId::Browser, Ignore::Today, UNIX_NOON_DAY_20000, 0);
    assert!(c.ignored(AppId::Browser, UNIX_NOON_DAY_20000 + 5, 0));
    assert!(!c.ignored(AppId::Browser, UNIX_NOON_DAY_20000 + 86_400, 0), "the grant ends with the day");
    assert_eq!(c.block(AppId::Browser, 4000, UNIX_NOON_DAY_20000 + 86_400, 0), Some(Reason::Limit));
}

#[test]
fn headless_the_block_reasons_are_distinct_values() {
    assert_ne!(Reason::Limit, Reason::Downtime);
    assert_ne!(Ignore::Fifteen, Ignore::Today);
}
