//! Scene composition: turns the window stack (background and bottom layer-shell surfaces, windows back to front,
//! top and overlay layer-shell surfaces, the cursor) into textured quads with transforms, opacity, rounded clips and
//! materials, culled and scissored to the output's damage, and records them into one render pass with
//! `gpu/shaders/compose.vert` + `compose.frag` (premultiplied textured quad with rounded-rect clip and opacity).
//! Layer-tree surfaces are drawn from `layers::Context::presentation` (each presented layer with contents is a quad
//! of its bound texture; background colours and borders are SDF quads; shadows and materials come from `material`).
//! Damage: per output, the union of surface damage (transformed), layer tree damage and moved/opened/closed windows,
//! accumulated over the buffer age of the target.
//!
//! The pieces, in the order a frame uses them:
//! - `merge_damage` (pure): the output's damage rectangles, clipped, merged and bounded.
//! - `build_items` (pure): a plain description of the stack (`SceneNode`, device-independent) becomes `DrawItem`s in
//!   output pixels, dropping items that cannot touch the damage.
//! - `record`: draws the items into the target. An item with a material first ends the open render pass, asks the
//!   `MaterialPass` to blur the backdrop (compute passes cannot run inside a render pass), then draws the glass;
//!   a shadow is drawn before its item. Everything else is one render pass per run of draws, scissored to the
//!   damage's bounding box.
//!
//! Shader contract (`compose.vert` / `compose.frag`): one push-constant block of 128 bytes (layout in `Push`),
//! a triangle strip of four corners (top-left, top-right, bottom-left, bottom-right), texture binding 0 and sampler
//! binding 1 of set 0. Flags: `FLAG_TEXTURED` samples the texture, `FLAG_SHADOW` draws an analytic Gaussian-edge
//! shadow of `shadow_rect` (the solid colour is the shadow colour), `FLAG_MATERIAL` samples the blurred backdrop,
//! applies saturation and puts `color` (the tint, premultiplied) over it. Solid draws bind the one-pixel `dummy`.
//!
//! Owner: wave G1 agent `wm-compose`.

use crate::render_server::material::{MaterialRegion, ShadowSpec};
use bifrost_gpu::pipeline::{BlendMode, Filter};
use bifrost_gpu::{
    CommandBuffer, Device, LoadAction, PixelFormat, Region, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, Result,
    Sampler, SamplerDescriptor, StoreAction, Texture, TextureDescriptor, TextureUsage,
};

/// One thing to draw, in output pixels, back to front.
#[derive(Clone)]
pub struct DrawItem {
    pub texture: Option<Texture>,
    /// Destination quad corners after transform: top-left, top-right, bottom-left, bottom-right.
    pub quad: [[f32; 2]; 4],
    /// Source rectangle in texels: x, y, width, height. A width or height of 0 means the whole texture.
    pub src: [f32; 4],
    pub opacity: f32,
    /// Solid colour (premultiplied RGBA) when there is no texture.
    pub color: [f32; 4],
    /// Rounded clip in output pixels (x, y, w, h, radius); w = 0 disables it.
    pub clip: [f32; 5],
    /// A material drawn behind this item first (blur of what is already composited).
    pub material: Option<MaterialRegion>,
    /// A server-drawn drop shadow behind this item.
    pub shadow: Option<ShadowSpec>,
}

/// The output's damage as rectangles in output pixels.
pub type DamageRects = Vec<[i32; 4]>;

/// Above this many rectangles the damage collapses to its bounding box.
const MAX_DAMAGE_RECTS: usize = 16;

/// Pixels of shadow falloff drawn beyond the shadow rect, in multiples of its sigma.
const SHADOW_SIGMA_REACH: f32 = 3.0;

/// Push-constant flags (the `flags` field of `Push`).
pub const FLAG_TEXTURED: u32 = 1;
pub const FLAG_SHADOW: u32 = 2;
pub const FLAG_MATERIAL: u32 = 4;

/// Builds the damage for a frame from the pieces (pure, host-tested): union, clipped to the output, merged when
/// overlapping, collapsed to the bounding box above 16 rectangles. Rectangles are `[x, y, w, h]`; an empty result
/// means nothing to draw.
pub fn merge_damage(rects: &[[i32; 4]], output: (i32, i32)) -> DamageRects {
    let screen = [0, 0, output.0, output.1];
    let mut v: Vec<[i32; 4]> = rects.iter().filter_map(|r| intersect(*r, screen)).collect();
    // Merge overlapping pairs until no two overlap. Touching rectangles stay apart.
    'again: loop {
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                if overlaps(v[i], v[j]) {
                    v[i] = union(v[i], v[j]);
                    v.swap_remove(j);
                    continue 'again;
                }
            }
        }
        break;
    }
    if v.len() > MAX_DAMAGE_RECTS {
        v = match bounding(&v) {
            Some(b) => vec![b],
            None => Vec::new(),
        };
    }
    v
}

/// The intersection of two `[x, y, w, h]` rectangles, or `None` when it has no area.
fn intersect(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some([x0, y0, x1 - x0, y1 - y0])
    }
}

/// Whether two rectangles share some area (touching edges do not count).
fn overlaps(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The smallest rectangle that holds all of `v`, or `None` when `v` is empty.
fn bounding(v: &[[i32; 4]]) -> Option<[i32; 4]> {
    let first = *v.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0] + first[2], first[1] + first[3]);
    for r in &v[1..] {
        x0 = x0.min(r[0]);
        y0 = y0.min(r[1]);
        x1 = x1.max(r[0] + r[2]);
        y1 = y1.max(r[1] + r[3]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// The union of two `[x, y, w, h]` rectangles.
fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    bounding(&[a, b]).unwrap_or(a)
}

/// A glass material of a node, in logical pixels: blur radius, premultiplied tint, saturation (1 = none).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glass {
    pub blur: f32,
    pub tint: [f32; 4],
    pub saturation: f32,
}

/// One node of the stack as the window server knows it: a window, a layer-shell surface, a layer-tree layer or the
/// cursor. Plain data, so the builder is host-testable. Nodes are given back to front.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    /// The content's texture key (`import::Importer` key); `None` draws `color` alone.
    pub texture: Option<u64>,
    /// Source rectangle in texels (x, y, w, h); w or h of 0 means the whole texture.
    pub src: [f32; 4],
    /// Rectangle in logical pixels: x, y, w, h.
    pub rect: [f32; 4],
    /// Affine transform (a, b, c, d, tx, ty) applied to the rectangle's corners: x' = a x + c y + tx,
    /// y' = b x + d y + ty. Identity is [1, 0, 0, 1, 0, 0].
    pub to_root: [f32; 6],
    /// Output device pixels per logical pixel.
    pub scale: f32,
    pub opacity: f32,
    /// Corner radius in logical pixels; the content is clipped to the rounded rectangle when it is above 0.
    pub corner_radius: f32,
    /// Premultiplied colour, used when there is no texture (or the texture is missing).
    pub color: [f32; 4],
    /// Glass behind the node, over its rectangle.
    pub material: Option<Glass>,
    /// A shadow in device pixels (computed by the caller, `material::window_shadow`).
    pub shadow: Option<ShadowSpec>,
}

impl SceneNode {
    /// A solid node with no transform, scale 1, full opacity and no effects (a starting point for the builder).
    pub fn solid(rect: [f32; 4], color: [f32; 4]) -> SceneNode {
        SceneNode {
            texture: None,
            src: [0.0; 4],
            rect,
            to_root: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            scale: 1.0,
            opacity: 1.0,
            corner_radius: 0.0,
            color,
            material: None,
            shadow: None,
        }
    }
}

/// The device-pixel corners of a node's quad: top-left, top-right, bottom-left, bottom-right.
fn node_quad(n: &SceneNode) -> [[f32; 2]; 4] {
    let [x, y, w, h] = n.rect;
    let [a, b, c, d, tx, ty] = n.to_root;
    let map = |px: f32, py: f32| [(a * px + c * py + tx) * n.scale, (b * px + d * py + ty) * n.scale];
    [map(x, y), map(x + w, y), map(x, y + h), map(x + w, y + h)]
}

/// The axis-aligned bounds of corners, as `[x, y, w, h]`.
fn corners_bounds(c: &[[f32; 2]; 4]) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (c[0][0], c[0][1], c[0][0], c[0][1]);
    for p in &c[1..] {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// The rectangle a shadow reaches: its shape offset, grown by three sigmas on every side.
pub fn shadow_extent(s: &ShadowSpec) -> [f32; 4] {
    let reach = SHADOW_SIGMA_REACH * s.sigma;
    [
        s.rect[0] + s.offset[0] - reach,
        s.rect[1] + s.offset[1] - reach,
        s.rect[2] + 2.0 * reach,
        s.rect[3] + 2.0 * reach,
    ]
}

/// Everything an item covers on the output: its quad, its shadow and its material.
pub fn item_extent(item: &DrawItem) -> [f32; 4] {
    let mut e = corners_bounds(&item.quad);
    if let Some(s) = &item.shadow {
        e = union_f(e, shadow_extent(s));
    }
    if let Some(m) = &item.material {
        e = union_f(e, m.rect);
    }
    e
}

fn union_f(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Whether two float rectangles share some area.
fn overlaps_f(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// Builds the draw items of a stack (pure, host-tested). `nodes` are back to front. A node whose texture the
/// `texture` lookup cannot find draws its solid colour instead. Nodes with zero opacity, or with nothing visible
/// (no colour, texture, glass or shadow), are dropped, and so is every item whose extent misses the damage, so the
/// items are the work for this frame only. Output order is the input order.
pub fn build_items(nodes: &[SceneNode], damage: &DamageRects, texture: &dyn Fn(u64) -> Option<Texture>) -> Vec<DrawItem> {
    let damage_f: Vec<[f32; 4]> = damage.iter().map(|d| [d[0] as f32, d[1] as f32, d[2] as f32, d[3] as f32]).collect();
    let mut out = Vec::new();
    for n in nodes {
        if n.opacity <= 0.0 {
            continue;
        }
        let tex = n.texture.and_then(|k| texture(k));
        let color = if tex.is_some() { [0.0; 4] } else { n.color };
        if tex.is_none() && color[3] <= 0.0 && n.material.is_none() && n.shadow.is_none() {
            continue;
        }
        let quad = node_quad(n);
        let bb = corners_bounds(&quad);
        let clip = if n.corner_radius > 0.0 { [bb[0], bb[1], bb[2], bb[3], n.corner_radius * n.scale] } else { [0.0; 5] };
        let material = n.material.map(|g| MaterialRegion {
            rect: bb,
            radius: n.corner_radius * n.scale,
            blur: g.blur * n.scale,
            tint: g.tint,
            saturation: g.saturation,
        });
        let item = DrawItem { texture: tex, quad, src: n.src, opacity: n.opacity, color, clip, material, shadow: n.shadow };
        let ext = item_extent(&item);
        if damage_f.iter().any(|d| overlaps_f(ext, *d)) {
            out.push(item);
        }
    }
    out
}

/// The 128-byte push-constant block shared by `compose.vert` and `compose.frag`. Offsets (std430):
/// corners 0..32, src 32, color 48, clip 64, clip_radius 80, opacity 84, flags 88, saturation 92, out_size 96,
/// shadow_sigma 104, shadow_radius 108, shadow_rect 112.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Push {
    pub corners: [[f32; 2]; 4],
    pub src: [f32; 4],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    pub clip_radius: f32,
    pub opacity: f32,
    pub flags: u32,
    pub saturation: f32,
    pub out_size: [f32; 2],
    pub shadow_sigma: f32,
    pub shadow_radius: f32,
    pub shadow_rect: [f32; 4],
}

/// The push-constant size in bytes.
pub const PUSH_BYTES: usize = 128;

impl Push {
    /// The block with every field zero, except the saturation and the output size.
    fn base(out: (i32, i32)) -> Push {
        Push {
            corners: [[0.0; 2]; 4],
            src: [0.0; 4],
            color: [0.0; 4],
            clip: [0.0; 4],
            clip_radius: 0.0,
            opacity: 1.0,
            flags: 0,
            saturation: 1.0,
            out_size: [out.0 as f32, out.1 as f32],
            shadow_sigma: 0.0,
            shadow_radius: 0.0,
            shadow_rect: [0.0; 4],
        }
    }

    /// The little-endian bytes of the block.
    pub fn to_bytes(&self) -> [u8; PUSH_BYTES] {
        let mut b = [0u8; PUSH_BYTES];
        for (k, c) in self.corners.iter().enumerate() {
            put_f32(&mut b, k * 8, c);
        }
        put_f32(&mut b, 32, &self.src);
        put_f32(&mut b, 48, &self.color);
        put_f32(&mut b, 64, &self.clip);
        put_f32(&mut b, 80, &[self.clip_radius, self.opacity]);
        b[88..92].copy_from_slice(&self.flags.to_le_bytes());
        put_f32(&mut b, 92, &[self.saturation]);
        put_f32(&mut b, 96, &self.out_size);
        put_f32(&mut b, 104, &[self.shadow_sigma, self.shadow_radius]);
        put_f32(&mut b, 112, &self.shadow_rect);
        b
    }
}

fn put_f32(b: &mut [u8; PUSH_BYTES], off: usize, vals: &[f32]) {
    for (i, v) in vals.iter().enumerate() {
        b[off + 4 * i..off + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// The corners of an axis-aligned `[x, y, w, h]` rectangle in strip order.
fn rect_corners(r: [f32; 4]) -> [[f32; 2]; 4] {
    [[r[0], r[1]], [r[0] + r[2], r[1]], [r[0], r[1] + r[3]], [r[0] + r[2], r[1] + r[3]]]
}

/// The rounded clip of an item as the shader takes it, or zeros when disabled.
fn clip_of(c: [f32; 5]) -> ([f32; 4], f32) {
    ([c[0], c[1], c[2], c[3]], c[4])
}

/// The push block for an item's content (solid or textured). `tex_size` is the texture's size in texels when the
/// item is textured (the source rectangle is normalised by it).
pub fn content_push(item: &DrawItem, out: (i32, i32), tex_size: Option<(f32, f32)>) -> Push {
    let mut p = Push::base(out);
    p.corners = item.quad;
    p.opacity = item.opacity;
    p.color = item.color;
    let (clip, radius) = clip_of(item.clip);
    p.clip = clip;
    p.clip_radius = radius;
    if let Some((tw, th)) = tex_size {
        p.flags |= FLAG_TEXTURED;
        p.src = if item.src[2] <= 0.0 || item.src[3] <= 0.0 {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [item.src[0] / tw, item.src[1] / th, item.src[2] / tw, item.src[3] / th]
        };
    }
    p
}

/// The push block of a shadow: the quad is the shadow's extent, the shape is its rect moved by its offset.
pub fn shadow_push(item: &DrawItem, s: &ShadowSpec, out: (i32, i32)) -> Push {
    let mut p = Push::base(out);
    p.corners = rect_corners(shadow_extent(s));
    p.opacity = item.opacity;
    p.color = s.color;
    p.flags = FLAG_SHADOW;
    p.shadow_rect = [s.rect[0] + s.offset[0], s.rect[1] + s.offset[1], s.rect[2], s.rect[3]];
    p.shadow_radius = s.radius;
    p.shadow_sigma = s.sigma;
    p
}

/// The push block of glass: the quad is the material's rect, sampling the whole blurred backdrop (`MaterialPass`
/// returns a texture covering exactly the rect).
pub fn material_push(item: &DrawItem, m: &MaterialRegion, out: (i32, i32), _blur_size: (f32, f32)) -> Push {
    let mut p = Push::base(out);
    p.corners = rect_corners(m.rect);
    p.opacity = item.opacity;
    p.flags = FLAG_TEXTURED | FLAG_MATERIAL;
    p.src = [0.0, 0.0, 1.0, 1.0];
    p.color = m.tint;
    p.saturation = m.saturation;
    p.clip = m.rect;
    p.clip_radius = m.radius;
    p
}

/// The pipeline, a one-pixel white texture for solid draws, and the sampler the composition uses.
pub struct ComposePipelines {
    pub pipeline: RenderPipeline,
    pub dummy: Texture,
    pub sampler: Sampler,
}

impl ComposePipelines {
    /// Makes the composition pipeline for a target of `target_format`.
    pub fn new(device: &Device, target_format: PixelFormat) -> Result<ComposePipelines> {
        let pipeline = device.new_render_pipeline(&RenderPipelineDescriptor {
            vertex: "compose.vert",
            fragment: "compose.frag",
            target_format,
            blend: BlendMode::PremultipliedOver,
            vertex_layout: None,
            strip: true,
            textures: 1,
            samplers: 1,
            push_constant_bytes: PUSH_BYTES as u32,
        })?;
        let dummy = device.new_texture(&TextureDescriptor {
            width: 1,
            height: 1,
            format: PixelFormat::Bgra8Unorm,
            usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST,
            exportable: false,
            modifiers: Vec::new(),
        })?;
        dummy.replace_region(Region { x: 0, y: 0, width: 1, height: 1 }, &[255, 255, 255, 255], 4)?;
        let sampler = device.new_sampler(&SamplerDescriptor { filter: Filter::Linear })?;
        Ok(ComposePipelines { pipeline, dummy, sampler })
    }
}

/// Blurs the backdrop under a material, the way `material` does it (compute passes, recorded into `cb` while no
/// render pass is open).
pub trait MaterialPass {
    /// Returns the blurred backdrop of `region.rect` as a texture covering exactly that rectangle (at any
    /// resolution; `material::Blur` returns half resolution).
    fn blur(&mut self, cb: &mut CommandBuffer, backdrop: &Texture, region: &MaterialRegion) -> Result<Texture>;
}

/// One draw in the open render pass.
struct Draw {
    tex: Option<Texture>,
    push: Push,
}

/// Ends the current pass by drawing `pending` into a new render pass.
fn flush(cb: &mut CommandBuffer, pipes: &ComposePipelines, target: &Texture, load: &mut LoadAction, area: Region, pending: &mut Vec<Draw>) -> Result<()> {
    let mut enc = cb.render_encoder(&RenderPassDescriptor {
        target: target.clone(),
        load: *load,
        store: StoreAction::Store,
        render_area: Some(area),
    })?;
    enc.set_pipeline(&pipes.pipeline);
    enc.set_scissor(area);
    for d in pending.drain(..) {
        let tex = d.tex.as_ref().unwrap_or(&pipes.dummy);
        enc.set_texture(0, tex);
        enc.set_sampler(1, &pipes.sampler);
        enc.set_bytes(&d.push.to_bytes());
        enc.draw(4, 1, 0, 0);
    }
    enc.end();
    *load = LoadAction::Load;
    Ok(())
}

/// Records `items` into `target`, back to front, restricted to the damage's bounding box. `load` is what the target
/// holds on entry (`Load` keeps the buffer-age contents outside the damage; use `DontCare` only when the items
/// cover the whole target). Empty damage records nothing.
pub fn record(
    cb: &mut CommandBuffer,
    pipes: &ComposePipelines,
    target: &Texture,
    load: LoadAction,
    damage: &DamageRects,
    items: &[DrawItem],
    materials: &mut dyn MaterialPass,
) -> Result<()> {
    let Some(b) = bounding(damage) else { return Ok(()) };
    let out = (target.width() as i32, target.height() as i32);
    let area = Region { x: b[0].max(0) as u32, y: b[1].max(0) as u32, width: b[2].max(0) as u32, height: b[3].max(0) as u32 };
    let mut load = load;
    let mut pending: Vec<Draw> = Vec::new();
    for item in items {
        let tex_size = item.texture.as_ref().map(|t| (t.width() as f32, t.height() as f32));
        if let Some(s) = &item.shadow {
            pending.push(Draw { tex: None, push: shadow_push(item, s, out) });
        }
        if let Some(m) = &item.material {
            // The backdrop is what is composited so far, shadow included: close the pass, blur, then draw the glass.
            flush(cb, pipes, target, &mut load, area, &mut pending)?;
            let blurred = materials.blur(cb, target, m)?;
            let bsize = (blurred.width() as f32, blurred.height() as f32);
            pending.push(Draw { tex: Some(blurred), push: material_push(item, m, out, bsize) });
        }
        pending.push(Draw { tex: item.texture.clone(), push: content_push(item, out, tex_size) });
    }
    flush(cb, pipes, target, &mut load, area, &mut pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> [i32; 4] {
        [x, y, w, h]
    }

    #[test]
    fn damage_empty_stays_empty() {
        assert!(merge_damage(&[], (100, 100)).is_empty());
    }

    #[test]
    fn damage_outside_the_output_is_dropped() {
        assert!(merge_damage(&[r(-10, -10, 5, 5), r(100, 0, 10, 10), r(0, 0, 0, 10)], (100, 100)).is_empty());
    }

    #[test]
    fn damage_is_clipped_to_the_output() {
        assert_eq!(merge_damage(&[r(90, 90, 20, 20)], (100, 100)), vec![r(90, 90, 10, 10)]);
    }

    #[test]
    fn overlapping_damage_merges_into_the_union() {
        assert_eq!(merge_damage(&[r(0, 0, 10, 10), r(5, 5, 10, 10)], (100, 100)), vec![r(0, 0, 15, 15)]);
    }

    #[test]
    fn touching_damage_is_kept_apart() {
        let d = merge_damage(&[r(0, 0, 10, 10), r(10, 0, 10, 10)], (100, 100));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn a_rectangle_that_bridges_two_others_merges_all_three() {
        // (0..10) and (20..30) are apart; (8..22) overlaps both, so the three become one.
        let d = merge_damage(&[r(0, 0, 10, 10), r(20, 0, 10, 10), r(8, 0, 14, 10)], (100, 100));
        assert_eq!(d, vec![r(0, 0, 30, 10)]);
    }

    #[test]
    fn sixteen_disjoint_rectangles_are_kept() {
        let rects: Vec<[i32; 4]> = (0..16).map(|i| r(i * 10, 0, 5, 5)).collect();
        assert_eq!(merge_damage(&rects, (1000, 100)).len(), 16);
    }

    #[test]
    fn seventeen_disjoint_rectangles_collapse_to_the_bounding_box() {
        let rects: Vec<[i32; 4]> = (0..17).map(|i| r(i * 10, 0, 5, 5)).collect();
        // x from 0 to 160 + 5, y from 0 to 5
        assert_eq!(merge_damage(&rects, (1000, 100)), vec![r(0, 0, 165, 5)]);
    }

    #[test]
    fn push_block_offsets_match_the_shader() {
        let mut p = Push::base((2560, 1440));
        p.corners = [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0]];
        p.src = [0.25, 0.5, 0.75, 1.0];
        p.clip_radius = 17.0;
        p.opacity = 0.5;
        p.flags = FLAG_TEXTURED | FLAG_MATERIAL;
        p.saturation = 1.5;
        p.shadow_sigma = 24.0;
        p.shadow_radius = 13.0;
        p.shadow_rect = [9.0, 10.0, 11.0, 12.0];
        let b = p.to_bytes();
        assert_eq!(b.len(), 128);
        let f = |off: usize| f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
        assert_eq!(f(4), 2.0); // corner 0 y
        assert_eq!(f(24), 7.0); // corner 3 x
        assert_eq!(f(32), 0.25); // src x
        assert_eq!(f(80), 17.0); // clip radius
        assert_eq!(f(84), 0.5); // opacity
        assert_eq!(u32::from_le_bytes([b[88], b[89], b[90], b[91]]), 5);
        assert_eq!(f(92), 1.5); // saturation
        assert_eq!(f(96), 2560.0); // out width
        assert_eq!(f(100), 1440.0); // out height
        assert_eq!(f(104), 24.0); // sigma
        assert_eq!(f(108), 13.0); // shadow radius
        assert_eq!(f(112), 9.0); // shadow rect x
        assert_eq!(f(124), 12.0); // shadow rect h
    }

    #[test]
    fn solid_node_becomes_a_scaled_quad() {
        let mut n = SceneNode::solid([10.0, 20.0, 30.0, 40.0], [1.0, 0.0, 0.0, 1.0]);
        n.scale = 2.0;
        let items = build_items(&[n], &vec![r(0, 0, 1000, 1000)], &|_| None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].quad, [[20.0, 40.0], [80.0, 40.0], [20.0, 120.0], [80.0, 120.0]]);
        assert!(items[0].texture.is_none());
        assert_eq!(items[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(items[0].clip, [0.0; 5]);
    }

    #[test]
    fn items_outside_the_damage_are_culled() {
        let a = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4]);
        let b = SceneNode::solid([500.0, 500.0, 10.0, 10.0], [1.0; 4]);
        let items = build_items(&[a, b], &vec![r(0, 0, 100, 100)], &|_| None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].quad[0], [0.0, 0.0]);
    }

    #[test]
    fn transparent_and_empty_nodes_are_dropped_and_order_is_kept() {
        let mut hidden = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4]);
        hidden.opacity = 0.0;
        let empty = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [0.0; 4]);
        let mut first = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0, 0.0, 0.0, 1.0]);
        first.opacity = 0.5;
        let last = SceneNode::solid([5.0, 5.0, 10.0, 10.0], [0.0, 0.0, 1.0, 1.0]);
        let items = build_items(&[hidden, empty, first, last], &vec![r(0, 0, 100, 100)], &|_| None);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].opacity, 0.5);
        assert_eq!(items[1].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn rounded_clip_takes_the_quad_bounds_and_scaled_radius() {
        let mut n = SceneNode::solid([10.0, 10.0, 100.0, 50.0], [1.0; 4]);
        n.scale = 2.0;
        n.corner_radius = 13.0;
        let items = build_items(&[n], &vec![r(0, 0, 1000, 1000)], &|_| None);
        assert_eq!(items[0].clip, [20.0, 20.0, 200.0, 100.0, 26.0]);
    }

    #[test]
    fn a_missing_texture_falls_back_to_the_colour() {
        let mut n = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [0.0; 4]);
        n.texture = Some(7);
        // no colour and no texture: nothing to draw
        assert!(build_items(&[n.clone()], &vec![r(0, 0, 100, 100)], &|_| None).is_empty());
        n.color = [0.5, 0.0, 0.0, 0.5];
        let items = build_items(&[n], &vec![r(0, 0, 100, 100)], &|_| None);
        assert_eq!(items[0].color, [0.5, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn transform_moves_the_quad() {
        let mut n = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4]);
        n.to_root = [1.0, 0.0, 0.0, 1.0, 30.0, 40.0];
        let items = build_items(&[n], &vec![r(0, 0, 100, 100)], &|_| None);
        assert_eq!(items[0].quad[0], [30.0, 40.0]);
        assert_eq!(items[0].quad[3], [40.0, 50.0]);
    }

    #[test]
    fn shadow_extent_grows_by_three_sigma_around_the_offset_shape() {
        let s = ShadowSpec { rect: [100.0, 100.0, 200.0, 100.0], radius: 10.0, sigma: 24.0, offset: [0.0, 16.0], color: [0.0, 0.0, 0.0, 0.35] };
        // reach 72: x 28, y 100 + 16 - 72 = 44, w 200 + 144, h 100 + 144
        assert_eq!(shadow_extent(&s), [28.0, 44.0, 344.0, 244.0]);
    }

    #[test]
    fn item_extent_includes_shadow_and_material() {
        let item = DrawItem {
            texture: None,
            quad: [[10.0, 10.0], [20.0, 10.0], [10.0, 20.0], [20.0, 20.0]],
            src: [0.0; 4],
            opacity: 1.0,
            color: [1.0; 4],
            clip: [0.0; 5],
            material: Some(MaterialRegion { rect: [0.0, 0.0, 15.0, 30.0], radius: 0.0, blur: 4.0, tint: [0.0; 4], saturation: 1.0 }),
            shadow: None,
        };
        assert_eq!(item_extent(&item), [0.0, 0.0, 20.0, 30.0]);
    }

    #[test]
    fn shadow_push_expands_the_quad_and_moves_the_shape() {
        let s = ShadowSpec { rect: [100.0, 100.0, 200.0, 100.0], radius: 10.0, sigma: 12.0, offset: [0.0, 6.0], color: [0.0, 0.0, 0.0, 0.2] };
        let item = DrawItem { texture: None, quad: [[0.0; 2]; 4], src: [0.0; 4], opacity: 1.0, color: [0.0; 4], clip: [0.0; 5], material: None, shadow: Some(s) };
        let p = shadow_push(&item, &s, (1000, 1000));
        assert_eq!(p.flags, FLAG_SHADOW);
        assert_eq!(p.corners[0], [64.0, 70.0]); // x 100 - 36, y 100 + 6 - 36
        assert_eq!(p.shadow_rect, [100.0, 106.0, 200.0, 100.0]);
        assert_eq!(p.shadow_sigma, 12.0);
    }

    #[test]
    fn content_push_normalises_the_source_by_the_texture_size() {
        let item = DrawItem {
            texture: None,
            quad: [[1.0, 2.0], [3.0, 2.0], [1.0, 4.0], [3.0, 4.0]],
            src: [100.0, 50.0, 200.0, 100.0],
            opacity: 0.5,
            color: [0.0; 4],
            clip: [1.0, 2.0, 2.0, 2.0, 4.0],
            material: None,
            shadow: None,
        };
        let p = content_push(&item, (640, 480), Some((400.0, 200.0)));
        assert_eq!(p.flags, FLAG_TEXTURED);
        assert_eq!(p.src, [0.25, 0.25, 0.5, 0.5]);
        assert_eq!(p.clip, [1.0, 2.0, 2.0, 2.0]);
        assert_eq!(p.clip_radius, 4.0);
        assert_eq!(p.opacity, 0.5);
        // whole texture when the source has no size
        let whole = DrawItem { src: [0.0; 4], ..item.clone() };
        assert_eq!(content_push(&whole, (640, 480), Some((400.0, 200.0))).src, [0.0, 0.0, 1.0, 1.0]);
        // solid: no texture flag
        assert_eq!(content_push(&item, (640, 480), None).flags, 0);
    }

    #[test]
    fn material_push_samples_the_whole_region_sized_blur() {
        let m = MaterialRegion { rect: [100.0, 50.0, 200.0, 100.0], radius: 13.0, blur: 20.0, tint: [0.1, 0.1, 0.1, 0.4], saturation: 1.8 };
        let item = DrawItem { texture: None, quad: [[0.0; 2]; 4], src: [0.0; 4], opacity: 1.0, color: [0.0; 4], clip: [0.0; 5], material: Some(m), shadow: None };
        let p = material_push(&item, &m, (1000, 500), (1000.0, 500.0));
        assert_eq!(p.flags, FLAG_TEXTURED | FLAG_MATERIAL);
        assert_eq!(p.src, [0.0, 0.0, 1.0, 1.0], "the blur covers exactly the glass rect");
        assert_eq!(p.corners[3], [300.0, 150.0]);
        assert_eq!(p.clip, [100.0, 50.0, 200.0, 100.0]);
        assert_eq!(p.clip_radius, 13.0);
        assert_eq!(p.color, [0.1, 0.1, 0.1, 0.4]);
        assert_eq!(p.saturation, 1.8);
    }

    #[test]
    fn glass_node_becomes_a_material_in_device_pixels() {
        let mut n = SceneNode::solid([10.0, 10.0, 100.0, 40.0], [0.0; 4]);
        n.scale = 2.0;
        n.corner_radius = 9.0;
        n.material = Some(Glass { blur: 12.0, tint: [0.0, 0.0, 0.0, 0.2], saturation: 1.5 });
        let items = build_items(&[n], &vec![r(0, 0, 1000, 1000)], &|_| None);
        let m = items[0].material.unwrap();
        assert_eq!(m.rect, [20.0, 20.0, 200.0, 80.0]);
        assert_eq!(m.radius, 18.0);
        assert_eq!(m.blur, 24.0);
    }

    #[test]
    fn push_block_bytes_are_128() {
        assert_eq!(PUSH_BYTES, 128);
        assert_eq!(Push::base((1, 1)).to_bytes().len(), 128);
    }
}
