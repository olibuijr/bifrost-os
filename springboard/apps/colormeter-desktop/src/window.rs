//! The Color Meter window's state and geometry: the toolbar, the magnifier on the left and the readout on the right.
//! Layout is in points and derives from the window size; the drawing and the hit tests read the same rectangles, so a
//! tap lands on what is drawn. The state reads pixels only through `ScreenSource`.

use crate::color::{self, convert, copy_text, CopyFormat, Reading, Rgb8, Space};
use crate::loupe::{self, Pointer};
use crate::source::{PixelRect, ScreenSource};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
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

/// The loupe sizes the Zoom button cycles through, in cells across.
pub const ZOOMS: [u32; 4] = [11, 15, 21, 31];

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    Loupe,
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
            cells: ZOOMS[1],
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
        let rect = PixelRect { x: self.pointer.x, y: self.pointer.y, w: 1, h: 1 };
        let px = self.source.read(rect)?.first().copied()?;
        Some(convert(px, self.space))
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
                rows.push(("Percent".into(), color::percent(r.values)));
            }
            None => rows.push(("Value".into(), "Not available".into())),
        }
        rows.push(("Position".into(), format!("{}, {}", self.pointer.x, self.pointer.y)));
        rows.push(("Pointer".into(), if self.pointer.locked { "Locked".into() } else { "Following".into() }));
        if let Some(text) = &self.copied {
            rows.push(("Copied".into(), text.clone()));
        }
        rows
    }

    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(B_LOCK, if self.pointer.locked { "Unlock" } else { "Lock" }),
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
        if self.loupe_rect().contains(x, y) {
            return Target::Loupe;
        }
        Target::Nothing
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
            Target::Toolbar(B_SWATCH) => match self.reading() {
                Some(r) => {
                    self.copied = Some("Swatch".into());
                    Outcome::CopySwatch(r.codes)
                }
                None => Outcome::Nothing,
            },
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

    fn copy(&mut self, format: CopyFormat) -> Outcome {
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
        let m = ColorMeter::new(Box::new(crate::source::NoCapture));
        assert!(m.reading().is_none());
        assert!(m.loupe_pixels().is_none());
        assert!(m.rows().iter().any(|(_, v)| v == "Not available"));
    }

    #[test]
    fn the_loupe_is_a_square_that_fits_the_window() {
        let mut m = ColorMeter::new(Box::new(crate::source::NoCapture));
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
        assert_eq!(m.cells, ZOOMS[2]);
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
