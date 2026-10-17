//! Removable volumes for the Files sidebar: the disks mounted under the media folders, and the storage tool's commands
//! that unmount one and power its disk off (the Eject text on a Locations row). The commands are planned as data so the
//! tests check them without a disk; `run_eject` runs them in order and stops at the first failure.
use std::path::PathBuf;

/// The folders a removable volume is mounted under (`/run/media/<user>/<name>`, or `/media/<name>`).
const MEDIA_ROOTS: [&str; 2] = ["/run/media/", "/media/"];
/// The storage tool that unmounts a partition and powers its disk off (the tool the shell's disk image attach uses).
const STORAGE_TOOL: &str = "udisksctl";

/// A removable volume: its name (the folder it is mounted in), where it is mounted, and its partition (`/dev/sdb1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub mount: PathBuf,
    pub device: String,
}

/// Decodes the octal escapes `/proc/mounts` writes for space, tab, newline and backslash (`\040`, `\011`, `\012`, `\134`).
pub fn unescape(field: &str) -> String {
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = i + 3 < b.len() && b[i] == b'\\' && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c));
        if octal {
            let value = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            out.push(value as u8);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The removable volumes in a `/proc/mounts` listing: block devices mounted under a media folder.
pub fn parse_mounts(text: &str) -> Vec<Volume> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = fields.next()?;
            let mount = unescape(fields.next()?);
            if !device.starts_with("/dev/") || !MEDIA_ROOTS.iter().any(|root| mount.starts_with(root)) {
                return None;
            }
            // the volume's name is the last folder of its mount point
            let name = mount.rsplit('/').find(|s| !s.is_empty())?.to_string();
            Some(Volume { name, mount: PathBuf::from(mount), device: device.to_string() })
        })
        .collect()
}

/// The removable volumes mounted now, read from `/proc/mounts` (none when it cannot be read).
pub fn mounted() -> Vec<Volume> {
    std::fs::read_to_string("/proc/mounts").map(|text| parse_mounts(&text)).unwrap_or_default()
}

/// The whole disk a partition belongs to: `/dev/sdb1` gives `/dev/sdb`, `/dev/nvme0n1p2` gives `/dev/nvme0n1`.
/// A whole disk comes back as it is.
pub fn disk_of(device: &str) -> String {
    let name = device.rsplit('/').next().unwrap_or(device);
    let base = if name.starts_with("nvme") || name.starts_with("mmcblk") {
        // these disks number their partitions after a "p" that follows a digit
        match name.rfind('p') {
            Some(p) if p > 0 && name[..p].ends_with(|c: char| c.is_ascii_digit()) && name[p + 1..].bytes().all(|c| c.is_ascii_digit()) && p + 1 < name.len() => &name[..p],
            _ => name,
        }
    } else {
        name.trim_end_matches(|c: char| c.is_ascii_digit())
    };
    format!("/dev/{base}")
}

/// The commands that eject a volume's partition: unmount it, then power its disk off. Each is (program, arguments).
pub fn eject_commands(device: &str) -> Vec<(String, Vec<String>)> {
    vec![
        (STORAGE_TOOL.into(), vec!["unmount".into(), "-b".into(), device.into()]),
        (STORAGE_TOOL.into(), vec!["power-off".into(), "-b".into(), disk_of(device)]),
    ]
}

/// Runs the eject commands in order. Returns the first failure's message (the tool's error text, or why it did not run).
pub fn run_eject(device: &str) -> Result<(), String> {
    for (program, args) in eject_commands(device) {
        let out = std::process::Command::new(&program).args(&args).output().map_err(|e| format!("{program}: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_under_the_media_folders_are_removable_volumes() {
        let text = "/dev/sda2 / ext4 rw 0 0\n\
                    /dev/sdb1 /run/media/olibuijr/Stick vfat rw 0 0\n\
                    /dev/sdc1 /media/Card\\040Reader exfat rw 0 0\n\
                    tmpfs /run/media/x tmpfs rw 0 0\n\
                    /dev/sdd1 /home/olibuijr/mnt ext4 rw 0 0\n";
        let volumes = parse_mounts(text);
        assert_eq!(
            volumes,
            vec![
                Volume { name: "Stick".into(), mount: PathBuf::from("/run/media/olibuijr/Stick"), device: "/dev/sdb1".into() },
                Volume { name: "Card Reader".into(), mount: PathBuf::from("/media/Card Reader"), device: "/dev/sdc1".into() },
            ]
        );
    }

    #[test]
    fn octal_escapes_decode_and_other_bytes_stay() {
        assert_eq!(unescape("a\\040b\\011c\\134d"), "a b\tc\\d");
        assert_eq!(unescape("trailing\\04"), "trailing\\04", "a short escape is kept as written");
        assert_eq!(unescape("Åsa"), "Åsa");
    }

    #[test]
    fn a_partition_maps_to_its_disk() {
        assert_eq!(disk_of("/dev/sdb1"), "/dev/sdb");
        assert_eq!(disk_of("/dev/sdb"), "/dev/sdb");
        assert_eq!(disk_of("/dev/nvme0n1p2"), "/dev/nvme0n1");
        assert_eq!(disk_of("/dev/nvme0n1"), "/dev/nvme0n1");
        assert_eq!(disk_of("/dev/mmcblk0p1"), "/dev/mmcblk0");
    }

    #[test]
    fn eject_unmounts_the_partition_then_powers_the_disk_off() {
        assert_eq!(
            eject_commands("/dev/sdb1"),
            vec![
                ("udisksctl".to_string(), vec!["unmount".to_string(), "-b".to_string(), "/dev/sdb1".to_string()]),
                ("udisksctl".to_string(), vec!["power-off".to_string(), "-b".to_string(), "/dev/sdb".to_string()]),
            ]
        );
    }
}
