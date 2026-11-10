//! Disk images on the desktop. Attaching binds an image in the account's home folder to a loop device through privd,
//! then mounts that device where the account can read it. Detaching unmounts the device and releases the loop device.
//! The control socket's `image-attach` and `image-detach` lines and the `hdiutil` tool come here. A sandboxed app
//! (Files) has no control socket; it calls the `disks` service (`attach_image`, `detach_image`), which runs the work
//! on a thread ([`Jobs`]) and answers with an `image_attached` or `image_detached` event.
//!
//! Who mounts depends on who the shell runs as. A shell running as the account asks the storage service
//! (`udisksctl`), which mounts under `/run/media/<account>`. A shell running as root would get a mount under root's
//! own media folder, which the account cannot open, so it mounts the volume itself under `/run/media/<account>`,
//! owned by the account where the file system keeps no owners of its own.

use crate::bare::account::Account;
use crate::privd_client;
use bifrost_core::json::Json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// The `disks` service methods and events for disk images (`services/disks.rs` hands them to [`Jobs`]).
pub const M_ATTACH: &str = "attach_image";
pub const M_DETACH: &str = "detach_image";
pub const E_ATTACHED: &str = "image_attached";
pub const E_DETACHED: &str = "image_detached";

/// Where removable volumes are mounted, one folder per account.
const MEDIA_ROOT: &str = "/run/media";
/// File systems that store no owner per file: the mount names the account as the owner of everything.
const OWNERLESS: [&str; 9] = ["vfat", "msdos", "exfat", "ntfs", "ntfs3", "iso9660", "udf", "hfs", "hfsplus"];
/// File systems with their own owners that a root shell also mounts.
const OWNED: [&str; 6] = ["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs"];
/// How often the storage service is asked again while it has not seen the new device yet, and the wait between.
const PROBE_TRIES: u32 = 10;
const PROBE_WAIT: Duration = Duration::from_millis(300);
/// The most attach and detach requests running at once.
const MAX_JOBS: usize = 4;

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
    match mount(&device, path, read_only) {
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
    if let Some(point) = mounted_point_of(&mounts, &device) {
        unmount(&device, &point)?;
    }
    privd_client::connect().loop_detach(&device).map_err(|e| e.message)?;
    Ok(device)
}

fn shell_is_root() -> bool {
    crate::privd::server::euid() == 0
}

/// Mounts a loop device where the account can read it and returns the folder.
fn mount(device: &str, image: &str, read_only: bool) -> Result<String, String> {
    if shell_is_root() {
        let account = Account::installed().map_err(|e| e.to_string())?;
        return mount_for_account(device, image, read_only, &account);
    }
    // udev reads a new loop device after the kernel announces it and the storage service learns of it from udev: a
    // mount asked for before that is refused as "not a mountable filesystem". Wait for udev's queue, then ask again
    // a bounded number of times while the service still answers that way.
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).output();
    let out = retry_until_probed(PROBE_TRIES, PROBE_WAIT, || run("udisksctl", &["mount", "--no-user-interaction", "-b", device]))?;
    mounted_at(&out).ok_or_else(|| format!("the volume on {device} did not mount"))
}

/// True for the storage service's answers that mean it has not read the device yet (the device is new).
pub fn not_probed_yet(error: &str) -> bool {
    error.contains("is not a mountable filesystem") || error.contains("Error looking up object for device")
}

/// Runs `mount` up to `tries` times, sleeping `wait` after each answer that says the device is not read yet. Any
/// other answer, and the last one, is returned as it is.
pub fn retry_until_probed(tries: u32, wait: Duration, mut mount: impl FnMut() -> Result<String, String>) -> Result<String, String> {
    let mut left = tries.max(1);
    loop {
        match mount() {
            Err(e) if left > 1 && not_probed_yet(&e) => {
                left -= 1;
                std::thread::sleep(wait);
            }
            other => return other,
        }
    }
}

/// A root shell's mount: under the account's media folder, never root's. The file system type and label are read
/// from the device itself (`blkid -p`), so nothing waits for udev here.
fn mount_for_account(device: &str, image: &str, read_only: bool, account: &Account) -> Result<String, String> {
    let probe = |tag: &str| run("blkid", &["-p", "-o", "value", "-s", tag, "--", device]).map(|t| t.trim().to_string()).unwrap_or_default();
    let fs = probe("TYPE");
    if !OWNERLESS.contains(&fs.as_str()) && !OWNED.contains(&fs.as_str()) {
        return Err("it holds no file system that can be opened".to_string());
    }
    let root = Path::new(MEDIA_ROOT).join(&account.name);
    make_media_root(&root, account.gid)?;
    let point = fresh_folder(&root, &volume_name(&probe("LABEL"), image))?;
    let folder = point.to_string_lossy().into_owned();
    match run("mount", &["-o", mount_options(&fs, read_only, account.uid, account.gid).as_str(), "--", device, folder.as_str()]) {
        Ok(_) => Ok(folder),
        Err(e) => {
            let _ = std::fs::remove_dir(&point);
            Err(e)
        }
    }
}

/// The account's media folder: root's, readable by the account's group only (the storage service makes it the same
/// way). One that exists is left as it is.
fn make_media_root(root: &Path, gid: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if root.is_dir() {
        return Ok(());
    }
    let failed = |e: std::io::Error| format!("{} could not be made: {e}", root.display());
    std::fs::create_dir_all(root).map_err(failed)?;
    std::os::unix::fs::chown(root, Some(0), Some(gid)).map_err(failed)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o750)).map_err(failed)
}

/// A new empty folder for the volume: `name`, or `name 2`, `name 3`… while the name is taken. An empty folder left
/// behind by an earlier mount is used again (removing a folder something is mounted on fails, so that one is kept).
fn fresh_folder(root: &Path, name: &str) -> Result<PathBuf, String> {
    for n in 1..=32 {
        let point = root.join(if n == 1 { name.to_string() } else { format!("{name} {n}") });
        match std::fs::create_dir(&point) {
            Ok(()) => return Ok(point),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::remove_dir(&point).is_ok() && std::fs::create_dir(&point).is_ok() {
                    return Ok(point);
                }
            }
            Err(e) => return Err(format!("{} could not be made: {e}", point.display())),
        }
    }
    Err(format!("too many volumes named {name} are open"))
}

/// The volume's folder name: its label, else the image file's name without the extension, without `/`, control
/// characters or leading dots, at most 64 characters.
pub fn volume_name(label: &str, image: &str) -> String {
    let clean = |text: &str| {
        let kept: String = text.chars().filter(|c| !c.is_control() && *c != '/').collect();
        kept.trim().trim_start_matches('.').trim().chars().take(64).collect::<String>()
    };
    let stem = Path::new(image).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    [clean(label), clean(&stem)].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "Disk Image".to_string())
}

/// The options of a root shell's mount: never set-uid programs or device nodes from an image, read-only when asked,
/// and the account as owner where the file system `fs` has no owners of its own.
pub fn mount_options(fs: &str, read_only: bool, uid: u32, gid: u32) -> String {
    let mut options = vec!["nosuid".to_string(), "nodev".to_string()];
    if read_only {
        options.push("ro".to_string());
    }
    if OWNERLESS.contains(&fs) {
        options.push(format!("uid={uid}"));
        options.push(format!("gid={gid}"));
    }
    options.join(",")
}

/// Unmounts `device` from `point`. A root shell unmounts only what is under the media folders, so a loop device the
/// system itself uses is never pulled away.
fn unmount(device: &str, point: &str) -> Result<(), String> {
    if !shell_is_root() {
        return run("udisksctl", &["unmount", "--no-user-interaction", "-b", device]).map(|_| ());
    }
    if !Path::new(point).starts_with(MEDIA_ROOT) {
        return Err(format!("{device} is in use by the system"));
    }
    run("umount", &["--", point])?;
    let _ = std::fs::remove_dir(point);
    Ok(())
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program} could not run: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("{program} failed") } else { err })
}

/// One attach or detach running on a thread for an app.
struct Job {
    app: String,
    event: &'static str,
    /// the body's key for what was asked (`path` or `target`) and its value
    subject: (&'static str, String),
    done: Receiver<Result<Vec<(&'static str, String)>, String>>,
}

/// The attach and detach requests of apps. Mounting waits for the kernel and the storage service, so each request
/// runs on a thread; the `disks` service collects the finished ones on its tick and sends each app its event.
#[derive(Default)]
pub struct Jobs {
    running: Vec<Job>,
}

impl Jobs {
    /// Whether `method` is one of the disk image methods.
    pub fn handles(method: &str) -> bool {
        method == M_ATTACH || method == M_DETACH
    }

    /// Starts `attach_image {path, read_only?}` (read-only unless `read_only` is false) or `detach_image {target}`
    /// for `app` and answers `{started: true}`. The outcome follows as an event ([`Jobs::finished`]).
    pub fn start(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        if self.running.len() >= MAX_JOBS {
            return Err("too many disk images are being opened; try again in a moment".to_string());
        }
        let (tx, done) = channel();
        let (event, subject) = match method {
            M_ATTACH => {
                let path = params.str_field("path").filter(|p| p.starts_with('/')).ok_or("the image's full path is needed")?;
                let read_only = !matches!(params.get("read_only"), Some(Json::Bool(false)));
                let image = path.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(attach(&image, read_only).map(|a| vec![("device", a.device), ("mount_point", a.mount_point)]));
                });
                (E_ATTACHED, ("path", path))
            }
            M_DETACH => {
                let target = params.str_field("target").filter(|p| p.starts_with('/')).ok_or("the volume's device or folder is needed")?;
                let what = target.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(detach(&what).map(|device| vec![("device", device)]));
                });
                (E_DETACHED, ("target", target))
            }
            _ => return Err(format!("disks has no method {method}")),
        };
        self.running.push(Job { app: app.to_string(), event, subject, done });
        Ok(Json::Obj(vec![("started".to_string(), Json::Bool(true))]))
    }

    /// The requests that ended since the last call: the app that asked, the event name and the event body
    /// ([`outcome`]). A request whose thread died is reported as failed, never dropped.
    pub fn finished(&mut self) -> Vec<(String, &'static str, Json)> {
        let mut out = Vec::new();
        self.running.retain(|job| {
            let result = match job.done.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => Err("the request stopped before it finished".to_string()),
            };
            out.push((job.app.clone(), job.event, outcome(job.subject.0, &job.subject.1, result)));
            false
        });
        out
    }
}

/// An event body: what was asked under `key`, `ok`, and then the result's fields (`device`, `mount_point`) or `error`.
pub fn outcome(key: &str, subject: &str, result: Result<Vec<(&'static str, String)>, String>) -> Json {
    let mut fields = vec![(key.to_string(), Json::Str(subject.to_string())), ("ok".to_string(), Json::Bool(result.is_ok()))];
    match result {
        Ok(more) => fields.extend(more.into_iter().map(|(k, v)| (k.to_string(), Json::Str(v)))),
        Err(e) => fields.push(("error".to_string(), Json::Str(e))),
    }
    Json::Obj(fields)
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

    #[test]
    fn a_device_the_storage_service_has_not_read_yet_is_asked_for_again() {
        let race = "Object /org/freedesktop/UDisks2/block_devices/loop0 is not a mountable filesystem.";
        assert!(not_probed_yet(race));
        assert!(not_probed_yet("Error looking up object for device /dev/loop0"));
        assert!(!not_probed_yet("Error mounting /dev/loop0: wrong fs type"));
        let mut calls = 0;
        let out = retry_until_probed(5, Duration::ZERO, || {
            calls += 1;
            if calls < 3 { Err(race.to_string()) } else { Ok("Mounted /dev/loop0 at /run/media/me/T.".to_string()) }
        });
        assert_eq!((calls, out.as_deref().ok().and_then(mounted_at).as_deref()), (3, Some("/run/media/me/T")));
        // bounded: a device that never becomes readable gives the service's answer after the last try
        let mut calls = 0;
        let out = retry_until_probed(4, Duration::ZERO, || {
            calls += 1;
            Err(race.to_string())
        });
        assert_eq!((calls, out), (4, Err(race.to_string())));
        // any other failure is final at once
        let mut calls = 0;
        let out = retry_until_probed(4, Duration::ZERO, || {
            calls += 1;
            Err("Not authorized to perform operation".to_string())
        });
        assert_eq!((calls, out.is_err()), (1, true));
    }

    #[test]
    fn a_root_shell_mounts_for_the_account_and_never_trusts_the_image() {
        assert_eq!(mount_options("vfat", true, 1000, 1001), "nosuid,nodev,ro,uid=1000,gid=1001");
        assert_eq!(mount_options("iso9660", false, 1000, 1000), "nosuid,nodev,uid=1000,gid=1000");
        assert_eq!(mount_options("ext4", false, 1000, 1000), "nosuid,nodev", "ext4 keeps its own owners");
    }

    #[test]
    fn the_volume_folder_is_named_after_the_label_or_the_image() {
        assert_eq!(volume_name("TESTVOL", "/home/me/t.img"), "TESTVOL");
        assert_eq!(volume_name("", "/home/me/My Disk.img"), "My Disk");
        assert_eq!(volume_name(" ../a/b\n", "/home/me/t.img"), "ab");
        assert_eq!(volume_name("/", "/home/me/.img"), "img");
        assert_eq!(volume_name("", ""), "Disk Image");
    }

    #[test]
    fn a_taken_volume_folder_gets_a_number_and_an_empty_one_is_used_again() {
        let root = std::env::temp_dir().join(format!("disk-image-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Disk")).unwrap();
        std::fs::write(root.join("Disk").join("in use"), "").unwrap();
        assert_eq!(fresh_folder(&root, "Disk").unwrap(), root.join("Disk 2"));
        assert_eq!(fresh_folder(&root, "Disk").unwrap(), root.join("Disk 2"), "an empty leftover is taken again");
        assert_eq!(fresh_folder(&root, "Other").unwrap(), root.join("Other"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_event_body_names_what_was_asked_and_how_it_ended() {
        let ok = outcome("path", "/home/me/t.img", Ok(vec![("device", "/dev/loop0".into()), ("mount_point", "/run/media/me/T".into())]));
        assert_eq!(ok.get("ok"), Some(&Json::Bool(true)));
        assert_eq!(ok.str_field("path").as_deref(), Some("/home/me/t.img"));
        assert_eq!(ok.str_field("mount_point").as_deref(), Some("/run/media/me/T"));
        let failed = outcome("target", "/dev/loop0", Err("target is busy".into()));
        assert_eq!(failed.get("ok"), Some(&Json::Bool(false)));
        assert_eq!(failed.str_field("error").as_deref(), Some("target is busy"));
    }

    #[test]
    fn a_request_without_a_full_path_is_refused_and_starts_nothing() {
        let mut jobs = Jobs::default();
        assert!(Jobs::handles(M_ATTACH) && Jobs::handles(M_DETACH) && !Jobs::handles("list"));
        let relative = Json::Obj(vec![("path".to_string(), Json::Str("t.img".into()))]);
        assert!(jobs.start("app", M_ATTACH, &relative).is_err());
        assert!(jobs.start("app", M_DETACH, &Json::Null).is_err());
        assert!(jobs.running.is_empty() && jobs.finished().is_empty());
    }

    #[test]
    fn a_request_whose_thread_died_is_reported_as_failed() {
        let (tx, done) = channel();
        drop(tx);
        let mut jobs = Jobs { running: vec![Job { app: "app".into(), event: E_ATTACHED, subject: ("path", "/home/me/t.img".into()), done }] };
        let ended = jobs.finished();
        assert_eq!(ended.len(), 1);
        assert_eq!((ended[0].0.as_str(), ended[0].1), ("app", E_ATTACHED));
        assert_eq!(ended[0].2.get("ok"), Some(&Json::Bool(false)));
        assert!(jobs.running.is_empty());
    }
}
