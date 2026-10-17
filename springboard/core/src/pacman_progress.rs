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

/// How far a transaction is, as a fraction from 0 to 1: `done` of `expected` steps. A transaction with no steps
/// expected is complete.
pub fn fraction(done: usize, expected: usize) -> f32 {
    if expected == 0 {
        return 1.0;
    }
    (done as f32 / expected as f32).clamp(0.0, 1.0)
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
}
