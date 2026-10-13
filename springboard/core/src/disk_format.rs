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
}

/// Verify is read-only; repair fixes what it finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Verify,
    Repair,
}

impl Personality {
    /// The name the user picks: `ext4`, `exfat` or `fat32`.
    pub fn parse(text: &str) -> Option<Personality> {
        match text {
            "ext4" => Some(Personality::Ext4),
            "exfat" => Some(Personality::ExFat),
            "fat32" => Some(Personality::Fat32),
            _ => None,
        }
    }

    /// The file system type the kernel and the udev database report for a volume (`ID_FS_TYPE`, `/proc/mounts`).
    pub fn from_probe_type(text: &str) -> Option<Personality> {
        match text {
            "ext4" => Some(Personality::Ext4),
            "exfat" => Some(Personality::ExFat),
            "vfat" => Some(Personality::Fat32),
            _ => None,
        }
    }

    /// The longest label the file system stores, in bytes.
    pub fn label_limit(self) -> usize {
        match self {
            Personality::Ext4 => 16,
            Personality::ExFat => 15,
            Personality::Fat32 => 11,
        }
    }

    /// The GPT partition type code for a new partition of this file system (Linux data, or Microsoft basic data).
    fn gpt_code(self) -> &'static str {
        match self {
            Personality::Ext4 => "8300",
            Personality::ExFat | Personality::Fat32 => "0700",
        }
    }

    /// The command that writes an empty file system on `dev`. An empty label leaves the volume unlabelled.
    fn make_command(self, dev: &str, label: &str) -> Command {
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
        };
        args.push(dev.to_string());
        Command { program, args }
    }

    /// The check command for `dev`: verify is a read-only pass, repair fixes errors without asking.
    fn check_command(self, dev: &str, mode: CheckMode) -> Command {
        let (program, flag) = match (self, mode) {
            (Personality::Ext4, CheckMode::Verify) => ("e2fsck", "-n"),
            (Personality::Ext4, CheckMode::Repair) => ("e2fsck", "-p"),
            (Personality::ExFat, CheckMode::Verify) => ("fsck.exfat", "-n"),
            (Personality::ExFat, CheckMode::Repair) => ("fsck.exfat", "-a"),
            (Personality::Fat32, CheckMode::Verify) => ("fsck.fat", "-n"),
            (Personality::Fat32, CheckMode::Repair) => ("fsck.fat", "-a"),
        };
        Command { program, args: vec![flag.to_string(), dev.to_string()] }
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

/// Plans an erase of a whole disk: the partition table is replaced by one GPT partition that takes the whole disk,
/// which is then formatted. `partitions` are the disk's own partitions; any of them mounted refuses the erase.
pub fn erase_disk(
    disk: &Disk,
    partitions: &[Disk],
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
    let mounted = disk.mount_point.iter().chain(partitions.iter().filter_map(|p| p.mount_point.as_ref())).next();
    if let Some(point) = mounted {
        return Err(Refusal::Mounted(point.clone()));
    }
    check_label(label, fs)?;
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
        assert_eq!(Personality::parse("ntfs"), None);
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
        let plan = erase_disk(&disk("sdb", None), &[disk("sdb1", None)], Personality::Fat32, "STICK", &Approvals::new(), &Claims::new(), "disk").unwrap();
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
        let plan = erase_disk(&disk("nvme0n1", None), &[], Personality::Ext4, "", &Approvals::new(), &Claims::new(), "disk").unwrap();
        assert_eq!(plan[1].args, s(&["-n", "1:0:0", "-t", "1:8300", "/dev/nvme0n1"]));
        assert_eq!(plan[4].args.last().map(String::as_str), Some("/dev/nvme0n1p1"));
    }

    #[test]
    fn a_disk_with_a_mounted_partition_is_not_erased() {
        let parts = vec![disk("sdb1", Some("/media/a"))];
        let err = erase_disk(&disk("sdb", None), &parts, Personality::ExFat, "", &Approvals::new(), &Claims::new(), "disk");
        assert_eq!(err, Err(Refusal::Mounted("/media/a".into())));
    }

    #[test]
    fn another_owners_claim_stops_an_erase() {
        let mut claims = Claims::new();
        claims.claim("sdb", "installer").unwrap();
        let err = erase_disk(&disk("sdb", None), &[], Personality::Ext4, "", &Approvals::new(), &claims, "disk");
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
        let err = erase_disk(&disk("../sda", None), &[], Personality::Ext4, "", &Approvals::new(), &Claims::new(), "disk");
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
            check_volume(&disk("sdb1", None), "ntfs", CheckMode::Verify, &approvals, &claims, "disk"),
            Err(Refusal::InvalidOption("ntfs".into()))
        );
    }
}
