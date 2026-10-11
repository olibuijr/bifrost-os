//! bifrost-style dark Calculator: a basic keypad with editable decimal input,
//! a scientific keypad (landscape or toggled), a persisted history,
//! clipboard copy/paste and Icelandic decimal-comma formatting.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXXL};
use bifrost_app::kit::ui::{DISPLAY_LARGE_PT, DISPLAY_MEDIUM_PT, MAC_BODY_PT, MAC_LARGE_TITLE_PT, MAC_TITLE3_PT};
use bifrost_app::kit::ui::{MAC_CALLOUT_PT, MAC_TITLE1_PT};
use bifrost_app::kit::ui::{home_zone, Ui, Weight, CARD_BG, CONTROL_PRESSED, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LABEL_3, ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, SIDEBAR_BG, WHITE};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use bifrost_app::kit::{clipboard, Event, Surface, TextCache};
use crate::engine::{Engine, Key, Operator};
use crate::eval::Angle;
use crate::format::{format_number, localize};
use crate::history::{self, Entry, History, Settings};
use crate::paste;
use crate::sci::{Sci, SciKey};
use std::path::PathBuf;
use bifrost_app::kit::ui::{SPACE_XS};

pub const ORANGE: u32 = bifrost_app::kit::ui::SYSTEM_ORANGE;
/// Key fills in the current appearance (Golden Gate Calculator: grey digit keys, lighter utility keys).
fn numeric() -> u32 { FIELD_BG.now() }
fn utility() -> u32 { CONTROL_PRESSED.now() }
fn scientific() -> u32 { SEGMENT_ON.now() }
/// A pressed grey key.
fn key_down() -> u32 { LABEL_3.now() }

/// Toolbar item ids (also the semantic control ids).
const BAR_HISTORY: u32 = 1;
const BAR_BASIC: u32 = 2;
const BAR_SCIENTIFIC: u32 = 3;
const BAR_DECIMAL: u32 = 4;

/// How long a touch on the display must stay down to open Copy/Paste.
const HOLD_MS: f64 = 500.0;
/// Key label sizes as a share of the key's height: scientific keys, utility keys, digit and operator keys.
const SCI_LABEL_EM: f32 = 0.36;
const UTILITY_LABEL_EM: f32 = 0.34;
const KEY_LABEL_EM: f32 = 0.45;
/// Semantic control ids (the toolbar uses 1 to 4 above): keypad 100 + index, then the rest of the screen.
const ID_KEY: u32 = 100;
const ID_DELETE: u32 = 200;
const ID_DISPLAY: u32 = 201;
const ID_CLEAR_HISTORY: u32 = 202;
const ID_COPY: u32 = 210;
const ID_PASTE: u32 = 211;
const ID_HISTORY: u32 = 1000;

/// Corner radius of a key or capsule: half its shorter side, so it stays a capsule at any size.
fn capsule(w: f32, h: f32) -> f32 {
    w.min(h) / 2.0
}
const SCI_COLS: usize = 6;
const SCI_ROWS: usize = 5;

const KEYS: [Key; 19] = [
    Key::Clear,
    Key::Sign,
    Key::Percent,
    Key::Operator(Operator::Divide),
    Key::Digit(7),
    Key::Digit(8),
    Key::Digit(9),
    Key::Operator(Operator::Multiply),
    Key::Digit(4),
    Key::Digit(5),
    Key::Digit(6),
    Key::Operator(Operator::Subtract),
    Key::Digit(1),
    Key::Digit(2),
    Key::Digit(3),
    Key::Operator(Operator::Add),
    Key::Digit(0),
    Key::Decimal,
    Key::Equals,
];

/// What a scientific cell does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Key(SciKey),
    Second,
    Angle,
}

/// A scientific key: its label and action, and those while 2nd is on.
#[derive(Clone, Copy)]
struct SciDef {
    label: &'static str,
    act: Act,
    label2: &'static str,
    act2: Act,
}

const fn one(label: &'static str, key: SciKey) -> SciDef {
    SciDef { label, act: Act::Key(key), label2: label, act2: Act::Key(key) }
}

const fn two(label: &'static str, key: SciKey, label2: &'static str, key2: SciKey) -> SciDef {
    SciDef { label, act: Act::Key(key), label2, act2: Act::Key(key2) }
}

const fn special(label: &'static str, act: Act) -> SciDef {
    SciDef { label, act, label2: label, act2: act }
}

const SCI_KEYS: [SciDef; SCI_COLS * SCI_ROWS] = [
    one("(", SciKey::Open),
    one(")", SciKey::Close),
    one("mc", SciKey::MemClear),
    one("m+", SciKey::MemAdd),
    one("m-", SciKey::MemSub),
    one("mr", SciKey::MemRecall),
    special("2nd", Act::Second),
    one("x²", SciKey::Postfix("^2")),
    one("x³", SciKey::Postfix("^3")),
    one("xʸ", SciKey::Postfix("^")),
    two("eˣ", SciKey::Func("exp"), "2ˣ", SciKey::Text("2^")),
    one("10ˣ", SciKey::Func("exp10")),
    one("1/x", SciKey::Postfix("^-1")),
    one("√x", SciKey::Func("√")),
    one("∛x", SciKey::Func("∛")),
    one("ʸ√x", SciKey::Postfix("^(1/")),
    one("ln", SciKey::Func("ln")),
    two("log10", SciKey::Func("log"), "log2", SciKey::Func("log2")),
    one("x!", SciKey::Postfix("!")),
    two("sin", SciKey::Func("sin"), "asin", SciKey::Func("asin")),
    two("cos", SciKey::Func("cos"), "acos", SciKey::Func("acos")),
    two("tan", SciKey::Func("tan"), "atan", SciKey::Func("atan")),
    one("e", SciKey::Text("e")),
    one("Rand", SciKey::Rand),
    special("Rad", Act::Angle),
    two("sinh", SciKey::Func("sinh"), "asinh", SciKey::Func("asinh")),
    two("cosh", SciKey::Func("cosh"), "acosh", SciKey::Func("acosh")),
    two("tanh", SciKey::Func("tanh"), "atanh", SciKey::Func("atanh")),
    one("π", SciKey::Text("π")),
    one("EE", SciKey::Exp),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Key(usize),
    Display,
    Backspace,
    History,
    Done,
    ClearHistory,
    Result(usize),
    Scientific,
    Decimal,
    MenuCopy,
    MenuPaste,
    MenuDismiss,
}

#[derive(Clone, Copy)]
struct Layout {
    x: f32,
    y: f32,
    cw: f32,
    ch: f32,
    gap: f32,
    width: f32,
    sci: bool,
}

pub struct Calculator {
    w: i32,
    h: i32,
    engine: Engine,
    sci: Sci,
    store: History,
    settings: Settings,
    settings_path: Option<PathBuf>,
    /// Engine calculations already mirrored into the history store.
    seen: u64,
    history_open: bool,
    hist_offset: usize,
    hist_start: usize,
    menu: bool,
    hold: Option<f64>,
    pressed: Option<Target>,
    down: (f32, f32),
    moved: bool,
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
}

/// Typographic operators for the scientific expression line.
fn pretty(expr: &str) -> String {
    expr.replace('*', "×").replace('/', "÷")
}

/// Shows the tail of `text` (after an ellipsis) when it is too wide.
fn fit_tail(ui: &Ui, text: &str, pt: f32, max: f32) -> String {
    if ui.measure(text, pt, Weight::Regular) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let s: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if ui.measure(&s, pt, Weight::Regular) <= max {
            return s;
        }
    }
    "…".into()
}

impl Calculator {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// A calculator that keeps nothing on disk.
    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            engine: Engine::default(),
            sci: Sci::default(),
            store: History::memory(),
            settings: Settings::default(),
            settings_path: None,
            seen: 0,
            history_open: false,
            hist_offset: 0,
            hist_start: 0,
            menu: false,
            hold: None,
            pressed: None,
            down: (0.0, 0.0),
            moved: false,
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
        }
    }

    /// Keeps the history and settings in `dir` from now on.
    pub fn with_storage(mut self, dir: PathBuf) -> Self {
        self.store = History::load(dir.join(history::HISTORY_FILE));
        let path = dir.join(history::SETTINGS_FILE);
        self.settings = Settings::load(&path);
        self.settings_path = Some(path);
        self.sci.angle = self.settings.angle;
        self
    }

    pub fn display(&self) -> &str {
        self.engine.display()
    }

    fn sci_active(&self, ui: &Ui) -> bool {
        self.w as f32 >= ui.pt(520.0) && (self.settings.scientific || self.w > self.h)
    }

    fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            self.settings.save(path);
        }
    }

    fn layout(&self, ui: &Ui) -> Layout {
        let bottom = self.h as f32 - home_zone(ui) - ui.pt(SPACE_XXL);
        let available = bottom - ui.pt(230.0);
        if self.sci_active(ui) {
            let gap = ui.pt(SPACE_S);
            let cw = ((self.w as f32 - ui.pt(48.0) - gap * 9.0) / 10.0).max(ui.pt(SPACE_XXL));
            let ch = ui
                .pt(96.0)
                .min((available - gap * 4.0) / 5.0)
                .max(ui.pt(30.0));
            let width = cw * 10.0 + gap * 9.0;
            return Layout {
                x: (self.w as f32 - width) / 2.0,
                y: bottom - ch * 5.0 - gap * 4.0,
                cw,
                ch,
                gap,
                width,
                sci: true,
            };
        }
        let gap = ui.pt(SPACE_M);
        let cell = ui
            .pt(135.0)
            .min((self.w as f32 - ui.pt(64.0) - gap * 3.0) / 4.0)
            .min((available - gap * 4.0) / 5.0)
            .max(ui.pt(30.0));
        let width = cell * 4.0 + gap * 3.0;
        Layout {
            x: (self.w as f32 - width) / 2.0,
            y: bottom - cell * 5.0 - gap * 4.0,
            cw: cell,
            ch: cell,
            gap,
            width,
            sci: false,
        }
    }

    fn key_count(&self, ui: &Ui) -> usize {
        KEYS.len() + if self.sci_active(ui) { SCI_KEYS.len() } else { 0 }
    }

    fn key_rect(&self, ui: &Ui, index: usize) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        if index >= KEYS.len() {
            let k = index - KEYS.len();
            return (
                l.x + (k % SCI_COLS) as f32 * (l.cw + l.gap),
                l.y + (k / SCI_COLS) as f32 * (l.ch + l.gap),
                l.cw,
                l.ch,
            );
        }
        let offset = if l.sci { SCI_COLS as f32 * (l.cw + l.gap) } else { 0.0 };
        let (row, col, span) = match index {
            16 => (4, 0, 2),
            17 => (4, 2, 1),
            18 => (4, 3, 1),
            i => (i / 4, i % 4, 1),
        };
        (
            l.x + offset + col as f32 * (l.cw + l.gap),
            l.y + row as f32 * (l.ch + l.gap),
            l.cw * span as f32 + l.gap * (span - 1) as f32,
            l.ch,
        )
    }

    /// The centre of a visible basic key, also useful to host previews.
    pub fn key_centre(&self, ui: &Ui, key: Key) -> Option<(f32, f32)> {
        let i = KEYS.iter().position(|&k| k == key)?;
        let (x, y, w, h) = self.key_rect(ui, i);
        Some((x + w / 2.0, y + h / 2.0))
    }

    /// The centre of a visible scientific key (by its un-2nd action).
    pub fn sci_key_centre(&self, ui: &Ui, key: SciKey) -> Option<(f32, f32)> {
        if !self.sci_active(ui) {
            return None;
        }
        let k = SCI_KEYS.iter().position(|d| d.act == Act::Key(key))?;
        let (x, y, w, h) = self.key_rect(ui, KEYS.len() + k);
        Some((x + w / 2.0, y + h / 2.0))
    }

    /// The unified toolbar: the history sidebar toggle, Basic / Scientific, and the decimal separator.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let sci = self.sci_active(ui);
        let sample = if self.settings.locale.decimal() == ',' { "1,5" } else { "1.5" };
        Toolbar::new(vec![
            ToolbarItem::icon(BAR_HISTORY, "History", "sidebar.left").selected(self.history_open),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(BAR_BASIC, "Basic").in_group(1).selected(!sci),
            ToolbarItem::button(BAR_SCIENTIFIC, "Scientific").in_group(1).selected(sci),
            ToolbarItem::button(BAR_DECIMAL, sample),
        ])
    }

    /// The toolbar's rectangle in points, right of the traffic lights.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (self.w as f32 / ui.pt(1.0) - x - SPACE_S).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The toolbar laid out as `widgets::toolbar` draws it (titled buttons measured at body size).
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        let h = widgets::CAPSULE_PT;
        let b = self.bar_bounds(ui);
        self.bar(ui).layout_with(Rect { y: b.y + (b.h - h) / 2.0, h, ..b }, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Toolbar item `id` in pixels, or an empty rectangle when it overflowed.
    fn bar_rect(&self, ui: &Ui, id: u32) -> (f32, f32, f32, f32) {
        self.bar_layout(ui).rect_of(id).map_or((0.0, 0.0, 0.0, 0.0), |r| (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)))
    }

    fn history_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, BAR_HISTORY)
    }

    /// The Scientific segment (Basic when the scientific keypad shows, so a tap toggles it).
    fn scientific_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, if self.sci_active(ui) { BAR_BASIC } else { BAR_SCIENTIFIC })
    }

    fn decimal_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, BAR_DECIMAL)
    }

    /// The name a control reader gives keypad key `index` (its label in the current second and angle state).
    fn key_name(&self, index: usize) -> String {
        if index >= KEYS.len() {
            let k = index - KEYS.len();
            return match self.sci_action(k) {
                Act::Angle => (if self.sci.angle == Angle::Deg { "Deg" } else { "Rad" }).into(),
                _ if self.sci.second => SCI_KEYS[k].label2.into(),
                _ => SCI_KEYS[k].label.into(),
            };
        }
        match KEYS[index] {
            Key::Digit(digit) => digit.to_string(),
            Key::Decimal => "Decimal".into(),
            Key::Operator(Operator::Add) => "Add".into(),
            Key::Operator(Operator::Subtract) => "Subtract".into(),
            Key::Operator(Operator::Multiply) => "Multiply".into(),
            Key::Operator(Operator::Divide) => "Divide".into(),
            Key::Equals => "Equals".into(),
            Key::Clear => (if self.engine.clear_label() == "AC" { "All Clear" } else { "Clear" }).into(),
            Key::Sign => "Change Sign".into(),
            Key::Percent => "Percent".into(),
            Key::Backspace => "Delete".into(),
        }
    }

    /// Every control on screen as a semantic node, in points: the toolbar, then either the history sidebar (its
    /// rows and Clear History) or the keypad with Delete and the display, plus the Copy and Paste items while the
    /// touch menu is open. The shell keeps the window from dragging over these.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let p = ui.pt(1.0);
        let to_points = |(x, y, w, h): (f32, f32, f32, f32)| Rect { x: x / p, y: y / p, w: w / p, h: h / p };
        let node = |id: u32, role: Role, label: String, value: String, rect: (f32, f32, f32, f32), enabled: bool| Node {
            id: id as u64,
            role,
            label,
            value,
            bounds: to_points(rect),
            enabled,
        };
        let bar = self.bar(ui);
        let mut nodes: Vec<Node> = self.bar_layout(ui).visible.iter().filter_map(|(id, r)| {
            let item = bar.items.iter().find(|i| i.id == *id)?;
            let role = if item.group == 1 { Role::SegmentedControl } else { Role::Button };
            Some(Node { id: *id as u64, role, label: item.label.clone(), value: if item.selected { "selected".into() } else { String::new() }, bounds: *r, enabled: item.enabled })
        }).collect();
        if self.menu {
            nodes.push(node(ID_COPY, Role::MenuItem, "Copy".into(), String::new(), self.menu_item(ui, false), self.can_copy(ui)));
            nodes.push(node(ID_PASTE, Role::MenuItem, "Paste".into(), String::new(), self.menu_item(ui, true), self.can_paste()));
        }
        if self.history_open {
            for visual in 0..self.rows_visible(ui) {
                let index = self.hist_offset + visual;
                let Some(entry) = self.store.get(index) else { break };
                nodes.push(node(ID_HISTORY + index as u32, Role::Row, entry.expression.clone(), entry.result.clone(), self.result_rect(ui, visual), true));
            }
            let (px, py, pw, ph) = self.panel(ui);
            nodes.push(node(ID_CLEAR_HISTORY, Role::Button, "Clear History".into(), String::new(), (px, py + ph - ui.pt(64.0), pw, ui.pt(64.0)), true));
            return nodes;
        }
        for index in 0..self.key_count(ui) {
            nodes.push(node(ID_KEY + index as u32, Role::Button, self.key_name(index), String::new(), self.key_rect(ui, index), true));
        }
        nodes.push(node(ID_DELETE, Role::Button, "Delete".into(), String::new(), self.backspace_button(ui), true));
        nodes.push(node(ID_DISPLAY, Role::StaticText, "Display".into(), self.display_text(ui), self.display_rect(ui), true));
        nodes
    }

    fn backspace_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        (
            l.x + l.width - ui.pt(105.0),
            l.y - ui.pt(55.0),
            ui.pt(105.0),
            ui.pt(44.0),
        )
    }

    fn display_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        (
            l.x,
            ui.pt(90.0),
            l.width,
            (l.y - ui.pt(145.0)).max(ui.pt(SPACE_HUGE)),
        )
    }

    fn menu_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.display_rect(ui);
        (x + w - ui.pt(236.0), y + ui.pt(SPACE_S), ui.pt(224.0), ui.pt(52.0))
    }

    fn menu_item(&self, ui: &Ui, paste: bool) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.menu_rect(ui);
        (if paste { x + w / 2.0 } else { x }, y, w / 2.0, h)
    }

    fn panel(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        // A full-height history sidebar under the traffic lights (Golden Gate Calculator's sidebar).
        (0.0, 0.0, ui.pt(280.0).min(self.w as f32 * 0.85), self.h as f32 - home_zone(ui))
    }

    fn row_height(&self, ui: &Ui) -> f32 {
        let (_, _, _, h) = self.panel(ui);
        ui.pt(84.0).min((h - ui.pt(145.0)) / 8.0).max(ui.pt(30.0))
    }

    /// How many history rows fit between the title and Clear History.
    fn rows_visible(&self, ui: &Ui) -> usize {
        let (_, _, _, h) = self.panel(ui);
        (((h - ui.pt(136.0)) / self.row_height(ui)).floor() as usize).max(1)
    }

    /// The `visual`-th visible row (0 is the top one on screen).
    fn result_rect(&self, ui: &Ui, visual: usize) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.panel(ui);
        let row = self.row_height(ui);
        (
            x + ui.pt(SPACE_L),
            y + ui.pt(72.0) + row * visual as f32,
            w - ui.pt(SPACE_XXXL),
            row,
        )
    }

    fn contains((rx, ry, rw, rh): (f32, f32, f32, f32), x: f32, y: f32) -> bool {
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y >= self.h as f32 - home_zone(ui) {
            return None;
        }
        if self.menu {
            if Self::contains(self.menu_item(ui, false), x, y) {
                return Some(Target::MenuCopy);
            }
            if Self::contains(self.menu_item(ui, true), x, y) {
                return Some(Target::MenuPaste);
            }
            return Some(Target::MenuDismiss);
        }
        if y < ui.pt(UNIFIED_BAR_PT) {
            let p = ui.pt(1.0);
            return match self.bar_layout(ui).hit(x / p, y / p)? {
                BAR_HISTORY => Some(if self.history_open { Target::Done } else { Target::History }),
                BAR_BASIC if self.sci_active(ui) => Some(Target::Scientific),
                BAR_SCIENTIFIC if !self.sci_active(ui) => Some(Target::Scientific),
                BAR_DECIMAL => Some(Target::Decimal),
                _ => None,
            };
        }
        if self.history_open {
            let (px, py, pw, ph) = self.panel(ui);
            if Self::contains((px, py + ph - ui.pt(64.0), pw, ui.pt(64.0)), x, y) {
                return Some(Target::ClearHistory);
            }
            let visible = self.rows_visible(ui);
            for v in 0..visible {
                let i = self.hist_offset + v;
                if i < self.store.len() && Self::contains(self.result_rect(ui, v), x, y) {
                    return Some(Target::Result(i));
                }
            }
            return (!Self::contains(self.panel(ui), x, y)).then_some(Target::Done);
        }
        if Self::contains(self.backspace_button(ui), x, y) {
            return Some(Target::Backspace);
        }
        for i in 0..self.key_count(ui) {
            if Self::contains(self.key_rect(ui, i), x, y) {
                return Some(Target::Key(i));
            }
        }
        Self::contains(self.display_rect(ui), x, y).then_some(Target::Display)
    }

    /// The scientific action behind `index` (0-based among scientific keys).
    fn sci_action(&self, k: usize) -> Act {
        if self.sci.second {
            SCI_KEYS[k].act2
        } else {
            SCI_KEYS[k].act
        }
    }

    fn mirror_engine_history(&mut self) {
        while self.seen < self.engine.serial {
            self.seen += 1;
            if let Some(c) = self.engine.history.first() {
                self.store.push(Entry {
                    expression: c.expression.clone(),
                    result: c.result.clone(),
                    value: c.value,
                });
            }
        }
    }

    fn sci_press(&mut self, key: SciKey) {
        if let Some(entry) = self.sci.press(key) {
            self.store.push(entry);
        }
    }

    /// A physical key uses the same arithmetic, history and scientific mapping as touch.
    pub fn hardware_key(&mut self, ui: &Ui, key: Key) {
        self.press_basic(ui, key);
        self.dirty = true;
    }

    /// Presses a basic key on whichever keypad is active.
    fn press_basic(&mut self, ui: &Ui, key: Key) {
        if !self.sci_active(ui) {
            self.engine.press(key);
            self.mirror_engine_history();
            return;
        }
        let mapped = match key {
            Key::Digit(d) => SciKey::Digit(d),
            Key::Decimal => SciKey::Decimal,
            Key::Operator(Operator::Add) => SciKey::Op('+'),
            Key::Operator(Operator::Subtract) => SciKey::Op('-'),
            Key::Operator(Operator::Multiply) => SciKey::Op('*'),
            Key::Operator(Operator::Divide) => SciKey::Op('/'),
            Key::Equals => SciKey::Equals,
            Key::Clear => SciKey::Clear,
            Key::Sign => SciKey::Sign,
            Key::Percent => SciKey::Postfix("%"),
            Key::Backspace => SciKey::Backspace,
        };
        self.sci_press(mapped);
    }

    fn press_index(&mut self, ui: &Ui, index: usize) {
        if index < KEYS.len() {
            self.press_basic(ui, KEYS[index]);
            return;
        }
        match self.sci_action(index - KEYS.len()) {
            Act::Key(k) => self.sci_press(k),
            Act::Second => self.sci.second = !self.sci.second,
            Act::Angle => {
                self.sci.angle = self.sci.angle.toggled();
                self.settings.angle = self.sci.angle;
                self.save_settings();
            }
        }
    }

    /// The raw number text for the clipboard (ASCII, no separators).
    pub fn copy_text(&self, ui: &Ui) -> Option<String> {
        if self.sci_active(ui) {
            return self.sci.current_value().map(format_number);
        }
        let text = self.engine.display();
        (text != "Error").then(|| text.trim_end_matches('.').to_string())
    }

    /// Types the number found in `text` (thousands separators allowed).
    pub fn paste_text(&mut self, ui: &Ui, text: &str) -> bool {
        let Some(value) = paste::parse(text, self.settings.locale) else {
            return false;
        };
        if self.sci_active(ui) {
            self.sci.set_value(value);
        } else {
            self.engine.paste_value(value);
        }
        true
    }

    /// Edit > Copy (Cmd+C): the same copy as the Copy item of the touch menu.
    pub fn copy(&self, ui: &Ui) {
        if let Some(text) = self.copy_text(ui) {
            clipboard::set_text(&text);
        }
    }

    /// Edit > Paste (Cmd+V): the same paste as the Paste item of the touch menu.
    pub fn paste(&mut self, ui: &Ui) -> bool {
        clipboard::text().is_some_and(|text| self.paste_text(ui, &text))
    }

    /// Whether Copy has a number to copy, and Paste a number on the clipboard (`validateMenuItem:`).
    pub fn can_copy(&self, ui: &Ui) -> bool {
        self.copy_text(ui).is_some()
    }

    pub fn can_paste(&self) -> bool {
        clipboard::text().is_some_and(|t| paste::parse(&t, self.settings.locale).is_some())
    }

    fn recall(&mut self, ui: &Ui, index: usize) {
        let Some(value) = self.store.get(index).map(|e| e.value) else {
            return;
        };
        if self.sci_active(ui) {
            self.sci.set_value(value);
        } else {
            self.engine.recall_value(value);
        }
    }

    fn clear_history(&mut self) {
        self.store.clear();
        self.engine.history.clear();
        self.hist_offset = 0;
    }

    fn display_text(&self, ui: &Ui) -> String {
        if self.sci_active(ui) {
            pretty(&self.sci.display())
        } else {
            self.engine.display().to_string()
        }
    }

    fn expression_text(&self, ui: &Ui) -> String {
        if self.sci_active(ui) {
            pretty(self.sci.expression_line())
        } else {
            self.engine.expression.clone()
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        match target {
            Target::Key(i) => self.press_index(ui, i),
            Target::Backspace => {
                if self.sci_active(ui) {
                    self.sci_press(SciKey::Backspace);
                } else {
                    self.engine.press(Key::Backspace);
                }
            }
            Target::History => {
                self.history_open = true;
                self.hist_offset = 0;
            }
            Target::Done => self.history_open = false,
            Target::ClearHistory => self.clear_history(),
            Target::Result(i) => {
                self.recall(ui, i);
                self.history_open = false;
            }
            Target::Scientific => {
                self.settings.scientific = !self.settings.scientific;
                self.save_settings();
            }
            Target::Decimal => {
                self.settings.locale = self.settings.locale.toggled();
                self.save_settings();
            }
            Target::MenuCopy => {
                self.menu = false;
                self.copy(ui);
            }
            Target::MenuPaste => {
                self.menu = false;
                self.paste(ui);
            }
            Target::MenuDismiss => self.menu = false,
            Target::Display => {}
        }
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                self.down = (x, y);
                self.moved = false;
                self.hold = (self.pressed == Some(Target::Display)).then_some(t);
                self.hist_start = self.hist_offset;
                self.dirty = true;
            }
            Event::Move(x, y) => {
                if (x - self.down.0).abs().max((y - self.down.1).abs()) > ui.pt(SPACE_M) {
                    self.moved = true;
                    self.hold = None;
                }
                if self.history_open && self.moved {
                    let delta = ((self.down.1 - y) / self.row_height(ui)).round() as isize;
                    let max = self.store.len().saturating_sub(self.rows_visible(ui)) as isize;
                    self.hist_offset = (self.hist_start as isize + delta).clamp(0, max) as usize;
                    self.dirty = true;
                }
                // Sliding out cancels a key press permanently, so a home
                // gesture cannot accidentally activate a button on release.
                if self.pressed != Some(Target::Display) && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                self.hold = None;
                let pressed = self.pressed.take();
                let dx = (x - self.down.0).abs();
                let dy = (y - self.down.1).abs();
                if pressed == Some(Target::Display) && dx > ui.pt(SPACE_HUGE) && dx > dy * 1.5 {
                    self.activate(ui, Target::Backspace);
                } else if !self.moved && pressed.is_some() && self.hit(ui, x, y) == pressed {
                    self.activate(ui, pressed.unwrap());
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        if let Some(t0) = self.hold {
            if t - t0 >= HOLD_MS && self.pressed == Some(Target::Display) && !self.moved {
                self.hold = None;
                self.pressed = None;
                self.menu = true;
                self.dirty = true;
            }
        }
        self.dirty
    }

    pub fn next_wake(&self, _t: f64) -> Option<f64> {
        self.hold.map(|t0| t0 + HOLD_MS)
    }

    pub fn describe(&self) -> String {
        format!(
            "calculator_display={:?} calculator_history={} calculator_history_open={} calculator_scientific={} calculator_locale={}",
            self.display(),
            self.store.len(),
            self.history_open,
            self.settings.scientific || self.w > self.h,
            self.settings.locale.code()
        )
    }

    fn label(
        &mut self,
        s: &mut Surface,
        ui: &Ui,
        text: &str,
        pt: f32,
        color: u32,
        rect: (f32, f32, f32, f32),
    ) {
        let (x, y, w, h) = rect;
        // SF UI does not contain the modifier-letter exponent glyphs.
        // Compose ordinary letters at a smaller size, preserving scientific notation.
        let exponent = match text {
            "xʸ" => Some(("x", "y", false)),
            "eˣ" => Some(("e", "x", false)),
            "2ˣ" => Some(("2", "x", false)),
            "10ˣ" => Some(("10", "x", false)),
            "ʸ√x" => Some(("√x", "y", true)),
            _ => None,
        };
        if let Some((base, power, leading)) = exponent {
            let base = self.cache.get(ui, base, pt, Weight::Regular, color).clone();
            let power = self.cache.get(ui, power, pt * 0.65, Weight::Regular, color);
            let left = x + (w - (base.w + power.w) as f32) / 2.;
            let top = y + (h - base.h as f32) / 2.;
            let (base_x, power_x) = if leading {
                (left + power.w as f32, left)
            } else {
                (left, left + base.w as f32)
            };
            s.blit(power, power_x as i32, (top - pt * ui.s * 0.2) as i32, 255, 255);
            s.blit(&base, base_x as i32, top as i32, 255, 255);
            return;
        }
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        s.blit(
            image,
            (x + (w - image.w as f32) / 2.0) as i32,
            (y + (h - image.h as f32) / 2.0) as i32,
            255,
            255,
        );
    }

    fn draw_key(&mut self, s: &mut Surface, ui: &Ui, index: usize) {
        let rect = self.key_rect(ui, index);
        let (x, y, w, h) = rect;
        let sci_layout = self.sci_active(ui);
        if index >= KEYS.len() {
            let k = index - KEYS.len();
            let def = SCI_KEYS[k];
            let act = self.sci_action(k);
            let second_on = act == Act::Second && self.sci.second;
            let pressed = self.pressed == Some(Target::Key(index));
            let fill = if second_on {
                LABEL.now()
            } else if pressed {
                key_down()
            } else {
                scientific()
            };
            let color = if second_on { CARD_BG.now() } else { LABEL.now() };
            s.fill_rrect(x, y, w, h, capsule(w, h), fill);
            let label = match act {
                Act::Angle => if self.sci.angle == Angle::Deg { "Deg" } else { "Rad" },
                _ if self.sci.second => def.label2,
                _ => def.label,
            };
            let pt = (h / ui.s * SCI_LABEL_EM).min(MAC_LARGE_TITLE_PT);
            self.label(s, ui, label, pt, color, rect);
            return;
        }
        let key = KEYS[index];
        let selected = !sci_layout
            && matches!(key, Key::Operator(op) if self.engine.selected_operator() == Some(op));
        let pressed = self.pressed == Some(Target::Key(index));
        let is_utility = index < 3;
        let operator = matches!(key, Key::Operator(_) | Key::Equals);
        let fill = if selected {
            LABEL.now()
        } else if operator {
            ORANGE
        } else if is_utility {
            utility()
        } else {
            numeric()
        };
        let fill = if pressed {
            if operator {
                bifrost_app::kit::ui::CALC_OPERATOR_DOWN
            } else {
                key_down()
            }
        } else {
            fill
        };
        let color = if selected {
            ORANGE
        } else if operator {
            WHITE
        } else {
            LABEL.now()
        };
        s.fill_rrect(x, y, w, h, capsule(w, h), fill);
        let clear = if sci_layout {
            self.sci.clear_label()
        } else {
            self.engine.clear_label()
        };
        let decimal = self.settings.locale.decimal().to_string();
        let label = match key {
            Key::Digit(digit) => digit.to_string(),
            Key::Decimal => decimal,
            Key::Operator(op) => op.label().into(),
            Key::Equals => "=".into(),
            Key::Clear => clear.into(),
            Key::Sign => "+/−".into(),
            Key::Percent => "%".into(),
            Key::Backspace => unreachable!(),
        };
        let pt = (h / ui.s * if is_utility { UTILITY_LABEL_EM } else { KEY_LABEL_EM }).min(DISPLAY_MEDIUM_PT);
        let rect = if key == Key::Digit(0) {
            (x, y, self.layout(ui).cw, h)
        } else {
            rect
        };
        self.label(s, ui, &label, pt, color, rect);
    }

    fn draw_history(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.panel(ui);
        let locale = self.settings.locale;
        s.fill_rect(x as usize, y as usize, w as usize, h as usize, SIDEBAR_BG.now());
        s.fill_rect((x + w) as usize, y as usize, 1, h as usize, SEPARATOR_C.now());
        if self.store.is_empty() {
            self.label(
                s,
                ui,
                "No Recent Calculations",
                MAC_TITLE3_PT,
                LABEL_2.now(),
                (x, y + ui.pt(130.0), w, ui.pt(70.0)),
            );
        }
        for visual in 0..self.rows_visible(ui) {
            let index = self.hist_offset + visual;
            let Some(entry) = self.store.get(index) else {
                break;
            };
            let expression_text = localize(&pretty(&entry.expression), locale);
            let result = localize(&entry.result, locale);
            let (rx, ry, rw, rh) = self.result_rect(ui, visual);
            if self.pressed == Some(Target::Result(index)) {
                s.fill_rrect(rx, ry, rw, rh, ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
            }
            let expression = ui.fit(&expression_text, MAC_CALLOUT_PT, rw - ui.pt(SPACE_XXL));
            let image = self
                .cache
                .get(ui, &expression, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now());
            s.blit(
                image,
                (rx + rw - ui.pt(SPACE_M) - image.w as f32) as i32,
                (ry + ui.pt(SPACE_XS)) as i32,
                255,
                255,
            );
            let image = self.cache.get(ui, &result, MAC_LARGE_TITLE_PT, Weight::Regular, LABEL.now());
            s.blit(
                image,
                (rx + rw - ui.pt(SPACE_M) - image.w as f32) as i32,
                (ry + rh - ui.pt(SPACE_XS) - image.h as f32) as i32,
                255,
                255,
            );
            s.fill_rrect(rx, ry + rh - 1.0, rw, 1.0, 0.0, SEPARATOR_C.now());
        }
        self.label(
            s,
            ui,
            "Clear History",
            MAC_BODY_PT,
            DESTRUCTIVE,
            (x, y + h - ui.pt(64.0), w, ui.pt(64.0)),
        );
    }

    fn draw_menu(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.menu_rect(ui);
        s.fill_rrect(x, y, w, h, ui.pt(m::SELECTION_RADIUS), FIELD_BG.now());
        s.fill_rrect(x + w / 2.0, y + ui.pt(SPACE_S), 1.0, h - ui.pt(SPACE_L), 0.0, SEPARATOR_C.now());
        let can_paste = clipboard::text()
            .is_some_and(|t| paste::parse(&t, self.settings.locale).is_some());
        let can_copy = self.copy_text(ui).is_some();
        let copy = self.menu_item(ui, false);
        let paste_item = self.menu_item(ui, true);
        self.label(s, ui, "Copy", MAC_BODY_PT, if can_copy { LABEL.now() } else { LABEL_3.now() }, copy);
        self.label(s, ui, "Paste", MAC_BODY_PT, if can_paste { LABEL.now() } else { LABEL_3.now() }, paste_item);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        // Cap the cache: results vary indefinitely, whereas fixed keypad
        // labels should remain cached during ordinary interaction.
        if self.cache.len() > 192 {
            self.cache.clear();
        }
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CARD_BG.now());
        let sci_on = self.sci_active(ui);
        let l = self.layout(ui);
        let locale = self.settings.locale;
        let display = localize(&self.display_text(ui), locale);
        let mut pt = DISPLAY_LARGE_PT;
        let room = l.width - ui.pt(SPACE_XL);
        while ui.measure(&display, pt, Weight::Regular) > room && pt > MAC_LARGE_TITLE_PT {
            pt -= 2.0;
        }
        let display = fit_tail(ui, &display, pt, room);
        let image = self.cache.get(ui, &display, pt, Weight::Regular, LABEL.now());
        let display_y = l.y - ui.pt(62.0) - image.h as f32;
        s.blit(
            image,
            (l.x + l.width - ui.pt(SPACE_S) - image.w as f32) as i32,
            display_y as i32,
            255,
            255,
        );
        let expression = localize(&self.expression_text(ui), locale);
        let expression = ui.fit(&expression, MAC_TITLE1_PT, l.width - ui.pt(SPACE_L));
        let image = self
            .cache
            .get(ui, &expression, MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
        s.blit(
            image,
            (l.x + l.width - ui.pt(SPACE_S) - image.w as f32) as i32,
            (display_y - image.h as f32 - ui.pt(SPACE_M)) as i32,
            255,
            255,
        );
        if sci_on && self.sci.memory != 0.0 {
            let image = self.cache.get(ui, "M", MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
            s.blit(image, (l.x + ui.pt(SPACE_S)) as i32, display_y as i32, 255, 255);
        }
        self.label(s, ui, "Delete", MAC_CALLOUT_PT, LABEL_2.now(), self.backspace_button(ui));
        for i in 0..self.key_count(ui) {
            self.draw_key(s, ui, i);
        }
        if self.history_open {
            self.draw_history(s, ui);
        }
        self.draw_bar(s, ui);
        if self.menu {
            self.draw_menu(s, ui);
        }
    }

    /// The unified toolbar over the content (the band only right of an open history sidebar).
    fn draw_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let p = ui.pt(1.0);
        let side = if self.history_open { self.panel(ui).2 / p } else { 0.0 };
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0.0, w: self.w as f32 / p - side, h: UNIFIED_BAR_PT });
        let pressed = match self.pressed {
            Some(Target::History) | Some(Target::Done) => Some(BAR_HISTORY),
            Some(Target::Decimal) => Some(BAR_DECIMAL),
            Some(Target::Scientific) => Some(if self.sci_active(ui) { BAR_BASIC } else { BAR_SCIENTIFIC }),
            _ => None,
        };
        widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), pressed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format;
    use crate::icon::icon;

    fn tap(app: &mut Calculator, ui: &Ui, x: f32, y: f32) {
        app.touch(ui, Event::Down(x, y), 0.0);
        app.touch(ui, Event::Up(x, y), 20.0);
    }

    fn tap_key(app: &mut Calculator, ui: &Ui, key: Key) {
        let (x, y) = app.key_centre(ui, key).unwrap();
        tap(app, ui, x, y);
    }

    fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn touchscreen_keys_swipe_delete_history_and_recall() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        for key in [Key::Digit(1), Key::Digit(2), Key::Digit(3)] {
            tap_key(&mut app, &ui, key);
        }
        let (x, y) = centre(app.display_rect(&ui));
        app.touch(&ui, Event::Down(x, y), 100.0);
        app.touch(&ui, Event::Move(x - ui.pt(65.0), y), 120.0);
        app.touch(&ui, Event::Up(x - ui.pt(65.0), y), 150.0);
        assert_eq!(app.display(), "12");
        for key in [Key::Operator(Operator::Add), Key::Digit(3), Key::Equals] {
            tap_key(&mut app, &ui, key);
        }
        assert_eq!(app.display(), "15");
        let (x, y) = centre(app.history_button(&ui));
        tap(&mut app, &ui, x, y);
        assert!(app.history_open);
        let (x, y) = centre(app.result_rect(&ui, 0));
        tap(&mut app, &ui, x, y);
        assert!(!app.history_open);
        for key in [
            Key::Operator(Operator::Multiply),
            Key::Digit(2),
            Key::Equals,
        ] {
            tap_key(&mut app, &ui, key);
        }
        assert_eq!(app.display(), "30");
        assert!(app.describe().contains("calculator_history=2"));
    }

    #[test]
    fn chrome_is_reserved_dragged_keys_cancel_and_rendering_settles() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        assert_eq!(app.hit(&ui, 600.0, 0.0), None);
        assert_eq!(app.hit(&ui, 600.0, 1915.0), None);
        let (x, y) = app.key_centre(&ui, Key::Digit(9)).unwrap();
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Move(0.0, 0.0), 10.0);
        app.touch(&ui, Event::Up(x, y), 20.0);
        assert_eq!(app.display(), "0");
        let mut buf = vec![0; 1200 * 1920];
        app.draw(
            &mut Surface {
                w: 1200,
                h: 1920,
                stride: 1200,
                px: &mut buf,
                y0: 0,
            },
            &ui,
            30.0,
        );
        assert!(buf.contains(&ORANGE));
        assert!(buf.contains(&WHITE));
        assert!(!app.tick(&ui, 40.0));
        assert_eq!(app.next_wake(40.0), None);
        let image = icon(96);
        assert_eq!(image.px.len(), 96 * 96);
        assert!(image.px.contains(&ORANGE));
        assert!(image.px.iter().all(|p| p >> 24 == 255));
    }

    fn landscape() -> (Ui, Calculator) {
        let ui = crate::test_ui();
        let app = Calculator::new(&ui, 1920, 1200);
        (ui, app)
    }

    fn tap_sci(app: &mut Calculator, ui: &Ui, key: SciKey) {
        let (x, y) = app.sci_key_centre(ui, key).unwrap();
        tap(app, ui, x, y);
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-calc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn landscape_shows_the_scientific_keypad_and_evaluates_expressions() {
        let (ui, mut app) = landscape();
        assert!(app.sci_active(&ui));
        assert_eq!(app.key_count(&ui), KEYS.len() + SCI_KEYS.len());
        for key in [Key::Digit(2), Key::Operator(Operator::Add), Key::Digit(3), Key::Operator(Operator::Multiply), Key::Digit(4), Key::Equals] {
            tap_key(&mut app, &ui, key);
        }
        assert_eq!(app.display_text(&ui), "14");
        assert_eq!(app.store.len(), 1);
        tap_key(&mut app, &ui, Key::Clear);
        tap_sci(&mut app, &ui, SciKey::Func("sin"));
        assert_eq!(app.display_text(&ui), "sin(");
        tap_sci(&mut app, &ui, SciKey::Text("π"));
        tap_key(&mut app, &ui, Key::Equals);
        assert_eq!(app.display_text(&ui), "0");
    }

    #[test]
    fn portrait_is_basic_until_toggled_and_the_narrow_layout_keeps_keys_in_bounds() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        assert!(!app.sci_active(&ui));
        assert!(app.sci_key_centre(&ui, SciKey::Open).is_none());
        let (x, y, w, h) = app.scientific_button(&ui);
        tap(&mut app, &ui, x + w / 2.0, y + h / 2.0);
        assert!(app.sci_active(&ui));
        for i in 0..app.key_count(&ui) {
            let (x, y, w, h) = app.key_rect(&ui, i);
            assert!(x >= 0.0 && x + w <= 1200.0 && y >= 0.0 && y + h <= 1920.0, "key {i}");
        }
        let mut narrow = Calculator::new(&ui, 400, 1200);
        narrow.settings.scientific = true;
        assert!(!narrow.sci_active(&ui));
    }

    #[test]
    fn second_toggles_inverse_functions_and_degrees_persist_in_settings() {
        let (ui, mut app) = landscape();
        let (x, y) = {
            let k = SCI_KEYS.iter().position(|d| d.act == Act::Second).unwrap();
            let (x, y, w, h) = app.key_rect(&ui, KEYS.len() + k);
            (x + w / 2.0, y + h / 2.0)
        };
        tap(&mut app, &ui, x, y);
        assert!(app.sci.second);
        let sin = SCI_KEYS.iter().position(|d| d.act == Act::Key(SciKey::Func("sin"))).unwrap();
        let (kx, ky, kw, kh) = app.key_rect(&ui, KEYS.len() + sin);
        tap(&mut app, &ui, kx + kw / 2.0, ky + kh / 2.0);
        assert_eq!(app.sci.expr, "asin(");
        let angle = SCI_KEYS.iter().position(|d| d.act == Act::Angle).unwrap();
        let (ax, ay, aw, ah) = app.key_rect(&ui, KEYS.len() + angle);
        tap(&mut app, &ui, ax + aw / 2.0, ay + ah / 2.0);
        assert_eq!(app.sci.angle, Angle::Deg);
        assert_eq!(app.settings.angle, Angle::Deg);
    }

    #[test]
    fn history_and_settings_persist_and_clear_across_restarts() {
        let dir = scratch("persist");
        let ui = crate::test_ui();
        {
            let mut app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
            for key in [Key::Digit(6), Key::Operator(Operator::Multiply), Key::Digit(7), Key::Equals] {
                tap_key(&mut app, &ui, key);
            }
            let (x, y, w, h) = app.decimal_button(&ui);
            tap(&mut app, &ui, x + w / 2.0, y + h / 2.0);
        }
        let mut app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
        assert_eq!(app.store.len(), 1);
        assert_eq!(app.store.get(0).unwrap().result, "42");
        assert_eq!(app.settings.locale, format::Locale::Is);
        let (x, y) = centre(app.history_button(&ui));
        tap(&mut app, &ui, x, y);
        let (x, y) = centre(app.result_rect(&ui, 0));
        tap(&mut app, &ui, x, y);
        assert_eq!(app.display(), "42");
        let (x, y) = centre(app.history_button(&ui));
        tap(&mut app, &ui, x, y);
        let (px, py, pw, ph) = app.panel(&ui);
        tap(&mut app, &ui, px + pw / 2.0, py + ph - ui.pt(30.0));
        assert!(app.store.is_empty());
        let app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
        assert!(app.store.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_history_scrolls_by_dragging() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        for n in 0..30 {
            app.store.push(Entry { expression: format!("{n}+1"), result: (n + 1).to_string(), value: (n + 1) as f64 });
        }
        let (x, y) = centre(app.history_button(&ui));
        tap(&mut app, &ui, x, y);
        let visible = app.rows_visible(&ui);
        assert!(visible >= 1 && visible < 30);
        let (x, y) = centre(app.result_rect(&ui, 1));
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Move(x, y - app.row_height(&ui) * 3.0), 10.0);
        app.touch(&ui, Event::Up(x, y - app.row_height(&ui) * 3.0), 20.0);
        assert_eq!(app.hist_offset, 3);
        assert!(app.history_open, "a drag is not a tap");
        let (x, y) = centre(app.result_rect(&ui, 0));
        tap(&mut app, &ui, x, y);
        assert_eq!(app.display(), "27", "row 3 is the 27th newest result");
    }

    #[test]
    fn long_press_on_the_display_opens_copy_and_paste() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        let (x, y) = centre(app.display_rect(&ui));
        app.touch(&ui, Event::Down(x, y), 1000.0);
        assert_eq!(app.next_wake(1000.0), Some(1000.0 + HOLD_MS));
        assert!(!app.tick(&ui, 1100.0) || !app.menu);
        assert!(!app.menu);
        app.tick(&ui, 1000.0 + HOLD_MS + 1.0);
        assert!(app.menu);
        assert_eq!(app.next_wake(1600.0), None);
        app.touch(&ui, Event::Up(x, y), 1700.0);
        assert!(app.menu, "releasing keeps the menu open");
        let (cx, cy) = centre(app.menu_item(&ui, false));
        assert_eq!(app.hit(&ui, cx, cy), Some(Target::MenuCopy));
        let (px, py) = centre(app.menu_item(&ui, true));
        assert_eq!(app.hit(&ui, px, py), Some(Target::MenuPaste));
        tap(&mut app, &ui, 20.0, 1000.0);
        assert!(!app.menu, "tapping elsewhere dismisses without pressing a key");
        assert_eq!(app.display(), "0");
    }

    #[test]
    fn moving_or_releasing_early_cancels_the_long_press() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        let (x, y) = centre(app.display_rect(&ui));
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Move(x - ui.pt(30.0), y), 100.0);
        assert_eq!(app.next_wake(100.0), None);
        app.tick(&ui, 2000.0);
        assert!(!app.menu);
        app.touch(&ui, Event::Up(x - ui.pt(30.0), y), 2100.0);
        app.touch(&ui, Event::Down(x, y), 3000.0);
        app.touch(&ui, Event::Up(x, y), 3100.0);
        app.tick(&ui, 4000.0);
        assert!(!app.menu);
    }

    #[test]
    fn pasting_parses_thousands_separators_and_copy_gives_the_raw_number() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        assert!(app.paste_text(&ui, "$1,234,567.5"));
        assert_eq!(app.display(), "1234567.5");
        assert_eq!(app.copy_text(&ui).as_deref(), Some("1234567.5"));
        assert!(!app.paste_text(&ui, "hello"));
        assert_eq!(app.display(), "1234567.5");
        app.engine.press(Key::Decimal);
        assert!(app.copy_text(&ui).is_some());
        tap_key(&mut app, &ui, Key::Clear);
        tap_key(&mut app, &ui, Key::Clear);
        tap_key(&mut app, &ui, Key::Operator(Operator::Add));
        assert!(app.paste_text(&ui, "0x10"));
        tap_key(&mut app, &ui, Key::Equals);
        assert_eq!(app.display(), "16");
        let (ui, mut sci) = landscape();
        assert!(sci.paste_text(&ui, "1.000,5"));
        assert_eq!(sci.sci.expr, "1000.5");
        tap_key(&mut sci, &ui, Key::Operator(Operator::Add));
        tap_key(&mut sci, &ui, Key::Digit(1));
        tap_key(&mut sci, &ui, Key::Equals);
        assert_eq!(sci.copy_text(&ui).as_deref(), Some("1001.5"));
    }

    #[test]
    fn swiping_the_display_deletes_in_scientific_mode_too() {
        let (ui, mut app) = landscape();
        for key in [Key::Digit(1), Key::Digit(2), Key::Digit(3)] {
            tap_key(&mut app, &ui, key);
        }
        let (x, y) = centre(app.display_rect(&ui));
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Move(x - ui.pt(80.0), y), 10.0);
        app.touch(&ui, Event::Up(x - ui.pt(80.0), y), 20.0);
        assert_eq!(app.display_text(&ui), "12");
    }

    #[test]
    fn the_icelandic_locale_shows_decimal_commas_and_grouping() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        let (x, y, w, h) = app.decimal_button(&ui);
        tap(&mut app, &ui, x + w / 2.0, y + h / 2.0);
        assert_eq!(app.settings.locale, format::Locale::Is);
        assert!(app.describe().contains("calculator_locale=is"));
        assert!(app.paste_text(&ui, "1.234,5"));
        assert_eq!(app.display(), "1234.5");
        assert_eq!(localize(app.display(), app.settings.locale), "1.234,5");
        let mut buf = vec![0; 1200 * 1920];
        app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
        assert!(buf.contains(&WHITE));
    }

    #[test]
    fn hardware_keys_calculate_and_edit_like_touch_keys() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        for key in [Key::Digit(2), Key::Operator(Operator::Add), Key::Digit(3), Key::Equals] {
            app.hardware_key(&ui, key);
        }
        assert_eq!(app.display(), "5");
        for key in [Key::Clear, Key::Digit(1), Key::Digit(2), Key::Backspace] {
            app.hardware_key(&ui, key);
        }
        assert_eq!(app.display(), "1");
    }

    #[test]
    fn scientific_layout_draws_and_corners_use_mobile_radii() {
        let (ui, mut app) = landscape();
        app.sci.memory = 3.0;
        app.sci.second = true;
        let mut buf = vec![0; 1920 * 1200];
        app.draw(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut buf, y0: 0 }, &ui, 0.0);
        assert!(buf.contains(&scientific()));
        assert!(buf.contains(&ORANGE));
        
    }
}
