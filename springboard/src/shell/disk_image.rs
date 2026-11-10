//! Disk images on the desktop. Attaching binds an image in the account's home folder to a loop device through privd,
//! then mounts that device where the account can read it. Detaching unmounts the device and releases the loop device.
//! The control socket's `image-attach` and `image-detach` lines and the `hdiutil` tool come here. A sandboxed app
//! (Files) has no control socket; it calls the `disks` service (`attach_image`, `detach_image`), which runs the work
//! on a thread ([`Jobs`]) and answers with an `image_attached` or `image_detached` event. The same service ejects a
//! removable volume for the app (`eject_volume`, `shell::volume_eject`).
//!
//! An image that is one file system is one volume. An image with a partition table (privd binds every image with
//! partition scanning) gives one volume per partition that holds a file system the system mounts; the others (swap,
//! encrypted, unknown) are passed over. The partitions are the ones the kernel lists for the loop device in `/sys/block`,
//! and what each holds is read from the partition itself. Ejecting any volume of an image, or the image, unmounts all
//! its volumes and then releases the loop device once.
//!
//! Who mounts depends on who the shell runs as. A shell running as the account asks the storage service
//! (`udisksctl`), which mounts under `/run/media/<account>`. A shell running as root would get a mount under root's
//! own media folder, which the account cannot open, so it mounts the volume itself under `/run/media/<account>`,
//! owned by the account where the file system keeps no owners of its own.

use crate::bare::account::Account;
use super::volume_eject::{E_EJECTED, M_EJECT};
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
/// The most attach, detach and eject requests running at once.
const MAX_JOBS: usize = 4;
/// Where the kernel lists a loop device's partitions.
const SYS_BLOCK: &str = "/sys/block";
/// The most volumes one image gives. A partition table can name far more partitions than anyone opens at once.
pub const MAX_VOLUMES: usize = 16;
/// Why an image with nothing to mount is refused.
const NOTHING_TO_OPEN: &str = "it holds no file system that can be opened";

/// What an attach produced: the loop device and the folders its volumes are mounted on. `mount_point` is the first
/// of `volumes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub device: String,
    pub mount_point: String,
    pub volumes: Vec<String>,
}

/// True for a loop device path (`/dev/loop` and digits). Only these and their partitions reach a command.
pub fn is_loop_device(path: &str) -> bool {
    path.strip_prefix("/dev/loop").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The loop device a disk image volume is on: the device itself for `/dev/loopN`, the device of the partition
/// `/dev/loopNpM`, and `None` for every other path (a USB stick's `/dev/sdb1`).
pub fn image_device_of(path: &str) -> Option<String> {
    if is_loop_device(path) {
        return Some(path.to_string());
    }
    let (device, number) = path.rsplit_once('p')?;
    let numbered = !number.is_empty() && number.len() <= 4 && number.bytes().all(|b| b.is_ascii_digit());
    (numbered && is_loop_device(device)).then(|| device.to_string())
}

/// The partitions the kernel lists for the loop device `device` under `sys_block`, as (number, device path), by
/// number. Only folders that carry a `partition` file and are named `<device>p<that number>` count, so every path
/// that comes back is a partition of this device.
pub fn partitions_in(sys_block: &Path, device: &str) -> Vec<(u32, String)> {
    let Some(name) = device.strip_prefix("/dev/").filter(|_| is_loop_device(device)) else { return Vec::new() };
    let mut out: Vec<(u32, String)> = std::fs::read_dir(sys_block.join(name))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let number: u32 = std::fs::read_to_string(entry.path().join("partition")).ok()?.trim().parse().ok()?;
            (entry.file_name().to_str()? == format!("{name}p{number}")).then(|| (number, format!("{device}p{number}")))
        })
        .collect();
    out.sort();
    out
}

/// What a device holds, as far as mounting goes: its file system type and label (both empty when it has none).
/// `partition` is the partition's number, `None` for the whole device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub device: String,
    pub partition: Option<u32>,
    pub fs: String,
    pub label: String,
}

/// Reads `TYPE` and `LABEL` from the `KEY=value` lines of `blkid -p -o export`. That format puts a backslash before
/// spaces, quotes and a few other characters of a value; they are taken off again.
pub fn parse_probe(device: &str, partition: Option<u32>, export: &str) -> Probe {
    let value = |key: &str| {
        let found = export.lines().find_map(|line| line.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v));
        let mut out = String::new();
        let mut chars = found.unwrap_or_default().chars();
        while let Some(c) = chars.next() {
            out.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
        }
        out
    };
    Probe { device: device.to_string(), partition, fs: value("TYPE"), label: value("LABEL") }
}

/// Whether the system mounts the file system type `fs` for a person. Swap, encrypted containers, volume-manager
/// members and an empty or unknown type are not mounted.
pub fn mountable(fs: &str) -> bool {
    OWNERLESS.contains(&fs) || OWNED.contains(&fs)
}

/// One volume to mount: its device, file system type and the name of its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedVolume {
    pub device: String,
    pub fs: String,
    pub name: String,
}

/// The volumes of the image file `image`. A whole device that holds a file system is the one volume, also when it
/// carries a partition table beside it (an installer disc image does), so such an image opens as it always did.
/// Otherwise each partition with a mountable file system is a volume, by partition number, at most [`MAX_VOLUMES`]:
/// named by its label, or by the image's name and the partition's number when it has none. Every other partition is
/// left out. System partitions with a file system (an EFI one is `vfat`) are volumes like the rest.
pub fn plan_volumes(image: &str, whole: &Probe, partitions: &[Probe]) -> Vec<PlannedVolume> {
    if mountable(&whole.fs) {
        return vec![PlannedVolume { device: whole.device.clone(), fs: whole.fs.clone(), name: volume_name(&whole.label, image) }];
    }
    let mut parts: Vec<&Probe> = partitions.iter().filter(|p| p.partition.is_some() && mountable(&p.fs)).collect();
    parts.sort_by_key(|p| p.partition);
    parts
        .into_iter()
        .take(MAX_VOLUMES)
        .map(|p| {
            let label = clean_name(&p.label);
            let name = if label.is_empty() { format!("{} {}", volume_name("", image), p.partition.unwrap_or(0)) } else { label };
            PlannedVolume { device: p.device.clone(), fs: p.fs.clone(), name }
        })
        .collect()
}

/// Attaches the image at `path`: a loop device through privd, then a mount. A mount that fails gives the loop device
/// back, so a failed attach leaves nothing behind.
pub fn attach(path: &str, read_only: bool) -> Result<Attached, String> {
    let client = privd_client::connect();
    let device = client.loop_attach(path, read_only).map_err(|e| e.message)?;
    match mount(&device, path, read_only) {
        Ok(volumes) => Ok(Attached { device, mount_point: volumes.first().cloned().unwrap_or_default(), volumes }),
        Err(e) => {
            let _ = client.loop_detach(&device);
            Err(e)
        }
    }
}

/// Unmounts every volume of an attached image and releases its loop device, once. `target` is the loop device, one
/// of its partitions, or the folder a volume of the image is mounted on. Returns the device that was released.
pub fn detach(target: &str) -> Result<String, String> {
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    let device = detach_device(target, &mounts)?;
    let volumes = teardown(&mounts, &device);
    // a root shell takes nothing away unless every mount of the device is a volume it may unmount
    if shell_is_root() {
        if let Some((part, _)) = volumes.iter().find(|(_, point)| !Path::new(point).starts_with(MEDIA_ROOT)) {
            return Err(format!("{part} is in use by the system"));
        }
    }
    for (part, point) in &volumes {
        unmount(part, point).map_err(|e| {
            eprintln!("disks: detach {device}: unmount of {point} ({part}) failed: {e}");
            e
        })?;
    }
    privd_client::connect().loop_detach(&device).map_err(|e| {
        eprintln!("disks: detach {device}: release of the loop device failed: {}", e.message);
        e.message
    })?;
    Ok(device)
}

/// The loop device a detach of `target` releases: `target`'s own when it is a loop device or a partition of one,
/// else the device of the image volume mounted on the folder `target` in the mount table `mounts`.
pub fn detach_device(target: &str, mounts: &str) -> Result<String, String> {
    if let Some(device) = image_device_of(target) {
        return Ok(device);
    }
    if target.starts_with("/dev/") {
        return Err(format!("{target} is not a disk image device"));
    }
    mounted_device_at(mounts, target).and_then(|d| image_device_of(&d)).ok_or_else(|| format!("{target} is not a mounted disk image"))
}

/// What is unmounted before the loop device `device` is released: every mount of the device and of its partitions
/// in the mount table, as (device, mount point), the last mounted first.
pub fn teardown(mounts: &str, device: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let part = f.next()?;
            (image_device_of(part).as_deref() == Some(device)).then_some(())?;
            Some((part.to_string(), unescape(f.next()?)))
        })
        .collect();
    out.reverse();
    out
}

fn shell_is_root() -> bool {
    crate::privd::server::euid() == 0
}

/// Mounts the volumes of a loop device where the account can read them and returns their folders (at least one).
fn mount(device: &str, image: &str, read_only: bool) -> Result<Vec<String>, String> {
    if shell_is_root() {
        let account = Account::installed().map_err(|e| e.to_string())?;
        return mount_for_account(device, image, read_only, &account);
    }
    // udev reads a new loop device after the kernel announces it and the storage service learns of it from udev: a
    // mount asked for before that is refused as "not a mountable filesystem". Wait for udev's queue, then ask again
    // a bounded number of times while the service still answers that way.
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).output();
    let ask = |part: &str| {
        let out = retry_until_probed(PROBE_TRIES, PROBE_WAIT, || run("udisksctl", &["mount", "--no-user-interaction", "-b", part]))?;
        mounted_at(&out).ok_or_else(|| format!("the volume on {part} did not mount"))
    };
    // the whole device first, as before; when it holds no file system, each partition the kernel lists for it
    let whole = match ask(device) {
        Ok(folder) => return Ok(vec![folder]),
        Err(e) => e,
    };
    let mounted: Vec<String> = partitions_in(Path::new(SYS_BLOCK), device)
        .into_iter()
        .take(MAX_VOLUMES)
        .filter_map(|(_, part)| ask(&part).map_err(|e| eprintln!("disks: attach {device}: {part} not mounted: {e}")).ok())
        .collect();
    if mounted.is_empty() {
        return Err(whole);
    }
    Ok(mounted)
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
/// from each device itself (`blkid -p`), so nothing waits for udev here. A volume that does not mount is passed over
/// while another one does; when none mounts, the first failure is the answer.
fn mount_for_account(device: &str, image: &str, read_only: bool, account: &Account) -> Result<Vec<String>, String> {
    let probe = |part: &str, number: Option<u32>| parse_probe(part, number, &run("blkid", &["-p", "-o", "export", "--", part]).unwrap_or_default());
    let whole = probe(device, None);
    let partitions: Vec<Probe> = if mountable(&whole.fs) { Vec::new() } else { partitions_in(Path::new(SYS_BLOCK), device).into_iter().map(|(n, part)| probe(&part, Some(n))).collect() };
    let plan = plan_volumes(image, &whole, &partitions);
    if plan.is_empty() {
        return Err(NOTHING_TO_OPEN.to_string());
    }
    let root = Path::new(MEDIA_ROOT).join(&account.name);
    make_media_root(&root, account.gid)?;
    let mut mounted = Vec::new();
    let mut failed = None;
    for volume in &plan {
        match mount_volume(volume, &root, read_only, account) {
            Ok(folder) => mounted.push(folder),
            Err(e) => {
                eprintln!("disks: attach {device}: {} ({}) not mounted: {e}", volume.device, volume.fs);
                failed.get_or_insert(e);
            }
        }
    }
    if mounted.is_empty() {
        return Err(failed.unwrap_or_else(|| NOTHING_TO_OPEN.to_string()));
    }
    Ok(mounted)
}

/// Mounts one planned volume on a new folder under `root` and returns the folder.
fn mount_volume(volume: &PlannedVolume, root: &Path, read_only: bool, account: &Account) -> Result<String, String> {
    let point = fresh_folder(root, &volume.name)?;
    let folder = point.to_string_lossy().into_owned();
    match run("mount", &["-o", mount_options(&volume.fs, read_only, account.uid, account.gid).as_str(), "--", volume.device.as_str(), folder.as_str()]) {
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
    let stem = Path::new(image).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    [clean_name(label), clean_name(&stem)].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "Disk Image".to_string())
}

/// `text` as a folder name: without `/`, control characters or leading dots, at most 64 characters. May be empty.
fn clean_name(text: &str) -> String {
    let kept: String = text.chars().filter(|c| !c.is_control() && *c != '/').collect();
    kept.trim().trim_start_matches('.').trim().chars().take(64).collect()
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

pub(crate) fn run(program: &str, args: &[&str]) -> Result<String, String> {
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
    /// Whether `method` is one of the disk image methods or the eject of a removable volume.
    pub fn handles(method: &str) -> bool {
        method == M_ATTACH || method == M_DETACH || method == M_EJECT
    }

    /// Starts `attach_image {path, read_only?}` (read-only unless `read_only` is false), `detach_image {target}` or
    /// `eject_volume {target}` (the partition of a removable volume, `shell::volume_eject`) for `app` and answers
    /// `{started: true}`. The outcome follows as an event ([`Jobs::finished`]).
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
            M_EJECT => {
                let target = params.str_field("target").filter(|p| super::volume_eject::valid_device(p)).ok_or("the volume's device is needed")?;
                let what = target.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(super::volume_eject::eject(&what));
                });
                (E_EJECTED, ("target", target))
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

/// The loop device, or the partition of one, mounted on `point` in a mount table.
pub fn mounted_device_at(mounts: &str, point: &str) -> Option<String> {
    mounts.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let device = f.next()?;
        (unescape(f.next()?) == point && image_device_of(device).is_some()).then(|| device.to_string())
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
    fn the_mount_table_gives_the_device_of_a_volume_folder() {
        let table = "/dev/sda1 / ext4 rw 0 0\n/dev/loop3 /run/media/me/My\\040Disk iso9660 ro 0 0\n/dev/loop4p2 /run/media/me/Data ext4 ro 0 0\n";
        assert_eq!(mounted_device_at(table, "/run/media/me/My Disk").as_deref(), Some("/dev/loop3"));
        assert_eq!(mounted_device_at(table, "/run/media/me/Data").as_deref(), Some("/dev/loop4p2"));
        assert_eq!(mounted_device_at(table, "/"), None, "a disk's partition is not a disk image");
    }

    #[test]
    fn a_volume_of_an_image_is_told_from_any_other_device() {
        assert_eq!(image_device_of("/dev/loop0").as_deref(), Some("/dev/loop0"));
        assert_eq!(image_device_of("/dev/loop12p3").as_deref(), Some("/dev/loop12"));
        for other in ["/dev/sdb1", "/dev/nvme0n1p2", "/dev/mmcblk0p1", "/dev/loop0p", "/dev/loopp1", "/dev/loop0p1x", "/dev/loop0p1/../sda", "/dev/loop0p12345", "loop0p1"] {
            assert_eq!(image_device_of(other), None, "{other}");
        }
    }

    #[test]
    fn the_partitions_come_from_the_kernels_list_for_the_device() {
        let sys = std::env::temp_dir().join(format!("disk-image-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sys);
        let part = |disk: &str, folder: &str, number: Option<&str>| {
            std::fs::create_dir_all(sys.join(disk).join(folder)).unwrap();
            if let Some(n) = number {
                std::fs::write(sys.join(disk).join(folder).join("partition"), format!("{n}\n")).unwrap();
            }
        };
        part("loop0", "loop0p2", Some("2"));
        part("loop0", "loop0p10", Some("10"));
        part("loop0", "loop0p1", Some("1"));
        part("loop0", "queue", None);
        part("loop0", "holders", None);
        part("loop0", "sda1", Some("1"));
        part("loop0", "loop0p7", Some("3"));
        part("loop1", "loop1p1", Some("1"));
        let p = |n: u32| (n, format!("/dev/loop0p{n}"));
        assert_eq!(partitions_in(&sys, "/dev/loop0"), vec![p(1), p(2), p(10)], "by number; a folder whose name and number disagree is not one");
        assert!(partitions_in(&sys, "/dev/loop5").is_empty(), "no such device");
        assert!(partitions_in(&sys, "/dev/sda").is_empty(), "only a loop device is looked at");
        assert!(partitions_in(&sys, "/dev/loop0/../loop1").is_empty());
        std::fs::remove_dir_all(&sys).unwrap();
    }

    #[test]
    fn a_probe_reads_the_type_and_the_label_from_the_export_lines() {
        let text = "DEVNAME=/dev/loop0p1\nLABEL=My\\ Disk\\ \\\"A\\\"\nUUID=1234-ABCD\nBLOCK_SIZE=512\nTYPE=vfat\nUSAGE=filesystem\nPART_ENTRY_TYPE=0xc\n";
        let probe = parse_probe("/dev/loop0p1", Some(1), text);
        assert_eq!(probe, Probe { device: "/dev/loop0p1".into(), partition: Some(1), fs: "vfat".into(), label: "My Disk \"A\"".into() });
        // a partition table and nothing else: no type, no label
        assert_eq!(parse_probe("/dev/loop0", None, "DEVNAME=/dev/loop0\nPTUUID=abc\nPTTYPE=gpt\n"), Probe { device: "/dev/loop0".into(), ..Probe::default() });
        // PART_ENTRY_TYPE and SEC_TYPE are not the type
        assert_eq!(parse_probe("/dev/loop0p2", Some(2), "SEC_TYPE=msdos\nPART_ENTRY_TYPE=0x83\n").fs, "");
        assert_eq!(parse_probe("/dev/loop0p2", Some(2), "").fs, "", "a probe that failed");
    }

    fn held(device: &str, partition: Option<u32>, fs: &str, label: &str) -> Probe {
        Probe { device: device.into(), partition, fs: fs.into(), label: label.into() }
    }

    fn planned(device: &str, fs: &str, name: &str) -> PlannedVolume {
        PlannedVolume { device: device.into(), fs: fs.into(), name: name.into() }
    }

    #[test]
    fn a_partitioned_image_gives_a_volume_for_each_partition_that_can_be_mounted() {
        let whole = held("/dev/loop0", None, "", "");
        // a memory-card dump: a boot partition, a root one without a label, swap, an encrypted one, an unknown one
        let parts = [
            held("/dev/loop0p3", Some(3), "swap", "swap"),
            held("/dev/loop0p2", Some(2), "ext4", ""),
            held("/dev/loop0p1", Some(1), "vfat", "BOOT"),
            held("/dev/loop0p4", Some(4), "crypto_LUKS", ""),
            held("/dev/loop0p5", Some(5), "", ""),
            held("/dev/loop0p6", Some(6), "LVM2_member", ""),
            held("/dev/loop0p7", Some(7), "exfat", " ../\n"),
        ];
        assert_eq!(
            plan_volumes("/home/me/card dump.img", &whole, &parts),
            vec![planned("/dev/loop0p1", "vfat", "BOOT"), planned("/dev/loop0p2", "ext4", "card dump 2"), planned("/dev/loop0p7", "exfat", "card dump 7")],
            "by partition number; a label that leaves no name counts as none"
        );
        // an installer image with an EFI system partition: it is vfat and is a volume like the other
        let installer = [held("/dev/loop0p1", Some(1), "vfat", "EFI"), held("/dev/loop0p2", Some(2), "ext4", "root")];
        assert_eq!(plan_volumes("/home/me/os.img", &whole, &installer), vec![planned("/dev/loop0p1", "vfat", "EFI"), planned("/dev/loop0p2", "ext4", "root")]);
    }

    #[test]
    fn an_image_that_is_one_file_system_stays_one_volume_and_nothing_mountable_is_no_plan() {
        // a disc image with a partition table beside its file system: the whole device, as before
        let disc = held("/dev/loop0", None, "iso9660", "INSTALL");
        let beside = [held("/dev/loop0p1", Some(1), "iso9660", "INSTALL"), held("/dev/loop0p2", Some(2), "vfat", "EFI")];
        assert_eq!(plan_volumes("/home/me/os.iso", &disc, &beside), vec![planned("/dev/loop0", "iso9660", "INSTALL")]);
        assert_eq!(plan_volumes("/home/me/t.img", &held("/dev/loop0", None, "vfat", ""), &[]), vec![planned("/dev/loop0", "vfat", "t")]);
        // nothing to open: no file system at all, or only partitions that are not mounted
        let empty = held("/dev/loop0", None, "", "");
        assert!(plan_volumes("/home/me/t.img", &empty, &[]).is_empty());
        assert!(plan_volumes("/home/me/t.img", &empty, &[held("/dev/loop0p1", Some(1), "swap", ""), held("/dev/loop0p2", Some(2), "crypto_LUKS", "")]).is_empty());
        // a table with more partitions than anyone opens is cut off
        let many: Vec<Probe> = (1..=40).map(|n| held(&format!("/dev/loop0p{n}"), Some(n), "ext4", "")).collect();
        let plan = plan_volumes("/home/me/t.img", &empty, &many);
        assert_eq!((plan.len(), plan.last().map(|v| v.name.as_str())), (MAX_VOLUMES, Some("t 16")));
    }

    #[test]
    fn ejecting_any_volume_of_an_image_unmounts_them_all_and_names_the_one_device_to_release() {
        let table = "/dev/sda1 / ext4 rw 0 0\n\
                     /dev/loop0p1 /run/media/me/BOOT vfat ro 0 0\n\
                     /dev/loop1 /run/media/me/Other iso9660 ro 0 0\n\
                     /dev/loop0p2 /run/media/me/card\\040dump\\0402 ext4 ro 0 0\n\
                     /dev/sdb1 /run/media/me/Stick vfat rw 0 0\n";
        // the device, a partition of it and the folder of either volume all mean the same image
        for target in ["/dev/loop0", "/dev/loop0p1", "/dev/loop0p2", "/run/media/me/BOOT", "/run/media/me/card dump 2"] {
            assert_eq!(detach_device(target, table).as_deref(), Ok("/dev/loop0"), "{target}");
        }
        assert_eq!(detach_device("/run/media/me/Other", table).as_deref(), Ok("/dev/loop1"));
        // the last mounted comes off first; the other image and the stick are left alone
        let order = vec![("/dev/loop0p2".to_string(), "/run/media/me/card dump 2".to_string()), ("/dev/loop0p1".to_string(), "/run/media/me/BOOT".to_string())];
        assert_eq!(teardown(table, "/dev/loop0"), order);
        assert_eq!(teardown(table, "/dev/loop1"), vec![("/dev/loop1".to_string(), "/run/media/me/Other".to_string())]);
        assert!(teardown(table, "/dev/loop7").is_empty(), "nothing mounted: only the device is released");
        // what is not an image is refused before anything runs
        assert!(detach_device("/run/media/me/Stick", table).is_err());
        assert!(detach_device("/dev/sdb1", table).is_err());
        assert!(detach_device("/", table).is_err());
        assert!(detach_device("/run/media/me/Gone", table).is_err());
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
        assert!(Jobs::handles(M_ATTACH) && Jobs::handles(M_DETACH) && Jobs::handles(M_EJECT) && !Jobs::handles("list"));
        let relative = Json::Obj(vec![("path".to_string(), Json::Str("t.img".into()))]);
        assert!(jobs.start("app", M_ATTACH, &relative).is_err());
        assert!(jobs.start("app", M_DETACH, &Json::Null).is_err());
        // an eject takes a plain device name and nothing else: not a folder, not a path that climbs out of /dev
        for bad in ["/run/media/me/Stick", "/dev/../etc/passwd", "sdb1", "/dev/-b"] {
            let target = Json::Obj(vec![("target".to_string(), Json::Str(bad.into()))]);
            assert!(jobs.start("app", M_EJECT, &target).is_err(), "{bad}");
        }
        assert!(jobs.start("app", M_EJECT, &Json::Null).is_err());
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
