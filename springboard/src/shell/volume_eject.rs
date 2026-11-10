//! Ejecting a removable volume (a USB stick, a memory card) for an app: the `disks` service method `eject_volume`
//! `{target}` and its `volume_ejected` event, run by `disk_image::Jobs` on a thread.
//!
//! The shell does this, not the app. An app's sandbox has no system bus, its own empty `/dev` and its own process
//! list, so the storage tool run from inside Files could not reach its service, could not see the partition and
//! could not tell which programs hold the volume.
//!
//! What is done is planned from the mount table and the kernel's device list ([`plan`]) and then run ([`eject`]):
//! every mount of the partition is unmounted, and the disk is powered off when nothing else of it is mounted. Only a
//! volume mounted under a media folder is touched; a partition mounted anywhere else belongs to the system. A disk
//! image's volume is not ejected here (`disk_image::detach` releases its loop device).

use super::disk_image::{image_device_of, run};
use crate::files::volumes;
use std::path::Path;

/// The `disks` service method and event.
pub const M_EJECT: &str = "eject_volume";
pub const E_EJECTED: &str = "volume_ejected";

/// The folders removable volumes are mounted under.
const MEDIA_ROOTS: [&str; 2] = ["/run/media", "/media"];
/// Where the kernel lists the disks and their partitions.
const SYS_BLOCK: &str = "/sys/block";

/// True for a block device path that may reach a command: `/dev/` and a plain kernel name (`sdb1`, `nvme0n1p2`,
/// `mmcblk0p1`), letters and digits only.
pub fn valid_device(path: &str) -> bool {
    path.strip_prefix("/dev/").is_some_and(|n| !n.is_empty() && n.len() <= 32 && n.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A disk and everything on it, by kernel name: the disk first, then its partitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub disk: String,
    pub members: Vec<String>,
}

/// The disk the block device `name` (`sdb1`, or a whole disk `sdb`) belongs to, with its partitions, read from the
/// kernel's list under `sys_block`. `None` when the kernel lists no such device.
pub fn family_of(sys_block: &Path, name: &str) -> Option<Family> {
    let disk = if sys_block.join(name).is_dir() {
        name.to_string()
    } else {
        std::fs::read_dir(sys_block).ok()?.flatten().find(|d| d.path().join(name).join("partition").is_file())?.file_name().into_string().ok()?
    };
    let mut members = vec![disk.clone()];
    members.extend(bifrost_core::disk_magic::partitions_of(sys_block, &disk));
    Some(Family { disk, members })
}

/// One step of an eject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Unmount the partition `device` from the folder `point`.
    Unmount { device: String, point: String },
    /// Power the whole disk off (`/dev/sdb`).
    PowerOff { disk: String },
}

/// The steps that eject the partition `device`, from the mount table `mounts` and the partition's disk (`None` when
/// the kernel does not list it). Every mount of the partition is unmounted, the last mounted first. The disk is
/// powered off after that only when it is known and no other part of it is mounted. Refused: a path that is not a
/// plain device name, a disk image's device, a partition that is not mounted, and one mounted outside the media
/// folders.
pub fn plan(device: &str, mounts: &str, family: Option<&Family>) -> Result<Vec<Step>, String> {
    if !valid_device(device) {
        return Err(format!("{device} is not a volume's device"));
    }
    if image_device_of(device).is_some() {
        return Err(format!("{device} is a disk image's volume"));
    }
    let table: Vec<(&str, String)> = mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?, volumes::unescape(f.next()?)))
        })
        .collect();
    let mut steps: Vec<Step> = table.iter().filter(|(d, _)| *d == device).map(|(d, point)| Step::Unmount { device: d.to_string(), point: point.clone() }).collect();
    if steps.is_empty() {
        return Err(format!("{device} is not mounted"));
    }
    let removable = |point: &str| MEDIA_ROOTS.iter().any(|root| Path::new(point).starts_with(root) && Path::new(point) != Path::new(root));
    if steps.iter().any(|s| matches!(s, Step::Unmount { point, .. } if !removable(point))) {
        return Err(format!("{device} is in use by the system"));
    }
    steps.reverse();
    if let Some(family) = family {
        let others_mounted = table.iter().any(|(d, _)| *d != device && d.strip_prefix("/dev/").is_some_and(|n| family.members.iter().any(|m| m == n)));
        if !others_mounted {
            steps.push(Step::PowerOff { disk: format!("/dev/{}", family.disk) });
        }
    }
    Ok(steps)
}

/// The command of one step, as (program, arguments). A root shell unmounts the folder itself, whoever mounted it; a
/// shell running as the account asks the storage service, which is also the one that powers a disk off.
pub fn command(step: &Step, as_root: bool) -> (&'static str, Vec<String>) {
    let words = |args: &[&str]| -> Vec<String> { args.iter().map(|a| a.to_string()).collect() };
    match step {
        Step::Unmount { point, .. } if as_root => ("umount", words(&["--", point])),
        Step::Unmount { device, .. } => ("udisksctl", words(&["unmount", "--no-user-interaction", "-b", device])),
        Step::PowerOff { disk } => ("udisksctl", words(&["power-off", "--no-user-interaction", "-b", disk])),
    }
}

/// Ejects the partition `device`: plans from the mount table and the kernel's device list now, then runs the steps.
/// A failed unmount ends it with the reason, and every failing step is logged by name. A disk that does not power
/// off after its volume was unmounted is logged and not a failure: the volume is closed and can be taken out.
/// Returns the result's fields (`device`, `powered_off`).
pub fn eject(device: &str) -> Result<Vec<(&'static str, String)>, String> {
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    let family = device.strip_prefix("/dev/").filter(|_| valid_device(device)).and_then(|name| family_of(Path::new(SYS_BLOCK), name));
    let steps = plan(device, &mounts, family.as_ref()).map_err(|e| {
        eprintln!("disks: eject {device}: refused: {e}");
        e
    })?;
    let as_root = crate::privd::server::euid() == 0;
    let mut powered_off = false;
    for step in &steps {
        let (program, args) = command(step, as_root);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match (step, run(program, &args)) {
            (Step::Unmount { point, .. }, Ok(_)) => {
                // the folder a root shell's own mount made; one the storage service made is its to remove
                if as_root {
                    let _ = std::fs::remove_dir(point);
                }
            }
            (Step::Unmount { point, .. }, Err(e)) => {
                eprintln!("disks: eject {device}: unmount of {point} with {program} failed: {e}");
                // the programs that hold the volume, when there are any, say more than the tool's "target is busy"
                return Err(volumes::in_use_line(&volumes::blockers(Path::new(point))).unwrap_or(e));
            }
            (Step::PowerOff { .. }, Ok(_)) => powered_off = true,
            (Step::PowerOff { disk }, Err(e)) => eprintln!("disks: eject {device}: unmounted, but power-off of {disk} failed: {e}"),
        }
    }
    Ok(vec![("device", device.to_string()), ("powered_off", powered_off.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "/dev/sda2 / ext4 rw 0 0\n\
                         /dev/sda3 /run/media/me/Data ext4 rw 0 0\n\
                         /dev/sdb1 /run/media/me/My\\040Stick vfat rw 0 0\n\
                         /dev/sdc1 /media/Card exfat rw 0 0\n\
                         /dev/sdc2 /media/Card\\0402 ext4 rw 0 0\n\
                         /dev/sdd1 /home/me/mnt ext4 rw 0 0\n\
                         /dev/loop0p1 /run/media/me/BOOT vfat ro 0 0\n";

    fn family(disk: &str, parts: &[&str]) -> Family {
        Family { disk: disk.into(), members: std::iter::once(disk).chain(parts.iter().copied()).map(String::from).collect() }
    }

    fn unmount(device: &str, point: &str) -> Step {
        Step::Unmount { device: device.into(), point: point.into() }
    }

    #[test]
    fn a_stick_is_unmounted_and_then_its_disk_powered_off() {
        let steps = plan("/dev/sdb1", TABLE, Some(&family("sdb", &["sdb1"]))).unwrap();
        assert_eq!(steps, vec![unmount("/dev/sdb1", "/run/media/me/My Stick"), Step::PowerOff { disk: "/dev/sdb".into() }]);
        // a volume on a whole disk without a partition table
        let whole = "/dev/sde /run/media/me/Plain vfat rw 0 0\n";
        assert_eq!(plan("/dev/sde", whole, Some(&family("sde", &[]))).unwrap(), vec![unmount("/dev/sde", "/run/media/me/Plain"), Step::PowerOff { disk: "/dev/sde".into() }]);
    }

    #[test]
    fn a_disk_with_another_volume_still_mounted_is_not_powered_off() {
        // the card's second partition stays mounted
        assert_eq!(plan("/dev/sdc1", TABLE, Some(&family("sdc", &["sdc1", "sdc2"]))).unwrap(), vec![unmount("/dev/sdc1", "/media/Card")]);
        // a spare partition of the system disk under the media folder: unmounted, and the disk keeps running
        assert_eq!(plan("/dev/sda3", TABLE, Some(&family("sda", &["sda1", "sda2", "sda3"]))).unwrap(), vec![unmount("/dev/sda3", "/run/media/me/Data")]);
        // a disk the kernel does not list is only unmounted
        assert_eq!(plan("/dev/sdb1", TABLE, None).unwrap(), vec![unmount("/dev/sdb1", "/run/media/me/My Stick")]);
    }

    #[test]
    fn every_mount_of_the_partition_comes_off_the_last_first() {
        let twice = "/dev/sdb1 /run/media/me/Stick vfat rw 0 0\n/dev/sdb1 /media/Again vfat rw 0 0\n";
        let steps = plan("/dev/sdb1", twice, Some(&family("sdb", &["sdb1"]))).unwrap();
        assert_eq!(steps, vec![unmount("/dev/sdb1", "/media/Again"), unmount("/dev/sdb1", "/run/media/me/Stick"), Step::PowerOff { disk: "/dev/sdb".into() }]);
    }

    #[test]
    fn what_is_not_a_removable_volume_is_refused_before_anything_runs() {
        let sda = family("sda", &["sda1", "sda2", "sda3"]);
        assert_eq!(plan("/dev/sda2", TABLE, Some(&sda)), Err("/dev/sda2 is in use by the system".into()), "the system's own volume");
        assert_eq!(plan("/dev/sdd1", TABLE, None), Err("/dev/sdd1 is in use by the system".into()), "mounted outside the media folders");
        let both = "/dev/sdb1 /run/media/me/Stick vfat rw 0 0\n/dev/sdb1 /var/lib/thing vfat rw 0 0\n";
        assert!(plan("/dev/sdb1", both, None).is_err(), "one mount outside is enough");
        assert!(plan("/dev/sdb1", "/dev/sdb1 /run/media vfat rw 0 0\n", None).is_err(), "the media folder itself is not a volume");
        assert_eq!(plan("/dev/sdz1", TABLE, None), Err("/dev/sdz1 is not mounted".into()));
        assert_eq!(plan("/dev/loop0p1", TABLE, None), Err("/dev/loop0p1 is a disk image's volume".into()));
        for bad in ["/dev/../etc", "/dev/mapper/x", "/dev/sd b1", "/dev/-b", "/dev/", "sdb1", "/run/media/me/My Stick", "/dev/sdb1\n"] {
            assert!(!valid_device(bad), "{bad}");
            assert!(plan(bad, TABLE, None).is_err(), "{bad}");
        }
        for good in ["/dev/sdb1", "/dev/nvme0n1p2", "/dev/mmcblk0p1"] {
            assert!(valid_device(good), "{good}");
        }
    }

    #[test]
    fn the_disk_of_a_partition_comes_from_the_kernels_list() {
        let sys = std::env::temp_dir().join(format!("volume-eject-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sys);
        for (disk, part) in [("sdb", "sdb1"), ("sdb", "sdb2"), ("nvme0n1", "nvme0n1p1"), ("mmcblk0", "mmcblk0p1")] {
            std::fs::create_dir_all(sys.join(disk).join(part)).unwrap();
            std::fs::write(sys.join(disk).join(part).join("partition"), "1\n").unwrap();
        }
        std::fs::create_dir_all(sys.join("sdb").join("queue")).unwrap();
        std::fs::create_dir_all(sys.join("sde")).unwrap();
        assert_eq!(family_of(&sys, "sdb2"), Some(family("sdb", &["sdb1", "sdb2"])));
        assert_eq!(family_of(&sys, "sdb"), Some(family("sdb", &["sdb1", "sdb2"])));
        assert_eq!(family_of(&sys, "nvme0n1p1"), Some(family("nvme0n1", &["nvme0n1p1"])));
        assert_eq!(family_of(&sys, "mmcblk0p1").map(|f| f.disk), Some("mmcblk0".to_string()));
        assert_eq!(family_of(&sys, "sde"), Some(family("sde", &[])), "a disk without partitions");
        assert_eq!(family_of(&sys, "queue"), None, "a folder that is not a partition");
        assert_eq!(family_of(&sys, "sdz9"), None);
        std::fs::remove_dir_all(&sys).unwrap();
    }

    #[test]
    fn each_step_is_one_fixed_command_with_the_path_as_an_argument() {
        let words = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<String>>();
        let step = unmount("/dev/sdb1", "/run/media/me/My Stick");
        assert_eq!(command(&step, true), ("umount", words(&["--", "/run/media/me/My Stick"])), "a root shell unmounts the folder");
        assert_eq!(command(&step, false), ("udisksctl", words(&["unmount", "--no-user-interaction", "-b", "/dev/sdb1"])));
        let off = Step::PowerOff { disk: "/dev/sdb".into() };
        assert_eq!(command(&off, true), ("udisksctl", words(&["power-off", "--no-user-interaction", "-b", "/dev/sdb"])));
        assert_eq!(command(&off, true), command(&off, false));
    }
}
