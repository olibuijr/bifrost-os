//! Parity sdk-tests-11: headless tests of the App Intents shortcut model. The manifest is read as text by the public
//! `Manifest::parse`, the runnable filter that Search applies is `ManifestIntent::runnable`, and the control socket
//! request and reply are built and read as plain strings. No socket is opened and no app is launched.

use bifrost_proto::{DocErrorCode, DocMode};
use springboard::bare::docbroker::{valid_name, Grants, Opener};
use springboard::shell::apphost::Manifest;
use springboard::shell::intents::{parse_reply, request_line, request_line_with, ManifestIntent};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};

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

// --- the document broker: the account's files beneath its home, saves and per-app grants ---

/// A fresh account home with one document and one file outside the home, under the test target directory.
fn account(tag: &str) -> (PathBuf, Opener) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sdk-tests-11-{tag}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("home/Documents")).unwrap();
    fs::write(root.join("home/Documents/a.txt"), "hello").unwrap();
    fs::write(root.join("secret"), "no").unwrap();
    let meta = fs::metadata(&root).unwrap();
    let opener = Opener::new(&root.join("home"), meta.uid(), meta.gid()).unwrap();
    (root, opener)
}

#[test]
fn a_file_name_is_one_component_that_is_not_dot_or_empty() {
    assert!(valid_name("notes.txt"));
    assert!(valid_name(".hidden"));
    assert!(!valid_name(""));
    assert!(!valid_name("."));
    assert!(!valid_name(".."));
    assert!(!valid_name("a/b"));
    assert!(!valid_name("a\0b"));
    assert!(valid_name(&"x".repeat(255)));
    assert!(!valid_name(&"x".repeat(256)));
}

#[test]
fn the_account_reads_its_own_files_beneath_its_home() {
    let (root, o) = account("read");
    let mut text = String::new();
    o.open(Path::new("Documents/a.txt"), DocMode::Read).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "hello");
    assert_eq!(o.home(), root.join("home").as_path());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_path_outside_the_home_is_denied_and_a_missing_file_is_not_found() {
    let (root, o) = account("escape");
    assert_eq!(o.open(Path::new("../secret"), DocMode::Read).unwrap_err(), DocErrorCode::Denied);
    assert_eq!(o.open(&root.join("secret"), DocMode::Read).unwrap_err(), DocErrorCode::Denied, "an absolute path");
    assert_eq!(o.open(Path::new("Documents/none.txt"), DocMode::Read).unwrap_err(), DocErrorCode::NotFound);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_symlink_inside_the_home_is_refused() {
    let (root, o) = account("symlink");
    symlink(root.join("secret"), root.join("home/link")).unwrap();
    assert_eq!(o.open(Path::new("link"), DocMode::Read).unwrap_err(), DocErrorCode::Denied);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn listing_the_home_shows_folders_and_files_but_no_symlinks() {
    let (root, o) = account("list");
    symlink(root.join("secret"), root.join("home/link")).unwrap();
    let mut home = o.list(Path::new("")).unwrap();
    home.sort();
    assert_eq!(home, [("Documents".to_string(), true)]);
    assert_eq!(o.list(Path::new("Documents")).unwrap(), [("a.txt".to_string(), false)]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_save_writes_a_new_file_into_its_folder_and_leaves_no_temporary_file() {
    let (root, o) = account("save");
    let (pending, mut file) = o.begin_save(Path::new("Documents"), "new.txt").unwrap();
    file.write_all(b"saved text").unwrap();
    drop(file);
    pending.commit(&o).unwrap();
    assert_eq!(fs::read_to_string(root.join("home/Documents/new.txt")).unwrap(), "saved text");
    let names: Vec<String> =
        fs::read_dir(root.join("home/Documents")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 2, "a.txt and new.txt only");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_aborted_save_leaves_the_folder_as_it_was() {
    let (root, o) = account("abort");
    let (pending, _file) = o.begin_save(Path::new("Documents"), "gone.txt").unwrap();
    pending.abort(&o);
    assert!(!root.join("home/Documents/gone.txt").exists());
    assert_eq!(fs::read_dir(root.join("home/Documents")).unwrap().count(), 1, "only a.txt is left");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_save_name_with_a_slash_is_refused_before_anything_is_created() {
    let (root, o) = account("badname");
    assert!(matches!(o.begin_save(Path::new("Documents"), "x/y"), Err(DocErrorCode::Denied)));
    assert_eq!(fs::read_dir(root.join("home/Documents")).unwrap().count(), 1);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_grant_opens_its_file_only_for_the_app_it_was_given_to() {
    let (root, o) = account("grant");
    let grants_file = root.join("grants.json");
    assert!(Grants::load(&grants_file).of_app("notes-app").next().is_none(), "a missing file starts empty");
    let mut grants = Grants::load(&grants_file);
    let file = o.open(Path::new("Documents/a.txt"), DocMode::Read).unwrap();
    let id = grants.add("notes-app", Path::new("Documents/a.txt"), DocMode::Read, &file).unwrap();
    let grant = grants.get("notes-app", &id).unwrap();
    assert_eq!(grant.app_id, "notes-app");
    assert_eq!(grant.path.as_path(), Path::new("Documents/a.txt"));
    assert_eq!(grant.mode, DocMode::Read);
    assert_eq!(grants.get("other-app", &id).unwrap_err(), DocErrorCode::Denied);
    grants.release("notes-app", &id).unwrap();
    assert_eq!(grants.get("notes-app", &id).unwrap_err(), DocErrorCode::Denied, "a released grant is gone");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_grant_survives_a_restart_and_a_replaced_file_is_reported_stale() {
    let (root, o) = account("restart");
    let grants_file = root.join("grants.json");
    let id = {
        let mut grants = Grants::load(&grants_file);
        let file = o.open(Path::new("Documents/a.txt"), DocMode::Read).unwrap();
        grants.add("notes-app", Path::new("Documents/a.txt"), DocMode::Read, &file).unwrap()
    };
    let reloaded = Grants::load(&grants_file);
    assert_eq!(reloaded.of_app("notes-app").count(), 1);
    fs::remove_file(root.join("home/Documents/a.txt")).unwrap();
    fs::write(root.join("home/Documents/a.txt"), "replaced").unwrap();
    assert!(matches!(reloaded.reopen(&o, "notes-app", &id), Err(DocErrorCode::Stale)));
    fs::remove_dir_all(&root).unwrap();
}
