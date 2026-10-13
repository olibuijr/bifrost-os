//! `launchctl`: lists, shows and controls the user's background services. The services are the user's systemd units
//! (`systemctl --user`): a label such as `is.olibuijr.iosterm` names the unit `is.olibuijr.iosterm.service`, and a
//! label that already ends in `.service` is used as it is.
//! Supported: `list`, `print <label>`, `start`, `stop`, `enable`, `disable` and `kickstart [-k] <label>` (`-k` restarts
//! a running service, without it the service is only started when it is not running).
//! Gap: launchd's per-domain targets (`gui/<uid>`, `system`), `load`/`unload` plists, and the last exit status column
//! (the list shows the unit's state instead, and the PID column is `-`) are not reproduced.

use std::fmt::Write as _;

pub const USAGE: &str = "usage: launchctl [list | print <label> | start | stop | enable | disable <label> | kickstart [-k] <label>]";
/// Properties `print` reads from the unit.
pub const SHOW_PROPERTIES: &str = "--property=Id,Description,LoadState,ActiveState,SubState,MainPID,UnitFileState,FragmentPath";

/// The simple verbs that act on one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Start,
    Stop,
    Enable,
    Disable,
}

impl Verb {
    fn word(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Enable => "enable",
            Verb::Disable => "disable",
        }
    }
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    List,
    Print(String),
    Act(Verb, String),
    Kickstart { restart: bool, label: String },
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => Ok(Command::List),
        ["-h" | "-help" | "--help"] => Ok(Command::Help),
        ["list"] => Ok(Command::List),
        ["print", label] => Ok(Command::Print(checked_label(label)?)),
        ["start", label] => Ok(Command::Act(Verb::Start, checked_label(label)?)),
        ["stop", label] => Ok(Command::Act(Verb::Stop, checked_label(label)?)),
        ["enable", label] => Ok(Command::Act(Verb::Enable, checked_label(label)?)),
        ["disable", label] => Ok(Command::Act(Verb::Disable, checked_label(label)?)),
        ["kickstart", label] => Ok(Command::Kickstart { restart: false, label: checked_label(label)? }),
        ["kickstart", "-k", label] => Ok(Command::Kickstart { restart: true, label: checked_label(label)? }),
        [verb, ..] if ["print", "start", "stop", "enable", "disable", "kickstart"].contains(verb) => {
            Err(format!("{verb} needs one service label\n{USAGE}"))
        }
        [other, ..] => Err(format!("unknown subcommand {other}\n{USAGE}")),
    }
}

/// A label is a plain name: letters, digits and `.`, `-`, `_`, `@`, no slash and no leading dash.
pub fn checked_label(label: &str) -> Result<String, String> {
    let plain = !label.is_empty()
        && !label.starts_with('-')
        && label.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'@'));
    if plain {
        Ok(label.to_string())
    } else {
        Err(format!("\u{201c}{label}\u{201d} is not a service label"))
    }
}

/// The systemd unit a label names.
pub fn unit_name(label: &str) -> String {
    if label.ends_with(".service") {
        label.to_string()
    } else {
        format!("{label}.service")
    }
}

/// The arguments for `systemctl --user` that carry out a command.
pub fn systemctl_args(cmd: &Command) -> Option<Vec<String>> {
    let s = |v: &[&str]| -> Option<Vec<String>> { Some(v.iter().map(|x| x.to_string()).collect()) };
    match cmd {
        Command::Help => None,
        Command::List => s(&["list-units", "--type=service", "--all", "--no-legend", "--plain", "--no-pager"]),
        Command::Print(label) => {
            let mut args = vec!["show".to_string(), unit_name(label), SHOW_PROPERTIES.to_string()];
            args.push("--no-pager".into());
            Some(args)
        }
        Command::Act(verb, label) => Some(vec![verb.word().into(), unit_name(label)]),
        Command::Kickstart { restart, label } => Some(vec![if *restart { "restart" } else { "start" }.into(), unit_name(label)]),
    }
}

/// The table `launchctl list` prints, from `systemctl --user list-units` output.
pub fn list_text(units: &str) -> String {
    let mut out = String::from("PID\tState\tLabel\n");
    for line in units.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [unit, _load, active, sub, ..] = fields.as_slice() else { continue };
        let Some(label) = unit.strip_suffix(".service") else { continue };
        let _ = writeln!(out, "-\t{active}/{sub}\t{label}");
    }
    out
}

/// The block `launchctl print <label>` prints, from `systemctl --user show` output (`Key=value` lines).
pub fn print_text(label: &str, show: &str) -> Result<String, String> {
    let prop = |key: &str| show.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).unwrap_or("");
    if prop("LoadState") == "not-found" {
        return Err(format!("Could not find service \"{label}\" in domain for user"));
    }
    let mut out = format!("{label} = {{\n");
    let _ = writeln!(out, "\tstate = {} ({})", prop("ActiveState"), prop("SubState"));
    let pid = prop("MainPID");
    if !pid.is_empty() && pid != "0" {
        let _ = writeln!(out, "\tpid = {pid}");
    }
    let _ = writeln!(out, "\tenabled = {}", prop("UnitFileState"));
    let _ = writeln!(out, "\tdescription = {}", prop("Description"));
    let _ = writeln!(out, "\tpath = {}", prop("FragmentPath"));
    out.push_str("}\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn subcommands_parse_with_their_labels() {
        assert_eq!(parse(&args(&[])), Ok(Command::List));
        assert_eq!(parse(&args(&["list"])), Ok(Command::List));
        assert_eq!(parse(&args(&["print", "is.olibuijr.iosterm"])), Ok(Command::Print("is.olibuijr.iosterm".into())));
        assert_eq!(parse(&args(&["stop", "a.b"])), Ok(Command::Act(Verb::Stop, "a.b".into())));
        assert_eq!(parse(&args(&["kickstart", "-k", "a.b"])), Ok(Command::Kickstart { restart: true, label: "a.b".into() }));
        assert_eq!(parse(&args(&["-h"])), Ok(Command::Help));
    }

    #[test]
    fn bad_subcommands_and_labels_are_refused() {
        assert!(parse(&args(&["start"])).unwrap_err().contains("one service label"));
        assert!(parse(&args(&["load", "x"])).unwrap_err().contains("unknown subcommand load"));
        assert!(parse(&args(&["print", "../etc"])).unwrap_err().contains("not a service label"));
        assert!(parse(&args(&["print", "-x"])).is_err());
    }

    #[test]
    fn labels_name_user_units() {
        assert_eq!(unit_name("is.olibuijr.iosterm"), "is.olibuijr.iosterm.service");
        assert_eq!(unit_name("svc.service"), "svc.service");
    }

    #[test]
    fn each_command_becomes_one_systemctl_call() {
        assert_eq!(systemctl_args(&Command::Act(Verb::Disable, "x".into())).unwrap(), vec!["disable", "x.service"]);
        assert_eq!(systemctl_args(&Command::Kickstart { restart: true, label: "x".into() }).unwrap(), vec!["restart", "x.service"]);
        assert_eq!(systemctl_args(&Command::Kickstart { restart: false, label: "x".into() }).unwrap(), vec!["start", "x.service"]);
        assert_eq!(systemctl_args(&Command::Help), None);
    }

    #[test]
    fn the_list_shows_one_row_per_service() {
        let units = "a.b.service loaded active running Some Thing\nc.service loaded failed failed Broken\nmount.mount loaded active mounted x\n";
        let text = list_text(units);
        assert!(text.starts_with("PID\tState\tLabel\n"));
        assert!(text.contains("-\tactive/running\ta.b"));
        assert!(text.contains("-\tfailed/failed\tc"));
        assert!(!text.contains("mount"));
    }

    #[test]
    fn print_shows_state_pid_and_path() {
        let show = "Id=x.service\nLoadState=loaded\nActiveState=active\nSubState=running\nMainPID=42\nUnitFileState=enabled\nDescription=X\nFragmentPath=/u/x.service\n";
        let text = print_text("x", show).unwrap();
        assert!(text.contains("state = active (running)"));
        assert!(text.contains("pid = 42"));
        assert!(text.contains("enabled = enabled"));
        assert!(text.contains("path = /u/x.service"));
    }

    #[test]
    fn print_of_an_unknown_label_is_an_error() {
        let show = "LoadState=not-found\nActiveState=inactive\n";
        assert!(print_text("nope", show).unwrap_err().contains("Could not find service"));
    }

    #[test]
    fn a_stopped_service_has_no_pid_line() {
        let show = "LoadState=loaded\nActiveState=inactive\nSubState=dead\nMainPID=0\n";
        assert!(!print_text("y", show).unwrap().contains("pid ="));
    }
}
