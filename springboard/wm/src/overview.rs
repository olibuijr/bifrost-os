//! The overview: every window on the output as a thumbnail in a grid, with the desktops in a strip along the top, and
//! the app overview, the same grid limited to one app's windows. Pure layout; the shell draws the thumbnails into the
//! frames it gets back and hit-tests them the same way.

use crate::wm::{Rect, WindowId};

/// Height of the desktop strip along the top, px.
pub const STRIP_H: i32 = 104;
/// Space between grid cells and around the grid, px.
pub const GUTTER: i32 = 24;
/// Size of one desktop tile in the strip, px (16:10).
pub const DESKTOP_W: i32 = 128;
pub const DESKTOP_H: i32 = 80;

/// The overview of one output: the desktop tiles and each window's thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub desktops: Vec<Rect>,
    pub windows: Vec<(WindowId, Rect)>,
}

/// Grid cells for `count` items inside `area`: the columns are the smallest square-ish count that holds them, and the
/// last row is centred. Empty for no items.
pub fn cells(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let mut cols = 1;
    while cols * cols < n {
        cols += 1;
    }
    let rows = (n + cols - 1) / cols;
    let cw = ((area.w - GUTTER * (cols + 1)) / cols).max(1);
    let ch = ((area.h - GUTTER * (rows + 1)) / rows).max(1);
    (0..n)
        .map(|i| {
            let (c, r) = (i % cols, i / cols);
            let in_row = if r == rows - 1 { n - r * cols } else { cols };
            let shift = (cols - in_row) * (cw + GUTTER) / 2;
            Rect::new(area.x + GUTTER + shift + c * (cw + GUTTER), area.y + GUTTER + r * (ch + GUTTER), cw, ch)
        })
        .collect()
}

/// `src` fitted into `cell` with its aspect kept, centred.
pub fn fit(src: Rect, cell: Rect) -> Rect {
    let sw = src.w.max(1) as i64;
    let sh = src.h.max(1) as i64;
    let (cw, ch) = (cell.w as i64, cell.h as i64);
    let (w, h) = if cw * sh <= ch * sw { (cw, cw * sh / sw) } else { (ch * sw / sh, ch) };
    let (w, h) = (w as i32, h as i32);
    Rect::new(cell.x + (cell.w - w) / 2, cell.y + (cell.h - h) / 2, w, h)
}

/// The desktop tiles along the top of `area`, centred, one per desktop.
pub fn desktop_strip(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let total = n * DESKTOP_W + (n - 1) * GUTTER;
    let x0 = area.x + (area.w - total) / 2;
    let y = area.y + (STRIP_H - DESKTOP_H) / 2;
    (0..n).map(|i| Rect::new(x0 + i * (DESKTOP_W + GUTTER), y, DESKTOP_W, DESKTOP_H)).collect()
}

/// The whole overview: the desktop strip on top and the windows' thumbnails in the grid below it. `windows` holds each
/// window's frame, which the thumbnail keeps the shape of.
pub fn overview(area: Rect, desktops: usize, windows: &[(WindowId, Rect)]) -> Overview {
    let body = Rect::new(area.x, area.y + STRIP_H, area.w, (area.h - STRIP_H).max(1));
    Overview { desktops: desktop_strip(area, desktops), windows: grid(body, windows) }
}

/// The app overview: the windows of one app (`app` matches the tag of its windows) as thumbnails in a grid over the
/// whole area. Windows of other apps are left out.
pub fn app_overview<K: PartialEq>(area: Rect, windows: &[(WindowId, Rect, K)], app: &K) -> Vec<(WindowId, Rect)> {
    let mine: Vec<(WindowId, Rect)> = windows.iter().filter(|(_, _, k)| k == app).map(|(id, r, _)| (*id, *r)).collect();
    grid(area, &mine)
}

fn grid(area: Rect, windows: &[(WindowId, Rect)]) -> Vec<(WindowId, Rect)> {
    cells(area, windows.len()).into_iter().zip(windows).map(|(cell, &(id, src))| (id, fit(src, cell))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 28, 1920, 1052);

    #[test]
    fn a_grid_of_cells_holds_every_item_inside_the_area_without_overlap() {
        for count in 1..=11 {
            let cs = cells(AREA, count);
            assert_eq!(cs.len(), count);
            for c in &cs {
                assert!(c.x >= AREA.x && c.y >= AREA.y, "{c:?}");
                assert!(c.x + c.w <= AREA.x + AREA.w && c.y + c.h <= AREA.y + AREA.h, "{c:?}");
            }
            for (i, a) in cs.iter().enumerate() {
                for b in &cs[i + 1..] {
                    let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                    assert!(apart, "{a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn four_windows_make_a_two_by_two_grid_and_five_leave_the_last_row_centred() {
        let four = cells(AREA, 4);
        assert_eq!(four[0].y, four[1].y);
        assert_eq!(four[2].y, four[3].y);
        assert!(four[2].y > four[0].y);
        let five = cells(AREA, 5);
        assert_eq!(five[0].y, five[1].y, "the first row is full and not shifted");
        let row_left_gap = five[3].x - AREA.x;
        let row_right_gap = AREA.x + AREA.w - (five[4].x + five[4].w);
        assert!((row_left_gap - row_right_gap).abs() <= 2 * GUTTER, "{row_left_gap} vs {row_right_gap}");
        assert!(cells(AREA, 0).is_empty());
    }

    #[test]
    fn a_thumbnail_keeps_its_windows_shape_and_is_centred_in_its_cell() {
        let cell = Rect::new(100, 100, 400, 400);
        let f = fit(Rect::new(0, 0, 800, 600), cell);
        assert_eq!((f.w, f.h), (400, 300));
        assert_eq!((f.x, f.y), (100, 150));
        let tall = fit(Rect::new(0, 0, 300, 900), cell);
        assert_eq!((tall.w, tall.h), (133, 400));
    }

    #[test]
    fn the_overview_puts_desktops_on_top_and_windows_below_the_strip() {
        let o = overview(AREA, 3, &[(1, Rect::new(0, 0, 800, 600)), (2, Rect::new(0, 0, 800, 600))]);
        assert_eq!(o.desktops.len(), 3);
        assert!(o.desktops.iter().all(|r| r.y + r.h <= AREA.y + STRIP_H));
        assert_eq!(o.windows.len(), 2);
        assert!(o.windows.iter().all(|(_, r)| r.y >= AREA.y + STRIP_H));
        assert_eq!(o.windows[0].0, 1, "order is kept");
    }

    #[test]
    fn the_app_overview_shows_only_one_apps_windows() {
        let ws = [(1, Rect::new(0, 0, 800, 600), "browser"), (2, Rect::new(0, 0, 800, 600), "mail"), (3, Rect::new(0, 0, 400, 400), "browser")];
        let out = app_overview(AREA, &ws, &"browser");
        assert_eq!(out.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 3]);
        assert!(app_overview(AREA, &ws, &"terminal").is_empty());
    }

    #[test]
    fn the_desktop_strip_is_centred_and_empty_without_desktops() {
        let s = desktop_strip(AREA, 2);
        assert_eq!(s.len(), 2);
        let left = s[0].x - AREA.x;
        let right = AREA.x + AREA.w - (s[1].x + s[1].w);
        assert!((left - right).abs() <= 1);
        assert!(desktop_strip(AREA, 0).is_empty());
    }
}
