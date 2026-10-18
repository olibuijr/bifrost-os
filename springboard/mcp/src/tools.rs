//! Tool definitions and dispatch. Every tool answers in one round trip with compact text; images are
//! PNG, downscaled to `max_size` so they stay cheap in a model's context.

use crate::{keys, sys, touch, wl};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const INSTRUCTIONS: &str = "Drives a BifrOSt PC (Golden Gate-style Linux desktop) from inside the machine. \
Start with `observe` (windows, focus, shell state) and `screenshot`. Coordinates are screen pixels unless a \
tool says otherwise; screenshots report the factor between image pixels and screen pixels. Prefer semantic \
tools (`ui` press by name, `app` control-socket methods, `open`, `window`) over pixel clicks; use `batch` to \
send several steps in one call. `exec` runs bash as root on the machine.";

pub struct Ctx {
    wl: Option<wl::Wl>,
    kb: Option<keys::Keyboard>,
    touch: Option<touch::Touch>,
    /// Where each window's image sits on screen (window image px + offset = screen px), by window query.
    offsets: std::collections::HashMap<String, (i32, i32)>,
    dev: crate::dev::Dev,
}

pub(crate) fn text(s: impl Into<String>) -> Value {
    json!({"content":[{"type":"text","text":s.into()}]})
}

pub(crate) fn err(s: impl Into<String>) -> Value {
    json!({"content":[{"type":"text","text":s.into()}],"isError":true})
}

pub(crate) fn f(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
}

pub(crate) fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Ctx {
    pub fn new() -> Ctx {
        for (k, v) in sys::session_env() {
            if k == "XDG_RUNTIME_DIR" || k == "WAYLAND_DISPLAY" {
                std::env::set_var(k, v);
            }
        }
        Ctx { wl: None, kb: None, touch: None, offsets: Default::default(), dev: Default::default() }
    }

    /// The compositor connection, reconnecting after a shell or compositor restart.
    fn wl(&mut self) -> Result<&mut wl::Wl, String> {
        if let Some(w) = &mut self.wl {
            if w.refresh().is_err() {
                self.wl = None;
            }
        }
        if self.wl.is_none() {
            for (k, v) in sys::session_env() {
                if k == "XDG_RUNTIME_DIR" || k == "WAYLAND_DISPLAY" {
                    std::env::set_var(k, v);
                }
            }
            self.wl = Some(wl::Wl::connect()?);
        }
        Ok(self.wl.as_mut().unwrap())
    }

    fn kb(&mut self) -> Result<&mut keys::Keyboard, String> {
        if self.kb.is_none() {
            self.kb = Some(keys::Keyboard::open()?);
        }
        Ok(self.kb.as_mut().unwrap())
    }

    /// The uinput touchscreen, created on first use at the current screen size.
    fn touch(&mut self) -> Result<&mut touch::Touch, String> {
        if self.touch.is_none() {
            let (sw, sh) = self.wl()?.screen_size();
            self.touch = Some(touch::Touch::open((sw.max(0) as u32, sh.max(0) as u32))?);
        }
        Ok(self.touch.as_mut().unwrap())
    }

    pub fn call(&mut self, name: &str, a: &Value) -> Value {
        let a = if a.is_null() { &json!({}) } else { a };
        match self.dispatch(name, a) {
            Ok(mut v) => {
                crate::app::cap_content(&mut v, crate::app::OUT_CAP);
                v
            }
            Err(e) => err(crate::app::classify(&e)),
        }
    }

    fn dispatch(&mut self, name: &str, a: &Value) -> Result<Value, String> {
        match name {
            "observe" => self.observe(a),
            "screenshot" => self.screenshot(a),
            "click" => self.click(a),
            "touch" => self.touch_tool(a),
            "pointer" => self.pointer(a),
            "scroll" => self.scroll(a),
            "key" => self.key(a),
            "type" => self.type_text(a),
            "window" => self.window(a),
            "open" => self.open(a),
            "ui" => self.ui(a),
            "app" => self.app(a),
            "wait" => self.wait(a),
            "exec" => self.exec(a),
            "file" => file(a),
            "system" => system(a),
            "lifecycle" => crate::lifecycle::lifecycle(a),
            "browser" => crate::browser::run(self, a),
            "term" => crate::term::run(a),
            "files" => crate::files::run(self, a),
            "logs" => self.dev.logs(a),
            "crash" => crate::dev::crash(a),
            "dev" => crate::dev::dev(a),
            "perf" => crate::dev::perf(a),
            "shell" => Ok(text(sys::shell_text(s(a, "command").ok_or("command is required")?)?)),
            "batch" => self.batch(a),
            "wayland" => {
                let w = self.wl()?;
                let mut t = format!("toplevels wlr={} ext={}\n", w.st_counts().0, w.st_counts().1);
                for g in &w.st.globals {
                    t += &format!("{} v{}\n", g.1, g.2);
                }
                Ok(text(t))
            }
            n => Err(format!("unknown tool {n:?}")),
        }
    }

    fn observe(&mut self, a: &Value) -> Result<Value, String> {
        let mut out = String::new();
        match self.wl() {
            Ok(w) => {
                let (sw, sh) = w.screen_size();
                out += &format!("screen {sw}x{sh}\n");
                let wins = w.windows();
                out += &format!("windows ({}; * = focused, front first is not guaranteed):\n", wins.len());
                for x in &wins {
                    let mut flags = String::new();
                    for (on, tag) in [(x.maximized, " maximized"), (x.minimized, " minimized"), (x.fullscreen, " fullscreen")] {
                        if on {
                            flags += tag;
                        }
                    }
                    out += &format!("{} {:?} app_id={}{}\n", if x.activated { "*" } else { "-" }, x.title, x.app_id, flags);
                }
            }
            Err(e) => out += &format!("compositor: {e}\n"),
        }
        let socks = sys::app_sockets();
        if !socks.is_empty() {
            out += "apps with a control socket:\n";
            for (id, p) in &socks {
                out += &format!("  {}\n", crate::app::app_status_line(id, p));
            }
        }
        match sys::shell_text("state") {
            Ok(state) => {
                let all = a.get("all").and_then(Value::as_bool).unwrap_or(false);
                const KEEP: [&str; 14] = ["version", "ready", "screen", "menu", "quicksearch", "cc", "center", "banner", "notifications", "hud", "apps_running", "dock_visible", "volume", "window"];
                out += "shell:\n";
                for l in state.lines().flat_map(|l| l.split("window=").enumerate().map(|(i, p)| if i == 0 { p.to_string() } else { format!("window={p}") })) {
                    let key = l.split('=').next().unwrap_or("");
                    if !l.is_empty() && (all || KEEP.contains(&key)) {
                        out += &l;
                        out.push('\n');
                    }
                }
            }
            Err(e) => out += &format!("shell: {e}\n"),
        }
        Ok(text(out.trim_end()))
    }

    fn screenshot(&mut self, a: &Value) -> Result<Value, String> {
        let app_q = s(a, "app").filter(|_| s(a, "window").is_none());
        let target = s(a, "window").or(app_q);
        let max = f(a, "max_size").unwrap_or(1280.0).clamp(64.0, 4096.0) as u32;
        let region = a.get("region").and_then(Value::as_array).map(|r| {
            let n = |i: usize| r.get(i).and_then(Value::as_f64).unwrap_or(0.0) as i32;
            (n(0), n(1), n(2), n(3))
        });
        let cursor = a.get("cursor").and_then(Value::as_bool).unwrap_or(false);
        let (img, what, at) = match target {
            Some(q) => {
                if app_q.is_some() {
                    // an app id: bring it forward if minimized or covered, then retry until a frame is present
                    let w = self.wl()?;
                    let _ = w.window_action(q, "unminimize");
                    let _ = w.window_action(q, "activate");
                }
                let mut tries = if app_q.is_some() { 8 } else { 1 };
                let img = loop {
                    match self.wl()?.capture_window(q) {
                        Ok(i) => break i,
                        Err(e) if tries <= 1 => return Err(e),
                        Err(_) => {
                            tries -= 1;
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                };
                let hint: Vec<(i32, i32)> = self.offsets.get(&q.to_lowercase()).copied().into_iter().collect();
                let w = self.wl()?;
                let at = w.capture_screen(None, false).ok().and_then(|scr| locate(&img, &scr, &hint));
                (img, format!("window {q:?}"), at)
            }
            None => (self.wl()?.capture_screen(region, cursor)?, match region {
                Some((x, y, rw, rh)) => format!("screen region {rw}x{rh} at ({x},{y})"),
                None => "screen".into(),
            }, region.map(|r| (r.0, r.1))),
        };
        if let (Some(q), Some(o)) = (target, at) {
            self.offsets.insert(q.to_lowercase(), o);
        }
        let diff = a.get("diff").and_then(Value::as_bool).unwrap_or(false);
        let diff_note = self.dev.diff_note(&format!("{target:?}{region:?}"), &img, diff);
        let (ow, oh) = (img.w, img.h);
        let img = fit(img, max);
        let k = ow as f64 / img.w as f64;
        let mut note = format!("{what}: {}x{}", img.w, img.h);
        if (k - 1.0).abs() > 1e-6 {
            note += &format!(" (downscaled from {ow}x{oh}; image_scale={k:.4})");
        }
        note += &match (target, at) {
            (Some(q), Some((x, y))) => format!(". Window image origin is screen ({x},{y}): click with window={q:?} and image coordinates{}, or screen = image*{k:.4} + ({x},{y})", if (k - 1.0).abs() > 1e-6 { format!(" plus image_scale={k:.4}") } else { String::new() }),
            (Some(_), None) => ". Window is not visible on screen (covered or minimized); activate it before clicking".into(),
            (None, Some((x, y))) => format!(". screen = image*{k:.4} + ({x},{y})"),
            (None, None) if (k - 1.0).abs() > 1e-6 => format!(". Pass image_scale={k:.4} to click/pointer/scroll with image coordinates"),
            (None, None) => ". Image px = screen px".into(),
        };
        if let Some(d) = diff_note {
            note += &format!("\n{d}");
        }
        Ok(json!({"content":[{"type":"image","data":b64(&png(&img)),"mimeType":"image/png"},{"type":"text","text":note}]}))
    }

    /// x, y in screen pixels; `image_scale` multiplies them first (screenshot factor).
    fn point(&mut self, a: &Value) -> Result<(f64, f64), String> {
        let k = f(a, "image_scale").unwrap_or(1.0);
        let (x, y) = (f(a, "x").ok_or("x is required")? * k, f(a, "y").ok_or("y is required")? * k);
        let (ox, oy) = self.origin(a)?;
        Ok((x + ox, y + oy))
    }

    /// The screen offset of `window` (window image px -> screen px), (0,0) without one.
    fn origin(&mut self, a: &Value) -> Result<(f64, f64), String> {
        let Some(q) = s(a, "window") else { return Ok((0.0, 0.0)) };
        let key = q.to_lowercase();
        let w = self.wl()?;
        let win = w.capture_window(q)?;
        let hint: Vec<(i32, i32)> = self.offsets.get(&key).copied().into_iter().collect();
        if let Some(o) = self.wl()?.capture_screen(None, false).ok().and_then(|scr| locate(&win, &scr, &hint)) {
            self.offsets.insert(key, o);
            return Ok((o.0 as f64, o.1 as f64));
        }
        Err(format!("window {q:?} is not visible on screen; activate it first (window tool)"))
    }

    fn click(&mut self, a: &Value) -> Result<Value, String> {
        let (x, y) = self.point(a)?;
        let button = match s(a, "button").unwrap_or("left") {
            "left" => 0x110,
            "right" => 0x111,
            "middle" => 0x112,
            b => return Err(format!("unknown button {b:?}")),
        };
        let count = f(a, "count").unwrap_or(1.0).clamp(1.0, 3.0) as u32;
        let mods = s(a, "modifiers").map(keys::parse).transpose()?;
        if !self.wl()?.has_pointer() {
            if mods.is_some() {
                return Err("modifiers need a pointer; a touch screen has none".into());
            }
            let t = self.touch()?;
            return match button {
                0x110 => {
                    for i in 0..count {
                        t.tap(x, y, 60);
                        if i + 1 < count {
                            std::thread::sleep(Duration::from_millis(120));
                        }
                    }
                    Ok(text(format!("tapped at ({x:.0},{y:.0}){}", if count > 1 { format!(" x{count}") } else { String::new() })))
                }
                0x111 => {
                    t.tap(x, y, 700);
                    Ok(text(format!("long-pressed at ({x:.0},{y:.0})")))
                }
                _ => Err("no middle button on a touch screen".into()),
            };
        }
        if mods.is_some() {
            self.kb()?;
        }
        let w = self.wl()?;
        w.motion(x, y)?;
        std::thread::sleep(Duration::from_millis(16));
        let _ = mods; // modifiers are held through the uinput keyboard below
        for i in 0..count {
            w.button(button, true)?;
            std::thread::sleep(Duration::from_millis(50));
            w.button(button, false)?;
            if i + 1 < count {
                std::thread::sleep(Duration::from_millis(60));
            }
        }
        w.sync()?;
        Ok(text(format!("clicked {} at ({x:.0},{y:.0}){}", s(a, "button").unwrap_or("left"), if count > 1 { format!(" x{count}") } else { String::new() })))
    }

    /// `touch`: a finger on the uinput touchscreen. Same coordinates as click and pointer.
    fn touch_tool(&mut self, a: &Value) -> Result<Value, String> {
        let (x, y) = self.point(a)?;
        let k = f(a, "image_scale").unwrap_or(1.0);
        let (ox, oy) = (x - f(a, "x").unwrap_or(0.0) * k, y - f(a, "y").unwrap_or(0.0) * k);
        match s(a, "action").unwrap_or("tap") {
            "tap" => {
                self.touch()?.tap(x, y, 60);
                Ok(text(format!("tapped at ({x:.0},{y:.0})")))
            }
            "hold" => {
                let ms = f(a, "ms").unwrap_or(700.0).clamp(0.0, 10_000.0);
                self.touch()?.tap(x, y, ms as u64);
                Ok(text(format!("long-pressed at ({x:.0},{y:.0}) for {ms:.0} ms")))
            }
            "swipe" => {
                let (Some(tx), Some(ty)) = (f(a, "to_x"), f(a, "to_y")) else {
                    return Err("swipe needs to_x and to_y".into());
                };
                let (tx, ty) = (tx * k + ox, ty * k + oy);
                let ms = f(a, "ms").unwrap_or(300.0).clamp(0.0, 10_000.0);
                self.touch()?.swipe(x, y, tx, ty, ms);
                Ok(text(format!("swiped ({x:.0},{y:.0}) -> ({tx:.0},{ty:.0}) in {ms:.0} ms")))
            }
            other => Err(format!("unknown touch action {other:?} (tap, hold, swipe)")),
        }
    }

    fn pointer(&mut self, a: &Value) -> Result<Value, String> {
        let (x, y) = self.point(a)?;
        let k = f(a, "image_scale").unwrap_or(1.0);
        let (ox, oy) = (x - f(a, "x").unwrap_or(0.0) * k, y - f(a, "y").unwrap_or(0.0) * k);
        let to = match (f(a, "to_x"), f(a, "to_y")) {
            (Some(tx), Some(ty)) => Some((tx * k + ox, ty * k + oy)),
            _ => None,
        };
        let ms = f(a, "ms").unwrap_or(300.0).clamp(0.0, 10_000.0);
        if !self.wl()?.has_pointer() {
            let Some((tx, ty)) = to else {
                return Err("a touch screen has no hover; use click or touch".into());
            };
            self.touch()?.swipe(x, y, tx, ty, ms);
            return Ok(text(format!("dragged ({x:.0},{y:.0}) -> ({tx:.0},{ty:.0}) in {ms:.0} ms")));
        }
        let w = self.wl()?;
        w.motion(x, y)?;
        let Some((tx, ty)) = to else {
            w.sync()?;
            return Ok(text(format!("pointer at ({x:.0},{y:.0})")));
        };
        w.button(0x110, true)?;
        let steps = ((ms / 16.0) as u32).max(2);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            std::thread::sleep(Duration::from_millis((ms / steps as f64) as u64));
            w.motion(x + (tx - x) * t, y + (ty - y) * t)?;
        }
        w.button(0x110, false)?;
        w.sync()?;
        Ok(text(format!("dragged ({x:.0},{y:.0}) -> ({tx:.0},{ty:.0}) in {ms:.0} ms")))
    }

    fn scroll(&mut self, a: &Value) -> Result<Value, String> {
        let dy = f(a, "dy").unwrap_or(0.0);
        let dx = f(a, "dx").unwrap_or(0.0);
        let at = if a.get("x").is_some() { Some(self.point(a)?) } else { None };
        if !self.wl()?.has_pointer() {
            // a finger drag: dy>0 scrolls down, so the finger moves up; 80 px per wheel click
            let (sw, sh) = self.wl()?.screen_size();
            let (sw, sh) = (sw as f64, sh as f64);
            let (x, y) = at.unwrap_or((sw / 2.0, sh / 2.0));
            let (x, y) = (x.clamp(0.0, sw - 1.0), y.clamp(0.0, sh - 1.0));
            let (tx, ty) = ((x - dx * 80.0).clamp(0.0, sw - 1.0), (y - dy * 80.0).clamp(0.0, sh - 1.0));
            self.touch()?.swipe(x, y, tx, ty, 250.0);
            return Ok(text(format!("scrolled dx={dx} dy={dy}")));
        }
        let w = self.wl()?;
        if let Some((x, y)) = at {
            w.motion(x, y)?;
        }
        w.scroll(dx, dy)?;
        w.sync()?;
        Ok(text(format!("scrolled dx={dx} dy={dy}")))
    }

    fn key(&mut self, a: &Value) -> Result<Value, String> {
        let list: Vec<String> = match a.get("keys") {
            Some(Value::Array(v)) => v.iter().filter_map(|k| k.as_str().map(String::from)).collect(),
            Some(Value::String(k)) => k.split_whitespace().map(String::from).collect(),
            _ => return Err("keys is required, for example \"cmd+space\" or [\"down\",\"enter\"]".into()),
        };
        let repeat = f(a, "repeat").unwrap_or(1.0).clamp(1.0, 100.0) as usize;
        // a lone symbol (\"+\", \"*\", \"(\") is text: type it, whatever the layout
        if list.iter().all(|k| k.chars().count() == 1 && keys::parse(k).is_err()) || list.iter().any(|k| k == "+") && list.iter().all(|k| k.chars().count() == 1) {
            let t: String = list.concat().repeat(repeat);
            self.wl()?.type_text(&t, 0)?;
            return Ok(text(format!("typed {t:?}")));
        }
        for k in &list {
            keys::parse(k)?;
        }
        let kb = self.kb()?;
        for _ in 0..repeat {
            for k in &list {
                kb.chord(k, 20)?;
            }
        }
        Ok(text(format!("pressed {}{}", list.join(" "), if repeat > 1 { format!(" x{repeat}") } else { String::new() })))
    }

    fn type_text(&mut self, a: &Value) -> Result<Value, String> {
        let t = s(a, "text").ok_or("text is required")?;
        let delay = f(a, "delay_ms").unwrap_or(0.0).clamp(0.0, 500.0) as u64;
        // ASCII runs go through the kernel keyboard (what the shell's own fields read); the rest through
        // the virtual keyboard's generated keymap (any Unicode, for Wayland text clients)
        let mut run = String::new();
        let mut ascii = true;
        for c in t.chars().chain(std::iter::once('\0')) {
            let is_ascii = keys::ascii_key(c).is_some();
            if (c == '\0' || is_ascii != ascii) && !run.is_empty() {
                if ascii { self.kb()?.type_ascii(&run, delay)? } else { self.wl()?.type_text(&run, delay)? }
                run.clear();
            }
            ascii = is_ascii;
            if c != '\0' {
                run.push(c);
            }
        }
        if a.get("submit").and_then(Value::as_bool).unwrap_or(false) {
            self.kb()?.chord("enter", 20)?;
        }
        Ok(text(format!("typed {} chars", t.chars().count())))
    }

    fn window(&mut self, a: &Value) -> Result<Value, String> {
        let action = s(a, "action").unwrap_or("activate");
        if action == "list" {
            let q = s(a, "window").or(s(a, "app")).map(str::to_lowercase);
            let wins = self.wl()?.windows();
            let rows: Vec<String> = wins
                .iter()
                .filter(|w| q.as_ref().is_none_or(|q| w.app_id.to_lowercase().contains(q) || w.title.to_lowercase().contains(q)))
                .map(|w| format!("{} {:?} app_id={} maximized={} minimized={} fullscreen={}", if w.activated { "*" } else { "-" }, w.title, w.app_id, w.maximized, w.minimized, w.fullscreen))
                .collect();
            return Ok(text(if rows.is_empty() { "no windows".into() } else { rows.join("\n") }));
        }
        if action == "resize" || action == "move" {
            return Err(crate::app::coded("E_UNSUPPORTED", "wlr-foreign-toplevel has no resize/move request (bifrost-wm answers only activate, close and the maximize, minimize and fullscreen states); use maximize/unmaximize/fullscreen, or drag the title bar with `pointer`"));
        }
        let q = s(a, "window").or(s(a, "app")).ok_or("window is required (title or app id, substring ok)")?;
        let w = self.wl()?.window_action(q, action)?;
        Ok(text(format!("{action}: {:?} app_id={} focused={}", w.title, w.app_id, w.activated)))
    }

    fn open(&mut self, a: &Value) -> Result<Value, String> {
        let line = if let Some(id) = s(a, "app") {
            // a bundle id goes straight to the shell; a name ("calculator", "Voice Memos") resolves through
            // the shell's app map (os/apps/*), which also covers system apps
            if !id.contains('.') || sys::shell_text(&format!("open-app {id}")).is_err() {
                let rows = sys::shell_text("ax list os/apps/")?;
                let q = id.rsplit('.').find(|p| *p != "desktop").unwrap_or(id).to_lowercase();
                let names: Vec<(&str, &str)> = rows.lines().filter_map(|l| {
                    let mut c = l.split('\t');
                    Some((c.next()?, c.nth(1)?))
                }).collect();
                let hit = names.iter().find(|(addr, n)| n.to_lowercase() == q || addr.ends_with(&format!("/{}", q.replace(' ', "-"))))
                    .or_else(|| names.iter().find(|(_, n)| n.to_lowercase().contains(&q)))
                    .ok_or_else(|| format!("no app named {id:?}; installed: {}", names.iter().map(|n| n.1).collect::<Vec<_>>().join(", ")))?;
                format!("ax press {}", hit.0)
            } else {
                format!("open-app {id}")
            }
        } else if let Some(u) = s(a, "url") {
            format!("open-url {u}")
        } else if let Some(p) = s(a, "folder") {
            format!("open-folder is.olibuijr.files.desktop {p}")
        } else {
            return Err("give one of app (name or bundle id), url or folder".into());
        };
        let r = sys::shell_text(&line)?;
        if let Some(id) = s(a, "app") {
            if a.get("wait").and_then(Value::as_bool).unwrap_or(true) {
                let t = Instant::now();
                let short = id.rsplit('.').find(|p| *p != "desktop").unwrap_or(id).to_lowercase();
                let before: Vec<String> = self.wl()?.windows().iter().map(|w| w.app_id.clone()).collect();
                while t.elapsed() < Duration::from_secs(8) {
                    let w = self.wl()?.windows();
                    // the app's window by name, or any window that was not there before (Settings for bifrost.settings)
                    let hit = w.iter().find(|x| x.app_id.to_lowercase().contains(&short) || x.title.to_lowercase().contains(&short))
                        .or_else(|| w.iter().find(|x| !before.contains(&x.app_id)));
                    if let Some(x) = hit {
                        return Ok(text(format!("opened {id}: window {:?} app_id={} after {} ms", x.title, x.app_id, t.elapsed().as_millis())));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                return Ok(text(format!("{} (no new window within 8 s; it may already be open: check observe)", r.trim())));
            }
        }
        Ok(text(r.trim().to_string()))
    }

    fn ui(&mut self, a: &Value) -> Result<Value, String> {
        match s(a, "press") {
            Some(q) => Ok(text(sys::shell_text(&format!("ax press {q}"))?.trim().to_string())),
            None => {
                let rows = sys::shell_text(&format!("ax list {}", s(a, "prefix").unwrap_or("")))?;
                let filter = s(a, "filter").map(str::to_lowercase);
                let rows: Vec<&str> = rows.lines().filter(|l| filter.as_ref().is_none_or(|q| l.to_lowercase().contains(q))).collect();
                let limit = f(a, "limit").unwrap_or(200.0) as usize;
                let more = rows.len().saturating_sub(limit);
                let mut out = rows.into_iter().take(limit).collect::<Vec<_>>().join("\n");
                if more > 0 {
                    out += &format!("\n... {more} more (use filter or prefix)");
                }
                Ok(text(if out.is_empty() { "no elements".into() } else { out }))
            }
        }
    }

    fn app(&mut self, a: &Value) -> Result<Value, String> {
        crate::app::app_tool(a)
    }

    /// `wait` on pixels: `screen_changed` returns when a capture differs from the first one, `screen_stable` when
    /// the capture has not changed for `stable_ms`. Compares hashes of cheap captures (region or window optional).
    fn wait_screen(&mut self, a: &Value, changed: bool, timeout: Duration) -> Result<Value, String> {
        let region = a.get("region").and_then(Value::as_array).map(|r| {
            let n = |i: usize| r.get(i).and_then(Value::as_f64).unwrap_or(0.0) as i32;
            (n(0), n(1), n(2), n(3))
        });
        let stable = Duration::from_millis(f(a, "stable_ms").unwrap_or(300.0).clamp(50.0, 10_000.0) as u64);
        let t = Instant::now();
        let grab = |me: &mut Ctx| -> Result<u64, String> {
            let img = match s(a, "window") {
                Some(q) => me.wl()?.capture_window(q)?,
                None => me.wl()?.capture_screen(region, false)?,
            };
            Ok(crate::dev::hash_image(&img))
        };
        let mut last = grab(self)?;
        let mut since = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(40));
            let h = grab(self)?;
            if h != last {
                if changed {
                    return Ok(text(format!("screen changed after {} ms", t.elapsed().as_millis())));
                }
                last = h;
                since = Instant::now();
            } else if !changed && since.elapsed() >= stable {
                return Ok(text(format!("screen stable for {} ms (waited {} ms)", stable.as_millis(), t.elapsed().as_millis())));
            }
            if t.elapsed() >= timeout {
                return Err(format!("timed out after {} ms waiting for the screen to {}", t.elapsed().as_millis(), if changed { "change" } else { "settle" }));
            }
        }
    }

    fn wait(&mut self, a: &Value) -> Result<Value, String> {
        if crate::app::WAIT_KEYS.iter().any(|k| s(a, k).is_some()) {
            return crate::app::wait_ext(a);
        }
        let timeout = Duration::from_secs_f64(f(a, "timeout_s").unwrap_or(5.0).clamp(0.05, 60.0));
        for (k, changed) in [("screen_changed", true), ("screen_stable", false)] {
            if a.get(k).and_then(Value::as_bool).unwrap_or(false) {
                return self.wait_screen(a, changed, timeout);
            }
        }
        let gone = a.get("gone").and_then(Value::as_bool).unwrap_or(false);
        let t = Instant::now();
        loop {
            let (hit, what) = if let Some(q) = s(a, "window") {
                let ql = q.to_lowercase();
                let w = self.wl()?.windows();
                (w.iter().any(|x| x.title.to_lowercase().contains(&ql) || x.app_id.to_lowercase().contains(&ql)), format!("window {q:?}"))
            } else if let Some(q) = s(a, "element") {
                let rows = sys::shell_text("ax list").unwrap_or_default().to_lowercase();
                (rows.contains(&q.to_lowercase()), format!("element {q:?}"))
            } else if let Some(kv) = s(a, "state") {
                let st = sys::shell_text("state").unwrap_or_default();
                (st.lines().flat_map(|l| l.split("window=")).any(|l| l.trim() == kv || l.starts_with(kv)), format!("state {kv:?}"))
            } else if let Some(cmd) = s(a, "command") {
                let r = sys::exec(cmd, false, None, Duration::from_secs(5), None)?;
                (r.code == Some(0), format!("command {cmd:?}"))
            } else {
                std::thread::sleep(timeout);
                return Ok(text(format!("waited {} ms", timeout.as_millis())));
            };
            if hit != gone {
                return Ok(text(format!("{what} {} after {} ms", if gone { "gone" } else { "present" }, t.elapsed().as_millis())));
            }
            if t.elapsed() >= timeout {
                return Err(format!("timed out after {} ms waiting for {what}{}", t.elapsed().as_millis(), if gone { " to go" } else { "" }));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn exec(&mut self, a: &Value) -> Result<Value, String> {
        let cmd = s(a, "command").ok_or("command is required")?;
        let timeout = Duration::from_secs_f64(f(a, "timeout_s").unwrap_or(30.0).clamp(0.1, 600.0));
        let as_user = a.get("as_user").and_then(Value::as_bool).unwrap_or(false);
        let r = sys::exec(cmd, as_user, s(a, "cwd"), timeout, s(a, "stdin"))?;
        let max = f(a, "max_output").unwrap_or(20_000.0) as usize;
        let mut out = String::new();
        out += &clip(&r.out, max);
        if !r.err.is_empty() {
            out += &format!("\n[stderr]\n{}", clip(&r.err, max / 2));
        }
        out += &format!("\n[exit {}{} in {} ms]", r.code.map(|c| c.to_string()).unwrap_or("signal".into()), if r.timed_out { ", timed out" } else { "" }, r.ms);
        Ok(if r.code == Some(0) { text(out.trim_start()) } else { err(out.trim_start()) })
    }

    fn batch(&mut self, a: &Value) -> Result<Value, String> {
        let steps = a.get("steps").and_then(Value::as_array).ok_or("steps is required: [{\"tool\":\"click\",\"args\":{...}}, ...]")?;
        let stop = a.get("stop_on_error").and_then(Value::as_bool).unwrap_or(true);
        let mut content = Vec::new();
        let mut failed = false;
        for (i, st) in steps.iter().enumerate() {
            let tool = st["tool"].as_str().unwrap_or("");
            if tool == "batch" {
                return Err("batch cannot nest".into());
            }
            let t = Instant::now();
            let r = self.call(tool, &st["args"]);
            let bad = r["isError"] == true;
            for c in r["content"].as_array().into_iter().flatten() {
                let mut c = c.clone();
                if let Some(t2) = c["text"].as_str() {
                    c["text"] = json!(format!("{}. {tool} ({} ms): {t2}", i + 1, t.elapsed().as_millis()));
                }
                content.push(c);
            }
            if let Some(ms) = st.get("then_wait_ms").and_then(Value::as_u64) {
                std::thread::sleep(Duration::from_millis(ms.min(10_000)));
            }
            if bad {
                failed = true;
                if stop {
                    content.push(json!({"type":"text","text":format!("stopped at step {} of {}", i + 1, steps.len())}));
                    break;
                }
            }
        }
        Ok(json!({"content":content,"isError":failed}))
    }
}

fn file(a: &Value) -> Result<Value, String> {
    let path = s(a, "path").ok_or("path is required")?;
    match s(a, "action").unwrap_or("read") {
        "read" => {
            let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let text_ = String::from_utf8_lossy(&data);
            let off = f(a, "offset").unwrap_or(1.0).max(1.0) as usize - 1;
            let lim = f(a, "limit").unwrap_or(400.0) as usize;
            let lines: Vec<&str> = text_.lines().collect();
            let body: Vec<String> = lines.iter().enumerate().skip(off).take(lim).map(|(i, l)| format!("{:>5}\t{l}", i + 1)).collect();
            let more = lines.len().saturating_sub(off + lim);
            Ok(text(format!("{}{}", body.join("\n"), if more > 0 { format!("\n... {more} more lines") } else { String::new() })))
        }
        "write" | "append" => {
            let content = a.get("content").and_then(Value::as_str).ok_or("content is required")?;
            if let Some(dir) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            use std::io::Write;
            let mut fh = std::fs::OpenOptions::new().create(true).write(true).append(s(a, "action") == Some("append")).truncate(s(a, "action") != Some("append")).open(path).map_err(|e| format!("{path}: {e}"))?;
            fh.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
            Ok(text(format!("wrote {} bytes to {path}", content.len())))
        }
        "list" => {
            let mut rows: Vec<String> = std::fs::read_dir(path).map_err(|e| format!("{path}: {e}"))?.flatten().map(|e| {
                let m = e.metadata().ok();
                let dir = m.as_ref().is_some_and(|m| m.is_dir());
                format!("{}{}\t{}", e.file_name().to_string_lossy(), if dir { "/" } else { "" }, m.map(|m| m.len()).unwrap_or(0))
            }).collect();
            rows.sort();
            Ok(text(rows.join("\n")))
        }
        "stat" => {
            let m = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
            use std::os::unix::fs::MetadataExt;
            Ok(text(format!("size={} mode={:o} uid={} gid={} mtime={} dir={}", m.len(), m.mode(), m.uid(), m.gid(), m.mtime(), m.is_dir())))
        }
        x => Err(format!("unknown file action {x:?}")),
    }
}

fn system(a: &Value) -> Result<Value, String> {
    match s(a, "what").unwrap_or("overview") {
        "overview" => {
            let load = sys::read("/proc/loadavg");
            let up = sys::read("/proc/uptime");
            let mem = sys::read("/proc/meminfo");
            let mi = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) / 1024;
            let ver = sys::read("/usr/share/springboard/version").trim().to_string();
            let mut t = format!("host={} kernel={} uptime={}s load={}\nmem total={}M available={}M\n",
                sys::read("/proc/sys/kernel/hostname").trim(), sys::read("/proc/sys/kernel/osrelease").trim(),
                up.split('.').next().unwrap_or("?"), load.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
                mi("MemTotal"), mi("MemAvailable"));
            if !ver.is_empty() {
                t += &format!("springboard={ver}\n");
            }
            if let Ok(v) = sys::shell_text("version") {
                t += v.trim();
            }
            Ok(text(t))
        }
        "processes" => {
            let mut p = sys::processes();
            let filter = s(a, "filter").map(str::to_lowercase);
            p.retain(|x| filter.as_ref().is_none_or(|q| x.3.to_lowercase().contains(q)));
            p.sort_by_key(|x| std::cmp::Reverse(x.1));
            let lim = f(a, "limit").unwrap_or(40.0) as usize;
            Ok(text(p.iter().take(lim).map(|(pid, rss, ticks, cmd)| format!("{pid}\t{}M\t{}s\t{}", rss / 1024, ticks / 100, clip(cmd, 160))).collect::<Vec<_>>().join("\n")))
        }
        "journal" => {
            let n = f(a, "lines").unwrap_or(60.0) as u32;
            let unit = s(a, "unit").map(|u| format!(" -u {u}")).unwrap_or_default();
            let grep = s(a, "filter").map(|g| format!(" --grep {}", shq(g))).unwrap_or_default();
            let r = sys::exec(&format!("journalctl -b --no-pager -o short-monotonic -n {n}{unit}{grep}"), false, None, Duration::from_secs(10), None)?;
            Ok(text(r.out))
        }
        w => Err(format!("unknown what {w:?}: overview | processes | journal")),
    }
}

pub(crate) fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}... [{} more bytes]", &s[..cut], s.len() - cut)
}


/// Where `win` (a window's own image) appears in `scr`: the offset to add to window pixels,
/// found by exact matches of opaque, textured 16x16 blocks (translucent edges and shadows differ).
pub fn locate(win: &wl::Image, scr: &wl::Image, hints: &[(i32, i32)]) -> Option<(i32, i32)> {
    const B: u32 = 16;
    if win.w < 4 * B || win.h < 4 * B {
        return None;
    }
    let px = |img: &wl::Image, x: u32, y: u32| -> u32 {
        let i = ((y * img.w + x) * 4) as usize;
        u32::from_le_bytes([img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]])
    };
    // textured blocks on a 7x7 grid inside the window (flat ones would match anywhere)
    let mut blocks = Vec::new();
    for gy in 1..8u32 {
        for gx in 1..8u32 {
            let (bx, by) = (win.w * gx / 8 - B / 2, win.h * gy / 8 - B / 2);
            let mut d: Vec<u32> = (0..B * B).map(|i| px(win, bx + i % B, by + i / B)).collect();
            d.sort_unstable();
            d.dedup();
            if d.len() >= 12 {
                blocks.push((bx, by));
            }
        }
    }
    let block_at = |(bx, by): (u32, u32), (ox, oy): (i32, i32)| -> bool {
        let (sx, sy) = (bx as i32 + ox, by as i32 + oy);
        sx >= 0 && sy >= 0 && sx as u32 + B <= scr.w && sy as u32 + B <= scr.h
            && (0..B * B).all(|i| px(scr, sx as u32 + i % B, sy as u32 + i / B) == px(win, bx + i % B, by + i / B))
    };
    // an offset counts when four textured blocks agree (or all of them, if fewer)
    let confirmed = |o: (i32, i32)| blocks.iter().filter(|&&b| block_at(b, o)).take(4).count() >= blocks.len().min(4);
    if blocks.is_empty() {
        return None;
    }
    let centred = (((scr.w as i32) - win.w as i32) / 2, ((scr.h as i32) - win.h as i32) / 2);
    for &h in hints.iter().chain([&centred]) {
        if confirmed(h) {
            return Some(h);
        }
    }
    for &(bx, by) in &blocks {
        let first = px(win, bx, by);
        for sy in 0..scr.h.saturating_sub(B) {
            for sx in 0..scr.w.saturating_sub(B) {
                if px(scr, sx, sy) == first {
                    let o = (sx as i32 - bx as i32, sy as i32 - by as i32);
                    if block_at((bx, by), o) && confirmed(o) {
                        return Some(o);
                    }
                }
            }
        }
    }
    None
}

/// Box-filter downscale so the longer side is at most `max`.
fn fit(img: wl::Image, max: u32) -> wl::Image {
    let long = img.w.max(img.h);
    if long <= max {
        return img;
    }
    let (nw, nh) = ((img.w as u64 * max as u64 / long as u64).max(1) as u32, (img.h as u64 * max as u64 / long as u64).max(1) as u32);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let (y0, y1) = (y * img.h / nh, ((y + 1) * img.h / nh).max(y * img.h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * img.w / nw, ((x + 1) * img.w / nw).max(x * img.w / nw + 1));
            let mut acc = [0u32; 4];
            for sy in y0..y1 {
                let row = (sy * img.w) as usize * 4;
                for sx in x0..x1 {
                    let p = row + sx as usize * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += img.rgba[p + c] as u32;
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    wl::Image { w: nw, h: nh, rgba: out }
}

fn png(img: &wl::Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, img.w, img.h);
    e.set_color(png::ColorType::Rgba);
    e.set_compression(png::Compression::Fast);
    let mut w = e.write_header().expect("png header");
    w.write_image_data(&img.rgba).expect("png data");
    drop(w);
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { B64[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

pub fn b64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for b in s.bytes() {
        let Some(v) = B64.iter().position(|&c| c == b) else { continue };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The tool list (tools/list). Descriptions are written for the model: what it does, when to use it.
pub fn list() -> Value {
    let o = |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    let mut tools = json!([
        {"name":"observe","description":"Snapshot of the desktop as text: screen size, open windows (title, app_id, focus, state) and the shell state (menu, Search, Control Center, running apps). Also lists each app control socket with pids, RSS/PSS and cgroup memory. Cheap; call it first and after actions instead of a screenshot when text is enough.",
         "inputSchema":o(json!({"all":{"type":"boolean","description":"every shell state key, not only the useful ones"}}),&[])},
        {"name":"screenshot","description":"PNG of the whole screen, a screen region, one window by name (window capture works even when covered), or app=<id> (resolves the app's window, unminimizes and activates it, retries until a frame is present). Returns the factor from image to screen pixels; pass it as image_scale to click/pointer/scroll, or multiply yourself. diff=true also reports the bounding box that changed since the previous capture of the same target (or unchanged), to verify an action had an effect.",
         "inputSchema":o(json!({"window":{"type":"string","description":"window title or app id (substring ok); omit for the screen"},"app":{"type":"string","description":"app id (e.g. browser): window of that app, brought forward if covered"},"region":{"type":"array","items":{"type":"number"},"description":"[x,y,w,h] screen pixels"},"max_size":{"type":"number","description":"longest side in pixels, default 1280"},"cursor":{"type":"boolean"},"diff":{"type":"boolean","description":"report the changed bounding box vs the previous capture of this target"}}),&[])},
        {"name":"click","description":"Move the pointer to (x,y) and click. Screen pixels by default; with window=NAME, pixels of that window's screenshot; with image_scale (from a downscaled screenshot), image pixels.",
         "inputSchema":o(json!({"x":{"type":"number"},"y":{"type":"number"},"window":{"type":"string","description":"coordinates are relative to this window's screenshot"},"image_scale":{"type":"number"},"button":{"type":"string","enum":["left","right","middle"]},"count":{"type":"integer","description":"2 = double click"}}),&["x","y"])},
        {"name":"touch","description":"Touch the screen like a finger: action tap (default), hold (long press, ms default 700) or swipe to (to_x,to_y) over ms (default 300). Same coordinates as click: screen pixels, or window=NAME / image_scale.",
         "inputSchema":o(json!({"action":{"type":"string","enum":["tap","hold","swipe"]},"x":{"type":"number"},"y":{"type":"number"},"to_x":{"type":"number"},"to_y":{"type":"number"},"ms":{"type":"number"},"window":{"type":"string"},"image_scale":{"type":"number"}}),&["x","y"])},
        {"name":"pointer","description":"Move the pointer (hover) to (x,y), or drag with the left button from (x,y) to (to_x,to_y) over ms milliseconds.",
         "inputSchema":o(json!({"x":{"type":"number"},"y":{"type":"number"},"to_x":{"type":"number"},"to_y":{"type":"number"},"ms":{"type":"number"},"image_scale":{"type":"number"},"window":{"type":"string"}}),&["x","y"])},
        {"name":"scroll","description":"Scroll with the wheel, optionally at (x,y) first. dy>0 scrolls down, in wheel clicks.",
         "inputSchema":o(json!({"dy":{"type":"number"},"dx":{"type":"number"},"x":{"type":"number"},"y":{"type":"number"},"image_scale":{"type":"number"},"window":{"type":"string"}}),&[])},
        {"name":"key","description":"Press keys or chords on a real (kernel) keyboard: \"enter\", \"cmd+space\" (Search), \"cmd+q\", \"cmd+shift+s\", [\"down\",\"down\",\"enter\"]. Names: cmd ctrl shift alt, enter esc tab space backspace delete home end pageup pagedown up down left right f1-f12, letters, digits, - = [ ] ; ' ` \\ , . /, volumeup volumedown mute. US physical keys; use type for text.",
         "inputSchema":o(json!({"keys":{"description":"one chord string, space-separated chords, or an array","anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]},"repeat":{"type":"integer"}}),&["keys"])},
        {"name":"type","description":"Type text into the focused field, including the shell's own fields (Search). ASCII is typed as US keys; other characters (Icelandic, emoji) through a virtual keyboard, which Wayland apps accept. submit=true presses Enter after.",
         "inputSchema":o(json!({"text":{"type":"string"},"submit":{"type":"boolean"},"delay_ms":{"type":"number","description":"per character, default 0"}}),&["text"])},
        {"name":"window","description":"Act on a window by title or app id (substring ok): activate (focus and raise), close, maximize, unmaximize, minimize, unminimize, fullscreen, unfullscreen. action=list shows windows (optionally filtered by window/app id: title, app_id, state). resize and move are not available (the foreign-toplevel protocol has no such request) and answer E_UNSUPPORTED.",
         "inputSchema":o(json!({"window":{"type":"string"},"app":{"type":"string","description":"app id, same as window"},"action":{"type":"string","enum":["activate","close","maximize","unmaximize","minimize","unminimize","fullscreen","unfullscreen","list","resize","move"]}}),&[])},
        {"name":"open","description":"Launch an app by name (\"Calculator\", \"Voice Memos\") or bundle id and wait for its window, open a URL in the browser, or open a folder in Files.",
         "inputSchema":o(json!({"app":{"type":"string"},"url":{"type":"string"},"folder":{"type":"string"},"wait":{"type":"boolean","description":"wait for the app window, default true"}}),&[])},
        {"name":"ui","description":"The shell's accessibility map: list named elements (menu bar, dock, Settings rows, buttons) with their address and frame, or press one by address or name. Faster and steadier than clicking pixels. SDK apps appear under prefix app/<id> (their controls tree: buttons, fields, lists, rows, with hierarchy), e.g. prefix=app/ with the app open; press works by address or unique name.",
         "inputSchema":o(json!({"press":{"type":"string","description":"address or name to press"},"filter":{"type":"string","description":"substring filter on list rows"},"prefix":{"type":"string"},"limit":{"type":"integer"}}),&[])},
        {"name":"app","description":"Apps' JSON-RPC control sockets. No app: list apps with a socket. app only: its methods (app.methods). app+method(+params): call it, e.g. notes app.state, intent.<id>. Reads and edits app data without touching the screen. describe=true lists methods with parameter signatures (app.methods verbose; method filters by substring). A result shaped {encoding:\"base64\",format:\"png\",data,...} comes back as a real image block; file=<path> saves any result (decoded bytes for images) and is the way to get results larger than max_output (default 100000 bytes). Errors carry codes: E_NO_APP, E_NO_METHOD, E_BAD_PARAMS, E_TIMEOUT, E_NO_TAB, E_ENGINE_DOWN, E_APP_DOWN.",
         "inputSchema":o(json!({"app":{"type":"string","description":"app id or substring, e.g. notes"},"method":{"type":"string"},"params":{"type":"object"},"timeout_s":{"type":"number","description":"default 10, max 120"},"describe":{"type":"boolean"},"file":{"type":"string","description":"save the result to this path"},"max_output":{"type":"integer"}}),&[])},
        {"name":"wait","description":"Wait until a window appears (or is gone), an accessibility element appears, a shell state line matches (key=value), or a command exits 0. screen_changed=true returns when the pixels (optionally region or window) differ from now; screen_stable=true when they have not changed for stable_ms (default 300), e.g. after an animation or a restart. Also: app_ready=<id> (control socket answers), app_method=<id> + method (+params, path as a.b[0].c, and equals | contains | matches (* glob); truthy if none) polls a method until the value matches, log_line=<text> (a NEW journal line containing it; optional unit, priority, subsystem), process_exit=<pid or command substring>, app_exit=<id>. Timeouts (timeout_s, default 5 for screen/window waits, 10 for the app waits, max 300 there) report the last observed value. Use instead of sleeping.",
         "inputSchema":o(json!({"app_ready":{"type":"string"},"app_method":{"type":"string"},"method":{"type":"string"},"params":{"type":"object"},"path":{"type":"string"},"equals":{},"contains":{"type":"string"},"matches":{"type":"string"},"log_line":{"type":"string"},"unit":{"type":"string"},"priority":{"type":"string"},"subsystem":{"type":"string"},"process_exit":{"type":"string"},"app_exit":{"type":"string"},"window":{"type":"string"},"element":{"type":"string"},"state":{"type":"string","description":"e.g. quicksearch=visible"},"command":{"type":"string"},"gone":{"type":"boolean"},"screen_changed":{"type":"boolean"},"screen_stable":{"type":"boolean"},"stable_ms":{"type":"number"},"region":{"type":"array","items":{"type":"number"},"description":"[x,y,w,h] for screen_changed/screen_stable"},"timeout_s":{"type":"number","description":"default 5"}}),&[])},
        {"name":"exec","description":"Run a bash command on the machine as root (as_user=true: as the desktop account inside the Wayland session, for GUI tools like grim or wl-copy). Returns stdout, stderr and the exit code.",
         "inputSchema":o(json!({"command":{"type":"string"},"as_user":{"type":"boolean"},"cwd":{"type":"string"},"stdin":{"type":"string"},"timeout_s":{"type":"number","description":"default 30"},"max_output":{"type":"integer"}}),&["command"])},
        {"name":"file","description":"Files on the machine: read (numbered lines, offset/limit), write, append, list a directory, stat.",
         "inputSchema":o(json!({"path":{"type":"string"},"action":{"type":"string","enum":["read","write","append","list","stat"]},"content":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}),&["path"])},
        {"name":"system","description":"System facts without a shell: overview (host, kernel, uptime, load, memory, BifrOSt version), processes (pid, resident memory, CPU time, command; sorted by memory, biggest first; filter), journal (system log of this boot; unit, filter, lines).",
         "inputSchema":o(json!({"what":{"type":"string","enum":["overview","processes","journal"]},"filter":{"type":"string"},"unit":{"type":"string"},"lines":{"type":"integer"},"limit":{"type":"integer"}}),&[])},
        {"name":"lifecycle","description":"Machine lifecycle of the LAB machine (Midget). status (lab flag, uptime, version, boot entries, pending unattended preset). restart (reboot in 3 s). netboot (BootNext = PXE, reboot into the lab ISO; needs confirm=\"NETBOOT <hostname>\"). reinstall (writes an unattended preset to the ESP, BootNext = PXE, reboot; the lab installer erases the system disk and installs a fresh system with the `tester` account, MCP enabled; needs confirm=\"REINSTALL <hostname>\"). netboot and reinstall refuse on any machine that is not the lab hardware. All return a job id at once; the machine goes down 3 s later, so poll by reconnecting.",
         "inputSchema":o(json!({"action":{"type":"string","enum":["status","restart","netboot","reinstall"]},"confirm":{"type":"string","description":"exact phrase: NETBOOT <hostname> or REINSTALL <hostname>"},"password_url":{"type":"string"},"mirror":{"type":"string","description":"lab package mirror (default http://10.99.0.1:8078/mirror/)"},"locale":{"type":"string"},"keyboard":{"type":"string"}}),&["action"])},
        {"name":"logs","description":"Journal lines that are new since your previous logs call with the same filters (a cursor is kept per session and filter set; the first call returns the last `lines`). Filter by unit, priority (err, warning, 0..3), text (filter), or app=<id> (SDK apps log to the springboard unit through the shell's stderr, so this is unit springboard filtered on lines that mention the id, plus the app's Unified Logging subsystem when it is the app id, shown in a second section), or subsystem/category (exact Unified Logging fields from `Logger`, any unit; prefer these for apps that use `Logger`). follow_ms waits up to that long for new lines. reset=true restarts from the tail. Use after each action to see what it logged, instead of reading the whole journal.",
         "inputSchema":o(json!({"unit":{"type":"string"},"app":{"type":"string"},"subsystem":{"type":"string"},"category":{"type":"string"},"priority":{"type":"string"},"filter":{"type":"string"},"lines":{"type":"integer","description":"first read only, default 40"},"follow_ms":{"type":"number","description":"wait up to this long for new lines, max 30000"},"reset":{"type":"boolean"},"session":{"type":"string","description":"separate cursor set, default shared"},"after_cursor":{"type":"string"},"max_output":{"type":"integer"}}),&[])},
        {"name":"crash","description":"Crashes on this boot: what=list (coredumpctl entries since boot or `since`, e.g. -1h, plus the shell's own crash.log tail), info (pid or exe: signal, identity and a trimmed backtrace; frames caps the stack), panic (the shell's panic log crash.log and last_exit). app=<id> gives one app's picture: its processes and control socket, coredumps and panic-log lines that name it (engine/content children included by name), the shell's last exit and recent warnings in the journal. Use when the desktop vanished, an app disappeared or the shell restarted.",
         "inputSchema":o(json!({"what":{"type":"string","enum":["list","info","panic"]},"app":{"type":"string"},"since":{"type":"string"},"pid":{"type":"string"},"exe":{"type":"string"},"frames":{"type":"integer"},"lines":{"type":"integer"},"max_output":{"type":"integer"}}),&[])},
        {"name":"dev","description":"Dev loop on this machine. upload (path, content_b64, append=true for further chunks of a large file, mode e.g. 755; returns size and sha256 of the whole file), install_package (path of a .aap on the machine), install_store_app (id), restart (target shell | compositor | app:<id> | mcp; waits up to wait_s for the shell and reports the version, or relaunches the app and waits for its control socket). compositor restarts with the shell session. Restarting the shell locks the screen: key enter unlocks.",
         "inputSchema":o(json!({"action":{"type":"string","enum":["upload","install_package","install_store_app","restart"]},"path":{"type":"string"},"content_b64":{"type":"string"},"append":{"type":"boolean"},"mode":{"type":"string"},"id":{"type":"string"},"target":{"type":"string"},"wait_s":{"type":"number","description":"default 30"}}),&["action"])},
        {"name":"perf","description":"Performance snapshot: the shell's own perf line, machine CPU busy, and CPU% per process (100 = one core) sampled over ms (default 1000) from /proc, busiest first (shell, bifrost-wm, apps). filter narrows by command, limit caps rows. app=<id> restricts to that app's process family (the app and every descendant, engine and content processes included) and adds their summed RSS and PSS.",
         "inputSchema":o(json!({"app":{"type":"string"},"ms":{"type":"number"},"filter":{"type":"string"},"limit":{"type":"integer"}}),&[])},
        {"name":"shell","description":"Raw line on the shell's control port (docs/agent-api.md): state, version, perf, appearance light|dark|auto, trace on|off, ax list, open-app ID. For anything the other tools lack.",
         "inputSchema":o(json!({"command":{"type":"string"}}),&["command"])},
        {"name":"batch","description":"Run several tool calls in order in one round trip, e.g. open, wait, click, type, screenshot. Each step is {tool, args, then_wait_ms?}. Stops at the first error unless stop_on_error=false.",
         "inputSchema":o(json!({"steps":{"type":"array","items":{"type":"object","properties":{"tool":{"type":"string"},"args":{"type":"object"},"then_wait_ms":{"type":"integer"}},"required":["tool"]}},"stop_on_error":{"type":"boolean"}}),&["steps"])}
    ]);
    tools.as_array_mut().unwrap().push(crate::browser::schema());
    tools.as_array_mut().unwrap().push(crate::term::schema());
    tools.as_array_mut().unwrap().push(crate::files::schema());
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for n in 0..10 {
            let d: Vec<u8> = (0..n).map(|i| (i * 37) as u8).collect();
            assert_eq!(b64_decode(&b64(&d)), d);
        }
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
    }

    #[test]
    fn fit_keeps_the_aspect() {
        let img = wl::Image { w: 2560, h: 1600, rgba: vec![255; 2560 * 1600 * 4] };
        let s = fit(img, 1280);
        assert_eq!((s.w, s.h), (1280, 800));
        assert!(s.rgba.iter().all(|&b| b == 255));
    }

    #[test]
    fn locate_finds_a_window_in_the_screen() {
        let pat = |x: u32, y: u32| [(x * 7 + y * 13) as u8, (x * x + y) as u8, (y * 3) as u8, 255];
        let (ww, wh) = (200u32, 150u32);
        let win = wl::Image { w: ww, h: wh, rgba: (0..ww * wh).flat_map(|i| pat(i % ww, i / ww)).collect() };
        let (sw, sh, ox, oy) = (800u32, 600u32, 333u32, 211u32);
        let mut scr = wl::Image { w: sw, h: sh, rgba: vec![40; (sw * sh * 4) as usize] };
        for y in 0..wh {
            for x in 0..ww {
                let i = (((y + oy) * sw + x + ox) * 4) as usize;
                scr.rgba[i..i + 4].copy_from_slice(&pat(x, y));
            }
        }
        assert_eq!(locate(&win, &scr, &[]), Some((333, 211)));
        assert_eq!(locate(&win, &scr, &[(333, 211)]), Some((333, 211)));
        scr.rgba.fill(40);
        assert_eq!(locate(&win, &scr, &[]), None);
    }

    #[test]
    fn every_tool_has_a_schema() {
        for t in list().as_array().unwrap() {
            assert!(t["description"].as_str().unwrap().len() > 40);
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }
}
