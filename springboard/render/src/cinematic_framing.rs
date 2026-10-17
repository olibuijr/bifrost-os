//! Automatic subject framing for video (a virtual camera operator), as pure code (std only). Given the boxes of the
//! people or faces in a frame, `target_crop` computes the crop rectangle that holds all of them with a margin at the
//! output aspect ratio, limited to a maximum zoom and kept inside the frame. `Framer` moves a live crop toward that
//! target the way a camera operator would: exponential smoothing with a time constant, a dead zone that ignores small
//! changes (no jitter), and a hold time before it zooms back out after everyone leaves. `apply_crop` resamples a
//! plane through the crop.
//!
//! The boxes come from a detector (face or body); this module does not detect. Pair it with `masked_blur` for a
//! background blur and `portrait_lighting` for relighting to get the video-call effect stack.

use crate::effects::Plane;

/// A rectangle in frame pixels, top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    fn valid(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.w.is_finite() && self.h.is_finite() && self.w > 0.0 && self.h > 0.0
    }
}

/// How the crop is chosen and how fast it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramingConfig {
    /// Output width over height.
    pub aspect: f32,
    /// Extra room around the subjects, as a fraction of their box size on each side.
    pub margin: f32,
    /// Shifts the crop down relative to the subjects by this fraction of the crop height, so the subjects sit high
    /// (headroom). 0 centres them.
    pub bias: f32,
    /// The crop is never smaller than the full crop divided by this (a zoom limit, at least 1).
    pub max_zoom: f32,
    /// Smoothing time constant in seconds.
    pub response: f32,
    /// Changes smaller than this fraction of the crop height are ignored.
    pub deadzone: f32,
    /// Seconds to keep the crop after the last subject disappears before zooming out.
    pub hold: f32,
}

impl Default for FramingConfig {
    fn default() -> Self {
        FramingConfig { aspect: 16.0 / 9.0, margin: 0.35, bias: 0.08, max_zoom: 3.0, response: 0.4, deadzone: 0.04, hold: 2.0 }
    }
}

/// The largest crop of the output aspect that fits the frame, centred.
pub fn full_crop(frame_w: f32, frame_h: f32, aspect: f32) -> Rect {
    let h = frame_h.min(frame_w / aspect);
    let w = h * aspect;
    Rect { x: (frame_w - w) * 0.5, y: (frame_h - h) * 0.5, w, h }
}

/// The crop for these subject boxes. No usable subject gives the full crop. The crop height is what the union of the
/// boxes (plus margin) needs in either direction, clamped between the full height divided by `max_zoom` and the full
/// height; it is centred on the union (lowered by `bias`) and pushed back inside the frame.
pub fn target_crop(frame_w: f32, frame_h: f32, subjects: &[Rect], cfg: &FramingConfig) -> Rect {
    let full = full_crop(frame_w, frame_h, cfg.aspect);
    let mut it = subjects.iter().filter(|r| r.valid());
    let Some(first) = it.next() else {
        return full;
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for r in it {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w);
        y1 = y1.max(r.y + r.h);
    }
    let grow = 1.0 + 2.0 * cfg.margin.max(0.0);
    let needed = ((y1 - y0) * grow).max((x1 - x0) * grow / cfg.aspect);
    let h = needed.clamp(full.h / cfg.max_zoom.max(1.0), full.h);
    let w = h * cfg.aspect;
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5 + cfg.bias * h;
    Rect { x: (cx - w * 0.5).clamp(0.0, frame_w - w), y: (cy - h * 0.5).clamp(0.0, frame_h - h), w, h }
}

/// A live crop that follows the subjects smoothly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framer {
    frame_w: f32,
    frame_h: f32,
    current: Rect,
    lost_for: f32,
}

impl Framer {
    /// Starts at the full crop. `None` for a non-positive frame size or aspect.
    pub fn new(frame_w: f32, frame_h: f32, cfg: &FramingConfig) -> Option<Framer> {
        if !(frame_w > 0.0 && frame_h > 0.0 && cfg.aspect > 0.0) {
            return None;
        }
        Some(Framer { frame_w, frame_h, current: full_crop(frame_w, frame_h, cfg.aspect), lost_for: 0.0 })
    }

    /// The crop now.
    pub fn crop(&self) -> Rect {
        self.current
    }

    /// Advances by `dt` seconds with this frame's subject boxes and returns the new crop.
    pub fn update(&mut self, subjects: &[Rect], cfg: &FramingConfig, dt: f32) -> Rect {
        let target = if subjects.iter().any(|r| r.valid()) {
            self.lost_for = 0.0;
            target_crop(self.frame_w, self.frame_h, subjects, cfg)
        } else {
            self.lost_for += dt;
            if self.lost_for < cfg.hold {
                return self.current;
            }
            full_crop(self.frame_w, self.frame_h, cfg.aspect)
        };
        let (cx, cy) = self.current.center();
        let (tx, ty) = target.center();
        let h = self.current.h;
        let small = (tx - cx).abs() / h < cfg.deadzone && (ty - cy).abs() / h < cfg.deadzone && (target.h - h).abs() / h < cfg.deadzone;
        if small {
            return self.current;
        }
        let alpha = if cfg.response > 0.0 { 1.0 - (-dt.max(0.0) / cfg.response).exp() } else { 1.0 };
        let nh = (h + (target.h - h) * alpha).clamp(1.0, self.frame_h.min(self.frame_w / cfg.aspect));
        let nw = nh * cfg.aspect;
        let ncx = cx + (tx - cx) * alpha;
        let ncy = cy + (ty - cy) * alpha;
        self.current = Rect {
            x: (ncx - nw * 0.5).clamp(0.0, self.frame_w - nw),
            y: (ncy - nh * 0.5).clamp(0.0, self.frame_h - nh),
            w: nw,
            h: nh,
        };
        self.current
    }
}

/// Resamples the crop region of `src` to `out_w` by `out_h` with bilinear filtering (sampled at pixel centres, edges
/// clamped). `None` for an empty source, a zero output or an invalid crop.
pub fn apply_crop(src: &Plane, crop: &Rect, out_w: usize, out_h: usize) -> Option<Plane> {
    if src.w == 0 || src.h == 0 || out_w == 0 || out_h == 0 || !crop.valid() {
        return None;
    }
    let get = |x: i32, y: i32| src.a[(y.clamp(0, src.h as i32 - 1) as usize) * src.w + x.clamp(0, src.w as i32 - 1) as usize];
    let mut out = Plane::new(out_w, out_h);
    for y in 0..out_h {
        let sy = crop.y + (y as f32 + 0.5) * crop.h / out_h as f32 - 0.5;
        let y0 = sy.floor();
        let fy = sy - y0;
        for x in 0..out_w {
            let sx = crop.x + (x as f32 + 0.5) * crop.w / out_w as f32 - 0.5;
            let x0 = sx.floor();
            let fx = sx - x0;
            let (xi, yi) = (x0 as i32, y0 as i32);
            let top = get(xi, yi) * (1.0 - fx) + get(xi + 1, yi) * fx;
            let bottom = get(xi, yi + 1) * (1.0 - fx) + get(xi + 1, yi + 1) * fx;
            out.a[y * out_w + x] = top * (1.0 - fy) + bottom * fy;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FW: f32 = 1920.0;
    const FH: f32 = 1080.0;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn inside(c: &Rect) -> bool {
        c.x >= -1e-3 && c.y >= -1e-3 && c.x + c.w <= FW + 1e-3 && c.y + c.h <= FH + 1e-3
    }

    #[test]
    fn no_subjects_gives_the_full_crop() {
        let cfg = FramingConfig::default();
        let c = target_crop(FW, FH, &[], &cfg);
        assert!(c.x.abs() < 1e-2 && c.y.abs() < 1e-2 && (c.w - FW).abs() < 1e-2 && (c.h - FH).abs() < 1e-2, "{c:?}");
        let bad = rect(f32::NAN, 0.0, 10.0, 10.0);
        assert_eq!(target_crop(FW, FH, &[bad, rect(1.0, 1.0, 0.0, 5.0)], &cfg), c);
        // A different aspect crops the frame to the largest centred rectangle.
        let sq = full_crop(FW, FH, 1.0);
        assert_eq!((sq.w, sq.h, sq.x, sq.y), (1080.0, 1080.0, 420.0, 0.0));
    }

    #[test]
    fn crop_holds_the_subject_at_the_output_aspect() {
        let cfg = FramingConfig::default();
        let s = rect(800.0, 300.0, 200.0, 300.0);
        let c = target_crop(FW, FH, &[s], &cfg);
        assert!((c.w / c.h - cfg.aspect).abs() < 1e-4);
        assert!(inside(&c));
        assert!(c.x <= s.x && c.x + c.w >= s.x + s.w && c.y <= s.y && c.y + c.h >= s.y + s.h);
        assert!(c.h < FH);
    }

    #[test]
    fn zoom_is_limited_and_the_crop_stays_in_the_frame() {
        let cfg = FramingConfig::default();
        let tiny = rect(1850.0, 20.0, 40.0, 40.0);
        let c = target_crop(FW, FH, &[tiny], &cfg);
        assert!((c.h - FH / 3.0).abs() < 1e-3);
        assert!(inside(&c));
    }

    #[test]
    fn several_subjects_are_all_kept() {
        let cfg = FramingConfig::default();
        let a = rect(200.0, 400.0, 150.0, 200.0);
        let b = rect(1500.0, 420.0, 150.0, 200.0);
        let c = target_crop(FW, FH, &[a, b], &cfg);
        for s in [a, b] {
            assert!(c.x <= s.x && c.x + c.w >= s.x + s.w, "{c:?}");
        }
    }

    #[test]
    fn headroom_bias_lowers_the_crop() {
        let s = rect(900.0, 400.0, 120.0, 160.0);
        let centred = target_crop(FW, FH, &[s], &FramingConfig { bias: 0.0, ..FramingConfig::default() });
        let biased = target_crop(FW, FH, &[s], &FramingConfig { bias: 0.1, ..FramingConfig::default() });
        assert!(biased.y > centred.y);
    }

    #[test]
    fn framer_moves_smoothly_then_settles() {
        let cfg = FramingConfig::default();
        let mut f = Framer::new(FW, FH, &cfg).unwrap();
        assert_eq!(f.crop(), full_crop(FW, FH, cfg.aspect));
        let s = rect(300.0, 300.0, 160.0, 220.0);
        let target = target_crop(FW, FH, &[s], &cfg);
        let first = f.update(&[s], &cfg, 1.0 / 30.0);
        // One step moves part of the way.
        assert!(first.h < FH && first.h > target.h);
        for _ in 0..600 {
            f.update(&[s], &cfg, 1.0 / 30.0);
        }
        let c = f.crop();
        assert!((c.h - target.h).abs() <= cfg.deadzone * FH + 1.0, "{} vs {}", c.h, target.h);
        assert!(inside(&c));
        assert!((c.w / c.h - cfg.aspect).abs() < 1e-3);
    }

    #[test]
    fn framer_ignores_small_changes_and_holds_then_zooms_out() {
        let cfg = FramingConfig::default();
        let mut f = Framer::new(FW, FH, &cfg).unwrap();
        let s = rect(900.0, 300.0, 160.0, 220.0);
        for _ in 0..300 {
            f.update(&[s], &cfg, 1.0 / 30.0);
        }
        let settled = f.crop();
        // Settled means the target is inside the dead zone, so the same subject moves nothing.
        assert_eq!(f.update(&[s], &cfg, 1.0 / 30.0), settled);
        // Lost subject: the crop is held for `hold` seconds, then drifts back to the full frame.
        for _ in 0..30 {
            assert_eq!(f.update(&[], &cfg, 1.0 / 30.0), settled);
        }
        for _ in 0..600 {
            f.update(&[], &cfg, 1.0 / 30.0);
        }
        assert!(f.crop().h > 0.9 * FH);
    }

    #[test]
    fn framer_rejects_a_bad_frame() {
        assert!(Framer::new(0.0, 100.0, &FramingConfig::default()).is_none());
        assert!(Framer::new(100.0, 100.0, &FramingConfig { aspect: 0.0, ..FramingConfig::default() }).is_none());
    }

    #[test]
    fn crop_resamples_a_plane() {
        let mut p = Plane::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                p.a[y * 8 + x] = x as f32;
            }
        }
        let same = apply_crop(&p, &rect(0.0, 0.0, 8.0, 8.0), 8, 8).unwrap();
        for i in 0..64 {
            assert!((same.a[i] - p.a[i]).abs() < 1e-5, "{i}");
        }
        // The right half, enlarged: values run from 3.5 + 0.25 up toward 7.
        let half = apply_crop(&p, &rect(4.0, 0.0, 4.0, 4.0), 8, 4).unwrap();
        assert!((half.a[0] - 3.75).abs() < 1e-5);
        assert!(half.a[7] > half.a[0]);
        assert!(apply_crop(&p, &rect(0.0, 0.0, 0.0, 4.0), 4, 4).is_none());
    }
}
