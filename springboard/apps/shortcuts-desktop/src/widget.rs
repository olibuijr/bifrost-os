//! Shortcuts: a grid of coloured tiles that run built-in actions (Wi-Fi, Low
//! Power Mode, Do Not Disturb, brightness, open an app, screenshot, timer),
//! and an editor that shows a shortcut as a list of actions.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CALLOUT_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT};
use crate::model;
use bifrost_app::kit::ui::{
    Ui, Weight, CARD_BG, CARD_RADIUS_PT, CELL_RADIUS_PT, CONTROL_PRESSED, DETAIL_BG, LABEL, LABEL_2, LABEL_3, LINK,
    MAC_TITLE3_PT, PRESSED, ROW_PRESSED, SEPARATOR_C, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK,
    SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW, TOAST_FILL, WHITE,
};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect as PtRect, Role};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::kit::{Event, Surface, TextCache};
use crate::model::{Action, Shortcut, KINDS, MAX_ACTIONS, MAX_SHORTCUTS};
use std::path::PathBuf;

/// Tile colours a shortcut can take, as the Shortcuts app offers them.
pub const PALETTE: [u32; 8] =
    [SYSTEM_RED, SYSTEM_ORANGE, SYSTEM_YELLOW, SYSTEM_GREEN, SYSTEM_TEAL, SYSTEM_BLUE, SYSTEM_PURPLE, SYSTEM_PINK];

type Rect = (f32, f32, f32, f32);

/// Sidebar width in points (Shortcuts' Library column).
const SIDEBAR_W: f32 = 200.0;
/// Grid tile: the narrowest a tile may get and its height, in points.
const TILE_MIN_W: f32 = 200.0;
const TILE_H: f32 = 100.0;
/// The glyph on the red Remove badge: white is text on the accent here.
const ON_BADGE: u32 = WHITE;
/// Sidebar row id of All Shortcuts.
const ROW_ALL: u32 = 100;
/// Toolbar item ids; they double as control ids.
const BAR_BACK: u32 = 1;
const BAR_TITLE: u32 = 2;
const BAR_ADD: u32 = 3;
const BAR_EDIT: u32 = 4;
const BAR_RUN: u32 = 5;
/// The widest the toolbar title may get, in points.
const TITLE_MAX_W: f32 = 320.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Grid,
    Editor(usize),
    Picker(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Tile(usize),
    Create,
    EditToggle,
    Back,
    Run,
    Remove(usize),
    Dec(usize),
    Inc(usize),
    AddAction,
    Pick(usize),
    Color(usize),
    Delete,
    /// All Shortcuts in the sidebar: back to the grid from any view.
    Home,
}

enum Look {
    Tile { name: String, color: u32, count: usize },
    CreateTile,
    Header(String),
    Row { text: String, value: String, tint: u32, pos: u8, chevron: bool, centre: bool },
    Minus,
    Stepper(bool),
    Swatch { color: u32, on: bool },
}

struct Cell {
    target: Option<Target>,
    rect: Rect,
    look: Look,
}

pub struct Shortcuts {
    w: i32,
    h: i32,
    path: PathBuf,
    list: Vec<Shortcut>,
    view: View,
    edit_mode: bool,
    scroll: f32,
    pressed: Option<Target>,
    down_y: f32,
    last_y: f32,
    dragging: bool,
    runs: Vec<Action>,
    toast: Option<(String, f64)>,
    now: f64,
    dirty: bool,
    cache: TextCache,
}

const ROW_POS_FIRST: u8 = 1;
const ROW_POS_LAST: u8 = 2;

impl Shortcuts {
    pub fn new(_ui: &Ui, w: i32, h: i32, path: PathBuf) -> Self {
        let mut list = model::load(&path);
        if list.is_empty() && !path.exists() {
            list = model::defaults();
        }
        Self {
            w,
            h,
            path,
            list,
            view: View::Grid,
            edit_mode: false,
            scroll: 0.0,
            pressed: None,
            down_y: 0.0,
            last_y: 0.0,
            dragging: false,
            runs: Vec::new(),
            toast: None,
            now: 0.0,
            dirty: true,
            cache: TextCache::default(),
        }
    }

    /// The list as the window holds it now. Intents and scripts read it, so they never see a stale copy.
    pub fn list(&self) -> &[Shortcut] {
        &self.list
    }

    /// Runs `f` on a copy of the list and keeps the copy, and saves it, only when `f` changed it. Intents and
    /// scripts change the list through here, so the window and the saved file stay the same.
    pub fn edit_list<T>(&mut self, f: impl FnOnce(&mut Vec<Shortcut>) -> T) -> T {
        let mut next = self.list.clone();
        let out = f(&mut next);
        if next != self.list {
            self.list = next;
            self.dirty = true;
            self.save();
        }
        out
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    /// An external host can report an unavailable run instead of showing success.
    pub fn show_run_unavailable(&mut self, t: f64) {
        self.toast = Some(("This shortcut cannot run on this device.".into(), t + 3000.0));
        self.dirty = true;
    }

    /// The actions the last tap asked the shell to run, oldest first.
    pub fn take_runs(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.runs)
    }

    fn top(ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT)
    }

    /// Left edge of the content column, in pixels.
    fn side(ui: &Ui) -> f32 {
        ui.pt(SIDEBAR_W)
    }

    fn sidebar(&self) -> SourceList {
        let items = vec![
            SidebarItem::header("Library"),
            SidebarItem::row(ROW_ALL, "All Shortcuts").with_symbol("square.grid.2x2").with_count(self.list.len() as u32),
        ];
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(ROW_ALL));
        list
    }

    fn bar_bounds(&self, ui: &Ui) -> PtRect {
        let x = SIDEBAR_W + 12.0;
        let h = widgets::CAPSULE_PT;
        PtRect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.s - x - 16.0).max(0.0), h }
    }

    fn title(&self) -> String {
        match self.view {
            View::Grid => "All Shortcuts".into(),
            View::Editor(i) => self.list.get(i).map(|s| s.name.clone()).unwrap_or_default(),
            View::Picker(_) => "Add Action".into(),
        }
    }

    /// The unified toolbar (`NativeToolbar`): back, title, then the view's commands.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let room = (self.bar_bounds(ui).w - 5.0 * widgets::CAPSULE_PT - 4.0 * widgets::TOOLBAR_GAP_PT).clamp(40.0, TITLE_MAX_W);
        let mut items = vec![];
        if self.view != View::Grid {
            items.push(ToolbarItem::icon(BAR_BACK, "Back", "chevron.left").in_group(1));
        }
        items.push(ToolbarItem::custom(BAR_TITLE, "Title", room));
        items.push(ToolbarItem::flexible_space());
        match self.view {
            View::Grid => {
                items.push(ToolbarItem::icon(BAR_ADD, "New Shortcut", "plus").in_group(2).enabled(self.list.len() < MAX_SHORTCUTS));
                items.push(ToolbarItem::button(BAR_EDIT, if self.edit_mode { "Done" } else { "Edit" }).selected(self.edit_mode));
            }
            View::Editor(_) => items.push(ToolbarItem::button(BAR_RUN, "Run")),
            View::Picker(_) => {}
        }
        Toolbar::new(items)
    }

    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        self.bar(ui).layout_with(self.bar_bounds(ui), |_| widgets::CAPSULE_PT, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    fn bar_target(id: u32) -> Option<Target> {
        match id {
            BAR_BACK => Some(Target::Back),
            BAR_ADD => Some(Target::Create),
            BAR_EDIT => Some(Target::EditToggle),
            BAR_RUN => Some(Target::Run),
            _ => None,
        }
    }

    /// The toolbar buttons as controls (points), so the shell keeps them clickable in the drag band.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        self.bar_layout(ui)
            .visible
            .iter()
            .filter(|(id, _)| *id != BAR_TITLE)
            .filter_map(|(id, r)| {
                let it = bar.items.iter().find(|i| i.id == *id)?;
                Some(Node { id: *id as u64, role: Role::Button, label: it.label.clone(), value: String::new(), bounds: *r, enabled: it.enabled })
            })
            .collect()
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = model::save(&self.path, &self.list) {
            eprintln!("springboard: shortcuts: save: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(&self, ui: &Ui, cells: &mut Vec<Cell>, y: &mut f32, text: &str, value: &str, tint: u32, pos: u8, target: Option<Target>) -> Rect {
        let (m, x0) = (ui.pt(SPACE_XL), Self::side(ui));
        let rect = (x0 + m, *y, self.w as f32 - x0 - 2.0 * m, ui.pt(44.0));
        cells.push(Cell {
            target,
            rect,
            look: Look::Row { text: text.into(), value: value.into(), tint, pos, chevron: false, centre: false },
        });
        *y += rect.3;
        rect
    }

    /// Every drawn and tappable thing in screen coordinates, plus the content bottom.
    fn layout(&self, ui: &Ui) -> (Vec<Cell>, f32) {
        let (m, top) = (ui.pt(SPACE_XL), Self::top(ui));
        let x0 = Self::side(ui);
        let w = self.w as f32 - x0;
        let m0 = m;
        let m = x0 + m0;
        let mut cells = Vec::new();
        let mut y = top - self.scroll;
        match self.view {
            View::Grid => {
                y += ui.pt(SPACE_XL);
                let gap = ui.pt(SPACE_L);
                let cols = (((w - 2.0 * m0 + gap) / (ui.pt(TILE_MIN_W) + gap)).floor() as usize).max(2);
                let tw = (w - 2.0 * m0 - gap * (cols - 1) as f32) / cols as f32;
                let th = ui.pt(TILE_H);
                let total = self.list.len() + 1;
                for i in 0..total {
                    let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
                    let rect = (m + cx * (tw + gap), y + cy * (th + gap), tw, th);
                    if let Some(s) = self.list.get(i) {
                        let look = Look::Tile { name: s.name.clone(), color: PALETTE[s.color % PALETTE.len()], count: s.actions.len() };
                        cells.push(Cell { target: Some(Target::Tile(i)), rect, look });
                    } else {
                        cells.push(Cell { target: Some(Target::Create), rect, look: Look::CreateTile });
                    }
                }
                y += (total.div_ceil(cols)) as f32 * (th + gap);
            }
            View::Editor(i) => {
                let Some(sc) = self.list.get(i) else { return (cells, y) };
                y += ui.pt(SPACE_L);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Actions".into()) });
                y += ui.pt(SPACE_XXXL);
                let n = sc.actions.len();
                for (k, a) in sc.actions.iter().enumerate() {
                    let pos = (if k == 0 { ROW_POS_FIRST } else { 0 }) | if k + 1 == n { ROW_POS_LAST } else { 0 };
                    let value = if a.adjustable() { a.value() } else { String::new() };
                    let r = self.row(ui, &mut cells, &mut y, a.title(), &value, LABEL.now(), pos, None);
                    cells.push(Cell { target: Some(Target::Remove(k)), rect: (r.0, r.1, ui.pt(44.0), r.3), look: Look::Minus });
                    if a.adjustable() {
                        let bw = ui.pt(44.0);
                        cells.push(Cell { target: Some(Target::Inc(k)), rect: (r.0 + r.2 - bw, r.1, bw, r.3), look: Look::Stepper(true) });
                        cells.push(Cell { target: Some(Target::Dec(k)), rect: (r.0 + r.2 - 2.0 * bw, r.1, bw, r.3), look: Look::Stepper(false) });
                    }
                }
                if n < MAX_ACTIONS {
                    let pos = ROW_POS_LAST | if n == 0 { ROW_POS_FIRST } else { 0 };
                    self.row(ui, &mut cells, &mut y, "Add Action", "", SYSTEM_BLUE, pos, Some(Target::AddAction));
                }
                y += ui.pt(35.0) - ui.pt(SPACE_S);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Color".into()) });
                y += ui.pt(SPACE_XXXL);
                let sw = ui.pt(SPACE_XXXL);
                let step = (w - 2.0 * m0 - ui.pt(SPACE_XXL)) / PALETTE.len() as f32;
                cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(56.0)), look: Look::Row { text: String::new(), value: String::new(), tint: LABEL.now(), pos: ROW_POS_FIRST | ROW_POS_LAST, chevron: false, centre: false } });
                for (c, color) in PALETTE.iter().enumerate() {
                    let cx = m + ui.pt(SPACE_M) + step * c as f32 + (step - sw) / 2.0;
                    cells.push(Cell { target: Some(Target::Color(c)), rect: (cx, y + ui.pt(SPACE_M), sw, sw), look: Look::Swatch { color: *color, on: sc.color % PALETTE.len() == c } });
                }
                y += ui.pt(56.0) + ui.pt(35.0);
                let r = self.row(ui, &mut cells, &mut y, "Delete Shortcut", "", SYSTEM_RED, ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Delete));
                if let Some(Cell { look: Look::Row { centre, .. }, .. }) = cells.iter_mut().rev().find(|c| c.rect == r) {
                    *centre = true;
                }
            }
            View::Picker(_) => {
                y += ui.pt(SPACE_L);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Built-in Actions".into()) });
                y += ui.pt(SPACE_XXXL);
                for (k, name) in KINDS.iter().enumerate() {
                    let pos = (if k == 0 { ROW_POS_FIRST } else { 0 }) | if k + 1 == KINDS.len() { ROW_POS_LAST } else { 0 };
                    let r = self.row(ui, &mut cells, &mut y, name, "", LABEL.now(), pos, Some(Target::Pick(k)));
                    if let Some(Cell { look: Look::Row { chevron, .. }, .. }) = cells.iter_mut().rev().find(|c| c.rect == r) {
                        *chevron = true;
                    }
                }
            }
        }
        (cells, y + self.scroll)
    }

    fn max_scroll(&self, ui: &Ui) -> f32 {
        let (_, bottom) = self.layout(ui);
        (bottom + ui.pt(SPACE_XXL) - self.h as f32).max(0.0)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y < Self::top(ui) {
            return self.bar_layout(ui).hit(x / ui.s, y / ui.s).filter(|id| {
                self.bar(ui).items.iter().any(|i| i.id == *id && i.enabled)
            }).and_then(Self::bar_target);
        }
        if x < Self::side(ui) {
            let list = self.sidebar();
            let sy = y / ui.s + UNIFIED_BAR_PT + 4.0 - list.header_h;
            return (sy >= 0.0 && sy < list.row_h && self.view != View::Grid).then_some(Target::Home);
        }
        let (cells, _) = self.layout(ui);
        cells
            .iter()
            .rev()
            .filter(|c| c.target.is_some())
            .find(|c| {
                let (cx, cy, cw, ch) = c.rect;
                x >= cx && x < cx + cw && y >= cy && y < cy + ch
            })
            .and_then(|c| c.target)
    }

    fn run_all(&mut self, i: usize) {
        let Some(s) = self.list.get(i) else { return };
        let msg = if s.actions.is_empty() { format!("{} has no actions", s.name) } else { format!("Ran {}", s.name) };
        self.runs.extend(s.actions.iter().cloned());
        self.toast = Some((msg, self.now + 2000.0));
    }

    fn editing(&mut self) -> Option<&mut Shortcut> {
        match self.view {
            View::Editor(i) | View::Picker(i) => self.list.get_mut(i),
            View::Grid => None,
        }
    }

    fn activate(&mut self, t: Target) {
        match t {
            Target::Tile(i) if self.edit_mode => {
                self.view = View::Editor(i);
                self.scroll = 0.0;
            }
            Target::Tile(i) => self.run_all(i),
            Target::Create => {
                if self.list.len() < MAX_SHORTCUTS {
                    let name = (1..).map(|n| format!("Shortcut {n}")).find(|n| self.list.iter().all(|s| &s.name != n)).unwrap_or_default();
                    let color = self.list.len() % PALETTE.len();
                    self.list.push(Shortcut { name, color, actions: Vec::new() });
                    self.view = View::Editor(self.list.len() - 1);
                    self.scroll = 0.0;
                    self.save();
                }
            }
            Target::Home => {
                self.view = View::Grid;
                self.scroll = 0.0;
            }
            Target::EditToggle => self.edit_mode = !self.edit_mode,
            Target::Back => {
                self.view = match self.view {
                    View::Picker(i) => View::Editor(i),
                    _ => View::Grid,
                };
                self.scroll = 0.0;
            }
            Target::Run => {
                if let View::Editor(i) = self.view {
                    self.run_all(i);
                }
            }
            Target::Remove(k) => {
                if let Some(s) = self.editing().filter(|s| k < s.actions.len()) {
                    s.actions.remove(k);
                    self.save();
                }
            }
            Target::Dec(k) | Target::Inc(k) => {
                if let Some(a) = self.editing().and_then(|s| s.actions.get_mut(k)) {
                    a.step(matches!(t, Target::Inc(_)));
                    self.save();
                }
            }
            Target::AddAction => {
                if let View::Editor(i) = self.view {
                    self.view = View::Picker(i);
                    self.scroll = 0.0;
                }
            }
            Target::Pick(k) => {
                if let Some(s) = self.editing() {
                    s.actions.push(Action::of_kind(k));
                }
                if let View::Picker(i) = self.view {
                    self.view = View::Editor(i);
                }
                self.save();
            }
            Target::Color(c) => {
                if let Some(s) = self.editing() {
                    s.color = c;
                    self.save();
                }
            }
            Target::Delete => {
                if let View::Editor(i) = self.view {
                    if i < self.list.len() {
                        self.list.remove(i);
                    }
                    self.view = View::Grid;
                    self.scroll = 0.0;
                    self.save();
                }
            }
        }
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) -> bool {
        self.now = t;
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                self.down_y = y;
                self.last_y = y;
                self.dragging = false;
                self.dirty = true;
            }
            Event::Move(_, y) => {
                if !self.dragging && (y - self.down_y).abs() > ui.pt(SPACE_M) {
                    self.dragging = true;
                    self.pressed = None;
                }
                if self.dragging {
                    let max = self.max_scroll(ui);
                    self.scroll = (self.scroll - (y - self.last_y)).clamp(0.0, max);
                    self.dirty = true;
                }
                self.last_y = y;
            }
            Event::Up(x, y) => {
                if let Some(p) = self.pressed.take() {
                    if !self.dragging && self.hit(ui, x, y) == Some(p) {
                        self.activate(p);
                    }
                }
                self.dragging = false;
                self.dirty = true;
            }
            _ => {}
        }
        true
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        self.now = t;
        if matches!(&self.toast, Some((_, until)) if t >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, _t: f64) -> Option<f64> {
        self.toast.as_ref().map(|(_, until)| *until)
    }

    pub fn describe(&self) -> String {
        format!("shortcuts={} shortcuts_view={:?}", self.list.len(), self.view)
    }

    fn put(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32, h: f32, align: i8) {
        let img = self.cache.get(ui, text, pt, weight, color);
        let ix = match align {
            0 => x,
            1 => x - img.w as f32 / 2.0,
            _ => x - img.w as f32,
        };
        s.blit(img, ix as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        if self.cache.len() > 128 {
            self.cache.clear();
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(ui));
        let (cells, _) = self.layout(ui);
        let (top, x0) = (Self::top(ui), Self::side(ui));
        s.fill_rect(0, 0, self.w as usize, self.h as usize, DETAIL_BG.now());
        let r = ui.pt(CELL_RADIUS_PT);
        for c in cells.iter() {
            let (x, y, cw, ch) = c.rect;
            if y + ch < top || y > self.h as f32 {
                continue;
            }
            let pressed = c.target.is_some() && c.target == self.pressed;
            match &c.look {
                Look::Header(t) => self.put(s, ui, t, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y, ch, 0),
                Look::Tile { name, color, count } => {
                    s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), *color);
                    if pressed {
                        s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), PRESSED);
                    }
                    let (gx, gy, g) = (x + ui.pt(SPACE_L), y + ui.pt(SPACE_L), ui.pt(SPACE_XXL));
                    s.stroke_rrect(gx, gy, g, g, g * 0.28, ui.pt(SPACE_XXS), WHITE);
                    s.stroke_rrect(gx + g * 0.4, gy + g * 0.4, g, g, g * 0.28, ui.pt(SPACE_XXS), WHITE);
                    if self.edit_mode {
                        self.put(s, ui, "Edit", MAC_FOOTNOTE_PT, Weight::Semibold, WHITE, x + cw - ui.pt(SPACE_L), y + ui.pt(SPACE_M), ui.pt(SPACE_XL), 2);
                    } else if *count > 0 {
                        let n = format!("{count} {}", if *count == 1 { "action" } else { "actions" });
                        self.put(s, ui, &n, MAC_CAPTION1_PT, Weight::Regular, WHITE, x + cw - ui.pt(SPACE_L), y + ui.pt(SPACE_M), ui.pt(SPACE_XL), 2);
                    }
                    let fitted = ui.fit(name, MAC_BODY_PT, cw - ui.pt(28.0));
                    self.put(s, ui, &fitted, MAC_BODY_PT, Weight::Semibold, WHITE, x + ui.pt(SPACE_L), y + ch - ui.pt(SPACE_HUGE), ui.pt(28.0), 0);
                }
                Look::CreateTile => {
                    s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), if pressed { CONTROL_PRESSED.now() } else { CARD_BG.now() });
                    self.put(s, ui, "+", MAC_TITLE1_PT, Weight::Regular, SYSTEM_BLUE, x + ui.pt(SPACE_L), y + ui.pt(SPACE_S), ui.pt(SPACE_HUGE), 0);
                    self.put(s, ui, "Create Shortcut", MAC_BODY_PT, Weight::Semibold, LABEL.now(), x + ui.pt(SPACE_L), y + ch - ui.pt(SPACE_HUGE), ui.pt(28.0), 0);
                }
                Look::Row { text, value, tint, pos, chevron, centre } => {
                    let fill = if pressed { ROW_PRESSED.now() } else { CARD_BG.now() };
                    if pos & ROW_POS_FIRST != 0 && pos & ROW_POS_LAST != 0 {
                        s.fill_rrect(x, y, cw, ch, r, fill);
                    } else if pos & ROW_POS_FIRST != 0 {
                        s.fill_rrect(x, y, cw, ch + r, r, fill);
                    } else if pos & ROW_POS_LAST != 0 {
                        s.fill_rrect(x, y - r, cw, ch + r, r, fill);
                    } else {
                        s.fill_rect(x as usize, y.max(0.0) as usize, cw as usize, ch as usize, fill);
                    }
                    if pos & ROW_POS_LAST == 0 {
                        let sx = x + ui.pt(46.0);
                        s.fill_rect(sx as usize, (y + ch - 1.0).max(0.0) as usize, (x + cw - sx) as usize, 1, SEPARATOR_C.now());
                    }
                    if *centre {
                        self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, *tint, x + cw / 2.0, y, ch, 1);
                    } else {
                        let tx = if text == "Add Action" || *chevron || matches!(c.target, Some(Target::Pick(_))) { x + ui.pt(SPACE_L) } else { x + ui.pt(46.0) };
                        self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, *tint, tx, y, ch, 0);
                    }
                    if !value.is_empty() {
                        self.put(s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + cw - ui.pt(96.0), y, ch, 2);
                    }
                    if *chevron {
                        self.put(s, ui, ">", MAC_BODY_PT, Weight::Semibold, LABEL_3.now(), x + cw - ui.pt(SPACE_L), y, ch, 2);
                    }
                }
                Look::Minus => {
                    let (d, cx, cy) = (ui.pt(SPACE_XXL), x + ui.pt(SPACE_XXL), y + ch / 2.0);
                    s.fill_rrect(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, SYSTEM_RED);
                    let bar = ui.pt(SPACE_XXS);
                    s.fill_rrect(cx - d * 0.28, cy - bar / 2.0, d * 0.56, bar, bar / 2.0, ON_BADGE);
                }
                Look::Stepper(up) => {
                    let col = if pressed { LABEL_2.now() } else { LINK };
                    self.put(s, ui, if *up { "+" } else { "\u{2212}" }, MAC_TITLE1_PT, Weight::Regular, col, x + cw / 2.0, y, ch, 1);
                }
                Look::Swatch { color, on } => {
                    if *on {
                        s.stroke_rrect(x - 4.0, y - 4.0, cw + 8.0, ch + 8.0, (cw + 8.0) / 2.0, ui.pt(SPACE_XXS), LABEL.now());
                    }
                    s.fill_rrect(x, y, cw, ch, cw / 2.0, *color);
                }
            }
        }
        // the unified toolbar over the content, then the sidebar beside it
        let wp = self.w as f32 / ui.s;
        widgets::toolbar_band(s, ui, PtRect { x: SIDEBAR_W, y: 0.0, w: wp - SIDEBAR_W, h: UNIFIED_BAR_PT });
        let pressed = self.bar(ui).items.iter().map(|i| i.id).find(|id| Self::bar_target(*id).is_some() && Self::bar_target(*id) == self.pressed);
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), pressed);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let fitted = ui.fit(&self.title(), MAC_TITLE3_PT, ui.pt(r.w));
            self.put(s, ui, &fitted, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), ui.pt(r.x), ui.pt(r.y), ui.pt(r.h), 0);
        }
        widgets::source_list(s, ui, PtRect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.s }, &self.sidebar(), -(UNIFIED_BAR_PT + 4.0), None);
        s.fill_rect(x0 as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        if let Some((msg, _)) = self.toast.clone() {
            let img = self.cache.get(ui, &msg, MAC_CALLOUT_PT, Weight::Regular, WHITE);
            let pw = img.w as f32 + ui.pt(SPACE_XXXL);
            let ph = img.h as f32 + ui.pt(SPACE_L);
            let x = x0 + (self.w as f32 - x0 - pw) / 2.0;
            let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
            s.fill_rrect(x, y, pw, ph, 1.0e4, TOAST_FILL);
            s.blit(img, (x + ui.pt(SPACE_L)) as i32, (y + ui.pt(SPACE_S)) as i32, 255, 255);
        }
    }
}
