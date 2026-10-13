//! Reads the disks and their partitions from the kernel's device tree (`/sys/block`) and the file system type and label
//! from the udev database (`/run/udev/data`). Only reads files; nothing is written.

use crate::model::{is_hidden_device, parse_udev_db, parse_udev_value, Kind, Node};
use std::fs;
use std::path::Path;

/// The trimmed text of a small file, or an empty string when it cannot be read.
fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The image file a loop device holds, from its `loop/backing_file`. `None` for a device with no file behind it.
fn backing_file(disk_dir: &Path) -> Option<String> {
    let file = read_trim(&disk_dir.join("loop/backing_file"));
    (!file.is_empty()).then_some(file)
}

/// A sysfs size (512-byte sectors) in bytes.
fn sectors_to_bytes(text: &str) -> u64 {
    text.parse::<u64>().unwrap_or(0).saturating_mul(512)
}

/// The file system type and label of a block device, from its `dev` file (`major:minor`) and the udev database.
fn udev_fields(sys_dev: &Path, udev_dir: &Path) -> (Option<String>, Option<String>) {
    let dev = read_trim(&sys_dev.join("dev"));
    if dev.is_empty() {
        return (None, None);
    }
    match fs::read_to_string(udev_dir.join(format!("b{dev}"))) {
        Ok(text) => parse_udev_db(&text),
        Err(_) => (None, None),
    }
}

/// The raw udev database text of a block device, or empty when it has none.
fn udev_text(sys_dev: &Path, udev_dir: &Path) -> String {
    let dev = read_trim(&sys_dev.join("dev"));
    if dev.is_empty() {
        return String::new();
    }
    fs::read_to_string(udev_dir.join(format!("b{dev}"))).unwrap_or_default()
}

/// Reads every disk under `sys_root/block`, with its partitions after it. `sys_root` is `/sys` on the device and
/// `udev_dir` is `/run/udev/data`. Hidden devices (memory, device-mapper, RAID, and loop devices with no image) are left out.
pub fn read_nodes(sys_root: &Path, udev_dir: &Path) -> Vec<Node> {
    let mut names: Vec<String> = match fs::read_dir(sys_root.join("block")) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => return Vec::new(),
    };
    // a loop device shows only while it holds a disk image; the other loop devices are the system's own
    names.retain(|n| !is_hidden_device(n) || backing_file(&sys_root.join("block").join(n)).is_some());
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let disk_dir = sys_root.join("block").join(&name);
        let (fs_type, label) = udev_fields(&disk_dir, udev_dir);
        let image_file = backing_file(&disk_dir);
        let udev = udev_text(&disk_dir, udev_dir);
        let block_size = read_trim(&disk_dir.join("queue/logical_block_size")).parse::<u64>().ok();
        let model = if image_file.is_some() { "Disk image".to_string() } else { read_trim(&disk_dir.join("device/model")) };
        out.push(Node {
            name: name.clone(),
            kind: Kind::Disk,
            parent: None,
            size: sectors_to_bytes(&read_trim(&disk_dir.join("size"))),
            model,
            removable: read_trim(&disk_dir.join("removable")) == "1",
            read_only: read_trim(&disk_dir.join("ro")) == "1",
            fs_type,
            label,
            image_file,
            uuid: parse_udev_value(&udev, "ID_FS_UUID"),
            part_table: parse_udev_value(&udev, "ID_PART_TABLE_TYPE"),
            serial: parse_udev_value(&udev, "ID_SERIAL_SHORT"),
            block_size,
            ..Node::default()
        });
        let mut parts: Vec<String> = match fs::read_dir(&disk_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|child| disk_dir.join(child).join("partition").exists())
                .collect(),
            Err(_) => Vec::new(),
        };
        parts.sort();
        for part in parts {
            let part_dir = disk_dir.join(&part);
            let (fs_type, label) = udev_fields(&part_dir, udev_dir);
            out.push(Node {
                name: part.clone(),
                kind: Kind::Partition,
                parent: Some(name.clone()),
                size: sectors_to_bytes(&read_trim(&part_dir.join("size"))),
                removable: out.last().is_some_and(|d| d.removable),
                read_only: read_trim(&part_dir.join("ro")) == "1",
                fs_type,
                label,
                uuid: parse_udev_value(&udev_text(&part_dir, udev_dir), "ID_FS_UUID"),
                block_size,
                ..Node::default()
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sectors_become_bytes() {
        assert_eq!(sectors_to_bytes("7814037168"), 7_814_037_168 * 512);
        assert_eq!(sectors_to_bytes(""), 0);
    }
}
