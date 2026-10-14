//! The `disks` service: the block devices the kernel reports, pushed to apps as they appear, disappear or change
//! (mounted, unmounted, resized). It is the shell side of Disk Arbitration (`DARegisterDiskAppearedCallback` and its
//! two siblings); the SDK side is `bifrost_app::disk_arbitration`. The model is `bifrost_core::disk_session`.
//!
//! The shell scans `/sys/block` and `/proc/mounts` (`bifrost_core::disk_scan`) every [`SCAN_EVERY_MS`]. The first scan
//! only records the state; an app that wants the disks already there calls `list`.
//!
//! Methods (JSON params):
//! - `list` -> `{disks: [disk]}`, a disk being `bifrost_core::disk_wire::encode_disk`
//! - `describe` `{bsd_name}` -> `{description: {key: text}}` (`disk_events::description`)
//! - `plan_unmount` / `plan_eject` `{bsd_name}` -> `{commands: [{program, args}]}`, the plan after the claim, approval
//!   and device checks. The service validates and plans; it does not run the commands yet (they need the privileged
//!   helper, which still has to be tried on Midget with a real USB disk).
//! - `claim` / `unclaim` `{bsd_name}` -> `{ok}`: exclusive use of a disk for the calling app.
//!
//! Events (to every app, from `bifrost_core::disk_wire`): `disk_appeared`, `disk_disappeared`,
//! `disk_description_changed`.
use super::{Event, Shell, SystemService};
use bifrost_core::disk_scan;
use bifrost_core::disk_session::Session;
use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use std::path::PathBuf;

/// The shell clock `t` of a service tick is in milliseconds. How often the device list is read again.
pub const SCAN_EVERY_MS: f64 = 2_000.0;

pub struct Disks {
    session: Session,
    sys_block: PathBuf,
    mounts: PathBuf,
    /// Whether a snapshot has been taken; the first one is recorded without events.
    primed: bool,
    next_scan: f64,
}

impl Default for Disks {
    fn default() -> Self {
        Self::at("/sys/block", "/proc/mounts")
    }
}

impl Disks {
    /// A service reading the device tree at `sys_block` and the mount table file at `mounts`.
    pub fn at(sys_block: impl Into<PathBuf>, mounts: impl Into<PathBuf>) -> Self {
        Disks { session: Session::new(), sys_block: sys_block.into(), mounts: mounts.into(), primed: false, next_scan: 0.0 }
    }

    /// Reads the devices now and returns the events since the last read (none the first time).
    pub fn rescan(&mut self) -> Vec<Event> {
        let text = std::fs::read_to_string(&self.mounts).unwrap_or_default();
        let snapshot = disk_scan::scan(&self.sys_block, &text).into_iter().map(|e| e.disk).collect();
        let events = self.session.observe(snapshot);
        if !std::mem::replace(&mut self.primed, true) {
            return Vec::new();
        }
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
        if !self.primed {
            self.rescan();
        }
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, _shell: &mut Shell, t: f64) -> Vec<Event> {
        if t < self.next_scan {
            return Vec::new();
        }
        self.next_scan = t + SCAN_EVERY_MS;
        self.rescan()
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
