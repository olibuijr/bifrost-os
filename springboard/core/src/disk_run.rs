//! Runs a disk plan (`disk_commands::Command` list) in order. Each command gets its own argument words, never a shell.
//! The desktop app and the `diskutil` tool run plans as the user (mount, unmount, eject); the privileged helper runs
//! the erase, partition and first-aid plans from `disk_format` as root.

use crate::disk_commands::Command;

/// Runs `plan` in order and stops at the first failure. Returns the program names that completed, so a caller can
/// show progress. The error is the failing tool's last stderr line, or a fallback that names the tool.
///
/// `repairing` is set for a first-aid repair. A repairing checker exits 1 when it fixed something, which is success,
/// the same as its preen mode; a verify (`-n`) that finds errors exits 1 as well and is a failure.
pub fn run_plan(plan: &[Command], repairing: bool) -> Result<Vec<&'static str>, String> {
    let mut done = Vec::with_capacity(plan.len());
    for command in plan {
        let out = std::process::Command::new(command.program)
            .args(&command.args)
            .output()
            .map_err(|e| format!("{} could not run: {e}", command.program))?;
        let checker = command.program.starts_with("e2fsck") || command.program.starts_with("fsck");
        let fixed = repairing && checker && out.status.code() == Some(1);
        if !out.status.success() && !fixed {
            return Err(failure_text(command.program, &String::from_utf8_lossy(&out.stderr)));
        }
        done.push(command.program);
    }
    Ok(done)
}

/// The text a failed tool reports: its last non-empty stderr line, or the program name when it wrote nothing.
pub fn failure_text(program: &str, stderr: &str) -> String {
    match stderr.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(line) => line.trim().to_string(),
        None => format!("{program} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_text_is_the_last_stderr_line() {
        assert_eq!(failure_text("mkfs.ext4", "mke2fs 1.47\nno such device\n\n"), "no such device");
        assert_eq!(failure_text("sgdisk", "  "), "sgdisk failed");
    }

    #[test]
    fn an_empty_plan_completes_with_no_steps() {
        assert_eq!(run_plan(&[], false), Ok(vec![]));
    }

    #[test]
    fn a_missing_tool_is_reported_by_name() {
        let plan = [Command { program: "bifrost-no-such-tool", args: vec![] }];
        let err = run_plan(&plan, false).unwrap_err();
        assert!(err.starts_with("bifrost-no-such-tool could not run"), "{err}");
    }
}
