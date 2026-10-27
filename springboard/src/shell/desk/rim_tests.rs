use super::*;

const RIM: u32 = 0x2626_2626; // white at 15%, as `window_rim_color` gives in Dark
const TITLE: u32 = 0xff30_3030;
const APP: u32 = 0xff20_2020;

/// The window as the full draw makes it: title bar, the app's frame below it (dimmed behind), one rim stroke, the
/// corners clipped.
fn full_draw(width: usize, height: usize, top: usize, src: &[u32], dim: Option<u32>, radius: f32) -> Vec<u32> {
    let mut px = vec![TITLE; width * height];
    px[top * width..].copy_from_slice(src);
    if let Some(a) = dim { dim_rect(&mut px, width, 0, top, width, height, a); }
    stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut px[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, RIM);
    clip_window_corners(&mut px, width, height, radius, None);
    px
}

/// The app repainted [x0, x1) x [y0, y1) of its body: the fast path copies that part of its frame in again.
fn app_repaint(px: &mut [u32], width: usize, top: usize, src: &[u32], dim: Option<u32>, (x0, y0, x1, y1): (usize, usize, usize, usize)) {
    for y in y0..y1 {
        px[(y + top) * width + x0..(y + top) * width + x1].copy_from_slice(&src[y * width + x0..y * width + x1]);
        if let Some(a) = dim { dim_rect(px, width, x0, y + top, x1, y + top + 1, a); }
    }
}

/// E2E-10: repeated retained-path repaints that touch an edge leave every edge pixel at the single-stroke value
/// (the Dark rim stacked 15% white per repaint into a solid white border before).
#[test]
fn repeated_edge_repaints_keep_the_rim_at_one_stroke() {
    for (top, dim, radius) in [(0usize, None, 8.0f32), (10, None, 8.0), (0, Some(BACKGROUND_DIM), 6.0), (12, Some(BACKGROUND_DIM), 10.0)] {
        let (width, height) = (64usize, 48usize);
        let body_h = height - top;
        let src = vec![APP; width * body_h];
        let want = full_draw(width, height, top, &src, dim, radius);
        let mut px = want.clone();
        // list click (left and bottom), body scroll (right), a full repaint, a corner
        let repaints = [(0, 0, 20, body_h), (width - 20, 0, width, body_h), (0, 0, width, body_h), (0, body_h - 5, 30, body_h), (width - 4, 0, width, 4)];
        for round in 0..8 {
            let r = repaints[round % repaints.len()];
            app_repaint(&mut px, width, top, &src, dim, r);
            restore_window_edge(&mut px, width, height, top, (&src, width, body_h), dim, radius, RIM);
            assert_eq!(px, want, "round {round}: top {top}, dim {dim:?}: the edge holds exactly one rim stroke");
        }
        // the left edge pixel half way down is the app's colour with one 15% white stroke over it, not white
        let edge = px[(top + body_h / 2) * width];
        assert!(edge & 0xff < 0x50, "left edge {edge:08x} stays dark");
    }
}

/// The edge band covers the 1 px rim and every pixel the corner clip changes.
#[test]
fn the_edge_band_covers_the_rim_and_the_corners() {
    let (width, height, radius) = (40usize, 30usize, 9.0f32);
    let mut a = vec![APP; width * height];
    let b = a.clone();
    stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut a[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, RIM);
    clip_window_corners(&mut a, width, height, radius, None);
    let rects = edge_band_rects(width, height, radius);
    for y in 0..height { for x in 0..width {
        if a[y * width + x] != b[y * width + x] {
            let (x, y) = (x as i32, y as i32);
            assert!(rects.iter().any(|r| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3), "({x},{y}) changed outside the band");
        }
    }}
}
