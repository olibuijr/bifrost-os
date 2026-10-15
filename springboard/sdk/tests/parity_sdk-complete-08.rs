//! Parity sdk-complete-08: headless tests for the bundle lookups of embedded helpers and job property lists, pixel buffers
//! with their colour attachments, and audio boxes. Everything runs on in-memory values: no bundle is read from disk,
//! no buffer reaches a display and no audio device is opened.

use bifrost_app::audio_box::{AudioBox, BoxRegistry};
use bifrost_app::pixel_buffer::{
    Attachments, PixelBuffer, PixelBufferError, PixelBufferPool, PoolConfig, PoolError, Primaries, Transfer, YcbcrMatrix,
};
use bifrost_app::service_bundle::{BundleServices, JobKind, LOGIN_ITEMS_DIR};
use bifrost_app::service_management::{ServiceError, Status};
use bifrost_render::buffer_attach::Rect;
use bifrost_render::colorspace::Space;
use bifrost_render::pixel_format::FourCc;

fn code(s: &str) -> FourCc {
    FourCc::parse(s).unwrap()
}

fn bundle() -> BundleServices {
    BundleServices::new(
        &[
            "Contents/Library/LoginItems/Updater.app",
            "Contents/Library/LoginItems/Sync.app",
            "Contents/Library/LoginItems/Empty.app",
            "Contents/Resources/Stray.app",
            "Contents/Library/LaunchAgents/com.example.agent.plist",
            "Contents/Library/LaunchDaemons/com.example.daemon.plist",
        ],
        &[
            ("Contents/Library/LoginItems/Updater.app", "org.example.updater"),
            ("Contents/Library/LoginItems/Sync.app", "org.example.sync"),
            ("Contents/Library/LoginItems/Empty.app", "   "),
            ("Contents/Resources/Stray.app", "org.example.stray"),
        ],
    )
}

#[test]
fn bundle_helpers_keep_only_apps_in_the_login_item_directory_with_an_identifier() {
    let b = bundle();
    let paths: Vec<&str> = b.helpers().iter().map(|h| h.path.as_str()).collect();
    assert_eq!(paths, ["Contents/Library/LoginItems/Updater.app", "Contents/Library/LoginItems/Sync.app"]);
    assert!(b.helpers().iter().all(|h| h.path.starts_with(LOGIN_ITEMS_DIR)));
}

#[test]
fn a_login_item_is_found_by_its_bundle_identifier() {
    let b = bundle();
    let item = b.login_item("org.example.sync").unwrap();
    assert_eq!(item.path, "Contents/Library/LoginItems/Sync.app");
    assert_eq!(item.bundle_id, "org.example.sync");
}

#[test]
fn an_identifier_that_names_no_embedded_helper_is_job_not_found() {
    let b = bundle();
    assert_eq!(b.login_item("org.example.stray"), Err(ServiceError::JobNotFound), "outside the login item directory");
    assert_eq!(b.login_item("org.example.empty"), Err(ServiceError::JobNotFound));
    assert_eq!(b.login_item(""), Err(ServiceError::JobNotFound));
    assert_eq!(b.login_item("org.example/sync"), Err(ServiceError::JobNotFound));
    assert_eq!(BundleServices::default().login_item("org.example.sync"), Err(ServiceError::JobNotFound));
}

#[test]
fn a_login_item_status_is_enabled_only_when_the_user_switched_it_on() {
    let b = bundle();
    assert_eq!(b.login_item_status("org.example.updater", &["org.example.updater"]), Status::Enabled);
    assert_eq!(b.login_item_status("org.example.updater", &["org.example.sync"]), Status::NotRegistered);
    assert_eq!(b.login_item_status("org.example.stray", &["org.example.stray"]), Status::NotFound);
}

#[test]
fn a_launch_agent_is_found_only_in_the_agents_directory() {
    let b = bundle();
    assert_eq!(
        b.job_plist(JobKind::Agent, "com.example.agent.plist"),
        Ok("Contents/Library/LaunchAgents/com.example.agent.plist".to_string())
    );
    assert_eq!(b.job_plist(JobKind::Agent, "com.example.daemon.plist"), Err(ServiceError::JobPlistNotFound));
    assert_eq!(b.job_plist(JobKind::Agent, "com.example.gone.plist"), Err(ServiceError::JobPlistNotFound));
}

#[test]
fn a_launch_daemon_is_found_only_in_the_daemons_directory() {
    let b = bundle();
    assert_eq!(
        b.job_plist(JobKind::Daemon, "com.example.daemon.plist"),
        Ok("Contents/Library/LaunchDaemons/com.example.daemon.plist".to_string())
    );
    assert_eq!(b.job_plist(JobKind::Daemon, "com.example.agent.plist"), Err(ServiceError::JobPlistNotFound));
}

#[test]
fn a_job_name_that_is_not_a_plist_name_is_invalid_before_the_bundle_is_searched() {
    let b = bundle();
    assert_eq!(b.job_plist(JobKind::Agent, "com.example.agent"), Err(ServiceError::InvalidPlist));
    assert_eq!(b.job_plist(JobKind::Daemon, "dir/com.example.daemon.plist"), Err(ServiceError::InvalidPlist));
    assert_eq!(BundleServices::default().job_plist(JobKind::Agent, "com.example.agent.plist"), Err(ServiceError::JobPlistNotFound));
}

#[test]
fn a_nv12_picture_with_odd_chroma_rounds_the_chroma_plane_up() {
    // 5 x 3 luma: 5 bytes by 3 rows. Chroma: 3 samples of 2 bytes by 2 rows, a 6-byte stride.
    let buf = PixelBuffer::new(code("NV12"), 5, 3).unwrap();
    assert_eq!(buf.bytes_per_row(0), Some(5));
    assert_eq!(buf.bytes_per_row(1), Some(6));
    assert_eq!(buf.plane_width(1), Some(3));
    assert_eq!(buf.plane_height(1), Some(2));
    assert_eq!(buf.data_size(), 27);
}

#[test]
fn a_buffer_in_an_unknown_format_or_with_no_size_is_refused() {
    assert_eq!(PixelBuffer::new(code("XX99"), 4, 4), Err(PixelBufferError::UnknownFormat));
    assert_eq!(PixelBuffer::new(code("AR24"), 0, 4), Err(PixelBufferError::InvalidSize));
    let config = PoolConfig { min_count: 0, max_age: None, threshold: None };
    assert!(matches!(PixelBufferPool::new(code("XX99"), 4, 4, config), Err(PixelBufferError::UnknownFormat)));
}

#[test]
fn bytes_are_reachable_only_under_a_lock_and_writable_only_under_a_read_write_lock() {
    let mut buf = PixelBuffer::new(code("AR24"), 3, 2).unwrap();
    assert!(buf.base_address(0).is_none());
    buf.lock(true);
    assert_eq!(buf.base_address(0).map(|b| b.len()), Some(24));
    assert!(buf.base_address_mut(0).is_none(), "a read-only lock denies writing");
    buf.unlock(true).unwrap();
    buf.lock(false);
    buf.base_address_mut(0).unwrap()[3] = 0x80;
    buf.unlock(false).unwrap();
}

#[test]
fn the_revision_advances_on_each_read_write_unlock_and_not_on_a_read_only_one() {
    let mut buf = PixelBuffer::new(code("AR24"), 2, 2).unwrap();
    buf.lock(true);
    buf.unlock(true).unwrap();
    assert_eq!(buf.revision(), 0);
    buf.lock(false);
    buf.lock(false);
    buf.unlock(false).unwrap();
    buf.unlock(false).unwrap();
    assert_eq!(buf.revision(), 2);
    assert_eq!(buf.unlock(false), Err(PixelBufferError::NotLocked));
    assert_eq!(buf.revision(), 2, "a refused unlock is not a change");
}

#[test]
fn the_colour_space_follows_the_attachments_the_buffer_carries() {
    let mut buf = PixelBuffer::new(code("NV12"), 16, 16).unwrap();
    assert_eq!(buf.color_space(), None, "no attachments, no space");
    buf.set_attachments(Attachments {
        primaries: Some(Primaries::Bt709),
        transfer: Some(Transfer::Srgb),
        matrix: Some(YcbcrMatrix::Bt709),
        clean: None,
    });
    assert_eq!(buf.color_space(), Some(Space::Srgb));
    buf.set_attachments(Attachments { primaries: Some(Primaries::DisplayP3), transfer: Some(Transfer::Srgb), ..Attachments::default() });
    assert_eq!(buf.color_space(), Some(Space::DisplayP3));
    buf.set_attachments(Attachments { primaries: Some(Primaries::Bt2020), transfer: Some(Transfer::Pq), ..Attachments::default() });
    assert_eq!(buf.color_space(), None, "HDR primaries have no working space here");
}

#[test]
fn setting_attachments_does_not_change_the_pixels_or_the_revision() {
    let mut buf = PixelBuffer::new(code("AR24"), 4, 4).unwrap();
    buf.lock(false);
    buf.base_address_mut(0).unwrap()[0] = 0x7f;
    buf.unlock(false).unwrap();
    let revision = buf.revision();
    buf.set_attachments(Attachments { matrix: Some(YcbcrMatrix::Bt601), ..Attachments::default() });
    assert_eq!(buf.revision(), revision);
    assert_eq!(buf.attachments().matrix, Some(YcbcrMatrix::Bt601));
}

#[test]
fn the_clean_aperture_is_clipped_to_the_buffer() {
    let mut buf = PixelBuffer::new(code("NV12"), 12, 8).unwrap();
    assert_eq!(buf.clean_rect(), Rect { x: 0, y: 0, w: 12, h: 8 }, "no aperture means the whole buffer");
    buf.set_attachments(Attachments { clean: Some(Rect { x: 4, y: 2, w: 10, h: 10 }), ..Attachments::default() });
    assert_eq!(buf.clean_rect(), Rect { x: 4, y: 2, w: 8, h: 6 });
}

#[test]
fn a_pool_hands_out_buffers_of_its_format_up_to_the_threshold() {
    let config = PoolConfig { min_count: 1, max_age: None, threshold: Some(2) };
    let mut pool = PixelBufferPool::new(code("AR24"), 2, 2, config).unwrap();
    assert_eq!(pool.prewarm(0), 1);
    let a = pool.acquire().unwrap();
    let b = pool.acquire().unwrap();
    assert_eq!(a.data_size(), 16);
    assert_eq!(pool.acquire(), Err(PoolError::WouldExceedThreshold));
    pool.release(b, 1);
    assert_eq!(pool.free_count(), 1);
    pool.release(a, 2);
    assert_eq!(pool.flush(), 2);
    assert_eq!(pool.live(), 0);
}

fn box_with_devices() -> BoxRegistry {
    let mut r = BoxRegistry::new();
    r.add(AudioBox::new("dock", "Dock", "Maker")).unwrap();
    r.add(AudioBox::new("hub", "Hub", "Maker")).unwrap();
    r.add_device("dock", "dock-in").unwrap();
    r.add_device("dock", "dock-out").unwrap();
    r
}

#[test]
fn a_device_belongs_to_at_most_one_box() {
    let mut r = box_with_devices();
    assert!(r.add_device("hub", "dock-in").is_err(), "already in the dock");
    assert_eq!(r.get("dock").unwrap().devices, ["dock-in", "dock-out"]);
    assert!(r.get("hub").unwrap().devices.is_empty());
}

#[test]
fn boxes_keep_their_attach_order() {
    let r = box_with_devices();
    let uids: Vec<&str> = r.boxes().iter().map(|b| b.uid.as_str()).collect();
    assert_eq!(uids, ["dock", "hub"]);
}

#[test]
fn a_box_can_be_acquired_once_and_is_freed_by_release() {
    let mut r = box_with_devices();
    r.acquire("dock").unwrap();
    assert!(r.acquire("dock").is_err(), "a second client is refused");
    assert!(r.set_enabled("dock", false).is_err(), "an acquired box stays enabled");
    r.release("dock").unwrap();
    r.set_enabled("dock", false).unwrap();
    assert!(r.acquire("dock").is_err(), "a disabled box cannot be acquired");
}

#[test]
fn releasing_a_box_nobody_holds_is_refused_and_unknown_boxes_are_reported() {
    let mut r = box_with_devices();
    assert!(r.release("dock").is_err());
    assert!(r.acquire("missing").is_err());
    assert!(r.set_enabled("missing", true).is_err());
    assert!(r.get("missing").is_none());
}
