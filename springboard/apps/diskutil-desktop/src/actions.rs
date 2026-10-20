//! Mount, unmount and eject, and the privileged erase and first-aid requests. Mount, unmount and eject plan through the
//! disk session in `bifrost-core` (`disk_session`, `disk_commands`), the same claims and approval gates the shell uses,
//! and run the plan's commands on a thread. Erase and first aid are `disk_request` requests: they are checked against
//! the disks here before a confirmation sheet opens, and then sent to the privileged helper (`privd`) on a thread.

use crate::model::{to_disk, Node};
use bifrost_core::disk_claims::Claims;
use bifrost_core::disk_commands::{self, Command, MountOptions, Refusal};
use bifrost_core::disk_events::{Approvals, Disk};
use bifrost_core::disk_request::{self, DiskRequest};
use bifrost_core::disk_run;
use bifrost_core::disk_scan::Entry;
use bifrost_core::disk_session::Session;
use bifrost_core::json::Json;
use bifrost_core::privd_wire;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// The helper's socket when the session does not name one (`SPRINGBOARD_PRIVD`).
const PRIVD_SOCKET: &str = "/run/bifrost/privd.sock";
/// A privileged request may run for a long time (a full erase): the helper answers when it is done.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3600);

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

/// The `file://` URL that opens a mount point in the file manager. Bytes outside the URL-safe set are percent-encoded.
pub fn mount_url(mount_point: &str) -> Option<String> {
    if !mount_point.starts_with('/') {
        return None;
    }
    let mut url = String::from("file://");
    for b in mount_point.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    Some(url)
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
    spawn_with(plan, false)
}

/// Like `spawn`. With `repairing` a checker that exits 1 after fixing something counts as a success (an image resize
/// runs `e2fsck` first).
pub fn spawn_with(plan: Vec<Command>, repairing: bool) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    DispatchQueue::global(Qos::Utility).async_(move || {
        let result = disk_run::run_plan(&plan, repairing).map(|_| ()).map_err(|e| disk_service_text(&plan, e));
        let _ = tx.send(result);
        DispatchQueue::main().async_(|| {});
    });
    rx
}

/// The sentence for a failed plan. When a `udisksctl` step could not reach the disk service (udisks2 is not running, or
/// this sandbox has no bus to it) the person is told that plainly, not shown the tool's own error line.
pub fn disk_service_text(plan: &[Command], error: String) -> String {
    let lower = error.to_lowercase();
    let unreachable = ["error connecting", "could not connect", "could not run", "not activatable", "not provided by any"].iter().any(|p| lower.contains(p));
    if unreachable && plan.iter().any(|c| c.program == "udisksctl") {
        "The disk service is not available, so this cannot be done here.".to_string()
    } else {
        error
    }
}

/// The disks as planning sees them: each row with its parent.
pub fn entries_of(nodes: &[Node]) -> Vec<Entry> {
    nodes.iter().map(|n| Entry { disk: to_disk(n), parent: n.parent.clone() }).collect()
}

/// Checks a privileged request against the disks the window shows, so a refusal (a mounted volume, a partition where
/// a disk is needed) is known before a confirmation sheet opens. The helper plans it again with its own snapshot.
pub fn check_request(request: &DiskRequest, nodes: &[Node]) -> Result<(), String> {
    let probed = nodes.iter().find(|n| n.name == request.device()).and_then(|n| n.fs_type.clone()).unwrap_or_default();
    disk_request::plan(request, &entries_of(nodes), &probed).map(|_| ()).map_err(|refusal| refusal.sentence())
}

/// Sends a privileged request to the helper on a new thread. The receiver gets Ok when the helper ran it, else the
/// sentence for the footer.
pub fn spawn_request(request: DiskRequest) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    DispatchQueue::global(Qos::Utility).async_(move || {
        let _ = tx.send(send_request(&request));
        DispatchQueue::main().async_(|| {});
    });
    rx
}

/// One request over the helper's socket. The session token is the shell's to hold, so this program sends none and
/// the helper answers the unlock-only methods to it with `unauthorized`; that is shown as a sentence, not a code.
fn send_request(request: &DiskRequest) -> Result<(), String> {
    let socket = std::env::var("SPRINGBOARD_PRIVD").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| PRIVD_SOCKET.to_string());
    let fields = match request.to_params() {
        Json::Obj(fields) => fields,
        _ => Vec::new(),
    };
    privd_wire::call(Path::new(&socket), None, request.method(), fields, REQUEST_TIMEOUT).map(|_| ()).map_err(|e| match e.code.as_str() {
        "unauthorized" | "locked" => "The disk helper needs an unlocked BifrOSt session, and this app does not have one.".to_string(),
        "unavailable" => "The disk helper is not running.".to_string(),
        _ => e.message,
    })
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
    fn an_unreachable_disk_service_is_named_plainly() {
        let plan = vec![Command { program: "udisksctl", args: vec!["mount".to_string()] }];
        let said = disk_service_text(&plan, "Error connecting: Could not connect: No such file or directory".to_string());
        assert_eq!(said, "The disk service is not available, so this cannot be done here.");
        assert_eq!(disk_service_text(&plan, "Error mounting /dev/sdb1: busy".to_string()), "Error mounting /dev/sdb1: busy");
        let other = vec![Command { program: "mkfs.ext4", args: vec![] }];
        assert_eq!(disk_service_text(&other, "could not run".to_string()), "could not run");
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
    fn a_mount_point_becomes_an_encoded_file_url() {
        assert_eq!(mount_url("/run/media/me/USB DISK").as_deref(), Some("file:///run/media/me/USB%20DISK"));
        assert_eq!(mount_url("relative"), None);
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
        assert!(Refusal::Mounted("/mnt".into()).sentence().contains("/mnt"));
        assert!(Refusal::NoDisk("sdq".into()).sentence().contains("sdq"));
    }
}
