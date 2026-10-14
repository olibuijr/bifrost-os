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
}

/// One line of the mount table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub point: String,
    pub fs_type: String,
}

/// Device names that are never shown: memory disks, loop devices and device-mapper devices. Software RAID arrays (`md`)
/// are shown, so an array the RAID Assistant made, or one assembled at boot, can be selected and erased or inspected.
pub fn is_hidden_device(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "fd", "nbd", "rbd"].iter().any(|p| name.starts_with(p))
}

/// A software RAID array's device name: `md` and then digits (the kernel's `mdN`, and `mdNpM` for its partitions).
pub fn is_raid_array(name: &str) -> bool {
    name.strip_prefix("md").is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// A size in decimal units, as the sizes in a disk's own label read (1 GB is 1000 MB).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    // compliance-allow: LAY-03 Decimal bytes-per-unit threshold for capacity formatting, not window size.
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

/// Breaks `text` into lines of at most `width` characters at spaces, keeping its own line breaks. A word longer than the
/// width gets a line to itself. An alert sheet draws one line per `\n` and cuts what does not fit, so its message is
/// wrapped here first.
pub fn wrap_text(text: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Reads the `KEY="value"` pairs of the line `lsblk -P` prints for a device. Values hold `\xNN` escapes for bytes that
/// are not safe to print (a quote is `\x22`), which are decoded here.
pub fn parse_lsblk_pairs(text: &str) -> Vec<(String, String)> {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut pairs = Vec::new();
    let mut rest = line.trim_start();
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim();
        let body = &rest[eq + 2..];
        let Some(end) = body.find('"') else { break };
        pairs.push((key.to_string(), unescape_lsblk(&body[..end])));
        rest = body[end + 1..].trim_start();
    }
    pairs
}

/// Decodes the `\xNN` escapes of an `lsblk -P` value.
fn unescape_lsblk(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'x') { text.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) } else { None };
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `lsblk` fields Get Info adds to the info pane's rows, in the order they are listed, with the labels they show.
const LSBLK_FIELDS: [(&str, &str); 14] = [
    ("UUID", "UUID"),
    ("PARTUUID", "Partition UUID"),
    ("PARTLABEL", "Partition name"),
    ("PARTTYPENAME", "Partition type"),
    ("PTTYPE", "Partition map"),
    ("PTUUID", "Disk UUID"),
    ("LOG-SEC", "Logical block size"),
    ("PHY-SEC", "Physical block size"),
    ("ROTA", "Rotational"),
    ("TRAN", "Transport"),
    ("VENDOR", "Vendor"),
    ("SERIAL", "Serial number"),
    ("WWN", "WWN"),
    ("REV", "Firmware revision"),
];

/// The rows Get Info adds from the pairs `parse_lsblk_pairs` read: the identifiers, block sizes and drive facts that the
/// scan of `/sys` does not give. Empty fields are left out.
pub fn extra_rows(pairs: &[(String, String)]) -> Vec<(String, String)> {
    LSBLK_FIELDS
        .iter()
        .filter_map(|(key, label)| {
            let value = pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())?;
            let shown = match *key {
                "ROTA" => (if value == "1" { "Yes" } else { "No" }).to_string(),
                "LOG-SEC" | "PHY-SEC" => format!("{value} bytes"),
                _ => value.to_string(),
            };
            Some((label.to_string(), shown))
        })
        .collect()
}

/// The lines of the Get Info sheet: every row of `detail_rows`, then the extra rows.
pub fn info_lines(node: &Node, extra: &[(String, String)]) -> Vec<String> {
    detail_rows(node).iter().map(|(k, v)| format!("{k}: {v}")).chain(extra.iter().map(|(k, v)| format!("{k}: {v}"))).collect()
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
        assert!(!is_hidden_device("md0"), "a RAID array is a disk the user can select");
        assert!(!is_hidden_device("sdb"));
        assert!(!is_hidden_device("nvme0n1"));
    }

    #[test]
    fn raid_arrays_are_md_and_a_number_and_nothing_else() {
        assert!(is_raid_array("md0"));
        assert!(is_raid_array("md127"));
        assert!(is_raid_array("md0p1"));
        assert!(!is_raid_array("mmcblk0"), "an SD card is not an array");
        assert!(!is_raid_array("md"));
        assert!(!is_raid_array("sdb"));
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
    fn long_messages_wrap_at_spaces_and_keep_their_own_breaks() {
        assert_eq!(wrap_text("Erase sdb and format it as FAT32? All data on it will be lost.", 34), "Erase sdb and format it as FAT32?\nAll data on it will be lost.");
        assert_eq!(wrap_text("one\ntwo three", 5), "one\ntwo\nthree");
        assert_eq!(wrap_text("unbreakable-word-here ok", 8), "unbreakable-word-here\nok");
        assert_eq!(wrap_text("", 10), "");
        for line in wrap_text("Create the RAID 5 array md0 from sdb, sdc, sdd? All data on the members will be lost.", 34).lines() {
            assert!(line.chars().count() <= 34, "{line}");
        }
    }

    #[test]
    fn lsblk_pairs_keep_values_with_spaces_and_decode_escapes() {
        let text = "NAME=\"sdb1\" LABEL=\"My\\x20Stick\" UUID=\"1A2B-3C4D\" SERIAL=\"\" NOTE=\"say \\x22hi\\x22\"\n";
        let pairs = parse_lsblk_pairs(text);
        assert_eq!(pairs.len(), 5);
        assert_eq!(pairs[0], ("NAME".to_string(), "sdb1".to_string()));
        assert_eq!(pairs[1].1, "My Stick");
        assert_eq!(pairs[3], ("SERIAL".to_string(), String::new()));
        assert_eq!(pairs[4].1, "say \"hi\"");
        assert!(parse_lsblk_pairs("").is_empty());
        assert!(parse_lsblk_pairs("\n\nNAME=\"unterminated").is_empty());
    }

    #[test]
    fn get_info_adds_identifiers_and_block_sizes_and_skips_empty_fields() {
        let pairs = parse_lsblk_pairs("NAME=\"sdb1\" UUID=\"1A2B-3C4D\" PARTUUID=\"\" LOG-SEC=\"512\" PHY-SEC=\"4096\" ROTA=\"0\" TRAN=\"usb\" SERIAL=\"X9\"");
        let rows = extra_rows(&pairs);
        let labels: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(labels, vec!["UUID", "Logical block size", "Physical block size", "Rotational", "Transport", "Serial number"]);
        assert_eq!(rows[1].1, "512 bytes");
        assert_eq!(rows[3].1, "No");
        assert!(extra_rows(&[]).is_empty());
    }

    #[test]
    fn the_info_lines_list_the_detail_rows_then_the_extra_rows() {
        let node = Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), ..Node::default() };
        let lines = info_lines(&node, &[("UUID".to_string(), "1A2B".to_string())]);
        assert!(lines.contains(&"Device: /dev/sdb1".to_string()));
        assert_eq!(lines.last().map(String::as_str), Some("UUID: 1A2B"));
    }
}
