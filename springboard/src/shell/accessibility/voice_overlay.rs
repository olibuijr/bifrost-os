//! Voice Control overlays: the labels "show numbers", "show names" and "show grid" draw over the front window so a target
//! can be pressed by what it is called or by a number.
//!
//! The targets are what the screen reader walks (`Shell::reader_walk`): the front window's Settings rows or app elements, and
//! the menu bar titles, or only the rows of a menu while one is open. Each actionable one with bounds on screen is a target;
//! the content comes first in the numbering and the menu bar titles last, so a window's numbers do not move when the menu
//! bar changes. "press 3" presses the third target, "press Save" (with numbers or names shown) the one called Save; an
//! ambiguous name is refused rather than guessed.
//!
//! The grid cuts the front window into 3 by 3 numbered cells. "press 5" narrows the grid to cell 5 and the next grid is drawn
//! inside it; once a cell is no larger than [`MIN_CELL_PT`] pressing its number clicks its centre. "click here" clicks the
//! centre of the grid as it stands. "show grid" starts again from the whole window.
//!
//! One overlay is shown at a time ("show names" replaces "show numbers"). The overlay exists only while the `voice_control`
//! preference is `1`; switching Voice Control off in Settings drops it. The model here is pure; the `Shell` methods at the
//! bottom are the glue (`voice_control.rs` calls them, `Shell::draw` draws).

use crate::gfx::Surface;
use super::walk::{BAR, BAR_ROW};
use crate::shell::services::recognizer::VOICE_CONTROL_PREF;
use crate::shell::ui::{Weight, LINK, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, WHITE};
use crate::shell::{Effect, Event, Shell};
use bifrost_ui::ax::Element;
use bifrost_ui::voice_commands::{self, Overlay, Resolution, Target};
use bifrost_ui::Rect;

/// The grid is this many cells wide and high.
pub const GRID_SIDE: usize = 3;
/// A grid cell no larger than this (points) is clicked instead of narrowed to.
pub const MIN_CELL_PT: f32 = 48.0;
/// A name label is cut to this many characters.
const NAME_MAX_CHARS: usize = 24;
/// Label padding, in points.
const PAD_X_PT: f32 = 5.0;
const PAD_Y_PT: f32 = 1.0;
/// The stroke of the grid lines, in points.
const GRID_LINE_PT: f32 = 1.0;

/// Roles that are not pressed: they are read, not used.
const NOT_TARGET: [&str; 6] = ["window", "text", "list", "progress-indicator", "alert", "image"];

/// Whether an element can be pressed and is on screen.
pub fn is_target(e: &Element) -> bool {
    e.enabled && !NOT_TARGET.contains(&e.role.as_str()) && e.bounds.is_some_and(|b| b.w > 0.0 && b.h > 0.0)
}

/// The targets among the walked elements, in numbering order. `window` is the front window in screen pixels: a content
/// element counts only when its centre is inside it (a Settings row scrolled out of view does not). While a menu is open its
/// rows are the only targets.
pub fn targets(walked: &[Element], window: Option<Rect>) -> Vec<Element> {
    if walked.iter().any(|e| e.handle == BAR_ROW) {
        return walked.iter().filter(|e| e.handle == BAR_ROW && is_target(e)).cloned().collect();
    }
    let inside = |e: &Element| match (window, e.bounds) {
        (Some(w), Some(b)) => w.contains(b.x + b.w / 2.0, b.y + b.h / 2.0),
        _ => false,
    };
    let content = walked.iter().filter(|e| e.handle != BAR && is_target(e) && inside(e));
    let bar = walked.iter().filter(|e| e.handle == BAR && is_target(e));
    content.chain(bar).cloned().collect()
}

/// The number a target is labelled with: its place in `targets`, from 1.
pub fn number_of(index: usize) -> u32 {
    index as u32 + 1
}

/// The target a spoken number or name picks, as an index into `targets`. A name matches ignoring case and spacing; a number
/// is the label drawn by "show numbers".
pub fn pick(targets: &[Element], spoken: &str) -> Resolution {
    let list: Vec<Target> = targets.iter().enumerate().map(|(i, e)| Target { id: i as u32, name: e.name.clone(), actionable: true }).collect();
    voice_commands::resolve(&voice_commands::Command::Click(spoken.to_string()), &list)
}

/// The text a label shows: the number, or the (cut) name.
pub fn label_text(overlay: Overlay, index: usize, name: &str) -> String {
    match overlay {
        Overlay::Names => {
            let mut chars = name.chars();
            let head: String = chars.by_ref().take(NAME_MAX_CHARS).collect();
            if chars.next().is_some() { format!("{head}…") } else { head }
        }
        _ => number_of(index).to_string(),
    }
}

/// The cells of a grid over `area`, row by row, so cell 1 is the top left and cell 9 the bottom right.
pub fn grid_cells(area: Rect) -> Vec<Rect> {
    let (w, h) = (area.w / GRID_SIDE as f32, area.h / GRID_SIDE as f32);
    (0..GRID_SIDE * GRID_SIDE)
        .map(|i| Rect { x: area.x + (i % GRID_SIDE) as f32 * w, y: area.y + (i / GRID_SIDE) as f32 * h, w, h })
        .collect()
}

/// What pressing a grid number did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The grid now covers that cell.
    Narrowed,
    /// The cell is small enough: click its centre.
    Click(f32, f32),
    /// No such cell.
    Refused,
}

/// The overlay on screen and its bookkeeping.
#[derive(Debug, Default)]
pub struct VoiceOverlay {
    shown: Option<Overlay>,
    /// the area the grid has narrowed to; None is the whole base area
    grid: Option<Rect>,
    /// the last frame drew the overlay, so the frame that removes it repaints the area
    drawn: bool,
    /// a click the grid asked for, taken by the next tick
    click: Option<(f32, f32)>,
}

impl VoiceOverlay {
    pub fn shown(&self) -> Option<Overlay> {
        self.shown
    }

    /// Shows `overlay` in place of any other, with the grid back at the whole area.
    pub fn show(&mut self, overlay: Overlay) {
        self.shown = Some(overlay);
        self.grid = None;
    }

    /// Hides `overlay` when it is the one shown. True when something was hidden.
    pub fn hide(&mut self, overlay: Overlay) -> bool {
        let was = self.shown == Some(overlay);
        if was {
            self.shown = None;
            self.grid = None;
        }
        was
    }

    pub fn clear(&mut self) {
        self.shown = None;
        self.grid = None;
        self.click = None;
    }

    /// The area the grid covers now.
    pub fn area(&self, base: Rect) -> Rect {
        self.grid.unwrap_or(base)
    }

    /// Presses grid cell `n` (from 1): narrows the grid to it, or asks for a click when it is no larger than `min_cell`
    /// pixels.
    pub fn narrow(&mut self, base: Rect, n: u32, min_cell: f32) -> Zoom {
        let cells = grid_cells(self.area(base));
        let Some(cell) = (n as usize).checked_sub(1).and_then(|i| cells.get(i)).copied() else { return Zoom::Refused };
        if cell.w <= min_cell || cell.h <= min_cell {
            self.grid = None;
            return Zoom::Click(cell.x + cell.w / 2.0, cell.y + cell.h / 2.0);
        }
        self.grid = Some(cell);
        Zoom::Narrowed
    }

    /// The centre of the area the grid covers now.
    pub fn centre(&self, base: Rect) -> (f32, f32) {
        let a = self.area(base);
        (a.x + a.w / 2.0, a.y + a.h / 2.0)
    }

    /// Starts the grid over from the whole area.
    pub fn reset_grid(&mut self) {
        self.grid = None;
    }
}

impl Shell {
    /// Whether Voice Control is switched on (Settings > Accessibility > Voice Control).
    pub(in crate::shell) fn voice_control_on(&self) -> bool {
        self.prefs.get(VOICE_CONTROL_PREF) == Some("1")
    }

    /// The front window in screen pixels.
    fn voice_window(&self) -> Option<Rect> {
        let w = self.desk.as_ref()?.front()?;
        Some(Rect { x: w.r.x as f32, y: w.r.y as f32, w: w.r.w as f32, h: w.r.h as f32 })
    }

    /// The area the grid starts from: the front window, or the screen when no window is open.
    fn voice_base(&self) -> Rect {
        self.voice_window().unwrap_or(Rect { x: 0.0, y: 0.0, w: self.w as f32, h: self.h as f32 })
    }

    /// The overlay shown now, None when Voice Control is off.
    pub(in crate::shell) fn voice_overlay_shown(&self) -> Option<Overlay> {
        self.voice_overlay.shown().filter(|_| self.voice_control_on())
    }

    /// What "show numbers" and its kin label now, in numbering order.
    pub(in crate::shell) fn voice_targets(&self) -> Vec<Element> {
        targets(&self.reader_walk(), self.voice_window())
    }

    /// "show numbers", "show names" or "show grid". The answer names the overlay.
    pub(in crate::shell) fn voice_show(&mut self, overlay: Overlay) -> String {
        self.voice_overlay.show(overlay);
        self.dirty = true;
        format!("overlay:{}", overlay_name(overlay))
    }

    /// "hide numbers" and kin: hides the overlay when it is the one shown; hiding one that is not shown is refused.
    pub(in crate::shell) fn voice_hide(&mut self, overlay: Overlay) -> Result<String, String> {
        if !self.voice_overlay.hide(overlay) {
            return Err(format!("{} is not shown", overlay_name(overlay)));
        }
        self.dirty = true;
        Ok(format!("overlay:{} hidden", overlay_name(overlay)))
    }

    /// "press 3": with numbers shown, presses the third target; with the grid shown, narrows the grid to its third cell or
    /// clicks it when it is small. Nothing shown is an error that says what to say first.
    pub(in crate::shell) fn voice_press_number(&mut self, n: u32, t: f64) -> Result<(String, Vec<Effect>), String> {
        match self.voice_overlay_shown() {
            Some(Overlay::Numbers) => {
                let targets = self.voice_targets();
                let el = (n as usize).checked_sub(1).and_then(|i| targets.get(i)).cloned().ok_or_else(|| format!("no label {n}"))?;
                let fx = self.press_walked(&el, t);
                self.dirty = true;
                Ok((el.address, fx))
            }
            Some(Overlay::Grid) => {
                let base = self.voice_base();
                let min = self.ui.pt(MIN_CELL_PT);
                match self.voice_overlay.narrow(base, n, min) {
                    Zoom::Narrowed => {
                        self.dirty = true;
                        Ok((format!("grid:{n}"), Vec::new()))
                    }
                    Zoom::Click(x, y) => {
                        self.voice_overlay.click = Some((x, y));
                        self.dirty = true;
                        Ok((format!("grid:{n}:click"), Vec::new()))
                    }
                    Zoom::Refused => Err(format!("no grid cell {n}")),
                }
            }
            _ => Err("say \"show numbers\" or \"show grid\" first".to_string()),
        }
    }

    /// "press Save" with numbers or names shown: presses the visible target called that. None when no label overlay is shown
    /// or no visible target has the name (the caller then looks in the whole element map); an ambiguous name is an error.
    pub(in crate::shell) fn voice_press_label(&mut self, name: &str, t: f64) -> Option<Result<(String, Vec<Effect>), String>> {
        if !matches!(self.voice_overlay_shown(), Some(Overlay::Numbers | Overlay::Names)) {
            return None;
        }
        let targets = self.voice_targets();
        match pick(&targets, name) {
            Resolution::NotFound => None,
            Resolution::Ambiguous(ids) => Some(Err(format!("{name:?} labels {} elements: say a number", ids.len()))),
            Resolution::Matched(i) => {
                let el = targets[i as usize].clone();
                let fx = self.press_walked(&el, t);
                self.dirty = true;
                Some(Ok((el.address, fx)))
            }
        }
    }

    /// "click here": clicks the centre of the grid as it stands. None when the grid is not shown.
    pub(in crate::shell) fn voice_click_here(&mut self) -> Option<Result<(String, Vec<Effect>), String>> {
        if self.voice_overlay_shown() != Some(Overlay::Grid) {
            return None;
        }
        let at = self.voice_overlay.centre(self.voice_base());
        self.voice_overlay.click = Some(at);
        self.dirty = true;
        Some(Ok(("grid:click".to_string(), Vec::new())))
    }

    /// Takes the click the grid asked for and delivers it as a pointer press at that point, then starts the grid over.
    /// Called after the services have ticked, so the click runs like any pointer input.
    pub(in crate::shell) fn voice_click_tick(&mut self, t: f64) -> Vec<Effect> {
        let Some((x, y)) = self.voice_overlay.click.take() else { return Vec::new() };
        self.voice_overlay.reset_grid();
        let mut fx = self.event(Event::Down(x, y), t);
        fx.extend(self.event(Event::Up(x, y), t + 40.0));
        fx
    }

    /// Whether the frame must repaint everything under the overlay.
    pub(in crate::shell) fn voice_overlay_damage(&self) -> bool {
        self.voice_overlay.drawn || self.voice_overlay_shown().is_some()
    }

    /// Draws the labels or the grid. Switching Voice Control off drops the overlay.
    pub(in crate::shell) fn draw_voice_overlay(&mut self, s: &mut Surface) {
        if !self.voice_control_on() {
            self.voice_overlay.clear();
        }
        let mut drawn = false;
        match self.voice_overlay.shown() {
            Some(overlay @ (Overlay::Numbers | Overlay::Names)) => {
                let targets = self.voice_targets();
                for (i, e) in targets.iter().enumerate() {
                    if let Some(b) = e.bounds {
                        let text = label_text(overlay, i, &e.name);
                        self.draw_voice_label(s, &text, b.x, b.y, false);
                    }
                }
                drawn = true;
            }
            Some(Overlay::Grid) => {
                let area = self.voice_overlay.area(self.voice_base());
                let line = self.ui.pt(GRID_LINE_PT);
                for (i, cell) in grid_cells(area).into_iter().enumerate() {
                    s.stroke_rrect(cell.x, cell.y, cell.w, cell.h, 0.0, line, LINK);
                    self.draw_voice_label(s, &number_of(i).to_string(), cell.x + cell.w / 2.0, cell.y + cell.h / 2.0, true);
                }
                drawn = true;
            }
            None => {}
        }
        self.voice_overlay.drawn = drawn;
    }

    /// One label: a systemBlue pill with white text, with its top-left corner at (x, y), or centred on it.
    fn draw_voice_label(&self, s: &mut Surface, text: &str, x: f32, y: f32, centred: bool) {
        let pt = if centred { MAC_CALLOUT_PT } else { MAC_FOOTNOTE_PT };
        let img = self.ui.text(text, pt, Weight::Bold, WHITE);
        let (pad_x, pad_y) = (self.ui.pt(PAD_X_PT), self.ui.pt(PAD_Y_PT));
        let (w, h) = (img.w as f32 + 2.0 * pad_x, img.h as f32 + 2.0 * pad_y);
        let (x, y) = if centred { (x - w / 2.0, y - h / 2.0) } else { (x, y) };
        let x = x.clamp(0.0, (s.w as f32 - w).max(0.0)).round();
        let y = y.clamp(0.0, (s.h as f32 - h).max(0.0)).round();
        s.fill_rrect(x, y, w, h, h / 2.0, LINK);
        s.blit_colored(&img, (x + pad_x) as i32, (y + pad_y) as i32, WHITE, 255);
    }
}

/// The word the overlay is named by in commands and answers.
pub fn overlay_name(overlay: Overlay) -> &'static str {
    match overlay {
        Overlay::Numbers => "numbers",
        Overlay::Names => "names",
        Overlay::Grid => "grid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::walk::SETTINGS;

    fn el(address: &str, role: &str, name: &str, handle: &str, bounds: Option<Rect>) -> Element {
        Element { address: address.into(), name: name.into(), role: role.into(), value: String::new(), enabled: true, bounds, handle: handle.into() }
    }

    fn at(x: f32, y: f32) -> Option<Rect> {
        Some(Rect { x, y, w: 40.0, h: 20.0 })
    }

    const WINDOW: Rect = Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };

    #[test]
    fn content_is_numbered_before_the_menu_bar_and_windows_are_not_targets() {
        let walked = vec![
            el("os/menubar/file", "menu", "File", BAR, at(10.0, 0.0)),
            el("os/window/1", "window", "Settings", "", Some(WINDOW)),
            el("settings/general", "page", "General", SETTINGS, at(120.0, 120.0)),
            el("settings/about", "text", "About", SETTINGS, at(120.0, 160.0)),
            el("settings/scrolled", "page", "Scrolled away", SETTINGS, at(120.0, 900.0)),
        ];
        let t = targets(&walked, Some(WINDOW));
        let names: Vec<&str> = t.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["General", "File"], "text, the window and a row outside the window are left out; content first");
    }

    #[test]
    fn an_open_menu_makes_its_rows_the_only_targets() {
        let walked = vec![
            el("os/menubar/file", "menu", "File", BAR, at(10.0, 0.0)),
            el("os/menubar-row/0", "menu-item", "New", BAR_ROW, at(10.0, 30.0)),
            el("settings/general", "page", "General", SETTINGS, at(120.0, 120.0)),
        ];
        let t = targets(&walked, Some(WINDOW));
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "New");
    }

    #[test]
    fn a_disabled_element_or_one_without_bounds_is_not_a_target() {
        let mut off = el("a", "button", "Off", SETTINGS, at(120.0, 120.0));
        off.enabled = false;
        let nowhere = el("b", "button", "Nowhere", SETTINGS, None);
        assert!(targets(&[off, nowhere], Some(WINDOW)).is_empty());
    }

    #[test]
    fn a_spoken_number_or_name_picks_one_target() {
        let t = vec![
            el("a", "button", "Save", SETTINGS, at(120.0, 120.0)),
            el("b", "button", "Cancel", SETTINGS, at(120.0, 150.0)),
            el("c", "button", "Cancel", SETTINGS, at(120.0, 180.0)),
        ];
        assert_eq!(pick(&t, "2"), Resolution::Matched(1), "numbers count from 1");
        assert_eq!(pick(&t, "save"), Resolution::Matched(0));
        assert_eq!(pick(&t, "Cancel"), Resolution::Ambiguous(vec![1, 2]));
        assert_eq!(pick(&t, "9"), Resolution::NotFound);
    }

    #[test]
    fn labels_show_the_number_or_the_cut_name() {
        assert_eq!(label_text(Overlay::Numbers, 0, "Save"), "1");
        assert_eq!(label_text(Overlay::Names, 0, "Save"), "Save");
        let long = "x".repeat(40);
        assert_eq!(label_text(Overlay::Names, 0, &long), format!("{}…", "x".repeat(NAME_MAX_CHARS)));
    }

    #[test]
    fn the_grid_has_nine_cells_numbered_row_by_row() {
        let cells = grid_cells(Rect { x: 0.0, y: 0.0, w: 300.0, h: 90.0 });
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0], Rect { x: 0.0, y: 0.0, w: 100.0, h: 30.0 });
        assert_eq!(cells[2].x, 200.0, "cell 3 ends the first row");
        assert_eq!(cells[3].y, 30.0, "cell 4 starts the second");
        assert_eq!(cells[8], Rect { x: 200.0, y: 60.0, w: 100.0, h: 30.0 });
    }

    #[test]
    fn grid_numbers_narrow_until_the_cell_is_small_then_click() {
        let mut o = VoiceOverlay::default();
        o.show(Overlay::Grid);
        let base = Rect { x: 0.0, y: 0.0, w: 270.0, h: 270.0 };
        assert_eq!(o.narrow(base, 5, 40.0), Zoom::Narrowed, "a 90 point cell is narrowed to");
        assert_eq!(o.area(base), Rect { x: 90.0, y: 90.0, w: 90.0, h: 90.0 });
        assert_eq!(o.narrow(base, 5, 40.0), Zoom::Click(135.0, 135.0), "the next cell is 30 points: at most 40, so it is clicked");
        assert_eq!(o.area(base), base, "and the grid starts over");
    }

    #[test]
    fn a_small_cell_is_clicked_at_its_centre_and_the_grid_starts_over() {
        let mut o = VoiceOverlay::default();
        o.show(Overlay::Grid);
        let base = Rect { x: 0.0, y: 0.0, w: 90.0, h: 90.0 };
        assert_eq!(o.narrow(base, 1, 40.0), Zoom::Click(15.0, 15.0));
        assert_eq!(o.area(base), base, "back to the whole area");
        assert_eq!(o.narrow(base, 10, 40.0), Zoom::Refused);
        assert_eq!(o.narrow(base, 0, 40.0), Zoom::Refused);
    }

    #[test]
    fn only_the_shown_overlay_can_be_hidden_and_a_new_one_replaces_it() {
        let mut o = VoiceOverlay::default();
        o.show(Overlay::Numbers);
        assert!(!o.hide(Overlay::Grid));
        assert_eq!(o.shown(), Some(Overlay::Numbers));
        o.show(Overlay::Names);
        assert_eq!(o.shown(), Some(Overlay::Names));
        assert!(o.hide(Overlay::Names));
        assert_eq!(o.shown(), None);
    }

    // --- the wired path: a spoken command, the Settings switch, the drawn labels and the press -----------------------------

    use crate::shell::pages::PageId;
    use crate::shell::tests::desktop_shell;
    use bifrost_ui::whisper::Transcript;

    /// A desktop shell with Voice Control on and the Software Update page of Settings in front.
    fn voice_shell() -> Shell {
        let mut sh = desktop_shell();
        sh.prefs.set(VOICE_CONTROL_PREF, "1").unwrap();
        sh.ax_press("General > Software Update", 1.0).expect("opens Settings");
        settle_settings(&mut sh);
        sh
    }

    /// Lets the page slide of Settings end, as the frames would: a touch is ignored while it runs.
    fn settle_settings(sh: &mut Shell) {
        let ui = &sh.ui;
        sh.settings.as_mut().expect("Settings is open").tick(ui, 1e9);
    }

    fn say(sh: &mut Shell, text: &str, t: f64) -> Option<Result<(String, Vec<Effect>), String>> {
        sh.voice_press(&Transcript { text: text.into(), is_final: true }, t)
    }

    /// Draws only the overlay onto a blank surface and counts the pixels it touched.
    fn painted(sh: &mut Shell) -> usize {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; w * h];
        sh.draw_voice_overlay(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        buf.iter().filter(|&&p| p != 0).count()
    }

    fn index_of(targets: &[Element], address_prefix: &str, name: &str) -> usize {
        targets.iter().position(|e| e.address.starts_with(address_prefix) && e.name == name).unwrap_or_else(|| {
            panic!("no target {name:?} under {address_prefix}: {:?}", targets.iter().map(|e| (&e.address, &e.name)).collect::<Vec<_>>())
        })
    }

    #[test]
    fn the_settings_switch_writes_the_preference_the_recognizer_reads() {
        let mut sh = desktop_shell();
        assert!(!sh.voice_control_on());
        assert!(sh.ax_map(0.0).get("os/settings/accessibility/voice-control").is_some_and(|e| e.role == "toggle"), "the switch is on the Accessibility page");
        sh.ax_press("Accessibility > Voice Control", 1.0).expect("flips the switch");
        assert_eq!(sh.prefs.get(VOICE_CONTROL_PREF), Some("1"));
        assert!(sh.voice_control_on());
        settle_settings(&mut sh);
        sh.ax_press("Accessibility > Voice Control", 2.0).expect("flips it back");
        assert_eq!(sh.prefs.get(VOICE_CONTROL_PREF), Some("0"));
        assert!(!sh.voice_control_on());
    }

    #[test]
    fn show_numbers_labels_the_front_window_and_hide_removes_the_labels() {
        let mut sh = voice_shell();
        assert_eq!(painted(&mut sh), 0, "nothing is drawn before the command");
        let answer = say(&mut sh, "show numbers", 2.0).expect("a command").expect("runs");
        assert_eq!(answer.0, "overlay:numbers");
        assert_eq!(sh.voice_overlay_shown(), Some(Overlay::Numbers));
        let targets = sh.voice_targets();
        assert!(targets.iter().any(|e| e.address.starts_with("settings-sidebar/") && e.name == "General"), "the sidebar rows are labelled");
        assert!(targets.iter().any(|e| e.name == "Roll Back to Previous System"), "so are the rows of the page");
        assert!(targets.iter().all(|e| e.role != "window" && e.role != "text"), "windows and text are not labelled");
        assert!(painted(&mut sh) > 0, "the labels are drawn");
        assert!(sh.voice_overlay_damage(), "a shown overlay repaints what is under it");
        assert!(say(&mut sh, "hide names", 3.0).expect("a command").is_err(), "only the overlay that is shown can be hidden");
        say(&mut sh, "hide numbers", 4.0).expect("a command").expect("hidden");
        assert!(sh.voice_overlay_damage(), "the frame that removes the labels repaints their area");
        assert_eq!(painted(&mut sh), 0);
        assert!(!sh.voice_overlay_damage(), "then the shell is quiet again");
    }

    #[test]
    fn a_spoken_number_presses_the_numbered_sidebar_row() {
        let mut sh = voice_shell();
        assert_eq!(sh.settings.as_ref().unwrap().page(), PageId::SoftwareUpdate);
        assert!(say(&mut sh, "press 1", 2.0).expect("a command").is_err(), "no labels yet: it says what to say first");
        say(&mut sh, "show numbers", 3.0).unwrap().unwrap();
        let n = index_of(&sh.voice_targets(), "settings-sidebar/", "General") + 1;
        let (address, _) = say(&mut sh, &format!("press {n}"), 4.0).expect("a command").expect("presses");
        assert!(address.starts_with("settings-sidebar/"), "{address}");
        settle_settings(&mut sh);
        assert_eq!(sh.settings.as_ref().unwrap().page(), PageId::General, "the General row was tapped");
        assert!(say(&mut sh, "press 999", 5.0).expect("a command").is_err(), "a number nothing carries is refused");
    }

    #[test]
    fn with_names_shown_a_spoken_name_presses_the_visible_row() {
        let mut sh = voice_shell();
        say(&mut sh, "show names", 2.0).unwrap().unwrap();
        assert_eq!(sh.voice_overlay_shown(), Some(Overlay::Names));
        assert!(painted(&mut sh) > 0);
        let (address, _) = say(&mut sh, "press roll back to previous system", 3.0).expect("a command").expect("presses");
        assert!(address.ends_with("roll-back-to-previous-system"), "{address}");
        let map = sh.ax_map(4.0);
        assert!(map.get("os/settings/alert").is_some_and(|e| e.value == "Roll Back to Previous System?"), "the row's action ran: its alert is open");
    }

    #[test]
    fn the_grid_narrows_to_a_cell_then_clicks_and_starts_over() {
        let mut sh = voice_shell();
        say(&mut sh, "show grid", 2.0).unwrap().unwrap();
        assert_eq!(sh.voice_overlay_shown(), Some(Overlay::Grid));
        assert!(painted(&mut sh) > 0, "the grid is drawn");
        let window = sh.voice_base();
        // the top left cell, again and again: two narrowings, then a click in the window's top left corner
        assert_eq!(say(&mut sh, "press 1", 3.0).unwrap().unwrap().0, "grid:1");
        let narrowed = sh.voice_overlay.area(window);
        assert!(narrowed.w < window.w / 2.0 && narrowed.x == window.x, "the grid now covers one cell: {narrowed:?}");
        assert_eq!(say(&mut sh, "tap 1", 4.0).unwrap().unwrap().0, "grid:1");
        assert_eq!(say(&mut sh, "press 1", 5.0).unwrap().unwrap().0, "grid:1:click");
        let (x, y) = sh.voice_overlay.click.expect("a click is waiting for the tick");
        assert!(window.contains(x, y) && x < window.x + window.w / 3.0 && y < window.y + window.h / 3.0, "({x}, {y}) in {window:?}");
        assert_eq!(sh.voice_overlay.area(window), window, "the grid starts over");
        sh.voice_click_tick(6.0);
        assert!(sh.voice_overlay.click.is_none(), "the tick delivered it");
        assert!(say(&mut sh, "press 10", 7.0).unwrap().is_err(), "the grid has nine cells");
        // click here: the centre of the grid as it stands
        say(&mut sh, "press 5", 8.0).unwrap().unwrap();
        let centre = sh.voice_overlay.centre(window);
        say(&mut sh, "click here", 9.0).unwrap().unwrap();
        assert_eq!(sh.voice_overlay.click, Some(centre));
    }

    #[test]
    fn click_here_without_the_grid_names_an_element_and_a_new_overlay_replaces_the_old() {
        let mut sh = voice_shell();
        assert!(say(&mut sh, "click here", 2.0).expect("a command").is_err(), "no grid and no element called Here");
        say(&mut sh, "show grid", 3.0).unwrap().unwrap();
        say(&mut sh, "show numbers", 4.0).unwrap().unwrap();
        assert_eq!(sh.voice_overlay_shown(), Some(Overlay::Numbers), "one overlay at a time");
    }

    #[test]
    fn switching_voice_control_off_drops_the_overlay() {
        let mut sh = voice_shell();
        say(&mut sh, "show numbers", 2.0).unwrap().unwrap();
        assert!(painted(&mut sh) > 0);
        sh.prefs.set(VOICE_CONTROL_PREF, "0").unwrap();
        assert_eq!(sh.voice_overlay_shown(), None, "off means no overlay even before the next frame");
        assert_eq!(painted(&mut sh), 0);
        sh.prefs.set(VOICE_CONTROL_PREF, "1").unwrap();
        assert_eq!(sh.voice_overlay_shown(), None, "and it does not come back by itself");
    }
}
