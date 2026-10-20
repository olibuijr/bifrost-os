//! The `disks` service: the block devices the kernel reports, pushed to apps as they appear, disappear or change
//! (mounted, unmounted, resized). It is the shell side of Disk Arbitration (`DARegisterDiskAppearedCallback` and its
//! two siblings); the SDK side is `bifrost_app::disk_arbitration`. The model is `bifrost_core::disk_session`.
//!
//! The shell scans `/sys/block` and `/proc/mounts` (`bifrost_core::disk_scan`) every [`SCAN_EVERY_MS`]. The first scan
//! only records the state; an app that wants the disks already there calls `list`.
//!
//! Methods (JSON params):
//! - `list` -> `{disks: [disk]}`, a disk being `bifrost_core::disk_wire::encode_disk`
//! - `volumes` -> `{volumes: [volume]}`, every disk and partition read now, with its parent disk, file system type and
//!   label (`bifrost_core::disk_wire::encode_volume`); Disk Utility's sandbox has no `/sys`, so it asks for these
//! - `describe` `{bsd_name}` -> `{description: {key: text}}` (`disk_events::description`)
//! - `plan_unmount` / `plan_eject` `{bsd_name}` -> `{commands: [{program, args}]}`, the plan after the claim, approval
//!   and device checks. The service validates and plans; it does not run the commands yet (they need the privileged
//!   helper, which still has to be tried on Midget with a real USB disk).
//! - `claim` / `unclaim` `{bsd_name}` -> `{ok}`: exclusive use of a disk for the calling app.
//!
//! Events (to every app, from `bifrost_core::disk_wire`): `disk_appeared`, `disk_disappeared`,
//! `disk_description_changed`.
//!
//! When a removable whole disk (not a partition) appears, the shell also posts a "USB Disk Connected" notice in
//! Settings. The notice is for the arrival only: the disk's partitions and the eject that follows get no notice.
//!
//! The same arrival starts a probe on a thread (`bifrost_core::disk_magic`): the first sectors of each partition (or
//! of the disk itself when it has none) are read and matched against the filesystems the system mounts. When a
//! volume matches none (and is not an encrypted volume or swap), the shell posts a "Disk Not Readable" notice that
//! points at Disk Utility, the counterpart of the reference's prompt for a disk it cannot read.
use super::{Event, Shell, SystemService};
use crate::shell::apps::AppId;
use crate::shell::notify::store::Notification;
use bifrost_core::disk_events::{Disk, DiskEvent};
use bifrost_core::disk_magic::{self, Verdict};
use bifrost_core::disk_scan;
use bifrost_core::disk_session::Session;
use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// The shell clock `t` of a service tick is in milliseconds. How often the device list is read again.
pub const SCAN_EVERY_MS: f64 = 2_000.0;

pub struct Disks {
    session: Session,
    sys_block: PathBuf,
    mounts: PathBuf,
    /// the udev database the volumes call reads file system types and labels from
    udev: PathBuf,
    /// Whether a snapshot has been taken; the first one is recorded without events.
    primed: bool,
    next_scan: f64,
    /// The removable whole disks that appeared in the last read, waiting for their notice.
    arrivals: Vec<Disk>,
    /// where the device nodes are (`/dev`)
    dev: PathBuf,
    /// the probes running for arrived disks: each sends the disk's label and a verdict per volume
    probes: Vec<Receiver<(String, Vec<(String, Verdict)>)>>,
}

impl Default for Disks {
    fn default() -> Self {
        Self::at("/sys/block", "/proc/mounts")
    }
}

impl Disks {
    /// A service reading the device tree at `sys_block` and the mount table file at `mounts`.
    pub fn at(sys_block: impl Into<PathBuf>, mounts: impl Into<PathBuf>) -> Self {
        Disks {
            session: Session::new(),
            sys_block: sys_block.into(),
            mounts: mounts.into(),
            udev: PathBuf::from("/run/udev/data"),
            primed: false,
            next_scan: 0.0,
            arrivals: Vec::new(),
            dev: PathBuf::from("/dev"),
            probes: Vec::new(),
        }
    }

    /// Starts reading the volumes of `disk` on a thread, so a flaky medium cannot stall the shell.
    fn probe(&mut self, disk: &Disk) {
        let mut volumes = disk_magic::partitions_of(&self.sys_block, &disk.bsd_name);
        if volumes.is_empty() {
            volumes.push(disk.bsd_name.clone());
        }
        let label = if disk.model.is_empty() { disk.bsd_name.clone() } else { disk.model.clone() };
        let dev = self.dev.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let results = volumes.into_iter().filter_map(|v| disk_magic::classify(&dev.join(&v)).map(|verdict| (v, verdict))).collect();
            let _ = tx.send((label, results));
        });
        self.probes.push(rx);
    }

    /// The finished probes' sentences for volumes that cannot be read.
    fn unreadable_notices(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.probes.retain(|rx| match rx.try_recv() {
            Ok((label, results)) => {
                out.extend(disk_magic::unreadable_notice(&label, &results));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        out
    }

    /// Reads the devices now and returns the events since the last read (none the first time).
    pub fn rescan(&mut self) -> Vec<Event> {
        let text = std::fs::read_to_string(&self.mounts).unwrap_or_default();
        let snapshot = disk_scan::scan(&self.sys_block, &text).into_iter().map(|e| e.disk).collect();
        let events = self.session.observe(snapshot);
        if !std::mem::replace(&mut self.primed, true) {
            return Vec::new();
        }
        self.arrivals = events
            .iter()
            .filter_map(|e| match e {
                DiskEvent::Appeared(d) if d.removable && self.sys_block.join(&d.bsd_name).is_dir() => Some(d.clone()),
                _ => None,
            })
            .collect();
        events
            .iter()
            .map(|e| {
                let (event, body) = disk_wire::encode_event(e);
                Event { app: None, event: event.to_string(), body }
            })
            .collect()
    }

    /// Answers one call from `app`.
    pub fn handle(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "list" => {
                let disks = self.session.disks().iter().map(disk_wire::encode_disk).collect();
                Ok(obj(vec![("disks", Json::Arr(disks))]))
            }
            // every disk and partition with its parent, file system and label, read now: Disk Utility's sandbox has no
            // `/sys`, no udev database and no host mount table, so it asks the shell what they hold
            "volumes" => {
                let text = std::fs::read_to_string(&self.mounts).unwrap_or_default();
                let volumes = disk_scan::scan_volumes(&self.sys_block, &self.udev, &text).iter().map(disk_wire::encode_volume).collect();
                Ok(obj(vec![("volumes", Json::Arr(volumes))]))
            }
            "describe" => {
                let name = name_param(params)?;
                let d = self.session.description(&name).ok_or_else(|| format!("disks: {name} is not connected"))?;
                let fields = d.into_iter().map(|(k, v)| (k.to_string(), Json::Str(v))).collect();
                Ok(obj(vec![("description", Json::Obj(fields))]))
            }
            "plan_unmount" | "plan_eject" => {
                let name = name_param(params)?;
                let plan = if method == "plan_unmount" { self.session.unmount(&name, app) } else { self.session.eject(&name, app) };
                let commands = plan.map_err(|r| r.sentence())?;
                let list = commands
                    .into_iter()
                    .map(|c| {
                        let args = c.args.into_iter().map(Json::Str).collect();
                        obj(vec![("program", Json::Str(c.program.to_string())), ("args", Json::Arr(args))])
                    })
                    .collect();
                Ok(obj(vec![("commands", Json::Arr(list))]))
            }
            "claim" | "unclaim" => {
                let name = name_param(params)?;
                let r = if method == "claim" { self.session.claim(&name, app) } else { self.session.unclaim(&name, app) };
                r.map_err(|e| e.sentence())?;
                Ok(obj(vec![("ok", Json::Bool(true))]))
            }
            _ => Err(format!("disks has no method {method}")),
        }
    }
}

impl SystemService for Disks {
    fn name(&self) -> &'static str {
        disk_wire::SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        // volumes, labels and mount points are information the sandbox hides, and a claim locks a disk against Disk
        // Utility: the service answers system apps only
        super::require_system_app(app_id, disk_wire::SERVICE)?;
        if !self.primed {
            self.rescan();
        }
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        if t < self.next_scan {
            return Vec::new();
        }
        self.next_scan = t + SCAN_EVERY_MS;
        let events = self.rescan();
        // a claim does not outlive its app: a quit app would otherwise lock the disk against Disk Utility until restart
        for owner in self.session.claim_owners() {
            let running = crate::shell::apphost::slot_of(&owner).is_some_and(|slot| matches!(shell.ext.get(slot), Some(Some(_))));
            if !running {
                self.session.release_owner(&owner);
            }
        }
        let now = crate::bare::sys::wall_time().0;
        for disk in std::mem::take(&mut self.arrivals) {
            let label = if disk.model.is_empty() { disk.bsd_name.clone() } else { disk.model.clone() };
            shell.notify(Notification::new(AppId::Settings, "USB Disk Connected", format!("{label} is ready to use"), now));
            self.probe(&disk);
        }
        for body in self.unreadable_notices() {
            shell.notify(Notification::new(AppId::Settings, "Disk Not Readable", body, now));
        }
        events
    }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn name_param(params: &Json) -> Result<String, String> {
    params.str_field("bsd_name").filter(|n| !n.is_empty()).ok_or_else(|| "disks: a bsd_name is needed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A sysfs-like tree with one fixed disk `sda` and its partition `sda1`.
    fn fixture(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("bifrost-disks-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sys = root.join("block");
        add_disk(&sys, "sda", "0", "100000");
        let mounts = root.join("mounts");
        fs::write(&mounts, "").unwrap();
        (sys, mounts)
    }

    fn add_disk(sys: &PathBuf, name: &str, removable: &str, sectors: &str) {
        let dir = sys.join(name);
        fs::create_dir_all(dir.join("device")).unwrap();
        fs::write(dir.join("removable"), removable).unwrap();
        fs::write(dir.join("size"), sectors).unwrap();
        fs::write(dir.join("ro"), "0").unwrap();
        fs::write(dir.join("device/model"), "Flash Disk").unwrap();
    }

    fn names(events: &[Event]) -> Vec<String> {
        events.iter().map(|e| format!("{}:{}", e.event, e.body.str_field("bsd_name").unwrap_or_default())).collect()
    }

    #[test]
    fn the_first_scan_is_silent_and_lists_the_disks() {
        let (sys, mounts) = fixture("first");
        let mut s = Disks::at(&sys, &mounts);
        assert!(s.rescan().is_empty());
        let r = s.handle("a", "list", &Json::Null).unwrap();
        assert_eq!(r.get("disks").and_then(Json::as_array).map(|d| d.len()), Some(1));
    }

    #[test]
    fn the_volumes_call_lists_partitions_with_their_disk() {
        let (sys, mounts) = fixture("volumes");
        let part = sys.join("sda").join("sda1");
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("partition"), "1").unwrap();
        fs::write(part.join("size"), "50000").unwrap();
        fs::write(&mounts, "/dev/sda1 /media/data ext4 rw 0 0\n").unwrap();
        let mut s = Disks::at(&sys, &mounts);
        let r = s.handle("a", "volumes", &Json::Null).unwrap();
        let volumes: Vec<_> = r.get("volumes").and_then(Json::as_array).unwrap().iter().filter_map(disk_wire::decode_volume).collect();
        assert_eq!(volumes.iter().map(|v| v.disk.bsd_name.as_str()).collect::<Vec<_>>(), vec!["sda", "sda1"]);
        assert_eq!(volumes[1].parent.as_deref(), Some("sda"));
        assert_eq!(volumes[1].disk.mount_point.as_deref(), Some("/media/data"));
    }

    #[test]
    fn a_plugged_in_disk_appears_and_leaves() {
        let (sys, mounts) = fixture("plug");
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        add_disk(&sys, "sdb", "1", "2000");
        assert_eq!(names(&s.rescan()), vec!["disk_appeared:sdb"]);
        assert!(s.rescan().is_empty());
        fs::remove_dir_all(sys.join("sdb")).unwrap();
        assert_eq!(names(&s.rescan()), vec!["disk_disappeared:sdb"]);
    }

    #[test]
    fn a_removable_disk_arrival_is_queued_for_a_notice_and_a_fixed_one_is_not() {
        let (sys, mounts) = fixture("notice");
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        add_disk(&sys, "sdb", "1", "2000");
        s.rescan();
        assert_eq!(s.arrivals.iter().map(|d| d.bsd_name.as_str()).collect::<Vec<_>>(), vec!["sdb"]);
        add_disk(&sys, "sdc", "0", "3000");
        s.rescan();
        assert!(s.arrivals.is_empty());
    }

    #[test]
    fn a_probe_reports_only_the_volumes_that_match_no_filesystem() {
        let (sys, mounts) = fixture("probe");
        add_disk(&sys, "sdb", "1", "2000");
        let part = sys.join("sdb").join("sdb1");
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("partition"), "1").unwrap();
        let dev = sys.parent().unwrap().join("dev");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join("sdb1"), vec![0u8; 8192]).unwrap();
        let mut s = Disks::at(&sys, &mounts);
        s.dev = dev;
        let disk = Disk { bsd_name: "sdb".into(), size: 1, removable: true, read_only: false, model: "Flash Disk".into(), mount_point: None };
        s.probe(&disk);
        let mut notices = Vec::new();
        for _ in 0..200 {
            notices = s.unreadable_notices();
            if !s.probes.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            } else {
                break;
            }
        }
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("Flash Disk (sdb1)"), "{notices:?}");
        assert!(s.probes.is_empty());
    }

    #[test]
    fn mounting_changes_the_description() {
        let (sys, mounts) = fixture("mount");
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        fs::write(&mounts, "/dev/sda /media/data ext4 rw 0 0\n").unwrap();
        let events = s.rescan();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, disk_wire::DESCRIPTION_CHANGED);
        let d = s.handle("a", "describe", &obj(vec![("bsd_name", Json::Str("sda".into()))])).unwrap();
        assert_eq!(d.get("description").and_then(|d| d.str_field("volume_path")).as_deref(), Some("/media/data"));
    }

    #[test]
    fn unmount_and_eject_are_validated() {
        let (sys, mounts) = fixture("plan");
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        let gone = obj(vec![("bsd_name", Json::Str("sdz".into()))]);
        assert!(s.handle("a", "plan_unmount", &gone).unwrap_err().contains("sdz"));
        let bad = obj(vec![("bsd_name", Json::Str("../etc".into()))]);
        assert!(s.handle("a", "plan_eject", &bad).is_err());
        assert!(s.handle("a", "plan_eject", &Json::Null).is_err());
        assert!(s.handle("a", "nope", &Json::Null).is_err());
    }

    #[test]
    fn a_claim_keeps_other_apps_from_planning() {
        let (sys, mounts) = fixture("claim");
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        let sda = obj(vec![("bsd_name", Json::Str("sda".into()))]);
        s.handle("owner", "claim", &sda).unwrap();
        assert!(s.handle("other", "plan_eject", &sda).unwrap_err().contains("in use by owner"));
    }
}
