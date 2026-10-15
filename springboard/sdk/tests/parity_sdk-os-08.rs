//! Parity sdk-os-08: headless tests of the volume capacity and file ownership surfaces that apps get.
//! They read filesystem metadata only; no test touches a device or a network.

use bifrost_app::file_identity::{DeviceId, FileIdentity, GroupId, Inode, UserId};
use bifrost_app::volume::{capacity, Capacity};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn manifest_file(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

#[test]
fn the_root_volume_reports_capacity_in_order_free_then_total() {
    let c = capacity(Path::new("/")).expect("the root volume is always mounted");
    assert!(c.total > 0, "a mounted filesystem has a size");
    assert!(c.free <= c.total, "free space cannot exceed the size");
    assert!(c.available <= c.free, "the unprivileged share cannot exceed the free space");
}

#[test]
fn a_missing_path_has_no_volume_capacity() {
    assert!(capacity(Path::new("/nonexistent/bifrost/volume/sdk-os-08")).is_err());
}

#[test]
fn the_used_bytes_of_a_volume_follow_its_free_bytes() {
    let c = Capacity { total: 200, free: 50, available: 40 };
    assert_eq!(c.used(), 150);
    assert!((c.used_fraction() - 0.75).abs() < 1e-9);
}

#[test]
fn file_identity_matches_the_metadata_the_system_reports() {
    let path = manifest_file("Cargo.toml");
    let meta = std::fs::metadata(&path).unwrap();
    let id = FileIdentity::of(&path).unwrap();
    assert_eq!(id.user, UserId(meta.uid()));
    assert_eq!(id.group, GroupId(meta.gid()));
    assert_eq!(id.device, DeviceId(meta.dev()));
    assert_eq!(id.inode, Inode(meta.ino()));
}

#[test]
fn a_file_is_owned_by_its_own_user_and_by_no_other() {
    let id = FileIdentity::of(&manifest_file("Cargo.toml")).unwrap();
    assert!(id.is_owned_by(id.user));
    assert!(!id.is_owned_by(UserId(id.user.0.wrapping_add(1))));
}

#[test]
fn two_paths_to_one_file_are_the_same_file() {
    let direct = FileIdentity::of(&manifest_file("Cargo.toml")).unwrap();
    let again = FileIdentity::of(&manifest_file("src/../Cargo.toml")).unwrap();
    assert!(direct.is_same_file(&again));
}

#[test]
fn two_different_files_are_not_the_same_file() {
    let manifest = FileIdentity::of(&manifest_file("Cargo.toml")).unwrap();
    let source = FileIdentity::of(&manifest_file("src/lib.rs")).unwrap();
    assert!(!manifest.is_same_file(&source), "distinct files have distinct inodes");
}

#[test]
fn a_missing_path_has_no_file_identity() {
    assert!(FileIdentity::of(Path::new("/nonexistent/bifrost/identity/sdk-os-08")).is_err());
}
