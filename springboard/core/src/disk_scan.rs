//! Reads the disks and partitions the kernel reports, from the device tree (`/sys/block`) and the mount table
//! (`/proc/mounts`). Pure reads: the caller passes the paths and the mount table text. The result feeds
//! `disk_session` and the disk plans in `disk_format`, for the command line tool and the privileged helper.

use crate::disk_events::Disk;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A disk or a partition, and the disk a partition belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub disk: Disk,
    pub parent: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_table_lines_unescape_spaces() {
        let parsed = parse_mounts("/dev/sdb1 /mnt/my\\040stick vfat rw 0 0\nshort\n");
        assert_eq!(parsed, vec![("/dev/sdb1".to_string(), "/mnt/my stick".to_string())]);
    }

    #[test]
    fn scan_reads_disks_and_partitions_from_a_device_tree() {
        let root = std::env::temp_dir().join(format!("bifrost-disk-scan-{}", std::process::id()));
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
