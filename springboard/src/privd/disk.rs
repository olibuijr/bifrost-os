//! The privileged disk requests (`disk.*`): erase a disk or a volume, partition a disk, verify, repair or rename a
//! volume, restore one volume onto another, and create a RAID array. The request is read and checked by `bifrost_core::disk_request`, planned against a fresh snapshot of the
//! disks taken here (so a stale client view never reaches a command), and the plan runs as root with fixed argv.
//! Like every method other than the unlock ones, it needs the unlocked session's token (see `server::dispatch`).

use super::{obj, s, PErr};
use bifrost_core::disk_request::{self, DiskRequest};
use bifrost_core::disk_run;
use bifrost_core::disk_scan;
use bifrost_core::json::Json;
use bifrost_core::volume_unlock;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The methods this module serves.
pub fn handles(method: &str) -> bool {
    matches!(
        method,
        disk_request::M_ERASE_DISK
            | disk_request::M_ERASE_VOLUME
            | disk_request::M_PARTITION_DISK
            | disk_request::M_VERIFY_VOLUME
            | disk_request::M_REPAIR_VOLUME
            | disk_request::M_RENAME_VOLUME
            | disk_request::M_RESTORE_VOLUME
            | disk_request::M_CREATE_RAID
            | volume_unlock::M_UNLOCK_VOLUME
    )
}

/// Runs one disk request. The reply carries the device and the tools that completed, in order.
pub fn call(method: &str, params: &Json) -> Result<Json, PErr> {
    if method == volume_unlock::M_UNLOCK_VOLUME {
        return unlock(params);
    }
    let request = DiskRequest::from_wire(method, params).map_err(PErr::bad)?;
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let entries = disk_scan::scan(Path::new("/sys/block"), &mounts);
    let probed = match request {
        DiskRequest::VerifyVolume { .. } | DiskRequest::RepairVolume { .. } | DiskRequest::RenameVolume { .. }
            if entries.iter().any(|e| e.disk.bsd_name == request.device()) =>
        {
            probe_type(request.device())?
        }
        _ => String::new(),
    };
    let plan = disk_request::plan(&request, &entries, &probed).map_err(|refusal| PErr::bad(refusal.sentence()))?;
    let steps = disk_run::run_plan(&plan, disk_request::is_repair(&request)).map_err(PErr::internal)?;
    Ok(obj(vec![("device", s(request.device())), ("steps", Json::Arr(steps.into_iter().map(s).collect()))]))
}

/// Opens a locked LUKS volume with the passphrase, then mounts the mapping through udisks. The device must be a LUKS
/// container with no open mapping. The passphrase goes to `cryptsetup` on stdin and is never logged or echoed.
fn unlock(params: &Json) -> Result<Json, PErr> {
    let device = params.get("device").and_then(Json::as_str).ok_or_else(|| PErr::bad("missing device"))?;
    let password = params.get("password").and_then(Json::as_str).ok_or_else(|| PErr::bad("missing password"))?;
    volume_unlock::check_device(device).map_err(PErr::bad)?;
    if password.is_empty() || password.len() > 512 || password.contains('\0') {
        return Err(PErr::bad("the passphrase is empty or has characters a passphrase cannot have"));
    }
    let listing = run_with_stdin(&argv(&["lsblk", "-rpno", "NAME,TYPE,FSTYPE,PKNAME"]), "")?;
    if !volume_unlock::locked(&listing).iter().any(|d| d == device) {
        return Err(PErr::bad(format!("{device} is not a locked encrypted volume")));
    }
    let mapper = volume_unlock::mapper_name(device);
    run_with_stdin(&unlock_args(device, &mapper), password)?;
    // the volume is open even when udisks cannot mount it, so the reply says which
    let mounted = run_with_stdin(&mount_args(&mapper), "").is_ok();
    Ok(obj(vec![("device", s(device)), ("mapper", s(&mapper)), ("mounted", Json::Bool(mounted))]))
}

/// The `cryptsetup` command that opens `device` as `mapper`, reading the passphrase from stdin.
pub fn unlock_args(device: &str, mapper: &str) -> Vec<String> {
    argv(&["cryptsetup", "open", "--type", "luks", "--key-file=-", device, mapper])
}

/// The udisks command that mounts an opened mapping.
pub fn mount_args(mapper: &str) -> Vec<String> {
    let target = format!("/dev/mapper/{mapper}");
    argv(&["udisksctl", "mount", "--no-user-interaction", "--block-device", target.as_str()])
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Runs one command with `stdin` written to it and returns its stdout. A failure carries the tool's last stderr line.
fn run_with_stdin(args: &[String], stdin: &str) -> Result<String, PErr> {
    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PErr::internal(format!("{}: {e}", args[0])))?;
    if let Some(mut pipe) = child.stdin.take() {
        // a tool that stops reading early is judged by its exit status below
        let _ = pipe.write_all(stdin.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| PErr::internal(format!("{}: {e}", args[0])))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string();
    Err(PErr::internal(format!("{}: {last}", args[0])))
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
    fn only_the_eight_disk_methods_are_served() {
        assert!(handles(disk_request::M_ERASE_DISK) && handles(disk_request::M_REPAIR_VOLUME));
        assert!(handles(disk_request::M_RENAME_VOLUME) && handles(disk_request::M_RESTORE_VOLUME) && handles(disk_request::M_CREATE_RAID));
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
    fn unlock_is_served_and_refuses_a_bad_name_before_any_command() {
        assert!(handles(volume_unlock::M_UNLOCK_VOLUME));
        let p = params(vec![("device", s("sda2 --batch")), ("password", s("x"))]);
        let e = call(volume_unlock::M_UNLOCK_VOLUME, &p).unwrap_err();
        assert_eq!(e.code, "bad_params");
    }

    #[test]
    fn unlock_refuses_an_empty_passphrase_before_any_command() {
        let p = params(vec![("device", s("/dev/sda2")), ("password", s(""))]);
        assert_eq!(call(volume_unlock::M_UNLOCK_VOLUME, &p).unwrap_err().code, "bad_params");
    }

    #[test]
    fn the_unlock_commands_read_the_key_from_stdin_and_mount_the_mapping() {
        assert_eq!(unlock_args("/dev/sda2", "bifrost-sda2"), ["cryptsetup", "open", "--type", "luks", "--key-file=-", "/dev/sda2", "bifrost-sda2"]);
        assert_eq!(mount_args("bifrost-sda2"), ["udisksctl", "mount", "--no-user-interaction", "--block-device", "/dev/mapper/bifrost-sda2"]);
    }

    #[test]
    fn an_unknown_file_system_is_refused_before_the_disks_are_read() {
        let p = params(vec![("device", s("sdb")), ("fs", s("xfs"))]);
        assert!(call(disk_request::M_ERASE_VOLUME, &p).unwrap_err().message.contains("unknown file system"));
    }
}
