//! `open`: opens folders, URLs and apps with the desktop's own handlers. The shell does the work over its control socket,
//! the same channel `bifrost-open-url` uses: `open-app <id>` starts an installed app, `open-folder <id> <path>` shows a
//! folder in Files, and `open-url <url>` routes a link by its scheme. The shell answers `ok` or `error: <why>`.
//!
//! Supported: `-a`/`-b` (an installed app id, with no operands), `-e`/`-t` (the text editor, with no operands), `-R` (show
//! a file's folder in Files), `-u` (URL operands), URL operands, `file://` URLs, and folder operands (shown in Files).
//! Refused with a message rather than run wrongly: an ordinary file opened with its default app, any file passed to an
//! app, and the options `-W -n -g -j -F -f -h -s --arch --env --stdin --stdout --stderr --args`. The shell has no request
//! for a file yet, so those wait on one.

use crate::control;
use std::path::{Path, PathBuf};

/// The shell's control socket (see `control::socket`).
pub use crate::control::socket;

/// The Files app, which shows folders (`-R` and folder operands).
pub const FILES_APP: &str = "is.olibuijr.files.desktop";
/// The text editor app (`-e` and `-t`).
pub const TEXT_EDITOR_APP: &str = "is.olibuijr.textedit.desktop";

pub const USAGE: &str = "usage: open [-a app] [-b app] [-e] [-t] [-R] [-u] [file | folder | URL ...]";

/// The parsed command line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    /// The app named by `-a` or `-b`: an installed app id.
    pub app: Option<String>,
    /// `-e` or `-t`: the text editor.
    pub editor: bool,
    /// `-R`: show in Files instead of opening.
    pub reveal: bool,
    /// `-u`: every operand is a URL.
    pub url: bool,
    /// The operands, in order: folders, files, URLs.
    pub operands: Vec<String>,
}

/// What the command line asks for: the help text, or a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments. An option that is not supported is an error that names it.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    let mut only_operands = false;
    while let Some(arg) = it.next() {
        if only_operands || !arg.starts_with('-') || arg.as_str() == "-" {
            o.operands.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => only_operands = true,
            "-h" | "-help" | "--help" => return Ok(Parsed::Help),
            "-a" | "-b" => {
                let id = it.next().ok_or(format!("{arg} needs an app id"))?;
                if o.app.replace(id.clone()).is_some() {
                    return Err("only one app can be named".into());
                }
            }
            "-e" | "-t" => o.editor = true,
            "-R" => o.reveal = true,
            "-u" => o.url = true,
            other => return Err(format!("option {other} is not supported yet")),
        }
    }
    Ok(Parsed::Run(o))
}

/// One request to the shell's control socket.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    OpenApp(String),
    OpenUrl(String),
    OpenFolder { app: String, folder: PathBuf },
}

impl Request {
    /// The control line for this request, without the newline.
    pub fn line(&self) -> String {
        match self {
            Request::OpenApp(id) => format!("open-app {id}"),
            Request::OpenUrl(url) => format!("open-url {url}"),
            Request::OpenFolder { app, folder } => format!("open-folder {app} {}", folder.to_string_lossy()),
        }
    }
}

/// The requests for a parsed command line, in order. Errors name the operand or option they refer to.
pub fn plan(o: &Options) -> Result<Vec<Request>, String> {
    if o.app.is_some() && o.editor {
        return Err("-e and -t cannot be combined with -a".into());
    }
    if o.app.is_some() || o.editor {
        if !o.operands.is_empty() {
            return Err("opening files with an app is not supported yet (the shell takes no file for an app)".into());
        }
        if o.reveal || o.url {
            return Err("-R and -u need a file or a URL, not an app".into());
        }
        let app = o.app.clone().unwrap_or_else(|| TEXT_EDITOR_APP.to_string());
        return Ok(vec![Request::OpenApp(app)]);
    }
    if o.operands.is_empty() {
        return Err(USAGE.to_string());
    }
    o.operands.iter().map(|op| plan_operand(op, o)).collect()
}

fn plan_operand(op: &str, o: &Options) -> Result<Request, String> {
    if o.url {
        return if is_url(op) { plan_url(op, o) } else { Err(format!("{op}: not a URL")) };
    }
    if let Some(path) = file_url_path(op) {
        return plan_path(&path, o);
    }
    if is_url(op) && !Path::new(op).exists() {
        return plan_url(op, o);
    }
    plan_path(Path::new(op), o)
}

fn plan_url(url: &str, o: &Options) -> Result<Request, String> {
    if o.reveal {
        return Err(format!("{url}: -R needs a file, not a URL"));
    }
    Ok(Request::OpenUrl(url.to_string()))
}

fn plan_path(path: &Path, o: &Options) -> Result<Request, String> {
    let full = std::fs::canonicalize(path).map_err(|e| format!("{}: {}", path.display(), io_text(&e)))?;
    if o.reveal {
        return Ok(Request::OpenFolder { app: FILES_APP.into(), folder: reveal_target(&full) });
    }
    if full.is_dir() {
        return Ok(Request::OpenFolder { app: FILES_APP.into(), folder: full });
    }
    Err(format!("{}: opening a file with its default app is not supported yet (use -R to show its folder)", path.display()))
}

/// The folder Reveal shows for `path`: the folder itself, or the folder that holds the file.
pub fn reveal_target(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map_or_else(|| path.to_path_buf(), Path::to_path_buf)
    }
}

/// Whether `s` has the shape of a URL: a scheme (a letter, then letters, digits, `+`, `-` or `.`), a colon, and more text
/// with no spaces or control characters.
pub fn is_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else { return false };
    !rest.is_empty()
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The path of a `file://` URL (with an optional `localhost` host), percent-decoded. None for any other text.
pub fn file_url_path(s: &str) -> Option<PathBuf> {
    let rest = s.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    rest.starts_with('/').then(|| PathBuf::from(percent_decode(rest)))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        _ => e.to_string(),
    }
}

/// The shell's answer as a result: `ok` succeeds, `error: <why>` fails with the reason.
pub fn reply_result(text: &str) -> Result<(), String> {
    let t = text.trim();
    if t.starts_with("ok") {
        Ok(())
    } else if t.is_empty() {
        Err("no answer from the desktop".into())
    } else {
        Err(t.strip_prefix("error: ").unwrap_or(t).to_string())
    }
}

/// Sends one request to the shell's control socket and returns its answer.
pub fn send(req: &Request, socket: &Path) -> Result<(), String> {
    let answer = control::request(&req.line(), socket)?;
    reply_result(&String::from_utf8_lossy(&answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-open-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn options_and_operands_are_read_in_order() {
        let Parsed::Run(o) = parse(&args(&["-a", "is.x.app", "-R", "one", "--", "-two"])).unwrap() else { panic!("expected a run") };
        assert_eq!(o.app.as_deref(), Some("is.x.app"));
        assert!(o.reveal && !o.editor && !o.url);
        assert_eq!(o.operands, ["one", "-two"]);
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
        assert_eq!(parse(&args(&["-a"])), Err("-a needs an app id".into()));
        assert_eq!(parse(&args(&["-a", "x", "-b", "y"])), Err("only one app can be named".into()));
    }

    #[test]
    fn unsupported_options_are_named_in_the_error() {
        for opt in ["-W", "-n", "--args", "--env"] {
            let err = parse(&args(&[opt])).unwrap_err();
            assert!(err.contains(opt), "{err}");
        }
    }

    #[test]
    fn url_shape_and_file_urls() {
        assert!(is_url("https://example.com/a"));
        assert!(is_url("mailto:a@b.is"));
        assert!(!is_url("/tmp/a:b"));
        assert!(!is_url("notes:"));
        assert!(!is_url("a b:c"));
        assert!(!is_url("plain"));
        assert_eq!(file_url_path("file:///tmp/a%20b.txt"), Some(PathBuf::from("/tmp/a b.txt")));
        assert_eq!(file_url_path("file://localhost/x"), Some(PathBuf::from("/x")));
        assert_eq!(file_url_path("file://host/x"), None);
        assert_eq!(file_url_path("https://x"), None);
    }

    #[test]
    fn app_and_editor_requests_take_no_operands() {
        let o = Options { app: Some("is.x.app".into()), ..Options::default() };
        assert_eq!(plan(&o), Ok(vec![Request::OpenApp("is.x.app".into())]));
        let e = Options { editor: true, ..Options::default() };
        assert_eq!(plan(&e), Ok(vec![Request::OpenApp(TEXT_EDITOR_APP.into())]));
        let both = Options { app: Some("x".into()), editor: true, ..Options::default() };
        assert!(plan(&both).is_err());
        let with_file = Options { app: Some("x".into()), operands: vec!["a.txt".into()], ..Options::default() };
        assert!(plan(&with_file).unwrap_err().contains("not supported yet"));
    }

    #[test]
    fn urls_are_routed_and_reveal_refuses_them() {
        let o = Options { operands: vec!["https://a.is".into()], ..Options::default() };
        assert_eq!(plan(&o), Ok(vec![Request::OpenUrl("https://a.is".into())]));
        let reveal = Options { reveal: true, operands: vec!["https://a.is".into()], ..Options::default() };
        assert!(plan(&reveal).is_err());
        let plain = Options { url: true, operands: vec!["notaurl".into()], ..Options::default() };
        assert!(plan(&plain).unwrap_err().contains("not a URL"));
        assert!(plan(&Options::default()).unwrap_err().starts_with("usage: open"));
    }

    #[test]
    fn folders_open_in_files_and_reveal_shows_the_holding_folder() {
        let dir = scratch("folders");
        let file = dir.join("note.txt");
        std::fs::write(&file, b"x").unwrap();
        let canon_dir = std::fs::canonicalize(&dir).unwrap();

        let open_dir = Options { operands: vec![dir.to_string_lossy().into_owned()], ..Options::default() };
        assert_eq!(plan(&open_dir), Ok(vec![Request::OpenFolder { app: FILES_APP.into(), folder: canon_dir.clone() }]));

        let reveal = Options { reveal: true, operands: vec![file.to_string_lossy().into_owned()], ..Options::default() };
        assert_eq!(plan(&reveal), Ok(vec![Request::OpenFolder { app: FILES_APP.into(), folder: canon_dir }]));

        let plain_file = Options { operands: vec![file.to_string_lossy().into_owned()], ..Options::default() };
        assert!(plan(&plain_file).unwrap_err().contains("not supported yet"));

        let missing = Options { operands: vec![dir.join("nope").to_string_lossy().into_owned()], ..Options::default() };
        assert!(plan(&missing).unwrap_err().contains("No such file or directory"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn request_lines_match_the_control_protocol() {
        assert_eq!(Request::OpenApp("is.x.app".into()).line(), "open-app is.x.app");
        assert_eq!(Request::OpenUrl("https://a.is".into()).line(), "open-url https://a.is");
        let folder = Request::OpenFolder { app: FILES_APP.into(), folder: PathBuf::from("/home/me/My Files") };
        assert_eq!(folder.line(), "open-folder is.olibuijr.files.desktop /home/me/My Files");
    }

    #[test]
    fn replies_map_ok_and_errors() {
        assert_eq!(reply_result("ok\n"), Ok(()));
        assert_eq!(reply_result("error: App not installed\n"), Err("App not installed".into()));
        assert_eq!(reply_result("  "), Err("no answer from the desktop".into()));
    }

    #[test]
    fn send_writes_the_line_and_reads_the_answer() {
        let dir = std::env::temp_dir().join(format!("bifrost-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("c.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let n = conn.read(&mut buf).unwrap();
            conn.write_all(b"ok\n").unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        send(&Request::OpenApp("is.olibuijr.notes.desktop".into()), &sock).unwrap();
        assert_eq!(server.join().unwrap(), "open-app is.olibuijr.notes.desktop\n");
    }
}
