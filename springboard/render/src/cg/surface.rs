//! Path, line, ellipse and arc drawing on a pixel [`Surface`], through the scan converter. Colours are premultiplied
//! packed pixels, like the other Surface calls.
use super::path::{Affine, Path, Point, Rect};
use super::raster::{self, Coverage, FillRule, LineCap, StrokeStyle};
use crate::gfx::{over, scale, Surface};

impl Surface<'_> {
    /// Blends `color` through a coverage mask (source-over); pixels outside the surface are skipped.
    pub fn fill_coverage(&mut self, cov: &Coverage, color: u32) {
        if cov.is_empty() {
            return;
        }
        for row in 0..cov.h {
            let y = cov.y + row as i32;
            if y < self.y0 as i32 || y >= self.h as i32 {
                continue;
            }
            for col in 0..cov.w {
                let x = cov.x + col as i32;
                let a = cov.a[row * cov.w + col] as u32;
                if a == 0 || x < 0 || x >= self.w as i32 {
                    continue;
                }
                let src = if a == 255 { color } else { scale(color, a) };
                let d = self.at(x as usize, y as usize);
                *d = over(*d, src);
            }
        }
    }

    /// Fills `path` mapped through `t`.
    pub fn fill_path(&mut self, path: &Path, rule: FillRule, t: &Affine, color: u32) {
        let cov = raster::fill(path, rule, t, None, self.w, self.h);
        self.fill_coverage(&cov, color);
    }

    /// Strokes `path` with `style`, then maps the outline through `t`.
    pub fn stroke_path(&mut self, path: &Path, style: &StrokeStyle, t: &Affine, color: u32) {
        let cov = raster::fill_stroke(path, style, t, None, self.w, self.h);
        self.fill_coverage(&cov, color);
    }

    /// A straight line of `width` pixels with the given end caps.
    pub fn stroke_line(&mut self, from: Point, to: Point, width: f32, cap: LineCap, color: u32) {
        let mut p = Path::new();
        p.add_lines(&[from, to]);
        let style = StrokeStyle { width, cap, ..Default::default() };
        self.stroke_path(&p, &style, &Affine::IDENTITY, color);
    }

    pub fn fill_ellipse(&mut self, r: Rect, color: u32) {
        self.fill_path(&Path::from_ellipse(r), FillRule::NonZero, &Affine::IDENTITY, color);
    }

    pub fn stroke_ellipse(&mut self, r: Rect, width: f32, color: u32) {
        let style = StrokeStyle { width, ..Default::default() };
        self.stroke_path(&Path::from_ellipse(r), &style, &Affine::IDENTITY, color);
    }

    /// An arc about `center` from angle `start` to `end` (radians, 0 along +x); see [`Path::add_arc`].
    pub fn stroke_arc(&mut self, center: Point, radius: f32, start: f32, end: f32, clockwise: bool, width: f32, cap: LineCap, color: u32) {
        let mut p = Path::new();
        p.add_arc(center, radius, start, end, clockwise);
        let style = StrokeStyle { width, cap, ..Default::default() };
        self.stroke_path(&p, &style, &Affine::IDENTITY, color);
    }

    /// A filled pie wedge: the arc closed through the centre.
    pub fn fill_arc(&mut self, center: Point, radius: f32, start: f32, end: f32, clockwise: bool, color: u32) {
        let mut p = Path::new();
        p.move_to(center);
        p.add_arc(center, radius, start, end, clockwise);
        p.close();
        self.fill_path(&p, FillRule::NonZero, &Affine::IDENTITY, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(buf: &mut Vec<u32>, w: usize, h: usize) -> Surface<'_> {
        Surface { w, h, stride: w, px: buf, y0: 0 }
    }
    const RED: u32 = 0xFF00_00FF;

    #[test]
    fn ellipse_fill_hits_the_centre_not_the_corner() {
        let mut buf = vec![0u32; 20 * 20];
        let mut s = surface(&mut buf, 20, 20);
        s.fill_ellipse(Rect::new(2.0, 2.0, 16.0, 16.0), RED);
        assert_eq!(*s.at(10, 10), RED);
        assert_eq!(*s.at(2, 2), 0);
    }

    #[test]
    fn line_and_arc_draw() {
        let mut buf = vec![0u32; 20 * 20];
        let mut s = surface(&mut buf, 20, 20);
        s.stroke_line((2.0, 10.0), (18.0, 10.0), 2.0, LineCap::Butt, RED);
        assert_eq!(*s.at(10, 9), RED);
        assert_eq!(*s.at(10, 5), 0);
        s.stroke_arc((10.0, 10.0), 6.0, 0.0, std::f32::consts::PI, false, 2.0, LineCap::Butt, RED);
        // The arc's edges are anti-aliased, so the bottom pixel is red at most of its coverage, not a hard RED
        // (the old hard-edge expectation). Only the red channel and alpha carry it: no green or blue.
        let bottom = *s.at(10, 16);
        assert!(bottom >> 24 >= 0xE0, "bottom of the half circle (y grows down): {bottom:#x}");
        assert_eq!(bottom & 0x00FF_FF00, 0, "red only");
        assert_eq!(*s.at(10, 4), 0);
    }

    #[test]
    fn stroke_ellipse_leaves_the_middle_empty() {
        let mut buf = vec![0u32; 20 * 20];
        let mut s = surface(&mut buf, 20, 20);
        s.stroke_ellipse(Rect::new(2.0, 2.0, 16.0, 16.0), 2.0, RED);
        assert_eq!(*s.at(10, 10), 0);
        assert_eq!(*s.at(2, 10), RED);
    }

    #[test]
    fn clipped_to_the_surface() {
        let mut buf = vec![0u32; 8 * 8];
        let mut s = surface(&mut buf, 8, 8);
        s.fill_ellipse(Rect::new(-20.0, -20.0, 60.0, 60.0), RED);
        assert_eq!(*s.at(7, 7), RED);
    }
}
