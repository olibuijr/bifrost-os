//! Auto Layout for retained views (`NativeLayoutConstraint`, `NativeLayoutAnchor`, `NativeLayoutGuide`), on the Cassowary solver of
//! `bifrost_ui::constraint`.
//!
//! Views are named by id (`View::id()`); [`anchors`] gives their [`Attr`]s and a [`Constraints`] builder collects
//! [`Spec`]s (`first = multiplier * second + constant` with a priority 1 to 1000) which `Window::constrain` hands to the
//! window. A view that no constraint mentions keeps the frame its parent gave it. A mentioned view also contributes its
//! intrinsic size: `width <= intrinsic` at the content hugging priority and `width >= intrinsic` at the compression
//! resistance priority (defaults 250 and 750, `View::hugging` / `View::compression`, or `Constraints::hugging`).
//! The frame it already has is a priority-1 preference, so an under-constrained view stays put instead of collapsing.
//!
//! ```ignore
//! let mut cs = Constraints::new();
//! cs.add(Spec::eq(anchors(label).leading(), anchors(Item::Margins).leading()));
//! cs.add(Spec::eq(anchors(label).top(), anchors(Item::SafeArea).top()).constant(8.0));
//! cs.add(Spec::eq(anchors(field).leading(), anchors(label).trailing()).constant(8.0));
//! window.constrain(cs);
//! ```
use bifrost_ui::constraint::{Expr, Rel, Solver, Strength, Var};
use bifrost_ui::stack::Axis;
use bifrost_ui::Rect;
use std::collections::BTreeMap;

/// `NativeLayoutConstraint.Attribute` (the edges are in window points; y grows downwards, so `Top` < `Bottom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Leading,
    Trailing,
    Top,
    Bottom,
    Width,
    Height,
    CenterX,
    CenterY,
}

/// What an anchor belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    /// The window's content area (`contentLayoutGuide`).
    Window,
    /// The content area below the top safe inset (`safeAreaLayoutGuide`, `layout::top_inset()`).
    SafeArea,
    /// The content area inset by the standard margin (`layoutMarginsGuide`, `prefs_layout::m::DETAIL_MARGIN`).
    Margins,
    /// A view by `View::id()`.
    View(u64),
    /// A frameless `NativeLayoutGuide` from [`Constraints::guide`].
    Guide(u64),
}

impl From<u64> for Item {
    fn from(id: u64) -> Item {
        Item::View(id)
    }
}

/// One anchor of one item (`NativeLayoutXAxisAnchor` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub item: Item,
    pub anchor: Anchor,
}

/// Anchor factory for an item: `anchors(view_id).leading()`.
#[derive(Clone, Copy, Debug)]
pub struct Anchors(pub Item);

pub fn anchors(item: impl Into<Item>) -> Anchors {
    Anchors(item.into())
}

macro_rules! anchor_fns {
    ($($name:ident => $a:ident),*) => {
        impl Anchors { $(pub fn $name(self) -> Attr { Attr { item: self.0, anchor: Anchor::$a } })* }
    };
}
anchor_fns!(leading => Leading, trailing => Trailing, top => Top, bottom => Bottom, width => Width, height => Height, center_x => CenterX, center_y => CenterY);

/// An `NativeLayoutConstraint`: `first rel multiplier * second + constant` at `priority`; without `second` it is
/// `first rel constant` (a size or an absolute position).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub first: Attr,
    pub rel: Rel,
    pub second: Option<Attr>,
    pub multiplier: f64,
    pub constant: f64,
    /// `NativeLayoutPriority` 1 to 1000; 1000 is required.
    pub priority: u16,
}

impl Spec {
    pub fn new(first: Attr, rel: Rel, second: Attr) -> Spec {
        Spec { first, rel, second: Some(second), multiplier: 1.0, constant: 0.0, priority: 1000 }
    }
    pub fn eq(first: Attr, second: Attr) -> Spec {
        Spec::new(first, Rel::Eq, second)
    }
    pub fn le(first: Attr, second: Attr) -> Spec {
        Spec::new(first, Rel::Le, second)
    }
    pub fn ge(first: Attr, second: Attr) -> Spec {
        Spec::new(first, Rel::Ge, second)
    }
    /// `first rel constant`, e.g. `Spec::fixed(anchors(v).width(), Rel::Eq, 120.0)`.
    pub fn fixed(first: Attr, rel: Rel, constant: f64) -> Spec {
        Spec { first, rel, second: None, multiplier: 1.0, constant, priority: 1000 }
    }
    pub fn constant(mut self, c: f64) -> Spec {
        self.constant = c;
        self
    }
    pub fn multiplier(mut self, m: f64) -> Spec {
        self.multiplier = m;
        self
    }
    pub fn priority(mut self, p: u16) -> Spec {
        self.priority = p.clamp(1, 1000);
        self
    }
}

/// A set of constraints plus per-view priority overrides.
#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub specs: Vec<Spec>,
    hugging: BTreeMap<(u64, bool), u16>,
    compression: BTreeMap<(u64, bool), u16>,
    next_guide: u64,
}

fn horizontal(a: Axis) -> bool {
    a == Axis::Horizontal
}

impl Constraints {
    pub fn new() -> Constraints {
        Constraints::default()
    }
    pub fn add(&mut self, s: Spec) -> &mut Self {
        self.specs.push(s);
        self
    }
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
    pub fn extend(&mut self, other: Constraints) {
        self.specs.extend(other.specs);
        self.hugging.extend(other.hugging);
        self.compression.extend(other.compression);
        self.next_guide = self.next_guide.max(other.next_guide);
    }
    /// `setContentHuggingPriority(_:for:)` for view `id`.
    pub fn hugging(&mut self, id: u64, axis: Axis, priority: u16) -> &mut Self {
        self.hugging.insert((id, horizontal(axis)), priority.clamp(1, 1000));
        self
    }
    /// `setContentCompressionResistancePriority(_:for:)` for view `id`.
    pub fn compression(&mut self, id: u64, axis: Axis, priority: u16) -> &mut Self {
        self.compression.insert((id, horizontal(axis)), priority.clamp(1, 1000));
        self
    }
    /// A new `NativeLayoutGuide`: an invisible rectangle to align views against (spacers, equal-width groups).
    pub fn guide(&mut self) -> Item {
        self.next_guide += 1;
        Item::Guide(self.next_guide)
    }
    /// Pins all four edges of `id` to `parent` inset by `inset` (`edges == superview.edges`).
    pub fn fill(&mut self, id: u64, parent: Item, inset: f64) -> &mut Self {
        let (c, p) = (anchors(id), anchors(parent));
        self.add(Spec::eq(c.leading(), p.leading()).constant(inset));
        self.add(Spec::eq(c.trailing(), p.trailing()).constant(-inset));
        self.add(Spec::eq(c.top(), p.top()).constant(inset));
        self.add(Spec::eq(c.bottom(), p.bottom()).constant(-inset))
    }
    /// Centres `id` in `parent` on both axes.
    pub fn center(&mut self, id: u64, parent: Item) -> &mut Self {
        let (c, p) = (anchors(id), anchors(parent));
        self.add(Spec::eq(c.center_x(), p.center_x()));
        self.add(Spec::eq(c.center_y(), p.center_y()))
    }
    /// Fixed size.
    pub fn size(&mut self, id: u64, w: f64, h: f64) -> &mut Self {
        self.add(Spec::fixed(anchors(id).width(), Rel::Eq, w));
        self.add(Spec::fixed(anchors(id).height(), Rel::Eq, h))
    }
    /// Gives all `ids` the same width (`a.width == b.width`).
    pub fn equal_widths(&mut self, ids: &[u64]) -> &mut Self {
        for w in ids.windows(2) {
            self.add(Spec::eq(anchors(w[1]).width(), anchors(w[0]).width()));
        }
        self
    }
    /// Lays `ids` out left to right, `gap` apart (`leading == previous.trailing + gap`).
    pub fn chain_h(&mut self, ids: &[u64], gap: f64) -> &mut Self {
        for w in ids.windows(2) {
            self.add(Spec::eq(anchors(w[1]).leading(), anchors(w[0]).trailing()).constant(gap));
        }
        self
    }
    /// Lays `ids` out top to bottom, `gap` apart.
    pub fn chain_v(&mut self, ids: &[u64], gap: f64) -> &mut Self {
        for w in ids.windows(2) {
            self.add(Spec::eq(anchors(w[1]).top(), anchors(w[0]).bottom()).constant(gap));
        }
        self
    }

    /// Ids of the views any spec mentions.
    pub fn view_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self
            .specs
            .iter()
            .flat_map(|s| [Some(s.first), s.second])
            .flatten()
            .filter_map(|a| if let Item::View(id) = a.item { Some(id) } else { None })
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
    fn hug(&self, id: u64, h: bool, default: u16) -> u16 {
        self.hugging.get(&(id, h)).copied().unwrap_or(default)
    }
    fn comp(&self, id: u64, h: bool, default: u16) -> u16 {
        self.compression.get(&(id, h)).copied().unwrap_or(default)
    }
}

/// What the solver needs to know about a view, gathered by the window.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub id: u64,
    /// The frame the manual layout gave it (the weakest preference).
    pub frame: Rect,
    /// `View::intrinsic`, 0 = none.
    pub intrinsic: (f32, f32),
    /// `View::hugging` and `View::compression` as `(horizontal, vertical)`.
    pub hugging: (u16, u16),
    pub compression: (u16, u16),
}

/// The result of one solve.
#[derive(Debug, Default)]
pub struct Solution {
    pub frames: BTreeMap<u64, Rect>,
    /// Indices into `Constraints::specs` of required constraints that conflicted and were left out (the console message
    /// "Unable to simultaneously satisfy constraints", without the debugging UI).
    pub broken: Vec<usize>,
}

struct Frame {
    x: Var,
    y: Var,
    w: Var,
    h: Var,
}

fn lin(s: &mut Solver, v: Var, value: f64, strength: Strength) {
    let _ = s.add(Expr::var(v) - Expr::constant(value), Rel::Eq, strength);
}

/// Solves `cs` for the views in `views` inside `bounds` (window points). `top_inset` is the safe area's top, `margin`
/// the layout margin on every side. Constraints naming a view that is not in `views` are ignored and reported broken.
pub fn solve(cs: &Constraints, bounds: Rect, top_inset: f32, margin: f32, views: &[ViewInfo]) -> Solution {
    let mut s = Solver::new();
    let mut frames: BTreeMap<Item, Frame> = BTreeMap::new();
    let new_frame = |s: &mut Solver, item: Item, frames: &mut BTreeMap<Item, Frame>| {
        let f = Frame { x: s.var(), y: s.var(), w: s.var(), h: s.var() };
        // A frame has no negative size.
        let _ = s.add(Expr::var(f.w), Rel::Ge, Strength::REQUIRED);
        let _ = s.add(Expr::var(f.h), Rel::Ge, Strength::REQUIRED);
        frames.insert(item, f);
    };
    let fixed = |s: &mut Solver, f: &Frame, r: Rect| {
        lin(s, f.x, r.x as f64, Strength::REQUIRED);
        lin(s, f.y, r.y as f64, Strength::REQUIRED);
        lin(s, f.w, r.w as f64, Strength::REQUIRED);
        lin(s, f.h, r.h as f64, Strength::REQUIRED);
    };
    let (ti, m) = (top_inset.max(0.0), margin.max(0.0));
    let areas = [
        (Item::Window, bounds),
        (Item::SafeArea, Rect { x: bounds.x, y: bounds.y + ti, w: bounds.w, h: (bounds.h - ti).max(0.0) }),
        (Item::Margins, Rect { x: bounds.x + m, y: bounds.y + m, w: (bounds.w - 2.0 * m).max(0.0), h: (bounds.h - 2.0 * m).max(0.0) }),
    ];
    for (item, r) in areas {
        new_frame(&mut s, item, &mut frames);
        fixed(&mut s, &frames[&item], r);
    }
    for v in views {
        let item = Item::View(v.id);
        new_frame(&mut s, item, &mut frames);
        let f = &frames[&item];
        // The frame the view has now: priority 1, below every real preference.
        let weakest = Strength::from_priority(1);
        lin(&mut s, f.x, v.frame.x as f64, weakest);
        lin(&mut s, f.y, v.frame.y as f64, weakest);
        lin(&mut s, f.w, v.frame.w as f64, weakest);
        lin(&mut s, f.h, v.frame.h as f64, weakest);
        for (len, var, hp, cp, h) in [(v.intrinsic.0, f.w, v.hugging.0, v.compression.0, true), (v.intrinsic.1, f.h, v.hugging.1, v.compression.1, false)] {
            if len > 0.0 {
                let _ = s.add(Expr::var(var) - Expr::constant(len as f64), Rel::Le, Strength::from_priority(cs.hug(v.id, h, hp)));
                let _ = s.add(Expr::var(var) - Expr::constant(len as f64), Rel::Ge, Strength::from_priority(cs.comp(v.id, h, cp)));
            }
        }
    }
    let mut broken = Vec::new();
    // Guides named by a spec exist even when nothing else constrains them.
    for sp in &cs.specs {
        for a in [Some(sp.first), sp.second].into_iter().flatten() {
            if matches!(a.item, Item::Guide(_)) && !frames.contains_key(&a.item) {
                new_frame(&mut s, a.item, &mut frames);
            }
        }
    }
    let attr = |frames: &BTreeMap<Item, Frame>, a: Attr| -> Option<Expr> {
        let f = frames.get(&a.item)?;
        let (x, y, w, h) = (Expr::var(f.x), Expr::var(f.y), Expr::var(f.w), Expr::var(f.h));
        Some(match a.anchor {
            Anchor::Leading => x,
            Anchor::Trailing => x + w,
            Anchor::Top => y,
            Anchor::Bottom => y + h,
            Anchor::Width => w,
            Anchor::Height => h,
            Anchor::CenterX => x + w * 0.5,
            Anchor::CenterY => y + h * 0.5,
        })
    };
    for (i, sp) in cs.specs.iter().enumerate() {
        let Some(first) = attr(&frames, sp.first) else {
            broken.push(i);
            continue;
        };
        let rhs = match sp.second {
            Some(b) => match attr(&frames, b) {
                Some(e) => e * sp.multiplier + Expr::constant(sp.constant),
                None => {
                    broken.push(i);
                    continue;
                }
            },
            None => Expr::constant(sp.constant),
        };
        if s.add(first - rhs, sp.rel, Strength::from_priority(sp.priority)).is_err() {
            broken.push(i);
        }
    }
    let frames_out = views
        .iter()
        .map(|v| {
            let f = &frames[&Item::View(v.id)];
            (v.id, Rect { x: s.value(f.x) as f32, y: s.value(f.y) as f32, w: s.value(f.w) as f32, h: s.value(f.h) as f32 })
        })
        .collect();
    Solution { frames: frames_out, broken }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Rect = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };

    fn info(id: u64, intrinsic: (f32, f32)) -> ViewInfo {
        ViewInfo { id, frame: Rect::default(), intrinsic, hugging: (250, 250), compression: (750, 750) }
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_fixed_size_view_is_centered_in_the_window() {
        let mut cs = Constraints::new();
        cs.size(1, 100.0, 40.0).center(1, Item::Window);
        let sol = solve(&cs, WIN, 0.0, 20.0, &[info(1, (0.0, 0.0))]);
        let r = sol.frames[&1];
        assert!(near(r.x, 150.0) && near(r.y, 130.0) && near(r.w, 100.0) && near(r.h, 40.0), "{r:?}");
        assert!(sol.broken.is_empty());
    }

    #[test]
    fn fill_pins_to_the_safe_area_and_margins() {
        let mut cs = Constraints::new();
        cs.fill(1, Item::Margins, 0.0).fill(2, Item::SafeArea, 4.0);
        let sol = solve(&cs, WIN, 30.0, 20.0, &[info(1, (0.0, 0.0)), info(2, (0.0, 0.0))]);
        let (a, b) = (sol.frames[&1], sol.frames[&2]);
        assert!(near(a.x, 20.0) && near(a.w, 360.0) && near(a.h, 260.0), "{a:?}");
        assert!(near(b.y, 34.0) && near(b.h, 262.0) && near(b.x, 4.0), "{b:?}");
    }

    #[test]
    fn intrinsic_size_is_kept_and_the_lower_hugging_view_stretches() {
        // label | field in a row of 300 pt; the field hugs less so it takes the slack.
        let mut cs = Constraints::new();
        cs.add(Spec::eq(anchors(1).leading(), anchors(Item::Window).leading()));
        cs.add(Spec::eq(anchors(2).leading(), anchors(1).trailing()).constant(8.0));
        cs.add(Spec::eq(anchors(2).trailing(), anchors(Item::Window).trailing()));
        cs.hugging(2, Axis::Horizontal, 100);
        let mut label = info(1, (60.0, 20.0));
        label.hugging.0 = 750;
        let sol = solve(&cs, WIN, 0.0, 0.0, &[label, info(2, (80.0, 22.0))]);
        assert!(near(sol.frames[&1].w, 60.0), "{:?}", sol.frames);
        assert!(near(sol.frames[&2].w, 400.0 - 68.0), "{:?}", sol.frames);
    }

    #[test]
    fn compression_resistance_decides_who_shrinks() {
        // Two views of 200 pt each in 300 pt: the one that resists less gives way.
        let mut cs = Constraints::new();
        cs.add(Spec::eq(anchors(1).leading(), anchors(Item::Window).leading()));
        cs.add(Spec::eq(anchors(2).leading(), anchors(1).trailing()));
        cs.add(Spec::eq(anchors(2).trailing(), anchors(Item::Window).trailing()));
        cs.compression(1, Axis::Horizontal, 900);
        cs.compression(2, Axis::Horizontal, 300);
        let win = Rect { w: 300.0, ..WIN };
        let sol = solve(&cs, win, 0.0, 0.0, &[info(1, (200.0, 10.0)), info(2, (200.0, 10.0))]);
        assert!(near(sol.frames[&1].w, 200.0) && near(sol.frames[&2].w, 100.0), "{:?}", sol.frames);
    }

    #[test]
    fn equal_widths_with_a_guide_and_multiplier() {
        let mut cs = Constraints::new();
        let g = cs.guide();
        cs.add(Spec::eq(anchors(g).leading(), anchors(Item::Window).leading()));
        cs.add(Spec::eq(anchors(g).trailing(), anchors(Item::Window).trailing()));
        cs.equal_widths(&[1, 2]);
        cs.chain_h(&[1, 2], 0.0);
        cs.add(Spec::eq(anchors(1).leading(), anchors(g).leading()));
        cs.add(Spec::eq(anchors(2).trailing(), anchors(g).trailing()));
        cs.add(Spec::eq(anchors(3).width(), anchors(1).width()).multiplier(0.5));
        let sol = solve(&cs, WIN, 0.0, 0.0, &[info(1, (0.0, 0.0)), info(2, (0.0, 0.0)), info(3, (0.0, 0.0))]);
        assert!(near(sol.frames[&1].w, 200.0) && near(sol.frames[&2].x, 200.0) && near(sol.frames[&3].w, 100.0), "{:?}", sol.frames);
    }

    #[test]
    fn conflicting_required_constraints_are_reported_by_index() {
        let mut cs = Constraints::new();
        cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 100.0));
        cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 150.0));
        cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 150.0).priority(500));
        let sol = solve(&cs, WIN, 0.0, 0.0, &[info(1, (0.0, 0.0))]);
        assert_eq!(sol.broken, vec![1]);
        assert!(near(sol.frames[&1].w, 100.0));
    }

    #[test]
    fn a_priority_below_required_gives_way_without_being_reported() {
        let mut cs = Constraints::new();
        cs.add(Spec::fixed(anchors(1).width(), Rel::Le, 80.0));
        cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 200.0).priority(750));
        let sol = solve(&cs, WIN, 0.0, 0.0, &[info(1, (0.0, 0.0))]);
        assert!(near(sol.frames[&1].w, 80.0) && sol.broken.is_empty());
    }

    #[test]
    fn an_unconstrained_axis_keeps_the_manual_frame() {
        let mut cs = Constraints::new();
        cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 50.0));
        let mut v = info(1, (0.0, 0.0));
        v.frame = Rect { x: 7.0, y: 9.0, w: 10.0, h: 11.0 };
        let r = solve(&cs, WIN, 0.0, 0.0, &[v]).frames[&1];
        assert!(near(r.x, 7.0) && near(r.y, 9.0) && near(r.w, 50.0) && near(r.h, 11.0), "{r:?}");
    }

    #[test]
    fn unknown_views_break_their_constraints() {
        let mut cs = Constraints::new();
        cs.add(Spec::eq(anchors(1).leading(), anchors(99).trailing()));
        assert_eq!(solve(&cs, WIN, 0.0, 0.0, &[info(1, (0.0, 0.0))]).broken, vec![0]);
        assert_eq!(cs.view_ids(), vec![1, 99]);
    }

    #[test]
    fn resizing_the_window_resolves_the_same_constraints() {
        let mut cs = Constraints::new();
        cs.fill(1, Item::Margins, 0.0);
        for w in [300.0, 800.0] {
            let r = solve(&cs, Rect { w, ..WIN }, 0.0, 20.0, &[info(1, (0.0, 0.0))]).frames[&1];
            assert!(near(r.w, w - 40.0));
        }
    }
}
