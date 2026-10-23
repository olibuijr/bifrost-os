//! Screen Zoom for the whole output (`bifrost_window_level_v1` `set_zoom`, version 6): the shell sends the
//! magnification (Settings > Accessibility > Zoom, Option+Cmd+8 / = / -) and the window server scales everything it
//! composes, the wallpaper, every window, the menu bar, the Dock and the pointer, around a viewport that follows the
//! pointer. Input stays in desktop coordinates: the point under the drawn pointer is the point the pointer is at, so
//! clicks land where they are seen.
//!
//! The viewport is the shell's Full Screen model (`accessibility/zoom.rs` `source_rect`): the output shrunk by the
//! factor, centred on the pointer and kept on the output. Each element is rescaled about one origin (smithay's
//! `RescaleRenderElement`), the point that the scale leaves in place, so that the viewport fills the output.

/// The wire value of 1.0 (no zoom).
pub const ONE: u32 = 256;
/// The largest factor taken, as the shell's `MAX_FACTOR` (8x).
pub const MAX: u32 = 8 * ONE;

/// The factor a `set_zoom` wire value asks for: 256 is 1.0, values below are 1.0 and above 8x are 8x.
pub fn factor(wire: u32) -> f64 {
    f64::from(wire.clamp(ONE, MAX)) / f64::from(ONE)
}

/// The viewport's top-left corner on the desktop for `zoom` with the pointer at `pointer` on an output of `size` (both
/// in the same pixels), or None at 1.0.
pub fn viewport(zoom: f64, pointer: (f64, f64), size: (f64, f64)) -> Option<(f64, f64)> {
    if zoom <= 1.0 + 1e-6 {
        return None;
    }
    let axis = |p: f64, s: f64| {
        let w = s / zoom;
        (p - w / 2.0).max(0.0).min((s - w).max(0.0))
    };
    Some((axis(pointer.0, size.0), axis(pointer.1, size.1)))
}

/// The point the rescale keeps in place so the viewport at `corner` fills the output: a desktop point `x` is shown at
/// `(x - corner) * zoom`, which is `origin + (x - origin) * zoom` for `origin = corner * zoom / (zoom - 1)`.
pub fn origin(zoom: f64, pointer: (f64, f64), size: (f64, f64)) -> Option<(i32, i32)> {
    let (cx, cy) = viewport(zoom, pointer, size)?;
    let k = zoom / (zoom - 1.0);
    Some(((cx * k).round() as i32, (cy * k).round() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);

    /// Where `origin` puts desktop point `x` on the output (one axis).
    fn shown(o: i32, zoom: f64, x: f64) -> f64 {
        o as f64 + (x - o as f64) * zoom
    }

    #[test]
    fn the_wire_value_is_clamped_to_one_through_eight() {
        assert_eq!(factor(0), 1.0);
        assert_eq!(factor(ONE), 1.0);
        assert_eq!(factor(512), 2.0);
        assert_eq!(factor(u32::MAX), 8.0);
    }

    #[test]
    fn no_zoom_has_no_transform() {
        assert_eq!(origin(1.0, (500.0, 500.0), SIZE), None);
        assert_eq!(viewport(factor(ONE), (0.0, 0.0), SIZE), None);
    }

    #[test]
    fn the_viewport_follows_the_pointer_and_fills_the_output() {
        let z = 2.0;
        // pointer in the middle: the middle half of the desktop fills the output, the pointer stays in the middle
        let (ox, oy) = origin(z, (960.0, 540.0), SIZE).unwrap();
        assert!((shown(ox, z, 480.0) - 0.0).abs() < 1.0 && (shown(ox, z, 1440.0) - 1920.0).abs() < 1.0);
        assert!((shown(oy, z, 270.0) - 0.0).abs() < 1.0 && (shown(oy, z, 810.0) - 1080.0).abs() < 1.0);
        assert!((shown(ox, z, 960.0) - 960.0).abs() < 1.0, "the pointer is drawn where it is");
        // at the top-left corner the corner stays put; at the bottom-right the far edge does
        assert_eq!(origin(z, (0.0, 0.0), SIZE), Some((0, 0)));
        let (ox, _) = origin(z, (1920.0, 1080.0), SIZE).unwrap();
        assert!((shown(ox, z, 1920.0) - 1920.0).abs() < 1.0);
        assert!((shown(ox, z, 960.0) - 0.0).abs() < 1.0);
    }

    #[test]
    fn a_centred_viewport_draws_the_pointer_mid_screen() {
        let z = 3.0;
        let p = (700.0, 400.0);
        let (ox, oy) = origin(z, p, SIZE).unwrap();
        let (sx, sy) = (shown(ox, z, p.0), shown(oy, z, p.1));
        assert!((sx - 960.0).abs() < 2.0 && (sy - 540.0).abs() < 2.0);
    }
}
