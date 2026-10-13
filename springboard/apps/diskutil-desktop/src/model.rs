//! The disk model: what a disk or a volume is, and the parsers that turn the kernel's and the udev database's text
//! into it. Everything here is pure (no file or process access), so it is tested on the host.

use bifrost_core::disk_events::Disk;

/// A whole disk or a partition of one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Disk,
    Partition,
}

/// Space used on a mounted volume, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

/// One row of the sidebar: a disk or a partition, with what the system knows about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// The kernel's device name, such as `sdb` or `nvme0n1p1`.
    pub name: String,
    pub kind: Kind,
    /// For a partition, the name of its disk.
    pub parent: Option<String>,
    pub size: u64,
    pub model: String,
    pub removable: bool,
    pub read_only: bool,
    pub fs_type: Option<String>,
    pub label: Option<String>,
    pub mount_point: Option<String>,
    pub usage: Option<Usage>,
    /// The SMART overall health, when the drive reports it (`Passed`, `Failing`, or `Not available`).
    pub smart: Option<String>,
    /// For a loop device that holds a disk image, the path of the image file (from the kernel's `loop/backing_file`).
    pub image_file: Option<String>,
    /// The file system UUID from the udev database (`ID_FS_UUID`).
    pub uuid: Option<String>,
    /// The partition table type of a disk (`ID_PART_TABLE_TYPE`: `gpt` or `dos`).
    pub part_table: Option<String>,
    /// The drive's serial number (`ID_SERIAL_SHORT`).
    pub serial: Option<String>,
    /// The logical block size of the disk in bytes (`queue/logical_block_size`).
    pub block_size: Option<u64>,
}

/// One line of the mount table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub point: String,
    pub fs_type: String,
}

/// Device names that are never shown: memory disks, loop devices, device-mapper and software RAID.
pub fn is_hidden_device(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "fd", "nbd", "rbd"].iter().any(|p| name.starts_with(p))
}

/// A size in decimal units, as the sizes in a disk's own label read (1 GB is 1000 MB).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Undoes the octal escapes the mount table uses for spaces, tabs, newlines and backslashes.
pub fn unescape_mount(text: &str) -> String {
    text.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\")
}

/// The mount table (`/proc/mounts`): one mount per line, `device point type options dump pass`.
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = fields.next()?;
            let point = fields.next()?;
            let fs_type = fields.next()?;
            Some(Mount { device: unescape_mount(device), point: unescape_mount(point), fs_type: fs_type.to_string() })
        })
        .collect()
}

/// The file system type and label that the udev database (`/run/udev/data/b<major>:<minor>`) holds for a device.
pub fn parse_udev_db(text: &str) -> (Option<String>, Option<String>) {
    let (mut fs, mut label) = (None, None);
    for line in text.lines() {
        let Some(entry) = line.strip_prefix("E:") else { continue };
        if let Some(value) = entry.strip_prefix("ID_FS_TYPE=") {
            fs = Some(value.to_string()).filter(|v| !v.is_empty());
        } else if let Some(value) = entry.strip_prefix("ID_FS_LABEL=") {
            label = Some(value.to_string()).filter(|v| !v.is_empty());
        }
    }
    (fs, label)
}

/// The file system's name as the Disk menu and the sheets write it.
pub fn fs_title(fs: bifrost_core::disk_format::Personality) -> &'static str {
    use bifrost_core::disk_format::Personality;
    match fs {
        Personality::Ext4 => "ext4",
        Personality::ExFat => "exFAT",
        Personality::Fat32 => "FAT32",
        Personality::Ntfs => "NTFS",
        Personality::Btrfs => "Btrfs",
    }
}

/// The value of one `E:KEY=value` line of the udev database. `None` when the key is missing or empty.
pub fn parse_udev_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("E:{key}=");
    text.lines().find_map(|line| line.strip_prefix(prefix.as_str())).map(str::to_string).filter(|v| !v.is_empty())
}

/// The partition table type as a user reads it: `gpt` is GUID Partition Map, `dos` is Master Boot Record.
pub fn scheme_name(part_table: &str) -> String {
    match part_table {
        "gpt" => "GUID Partition Map".to_string(),
        "dos" => "Master Boot Record".to_string(),
        other => other.to_string(),
    }
}

/// The output of `df -B1 --output=target,size,used,avail`: the mount point and its usage, per line. The header and any
/// line whose last three fields are not numbers are skipped; the mount point may hold spaces.
pub fn parse_df(text: &str) -> Vec<(String, Usage)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        let n = fields.len();
        let numbers: Option<Vec<u64>> = fields[n - 3..].iter().map(|f| f.parse::<u64>().ok()).collect();
        let Some(numbers) = numbers else { continue };
        let point = fields[..n - 3].join(" ");
        out.push((point, Usage { total: numbers[0], used: numbers[1], free: numbers[2] }));
    }
    out
}

/// The SMART overall health from `smartctl -H` output. None when the report has no health line.
pub fn parse_smart(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SMART overall-health self-assessment test result:") {
            return Some(match rest.trim() {
                "PASSED" => "Passed".to_string(),
                "FAILED!" => "Failing".to_string(),
                other => other.to_string(),
            });
        }
        if line.contains("SMART support is:") && line.contains("Unavailable") {
            return Some("Not available".to_string());
        }
    }
    None
}

/// Copies the mount table into the nodes: a partition is mounted when its device path is in the table.
pub fn apply_mounts(nodes: &mut [Node], mounts: &[Mount]) {
    for node in nodes.iter_mut() {
        let device = format!("/dev/{}", node.name);
        if let Some(m) = mounts.iter().find(|m| m.device == device) {
            node.mount_point = Some(m.point.clone());
            if node.fs_type.is_none() {
                node.fs_type = Some(m.fs_type.clone());
            }
        }
    }
}

/// Copies the usage table into the mounted nodes, by mount point.
pub fn apply_usage(nodes: &mut [Node], table: &[(String, Usage)]) {
    for node in nodes.iter_mut() {
        if let Some(point) = &node.mount_point {
            node.usage = table.iter().find(|(p, _)| p == point).map(|(_, u)| *u);
        }
    }
}

/// Keeps the usage the last full read measured, for the volumes that are still mounted at the same point.
pub fn carry_usage(nodes: &mut [Node], previous: &[Node]) {
    for node in nodes.iter_mut() {
        if let Some(old) = previous.iter().find(|o| o.name == node.name && o.mount_point == node.mount_point) {
            node.usage = old.usage;
        }
    }
}

/// The part of a volume that is used, from 0 to 1 (0 when the size is unknown).
pub fn used_fraction(usage: Usage) -> f32 {
    if usage.total == 0 {
        return 0.0;
    }
    (usage.used as f32 / usage.total as f32).clamp(0.0, 1.0)
}

/// What the disk session needs for a device.
pub fn to_disk(node: &Node) -> Disk {
    Disk {
        bsd_name: node.name.clone(),
        size: node.size,
        removable: node.removable,
        read_only: node.read_only,
        model: node.model.clone(),
        mount_point: node.mount_point.clone(),
    }
}

/// The name a row shows: the drive's model for a disk, the volume label for a partition, else the device name.
pub fn display_name(node: &Node) -> String {
    match node.kind {
        Kind::Disk if !node.model.is_empty() => node.model.clone(),
        Kind::Partition => node.label.clone().unwrap_or_else(|| node.name.clone()),
        _ => node.name.clone(),
    }
}

/// The facts the info pane lists, in order (ten rows; the SMART row is the last).
pub fn info_rows(node: &Node) -> Vec<(&'static str, String)> {
    let kind = match (node.kind, &node.parent) {
        (Kind::Partition, Some(parent)) => format!("Partition of {parent}"),
        (Kind::Partition, None) => "Partition".to_string(),
        (Kind::Disk, _) if node.image_file.is_some() => "Disk image".to_string(),
        (Kind::Disk, _) if node.removable => "Removable disk".to_string(),
        (Kind::Disk, _) => "Disk".to_string(),
    };
    let (used, free) = match node.usage {
        Some(u) => (format_size(u.used), format_size(u.free)),
        None => ("Not mounted".to_string(), "Not mounted".to_string()),
    };
    vec![
        ("Capacity", format_size(node.size)),
        ("Used", used),
        ("Available", free),
        ("Mount point", node.mount_point.clone().unwrap_or_else(|| "Not mounted".to_string())),
        ("File system", node.fs_type.clone().unwrap_or_else(|| "Unknown".to_string())),
        ("Volume name", node.label.clone().unwrap_or_else(|| "None".to_string())),
        ("Device", format!("/dev/{}", node.name)),
        ("Type", kind),
        ("Writable", if node.read_only { "No".to_string() } else { "Yes".to_string() }),
        ("SMART status", node.smart.clone().unwrap_or_else(|| if node.kind == Kind::Disk { "Not checked".to_string() } else { "Not applicable".to_string() })),
    ]
}

/// Every fact the Get Info sheet lists: the info pane's rows, then the model, the disk a volume sits on, the volume
/// label and the image file where there are any.
pub fn detail_rows(node: &Node) -> Vec<(&'static str, String)> {
    let mut rows = info_rows(node);
    if !node.model.is_empty() {
        rows.push(("Model", node.model.clone()));
    }
    if let Some(parent) = &node.parent {
        rows.push(("Disk", format!("/dev/{parent}")));
    }
    rows.push(("Removable", if node.removable { "Yes".to_string() } else { "No".to_string() }));
    if let Some(uuid) = &node.uuid {
        rows.push(("UUID", uuid.clone()));
    }
    if let Some(table) = &node.part_table {
        rows.push(("Partition map", scheme_name(table)));
    }
    if let Some(serial) = &node.serial {
        rows.push(("Serial number", serial.clone()));
    }
    if let Some(block) = node.block_size {
        rows.push(("Block size", format!("{block} bytes")));
    }
    if let Some(image) = &node.image_file {
        rows.push(("Image file", image.clone()));
    }
    rows
}

/// The volume name an erase keeps: the old label cut to what the new file system stores, or empty when it holds a
/// character the format tools refuse.
pub fn erase_label(label: Option<&str>, limit: usize) -> String {
    let Some(label) = label else { return String::new() };
    let ok = label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-' || b == b'_');
    if !ok {
        return String::new();
    }
    label.chars().take(limit).collect::<String>().trim().to_string()
}

/// The sizes for `count` equal partitions of a disk of `bytes`, in MiB: the last takes the rest (0). Empty when the
/// disk is too small to give each part at least 1 MiB past a 2 MiB table margin, or `count` is 0.
pub fn equal_parts_mib(bytes: u64, count: usize) -> Vec<u64> {
    let total = (bytes / (1024 * 1024)).saturating_sub(2);
    if count == 0 || total < count as u64 {
        return Vec::new();
    }
    let each = total / count as u64;
    let mut sizes = vec![each; count];
    sizes[count - 1] = 0;
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str) -> Node {
        Node { name: name.into(), size: 4_000_000_000, ..Node::default() }
    }

    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(999), "999 bytes");
        assert_eq!(format_size(1_000), "1.0 KB");
        assert_eq!(format_size(4_000_000_000), "4.0 GB");
        assert_eq!(format_size(512_110_190_592), "512 GB");
        assert_eq!(format_size(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn mounts_unescape_spaces_and_skip_short_lines() {
        let text = "/dev/sdb1 /run/media/me/My\\040Stick vfat rw,nosuid 0 0\nproc /proc proc rw 0 0\nbroken\n";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].device, "/dev/sdb1");
        assert_eq!(mounts[0].point, "/run/media/me/My Stick");
        assert_eq!(mounts[0].fs_type, "vfat");
    }

    #[test]
    fn udev_db_gives_type_and_label() {
        let text = "I:1700000000\nE:ID_FS_TYPE=ext4\nE:ID_FS_LABEL=Games\nE:ID_PART_ENTRY_SIZE=2048\n";
        assert_eq!(parse_udev_db(text), (Some("ext4".into()), Some("Games".into())));
        assert_eq!(parse_udev_db("E:ID_PART_TABLE_TYPE=gpt\n"), (None, None));
        assert_eq!(parse_udev_db("E:ID_FS_TYPE=\n"), (None, None));
    }

    #[test]
    fn df_skips_the_header_and_keeps_spaced_targets() {
        let text = "Mounted on        1B-blocks          Used         Avail\n/ 1000 400 600\n/run/media/me/My Stick 2000 500 1500\n";
        let table = parse_df(text);
        assert_eq!(table.len(), 2);
        assert_eq!(table[0], ("/".to_string(), Usage { total: 1000, used: 400, free: 600 }));
        assert_eq!(table[1].0, "/run/media/me/My Stick");
    }

    #[test]
    fn smart_health_lines_are_read() {
        let passed = "=== START OF READ SMART DATA SECTION ===\nSMART overall-health self-assessment test result: PASSED\n";
        assert_eq!(parse_smart(passed), Some("Passed".to_string()));
        assert_eq!(parse_smart("SMART overall-health self-assessment test result: FAILED!\n"), Some("Failing".to_string()));
        assert_eq!(parse_smart("SMART support is: Unavailable - device lacks SMART capability.\n"), Some("Not available".to_string()));
        assert_eq!(parse_smart("smartctl: permission denied\n"), None);
    }

    #[test]
    fn mounts_and_usage_land_on_the_right_nodes() {
        let mut nodes = vec![disk("sdb"), Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), ..disk("sdb1") }];
        let mounts = vec![Mount { device: "/dev/sdb1".into(), point: "/mnt/x".into(), fs_type: "exfat".into() }];
        apply_mounts(&mut nodes, &mounts);
        assert_eq!(nodes[0].mount_point, None);
        assert_eq!(nodes[1].mount_point.as_deref(), Some("/mnt/x"));
        assert_eq!(nodes[1].fs_type.as_deref(), Some("exfat"));
        apply_usage(&mut nodes, &[("/mnt/x".into(), Usage { total: 10, used: 4, free: 6 })]);
        assert_eq!(nodes[1].usage, Some(Usage { total: 10, used: 4, free: 6 }));
        assert!((used_fraction(nodes[1].usage.unwrap()) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn carried_usage_needs_the_same_mount_point() {
        let mut old = disk("sdb1");
        old.mount_point = Some("/a".into());
        old.usage = Some(Usage { total: 1, used: 1, free: 0 });
        let mut fresh = disk("sdb1");
        fresh.mount_point = Some("/a".into());
        carry_usage(std::slice::from_mut(&mut fresh), &[old.clone()]);
        assert_eq!(fresh.usage, old.usage);
        let mut moved = disk("sdb1");
        moved.mount_point = Some("/b".into());
        carry_usage(std::slice::from_mut(&mut moved), &[old]);
        assert_eq!(moved.usage, None);
    }

    #[test]
    fn info_rows_describe_a_partition_and_an_unmounted_disk() {
        let part = Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), size: 1_000_000, ..Node::default() };
        let rows = info_rows(&part);
        assert_eq!(rows[0], ("Capacity", "1.0 MB".to_string()));
        assert_eq!(rows[1], ("Used", "Not mounted".to_string()));
        assert_eq!(rows[7], ("Type", "Partition of sdb".to_string()));
        assert_eq!(rows[9], ("SMART status", "Not applicable".to_string()));
        let usb = Node { name: "sdc".into(), removable: true, ..Node::default() };
        assert_eq!(info_rows(&usb)[7], ("Type", "Removable disk".to_string()));
    }

    #[test]
    fn hidden_devices_are_filtered() {
        assert!(is_hidden_device("loop3"));
        assert!(is_hidden_device("dm-0"));
        assert!(!is_hidden_device("sdb"));
        assert!(!is_hidden_device("nvme0n1"));
    }

    #[test]
    fn display_name_prefers_model_then_label() {
        let d = Node { name: "sdb".into(), model: "Cruzer".into(), ..Node::default() };
        assert_eq!(display_name(&d), "Cruzer");
        let p = Node { name: "sdb1".into(), kind: Kind::Partition, label: Some("Games".into()), ..Node::default() };
        assert_eq!(display_name(&p), "Games");
        assert_eq!(display_name(&disk("sdc")), "sdc");
    }

    #[test]
    fn detail_rows_extend_the_info_rows() {
        let n = Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), removable: true, ..Node::default() };
        let rows = detail_rows(&n);
        assert_eq!(rows.len(), info_rows(&n).len() + 2);
        assert!(rows.contains(&("Disk", "/dev/sdb".to_string())));
        let img = Node { name: "loop0".into(), image_file: Some("/a.iso".into()), model: "Loop".into(), ..Node::default() };
        let rows = detail_rows(&img);
        assert!(rows.contains(&("Image file", "/a.iso".to_string())) && rows.contains(&("Model", "Loop".to_string())));
    }

    #[test]
    fn the_erase_label_is_kept_only_when_the_format_accepts_it() {
        assert_eq!(erase_label(Some("Games"), 11), "Games");
        assert_eq!(erase_label(Some("A very long volume"), 11), "A very long");
        assert_eq!(erase_label(Some("Caf\u{e9}"), 11), "");
        assert_eq!(erase_label(None, 11), "");
        assert_eq!(erase_label(Some("Ab cdefghi jk"), 11), "Ab cdefghi");
    }

    #[test]
    fn equal_parts_leave_the_rest_to_the_last() {
        assert_eq!(equal_parts_mib(8 * 1024 * 1024 * 1024, 2), vec![4095, 0]);
        assert_eq!(equal_parts_mib(10 * 1024 * 1024, 3), vec![2, 2, 0]);
        assert!(equal_parts_mib(3 * 1024 * 1024, 3).is_empty());
        assert!(equal_parts_mib(1 << 30, 0).is_empty());
    }

    #[test]
    fn a_udev_value_is_read_by_key() {
        let text = "E:ID_FS_UUID=1234-ABCD\nE:ID_PART_TABLE_TYPE=gpt\nE:ID_SERIAL_SHORT=\n";
        assert_eq!(parse_udev_value(text, "ID_FS_UUID").as_deref(), Some("1234-ABCD"));
        assert_eq!(parse_udev_value(text, "ID_SERIAL_SHORT"), None);
        assert_eq!(parse_udev_value(text, "ID_MISSING"), None);
        assert_eq!(scheme_name("gpt"), "GUID Partition Map");
        assert_eq!(scheme_name("dos"), "Master Boot Record");
    }

    #[test]
    fn get_info_lists_the_uuid_map_serial_and_block_size() {
        let node = Node { name: "sdb".into(), uuid: Some("1234".into()), part_table: Some("gpt".into()), serial: Some("S1".into()), block_size: Some(512), ..Node::default() };
        let rows = detail_rows(&node);
        for (key, value) in [("UUID", "1234"), ("Partition map", "GUID Partition Map"), ("Serial number", "S1"), ("Block size", "512 bytes")] {
            assert!(rows.iter().any(|(k, v)| *k == key && v == value), "{key}");
        }
    }
}
