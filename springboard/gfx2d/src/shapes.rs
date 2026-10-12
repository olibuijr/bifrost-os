//! SDF shapes: one instanced-quad pipeline (`gpu/shaders/shape.vert`, `shape.frag`) draws rects, rounded rects, strokes,
//! shadows, linear gradients, images and masks. Each instance carries its rect, radius, kind, colours, the transform
//! and the active rounded clip; the fragment shader evaluates the rounded-rect distance for coverage (1 px
//! antialiasing), the clip distance in device pixels, and the Gaussian shadow (0.5 * erfc of the distance).
//!
//! Instance layout (128 bytes, eight vec4 attributes at locations 0..7, `words()` gives the order):
//!
//! | slot | rect | params | color | color2 | xform | xform_t | clip | extra |
//! |---|---|---|---|---|---|---|---|---|
//! | x, y, z, w | x, y, w, h (local) | radius, width / shadow sigma / gradient t0, kind, gradient t1 | straight RGBA; gradient colour at t0; mask tint; image alpha | gradient colour at t1; image and mask uv (u0, v0, u1, v1) | a, b, c, d | tx, ty; gradient start x, y | device clip x, y, w, h (w = 0: none) | gradient end x, y; clip corner radius; gradient flags |
//!
//! Gradient flags: bit 0 (value 1) pads before t0, bit 1 (value 2) pads after t1. A gradient with more than two stops
//! is several instances, one per pair of neighbouring stops, each covering t in [t0, t1] (the pads are on the first and
//! last instance only), so premultiplied blending never double-composites a region.
//!
//! The instance's rect and gradient points are in local pixels (the coordinates the caller draws in, before the
//! current transform); `xform` maps them to device pixels; the clip is in device pixels.
//!
//! Owner: wave G1 agent `gfx2d-shapes`.

use bifrost_gpu::pipeline::{BlendMode, Filter, RenderPipelineDescriptor, SamplerDescriptor, VertexLayout};
use bifrost_gpu::{Buffer, Device, PixelFormat, RenderEncoder, RenderPipeline, Result, Sampler, Texture};
use bifrost_render::canvas::{RectF, Stop, Transform2D};
use bifrost_render::display_list::Op;

/// Shape kinds (the shader's `kind` switch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Fill = 0,
    Stroke = 1,
    Shadow = 2,
    LinearGradient = 3,
    /// Samples a texture (images and glyphs reuse the instance layout with `kind` image or mask).
    Image = 4,
    Mask = 5,
}

/// Bytes per instance in the vertex buffer.
pub const INSTANCE_BYTES: usize = 128;
/// Push constant bytes: the render target size as two f32 (`target_size`).
pub const PUSH_BYTES: u32 = 8;

/// One instance, laid out for the vertex buffer (eight vec4, std430-compatible, 128 bytes). Field meanings are in
/// the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Instance {
    /// x, y, w, h of the shape in local pixels.
    pub rect: [f32; 4],
    /// corner radius, stroke width (or shadow sigma, or gradient t0), kind, gradient t1
    pub params: [f32; 4],
    /// straight RGBA colour 0..1 (premultiplied in the shader); gradient colour at t0; image alpha in `.a`
    pub color: [f32; 4],
    /// gradient colour at t1; image and mask uv rect (u0, v0, u1, v1)
    pub color2: [f32; 4],
    /// transform a, b, c, d
    pub xform: [f32; 4],
    /// transform tx, ty; gradient start x, y (local)
    pub xform_t: [f32; 4],
    /// clip x, y, w, h in device pixels (w = 0: no clip)
    pub clip: [f32; 4],
    /// gradient end x, y (local); clip corner radius (device pixels); gradient flags (bit 0 pad before, bit 1 pad after)
    pub extra: [f32; 4],
}

impl Instance {
    /// The 32 floats in attribute order (rect, params, color, color2, xform, xform_t, clip, extra).
    pub fn words(&self) -> [f32; 32] {
        let mut w = [0.0f32; 32];
        let parts = [self.rect, self.params, self.color, self.color2, self.xform, self.xform_t, self.clip, self.extra];
        for (i, p) in parts.iter().enumerate() {
            w[i * 4..i * 4 + 4].copy_from_slice(p);
        }
        w
    }
}

/// A rounded device-pixel clip, as `push_clip` gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub rect: RectF,
    pub radius: f32,
}

/// The pipelines for shapes.
pub struct Shapes {
    pub(crate) pipeline: RenderPipeline,
    pub(crate) sampler: Sampler,
}

impl Shapes {
    pub fn new(device: &Device, target: PixelFormat) -> Result<Shapes> {
        let pipeline = device.new_render_pipeline(&RenderPipelineDescriptor {
            vertex: "shape.vert",
            fragment: "shape.frag",
            target_format: target,
            blend: BlendMode::PremultipliedOver,
            vertex_layout: Some(vertex_layout()),
            strip: true,
            textures: 1,
            samplers: 1,
            push_constant_bytes: PUSH_BYTES,
        })?;
        let sampler = device.new_sampler(&SamplerDescriptor { filter: Filter::Linear })?;
        Ok(Shapes { pipeline, sampler })
    }

    /// The sampler bound at binding 1 for image and mask instances.
    pub fn sampler(&self) -> &Sampler {
        &self.sampler
    }

    /// Records one draw of `count` instances starting at instance `first` of `instances`, into a target of
    /// `target` device pixels. `texture` is bound at binding 0 (any texture when no instance samples one).
    pub fn draw(&self, enc: &mut RenderEncoder<'_>, instances: &Buffer, first: u32, count: u32, target: (u32, u32), texture: &Texture) {
        if count == 0 {
            return;
        }
        enc.set_pipeline(&self.pipeline);
        enc.set_viewport(0.0, 0.0, target.0 as f32, target.1 as f32);
        enc.set_bytes(&push_constants(target.0, target.1));
        enc.set_vertex_buffer(0, instances, first as usize * INSTANCE_BYTES);
        enc.set_texture(0, texture);
        enc.set_sampler(1, &self.sampler);
        enc.draw(4, count, 0, 0);
    }
}

/// The per-instance vertex layout: eight `R32G32B32A32_SFLOAT` attributes at locations 0..7.
pub fn vertex_layout() -> VertexLayout {
    use bifrost_gpu::ash::vk::Format;
    VertexLayout {
        stride: INSTANCE_BYTES as u32,
        attributes: (0..8u32).map(|i| (i, Format::R32G32B32A32_SFLOAT, i * 16)).collect(),
        per_instance: true,
    }
}

/// The push constant bytes for a target of `width` by `height` device pixels.
pub fn push_constants(width: u32, height: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&(width as f32).to_le_bytes());
    out[4..].copy_from_slice(&(height as f32).to_le_bytes());
    out
}

/// Appends the little-endian bytes of `instances` to `out` (the vertex buffer contents).
pub fn encode(instances: &[Instance], out: &mut Vec<u8>) {
    out.reserve(instances.len() * INSTANCE_BYTES);
    for inst in instances {
        for f in inst.words() {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
}

/// A packed premultiplied `u32` colour (`gfx::pack`, r in the low byte) as straight RGBA 0..1.
pub fn unpack(color: u32) -> [f32; 4] {
    let r = (color & 0xff) as f32;
    let g = ((color >> 8) & 0xff) as f32;
    let b = ((color >> 16) & 0xff) as f32;
    let a8 = (color >> 24) & 0xff;
    if a8 == 0 {
        return [0.0; 4];
    }
    let a = a8 as f32;
    let straight = |c: f32| (c / a).clamp(0.0, 1.0);
    [straight(r), straight(g), straight(b), a / 255.0]
}

/// The extra quad margin, in local pixels, that the vertex shader adds around the rect: one device pixel of
/// antialiasing fringe, plus three sigma for shadows. `xform` is the instance transform (a, b, c, d).
pub fn margin(kind: Kind, sigma: f32, xform: [f32; 4]) -> f32 {
    let scale = (xform[0] * xform[0] + xform[1] * xform[1])
        .sqrt()
        .min((xform[2] * xform[2] + xform[3] * xform[3]).sqrt());
    let aa = 1.0 / scale.max(1e-3);
    if kind == Kind::Shadow {
        aa + 3.0 * sigma.max(0.0)
    } else {
        aa
    }
}

/// The quad the vertex shader draws for `inst`, in local pixels: its rect grown by `margin`.
pub fn quad(inst: &Instance) -> RectF {
    let kind = kind_of(inst);
    let m = margin(kind, inst.params[1], inst.xform);
    RectF::new(inst.rect[0] - m, inst.rect[1] - m, inst.rect[2] + 2.0 * m, inst.rect[3] + 2.0 * m)
}

/// The kind an instance encodes.
pub fn kind_of(inst: &Instance) -> Kind {
    match inst.params[2].round() as u32 {
        1 => Kind::Stroke,
        2 => Kind::Shadow,
        3 => Kind::LinearGradient,
        4 => Kind::Image,
        5 => Kind::Mask,
        _ => Kind::Fill,
    }
}

fn kind_f(kind: Kind) -> f32 {
    kind as u32 as f32
}

fn rect_words(r: RectF) -> [f32; 4] {
    [r.x, r.y, r.w, r.h]
}

fn xform_words(t: Transform2D) -> [f32; 4] {
    [t.a, t.b, t.c, t.d]
}

/// The clip words and the clip radius for an optional clip. An empty clip draws nothing (a tiny box far off screen).
fn clip_words(clip: Option<Clip>) -> ([f32; 4], f32) {
    match clip {
        None => ([0.0; 4], 0.0),
        Some(c) if c.rect.w > 0.0 && c.rect.h > 0.0 => ([c.rect.x, c.rect.y, c.rect.w, c.rect.h], c.radius.max(0.0)),
        Some(_) => ([-1.0e6, -1.0e6, 1.0, 1.0], 0.0),
    }
}

/// An instance with its transform and clip set and every other field zero.
fn placed(xf: Transform2D, clip: Option<Clip>) -> Instance {
    let (clip_xywh, clip_radius) = clip_words(clip);
    Instance {
        xform: xform_words(xf),
        xform_t: [xf.tx, xf.ty, 0.0, 0.0],
        clip: clip_xywh,
        extra: [0.0, 0.0, clip_radius, 0.0],
        ..Instance::default()
    }
}

/// A filled rounded rectangle.
pub fn fill(r: RectF, radius: f32, color: u32, xf: Transform2D, clip: Option<Clip>) -> Instance {
    let mut i = placed(xf, clip);
    i.rect = rect_words(r);
    i.params = [radius.max(0.0), 0.0, kind_f(Kind::Fill), 0.0];
    i.color = unpack(color);
    i
}

/// A rounded-rectangle outline of `width` drawn inside `r`.
pub fn stroke(r: RectF, radius: f32, width: f32, color: u32, xf: Transform2D, clip: Option<Clip>) -> Instance {
    let mut i = placed(xf, clip);
    i.rect = rect_words(r);
    i.params = [radius.max(0.0), width.max(0.0), kind_f(Kind::Stroke), 0.0];
    i.color = unpack(color);
    i
}

/// A soft shadow of a rounded rectangle: Gaussian with standard deviation `blur / 2`, offset by `offset`.
pub fn shadow(r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32, xf: Transform2D, clip: Option<Clip>) -> Instance {
    let mut i = placed(xf, clip);
    i.rect = rect_words(RectF::new(r.x + offset.0, r.y + offset.1, r.w, r.h));
    i.params = [radius.max(0.0), (blur * 0.5).max(0.0), kind_f(Kind::Shadow), 0.0];
    i.color = unpack(color);
    i
}

/// A textured rectangle: `uv` is the source rect (u0, v0, u1, v1) in normalised texture coordinates, `alpha` the
/// opacity. The texture is bound by the caller at binding 0.
pub fn image(dst: RectF, radius: f32, uv: [f32; 4], alpha: f32, xf: Transform2D, clip: Option<Clip>) -> Instance {
    let mut i = placed(xf, clip);
    i.rect = rect_words(dst);
    i.params = [radius.max(0.0), 0.0, kind_f(Kind::Image), 0.0];
    i.color = [1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0)];
    i.color2 = uv;
    i
}

/// A texture used as a coverage mask (red channel) and filled with `color`.
pub fn mask(dst: RectF, radius: f32, uv: [f32; 4], color: u32, xf: Transform2D, clip: Option<Clip>) -> Instance {
    let mut i = placed(xf, clip);
    i.rect = rect_words(dst);
    i.params = [radius.max(0.0), 0.0, kind_f(Kind::Mask), 0.0];
    i.color = unpack(color);
    i.color2 = uv;
    i
}

/// A linear gradient over the rounded rectangle `r`, from `start` (stop position 0) to `end` (stop position 1), in
/// the coordinates of `r`. Stops are sorted by position; each neighbouring pair with a positive width becomes one
/// instance. Colours before the first stop and after the last one are the end colours (the pads). With no stops
/// nothing is drawn; with one stop, or with stops that all sit at one position, the shape is filled with a stop's
/// colour.
pub fn gradient(r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: &[Stop], xf: Transform2D, clip: Option<Clip>) -> Vec<Instance> {
    let mut sorted = stops.to_vec();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    match sorted.len() {
        0 => return Vec::new(),
        1 => return vec![fill(r, radius, sorted[0].color, xf, clip)],
        _ => {}
    }
    let pairs: Vec<(Stop, Stop)> = sorted
        .windows(2)
        .filter(|w| w[1].at > w[0].at)
        .map(|w| (w[0], w[1]))
        .collect();
    if pairs.is_empty() {
        return vec![fill(r, radius, sorted[sorted.len() - 1].color, xf, clip)];
    }
    let (clip_xywh, clip_radius) = clip_words(clip);
    let last = pairs.len() - 1;
    pairs
        .iter()
        .enumerate()
        .map(|(n, (a, b))| {
            let mut flags = 0.0;
            if n == 0 {
                flags += 1.0;
            }
            if n == last {
                flags += 2.0;
            }
            Instance {
                rect: rect_words(r),
                params: [radius.max(0.0), a.at, kind_f(Kind::LinearGradient), b.at],
                color: unpack(a.color),
                color2: unpack(b.color),
                xform: xform_words(xf),
                xform_t: [xf.tx, xf.ty, start.0, start.1],
                clip: clip_xywh,
                extra: [end.0, end.1, clip_radius, flags],
            }
        })
        .collect()
}

/// Multiplies the instance's opacity by `opacity` (the alpha of the colour; for a gradient also the end colour).
fn fade(inst: &mut Instance, opacity: f32) {
    inst.color[3] *= opacity;
    if kind_of(inst) == Kind::LinearGradient {
        inst.color2[3] *= opacity;
    }
}

/// Places an instance built in its own coordinates (glyph quads) into a transform, clip and opacity, the way the
/// op helpers place theirs.
pub fn restate(inst: &mut Instance, xf: Transform2D, clip: Option<Clip>, opacity: f32) {
    let p = placed(xf, clip);
    inst.xform = p.xform;
    inst.xform_t[0] = p.xform_t[0];
    inst.xform_t[1] = p.xform_t[1];
    inst.clip = p.clip;
    inst.extra[2] = p.extra[2];
    if opacity < 1.0 {
        fade(inst, opacity.max(0.0));
    }
}

/// The instances that draw one recorded op in the transform, clip and opacity state current when it was recorded.
/// Images and masks return instances whose texture (`Op::Image::id`, `Op::ImageTinted::id`) the caller binds. Text,
/// paths, materials and the state ops return nothing (they are the glyph, tessellation and material modules' ops).
pub fn op_instances(op: &Op, xf: Transform2D, clip: Option<Clip>, opacity: f32) -> Vec<Instance> {
    let mut out = match op {
        Op::FillRect { r, color } => vec![fill(*r, 0.0, *color, xf, clip)],
        Op::FillRRect { r, radius, color } => vec![fill(*r, *radius, *color, xf, clip)],
        Op::StrokeRRect { r, radius, width, color } => vec![stroke(*r, *radius, *width, *color, xf, clip)],
        Op::LinearGradient { r, radius, start, end, stops } => gradient(*r, *radius, *start, *end, stops, xf, clip),
        Op::Shadow { r, radius, blur, offset, color } => vec![shadow(*r, *radius, *blur, *offset, *color, xf, clip)],
        Op::Image { dst, alpha, .. } => vec![image(*dst, 0.0, [0.0, 0.0, 1.0, 1.0], *alpha, xf, clip)],
        Op::ImageTinted { dst, color, .. } => vec![mask(*dst, 0.0, [0.0, 0.0, 1.0, 1.0], *color, xf, clip)],
        _ => Vec::new(),
    };
    if opacity < 1.0 {
        for inst in &mut out {
            fade(inst, opacity.max(0.0));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::gfx::{pack, rgba};

    const ID: Transform2D = Transform2D::IDENTITY;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    /// The shader's erf (Abramowitz and Stegun 7.1.26), mirrored for the host tests.
    fn erf_ref(x: f32) -> f32 {
        let s = if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 };
        let ax = x.abs();
        let t = 1.0 / (1.0 + 0.3275911 * ax);
        let poly = (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t;
        s * (1.0 - poly * (-ax * ax).exp())
    }

    /// The shader's shadow coverage at signed distance `d` for sigma.
    fn shadow_cov(d: f32, sigma: f32) -> f32 {
        0.5 * (1.0 - erf_ref(d / (sigma * std::f32::consts::SQRT_2)))
    }

    #[test]
    fn unpack_gives_straight_colour() {
        assert_eq!(unpack(pack(255, 0, 0, 255)), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(unpack(0), [0.0; 4]);
        // Half-alpha red: premultiplied r = 128 with a = 128 gives straight red at alpha 128/255.
        let c = unpack(rgba(255, 0, 0, 128));
        assert!(near(c[0], 1.0) && near(c[1], 0.0) && near(c[2], 0.0));
        assert!(near(c[3], 128.0 / 255.0));
    }

    #[test]
    fn unpack_recovers_mid_grey() {
        // Straight (100, 150, 200) at alpha 200 premultiplies to (78, 118, 157); unpacking goes back to ~straight.
        let c = unpack(rgba(100, 150, 200, 200));
        assert!((c[0] * 255.0 - 100.0).abs() < 1.5);
        assert!((c[1] * 255.0 - 150.0).abs() < 1.5);
        assert!((c[2] * 255.0 - 200.0).abs() < 1.5);
        assert!(near(c[3], 200.0 / 255.0));
    }

    #[test]
    fn instance_is_128_bytes_with_32_words() {
        assert_eq!(std::mem::size_of::<Instance>(), INSTANCE_BYTES);
        let mut inst = Instance::default();
        inst.rect = [1.0, 2.0, 3.0, 4.0];
        inst.extra = [9.0, 8.0, 7.0, 6.0];
        let w = inst.words();
        assert_eq!(&w[0..4], &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(&w[28..32], &[9.0, 8.0, 7.0, 6.0]);
    }

    #[test]
    fn encode_writes_little_endian_words() {
        let inst = fill(RectF::new(1.0, 0.0, 2.0, 3.0), 0.0, pack(0, 0, 0, 255), ID, None);
        let mut bytes = Vec::new();
        encode(&[inst, inst], &mut bytes);
        assert_eq!(bytes.len(), 2 * INSTANCE_BYTES);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()), 3.0);
        // The second instance starts at byte 128: its rect.x is word 0 of that instance.
        assert_eq!(f32::from_le_bytes(bytes[128..132].try_into().unwrap()), 1.0);
    }

    #[test]
    fn push_constants_hold_the_target_size() {
        let p = push_constants(1280, 800);
        assert_eq!(f32::from_le_bytes(p[0..4].try_into().unwrap()), 1280.0);
        assert_eq!(f32::from_le_bytes(p[4..8].try_into().unwrap()), 800.0);
    }

    #[test]
    fn margin_is_one_device_pixel_and_three_sigma_for_shadows() {
        assert!(near(margin(Kind::Fill, 0.0, [1.0, 0.0, 0.0, 1.0]), 1.0));
        // Zoomed by 2: one device pixel is half a local pixel.
        assert!(near(margin(Kind::Stroke, 0.0, [2.0, 0.0, 0.0, 2.0]), 0.5));
        // Shrunk by 0.5: one device pixel is two local pixels.
        assert!(near(margin(Kind::Fill, 0.0, [0.5, 0.0, 0.0, 0.5]), 2.0));
        // Shadow with blur 10 (sigma 5): 1 + 15.
        assert!(near(margin(Kind::Shadow, 5.0, [1.0, 0.0, 0.0, 1.0]), 16.0));
        // Shadow sigma 5 at half scale: 2 + 15.
        assert!(near(margin(Kind::Shadow, 5.0, [0.5, 0.0, 0.0, 0.5]), 17.0));
    }

    #[test]
    fn quad_grows_the_rect_by_the_margin() {
        let inst = shadow(RectF::new(10.0, 20.0, 100.0, 50.0), 8.0, 10.0, (0.0, 0.0), pack(0, 0, 0, 255), ID, None);
        let q = quad(&inst);
        // sigma 5: margin 16 on each side.
        assert_eq!(q, RectF::new(-6.0, 4.0, 132.0, 82.0));
        let f = fill(RectF::new(0.0, 0.0, 10.0, 10.0), 0.0, pack(0, 0, 0, 255), ID, None);
        assert_eq!(quad(&f), RectF::new(-1.0, -1.0, 12.0, 12.0));
    }

    #[test]
    fn fill_encodes_rect_radius_colour_and_transform() {
        let xf = Transform2D { a: 2.0, b: 0.0, c: 0.0, d: 3.0, tx: 5.0, ty: 6.0 };
        let i = fill(RectF::new(1.0, 2.0, 30.0, 40.0), 6.0, rgba(0, 0, 255, 255), xf, None);
        assert_eq!(i.rect, [1.0, 2.0, 30.0, 40.0]);
        assert_eq!(i.params, [6.0, 0.0, 0.0, 0.0]);
        assert_eq!(i.color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(i.xform, [2.0, 0.0, 0.0, 3.0]);
        assert_eq!(i.xform_t, [5.0, 6.0, 0.0, 0.0]);
        assert_eq!(i.clip, [0.0; 4]);
        assert_eq!(kind_of(&i), Kind::Fill);
    }

    #[test]
    fn stroke_carries_its_width_and_kind() {
        let i = stroke(RectF::new(0.0, 0.0, 20.0, 20.0), 4.0, 2.0, pack(255, 255, 255, 255), ID, None);
        assert_eq!(i.params, [4.0, 2.0, 1.0, 0.0]);
        assert_eq!(kind_of(&i), Kind::Stroke);
        assert_eq!(quad(&i), RectF::new(-1.0, -1.0, 22.0, 22.0));
    }

    #[test]
    fn shadow_offsets_the_rect_and_halves_the_blur() {
        let i = shadow(RectF::new(10.0, 20.0, 100.0, 50.0), 8.0, 12.0, (4.0, -2.0), pack(0, 0, 0, 128), ID, None);
        assert_eq!(i.rect, [14.0, 18.0, 100.0, 50.0]);
        assert_eq!(i.params[1], 6.0, "sigma is blur / 2");
        assert_eq!(kind_of(&i), Kind::Shadow);
        assert!(near(i.color[3], 128.0 / 255.0));
    }

    #[test]
    fn shadow_coverage_is_one_half_at_the_edge() {
        // The shader's erf: erf(0) = 0, erf(1) = 0.8427007929, erf(2) = 0.9953222650.
        assert!(erf_ref(0.0).abs() < 1e-6);
        assert!((erf_ref(1.0) - 0.842_700_8).abs() < 1e-5);
        assert!((erf_ref(-1.0) + 0.842_700_8).abs() < 1e-5);
        assert!((erf_ref(2.0) - 0.995_322_3).abs() < 1e-5);
        // Half at the edge, nearly 1 three sigma inside, nearly 0 three sigma outside.
        assert!((shadow_cov(0.0, 5.0) - 0.5).abs() < 1e-6);
        assert!((shadow_cov(-15.0, 5.0) - 0.998_65).abs() < 1e-3);
        assert!((shadow_cov(15.0, 5.0) - 0.001_35).abs() < 1e-3);
    }

    #[test]
    fn stroke_band_is_centred_on_the_half_width() {
        // The shader's band distance |d + w/2| - w/2 is zero on the outer edge (d = 0) and on the inner edge
        // (d = -w), and negative (inside the band) between them.
        let w = 2.0f32;
        let band = |d: f32| (d + 0.5 * w).abs() - 0.5 * w;
        assert_eq!(band(0.0), 0.0);
        assert_eq!(band(-2.0), 0.0);
        assert_eq!(band(-1.0), -1.0);
        assert!(band(1.0) > 0.0 && band(-3.0) > 0.0);
    }

    #[test]
    fn image_and_mask_carry_uv_and_colour() {
        let uv = [0.0, 0.0, 0.5, 1.0];
        let img = image(RectF::new(0.0, 0.0, 64.0, 32.0), 4.0, uv, 0.25, ID, None);
        assert_eq!(img.color, [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(img.color2, uv);
        assert_eq!(kind_of(&img), Kind::Image);
        let m = mask(RectF::new(0.0, 0.0, 8.0, 8.0), 0.0, uv, pack(255, 0, 0, 255), ID, None);
        assert_eq!(m.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.color2, uv);
        assert_eq!(kind_of(&m), Kind::Mask);
    }

    #[test]
    fn clip_is_device_space_with_its_radius_and_empty_clips_draw_nothing() {
        let clip = Some(Clip { rect: RectF::new(10.0, 12.0, 100.0, 50.0), radius: 8.0 });
        let i = fill(RectF::new(0.0, 0.0, 5.0, 5.0), 0.0, pack(0, 0, 0, 255), ID, clip);
        assert_eq!(i.clip, [10.0, 12.0, 100.0, 50.0]);
        assert_eq!(i.extra[2], 8.0);
        let empty = fill(RectF::new(0.0, 0.0, 5.0, 5.0), 0.0, pack(0, 0, 0, 255), ID, Some(Clip { rect: RectF::new(0.0, 0.0, 0.0, 9.0), radius: 0.0 }));
        assert!(empty.clip[2] > 0.0 && empty.clip[0] < -1000.0, "an empty clip must cover nothing on screen");
    }

    #[test]
    fn gradient_with_two_stops_is_one_instance_with_both_pads() {
        let stops = [Stop { at: 0.0, color: pack(255, 0, 0, 255) }, Stop { at: 1.0, color: pack(0, 0, 255, 255) }];
        let v = gradient(RectF::new(0.0, 0.0, 100.0, 40.0), 6.0, (0.0, 0.0), (0.0, 40.0), &stops, ID, None);
        assert_eq!(v.len(), 1);
        let g = v[0];
        assert_eq!(kind_of(&g), Kind::LinearGradient);
        assert_eq!(g.params, [6.0, 0.0, 3.0, 1.0]);
        assert_eq!(g.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(g.color2, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(g.xform_t, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(g.extra[0..2], [0.0, 40.0]);
        assert_eq!(g.extra[3], 3.0, "pads before and after");
    }

    #[test]
    fn gradient_with_three_stops_is_two_instances_with_one_pad_each() {
        let stops = [
            Stop { at: 0.0, color: pack(255, 0, 0, 255) },
            Stop { at: 0.5, color: pack(0, 255, 0, 255) },
            Stop { at: 1.0, color: pack(0, 0, 255, 255) },
        ];
        let v = gradient(RectF::new(0.0, 0.0, 10.0, 10.0), 0.0, (1.0, 2.0), (3.0, 4.0), &stops, ID, None);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].params[1], 0.0);
        assert_eq!(v[0].params[3], 0.5);
        assert_eq!(v[0].extra[3], 1.0, "first instance pads before");
        assert_eq!(v[0].color2, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(v[1].params[1], 0.5);
        assert_eq!(v[1].params[3], 1.0);
        assert_eq!(v[1].extra[3], 2.0, "last instance pads after");
        assert_eq!(v[1].color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(v[1].xform_t[2..4], [1.0, 2.0]);
    }

    #[test]
    fn gradient_skips_zero_width_pairs_and_handles_degenerate_stops() {
        let stops = [
            Stop { at: 0.5, color: pack(255, 0, 0, 255) },
            Stop { at: 0.5, color: pack(0, 255, 0, 255) },
            Stop { at: 1.0, color: pack(0, 0, 255, 255) },
        ];
        let v = gradient(RectF::new(0.0, 0.0, 10.0, 10.0), 0.0, (0.0, 0.0), (1.0, 0.0), &stops, ID, None);
        assert_eq!(v.len(), 1, "the zero-width pair is skipped");
        assert_eq!(v[0].color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(v[0].extra[3], 3.0);
        assert!(gradient(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0, (0.0, 0.0), (1.0, 0.0), &[], ID, None).is_empty());
        let one = gradient(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0, (0.0, 0.0), (1.0, 0.0), &[Stop { at: 0.3, color: pack(0, 0, 0, 255) }], ID, None);
        assert_eq!(one.len(), 1);
        assert_eq!(kind_of(&one[0]), Kind::Fill);
        let same = [Stop { at: 0.2, color: pack(9, 9, 9, 255) }, Stop { at: 0.2, color: pack(1, 1, 1, 255) }];
        let flat = gradient(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0, (0.0, 0.0), (1.0, 0.0), &same, ID, None);
        assert_eq!(flat.len(), 1);
        assert_eq!(kind_of(&flat[0]), Kind::Fill);
    }

    #[test]
    fn gradient_sorts_unordered_stops() {
        let stops = [Stop { at: 1.0, color: pack(0, 0, 255, 255) }, Stop { at: 0.0, color: pack(255, 0, 0, 255) }];
        let v = gradient(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0, (0.0, 0.0), (1.0, 0.0), &stops, ID, None);
        assert_eq!(v[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v[0].color2, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn op_instances_maps_each_drawing_op_and_applies_opacity() {
        let red = pack(255, 0, 0, 255);
        let fr = Op::FillRect { r: RectF::new(0.0, 0.0, 4.0, 4.0), color: red };
        let v = op_instances(&fr, ID, None, 1.0);
        assert_eq!(v.len(), 1);
        assert_eq!(kind_of(&v[0]), Kind::Fill);
        let half = op_instances(&fr, ID, None, 0.5);
        assert!(near(half[0].color[3], 0.5));
        let st = Op::StrokeRRect { r: RectF::new(0.0, 0.0, 4.0, 4.0), radius: 1.0, width: 2.0, color: red };
        assert_eq!(op_instances(&st, ID, None, 1.0)[0].params, [1.0, 2.0, 1.0, 0.0]);
        let sh = Op::Shadow { r: RectF::new(0.0, 0.0, 4.0, 4.0), radius: 1.0, blur: 4.0, offset: (1.0, 1.0), color: red };
        assert_eq!(kind_of(&op_instances(&sh, ID, None, 1.0)[0]), Kind::Shadow);
        let im = Op::Image { id: 7, dst: RectF::new(0.0, 0.0, 4.0, 4.0), alpha: 1.0 };
        let img = op_instances(&im, ID, None, 0.5);
        assert_eq!(kind_of(&img[0]), Kind::Image);
        assert!(near(img[0].color[3], 0.5));
        assert_eq!(img[0].color2, [0.0, 0.0, 1.0, 1.0]);
        let tint = Op::ImageTinted { id: 8, dst: RectF::new(0.0, 0.0, 4.0, 4.0), color: red };
        assert_eq!(kind_of(&op_instances(&tint, ID, None, 1.0)[0]), Kind::Mask);
        assert!(op_instances(&Op::PopClip, ID, None, 1.0).is_empty());
        let lg = Op::LinearGradient {
            r: RectF::new(0.0, 0.0, 4.0, 4.0),
            radius: 0.0,
            start: (0.0, 0.0),
            end: (0.0, 4.0),
            stops: vec![Stop { at: 0.0, color: red }, Stop { at: 1.0, color: red }],
        };
        let faded = op_instances(&lg, ID, None, 0.5);
        assert!(near(faded[0].color[3], 0.5) && near(faded[0].color2[3], 0.5));
    }

    #[test]
    fn kind_round_trips_through_params() {
        for k in [Kind::Fill, Kind::Stroke, Kind::Shadow, Kind::LinearGradient, Kind::Image, Kind::Mask] {
            let i = Instance { params: [0.0, 0.0, kind_f(k), 0.0], ..Instance::default() };
            assert_eq!(kind_of(&i), k);
        }
    }
}
