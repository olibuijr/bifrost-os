//! The privileged disk requests: erase a disk or a volume, partition a disk, first-aid a volume (verify or repair),
//! rename a volume, restore one volume onto another, and create a software RAID array.
//! This is the one contract the privileged helper (`privd`), the `diskutil` tool and the Disk Utility app share: the
//! method names, the parameters on the wire, the confirmation sentence and the plan. A request is planned against a
//! fresh snapshot of the disks (`disk_scan`), and the plan comes from `disk_format`. Nothing here runs a command.

use crate::disk_claims::Claims;
use crate::disk_commands::{Command, Refusal};
use crate::disk_events::{Approvals, Disk};
use crate::disk_format::{self, CheckMode, PartitionSpec, Personality, RaidLevel, Scheme};
use crate::disk_scan::Entry;
use crate::json::Json;

/// The owner the helper claims devices under while it plans a request.
pub const OWNER: &str = "is.olibuijr.diskutil.privd";
pub const M_ERASE_DISK: &str = "disk.erase_disk";
pub const M_ERASE_VOLUME: &str = "disk.erase_volume";
pub const M_PARTITION_DISK: &str = "disk.partition_disk";
pub const M_VERIFY_VOLUME: &str = "disk.verify_volume";
pub const M_REPAIR_VOLUME: &str = "disk.repair_volume";
pub const M_RENAME_VOLUME: &str = "disk.rename_volume";
pub const M_RESTORE_VOLUME: &str = "disk.restore_volume";
pub const M_CREATE_RAID: &str = "disk.create_raid";

/// The most MiB one partition may ask for (1 PiB), so a size cannot overflow the command words.
pub const MAX_SIZE_MIB: u64 = 1 << 30;
/// The longest device name a request may carry.
const MAX_DEVICE_LEN: usize = 32;

/// One privileged request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskRequest {
    /// Replaces the whole partition table (of the given scheme) with one partition that takes the disk, and formats it.
    EraseDisk { device: String, fs: Personality, label: String, scheme: Scheme },
    /// Formats a partition empty. The data on it is lost.
    EraseVolume { device: String, fs: Personality, label: String },
    /// Replaces the partition table with the given partitions, in order, each formatted empty.
    PartitionDisk { device: String, partitions: Vec<PartitionSpec> },
    /// A read-only file system check of an unmounted partition.
    VerifyVolume { device: String },
    /// A file system check that fixes what it finds, of an unmounted partition.
    RepairVolume { device: String },
    /// Sets the label of the file system on an unmounted volume.
    RenameVolume { device: String, label: String },
    /// Copies the volume `source` onto the volume `device`, block for block. The data on `device` is lost.
    RestoreVolume { device: String, source: String },
    /// Creates the software RAID array `name` (such as `md0`) of `level` over the `members`, which are erased.
    CreateRaid { name: String, level: RaidLevel, members: Vec<String> },
}

impl DiskRequest {
    /// The device the request is about.
    pub fn device(&self) -> &str {
        match self {
            DiskRequest::EraseDisk { device, .. }
            | DiskRequest::EraseVolume { device, .. }
            | DiskRequest::PartitionDisk { device, .. }
            | DiskRequest::VerifyVolume { device }
            | DiskRequest::RepairVolume { device }
            | DiskRequest::RenameVolume { device, .. }
            | DiskRequest::RestoreVolume { device, .. } => device.as_str(),
            DiskRequest::CreateRaid { name, .. } => name.as_str(),
        }
    }

    /// The privileged method name this request is sent as.
    pub fn method(&self) -> &'static str {
        match self {
            DiskRequest::EraseDisk { .. } => M_ERASE_DISK,
            DiskRequest::EraseVolume { .. } => M_ERASE_VOLUME,
            DiskRequest::PartitionDisk { .. } => M_PARTITION_DISK,
            DiskRequest::VerifyVolume { .. } => M_VERIFY_VOLUME,
            DiskRequest::RepairVolume { .. } => M_REPAIR_VOLUME,
            DiskRequest::RenameVolume { .. } => M_RENAME_VOLUME,
            DiskRequest::RestoreVolume { .. } => M_RESTORE_VOLUME,
            DiskRequest::CreateRaid { .. } => M_CREATE_RAID,
        }
    }

    /// The progress word shown while the request runs, such as "Erasing".
    pub fn progress_word(&self) -> &'static str {
        match self {
            DiskRequest::EraseDisk { .. } | DiskRequest::EraseVolume { .. } => "Erasing",
            DiskRequest::PartitionDisk { .. } => "Partitioning",
            DiskRequest::VerifyVolume { .. } => "Checking",
            DiskRequest::RepairVolume { .. } => "Repairing",
            DiskRequest::RenameVolume { .. } => "Renaming",
            DiskRequest::RestoreVolume { .. } => "Restoring",
            DiskRequest::CreateRaid { .. } => "Creating array",
        }
    }

    /// The question a confirmation sheet asks before the request runs. Erasing and partitioning name the data loss.
    pub fn confirm_text(&self) -> String {
        match self {
            DiskRequest::EraseDisk { device, fs, label, scheme } => {
                let table = match scheme {
                    Scheme::Gpt => "",
                    Scheme::Mbr => " with a Master Boot Record",
                };
                format!("Erase {device} and format the whole disk as {}{}{table}? All data on it will be lost.", fs_name(*fs), label_clause(label))
            }
            DiskRequest::EraseVolume { device, fs, label } => {
                format!("Erase {device} and format it as {}{}? All data on it will be lost.", fs_name(*fs), label_clause(label))
            }
            DiskRequest::PartitionDisk { device, partitions } => {
                format!("Replace the partitions of {device} with {} new ones? All data on it will be lost.", partitions.len())
            }
            DiskRequest::VerifyVolume { device } => format!("Check {device} for errors without changing it?"),
            DiskRequest::RepairVolume { device } => format!("Check {device} and repair the errors it finds?"),
            DiskRequest::RenameVolume { device, label } => format!("Rename {device} to \u{201c}{label}\u{201d}?"),
            DiskRequest::RestoreVolume { device, source } => {
                format!("Copy {source} onto {device}? Everything on {device} will be replaced and cannot be recovered.")
            }
            DiskRequest::CreateRaid { name, level, members } => {
                format!("Create the {} array {name} from {}? All data on the members will be lost.", level.title(), members.join(", "))
            }
        }
    }

    /// The parameters as the wire carries them: `device` (the array name for `CreateRaid`), and for the other requests
    /// `fs`, `label`, `scheme`, `partitions`, `source`, `level` or `members`.
    pub fn to_params(&self) -> Json {
        let mut fields = vec![("device".to_string(), Json::Str(self.device().to_string()))];
        match self {
            DiskRequest::EraseDisk { fs, label, scheme, .. } => {
                fields.push(("fs".into(), Json::Str(fs_word(*fs).into())));
                fields.push(("label".into(), Json::Str(label.clone())));
                fields.push(("scheme".into(), Json::Str(scheme.word().into())));
            }
            DiskRequest::EraseVolume { fs, label, .. } => {
                fields.push(("fs".into(), Json::Str(fs_word(*fs).into())));
                fields.push(("label".into(), Json::Str(label.clone())));
            }
            DiskRequest::RenameVolume { label, .. } => fields.push(("label".into(), Json::Str(label.clone()))),
            DiskRequest::RestoreVolume { source, .. } => fields.push(("source".into(), Json::Str(source.clone()))),
            DiskRequest::CreateRaid { level, members, .. } => {
                fields.push(("level".into(), Json::Str(level.word().into())));
                fields.push(("members".into(), Json::Arr(members.iter().map(|m| Json::Str(m.clone())).collect())));
            }
            DiskRequest::PartitionDisk { partitions, .. } => {
                let specs = partitions
                    .iter()
                    .map(|p| {
                        Json::Obj(vec![
                            ("size_mib".into(), Json::Num(p.size_mib as f64)),
                            ("fs".into(), Json::Str(fs_word(p.fs).into())),
                            ("label".into(), Json::Str(p.label.clone())),
                        ])
                    })
                    .collect();
                fields.push(("partitions".into(), Json::Arr(specs)));
            }
            DiskRequest::VerifyVolume { .. } | DiskRequest::RepairVolume { .. } => {}
        }
        Json::Obj(fields)
    }

    /// Reads a request from its method name and parameters. Every value is checked for its shape here; the names and
    /// the sizes of the disks are checked when the request is planned.
    pub fn from_wire(method: &str, params: &Json) -> Result<DiskRequest, String> {
        let device = device_param(params)?;
        let text = |key: &str| -> Result<String, String> {
            params.get(key).and_then(Json::as_str).map(str::to_string).ok_or_else(|| format!("missing {key}"))
        };
        let fs = || -> Result<Personality, String> {
            let word = text("fs")?;
            Personality::parse(&word).ok_or_else(|| format!("unknown file system {word}"))
        };
        match method {
            M_ERASE_DISK => Ok(DiskRequest::EraseDisk { device, fs: fs()?, label: label_param(params)?, scheme: scheme_param(params)? }),
            M_ERASE_VOLUME => Ok(DiskRequest::EraseVolume { device, fs: fs()?, label: label_param(params)? }),
            M_PARTITION_DISK => {
                let list = params.get("partitions").and_then(Json::as_array).ok_or("missing partitions")?;
                let partitions = list.iter().map(partition_spec).collect::<Result<Vec<_>, _>>()?;
                Ok(DiskRequest::PartitionDisk { device, partitions })
            }
            M_VERIFY_VOLUME => Ok(DiskRequest::VerifyVolume { device }),
            M_REPAIR_VOLUME => Ok(DiskRequest::RepairVolume { device }),
            M_RENAME_VOLUME => Ok(DiskRequest::RenameVolume { device, label: text("label")? }),
            M_RESTORE_VOLUME => {
                let source = name_word(&text("source")?)?;
                Ok(DiskRequest::RestoreVolume { device, source })
            }
            M_CREATE_RAID => {
                let word = text("level")?;
                let level = RaidLevel::parse(&word).ok_or_else(|| format!("unknown RAID level {word}"))?;
                let list = params.get("members").and_then(Json::as_array).ok_or("missing members")?;
                if list.len() > disk_format::MAX_RAID_MEMBERS {
                    return Err("too many members".to_string());
                }
                let members = list.iter().map(|m| m.as_str().ok_or_else(|| "a member must be a device name".to_string()).and_then(name_word)).collect::<Result<Vec<_>, _>>()?;
                Ok(DiskRequest::CreateRaid { name: device, level, members })
            }
            other => Err(format!("unknown disk method {other}")),
        }
    }
}

/// The plan for `request` against the disks in `entries` (a fresh snapshot). `probed_type` is the file system type
/// the volume reports (`blkid` on the device); only the first-aid and rename requests read it. Refuses like
/// `disk_format` does, and also when the device is missing, or is a partition where a whole disk is needed (or the
/// reverse).
pub fn plan(request: &DiskRequest, entries: &[Entry], probed_type: &str) -> Result<Vec<Command>, Refusal> {
    let (approvals, claims) = (Approvals::new(), Claims::new());
    if let DiskRequest::CreateRaid { name, level, members } = request {
        let mut chosen = Vec::with_capacity(members.len());
        for member in members {
            let entry = entries.iter().find(|e| &e.disk.bsd_name == member).ok_or_else(|| Refusal::NoDisk(member.clone()))?;
            chosen.push((entry.disk.clone(), children_of(entries, member)));
        }
        let exists = entries.iter().any(|e| &e.disk.bsd_name == name);
        return disk_format::create_raid(name, exists, *level, &chosen, &approvals, &claims, OWNER);
    }
    let name = request.device();
    let entry = entries.iter().find(|e| e.disk.bsd_name == name).ok_or_else(|| Refusal::NoDisk(name.to_string()))?;
    let disk: &Disk = &entry.disk;
    let children = children_of(entries, name);
    let whole = || -> Result<(), Refusal> {
        if entry.parent.is_some() { Err(Refusal::NotWhole(name.to_string())) } else { Ok(()) }
    };
    let partition = || -> Result<(), Refusal> {
        if entry.parent.is_none() { Err(Refusal::NotPartition(name.to_string())) } else { Ok(()) }
    };
    match request {
        DiskRequest::EraseDisk { fs, label, scheme, .. } => {
            whole()?;
            disk_format::erase_disk(disk, &children, *fs, label, *scheme, &approvals, &claims, OWNER)
        }
        DiskRequest::EraseVolume { fs, label, .. } => {
            partition()?;
            disk_format::erase_volume(disk, *fs, label, &approvals, &claims, OWNER)
        }
        DiskRequest::PartitionDisk { partitions, .. } => {
            whole()?;
            disk_format::partition_disk(disk, partitions, &children, &approvals, &claims, OWNER)
        }
        DiskRequest::VerifyVolume { .. } | DiskRequest::RepairVolume { .. } => {
            partition()?;
            let mode = if matches!(request, DiskRequest::RepairVolume { .. }) { CheckMode::Repair } else { CheckMode::Verify };
            disk_format::check_volume(disk, probed_type, mode, &approvals, &claims, OWNER)
        }
        DiskRequest::RenameVolume { label, .. } => disk_format::rename_volume(disk, probed_type, label, &approvals, &claims, OWNER),
        DiskRequest::RestoreVolume { source, .. } => {
            let from = entries.iter().find(|e| &e.disk.bsd_name == source).ok_or_else(|| Refusal::NoDisk(source.clone()))?;
            let from_children = children_of(entries, source);
            disk_format::restore_volume(disk, &children, &from.disk, &from_children, &approvals, &claims, OWNER)
        }
        // an array request is planned above, before the device lookup; it never gets here
        DiskRequest::CreateRaid { name, .. } => Err(Refusal::NoDisk(name.clone())),
    }
}

/// The disks that are partitions of `name`.
fn children_of(entries: &[Entry], name: &str) -> Vec<Disk> {
    entries.iter().filter(|e| e.parent.as_deref() == Some(name)).map(|e| e.disk.clone()).collect()
}

/// Whether the device is repaired by a checker that exits 1 on a successful fix (see `disk_run::run_plan`).
pub fn is_repair(request: &DiskRequest) -> bool {
    matches!(request, DiskRequest::RepairVolume { .. })
}

fn fs_word(fs: Personality) -> &'static str {
    match fs {
        Personality::Ext4 => "ext4",
        Personality::ExFat => "exfat",
        Personality::Fat32 => "fat32",
        Personality::Ntfs => "ntfs",
        Personality::Btrfs => "btrfs",
    }
}

fn fs_name(fs: Personality) -> &'static str {
    match fs {
        Personality::Ext4 => "Ext4",
        Personality::ExFat => "ExFAT",
        Personality::Fat32 => "FAT32",
        Personality::Ntfs => "NTFS",
        Personality::Btrfs => "Btrfs",
    }
}

fn label_clause(label: &str) -> String {
    if label.is_empty() { String::new() } else { format!(" named \u{201c}{label}\u{201d}") }
}

/// A device name is a plain alphanumeric word, so it names a path under `/dev` and nothing else.
fn device_param(params: &Json) -> Result<String, String> {
    let name = params.get("device").and_then(Json::as_str).ok_or("missing device")?;
    name_word(name)
}

/// Checks that `name` is a plain device name.
fn name_word(name: &str) -> Result<String, String> {
    if name.is_empty() || name.len() > MAX_DEVICE_LEN || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(format!("{name} is not a device name"));
    }
    Ok(name.to_string())
}

/// The partition scheme of an erase; a request that names none (an older client) is a GPT erase.
fn scheme_param(params: &Json) -> Result<Scheme, String> {
    match params.get("scheme") {
        None | Some(Json::Null) => Ok(Scheme::Gpt),
        Some(Json::Str(word)) => Scheme::parse(word).ok_or_else(|| format!("unknown partition scheme {word}")),
        Some(_) => Err("scheme must be text".to_string()),
    }
}

fn label_param(params: &Json) -> Result<String, String> {
    match params.get("label") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Str(label)) => Ok(label.clone()),
        Some(_) => Err("label must be text".to_string()),
    }
}

fn partition_spec(p: &Json) -> Result<PartitionSpec, String> {
    let size = p.get("size_mib").and_then(Json::number).filter(|n| n.fract() == 0.0 && *n >= 0.0).ok_or("size_mib must be a whole number")?;
    if size > MAX_SIZE_MIB as f64 {
        return Err("size_mib is too large".to_string());
    }
    let size_mib = size as u64;
    let word = p.get("fs").and_then(Json::as_str).ok_or("missing fs")?;
    let fs = Personality::parse(word).ok_or_else(|| format!("unknown file system {word}"))?;
    Ok(PartitionSpec { size_mib, fs, label: label_param(p)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, parent: Option<&str>, mount: Option<&str>) -> Entry {
        Entry {
            disk: Disk { bsd_name: name.into(), size: 8 << 30, removable: true, read_only: false, model: "Stick".into(), mount_point: mount.map(String::from) },
            parent: parent.map(String::from),
        }
    }

    fn snapshot() -> Vec<Entry> {
        vec![entry("sdb", None, None), entry("sdb1", Some("sdb"), None), entry("sdc", None, None), entry("sdc1", Some("sdc"), Some("/media/c"))]
    }

    fn words(plan: &[Command]) -> Vec<(&'static str, Vec<String>)> {
        plan.iter().map(|c| (c.program, c.args.clone())).collect()
    }

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn text(v: &str) -> Json {
        Json::Str(v.into())
    }

    #[test]
    fn requests_round_trip_through_their_parameters() {
        let requests = vec![
            DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into(), scheme: Scheme::Gpt },
            DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Ntfs, label: String::new(), scheme: Scheme::Mbr },
            DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::Btrfs, label: String::new() },
            DiskRequest::RenameVolume { device: "sdb1".into(), label: "PHOTOS".into() },
            DiskRequest::RestoreVolume { device: "sdc1".into(), source: "sdb1".into() },
            DiskRequest::CreateRaid { name: "md0".into(), level: RaidLevel::Raid5, members: vec!["sdb".into(), "sdc".into(), "sdd".into()] },
            DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::Ext4, label: String::new() },
            DiskRequest::PartitionDisk {
                device: "sdb".into(),
                partitions: vec![PartitionSpec { size_mib: 512, fs: Personality::Fat32, label: "BOOT".into() }, PartitionSpec { size_mib: 0, fs: Personality::Ext4, label: String::new() }],
            },
            DiskRequest::VerifyVolume { device: "sdb1".into() },
            DiskRequest::RepairVolume { device: "sdb1".into() },
        ];
        for r in requests {
            assert_eq!(DiskRequest::from_wire(r.method(), &r.to_params()), Ok(r.clone()), "{}", r.method());
        }
    }

    #[test]
    fn a_device_name_with_a_slash_is_refused_before_any_plan() {
        let p = obj(vec![("device", text("../sda")), ("fs", text("ext4"))]);
        assert!(DiskRequest::from_wire(M_ERASE_DISK, &p).unwrap_err().contains("not a device name"));
        assert!(DiskRequest::from_wire(M_VERIFY_VOLUME, &obj(vec![])).unwrap_err().contains("missing device"));
    }

    #[test]
    fn bad_file_systems_and_partition_sizes_are_refused() {
        let bad_fs = obj(vec![("device", text("sdb")), ("fs", text("xfs"))]);
        assert!(DiskRequest::from_wire(M_ERASE_DISK, &bad_fs).unwrap_err().contains("unknown file system"));
        let huge = obj(vec![("device", text("sdb")), ("partitions", Json::Arr(vec![obj(vec![("size_mib", Json::Num(2e15)), ("fs", text("ext4"))])]))]);
        assert!(DiskRequest::from_wire(M_PARTITION_DISK, &huge).unwrap_err().contains("too large"));
        let fraction = obj(vec![("device", text("sdb")), ("partitions", Json::Arr(vec![obj(vec![("size_mib", Json::Num(1.5)), ("fs", text("ext4"))])]))]);
        assert!(DiskRequest::from_wire(M_PARTITION_DISK, &fraction).is_err());
    }

    #[test]
    fn an_erase_of_a_whole_disk_plans_from_the_snapshot() {
        let r = DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into(), scheme: Scheme::Gpt };
        let plan = plan(&r, &snapshot(), "").unwrap();
        assert_eq!(plan.last().map(|c| c.program), Some("mkfs.vfat"));
        assert_eq!(plan.last().unwrap().args.last().map(String::as_str), Some("/dev/sdb1"));
    }

    #[test]
    fn a_whole_disk_request_on_a_partition_is_refused_and_the_reverse() {
        let erase_part = DiskRequest::EraseDisk { device: "sdb1".into(), fs: Personality::Ext4, label: String::new(), scheme: Scheme::Gpt };
        assert_eq!(plan(&erase_part, &snapshot(), ""), Err(Refusal::NotWhole("sdb1".into())));
        let format_disk = DiskRequest::EraseVolume { device: "sdb".into(), fs: Personality::Ext4, label: String::new(), scheme: Scheme::Gpt };
        assert_eq!(plan(&format_disk, &snapshot(), ""), Err(Refusal::NotPartition("sdb".into())));
    }

    #[test]
    fn a_mounted_partition_stops_an_erase_of_its_disk() {
        let r = DiskRequest::EraseDisk { device: "sdc".into(), fs: Personality::Fat32, label: String::new(), scheme: Scheme::Gpt };
        assert_eq!(plan(&r, &snapshot(), ""), Err(Refusal::Mounted("/media/c".into())));
    }

    #[test]
    fn a_missing_device_is_refused() {
        let r = DiskRequest::VerifyVolume { device: "sdz1".into() };
        assert_eq!(plan(&r, &snapshot(), "ext4"), Err(Refusal::NoDisk("sdz1".into())));
    }

    #[test]
    fn first_aid_uses_the_probed_type_and_the_repair_flag() {
        let verify = DiskRequest::VerifyVolume { device: "sdb1".into() };
        assert_eq!(words(&plan(&verify, &snapshot(), "vfat").unwrap()), vec![("fsck.fat", vec!["-n".to_string(), "/dev/sdb1".to_string()])]);
        let repair = DiskRequest::RepairVolume { device: "sdb1".into() };
        assert_eq!(words(&plan(&repair, &snapshot(), "ext4").unwrap()), vec![("e2fsck", vec!["-p".to_string(), "/dev/sdb1".to_string()])]);
        assert!(is_repair(&repair) && !is_repair(&verify));
        assert!(plan(&repair, &snapshot(), "xfs").is_err());
    }

    #[test]
    fn the_confirmation_names_the_device_and_the_data_loss() {
        let r = DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into(), scheme: Scheme::Gpt };
        let text = r.confirm_text();
        assert!(text.contains("sdb") && text.contains("FAT32") && text.contains("STICK") && text.contains("lost"), "{text}");
        assert_eq!(r.progress_word(), "Erasing");
        assert!(DiskRequest::RepairVolume { device: "sdb1".into() }.confirm_text().contains("repair"));
    }

    #[test]
    fn an_erase_names_a_dos_table_and_a_missing_scheme_means_gpt() {
        let mbr = DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Ntfs, label: String::new(), scheme: Scheme::Mbr };
        assert!(mbr.confirm_text().contains("NTFS") && mbr.confirm_text().contains("Master Boot Record"));
        let old = obj(vec![("device", text("sdb")), ("fs", text("ext4"))]);
        assert_eq!(
            DiskRequest::from_wire(M_ERASE_DISK, &old),
            Ok(DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Ext4, label: String::new(), scheme: Scheme::Gpt })
        );
        let bad = obj(vec![("device", text("sdb")), ("fs", text("ext4")), ("scheme", text("apm"))]);
        assert!(DiskRequest::from_wire(M_ERASE_DISK, &bad).unwrap_err().contains("unknown partition scheme"));
    }

    #[test]
    fn a_rename_plans_the_label_tool_from_the_probed_type() {
        let r = DiskRequest::RenameVolume { device: "sdb1".into(), label: "PHOTOS".into() };
        let plan = plan(&r, &snapshot(), "ext4").unwrap();
        assert_eq!(words(&plan), vec![("e2label", vec!["/dev/sdb1".to_string(), "PHOTOS".to_string()])]);
        assert!(super::plan(&r, &snapshot(), "").is_err());
        let held = DiskRequest::RenameVolume { device: "sdc1".into(), label: "X".into() };
        assert_eq!(super::plan(&held, &snapshot(), "ext4"), Err(Refusal::Mounted("/media/c".into())));
        assert!(r.confirm_text().contains("PHOTOS") && r.progress_word() == "Renaming");
    }

    #[test]
    fn a_restore_needs_both_volumes_to_be_connected() {
        let r = DiskRequest::RestoreVolume { device: "sdb1".into(), source: "sdz1".into() };
        assert_eq!(plan(&r, &snapshot(), ""), Err(Refusal::NoDisk("sdz1".into())));
        let gone = DiskRequest::RestoreVolume { device: "sdz1".into(), source: "sdb1".into() };
        assert_eq!(plan(&gone, &snapshot(), ""), Err(Refusal::NoDisk("sdz1".into())));
        let same_disk = DiskRequest::RestoreVolume { device: "sdb".into(), source: "sdb1".into() };
        assert!(matches!(plan(&same_disk, &snapshot(), ""), Err(Refusal::Invalid(_))));
        let ok = DiskRequest::RestoreVolume { device: "sdb1".into(), source: "sdb".into() };
        assert!(matches!(plan(&ok, &snapshot(), ""), Err(Refusal::Invalid(_))));
        assert!(DiskRequest::RestoreVolume { device: "sdb1".into(), source: "sdc".into() }.confirm_text().contains("replaced"));
    }

    #[test]
    fn an_array_is_planned_from_the_snapshot_and_a_taken_name_is_refused() {
        let members = vec!["sdb".to_string(), "sdc".to_string()];
        let r = DiskRequest::CreateRaid { name: "md0".into(), level: RaidLevel::Raid1, members: members.clone() };
        assert_eq!(plan(&r, &snapshot(), ""), Err(Refusal::Mounted("/media/c".into())));
        let only_b = vec![entry("sdb", None, None), entry("sdc", None, None), entry("md0", None, None)];
        assert!(matches!(plan(&r, &only_b, ""), Err(Refusal::Invalid(_))));
        let fresh = vec![entry("sdb", None, None), entry("sdc", None, None)];
        let made = plan(&r, &fresh, "").unwrap();
        assert_eq!(made[0].program, "mdadm");
        let missing = DiskRequest::CreateRaid { name: "md0".into(), level: RaidLevel::Raid1, members: vec!["sdb".into(), "sdq".into()] };
        assert_eq!(plan(&missing, &fresh, ""), Err(Refusal::NoDisk("sdq".into())));
        assert!(r.confirm_text().contains("RAID 1") && r.confirm_text().contains("sdb, sdc") && r.device() == "md0");
    }

    #[test]
    fn array_requests_are_checked_for_shape_on_the_wire() {
        let bad_level = obj(vec![("device", text("md0")), ("level", text("2")), ("members", Json::Arr(vec![]))]);
        assert!(DiskRequest::from_wire(M_CREATE_RAID, &bad_level).unwrap_err().contains("unknown RAID level"));
        let bad_member = obj(vec![("device", text("md0")), ("level", text("1")), ("members", Json::Arr(vec![text("../sda")]))]);
        assert!(DiskRequest::from_wire(M_CREATE_RAID, &bad_member).unwrap_err().contains("not a device name"));
        let no_source = obj(vec![("device", text("sdb1"))]);
        assert!(DiskRequest::from_wire(M_RESTORE_VOLUME, &no_source).unwrap_err().contains("missing source"));
        let slash = obj(vec![("device", text("sdb1")), ("source", text("../sda"))]);
        assert!(DiskRequest::from_wire(M_RESTORE_VOLUME, &slash).unwrap_err().contains("not a device name"));
    }
}
