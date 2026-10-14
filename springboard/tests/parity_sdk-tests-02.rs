//! Parity sdk-tests-02: headless tests of the disk service feed (`springboard::shell::services::disks`): the
//! device list, the appear, leave and mount events, the description, and the validated plans. The device tree is
//! a temporary sysfs-like directory; no disk, mount or shell is touched.

use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use springboard::shell::services::disks::Disks;
use springboard::shell::services::{Event, SystemService};
use std::fs;
use std::path::{Path, PathBuf};

fn add_disk(sys: &Path, name: &str, removable: &str, sectors: &str) {
    let dir = sys.join(name);
    fs::create_dir_all(dir.join("device")).unwrap();
    fs::write(dir.join("removable"), removable).unwrap();
    fs::write(dir.join("size"), sectors).unwrap();
    fs::write(dir.join("ro"), "0").unwrap();
    fs::write(dir.join("device/model"), "Flash Disk").unwrap();
}

/// A device tree with one fixed disk, `sda`, and an empty mount table. The directory lives as long as the guard.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let sys = root.path().join("block");
    add_disk(&sys, "sda", "0", "100000");
    let mounts = root.path().join("mounts");
    fs::write(&mounts, "").unwrap();
    (root, sys, mounts)
}

fn names(events: &[Event]) -> Vec<String> {
    events.iter().map(|e| format!("{}:{}", e.event, e.body.str_field("bsd_name").unwrap_or_default())).collect()
}

fn named(bsd_name: &str) -> Json {
    Json::Obj(vec![("bsd_name".into(), Json::Str(bsd_name.into()))])
}

#[test]
fn headless_the_disk_service_answers_to_the_name_the_wire_uses() {
    let disks = Disks::default();
    assert_eq!(disks.name(), disk_wire::SERVICE);
    assert_eq!(disks.name(), "disks");
}

#[test]
fn headless_the_first_scan_is_silent_and_lists_the_connected_disks() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    assert!(service.rescan().is_empty(), "the first snapshot is recorded without events");
    let reply = service.handle("app", "list", &Json::Null).unwrap();
    let list = reply.get("disks").and_then(Json::as_array).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].str_field("bsd_name").as_deref(), Some("sda"));
}

#[test]
fn headless_a_plugged_in_disk_is_announced_once_and_a_removed_one_leaves() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    add_disk(&sys, "sdb", "1", "2000");
    assert_eq!(names(&service.rescan()), vec!["disk_appeared:sdb"]);
    assert!(service.rescan().is_empty(), "an unchanged tree sends nothing");
    fs::remove_dir_all(sys.join("sdb")).unwrap();
    assert_eq!(names(&service.rescan()), vec!["disk_disappeared:sdb"]);
}

#[test]
fn headless_mounting_a_disk_sends_a_description_change_and_describe_shows_the_mount() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    fs::write(&mounts, "/dev/sda /media/data ext4 rw 0 0\n").unwrap();
    let events = service.rescan();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, disk_wire::DESCRIPTION_CHANGED);
    let reply = service.handle("app", "describe", &named("sda")).unwrap();
    let description = reply.get("description").unwrap();
    assert_eq!(description.str_field("volume_path").as_deref(), Some("/media/data"));
    assert_eq!(description.str_field("device_model").as_deref(), Some("Flash Disk"));
}

#[test]
fn headless_describe_names_a_disk_that_is_not_connected() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    let err = service.handle("app", "describe", &named("sdz")).unwrap_err();
    assert_eq!(err, "disks: sdz is not connected");
}

#[test]
fn headless_a_plan_is_refused_for_a_disk_that_is_not_connected() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    let err = service.handle("app", "plan_unmount", &named("sdz")).unwrap_err();
    assert!(err.contains("sdz"), "the refusal names the disk: {err}");
}

#[test]
fn headless_a_device_name_that_is_a_path_is_refused_before_any_plan() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    assert!(service.handle("app", "plan_eject", &named("../etc")).is_err());
    assert!(service.handle("app", "plan_eject", &Json::Null).is_err(), "a call without a name is refused");
    assert!(service.handle("app", "no_such_method", &Json::Null).is_err());
}

#[test]
fn headless_a_disk_claimed_by_one_app_refuses_an_eject_from_another() {
    let (_root, sys, mounts) = fixture();
    let mut service = Disks::at(&sys, &mounts);
    service.rescan();
    service.handle("owner", "claim", &named("sda")).unwrap();
    let err = service.handle("other", "plan_eject", &named("sda")).unwrap_err();
    assert!(err.contains("in use by owner"), "{err}");
}
