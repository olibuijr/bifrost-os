//! Damage rectangles: clipping, merging and the row copies that move the
//! rendered pixels from the GPU's dma-buf into the scan-out buffer.

use crate::gfx::Area;
use crate::layout::Rect;

/// Beyond this many rectangles the per-rectangle overhead beats the savings,
/// so the list collapses to its bounding box.
pub const MAX_AREAS: usize = 16;

/// Clips `r` to a `w` x `h` frame; None when nothing is left.
pub fn clip(r: Rect, w: usize, h: usize) -> Option<Area> {
    let x0 = (r.x as i64).clamp(0, w as i64);
    let y0 = (r.y as i64).clamp(0, h as i64);
    let x1 = (r.x as i64 + r.w as i64).clamp(0, w as i64);
    let y1 = (r.y as i64 + r.h as i64).clamp(0, h as i64);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Area { x0: x0 as usize, y0: y0 as usize, x1: x1 as usize, y1: y1 as usize })
}

/// Smallest rectangle holding both.
pub fn union(a: Rect, b: Rect) -> Rect {
    if a.w <= 0 || a.h <= 0 {
        return b;
    }
    if b.w <= 0 || b.h <= 0 {
        return a;
    }
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// Grows `r` by `by` pixels on every side.
pub fn inflate(r: Rect, by: i32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by }
}

fn touches(a: &Area, b: &Area) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn bounds(a: &Area, b: &Area) -> Area {
    Area { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }
}

/// Clips every rectangle to the frame, drops empty ones and merges rectangles
/// that touch whenever their bounding box costs no more than copying both
/// (overlap counts twice). More than `MAX_AREAS` survivors collapse into one.
pub fn merge(rects: &[Rect], w: usize, h: usize) -> Vec<Area> {
    let mut out: Vec<Area> = rects.iter().filter_map(|r| clip(*r, w, h)).collect();
    loop {
        let mut merged = false;
        'scan: for i in 0..out.len() {
            for j in i + 1..out.len() {
                if touches(&out[i], &out[j]) {
                    let b = bounds(&out[i], &out[j]);
                    if b.pixels() <= out[i].pixels() + out[j].pixels() {
                        out[i] = b;
                        out.swap_remove(j);
                        merged = true;
                        break 'scan;
                    }
                }
            }
        }
        if !merged {
            break;
        }
    }
    if out.len() > MAX_AREAS {
        let all = out.iter().skip(1).fold(out[0], |a, b| bounds(&a, b));
        out = vec![all];
    }
    out.sort_by_key(|a| (a.y0, a.x0));
    out
}

/// Swaps red and blue and forces alpha opaque: XRGB (B in the low byte) to
/// the shell's RGBA order (R in the low byte).
#[inline]
pub fn swap_rb(p: u32) -> u32 {
    0xff00_0000 | (p & 0x0000_ff00) | ((p >> 16) & 0xff) | ((p & 0xff) << 16)
}

/// Why a copy was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum CopyError {
    /// source or destination slice ends before the rectangle does
    TooShort,
    /// a stride narrower than the rectangle's right edge
    BadStride,
}

/// Copies `areas` from `src` to `dst` (both w*stride pixel buffers sharing
/// pixel coordinates) row by row; rows are contiguous so the copy is a plain
/// memcpy the compiler vectorises with NEON. Returns the pixels copied.
pub fn copy_areas(
    src: &[u32],
    src_stride: usize,
    dst: &mut [u32],
    dst_stride: usize,
    areas: &[Area],
    swap: bool,
) -> Result<usize, CopyError> {
    let mut n = 0;
    for a in areas {
        if a.x1 > src_stride || a.x1 > dst_stride {
            return Err(CopyError::BadStride);
        }
        if a.x1 <= a.x0 || a.y1 <= a.y0 {
            continue;
        }
        let last_s = (a.y1 - 1) * src_stride + a.x1;
        let last_d = (a.y1 - 1) * dst_stride + a.x1;
        if last_s > src.len() || last_d > dst.len() {
            return Err(CopyError::TooShort);
        }
        let len = a.x1 - a.x0;
        for y in a.y0..a.y1 {
            let s = &src[y * src_stride + a.x0..][..len];
            let d = &mut dst[y * dst_stride + a.x0..][..len];
            if swap {
                for (o, i) in d.iter_mut().zip(s) {
                    *o = swap_rb(*i);
                }
            } else {
                d.copy_from_slice(s);
            }
        }
        n += a.pixels();
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn clip_inside_partial_outside_and_empty() {
        assert_eq!(clip(r(10, 10, 20, 20), 100, 100), Some(Area { x0: 10, y0: 10, x1: 30, y1: 30 }));
        assert_eq!(clip(r(-5, -5, 10, 10), 100, 100), Some(Area { x0: 0, y0: 0, x1: 5, y1: 5 }));
        assert_eq!(clip(r(95, 95, 20, 20), 100, 100), Some(Area { x0: 95, y0: 95, x1: 100, y1: 100 }));
        assert_eq!(clip(r(100, 0, 5, 5), 100, 100), None);
        assert_eq!(clip(r(0, 0, 0, 5), 100, 100), None);
        assert_eq!(clip(r(0, 0, -3, 5), 100, 100), None);
        assert_eq!(clip(r(i32::MAX - 1, 0, i32::MAX, 5), 100, 100), None);
    }

    #[test]
    fn union_and_inflate() {
        assert_eq!(union(r(0, 0, 10, 10), r(20, 5, 10, 10)), r(0, 0, 30, 15));
        assert_eq!(union(r(0, 0, 0, 0), r(3, 4, 5, 6)), r(3, 4, 5, 6));
        assert_eq!(union(r(3, 4, 5, 6), r(0, 0, 0, 0)), r(3, 4, 5, 6));
        assert_eq!(inflate(r(10, 10, 4, 4), 2), r(8, 8, 8, 8));
    }

    #[test]
    fn merge_overlapping_and_adjacent_rects() {
        let m = merge(&[r(0, 0, 10, 10), r(5, 0, 10, 10)], 100, 100);
        assert_eq!(m, vec![Area { x0: 0, y0: 0, x1: 15, y1: 10 }]);
        let m = merge(&[r(0, 0, 10, 10), r(10, 0, 10, 10)], 100, 100);
        assert_eq!(m, vec![Area { x0: 0, y0: 0, x1: 20, y1: 10 }]);
    }

    #[test]
    fn merge_keeps_far_apart_rects_separate_and_sorted() {
        let m = merge(&[r(50, 50, 5, 5), r(0, 0, 5, 5)], 100, 100);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].x0, 0);
        assert_eq!(m[1].x0, 50);
    }

    #[test]
    fn merge_does_not_join_rects_whose_bounds_waste_pixels() {
        // an L shape: the bounding box is far larger than both parts
        let m = merge(&[r(0, 0, 100, 4), r(0, 0, 4, 100)], 200, 200);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn merge_clips_and_drops_empty() {
        let m = merge(&[r(-10, -10, 20, 20), r(0, 0, 0, 0), r(500, 500, 5, 5)], 100, 100);
        assert_eq!(m, vec![Area { x0: 0, y0: 0, x1: 10, y1: 10 }]);
        assert!(merge(&[], 100, 100).is_empty());
    }

    #[test]
    fn merge_collapses_when_too_many() {
        let rects: Vec<Rect> = (0..40).map(|i| r(i * 20, (i % 5) * 50, 2, 2)).collect();
        let m = merge(&rects, 1000, 1000);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].x0, 0);
        assert_eq!(m[0].x1, 782);
    }

    #[test]
    fn swap_rb_exchanges_channels() {
        assert_eq!(swap_rb(0x0011_2233), 0xff33_2211);
        assert_eq!(swap_rb(swap_rb(0x0044_5566)), 0xff44_5566);
    }

    #[test]
    fn copy_only_touches_damage() {
        let src: Vec<u32> = (0..64).collect();
        let mut dst = vec![0u32; 80];
        let a = [Area { x0: 2, y0: 1, x1: 5, y1: 3 }];
        let n = copy_areas(&src, 8, &mut dst, 10, &a, false).unwrap();
        assert_eq!(n, 6);
        assert_eq!(&dst[12..15], &[10, 11, 12]);
        assert_eq!(&dst[22..25], &[18, 19, 20]);
        assert_eq!(dst.iter().filter(|&&p| p != 0).count(), 6);
        assert_eq!(dst[11], 0, "pixel left of the area stays");
    }

    #[test]
    fn copy_with_swap_converts_pixels() {
        let src = vec![0x0000_00ffu32; 4];
        let mut dst = vec![0u32; 4];
        copy_areas(&src, 2, &mut dst, 2, &[Area { x0: 0, y0: 0, x1: 2, y1: 2 }], true).unwrap();
        assert!(dst.iter().all(|&p| p == 0xffff_0000));
    }

    #[test]
    fn copy_rejects_short_buffers_and_bad_strides() {
        let src = vec![0u32; 16];
        let mut dst = vec![0u32; 16];
        let a = Area { x0: 0, y0: 0, x1: 4, y1: 5 };
        assert_eq!(copy_areas(&src, 4, &mut dst, 4, &[a], false), Err(CopyError::TooShort));
        let a = Area { x0: 0, y0: 0, x1: 5, y1: 1 };
        assert_eq!(copy_areas(&src, 4, &mut dst, 4, &[a], false), Err(CopyError::BadStride));
    }

    #[test]
    fn copy_of_nothing_is_zero() {
        let src = vec![0u32; 4];
        let mut dst = vec![0u32; 4];
        assert_eq!(copy_areas(&src, 2, &mut dst, 2, &[], false), Ok(0));
    }
}
