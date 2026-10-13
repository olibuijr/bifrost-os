//! `diskutil`: lists disks and their volumes, shows one device's details, and mounts, unmounts or ejects it. Reads are
//! from the kernel's device tree (`/sys/block`) and the mount table (`/proc/mounts`); changes go through the disk
//! session in `bifrost-core` (`disk_session`, `disk_commands`), the same claims and approval gates the desktop's Disk
//! Utility uses, and the commands it plans run here on the command line.
//! Gap: there is no erase, partition or rename, and the list has no file system or label column yet (the desktop app
//! reads them from the udev database, which this tool does not).

use bifrost_core::disk_claims::Claims;
use bifrost_core::disk_commands::{self, Command, MountOptions, Refusal};
use bifrost_core::disk_events::{Approvals, Disk};
use bifrost_core::disk_session::Session;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const USAGE: &str = "usage: diskutil [list | info <device> | mount <device> | unmount <device> | eject <device>]";
/// The owner name this tool claims devices under.
pub const OWNER: &str = "is.olibuijr.diskutil.cli";

/// A disk or a partition, and the disk a partition belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub disk: Disk,
    pub parent: Option<String>,
}

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
}

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
        [verb, ..] if ["info", "mount", "unmount", "eject"].contains(verb) => Err(format!("{verb} needs one device name\n{USAGE}")),
        [other, ..] => Err(format!("unknown verb {other}\n{USAGE}")),
    }
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

/// The mount table as (device path, mount point) pairs. Spaces and other special characters are octal escapes.
pub fn parse_mounts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let dev = fields.next()?;
            let point = fields.next()?;
            Some((dev.to_string(), unescape(point)))
        })
        .collect()
}

/// Decodes the octal escapes the mount table uses (`\040` is a space).
pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 4 <= bytes.len() {
            if let Some(v) = text.get(i + 1..i + 4).and_then(|o| u8::from_str_radix(o, 8).ok()) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Devices the list leaves out: loop devices, RAM disks and compressed RAM.
fn is_hidden(name: &str) -> bool {
    name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram")
}

fn entry(dir: &Path, name: &str, parent: Option<&str>, removable: bool, mounts: &HashMap<String, String>) -> Entry {
    let size = read_trim(&dir.join("size")).parse::<u64>().unwrap_or(0).saturating_mul(512);
    let read_only = read_trim(&dir.join("ro")) == "1";
    let model = if parent.is_some() { String::new() } else { read_trim(&dir.join("device/model")) };
    let mount_point = mounts.get(&format!("/dev/{name}")).cloned();
    let removable = if parent.is_some() { removable } else { read_trim(&dir.join("removable")) == "1" };
    Entry {
        disk: Disk { bsd_name: name.to_string(), size, removable, read_only, model, mount_point },
        parent: parent.map(String::from),
    }
}

/// Reads every disk under `sys_block` and its partitions after it, sorted by name. `mounts_text` is `/proc/mounts`.
pub fn scan(sys_block: &Path, mounts_text: &str) -> Vec<Entry> {
    let mounts: HashMap<String, String> = parse_mounts(mounts_text).into_iter().collect();
    let Ok(read) = fs::read_dir(sys_block) else { return Vec::new() };
    let mut names: Vec<String> = read.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut out = Vec::new();
    for name in names.iter().filter(|n| !is_hidden(n)) {
        let dir = sys_block.join(name);
        let removable_disk = read_trim(&dir.join("removable")) == "1";
        out.push(entry(&dir, name, None, removable_disk, &mounts));
        let Ok(children) = fs::read_dir(&dir) else { continue };
        let mut parts: Vec<String> = children
            .filter_map(Result::ok)
            .filter(|c| c.path().join("partition").exists())
            .map(|c| c.file_name().to_string_lossy().into_owned())
            .filter(|c| c.starts_with(name.as_str()))
            .collect();
        parts.sort();
        for part in parts {
            out.push(entry(&dir.join(&part), &part, Some(name.as_str()), removable_disk, &mounts));
        }
    }
    out
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

/// Plans an action on `name`. An eject unmounts the mounted partitions first, then powers the disk off. Returns the
/// commands, or the reason the session refused.
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

/// Runs the commands in order. The first failure stops the run and its stderr text (or a fallback) is returned.
pub fn run(plan: &[Command]) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn sizes_use_decimal_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1_500_000_000), "1.5 GB");
        assert_eq!(format_size(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn mount_table_lines_unescape_spaces() {
        let parsed = parse_mounts("/dev/sdb1 /mnt/my\\040stick vfat rw 0 0\nshort\n");
        assert_eq!(parsed, vec![("/dev/sdb1".to_string(), "/mnt/my stick".to_string())]);
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
    fn scan_reads_disks_and_partitions_from_a_device_tree() {
        let root = std::env::temp_dir().join(format!("bifrost-diskutil-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let disk_dir = root.join("sdb");
        fs::create_dir_all(disk_dir.join("device")).unwrap();
        fs::create_dir_all(disk_dir.join("sdb1")).unwrap();
        fs::create_dir_all(root.join("loop0")).unwrap();
        fs::write(disk_dir.join("size"), "4000\n").unwrap();
        fs::write(disk_dir.join("removable"), "1\n").unwrap();
        fs::write(disk_dir.join("ro"), "0\n").unwrap();
        fs::write(disk_dir.join("device/model"), "USB Stick  \n").unwrap();
        fs::write(disk_dir.join("sdb1/partition"), "1\n").unwrap();
        fs::write(disk_dir.join("sdb1/size"), "2000\n").unwrap();
        fs::write(disk_dir.join("sdb1/ro"), "0\n").unwrap();
        let entries = scan(&root, "/dev/sdb1 /mnt/usb vfat rw 0 0\n");
        let _ = fs::remove_dir_all(&root);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].disk.bsd_name, "sdb");
        assert_eq!(entries[0].disk.size, 4000 * 512);
        assert!(entries[0].disk.removable);
        assert_eq!(entries[0].disk.model, "USB Stick");
        assert_eq!(entries[1].parent.as_deref(), Some("sdb"));
        assert_eq!(entries[1].disk.mount_point.as_deref(), Some("/mnt/usb"));
        assert!(entries[1].disk.removable, "a partition takes its disk's removable flag");
    }
}
