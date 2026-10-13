//! Remote control socket (127.0.0.1 only; reach it through `adb forward`).
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

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub const PORT: u16 = 7777;

/// The port to listen on: `SPRINGBOARD_CONTROL_PORT` when set (several emulators on one host), else `PORT`.
pub fn port() -> u16 {
    port_from(std::env::var("SPRINGBOARD_CONTROL_PORT").ok().as_deref())
}

fn port_from(v: Option<&str>) -> u16 {
    v.and_then(|s| s.trim().parse().ok()).unwrap_or(PORT)
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
    Version,
    Ready,
    Perf,
    Trace(bool),
    QuickSearch,
    Lock,
    /// `term <verb> key=value ...` (values percent-encoded): Term agent control, see `shell/terminal/control.rs`.
    Term(String, Vec<(String, String)>),
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

pub fn parse(line: &str) -> Result<Request, String> {
    if let Some(rest) = line.trim().strip_prefix("term ") {
        let mut w = rest.split_whitespace();
        let verb = w.next().ok_or("usage: term <verb> key=value ...")?.to_string();
        let args = w.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), pct_decode(v))).collect();
        return Ok(Request::Term(verb, args));
    }
    // the folder may contain spaces, so it takes the rest of the line
    if let Some(rest) = line.trim_end().strip_prefix("open-folder ") {
        let (id, folder) = rest.split_once(' ').ok_or("Expected an app ID and a folder")?;
        return Ok(Request::OpenFolder(id.into(), folder.into()));
    }
    let w: Vec<&str> = line.split_whitespace().collect();
    let num = |i: usize| -> Result<f32, String> { w.get(i).ok_or(format!("missing argument {i}"))?.parse::<f32>().map_err(|e| e.to_string()).and_then(|v| if v.is_finite() { Ok(v) } else { Err("not a finite number".to_string()) }) };
    match w.first().copied() {
        Some("shot") => Ok(Request::Shot(w.get(1).map(|s| s.parse().unwrap_or(1)).unwrap_or(1).clamp(1, 8))),
        Some("open-app") if w.len()==2=>Ok(Request::OpenApp(w[1].into())),
        Some("appearance") if matches!(w.get(1).copied(),Some("light"|"dark"|"auto"))&&w.len()==2=>Ok(Request::Appearance(w[1].into())),
        Some("context")=>Ok(Request::Context(num(1)?,num(2)?)),
        Some("hover")=>Ok(Request::Hover(num(1)?,num(2)?)),
        Some("open-url")=>{let url=w.get(1).ok_or("Missing URL")?;if w.len()!=2||!(url.starts_with("http://")||url.starts_with("https://")){return Err("Expected HTTP or HTTPS URL".into());}Ok(Request::OpenUrl((*url).into()))},
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
            _ => Err("usage: ax list [prefix] | ax press <address or name>".into()),
        },
        Some("state") => Ok(Request::State),
        Some("version") => Ok(Request::Version),
        Some("ready") => Ok(Request::Ready),
        Some("perf") => Ok(Request::Perf),
        Some("trace") => Ok(Request::Trace(w.get(1) == Some(&"on"))),
        Some("quicksearch") => Ok(Request::QuickSearch),
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
    listener: TcpListener,
}

impl Server {
    pub fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port()))?;
        listener.set_nonblocking(true)?;
        Ok(Server { listener })
    }

    pub fn fd(&self) -> std::ffi::c_int {
        use std::os::fd::AsRawFd;
        self.listener.as_raw_fd()
    }

    /// Accepts one pending connection and reads its request line.
    pub fn accept(&self) -> Option<(TcpStream, Result<Request, String>)> {
        let (stream, _) = self.listener.accept().ok()?;
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
        let mut line = String::new();
        BufReader::new((&stream).take(MAX_LINE)).read_line(&mut line).ok()?;
        Some((stream, parse(&line)))
    }
}

pub fn reply(mut stream: TcpStream, body: &[u8]) {
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
        assert!(parse("ready now").is_ok());
        assert_eq!(parse("lock"), Ok(Request::Lock));
        assert!(parse("lock now").is_err());
    }

    #[test]
    fn port_defaults_and_overrides() {
        assert_eq!(port_from(None), 7777);
        assert_eq!(port_from(Some("7801")), 7801);
        assert_eq!(port_from(Some(" 7802\n")), 7802);
        assert_eq!(port_from(Some("nope")), 7777);
        assert_eq!(port_from(Some("99999")), 7777);
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
    fn parses_commands() {
        assert_eq!(parse("shot 2\n"), Ok(Request::Shot(2)));
        assert_eq!(parse("shot"), Ok(Request::Shot(1)));
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
}
