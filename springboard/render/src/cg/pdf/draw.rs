//! Page drawing through `cg::context`. [`ContextSink`] paints each resolved operation that [`super::interpret`]
//! reports into a drawing context, and [`render_page`] rasterises a whole page.
//!
//! The context stays at the identity transform: paths, images and shadings are mapped to device pixels here, so the
//! context's own transform never has to be unwound. Images and smooth shadings are resampled into a device-aligned
//! picture (nearest neighbour) and composited with `draw_image`, which keeps the clip and the blend mode.

use super::doc::Doc;
use super::interpret;
use super::page::{self, Matrix, Page, PageBox};
use super::sink::{Blend, FillRule, LineCap, LineJoin, PathSeg, PdfImage, PdfSink, Rgb, Shading, ShadingKind, Stroke, Vertex};
use crate::cg::context as cx;
use crate::cg::path::Path;
use crate::cg::raster;
use crate::gfx::{rgba, Image};

/// Page rasters larger than this on either side are refused.
pub const MAX_SIDE: usize = 8192;

/// Paints PDF drawing operations into a drawing context.
pub struct ContextSink<'c, 'a> {
    ctx: &'c mut cx::DrawContext<'a>,
}

impl<'c, 'a> ContextSink<'c, 'a> {
    pub fn new(ctx: &'c mut cx::DrawContext<'a>) -> Self {
        ContextSink { ctx }
    }
}

impl PdfSink for ContextSink<'_, '_> {
    fn save(&mut self) {
        self.ctx.save_gstate();
    }

    fn restore(&mut self) {
        self.ctx.restore_gstate();
    }

    fn clip(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule) {
        let p = device_path(path, ctm);
        self.ctx.clip_to_path(&p, winding(rule));
    }

    fn fill(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule, color: Rgb, alpha: f32, blend: Blend) {
        let p = device_path(path, ctm);
        self.ctx.save_gstate();
        self.ctx.set_fill_rgb(color[0], color[1], color[2], 1.0);
        self.ctx.set_alpha(alpha);
        self.ctx.set_blend_mode(blend_mode(blend));
        self.ctx.fill(&p, winding(rule));
        self.ctx.restore_gstate();
    }

    fn stroke(&mut self, path: &[PathSeg], ctm: &Matrix, stroke: &Stroke, color: Rgb, alpha: f32, blend: Blend) {
        let p = device_path(path, ctm);
        let s = scale_of(ctm);
        self.ctx.save_gstate();
        self.ctx.set_stroke_rgb(color[0], color[1], color[2], 1.0);
        self.ctx.set_alpha(alpha);
        self.ctx.set_blend_mode(blend_mode(blend));
        // hairlines stay visible: a zero width still draws a thin line
        self.ctx.set_line_width(((stroke.width * s) as f32).max(0.75));
        self.ctx.set_line_cap(cap(stroke.cap));
        self.ctx.set_line_join(join(stroke.join));
        self.ctx.set_miter_limit(stroke.miter_limit as f32);
        if !stroke.dash.is_empty() {
            let dash: Vec<f32> = stroke.dash.iter().map(|d| (d * s) as f32).collect();
            self.ctx.set_line_dash((stroke.phase * s) as f32, &dash);
        }
        self.ctx.stroke(&p);
        self.ctx.restore_gstate();
    }

    fn image(&mut self, img: &PdfImage, ctm: &Matrix, color: Rgb, alpha: f32, blend: Blend, _interpolate: bool) {
        let (cw, ch) = self.ctx.size();
        let Some(bx) = device_box(ctm, cw, ch) else { return };
        let pixels = warp(img, ctm, bx, color);
        self.ctx.save_gstate();
        self.ctx.set_alpha(alpha);
        self.ctx.set_blend_mode(blend_mode(blend));
        self.ctx.draw_image(&pixels, cx::Rect::new(bx.0 as f32, bx.1 as f32, bx.2 as f32, bx.3 as f32));
        self.ctx.restore_gstate();
    }

    fn shade(&mut self, shading: &Shading, ctm: &Matrix, alpha: f32, blend: Blend) {
        let (cw, ch) = self.ctx.size();
        let clip = self.ctx.clip_bounding_box();
        let Some(bx) = clamp_box(clip.x, clip.y, clip.x + clip.w, clip.y + clip.h, cw, ch) else { return };
        let pixels = shade_image(shading, ctm, bx);
        self.ctx.save_gstate();
        self.ctx.set_alpha(alpha);
        self.ctx.set_blend_mode(blend_mode(blend));
        self.ctx.draw_image(&pixels, cx::Rect::new(bx.0 as f32, bx.1 as f32, bx.2 as f32, bx.3 as f32));
        self.ctx.restore_gstate();
    }
}

fn winding(rule: FillRule) -> raster::FillRule {
    match rule {
        FillRule::NonZero => raster::FillRule::NonZero,
        FillRule::EvenOdd => raster::FillRule::EvenOdd,
    }
}

fn cap(c: LineCap) -> cx::LineCap {
    match c {
        LineCap::Butt => cx::LineCap::Butt,
        LineCap::Round => cx::LineCap::Round,
        LineCap::Square => cx::LineCap::Square,
    }
}

fn join(j: LineJoin) -> cx::LineJoin {
    match j {
        LineJoin::Miter => cx::LineJoin::Miter,
        LineJoin::Round => cx::LineJoin::Round,
        LineJoin::Bevel => cx::LineJoin::Bevel,
    }
}

fn blend_mode(b: Blend) -> cx::BlendMode {
    match b {
        Blend::Normal => cx::BlendMode::Normal,
        Blend::Multiply => cx::BlendMode::Multiply,
        Blend::Screen => cx::BlendMode::Screen,
        Blend::Overlay => cx::BlendMode::Overlay,
        Blend::Darken => cx::BlendMode::Darken,
        Blend::Lighten => cx::BlendMode::Lighten,
        Blend::ColorDodge => cx::BlendMode::ColorDodge,
        Blend::ColorBurn => cx::BlendMode::ColorBurn,
        Blend::HardLight => cx::BlendMode::HardLight,
        Blend::SoftLight => cx::BlendMode::SoftLight,
        Blend::Difference => cx::BlendMode::Difference,
        Blend::Exclusion => cx::BlendMode::Exclusion,
        Blend::Hue => cx::BlendMode::Hue,
        Blend::Saturation => cx::BlendMode::Saturation,
        Blend::Color => cx::BlendMode::Color,
        Blend::Luminosity => cx::BlendMode::Luminosity,
    }
}

/// A point through `m`, in device pixels.
fn dev(m: &Matrix, x: f64, y: f64) -> (f32, f32) {
    let (u, v) = page::apply(m, x, y);
    (u as f32, v as f32)
}

/// Path segments mapped through `m` into a device-space path.
fn device_path(segs: &[PathSeg], m: &Matrix) -> Path {
    let mut p = Path::new();
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => p.move_to(dev(m, x, y)),
            PathSeg::Line(x, y) => p.line_to(dev(m, x, y)),
            PathSeg::Cubic(x1, y1, x2, y2, x, y) => p.cubic_to(dev(m, x1, y1), dev(m, x2, y2), dev(m, x, y)),
            PathSeg::Close => p.close(),
        }
    }
    p
}

/// The mean scale of `m` (the square root of its area factor), used to size line widths and dashes.
fn scale_of(m: &Matrix) -> f64 {
    (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
}

fn channel(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The device pixel box `(x, y, w, h)` inside a `cw` x `ch` canvas, from float corners. None when it is empty.
fn clamp_box(x0: f32, y0: f32, x1: f32, y1: f32, cw: usize, ch: usize) -> Option<(usize, usize, usize, usize)> {
    let bx0 = x0.floor().max(0.0);
    let by0 = y0.floor().max(0.0);
    let bx1 = x1.ceil().min(cw as f32);
    let by1 = y1.ceil().min(ch as f32);
    if !(bx1 > bx0 && by1 > by0) {
        return None;
    }
    Some((bx0 as usize, by0 as usize, (bx1 - bx0) as usize, (by1 - by0) as usize))
}

/// The device box covered by the unit square under `m`, clipped to the canvas.
fn device_box(m: &Matrix, cw: usize, ch: usize) -> Option<(usize, usize, usize, usize)> {
    let pts = [dev(m, 0.0, 0.0), dev(m, 1.0, 0.0), dev(m, 0.0, 1.0), dev(m, 1.0, 1.0)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    clamp_box(x0, y0, x1, y1, cw, ch)
}

/// The picture as it lands in the device box `bx` under `m`: each device pixel centre is mapped back through the
/// inverse of `m` to the unit square and sampled nearest. A stencil paints `color` with its alpha.
fn warp(img: &PdfImage, m: &Matrix, bx: (usize, usize, usize, usize), color: Rgb) -> Image {
    let (x0, y0, w, h) = bx;
    let mut out = Image::new(w, h);
    let Some(inv) = page::invert(m) else { return out };
    if img.w == 0 || img.h == 0 {
        return out;
    }
    let c = [channel(color[0]), channel(color[1]), channel(color[2])];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = ((x0 + x) as f64 + 0.5, (y0 + y) as f64 + 0.5);
            let (u, v) = page::apply(&inv, dx, dy);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            // image row 0 is at the top of the unit square (v = 1)
            let col = ((u * img.w as f64) as usize).min(img.w - 1);
            let row = (((1.0 - v) * img.h as f64) as usize).min(img.h - 1);
            let Some(p) = img.rgba.get((row * img.w + col) * 4..(row * img.w + col) * 4 + 4) else { continue };
            out.px[y * w + x] = if img.stencil { rgba(c[0], c[1], c[2], p[3]) } else { rgba(p[0], p[1], p[2], p[3]) };
        }
    }
    out
}

/// The shading rasterised over the device box `bx`. Smooth shadings are evaluated per pixel centre in shading space
/// (through the inverse of `ctm`); triangle meshes are rasterised with their colours interpolated.
fn shade_image(sh: &Shading, ctm: &Matrix, bx: (usize, usize, usize, usize)) -> Image {
    let (x0, y0, w, h) = bx;
    let mut out = Image::new(w, h);
    if let ShadingKind::Triangles(tris) = &sh.kind {
        for t in tris {
            triangle(&mut out, (x0, y0), ctm, t);
        }
        return out;
    }
    let Some(inv) = page::invert(ctm) else { return out };
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = ((x0 + x) as f64 + 0.5, (y0 + y) as f64 + 0.5);
            let (px, py) = page::apply(&inv, dx, dy);
            if let Some(c) = shade_at(&sh.kind, px, py) {
                out.px[y * w + x] = rgba(channel(c[0]), channel(c[1]), channel(c[2]), 255);
            }
        }
    }
    out
}

/// Colour of a smooth shading at shading-space point (x, y); None outside its extent.
fn shade_at(kind: &ShadingKind, x: f64, y: f64) -> Option<Rgb> {
    match kind {
        ShadingKind::Axial { x0, y0, x1, y1, extend, ramp } => {
            let (x0, y0, x1, y1) = (*x0, *y0, *x1, *y1);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let den = dx * dx + dy * dy;
            if den == 0.0 {
                return None;
            }
            let mut t = ((x - x0) * dx + (y - y0) * dy) / den;
            if t < 0.0 {
                if !extend.0 {
                    return None;
                }
                t = 0.0;
            }
            if t > 1.0 {
                if !extend.1 {
                    return None;
                }
                t = 1.0;
            }
            ramp_at(ramp, t)
        }
        ShadingKind::Radial { x0, y0, r0, x1, y1, r1, extend, ramp } => {
            let (cx0, cy0, r0, cx1, cy1, r1) = (*x0, *y0, *r0, *x1, *y1, *r1);
            let (cdx, cdy, dr) = (cx1 - cx0, cy1 - cy0, r1 - r0);
            let (pdx, pdy) = (x - cx0, y - cy0);
            // |p - c(s)| = r(s) with c(s) = c0 + s (c1 - c0) and r(s) = r0 + s (r1 - r0): a quadratic in s
            let a = cdx * cdx + cdy * cdy - dr * dr;
            let b = pdx * cdx + pdy * cdy + r0 * dr;
            let c = pdx * pdx + pdy * pdy - r0 * r0;
            let mut roots: Vec<f64> = vec![];
            if a.abs() < 1e-12 {
                if b.abs() > 1e-12 {
                    roots.push(c / (2.0 * b));
                }
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    return None;
                }
                let sq = disc.sqrt();
                roots.push((b + sq) / a);
                roots.push((b - sq) / a);
            }
            // the largest parameter whose circle has a non-negative radius and lies in the extent wins
            roots.sort_by(|p, q| q.total_cmp(p));
            for s in roots {
                if r0 + s * dr < 0.0 {
                    continue;
                }
                let inside = if s < 0.0 {
                    extend.0
                } else if s > 1.0 {
                    extend.1
                } else {
                    true
                };
                if inside {
                    return ramp_at(ramp, s);
                }
            }
            None
        }
        ShadingKind::Grid { matrix, w, h, colors } => {
            let (w, h) = (*w, *h);
            if w == 0 || h == 0 || colors.len() < w * h {
                return None;
            }
            let inv = page::invert(matrix)?;
            let (u, v) = page::apply(&inv, x, y);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                return None;
            }
            let col = ((u * (w - 1) as f64).round() as usize).min(w - 1);
            let row = ((v * (h - 1) as f64).round() as usize).min(h - 1);
            colors.get(row * w + col).copied()
        }
        ShadingKind::Triangles(_) => None,
    }
}

/// The colour at parameter `t` (0 to 1) of an evenly spaced ramp.
fn ramp_at(ramp: &[Rgb], t: f64) -> Option<Rgb> {
    if ramp.is_empty() {
        return None;
    }
    let i = (t.clamp(0.0, 1.0) * (ramp.len() - 1) as f64).round() as usize;
    ramp.get(i).copied()
}

/// Paints one Gouraud triangle (shading-space vertices) into `out`, whose pixel (0, 0) is device pixel `origin`.
fn triangle(out: &mut Image, origin: (usize, usize), ctm: &Matrix, t: &[Vertex; 3]) {
    let p: [(f64, f64); 3] = [page::apply(ctm, t[0].x, t[0].y), page::apply(ctm, t[1].x, t[1].y), page::apply(ctm, t[2].x, t[2].y)];
    let den = (p[1].1 - p[2].1) * (p[0].0 - p[2].0) + (p[2].0 - p[1].0) * (p[0].1 - p[2].1);
    if den.abs() < 1e-12 {
        return;
    }
    let (w, h) = (out.w as i64, out.h as i64);
    let min_x = p.iter().map(|q| q.0).fold(f64::MAX, f64::min);
    let max_x = p.iter().map(|q| q.0).fold(f64::MIN, f64::max);
    let min_y = p.iter().map(|q| q.1).fold(f64::MAX, f64::min);
    let max_y = p.iter().map(|q| q.1).fold(f64::MIN, f64::max);
    let (ox, oy) = (origin.0 as i64, origin.1 as i64);
    let lo_x = (min_x.floor() as i64 - ox).max(0);
    let hi_x = (max_x.ceil() as i64 - ox).min(w);
    let lo_y = (min_y.floor() as i64 - oy).max(0);
    let hi_y = (max_y.ceil() as i64 - oy).min(h);
    for y in lo_y..hi_y {
        for x in lo_x..hi_x {
            let (px, py) = ((ox + x) as f64 + 0.5, (oy + y) as f64 + 0.5);
            let l0 = ((p[1].1 - p[2].1) * (px - p[2].0) + (p[2].0 - p[1].0) * (py - p[2].1)) / den;
            let l1 = ((p[2].1 - p[0].1) * (px - p[2].0) + (p[0].0 - p[2].0) * (py - p[2].1)) / den;
            let l2 = 1.0 - l0 - l1;
            const EDGE: f64 = -1e-9;
            if l0 < EDGE || l1 < EDGE || l2 < EDGE {
                continue;
            }
            let (c0, c1, c2) = (t[0].color, t[1].color, t[2].color);
            let mut col = [0.0f64; 3];
            for k in 0..3 {
                col[k] = c0[k] as f64 * l0 + c1[k] as f64 * l1 + c2[k] as f64 * l2;
            }
            out.px[y as usize * out.w + x as usize] = rgba(channel(col[0] as f32), channel(col[1] as f32), channel(col[2] as f32), 255);
        }
    }
}

/// Rasterises `page` to exactly `w` x `h` pixels. The crop box fills the picture (keeping its aspect, centred) over
/// white paper. None for an empty or oversized picture.
pub fn render_page(doc: &Doc, page: &Page, w: usize, h: usize) -> Option<Image> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let ctm = page.transform(PageBox::Crop, (0.0, 0.0, w as f64, h as f64), true);
    let mut bm = cx::Bitmap::new(w, h);
    {
        let mut ctx = bm.context();
        ctx.set_fill_rgb(1.0, 1.0, 1.0, 1.0);
        ctx.fill_rect(cx::Rect::new(0.0, 0.0, w as f32, h as f32));
        let mut sink = ContextSink::new(&mut ctx);
        interpret::draw_page(doc, page, ctm, &mut sink);
    }
    Some(bm.make_image())
}

/// The picture size [`render_page_fit`] gives `page` in a `max_w` x `max_h` box.
pub fn fit_size(page: &Page, max_w: usize, max_h: usize) -> Option<(usize, usize)> {
    let (pw, ph) = page.size(PageBox::Crop);
    if !(pw > 0.0 && ph > 0.0) {
        return None;
    }
    let k = (max_w as f64 / pw).min(max_h as f64 / ph);
    let w = (pw * k).round().max(1.0) as usize;
    let h = (ph * k).round().max(1.0) as usize;
    Some((w, h))
}

/// Rasterises `page` scaled to fit inside `max_w` x `max_h` pixels, keeping its aspect.
pub fn render_page_fit(doc: &Doc, page: &Page, max_w: usize, max_h: usize) -> Option<Image> {
    let (w, h) = fit_size(page, max_w, max_h)?;
    render_page(doc, page, w, h)
}

/// Page `index` (zero based) of the PDF in `data`, scaled to fit `max_w` x `max_h` pixels.
pub fn render_file_page(data: Vec<u8>, index: usize, max_w: usize, max_h: usize) -> Result<Image, String> {
    let doc = Doc::open(data)?;
    let list = page::pages(&doc);
    let page = list.get(index).ok_or("the PDF has no such page")?;
    render_page_fit(&doc, page, max_w, max_h).ok_or_else(|| "the page could not be drawn".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;

    #[test]
    fn warped_images_sample_nearest_and_keep_their_colours() {
        let img = PdfImage { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255], stencil: false };
        // the unit square covers a 4 x 2 device box
        let out = warp(&img, &[4.0, 0.0, 0.0, 2.0, 0.0, 0.0], (0, 0, 4, 2), [0.0; 3]);
        assert_eq!((out.w, out.h), (4, 2));
        assert_eq!(out.px[0], rgba(255, 0, 0, 255));
        assert_eq!(out.px[3], rgba(0, 0, 255, 255));
    }

    #[test]
    fn stencils_paint_their_colour_with_the_mask_alpha() {
        let img = PdfImage { w: 1, h: 1, rgba: vec![0, 0, 0, 128], stencil: true };
        let out = warp(&img, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], (0, 0, 1, 1), [1.0, 1.0, 1.0]);
        assert_eq!(out.px[0], rgba(255, 255, 255, 128));
    }

    #[test]
    fn device_boxes_are_clipped_to_the_canvas() {
        assert_eq!(clamp_box(-5.0, -5.0, 3.0, 2.0, 10, 10), Some((0, 0, 3, 2)));
        assert_eq!(clamp_box(20.0, 0.0, 30.0, 4.0, 10, 10), None);
        assert_eq!(device_box(&[10.0, 0.0, 0.0, 10.0, 1.0, 1.0], 100, 100), Some((1, 1, 10, 10)));
    }

    #[test]
    fn axial_shadings_follow_the_ramp_and_respect_extend() {
        let kind = ShadingKind::Axial { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0, extend: (false, true), ramp: vec![[0.0; 3], [1.0; 3]] };
        assert_eq!(shade_at(&kind, 2.5, 0.0), Some([0.0; 3]));
        assert_eq!(shade_at(&kind, 7.5, 0.0), Some([1.0; 3]));
        assert_eq!(shade_at(&kind, -1.0, 0.0), None, "no extend before the start");
        assert_eq!(shade_at(&kind, 12.0, 0.0), Some([1.0; 3]), "extended after the end");
    }

    #[test]
    fn radial_shadings_grow_outwards() {
        let kind = ShadingKind::Radial { x0: 0.0, y0: 0.0, r0: 0.0, x1: 0.0, y1: 0.0, r1: 10.0, extend: (false, false), ramp: vec![[0.0; 3], [1.0; 3]] };
        assert_eq!(shade_at(&kind, 5.0, 0.0), Some([1.0; 3]), "half way out is the last ramp colour at t = 0.5 rounded up");
        assert_eq!(shade_at(&kind, 0.0, 0.0), Some([0.0; 3]));
        assert_eq!(shade_at(&kind, 20.0, 0.0), None);
    }

    #[test]
    fn triangles_interpolate_their_vertex_colours() {
        let v = |x: f64, y: f64, c: Rgb| Vertex { x, y, color: c };
        let tri = [v(0.0, 0.0, [1.0, 0.0, 0.0]), v(4.0, 0.0, [0.0, 1.0, 0.0]), v(0.0, 4.0, [0.0, 0.0, 1.0])];
        let sh = Shading { kind: ShadingKind::Triangles(vec![tri]), background: None, bbox: None };
        let out = shade_image(&sh, &page::IDENTITY, (0, 0, 4, 4));
        assert_ne!(out.px[0], 0, "a pixel inside the triangle is painted");
        assert_eq!(out.px[3 * 4 + 3], 0, "the far corner is outside");
    }

    #[test]
    fn a_page_renders_its_fills_at_the_requested_size() {
        let doc = Doc::open(tiny_pdf("1 0 0 rg 0 0 612 792 re f")).unwrap();
        let list = page::pages(&doc);
        let img = render_page_fit(&doc, &list[0], 61, 79).unwrap();
        assert_eq!((img.w, img.h), (61, 79));
        assert_eq!(img.px[40 * 61 + 30], rgba(255, 0, 0, 255));
        assert!(render_page(&doc, &list[0], 0, 10).is_none());
        assert!(render_page(&doc, &list[0], MAX_SIDE + 1, 10).is_none());
    }

    #[test]
    fn a_file_page_comes_out_scaled_and_bad_indexes_are_errors() {
        let data = tiny_pdf("0 0 1 rg 0 0 612 792 re f");
        let img = render_file_page(data.clone(), 0, 30, 40).unwrap();
        assert!(img.w <= 30 && img.h <= 40 && img.w > 0);
        assert!(render_file_page(data, 5, 30, 40).is_err());
        assert!(render_file_page(b"%PDF-1.4 nonsense".to_vec(), 0, 30, 40).is_err());
    }
}
