//! Vertex batching: every primitive of a frame is appended to one vertex
//! list, and consecutive primitives that sample the same texture share a run
//! (one draw call). Fills and shadows need no texture and join any run.

use super::shaders::{KIND_FILL, KIND_SHADOW, KIND_TEXTURE};
use super::TextureId;
use crate::layout::Rect;

/// One vertex, 64 bytes, laid out for `glVertexAttribPointer`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vertex {
    /// position in target pixels, origin top-left
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// offset from the rounded rectangle's centre, in pixels
    pub local: [f32; 2],
    /// half width and height of the rounded rectangle
    pub half: [f32; 2],
    /// straight (non-premultiplied) colour
    pub color: [f32; 4],
    /// radius, kind, blur (shadow) or alpha (texture), unused
    pub params: [f32; 4],
}

/// `(location, floats, byte offset)` per attribute, derived from `Vertex`.
pub const ATTRIBS: [(u32, i32, usize); 6] = [
    (super::shaders::LOC_POS, 2, std::mem::offset_of!(Vertex, pos)),
    (super::shaders::LOC_UV, 2, std::mem::offset_of!(Vertex, uv)),
    (super::shaders::LOC_LOCAL, 2, std::mem::offset_of!(Vertex, local)),
    (super::shaders::LOC_HALF, 2, std::mem::offset_of!(Vertex, half)),
    (super::shaders::LOC_COLOR, 4, std::mem::offset_of!(Vertex, color)),
    (super::shaders::LOC_PARAMS, 4, std::mem::offset_of!(Vertex, params)),
];

pub const VERTEX_BYTES: usize = std::mem::size_of::<Vertex>();
pub const VERTS_PER_QUAD: usize = 6;
/// One buffer upload holds at most this many quads; the caller flushes before.
pub const MAX_QUADS: usize = 4096;

/// A span of vertices drawn with one texture binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub tex: Option<TextureId>,
    pub start: usize,
    pub count: usize,
}

#[derive(Default)]
pub struct Batch {
    verts: Vec<Vertex>,
    runs: Vec<Run>,
}

/// Radius limited to what fits in the rectangle.
pub fn clamp_radius(radius: f32, w: f32, h: f32) -> f32 {
    radius.max(0.0).min(w.max(0.0) / 2.0).min(h.max(0.0) / 2.0)
}

fn rect_f(r: Rect) -> [f32; 4] {
    [r.x as f32, r.y as f32, r.w as f32, r.h as f32]
}

/// Two triangles covering `[x, y, w, h]` (already inflated for shadows),
/// with `uv` as `[u0, v0, u1, v1]` across it.
fn quad(area: [f32; 4], uv: [f32; 4], centre: [f32; 2], half: [f32; 2], color: [f32; 4], params: [f32; 4]) -> [Vertex; 6] {
    let [x, y, w, h] = area;
    let corner = |cx: f32, cy: f32, u: f32, v: f32| Vertex {
        pos: [cx, cy],
        uv: [u, v],
        local: [cx - centre[0], cy - centre[1]],
        half,
        color,
        params,
    };
    let tl = corner(x, y, uv[0], uv[1]);
    let tr = corner(x + w, y, uv[2], uv[1]);
    let bl = corner(x, y + h, uv[0], uv[3]);
    let br = corner(x + w, y + h, uv[2], uv[3]);
    [tl, tr, bl, tr, br, bl]
}

impl Batch {
    pub fn new() -> Self {
        Batch::default()
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.runs.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }

    pub fn quads(&self) -> usize {
        self.verts.len() / VERTS_PER_QUAD
    }

    pub fn is_full(&self) -> bool {
        self.quads() >= MAX_QUADS
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.verts
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// True when a pending run samples `t`.
    pub fn uses(&self, t: TextureId) -> bool {
        self.runs.iter().any(|r| r.tex == Some(t))
    }

    fn append(&mut self, tex: Option<TextureId>, q: [Vertex; 6]) {
        let need_new = match (self.runs.last(), tex) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(last), Some(t)) => last.tex.is_some() && last.tex != Some(t),
        };
        let start = self.verts.len();
        if need_new {
            self.runs.push(Run { tex, start, count: 0 });
        } else if let (Some(last), Some(t)) = (self.runs.last_mut(), tex) {
            // a run that began with fills adopts the first texture it meets
            if last.tex.is_none() {
                last.tex = Some(t);
            }
        }
        self.verts.extend_from_slice(&q);
        if let Some(last) = self.runs.last_mut() {
            last.count += VERTS_PER_QUAD;
        }
    }

    pub fn push_fill(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let a = rect_f(rect);
        let half = [a[2] / 2.0, a[3] / 2.0];
        let centre = [a[0] + half[0], a[1] + half[1]];
        let params = [clamp_radius(radius, a[2], a[3]), KIND_FILL, 0.0, 0.0];
        self.append(None, quad(a, [0.0; 4], centre, half, color, params));
    }

    /// A blurred rounded-rectangle shadow; the quad grows by `blur` all round.
    pub fn push_shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: [f32; 4]) {
        if rect.w <= 0 || rect.h <= 0 || blur <= 0.0 {
            return;
        }
        let a = rect_f(rect);
        let half = [a[2] / 2.0, a[3] / 2.0];
        let centre = [a[0] + half[0], a[1] + half[1]];
        let grown = [a[0] - blur, a[1] - blur, a[2] + 2.0 * blur, a[3] + 2.0 * blur];
        let params = [clamp_radius(radius, a[2], a[3]), KIND_SHADOW, blur, 0.0];
        self.append(None, quad(grown, [0.0; 4], centre, half, color, params));
    }

    /// A textured quad; `uv` is `[u0, v0, u1, v1]` (see `texture::uv`).
    pub fn push_texture(&mut self, tex: TextureId, uv: [f32; 4], dst: Rect, radius: f32, alpha: f32) {
        if dst.w <= 0 || dst.h <= 0 {
            return;
        }
        let a = rect_f(dst);
        let half = [a[2] / 2.0, a[3] / 2.0];
        let centre = [a[0] + half[0], a[1] + half[1]];
        let params = [clamp_radius(radius, a[2], a[3]), KIND_TEXTURE, alpha.clamp(0.0, 1.0), 0.0];
        self.append(Some(tex), quad(a, uv, centre, half, [1.0; 4], params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    const T1: TextureId = TextureId(1);
    const T2: TextureId = TextureId(2);
    const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    #[test]
    fn vertex_is_64_bytes_for_the_attribute_layout() {
        assert_eq!(VERTEX_BYTES, 64);
    }

    #[test]
    fn attribute_offsets_are_packed_in_order() {
        let offs: Vec<usize> = ATTRIBS.iter().map(|a| a.2).collect();
        assert_eq!(offs, vec![0, 8, 16, 24, 32, 48]);
        let floats: i32 = ATTRIBS.iter().map(|a| a.1).sum();
        assert_eq!(floats as usize * 4, VERTEX_BYTES);
    }

    #[test]
    fn fill_makes_one_quad_covering_the_rect() {
        let mut b = Batch::new();
        b.push_fill(r(10, 20, 30, 40), 5.0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.quads(), 1);
        let v = b.vertices();
        assert_eq!(v.len(), 6);
        let xs: Vec<f32> = v.iter().map(|v| v.pos[0]).collect();
        let ys: Vec<f32> = v.iter().map(|v| v.pos[1]).collect();
        assert_eq!(xs.iter().cloned().fold(f32::MAX, f32::min), 10.0);
        assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 40.0);
        assert_eq!(ys.iter().cloned().fold(f32::MAX, f32::min), 20.0);
        assert_eq!(ys.iter().cloned().fold(f32::MIN, f32::max), 60.0);
        assert_eq!(v[0].half, [15.0, 20.0]);
        assert_eq!(v[0].params, [5.0, KIND_FILL, 0.0, 0.0]);
        // top-left corner sits at (-half) from the centre
        assert_eq!(v[0].local, [-15.0, -20.0]);
    }

    #[test]
    fn two_triangles_cover_all_four_corners() {
        let mut b = Batch::new();
        b.push_fill(r(0, 0, 4, 2), 0.0, [1.0; 4]);
        let mut corners: Vec<[i32; 2]> = b.vertices().iter().map(|v| [v.pos[0] as i32, v.pos[1] as i32]).collect();
        corners.sort();
        corners.dedup();
        assert_eq!(corners, vec![[0, 0], [0, 2], [4, 0], [4, 2]]);
    }

    #[test]
    fn radius_is_clamped_to_half_the_short_side() {
        assert_eq!(clamp_radius(100.0, 40.0, 20.0), 10.0);
        assert_eq!(clamp_radius(-3.0, 40.0, 20.0), 0.0);
        assert_eq!(clamp_radius(4.0, 40.0, 20.0), 4.0);
        let mut b = Batch::new();
        b.push_fill(r(0, 0, 40, 20), 99.0, [1.0; 4]);
        assert_eq!(b.vertices()[0].params[0], 10.0);
    }

    #[test]
    fn empty_rects_add_nothing() {
        let mut b = Batch::new();
        b.push_fill(r(0, 0, 0, 5), 0.0, [1.0; 4]);
        b.push_shadow(r(0, 0, 5, 0), 0.0, 4.0, [0.0; 4]);
        b.push_shadow(r(0, 0, 5, 5), 0.0, 0.0, [0.0; 4]);
        b.push_texture(T1, FULL, r(0, 0, -1, 5), 0.0, 1.0);
        assert!(b.is_empty());
        assert!(b.runs().is_empty());
    }

    #[test]
    fn shadow_quad_grows_by_blur_but_keeps_the_caster_box() {
        let mut b = Batch::new();
        b.push_shadow(r(100, 100, 50, 50), 8.0, 10.0, [0.0, 0.0, 0.0, 0.5]);
        let v = b.vertices();
        let minx = v.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
        let maxy = v.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert_eq!(minx, 90.0);
        assert_eq!(maxy, 160.0);
        assert_eq!(v[0].half, [25.0, 25.0]);
        assert_eq!(v[0].params, [8.0, KIND_SHADOW, 10.0, 0.0]);
        assert_eq!(v[0].local, [-35.0, -35.0]);
    }

    #[test]
    fn texture_quad_carries_uv_and_alpha() {
        let mut b = Batch::new();
        b.push_texture(T1, [0.25, 0.5, 0.75, 1.0], r(0, 0, 10, 10), 2.0, 1.5);
        let v = b.vertices();
        assert_eq!(v[0].uv, [0.25, 0.5]);
        assert_eq!(v[4].uv, [0.75, 1.0]);
        assert_eq!(v[0].params[1], KIND_TEXTURE);
        assert_eq!(v[0].params[2], 1.0, "alpha clamped");
        assert_eq!(v[0].color, [1.0; 4]);
    }

    #[test]
    fn same_texture_shares_one_run() {
        let mut b = Batch::new();
        b.push_texture(T1, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        b.push_texture(T1, FULL, r(10, 0, 10, 10), 0.0, 1.0);
        assert_eq!(b.runs(), &[Run { tex: Some(T1), start: 0, count: 12 }]);
    }

    #[test]
    fn texture_change_starts_a_new_run() {
        let mut b = Batch::new();
        b.push_texture(T1, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        b.push_texture(T2, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        b.push_texture(T1, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        assert_eq!(
            b.runs(),
            &[
                Run { tex: Some(T1), start: 0, count: 6 },
                Run { tex: Some(T2), start: 6, count: 6 },
                Run { tex: Some(T1), start: 12, count: 6 },
            ]
        );
    }

    #[test]
    fn fills_join_the_current_run_without_breaking_it() {
        let mut b = Batch::new();
        b.push_texture(T1, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        b.push_fill(r(0, 0, 5, 5), 0.0, [1.0; 4]);
        b.push_shadow(r(0, 0, 5, 5), 0.0, 3.0, [0.0; 4]);
        b.push_texture(T1, FULL, r(0, 0, 10, 10), 0.0, 1.0);
        assert_eq!(b.runs().len(), 1);
        assert_eq!(b.runs()[0].count, 24);
    }

    #[test]
    fn leading_fills_run_then_adopt_the_first_texture() {
        let mut b = Batch::new();
        b.push_fill(r(0, 0, 5, 5), 0.0, [1.0; 4]);
        b.push_shadow(r(0, 0, 5, 5), 0.0, 3.0, [0.0; 4]);
        assert_eq!(b.runs(), &[Run { tex: None, start: 0, count: 12 }]);
        b.push_texture(T2, FULL, r(0, 0, 5, 5), 0.0, 1.0);
        assert_eq!(b.runs(), &[Run { tex: Some(T2), start: 0, count: 18 }]);
    }

    #[test]
    fn run_counts_always_sum_to_the_vertices() {
        let mut b = Batch::new();
        for i in 0..20 {
            match i % 3 {
                0 => b.push_fill(r(i, 0, 5, 5), 1.0, [1.0; 4]),
                1 => b.push_texture(if i % 2 == 0 { T1 } else { T2 }, FULL, r(i, 0, 5, 5), 1.0, 1.0),
                _ => b.push_shadow(r(i, 0, 5, 5), 1.0, 2.0, [0.0; 4]),
            }
        }
        assert_eq!(b.runs().iter().map(|r| r.count).sum::<usize>(), b.vertices().len());
        let mut at = 0;
        for run in b.runs() {
            assert_eq!(run.start, at);
            at += run.count;
        }
    }

    #[test]
    fn uses_and_clear() {
        let mut b = Batch::new();
        b.push_texture(T1, FULL, r(0, 0, 5, 5), 0.0, 1.0);
        assert!(b.uses(T1));
        assert!(!b.uses(T2));
        b.clear();
        assert!(b.is_empty() && !b.uses(T1) && b.runs().is_empty());
    }

    #[test]
    fn fullness_is_reported() {
        let mut b = Batch::new();
        for _ in 0..MAX_QUADS {
            assert!(!b.is_full());
            b.push_fill(r(0, 0, 1, 1), 0.0, [1.0; 4]);
        }
        assert!(b.is_full());
    }
}
