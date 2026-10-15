//! Parity sdk-rest-16: headless tests for force-feedback cooperation and coordinates, and for the network provider host.
//! Each test drives the public API with concrete values. No device node, network, clock or process is used.

use bifrost_core::extension_process::Error as ProcessError;
use bifrost_core::ff_cooperation::{Access, Arbiter, CoordinateError, CoordinateSystem, CooperationError, CooperativeLevel, Direction, Focus};
use bifrost_core::provider_host::{ConfigError, Configuration, Point, ProviderHost};

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-3
}

fn level(access: Access, focus: Focus) -> CooperativeLevel {
    CooperativeLevel::new(access, focus).unwrap()
}

// ---------------------------------------------------------------------------------------------------------------------
// ff_cooperation: cooperative levels and the holder of a device
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_background_app_cannot_take_exclusive_access() {
    assert_eq!(CooperativeLevel::new(Access::Exclusive, Focus::Background), Err(CooperationError::ExclusiveInBackground));
    assert_eq!(CooperativeLevel::new(Access::Exclusive, Focus::Foreground).unwrap().access(), Access::Exclusive);
}

#[test]
fn a_shared_background_level_is_allowed_and_keeps_its_focus() {
    let shared = CooperativeLevel::new(Access::Shared, Focus::Background).unwrap();
    assert_eq!(shared.access(), Access::Shared);
    assert_eq!(shared.focus(), Focus::Background);
}

#[test]
fn an_exclusive_holder_blocks_every_other_app() {
    let mut arbiter = Arbiter::new();
    arbiter.request("wheel0", "racer", level(Access::Exclusive, Focus::Foreground)).unwrap();
    assert_eq!(arbiter.request("wheel0", "music", level(Access::Shared, Focus::Foreground)), Err(CooperationError::DeviceHeldExclusively));
    assert_eq!(arbiter.request("wheel0", "racer2", level(Access::Exclusive, Focus::Foreground)), Err(CooperationError::DeviceInUse));
    assert_eq!(arbiter.exclusive_holder("wheel0"), Some("racer"));
    assert_eq!(arbiter.holder_count("wheel0"), 1);
}

#[test]
fn shared_holders_coexist_and_refuse_an_exclusive_request() {
    let mut arbiter = Arbiter::new();
    arbiter.request("pad1", "a", level(Access::Shared, Focus::Foreground)).unwrap();
    arbiter.request("pad1", "b", level(Access::Shared, Focus::Background)).unwrap();
    assert_eq!(arbiter.holder_count("pad1"), 2);
    assert_eq!(arbiter.exclusive_holder("pad1"), None);
    assert_eq!(arbiter.request("pad1", "c", level(Access::Exclusive, Focus::Foreground)), Err(CooperationError::DeviceInUse));
    assert_eq!(arbiter.holder_count("pad1"), 2, "a refused request leaves the holders as they were");
}

#[test]
fn an_app_can_change_its_own_level_on_a_device() {
    let mut arbiter = Arbiter::new();
    arbiter.request("pad1", "a", level(Access::Shared, Focus::Foreground)).unwrap();
    arbiter.request("pad1", "a", level(Access::Exclusive, Focus::Foreground)).unwrap();
    assert_eq!(arbiter.exclusive_holder("pad1"), Some("a"));
    assert_eq!(arbiter.holder_count("pad1"), 1);
    assert_eq!(arbiter.level("pad1", "a"), Some(level(Access::Exclusive, Focus::Foreground)));
}

#[test]
fn releasing_the_last_holder_frees_the_device_for_exclusive_use() {
    let mut arbiter = Arbiter::new();
    arbiter.request("wheel0", "a", level(Access::Exclusive, Focus::Foreground)).unwrap();
    arbiter.release("wheel0", "a").unwrap();
    assert_eq!(arbiter.holder_count("wheel0"), 0);
    arbiter.request("wheel0", "b", level(Access::Exclusive, Focus::Foreground)).unwrap();
    assert_eq!(arbiter.exclusive_holder("wheel0"), Some("b"));
}

#[test]
fn releasing_a_device_the_app_does_not_hold_is_refused() {
    let mut arbiter = Arbiter::new();
    arbiter.request("wheel0", "a", level(Access::Shared, Focus::Foreground)).unwrap();
    assert_eq!(arbiter.release("wheel0", "b"), Err(CooperationError::NotHolder));
    assert_eq!(arbiter.release("pad9", "a"), Err(CooperationError::NotHolder));
    assert_eq!(arbiter.holder_count("wheel0"), 1);
}

#[test]
fn two_devices_are_arbitrated_separately() {
    let mut arbiter = Arbiter::new();
    arbiter.request("wheel0", "racer", level(Access::Exclusive, Focus::Foreground)).unwrap();
    arbiter.request("pad1", "music", level(Access::Exclusive, Focus::Foreground)).unwrap();
    assert_eq!(arbiter.exclusive_holder("wheel0"), Some("racer"));
    assert_eq!(arbiter.exclusive_holder("pad1"), Some("music"));
}

// ---------------------------------------------------------------------------------------------------------------------
// ff_cooperation: force directions in polar and Cartesian form
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_polar_angle_wraps_into_zero_to_360_degrees() {
    assert_eq!(Direction::polar(-90.0), Ok(Direction::Polar(270.0)));
    assert_eq!(Direction::polar(720.0), Ok(Direction::Polar(0.0)));
    assert_eq!(Direction::polar(45.0), Ok(Direction::Polar(45.0)));
}

#[test]
fn a_non_finite_direction_is_refused() {
    assert_eq!(Direction::polar(f32::NAN), Err(CoordinateError::NotFinite));
    assert_eq!(Direction::polar(f32::INFINITY), Err(CoordinateError::NotFinite));
    assert_eq!(Direction::cartesian(1.0, f32::NAN), Err(CoordinateError::NotFinite));
}

#[test]
fn the_zero_vector_has_no_direction() {
    assert_eq!(Direction::cartesian(0.0, 0.0), Err(CoordinateError::ZeroVector));
}

#[test]
fn a_cartesian_vector_gives_the_angle_counter_clockwise_from_the_x_axis() {
    assert!(near(Direction::cartesian(0.0, 2.0).unwrap().angle_degrees(), 90.0));
    assert!(near(Direction::cartesian(-1.0, 0.0).unwrap().angle_degrees(), 180.0));
    assert!(near(Direction::cartesian(1.0, -1.0).unwrap().angle_degrees(), 315.0));
}

#[test]
fn a_polar_direction_gives_its_unit_vector() {
    let (x, y) = Direction::polar(0.0).unwrap().unit_vector();
    assert!(near(x, 1.0) && near(y, 0.0));
    let (x, y) = Direction::polar(90.0).unwrap().unit_vector();
    assert!(near(x, 0.0) && near(y, 1.0));
}

#[test]
fn a_direction_converts_between_polar_and_cartesian_form() {
    match Direction::polar(180.0).unwrap().in_system(CoordinateSystem::Cartesian) {
        Direction::Cartesian { x, y } => assert!(near(x, -1.0) && near(y, 0.0)),
        other => panic!("expected a Cartesian direction, got {other:?}"),
    }
    match Direction::cartesian(0.0, 5.0).unwrap().in_system(CoordinateSystem::Polar) {
        Direction::Polar(degrees) => assert!(near(degrees, 90.0)),
        other => panic!("expected a polar direction, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// provider_host: configurations, registration and the one enabled provider per point
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_configuration_needs_a_reverse_dns_bundle_identifier() {
    assert_eq!(Configuration::new("filter", Point::FilterData, "Filter"), Err(ConfigError::InvalidBundleId));
    assert_eq!(Configuration::new("is.olibuijr.-bad.filter", Point::FilterData, "Filter"), Err(ConfigError::InvalidBundleId));
    assert_eq!(Configuration::new("is..olibuijr.filter", Point::FilterData, "Filter"), Err(ConfigError::InvalidBundleId));
    let ok = Configuration::new("is.olibuijr.filter", Point::FilterData, "Filter").unwrap();
    assert_eq!(ok.bundle_id(), "is.olibuijr.filter");
    assert_eq!(ok.point(), Point::FilterData);
}

#[test]
fn a_provider_class_starts_with_a_letter_and_has_no_spaces() {
    assert_eq!(Configuration::new("is.olibuijr.filter", Point::FilterData, "1Filter"), Err(ConfigError::InvalidProviderClass));
    assert_eq!(Configuration::new("is.olibuijr.filter", Point::FilterData, "Filter Provider"), Err(ConfigError::InvalidProviderClass));
    let ok = Configuration::new("is.olibuijr.filter", Point::FilterData, "Filter.Provider_2").unwrap();
    assert_eq!(ok.provider_class(), "Filter.Provider_2");
}

#[test]
fn each_point_has_a_stable_name_that_parses_back() {
    for point in Point::ALL {
        assert_eq!(Point::parse(point.name()), Some(point));
    }
    assert_eq!(Point::parse("tunnel"), None);
    assert_eq!(Point::PacketTunnel.name(), "packet-tunnel");
}

#[test]
fn a_bundle_is_registered_once_per_point() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.vpn", Point::PacketTunnel, "Tunnel").unwrap()).unwrap();
    assert_eq!(
        host.register(Configuration::new("is.olibuijr.vpn", Point::PacketTunnel, "Tunnel").unwrap()),
        Err(ConfigError::Duplicate)
    );
    host.register(Configuration::new("is.olibuijr.vpn", Point::DnsProxy, "Resolver").unwrap()).unwrap();
}

#[test]
fn only_one_provider_is_enabled_per_point() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.proxy", Point::AppProxy, "Proxy").unwrap()).unwrap();
    host.register(Configuration::new("is.olibuijr.other", Point::AppProxy, "Other").unwrap()).unwrap();
    host.enable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    assert_eq!(
        host.enable("is.olibuijr.other", Point::AppProxy),
        Err(ConfigError::AnotherEnabled("is.olibuijr.proxy".to_string()))
    );
    host.enable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    assert_eq!(host.enabled_for(Point::AppProxy).map(Configuration::bundle_id), Some("is.olibuijr.proxy"));
}

#[test]
fn enabling_an_unregistered_provider_is_refused() {
    let mut host = ProviderHost::new();
    assert_eq!(host.enable("is.olibuijr.ghost", Point::FilterControl), Err(ConfigError::NotRegistered));
    host.register(Configuration::new("is.olibuijr.ghost", Point::FilterData, "Ghost").unwrap()).unwrap();
    assert_eq!(host.enable("is.olibuijr.ghost", Point::FilterControl), Err(ConfigError::NotRegistered));
}

#[test]
fn disabling_frees_the_point_for_another_provider() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.proxy", Point::AppProxy, "Proxy").unwrap()).unwrap();
    host.register(Configuration::new("is.olibuijr.other", Point::AppProxy, "Other").unwrap()).unwrap();
    host.enable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    host.disable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    assert_eq!(host.disable("is.olibuijr.proxy", Point::AppProxy), Err(ConfigError::NotEnabled));
    host.enable("is.olibuijr.other", Point::AppProxy).unwrap();
    assert_eq!(host.enabled_for(Point::AppProxy).map(Configuration::provider_class), Some("Other"));
}

// ---------------------------------------------------------------------------------------------------------------------
// provider_host: the provider's process and its connections
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn starting_a_provider_without_one_enabled_is_refused() {
    let mut host = ProviderHost::new();
    assert_eq!(host.start(Point::DnsProxy), Err(ConfigError::NotEnabled));
}

#[test]
fn a_provider_keeps_one_process_per_point() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.vpn", Point::PacketTunnel, "Tunnel").unwrap()).unwrap();
    host.register(Configuration::new("is.olibuijr.vpn", Point::DnsProxy, "Resolver").unwrap()).unwrap();
    host.enable("is.olibuijr.vpn", Point::PacketTunnel).unwrap();
    host.enable("is.olibuijr.vpn", Point::DnsProxy).unwrap();

    let tunnel = host.start(Point::PacketTunnel).unwrap();
    assert_eq!(host.start(Point::PacketTunnel), Ok(tunnel), "a live process is reused");
    let resolver = host.start(Point::DnsProxy).unwrap();
    assert_ne!(tunnel, resolver, "each point gets its own process");
}

#[test]
fn a_provider_process_takes_connections_only_once_it_is_running() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.proxy", Point::AppProxy, "Proxy").unwrap()).unwrap();
    host.enable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    let process = host.start(Point::AppProxy).unwrap();

    assert_eq!(host.connect(process), Err(ConfigError::Process(ProcessError::NotRunning)));
    host.mark_running(process).unwrap();
    assert!(host.connect(process).is_ok());
}

#[test]
fn a_process_id_from_another_host_is_unknown_here() {
    let mut host = ProviderHost::new();
    host.register(Configuration::new("is.olibuijr.proxy", Point::AppProxy, "Proxy").unwrap()).unwrap();
    host.enable("is.olibuijr.proxy", Point::AppProxy).unwrap();
    let process = host.start(Point::AppProxy).unwrap();

    let mut other = ProviderHost::new();
    assert_eq!(other.mark_running(process), Err(ConfigError::Process(ProcessError::NoProcess)));
    assert_eq!(other.connect(process), Err(ConfigError::Process(ProcessError::NoProcess)));
}
