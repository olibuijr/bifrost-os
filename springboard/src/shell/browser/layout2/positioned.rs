//! Positioning after taffy: relative offsets, absolute and fixed boxes
//! against their containing blocks, page positions and ink bounds, plus the
//! sticky and transform helpers shared by painting and hit testing.
//!
//! Absolute and fixed boxes are not children of their parent for taffy; they
//! are laid out here as the roots of their own layouts once the in-flow
//! layout is known, against the nearest positioned ancestor (or the initial
//! containing block).

use super::super::display::{Xf, IDENTITY};
use super::super::style::{self as cs};
use super::paint::BoxPaint;
use super::tree::*;
use super::Ctx;
use taffy as tf;

/// A containing block in page space (the padding box of its element).
#[derive(Clone, Copy, Debug)]
pub struct Cb {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

fn union4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// How far a sticky box is pushed: `st` is top, right, bottom, left (NaN =
/// auto); the rectangles are `[x, y, w, h]` in the same space: the box, its
/// containing block's content box, and the scrollport.
pub fn sticky_delta(st: [f32; 4], rect: [f32; 4], parent: [f32; 4], port: [f32; 4]) -> (f32, f32) {
    let mut dy = 0.0f32;
    if !st[0].is_nan() {
        dy = dy.max(port[1] + st[0] - rect[1]);
    }
    if !st[2].is_nan() {
        dy = dy.min(port[1] + port[3] - st[2] - rect[3] - rect[1]);
    }
    let mut dx = 0.0f32;
    if !st[3].is_nan() {
        dx = dx.max(port[0] + st[3] - rect[0]);
    }
    if !st[1].is_nan() {
        dx = dx.min(port[0] + port[2] - st[1] - rect[2] - rect[0]);
    }
    dy = dy.min(parent[1] + parent[3] - (rect[1] + rect[3]));
    dy = dy.max(parent[1] - rect[1]);
    dx = dx.min(parent[0] + parent[2] - (rect[0] + rect[2]));
    dx = dx.max(parent[0] - rect[0]);
    (dx, dy)
}

/// The page-space matrix of a transform about its origin for a box whose
/// painted top-left is `pos`. `m` is the resolved CSS matrix, `origin` the
/// transform origin relative to the box.
pub fn page_matrix(m: &[f32; 6], origin: [f32; 2], pos: (f32, f32)) -> Xf {
    let (ox, oy) = (pos.0 + origin[0], pos.1 + origin[1]);
    let mut out = IDENTITY;
    out[0] = m[0];
    out[1] = m[1];
    out[2] = m[2];
    out[3] = m[3];
    out[4] = m[4] + ox - m[0] * ox - m[2] * oy;
    out[5] = m[5] + oy - m[1] * ox - m[3] * oy;
    out
}

/// The inverse of an affine matrix, if it has one.
pub fn invert(m: &Xf) -> Option<Xf> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1.0e-9 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

impl<'a> Ctx<'a> {
    /// Positions everything after taffy: page positions, out-of-flow boxes,
    /// final lines of inline formatting roots, paint data and ink bounds.
    pub fn finalize(&mut self) {
        let n = self.t.boxes.len();
        self.t.paint.clear();
        self.t.paint.resize_with(n, BoxPaint::empty);
        let vp = self.t.viewport;
        let icb = Cb {
            x: 0.0,
            y: 0.0,
            w: vp.0,
            h: vp.1,
        };
        let root = self.t.root;
        self.place(root, [0.0, 0.0], icb, icb);
    }

    /// Whether the box's overflow belongs to the viewport (root, body).
    pub fn propagates(&self, b: u32) -> bool {
        let bx = &self.t.boxes[b as usize];
        if bx.parent == NONE {
            return true;
        }
        if bx.parent == self.t.root && bx.pseudo == 0 {
            return self.dom.tag(super::super::dom::NodeId(bx.dom)) == Some("body");
        }
        false
    }

    fn place(&mut self, b: u32, parent_abs: [f32; 2], cb: Cb, fcb: Cb) -> ([f32; 4], bool) {
        let bi = b as usize;
        let cp = self.comp_of(b);
        let vp = self.t.viewport;
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let mut force_self = false;
        let mut xform = false;
        let mut clip = false;
        let mut positioned = false;
        if let Some(c) = cp {
            let pw = self.t.boxes[bi].pw;
            let bw = if pw[0].is_nan() { vp.0 } else { pw[0] };
            let bh = if pw[1].is_nan() { vp.1 } else { pw[1] };
            if c.position == cs::Position::Relative {
                let l = c.left.resolve(Some(bw));
                let r = c.right.resolve(Some(bw));
                let t = c.top.resolve(Some(bh));
                let bt = c.bottom.resolve(Some(bh));
                dx = l.or(r.map(|v| -v)).unwrap_or(0.0);
                dy = t.or(bt.map(|v| -v)).unwrap_or(0.0);
            }
            force_self = matches!(c.position, cs::Position::Fixed | cs::Position::Sticky)
                || c.transform.is_some();
            xform = c.transform.is_some();
            clip = !matches!(c.overflow_x, cs::Overflow::Visible)
                || !matches!(c.overflow_y, cs::Overflow::Visible);
            positioned = c.position != cs::Position::Static || xform;
        }
        if clip && self.propagates(b) {
            clip = false;
        }
        let loc = self.t.boxes[bi].loc;
        let abs = [parent_abs[0] + loc[0] + dx, parent_abs[1] + loc[1] + dy];
        self.t.boxes[bi].abs = abs;
        if self.t.boxes[bi].kind == Kind::Ifc {
            self.final_lines(b);
        }
        let (size, border, scroll, k0, nk, na) = {
            let bx = &self.t.boxes[bi];
            (
                bx.size,
                bx.border,
                bx.scroll_size,
                bx.kids as usize,
                bx.nkids as usize,
                bx.nabs as usize,
            )
        };
        let mut cb2 = cb;
        let mut fcb2 = fcb;
        if positioned {
            cb2 = Cb {
                x: abs[0] + border[3],
                y: abs[1] + border[0],
                w: size[0] - border[1] - border[3],
                h: size[1] - border[0] - border[2],
            };
            if xform {
                fcb2 = cb2;
            }
        }
        self.prep_paint(b);
        let mut ink = [abs[0], abs[1], abs[0] + size[0], abs[1] + size[1]];
        let mut up = xform;
        for i in 0..nk {
            let k = self.t.kids[k0 + i];
            let (ki, ku) = self.place(k, abs, cb2, fcb2);
            up |= ku;
            if !clip && self.t.boxes[k as usize].flags & F_FIXED == 0 {
                ink = union4(ink, ki);
            }
        }
        for j in 0..na {
            let k = self.t.kids[k0 + nk + j];
            self.layout_oof(k, b, abs, cb2, fcb2);
            let (ki, ku) = self.place(k, abs, cb2, fcb2);
            up |= ku;
            if !clip && self.t.boxes[k as usize].flags & F_FIXED == 0 {
                ink = union4(ink, ki);
            }
        }
        if !clip {
            ink = union4(
                ink,
                [abs[0], abs[1], abs[0] + scroll[0], abs[1] + scroll[1]],
            );
        }
        let bx = &mut self.t.boxes[bi];
        bx.ink = ink;
        if force_self {
            bx.flags |= F_FORCE;
        }
        (ink, up)
    }

    /// Lays out an absolutely positioned or fixed box against its
    /// containing block and stores its location relative to `parent`.
    fn layout_oof(&mut self, k: u32, parent: u32, parent_abs: [f32; 2], cb: Cb, fcb: Cb) {
        let ki = k as usize;
        let cp = match self.comp_of(k) {
            Some(c) => c,
            None => return,
        };
        let fixed = self.t.boxes[ki].flags & F_FIXED != 0;
        let c = if fixed { fcb } else { cb };
        let off = |l: &cs::Length, basis: f32| -> Option<f32> {
            match l {
                cs::Length::Auto | cs::Length::None => None,
                other => other.resolve(Some(basis)),
            }
        };
        let mg = |l: &cs::Length| -> f32 {
            match l {
                cs::Length::Auto => 0.0,
                other => other.resolve(Some(c.w)).unwrap_or(0.0),
            }
        };
        let (l, r, t, bm) = (
            off(&cp.left, c.w),
            off(&cp.right, c.w),
            off(&cp.top, c.h),
            off(&cp.bottom, c.h),
        );
        let (ml, mr, mt, mb) = (
            mg(&cp.margin_left),
            mg(&cp.margin_right),
            mg(&cp.margin_top),
            mg(&cp.margin_bottom),
        );
        let replaced = self.t.boxes[ki].kind == Kind::Replaced;
        let auto_w = !replaced
            && matches!(
                cp.width,
                cs::Length::Auto
                    | cs::Length::MinContent
                    | cs::Length::MaxContent
                    | cs::Length::FitContent
            );
        let mut known = tf::Size::NONE;
        if auto_w {
            known.width = match (l, r) {
                (Some(l), Some(r)) => Some((c.w - l - r - ml - mr).max(0.0)),
                _ => {
                    let avail = (c.w - l.unwrap_or(0.0) - r.unwrap_or(0.0) - ml - mr).max(0.0);
                    Some(self.shrink_to_fit(k, Some(avail)))
                }
            };
        }
        if matches!(cp.height, cs::Length::Auto) {
            if let (Some(t), Some(b)) = (t, bm) {
                known.height = Some((c.h - t - b - mt - mb).max(0.0));
            }
        }
        let aw = match known.width {
            Some(w) => tf::AvailableSpace::Definite(w),
            None => tf::AvailableSpace::MaxContent,
        };
        let ah = match known.height {
            Some(h) => tf::AvailableSpace::Definite(h),
            None => tf::AvailableSpace::MaxContent,
        };
        let out = self.perform(
            k,
            known,
            tf::Size {
                width: Some(c.w),
                height: Some(c.h),
            },
            tf::Size {
                width: aw,
                height: ah,
            },
            tf::RunMode::PerformLayout,
        );
        let (w, h) = (out.size.width, out.size.height);
        // Static position: after the previous in-flow sibling, else at the
        // parent's content origin.
        let (px, py, prev) = {
            let p = &self.t.boxes[parent as usize];
            (
                parent_abs[0] + p.border[3] + p.padding[3],
                parent_abs[1] + p.border[0] + p.padding[0],
                self.t.boxes[ki].aux,
            )
        };
        let sy = if prev != NONE && prev != k {
            let pb = &self.t.boxes[prev as usize];
            pb.abs[1] + pb.size[1]
        } else {
            py
        };
        let x = match (l, r) {
            (Some(l), _) => c.x + l + ml,
            (None, Some(r)) => c.x + c.w - r - mr - w,
            _ => px + ml,
        };
        let y = match (t, bm) {
            (Some(t), _) => c.y + t + mt,
            (None, Some(b)) => c.y + c.h - b - mb - h,
            _ => sy + mt,
        };
        self.commit_layout(k, &out, [x - parent_abs[0], y - parent_abs[1]], c.w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_top_pushes_down_within_parent() {
        let parent = [0.0, 0.0, 100.0, 500.0];
        let port = [0.0, 200.0, 100.0, 100.0];
        let rect = [0.0, 50.0, 100.0, 20.0];
        let (_, dy) = sticky_delta([0.0, f32::NAN, f32::NAN, f32::NAN], rect, parent, port);
        assert_eq!(dy, 150.0);
        // Stops at the bottom of the parent.
        let port = [0.0, 600.0, 100.0, 100.0];
        let (_, dy) = sticky_delta([0.0, f32::NAN, f32::NAN, f32::NAN], rect, parent, port);
        assert_eq!(dy, 430.0);
    }

    #[test]
    fn sticky_does_nothing_when_visible() {
        let parent = [0.0, 0.0, 100.0, 500.0];
        let port = [0.0, 0.0, 100.0, 100.0];
        let rect = [0.0, 50.0, 100.0, 20.0];
        let (dx, dy) = sticky_delta([0.0, f32::NAN, f32::NAN, f32::NAN], rect, parent, port);
        assert_eq!((dx, dy), (0.0, 0.0));
    }

    #[test]
    fn transform_matrix_is_about_the_origin() {
        // Scale 2 about the box centre (50, 25) at pos (10, 10).
        let m = page_matrix(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0], [50.0, 25.0], (10.0, 10.0));
        let (x, y) = super::super::super::display::xf_point(&m, 60.0, 35.0);
        assert_eq!((x, y), (60.0, 35.0));
        let (x, y) = super::super::super::display::xf_point(&m, 10.0, 10.0);
        assert_eq!((x, y), (-40.0, -15.0));
    }

    #[test]
    fn inverse_round_trips() {
        let m = page_matrix(&[0.0, 1.0, -1.0, 0.0, 5.0, 7.0], [10.0, 10.0], (0.0, 0.0));
        let inv = invert(&m).unwrap();
        let (x, y) = super::super::super::display::xf_point(&m, 3.0, 4.0);
        let (bx, by) = super::super::super::display::xf_point(&inv, x, y);
        assert!((bx - 3.0).abs() < 1e-4 && (by - 4.0).abs() < 1e-4);
    }
}
