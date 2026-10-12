//! The executor: walks a display list in order, keeps the clip/transform/opacity stacks, turns ops into batched
//! instances (`shapes::Instance`, `text` glyph quads, `images` quads, `paths` triangles), uploads what is new and
//! records one render pass into the target. Consecutive ops with the same pipeline and textures share a draw.
//!
//! Owner: wave G1 agent `gfx2d-renderer`.
//!
//! What this module does today, against the skeleton of the other modules:
//! - `Renderer::new` builds the shape pipeline, the glyph atlas, the image cache and a ring of three MAPPED instance
//!   buffers.
//! - `draw` walks the list. Transform, clip (device-space rectangle, intersected) and opacity (multiplied into the
//!   colour alpha) are kept on stacks. Shape ops (fill, rounded fill, stroke, shadow, material tint) become
//!   `shapes::Instance`s and are batched by `batch_runs`. Damage culls ops by their recorded device bounds and
//!   scissors the render pass.
//! - Gradients, images, tinted images, text and paths need the encoders of `shapes`, `text`, `images` and `paths`
//!   (a gradient end point and a triangle pipeline are not in the instance layout yet). They are skipped here and
//!   listed in `docs/graphics/g1-gfx2d-renderer.md`.
//!
//! Push constants for the shape pipeline: the target size as two little-endian f32 (width, height), 8 bytes.
//! Instance buffers are reused every `RING` frames: the caller must have waited for the submission of the frame
//! `RING` frames back (`Submission::wait` or `is_complete`) before it records a new frame into the same device.

use crate::shapes::{Instance, Kind, Shapes};
use bifrost_gpu::{Buffer, BufferUsage, CommandBuffer, Device, LoadAction, PixelFormat, RenderPassDescriptor, RenderPipeline, Region, Result, StoreAction, Texture};
use bifrost_render::canvas::{RectF, Transform2D};
use bifrost_render::display_list::{DisplayList, Op};
use std::time::Instant;

/// What one `draw` did, for the bench and the perf HUD.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameStats {
    pub ops: u32,
    pub draws: u32,
    pub instances: u32,
    pub glyphs_uploaded: u32,
    pub images_uploaded: u32,
    /// CPU time spent encoding, in microseconds.
    pub encode_us: u32,
}

/// Instance buffers in the ring.
const RING: usize = 3;
/// Bytes of one encoded `shapes::Instance` (seven float4 fields).
const INSTANCE_BYTES: usize = 112;
/// Starting capacity of each ring buffer: 256 instances.
const RING_INITIAL_BYTES: usize = 256 * INSTANCE_BYTES;
/// Default image cache budget (docs/graphics-architecture.md, "bifrost-2d" images: LRU, 64 MiB).
const IMAGE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

const _: () = assert!(std::mem::size_of::<Instance>() == INSTANCE_BYTES);

/// Owns the pipelines, the glyph atlas and the image cache for one device. One per process is enough.
pub struct Renderer {
    pub(crate) device: Device,
    pub(crate) shapes: crate::shapes::Shapes,
    // Held for the glyph and image draws, which wait for their modules' encoders (see the module docs).
    #[allow(dead_code)]
    pub(crate) text: crate::text::GlyphAtlas,
    #[allow(dead_code)]
    pub(crate) images: crate::images::ImageCache,
    /// The format `shapes` was made for.
    shapes_format: PixelFormat,
    /// Shape pipelines for other target formats, made on first use.
    extra_shapes: Vec<(PixelFormat, Shapes)>,
    /// The per-frame instance buffers, used round robin.
    ring: Vec<Buffer>,
    ring_next: usize,
}

impl Renderer {
    pub fn new(device: &Device) -> Result<Renderer> {
        let shapes_format = PixelFormat::Bgra8Unorm;
        let shapes = Shapes::new(device, shapes_format)?;
        let text = crate::text::GlyphAtlas::new(device)?;
        let images = crate::images::ImageCache::new(device, IMAGE_BUDGET_BYTES)?;
        let mut ring = Vec::with_capacity(RING);
        for _ in 0..RING {
            ring.push(device.new_buffer(RING_INITIAL_BYTES, BufferUsage::VERTEX | BufferUsage::MAPPED)?);
        }
        Ok(Renderer { device: device.clone(), shapes, text, images, shapes_format, extra_shapes: Vec::new(), ring, ring_next: 0 })
    }

    /// Records drawing `list` into `target` (premultiplied BGRA8 or RGBA8). With `damage` only that region is
    /// cleared and redrawn (scissored); `None` redraws everything.
    pub fn draw(&mut self, cb: &mut CommandBuffer, list: &DisplayList, target: &Texture, damage: Option<RectF>) -> Result<FrameStats> {
        let started = Instant::now();
        let (w, h) = (target.width(), target.height());
        let full = RectF::new(0.0, 0.0, w as f32, h as f32);
        let mut walk = Walk::new(full);
        let mut stats = FrameStats { ops: list.ops.len() as u32, ..FrameStats::default() };
        let mut instances: Vec<Instance> = Vec::with_capacity(list.ops.len());
        let mut keys: Vec<BatchKey> = Vec::with_capacity(list.ops.len());

        for (i, op) in list.ops.iter().enumerate() {
            match op {
                Op::PushClip { r, .. } => walk.push_clip(*r),
                Op::PopClip => walk.pop_clip(),
                Op::PushTransform(t) => walk.push_transform(*t),
                Op::PopTransform => walk.pop_transform(),
                Op::PushOpacity(a) => walk.push_opacity(*a),
                Op::PopOpacity => walk.pop_opacity(),
                _ => {
                    if let (Some(d), Some(b)) = (damage, list.bounds.get(i)) {
                        if intersect(*b, d).is_none() {
                            continue;
                        }
                    }
                    if let Some((inst, key)) = encode(op, &walk) {
                        instances.push(inst);
                        keys.push(key);
                    }
                }
            }
        }

        let runs = batch_runs(&keys);
        let pipeline = self.shape_pipeline(target.format())?;
        let buffer = self.instance_buffer(instances.len() * INSTANCE_BYTES)?;
        let mut bytes = Vec::with_capacity(instances.len() * INSTANCE_BYTES);
        for inst in &instances {
            bytes.extend_from_slice(&instance_bytes(inst));
        }
        if !bytes.is_empty() {
            buffer.write(0, &bytes)?;
        }

        let (load, render_area) = match damage {
            Some(d) => (LoadAction::Load, Some(damage_region(d, w, h))),
            None => (LoadAction::Clear([0.0; 4]), None),
        };
        let pass = RenderPassDescriptor { target: target.clone(), load, store: StoreAction::Store, render_area };
        let mut enc = cb.render_encoder(&pass)?;
        enc.set_pipeline(&pipeline);
        let mut push = [0u8; 8];
        push[..4].copy_from_slice(&(w as f32).to_le_bytes());
        push[4..].copy_from_slice(&(h as f32).to_le_bytes());
        enc.set_bytes(&push);
        enc.set_vertex_buffer(0, &buffer, 0);
        for run in &runs {
            match run.key {
                BatchKey::Shape => {
                    enc.draw(4, run.count, 0, run.first);
                    stats.draws += 1;
                }
                // Not produced yet: images and glyphs need the encoders of their modules (see the module docs).
                BatchKey::Mask(_) | BatchKey::Image(_) => {}
            }
        }
        enc.end();

        stats.instances = instances.len() as u32;
        stats.encode_us = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
        Ok(stats)
    }

    /// The shape pipeline for a target format: the one made in `new` or one made on first use.
    fn shape_pipeline(&mut self, format: PixelFormat) -> Result<RenderPipeline> {
        if format == self.shapes_format {
            return Ok(self.shapes.pipeline.clone());
        }
        if let Some((_, s)) = self.extra_shapes.iter().find(|(f, _)| *f == format) {
            return Ok(s.pipeline.clone());
        }
        let shapes = Shapes::new(&self.device, format)?;
        let pipeline = shapes.pipeline.clone();
        self.extra_shapes.push((format, shapes));
        Ok(pipeline)
    }

    /// The next ring buffer, grown to hold `bytes`.
    fn instance_buffer(&mut self, bytes: usize) -> Result<Buffer> {
        let slot = self.ring_next % RING;
        self.ring_next = self.ring_next.wrapping_add(1);
        if self.ring[slot].len() < bytes.max(1) {
            let len = bytes.max(self.ring[slot].len() * 2).max(RING_INITIAL_BYTES);
            self.ring[slot] = self.device.new_buffer(len, BufferUsage::VERTEX | BufferUsage::MAPPED)?;
        }
        Ok(self.ring[slot].clone())
    }
}

/// How consecutive instances share a draw: same pipeline and same texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchKey {
    /// The SDF shape pipeline (no texture).
    Shape,
    /// The glyph-mask pipeline sampling the atlas page `texture`.
    Mask(u64),
    /// The image pipeline sampling the image `texture`.
    Image(u64),
}

/// A run of consecutive instances with one key: `count` instances starting at `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub key: BatchKey,
    pub first: u32,
    pub count: u32,
}

/// Groups instances in order into runs of equal keys. Order is kept, so runs never reorder paint.
pub(crate) fn batch_runs(keys: &[BatchKey]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, &key) in keys.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.key == key => run.count += 1,
            _ => runs.push(Run { key, first: i as u32, count: 1 }),
        }
    }
    runs
}

/// The state of the walk: stacks of transforms, device-space clips and opacities. The base entry of each stack is
/// never popped, so the top always exists; unbalanced pops are ignored.
pub(crate) struct Walk {
    xf: Vec<Transform2D>,
    clip: Vec<RectF>,
    alpha: Vec<f32>,
}

impl Walk {
    pub(crate) fn new(full: RectF) -> Walk {
        Walk { xf: vec![Transform2D::IDENTITY], clip: vec![full], alpha: vec![1.0] }
    }

    pub(crate) fn transform(&self) -> Transform2D {
        self.xf[self.xf.len() - 1]
    }

    /// The device-space clip; an empty clip has zero width or height.
    pub(crate) fn clip(&self) -> RectF {
        self.clip[self.clip.len() - 1]
    }

    pub(crate) fn opacity(&self) -> f32 {
        self.alpha[self.alpha.len() - 1]
    }

    pub(crate) fn push_transform(&mut self, t: Transform2D) {
        let next = mul(self.transform(), t);
        self.xf.push(next);
    }

    pub(crate) fn pop_transform(&mut self) {
        if self.xf.len() > 1 {
            self.xf.pop();
        }
    }

    /// Intersects the clip with the device bounds of `r` (a rounded clip is drawn as its rectangle).
    pub(crate) fn push_clip(&mut self, r: RectF) {
        let device = bbox(self.transform(), r);
        let next = intersect_or_empty(self.clip(), device);
        self.clip.push(next);
    }

    pub(crate) fn pop_clip(&mut self) {
        if self.clip.len() > 1 {
            self.clip.pop();
        }
    }

    pub(crate) fn push_opacity(&mut self, alpha: f32) {
        let next = self.opacity() * alpha.clamp(0.0, 1.0);
        self.alpha.push(next);
    }

    pub(crate) fn pop_opacity(&mut self) {
        if self.alpha.len() > 1 {
            self.alpha.pop();
        }
    }
}

/// Turns one drawing op into a shape instance in the walk's state. `None` for ops the shape pipeline cannot draw
/// (gradients, images, text, paths), for empty rectangles and for an empty clip.
pub(crate) fn encode(op: &Op, walk: &Walk) -> Option<(Instance, BatchKey)> {
    let clip = walk.clip();
    if clip.w <= 0.0 || clip.h <= 0.0 {
        return None;
    }
    let (rect, radius, width, kind, color) = match *op {
        Op::FillRect { r, color } => (r, 0.0, 0.0, Kind::Fill, color),
        Op::FillRRect { r, radius, color } => (r, radius, 0.0, Kind::Fill, color),
        Op::StrokeRRect { r, radius, width, color } => (r, radius, width, Kind::Stroke, color),
        // The shadow's shape is its rectangle moved by the offset; the blur travels as its sigma (blur / 2).
        Op::Shadow { r, radius, blur, offset, color } => {
            (RectF::new(r.x + offset.0, r.y + offset.1, r.w, r.h), radius, blur * 0.5, Kind::Shadow, color)
        }
        // Glass is composed by the window server; in the 2D executor a material draws its tint.
        Op::Material { r, radius, style } => (r, radius, 0.0, Kind::Fill, style.tint),
        _ => return None,
    };
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let t = walk.transform();
    let inst = Instance {
        rect: [rect.x, rect.y, rect.w, rect.h],
        params: [radius, width, kind as u32 as f32, 0.0],
        color: straight_color(color, walk.opacity()),
        color2: [0.0; 4],
        xform: [t.a, t.b, t.c, t.d],
        xform_t: [t.tx, t.ty, 0.0, 0.0],
        clip: [clip.x, clip.y, clip.w, clip.h],
    };
    Some((inst, BatchKey::Shape))
}

/// Unpremultiplies a packed premultiplied RGBA8 colour (R in the low byte) and multiplies its alpha by `opacity`.
pub(crate) fn straight_color(packed: u32, opacity: f32) -> [f32; 4] {
    let ch = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.0;
    let a = ch(24);
    let unpremul = |c: f32| if a > 0.0 { (c / a).min(1.0) } else { 0.0 };
    [unpremul(ch(0)), unpremul(ch(8)), unpremul(ch(16)), a * opacity.clamp(0.0, 1.0)]
}

/// Little-endian bytes of an instance, fields in declaration order (112 bytes).
pub(crate) fn instance_bytes(i: &Instance) -> [u8; INSTANCE_BYTES] {
    let fields: [[f32; 4]; 7] = [i.rect, i.params, i.color, i.color2, i.xform, i.xform_t, i.clip];
    let mut out = [0u8; INSTANCE_BYTES];
    for (fi, field) in fields.iter().enumerate() {
        for (ci, v) in field.iter().enumerate() {
            let o = (fi * 4 + ci) * 4;
            out[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// `a` after `b`: the point `p` maps to `a(b(p))`.
pub(crate) fn mul(a: Transform2D, b: Transform2D) -> Transform2D {
    Transform2D {
        a: a.a * b.a + a.c * b.b,
        b: a.b * b.a + a.d * b.b,
        c: a.a * b.c + a.c * b.d,
        d: a.b * b.c + a.d * b.d,
        tx: a.a * b.tx + a.c * b.ty + a.tx,
        ty: a.b * b.tx + a.d * b.ty + a.ty,
    }
}

/// The bounding box of `r` after transform `t`.
pub(crate) fn bbox(t: Transform2D, r: RectF) -> RectF {
    let corners = [(r.x, r.y), (r.x + r.w, r.y), (r.x, r.y + r.h), (r.x + r.w, r.y + r.h)];
    let mut min = (f32::INFINITY, f32::INFINITY);
    let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in corners {
        let (px, py) = (t.a * x + t.c * y + t.tx, t.b * x + t.d * y + t.ty);
        min = (min.0.min(px), min.1.min(py));
        max = (max.0.max(px), max.1.max(py));
    }
    RectF::new(min.0, min.1, max.0 - min.0, max.1 - min.1)
}

/// The overlap of two rectangles, or `None` when they do not overlap.
pub(crate) fn intersect(a: RectF, b: RectF) -> Option<RectF> {
    let r = intersect_or_empty(a, b);
    (r.w > 0.0 && r.h > 0.0).then_some(r)
}

/// The overlap of two rectangles; an empty overlap has zero width or height.
fn intersect_or_empty(a: RectF, b: RectF) -> RectF {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    RectF::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// The scissor for damage `d`: rounded outwards to whole pixels and clipped to the target.
pub(crate) fn damage_region(d: RectF, w: u32, h: u32) -> Region {
    let (wf, hf) = (w as f32, h as f32);
    let x0 = d.x.floor().clamp(0.0, wf);
    let y0 = d.y.floor().clamp(0.0, hf);
    let x1 = (d.x + d.w).ceil().clamp(x0, wf);
    let y1 = (d.y + d.h).ceil().clamp(y0, hf);
    Region { x: x0 as u32, y: y0 as u32, width: (x1 - x0) as u32, height: (y1 - y0) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::canvas::{MaterialStyle, PathData, Transform2D};
    use bifrost_render::gfx::pack;

    fn r(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF::new(x, y, w, h)
    }

    fn translate(tx: f32, ty: f32) -> Transform2D {
        Transform2D { tx, ty, ..Transform2D::IDENTITY }
    }

    fn full_walk() -> Walk {
        Walk::new(r(0.0, 0.0, 800.0, 600.0))
    }

    #[test]
    fn mul_with_identity_is_unchanged() {
        let t = Transform2D { a: 2.0, b: 0.5, c: -1.0, d: 3.0, tx: 7.0, ty: -4.0 };
        assert_eq!(mul(t, Transform2D::IDENTITY), t);
        assert_eq!(mul(Transform2D::IDENTITY, t), t);
    }

    #[test]
    fn mul_composes_translations_and_scales() {
        // Scale by 2 then translate by (10, 20) in local space: point (1, 1) -> (2, 2) -> (12, 22).
        let scale = Transform2D { a: 2.0, d: 2.0, ..Transform2D::IDENTITY };
        let composed = mul(translate(10.0, 20.0), scale);
        assert_eq!((composed.a, composed.d), (2.0, 2.0));
        assert_eq!((composed.tx, composed.ty), (10.0, 20.0));
        let (x, y) = (composed.a * 1.0 + composed.c * 1.0 + composed.tx, composed.b + composed.d + composed.ty);
        assert_eq!((x, y), (12.0, 22.0));
    }

    #[test]
    fn bbox_of_a_rotated_rect_is_its_extent() {
        // Rotation by 90 degrees: (x, y) -> (-y, x). A 10 x 4 rect becomes 4 wide and 10 tall.
        let rot = Transform2D { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 0.0, ty: 0.0 };
        let b = bbox(rot, r(0.0, 0.0, 10.0, 4.0));
        assert_eq!(b, r(-4.0, 0.0, 4.0, 10.0));
    }

    #[test]
    fn intersect_returns_overlap_or_none() {
        assert_eq!(intersect(r(0.0, 0.0, 10.0, 10.0), r(5.0, 5.0, 10.0, 10.0)), Some(r(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(intersect(r(0.0, 0.0, 10.0, 10.0), r(10.0, 0.0, 5.0, 5.0)), None);
        assert_eq!(intersect(r(0.0, 0.0, 10.0, 10.0), r(2.0, 2.0, 1.0, 1.0)), Some(r(2.0, 2.0, 1.0, 1.0)));
    }

    #[test]
    fn damage_rounds_outwards_and_clamps() {
        let reg = damage_region(r(10.4, 20.6, 5.2, 3.0), 100, 100);
        assert_eq!(reg, Region { x: 10, y: 20, width: 6, height: 4 });
        let edge = damage_region(r(90.0, 90.0, 50.0, 50.0), 100, 100);
        assert_eq!(edge, Region { x: 90, y: 90, width: 10, height: 10 });
        let off = damage_region(r(-5.0, -5.0, 3.0, 3.0), 100, 100);
        assert_eq!(off, Region { x: 0, y: 0, width: 0, height: 0 });
    }

    #[test]
    fn straight_color_unpremultiplies_and_applies_opacity() {
        // Premultiplied (128, 0, 0, 128) is pure red at half alpha.
        let c = straight_color(pack(128, 0, 0, 128), 1.0);
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert_eq!(c[1], 0.0);
        assert!((c[3] - 128.0 / 255.0).abs() < 1e-6);
        let half = straight_color(pack(255, 255, 255, 255), 0.5);
        assert!((half[3] - 0.5).abs() < 1e-6);
        assert_eq!(straight_color(pack(0, 0, 0, 0), 1.0), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn batch_runs_merge_equal_neighbours_only() {
        let keys = [BatchKey::Shape, BatchKey::Shape, BatchKey::Image(7), BatchKey::Image(7), BatchKey::Image(8), BatchKey::Shape];
        let runs = batch_runs(&keys);
        assert_eq!(
            runs,
            vec![
                Run { key: BatchKey::Shape, first: 0, count: 2 },
                Run { key: BatchKey::Image(7), first: 2, count: 2 },
                Run { key: BatchKey::Image(8), first: 4, count: 1 },
                Run { key: BatchKey::Shape, first: 5, count: 1 },
            ]
        );
    }

    #[test]
    fn batch_runs_empty_and_uniform() {
        assert!(batch_runs(&[]).is_empty());
        let uniform = vec![BatchKey::Mask(3); 40];
        assert_eq!(batch_runs(&uniform), vec![Run { key: BatchKey::Mask(3), first: 0, count: 40 }]);
    }

    #[test]
    fn walk_stacks_restore_on_pop_and_ignore_extra_pops() {
        let mut w = full_walk();
        w.push_transform(translate(5.0, 5.0));
        w.push_transform(translate(5.0, 0.0));
        assert_eq!((w.transform().tx, w.transform().ty), (10.0, 5.0));
        w.pop_transform();
        assert_eq!((w.transform().tx, w.transform().ty), (5.0, 5.0));
        w.pop_transform();
        w.pop_transform();
        w.pop_clip();
        w.pop_opacity();
        assert_eq!(w.transform(), Transform2D::IDENTITY);
        assert_eq!(w.opacity(), 1.0);
    }

    #[test]
    fn walk_clip_is_intersected_in_device_space() {
        let mut w = full_walk();
        w.push_transform(translate(100.0, 100.0));
        w.push_clip(r(0.0, 0.0, 50.0, 50.0));
        assert_eq!(w.clip(), r(100.0, 100.0, 50.0, 50.0));
        w.push_clip(r(0.0, 0.0, 20.0, 20.0));
        assert_eq!(w.clip(), r(100.0, 100.0, 20.0, 20.0));
        w.push_clip(r(500.0, 500.0, 10.0, 10.0));
        assert!(w.clip().w <= 0.0);
        w.pop_clip();
        w.pop_clip();
        assert_eq!(w.clip(), r(100.0, 100.0, 50.0, 50.0));
    }

    #[test]
    fn walk_opacity_multiplies_and_clamps() {
        let mut w = full_walk();
        w.push_opacity(0.5);
        w.push_opacity(0.5);
        assert!((w.opacity() - 0.25).abs() < 1e-6);
        w.push_opacity(3.0);
        assert_eq!(w.opacity(), 0.25);
        w.pop_opacity();
        w.pop_opacity();
        assert!((w.opacity() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn fill_rect_encodes_rect_colour_and_transform() {
        let mut w = full_walk();
        w.push_transform(translate(10.0, 20.0));
        w.push_opacity(0.5);
        let op = Op::FillRect { r: r(1.0, 2.0, 3.0, 4.0), color: pack(255, 255, 255, 255) };
        let (inst, key) = encode(&op, &w).unwrap();
        assert_eq!(key, BatchKey::Shape);
        assert_eq!(inst.rect, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(inst.params[2], Kind::Fill as u32 as f32);
        assert_eq!(inst.xform, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(inst.xform_t, [10.0, 20.0, 0.0, 0.0]);
        assert!((inst.color[3] - 0.5).abs() < 1e-6);
        assert_eq!(inst.clip, [0.0, 0.0, 800.0, 600.0]);
    }

    #[test]
    fn stroke_and_rounded_fill_carry_radius_and_width() {
        let w = full_walk();
        let stroke = Op::StrokeRRect { r: r(0.0, 0.0, 10.0, 10.0), radius: 3.0, width: 2.0, color: pack(0, 0, 0, 255) };
        let (inst, _) = encode(&stroke, &w).unwrap();
        assert_eq!(inst.params[0], 3.0);
        assert_eq!(inst.params[1], 2.0);
        assert_eq!(inst.params[2], Kind::Stroke as u32 as f32);
        let fill = Op::FillRRect { r: r(0.0, 0.0, 10.0, 10.0), radius: 5.0, color: pack(0, 0, 0, 255) };
        let (inst, _) = encode(&fill, &w).unwrap();
        assert_eq!(inst.params[0], 5.0);
        assert_eq!(inst.params[2], Kind::Fill as u32 as f32);
    }

    #[test]
    fn shadow_moves_by_offset_and_sigma_is_half_blur() {
        let w = full_walk();
        let op = Op::Shadow { r: r(10.0, 10.0, 100.0, 40.0), radius: 8.0, blur: 24.0, offset: (0.0, 4.0), color: pack(0, 0, 0, 128) };
        let (inst, _) = encode(&op, &w).unwrap();
        assert_eq!(inst.rect, [10.0, 14.0, 100.0, 40.0]);
        assert_eq!(inst.params[1], 12.0);
        assert_eq!(inst.params[2], Kind::Shadow as u32 as f32);
    }

    #[test]
    fn material_draws_its_tint() {
        let w = full_walk();
        let op = Op::Material { r: r(0.0, 0.0, 4.0, 4.0), radius: 0.0, style: MaterialStyle { blur: 20.0, tint: pack(255, 255, 255, 255), saturation: 1.0 } };
        let (inst, key) = encode(&op, &w).unwrap();
        assert_eq!(key, BatchKey::Shape);
        assert_eq!(inst.color, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn unsupported_and_empty_ops_encode_to_nothing() {
        let w = full_walk();
        assert!(encode(&Op::Path { path: PathData::default(), paint: bifrost_render::canvas::PathPaint::FillNonZero, color: 0 }, &w).is_none());
        assert!(encode(&Op::FillRect { r: r(0.0, 0.0, 0.0, 5.0), color: 0 }, &w).is_none());
        assert!(encode(&Op::PopClip, &w).is_none());
        let mut clipped = full_walk();
        clipped.push_clip(r(900.0, 900.0, 5.0, 5.0));
        assert!(encode(&Op::FillRect { r: r(0.0, 0.0, 1.0, 1.0), color: 0 }, &clipped).is_none());
    }

    #[test]
    fn instance_bytes_are_little_endian_fields_in_order() {
        let inst = Instance { rect: [1.0, 0.0, 0.0, 0.0], clip: [0.0, 0.0, 0.0, 2.0], ..Instance::default() };
        let bytes = instance_bytes(&inst);
        assert_eq!(bytes.len(), 112);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[108..112], &2.0f32.to_le_bytes());
        assert!(bytes[4..108].iter().all(|&b| b == 0));
    }
}
