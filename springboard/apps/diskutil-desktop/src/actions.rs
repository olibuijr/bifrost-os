//! Mount, unmount and eject. The plans come from the disk session in `bifrost-core` (`disk_session`, `disk_commands`):
//! the same claims and approval gates the shell uses. This module feeds the session the disks it read, asks it for a
//! plan, and runs the plan's commands on a thread so the window keeps drawing. There are no erase or partition
//! operations in this app.

use crate::model::{to_disk, Node};
use bifrost_core::disk_claims::Claims;
use bifrost_core::disk_commands::{self, Command, MountOptions, Refusal};
use bifrost_core::disk_events::{Approvals, Disk};
use bifrost_core::disk_session::Session;
use std::sync::mpsc::{channel, Receiver};

/// The owner name this app claims devices under.
pub const OWNER: &str = "is.olibuijr.diskutil.desktop";

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Mount,
    Unmount,
    Eject,
    /// Unmounts a disk image and releases its loop device (`hdiutil detach`).
    Detach,
}

impl Action {
    /// The progress word shown while the action runs.
    pub fn verb(self) -> &'static str {
        match self {
            Action::Mount => "Mounting",
            Action::Unmount => "Unmounting",
            Action::Eject => "Ejecting",
            Action::Detach => "Detaching",
        }
    }
}

/// A loop device name: `loop` and then digits.
fn is_loop_name(name: &str) -> bool {
    name.strip_prefix("loop").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The disks the session should know about: every disk and partition the window shows.
pub fn disks_of(nodes: &[Node]) -> Vec<Disk> {
    nodes.iter().map(to_disk).collect()
}

/// Plans an action on the device `name`. For an eject, `mounted_partitions` are the partitions of the disk that are
/// mounted: each is unmounted first, then the disk is powered off. Returns the commands, or why the session refused.
pub fn plan(action: Action, disks: &[Disk], name: &str, mounted_partitions: &[String]) -> Result<Vec<Command>, Refusal> {
    let mut session = Session::new();
    session.observe(disks.to_vec());
    match action {
        Action::Mount => {
            let disk = session.disk(name).ok_or_else(|| Refusal::NoDisk(name.to_string()))?;
            disk_commands::mount(disk, &MountOptions::default(), &Approvals::new(), &Claims::default(), OWNER)
        }
        Action::Unmount => session.unmount(name, OWNER),
        Action::Eject => {
            let mut commands = Vec::new();
            for part in mounted_partitions {
                commands.extend(session.unmount(part, OWNER)?);
            }
            commands.extend(session.eject(name, OWNER)?);
            Ok(commands)
        }
        Action::Detach => {
            if !is_loop_name(name) {
                return Err(Refusal::InvalidDevice(name.to_string()));
            }
            // the image tool unmounts the device and releases the loop device, the same command the desktop's control line uses
            Ok(vec![Command { program: "hdiutil", args: vec!["detach".to_string(), format!("/dev/{name}")] }])
        }
    }
}

/// Runs the commands in order on a new thread. The receiver gets one result: Ok when every command succeeded, else the
/// first error text (the tool's stderr when it has one).
pub fn spawn(plan: Vec<Command>) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = run(&plan);
        let _ = tx.send(result);
    });
    rx
}

fn run(plan: &[Command]) -> Result<(), String> {
    for command in plan {
        let out = std::process::Command::new(command.program)
            .args(&command.args)
            .output()
            .map_err(|e| format!("{} could not run: {e}", command.program))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if stderr.is_empty() { format!("{} failed", command.program) } else { stderr });
        }
    }
    Ok(())
}

/// A sentence for a refusal, shown in the window's footer.
pub fn refusal_text(refusal: &Refusal) -> String {
    match refusal {
        Refusal::InvalidDevice(name) => format!("\u{201c}{name}\u{201d} is not a device name this app can use."),
        Refusal::InvalidOption(option) => format!("The file system option \u{201c}{option}\u{201d} is not valid."),
        Refusal::InvalidLabel(label) => format!("The label \u{201c}{label}\u{201d} is not valid for this file system."),
        Refusal::Claimed { device, owner } => format!("{device} is in use by {owner}."),
        Refusal::Vetoed(reason) => reason.clone(),
        Refusal::Mounted(point) => format!("The volume is mounted at {point}. Unmount it first."),
        Refusal::NotMounted => "The volume is not mounted.".to_string(),
        Refusal::ReadOnly => "The volume is read-only.".to_string(),
        Refusal::NoDisk(name) => format!("{name} is no longer connected."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str, mount: Option<&str>, removable: bool) -> Disk {
        Disk { bsd_name: name.into(), size: 1000, removable, read_only: false, model: String::new(), mount_point: mount.map(String::from) }
    }

    fn tools(plan: &[Command]) -> Vec<(&'static str, String)> {
        plan.iter().map(|c| (c.program, c.args.join(" "))).collect()
    }

    #[test]
    fn mount_plans_one_udisks_call() {
        let disks = vec![disk("sdb1", None, true)];
        let plan = plan(Action::Mount, &disks, "sdb1", &[]).unwrap();
        assert_eq!(tools(&plan), vec![("udisksctl", "mount -b /dev/sdb1".to_string())]);
    }

    #[test]
    fn mount_of_a_mounted_volume_is_refused() {
        let disks = vec![disk("sdb1", Some("/mnt/x"), true)];
        assert_eq!(plan(Action::Mount, &disks, "sdb1", &[]), Err(Refusal::Mounted("/mnt/x".into())));
    }

    #[test]
    fn unmount_needs_a_mounted_volume() {
        let disks = vec![disk("sdb1", None, true)];
        assert_eq!(plan(Action::Unmount, &disks, "sdb1", &[]), Err(Refusal::NotMounted));
        let mounted = vec![disk("sdb1", Some("/mnt/x"), true)];
        assert_eq!(tools(&plan(Action::Unmount, &mounted, "sdb1", &[]).unwrap()), vec![("udisksctl", "unmount -b /dev/sdb1".to_string())]);
    }

    #[test]
    fn eject_unmounts_partitions_then_powers_off() {
        let disks = vec![disk("sdb", None, true), disk("sdb1", Some("/mnt/x"), true)];
        let plan = plan(Action::Eject, &disks, "sdb", &["sdb1".to_string()]).unwrap();
        assert_eq!(
            tools(&plan),
            vec![("udisksctl", "unmount -b /dev/sdb1".to_string()), ("udisksctl", "power-off -b /dev/sdb".to_string())]
        );
    }

    #[test]
    fn detach_releases_a_loop_device_through_the_image_tool() {
        let plan = plan(Action::Detach, &[], "loop3", &[]).unwrap();
        assert_eq!(tools(&plan), vec![("hdiutil", "detach /dev/loop3".to_string())]);
        assert!(matches!(plan_detach_refusal(), Some(Refusal::InvalidDevice(_))));
    }

    fn plan_detach_refusal() -> Option<Refusal> {
        plan(Action::Detach, &[], "sdb", &[]).err()
    }

    #[test]
    fn an_unknown_device_is_refused() {
        assert_eq!(plan(Action::Mount, &[], "sdz", &[]), Err(Refusal::NoDisk("sdz".into())));
    }

    #[test]
    fn a_device_name_with_a_slash_never_reaches_a_command() {
        let disks = vec![disk("../sda", None, true)];
        assert!(matches!(plan(Action::Mount, &disks, "../sda", &[]), Err(Refusal::InvalidDevice(_))));
    }

    #[test]
    fn every_refusal_has_a_sentence() {
        assert!(refusal_text(&Refusal::Mounted("/mnt".into())).contains("/mnt"));
        assert!(refusal_text(&Refusal::NoDisk("sdq".into())).contains("sdq"));
    }
}
