//! App Intents in the shell (`AppShortcut` in Search). Apps declare intents in `manifest.toml` as `[[intent]]`
//! tables (SDK: `bifrost_app::intents`); the shell reads them without launching the app, lists the runnable ones
//! in Search's "Shortcuts" section, and runs one by launching the app and calling `intent.<id>` on its control
//! socket (`$XDG_RUNTIME_DIR/bifrost/apps/<app id>.sock`, JSON-RPC 2.0, one request per connection).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One `[[intent]]` table of a manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestIntent {
    pub id: String,
    pub title: String,
    pub description: String,
    /// `AppShortcut`: offered in Search
    pub shortcut: bool,
    /// `name:type` with a trailing `?` when optional, for example `due:date?`
    pub params: Vec<String>,
}

impl ManifestIntent {
    /// Search can run it with no input: flagged as a shortcut, a safe id, and no required parameter.
    pub fn runnable(&self) -> bool {
        self.shortcut && !self.title.is_empty() && !self.id.is_empty() && self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && self.params.iter().all(|p| p.ends_with('?'))
    }
}

/// One runnable shortcut intent of an installed app: what Search and the Shortcuts action library list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppShortcut {
    pub app_id: String,
    pub app_name: String,
    pub intent: ManifestIntent,
}

/// The runnable shortcut intents of the given installed apps, in install order and then manifest order. Apps that
/// declare none contribute nothing. Reads only what the manifests say; no app is launched.
pub fn runnable_shortcuts(apps: &[super::apphost::Installed]) -> Vec<AppShortcut> {
    apps.iter()
        .flat_map(|app| {
            app.manifest.intents.iter().filter(|i| i.runnable()).map(move |i| AppShortcut {
                app_id: app.manifest.id.clone(),
                app_name: app.manifest.name.clone(),
                intent: i.clone(),
            })
        })
        .collect()
}

/// The JSON-RPC request line that runs intent `id` with no parameters.
pub fn request_line(id: &str) -> String {
    request_line_with(id, &[])
}

/// The JSON-RPC request line that runs intent `id` with string parameters, `(name, value)` pairs.
pub fn request_line_with(id: &str, params: &[(&str, &str)]) -> String {
    let body: Vec<String> = params.iter().map(|(name, value)| format!("{}:{}", json_string(name), json_string(value))).collect();
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.{id}\",\"params\":{{{}}}}}\n", body.join(","))
}

/// A JSON string literal for `s`.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The control socket of a running app.
pub fn socket_path(app_id: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
        PathBuf::from(format!("/run/user/{uid}"))
    });
    base.join("bifrost/apps").join(format!("{app_id}.sock"))
}

/// What an RPC reply says: `Ok(dialog)` for a result, `Err(message)` for an error object.
pub fn parse_reply(reply: &str) -> Result<Option<String>, String> {
    let v = super::store::json::parse(reply.trim()).map_err(|e| format!("malformed reply: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(e.str_field("message").unwrap_or_else(|| "error".into()));
    }
    Ok(v.get("result").and_then(|r| r.str_field("dialog")))
}

/// Calls the intent, retrying while the app starts (up to `wait`). Blocking: run it off the UI thread.
pub fn run(app_id: &str, intent_id: &str, wait: Duration) -> Result<Option<String>, String> {
    run_at(&socket_path(app_id), &request_line(intent_id), wait)
}

/// Runs intent `intent_id` of a running app with string parameters (see [`request_line_with`]).
pub fn run_with(app_id: &str, intent_id: &str, params: &[(&str, &str)], wait: Duration) -> Result<Option<String>, String> {
    run_at(&socket_path(app_id), &request_line_with(intent_id, params), wait)
}

/// Sends one JSON-RPC request line to a running app's control socket and returns the raw reply. Retries the connect
/// while the app starts (up to `wait`). Blocking: run it off the UI thread. Services use it for `service.<method>`.
pub fn exchange(app_id: &str, line: &str, wait: Duration) -> Result<String, String> {
    exchange_at(&socket_path(app_id), line, wait)
}

fn run_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<Option<String>, String> {
    parse_reply(&exchange_at(path, line, wait)?)
}

fn exchange_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<String, String> {
    let deadline = Instant::now() + wait;
    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(s) => break s,
            Err(e) if Instant::now() >= deadline => return Err(format!("{}: {e}", path.display())),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(shortcut: bool, params: &[&str]) -> ManifestIntent {
        ManifestIntent { id: "new-note".into(), title: "New Note".into(), description: String::new(), shortcut, params: params.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn only_parameterless_shortcuts_are_runnable() {
        assert!(intent(true, &[]).runnable());
        assert!(intent(true, &["due:date?"]).runnable());
        assert!(!intent(true, &["text:string"]).runnable());
        assert!(!intent(false, &[]).runnable());
        let mut bad = intent(true, &[]);
        bad.id = "a\"b".into();
        assert!(!bad.runnable());
    }

    #[test]
    fn runnable_shortcuts_lists_only_parameterless_shortcuts_per_app() {
        use super::super::apphost::{Installed, Manifest};
        let apps = [
            Installed {
                manifest: Manifest::parse("id = \"a.notes\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nshortcut = true\nparams = [\"text:string\"]\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
            Installed {
                manifest: Manifest::parse("id = \"a.quiet\"\nname = \"Quiet\"\nexec = \"x\"\n\n[[intent]]\nid = \"hush\"\ntitle = \"Hush\"\nshortcut = false\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
        ];
        let found = runnable_shortcuts(&apps);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].app_id.as_str(), found[0].app_name.as_str(), found[0].intent.id.as_str()), ("a.notes", "Notes", "new-note"));
        assert!(runnable_shortcuts(&[]).is_empty());
    }

    #[test]
    fn request_and_reply_round_trip() {
        assert_eq!(request_line("new-note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null,"dialog":"Created"}}"#), Ok(Some("Created".into())));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"dialog":null}}"#), Ok(None));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no intent x"}}"#), Err("no intent x".into()));
        assert!(parse_reply("nonsense").is_err());
    }

    #[test]
    fn request_lines_carry_string_parameters_escaped() {
        assert_eq!(
            request_line_with("create-note", &[("text", "a \"quote\"\nline\\")]),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.create-note\",\"params\":{\"text\":\"a \\\"quote\\\"\\nline\\\\\"}}\n"
        );
        assert_eq!(request_line_with("look-up", &[("word", "cat")]), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.look-up\",\"params\":{\"word\":\"cat\"}}\n");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn run_talks_to_a_socket() {
        let dir = std::env::temp_dir().join(format!("bifrost-intents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("t.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            assert!(req.contains("\"intent.go\""));
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":null,\"dialog\":\"Went\"}}\n").unwrap();
        });
        assert_eq!(run_at(&sock, &request_line("go"), Duration::from_secs(1)), Ok(Some("Went".into())));
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
