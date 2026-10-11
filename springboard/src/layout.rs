//! bifrost 15 home-screen geometry. Measurements are in bifrost points (bifrost mini
//! portrait is 768pt wide) and scaled so that the short side fills the
//! screen: 1.5625 px per point in both orientations. Portrait pages hold
//! 5 x 6 icons, landscape pages 6 x 5, so a page keeps its icons when the
//! device turns.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && y >= self.y as f32 && x < (self.x + self.w) as f32 && y < (self.y + self.h) as f32
    }
}

/// Whether the point (`x`, `y`) lies inside the `(x, y, w, h)` box: left and top edges
/// count, right and bottom edges do not.
pub fn in_rect(x: f32, y: f32, (rx, ry, rw, rh): (f32, f32, f32, f32)) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// Portrait grid.
pub const COLS: usize = 5;
pub const ROWS: usize = 6;
/// Landscape grid: the same icons per page, one column more, one row fewer.
pub const LAND_COLS: usize = 6;
pub const LAND_ROWS: usize = 5;
pub const PER_PAGE: usize = COLS * ROWS;

// Measured on an bifrost mini (bifrost 15.7.8) home-screen capture at 2x.
const bifrost_WIDTH_PT: f32 = 768.0;
const STATUS_PT: f32 = 20.0;
const ICON_PT: f32 = 64.0;
/// 5 columns on a 111 pt pitch, centred
const SIDE_INSET_PT: f32 = 106.5;
/// first icon row starts 68 pt from the top (bifrost-ref/screens/home.png)
const GRID_TOP_PT: f32 = 48.0;
/// row pitch of the reference: 140 pt, not stretched to the screen height
const ROW_PITCH_PT: f32 = 140.0;
/// landscape: the grid starts higher and the columns spread to the width
const LAND_GRID_TOP_PT: f32 = 30.0;
/// landscape side inset as a share of the width (98 pt of the bifrost's 1024)
const LAND_SIDE_INSET: f32 = 98.0 / 1024.0;
const LABEL_GAP_PT: f32 = 5.0;
const LABEL_PT: f32 = 16.0;
const DOCK_PAD_X_PT: f32 = 17.0;
const DOCK_PAD_Y_PT: f32 = 17.5;
/// extra space the separator adds between the pinned and the recent icons
const DOCK_SEP_EXTRA_PT: f32 = 19.0;
const DOCK_GAP_PT: f32 = 18.0;
/// the landscape dock spreads its icons a little wider
const LAND_DOCK_GAP_PT: f32 = 26.0;
const DOCK_MARGIN_PT: f32 = 19.0;
pub const DOCK_RADIUS_PT: f32 = 24.0;
const DOTS_ABOVE_DOCK_PT: f32 = 22.0;
const DOT_PT: f32 = 7.0;
const DOT_PITCH_PT: f32 = 16.0;

/// Adaptive (desktop) pt -> px scale chosen for the panel; 0 on the bifrost, which always
/// derives its scale from the short side so its numbers never change.
static AUTO_SCALE: AtomicU32 = AtomicU32::new(0);
/// The scale picked in Settings > Display; 0 follows the panel (Auto).
static USER_SCALE: AtomicU32 = AtomicU32::new(0);

/// The scales Settings > Display offers on a desktop, besides Auto.
pub const SCALE_CHOICES: [f32; 8] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const SCALE_MIN: f32 = 0.5;
const SCALE_MAX: f32 = 4.0;
/// the shell needs at least this many points on the short side (rows of icons, dock, bars)
const MIN_SHORT_PT: f32 = 480.0;
/// one point is 1/96 in at scale 1.0
const REFERENCE_DPI: f32 = 96.0;

/// The scale a desktop panel gets without a user choice: from the physical width in mm (EDID,
/// via KMS) when the panel reports a sane size, else from the short side (1080 px = 1.0);
/// in steps of 0.25 and never so large that the short side drops below `MIN_SHORT_PT`.
pub fn auto_scale(w: i32, h: i32, size_mm: Option<(u32, u32)>) -> f32 {
    let short = w.min(h).max(1) as f32;
    let by_dpi = size_mm.filter(|&(mw, mh)| mw >= 50 && mh >= 50).map(|(mw, mh)| {
        let (px, mm) = if w >= h { (w, mw.max(mh)) } else { (h, mw.max(mh)) };
        px as f32 / (mm as f32 / 25.4) / REFERENCE_DPI
    });
    let raw = by_dpi.unwrap_or(short / 1080.0);
    ((raw * 4.0).round() / 4.0).clamp(SCALE_MIN, SCALE_MAX).min(short / MIN_SHORT_PT)
}

/// Switches the shell to the desktop's adaptive scaling with `auto` as the panel's own scale.
pub fn set_adaptive(auto: f32) {
    AUTO_SCALE.store(auto.clamp(SCALE_MIN, SCALE_MAX).to_bits(), Relaxed);
}

/// Whether the desktop's panel-derived layout is in force (always false on the bifrost).
pub fn adaptive() -> bool {
    AUTO_SCALE.load(Relaxed) != 0
}

/// The user's Settings > Display scale; None is Auto.
pub fn set_user_scale(s: Option<f32>) {
    USER_SCALE.store(s.map_or(0, |s| s.clamp(SCALE_MIN, SCALE_MAX).to_bits()), Relaxed);
}

pub fn user_scale() -> Option<f32> {
    Some(f32::from_bits(USER_SCALE.load(Relaxed))).filter(|&s| s > 0.0)
}

/// Pixels per point for a `w` x `h` frame: the bifrost's short side over 768 pt, or on the
/// desktop the user's choice (else the panel's auto scale), kept small enough to fit.
pub fn base_scale(w: i32, h: i32) -> f32 {
    if !adaptive() {
        return w.min(h) as f32 / bifrost_WIDTH_PT;
    }
    let auto = f32::from_bits(AUTO_SCALE.load(Relaxed));
    user_scale().unwrap_or(auto).min(w.min(h) as f32 / MIN_SHORT_PT)
}

/// Columns of a wide (desktop landscape) page: as many 150 pt columns as fit, 6 to 10.
fn wide_cols(w: i32, s: f32) -> usize {
    (((w as f32 / s) * (1.0 - 2.0 * LAND_SIDE_INSET) / 150.0) as usize).clamp(LAND_COLS, 10)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub w: i32,
    pub h: i32,
    /// pixels per bifrost point
    pub s: f32,
    /// grid columns on a page (rows follow from PER_PAGE)
    pub cols: usize,
    pub icon: i32,
    pub status_h: i32,
    /// icon rects for one page, page-relative (page 0 at x = 0)
    pub cells: Vec<Rect>,
    pub label_w: i32,
    pub label_gap: i32,
    pub dock: Rect,
    pub dock_icons: Vec<Rect>,
    /// dock icons before the thin separator of the recent apps (0 = none)
    pub dock_sep: usize,
    pub dots_y: i32,
    pub dot: i32,
    pub dot_pitch: i32,
}

impl Layout {
    pub fn new(w: i32, h: i32, dock_count: usize) -> Self {
        Self::zoomed(w, h, dock_count, 1.0)
    }

    /// The layout under Display Zoom: icons, labels, dock padding and dots
    /// grow by `zoom` while `s` and the column pitch still follow the short side.
    /// At 1.0 every product is exact, so it equals the standard layout.
    pub fn zoomed(w: i32, h: i32, dock_count: usize, zoom: f32) -> Self {
        Self::build(w, h, dock_count, zoom, base_scale(w, h), adaptive())
    }

    /// `zoomed` at an explicit scale `s`; `wide` spreads a landscape page over more columns
    /// and fewer rows (the desktop) instead of the bifrost's 6 x 5 grid.
    pub fn build(w: i32, h: i32, dock_count: usize, zoom: f32, s: f32, wide: bool) -> Self {
        let landscape = w > h;
        let px = |pt: f32| (pt * s).round() as i32;
        let zpx = |pt: f32| (pt * zoom * s).round() as i32;
        let icon = zpx(ICON_PT);
        let (cols, rows) = match (landscape, wide) {
            (true, true) => {
                let c = wide_cols(w, s);
                (c, PER_PAGE.div_ceil(c))
            }
            (true, false) => (LAND_COLS, LAND_ROWS),
            _ => (COLS, ROWS),
        };
        let dock_gap = zpx(if landscape { LAND_DOCK_GAP_PT } else { DOCK_GAP_PT });

        let dock_h = icon + 2 * zpx(DOCK_PAD_Y_PT);
        let dock_w = if dock_count == 0 {
            0
        } else {
            dock_count as i32 * icon + (dock_count as i32 - 1) * dock_gap + 2 * zpx(DOCK_PAD_X_PT)
        };
        let dock = Rect { x: (w - dock_w) / 2, y: h - px(DOCK_MARGIN_PT) - dock_h, w: dock_w, h: dock_h };
        let dock_icons = (0..dock_count as i32)
            .map(|i| Rect {
                x: dock.x + zpx(DOCK_PAD_X_PT) + i * (icon + dock_gap),
                y: dock.y + zpx(DOCK_PAD_Y_PT),
                w: icon,
                h: icon,
            })
            .collect();

        let dots_y = dock.y - px(DOTS_ABOVE_DOCK_PT);
        let status_h = px(STATUS_PT);
        let top = status_h + px(if landscape { LAND_GRID_TOP_PT } else { GRID_TOP_PT });
        let cell_h = icon + zpx(LABEL_GAP_PT) + zpx(LABEL_PT);
        let bottom = dots_y - zpx(DOT_PT) - zpx(LABEL_PT);
        let inset = if landscape { LAND_SIDE_INSET * w as f32 } else { SIDE_INSET_PT * s };
        let col_pitch = (w as f32 - 2.0 * inset) / cols as f32;
        // the bifrost keeps the reference's 140 pt row pitch; a wide page lets rows follow the columns
        let pitch_cap = if wide && landscape { (ROW_PITCH_PT * zoom * s).max(col_pitch * 0.75) } else { ROW_PITCH_PT * zoom * s };
        let row_pitch = ((bottom - top - cell_h) as f32 / (rows - 1) as f32).min(pitch_cap);
        let label_w = ((col_pitch - 8.0 * s) as i32).max(icon);
        let cells = (0..PER_PAGE)
            .map(|i| {
                let (c, r) = ((i % cols) as f32, (i / cols) as f32);
                let cx = inset + col_pitch * (c + 0.5);
                Rect { x: (cx - icon as f32 / 2.0).round() as i32, y: top + (row_pitch * r).round() as i32, w: icon, h: icon }
            })
            .collect();

        Layout {
            w,
            h,
            s,
            cols,
            icon,
            status_h,
            cells,
            label_w,
            label_gap: zpx(LABEL_GAP_PT),
            dock,
            dock_icons,
            dock_sep: 0,
            dots_y,
            dot: zpx(DOT_PT),
            dot_pitch: px(DOT_PITCH_PT),
        }
    }

    /// Marks the separator that follows the first `n` dock icons.
    pub fn with_dock_sep(mut self, n: usize) -> Self {
        self.dock_sep = if n < self.dock_icons.len() { n } else { 0 };
        if self.dock_sep > 0 {
            let extra = (self.icon as f32 * DOCK_SEP_EXTRA_PT / ICON_PT).round() as i32;
            let x = (self.w - (self.dock.w + extra)) / 2;
            let shift = x - self.dock.x;
            for (i, r) in self.dock_icons.iter_mut().enumerate() {
                r.x += shift + if i >= n { extra } else { 0 };
            }
            self.dock.x = x;
            self.dock.w += extra;
        }
        self
    }

    pub fn pt(&self, pt: f32) -> f32 {
        pt * self.s
    }

    pub fn pages(apps_on_grid: usize) -> usize {
        apps_on_grid.div_ceil(PER_PAGE).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i32 = 1200;
    const H: i32 = 1920;

    #[test]
    fn icons_scale_with_width() {
        let l = Layout::new(W, H, 4);
        assert_eq!(l.icon, 100); // 64pt at 1.5625 px/pt
        assert_eq!(l.cells.len(), PER_PAGE);
        assert_eq!(l.status_h, 31); // 20pt
        assert_eq!(l.cells[0].y, 106); // 68pt
        assert_eq!(l.cells[COLS].y - l.cells[0].y, 219); // 140pt row pitch
        assert!((173..=174).contains(&(l.cells[1].x - l.cells[0].x))); // 111pt pitch
    }

    #[test]
    fn zoom_one_is_the_standard_layout_and_zoom_grows_icons() {
        assert_eq!(Layout::zoomed(W, H, 4, 1.0), Layout::new(W, H, 4));
        let z = Layout::zoomed(W, H, 4, 1.15);
        assert!(z.icon > Layout::new(W, H, 4).icon);
        assert_eq!(z.s, Layout::new(W, H, 4).s);
        for c in &z.cells {
            assert!(c.x >= 0 && c.x + c.w <= W);
        }
    }

    #[test]
    fn grid_is_inside_screen_and_above_dots() {
        let l = Layout::new(W, H, 4);
        for c in &l.cells {
            assert!(c.x >= 0 && c.x + c.w <= W);
            assert!(c.y > l.status_h);
            assert!(c.y + c.h + l.label_gap + l.pt(16.0) as i32 <= l.dots_y - l.dot);
        }
    }

    #[test]
    fn grid_cells_do_not_overlap_and_columns_are_symmetric() {
        let l = Layout::new(W, H, 0);
        for (i, a) in l.cells.iter().enumerate() {
            for b in &l.cells[i + 1..] {
                let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        let left = l.cells[0].x;
        let right = W - (l.cells[COLS - 1].x + l.icon);
        assert!((left - right).abs() <= 1);
    }

    #[test]
    fn dock_is_centred_and_sized_to_content() {
        let l = Layout::new(W, H, 3);
        assert_eq!(l.dock_icons.len(), 3);
        assert!(((l.dock.x) - (W - l.dock.x - l.dock.w)).abs() <= 1);
        assert!(l.dock.y + l.dock.h < H);
        for r in &l.dock_icons {
            assert!(r.x > l.dock.x && r.x + r.w < l.dock.x + l.dock.w);
            assert!(r.y > l.dock.y && r.y + r.h < l.dock.y + l.dock.h);
        }
        assert!(Layout::new(W, H, 5).dock.w > l.dock.w);
        assert_eq!(Layout::new(W, H, 0).dock.w, 0);
    }

    #[test]
    fn landscape_keeps_the_scale_and_fills_six_columns_by_five_rows() {
        let p = Layout::new(W, H, 3);
        let l = Layout::new(H, W, 3);
        assert_eq!(l.s, p.s, "points stay the same size when the device turns");
        assert_eq!((l.icon, l.status_h), (p.icon, p.status_h));
        assert_eq!((l.cols, p.cols), (LAND_COLS, COLS));
        assert_eq!(l.cells.len(), PER_PAGE, "a page holds the same icons either way");
        let xs: std::collections::BTreeSet<i32> = l.cells.iter().map(|c| c.x).collect();
        let ys: std::collections::BTreeSet<i32> = l.cells.iter().map(|c| c.y).collect();
        assert_eq!((xs.len(), ys.len()), (LAND_COLS, LAND_ROWS));
        for c in &l.cells {
            assert!(c.x >= 0 && c.x + c.w <= H, "{c:?}");
            assert!(c.y > l.status_h);
            assert!(c.y + c.h + l.label_gap + l.pt(16.0) as i32 <= l.dots_y - l.dot, "{c:?} clears the dots");
        }
        let left = l.cells[0].x;
        let right = H - (l.cells[LAND_COLS - 1].x + l.icon);
        assert!((left - right).abs() <= 1, "columns are centred");
        assert!(l.cells[1].x - l.cells[0].x > p.cells[1].x - p.cells[0].x, "wider column pitch");
        assert!(l.cells[LAND_COLS].y - l.cells[0].y < p.cells[COLS].y - p.cells[0].y, "tighter row pitch");
        assert!(l.dock.w > p.dock.w, "the landscape dock is wider");
        assert!(((l.dock.x) - (H - l.dock.x - l.dock.w)).abs() <= 1);
        assert_eq!(l.dock.y + l.dock.h + l.pt(19.0).round() as i32, W);
        assert!(l.label_w > p.label_w);
    }

    #[test]
    fn bifrost_scale_is_the_short_side_over_768() {
        assert!(!adaptive());
        assert_eq!(base_scale(W, H), 1200.0 / 768.0);
        assert_eq!(Layout::zoomed(W, H, 4, 1.0), Layout::build(W, H, 4, 1.0, 1200.0 / 768.0, false));
    }

    #[test]
    fn auto_scale_follows_dpi_then_height() {
        assert_eq!(auto_scale(1920, 1080, None), 1.0);
        assert_eq!(auto_scale(3840, 2160, None), 2.0);
        assert_eq!(auto_scale(2560, 1440, None), 1.25);
        assert_eq!(auto_scale(1280, 800, None), 0.75);
        // a 27 in 4K panel (597 x 336 mm) is 163 dpi
        assert_eq!(auto_scale(3840, 2160, Some((597, 336))), 1.75);
        // a bogus EDID size falls back to the height
        assert_eq!(auto_scale(1920, 1080, Some((0, 0))), 1.0);
    }

    #[test]
    fn wide_pages_use_more_columns_and_stay_on_screen() {
        for (w, h) in [(1920, 1080), (1280, 800), (2560, 1440), (3840, 2160), (3440, 1440)] {
            let s = auto_scale(w, h, None);
            let l = Layout::build(w, h, 4, 1.0, s, true);
            assert_eq!(l.cells.len(), PER_PAGE);
            assert!((LAND_COLS..=10).contains(&l.cols), "{w}x{h}: {} columns", l.cols);
            for c in &l.cells {
                assert!(c.x >= 0 && c.x + c.w <= w && c.y > l.status_h, "{w}x{h}: {c:?}");
                assert!(c.y + c.h + l.label_gap <= l.dots_y, "{w}x{h}: {c:?} clears the dots");
            }
        }
    }

    #[test]
    fn page_count() {
        assert_eq!(Layout::pages(0), 1);
        assert_eq!(Layout::pages(PER_PAGE), 1);
        assert_eq!(Layout::pages(PER_PAGE + 1), 2);
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect { x: 10, y: 10, w: 5, h: 5 };
        assert!(r.contains(10.0, 14.9));
        assert!(!r.contains(15.0, 12.0));
    }
}
