//! Displays for apps (`NativeScreen`): the frame and visible frame of each screen in global points, the backing scale
//! that maps points to device pixels, and which screen holds a point or a window. The shell reports the screens; this
//! module only does the geometry.
use crate::Rect;

/// How a rectangle is snapped to whole device pixels (`NativeAlignmentOptions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Grow to the pixels that cover the rectangle.
    Outward,
    /// Shrink to the pixels that lie entirely inside the rectangle.
    Inward,
    /// Round each edge to the nearest pixel.
    Nearest,
}

/// One display. `frame` and `visible_frame` are in global points; the visible frame excludes the menu bar and Dock.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub frame: Rect,
    pub visible_frame: Rect,
    /// Device pixels per point (`backingScaleFactor`). Zero or negative values are read as 1.
    pub backing_scale: f32,
    /// Bits per pixel (`depth`), used to choose the screen that represents colour best.
    pub bits_per_pixel: u32,
}

impl Screen {
    pub fn new(name: &str, frame: Rect, backing_scale: f32) -> Self {
        Screen {
            name: name.into(),
            frame,
            visible_frame: frame,
            backing_scale: if backing_scale > 0.0 { backing_scale } else { 1.0 },
            bits_per_pixel: 32,
        }
    }

    /// Sets the visible frame, clipped to the screen frame. A visible frame that misses the screen is empty.
    pub fn with_visible_frame(mut self, visible: Rect) -> Self {
        self.visible_frame = visible.intersection(self.frame).unwrap_or(Rect { x: self.frame.x, y: self.frame.y, w: 0.0, h: 0.0 });
        self
    }

    pub fn with_bits_per_pixel(mut self, bits: u32) -> Self {
        self.bits_per_pixel = bits;
        self
    }

    /// Converts a rectangle in global points to device pixels relative to this screen's origin
    /// (`convertRectToBacking`).
    pub fn to_backing(&self, r: Rect) -> Rect {
        let s = self.backing_scale;
        Rect { x: (r.x - self.frame.x) * s, y: (r.y - self.frame.y) * s, w: r.w * s, h: r.h * s }
    }

    /// The inverse of [`Screen::to_backing`] (`convertRectFromBacking`).
    pub fn from_backing(&self, r: Rect) -> Rect {
        let s = self.backing_scale;
        Rect { x: r.x / s + self.frame.x, y: r.y / s + self.frame.y, w: r.w / s, h: r.h / s }
    }

    /// Snaps a rectangle in global points to the device pixel grid (`backingAlignedRect`). The result is in points.
    pub fn backing_aligned(&self, r: Rect, align: Align) -> Rect {
        let p = self.to_backing(r);
        let (x0, x1) = edges(p.x, p.x + p.w, align);
        let (y0, y1) = edges(p.y, p.y + p.h, align);
        self.from_backing(Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) })
    }
}

/// Snaps the two edges of one axis, in pixels. An inward snap that crosses over collapses to a zero-length edge.
fn edges(lo: f32, hi: f32, align: Align) -> (f32, f32) {
    match align {
        Align::Outward => (lo.floor(), hi.ceil()),
        Align::Inward => {
            let (a, b) = (lo.ceil(), hi.floor());
            if a <= b { (a, b) } else { (a, a) }
        }
        Align::Nearest => (lo.round(), hi.round()),
    }
}

/// The displays the app sees (`NativeScreen.screens`), with the main screen chosen by the shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screens {
    screens: Vec<Screen>,
    main: usize,
}

impl Screens {
    pub fn new(screens: Vec<Screen>) -> Self {
        Screens { screens, main: 0 }
    }

    /// Replaces the list (a display was plugged in or rearranged). The main index is kept when it still exists.
    pub fn update(&mut self, screens: Vec<Screen>) {
        self.screens = screens;
        if self.main >= self.screens.len() {
            self.main = 0;
        }
    }

    pub fn all(&self) -> &[Screen] {
        &self.screens
    }

    /// Selects the main screen (`NativeScreen.main`, the one with keyboard focus). False when the index is out of range.
    pub fn set_main(&mut self, index: usize) -> bool {
        if index < self.screens.len() {
            self.main = index;
            true
        } else {
            false
        }
    }

    pub fn main(&self) -> Option<&Screen> {
        self.screens.get(self.main)
    }

    /// The first screen with the most bits per pixel (`NativeScreen.deepest`).
    pub fn deepest(&self) -> Option<&Screen> {
        let mut best: Option<&Screen> = None;
        for s in &self.screens {
            if best.map_or(true, |b| s.bits_per_pixel > b.bits_per_pixel) {
                best = Some(s);
            }
        }
        best
    }

    /// The screen whose frame holds the point. Frames are half-open, so a point on a shared edge belongs to the
    /// screen to its right or below.
    pub fn containing(&self, x: f32, y: f32) -> Option<&Screen> {
        self.screens.iter().find(|s| s.frame.contains(x, y))
    }

    /// The screen that shows most of a rectangle, such as a window's screen. Ties go to the earlier screen.
    /// None when the rectangle is on no screen.
    pub fn containing_rect(&self, r: Rect) -> Option<&Screen> {
        let mut best: Option<(&Screen, f32)> = None;
        for s in &self.screens {
            let area = s.frame.intersection(r).map_or(0.0, |i| i.w * i.h);
            if area > 0.0 && best.map_or(true, |(_, a)| area > a) {
                best = Some((s, area));
            }
        }
        best.map(|(s, _)| s)
    }

    /// The bounding frame of every screen, in global points (the desktop as one rectangle).
    pub fn union_frame(&self) -> Option<Rect> {
        self.screens.iter().map(|s| s.frame).reduce(|a, b| a.union(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn backing_conversion_is_relative_to_the_screen_origin_and_round_trips() {
        let s = Screen::new("Built-in", rect(-1440.0, 100.0, 1440.0, 900.0), 2.0);
        let p = rect(-1000.0, 150.0, 10.5, 20.0);
        let px = s.to_backing(p);
        assert_eq!(px, rect(880.0, 100.0, 21.0, 40.0));
        assert_eq!(s.from_backing(px), p);
    }

    #[test]
    fn non_positive_scale_reads_as_one_point_per_pixel() {
        let s = Screen::new("Plain", rect(0.0, 0.0, 800.0, 600.0), 0.0);
        assert_eq!(s.backing_scale, 1.0);
        assert_eq!(s.to_backing(rect(5.0, 5.0, 3.0, 3.0)), rect(5.0, 5.0, 3.0, 3.0));
    }

    #[test]
    fn pixel_alignment_outward_inward_and_nearest() {
        let s = Screen::new("Retina", rect(0.0, 0.0, 1000.0, 800.0), 2.0);
        // in pixels this is x 1.6..7.8 and y 2.4..4.6
        let r = rect(0.8, 1.2, 3.1, 1.1);
        assert_eq!(s.backing_aligned(r, Align::Outward), rect(0.5, 1.0, 3.5, 1.5));
        assert_eq!(s.backing_aligned(r, Align::Inward), rect(1.0, 1.5, 2.5, 0.5));
        assert_eq!(s.backing_aligned(r, Align::Nearest), rect(1.0, 1.0, 3.0, 1.5));
    }

    #[test]
    fn inward_alignment_of_a_sub_pixel_rectangle_collapses_to_zero() {
        let s = Screen::new("Plain", rect(0.0, 0.0, 100.0, 100.0), 1.0);
        let out = s.backing_aligned(rect(10.2, 10.0, 0.4, 5.0), Align::Inward);
        assert_eq!(out.w, 0.0);
    }

    #[test]
    fn visible_frame_is_clipped_to_the_screen() {
        let s = Screen::new("Panel", rect(0.0, 0.0, 1000.0, 800.0), 1.0).with_visible_frame(rect(0.0, 24.0, 1200.0, 776.0));
        assert_eq!(s.visible_frame, rect(0.0, 24.0, 1000.0, 776.0));
    }

    #[test]
    fn point_lookup_uses_half_open_frames() {
        let screens = Screens::new(vec![
            Screen::new("Left", rect(0.0, 0.0, 1000.0, 800.0), 1.0),
            Screen::new("Right", rect(1000.0, 0.0, 1000.0, 800.0), 2.0),
        ]);
        assert_eq!(screens.containing(999.9, 10.0).map(|s| s.name.as_str()), Some("Left"));
        assert_eq!(screens.containing(1000.0, 10.0).map(|s| s.name.as_str()), Some("Right"));
        assert!(screens.containing(2000.0, 10.0).is_none());
    }

    #[test]
    fn window_screen_is_the_one_with_the_largest_overlap() {
        let screens = Screens::new(vec![
            Screen::new("Left", rect(0.0, 0.0, 1000.0, 800.0), 1.0),
            Screen::new("Right", rect(1000.0, 0.0, 1000.0, 800.0), 1.0),
        ]);
        // 300 points on the left, 700 on the right
        let window = rect(700.0, 100.0, 1000.0, 400.0);
        assert_eq!(screens.containing_rect(window).map(|s| s.name.as_str()), Some("Right"));
        assert!(screens.containing_rect(rect(5000.0, 0.0, 10.0, 10.0)).is_none());
    }

    #[test]
    fn deepest_prefers_more_bits_and_the_first_on_a_tie() {
        let screens = Screens::new(vec![
            Screen::new("A", rect(0.0, 0.0, 10.0, 10.0), 1.0).with_bits_per_pixel(24),
            Screen::new("B", rect(10.0, 0.0, 10.0, 10.0), 1.0).with_bits_per_pixel(30),
            Screen::new("C", rect(20.0, 0.0, 10.0, 10.0), 1.0).with_bits_per_pixel(30),
        ]);
        assert_eq!(screens.deepest().map(|s| s.name.as_str()), Some("B"));
    }

    #[test]
    fn main_screen_is_kept_across_updates_until_it_disappears() {
        let mut screens = Screens::new(vec![Screen::new("A", rect(0.0, 0.0, 10.0, 10.0), 1.0), Screen::new("B", rect(10.0, 0.0, 10.0, 10.0), 1.0)]);
        assert!(screens.set_main(1));
        assert!(!screens.set_main(2));
        assert_eq!(screens.main().map(|s| s.name.as_str()), Some("B"));
        screens.update(vec![Screen::new("A", rect(0.0, 0.0, 10.0, 10.0), 1.0), Screen::new("B", rect(10.0, 0.0, 10.0, 10.0), 1.0)]);
        assert_eq!(screens.main().map(|s| s.name.as_str()), Some("B"));
        screens.update(vec![Screen::new("A", rect(0.0, 0.0, 10.0, 10.0), 1.0)]);
        assert_eq!(screens.main().map(|s| s.name.as_str()), Some("A"));
    }

    #[test]
    fn union_frame_spans_displays_with_negative_origins() {
        let screens = Screens::new(vec![
            Screen::new("Left", rect(-1920.0, 0.0, 1920.0, 1080.0), 1.0),
            Screen::new("Right", rect(0.0, -200.0, 2560.0, 1440.0), 1.0),
        ]);
        assert_eq!(screens.union_frame(), Some(rect(-1920.0, -200.0, 4480.0, 1440.0)));
        assert_eq!(Screens::default().union_frame(), None);
    }
}
