//! `caffeinate`: keeps the machine awake while a command runs, for a time, or until a process exits. The inhibitors are
//! logind block inhibitors taken through `systemd-inhibit`, the same mechanism the shell uses for its lid inhibitor
//! (`bare::sleep::LidInhibitor`): `-i` blocks idle sleep (`--what=idle`) and `-s` blocks system sleep (`--what=sleep`).
//! With neither flag, `-i` is held, as the man page says.
//!
//! Supported: `-i`, `-s`, `-t seconds` (ends the hold after the time), `-w pid` (ends it when the process exits), and a
//! utility to run under the hold (its exit status is passed on). With a utility, `-t` and `-w` are ignored, as the reference tool does.
//! Refused: `-d` (display sleep) and `-u` (user activity), which need a display assertion the desktop does not keep yet,
//! and `-m` (disk idle), which logind has no block for.
//! Gap: `-s` is held on battery as well as on mains power; the man page asks for mains only.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const USAGE: &str = "usage: caffeinate [-i] [-s] [-t seconds] [-w pid] [utility [arguments ...]]";
const WHO: &str = "--who=caffeinate";
const WHY: &str = "--why=caffeinate is keeping the system awake";
/// How often `-w` checks whether its process is still there.
const POLL: Duration = Duration::from_millis(500);

/// The parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `-i`: block idle sleep.
    pub idle: bool,
    /// `-s`: block system sleep.
    pub system: bool,
    /// `-t`: seconds to hold for.
    pub timeout: Option<u64>,
    /// `-w`: a process to hold for.
    pub wait_pid: Option<u32>,
    /// The utility and its arguments, when one is given.
    pub command: Vec<String>,
}

/// What the command line asks for: the help text, or a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments. Flags bundle (`-is`), value options take the next argument, and the first word that is not an
/// option starts the utility, whose own arguments are passed through untouched.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut it = args.iter().enumerate();
    while let Some((i, arg)) = it.next() {
        if arg == "-h" || arg == "-help" || arg == "--help" {
            return Ok(Parsed::Help);
        }
        if !arg.starts_with('-') || arg == "-" {
            o.command = args[i..].to_vec();
            break;
        }
        if arg == "--" {
            o.command = args[i + 1..].to_vec();
            break;
        }
        let mut chars = arg[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                'i' => o.idle = true,
                's' => o.system = true,
                't' | 'w' => {
                    let rest: String = chars.by_ref().collect();
                    let value = if rest.is_empty() { it.next().map(|(_, v)| v.clone()).ok_or(format!("-{c} needs a value"))? } else { rest };
                    if c == 't' {
                        o.timeout = Some(value.trim().parse().map_err(|_| format!("-t needs a number of seconds, not {value}"))?);
                    } else {
                        o.wait_pid = Some(value.trim().parse().map_err(|_| format!("-w needs a process id, not {value}"))?);
                    }
                    break;
                }
                'd' | 'u' => return Err(format!("-{c}: the display and user-activity holds need an assertion the desktop does not keep yet")),
                'm' => return Err("-m: there is no disk-idle block to hold on this desktop".into()),
                other => return Err(format!("unknown option -{other}")),
            }
        }
    }
    if !o.idle && !o.system {
        o.idle = true;
    }
    Ok(Parsed::Run(o))
}

/// The `systemd-inhibit` arguments for the holds in `o`, ending before the command.
pub fn inhibit_args(o: &Options) -> Vec<String> {
    let mut what = Vec::new();
    if o.idle {
        what.push("idle");
    }
    if o.system {
        what.push("sleep");
    }
    vec![format!("--what={}", what.join(":")), WHO.into(), WHY.into(), "--mode=block".into()]
}

/// The full argument list (program first) for holding with `command` after the inhibitor, or with `sleep` for the time.
pub fn hold_argv(o: &Options) -> Vec<String> {
    let mut argv = vec!["systemd-inhibit".to_string()];
    argv.extend(inhibit_args(o));
    if o.command.is_empty() {
        argv.push("sleep".into());
        argv.push(o.timeout.map_or("infinity".into(), |t| t.to_string()));
    } else {
        argv.extend(o.command.iter().cloned());
    }
    argv
}

/// Whether process `pid` exists now.
pub fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn spawn(argv: &[String]) -> Result<Child, String> {
    Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn().map_err(|e| format!("{}: {e}", argv[0]))
}

/// Holds the sleep blocks as `o` asks and returns the exit status to pass on: a utility's own status, else 0.
pub fn run(o: &Options) -> Result<i32, String> {
    if !o.command.is_empty() {
        let mut child = spawn(&hold_argv(o))?;
        let status = child.wait().map_err(|e| e.to_string())?;
        return Ok(status.code().unwrap_or(1));
    }
    let mut child = spawn(&hold_argv(o))?;
    if let Some(pid) = o.wait_pid {
        while pid_alive(pid) && child.try_wait().map_err(|e| e.to_string())?.is_none() {
            std::thread::sleep(POLL);
        }
        let _ = child.kill();
    }
    child.wait().map_err(|e| e.to_string())?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn run_of(v: &[&str]) -> Options {
        match parse(&args(v)).unwrap() {
            Parsed::Run(o) => o,
            Parsed::Help => panic!("help"),
        }
    }

    #[test]
    fn idle_is_held_when_no_flag_is_given() {
        let o = run_of(&[]);
        assert!(o.idle && !o.system);
        assert_eq!(inhibit_args(&o)[0], "--what=idle");
    }

    #[test]
    fn flags_bundle_and_map_to_logind_blocks() {
        let o = run_of(&["-is"]);
        assert!(o.idle && o.system);
        assert_eq!(inhibit_args(&o)[0], "--what=idle:sleep");
        let s = run_of(&["-s"]);
        assert!(!s.idle, "-s alone holds only system sleep");
        assert_eq!(inhibit_args(&s)[0], "--what=sleep");
    }

    #[test]
    fn timeout_and_pid_take_values() {
        let o = run_of(&["-t", "90", "-w", "4321"]);
        assert_eq!((o.timeout, o.wait_pid), (Some(90), Some(4321)));
        assert_eq!(hold_argv(&o)[5..], args(&["sleep", "90"])[..]);
        let attached = run_of(&["-t30"]);
        assert_eq!(attached.timeout, Some(30));
        assert_eq!(hold_argv(&run_of(&[]))[5..], args(&["sleep", "infinity"])[..]);
    }

    #[test]
    fn a_utility_ends_the_options_and_keeps_its_arguments() {
        let o = run_of(&["-i", "make", "-j4", "-t"]);
        assert_eq!(o.command, args(&["make", "-j4", "-t"]));
        assert_eq!(o.timeout, None, "-t after the utility belongs to the utility");
        let argv = hold_argv(&o);
        assert_eq!(argv[argv.len() - 3..], args(&["make", "-j4", "-t"])[..]);
        assert_eq!(run_of(&["--", "-i"]).command, args(&["-i"]));
    }

    #[test]
    fn display_user_and_disk_holds_are_refused_by_name() {
        assert!(parse(&args(&["-d"])).unwrap_err().contains("-d"));
        assert!(parse(&args(&["-u"])).unwrap_err().contains("user-activity"));
        assert!(parse(&args(&["-m"])).unwrap_err().contains("disk-idle"));
        assert!(parse(&args(&["-t", "soon"])).unwrap_err().contains("number of seconds"));
        assert!(parse(&args(&["-w", "x"])).unwrap_err().contains("process id"));
        assert!(parse(&args(&["-q"])).unwrap_err().contains("unknown option"));
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
    }

    #[test]
    fn a_process_that_does_not_exist_is_not_alive() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(u32::MAX - 1));
    }
}
