//! Graphics device configuration for a guest: the scanouts a guest's displays come from, and the checks that refuse
//! a graphics device with no display, too many displays, a display with a bad size, or a second graphics device.
//!
//! Headless: the configuration is checked as plain data. Nothing is started, and no hypervisor, display or device is
//! touched.

use bifrost_core::guest_vm::{ConfigError, Device, GraphicsDevice, GuestConfig, Scanout, MAX_SCANOUTS, MAX_SCANOUT_PIXELS, MIB};

const HOST_CPUS: u32 = 4;

fn config(devices: Vec<Device>) -> GuestConfig {
    GuestConfig { cpu_count: 2, memory_bytes: 1024 * MIB, devices }
}

fn graphics(sizes: &[(u32, u32)]) -> Device {
    Device::Graphics(GraphicsDevice { scanouts: sizes.iter().map(|&(width, height)| Scanout { width, height }).collect() })
}

#[test]
fn a_graphics_device_with_one_display_is_valid() {
    assert_eq!(config(vec![graphics(&[(1920, 1080)])]).validate(HOST_CPUS), Ok(()));
}

#[test]
fn a_graphics_device_may_drive_up_to_the_scanout_limit() {
    assert_eq!(MAX_SCANOUTS, 4);
    assert_eq!(config(vec![graphics(&[(1280, 800); 4])]).validate(HOST_CPUS), Ok(()));
    assert_eq!(config(vec![graphics(&[(1280, 800); 5])]).validate(HOST_CPUS), Err(ConfigError::TooManyScanouts));
}

#[test]
fn a_graphics_device_needs_at_least_one_display() {
    assert_eq!(config(vec![graphics(&[])]).validate(HOST_CPUS), Err(ConfigError::NoScanout));
}

#[test]
fn a_display_side_must_be_between_one_and_the_pixel_limit() {
    assert_eq!(MAX_SCANOUT_PIXELS, 8192);
    assert_eq!(config(vec![graphics(&[(0, 600)])]).validate(HOST_CPUS), Err(ConfigError::BadScanoutSize), "zero width");
    assert_eq!(config(vec![graphics(&[(800, 0)])]).validate(HOST_CPUS), Err(ConfigError::BadScanoutSize), "zero height");
    assert_eq!(
        config(vec![graphics(&[(MAX_SCANOUT_PIXELS + 1, 600)])]).validate(HOST_CPUS),
        Err(ConfigError::BadScanoutSize),
        "one pixel above the limit"
    );
    assert_eq!(config(vec![graphics(&[(MAX_SCANOUT_PIXELS, 1)])]).validate(HOST_CPUS), Ok(()), "at the limit");
}

#[test]
fn a_second_graphics_device_is_a_duplicate() {
    assert_eq!(
        config(vec![graphics(&[(800, 600)]), graphics(&[(640, 480)])]).validate(HOST_CPUS),
        Err(ConfigError::DuplicateDevice("graphics"))
    );
}

#[test]
fn graphics_sits_beside_the_other_devices_without_changing_their_rules() {
    let devices = vec![Device::Entropy, Device::Socket, graphics(&[(1024, 768)])];
    assert_eq!(config(devices).validate(HOST_CPUS), Ok(()));
    let doubled = vec![Device::Entropy, Device::Entropy, graphics(&[(1024, 768)])];
    assert_eq!(config(doubled).validate(HOST_CPUS), Err(ConfigError::DuplicateDevice("entropy")));
}

#[test]
fn the_cpu_check_still_comes_first_for_a_configuration_with_graphics() {
    let no_cpu = GuestConfig { cpu_count: 0, memory_bytes: 1024 * MIB, devices: vec![graphics(&[])] };
    assert_eq!(no_cpu.validate(HOST_CPUS), Err(ConfigError::BadCpuCount));
}
