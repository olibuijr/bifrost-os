//! Parity sdk-complete-06: the graphics device of a guest as plain data. Framebuffer sizes per scanout and the lookup
//! of the device in a configuration. Headless: no framebuffer is allocated and no guest is started.

use bifrost_core::guest_vm::{Device, DirectoryShare, GraphicsDevice, GuestConfig, Scanout, MIB};

fn graphics(sizes: &[(u32, u32)]) -> GraphicsDevice {
    GraphicsDevice { scanouts: sizes.iter().map(|&(width, height)| Scanout { width, height }).collect() }
}

fn config(devices: Vec<Device>) -> GuestConfig {
    GuestConfig { cpu_count: 2, memory_bytes: 1024 * MIB, devices }
}

#[test]
fn a_scanout_framebuffer_is_four_bytes_per_pixel() {
    let device = graphics(&[(1920, 1080)]);
    assert_eq!(device.framebuffer_bytes(0), Some(1920 * 1080 * 4));
}

#[test]
fn a_scanout_index_past_the_last_display_has_no_framebuffer() {
    let device = graphics(&[(1280, 800), (800, 600)]);
    assert_eq!(device.framebuffer_bytes(1), Some(800 * 600 * 4));
    assert_eq!(device.framebuffer_bytes(2), None);
}

#[test]
fn the_total_framebuffer_is_the_sum_of_every_scanout() {
    let device = graphics(&[(1280, 800), (800, 600)]);
    assert_eq!(device.total_framebuffer_bytes(), 1280 * 800 * 4 + 800 * 600 * 4);
}

#[test]
fn the_largest_allowed_device_fits_in_a_64_bit_count() {
    let device = graphics(&[(8192, 8192); 4]);
    assert_eq!(device.total_framebuffer_bytes(), 4 * 8192 * 8192 * 4);
}

#[test]
fn a_device_with_no_scanouts_has_no_framebuffer_and_no_bytes() {
    let device = graphics(&[]);
    assert_eq!(device.framebuffer_bytes(0), None);
    assert_eq!(device.total_framebuffer_bytes(), 0);
}

#[test]
fn the_configuration_returns_its_graphics_device() {
    let cfg = config(vec![Device::Entropy, Device::Graphics(graphics(&[(1024, 768)]))]);
    assert_eq!(cfg.graphics(), Some(&graphics(&[(1024, 768)])));
}

#[test]
fn a_configuration_without_graphics_has_no_graphics_device() {
    let share = Device::DirectoryShare(DirectoryShare { tag: "projects".to_string(), read_only: true });
    assert_eq!(config(vec![Device::Entropy, share]).graphics(), None);
}
