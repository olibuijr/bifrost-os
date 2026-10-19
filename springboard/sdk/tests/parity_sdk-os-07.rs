//! Parity sdk-os-07: headless tests for the system service surfaces that apps reach through the SDK. Pure host tests:
//! no network, no device, no display and no clock. Each test drives the public API of the module it names.

use std::io;
use std::net::Ipv4Addr;

use bifrost_app::device_registry::{changes, matching, Change, Device, Matching};
use bifrost_app::display_link::DisplayLink;
use bifrost_app::keychain_protection::{DeviceState, Protection};
use bifrost_app::local_auth::{Device as AuthDevice, LAContext, LAError, LAPolicy};
use bifrost_app::network_path::{IfaceKind, Interface, Path, PathMonitor, Status};
use bifrost_app::pixel_buffer::{PixelBuffer, PixelBufferError};
use bifrost_app::reachability::{classify, has_default_route, Reach};
use bifrost_app::system::authorization_lock::{AuthorizationLock, ButtonType, LockState, PasswordError};
use bifrost_app::system::dma_buf::{self, DmaBufError, Descriptor, Plane};
use bifrost_app::system::effect_nodes::{Affine, CropNode, EffectError, EffectNode, Filter, Rect};
use bifrost_app::system::ipv4_config::{self, ConfigError, Method};
use bifrost_app::system::sprite_shader::{AttributeKind, Shader, ShaderError, Value};
use bifrost_app::system::update_integrity::{self, Expected, IntegrityError};
use bifrost_render::pixel_format::FourCc;

fn iface(name: &str, kind: IfaceKind) -> Interface {
    Interface { name: name.to_string(), kind }
}

// --- keychain protection classes -------------------------------------------------------------------------------------

#[test]
fn a_when_unlocked_item_is_unreadable_while_the_session_is_locked() {
    let locked = DeviceState { unlocked: false, unlocked_since_boot: true };
    assert!(!Protection::WhenUnlocked.readable(locked));
    assert!(Protection::AfterFirstUnlock.readable(locked));
    assert!(Protection::Always.readable(DeviceState { unlocked: false, unlocked_since_boot: false }));
}

#[test]
fn a_this_device_only_item_never_migrates_to_a_backup() {
    assert!(Protection::AfterFirstUnlock.migrates());
    assert!(!Protection::AfterFirstUnlockThisDeviceOnly.migrates());
    assert!(!Protection::AlwaysThisDeviceOnly.migrates());
}

#[test]
fn protection_wire_codes_round_trip_and_unknown_codes_are_refused() {
    assert_eq!(Protection::default(), Protection::AfterFirstUnlock);
    for code in 1..=6u8 {
        let class = Protection::from_code(code).expect("codes one to six are classes");
        assert_eq!(class.code(), code);
    }
    assert_eq!(Protection::from_code(0), None);
    assert_eq!(Protection::from_code(7), None);
}

// --- local authentication context ------------------------------------------------------------------------------------

#[test]
fn owner_authentication_needs_a_passcode_on_the_device() {
    let no_passcode = LAContext::new(AuthDevice { password_set: false });
    assert_eq!(no_passcode.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Err(LAError::PasscodeNotSet));
    let with_passcode = LAContext::new(AuthDevice { password_set: true });
    assert_eq!(with_passcode.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Ok(()));
}

#[test]
fn biometry_is_never_available_on_this_system() {
    let context = LAContext::new(AuthDevice { password_set: true });
    assert_eq!(context.can_evaluate_policy(LAPolicy::DeviceOwnerAuthenticationWithBiometrics), Err(LAError::BiometryNotAvailable));
}

#[test]
fn an_invalidated_context_refuses_every_evaluation() {
    let mut context = LAContext::new(AuthDevice { password_set: true });
    context.invalidate();
    assert_eq!(context.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Err(LAError::InvalidContext));
}

#[test]
fn local_auth_error_wire_names_decode_to_their_errors() {
    assert_eq!(LAError::from_wire("user_cancel"), Some(LAError::UserCancel));
    assert_eq!(LAError::UserCancel.wire(), "user_cancel");
    assert_eq!(LAError::from_wire("no_such_error"), None);
}

// --- network path and change notifications --------------------------------------------------------------------------

#[test]
fn a_wired_link_with_an_address_is_satisfied() {
    let path = Path::new(vec![iface("eth0", IfaceKind::Wired)], true, false);
    assert_eq!(path.status, Status::Satisfied);
    assert!(!path.expensive);
}

#[test]
fn a_link_without_an_address_requires_a_connection() {
    let path = Path::new(vec![iface("eth0", IfaceKind::Wired)], false, false);
    assert_eq!(path.status, Status::RequiresConnection);
}

#[test]
fn loopback_alone_is_unsatisfied() {
    let path = Path::new(vec![iface("lo", IfaceKind::Loopback)], true, true);
    assert_eq!(path.status, Status::Unsatisfied);
    assert_eq!(Path::offline().status, Status::Unsatisfied);
}

#[test]
fn wired_links_come_before_wifi_in_the_path_order() {
    let path = Path::new(vec![iface("wlan0", IfaceKind::Wifi), iface("eth0", IfaceKind::Wired)], true, false);
    assert_eq!(path.interfaces[0].name, "eth0");
    assert_eq!(path.interfaces[1].name, "wlan0");
}

#[test]
fn a_cellular_primary_link_is_expensive() {
    assert!(Path::new(vec![iface("wwan0", IfaceKind::Cellular)], true, false).expensive);
    assert!(!Path::new(vec![iface("wlan0", IfaceKind::Wifi)], true, false).expensive);
}

#[test]
fn the_path_monitor_reports_a_change_once_and_stays_quiet_for_the_same_path() {
    let mut monitor = PathMonitor::new();
    assert_eq!(monitor.current(), &Path::offline());
    assert!(monitor.update(Path::offline()).is_none(), "offline to offline is not a change");
    let online = Path::new(vec![iface("eth0", IfaceKind::Wired)], true, false);
    assert!(monitor.update(online.clone()).is_some());
    assert!(monitor.update(online).is_none(), "the same path is reported once");
}

// --- reachability ---------------------------------------------------------------------------------------------------

#[test]
fn a_routing_table_with_a_default_route_has_one() {
    let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                 eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    assert!(has_default_route(table));
}

#[test]
fn a_routing_table_with_only_a_subnet_route_has_no_default_route() {
    let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                 eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
    assert!(!has_default_route(table));
    assert!(!has_default_route(""));
}

#[test]
fn a_refused_connection_is_reported_as_refused() {
    assert_eq!(classify(&io::Error::from(io::ErrorKind::ConnectionRefused)), Reach::Refused);
    assert_eq!(classify(&io::Error::from(io::ErrorKind::TimedOut)), Reach::TimedOut);
}

// --- device registry (the kernel's device classes) ------------------------------------------------------------------

#[test]
fn matching_selects_devices_of_a_class_with_a_property_value() {
    let devices = vec![
        Device::new("drm", "card0").with_property("status", "connected"),
        Device::new("drm", "card1").with_property("status", "disconnected"),
        Device::new("video4linux", "video0"),
    ];
    let found = matching(&devices, &Matching::class("drm").with_property("status", "connected"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "card0");
}

#[test]
fn a_device_that_appears_is_an_arrival_and_one_that_goes_is_a_removal() {
    let card0 = Device::new("drm", "card0");
    let card1 = Device::new("drm", "card1");
    let video0 = Device::new("video4linux", "video0");
    assert_eq!(changes(&[card0.clone()], &[card0.clone(), card1.clone()]), vec![Change::Arrived(card1)]);
    assert_eq!(changes(&[card0.clone(), video0.clone()], &[card0]), vec![Change::Removed(video0)]);
}

#[test]
fn a_property_change_alone_is_not_reported_as_a_device_change() {
    let before = vec![Device::new("drm", "card0").with_property("status", "connected")];
    let after = vec![Device::new("drm", "card0").with_property("status", "disconnected")];
    assert!(changes(&before, &after).is_empty());
}

// --- pixel buffers and display timing ------------------------------------------------------------------------------

#[test]
fn a_pixel_buffer_rows_are_packed_tightly() {
    let buffer = PixelBuffer::new(FourCc::new(*b"AR24"), 4, 2).expect("ARGB8888 is a known format");
    assert_eq!(buffer.bytes_per_row(0), Some(16));
    assert_eq!(buffer.data_size(), 32);
}

#[test]
fn a_pixel_buffer_refuses_an_unknown_format_and_an_empty_size() {
    assert!(matches!(PixelBuffer::new(FourCc::new(*b"ZZZZ"), 4, 2), Err(PixelBufferError::UnknownFormat)));
    assert!(matches!(PixelBuffer::new(FourCc::new(*b"AR24"), 0, 2), Err(PixelBufferError::InvalidSize)));
}

#[test]
fn bytes_are_reachable_only_under_a_lock_and_a_read_write_unlock_counts_as_a_change() {
    let mut buffer = PixelBuffer::new(FourCc::new(*b"AR24"), 4, 2).expect("ARGB8888 is a known format");
    assert!(buffer.base_address(0).is_none(), "no lock is held");
    buffer.lock(true);
    assert!(buffer.base_address(0).is_some());
    assert!(buffer.unlock(false).is_err(), "no read-write lock is held");
    assert!(buffer.unlock(true).is_ok());
    buffer.lock(false);
    assert!(buffer.unlock(false).is_ok());
    assert_eq!(buffer.revision(), 1);
}

#[test]
fn a_display_link_reports_each_refresh_once_and_counts_the_missed_ones() {
    assert!(DisplayLink::new(0).is_none());
    let mut link = DisplayLink::new(100).expect("100 Hz is a rate");
    assert_eq!(link.refresh_period(), 10_000_000);
    assert!(link.tick(1_000).is_none(), "a stopped link reports nothing");
    link.start(1_000);
    let first = link.tick(1_000).expect("refresh zero has begun");
    assert_eq!(first.frame_number, 0);
    assert!(link.tick(1_000).is_none(), "the same refresh is reported once");
    let later = link.tick(1_000 + 3 * 10_000_000 + 5).expect("refresh three has begun");
    assert_eq!(later.frame_number, 3);
    assert_eq!(later.video_refresh_period, 10_000_000);
    assert_eq!(link.missed_refreshes(), 2);
}

// --- update download integrity -------------------------------------------------------------------------------------

const ABC_DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn a_download_with_the_announced_size_and_digest_is_accepted() {
    let expected = Expected::new(3, ABC_DIGEST).expect("a 64-digit digest");
    assert_eq!(update_integrity::verify(b"abc", &expected), Ok(ABC_DIGEST.to_string()));
}

#[test]
fn a_download_of_the_wrong_size_is_refused_before_its_digest_is_checked() {
    let expected = Expected::new(4, ABC_DIGEST).expect("a 64-digit digest");
    assert_eq!(update_integrity::verify(b"abc", &expected), Err(IntegrityError::SizeMismatch { expected: 4, actual: 3 }));
}

#[test]
fn a_download_with_another_digest_is_refused() {
    let expected = Expected::new(3, &"0".repeat(64)).expect("a 64-digit digest");
    assert_eq!(update_integrity::verify(b"abc", &expected), Err(IntegrityError::DigestMismatch));
}

#[test]
fn an_announced_digest_must_be_64_hexadecimal_digits_and_is_stored_in_lowercase() {
    assert_eq!(Expected::new(3, "abc"), Err(IntegrityError::MalformedDigest));
    assert_eq!(Expected::new(3, &"g".repeat(64)), Err(IntegrityError::MalformedDigest));
    let upper = Expected::new(3, &ABC_DIGEST.to_ascii_uppercase()).expect("upper case is accepted");
    assert_eq!(upper.sha256, ABC_DIGEST);
}

#[test]
fn the_empty_download_has_the_digest_of_no_bytes() {
    let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let expected = Expected::new(0, empty).expect("a 64-digit digest");
    assert_eq!(update_integrity::verify(b"", &expected), Ok(empty.to_string()));
}

// --- wired IPv4 configuration -----------------------------------------------------------------------------------

#[test]
fn a_manual_address_with_a_router_on_its_subnet_is_accepted() {
    let address = Ipv4Addr::new(192, 168, 1, 20);
    let router = Ipv4Addr::new(192, 168, 1, 1);
    assert_eq!(
        ipv4_config::manual(address, 24, Some(router)),
        Ok(Method::Manual { address, prefix: 24, router: Some(router) })
    );
}

#[test]
fn a_manual_address_must_not_be_the_network_or_broadcast_address() {
    assert_eq!(ipv4_config::manual(Ipv4Addr::new(192, 168, 1, 0), 24, None), Err(ConfigError::AddressIsNetwork));
    assert_eq!(ipv4_config::manual(Ipv4Addr::new(192, 168, 1, 255), 24, None), Err(ConfigError::AddressIsBroadcast));
}

#[test]
fn a_point_to_point_subnet_keeps_both_of_its_addresses() {
    assert!(ipv4_config::manual(Ipv4Addr::new(10, 0, 0, 0), 31, None).is_ok());
}

#[test]
fn a_prefix_above_32_and_a_router_off_the_subnet_are_refused() {
    assert_eq!(ipv4_config::manual(Ipv4Addr::new(10, 0, 0, 5), 33, None), Err(ConfigError::PrefixOutOfRange(33)));
    let router = Ipv4Addr::new(192, 168, 2, 1);
    assert_eq!(ipv4_config::manual(Ipv4Addr::new(192, 168, 1, 20), 24, Some(router)), Err(ConfigError::RouterOutsideSubnet));
}

#[test]
fn masks_and_prefixes_convert_both_ways() {
    assert_eq!(ipv4_config::mask(24), Some(0xFFFF_FF00));
    assert_eq!(ipv4_config::mask(0), Some(0));
    assert_eq!(ipv4_config::mask(32), Some(u32::MAX));
    assert_eq!(ipv4_config::mask(33), None);
    assert_eq!(ipv4_config::prefix_of_mask(Ipv4Addr::new(255, 255, 255, 0)), Some(24));
    assert_eq!(ipv4_config::prefix_of_mask(Ipv4Addr::new(255, 0, 255, 0)), None);
    assert_eq!(ipv4_config::network_of(Ipv4Addr::new(10, 20, 30, 40), 16), Some(Ipv4Addr::new(10, 20, 0, 0)));
}

// --- dma-buf surface descriptors ---------------------------------------------------------------------------------

fn argb_descriptor(width: u32, height: u32, stride: u32, offset: u32) -> Descriptor {
    Descriptor {
        width,
        height,
        format: dma_buf::ARGB8888,
        modifier: dma_buf::MODIFIER_LINEAR,
        planes: vec![Plane { offset, stride }],
    }
}

#[test]
fn a_linear_argb_descriptor_covers_its_rows_inside_the_file() {
    assert_eq!(argb_descriptor(4, 2, 16, 0).validate(32), Ok(32));
}

#[test]
fn the_plane_offset_counts_toward_the_bytes_a_descriptor_needs() {
    assert_eq!(argb_descriptor(4, 2, 16, 8).validate(40), Ok(40));
    assert_eq!(argb_descriptor(4, 2, 16, 8).validate(39), Err(DmaBufError::OutOfBounds { needed: 40, size: 39 }));
}

#[test]
fn a_stride_shorter_than_one_row_of_pixels_is_refused() {
    assert_eq!(argb_descriptor(4, 2, 15, 0).validate(32), Err(DmaBufError::StrideTooSmall { stride: 15, minimum: 16 }));
}

#[test]
fn a_descriptor_with_a_tiled_modifier_or_another_plane_count_is_refused() {
    let mut tiled = argb_descriptor(4, 2, 16, 0);
    tiled.modifier = 1;
    assert_eq!(tiled.validate(32), Err(DmaBufError::UnsupportedModifier(1)));
    let mut two_planes = argb_descriptor(4, 2, 16, 0);
    two_planes.planes.push(Plane { offset: 0, stride: 16 });
    assert_eq!(two_planes.validate(32), Err(DmaBufError::PlaneCount(2)));
}

#[test]
fn a_descriptor_in_a_planar_format_or_with_an_empty_size_is_refused() {
    let mut planar = argb_descriptor(4, 2, 16, 0);
    planar.format = dma_buf::fourcc(*b"NV16");
    assert_eq!(planar.validate(32), Err(DmaBufError::UnsupportedFormat(dma_buf::fourcc(*b"NV16"))));
    assert_eq!(argb_descriptor(0, 2, 16, 0).validate(32), Err(DmaBufError::EmptySize));
}

#[test]
fn the_argb_format_code_is_the_little_endian_letters_ar24() {
    assert_eq!(dma_buf::ARGB8888, 0x3432_5241);
}

// --- shader descriptions -------------------------------------------------------------------------------------------

#[test]
fn a_shader_with_named_attributes_and_uniforms_validates() {
    let mut shader = Shader::new("return color * gain;").with_attribute("position", AttributeKind::Vec2);
    shader.set_uniform("gain", Value::Float(1.5));
    shader.set_uniform("_tint_1", Value::Vec4([1.0, 0.5, 0.25, 1.0]));
    assert_eq!(shader.validate(), Ok(()));
}

#[test]
fn a_shader_needs_a_source_with_something_in_it() {
    assert_eq!(Shader::new("   ").validate(), Err(ShaderError::EmptySource));
}

#[test]
fn a_shader_name_must_be_an_identifier() {
    let shader = Shader::new("return color;").with_attribute("1bad", AttributeKind::Float);
    assert_eq!(shader.validate(), Err(ShaderError::InvalidName("1bad".to_string())));
}

#[test]
fn an_attribute_and_a_uniform_cannot_share_a_name() {
    let mut shader = Shader::new("return color;").with_attribute("tint", AttributeKind::Float);
    shader.set_uniform("tint", Value::Float(0.5));
    assert_eq!(shader.validate(), Err(ShaderError::DuplicateName("tint".to_string())));
}

#[test]
fn a_non_finite_uniform_value_is_refused() {
    let mut shader = Shader::new("return color;");
    shader.set_uniform("gain", Value::Float(f32::NAN));
    assert_eq!(shader.validate(), Err(ShaderError::NonFiniteValue("gain".to_string())));
}

#[test]
fn setting_a_uniform_again_replaces_its_value() {
    let mut shader = Shader::new("return color;");
    shader.set_uniform("a", Value::Float(1.0));
    shader.set_uniform("a", Value::Float(2.0));
    assert_eq!(shader.uniforms.len(), 1);
    assert_eq!(shader.uniform("a"), Some(Value::Float(2.0)));
}

// --- crop, transform and effect nodes ---------------------------------------------------------------------------

#[test]
fn two_overlapping_rectangles_share_their_overlap() {
    let a = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
    let b = Rect { x: 5.0, y: 5.0, width: 10.0, height: 10.0 };
    assert_eq!(a.intersection(b), Some(Rect { x: 5.0, y: 5.0, width: 5.0, height: 5.0 }));
}

#[test]
fn rectangles_that_only_touch_share_no_area() {
    let a = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
    let b = Rect { x: 10.0, y: 0.0, width: 5.0, height: 5.0 };
    assert_eq!(a.intersection(b), None);
}

#[test]
fn a_crop_node_shows_only_the_part_of_a_child_inside_its_mask() {
    let crop = CropNode { mask: Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 } };
    let child = Rect { x: -10.0, y: 10.0, width: 30.0, height: 100.0 };
    assert_eq!(crop.visible(child), Some(Rect { x: 0.0, y: 10.0, width: 20.0, height: 40.0 }));
    assert_eq!(crop.visible(Rect { x: 200.0, y: 0.0, width: 10.0, height: 10.0 }), None);
}

#[test]
fn a_transform_applies_the_first_step_before_the_second() {
    let moved_then_scaled = Affine::translation(10.0, 0.0).then(Affine::scale(2.0, 2.0));
    assert_eq!(moved_then_scaled.apply(1.0, 1.0), (22.0, 2.0));
    let scaled_then_moved = Affine::scale(2.0, 2.0).then(Affine::translation(10.0, 0.0));
    assert_eq!(scaled_then_moved.apply(1.0, 1.0), (12.0, 2.0));
}

#[test]
fn a_quarter_turn_maps_the_x_axis_onto_the_y_axis() {
    let (x, y) = Affine::rotation(std::f64::consts::FRAC_PI_2).apply(1.0, 0.0);
    assert!(x.abs() < 1e-9, "x is {x}");
    assert!((y - 1.0).abs() < 1e-9, "y is {y}");
    assert_eq!(Affine::IDENTITY.apply(3.0, 4.0), (3.0, 4.0));
}

#[test]
fn a_blur_radius_must_be_a_finite_number_of_zero_or_more() {
    let blur = |radius| EffectNode { filter: Some(Filter::Blur { radius }), rasterize: true };
    assert_eq!(blur(4.0).validate(), Ok(()));
    assert_eq!(blur(-1.0).validate(), Err(EffectError::InvalidRadius(-1.0)));
    assert!(matches!(blur(f64::NAN).validate(), Err(EffectError::InvalidRadius(_))));
    assert_eq!(EffectNode { filter: None, rasterize: false }.validate(), Ok(()));
}

// --- authorization lock view ------------------------------------------------------------------------------------

#[test]
fn a_new_gate_is_locked_and_offers_to_unlock() {
    let gate = AuthorizationLock::new();
    assert_eq!(gate.state(), LockState::Locked);
    assert_eq!(gate.button(), ButtonType::Unlock);
    assert!(gate.message().is_none());
}

#[test]
fn a_correct_password_unlocks_the_gate_with_its_token() {
    let mut gate = AuthorizationLock::new();
    assert!(gate.authenticated("9f2c"));
    assert!(gate.is_unlocked());
    assert_eq!(gate.token(), Some("9f2c"));
    assert_eq!(gate.button(), ButtonType::Lock);
}

#[test]
fn an_empty_token_does_not_unlock_the_gate() {
    let mut gate = AuthorizationLock::new();
    assert!(!gate.authenticated(""));
    assert_eq!(gate.state(), LockState::Locked);
}

#[test]
fn a_wrong_password_keeps_the_gate_locked_and_shows_the_wait() {
    let mut gate = AuthorizationLock::new();
    gate.refused(PasswordError::from_code("wrong_password", 2, 5));
    assert_eq!(gate.state(), LockState::Locked);
    assert_eq!(gate.message().as_deref(), Some("Incorrect password. Try again in 5 seconds."));
}

#[test]
fn a_wrong_password_without_a_wait_has_a_short_message() {
    assert_eq!(PasswordError::from_code("wrong_password", 1, 0).message(), "Incorrect password.");
}

#[test]
fn a_locked_out_gate_names_the_remaining_wait() {
    assert_eq!(PasswordError::from_code("locked_out", 0, 1).message(), "Too many failed attempts. Try again in 1 second.");
}

#[test]
fn a_missing_password_and_an_unknown_code_are_distinct_errors() {
    assert_eq!(PasswordError::from_code("no_password", 0, 0), PasswordError::NoPassword);
    assert_eq!(
        PasswordError::from_code("internal", 0, 0),
        PasswordError::Unclassified("internal".to_string())
    );
}

#[test]
fn locking_again_drops_the_token() {
    let mut gate = AuthorizationLock::new();
    gate.authenticated("9f2c");
    gate.lock();
    assert_eq!(gate.state(), LockState::Locked);
    assert_eq!(gate.token(), None);
}

// --- launch policy and sandbox grants (the shell's app policy, on Linux) ----------------------------------------

#[cfg(target_os = "linux")]
fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[cfg(target_os = "linux")]
#[test]
fn a_launch_with_known_permissions_is_allowed_and_grants_exactly_those() {
    use bifrost_app::system::execution_policy::check;
    let policy = check("is.olibuijr.notes", &strings(&["network", "audio"])).expect("both permissions are known");
    assert!(policy.network && policy.audio);
    assert!(!policy.storage);
}

#[cfg(target_os = "linux")]
#[test]
fn a_launch_with_an_unknown_permission_is_refused_by_name() {
    use bifrost_app::system::execution_policy::{check, LaunchRefusal};
    assert_eq!(
        check("is.olibuijr.notes", &strings(&["teleport"])),
        Err(LaunchRefusal::UnsupportedPermission("teleport".to_string()))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn remote_desktop_needs_network_and_storage_and_only_the_remote_app_may_use_it() {
    use bifrost_app::system::execution_policy::{check, LaunchRefusal};
    assert_eq!(
        check("is.olibuijr.remote.desktop", &strings(&["remote-desktop"])),
        Err(LaunchRefusal::RemoteNeedsNetworkAndStorage)
    );
    assert_eq!(
        check("is.olibuijr.notes", &strings(&["remote-desktop"])),
        Err(LaunchRefusal::UnsupportedPermission("remote-desktop".to_string()))
    );
    let granted = check("is.olibuijr.remote.desktop", &strings(&["remote-desktop", "network", "storage"]));
    assert!(granted.expect("with network and storage").remote);
}

#[cfg(target_os = "linux")]
#[test]
fn each_refusal_has_its_own_code_in_the_execution_policy_domain() {
    use bifrost_app::system::execution_policy::{LaunchRefusal, DOMAIN};
    assert_eq!(DOMAIN, "is.olibuijr.bifrost.execution-policy");
    assert_eq!(LaunchRefusal::UnsupportedPermission("x".to_string()).code(), 1);
    assert_eq!(LaunchRefusal::RemoteNeedsNetworkAndStorage.code(), 2);
    assert_eq!(LaunchRefusal::Unclassified("x".to_string()).code(), 3);
}

#[cfg(target_os = "linux")]
#[test]
fn a_refusal_the_model_does_not_name_keeps_the_shell_message() {
    use bifrost_app::system::execution_policy::LaunchRefusal;
    let error = io::Error::other("disk gone");
    assert_eq!(LaunchRefusal::from_error(&error), LaunchRefusal::Unclassified("disk gone".to_string()));
}

#[cfg(target_os = "linux")]
#[test]
fn a_default_policy_grants_no_sandbox_capability() {
    use bifrost_app::system::app_sandbox::{granted, permits, Capability};
    use springboard::bare::app_policy::Policy;
    assert!(granted(&Policy::default()).is_empty());
    assert!(!permits(&Policy::default(), Capability::Network));
}

#[cfg(target_os = "linux")]
#[test]
fn the_sandbox_grants_list_the_manifest_permissions_in_manifest_order() {
    use bifrost_app::system::app_sandbox::{granted, permits, Capability};
    use springboard::bare::app_policy::Policy;
    let policy = Policy::from_permissions("is.olibuijr.notes", &strings(&["network", "keychain"])).expect("known permissions");
    assert_eq!(granted(&policy), vec![Capability::Network, Capability::Keychain]);
    assert!(permits(&policy, Capability::Keychain));
    assert!(!permits(&policy, Capability::Audio));
    assert_eq!(Capability::RemoteDesktop.name(), "remote-desktop");
}
