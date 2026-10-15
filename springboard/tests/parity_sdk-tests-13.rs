//! Parity sdk-tests-13: headless tests of App Intents in the shell. The manifest `[[intent]]` tables are read by
//! `shell::apphost::Manifest::parse`; `shell::intents` decides which intents Search can run and builds the JSON-RPC
//! lines that run them. No socket is opened and no app is launched.

use springboard::shell::apphost::Manifest;
use springboard::shell::intents::{parse_reply, request_line, request_line_with, socket_path, ManifestIntent};

fn intent(id: &str, shortcut: bool, params: &[&str]) -> ManifestIntent {
    ManifestIntent {
        id: id.into(),
        title: "Title".into(),
        description: String::new(),
        shortcut,
        params: params.iter().map(|p| p.to_string()).collect(),
    }
}

#[test]
fn headless_each_intent_table_of_a_manifest_becomes_one_intent() {
    let text = "id = \"is.example.notes\"\nname = \"Notes\"\nexec = \"notes\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nparams = [\"text:string\", \"due:date?\"]\n\n[other]\nname = \"ignored\"\n";
    let m = Manifest::parse(text).expect("the manifest parses");
    assert_eq!(m.id, "is.example.notes");
    assert_eq!(m.name, "Notes", "keys after an intent table do not leak into the intent");
    assert_eq!(m.intents.len(), 2);
    assert_eq!(
        m.intents[0],
        ManifestIntent { id: "new-note".into(), title: "New Note".into(), description: String::new(), shortcut: true, params: Vec::new() }
    );
    assert_eq!(m.intents[1].params, vec!["text:string".to_string(), "due:date?".to_string()]);
    assert!(!m.intents[1].shortcut, "shortcut defaults to false");
}

#[test]
fn headless_a_manifest_without_intent_tables_declares_no_intents() {
    let m = Manifest::parse("id = \"is.example.clock\"\nname = \"Clock\"\nexec = \"clock\"\n").expect("the manifest parses");
    assert!(m.intents.is_empty());
}

#[test]
fn headless_only_parameterless_or_optional_shortcuts_are_runnable_from_search() {
    assert!(intent("new-note", true, &[]).runnable());
    assert!(intent("find", true, &["due:date?"]).runnable(), "optional parameters do not block a run");
    assert!(!intent("find", true, &["text:string"]).runnable(), "a required parameter needs input");
    assert!(!intent("new-note", false, &[]).runnable(), "only shortcuts are offered in Search");
}

#[test]
fn headless_an_intent_needs_a_title_and_an_id_with_safe_characters() {
    assert!(intent("new_note-2", true, &[]).runnable());
    assert!(!intent("a b", true, &[]).runnable(), "a space is not a safe id");
    assert!(!intent("a\"b", true, &[]).runnable(), "a quote would break the request line");
    assert!(!intent("", true, &[]).runnable());
    let untitled = ManifestIntent { title: String::new(), ..intent("new-note", true, &[]) };
    assert!(!untitled.runnable());
}

#[test]
fn headless_a_shortcut_read_from_a_manifest_is_runnable_when_it_takes_no_required_input() {
    let m = Manifest::parse("id = \"is.example.notes\"\nname = \"Notes\"\nexec = \"notes\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\n").expect("the manifest parses");
    assert!(m.intents[0].runnable());
}

#[test]
fn headless_a_run_request_names_the_intent_method_with_no_parameters() {
    assert_eq!(request_line("new-note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
}

#[test]
fn headless_string_parameters_are_carried_and_escaped_in_the_request() {
    assert_eq!(
        request_line_with("look-up", &[("word", "cat")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\"}}\n"
    );
    assert_eq!(
        request_line_with("create-note", &[("text", "a \"quote\"\n")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\n\"}}\n"
    );
    assert_eq!(
        request_line_with("x", &[("k", "\u{1}")]),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.x\",\"params\":{\"k\":\"\\u0001\"}}\n"
    );
}

#[test]
fn headless_a_reply_yields_its_dialog_or_its_error_message() {
    assert_eq!(
        parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#),
        Ok(Some("Created".to_string()))
    );
    assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#), Ok(None));
    assert_eq!(
        parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#),
        Err("no intent x".to_string())
    );
    assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000}}"#), Err("error".to_string()));
    assert!(parse_reply("nonsense").is_err());
}

#[test]
fn headless_the_control_socket_of_an_app_is_named_by_its_id() {
    assert!(socket_path("is.example.notes").ends_with("bifrost/apps/is.example.notes.sock"));
}
