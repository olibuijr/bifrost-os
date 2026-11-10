//! Loop devices for disk images: `disk.loop_attach` and `disk.loop_detach`. An unprivileged session cannot run
//! `losetup` (the kernel refuses it without root), so privd does the two steps and the caller mounts the device.
//! The device is bound with partition scanning on, so an image that holds a partition table (an installer image, a
//! memory-card dump) shows its partitions as `/dev/loopNpM`; an image without one is unchanged.
//!
//! The image must be what root can trust without following anything the user could redirect: an absolute path ending
//! in `.iso` or `.img`, reached with no symbolic link, a regular file owned by the installed account, under that
//! account's home directory. Detach only takes loop devices whose backing file is such an image.

use super::{account_ids, obj, s, PErr};
use bifrost_core::json::Json;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

/// File name endings that may be attached (any case).
const EXTENSIONS: [&str; 2] = ["iso", "img"];
/// Where the kernel reports a loop device's backing file.
const SYS_BLOCK: &str = "/sys/block";
/// Where the kernel hands out free loop devices.
const LOOP_CONTROL: &str = "/dev/loop-control";
/// Opens a handle on the file itself and nothing more: no read, no write, no side effect on a device or a pipe.
const O_PATH: i32 = 0o10000000;
/// Binds the file of the descriptor given as argument to the loop device.
const LOOP_SET_FD: std::os::raw::c_ulong = 0x4C00;
/// Returns the number of a free loop device (on the control device).
const LOOP_CTL_GET_FREE: std::os::raw::c_ulong = 0x4C82;
/// Sets the status of a bound loop device from a [`LoopInfo64`].
const LOOP_SET_STATUS64: std::os::raw::c_ulong = 0x4C04;
/// The status flag that makes the kernel read the image's partition table.
const LO_FLAGS_PARTSCAN: u32 = 8;
const EBUSY: i32 = 16;
/// How often a loop device that another process took first is given up for the next free one.
const BIND_TRIES: usize = 8;

extern "C" {
    fn ioctl(fd: i32, request: std::os::raw::c_ulong, ...) -> i32;
}

/// The kernel's `struct loop_info64` (232 bytes), the argument of `LOOP_SET_STATUS64`.
#[repr(C)]
#[allow(dead_code)] // the kernel reads the fields
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

/// The status that turns partition scanning on and changes nothing else: no offset, no size limit, and the kernel
/// keeps the read-only flag the bind gave the device (it is not one this request can clear).
fn partscan_status() -> LoopInfo64 {
    LoopInfo64 {
        lo_device: 0,
        lo_inode: 0,
        lo_rdevice: 0,
        lo_offset: 0,
        lo_sizelimit: 0,
        lo_number: 0,
        lo_encrypt_type: 0,
        lo_encrypt_key_size: 0,
        lo_flags: LO_FLAGS_PARTSCAN,
        lo_file_name: [0; 64],
        lo_crypt_name: [0; 64],
        lo_encrypt_key: [0; 32],
        lo_init: [0; 2],
    }
}

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

/// Checks the image file as root must see it: named as an image, reached with no link anywhere on the path, a regular
/// file, owned by `uid` and under `home`. The file is opened once, without following a link, and every check reads the
/// open descriptor, so what was checked is what gets attached whatever happens to the name afterwards. Returns that
/// descriptor (a handle only, see [`reopen`]) and the path.
fn check_file(path: &str, uid: u32, home: &Path) -> Result<(File, PathBuf), PErr> {
    if !image_name_ok(path) {
        return Err(PErr::bad("the path must be an absolute .iso or .img file"));
    }
    let given = Path::new(path);
    let handle = OpenOptions::new().read(true).custom_flags(O_PATH | super::methods::O_NOFOLLOW).open(given).map_err(|_| PErr::bad("no such image file"))?;
    let meta = handle.metadata().map_err(|_| PErr::bad("no such image file"))?;
    if meta.file_type().is_symlink() {
        return Err(PErr::bad("the image path must not go through a symbolic link"));
    }
    if !meta.file_type().is_file() {
        return Err(PErr::bad("the image is not a regular file"));
    }
    if meta.uid() != uid {
        return Err(PErr::bad("the image is not owned by the account"));
    }
    // where the kernel says the open file is: a link in a parent folder, or a rename since the open, shows here
    let real = std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd())).map_err(|_| PErr::bad("no such image file"))?;
    if real != given {
        return Err(PErr::bad("the image path must not go through a symbolic link"));
    }
    if !real.starts_with(home) {
        return Err(PErr::bad("the image is outside the account's home folder"));
    }
    Ok((handle, real))
}

/// Opens the checked file for the loop device. It goes through the descriptor, not the name, so it is the same file
/// even when the name now leads elsewhere.
fn reopen(handle: &File, read_only: bool) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(!read_only).open(format!("/proc/self/fd/{}", handle.as_raw_fd()))
}

/// Binds the open `image` to a free loop device and returns the device path. The kernel makes the device read-only
/// when the image was opened read-only. `losetup` is not used here: it takes a name and opens it again.
fn bind_free(image: &File, read_only: bool) -> Result<String, PErr> {
    let control = OpenOptions::new().read(true).write(true).open(LOOP_CONTROL).map_err(|e| PErr::internal(format!("{LOOP_CONTROL}: {e}")))?;
    for _ in 0..BIND_TRIES {
        // SAFETY: `control` is an open descriptor and this request takes no argument.
        let n = unsafe { ioctl(control.as_raw_fd(), LOOP_CTL_GET_FREE) };
        if n < 0 {
            return Err(PErr::internal(format!("no free loop device: {}", std::io::Error::last_os_error())));
        }
        let device = format!("/dev/loop{n}");
        let dev = OpenOptions::new().read(true).write(!read_only).open(&device).map_err(|e| PErr::internal(format!("{device}: {e}")))?;
        // SAFETY: both descriptors are open for the length of the call; the request takes the image's descriptor.
        if unsafe { ioctl(dev.as_raw_fd(), LOOP_SET_FD, image.as_raw_fd() as std::os::raw::c_ulong) } == 0 {
            // Read the partition table. A kernel that refuses leaves the device bound as before: an image that is one
            // file system still opens, and a partitioned one is reported as holding nothing that can be opened.
            let status = partscan_status();
            // SAFETY: `dev` is open and `status` is a whole `loop_info64` that lives for the length of the call.
            if unsafe { ioctl(dev.as_raw_fd(), LOOP_SET_STATUS64, &status as *const LoopInfo64) } != 0 {
                eprintln!("privd: {device}: partition scan not turned on: {}", std::io::Error::last_os_error());
            }
            return Ok(device);
        }
        let e = std::io::Error::last_os_error();
        // another process bound this device between the two calls: ask for the next free one
        if e.raw_os_error() != Some(EBUSY) {
            return Err(PErr::internal(format!("{device}: {e}")));
        }
    }
    Err(PErr::internal("no free loop device"))
}

/// The last non-empty line of a command's stderr, or a fixed word when it printed nothing.
fn last_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).unwrap_or_else(|| "failed".into())
}

/// `disk.loop_attach {path, read_only}`: binds the image to a free loop device and returns `{device}`.
pub fn attach(path: &str, read_only: bool) -> Result<Json, PErr> {
    let (uid, _gid, home) = account_ids().ok_or_else(|| PErr::internal("no installed account"))?;
    let (handle, _real) = check_file(path, uid, Path::new(&home))?;
    let image = reopen(&handle, read_only).map_err(|e| PErr::internal(format!("the image cannot be opened: {e}")))?;
    let device = bind_free(&image, read_only)?;
    if !valid_device(&device) {
        return Err(PErr::internal("the kernel gave no loop device"));
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
    fn the_partition_scan_status_has_the_kernel_layout_and_sets_only_that_flag() {
        assert_eq!(std::mem::size_of::<LoopInfo64>(), 232);
        let status = partscan_status();
        assert_eq!(status.lo_flags, LO_FLAGS_PARTSCAN);
        assert_eq!((status.lo_offset, status.lo_sizelimit, status.lo_encrypt_type), (0, 0, 0));
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
        assert_eq!(check_file(file.to_str().unwrap(), uid_of(&home), &home).map(|(_, real)| real), Ok(file.clone()));
    }

    #[test]
    fn the_checked_file_is_the_one_attached_after_its_name_is_swapped_for_a_link() {
        let base = scratch("swap");
        let home = base.join("home");
        let file = home.join("disk.img");
        std::fs::write(&file, b"image").unwrap();
        let secret = base.join("secret.img");
        std::fs::write(&secret, b"secret").unwrap();
        let (handle, _) = check_file(file.to_str().unwrap(), uid_of(&home), &home).unwrap();
        // after the check, the name is made to lead to a file outside the home folder
        std::fs::rename(&file, home.join("moved.img")).unwrap();
        std::os::unix::fs::symlink(&secret, &file).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut reopen(&handle, true).unwrap(), &mut text).unwrap();
        assert_eq!(text, "image");
    }

    #[test]
    fn a_link_in_a_parent_folder_is_refused() {
        let base = scratch("parent");
        let home = base.join("home");
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        std::fs::write(base.join("elsewhere").join("disk.img"), b"x").unwrap();
        std::os::unix::fs::symlink(base.join("elsewhere"), home.join("folder")).unwrap();
        assert!(check_file(home.join("folder").join("disk.img").to_str().unwrap(), uid_of(&home), &home).is_err());
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
