//! Parity sdk-complete-46: headless tests of the app policy error domain and the keyboard usage table. No test opens a
//! device, a socket or a service.

use std::io;
use std::path::Path;

use springboard::bare::app_policy::{remote_sharing, Policy, PolicyError};
use springboard::bare::key_usage::{boot_report, code_of, usage_of};

#[test]
fn an_unknown_permission_is_reported_with_its_name_and_domain() {
    let err = Policy::from_permissions("example", &["system-update".into()]).unwrap_err();
    assert_eq!(PolicyError::of(&err), Some(&PolicyError::UnsupportedPermission("system-update".into())));
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "Unsupported app permission: system-update");
    assert_eq!(PolicyError::UnsupportedPermission(String::new()).domain(), "bifrost.app-policy");
}

#[test]
fn remote_sharing_without_network_and_storage_has_its_own_code() {
    let err = Policy::from_permissions("is.olibuijr.remote.desktop", &["remote-desktop".into()]).unwrap_err();
    assert_eq!(PolicyError::of(&err), Some(&PolicyError::RemoteNeedsNetworkAndStorage));
    assert_eq!(PolicyError::RemoteNeedsNetworkAndStorage.code(), 2);
}

#[test]
fn a_remote_sharing_refusal_is_a_permission_error_with_the_refusal_code() {
    let err = remote_sharing("example", Policy::default(), true, Path::new("/missing")).unwrap_err();
    assert_eq!(PolicyError::of(&err), Some(&PolicyError::RemoteDenied));
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(err.to_string(), "Remote sharing permission denied");
}

#[test]
fn a_policy_error_converts_to_an_io_error_that_reads_back_unchanged() {
    let err = io::Error::from(PolicyError::MissingPackageDirectory);
    assert_eq!(err.to_string(), "Missing app directory");
    assert_eq!(PolicyError::of(&err), Some(&PolicyError::MissingPackageDirectory));
}

#[test]
fn an_io_error_from_elsewhere_carries_no_policy_error() {
    assert_eq!(PolicyError::of(&io::Error::other("disk full")), None);
}

#[test]
fn every_policy_error_has_a_distinct_code_in_one_domain() {
    let all = [
        PolicyError::UnsupportedPermission(String::new()),
        PolicyError::RemoteNeedsNetworkAndStorage,
        PolicyError::RemoteDenied,
        PolicyError::RemoteServerEscapesPackage,
        PolicyError::RemoteServiceFailed,
        PolicyError::DataIsSymlink,
        PolicyError::DataOwnershipFailed,
        PolicyError::MissingPackageDirectory,
    ];
    let mut codes: Vec<i32> = all.iter().map(PolicyError::code).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes, (1..=8).collect::<Vec<i32>>());
    assert!(all.iter().all(|e| e.domain() == PolicyError::DOMAIN));
}

#[test]
fn a_letter_and_its_usage_round_trip_through_the_table() {
    assert_eq!(usage_of(30), Some(0x04));
    assert_eq!(code_of(0x04), Some(30));
    assert_eq!(usage_of(114), None, "a volume key has no keyboard usage");
}

#[test]
fn a_control_shift_chord_builds_the_boot_report_a_keyboard_sends() {
    // Left control, left shift and the A key down together.
    assert_eq!(boot_report(&[29, 42, 30]), [0x03, 0x00, 0x04, 0, 0, 0, 0, 0]);
}
