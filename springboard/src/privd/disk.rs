//! The privileged disk requests (`disk.*`): erase a disk or a volume, partition a disk, and verify or repair a
//! volume. The request is read and checked by `bifrost_core::disk_request`, planned against a fresh snapshot of the
//! disks taken here (so a stale client view never reaches a command), and the plan runs as root with fixed argv.
//! Like every method other than the unlock ones, it needs the unlocked session's token (see `server::dispatch`).

use super::{obj, s, PErr};
use bifrost_core::disk_request::{self, DiskRequest};
use bifrost_core::disk_run;
use bifrost_core::disk_scan;
use bifrost_core::json::Json;
use std::path::Path;
use std::process::Command;

/// The methods this module serves.
pub fn handles(method: &str) -> bool {
    matches!(
        method,
        disk_request::M_ERASE_DISK | disk_request::M_ERASE_VOLUME | disk_request::M_PARTITION_DISK | disk_request::M_VERIFY_VOLUME | disk_request::M_REPAIR_VOLUME
    )
}

/// Runs one disk request. The reply carries the device and the tools that completed, in order.
pub fn call(method: &str, params: &Json) -> Result<Json, PErr> {
    let request = DiskRequest::from_wire(method, params).map_err(PErr::bad)?;
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let entries = disk_scan::scan(Path::new("/sys/block"), &mounts);
    let probed = match request {
        DiskRequest::VerifyVolume { .. } | DiskRequest::RepairVolume { .. } if entries.iter().any(|e| e.disk.bsd_name == request.device()) => {
            probe_type(request.device())?
        }
        _ => String::new(),
    };
    let plan = disk_request::plan(&request, &entries, &probed).map_err(|refusal| PErr::bad(refusal.sentence()))?;
    let steps = disk_run::run_plan(&plan, disk_request::is_repair(&request)).map_err(PErr::internal)?;
    Ok(obj(vec![("device", s(request.device())), ("steps", Json::Arr(steps.into_iter().map(s).collect()))]))
}

/// The file system type `blkid` reports for the device. Empty output means the volume has no file system to check.
fn probe_type(device: &str) -> Result<String, PErr> {
    let out = Command::new("blkid")
        .args(["-o", "value", "-s", "TYPE", "--"])
        .arg(format!("/dev/{device}"))
        .output()
        .map_err(|e| PErr::internal(format!("blkid: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err(PErr::bad(format!("/dev/{device} has no file system to check")));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: Vec<(&str, Json)>) -> Json {
        obj(pairs)
    }

    #[test]
    fn only_the_five_disk_methods_are_served() {
        assert!(handles(disk_request::M_ERASE_DISK) && handles(disk_request::M_REPAIR_VOLUME));
        assert!(!handles("disk.mount") && !handles("store.install") && !handles("disk"));
    }

    #[test]
    fn a_bad_device_name_is_refused_before_any_command_runs() {
        let p = params(vec![("device", s("../sda")), ("fs", s("ext4"))]);
        let e = call(disk_request::M_ERASE_DISK, &p).unwrap_err();
        assert_eq!(e.code, "bad_params");
        assert!(e.message.contains("not a device name"));
    }

    #[test]
    fn a_device_that_is_not_connected_is_refused() {
        let p = params(vec![("device", s("bifrostnodisk9")), ("fs", s("fat32"))]);
        let e = call(disk_request::M_ERASE_DISK, &p).unwrap_err();
        assert_eq!(e.code, "bad_params");
        assert!(e.message.contains("no longer connected"), "{}", e.message);
    }

    #[test]
    fn an_unknown_file_system_is_refused_before_the_disks_are_read() {
        let p = params(vec![("device", s("sdb")), ("fs", s("ntfs"))]);
        assert!(call(disk_request::M_ERASE_VOLUME, &p).unwrap_err().message.contains("unknown file system"));
    }
}
