//! Disk images on the desktop. Attaching binds an image in the account's home folder to a loop device through privd,
//! then mounts that device through the storage service (`udisksctl`, the same tool Disk Utility uses). Detaching
//! unmounts the device and releases the loop device. The control socket's `image-attach` and `image-detach` lines and
//! the `hdiutil` tool come here; Files reaches them through `hdiutil`.

use crate::privd_client;
use std::process::Command;

/// What an attach produced: the loop device and the folder the volume is mounted on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub device: String,
    pub mount_point: String,
}

/// True for a loop device path (`/dev/loop` and digits). Only these reach a command.
pub fn is_loop_device(path: &str) -> bool {
    path.strip_prefix("/dev/loop").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Attaches the image at `path`: a loop device through privd, then a mount. A mount that fails gives the loop device
/// back, so a failed attach leaves nothing behind.
pub fn attach(path: &str, read_only: bool) -> Result<Attached, String> {
    let client = privd_client::connect();
    let device = client.loop_attach(path, read_only).map_err(|e| e.message)?;
    match mount(&device) {
        Ok(mount_point) => Ok(Attached { device, mount_point }),
        Err(e) => {
            let _ = client.loop_detach(&device);
            Err(e)
        }
    }
}

/// Unmounts an attached image and releases its loop device. `target` is the loop device or the folder it is mounted on.
/// Returns the device that was released.
pub fn detach(target: &str) -> Result<String, String> {
    let device = if is_loop_device(target) {
        target.to_string()
    } else {
        let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
        mounted_device_at(&mounts, target).ok_or_else(|| format!("{target} is not a mounted disk image"))?
    };
    if !is_loop_device(&device) {
        return Err(format!("{device} is not a disk image device"));
    }
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    if mounted_point_of(&mounts, &device).is_some() {
        run("udisksctl", &["unmount", "-b", device.as_str()])?;
    }
    privd_client::connect().loop_detach(&device).map_err(|e| e.message)?;
    Ok(device)
}

/// Mounts a loop device and returns the folder it is mounted on, read from the storage service's answer.
fn mount(device: &str) -> Result<String, String> {
    let out = run("udisksctl", &["mount", "-b", device])?;
    mounted_at(&out).ok_or_else(|| format!("the volume on {device} did not mount"))
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program} could not run: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("{program} failed") } else { err })
}

/// The folder named in a storage service line such as `Mounted /dev/loop0 at /run/media/me/Disk.`.
pub fn mounted_at(output: &str) -> Option<String> {
    let line = output.lines().find(|l| l.starts_with("Mounted "))?;
    let (_, point) = line.split_once(" at ")?;
    let point = point.trim().trim_end_matches('.');
    (!point.is_empty()).then(|| point.to_string())
}

/// The mount point of `device` in a mount table (`/proc/mounts`), with the table's octal escapes undone.
pub fn mounted_point_of(mounts: &str, device: &str) -> Option<String> {
    mounts.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        if f.next()? != device {
            return None;
        }
        Some(unescape(f.next()?))
    })
}

/// The loop device mounted on `point` in a mount table.
pub fn mounted_device_at(mounts: &str, point: &str) -> Option<String> {
    mounts.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let device = f.next()?;
        (unescape(f.next()?) == point && is_loop_device(device)).then(|| device.to_string())
    })
}

fn unescape(text: &str) -> String {
    text.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loop_device_paths_are_accepted() {
        assert!(is_loop_device("/dev/loop0"));
        assert!(is_loop_device("/dev/loop12"));
        for bad in ["/dev/loop", "/dev/sda1", "/dev/loop0/../sda", "loop0", "/dev/loop1234567"] {
            assert!(!is_loop_device(bad), "{bad}");
        }
    }

    #[test]
    fn the_storage_service_answer_gives_the_folder() {
        assert_eq!(mounted_at("Mounted /dev/loop0 at /run/media/me/Ubuntu 24.04.\n").as_deref(), Some("/run/media/me/Ubuntu 24.04"));
        assert_eq!(mounted_at("Failed to mount"), None);
        assert_eq!(mounted_at("Mounted /dev/loop0 at ."), None);
    }

    #[test]
    fn the_mount_table_gives_the_point_and_the_device() {
        let table = "/dev/sda1 / ext4 rw 0 0\n/dev/loop3 /run/media/me/My\\040Disk iso9660 ro 0 0\n";
        assert_eq!(mounted_point_of(table, "/dev/loop3").as_deref(), Some("/run/media/me/My Disk"));
        assert_eq!(mounted_device_at(table, "/run/media/me/My Disk").as_deref(), Some("/dev/loop3"));
        assert_eq!(mounted_device_at(table, "/"), None, "a partition is not a disk image");
        assert_eq!(mounted_point_of(table, "/dev/loop9"), None);
    }
}
