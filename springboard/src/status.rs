//! bifrost status bar pieces: clock/date strings and the wifi and battery glyphs.

use crate::gfx::{pack, rgba, scale, Surface};

pub fn clock_text(hour: i32, min: i32) -> String {
    format!("{hour}:{min:02}")
}

/// "2:05 PM" style, as bifrost shows with 24-Hour Time off.
pub fn clock_12h(hour: i32, min: i32) -> String {
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h}:{min:02} {}", if hour < 12 { "AM" } else { "PM" })
}

/// The clock in the user's chosen style.
pub fn clock_in(hour: i32, min: i32, h24: bool) -> String {
    if h24 {
        clock_text(hour, min)
    } else {
        clock_12h(hour, min)
    }
}

pub fn date_text(wday: i32, mday: i32, mon: i32) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {mday} {}", DAYS[wday.rem_euclid(7) as usize], MONTHS[mon.rem_euclid(12) as usize])
}

/// Status-bar wifi fan radius in points (measured on the bifrost: 9.75 pt tall,
/// 13.5 pt wide).
pub const WIFI_PT: f32 = 9.75;
/// Battery body in points, and the nub plus its gap.
pub const BATTERY_W_PT: f32 = 24.0;
pub const BATTERY_H_PT: f32 = 12.0;
pub const BATTERY_NUB_PT: f32 = 2.5;

/// Wifi fan: a wedge plus two arcs inside a 90-degree sector. `size` is the
/// sector radius in px; (x, y) is the top-left of its size*2 x size box.
pub fn draw_wifi(s: &mut Surface, x: f32, y: f32, size: f32, color: u32) {
    draw_wifi_bars(s, x, y, size, 3, color);
}

/// The wifi fan showing signal strength: the first `bars` (0..=3) bands are
/// lit, the rest drawn faintly as in the bifrost network list. Band radii are
/// fractions of `size`, measured on the bifrost's status bar.
pub fn draw_wifi_bars(s: &mut Surface, x: f32, y: f32, size: f32, bars: u8, color: u32) {
    const BANDS: [(f32, f32); 3] = [(0.0, 0.28), (0.44, 0.64), (0.79, 1.0)];
    const DIM: f32 = 0.3;
    let (cx, cy) = (x + size, y + size);
    for py in (y.max(0.0) as usize).max(s.y0)..((cy.ceil() as usize).min(s.h)) {
        for px in (x.max(0.0) as usize)..(((x + 2.0 * size).ceil() as usize).min(s.w)) {
            let (dx, dy) = (px as f32 + 0.5 - cx, cy - (py as f32 + 0.5));
            let r = (dx * dx + dy * dy).sqrt();
            // signed distance to the +/-45 degree sector edges, in px
            let side = (dy - dx.abs()) / std::f32::consts::SQRT_2;
            let mut cov = 0.0f32;
            for (i, (a, b)) in BANDS.into_iter().enumerate() {
                let radial = (r - a * size + 0.5).min(b * size - r + 0.5).clamp(0.0, 1.0);
                cov = cov.max(if i < bars as usize { radial } else { radial * DIM });
            }
            cov *= (side + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let d = s.at(px, py);
                *d = crate::gfx::over(*d, scale(color, (cov * 255.0) as u32));
            }
        }
    }
}

/// Battery: rounded outline, level fill (green while charging) and nub.
/// `pt` is px per point; the body is 24 x 12 pt with (x, y) its top-left,
/// a 1 pt outline at 40% white, the fill inset 2 pt and a 1.5 x 4.5 pt nub
/// 1 pt to the right, all as measured on the bifrost.
pub fn draw_battery(s: &mut Surface, x: f32, y: f32, pt: f32, level: u8, charging: bool) {
    draw_battery_mode(s, x, y, pt, level, charging, false);
}

/// `draw_battery` with Low Power Mode: the fill turns systemYellow (charging
/// green wins, as on bifrost).
pub fn draw_battery_mode(s: &mut Surface, x: f32, y: f32, pt: f32, level: u8, charging: bool, low_power: bool) {
    let (w, h) = (BATTERY_W_PT * pt, BATTERY_H_PT * pt);
    s.stroke_rrect(x, y, w, h, 3.5 * pt, pt, rgba(255, 255, 255, 102));
    let inner = (x + 2.0 * pt, y + 2.0 * pt, w - 4.0 * pt, h - 4.0 * pt);
    let fill_w = inner.2 * level.min(100) as f32 / 100.0;
    let color = if charging {
        pack(52, 199, 89, 255)
    } else if low_power {
        pack(255, 204, 0, 255)
    } else {
        pack(255, 255, 255, 255)
    };
    if fill_w > 0.5 {
        s.fill_rrect(inner.0, inner.1, fill_w, inner.3, 1.6 * pt, color);
    }
    s.fill_rrect(x + w + pt, y + 3.75 * pt, 1.5 * pt, 4.5 * pt, 0.75 * pt, rgba(255, 255, 255, 128));
    if charging {
        draw_bolt(s, x + w / 2.0 - 4.0 * pt, y - 1.5 * pt, 8.0 * pt, h + 3.0 * pt, pack(255, 255, 255, 255));
    }
}

/// The charging bolt (an outlined-by-shadow lightning glyph), 4x4 supersampled.
pub fn draw_bolt(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, color: u32) {
    const P: [(f32, f32); 6] = [(0.62, 0.0), (0.08, 0.58), (0.46, 0.58), (0.36, 1.0), (0.92, 0.40), (0.54, 0.40)];
    let inside = |px: f32, py: f32| {
        let mut odd = false;
        for i in 0..P.len() {
            let (a, b) = (P[i], P[(i + 1) % P.len()]);
            let (ax, ay, bx, by) = (x + a.0 * w, y + a.1 * h, x + b.0 * w, y + b.1 * h);
            if (ay > py) != (by > py) && px < ax + (py - ay) * (bx - ax) / (by - ay) {
                odd = !odd;
            }
        }
        odd
    };
    for py in (y.max(0.0) as usize).max(s.y0)..((y + h).ceil() as usize).min(s.h) {
        for px in (x.max(0.0) as usize)..((x + w).ceil() as usize).min(s.w) {
            let hits = (0..16).filter(|i| inside(px as f32 + (i % 4) as f32 / 4.0 + 0.125, py as f32 + (i / 4) as f32 / 4.0 + 0.125)).count();
            if hits > 0 {
                let d = s.at(px, py);
                *d = crate::gfx::over(*d, scale(color, (hits * 255 / 16) as u32));
            }
        }
    }
}

/// The Do Not Disturb crescent moon in a `size` px square at (x, y), 4x4 supersampled.
pub fn draw_moon(s: &mut Surface, x: f32, y: f32, size: f32, color: u32) {
    let inside = |px: f32, py: f32| {
        let (u, v) = ((px - x) / size, (py - y) / size);
        let d = |cx: f32, cy: f32| ((u - cx) * (u - cx) + (v - cy) * (v - cy)).sqrt();
        d(0.5, 0.5) <= 0.5 && d(0.78, 0.3) > 0.4
    };
    for py in (y.max(0.0) as usize).max(s.y0)..((y + size).ceil() as usize).min(s.h) {
        for px in (x.max(0.0) as usize)..((x + size).ceil() as usize).min(s.w) {
            let hits = (0..16).filter(|i| inside(px as f32 + (i % 4) as f32 / 4.0 + 0.125, py as f32 + (i / 4) as f32 / 4.0 + 0.125)).count();
            if hits > 0 {
                let d = s.at(px, py);
                *d = crate::gfx::over(*d, scale(color, (hits * 255 / 16) as u32));
            }
        }
    }
}

/// The Rotation Lock glyph: a padlock inside a ring, in a `size` px square at (x, y).
pub fn draw_rotation_lock(s: &mut Surface, x: f32, y: f32, size: f32, color: u32) {
    let u = size / 10.0;
    s.stroke_rrect(x, y, size, size, size / 2.0, 1.0_f32.max(0.9 * u), color);
    s.stroke_rrect(x + 3.4 * u, y + 2.2 * u, 3.2 * u, 3.4 * u, 1.6 * u, 0.9 * u, color);
    s.fill_rrect(x + 2.9 * u, y + 4.6 * u, 4.2 * u, 3.1 * u, 0.7 * u, color);
}

/// Wired-link glyph: an Ethernet port (body outline and contact) in the same
/// 2:1 box as the Wi-Fi fan, so the status bar keeps its spacing.
pub fn draw_lan(s: &mut Surface, x: f32, y: f32, size: f32, color: u32) {
    let u = size / 10.0;
    s.stroke_rrect(x + 4.0 * u, y + 0.5 * u, 12.0 * u, 9.0 * u, 1.5 * u, 1.0_f32.max(0.9 * u), color);
    s.fill_rrect(x + 7.0 * u, y + 3.0 * u, 6.0 * u, 4.0 * u, 0.5 * u, color);
}

/// bifrost-style bars for an RSSI in dBm.
pub fn bars_for(rssi: i32) -> u8 {
    match rssi {
        r if r >= -60 => 3,
        r if r >= -72 => 2,
        r if r >= -84 => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_pads_minutes_not_hours() {
        assert_eq!(clock_text(9, 5), "9:05");
        assert_eq!(clock_text(21, 44), "21:44");
        assert_eq!(clock_12h(0, 5), "12:05 AM");
        assert_eq!(clock_12h(12, 0), "12:00 PM");
        assert_eq!(clock_12h(21, 44), "9:44 PM");
        assert_eq!(clock_in(21, 44, true), "21:44");
        assert_eq!(clock_in(21, 44, false), "9:44 PM");
    }

    #[test]
    fn date_like_bifrost() {
        assert_eq!(date_text(6, 3, 9), "Sat 3 Oct");
        assert_eq!(date_text(0, 31, 11), "Sun 31 Dec");
    }

    #[test]
    fn wifi_draws_inside_its_box_only() {
        let mut buf = vec![0u32; 40 * 30];
        let mut s = Surface { w: 40, h: 30, stride: 40, px: &mut buf, y0: 0 };
        draw_wifi(&mut s, 5.0, 5.0, 15.0, pack(255, 255, 255, 255));
        // the wedge tip just above the centre-bottom is lit
        assert!(buf[18 * 40 + 20] >> 24 > 200);
        // outside the 90 degree sector stays clear
        assert_eq!(buf[19 * 40 + 6], 0);
        // below the box stays clear
        assert!(buf[25 * 40..].iter().all(|&p| p == 0));
    }

    #[test]
    fn lan_glyph_draws_inside_its_box() {
        let mut buf = vec![0u32; 40 * 30];
        let mut s = Surface { w: 40, h: 30, stride: 40, px: &mut buf, y0: 0 };
        draw_lan(&mut s, 5.0, 5.0, 10.0, pack(255, 255, 255, 255));
        assert!(buf.iter().any(|&p| p >> 24 > 200), "the port is drawn");
        assert!(buf[..5 * 40].iter().all(|&p| p == 0), "nothing above the box");
        assert!(buf[16 * 40..].iter().all(|&p| p == 0), "nothing below the box");
    }

    #[test]
    fn signal_bars_dim_the_outer_arcs() {
        let outer = |bars: u8| {
            let mut buf = vec![0u32; 40 * 30];
            let mut s = Surface { w: 40, h: 30, stride: 40, px: &mut buf, y0: 0 };
            draw_wifi_bars(&mut s, 5.0, 5.0, 15.0, bars, pack(255, 255, 255, 255));
            // top of the outermost arc, straight above the centre
            (buf[(20 - 14) * 40 + 20] >> 24, buf[18 * 40 + 20] >> 24)
        };
        let (arc3, tip3) = outer(3);
        let (arc1, tip1) = outer(1);
        assert!(arc3 > 200 && tip3 > 200);
        assert!(arc1 < 120 && arc1 > 0, "unlit arcs stay faintly visible");
        assert!(tip1 > 200);
    }

    #[test]
    fn charging_adds_a_bolt() {
        let lit = |charging: bool| {
            let mut buf = vec![0u32; 80 * 40];
            let mut s = Surface { w: 80, h: 40, stride: 80, px: &mut buf, y0: 0 };
            draw_battery(&mut s, 5.0, 8.0, 2.0, 100, charging);
            buf.iter().filter(|&&p| p == pack(255, 255, 255, 255)).count()
        };
        assert!(lit(true) > 0, "white bolt over the green fill");
        assert!(lit(false) > lit(true), "full white fill when not charging");
    }

    #[test]
    fn rssi_to_bars() {
        assert_eq!(bars_for(-45), 3);
        assert_eq!(bars_for(-70), 2);
        assert_eq!(bars_for(-80), 1);
        assert_eq!(bars_for(-95), 0);
    }

    #[test]
    fn battery_fill_tracks_level() {
        let lit = |level: u8| {
            let mut buf = vec![0u32; 60 * 20];
            let mut s = Surface { w: 60, h: 20, stride: 60, px: &mut buf, y0: 0 };
            draw_battery(&mut s, 2.0, 2.0, 2.0, level, false);
            buf.iter().filter(|&&p| p == pack(255, 255, 255, 255)).count()
        };
        assert!(lit(100) > lit(50));
        assert!(lit(50) > lit(0));
    }
}
