//! The window's state, its geometry in points, and what a touch or a key does. Painting is in `draw.rs`, and the plot
//! itself in `plot.rs`. Every rectangle here is in points; touch positions arrive in device pixels and are converted
//! with `ui.pt(1.0)`.

use crate::model::{Model, Row};
use crate::plot::{self, Area};
use crate::view::View;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{pack, TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::{KeyInput, TouchKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The width of the equation sidebar, in points.
pub const SIDE_PT: f32 = 260.0;
/// The height of one equation row in the sidebar, in points.
pub const ROW_PT: f32 = 26.0;
/// The height of the panel under the sidebar list: the equation field, the hint and the status line.
pub const PANEL_PT: f32 = 112.0;
/// The file the equations are saved to, inside the app's data folder.
pub const SAVE_FILE: &str = "equations.txt";
/// The folder PNG exports are written to, inside the app's data folder.
pub const EXPORT_DIR: &str = "exports";

/// Toolbar item ids.
pub const B_ADD: u32 = 1;
pub const B_REMOVE: u32 = 2;
pub const B_ZOOM_OUT: u32 = 3;
pub const B_ZOOM_IN: u32 = 4;
pub const B_RESET: u32 = 5;
pub const B_EXPORT: u32 = 6;

/// Sidebar row ids are `ROW_BASE + row id`, so they never meet the toolbar ids.
pub const ROW_BASE: u32 = 100;
/// The zoom factor of one press of a zoom button.
const ZOOM_STEP: f64 = 1.25;

/// What a press or a release was on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Button(u32),
    Row(u32),
    Plot,
}

pub struct Grapher {
    pub model: Model,
    pub labels: TextCache,
    /// The window size in points.
    pub size: (f32, f32),
    /// The target of the press in progress.
    pub press: Target,
    /// The last pointer position in device pixels, while a drag pans the plot.
    pub last_px: (f32, f32),
    /// The line under the sidebar: the result of the last export, or the last failure to save.
    pub message: String,
    /// True once the saved equations were read (or the first run found none).
    pub loaded: bool,
}

impl Grapher {
    pub fn new() -> Grapher {
        Grapher {
            model: Model::starter(),
            labels: TextCache::default(),
            size: (0.0, 0.0),
            press: Target::Nothing,
            last_px: (0.0, 0.0),
            message: String::new(),
            loaded: false,
        }
    }

    /// Reads the saved equations once. A first run, with no file yet, keeps the starter equations.
    pub fn load(&mut self, dir: &Path) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Ok(text) = std::fs::read_to_string(dir.join(SAVE_FILE)) {
            self.model = Model::from_text(&text);
        }
    }

    fn persist(&mut self, dir: &Path) {
        let result = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(SAVE_FILE), self.model.to_text()));
        if let Err(error) = result {
            self.message = format!("Could not save the equations: {error}");
        }
    }

    // ---- geometry, in points ----

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    pub fn sidebar_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT, h: (self.size.1 - UNIFIED_BAR_PT).max(0.0) }
    }

    /// The equation list: the sidebar above the panel.
    pub fn list_rect(&self) -> Rect {
        let side = self.sidebar_rect();
        Rect { x: side.x, y: side.y, w: side.w, h: (side.h - PANEL_PT).max(0.0) }
    }

    /// The field where the selected equation is typed: the top of the panel.
    pub fn field_rect(&self) -> Rect {
        let panel_top = self.size.1 - PANEL_PT;
        Rect { x: 12.0, y: panel_top + 12.0, w: SIDE_PT - 24.0, h: 26.0 }
    }

    /// The plot, to the right of the sidebar and under the bar.
    pub fn plot_rect(&self) -> Rect {
        Rect {
            x: SIDE_PT,
            y: UNIFIED_BAR_PT,
            w: (self.size.0 - SIDE_PT).max(0.0),
            h: (self.size.1 - UNIFIED_BAR_PT).max(0.0),
        }
    }

    /// The plot in device pixels.
    pub fn plot_area(&self, ui: &Ui) -> Area {
        let r = self.plot_rect();
        Area {
            x: ui.pt(r.x).max(0.0) as usize,
            y: ui.pt(r.y).max(0.0) as usize,
            w: ui.pt(r.w).max(0.0) as usize,
            h: ui.pt(r.h).max(0.0) as usize,
        }
    }

    // ---- the parts of the window ----

    pub fn toolbar(&self) -> Toolbar {
        let has_selection = self.model.selected.is_some();
        Toolbar::new(vec![
            ToolbarItem::icon(B_ADD, "Add Equation", "plus"),
            ToolbarItem::icon(B_REMOVE, "Remove Equation", "trash").enabled(has_selection),
            ToolbarItem::icon(B_ZOOM_OUT, "Zoom Out", "minus.magnifyingglass").in_group(1),
            ToolbarItem::icon(B_ZOOM_IN, "Zoom In", "plus.magnifyingglass").in_group(1),
            ToolbarItem::button(B_RESET, "Reset View"),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(B_EXPORT, "Export Image"),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The sidebar list: one row per equation, with its colour as the dot.
    pub fn sidebar_list(&self) -> SourceList {
        let items: Vec<SidebarItem> = self
            .model
            .rows
            .iter()
            .map(|row: &Row| {
                let (r, g, b) = row.rgb();
                let label = if row.text.trim().is_empty() { "Empty equation" } else { row.text.as_str() };
                SidebarItem::row(ROW_BASE + row.id, label).with_dot(pack(r, g, b, 255))
            })
            .collect();
        let mut list = SourceList::new(items, ROW_PT, 0.0);
        list.set_selected(self.model.selected.map(|id| ROW_BASE + id));
        list
    }

    // ---- hit testing ----

    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return match self.toolbar_layout(ui).hit(x, y) {
                Some(id) => Target::Button(id),
                None => Target::Nothing,
            };
        }
        if x < SIDE_PT {
            let list = self.list_rect();
            if y >= list.y + list.h {
                return Target::Nothing;
            }
            return match self.sidebar_list().hit(list, 0.0, x, y) {
                Some(id) if id >= ROW_BASE => Target::Row(id - ROW_BASE),
                _ => Target::Nothing,
            };
        }
        Target::Plot
    }

    // ---- input ----

    /// A touch in device pixels. True when the window needs a new frame.
    pub fn touch(&mut self, ui: &Ui, dir: &Path, kind: TouchKind, x: f32, y: f32) -> bool {
        let scale = ui.pt(1.0);
        let (px, py) = (x, y);
        let (x, y) = (x / scale, y / scale);
        match kind {
            TouchKind::Down => {
                self.press = self.hit(ui, x, y);
                self.last_px = (px, py);
                false
            }
            TouchKind::Move => {
                if self.press != Target::Plot {
                    return false;
                }
                let (dx, dy) = ((px - self.last_px.0) as f64, (py - self.last_px.1) as f64);
                self.last_px = (px, py);
                if dx == 0.0 && dy == 0.0 {
                    return false;
                }
                self.model.view.pan(dx, dy);
                true
            }
            TouchKind::Up => {
                let released = self.hit(ui, x, y);
                let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                if released == pressed {
                    self.activate(ui, dir, released)
                } else {
                    false
                }
            }
            TouchKind::Cancel => {
                self.press = Target::Nothing;
                false
            }
        }
    }

    /// A key: text types into the selected equation, Backspace deletes, Return starts a new equation.
    pub fn key(&mut self, dir: &Path, key: KeyInput) -> bool {
        let changed = match key {
            KeyInput::Text(text) => {
                let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                self.model.type_text(&clean)
            }
            KeyInput::Code(8) => self.model.backspace(),
            KeyInput::Code(13) => {
                self.model.add("");
                true
            }
            _ => false,
        };
        if changed {
            self.persist(dir);
        }
        changed
    }

    /// The action of a completed tap. True when the window needs a new frame.
    pub fn activate(&mut self, ui: &Ui, dir: &Path, target: Target) -> bool {
        match target {
            Target::Nothing | Target::Plot => false,
            Target::Row(id) => self.model.select(id),
            Target::Button(id) => self.button(ui, dir, id),
        }
    }

    /// Adds an empty equation and selects it, so what is typed next goes into it and not into an existing one.
    fn add_equation(&mut self, dir: &Path) -> bool {
        self.model.add("");
        self.persist(dir);
        true
    }

    fn button(&mut self, ui: &Ui, dir: &Path, id: u32) -> bool {
        let enabled = self.toolbar().items.iter().any(|item| item.id == id && item.enabled);
        if !enabled {
            return false;
        }
        match id {
            B_ADD => self.add_equation(dir),
            B_REMOVE => match self.model.selected {
                Some(selected) => {
                    self.model.remove(selected);
                    self.persist(dir);
                    true
                }
                None => false,
            },
            B_ZOOM_OUT => self.zoom(ui, 1.0 / ZOOM_STEP),
            B_ZOOM_IN => self.zoom(ui, ZOOM_STEP),
            B_RESET => {
                self.model.view = View::default();
                true
            }
            B_EXPORT => {
                self.export(ui, dir);
                true
            }
            _ => false,
        }
    }

    /// Zooms about the centre of the plot.
    fn zoom(&mut self, ui: &Ui, factor: f64) -> bool {
        let area = self.plot_area(ui);
        if area.w == 0 || area.h == 0 {
            return false;
        }
        let (w, h) = (area.w as f64, area.h as f64);
        self.model.view.zoom_at(factor, w / 2.0, h / 2.0, w, h);
        true
    }

    /// Writes the plot as a PNG into the export folder and says where it went.
    fn export(&mut self, ui: &Ui, dir: &Path) {
        let area = self.plot_area(ui);
        if area.w == 0 || area.h == 0 {
            self.message = "There is no plot to export".to_string();
            return;
        }
        let bytes = plot::export_png(ui, &mut self.labels, &self.model, area.w, area.h);
        let folder: PathBuf = dir.join(EXPORT_DIR);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let path = folder.join(format!("grapher-{stamp}.png"));
        self.message = match std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&path, bytes)) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("Export failed: {error}"),
        };
    }
}

impl Default for Grapher {
    fn default() -> Self {
        Grapher::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plot_fills_the_space_right_of_the_sidebar() {
        let mut g = Grapher::new();
        g.size = (1000.0, 700.0);
        let plot = g.plot_rect();
        assert_eq!(plot.x, SIDE_PT);
        assert_eq!(plot.y, UNIFIED_BAR_PT);
        assert_eq!(plot.w, 1000.0 - SIDE_PT);
        assert_eq!(plot.h, 700.0 - UNIFIED_BAR_PT);
    }

    #[test]
    fn the_list_stops_above_the_panel() {
        let mut g = Grapher::new();
        g.size = (1000.0, 700.0);
        let list = g.list_rect();
        assert_eq!(list.y + list.h, 700.0 - PANEL_PT);
        let field = g.field_rect();
        assert!(field.y >= 700.0 - PANEL_PT);
    }

    #[test]
    fn the_sidebar_rows_carry_the_equation_ids() {
        let mut g = Grapher::new();
        g.model.select(2);
        let list = g.sidebar_list();
        assert_eq!(list.selected(), Some(ROW_BASE + 2));
    }

    #[test]
    fn add_makes_an_empty_selected_row_that_typing_fills() {
        let dir = std::env::temp_dir().join("grapher-add-test");
        let mut g = Grapher::new();
        let before: Vec<String> = g.model.rows.iter().map(|r| r.text.clone()).collect();
        g.add_equation(&dir);
        assert_eq!(g.model.selected_row().map(|r| r.text.as_str()), Some(""));
        g.key(&dir, KeyInput::Text("y = x".into()));
        let after: Vec<String> = g.model.rows.iter().map(|r| r.text.clone()).collect();
        assert_eq!(&after[..before.len()], &before[..]);
        assert_eq!(after.last().map(String::as_str), Some("y = x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_remove_is_disabled_without_a_selection() {
        let mut g = Grapher::new();
        g.model.selected = None;
        let remove = g.toolbar().items.into_iter().find(|i| i.id == B_REMOVE).unwrap();
        assert!(!remove.enabled);
    }

    #[test]
    fn the_configured_pixel_size_becomes_points() {
        assert_eq!(crate::size_pt(1250, 750, 1.5625), (800.0, 480.0));
        assert_eq!(crate::size_pt(600, 400, 1.0), (600.0, 400.0));
    }
}
