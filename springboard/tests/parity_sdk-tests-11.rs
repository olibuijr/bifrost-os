//! Parity sdk-tests-11: headless tests of the App Intents shortcut model. The manifest is read as text by the public
//! `Manifest::parse`, the runnable filter that Search applies is `ManifestIntent::runnable`, and the control socket
//! request and reply are built and read as plain strings. No socket is opened and no app is launched.

use springboard::shell::apphost::Manifest;
use springboard::shell::intents::{parse_reply, request_line, request_line_with, ManifestIntent};

const NOTES_MANIFEST: &str = r#"
id = "com.example.notes"
name = "Notes"
exec = "notes"

[[intent]]
id = "new-note"
title = "New Note"
description = "Create an empty note"
shortcut = true

[[intent]]
id = "due-reminder"
title = "Remind Me"
shortcut = true
params = ["due:date?"]

[[intent]]
id = "find-note"
title = "Find Note"
shortcut = true
params = ["text:string"]

[[intent]]
id = "archive-all"
title = "Archive All"
shortcut = false
"#;

fn intent(id: &str, shortcut: bool, params: &[&str]) -> ManifestIntent {
    ManifestIntent {
        id: id.into(),
        title: "A Title".into(),
        description: String::new(),
        shortcut,
        params: params.iter().map(|p| p.to_string()).collect(),
    }
}

fn runnable_ids(manifest: &Manifest) -> Vec<&str> {
    manifest.intents.iter().filter(|i| i.runnable()).map(|i| i.id.as_str()).collect()
}

#[test]
fn every_intent_table_of_a_manifest_is_read_in_order() {
    let manifest = Manifest::parse(NOTES_MANIFEST).expect("the manifest is valid");
    let ids: Vec<&str> = manifest.intents.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["new-note", "due-reminder", "find-note", "archive-all"]);
    assert_eq!(manifest.intents[0].title, "New Note");
    assert_eq!(manifest.intents[0].description, "Create an empty note");
    assert_eq!(manifest.intents[1].params, ["due:date?"]);
}

#[test]
fn search_offers_only_shortcuts_that_run_without_required_input() {
    let manifest = Manifest::parse(NOTES_MANIFEST).expect("the manifest is valid");
    assert_eq!(runnable_ids(&manifest), ["new-note", "due-reminder"]);
}

#[test]
fn a_manifest_without_intent_tables_offers_no_shortcuts() {
    let manifest = Manifest::parse("id = \"com.example.plain\"\nname = \"Plain\"\nexec = \"plain\"\n").expect("valid");
    assert!(manifest.intents.is_empty());
    assert!(runnable_ids(&manifest).is_empty());
}

#[test]
fn an_intent_that_is_not_flagged_as_a_shortcut_is_not_offered() {
    assert!(!intent("archive-all", false, &[]).runnable());
    assert!(intent("archive-all", true, &[]).runnable());
}

#[test]
fn a_required_parameter_keeps_an_intent_out_of_search() {
    assert!(!intent("find-note", true, &["text:string"]).runnable());
    assert!(intent("due-reminder", true, &["due:date?"]).runnable(), "an optional parameter may be left out");
}

#[test]
fn an_intent_id_outside_the_safe_alphabet_is_not_offered() {
    assert!(intent("new-note_2", true, &[]).runnable());
    assert!(!intent("new note", true, &[]).runnable());
    assert!(!intent("a\"b", true, &[]).runnable());
    assert!(!intent("", true, &[]).runnable());
}

#[test]
fn an_intent_without_a_title_is_not_offered() {
    let mut untitled = intent("new-note", true, &[]);
    untitled.title.clear();
    assert!(!untitled.runnable());
}

#[test]
fn a_request_runs_the_intent_method_with_empty_params() {
    assert_eq!(
        request_line("new-note"),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n"
    );
}

#[test]
fn a_request_with_parameters_escapes_quotes_backslashes_and_newlines() {
    assert_eq!(
        request_line_with("create-note", &[("text", "a \"quote\"\nline\\")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\nline\\\\\"}}\n"
    );
}

#[test]
fn a_request_with_two_parameters_lists_them_in_order() {
    assert_eq!(
        request_line_with("look-up", &[("word", "cat"), ("lang", "en")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\",\"lang\":\"en\"}}\n"
    );
}

#[test]
fn a_reply_with_a_dialog_gives_the_dialog_text() {
    let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#;
    assert_eq!(parse_reply(reply), Ok(Some("Created".to_string())));
}

#[test]
fn a_reply_without_a_dialog_gives_no_text() {
    let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#;
    assert_eq!(parse_reply(reply), Ok(None));
}

#[test]
fn an_error_reply_gives_the_error_message() {
    let reply = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#;
    assert_eq!(parse_reply(reply), Err("no intent x".to_string()));
}

#[test]
fn a_reply_that_is_not_json_rpc_is_refused() {
    assert!(parse_reply("nonsense").is_err());
}
