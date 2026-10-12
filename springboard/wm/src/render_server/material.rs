//! Materials drawn by the server: live glass (dual-Kawase blur in compute shaders `kawase_down.comp` and
//! `kawase_up.comp` over the already-composited content under a region, at half resolution, with `passes` from the
//! blur radius; then the blurred region is drawn back clipped to the rounded rect with the saturation boost and the
//! tint), and window drop shadows (SDF Gaussian approximation in `shadow.frag`, the reference platform's large soft
//! active-window shadow and smaller inactive one).
//!
//! Data flow of one blur (`Pipelines::record_blur`), with T_k the pyramid level of size region >> k:
//! down pass k reads T_(k-1) (T_0 is the target region itself, read through the sampler: this is the copy) and writes
//! T_k; up pass k (from n-1 down to 1) reads T_(k+1) and writes T_k; the final up step reads T_1 and writes the target
//! region at full resolution, saturated, tinted and clipped to the rounded rect (SDF coverage) over what the target
//! already holds. Everything is recorded into the caller's command buffer; nothing here waits on the CPU.
//!
//! Shadows (`Pipelines::record_shadow`) are one fullscreen triangle scissored to the shadow's bounds, drawn with
//! premultiplied source-over.
//!
//! Requirements on the targets: a material's target and its pyramid are `Rgba8Unorm` textures with `SAMPLED | STORAGE`
//! usage (the compose target), because the compute shaders write `rgba8` storage images.
//!
//! Owner: wave G1 agent `wm-material`.

use bifrost_gpu::pipeline::Filter;
use bifrost_gpu::{
    BlendMode, CommandBuffer, ComputePipeline, ComputePipelineDescriptor, Device, LoadAction, PixelFormat,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, Region, Result, Sampler, SamplerDescriptor,
    StoreAction, Texture, TextureDescriptor, TextureUsage,
};

/// A blurred region behind an item, in output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialRegion {
    pub rect: [f32; 4],
    pub radius: f32,
    /// Blur radius in output pixels.
    pub blur: f32,
    /// Premultiplied tint drawn over the blur.
    pub tint: [f32; 4],
    pub saturation: f32,
}

/// A drop shadow behind an item, in output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    pub rect: [f32; 4],
    pub radius: f32,
    pub sigma: f32,
    pub offset: [f32; 2],
    pub color: [f32; 4],
}

/// Saturation of the blurred backdrop, as the shell's materials (`wm/src/glass.rs`, `SATURATION`).
pub const SATURATION: f32 = 1.8;

/// Largest number of dual-Kawase passes the server runs.
pub const MAX_PASSES: u32 = 6;

/// Bytes of the push block read by the blur shaders (`PushBlock`, std430 layout).
pub const BLUR_PUSH_BYTES: u32 = 88;
/// Bytes of the push block read by `shadow.frag` (`ShadowPush`, std430 layout).
pub const SHADOW_PUSH_BYTES: u32 = 48;

/// The number of dual-Kawase down/up passes for a blur radius in pixels (pure): 1 pass ~ 2 px, each further pass
/// doubles the reach; clamped to 1..6.
pub fn kawase_passes(radius_px: f32) -> u32 {
    if !(radius_px > 2.0) {
        return 1;
    }
    let mut passes = 1;
    let mut reach = 2.0_f32;
    while reach < radius_px && passes < MAX_PASSES {
        passes += 1;
        reach *= 2.0;
    }
    passes
}

/// The kernel offset in source texels for a blur radius and pass count (pure): the reach of `passes` levels is
/// about `2^passes` output pixels times the offset, so the offset is `radius / 2^passes`, at least half a texel.
pub fn kawase_offset(radius_px: f32, passes: u32) -> f32 {
    let scale = (1u32 << passes.clamp(1, MAX_PASSES)) as f32;
    (radius_px / scale).max(0.5)
}

/// The window shadow for a window rect: active windows get the large shadow, inactive the small one (values in the
/// module tests, measured from the reference look: active sigma 24 px offset 0/16 alpha 0.35; inactive sigma 12 px
/// offset 0/6 alpha 0.20, scaled by the output scale).
pub fn window_shadow(rect: [f32; 4], radius: f32, active: bool, scale: f32) -> ShadowSpec {
    let (sigma, dy, alpha) = if active { (24.0, 16.0, 0.35) } else { (12.0, 6.0, 0.20) };
    ShadowSpec {
        rect,
        radius,
        sigma: sigma * scale,
        offset: [0.0, dy * scale],
        // black, so premultiplied colour is (0, 0, 0, alpha)
        color: [0.0, 0.0, 0.0, alpha],
    }
}

/// The whole-pixel region an `[x, y, w, h]` rectangle covers, rounded outward (pure). Width and height are zero for
/// an empty rectangle.
pub fn device_region(rect: [f32; 4]) -> [i32; 4] {
    let x0 = rect[0].floor();
    let y0 = rect[1].floor();
    let x1 = (rect[0] + rect[2]).ceil();
    let y1 = (rect[1] + rect[3]).ceil();
    let w = (x1 - x0).max(0.0) as i32;
    let h = (y1 - y0).max(0.0) as i32;
    [x0 as i32, y0 as i32, w, h]
}

/// The sizes of the pyramid levels T_1..T_passes for a `w` x `h` region (pure): each level is half the one before,
/// rounded up, at least one texel.
pub fn pyramid_sizes(w: u32, h: u32, passes: u32) -> Vec<(u32, u32)> {
    (1..=passes)
        .map(|k| {
            let div = 1u32 << k;
            (((w + div - 1) / div).max(1), ((h + div - 1) / div).max(1))
        })
        .collect()
}

/// The clipped whole-pixel bounds `[x, y, w, h]` a shadow can touch on a `tw` x `th` target (pure): the shadow rect
/// moved by its offset and grown by three sigma. Empty when the shadow lies outside the target.
pub fn shadow_bounds(spec: &ShadowSpec, tw: i32, th: i32) -> [i32; 4] {
    let pad = 3.0 * spec.sigma.max(0.0);
    let r = spec.rect;
    let x0 = (r[0] + spec.offset[0] - pad).floor().clamp(0.0, tw.max(0) as f32);
    let y0 = (r[1] + spec.offset[1] - pad).floor().clamp(0.0, th.max(0) as f32);
    let x1 = (r[0] + r[2] + spec.offset[0] + pad).ceil().clamp(0.0, tw.max(0) as f32);
    let y1 = (r[1] + r[3] + spec.offset[1] + pad).ceil().clamp(0.0, th.max(0) as f32);
    [x0 as i32, y0 as i32, (x1 - x0).max(0.0) as i32, (y1 - y0).max(0.0) as i32]
}

/// The push block of the blur shaders (pure encoding). Field order and offsets match the GLSL `Push` block in
/// `kawase_down.comp` and `kawase_up.comp` (std430: vec4s first, so every member sits on its natural alignment).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PushBlock {
    /// Clip rectangle in target texels: x0, y0, x1, y1 (up pass only).
    pub rect: [f32; 4],
    /// Premultiplied tint (final pass only).
    pub tint: [f32; 4],
    pub src_size: [f32; 2],
    pub src_origin: [f32; 2],
    pub src_extent: [f32; 2],
    pub dst_origin: [f32; 2],
    pub dst_size: [f32; 2],
    pub offset: f32,
    pub saturation: f32,
    pub radius: f32,
    /// 1 for the final pass that writes the target.
    pub final_pass: u32,
}

impl PushBlock {
    pub fn to_bytes(&self) -> [u8; BLUR_PUSH_BYTES as usize] {
        let mut out = [0u8; BLUR_PUSH_BYTES as usize];
        let mut at = 0;
        let mut put = |bytes: [u8; 4]| {
            out[at..at + 4].copy_from_slice(&bytes);
            at += 4;
        };
        for v in self.rect.iter().chain(self.tint.iter()) {
            put(v.to_le_bytes());
        }
        for v in self.src_size.iter().chain(self.src_origin.iter()).chain(self.src_extent.iter()) {
            put(v.to_le_bytes());
        }
        for v in self.dst_origin.iter().chain(self.dst_size.iter()) {
            put(v.to_le_bytes());
        }
        put(self.offset.to_le_bytes());
        put(self.saturation.to_le_bytes());
        put(self.radius.to_le_bytes());
        put(self.final_pass.to_le_bytes());
        debug_assert_eq!(at, out.len());
        out
    }
}

/// The push block of `shadow.frag` (pure encoding): rect, offset, radius, sigma, colour, as the GLSL `Push` block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowPush {
    pub rect: [f32; 4],
    pub offset: [f32; 2],
    pub radius: f32,
    pub sigma: f32,
    pub color: [f32; 4],
}

impl ShadowPush {
    pub fn to_bytes(&self) -> [u8; SHADOW_PUSH_BYTES as usize] {
        let mut out = [0u8; SHADOW_PUSH_BYTES as usize];
        let mut at = 0;
        let mut put = |bytes: [u8; 4]| {
            out[at..at + 4].copy_from_slice(&bytes);
            at += 4;
        };
        for v in self.rect.iter() {
            put(v.to_le_bytes());
        }
        for v in self.offset.iter() {
            put(v.to_le_bytes());
        }
        put(self.radius.to_le_bytes());
        put(self.sigma.to_le_bytes());
        for v in self.color.iter() {
            put(v.to_le_bytes());
        }
        debug_assert_eq!(at, out.len());
        out
    }
}

/// The GPU pipelines of the materials: the two blur compute passes, the shadow render pipeline and the bilinear
/// sampler the blur reads with. Built once per device.
pub struct Pipelines {
    down: ComputePipeline,
    up: ComputePipeline,
    shadow: RenderPipeline,
    sampler: Sampler,
}

impl Pipelines {
    pub fn new(device: &Device) -> Result<Self> {
        let blur = |shader: &'static str| ComputePipelineDescriptor {
            shader,
            sampled_textures: 1,
            storage_textures: 1,
            samplers: 1,
            push_constant_bytes: BLUR_PUSH_BYTES,
        };
        let down = device.new_compute_pipeline(&blur("kawase_down.comp"))?;
        let up = device.new_compute_pipeline(&blur("kawase_up.comp"))?;
        let shadow = device.new_render_pipeline(&RenderPipelineDescriptor {
            vertex: "fullscreen.vert",
            fragment: "shadow.frag",
            target_format: PixelFormat::Rgba8Unorm,
            blend: BlendMode::PremultipliedOver,
            vertex_layout: None,
            strip: false,
            textures: 0,
            samplers: 0,
            push_constant_bytes: SHADOW_PUSH_BYTES,
        })?;
        let sampler = device.new_sampler(&SamplerDescriptor { filter: Filter::Linear })?;
        Ok(Pipelines { down, up, shadow, sampler })
    }

    /// Records the live glass of `region` into `target`. `pyramid` holds T_1..T_n (see `blur_pyramid`) with
    /// n = `kawase_passes(region.blur)`; a mismatch is an `Invalid` error. Empty regions record nothing.
    pub fn record_blur(
        &self,
        cb: &mut CommandBuffer,
        target: &Texture,
        region: &MaterialRegion,
        pyramid: &[Texture],
    ) -> Result<()> {
        let [x, y, w, h] = device_region(region.rect);
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        let passes = kawase_passes(region.blur);
        if pyramid.len() != passes as usize {
            return Err(bifrost_gpu::Error::Invalid(format!(
                "blur pyramid has {} levels, the radius {} needs {passes}",
                pyramid.len(),
                region.blur
            )));
        }
        let offset = kawase_offset(region.blur, passes);
        let n = pyramid.len();
        let dims = |t: &Texture| (t.width() as f32, t.height() as f32);
        let base = PushBlock {
            rect: [x as f32, y as f32, (x + w) as f32, (y + h) as f32],
            tint: region.tint,
            src_size: [0.0; 2],
            src_origin: [0.0; 2],
            src_extent: [0.0; 2],
            dst_origin: [0.0; 2],
            dst_size: [0.0; 2],
            offset,
            saturation: SATURATION,
            radius: region.radius,
            final_pass: 0,
        };

        // down passes: T_0 (the region of target) -> T_1 -> ... -> T_n
        for k in 1..=n {
            let (src, src_origin, src_extent) = if k == 1 {
                (target, [x as f32, y as f32], [w as f32, h as f32])
            } else {
                let s = &pyramid[k - 2];
                let (sw, sh) = dims(s);
                (s, [0.0, 0.0], [sw, sh])
            };
            let (src_w, src_h) = dims(src);
            let dst = &pyramid[k - 1];
            let (dw, dh) = dims(dst);
            let push = PushBlock {
                src_size: [src_w, src_h],
                src_origin,
                src_extent,
                dst_size: [dw, dh],
                ..base
            };
            self.dispatch(cb, &self.down, src, dst, &push, dw as u32, dh as u32)?;
        }
        // up passes: T_n -> T_(n-1) ... -> T_1
        for k in (1..n).rev() {
            let src = &pyramid[k];
            let dst = &pyramid[k - 1];
            let (sw, sh) = dims(src);
            let (dw, dh) = dims(dst);
            let push = PushBlock {
                src_size: [sw, sh],
                src_origin: [0.0, 0.0],
                src_extent: [sw, sh],
                dst_size: [dw, dh],
                ..base
            };
            self.dispatch(cb, &self.up, src, dst, &push, dw as u32, dh as u32)?;
        }
        // final: T_1 -> target region, saturated, tinted, clipped to the rounded rect
        let first = &pyramid[0];
        let (sw, sh) = dims(first);
        let push = PushBlock {
            src_size: [sw, sh],
            src_origin: [0.0, 0.0],
            src_extent: [sw, sh],
            dst_origin: [x as f32, y as f32],
            dst_size: [w as f32, h as f32],
            final_pass: 1,
            ..base
        };
        self.dispatch(cb, &self.up, first, target, &push, w as u32, h as u32)
    }

    /// Records one compute dispatch covering `groups` 8x8 tiles of `dst_w` x `dst_h` texels.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        cb: &mut CommandBuffer,
        pipeline: &ComputePipeline,
        src: &Texture,
        dst: &Texture,
        push: &PushBlock,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        let mut enc = cb.compute_encoder()?;
        enc.set_pipeline(pipeline);
        enc.set_texture(0, src, false);
        enc.set_sampler(1, &self.sampler);
        enc.set_texture(2, dst, true);
        enc.set_bytes(&push.to_bytes());
        enc.dispatch(dst_w.div_ceil(8), dst_h.div_ceil(8), 1);
        enc.end();
        Ok(())
    }

    /// Records the drop shadow `spec` over the current contents of `target` (premultiplied source-over). Nothing is
    /// recorded when the shadow lies outside the target.
    pub fn record_shadow(&self, cb: &mut CommandBuffer, target: &Texture, spec: &ShadowSpec) -> Result<()> {
        let [x, y, w, h] = shadow_bounds(spec, target.width() as i32, target.height() as i32);
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        let area = Region { x: x as u32, y: y as u32, width: w as u32, height: h as u32 };
        let pass = RenderPassDescriptor {
            target: target.clone(),
            load: LoadAction::Load,
            store: StoreAction::Store,
            render_area: Some(area),
        };
        let mut enc = cb.render_encoder(&pass)?;
        enc.set_pipeline(&self.shadow);
        enc.set_scissor(area);
        let push = ShadowPush {
            rect: spec.rect,
            offset: spec.offset,
            radius: spec.radius,
            sigma: spec.sigma,
            color: spec.color,
        };
        enc.set_bytes(&push.to_bytes());
        enc.draw(3, 1, 0, 0);
        enc.end();
        Ok(())
    }
}

/// Allocates the blur pyramid T_1..T_passes for a `w` x `h` region: `Rgba8Unorm`, sampled and storage.
pub fn blur_pyramid(device: &Device, w: u32, h: u32, passes: u32) -> Result<Vec<Texture>> {
    pyramid_sizes(w, h, passes)
        .into_iter()
        .map(|(width, height)| {
            device.new_texture(&TextureDescriptor {
                width,
                height,
                format: PixelFormat::Rgba8Unorm,
                usage: TextureUsage::SAMPLED | TextureUsage::STORAGE,
                exportable: false,
                modifiers: Vec::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kawase_passes_grows_by_doubling_the_reach() {
        // 1 pass reaches 2 px, 2 passes 4 px, 3 passes 8 px, and so on up to 6 passes (64 px)
        assert_eq!(kawase_passes(0.0), 1);
        assert_eq!(kawase_passes(2.0), 1);
        assert_eq!(kawase_passes(2.5), 2);
        assert_eq!(kawase_passes(4.0), 2);
        assert_eq!(kawase_passes(5.0), 3);
        assert_eq!(kawase_passes(8.0), 3);
        assert_eq!(kawase_passes(9.0), 4);
        assert_eq!(kawase_passes(16.0), 4);
        assert_eq!(kawase_passes(17.0), 5);
        assert_eq!(kawase_passes(32.0), 5);
        assert_eq!(kawase_passes(33.0), 6);
    }

    #[test]
    fn kawase_passes_is_clamped_and_rejects_bad_radii() {
        assert_eq!(kawase_passes(64.0), 6);
        assert_eq!(kawase_passes(1000.0), 6);
        assert_eq!(kawase_passes(-5.0), 1);
        assert_eq!(kawase_passes(f32::NAN), 1);
        assert_eq!(kawase_passes(f32::INFINITY), 6);
    }

    #[test]
    fn kawase_offset_spreads_the_radius_over_the_levels() {
        assert_eq!(kawase_offset(16.0, 4), 1.0);
        assert_eq!(kawase_offset(40.0, 6), 0.625);
        // small radii keep at least half a texel so the taps still blend
        assert_eq!(kawase_offset(0.0, 1), 0.5);
        assert_eq!(kawase_offset(3.0, 1), 1.5);
    }

    #[test]
    fn window_shadow_active_and_inactive_values() {
        let rect = [10.0, 20.0, 300.0, 200.0];
        let a = window_shadow(rect, 12.0, true, 1.0);
        assert_eq!(a.rect, rect);
        assert_eq!(a.radius, 12.0);
        assert_eq!(a.sigma, 24.0);
        assert_eq!(a.offset, [0.0, 16.0]);
        assert_eq!(a.color, [0.0, 0.0, 0.0, 0.35]);

        let i = window_shadow(rect, 12.0, false, 1.0);
        assert_eq!(i.sigma, 12.0);
        assert_eq!(i.offset, [0.0, 6.0]);
        assert_eq!(i.color, [0.0, 0.0, 0.0, 0.20]);
    }

    #[test]
    fn window_shadow_scales_sigma_and_offset_only() {
        let rect = [0.0, 0.0, 100.0, 100.0];
        let a = window_shadow(rect, 9.0, true, 2.0);
        assert_eq!(a.sigma, 48.0);
        assert_eq!(a.offset, [0.0, 32.0]);
        assert_eq!(a.color[3], 0.35);
        assert_eq!(a.radius, 9.0);
        let i = window_shadow(rect, 9.0, false, 1.5);
        assert_eq!(i.sigma, 18.0);
        assert_eq!(i.offset, [0.0, 9.0]);
    }

    #[test]
    fn device_region_rounds_outward() {
        assert_eq!(device_region([10.5, 20.25, 100.0, 50.0]), [10, 20, 101, 51]);
        assert_eq!(device_region([0.0, 0.0, 0.0, 10.0]), [0, 0, 0, 10]);
        assert_eq!(device_region([-3.5, 4.0, 8.0, 2.0]), [-4, 4, 9, 2]);
    }

    #[test]
    fn pyramid_halves_and_rounds_up() {
        assert_eq!(pyramid_sizes(100, 50, 3), vec![(50, 25), (25, 13), (13, 7)]);
        assert_eq!(pyramid_sizes(1, 1, 2), vec![(1, 1), (1, 1)]);
        assert_eq!(pyramid_sizes(640, 480, 0), Vec::<(u32, u32)>::new());
    }

    #[test]
    fn shadow_bounds_cover_three_sigma_and_clip_to_target() {
        let spec = window_shadow([100.0, 100.0, 200.0, 100.0], 0.0, false, 1.0);
        // sigma 12, pad 36, offset y 6: x 64..336, y 70..242
        assert_eq!(shadow_bounds(&spec, 1000, 1000), [64, 70, 272, 172]);
        // clipped at the target edge
        assert_eq!(shadow_bounds(&spec, 200, 150), [64, 70, 136, 80]);
        // entirely outside
        let far = window_shadow([-500.0, -500.0, 10.0, 10.0], 0.0, false, 1.0);
        assert_eq!(shadow_bounds(&far, 100, 100), [0, 0, 0, 0]);
    }

    #[test]
    fn push_block_bytes_follow_the_shader_layout() {
        let p = PushBlock {
            rect: [1.0, 2.0, 3.0, 4.0],
            tint: [0.5, 0.25, 0.0, 0.125],
            src_size: [10.0, 11.0],
            src_origin: [12.0, 13.0],
            src_extent: [14.0, 15.0],
            dst_origin: [16.0, 17.0],
            dst_size: [18.0, 19.0],
            offset: 0.75,
            saturation: 1.8,
            radius: 9.0,
            final_pass: 1,
        };
        let b = p.to_bytes();
        assert_eq!(b.len(), 88);
        let f = |at: usize| f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        assert_eq!(f(0), 1.0); // rect.x at 0
        assert_eq!(f(12), 4.0); // rect.w at 12
        assert_eq!(f(16), 0.5); // tint at 16
        assert_eq!(f(32), 10.0); // src_size at 32
        assert_eq!(f(56), 16.0); // dst_origin at 56
        assert_eq!(f(64), 18.0); // dst_size at 64
        assert_eq!(f(72), 0.75); // offset at 72
        assert_eq!(f(80), 9.0); // radius at 80
        assert_eq!(u32::from_le_bytes([b[84], b[85], b[86], b[87]]), 1); // final_pass at 84
    }

    #[test]
    fn shadow_push_bytes_follow_the_shader_layout() {
        let p = ShadowPush {
            rect: [5.0, 6.0, 7.0, 8.0],
            offset: [0.0, 16.0],
            radius: 12.0,
            sigma: 24.0,
            color: [0.0, 0.0, 0.0, 0.35],
        };
        let b = p.to_bytes();
        assert_eq!(b.len(), 48);
        let f = |at: usize| f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        assert_eq!(f(0), 5.0);
        assert_eq!(f(12), 8.0);
        assert_eq!(f(20), 16.0); // offset.y
        assert_eq!(f(24), 12.0); // radius
        assert_eq!(f(28), 24.0); // sigma
        assert_eq!(f(44), 0.35); // color.a at 32 + 12
    }
}
