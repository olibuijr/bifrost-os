//! Parity sdk-complete-23: headless tests for the file system resource kinds and the maintenance task log and
//! completion. Every test drives the public API with concrete text and fixed values. No device is opened.

use bifrost_ui::fs_maintenance::{MaintenanceError, MaintenanceQueue, Operation, Phase};
use bifrost_ui::fs_resource::{Resource, ResourceError};

#[test]
fn headless_a_block_device_is_one_node_name_under_dev() {
    assert_eq!(Resource::block_device("/dev/sdb1").unwrap().text(), "/dev/sdb1");
    assert_eq!(Resource::block_device("/dev/disk-2_part.0").unwrap().text(), "/dev/disk-2_part.0");
    assert_eq!(Resource::block_device("/dev/"), Err(ResourceError::NotADeviceNode));
    assert_eq!(Resource::block_device("/tmp/sdb1"), Err(ResourceError::NotADeviceNode));
    assert_eq!(Resource::block_device("/dev/../etc"), Err(ResourceError::NotADeviceNode));
    assert_eq!(Resource::block_device("/dev/sub/dir"), Err(ResourceError::NotADeviceNode));
    assert_eq!(Resource::block_device("/dev/a b"), Err(ResourceError::NotADeviceNode));
}

#[test]
fn headless_a_path_resource_is_absolute_and_has_no_dot_parts() {
    assert_eq!(Resource::path_url("/Volumes/archive.img").unwrap().text(), "/Volumes/archive.img");
    assert!(Resource::path_url("/").is_ok());
    assert!(Resource::path_url("/a//b/").is_ok());
    assert_eq!(Resource::path_url("images/a.img"), Err(ResourceError::NotAbsolutePath));
    assert_eq!(Resource::path_url("/a/../b"), Err(ResourceError::DotSegment));
    assert_eq!(Resource::path_url("/a/./b"), Err(ResourceError::DotSegment));
}

#[test]
fn headless_a_generic_url_needs_a_scheme_and_a_location() {
    assert_eq!(
        Resource::generic_url("https://example.com/disk.img").unwrap().text(),
        "https://example.com/disk.img"
    );
    assert!(Resource::generic_url("ftp+x.y-z:data").is_ok());
    assert_eq!(Resource::generic_url("no-scheme-here"), Err(ResourceError::BadScheme));
    assert_eq!(Resource::generic_url("1http://x"), Err(ResourceError::BadScheme));
    assert_eq!(Resource::generic_url("a b:c"), Err(ResourceError::BadScheme));
    assert_eq!(Resource::generic_url("https:"), Err(ResourceError::MissingLocation));
}

#[test]
fn headless_empty_text_and_control_characters_are_refused_for_every_kind() {
    assert_eq!(Resource::block_device(""), Err(ResourceError::Empty));
    assert_eq!(Resource::path_url(""), Err(ResourceError::Empty));
    assert_eq!(Resource::generic_url(""), Err(ResourceError::Empty));
    assert_eq!(Resource::path_url("/a\nb"), Err(ResourceError::ControlCharacter));
    assert_eq!(Resource::generic_url("https://x\u{0}"), Err(ResourceError::ControlCharacter));
}

#[test]
fn headless_parse_picks_the_kind_from_the_prefix() {
    assert_eq!(Resource::parse("/dev/sdc").unwrap(), Resource::BlockDevice("/dev/sdc".into()));
    assert_eq!(Resource::parse("/home/a.img").unwrap(), Resource::PathUrl("/home/a.img".into()));
    assert_eq!(Resource::parse("https://h/x").unwrap(), Resource::GenericUrl("https://h/x".into()));
    assert_eq!(Resource::parse("/dev/"), Err(ResourceError::NotADeviceNode));
}

#[test]
fn headless_a_new_maintenance_request_is_queued_with_the_first_id() {
    let mut queue = MaintenanceQueue::new();
    let id = queue.submit("disk-a", Operation::Check).unwrap();
    assert_eq!(id.get(), 1);
    let job = queue.job(id).unwrap();
    assert_eq!(job.phase, Phase::Queued);
    assert_eq!(job.resource, "disk-a");
    assert!(job.messages.is_empty());
}

#[test]
fn headless_a_second_request_for_the_same_resource_is_busy_until_the_first_finishes() {
    let mut queue = MaintenanceQueue::new();
    let first = queue.submit("disk-a", Operation::Check).unwrap();
    assert_eq!(
        queue.submit("disk-a", Operation::Format),
        Err(MaintenanceError::Busy { resource: "disk-a".into(), active: first })
    );
    queue.start(first).unwrap();
    queue.finish(first, Ok(())).unwrap();
    assert!(queue.submit("disk-a", Operation::Format).is_ok());
}

#[test]
fn headless_requests_for_different_resources_do_not_block_each_other() {
    let mut queue = MaintenanceQueue::new();
    queue.submit("disk-a", Operation::Check).unwrap();
    assert!(queue.submit("disk-b", Operation::Check).is_ok());
}

#[test]
fn headless_a_task_logs_its_messages_in_order() {
    let mut queue = MaintenanceQueue::new();
    let id = queue.submit("disk-a", Operation::Format).unwrap();
    queue.start(id).unwrap();
    queue.log(id, "wiping header").unwrap();
    queue.log(id, "writing table").unwrap();
    assert_eq!(queue.job(id).unwrap().messages, vec!["wiping header".to_string(), "writing table".to_string()]);
}

#[test]
fn headless_a_failed_task_keeps_its_reason_and_its_log() {
    let mut queue = MaintenanceQueue::new();
    let id = queue.submit("disk-a", Operation::Check).unwrap();
    queue.start(id).unwrap();
    queue.log(id, "scanning").unwrap();
    queue.finish(id, Err("bad sector 42")).unwrap();
    let job = queue.job(id).unwrap();
    assert_eq!(job.phase, Phase::Failed("bad sector 42".into()));
    assert_eq!(job.messages, vec!["scanning".to_string()]);
    assert!(job.phase.is_finished());
}

#[test]
fn headless_a_task_completes_once_and_a_finished_task_cannot_be_cancelled() {
    let mut queue = MaintenanceQueue::new();
    let id = queue.submit("disk-a", Operation::Check).unwrap();
    queue.start(id).unwrap();
    queue.finish(id, Ok(())).unwrap();
    assert_eq!(queue.finish(id, Ok(())), Err(MaintenanceError::InvalidTransition));
    assert_eq!(queue.cancel(id), Err(MaintenanceError::InvalidTransition));
    assert_eq!(queue.job(id).unwrap().phase, Phase::Succeeded);
}

#[test]
fn headless_a_queued_task_can_be_cancelled_and_the_runner_skips_it() {
    let mut queue = MaintenanceQueue::new();
    let first = queue.submit("disk-a", Operation::Check).unwrap();
    let second = queue.submit("disk-b", Operation::Check).unwrap();
    queue.cancel(first).unwrap();
    assert_eq!(queue.job(first).unwrap().phase, Phase::Cancelled);
    assert_eq!(queue.next_queued(), Some(second));
}

#[test]
fn headless_a_mount_needs_a_bundle_and_a_resource_name() {
    let mut queue = MaintenanceQueue::new();
    let mount = Operation::Mount { bundle_id: String::new() };
    assert_eq!(queue.submit("disk-a", mount), Err(MaintenanceError::EmptyBundleId));
    assert_eq!(queue.submit("", Operation::Check), Err(MaintenanceError::EmptyResource));
}
