//! Layered and fitting containers over the pure layout models in `bifrost_ui::stack`: `ZStack` (children share one
//! area, back to front, each aligned to an anchor), `ViewThatFits` (shows the first candidate that fits the offered
//! rectangle) and `Grid` (equal cells in rows, fixed or adaptive column count). Grid is not lazy: every cell is laid out.
use super::{View, ViewBase};
use crate::kit::Ui;
use bifrost_ui::geometry::EdgeInsets;
use bifrost_ui::stack::{adaptive_columns, first_fitting, grid, overlay, Anchor};
use bifrost_ui::Rect;

/// Intrinsic size of a child, with a zero side replaced by `fill` (a child with no preference takes the offered space).
fn sized(ui: &Ui, c: &dyn View, fill: (f32, f32)) -> (f32, f32) {
    let (w, h) = c.intrinsic(ui);
    (if w > 0.0 { w } else { fill.0 }, if h > 0.0 { h } else { fill.1 })
}

/// `ZStack`: lays every child over the same area, back to front in the order given, aligned to `anchor`. A child
/// with an intrinsic size keeps it (clamped to the area); a child without one fills the area.
pub struct ZStack {
    base: ViewBase,
    pub insets: EdgeInsets,
    pub anchor: Anchor,
    children: Vec<Box<dyn View>>,
}

impl ZStack {
    pub fn new(children: Vec<Box<dyn View>>) -> Self {
        ZStack { base: ViewBase::default(), insets: EdgeInsets::ZERO, anchor: Anchor::Center, children }
    }
    /// `ZStack(alignment:)`.
    pub fn anchored(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }
    pub fn inset(mut self, insets: EdgeInsets) -> Self {
        self.insets = insets;
        self
    }
}

impl View for ZStack {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        self.children.iter().map(|c| c.intrinsic(ui)).fold((0.0, 0.0), |a, b| (a.0.max(b.0), a.1.max(b.1)))
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let area = bounds.inset(self.insets);
        let sizes: Vec<(f32, f32)> = self.children.iter().map(|c| sized(ui, c.as_ref(), (area.w, area.h))).collect();
        let rects = overlay(bounds, self.insets, self.anchor, &sizes);
        for (c, r) in self.children.iter_mut().zip(rects) {
            c.layout(ui, r);
        }
    }
}

/// `ViewThatFits`: offers the whole bounds to the candidates in order and shows the first one whose intrinsic size fits
/// the constrained axes (`axes` is `(horizontal, vertical)`, both by default). The others are hidden. When none fits
/// the last candidate is shown, as SwiftUI does for its fallback.
pub struct ViewThatFits {
    base: ViewBase,
    pub axes: (bool, bool),
    children: Vec<Box<dyn View>>,
}

impl ViewThatFits {
    pub fn new(children: Vec<Box<dyn View>>) -> Self {
        ViewThatFits { base: ViewBase::default(), axes: (true, true), children }
    }
    /// Which axes the candidates must fit (`in: .horizontal` and so on).
    pub fn constrained(mut self, horizontal: bool, vertical: bool) -> Self {
        self.axes = (horizontal, vertical);
        self
    }
    /// Index of the candidate currently shown, or `None` before any layout with candidates.
    pub fn shown(&self) -> Option<usize> {
        self.children.iter().position(|c| !c.base().hidden)
    }
}

impl View for ViewThatFits {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        self.children.iter().find(|c| !c.base().hidden).map_or((0.0, 0.0), |c| c.intrinsic(ui))
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let sizes: Vec<(f32, f32)> = self.children.iter().map(|c| c.intrinsic(ui)).collect();
        let pick = first_fitting((bounds.w, bounds.h), &sizes, self.axes).unwrap_or(sizes.len().saturating_sub(1));
        for (i, c) in self.children.iter_mut().enumerate() {
            let hidden = i != pick;
            if c.base().hidden != hidden {
                c.base_mut().hidden = hidden;
                c.base_mut().dirty = true;
            }
            if !hidden {
                c.layout(ui, bounds);
            }
        }
    }
}

/// How many columns a [`Grid`] has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Columns {
    /// A fixed count (at least 1).
    Fixed(usize),
    /// As many columns of at least `min_cell` points as fit (`GridItem(.adaptive(minimum:))`).
    Adaptive { min_cell: f32 },
}

/// `LazyVGrid` without the laziness: children fill rows left to right in equal cells of `row_height` points. A zero
/// row height uses the tallest child's intrinsic height.
pub struct Grid {
    base: ViewBase,
    pub columns: Columns,
    pub row_height: f32,
    pub spacing_x: f32,
    pub spacing_y: f32,
    pub insets: EdgeInsets,
    children: Vec<Box<dyn View>>,
}

impl Grid {
    pub fn new(columns: Columns, row_height: f32, spacing: f32, children: Vec<Box<dyn View>>) -> Self {
        Grid { base: ViewBase::default(), columns, row_height, spacing_x: spacing, spacing_y: spacing, insets: EdgeInsets::ZERO, children }
    }
    pub fn inset(mut self, insets: EdgeInsets) -> Self {
        self.insets = insets;
        self
    }
    /// Column count for an area of `width` points.
    pub fn column_count(&self, width: f32) -> usize {
        match self.columns {
            Columns::Fixed(n) => n.max(1),
            Columns::Adaptive { min_cell } => adaptive_columns(width, min_cell, self.spacing_x),
        }
    }
}

impl View for Grid {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let area = bounds.inset(self.insets);
        let cols = self.column_count(area.w);
        let row_h = if self.row_height > 0.0 { self.row_height } else { self.children.iter().map(|c| c.intrinsic(ui).1).fold(0.0, f32::max) };
        let cells = grid(area, cols, self.spacing_x, self.spacing_y, row_h, self.children.len());
        for (c, r) in self.children.iter_mut().zip(cells) {
            c.layout(ui, r);
        }
    }
}
