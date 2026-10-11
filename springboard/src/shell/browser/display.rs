//! Retained, backend-neutral display list.
//!
//! Layout emits [`DisplayItem`]s through a [`DisplayListBuilder`], which
//! orders them by stacking context and paint phase. The finished
//! [`DisplayList`] knows every item's bounding box in page space (clips and
//! transforms applied), indexes them on a uniform grid so `items_in(rect)`
//! touches only nearby items, and can be diffed against the previous frame
//! with [`damage`] to find the minimal set of rectangles to repaint.
//!
//! Transforms are affine `[a, b, c, d, e, f]` with `x' = a*x + c*y + e` and
//! `y' = b*x + d*y + f`. Bounds are the axis-aligned box of the transformed
//! item.

use super::css::Color;
use crate::gfx::Image;
use std::collections::HashMap;
use std::sync::Arc;

/// An axis-aligned rectangle in page pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        !(self.w > 0.0 && self.h > 0.0)
    }

    pub fn area(&self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            self.w * self.h
        }
    }

    /// True when the two rectangles share interior area.
    pub fn intersects(&self, o: &Rect) -> bool {
        !self.is_empty() && !o.is_empty() && self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// True when they overlap or share an edge.
    pub fn touches(&self, o: &Rect) -> bool {
        !self.is_empty() && !o.is_empty() && self.x <= o.right() && o.x <= self.right() && self.y <= o.bottom() && o.y <= self.bottom()
    }

    pub fn contains(&self, o: &Rect) -> bool {
        o.is_empty() || (self.x <= o.x && self.y <= o.y && self.right() >= o.right() && self.bottom() >= o.bottom())
    }

    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect { x, y, w: self.right().max(o.right()) - x, h: self.bottom().max(o.bottom()) - y }
    }

    pub fn intersection(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        if r <= x || b <= y {
            Rect::ZERO
        } else {
            Rect { x, y, w: r - x, h: b - y }
        }
    }

    pub fn inflate(&self, d: f32) -> Rect {
        Rect { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }
}

pub type Xf = [f32; 6];
pub const IDENTITY: Xf = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` applied after `n` (so `mul(parent, child)` maps child space to page).
pub fn xf_mul(m: &Xf, n: &Xf) -> Xf {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

pub fn xf_point(m: &Xf, x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// The axis-aligned box of `r` under `m`.
pub fn xf_rect(m: &Xf, r: &Rect) -> Rect {
    if r.is_empty() {
        return Rect::ZERO;
    }
    let pts = [xf_point(m, r.x, r.y), xf_point(m, r.right(), r.y), xf_point(m, r.right(), r.bottom()), xf_point(m, r.x, r.bottom())];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Stretch to the rectangle.
    Fill,
    /// Scale to fit inside, centred.
    Contain,
    /// Scale to cover, centred, cropped.
    Cover,
    /// Natural size at the top-left, cropped.
    None,
}

/// A shaped, pre-rasterised line of text: `coverage` is `w * h` alpha bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    pub w: usize,
    pub h: usize,
    pub coverage: Arc<Vec<u8>>,
}

impl GlyphRun {
    pub fn new(w: usize, h: usize, coverage: Vec<u8>) -> GlyphRun {
        debug_assert_eq!(coverage.len(), w * h);
        GlyphRun { w, h, coverage: Arc::new(coverage) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    Rect { rect: Rect, color: Color },
    RoundedRect { rect: Rect, radius: [f32; 4], color: Color },
    /// Widths and colours are top, right, bottom, left; radii are
    /// top-left, top-right, bottom-right, bottom-left.
    Border { rect: Rect, widths: [f32; 4], colors: [Color; 4], radius: [f32; 4] },
    /// `rect` is the box casting the shadow (offset already applied). An
    /// outer shadow is drawn outside the box only; an inset one inside it.
    Shadow { rect: Rect, radius: f32, blur: f32, spread: f32, color: Color, inset: bool },
    Text { origin: (f32, f32), run: GlyphRun, color: Color },
    Image { rect: Rect, src: ImageId, fit: Fit, radius: f32 },
    /// CSS angle in degrees: 0 points up, 90 points right.
    LinearGradient { rect: Rect, angle: f32, stops: Vec<(f32, Color)> },
    /// `center` is absolute page position, `radii` the horizontal and
    /// vertical radius at offset 1.0.
    RadialGradient { rect: Rect, center: (f32, f32), radii: (f32, f32), stops: Vec<(f32, Color)> },
    PushClip { rect: Rect, radius: f32 },
    PopClip,
    PushTransform { matrix: Xf },
    PopTransform,
    PushOpacity { alpha: f32 },
    PopOpacity,
}

impl DisplayItem {
    /// Bounds in the item's own space (before transforms and clips).
    pub fn local_bounds(&self) -> Rect {
        match self {
            DisplayItem::Rect { rect, .. } | DisplayItem::RoundedRect { rect, .. } | DisplayItem::Border { rect, .. } => *rect,
            DisplayItem::Shadow { rect, blur, spread, inset, .. } => {
                if *inset {
                    *rect
                } else {
                    rect.inflate((spread + blur).max(0.0))
                }
            }
            DisplayItem::Text { origin, run, .. } => Rect::new(origin.0, origin.1, run.w as f32, run.h as f32),
            DisplayItem::Image { rect, .. } | DisplayItem::LinearGradient { rect, .. } | DisplayItem::RadialGradient { rect, .. } => *rect,
            _ => Rect::ZERO,
        }
    }

    pub fn is_push(&self) -> bool {
        matches!(self, DisplayItem::PushClip { .. } | DisplayItem::PushTransform { .. } | DisplayItem::PushOpacity { .. })
    }

    pub fn is_pop(&self) -> bool {
        matches!(self, DisplayItem::PopClip | DisplayItem::PopTransform | DisplayItem::PopOpacity)
    }

    fn closer(&self) -> Option<DisplayItem> {
        match self {
            DisplayItem::PushClip { .. } => Some(DisplayItem::PopClip),
            DisplayItem::PushTransform { .. } => Some(DisplayItem::PopTransform),
            DisplayItem::PushOpacity { .. } => Some(DisplayItem::PopOpacity),
            _ => None,
        }
    }
}

/// An item with its bounds in page space.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub item: DisplayItem,
    pub bounds: Rect,
}

const CELL: f32 = 128.0;
/// Items spanning more cells than this live in a side list instead.
const BIG_CELLS: usize = 64;

/// Finished, immutable display list with per-item bounds and a spatial index.
#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    entries: Vec<Entry>,
    images: HashMap<ImageId, Arc<Image>>,
    bounds: Rect,
    cols: usize,
    rows: usize,
    grid: Vec<Vec<u32>>,
    big: Vec<u32>,
}

struct Frame {
    start: usize,
    clip: Option<Rect>,
    xf: Xf,
    union: Rect,
}

impl DisplayList {
    pub fn new(items: Vec<DisplayItem>, images: HashMap<ImageId, Arc<Image>>) -> DisplayList {
        let entries = Self::measure(items);
        let bounds = entries.iter().fold(Rect::ZERO, |a, e| a.union(&e.bounds));
        let mut list = DisplayList { entries, images, bounds, ..Default::default() };
        list.index();
        list
    }

    fn measure(items: Vec<DisplayItem>) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::with_capacity(items.len());
        let mut stack: Vec<Frame> = vec![Frame { start: usize::MAX, clip: None, xf: IDENTITY, union: Rect::ZERO }];
        for item in items {
            let top = stack.last().unwrap();
            let (xf, clip) = (top.xf, top.clip);
            if item.is_pop() {
                if stack.len() > 1 {
                    let f = stack.pop().unwrap();
                    out[f.start].bounds = f.union;
                    let p = stack.last_mut().unwrap();
                    p.union = p.union.union(&f.union);
                    out.push(Entry { item, bounds: f.union });
                } else {
                    out.push(Entry { item, bounds: Rect::ZERO });
                }
                continue;
            }
            if item.is_push() {
                let (mut nxf, mut nclip) = (xf, clip);
                match &item {
                    DisplayItem::PushClip { rect, .. } => {
                        let r = xf_rect(&xf, rect);
                        nclip = Some(clip.map_or(r, |c| c.intersection(&r)));
                    }
                    DisplayItem::PushTransform { matrix } => nxf = xf_mul(&xf, matrix),
                    _ => {}
                }
                out.push(Entry { item, bounds: Rect::ZERO });
                stack.push(Frame { start: out.len() - 1, clip: nclip, xf: nxf, union: Rect::ZERO });
                continue;
            }
            let mut b = xf_rect(&xf, &item.local_bounds());
            if let Some(c) = clip {
                b = b.intersection(&c);
            }
            let top = stack.last_mut().unwrap();
            top.union = top.union.union(&b);
            out.push(Entry { item, bounds: b });
        }
        // implicitly close anything left open
        while stack.len() > 1 {
            let f = stack.pop().unwrap();
            out[f.start].bounds = f.union;
            let p = stack.last_mut().unwrap();
            p.union = p.union.union(&f.union);
        }
        out
    }

    fn index(&mut self) {
        if self.bounds.is_empty() {
            return;
        }
        self.cols = (self.bounds.right().max(1.0) / CELL).ceil() as usize;
        self.rows = (self.bounds.bottom().max(1.0) / CELL).ceil() as usize;
        self.grid = vec![Vec::new(); self.cols * self.rows];
        for (i, e) in self.entries.iter().enumerate() {
            if e.bounds.is_empty() {
                continue;
            }
            let (c0, r0, c1, r1) = self.cells(&e.bounds);
            if (c1 - c0 + 1) * (r1 - r0 + 1) > BIG_CELLS {
                self.big.push(i as u32);
                continue;
            }
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.grid[r * self.cols + c].push(i as u32);
                }
            }
        }
    }

    /// Inclusive cell range covering `r`, clamped to the grid.
    fn cells(&self, r: &Rect) -> (usize, usize, usize, usize) {
        let cell = |v: f32, n: usize| ((v / CELL).floor().max(0.0) as usize).min(n - 1);
        (cell(r.x, self.cols), cell(r.y, self.rows), cell(r.right(), self.cols), cell(r.bottom(), self.rows))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Union of every item's bounds.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn image(&self, id: ImageId) -> Option<&Arc<Image>> {
        self.images.get(&id)
    }

    /// Indices (ascending, so paint order) of the entries whose bounds
    /// intersect `rect`. Push and pop of one group always appear together.
    pub fn items_in(&self, rect: &Rect) -> Vec<usize> {
        if self.grid.is_empty() || rect.is_empty() || !self.bounds.intersects(rect) {
            return Vec::new();
        }
        let (c0, r0, c1, r1) = self.cells(rect);
        let mut hit: Vec<u32> = self.big.clone();
        for r in r0..=r1 {
            for c in c0..=c1 {
                hit.extend_from_slice(&self.grid[r * self.cols + c]);
            }
        }
        hit.sort_unstable();
        hit.dedup();
        hit.into_iter().map(|i| i as usize).filter(|&i| self.entries[i].bounds.intersects(rect)).collect()
    }
}

/// Which pass of a stacking context an item belongs to, in paint order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Background = 0,
    Float = 1,
    Inline = 2,
    Positioned = 3,
}

struct Ctx {
    z: i32,
    seq: usize,
    opens: Vec<DisplayItem>,
    phases: [Vec<DisplayItem>; 4],
    below: Vec<(i32, usize, Vec<DisplayItem>)>,
    above: Vec<(i32, usize, Vec<DisplayItem>)>,
}

impl Ctx {
    fn new(z: i32, seq: usize, opens: Vec<DisplayItem>) -> Ctx {
        Ctx { z, seq, opens, phases: Default::default(), below: Vec::new(), above: Vec::new() }
    }

    /// CSS painting order: own backgrounds, negative z children, floats,
    /// inline content, positioned content, then z >= 0 children by z-index
    /// (ties in document order).
    fn flatten(mut self) -> Vec<DisplayItem> {
        let closes: Vec<DisplayItem> = self.opens.iter().rev().filter_map(|i| i.closer()).collect();
        self.below.sort_by_key(|c| (c.0, c.1));
        self.above.sort_by_key(|c| (c.0, c.1));
        let [bg, fl, inl, pos] = self.phases;
        let mut out = self.opens;
        out.extend(bg);
        out.extend(self.below.into_iter().flat_map(|c| c.2));
        out.extend(fl);
        out.extend(inl);
        out.extend(pos);
        out.extend(self.above.into_iter().flat_map(|c| c.2));
        out.extend(closes);
        out
    }
}

pub struct DisplayListBuilder {
    stack: Vec<Ctx>,
    seq: usize,
    images: HashMap<ImageId, Arc<Image>>,
}

impl Default for DisplayListBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayListBuilder {
    pub fn new() -> Self {
        DisplayListBuilder { stack: vec![Ctx::new(0, 0, Vec::new())], seq: 1, images: HashMap::new() }
    }

    pub fn add_image(&mut self, id: ImageId, img: Arc<Image>) {
        self.images.insert(id, img);
    }

    /// Opens a child stacking context. `opens` are push items (clip,
    /// transform, opacity) wrapped around the whole context; the matching
    /// pops are emitted automatically when it ends.
    pub fn begin_context(&mut self, z: i32, opens: Vec<DisplayItem>) {
        self.seq += 1;
        self.stack.push(Ctx::new(z, self.seq, opens));
    }

    /// Closes the innermost context and files it in its parent. The root
    /// context cannot be ended.
    pub fn end_context(&mut self) {
        if self.stack.len() < 2 {
            return;
        }
        let c = self.stack.pop().unwrap();
        let (z, seq) = (c.z, c.seq);
        let flat = c.flatten();
        let parent = self.stack.last_mut().unwrap();
        if z < 0 {
            parent.below.push((z, seq, flat));
        } else {
            parent.above.push((z, seq, flat));
        }
    }

    pub fn push(&mut self, phase: Phase, item: DisplayItem) {
        self.stack.last_mut().unwrap().phases[phase as usize].push(item);
    }

    pub fn build(mut self) -> DisplayList {
        while self.stack.len() > 1 {
            self.end_context();
        }
        let items = self.stack.pop().unwrap().flatten();
        DisplayList::new(items, self.images)
    }
}

/// Merges rectangles that overlap, or touch such that their union wastes no
/// more area than the two pieces together hold, until none remain.
fn coalesce(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.retain(|r| !r.is_empty());
    if rects.len() > 256 {
        return vec![rects.iter().fold(Rect::ZERO, |a, r| a.union(r))];
    }
    loop {
        let mut merged = false;
        'outer: for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (a, b) = (rects[i], rects[j]);
                let u = a.union(&b);
                if a.intersects(&b) || (a.touches(&b) && u.area() <= a.area() + b.area() + 0.01) {
                    rects[i] = u;
                    rects.swap_remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    rects.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap_or(std::cmp::Ordering::Equal));
    rects
}

/// Largest middle section (old x new) the exact diff is attempted on.
const LCS_LIMIT: usize = 1 << 22;

/// The page regions that differ between two display lists. Entries are
/// matched in paint order (longest common subsequence); every unmatched old
/// entry damages where it used to paint and every unmatched new entry where
/// it paints now. Entries compare by item and by page-space bounds, so a
/// moved group damages its children too.
pub fn damage(old: &DisplayList, new: &DisplayList) -> Vec<Rect> {
    let (a, b) = (old.entries(), new.entries());
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut out = Vec::new();
    if ma.len().saturating_mul(mb.len()) > LCS_LIMIT {
        out.extend(ma.iter().chain(mb).map(|e| e.bounds));
        return coalesce(vec![out.iter().fold(Rect::ZERO, |a, r| a.union(r))]);
    }
    let (n, m) = (ma.len(), mb.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[at(i, j)] = if ma[i] == mb[j] { t[at(i + 1, j + 1)] + 1 } else { t[at(i + 1, j)].max(t[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ma[i] == mb[j] {
            i += 1;
            j += 1;
        } else if t[at(i + 1, j)] >= t[at(i, j + 1)] {
            out.push(ma[i].bounds);
            i += 1;
        } else {
            out.push(mb[j].bounds);
            j += 1;
        }
    }
    out.extend(ma[i..].iter().map(|e| e.bounds));
    out.extend(mb[j..].iter().map(|e| e.bounds));
    // push/pop markers carry their group's bounds; damage by content only
    // matters when the group itself changed, which is already included.
    coalesce(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);

    fn r(x: f32, y: f32, w: f32, h: f32) -> DisplayItem {
        DisplayItem::Rect { rect: Rect::new(x, y, w, h), color: RED }
    }

    fn list(items: Vec<DisplayItem>) -> DisplayList {
        DisplayList::new(items, HashMap::new())
    }

    #[test]
    fn rect_math() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.intersects(&b));
        assert_eq!(a.intersection(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!(!a.intersects(&Rect::new(10.0, 0.0, 5.0, 5.0)));
        assert!(a.touches(&Rect::new(10.0, 0.0, 5.0, 5.0)));
        assert_eq!(a.union(&Rect::ZERO), a);
        assert!(a.intersection(&Rect::new(20.0, 20.0, 1.0, 1.0)).is_empty());
        assert!(a.contains(&Rect::new(1.0, 1.0, 2.0, 2.0)));
        assert_eq!(a.inflate(2.0), Rect::new(-2.0, -2.0, 14.0, 14.0));
    }

    #[test]
    fn transform_math() {
        let t = [2.0, 0.0, 0.0, 3.0, 10.0, 20.0];
        assert_eq!(xf_point(&t, 1.0, 1.0), (12.0, 23.0));
        assert_eq!(xf_rect(&t, &Rect::new(0.0, 0.0, 1.0, 1.0)), Rect::new(10.0, 20.0, 2.0, 3.0));
        let shift = [1.0, 0.0, 0.0, 1.0, 5.0, 0.0];
        // child shift applied first, then the parent scale
        assert_eq!(xf_point(&xf_mul(&t, &shift), 0.0, 0.0), (20.0, 20.0));
        assert_eq!(xf_mul(&IDENTITY, &t), t);
        // 90 degree rotation maps to the box of the rotated corners
        let rot = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
        assert_eq!(xf_rect(&rot, &Rect::new(0.0, 0.0, 4.0, 2.0)), Rect::new(-2.0, 0.0, 2.0, 4.0));
    }

    #[test]
    fn bounds_per_item_kind() {
        let run = GlyphRun::new(4, 2, vec![255; 8]);
        let shadow = DisplayItem::Shadow { rect: Rect::new(10.0, 10.0, 20.0, 20.0), radius: 0.0, blur: 4.0, spread: 2.0, color: RED, inset: false };
        assert_eq!(shadow.local_bounds(), Rect::new(4.0, 4.0, 32.0, 32.0));
        let inset = DisplayItem::Shadow { rect: Rect::new(10.0, 10.0, 20.0, 20.0), radius: 0.0, blur: 4.0, spread: 2.0, color: RED, inset: true };
        assert_eq!(inset.local_bounds(), Rect::new(10.0, 10.0, 20.0, 20.0));
        let text = DisplayItem::Text { origin: (3.0, 4.0), run, color: RED };
        assert_eq!(text.local_bounds(), Rect::new(3.0, 4.0, 4.0, 2.0));
        assert!(DisplayItem::PopClip.local_bounds().is_empty());
    }

    #[test]
    fn clip_bounds_limit_children() {
        let l = list(vec![
            DisplayItem::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0), radius: 0.0 },
            r(5.0, 5.0, 100.0, 100.0),
            DisplayItem::PopClip,
        ]);
        assert_eq!(l.entries()[1].bounds, Rect::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(l.entries()[0].bounds, l.entries()[1].bounds);
        assert_eq!(l.entries()[2].bounds, l.entries()[1].bounds);
        assert_eq!(l.bounds(), Rect::new(5.0, 5.0, 5.0, 5.0));
    }

    #[test]
    fn transform_bounds_apply_to_children() {
        let l = list(vec![
            DisplayItem::PushTransform { matrix: [1.0, 0.0, 0.0, 1.0, 100.0, 50.0] },
            r(0.0, 0.0, 10.0, 10.0),
            DisplayItem::PopTransform,
            r(0.0, 0.0, 10.0, 10.0),
        ]);
        assert_eq!(l.entries()[1].bounds, Rect::new(100.0, 50.0, 10.0, 10.0));
        assert_eq!(l.entries()[3].bounds, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(l.entries()[0].bounds, l.entries()[1].bounds);
    }

    #[test]
    fn nested_groups_propagate_union() {
        let l = list(vec![
            DisplayItem::PushOpacity { alpha: 0.5 },
            DisplayItem::PushTransform { matrix: [1.0, 0.0, 0.0, 1.0, 20.0, 0.0] },
            r(0.0, 0.0, 10.0, 10.0),
            DisplayItem::PopTransform,
            r(0.0, 0.0, 5.0, 5.0),
            DisplayItem::PopOpacity,
        ]);
        assert_eq!(l.entries()[0].bounds, Rect::new(0.0, 0.0, 30.0, 10.0));
        assert_eq!(l.entries()[5].bounds, l.entries()[0].bounds);
    }

    #[test]
    fn unbalanced_lists_do_not_panic() {
        let l = list(vec![DisplayItem::PopClip, DisplayItem::PushOpacity { alpha: 1.0 }, r(0.0, 0.0, 4.0, 4.0)]);
        assert_eq!(l.entries()[1].bounds, Rect::new(0.0, 0.0, 4.0, 4.0));
    }

    #[test]
    fn items_in_matches_brute_force() {
        let mut items = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                items.push(r(i as f32 * 37.0, j as f32 * 41.0, 30.0, 20.0));
            }
        }
        items.push(r(0.0, 0.0, 1500.0, 1700.0)); // big
        let l = list(items);
        for q in [Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(300.0, 400.0, 200.0, 90.0), Rect::new(1400.0, 1600.0, 500.0, 500.0), Rect::new(-50.0, -50.0, 60.0, 60.0)] {
            let want: Vec<usize> = l.entries().iter().enumerate().filter(|(_, e)| e.bounds.intersects(&q)).map(|(i, _)| i).collect();
            assert_eq!(l.items_in(&q), want, "{q:?}");
        }
        assert!(l.items_in(&Rect::new(5000.0, 5000.0, 10.0, 10.0)).is_empty());
        assert!(l.items_in(&Rect::ZERO).is_empty());
    }

    #[test]
    fn items_in_is_in_paint_order_and_unique() {
        let l = list(vec![r(0.0, 0.0, 300.0, 300.0), r(10.0, 10.0, 5.0, 5.0), r(200.0, 200.0, 50.0, 50.0)]);
        assert_eq!(l.items_in(&Rect::new(0.0, 0.0, 400.0, 400.0)), vec![0, 1, 2]);
        assert_eq!(l.items_in(&Rect::new(205.0, 205.0, 5.0, 5.0)), vec![0, 2]);
    }

    #[test]
    fn group_markers_travel_together() {
        let l = list(vec![
            DisplayItem::PushOpacity { alpha: 0.5 },
            r(0.0, 0.0, 10.0, 10.0),
            r(500.0, 500.0, 10.0, 10.0),
            DisplayItem::PopOpacity,
        ]);
        assert_eq!(l.items_in(&Rect::new(0.0, 0.0, 20.0, 20.0)), vec![0, 1, 3]);
        assert_eq!(l.items_in(&Rect::new(495.0, 495.0, 20.0, 20.0)), vec![0, 2, 3]);
    }

    #[test]
    fn builder_orders_phases() {
        let mut b = DisplayListBuilder::new();
        b.push(Phase::Positioned, r(3.0, 0.0, 1.0, 1.0));
        b.push(Phase::Inline, r(2.0, 0.0, 1.0, 1.0));
        b.push(Phase::Float, r(1.0, 0.0, 1.0, 1.0));
        b.push(Phase::Background, r(0.0, 0.0, 1.0, 1.0));
        let xs: Vec<f32> = b.build().entries().iter().map(|e| e.bounds.x).collect();
        assert_eq!(xs, vec![0.0, 1.0, 2.0, 3.0]);
    }

    fn xs(l: &DisplayList) -> Vec<f32> {
        l.entries().iter().filter(|e| !e.item.is_push() && !e.item.is_pop()).map(|e| e.bounds.x).collect()
    }

    #[test]
    fn builder_orders_by_z_index() {
        let mut b = DisplayListBuilder::new();
        b.push(Phase::Background, r(0.0, 0.0, 1.0, 1.0));
        for (z, x) in [(5, 5.0), (-1, 1.0), (2, 2.0), (0, 3.0), (-3, 4.0)] {
            b.begin_context(z, vec![]);
            b.push(Phase::Inline, r(x, 0.0, 1.0, 1.0));
            b.end_context();
        }
        b.push(Phase::Inline, r(9.0, 0.0, 1.0, 1.0));
        // background, z<0 (-3, -1), own inline, z>=0 (0, 2, 5)
        assert_eq!(xs(&b.build()), vec![0.0, 4.0, 1.0, 9.0, 3.0, 2.0, 5.0]);
    }

    #[test]
    fn equal_z_keeps_document_order() {
        let mut b = DisplayListBuilder::new();
        for x in [1.0, 2.0, 3.0] {
            b.begin_context(1, vec![]);
            b.push(Phase::Inline, r(x, 0.0, 1.0, 1.0));
            b.end_context();
        }
        assert_eq!(xs(&b.build()), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn contexts_nest_and_wrap_with_pops() {
        let mut b = DisplayListBuilder::new();
        b.begin_context(0, vec![DisplayItem::PushOpacity { alpha: 0.5 }, DisplayItem::PushClip { rect: Rect::new(0.0, 0.0, 50.0, 50.0), radius: 0.0 }]);
        b.push(Phase::Inline, r(1.0, 1.0, 5.0, 5.0));
        b.begin_context(-1, vec![]);
        b.push(Phase::Inline, r(2.0, 2.0, 5.0, 5.0));
        // left open on purpose: build() closes it
        let l = b.build();
        let kinds: Vec<&DisplayItem> = l.entries().iter().map(|e| &e.item).collect();
        assert!(matches!(kinds[0], DisplayItem::PushOpacity { .. }));
        assert!(matches!(kinds[1], DisplayItem::PushClip { .. }));
        assert!(matches!(kinds[kinds.len() - 2], DisplayItem::PopClip));
        assert!(matches!(kinds[kinds.len() - 1], DisplayItem::PopOpacity));
        // negative-z child paints before the parent's inline content
        assert_eq!(l.entries()[2].bounds.x, 2.0);
        assert_eq!(l.entries()[3].bounds.x, 1.0);
    }

    #[test]
    fn root_context_cannot_be_ended() {
        let mut b = DisplayListBuilder::new();
        b.end_context();
        b.push(Phase::Inline, r(0.0, 0.0, 1.0, 1.0));
        assert_eq!(b.build().len(), 1);
    }

    #[test]
    fn builder_keeps_images() {
        let mut b = DisplayListBuilder::new();
        b.add_image(ImageId(7), Arc::new(Image::new(2, 2)));
        let l = b.build();
        assert!(l.image(ImageId(7)).is_some());
        assert!(l.image(ImageId(8)).is_none());
    }

    #[test]
    fn damage_of_identical_lists_is_empty() {
        let a = list(vec![r(0.0, 0.0, 10.0, 10.0), r(20.0, 0.0, 10.0, 10.0)]);
        assert!(damage(&a, &a.clone()).is_empty());
        assert!(damage(&DisplayList::default(), &DisplayList::default()).is_empty());
    }

    #[test]
    fn damage_of_changed_colour_is_that_rect() {
        let a = list(vec![r(0.0, 0.0, 10.0, 10.0), r(100.0, 0.0, 10.0, 10.0), r(200.0, 0.0, 10.0, 10.0)]);
        let b = list(vec![r(0.0, 0.0, 10.0, 10.0), DisplayItem::Rect { rect: Rect::new(100.0, 0.0, 10.0, 10.0), color: BLUE }, r(200.0, 0.0, 10.0, 10.0)]);
        assert_eq!(damage(&a, &b), vec![Rect::new(100.0, 0.0, 10.0, 10.0)]);
    }

    #[test]
    fn damage_of_insertion_is_new_item_only() {
        let a = list(vec![r(0.0, 0.0, 10.0, 10.0), r(200.0, 0.0, 10.0, 10.0)]);
        let b = list(vec![r(0.0, 0.0, 10.0, 10.0), r(100.0, 0.0, 10.0, 10.0), r(200.0, 0.0, 10.0, 10.0)]);
        assert_eq!(damage(&a, &b), vec![Rect::new(100.0, 0.0, 10.0, 10.0)]);
        assert_eq!(damage(&b, &a), vec![Rect::new(100.0, 0.0, 10.0, 10.0)]);
    }

    #[test]
    fn damage_of_move_covers_old_and_new() {
        let a = list(vec![r(0.0, 0.0, 10.0, 10.0)]);
        let b = list(vec![r(300.0, 300.0, 10.0, 10.0)]);
        let d = damage(&a, &b);
        assert_eq!(d.len(), 2);
        assert!(d.contains(&Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(d.contains(&Rect::new(300.0, 300.0, 10.0, 10.0)));
    }

    #[test]
    fn damage_merges_overlapping_rects() {
        let a = list(vec![r(0.0, 0.0, 10.0, 10.0)]);
        let b = list(vec![r(5.0, 5.0, 10.0, 10.0)]);
        assert_eq!(damage(&a, &b), vec![Rect::new(0.0, 0.0, 15.0, 15.0)]);
    }

    #[test]
    fn damage_merges_flush_neighbours_but_not_l_shapes() {
        let flush = coalesce(vec![Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(10.0, 0.0, 10.0, 10.0)]);
        assert_eq!(flush, vec![Rect::new(0.0, 0.0, 20.0, 10.0)]);
        let diag = coalesce(vec![Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(10.0, 10.0, 10.0, 10.0)]);
        assert_eq!(diag.len(), 2);
    }

    #[test]
    fn damage_of_moved_group_includes_children() {
        let mk = |dx: f32| {
            list(vec![DisplayItem::PushTransform { matrix: [1.0, 0.0, 0.0, 1.0, dx, 0.0] }, r(0.0, 0.0, 10.0, 10.0), DisplayItem::PopTransform])
        };
        let d = damage(&mk(0.0), &mk(100.0));
        assert!(d.iter().any(|r| r.contains(&Rect::new(0.0, 0.0, 10.0, 10.0))));
        assert!(d.iter().any(|r| r.contains(&Rect::new(100.0, 0.0, 10.0, 10.0))));
    }

    #[test]
    fn damage_falls_back_for_huge_middles() {
        let big = |c: u8| list((0..2100).map(|i| DisplayItem::Rect { rect: Rect::new(i as f32, 0.0, 1.0, 1.0), color: Color::rgb(c, 0, 0) }).collect());
        let d = damage(&big(1), &big(2));
        assert_eq!(d, vec![Rect::new(0.0, 0.0, 2100.0, 1.0)]);
    }
}
