//! The pure parts of the commands that take typed input: sizes for a custom partition layout, the blank disk image
//! plan, attaching and converting an image file, and the volumes a restore can copy from. Nothing here touches a file
//! or runs a process; the window runs the plans on a thread.

use crate::model::{self, Kind, Node};
use bifrost_core::disk_commands::Command;
use bifrost_core::disk_format::{self, Personality, MAX_PARTITIONS};

const MIB: u64 = 1024 * 1024;

/// Reads a size such as `64`, `1.5 GB`, `512MB` or `2 GiB` in bytes. A bare number is gigabytes; `KB`, `MB`, `GB` and
/// `TB` are decimal (the units the window shows), `KiB`, `MiB`, `GiB` and `TiB` are binary. `None` for anything else.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number.parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
    let factor: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "g" | "gb" => 1e9,
        "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => MIB as f64,
        "gib" => (1u64 << 30) as f64,
        "tib" => (1u64 << 40) as f64,
        _ => return None,
    };
    let bytes = value * factor;
    (bytes < u64::MAX as f64).then_some(bytes as u64)
}

/// Reads the sizes typed for a custom partition layout, such as `64 GB, 25%, rest`, into MiB per partition. `rest` (or an
/// empty entry) takes what is left and is allowed for the last partition only; it is returned as 0, the value a
/// partition request uses for it. `disk_bytes` is the size of the disk, less a 2 MiB margin for the table.
pub fn parse_partition_sizes(text: &str, disk_bytes: u64) -> Result<Vec<u64>, String> {
    if text.trim().is_empty() {
        return Err("Enter a size for each partition, separated by commas.".to_string());
    }
    let total = (disk_bytes / MIB).saturating_sub(2);
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() > MAX_PARTITIONS {
        return Err(format!("A disk takes at most {MAX_PARTITIONS} partitions."));
    }
    let last = parts.len() - 1;
    let mut sizes = Vec::with_capacity(parts.len());
    let mut used = 0u64;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || part.eq_ignore_ascii_case("rest") || *part == "*" {
            if i != last {
                return Err("Only the last partition can take the rest of the disk.".to_string());
            }
            sizes.push(0);
            continue;
        }
        let mib = if let Some(percent) = part.strip_suffix('%') {
            let p: f64 = percent.trim().parse().ok().filter(|p: &f64| *p > 0.0 && *p <= 100.0).ok_or_else(|| format!("\u{201c}{part}\u{201d} is not a percentage."))?;
            (total as f64 * p / 100.0) as u64
        } else {
            parse_size(part).ok_or_else(|| format!("\u{201c}{part}\u{201d} is not a size."))? / MIB
        };
        if mib == 0 {
            return Err(format!("\u{201c}{part}\u{201d} is smaller than 1 MiB."));
        }
        used += mib;
        sizes.push(mib);
    }
    let rest_wanted = sizes.last() == Some(&0);
    if used > total || (rest_wanted && used >= total) {
        return Err("The partitions do not fit on the disk.".to_string());
    }
    Ok(sizes)
}

/// The names of the volumes a restore onto `target` can copy from: the same kind of device, not mounted (a disk with a
/// mounted volume does not count), not the target, and no larger than the target.
pub fn restore_sources(nodes: &[Node], target: &Node) -> Vec<String> {
    let busy = |n: &Node| n.mount_point.is_some() || nodes.iter().any(|c| c.parent.as_deref() == Some(n.name.as_str()) && c.mount_point.is_some());
    nodes.iter().filter(|n| n.name != target.name && n.kind == target.kind && n.size > 0 && n.size <= target.size && !busy(n)).map(|n| n.name.clone()).collect()
}

/// The file name for a new image: the typed name without a trailing `.img`, made of letters, digits, spaces, `-`, `_`
/// and `.`, up to 64 bytes. Returns `(stem, file name)`.
pub fn image_file_name(name: &str) -> Result<(String, String), String> {
    let name = name.trim();
    let stem = name.strip_suffix(".img").unwrap_or(name).trim();
    let ok = !stem.is_empty()
        && stem.len() <= 64
        && !stem.starts_with('.')
        && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-' || b == b'_' || b == b'.');
    if !ok {
        return Err("Name the image with letters, digits, spaces, dashes and underscores.".to_string());
    }
    Ok((stem.to_string(), format!("{stem}.img")))
}

/// Where a new image is saved: the user's Documents folder.
pub fn images_dir(home: &str) -> String {
    format!("{}/Documents", home.trim_end_matches('/'))
}

/// The plan for a new blank image: make the folder, create and format the file, attach it for writing. Returns the
/// image's path and the commands.
pub fn new_image_plan(home: &str, name: &str, size_text: &str, fs: Personality) -> Result<(String, Vec<Command>), String> {
    if !home.starts_with('/') {
        return Err("The home folder is not known.".to_string());
    }
    let (stem, file) = image_file_name(name)?;
    let size = parse_size(size_text).ok_or_else(|| format!("\u{201c}{}\u{201d} is not a size.", size_text.trim()))?;
    let dir = images_dir(home);
    let path = format!("{dir}/{file}");
    let label = model::erase_label(Some(&stem), fs.label_limit());
    let mut plan = vec![Command { program: "mkdir", args: vec!["-p".to_string(), dir] }];
    plan.extend(disk_format::blank_image_plan(&path, size, fs, &label).map_err(|r| r.sentence())?);
    plan.push(Command { program: "hdiutil", args: vec!["attach".to_string(), "-readwrite".to_string(), path.clone()] });
    Ok((path, plan))
}

/// The plan that attaches an image file the user opened. Read-only is the image tool's default.
pub fn attach_plan(path: &str) -> Result<Vec<Command>, String> {
    if !path.starts_with('/') {
        return Err("The image file's location is not known.".to_string());
    }
    Ok(vec![Command { program: "hdiutil", args: vec!["attach".to_string(), path.to_string()] }])
}

/// The formats Images > Convert writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Qcow2,
    Raw,
}

impl ImageFormat {
    fn tool_name(self) -> &'static str {
        match self {
            ImageFormat::Qcow2 => "qcow2",
            ImageFormat::Raw => "raw",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            ImageFormat::Qcow2 => "qcow2",
            ImageFormat::Raw => "img",
        }
    }
}

/// The plan that writes a converted copy of `source` next to it. Returns the copy's path and the commands. The copy
/// never replaces the source: when the source already has the copy's name, the conversion is refused.
pub fn convert_plan(source: &str, format: ImageFormat) -> Result<(String, Vec<Command>), String> {
    if !source.starts_with('/') {
        return Err("The image file's location is not known.".to_string());
    }
    let (dir, file) = source.rsplit_once('/').unwrap_or(("", source));
    let stem = file.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(file);
    let target = format!("{dir}/{stem}.{}", format.extension());
    if target == source {
        return Err(format!("{file} is already in that format."));
    }
    let args = ["convert", "-O", format.tool_name(), source, target.as_str()].map(String::from).to_vec();
    Ok((target.clone(), vec![Command { program: "qemu-img", args }]))
}

/// Whether the node is a loop device holding an image (Convert and Detach apply to it).
pub fn is_image(node: &Node) -> bool {
    node.kind == Kind::Disk && node.image_file.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    #[test]
    fn sizes_read_decimal_and_binary_units_and_default_to_gigabytes() {
        assert_eq!(parse_size("64"), Some(64 * GB));
        assert_eq!(parse_size("1.5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("512MB"), Some(512_000_000));
        assert_eq!(parse_size("2 GiB"), Some(2 << 30));
        assert_eq!(parse_size("100 mib"), Some(100 * MIB));
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("5 parsecs"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn a_custom_layout_gives_mib_per_partition_and_zero_for_the_rest() {
        let disk = 128 * GB;
        let sizes = parse_partition_sizes("32 GB, 25%, rest", disk).unwrap();
        assert_eq!(sizes.len(), 3);
        assert_eq!(sizes[0], 32 * GB / MIB);
        assert_eq!(sizes[1], ((disk / MIB - 2) as f64 * 0.25) as u64);
        assert_eq!(sizes[2], 0);
        assert_eq!(parse_partition_sizes("10, ", disk).unwrap(), vec![10 * GB / MIB, 0]);
    }

    #[test]
    fn a_layout_that_does_not_fit_or_misplaces_the_rest_is_refused() {
        let disk = 8 * GB;
        assert!(parse_partition_sizes("", disk).unwrap_err().contains("Enter"));
        assert!(parse_partition_sizes("6, 6", disk).unwrap_err().contains("fit"));
        assert!(parse_partition_sizes("8, rest", disk).unwrap_err().contains("fit"));
        assert!(parse_partition_sizes("rest, 1", disk).unwrap_err().contains("last"));
        assert!(parse_partition_sizes("1 foo", disk).unwrap_err().contains("not a size"));
        assert!(parse_partition_sizes("0 MB", disk).unwrap_err().contains("smaller"));
        assert!(parse_partition_sizes("150%", disk).unwrap_err().contains("percentage"));
        let many = vec!["1 MB"; MAX_PARTITIONS + 1].join(",");
        assert!(parse_partition_sizes(&many, disk).unwrap_err().contains("at most"));
    }

    fn node(name: &str, kind: Kind, size: u64, mount: Option<&str>, parent: Option<&str>) -> Node {
        Node { name: name.into(), kind, size, mount_point: mount.map(String::from), parent: parent.map(String::from), ..Node::default() }
    }

    #[test]
    fn restore_sources_are_unmounted_smaller_devices_of_the_same_kind() {
        let nodes = vec![
            node("sdb", Kind::Disk, 8 * GB, None, None),
            node("sdb1", Kind::Partition, 8 * GB, None, Some("sdb")),
            node("sdc", Kind::Disk, 16 * GB, None, None),
            node("sdc1", Kind::Partition, 4 * GB, Some("/m"), Some("sdc")),
            node("sdd1", Kind::Partition, 2 * GB, None, Some("sdd")),
            node("sde1", Kind::Partition, 32 * GB, None, Some("sde")),
        ];
        assert_eq!(restore_sources(&nodes, &nodes[1]), vec!["sdd1".to_string()]);
        // a disk whose volume is mounted is not a source for another disk
        assert_eq!(restore_sources(&nodes, &nodes[2]), vec!["sdb".to_string()]);
    }

    #[test]
    fn an_image_name_is_cleaned_and_gets_the_img_extension() {
        assert_eq!(image_file_name("  Backup  "), Ok(("Backup".to_string(), "Backup.img".to_string())));
        assert_eq!(image_file_name("Backup.img").unwrap().1, "Backup.img");
        assert!(image_file_name("../x").is_err());
        assert!(image_file_name("").is_err());
        assert!(image_file_name(".hidden").is_err());
        assert!(image_file_name("a/b").is_err());
    }

    #[test]
    fn a_new_image_is_made_formatted_and_attached() {
        let (path, plan) = new_image_plan("/home/me/", "Backup", "100 MB", Personality::Ext4).unwrap();
        assert_eq!(path, "/home/me/Documents/Backup.img");
        let programs: Vec<&str> = plan.iter().map(|c| c.program).collect();
        assert_eq!(programs, vec!["mkdir", "truncate", "mkfs.ext4", "hdiutil"]);
        assert_eq!(plan[1].args, vec!["-s".to_string(), "100000000".to_string(), path.clone()]);
        assert_eq!(plan[3].args, vec!["attach".to_string(), "-readwrite".to_string(), path]);
    }

    #[test]
    fn a_new_image_refuses_a_bad_size_name_or_home() {
        assert!(new_image_plan("/home/me", "Backup", "lots", Personality::Ext4).unwrap_err().contains("not a size"));
        assert!(new_image_plan("/home/me", "Backup", "1 KB", Personality::Ext4).is_err());
        assert!(new_image_plan("/home/me", "a/b", "1", Personality::Ext4).is_err());
        assert!(new_image_plan("home", "Backup", "1", Personality::Ext4).is_err());
    }

    #[test]
    fn a_fat_image_keeps_a_label_the_format_accepts() {
        let (_, plan) = new_image_plan("/h", "A very long image name", "1", Personality::Fat32).unwrap();
        let label_at = plan[2].args.iter().position(|a| a == "-n").expect("label flag");
        assert!(plan[2].args[label_at + 1].len() <= 11);
    }

    #[test]
    fn attach_needs_an_absolute_path() {
        assert_eq!(attach_plan("/home/me/a.iso").unwrap()[0].args, vec!["attach".to_string(), "/home/me/a.iso".to_string()]);
        assert!(attach_plan("a.iso").is_err());
    }

    #[test]
    fn a_conversion_writes_a_copy_next_to_the_source() {
        let (target, plan) = convert_plan("/home/me/a.img", ImageFormat::Qcow2).unwrap();
        assert_eq!(target, "/home/me/a.qcow2");
        assert_eq!(plan[0].program, "qemu-img");
        assert_eq!(plan[0].args, ["convert", "-O", "qcow2", "/home/me/a.img", "/home/me/a.qcow2"].map(String::from).to_vec());
        assert!(convert_plan("/home/me/a.img", ImageFormat::Raw).unwrap_err().contains("already"));
        assert!(convert_plan("a.img", ImageFormat::Raw).is_err());
    }
}
