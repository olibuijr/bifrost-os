//! What of the CPU frame has to go to the GPU. Apps repaint in full, so the shell's damage
//! says little; comparing the frame with the copy uploaded last time finds the bands that
//! really changed (a video overlay's controls, a clock) and only those are uploaded. No GPU
//! needed.

use crate::gfx::Area;
use crate::layout::Rect;

/// Rows compared (and uploaded) together.
pub const BAND: usize = 16;
/// Pixels compared together along a row; changed spans are widened to whole chunks.
pub const CHUNK: usize = 64;
/// More upload rectangles than this collapse into their bounding box (fewer GL calls).
pub const MAX_RECTS: usize = 32;

/// Compares `cur` with `prev` (both `w` x `h`, tightly packed) inside `areas`, copies what
/// changed into `prev` and returns the changed rectangles: whole bands of rows, chunk-aligned
/// across, with bands of the same span merged downwards.
pub fn diff_update(prev: &mut [u32], cur: &[u32], w: usize, h: usize, areas: &[Area]) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    if prev.len() < w * h || cur.len() < w * h {
        return out;
    }
    for a in areas {
        let (x0, x1) = (a.x0.min(w), a.x1.min(w));
        let (y0, y1) = (a.y0.min(h), a.y1.min(h));
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let mut top = y0;
        while top < y1 {
            let bottom = (top + BAND).min(y1);
            let mut span: Option<(usize, usize)> = None;
            for row in top..bottom {
                let base = row * w;
                let mut x = x0;
                while x < x1 {
                    let end = (x + CHUNK).min(x1);
                    if cur[base + x..base + end] != prev[base + x..base + end] {
                        span = Some(match span {
                            Some((s, e)) => (s.min(x), e.max(end)),
                            None => (x, end),
                        });
                    }
                    x = end;
                }
            }
            if let Some((s, e)) = span {
                for row in top..bottom {
                    let base = row * w;
                    prev[base + s..base + e].copy_from_slice(&cur[base + s..base + e]);
                }
                let r = Rect { x: s as i32, y: top as i32, w: (e - s) as i32, h: (bottom - top) as i32 };
                match out.last_mut() {
                    Some(last) if last.x == r.x && last.w == r.w && last.y + last.h == r.y => last.h += r.h,
                    _ => out.push(r),
                }
            }
            top = bottom;
        }
    }
    if out.len() > MAX_RECTS {
        let all = out.iter().skip(1).fold(out[0], |a, b| super::damage::union(a, *b));
        out = vec![all];
    }
    out
}

/// Copies the whole frame into `prev` (after a full upload).
pub fn sync_all(prev: &mut Vec<u32>, cur: &[u32]) {
    prev.clear();
    prev.extend_from_slice(cur);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(w: usize, h: usize) -> Vec<Area> {
        vec![Area { x0: 0, y0: 0, x1: w, y1: h }]
    }

    #[test]
    fn nothing_changed_uploads_nothing() {
        let (w, h) = (200, 100);
        let cur = vec![7u32; w * h];
        let mut prev = cur.clone();
        assert!(diff_update(&mut prev, &cur, w, h, &all(w, h)).is_empty());
        assert_eq!(prev, cur);
    }

    #[test]
    fn one_pixel_gives_one_band_chunk_and_is_copied() {
        let (w, h) = (300, 100);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        cur[37 * w + 130] = 0xff00_00ff;
        let rects = diff_update(&mut prev, &cur, w, h, &all(w, h));
        assert_eq!(rects, vec![Rect { x: 128, y: 32, w: 64, h: 16 }]);
        assert_eq!(prev, cur, "prev holds the uploaded state");
        assert!(diff_update(&mut prev, &cur, w, h, &all(w, h)).is_empty(), "and the next frame sees no change");
    }

    #[test]
    fn spans_widen_across_rows_of_a_band() {
        let (w, h) = (256, 32);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        cur[2 * w + 10] = 1;
        cur[9 * w + 200] = 1;
        let rects = diff_update(&mut prev, &cur, w, h, &all(w, h));
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 256, h: 16 }]);
    }

    #[test]
    fn adjacent_bands_with_the_same_span_merge() {
        let (w, h) = (128, 64);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        for y in 0..40 {
            cur[y * w + 5] = 9;
        }
        let rects = diff_update(&mut prev, &cur, w, h, &all(w, h));
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 64, h: 48 }]);
    }

    #[test]
    fn far_apart_changes_stay_separate() {
        let (w, h) = (128, 128);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        cur[3 * w + 3] = 1;
        cur[120 * w + 100] = 1;
        let rects = diff_update(&mut prev, &cur, w, h, &all(w, h));
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 64, h: 16 }, Rect { x: 64, y: 112, w: 64, h: 16 }]);
    }

    #[test]
    fn only_the_given_areas_are_compared() {
        let (w, h) = (128, 64);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        cur[50 * w + 100] = 1;
        let rects = diff_update(&mut prev, &cur, w, h, &[Area { x0: 0, y0: 0, x1: 64, y1: 32 }]);
        assert!(rects.is_empty());
        assert_ne!(prev, cur, "outside the areas nothing is copied");
    }

    #[test]
    fn many_rects_collapse_to_their_bounds() {
        let (w, h) = (128, 16 * 70);
        let mut cur = vec![0u32; w * h];
        let mut prev = cur.clone();
        for b in 0..70 {
            // alternate the chunk so neighbouring bands never merge
            cur[(b * 16) * w + if b % 2 == 0 { 0 } else { 100 }] = 1;
        }
        let rects = diff_update(&mut prev, &cur, w, h, &all(w, h));
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 128, h: 16 * 70 }]);
    }

    #[test]
    fn short_buffers_are_refused() {
        let mut prev = vec![0u32; 10];
        assert!(diff_update(&mut prev, &[1u32; 10], 4, 4, &all(4, 4)).is_empty());
    }

    #[test]
    fn sync_all_copies() {
        let mut prev = vec![1u32; 3];
        sync_all(&mut prev, &[4, 5, 6, 7]);
        assert_eq!(prev, vec![4, 5, 6, 7]);
    }
}
