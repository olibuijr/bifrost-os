//! The `term` MCP tool: drive and debug Term (the BifrOSt terminal built into the shell) through the shell's
//! control port (`term <verb> key=value ...`, values percent-encoded; shell side in
//! `src/shell/terminal/control.rs`). `wait_for` polls `read_screen` here so the shell loop never blocks.
//! The transport is a closure so tests run without a shell.

use crate::sys;
use crate::tools::{f, s, text};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub type SendFn<'a> = dyn FnMut(&str) -> Result<String, String> + 'a;

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

fn num(a: &Value, k: &str) -> Option<u64> {
    f(a, k).filter(|v| *v >= 0.0).map(|v| v as u64)
}

/// The control-port line for one action (`wait_for` is handled by polling and has none).
pub fn build_line(action: &str, a: &Value) -> Result<String, String> {
    let mut kv: Vec<(&str, String)> = Vec::new();
    let tab = num(a, "tab").or_else(|| num(a, "id"));
    let verb = match action {
        "state" => "state",
        "new_tab" => {
            for k in ["cwd", "command"] {
                if let Some(v) = s(a, k) {
                    kv.push((k, pct_encode(v)));
                }
            }
            "new_tab"
        }
        "select_tab" => {
            kv.push(("tab", tab.ok_or("tab is required")?.to_string()));
            "select_tab"
        }
        "close_tab" => {
            if let Some(t) = tab {
                kv.push(("tab", t.to_string()));
            }
            "close_tab"
        }
        "send_text" => {
            // an empty string is allowed, but the key must be there
            let t = a.get("text").and_then(Value::as_str).ok_or("text is required")?;
            kv.push(("text", pct_encode(t)));
            "send_text"
        }
        "send_keys" => {
            let k = match a.get("keys") {
                Some(Value::String(k)) => k.clone(),
                Some(Value::Array(v)) => v.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                _ => return Err("keys is required: \"ctrl+c\", \"Up Up Enter\" or [\"Escape\", \":\", \"q\"]".into()),
            };
            kv.push(("keys", pct_encode(&k)));
            "send_keys"
        }
        "read_screen" => {
            if let Some(n) = num(a, "scrollback_lines") {
                kv.push(("scrollback_lines", n.to_string()));
            }
            "read"
        }
        "resize" => {
            let (c, r) = (num(a, "cols"), num(a, "rows"));
            if c.is_none() && r.is_none() {
                return Err("cols or rows is required".into());
            }
            for (k, v) in [("cols", c), ("rows", r)] {
                if let Some(v) = v {
                    kv.push((k, v.to_string()));
                }
            }
            "resize"
        }
        "set_profile" => {
            kv.push(("name", pct_encode(s(a, "name").or_else(|| s(a, "profile")).ok_or("name is required (see state profiles)")?)));
            "set_profile"
        }
        "clear" => {
            if a.get("screen").and_then(Value::as_bool) == Some(true) {
                kv.push(("screen", "1".into()));
            }
            "clear"
        }
        other => return Err(format!("unknown action {other:?}: state new_tab select_tab close_tab send_text send_keys read_screen wait_for resize set_profile clear")),
    };
    if !matches!(action, "new_tab" | "select_tab" | "set_profile") {
        if let Some(t) = tab {
            if !kv.iter().any(|(k, _)| *k == "tab") {
                kv.push(("tab", t.to_string()));
            }
        }
    }
    Ok(kv.iter().fold(format!("term {verb}"), |l, (k, v)| format!("{l} {k}={v}")))
}

/// `state` reply to JSON: top-level `key=value` lines plus one object per `tab=` line.
pub fn parse_state(reply: &str) -> Value {
    let mut top = serde_json::Map::new();
    let mut tabs = Vec::new();
    for line in reply.lines() {
        if line.starts_with("tab=") {
            let mut o = serde_json::Map::new();
            for w in line.split_whitespace() {
                if let Some((k, v)) = w.split_once('=') {
                    let v = pct_decode(v);
                    o.insert(k.into(), v.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(v)));
                }
            }
            for k in ["current", "alive", "unread"] {
                if let Some(n) = o.get(k).and_then(Value::as_i64) {
                    o.insert(k.into(), json!(n != 0));
                }
            }
            tabs.push(Value::Object(o));
        } else if let Some((k, v)) = line.split_once('=') {
            let v = pct_decode(v);
            top.insert(k.into(), v.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(v)));
        }
    }
    if let Some(Value::String(p)) = top.get("profiles").cloned() {
        top.insert("profiles".into(), json!(p.split(',').map(pct_decode).collect::<Vec<_>>()));
    }
    top.insert("tab_list".into(), Value::Array(tabs));
    Value::Object(top)
}

/// A `read` reply: header lines, `---`, then the screen text.
pub struct Screen {
    pub cursor: (usize, usize),
    pub size: (usize, usize),
    pub alive: bool,
    pub text: String,
}

pub fn parse_screen(reply: &str) -> Result<Screen, String> {
    let (head, body) = reply.split_once("---\n").ok_or("malformed read_screen reply")?;
    let pair = |key: &str, sep: char| -> (usize, usize) {
        head.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_once(sep))
            .map(|(a, b)| (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0)))
            .unwrap_or((0, 0))
    };
    Ok(Screen { cursor: pair("cursor=", ','), size: pair("size=", 'x'), alive: head.lines().any(|l| l == "alive=1"), text: body.to_string() })
}

fn render_screen(sc: &Screen) -> String {
    format!("cursor: col {} row {} (0-based), size {}x{}{}\n---\n{}", sc.cursor.0, sc.cursor.1, sc.size.0, sc.size.1, if sc.alive { "" } else { ", shell exited" }, sc.text)
}

/// Matcher for `wait_for`: a plain substring, or a small regex (see `Re`).
pub enum Matcher {
    Text(String),
    Re(Re),
}

impl Matcher {
    pub fn from_args(a: &Value) -> Result<Matcher, String> {
        match (s(a, "text"), s(a, "regex")) {
            (Some(_), Some(_)) => Err("give text or regex, not both".into()),
            (Some(t), None) => Ok(Matcher::Text(t.to_string())),
            (None, Some(r)) => Re::new(r).map(Matcher::Re),
            (None, None) => Err("text or regex is required".into()),
        }
    }

    /// The matched text, if the screen holds it.
    pub fn find(&self, screen: &str) -> Option<String> {
        match self {
            Matcher::Text(t) => screen.contains(t.as_str()).then(|| t.clone()),
            Matcher::Re(r) => r.find(screen),
        }
    }
}

/// A small backtracking regex (no regex crate in this binary): literals, `.`, `[a-z]` / `[^x]`, `\d \s \w` and
/// escapes, `* + ?`, `^ $` per line, and top-level `|`. Matching is per line, case-sensitive.
pub struct Re {
    alts: Vec<Vec<(Atom, Rep)>>,
}

#[derive(Clone)]
enum Atom {
    Any,
    Lit(char),
    Class(Vec<(char, char)>, bool),
    Bol,
    Eol,
}

#[derive(Clone, Copy, PartialEq)]
enum Rep {
    One,
    Opt,
    Star,
    Plus,
}

fn esc_class(c: char) -> Option<(Vec<(char, char)>, bool)> {
    let d = vec![('0', '9')];
    let w = vec![('0', '9'), ('a', 'z'), ('A', 'Z'), ('_', '_')];
    let sp = vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')];
    Some(match c {
        'd' => (d, false),
        'D' => (d, true),
        'w' => (w, false),
        'W' => (w, true),
        's' => (sp, false),
        'S' => (sp, true),
        _ => return None,
    })
}

impl Re {
    pub fn new(pat: &str) -> Result<Re, String> {
        let cs: Vec<char> = pat.chars().collect();
        let mut alts = vec![Vec::new()];
        let mut i = 0;
        while i < cs.len() {
            let atom = match cs[i] {
                '|' => {
                    alts.push(Vec::new());
                    i += 1;
                    continue;
                }
                '.' => Atom::Any,
                '^' => Atom::Bol,
                '$' => Atom::Eol,
                '\\' => {
                    i += 1;
                    let c = *cs.get(i).ok_or("trailing backslash")?;
                    match esc_class(c) {
                        Some((r, n)) => Atom::Class(r, n),
                        None => Atom::Lit(match c {
                            'n' => '\n',
                            't' => '\t',
                            c => c,
                        }),
                    }
                }
                '[' => {
                    let neg = cs.get(i + 1) == Some(&'^');
                    i += 1 + usize::from(neg);
                    let mut ranges = Vec::new();
                    let mut first = true;
                    loop {
                        let c = *cs.get(i).ok_or("unclosed [")?;
                        if c == ']' && !first {
                            break;
                        }
                        first = false;
                        let c = if c == '\\' {
                            i += 1;
                            let e = *cs.get(i).ok_or("unclosed [")?;
                            if let Some((r, false)) = esc_class(e) {
                                ranges.extend(r);
                                i += 1;
                                continue;
                            }
                            e
                        } else {
                            c
                        };
                        if cs.get(i + 1) == Some(&'-') && cs.get(i + 2).is_some_and(|c| *c != ']') {
                            ranges.push((c, cs[i + 2]));
                            i += 3;
                        } else {
                            ranges.push((c, c));
                            i += 1;
                        }
                    }
                    Atom::Class(ranges, neg)
                }
                '*' | '+' | '?' => return Err(format!("nothing to repeat before {:?}", cs[i])),
                '(' | ')' | '{' => return Err(format!("{:?} is not supported by wait_for regex (use text, or [..] and |)", cs[i])),
                c => Atom::Lit(c),
            };
            i += 1;
            let rep = match cs.get(i) {
                Some('*') => Rep::Star,
                Some('+') => Rep::Plus,
                Some('?') => Rep::Opt,
                _ => Rep::One,
            };
            if rep != Rep::One {
                i += 1;
            }
            alts.last_mut().unwrap().push((atom, rep));
        }
        Ok(Re { alts })
    }

    fn one(a: &Atom, c: char) -> bool {
        match a {
            Atom::Any => true,
            Atom::Lit(l) => *l == c,
            Atom::Class(r, neg) => r.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) != *neg,
            Atom::Bol | Atom::Eol => false,
        }
    }

    /// End of a match of `seq` starting at `pos` of `line`.
    fn run(seq: &[(Atom, Rep)], line: &[char], pos: usize) -> Option<usize> {
        let Some(((atom, rep), rest)) = seq.split_first() else { return Some(pos) };
        match atom {
            Atom::Bol => return if pos == 0 { Self::run(rest, line, pos) } else { None },
            Atom::Eol => return if pos == line.len() { Self::run(rest, line, pos) } else { None },
            _ => {}
        }
        let (min, max) = match rep {
            Rep::One => (1, 1),
            Rep::Opt => (0, 1),
            Rep::Star => (0, usize::MAX),
            Rep::Plus => (1, usize::MAX),
        };
        let mut n = 0;
        while n < max && line.get(pos + n).is_some_and(|c| Self::one(atom, *c)) {
            n += 1;
        }
        loop {
            if n >= min {
                if let Some(e) = Self::run(rest, line, pos + n) {
                    return Some(e);
                }
            }
            if n == 0 || n <= min {
                return None;
            }
            n -= 1;
        }
    }

    /// The first match in any line of `screen`.
    pub fn find(&self, screen: &str) -> Option<String> {
        for l in screen.lines() {
            let line: Vec<char> = l.chars().collect();
            for start in 0..=line.len() {
                for seq in &self.alts {
                    if let Some(end) = Self::run(seq, &line, start) {
                        return Some(line[start..end].iter().collect());
                    }
                }
            }
        }
        None
    }
}

/// Entry from tools.rs dispatch over the real control port.
pub fn run(a: &Value) -> Result<Value, String> {
    let mut send = |line: &str| sys::shell_text(line);
    exec(&mut send, &mut |ms| std::thread::sleep(Duration::from_millis(ms)), a)
}

/// The action logic, independent of the transport; `sleep` is injected so tests do not wait.
pub fn exec(send: &mut SendFn, sleep: &mut dyn FnMut(u64), a: &Value) -> Result<Value, String> {
    let action = s(a, "action").ok_or("action is required: state new_tab select_tab close_tab send_text send_keys read_screen wait_for resize set_profile clear")?;
    let hint = |e: String| -> String {
        if e.contains("connect") || e.contains("refused") {
            format!("term: {e}\nhint: the shell control port is not reachable; is the desktop running? (dev status)")
        } else if e.contains("no tabs") {
            format!("term: {e}\nhint: Term has no open tab; call action=new_tab first")
        } else {
            format!("term: {e}")
        }
    };
    match action {
        "wait_for" => {
            let m = Matcher::from_args(a)?;
            let budget = Duration::from_millis(f(a, "timeout_ms").unwrap_or(10_000.0).clamp(100.0, 120_000.0) as u64);
            let line = build_line("read_screen", &json!({"tab": a.get("tab"), "scrollback_lines": a.get("scrollback_lines")}))?;
            let t = Instant::now();
            let mut last;
            loop {
                let sc = parse_screen(&send(&line).map_err(hint)?)?;
                if let Some(hit) = m.find(&sc.text) {
                    return Ok(text(format!("matched {hit:?} after {} ms\n{}", t.elapsed().as_millis(), render_screen(&sc))));
                }
                let exited = !sc.alive;
                last = sc;
                if exited || t.elapsed() >= budget {
                    break;
                }
                sleep(100);
            }
            Ok(crate::tools::err(format!("wait_for: {} within {} ms\nlast screen:\n{}", if last.alive { "no match" } else { "shell exited without a match" }, budget.as_millis(), render_screen(&last))))
        }
        _ => {
            let line = build_line(action, a)?;
            let reply = send(&line).map_err(hint)?;
            Ok(text(match action {
                "state" => serde_json::to_string_pretty(&parse_state(&reply)).unwrap_or_default(),
                "read_screen" => render_screen(&parse_screen(&reply)?),
                _ => reply.trim().to_string(),
            }))
        }
    }
}

pub fn schema() -> Value {
    json!({"name":"term","description":"Drive and debug Term, the BifrOSt terminal (Term clone built into the shell). One tool, `action` picks: state (tabs with title, cwd, foreground job, cols x rows, alive/unread, current tab, profile and the profile names); new_tab {cwd, command} (opens the Term window if needed; command is typed and run); select_tab {tab}; close_tab {tab}; send_text {text, tab} types text (a newline is Enter); send_keys {keys, tab} presses keys: a string or list of \"Enter\", \"Tab\", \"Escape\", \"Backspace\", \"Up\"/\"Down\"/\"Left\"/\"Right\", \"Home\", \"End\", \"PageUp\", \"PageDown\", \"Delete\", \"F1\"-\"F12\", \"ctrl+c\", \"alt+x\" or one character; read_screen {tab, scrollback_lines} returns the visible screen as text with the cursor (0-based col,row) and size, plus that many history lines above it; wait_for {text | regex, timeout_ms=10000, tab} polls the screen until it matches or the shell exits (regex: literals, ., [a-z], [^x], \\d \\s \\w, * + ?, ^ $, |; no groups); resize {cols, rows} sets the grid of a tab until the window next changes size; set_profile {name} (see state for names) changes the colour profile of all tabs; clear {screen} clears scrollback, and the screen too with screen=true. `tab` is the index from state and defaults to the current tab. Typical loop: new_tab command=\"make\", wait_for regex=\"[$#%]$\", read_screen.",
        "inputSchema":{"type":"object","properties":{
            "action":{"type":"string","enum":["state","new_tab","select_tab","close_tab","send_text","send_keys","read_screen","wait_for","resize","set_profile","clear"]},
            "tab":{"type":"integer","description":"tab index from state; default the current tab"},
            "cwd":{"type":"string"},"command":{"type":"string"},"text":{"type":"string"},"regex":{"type":"string"},
            "keys":{"description":"key names, space separated string or list","anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]},
            "scrollback_lines":{"type":"integer"},"timeout_ms":{"type":"number"},
            "cols":{"type":"integer"},"rows":{"type":"integer"},"name":{"type":"string"},"screen":{"type":"boolean"}
        },"required":["action"]}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_built_and_encoded() {
        assert_eq!(build_line("state", &json!({})).unwrap(), "term state");
        assert_eq!(build_line("new_tab", &json!({"cwd":"/tmp/a b","command":"ls -l"})).unwrap(), "term new_tab cwd=/tmp/a%20b command=ls%20-l");
        assert_eq!(build_line("send_text", &json!({"text":"echo hi\n","tab":2})).unwrap(), "term send_text text=echo%20hi%0A tab=2");
        assert_eq!(build_line("send_keys", &json!({"keys":["ctrl+c","Enter"]})).unwrap(), "term send_keys keys=ctrl%2Bc%20Enter");
        assert_eq!(build_line("read_screen", &json!({"scrollback_lines":50})).unwrap(), "term read scrollback_lines=50");
        assert_eq!(build_line("resize", &json!({"cols":100,"rows":30})).unwrap(), "term resize cols=100 rows=30");
        assert_eq!(build_line("clear", &json!({"screen":true})).unwrap(), "term clear screen=1");
        assert_eq!(build_line("close_tab", &json!({"tab":1})).unwrap(), "term close_tab tab=1");
        assert!(build_line("select_tab", &json!({})).is_err());
        assert!(build_line("send_text", &json!({})).is_err());
        assert!(build_line("resize", &json!({})).is_err());
        assert!(build_line("bogus", &json!({})).is_err());
    }

    #[test]
    fn codec_round_trips() {
        for t in ["plain", "a b%c\n", "ünï/ç", ""] {
            assert_eq!(pct_decode(&pct_encode(t)), t);
        }
    }

    #[test]
    fn state_reply_parses() {
        let v = parse_state("tabs=2\ncurrent=1\nprofile=Basic\ntheme=0\nzoom=3\nprofiles=Basic,Pro\ntab=0 current=0 title=shell%201 cwd=/home/me job= cols=80 rows=24 alive=1 unread=0\ntab=1 current=1 title=vim cwd=/tmp job=vim cols=80 rows=24 alive=1 unread=1\n");
        assert_eq!(v["tabs"], 2);
        assert_eq!(v["profiles"], json!(["Basic", "Pro"]));
        assert_eq!(v["tab_list"][0]["title"], "shell 1");
        assert_eq!(v["tab_list"][1]["job"], "vim");
        assert_eq!(v["tab_list"][1]["unread"], true);
        assert_eq!(v["tab_list"][0]["current"], false);
    }

    #[test]
    fn screen_reply_parses() {
        let sc = parse_screen("tab=0\ncursor=5,2\nsize=80x24\nalive=1\n---\nline one\n$ ").unwrap();
        assert_eq!((sc.cursor, sc.size, sc.alive), ((5, 2), (80, 24), true));
        assert_eq!(sc.text, "line one\n$ ");
        assert!(parse_screen("garbage").is_err());
    }

    #[test]
    fn regex_matches_lines() {
        let m = |p: &str, s: &str| Re::new(p).unwrap().find(s);
        assert_eq!(m("build [0-9]+ ok", "x\nbuild 42 ok\n").as_deref(), Some("build 42 ok"));
        assert_eq!(m("^\\$ $", "out\n$ ").as_deref(), Some("$ "));
        assert_eq!(m("error|fail", "it did fail").as_deref(), Some("fail"));
        assert_eq!(m("colou?r", "color").as_deref(), Some("color"));
        assert_eq!(m("a.*c", "abbbc").as_deref(), Some("abbbc"));
        assert_eq!(m("\\d+\\.\\d", "v 10.5").as_deref(), Some("10.5"));
        assert_eq!(m("[^a-z]+", "abc123").as_deref(), Some("123"));
        assert_eq!(m("^b", "ab"), None);
        assert!(Re::new("(a)").is_err());
        assert!(Re::new("*a").is_err());
        assert!(Re::new("[a").is_err());
    }

    #[test]
    fn wait_for_polls_until_a_match() {
        let mut n = 0;
        let mut sent = Vec::new();
        let mut send = |l: &str| -> Result<String, String> {
            sent.push(l.to_string());
            n += 1;
            Ok(format!("cursor=0,0\nsize=80x24\nalive=1\n---\n{}", if n < 3 { "building" } else { "done 0 errors\n$ " }))
        };
        let mut slept = 0;
        let r = exec(&mut send, &mut |_| slept += 1, &json!({"action":"wait_for","regex":"done \\d errors"})).unwrap();
        assert!(r["isError"].is_null());
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("matched \"done 0 errors\""));
        assert_eq!(slept, 2);
        assert_eq!(sent[0], "term read");
    }

    #[test]
    fn wait_for_times_out_and_stops_when_the_shell_exits() {
        let mut send = |_: &str| -> Result<String, String> { Ok("cursor=0,0\nsize=80x24\nalive=0\n---\nbye".into()) };
        let r = exec(&mut send, &mut |_| {}, &json!({"action":"wait_for","text":"never"})).unwrap();
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("shell exited"));
        assert!(exec(&mut send, &mut |_| {}, &json!({"action":"wait_for"})).is_err());
    }

    #[test]
    fn errors_get_hints() {
        let mut send = |_: &str| -> Result<String, String> { Err("Term has no tabs; use new_tab".into()) };
        let e = exec(&mut send, &mut |_| {}, &json!({"action":"read_screen"})).unwrap_err();
        assert!(e.contains("hint: Term has no open tab"));
    }
}
