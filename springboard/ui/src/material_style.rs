//! Declarative materials and Liquid Glass for the view layer (`Material`, `View.background(_:in:)`,
//! `glassEffect(_:in:)`, `GlassEffectContainer`, `glassEffectID`, `glassEffectUnion`,
//! `backgroundExtensionEffect`, `.glass` and `.glassProminent` button styles).
//!
//! Mapping onto the shared model (`StyleRequest::to_material_request`): a declarative UI `Material` becomes a
//! `visual_effect::MaterialKind` plus `strength` (its `level`): ultraThin -> UnderWindowBackground, thin ->
//! ContentBackground, regular -> Popover, thick -> Sheet, ultraThick -> HudWindow, bar -> HeaderView. Glass becomes
//! kind Popover with `glass` set (the window server draws GlassRegular/GlassClear). The mask is a rounded rectangle
//! of the four radii.
//!
//! Views are imperative in this SDK, so each modifier is a builder that yields [`StyleRequest`]s (convertible to the shared `visual_effect::MaterialRequest` with `to_material_request`): one per
//! shape (rect, corner radii or capsule) for the window server. [`Material`] is the minimal enum compatible with
//! the shared material request; `Material::level` is the blur strength the compositor maps onto its material
//! families (ultraThin 0 .. ultraThick 4, bar is the chrome material). Everything here is pure data and unit tested.
use crate::anim::Animation;
use crate::visual_effect::{Corners, GlassKind, Mask, MaterialKind, MaterialRequest};
use crate::Rect;
use std::collections::HashMap;

/// declarative UI `Material`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    UltraThin,
    Thin,
    Regular,
    Thick,
    UltraThick,
    Bar,
}

impl Material {
    /// Blur strength rank, thin to thick; `Bar` sits at `Regular`'s strength with the chrome tint.
    pub fn level(self) -> u8 {
        match self {
            Material::UltraThin => 0,
            Material::Thin => 1,
            Material::Regular | Material::Bar => 2,
            Material::Thick => 3,
            Material::UltraThick => 4,
        }
    }
}

/// The outline a material or glass fills (`Shape`, `RoundedRectangle`, `Capsule`, `UnevenRoundedRectangle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatShape {
    Rect,
    /// Uniform corner radius, clamped to half the short side.
    Rounded(f32),
    /// Fully rounded ends (`Capsule`); a square is a circle.
    Capsule,
    /// top-left, top-right, bottom-right, bottom-left.
    Corners([f32; 4]),
}

impl MatShape {
    /// The four corner radii for `r`, resolved against the rectangle.
    pub fn radii(self, r: Rect) -> [f32; 4] {
        let max = r.w.min(r.h).max(0.0) / 2.0;
        match self {
            MatShape::Rect => [0.0; 4],
            MatShape::Rounded(v) => [v.clamp(0.0, max); 4],
            MatShape::Capsule => [max; 4],
            MatShape::Corners(c) => c.map(|v| v.clamp(0.0, max)),
        }
    }
}

/// `Glass.regular` / `Glass.clear`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassVariant {
    Regular,
    Clear,
}

/// `Glass`: the variant plus `.tint(_:)` and `.interactive()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub variant: GlassVariant,
    pub tint: Option<[u8; 4]>,
    pub interactive: bool,
}

impl Glass {
    pub const REGULAR: Glass = Glass { variant: GlassVariant::Regular, tint: None, interactive: false };
    pub const CLEAR: Glass = Glass { variant: GlassVariant::Clear, tint: None, interactive: false };
    pub fn tint(mut self, rgba: [u8; 4]) -> Glass {
        self.tint = Some(rgba);
        self
    }
    pub fn interactive(mut self) -> Glass {
        self.interactive = true;
        self
    }
}

/// What the window server paints behind a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Blur(Material),
    Glass(Glass),
}

/// One material request: the shape the window server fills. `group` is set for glass that merged in a container.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRequest {
    pub rect: Rect,
    pub radii: [f32; 4],
    pub fill: Fill,
    pub group: Option<u32>,
}

impl Material {
    /// The shared `MaterialKind` this level maps onto (see the module docs).
    pub fn model_kind(self) -> MaterialKind {
        match self {
            Material::UltraThin => MaterialKind::UnderWindowBackground,
            Material::Thin => MaterialKind::ContentBackground,
            Material::Regular => MaterialKind::Popover,
            Material::Thick => MaterialKind::Sheet,
            Material::UltraThick => MaterialKind::HudWindow,
            Material::Bar => MaterialKind::HeaderView,
        }
    }
}

impl StyleRequest {
    /// The shared window-server request: behind-window, follows-active, rounded-rect mask from `radii`.
    pub fn to_material_request(&self) -> MaterialRequest {
        let [tl, tr, br, bl] = self.radii;
        let req = MaterialRequest::new(MaterialKind::Popover, self.rect, 0.0).mask(Mask::RoundedRect(Corners { tl, tr, br, bl }));
        match self.fill {
            Fill::Blur(m) => MaterialRequest { kind: m.model_kind(), ..req }.strength(m.level()),
            Fill::Glass(g) => req.glass(match g.variant {
                GlassVariant::Regular => GlassKind::Regular,
                GlassVariant::Clear => GlassKind::Clear,
            }),
        }
    }
}

/// `.background(material, in: shape)`.
pub fn background(material: Material, shape: MatShape, rect: Rect) -> StyleRequest {
    StyleRequest { rect, radii: shape.radii(rect), fill: Fill::Blur(material), group: None }
}

/// `.glassEffect(glass, in: shape)`.
pub fn glass_effect(glass: Glass, shape: MatShape, rect: Rect) -> StyleRequest {
    StyleRequest { rect, radii: shape.radii(rect), fill: Fill::Glass(glass), group: None }
}

/// One glass view inside a container.
#[derive(Clone, Debug)]
pub struct GlassItem {
    /// `glassEffectID`: names the view across states so it morphs instead of cutting.
    pub id: u32,
    pub rect: Rect,
    pub shape: MatShape,
    pub glass: Glass,
    /// `glassEffectUnion(id:)`: views with the same union id always merge, near or far.
    pub union: Option<u32>,
}

/// Smallest gap between two rectangles (0 when they touch or overlap).
fn gap(a: Rect, b: Rect) -> f32 {
    let dx = (b.x - (a.x + a.w)).max(a.x - (b.x + b.w)).max(0.0);
    let dy = (b.y - (a.y + a.h)).max(a.y - (b.y + b.h)).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

fn find(p: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while p[r] != r {
        r = p[r];
    }
    let mut c = i;
    while p[c] != r {
        let nx = p[c];
        p[c] = r;
        c = nx;
    }
    r
}

/// `GlassEffectContainer(spacing:)`.
#[derive(Clone, Debug)]
pub struct GlassContainer {
    pub spacing: f32,
    pub items: Vec<GlassItem>,
}

impl GlassContainer {
    pub fn new(spacing: f32) -> Self {
        GlassContainer { spacing: spacing.max(0.0), items: Vec::new() }
    }
    pub fn add(&mut self, item: GlassItem) -> &mut Self {
        self.items.push(item);
        self
    }

    /// Merge groups as indices into `items`, ordered by first member. Two shapes merge when they are within
    /// `spacing` of each other, or share a union id; merging is transitive. Clear and regular glass do not
    /// merge by proximity (they sample differently); a shared union id still merges them.
    pub fn groups(&self) -> Vec<Vec<usize>> {
        let n = self.items.len();
        let mut parent: Vec<usize> = (0..n).collect();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.items[i], &self.items[j]);
                let union = a.union.is_some() && a.union == b.union;
                let near = a.glass.variant == b.glass.variant && gap(a.rect, b.rect) <= self.spacing;
                if union || near {
                    let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                    if ra != rb {
                        parent[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
        }
        let mut out: Vec<Vec<usize>> = Vec::new();
        let mut slot: HashMap<usize, usize> = HashMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            let s = *slot.entry(r).or_insert_with(|| {
                out.push(Vec::new());
                out.len() - 1
            });
            out[s].push(i);
        }
        out
    }

    /// One request per shape; shapes in a multi-member group carry the group number (the first member's `id`)
    /// so the window server blends their edges into one surface.
    pub fn requests(&self) -> Vec<StyleRequest> {
        let mut out = Vec::new();
        for g in self.groups() {
            let gid = (g.len() > 1).then(|| self.items[g[0]].id);
            for &i in &g {
                let it = &self.items[i];
                let mut r = glass_effect(it.glass, it.shape, it.rect);
                r.group = gid;
                out.push(r);
            }
        }
        out
    }
}

/// A shape in motion between two states of one `glassEffectID`.
#[derive(Clone, Copy, Debug)]
pub struct Morph {
    x: Animation,
    y: Animation,
    w: Animation,
    h: Animation,
    r: Animation,
}

/// Radius used for a morph's interpolation: the largest corner (uniform shapes are the common case).
fn radius(shape: MatShape, rect: Rect) -> f32 {
    shape.radii(rect).iter().cloned().fold(0.0, f32::max)
}

impl Morph {
    /// The glass morph spring, matching the shell's panel-reveal feel (response 0.35 s, damping 0.8).
    pub const DAMPING: f32 = 0.8;
    pub const RESPONSE: f32 = 0.35;

    pub fn new(from: (Rect, MatShape), to: (Rect, MatShape), now: f64) -> Morph {
        let mk = |a: f32, b: f32| Animation::spring(a, b, now, Self::DAMPING, Self::RESPONSE);
        Morph {
            x: mk(from.0.x, to.0.x),
            y: mk(from.0.y, to.0.y),
            w: mk(from.0.w, to.0.w),
            h: mk(from.0.h, to.0.h),
            r: mk(radius(from.1, from.0), radius(to.1, to.0)),
        }
    }
    /// Geometry at `now`: the rectangle and the uniform corner radius (never below 0 when a spring overshoots).
    pub fn at(&self, now: f64) -> (Rect, f32) {
        let rect = Rect { x: self.x.value(now), y: self.y.value(now), w: self.w.value(now).max(0.0), h: self.h.value(now).max(0.0) };
        (rect, self.r.value(now).max(0.0))
    }
    pub fn is_done(&self, now: f64) -> bool {
        self.x.is_done(now) && self.y.is_done(now) && self.w.is_done(now) && self.h.is_done(now) && self.r.is_done(now)
    }
    /// Interrupted morphs continue from the current geometry to a new target without a jump.
    pub fn retarget(&self, now: f64, to: (Rect, MatShape)) -> Morph {
        let (rect, r) = self.at(now);
        Morph::new((rect, MatShape::Rounded(r)), to, now)
    }
}

/// Tracks `glassEffectID`s between frames: an id seen before with a new geometry starts a [`Morph`].
#[derive(Default)]
pub struct GlassIds {
    last: HashMap<u32, (Rect, MatShape)>,
    morphs: HashMap<u32, Morph>,
}

impl GlassIds {
    /// Reports this frame's items and returns (id, rect, radius) to draw each at, `now` being the frame time.
    /// Ids that disappeared are forgotten (their removal is a plain fade, done by the caller).
    pub fn frame(&mut self, items: &[GlassItem], now: f64) -> Vec<(u32, Rect, f32)> {
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        for it in items {
            let target = (it.rect, it.shape);
            if let Some(prev) = self.last.get(&it.id) {
                if prev.0 != it.rect || prev.1 != it.shape {
                    let m = match self.morphs.get(&it.id) {
                        Some(m) => m.retarget(now, target),
                        None => Morph::new(*prev, target, now),
                    };
                    self.morphs.insert(it.id, m);
                }
            }
            match self.morphs.get(&it.id) {
                Some(m) if !m.is_done(now) => {
                    let (r, rad) = m.at(now);
                    out.push((it.id, r, rad));
                }
                _ => {
                    self.morphs.remove(&it.id);
                    out.push((it.id, it.rect, radius(it.shape, it.rect)));
                }
            }
            seen.insert(it.id, target);
        }
        self.morphs.retain(|k, _| seen.contains_key(k));
        self.last = seen;
        out
    }
}

/// An edge of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Leading,
    Trailing,
}

/// `backgroundExtensionEffect()`: the strips outside `content` (inside `bounds`) that mirror and blur the
/// content's edge so it appears to continue under a sidebar or toolbar. Returns (edge, strip) per non-empty strip.
pub fn background_extension(content: Rect, bounds: Rect) -> Vec<(Edge, Rect)> {
    let mut out = Vec::new();
    let (cr, br) = (content.x + content.w, bounds.x + bounds.w);
    let (cb, bb) = (content.y + content.h, bounds.y + bounds.h);
    if content.y > bounds.y {
        out.push((Edge::Top, Rect { x: content.x, y: bounds.y, w: content.w, h: content.y - bounds.y }));
    }
    if bb > cb {
        out.push((Edge::Bottom, Rect { x: content.x, y: cb, w: content.w, h: bb - cb }));
    }
    if content.x > bounds.x {
        out.push((Edge::Leading, Rect { x: bounds.x, y: content.y, w: content.x - bounds.x, h: content.h }));
    }
    if br > cr {
        out.push((Edge::Trailing, Rect { x: cr, y: content.y, w: br - cr, h: content.h }));
    }
    out
}

/// `.glass` and `.glassProminent` button styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassButtonStyle {
    Glass,
    GlassProminent,
}

impl GlassButtonStyle {
    /// The request for a button at `rect`: capsule, interactive; prominent is tinted with the accent.
    pub fn request(self, rect: Rect, accent: [u8; 4]) -> StyleRequest {
        let g = match self {
            GlassButtonStyle::Glass => Glass::REGULAR.interactive(),
            GlassButtonStyle::GlassProminent => Glass::REGULAR.tint(accent).interactive(),
        };
        glass_effect(g, MatShape::Capsule, rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    fn item(id: u32, rect: Rect) -> GlassItem {
        GlassItem { id, rect, shape: MatShape::Capsule, glass: Glass::REGULAR, union: None }
    }

    #[test]
    fn near_shapes_merge_transitively() {
        let mut c = GlassContainer::new(10.0);
        c.add(item(1, r(0.0, 0.0, 40.0, 40.0))).add(item(2, r(48.0, 0.0, 40.0, 40.0))).add(item(3, r(96.0, 0.0, 40.0, 40.0)));
        c.add(item(4, r(300.0, 0.0, 40.0, 40.0)));
        assert_eq!(c.groups(), vec![vec![0, 1, 2], vec![3]]);
        let reqs = c.requests();
        assert_eq!(reqs[0].group, Some(1));
        assert_eq!(reqs[3].group, None);
    }

    #[test]
    fn spacing_zero_merges_only_touching() {
        let mut c = GlassContainer::new(0.0);
        c.add(item(1, r(0.0, 0.0, 40.0, 40.0))).add(item(2, r(40.0, 0.0, 40.0, 40.0))).add(item(3, r(81.0, 0.0, 40.0, 40.0)));
        assert_eq!(c.groups(), vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn union_merges_far_and_variants_stay_apart() {
        let mut c = GlassContainer::new(4.0);
        let mut a = item(1, r(0.0, 0.0, 40.0, 40.0));
        let mut b = item(2, r(500.0, 0.0, 40.0, 40.0));
        a.union = Some(7);
        b.union = Some(7);
        let mut clear = item(3, r(42.0, 0.0, 40.0, 40.0));
        clear.glass = Glass::CLEAR;
        c.add(a).add(b).add(clear);
        assert_eq!(c.groups(), vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn shape_radii_clamp() {
        assert_eq!(MatShape::Capsule.radii(r(0.0, 0.0, 100.0, 40.0)), [20.0; 4]);
        assert_eq!(MatShape::Rounded(50.0).radii(r(0.0, 0.0, 100.0, 40.0)), [20.0; 4]);
        assert_eq!(MatShape::Rect.radii(r(0.0, 0.0, 10.0, 10.0)), [0.0; 4]);
    }

    #[test]
    fn morph_lands_exactly_and_starts_at_from() {
        let from = (r(0.0, 0.0, 40.0, 40.0), MatShape::Capsule);
        let to = (r(100.0, 20.0, 200.0, 60.0), MatShape::Rounded(12.0));
        let m = Morph::new(from, to, 1.0);
        let (s, sr) = m.at(1.0);
        assert_eq!(s, from.0);
        assert!((sr - 20.0).abs() < 1e-3);
        let (e, er) = m.at(100.0);
        assert_eq!(e, to.0);
        assert!((er - 12.0).abs() < 1e-3);
        assert!(m.is_done(100.0) && !m.is_done(1.05));
        let (mid, _) = m.at(1.1);
        assert!(mid.x > 0.0 && mid.x < 150.0);
    }

    #[test]
    fn ids_morph_on_change_and_settle() {
        let mut ids = GlassIds::default();
        let a = item(1, r(0.0, 0.0, 40.0, 40.0));
        assert_eq!(ids.frame(&[a.clone()], 0.0)[0].1, a.rect);
        let b = item(1, r(200.0, 0.0, 40.0, 40.0));
        let mid = ids.frame(&[b.clone()], 0.05)[0].1;
        assert!(mid.x < 200.0);
        assert_eq!(ids.frame(&[b.clone()], 50.0)[0].1, b.rect);
    }

    #[test]
    fn background_extension_strips() {
        let strips = background_extension(r(100.0, 0.0, 400.0, 300.0), r(0.0, 0.0, 500.0, 300.0));
        assert_eq!(strips, vec![(Edge::Leading, r(0.0, 0.0, 100.0, 300.0))]);
    }

    #[test]
    fn prominent_button_is_tinted_capsule() {
        let q = GlassButtonStyle::GlassProminent.request(r(0.0, 0.0, 80.0, 30.0), [0, 122, 255, 255]);
        assert_eq!(q.radii, [15.0; 4]);
        assert!(matches!(q.fill, Fill::Glass(Glass { tint: Some(_), interactive: true, .. })));
    }

    #[test]
    fn style_requests_become_the_shared_request() {
        let q = background(Material::Thick, MatShape::Rounded(8.0), r(0.0, 0.0, 100.0, 40.0)).to_material_request();
        assert_eq!((q.kind, q.strength, q.glass), (MaterialKind::Sheet, Some(3), None));
        assert_eq!(q.corners().tl, 8.0);
        let g = glass_effect(Glass::CLEAR, MatShape::Capsule, r(0.0, 0.0, 100.0, 40.0)).to_material_request();
        assert_eq!(g.glass, Some(GlassKind::Clear));
        assert_eq!(g.corners().br, 20.0);
        // every level maps to a distinct kind
        let kinds: std::collections::HashSet<_> = [Material::UltraThin, Material::Thin, Material::Regular, Material::Thick, Material::UltraThick, Material::Bar].map(Material::model_kind).into();
        assert_eq!(kinds.len(), 6);
    }

    #[test]
    fn material_levels_ascend() {
        assert!(Material::UltraThin.level() < Material::Thin.level() && Material::Thick.level() < Material::UltraThick.level());
    }
}
