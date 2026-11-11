//! Mounting removable volumes (a USB stick, a memory card) when they are plugged in, and for the ones already there
//! when the shell starts or the account signs in. The eject is `shell::volume_eject`; a disk image has its own path
//! (`shell::disk_image`) and is never touched here.
//!
//! The `disks` service drives it ([`Auto`]): its device scan reports what appeared, went away or changed, and a
//! moment after the last arrival (partitions show up after their disk) one pass runs on a thread, the way the disk
//! image requests do. Every device name of a pass comes from the kernel's list under `/sys/block`.
//!
//! The scan runs when the kernel made or removed a device node (`shell::block_watch`), so a plug is seen at once and
//! an idle shell is not woken to look for one. The watch also names each node that left, which the scan alone cannot
//! when a disk is pulled and plugged in again between two reads: such a name is forgotten like any other that left
//! ([`Auto::unplugged`]), and a mount its old device left behind is taken away although the name is there again.
//! While something is waiting the service asks the shell's loop for its next wake-up ([`Auto::next_wake`]): the end
//! of the settling time, and a look now and then for the end of a running pass. With nothing waiting it asks for
//! none.
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
//! Nothing is mounted while the screen is locked or the login window is up; the pass waits for the unlock and starts
//! on the first tick after it. A pass that was already running when the screen locked finishes what it started; no
//! new one starts. Volumes that are mounted when the screen locks stay mounted: the lock hides the session, it does
//! not end it, and a copy to the stick goes on behind it.
//!
//! This is a single-account computer (`Account::installed`): Log Out closes the apps and returns to the login window
//! of the same shell, and the next person to sign in is the same account. So the volumes stay mounted over a log out
//! too, as they do over a lock. They are the account's alone meanwhile: the media folder is `root:<group>` 0750 and a
//! file system without owners is mounted with the account's ids and no access for others. There is no second account
//! that could find them. When accounts are added, the end of a session has to unmount what was mounted for it.
//!
//! A device that was decided on once (mounted, failed, nothing to mount, ejected by the person, or found mounted
//! already) is not tried again until it is unplugged, so a failure is one notice and never a loop, and an ejected
//! volume stays ejected. When a mounted disk is pulled out without an eject, its mounts under the media folders are
//! taken away lazily and their empty folders removed.
//!
//! An ejected volume whose disk is still attached (its power could not be turned off) can be mounted again on
//! request: the `disks` method `mount_volume` `{target}` ([`mount_requested`]). The request is checked like an eject
//! and then like a volume of a pass: a plain device name, a disk the kernel lists, the same [`candidates`] and
//! [`plan_mounts`], the same options. It never reaches a disk a pass would not have mounted.
//!
//! A volume that does not mount is told to the person in plain words ([`plain_reason`]); the tool's own message goes
//! to the log.

use super::disk_image::{self, export_value, image_device_of, run};
use super::volume_eject::valid_device;
use crate::bare::account::Account;
use bifrost_core::disk_events::{Disk, DiskEvent};
use bifrost_core::json::Json;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::Duration;

/// The `disks` service events: a removable volume was mounted `{ok, device, mount_point, name}`, and the mount of a
/// volume whose disk was pulled out was taken away `{ok, device, mount_point}`.
pub const E_MOUNTED: &str = "volume_mounted";
pub const E_UNMOUNTED: &str = "volume_unmounted";
/// The `disks` service method that mounts one volume again on request ([`mount_requested`]); its outcome is
/// `volume_mounted` `{target, ok, device, mount_point, name}` or `{target, ok, error}` to the app that asked.
pub const M_MOUNT: &str = "mount_volume";

/// Where a root shell mounts volumes, one folder per account.
const MEDIA_ROOT: &str = "/run/media";
/// The folders removable volumes are mounted under.
const MEDIA_ROOTS: [&str; 2] = ["/run/media", "/media"];
const SYS_BLOCK: &str = "/sys/block";
/// Where the kernel lists every block device, disk or partition, by name.
const SYS_CLASS_BLOCK: &str = "/sys/class/block";
/// How long after the last arrival a pass waits, in shell milliseconds: a disk's partitions appear after the disk.
/// The kernel reads the partition table as it adds the disk, so the nodes follow each other within milliseconds.
pub const SETTLE_MS: f64 = 400.0;
/// How often the loop looks for the end of a running pass, in shell milliseconds: often while it is young (a pass
/// over a healthy stick is done well inside a second), seldom once it has run longer than [`PASS_QUICK_MS`] (a
/// medium that does not answer can hold the probe for half a minute).
pub const PASS_POLL_MS: f64 = 100.0;
pub const PASS_SLOW_POLL_MS: f64 = 1_000.0;
pub const PASS_QUICK_MS: f64 = 2_000.0;
/// One mount at a time: a pass and a request never mount the same volume twice.
static MOUNTING: Mutex<()> = Mutex::new(());
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

/// Checks a request to mount `device` (`/dev/sdb1`) against the kernel's `disks`, the mount table and the swap list,
/// and returns what a pass would read of its disk. Refused: a path that is not a plain device name, a disk image's
/// device, a device the kernel does not list, one that is mounted already, a disk that is not removable
/// ([`eligible`]), a disk with any part in use by the system, and a device [`candidates`] leaves out of that disk
/// (a partition without a size, the disk itself while one of its partitions is mounted).
pub fn request(device: &str, disks: &[BlockDisk], mounts: &str, swaps: &str) -> Result<Wanted, String> {
    let name = device.strip_prefix("/dev/").filter(|_| valid_device(device)).ok_or_else(|| format!("{device} is not a volume's device"))?;
    if image_device_of(device).is_some() {
        return Err(format!("{device} is a disk image's volume"));
    }
    let disk = disks.iter().find(|d| d.name == name || d.parts.iter().any(|p| p.name == name)).ok_or_else(|| format!("{device} is not connected"))?;
    if !points_of(&bifrost_core::disk_scan::parse_mounts(mounts), name).is_empty() {
        return Err(format!("{device} is mounted already"));
    }
    if !eligible(disk) {
        return Err(format!("{device} is not a removable volume"));
    }
    let wanted = candidates(std::slice::from_ref(disk), mounts, swaps, &BTreeSet::new()).pop().ok_or_else(|| format!("{device} is in use by the system"))?;
    let listed = if name == disk.name { wanted.whole } else { wanted.parts.iter().any(|p| p.name == name) };
    if !listed {
        return Err(format!("{device} is not a volume that can be mounted"));
    }
    Ok(wanted)
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
    /// a volume that should have mounted did not: the reason in plain words ([`plain_reason`])
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

/// The name a person knows the file system `fs` by ([`supported`]'s names).
pub fn format_name(fs: &str) -> &'static str {
    match fs {
        "vfat" => "FAT",
        "exfat" => "exFAT",
        "ntfs3" => "NTFS",
        "ext2" => "ext2",
        "ext3" => "ext3",
        "ext4" => "ext4",
        "btrfs" => "Btrfs",
        "xfs" => "XFS",
        "f2fs" => "F2FS",
        "iso9660" => "ISO 9660",
        "udf" => "UDF",
        _ => "unknown",
    }
}

/// What the person is told when the mount of a volume of type `fs` failed with the tool's message `error`. The
/// message itself is for the log. A type the kernel has no driver for (its module is not on this computer) is named;
/// every other failure is one general sentence.
pub fn plain_reason(fs: &str, error: &str) -> String {
    if error.contains("unknown filesystem type") {
        format!("This disk uses a format this computer cannot read: {}", format_name(fs))
    } else {
        "It may be damaged. The details are in the log.".to_string()
    }
}

/// The one notice for the volumes of a pass that did not mount, `None` when all did. Each failure carries its
/// reason in plain words ([`plain_reason`]); several volumes share one sentence when they failed alike.
pub fn failure_notice(report: &Report) -> Option<String> {
    let failed: Vec<(&Outcome, &str)> = report
        .volumes
        .iter()
        .filter_map(|v| match &v.done {
            Done::Failed(reason) => Some((v, reason.trim())),
            _ => None,
        })
        .collect();
    let named = |v: &Outcome| format!("\u{201c}{}\u{201d} ({})", v.name, v.device);
    let (first, rest) = failed.split_first()?;
    let alike = Some(first.1).filter(|r| !r.is_empty() && rest.iter().all(|(_, other)| other == r));
    let head = match rest {
        [] => format!("The volume {} could not be opened.", named(first.0)),
        _ => format!("{} volumes could not be opened: {}.", failed.len(), failed.iter().map(|(v, _)| named(v)).collect::<Vec<_>>().join(", ")),
    };
    Some(match alike {
        Some(reason) => format!("{head} {reason}"),
        None => head,
    })
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
    /// The names that left since the last sweep. A mount of one of them is stale even when the name is there again:
    /// the disk was pulled and plugged in between two scans, and the mount belongs to the device that left.
    gone: BTreeSet<String>,
    /// The names the running pass is sweeping: their mounts are on the way out.
    sweeping: BTreeSet<String>,
    /// The names that left while the running pass was at work: what it decided about them is not kept.
    left: BTreeSet<String>,
    running: Option<Receiver<Report>>,
    /// When the running pass started, in shell milliseconds.
    started: f64,
}

impl Auto {
    /// Takes in one device scan: the `events` since the last one and the devices there now. The first scan asks for
    /// a pass over everything already plugged in. An arrival, or a medium put into a reader, asks for a mount; a
    /// device that went away is forgotten (it may be decided on again when it comes back) and asks for a sweep.
    /// Every device seen mounted counts as decided on, so one unmounted by hand is not mounted again; but not one
    /// whose name left and whose mount is still to be swept: that mount is the old device's, not this one's.
    pub fn observe(&mut self, events: &[DiskEvent], disks: &[Disk]) {
        if !std::mem::replace(&mut self.primed, true) {
            self.changed = true;
            self.sweep = true;
        } else {
            for event in events {
                match event {
                    DiskEvent::Appeared(d) => self.plugged(&d.bsd_name),
                    DiskEvent::Disappeared(name) => self.unplugged(name),
                    DiskEvent::DescriptionChanged { before, after } if before.size != after.size => {
                        self.handled.remove(&after.bsd_name);
                        self.changed = true;
                    }
                    DiskEvent::DescriptionChanged { .. } => {}
                }
            }
        }
        let stale = |name: &String| self.gone.contains(name) || self.sweeping.contains(name);
        let mounted: Vec<String> = disks.iter().filter(|d| d.mount_point.is_some() && !stale(&d.bsd_name)).map(|d| d.bsd_name.clone()).collect();
        self.handled.extend(mounted);
    }

    /// The device `name` arrived: a mount is asked for. A disk that had nothing to mount and now got a partition is
    /// looked at again.
    pub fn plugged(&mut self, name: &str) {
        self.handled.retain(|h| h == name || !name.starts_with(h.as_str()));
        self.changed = true;
    }

    /// The device `name` left: it is forgotten, so it is decided on again when it comes back, and a sweep is asked
    /// for. The sweep takes its mounts away even if the name is back by then ([`stale_mounts`]), and a pass at work
    /// right now does not get to mark it as decided on.
    pub fn unplugged(&mut self, name: &str) {
        self.handled.remove(name);
        self.gone.insert(name.to_string());
        if self.running.is_some() {
            self.left.insert(name.to_string());
        }
        self.sweep = true;
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

    /// Takes in a finished pass: every device it decided on is left alone from now on, except one that left while
    /// the pass ran (the decision was about a device that is gone).
    pub fn absorb(&mut self, report: &Report) {
        let left = std::mem::take(&mut self.left);
        self.handled.extend(report.volumes.iter().filter(|v| !left.contains(&v.device)).map(|v| v.device.clone()));
    }

    /// Whether a mount is still to come for what arrived: asked for, settling, or a pass at work.
    pub fn mount_waiting(&self) -> bool {
        self.changed || self.due.is_some() || self.running.is_some()
    }

    /// When the service needs its next tick, in shell milliseconds; `None` when nothing is waiting, which is the
    /// state of an idle desktop. A change not yet taken in and a sweep want the next tick; a mount wants the end of
    /// its settling time, and nothing while the screen is locked (the unlock is input and brings its own tick); a
    /// running pass is looked at every [`PASS_POLL_MS`], then every [`PASS_SLOW_POLL_MS`].
    pub fn next_wake(&self, t: f64, locked: bool) -> Option<f64> {
        if self.running.is_some() {
            return Some(t + if t - self.started > PASS_QUICK_MS { PASS_SLOW_POLL_MS } else { PASS_POLL_MS });
        }
        if self.changed || self.sweep {
            return Some(t);
        }
        self.due.filter(|_| !locked).map(|due| due.max(t))
    }

    /// A test's stand-in for a pass at work: its report comes through `rx`.
    #[cfg(test)]
    pub fn set_running(&mut self, rx: Receiver<Report>) {
        self.running = Some(rx);
    }

    /// Collects the pass that ended, if one did. Runs on the service's tick, before the device scan.
    pub fn finish(&mut self) -> Option<Report> {
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
            self.sweeping.clear();
            self.absorb(report);
        }
        ended
    }

    /// Starts the next pass on a thread when one is due ([`Auto::next_pass`]). Runs on the service's tick, after
    /// the device scan.
    pub fn start(&mut self, t: f64, locked: bool) {
        let Some(pass) = self.next_pass(t, locked, self.running.is_some()) else { return };
        let gone = if pass.sweep { std::mem::take(&mut self.gone) } else { BTreeSet::new() };
        // a unit test never reads or mounts the disks of the machine it runs on
        if !cfg!(test) {
            let handled = self.handled.clone();
            self.sweeping = gone.clone();
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(run_pass(pass, &handled, &gone));
            });
            self.running = Some(rx);
            self.started = t;
        }
    }
}

/// Reads what the device `name` holds with `blkid -p`.
fn probe(name: &str, partition: Option<u32>, read_only: bool) -> Held {
    let device = format!("/dev/{name}");
    parse_held(name, partition, read_only, &run("blkid", &["-p", "-o", "export", "--", device.as_str()]).unwrap_or_default())
}

/// Reads the devices of `disk` and plans its volumes. Also returns the names that were read, for the ones that hold
/// nothing to mount: the disk itself only when it is the volume or has no partitions.
fn read_plan(disk: &Wanted) -> (Vec<Planned>, Vec<String>) {
    let whole = disk.whole.then(|| probe(&disk.disk, None, disk.read_only)).filter(|h| supported(&h.fs).is_some());
    let parts: Vec<Held> = if whole.is_some() { Vec::new() } else { disk.parts.iter().map(|p| probe(&p.name, Some(p.number), disk.read_only || p.read_only)).collect() };
    let read = if whole.is_some() || disk.parts.is_empty() { vec![disk.disk.clone()] } else { disk.parts.iter().map(|p| p.name.clone()).collect() };
    (plan_mounts(whole.as_ref(), &parts), read)
}

/// Mounts the volume on `device` (`/dev/sdb1`) for the `disks` method `mount_volume`, on the request's thread: the
/// request is checked against the kernel's list and the mount table now ([`request`]), the disk is read and planned
/// as a pass does, and only a volume that plan holds is mounted, with a pass's options. Returns the result's fields
/// (`device`, `mount_point`, `name`); a refusal and the tool's message are logged, the error that goes back is for
/// the person.
pub fn mount_requested(device: &str) -> Result<Vec<(&'static str, String)>, String> {
    let _one = MOUNTING.lock().unwrap_or_else(|e| e.into_inner());
    let refused = |e: String| {
        eprintln!("disks: mount {device}: refused: {e}");
        e
    };
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    let wanted = request(device, &read_disks(Path::new(SYS_BLOCK)), &mounts, &swaps).map_err(refused)?;
    let name = device.strip_prefix("/dev/").unwrap_or_default();
    let (plan, _) = read_plan(&wanted);
    let volume = plan.iter().find(|v| v.name == name).ok_or_else(|| refused(format!("{device} holds no volume this computer opens")))?;
    let account = Account::installed().map_err(|e| refused(format!("no account to mount for: {e}")))?;
    match mount_one(volume, &account, crate::privd::server::euid() == 0) {
        Ok(folder) => Ok(vec![("device", device.to_string()), ("mount_point", folder), ("name", volume.folder.clone())]),
        Err(e) => {
            eprintln!("disks: mount {device} ({}) failed: {e}", volume.fs);
            Err(plain_reason(volume.fs, &e))
        }
    }
}

/// One pass, on its own thread: the sweep first, then the mounts. `gone` are the names that left since the last
/// sweep: a mount of one of them is taken away even when a device of that name is there again.
fn run_pass(pass: Pass, handled: &BTreeSet<String>, gone: &BTreeSet<String>) -> Report {
    let _one = MOUNTING.lock().unwrap_or_else(|e| e.into_inner());
    let as_root = crate::privd::server::euid() == 0;
    let mut report = Report::default();
    let mut mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    if pass.sweep && as_root {
        // a shell running as the account leaves this to the storage service, which made those mounts
        for (device, point) in stale_mounts(&mounts, |name| !gone.contains(name) && Path::new(SYS_CLASS_BLOCK).join(name).exists()) {
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
    if !report.removed.is_empty() {
        // the table without what was just taken away, so a name that came back is not taken for mounted
        mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
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
    for disk in &wanted {
        let (plan, read) = read_plan(disk);
        // what was read and holds nothing to mount is decided on too
        for name in read.into_iter().filter(|n| !plan.iter().any(|v| v.name == *n)) {
            report.volumes.push(Outcome { device: name, name: String::new(), done: Done::Passed });
        }
        for volume in &plan {
            let done = match mount_one(volume, &account, as_root) {
                Ok(folder) => Done::Mounted(folder),
                Err(e) => {
                    // the tool's message is for the log, the person gets the reason in plain words
                    eprintln!("disks: automount: /dev/{} ({}) not mounted: {e}", volume.name, volume.fs);
                    Done::Failed(plain_reason(volume.fs, &e))
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
        let no_driver = plain_reason("exfat", "mount: /run/media/me/Stick: unknown filesystem type 'exfat'.\n       dmesg(1) may have more information after failed mount system call.");
        report.volumes.push(outcome("sdc1", "Stick", Done::Failed(no_driver.clone())));
        assert_eq!(failure_notice(&report).as_deref(), Some("The volume \u{201c}Stick\u{201d} (sdc1) could not be opened. This disk uses a format this computer cannot read: exFAT"));
        assert_eq!(events(&report).len(), 2, "a failure is a notice, not an event");
        // two volumes that failed alike are one notice with the one reason
        report.volumes.push(outcome("sdc2", "Untitled", Done::Failed(no_driver)));
        assert_eq!(
            failure_notice(&report).as_deref(),
            Some("2 volumes could not be opened: \u{201c}Stick\u{201d} (sdc1), \u{201c}Untitled\u{201d} (sdc2). This disk uses a format this computer cannot read: exFAT")
        );
        // two that failed differently are still one notice, without a reason that fits only one
        report.volumes.pop();
        report.volumes.push(outcome("sdc2", "Untitled", Done::Failed(plain_reason("ext4", "mount: wrong fs type, bad option, bad superblock on /dev/sdc2"))));
        assert_eq!(failure_notice(&report).as_deref(), Some("2 volumes could not be opened: \u{201c}Stick\u{201d} (sdc1), \u{201c}Untitled\u{201d} (sdc2)."));
        report.volumes.truncate(2);
        report.volumes.push(outcome("sdc2", "Untitled", Done::Failed("  ".into())));
        assert_eq!(failure_notice(&report).as_deref(), Some("The volume \u{201c}Untitled\u{201d} (sdc2) could not be opened."));
    }

    #[test]
    fn a_failed_mount_is_told_in_plain_words_and_never_as_the_tools_message() {
        let raw = "mount: /run/media/me/Stick: unknown filesystem type 'exfat'.";
        assert_eq!(plain_reason("exfat", raw), "This disk uses a format this computer cannot read: exFAT");
        assert_eq!(plain_reason("ntfs3", "mount: /run/media/me/Data: unknown filesystem type 'ntfs3'."), "This disk uses a format this computer cannot read: NTFS");
        // the storage service words it the same way
        assert_eq!(plain_reason("exfat", "Error mounting /dev/sdb1 at /run/media/me/X: unknown filesystem type 'exfat'"), "This disk uses a format this computer cannot read: exFAT");
        for other in ["mount: /run/media/me/Stick: wrong fs type, bad option, bad superblock on /dev/sdb1, missing codepage or helper program, or other error.", "mount: /dev/sdb1: can't read superblock.", "", "udisksctl could not run: No such file or directory"] {
            let told = plain_reason("vfat", other);
            assert_eq!(told, "It may be damaged. The details are in the log.");
            assert!(!told.contains("mount:") && !told.contains("/dev/"), "{told}");
        }
        for fs in ["vfat", "exfat", "ntfs", "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "iso9660", "udf"] {
            assert_ne!(format_name(supported(fs).unwrap()), "unknown", "{fs}");
        }
        assert_eq!(format_name("swap"), "unknown");
    }

    #[test]
    fn a_tick_in_a_test_starts_no_pass_on_the_machine() {
        let mut auto = Auto::default();
        auto.observe(&[], &[]);
        assert_eq!(auto.finish(), None);
        auto.start(0.0, false);
        auto.start(10_000.0, false);
        assert!(auto.running.is_none());
        // a pass whose thread died is over, and the next one may start
        let (tx, rx) = channel::<Report>();
        drop(tx);
        auto.running = Some(rx);
        assert_eq!(auto.finish(), Some(Report::default()));
        assert!(auto.running.is_none());
    }

    fn mounted(device: &str, folder: &str) -> Report {
        Report { volumes: vec![Outcome { device: device.into(), name: folder.into(), done: Done::Mounted(format!("/run/media/me/{folder}")) }], removed: Vec::new() }
    }

    #[test]
    fn nothing_is_mounted_while_the_screen_is_locked() {
        let mut auto = Auto::default();
        // started at the login window with nothing plugged in: the first sweep, then nothing waits for a tick
        auto.observe(&[], &[]);
        auto.start(0.0, true);
        assert!(auto.due.is_some() && !auto.sweep);
        assert_eq!(auto.next_wake(1.0, true), None, "a locked screen is not woken for a mount that waits");
        // a stick is attached while locked: no pass, however long it stays locked, and no wake-up either
        auto.observe(&[DiskEvent::Appeared(disk("sdb", GB, None)), DiskEvent::Appeared(disk("sdb1", GB, None))], &[]);
        assert_eq!(auto.next_wake(1_000.0, true), Some(1_000.0), "the arrival is taken in on the next tick");
        for t in [1_000.0, 1_000.0 + SETTLE_MS, 600_000.0] {
            assert_eq!(auto.next_pass(t, true, false), None, "locked at {t}");
            assert_eq!(auto.next_wake(t, true), None);
        }
        // unlocked: the pass starts on that very tick, once
        assert_eq!(auto.next_wake(600_001.0, false), Some(600_001.0));
        assert_eq!(auto.next_pass(600_001.0, false, false), Some(Pass { mount: true, sweep: false }));
        assert_eq!(auto.next_pass(600_002.0, false, false), None);
        assert_eq!(auto.next_wake(600_002.0, false), None, "nothing waits: the idle desktop gets no wake-up");
    }

    #[test]
    fn a_pass_running_at_the_lock_finishes_and_no_new_one_starts() {
        let mut auto = Auto::default();
        auto.observe(&[], &[]);
        auto.start(0.0, false);
        auto.start(SETTLE_MS, false);
        // a pass over sdb is at work when the screen locks, and a second stick arrives behind the lock
        let (tx, rx) = channel::<Report>();
        auto.running = Some(rx);
        auto.started = 1_000.0;
        auto.observe(&[DiskEvent::Appeared(disk("sdc", GB, None))], &[]);
        assert_eq!(auto.finish(), None, "still at work");
        auto.start(1_100.0, true);
        assert_eq!(auto.next_wake(1_100.0, true), Some(1_100.0 + PASS_POLL_MS), "the loop looks for its end");
        assert_eq!(auto.next_wake(1_000.0 + PASS_QUICK_MS + 1.0, true), Some(1_000.0 + PASS_QUICK_MS + 1.0 + PASS_SLOW_POLL_MS), "seldom when it takes long");
        // it finishes what it started
        tx.send(mounted("sdb1", "Stick")).unwrap();
        assert_eq!(auto.finish(), Some(mounted("sdb1", "Stick")));
        assert!(auto.is_handled("sdb1") && auto.running.is_none());
        // and nothing new starts behind the lock: the second stick waits
        for t in [1_200.0, 1_200.0 + SETTLE_MS, 900_000.0] {
            auto.start(t, true);
            assert!(auto.running.is_none() && auto.due.is_some(), "locked at {t}");
            assert_eq!(auto.next_pass(t, true, false), None);
            assert_eq!(auto.next_wake(t, true), None);
        }
        assert!(!auto.is_handled("sdc"));
        assert_eq!(auto.next_pass(900_001.0, false, false), Some(Pass { mount: true, sweep: false }), "the first tick after the unlock");
    }

    #[test]
    fn a_disk_pulled_and_plugged_in_again_between_two_scans_is_a_new_device() {
        let mut auto = Auto::default();
        let there = [disk("sda", GB, None), disk("sda1", GB, Some("/run/media/me/E2EUSB"))];
        auto.observe(&[], &there);
        auto.start(0.0, false);
        auto.start(SETTLE_MS, false);
        assert!(auto.is_handled("sda1") && auto.gone.is_empty());
        // the scan sees the same names as before and reports nothing; the watch saw both the pull and the plug
        for name in ["sda1", "sda"] {
            auto.unplugged(name);
        }
        for name in ["sda", "sda1"] {
            auto.plugged(name);
        }
        // the old device's mount is still in the table at the next scan: it does not make the new one decided on
        auto.observe(&[], &there);
        assert!(!auto.is_handled("sda1") && !auto.is_handled("sda"), "decided on again");
        auto.sweeping = set(&["sdx1"]);
        auto.observe(&[], &[disk("sdx1", GB, Some("/run/media/me/Old")), disk("sdy1", GB, Some("/run/media/me/New"))]);
        assert!(!auto.is_handled("sdx1") && auto.is_handled("sdy1"), "nor does a mount the running pass is taking away");
        auto.sweeping.clear();
        auto.handled.remove("sdy1");
        assert_eq!(auto.gone, set(&["sda", "sda1"]));
        assert_eq!(auto.next_wake(5_000.0, false), Some(5_000.0));
        // the sweep goes first and takes the old device's mount away although the name is there again
        let table = "/dev/sda1 /run/media/me/E2EUSB vfat rw 0 0\n/dev/sdb1 /run/media/me/Other vfat rw 0 0\n";
        let present = |name: &str| !auto.gone.contains(name);
        assert_eq!(stale_mounts(table, present), vec![("/dev/sda1".to_string(), "/run/media/me/E2EUSB".to_string())]);
        auto.start(5_000.0, false);
        assert!(auto.gone.is_empty() && !auto.sweep, "the sweep took the names with it");
        // then the mount, a moment after the plug
        assert_eq!(auto.next_wake(5_001.0, false), Some(5_000.0 + SETTLE_MS));
        assert_eq!(auto.next_pass(5_000.0 + SETTLE_MS, false, false), Some(Pass { mount: true, sweep: false }));
        assert_eq!(candidates(&[stick("sda", &["sda1"])], "", "", &auto.handled).len(), 1);
    }

    #[test]
    fn what_a_pass_decided_about_a_device_that_left_meanwhile_is_not_kept() {
        let mut auto = Auto::default();
        auto.observe(&[], &[]);
        let (tx, rx) = channel::<Report>();
        auto.running = Some(rx);
        // the stick is pulled while the pass that mounts it is at work
        auto.unplugged("sdb1");
        auto.unplugged("sdb");
        let mut report = mounted("sdb1", "Stick");
        report.volumes.push(Outcome { device: "sdc1".into(), name: "Card".into(), done: Done::Failed(plain_reason("vfat", "x")) });
        tx.send(report).unwrap();
        assert!(auto.finish().is_some());
        assert!(!auto.is_handled("sdb1"), "plugged in again, it is looked at again");
        assert!(auto.is_handled("sdc1"));
        assert!(auto.left.is_empty());
        // a device that leaves with no pass at work is not remembered for the next one
        auto.unplugged("sdd1");
        assert!(auto.left.is_empty());
        auto.absorb(&mounted("sdd1", "Later"));
        assert!(auto.is_handled("sdd1"));
    }

    #[test]
    fn an_ejected_volume_that_stays_attached_can_be_mounted_again_on_request() {
        let disks = [BlockDisk { removable: false, hotplug: false, ..stick("nvme0n1", &["nvme0n1p1", "nvme0n1p2"]) }, stick("sdb", &["sdb1", "sdb2"]), stick("sdc", &[]), stick("loop0", &["loop0p1"])];
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/nvme0n1p1 /boot vfat rw 0 0\n";
        // the automatic pass leaves an ejected volume alone; the request reads the same disk the pass would
        let wanted = request("/dev/sdb1", &disks, table, "").unwrap();
        assert_eq!(names(std::slice::from_ref(&wanted)), vec![("sdb".to_string(), true, vec!["sdb1".to_string(), "sdb2".to_string()])]);
        assert_eq!(wanted, candidates(&disks, table, "", &BTreeSet::new())[0], "the same filter as the automatic pass");
        // a stick that is one file system without a partition table
        assert!(request("/dev/sdc", &disks, table, "").is_ok_and(|w| w.whole && w.parts.is_empty()));
        // with its other partition mounted the partition is still wanted, the disk itself is not a volume
        let half = format!("{table}/dev/sdb2 /run/media/me/Two ext4 rw 0 0\n");
        assert!(request("/dev/sdb1", &disks, &half, "").is_ok_and(|w| !w.whole));
        assert_eq!(request("/dev/sdb", &disks, &half, "").unwrap_err(), "/dev/sdb is not a volume that can be mounted");
    }

    #[test]
    fn a_request_to_mount_is_refused_for_everything_a_pass_would_not_mount() {
        let system = BlockDisk { removable: false, hotplug: false, ..stick("nvme0n1", &["nvme0n1p1", "nvme0n1p2", "nvme0n1p3"]) };
        let disks = [system, stick("sdb", &["sdb1", "sdb2"]), stick("loop0", &["loop0p1"]), stick("sda", &["sda1", "sda2", "sda3"])];
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/sda2 / ext4 rw 0 0\n/dev/sdb2 /run/media/me/Two ext4 rw 0 0\n";
        let refused = |device: &str| request(device, &disks, table, "").unwrap_err();
        // never a path, an option or a name that is not plain
        for bad in ["", "sdb1", "/dev/", "/dev/../etc/shadow", "/dev/sdb1 -o suid", "/dev/mapper/x", "/run/media/me/Two", "-o", "/dev/sd*", "/dev/disk/by-label/E2EUSB"] {
            assert_eq!(refused(bad), format!("{bad} is not a volume's device"));
        }
        assert_eq!(refused("/dev/loop0p1"), "/dev/loop0p1 is a disk image's volume");
        assert_eq!(refused("/dev/sdz1"), "/dev/sdz1 is not connected");
        assert_eq!(refused("/dev/sdb2"), "/dev/sdb2 is mounted already");
        // never the system disk: not its unmounted partition, not the disk
        assert_eq!(refused("/dev/nvme0n1p3"), "/dev/nvme0n1p3 is not a removable volume");
        assert_eq!(refused("/dev/nvme0n1p1"), "/dev/nvme0n1p1 is not a removable volume");
        assert_eq!(refused("/dev/nvme0n1"), "/dev/nvme0n1 is not a removable volume");
        // nor a removable disk the system runs from, whichever part of it is asked for
        assert_eq!(refused("/dev/sda1"), "/dev/sda1 is in use by the system");
        assert_eq!(refused("/dev/sda3"), "/dev/sda3 is in use by the system");
        assert_eq!(refused("/dev/sda"), "/dev/sda is in use by the system");
        assert_eq!(request("/dev/sda3", &[stick("sda", &["sda1", "sda3"])], "", "/dev/sda1 partition 1 0 -2\n").unwrap_err(), "/dev/sda3 is in use by the system", "a disk with swap in use");
        let mut built = stick("sdd", &["sdd1", "sdd2"]);
        built.parts[1].held = true;
        assert_eq!(request("/dev/sdd1", &[built], "", "").unwrap_err(), "/dev/sdd1 is in use by the system", "another device is built on the disk");
        // a reader without a card, and an extended partition's placeholder
        assert_eq!(request("/dev/sde", &[BlockDisk { size: 0, ..stick("sde", &[]) }], "", "").unwrap_err(), "/dev/sde is not a removable volume");
        let mut old = stick("sdf", &["sdf1", "sdf5"]);
        old.parts[0].size = 0;
        assert_eq!(request("/dev/sdf1", &[old], "", "").unwrap_err(), "/dev/sdf1 is not a volume that can be mounted");
    }
}
