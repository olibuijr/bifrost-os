//! The Color Meter window's state and geometry: the toolbar, the magnifier on the left and the readout on the right.
//! Layout is in points and derives from the window size; the drawing and the hit tests read the same rectangles, so a
//! tap lands on what is drawn. The state reads pixels only through `ScreenSource`.

use crate::color::{self, convert, copy_text, CopyFormat, Reading, Rgb8, Space};
use crate::loupe::{self, Pointer};
use crate::source::{Ask, PixelRect, ScreenSource};
use bifrost_core::json::Json;
use bifrost_app::controls::values::Slider;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::proto::AxAction;
use bifrost_app::widgets;

/// The outer padding (points).
pub const PAD_PT: f32 = 20.0;
/// One readout row (points).
pub const LINE_PT: f32 = 28.0;
/// The readout's colour swatch (points).
pub const SWATCH_PT: f32 = 120.0;
/// The readout's width when the window is wide enough (points).
pub const PANEL_PT: f32 = 320.0;

pub const B_LOCK: u32 = 1;
pub const B_COPY: u32 = 2;
pub const B_SPACE: u32 = 3;
pub const B_FORMAT: u32 = 4;
pub const B_SWATCH: u32 = 5;
pub const B_ZOOM: u32 = 6;
/// The accessibility id of the aperture slider.
pub const CTL_APERTURE: u32 = 7;

/// The loupe sizes the Zoom button and the Magnification menu go through, in cells across (fewer cells is a stronger
/// magnification).
pub const ZOOMS: [u32; 6] = [7, 11, 15, 21, 31, 41];
/// The loupe size a new window starts with.
pub const DEFAULT_CELLS: u32 = 15;
/// The aperture sizes, in display pixels across. The reading is the mean of the square this wide around the pointer.
/// Sizes are odd, so the square has a centre pixel and the pointer pixel is its middle.
pub const APERTURE_MIN: u32 = 1;
pub const APERTURE_MAX: u32 = 31;

/// Rounds an aperture size up to the odd size it stands for, within the limits: an even size would centre on no pixel.
pub fn odd_aperture(size: u32) -> u32 {
    size.clamp(APERTURE_MIN, APERTURE_MAX) | 1
}
/// The aperture row under the swatch: its height, and the height and side margin of the slider track in it (points).
pub const APERTURE_PT: f32 = 56.0;
pub const TRACK_PT: f32 = 24.0;
pub const TRACK_PAD_PT: f32 = 16.0;

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    Loupe,
    /// The aperture slider's track: a press or a drag sets the size.
    Aperture,
}

/// What an action asks the app shell to do besides redrawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Redraw,
    /// Put this text on the clipboard.
    CopyText(String),
    /// Put a swatch of this colour on the clipboard as an image.
    CopySwatch(Rgb8),
}

pub struct ColorMeter {
    pub source: Box<dyn ScreenSource + Send>,
    pub pointer: Pointer,
    pub cells: u32,
    /// The side of the square of pixels the reading averages, centred on the pointer pixel.
    pub aperture: u32,
    /// Whether the loupe is sampled on every refresh. Off, it is sampled only when the readout's pixel or the loupe size
    /// changed (the pointer is still polled, to notice it move).
    pub continuous: bool,
    /// Whether the readout lists the pointer position.
    pub show_position: bool,
    /// The readout pixel and loupe size the last loupe request was sent for, to skip repeats while not continuous.
    pub sampled: Option<(u32, u32, u32)>,
    pub space: Space,
    pub format: CopyFormat,
    /// The last thing copied, shown in the readout.
    pub copied: Option<String>,
    /// The window size, in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target a press started on; the release acts only when it lands on the same one.
    pub press: Target,
}

impl ColorMeter {
    pub fn new(source: Box<dyn ScreenSource + Send>) -> ColorMeter {
        ColorMeter {
            source,
            pointer: Pointer::default(),
            cells: DEFAULT_CELLS,
            aperture: APERTURE_MIN,
            continuous: true,
            show_position: true,
            sampled: None,
            space: Space::Srgb,
            format: CopyFormat::Hex,
            copied: None,
            size: (1000.0, 640.0),
            labels: TextCache::default(),
            press: Target::Nothing,
        }
    }

    /// Takes the pointer position from the source, unless the readout is locked. True when the readout moved.
    pub fn refresh(&mut self) -> bool {
        match self.source.pointer() {
            Some((x, y)) => self.pointer.follow(x, y),
            None => false,
        }
    }

    /// The requests the source needs sent now, around the readout's pixel.
    ///
    /// The square asked for covers the aperture as well as the loupe. While not updating continuously, the loupe is asked
    /// again only when the readout's pixel or the loupe size changed since the last request.
    pub fn wants(&mut self) -> Vec<Ask> {
        let side = self.cells.max(self.aperture);
        let mut asks = self.source.wants((self.pointer.x, self.pointer.y), side);
        if !self.continuous {
            let key = (self.pointer.x, self.pointer.y, side);
            if self.sampled == Some(key) {
                asks.retain(|a| !matches!(a, Ask::Loupe(_)));
            } else if asks.iter().any(|a| matches!(a, Ask::Loupe(_))) {
                self.sampled = Some(key);
            }
        }
        asks
    }

    /// Turns continuous updating on or off. Either way the next refresh takes a fresh sample.
    pub fn set_continuous(&mut self, on: bool) {
        self.continuous = on;
        self.sampled = None;
    }

    /// Records the request id the shell gave `ask`.
    pub fn track(&mut self, request: u32, ask: Ask) {
        self.source.track(request, ask);
    }

    /// Takes the shell's reply to `request` when it was one of ours. True when a frame is needed.
    pub fn answer(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        self.source.answer(request, result)
    }

    /// The display size, when the source can read one.
    pub fn display(&self) -> Option<(u32, u32)> {
        self.source.display_size()
    }

    /// The pixel the readout describes, in each space.
    pub fn reading(&self) -> Option<Reading> {
        let (w, h) = self.display()?;
        if loupe::pixel_at(self.pointer.x as f32, self.pointer.y as f32, w, h).is_none() {
            return None;
        }
        let rect = self.aperture_rect()?;
        let px = color::average(&self.source.read(rect)?)?;
        Some(convert(px, self.space))
    }

    /// The display pixels the reading averages: the aperture square around the readout's pixel.
    pub fn aperture_rect(&self) -> Option<PixelRect> {
        let (w, h) = self.display()?;
        Some(loupe::sample_rect(self.pointer.x, self.pointer.y, self.aperture, w, h))
    }

    /// Sets the aperture size, kept within its limits and odd. True when it changed.
    pub fn set_aperture(&mut self, size: u32) -> bool {
        let size = odd_aperture(size);
        let changed = size != self.aperture;
        self.aperture = size;
        changed
    }

    /// The aperture slider with the current size.
    fn aperture_slider(&self) -> Slider {
        Slider::new(APERTURE_MIN as f32, APERTURE_MAX as f32, self.aperture as f32, Some(1.0))
    }

    /// Sets the aperture from a pointer at `x` (points) on the slider's track. True when it changed.
    pub fn drag_aperture(&mut self, x: f32) -> bool {
        let mut slider = self.aperture_slider();
        slider.drag(self.track_rect(), x);
        self.set_aperture(slider.value().round() as u32)
    }

    /// Where the knob sits on the track as a 0 to 1 fraction.
    pub fn aperture_fraction(&self) -> f32 {
        self.aperture_slider().fraction()
    }

    /// An assistive technology steps or sets the aperture. True when it changed.
    pub fn aperture_action(&mut self, action: &AxAction) -> bool {
        match action {
            AxAction::Increment => self.set_aperture(self.aperture.saturating_add(2)),
            AxAction::Decrement => self.set_aperture(self.aperture.saturating_sub(2)),
            AxAction::SetValue(text) => text.trim().parse::<f32>().map_or(false, |v| self.set_aperture(v.round().max(0.0) as u32)),
            _ => false,
        }
    }

    /// The loupe's sample square and its pixels, row by row.
    pub fn loupe_pixels(&self) -> Option<(PixelRect, Vec<Rgb8>)> {
        let (w, h) = self.display()?;
        let rect = loupe::sample_rect(self.pointer.x, self.pointer.y, self.cells, w, h);
        let px = self.source.read(rect)?;
        Some((rect, px))
    }

    /// The readout rows, label and value.
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![("Colour space".to_string(), self.space.name().to_string())];
        match self.reading() {
            Some(r) => {
                rows.push(("Hex".into(), color::hex(r.codes)));
                rows.push(("Decimal".into(), color::decimal(r.codes)));
                rows.push(("16-bit".into(), color::bit16(r.values)));
                rows.push(("Percent".into(), color::percent(r.values)));
            }
            None => rows.push(("Value".into(), "Not available".into())),
        }
        rows.push(("Copy as".into(), self.format.name().to_string()));
        if self.show_position {
            rows.push(("Position".into(), format!("{}, {}", self.pointer.x, self.pointer.y)));
        }
        rows.push(("Pointer".into(), self.pointer_state().into()));
        if let Some(text) = &self.copied {
            rows.push(("Copied".into(), text.clone()));
        }
        rows
    }

    /// How the pointer is followed: "Locked", "Following", or the axis that is locked.
    pub fn pointer_state(&self) -> &'static str {
        match (self.pointer.lock_x, self.pointer.lock_y) {
            (true, true) => "Locked",
            (true, false) => "X locked",
            (false, true) => "Y locked",
            (false, false) => "Following",
        }
    }

    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(B_LOCK, if self.pointer.locked() { "Unlock" } else { "Lock" }),
            ToolbarItem::button(B_COPY, "Copy"),
            ToolbarItem::button(B_FORMAT, self.format.name()),
            ToolbarItem::button(B_SWATCH, "Copy Swatch"),
            ToolbarItem::button(B_SPACE, self.space.name()),
            ToolbarItem::button(B_ZOOM, "Zoom"),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    /// The square magnifier: as large as the window allows, on the left.
    pub fn loupe_rect(&self) -> Rect {
        let avail_h = (self.size.1 - UNIFIED_BAR_PT - 2.0 * PAD_PT).max(0.0);
        let avail_w = (self.size.0 - PANEL_PT - 3.0 * PAD_PT).max(0.0);
        let side = avail_h.min(avail_w);
        Rect { x: PAD_PT, y: UNIFIED_BAR_PT + PAD_PT, w: side, h: side }
    }

    /// The readout column, on the right of the magnifier.
    pub fn panel_rect(&self) -> Rect {
        let x = self.loupe_rect().x + self.loupe_rect().w + PAD_PT;
        Rect { x, y: UNIFIED_BAR_PT + PAD_PT, w: (self.size.0 - x - PAD_PT).max(0.0), h: (self.size.1 - UNIFIED_BAR_PT - 2.0 * PAD_PT).max(0.0) }
    }

    /// The swatch at the top of the readout column.
    pub fn swatch_rect(&self) -> Rect {
        let p = self.panel_rect();
        Rect { x: p.x, y: p.y, w: p.w, h: SWATCH_PT.min(p.h) }
    }

    /// The aperture row, under the swatch: a label and the slider.
    pub fn aperture_row(&self) -> Rect {
        let sw = self.swatch_rect();
        Rect { x: sw.x, y: sw.y + sw.h + PAD_PT / 2.0, w: sw.w, h: APERTURE_PT }
    }

    /// The aperture slider's track, in the lower part of the aperture row.
    pub fn track_rect(&self) -> Rect {
        let a = self.aperture_row();
        Rect { x: a.x + TRACK_PAD_PT, y: a.y + APERTURE_PT - TRACK_PT - 6.0, w: (a.w - 2.0 * TRACK_PAD_PT).max(0.0), h: TRACK_PT }
    }

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            let bar = self.toolbar();
            return self
                .toolbar_layout(ui)
                .visible
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| Target::Toolbar(i.id)))
                .unwrap_or(Target::Nothing);
        }
        self.hit_body(x, y)
    }

    /// What is under the point `(x, y)` below the toolbar: the loupe or the aperture slider.
    pub fn hit_body(&self, x: f32, y: f32) -> Target {
        if self.loupe_rect().contains(x, y) {
            return Target::Loupe;
        }
        if self.track_rect().contains(x, y) {
            return Target::Aperture;
        }
        Target::Nothing
    }

    /// The top of the readout rows, under the aperture row.
    pub fn rows_top(&self) -> f32 {
        let a = self.aperture_row();
        a.y + a.h + PAD_PT / 2.0
    }

    /// Picks the display pixel under the point `(x, y)` of the loupe: a tap on a cell locks the readout there.
    pub fn pick_loupe(&mut self, x: f32, y: f32) -> bool {
        let Some((rect, _)) = self.loupe_pixels() else { return false };
        let lr = self.loupe_rect();
        let cell_pt = lr.w / self.cells as f32;
        if cell_pt <= 0.0 {
            return false;
        }
        let cx = ((x - lr.x) / cell_pt).floor().max(0.0) as u32;
        let cy = ((y - lr.y) / cell_pt).floor().max(0.0) as u32;
        let px = rect.x + cx.min(rect.w.saturating_sub(1));
        let py = rect.y + cy.min(rect.h.saturating_sub(1));
        let changed = (self.pointer.x, self.pointer.y) != (px, py);
        self.pointer.x = px;
        self.pointer.y = py;
        changed
    }

    /// Moves the readout by one pixel with an arrow key, when the source has a display.
    pub fn nudge(&mut self, dx: i32, dy: i32) -> bool {
        match self.display() {
            Some((w, h)) => self.pointer.nudge(dx, dy, w, h),
            None => false,
        }
    }

    /// The action of a target.
    pub fn activate(&mut self, target: Target) -> Outcome {
        match target {
            Target::Toolbar(B_LOCK) => {
                self.pointer.toggle_lock();
                Outcome::Redraw
            }
            Target::Toolbar(B_COPY) => self.copy(self.format),
            Target::Toolbar(B_FORMAT) => {
                self.format = self.format.next();
                Outcome::Redraw
            }
            Target::Toolbar(B_SWATCH) => self.copy_swatch(),
            Target::Toolbar(B_SPACE) => {
                self.space = self.space.next();
                Outcome::Redraw
            }
            Target::Toolbar(B_ZOOM) => {
                let i = ZOOMS.iter().position(|&z| z == self.cells).map_or(0, |i| (i + 1) % ZOOMS.len());
                self.cells = ZOOMS[i];
                Outcome::Redraw
            }
            _ => Outcome::Nothing,
        }
    }

    /// Copies the reading's colour as a swatch image.
    pub fn copy_swatch(&mut self) -> Outcome {
        match self.reading() {
            Some(r) => {
                self.copied = Some("Swatch".into());
                Outcome::CopySwatch(r.codes)
            }
            None => Outcome::Nothing,
        }
    }

    pub fn copy(&mut self, format: CopyFormat) -> Outcome {
        match self.reading() {
            Some(r) => {
                let text = copy_text(&r, format);
                self.copied = Some(text.clone());
                Outcome::CopyText(text)
            }
            None => Outcome::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Fixture;

    fn meter(w: u32, h: u32, at: (u32, u32)) -> ColorMeter {
        let px: Vec<Rgb8> = (0..w * h).map(|i| Rgb8::new((i % 256) as u8, 40, 200)).collect();
        let mut f = Fixture::new(w, h, px).unwrap();
        f.at = Some(at);
        let mut m = ColorMeter::new(Box::new(f));
        m.refresh();
        m
    }

    #[test]
    fn the_readout_follows_the_source_pointer() {
        let m = meter(20, 20, (3, 4));
        assert_eq!((m.pointer.x, m.pointer.y), (3, 4));
        let r = m.reading().unwrap();
        assert_eq!(r.codes, Rgb8::new(3 + 4 * 20, 40, 200));
    }

    #[test]
    fn a_locked_readout_keeps_its_pixel() {
        let mut m = meter(20, 20, (3, 4));
        m.pointer.toggle_lock();
        assert!(!m.refresh());
        assert_eq!((m.pointer.x, m.pointer.y), (3, 4));
    }

    #[test]
    fn without_a_source_there_is_no_reading_and_no_loupe() {
        let m = ColorMeter::new(Box::new(crate::source::ShellSource::default()));
        assert!(m.reading().is_none());
        assert!(m.loupe_pixels().is_none());
        assert!(m.rows().iter().any(|(_, v)| v == "Not available"));
    }

    #[test]
    fn the_loupe_is_a_square_that_fits_the_window() {
        let mut m = ColorMeter::new(Box::new(crate::source::ShellSource::default()));
        m.size = (1000.0, 640.0);
        let l = m.loupe_rect();
        assert_eq!(l.w, l.h);
        assert!(l.x + l.w <= m.size.0 - PANEL_PT);
        assert!(l.y + l.h <= m.size.1);
        m.size = (400.0, 300.0);
        let s = m.loupe_rect();
        assert!(s.w >= 0.0 && s.w <= 400.0);
    }

    #[test]
    fn copy_puts_the_format_text_on_the_clipboard_outcome() {
        let mut m = meter(20, 20, (0, 0));
        m.format = CopyFormat::Decimal;
        assert_eq!(m.activate(Target::Toolbar(B_COPY)), Outcome::CopyText("0, 40, 200".into()));
        assert_eq!(m.copied.as_deref(), Some("0, 40, 200"));
    }

    #[test]
    fn toolbar_actions_cycle_space_format_and_zoom() {
        let mut m = meter(20, 20, (0, 0));
        m.activate(Target::Toolbar(B_SPACE));
        assert_eq!(m.space, Space::DisplayP3);
        m.activate(Target::Toolbar(B_FORMAT));
        assert_eq!(m.format, CopyFormat::Decimal);
        m.activate(Target::Toolbar(B_ZOOM));
        let start = ZOOMS.iter().position(|&z| z == DEFAULT_CELLS).unwrap();
        assert_eq!(m.cells, ZOOMS[start + 1]);
        // the last size wraps to the first
        m.cells = ZOOMS[ZOOMS.len() - 1];
        m.activate(Target::Toolbar(B_ZOOM));
        assert_eq!(m.cells, ZOOMS[0]);
    }

    /// A source whose loupe request is always due.
    struct Probe;

    impl ScreenSource for Probe {
        fn display_size(&self) -> Option<(u32, u32)> {
            Some((100, 100))
        }
        fn pointer(&self) -> Option<(u32, u32)> {
            None
        }
        fn read(&self, _: PixelRect) -> Option<Vec<Rgb8>> {
            None
        }
        fn wants(&mut self, center: (u32, u32), cells: u32) -> Vec<Ask> {
            vec![Ask::Pointer, Ask::Loupe(loupe::sample_rect(center.0, center.1, cells, 100, 100))]
        }
    }

    fn loupe_asks(m: &mut ColorMeter) -> usize {
        m.wants().iter().filter(|a| matches!(a, Ask::Loupe(_))).count()
    }

    #[test]
    fn continuous_updating_asks_for_the_loupe_every_time() {
        let mut m = ColorMeter::new(Box::new(Probe));
        assert!(m.continuous);
        assert_eq!(loupe_asks(&mut m), 1);
        assert_eq!(loupe_asks(&mut m), 1);
    }

    #[test]
    fn without_continuous_updating_the_loupe_is_asked_only_when_the_pixel_moves() {
        let mut m = ColorMeter::new(Box::new(Probe));
        m.set_continuous(false);
        m.pointer.follow(10, 10);
        assert_eq!(loupe_asks(&mut m), 1);
        // the pointer is still polled, the loupe is not repeated
        let again = m.wants();
        assert!(again.contains(&Ask::Pointer));
        assert!(!again.iter().any(|a| matches!(a, Ask::Loupe(_))));
        // a move, a change of loupe size or of aperture is sampled once
        m.pointer.follow(11, 10);
        assert_eq!(loupe_asks(&mut m), 1);
        assert_eq!(loupe_asks(&mut m), 0);
        m.cells = 21;
        assert_eq!(loupe_asks(&mut m), 1);
        m.set_aperture(31);
        assert_eq!(loupe_asks(&mut m), 1);
        assert_eq!(loupe_asks(&mut m), 0);
    }

    #[test]
    fn turning_continuous_updating_back_on_samples_again() {
        let mut m = ColorMeter::new(Box::new(Probe));
        m.set_continuous(false);
        assert_eq!(loupe_asks(&mut m), 1);
        assert_eq!(loupe_asks(&mut m), 0);
        m.set_continuous(true);
        assert_eq!(loupe_asks(&mut m), 1);
        assert_eq!(loupe_asks(&mut m), 1);
        m.set_continuous(false);
        assert_eq!(loupe_asks(&mut m), 1);
    }

    #[test]
    fn the_requested_square_covers_a_wide_aperture() {
        let mut m = ColorMeter::new(Box::new(Probe));
        m.pointer.follow(50, 50);
        m.set_aperture(31);
        let Some(Ask::Loupe(r)) = m.wants().into_iter().find(|a| matches!(a, Ask::Loupe(_))) else { panic!("no loupe ask") };
        assert_eq!((r.w, r.h), (31, 31));
    }

    #[test]
    fn the_aperture_reading_is_the_mean_of_its_square() {
        // a 5 x 5 display whose red channel is the column and green the row
        let px: Vec<Rgb8> = (0..25u32).map(|i| Rgb8::new((i % 5) as u8 * 10, (i / 5) as u8 * 10, 7)).collect();
        let mut f = Fixture::new(5, 5, px).unwrap();
        f.at = Some((2, 2));
        let mut m = ColorMeter::new(Box::new(f));
        m.refresh();
        assert_eq!(m.reading().unwrap().codes, Rgb8::new(20, 20, 7));
        // a 3 x 3 square around (2, 2) has the same mean; around the corner it shrinks inward, mean of (0..3, 0..3)
        assert!(m.set_aperture(3));
        assert_eq!(m.reading().unwrap().codes, Rgb8::new(20, 20, 7));
        m.pointer.nudge(-2, -2, 5, 5);
        assert_eq!(m.reading().unwrap().codes, Rgb8::new(10, 10, 7));
        // a 1-pixel aperture is the pixel itself
        m.set_aperture(1);
        assert_eq!(m.reading().unwrap().codes, Rgb8::new(0, 0, 7));
    }

    #[test]
    fn the_aperture_is_kept_within_its_limits() {
        let mut m = meter(40, 40, (5, 5));
        assert!(!m.set_aperture(0));
        assert_eq!(m.aperture, APERTURE_MIN);
        assert!(m.set_aperture(99));
        assert_eq!(m.aperture, APERTURE_MAX);
        assert!(!m.set_aperture(APERTURE_MAX));
    }

    #[test]
    fn an_even_aperture_size_is_rounded_up_to_the_odd_size_above_it() {
        let mut m = meter(40, 40, (5, 5));
        assert!(m.set_aperture(4));
        assert_eq!(m.aperture, 5);
        assert!(!m.set_aperture(5));
        assert_eq!(odd_aperture(30), 31);
        assert_eq!(odd_aperture(0), APERTURE_MIN);
    }

    #[test]
    fn the_aperture_square_is_centred_on_the_pointer_pixel() {
        let mut m = meter(40, 40, (20, 20));
        for size in [1, 2, 9, 30] {
            m.set_aperture(size);
            let r = m.aperture_rect().unwrap();
            assert_eq!(r.w % 2, 1, "an odd square has a centre pixel");
            assert_eq!(r.x + r.w / 2, 20, "the centre column is the pointer column");
            assert_eq!(r.y + r.h / 2, 20, "the centre row is the pointer row");
        }
    }

    #[test]
    fn the_aperture_slider_maps_the_track_to_sizes() {
        let mut m = meter(40, 40, (5, 5));
        let t = m.track_rect();
        assert!(m.drag_aperture(t.x + t.w));
        assert_eq!(m.aperture, APERTURE_MAX);
        assert!(m.drag_aperture(t.x));
        assert_eq!(m.aperture, APERTURE_MIN);
        assert!(m.drag_aperture(t.x + t.w / 2.0));
        assert_eq!(m.aperture, 17, "the middle of the track is 16, which rounds up to the odd size 17");
        // beyond the ends it clamps
        m.drag_aperture(t.x + 10.0 * t.w);
        assert_eq!(m.aperture, APERTURE_MAX);
        assert!((m.aperture_fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_track_is_found_by_the_hit_test_and_sits_between_the_swatch_and_the_rows() {
        let m = meter(40, 40, (5, 5));
        let t = m.track_rect();
        assert_eq!(m.hit_body(t.x + t.w / 2.0, t.y + t.h / 2.0), Target::Aperture);
        let l = m.loupe_rect();
        assert_eq!(m.hit_body(l.x + l.w / 2.0, l.y + l.h / 2.0), Target::Loupe);
        assert_eq!(m.hit_body(t.x + t.w / 2.0, t.y - 1.0), Target::Nothing);
        assert!(t.y >= m.swatch_rect().y + m.swatch_rect().h);
        assert!(m.rows_top() >= m.aperture_row().y + m.aperture_row().h);
    }

    #[test]
    fn assistive_actions_step_and_set_the_aperture() {
        let mut m = meter(40, 40, (5, 5));
        assert!(m.aperture_action(&AxAction::Increment));
        assert_eq!(m.aperture, 3, "steps move between odd sizes");
        assert!(m.aperture_action(&AxAction::SetValue("9".into())));
        assert_eq!(m.aperture, 9);
        assert!(m.aperture_action(&AxAction::Decrement));
        assert_eq!(m.aperture, 7);
        assert!(!m.aperture_action(&AxAction::SetValue("not a number".into())));
        assert!(!m.aperture_action(&AxAction::Focus));
        assert_eq!(m.aperture, 7);
    }

    #[test]
    fn locking_an_axis_shows_in_the_pointer_row_and_the_toolbar() {
        let mut m = meter(20, 20, (3, 4));
        assert_eq!(m.pointer_state(), "Following");
        m.pointer.toggle_lock_x();
        assert_eq!(m.pointer_state(), "X locked");
        m.pointer.toggle_lock_y();
        assert_eq!(m.pointer_state(), "Locked");
        assert_eq!(m.toolbar().items[0].label, "Unlock");
        m.pointer.toggle_lock_x();
        assert_eq!(m.pointer_state(), "Y locked");
        assert_eq!(m.toolbar().items[0].label, "Lock");
    }

    #[test]
    fn the_position_row_follows_the_show_location_setting() {
        let mut m = meter(20, 20, (3, 4));
        assert!(m.rows().iter().any(|(k, v)| k == "Position" && v == "3, 4"));
        m.show_position = false;
        assert!(!m.rows().iter().any(|(k, _)| k == "Position"));
    }

    #[test]
    fn the_readout_lists_every_format_and_marks_the_copy_format() {
        let mut m = meter(20, 20, (0, 0));
        m.format = CopyFormat::Bit16;
        let rows = m.rows();
        for key in ["Hex", "Decimal", "16-bit", "Percent"] {
            assert!(rows.iter().any(|(k, _)| k == key), "{key}");
        }
        assert!(rows.iter().any(|(k, v)| k == "Copy as" && v == "16-bit"));
        assert!(rows.iter().any(|(k, v)| k == "16-bit" && v == "0, 10280, 51400"));
    }

    #[test]
    fn copying_in_16_bit_puts_the_wide_codes_on_the_clipboard_outcome() {
        let mut m = meter(20, 20, (0, 0));
        assert_eq!(m.copy(CopyFormat::Bit16), Outcome::CopyText("0, 10280, 51400".into()));
    }

    #[test]
    fn tapping_a_loupe_cell_picks_that_pixel() {
        let mut m = meter(40, 40, (20, 20));
        let (rect, _) = m.loupe_pixels().unwrap();
        let l = m.loupe_rect();
        let cell = l.w / m.cells as f32;
        // the centre of cell (1, 0) of the loupe
        assert!(m.pick_loupe(l.x + 1.5 * cell, l.y + 0.5 * cell));
        assert_eq!((m.pointer.x, m.pointer.y), (rect.x + 1, rect.y));
    }

    #[test]
    fn arrow_keys_nudge_the_readout_on_the_display() {
        let mut m = meter(10, 10, (0, 0));
        assert!(m.nudge(1, 1));
        assert_eq!((m.pointer.x, m.pointer.y), (1, 1));
        assert!(m.nudge(-5, -5));
        assert_eq!((m.pointer.x, m.pointer.y), (0, 0));
        assert!(!m.nudge(-1, 0));
    }
}
