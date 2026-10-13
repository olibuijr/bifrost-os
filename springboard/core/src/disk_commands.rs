//! Mount, unmount, eject and rename requests, turned into the commands that carry them out. Each request is checked in
//! the same order: the device name, the claims (`disk_claims`), the approval chain (`disk_events::Approvals`) and then
//! the state of the device. A request that passes returns its plan as a list of commands. This module runs nothing:
//! the caller decides whether and how to run the plan.

use crate::disk_claims::Claims;
use crate::disk_events::{Approvals, Disk, Operation};

/// The volume helper and the arguments it takes, as a plan. The arguments are passed as separate words, never through
/// a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The device name is not a plain alphanumeric device name, so it cannot name a path under `/dev`.
    InvalidDevice(String),
    /// A filesystem type or mount option contains something other than letters, digits, `-` or `_`.
    InvalidOption(String),
    /// The label is empty, too long for the filesystem, or holds a space or a slash.
    InvalidLabel(String),
    /// Another owner holds the device.
    Claimed { device: String, owner: String },
    /// An approval gate refused the request, with its reason.
    Vetoed(String),
    /// The volume is mounted at this path, so the request needs it unmounted first.
    Mounted(String),
    /// The request needs a mounted volume and the device is not mounted.
    NotMounted,
    /// The device is read-only, so its label cannot change.
    ReadOnly,
    /// The last snapshot has no disk with this device name.
    NoDisk(String),
    /// The request needs a whole disk and the device is a partition.
    NotWhole(String),
    /// The request needs a partition and the device is a whole disk.
    NotPartition(String),
}

impl Refusal {
    /// A sentence for the refusal, shown to the user in a footer or printed on a command line.
    pub fn sentence(&self) -> String {
        match self {
            Refusal::InvalidDevice(name) => format!("\u{201c}{name}\u{201d} is not a device name this app can use."),
            Refusal::InvalidOption(option) => format!("The file system option \u{201c}{option}\u{201d} is not valid."),
            Refusal::InvalidLabel(label) => format!("The label \u{201c}{label}\u{201d} is not valid for this file system."),
            Refusal::Claimed { device, owner } => format!("{device} is in use by {owner}."),
            Refusal::Vetoed(reason) => reason.clone(),
            Refusal::Mounted(point) => format!("The volume is mounted at {point}. Unmount it first."),
            Refusal::NotMounted => "The volume is not mounted.".to_string(),
            Refusal::ReadOnly => "The volume is read-only.".to_string(),
            Refusal::NoDisk(name) => format!("{name} is no longer connected."),
            Refusal::NotWhole(name) => format!("{name} is a partition. Choose the whole disk instead."),
            Refusal::NotPartition(name) => format!("{name} is a whole disk. Choose one of its partitions instead."),
        }
    }
}

/// Options for a mount request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountOptions {
    pub read_only: bool,
    /// The filesystem type to mount as, when the caller knows it; `None` lets the helper detect it.
    pub fs_type: Option<String>,
}

/// The filesystems whose labels this module can change, with their label limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filesystem {
    Fat,
    Ext,
}

impl Filesystem {
    /// The longest label the filesystem stores, in bytes.
    pub fn max_label_len(self) -> usize {
        match self {
            Filesystem::Fat => 11,
            Filesystem::Ext => 16,
        }
    }

    fn label_tool(self) -> &'static str {
        match self {
            Filesystem::Fat => "fatlabel",
            Filesystem::Ext => "e2label",
        }
    }
}

const VOLUME_TOOL: &str = "udisksctl";

/// The device path for a name, when the name is a plain alphanumeric device name such as `sdb1` or `nvme0n1p1`.
pub(crate) fn device_path(disk: &Disk) -> Result<String, Refusal> {
    let name = &disk.bsd_name;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(Refusal::InvalidDevice(name.clone()));
    }
    Ok(format!("/dev/{name}"))
}

fn is_token(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(crate) fn gate_claim(claims: &Claims, disk: &Disk, owner: &str) -> Result<(), Refusal> {
    claims
        .check(&disk.bsd_name, owner)
        .map_err(|c| Refusal::Claimed { device: c.device, owner: c.owner })
}

pub(crate) fn gate_approval(approvals: &Approvals, op: Operation, disk: &Disk) -> Result<(), Refusal> {
    approvals.approve(op, disk).map_err(|d| Refusal::Vetoed(d.reason))
}

fn check_label(label: &str, fs: Filesystem) -> Result<(), Refusal> {
    let valid = !label.is_empty()
        && label.len() <= fs.max_label_len()
        && label.bytes().all(|b| b.is_ascii_graphic() && b != b'/');
    if valid {
        Ok(())
    } else {
        Err(Refusal::InvalidLabel(label.to_string()))
    }
}

fn unmount_command(dev: &str) -> Command {
    Command { program: VOLUME_TOOL, args: vec!["unmount".into(), "-b".into(), dev.to_string()] }
}

/// Plans a mount of an unmounted volume. The approval chain sees the request as `Mount`.
pub fn mount(
    disk: &Disk,
    opts: &MountOptions,
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let dev = device_path(disk)?;
    gate_claim(claims, disk, owner)?;
    gate_approval(approvals, Operation::Mount, disk)?;
    if let Some(mount) = &disk.mount_point {
        return Err(Refusal::Mounted(mount.clone()));
    }
    let mut args = vec!["mount".to_string(), "-b".to_string(), dev];
    if let Some(fs) = &opts.fs_type {
        if !is_token(fs) {
            return Err(Refusal::InvalidOption(fs.clone()));
        }
        args.push("-t".into());
        args.push(fs.clone());
    }
    if opts.read_only {
        args.push("-o".into());
        args.push("ro".into());
    }
    Ok(vec![Command { program: VOLUME_TOOL, args }])
}

/// Plans an unmount of a mounted volume.
pub fn unmount(disk: &Disk, approvals: &Approvals, claims: &Claims, owner: &str) -> Result<Vec<Command>, Refusal> {
    let dev = device_path(disk)?;
    gate_claim(claims, disk, owner)?;
    gate_approval(approvals, Operation::Unmount, disk)?;
    if disk.mount_point.is_none() {
        return Err(Refusal::NotMounted);
    }
    Ok(vec![unmount_command(&dev)])
}

/// Plans an eject. A mounted volume is unmounted first, and both the unmount and the eject pass the approval chain.
pub fn eject(disk: &Disk, approvals: &Approvals, claims: &Claims, owner: &str) -> Result<Vec<Command>, Refusal> {
    let dev = device_path(disk)?;
    gate_claim(claims, disk, owner)?;
    gate_approval(approvals, Operation::Eject, disk)?;
    let mut plan = Vec::new();
    if disk.mount_point.is_some() {
        gate_approval(approvals, Operation::Unmount, disk)?;
        plan.push(unmount_command(&dev));
    }
    plan.push(Command { program: VOLUME_TOOL, args: vec!["power-off".into(), "-b".into(), dev] });
    Ok(plan)
}

/// Plans a label change. The device must be writable and unmounted, and the label must fit the filesystem.
pub fn rename(disk: &Disk, label: &str, fs: Filesystem, claims: &Claims, owner: &str) -> Result<Vec<Command>, Refusal> {
    let dev = device_path(disk)?;
    gate_claim(claims, disk, owner)?;
    if disk.read_only {
        return Err(Refusal::ReadOnly);
    }
    if let Some(mount) = &disk.mount_point {
        return Err(Refusal::Mounted(mount.clone()));
    }
    check_label(label, fs)?;
    Ok(vec![Command { program: fs.label_tool(), args: vec![dev, label.to_string()] }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str, mount: Option<&str>) -> Disk {
        Disk {
            bsd_name: name.to_string(),
            size: 8 << 30,
            removable: true,
            read_only: false,
            model: "Stick".to_string(),
            mount_point: mount.map(str::to_string),
        }
    }

    #[test]
    fn a_read_only_fat_mount_plans_one_command_with_the_type_and_option() {
        let opts = MountOptions { read_only: true, fs_type: Some("vfat".into()) };
        let plan = mount(&disk("sdb1", None), &opts, &Approvals::new(), &Claims::new(), "files").unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].program, "udisksctl");
        assert_eq!(plan[0].args, vec!["mount", "-b", "/dev/sdb1", "-t", "vfat", "-o", "ro"]);
    }

    #[test]
    fn a_mount_is_refused_while_another_owner_claims_the_device() {
        let mut claims = Claims::new();
        claims.claim("sdb1", "installer").unwrap();
        let err = mount(&disk("sdb1", None), &MountOptions::default(), &Approvals::new(), &claims, "files");
        assert_eq!(err, Err(Refusal::Claimed { device: "sdb1".into(), owner: "installer".into() }));
        assert!(mount(&disk("sdb1", None), &MountOptions::default(), &Approvals::new(), &claims, "installer").is_ok());
    }

    #[test]
    fn an_approval_gate_vetoes_the_mount_with_its_reason() {
        let mut approvals = Approvals::new();
        approvals.register(|op, _| if op == Operation::Mount { Err("policy".to_string()) } else { Ok(()) });
        let err = mount(&disk("sdb1", None), &MountOptions::default(), &approvals, &Claims::new(), "files");
        assert_eq!(err, Err(Refusal::Vetoed("policy".into())));
        assert!(unmount(&disk("sdb1", Some("/media/stick")), &approvals, &Claims::new(), "files").is_ok());
    }

    #[test]
    fn a_mount_option_with_a_comma_is_refused_before_any_command_is_planned() {
        let opts = MountOptions { read_only: false, fs_type: Some("vfat,uid=0".into()) };
        let err = mount(&disk("sdb1", None), &opts, &Approvals::new(), &Claims::new(), "files");
        assert_eq!(err, Err(Refusal::InvalidOption("vfat,uid=0".into())));
    }

    #[test]
    fn a_device_name_with_a_path_separator_is_refused() {
        let err = unmount(&disk("../sda", Some("/x")), &Approvals::new(), &Claims::new(), "files");
        assert_eq!(err, Err(Refusal::InvalidDevice("../sda".into())));
    }

    #[test]
    fn eject_of_a_mounted_volume_unmounts_then_powers_off() {
        let plan = eject(&disk("sdb1", Some("/media/stick")), &Approvals::new(), &Claims::new(), "files").unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].args, vec!["unmount", "-b", "/dev/sdb1"]);
        assert_eq!(plan[1].args, vec!["power-off", "-b", "/dev/sdb1"]);
    }

    #[test]
    fn eject_of_an_unmounted_device_is_a_single_power_off() {
        let plan = eject(&disk("sdb", None), &Approvals::new(), &Claims::new(), "files").unwrap();
        assert_eq!(plan, vec![Command { program: "udisksctl", args: vec!["power-off".into(), "-b".into(), "/dev/sdb".into()] }]);
    }

    #[test]
    fn unmount_of_an_unmounted_device_is_refused() {
        let err = unmount(&disk("sdb", None), &Approvals::new(), &Claims::new(), "files");
        assert_eq!(err, Err(Refusal::NotMounted));
    }

    #[test]
    fn a_fat_label_of_twelve_bytes_is_too_long_and_eleven_is_planned() {
        let claims = Claims::new();
        let long = rename(&disk("sdb1", None), "ABCDEFGHIJKL", Filesystem::Fat, &claims, "files");
        assert_eq!(long, Err(Refusal::InvalidLabel("ABCDEFGHIJKL".into())));
        let plan = rename(&disk("sdb1", None), "BIFROST", Filesystem::Fat, &claims, "files").unwrap();
        assert_eq!(plan[0].program, "fatlabel");
        assert_eq!(plan[0].args, vec!["/dev/sdb1", "BIFROST"]);
        assert!(rename(&disk("sdb1", None), "ABCDEFGHIJK", Filesystem::Fat, &claims, "files").is_ok());
    }

    #[test]
    fn an_ext_label_allows_sixteen_bytes_and_refuses_a_space() {
        let claims = Claims::new();
        assert!(rename(&disk("sdb2", None), &"x".repeat(16), Filesystem::Ext, &claims, "files").is_ok());
        assert!(rename(&disk("sdb2", None), &"x".repeat(17), Filesystem::Ext, &claims, "files").is_err());
        assert!(rename(&disk("sdb2", None), "my disk", Filesystem::Ext, &claims, "files").is_err());
    }

    #[test]
    fn rename_refuses_a_read_only_or_mounted_volume() {
        let claims = Claims::new();
        let mut ro = disk("sr0", None);
        ro.read_only = true;
        assert_eq!(rename(&ro, "DISC", Filesystem::Fat, &claims, "files"), Err(Refusal::ReadOnly));
        let mounted = disk("sdb1", Some("/media/stick"));
        assert_eq!(
            rename(&mounted, "NEW", Filesystem::Fat, &claims, "files"),
            Err(Refusal::Mounted("/media/stick".into()))
        );
    }
}
