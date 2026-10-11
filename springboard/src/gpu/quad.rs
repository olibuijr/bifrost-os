//! Geometry of the layer compositor: the shell draws a logical frame (`sw` x `sh`, turned on
//! its side in landscape) and the GPU target is the portrait panel. A layer quad is given in
//! logical pixels; every corner is moved onto the panel with the same quarter turn the CPU
//! presenter uses (`Rotation::area`), keeping its texture coordinate, so the texture comes out
//! turned without any rotation in the shader. No GPU needed.

use super::damage;
use crate::gfx::rotate::Rotation;
use crate::layout::Rect;

/// One vertex of the layer program: target pixel position (x, y), texture coordinate (u, v).
pub type LayerVertex = [f32; 4];

/// Where the logical point (`x`, `y`) of a `sw` x `sh` frame lands on the panel after `rot`
/// (continuous coordinates: pixel edges, not centres).
pub fn rotate_point(rot: Rotation, x: f32, y: f32, sw: f32, sh: f32) -> (f32, f32) {
    match rot {
        Rotation::None => (x, y),
        Rotation::Cw => (sh - y, x),
        Rotation::Ccw => (y, sw - x),
        Rotation::Half => (sw - x, sh - y),
    }
}

/// Two triangles drawing `dst` (logical pixels) of a `sw` x `sh` frame onto the panel, with
/// `uv` = `[u0, v0, u1, v1]` spread over it the way it would be over the unturned rectangle
/// (u0, v0 at its top-left).
pub fn layer_quad(rot: Rotation, dst: Rect, uv: [f32; 4], sw: usize, sh: usize) -> [LayerVertex; 6] {
    let (sw, sh) = (sw as f32, sh as f32);
    let (x0, y0) = (dst.x as f32, dst.y as f32);
    let (x1, y1) = ((dst.x + dst.w) as f32, (dst.y + dst.h) as f32);
    let corner = |x: f32, y: f32, u: f32, v: f32| {
        let (px, py) = rotate_point(rot, x, y, sw, sh);
        [px, py, u, v]
    };
    let tl = corner(x0, y0, uv[0], uv[1]);
    let tr = corner(x1, y0, uv[2], uv[1]);
    let bl = corner(x0, y1, uv[0], uv[3]);
    let br = corner(x1, y1, uv[2], uv[3]);
    [tl, tr, bl, tr, br, bl]
}

/// The rotation turn's rectangle for the GPU: a `cw` x `ch` content rectangle centred in the
/// `sw` x `sh` logical frame and turned by `theta` (radians, clockwise on screen), with the
/// whole texture (`uv` 0..1) over it, then moved onto the panel with `rot`. Both of the turn's
/// pictures share this geometry (they differ only in texture), as on the CPU path.
pub fn turn_quad(rot: Rotation, theta: f32, (cw, ch): (f32, f32), sw: usize, sh: usize) -> [LayerVertex; 6] {
    let (sin, cos) = theta.sin_cos();
    let (cx, cy) = (sw as f32 / 2.0, sh as f32 / 2.0);
    let corner = |a: f32, b: f32, u: f32, v: f32| {
        let (x, y) = (cx + a * cos - b * sin, cy + a * sin + b * cos);
        let (px, py) = rotate_point(rot, x, y, sw as f32, sh as f32);
        [px, py, u, v]
    };
    let (hw, hh) = (cw / 2.0, ch / 2.0);
    let tl = corner(-hw, -hh, 0.0, 0.0);
    let tr = corner(hw, -hh, 1.0, 0.0);
    let bl = corner(-hw, hh, 0.0, 1.0);
    let br = corner(hw, hh, 1.0, 1.0);
    [tl, tr, bl, tr, br, bl]
}

/// The content size at progress `k`: the old frame's sliding to the new one's.
pub fn turn_size(old: (usize, usize), new: (usize, usize), k: f32) -> (f32, f32) {
    let k = k.clamp(0.0, 1.0);
    (old.0 as f32 + (new.0 as f32 - old.0 as f32) * k, old.1 as f32 + (new.1 as f32 - old.1 as f32) * k)
}

/// `r` (logical) clipped to the `sw` x `sh` frame and turned onto the panel; None when nothing
/// of it is on screen.
pub fn to_panel(rot: Rotation, r: Rect, sw: usize, sh: usize) -> Option<Rect> {
    let a = rot.area(damage::clip(r, sw, sh)?, sw, sh);
    Some(Rect { x: a.x0 as i32, y: a.y0 as i32, w: (a.x1 - a.x0) as i32, h: (a.y1 - a.y0) as i32 })
}

/// A `pw` x `ph` picture fitted (aspect kept, centred) into the view `(x, y, w, h)`, sized the
/// way the CPU path sizes it (`convert::fit`), so both paths put the picture in the same place.
pub fn fit_rect(pw: u32, ph: u32, view: (f32, f32, f32, f32)) -> Rect {
    let (vx, vy, vw, vh) = view;
    let (w, h) = crate::media::convert::fit(pw, ph, vw.round().max(2.0) as u32, vh.round().max(2.0) as u32);
    let x = (vx + (vw - w as f32) / 2.0).round() as i32;
    let y = (vy + (vh - h as f32) / 2.0).round() as i32;
    Rect { x, y, w: w as i32, h: h as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Rotation; 4] = [Rotation::None, Rotation::Cw, Rotation::Ccw, Rotation::Half];

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Bounding box of a quad's positions.
    fn bounds(q: &[LayerVertex; 6]) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for v in q {
            x0 = x0.min(v[0]);
            y0 = y0.min(v[1]);
            x1 = x1.max(v[0]);
            y1 = y1.max(v[1]);
        }
        r(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
    }

    /// The uv at the panel corner nearest (px, py).
    fn uv_at(q: &[LayerVertex; 6], px: f32, py: f32) -> (f32, f32) {
        let v = q.iter().min_by(|a, b| ((a[0] - px).abs() + (a[1] - py).abs()).total_cmp(&((b[0] - px).abs() + (b[1] - py).abs()))).unwrap();
        (v[2], v[3])
    }

    #[test]
    fn turn_quad_ends_cover_the_frame_and_start_turned() {
        // finished: upright, the new frame's size: the whole logical frame, turned to the panel
        for rot in [Rotation::None, Rotation::Cw, Rotation::Ccw, Rotation::Half] {
            let (sw, sh) = if matches!(rot, Rotation::Cw | Rotation::Ccw) { (1920, 1200) } else { (1200, 1920) };
            let q = turn_quad(rot, 0.0, (sw as f32, sh as f32), sw, sh);
            let direct = layer_quad(rot, r(0, 0, sw as i32, sh as i32), [0.0, 0.0, 1.0, 1.0], sw, sh);
            for (a, b) in q.iter().zip(direct.iter()) {
                assert!((a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3 && a[2] == b[2] && a[3] == b[3], "{rot:?}");
            }
        }
        // a portrait frame lying a quarter turn clockwise in a landscape frame: its top-left
        // corner is the frame's top-right
        let q = turn_quad(Rotation::None, std::f32::consts::FRAC_PI_2, (1200.0, 1920.0), 1920, 1200);
        assert!((q[0][0] - 1920.0).abs() < 1e-2 && q[0][1].abs() < 1e-2);
        assert_eq!((q[0][2], q[0][3]), (0.0, 0.0));
    }

    #[test]
    fn turn_size_slides_between_the_frames() {
        assert_eq!(turn_size((1200, 1920), (1920, 1200), 0.0), (1200.0, 1920.0));
        assert_eq!(turn_size((1200, 1920), (1920, 1200), 1.0), (1920.0, 1200.0));
        assert_eq!(turn_size((1200, 1920), (1920, 1200), 0.5), (1560.0, 1560.0));
        assert_eq!(turn_size((10, 10), (20, 20), 7.0), (20.0, 20.0));
    }

    #[test]
    fn points_turn_like_rotation_area() {
        // a one-pixel area and its top-left corner must land on the same panel pixel
        let (sw, sh) = (1920usize, 1200usize);
        for rot in ALL {
            for (x, y) in [(0usize, 0usize), (10, 20), (1919, 1199), (500, 7)] {
                let a = rot.area(crate::gfx::Area { x0: x, y0: y, x1: x + 1, y1: y + 1 }, sw, sh);
                let q = layer_quad(rot, r(x as i32, y as i32, 1, 1), [0.0, 0.0, 1.0, 1.0], sw, sh);
                assert_eq!(bounds(&q), r(a.x0 as i32, a.y0 as i32, 1, 1), "{rot:?} ({x}, {y})");
            }
        }
    }

    #[test]
    fn the_full_landscape_frame_covers_the_portrait_panel() {
        for rot in [Rotation::Cw, Rotation::Ccw] {
            let q = layer_quad(rot, r(0, 0, 1920, 1200), [0.0, 0.0, 1.0, 1.0], 1920, 1200);
            assert_eq!(bounds(&q), r(0, 0, 1200, 1920), "{rot:?}");
        }
        for rot in [Rotation::None, Rotation::Half] {
            let q = layer_quad(rot, r(0, 0, 1200, 1920), [0.0, 0.0, 1.0, 1.0], 1200, 1920);
            assert_eq!(bounds(&q), r(0, 0, 1200, 1920), "{rot:?}");
        }
    }

    #[test]
    fn the_texture_top_left_follows_the_turn() {
        let uv = [0.0, 0.0, 1.0, 1.0];
        // clockwise: the frame's top row becomes the panel's right column, so its top-left
        // corner is the panel's top-right
        let q = layer_quad(Rotation::Cw, r(0, 0, 1920, 1200), uv, 1920, 1200);
        assert_eq!(uv_at(&q, 1200.0, 0.0), (0.0, 0.0));
        assert_eq!(uv_at(&q, 0.0, 1920.0), (1.0, 1.0));
        // counter-clockwise: the top row becomes the left column, read bottom to top
        let q = layer_quad(Rotation::Ccw, r(0, 0, 1920, 1200), uv, 1920, 1200);
        assert_eq!(uv_at(&q, 0.0, 1920.0), (0.0, 0.0));
        assert_eq!(uv_at(&q, 1200.0, 0.0), (1.0, 1.0));
        let q = layer_quad(Rotation::Half, r(0, 0, 1200, 1920), uv, 1200, 1920);
        assert_eq!(uv_at(&q, 1200.0, 1920.0), (0.0, 0.0));
        let q = layer_quad(Rotation::None, r(0, 0, 1200, 1920), uv, 1200, 1920);
        assert_eq!(uv_at(&q, 0.0, 0.0), (0.0, 0.0));
        assert_eq!(uv_at(&q, 1200.0, 1920.0), (1.0, 1.0));
    }

    #[test]
    fn crop_uvs_ride_on_the_corners() {
        // a 1280x720 picture inside a 1280x736 buffer: v1 stops short of the padding rows
        let uv = crate::gpu::texture::uv(r(0, 0, 1280, 720), 1280, 736);
        assert!((uv[3] - 720.0 / 736.0).abs() < 1e-6);
        for rot in ALL {
            let q = layer_quad(rot, r(0, 60, 1920, 1080), uv, 1920, 1200);
            let (us, vs): (Vec<f32>, Vec<f32>) = q.iter().map(|v| (v[2], v[3])).unzip();
            assert!(us.iter().all(|&u| u == uv[0] || u == uv[2]), "{rot:?}");
            assert!(vs.iter().all(|&v| v == uv[1] || v == uv[3]), "{rot:?}");
        }
    }

    #[test]
    fn letterboxed_video_turns_into_a_panel_column() {
        // 1920x1080 picture centred in the 1920x1200 landscape frame
        let dst = r(0, 60, 1920, 1080);
        let q = layer_quad(Rotation::Cw, dst, [0.0, 0.0, 1.0, 1.0], 1920, 1200);
        assert_eq!(bounds(&q), r(60, 0, 1080, 1920));
        assert_eq!(to_panel(Rotation::Cw, dst, 1920, 1200), Some(r(60, 0, 1080, 1920)));
        assert_eq!(to_panel(Rotation::Ccw, dst, 1920, 1200), Some(r(60, 0, 1080, 1920)));
    }

    #[test]
    fn to_panel_clips_first() {
        assert_eq!(to_panel(Rotation::None, r(-10, -10, 20, 20), 100, 50), Some(r(0, 0, 10, 10)));
        assert_eq!(to_panel(Rotation::Cw, r(200, 0, 10, 10), 100, 50), None);
        assert_eq!(to_panel(Rotation::Half, r(0, 0, 10, 10), 100, 50), Some(r(90, 40, 10, 10)));
    }

    #[test]
    fn fit_rect_matches_the_cpu_letterbox() {
        assert_eq!(fit_rect(1280, 720, (0.0, 0.0, 1920.0, 1200.0)), r(0, 60, 1920, 1080));
        assert_eq!(fit_rect(1920, 1080, (0.0, 0.0, 1200.0, 1920.0)), r(0, 623, 1200, 675));
        assert_eq!(fit_rect(1280, 720, (100.0, 0.0, 1280.0, 720.0)), r(100, 0, 1280, 720));
    }
}
