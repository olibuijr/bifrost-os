//! Removable volumes for the Files sidebar: the disks mounted under the media folders, and the words for an eject
//! that failed. Files does not eject a volume itself: its sandbox has no system bus and no device nodes, so it asks
//! the shell's `disks` service (`shell::volume_eject`, and `shell::disk_image` for a disk image's volume).
//!
//! When an unmount fails because a program still has a file or folder of the volume open, `blockers` finds those
//! programs (the shell runs it, it sees every process) and `in_use_line` names them, so the person can close them and
//! try again. `eject_notice` writes the one line the Files status bar shows.
use std::path::{Path, PathBuf};

/// The folders a removable volume is mounted under (`/run/media/<user>/<name>`, or `/media/<name>`).
const MEDIA_ROOTS: [&str; 2] = ["/run/media/", "/media/"];

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

/// Whether `path` is under one of the media folders, where removable volumes are mounted.
pub fn under_media(path: &Path) -> bool {
    MEDIA_ROOTS.iter().any(|root| path.starts_with(root))
}

/// Whether `path` is a folder of a volume that is gone: under a media folder, not there any more (`exists`), and inside
/// none of the volumes mounted now (`mounts`, their mount points). A folder deleted from a volume that is still mounted
/// is not one, and neither is anything outside the media folders.
pub fn on_lost_volume(path: &Path, exists: bool, mounts: &[PathBuf]) -> bool {
    !exists && under_media(path) && !mounts.iter().any(|m| path.starts_with(m))
}

/// The removable volumes mounted now, read from `/proc/mounts` (none when it cannot be read).
pub fn mounted() -> Vec<Volume> {
    std::fs::read_to_string("/proc/mounts").map(|text| parse_mounts(&text)).unwrap_or_default()
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

/// The sentences that name the programs holding a volume (`In use by a, b. Close them and try again.`), each program
/// once; `None` without any.
pub fn in_use_line(blockers: &[Blocker]) -> Option<String> {
    let mut names: Vec<&str> = Vec::new();
    for b in blockers {
        if !names.contains(&b.name.as_str()) {
            names.push(&b.name);
        }
    }
    if names.is_empty() {
        return None;
    }
    let shown: Vec<&str> = names.iter().take(NOTICE_NAMES).copied().collect();
    let mut list = shown.join(", ");
    if names.len() > shown.len() {
        list.push_str(&format!(" and {} more", names.len() - shown.len()));
    }
    let them = if names.len() == 1 { "it" } else { "them" };
    Some(format!("In use by {list}. Close {them} and try again."))
}

/// The line shown when `volume` could not be ejected. With `blockers` it names the programs to close; without any it
/// gives the first line of the tool's `error`, or just the failure when the tool said nothing.
pub fn eject_notice(volume: &str, error: &str, blockers: &[Blocker]) -> String {
    if let Some(line) = in_use_line(blockers) {
        return format!("\u{201c}{volume}\u{201d} could not be ejected. {line}");
    }
    match error.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) => format!("\u{201c}{volume}\u{201d} could not be ejected: {line}"),
        None => format!("\u{201c}{volume}\u{201d} could not be ejected."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_of_an_ejected_volume_is_lost_and_others_are_not() {
        let mounts = vec![PathBuf::from("/run/media/me/Stick")];
        let lost = |p: &str, exists: bool| on_lost_volume(Path::new(p), exists, &mounts);
        assert!(lost("/run/media/me/TESTVOL", false), "the mount point of a volume that is gone");
        assert!(lost("/run/media/me/TESTVOL/photos/2024", false), "a folder inside it");
        assert!(lost("/media/Card", false));
        assert!(!lost("/run/media/me/Stick/gone", false), "deleted from a volume that is still mounted");
        assert!(!lost("/run/media/me", true), "the folder the volumes are mounted in is still there");
        assert!(!lost("/home/me/Documents/old", false), "a removed folder outside the media folders");
        assert!(!lost("/run/mediax/me/V", false), "only whole path components match");
        assert!(under_media(Path::new("/media/Card/a")) && !under_media(Path::new("/home/me/media")));
    }

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
        assert_eq!(in_use_line(&[blocker(1, "vlc"), blocker(2, "vlc")]).as_deref(), Some("In use by vlc. Close it and try again."));
        assert_eq!(in_use_line(&[]), None);
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
}
