//! The GPU executor must draw a display list the way the CPU `Surface` does (within antialiasing differences).
#![cfg(feature = "gpu-tests")]

use bifrost_2d::Renderer;
use bifrost_gpu::{Device, PixelFormat, Region, TextureDescriptor, TextureUsage};
use bifrost_render::canvas::{self, Canvas, ImageRef, PathData, PathPaint, PathVerb, RectF, Stop, TextRun, Transform2D};
use bifrost_render::display_list::DisplayList;
use bifrost_render::gfx::{pack, Image, Surface};

const W: u32 = 240;
const H: u32 = 160;

fn scene() -> DisplayList {
    let mut d = DisplayList::new(W, H);
    d.fill_rect(RectF::new(0.0, 0.0, W as f32, H as f32), pack(240, 240, 245, 255));
    d.shadow(RectF::new(20.0, 20.0, 90.0, 60.0), 10.0, 12.0, (0.0, 4.0), pack(0, 0, 0, 90));
    d.fill_rrect(RectF::new(20.0, 20.0, 90.0, 60.0), 10.0, pack(255, 255, 255, 255));
    d.stroke_rrect(RectF::new(20.0, 20.0, 90.0, 60.0), 10.0, 1.0, pack(0, 0, 0, 40));
    d.linear_gradient(RectF::new(130.0, 20.0, 90.0, 30.0), 6.0, (130.0, 20.0), (220.0, 20.0), &[Stop { at: 0.0, color: pack(0, 122, 255, 255) }, Stop { at: 1.0, color: pack(52, 199, 89, 255) }]);
    let mut img = Image::new(16, 16);
    for (i, p) in img.px.iter_mut().enumerate() {
        *p = if (i / 16 + i % 16) % 2 == 0 { pack(255, 59, 48, 255) } else { pack(255, 204, 0, 255) };
    }
    d.image(ImageRef { id: 42, image: &img }, RectF::new(140.0, 70.0, 32.0, 32.0), 1.0);
    let face = canvas::register_face(include_bytes!("../../assets/SFUI.ttf"));
    d.text(&TextRun { face, text: "BifrOSt".into(), px: 18.0, vars: vec![], x: 30.0, y: 36.0 }, pack(29, 29, 31, 255));
    let tri = PathData { verbs: vec![PathVerb::Move(190.0, 110.0), PathVerb::Line(230.0, 150.0), PathVerb::Line(150.0, 150.0), PathVerb::Close] };
    d.path(&tri, PathPaint::FillNonZero, pack(88, 86, 214, 255));
    d.push_clip(RectF::new(20.0, 100.0, 100.0, 40.0), 12.0);
    d.push_transform(Transform2D::translate(10.0, 0.0));
    d.fill_rect(RectF::new(0.0, 90.0, 200.0, 70.0), pack(255, 149, 0, 255));
    d.pop_transform();
    d.pop_clip();
    d
}

fn channels(px: u32) -> [i32; 4] {
    [(px & 255) as i32, (px >> 8 & 255) as i32, (px >> 16 & 255) as i32, (px >> 24) as i32]
}

#[test]
fn gpu_and_cpu_draw_the_same_scene() {
    let list = scene();
    // CPU reference through the Canvas impl of Surface.
    let mut cpu = vec![0u32; (W * H) as usize];
    {
        let mut s = Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut cpu[..], y0: 0 };
        list.replay(&mut s);
    }
    // GPU.
    let device = Device::open(None).expect("a Vulkan device");
    let mut renderer = Renderer::new(&device).expect("renderer");
    let target = device
        .new_texture(&TextureDescriptor { width: W, height: H, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC, exportable: false, modifiers: vec![] })
        .expect("target");
    let queue = device.new_command_queue();
    let mut cb = queue.command_buffer().expect("command buffer");
    let stats = renderer.draw(&mut cb, &list, &target, None).expect("draw");
    cb.commit().expect("commit").wait().expect("wait");
    let bytes = target.read_region(Region { x: 0, y: 0, width: W, height: H }).expect("readback");
    let gpu: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();

    if let Ok(dir) = std::env::var("BIFROST_DUMP") {
        let raw = |v: &[u32]| v.iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<u8>>();
        std::fs::write(format!("{dir}/gpu.rgba"), raw(&gpu)).unwrap();
        std::fs::write(format!("{dir}/cpu.rgba"), raw(&cpu)).unwrap();
    }
    let mut worst = 0;
    let mut off = 0usize;
    let mut sum = 0i64;
    for (g, c) in gpu.iter().zip(cpu.iter()) {
        let d = channels(*g).iter().zip(channels(*c).iter()).map(|(a, b)| (a - b).abs()).max().unwrap();
        worst = worst.max(d);
        sum += d as i64;
        if d > 24 {
            off += 1;
        }
    }
    let n = gpu.len();
    let mean = sum as f64 / n as f64;
    eprintln!("draws {} instances {} glyphs {} images {}; mean diff {mean:.2}, worst {worst}, pixels off by >24: {off} of {n}", stats.draws, stats.instances, stats.glyphs_uploaded, stats.images_uploaded);
    assert!(stats.glyphs_uploaded > 0 && stats.images_uploaded == 1, "text and the image went through the GPU caches");
    assert!(mean < 2.0, "mean channel difference {mean:.2}");
    assert!(off * 100 < n, "{off} of {n} pixels differ by more than 24 levels");
}

#[test]
fn a_solid_image_samples_its_colour() {
    let device = Device::open(None).expect("a Vulkan device");
    let mut renderer = Renderer::new(&device).expect("renderer");
    let mut img = Image::new(4, 4);
    for p in img.px.iter_mut() {
        *p = pack(255, 0, 0, 255);
    }
    let mut d = DisplayList::new(16, 16);
    d.image(ImageRef { id: 7, image: &img }, RectF::new(0.0, 0.0, 16.0, 16.0), 1.0);
    let target = device
        .new_texture(&TextureDescriptor { width: 16, height: 16, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC, exportable: false, modifiers: vec![] })
        .expect("target");
    let queue = device.new_command_queue();
    let mut cb = queue.command_buffer().expect("command buffer");
    let stats = renderer.draw(&mut cb, &d, &target, None).expect("draw");
    cb.commit().expect("commit").wait().expect("wait");
    let px = target.read_region(Region { x: 8, y: 8, width: 1, height: 1 }).expect("readback");
    eprintln!("stats {stats:?} centre {px:?}");
    assert_eq!(px, vec![255, 0, 0, 255]);
}
