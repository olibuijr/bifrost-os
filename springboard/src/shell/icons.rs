//! The coloured icon tiles of the Settings list and the small glyphs its
//! rows use (padlock, info button). Glyphs come from the symbol fonts when
//! they have them (gear, airplane) and are drawn from primitives otherwise,
//! so the look never depends on an asset.

use super::ui::{Ui, Weight, WHITE};
use crate::gfx::{over, pack, rgba, scale, Surface};
use crate::status::{draw_wifi_bars, BATTERY_H_PT, BATTERY_W_PT};
use crate::symbols::Symbol;

/// Side of an icon tile in points, as on the bifrost (29 pt, 6.5 pt corners).
pub const TILE_PT: f32 = 29.0;
pub const TILE_RADIUS_PT: f32 = 6.5;

pub const BLUE: u32 = pack(10, 132, 255, 255);
pub const RED: u32 = pack(255, 69, 58, 255);
const ORANGE: u32 = pack(255, 159, 10, 255);
const PINK: u32 = pack(255, 45, 85, 255);
const INDIGO: u32 = pack(94, 92, 230, 255);
const GRAY: u32 = pack(142, 142, 147, 255);
const TEAL: u32 = pack(50, 173, 230, 255);
pub const GREEN: u32 = pack(52, 199, 89, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Airplane,
    Wifi,
    Bluetooth,
    Notifications,
    Sounds,
    Focus,
    ScreenTime,
    General,
    ControlCentre,
    Display,
    HomeScreen,
    Accessibility,
    Wallpaper,
    Battery,
    Privacy,
    Passcode,
}

impl Icon {
    pub const ALL: [Icon; 16] = [
        Icon::Airplane,
        Icon::Wifi,
        Icon::Bluetooth,
        Icon::Notifications,
        Icon::Sounds,
        Icon::Focus,
        Icon::ScreenTime,
        Icon::General,
        Icon::ControlCentre,
        Icon::Display,
        Icon::HomeScreen,
        Icon::Accessibility,
        Icon::Wallpaper,
        Icon::Battery,
        Icon::Privacy,
        Icon::Passcode,
    ];

    /// The tile's background colour.
    pub fn color(self) -> u32 {
        match self {
            Icon::Airplane => ORANGE,
            Icon::Wifi | Icon::Bluetooth | Icon::Display | Icon::HomeScreen | Icon::Accessibility | Icon::Privacy => BLUE,
            Icon::Notifications | Icon::Passcode => RED,
            Icon::Sounds => PINK,
            Icon::Focus | Icon::ScreenTime => INDIGO,
            Icon::General | Icon::ControlCentre => GRAY,
            Icon::Wallpaper => TEAL,
            Icon::Battery => GREEN,
        }
    }
}

/// An anti-aliased line segment with round caps, `w` px wide.
pub fn line(s: &mut Surface, (x0, y0): (f32, f32), (x1, y1): (f32, f32), w: f32, color: u32) {
    let r = w / 2.0;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = dx * dx + dy * dy;
    let (bx0, by0) = ((x0.min(x1) - r - 1.0).floor().max(0.0), (y0.min(y1) - r - 1.0).floor().max(0.0));
    let (bx1, by1) = ((x0.max(x1) + r + 1.0).ceil(), (y0.max(y1) + r + 1.0).ceil());
    for py in (by0 as usize).max(s.y0)..(by1 as usize).min(s.h) {
        for px in bx0 as usize..(bx1 as usize).min(s.w) {
            let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
            let k = if len2 > 0.0 { ((cx - x0) * dx + (cy - y0) * dy) / len2 } else { 0.0 }.clamp(0.0, 1.0);
            let (nx, ny) = (x0 + k * dx, y0 + k * dy);
            let d = ((cx - nx).powi(2) + (cy - ny).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let p = s.at(px, py);
                *p = over(*p, scale(color, (cov * 255.0) as u32));
            }
        }
    }
}

/// A filled polygon, 4x4 supersampled (even-odd rule).
pub fn polygon(s: &mut Surface, pts: &[(f32, f32)], color: u32) {
    let inside = |px: f32, py: f32| {
        let mut odd = false;
        for i in 0..pts.len() {
            let ((ax, ay), (bx, by)) = (pts[i], pts[(i + 1) % pts.len()]);
            if (ay > py) != (by > py) && px < ax + (py - ay) * (bx - ax) / (by - ay) {
                odd = !odd;
            }
        }
        odd
    };
    let (x0, x1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    for py in (y0.max(0.0) as usize).max(s.y0)..(y1.ceil() as usize).min(s.h) {
        for px in (x0.max(0.0) as usize)..(x1.ceil() as usize).min(s.w) {
            let hits = (0..16).filter(|i| inside(px as f32 + (i % 4) as f32 / 4.0 + 0.125, py as f32 + (i / 4) as f32 / 4.0 + 0.125)).count();
            if hits > 0 {
                let p = s.at(px, py);
                *p = over(*p, scale(color, (hits * 255 / 16) as u32));
            }
        }
    }
}

/// A filled circle of radius `r` around (cx, cy).
pub fn disc(s: &mut Surface, cx: f32, cy: f32, r: f32, color: u32) {
    s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, color);
}

/// The lock screen's camera glyph, centred on (cx, cy), 14 pt wide.
pub fn camera(s: &mut Surface, ui: &Ui, cx: f32, cy: f32, color: u32) {
    let (w, h) = (ui.pt(14.0), ui.pt(10.0));
    s.fill_rrect(cx - w / 2.0, cy - h / 2.0 + ui.pt(1.0), w, h, ui.pt(2.0), color);
    s.fill_rrect(cx - ui.pt(2.5), cy - h / 2.0 - ui.pt(0.5), ui.pt(5.0), ui.pt(2.0), ui.pt(1.0), color);
    disc(s, cx, cy + ui.pt(1.0), ui.pt(2.6), rgba(0, 0, 0, 140));
}

/// A circle outline of radius `r`, `t` px thick (inside the radius).
pub fn ring(s: &mut Surface, cx: f32, cy: f32, r: f32, t: f32, color: u32) {
    s.stroke_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, t, color);
}

/// An arc of radius `r`, `t` px thick, between angles `a0..a1` (radians,
/// clockwise from +x on screen).
pub fn arc(s: &mut Surface, cx: f32, cy: f32, r: f32, t: f32, a0: f32, a1: f32, color: u32) {
    let (lo, hi) = (r - t / 2.0, r + t / 2.0);
    for py in ((cy - hi - 1.0).max(0.0) as usize).max(s.y0)..((cy + hi + 1.0).ceil() as usize).min(s.h) {
        for px in (cx - hi - 1.0).max(0.0) as usize..((cx + hi + 1.0).ceil() as usize).min(s.w) {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let a = dy.atan2(dx);
            if a < a0 || a > a1 {
                continue;
            }
            let d = (dx * dx + dy * dy).sqrt();
            let cov = (d - lo + 0.5).min(hi - d + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let p = s.at(px, py);
                *p = over(*p, scale(color, (cov * 255.0) as u32));
            }
        }
    }
}

/// Padlock: rounded body under a stroked shackle; (x, y) is the top-left of
/// its 9 x 12 pt box.
pub fn lock(s: &mut Surface, ui: &Ui, x: f32, y: f32, color: u32) {
    let (bw, sw) = (ui.pt(9.0), ui.pt(6.4));
    s.stroke_rrect(x + (bw - sw) / 2.0, y, sw, ui.pt(9.0), sw / 2.0, ui.pt(1.5), color);
    s.fill_rrect(x, y + ui.pt(5.0), bw, ui.pt(7.0), ui.pt(1.6), color);
}

/// The ⓘ detail button, 22 pt across, centred on (cx, cy).
pub fn info(s: &mut Surface, ui: &Ui, cx: f32, cy: f32) {
    ring(s, cx, cy, ui.pt(11.0), ui.pt(1.3), BLUE);
    let i = ui.chrome("i", 14.0, Weight::Semibold, BLUE);
    s.blit(&i, (cx - i.w as f32 / 2.0).round() as i32, (cy - i.h as f32 / 2.0).round() as i32, 255, 255);
}

/// A symbol from the loaded fonts fitted into `w` x `h` px, centred on (cx, cy).
fn symbol(s: &mut Surface, ui: &Ui, sym: Symbol, cx: f32, cy: f32, w: f32, h: f32, color: u32) -> bool {
    match ui.symbols.render_fit(sym, w, h, color) {
        Some(img) => {
            s.blit(&img, (cx - img.w as f32 / 2.0).round() as i32, (cy - img.h as f32 / 2.0).round() as i32, 255, 255);
            true
        }
        None => false,
    }
}

/// `tile` at a side of `side_pt` points: the artwork is decoded at that size; a drawn fallback is rendered at the
/// standard tile size and scaled down so its details stay in proportion.
pub fn tile_sized(s: &mut Surface, ui: &Ui, icon: Icon, x: f32, y: f32, side_pt: f32) {
    let side = ui.pt(side_pt).round().max(1.0) as usize;
    if let Some(img) = artwork(icon, side) {
        s.blit(&img, x.round() as i32, y.round() as i32, 255, 255);
        return;
    }
    let full = ui.pt(TILE_PT).round().max(1.0) as usize;
    let mut tmp = crate::gfx::Image::new(full, full);
    {
        let mut ts = Surface { w: full, h: full, stride: full, px: &mut tmp.px, y0: 0 };
        tile(&mut ts, ui, icon, 0.0, 0.0);
    }
    let small = tmp.resized(side, side);
    s.blit(&small, x.round() as i32, y.round() as i32, 255, 255);
}

/// Draws the tile for `icon` with its top-left at (x, y) px.
pub fn tile(s: &mut Surface, ui: &Ui, icon: Icon, x: f32, y: f32) {
    let size = ui.pt(TILE_PT);
    if let Some(img) = artwork(icon, size.round() as usize) {
        s.blit(&img, x.round() as i32, y.round() as i32, 255, 255);
        return;
    }
    s.fill_rrect(x, y, size, size, ui.pt(TILE_RADIUS_PT), icon.color());
    // tile-local coordinates in points
    let p = |px: f32, py: f32| (x + ui.pt(px), y + ui.pt(py));
    let (cx, cy) = p(TILE_PT / 2.0, TILE_PT / 2.0);
    let bg = icon.color();
    match icon {
        Icon::Airplane => {
            if !symbol(s, ui, Symbol::Airplane, cx, cy, ui.pt(19.0), ui.pt(19.0), WHITE) {
                polygon(s, &[p(14.5, 5.0), p(16.5, 7.0), p(16.5, 12.0), p(24.0, 17.0), p(24.0, 19.0), p(16.5, 16.5), p(16.5, 21.0), p(19.0, 23.0), p(19.0, 24.5), p(14.5, 23.5), p(10.0, 24.5), p(10.0, 23.0), p(12.5, 21.0), p(12.5, 16.5), p(5.0, 19.0), p(5.0, 17.0), p(12.5, 12.0), p(12.5, 7.0)], WHITE);
            }
        }
        Icon::Wifi => {
            let r = ui.pt(9.5);
            draw_wifi_bars(s, cx - r, cy - r * 0.55, r, 3, WHITE);
        }
        Icon::Bluetooth => {
            let w = ui.pt(2.0);
            line(s, p(14.5, 5.0), p(14.5, 24.0), w, WHITE);
            line(s, p(14.5, 5.0), p(19.5, 10.0), w, WHITE);
            line(s, p(19.5, 10.0), p(9.5, 19.0), w, WHITE);
            line(s, p(14.5, 24.0), p(19.5, 19.0), w, WHITE);
            line(s, p(19.5, 19.0), p(9.5, 10.0), w, WHITE);
        }
        Icon::Notifications => {
            let (dx, dy) = p(8.0, 12.0);
            disc(s, cx, dy, ui.pt(6.5), WHITE);
            s.fill_rrect(dx, dy, ui.pt(13.0), ui.pt(7.5), 0.0, WHITE);
            let (lx, ly) = p(6.0, 18.5);
            s.fill_rrect(lx, ly, ui.pt(17.0), ui.pt(2.4), ui.pt(1.2), WHITE);
            let (_, ky) = p(0.0, 22.8);
            disc(s, cx, ky, ui.pt(2.0), WHITE);
        }
        Icon::Sounds => {
            polygon(s, &[p(6.0, 11.5), p(10.0, 11.5), p(15.0, 7.5), p(15.0, 21.5), p(10.0, 17.5), p(6.0, 17.5)], WHITE);
            let (ax, ay) = p(15.5, 14.5);
            let q = std::f32::consts::FRAC_PI_4;
            arc(s, ax, ay, ui.pt(4.5), ui.pt(1.7), -q, q, WHITE);
            arc(s, ax, ay, ui.pt(8.0), ui.pt(1.7), -q, q, WHITE);
        }
        Icon::Focus => {
            disc(s, cx, cy, ui.pt(8.0), WHITE);
            let (mx, my) = p(18.5, 12.0);
            disc(s, mx, my, ui.pt(7.0), bg);
        }
        Icon::ScreenTime => {
            polygon(s, &[p(9.0, 6.5), p(20.0, 6.5), p(14.5, 14.5)], WHITE);
            polygon(s, &[p(9.0, 22.5), p(20.0, 22.5), p(14.5, 14.5)], WHITE);
            let (bx, by) = p(8.0, 5.0);
            s.fill_rrect(bx, by, ui.pt(13.0), ui.pt(2.0), ui.pt(1.0), WHITE);
            let (bx, by) = p(8.0, 22.0);
            s.fill_rrect(bx, by, ui.pt(13.0), ui.pt(2.0), ui.pt(1.0), WHITE);
        }
        Icon::General => {
            if !symbol(s, ui, Symbol::Gear, cx, cy, ui.pt(21.0), ui.pt(21.0), WHITE) {
                for i in 0..8 {
                    let a = i as f32 * std::f32::consts::FRAC_PI_4;
                    line(s, (cx + a.cos() * ui.pt(6.0), cy + a.sin() * ui.pt(6.0)), (cx + a.cos() * ui.pt(9.5), cy + a.sin() * ui.pt(9.5)), ui.pt(3.2), WHITE);
                }
                ring(s, cx, cy, ui.pt(7.0), ui.pt(3.0), WHITE);
            }
        }
        Icon::ControlCentre => {
            for (ty, knob_right) in [(8.0, true), (16.0, false)] {
                let (tx, ty) = p(7.0, ty);
                let (w, h) = (ui.pt(15.0), ui.pt(5.5));
                s.fill_rrect(tx, ty, w, h, h / 2.0, WHITE);
                let kx = if knob_right { tx + w - h / 2.0 } else { tx + h / 2.0 };
                disc(s, kx, ty + h / 2.0, h / 2.0 - ui.pt(0.9), bg);
            }
        }
        Icon::Display => {
            let small = ui.chrome("A", 12.0, Weight::Semibold, WHITE);
            let big = ui.chrome("A", 17.0, Weight::Semibold, WHITE);
            let total = small.w as f32 + big.w as f32 - ui.pt(3.0);
            let left = (cx - total / 2.0).round() as i32;
            let base = (cy + big.h as f32 / 2.0) as i32;
            s.blit(&small, left, base - small.h as i32 - ui.pt(1.0) as i32, 255, 255);
            s.blit(&big, left + small.w as i32 - ui.pt(3.0) as i32, base - big.h as i32, 255, 255);
        }
        Icon::HomeScreen => {
            const DOTS: [u32; 12] = [
                pack(255, 69, 58, 255),
                pack(255, 159, 10, 255),
                pack(255, 214, 10, 255),
                pack(48, 209, 88, 255),
                pack(100, 210, 255, 255),
                pack(191, 90, 242, 255),
                pack(255, 55, 95, 255),
                pack(255, 214, 10, 255),
                WHITE,
                WHITE,
                WHITE,
                WHITE,
            ];
            let d = ui.pt(3.4);
            let gap = ui.pt(1.6);
            let (gx, gy) = p(5.5, 6.5);
            for (i, c) in DOTS.iter().enumerate() {
                let (col, row) = ((i % 4) as f32, (i / 4) as f32);
                s.fill_rrect(gx + col * (d + gap), gy + row * (d + gap) + if row == 2.0 { ui.pt(1.5) } else { 0.0 }, d, d, ui.pt(0.8), *c);
            }
        }
        Icon::Accessibility => {
            ring(s, cx, cy, ui.pt(10.5), ui.pt(1.6), WHITE);
            let w = ui.pt(1.8);
            disc(s, cx, cy - ui.pt(5.5), ui.pt(1.9), WHITE);
            line(s, p(9.5, 12.0), p(19.5, 12.0), w, WHITE);
            line(s, (cx, cy - ui.pt(2.5)), (cx, cy + ui.pt(2.5)), w, WHITE);
            line(s, (cx, cy + ui.pt(2.5)), p(11.5, 21.0), w, WHITE);
            line(s, (cx, cy + ui.pt(2.5)), p(17.5, 21.0), w, WHITE);
        }
        Icon::Wallpaper => {
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::FRAC_PI_3;
                disc(s, cx + a.cos() * ui.pt(5.2), cy + a.sin() * ui.pt(5.2), ui.pt(3.4), WHITE);
            }
            disc(s, cx, cy, ui.pt(2.6), bg);
        }
        Icon::Battery => {
            let k = ui.pt(0.72);
            let (bw, bh) = (BATTERY_W_PT * k, BATTERY_H_PT * k);
            let (bx, by) = (cx - (bw + 2.0 * k) / 2.0, cy - bh / 2.0);
            s.stroke_rrect(bx, by, bw, bh, 3.5 * k, 1.4 * k, WHITE);
            s.fill_rrect(bx + 2.6 * k, by + 2.6 * k, bw - 5.2 * k, bh - 5.2 * k, 1.6 * k, WHITE);
            s.fill_rrect(bx + bw + k, by + 3.5 * k, 1.6 * k, bh - 7.0 * k, 0.8 * k, WHITE);
        }
        Icon::Privacy | Icon::Passcode => {
            let (px0, py0) = p(9.0, 13.0);
            s.fill_rrect(px0, py0, ui.pt(11.5), ui.pt(10.5), ui.pt(3.5), WHITE);
            let w = ui.pt(2.6);
            line(s, p(10.5, 14.0), p(10.5, 8.5), w, WHITE);
            line(s, p(13.6, 14.0), p(13.6, 6.0), w, WHITE);
            line(s, p(16.7, 14.0), p(16.7, 7.0), w, WHITE);
            line(s, p(19.5, 15.0), p(19.5, 9.5), w, WHITE);
            line(s, p(10.0, 17.0), p(6.0, 13.0), w, WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Vec<u32> {
        vec![0u32; 80 * 80]
    }

    fn surface(buf: &mut [u32]) -> Surface<'_> {
        Surface { w: 80, h: 80, stride: 80, px: buf, y0: 0 }
    }

    #[test]
    fn primitives_cover_their_shape_only() {
        let mut buf = canvas();
        line(&mut surface(&mut buf), (10.0, 40.0), (70.0, 40.0), 4.0, WHITE);
        assert_eq!(buf[40 * 80 + 40], WHITE, "on the line");
        assert_eq!(buf[40 * 80 + 5], 0, "before the cap");
        assert_eq!(buf[30 * 80 + 40], 0, "off the line");
        let mut buf = canvas();
        polygon(&mut surface(&mut buf), &[(10.0, 10.0), (70.0, 10.0), (40.0, 70.0)], WHITE);
        assert_eq!(buf[20 * 80 + 40], WHITE);
        assert_eq!(buf[60 * 80 + 10], 0);
        let mut buf = canvas();
        arc(&mut surface(&mut buf), 40.0, 40.0, 20.0, 4.0, -0.8, 0.8, WHITE);
        assert!(buf[40 * 80 + 60] >> 24 > 200, "on the arc to the right");
        assert_eq!(buf[40 * 80 + 20], 0, "nothing on the left");
        let mut buf = canvas();
        disc(&mut surface(&mut buf), 40.0, 40.0, 10.0, WHITE);
        ring(&mut surface(&mut buf), 40.0, 40.0, 30.0, 2.0, WHITE);
        assert_eq!(buf[40 * 80 + 40], WHITE);
        assert_eq!(buf[40 * 80 + 55], 0, "between disc and ring");
        assert!(buf[40 * 80 + 11] >> 24 > 128, "on the ring");
    }

    #[test]
    fn every_tile_shows_its_bifrost_artwork() {
        let ui = crate::shell::tests::ui();
        for icon in Icon::ALL {
            let size = ui.pt(TILE_PT).round() as usize;
            assert!(artwork(icon, size).is_some(), "{icon:?} artwork decodes");
            let mut buf = canvas();
            tile(&mut surface(&mut buf), &ui, icon, 10.0, 10.0);
            let inside = |x: usize, y: usize| buf[(10 + y) * 80 + 10 + x];
            assert_eq!(inside(2, size / 2) >> 24, 255, "{icon:?} tile is opaque at its edge");
            assert!(inside(0, 0) >> 24 < 128, "{icon:?} corner is rounded");
            assert_eq!(buf[5 * 80 + 5], 0, "nothing outside the tile");
        }
    }

    #[test]
    fn lock_and_info_button() {
        let ui = crate::shell::tests::ui();
        let mut buf = canvas();
        lock(&mut surface(&mut buf), &ui, 20.0, 20.0, WHITE);
        assert!(buf.iter().any(|&p| p == WHITE));
        let mut buf = canvas();
        info(&mut surface(&mut buf), &ui, 40.0, 40.0);
        assert!(buf[40 * 80 + 40] >> 24 > 100, "the i is at the centre");
        assert!(buf[40 * 80 + (40.0 - ui.pt(10.4)) as usize] >> 24 > 100, "the ring");
    }
}

/// The bifrost tile captured from the reference bifrost (`res/settings-icons`, 58 px at 2x),
/// scaled once per size; None falls back to the drawn tile.
fn artwork(icon: Icon, px: usize) -> Option<std::sync::Arc<crate::gfx::Image>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<(Icon, usize), Option<Arc<crate::gfx::Image>>>>> = OnceLock::new();
    let bytes: &[u8] = match icon {
        Icon::Airplane => include_bytes!("../../res/settings-icons/airplane.png"),
        Icon::Wifi => include_bytes!("../../res/settings-icons/wifi.png"),
        Icon::Bluetooth => include_bytes!("../../res/settings-icons/bluetooth.png"),
        Icon::Notifications => include_bytes!("../../res/settings-icons/notifications.png"),
        Icon::Sounds => include_bytes!("../../res/settings-icons/sounds.png"),
        Icon::Focus => include_bytes!("../../res/settings-icons/focus.png"),
        Icon::ScreenTime => include_bytes!("../../res/settings-icons/screentime.png"),
        Icon::General => include_bytes!("../../res/settings-icons/general.png"),
        Icon::ControlCentre => include_bytes!("../../res/settings-icons/controlcentre.png"),
        Icon::Display => include_bytes!("../../res/settings-icons/display.png"),
        Icon::HomeScreen => include_bytes!("../../res/settings-icons/homescreen.png"),
        Icon::Accessibility => include_bytes!("../../res/settings-icons/accessibility.png"),
        Icon::Wallpaper => include_bytes!("../../res/settings-icons/wallpaper.png"),
        Icon::Battery => include_bytes!("../../res/settings-icons/battery.png"),
        Icon::Privacy => include_bytes!("../../res/settings-icons/privacy.png"),
        Icon::Passcode => include_bytes!("../../res/settings-icons/passcode.png"),
    };
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().ok()?;
    c.entry((icon, px)).or_insert_with(|| crate::cg::imageio::decode(bytes).map(|i| Arc::new(i.resized(px, px)))).clone()
}
