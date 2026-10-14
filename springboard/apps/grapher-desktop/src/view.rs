//! The window onto the plane: the world point at the centre of the plot and the scale in device pixels per unit.
//! Both axes share one scale, so a circle is drawn round. Pixel coordinates have their origin at the top left of the
//! plot, with y growing downwards.

/// The scale of a new view, in device pixels per unit.
pub const DEFAULT_PPU: f64 = 40.0;
/// The closest and the farthest zoom, in device pixels per unit.
pub const MIN_PPU: f64 = 0.5;
pub const MAX_PPU: f64 = 1.0e6;
/// The number of grid lines a plot aims for along an axis; the grid step is the nice step for that many lines.
const TARGET_TICKS: f64 = 8.0;
/// A bound on the number of ticks, so a degenerate range cannot make a long list.
const MAX_TICKS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The world coordinates of the centre of the plot.
    pub cx: f64,
    pub cy: f64,
    /// Device pixels per world unit, on both axes.
    pub ppu: f64,
}

impl Default for View {
    fn default() -> Self {
        View { cx: 0.0, cy: 0.0, ppu: DEFAULT_PPU }
    }
}

impl View {
    /// The pixel position of the world point `(x, y)` in a plot of `w` by `h` pixels.
    pub fn to_px(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
        (w / 2.0 + (x - self.cx) * self.ppu, h / 2.0 - (y - self.cy) * self.ppu)
    }

    /// The world point under the pixel `(px, py)` of a plot of `w` by `h` pixels.
    pub fn to_world(&self, px: f64, py: f64, w: f64, h: f64) -> (f64, f64) {
        (self.cx + (px - w / 2.0) / self.ppu, self.cy - (py - h / 2.0) / self.ppu)
    }

    /// Moves the view by a drag of `(dx, dy)` device pixels, so the content follows the pointer.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.cx -= dx / self.ppu;
        self.cy += dy / self.ppu;
    }

    /// Multiplies the scale by `factor` (clamped to the zoom range), keeping the world point under the pixel
    /// `(px, py)` where it was.
    pub fn zoom_at(&mut self, factor: f64, px: f64, py: f64, w: f64, h: f64) {
        let (x, y) = self.to_world(px, py, w, h);
        self.ppu = (self.ppu * factor).clamp(MIN_PPU, MAX_PPU);
        let (nx, ny) = self.to_world(px, py, w, h);
        self.cx += x - nx;
        self.cy += y - ny;
    }

    /// The world range visible horizontally in a plot `w` pixels wide.
    pub fn x_range(&self, w: f64) -> (f64, f64) {
        let half = w / 2.0 / self.ppu;
        (self.cx - half, self.cx + half)
    }

    /// The world range visible vertically in a plot `h` pixels high.
    pub fn y_range(&self, h: f64) -> (f64, f64) {
        let half = h / 2.0 / self.ppu;
        (self.cy - half, self.cy + half)
    }

    /// The grid step for an axis `pixels` long: about `TARGET_TICKS` steps across it.
    pub fn grid_step(&self, pixels: f64) -> f64 {
        nice_step(pixels / self.ppu / TARGET_TICKS)
    }
}

/// The smallest step of 1, 2 or 5 times a power of ten that is at least `raw`.
pub fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    for multiple in [1.0, 2.0, 5.0, 10.0] {
        if multiple * magnitude >= raw * (1.0 - 1e-12) {
            return multiple * magnitude;
        }
    }
    10.0 * magnitude
}

/// The multiples of `step` inside `[min, max]`, in order.
pub fn ticks(min: f64, max: f64, step: f64) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || !step.is_finite() || step <= 0.0 || max < min {
        return Vec::new();
    }
    let first = (min / step).ceil() as i64;
    let last = (max / step).floor() as i64;
    (first..=last).take(MAX_TICKS).map(|k| k as f64 * step).collect()
}

/// A tick value as text, with as many decimals as the step needs.
pub fn tick_label(value: f64, step: f64) -> String {
    if value.abs() < step * 1e-6 {
        return "0".to_string();
    }
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    format!("{value:.decimals$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn pixels_and_world_points_are_inverse() {
        let view = View { cx: 2.0, cy: -3.0, ppu: 25.0 };
        let (w, h) = (640.0, 480.0);
        let (px, py) = view.to_px(4.5, 1.25, w, h);
        let (x, y) = view.to_world(px, py, w, h);
        assert!(close(x, 4.5) && close(y, 1.25));
    }

    #[test]
    fn the_centre_of_the_view_is_the_centre_of_the_plot() {
        let view = View { cx: 1.0, cy: 2.0, ppu: 10.0 };
        assert_eq!(view.to_px(1.0, 2.0, 200.0, 100.0), (100.0, 50.0));
        assert!(close(view.to_world(0.0, 0.0, 200.0, 100.0).0, 1.0 - 10.0));
    }

    #[test]
    fn y_grows_upwards() {
        let view = View::default();
        let (_, top) = view.to_px(0.0, 1.0, 100.0, 100.0);
        let (_, bottom) = view.to_px(0.0, -1.0, 100.0, 100.0);
        assert!(top < bottom);
    }

    #[test]
    fn panning_moves_the_content_with_the_pointer() {
        let mut view = View { cx: 0.0, cy: 0.0, ppu: 20.0 };
        let (w, h) = (400.0, 300.0);
        let before = view.to_world(100.0, 100.0, w, h);
        view.pan(40.0, -20.0);
        // the point that was under the pixel (100, 100) is now under (140, 80)
        let after = view.to_world(140.0, 80.0, w, h);
        assert!(close(before.0, after.0) && close(before.1, after.1));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let mut view = View { cx: 3.0, cy: -1.0, ppu: 30.0 };
        let (w, h) = (500.0, 400.0);
        let under = view.to_world(120.0, 300.0, w, h);
        view.zoom_at(2.0, 120.0, 300.0, w, h);
        assert!(close(view.ppu, 60.0));
        let now = view.to_world(120.0, 300.0, w, h);
        assert!(close(under.0, now.0) && close(under.1, now.1));
    }

    #[test]
    fn zoom_is_clamped() {
        let mut view = View::default();
        for _ in 0..200 {
            view.zoom_at(0.5, 0.0, 0.0, 100.0, 100.0);
        }
        assert_eq!(view.ppu, MIN_PPU);
        for _ in 0..400 {
            view.zoom_at(2.0, 0.0, 0.0, 100.0, 100.0);
        }
        assert_eq!(view.ppu, MAX_PPU);
    }

    #[test]
    fn visible_ranges_are_centred_on_the_view() {
        let view = View { cx: 5.0, cy: 0.0, ppu: 10.0 };
        assert_eq!(view.x_range(200.0), (-5.0, 15.0));
        assert_eq!(view.y_range(100.0), (-5.0, 5.0));
    }

    #[test]
    fn nice_steps_are_one_two_or_five_times_a_power_of_ten() {
        assert!(close(nice_step(1.0), 1.0));
        assert!(close(nice_step(1.25), 2.0));
        assert!(close(nice_step(3.0), 5.0));
        assert!(close(nice_step(12.0), 20.0));
        assert!((nice_step(0.07) - 0.1).abs() < 1e-12);
        assert!(close(nice_step(0.0), 1.0));
        assert!(close(nice_step(f64::NAN), 1.0));
    }

    #[test]
    fn ticks_cover_the_range_inclusively() {
        assert_eq!(ticks(-1.0, 1.0, 0.5), vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(ticks(0.2, 0.9, 0.5), vec![0.5]);
        assert!(ticks(2.0, 1.0, 1.0).is_empty());
        assert!(ticks(0.0, 1.0, 0.0).is_empty());
        assert!(ticks(0.0, 1e300, 1.0).len() <= MAX_TICKS);
    }

    #[test]
    fn tick_labels_match_the_step() {
        assert_eq!(tick_label(0.1 * 3.0, 0.1), "0.3");
        assert_eq!(tick_label(2.0, 2.0), "2");
        assert_eq!(tick_label(-0.5, 0.5), "-0.5");
        assert_eq!(tick_label(0.0, 0.5), "0");
        assert_eq!(tick_label(-1e-15, 0.5), "0");
    }
}
