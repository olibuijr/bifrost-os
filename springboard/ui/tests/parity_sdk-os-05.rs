//! Parity sdk-os-05: headless tests for the shared buffer layout, the window capture filter, multicast membership and
//! TLS options. No test opens a socket, a display or a device.

use bifrost_ui::capture_content::{CaptureDisplay, CaptureWindow, ShareableContent};
use bifrost_ui::capture_filter::{ContentFilter, Rule};
use bifrost_ui::multicast::{MembershipError, MulticastMembership};
use bifrost_ui::surface_layout::{LayoutError, PixelFormat, SurfaceLayout};
use bifrost_ui::tls_options::{TlsOptions, TlsOptionsError, TlsVersion, Trust};
use bifrost_ui::Rect;
use std::net::Ipv4Addr;

// Shared buffer layout (surface_layout)

#[test]
fn headless_a_720p_i420_buffer_places_its_chroma_planes_after_the_luma_plane() {
    let l = SurfaceLayout::new(PixelFormat::I420, 1280, 720, 64).unwrap();
    assert_eq!(l.plane(0).unwrap().size, 921_600);
    assert_eq!(l.plane(1).unwrap().offset, 921_600);
    assert_eq!(l.plane(1).unwrap().width, 640);
    assert_eq!(l.plane(1).unwrap().height, 360);
    assert_eq!(l.plane(2).unwrap().offset, 1_152_000);
    assert_eq!(l.total_size(), 1_382_400);
}

#[test]
fn headless_a_sixteen_bit_row_is_padded_to_the_alignment_it_was_given() {
    // Three pixels of two bytes are six bytes, padded to four-byte rows.
    let l = SurfaceLayout::new(PixelFormat::Rgb565, 3, 2, 4).unwrap();
    assert_eq!(l.plane(0).unwrap().bytes_per_row, 8);
    assert_eq!(l.total_size(), 16);
}

#[test]
fn headless_a_subsampled_buffer_with_an_odd_height_is_refused() {
    assert_eq!(SurfaceLayout::new(PixelFormat::Nv12, 4, 5, 1), Err(LayoutError::OddSize));
}

#[test]
fn headless_fourcc_codes_name_the_formats_the_graphics_stack_uses() {
    assert_eq!(PixelFormat::Argb8888.fourcc(), *b"AR24");
    assert_eq!(PixelFormat::from_fourcc(*b"YU12"), Some(PixelFormat::I420));
    assert_eq!(PixelFormat::I420.plane_count(), 3);
}

// Window capture filter (capture_filter)

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn window(id: u32, pid: u32, frame: Rect) -> CaptureWindow {
    CaptureWindow { id, title: format!("window {id}"), app_pid: pid, frame, on_screen: true, desktop: false }
}

/// Two displays. Window 1 is covered by window 2 on display 1 (front to back: 2, then 1), and window 3 sits on display 2.
fn two_displays() -> ShareableContent {
    let displays = vec![
        CaptureDisplay { id: 1, frame: rect(0.0, 0.0, 1000.0, 800.0) },
        CaptureDisplay { id: 2, frame: rect(1000.0, 0.0, 1000.0, 800.0) },
    ];
    let windows = vec![
        window(2, 20, rect(150.0, 150.0, 300.0, 300.0)),
        window(1, 10, rect(100.0, 100.0, 300.0, 300.0)),
        window(3, 30, rect(1100.0, 100.0, 300.0, 300.0)),
    ];
    ShareableContent::new(displays, Vec::new(), windows)
}

#[test]
fn headless_a_window_filter_captures_a_covered_window_by_its_id() {
    let content = two_displays();
    let captured: Vec<u32> = ContentFilter::window(1).captured(&content).iter().map(|w| w.id).collect();
    assert_eq!(captured, vec![1], "window 1 is covered by window 2 but is still the one captured");
}

#[test]
fn headless_a_window_filter_for_an_unknown_window_captures_nothing() {
    let content = two_displays();
    assert!(ContentFilter::window(99).captured(&content).is_empty());
}

#[test]
fn headless_a_display_rule_keeps_only_the_windows_of_that_display() {
    let content = two_displays();
    let filter = ContentFilter::display(2, Rule::IncludeWindows(vec![1, 3]));
    let captured: Vec<u32> = filter.captured(&content).iter().map(|w| w.id).collect();
    assert_eq!(captured, vec![3], "window 1 lives on display 1 and is left out");
}

// Multicast membership (multicast)

#[test]
fn headless_a_group_is_joined_once_and_kept_in_join_order() {
    let mut m = MulticastMembership::new();
    m.join(Ipv4Addr::new(239, 255, 0, 1)).unwrap();
    m.join(Ipv4Addr::new(224, 0, 0, 251)).unwrap();
    assert_eq!(m.groups(), &[Ipv4Addr::new(239, 255, 0, 1), Ipv4Addr::new(224, 0, 0, 251)]);
    assert_eq!(m.join(Ipv4Addr::new(239, 255, 0, 1)), Err(MembershipError::AlreadyJoined));
}

#[test]
fn headless_a_unicast_address_cannot_join_a_group() {
    let mut m = MulticastMembership::new();
    assert_eq!(m.join(Ipv4Addr::new(10, 0, 0, 1)), Err(MembershipError::NotMulticast));
    assert_eq!(m.join(Ipv4Addr::new(240, 0, 0, 0)), Err(MembershipError::NotMulticast));
    assert!(m.groups().is_empty());
}

#[test]
fn headless_leaving_a_group_that_is_not_joined_is_refused() {
    let mut m = MulticastMembership::new();
    let group = Ipv4Addr::new(239, 1, 2, 3);
    assert_eq!(m.leave(group), Err(MembershipError::NotJoined));
    m.join(group).unwrap();
    m.leave(group).unwrap();
    assert!(!m.is_member(group));
}

#[test]
fn headless_the_send_options_default_to_the_local_link_with_loopback() {
    let mut m = MulticastMembership::new();
    assert_eq!(m.time_to_live(), 1);
    assert!(m.loopback());
    assert_eq!(m.set_time_to_live(0), Err(MembershipError::ZeroTimeToLive));
    m.set_time_to_live(16).unwrap();
    m.set_loopback(false);
    assert_eq!((m.time_to_live(), m.loopback()), (16, false));
}

// TLS options (tls_options)

#[test]
fn headless_the_default_tls_options_allow_1_2_and_1_3_against_the_system_roots() {
    let o = TlsOptions::new();
    assert!(o.allows(TlsVersion::Tls12));
    assert!(o.allows(TlsVersion::Tls13));
    assert_eq!(o.trust(), &Trust::SystemRoots);
    assert_eq!(o.server_name(), None);
}

#[test]
fn headless_a_version_range_with_a_minimum_above_the_maximum_is_refused() {
    let err = TlsOptions::new().with_versions(TlsVersion::Tls13, TlsVersion::Tls12).unwrap_err();
    assert_eq!(err, TlsOptionsError::InvalidVersionRange);
    let only_new = TlsOptions::new().with_versions(TlsVersion::Tls13, TlsVersion::Tls13).unwrap();
    assert!(!only_new.allows(TlsVersion::Tls12));
}

#[test]
fn headless_application_protocols_keep_their_preference_order_and_reject_bad_names() {
    let o = TlsOptions::new().with_alpn(&["h2", "http/1.1"]).unwrap();
    assert_eq!(o.alpn(), &["h2".to_string(), "http/1.1".to_string()]);
    assert_eq!(TlsOptions::new().with_alpn(&[""]), Err(TlsOptionsError::InvalidProtocolName));
    let long = "p".repeat(256);
    assert_eq!(TlsOptions::new().with_alpn(&[long.as_str()]), Err(TlsOptionsError::InvalidProtocolName));
    let limit = "p".repeat(255);
    assert!(TlsOptions::new().with_alpn(&[limit.as_str()]).is_ok());
}

#[test]
fn headless_a_server_name_must_be_a_plain_host_name() {
    let o = TlsOptions::new().with_server_name("api.example.com").unwrap();
    assert_eq!(o.server_name(), Some("api.example.com"));
    assert_eq!(TlsOptions::new().with_server_name("bad name"), Err(TlsOptionsError::InvalidServerName));
    assert_eq!(TlsOptions::new().with_server_name(""), Err(TlsOptionsError::InvalidServerName));
    let too_long = "a".repeat(254);
    assert_eq!(TlsOptions::new().with_server_name(&too_long), Err(TlsOptionsError::InvalidServerName));
}

#[test]
fn headless_a_pinned_trust_set_needs_at_least_one_digest() {
    assert_eq!(
        TlsOptions::new().with_trust(Trust::Pinned(Vec::new())),
        Err(TlsOptionsError::EmptyPinSet)
    );
    let pinned = TlsOptions::new().with_trust(Trust::Pinned(vec![[7u8; 32]])).unwrap();
    assert_eq!(pinned.trust(), &Trust::Pinned(vec![[7u8; 32]]));
}
