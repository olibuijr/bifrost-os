//! Where the remote desktop sits in the window, and how window points map to remote pixels. Pure: no drawing and
//! no connection, so every rule here has a unit test.

use bifrost_rfb::Framebuffer;

/// A rectangle in window points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// The remote desktop's placement: its top-left corner in points and the scale from remote pixels to points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

/// Places a `fb_w` x `fb_h` desktop in `area`, centred. With `fit` it is scaled to the largest size that fits
/// (up or down); without, it is shown at one remote pixel per point and cropped by the area.
pub fn place(fb_w: usize, fb_h: usize, area: Area, fit: bool) -> View {
    if fb_w == 0 || fb_h == 0 || area.w <= 0.0 || area.h <= 0.0 {
        return View { x: area.x, y: area.y, scale: 1.0 };
    }
    let (fw, fh) = (fb_w as f32, fb_h as f32);
    let scale = if fit { (area.w / fw).min(area.h / fh) } else { 1.0 };
    View { x: area.x + (area.w - fw * scale) / 2.0, y: area.y + (area.h - fh * scale) / 2.0, scale }
}

/// The remote pixel under a window point, clamped to the desktop (a drag that leaves the area stays on the edge).
pub fn remote_point(v: View, fb_w: usize, fb_h: usize, px: f32, py: f32) -> (u16, u16) {
    let max_x = fb_w.saturating_sub(1) as f32;
    let max_y = fb_h.saturating_sub(1) as f32;
    let rx = ((px - v.x) / v.scale).floor().clamp(0.0, max_x);
    let ry = ((py - v.y) / v.scale).floor().clamp(0.0, max_y);
    (rx as u16, ry as u16)
}

/// The device-pixel rectangle `(x, y, w, h)` where the desktop shows inside `area`, or None when it is off screen.
/// `ox`, `oy` are the desktop's top-left corner and `ds` the scale, all in device pixels.
pub fn visible(fb_w: usize, fb_h: usize, ox: f32, oy: f32, ds: f32, area: Area) -> Option<(i32, i32, usize, usize)> {
    let x0 = (area.x).max(ox).ceil();
    let y0 = (area.y).max(oy).ceil();
    let x1 = (area.x + area.w).min(ox + fb_w as f32 * ds).floor();
    let y1 = (area.y + area.h).min(oy + fb_h as f32 * ds).floor();
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 as i32, y0 as i32, (x1 - x0) as usize, (y1 - y0) as usize))
}

/// Samples the framebuffer for a device-pixel rectangle `(x0, y0, w, h)`, nearest neighbour, as 0x00RRGGBB rows.
/// The desktop's top-left is (`ox`, `oy`) in device pixels and one remote pixel is `ds` device pixels.
pub fn sample(fb: &Framebuffer, ox: f32, oy: f32, ds: f32, (x0, y0, w, h): (i32, i32, usize, usize)) -> Vec<u32> {
    let mut out = Vec::with_capacity(w * h);
    if fb.w == 0 || fb.h == 0 || ds <= 0.0 {
        out.resize(w * h, 0);
        return out;
    }
    let max_x = (fb.w - 1) as f32;
    let max_y = (fb.h - 1) as f32;
    for dy in 0..h {
        let sy = (((y0 + dy as i32) as f32 + 0.5 - oy) / ds).floor().clamp(0.0, max_y) as usize;
        for dx in 0..w {
            let sx = (((x0 + dx as i32) as f32 + 0.5 - ox) / ds).floor().clamp(0.0, max_x) as usize;
            out.push(fb.px[sy * fb.w + sx]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Area = Area { x: 16.0, y: 240.0, w: 800.0, h: 400.0 };

    #[test]
    fn fit_scales_and_centres_within_the_area() {
        // 1600x800 desktop in an 800x400 area: half scale, filling it exactly.
        let v = place(1600, 800, AREA, true);
        assert_eq!(v.scale, 0.5);
        assert_eq!((v.x, v.y), (16.0, 240.0));
        // A 400x400 desktop is scaled up to the height and centred horizontally.
        let v = place(400, 400, AREA, true);
        assert_eq!(v.scale, 1.0);
        assert_eq!(v.x, 16.0 + (800.0 - 400.0) / 2.0);
    }

    #[test]
    fn actual_size_keeps_one_pixel_per_point() {
        let v = place(1600, 800, AREA, false);
        assert_eq!(v.scale, 1.0);
        // Larger than the area, so it is centred and cropped on both sides.
        assert_eq!(v.x, 16.0 + (800.0 - 1600.0) / 2.0);
    }

    #[test]
    fn pointer_maps_to_remote_pixels_and_clamps() {
        let v = View { x: 16.0, y: 240.0, scale: 0.5 };
        assert_eq!(remote_point(v, 1600, 800, 16.0 + 10.0, 240.0 + 20.0), (20, 40));
        // Left of and above the desktop clamps to 0, right of it clamps to the last pixel.
        assert_eq!(remote_point(v, 1600, 800, 0.0, 0.0), (0, 0));
        assert_eq!(remote_point(v, 1600, 800, 10_000.0, 10_000.0), (1599, 799));
    }

    #[test]
    fn visible_is_the_overlap_with_the_area() {
        let area = Area { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        assert_eq!(visible(50, 50, 0.0, 0.0, 1.0, area), Some((0, 0, 50, 50)));
        assert_eq!(visible(200, 200, -50.0, -50.0, 1.0, area), Some((0, 0, 100, 100)));
        assert_eq!(visible(10, 10, 200.0, 0.0, 1.0, area), None);
    }

    #[test]
    fn sample_is_nearest_neighbour() {
        let mut fb = Framebuffer::new(2, 1);
        fb.px = vec![0x111111, 0x222222];
        // Each remote pixel covers two device pixels.
        let out = sample(&fb, 0.0, 0.0, 2.0, (0, 0, 4, 1));
        assert_eq!(out, vec![0x111111, 0x111111, 0x222222, 0x222222]);
        // A window starting in the middle of the desktop samples from there.
        assert_eq!(sample(&fb, 0.0, 0.0, 2.0, (2, 0, 2, 1)), vec![0x222222, 0x222222]);
    }

    #[test]
    fn empty_framebuffer_samples_black() {
        let fb = Framebuffer::new(0, 0);
        assert_eq!(sample(&fb, 0.0, 0.0, 1.0, (0, 0, 2, 2)), vec![0; 4]);
    }
}
