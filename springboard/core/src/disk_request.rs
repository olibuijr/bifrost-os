//! The privileged disk requests: erase a disk or a volume, partition a disk, and first-aid a volume (verify or repair).
//! This is the one contract the privileged helper (`privd`), the `diskutil` tool and the Disk Utility app share: the
//! method names, the parameters on the wire, the confirmation sentence and the plan. A request is planned against a
//! fresh snapshot of the disks (`disk_scan`), and the plan comes from `disk_format`. Nothing here runs a command.

use crate::disk_claims::Claims;
use crate::disk_commands::{Command, Refusal};
use crate::disk_events::{Approvals, Disk};
use crate::disk_format::{self, CheckMode, PartitionSpec, Personality};
use crate::disk_scan::Entry;
use crate::json::Json;

/// The owner the helper claims devices under while it plans a request.
pub const OWNER: &str = "is.olibuijr.diskutil.privd";
pub const M_ERASE_DISK: &str = "disk.erase_disk";
pub const M_ERASE_VOLUME: &str = "disk.erase_volume";
pub const M_PARTITION_DISK: &str = "disk.partition_disk";
pub const M_VERIFY_VOLUME: &str = "disk.verify_volume";
pub const M_REPAIR_VOLUME: &str = "disk.repair_volume";

/// The most MiB one partition may ask for (1 PiB), so a size cannot overflow the command words.
pub const MAX_SIZE_MIB: u64 = 1 << 30;
/// The longest device name a request may carry.
const MAX_DEVICE_LEN: usize = 32;

/// One privileged request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskRequest {
    /// Replaces the whole partition table with one partition that takes the disk, and formats it.
    EraseDisk { device: String, fs: Personality, label: String },
    /// Formats a partition empty. The data on it is lost.
    EraseVolume { device: String, fs: Personality, label: String },
    /// Replaces the partition table with the given partitions, in order, each formatted empty.
    PartitionDisk { device: String, partitions: Vec<PartitionSpec> },
    /// A read-only file system check of an unmounted partition.
    VerifyVolume { device: String },
    /// A file system check that fixes what it finds, of an unmounted partition.
    RepairVolume { device: String },
}

impl DiskRequest {
    /// The device the request is about.
    pub fn device(&self) -> &str {
        match self {
            DiskRequest::EraseDisk { device, .. }
            | DiskRequest::EraseVolume { device, .. }
            | DiskRequest::PartitionDisk { device, .. }
            | DiskRequest::VerifyVolume { device }
            | DiskRequest::RepairVolume { device } => device.as_str(),
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
        }
    }

    /// The progress word shown while the request runs, such as "Erasing".
    pub fn progress_word(&self) -> &'static str {
        match self {
            DiskRequest::EraseDisk { .. } | DiskRequest::EraseVolume { .. } => "Erasing",
            DiskRequest::PartitionDisk { .. } => "Partitioning",
            DiskRequest::VerifyVolume { .. } => "Checking",
            DiskRequest::RepairVolume { .. } => "Repairing",
        }
    }

    /// The question a confirmation sheet asks before the request runs. Erasing and partitioning name the data loss.
    pub fn confirm_text(&self) -> String {
        match self {
            DiskRequest::EraseDisk { device, fs, label } => {
                format!("Erase {device} and format the whole disk as {}{}? All data on it will be lost.", fs_name(*fs), label_clause(label))
            }
            DiskRequest::EraseVolume { device, fs, label } => {
                format!("Erase {device} and format it as {}{}? All data on it will be lost.", fs_name(*fs), label_clause(label))
            }
            DiskRequest::PartitionDisk { device, partitions } => {
                format!("Replace the partitions of {device} with {} new ones? All data on it will be lost.", partitions.len())
            }
            DiskRequest::VerifyVolume { device } => format!("Check {device} for errors without changing it?"),
            DiskRequest::RepairVolume { device } => format!("Check {device} and repair the errors it finds?"),
        }
    }

    /// The parameters as the wire carries them: `device`, and for the erase and partition requests `fs`, `label` or
    /// `partitions`.
    pub fn to_params(&self) -> Json {
        let mut fields = vec![("device".to_string(), Json::Str(self.device().to_string()))];
        match self {
            DiskRequest::EraseDisk { fs, label, .. } | DiskRequest::EraseVolume { fs, label, .. } => {
                fields.push(("fs".into(), Json::Str(fs_word(*fs).into())));
                fields.push(("label".into(), Json::Str(label.clone())));
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
            M_ERASE_DISK => Ok(DiskRequest::EraseDisk { device, fs: fs()?, label: label_param(params)? }),
            M_ERASE_VOLUME => Ok(DiskRequest::EraseVolume { device, fs: fs()?, label: label_param(params)? }),
            M_PARTITION_DISK => {
                let list = params.get("partitions").and_then(Json::as_array).ok_or("missing partitions")?;
                let partitions = list.iter().map(partition_spec).collect::<Result<Vec<_>, _>>()?;
                Ok(DiskRequest::PartitionDisk { device, partitions })
            }
            M_VERIFY_VOLUME => Ok(DiskRequest::VerifyVolume { device }),
            M_REPAIR_VOLUME => Ok(DiskRequest::RepairVolume { device }),
            other => Err(format!("unknown disk method {other}")),
        }
    }
}

/// The plan for `request` against the disks in `entries` (a fresh snapshot). `probed_type` is the file system type
/// the volume reports (`blkid` on the device); only the first-aid requests read it. Refuses like `disk_format` does,
/// and also when the device is missing, or is a partition where a whole disk is needed (or the reverse).
pub fn plan(request: &DiskRequest, entries: &[Entry], probed_type: &str) -> Result<Vec<Command>, Refusal> {
    let name = request.device();
    let entry = entries.iter().find(|e| e.disk.bsd_name == name).ok_or_else(|| Refusal::NoDisk(name.to_string()))?;
    let disk: &Disk = &entry.disk;
    let children: Vec<Disk> = entries.iter().filter(|e| e.parent.as_deref() == Some(name)).map(|e| e.disk.clone()).collect();
    let (approvals, claims) = (Approvals::new(), Claims::new());
    let whole = || -> Result<(), Refusal> {
        if entry.parent.is_some() { Err(Refusal::NotWhole(name.to_string())) } else { Ok(()) }
    };
    let partition = || -> Result<(), Refusal> {
        if entry.parent.is_none() { Err(Refusal::NotPartition(name.to_string())) } else { Ok(()) }
    };
    match request {
        DiskRequest::EraseDisk { fs, label, .. } => {
            whole()?;
            disk_format::erase_disk(disk, &children, *fs, label, &approvals, &claims, OWNER)
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
    }
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
    }
}

fn fs_name(fs: Personality) -> &'static str {
    match fs {
        Personality::Ext4 => "Ext4",
        Personality::ExFat => "ExFAT",
        Personality::Fat32 => "FAT32",
    }
}

fn label_clause(label: &str) -> String {
    if label.is_empty() { String::new() } else { format!(" named \u{201c}{label}\u{201d}") }
}

/// A device name is a plain alphanumeric word, so it names a path under `/dev` and nothing else.
fn device_param(params: &Json) -> Result<String, String> {
    let name = params.get("device").and_then(Json::as_str).ok_or("missing device")?;
    if name.is_empty() || name.len() > MAX_DEVICE_LEN || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(format!("{name} is not a device name"));
    }
    Ok(name.to_string())
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
            DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into() },
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
        let bad_fs = obj(vec![("device", text("sdb")), ("fs", text("ntfs"))]);
        assert!(DiskRequest::from_wire(M_ERASE_DISK, &bad_fs).unwrap_err().contains("unknown file system"));
        let huge = obj(vec![("device", text("sdb")), ("partitions", Json::Arr(vec![obj(vec![("size_mib", Json::Num(2e15)), ("fs", text("ext4"))])]))]);
        assert!(DiskRequest::from_wire(M_PARTITION_DISK, &huge).unwrap_err().contains("too large"));
        let fraction = obj(vec![("device", text("sdb")), ("partitions", Json::Arr(vec![obj(vec![("size_mib", Json::Num(1.5)), ("fs", text("ext4"))])]))]);
        assert!(DiskRequest::from_wire(M_PARTITION_DISK, &fraction).is_err());
    }

    #[test]
    fn an_erase_of_a_whole_disk_plans_from_the_snapshot() {
        let r = DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into() };
        let plan = plan(&r, &snapshot(), "").unwrap();
        assert_eq!(plan.last().map(|c| c.program), Some("mkfs.vfat"));
        assert_eq!(plan.last().unwrap().args.last().map(String::as_str), Some("/dev/sdb1"));
    }

    #[test]
    fn a_whole_disk_request_on_a_partition_is_refused_and_the_reverse() {
        let erase_part = DiskRequest::EraseDisk { device: "sdb1".into(), fs: Personality::Ext4, label: String::new() };
        assert_eq!(plan(&erase_part, &snapshot(), ""), Err(Refusal::NotWhole("sdb1".into())));
        let format_disk = DiskRequest::EraseVolume { device: "sdb".into(), fs: Personality::Ext4, label: String::new() };
        assert_eq!(plan(&format_disk, &snapshot(), ""), Err(Refusal::NotPartition("sdb".into())));
    }

    #[test]
    fn a_mounted_partition_stops_an_erase_of_its_disk() {
        let r = DiskRequest::EraseDisk { device: "sdc".into(), fs: Personality::Fat32, label: String::new() };
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
        assert!(plan(&repair, &snapshot(), "ntfs").is_err());
    }

    #[test]
    fn the_confirmation_names_the_device_and_the_data_loss() {
        let r = DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into() };
        let text = r.confirm_text();
        assert!(text.contains("sdb") && text.contains("FAT32") && text.contains("STICK") && text.contains("lost"), "{text}");
        assert_eq!(r.progress_word(), "Erasing");
        assert!(DiskRequest::RepairVolume { device: "sdb1".into() }.confirm_text().contains("repair"));
    }
}
