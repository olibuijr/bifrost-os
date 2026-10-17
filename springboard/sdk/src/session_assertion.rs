//! Power assertions for apps: keep the machine or the screen awake while an app works (a video call, a long export,
//! a presentation). The model is the logind block inhibitor that the shell's `caffeinate` command and lid handling
//! already use: `systemd-inhibit --what=... --mode=block`. This module builds the arguments and the holding command;
//! it spawns nothing and keeps no state. The caller runs the command and stops the hold by ending that process.
//!
//! ```ignore
//! use bifrost_app::session_assertion::{PowerAssertion, Hold};
//! let argv = PowerAssertion::new(Hold::IdleAndSleep, "Exporting video", "Video Export").hold_argv(None);
//! ```

/// What a power assertion keeps from happening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Stops the idle timer from suspending or blanking the session.
    Idle,
    /// Stops system sleep (suspend and hibernate), for example while a transfer runs.
    Sleep,
    /// Both of the above.
    IdleAndSleep,
}

impl Hold {
    /// The value of `--what=` for this hold.
    pub fn what(self) -> &'static str {
        match self {
            Hold::Idle => "idle",
            Hold::Sleep => "sleep",
            Hold::IdleAndSleep => "idle:sleep",
        }
    }
}

/// One power assertion request: what to hold, and the reason and the app name the session shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerAssertion {
    hold: Hold,
    reason: String,
    who: String,
}

impl PowerAssertion {
    /// A request to hold `hold`. An empty `reason` or `who` is replaced by a fixed text so the session always has one.
    pub fn new(hold: Hold, reason: &str, who: &str) -> PowerAssertion {
        let reason = if reason.trim().is_empty() { "Running" } else { reason.trim() };
        let who = if who.trim().is_empty() { "BifrOSt app" } else { who.trim() };
        PowerAssertion { hold, reason: reason.to_string(), who: who.to_string() }
    }

    /// The hold this request takes.
    pub fn hold(&self) -> Hold {
        self.hold
    }

    /// The `systemd-inhibit` options for this request, without the program name or the command.
    pub fn inhibit_args(&self) -> Vec<String> {
        vec![
            format!("--what={}", self.hold.what()),
            format!("--who={}", self.who),
            format!("--why={}", self.reason),
            "--mode=block".to_string(),
        ]
    }

    /// The full command that holds the assertion. With no command the hold runs `sleep infinity` until the process is
    /// killed. With a command, the hold lasts as long as that command runs.
    pub fn hold_argv(&self, command: Option<&[String]>) -> Vec<String> {
        let mut argv = vec!["systemd-inhibit".to_string()];
        argv.extend(self.inhibit_args());
        match command {
            Some(cmd) if !cmd.is_empty() => argv.extend(cmd.iter().cloned()),
            _ => {
                argv.push("sleep".to_string());
                argv.push("infinity".to_string());
            }
        }
        argv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_names_match_logind() {
        assert_eq!(Hold::Idle.what(), "idle");
        assert_eq!(Hold::Sleep.what(), "sleep");
        assert_eq!(Hold::IdleAndSleep.what(), "idle:sleep");
    }

    #[test]
    fn empty_reason_and_name_get_defaults() {
        let a = PowerAssertion::new(Hold::Idle, "  ", "");
        assert!(a.inhibit_args().contains(&"--why=Running".to_string()));
        assert!(a.inhibit_args().contains(&"--who=BifrOSt app".to_string()));
    }

    #[test]
    fn inhibit_args_block_the_chosen_hold() {
        let a = PowerAssertion::new(Hold::IdleAndSleep, "Exporting video", "Video Export");
        assert_eq!(
            a.inhibit_args(),
            vec!["--what=idle:sleep", "--who=Video Export", "--why=Exporting video", "--mode=block"]
        );
    }

    #[test]
    fn hold_without_command_sleeps_forever() {
        let a = PowerAssertion::new(Hold::Sleep, "Transfer", "Files");
        let argv = a.hold_argv(None);
        assert_eq!(argv[0], "systemd-inhibit");
        assert_eq!(&argv[argv.len() - 2..], ["sleep", "infinity"]);
    }

    #[test]
    fn hold_with_command_ends_with_that_command() {
        let a = PowerAssertion::new(Hold::Idle, "Call", "Chat");
        let cmd = vec!["/usr/bin/true".to_string(), "--flag".to_string()];
        let argv = a.hold_argv(Some(&cmd));
        assert_eq!(&argv[argv.len() - 2..], ["/usr/bin/true", "--flag"]);
        assert!(!argv.contains(&"sleep".to_string()));
        assert_eq!(a.hold(), Hold::Idle);
    }
}
