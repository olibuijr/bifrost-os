//! The render server's composition and live glass on a real GPU: a hard black/white edge under a glass panel must
//! come out blurred inside the panel and sharp outside it.
#![cfg(feature = "gpu-tests")]

use bifrost_gpu::{Device, LoadAction, PixelFormat, Region, TextureDescriptor, TextureUsage};
use bifrost_wm::render_server::compose::{self, ComposePipelines, DrawItem};
use bifrost_wm::render_server::material::{Blur, MaterialRegion};

const W: u32 = 128;
const H: u32 = 64;

fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> DrawItem {
    DrawItem { texture: None, quad: [[x, y], [x + w, y], [x, y + h], [x + w, y + h]], src: [0.0; 4], opacity: 1.0, color, clip: [0.0; 5], material: None, shadow: None }
}

#[test]
fn glass_blurs_the_edge_behind_it_and_nothing_else() {
    let device = Device::open(None).expect("a Vulkan device");
    let target = device
        .new_texture(&TextureDescriptor { width: W, height: H, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::RENDER_TARGET | TextureUsage::SAMPLED | TextureUsage::COPY_SRC, exportable: false, modifiers: vec![] })
        .expect("target");
    let pipes = ComposePipelines::new(&device, PixelFormat::Rgba8Unorm).expect("compose pipelines");
    let mut blur = Blur::new(&device).expect("blur");
    let glass_rect = [32.0, 8.0, 64.0, 48.0];
    let mut glass = solid(32.0, 8.0, 64.0, 48.0, [0.0; 4]);
    glass.material = Some(MaterialRegion { rect: glass_rect, radius: 8.0, blur: 16.0, tint: [0.0; 4], saturation: 1.0 });
    let items = vec![solid(0.0, 0.0, 64.0, H as f32, [1.0, 1.0, 1.0, 1.0]), solid(64.0, 0.0, 64.0, H as f32, [0.0, 0.0, 0.0, 1.0]), glass];
    let queue = device.new_command_queue();
    let mut cb = queue.command_buffer().expect("command buffer");
    compose::record(&mut cb, &pipes, &target, LoadAction::Clear([0.0; 4]), &vec![[0, 0, W as i32, H as i32]], &items, &mut blur).expect("record");
    cb.commit().expect("commit").wait().expect("wait");
    let px = target.read_region(Region { x: 0, y: 0, width: W, height: H }).expect("readback");
    let at = |x: u32, y: u32| px[((y * W + x) * 4) as usize] as i32;
    // Outside the glass the edge stays hard.
    assert_eq!((at(62, 2), at(65, 2)), (255, 0), "row 2 is above the glass");
    // Inside the glass the edge is spread out: a smooth ramp, darker to the right.
    let row = 32;
    let ramp: Vec<i32> = (56..72).map(|x| at(x, row)).collect();
    eprintln!("glass row {row} around the edge: {ramp:?}");
    assert!(ramp.windows(2).all(|w| w[0] >= w[1] - 2), "the blurred edge falls monotonically: {ramp:?}");
    assert!(at(63, row) < 245 && at(64, row) > 10, "the edge is no longer a step: {ramp:?}");
    assert!(at(40, row) > 200 && at(88, row) < 55, "far from the edge the glass keeps the backdrop colour");
}
