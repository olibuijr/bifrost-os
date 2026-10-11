//! A card grid (`NativeCollectionView` with a compositional layout): adaptive columns from a minimum item width, an
//! optional full-width hero item first (News Top Stories), fixed-height rows. Pure model: frames and hit-testing.
use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardGrid {
    /// Narrowest an item may get before a column is dropped (`NativeCollectionLayoutDimension.estimated`).
    pub min_item_w: f32,
    /// Space between items and rows (`interItemSpacing`, `interGroupSpacing`).
    pub spacing: f32,
    /// Most columns, however wide the container (0 = no limit).
    pub max_columns: usize,
    pub item_h: f32,
    /// Height of a full-width first item; `None` when every item is a card.
    pub hero_h: Option<f32>,
}

impl CardGrid {
    pub fn new(min_item_w: f32, item_h: f32, spacing: f32) -> Self {
        CardGrid { min_item_w, spacing, max_columns: 0, item_h, hero_h: None }
    }
    /// The first item spans the width at `h` points.
    pub fn with_hero(mut self, h: f32) -> Self {
        self.hero_h = Some(h);
        self
    }
    pub fn with_max_columns(mut self, n: usize) -> Self {
        self.max_columns = n;
        self
    }
    /// Columns that fit `width` (at least one).
    pub fn columns(&self, width: f32) -> usize {
        let n = (((width + self.spacing) / (self.min_item_w + self.spacing).max(1.0)).floor() as usize).max(1);
        if self.max_columns > 0 { n.min(self.max_columns) } else { n }
    }
    /// Frames of `count` items laid out in `bounds` (its height is ignored), scrolled up by `scroll`.
    pub fn frames(&self, bounds: Rect, count: usize, scroll: f32) -> Vec<Rect> {
        let cols = self.columns(bounds.w);
        let w = ((bounds.w - self.spacing * (cols - 1) as f32) / cols as f32).max(0.0);
        let mut out = Vec::with_capacity(count);
        let mut y = bounds.y - scroll;
        let mut first = 0;
        if let (Some(h), true) = (self.hero_h, count > 0) {
            out.push(Rect { x: bounds.x, y, w: bounds.w, h });
            y += h + self.spacing;
            first = 1;
        }
        for k in 0..count - first {
            let (row, col) = (k / cols, k % cols);
            out.push(Rect { x: bounds.x + col as f32 * (w + self.spacing), y: y + row as f32 * (self.item_h + self.spacing), w, h: self.item_h });
        }
        out
    }
    /// Total height of `count` items in a container `width` wide (the scrollable content size).
    pub fn content_height(&self, width: f32, count: usize) -> f32 {
        let hero = match (self.hero_h, count > 0) {
            (Some(h), true) => h + self.spacing,
            _ => 0.0,
        };
        let rest = count - usize::from(hero > 0.0);
        let rows = rest.div_ceil(self.columns(width));
        (hero + rows as f32 * (self.item_h + self.spacing) - if rows > 0 || hero > 0.0 { self.spacing } else { 0.0 }).max(0.0)
    }
    /// The item index under `(x, y)`.
    pub fn hit(&self, bounds: Rect, count: usize, scroll: f32, x: f32, y: f32) -> Option<usize> {
        self.frames(bounds, count, scroll).iter().position(|r| r.contains(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 10.0, y: 20.0, w: 620.0, h: 400.0 };

    #[test]
    fn columns_follow_the_width_and_the_cap() {
        let g = CardGrid::new(200.0, 150.0, 10.0);
        assert_eq!(g.columns(620.0), 3);
        assert_eq!(g.columns(409.0), 1, "two need 410");
        assert_eq!(g.columns(50.0), 1);
        assert_eq!(g.with_max_columns(2).columns(2000.0), 2);
    }

    #[test]
    fn a_hero_spans_the_width_and_cards_fill_rows_below_it() {
        let g = CardGrid::new(300.0, 100.0, 20.0).with_hero(240.0);
        let f = g.frames(B, 4, 0.0);
        assert_eq!(f[0], Rect { x: 10.0, y: 20.0, w: 620.0, h: 240.0 });
        assert_eq!(f[1], Rect { x: 10.0, y: 280.0, w: 300.0, h: 100.0 });
        assert_eq!(f[2].x, 330.0);
        assert_eq!(f[3].y, 400.0, "third card wraps to a second row");
        assert_eq!(g.content_height(620.0, 4), 240.0 + 20.0 + 100.0 + 20.0 + 100.0);
        assert_eq!(g.hit(B, 4, 0.0, 400.0, 300.0), Some(2));
        assert_eq!(g.hit(B, 4, 0.0, 320.0, 300.0), None, "the gutter");
        assert_eq!(g.content_height(620.0, 0), 0.0);
        assert!(g.frames(B, 0, 0.0).is_empty());
    }
}
