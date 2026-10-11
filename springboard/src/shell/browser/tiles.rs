//! Tile cache and CPU rasteriser for the display list.
//!
//! The page is cut into square tiles (512 px by default) painted lazily and
//! kept under a byte budget with least-recently-used eviction. Scrolling is
//! then a row-wise copy of the visible tiles into the destination surface.
//!
//! The rasteriser draws with `crate::gfx` primitives where they fit (solid
//! and rounded rectangles) and per-pixel signed-distance coverage elsewhere.
//! Clips and opacity groups render into a layer the size of their bounds and
//! are composited when the group ends. Transforms are applied as the box
//! mapping of each item, which is exact for translate and scale; rotation
//! and skew only move and size the item's bounding box.

use super::css::Color;
use super::display::{xf_mul, xf_point, xf_rect, DisplayItem, DisplayList, Fit, Rect, Xf, IDENTITY};
use crate::gfx::{over, rgba, rrect_coverage, scale, Surface};
use std::collections::{HashMap, HashSet};

pub const TILE: usize = 512;

fn pm(c: Color) -> u32 {
    rgba(c.r, c.g, c.b, c.a)
}

fn a255(a: f32) -> u32 {
    (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

// ---------------------------------------------------------------- shapes

fn sdf(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let qx = (px - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (py - h / 2.0).abs() - (h / 2.0 - r);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

fn quad_radius(rad: &[f32; 4], lx: f32, ly: f32, w: f32, h: f32) -> f32 {
    match (ly < h / 2.0, lx < w / 2.0) {
        (true, true) => rad[0],
        (true, false) => rad[1],
        (false, false) => rad[2],
        (false, true) => rad[3],
    }
}

/// Coverage of a box whose corners have individual radii.
fn cov4(lx: f32, ly: f32, w: f32, h: f32, rad: &[f32; 4]) -> f32 {
    let r = quad_radius(rad, lx, ly, w, h).min(w / 2.0).min(h / 2.0).max(0.0);
    rrect_coverage(lx, ly, w, h, r)
}

fn blur_alpha(d: f32, blur: f32) -> f32 {
    if blur <= 0.0 {
        return (0.5 - d).clamp(0.0, 1.0);
    }
    let t = (0.5 - d / (2.0 * blur)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------ primitives

/// Calls `f(centre_x, centre_y)` for every pixel whose box touches `r`,
/// blending the returned premultiplied colour over the surface.
fn per_pixel(s: &mut Surface, r: &Rect, mut f: impl FnMut(f32, f32) -> u32) {
    if r.is_empty() {
        return;
    }
    let x0 = (r.x.floor().max(0.0) as usize).min(s.w);
    let y0 = (r.y.floor().max(0.0) as usize).min(s.h);
    let x1 = (r.right().ceil().max(0.0) as usize).min(s.w);
    let y1 = (r.bottom().ceil().max(0.0) as usize).min(s.h);
    for y in y0..y1 {
        let row = y * s.stride;
        for x in x0..x1 {
            let c = f(x as f32 + 0.5, y as f32 + 0.5);
            if c != 0 {
                let d = &mut s.px[row + x];
                *d = over(*d, c);
            }
        }
    }
}

fn fill_box(s: &mut Surface, r: &Rect, color: u32) {
    if r.is_empty() || color >> 24 == 0 {
        return;
    }
    let whole = |v: f32| v.fract() == 0.0;
    if !(whole(r.x) && whole(r.y) && whole(r.w) && whole(r.h)) {
        return s.fill_rrect(r.x, r.y, r.w, r.h, 0.0, color);
    }
    let x0 = (r.x as i64).clamp(0, s.w as i64) as usize;
    let x1 = (r.right() as i64).clamp(0, s.w as i64) as usize;
    let y0 = (r.y as i64).clamp(0, s.h as i64) as usize;
    let y1 = (r.bottom() as i64).clamp(0, s.h as i64) as usize;
    for y in y0..y1 {
        let row = &mut s.px[y * s.stride + x0..y * s.stride + x1];
        if color >> 24 == 255 {
            row.fill(color);
        } else {
            row.iter_mut().for_each(|d| *d = over(*d, color));
        }
    }
}

fn rounded(s: &mut Surface, r: &Rect, rad: [f32; 4], color: u32) {
    if rad.iter().all(|&v| v <= 0.0) {
        return fill_box(s, r, color);
    }
    if rad.iter().all(|&v| v == rad[0]) {
        return s.fill_rrect(r.x, r.y, r.w, r.h, rad[0], color);
    }
    per_pixel(s, r, |px, py| {
        let c = cov4(px - r.x, py - r.y, r.w, r.h, &rad);
        if c > 0.0 {
            scale(color, a255(c))
        } else {
            0
        }
    });
}

fn border(s: &mut Surface, r: &Rect, wd: [f32; 4], colors: &[Color; 4], rad: [f32; 4]) {
    let [wt, wr, wb, wl] = wd;
    let (iw, ih) = (r.w - wl - wr, r.h - wt - wb);
    let rmax = rad.iter().cloned().fold(0.0, f32::max);
    let inner_rad = [rad[0] - wl.max(wt), rad[1] - wr.max(wt), rad[2] - wr.max(wb), rad[3] - wl.max(wb)];
    let (hx0, hx1) = (wl.max(rmax), r.w - wr.max(rmax));
    let (hy0, hy1) = (wt.max(rmax), r.h - wb.max(rmax));
    let ratio = |d: f32, w: f32| if w > 0.0 { d / w } else { f32::INFINITY };
    per_pixel(s, r, |px, py| {
        let (lx, ly) = (px - r.x, py - r.y);
        if lx >= hx0 && lx < hx1 && ly >= hy0 && ly < hy1 {
            return 0; // solid interior of the box: no border here
        }
        let outer = cov4(lx, ly, r.w, r.h, &rad);
        if outer <= 0.0 {
            return 0;
        }
        let inner = if iw <= 0.0 || ih <= 0.0 { 0.0 } else { cov4(lx - wl, ly - wt, iw, ih, &inner_rad) };
        let a = outer * (1.0 - inner);
        if a <= 0.0 {
            return 0;
        }
        let d = [ratio(ly, wt), ratio(r.w - lx, wr), ratio(r.h - ly, wb), ratio(lx, wl)];
        let side = (0..4).fold(0, |b, i| if d[i] < d[b] { i } else { b });
        scale(pm(colors[side]), a255(a))
    });
}

fn shadow(s: &mut Surface, r: &Rect, radius: f32, blur: f32, spread: f32, color: u32, inset: bool) {
    if inset {
        let hole = r.inflate(-spread);
        let rh = (radius - spread).max(0.0);
        per_pixel(s, r, |px, py| {
            let boxc = rrect_coverage(px - r.x, py - r.y, r.w, r.h, radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0));
            let a = if hole.is_empty() { 1.0 } else { 1.0 - blur_alpha(sdf(px - hole.x, py - hole.y, hole.w, hole.h, rh), blur) };
            let a = a * boxc;
            if a > 0.0 {
                scale(color, a255(a))
            } else {
                0
            }
        });
        return;
    }
    let sh = r.inflate(spread);
    if sh.is_empty() {
        return;
    }
    let rs = (radius + spread).max(0.0);
    per_pixel(s, &sh.inflate(blur.max(0.0)), |px, py| {
        let a = blur_alpha(sdf(px - sh.x, py - sh.y, sh.w, sh.h, rs), blur);
        if a <= 0.0 {
            return 0;
        }
        let boxc = rrect_coverage(px - r.x, py - r.y, r.w, r.h, radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0));
        let a = a * (1.0 - boxc);
        if a > 0.0 {
            scale(color, a255(a))
        } else {
            0
        }
    });
}

fn text(s: &mut Surface, x: f32, y: f32, sx: f32, sy: f32, w: usize, h: usize, cov: &[u8], color: u32) {
    let (dw, dh) = (((w as f32 * sx).round() as i64).max(0), ((h as f32 * sy).round() as i64).max(0));
    if dw == 0 || dh == 0 || cov.len() < w * h {
        return;
    }
    let (ox, oy) = (x.round() as i64, y.round() as i64);
    for j in 0..dh {
        let py = oy + j;
        if py < 0 || py >= s.h as i64 {
            continue;
        }
        let srow = (j * h as i64 / dh) as usize * w;
        for i in 0..dw {
            let px = ox + i;
            if px < 0 || px >= s.w as i64 {
                continue;
            }
            let c = cov[srow + (i * w as i64 / dw) as usize];
            if c != 0 {
                let d = &mut s.px[py as usize * s.stride + px as usize];
                *d = over(*d, scale(color, c as u32));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn image(s: &mut Surface, r: &Rect, img: &crate::gfx::Image, fit: Fit, radius: f32, sx: f32, sy: f32) {
    if img.w == 0 || img.h == 0 || r.is_empty() {
        return;
    }
    let (iw, ih) = (img.w as f32, img.h as f32);
    let content = match fit {
        Fit::Fill => *r,
        Fit::Contain | Fit::Cover => {
            let (kx, ky) = (r.w / iw, r.h / ih);
            let k = if fit == Fit::Contain { kx.min(ky) } else { kx.max(ky) };
            let (w, h) = (iw * k, ih * k);
            Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h)
        }
        Fit::None => Rect::new(r.x, r.y, iw * sx, ih * sy),
    };
    let area = r.intersection(&content);
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    per_pixel(s, &area, |px, py| {
        if !r.contains_point(px, py) || !content.contains_point(px, py) {
            return 0;
        }
        let u = (((px - content.x) / content.w * iw) as usize).min(img.w - 1);
        let v = (((py - content.y) / content.h * ih) as usize).min(img.h - 1);
        let c = img.px[v * img.w + u];
        if radius > 0.0 {
            scale(c, a255(rrect_coverage(px - r.x, py - r.y, r.w, r.h, radius)))
        } else {
            c
        }
    });
}

fn stop_color(stops: &[(f32, Color)], t: f32) -> u32 {
    let first = stops[0];
    if stops.len() == 1 || t <= first.0 {
        return pm(first.1);
    }
    for w in stops.windows(2) {
        let ((o0, c0), (o1, c1)) = (w[0], w[1]);
        if t <= o1 {
            let f = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round().clamp(0.0, 255.0) as u8;
            return rgba(mix(c0.r, c1.r), mix(c0.g, c1.g), mix(c0.b, c1.b), mix(c0.a, c1.a));
        }
    }
    pm(stops[stops.len() - 1].1)
}

fn linear(s: &mut Surface, r: &Rect, angle: f32, stops: &[(f32, Color)]) {
    if stops.is_empty() || r.is_empty() {
        return;
    }
    let (sn, cs) = angle.to_radians().sin_cos();
    let (dx, dy) = (sn, -cs);
    let len = (r.w * sn).abs() + (r.h * cs).abs();
    if len <= 0.0 {
        return;
    }
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    per_pixel(s, r, |px, py| {
        if !r.contains_point(px, py) {
            return 0;
        }
        stop_color(stops, ((px - cx) * dx + (py - cy) * dy) / len + 0.5)
    });
}

fn radial(s: &mut Surface, r: &Rect, c: (f32, f32), rad: (f32, f32), stops: &[(f32, Color)]) {
    if stops.is_empty() || r.is_empty() || rad.0 <= 0.0 || rad.1 <= 0.0 {
        return;
    }
    per_pixel(s, r, |px, py| {
        if !r.contains_point(px, py) {
            return 0;
        }
        let (dx, dy) = ((px - c.0) / rad.0, (py - c.1) / rad.1);
        stop_color(stops, (dx * dx + dy * dy).sqrt())
    });
}

// ------------------------------------------------------------ rasteriser

enum Kind {
    Clip { rect: Rect, radius: f32 },
    Opacity(u32),
}

/// An offscreen buffer for a clip or opacity group, in tile-local pixels.
struct Layer {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    px: Vec<u32>,
    kind: Kind,
}

struct Raster<'a, 'b> {
    base: &'a mut Surface<'b>,
    list: &'a DisplayList,
    ox: f32,
    oy: f32,
    layers: Vec<Layer>,
    xfs: Vec<Xf>,
}

impl Raster<'_, '_> {
    /// The surface being drawn on and the page position of its pixel (0, 0).
    fn target(&mut self) -> (Surface<'_>, f32, f32) {
        let (ox, oy) = (self.ox, self.oy);
        match self.layers.last_mut() {
            Some(l) => (Surface { w: l.w, h: l.h, stride: l.w, px: &mut l.px, y0: 0 }, ox + l.x as f32, oy + l.y as f32),
            None => (Surface { w: self.base.w, h: self.base.h, stride: self.base.stride, px: &mut *self.base.px, y0: 0 }, ox, oy),
        }
    }

    fn push_layer(&mut self, region: (i32, i32, i32, i32), kind: Kind) {
        let (sw, sh) = (self.base.w as i32, self.base.h as i32);
        let (x0, y0) = (region.0.clamp(0, sw), region.1.clamp(0, sh));
        let (x1, y1) = (region.2.clamp(x0, sw), region.3.clamp(y0, sh));
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        self.layers.push(Layer { x: x0, y: y0, w, h, px: vec![0; w * h], kind });
    }

    fn pop_layer(&mut self) {
        let Some(l) = self.layers.pop() else { return };
        let (px0, py0) = self.layers.last().map_or((0, 0), |p| (p.x, p.y));
        let (dst, _, _) = self.target();
        let clip = match l.kind {
            Kind::Clip { rect, radius } if radius > 0.0 => Some((rect, radius.min(rect.w / 2.0).min(rect.h / 2.0))),
            _ => None,
        };
        let alpha = if let Kind::Opacity(a) = l.kind { a } else { 255 };
        for j in 0..l.h {
            let dy = l.y - py0 + j as i32;
            if dy < 0 || dy >= dst.h as i32 {
                continue;
            }
            for i in 0..l.w {
                let dx = l.x - px0 + i as i32;
                if dx < 0 || dx >= dst.w as i32 {
                    continue;
                }
                let mut c = l.px[j * l.w + i];
                if c == 0 {
                    continue;
                }
                if let Some((r, rad)) = clip {
                    let cv = rrect_coverage((l.x + i as i32) as f32 + 0.5 - r.x, (l.y + j as i32) as f32 + 0.5 - r.y, r.w, r.h, rad);
                    c = scale(c, a255(cv));
                }
                if alpha != 255 {
                    c = scale(c, alpha);
                }
                let d = &mut dst.px[dy as usize * dst.stride + dx as usize];
                *d = over(*d, c);
            }
        }
    }

    fn pop_kind(&mut self, clip: bool) {
        if self.layers.last().is_some_and(|l| matches!(l.kind, Kind::Clip { .. }) == clip) {
            self.pop_layer();
        }
    }

    fn local(&self, r: &Rect) -> Rect {
        r.translate(-self.ox, -self.oy)
    }

    fn apply(&mut self, e: &super::display::Entry) {
        let xf = *self.xfs.last().unwrap();
        match &e.item {
            DisplayItem::PushTransform { matrix } => self.xfs.push(xf_mul(&xf, matrix)),
            DisplayItem::PopTransform => {
                if self.xfs.len() > 1 {
                    self.xfs.pop();
                }
            }
            DisplayItem::PushClip { rect, radius } => {
                let dev = self.local(&xf_rect(&xf, rect));
                let b = self.local(&e.bounds);
                let r = (dev.x.round() as i32, dev.y.round() as i32, dev.right().round() as i32, dev.bottom().round() as i32);
                let region = (r.0.max(b.x.floor() as i32), r.1.max(b.y.floor() as i32), r.2.min(b.right().ceil() as i32), r.3.min(b.bottom().ceil() as i32));
                let sr = (xf[0].hypot(xf[1]) + xf[2].hypot(xf[3])) / 2.0;
                self.push_layer(region, Kind::Clip { rect: dev, radius: radius * sr });
            }
            DisplayItem::PopClip => self.pop_kind(true),
            DisplayItem::PushOpacity { alpha } => {
                let b = self.local(&e.bounds);
                self.push_layer((b.x.floor() as i32, b.y.floor() as i32, b.right().ceil() as i32, b.bottom().ceil() as i32), Kind::Opacity(a255(*alpha)));
            }
            DisplayItem::PopOpacity => self.pop_kind(false),
            item => self.draw(item, &xf),
        }
    }

    fn draw(&mut self, item: &DisplayItem, xf: &Xf) {
        let list = self.list;
        let (sx, sy) = (xf[0].hypot(xf[1]), xf[2].hypot(xf[3]));
        let sr = (sx + sy) / 2.0;
        let (mut s, ox, oy) = self.target();
        let dev = |r: &Rect| xf_rect(xf, r).translate(-ox, -oy);
        match item {
            DisplayItem::Rect { rect, color } => fill_box(&mut s, &dev(rect), pm(*color)),
            DisplayItem::RoundedRect { rect, radius, color } => {
                let r = dev(rect);
                let rad = [radius[0] * sr, radius[1] * sr, radius[2] * sr, radius[3] * sr];
                let cap = r.w.min(r.h) / 2.0;
                rounded(&mut s, &r, rad.map(|v| v.min(cap)), pm(*color))
            }
            DisplayItem::Border { rect, widths, colors, radius } => {
                let w = [widths[0] * sy, widths[1] * sx, widths[2] * sy, widths[3] * sx];
                border(&mut s, &dev(rect), w, colors, radius.map(|v| v * sr))
            }
            DisplayItem::Shadow { rect, radius, blur, spread, color, inset } => {
                shadow(&mut s, &dev(rect), radius * sr, blur * sr, spread * sr, pm(*color), *inset)
            }
            DisplayItem::Text { origin, run, color } => {
                let (x, y) = xf_point(xf, origin.0, origin.1);
                text(&mut s, x - ox, y - oy, sx, sy, run.w, run.h, &run.coverage, pm(*color))
            }
            DisplayItem::Image { rect, src, fit, radius } => {
                if let Some(img) = list.image(*src) {
                    image(&mut s, &dev(rect), img, *fit, radius * sr, sx, sy)
                }
            }
            DisplayItem::LinearGradient { rect, angle, stops } => linear(&mut s, &dev(rect), *angle, stops),
            DisplayItem::RadialGradient { rect, center, radii, stops } => {
                let (cx, cy) = xf_point(xf, center.0, center.1);
                radial(&mut s, &dev(rect), (cx - ox, cy - oy), (radii.0 * sx, radii.1 * sy), stops)
            }
            _ => {}
        }
    }
}

/// Paints the part of `list` inside `tile` (page coordinates) onto `surf`,
/// whose pixel (0, 0) is the tile's top-left. Existing contents are kept and
/// drawn over, so fill the page background first.
pub fn paint_tile(list: &DisplayList, tile: Rect, surf: &mut Surface) {
    let mut r = Raster { base: surf, list, ox: tile.x, oy: tile.y, layers: Vec::new(), xfs: vec![IDENTITY] };
    for i in list.items_in(&tile) {
        r.apply(&list.entries()[i]);
    }
    while !r.layers.is_empty() {
        r.pop_layer();
    }
}

// ----------------------------------------------------------------- cache

struct Tile {
    px: Vec<u32>,
    used: u64,
}

type Key = (i32, i32);

pub struct TileCache {
    size: usize,
    budget: usize,
    page: (f32, f32),
    tiles: HashMap<Key, Tile>,
    bytes: usize,
    clock: u64,
    /// Premultiplied colour tiles start with, usually opaque white.
    pub background: u32,
}

impl TileCache {
    pub fn new(budget_bytes: usize) -> TileCache {
        Self::with_tile_size(TILE, budget_bytes)
    }

    pub fn with_tile_size(size: usize, budget_bytes: usize) -> TileCache {
        TileCache { size: size.max(1), budget: budget_bytes, page: (0.0, 0.0), tiles: HashMap::new(), bytes: 0, clock: 0, background: 0xffff_ffff }
    }

    pub fn tile_size(&self) -> usize {
        self.size
    }

    pub fn tile_bytes(&self) -> usize {
        self.size * self.size * 4
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn contains(&self, key: Key) -> bool {
        self.tiles.contains_key(&key)
    }

    /// Page extent; tiles outside it are never requested. 0 means unbounded.
    pub fn set_page_size(&mut self, w: f32, h: f32) {
        self.page = (w, h);
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    pub fn tile_rect(&self, key: Key) -> Rect {
        let s = self.size as f32;
        Rect::new(key.0 as f32 * s, key.1 as f32 * s, s, s)
    }

    fn range(&self, lo: f32, hi: f32, margin: i32, extent: f32) -> (i32, i32) {
        let s = self.size as f32;
        let a = ((lo / s).floor() as i32 - margin).max(0);
        let mut b = (hi / s).ceil() as i32 - 1 + margin;
        if extent > 0.0 {
            b = b.min((extent / s).ceil() as i32 - 1);
        }
        (a, b)
    }

    fn keys(&self, view: &Rect, margin: i32) -> Vec<Key> {
        if view.is_empty() {
            return Vec::new();
        }
        let (x0, x1) = self.range(view.x, view.right(), margin, self.page.0);
        let (y0, y1) = self.range(view.y, view.bottom(), margin, self.page.1);
        (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect()
    }

    /// Tiles touching the viewport, row-major.
    pub fn covering_tiles(&self, viewport: &Rect) -> Vec<Key> {
        self.keys(viewport, 0)
    }

    /// Tiles touching the viewport plus one tile of prefetch on every side.
    pub fn visible_tiles(&self, viewport: &Rect) -> Vec<Key> {
        self.keys(viewport, 1)
    }

    /// Drops every tile touching one of `rects` (page coordinates) and
    /// returns how many went.
    pub fn invalidate(&mut self, rects: &[Rect]) -> usize {
        let size = self.size;
        let hit: Vec<Key> = self.tiles.keys().copied().filter(|&k| rects.iter().any(|r| self.tile_rect(k).intersects(r))).collect();
        for k in &hit {
            self.tiles.remove(k);
        }
        self.bytes = self.tiles.len() * size * size * 4;
        hit.len()
    }

    pub fn invalidate_all(&mut self) {
        self.tiles.clear();
        self.bytes = 0;
    }

    fn evict_for(&mut self, protect: &HashSet<Key>) -> bool {
        let need = self.tile_bytes();
        while self.bytes + need > self.budget {
            let Some(k) = self.tiles.iter().filter(|(k, _)| !protect.contains(k)).min_by_key(|(_, t)| t.used).map(|(k, _)| *k) else {
                return false;
            };
            self.tiles.remove(&k);
            self.bytes -= need;
        }
        true
    }

    fn paint(&mut self, list: &DisplayList, key: Key) {
        let n = self.size;
        let mut px = vec![self.background; n * n];
        paint_tile(list, self.tile_rect(key), &mut Surface { w: n, h: n, stride: n, px: &mut px, y0: 0 });
        self.clock += 1;
        self.bytes += self.tile_bytes();
        self.tiles.insert(key, Tile { px, used: self.clock });
    }

    /// Paints any missing tile of the viewport (always, evicting the least
    /// recently used off-screen tiles to make room) and of its prefetch ring
    /// (only while there is room; prefetch never evicts). Returns how many tiles were painted.
    pub fn update(&mut self, list: &DisplayList, viewport: &Rect) -> usize {
        let on_screen = self.covering_tiles(viewport);
        let protect: HashSet<Key> = on_screen.iter().copied().collect();
        let mut painted = 0;
        for &k in &on_screen {
            if let Some(t) = self.tiles.get_mut(&k) {
                self.clock += 1;
                t.used = self.clock;
            } else {
                self.evict_for(&protect);
                self.paint(list, k);
                painted += 1;
            }
        }
        for k in self.visible_tiles(viewport) {
            if protect.contains(&k) || self.tiles.contains_key(&k) {
                continue;
            }
            if self.bytes + self.tile_bytes() <= self.budget {
                self.paint(list, k);
                painted += 1;
            }
        }
        painted
    }

    /// Copies the cached tiles under the viewport (top-left at `scroll`,
    /// sized like `dst`) into `dst`, one contiguous row at a time. Missing
    /// tiles leave `dst` untouched; their number is returned.
    pub fn composite(&mut self, scroll: (i32, i32), dst: &mut Surface) -> usize {
        let view = Rect::new(scroll.0 as f32, scroll.1 as f32, dst.w as f32, dst.h as f32);
        let n = self.size as i64;
        let (vx0, vy0) = (scroll.0 as i64, scroll.1 as i64);
        let (vx1, vy1) = (vx0 + dst.w as i64, vy0 + dst.h as i64);
        let mut missing = 0;
        for key in self.covering_tiles(&view) {
            self.clock += 1;
            let clock = self.clock;
            let Some(t) = self.tiles.get_mut(&key) else {
                missing += 1;
                continue;
            };
            t.used = clock;
            let (tx, ty) = (key.0 as i64 * n, key.1 as i64 * n);
            let (x0, x1) = (tx.max(vx0), (tx + n).min(vx1));
            let (y0, y1) = (ty.max(vy0), (ty + n).min(vy1));
            if x0 >= x1 {
                continue;
            }
            let len = (x1 - x0) as usize;
            for y in y0..y1 {
                let src = (y - ty) as usize * n as usize + (x0 - tx) as usize;
                let at = (y - vy0) as usize * dst.stride + (x0 - vx0) as usize;
                dst.px[at..at + len].copy_from_slice(&t.px[src..src + len]);
            }
        }
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::super::display::{GlyphRun, ImageId};
    use super::*;
    use crate::gfx::{pack, Image};
    use std::sync::Arc;

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);
    const R: u32 = pack(255, 0, 0, 255);
    const WHITE: u32 = 0xffff_ffff;

    fn rect(x: f32, y: f32, w: f32, h: f32, color: Color) -> DisplayItem {
        DisplayItem::Rect { rect: Rect::new(x, y, w, h), color }
    }

    fn raster(items: Vec<DisplayItem>, images: HashMap<ImageId, Arc<Image>>, tile: Rect, bg: u32) -> Vec<u32> {
        let list = DisplayList::new(items, images);
        let (w, h) = (tile.w as usize, tile.h as usize);
        let mut px = vec![bg; w * h];
        paint_tile(&list, tile, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
        px
    }

    fn paint(items: Vec<DisplayItem>, w: usize, h: usize) -> Vec<u32> {
        raster(items, HashMap::new(), Rect::new(0.0, 0.0, w as f32, h as f32), 0)
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn opaque_rect_golden() {
        let px = paint(vec![rect(2.0, 3.0, 4.0, 2.0, RED)], 10, 8);
        for y in 0..8 {
            for x in 0..10 {
                let inside = (2..6).contains(&x) && (3..5).contains(&y);
                assert_eq!(px[y * 10 + x], if inside { R } else { 0 }, "({x},{y})");
            }
        }
    }

    #[test]
    fn translucent_rect_blends_over_background() {
        let half = Color { r: 255, g: 0, b: 0, a: 128 };
        let px = raster(vec![rect(0.0, 0.0, 4.0, 4.0, half)], HashMap::new(), Rect::new(0.0, 0.0, 4.0, 4.0), WHITE);
        assert_eq!(px[0], over(WHITE, rgba(255, 0, 0, 128)));
    }

    #[test]
    fn rect_is_clipped_to_tile() {
        let px = paint(vec![rect(-5.0, -5.0, 100.0, 100.0, RED)], 4, 4);
        assert!(px.iter().all(|&p| p == R));
    }

    #[test]
    fn tile_origin_shifts_content() {
        let px = raster(vec![rect(60.0, 0.0, 10.0, 4.0, RED)], HashMap::new(), Rect::new(64.0, 0.0, 16.0, 4.0), 0);
        assert_eq!(px[0], R);
        assert_eq!(px[5], R);
        assert_eq!(px[6], 0);
        // an item left of the tile paints nothing
        let none = raster(vec![rect(0.0, 0.0, 10.0, 4.0, RED)], HashMap::new(), Rect::new(64.0, 0.0, 16.0, 4.0), 0);
        assert!(none.iter().all(|&p| p == 0));
    }

    #[test]
    fn later_items_paint_over_earlier() {
        let px = paint(vec![rect(0.0, 0.0, 4.0, 4.0, RED), rect(2.0, 2.0, 4.0, 4.0, BLUE)], 6, 6);
        assert_eq!(px[0], R);
        assert_eq!(px[2 * 6 + 2], pack(0, 0, 255, 255));
    }

    #[test]
    fn rounded_rect_golden() {
        let px = paint(vec![DisplayItem::RoundedRect { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: [5.0; 4], color: RED }], 20, 20);
        assert_eq!(px[0], 0, "corner is cut");
        assert_eq!(px[10 * 20 + 10], R);
        assert_eq!(px[10], R, "top edge middle is solid");
        assert_eq!(px[10 * 20], R, "left edge middle is solid");
        // symmetric corners
        assert_eq!(px[19], 0);
        assert_eq!(px[19 * 20], 0);
        assert_eq!(px[19 * 20 + 19], 0);
    }

    #[test]
    fn rounded_rect_with_mixed_radii() {
        let px = paint(vec![DisplayItem::RoundedRect { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: [10.0, 0.0, 0.0, 0.0], color: RED }], 20, 20);
        assert_eq!(px[0], 0, "round corner");
        assert_eq!(px[19], R, "square corner");
        assert_eq!(px[19 * 20], R);
        assert_eq!(px[19 * 20 + 19], R);
    }

    #[test]
    fn zero_radius_rounded_rect_is_a_rect() {
        let px = paint(vec![DisplayItem::RoundedRect { rect: Rect::new(1.0, 1.0, 3.0, 3.0), radius: [0.0; 4], color: RED }], 5, 5);
        assert_eq!(px[0], 0);
        assert_eq!(px[6], R);
        assert_eq!(px[3 * 5 + 3], R);
        assert_eq!(px[4 * 5 + 4], 0);
    }

    #[test]
    fn rect_clip_limits_drawing() {
        let px = paint(
            vec![
                DisplayItem::PushClip { rect: Rect::new(10.0, 10.0, 10.0, 10.0), radius: 0.0 },
                rect(0.0, 0.0, 40.0, 40.0, RED),
                DisplayItem::PopClip,
            ],
            40,
            40,
        );
        assert_eq!(px[15 * 40 + 15], R);
        assert_eq!(px[10 * 40 + 10], R);
        assert_eq!(px[19 * 40 + 19], R);
        assert_eq!(px[5 * 40 + 5], 0);
        assert_eq!(px[20 * 40 + 20], 0);
        assert_eq!(px[15 * 40 + 9], 0);
        assert_eq!(px[15 * 40 + 20], 0);
    }

    #[test]
    fn rounded_clip_cuts_corners() {
        let px = paint(
            vec![
                DisplayItem::PushClip { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: 10.0 },
                rect(0.0, 0.0, 40.0, 40.0, RED),
                DisplayItem::PopClip,
            ],
            40,
            40,
        );
        assert_eq!(px[0], 0);
        assert_eq!(px[10 * 40 + 10], R);
        assert_eq!(px[30 * 40 + 30], 0);
        // partially covered edge pixel on the arc
        let edge = alpha(px[2 * 40 + 2]);
        assert!(edge < 255);
    }

    #[test]
    fn nested_clips_intersect() {
        let px = paint(
            vec![
                DisplayItem::PushClip { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: 0.0 },
                DisplayItem::PushClip { rect: Rect::new(10.0, 10.0, 20.0, 20.0), radius: 0.0 },
                rect(0.0, 0.0, 40.0, 40.0, RED),
                DisplayItem::PopClip,
                DisplayItem::PopClip,
                rect(35.0, 35.0, 2.0, 2.0, BLUE),
            ],
            40,
            40,
        );
        assert_eq!(px[15 * 40 + 15], R);
        assert_eq!(px[25 * 40 + 25], 0);
        assert_eq!(px[5 * 40 + 5], 0);
        assert_eq!(px[35 * 40 + 35], pack(0, 0, 255, 255), "drawing after the clip is unclipped");
    }

    #[test]
    fn opacity_group_golden() {
        let px = raster(
            vec![DisplayItem::PushOpacity { alpha: 0.5 }, rect(0.0, 0.0, 4.0, 4.0, RED), DisplayItem::PopOpacity],
            HashMap::new(),
            Rect::new(0.0, 0.0, 8.0, 8.0),
            WHITE,
        );
        assert_eq!(px[0], over(WHITE, scale(R, 128)));
        assert_eq!(px[7 * 8 + 7], WHITE);
    }

    #[test]
    fn opacity_applies_to_group_not_each_item() {
        // two overlapping opaque rects at 50%: overlap must not double blend
        let px = raster(
            vec![DisplayItem::PushOpacity { alpha: 0.5 }, rect(0.0, 0.0, 6.0, 6.0, RED), rect(2.0, 2.0, 6.0, 6.0, RED), DisplayItem::PopOpacity],
            HashMap::new(),
            Rect::new(0.0, 0.0, 8.0, 8.0),
            WHITE,
        );
        assert_eq!(px[0], px[3 * 8 + 3]);
        assert_eq!(px[0], over(WHITE, scale(R, 128)));
    }

    #[test]
    fn nested_opacity_multiplies() {
        let px = raster(
            vec![
                DisplayItem::PushOpacity { alpha: 0.5 },
                DisplayItem::PushOpacity { alpha: 0.5 },
                rect(0.0, 0.0, 4.0, 4.0, RED),
                DisplayItem::PopOpacity,
                DisplayItem::PopOpacity,
            ],
            HashMap::new(),
            Rect::new(0.0, 0.0, 4.0, 4.0),
            WHITE,
        );
        let want = over(WHITE, scale(scale(R, 128), 128));
        assert_eq!(px[0], want);
    }

    #[test]
    fn opacity_zero_draws_nothing_and_one_is_plain() {
        let none = raster(vec![DisplayItem::PushOpacity { alpha: 0.0 }, rect(0.0, 0.0, 4.0, 4.0, RED), DisplayItem::PopOpacity], HashMap::new(), Rect::new(0.0, 0.0, 4.0, 4.0), WHITE);
        assert!(none.iter().all(|&p| p == WHITE));
        let full = paint(vec![DisplayItem::PushOpacity { alpha: 1.0 }, rect(0.0, 0.0, 4.0, 4.0, RED), DisplayItem::PopOpacity], 4, 4);
        assert!(full.iter().all(|&p| p == R));
    }

    #[test]
    fn unbalanced_groups_are_flushed() {
        let px = paint(vec![DisplayItem::PopClip, DisplayItem::PushOpacity { alpha: 1.0 }, rect(0.0, 0.0, 4.0, 4.0, RED)], 4, 4);
        assert_eq!(px[0], R);
    }

    #[test]
    fn transform_translates_and_scales() {
        let px = paint(
            vec![DisplayItem::PushTransform { matrix: [1.0, 0.0, 0.0, 1.0, 10.0, 10.0] }, rect(0.0, 0.0, 5.0, 5.0, RED), DisplayItem::PopTransform],
            20,
            20,
        );
        assert_eq!(px[12 * 20 + 12], R);
        assert_eq!(px[2 * 20 + 2], 0);
        assert_eq!(px[16 * 20 + 16], 0);
        let big = paint(
            vec![DisplayItem::PushTransform { matrix: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] }, rect(1.0, 1.0, 2.0, 2.0, RED), DisplayItem::PopTransform],
            8,
            8,
        );
        assert_eq!(big[2 * 8 + 2], R);
        assert_eq!(big[5 * 8 + 5], R);
        assert_eq!(big[6 * 8 + 6], 0);
        assert_eq!(big[8 + 1], 0);
    }

    #[test]
    fn border_golden() {
        let px = paint(
            vec![DisplayItem::Border { rect: Rect::new(0.0, 0.0, 10.0, 10.0), widths: [2.0; 4], colors: [RED, BLUE, RED, BLUE], radius: [0.0; 4] }],
            10,
            10,
        );
        assert_eq!(px[5 * 10], pack(0, 0, 255, 255), "left");
        assert_eq!(px[5 * 10 + 1], pack(0, 0, 255, 255));
        assert_eq!(px[5 * 10 + 2], 0, "interior");
        assert_eq!(px[5], R, "top");
        assert_eq!(px[9 * 10 + 5], R, "bottom");
        assert_eq!(px[5 * 10 + 9], pack(0, 0, 255, 255), "right");
        assert_eq!(px[5 * 10 + 5], 0);
    }

    #[test]
    fn border_with_zero_side_and_radius() {
        let px = paint(
            vec![DisplayItem::Border { rect: Rect::new(0.0, 0.0, 20.0, 20.0), widths: [0.0, 0.0, 2.0, 0.0], colors: [RED; 4], radius: [0.0; 4] }],
            20,
            20,
        );
        assert_eq!(px[10 * 20 + 10], 0);
        assert_eq!(px[19 * 20 + 10], R);
        assert_eq!(px[10], 0, "no top border");
        let round = paint(
            vec![DisplayItem::Border { rect: Rect::new(0.0, 0.0, 20.0, 20.0), widths: [2.0; 4], colors: [RED; 4], radius: [10.0; 4] }],
            20,
            20,
        );
        assert_eq!(round[0], 0, "rounded corner is empty");
        assert!(alpha(round[10]) > 240, "top edge is solid");
        assert_eq!(round[10 * 20 + 10], 0);
    }

    #[test]
    fn outer_shadow_surrounds_but_spares_the_box() {
        let px = paint(
            vec![DisplayItem::Shadow { rect: Rect::new(20.0, 20.0, 20.0, 20.0), radius: 0.0, blur: 4.0, spread: 0.0, color: Color { r: 0, g: 0, b: 0, a: 255 }, inset: false }],
            60,
            60,
        );
        assert_eq!(px[30 * 60 + 30], 0, "inside the box");
        let near = alpha(px[30 * 60 + 19]);
        let far = alpha(px[30 * 60 + 15]);
        assert!(near > 0 && near > far, "{near} {far}");
        assert_eq!(px[0], 0);
        assert_eq!(px[30 * 60 + 10], 0, "beyond blur reach");
    }

    #[test]
    fn spread_widens_outer_shadow() {
        let mk = |spread| paint(vec![DisplayItem::Shadow { rect: Rect::new(20.0, 20.0, 20.0, 20.0), radius: 0.0, blur: 0.0, spread, color: Color::BLACK, inset: false }], 60, 60);
        assert_eq!(mk(0.0)[30 * 60 + 18], 0);
        assert_eq!(alpha(mk(5.0)[30 * 60 + 18]), 255);
    }

    #[test]
    fn inset_shadow_hugs_the_edge() {
        let px = paint(
            vec![DisplayItem::Shadow { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: 0.0, blur: 2.0, spread: 0.0, color: Color::BLACK, inset: true }],
            30,
            30,
        );
        assert!(alpha(px[10 * 30]) > 0);
        assert_eq!(px[10 * 30 + 10], 0);
        assert_eq!(px[10 * 30 + 25], 0, "outside the box");
    }

    #[test]
    fn text_golden() {
        let run = GlyphRun::new(2, 2, vec![255, 128, 0, 255]);
        let px = paint(vec![DisplayItem::Text { origin: (1.0, 1.0), run, color: RED }], 5, 5);
        assert_eq!(px[5 + 1], R);
        assert_eq!(px[5 + 2], scale(R, 128));
        assert_eq!(px[2 * 5 + 1], 0);
        assert_eq!(px[2 * 5 + 2], R);
        assert_eq!(px[0], 0);
    }

    #[test]
    fn text_scales_with_transform() {
        let run = GlyphRun::new(1, 1, vec![255]);
        let px = paint(
            vec![DisplayItem::PushTransform { matrix: [3.0, 0.0, 0.0, 3.0, 0.0, 0.0] }, DisplayItem::Text { origin: (0.0, 0.0), run, color: RED }, DisplayItem::PopTransform],
            4,
            4,
        );
        assert_eq!(px[2 * 4 + 2], R);
        assert_eq!(px[3 * 4 + 3], 0);
    }

    fn quad_image() -> HashMap<ImageId, Arc<Image>> {
        let (a, b, c, d) = (pack(255, 0, 0, 255), pack(0, 255, 0, 255), pack(0, 0, 255, 255), pack(255, 255, 255, 255));
        HashMap::from([(ImageId(1), Arc::new(Image { w: 2, h: 2, px: vec![a, b, c, d] }))])
    }

    #[test]
    fn image_fill_stretches() {
        let item = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 4.0, 4.0), src: ImageId(1), fit: Fit::Fill, radius: 0.0 };
        let px = raster(vec![item], quad_image(), Rect::new(0.0, 0.0, 6.0, 6.0), 0);
        assert_eq!(px[0], pack(255, 0, 0, 255));
        assert_eq!(px[3], pack(0, 255, 0, 255));
        assert_eq!(px[3 * 6], pack(0, 0, 255, 255));
        assert_eq!(px[3 * 6 + 3], pack(255, 255, 255, 255));
        assert_eq!(px[4], 0, "outside the rect");
    }

    #[test]
    fn image_contain_letterboxes_and_cover_crops() {
        let wide = HashMap::from([(ImageId(1), Arc::new(Image { w: 2, h: 1, px: vec![pack(255, 0, 0, 255), pack(0, 255, 0, 255)] }))]);
        let contain = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 4.0, 4.0), src: ImageId(1), fit: Fit::Contain, radius: 0.0 };
        let px = raster(vec![contain], wide.clone(), Rect::new(0.0, 0.0, 4.0, 4.0), 0);
        assert_eq!(px[0], 0, "letterbox");
        assert_eq!(px[4], pack(255, 0, 0, 255));
        assert_eq!(px[4 + 3], pack(0, 255, 0, 255));
        assert_eq!(px[3 * 4], 0);
        let cover = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 4.0, 4.0), src: ImageId(1), fit: Fit::Cover, radius: 0.0 };
        let px = raster(vec![cover], wide, Rect::new(0.0, 0.0, 4.0, 4.0), 0);
        assert!(px.iter().all(|&p| p != 0), "cover fills the whole rect");
    }

    #[test]
    fn image_natural_size_and_rounding() {
        let item = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 8.0, 8.0), src: ImageId(1), fit: Fit::None, radius: 0.0 };
        let px = raster(vec![item], quad_image(), Rect::new(0.0, 0.0, 8.0, 8.0), 0);
        assert_ne!(px[0], 0);
        assert_ne!(px[8 + 1], 0);
        assert_eq!(px[2], 0, "past natural width");
        let big = HashMap::from([(ImageId(1), Arc::new(Image { w: 20, h: 20, px: vec![R; 400] }))]);
        let round = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 20.0, 20.0), src: ImageId(1), fit: Fit::Fill, radius: 10.0 };
        let px = raster(vec![round], big, Rect::new(0.0, 0.0, 20.0, 20.0), 0);
        assert_eq!(px[0], 0);
        assert_eq!(px[10 * 20 + 10], R);
    }

    #[test]
    fn missing_image_is_skipped() {
        let item = DisplayItem::Image { rect: Rect::new(0.0, 0.0, 4.0, 4.0), src: ImageId(9), fit: Fit::Fill, radius: 0.0 };
        assert!(paint(vec![item], 4, 4).iter().all(|&p| p == 0));
    }

    fn red_of(p: u32) -> u32 {
        p & 0xff
    }

    #[test]
    fn linear_gradient_runs_left_to_right() {
        let item = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 100.0, 10.0), angle: 90.0, stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)] };
        let px = paint(vec![item], 100, 10);
        assert!(red_of(px[0]) < 5);
        assert!(red_of(px[99]) > 250);
        assert!((red_of(px[50]) as i32 - 128).abs() <= 3);
        assert!((1..100).all(|x| red_of(px[x]) >= red_of(px[x - 1])));
        assert_eq!(px[5 * 100 + 50], px[50], "constant down a column");
    }

    #[test]
    fn linear_gradient_angle_zero_runs_bottom_to_top() {
        let item = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 10.0, 100.0), angle: 0.0, stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)] };
        let px = paint(vec![item], 10, 100);
        assert!(red_of(px[99 * 10]) < 5, "bottom is the start");
        assert!(red_of(px[0]) > 250);
    }

    #[test]
    fn gradient_stops_clamp_and_hard_stop() {
        let item = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 10.0, 1.0), angle: 90.0, stops: vec![(0.4, RED), (0.4, BLUE), (1.0, BLUE)] };
        let px = paint(vec![item], 10, 1);
        assert_eq!(px[0], R);
        assert_eq!(px[9], pack(0, 0, 255, 255));
        let solo = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 4.0, 1.0), angle: 90.0, stops: vec![(0.5, RED)] };
        assert!(paint(vec![solo], 4, 1).iter().all(|&p| p == R));
        let empty = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 4.0, 1.0), angle: 90.0, stops: vec![] };
        assert!(paint(vec![empty], 4, 1).iter().all(|&p| p == 0));
    }

    #[test]
    fn radial_gradient_centre_and_edge() {
        let item = DisplayItem::RadialGradient { rect: Rect::new(0.0, 0.0, 40.0, 40.0), center: (20.0, 20.0), radii: (20.0, 20.0), stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)] };
        let px = paint(vec![item], 40, 40);
        assert!(red_of(px[20 * 40 + 20]) < 15);
        assert!(red_of(px[20 * 40 + 1]) > 230);
        assert_eq!(px[20 * 40 + 5], px[5 * 40 + 20], "symmetric");
    }

    #[test]
    fn translucent_gradient_blends() {
        let item = DisplayItem::LinearGradient { rect: Rect::new(0.0, 0.0, 4.0, 1.0), angle: 90.0, stops: vec![(0.0, Color { r: 255, g: 0, b: 0, a: 0 }), (1.0, Color { r: 255, g: 0, b: 0, a: 0 })] };
        let px = raster(vec![item], HashMap::new(), Rect::new(0.0, 0.0, 4.0, 1.0), WHITE);
        assert!(px.iter().all(|&p| p == WHITE));
    }

    // ------------------------------------------------------------ cache

    fn page_list() -> DisplayList {
        DisplayList::new(vec![rect(0.0, 0.0, 100.0, 100.0, RED), rect(40.0, 40.0, 4.0, 4.0, BLUE)], HashMap::new())
    }

    #[test]
    fn visible_tiles_adds_one_tile_of_prefetch() {
        let c = TileCache::new(usize::MAX);
        let v = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(c.covering_tiles(&v), vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(c.visible_tiles(&v).len(), 9);
        let mid = Rect::new(600.0, 600.0, 300.0, 300.0); // tiles 1..=1 -> 0..=2 with prefetch
        assert_eq!(c.covering_tiles(&mid), vec![(1, 1)]);
        assert_eq!(c.visible_tiles(&mid).len(), 9);
        assert!(c.visible_tiles(&mid).contains(&(0, 0)));
    }

    #[test]
    fn tile_math_at_boundaries() {
        let c = TileCache::new(usize::MAX);
        assert_eq!(c.covering_tiles(&Rect::new(0.0, 0.0, 512.0, 512.0)), vec![(0, 0)], "exact tile");
        assert_eq!(c.covering_tiles(&Rect::new(511.0, 0.0, 2.0, 1.0)), vec![(0, 0), (1, 0)]);
        assert_eq!(c.covering_tiles(&Rect::new(512.0, 512.0, 1.0, 1.0)), vec![(1, 1)]);
        assert!(c.covering_tiles(&Rect::ZERO).is_empty());
        assert_eq!(c.tile_rect((2, 3)), Rect::new(1024.0, 1536.0, 512.0, 512.0));
        assert_eq!(c.tile_bytes(), 512 * 512 * 4);
        // negative scroll (overscroll) clamps to the first tile
        assert_eq!(c.covering_tiles(&Rect::new(-100.0, -100.0, 300.0, 300.0)), vec![(0, 0)]);
    }

    #[test]
    fn page_size_bounds_tiles() {
        let mut c = TileCache::new(usize::MAX);
        c.set_page_size(600.0, 300.0);
        let v = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        assert_eq!(c.covering_tiles(&v), vec![(0, 0), (1, 0)]);
        assert_eq!(c.visible_tiles(&v), vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn update_paints_lazily_and_once() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(64, usize::MAX);
        c.set_page_size(128.0, 128.0);
        assert_eq!(c.update(&l, &Rect::new(0.0, 0.0, 64.0, 64.0)), 4, "visible plus ring, bounded by page");
        assert_eq!(c.len(), 4);
        assert_eq!(c.update(&l, &Rect::new(0.0, 0.0, 64.0, 64.0)), 0);
        assert_eq!(c.bytes(), 4 * 64 * 64 * 4);
    }

    #[test]
    fn composite_reproduces_the_page() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(64, usize::MAX);
        let view = Rect::new(30.0, 30.0, 80.0, 80.0);
        c.update(&l, &view);
        let mut px = vec![0u32; 80 * 80];
        let missing = c.composite((30, 30), &mut Surface { w: 80, h: 80, stride: 80, px: &mut px, y0: 0 });
        assert_eq!(missing, 0);
        // page (35,35) is red; page (41,41) is the blue speck; page (105,105) is background
        assert_eq!(px[5 * 80 + 5], R);
        assert_eq!(px[11 * 80 + 11], pack(0, 0, 255, 255));
        assert_eq!(px[75 * 80 + 75], WHITE);
        // matches painting the viewport directly
        let direct = raster(vec![rect(0.0, 0.0, 100.0, 100.0, RED), rect(40.0, 40.0, 4.0, 4.0, BLUE)], HashMap::new(), view, WHITE);
        assert_eq!(px, direct);
    }

    #[test]
    fn composite_honours_dst_stride() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(64, usize::MAX);
        c.update(&l, &Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut px = vec![7u32; 16 * 10];
        c.composite((0, 0), &mut Surface { w: 10, h: 10, stride: 16, px: &mut px, y0: 0 });
        assert_eq!(px[0], R);
        assert_eq!(px[9], R);
        assert_eq!(px[10], 7, "padding untouched");
    }

    #[test]
    fn composite_reports_missing_tiles() {
        let mut c = TileCache::with_tile_size(64, usize::MAX);
        let mut px = vec![5u32; 100 * 100];
        let missing = c.composite((0, 0), &mut Surface { w: 100, h: 100, stride: 100, px: &mut px, y0: 0 });
        assert_eq!(missing, 4);
        assert!(px.iter().all(|&p| p == 5));
    }

    #[test]
    fn invalidate_drops_only_touched_tiles() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(64, usize::MAX);
        c.update(&l, &Rect::new(0.0, 0.0, 128.0, 128.0));
        let before = c.len();
        assert_eq!(c.invalidate(&[Rect::new(70.0, 10.0, 5.0, 5.0)]), 1);
        assert!(!c.contains((1, 0)));
        assert!(c.contains((0, 0)));
        assert_eq!(c.len(), before - 1);
        assert_eq!(c.bytes(), c.len() * c.tile_bytes());
        // touching an edge does not count
        assert_eq!(c.invalidate(&[Rect::new(64.0, 64.0, 0.0, 0.0)]), 0);
        c.invalidate_all();
        assert!(c.is_empty());
        assert_eq!(c.bytes(), 0);
    }

    #[test]
    fn invalidated_tile_repaints_from_new_list() {
        let mut c = TileCache::with_tile_size(16, usize::MAX);
        let a = DisplayList::new(vec![rect(0.0, 0.0, 16.0, 16.0, RED)], HashMap::new());
        let b = DisplayList::new(vec![rect(0.0, 0.0, 16.0, 16.0, BLUE)], HashMap::new());
        let view = Rect::new(0.0, 0.0, 16.0, 16.0);
        c.update(&a, &view);
        c.invalidate(&[view]);
        c.update(&b, &view);
        let mut px = vec![0u32; 256];
        c.composite((0, 0), &mut Surface { w: 16, h: 16, stride: 16, px: &mut px, y0: 0 });
        assert_eq!(px[0], pack(0, 0, 255, 255));
    }

    #[test]
    fn lru_evicts_least_recently_used_off_screen_tile() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(16, 16 * 16 * 4 * 2);
        c.set_page_size(48.0, 16.0);
        c.update(&l, &Rect::new(0.0, 0.0, 16.0, 16.0)); // (0,0) and prefetched (1,0)
        c.update(&l, &Rect::new(16.0, 0.0, 16.0, 16.0)); // touches (1,0); no room to prefetch (2,0)
        assert!(!c.contains((2, 0)));
        c.update(&l, &Rect::new(32.0, 0.0, 16.0, 16.0)); // (2,0) evicts the oldest, (0,0)
        assert!(c.contains((2, 0)) && c.contains((1, 0)));
        assert!(!c.contains((0, 0)));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn visible_tiles_survive_eviction_even_over_budget() {
        let l = page_list();
        let tb = 16 * 16 * 4;
        let mut c = TileCache::with_tile_size(16, tb); // room for one tile
        c.update(&l, &Rect::new(0.0, 0.0, 32.0, 32.0)); // four tiles on screen
        for k in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert!(c.contains(k), "{k:?}");
        }
        let mut px = vec![0u32; 32 * 32];
        assert_eq!(c.composite((0, 0), &mut Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 }), 0);
    }

    #[test]
    fn prefetch_never_evicts_visible_tiles_or_busts_budget() {
        let l = page_list();
        let tb = 16 * 16 * 4;
        let mut c = TileCache::with_tile_size(16, tb * 2);
        c.update(&l, &Rect::new(16.0, 16.0, 16.0, 16.0));
        assert!(c.contains((1, 1)));
        assert!(c.bytes() <= tb * 2);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn recently_composited_tile_outlives_older_ones() {
        let l = page_list();
        let mut c = TileCache::with_tile_size(16, 16 * 16 * 4 * 2);
        c.set_page_size(48.0, 16.0);
        c.update(&l, &Rect::new(0.0, 0.0, 16.0, 16.0)); // (0,0) then (1,0)
        let mut px = vec![0u32; 16 * 16];
        c.composite((0, 0), &mut Surface { w: 16, h: 16, stride: 16, px: &mut px, y0: 0 }); // touches (0,0)
        c.update(&l, &Rect::new(32.0, 0.0, 16.0, 16.0)); // evicts (1,0), now the oldest
        assert!(c.contains((0, 0)) && c.contains((2, 0)));
        assert!(!c.contains((1, 0)));
    }

    #[test]
    fn background_fills_unpainted_pixels() {
        let l = DisplayList::default();
        let mut c = TileCache::with_tile_size(8, usize::MAX);
        c.background = pack(1, 2, 3, 255);
        let view = Rect::new(0.0, 0.0, 8.0, 8.0);
        c.update(&l, &view);
        let mut px = vec![0u32; 64];
        c.composite((0, 0), &mut Surface { w: 8, h: 8, stride: 8, px: &mut px, y0: 0 });
        assert!(px.iter().all(|&p| p == pack(1, 2, 3, 255)));
    }
}
