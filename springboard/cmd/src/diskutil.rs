//! `diskutil`: lists disks and their volumes, shows one device's details, mounts, unmounts or ejects it, and runs the
//! privileged requests: `eraseDisk`, `eraseVolume`, `partitionDisk`, `verifyVolume` and `repairVolume`. Reads are from
//! the kernel's device tree (`bifrost_core::disk_scan`); mount, unmount and eject go through the disk session in
//! `bifrost-core` (the same claims and approval gates the desktop's Disk Utility uses). The privileged requests are the
//! ones the desktop sends too (`bifrost_core::disk_request`), and the helper plans and runs them as root, so they need
//! root here.
//! Gap: the list has no file system or label column yet (the desktop app reads them from the udev database, which this
//! tool does not).

use bifrost_core::disk_claims::Claims;
use bifrost_core::disk_commands::{self, Command, MountOptions, Refusal};
use bifrost_core::disk_events::Approvals;
use bifrost_core::disk_format::{PartitionSpec, Personality};
use bifrost_core::disk_request::DiskRequest;
use bifrost_core::disk_scan::Entry;
use bifrost_core::disk_session::Session;

pub const USAGE: &str = "usage: diskutil [list | info <device> | mount <device> | unmount <device> | eject <device>\n       | eraseDisk <device> <fs> [label] | eraseVolume <device> <fs> [label]\n       | partitionDisk <device> <fs[:size_mib[:label]]>... | verifyVolume <device> | repairVolume <device>\n       | renameVolume <device> <label> | restoreVolume <device> <source-device>]\n       fs is ext4, exfat, fat32, ntfs or btrfs; size 0 or none takes the rest of the disk. The erase, partition and check verbs need root.";
/// The owner name this tool claims devices under.
pub const OWNER: &str = "is.olibuijr.diskutil.cli";

/// The actions a device can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Mount,
    Unmount,
    Eject,
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    List,
    Info(String),
    Act(Action, String),
    /// A privileged request, run by the helper.
    Request(DiskRequest),
}

const REQUEST_VERBS: [&str; 7] = ["eraseDisk", "eraseVolume", "partitionDisk", "verifyVolume", "repairVolume", "renameVolume", "restoreVolume"];

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let device = |name: &str| -> Result<String, String> {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
            Err(format!("\u{201c}{name}\u{201d} is not a device name"))
        } else {
            Ok(name.to_string())
        }
    };
    match words.as_slice() {
        [] | ["list"] => Ok(Cmd::List),
        ["-h" | "-help" | "--help"] => Ok(Cmd::Help),
        ["info", name] => Ok(Cmd::Info(device(name)?)),
        ["mount", name] => Ok(Cmd::Act(Action::Mount, device(name)?)),
        ["unmount", name] => Ok(Cmd::Act(Action::Unmount, device(name)?)),
        ["eject", name] => Ok(Cmd::Act(Action::Eject, device(name)?)),
        ["eraseDisk", name, fs, rest @ ..] if rest.len() <= 1 => {
            Ok(Cmd::Request(DiskRequest::EraseDisk { device: device(name)?, fs: fs_arg(fs)?, label: rest.first().unwrap_or(&"").to_string() }))
        }
        ["eraseVolume", name, fs, rest @ ..] if rest.len() <= 1 => {
            Ok(Cmd::Request(DiskRequest::EraseVolume { device: device(name)?, fs: fs_arg(fs)?, label: rest.first().unwrap_or(&"").to_string() }))
        }
        ["partitionDisk", name, specs @ ..] if !specs.is_empty() => {
            let partitions = specs.iter().map(|s| partition_arg(s)).collect::<Result<Vec<PartitionSpec>, String>>()?;
            Ok(Cmd::Request(DiskRequest::PartitionDisk { device: device(name)?, partitions }))
        }
        ["verifyVolume", name] => Ok(Cmd::Request(DiskRequest::VerifyVolume { device: device(name)? })),
        ["repairVolume", name] => Ok(Cmd::Request(DiskRequest::RepairVolume { device: device(name)? })),
        ["renameVolume", name, label] => Ok(Cmd::Request(DiskRequest::RenameVolume { device: device(name)?, label: label.to_string() })),
        ["restoreVolume", name, source] => Ok(Cmd::Request(DiskRequest::RestoreVolume { device: device(name)?, source: device(source)? })),
        [verb, ..] if ["info", "mount", "unmount", "eject"].contains(verb) => Err(format!("{verb} needs one device name\n{USAGE}")),
        [verb, ..] if REQUEST_VERBS.contains(verb) => Err(format!("{verb}: wrong arguments\n{USAGE}")),
        [other, ..] => Err(format!("unknown verb {other}\n{USAGE}")),
    }
}

fn fs_arg(word: &str) -> Result<Personality, String> {
    Personality::parse(word).ok_or_else(|| format!("\u{201c}{word}\u{201d} is not a file system: use ext4, exfat, fat32, ntfs or btrfs"))
}

/// One partition of `partitionDisk`: `fs`, `fs:size_mib` or `fs:size_mib:label`.
pub fn partition_arg(word: &str) -> Result<PartitionSpec, String> {
    let mut parts = word.splitn(3, ':');
    let fs = fs_arg(parts.next().unwrap_or(""))?;
    let size_mib = match parts.next() {
        None | Some("") => 0,
        Some(n) => n.parse::<u64>().map_err(|_| format!("\u{201c}{n}\u{201d} is not a size in MiB"))?,
    };
    Ok(PartitionSpec { size_mib, fs, label: parts.next().unwrap_or("").to_string() })
}

/// A byte count in decimal units, as the desktop shows sizes: `512 B`, `1.5 GB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The table `diskutil list` prints: each disk, then its partitions indented under it.
pub fn list_text(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries.iter().filter(|e| e.parent.is_none()) {
        let kind = if e.disk.removable { "external" } else { "internal" };
        let model = if e.disk.model.is_empty() { String::new() } else { format!(" {}", e.disk.model) };
        out.push_str(&format!("/dev/{}  ({kind}, {}){model}\n", e.disk.bsd_name, format_size(e.disk.size)));
        for part in entries.iter().filter(|p| p.parent.as_deref() == Some(e.disk.bsd_name.as_str())) {
            let mounted = match &part.disk.mount_point {
                Some(point) => format!("mounted at {point}"),
                None => "not mounted".to_string(),
            };
            out.push_str(&format!("   /dev/{}  {}  {mounted}\n", part.disk.bsd_name, format_size(part.disk.size)));
        }
    }
    if out.is_empty() {
        out.push_str("No disks found.\n");
    }
    out
}

/// The details `diskutil info` prints for one device.
pub fn info_text(entries: &[Entry], name: &str) -> Result<String, String> {
    let e = entries.iter().find(|e| e.disk.bsd_name == name).ok_or_else(|| format!("{name} is not a connected device"))?;
    let yes = |b: bool| if b { "Yes" } else { "No" };
    let mut rows: Vec<(&str, String)> = vec![
        ("Device Identifier", e.disk.bsd_name.clone()),
        ("Device Node", format!("/dev/{}", e.disk.bsd_name)),
        ("Total Size", format!("{} ({} Bytes)", format_size(e.disk.size), e.disk.size)),
        ("Removable Media", yes(e.disk.removable).into()),
        ("Read-Only Media", yes(e.disk.read_only).into()),
        ("Mount Point", e.disk.mount_point.clone().unwrap_or_else(|| "Not mounted".into())),
    ];
    if let Some(parent) = &e.parent {
        rows.push(("Partition of", parent.clone()));
    }
    if !e.disk.model.is_empty() {
        rows.push(("Device / Media Name", e.disk.model.clone()));
    }
    Ok(rows.iter().map(|(k, v)| format!("   {k}: {v}\n")).collect())
}

/// The partitions of `name` that are mounted now.
fn mounted_partitions(entries: &[Entry], name: &str) -> Vec<String> {
    entries.iter().filter(|p| p.parent.as_deref() == Some(name) && p.disk.mount_point.is_some()).map(|p| p.disk.bsd_name.clone()).collect()
}

/// Plans a mount, unmount or eject of `name`. An eject unmounts the mounted partitions first, then powers the disk
/// off. Returns the commands, or the reason the session refused.
pub fn plan(action: Action, entries: &[Entry], name: &str) -> Result<Vec<Command>, Refusal> {
    let mut session = Session::new();
    session.observe(entries.iter().map(|e| e.disk.clone()).collect());
    match action {
        Action::Mount => {
            let disk = session.disk(name).ok_or_else(|| Refusal::NoDisk(name.to_string()))?;
            disk_commands::mount(disk, &MountOptions::default(), &Approvals::new(), &Claims::default(), OWNER)
        }
        Action::Unmount => session.unmount(name, OWNER),
        Action::Eject => {
            let mut commands = Vec::new();
            for part in mounted_partitions(entries, name) {
                commands.extend(session.unmount(&part, OWNER)?);
            }
            commands.extend(session.eject(name, OWNER)?);
            Ok(commands)
        }
    }
}

/// Sends a privileged request to the helper and returns the tools that completed. Root only: the helper runs the
/// request in this process for root, and a user has no session token to send it with.
pub fn request(req: &DiskRequest) -> Result<Vec<String>, String> {
    if springboard::privd::server::euid() != 0 {
        return Err(format!("{} needs root. Run it with sudo.", request_verb(req)));
    }
    springboard::privd_client::connect().disk_request(req).map_err(|e| e.message)
}

/// The command word of a privileged request, as the user typed it.
pub fn request_verb(req: &DiskRequest) -> &'static str {
    match req {
        DiskRequest::EraseDisk { .. } => "eraseDisk",
        DiskRequest::EraseVolume { .. } => "eraseVolume",
        DiskRequest::PartitionDisk { .. } => "partitionDisk",
        DiskRequest::VerifyVolume { .. } => "verifyVolume",
        DiskRequest::RepairVolume { .. } => "repairVolume",
        DiskRequest::RenameVolume { .. } => "renameVolume",
        DiskRequest::RestoreVolume { .. } => "restoreVolume",
    }
}

/// The line printed when a privileged request finished.
pub fn request_done_text(req: &DiskRequest) -> String {
    let device = req.device();
    match req {
        DiskRequest::EraseDisk { .. } | DiskRequest::EraseVolume { .. } => format!("Erased {device}."),
        DiskRequest::PartitionDisk { .. } => format!("Partitioned {device}."),
        DiskRequest::VerifyVolume { .. } => format!("Checked {device}."),
        DiskRequest::RepairVolume { .. } => format!("Repaired {device}."),
        DiskRequest::RenameVolume { .. } => format!("Renamed {device}."),
        DiskRequest::RestoreVolume { .. } => format!("Restored {device}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::disk_events::Disk;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn disk(name: &str, size: u64, mount: Option<&str>) -> Disk {
        Disk { bsd_name: name.into(), size, removable: true, read_only: false, model: "Stick".into(), mount_point: mount.map(String::from) }
    }

    fn sample() -> Vec<Entry> {
        vec![
            Entry { disk: disk("sdb", 2_000_000_000, None), parent: None },
            Entry { disk: disk("sdb1", 1_000_000_000, Some("/mnt/usb")), parent: Some("sdb".into()) },
        ]
    }

    #[test]
    fn verbs_parse_with_one_device() {
        assert_eq!(parse(&args(&[])), Ok(Cmd::List));
        assert_eq!(parse(&args(&["info", "sdb"])), Ok(Cmd::Info("sdb".into())));
        assert_eq!(parse(&args(&["eject", "sdb"])), Ok(Cmd::Act(Action::Eject, "sdb".into())));
        assert!(parse(&args(&["mount"])).unwrap_err().contains("one device name"));
        assert!(parse(&args(&["erase", "sdb"])).unwrap_err().contains("unknown verb erase"));
        assert!(parse(&args(&["mount", "../etc"])).unwrap_err().contains("not a device name"));
    }

    #[test]
    fn erase_and_check_verbs_parse_into_privileged_requests() {
        assert_eq!(
            parse(&args(&["eraseDisk", "sdb", "fat32", "STICK"])),
            Ok(Cmd::Request(DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into() }))
        );
        assert_eq!(
            parse(&args(&["eraseVolume", "sdb1", "ext4"])),
            Ok(Cmd::Request(DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::Ext4, label: String::new() }))
        );
        assert_eq!(parse(&args(&["verifyVolume", "sdb1"])), Ok(Cmd::Request(DiskRequest::VerifyVolume { device: "sdb1".into() })));
        assert_eq!(parse(&args(&["repairVolume", "sdb1"])), Ok(Cmd::Request(DiskRequest::RepairVolume { device: "sdb1".into() })));
    }

    #[test]
    fn a_partition_list_parses_size_and_label_per_partition() {
        let got = parse(&args(&["partitionDisk", "sdb", "fat32:512:BOOT", "ext4::", "ext4"])).unwrap();
        let expected = vec![
            PartitionSpec { size_mib: 512, fs: Personality::Fat32, label: "BOOT".into() },
            PartitionSpec { size_mib: 0, fs: Personality::Ext4, label: String::new() },
            PartitionSpec { size_mib: 0, fs: Personality::Ext4, label: String::new() },
        ];
        assert_eq!(got, Cmd::Request(DiskRequest::PartitionDisk { device: "sdb".into(), partitions: expected }));
    }

    #[test]
    fn bad_file_systems_sizes_and_arity_are_refused_with_usage() {
        assert!(parse(&args(&["eraseDisk", "sdb", "zfs"])).unwrap_err().contains("not a file system"));
        assert!(parse(&args(&["partitionDisk", "sdb", "ext4:big"])).unwrap_err().contains("not a size"));
        assert!(parse(&args(&["eraseDisk", "sdb"])).unwrap_err().contains("wrong arguments"));
        assert!(parse(&args(&["verifyVolume"])).unwrap_err().contains("wrong arguments"));
    }

    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1_500_000_000), "1.5 GB");
        assert_eq!(format_size(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn the_list_nests_partitions_under_their_disk() {
        let text = list_text(&sample());
        assert!(text.contains("/dev/sdb  (external, 2.0 GB) Stick\n"));
        assert!(text.contains("   /dev/sdb1  1.0 GB  mounted at /mnt/usb\n"));
        assert_eq!(list_text(&[]), "No disks found.\n");
    }

    #[test]
    fn info_shows_mount_point_and_parent() {
        let text = info_text(&sample(), "sdb1").unwrap();
        assert!(text.contains("Mount Point: /mnt/usb"));
        assert!(text.contains("Partition of: sdb"));
        assert!(info_text(&sample(), "sdz").unwrap_err().contains("not a connected device"));
    }

    #[test]
    fn eject_unmounts_mounted_partitions_then_powers_off() {
        let commands = plan(Action::Eject, &sample(), "sdb").unwrap();
        assert!(commands.len() >= 2, "one unmount for the partition, then the power-off");
        assert!(commands[0].args.iter().any(|a| a.ends_with("sdb1")));
    }

    #[test]
    fn unmount_of_an_unmounted_volume_is_refused() {
        let err = plan(Action::Unmount, &sample(), "sdb").unwrap_err();
        assert!(err.sentence().contains("not mounted"));
    }

    #[test]
    fn a_user_is_told_the_request_needs_root_and_the_done_lines_name_the_device() {
        // `request` is not called here: as root it would run the real helper.
        let req = DiskRequest::VerifyVolume { device: "sdb1".into() };
        assert_eq!(request_verb(&req), "verifyVolume");
        assert_eq!(request_done_text(&req), "Checked sdb1.");
        assert_eq!(request_done_text(&DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Ext4, label: String::new() }), "Erased sdb.");
    }
}
