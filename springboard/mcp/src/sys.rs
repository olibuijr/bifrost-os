//! The shell's control port, app control sockets, processes and files: plain syscalls, no helpers.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const RUNTIME: &str = "/run/bifrost/user";
pub const APPDATA: &str = "/var/lib/springboard/appdata";

/// One request on the shell's control port (127.0.0.1:7777, one line per connection).
pub fn shell(line: &str) -> Result<Vec<u8>, String> {
    let port = std::env::var("SPRINGBOARD_CONTROL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7777u16);
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)).map_err(|e| format!("shell control port: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    s.write_all(format!("{}\n", line.trim()).as_bytes()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    s.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn shell_text(line: &str) -> Result<String, String> {
    let r = String::from_utf8_lossy(&shell(line)?).into_owned();
    match r.strip_prefix("error:") {
        Some(e) => Err(e.trim().to_string()),
        None => Ok(r),
    }
}

/// The local account (`username=` in /var/lib/springboard/account) and its uid/gid.
pub fn account() -> Option<(String, u32, u32)> {
    let text = std::fs::read_to_string("/var/lib/springboard/account").ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("username="))?.trim().to_string();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    let f: Vec<&str> = passwd.lines().find(|l| l.split(':').next() == Some(&name))?.split(':').collect();
    Some((name, f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?))
}

/// The Wayland session environment the shell published (`session.json`).
pub fn session_env() -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(Path::new(RUNTIME).join("session.json")) else {
        return vec![("XDG_RUNTIME_DIR".into(), RUNTIME.into()), ("WAYLAND_DISPLAY".into(), "wayland-0".into())];
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    v.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()
}

/// Control sockets of running apps: (app id, socket path).
pub fn app_sockets() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(dirs) = std::fs::read_dir(APPDATA) {
        for d in dirs.flatten() {
            let p = d.path().join("control.sock");
            if p.exists() {
                out.push((d.file_name().to_string_lossy().into_owned(), p));
            }
        }
    }
    for base in [PathBuf::from(RUNTIME).join("bifrost/apps")] {
        if let Ok(files) = std::fs::read_dir(base) {
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if let Some(id) = name.strip_suffix(".sock") {
                    if !out.iter().any(|(i, _)| i == id) {
                        out.push((id.to_string(), f.path()));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// A JSON-RPC error from an app (or a transport failure, code 0).
#[derive(Debug, Clone)]
pub struct RpcErr {
    pub code: i64,
    pub message: String,
    pub data: serde_json::Value,
}

impl std::fmt::Display for RpcErr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.code == 0 {
            write!(f, "{}", self.message)
        } else {
            let reason = self.data.get("reason").and_then(|r| r.as_str()).map(|r| format!(" ({r})")).unwrap_or_default();
            write!(f, "{} [rpc {}]{reason}", self.message, self.code)
        }
    }
}

fn transport(m: impl Into<String>) -> RpcErr {
    RpcErr { code: 0, message: m.into(), data: serde_json::Value::Null }
}

/// One JSON-RPC call on an app's control socket. The socket admits only its owner (SO_PEERCRED),
/// so the connect runs with the account's effective uid.
pub fn app_rpc(path: &Path, method: &str, params: &serde_json::Value, timeout: Duration) -> Result<serde_json::Value, String> {
    app_rpc_full(path, method, params, timeout).map_err(|e| e.message)
}

/// Like `app_rpc`, but keeps the JSON-RPC error code and data.
pub fn app_rpc_full(path: &Path, method: &str, params: &serde_json::Value, timeout: Duration) -> Result<serde_json::Value, RpcErr> {
    let uid = std::fs::metadata(path).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).map_err(|e| transport(e.to_string()))?;
    let me = unsafe { libc::geteuid() };
    if me == 0 && uid != 0 {
        unsafe { libc::seteuid(uid) };
    }
    let conn = UnixStream::connect(path);
    if me == 0 && uid != 0 {
        unsafe { libc::seteuid(0) };
    }
    let mut s = conn.map_err(|e| transport(format!("not running ({e})")))?;
    s.set_read_timeout(Some(timeout)).ok();
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    s.write_all(format!("{req}\n").as_bytes()).map_err(|e| transport(e.to_string()))?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| transport(format!("no reply: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(out.trim()).map_err(|e| transport(format!("malformed reply: {e}")))?;
    if let Some(e) = v.get("error") {
        return Err(RpcErr { code: e["code"].as_i64().unwrap_or(-32000), message: e["message"].as_str().unwrap_or("error").to_string(), data: e["data"].clone() });
    }
    Ok(v["result"].clone())
}

pub struct Ran {
    pub code: Option<i32>,
    pub out: String,
    pub err: String,
    pub timed_out: bool,
    pub ms: u128,
}

/// Runs `bash -c command` (as root, or as the account inside the Wayland session), with a deadline.
pub fn exec(command: &str, as_user: bool, cwd: Option<&str>, timeout: Duration, stdin: Option<&str>) -> Result<Ran, String> {
    let t = Instant::now();
    let mut cmd = if as_user {
        let (name, ..) = account().ok_or("no local account")?;
        let mut c = Command::new("runuser");
        c.args(["-u", &name, "--", "env"]);
        for (k, v) in session_env() {
            c.arg(format!("{k}={v}"));
        }
        c.args(["bash", "-c", command]);
        c
    } else {
        let mut c = Command::new("bash");
        c.args(["-c", command]);
        c
    };
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let (mut o, mut e) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let ot = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = o.read_to_end(&mut b);
        b
    });
    let et = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = e.read_to_end(&mut b);
        b
    });
    let pid = child.id() as i32;
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(s);
        }
        if t.elapsed() > timeout {
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            timed_out = true;
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    Ok(Ran {
        code: status.and_then(|s| s.code()),
        out: String::from_utf8_lossy(&ot.join().unwrap_or_default()).into_owned(),
        err: String::from_utf8_lossy(&et.join().unwrap_or_default()).into_owned(),
        timed_out,
        ms: t.elapsed().as_millis(),
    })
}

/// Running processes from /proc: (pid, rss KiB, cpu ticks, command line).
pub fn processes() -> Vec<(u32, u64, u64, String)> {
    let mut v = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ticks = f.get(11).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let rss = f.get(21).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 4;
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let mut cmd = String::from_utf8_lossy(&cmd).replace('\0', " ").trim().to_string();
        if cmd.is_empty() {
            cmd = format!("[{}]", stat.split_once('(').and_then(|(_, r)| r.rsplit_once(')')).map(|(n, _)| n).unwrap_or("?"));
        }
        v.push((pid, rss, ticks, cmd));
    }
    v
}

pub fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
