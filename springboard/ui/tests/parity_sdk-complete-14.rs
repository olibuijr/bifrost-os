//! Parity sdk-complete-14: headless tests of the camera device and device browser models. The system layer is
//! simulated by calling the models directly; no USB bus, camera or scanner is touched.

use bifrost_ui::camera_device::{Capabilities, CameraDevice, CameraError, DeleteError, Media};
use bifrost_ui::device_browser::{Device, DeviceBrowser, DeviceEvent, DeviceKind, DeviceMask, Transport};

fn card_camera() -> CameraDevice {
    let caps = Capabilities::TAKE_PICTURE
        .union(Capabilities::DELETE_ONE)
        .union(Capabilities::SYNC_CLOCK);
    let mut camera = CameraDevice::new(caps);
    let contents = camera.contents_mut();
    let dcim = contents.add_folder(None, "DCIM").expect("top-level folder");
    contents.add_file(Some(dcim), "IMG_0001.JPG", 2_000, Media::Image).expect("first file");
    contents.add_file(Some(dcim), "IMG_0002.JPG", 2_100, Media::Image).expect("second file");
    contents.add_file(Some(dcim), "CLIP_0003.MOV", 90_000, Media::Movie).expect("movie");
    camera
}

#[test]
fn a_locked_camera_refuses_every_delete_and_keeps_the_files() {
    let mut camera = card_camera();
    camera.set_locked(true);
    assert!(camera.is_locked());
    let results = camera.request_delete(&[1, 2]);
    assert_eq!(results, vec![(1, Err(DeleteError::Locked)), (2, Err(DeleteError::Locked))]);
    assert_eq!(camera.contents().children(Some(0)).len(), 3);
}

#[test]
fn an_unlocked_delete_reports_each_index_and_leaves_no_tombstone_in_the_listing() {
    let mut camera = card_camera();
    let results = camera.request_delete(&[1, 0, 9]);
    assert_eq!(
        results,
        vec![(1, Ok(())), (0, Err(DeleteError::NotAFile)), (9, Err(DeleteError::NotFound))],
        "one failure does not hide the others"
    );
    assert_eq!(camera.contents().children(Some(0)), vec![2, 3]);
    assert!(camera.contents().entry(1).is_none(), "a deleted index is not listed again");
}

#[test]
fn a_camera_without_a_delete_capability_refuses_deletes_with_not_supported() {
    let mut camera = CameraDevice::new(Capabilities::TAKE_PICTURE);
    camera.contents_mut().add_file(None, "A.JPG", 1, Media::Image);
    assert_eq!(camera.request_delete(&[0]), vec![(0, Err(DeleteError::NotSupported))]);
}

#[test]
fn a_locked_state_can_be_cleared_and_deletes_then_succeed() {
    let mut camera = card_camera();
    camera.set_locked(true);
    camera.set_locked(false);
    assert_eq!(camera.request_delete(&[2]), vec![(2, Ok(()))]);
}

#[test]
fn taking_a_picture_needs_the_capability_and_tethering_first() {
    let mut camera = card_camera();
    assert_eq!(camera.request_take_picture(), Err(CameraError::NotTethered));
    assert_eq!(camera.request_enable_tethering(), Ok(()));
    assert!(camera.is_tethered());
    assert_eq!(camera.request_take_picture(), Ok(()));
    camera.request_disable_tethering();
    assert_eq!(camera.request_take_picture(), Err(CameraError::NotTethered));
}

#[test]
fn tethering_without_the_take_picture_capability_is_not_supported() {
    let mut camera = CameraDevice::new(Capabilities::DELETE_ONE);
    assert_eq!(camera.request_enable_tethering(), Err(CameraError::NotSupported));
    assert!(!camera.is_tethered());
}

#[test]
fn a_clock_sync_reports_the_offset_before_it_and_then_reads_zero() {
    let mut camera = card_camera();
    assert_eq!(camera.time_offset_seconds(), None);
    assert_eq!(camera.request_sync_clock(1_000_120, 1_000_000), Ok(120));
    assert_eq!(camera.time_offset_seconds(), Some(0));
}

#[test]
fn a_camera_without_the_clock_capability_cannot_sync() {
    let mut camera = CameraDevice::new(Capabilities::NONE);
    assert_eq!(camera.request_sync_clock(5, 1), Err(CameraError::NotSupported));
    assert_eq!(camera.time_offset_seconds(), None);
}

#[test]
fn the_battery_level_is_clamped_and_unknown_when_not_reported() {
    let mut camera = card_camera();
    assert_eq!(camera.battery_level(), None);
    camera.set_battery_level(Some(140));
    assert_eq!(camera.battery_level(), Some(100));
    camera.set_battery_level(None);
    assert_eq!(camera.battery_level(), None);
}

#[test]
fn a_folder_path_joins_names_from_the_top_level() {
    let camera = card_camera();
    assert_eq!(camera.contents().path(1), Some("DCIM/IMG_0001.JPG".to_string()));
    assert_eq!(camera.contents().files_of(Media::Movie), vec![3]);
}

fn cam(id: &str) -> Device {
    Device {
        id: id.into(),
        name: format!("Camera {id}"),
        kind: DeviceKind::Camera,
        transport: Transport::Usb,
        serial: None,
    }
}

fn scanner(id: &str) -> Device {
    Device {
        id: id.into(),
        name: format!("Scanner {id}"),
        kind: DeviceKind::Scanner,
        transport: Transport::Usb,
        serial: None,
    }
}

#[test]
fn a_browser_reports_removals_before_additions_in_id_order() {
    let mut browser = DeviceBrowser::new();
    assert!(browser.start(DeviceMask::ALL));
    assert_eq!(browser.update(vec![cam("b"), cam("a")]), vec![DeviceEvent::Added(cam("a")), DeviceEvent::Added(cam("b"))]);
    let events = browser.update(vec![cam("c")]);
    assert_eq!(events, vec![DeviceEvent::Removed(cam("a")), DeviceEvent::Removed(cam("b")), DeviceEvent::Added(cam("c"))]);
}

#[test]
fn a_device_that_returns_under_the_same_id_is_added_again() {
    let mut browser = DeviceBrowser::new();
    browser.start(DeviceMask::CAMERA);
    browser.update(vec![cam("sn-1")]);
    browser.update(Vec::new());
    assert_eq!(browser.update(vec![cam("sn-1")]), vec![DeviceEvent::Added(cam("sn-1"))]);
}

#[test]
fn a_browser_only_reports_the_kinds_in_its_mask() {
    let mut browser = DeviceBrowser::new();
    browser.start(DeviceMask::SCANNER);
    assert!(browser.update(vec![cam("c1")]).is_empty());
    assert_eq!(browser.update(vec![scanner("s1")]), vec![DeviceEvent::Added(scanner("s1"))]);
}

#[test]
fn an_empty_mask_does_not_start_and_a_stopped_browser_ignores_snapshots() {
    let mut browser = DeviceBrowser::new();
    assert!(!browser.start(DeviceMask::NONE));
    assert!(!browser.is_browsing());
    browser.start(DeviceMask::ALL);
    browser.stop();
    assert!(browser.update(vec![cam("c1")]).is_empty());
    assert!(browser.devices().next().is_none());
}

#[test]
fn the_preferred_device_is_kept_while_away_and_selected_again_on_return() {
    let mut browser = DeviceBrowser::new();
    browser.start(DeviceMask::ALL);
    browser.update(vec![cam("a"), scanner("s")]);
    assert!(browser.set_preferred("s"));
    assert_eq!(browser.preferred_device().map(|d| d.id.as_str()), Some("s"));
    browser.update(vec![cam("a")]);
    assert_eq!(browser.preferred_device().map(|d| d.id.as_str()), Some("a"), "the first attached device stands in");
    browser.update(vec![cam("a"), scanner("s")]);
    assert_eq!(browser.preferred_device().map(|d| d.id.as_str()), Some("s"));
    assert!(!browser.set_preferred("missing"));
}
