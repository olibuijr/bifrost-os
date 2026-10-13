//! Erase, partition and first-aid plans for whole disks and volumes. Like `disk_commands`, this module runs nothing: each
//! request is checked and turned into a list of commands. Those commands write partition tables and file systems or
//! check and repair them, so they need root. The privileged helper runs the plan; the caller never runs it as the user.
//!
//! The checks come in the order `disk_commands` uses: the device name, the claims, the approval chain (`Operation`),
//! then the state of the device (read-only, mounted) and the label. A check tool that repairs can exit 1 when it fixed
//! something; the runner must treat that as success, the same as the tool's own preen mode does.

use crate::disk_claims::Claims;
use crate::disk_commands::{self, Command, Refusal};
use crate::disk_events::{Approvals, Disk, Operation};

/// The file systems this module writes and checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    Ext4,
    ExFat,
    Fat32,
    Ntfs,
    Btrfs,
}

/// Verify is read-only; repair fixes what it finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Verify,
    Repair,
}

/// How a whole disk is partitioned when it is erased: a GUID partition table, or a DOS (MBR) one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Gpt,
    Mbr,
}

impl Scheme {
    /// The name on the wire and the command line: `gpt` or `mbr`.
    pub fn parse(text: &str) -> Option<Scheme> {
        match text {
            "gpt" => Some(Scheme::Gpt),
            "mbr" => Some(Scheme::Mbr),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Scheme::Gpt => "gpt",
            Scheme::Mbr => "mbr",
        }
    }

    /// The name a control shows.
    pub fn title(self) -> &'static str {
        match self {
            Scheme::Gpt => "GUID Partition Map",
            Scheme::Mbr => "Master Boot Record",
        }
    }
}

/// The RAID levels the array creation plan supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaidLevel {
    Raid0,
    Raid1,
    Raid5,
    Raid6,
    Raid10,
}

/// The most members one array takes.
pub const MAX_RAID_MEMBERS: usize = 16;

impl RaidLevel {
    pub const ALL: [RaidLevel; 5] = [RaidLevel::Raid0, RaidLevel::Raid1, RaidLevel::Raid5, RaidLevel::Raid6, RaidLevel::Raid10];

    /// The level number `mdadm --level` takes: `0`, `1`, `5`, `6` or `10`.
    pub fn parse(text: &str) -> Option<RaidLevel> {
        RaidLevel::ALL.into_iter().find(|l| l.word() == text)
    }

    pub fn word(self) -> &'static str {
        match self {
            RaidLevel::Raid0 => "0",
            RaidLevel::Raid1 => "1",
            RaidLevel::Raid5 => "5",
            RaidLevel::Raid6 => "6",
            RaidLevel::Raid10 => "10",
        }
    }

    /// The name a control shows.
    pub fn title(self) -> &'static str {
        match self {
            RaidLevel::Raid0 => "RAID 0",
            RaidLevel::Raid1 => "RAID 1",
            RaidLevel::Raid5 => "RAID 5",
            RaidLevel::Raid6 => "RAID 6",
            RaidLevel::Raid10 => "RAID 10",
        }
    }

    /// What the level does, in a few words.
    pub fn summary(self) -> &'static str {
        match self {
            RaidLevel::Raid0 => "Striped: all the space, no redundancy.",
            RaidLevel::Raid1 => "Mirrored: every member holds a full copy.",
            RaidLevel::Raid5 => "Striped with parity: survives one failure.",
            RaidLevel::Raid6 => "Striped with double parity: survives two failures.",
            RaidLevel::Raid10 => "Mirrored stripes: survives one failure per pair.",
        }
    }

    /// The fewest members the level needs.
    pub fn min_members(self) -> usize {
        match self {
            RaidLevel::Raid0 | RaidLevel::Raid1 => 2,
            RaidLevel::Raid5 => 3,
            RaidLevel::Raid6 | RaidLevel::Raid10 => 4,
        }
    }
}

impl Personality {
    /// Every file system this module writes, in the order a control lists them.
    pub const ALL: [Personality; 5] = [Personality::Ext4, Personality::ExFat, Personality::Fat32, Personality::Ntfs, Personality::Btrfs];

    /// The name the user picks: `ext4`, `exfat`, `fat32`, `ntfs` or `btrfs`.
    pub fn parse(text: &str) -> Option<Personality> {
        match text {
            "ext4" => Some(Personality::Ext4),
            "exfat" => Some(Personality::ExFat),
            "fat32" => Some(Personality::Fat32),
            "ntfs" => Some(Personality::Ntfs),
            "btrfs" => Some(Personality::Btrfs),
            _ => None,
        }
    }

    /// The name a control shows.
    pub fn title(self) -> &'static str {
        match self {
            Personality::Ext4 => "ext4",
            Personality::ExFat => "exFAT",
            Personality::Fat32 => "FAT32",
            Personality::Ntfs => "NTFS",
            Personality::Btrfs => "btrfs",
        }
    }

    /// The file system type the kernel and the udev database report for a volume (`ID_FS_TYPE`, `/proc/mounts`).
    pub fn from_probe_type(text: &str) -> Option<Personality> {
        match text {
            "ext4" => Some(Personality::Ext4),
            "exfat" => Some(Personality::ExFat),
            "vfat" => Some(Personality::Fat32),
            "ntfs" => Some(Personality::Ntfs),
            "btrfs" => Some(Personality::Btrfs),
            _ => None,
        }
    }

    /// The longest label the file system stores, in bytes.
    pub fn label_limit(self) -> usize {
        match self {
            Personality::Ext4 => 16,
            Personality::ExFat => 15,
            Personality::Fat32 => 11,
            // both store more, but a label also names the GPT partition, which holds 36 characters
            Personality::Ntfs | Personality::Btrfs => 32,
        }
    }

    /// The GPT partition type code for a new partition of this file system (Linux data, or Microsoft basic data).
    fn gpt_code(self) -> &'static str {
        match self {
            Personality::Ext4 | Personality::Btrfs => "8300",
            Personality::ExFat | Personality::Fat32 | Personality::Ntfs => "0700",
        }
    }

    /// The file system word `parted mkpart` takes, which only picks the partition type byte of a DOS table.
    fn parted_word(self) -> &'static str {
        match self {
            Personality::Fat32 => "fat32",
            Personality::ExFat | Personality::Ntfs => "ntfs",
            Personality::Ext4 | Personality::Btrfs => "ext4",
        }
    }

    /// The command that writes an empty file system on `dev`. An empty label leaves the volume unlabelled.
    pub fn make_command(self, dev: &str, label: &str) -> Command {
        let mut args: Vec<String> = Vec::new();
        let program = match self {
            Personality::Ext4 => {
                // -F: the tool would otherwise ask on a terminal, and there is none under the helper.
                args.extend(["-F", "-q"].map(String::from));
                push_label(&mut args, "-L", label);
                "mkfs.ext4"
            }
            Personality::ExFat => {
                push_label(&mut args, "-n", label);
                "mkfs.exfat"
            }
            Personality::Fat32 => {
                args.extend(["-F", "32"].map(String::from));
                push_label(&mut args, "-n", label);
                "mkfs.vfat"
            }
            Personality::Ntfs => {
                // -F: a whole device or an image file is accepted; -Q: a quick format without a zeroing pass
                args.extend(["-F", "-Q"].map(String::from));
                push_label(&mut args, "-L", label);
                "mkfs.ntfs"
            }
            Personality::Btrfs => {
                args.push("-f".to_string());
                push_label(&mut args, "-L", label);
                "mkfs.btrfs"
            }
        };
        args.push(dev.to_string());
        Command { program, args }
    }

    /// The check command for `dev`: verify is a read-only pass, repair fixes errors without asking.
    fn check_command(self, dev: &str, mode: CheckMode) -> Command {
        let (program, flags): (&'static str, &[&str]) = match (self, mode) {
            (Personality::Ext4, CheckMode::Verify) => ("e2fsck", &["-n"]),
            (Personality::Ext4, CheckMode::Repair) => ("e2fsck", &["-p"]),
            (Personality::ExFat, CheckMode::Verify) => ("fsck.exfat", &["-n"]),
            (Personality::ExFat, CheckMode::Repair) => ("fsck.exfat", &["-a"]),
            (Personality::Fat32, CheckMode::Verify) => ("fsck.fat", &["-n"]),
            (Personality::Fat32, CheckMode::Repair) => ("fsck.fat", &["-a"]),
            (Personality::Ntfs, CheckMode::Verify) => ("ntfsfix", &["-n"]),
            (Personality::Ntfs, CheckMode::Repair) => ("ntfsfix", &["-d"]),
            (Personality::Btrfs, CheckMode::Verify) => ("btrfs", &["check", "--readonly"]),
            (Personality::Btrfs, CheckMode::Repair) => ("btrfs", &["check", "--repair"]),
        };
        let mut args: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
        args.push(dev.to_string());
        Command { program, args }
    }

    /// The command that sets the label of the file system on `dev`.
    pub fn label_command(self, dev: &str, label: &str) -> Command {
        let (program, mut args): (&'static str, Vec<String>) = match self {
            Personality::Ext4 => ("e2label", Vec::new()),
            Personality::ExFat => ("exfatlabel", Vec::new()),
            Personality::Fat32 => ("fatlabel", Vec::new()),
            Personality::Ntfs => ("ntfslabel", Vec::new()),
            Personality::Btrfs => ("btrfs", vec!["filesystem".to_string(), "label".to_string()]),
        };
        args.push(dev.to_string());
        args.push(label.to_string());
        Command { program, args }
    }
}

fn push_label(args: &mut Vec<String>, flag: &str, label: &str) {
    if !label.is_empty() {
        args.push(flag.to_string());
        args.push(label.to_string());
    }
}

/// The device path of partition `index` (1-based) on a whole disk: `/dev/sdb1`, and `/dev/nvme0n1p1` where the disk's
/// name ends in a digit.
pub fn partition_device(disk: &str, index: u32) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("/dev/{disk}p{index}")
    } else {
        format!("/dev/{disk}{index}")
    }
}

/// A label is at most the file system's limit, uses letters, digits, spaces, `-` and `_`, and does not start or end
/// with a space. An empty label is allowed.
fn check_label(label: &str, fs: Personality) -> Result<(), Refusal> {
    let ok = label.len() <= fs.label_limit()
        && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-' || b == b'_')
        && !label.starts_with(' ')
        && !label.ends_with(' ');
    if ok {
        Ok(())
    } else {
        Err(Refusal::InvalidLabel(label.to_string()))
    }
}

/// The device path after the claim and approval gates pass for `op`.
fn gate(disk: &Disk, op: Operation, approvals: &Approvals, claims: &Claims, owner: &str) -> Result<String, Refusal> {
    let dev = disk_commands::device_path(disk)?;
    disk_commands::gate_claim(claims, disk, owner)?;
    disk_commands::gate_approval(approvals, op, disk)?;
    Ok(dev)
}

/// Plans a new empty file system on an unmounted partition. Any data on the partition is lost.
pub fn erase_volume(
    disk: &Disk,
    fs: Personality,
    label: &str,
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let dev = gate(disk, Operation::Erase, approvals, claims, owner)?;
    if disk.read_only {
        return Err(Refusal::ReadOnly);
    }
    if let Some(point) = &disk.mount_point {
        return Err(Refusal::Mounted(point.clone()));
    }
    check_label(label, fs)?;
    Ok(vec![fs.make_command(&dev, label)])
}

/// Plans an erase of a whole disk: the partition table is replaced by one partition that takes the whole disk (in a
/// GPT or a DOS table, as `scheme` says), which is then formatted. `partitions` are the disk's own partitions; any of
/// them mounted refuses the erase. A DOS table has no partition names, so the label names the file system only.
#[allow(clippy::too_many_arguments)]
pub fn erase_disk(
    disk: &Disk,
    partitions: &[Disk],
    fs: Personality,
    label: &str,
    scheme: Scheme,
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let dev = gate(disk, Operation::Erase, approvals, claims, owner)?;
    if disk.read_only {
        return Err(Refusal::ReadOnly);
    }
    let mounted = disk.mount_point.iter().chain(partitions.iter().filter_map(|p| p.mount_point.as_ref())).next();
    if let Some(point) = mounted {
        return Err(Refusal::Mounted(point.clone()));
    }
    check_label(label, fs)?;
    if scheme == Scheme::Mbr {
        return Ok(vec![
            // zap-all clears the GPT, its backup at the end of the disk and any old DOS table
            Command { program: "sgdisk", args: vec!["--zap-all".to_string(), dev.clone()] },
            Command { program: "parted", args: vec!["-s".to_string(), dev.clone(), "mklabel".to_string(), "msdos".to_string()] },
            Command {
                program: "parted",
                args: ["-s", "-a", "optimal", dev.as_str(), "mkpart", "primary", fs.parted_word(), "1MiB", "100%"].iter().map(|w| w.to_string()).collect(),
            },
            Command { program: "partprobe", args: vec![dev.clone()] },
            Command { program: "udevadm", args: vec!["settle".to_string()] },
            fs.make_command(&partition_device(&disk.bsd_name, 1), label),
        ]);
    }
    let mut create = vec!["-n".to_string(), "1:0:0".to_string(), "-t".to_string(), format!("1:{}", fs.gpt_code())];
    if !label.is_empty() {
        create.push("-c".to_string());
        create.push(format!("1:{label}"));
    }
    create.push(dev.clone());
    Ok(vec![
        Command { program: "sgdisk", args: vec!["--zap-all".to_string(), dev.clone()] },
        Command { program: "sgdisk", args: create },
        Command { program: "partprobe", args: vec![dev.clone()] },
        Command { program: "udevadm", args: vec!["settle".to_string()] },
        fs.make_command(&partition_device(&disk.bsd_name, 1), label),
    ])
}

/// The most partitions one partition plan creates.
pub const MAX_PARTITIONS: usize = 16;

/// One partition of a partition plan: its size in MiB (0 takes the rest of the disk, allowed for the last one only),
/// the file system it is formatted with, and its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionSpec {
    pub size_mib: u64,
    pub fs: Personality,
    pub label: String,
}

/// Plans a new partition table on a whole disk, one GPT partition per spec in order, each formatted empty. Like
/// `erase_disk`, every partition of the disk is replaced and any mounted one refuses the plan.
pub fn partition_disk(
    disk: &Disk,
    partitions: &[PartitionSpec],
    existing: &[Disk],
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let dev = gate(disk, Operation::Erase, approvals, claims, owner)?;
    if disk.read_only {
        return Err(Refusal::ReadOnly);
    }
    let mounted = disk.mount_point.iter().chain(existing.iter().filter_map(|p| p.mount_point.as_ref())).next();
    if let Some(point) = mounted {
        return Err(Refusal::Mounted(point.clone()));
    }
    if partitions.is_empty() || partitions.len() > MAX_PARTITIONS {
        return Err(Refusal::InvalidOption(format!("{} partitions", partitions.len())));
    }
    let last = partitions.len() - 1;
    let mut create: Vec<String> = Vec::new();
    let mut formats: Vec<Command> = Vec::new();
    for (i, spec) in partitions.iter().enumerate() {
        if spec.size_mib == 0 && i != last {
            return Err(Refusal::InvalidOption("only the last partition takes the rest of the disk".to_string()));
        }
        check_label(&spec.label, spec.fs)?;
        let index = i + 1;
        let end = if spec.size_mib == 0 { "0".to_string() } else { format!("+{}M", spec.size_mib) };
        create.extend(["-n".to_string(), format!("{index}:0:{end}"), "-t".to_string(), format!("{index}:{}", spec.fs.gpt_code())]);
        if !spec.label.is_empty() {
            create.extend(["-c".to_string(), format!("{index}:{}", spec.label)]);
        }
        formats.push(spec.fs.make_command(&partition_device(&disk.bsd_name, index as u32), &spec.label));
    }
    create.push(dev.clone());
    let mut plan = vec![
        Command { program: "sgdisk", args: vec!["--zap-all".to_string(), dev.clone()] },
        Command { program: "sgdisk", args: create },
        Command { program: "partprobe", args: vec![dev.clone()] },
        Command { program: "udevadm", args: vec!["settle".to_string()] },
    ];
    plan.extend(formats);
    Ok(plan)
}

/// Plans a file system check (first aid) of an unmounted volume. `fs_type` is the type the volume reports; a type this
/// module has no checker for is refused.
pub fn check_volume(
    disk: &Disk,
    fs_type: &str,
    mode: CheckMode,
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let op = match mode {
        CheckMode::Verify => Operation::Verify,
        CheckMode::Repair => Operation::Repair,
    };
    let dev = gate(disk, op, approvals, claims, owner)?;
    let fs = Personality::from_probe_type(fs_type).ok_or_else(|| Refusal::InvalidOption(fs_type.to_string()))?;
    if let Some(point) = &disk.mount_point {
        return Err(Refusal::Mounted(point.clone()));
    }
    Ok(vec![fs.check_command(&dev, mode)])
}

/// Plans a new label for the file system on an unmounted volume. `fs_type` is the type the volume reports (empty when
/// it holds no file system). The label may be empty only to clear it, which the label tools of some file systems
/// refuse, so an empty label is refused here.
pub fn rename_volume(
    disk: &Disk,
    fs_type: &str,
    label: &str,
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let dev = gate(disk, Operation::Erase, approvals, claims, owner)?;
    if fs_type.is_empty() {
        return Err(Refusal::Invalid(format!("{} has no file system to rename.", disk.bsd_name)));
    }
    let fs = Personality::from_probe_type(fs_type).ok_or_else(|| Refusal::InvalidOption(fs_type.to_string()))?;
    if disk.read_only {
        return Err(Refusal::ReadOnly);
    }
    if let Some(point) = &disk.mount_point {
        return Err(Refusal::Mounted(point.clone()));
    }
    if label.is_empty() {
        return Err(Refusal::InvalidLabel(String::new()));
    }
    check_label(label, fs)?;
    Ok(vec![fs.label_command(&dev, label)])
}

/// Plans a copy of the volume `source` onto `dest`, block for block. The destination is overwritten, so it must be
/// writable, unmounted (with its own partitions, `dest_children`) and at least as large as the source; the source
/// must be unmounted too (with `source_children`), or the copy would capture a file system that is still changing.
/// Neither may contain the other.
pub fn restore_volume(
    dest: &Disk,
    dest_children: &[Disk],
    source: &Disk,
    source_children: &[Disk],
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    let to = gate(dest, Operation::Erase, approvals, claims, owner)?;
    let from = disk_commands::device_path(source)?;
    disk_commands::gate_claim(claims, source, owner)?;
    if dest.bsd_name == source.bsd_name {
        return Err(Refusal::Invalid("The source and the destination are the same volume.".to_string()));
    }
    let holds = |children: &[Disk], name: &str| children.iter().any(|c| c.bsd_name == name);
    if holds(source_children, &dest.bsd_name) || holds(dest_children, &source.bsd_name) {
        return Err(Refusal::Invalid("The source and the destination overlap: one is part of the other.".to_string()));
    }
    if dest.read_only {
        return Err(Refusal::ReadOnly);
    }
    let mounted = |d: &Disk, children: &[Disk]| d.mount_point.clone().or_else(|| children.iter().find_map(|c| c.mount_point.clone()));
    if let Some(point) = mounted(dest, dest_children).or_else(|| mounted(source, source_children)) {
        return Err(Refusal::Mounted(point));
    }
    if source.size > dest.size {
        return Err(Refusal::Invalid(format!("{} is larger than {}, so it does not fit.", source.bsd_name, dest.bsd_name)));
    }
    Ok(vec![
        Command {
            program: "dd",
            args: vec![format!("if={from}"), format!("of={to}"), "bs=4M".to_string(), "conv=fsync".to_string(), "status=none".to_string()],
        },
        Command { program: "partprobe", args: vec![to] },
        Command { program: "udevadm", args: vec!["settle".to_string()] },
    ])
}

/// A name for a new software RAID array: `md` and one to four digits, such as `md0`.
pub fn is_array_name(name: &str) -> bool {
    name.strip_prefix("md").is_some_and(|n| (1..=4).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Plans a new software RAID array `array` of `level` over `members`, each a disk or partition with its own partitions.
/// A member must be writable and unmounted, none may hold another, and the level needs its minimum member count.
/// `array_exists` is true when a device of that name is already there.
pub fn create_raid(
    array: &str,
    array_exists: bool,
    level: RaidLevel,
    members: &[(Disk, Vec<Disk>)],
    approvals: &Approvals,
    claims: &Claims,
    owner: &str,
) -> Result<Vec<Command>, Refusal> {
    if !is_array_name(array) {
        return Err(Refusal::InvalidDevice(array.to_string()));
    }
    if array_exists {
        return Err(Refusal::Invalid(format!("{array} already exists. Choose another array name.")));
    }
    if members.len() < level.min_members() {
        return Err(Refusal::Invalid(format!("{} needs at least {} members.", level.title(), level.min_members())));
    }
    if members.len() > MAX_RAID_MEMBERS {
        return Err(Refusal::Invalid(format!("An array takes at most {MAX_RAID_MEMBERS} members.")));
    }
    let mut devices = Vec::with_capacity(members.len());
    for (i, (disk, children)) in members.iter().enumerate() {
        let dev = gate(disk, Operation::Erase, approvals, claims, owner)?;
        if disk.read_only {
            return Err(Refusal::ReadOnly);
        }
        if let Some(point) = disk.mount_point.iter().chain(children.iter().filter_map(|c| c.mount_point.as_ref())).next() {
            return Err(Refusal::Mounted(point.clone()));
        }
        for (j, (other, _)) in members.iter().enumerate() {
            let nested = children.iter().any(|c| c.bsd_name == other.bsd_name);
            if i != j && (other.bsd_name == disk.bsd_name || nested) {
                return Err(Refusal::Invalid(format!("{} and {} overlap: choose one of them.", disk.bsd_name, other.bsd_name)));
            }
        }
        devices.push(dev);
    }
    let mut create = vec![
        "--create".to_string(),
        format!("/dev/{array}"),
        // --run: do not stop to ask whether to continue over members that still hold a file system
        "--run".to_string(),
        format!("--level={}", level.word()),
        format!("--raid-devices={}", devices.len()),
    ];
    create.extend(devices);
    Ok(vec![Command { program: "mdadm", args: create }, Command { program: "udevadm", args: vec!["settle".to_string()] }])
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

    fn words(plan: &[Command]) -> Vec<(&'static str, Vec<String>)> {
        plan.iter().map(|c| (c.program, c.args.clone())).collect()
    }

    fn s(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn personalities_parse_and_keep_their_label_limits() {
        assert_eq!(Personality::parse("fat32"), Some(Personality::Fat32));
        assert_eq!(Personality::parse("ntfs"), Some(Personality::Ntfs));
        assert_eq!(Personality::parse("xfs"), None);
        assert_eq!(Personality::from_probe_type("vfat"), Some(Personality::Fat32));
        assert_eq!(Personality::Fat32.label_limit(), 11);
        assert_eq!(Personality::ExFat.label_limit(), 15);
        assert_eq!(Personality::Ext4.label_limit(), 16);
    }

    #[test]
    fn partition_names_follow_the_kernel_rule() {
        assert_eq!(partition_device("sdb", 1), "/dev/sdb1");
        assert_eq!(partition_device("nvme0n1", 1), "/dev/nvme0n1p1");
        assert_eq!(partition_device("mmcblk0", 2), "/dev/mmcblk0p2");
    }

    #[test]
    fn an_ext4_volume_is_made_non_interactively_with_its_label() {
        let plan = erase_volume(&disk("sdb1", None), Personality::Ext4, "BIFROST", &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(words(&plan), vec![("mkfs.ext4", s(&["-F", "-q", "-L", "BIFROST", "/dev/sdb1"]))]);
    }

    #[test]
    fn a_fat32_volume_takes_the_fat_type_flag_and_an_exfat_one_its_own_tool() {
        let fat = erase_volume(&disk("sdb1", None), Personality::Fat32, "STICK", &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(words(&fat), vec![("mkfs.vfat", s(&["-F", "32", "-n", "STICK", "/dev/sdb1"]))]);
        let ex = erase_volume(&disk("sdb2", None), Personality::ExFat, "", &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(words(&ex), vec![("mkfs.exfat", s(&["/dev/sdb2"]))]);
    }

    #[test]
    fn erasing_a_mounted_volume_is_refused() {
        let err = erase_volume(&disk("sdb1", Some("/media/stick")), Personality::Fat32, "X", &Approvals::new(), &Claims::new(), "disk");
        assert_eq!(err, Err(Refusal::Mounted("/media/stick".into())));
    }

    #[test]
    fn a_label_over_the_limit_or_with_a_slash_is_refused() {
        let claims = Claims::new();
        let approvals = Approvals::new();
        let long = erase_volume(&disk("sdb1", None), Personality::Fat32, "ABCDEFGHIJKL", &approvals, &claims, "disk");
        assert_eq!(long, Err(Refusal::InvalidLabel("ABCDEFGHIJKL".into())));
        let slash = erase_volume(&disk("sdb1", None), Personality::Ext4, "a/b", &approvals, &claims, "disk");
        assert_eq!(slash, Err(Refusal::InvalidLabel("a/b".into())));
        let padded = erase_volume(&disk("sdb1", None), Personality::ExFat, " x", &approvals, &claims, "disk");
        assert!(padded.is_err());
    }

    #[test]
    fn an_erase_of_a_disk_writes_one_gpt_partition_then_formats_it() {
        let plan = erase_disk(&disk("sdb", None), &[disk("sdb1", None)], Personality::Fat32, "STICK", Scheme::Gpt, &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("sgdisk", s(&["--zap-all", "/dev/sdb"])),
                ("sgdisk", s(&["-n", "1:0:0", "-t", "1:0700", "-c", "1:STICK", "/dev/sdb"])),
                ("partprobe", s(&["/dev/sdb"])),
                ("udevadm", s(&["settle"])),
                ("mkfs.vfat", s(&["-F", "32", "-n", "STICK", "/dev/sdb1"])),
            ]
        );
    }

    #[test]
    fn an_ext4_disk_erase_uses_the_linux_type_and_the_nvme_partition_name() {
        let plan = erase_disk(&disk("nvme0n1", None), &[], Personality::Ext4, "", Scheme::Gpt, &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(plan[1].args, s(&["-n", "1:0:0", "-t", "1:8300", "/dev/nvme0n1"]));
        assert_eq!(plan[4].args.last().map(String::as_str), Some("/dev/nvme0n1p1"));
    }

    #[test]
    fn a_disk_with_a_mounted_partition_is_not_erased() {
        let parts = vec![disk("sdb1", Some("/media/a"))];
        let err = erase_disk(&disk("sdb", None), &parts, Personality::ExFat, "", Scheme::Gpt, &Approvals::new(), &Claims::new(), "disk");
        assert_eq!(err, Err(Refusal::Mounted("/media/a".into())));
    }

    #[test]
    fn another_owners_claim_stops_an_erase() {
        let mut claims = Claims::new();
        claims.claim("sdb", "installer").unwrap();
        let err = erase_disk(&disk("sdb", None), &[], Personality::Ext4, "", Scheme::Gpt, &Approvals::new(), &claims, "disk");
        assert_eq!(err, Err(Refusal::Claimed { device: "sdb".into(), owner: "installer".into() }));
    }

    #[test]
    fn an_approval_gate_vetoes_an_erase_and_passes_other_operations() {
        let mut approvals = Approvals::new();
        approvals.register(|op, _| if op == Operation::Erase { Err("no erasing today".to_string()) } else { Ok(()) });
        let err = erase_volume(&disk("sdb1", None), Personality::Ext4, "", &approvals, &Claims::new(), "disk");
        assert_eq!(err, Err(Refusal::Vetoed("no erasing today".into())));
        assert!(check_volume(&disk("sdb1", None), "ext4", CheckMode::Verify, &approvals, &Claims::new(), "disk").is_ok());
    }

    #[test]
    fn a_device_name_with_a_path_separator_never_reaches_a_command() {
        let err = erase_disk(&disk("../sda", None), &[], Personality::Ext4, "", Scheme::Gpt, &Approvals::new(), &Claims::new(), "disk");
        assert_eq!(err, Err(Refusal::InvalidDevice("../sda".into())));
    }

    #[test]
    fn a_read_only_device_is_not_erased() {
        let mut ro = disk("sr0", None);
        ro.read_only = true;
        assert_eq!(erase_volume(&ro, Personality::Ext4, "", &Approvals::new(), &Claims::new(), "disk"), Err(Refusal::ReadOnly));
    }

    #[test]
    fn first_aid_repairs_or_verifies_with_the_checker_for_the_type() {
        let claims = Claims::new();
        let approvals = Approvals::new();
        let d = disk("sdb1", None);
        let repair = check_volume(&d, "ext4", CheckMode::Repair, &approvals, &claims, "disk").unwrap();
        assert_eq!(words(&repair), vec![("e2fsck", s(&["-p", "/dev/sdb1"]))]);
        let verify = check_volume(&d, "vfat", CheckMode::Verify, &approvals, &claims, "disk").unwrap();
        assert_eq!(words(&verify), vec![("fsck.fat", s(&["-n", "/dev/sdb1"]))]);
        let exfat = check_volume(&d, "exfat", CheckMode::Repair, &approvals, &claims, "disk").unwrap();
        assert_eq!(words(&exfat), vec![("fsck.exfat", s(&["-a", "/dev/sdb1"]))]);
    }

    fn spec(size_mib: u64, fs: Personality, label: &str) -> PartitionSpec {
        PartitionSpec { size_mib, fs, label: label.to_string() }
    }

    #[test]
    fn a_partition_plan_creates_each_partition_then_formats_it() {
        let parts = vec![spec(512, Personality::Fat32, "BOOT"), spec(0, Personality::Ext4, "")];
        let plan = partition_disk(&disk("sdb", None), &parts, &[], &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("sgdisk", s(&["--zap-all", "/dev/sdb"])),
                ("sgdisk", s(&["-n", "1:0:+512M", "-t", "1:0700", "-c", "1:BOOT", "-n", "2:0:0", "-t", "2:8300", "/dev/sdb"])),
                ("partprobe", s(&["/dev/sdb"])),
                ("udevadm", s(&["settle"])),
                ("mkfs.vfat", s(&["-F", "32", "-n", "BOOT", "/dev/sdb1"])),
                ("mkfs.ext4", s(&["-F", "-q", "/dev/sdb2"])),
            ]
        );
    }

    #[test]
    fn a_partition_plan_refuses_a_rest_of_disk_size_that_is_not_last_and_bad_counts() {
        let approvals = Approvals::new();
        let claims = Claims::new();
        let middle_rest = vec![spec(0, Personality::Ext4, ""), spec(64, Personality::Ext4, "")];
        assert!(partition_disk(&disk("sdb", None), &middle_rest, &[], &approvals, &claims, "disk").is_err());
        assert!(partition_disk(&disk("sdb", None), &[], &[], &approvals, &claims, "disk").is_err());
        let too_many = vec![spec(1, Personality::Ext4, ""); MAX_PARTITIONS + 1];
        assert!(partition_disk(&disk("sdb", None), &too_many, &[], &approvals, &claims, "disk").is_err());
    }

    #[test]
    fn a_partition_plan_refuses_a_mounted_partition_and_a_bad_label() {
        let approvals = Approvals::new();
        let claims = Claims::new();
        let parts = vec![spec(0, Personality::Fat32, "OK")];
        let held = vec![disk("sdb1", Some("/media/a"))];
        assert_eq!(
            partition_disk(&disk("sdb", None), &parts, &held, &approvals, &claims, "disk"),
            Err(Refusal::Mounted("/media/a".into()))
        );
        let bad = vec![spec(0, Personality::Fat32, "TOOLONGLABEL")];
        assert_eq!(
            partition_disk(&disk("sdb", None), &bad, &[], &approvals, &claims, "disk"),
            Err(Refusal::InvalidLabel("TOOLONGLABEL".into()))
        );
    }

    #[test]
    fn first_aid_refuses_a_mounted_volume_and_an_unknown_type() {
        let claims = Claims::new();
        let approvals = Approvals::new();
        let mounted = disk("sdb1", Some("/media/a"));
        assert_eq!(
            check_volume(&mounted, "ext4", CheckMode::Repair, &approvals, &claims, "disk"),
            Err(Refusal::Mounted("/media/a".into()))
        );
        assert_eq!(
            check_volume(&disk("sdb1", None), "xfs", CheckMode::Verify, &approvals, &claims, "disk"),
            Err(Refusal::InvalidOption("xfs".into()))
        );
    }

    #[test]
    fn ntfs_and_btrfs_volumes_are_made_checked_and_labelled_with_their_own_tools() {
        let a = Approvals::new();
        let c = Claims::new();
        let ntfs = erase_volume(&disk("sdb1", None), Personality::Ntfs, "DATA", &a, &c, "disk").unwrap();
        assert_eq!(words(&ntfs), vec![("mkfs.ntfs", s(&["-F", "-Q", "-L", "DATA", "/dev/sdb1"]))]);
        let btrfs = erase_volume(&disk("sdb1", None), Personality::Btrfs, "", &a, &c, "disk").unwrap();
        assert_eq!(words(&btrfs), vec![("mkfs.btrfs", s(&["-f", "/dev/sdb1"]))]);
        let verify = check_volume(&disk("sdb1", None), "btrfs", CheckMode::Verify, &a, &c, "disk").unwrap();
        assert_eq!(words(&verify), vec![("btrfs", s(&["check", "--readonly", "/dev/sdb1"]))]);
        let fix = check_volume(&disk("sdb1", None), "ntfs", CheckMode::Repair, &a, &c, "disk").unwrap();
        assert_eq!(words(&fix), vec![("ntfsfix", s(&["-d", "/dev/sdb1"]))]);
        assert_eq!(Personality::Btrfs.label_command("/dev/sdb1", "X").args, s(&["filesystem", "label", "/dev/sdb1", "X"]));
        assert_eq!(Personality::ExFat.label_command("/dev/sdb1", "X").program, "exfatlabel");
        assert_eq!(Personality::from_probe_type("ntfs"), Some(Personality::Ntfs));
        // every file system a popup lists parses back from its word
        for fs in Personality::ALL {
            assert_eq!(Personality::parse(&fs.title().to_lowercase()), Some(fs));
        }
    }

    #[test]
    fn a_dos_table_erase_zaps_the_disk_then_makes_one_primary_partition() {
        let plan = erase_disk(&disk("sdb", None), &[], Personality::ExFat, "KEYS", Scheme::Mbr, &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("sgdisk", s(&["--zap-all", "/dev/sdb"])),
                ("parted", s(&["-s", "/dev/sdb", "mklabel", "msdos"])),
                ("parted", s(&["-s", "-a", "optimal", "/dev/sdb", "mkpart", "primary", "ntfs", "1MiB", "100%"])),
                ("partprobe", s(&["/dev/sdb"])),
                ("udevadm", s(&["settle"])),
                ("mkfs.exfat", s(&["-n", "KEYS", "/dev/sdb1"])),
            ]
        );
        assert_eq!(Scheme::parse("mbr"), Some(Scheme::Mbr));
        assert_eq!(Scheme::parse(Scheme::Gpt.word()), Some(Scheme::Gpt));
        assert_eq!(Scheme::parse("apm"), None);
    }

    #[test]
    fn a_rename_uses_the_label_tool_of_the_volumes_file_system() {
        let a = Approvals::new();
        let c = Claims::new();
        let plan = rename_volume(&disk("sdb1", None), "vfat", "PHOTOS", &a, &c, "disk").unwrap();
        assert_eq!(words(&plan), vec![("fatlabel", s(&["/dev/sdb1", "PHOTOS"]))]);
        assert_eq!(rename_volume(&disk("sdb1", Some("/media/a")), "ext4", "X", &a, &c, "disk"), Err(Refusal::Mounted("/media/a".into())));
        assert_eq!(rename_volume(&disk("sdb1", None), "vfat", "TOO LONG A LABEL", &a, &c, "disk"), Err(Refusal::InvalidLabel("TOO LONG A LABEL".into())));
        assert_eq!(rename_volume(&disk("sdb1", None), "vfat", "", &a, &c, "disk"), Err(Refusal::InvalidLabel(String::new())));
        assert!(matches!(rename_volume(&disk("sdb1", None), "", "X", &a, &c, "disk"), Err(Refusal::Invalid(_))));
        assert_eq!(rename_volume(&disk("sdb1", None), "xfs", "X", &a, &c, "disk"), Err(Refusal::InvalidOption("xfs".into())));
    }

    fn sized(name: &str, size: u64, mount: Option<&str>) -> Disk {
        Disk { size, ..disk(name, mount) }
    }

    #[test]
    fn a_restore_copies_the_source_onto_the_destination_with_dd() {
        let a = Approvals::new();
        let c = Claims::new();
        let plan = restore_volume(&sized("sdc1", 4 << 30, None), &[], &sized("sdb1", 2 << 30, None), &[], &a, &c, "disk").unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("dd", s(&["if=/dev/sdb1", "of=/dev/sdc1", "bs=4M", "conv=fsync", "status=none"])),
                ("partprobe", s(&["/dev/sdc1"])),
                ("udevadm", s(&["settle"])),
            ]
        );
    }

    #[test]
    fn a_restore_refuses_a_mounted_side_a_smaller_destination_and_an_overlap() {
        let a = Approvals::new();
        let c = Claims::new();
        let small = sized("sdc1", 1 << 30, None);
        let big = sized("sdb1", 2 << 30, None);
        assert!(matches!(restore_volume(&small, &[], &big, &[], &a, &c, "disk"), Err(Refusal::Invalid(_))));
        let mounted_dest = sized("sdc1", 4 << 30, Some("/media/c"));
        assert_eq!(restore_volume(&mounted_dest, &[], &big, &[], &a, &c, "disk"), Err(Refusal::Mounted("/media/c".into())));
        let mounted_source = sized("sdb1", 2 << 30, Some("/media/b"));
        assert_eq!(restore_volume(&sized("sdc1", 4 << 30, None), &[], &mounted_source, &[], &a, &c, "disk"), Err(Refusal::Mounted("/media/b".into())));
        let child = sized("sdc2", 1 << 29, Some("/media/c2"));
        assert_eq!(restore_volume(&sized("sdc", 4 << 30, None), &[child], &big, &[], &a, &c, "disk"), Err(Refusal::Mounted("/media/c2".into())));
        let whole = sized("sdb", 8 << 30, None);
        let part = sized("sdb1", 2 << 30, None);
        assert!(matches!(restore_volume(&whole, std::slice::from_ref(&part), &part, &[], &a, &c, "disk"), Err(Refusal::Invalid(_))));
        assert!(matches!(restore_volume(&part, &[], &part, &[], &a, &c, "disk"), Err(Refusal::Invalid(_))));
        let mut ro = sized("sdc1", 4 << 30, None);
        ro.read_only = true;
        assert_eq!(restore_volume(&ro, &[], &big, &[], &a, &c, "disk"), Err(Refusal::ReadOnly));
    }

    fn member(name: &str) -> (Disk, Vec<Disk>) {
        (disk(name, None), Vec::new())
    }

    #[test]
    fn an_array_is_created_over_its_members_with_the_level_flags() {
        let a = Approvals::new();
        let c = Claims::new();
        let members = vec![member("sdb"), member("sdc"), member("sdd")];
        let plan = create_raid("md0", false, RaidLevel::Raid5, &members, &a, &c, "disk").unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("mdadm", s(&["--create", "/dev/md0", "--run", "--level=5", "--raid-devices=3", "/dev/sdb", "/dev/sdc", "/dev/sdd"])),
                ("udevadm", s(&["settle"])),
            ]
        );
    }

    #[test]
    fn an_array_needs_enough_unmounted_distinct_members_and_a_free_name() {
        let a = Approvals::new();
        let c = Claims::new();
        let two = vec![member("sdb"), member("sdc")];
        assert!(matches!(create_raid("md0", false, RaidLevel::Raid5, &two, &a, &c, "disk"), Err(Refusal::Invalid(_))));
        assert!(create_raid("md0", false, RaidLevel::Raid1, &two, &a, &c, "disk").is_ok());
        assert!(matches!(create_raid("md0", true, RaidLevel::Raid1, &two, &a, &c, "disk"), Err(Refusal::Invalid(_))));
        assert_eq!(create_raid("../md0", false, RaidLevel::Raid1, &two, &a, &c, "disk"), Err(Refusal::InvalidDevice("../md0".into())));
        assert_eq!(create_raid("sda", false, RaidLevel::Raid1, &two, &a, &c, "disk"), Err(Refusal::InvalidDevice("sda".into())));
        let held = vec![member("sdb"), (disk("sdc", None), vec![disk("sdc1", Some("/media/c"))])];
        assert_eq!(create_raid("md0", false, RaidLevel::Raid1, &held, &a, &c, "disk"), Err(Refusal::Mounted("/media/c".into())));
        let nested = vec![member("sdb"), (disk("sdc", None), vec![disk("sdb", None)])];
        assert!(matches!(create_raid("md0", false, RaidLevel::Raid1, &nested, &a, &c, "disk"), Err(Refusal::Invalid(_))));
        let twice = vec![member("sdb"), member("sdb")];
        assert!(matches!(create_raid("md0", false, RaidLevel::Raid1, &twice, &a, &c, "disk"), Err(Refusal::Invalid(_))));
    }

    #[test]
    fn raid_levels_parse_and_know_their_minimum_members() {
        for level in RaidLevel::ALL {
            assert_eq!(RaidLevel::parse(level.word()), Some(level));
        }
        assert_eq!(RaidLevel::parse("2"), None);
        assert_eq!(RaidLevel::Raid10.min_members(), 4);
        assert!(is_array_name("md127") && !is_array_name("md") && !is_array_name("md12345") && !is_array_name("sda") && !is_array_name("md1a"));
    }
}
