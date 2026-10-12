//! The executor: walks a display list in order, keeps the clip/transform/opacity stacks, turns ops into instances of
//! the one SDF shape pipeline (`shapes`), and records one render pass into the target. Consecutive instances that
//! sample the same texture share a draw, so paint order is kept exactly.
//!
//! - Shapes (fills, rounded fills, strokes, shadows, gradients) come from `shapes::op_instances`.
//! - Text is laid out by the glyph atlas (`text::GlyphAtlas::layout`) into mask instances, then placed in the current
//!   transform, clip and opacity.
//! - Images and tinted images are uploaded once per id by the image cache (`images::ImageCache`).
//! - Paths are rasterised on the CPU into a mask the size of their device bounds (through the `Canvas` impl of
//!   `Surface`) and drawn as an image; a GPU path pipeline can replace this without changing callers.
//! - A material draws its tint only: the window server blurs what is behind the layer (docs/graphics-architecture.md).
//!
//! Instance data goes into a fresh host-visible buffer per draw. The command buffer keeps it alive until the GPU is
//! done with it, so a frame never overwrites data an earlier, still running frame reads.
//!
//! Owner: wave G1 agent `gfx2d-renderer`; rewritten in the lead's review to use the shapes, text and images helpers.

use crate::shapes::{self, Clip, Instance, Shapes, INSTANCE_BYTES};
use bifrost_gpu::{BufferUsage, CommandBuffer, Device, LoadAction, PixelFormat, Region, RenderPassDescriptor, Result, StoreAction, Texture, TextureDescriptor, TextureUsage};
use bifrost_render::canvas::{self, Canvas, ImageRef, RectF, Transform2D};
use bifrost_render::display_list::{DisplayList, Op};
use bifrost_render::gfx::{Image, Surface};
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

/// The byte budget of the image cache.
const IMAGE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

/// The glyph atlas is reset at the start of a frame once its shelves reach this fraction of its height, so a reset
/// never happens half way through a frame (that would move glyphs earlier runs of the frame point at).
const ATLAS_RESET_FILL: f32 = 0.75;

const _: () = assert!(std::mem::size_of::<Instance>() == INSTANCE_BYTES);

/// Owns the pipelines, the glyph atlas and the image cache for one device. One per process is enough.
pub struct Renderer {
    pub(crate) device: Device,
    pub(crate) shapes: crate::shapes::Shapes,
    pub(crate) text: crate::text::GlyphAtlas,
    pub(crate) images: crate::images::ImageCache,
    /// The format `shapes` was made for.
    shapes_format: PixelFormat,
    /// Shape pipelines for other target formats, made on first use.
    extra_shapes: Vec<(PixelFormat, Shapes)>,
}

/// The texture an instance samples, as a batching key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tex {
    /// No texture (shapes); the atlas is bound as a placeholder.
    None,
    Atlas,
    Image(u64),
    /// A texture made for this frame only (a rasterised path), by index into the frame's list.
    Temp(usize),
}

/// A run of consecutive instances with one texture: `count` instances starting at `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub tex: Tex,
    pub first: u32,
    pub count: u32,
}

impl Renderer {
    pub fn new(device: &Device) -> Result<Renderer> {
        let shapes_format = PixelFormat::Bgra8Unorm;
        let shapes = Shapes::new(device, shapes_format)?;
        let text = crate::text::GlyphAtlas::new(device)?;
        let images = crate::images::ImageCache::new(device, IMAGE_BUDGET_BYTES)?;
        Ok(Renderer { device: device.clone(), shapes, text, images, shapes_format, extra_shapes: Vec::new() })
    }

    /// Records drawing `list` into `target` (premultiplied BGRA8 or RGBA8). With `damage` only that region is
    /// redrawn (ops outside it are culled and the pass is scissored to it, keeping the rest); `None` clears and
    /// redraws everything.
    pub fn draw(&mut self, cb: &mut CommandBuffer, list: &DisplayList, target: &Texture, damage: Option<RectF>) -> Result<FrameStats> {
        let started = Instant::now();
        let (w, h) = (target.width(), target.height());
        let mut walk = Walk::new(RectF::new(0.0, 0.0, w as f32, h as f32));
        let mut stats = FrameStats { ops: list.ops.len() as u32, ..FrameStats::default() };
        let mut instances: Vec<Instance> = Vec::with_capacity(list.ops.len());
        let mut keys: Vec<Tex> = Vec::with_capacity(list.ops.len());
        let mut temps: Vec<Texture> = Vec::new();

        if self.text.fill_ratio() >= ATLAS_RESET_FILL {
            self.text.reset();
        }

        for (i, op) in list.ops.iter().enumerate() {
            match op {
                Op::PushClip { r, radius } => walk.push_clip(*r, *radius),
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
                    if walk.clipped_out() {
                        continue;
                    }
                    let (xf, clip, opacity) = (walk.transform(), walk.clip(), walk.opacity());
                    match op {
                        Op::Text { run, color } => {
                            let start = instances.len();
                            let atlas = &mut self.text;
                            let uploaded = canvas::with_face(run.face, |font| atlas.layout(font, run, *color, &mut instances));
                            match uploaded {
                                Some(Ok(n)) => stats.glyphs_uploaded += n,
                                Some(Err(e)) => return Err(e),
                                None => {}
                            }
                            for inst in &mut instances[start..] {
                                shapes::restate(inst, xf, clip, opacity);
                                keys.push(Tex::Atlas);
                            }
                        }
                        Op::Image { id, .. } | Op::ImageTinted { id, .. } => {
                            let Some(image) = list.images.iter().find(|(k, _)| k == id).map(|(_, img)| img.clone()) else { continue };
                            let resident = self.images.contains(*id);
                            self.images.get(&ImageRef { id: *id, image: &image })?;
                            if !resident {
                                stats.images_uploaded += 1;
                            }
                            for inst in shapes::op_instances(op, xf, clip, opacity) {
                                instances.push(inst);
                                keys.push(Tex::Image(*id));
                            }
                        }
                        Op::Path { path, paint, color } => {
                            let Some(b) = list.bounds.get(i).copied().and_then(|b| intersect(b, walk.full)) else { continue };
                            let Some((img, dst)) = rasterize_path(path, *paint, *color, xf, b) else { continue };
                            let tex = upload_temp(&self.device, &img)?;
                            temps.push(tex);
                            let mut inst = shapes::image(dst, 0.0, [0.0, 0.0, 1.0, 1.0], opacity, Transform2D::IDENTITY, clip);
                            inst.color[3] = opacity.clamp(0.0, 1.0);
                            instances.push(inst);
                            keys.push(Tex::Temp(temps.len() - 1));
                        }
                        Op::Material { r, radius, style } => {
                            for inst in shapes::op_instances(&Op::FillRRect { r: *r, radius: *radius, color: style.tint }, xf, clip, opacity) {
                                instances.push(inst);
                                keys.push(Tex::None);
                            }
                        }
                        _ => {
                            for inst in shapes::op_instances(op, xf, clip, opacity) {
                                instances.push(inst);
                                keys.push(Tex::None);
                            }
                        }
                    }
                }
            }
        }

        let runs = batch_runs(&keys);
        let pipeline_shapes = self.shapes_for(target.format())?;
        let (load, render_area) = match damage {
            Some(d) => (LoadAction::Load, Some(damage_region(d, w, h))),
            None => (LoadAction::Clear([0.0; 4]), None),
        };
        let pass = RenderPassDescriptor { target: target.clone(), load, store: StoreAction::Store, render_area };
        let mut enc = cb.render_encoder(&pass)?;
        if !instances.is_empty() {
            let mut bytes = Vec::with_capacity(instances.len() * INSTANCE_BYTES);
            shapes::encode(&instances, &mut bytes);
            let buffer = self.device.new_buffer(bytes.len(), BufferUsage::VERTEX | BufferUsage::MAPPED)?;
            buffer.write(0, &bytes)?;
            for run in &runs {
                let tex = match run.tex {
                    Tex::None | Tex::Atlas => self.text.texture().clone(),
                    Tex::Image(id) => match self.images.texture(id) {
                        Some(t) => t,
                        None => continue,
                    },
                    Tex::Temp(i) => temps[i].clone(),
                };
                let shapes = match pipeline_shapes {
                    None => &self.shapes,
                    Some(i) => &self.extra_shapes[i].1,
                };
                shapes.draw(&mut enc, &buffer, run.first, run.count, (w, h), &tex);
                stats.draws += 1;
            }
        }
        enc.end();

        stats.instances = instances.len() as u32;
        stats.encode_us = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
        Ok(stats)
    }

    /// The shapes pipeline for a target format: `None` for the one made in `new`, else the index of one made on
    /// first use.
    fn shapes_for(&mut self, format: PixelFormat) -> Result<Option<usize>> {
        if format == self.shapes_format {
            return Ok(None);
        }
        if let Some(i) = self.extra_shapes.iter().position(|(f, _)| *f == format) {
            return Ok(Some(i));
        }
        self.extra_shapes.push((format, Shapes::new(&self.device, format)?));
        Ok(Some(self.extra_shapes.len() - 1))
    }
}

/// Rasterises a path on the CPU into a premultiplied image covering `bounds` (device pixels), in transform `xf`.
/// Returns the image and the device rectangle it covers; `None` when the area is empty.
pub(crate) fn rasterize_path(path: &canvas::PathData, paint: canvas::PathPaint, color: u32, xf: Transform2D, bounds: RectF) -> Option<(Image, RectF)> {
    let x0 = bounds.x.floor();
    let y0 = bounds.y.floor();
    let w = ((bounds.x + bounds.w).ceil() - x0).max(0.0) as usize;
    let h = ((bounds.y + bounds.h).ceil() - y0).max(0.0) as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let mut img = Image::new(w, h);
    {
        let mut s = Surface { w, h, stride: w, px: &mut img.px[..], y0: 0 };
        // Device -> image pixels: subtract the image origin after the op's own transform.
        let local = Transform2D { tx: xf.tx - x0, ty: xf.ty - y0, ..xf };
        s.push_transform(local);
        s.path(path, paint, color);
        s.pop_transform();
    }
    Some((img, RectF::new(x0, y0, w as f32, h as f32)))
}

/// Uploads a frame-only texture (kept alive by the command buffer that samples it).
fn upload_temp(device: &Device, img: &Image) -> Result<Texture> {
    let (w, h) = (img.w as u32, img.h as u32);
    let tex = device.new_texture(&TextureDescriptor { width: w, height: h, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST, exportable: false, modifiers: Vec::new() })?;
    let bytes: Vec<u8> = img.px.iter().flat_map(|p| p.to_le_bytes()).collect();
    tex.replace_region(Region { x: 0, y: 0, width: w, height: h }, &bytes, w as usize * 4)?;
    Ok(tex)
}

/// Groups instances in order into runs of equal textures. Order is kept, so runs never reorder paint.
pub(crate) fn batch_runs(keys: &[Tex]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        match runs.last_mut() {
            // A shape needs no texture, so it joins any run; a textured instance joins a run of its texture or of
            // shapes only.
            Some(last) if compatible(last.tex, *k) => {
                if last.tex == Tex::None {
                    last.tex = *k;
                }
                last.count += 1;
            }
            _ => runs.push(Run { tex: *k, first: i as u32, count: 1 }),
        }
    }
    runs
}

fn compatible(run: Tex, next: Tex) -> bool {
    run == next || next == Tex::None || run == Tex::None
}

/// The clip/transform/opacity state while walking a list.
#[derive(Debug, Clone)]
pub(crate) struct Walk {
    pub(crate) full: RectF,
    transforms: Vec<Transform2D>,
    /// Device-space clips, each the intersection with its parent; the radius is the innermost rounded clip's.
    clips: Vec<Clip>,
    opacities: Vec<f32>,
}

impl Walk {
    pub(crate) fn new(full: RectF) -> Walk {
        Walk { full, transforms: Vec::new(), clips: Vec::new(), opacities: Vec::new() }
    }

    pub(crate) fn transform(&self) -> Transform2D {
        self.transforms.last().copied().unwrap_or(Transform2D::IDENTITY)
    }

    /// The current clip, or `None` when nothing is clipped.
    pub(crate) fn clip(&self) -> Option<Clip> {
        self.clips.last().copied()
    }

    /// Whether the current clip is empty (everything is cut).
    pub(crate) fn clipped_out(&self) -> bool {
        self.clips.last().is_some_and(|c| c.rect.w <= 0.0 || c.rect.h <= 0.0)
    }

    pub(crate) fn opacity(&self) -> f32 {
        self.opacities.last().copied().unwrap_or(1.0)
    }

    pub(crate) fn push_transform(&mut self, t: Transform2D) {
        let next = self.transform().mul(t);
        self.transforms.push(next);
    }

    pub(crate) fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    /// Clips to `r` (in the current transform): its device bounding box intersected with the current clip. The
    /// radius is kept for axis-aligned transforms (scaled by them); under rotation the clip is the bounding box.
    pub(crate) fn push_clip(&mut self, r: RectF, radius: f32) {
        let t = self.transform();
        let rect = bbox(t, r);
        let axis_aligned = t.b == 0.0 && t.c == 0.0;
        let radius = if axis_aligned { radius * t.a.abs().min(t.d.abs()) } else { 0.0 };
        let parent = self.clips.last().map(|c| c.rect).unwrap_or(self.full);
        let rect = intersect_or_empty(rect, parent);
        self.clips.push(Clip { rect, radius });
    }

    pub(crate) fn pop_clip(&mut self) {
        self.clips.pop();
    }

    pub(crate) fn push_opacity(&mut self, alpha: f32) {
        let next = (self.opacity() * alpha).clamp(0.0, 1.0);
        self.opacities.push(next);
    }

    pub(crate) fn pop_opacity(&mut self) {
        self.opacities.pop();
    }
}

/// The bounding box of `r` after transform `t`.
pub(crate) fn bbox(t: Transform2D, r: RectF) -> RectF {
    let corners = [(r.x, r.y), (r.x + r.w, r.y), (r.x, r.y + r.h), (r.x + r.w, r.y + r.h)];
    let mut min = (f32::INFINITY, f32::INFINITY);
    let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in corners {
        let (px, py) = t.apply(x, y);
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

    fn r(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF::new(x, y, w, h)
    }

    fn walk() -> Walk {
        Walk::new(r(0.0, 0.0, 800.0, 600.0))
    }

    #[test]
    fn runs_keep_order_and_shapes_join_any_texture_run() {
        use Tex::*;
        let runs = batch_runs(&[None, None, Atlas, None, Image(7), Image(7), Atlas]);
        // [None, None] become the atlas run with the glyph and the shape after it; then image 7; then the atlas.
        assert_eq!(runs, vec![Run { tex: Atlas, first: 0, count: 4 }, Run { tex: Image(7), first: 4, count: 2 }, Run { tex: Atlas, first: 6, count: 1 }]);
        assert!(batch_runs(&[]).is_empty());
        assert_eq!(batch_runs(&[None, None]), vec![Run { tex: None, first: 0, count: 2 }]);
    }

    #[test]
    fn different_textures_never_share_a_run() {
        let runs = batch_runs(&[Tex::Image(1), Tex::Image(2), Tex::Temp(0)]);
        assert_eq!(runs.len(), 3);
    }

    #[test]
    fn clips_intersect_in_device_space_and_keep_a_scaled_radius() {
        let mut w = walk();
        w.push_transform(Transform2D::translate(10.0, 20.0));
        w.push_clip(r(0.0, 0.0, 100.0, 50.0), 8.0);
        assert_eq!(w.clip(), Some(Clip { rect: r(10.0, 20.0, 100.0, 50.0), radius: 8.0 }));
        w.push_transform(Transform2D::scale(2.0, 2.0));
        w.push_clip(r(0.0, 0.0, 30.0, 30.0), 4.0);
        // 30 x 30 scaled by 2 at (10, 20) is (10, 20, 60, 60), cut to the parent's height 50; radius 4 * 2.
        assert_eq!(w.clip(), Some(Clip { rect: r(10.0, 20.0, 60.0, 50.0), radius: 8.0 }));
        w.pop_clip();
        w.pop_transform();
        assert_eq!(w.clip().unwrap().rect, r(10.0, 20.0, 100.0, 50.0));
        w.pop_clip();
        assert_eq!(w.clip(), None);
    }

    #[test]
    fn a_disjoint_clip_cuts_everything() {
        let mut w = walk();
        w.push_clip(r(0.0, 0.0, 10.0, 10.0), 0.0);
        w.push_clip(r(50.0, 50.0, 10.0, 10.0), 0.0);
        assert!(w.clipped_out());
        w.pop_clip();
        assert!(!w.clipped_out());
    }

    #[test]
    fn rotation_drops_the_clip_radius() {
        let mut w = walk();
        w.push_transform(Transform2D { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 100.0, ty: 0.0 });
        w.push_clip(r(0.0, 0.0, 10.0, 20.0), 5.0);
        assert_eq!(w.clip().unwrap().radius, 0.0);
    }

    #[test]
    fn opacity_multiplies_and_restores() {
        let mut w = walk();
        w.push_opacity(0.5);
        w.push_opacity(0.5);
        assert_eq!(w.opacity(), 0.25);
        w.pop_opacity();
        assert_eq!(w.opacity(), 0.5);
        w.pop_opacity();
        w.pop_opacity();
        assert_eq!(w.opacity(), 1.0, "extra pops are harmless");
    }

    #[test]
    fn bbox_of_a_quarter_turn_swaps_the_sides() {
        let t = Transform2D { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 0.0, ty: 0.0 };
        assert_eq!(bbox(t, r(0.0, 0.0, 10.0, 20.0)), r(-20.0, 0.0, 20.0, 10.0));
    }

    #[test]
    fn damage_rounds_outwards_and_clamps() {
        assert_eq!(damage_region(r(1.5, 2.25, 3.0, 4.0), 100, 100), Region { x: 1, y: 2, width: 4, height: 5 });
        assert_eq!(damage_region(r(-10.0, 90.0, 50.0, 50.0), 100, 100), Region { x: 0, y: 90, width: 40, height: 10 });
    }

    #[test]
    fn a_path_rasterises_into_its_device_bounds() {
        use bifrost_render::canvas::{PathData, PathPaint, PathVerb};
        let path = PathData { verbs: vec![PathVerb::Move(0.0, 0.0), PathVerb::Line(10.0, 0.0), PathVerb::Line(10.0, 10.0), PathVerb::Line(0.0, 10.0), PathVerb::Close] };
        let xf = Transform2D::translate(20.0, 30.0);
        let (img, dst) = rasterize_path(&path, PathPaint::FillNonZero, 0xffff_ffff, xf, r(20.0, 30.0, 10.0, 10.0)).unwrap();
        assert_eq!(dst, r(20.0, 30.0, 10.0, 10.0));
        assert_eq!((img.w, img.h), (10, 10));
        // The square fills its image: the centre pixel is opaque white.
        assert_eq!(img.px[5 * 10 + 5], 0xffff_ffff);
        assert!(rasterize_path(&path, PathPaint::FillNonZero, 0xffff_ffff, xf, r(0.0, 0.0, 0.0, 5.0)).is_none());
    }
}
