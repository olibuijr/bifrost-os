//! Mounting removable volumes (a USB stick, a memory card) when they are plugged in, and for the ones already there
//! when the shell starts or the account signs in. The eject is `shell::volume_eject`; a disk image has its own path
//! (`shell::disk_image`) and is never touched here.
//!
//! The `disks` service drives it ([`Auto`]): its device scan reports what appeared, went away or changed, and a
//! moment after the last arrival (partitions show up after their disk) one pass runs on a thread, the way the disk
//! image requests do. No app asks for a mount: every device name comes from the kernel's list under `/sys/block`.
//!
//! A pass is planned in two steps of pure functions. [`candidates`] picks the devices worth reading from the kernel's
//! list, the mount table and the swap list: only disks the kernel marks removable or that sit on a bus things are
//! plugged into, never a disk with a part in use by the system. [`plan_mounts`] then decides from what each device
//! holds (`blkid -p`, read from the device itself): only the file systems in [`supported`], never swap, an encrypted
//! container or a volume-manager member, and never a partition its table marks as a system or hidden one.
//!
//! Unlike a disk image, which a person opens on purpose and which mounts its EFI partition too, a boot partition of a
//! plugged-in disk is left unmounted: nobody asked to see it, and a stray write to it can stop the disk from starting
//! a computer. For the same reason every mount here is `noexec` (an image's is not): what arrives on a stick is
//! opened, not run.
//!
//! Nothing is mounted while the screen is locked or the login window is up; the pass waits for the unlock. A device
//! that was decided on once (mounted, failed, nothing to mount, ejected by the person, or found mounted already) is
//! not tried again until it is unplugged, so a failure is one notice and never a loop, and an ejected volume stays
//! ejected. When a mounted disk is pulled out without an eject, its mounts under the media folders are taken away
//! lazily and their empty folders removed.

use super::disk_image::{self, export_value, image_device_of, run};
use super::volume_eject::valid_device;
use crate::bare::account::Account;
use bifrost_core::disk_events::{Disk, DiskEvent};
use bifrost_core::json::Json;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// The `disks` service events: a removable volume was mounted `{ok, device, mount_point, name}`, and the mount of a
/// volume whose disk was pulled out was taken away `{ok, device, mount_point}`.
pub const E_MOUNTED: &str = "volume_mounted";
pub const E_UNMOUNTED: &str = "volume_unmounted";

/// Where a root shell mounts volumes, one folder per account.
const MEDIA_ROOT: &str = "/run/media";
/// The folders removable volumes are mounted under.
const MEDIA_ROOTS: [&str; 2] = ["/run/media", "/media"];
const SYS_BLOCK: &str = "/sys/block";
/// Where the kernel lists every block device, disk or partition, by name.
const SYS_CLASS_BLOCK: &str = "/sys/class/block";
/// How long after the last arrival a pass waits, in shell milliseconds: a disk's partitions appear after the disk.
pub const SETTLE_MS: f64 = 1_500.0;
/// The most volumes one disk gives.
const MAX_VOLUMES: usize = disk_image::MAX_VOLUMES;
/// The folder name of a volume without a usable label.
const UNTITLED: &str = "Untitled";
/// A shell running as the account asks the storage service, which may not have read a new device yet.
const PROBE_TRIES: u32 = 5;
const PROBE_WAIT: Duration = Duration::from_millis(300);

/// A partition as the kernel lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub number: u32,
    /// in bytes; 0 for an extended partition's placeholder
    pub size: u64,
    pub read_only: bool,
    /// another device is built on it (an opened encrypted container, a volume group)
    pub held: bool,
}

/// A whole disk as the kernel lists it, with its partitions by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockDisk {
    pub name: String,
    /// in bytes; 0 for a card reader or a drive without a medium
    pub size: u64,
    /// the kernel's `removable` flag
    pub removable: bool,
    /// on a bus things are plugged into ([`hotplug_bus`])
    pub hotplug: bool,
    pub read_only: bool,
    pub held: bool,
    pub parts: Vec<Part>,
}

/// Whether a disk whose device folder resolves to `resolved` (`/sys/devices/…/usb1/1-2/…/block/sdb`) sits on a bus
/// things are plugged into: any USB bus, or a memory-card host when the card's `kind` (its `device/type` file) says
/// it is a card in a slot (`SD`) and not storage soldered to the board (`MMC`).
pub fn hotplug_bus(resolved: &str, kind: &str) -> bool {
    let mut parts = resolved.split('/');
    let usb = |c: &str| c.strip_prefix("usb").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if parts.clone().any(usb) {
        return true;
    }
    parts.any(|c| c == "mmc_host") && kind.trim() == "SD"
}

/// Reads the disks and their partitions under `sys_block`, by name. Only plain kernel names are listed, so every
/// name that comes back may reach a command as `/dev/<name>`.
pub fn read_disks(sys_block: &Path) -> Vec<BlockDisk> {
    let text = |path: &Path| std::fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default();
    let bytes = |dir: &Path| text(&dir.join("size")).parse::<u64>().unwrap_or(0).saturating_mul(512);
    let held = |dir: &Path| std::fs::read_dir(dir.join("holders")).is_ok_and(|mut d| d.next().is_some());
    let plain = |name: &str| valid_device(&format!("/dev/{name}"));
    let mut names: Vec<String> = std::fs::read_dir(sys_block).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| plain(n)).collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let dir = sys_block.join(&name);
            let resolved = std::fs::canonicalize(&dir).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let mut parts: Vec<Part> = bifrost_core::disk_magic::partitions_of(sys_block, &name)
                .into_iter()
                .filter(|p| plain(p))
                .map(|p| {
                    let at = dir.join(&p);
                    Part { number: text(&at.join("partition")).parse().unwrap_or(0), size: bytes(&at), read_only: text(&at.join("ro")) == "1", held: held(&at), name: p }
                })
                .collect();
            parts.sort_by_key(|p| p.number);
            BlockDisk {
                size: bytes(&dir),
                removable: text(&dir.join("removable")) == "1",
                hotplug: hotplug_bus(&resolved, &text(&dir.join("device/type"))),
                read_only: text(&dir.join("ro")) == "1",
                held: held(&dir),
                parts,
                name,
            }
        })
        .collect()
}

/// Whether `disk` is one whose volumes are mounted for a person: removable or on a plug-in bus, with a medium in it.
/// A loop device (a disk image), a RAM disk, a device built from others and the boot and replay-protected areas of
/// soldered storage are never one.
pub fn eligible(disk: &BlockDisk) -> bool {
    let n = disk.name.as_str();
    let never = ["loop", "ram", "zram", "dm", "md", "nbd", "fd"].iter().any(|p| n.starts_with(p)) || (n.starts_with("mmcblk") && (n.contains("boot") || n.ends_with("rpmb")));
    !never && disk.size > 0 && (disk.removable || disk.hotplug)
}

/// True for a folder under a media folder (not the media folder itself).
fn in_media(point: &str) -> bool {
    MEDIA_ROOTS.iter().any(|root| Path::new(point).starts_with(root) && Path::new(point) != Path::new(root))
}

/// The folders the device `name` is mounted on in the mount table `table`.
fn points_of<'a>(table: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    table.iter().filter(|(d, _)| d.strip_prefix("/dev/") == Some(name)).map(|(_, p)| p.as_str()).collect()
}

/// The devices of one disk a pass reads: the disk itself when `whole` (it may hold a file system directly), and the
/// partitions in `parts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub disk: String,
    pub read_only: bool,
    pub whole: bool,
    pub parts: Vec<Part>,
}

/// The devices worth reading, from the kernel's `disks`, the mount table `mounts`, the swap list `swaps`
/// (`/proc/swaps`) and the names already decided on (`handled`). A disk is passed over whole when it is not
/// [`eligible`], was decided on, is mounted itself, or has any part in use by the system: mounted outside the media
/// folders, used as swap, or with another device built on it. Of the others, the partitions that are not mounted and
/// not decided on are wanted, at most [`MAX_VOLUMES`]; the disk itself is read too while none of its partitions is
/// mounted.
pub fn candidates(disks: &[BlockDisk], mounts: &str, swaps: &str, handled: &BTreeSet<String>) -> Vec<Wanted> {
    let table = bifrost_core::disk_scan::parse_mounts(mounts);
    let points = |name: &str| points_of(&table, name);
    let swap = |name: &str| swaps.lines().any(|l| l.split_whitespace().next().and_then(|d| d.strip_prefix("/dev/")) == Some(name));
    let system = |name: &str| swap(name) || points(name).iter().any(|p| !in_media(p));
    disks
        .iter()
        .filter(|d| eligible(d) && !handled.contains(&d.name))
        .filter(|d| !d.held && !d.parts.iter().any(|p| p.held))
        .filter(|d| !system(&d.name) && !d.parts.iter().any(|p| system(&p.name)))
        .filter(|d| points(&d.name).is_empty())
        .filter_map(|d| {
            let mounted = |p: &Part| !points(&p.name).is_empty();
            let parts: Vec<Part> = d.parts.iter().filter(|p| p.size > 0 && !mounted(p) && !handled.contains(&p.name)).take(MAX_VOLUMES).cloned().collect();
            // every partition is mounted or decided on: nothing is left on this disk
            if parts.is_empty() && !d.parts.is_empty() {
                return None;
            }
            Some(Wanted { disk: d.name.clone(), read_only: d.read_only, whole: !d.parts.iter().any(mounted), parts })
        })
        .collect()
}

/// What a device holds, read from the device itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// the kernel name (`sdb1`)
    pub name: String,
    /// the partition's number, `None` for a whole disk
    pub partition: Option<u32>,
    pub fs: String,
    pub label: String,
    /// the partition table's kind (`gpt`, `dos`), its type for this partition and, for `gpt`, its attribute bits
    pub scheme: String,
    pub entry_type: String,
    pub flags: u64,
    pub read_only: bool,
}

/// Reads what the device `name` holds from the `KEY=value` lines of `blkid -p -o export`.
pub fn parse_held(name: &str, partition: Option<u32>, read_only: bool, export: &str) -> Held {
    let flags = export_value(export, "PART_ENTRY_FLAGS");
    Held {
        name: name.to_string(),
        partition,
        fs: export_value(export, "TYPE"),
        label: export_value(export, "LABEL"),
        scheme: export_value(export, "PART_ENTRY_SCHEME"),
        entry_type: export_value(export, "PART_ENTRY_TYPE").to_ascii_lowercase(),
        flags: u64::from_str_radix(flags.trim_start_matches("0x"), 16).unwrap_or(0),
        read_only,
    }
}

/// The type the kernel mounts the probed file system `fs` as, `None` for everything that is left alone: swap, an
/// encrypted container, a volume-manager member, an unknown or empty type.
pub fn supported(fs: &str) -> Option<&'static str> {
    Some(match fs {
        "vfat" => "vfat",
        "exfat" => "exfat",
        "ntfs" | "ntfs3" => "ntfs3",
        "ext2" => "ext2",
        "ext3" => "ext3",
        "ext4" => "ext4",
        "btrfs" => "btrfs",
        "xfs" => "xfs",
        "f2fs" => "f2fs",
        "iso9660" => "iso9660",
        "udf" => "udf",
        _ => return None,
    })
}

/// Partition types of a GUID table that are not a person's volume: the EFI system partition, the firmware boot
/// partition, the reserved partition and the recovery partition.
const SYSTEM_GUIDS: [&str; 4] =
    ["c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "21686148-6449-6e6f-744e-656564454649", "e3c9e316-0b5c-4db8-817d-f92df00215ae", "de94bba4-06d1-4d40-a16a-bfd50179d6ac"];
/// Partition types of the older table that are not one: the EFI system partition, the vendor diagnostics and
/// recovery types, and the hidden forms of the ordinary types.
const SYSTEM_TYPES: [u32; 10] = [0xef, 0x12, 0x27, 0x11, 0x14, 0x16, 0x17, 0x1b, 0x1c, 0x1e];
/// The attribute bits of a GUID table entry that keep a partition out of sight: required by the platform (bit 0),
/// hidden (bit 62) and not to be mounted on its own (bit 63).
const HIDDEN_BITS: u64 = 1 | 1 << 62 | 1 << 63;

/// Whether the partition table marks `part` as a system or hidden partition.
pub fn hidden(part: &Held) -> bool {
    match part.scheme.as_str() {
        "gpt" => SYSTEM_GUIDS.contains(&part.entry_type.as_str()) || part.flags & HIDDEN_BITS != 0,
        "dos" => u32::from_str_radix(part.entry_type.trim_start_matches("0x"), 16).is_ok_and(|t| SYSTEM_TYPES.contains(&t)),
        _ => false,
    }
}

/// One volume to mount: its kernel name, the type it is mounted as, its folder name and whether it is read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub name: String,
    pub fs: &'static str,
    pub folder: String,
    pub read_only: bool,
}

/// The volume's folder name from its `label`: cleaned as a disk image's is (no `/`, no control characters, no
/// leading dots, at most 64 characters) and without leading hyphens; [`UNTITLED`] when nothing is left. A label is
/// never a path. A name in use gets a number when the folder is made.
pub fn folder_name(label: &str) -> String {
    let name = disk_image::clean_name(label);
    let name = name.trim_start_matches(|c: char| c == '.' || c == '-' || c.is_whitespace());
    if name.is_empty() { UNTITLED.to_string() } else { name.to_string() }
}

/// The volumes of one disk, from what the disk itself holds (`whole`, when it was read) and what its partitions
/// hold. A disk that carries a file system directly is the one volume, also with a partition table beside it (a
/// stick written from a disc image), as for a disk image. Otherwise each partition with a [`supported`] file system
/// that is not [`hidden`] is a volume, by number. Disc file systems and write-protected devices are read-only.
pub fn plan_mounts(whole: Option<&Held>, parts: &[Held]) -> Vec<Planned> {
    let planned = |h: &Held| supported(&h.fs).map(|fs| Planned { name: h.name.clone(), fs, folder: folder_name(&h.label), read_only: h.read_only || matches!(fs, "iso9660" | "udf") });
    if let Some(volume) = whole.and_then(planned) {
        return vec![volume];
    }
    let mut parts: Vec<&Held> = parts.iter().filter(|p| p.partition.is_some() && !hidden(p)).collect();
    parts.sort_by_key(|p| p.partition);
    parts.into_iter().filter_map(planned).take(MAX_VOLUMES).collect()
}

/// The options of a root shell's mount of a removable volume: never set-uid programs, device nodes or running a
/// program from it, read-only when asked. A file system without owners belongs to the account, and where it has a
/// mask for it (`vfat`, `exfat`, `ntfs3`) no other account gets in; one with owners keeps them.
pub fn mount_options(fs: &str, read_only: bool, uid: u32, gid: u32) -> String {
    let mut options = vec!["nosuid".to_string(), "nodev".to_string(), "noexec".to_string()];
    if read_only {
        options.push("ro".to_string());
    }
    if matches!(fs, "vfat" | "exfat" | "ntfs3" | "iso9660" | "udf") {
        options.push(format!("uid={uid}"));
        options.push(format!("gid={gid}"));
    }
    if matches!(fs, "vfat" | "exfat" | "ntfs3") {
        options.push("umask=0077".to_string());
    }
    options.join(",")
}

/// The arguments of a root shell's `mount` of `volume` on `folder`: the type is named, so no helper program is
/// looked up for it, and the device and folder come after `--`.
pub fn mount_arguments(volume: &Planned, folder: &str, uid: u32, gid: u32) -> Vec<String> {
    vec!["-t".to_string(), volume.fs.to_string(), "-o".to_string(), mount_options(volume.fs, volume.read_only, uid, gid), "--".to_string(), format!("/dev/{}", volume.name), folder.to_string()]
}

/// The mounts left behind by devices that are gone: every mount under a media folder in the table `mounts` whose
/// device is a plain kernel name that `present` no longer finds, as (device, mount point), the last mounted first.
/// A disk image's volume and a RAM disk are not looked at.
pub fn stale_mounts(mounts: &str, present: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = bifrost_core::disk_scan::parse_mounts(mounts)
        .into_iter()
        .filter(|(device, point)| {
            let name = device.strip_prefix("/dev/").unwrap_or_default();
            valid_device(device) && image_device_of(device).is_none() && !name.starts_with("ram") && !name.starts_with("zram") && in_media(point) && !present(name)
        })
        .collect();
    out.reverse();
    out
}

/// How one device of a pass ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    /// mounted on this folder
    Mounted(String),
    /// a volume that should have mounted did not: the reason
    Failed(String),
    /// nothing on it to mount
    Passed,
}

/// One device a pass decided on: its kernel name, the volume's name and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub device: String,
    pub name: String,
    pub done: Done,
}

/// What a pass did: the devices it decided on, and the mounts of pulled-out devices it took away (device, folder).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub volumes: Vec<Outcome>,
    pub removed: Vec<(String, String)>,
}

/// The `disks` events of a report: `volume_mounted` for each volume that mounted and `volume_unmounted` for each
/// mount taken away.
pub fn events(report: &Report) -> Vec<(&'static str, Json)> {
    let mounted = report.volumes.iter().filter_map(|v| match &v.done {
        Done::Mounted(point) => Some((E_MOUNTED, disk_image::outcome("device", &format!("/dev/{}", v.device), Ok(vec![("mount_point", point.clone()), ("name", v.name.clone())])))),
        _ => None,
    });
    let removed = report.removed.iter().map(|(device, point)| (E_UNMOUNTED, disk_image::outcome("device", device, Ok(vec![("mount_point", point.clone())]))));
    removed.chain(mounted).collect()
}

/// The one notice for the volumes of a pass that did not mount, `None` when all did.
pub fn failure_notice(report: &Report) -> Option<String> {
    let failed: Vec<(&Outcome, &str)> = report
        .volumes
        .iter()
        .filter_map(|v| match &v.done {
            Done::Failed(e) => Some((v, e.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it could not be mounted"))),
            _ => None,
        })
        .collect();
    let named = |v: &Outcome| format!("\u{201c}{}\u{201d} ({})", v.name, v.device);
    match failed.as_slice() {
        [] => None,
        [(v, reason)] => Some(format!("The volume {} could not be opened: {reason}", named(v))),
        many => Some(format!("{} volumes could not be opened: {}.", many.len(), many.iter().map(|(v, _)| named(v)).collect::<Vec<_>>().join(", "))),
    }
}

/// What a pass does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    /// mount the volumes that are waiting
    pub mount: bool,
    /// take away the mounts of devices that are gone
    pub sweep: bool,
}

/// The state of automatic mounting, kept by the `disks` service.
#[derive(Default)]
pub struct Auto {
    /// The kernel names decided on, each until it disappears.
    handled: BTreeSet<String>,
    /// Whether the first device list was seen.
    primed: bool,
    /// A device arrived or changed its medium since the last tick.
    changed: bool,
    /// A device went away since the last pass.
    sweep: bool,
    /// When the waiting volumes are mounted, in shell milliseconds.
    due: Option<f64>,
    running: Option<Receiver<Report>>,
}

impl Auto {
    /// Takes in one device scan: the `events` since the last one and the devices there now. The first scan asks for
    /// a pass over everything already plugged in. An arrival, or a medium put into a reader, asks for a mount; a
    /// device that went away is forgotten (it may be decided on again when it comes back) and asks for a sweep.
    /// Every device seen mounted counts as decided on, so one unmounted by hand is not mounted again.
    pub fn observe(&mut self, events: &[DiskEvent], disks: &[Disk]) {
        if !std::mem::replace(&mut self.primed, true) {
            self.changed = true;
            self.sweep = true;
        } else {
            for event in events {
                match event {
                    DiskEvent::Appeared(d) => {
                        // a disk that had nothing to mount got a partition: the disk is looked at again
                        self.handled.retain(|h| *h == d.bsd_name || !d.bsd_name.starts_with(h.as_str()));
                        self.changed = true;
                    }
                    DiskEvent::Disappeared(name) => {
                        self.handled.remove(name);
                        self.sweep = true;
                    }
                    DiskEvent::DescriptionChanged { before, after } if before.size != after.size => {
                        self.handled.remove(&after.bsd_name);
                        self.changed = true;
                    }
                    DiskEvent::DescriptionChanged { .. } => {}
                }
            }
        }
        self.handled.extend(disks.iter().filter(|d| d.mount_point.is_some()).map(|d| d.bsd_name.clone()));
    }

    /// The person asked to eject `device` (`/dev/sdb1`): it is not mounted again until it was unplugged.
    pub fn ejecting(&mut self, device: &str) {
        if let Some(name) = device.strip_prefix("/dev/").filter(|_| valid_device(device)) {
            self.handled.insert(name.to_string());
        }
    }

    /// Whether `name` was decided on and is left alone.
    pub fn is_handled(&self, name: &str) -> bool {
        self.handled.contains(name)
    }

    /// The pass to start at shell time `t`, if any. A mount waits [`SETTLE_MS`] after the last arrival and for the
    /// screen to be unlocked (`locked` is true at the login window too); a sweep does not wait. Nothing starts
    /// while a pass is running (`busy`).
    pub fn next_pass(&mut self, t: f64, locked: bool, busy: bool) -> Option<Pass> {
        if std::mem::take(&mut self.changed) {
            self.due = Some(t + SETTLE_MS);
        }
        if busy {
            return None;
        }
        let mount = !locked && self.due.is_some_and(|due| t >= due);
        if !mount && !self.sweep {
            return None;
        }
        if mount {
            self.due = None;
        }
        Some(Pass { mount, sweep: std::mem::take(&mut self.sweep) })
    }

    /// Takes in a finished pass: every device it decided on is left alone from now on.
    pub fn absorb(&mut self, report: &Report) {
        self.handled.extend(report.volumes.iter().map(|v| v.device.clone()));
    }

    /// Runs on the service's tick: collects the pass that ended, if one did, and starts the next one on a thread.
    pub fn tick(&mut self, t: f64, locked: bool) -> Option<Report> {
        let ended = match self.running.as_ref().map(Receiver::try_recv) {
            Some(Ok(report)) => Some(report),
            Some(Err(TryRecvError::Empty)) | None => None,
            Some(Err(TryRecvError::Disconnected)) => {
                eprintln!("disks: automount: the pass stopped before it finished");
                Some(Report::default())
            }
        };
        if let Some(report) = &ended {
            self.running = None;
            self.absorb(report);
        }
        if let Some(pass) = self.next_pass(t, locked, self.running.is_some()) {
            // a unit test never reads or mounts the disks of the machine it runs on
            if !cfg!(test) {
                let handled = self.handled.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(run_pass(pass, &handled));
                });
                self.running = Some(rx);
            }
        }
        ended
    }
}

/// One pass, on its own thread: the sweep first, then the mounts.
fn run_pass(pass: Pass, handled: &BTreeSet<String>) -> Report {
    let as_root = crate::privd::server::euid() == 0;
    let mut report = Report::default();
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    if pass.sweep && as_root {
        // a shell running as the account leaves this to the storage service, which made those mounts
        for (device, point) in stale_mounts(&mounts, |name| Path::new(SYS_CLASS_BLOCK).join(name).exists()) {
            match run("umount", &["-l", "--", point.as_str()]) {
                Ok(_) => {
                    let _ = std::fs::remove_dir(&point);
                    report.removed.push((device, point));
                }
                Err(e) => eprintln!("disks: automount: {device} is gone, but its mount on {point} could not be taken away: {e}"),
            }
        }
    }
    if !pass.mount {
        return report;
    }
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    let wanted = candidates(&read_disks(Path::new(SYS_BLOCK)), &mounts, &swaps, handled);
    if wanted.is_empty() {
        return report;
    }
    let account = match Account::installed() {
        Ok(account) => account,
        Err(e) => {
            eprintln!("disks: automount: no account to mount for: {e}");
            return report;
        }
    };
    if !as_root {
        // the storage service learns of a new device from udev
        let _ = std::process::Command::new("udevadm").args(["settle", "--timeout=5"]).output();
    }
    let probe = |name: &str, partition: Option<u32>, read_only: bool| {
        let device = format!("/dev/{name}");
        parse_held(name, partition, read_only, &run("blkid", &["-p", "-o", "export", "--", device.as_str()]).unwrap_or_default())
    };
    for disk in &wanted {
        let whole = disk.whole.then(|| probe(&disk.disk, None, disk.read_only)).filter(|h| supported(&h.fs).is_some());
        let parts: Vec<Held> = if whole.is_some() { Vec::new() } else { disk.parts.iter().map(|p| probe(&p.name, Some(p.number), disk.read_only || p.read_only)).collect() };
        let plan = plan_mounts(whole.as_ref(), &parts);
        // what was read and holds nothing to mount is decided on too: the disk itself only when it has no partitions
        let read: Vec<&str> = if whole.is_some() || disk.parts.is_empty() { vec![disk.disk.as_str()] } else { disk.parts.iter().map(|p| p.name.as_str()).collect() };
        for name in read.into_iter().filter(|n| !plan.iter().any(|v| v.name == *n)) {
            report.volumes.push(Outcome { device: name.to_string(), name: String::new(), done: Done::Passed });
        }
        for volume in &plan {
            let done = match mount_one(volume, &account, as_root) {
                Ok(folder) => Done::Mounted(folder),
                Err(e) => {
                    eprintln!("disks: automount: /dev/{} ({}) not mounted: {e}", volume.name, volume.fs);
                    Done::Failed(e)
                }
            };
            report.volumes.push(Outcome { device: volume.name.clone(), name: volume.folder.clone(), done });
        }
    }
    report
}

/// Mounts one planned volume and returns its folder. A root shell mounts it under the account's media folder, as it
/// does a disk image's volume; a shell running as the account asks the storage service, which picks the folder and
/// the options.
fn mount_one(volume: &Planned, account: &Account, as_root: bool) -> Result<String, String> {
    let device = format!("/dev/{}", volume.name);
    if !as_root {
        let out = disk_image::retry_until_probed(PROBE_TRIES, PROBE_WAIT, || run("udisksctl", &["mount", "--no-user-interaction", "-b", device.as_str()]))?;
        return disk_image::mounted_at(&out).ok_or_else(|| format!("the volume on {device} did not mount"));
    }
    let root = Path::new(MEDIA_ROOT).join(&account.name);
    disk_image::make_media_root(&root, account.gid)?;
    let point = disk_image::fresh_folder(&root, &volume.folder)?;
    let folder = point.to_string_lossy().into_owned();
    let args = mount_arguments(volume, &folder, account.uid, account.gid);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run("mount", &args) {
        Ok(_) => Ok(folder),
        Err(e) => {
            let _ = std::fs::remove_dir(&point);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn part(name: &str, number: u32) -> Part {
        Part { name: name.into(), number, size: GB, ..Part::default() }
    }

    fn stick(name: &str, parts: &[&str]) -> BlockDisk {
        BlockDisk { name: name.into(), size: 8 * GB, removable: true, hotplug: true, parts: parts.iter().enumerate().map(|(i, p)| part(p, i as u32 + 1)).collect(), ..BlockDisk::default() }
    }

    fn names(wanted: &[Wanted]) -> Vec<(String, bool, Vec<String>)> {
        wanted.iter().map(|w| (w.disk.clone(), w.whole, w.parts.iter().map(|p| p.name.clone()).collect())).collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_disk_on_a_plug_in_bus_is_told_from_one_built_in() {
        let usb = "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host4/target4:0:0/4:0:0:0/block/sdb";
        assert!(hotplug_bus(usb, ""));
        assert!(hotplug_bus("/sys/devices/platform/dummy_hcd.0/usb1/1-1/1-1:1.0/host0/target0:0:0/0:0:0:0/block/sda", ""));
        assert!(!hotplug_bus("/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda", ""));
        assert!(!hotplug_bus("/sys/devices/pci0000:00/0000:00:1d.0/0000:3d:00.0/nvme/nvme0/nvme0n1", ""));
        // a folder that only starts like a bus is not one
        assert!(!hotplug_bus("/sys/devices/platform/usbmisc/x/block/sda", ""));
        assert!(!hotplug_bus("/sys/devices/platform/usb/x/block/sda", ""));
        // a card in a slot, against storage soldered to the board on the same kind of host
        let card = "/sys/devices/platform/soc/11230000.mmc/mmc_host/mmc1/mmc1:aaaa/block/mmcblk1";
        assert!(hotplug_bus(card, "SD\n"));
        assert!(!hotplug_bus(card, "MMC\n"));
        assert!(!hotplug_bus(card, ""));
        assert!(!hotplug_bus("", "SD"));
    }

    #[test]
    fn only_removable_disks_with_a_medium_are_eligible() {
        assert!(eligible(&stick("sdb", &["sdb1"])));
        // a disk in a plug-in case is not marked removable by the kernel, a card reader's slot is
        assert!(eligible(&BlockDisk { removable: false, ..stick("sdc", &[]) }));
        assert!(eligible(&BlockDisk { hotplug: false, ..stick("sdc", &[]) }));
        assert!(!eligible(&BlockDisk { removable: false, hotplug: false, ..stick("sda", &[]) }), "a built-in disk");
        assert!(!eligible(&BlockDisk { size: 0, ..stick("sdd", &[]) }), "a reader without a card");
        for never in ["loop0", "ram0", "zram0", "dm0", "md0", "nbd0", "fd0", "mmcblk0boot0", "mmcblk0boot1", "mmcblk0rpmb"] {
            assert!(!eligible(&stick(never, &[])), "{never}");
        }
        assert!(eligible(&stick("mmcblk1", &["mmcblk1p1"])));
        assert!(eligible(&stick("sr0", &[])), "a disc in a drive");
    }

    #[test]
    fn the_devices_come_from_the_kernels_list() {
        let sys = std::env::temp_dir().join(format!("volume-mount-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sys);
        let put = |path: &str, text: &str| {
            let file = sys.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        put("sdb/size", "2000\n");
        put("sdb/removable", "1\n");
        put("sdb/ro", "0\n");
        put("sdb/sdb2/partition", "2\n");
        put("sdb/sdb2/size", "500\n");
        put("sdb/sdb2/ro", "1\n");
        put("sdb/sdb1/partition", "1\n");
        put("sdb/sdb1/size", "1000\n");
        put("sdb/sdb1/holders/dm-0/x", "");
        put("sdb/queue/x", "");
        put("sda/size", "9000\n");
        put("sda/removable", "0\n");
        std::fs::create_dir_all(sys.join("sda/holders")).unwrap();
        put("dm-0/size", "10\n");
        let disks = read_disks(&sys);
        std::fs::remove_dir_all(&sys).unwrap();
        assert_eq!(disks.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["sda", "sdb"], "a name that is not plain is not listed");
        assert_eq!((disks[0].removable, disks[0].size, disks[0].parts.len()), (false, 9000 * 512, 0));
        assert!(!disks[0].held, "an empty holders folder");
        let b = &disks[1];
        assert!(b.removable && !b.hotplug && !b.read_only && !b.held);
        assert_eq!(b.parts, vec![Part { name: "sdb1".into(), number: 1, size: 1000 * 512, read_only: false, held: true }, Part { name: "sdb2".into(), number: 2, size: 500 * 512, read_only: true, held: false }]);
        assert!(read_disks(Path::new("/nonexistent/sys/block")).is_empty());
    }

    #[test]
    fn a_plugged_in_stick_is_wanted_and_the_system_disk_never_is() {
        let disks = [
            BlockDisk { removable: false, hotplug: false, ..stick("nvme0n1", &["nvme0n1p1", "nvme0n1p2"]) },
            stick("sdb", &["sdb1"]),
            stick("sdc", &[]),
            stick("loop0", &["loop0p1"]),
        ];
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/nvme0n1p1 /boot vfat rw 0 0\n";
        let wanted = candidates(&disks, table, "", &BTreeSet::new());
        assert_eq!(names(&wanted), vec![("sdb".to_string(), true, vec!["sdb1".to_string()]), ("sdc".to_string(), true, vec![])]);
        // the system started from a stick: none of that disk is touched, whatever else is on it
        let live = [stick("sda", &["sda1", "sda2", "sda3"])];
        assert!(candidates(&live, "/dev/sda2 / ext4 rw 0 0\n", "", &BTreeSet::new()).is_empty());
        assert!(candidates(&live, "/dev/sda2 /home/me/mnt ext4 rw 0 0\n", "", &BTreeSet::new()).is_empty(), "mounted by hand outside the media folders");
        assert!(candidates(&live, "/dev/sda2 /run/media ext4 rw 0 0\n", "", &BTreeSet::new()).is_empty(), "the media folder itself is not a volume's folder");
        let swaps = "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/dev/sda3                               partition\t8388604\t\t0\t\t-2\n";
        assert!(candidates(&live, "", swaps, &BTreeSet::new()).is_empty(), "a partition in use as swap");
        assert_eq!(candidates(&live, "", "/dev/sda33 partition 1 0 -2\n", &BTreeSet::new()).len(), 1, "only the whole name matches");
        // another device is built on a partition (an opened encrypted container holds the system)
        let mut built = stick("sda", &["sda1", "sda2"]);
        built.parts[1].held = true;
        assert!(candidates(&[built], "", "", &BTreeSet::new()).is_empty());
        assert!(candidates(&[BlockDisk { held: true, ..stick("sda", &[]) }], "", "", &BTreeSet::new()).is_empty());
    }

    #[test]
    fn what_is_mounted_or_decided_on_is_not_wanted_again() {
        let card = [stick("sdc", &["sdc1", "sdc2", "sdc3"])];
        // one partition is mounted already: the others are wanted, and the disk itself is not read as a volume
        let table = "/dev/sdc1 /run/media/me/Card vfat rw 0 0\n";
        assert_eq!(names(&candidates(&card, table, "", &BTreeSet::new())), vec![("sdc".to_string(), false, vec!["sdc2".to_string(), "sdc3".to_string()])]);
        // an ejected partition stays out; with every partition decided on nothing is left
        assert_eq!(names(&candidates(&card, table, "", &set(&["sdc2"]))), vec![("sdc".to_string(), false, vec!["sdc3".to_string()])]);
        assert!(candidates(&card, table, "", &set(&["sdc2", "sdc3"])).is_empty());
        assert!(candidates(&card, "", "", &set(&["sdc1", "sdc2", "sdc3"])).is_empty());
        // a disk decided on as a whole (a volume without a partition table, ejected) is left alone with all its parts
        assert!(candidates(&card, "", "", &set(&["sdc"])).is_empty());
        // a disk mounted as a whole is one volume: its partitions are views of the same data
        assert!(candidates(&card, "/dev/sdc /media/Disc iso9660 ro 0 0\n", "", &BTreeSet::new()).is_empty());
        // an extended partition's placeholder has no size and is not read
        let mut old = stick("sdd", &["sdd1", "sdd5"]);
        old.parts[0].size = 0;
        assert_eq!(names(&candidates(&[old], "", "", &BTreeSet::new())), vec![("sdd".to_string(), true, vec!["sdd5".to_string()])]);
        // a table with more partitions than anyone opens is cut off
        let many: Vec<String> = (1..=40).map(|n| format!("sde{n}")).collect();
        let many = stick("sde", &many.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(candidates(&[many], "", "", &BTreeSet::new())[0].parts.len(), MAX_VOLUMES);
    }

    #[test]
    fn a_probe_reads_the_file_system_and_the_partition_tables_marks() {
        let text = "DEVNAME=/dev/sda1\nLABEL=E2EUSB\nUUID=1234-ABCD\nTYPE=vfat\nUSAGE=filesystem\nPART_ENTRY_SCHEME=dos\nPART_ENTRY_TYPE=0xc\nPART_ENTRY_FLAGS=0x80\nPART_ENTRY_NUMBER=1\n";
        let held = parse_held("sda1", Some(1), false, text);
        assert_eq!(held, Held { name: "sda1".into(), partition: Some(1), fs: "vfat".into(), label: "E2EUSB".into(), scheme: "dos".into(), entry_type: "0xc".into(), flags: 0x80, read_only: false });
        let efi = parse_held("sdb1", Some(1), true, "LABEL=My\\ Disk\nTYPE=vfat\nPART_ENTRY_SCHEME=gpt\nPART_ENTRY_TYPE=C12A7328-F81F-11D2-BA4B-00A0C93EC93B\nPART_ENTRY_FLAGS=0x8000000000000001\n");
        assert_eq!((efi.label.as_str(), efi.entry_type.as_str(), efi.flags, efi.read_only), ("My Disk", "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", 0x8000000000000001, true));
        // a probe that failed, and a disk with a partition table and nothing else
        assert_eq!(parse_held("sdc", None, false, ""), Held { name: "sdc".into(), ..Held::default() });
        assert_eq!(parse_held("sdc", None, false, "PTUUID=abc\nPTTYPE=gpt\n").fs, "");
        assert_eq!(parse_held("sdc1", Some(1), false, "PART_ENTRY_FLAGS=junk\n").flags, 0);
    }

    #[test]
    fn only_the_listed_file_systems_are_mounted() {
        for (fs, kernel) in [("vfat", "vfat"), ("exfat", "exfat"), ("ntfs", "ntfs3"), ("ext2", "ext2"), ("ext3", "ext3"), ("ext4", "ext4"), ("btrfs", "btrfs"), ("xfs", "xfs"), ("f2fs", "f2fs"), ("iso9660", "iso9660"), ("udf", "udf")] {
            assert_eq!(supported(fs), Some(kernel), "{fs}");
        }
        for other in ["", "swap", "crypto_LUKS", "LVM2_member", "linux_raid_member", "zfs_member", "hfsplus", "apfs", "bcachefs", "VFAT", "ext4 ", "../ext4"] {
            assert_eq!(supported(other), None, "{other}");
        }
    }

    fn holds(name: &str, partition: Option<u32>, fs: &str, label: &str) -> Held {
        Held { name: name.into(), partition, fs: fs.into(), label: label.into(), ..Held::default() }
    }

    fn typed(name: &str, number: u32, scheme: &str, entry_type: &str, flags: u64) -> Held {
        Held { scheme: scheme.into(), entry_type: entry_type.into(), flags, ..holds(name, Some(number), "vfat", "") }
    }

    #[test]
    fn system_and_hidden_partitions_are_told_from_a_persons_volumes() {
        let data = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
        assert!(!hidden(&typed("sdb1", 1, "gpt", data, 0)));
        assert!(!hidden(&typed("sdb1", 1, "gpt", "0fc63daf-8483-4772-8e79-3d69d8477de4", 0)));
        for system in SYSTEM_GUIDS {
            assert!(hidden(&typed("sdb1", 1, "gpt", system, 0)), "{system}");
        }
        for bit in [0u32, 62, 63] {
            assert!(hidden(&typed("sdb1", 1, "gpt", data, 1 << bit)), "bit {bit}");
        }
        assert!(!hidden(&typed("sdb1", 1, "gpt", data, 1 << 60)), "a read-only partition is still a volume");
        for plain in ["0xc", "0xb", "0x7", "0x83", "0x6", "0xe"] {
            assert!(!hidden(&typed("sdb1", 1, "dos", plain, 0x80)), "{plain}: the boot mark of the older table hides nothing");
        }
        for system in ["0xef", "0x12", "0x27", "0x1b", "0x1c", "0x17", "0x11", "0x14", "0x16", "0x1e"] {
            assert!(hidden(&typed("sdb1", 1, "dos", system, 0)), "{system}");
        }
        assert!(!hidden(&typed("sdb1", 1, "dos", "", 0)) && !hidden(&typed("sdb1", 1, "", "0xef", 1)), "no table, no marks");
    }

    fn planned(name: &str, fs: &'static str, folder: &str, read_only: bool) -> Planned {
        Planned { name: name.into(), fs, folder: folder.into(), read_only }
    }

    #[test]
    fn each_partition_a_person_can_use_is_a_volume() {
        // the stick of the report: one FAT partition with a label
        let whole = holds("sda", None, "", "");
        assert_eq!(plan_mounts(Some(&whole), &[holds("sda1", Some(1), "vfat", "E2EUSB")]), vec![planned("sda1", "vfat", "E2EUSB", false)]);
        let efi = typed("sdb1", 1, "gpt", "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", 0);
        let parts = [
            holds("sdb4", Some(4), "swap", ""),
            holds("sdb3", Some(3), "ntfs", "Data"),
            efi,
            holds("sdb2", Some(2), "ext4", ""),
            holds("sdb5", Some(5), "crypto_LUKS", ""),
            holds("sdb6", Some(6), "LVM2_member", ""),
            holds("sdb7", Some(7), "", ""),
            Held { read_only: true, ..holds("sdb8", Some(8), "exfat", " ../\n") },
            holds("sdb9", Some(9), "udf", "-rf"),
        ];
        assert_eq!(
            plan_mounts(Some(&whole), &parts),
            vec![planned("sdb2", "ext4", "Untitled", false), planned("sdb3", "ntfs3", "Data", false), planned("sdb8", "exfat", "Untitled", true), planned("sdb9", "udf", "rf", true)],
            "by number; no boot partition, swap, encrypted container or unknown content; a disc file system is read-only"
        );
        assert!(plan_mounts(None, &[holds("sdb1", Some(1), "crypto_LUKS", "secret")]).is_empty(), "an encrypted container is left unmounted");
        assert!(plan_mounts(Some(&whole), &[]).is_empty());
        assert!(plan_mounts(None, &[holds("sdb", None, "vfat", "")]).is_empty(), "a whole disk is not a partition");
    }

    #[test]
    fn a_disk_that_is_one_file_system_is_one_volume() {
        // a stick written from a disc image: the file system on the disk itself, a partition table beside it
        let disc = holds("sdb", None, "iso9660", "INSTALL");
        let beside = [holds("sdb1", Some(1), "iso9660", "INSTALL"), holds("sdb2", Some(2), "vfat", "EFI")];
        assert_eq!(plan_mounts(Some(&disc), &beside), vec![planned("sdb", "iso9660", "INSTALL", true)]);
        // a card without a partition table, write-protected by its switch
        let card = Held { read_only: true, ..holds("mmcblk1", None, "vfat", "") };
        assert_eq!(plan_mounts(Some(&card), &[]), vec![planned("mmcblk1", "vfat", "Untitled", true)]);
        assert!(plan_mounts(Some(&holds("sdb", None, "crypto_LUKS", "")), &[]).is_empty());
    }

    #[test]
    fn a_label_is_a_folder_name_and_never_a_path() {
        assert_eq!(folder_name("E2EUSB"), "E2EUSB");
        assert_eq!(folder_name("My Stick"), "My Stick");
        assert_eq!(folder_name(" ../../etc/passwd\n"), "etcpasswd");
        assert_eq!(folder_name(".."), "Untitled");
        assert_eq!(folder_name("/"), "Untitled");
        assert_eq!(folder_name(""), "Untitled");
        assert_eq!(folder_name("-o exec"), "o exec");
        assert_eq!(folder_name(".-.- x"), "x");
        assert_eq!(folder_name("a-b.c"), "a-b.c", "only leading dots and hyphens go");
        assert_eq!(folder_name(&"x".repeat(200)).chars().count(), 64);
        assert_eq!(folder_name("Gögn"), "Gögn");
    }

    #[test]
    fn a_removable_volume_is_mounted_for_the_account_and_never_trusted() {
        assert_eq!(mount_options("vfat", false, 1000, 1001), "nosuid,nodev,noexec,uid=1000,gid=1001,umask=0077");
        assert_eq!(mount_options("exfat", true, 1000, 1000), "nosuid,nodev,noexec,ro,uid=1000,gid=1000,umask=0077");
        assert_eq!(mount_options("ntfs3", false, 1000, 1000), "nosuid,nodev,noexec,uid=1000,gid=1000,umask=0077");
        assert_eq!(mount_options("iso9660", true, 1000, 1000), "nosuid,nodev,noexec,ro,uid=1000,gid=1000", "this one has no mask option");
        for owned in ["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs"] {
            assert_eq!(mount_options(owned, false, 1000, 1000), "nosuid,nodev,noexec", "{owned} keeps its own owners");
        }
        let args = mount_arguments(&planned("sda1", "vfat", "E2EUSB", false), "/run/media/me/E2EUSB", 1000, 1000);
        assert_eq!(args, ["-t", "vfat", "-o", "nosuid,nodev,noexec,uid=1000,gid=1000,umask=0077", "--", "/dev/sda1", "/run/media/me/E2EUSB"]);
    }

    #[test]
    fn the_mounts_of_a_pulled_out_disk_are_found_and_nothing_else() {
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n\
                     /dev/sdb1 /run/media/me/My\\040Stick vfat rw 0 0\n\
                     /dev/sdc1 /media/Card exfat rw 0 0\n\
                     /dev/sdc2 /run/media/me/Card\\0402 ext4 rw 0 0\n\
                     /dev/sdd1 /home/me/mnt ext4 rw 0 0\n\
                     /dev/sdf1 /run/media vfat rw 0 0\n\
                     /dev/loop0p1 /run/media/me/BOOT vfat ro 0 0\n\
                     /dev/zram0 /run/media/me/Z ext4 rw 0 0\n\
                     /dev/mapper/x /run/media/me/M ext4 rw 0 0\n\
                     tmpfs /run/media/me/T tmpfs rw 0 0\n\
                     /dev/sde1 /run/media/me/Here vfat rw 0 0\n";
        let present = |name: &str| ["nvme0n1p2", "sde1"].contains(&name);
        let gone = stale_mounts(table, present);
        let pair = |d: &str, p: &str| (d.to_string(), p.to_string());
        assert_eq!(gone, vec![pair("/dev/sdc2", "/run/media/me/Card 2"), pair("/dev/sdc1", "/media/Card"), pair("/dev/sdb1", "/run/media/me/My Stick")], "the last mounted first");
        assert!(stale_mounts(table, |_| true).is_empty());
        assert!(stale_mounts("", |_| false).is_empty());
    }

    fn disk(name: &str, size: u64, mount_point: Option<&str>) -> Disk {
        Disk { bsd_name: name.into(), size, removable: true, read_only: false, model: String::new(), mount_point: mount_point.map(String::from) }
    }

    #[test]
    fn the_volumes_already_plugged_in_wait_for_the_unlock() {
        let mut auto = Auto::default();
        auto.observe(&[], &[disk("sdb", GB, None), disk("sdb1", GB, None)]);
        // at the login window: only the sweep for mounts an earlier run left behind
        assert_eq!(auto.next_pass(1_000.0, true, false), Some(Pass { mount: false, sweep: true }));
        assert_eq!(auto.next_pass(60_000.0, true, false), None);
        // signed in: the waiting volumes are mounted, once
        assert_eq!(auto.next_pass(61_000.0, false, false), Some(Pass { mount: true, sweep: false }));
        assert_eq!(auto.next_pass(120_000.0, false, false), None);
        auto.observe(&[], &[disk("sdb", GB, None), disk("sdb1", GB, None)]);
        assert_eq!(auto.next_pass(122_000.0, false, false), None, "a scan without a change asks for nothing");
    }

    #[test]
    fn an_arrival_is_mounted_a_moment_after_the_last_one() {
        let mut auto = Auto::default();
        auto.observe(&[], &[]);
        let _ = auto.next_pass(0.0, false, false);
        let _ = auto.next_pass(SETTLE_MS, false, false);
        auto.observe(&[DiskEvent::Appeared(disk("sdb", GB, None))], &[]);
        assert_eq!(auto.next_pass(10_000.0, false, false), None, "the partitions are not there yet");
        auto.observe(&[DiskEvent::Appeared(disk("sdb1", GB, None))], &[]);
        assert_eq!(auto.next_pass(11_000.0, false, false), None, "the wait starts again");
        assert_eq!(auto.next_pass(11_000.0 + SETTLE_MS - 1.0, false, false), None);
        // a pass still running holds the next one back and loses nothing
        assert_eq!(auto.next_pass(11_000.0 + SETTLE_MS, false, true), None);
        assert_eq!(auto.next_pass(11_000.0 + SETTLE_MS, false, false), Some(Pass { mount: true, sweep: false }));
        // plugged in while locked: mounted at the unlock
        auto.observe(&[DiskEvent::Appeared(disk("sdc", GB, None))], &[]);
        assert_eq!(auto.next_pass(20_000.0, true, false), None);
        assert_eq!(auto.next_pass(90_000.0, true, false), None);
        assert_eq!(auto.next_pass(90_001.0, false, false), Some(Pass { mount: true, sweep: false }));
    }

    #[test]
    fn an_ejected_volume_is_left_alone_until_it_is_plugged_in_again() {
        let mut auto = Auto::default();
        auto.observe(&[], &[disk("sdb", GB, None), disk("sdb1", GB, Some("/run/media/me/Stick"))]);
        assert!(auto.is_handled("sdb1"), "a volume seen mounted is decided on");
        assert!(!auto.is_handled("sdb"));
        auto.ejecting("/dev/sdc1");
        assert!(auto.is_handled("sdc1"));
        for bad in ["/dev/../etc", "sdc2", "/run/media/me/Stick", "/dev/"] {
            auto.ejecting(bad);
        }
        assert_eq!(auto.handled, set(&["sdb1", "sdc1"]));
        // unmounted, the disk still plugged in (its power stayed on): scans come and go, it stays decided on
        auto.observe(&[], &[disk("sdb", GB, None), disk("sdb1", GB, None), disk("sdc1", GB, None)]);
        assert!(auto.is_handled("sdb1") && auto.is_handled("sdc1"));
        let stick = [stick("sdc", &["sdc1"])];
        assert!(candidates(&stick, "", "", &auto.handled).is_empty());
        // pulled out: forgotten, and its mounts are swept
        let _ = auto.next_pass(0.0, true, false);
        auto.observe(&[DiskEvent::Disappeared("sdc1".into()), DiskEvent::Disappeared("sdc".into())], &[]);
        assert!(!auto.is_handled("sdc1"));
        assert_eq!(auto.next_pass(1.0, true, false), Some(Pass { mount: false, sweep: true }), "a sweep runs while locked too");
        assert_eq!(candidates(&stick, "", "", &auto.handled).len(), 1, "plugged in again, it is mounted again");
    }

    #[test]
    fn a_volume_that_failed_is_not_tried_again_and_a_new_medium_is() {
        let mut auto = Auto::default();
        let report = Report {
            volumes: vec![
                Outcome { device: "sdb1".into(), name: "Stick".into(), done: Done::Failed("wrong fs type".into()) },
                Outcome { device: "sdb2".into(), name: String::new(), done: Done::Passed },
                Outcome { device: "sdc".into(), name: String::new(), done: Done::Passed },
                Outcome { device: "sdd1".into(), name: "Card".into(), done: Done::Mounted("/run/media/me/Card".into()) },
            ],
            removed: Vec::new(),
        };
        auto.absorb(&report);
        assert_eq!(auto.handled, set(&["sdb1", "sdb2", "sdc", "sdd1"]));
        assert!(candidates(&[stick("sdb", &["sdb1", "sdb2"])], "", "", &auto.handled).is_empty(), "no loop over a volume that does not mount");
        auto.observe(&[], &[]);
        // the empty disk got a partition table: the disk is looked at again, the others are not
        auto.observe(&[DiskEvent::Appeared(disk("sdc1", GB, None))], &[]);
        assert_eq!(auto.handled, set(&["sdb1", "sdb2", "sdd1"]));
        // a card was put into the reader: its size changed
        auto.absorb(&Report { volumes: vec![Outcome { device: "sde".into(), name: String::new(), done: Done::Passed }], removed: Vec::new() });
        let _ = auto.next_pass(0.0, false, false);
        let _ = auto.next_pass(SETTLE_MS, false, false);
        auto.observe(&[DiskEvent::DescriptionChanged { before: disk("sde", 0, None), after: disk("sde", GB, None) }], &[]);
        assert!(!auto.is_handled("sde"));
        assert_eq!(auto.next_pass(10_000.0 + SETTLE_MS, false, false), None, "the wait starts at the tick that saw the change");
        assert_eq!(auto.next_pass(10_000.0 + 2.0 * SETTLE_MS, false, false), Some(Pass { mount: true, sweep: false }));
        // a mount or an unmount changes a description too, and asks for nothing
        auto.observe(&[DiskEvent::DescriptionChanged { before: disk("sdd1", GB, Some("/run/media/me/Card")), after: disk("sdd1", GB, None) }], &[]);
        assert!(auto.is_handled("sdd1"));
        assert_eq!(auto.next_pass(99_000.0, false, false), None);
    }

    #[test]
    fn a_report_gives_the_events_and_one_notice_for_what_failed() {
        let outcome = |device: &str, name: &str, done: Done| Outcome { device: device.into(), name: name.into(), done };
        let mut report = Report {
            volumes: vec![outcome("sda1", "E2EUSB", Done::Mounted("/run/media/me/E2EUSB".into())), outcome("sda2", "", Done::Passed)],
            removed: vec![("/dev/sdb1".into(), "/run/media/me/Old".into())],
        };
        let sent = events(&report);
        assert_eq!(sent.iter().map(|(e, _)| *e).collect::<Vec<_>>(), [E_UNMOUNTED, E_MOUNTED]);
        assert_eq!(sent[0].1.str_field("device").as_deref(), Some("/dev/sdb1"));
        assert_eq!(sent[0].1.str_field("mount_point").as_deref(), Some("/run/media/me/Old"));
        assert_eq!(sent[1].1.get("ok"), Some(&Json::Bool(true)));
        assert_eq!(sent[1].1.str_field("device").as_deref(), Some("/dev/sda1"));
        assert_eq!(sent[1].1.str_field("mount_point").as_deref(), Some("/run/media/me/E2EUSB"));
        assert_eq!(sent[1].1.str_field("name").as_deref(), Some("E2EUSB"));
        assert_eq!(failure_notice(&report), None);
        report.volumes.push(outcome("sdc1", "Stick", Done::Failed("\nmount: /run/media/me/Stick: wrong fs type.\nmore".into())));
        assert_eq!(failure_notice(&report).as_deref(), Some("The volume \u{201c}Stick\u{201d} (sdc1) could not be opened: mount: /run/media/me/Stick: wrong fs type."));
        assert_eq!(events(&report).len(), 2, "a failure is a notice, not an event");
        report.volumes.push(outcome("sdc2", "Untitled", Done::Failed(String::new())));
        assert_eq!(failure_notice(&report).as_deref(), Some("2 volumes could not be opened: \u{201c}Stick\u{201d} (sdc1), \u{201c}Untitled\u{201d} (sdc2)."));
        report.volumes.truncate(2);
        report.volumes.push(outcome("sdc2", "Untitled", Done::Failed("  ".into())));
        assert_eq!(failure_notice(&report).as_deref(), Some("The volume \u{201c}Untitled\u{201d} (sdc2) could not be opened: it could not be mounted"));
    }

    #[test]
    fn a_tick_in_a_test_starts_no_pass_on_the_machine() {
        let mut auto = Auto::default();
        auto.observe(&[], &[]);
        assert_eq!(auto.tick(0.0, false), None);
        assert_eq!(auto.tick(10_000.0, false), None);
        assert!(auto.running.is_none());
        // a pass whose thread died is over, and the next one may start
        let (tx, rx) = channel::<Report>();
        drop(tx);
        auto.running = Some(rx);
        assert_eq!(auto.tick(20_000.0, false), Some(Report::default()));
        assert!(auto.running.is_none());
    }
}
