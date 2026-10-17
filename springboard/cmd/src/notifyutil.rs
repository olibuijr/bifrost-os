//! `notifyutil`: posts named notifications, watches them, and reads or sets the 64-bit state value kept for each name. It is
//! the command-line side of the shell's notify bus (`shell::services::notify_bus`). Each action is one `notify <verb> ...`
//! line on the shell's control socket. A watch registers a poll token and checks it every `POLL_MS` until a post arrives,
//! then cancels the token when it ends.
//! Gap: the bus is the shell's own, so the names are BifrOSt names, not the system-wide Darwin names. A watch polls (a post
//! is seen up to `POLL_MS` late) instead of waking on a pushed event. Names cannot hold white space, and state values are
//! whole numbers up to 2^53, the most the shell's JSON keeps exactly.

use bifrost_core::json::{self, Json};
use std::io::Write;

pub const USAGE: &str = "usage: notifyutil [-v] [-z msec] (-p name | -w [count] name | -1 name | -g name | -s name value)";
/// How often a watch checks its token, in milliseconds.
pub const POLL_MS: u64 = 50;
/// The largest state value: 2^53, the most a JSON number on the shell's bus keeps exactly.
pub const MAX_STATE: u64 = 9_007_199_254_740_992;
/// The longest name the bus takes, in bytes.
pub const MAX_NAME: usize = 128;

/// What the command line asks for. Exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `-p name`: posts the name.
    Post(String),
    /// `-w [count] name` or `-1 name`: prints each post of the name, stopping after `count` (every post when `None`).
    Wait { name: String, count: Option<u64> },
    /// `-g name`: prints the name's state value.
    Get(String),
    /// `-s name value`: sets the name's state value.
    Set(String, u64),
}

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub action: Action,
    /// `-v`: says what each action did.
    pub verbose: bool,
    /// `-z msec`: how long a watch waits before it gives up. `None` waits for as long as it takes.
    pub timeout_ms: Option<u64>,
}

/// A name the command line can send: non-empty, short enough, and no white space or control characters.
pub fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err(format!("a name is 1 to {MAX_NAME} bytes with no white space, not {name:?}"))
    } else {
        Ok(())
    }
}

/// Reads the options. The name or value comes after its flag; `-w` takes an optional count before its name.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut action: Option<Action> = None;
    let mut verbose = false;
    let mut timeout_ms = None;
    let mut it = args.iter().peekable();
    let mut set = |a: Action, action: &mut Option<Action>| -> Result<(), String> {
        if action.is_some() {
            return Err("give one of -p, -w, -1, -g or -s".into());
        }
        *action = Some(a);
        Ok(())
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-v" => verbose = true,
            "-z" => {
                let v = it.next().ok_or("-z needs a number of milliseconds")?;
                timeout_ms = Some(v.parse::<u64>().map_err(|_| format!("-z needs a number of milliseconds, not {v}"))?);
            }
            "-p" => {
                let name = it.next().ok_or("-p needs a name")?;
                valid_name(name)?;
                set(Action::Post(name.clone()), &mut action)?;
            }
            "-g" => {
                let name = it.next().ok_or("-g needs a name")?;
                valid_name(name)?;
                set(Action::Get(name.clone()), &mut action)?;
            }
            "-s" => {
                let name = it.next().ok_or("-s needs a name and a value")?;
                valid_name(name)?;
                let value = it.next().ok_or("-s needs a value after the name")?;
                let state = parse_state(value)?;
                set(Action::Set(name.clone(), state), &mut action)?;
            }
            "-1" => {
                let name = it.next().ok_or("-1 needs a name")?;
                valid_name(name)?;
                set(Action::Wait { name: name.clone(), count: Some(1) }, &mut action)?;
            }
            "-w" => {
                // a count is a number followed by a name; a number alone is the name
                let mut count = None;
                let mut name = it.next().ok_or("-w needs a name")?;
                if let Ok(n) = name.parse::<u64>() {
                    if n == 0 {
                        return Err("-w needs a count of at least 1".into());
                    }
                    count = Some(n);
                    name = it.next().ok_or("-w needs a name after the count")?;
                }
                valid_name(name)?;
                set(Action::Wait { name: name.clone(), count }, &mut action)?;
            }
            other => return Err(format!("unknown option or extra operand {other}")),
        }
    }
    let action = action.ok_or("give one of -p, -w, -1, -g or -s")?;
    Ok(Options { action, verbose, timeout_ms })
}

/// A state value: a whole number from 0 up to [`MAX_STATE`].
pub fn parse_state(value: &str) -> Result<u64, String> {
    value.parse::<u64>().ok().filter(|&n| n <= MAX_STATE).ok_or_else(|| format!("a state value is a whole number up to {MAX_STATE}, not {value}"))
}

/// The line for one action. Verbs and fields match `control::Request::Notify` in the shell.
pub fn line_for_post(name: &str) -> String {
    format!("notify post {name}")
}

/// The request line one command sends to the shell. The shell's word split is the same as this one.
pub fn line_for_get(name: &str) -> String {
    format!("notify get {name}")
}

/// Where a request line goes: the shell's control socket. `ask` returns the JSON the shell answered with.
pub trait Bus {
    fn ask(&mut self, line: &str) -> Result<Json, String>;
    /// Waits `ms` milliseconds between checks.
    fn pause(&mut self, ms: u64);
}

/// The shell's control socket, on its port.
pub struct Control {
    pub port: u16,
}

impl Bus for Control {
    fn ask(&mut self, line: &str) -> Result<Json, String> {
        let answer = bifrost_cmd_request(line, self.port)?;
        parse_answer(&answer)
    }

    fn pause(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

fn bifrost_cmd_request(line: &str, port: u16) -> Result<String, String> {
    let bytes = crate::control::request(line, port)?;
    String::from_utf8(bytes).map_err(|_| "the desktop answered with something that is not text".to_string())
}

/// Reads the shell's answer: `ok <json>` gives the JSON, `error: <why>` gives the reason.
pub fn parse_answer(answer: &str) -> Result<Json, String> {
    let text = answer.trim();
    if let Some(reason) = text.strip_prefix("error: ") {
        return Err(reason.to_string());
    }
    match text.strip_prefix("ok") {
        Some(body) => json::parse(body.trim()),
        None => Err(format!("the desktop answered {text:?}")),
    }
}

/// Runs the action on `bus`, printing to `out`. A watch that times out is an error.
pub fn run(bus: &mut dyn Bus, opts: &Options, out: &mut dyn Write) -> Result<(), String> {
    match &opts.action {
        Action::Post(name) => {
            let reply = bus.ask(&line_for_post(name))?;
            if opts.verbose {
                let delivered = reply.get("delivered").and_then(Json::as_u64).unwrap_or(0);
                writeln!(out, "posted {name} to {delivered} listener(s)").map_err(|e| e.to_string())?;
            }
        }
        Action::Get(name) => {
            let reply = bus.ask(&line_for_get(name))?;
            let state = reply.get("state").and_then(Json::as_u64).ok_or("the desktop gave no state")?;
            if opts.verbose {
                writeln!(out, "{name}: {state}").map_err(|e| e.to_string())?;
            } else {
                writeln!(out, "{state}").map_err(|e| e.to_string())?;
            }
        }
        Action::Set(name, state) => {
            bus.ask(&format!("notify set {name} {state}"))?;
            if opts.verbose {
                writeln!(out, "set {name} to {state}").map_err(|e| e.to_string())?;
            }
        }
        Action::Wait { name, count } => watch(bus, name, *count, opts, out)?,
    }
    Ok(())
}

/// Registers a poll token for `name`, checks it until the posts asked for have come, then cancels it.
fn watch(bus: &mut dyn Bus, name: &str, count: Option<u64>, opts: &Options, out: &mut dyn Write) -> Result<(), String> {
    let reply = bus.ask(&format!("notify register {name}"))?;
    let token = reply.get("token").and_then(Json::as_u64).ok_or("the desktop gave no token")?;
    let result = watch_token(bus, name, token, count, opts, out);
    // the token is cancelled whatever happened; the shell keeps it otherwise
    let _ = bus.ask(&format!("notify cancel {token}"));
    result
}

fn watch_token(bus: &mut dyn Bus, name: &str, token: u64, count: Option<u64>, opts: &Options, out: &mut dyn Write) -> Result<(), String> {
    let mut seen = 0u64;
    let mut waited_ms = 0u64;
    loop {
        let reply = bus.ask(&format!("notify check {token}"))?;
        if reply.get("pending") == Some(&Json::Bool(true)) {
            seen += 1;
            writeln!(out, "{name}").map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            if count.is_some_and(|c| seen >= c) {
                return Ok(());
            }
        } else {
            if opts.timeout_ms.is_some_and(|limit| waited_ms >= limit) {
                return Err(format!("timed out waiting for {name}"));
            }
            bus.pause(POLL_MS);
            waited_ms += POLL_MS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A bus that answers from the rules of the shell, with `pending_after` empty checks before a post shows.
    #[derive(Default)]
    struct Fake {
        lines: Vec<String>,
        pending_after: u64,
        checks: u64,
        paused: u64,
        state: u64,
    }

    impl Bus for Fake {
        fn ask(&mut self, line: &str) -> Result<Json, String> {
            self.lines.push(line.to_string());
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["notify", "post", _] => Ok(json::parse("{\"delivered\":1}")?),
                ["notify", "get", _] => Ok(json::parse(&format!("{{\"state\":{}}}", self.state))?),
                ["notify", "set", _, v] => {
                    self.state = v.parse().unwrap();
                    Ok(Json::Null)
                }
                ["notify", "register", _] => Ok(json::parse("{\"token\":7}")?),
                ["notify", "check", _] => {
                    self.checks += 1;
                    Ok(json::parse(&format!("{{\"pending\":{}}}", self.checks > self.pending_after))?)
                }
                ["notify", "cancel", _] => Ok(json::parse("{\"ok\":true}")?),
                _ => Err(format!("no rule for {line}")),
            }
        }

        fn pause(&mut self, ms: u64) {
            self.paused += ms;
        }
    }

    #[test]
    fn parses_each_action_and_the_options() {
        assert_eq!(parse(&args(&["-p", "a.b"])).unwrap().action, Action::Post("a.b".into()));
        assert_eq!(parse(&args(&["-g", "a.b"])).unwrap().action, Action::Get("a.b".into()));
        assert_eq!(parse(&args(&["-s", "a.b", "7"])).unwrap().action, Action::Set("a.b".into(), 7));
        assert_eq!(parse(&args(&["-1", "a.b"])).unwrap().action, Action::Wait { name: "a.b".into(), count: Some(1) });
        assert_eq!(parse(&args(&["-w", "a.b"])).unwrap().action, Action::Wait { name: "a.b".into(), count: None });
        assert_eq!(parse(&args(&["-w", "3", "a.b"])).unwrap().action, Action::Wait { name: "a.b".into(), count: Some(3) });
        let o = parse(&args(&["-v", "-z", "250", "-w", "a.b"])).unwrap();
        assert!(o.verbose);
        assert_eq!(o.timeout_ms, Some(250));
    }

    #[test]
    fn refuses_bad_command_lines() {
        assert!(parse(&args(&[])).is_err(), "an action is needed");
        assert!(parse(&args(&["-p", "a", "-g", "b"])).is_err(), "one action only");
        assert!(parse(&args(&["-p"])).is_err());
        assert!(parse(&args(&["-p", "has space"])).is_err());
        assert!(parse(&args(&["-p", ""])).is_err());
        assert!(parse(&args(&["-s", "a", "-1"])).is_err(), "a state value must be a number");
        assert!(parse(&args(&["-s", "a", "9007199254740993"])).is_err(), "above 2^53 is refused");
        assert!(parse(&args(&["-w", "0", "a"])).is_err());
        assert!(parse(&args(&["-z", "soon", "-p", "a"])).is_err());
        assert!(parse(&args(&["-q", "a"])).is_err());
    }

    #[test]
    fn post_and_get_and_set_send_one_line_each() {
        let mut bus = Fake::default();
        let mut out = Vec::new();
        run(&mut bus, &Options { action: Action::Set("a.b".into(), 9), verbose: false, timeout_ms: None }, &mut out).unwrap();
        run(&mut bus, &Options { action: Action::Get("a.b".into()), verbose: false, timeout_ms: None }, &mut out).unwrap();
        run(&mut bus, &Options { action: Action::Post("a.b".into()), verbose: false, timeout_ms: None }, &mut out).unwrap();
        assert_eq!(bus.lines, vec!["notify set a.b 9", "notify get a.b", "notify post a.b"]);
        assert_eq!(String::from_utf8(out).unwrap(), "9\n", "only the get prints, and a plain get prints the number alone");
    }

    #[test]
    fn wait_prints_each_post_and_cancels_its_token() {
        let mut bus = Fake { pending_after: 2, ..Fake::default() };
        let mut out = Vec::new();
        let opts = Options { action: Action::Wait { name: "a.b".into(), count: Some(2) }, verbose: false, timeout_ms: None };
        // two empty checks, then a post shows on every check: two lines, then the token is cancelled
        bus.pending_after = 0;
        run(&mut bus, &opts, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "a.b\na.b\n");
        assert_eq!(bus.lines.first().map(String::as_str), Some("notify register a.b"));
        assert_eq!(bus.lines.last().map(String::as_str), Some("notify cancel 7"));
    }

    #[test]
    fn wait_polls_until_the_post_comes() {
        let mut bus = Fake { pending_after: 3, ..Fake::default() };
        let mut out = Vec::new();
        let opts = Options { action: Action::Wait { name: "a.b".into(), count: Some(1) }, verbose: false, timeout_ms: Some(1000) };
        run(&mut bus, &opts, &mut out).unwrap();
        assert_eq!(bus.paused, 3 * POLL_MS, "three empty checks, each followed by a pause");
        assert_eq!(String::from_utf8(out).unwrap(), "a.b\n");
    }

    #[test]
    fn a_watch_times_out_and_still_cancels() {
        let mut bus = Fake { pending_after: u64::MAX, ..Fake::default() };
        let mut out = Vec::new();
        let opts = Options { action: Action::Wait { name: "a.b".into(), count: None }, verbose: false, timeout_ms: Some(120) };
        assert_eq!(run(&mut bus, &opts, &mut out), Err("timed out waiting for a.b".into()));
        assert_eq!(bus.lines.last().map(String::as_str), Some("notify cancel 7"));
        assert!(out.is_empty());
    }

    #[test]
    fn verbose_says_what_it_did() {
        let mut bus = Fake::default();
        let mut out = Vec::new();
        let opts = Options { action: Action::Post("a.b".into()), verbose: true, timeout_ms: None };
        run(&mut bus, &opts, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "posted a.b to 1 listener(s)\n");
    }

    #[test]
    fn answers_are_read_as_ok_json_or_error() {
        assert_eq!(parse_answer("ok {\"state\":4}\n").unwrap().get("state").and_then(Json::as_u64), Some(4));
        assert_eq!(parse_answer("ok null\n"), Ok(Json::Null));
        assert_eq!(parse_answer("error: notify_bus: nope\n"), Err("notify_bus: nope".into()));
        assert!(parse_answer("garbage").is_err());
    }
}
