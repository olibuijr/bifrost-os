//! Rendering a `Frame` into a 32-bit framebuffer: background, the BifrOSt mark (a vector shape scaled from the
//! panel's short side), the progress bar (geometry from the panel size) and each `Screen`. No allocation in `render`,
//! deterministic output (golden snapshot tests). Owner: task 02.
//!
//! The mark is `packaging/brand/bifrost-mark.svg` as data: its stroked polylines (butt ends, round joins) and its
//! even-odd filled polygons, tested per sample with 2x2 supersampling. Squared distances are compared, so the
//! rasteriser needs no square root and no floating point beyond `f32`.
//!
//! Screens that need text (unlock name, option labels, button labels) draw their shapes only; the labels come with
//! the boot glyph set (todo_left in docs/boot/02-bootframe.md).

use crate::{Appearance, Frame, Screen};

/// The byte order of a 32-bit pixel in the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOrder {
    /// B, G, R, X in memory (UEFI GOP BlueGreenRedReserved, DRM XRGB8888 little endian). Packed as `0xffRRGGBB`.
    Bgrx,
    /// R, G, B, X in memory (UEFI GOP RedGreenBlueReserved, DRM XBGR8888). Packed as `0xffBBGGRR`.
    Rgbx,
}

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// The colours of one appearance. Values mirror the shell's tokens in `springboard/render/src/ui.rs`
/// (TRACK, TRACK_C, SYSTEM_BLUE, SYSTEM_RED, FILL, FILL_GRAY) and the boot bar's fill of the old splash code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub bg: Rgb,
    pub track: Rgb,
    pub fill: Rgb,
    pub plate: Rgb,
    pub ink: Rgb,
    pub accent: Rgb,
    pub danger: Rgb,
}

/// The mark's blue (also the tint, `SYSTEM_BLUE`).
const BLUE: Rgb = Rgb(10, 132, 255);
const PURPLE: Rgb = Rgb(113, 96, 207);
const ORANGE: Rgb = Rgb(223, 113, 60);
const GOLD: Rgb = Rgb(232, 184, 90);

pub fn palette(appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Dark => Palette {
            bg: Rgb(0, 0, 0),
            track: Rgb(58, 58, 60),
            fill: Rgb(229, 229, 234),
            plate: Rgb(44, 44, 46),
            ink: Rgb(242, 242, 247),
            accent: BLUE,
            danger: Rgb(255, 69, 58),
        },
        Appearance::Light => Palette {
            bg: Rgb(255, 255, 255),
            track: Rgb(209, 209, 214),
            fill: Rgb(28, 28, 30),
            plate: Rgb(229, 229, 234),
            ink: Rgb(28, 28, 30),
            accent: BLUE,
            danger: Rgb(255, 59, 48),
        },
    }
}

fn pack(c: Rgb, order: PixelOrder) -> u32 {
    let Rgb(r, g, b) = c;
    match order {
        PixelOrder::Bgrx => 0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32,
        PixelOrder::Rgbx => 0xff00_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32,
    }
}

/// Which colour a mark element takes (resolved against the palette: only the light fills change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Blue,
    Purple,
    Orange,
    Gold,
    Ink,
}

impl Tone {
    fn rgb(self, p: &Palette) -> Rgb {
        match self {
            Tone::Blue => BLUE,
            Tone::Purple => PURPLE,
            Tone::Orange => ORANGE,
            Tone::Gold => GOLD,
            Tone::Ink => p.ink,
        }
    }
}

/// One element of the mark, in the SVG's 1024-unit square, painted in order.
enum Shape {
    /// A stroked polyline; `half` is half the stroke width.
    Stroke { pts: &'static [(f32, f32)], closed: bool, half: f32, tone: Tone },
    /// Filled subpaths with the even-odd rule across all of them.
    Fill { subs: &'static [&'static [(f32, f32)]], tone: Tone },
}

const OUTLINE: &[(f32, f32)] = &[(512.0, 48.0), (944.0, 280.0), (944.0, 744.0), (512.0, 976.0), (80.0, 744.0), (80.0, 280.0)];
const ARCH_BLUE: &[(f32, f32)] = &[
    (162.0, 756.0), (162.0, 620.0), (188.642, 486.061), (264.513, 372.513), (378.061, 296.642), (512.0, 270.0),
    (645.939, 296.642), (759.487, 372.513), (835.358, 486.061), (862.0, 620.0), (862.0, 756.0),
];
const ARCH_PURPLE: &[(f32, f32)] = &[
    (240.0, 756.0), (240.0, 620.0), (260.705, 515.910), (319.667, 427.667), (407.910, 368.705), (512.0, 348.0),
    (616.090, 368.705), (704.333, 427.667), (763.295, 515.910), (784.0, 620.0), (784.0, 756.0),
];
const ARCH_ORANGE: &[(f32, f32)] = &[
    (308.0, 756.0), (308.0, 620.0), (323.529, 541.933), (367.750, 475.750), (433.933, 431.529), (512.0, 416.0),
    (590.067, 431.529), (656.250, 475.750), (700.471, 541.933), (716.0, 620.0), (716.0, 756.0),
];
const ARCH_GOLD: &[(f32, f32)] = &[
    (376.0, 756.0), (376.0, 620.0), (386.352, 567.955), (415.833, 523.833), (459.955, 494.352), (512.0, 484.0),
    (564.045, 494.352), (608.167, 523.833), (637.648, 567.955), (648.0, 620.0), (648.0, 756.0),
];
const FEET_LEFT: &[(f32, f32)] = &[(140.0, 746.0), (394.0, 746.0), (394.0, 776.0), (372.0, 798.0), (190.0, 798.0), (160.0, 770.0)];
const FEET_RIGHT: &[(f32, f32)] = &[(884.0, 746.0), (630.0, 746.0), (630.0, 776.0), (652.0, 798.0), (834.0, 798.0), (864.0, 770.0)];
const STAR: &[(f32, f32)] = &[
    (494.0, 76.0), (530.0, 76.0), (530.0, 117.0), (559.0, 88.0), (585.0, 114.0), (556.0, 143.0), (596.0, 143.0),
    (596.0, 179.0), (556.0, 179.0), (585.0, 208.0), (559.0, 234.0), (530.0, 205.0), (530.0, 244.0), (494.0, 244.0),
    (494.0, 205.0), (465.0, 234.0), (439.0, 208.0), (468.0, 179.0), (428.0, 179.0), (428.0, 143.0), (468.0, 143.0),
    (439.0, 114.0), (465.0, 88.0), (494.0, 117.0),
];
const RUNE_OUTER: &[(f32, f32)] = &[
    (412.0, 576.0), (436.0, 596.0), (460.0, 576.0), (460.0, 610.0), (612.0, 674.0), (612.0, 706.0), (494.0, 754.0),
    (612.0, 808.0), (612.0, 840.0), (460.0, 882.0), (436.0, 910.0), (412.0, 882.0),
];
const RUNE_HOLE_A: &[(f32, f32)] = &[(460.0, 652.0), (460.0, 718.0), (540.0, 684.0)];
const RUNE_HOLE_B: &[(f32, f32)] = &[(460.0, 790.0), (460.0, 856.0), (540.0, 823.0)];
const RUNE: &[&[(f32, f32)]] = &[RUNE_OUTER, RUNE_HOLE_A, RUNE_HOLE_B];
const FEET: &[&[(f32, f32)]] = &[FEET_LEFT, FEET_RIGHT];
const STAR_SUBS: &[&[(f32, f32)]] = &[STAR];

/// The mark, painted in the SVG's order (`packaging/brand/bifrost-mark.svg`).
const MARK: &[Shape] = &[
    Shape::Stroke { pts: OUTLINE, closed: true, half: 10.0, tone: Tone::Blue },
    Shape::Stroke { pts: ARCH_BLUE, closed: false, half: 26.0, tone: Tone::Blue },
    Shape::Stroke { pts: ARCH_PURPLE, closed: false, half: 18.0, tone: Tone::Purple },
    Shape::Stroke { pts: ARCH_ORANGE, closed: false, half: 18.0, tone: Tone::Orange },
    Shape::Stroke { pts: ARCH_GOLD, closed: false, half: 18.0, tone: Tone::Gold },
    Shape::Fill { subs: FEET, tone: Tone::Purple },
    Shape::Fill { subs: STAR_SUBS, tone: Tone::Ink },
    Shape::Fill { subs: RUNE, tone: Tone::Ink },
];

/// Whether `p` is within `half` of a stroke: butt ends on the first and last segment, round joins at the vertices.
fn stroke_hit(pts: &[(f32, f32)], closed: bool, half: f32, p: (f32, f32)) -> bool {
    let n = pts.len();
    let h2 = half * half;
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let d = (b.0 - a.0, b.1 - a.1);
        let len2 = d.0 * d.0 + d.1 * d.1;
        if len2 == 0.0 {
            continue;
        }
        let q = (p.0 - a.0, p.1 - a.1);
        let t = q.0 * d.0 + q.1 * d.1;
        if t >= 0.0 && t <= len2 {
            let cross = d.0 * q.1 - d.1 * q.0;
            if cross * cross <= h2 * len2 {
                return true;
            }
        }
    }
    for (i, v) in pts.iter().enumerate() {
        if !closed && (i == 0 || i == n - 1) {
            continue;
        }
        let q = (p.0 - v.0, p.1 - v.1);
        if q.0 * q.0 + q.1 * q.1 <= h2 {
            return true;
        }
    }
    false
}

/// Even-odd fill test over every subpath (crossing count of a ray to the right).
fn fill_hit(subs: &[&[(f32, f32)]], p: (f32, f32)) -> bool {
    let mut inside = false;
    for poly in subs {
        let n = poly.len();
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            if (a.1 > p.1) != (b.1 > p.1) {
                let x = a.0 + (p.1 - a.1) * (b.0 - a.0) / (b.1 - a.1);
                if p.0 < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// The mark's tone at a point of the 1024-unit square, or None where the background shows. Later shapes win.
fn mark_tone_at(p: (f32, f32), pal: &Palette) -> Option<Rgb> {
    let mut out = None;
    for s in MARK {
        let hit = match s {
            Shape::Stroke { pts, closed, half, tone } => stroke_hit(pts, *closed, *half, p).then_some(*tone),
            Shape::Fill { subs, tone } => fill_hit(subs, p).then_some(*tone),
        };
        if let Some(t) = hit {
            out = Some(t.rgb(pal));
        }
    }
    out
}

/// The mark's square at a panel size: side and the top-left corner (x, y). Sits 110 px (at 1440 lines) above the
/// middle, so the bar sits 110 px below it.
pub fn mark_box(width: u32, height: u32) -> (u32, u32, u32) {
    let side = (160 * height / 1440).max(16);
    let x = width.saturating_sub(side) / 2;
    let y = (height / 2).saturating_sub(110 * height / 1440);
    (x, y, side)
}

/// The progress bar's rectangle (x, y, w, h) for a panel size, so stages can redraw only the bar.
pub fn bar_rect(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let bw = 240 * height / 1440;
    let bh = (4 * height / 1440).max(2);
    let x = width.saturating_sub(bw) / 2;
    let y = height / 2 + 110 * height / 1440;
    (x, y, bw, bh)
}

/// A drawing target: `px` with `stride` pixels per row, clipped to `w` x `h`.
struct Canvas<'a> {
    px: &'a mut [u32],
    stride: usize,
    w: i64,
    h: i64,
    order: PixelOrder,
}

impl Canvas<'_> {
    fn set(&mut self, x: i64, y: i64, v: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if let Some(p) = self.px.get_mut(y as usize * self.stride + x as usize) {
            *p = v;
        }
    }

    fn fill_all(&mut self, v: u32) {
        for y in 0..self.h as usize {
            let start = y * self.stride;
            if let Some(row) = self.px.get_mut(start..start + self.w as usize) {
                row.fill(v);
            }
        }
    }

    /// Fills the rectangle [x0, x1) x [y0, y1).
    fn rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        let v = pack(c, self.order);
        for y in y0.max(0)..y1.min(self.h) {
            for x in x0.max(0)..x1.min(self.w) {
                self.set(x, y, v);
            }
        }
    }

    fn disc(&mut self, cx: i64, cy: i64, r: i64, c: Rgb) {
        let v = pack(c, self.order);
        for y in (cy - r).max(0)..=(cy + r).min(self.h - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.w - 1) {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= r * r {
                    self.set(x, y, v);
                }
            }
        }
    }

    /// A rounded rectangle with radius h/2 (a capsule).
    fn capsule(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgb) {
        let r = h / 2;
        self.rect(x + r, y, x + w - r, y + h, c);
        self.disc(x + r, y + r, r, c);
        self.disc(x + w - r - 1, y + r, r, c);
    }

    /// The mark, supersampled 2x2 per pixel, inside the square (x0, y0, side).
    fn mark(&mut self, x0: i64, y0: i64, side: i64, pal: &Palette) {
        let k = 1024.0 / side as f32;
        for y in y0.max(0)..(y0 + side).min(self.h) {
            for x in x0.max(0)..(x0 + side).min(self.w) {
                let mut acc = [0u32; 3];
                for (sx, sy) in [(0.25f32, 0.25f32), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let p = ((x as f32 + sx - x0 as f32) * k, (y as f32 + sy - y0 as f32) * k);
                    let Rgb(r, g, b) = mark_tone_at(p, pal).unwrap_or(pal.bg);
                    acc[0] += r as u32;
                    acc[1] += g as u32;
                    acc[2] += b as u32;
                }
                let c = Rgb((acc[0] / 4) as u8, (acc[1] / 4) as u8, (acc[2] / 4) as u8);
                self.set(x, y, pack(c, self.order));
            }
        }
    }
}

/// Draws `frame` into `out` (`stride` pixels per row, at least `frame.width`, and `frame.height` rows).
pub fn render(frame: &Frame, out: &mut [u32], stride: usize, order: PixelOrder) {
    let (w, h) = (frame.width, frame.height);
    let pal = palette(frame.appearance);
    let mut c = Canvas { px: out, stride, w: w as i64, h: h as i64, order };
    c.fill_all(pack(pal.bg, order));
    let (mx, my, side) = mark_box(w, h);
    c.mark(mx as i64, my as i64, side as i64, &pal);
    // No progress bar: the boot frame is the mark alone (`frame.progress` is kept for the timing logic only).
    // the controls sit where the bar's row continues
    let (_, by, _, bh) = bar_rect(w, h);
    let y_ctl = (by + bh) as i64;
    let wi = w as i64;
    let unit = |v: u32| (v * h / 1440).max(1) as i64;
    match &frame.screen {
        Screen::Boot | Screen::Rollback | Screen::RestartedAfterProblem | Screen::ShuttingDown => {}
        Screen::Unlock { dots, failed, .. } => {
            // the password field sits where the bar is, and one disc per typed character
            let fw = unit(280).max(40);
            let fh = unit(28).max(10);
            let fx = (wi - fw) / 2;
            let fy = by as i64;
            if *failed {
                c.capsule(fx, fy, fw, fh, pal.danger);
                c.capsule(fx + 2, fy + 2, fw - 4, fh - 4, pal.plate);
            } else {
                c.capsule(fx, fy, fw, fh, pal.plate);
            }
            let n = (*dots).min(12) as i64;
            let d = (fh * 35 / 100).max(3);
            let step = 2 * d;
            let total = if n > 0 { (n - 1) * step + d } else { 0 };
            let start = fx + fw / 2 - total / 2;
            let cy = fy + fh / 2;
            for i in 0..n {
                c.disc(start + i * step + d / 2, cy, d / 2, pal.ink);
            }
        }
        Screen::Options { entries, selected } => {
            // one capsule per entry; the selected one takes the accent (labels come with the glyph set)
            let rh = unit(36).max(12);
            let rw = unit(300).max(120);
            let gap = unit(6).max(2);
            let y0 = y_ctl + unit(16).max(4);
            let x = (wi - rw) / 2;
            for i in 0..entries.len() as i64 {
                let on = i as u32 == *selected;
                c.capsule(x, y0 + i * (rh + gap), rw, rh, if on { pal.accent } else { pal.plate });
            }
        }
        Screen::StartFailed { selected } => {
            // two buttons, Restart then Recovery; the selected one takes the accent
            let bw = unit(150).max(60);
            let bh = unit(30).max(12);
            let gap = unit(16).max(4);
            let total = 2 * bw + gap;
            let x0 = wi / 2 - total / 2;
            let y = y_ctl + unit(24).max(6);
            for i in 0..2i64 {
                let on = i as u32 == *selected;
                c.capsule(x0 + i * (bw + gap), y, bw, bh, if on { pal.accent } else { pal.plate });
            }
        }
    }
}

/// Kept so callers need no change: the boot frame has no progress bar any more, so this draws nothing.
pub fn render_bar(_width: u32, _height: u32, _progress: f32, _appearance: Appearance, _out: &mut [u32], _stride: usize, _order: PixelOrder) {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn boot(w: u32, h: u32, appearance: Appearance, progress: Option<f32>, screen: Screen) -> Vec<u32> {
        let mut px = vec![0u32; (w * h) as usize];
        render(&Frame { width: w, height: h, appearance, progress, screen }, &mut px, w as usize, PixelOrder::Bgrx);
        px
    }

    #[test]
    fn bar_geometry_at_1080p_and_1440p() {
        // 1920x1080: bar 180x3 (4*1080/1440 = 3), at y = 540 + 82 (110*1080/1440 = 82.5 -> 82), x = (1920-180)/2
        assert_eq!(bar_rect(1920, 1080), (870, 622, 180, 3));
        // 2560x1440: bar 240x4 at y = 720 + 110, x = (2560-240)/2
        assert_eq!(bar_rect(2560, 1440), (1160, 830, 240, 4));
    }

    #[test]
    fn bar_rect_keeps_a_visible_height_on_small_panels() {
        // 640x360: 4*360/1440 = 1 -> clamped to 2 rows; 240*360/1440 = 60 wide
        let (x, _, w, h) = bar_rect(640, 360);
        assert_eq!((x, w, h), (290, 60, 2));
    }

    #[test]
    fn mark_box_is_centred_and_scales_with_the_short_side() {
        // 1920x1080: side 160*1080/1440 = 120, x = (1920-120)/2 = 900, y = 540 - 82 = 458
        assert_eq!(mark_box(1920, 1080), (900, 458, 120));
        assert_eq!(mark_box(2560, 1440), (1200, 610, 160));
        // 640x360: side 40, x = 300, y = 180 - 27 (110*360/1440 = 27.5 -> 27)
        assert_eq!(mark_box(640, 360), (300, 153, 40));
    }

    #[test]
    fn packing_follows_the_pixel_order() {
        let fill = Rgb(229, 229, 234);
        assert_eq!(pack(fill, PixelOrder::Bgrx), 0xffe5_e5ea);
        assert_eq!(pack(fill, PixelOrder::Rgbx), 0xffea_e5e5);
        assert_eq!(pack(Rgb(0, 0, 0), PixelOrder::Bgrx), 0xff00_0000);
    }

    #[test]
    fn the_boot_frame_has_no_progress_bar() {
        // with or without progress the frame is identical, and the old bar row is background
        let with = boot(1920, 1080, Appearance::Dark, Some(0.5), Screen::Boot);
        let without = boot(1920, 1080, Appearance::Dark, None, Screen::Boot);
        assert_eq!(with, without);
        assert_eq!(with[622 * 1920 + 915], 0xff00_0000);
        let mut px = with.clone();
        render_bar(1920, 1080, 1.0, Appearance::Dark, &mut px, 1920, PixelOrder::Bgrx);
        assert_eq!(px, with, "render_bar is a no-op");
    }

    #[test]
    fn background_is_black_in_dark_and_white_in_light() {
        let dark = boot(1920, 1080, Appearance::Dark, None, Screen::Boot);
        assert_eq!(dark[0], 0xff00_0000);
        assert_eq!(dark[1919 + 1079 * 1920], 0xff00_0000);
        let light = boot(1920, 1080, Appearance::Light, None, Screen::Boot);
        assert_eq!(light[0], 0xffff_ffff);
        // without progress there is no bar
        assert_eq!(dark[622 * 1920 + 915], 0xff00_0000);
    }

    #[test]
    fn stroke_has_butt_ends_and_round_joins() {
        // the blue arch's left leg: x = 162, y 620..756, half width 26
        let leg = ARCH_BLUE;
        assert!(stroke_hit(leg, false, 26.0, (162.0, 700.0)), "on the leg");
        assert!(stroke_hit(leg, false, 26.0, (138.0, 700.0)), "inside the stroke width");
        assert!(!stroke_hit(leg, false, 26.0, (100.0, 700.0)), "outside the stroke width");
        assert!(!stroke_hit(leg, false, 26.0, (162.0, 780.0)), "past the butt end at y 756");
        // the vertex at (512, 270) is a join: a round disc of radius 26 covers the top of the arch
        assert!(stroke_hit(leg, false, 26.0, (512.0, 250.0)));
    }

    #[test]
    fn fill_uses_even_odd_so_the_rune_has_holes() {
        // (430, 600) is inside the rune's outer outline, (460, 690) is in hole A
        assert!(fill_hit(RUNE, (430.0, 600.0)));
        assert!(!fill_hit(RUNE, (500.0, 690.0)), "hole A (triangle 460..540 x 652..718)");
        assert!(!fill_hit(RUNE, (300.0, 300.0)), "outside");
        // the star's stem (512, 230) is inside
        assert!(fill_hit(STAR_SUBS, (512.0, 230.0)));
    }

    #[test]
    fn mark_layers_match_the_svg_order_and_leave_the_arch_hollow() {
        let pal = palette(Appearance::Dark);
        // the outline's left edge at x = 80 is blue
        assert_eq!(mark_tone_at((80.0, 512.0), &pal), Some(BLUE));
        // the arch's interior at (512, 600) is background
        assert_eq!(mark_tone_at((512.0, 600.0), &pal), None);
        // the star is the light ink in Dark and the dark ink in Light
        assert_eq!(mark_tone_at((512.0, 230.0), &pal), Some(pal.ink));
        let light = palette(Appearance::Light);
        assert_eq!(mark_tone_at((512.0, 230.0), &light), Some(light.ink));
        assert_eq!(light.ink, Rgb(28, 28, 30));
    }

    #[test]
    fn every_screen_renders_deterministically_in_both_appearances_and_sizes() {
        let screens = [
            Screen::Boot,
            Screen::Unlock { name: alloc::string::String::from("n"), dots: 5, failed: true },
            Screen::Options { entries: alloc::vec![alloc::string::String::from("a"), alloc::string::String::from("b")], selected: 1 },
            Screen::Rollback,
            Screen::StartFailed { selected: 0 },
            Screen::RestartedAfterProblem,
            Screen::ShuttingDown,
        ];
        for (w, h) in [(1920u32, 1080u32), (2560, 1440)] {
            for appearance in [Appearance::Dark, Appearance::Light] {
                for screen in screens.iter() {
                    let frame = Frame { width: w, height: h, appearance, progress: Some(0.3), screen: screen.clone() };
                    let mut a = vec![0u32; (w * h) as usize];
                    let mut b = vec![0u32; (w * h) as usize];
                    render(&frame, &mut a, w as usize, PixelOrder::Bgrx);
                    render(&frame, &mut b, w as usize, PixelOrder::Bgrx);
                    assert_eq!(a, b, "{screen:?} {w}x{h} {appearance:?}");
                    assert_eq!(a[0], pack(palette(appearance).bg, PixelOrder::Bgrx), "corner is the background");
                }
            }
        }
    }

    #[test]
    fn the_unlock_field_shows_one_disc_per_character_and_a_red_ring_when_failed() {
        let (w, h) = (1920u32, 1080u32);
        let ok = boot(w, h, Appearance::Dark, None, Screen::Unlock { name: alloc::string::String::new(), dots: 3, failed: false });
        let bad = boot(w, h, Appearance::Dark, None, Screen::Unlock { name: alloc::string::String::new(), dots: 3, failed: true });
        // the field: fw = 280*1080/1440 = 210, fh = 28*1080/1440 = 21, x = 855, y = 622
        // the ring is 2 px wide on the field's left edge at mid height (x 855..857, y 632)
        assert_eq!(bad[632 * 1920 + 856] & 0x00ff_ffff, 0x00ff_453a, "red ring");
        assert_eq!(ok[632 * 1920 + 856] & 0x00ff_ffff, 0x002c_2c2e, "plate, no ring");
        // the centre row holds three dots of ink: fine check on the first dot's centre
        let d = 7; // fh*35/100 = 21*35/100 = 7
        let step = 14;
        let total = 2 * step + d; // 35
        let start = 855 + 210 / 2 - total / 2; // 855 + 105 - 17 = 943
        let cy = 622 + 21 / 2; // 632
        let first = (start + d / 2) as usize; // 946
        assert_eq!(ok[cy as usize * 1920 + first] & 0x00ff_ffff, 0x00f2_f2f7, "first dot centre is ink");
    }

    #[test]
    fn the_start_failed_selection_is_the_accent() {
        let (w, h) = (1920u32, 1080u32);
        let px = boot(w, h, Appearance::Dark, None, Screen::StartFailed { selected: 1 });
        // bar row 622 is the controls' row origin: y = 622 + 3 + 6 (unit 24 -> 24*1080/1440 = 18)
        // the buttons: bw 150*1080/1440 = 112, gap 16*1080/1440 = 12, total 236, x0 = 960 - 118 = 842
        // selected button 1 starts at 842 + 112 + 12 = 966; its centre row: y = 625 + 18 + 11
        let y = 625 + 18 + 11;
        assert_eq!(px[y * 1920 + 966 + 40] & 0x00ff_ffff, 0x000a_84ff, "recovery is the accent");
        assert_eq!(px[y * 1920 + 842 + 40] & 0x00ff_ffff, 0x002c_2c2e, "restart is the plate");
    }
}
