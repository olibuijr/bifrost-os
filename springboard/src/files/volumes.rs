//! Removable volumes for the Files sidebar: the disks mounted under the media folders, and the storage tool's commands
//! that unmount one and power its disk off (the Eject text on a Locations row). The commands are planned as data so the
//! tests check them without a disk; `run_eject` runs them in order and stops at the first failure.
//!
//! When an eject fails because a program still has a file or folder of the volume open, `blockers` finds those programs
//! (the unmount assistant) and `eject_notice` writes the one line the Files status bar shows: which programs hold the
//! volume, so the person can close them and try again. `eject_with_notice` is the whole flow.
use std::path::{Path, PathBuf};

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

/// A program that holds a file or folder of a volume open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocker {
    pub pid: u32,
    /// the program's name (its `comm`), or `process <pid>` when it has none
    pub name: String,
}

/// The programs under `proc_root` (`/proc`) whose working folder or an open file is inside `mount`, by process id.
/// `skip` is a process left out of the answer (the caller's own). A process that exits while the folder is read, or
/// that the caller may not inspect, is passed over.
pub fn blockers_in(proc_root: &Path, mount: &Path, skip: u32) -> Vec<Blocker> {
    let mut out: Vec<Blocker> = Vec::new();
    let Ok(dir) = std::fs::read_dir(proc_root) else { return out };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == skip {
            continue;
        }
        let base = entry.path();
        let inside = |link: &Path| std::fs::read_link(link).is_ok_and(|target| target.starts_with(mount));
        let mut holds = inside(&base.join("cwd"));
        if !holds {
            if let Ok(fds) = std::fs::read_dir(base.join("fd")) {
                holds = fds.flatten().any(|fd| inside(&fd.path()));
            }
        }
        if holds {
            let comm = std::fs::read_to_string(base.join("comm")).unwrap_or_default();
            let name = if comm.trim().is_empty() { format!("process {pid}") } else { comm.trim().to_string() };
            out.push(Blocker { pid, name });
        }
    }
    out.sort_by_key(|b| b.pid);
    out
}

/// The programs holding `mount` open now, leaving out this process.
pub fn blockers(mount: &Path) -> Vec<Blocker> {
    blockers_in(Path::new("/proc"), mount, std::process::id())
}

/// The most program names the notice lists before it counts the rest.
const NOTICE_NAMES: usize = 3;

/// The line shown when `volume` could not be ejected. With `blockers` it names the programs to close; without any it
/// gives the first line of the tool's `error`, or just the failure when the tool said nothing.
pub fn eject_notice(volume: &str, error: &str, blockers: &[Blocker]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for b in blockers {
        if !names.contains(&b.name.as_str()) {
            names.push(&b.name);
        }
    }
    if !names.is_empty() {
        let shown: Vec<&str> = names.iter().take(NOTICE_NAMES).copied().collect();
        let mut list = shown.join(", ");
        if names.len() > shown.len() {
            list.push_str(&format!(" and {} more", names.len() - shown.len()));
        }
        let them = if names.len() == 1 { "it" } else { "them" };
        return format!("\u{201c}{volume}\u{201d} could not be ejected. In use by {list}. Close {them} and try again.");
    }
    match error.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) => format!("\u{201c}{volume}\u{201d} could not be ejected: {line}"),
        None => format!("\u{201c}{volume}\u{201d} could not be ejected."),
    }
}

/// Ejects the volume at `mount` (partition `device`). `None` when it went; otherwise the notice for the status bar,
/// naming the programs that hold the volume when there are any.
pub fn eject_with_notice(name: &str, device: &str, mount: &Path) -> Option<String> {
    run_eject(device).err().map(|e| eject_notice(name, &e, &blockers(mount)))
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

    fn blocker(pid: u32, name: &str) -> Blocker {
        Blocker { pid, name: name.into() }
    }

    #[test]
    fn the_notice_names_the_programs_to_close() {
        let one = eject_notice("Stick", "target is busy", &[blocker(7, "vlc")]);
        assert_eq!(one, "\u{201c}Stick\u{201d} could not be ejected. In use by vlc. Close it and try again.");
        let many = [blocker(1, "a"), blocker(2, "b"), blocker(3, "a"), blocker(4, "c"), blocker(5, "d"), blocker(6, "e")];
        let text = eject_notice("Stick", "", &many);
        assert!(text.contains("In use by a, b, c and 2 more."), "{text}");
        assert!(text.contains("Close them"), "{text}");
    }

    #[test]
    fn without_programs_the_notice_gives_the_tools_first_line() {
        assert_eq!(eject_notice("Stick", "\n Error unmounting: busy\nmore", &[]), "\u{201c}Stick\u{201d} could not be ejected: Error unmounting: busy");
        assert_eq!(eject_notice("Stick", "  ", &[]), "\u{201c}Stick\u{201d} could not be ejected.");
    }

    #[cfg(unix)]
    #[test]
    fn processes_with_a_file_or_folder_on_the_volume_are_found() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("unmount-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |pid: &str, cwd: &str, fds: &[&str], comm: &str| {
            let base = root.join(pid);
            std::fs::create_dir_all(base.join("fd")).unwrap();
            symlink(cwd, base.join("cwd")).unwrap();
            for (i, target) in fds.iter().enumerate() {
                symlink(target, base.join("fd").join(i.to_string())).unwrap();
            }
            std::fs::write(base.join("comm"), format!("{comm}\n")).unwrap();
        };
        mk("100", "/home/me", &["/media/Stick/movie.mkv"], "player");
        mk("200", "/media/Stick/docs", &[], "shell");
        mk("300", "/media/Stick2/x", &["/media/StickyNotes/y"], "other");
        mk("400", "/home/me", &["/dev/null"], "idle");
        mk("500", "/media/Stick", &[], "");
        std::fs::create_dir_all(root.join("self")).unwrap();
        let found = blockers_in(&root, Path::new("/media/Stick"), 500 + 1);
        let pids: Vec<u32> = found.iter().map(|b| b.pid).collect();
        assert_eq!(pids, [100, 200, 500], "only whole path components match");
        assert_eq!(found[0].name, "player");
        assert_eq!(found[2].name, "process 500", "a process with no name is told by its number");
        let without_self = blockers_in(&root, Path::new("/media/Stick"), 100);
        assert!(without_self.iter().all(|b| b.pid != 100));
        std::fs::remove_dir_all(&root).unwrap();
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
