//! Loop devices for disk images: `disk.loop_attach` and `disk.loop_detach`. An unprivileged session cannot run
//! `losetup` (the kernel refuses it without root), so privd does the two steps and the caller mounts the device.
//!
//! The image must be what root can trust without following anything the user could redirect: an absolute path ending
//! in `.iso` or `.img`, reached with no symbolic link, a regular file owned by the installed account, under that
//! account's home directory. Detach only takes loop devices whose backing file is such an image.

use super::{account_ids, obj, s, PErr};
use bifrost_core::json::Json;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// File name endings that may be attached (any case).
const EXTENSIONS: [&str; 2] = ["iso", "img"];
/// Where the kernel reports a loop device's backing file.
const SYS_BLOCK: &str = "/sys/block";

/// A loop device name: `/dev/loop` and then digits.
pub fn valid_device(dev: &str) -> bool {
    dev.strip_prefix("/dev/loop").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `path` is an absolute path ending in `.iso` or `.img` with no NUL byte.
pub fn image_name_ok(path: &str) -> bool {
    if !path.starts_with('/') || path.contains('\0') || path.len() > 4096 {
        return false;
    }
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Checks the image file as root must see it: named as an image, canonical (no link anywhere on the path), a regular
/// file, owned by `uid` and under `home`. Returns the canonical path to hand to `losetup`.
fn check_file(path: &str, uid: u32, home: &Path) -> Result<PathBuf, PErr> {
    if !image_name_ok(path) {
        return Err(PErr::bad("the path must be an absolute .iso or .img file"));
    }
    let given = Path::new(path);
    let canon = given.canonicalize().map_err(|_| PErr::bad("no such image file"))?;
    if canon != given {
        return Err(PErr::bad("the image path must not go through a symbolic link"));
    }
    let meta = std::fs::symlink_metadata(&canon).map_err(|_| PErr::bad("no such image file"))?;
    if !meta.file_type().is_file() {
        return Err(PErr::bad("the image is not a regular file"));
    }
    if meta.uid() != uid {
        return Err(PErr::bad("the image is not owned by the account"));
    }
    if !canon.starts_with(home) {
        return Err(PErr::bad("the image is outside the account's home folder"));
    }
    Ok(canon)
}

/// The last non-empty line of a command's stderr, or a fixed word when it printed nothing.
fn last_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).unwrap_or_else(|| "failed".into())
}

/// `disk.loop_attach {path, read_only}`: binds the image to a free loop device and returns `{device}`.
pub fn attach(path: &str, read_only: bool) -> Result<Json, PErr> {
    let (uid, _gid, home) = account_ids().ok_or_else(|| PErr::internal("no installed account"))?;
    let file = check_file(path, uid, Path::new(&home))?;
    let mut cmd = Command::new("losetup");
    cmd.arg("--find").arg("--show");
    if read_only {
        cmd.arg("--read-only");
    }
    cmd.arg(&file);
    let out = cmd.output().map_err(|e| PErr::internal(format!("losetup: {e}")))?;
    if !out.status.success() {
        return Err(PErr::internal(last_line(&out.stderr)));
    }
    let device = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !valid_device(&device) {
        return Err(PErr::internal("losetup gave no loop device"));
    }
    Ok(obj(vec![("device", s(device))]))
}

/// The backing file of a loop device as the kernel reports it, without the trailing newline.
fn backing_file_in(sys_block: &Path, device: &str) -> Option<String> {
    let name = device.strip_prefix("/dev/")?;
    let text = std::fs::read_to_string(sys_block.join(name).join("loop").join("backing_file")).ok()?;
    Some(text.trim_end_matches('\n').to_string())
}

/// `disk.loop_detach {device}`: releases a loop device that holds one of the account's images.
pub fn detach(device: &str) -> Result<Json, PErr> {
    detach_in(device, Path::new(SYS_BLOCK))
}

fn detach_in(device: &str, sys_block: &Path) -> Result<Json, PErr> {
    if !valid_device(device) {
        return Err(PErr::bad("the device must be /dev/loopN"));
    }
    let (_uid, _gid, home) = account_ids().ok_or_else(|| PErr::internal("no installed account"))?;
    let backing = backing_file_in(sys_block, device).ok_or_else(|| PErr::bad("no such loop device"))?;
    if !image_name_ok(&backing) || !Path::new(&backing).starts_with(&home) {
        return Err(PErr::bad("the loop device does not hold one of the account's images"));
    }
    let out = Command::new("losetup").arg("-d").arg(device).output().map_err(|e| PErr::internal(format!("losetup: {e}")))?;
    if !out.status.success() {
        return Err(PErr::internal(last_line(&out.stderr)));
    }
    Ok(obj(vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp directory, canonical, with a sub-folder named `home`.
    fn scratch(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("privd-loop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("home")).unwrap();
        base.canonicalize().unwrap()
    }

    /// The owner of a folder this test created, so the owner check sees the account it expects.
    fn uid_of(folder: &Path) -> u32 {
        std::fs::metadata(folder).map(|m| m.uid()).unwrap_or(0)
    }

    #[test]
    fn device_names_are_loop_numbers_only() {
        assert!(valid_device("/dev/loop0"));
        assert!(valid_device("/dev/loop123"));
        for bad in ["/dev/loop", "/dev/loop0x", "/dev/sda", "loop0", "/dev/loop0/../sda", "/dev/loop9999999"] {
            assert!(!valid_device(bad), "{bad}");
        }
    }

    #[test]
    fn only_absolute_iso_and_img_names_pass() {
        assert!(image_name_ok("/home/me/Disk.ISO"));
        assert!(image_name_ok("/home/me/a.img"));
        for bad in ["relative.iso", "/home/me/a.txt", "/home/me/a.iso\0x", "/home/me/noext"] {
            assert!(!image_name_ok(bad), "{bad}");
        }
    }

    #[test]
    fn an_image_in_the_home_folder_is_accepted() {
        let base = scratch("ok");
        let home = base.join("home");
        let file = home.join("disk.iso");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(check_file(file.to_str().unwrap(), uid_of(&home), &home), Ok(file.clone()));
    }

    #[test]
    fn a_symbolic_link_or_a_file_outside_home_is_refused() {
        let base = scratch("link");
        let home = base.join("home");
        let real = home.join("real.iso");
        std::fs::write(&real, b"x").unwrap();
        let link = home.join("link.iso");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(check_file(link.to_str().unwrap(), uid_of(&home), &home).is_err());

        let outside = base.join("outside.iso");
        std::fs::write(&outside, b"x").unwrap();
        assert!(check_file(outside.to_str().unwrap(), uid_of(&home), &home).is_err());
    }

    #[test]
    fn a_folder_named_like_an_image_is_refused() {
        let base = scratch("dir");
        let home = base.join("home");
        let dir = home.join("folder.iso");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_file(dir.to_str().unwrap(), uid_of(&home), &home).is_err());
    }

    #[test]
    fn detach_refuses_a_device_that_is_not_an_account_image_before_running_anything() {
        let base = scratch("detach");
        let sys = base.join("sys");
        std::fs::create_dir_all(sys.join("loop0").join("loop")).unwrap();
        std::fs::write(sys.join("loop0").join("loop").join("backing_file"), "/usr/share/other.iso\n").unwrap();
        // Without an installed account the call stops before losetup, so this only checks the refusal path.
        assert!(detach_in("/dev/sda", &sys).is_err());
        assert_eq!(backing_file_in(&sys, "/dev/loop0").as_deref(), Some("/usr/share/other.iso"));
        assert_eq!(backing_file_in(&sys, "/dev/loop7"), None);
    }

    #[test]
    fn last_line_keeps_the_final_message() {
        assert_eq!(last_line(b"warning\nlosetup: /x: failed to set up loop device\n\n"), "losetup: /x: failed to set up loop device");
        assert_eq!(last_line(b""), "failed");
    }
}
