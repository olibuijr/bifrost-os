//! `softwareupdate`: lists and installs OS and app updates through the update agent (`updated`), over its service socket.
//! The agent does the checking, the download and the staging; this tool asks it and waits for its state.
//! Supported: `-l` (check now and list what is offered), `-i <label>` and `-a` (download and stage the offered OS update;
//! the desktop restarts to finish the install) and `--schedule` (prints the background check preference; `on|off` sets it,
//! and manual checks always run).
//! App updates have no install command on the agent's protocol yet, so `-i` names only the OS update.

use bifrost_proto::svc::{self, update::{Cmd, Evt, Phase, State}};
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = "usage: softwareupdate [-l] [-i label ...] [-a] [--schedule on | off]";
/// The socket name the agent listens on, under the shell's data folder.
pub const SOCKET_NAME: &str = "is.olibuijr.update.sock";
/// How long a reply may take before the tool gives up (a download can be slow, the check is not).
const TIMEOUT: Duration = Duration::from_secs(600);

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Help,
    List,
    /// `-i` labels and `-a`: the OS update is downloaded and staged when `all` or its label is named.
    Install { labels: Vec<String>, all: bool },
    /// `--schedule on|off`: whether the background checks run.
    Schedule { automatic: bool },
    /// `--schedule` alone: prints whether the background checks run.
    ScheduleQuery,
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Mode, String> {
    let mut list = false;
    let mut all = false;
    let mut labels = Vec::new();
    // outer: `--schedule` was given; inner: the value, when `on` or `off` follows it
    let mut schedule: Option<Option<bool>> = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "-help" | "--help" => return Ok(Mode::Help),
            "-l" => list = true,
            "-a" => all = true,
            "-i" => labels.push(it.next().ok_or("-i needs an update label")?.clone()),
            "--schedule" => {
                let value = match it.peek().map(|s| s.as_str()) {
                    Some("on") => Some(true),
                    Some("off") => Some(false),
                    _ => None,
                };
                if value.is_some() {
                    it.next();
                }
                schedule = Some(value);
            }
            other => return Err(format!("option {other} is not supported yet")),
        }
    }
    if let Some(value) = schedule {
        if list || all || !labels.is_empty() {
            return Err("--schedule cannot be combined with -l, -i or -a".into());
        }
        return Ok(match value {
            Some(automatic) => Mode::Schedule { automatic },
            None => Mode::ScheduleQuery,
        });
    }
    if list && (all || !labels.is_empty()) {
        return Err("-l lists updates; it cannot be combined with -i or -a".into());
    }
    if all || !labels.is_empty() {
        return Ok(Mode::Install { labels, all });
    }
    if list {
        return Ok(Mode::List);
    }
    Err(USAGE.to_string())
}

/// Where the update agent listens: `BIFROST_SERVICE_SOCK` when set, else `run/` under the shell's data folder.
pub fn socket_path(env_sock: Option<&str>, root: bool, xdg_data_home: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(sock) = env_sock.filter(|s| !s.is_empty()) {
        return PathBuf::from(sock);
    }
    let data = if root {
        PathBuf::from("/var/lib/springboard")
    } else if let Some(x) = xdg_data_home.filter(|s| !s.is_empty()) {
        PathBuf::from(x).join("springboard")
    } else {
        PathBuf::from(home.unwrap_or(".")).join(".local/share/springboard")
    };
    data.join("run").join(SOCKET_NAME)
}

/// Whether the agent's phase is one where it has finished the step the tool asked for.
pub fn is_settled(phase: Phase) -> bool {
    matches!(phase, Phase::UpToDate | Phase::Available | Phase::Staged | Phase::Failed)
}

/// The text `softwareupdate -l` prints for a state. Empty when nothing is offered.
pub fn list_text(state: &State) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(offer) = &state.offer {
        lines.push(format!("* Label: BifrOSt {}", offer.version));
        lines.push(format!("\tTitle: BifrOSt, Version: {}, Size: {}K, Action: restart", offer.version, offer.size / 1024));
    }
    for (id, version) in &state.apps {
        lines.push(format!("* Label: {id}-{version}"));
        let recommended = if state.system.contains(id) { "YES" } else { "NO" };
        lines.push(format!("\tTitle: {id}, Version: {version}, Recommended: {recommended}"));
    }
    if lines.is_empty() {
        return "No new software available.\n".into();
    }
    format!("Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n{}\n", lines.join("\n"))
}

/// The label of the offered OS update, the name `-i` takes for it.
pub fn os_label(state: &State) -> Option<String> {
    state.offer.as_ref().map(|o| format!("BifrOSt {}", o.version))
}

/// Sends one command on the connection.
fn send(stream: &mut UnixStream, cmd: &Cmd) -> io::Result<()> {
    svc::write_frame(stream, &cmd.encode())
}

/// Reads agent events until `done` says the state has settled, calling `on_state` for each state seen. Returns the
/// settled state. A connection that closes first is an error.
fn wait_until(stream: &mut UnixStream, mut done: impl FnMut(&State) -> bool, mut on_state: impl FnMut(&State)) -> Result<State, String> {
    loop {
        let payload = svc::read_frame(stream).map_err(|e| format!("the update agent stopped answering: {e}"))?;
        let evt = Evt::decode(&payload).map_err(|_| "the update agent sent a message this tool does not understand".to_string())?;
        if let Evt::State(state) = evt {
            on_state(&state);
            if done(&state) {
                return Ok(state);
            }
        }
    }
}

/// Connects to the agent, subscribes to its state, and sends the commands in order.
fn connect(path: &PathBuf) -> Result<UnixStream, String> {
    let mut stream = UnixStream::connect(path).map_err(|e| format!("the update agent is not running ({}): {e}", path.display()))?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    send(&mut stream, &Cmd::Subscribe).map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Checks for updates now and returns the state once the check has finished.
pub fn check(path: &PathBuf) -> Result<State, String> {
    let mut stream = connect(path)?;
    send(&mut stream, &Cmd::Check).map_err(|e| e.to_string())?;
    let state = wait_until(&mut stream, |s| is_settled(s.phase), |_| {})?;
    if state.phase == Phase::Failed {
        return Err(state.error.clone().unwrap_or_else(|| "the update check failed".into()));
    }
    Ok(state)
}

/// Switches the background checks on or off. The agent answers commands in order, so its pong after the switch
/// means the switch is written. Returns the text to print.
pub fn schedule(path: &PathBuf, automatic: bool) -> Result<String, String> {
    let mut stream = connect(path)?;
    send(&mut stream, &Cmd::Schedule { automatic }).map_err(|e| e.to_string())?;
    send(&mut stream, &Cmd::Ping).map_err(|e| e.to_string())?;
    loop {
        let payload = svc::read_frame(&mut stream).map_err(|e| format!("the update agent stopped answering: {e}"))?;
        match Evt::decode(&payload) {
            Ok(Evt::Pong { .. }) => break,
            Ok(_) => {}
            Err(_) => return Err("the update agent sent a message this tool does not understand".into()),
        }
    }
    Ok(schedule_text(automatic))
}

/// The line `--schedule` prints for the background check preference.
pub fn schedule_text(automatic: bool) -> String {
    format!("Automatic check is {}.\n", if automatic { "on" } else { "off" })
}

/// The preference `--schedule` alone prints, read from the switch file in the shell data folder.
pub fn schedule_query(dir: &std::path::Path) -> String {
    schedule_text(springboard::shell::ota::schedule_on(dir))
}

/// Downloads and stages the offered OS update. Returns the text to print when it is staged.
pub fn install(path: &PathBuf, labels: &[String], all: bool) -> Result<String, String> {
    let mut stream = connect(path)?;
    let state = wait_until(&mut stream, |s| s.phase != Phase::Checking, |_| {})?;
    let Some(label) = os_label(&state) else {
        return Err(if all || labels.is_empty() { "No new software available.".into() } else { format!("{} is not available.", labels.join(", ")) });
    };
    if !all && !labels.iter().any(|l| *l == label) {
        return Err(format!("{} is not available. Use -l to see the labels.", labels.join(", ")));
    }
    send(&mut stream, &Cmd::Download).map_err(|e| e.to_string())?;
    let done = wait_until(&mut stream, |s| matches!(s.phase, Phase::Staged | Phase::Failed | Phase::UpToDate), |_| {})?;
    match done.phase {
        Phase::Staged => Ok(format!("Downloaded {label}. Restart BifrOSt to finish installing.\n")),
        _ => Err(done.error.unwrap_or_else(|| "the download failed".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::svc::update::Offer;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn state(offer: Option<Offer>, apps: Vec<(String, String)>) -> State {
        State { phase: Phase::Available, offer, fraction: 0.0, apps, system: vec![], error: None }
    }

    fn offer() -> Offer {
        Offer { version: "0.7.0".into(), build: None, sha256: String::new(), size: 5 * 1024 * 1024, url: String::new(), notes: None }
    }

    #[test]
    fn list_install_and_help_are_read() {
        assert_eq!(parse(&args(&["-l"])), Ok(Mode::List));
        assert_eq!(parse(&args(&["-a"])), Ok(Mode::Install { labels: vec![], all: true }));
        assert_eq!(parse(&args(&["-i", "BifrOSt 0.7.0"])), Ok(Mode::Install { labels: vec!["BifrOSt 0.7.0".into()], all: false }));
        assert_eq!(parse(&args(&["-h"])), Ok(Mode::Help));
        assert_eq!(parse(&args(&[])), Err(USAGE.to_string()));
    }

    #[test]
    fn schedule_switches_the_background_checks_and_takes_on_or_off() {
        assert_eq!(parse(&args(&["--schedule", "on"])), Ok(Mode::Schedule { automatic: true }));
        assert_eq!(parse(&args(&["--schedule", "off"])), Ok(Mode::Schedule { automatic: false }));
        assert_eq!(parse(&args(&["--schedule"])), Ok(Mode::ScheduleQuery));
        assert!(parse(&args(&["--schedule", "maybe"])).unwrap_err().contains("maybe"));
        assert!(parse(&args(&["-l", "--schedule", "off"])).unwrap_err().contains("cannot be combined"));
        assert!(parse(&args(&["--schedule", "-l"])).unwrap_err().contains("cannot be combined"));
    }

    #[test]
    fn mixed_flags_are_refused_with_reasons() {
        assert!(parse(&args(&["-l", "-a"])).unwrap_err().contains("cannot be combined"));
        assert!(parse(&args(&["-x"])).unwrap_err().contains("-x"));
    }

    #[test]
    fn the_socket_follows_the_environment_then_the_data_folder() {
        assert_eq!(socket_path(Some("/tmp/s.sock"), false, None, Some("/h")), PathBuf::from("/tmp/s.sock"));
        assert_eq!(socket_path(None, true, None, None), PathBuf::from("/var/lib/springboard/run/is.olibuijr.update.sock"));
        assert_eq!(socket_path(None, false, Some("/x"), Some("/h")), PathBuf::from("/x/springboard/run/is.olibuijr.update.sock"));
        assert_eq!(socket_path(None, false, None, Some("/h")), PathBuf::from("/h/.local/share/springboard/run/is.olibuijr.update.sock"));
    }

    #[test]
    fn the_listing_names_the_os_offer_and_the_apps() {
        let s = state(Some(offer()), vec![("is.olibuijr.files.desktop".into(), "1.2.0".into())]);
        let text = list_text(&s);
        assert!(text.contains("* Label: BifrOSt 0.7.0"));
        assert!(text.contains("Size: 5120K"));
        assert!(text.contains("* Label: is.olibuijr.files.desktop-1.2.0"));
        assert_eq!(os_label(&s).as_deref(), Some("BifrOSt 0.7.0"));
    }

    #[test]
    fn an_empty_state_says_nothing_is_available() {
        assert_eq!(list_text(&state(None, vec![])), "No new software available.\n");
        assert_eq!(os_label(&state(None, vec![])), None);
    }

    #[test]
    fn schedule_prints_the_switch_file_it_is_given() {
        assert_eq!(schedule_text(true), "Automatic check is on.\n");
        let dir = std::env::temp_dir().join(format!("softwareupdate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        springboard::shell::ota::set_schedule(&dir, false).unwrap();
        assert_eq!(schedule_query(&dir), "Automatic check is off.\n");
        springboard::shell::ota::set_schedule(&dir, true).unwrap();
        assert_eq!(schedule_query(&dir), "Automatic check is on.\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_finished_phases_settle_a_wait() {
        assert!(is_settled(Phase::UpToDate));
        assert!(is_settled(Phase::Staged));
        assert!(!is_settled(Phase::Checking));
        assert!(!is_settled(Phase::Downloading));
    }
}
