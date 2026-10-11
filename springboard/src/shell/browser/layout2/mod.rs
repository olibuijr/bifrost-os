//! Layout engine on taffy 0.14.
//!
//! Block, flex and grid layout come from taffy, driven through its low-level
//! traits directly over a compact box arena (`tree`): no `TaffyTree`, no
//! copy of the styles; a `taffy::Style` is built from the `Computed` style
//! of a node whenever taffy asks for it. Inline formatting (`inline`, with
//! `text`) is ours: a block container with inline content is a taffy leaf
//! whose measure function breaks lines. After taffy has run, `positioned`
//! resolves out-of-flow boxes, relative offsets and page positions; `paint`
//! turns the result into a `display::DisplayList`; `hit` answers pointer
//! queries; `table` maps tables onto grid.
//!
//! Known limits: floats are laid out as inline blocks, grid named lines and
//! areas are ignored, `text-overflow: ellipsis` is not drawn, table
//! `border-spacing`/`border-collapse` are ignored, `calc()` with percentages
//! is resolved against the parent size seen by the last layout pass (taffy's
//! `calc` feature is not enabled).

pub mod hit;
pub mod inline;
pub mod paint;
pub mod positioned;
pub mod table;
pub mod text;
pub mod tree;

#[cfg(test)]
mod tests;

pub use hit::{hit_test, link_at, scroller_at};
pub use text::TextMetrics;

use super::css::Color;
use super::display::{ImageId, Rect};
use super::dom::{Dom, NodeId};
use super::style::{self as cs, StyleTree};
use crate::gfx::Image;
use inline::{IfcData, Wrap};
use paint::BoxPaint;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use taffy as tf;
use taffy::{
    CacheTree, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutPartialTree, TraversePartialTree,
};
use tree::{box_for, sig_of, Builder, Bx, Kind, NONE};

/// Natural image sizes (and, optionally, the decoded pixels).
pub trait ImageSizes {
    /// Natural size of the image at `src`, when known.
    fn size(&self, src: &str) -> Option<(u32, u32)>;
    /// The id the display list refers to the image by.
    fn id(&self, _src: &str) -> Option<ImageId> {
        None
    }
    /// The decoded image for an id from `id`.
    fn image(&self, _id: ImageId) -> Option<Arc<Image>> {
        None
    }
}

/// No images known.
pub struct NoImages;

impl ImageSizes for NoImages {
    fn size(&self, _src: &str) -> Option<(u32, u32)> {
        None
    }
}

/// `Rgba` to the display list's 8-bit color.
pub(crate) fn col(r: cs::Rgba) -> Color {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color {
        r: q(r.r),
        g: q(r.g),
        b: q(r.b),
        a: q(r.a),
    }
}

/// The laid-out document.
pub struct LayoutTree {
    pub(crate) boxes: Vec<Bx>,
    pub(crate) kids: Vec<u32>,
    pub(crate) caches: Vec<tf::Cache>,
    pub(crate) box_of: Vec<u32>,
    pub(crate) pbox: HashMap<u32, u32>,
    pub(crate) sigs: HashMap<u32, u64>,
    pub(crate) ifcs: Vec<IfcData>,
    pub(crate) paint: Vec<BoxPaint>,
    pub(crate) widths: text::WidthCache,
    pub(crate) scrolls: HashMap<u32, (f32, f32)>,
    pub(crate) viewport: (f32, f32),
    pub(crate) root: u32,
    pub(crate) images: HashMap<ImageId, Arc<Image>>,
    pub(crate) pw_changed: bool,
    pub(crate) calc_used: Cell<bool>,
    pub(crate) last_scroll: Cell<(f32, f32)>,
    doc: (f32, f32),
}

impl LayoutTree {
    fn empty(viewport: (f32, f32)) -> LayoutTree {
        LayoutTree {
            boxes: Vec::new(),
            kids: Vec::new(),
            caches: Vec::new(),
            box_of: Vec::new(),
            pbox: HashMap::new(),
            sigs: HashMap::new(),
            ifcs: Vec::new(),
            paint: Vec::new(),
            widths: text::WidthCache::new(8192),
            scrolls: HashMap::new(),
            viewport,
            root: 0,
            images: HashMap::new(),
            pw_changed: false,
            calc_used: Cell::new(false),
            last_scroll: Cell::new((0.0, 0.0)),
            doc: (0.0, 0.0),
        }
    }

    /// Generates boxes for the whole document and lays it out.
    pub fn build(
        dom: &Dom,
        styles: &StyleTree,
        viewport: (f32, f32),
        text: &dyn TextMetrics,
        images: &dyn ImageSizes,
    ) -> LayoutTree {
        let mut t = LayoutTree::empty(viewport);
        t.rebuild(dom, styles, text, images);
        t
    }

    fn rebuild(
        &mut self,
        dom: &Dom,
        styles: &StyleTree,
        text: &dyn TextMetrics,
        images: &dyn ImageSizes,
    ) {
        self.boxes.clear();
        self.kids.clear();
        self.caches.clear();
        self.box_of.clear();
        self.pbox.clear();
        self.sigs.clear();
        self.ifcs.clear();
        self.paint.clear();
        {
            let mut b = Builder {
                dom,
                styles,
                images,
                t: &mut *self,
            };
            b.build_root();
        }
        self.layout(dom, styles, text, images);
    }

    fn layout(
        &mut self,
        dom: &Dom,
        styles: &StyleTree,
        text: &dyn TextMetrics,
        images: &dyn ImageSizes,
    ) {
        let vp = self.viewport;
        let root = tf::NodeId::from(self.root as usize);
        for pass in 0..3 {
            self.pw_changed = false;
            self.calc_used.set(false);
            {
                let mut cx = Ctx {
                    t: &mut *self,
                    dom,
                    styles,
                    text,
                    images,
                };
                tf::compute_root_layout(
                    &mut cx,
                    root,
                    tf::Size {
                        width: tf::AvailableSpace::Definite(vp.0),
                        height: tf::AvailableSpace::Definite(vp.1),
                    },
                );
            }
            if self.calc_used.get() && self.pw_changed && pass < 2 {
                for c in self.caches.iter_mut() {
                    let _ = c.clear();
                }
            } else {
                break;
            }
        }
        {
            let mut cx = Ctx {
                t: &mut *self,
                dom,
                styles,
                text,
                images,
            };
            cx.finalize();
        }
        let r = &self.boxes[self.root as usize];
        self.doc = (r.size[0].max(r.ink[2]), r.size[1].max(r.ink[3]));
        // Decoded images the display list needs.
        self.images.clear();
        for p in self.paint.iter() {
            for id in p.image_ids() {
                if let Some(img) = images.image(id) {
                    self.images.insert(id, img);
                }
            }
        }
    }

    /// Re-lays out after DOM or style changes. `dirty` are the nodes whose
    /// style, content or children changed (the same list given to
    /// `StyleEngine::recompute`). Falls back to a full rebuild when the box
    /// structure changed.
    pub fn relayout(
        &mut self,
        dom: &Dom,
        styles: &StyleTree,
        dirty: &[NodeId],
        text: &dyn TextMetrics,
        images: &dyn ImageSizes,
    ) {
        if self.boxes.is_empty() {
            return self.rebuild(dom, styles, text, images);
        }
        let mut owners: Vec<u32> = Vec::new();
        for &n in dirty {
            let mut cur = Some(n);
            let mut owner = NONE;
            while let Some(c) = cur {
                if let Some(b) = box_for(&self.box_of, c) {
                    owner = b;
                    break;
                }
                cur = dom.parent(c);
            }
            if owner == NONE {
                return self.rebuild(dom, styles, text, images);
            }
            let parent = self.boxes[owner as usize].parent;
            let mut ids = vec![self.boxes[owner as usize].dom];
            if parent != NONE {
                ids.push(self.boxes[parent as usize].dom);
            }
            for id in ids {
                if id == NONE {
                    continue;
                }
                if let Some(old) = self.sigs.get(&id) {
                    if sig_of(dom, styles, NodeId(id)) != *old {
                        return self.rebuild(dom, styles, text, images);
                    }
                }
            }
            owners.push(owner);
        }
        for o in owners {
            self.invalidate(o);
        }
        self.layout(dom, styles, text, images);
    }

    /// Drops cached layout of a box, its subtree and its ancestors.
    fn invalidate(&mut self, b: u32) {
        let mut stack = vec![b];
        while let Some(x) = stack.pop() {
            let _ = self.caches[x as usize].clear();
            let bx = &self.boxes[x as usize];
            if bx.ifc != NONE {
                self.ifcs[bx.ifc as usize].gathered = false;
            }
            let (k0, n) = (bx.kids as usize, (bx.nkids + bx.nabs) as usize);
            for i in 0..n {
                stack.push(self.kids[k0 + i]);
            }
        }
        let mut p = self.boxes[b as usize].parent;
        while p != NONE {
            let _ = self.caches[p as usize].clear();
            p = self.boxes[p as usize].parent;
        }
    }

    /// Changes the viewport and re-lays out (styles that depend on the
    /// viewport must have been recomputed by the caller).
    pub fn resize(
        &mut self,
        dom: &Dom,
        styles: &StyleTree,
        viewport: (f32, f32),
        text: &dyn TextMetrics,
        images: &dyn ImageSizes,
    ) {
        self.viewport = viewport;
        self.rebuild(dom, styles, text, images);
    }

    /// Height of the laid-out document.
    pub fn doc_height(&self) -> f32 {
        self.doc.1
    }

    /// Width of the laid-out document (scrollable overflow included).
    pub fn doc_width(&self) -> f32 {
        self.doc.0
    }

    pub fn viewport(&self) -> (f32, f32) {
        self.viewport
    }

    /// The border box of an element in page space (`getBoundingClientRect`
    /// at scroll 0). Inline elements without a box report their first
    /// fragment.
    pub fn node_rect(&self, n: NodeId) -> Option<Rect> {
        if let Some(b) = box_for(&self.box_of, n) {
            let bx = &self.boxes[b as usize];
            return Some(Rect::new(bx.abs[0], bx.abs[1], bx.size[0], bx.size[1]));
        }
        for (bi, bx) in self.boxes.iter().enumerate() {
            if bx.ifc == NONE {
                continue;
            }
            let d = &self.ifcs[bx.ifc as usize];
            for f in d.frags.iter() {
                if f.kind != inline::FragKind::Atomic
                    && tree::dec(f.node).0 == n
                    && tree::dec(f.node).1 == 0
                {
                    let ox = bx.abs[0] + bx.border[3] + bx.padding[3];
                    let oy = bx.abs[1] + bx.border[0] + bx.padding[0];
                    let _ = bi;
                    return Some(Rect::new(ox + f.x, oy + f.y, f.w, f.h));
                }
            }
        }
        None
    }

    /// Sets the scroll offset of a scroll container.
    pub fn set_scroll(&mut self, n: NodeId, offset: (f32, f32)) {
        if offset == (0.0, 0.0) {
            self.scrolls.remove(&n.0);
        } else {
            self.scrolls.insert(n.0, offset);
        }
    }

    pub fn scroll_of(&self, n: NodeId) -> (f32, f32) {
        self.scrolls.get(&n.0).copied().unwrap_or((0.0, 0.0))
    }

    /// Scrollable size (width, height) of an element, for clamping scrolls.
    pub fn scroll_extent(&self, n: NodeId) -> Option<(f32, f32)> {
        let b = box_for(&self.box_of, n)?;
        let bx = &self.boxes[b as usize];
        Some((
            bx.scroll_size[0].max(bx.size[0]),
            bx.scroll_size[1].max(bx.size[1]),
        ))
    }

    /// Number of boxes (for tests and memory accounting).
    pub fn box_count(&self) -> usize {
        self.boxes.len()
    }
}

// ---------------------------------------------------------------- taffy glue

/// The layout context: the tree plus the inputs layout reads. Implements
/// taffy's low-level traits.
pub(crate) struct Ctx<'a> {
    pub t: &'a mut LayoutTree,
    pub dom: &'a Dom,
    pub styles: &'a StyleTree,
    pub text: &'a dyn TextMetrics,
    pub images: &'a dyn ImageSizes,
}

pub struct KidIter<'a>(std::slice::Iter<'a, u32>);

impl Iterator for KidIter<'_> {
    type Item = tf::NodeId;
    fn next(&mut self) -> Option<tf::NodeId> {
        self.0.next().map(|k| tf::NodeId::from(*k as usize))
    }
}

fn idx(id: tf::NodeId) -> usize {
    usize::from(id)
}

impl<'c> TraversePartialTree for Ctx<'c> {
    type ChildIter<'a>
        = KidIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, p: tf::NodeId) -> KidIter<'_> {
        let bx = &self.t.boxes[idx(p)];
        let n = if matches!(bx.kind, Kind::Ifc | Kind::Replaced) {
            0
        } else {
            bx.nkids as usize
        };
        let s = bx.kids as usize;
        KidIter(self.t.kids[s..s + n].iter())
    }

    fn child_count(&self, p: tf::NodeId) -> usize {
        let bx = &self.t.boxes[idx(p)];
        if matches!(bx.kind, Kind::Ifc | Kind::Replaced) {
            0
        } else {
            bx.nkids as usize
        }
    }

    fn get_child_id(&self, p: tf::NodeId, i: usize) -> tf::NodeId {
        let bx = &self.t.boxes[idx(p)];
        tf::NodeId::from(self.t.kids[bx.kids as usize + i] as usize)
    }
}

impl<'c> LayoutPartialTree for Ctx<'c> {
    type CoreContainerStyle<'a>
        = tf::Style
    where
        Self: 'a;

    type CustomIdent = String;

    fn get_core_container_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }

    fn set_unrounded_layout(&mut self, id: tf::NodeId, l: &tf::Layout) {
        let bx = &mut self.t.boxes[idx(id)];
        bx.loc = [l.location.x, l.location.y];
        bx.size = [l.size.width, l.size.height];
        bx.scroll_size = [
            l.scrollable_overflow_rect.right,
            l.scrollable_overflow_rect.bottom,
        ];
        bx.border = [l.border.top, l.border.right, l.border.bottom, l.border.left];
        bx.padding = [
            l.padding.top,
            l.padding.right,
            l.padding.bottom,
            l.padding.left,
        ];
    }

    fn compute_child_layout(
        &mut self,
        id: tf::NodeId,
        inputs: tf::LayoutInput,
    ) -> tf::LayoutOutput {
        {
            let bx = &mut self.t.boxes[idx(id)];
            if let Some(w) = inputs.parent_size.width {
                if (bx.pw[0] - w).abs() > 0.5 {
                    bx.pw[0] = w;
                    self.t.pw_changed = true;
                }
            }
            if let Some(h) = inputs.parent_size.height {
                if bx.pw[1].is_nan() || (bx.pw[1] - h).abs() > 0.5 {
                    bx.pw[1] = h;
                    self.t.pw_changed = true;
                }
            }
        }
        tf::compute_cached_layout(self, id, inputs, |cx, id, inputs| {
            let b = idx(id) as u32;
            match cx.t.boxes[b as usize].kind {
                Kind::Block => tf::compute_block_layout(cx, id, inputs, None),
                Kind::Flex => tf::compute_flexbox_layout(cx, id, inputs),
                Kind::Grid => tf::compute_grid_layout(cx, id, inputs),
                Kind::Ifc => cx.layout_ifc_leaf(b, inputs),
                Kind::Replaced => cx.layout_replaced_leaf(b, inputs),
            }
        })
    }
}

impl<'c> CacheTree for Ctx<'c> {
    fn cache_get(&mut self, id: tf::NodeId, input: &tf::LayoutInput) -> Option<tf::LayoutOutput> {
        self.t.caches[idx(id)].get(input)
    }

    fn cache_store(&mut self, id: tf::NodeId, input: &tf::LayoutInput, out: tf::LayoutOutput) {
        self.t.caches[idx(id)].store(input, out)
    }

    fn cache_clear(&mut self, id: tf::NodeId) {
        let _ = self.t.caches[idx(id)].clear();
    }
}

impl<'c> LayoutFlexboxContainer for Ctx<'c> {
    type FlexboxContainerStyle<'a>
        = tf::Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = tf::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }

    fn get_flexbox_child_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }
}

impl<'c> LayoutGridContainer for Ctx<'c> {
    type GridContainerStyle<'a>
        = tf::Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = tf::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }

    fn get_grid_child_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }
}

impl<'c> LayoutBlockContainer for Ctx<'c> {
    type BlockContainerStyle<'a>
        = tf::Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = tf::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }

    fn get_block_child_style(&self, id: tf::NodeId) -> tf::Style {
        self.style_for(idx(id) as u32)
    }
}

impl<'a> Ctx<'a> {
    /// Leaf layout of an inline formatting root: the measure function
    /// breaks lines at the offered width.
    fn layout_ifc_leaf(&mut self, b: u32, inputs: tf::LayoutInput) -> tf::LayoutOutput {
        let style = self.style_for(b);
        let mut base: Option<f32> = None;
        let mut out = tf::compute_leaf_layout(
            inputs,
            &style,
            |_v, _b| 0.0,
            |_known, avail| {
                let wrap = match avail.width {
                    tf::AvailableSpace::Definite(w) => Wrap::At(w),
                    tf::AvailableSpace::MinContent => Wrap::At(0.0),
                    tf::AvailableSpace::MaxContent => Wrap::None,
                };
                let r = self.run_lines(b, wrap, false);
                base = if r.height > 0.0 {
                    Some(r.baseline)
                } else {
                    None
                };
                tf::Size {
                    width: r.width,
                    height: r.height,
                }
            },
        );
        if let Some(bs) = base {
            let basis = self.t.boxes[b as usize].pw[0];
            let (border, padding) = self.insets(b, basis);
            out.baselines.first = Some(bs + border[0] + padding[0]);
        }
        out
    }

    /// Leaf layout of a replaced element: its intrinsic size.
    fn layout_replaced_leaf(&mut self, b: u32, inputs: tf::LayoutInput) -> tf::LayoutOutput {
        let style = self.style_for(b);
        let intr = self.t.boxes[b as usize].intr;
        let mut out = tf::compute_leaf_layout(
            inputs,
            &style,
            |_v, _b| 0.0,
            |known, _avail| {
                let (iw, ih) = (intr[0], intr[1]);
                let (w, h) = match (known.width, known.height) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => (w, if iw > 0.0 { w * ih / iw } else { ih }),
                    (None, Some(h)) => (if ih > 0.0 { h * iw / ih } else { iw }, h),
                    (None, None) => (iw, ih),
                };
                tf::Size {
                    width: w,
                    height: h,
                }
            },
        );
        // The leaf only grows a missing dimension from the ratio; replaced
        // elements derive it: `height: auto` follows the (possibly clamped) width.
        if intr[0] > 0.0 && intr[1] > 0.0 {
            let ratio = intr[0] / intr[1];
            let basis = self.t.boxes[b as usize].pw[0];
            let (border, padding) = self.insets(b, if basis.is_nan() { 0.0 } else { basis });
            let pbx = border[1] + border[3] + padding[1] + padding[3];
            let pby = border[0] + border[2] + padding[0] + padding[2];
            let h_free = inputs.known_dimensions.height.is_none() && style.size.height.is_auto();
            let w_free = inputs.known_dimensions.width.is_none() && style.size.width.is_auto();
            if h_free {
                out.size.height = ((out.size.width - pbx) / ratio).max(0.0) + pby;
            } else if w_free {
                out.size.width = ((out.size.height - pby) * ratio).max(0.0) + pbx;
            }
        }
        out
    }
}
