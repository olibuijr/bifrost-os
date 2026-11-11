//! The `disks` service: the block devices the kernel reports, pushed to apps as they appear, disappear or change
//! (mounted, unmounted, resized). It is the shell side of Disk Arbitration (`DARegisterDiskAppearedCallback` and its
//! two siblings); the SDK side is `bifrost_app::disk_arbitration`. The model is `bifrost_core::disk_session`.
//!
//! The shell scans `/sys/block` and `/proc/mounts` (`bifrost_core::disk_scan`) when something happened: the kernel
//! made or removed a device node (`shell::block_watch`, a watch on `/dev` in the shell's poll set), a mount pass or
//! an app's request ended. Nothing wakes the shell to scan otherwise. What the watch does not tell (a mount made by
//! another program, a card put into a reader) is picked up by a scan at most every [`SCAN_FALLBACK_MS`] on a
//! wake-up the shell has anyway; without the watch that is every [`SCAN_EVERY_MS`], as it was before the watch. The
//! first scan only records the state; an app that wants the disks already there calls `list`.
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
//! - `attach_image` `{path, read_only?}` / `detach_image` `{target}` -> `{started}`: mounts a disk image of the account
//!   or releases one (`shell::disk_image`). These two also answer an app with the `storage` permission (Files). The
//!   work runs on a thread; the outcome comes to the calling app as `image_attached` / `image_detached`
//!   `{ok, path|target, device, mount_point, error}`. A partitioned image gives one volume per partition that can be
//!   mounted (`mount_point` is the first); detaching any of them releases the whole image.
//! - `eject_volume` `{target}` -> `{started}`: unmounts a removable volume's partition (`/dev/sdb1`) and powers its
//!   disk off (`shell::volume_eject`), for the same callers and on the same thread pool; the outcome is the event
//!   `volume_ejected` `{ok, target, device, powered_off, error}`. A sandboxed app cannot do this itself.
//! - `mount_volume` `{target}` -> `{started}`: mounts a removable volume that is attached and not mounted, for the
//!   same callers: the way back for a volume that was ejected and whose disk stayed (`shell::volume_mount`). The
//!   target is checked like an eject's and then like a volume of the automatic pass (removable disks only, never a
//!   disk the system uses, the same file systems and options). Refused while the screen is locked. The outcome is
//!   the event `volume_mounted` `{target, ok, device, mount_point, name}` or `{target, ok, error}` to the caller.
//!
//! Events (to system apps, from `bifrost_core::disk_wire`): `disk_appeared`, `disk_disappeared`,
//! `disk_description_changed`.
//!
//! Removable volumes are mounted without anyone asking (`shell::volume_mount`): a moment after a device arrives,
//! and for the ones already plugged in once the screen is unlocked. Each mount is the event `volume_mounted`
//! `{ok, device, mount_point, name}`; when a mounted disk is pulled out its mount is taken away and the event is
//! `volume_unmounted` `{ok, device, mount_point}`. These two go to system apps and to apps with the `storage`
//! permission (`services::hears_volumes`), so the Files sidebar can follow them. A volume that does not mount is one
//! "Disk Could Not Be Opened" notice in plain words; the tool's message goes to the log. An ejected volume is not
//! mounted again until it is unplugged or an app asks with `mount_volume`.
//!
//! When a removable whole disk (not a partition) appears, the shell also posts a "USB Disk Connected" notice in
//! Settings. The notice is for the arrival only: the disk's partitions and the eject that follows get no notice. It
//! waits for the mount that follows the arrival, and is left out when a volume of the disk did not mount: that disk
//! gets the one "Disk Could Not Be Opened" notice and not a "ready to use" beside it ([`announced`]). While the
//! screen is locked nothing is mounted and nothing is said: no notice about a disk is posted behind a locked screen
//! (it would name the device on the lock screen and call a disk "ready" that is not mounted). The notices wait for
//! the unlock and the mount that follows it; a disk attached and pulled out again while locked leaves none.
//!
//! The same arrival starts a probe on a thread (`bifrost_core::disk_magic`): the first sectors of each partition (or
//! of the disk itself when it has none) are read and matched against the filesystems the system mounts. When a
//! volume matches none (and is not an encrypted volume or swap), the shell posts a "Disk Not Readable" notice that
//! points at Disk Utility, the counterpart of the reference's prompt for a disk it cannot read.
use super::{Event, Shell, SystemService};
use crate::shell::block_watch::{self, Change};
use crate::shell::volume_mount::{self, Done};
use crate::shell::apps::AppId;
use crate::shell::notify::store::Notification;
use bifrost_core::disk_events::{Disk, DiskEvent};
use bifrost_core::disk_magic::{self, Verdict};
use bifrost_core::disk_scan;
use bifrost_core::disk_session::Session;
use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// The shell clock `t` of a service tick is in milliseconds. How often the device list is read again when there is
/// no watch on `/dev`. Neither interval is a timer: a scan that is due waits for a tick the shell has anyway.
pub const SCAN_EVERY_MS: f64 = 2_000.0;
/// How often it is read again with the watch, for what the watch does not tell: a mount or an unmount made by
/// another program, a card put into a reader that makes no new device node.
pub const SCAN_FALLBACK_MS: f64 = 10_000.0;
/// How often the loop looks for the end of an app's request (image, eject, mount) while one runs: often while it is
/// young, seldom once it has run longer than [`JOB_QUICK_MS`] (an eject can wait on a long write).
const JOB_POLL_MS: f64 = 100.0;
const JOB_SLOW_POLL_MS: f64 = 1_000.0;
const JOB_QUICK_MS: f64 = 2_000.0;

/// Whether the device `name` (`sdb1`, `mmcblk1p1`) is the disk `disk` or one of its partitions: the disk's name and
/// a number, after a `p` when the disk's own name ends in a digit.
pub fn on_disk(name: &str, disk: &str) -> bool {
    let Some(rest) = name.strip_prefix(disk).filter(|_| !disk.is_empty()) else { return false };
    if rest.is_empty() {
        return true;
    }
    let number = if disk.ends_with(|c: char| c.is_ascii_digit()) { rest.strip_prefix('p') } else { Some(rest) };
    number.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The arrived disks that get their "USB Disk Connected" notice: all but the ones with a volume among `failed`
/// (kernel names), which get the notice of the failure instead.
pub fn announced<'a>(arrived: &'a [Disk], failed: &[String]) -> Vec<&'a Disk> {
    arrived.iter().filter(|d| !failed.iter().any(|f| on_disk(f, &d.bsd_name))).collect()
}

pub struct Disks {
    session: Session,
    sys_block: PathBuf,
    mounts: PathBuf,
    /// the udev database the volumes call reads file system types and labels from
    udev: PathBuf,
    /// Whether a snapshot has been taken; the first one is recorded without events.
    primed: bool,
    next_scan: f64,
    /// The removable whole disks that appeared in the last read.
    arrivals: Vec<Disk>,
    /// The arrived disks whose notice waits for the mount that follows, and the volumes that did not mount.
    connected: Vec<Disk>,
    failed: Vec<String>,
    /// The notices that came up while the screen was locked, posted at the unlock: the disk's kernel name (empty
    /// when the notice is about several), the title and the sentence.
    held: Vec<(String, &'static str, String)>,
    /// When the newest app request started, in shell milliseconds.
    job_from: f64,
    /// where the device nodes are (`/dev`)
    dev: PathBuf,
    /// the probes running for arrived disks: each sends the disk's kernel name, its label and a verdict per volume
    probes: Vec<Receiver<(String, String, Vec<(String, Verdict)>)>>,
    /// the disk image attach and detach requests still running
    images: crate::shell::disk_image::Jobs,
    /// the mounting of removable volumes as they arrive
    auto: crate::shell::volume_mount::Auto,
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
            connected: Vec::new(),
            failed: Vec::new(),
            held: Vec::new(),
            job_from: 0.0,
            dev: PathBuf::from("/dev"),
            probes: Vec::new(),
            images: Default::default(),
            auto: Default::default(),
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
        let name = disk.bsd_name.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let results = volumes.into_iter().filter_map(|v| disk_magic::classify(&dev.join(&v)).map(|verdict| (v, verdict))).collect();
            let _ = tx.send((name, label, results));
        });
        self.probes.push(rx);
    }

    /// The finished probes' sentences for volumes that cannot be read, each with its disk's kernel name.
    fn unreadable_notices(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        self.probes.retain(|rx| match rx.try_recv() {
            Ok((name, label, results)) => {
                out.extend(disk_magic::unreadable_notice(&label, &results).map(|body| (name, body)));
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
        self.auto.observe(&events, self.session.disks());
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

    /// One read of the device list on the tick, with what the watch on `/dev` saw since the last one. The scan
    /// compares two lists, so a disk pulled and plugged in again between two reads looks unchanged to it; the watch
    /// names both, and the mounting takes each in: a node removed for a device the list knew is a device that left,
    /// a node made for a device the list knows now is one that arrived. A name the list never knew (a terminal, an
    /// input node) is nothing.
    fn scan(&mut self, changes: &[Change]) -> Vec<Event> {
        let before: BTreeSet<String> = self.session.disks().iter().map(|d| d.bsd_name.clone()).collect();
        let events = self.rescan();
        // a disk that left takes the notices still waiting for it along
        for event in &events {
            if let Some(name) = (event.event == disk_wire::DISAPPEARED).then(|| event.body.str_field("bsd_name")).flatten() {
                self.forget(&name);
            }
        }
        for change in changes {
            match change {
                Change::Removed(name) if before.contains(name) => {
                    self.auto.unplugged(name);
                    self.forget(name);
                }
                Change::Added(name) => {
                    let Some(disk) = self.session.disks().iter().find(|d| d.bsd_name == *name).cloned() else { continue };
                    self.auto.plugged(name);
                    // pulled and plugged in again between two reads: the list shows no arrival, and this is one
                    let again = before.contains(name) && disk.removable && self.sys_block.join(name).is_dir();
                    if again && !self.connected.iter().chain(&self.arrivals).any(|d| d.bsd_name == *name) {
                        self.connected.push(disk);
                    }
                }
                _ => {}
            }
        }
        events
    }

    /// The disk `name` left: the notices waiting for it (its arrival, a volume of it that cannot be read) are dropped.
    fn forget(&mut self, name: &str) {
        self.connected.retain(|d| d.bsd_name != name);
        self.held.retain(|(disk, _, _)| disk != name);
        self.failed.retain(|f| !on_disk(f, name));
    }

    /// Posts the notices that are due. Behind a locked screen none is: nothing is mounted there, and a notice would
    /// show the device's name on the lock screen. They wait in `held` and `connected` for the unlock. Unlocked, the
    /// held ones go out at once and an arrival's "ready to use" once the mount that follows it has ended (`settled`),
    /// left out for a disk with a volume that did not mount.
    fn post_notices(&mut self, shell: &mut Shell, locked: bool, settled: bool, fresh: Vec<(String, &'static str, String)>) {
        self.held.extend(fresh);
        if locked {
            return;
        }
        let now = crate::bare::sys::wall_time().0;
        for (_, title, body) in std::mem::take(&mut self.held) {
            shell.notify(Notification::new(AppId::Settings, title, body, now));
        }
        if settled {
            let arrived = std::mem::take(&mut self.connected);
            for disk in announced(&arrived, &self.failed) {
                let label = if disk.model.is_empty() { disk.bsd_name.clone() } else { disk.model.clone() };
                shell.notify(Notification::new(AppId::Settings, "USB Disk Connected", format!("{label} is ready to use"), now));
            }
            self.failed.clear();
        }
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
                let scanned = disk_scan::scan_volumes(&self.sys_block, &self.udev, &text);
                let volumes = scanned.iter().map(disk_wire::encode_volume).collect();
                Ok(obj(vec![("volumes", Json::Arr(volumes)), ("usage", Json::Arr(usage_of(&scanned)))]))
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

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        // an app that already reads and writes the whole home folder (Files) may have one of the account's disk images
        // mounted or released: privd checks the image itself, and the answer shows nothing of the other disks
        if crate::shell::disk_image::Jobs::handles(method) {
            if !super::app_has_permission(app_id, "storage") {
                super::require_system_app(app_id, disk_wire::SERVICE)?;
            }
            // nothing is mounted behind a locked screen, on request as little as by itself
            if method == volume_mount::M_MOUNT && shell.is_locked() {
                return Err("disks: no volume is mounted while the screen is locked".to_string());
            }
            let started = self.images.start(app_id, method, params)?;
            self.job_from = t;
            // an ejected volume stays ejected: it is not mounted again while it is still plugged in
            if method == crate::shell::volume_eject::M_EJECT {
                self.auto.ejecting(&params.str_field("target").unwrap_or_default());
            }
            return Ok(started);
        }
        // volumes, labels and mount points are information the sandbox hides, and a claim locks a disk against Disk
        // Utility: the service answers system apps only
        super::require_system_app(app_id, disk_wire::SERVICE)?;
        if !self.primed {
            self.rescan();
        }
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        // a finished disk image request goes to the app that asked, on the tick it ends
        let ended = self.images.finished();
        let requests_ended = !ended.is_empty();
        let mut events: Vec<Event> = ended.into_iter().map(|(app, event, body)| Event { app: Some(app), event: event.to_string(), body }).collect();
        // a finished pass over the removable volumes: its mounts and unmounts go to the apps that show volumes, and
        // what did not mount is one notice
        let report = self.auto.finish();
        let mut fresh: Vec<(String, &'static str, String)> = Vec::new();
        if let Some(report) = &report {
            let sent = volume_mount::events(report);
            if !sent.is_empty() {
                let listeners = super::volume_listeners();
                events.extend(sent.iter().flat_map(|(event, body)| listeners.iter().map(move |app| Event { app: Some(app.clone()), event: event.to_string(), body: body.clone() })));
            }
            if let Some(body) = volume_mount::failure_notice(report) {
                fresh.push((String::new(), "Disk Could Not Be Opened", body));
            }
            self.failed.extend(report.volumes.iter().filter(|v| matches!(v.done, Done::Failed(_))).map(|v| v.device.clone()));
        }
        // the device list is read when something happened to it: a device node came or went, a pass or a request
        // changed the mounts; otherwise only when the slow interval is over and the shell is awake anyway. A unit
        // test never watches the machine it runs on.
        if !cfg!(test) {
            block_watch::open();
        }
        let (changes, lost) = block_watch::take();
        if !self.primed || lost || !changes.is_empty() || requests_ended || report.is_some() || t >= self.next_scan {
            self.next_scan = t + if block_watch::active() { SCAN_FALLBACK_MS } else { SCAN_EVERY_MS };
            events.extend(self.scan(&changes));
            // a claim does not outlive its app: a quit app would otherwise lock the disk against Disk Utility until restart
            for owner in self.session.claim_owners() {
                let running = crate::shell::apphost::slot_of(&owner).is_some_and(|slot| matches!(shell.ext.get(slot), Some(Some(_))));
                if !running {
                    self.session.release_owner(&owner);
                }
            }
            for disk in std::mem::take(&mut self.arrivals) {
                self.probe(&disk);
                self.connected.push(disk);
            }
        }
        let locked = shell.is_locked();
        self.auto.start(t, locked);
        // the arrival's notice waits for the mount that follows it, so a disk that did not open is not "ready to use"
        let settled = !self.auto.mount_waiting();
        fresh.extend(self.unreadable_notices().into_iter().map(|(disk, body)| (disk, "Disk Not Readable", body)));
        self.post_notices(shell, locked, settled, fresh);
        events
    }

    fn next_wake(&self, shell: &Shell, t: f64) -> Option<f64> {
        // a unit test's shell starts no pass and no request on the machine it runs on: it has nothing to wake for
        if cfg!(test) {
            return None;
        }
        let request = self.images.busy().then(|| t + if t - self.job_from > JOB_QUICK_MS { JOB_SLOW_POLL_MS } else { JOB_POLL_MS });
        match (self.auto.next_wake(t, shell.is_locked()), request) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The space on each mounted volume, measured here on the host mount point (an app sandbox sees its own mount table
/// and cannot `statvfs` the host's). A volume whose mount point cannot be read is left out. `free` is the space an
/// unprivileged write can use, as `df` reports it.
fn usage_of(volumes: &[disk_scan::Volume]) -> Vec<Json> {
    volumes
        .iter()
        .filter_map(|v| {
            let point = v.disk.mount_point.as_ref()?;
            let c = bifrost_core::volume::capacity(std::path::Path::new(point)).ok()?;
            Some(disk_wire::encode_usage(&disk_wire::VolumeUsage { bsd_name: v.disk.bsd_name.clone(), total: c.total, used: c.used(), free: c.available }))
        })
        .collect()
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
        // /media/data does not exist on the test host, so no usage is measured for it
        let usage = r.get("usage").and_then(Json::as_array).unwrap();
        assert!(usage.iter().filter_map(disk_wire::decode_usage).all(|u| u.bsd_name != "sda1"));
    }

    #[test]
    fn mounted_volumes_get_their_usage_from_the_host_mount_point() {
        let (sys, mounts) = fixture("usage");
        let part = sys.join("sda").join("sda1");
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("partition"), "1").unwrap();
        fs::write(part.join("size"), "50000").unwrap();
        // the root of the test host is always mounted and readable
        fs::write(&mounts, "/dev/sda1 /var/log btrfs rw 0 0\n/dev/sda1 / btrfs rw 0 0\n").unwrap();
        let mut s = Disks::at(&sys, &mounts);
        let r = s.handle("a", "volumes", &Json::Null).unwrap();
        let volumes: Vec<_> = r.get("volumes").and_then(Json::as_array).unwrap().iter().filter_map(disk_wire::decode_volume).collect();
        assert_eq!(volumes[1].disk.mount_point.as_deref(), Some("/"), "the system volume reads as /, not a subvolume");
        let usage: Vec<_> = r.get("usage").and_then(Json::as_array).unwrap().iter().filter_map(disk_wire::decode_usage).collect();
        let root = bifrost_core::volume::capacity(std::path::Path::new("/")).unwrap();
        let u = usage.iter().find(|u| u.bsd_name == "sda1").expect("usage for the mounted volume");
        assert_eq!(u.total, root.total);
        assert!(u.used <= u.total && u.free <= u.total);
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
            notices = s.unreadable_notices().into_iter().map(|(_, body)| body).collect::<Vec<_>>();
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
    fn the_scan_feeds_the_mounting_of_removable_volumes() {
        let (sys, mounts) = fixture("automount");
        fs::write(&mounts, "/dev/sda /media/data ext4 rw 0 0\n").unwrap();
        let mut s = Disks::at(&sys, &mounts);
        s.rescan();
        assert!(s.auto.is_handled("sda"), "a device found mounted is left alone");
        // the first scan asks for a pass over what is plugged in, which waits while the screen is locked
        assert_eq!(s.auto.next_pass(0.0, true, false).map(|p| (p.mount, p.sweep)), Some((false, true)));
        assert_eq!(s.auto.next_pass(60_000.0, false, false).map(|p| (p.mount, p.sweep)), Some((true, false)));
        // a stick arrives: a pass is due a moment later; it leaves: its mounts are swept
        add_disk(&sys, "sdb", "1", "2000");
        s.rescan();
        assert_eq!(s.auto.next_pass(70_000.0, false, false), None);
        assert!(s.auto.next_pass(70_000.0 + crate::shell::volume_mount::SETTLE_MS, false, false).is_some_and(|p| p.mount));
        fs::remove_dir_all(sys.join("sdb")).unwrap();
        s.rescan();
        assert!(s.auto.next_pass(80_000.0, true, false).is_some_and(|p| p.sweep && !p.mount));
    }

    fn removable(name: &str) -> Disk {
        Disk { bsd_name: name.into(), size: 1, removable: true, read_only: false, model: String::new(), mount_point: None }
    }

    #[test]
    fn a_disk_pulled_and_plugged_in_again_between_two_scans_is_seen_through_the_watch() {
        let (sys, mounts) = fixture("replug");
        add_disk(&sys, "sdb", "1", "2000");
        fs::write(&mounts, "/dev/sdb /run/media/me/Stick vfat rw 0 0\n").unwrap();
        let mut s = Disks::at(&sys, &mounts);
        s.scan(&[]);
        let _ = s.auto.next_pass(0.0, false, false);
        let _ = s.auto.next_pass(crate::shell::volume_mount::SETTLE_MS, false, false);
        assert!(s.auto.is_handled("sdb"), "found mounted, it is left alone");
        // the same name is back before the next scan, and the mount of the device that left is still in the table:
        // the two lists are equal, so the scan alone reports nothing and would leave the stale mount in place
        assert!(s.rescan().is_empty());
        assert_eq!(s.auto.next_pass(10_000.0, false, false), None);
        assert!(s.auto.is_handled("sdb"));
        // with the watch's two events the device is new: forgotten, swept at once, then mounted
        let seen = [Change::Removed("sdb".into()), Change::Added("sdb".into())];
        assert!(s.scan(&seen).is_empty(), "still no change in the list");
        assert!(!s.auto.is_handled("sdb"), "its stale mount does not make it decided on");
        assert_eq!(s.auto.next_pass(20_000.0, false, false).map(|p| (p.mount, p.sweep)), Some((false, true)));
        assert_eq!(s.auto.next_pass(20_000.0 + crate::shell::volume_mount::SETTLE_MS, false, false).map(|p| (p.mount, p.sweep)), Some((true, false)));
    }

    #[test]
    fn a_node_that_is_no_disk_asks_for_nothing() {
        let (sys, mounts) = fixture("nodes");
        let mut s = Disks::at(&sys, &mounts);
        s.scan(&[]);
        let _ = s.auto.next_pass(0.0, false, false);
        let _ = s.auto.next_pass(crate::shell::volume_mount::SETTLE_MS, false, false);
        // a terminal and an input node come and go; a disk node is removed that the list never knew
        let seen = [Change::Added("ttyACM0".into()), Change::Removed("ttyACM0".into()), Change::Removed("hidraw3".into()), Change::Removed("sdq".into()), Change::Added("sdq".into())];
        assert!(s.scan(&seen).is_empty());
        assert_eq!(s.auto.next_pass(10_000.0, false, false), None, "no sweep and no mount");
        assert_eq!(s.auto.next_wake(10_000.0, false), None);
        // a pull seen by the watch and by the scan is one device that left
        add_disk(&sys, "sdb", "1", "2000");
        s.scan(&[Change::Added("sdb".into())]);
        assert_eq!(s.auto.next_pass(20_000.0, false, false), None, "the settling time first");
        assert!(s.auto.next_pass(20_000.0 + crate::shell::volume_mount::SETTLE_MS, false, false).is_some_and(|p| p.mount));
        fs::remove_dir_all(sys.join("sdb")).unwrap();
        assert_eq!(names(&s.scan(&[Change::Removed("sdb".into())])), vec!["disk_disappeared:sdb"]);
        assert_eq!(s.auto.next_pass(30_000.0, false, false).map(|p| (p.mount, p.sweep)), Some((false, true)));
    }

    #[test]
    fn a_disk_that_did_not_open_is_not_announced_as_ready() {
        assert!(on_disk("sdb", "sdb") && on_disk("sdb1", "sdb") && on_disk("sdb12", "sdb"));
        assert!(on_disk("mmcblk1p1", "mmcblk1") && on_disk("nvme0n1p2", "nvme0n1") && on_disk("mmcblk1", "mmcblk1"));
        assert!(!on_disk("sdba1", "sdb") && !on_disk("sdc1", "sdb") && !on_disk("sd", "sdb") && !on_disk("sdbp1", "sdb"));
        assert!(!on_disk("mmcblk10", "mmcblk1") && !on_disk("mmcblk10p1", "mmcblk1") && !on_disk("mmcblk1p", "mmcblk1") && !on_disk("sdb1", ""));
        let arrived = [removable("sdb"), removable("sdc"), removable("mmcblk1")];
        let names = |failed: &[&str]| -> Vec<String> {
            let failed: Vec<String> = failed.iter().map(|f| f.to_string()).collect();
            announced(&arrived, &failed).into_iter().map(|d| d.bsd_name.clone()).collect()
        };
        assert_eq!(names(&[]), ["sdb", "sdc", "mmcblk1"]);
        assert_eq!(names(&["sdb1"]), ["sdc", "mmcblk1"], "the failure's notice is the one notice for that disk");
        assert_eq!(names(&["sdc", "mmcblk1p2"]), ["sdb"]);
        assert_eq!(names(&["sdd1"]), ["sdb", "sdc", "mmcblk1"]);
    }

    #[test]
    fn a_request_to_mount_is_refused_behind_a_locked_screen_and_for_a_target_that_is_no_device() {
        let (sys, mounts) = fixture("request");
        let mut s = Disks::at(&sys, &mounts);
        let mut sh = crate::shell::tests::shell();
        let target = |t: &str| obj(vec![("target", Json::Str(t.into()))]);
        // the shell itself asks (an empty app id), so the answer is about the request and not about who asked
        for bad in ["", "sdb1", "/dev/../etc/shadow", "/run/media/me/Stick", "/dev/sdb1 -o suid", "/dev/mapper/x"] {
            assert!(s.call(&mut sh, "", volume_mount::M_MOUNT, &target(bad), 0.0).is_err(), "{bad}");
        }
        assert!(s.call(&mut sh, "", volume_mount::M_MOUNT, &Json::Null, 0.0).is_err());
        sh.screen = crate::shell::Screen::Lock;
        let refused = s.call(&mut sh, "", volume_mount::M_MOUNT, &target("/dev/sdzz9"), 0.0).unwrap_err();
        assert!(refused.contains("locked"), "{refused}");
        // an app without the storage permission that is no system app does not get that far
        assert!(s.call(&mut sh, "com.example.other", volume_mount::M_MOUNT, &target("/dev/sdzz9"), 0.0).unwrap_err().contains("system apps"));
        assert!(!s.images.busy(), "nothing was started");
    }

    /// A service on the fixture whose probes read nothing of the machine the test runs on.
    fn quiet(sys: &PathBuf, mounts: &PathBuf) -> Disks {
        let mut s = Disks::at(sys, mounts);
        s.dev = sys.parent().unwrap().join("no-dev");
        s
    }

    /// The notices in the shell's store: (title, body).
    fn notices(sh: &Shell) -> Vec<(String, String)> {
        sh.notify.store().groups().iter().flat_map(|g| g.items.iter().map(|n| (n.title.clone(), n.body.clone()))).collect()
    }

    #[test]
    fn a_stick_attached_behind_a_locked_screen_is_announced_once_after_the_unlock() {
        let (sys, mounts) = fixture("locked-arrival");
        let mut s = quiet(&sys, &mounts);
        let mut sh = crate::shell::tests::shell();
        sh.screen = crate::shell::Screen::Lock;
        s.tick(&mut sh, 0.0);
        add_disk(&sys, "sdb", "1", "2000");
        s.tick(&mut sh, 3_000.0);
        assert_eq!(s.connected.iter().map(|d| d.bsd_name.as_str()).collect::<Vec<_>>(), vec!["sdb"], "the arrival waits");
        s.tick(&mut sh, 6_000.0);
        s.tick(&mut sh, 60_000.0);
        assert!(notices(&sh).is_empty(), "nothing is said on the lock screen, however long it stays locked");
        // the unlock: the waiting mount starts (a test runs none, so it has ended at once) and the notice follows it
        sh.screen = crate::shell::Screen::Home;
        s.tick(&mut sh, 61_000.0);
        assert_eq!(notices(&sh), vec![("USB Disk Connected".to_string(), "Flash Disk is ready to use".to_string())]);
        s.tick(&mut sh, 64_000.0);
        s.tick(&mut sh, 67_000.0);
        assert_eq!(notices(&sh).len(), 1, "once");
        assert!(s.connected.is_empty() && s.held.is_empty());
    }

    #[test]
    fn a_stick_that_does_not_mount_after_the_unlock_gets_the_failure_notice_and_no_ready_notice() {
        let (sys, mounts) = fixture("locked-failure");
        let mut s = quiet(&sys, &mounts);
        let mut sh = crate::shell::tests::shell();
        sh.screen = crate::shell::Screen::Lock;
        s.tick(&mut sh, 0.0);
        add_disk(&sys, "sdb", "1", "2000");
        s.tick(&mut sh, 3_000.0);
        // the unlock starts the pass; while it is at work nothing is said yet
        let (tx, rx) = channel();
        s.auto.set_running(rx);
        sh.screen = crate::shell::Screen::Home;
        s.tick(&mut sh, 10_000.0);
        assert!(notices(&sh).is_empty(), "the notice waits for the mount");
        assert_eq!(s.connected.len(), 1);
        let failed = volume_mount::Outcome { device: "sdb1".into(), name: "Stick".into(), done: Done::Failed("It may be damaged. The details are in the log.".into()) };
        tx.send(volume_mount::Report { volumes: vec![failed], removed: Vec::new() }).unwrap();
        s.tick(&mut sh, 10_100.0);
        let got = notices(&sh);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, "Disk Could Not Be Opened");
        assert!(got[0].1.contains("Stick"), "{got:?}");
        s.tick(&mut sh, 13_000.0);
        assert_eq!(notices(&sh).len(), 1, "and no \"ready to use\" after it");
        assert!(s.connected.is_empty() && s.failed.is_empty());
    }

    #[test]
    fn a_stick_attached_and_pulled_out_behind_a_locked_screen_leaves_no_notice() {
        let (sys, mounts) = fixture("locked-gone");
        let mut s = quiet(&sys, &mounts);
        let mut sh = crate::shell::tests::shell();
        sh.screen = crate::shell::Screen::Lock;
        s.tick(&mut sh, 0.0);
        add_disk(&sys, "sdb", "1", "2000");
        s.tick(&mut sh, 3_000.0);
        assert_eq!(s.connected.len(), 1);
        // its probe found a volume it cannot read: that notice waits too, and a failure of another disk stays
        s.held.push(("sdb".into(), "Disk Not Readable", "Flash Disk (sdb1) uses a format this computer cannot read.".into()));
        s.failed.push("sdb1".into());
        fs::remove_dir_all(sys.join("sdb")).unwrap();
        s.tick(&mut sh, 6_000.0);
        assert!(s.connected.is_empty() && s.held.is_empty() && s.failed.is_empty(), "what waited for the disk left with it");
        assert!(notices(&sh).is_empty());
        sh.screen = crate::shell::Screen::Home;
        s.tick(&mut sh, 9_000.0);
        s.tick(&mut sh, 12_000.0);
        assert!(notices(&sh).is_empty(), "nothing after the unlock either");
    }

    #[test]
    fn a_notice_about_a_disk_that_cannot_be_read_waits_for_the_unlock() {
        let (sys, mounts) = fixture("locked-unreadable");
        let mut s = quiet(&sys, &mounts);
        let mut sh = crate::shell::tests::shell();
        sh.screen = crate::shell::Screen::Lock;
        let fresh = vec![("sdb".to_string(), "Disk Not Readable", "Flash Disk (sdb1) uses a format this computer cannot read.".to_string())];
        s.post_notices(&mut sh, true, true, fresh);
        assert!(notices(&sh).is_empty());
        assert_eq!(s.held.len(), 1);
        s.post_notices(&mut sh, false, false, Vec::new());
        assert_eq!(notices(&sh).len(), 1, "posted at the unlock, whether or not a mount is still at work");
        assert_eq!(notices(&sh)[0].0, "Disk Not Readable");
        assert!(s.held.is_empty());
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
