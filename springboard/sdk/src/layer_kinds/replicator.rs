//! Replicator content: draws a source several times, each copy one more step of a transform, a colour offset and a
//! time delay further along than the one before. The transform is the model in `bifrost_render::replicator`; only its
//! 2D part (rotation, scale, shear, x and y translation) shows in a flat image, the z column is ignored.
use super::raster::{pack_premul, sample, unpack};
use bifrost_render::gfx::{self, Image};
pub use bifrost_render::replicator::{translation, Mat4, Replicator, IDENTITY};

#[derive(Clone, Debug, PartialEq)]
pub struct ReplicatorContent {
    pub replicator: Replicator,
    /// The point, in output points, the per-instance rotation and scale act around.
    pub anchor: (f32, f32),
}

impl ReplicatorContent {
    pub fn new(replicator: Replicator, anchor: (f32, f32)) -> Self {
        ReplicatorContent { replicator, anchor }
    }

    /// Draws `source` once per instance into a `w` x `h` image. `scale` is device pixels per point (translations are in
    /// points). Instance 0 is drawn first; later instances go over it.
    pub fn render(&self, w: usize, h: usize, scale: f32, source: &Image) -> Image {
        self.render_with(w, h, scale, &mut |_, _| source.clone())
    }

    /// Like `render`, asking `source` for the image of each instance and its time offset in seconds, so an animated
    /// source can be shown at a different moment per copy.
    pub fn render_with(&self, w: usize, h: usize, scale: f32, source: &mut dyn FnMut(usize, f64) -> Image) -> Image {
        let mut out = Image::new(w, h);
        let anchor = (self.anchor.0 * scale, self.anchor.1 * scale);
        for i in 0..self.replicator.instances() {
            let src = source(i, self.replicator.delay_of(i));
            let m = self.replicator.matrix(i);
            let tint = self.replicator.color_of(i, [1.0; 4]);
            draw_instance(&mut out, &src, &m, scale, anchor, tint);
        }
        out
    }
}

fn draw_instance(out: &mut Image, src: &Image, m: &Mat4, scale: f32, anchor: (f32, f32), tint: [f32; 4]) {
    let (a, b, c, d) = (m[0][0] as f32, m[0][1] as f32, m[1][0] as f32, m[1][1] as f32);
    let t = (m[0][3] as f32 * scale, m[1][3] as f32 * scale);
    let det = a * d - b * c;
    if det.abs() < 1e-9 || src.w == 0 || src.h == 0 {
        return;
    }
    let fwd = |x: f32, y: f32| (anchor.0 + a * (x - anchor.0) + b * (y - anchor.1) + t.0, anchor.1 + c * (x - anchor.0) + d * (y - anchor.1) + t.1);
    let corners = [fwd(0.0, 0.0), fwd(src.w as f32, 0.0), fwd(0.0, src.h as f32), fwd(src.w as f32, src.h as f32)];
    let x0 = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let y0 = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let x1 = (corners.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32).min(out.w as i32);
    let y1 = (corners.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32).min(out.h as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let (qx, qy) = (x as f32 + 0.5 - anchor.0 - t.0, y as f32 + 0.5 - anchor.1 - t.1);
            let sx = anchor.0 + (d * qx - b * qy) / det;
            let sy = anchor.1 + (-c * qx + a * qy) / det;
            if sx < 0.0 || sy < 0.0 || sx >= src.w as f32 || sy >= src.h as f32 {
                continue;
            }
            let p = sample(src, sx, sy);
            if p[3] <= 0.0 {
                continue;
            }
            let ta = tint[3];
            let q = pack_premul([p[0] * tint[0] * ta, p[1] * tint[1] * ta, p[2] * tint[2] * ta, p[3] * ta]);
            let dst = &mut out.px[y as usize * out.w + x as usize];
            *dst = gfx::over(*dst, q);
        }
    }
}

/// The premultiplied colour of `px` after the tint of instance `i`; exposed for callers that tint a flat colour.
pub fn tinted(r: &Replicator, i: usize, px: u32) -> u32 {
    let p = unpack(px);
    let t = r.color_of(i, [1.0; 4]);
    pack_premul([p[0] * t[0] * t[3], p[1] * t[1] * t[3], p[2] * t[2] * t[3], p[3] * t[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::gfx::rgba;

    fn dot() -> Image {
        // A 2x2 opaque red block.
        Image { w: 2, h: 2, px: vec![rgba(255, 0, 0, 255); 4] }
    }

    fn at(img: &Image, x: usize, y: usize) -> u32 {
        img.px[y * img.w + x]
    }

    #[test]
    fn instances_step_by_the_translation() {
        let r = Replicator { count: 3, transform: translation(4.0, 0.0, 0.0), ..Default::default() };
        let img = ReplicatorContent::new(r, (0.0, 0.0)).render(16, 4, 1.0, &dot());
        assert_eq!(at(&img, 0, 0), rgba(255, 0, 0, 255));
        assert_eq!(at(&img, 2, 0), 0);
        assert_eq!(at(&img, 4, 1), rgba(255, 0, 0, 255));
        assert_eq!(at(&img, 8, 0), rgba(255, 0, 0, 255));
        assert_eq!(at(&img, 12, 0), 0);
    }

    #[test]
    fn translation_is_in_points_and_scales_to_pixels() {
        let r = Replicator { count: 2, transform: translation(2.0, 0.0, 0.0), ..Default::default() };
        let img = ReplicatorContent::new(r, (0.0, 0.0)).render(16, 4, 2.0, &dot());
        assert_eq!(at(&img, 4, 0), rgba(255, 0, 0, 255), "two points at scale 2 is four pixels");
        assert_eq!(at(&img, 3, 0), 0);
    }

    #[test]
    fn the_colour_offset_tints_later_instances() {
        let r = Replicator { count: 2, transform: translation(4.0, 0.0, 0.0), color_offset: [-1.0, 0.0, 0.0, 0.0], ..Default::default() };
        let img = ReplicatorContent::new(r, (0.0, 0.0)).render(8, 4, 1.0, &dot());
        assert_eq!(at(&img, 0, 0), rgba(255, 0, 0, 255));
        assert_eq!(at(&img, 4, 0), rgba(0, 0, 0, 255), "the red channel is offset to nothing");
    }

    #[test]
    fn the_alpha_colour_fades_every_instance() {
        let r = Replicator { count: 1, color: [1.0, 1.0, 1.0, 0.5], ..Default::default() };
        let img = ReplicatorContent::new(r, (0.0, 0.0)).render(4, 4, 1.0, &dot());
        let p = at(&img, 0, 0);
        assert!(((p >> 24) as i32 - 128).abs() <= 1);
        assert!(((p & 255) as i32 - 128).abs() <= 1, "premultiplied red follows alpha");
    }

    #[test]
    fn a_quarter_turn_about_the_anchor_moves_the_copy() {
        // 90 degrees: x' = -y, y' = x (clockwise on screen, y down) about (4, 4).
        let mut rot = IDENTITY;
        rot[0][0] = 0.0;
        rot[0][1] = -1.0;
        rot[1][0] = 1.0;
        rot[1][1] = 0.0;
        let r = Replicator { count: 2, transform: rot, ..Default::default() };
        let src = Image { w: 8, h: 8, px: {
            let mut px = vec![0; 64];
            px[4 * 8 + 6] = rgba(255, 255, 255, 255);
            px
        } };
        let img = ReplicatorContent::new(r, (4.0, 4.0)).render(8, 8, 1.0, &src);
        // The pixel centre (6.5, 4.5) is offset (+2.5, +0.5) from the anchor; the turn maps it to (3.5, 6.5), pixel (3, 6).
        assert_eq!(at(&img, 6, 4), rgba(255, 255, 255, 255));
        assert_eq!(at(&img, 3, 6), rgba(255, 255, 255, 255));
    }

    #[test]
    fn the_source_callback_sees_each_instance_and_its_delay() {
        let r = Replicator { count: 3, delay: 0.5, ..Default::default() };
        let mut seen = Vec::new();
        ReplicatorContent::new(r, (0.0, 0.0)).render_with(4, 4, 1.0, &mut |i, d| {
            seen.push((i, d));
            Image::new(1, 1)
        });
        assert_eq!(seen, vec![(0, 0.0), (1, 0.5), (2, 1.0)]);
    }

    #[test]
    fn a_singular_transform_draws_nothing_for_that_instance() {
        let mut flat = IDENTITY;
        flat[0][0] = 0.0;
        let r = Replicator { count: 2, transform: flat, ..Default::default() };
        let img = ReplicatorContent::new(r, (0.0, 0.0)).render(4, 4, 1.0, &dot());
        assert_eq!(at(&img, 0, 0), rgba(255, 0, 0, 255), "instance 0 is the plain source");
    }
}
