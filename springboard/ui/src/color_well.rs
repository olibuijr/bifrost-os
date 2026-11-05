//! `NativeColorWell`: a control that shows a colour and lets the user change it, in the three styles of the reference
//! (default, minimal, expanded), with the shared colour panel behind it.
//!
//! [`ColorWells`] holds an app's wells and plays the shared-panel protocol: activating a well (exclusively or not) puts
//! its colour into the panel, the panel's changes go to every active well, and a well can take a colour dropped on
//! it. [`SwatchGrid`] lays out the swatch popover the minimal and expanded styles open, from a colour list's swatches
//! (`bifrost_app::color_list::ColorList::swatches`). [`draw_well`] draws a well with the named system colours.
//!
//! Behaviour reference: the colour well and colour panel documentation (REA evidence
//! ev_707cf40ef10ba0a4f1899622a486b7b1d441668d39a5c31a9cdc6fbef9e5e470 and
//! ev_6c3ad0e07099189a330191100d5b1564f636b6064b3a70563d484aca64e11954); docs/bifrost-ui.md, "Colour well".

use crate::panels::ColorPanel;
use bifrost_render::gfx::Surface;
use bifrost_render::ui::system;

/// Corner radius of a well, in points (the small legacy control radius).
pub const WELL_RADIUS_PT: f32 = 6.0;
/// The ring between a well's border and its swatch, in points.
pub const WELL_INSET_PT: f32 = 3.0;

/// `NativeColorWell.Style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WellStyle {
    /// A bordered swatch; a click activates the well and shows the colour panel.
    #[default]
    Default,
    /// A swatch with little adornment; a click opens the swatch popover.
    Minimal,
    /// A swatch with a popover for quick picks and a separate button that shows the colour panel.
    Expanded,
}

/// The part of a well under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WellPart {
    Swatch,
    /// The expanded style's panel button.
    PanelButton,
}

/// What a click on a well asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WellAction {
    /// The well became active and the colour panel shows its colour.
    Activated,
    /// The well was active and is now inactive (a second click on a default well).
    Deactivated,
    /// Show the swatch popover anchored on the well.
    ShowPopover,
    /// Nothing: the well is disabled.
    None,
}

/// One colour well.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorWell {
    pub color: (u8, u8, u8),
    /// Opacity 0..1; kept at 1 when the well does not support alpha.
    pub alpha: f32,
    pub style: WellStyle,
    /// `isBordered`.
    pub bordered: bool,
    /// `supportsAlpha`: whether the panel shows an opacity slider for this well.
    pub supports_alpha: bool,
    pub enabled: bool,
    active: bool,
}

impl ColorWell {
    /// A bordered, enabled, inactive well of `style` on an opaque colour.
    pub fn new(style: WellStyle, color: (u8, u8, u8)) -> Self {
        ColorWell { color, alpha: 1.0, style, bordered: true, supports_alpha: true, enabled: true, active: false }
    }

    /// `isActive`.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sets the colour; the alpha is dropped to opaque when the well does not support alpha.
    pub fn set_color(&mut self, color: (u8, u8, u8), alpha: f32) {
        self.color = color;
        self.alpha = if self.supports_alpha { alpha.clamp(0.0, 1.0) } else { 1.0 };
    }

    /// The colour as a premultiplied surface colour.
    pub fn surface_color(&self) -> u32 {
        let (r, g, b) = self.color;
        bifrost_render::gfx::rgba(r, g, b, (self.alpha * 255.0).round() as u8)
    }

    /// The swatch and (expanded style) the panel button inside a well's rect `(x, y, w, h)`, in the same units. The
    /// button is a square of the well's height at its trailing end; the swatch is inset by `inset` inside the
    /// border (no inset for an unbordered well).
    pub fn parts(&self, (x, y, w, h): (f32, f32, f32, f32), inset: f32) -> ((f32, f32, f32, f32), Option<(f32, f32, f32, f32)>) {
        let button = (self.style == WellStyle::Expanded && w > h * 2.0).then(|| (x + w - h, y, h, h));
        let body_w = if button.is_some() { w - h } else { w };
        let i = if self.bordered { inset.min(body_w / 4.0).min(h / 4.0) } else { 0.0 };
        ((x + i, y + i, (body_w - 2.0 * i).max(0.0), (h - 2.0 * i).max(0.0)), button)
    }

    /// The part at `(px, py)`, or none outside the well.
    pub fn hit(&self, rect: (f32, f32, f32, f32), inset: f32, px: f32, py: f32) -> Option<WellPart> {
        let inside = |(x, y, w, h): (f32, f32, f32, f32)| px >= x && px < x + w && py >= y && py < y + h;
        if !inside(rect) {
            return None;
        }
        match self.parts(rect, inset).1 {
            Some(b) if inside(b) => Some(WellPart::PanelButton),
            _ => Some(WellPart::Swatch),
        }
    }
}

/// The wells of an app (or a window) and the shared colour panel protocol between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorWells {
    wells: Vec<ColorWell>,
}

impl ColorWells {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a well; returns its index.
    pub fn add(&mut self, well: ColorWell) -> usize {
        self.wells.push(well);
        self.wells.len() - 1
    }

    pub fn get(&self, i: usize) -> Option<&ColorWell> {
        self.wells.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut ColorWell> {
        self.wells.get_mut(i)
    }

    pub fn len(&self) -> usize {
        self.wells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wells.is_empty()
    }

    /// The indices of the active wells.
    pub fn active(&self) -> Vec<usize> {
        (0..self.wells.len()).filter(|&i| self.wells[i].active).collect()
    }

    /// `activate(_:)`: makes well `i` active and puts its colour (and whether it supports alpha) into the panel. With
    /// `exclusive`, every other well is deactivated first; without, the others keep their state, so several wells can
    /// follow the panel (shift-click). False for an unknown or disabled well.
    pub fn activate(&mut self, i: usize, exclusive: bool, panel: &mut ColorPanel) -> bool {
        if !self.wells.get(i).is_some_and(|w| w.enabled) {
            return false;
        }
        if exclusive {
            for (k, w) in self.wells.iter_mut().enumerate() {
                w.active &= k == i;
            }
        }
        let w = &mut self.wells[i];
        w.active = true;
        panel.set_shows_alpha(w.supports_alpha);
        panel.set_rgb(w.color);
        panel.set_alpha(w.alpha);
        true
    }

    /// `deactivate()`: well `i` stops following the panel. Returns whether it was active.
    pub fn deactivate(&mut self, i: usize) -> bool {
        match self.wells.get_mut(i) {
            Some(w) if w.active => {
                w.active = false;
                true
            }
            _ => false,
        }
    }

    /// The panel's colour changed: every active well takes it. Returns the wells whose colour changed (each sends
    /// its action).
    pub fn panel_changed(&mut self, panel: &ColorPanel) -> Vec<usize> {
        let (c, a) = (panel.color(), panel.alpha());
        let mut changed = Vec::new();
        for (i, w) in self.wells.iter_mut().enumerate().filter(|(_, w)| w.active) {
            let before = (w.color, w.alpha);
            w.set_color(c, a);
            if (w.color, w.alpha) != before {
                changed.push(i);
            }
        }
        changed
    }

    /// `takeColorFrom(_:)` / a colour dropped on well `i`: the well takes the colour, and when it is active the panel
    /// follows. Returns whether the well's colour changed.
    pub fn take_color(&mut self, i: usize, color: (u8, u8, u8), alpha: f32, panel: &mut ColorPanel) -> bool {
        let Some(w) = self.wells.get_mut(i).filter(|w| w.enabled) else { return false };
        let before = (w.color, w.alpha);
        w.set_color(color, alpha);
        if w.active {
            panel.set_rgb(w.color);
            panel.set_alpha(w.alpha);
        }
        (w.color, w.alpha) != before
    }

    /// A click on `part` of well `i`, following the style: a default well toggles its activation (exclusive); a
    /// minimal well, and the swatch of an expanded well, open the swatch popover; the expanded well's button
    /// activates the well and shows the panel.
    pub fn click(&mut self, i: usize, part: WellPart, panel: &mut ColorPanel) -> WellAction {
        let Some((style, active)) = self.wells.get(i).filter(|w| w.enabled).map(|w| (w.style, w.active)) else {
            return WellAction::None;
        };
        match (style, part) {
            (WellStyle::Default, _) if active => {
                self.deactivate(i);
                WellAction::Deactivated
            }
            (WellStyle::Default, _) | (WellStyle::Expanded, WellPart::PanelButton) => {
                self.activate(i, true, panel);
                WellAction::Activated
            }
            _ => WellAction::ShowPopover,
        }
    }
}

/// The swatch grid of a colour well popover: `count` swatches in rows of `columns`, each `cell` square with `gap`
/// between, from the popover's content origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwatchGrid {
    pub count: usize,
    pub columns: usize,
    pub cell: f32,
    pub gap: f32,
}

impl SwatchGrid {
    /// The content size `(w, h)` the grid needs.
    pub fn size(&self) -> (f32, f32) {
        let cols = self.columns.max(1).min(self.count.max(1));
        let rows = self.count.div_ceil(self.columns.max(1));
        let span = |n: usize| if n == 0 { 0.0 } else { n as f32 * self.cell + (n - 1) as f32 * self.gap };
        (span(cols), span(rows))
    }

    /// The rect of swatch `i`.
    pub fn rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        if i >= self.count {
            return None;
        }
        let cols = self.columns.max(1);
        let step = self.cell + self.gap;
        Some(((i % cols) as f32 * step, (i / cols) as f32 * step, self.cell, self.cell))
    }

    /// The swatch under `(x, y)` (content coordinates); a point in a gap picks nothing.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let step = self.cell + self.gap;
        let (col, row) = ((x / step) as usize, (y / step) as usize);
        if col >= self.columns.max(1) || x - col as f32 * step >= self.cell || y - row as f32 * step >= self.cell {
            return None;
        }
        let i = row * self.columns.max(1) + col;
        (i < self.count).then_some(i)
    }
}

/// Draws `well` into the pixel rect `(x, y, w, h)` at `scale` pixels per point: the control face and border (when
/// bordered), a checkerboard under a translucent colour, the swatch, an accent ring when active, and the expanded
/// style's panel button face.
pub fn draw_well(s: &mut Surface, well: &ColorWell, rect: (f32, f32, f32, f32), scale: f32) {
    let r = WELL_RADIUS_PT * scale;
    let (x, y, w, h) = rect;
    if well.bordered {
        s.fill_rrect(x, y, w, h, r, system::CONTROL.now());
        s.stroke_rrect(x, y, w, h, r, scale.max(1.0), system::SEPARATOR.now());
    }
    let ((sx, sy, sw, sh), button) = well.parts(rect, WELL_INSET_PT * scale);
    let sr = if well.bordered { (r - WELL_INSET_PT * scale).max(scale) } else { r };
    if well.alpha < 1.0 && sw >= 2.0 && sh >= 2.0 {
        // the left half light, the right half dark, so the opacity reads against both
        s.fill_rrect(sx, sy, sw, sh, sr, system::CONTROL_BACKGROUND.now());
        let half = (sw / 2.0).floor();
        s.fill_rect((sx + half).max(0.0) as usize, sy.max(0.0) as usize, (sw - half) as usize, sh as usize, system::GRID.now());
    }
    s.fill_rrect(sx, sy, sw, sh, sr, well.surface_color());
    if !well.enabled {
        s.fill_rrect(sx, sy, sw, sh, sr, system::DISABLED_CONTROL_TEXT.now());
    }
    if well.active {
        s.stroke_rrect(x, y, w, h, r, 2.0 * scale.max(1.0), system::CONTROL_ACCENT.now());
    }
    if let Some((bx, by, bw, bh)) = button {
        s.fill_rrect(bx, by, bw, bh, r, system::CONTROL_HIGHLIGHT.now());
        s.stroke_rrect(bx, by, bw, bh, r, scale.max(1.0), system::SEPARATOR.now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_activation_leaves_one_well_following_the_panel() {
        let mut wells = ColorWells::new();
        let a = wells.add(ColorWell::new(WellStyle::Default, (255, 0, 0)));
        let b = wells.add(ColorWell::new(WellStyle::Default, (0, 0, 255)));
        let mut panel = ColorPanel::new((0, 0, 0));
        assert!(wells.activate(a, true, &mut panel));
        assert_eq!(panel.color(), (255, 0, 0), "the panel shows the well's colour");
        assert!(wells.activate(b, false, &mut panel));
        assert_eq!(wells.active(), vec![a, b], "a non-exclusive activation keeps the others");
        assert!(wells.activate(a, true, &mut panel));
        assert_eq!(wells.active(), vec![a]);
        panel.set_rgb((1, 2, 3));
        assert_eq!(wells.panel_changed(&panel), vec![a]);
        assert_eq!(wells.get(b).unwrap().color, (0, 0, 255), "an inactive well keeps its colour");
        assert!(wells.deactivate(a));
        assert!(!wells.deactivate(a));
    }

    #[test]
    fn a_well_without_alpha_stays_opaque_and_hides_the_opacity_slider() {
        let mut wells = ColorWells::new();
        let mut w = ColorWell::new(WellStyle::Default, (9, 9, 9));
        w.supports_alpha = false;
        let i = wells.add(w);
        let mut panel = ColorPanel::new((0, 0, 0));
        panel.set_shows_alpha(true);
        wells.activate(i, true, &mut panel);
        assert!(!panel.shows_alpha());
        panel.set_alpha(0.25);
        wells.panel_changed(&panel);
        assert_eq!(wells.get(i).unwrap().alpha, 1.0);
    }

    #[test]
    fn clicks_follow_the_style() {
        let mut wells = ColorWells::new();
        let d = wells.add(ColorWell::new(WellStyle::Default, (0, 0, 0)));
        let m = wells.add(ColorWell::new(WellStyle::Minimal, (0, 0, 0)));
        let e = wells.add(ColorWell::new(WellStyle::Expanded, (0, 0, 0)));
        let mut panel = ColorPanel::new((0, 0, 0));
        assert_eq!(wells.click(d, WellPart::Swatch, &mut panel), WellAction::Activated);
        assert_eq!(wells.click(d, WellPart::Swatch, &mut panel), WellAction::Deactivated);
        assert_eq!(wells.click(m, WellPart::Swatch, &mut panel), WellAction::ShowPopover);
        assert_eq!(wells.click(e, WellPart::Swatch, &mut panel), WellAction::ShowPopover);
        assert_eq!(wells.click(e, WellPart::PanelButton, &mut panel), WellAction::Activated);
        wells.get_mut(m).unwrap().enabled = false;
        assert_eq!(wells.click(m, WellPart::Swatch, &mut panel), WellAction::None);
        assert!(!wells.activate(m, true, &mut panel));
    }

    #[test]
    fn a_dropped_colour_reaches_the_panel_only_from_an_active_well() {
        let mut wells = ColorWells::new();
        let i = wells.add(ColorWell::new(WellStyle::Default, (0, 0, 0)));
        let mut panel = ColorPanel::new((7, 7, 7));
        assert!(wells.take_color(i, (10, 20, 30), 1.0, &mut panel));
        assert_eq!(panel.color(), (7, 7, 7));
        wells.activate(i, true, &mut panel);
        assert!(wells.take_color(i, (40, 50, 60), 0.5, &mut panel));
        assert_eq!((panel.color(), panel.alpha()), ((40, 50, 60), 0.5));
        assert!(!wells.take_color(i, (40, 50, 60), 0.5, &mut panel), "the same colour changes nothing");
    }

    #[test]
    fn the_expanded_style_has_a_square_panel_button_at_its_end() {
        let w = ColorWell::new(WellStyle::Expanded, (0, 0, 0));
        let (swatch, button) = w.parts((0.0, 0.0, 60.0, 20.0), 3.0);
        assert_eq!(button, Some((40.0, 0.0, 20.0, 20.0)));
        assert_eq!(swatch, (3.0, 3.0, 34.0, 14.0));
        assert_eq!(w.hit((0.0, 0.0, 60.0, 20.0), 3.0, 45.0, 5.0), Some(WellPart::PanelButton));
        assert_eq!(w.hit((0.0, 0.0, 60.0, 20.0), 3.0, 5.0, 5.0), Some(WellPart::Swatch));
        assert_eq!(w.hit((0.0, 0.0, 60.0, 20.0), 3.0, 65.0, 5.0), None);
        let mut plain = ColorWell::new(WellStyle::Default, (0, 0, 0));
        plain.bordered = false;
        assert_eq!(plain.parts((0.0, 0.0, 60.0, 20.0), 3.0), ((0.0, 0.0, 60.0, 20.0), None));
    }

    #[test]
    fn the_swatch_grid_lays_out_rows_and_hits_cells_not_gaps() {
        let g = SwatchGrid { count: 13, columns: 5, cell: 10.0, gap: 2.0 };
        assert_eq!(g.size(), (58.0, 34.0));
        assert_eq!(g.rect(6), Some((12.0, 12.0, 10.0, 10.0)));
        assert_eq!(g.rect(13), None);
        assert_eq!(g.hit(13.0, 13.0), Some(6));
        assert_eq!(g.hit(11.0, 1.0), None, "the gap between two swatches");
        assert_eq!(g.hit(40.0, 30.0), None, "past the last swatch");
        assert_eq!(g.hit(-1.0, 0.0), None);
    }

    #[test]
    fn a_drawn_well_shows_its_colour_in_the_swatch() {
        let (w, h) = (40usize, 20usize);
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let well = ColorWell::new(WellStyle::Default, (0, 200, 0));
        draw_well(&mut s, &well, (0.0, 0.0, w as f32, h as f32), 1.0);
        let c = px[10 * w + 20];
        assert_eq!((c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff), (0, 200, 0));
    }
}
