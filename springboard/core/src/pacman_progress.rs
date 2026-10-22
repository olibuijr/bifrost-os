//! Install progress for the Software Update and package pages. pacman does not report progress to the shell, but it
//! writes every step of a transaction to its log (`/var/log/pacman.log`): `transaction started`, then one `[ALPM]`
//! line per package (`installed`, `upgraded`, `removed`, `reinstalled`), then `transaction completed`. This module reads
//! the steps of the latest transaction from that text, so the shell can show how far along it is. Pure: the caller
//! reads the file.

/// What pacman did to one package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Installed,
    Upgraded,
    Removed,
    Reinstalled,
}

/// One package step of a transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub action: Action,
    pub package: String,
}

/// The `[ALPM]` line that opens a transaction in the log.
pub const TRANSACTION_STARTED: &str = "transaction started";
/// The `[ALPM]` line that closes a transaction in the log.
pub const TRANSACTION_COMPLETED: &str = "transaction completed";

/// One `[ALPM]` package line, `None` for any other line. The package is the word after the action.
pub fn parse_step(line: &str) -> Option<Step> {
    let rest = line.split_once("[ALPM] ")?.1;
    let (action, rest) = rest.split_once(' ')?;
    let action = match action {
        "installed" => Action::Installed,
        "upgraded" => Action::Upgraded,
        "removed" => Action::Removed,
        "reinstalled" => Action::Reinstalled,
        _ => return None,
    };
    let package = rest.split_whitespace().next()?.to_string();
    Some(Step { action, package })
}

/// The package steps of the latest transaction in `log`: the lines after the last `transaction started`, up to a
/// `transaction completed` after it when there is one. An empty log, or one with no transaction, gives no steps.
pub fn latest_steps(log: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut started = false;
    for line in log.lines() {
        if line.contains(TRANSACTION_STARTED) {
            steps.clear();
            started = true;
        } else if line.contains(TRANSACTION_COMPLETED) {
            started = false;
        } else if started {
            if let Some(step) = parse_step(line) {
                steps.push(step);
            }
        }
    }
    steps
}

/// Where the latest transaction in the log is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The log holds no transaction.
    Waiting,
    /// A transaction started and has not ended.
    Running,
    /// The latest transaction ended with `transaction completed`.
    Completed,
    /// The latest transaction ended with an error.
    Failed,
}

/// What the latest transaction has done: its phase, its package steps and a short line for the step it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    pub steps: Vec<Step>,
    /// The last thing the log shows, for example `Installed foo` or `Running 30-update.hook`, for the sheet to name
    /// beside the progress bar. `None` before anything has happened.
    pub last: Option<String>,
}

/// The sentence for one finished step, such as `Upgraded linux`.
pub fn describe(step: &Step) -> String {
    let verb = match step.action {
        Action::Installed => "Installed",
        Action::Upgraded => "Upgraded",
        Action::Removed => "Removed",
        Action::Reinstalled => "Reinstalled",
    };
    format!("{verb} {}", step.package)
}

/// The hook named by a `[ALPM] running 'NAME'...` line (hooks run before and after the package steps), `None` for any
/// other line.
pub fn parse_hook(line: &str) -> Option<String> {
    let rest = line.split_once("[ALPM] running '")?.1;
    let name = rest.split('\'').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether a log line ends a transaction without finishing it: pacman writes `transaction failed`, or an
/// `error: failed to commit transaction` line when it cannot commit.
fn is_failure(line: &str) -> bool {
    line.contains("transaction failed") || line.contains("failed to commit transaction")
}

/// The status of the latest transaction in `log`: the lines after the last `transaction started`, their package
/// steps, hooks and how it ended. The steps are those `latest_steps` gives.
pub fn latest_status(log: &str) -> Status {
    let mut status = Status { phase: Phase::Waiting, steps: Vec::new(), last: None };
    for line in log.lines() {
        if line.contains(TRANSACTION_STARTED) {
            status = Status { phase: Phase::Running, steps: Vec::new(), last: None };
        } else if status.phase == Phase::Waiting {
            continue;
        } else if line.contains(TRANSACTION_COMPLETED) {
            status.phase = Phase::Completed;
        } else if status.phase == Phase::Running {
            if is_failure(line) {
                status.phase = Phase::Failed;
            } else if let Some(step) = parse_step(line) {
                status.last = Some(describe(&step));
                status.steps.push(step);
            } else if let Some(hook) = parse_hook(line) {
                status.last = Some(format!("Running {hook}"));
            }
        }
    }
    status
}

/// The line the install sheet shows beside its progress bar: what the running transaction did last. `None` when no
/// transaction runs, so a finished one does not keep naming its last package.
pub fn working_label(log: &str) -> Option<String> {
    let status = latest_status(log);
    (status.phase == Phase::Running).then_some(status.last).flatten()
}

/// How far a transaction is, as a fraction from 0 to 1: `done` of `expected` steps. A transaction with no steps
/// expected is complete.
pub fn fraction(done: usize, expected: usize) -> f32 {
    if expected == 0 {
        return 1.0;
    }
    (done as f32 / expected as f32).clamp(0.0, 1.0)
}

/// The progress of the latest transaction in `log` against the `expected` packages the sheet listed, from 0 to 1.
pub fn progress(log: &str, expected: usize) -> f32 {
    fraction(latest_steps(log).len(), expected)
}

/// Whether an install of `packages` waits for the person's confirmation. pacman runs each package's install script as
/// it installs the package, so any install runs code the person has not seen: the sheet asks before the first one.
pub fn confirmation_needed(packages: &[String]) -> bool {
    !packages.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "[2026-10-01T12:00:00+0000] [ALPM] transaction started\n\
[2026-10-01T12:00:01+0000] [ALPM] upgraded linux (6.1.1-1 -> 6.1.2-1)\n\
[2026-10-01T12:00:02+0000] [ALPM] installed foo (1.0-1)\n\
[2026-10-01T12:00:03+0000] [ALPM] transaction completed\n\
[2026-10-02T09:30:00+0000] [ALPM] transaction started\n\
[2026-10-02T09:30:01+0000] [ALPM] removed bar (0.9-2)\n\
[2026-10-02T09:30:02+0000] [ALPM] reinstalled baz (2.0-1)\n";

    #[test]
    fn a_package_line_gives_its_action_and_name() {
        assert_eq!(
            parse_step("[2026-10-01T12:00:02+0000] [ALPM] installed foo (1.0-1)"),
            Some(Step { action: Action::Installed, package: "foo".into() })
        );
        assert_eq!(parse_step("[2026-10-01T12:00:00+0000] [PACMAN] Running 'pacman -Syu'"), None);
        assert_eq!(parse_step("[ALPM] mystery pkg"), None);
    }

    #[test]
    fn the_latest_transaction_is_the_steps_after_its_start() {
        let steps = latest_steps(LOG);
        assert_eq!(steps.len(), 2, "the first transaction is finished and does not count");
        assert_eq!(steps[0], Step { action: Action::Removed, package: "bar".into() });
        assert_eq!(steps[1].action, Action::Reinstalled);
    }

    #[test]
    fn a_running_transaction_counts_the_steps_so_far() {
        let running = "[ALPM] transaction started\n[ALPM] installed a (1)\n[ALPM] installed b (1)\n";
        assert_eq!(latest_steps(running).len(), 2);
    }

    #[test]
    fn no_transaction_means_no_steps() {
        assert!(latest_steps("").is_empty());
        assert!(latest_steps("[ALPM] installed a (1)\n").is_empty(), "a step outside a transaction is not counted");
    }

    #[test]
    fn progress_is_the_share_of_steps_done_and_complete_when_none_are_expected() {
        assert_eq!(fraction(1, 4), 0.25);
        assert_eq!(fraction(9, 4), 1.0, "never past complete");
        assert_eq!(fraction(0, 0), 1.0);
    }

    #[test]
    fn the_sheet_progress_counts_the_latest_transaction_against_the_expected_packages() {
        let log = "[2026-10-01T12:00:00+0000] [ALPM] transaction started\n[2026-10-01T12:00:01+0000] [ALPM] installed a (1-1)\n[2026-10-01T12:00:02+0000] [ALPM] upgraded b (1-1 -> 1-2)\n";
        assert_eq!(progress(log, 4), 0.5, "two of four packages so far");
        assert_eq!(progress(log, 0), 1.0, "nothing expected is complete");
        assert_eq!(progress("", 3), 0.0, "no transaction yet");
    }

    #[test]
    fn the_status_names_the_phase_and_the_last_step() {
        assert_eq!(latest_status("").phase, Phase::Waiting);
        let st = latest_status(LOG);
        assert_eq!(st.phase, Phase::Running, "the second transaction has not completed");
        assert_eq!(st.steps, latest_steps(LOG));
        assert_eq!(st.last.as_deref(), Some("Reinstalled baz"));
        let done = latest_status("[ALPM] transaction started\n[ALPM] installed a (1)\n[ALPM] transaction completed\n");
        assert_eq!(done.phase, Phase::Completed);
        assert_eq!(done.steps.len(), 1);
    }

    #[test]
    fn a_hook_is_named_while_it_runs() {
        assert_eq!(parse_hook("[2026-10-01T12:00:04+0000] [ALPM] running '30-systemd-update.hook'..."), Some("30-systemd-update.hook".to_string()));
        assert_eq!(parse_hook("[ALPM] installed a (1)"), None);
        assert_eq!(parse_hook("[ALPM] running ''..."), None);
        let log = "[ALPM] transaction started\n[ALPM] installed a (1)\n[ALPM] running '90-mkinitcpio.hook'...\n";
        let st = latest_status(log);
        assert_eq!(st.last.as_deref(), Some("Running 90-mkinitcpio.hook"));
        assert_eq!(st.steps.len(), 1, "a hook is not a package step");
        assert_eq!(working_label(log).as_deref(), Some("Running 90-mkinitcpio.hook"));
    }

    #[test]
    fn a_failed_transaction_is_reported_and_stops_naming_steps() {
        let log = "[ALPM] transaction started\n[ALPM] installed a (1)\n[ALPM] error: failed to commit transaction (conflicting files)\n[ALPM] installed late (1)\n";
        let st = latest_status(log);
        assert_eq!(st.phase, Phase::Failed);
        assert_eq!(st.steps.len(), 1, "nothing after the failure counts");
        assert_eq!(working_label(log), None);
        assert_eq!(latest_status("[ALPM] transaction started\n[ALPM] transaction failed\n").phase, Phase::Failed);
    }

    #[test]
    fn the_working_label_is_only_for_a_running_transaction() {
        assert_eq!(working_label(""), None);
        assert_eq!(working_label("[ALPM] transaction started\n"), None, "started, nothing done yet");
        assert_eq!(
            working_label("[ALPM] transaction started\n[ALPM] upgraded linux (1 -> 2)\n").as_deref(),
            Some("Upgraded linux")
        );
        assert_eq!(working_label("[ALPM] transaction started\n[ALPM] installed a (1)\n[ALPM] transaction completed\n"), None);
    }

    #[test]
    fn an_install_waits_for_confirmation_before_its_scripts_run() {
        assert!(confirmation_needed(&["a".to_string()]));
        assert!(!confirmation_needed(&[]), "nothing to install, nothing to confirm");
    }
}
