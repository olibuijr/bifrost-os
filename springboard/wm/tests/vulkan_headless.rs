//! The Vulkan headless path (`headless::VulkanHeadless`) on a real device: scene nodes land where their rectangles say,
//! damage limits what is drawn, and a resize reallocates the target. These run without a Wayland client; the full
//! window-stack harness (shm toplevel, layer-shell, animated layer) is in `tests/vulkan_wayland.rs`, still to come.
#![cfg(feature = "gpu-tests")]

use bifrost_wm::headless::{bgra_to_rgba, full_damage, rgba_at, VulkanHeadless};
use bifrost_wm::render_server::compose::SceneNode;

const W: u32 = 64;
const H: u32 = 48;
/// The clear colour 0.06, 0.06, 0.08 in 8 bits is about 15, 15, 20; the readback may round it by one.
const CLEAR: [u8; 3] = [15, 15, 20];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= 1)
}

fn rgba_frame(vk: &mut VulkanHeadless, nodes: &[SceneNode], damage: &Vec<[i32; 4]>) -> Vec<u8> {
    bgra_to_rgba(&vk.render(nodes, damage).expect("render"))
}

#[test]
fn nodes_land_at_their_rectangles() {
    let mut vk = VulkanHeadless::new(W, H).expect("a Vulkan device");
    let nodes = vec![
        SceneNode::solid([0.0, 0.0, W as f32, H as f32], [0.0, 0.0, 1.0, 1.0]),
        SceneNode::solid([8.0, 8.0, 16.0, 16.0], [1.0, 0.0, 0.0, 1.0]),
    ];
    let img = rgba_frame(&mut vk, &nodes, &full_damage(W, H));
    assert!(near(rgba_at(&img, W, H, 1, 1).unwrap(), BLUE), "the background node covers the corner");
    assert!(near(rgba_at(&img, W, H, 16, 16).unwrap(), RED), "the red node covers its centre");
    assert!(near(rgba_at(&img, W, H, 24, 24).unwrap(), BLUE), "one pixel past the red node's far corner is background");
    assert!(near(rgba_at(&img, W, H, 7, 7).unwrap(), BLUE), "one pixel before its near corner is background");
}

#[test]
fn damage_limits_what_is_drawn() {
    let mut vk = VulkanHeadless::new(W, H).expect("a Vulkan device");
    let nodes = vec![SceneNode::solid([0.0, 0.0, W as f32, H as f32], [0.0, 0.0, 1.0, 1.0])];
    // the damage covers only the top-left 8x8: the blue node is drawn there, and the rest is cleared
    let img = rgba_frame(&mut vk, &nodes, &vec![[0, 0, 8, 8]]);
    assert!(near(rgba_at(&img, W, H, 2, 2).unwrap(), BLUE));
    let outside = rgba_at(&img, W, H, 40, 30).unwrap();
    assert!(outside[0] <= 16 && outside[1] <= 16 && outside[2] <= 21, "outside the damage nothing is drawn: {outside:?}");
}

#[test]
fn an_empty_scene_is_the_clear_colour() {
    let mut vk = VulkanHeadless::new(W, H).expect("a Vulkan device");
    let img = rgba_frame(&mut vk, &[], &full_damage(W, H));
    let px = rgba_at(&img, W, H, W - 1, H - 1).unwrap();
    assert!(px[0] <= 16 && px[1] <= 16 && px[2] <= 21 && px[3] == 255, "clear colour, got {px:?}");
    assert!(CLEAR.iter().all(|c| *c <= 21));
}

#[test]
fn resize_reallocates_the_target() {
    let mut vk = VulkanHeadless::new(W, H).expect("a Vulkan device");
    vk.resize(32, 20).expect("resize");
    assert_eq!(vk.size(), (32, 20));
    let bytes = vk.render(&[], &full_damage(32, 20)).expect("render");
    assert_eq!(bytes.len(), 32 * 20 * 4, "the readback has the new size");
}

#[test]
fn a_frame_without_damage_has_no_pixels_drawn() {
    let mut vk = VulkanHeadless::new(W, H).expect("a Vulkan device");
    let nodes = vec![SceneNode::solid([0.0, 0.0, W as f32, H as f32], [1.0, 0.0, 0.0, 1.0])];
    let img = rgba_frame(&mut vk, &nodes, &Vec::new());
    let px = rgba_at(&img, W, H, 0, 0).unwrap();
    assert!(px[0] <= 16 && px[1] <= 16, "empty damage draws nothing over the clear colour, got {px:?}");
}
