//! bifrost-style dark Calculator: a basic keypad with editable decimal input,
//! a scientific keypad (landscape or toggled), a persisted history,
//! clipboard copy/paste and Icelandic decimal-comma formatting.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{DISPLAY_LARGE_PT, DISPLAY_MEDIUM_PT, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT};
use bifrost_app::kit::ui::{MAC_CALLOUT_PT, MAC_TITLE1_PT};
use bifrost_app::kit::ui::{home_zone, Ui, Weight, CARD_BG, CONTROL_PRESSED, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LABEL_3, SEGMENT_ON, SEPARATOR_C, WHITE};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::proto::ContextItem;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::{clipboard, Event, TextCache};
use crate::chrome;
use crate::convert::{self, Category, Converter};
use crate::rates::Rates;
use crate::copy_service::CopyRequest;
use crate::engine::{Engine, Key, Operator};
use crate::eval::Angle;
use crate::format::{format_number, format_styled, localize_with};
use crate::history::{self, Entry, History, Settings};
use crate::paste;
use crate::programmer::{Base, BinOp, PKey, Programmer};
use crate::rpn::{RKey, Rpn};
use crate::sci::{Sci, SciKey};
use crate::tape;
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
const BAR_CONVERT: u32 = 5;
const BAR_PROGRAMMER: u32 = 6;
/// The keypad pull-down a compact window shows instead of the four segments (Golden Gate Calculator's mode button).
const BAR_MODE: u32 = 7;

/// The window's content size for each keypad, in points: Golden Gate Calculator resizes its window when the keypad
/// changes (27-Golden-Gate-Calculator.png: the Basic window is 230 points wide, four 48-point keys 6 points apart and
/// 10 points in from the sides). The heights leave the display, the expression line and the Delete control room above
/// the keys (`DISPLAY_AND_KEYS_RESERVE_PT`).
pub const BASIC_SIZE_PT: (f32, f32) = (230.0, 512.0);
/// Ten 44-point columns (`wide_min_width` needs 520).
pub const SCIENTIFIC_SIZE_PT: (f32, f32) = (560.0, 522.0);
/// The scientific width with the binary row (`PROG_STRIP_PT`) above the keys.
pub const PROGRAMMER_SIZE_PT: (f32, f32) = (560.0, 522.0 + PROG_STRIP_PT);
/// Wide enough for seven category titles across the band, tall enough for the band over the keypad.
pub const CONVERT_SIZE_PT: (f32, f32) = (640.0, 660.0);
/// The width from which the basic keypad keeps its full margins (32-point sides, the home strip under it); a
/// narrower window scales them down, to the reference's 10-point sides at 230 points.
const FULL_MARGIN_W_PT: f32 = 736.0;

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
const ID_COPY_TAPE: u32 = 203;
const ID_COPY: u32 = 210;
const ID_PASTE: u32 = 211;
const ID_CATEGORY: u32 = 320;
const ID_FROM: u32 = 310;
const ID_SWAP: u32 = 311;
const ID_TO: u32 = 312;
/// The clickable bits of Programmer mode: 400 + bit.
const ID_BIT: u32 = 400;
const ID_HISTORY: u32 = 1000;

/// Height of the binary row above the Programmer keypad, in points.
const PROG_STRIP_PT: f32 = 64.0;
/// A wide keypad needs ten 40-point keys, their nine gaps and side insets.
const WIDE_KEY_PT: f32 = 40.0;
const WIDE_INSET_PT: f32 = 48.0;
const WIDE_COLUMNS: f32 = 10.0;
const WIDE_GAPS: f32 = 9.0;
/// Vertical room reserved for the display and the keypad's controls before sizing the keys.
const DISPLAY_AND_KEYS_RESERVE_PT: f32 = 230.0;
/// Backspace is a compact trailing control sized to fit a wide keypad's final cell cluster.
const BACKSPACE_CONTROL_WIDTH_PT: f32 = 105.0;
/// The display's minimum breathing room above the keypad deck.
const DISPLAY_DECK_INSET_PT: f32 = 145.0;
/// Context menu geometry for the two display actions.
const DISPLAY_MENU_TRAILING_INSET_PT: f32 = 236.0;
const DISPLAY_MENU_WIDTH_PT: f32 = 224.0;
const DISPLAY_MENU_HEIGHT_PT: f32 = 52.0;
/// Maximum history row height and the room reserved for its heading and footer controls.
const HISTORY_ROW_MAX_PT: f32 = 84.0;
const HISTORY_CHROME_RESERVE_PT: f32 = 145.0;
const HISTORY_VIEWPORT_RESERVE_PT: f32 = 136.0;

/// The label of Clear History after one tap: a second tap clears.
const CLEAR_CONFIRM_LABEL: &str = "Tap to Confirm";

/// Convert's two rows (categories, then units), in points, between the toolbar and the entry line.
const CATEGORY_TOP_PT: f32 = 60.0;
const CATEGORY_H_PT: f32 = 26.0;
/// Category segments per row; the thirteen categories take two rows.
const CATEGORY_COLS: usize = 7;
const UNITS_H_PT: f32 = 26.0;
/// The rates line under the units, shown for Currency only.
const RATES_NOTE_H_PT: f32 = 16.0;

fn category_rows() -> usize {
    (Category::ALL.len() + CATEGORY_COLS - 1) / CATEGORY_COLS
}

/// Top of the unit row: under the category rows, with a gap.
fn units_top_pt() -> f32 {
    CATEGORY_TOP_PT + category_rows() as f32 * CATEGORY_H_PT + 8.0
}

/// Height of Convert's whole band (both category rows, the gap and the unit row, plus a gap to the display), in
/// points: the keys shrink by this so the display never runs under the capsules (E2E W4-7).
fn convert_band_pt() -> f32 {
    units_top_pt() + UNITS_H_PT + SPACE_M - UNIFIED_BAR_PT
}

/// A line of text of `pt` points is about this many times `pt` tall (ascent plus descent).
const LINE_EM: f32 = 1.25;

/// The widest Convert band, in points: seven categories of up to 120 points each.
const CONVERT_MAX_W_PT: f32 = 840.0;

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
    two("mr", SciKey::MemRecall, "ans", SciKey::Answer),
    special("2nd", Act::Second),
    one("x²", SciKey::Postfix("^2")),
    one("x³", SciKey::Postfix("^3")),
    one("xʸ", SciKey::Postfix("^")),
    two("eˣ", SciKey::Func("exp"), "yˣ", SciKey::Postfix("^")),
    two("10ˣ", SciKey::Func("exp10"), "2ˣ", SciKey::Text("2^")),
    one("1/x", SciKey::Postfix("^-1")),
    one("√x", SciKey::Func("√")),
    one("³√x", SciKey::Func("cbrt")),
    two("ʸ√x", SciKey::Postfix("^(1/"), "ˣ√y", SciKey::Postfix(" rootx ")),
    two("ln", SciKey::Func("ln"), "logy", SciKey::Postfix(" logy ")),
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

/// The Programmer keypad (left of the basic keys): label and action, six keys a row.
const PROG_KEYS: [(&str, PKey); SCI_COLS * SCI_ROWS] = [
    ("A", PKey::Digit(10)),
    ("B", PKey::Digit(11)),
    ("C", PKey::Digit(12)),
    ("D", PKey::Digit(13)),
    ("E", PKey::Digit(14)),
    ("F", PKey::Digit(15)),
    ("AND", PKey::Op(BinOp::And)),
    ("OR", PKey::Op(BinOp::Or)),
    ("XOR", PKey::Op(BinOp::Xor)),
    ("NOR", PKey::Op(BinOp::Nor)),
    ("NOT", PKey::Not),
    ("2's", PKey::Negate),
    ("<<", PKey::Shl1),
    (">>", PKey::Shr1),
    ("X<<Y", PKey::Op(BinOp::Shl)),
    ("X>>Y", PKey::Op(BinOp::Shr)),
    ("RoL", PKey::Rol),
    ("RoR", PKey::Ror),
    ("FlipB", PKey::FlipBytes),
    ("FlipW", PKey::FlipWords),
    ("BIN", PKey::SetBase(Base::Bin)),
    ("OCT", PKey::SetBase(Base::Oct)),
    ("DEC", PKey::SetBase(Base::Dec)),
    ("HEX", PKey::SetBase(Base::Hex)),
    ("00", PKey::Pair(0)),
    ("FF", PKey::Pair(15)),
    ("8 bit", PKey::SetBits(8)),
    ("16 bit", PKey::SetBits(16)),
    ("32 bit", PKey::SetBits(32)),
    ("64 bit", PKey::SetBits(64)),
];

/// The three keypads the Basic, Scientific and Programmer segments choose between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Basic,
    Scientific,
    Programmer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Bit(u8),
    Mode(Mode),
    /// The keypad pull-down of a compact window (`BAR_MODE`).
    ModeMenu,
    Key(usize),
    Display,
    Backspace,
    History,
    Done,
    ClearHistory,
    CopyTape,
    Result(usize),
    Scientific,
    MenuCopy,
    MenuPaste,
    MenuDismiss,
    ShowConvert,
    LeaveConvert,
    Category(usize),
    From,
    Swap,
    To,
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
    /// Convert mode while set; the keypad then enters a conversion instead of a calculation.
    convert: Option<Converter>,
    /// The day's currency rates (None until a copy is loaded) and whether a check for new ones is running.
    rates: Option<Rates>,
    rates_pending: bool,
    /// The last Copy, waiting for the app to send it to the shell's pasteboard (taken with `take_copy`).
    copy_request: Option<CopyRequest>,
    /// Programmer mode while set (the keypad enters integers in a base on a fixed-size word).
    programmer: Option<Programmer>,
    /// The RPN stack, used while `Settings::rpn` is on.
    rpn: Rpn,
    /// Clear History was tapped once and waits for a second tap.
    clear_confirm: bool,
    /// The history row a touch went down on, for the swipe that deletes it.
    swipe_row: Option<usize>,
    /// The history row the context menu is open for.
    ctx_row: Option<usize>,
    /// The keypad pull-down was tapped and waits for the app to open it (`take_mode_menu`).
    mode_menu: bool,
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
            convert: None,
            rates: None,
            rates_pending: false,
            copy_request: None,
            programmer: None,
            rpn: Rpn::default(),
            clear_confirm: false,
            swipe_row: None,
            ctx_row: None,
            mode_menu: false,
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

    #[cfg(test)]
    pub fn display(&self) -> &str {
        self.engine.display()
    }

    /// Whether the scientific keypad shows. On the desktop only the Basic / Scientific choice decides it: a landscape
    /// window is the norm here, so the tablet rule "landscape is scientific" would pin the keypad (E2E W4-6).
    fn sci_active(&self, ui: &Ui) -> bool {
        self.convert.is_none()
            && self.programmer.is_none()
            && !self.settings.rpn
            && self.w as f32 >= Self::wide_min_width(ui)
            && self.settings.scientific
    }

    /// Whether the Programmer keypad is showing (it needs the wide layout, like the scientific one).
    fn programmer_active(&self, ui: &Ui) -> bool {
        self.convert.is_none() && self.programmer.is_some() && self.w as f32 >= Self::wide_min_width(ui)
    }

    fn wide_min_width(ui: &Ui) -> f32 {
        ui.pt(WIDE_INSET_PT + WIDE_KEY_PT * WIDE_COLUMNS + SPACE_S * WIDE_GAPS)
    }

    /// Whether the basic keypad drives the RPN stack.
    fn rpn_active(&self) -> bool {
        self.convert.is_none() && self.programmer.is_none() && self.settings.rpn
    }

    /// The wide layout: the basic keys on the right and a 6 x 5 keypad (scientific or Programmer) on the left.
    fn wide(&self, ui: &Ui) -> bool {
        self.sci_active(ui) || self.programmer_active(ui)
    }

    /// Height of the binary row above the keys (Programmer mode only).
    fn strip_h(&self, ui: &Ui) -> f32 {
        if self.programmer_active(ui) { ui.pt(PROG_STRIP_PT) } else { 0.0 }
    }

    /// Where the display block ends: the top of the keys, or of the binary row above them.
    fn deck_top(&self, ui: &Ui) -> f32 {
        self.layout(ui).y - self.strip_h(ui)
    }

    /// The largest display size, in points, that fits. In Convert the entry line and the result share the room between
    /// the unit row and the keys, so the result shrinks (down to title 1) instead of running under the capsules.
    fn display_pt_max(&self, ui: &Ui) -> f32 {
        if self.convert.is_none() {
            // between the toolbar (and the expression line under it) and the Delete control over the keys: a compact
            // window's display is smaller, a large one keeps the large display size
            let room = self.deck_top(ui) - ui.pt(62.0) - ui.pt(UNIFIED_BAR_PT) - ui.pt(MAC_TITLE1_PT * LINE_EM + SPACE_M);
            return (room / ui.pt(1.0) / LINE_EM).clamp(MAC_LARGE_TITLE_PT, DISPLAY_LARGE_PT);
        }
        let room = self.deck_top(ui) - ui.pt(62.0) - ui.pt(units_top_pt() + UNITS_H_PT + SPACE_S);
        let line = ui.pt(MAC_TITLE1_PT * LINE_EM + SPACE_M);
        ((room - line) / ui.pt(1.0) / LINE_EM).clamp(MAC_TITLE1_PT, DISPLAY_LARGE_PT)
    }

    fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            self.settings.save(path);
        }
    }

    /// How much of the full margins a window of this width keeps: 1 from `FULL_MARGIN_W_PT` up, less in a narrower one.
    fn margin_scale(&self, ui: &Ui) -> f32 {
        (self.w as f32 / ui.pt(FULL_MARGIN_W_PT)).clamp(0.0, 1.0)
    }

    fn layout(&self, ui: &Ui) -> Layout {
        let k = self.margin_scale(ui);
        let bottom = self.h as f32 - (home_zone(ui) + ui.pt(SPACE_XXL)) * k;
        let extra = self.strip_h(ui)
            + if self.convert.is_some() { ui.pt(convert_band_pt()) } else { 0.0 }
            + if self.convert_is_currency() { ui.pt(RATES_NOTE_H_PT) } else { 0.0 };
        let available = bottom - ui.pt(DISPLAY_AND_KEYS_RESERVE_PT) - extra;
        if self.wide(ui) {
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
        // a wide window: 12-point gaps and 32-point sides; a compact one keeps the reference's proportions (a gap of an
        // eighth of a key, sides that shrink with the width)
        let room = self.w as f32 - ui.pt(64.0) * k;
        let full_gap_cell = (room - ui.pt(SPACE_M) * 3.0) / 4.0;
        let (cell_w, gap) = if full_gap_cell >= ui.pt(SPACE_M * 8.0) {
            (full_gap_cell, ui.pt(SPACE_M))
        } else {
            let cell = room / (4.0 + 3.0 / 8.0);
            (cell, cell / 8.0)
        };
        let cell = ui
            .pt(135.0)
            .min(cell_w)
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
        KEYS.len() + if self.wide(ui) { SCI_KEYS.len() } else { 0 }
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
    #[cfg(test)]
    pub fn key_centre(&self, ui: &Ui, key: Key) -> Option<(f32, f32)> {
        let i = KEYS.iter().position(|&k| k == key)?;
        let (x, y, w, h) = self.key_rect(ui, i);
        Some((x + w / 2.0, y + h / 2.0))
    }

    /// The centre of a visible scientific key (by its un-2nd action).
    #[cfg(test)]
    pub fn sci_key_centre(&self, ui: &Ui, key: SciKey) -> Option<(f32, f32)> {
        if !self.sci_active(ui) {
            return None;
        }
        let k = SCI_KEYS.iter().position(|d| d.act == Act::Key(key))?;
        let (x, y, w, h) = self.key_rect(ui, KEYS.len() + k);
        Some((x + w / 2.0, y + h / 2.0))
    }

    /// The unified toolbar: the history sidebar toggle and the keypad segments. The decimal separator is a View menu
    /// item ("Decimal Comma"), not an unlabelled toolbar chip (E2E W4-7).
    /// A window too narrow for the four segments (the compact Basic window) shows the history button and one keypad
    /// pull-down instead, as Golden Gate Calculator does.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let full = self.full_bar(ui);
        if widgets::toolbar_layout(ui, self.bar_bounds(ui), &full).overflow.is_empty() {
            return full;
        }
        Toolbar::new(vec![
            ToolbarItem::icon(BAR_HISTORY, "History", "sidebar.left").selected(self.history_open),
            ToolbarItem::flexible_space(),
            ToolbarItem::menu(BAR_MODE, self.mode_title(), None),
        ])
    }

    /// The name of the keypad that shows (or that the window is resizing for), as the pull-down shows it.
    fn mode_title(&self) -> &'static str {
        if self.convert.is_some() {
            "Convert"
        } else if self.programmer.is_some() {
            "Programmer"
        } else if self.settings.scientific && !self.settings.rpn {
            "Scientific"
        } else {
            "Basic"
        }
    }

    /// The content size, in points, the window should have for the keypad that is chosen (`windows::set_content_size`).
    pub fn preferred_size(&self) -> (f32, f32) {
        if self.convert.is_some() {
            CONVERT_SIZE_PT
        } else if self.programmer.is_some() {
            PROGRAMMER_SIZE_PT
        } else if self.settings.scientific && !self.settings.rpn {
            SCIENTIFIC_SIZE_PT
        } else {
            BASIC_SIZE_PT
        }
    }

    /// The keypad pull-down, once it was tapped: where it opens (px, under the button) and its items, the chosen keypad
    /// checked. The app opens it as a context menu; the choice comes back to `context_action`.
    pub fn take_mode_menu(&mut self, ui: &Ui) -> Option<(f32, f32, Vec<ContextItem>)> {
        if !std::mem::take(&mut self.mode_menu) {
            return None;
        }
        let r = widgets::toolbar_layout(ui, self.bar_bounds(ui), &self.bar(ui)).rect_of(BAR_MODE)?;
        Some((ui.pt(r.x), ui.pt(r.y + r.h), self.mode_menu_items()))
    }

    /// The full toolbar: the history sidebar toggle and the four keypad segments.
    fn full_bar(&self, ui: &Ui) -> Toolbar {
        let sci = self.sci_active(ui);
        Toolbar::new(vec![
            ToolbarItem::icon(BAR_HISTORY, "History", "sidebar.left").selected(self.history_open),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(BAR_BASIC, "Basic").in_group(1).selected(!sci && self.convert.is_none() && self.programmer.is_none()),
            ToolbarItem::button(BAR_SCIENTIFIC, "Scientific").in_group(1).selected(sci),
            ToolbarItem::button(BAR_PROGRAMMER, "Programmer").in_group(1).selected(self.programmer.is_some() && self.convert.is_none()),
            ToolbarItem::button(BAR_CONVERT, "Convert").in_group(1).selected(self.convert.is_some()),
        ])
    }

    /// The toolbar's rectangle in points, right of the traffic lights.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (self.w as f32 / ui.pt(1.0) - x - SPACE_S).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The toolbar laid out as `widgets::toolbar` draws it (titled buttons measured at body size).
    /// The same layout `chrome::toolbar` draws (`widgets::toolbar_layout`), so a tap lands on what is drawn.
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_bounds(ui), &self.bar(ui))
    }

    /// Toolbar item `id` in pixels, or an empty rectangle when it overflowed.
    #[cfg(test)]
    fn bar_rect(&self, ui: &Ui, id: u32) -> (f32, f32, f32, f32) {
        self.bar_layout(ui).rect_of(id).map_or((0.0, 0.0, 0.0, 0.0), |r| (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)))
    }

    #[cfg(test)]
    fn history_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, BAR_HISTORY)
    }

    /// The Scientific segment (Basic when the scientific keypad shows, so a tap toggles it).
    #[cfg(test)]
    fn scientific_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, if self.sci_active(ui) { BAR_BASIC } else { BAR_SCIENTIFIC })
    }

    /// The label of the Clear key: AC when there is nothing to clear, else C.
    fn clear_label(&self, ui: &Ui) -> &'static str {
        if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            p.clear_label()
        } else if self.rpn_active() {
            self.rpn.clear_label()
        } else if self.sci_active(ui) {
            self.sci.clear_label()
        } else {
            self.engine.clear_label()
        }
    }

    /// The name a control reader gives keypad key `index` (its label in the current second and angle state).
    fn key_name(&self, ui: &Ui, index: usize) -> String {
        if index >= KEYS.len() && self.programmer_active(ui) {
            return PROG_KEYS[index - KEYS.len()].0.into();
        }
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
            Key::Equals if self.rpn_active() => "Enter".into(),
            Key::Equals => "Equals".into(),
            Key::Clear => (if self.clear_label(ui) == "AC" { "All Clear" } else { "Clear" }).into(),
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
            let row_y = py + ph - ui.pt(64.0);
            let half = pw / 2.0;
            nodes.push(node(ID_COPY_TAPE, Role::Button, "Copy Tape".into(), String::new(), (px, row_y, half, ui.pt(64.0)), !self.store.is_empty()));
            let clear = if self.clear_confirm { "Confirm Clear History" } else { "Clear History" };
            nodes.push(node(ID_CLEAR_HISTORY, Role::Button, clear.into(), String::new(), (px + half, row_y, pw - half, ui.pt(64.0)), true));
            return nodes;
        }
        if let Some(conv) = &self.convert {
            for (index, category) in Category::ALL.iter().enumerate() {
                let value = if *category == conv.category { "selected" } else { "" };
                nodes.push(node(ID_CATEGORY + index as u32, Role::Button, category.title().into(), value.into(), self.category_rect(ui, index), true));
            }
            let [from, swap, to] = self.unit_rects(ui);
            nodes.push(node(ID_FROM, Role::Button, format!("From: {}", conv.from_unit().name), String::new(), from, true));
            nodes.push(node(ID_SWAP, Role::Button, "Swap Units".into(), String::new(), swap, true));
            nodes.push(node(ID_TO, Role::Button, format!("To: {}", conv.to_unit().name), String::new(), to, true));
        }
        for index in 0..self.key_count(ui) {
            nodes.push(node(ID_KEY + index as u32, Role::Button, self.key_name(ui, index), String::new(), self.key_rect(ui, index), true));
        }
        if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            for bit in 0..p.bits() {
                let value = if p.bit(bit) { "1" } else { "0" };
                nodes.push(node(ID_BIT + u32::from(bit), Role::Button, format!("Bit {bit}"), value.into(), self.bit_rect(ui, bit), true));
            }
        }
        nodes.push(node(ID_DELETE, Role::Button, "Delete".into(), String::new(), self.backspace_button(ui), true));
        nodes.push(node(ID_DISPLAY, Role::StaticText, "Display".into(), self.display_text(ui), self.display_rect(ui), true));
        nodes
    }

    fn backspace_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        (
            l.x + l.width - ui.pt(BACKSPACE_CONTROL_WIDTH_PT),
            self.deck_top(ui) - ui.pt(55.0),
            ui.pt(BACKSPACE_CONTROL_WIDTH_PT),
            ui.pt(44.0),
        )
    }

    /// The `bit`-th cell of the binary row (bit 0 is the rightmost of the lower row; 63 the leftmost of the upper).
    fn bit_rect(&self, ui: &Ui, bit: u8) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        let strip = ui.pt(PROG_STRIP_PT);
        let gap = ui.pt(SPACE_XS);
        let row_h = (strip - gap) / 2.0;
        let cw = l.width / 32.0;
        let col = 31 - usize::from(bit % 32);
        let row = if bit >= 32 { 0.0 } else { 1.0 };
        (l.x + col as f32 * cw, l.y - strip + row * (row_h + gap), cw, row_h)
    }

    /// The bit of the word under (`x`, `y`), if the binary row is showing.
    fn bit_at(&self, ui: &Ui, x: f32, y: f32) -> Option<u8> {
        let p = self.programmer.as_ref().filter(|_| self.programmer_active(ui))?;
        (0..p.bits()).find(|bit| Self::contains(self.bit_rect(ui, *bit), x, y))
    }

    fn display_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let l = self.layout(ui);
        (
            l.x,
            ui.pt(90.0),
            l.width,
            (self.deck_top(ui) - ui.pt(DISPLAY_DECK_INSET_PT)).max(ui.pt(SPACE_HUGE)),
        )
    }

    fn menu_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.display_rect(ui);
        (x + w - ui.pt(DISPLAY_MENU_TRAILING_INSET_PT), y + ui.pt(SPACE_S), ui.pt(DISPLAY_MENU_WIDTH_PT), ui.pt(DISPLAY_MENU_HEIGHT_PT))
    }

    fn menu_item(&self, ui: &Ui, paste: bool) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.menu_rect(ui);
        (if paste { x + w / 2.0 } else { x }, y, w / 2.0, h)
    }

    fn panel(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        // A full-height history sidebar under the traffic lights (Golden Gate Calculator's sidebar).
        (0.0, 0.0, ui.pt(m::SIDEBAR_W).min(self.w as f32 * 0.85), self.h as f32 - home_zone(ui))
    }

    fn row_height(&self, ui: &Ui) -> f32 {
        let (_, _, _, h) = self.panel(ui);
        ui.pt(HISTORY_ROW_MAX_PT).min((h - ui.pt(HISTORY_CHROME_RESERVE_PT)) / 8.0).max(ui.pt(30.0))
    }

    /// How many history rows fit between the title and Clear History.
    fn rows_visible(&self, ui: &Ui) -> usize {
        let (_, _, _, h) = self.panel(ui);
        (((h - ui.pt(HISTORY_VIEWPORT_RESERVE_PT)) / self.row_height(ui)).floor() as usize).max(1)
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
                BAR_MODE => Some(Target::ModeMenu),
                BAR_CONVERT => Some(if self.convert.is_some() { Target::LeaveConvert } else { Target::ShowConvert }),
                BAR_BASIC | BAR_SCIENTIFIC | BAR_PROGRAMMER if self.convert.is_some() => Some(Target::LeaveConvert),
                BAR_PROGRAMMER if self.programmer.is_none() => Some(Target::Mode(Mode::Programmer)),
                BAR_BASIC if self.programmer.is_some() => Some(Target::Mode(Mode::Basic)),
                BAR_SCIENTIFIC if self.programmer.is_some() => Some(Target::Mode(Mode::Scientific)),
                BAR_BASIC if self.sci_active(ui) => Some(Target::Scientific),
                BAR_SCIENTIFIC if !self.sci_active(ui) => Some(Target::Scientific),
                _ => None,
            };
        }
        if self.history_open {
            let (px, py, pw, ph) = self.panel(ui);
            if Self::contains((px, py + ph - ui.pt(64.0), pw, ui.pt(64.0)), x, y) {
                // The left half copies the tape; the right half clears the history.
                return Some(if x < px + pw / 2.0 { Target::CopyTape } else { Target::ClearHistory });
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
        if self.convert.is_some() {
            if let Some(target) = self.convert_hit(ui, x, y) {
                return Some(target);
            }
        }
        if Self::contains(self.backspace_button(ui), x, y) {
            return Some(Target::Backspace);
        }
        if let Some(bit) = self.bit_at(ui, x, y) {
            return Some(Target::Bit(bit));
        }
        for i in 0..self.key_count(ui) {
            if Self::contains(self.key_rect(ui, i), x, y) {
                return Some(Target::Key(i));
            }
        }
        Self::contains(self.display_rect(ui), x, y).then_some(Target::Display)
    }

    /// Where Convert's band runs across the window (x and width in pixels): the window's width inside the side margins,
    /// at most `CONVERT_MAX_W_PT` and never narrower than the keypad, centred. The keypad alone is too narrow for
    /// seven category titles ("Len...", "Te...", E2E W4-7).
    fn convert_band(&self, ui: &Ui) -> (f32, f32) {
        let l = self.layout(ui);
        let w = (self.w as f32 - 2.0 * ui.pt(SPACE_XL)).min(ui.pt(CONVERT_MAX_W_PT)).max(l.width);
        ((self.w as f32 - w) / 2.0, w)
    }

    /// The category segments of Convert: rows of `CATEGORY_COLS` across the band's width.
    fn category_rect(&self, ui: &Ui, index: usize) -> (f32, f32, f32, f32) {
        let (bx, bw) = self.convert_band(ui);
        let (col, row) = ((index % CATEGORY_COLS) as f32, (index / CATEGORY_COLS) as f32);
        let w = bw / CATEGORY_COLS as f32;
        (bx + w * col, ui.pt(CATEGORY_TOP_PT) + row * ui.pt(CATEGORY_H_PT), w, ui.pt(CATEGORY_H_PT))
    }

    /// The From unit, the Swap control and the To unit, left to right, across the keypad's width.
    fn convert_is_currency(&self) -> bool {
        self.convert.as_ref().is_some_and(|c| c.category == Category::Currency)
    }

    /// Hands the day's currency rates to the calculator and to an open Convert panel.
    pub fn set_rates(&mut self, rates: Option<Rates>, pending: bool) {
        self.rates = rates;
        self.rates_pending = pending;
        if let Some(conv) = &mut self.convert {
            conv.rates = self.rates.clone();
        }
        self.dirty = true;
    }

    fn unit_rects(&self, ui: &Ui) -> [(f32, f32, f32, f32); 3] {
        let (bx, bw) = self.convert_band(ui);
        let gap = ui.pt(SPACE_M);
        let swap = ui.pt(96.0);
        let chip = (bw - swap - 2.0 * gap) / 2.0;
        let (y, h) = (ui.pt(units_top_pt()), ui.pt(UNITS_H_PT));
        [(bx, y, chip, h), (bx + chip + gap, y, swap, h), (bx + bw - chip, y, chip, h)]
    }

    fn convert_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        for index in 0..Category::ALL.len() {
            if Self::contains(self.category_rect(ui, index), x, y) {
                return Some(Target::Category(index));
            }
        }
        let [from, swap, to] = self.unit_rects(ui);
        if Self::contains(from, x, y) {
            Some(Target::From)
        } else if Self::contains(swap, x, y) {
            Some(Target::Swap)
        } else if Self::contains(to, x, y) {
            Some(Target::To)
        } else {
            None
        }
    }

    /// Convert's chrome: the category segments, then the From, Swap and To controls.
    fn draw_convert(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let Some(conv) = self.convert.clone() else { return };
        let (bx, bw) = self.convert_band(ui);
        for row in 0..category_rows() {
            let in_row = (Category::ALL.len() - row * CATEGORY_COLS).min(CATEGORY_COLS);
            let (rw, rh) = (bw * in_row as f32 / CATEGORY_COLS as f32, ui.pt(CATEGORY_H_PT));
            s.fill_rrect(RectF::new(bx, ui.pt(CATEGORY_TOP_PT) + row as f32 * rh, rw, rh), capsule(rw, rh), utility());
        }
        for (index, category) in Category::ALL.iter().enumerate() {
            let rect = self.category_rect(ui, index);
            let (x, y, w, h) = rect;
            if *category == conv.category {
                let inset = ui.pt(SPACE_XXS);
                s.fill_rrect(RectF::new(x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset), capsule(w, h), scientific());
            }
            let title = ui.fit(category.title(), MAC_CALLOUT_PT, w - ui.pt(SPACE_S));
            self.label(s, ui, &title, MAC_CALLOUT_PT, LABEL.now(), rect);
        }
        let [from, swap, to] = self.unit_rects(ui);
        if conv.category == Category::Currency {
            let note = convert::rates_note(conv.rates.as_ref(), self.rates_pending);
            let (x, w) = (bx, bw);
            let y = ui.pt(units_top_pt() + UNITS_H_PT + 2.0);
            let note = ui.fit(&note, MAC_FOOTNOTE_PT, w);
            self.label(s, ui, &note, MAC_FOOTNOTE_PT, LABEL_2.now(), (x, y, w, ui.pt(RATES_NOTE_H_PT)));
        }
        let from_text = ui.fit(&format!("From: {}", conv.from_unit().name), MAC_BODY_PT, from.2 - ui.pt(SPACE_L));
        s.fill_rrect(RectF::new(from.0, from.1, from.2, from.3), capsule(from.2, from.3), utility());
        self.label(s, ui, &from_text, MAC_BODY_PT, LABEL.now(), from);
        // Swap is a bordered button like the unit capsules beside it, not a bare link (E2E W4-7)
        let swap_fill = if self.pressed == Some(Target::Swap) { key_down() } else { utility() };
        s.fill_rrect(RectF::new(swap.0, swap.1, swap.2, swap.3), capsule(swap.2, swap.3), swap_fill);
        self.label(s, ui, "Swap", MAC_BODY_PT, LABEL.now(), swap);
        let to_text = ui.fit(&format!("To: {}", conv.to_unit().name), MAC_BODY_PT, to.2 - ui.pt(SPACE_L));
        s.fill_rrect(RectF::new(to.0, to.1, to.2, to.3), capsule(to.2, to.3), utility());
        self.label(s, ui, &to_text, MAC_BODY_PT, LABEL.now(), to);
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
        if let Some(conv) = &mut self.convert {
            match key {
                Key::Digit(digit) => conv.digit(digit),
                Key::Decimal => conv.decimal(),
                Key::Sign => conv.sign(),
                Key::Backspace => conv.backspace(),
                Key::Clear => conv.clear(),
                _ => {}
            }
            return;
        }
        if self.programmer_active(ui) {
            let pkey = match key {
                Key::Digit(d) => PKey::Digit(d),
                Key::Operator(Operator::Add) => PKey::Op(BinOp::Add),
                Key::Operator(Operator::Subtract) => PKey::Op(BinOp::Sub),
                Key::Operator(Operator::Multiply) => PKey::Op(BinOp::Mul),
                Key::Operator(Operator::Divide) => PKey::Op(BinOp::Div),
                Key::Equals => PKey::Equals,
                Key::Clear => PKey::Clear,
                Key::Sign => PKey::Negate,
                Key::Backspace => PKey::Backspace,
                // whole numbers only: no decimal point, no percent
                Key::Decimal | Key::Percent => return,
            };
            self.programmer_press(pkey);
            return;
        }
        if self.rpn_active() {
            let rkey = match key {
                Key::Digit(d) => RKey::Digit(d),
                Key::Decimal => RKey::Decimal,
                Key::Operator(Operator::Add) => RKey::Add,
                Key::Operator(Operator::Subtract) => RKey::Sub,
                Key::Operator(Operator::Multiply) => RKey::Mul,
                Key::Operator(Operator::Divide) => RKey::Div,
                Key::Equals => RKey::Enter,
                Key::Clear => RKey::Clear,
                Key::Sign => RKey::Sign,
                Key::Percent => RKey::Percent,
                Key::Backspace => RKey::Backspace,
            };
            self.rpn_press(rkey);
            return;
        }
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

    /// A Programmer key; a finished calculation goes to the history.
    fn programmer_press(&mut self, key: PKey) {
        let Some(p) = &mut self.programmer else { return };
        if let Some((expression, result, value)) = p.press(key) {
            self.store.push(Entry { expression, result, value });
        }
    }

    /// An RPN key; a finished operation goes to the history.
    fn rpn_press(&mut self, key: RKey) {
        if let Some((expression, result, value)) = self.rpn.press(key) {
            self.store.push(Entry { expression, result, value });
        }
    }

    /// Switches the keypad (the Basic, Scientific and Programmer segments and the View menu). RPN rides on the
    /// basic keypad, so it ends when another keypad is chosen.
    fn set_mode(&mut self, mode: Mode) {
        self.convert = None;
        match mode {
            Mode::Basic => {
                self.programmer = None;
                self.settings.scientific = false;
            }
            Mode::Scientific => {
                self.programmer = None;
                self.settings.scientific = true;
                self.settings.rpn = false;
            }
            Mode::Programmer => {
                self.settings.rpn = false;
                if self.programmer.is_none() {
                    self.programmer = Some(Programmer::default());
                }
            }
        }
        self.save_settings();
        self.dirty = true;
    }

    fn press_index(&mut self, ui: &Ui, index: usize) {
        if index < KEYS.len() {
            self.press_basic(ui, KEYS[index]);
            return;
        }
        if self.programmer_active(ui) {
            self.programmer_press(PROG_KEYS[index - KEYS.len()].1);
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
        if let Some(conv) = &self.convert {
            return conv.result().map(format_number);
        }
        if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            return (!p.is_error()).then(|| p.text());
        }
        if self.rpn_active() {
            return self.rpn.x().map(format_number);
        }
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
        if let Some(conv) = &mut self.convert {
            conv.set_value(value);
            return true;
        }
        self.load_value(ui, value)
    }

    /// Makes `value` the number on the active keypad (Paste, a recalled history result); false when the keypad
    /// cannot hold it (Programmer takes whole numbers only).
    fn load_value(&mut self, ui: &Ui, value: f64) -> bool {
        if self.programmer_active(ui) {
            return self.programmer.as_mut().is_some_and(|p| p.set_value(value));
        }
        if self.rpn_active() {
            self.rpn.set_value(value);
        } else if self.sci_active(ui) {
            self.sci.set_value(value);
        } else {
            self.engine.paste_value(value);
        }
        true
    }

    /// Edit > Copy (Cmd+C): the same copy as the Copy item of the touch menu. The copy is handed to the app
    /// (`take_copy`), which sends it to the shell's pasteboard.
    pub fn copy(&mut self, ui: &Ui) {
        if let Some(number) = self.copy_text(ui) {
            let shown = localize_with(&number, self.settings.locale, self.settings.thousands);
            self.copy_request = Some(CopyRequest { number, shown });
        }
    }

    /// The copy made since the last call, if any (each Copy is taken once).
    pub fn take_copy(&mut self) -> Option<CopyRequest> {
        self.copy_request.take()
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
        if let Some(conv) = &mut self.convert {
            conv.set_value(value);
            return;
        }
        if self.programmer_active(ui) || self.rpn_active() || self.sci_active(ui) {
            self.load_value(ui, value);
        } else {
            self.engine.recall_value(value);
        }
    }

    /// Puts the paper tape (the whole history, oldest first) on the shared clipboard as plain text.
    fn copy_tape(&mut self) {
        if self.store.is_empty() {
            return;
        }
        clipboard::set_text(&tape::render(self.store.entries(), self.settings.locale));
    }

    fn clear_history(&mut self) {
        self.store.clear();
        self.engine.history.clear();
        self.hist_offset = 0;
    }

    fn display_text(&self, ui: &Ui) -> String {
        if let Some(conv) = &self.convert {
            return conv.result().map_or_else(|| "Error".into(), format_number);
        }
        if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            p.text()
        } else if self.rpn_active() {
            self.rpn.display()
        } else if self.sci_active(ui) {
            pretty(&self.sci.display())
        } else {
            self.engine.display().to_string()
        }
    }

    fn expression_text(&self, ui: &Ui) -> String {
        if let Some(conv) = &self.convert {
            return format!("{} {}", conv.entry_text(), conv.from_unit().symbol);
        }
        if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            p.expression.clone()
        } else if self.rpn_active() {
            self.rpn.below().iter().map(|v| format_number(*v)).collect::<Vec<_>>().join("   ")
        } else if self.sci_active(ui) {
            pretty(self.sci.expression_line())
        } else {
            self.engine.expression.clone()
        }
    }

    /// The value of the number on the display when it is a result (not an entry being typed): the View settings
    /// for decimal places and scientific notation rewrite it from this value.
    fn result_value(&self, ui: &Ui) -> Option<f64> {
        if self.convert.is_some() || self.programmer.is_some() {
            return None;
        }
        if self.rpn_active() {
            self.rpn.result()
        } else if self.sci_active(ui) {
            if self.sci.error { None } else { self.sci.result }
        } else {
            self.engine.result_value()
        }
    }

    /// The text of the number on the display for the screen: Programmer shows its base as is, everything else is
    /// rewritten for the View settings and given the locale's separators.
    fn display_shown(&self, ui: &Ui) -> String {
        if self.programmer_active(ui) {
            return self.display_text(ui);
        }
        let style = self.settings.style();
        let text = match self.result_value(ui) {
            Some(value) if style.rewrites() => format_styled(value, style),
            _ => self.display_text(ui),
        };
        localize_with(&text, self.settings.locale, style.thousands)
    }

    /// Deletes history entry `index` (a swipe on its row, or the context menu).
    fn delete_entry(&mut self, ui: &Ui, index: usize) {
        if self.store.remove(index) {
            let max = self.store.len().saturating_sub(self.rows_visible(ui));
            self.hist_offset = self.hist_offset.min(max);
            self.dirty = true;
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        // Clear History asks twice: any other tap withdraws the question
        if target != Target::ClearHistory {
            self.clear_confirm = false;
        }
        match target {
            Target::Bit(bit) => {
                if let Some(p) = &mut self.programmer {
                    p.toggle_bit(bit);
                }
            }
            Target::Mode(mode) => self.set_mode(mode),
            Target::ModeMenu => self.mode_menu = true,
            Target::Key(i) => self.press_index(ui, i),
            Target::Backspace => self.press_basic(ui, Key::Backspace),
            Target::ShowConvert => self.convert = Some(Converter::with_rates(self.rates.clone())),
            Target::LeaveConvert => self.convert = None,
            Target::Category(index) => {
                if let (Some(conv), Some(category)) = (self.convert.as_mut(), Category::ALL.get(index)) {
                    conv.select_category(*category);
                }
            }
            Target::From => {
                if let Some(conv) = self.convert.as_mut() {
                    conv.next_from();
                }
            }
            Target::To => {
                if let Some(conv) = self.convert.as_mut() {
                    conv.next_to();
                }
            }
            Target::Swap => {
                if let Some(conv) = self.convert.as_mut() {
                    conv.swap();
                }
            }
            Target::History => {
                self.history_open = true;
                self.hist_offset = 0;
            }
            Target::Done => self.history_open = false,
            Target::ClearHistory => {
                if self.clear_confirm {
                    self.clear_confirm = false;
                    self.clear_history();
                } else if !self.store.is_empty() {
                    self.clear_confirm = true;
                }
            }
            Target::CopyTape => self.copy_tape(),
            Target::Result(i) => {
                self.recall(ui, i);
                self.history_open = false;
            }
            Target::Scientific => {
                self.settings.scientific = !self.settings.scientific;
                if self.settings.scientific {
                    self.settings.rpn = false;
                    self.programmer = None;
                }
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
                self.swipe_row = match self.pressed {
                    Some(Target::Result(i)) => Some(i),
                    _ => None,
                };
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
                let swiped = self.swipe_row.take();
                let dx = (x - self.down.0).abs();
                let dy = (y - self.down.1).abs();
                if let Some(row) = swiped.filter(|_| self.history_open && x < self.down.0 && dx > ui.pt(SPACE_HUGE) && dx > dy * 1.5) {
                    // a swipe to the left across a history row deletes it
                    self.delete_entry(ui, row);
                } else if pressed == Some(Target::Display) && dx > ui.pt(SPACE_HUGE) && dx > dy * 1.5 {
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

    /// The state a test reads back, in the same words as the app's `describe` line.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        format!(
            "calculator_display={:?} calculator_history={} calculator_history_open={} calculator_scientific={} calculator_locale={} calculator_convert={:?}",
            self.display(),
            self.store.len(),
            self.history_open,
            self.settings.scientific || self.w > self.h,
            self.settings.locale.code(),
            self.convert.as_ref().map_or_else(|| "off".to_string(), Converter::describe)
        )
    }

    fn label(
        &mut self,
        s: &mut dyn Canvas,
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
            "yˣ" => Some(("y", "x", false)),
            "ˣ√y" => Some(("√y", "x", true)),
            "10ˣ" => Some(("10", "x", false)),
            "ʸ√x" => Some(("√x", "y", true)),
            // The cube-root sign U+221B is in no bundled face, so the key is a small 3 before the square root.
            "³√x" => Some(("√x", "3", true)),
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
            chrome::blit(s, power, power_x as i32, (top - pt * ui.s * 0.2) as i32);
            chrome::blit(s, &base, base_x as i32, top as i32);
            return;
        }
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        chrome::blit(s, image, (x + (w - image.w as f32) / 2.0) as i32, (y + (h - image.h as f32) / 2.0) as i32);
    }

    fn draw_key(&mut self, s: &mut dyn Canvas, ui: &Ui, index: usize) {
        let rect = self.key_rect(ui, index);
        let (x, y, w, h) = rect;
        let sci_layout = self.wide(ui);
        if index >= KEYS.len() && self.programmer_active(ui) {
            self.draw_programmer_key(s, ui, index);
            return;
        }
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
            s.fill_rrect(RectF::new(x, y, w, h), capsule(w, h), fill);
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
        let selected = !sci_layout && self.convert.is_none() && !self.rpn_active()
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
        s.fill_rrect(RectF::new(x, y, w, h), capsule(w, h), fill);
        let clear = self.clear_label(ui);
        let decimal = self.settings.locale.decimal().to_string();
        let label = match key {
            Key::Digit(digit) => digit.to_string(),
            Key::Decimal => decimal,
            Key::Operator(op) => op.label().into(),
            Key::Equals if self.rpn_active() => "Enter".into(),
            Key::Equals => "=".into(),
            Key::Clear => clear.into(),
            Key::Sign => "+/−".into(),
            Key::Percent => "%".into(),
            Key::Backspace => unreachable!(),
        };
        let em = if is_utility || label.chars().count() > 2 { UTILITY_LABEL_EM } else { KEY_LABEL_EM };
        let pt = (h / ui.s * em).min(DISPLAY_MEDIUM_PT);
        let rect = if key == Key::Digit(0) {
            (x, y, self.layout(ui).cw, h)
        } else {
            rect
        };
        self.label(s, ui, &label, pt, color, rect);
    }

    /// A key of the Programmer keypad; the current base and word size are drawn selected.
    fn draw_programmer_key(&mut self, s: &mut dyn Canvas, ui: &Ui, index: usize) {
        let rect = self.key_rect(ui, index);
        let (x, y, w, h) = rect;
        let (label, key) = PROG_KEYS[index - KEYS.len()];
        let on = self.programmer.as_ref().is_some_and(|p| match key {
            PKey::SetBase(base) => p.base == base,
            PKey::SetBits(bits) => p.bits() == bits,
            _ => false,
        });
        let pressed = self.pressed == Some(Target::Key(index));
        let fill = if on {
            LABEL.now()
        } else if pressed {
            key_down()
        } else {
            scientific()
        };
        let color = if on { CARD_BG.now() } else { LABEL.now() };
        // a hex digit that the base cannot type is dimmed
        let usable = match key {
            PKey::Digit(d) | PKey::Pair(d) => self.programmer.as_ref().map_or(true, |p| u32::from(d) < p.base.radix()),
            _ => true,
        };
        let color = if usable { color } else { LABEL_3.now() };
        s.fill_rrect(RectF::new(x, y, w, h), capsule(w, h), fill);
        let pt = (h / ui.s * SCI_LABEL_EM).min(MAC_LARGE_TITLE_PT);
        self.label(s, ui, label, pt, color, rect);
    }

    /// The binary row: one cell per bit of the word, set bits filled, bits outside the word dimmed.
    fn draw_bits(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let Some(p) = self.programmer.clone() else { return };
        let inset = ui.pt(SPACE_XXS);
        for bit in (0..64u8).rev() {
            let rect = self.bit_rect(ui, bit);
            let (x, y, w, h) = rect;
            let inside = bit < p.bits();
            let set = inside && p.bit(bit);
            if inside {
                let fill = if set { scientific() } else { FIELD_BG.now() };
                s.fill_rrect(RectF::new(x + inset, y, w - 2.0 * inset, h), ui.pt(m::SELECTION_RADIUS), fill);
            }
            let color = if !inside { LABEL_3.now() } else if set { LABEL.now() } else { LABEL_2.now() };
            self.label(s, ui, if set { "1" } else { "0" }, MAC_CALLOUT_PT, color, rect);
        }
    }

    fn draw_history(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (x, y, w, h) = self.panel(ui);
        let locale = self.settings.locale;
        let thousands = self.settings.thousands;
        let now = history::now_secs();
        let mut items = vec![SidebarItem::header("History")];
        if self.store.is_empty() {
            items.push(SidebarItem::row(u32::MAX, "No Recent Calculations").disabled());
        } else {
            for (index, entry) in self.store.entries().iter().enumerate() {
                let expression = localize_with(&pretty(&entry.expression), locale, thousands);
                let result = localize_with(&entry.result, locale, thousands);
                let mut row = SidebarItem::row(ID_HISTORY + index as u32, &expression).with_subtitle(&result);
                if history::starts_day(|k| self.store.time(k), index) {
                    let caption = history::day_caption(self.store.time(index), now);
                    if !caption.is_empty() { row = row.with_trailing(&caption); }
                }
                items.push(row);
            }
        }
        let scale = ui.pt(1.0).max(0.001);
        let row_h = (self.row_height(ui) / scale - 16.0).max(8.0);
        let list = SourceList::new(items, row_h, 72.0);
        let scroll = self.hist_offset as f32 * (row_h + 16.0);
        // The sidebar runs the panel's full height, so its footer row sits on the sidebar and not on the keypad behind
        // (E2E W4-7: the labels floated over the keys in an empty strip). Rows past the last visible one are left out so
        // none runs under the footer.
        let shown = 1 + self.hist_offset + self.rows_visible(ui);
        let list = SourceList::new(list.items.into_iter().take(shown).collect(), row_h, 72.0);
        let bounds = bifrost_app::controls::Rect { x: x / scale, y: y / scale, w: w / scale, h: h / scale };
        let pressed = match self.pressed { Some(Target::Result(i)) => Some(ID_HISTORY + i as u32), _ => None };
        widgets::source_list(s, ui, bounds, &list, scroll, pressed);
        // the footer: a hairline, then Copy Tape and Clear as two bordered buttons (left and right halves)
        let (row_y, half) = (y + h - ui.pt(64.0), w / 2.0);
        s.fill_rect(RectF::new(x + ui.pt(SPACE_L), row_y, w - 2.0 * ui.pt(SPACE_L), 1.0), SEPARATOR_C.now());
        let button = |left: f32, width: f32| {
            let (bw, bh) = ((width - 1.5 * ui.pt(SPACE_L)).max(ui.pt(SPACE_XXL)), ui.pt(28.0));
            (left + (width - bw) / 2.0, row_y + (ui.pt(64.0) - bh) / 2.0, bw, bh)
        };
        let (tape, clear_rect) = (button(x, half), button(x + half, w - half));
        let tape_color = if self.store.is_empty() { LABEL_3.now() } else { LABEL.now() };
        let fill = |pressed: bool| if pressed { key_down() } else { utility() };
        s.fill_rrect(RectF::new(tape.0, tape.1, tape.2, tape.3), capsule(tape.2, tape.3), fill(self.pressed == Some(Target::CopyTape)));
        self.label(s, ui, "Copy Tape", MAC_BODY_PT, tape_color, tape);
        let clear = if self.clear_confirm { CLEAR_CONFIRM_LABEL } else { "Clear" };
        let clear_color = if self.store.is_empty() { LABEL_3.now() } else { DESTRUCTIVE };
        s.fill_rrect(RectF::new(clear_rect.0, clear_rect.1, clear_rect.2, clear_rect.3), capsule(clear_rect.2, clear_rect.3), fill(self.pressed == Some(Target::ClearHistory)));
        self.label(s, ui, clear, MAC_BODY_PT, clear_color, clear_rect);
    }

    fn draw_menu(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (x, y, w, h) = self.menu_rect(ui);
        s.fill_rrect(RectF::new(x, y, w, h), ui.pt(m::SELECTION_RADIUS), FIELD_BG.now());
        s.fill_rrect(RectF::new(x + w / 2.0, y + ui.pt(SPACE_S), 1.0, h - ui.pt(SPACE_L)), 0.0, SEPARATOR_C.now());
        let can_paste = clipboard::text()
            .is_some_and(|t| paste::parse(&t, self.settings.locale).is_some());
        let can_copy = self.copy_text(ui).is_some();
        let copy = self.menu_item(ui, false);
        let paste_item = self.menu_item(ui, true);
        self.label(s, ui, "Copy", MAC_BODY_PT, if can_copy { LABEL.now() } else { LABEL_3.now() }, copy);
        self.label(s, ui, "Paste", MAC_BODY_PT, if can_paste { LABEL.now() } else { LABEL_3.now() }, paste_item);
    }

    pub fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui, _t: f64) {
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
        s.fill_rect(chrome::whole(0.0, 0.0, self.w as f32, self.h as f32), CARD_BG.now());
        let sci_on = self.sci_active(ui);
        let l = self.layout(ui);
        let top = self.deck_top(ui);
        let locale = self.settings.locale;
        let style = self.settings.style();
        let display = match &self.convert {
            Some(conv) => conv.result_text(locale),
            None => self.display_shown(ui),
        };
        let mut pt = self.display_pt_max(ui).floor();
        let room = l.width - ui.pt(SPACE_XL);
        while ui.measure(&display, pt, Weight::Regular) > room && pt > MAC_LARGE_TITLE_PT {
            pt -= 2.0;
        }
        let display = fit_tail(ui, &display, pt, room);
        let image = self.cache.get(ui, &display, pt, Weight::Regular, LABEL.now());
        let display_y = top - ui.pt(62.0) - image.h as f32;
        chrome::blit(s, image, (l.x + l.width - ui.pt(SPACE_S) - image.w as f32) as i32, display_y as i32);
        let expression = if self.programmer_active(ui) {
            self.expression_text(ui)
        } else {
            localize_with(&self.expression_text(ui), locale, style.thousands)
        };
        let expression = ui.fit(&expression, MAC_TITLE1_PT, l.width - ui.pt(SPACE_L));
        let image = self
            .cache
            .get(ui, &expression, MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
        chrome::blit(s, image, (l.x + l.width - ui.pt(SPACE_S) - image.w as f32) as i32, (display_y - image.h as f32 - ui.pt(SPACE_M)) as i32);
        if sci_on && self.sci.memory != 0.0 {
            let image = self.cache.get(ui, "M", MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
            chrome::blit(s, image, (l.x + ui.pt(SPACE_S)) as i32, display_y as i32);
        }
        if self.programmer_active(ui) {
            let readout = self.programmer.as_ref().map(Programmer::readout).unwrap_or_default();
            if !readout.is_empty() {
                let image = self.cache.get(ui, &readout, MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
                chrome::blit(s, image, (l.x + ui.pt(SPACE_S)) as i32, display_y as i32);
            }
            self.draw_bits(s, ui);
        }
        self.label(s, ui, "Delete", MAC_CALLOUT_PT, LABEL_2.now(), self.backspace_button(ui));
        for i in 0..self.key_count(ui) {
            self.draw_key(s, ui, i);
        }
        if self.convert.is_some() {
            self.draw_convert(s, ui);
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
    fn draw_bar(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let p = ui.pt(1.0);
        let side = if self.history_open { self.panel(ui).2 / p } else { 0.0 };
        chrome::toolbar_band(s, ui, Rect { x: side, y: 0.0, w: self.w as f32 / p - side, h: UNIFIED_BAR_PT });
        let pressed = match self.pressed {
            Some(Target::History) | Some(Target::Done) => Some(BAR_HISTORY),
            Some(Target::ShowConvert) | Some(Target::LeaveConvert) => Some(BAR_CONVERT),
            Some(Target::Scientific) => Some(if self.sci_active(ui) { BAR_BASIC } else { BAR_SCIENTIFIC }),
            Some(Target::ModeMenu) => Some(BAR_MODE),
            Some(Target::Mode(Mode::Basic)) => Some(BAR_BASIC),
            Some(Target::Mode(Mode::Scientific)) => Some(BAR_SCIENTIFIC),
            Some(Target::Mode(Mode::Programmer)) => Some(BAR_PROGRAMMER),
            _ => None,
        };
        chrome::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), pressed);
    }
}

mod menu;
pub use menu::{window_menu, MENU_ALWAYS_ON_TOP};

#[cfg(test)]
mod convert_tests;
#[cfg(test)]
mod mode_tests;
#[cfg(test)]
mod size_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format;
    use crate::format::localize;
    use crate::icon::icon;
    use bifrost_app::kit::Surface;

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

    /// A wide window showing the scientific keypad (chosen, as the desktop opens on Basic).
    fn landscape() -> (Ui, Calculator) {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1920, 1200);
        app.settings.scientific = true;
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
            assert!(app.menu_action(&ui, menu::MENU_DECIMAL_COMMA));
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
        tap(&mut app, &ui, px + pw * 0.75, py + ph - ui.pt(30.0));
        assert!(app.clear_confirm, "the first tap asks");
        assert_eq!(app.store.len(), 1);
        tap(&mut app, &ui, px + pw * 0.75, py + ph - ui.pt(30.0));
        assert!(app.store.is_empty(), "the second tap clears");
        assert!(!app.clear_confirm);
        let app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
        assert!(app.store.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_bottom_row_copies_the_tape_on_the_left_and_clears_on_the_right() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1200, 1920);
        app.store.push(Entry { expression: "2+3".into(), result: "5".into(), value: 5.0 });
        app.history_open = true;
        let (px, py, pw, ph) = app.panel(&ui);
        let y = py + ph - ui.pt(32.0);
        assert_eq!(app.hit(&ui, px + pw * 0.25, y), Some(Target::CopyTape));
        assert_eq!(app.hit(&ui, px + pw * 0.75, y), Some(Target::ClearHistory));
        let controls = app.controls(&ui);
        assert!(controls.iter().any(|n| n.label == "Copy Tape" && n.enabled));
        assert!(controls.iter().any(|n| n.label == "Clear History"));
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
        assert!(app.menu_action(&ui, menu::MENU_DECIMAL_COMMA));
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

    /// E2E W4-6: a landscape desktop window opens on Basic, and the Basic segment leaves Scientific.
    #[test]
    fn a_wide_desktop_window_opens_on_basic_and_the_basic_segment_leaves_scientific() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1875, 1290);
        assert!(!app.sci_active(&ui), "landscape alone does not pick the scientific keypad");
        assert_eq!(app.key_count(&ui), KEYS.len());
        let (x, y) = centre(app.bar_rect(&ui, BAR_SCIENTIFIC));
        tap(&mut app, &ui, x, y);
        assert!(app.sci_active(&ui));
        let (x, y) = centre(app.bar_rect(&ui, BAR_BASIC));
        tap(&mut app, &ui, x, y);
        assert!(!app.sci_active(&ui), "Basic leaves Scientific");
        assert!(app.bar(&ui).items.iter().any(|i| i.label == "Basic" && i.selected));
        assert!(app.menu_action(&ui, 11), "View > Scientific");
        assert!(app.sci_active(&ui));
        assert!(app.menu_action(&ui, 10), "View > Basic (Cmd+1)");
        assert!(!app.sci_active(&ui));
    }

    /// E2E W4-7: the decimal separator is a labelled View menu item, not an unlabelled toolbar chip.
    #[test]
    fn the_decimal_comma_is_a_view_menu_item_not_a_toolbar_chip() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1875, 1290);
        assert!(!app.bar(&ui).items.iter().any(|i| i.label == "1.5" || i.label == "1,5"));
        let checked = |app: &Calculator| {
            let menus = app.menus(&ui);
            let view = menus.iter().find(|m| m.title == "View").unwrap();
            view.items.iter().find(|i| i.title == "Decimal Comma (1,5)").unwrap().checked
        };
        assert!(!checked(&app));
        assert!(app.menu_action(&ui, menu::MENU_DECIMAL_COMMA));
        assert_eq!(app.settings.locale, format::Locale::Is);
        assert!(checked(&app));
        assert!(app.menu_action(&ui, menu::MENU_DECIMAL_COMMA));
        assert_eq!(app.settings.locale, format::Locale::En);
    }

    /// E2E W4-7: Convert's categories fit their titles, Swap is a filled button, and the display sits under the unit row.
    #[test]
    fn convert_titles_fit_and_the_display_stays_under_the_unit_row() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1875, 1290);
        let (x, y) = centre(app.bar_rect(&ui, BAR_CONVERT));
        tap(&mut app, &ui, x, y);
        assert!(app.convert.is_some());
        for (index, category) in Category::ALL.iter().enumerate() {
            let (cx, _, cw, _) = app.category_rect(&ui, index);
            assert!(cx >= 0.0 && cx + cw <= 1875.0, "{} inside the window", category.title());
            assert!(ui.measure(category.title(), MAC_CALLOUT_PT, Weight::Regular) <= cw - ui.pt(SPACE_S), "{} fits", category.title());
        }
        let [_, swap, to] = app.unit_rects(&ui);
        let units_bottom = to.1 + to.3;
        let pt = app.display_pt_max(&ui);
        assert!(pt > MAC_TITLE1_PT, "the display is not squeezed to its minimum, got {pt}");
        let lines_top = app.deck_top(&ui) - ui.pt(62.0) - ui.pt(pt * LINE_EM) - ui.pt(MAC_TITLE1_PT * LINE_EM + SPACE_M);
        assert!(lines_top >= units_bottom, "the entry and result lines start under the unit row");
        let mut buf = vec![0; 1875 * 1290];
        app.draw(&mut Surface { w: 1875, h: 1290, stride: 1875, px: &mut buf, y0: 0 }, &ui, 0.0);
        let (sx, sy) = ((swap.0 + swap.2 / 8.0) as usize, (swap.1 + swap.3 / 2.0) as usize);
        assert_eq!(buf[sy * 1875 + sx], utility(), "Swap has a capsule fill");
    }

    /// E2E W4-7: the history footer is on the sidebar, with Copy Tape and Clear as buttons.
    #[test]
    fn the_history_footer_sits_on_the_sidebar_with_a_clear_button() {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1875, 1290);
        for key in [Key::Digit(4), Key::Operator(Operator::Add), Key::Digit(2), Key::Equals] {
            tap_key(&mut app, &ui, key);
        }
        let (x, y) = centre(app.history_button(&ui));
        tap(&mut app, &ui, x, y);
        assert!(app.history_open);
        let mut buf = vec![0; 1875 * 1290];
        app.draw(&mut Surface { w: 1875, h: 1290, stride: 1875, px: &mut buf, y0: 0 }, &ui, 0.0);
        let (px, py, pw, ph) = app.panel(&ui);
        let row_y = py + ph - ui.pt(64.0);
        // left of the Clear label, inside its button: the button's fill, not a key's colour from behind
        let (bx, by) = ((px + pw * 0.625) as usize, (row_y + ui.pt(32.0)) as usize);
        assert_eq!(buf[by * 1875 + bx], utility());
        assert_eq!(app.hit(&ui, px + pw * 0.75, row_y + ui.pt(32.0)), Some(Target::ClearHistory));
    }
}
