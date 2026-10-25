//! App Intents in the shell (`AppShortcut` in Search). Apps declare intents in `manifest.toml` as `[[intent]]`
//! tables (SDK: `bifrost_app::intents`); the shell reads them without launching the app, lists the runnable ones
//! in Search's "Shortcuts" section, and runs one by launching the app and calling `intent.<id>` on its control
//! socket (`control.sock` in the app's data directory, `bifrost_core::app_control_socket`; JSON-RPC 2.0, one request per
//! connection).

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

/// The control socket of a running app: `control.sock` in its data directory (`<root>/appdata/<app id>`), where the
/// SDK runtime binds it. The sandbox binds the data directory at its own path, so the shell sees the same socket.
pub fn socket_path(app_id: &str) -> PathBuf {
    bifrost_core::app_control_socket::socket_for(&super::apphost::root(), app_id)
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

/// The installed apps that answer visual searches: those that declare `[[intent]] id = "visual-search"` in their
/// manifest (`bifrost_ui::visual_search_wire::PROVIDER_INTENT`). Reads only what the manifests say.
pub fn visual_search_apps(apps: &[super::apphost::Installed]) -> Vec<String> {
    apps.iter()
        .filter(|a| a.manifest.intents.iter().any(|i| i.id == bifrost_ui::visual_search_wire::PROVIDER_INTENT))
        .map(|a| a.manifest.id.clone())
        .collect()
}

/// Asks the app listening at `path` which of its content matches `scene` (`visual_search.search`, see
/// `bifrost_ui::visual_search_wire`). Every match is stamped with `app_id`, whatever the app wrote, so an app cannot
/// answer for another one. An app that cannot be reached within `wait`, refuses, or answers badly gives no matches.
/// Blocking: run it off the UI thread.
pub fn visual_search_at(path: &std::path::Path, app_id: &str, scene: &bifrost_ui::visual_search::Scene, wait: Duration) -> Vec<bifrost_ui::visual_search::Match> {
    let line = bifrost_ui::visual_search_wire::request_line(scene, 1);
    let Ok(reply) = exchange_at(path, &line, wait) else { return Vec::new() };
    let mut found = bifrost_ui::visual_search_wire::matches_from_reply(&reply).unwrap_or_default();
    for m in &mut found {
        m.app = app_id.to_string();
    }
    found
}

/// Delivers `scene` to every running app that answers visual searches and returns the merged matches, best first, at
/// most `MAX_MATCHES`. An app that is not running is not started and is skipped. Blocking (each answering app may take
/// a few seconds): run it off the UI thread.
pub fn visual_search(apps: &[super::apphost::Installed], scene: &bifrost_ui::visual_search::Scene) -> Vec<bifrost_ui::visual_search::Match> {
    let mut all = Vec::new();
    for id in visual_search_apps(apps) {
        all.extend(visual_search_at(&socket_path(&id), &id, scene, Duration::ZERO));
    }
    bifrost_ui::visual_search::merge(all, bifrost_ui::visual_search::MAX_MATCHES)
}

fn run_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<Option<String>, String> {
    parse_reply(&exchange_at(path, line, wait)?)
}

fn exchange_at(path: &std::path::Path, line: &str, wait: Duration) -> Result<String, String> {
    let deadline = Instant::now() + wait;
    let dial_as = super::screenshot::account_owner();
    let mut stream = loop {
        match dial(path, dial_as) {
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

/// Connects to an app's control socket. `as_account` is the installed account's `(uid, gid)` when the shell runs as
/// root ([`super::screenshot::account_owner`]): apps run in a user namespace that maps only the account's uid, so a
/// connection made as root reaches the app as the overflow uid and the SDK's peer check (own uid) refuses it. The
/// connect then runs on a short-lived thread that takes the account's credentials, because `SO_PEERCRED` records the
/// credentials of the task that calls `connect`. Without an account, or when the shell is not root, the socket is
/// dialled directly as the process's own user.
fn dial(path: &std::path::Path, as_account: Option<(u32, u32)>) -> std::io::Result<UnixStream> {
    match as_account {
        #[cfg(target_os = "linux")]
        Some((uid, gid)) => account_dial::connect_as(path, uid, gid),
        _ => UnixStream::connect(path),
    }
}

/// The account-credential connect: one thread whose own credentials (not the process's) become the account's.
#[cfg(target_os = "linux")]
mod account_dial {
    use std::io;
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    #[cfg(target_arch = "x86_64")]
    mod nr {
        pub const SETGROUPS: i64 = 116;
        pub const SETRESUID: i64 = 117;
        pub const SETRESGID: i64 = 119;
    }
    #[cfg(target_arch = "aarch64")]
    mod nr {
        pub const SETGROUPS: i64 = 159;
        pub const SETRESUID: i64 = 147;
        pub const SETRESGID: i64 = 149;
    }

    extern "C" {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn syscall(n: i64, ...) -> i64;
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        fn geteuid() -> u32;
        fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    }
    const PR_GET_DUMPABLE: i32 = 3;
    const PR_SET_DUMPABLE: i32 = 4;

    /// Switches the calling thread, and only it, to `uid`/`gid` (real, effective and saved, plus the supplementary
    /// groups when it may set them). The raw syscalls are used on purpose: the libc wrappers apply a change to every
    /// thread of the process, which would drop the whole shell's privileges.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) fn become_thread(uid: u32, gid: u32) -> io::Result<()> {
        // SAFETY: plain credential syscalls on the calling thread; `groups` lives across the call.
        unsafe {
            if geteuid() == 0 {
                let groups = [gid];
                if syscall(nr::SETGROUPS, 1i64, groups.as_ptr()) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // The group first: once the uid is dropped the thread may no longer change it.
            if syscall(nr::SETRESGID, gid as i64, gid as i64, gid as i64) != 0 {
                return Err(io::Error::last_os_error());
            }
            if syscall(nr::SETRESUID, uid as i64, uid as i64, uid as i64) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    pub(super) fn become_thread(_uid: u32, _gid: u32) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "per-thread credentials are not wired for this architecture"))
    }

    /// Connects to `path` as `uid`/`gid` from a dedicated thread that ends right after, so the shell's other threads
    /// keep their credentials. The returned stream belongs to the process and is used as usual. A credential change
    /// resets the process's dumpable flag (the kernel does it on the shared mm), so it is put back afterwards and the
    /// shell keeps its core dumps.
    pub fn connect_as(path: &Path, uid: u32, gid: u32) -> io::Result<UnixStream> {
        // SAFETY: PR_GET_DUMPABLE takes no pointers.
        let dumpable = unsafe { prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) };
        let path = path.to_path_buf();
        let joined = std::thread::Builder::new()
            .name("app-dial".into())
            .spawn(move || {
                become_thread(uid, gid)?;
                UnixStream::connect(&path)
            })?
            .join();
        if dumpable >= 0 {
            // SAFETY: PR_GET_DUMPABLE and PR_SET_DUMPABLE take plain integers.
            unsafe {
                if prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) != dumpable {
                    prctl(PR_SET_DUMPABLE, dumpable as usize, 0, 0, 0);
                }
            }
        }
        joined.map_err(|_| io::Error::other("the dial thread panicked"))?
    }
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

    /// The uid the kernel recorded for the connecting end of `s`.
    #[cfg(target_os = "linux")]
    fn peer_uid(s: &std::os::unix::net::UnixStream) -> u32 {
        use std::os::fd::AsRawFd;
        #[repr(C)]
        struct Ucred {
            pid: i32,
            uid: u32,
            gid: u32,
        }
        extern "C" {
            fn getsockopt(fd: i32, level: i32, name: i32, val: *mut std::ffi::c_void, len: *mut u32) -> i32;
        }
        let mut cred = Ucred { pid: 0, uid: u32::MAX, gid: 0 };
        let mut len = std::mem::size_of::<Ucred>() as u32;
        // SAFETY: cred is a live ucred of the length passed; SOL_SOCKET = 1, SO_PEERCRED = 17.
        assert_eq!(unsafe { getsockopt(s.as_raw_fd(), 1, 17, &mut cred as *mut Ucred as *mut std::ffi::c_void, &mut len) }, 0);
        cred.uid
    }

    #[cfg(target_os = "linux")]
    extern "C" {
        fn geteuid() -> u32;
        fn getegid() -> u32;
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_connects_from_its_own_thread_and_leaves_the_caller_alone() {
        // without root the dial can only take the credentials it already has; the connection is made from the dial
        // thread, the app sees those credentials, and the calling thread (and the process) keep theirs
        let dir = std::env::temp_dir().join(format!("bifrost-intents-dial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("d.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let seen = peer_uid(&s);
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":null,\"dialog\":\"Dialled\"}}\n").unwrap();
            seen
        });
        let mut s = dial(&sock, Some((uid, gid))).unwrap();
        s.write_all(request_line("go").as_bytes()).unwrap();
        let _ = s.shutdown(std::net::Shutdown::Write);
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        assert_eq!(parse_reply(&reply), Ok(Some("Dialled".into())));
        assert_eq!(h.join().unwrap(), uid, "SO_PEERCRED carries the dial thread's uid");
        assert_eq!(unsafe { (geteuid(), getegid()) }, (uid, gid), "the caller's credentials are untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_account_dial_cannot_take_a_foreign_uid_without_privilege() {
        // proves the credential switch really runs: a non-root dial thread may not become another user, so the dial
        // fails with EPERM instead of quietly connecting as itself (skipped as root, where the switch succeeds)
        if unsafe { geteuid() } == 0 {
            return;
        }
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        let err = dial(std::path::Path::new("/nonexistent/bifrost-dial.sock"), Some((uid + 1, gid))).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
        assert_eq!(unsafe { geteuid() }, uid);
    }

    #[test]
    fn without_an_account_the_socket_is_dialled_directly() {
        assert!(dial(std::path::Path::new("/nonexistent/bifrost-dial.sock"), None).is_err());
    }

    #[test]
    fn the_shell_dials_the_control_socket_in_the_app_data_directory() {
        // the sandboxed app has no host runtime directory: it binds `control.sock` in BIFROST_DATA, which the launcher
        // sets to `<root>/appdata/<id>` and binds at that same path
        let id = "is.olibuijr.dictionary.desktop";
        let data = super::super::apphost::root().join("appdata").join(id);
        assert_eq!(socket_path(id), data.join("control.sock"));
        assert_eq!(socket_path(id), bifrost_core::app_control_socket::socket_path(None, &data), "the SDK runtime's rule");
        assert!(!socket_path(id).to_string_lossy().contains("bifrost/apps"));
    }

    #[test]
    fn only_apps_that_declare_the_visual_search_intent_are_asked() {
        use super::super::apphost::{Installed, Manifest};
        let apps = [
            Installed {
                manifest: Manifest::parse("id = \"a.plants\"\nname = \"Plants\"\nexec = \"x\"\n\n[[intent]]\nid = \"visual-search\"\ntitle = \"Visual Search\"\nshortcut = false\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
            Installed {
                manifest: Manifest::parse("id = \"a.notes\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n").unwrap(),
                dir: std::path::PathBuf::new(),
            },
        ];
        assert_eq!(visual_search_apps(&apps), vec!["a.plants".to_string()]);
        assert!(visual_search_apps(&[]).is_empty());
        let scene = bifrost_ui::visual_search::Scene::new(bifrost_ui::visual_search::SceneKind::Screenshot, 1, 1, vec![0, 0, 0, 255]).unwrap();
        assert!(visual_search(&apps, &scene).is_empty(), "an app that is not running is skipped, not started");
    }

    #[test]
    fn a_visual_search_goes_to_the_app_socket_and_the_matches_are_stamped_with_that_app() {
        use bifrost_ui::visual_search::{Scene, SceneKind};
        let dir = std::env::temp_dir().join(format!("bifrost-visual-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("p.sock");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&s), &mut req).unwrap();
            assert!(req.contains("\"visual_search.search\""));
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{\"intent\":\"open\",\"title\":\"Fern\",\"score\":70}]}\n").unwrap();
        });
        let scene = Scene::new(SceneKind::Screenshot, 2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap().with_labels(&["plant"]);
        let found = visual_search_at(&sock, "is.x.plants", &scene, Duration::from_secs(1));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].app.as_str(), found[0].title.as_str(), found[0].score), ("is.x.plants", "Fern", 70));
        h.join().unwrap();
        assert!(visual_search_at(&dir.join("none.sock"), "is.x.gone", &scene, Duration::ZERO).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
