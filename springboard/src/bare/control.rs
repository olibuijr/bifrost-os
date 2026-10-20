//! The shell's control socket: a Unix stream socket at `/run/bifrost/shell-control.sock` (mode 0600, owned by the
//! installed account, which runs the window server). There is no TCP listener: a loopback port would be open to every
//! local process, sandboxed apps included, which share the host network. The kernel reports the uid of each peer
//! (`SO_PEERCRED`); only root (lab tools over ssh, `bifrost-mcp`), the shell's own uid and the account the window server
//! runs as are served, everyone else is refused and logged once per uid. The sandbox mounts a fresh `/run`, so apps do not
//! even see the path.
//! One request per connection, a single text line:
//!   shot [scale]                 -> "RIMG" w h + premultiplied RGBA, downscaled
//!   tap x y                      -> "ok"
//!   swipe|drag x0 y0 x1 y1 [ms]  -> "ok"
//!   down|move|up x y             -> "ok" (a live finger: press, drag step, lift)
//!   hold|longpress x y [ms]      -> "ok" (finger held in place, default 700 ms)
//!   doubletap x y                -> "ok"
//!   key power|volup|voldown      -> "ok"
//!   state                        -> key=value lines (version, ready, fps)
//!   version                      -> version=X.Y.Z
//!   ready                        -> ready=1 once the desktop has drawn its first frame
//!   trace on|off                 -> "ok" (log every input event)
//!   lock                         -> "ok" (menu > Lock Screen)
//!   quicksearch                    -> "ok" (opens Search, as Cmd+Space; the compositor's Super+Space binding)
//!   overview                       -> "ok" (Overview: opens it, or closes it when open; the compositor's Ctrl+Up and F3 bindings)
//!   appgrid                        -> "ok" (the Apps grid: opens it, or closes it when open; the compositor's F4 binding)
//!   image-attach ro|rw <path>      -> "ok <loop device> <folder>" (binds a disk image and mounts it, `shell::disk_image`)
//!   image-detach <device|folder>   -> "ok <loop device>" (unmounts a disk image and releases its loop device)

use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Where the shell listens.
pub const SOCKET: &str = "/run/bifrost/shell-control.sock";
/// The socket's mode: its owner (and root) only.
pub const SOCKET_MODE: u32 = 0o600;

extern "C" {
    fn geteuid() -> u32;
    fn chown(path: *const std::os::raw::c_char, uid: u32, gid: u32) -> i32;
}

/// The socket to listen on: `SPRINGBOARD_CONTROL_SOCKET` when it is an absolute path (several emulators on one host, a
/// shell run without root), else `SOCKET`.
pub fn socket_path() -> PathBuf {
    socket_path_from(std::env::var("SPRINGBOARD_CONTROL_SOCKET").ok().as_deref())
}

fn socket_path_from(v: Option<&str>) -> PathBuf {
    match v.map(str::trim) {
        Some(p) if p.starts_with('/') && p.len() > 1 => PathBuf::from(p),
        _ => PathBuf::from(SOCKET),
    }
}

/// The uids served: root, the uid the shell runs as, and the installed account (the window server's uid) when known.
pub fn allowed_uids(own: u32, account: Option<u32>) -> Vec<u32> {
    let mut v = vec![0, own];
    v.extend(account);
    v.sort_unstable();
    v.dedup();
    v
}

/// True when a peer with this uid may use the control socket.
pub fn peer_allowed(uid: u32, allowed: &[u32]) -> bool {
    allowed.contains(&uid)
}
/// Longest synthetic swipe.
pub const MAX_SWIPE_MS: f32 = 10_000.0;
/// A `hold` without a duration: past bifrost's long-press threshold.
pub const HOLD_MS: f32 = 700.0;
/// Longest request line read.
const MAX_LINE: u64 = 65_536;

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Shot(usize),
    /// `capture select|window|screen`: opens the screenshot toolbar's capture session with that pick (`screencapture -i -w`)
    Capture(String),
    /// `rec start [screen|x,y,w,h] [file]`: starts a screen recording of the whole screen or of an area in screen pixels,
    /// into `file` (percent-encoded) or else a dated file on the Desktop
    RecStart { area: Option<(u32, u32, u32, u32)>, file: Option<String> },
    /// `rec stop`: ends the recording and answers with its file
    RecStop,
    /// `rec status`: `idle`, or `recording <file>`
    RecStatus,
    Tap(f32, f32),
    Context(f32,f32),
    Hover(f32,f32),
    OpenUrl(String),
    OpenApp(String),
    /// Settings > Appearance, live: `appearance light|dark|auto`.
    Appearance(String),
    /// Opens an installed app at a folder: `open-folder <app id> <absolute folder>`.
    OpenFolder(String, String),
    Swipe { from: (f32, f32), to: (f32, f32), ms: f32 },
    /// One finger event now: phase 0 down, 1 move, 2 up (see `path`).
    Touch(u8, f32, f32),
    DoubleTap(f32, f32),
    Key(Key),
    State,
    /// `ax list [prefix]`: the element map (docs/agent-api.md)
    AxList(String),
    /// `ax press <address or name>`
    AxPress(String),
    /// `ax actions <address or name>`: the custom action names of one element
    AxActions(String),
    /// `ax action <address or name> :: <action name>`: runs one custom action
    AxAction(String, String),
    Version,
    Ready,
    Perf,
    Trace(bool),
    QuickSearch,
    /// `overview`: Overview opens or closes (Ctrl+Up, F3)
    Overview,
    /// `appgrid`: the Apps grid opens or closes (F4)
    AppGrid,
    /// `forcequit`: the Force Quit window opens (Option+Cmd+Esc, taken by the window server)
    ForceQuit,
    /// `logout`: the Log Out card opens (Shift+Cmd+Q, taken by the window server)
    LogOut,
    Lock,
    /// `term <verb> key=value ...` (values percent-encoded): Term agent control, see `shell/terminal/control.rs`.
    Term(String, Vec<(String, String)>),
    /// `notify <verb> <args>`: the named-notification bus for the command line (`notifyutil`). The words are checked by
    /// `shell::services::notify_bus::control`, which runs them as a call on the bus service.
    Notify(Vec<String>),
    /// `image-attach ro|rw <absolute image path>`: binds the image to a loop device and mounts it (`shell::disk_image`).
    ImageAttach { read_only: bool, path: String },
    /// `image-detach <loop device or mount folder>`: unmounts the image and releases its loop device.
    ImageDetach(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Power,
    VolumeUp,
    VolumeDown,
}

/// What the control port still serves while the screen is locked: status, a screenshot, locking, and raw input (which the
/// shell routes to the login window only). Opening apps, folders or URLs, pressing or listing elements, appearance,
/// Term control, Search and desktop pointer events wait for a login.
pub fn allowed_while_locked(r: &Request) -> bool {
    matches!(
        r,
        Request::Shot(_) | Request::State | Request::Version | Request::Ready | Request::Perf | Request::Trace(_) | Request::Lock
            | Request::Key(_) | Request::Tap(..) | Request::Swipe { .. } | Request::Touch(..) | Request::DoubleTap(..)
    )
}

/// Percent-encodes everything but `[A-Za-z0-9._/-]`, so a value is one whitespace-free word.
pub fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"._/-".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// Inverse of `pct_encode`; malformed escapes stay literal.
pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                o.push(v);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// Parses `x,y,w,h` in screen pixels: a width and height of at least one pixel.
pub fn parse_area(s: &str) -> Result<(u32, u32, u32, u32), String> {
    let p: Vec<u32> = s.split(',').map(|v| v.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| format!("the area is x,y,w,h, not {s}"))?;
    match p[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok((x, y, w, h)),
        [_, _, _, _] => Err("the area needs a width and height of at least one pixel".into()),
        _ => Err(format!("the area is x,y,w,h, not {s}")),
    }
}

pub fn parse(line: &str) -> Result<Request, String> {
    if let Some(rest) = line.trim().strip_prefix("term ") {
        let mut w = rest.split_whitespace();
        let verb = w.next().ok_or("usage: term <verb> key=value ...")?.to_string();
        let args = w.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), pct_decode(v))).collect();
        return Ok(Request::Term(verb, args));
    }
    if let Some(rest) = line.trim().strip_prefix("notify ") {
        return Ok(Request::Notify(rest.split_whitespace().map(str::to_string).collect()));
    }
    // the folder may contain spaces, so it takes the rest of the line
    if let Some(rest) = line.trim_end().strip_prefix("open-folder ") {
        let (id, folder) = rest.split_once(' ').ok_or("Expected an app ID and a folder")?;
        return Ok(Request::OpenFolder(id.into(), folder.into()));
    }
    // the image path may contain spaces, so it takes the rest of the line
    if let Some(rest) = line.trim_end().strip_prefix("image-attach ") {
        let (mode, path) = rest.split_once(' ').ok_or("Expected ro or rw and an image path")?;
        let read_only = match mode {
            "ro" => true,
            "rw" => false,
            _ => return Err("Expected ro or rw".into()),
        };
        return Ok(Request::ImageAttach { read_only, path: path.into() });
    }
    if let Some(rest) = line.trim_end().strip_prefix("image-detach ") {
        return Ok(Request::ImageDetach(rest.trim().into()));
    }
    let w: Vec<&str> = line.split_whitespace().collect();
    let num = |i: usize| -> Result<f32, String> { w.get(i).ok_or(format!("missing argument {i}"))?.parse::<f32>().map_err(|e| e.to_string()).and_then(|v| if v.is_finite() { Ok(v) } else { Err("not a finite number".to_string()) }) };
    match w.first().copied() {
        Some("shot") => Ok(Request::Shot(w.get(1).map(|s| s.parse().unwrap_or(1)).unwrap_or(1).clamp(1, 8))),
        Some("capture") if w.len() == 2 && matches!(w[1], "select" | "window" | "screen") => Ok(Request::Capture(w[1].into())),
        Some("open-app") if w.len()==2=>Ok(Request::OpenApp(w[1].into())),
        Some("appearance") if matches!(w.get(1).copied(),Some("light"|"dark"|"auto"))&&w.len()==2=>Ok(Request::Appearance(w[1].into())),
        Some("context")=>Ok(Request::Context(num(1)?,num(2)?)),
        Some("hover")=>Ok(Request::Hover(num(1)?,num(2)?)),
        Some("open-url")=>{let url=w.get(1).ok_or("Missing URL")?;let scheme_ok=url.split_once(':').is_some_and(|(s,r)|!r.is_empty()&&s.as_bytes().first().is_some_and(|c|c.is_ascii_alphabetic())&&s.bytes().all(|c|c.is_ascii_alphanumeric()||b"+-.".contains(&c)));if w.len()!=2||!scheme_ok{return Err("Expected a URL with a scheme, such as https: or mailto:".into());}Ok(Request::OpenUrl((*url).into()))},
        Some("tap") => Ok(Request::Tap(num(1)?, num(2)?)),
        Some("down") => Ok(Request::Touch(0, num(1)?, num(2)?)),
        Some("move") => Ok(Request::Touch(1, num(1)?, num(2)?)),
        Some("up") => Ok(Request::Touch(2, num(1)?, num(2)?)),
        Some("hold" | "longpress") => {
            let (x, y) = (num(1)?, num(2)?);
            Ok(Request::Swipe { from: (x, y), to: (x, y), ms: if w.len() > 3 { num(3)? } else { HOLD_MS }.clamp(0.0, MAX_SWIPE_MS) })
        }
        Some("doubletap") => Ok(Request::DoubleTap(num(1)?, num(2)?)),
        Some("swipe" | "drag") => Ok(Request::Swipe { from: (num(1)?, num(2)?), to: (num(3)?, num(4)?), ms: if w.len() > 5 { num(5)? } else { 300.0 }.clamp(0.0, MAX_SWIPE_MS) }),
        Some("key") => match w.get(1).copied() {
            Some("power") => Ok(Request::Key(Key::Power)),
            Some("volup") => Ok(Request::Key(Key::VolumeUp)),
            Some("voldown") => Ok(Request::Key(Key::VolumeDown)),
            k => Err(format!("unknown key {k:?}")),
        },
        Some("ax") => match (w.get(1).copied(), line.trim().splitn(3, ' ').nth(2)) {
            (Some("list") | None, rest) => Ok(Request::AxList(rest.unwrap_or("").trim().into())),
            (Some("press"), Some(q)) if !q.trim().is_empty() => Ok(Request::AxPress(q.trim().into())),
            (Some("actions"), Some(q)) if !q.trim().is_empty() => Ok(Request::AxActions(q.trim().into())),
            (Some("action"), Some(q)) => match q.split_once("::") {
                Some((el, name)) if !el.trim().is_empty() && !name.trim().is_empty() => Ok(Request::AxAction(el.trim().into(), name.trim().into())),
                _ => Err("usage: ax action <address or name> :: <action name>".into()),
            },
            _ => Err("usage: ax list [prefix] | ax press <address or name> | ax actions <element> | ax action <element> :: <name>".into()),
        },
        Some("rec") => match w.get(1).copied() {
            Some("start") if w.len() <= 4 => {
                let mut rest: Vec<&str> = w[2..].to_vec();
                let mut area = None;
                if rest.first().is_some_and(|a| a.contains(',')) {
                    area = Some(parse_area(rest.remove(0))?);
                } else if rest.first() == Some(&"screen") {
                    rest.remove(0);
                }
                let file = rest.first().map(|f| pct_decode(f));
                Ok(Request::RecStart { area, file })
            }
            Some("stop") if w.len() == 2 => Ok(Request::RecStop),
            Some("status") if w.len() == 2 => Ok(Request::RecStatus),
            _ => Err("usage: rec start [screen|x,y,w,h] [file] | rec stop | rec status".into()),
        },
        Some("state") => Ok(Request::State),
        Some("version") => Ok(Request::Version),
        Some("ready") => Ok(Request::Ready),
        Some("perf") => Ok(Request::Perf),
        Some("trace") => Ok(Request::Trace(w.get(1) == Some(&"on"))),
        Some("quicksearch") => Ok(Request::QuickSearch),
        Some("overview") if w.len() == 1 => Ok(Request::Overview),
        Some("appgrid") if w.len() == 1 => Ok(Request::AppGrid),
        Some("forcequit") if w.len() == 1 => Ok(Request::ForceQuit),
        Some("logout") if w.len() == 1 => Ok(Request::LogOut),
        Some("lock") if w.len() == 1 => Ok(Request::Lock),
        c => Err(format!("unknown command {c:?}")),
    }
}

/// Box-filtered downscale of a w*h frame by `k`, as a RIMG blob.
pub fn snapshot(frame: &[u32], w: usize, h: usize, k: usize) -> Vec<u8> {
    let k = k.max(1);
    let (ow, oh) = (w / k, h / k);
    if frame.len() < w * h {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(12 + ow * oh * 4);
    out.extend(b"RIMG");
    out.extend((ow as u32).to_le_bytes());
    out.extend((oh as u32).to_le_bytes());
    let n = (k * k) as u32;
    for y in 0..oh {
        for x in 0..ow {
            let mut acc = [0u32; 4];
            for dy in 0..k {
                for dx in 0..k {
                    let p = frame[(y * k + dy) * w + x * k + dx];
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += (p >> (8 * c)) & 255;
                    }
                }
            }
            out.extend(acc.map(|a| ((a + n / 2) / n) as u8));
        }
    }
    out
}

/// A synthetic touch path: (time offset ms, phase, x, y), phase 0 down,
/// 1 move, 2 up; moves every ~16 ms.
pub fn path(from: (f32, f32), to: (f32, f32), ms: f32) -> Vec<(f64, u8, f32, f32)> {
    let ms = ms.clamp(0.0, MAX_SWIPE_MS);
    let steps = ((ms / 16.0).round() as usize).max(1);
    let mut v = vec![(0.0, 0, from.0, from.1)];
    for i in 1..=steps {
        let k = i as f32 / steps as f32;
        v.push(((ms * k) as f64, 1, from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k));
    }
    v.push((ms as f64 + 1.0, 2, to.0, to.1));
    v
}

/// Two taps 100 ms apart, as (time offset ms, phase, x, y).
pub fn double_tap(x: f32, y: f32) -> Vec<(f64, u8, f32, f32)> {
    vec![(0.0, 0, x, y), (40.0, 2, x, y), (140.0, 0, x, y), (180.0, 2, x, y)]
}

pub struct Server {
    listener: UnixListener,
    allowed: Vec<u32>,
    /// uids already logged as refused, so a busy local process cannot flood the journal
    refused: Mutex<HashSet<u32>>,
}

/// Binds `path` as a socket with `SOCKET_MODE`, owned by `owner` (uid, gid) when given.
///
/// The socket is made inside a private staging directory (mode 0700), given its owner and mode, and renamed into place,
/// so it is never reachable with the default creation mode. The process umask is not touched (it is process-wide).
pub fn bind_at(path: &Path, owner: Option<(u32, u32)>) -> io::Result<UnixListener> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let _ = std::fs::remove_file(path);
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let stage = parent.join(format!(".{name}.stage"));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::DirBuilder::new().mode(0o700).create(&stage)?;
    let staged = stage.join("s");
    let made = (|| {
        let l = UnixListener::bind(&staged)?;
        if let Some((uid, gid)) = owner {
            let c = std::ffi::CString::new(staged.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
            // SAFETY: `c` is a valid NUL-terminated path.
            if unsafe { chown(c.as_ptr(), uid, gid) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        std::fs::rename(&staged, path)?;
        Ok(l)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    made
}

impl Server {
    pub fn bind() -> io::Result<Self> {
        // SAFETY: geteuid has no preconditions.
        let own = unsafe { geteuid() };
        let account = crate::privd::account_ids();
        // as root the socket belongs to the account the window server runs as; root connects whatever the mode
        let owner = if own == 0 { account.as_ref().map(|(uid, gid, _)| (*uid, *gid)) } else { None };
        let listener = bind_at(&socket_path(), owner)?;
        listener.set_nonblocking(true)?;
        Ok(Server { listener, allowed: allowed_uids(own, account.map(|(uid, ..)| uid)), refused: Mutex::new(HashSet::new()) })
    }

    pub fn fd(&self) -> std::ffi::c_int {
        use std::os::fd::AsRawFd;
        self.listener.as_raw_fd()
    }

    /// Checks one connection's peer credentials: `true` to serve it. A refused uid is logged the first time only.
    fn admit(&self, stream: &UnixStream) -> bool {
        match crate::privd::server::peer(stream) {
            Ok((_, uid)) if peer_allowed(uid, &self.allowed) => true,
            Ok((pid, uid)) => {
                if self.refused.lock().is_ok_and(|mut seen| seen.insert(uid)) {
                    eprintln!("springboard: control socket: refused uid {uid} (pid {pid})");
                }
                false
            }
            Err(e) => {
                eprintln!("springboard: control socket: no peer credentials: {e}");
                false
            }
        }
    }

    /// Accepts one pending connection from an allowed peer and reads its request line. Refused peers are dropped
    /// without a read.
    pub fn accept(&self) -> Option<(UnixStream, Result<Request, String>)> {
        // a few refused connections must not hide an allowed one queued behind them
        for _ in 0..8 {
            let (stream, _) = self.listener.accept().ok()?;
            if !self.admit(&stream) {
                continue;
            }
            stream.set_nonblocking(false).ok()?;
            stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
            stream.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
            let mut line = String::new();
            BufReader::new((&stream).take(MAX_LINE)).read_line(&mut line).ok()?;
            return Some((stream, parse(&line)));
        }
        None
    }
}

pub fn reply(mut stream: UnixStream, body: &[u8]) {
    let _ = stream.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_screen_serves_status_and_input_only() {
        for ok in ["state", "version", "ready", "shot", "lock", "tap 10 10", "key power"] {
            assert!(allowed_while_locked(&parse(ok).unwrap()), "{ok} stays available while locked");
        }
        for no in ["open-app is.olibuijr.files.desktop", "open-url https://x.is", "ax press General", "ax list", "appearance light", "hover 1 1", "context 1 1"] {
            assert!(!allowed_while_locked(&parse(no).unwrap()), "{no} must wait for a login");
        }
    }

    #[test]
    fn version_and_ready_parse() {
        assert_eq!(parse("version"), Ok(Request::Version));
        assert_eq!(parse("ready"), Ok(Request::Ready));
        assert_eq!(parse("  state  "), Ok(Request::State));
        assert_eq!(parse("ax list"), Ok(Request::AxList(String::new())));
        assert_eq!(parse("ax list os/settings"), Ok(Request::AxList("os/settings".into())));
        assert_eq!(parse("ax press General > Software Update"), Ok(Request::AxPress("General > Software Update".into())));
        assert!(parse("ax press").is_err());
        assert_eq!(parse("ax actions os/windows/settings"), Ok(Request::AxActions("os/windows/settings".into())));
        assert_eq!(parse("ax action Settings :: Zoom"), Ok(Request::AxAction("Settings".into(), "Zoom".into())));
        assert!(parse("ax action Settings").is_err(), "the action name is required");
        assert!(parse("ax action :: Zoom").is_err(), "the element is required");
        assert!(parse("ready now").is_ok());
        assert_eq!(parse("lock"), Ok(Request::Lock));
        assert!(parse("lock now").is_err());
        assert_eq!(parse("overview"), Ok(Request::Overview));
        assert!(parse("overview now").is_err());
        assert!(!allowed_while_locked(&Request::Overview), "Overview waits for a login");
        assert_eq!(parse("appgrid"), Ok(Request::AppGrid));
        assert!(parse("appgrid now").is_err());
        assert!(!allowed_while_locked(&Request::AppGrid), "the Apps grid waits for a login");
        assert_eq!(parse("forcequit"), Ok(Request::ForceQuit));
        assert_eq!(parse("logout"), Ok(Request::LogOut));
        assert!(parse("forcequit now").is_err());
        assert!(parse("logout now").is_err());
        assert!(!allowed_while_locked(&Request::ForceQuit), "Force Quit waits for a login");
        assert!(!allowed_while_locked(&Request::LogOut), "Log Out waits for a login");
    }

    #[test]
    fn socket_path_defaults_and_overrides() {
        assert_eq!(SOCKET, "/run/bifrost/shell-control.sock");
        assert_eq!(socket_path_from(None), PathBuf::from(SOCKET));
        assert_eq!(socket_path_from(Some(" /tmp/emu1/control.sock\n")), PathBuf::from("/tmp/emu1/control.sock"));
        assert_eq!(socket_path_from(Some("relative.sock")), PathBuf::from(SOCKET), "only an absolute path overrides");
        assert_eq!(socket_path_from(Some("")), PathBuf::from(SOCKET));
        assert_eq!(socket_path_from(Some("/")), PathBuf::from(SOCKET));
    }

    #[test]
    fn only_root_the_shell_and_the_account_are_served() {
        let allowed = allowed_uids(0, Some(1000));
        assert_eq!(allowed, [0, 1000]);
        assert!(peer_allowed(0, &allowed), "root: lab tools over ssh and bifrost-mcp");
        assert!(peer_allowed(1000, &allowed), "the account the window server runs as");
        assert!(!peer_allowed(1001, &allowed), "another local user is refused");
        assert!(!peer_allowed(65534, &allowed), "nobody is refused");
        assert!(!peer_allowed(u32::MAX, &allowed), "an unreadable credential is refused");
        // no account yet: root only
        assert_eq!(allowed_uids(0, None), [0]);
        assert!(!peer_allowed(1000, &allowed_uids(0, None)));
        // a shell run as an ordinary user serves itself and root
        assert_eq!(allowed_uids(1234, None), [0, 1234]);
        assert_eq!(allowed_uids(1000, Some(1000)), [0, 1000], "no duplicates");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("control-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_socket_is_a_unix_socket_with_mode_0600() {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let dir = scratch("mode");
        let path = dir.join("run/shell-control.sock");
        let listener = bind_at(&path, None).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.file_type().is_socket());
        assert_eq!(meta.permissions().mode() & 0o7777, 0o600);
        // SAFETY: geteuid has no preconditions.
        assert_eq!(meta.uid(), unsafe { geteuid() });
        assert!(!dir.join("run/.shell-control.sock.stage").exists(), "the staging directory is removed");
        // binding again replaces a stale socket
        drop(listener);
        assert!(bind_at(&path, None).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_connection_from_an_allowed_uid_is_served_and_one_from_another_uid_is_refused() {
        // SAFETY: geteuid has no preconditions.
        let own = unsafe { geteuid() };
        for (allowed, served) in [(vec![own], true), (vec![own.wrapping_add(1)], false)] {
            let dir = scratch(if served { "served" } else { "refused" });
            let path = dir.join("c.sock");
            let listener = bind_at(&path, None).unwrap();
            listener.set_nonblocking(true).unwrap();
            let server = Server { listener, allowed, refused: Mutex::new(HashSet::new()) };
            let mut client = UnixStream::connect(&path).unwrap();
            client.write_all(b"version\n").unwrap();
            let got = server.accept();
            let mut answer = String::new();
            if served {
                let (stream, req) = got.expect("an allowed peer is served");
                assert_eq!(req, Ok(Request::Version));
                reply(stream, b"version=1\n");
                client.read_to_string(&mut answer).unwrap();
                assert_eq!(answer, "version=1\n");
            } else {
                assert!(got.is_none(), "a refused peer is dropped");
                // the server drops the connection without reading the request line, so the kernel ends it with a reset
                // (the request is still unread in the server's queue) or a plain close, depending on timing
                match client.read_to_string(&mut answer) {
                    Ok(_) => {}
                    Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::ConnectionReset, "a refused peer is cut off: {e}"),
                }
                assert!(answer.is_empty(), "a refused peer gets no answer");
                assert!(server.refused.lock().unwrap().contains(&own), "the refused uid is remembered for the one-time log");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn open_folder_takes_the_rest_of_the_line_as_the_path() {
        assert_eq!(
            parse("open-folder is.olibuijr.files.desktop /home/me/My Files\n"),
            Ok(Request::OpenFolder("is.olibuijr.files.desktop".into(), "/home/me/My Files".into()))
        );
        assert!(parse("open-folder is.olibuijr.files.desktop").is_err());
    }
    #[test]
    fn image_lines_take_the_mode_and_the_rest_of_the_path() {
        assert_eq!(
            parse("image-attach ro /home/me/My Disk.iso\n"),
            Ok(Request::ImageAttach { read_only: true, path: "/home/me/My Disk.iso".into() })
        );
        assert_eq!(parse("image-attach rw /home/me/a.img"), Ok(Request::ImageAttach { read_only: false, path: "/home/me/a.img".into() }));
        assert!(parse("image-attach maybe /home/me/a.img").is_err());
        assert!(parse("image-attach ro").is_err());
        assert_eq!(parse("image-detach /dev/loop3\n"), Ok(Request::ImageDetach("/dev/loop3".into())));
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse("shot 2\n"), Ok(Request::Shot(2)));
        assert_eq!(parse("shot"), Ok(Request::Shot(1)));
        assert_eq!(parse("capture window\n"), Ok(Request::Capture("window".into())));
        assert_eq!(parse("rec start"), Ok(Request::RecStart { area: None, file: None }));
        assert_eq!(parse("rec start screen"), Ok(Request::RecStart { area: None, file: None }));
        assert_eq!(parse("rec start 10,20,300,200 /tmp/a%20b.mov"), Ok(Request::RecStart { area: Some((10, 20, 300, 200)), file: Some("/tmp/a b.mov".into()) }));
        assert_eq!(parse("rec start screen clip.mov"), Ok(Request::RecStart { area: None, file: Some("clip.mov".into()) }));
        assert_eq!(parse("rec stop"), Ok(Request::RecStop));
        assert_eq!(parse("rec status"), Ok(Request::RecStatus));
        assert!(parse("rec start 1,2,0,4").is_err(), "an empty area is refused");
        assert!(parse("rec start 1,2,3").is_err());
        assert!(parse("rec pause").is_err());
        assert!(parse("capture movie\n").is_err(), "only select, window and screen");
        assert_eq!(parse("tap 10 20.5"), Ok(Request::Tap(10.0, 20.5)));
        assert_eq!(parse("swipe 1 2 3 4"), Ok(Request::Swipe { from: (1.0, 2.0), to: (3.0, 4.0), ms: 300.0 }));
        assert_eq!(parse("swipe 1 2 3 4 50"), Ok(Request::Swipe { from: (1.0, 2.0), to: (3.0, 4.0), ms: 50.0 }));
        assert_eq!(parse("key power"), Ok(Request::Key(Key::Power)));
        assert_eq!(parse("trace on"), Ok(Request::Trace(true)));
        assert_eq!(parse("perf"), Ok(Request::Perf));
        assert_eq!(parse("down 5 6"), Ok(Request::Touch(0, 5.0, 6.0)));
        assert_eq!(parse("move 7 8"), Ok(Request::Touch(1, 7.0, 8.0)));
        assert_eq!(parse("up 9 10"), Ok(Request::Touch(2, 9.0, 10.0)));
        assert_eq!(parse("hold 1 2"), Ok(Request::Swipe { from: (1.0, 2.0), to: (1.0, 2.0), ms: 700.0 }));
        assert_eq!(parse("longpress 1 2 1200"), Ok(Request::Swipe { from: (1.0, 2.0), to: (1.0, 2.0), ms: 1200.0 }));
        assert_eq!(parse("drag 1 2 3 4 90"), Ok(Request::Swipe { from: (1.0, 2.0), to: (3.0, 4.0), ms: 90.0 }));
        assert_eq!(parse("doubletap 3 4"), Ok(Request::DoubleTap(3.0, 4.0)));
        assert!(parse("down 5").is_err());
        assert!(parse("tap 1").is_err());
        assert!(parse("key nope").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn snapshot_averages_blocks() {
        let frame = vec![0xff00_0000, 0xff00_00ff, 0xff00_0000, 0xff00_00ff];
        let s = snapshot(&frame, 2, 2, 2);
        assert_eq!(&s[..4], b"RIMG");
        assert_eq!(u32::from_le_bytes(s[4..8].try_into().unwrap()), 1);
        assert_eq!(&s[12..16], &[128, 0, 0, 255]);
        assert_eq!(snapshot(&frame, 2, 2, 1).len(), 12 + 16);
    }

    #[test]
    fn double_tap_is_two_quick_taps() {
        let t = double_tap(3.0, 4.0);
        assert_eq!(t.iter().map(|e| e.1).collect::<Vec<_>>(), [0, 2, 0, 2]);
        assert!(t[2].0 - t[1].0 < 300.0 && t[2].0 > t[1].0);
    }

    #[test]
    fn path_runs_down_moves_up() {
        let p = path((0.0, 0.0), (100.0, 0.0), 64.0);
        assert_eq!(p.first().unwrap().1, 0);
        assert_eq!(p.last().unwrap().1, 2);
        assert_eq!(p.iter().filter(|e| e.1 == 1).count(), 4);
        assert_eq!(p[p.len() - 2].2, 100.0);
        let tap = path((5.0, 5.0), (5.0, 5.0), 0.0);
        assert_eq!(tap.len(), 3);
    }

    #[test]
    fn term_requests_carry_percent_encoded_values() {
        let line = format!("term send_text tab=2 text={}", pct_encode("echo 'hi there'\n100%"));
        assert_eq!(parse(&line), Ok(Request::Term("send_text".into(), vec![("tab".into(), "2".into()), ("text".into(), "echo 'hi there'\n100%".into())])));
        assert_eq!(parse("term state\n"), Ok(Request::Term("state".into(), vec![])));
        assert!(parse("term").is_err());
        assert_eq!(pct_decode("a%2"), "a%2");
        assert_eq!(pct_decode("%zz%41"), "%zzA");
    }

    #[test]
    fn notify_requests_keep_their_words() {
        assert_eq!(parse("notify post com.example.refresh\n"), Ok(Request::Notify(vec!["post".into(), "com.example.refresh".into()])));
        assert_eq!(parse("notify set a.b 42"), Ok(Request::Notify(vec!["set".into(), "a.b".into(), "42".into()])));
        assert_eq!(parse("notify"), Err("unknown command Some(\"notify\")".into()));
    }
}
