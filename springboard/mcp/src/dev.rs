//! Dev and debug tools for the Midget loop: `logs` (journal with cursors), `crash` (coredumps and the shell's
//! crash.log), `dev` (upload, install, restart), `perf` (CPU per process) and the screen-change helpers.
//! Pure helpers are unit tested; the tools shell out to journalctl, coredumpctl, systemctl and sha256sum.

use crate::sys;
use crate::tools::{b64_decode, clip, err, f, s, shq, text};
use crate::wl::Image;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const CLK_TCK: f64 = 100.0;

#[derive(Default)]
pub struct Dev {
    /// Journal cursor per (session, filters): the next `logs` call returns only what came after.
    cursors: HashMap<String, String>,
    /// The previous capture per target, for `screenshot diff=true`.
    shots: HashMap<String, Image>,
}

fn run(cmd: &str, secs: u64) -> Result<sys::Ran, String> {
    sys::exec(cmd, false, None, Duration::from_secs(secs), None)
}

// ---------------------------------------------------------------- logs

/// A safe journalctl priority: a name or digit, or a range such as `0..3`.
pub fn valid_priority(p: &str) -> bool {
    !p.is_empty() && p.len() <= 12 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// Splits journalctl `--show-cursor` output into the log lines and the cursor of the last one.
pub fn split_cursor(out: &str) -> (String, Option<String>) {
    let mut cursor = None;
    let mut lines = Vec::new();
    for l in out.lines() {
        match l.strip_prefix("-- cursor: ") {
            Some(c) => cursor = Some(c.trim().to_string()),
            None => lines.push(l),
        }
    }
    (lines.join("\n"), cursor)
}

pub struct LogQuery<'a> {
    pub unit: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub grep: Option<&'a str>,
    /// Unified Logging fields (`BIFROST_SUBSYSTEM`, `BIFROST_CATEGORY`), matched exactly by journald
    pub subsystem: Option<&'a str>,
    pub category: Option<&'a str>,
    pub cursor: Option<&'a str>,
    pub lines: u32,
}

pub fn journal_cmd(q: &LogQuery) -> String {
    let mut c = String::from("journalctl -b --no-pager -o short-monotonic --show-cursor");
    if let Some(u) = q.unit {
        c += &format!(" -u {}", shq(u));
    }
    if let Some(p) = q.priority {
        c += &format!(" -p {}", shq(p));
    }
    for (field, v) in [("BIFROST_SUBSYSTEM", q.subsystem), ("BIFROST_CATEGORY", q.category)] {
        if let Some(v) = v {
            c += &format!(" {}", shq(&format!("{field}={v}")));
        }
    }
    if let Some(g) = q.grep {
        c += &format!(" --case-sensitive=false --grep {}", shq(g));
    }
    match q.cursor {
        Some(cur) => c += &format!(" --after-cursor={}", shq(cur)),
        None => c += &format!(" -n {}", q.lines),
    }
    c
}

impl Dev {
    pub fn logs(&mut self, a: &Value) -> Result<Value, String> {
        // SDK apps are children of the shell (through bwrap) and inherit its stdout/stderr, so their output is in
        // the springboard unit; lines carry no app id, so `app` filters by the id appearing in the line.
        let resolved = s(a, "app").map(crate::app::resolve_id);
        let app = resolved.as_deref();
        // Unified Logging lines of the app's own subsystem come as a second section.
        if let (Some(id), None) = (app, s(a, "subsystem")) {
            if a.get("_sub").is_none() {
                let mut sub = a.clone();
                if let Some(o) = sub.as_object_mut() {
                    o.remove("app");
                    o.remove("filter");
                    o.remove("unit");
                    o.insert("subsystem".into(), json!(id));
                    o.insert("follow_ms".into(), json!(0));
                    o.insert("_sub".into(), json!(true));
                }
                let sub_text = self.logs(&sub).ok().and_then(|v| v["content"][0]["text"].as_str().map(String::from)).unwrap_or_default();
                let mut main = a.clone();
                main["_sub"] = json!(true);
                main["app"] = json!(id);
                main["subsystem"] = json!("");
                let mut r = self.logs(&main)?;
                let body = r["content"][0]["text"].as_str().unwrap_or("").to_string();
                if !sub_text.trim().is_empty() && sub_text != "no new lines" {
                    r["content"][0]["text"] = json!(format!("{body}\n--- Unified Logging subsystem {id} ---\n{sub_text}"));
                }
                return Ok(r);
            }
        }
        let unit = s(a, "unit").or(app.map(|_| "springboard"));
        let grep = s(a, "filter").or(app);
        if let Some(p) = s(a, "priority") {
            if !valid_priority(p) {
                return Err(format!("bad priority {p:?}: emerg..debug, 0-7 or a range like 0..3"));
            }
        }
        let key = format!("{}|{}|{}|{}|{}|{}", s(a, "session").unwrap_or("default"), unit.unwrap_or(""), s(a, "priority").unwrap_or(""), grep.unwrap_or(""), s(a, "subsystem").unwrap_or(""), s(a, "category").unwrap_or(""));
        if a.get("reset").and_then(Value::as_bool).unwrap_or(false) {
            self.cursors.remove(&key);
        }
        let explicit = s(a, "after_cursor").map(str::to_string);
        let follow = Duration::from_millis(f(a, "follow_ms").unwrap_or(0.0).clamp(0.0, 30_000.0) as u64);
        let lines = f(a, "lines").unwrap_or(40.0).clamp(1.0, 2000.0) as u32;
        let t = Instant::now();
        loop {
            let cur = explicit.clone().or_else(|| self.cursors.get(&key).cloned());
            let first = cur.is_none();
            let cmd = journal_cmd(&LogQuery { unit, priority: s(a, "priority"), grep, subsystem: s(a, "subsystem"), category: s(a, "category"), cursor: cur.as_deref(), lines });
            let r = run(&cmd, 10)?;
            if r.code != Some(0) && !r.err.is_empty() {
                return Err(r.err.trim().to_string());
            }
            let (body, new_cursor) = split_cursor(&r.out);
            if let Some(c) = &new_cursor {
                if self.cursors.len() > 256 {
                    self.cursors.clear(); // GC: stale per-session cursor sets must not pile up
                }
                self.cursors.insert(key.clone(), c.clone());
            }
            if !body.trim().is_empty() || t.elapsed() >= follow {
                let max = f(a, "max_output").unwrap_or(20_000.0) as usize;
                let head = if first { "(first read for this filter: last lines)\n" } else { "" };
                let tail = new_cursor.map(|c| format!("\n[cursor {c}]")).unwrap_or_default();
                return Ok(text(if body.trim().is_empty() { "no new lines".to_string() } else { format!("{head}{}{tail}", clip(&body, max)) }));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

// ---------------------------------------------------------------- crash

/// `--since` value for "this boot" from /proc/uptime seconds.
pub fn boot_since(uptime: &str) -> String {
    let secs = uptime.split_whitespace().next().and_then(|v| v.split('.').next()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(3600);
    format!("-{}s", secs + 5)
}

/// A coredumpctl info block reduced to its identity header and the first `frames` stack lines.
pub fn trim_info(info: &str, frames: usize) -> String {
    let mut out = Vec::new();
    let (mut in_stack, mut n) = (false, 0);
    for l in info.lines() {
        if l.trim().is_empty() {
            continue;
        }
        if l.contains("Stack trace of thread") {
            in_stack = true;
            n = 0;
        }
        if in_stack {
            if n < frames {
                out.push(l.to_string());
            } else if n == frames {
                out.push("        ...".into());
            }
            n += 1;
        } else if !["Storage:", "Cgroup:", "Slice:", "Owner UID:"].iter().any(|k| l.trim_start().starts_with(k)) {
            out.push(l.to_string());
        }
    }
    out.join("\n")
}

fn data_dir() -> String {
    std::env::var("SPRINGBOARD_DATA").unwrap_or_else(|_| "/var/lib/springboard".into())
}

/// The shell's own crash record: panics written by its hook (`crash.log`) and the last controlled exit.
fn panic_log(lines: usize) -> String {
    let dir = data_dir();
    let log = sys::read(&format!("{dir}/crash.log"));
    let mut tail: Vec<&str> = log.lines().rev().take(lines).collect();
    tail.reverse();
    let mut out = if tail.is_empty() { format!("{dir}/crash.log: empty or missing (no panics recorded)") } else { format!("{dir}/crash.log (last {}):\n{}", tail.len(), tail.join("\n")) };
    let last = sys::read(&format!("{dir}/last_exit"));
    if !last.trim().is_empty() {
        out += &format!("\nlast_exit: {}", last.trim());
    }
    out
}

pub fn crash(a: &Value) -> Result<Value, String> {
    if let Some(app) = s(a, "app") {
        return crate::app::app_crash(app, f(a, "frames").unwrap_or(25.0) as usize);
    }
    match s(a, "what").unwrap_or("list") {
        "list" => {
            let since = s(a, "since").map(str::to_string).unwrap_or_else(|| boot_since(&sys::read("/proc/uptime")));
            let r = run(&format!("coredumpctl list --no-pager --no-legend --since {}", shq(&since)), 10)?;
            let body = r.out.trim();
            let mut t = if body.is_empty() { format!("no coredumps since {since}") } else { format!("coredumps since {since} (TIME PID UID GID SIG COREFILE EXE):\n{body}") };
            t += &format!("\n{}", panic_log(3));
            Ok(text(t))
        }
        "info" => {
            let m = s(a, "pid").or(s(a, "exe")).ok_or("pid or exe is required (a PID or an executable name/path; what=list shows them)")?;
            if !m.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
                return Err(format!("bad match {m:?}"));
            }
            let r = run(&format!("coredumpctl info --no-pager -- {m}"), 20)?;
            if r.out.trim().is_empty() {
                return Err(if r.err.trim().is_empty() { format!("no coredump matches {m}") } else { r.err.trim().to_string() });
            }
            Ok(text(clip(&trim_info(&r.out, f(a, "frames").unwrap_or(25.0) as usize), f(a, "max_output").unwrap_or(8000.0) as usize)))
        }
        "panic" => Ok(text(panic_log(f(a, "lines").unwrap_or(20.0) as usize))),
        w => Err(format!("unknown what {w:?}: list | info | panic")),
    }
}

// ---------------------------------------------------------------- dev

/// An octal file mode such as `755` or `0644`.
pub fn parse_mode(m: &str) -> Option<u32> {
    if m.is_empty() || m.len() > 4 || !m.chars().all(|c| ('0'..='7').contains(&c)) {
        return None;
    }
    u32::from_str_radix(m, 8).ok()
}

fn upload(a: &Value) -> Result<Value, String> {
    use std::io::Write;
    let path = s(a, "path").ok_or("path is required")?;
    let data = b64_decode(s(a, "content_b64").ok_or("content_b64 is required (base64)")?);
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let append = a.get("append").and_then(Value::as_bool).unwrap_or(false);
    use std::os::unix::fs::OpenOptionsExt;
    let mut fh = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path).map_err(|e| format!("{path}: {e}"))?;
    fh.write_all(&data).map_err(|e| e.to_string())?;
    drop(fh);
    if let Some(m) = s(a, "mode") {
        use std::os::unix::fs::PermissionsExt;
        let mode = parse_mode(m).ok_or_else(|| format!("bad mode {m:?}: octal like 755"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // the hash covers the whole file, so after the last chunk it is the value to compare with the host's
    let sum = run(&format!("sha256sum {}", shq(path)), 60)?;
    let sha = sum.out.split_whitespace().next().unwrap_or("?").to_string();
    Ok(text(format!("{} {} bytes to {path}; size now {size}; sha256 {sha}", if append { "appended" } else { "wrote" }, data.len())))
}

fn restart_shell(wait: Duration) -> Result<String, String> {
    let r = run("systemctl restart springboard", 60)?;
    if r.code != Some(0) {
        return Err(format!("systemctl restart springboard: {}", r.err.trim()));
    }
    let t = Instant::now();
    loop {
        if let Ok(v) = sys::shell_text("version") {
            if !v.trim().is_empty() {
                return Ok(format!("shell back after {} ms: {}", t.elapsed().as_millis(), v.trim()));
            }
        }
        if t.elapsed() >= wait {
            return Err(format!("shell control port not answering after {} ms; try logs unit=springboard and crash", t.elapsed().as_millis()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn restart_app(id: &str, wait: Duration) -> Result<String, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)) {
        return Err(format!("bad app id {id:?}"));
    }
    // only processes run from this app's install directory, never a substring of some other command
    let dir = format!("/apps/{id}/");
    let me = std::process::id();
    let pids: Vec<u32> = sys::processes().into_iter().filter(|p| p.0 != me && p.3.contains(&dir)).map(|p| p.0).collect();
    for p in &pids {
        unsafe { libc::kill(*p as i32, libc::SIGTERM) };
    }
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) && sys::processes().iter().any(|p| pids.contains(&p.0)) {
        std::thread::sleep(Duration::from_millis(100));
    }
    sys::shell_text(&format!("open-app {id}"))?;
    let t2 = Instant::now();
    loop {
        let sock = sys::app_sockets().into_iter().find(|(i, _)| i == id).map(|x| x.1);
        if let Some(p) = sock {
            if sys::app_rpc(&p, "app.info", &json!({}), Duration::from_millis(500)).is_ok() {
                return Ok(format!("{id}: signalled {} process(es), relaunched; control socket answers after {} ms", pids.len(), t2.elapsed().as_millis()));
            }
        }
        if t2.elapsed() >= wait {
            return Ok(format!("{id}: signalled {} process(es), open-app sent; no control socket answer within {} ms (the app may not expose one)", pids.len(), t2.elapsed().as_millis()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn dev(a: &Value) -> Result<Value, String> {
    let wait = Duration::from_secs_f64(f(a, "wait_s").unwrap_or(30.0).clamp(1.0, 120.0));
    match s(a, "action").unwrap_or("") {
        "upload" => upload(a),
        "install_package" => {
            let p = s(a, "path").ok_or("path of the .aap on the machine is required (dev upload it first)")?;
            let r = run(&format!("/usr/bin/springboard --install-package {}", shq(p)), 120)?;
            let o = format!("{}{}", r.out.trim(), if r.err.is_empty() { String::new() } else { format!("\n{}", r.err.trim()) });
            Ok(if r.code == Some(0) { text(format!("{o}\n(open launches it; a running copy needs dev restart app:<id>)")) } else { err(o) })
        }
        "install_store_app" => {
            let id = s(a, "id").ok_or("id (App Store app id) is required")?;
            let r = run(&format!("/usr/bin/springboard --install-store-app {}", shq(id)), 300)?;
            let o = format!("{}{}", r.out.trim(), if r.err.is_empty() { String::new() } else { format!("\n{}", r.err.trim()) });
            Ok(if r.code == Some(0) { text(o) } else { err(o) })
        }
        "restart" => {
            let target = s(a, "target").ok_or("target is required: shell | compositor | app:<id> | mcp")?;
            match target {
                // bifrost-wm is started with the shell's session, so both come back with the springboard unit
                "shell" | "compositor" => restart_shell(wait).map(text),
                "mcp" => {
                    run("systemd-run --on-active=1 --unit=bifrost-mcp-restart systemctl restart bifrost-mcp", 10)?;
                    Ok(text("bifrost-mcp restarts in 1 s; the session reconnects"))
                }
                t if t.starts_with("app:") && t.len() > 4 => restart_app(&t[4..], wait).map(text),
                t => Err(format!("unknown target {t:?}: shell | compositor | app:<id> | mcp")),
            }
        }
        x => Err(format!("unknown action {x:?}: upload | install_package | install_store_app | restart")),
    }
}

// ---------------------------------------------------------------- perf

/// Percent of one core per pid between two `(pid, ticks)` samples taken `ms` apart; idle pids are dropped.
pub fn cpu_deltas(before: &[(u32, u64)], after: &[(u32, u64)], ms: u64) -> Vec<(u32, f64)> {
    let b: HashMap<u32, u64> = before.iter().copied().collect();
    let mut v: Vec<(u32, f64)> = after
        .iter()
        .filter_map(|(pid, t)| {
            let d = t.checked_sub(*b.get(pid)?)?;
            (d > 0).then(|| (*pid, d as f64 / CLK_TCK / (ms.max(1) as f64 / 1000.0) * 100.0))
        })
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

/// (busy, total) jiffies from the first line of /proc/stat.
fn busy(stat: &str) -> (u64, u64) {
    let n: Vec<u64> = stat.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let all: u64 = n.iter().sum();
    (all.saturating_sub(n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0)), all)
}

pub fn perf(a: &Value) -> Result<Value, String> {
    let ms = f(a, "ms").unwrap_or(1000.0).clamp(100.0, 10_000.0) as u64;
    let family: Option<Vec<u32>> = s(a, "app").map(|q| crate::app::app_family(&crate::app::resolve_id(q)));
    let before: Vec<(u32, u64)> = sys::processes().into_iter().map(|p| (p.0, p.2)).collect();
    let stat0 = sys::read("/proc/stat");
    std::thread::sleep(Duration::from_millis(ms));
    let procs = sys::processes();
    let after: Vec<(u32, u64)> = procs.iter().map(|p| (p.0, p.2)).collect();
    let stat1 = sys::read("/proc/stat");
    let mut out = String::new();
    match sys::shell_text("perf") {
        Ok(p) => out += &format!("shell perf: {}\n", p.trim()),
        Err(e) => out += &format!("shell perf: {e}\n"),
    }
    let ((b0, t0), (b1, t1)) = (busy(&stat0), busy(&stat1));
    if t1 > t0 {
        out += &format!("machine cpu {:.0}% busy over {ms} ms\n", b1.saturating_sub(b0) as f64 / (t1 - t0) as f64 * 100.0);
    }
    let name = |pid: u32| procs.iter().find(|p| p.0 == pid).map(|p| clip(&p.3, 100)).unwrap_or_default();
    let filter = s(a, "filter").map(str::to_lowercase);
    let rows: Vec<String> = cpu_deltas(&before, &after, ms)
        .into_iter()
        .filter(|(pid, _)| family.as_ref().is_none_or(|fam| fam.contains(pid)))
        .filter(|(pid, _)| filter.as_ref().is_none_or(|q| name(*pid).to_lowercase().contains(q)))
        .take(f(a, "limit").unwrap_or(10.0) as usize)
        .map(|(pid, pct)| format!("{pct:>5.1}%  {pid}  {}", name(pid)))
        .collect();
    if let Some(fam) = &family {
        let (rss, pss) = crate::app::mem_of(fam);
        out += &format!("app family: {} processes, RSS {} MiB, PSS {} MiB\n", fam.len(), rss / 1024, pss / 1024);
    }
    out += &if rows.is_empty() { "no process used CPU in the window".to_string() } else { format!("CPU% (100 = one core) per process:\n{}", rows.join("\n")) };
    Ok(text(out))
}

// ---------------------------------------------------------------- screen change

/// FNV-style hash over 8-byte words: cheap enough to run on every poll of a full-screen capture.
pub fn hash_image(img: &Image) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ ((img.w as u64) << 32) ^ img.h as u64;
    let mut chunks = img.rgba.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0x100000001b3).rotate_left(5);
    }
    for b in chunks.remainder() {
        h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

/// Bounding box (x, y, w, h) of the pixels that differ; None when equal. Different sizes count as all changed.
pub fn diff_bbox(a: &Image, b: &Image) -> Option<(u32, u32, u32, u32)> {
    if (a.w, a.h) != (b.w, b.h) {
        return Some((0, 0, b.w, b.h));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let stride = a.w as usize * 4;
    for y in 0..a.h as usize {
        let (ra, rb) = (&a.rgba[y * stride..(y + 1) * stride], &b.rgba[y * stride..(y + 1) * stride]);
        if ra == rb {
            continue;
        }
        let first = ra.chunks_exact(4).zip(rb.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0) as u32;
        let last = ra.chunks_exact(4).zip(rb.chunks_exact(4)).enumerate().filter(|(_, (p, q))| p != q).map(|(i, _)| i as u32).last().unwrap_or(0);
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0 = y0.min(y as u32);
        y1 = y1.max(y as u32);
    }
    (x0 != u32::MAX).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

impl Dev {
    /// `screenshot diff=true`: the note for this capture against the previous one of the same target, then store it.
    pub fn diff_note(&mut self, key: &str, img: &Image, want: bool) -> Option<String> {
        let prev = self.shots.get(key);
        let note = want.then(|| match prev {
            None => "diff: no previous capture of this target; stored as the baseline".to_string(),
            Some(p) if (p.w, p.h) != (img.w, img.h) => format!("diff: size changed {}x{} -> {}x{}", p.w, p.h, img.w, img.h),
            Some(p) => match diff_bbox(p, img) {
                None => "diff: unchanged since the previous capture".to_string(),
                Some((x, y, w, h)) => format!("diff: changed region x={x} y={y} w={w} h={h} (capture pixels) since the previous capture"),
            },
        });
        if want || prev.is_some() {
            if self.shots.len() >= 4 && !self.shots.contains_key(key) {
                self.shots.clear();
            }
            self.shots.insert(key.to_string(), Image { w: img.w, h: img.h, rgba: img.rgba.clone() });
        }
        note
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, v: u8) -> Image {
        Image { w, h, rgba: vec![v; (w * h * 4) as usize] }
    }

    #[test]
    fn cursor_is_split_off() {
        let (b, c) = split_cursor("[1.0] a\n[2.0] b\n-- cursor: s=1;i=2\n");
        assert_eq!(b, "[1.0] a\n[2.0] b");
        assert_eq!(c.as_deref(), Some("s=1;i=2"));
        assert_eq!(split_cursor("").1, None);
    }

    #[test]
    fn journal_command_follows_the_cursor() {
        let q = LogQuery { unit: Some("springboard"), priority: Some("0..3"), grep: Some("it's"), subsystem: None, category: None, cursor: None, lines: 5 };
        let c = journal_cmd(&q);
        assert!(c.contains("-u 'springboard'") && c.contains("-p '0..3'") && c.contains("-n 5") && c.contains("'it'\\''s'"));
        let q = LogQuery { cursor: Some("s=a;i=1"), ..q };
        let c = journal_cmd(&q);
        assert!(c.contains("--after-cursor='s=a;i=1'") && !c.contains("-n 5"));
        let q = LogQuery { subsystem: Some("is.o.notes"), category: Some("x'y"), ..q };
        let c = journal_cmd(&q);
        assert!(c.contains(" 'BIFROST_SUBSYSTEM=is.o.notes'") && c.contains("'BIFROST_CATEGORY=x'\\''y'"), "{c}");
        assert!(valid_priority("err") && valid_priority("0..3") && !valid_priority("3; rm"));
    }

    #[test]
    fn boot_since_uses_uptime() {
        assert_eq!(boot_since("120.55 400.1"), "-125s");
        assert_eq!(boot_since(""), "-3605s");
    }

    #[test]
    fn info_keeps_the_header_and_caps_the_stack() {
        let info = "           PID: 42 (shell)\n       Storage: /var/lib/x\n        Signal: 11 (SEGV)\n\nStack trace of thread 42:\n#0 a\n#1 b\n#2 c\n#3 d\n";
        let t = trim_info(info, 3);
        assert!(t.contains("PID: 42") && t.contains("SEGV") && !t.contains("Storage"));
        assert!(t.contains("#0 a") && t.contains("#1 b") && !t.contains("#2 c") && t.contains("..."));
    }

    #[test]
    fn modes_are_octal() {
        assert_eq!(parse_mode("755"), Some(0o755));
        assert_eq!(parse_mode("0644"), Some(0o644));
        assert_eq!(parse_mode("899"), None);
        assert_eq!(parse_mode(""), None);
    }

    #[test]
    fn cpu_percent_from_tick_deltas() {
        let v = cpu_deltas(&[(1, 100), (2, 50), (3, 10)], &[(1, 150), (2, 50), (3, 12), (4, 9)], 1000);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, 1);
        assert!((v[0].1 - 50.0).abs() < 1e-9 && (v[1].1 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn hash_and_bbox_see_a_changed_pixel() {
        let a = img(10, 6, 7);
        let mut b = img(10, 6, 7);
        assert_eq!(hash_image(&a), hash_image(&b));
        assert_eq!(diff_bbox(&a, &b), None);
        for (x, y) in [(2, 1), (5, 4)] {
            b.rgba[(y * 10 + x) * 4] = 9;
        }
        assert_ne!(hash_image(&a), hash_image(&b));
        assert_eq!(diff_bbox(&a, &b), Some((2, 1, 4, 4)));
    }

    #[test]
    fn diff_note_keeps_a_baseline() {
        let mut d = Dev::default();
        let a = img(4, 4, 1);
        assert!(d.diff_note("screen", &a, true).unwrap().contains("baseline"));
        assert!(d.diff_note("screen", &a, true).unwrap().contains("unchanged"));
        let mut b = img(4, 4, 1);
        b.rgba[0] = 2;
        assert!(d.diff_note("screen", &b, true).unwrap().contains("x=0 y=0 w=1 h=1"));
        assert!(d.diff_note("other", &a, false).is_none());
    }
}
