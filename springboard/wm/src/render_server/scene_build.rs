//! Scene building for the Vulkan render server (wave G2, `wm-scene`): the window stack of one output, back to front,
//! as `compose::SceneNode`s, plus per-output damage tracking with buffer age.
//!
//! Order, back to front:
//! 1. background and bottom layer-shell surfaces,
//! 2. windows in stacking order: each window's shadow (`material::window_shadow`, active or inactive), its
//!    `ext-background-effect` regions as glass nodes, then its surface tree (toplevel and subsurfaces, clipped to the
//!    window radius),
//! 3. the `bifrost_layer_v1` contexts, in context id order, each from `ContextStore::presentation` at the predicted
//!    presentation time (`layer_nodes` per presented layer),
//! 4. top and overlay layer-shell surfaces.
//! The cursor and the boot fade are not part of the window stack; the caller appends them.
//!
//! Buffers: a client dma-buf is imported once per `wl_buffer` through `Importer::import_dmabuf` (zero copy) and the
//! node names its key. shm buffers are not built here yet: reading shm bytes needs a safe copy helper (unsafe is kept
//! to `springboard/gpu`), so an shm surface produces no texture node until that helper exists (see the todo list in
//! `docs/graphics/g2-wm-scene.md`).
//!
//! Pure parts (host-tested): `layer_nodes` (a presented layer to nodes), `node_bounds` (device bounds of a node
//! including its shadow), `diff_damage` (what changed between two frames' stacks) and `DamageHistory` (buffer age).

use std::collections::VecDeque;

use smithay::{
    backend::renderer::utils::RendererSurfaceStateUserData,
    desktop::layer_map_for_output,
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{protocol::wl_buffer::WlBuffer, Resource},
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{with_surface_tree_downward, TraversalAction},
        dmabuf::get_dmabuf,
        seat::WaylandFocus,
        shell::wlr_layer::Layer as ShellLayer,
    },
};

use bifrost_layers::layer::{ContentsId, LayerContents, Material};
use bifrost_layers::tree::PresentedLayer;
use bifrost_layers::Color;

use crate::bifrost_material::effective_regions;
use crate::material_kind::{budgeted_blur, premultiply, MaterialSpec, Resolved};
use crate::render_server::compose::{self, Glass as GlassMat, SceneNode};
use crate::render_server::import::{dmabuf_to_gpu, Importer};
use crate::render_server::material::{device_region, window_shadow, ShadowSpec};
use crate::render_server::server::Server;
use crate::state::WmState;

/// Corner radius of toplevel windows, in logical px (CLAUDE.md design rules, 2026-10-09: window 17 pt).
pub const WINDOW_RADIUS: f32 = 17.0;

/// Blur of the glass behind an `ext-background-effect` region, in logical px.
pub const GLASS_BLUR: f32 = 20.0;

/// Tint over the glass (straight RGBA): a white veil at 25 %, like the shell's panels.
pub const GLASS_TINT: [f32; 4] = [1.0, 1.0, 1.0, 0.25];

/// The nodes of one material region of a window whose surface origin is `origin` (pure): one node per rectangle,
/// a blurred glass node when the material is active behind the window, else the opaque plate. The blur radius is
/// lowered to the frame-time budget, and the rounded outline applies when the region is a single rectangle.
pub fn material_nodes(spec: &MaterialSpec, origin: Point<i32, Logical>, scale: f32, window_active: bool, dark: bool) -> Vec<SceneNode> {
    let resolved = crate::material_kind::resolve(spec, window_active, dark);
    let radius = if spec.rects.len() == 1 { spec.mask_radius() } else { 0.0 };
    spec.rects
        .iter()
        .map(|r| {
            let rect = [(origin.x + r.loc.x) as f32, (origin.y + r.loc.y) as f32, r.size.w as f32, r.size.h as f32];
            let mut n = SceneNode::solid(rect, [0.0; 4]);
            n.scale = scale;
            n.corner_radius = radius;
            match resolved {
                Resolved::Glass { blur, saturation, tint, .. } => {
                    let blur = budgeted_blur(blur, (rect[2] * scale).ceil() as u32, (rect[3] * scale).ceil() as u32, scale);
                    n.material = Some(GlassMat { blur, tint: premultiply(tint), saturation });
                }
                Resolved::Plate(c) => n.color = premultiply(c),
            }
            n
        })
        .collect()
}

/// Premultiplies a straight colour.
fn premult(c: Color) -> [f32; 4] {
    [c.r * c.a, c.g * c.a, c.b * c.a, c.a]
}

/// The bounds of `rect` (x, y, w, h) after `to_root` (a, b, c, d, tx, ty) and `scale`, as `[x, y, w, h]` device px.
pub fn device_rect(rect: [f32; 4], to_root: [f32; 6], scale: f32) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let [a, b, c, d, tx, ty] = to_root;
    let map = |px: f32, py: f32| [(a * px + c * py + tx) * scale, (b * px + d * py + ty) * scale];
    let corners = [map(x, y), map(x + w, y), map(x, y + h), map(x + w, y + h)];
    let (mut x0, mut y0, mut x1, mut y1) = (corners[0][0], corners[0][1], corners[0][0], corners[0][1]);
    for p in &corners[1..] {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// The union of two `[x, y, w, h]` float rectangles.
fn union_f(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// The device pixels a node can touch: its quad, grown by its shadow when it has one.
pub fn node_bounds(n: &SceneNode) -> [i32; 4] {
    let mut r = device_rect(n.rect, n.to_root, n.scale);
    if let Some(s) = &n.shadow {
        r = union_f(r, compose::shadow_extent(s));
    }
    device_region(r)
}

/// A layer's contents from a presented layer (pure): shadow carrier, glass material, border, background, then the
/// bound contents. `bound` maps a contents id to the import key of its buffer. Layer values are in the layer's own
/// space; `to_root` and `scale` place them on the output.
///
/// The contents fill the bounds (`ContentsGravity::Resize`); other gravities are not mapped yet. A layer's rounded
/// corners clip its own nodes only when it `masks_to_bounds`; clips inherited from ancestors are not applied yet.
pub fn layer_nodes(pl: &PresentedLayer, bound: &dyn Fn(ContentsId) -> Option<u64>, scale: f32) -> Vec<SceneNode> {
    let l = &pl.layer;
    let b = l.bounds;
    let t = pl.to_root;
    let to_root = [t.a, t.b, t.c, t.d, t.tx, t.ty];
    let radius = if l.masks_to_bounds { l.corner_radius } else { 0.0 };
    let full = [b.x, b.y, b.w, b.h];
    let node = |rect: [f32; 4], color: [f32; 4], corner: f32| SceneNode {
        texture: None,
        src: [0.0; 4],
        rect,
        to_root,
        scale,
        opacity: pl.opacity,
        corner_radius: corner,
        color,
        material: None,
        shadow: None,
    };
    let mut out = Vec::new();

    // shadow: a transparent node that carries the shadow behind everything of the layer
    let alpha = (l.shadow.color.a * l.shadow.opacity).clamp(0.0, 1.0);
    if alpha > 0.0 {
        let mut n = node(full, [0.0; 4], radius);
        n.shadow = Some(ShadowSpec {
            rect: device_rect(full, to_root, scale),
            radius: radius * scale,
            sigma: l.shadow.radius * 0.5 * scale,
            offset: [l.shadow.offset.x * scale, l.shadow.offset.y * scale],
            color: [l.shadow.color.r * alpha, l.shadow.color.g * alpha, l.shadow.color.b * alpha, alpha],
        });
        out.push(n);
    }

    // glass: a transparent node whose material blurs what is behind the layer
    if let Material::Glass { blur, tint, saturation } = l.material {
        let mut n = node(full, [0.0; 4], radius);
        n.material = Some(GlassMat { blur, tint: premult(tint), saturation });
        out.push(n);
    }

    // border: the outer rounded rect in the border colour, the background inset by its width
    let bw = l.border.width.max(0.0).min(b.w * 0.5).min(b.h * 0.5);
    let inset = if bw > 0.0 && l.border.color.a > 0.0 {
        out.push(node(full, premult(l.border.color), radius));
        [b.x + bw, b.y + bw, b.w - 2.0 * bw, b.h - 2.0 * bw]
    } else {
        full
    };
    let inner_radius = (radius - bw).max(0.0);

    if l.background_color.a > 0.0 {
        out.push(node(inset, premult(l.background_color), if bw > 0.0 { inner_radius } else { radius }));
    }

    if let LayerContents::Buffer(id) = l.contents {
        if let Some(key) = bound(id) {
            let mut n = node(full, [0.0; 4], radius);
            n.texture = Some(key);
            out.push(n);
        }
    }
    out
}

/// Stable key of a `wl_buffer` for the importer's cache: one key per buffer object.
pub(crate) fn buffer_key(buffer: &WlBuffer) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    buffer.id().hash(&mut h);
    h.finish()
}

/// Appends the nodes of a surface tree at `origin` (output logical px): one node per surface with a dma-buf buffer,
/// subsurfaces placed by their offsets. Surfaces without a dma-buf (shm, see the module note) add nothing.
/// `corner` clips every node of the tree (windows pass `WINDOW_RADIUS`, layer-shell surfaces 0).
fn surface_nodes(importer: &mut Importer, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, origin: Point<i32, Logical>, scale: f32, corner: f32, out: &mut Vec<SceneNode>) {
    with_surface_tree_downward(
        surface,
        origin,
        |_, states, loc| {
            let mut loc = *loc;
            let view = states
                .data_map
                .get::<RendererSurfaceStateUserData>()
                .and_then(|d| d.lock().unwrap().view());
            match view {
                Some(view) => {
                    loc += view.offset;
                    TraversalAction::DoChildren(loc)
                }
                None => TraversalAction::SkipChildren,
            }
        },
        |_, states, loc| {
            let mut loc = *loc;
            let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() else { return };
            let st = data.lock().unwrap();
            let (Some(view), Some(size), Some(buffer)) = (st.view(), st.surface_size(), st.buffer()) else { return };
            loc += view.offset;
            let key = buffer_key(buffer);
            if importer.texture(key).is_none() {
                let Ok(dmabuf) = get_dmabuf(buffer) else { return };
                match dmabuf_to_gpu(dmabuf).and_then(|gpu| importer.import_dmabuf(key, &gpu)) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("bifrost-wm: scene: dma-buf import: {e}");
                        return;
                    }
                }
            }
            out.push(SceneNode {
                texture: Some(key),
                src: [0.0; 4],
                rect: [loc.x as f32, loc.y as f32, size.w as f32, size.h as f32],
                to_root: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                scale,
                opacity: 1.0,
                corner_radius: corner,
                color: [0.0; 4],
                material: None,
                shadow: None,
            });
        },
        |_, _, _| true,
    );
}

/// The surfaces of one layer-shell layer, bottom first (the map lists them top first).
fn layer_surfaces(importer: &mut Importer, map: &smithay::desktop::LayerMap, layer: ShellLayer, scale: f32, out: &mut Vec<SceneNode>) {
    let mut ls: Vec<_> = map.layers_on(layer).collect();
    ls.reverse();
    for l in ls {
        let Some(geo) = map.layer_geometry(l) else { continue };
        surface_nodes(importer, l.wl_surface(), geo.loc, scale, 0.0, out);
    }
}

/// The scene of one output at the predicted presentation time `t` (tree seconds), back to front. A held session lock
/// draws the lock surface alone.
pub fn build(state: &WmState, output: &Output, t: f64, server: &mut Server) -> Vec<SceneNode> {
    let scale = output.current_scale().fractional_scale() as f32;
    let Server { importer, layers, .. } = server;
    let mut out = Vec::new();
    if state.locked {
        if let Some(lock) = &state.lock_surface {
            surface_nodes(importer, lock.wl_surface(), Point::from((0, 0)), scale, 0.0, &mut out);
        }
        return out;
    }

    let map = layer_map_for_output(output);
    layer_surfaces(importer, &map, ShellLayer::Background, scale, &mut out);
    layer_surfaces(importer, &map, ShellLayer::Bottom, scale, &mut out);
    drop(map);

    for w in state.space.elements() {
        let Some(loc) = state.space.element_location(w) else { continue };
        // Wayland toplevels and X11 windows alike (WM-3)
        let Some(wl) = w.wl_surface().map(|s| s.into_owned()) else { continue };
        let geo = w.geometry();
        let origin = loc - geo.loc;
        let active = match w.toplevel() {
            Some(t) => t.current_state().states.contains(xdg_toplevel::State::Activated),
            None => crate::state::WmState::known_id(w).is_some() && crate::state::WmState::known_id(w) == state.wm.focused(),
        };

        // the shadow of the window frame, behind everything of it
        let frame = [loc.x as f32, loc.y as f32, geo.size.w as f32, geo.size.h as f32];
        let shadow_rect = [frame[0] * scale, frame[1] * scale, frame[2] * scale, frame[3] * scale];
        let mut shadow = SceneNode::solid(frame, [0.0; 4]);
        shadow.corner_radius = WINDOW_RADIUS;
        shadow.scale = scale;
        shadow.shadow = Some(window_shadow(shadow_rect, WINDOW_RADIUS * scale, active, scale));
        out.push(shadow);

        // materials behind the regions the window asks for (a kind per region, or the plain blur)
        for spec in effective_regions(&wl) {
            out.extend(material_nodes(&spec, origin, scale, active, state.material_dark));
        }

        surface_nodes(importer, &wl, origin, scale, WINDOW_RADIUS, &mut out);
    }

    // layer trees: each context's presented layers, in context id order
    let mut ids: Vec<u32> = layers.contexts.keys().copied().collect();
    ids.sort_unstable();
    for ctx in ids {
        let bound = |c: ContentsId| layers.binding(ctx, c);
        for pl in layers.presentation(ctx, t) {
            out.extend(layer_nodes(&pl, &bound, scale));
        }
    }

    let map = layer_map_for_output(output);
    layer_surfaces(importer, &map, ShellLayer::Top, scale, &mut out);
    layer_surfaces(importer, &map, ShellLayer::Overlay, scale, &mut out);
    out
}

/// The damage between two frames' stacks (pure): every node that is not the same at its position in both frames
/// contributes its old and its new device bounds. A node that appeared or vanished contributes its one bounds.
pub fn diff_damage(prev: &[SceneNode], cur: &[SceneNode]) -> Vec<[i32; 4]> {
    let mut out = Vec::new();
    for i in 0..prev.len().max(cur.len()) {
        match (prev.get(i), cur.get(i)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                out.push(node_bounds(a));
                out.push(node_bounds(b));
            }
            (Some(a), None) => out.push(node_bounds(a)),
            (None, Some(b)) => out.push(node_bounds(b)),
            (None, None) => {}
        }
    }
    out
}

/// Per-output damage history for buffer age (pure). It keeps the changed rectangles of the last frames, most recent
/// first, and tells the frame how much of the target must be redrawn: the changes of every frame since the target
/// last held the right content.
#[derive(Debug, Default)]
pub struct DamageHistory {
    past: VecDeque<Vec<[i32; 4]>>,
    capacity: usize,
}

impl DamageHistory {
    /// A history that keeps `capacity` frames (the swapchain depth is 3, so 4 is enough).
    pub fn new(capacity: usize) -> DamageHistory {
        DamageHistory { past: VecDeque::new(), capacity: capacity.max(1) }
    }

    /// The damage to draw this frame. `age` is the target's buffer age (0 when unknown, 1 when it holds the previous
    /// frame); `changed` is this frame's own damage, `output` the output size. Without a usable age the whole output
    /// is damaged. The frame's changes are recorded for the following frames.
    pub fn frame_damage(&mut self, age: u32, changed: Vec<[i32; 4]>, output: (i32, i32)) -> Vec<[i32; 4]> {
        let full = [0, 0, output.0, output.1];
        let usable = age >= 1 && (age as usize - 1) <= self.past.len();
        let result = if usable {
            let mut all = changed.clone();
            for frame in self.past.iter().take(age as usize - 1) {
                all.extend_from_slice(frame);
            }
            compose::merge_damage(&all, output)
        } else {
            vec![full]
        };
        self.past.push_front(changed);
        self.past.truncate(self.capacity);
        result
    }

    /// Forgets the history (the output changed size or mode, or its targets were reallocated).
    pub fn reset(&mut self) {
        self.past.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_server::material::SATURATION;
    use bifrost_layers::layer::{Border, Shadow};
    use bifrost_layers::{Affine, Layer, LayerId, Point as LPoint, Rect};

    fn color(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    /// A presented layer at `(x, y)` of its superlayer with a scale-free translation and the given layer values.
    fn presented(layer: Layer, to_root: Affine, opacity: f32) -> PresentedLayer {
        PresentedLayer { layer, to_root, opacity, clip: None }
    }

    fn layer(w: f32, h: f32) -> Layer {
        let mut l = Layer::new(LayerId(1));
        l.bounds = Rect::new(0.0, 0.0, w, h);
        l
    }

    fn none(_: ContentsId) -> Option<u64> {
        None
    }

    #[test]
    fn background_only_layer_is_one_solid_node() {
        let mut l = layer(100.0, 50.0);
        l.background_color = color(1.0, 0.0, 0.0, 1.0);
        let t = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 10.0, ty: 20.0 };
        let nodes = layer_nodes(&presented(l, t, 1.0), &none, 2.0);
        assert_eq!(nodes.len(), 1);
        let n = &nodes[0];
        assert_eq!(n.rect, [0.0, 0.0, 100.0, 50.0]);
        assert_eq!(n.to_root, [1.0, 0.0, 0.0, 1.0, 10.0, 20.0]);
        assert_eq!(n.scale, 2.0);
        assert_eq!(n.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(n.texture, None);
        assert_eq!(n.corner_radius, 0.0);
    }

    #[test]
    fn border_is_an_outer_node_and_the_background_is_inset() {
        let mut l = layer(100.0, 50.0);
        l.masks_to_bounds = true;
        l.corner_radius = 10.0;
        l.border = Border { width: 2.0, color: color(0.0, 0.0, 1.0, 1.0) };
        l.background_color = color(1.0, 1.0, 1.0, 1.0);
        let t = Affine::IDENTITY;
        let nodes = layer_nodes(&presented(l, t, 1.0), &none, 1.0);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].rect, [0.0, 0.0, 100.0, 50.0]);
        assert_eq!(nodes[0].color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(nodes[0].corner_radius, 10.0);
        assert_eq!(nodes[1].rect, [2.0, 2.0, 96.0, 46.0]);
        assert_eq!(nodes[1].color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(nodes[1].corner_radius, 8.0);
    }

    #[test]
    fn shadow_is_carried_by_a_transparent_node_with_premultiplied_colour() {
        let mut l = layer(100.0, 50.0);
        l.shadow = Shadow { color: color(0.0, 0.0, 0.0, 1.0), opacity: 0.5, radius: 20.0, offset: LPoint { x: 0.0, y: 4.0 } };
        let nodes = layer_nodes(&presented(l, Affine::IDENTITY, 1.0), &none, 1.0);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].color, [0.0; 4]);
        let s = nodes[0].shadow.expect("shadow");
        assert_eq!(s.rect, [0.0, 0.0, 100.0, 50.0]);
        assert_eq!(s.sigma, 10.0);
        assert_eq!(s.offset, [0.0, 4.0]);
        assert_eq!(s.color, [0.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn bound_contents_become_a_textured_node_and_unbound_contents_nothing() {
        let mut l = layer(64.0, 64.0);
        l.contents = LayerContents::Buffer(ContentsId(7));
        let bound = |c: ContentsId| if c == ContentsId(7) { Some(42) } else { None };
        let nodes = layer_nodes(&presented(l.clone(), Affine::IDENTITY, 1.0), &bound, 1.0);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].texture, Some(42));
        assert_eq!(nodes[0].rect, [0.0, 0.0, 64.0, 64.0]);
        assert!(layer_nodes(&presented(l, Affine::IDENTITY, 1.0), &none, 1.0).is_empty());
    }

    #[test]
    fn glass_material_is_a_transparent_node_with_a_premultiplied_tint() {
        let mut l = layer(10.0, 10.0);
        l.material = Material::Glass { blur: 20.0, tint: color(1.0, 1.0, 1.0, 0.25), saturation: SATURATION };
        let nodes = layer_nodes(&presented(l, Affine::IDENTITY, 1.0), &none, 2.0);
        assert_eq!(nodes.len(), 1);
        let g = nodes[0].material.expect("material");
        assert_eq!(g.blur, 20.0);
        assert_eq!(g.tint, [0.25, 0.25, 0.25, 0.25]);
        assert_eq!(g.saturation, SATURATION);
    }

    #[test]
    fn layer_opacity_is_carried_by_every_node() {
        let mut l = layer(10.0, 10.0);
        l.background_color = color(0.0, 1.0, 0.0, 1.0);
        let nodes = layer_nodes(&presented(l, Affine::IDENTITY, 0.5), &none, 1.0);
        assert_eq!(nodes[0].opacity, 0.5);
    }

    #[test]
    fn node_bounds_follow_the_transform_and_the_scale() {
        let n = SceneNode { to_root: [1.0, 0.0, 0.0, 1.0, 5.0, 0.0], scale: 2.0, ..SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4]) };
        assert_eq!(node_bounds(&n), [10, 0, 20, 20]);
    }

    #[test]
    fn node_bounds_grow_by_three_sigmas_for_a_shadow() {
        let mut n = SceneNode::solid([0.0, 0.0, 10.0, 10.0], [0.0; 4]);
        n.shadow = Some(ShadowSpec { rect: [0.0, 0.0, 10.0, 10.0], radius: 0.0, sigma: 10.0, offset: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.5] });
        assert_eq!(node_bounds(&n), [-30, -30, 70, 70]);
    }

    #[test]
    fn identical_stacks_have_no_damage() {
        let a = vec![SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4])];
        assert!(diff_damage(&a, &a.clone()).is_empty());
    }

    #[test]
    fn a_moved_node_damages_its_old_and_new_place() {
        let a = vec![SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4])];
        let b = vec![SceneNode::solid([20.0, 0.0, 10.0, 10.0], [1.0; 4])];
        assert_eq!(diff_damage(&a, &b), vec![[0, 0, 10, 10], [20, 0, 10, 10]]);
    }

    #[test]
    fn an_added_or_removed_node_damages_its_own_place() {
        let a = vec![SceneNode::solid([5.0, 5.0, 5.0, 5.0], [1.0; 4])];
        assert_eq!(diff_damage(&[], &a), vec![[5, 5, 5, 5]]);
        assert_eq!(diff_damage(&a, &[]), vec![[5, 5, 5, 5]]);
    }

    #[test]
    fn buffer_age_zero_redraws_the_whole_output() {
        let mut h = DamageHistory::new(4);
        assert_eq!(h.frame_damage(0, vec![[1, 1, 2, 2]], (100, 80)), vec![[0, 0, 100, 80]]);
    }

    #[test]
    fn buffer_age_one_redraws_only_this_frames_changes() {
        let mut h = DamageHistory::new(4);
        h.frame_damage(0, vec![], (100, 80));
        assert_eq!(h.frame_damage(1, vec![[1, 1, 2, 2]], (100, 80)), vec![[1, 1, 2, 2]]);
    }

    #[test]
    fn buffer_age_two_adds_the_previous_frames_changes() {
        let mut h = DamageHistory::new(4);
        h.frame_damage(0, vec![[10, 10, 5, 5]], (100, 80));
        // the target holds the frame before the previous one: both frames' changes
        assert_eq!(h.frame_damage(2, vec![[0, 0, 4, 4]], (100, 80)), vec![[0, 0, 4, 4], [10, 10, 5, 5]]);
    }

    #[test]
    fn an_age_beyond_the_history_redraws_the_whole_output() {
        let mut h = DamageHistory::new(4);
        h.frame_damage(0, vec![[1, 1, 1, 1]], (100, 80));
        assert_eq!(h.frame_damage(3, vec![], (100, 80)), vec![[0, 0, 100, 80]]);
    }

    #[test]
    fn the_history_keeps_only_its_capacity() {
        let mut h = DamageHistory::new(2);
        for i in 0..5 {
            h.frame_damage(1, vec![[i, 0, 1, 1]], (100, 80));
        }
        assert_eq!(h.past.len(), 2);
        // age 3 needs two past frames plus this one: fine; age 4 needs three past frames: whole output
        assert_eq!(h.frame_damage(4, vec![], (100, 80)), vec![[0, 0, 100, 80]]);
    }
}
