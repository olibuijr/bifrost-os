//! bifrost 15 dark on-screen keyboard (Icelandic layout, as the reference
//! bifrost): letters, numbers and symbols pages with the flick-down
//! alternates printed on each key, shift/caps, globe and dismiss keys,
//! held-key auto-repeat, a slide-in/out animation and an optional terminal
//! accessory row (esc, ctrl, tab, arrows). Geometry is the bifrost's in points;
//! wider than the bifrost's 768 pt (landscape) the keys stretch to the width
//! and grow up to a third taller, as the bifrost's landscape keyboard does.
//!
//! Optional smart typing (`set_predictive`): a three-suggestion bar over the
//! keys, autocorrect on space (undone by backspace), double-space period and
//! auto-capitalisation. Long-pressing a letter slides out its accents, the
//! globe key opens the emoji page, and dragging the dismiss key (the dock
//! handle) undocks the keyboard into a narrow floating or a split-thumb layout.
//! Smart typing and the extra pages leave the default layout untouched.

use springboard::shell::cache::TextCache;
use crate::emoji::{self, Emoji};
use springboard::shell::glyphs;
use crate::kb_floating as fl;
use crate::kb_trackpad::{self, Trackpad};
use crate::kb_shortcutbar;
use crate::kb_pressed::{self, ReturnKind};
use crate::predict::{self, Predictor};
use springboard::shell::ui::{Ui, Weight, AVATAR_BG, AVATAR_FG, BLACK, GRAY_DIM, LABEL_2, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE2_PT, SYSTEM_BLUE, WHITE};
use springboard::gfx::Surface;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether prose keyboards (`new_prose`: notes, reminders) start with smart
/// typing on. Search, password and terminal keyboards never do. The shell
/// binary sets it.
static PREDICTIVE_DEFAULT: AtomicBool = AtomicBool::new(false);

pub fn set_predictive_default(on: bool) {
    PREDICTIVE_DEFAULT.store(on, Ordering::Relaxed);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Text(String),
    Backspace,
    Enter,
    Hide,
    /// terminal keys: "Escape", "Tab", "Paste", "Up", "Down", "Left", "Right"
    Named(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Letters,
    Numbers,
    Symbols,
    Emoji,
}

/// How the keyboard sits on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Docked,
    /// two thumb clusters with a gap in the middle
    Split,
    /// narrow, slid sideways by dragging the dock handle
    Floating,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Char(&'static str),
    Backspace,
    Enter,
    Shift,
    ToNumbers,
    ToLetters,
    ToSymbols,
    Space,
    Globe,
    /// dictation key beside the globe; dictation is not available yet
    Mic,
    Hide,
    Named(&'static str),
    Ctrl,
    /// the n-th prediction in the bar
    Suggest(usize),
    /// shortcut-bar glyph button (Undo, Redo, Paste)
    Bar(usize),
    /// the n-th emoji on the emoji page
    EmojiKey(usize),
    /// a tab of the emoji page (0 is Recents)
    EmojiTab(usize),
    /// the search field above the emoji grid
    EmojiSearch,
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    kind: Kind,
    /// the character a downward flick types
    alt: Option<&'static str>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub const BG: u32 = AVATAR_FG;
const KEY: u32 = AVATAR_BG;
const SPECIAL: u32 = GRAY_DIM;

// bifrost keyboard geometry in points (768 pt wide)
const MARGIN_PT: f32 = 6.0;
const PITCH_PT: f32 = 61.0;
const KEY_W_PT: f32 = 49.0;
const KEY_H_PT: f32 = 55.0;
const ROW_PT: f32 = 66.0;
const GAP_PT: f32 = PITCH_PT - KEY_W_PT;
const TOP_PT: f32 = 8.0;
const BOTTOM_PT: f32 = 6.0;
/// backspace, right shift and dismiss
const WIDE_PT: f32 = 85.0;
const RETURN_PT: f32 = 68.0;
/// bottom-left page key and globe
const PAGE_PT: f32 = 55.0;
/// bottom-right page key
const PAGE_R_PT: f32 = 95.0;
const ACCESSORY_PT: f32 = 50.0;
const ACCESSORY_KEY_PT: f32 = 38.0;
/// the second row of letters starts this far in
const ROW2_INSET_PT: f32 = 17.0;
/// finger travel that turns a tap into a flick to the alternate
const FLICK_PT: f32 = 18.0;
const REPEAT_DELAY_MS: f64 = 400.0;
const REPEAT_MS: f64 = 60.0;
const SLIDE_MS: f64 = 250.0;
const DOUBLE_TAP_MS: f64 = 350.0;
/// the prediction bar above the keys
const BAR_PT: f32 = 55.0;
const BAR_X_PT: f32 = 7.0;
const BAR_KEY_PT: f32 = 34.0;
/// a held letter slides out its accents after this long
const LONG_PRESS_MS: f64 = 450.0;
/// a second space this soon after the first types a period
const DOUBLE_SPACE_MS: f64 = 700.0;
/// dock handle drag that changes the mode, vertically and sideways
const DOCK_DRAG_PT: f32 = 24.0;
const DOCK_SIDE_PT: f32 = 60.0;
/// how much split halves and the floating keyboard shrink horizontally
const SPLIT_SCALE: f32 = 0.78;
const FLOAT_SCALE: f32 = 0.62;
const POPUP_GAP_PT: f32 = 8.0;
/// the bifrost selection blue, for the highlighted accent
const ACCENT_BLUE: u32 = SYSTEM_BLUE;

fn fade(color: u32, alpha: u8) -> u32 {
    let scale = |shift: u32| (((color >> shift) & 0xff) * alpha as u32 / 255) << shift;
    scale(0) | scale(8) | scale(16) | scale(24)
}

/// Cubic ease-out for the flick pull, so the alternate leads the finger.
fn ease_out_cubic(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

/// The variants a long press on `base` offers (see `kb_variants`).
fn accents(base: &str) -> &'static [&'static str] {
    crate::kb_variants::variants(base)
}

/// The accent strip shown above a held key; `sel` follows the finger.
struct Popup {
    cells: Vec<String>,
    x0: f32,
    y: f32,
    cell_w: f32,
    h: f32,
    sel: usize,
    /// the globe's vertical keyboard list
    vertical: bool,
}
/// The keyboard is laid out for this width in points and stretched beyond it.
const DESIGN_W_PT: f32 = 768.0;
/// How much taller the keys get at most (the bifrost's landscape/portrait ratio).
const MAX_TALLER: f32 = 4.0 / 3.0;

const LETTERS: [&[&str]; 3] = [
    &["q", "w", "e", "r", "t", "y", "u", "i", "o", "p", "ð"],
    &["a", "s", "d", "f", "g", "h", "j", "k", "l", "æ", "ö"],
    &["z", "x", "c", "v", "b", "n", "m", ",", ".", "þ"],
];
const NUMBERS: [&[&str]; 3] = [
    &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "`"],
    &["@", "#", "kr", "&", "*", "(", ")", "'", "\"", "+", "•"],
    &["%", "_", "-", "=", "/", ";", ":", "!", "?", "´"],
];
const SYMBOLS: [&[&str]; 3] = [
    &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "`"],
    &["€", "$", "£", "^", "[", "]", "{", "}", "—", "°", "…"],
    &["§", "|", "~", "≠", "\\", "<", ">", "!", "?", "´"],
];

impl Page {
    fn chars(self) -> [&'static [&'static str]; 3] {
        match self {
            Page::Letters => LETTERS,
            Page::Numbers => NUMBERS,
            Page::Symbols => SYMBOLS,
            Page::Emoji => LETTERS,
        }
    }

    /// The page a flick reaches: letters flick to numbers, numbers to symbols.
    fn alternates(self) -> Option<[&'static [&'static str]; 3]> {
        match self {
            Page::Letters => Some(NUMBERS),
            Page::Numbers => Some(SYMBOLS),
            Page::Symbols | Page::Emoji => None,
        }
    }

    /// (left corner key, right corner key, bottom page key)
    fn corners(self) -> (Kind, Kind, Kind) {
        match self {
            Page::Letters => (Kind::Shift, Kind::Shift, Kind::ToNumbers),
            Page::Numbers => (Kind::ToSymbols, Kind::ToSymbols, Kind::ToLetters),
            Page::Symbols => (Kind::ToNumbers, Kind::ToNumbers, Kind::ToLetters),
            Page::Emoji => (Kind::ToLetters, Kind::ToLetters, Kind::ToLetters),
        }
    }
}

#[derive(Clone, Copy)]
struct Pressed {
    i: usize,
    y0: f32,
    flick: bool,
    /// how far the finger has pulled the alternate down, 0..1 (1 = flick)
    pull: f32,
    /// auto-repeat has fired, so release types nothing
    repeated: bool,
    next_repeat: f64,
    x0: f32,
    last_x: f32,
    t0: f64,
    /// the dock handle is being dragged, so release types nothing
    dragging: bool,
}

pub struct Keyboard {
    hardware_input: bool,
    pub(super) clicks: springboard::shell::kb_clicks::Clicks,
    pub w: f32,
    pub h: f32,
    s: f32,
    /// horizontal and vertical stretch beyond the design width
    kx: f32,
    ky: f32,
    page: Page,
    shift: bool,
    caps: bool,
    /// Settings: Enable Caps Lock and the "." Shortcut
    kbp: springboard::shell::kb_capslock::KbPrefs,
    ctrl: bool,
    /// shift turns itself on at the start of a sentence
    auto_caps: bool,
    /// the last character typed, for sentence detection
    last: Option<char>,
    accessory: bool,
    keys: Vec<Key>,
    pressed: Option<Pressed>,
    /// action word of the return key
    return_kind: ReturnKind,
    last_shift_tap: f64,
    shown: bool,
    /// (start, showing) of the slide in progress
    slide: Option<(f64, bool)>,
    labels: TextCache,
    /// smart typing: the prediction bar, autocorrect, double-space period
    predictive: bool,
    suggestions: Vec<String>,
    /// the letters typed since the last word break, as typed
    word: String,
    /// the lower-case word before `word`, for the bigram bonus
    prev_word: Option<String>,
    /// keys still to send after the one `up` returned (corrections, emoji)
    queue: VecDeque<Out>,
    /// (typed, corrected, word before) while backspace can still undo an
    /// autocorrect
    undo_correction: Option<(String, String, Option<String>)>,
    last_space_t: f64,
    /// the character before the last space was a letter or digit
    space_after_alnum: bool,
    popup: Option<Popup>,
    mode: Mode,
    lay: springboard::shell::kb_layouts::Layout,
    /// left edge of the floating keyboard in px
    float_x: f32,
    emoji: Emoji,
    emoji_page: Vec<String>,
    /// suggestion labels rendered since the label cache was last cleared
    label_churn: u32,
    /// dismiss-key long-press menu, pinch tracker, grabber drag (last x)
    fl_menu: Option<fl::Menu>,
    fl_pinch: fl::Pinch,
    fl_grab: Option<f32>,
    /// space-bar trackpad mode (see kb_trackpad.rs)
    trackpad: Trackpad,
    /// key a shift or 123 quick slide started on, and its key index
    swipe: Option<(crate::kb_shiftswipe::Origin, usize)>,
}

impl Keyboard {
    /// `w` is the screen width in px, `s` px per point; `accessory` adds the
    /// terminal row. Starts shown, without animation.
    pub fn new(w: f32, s: f32, accessory: bool) -> Self {
        let mut k = Keyboard {
            // The shell decides whether a keyboard appears (a desktop with a hardware keyboard never shows one). The sandbox
            // sets BIFROST_HARDWARE_KEYBOARD for every app, so it must not silence this process.
            hardware_input: false,
            clicks: Default::default(),
            w,
            h: 0.0,
            s,
            kx: 1.0,
            ky: 1.0,
            page: Page::Letters,
            shift: false,
            caps: false,
            kbp: Default::default(),
            ctrl: false,
            auto_caps: false,
            last: None,
            accessory,
            keys: vec![],
            pressed: None,
            return_kind: ReturnKind::Return,
            last_shift_tap: -1e9,
            shown: true,
            slide: None,
            labels: TextCache::default(),
            predictive: false,
            suggestions: vec![],
            word: String::new(),
            prev_word: None,
            queue: VecDeque::new(),
            undo_correction: None,
            last_space_t: -1e9,
            space_after_alnum: false,
            popup: None,
            mode: Mode::Docked,
            lay: springboard::shell::kb_layouts::active(),
            float_x: w * (1.0 - FLOAT_SCALE) / 2.0,
            emoji: Emoji::on_device(),
            emoji_page: vec![],
            label_churn: 0,
            fl_menu: None,
            fl_pinch: fl::Pinch::default(),
            fl_grab: None,
            trackpad: Trackpad::default(),
            swipe: None,
        };
        k.layout();
        k
    }

    fn pt(&self, v: f32) -> f32 {
        v * self.s
    }

    /// Horizontal design points to px, stretched to the width.
    fn px(&self, v: f32) -> f32 {
        v * self.s * self.kx
    }

    /// Vertical design points to px, taller on a wide keyboard.
    fn py(&self, v: f32) -> f32 {
        v * self.s * self.ky
    }

    /// Adopts a new width (the device turned), keeping page and state.
    pub fn resize(&mut self, w: f32) {
        self.w = w;
        self.pressed = None;
        self.layout();
    }

    fn layout(&mut self) {
        if self.hardware_input { self.h = 0.0; self.keys.clear(); return; }
        self.kx = (self.w / self.s / DESIGN_W_PT).max(1.0);
        self.ky = self.kx.min(MAX_TALLER);
        self.lay = springboard::shell::kb_layouts::active();
        let chars = if self.page == Page::Letters { springboard::shell::kb_layouts::letters(self.lay) } else { self.page.chars() };
        let alts = self.page.alternates();
        let (left, right_corner, page_key) = self.page.corners();
        let right = self.w - self.px(MARGIN_PT);
        let (kw, kh, pitch) = (self.px(KEY_W_PT), self.py(KEY_H_PT), self.px(PITCH_PT));
        let mut keys = vec![];
        let mut y = self.py(TOP_PT);
        if self.predictive {
            let by = self.py((BAR_PT - BAR_KEY_PT) / 2.0);
            let bx = self.px(kb_shortcutbar::BUTTON_PT);
            let x0 = self.px(MARGIN_PT + BAR_X_PT);
            for i in 0..kb_shortcutbar::BUTTONS {
                keys.push(Key { kind: Kind::Bar(i), alt: None, x: x0 + i as f32 * bx, y: by, w: bx, h: self.py(BAR_KEY_PT) });
            }
            let from = kb_shortcutbar::predictions_from(x0, self.px(1.0));
            let bw = (self.w - self.px(MARGIN_PT) - from) / 3.0;
            for i in 0..3 {
                keys.push(Key { kind: Kind::Suggest(i), alt: None, x: from + i as f32 * bw, y: by, w: bw, h: self.py(BAR_KEY_PT) });
            }
            y += self.py(BAR_PT);
        }
        if self.accessory {
            let ah = self.py(ACCESSORY_KEY_PT);
            let wide = self.px(65.0);
            let mut x = self.px(MARGIN_PT);
            for kind in [Kind::Named("Paste"), Kind::Ctrl, Kind::Named("Escape"), Kind::Named("Tab"), Kind::Named("More")] {
                keys.push(Key { kind, alt: None, x, y, w: wide, h: ah });
                x += wide + self.px(GAP_PT);
            }
            let arrows = [Kind::Named("Left"), Kind::Named("Up"), Kind::Named("Down"), Kind::Named("Right")];
            let mut x = right - kw - 3.0 * pitch;
            for kind in arrows {
                keys.push(Key { kind, alt: None, x, y, w: kw, h: ah });
                x += pitch;
            }
            y += self.py(ACCESSORY_PT);
        }
        let push = |keys: &mut Vec<Key>, kind: Kind, alt: Option<&'static str>, x: f32, y: f32, w: f32| keys.push(Key { kind, alt, x, y, w, h: kh });
        let alt_of = |row: usize, i: usize, c: &str| alts.and_then(|a| a[row].get(i).copied()).filter(|a| *a != c);
        let row_x = [self.px(MARGIN_PT), self.px(MARGIN_PT + ROW2_INSET_PT), self.px(MARGIN_PT) + pitch];
        if self.page == Page::Emoji {
            push(&mut keys, Kind::EmojiSearch, None, self.px(MARGIN_PT), y, right - self.px(MARGIN_PT) - self.px(MARGIN_PT));
            y += self.py(ROW_PT);
            for row in 0..3 {
                let mut x = self.px(MARGIN_PT);
                for col in 0..11 {
                    push(&mut keys, Kind::EmojiKey(row * 11 + col), None, x, y, kw);
                    x += pitch;
                }
                match row {
                    0 => push(&mut keys, Kind::Backspace, None, right - self.px(WIDE_PT), y, self.px(WIDE_PT)),
                    1 => push(&mut keys, Kind::Enter, None, right - self.px(RETURN_PT), y, self.px(RETURN_PT)),
                    _ => {}
                }
                y += self.py(ROW_PT);
            }
            let (page_w, gap) = (self.px(PAGE_PT), self.px(GAP_PT));
            let hide_x = right - self.px(WIDE_PT);
            push(&mut keys, Kind::ToLetters, None, self.px(MARGIN_PT), y, page_w);
            let from = self.px(MARGIN_PT) + page_w + gap;
            let tab_w = (hide_x - gap - from - (emoji::TABS - 1) as f32 * gap) / emoji::TABS as f32;
            for i in 0..emoji::TABS {
                push(&mut keys, Kind::EmojiTab(i), None, from + i as f32 * (tab_w + gap), y, tab_w);
            }
            push(&mut keys, Kind::Hide, None, hide_x, y, self.px(WIDE_PT));
            self.h = (y + kh + self.py(BOTTOM_PT)).round();
            self.keys = self.apply_mode(keys);
            return;
        }
        for (row, line) in chars.iter().enumerate() {
            let mut x = row_x[row];
            for (i, &c) in line.iter().enumerate() {
                push(&mut keys, Kind::Char(c), alt_of(row, i, c), x, y, kw);
                x += pitch;
            }
            match row {
                0 => push(&mut keys, Kind::Backspace, None, right - self.px(WIDE_PT), y, self.px(WIDE_PT)),
                1 => push(&mut keys, Kind::Enter, None, right - self.px(RETURN_PT), y, self.px(RETURN_PT)),
                _ => {
                    push(&mut keys, left.clone(), None, self.px(MARGIN_PT), y, kw);
                    push(&mut keys, right_corner.clone(), None, right - self.px(WIDE_PT), y, self.px(WIDE_PT));
                }
            }
            y += self.py(ROW_PT);
        }
        let (page_w, gap) = (self.px(PAGE_PT), self.px(GAP_PT));
        let hide_x = right - self.px(WIDE_PT);
        let page_r_x = hide_x - gap - self.px(PAGE_R_PT);
        let space_x = self.px(MARGIN_PT) + 3.0 * (page_w + gap);
        push(&mut keys, page_key.clone(), None, self.px(MARGIN_PT), y, page_w);
        push(&mut keys, Kind::Globe, None, self.px(MARGIN_PT) + page_w + gap, y, page_w);
        push(&mut keys, Kind::Mic, None, self.px(MARGIN_PT) + 2.0 * (page_w + gap), y, page_w);
        push(&mut keys, Kind::Space, None, space_x, y, page_r_x - gap - space_x);
        push(&mut keys, page_key, None, page_r_x, y, self.px(PAGE_R_PT));
        push(&mut keys, Kind::Hide, None, hide_x, y, self.px(WIDE_PT));
        self.h = (y + kh + self.py(BOTTOM_PT)).round();
        self.keys = self.apply_mode(keys);
    }

    /// Squeezes the docked layout for the current mode: split halves shrink
    /// toward their own edges, the floating keyboard shrinks and slides.
    fn apply_mode(&self, mut keys: Vec<Key>) -> Vec<Key> {
        let (w, m) = (self.w, self.px(MARGIN_PT));
        match self.mode {
            Mode::Docked => {}
            Mode::Split => {
                for k in keys.iter_mut().filter(|k| !matches!(k.kind, Kind::Suggest(_) | Kind::Bar(_))) {
                    let nw = k.w * SPLIT_SCALE;
                    if k.x + k.w / 2.0 < w / 2.0 {
                        k.x = m + (k.x - m) * SPLIT_SCALE;
                    } else {
                        let r = (w - m) - ((w - m) - (k.x + k.w)) * SPLIT_SCALE;
                        k.x = r - nw;
                    }
                    k.w = nw;
                }
            }
            Mode::Floating => {
                for k in keys.iter_mut() {
                    k.x = self.float_x + k.x * FLOAT_SCALE;
                    k.w *= FLOAT_SCALE;
                }
            }
        }
        keys
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        // nearest key in the touched row so gaps never swallow a tap
        let half_gap = self.py(ROW_PT - KEY_H_PT) / 2.0;
        if self.mode == Mode::Floating {
            let (lo, hi) = (self.float_x, self.float_x + self.w * FLOAT_SCALE);
            if x < lo || x > hi {
                return None;
            }
        }
        self.keys
            .iter()
            .enumerate()
            .filter(|(_, k)| y >= k.y - half_gap && y < k.y + k.h + half_gap)
            .min_by(|(_, a), (_, b)| {
                let d = |k: &Key| if x < k.x { k.x - x } else if x > k.x + k.w { x - k.x - k.w } else { 0.0 };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    }

    fn repeats(kind: &Kind) -> bool {
        matches!(kind, Kind::Backspace | Kind::Named("Left" | "Right" | "Up" | "Down"))
    }

    /// Coordinates are relative to the keyboard's top-left.
    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.popup = None;
        self.fl_menu = None;
        if self.mode == Mode::Floating && fl::on_grabber(x, y, self.float_x, self.float_x + self.w * FLOAT_SCALE, self.h - self.pt(fl::GRABBER_BAND_PT), self.s) {
            self.fl_grab = Some(x);
            self.pressed = None;
            return;
        }
        self.pressed = self.hit(x, y).map(|i| Pressed { i, y0: y, flick: false, pull: 0.0, repeated: false, next_repeat: t + REPEAT_DELAY_MS, x0: x, last_x: x, t0: t, dragging: false });
        self.swipe = None;
        if let Some(i) = self.pressed.map(|p| p.i) {
            use crate::kb_shiftswipe::Origin;
            match self.keys[i].kind {
                Kind::Shift => self.swipe = Some((Origin::Shift, i)),
                Kind::ToNumbers if self.page == Page::Letters => {
                    // bifrost shows the symbols page while the finger is down
                    self.switch(Page::Numbers);
                    self.pressed = self.hit(x, y).map(|i| Pressed { i, y0: y, flick: false, pull: 0.0, repeated: false, next_repeat: t + REPEAT_DELAY_MS, x0: x, last_x: x, t0: t, dragging: false });
                    self.swipe = self.pressed.map(|p| (Origin::Numbers, p.i));
                }
                _ => {}
            }
        }
    }

    pub fn moved(&mut self, x: f32, y: f32) {
        if let Some(last) = self.fl_grab {
            let nx = fl::drag_x(self.float_x, x - last, self.w, FLOAT_SCALE);
            self.fl_grab = Some(x);
            if nx != self.float_x {
                self.float_x = nx;
                self.layout();
            }
            return;
        }
        if let Some(m) = &mut self.fl_menu {
            m.track(y);
            return;
        }
        if self.trackpad.active() {
            let outs = self.trackpad.moved(x, y, self.s);
            if !outs.is_empty() {
                self.break_word();
            }
            self.queue.extend(outs);
            return;
        }
        if let Some(pop) = &mut self.popup {
            let i = if pop.vertical { ((y - pop.y) / pop.h).floor().max(0.0) as usize } else { ((x - pop.x0) / pop.cell_w).floor().max(0.0) as usize };
            pop.sel = i.min(pop.cells.len() - 1);
            return;
        }
        let Some(mut p) = self.pressed else { return };
        if self.keys[p.i].kind == Kind::Hide {
            self.drag_handle(&mut p, x, y);
            self.pressed = Some(p);
            return;
        }
        if self.keys[p.i].alt.is_some() {
            p.pull = ((y - p.y0) / self.pt(FLICK_PT)).clamp(0.0, 1.0);
            if p.pull >= 1.0 {
                p.flick = true;
                self.pressed = Some(p);
                return;
            }
        }
        p.flick = false;
        self.pressed = match self.hit(x, y) {
            Some(i) if i == p.i => Some(p),
            Some(i) => Some(Pressed { i, y0: y, pull: 0.0, repeated: false, ..p }),
            None => None,
        };
    }

    /// The dismiss key doubles as the dock handle: dragging it up undocks
    /// into the floating keyboard, down docks again, sideways toggles the
    /// split layout, and once floating it carries the keyboard sideways.
    fn drag_handle(&mut self, p: &mut Pressed, x: f32, y: f32) {
        let (dx, dy) = (x - p.x0, y - p.y0);
        if !p.dragging {
            let (up, side) = (self.pt(DOCK_DRAG_PT), self.pt(DOCK_SIDE_PT));
            let next = if dy <= -up && self.mode != Mode::Floating {
                Some(Mode::Floating)
            } else if dy >= up && self.mode != Mode::Docked {
                Some(Mode::Docked)
            } else if dx.abs() >= side && self.mode != Mode::Floating {
                Some(if self.mode == Mode::Docked { Mode::Split } else { Mode::Docked })
            } else if self.mode == Mode::Floating && dx.abs() >= self.pt(8.0) {
                Some(Mode::Floating)
            } else {
                None
            };
            let Some(mode) = next else { return };
            p.dragging = true;
            p.last_x = x;
            self.set_mode(mode);
            return;
        }
        if self.mode == Mode::Floating {
            let max = self.w * (1.0 - FLOAT_SCALE);
            let nx = (self.float_x + x - p.last_x).clamp(0.0, max);
            if nx != self.float_x {
                self.float_x = nx;
                self.layout();
            }
            p.last_x = x;
        }
    }

    /// Docks, splits or floats the keyboard.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.mode = mode;
            self.popup = None;
            self.layout();
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Turns the prediction bar, autocorrect, double-space period and
    /// word learning on or off; the bar changes the keyboard's height.
    pub fn set_predictive(&mut self, on: bool) {
        if on != self.predictive {
            self.predictive = on;
            self.pressed = None;
            self.reset_typing();
            self.layout();
        }
    }

    pub fn predictive(&self) -> bool {
        self.predictive
    }

    /// A keyboard without the accessory row that starts off screen, to be
    /// slid in with `set_shown` when a field is tapped.
    pub fn new_hidden(w: f32, s: f32) -> Self {
        let mut k = Keyboard::new(w, s, false);
        k.shown = false;
        k
    }

    /// Like `new_hidden`, for free text: smart typing follows the default set
    /// by `set_predictive_default`.
    pub fn new_prose(w: f32, s: f32) -> Self {
        let mut k = Keyboard::new_hidden(w, s);
        k.set_predictive(PREDICTIVE_DEFAULT.load(Ordering::Relaxed));
        k
    }

    /// Starts hidden and slides in: for a keyboard that appears with a sheet.
    pub fn appear(&mut self, t: f64) {
        self.shown = false;
        self.set_shown(true, t);
    }

    pub fn up(&mut self, x: f32, y: f32, t: f64) -> Option<Out> {
        if self.fl_grab.take().is_some() {
            return None;
        }
        if let Some(m) = self.fl_menu.take() {
            self.pressed = None;
            self.set_mode(m.chosen());
            return None;
        }
        if let Some(pop) = self.popup.take() {
            self.pressed = None;
            if pop.vertical {
                return self.pick_layout(&pop.cells[pop.sel]);
            }
            return self.type_text(&pop.cells[pop.sel]);
        }
        if self.trackpad.active() {
            self.trackpad.end(t);
            self.pressed = None;
            return None;
        }
        let p = self.pressed.take()?;
        if p.repeated || p.dragging {
            return None;
        }
        if let Some((origin, start)) = self.swipe.take() {
            use crate::kb_shiftswipe::{on_release, Action};
            let is_char = matches!(self.keys[p.i].kind, Kind::Char(_));
            match on_release(Some(origin), is_char && self.hit(x, y) == Some(p.i), p.i == start) {
                Action::TypeUpper => {
                    let Kind::Char(c) = self.keys[p.i].kind else { return None };
                    let (shift, caps) = (self.shift, self.caps);
                    self.shift = true;
                    let out = self.type_text(c);
                    self.shift = shift;
                    self.caps = caps;
                    self.refresh_suggestions();
                    return out;
                }
                Action::TypeThenLetters => {
                    let Kind::Char(c) = self.keys[p.i].kind else { return None };
                    let out = self.type_text(c);
                    self.switch(Page::Letters);
                    return out;
                }
                Action::Stay => return None,
                Action::Normal => {}
            }
        }
        if p.flick {
            let alt = self.keys[p.i].alt?;
            return self.type_other(alt);
        }
        if self.hit(x, y) != Some(p.i) {
            return None;
        }
        self.activate(p.i, t)
    }

    /// Clicks queued by key presses; the shell plays them if Keyboard Clicks is on.
    pub fn take_clicks(&mut self) -> Vec<springboard::shell::kb_clicks::Click> {
        self.clicks.take()
    }

    fn activate(&mut self, i: usize, t: f64) -> Option<Out> {
        let kind = self.keys[i].kind.clone();
        if let Some(out) = self.emoji_search_key(&kind) {
            return out;
        }
        self.clicks.push(match kind {
            Kind::Backspace => springboard::shell::kb_clicks::Click::Delete,
            Kind::Char(_) | Kind::Space => springboard::shell::kb_clicks::Click::Key,
            _ => springboard::shell::kb_clicks::Click::Modifier,
        });
        match kind {
            Kind::Char(c) => self.type_text(c),
            Kind::Space => self.space(t),
            Kind::Backspace => self.backspace(),
            Kind::Enter => {
                self.last = None;
                self.finish_word();
                self.prev_word = None;
                self.start_sentence();
                Some(Out::Enter)
            }
            Kind::Hide => Some(Out::Hide),
            Kind::Mic => None,
            Kind::Named(n) => {
                if matches!(n, "Left" | "Right" | "Up" | "Down") {
                    self.break_word();
                }
                Some(Out::Named(n))
            }
            Kind::Globe => {
                if let Some(l) = springboard::shell::kb_layouts::next(self.page == Page::Emoji) {
                    return self.pick_layout(l.name());
                }
                if self.page == Page::Emoji {
                    return self.switch(Page::Letters);
                }
                if !springboard::emoji::available() {
                    return None;
                }
                if self.emoji.tab() == 0 && self.emoji.recent().is_empty() {
                    self.emoji.set_tab(1);
                }
                self.switch(Page::Emoji)
            }
            Kind::Ctrl => {
                self.ctrl = !self.ctrl;
                None
            }
            Kind::Shift => {
                if self.caps {
                    self.caps = false;
                    self.shift = false;
                } else if self.kbp.enable_caps_lock && t - self.last_shift_tap < DOUBLE_TAP_MS {
                    self.caps = true;
                } else {
                    self.shift = !self.shift;
                }
                self.last_shift_tap = t;
                self.refresh_suggestions();
                None
            }
            Kind::ToNumbers => self.switch(Page::Numbers),
            Kind::ToSymbols => self.switch(Page::Symbols),
            Kind::ToLetters => self.switch(Page::Letters),
            Kind::Suggest(n) => self.pick_suggestion(n),
            Kind::Bar(n) => Some(Out::Named(kb_shortcutbar::command(n))),
            Kind::EmojiKey(n) => {
                let e = self.emoji_page.get(n)?.clone();
                self.emoji.used(&e);
                self.last = None;
                Some(Out::Text(e))
            }
            Kind::EmojiSearch => {
                self.emoji.begin_search();
                self.switch(Page::Letters)
            }
            Kind::EmojiTab(n) => {
                self.emoji.set_tab(n);
                self.emoji_page = self.emoji.page();
                self.pressed = None;
                None
            }
        }
    }

    /// Types a letter or symbol key's text: shift and caps lock apply, a held
    /// ctrl turns it into a control code, and smart typing follows the word.
    fn type_text(&mut self, c: &str) -> Option<Out> {
        let text = if self.shift || self.caps { c.to_uppercase() } else { c.to_string() };
        if self.shift && !self.caps {
            self.shift = false;
        }
        self.last = c.chars().next();
        if self.ctrl {
            self.ctrl = false;
            // One source for Ctrl bytes: the terminal-keys SDK kit. Keys
            // Ctrl does not combine with are dropped, consuming the Ctrl tap
            // instead of sending a bogus code.
            let byte = c.chars().next().and_then(bifrost_app::controls::terminal_keys::control_byte);
            return byte.map(|b| Out::Text((b as char).to_string()));
        }
        self.track(&text);
        Some(Out::Text(text))
    }

    /// Types a flicked alternate: never shifted, never part of a word.
    fn type_other(&mut self, text: &str) -> Option<Out> {
        self.last = text.chars().next();
        self.finish_word();
        if text.chars().any(|c| matches!(c, '.' | '!' | '?')) {
            self.prev_word = None;
        }
        self.undo_correction = None;
        self.refresh_suggestions();
        Some(Out::Text(text.to_string()))
    }

    fn space(&mut self, t: f64) -> Option<Out> {
        // Ctrl+Space is NUL (readline set-mark), from the terminal-keys SDK
        // kit; it never ends a word or triggers smart typing.
        if self.ctrl {
            self.ctrl = false;
            self.finish_word();
            self.last = None;
            return Some(Out::Text('\0'.to_string()));
        }
        let after_alnum = self.last.is_some_and(char::is_alphanumeric);
        let sentence_end = matches!(self.last, Some('.' | '!' | '?'));
        self.undo_correction = None;
        let mut outs = vec![];
        if self.predictive && self.kbp.period_shortcut && self.last == Some(' ') && self.space_after_alnum && t - self.last_space_t < DOUBLE_SPACE_MS {
            // "word  " becomes "word. "
            self.prev_word = None;
            self.last = Some(' ');
            self.space_after_alnum = false;
            self.last_space_t = t;
            self.start_sentence();
            return self.emit(vec![Out::Backspace, Out::Text(". ".into())]);
        }
        if self.predictive && !self.ctrl && !self.word.is_empty() {
            let typed = self.word.clone();
            if let Some(fixed) = self.autocorrect(&typed) {
                outs = (0..typed.chars().count()).map(|_| Out::Backspace).collect();
                outs.push(Out::Text(format!("{fixed} ")));
                let before = self.prev_word.clone();
                self.with_pred(|p| p.learn(before.as_deref().unwrap_or(""), &fixed));
                self.prev_word = Some(fixed.to_lowercase());
                self.word.clear();
                self.undo_correction = Some((typed, fixed, before));
            }
        }
        if outs.is_empty() {
            self.finish_word();
            outs.push(Out::Text(" ".into()));
        }
        if sentence_end {
            self.start_sentence();
        }
        self.space_after_alnum = after_alnum;
        self.last = Some(' ');
        self.last_space_t = t;
        self.refresh_suggestions();
        self.emit(outs)
    }

    /// The correction for the word just typed, if smart typing has one.
    fn autocorrect(&self, typed: &str) -> Option<String> {
        let letters = typed.chars().count();
        if letters > 24 {
            return None;
        }
        if typed == "i" {
            return Some("I".into());
        }
        // an all-capitals word is an acronym
        if letters > 1 && typed.chars().all(char::is_uppercase) {
            return None;
        }
        let prev = self.prev_word.clone();
        self.with_pred(|p| p.correct(prev.as_deref(), typed)).flatten()
    }

    /// The caret moved without typing (arrows, trackpad, a tap in the
    /// field): the word being typed, the last char and a pending correction
    /// no longer describe the text before it.
    pub fn caret_moved(&mut self) {
        self.break_word();
    }

    fn break_word(&mut self) {
        self.word.clear();
        self.prev_word = None;
        self.undo_correction = None;
        self.last = None;
        self.refresh_suggestions();
    }

    fn backspace(&mut self) -> Option<Out> {
        self.last = None;
        if let Some((typed, fixed, before)) = self.undo_correction.take() {
            // take the correction and its space back, leaving what was typed
            let mut outs: Vec<Out> = (0..fixed.chars().count() + 1).map(|_| Out::Backspace).collect();
            outs.push(Out::Text(typed.clone()));
            self.with_pred(|p| p.add_word(&typed));
            self.last = typed.chars().last();
            self.prev_word = before;
            self.word = typed;
            self.refresh_suggestions();
            return self.emit(outs);
        }
        if self.predictive {
            self.word.pop();
            self.refresh_suggestions();
        }
        Some(Out::Backspace)
    }

    /// Replaces the word being typed with suggestion `n` and a space.
    fn pick_suggestion(&mut self, n: usize) -> Option<Out> {
        let sug = self.suggestions.get(n)?.clone();
        let mut outs: Vec<Out> = (0..self.word.chars().count()).map(|_| Out::Backspace).collect();
        outs.push(Out::Text(format!("{sug} ")));
        let before = self.prev_word.clone();
        self.with_pred(|p| p.learn(before.as_deref().unwrap_or(""), &sug));
        self.prev_word = Some(sug.to_lowercase());
        self.word.clear();
        self.undo_correction = None;
        self.last = Some(' ');
        self.space_after_alnum = true;
        self.last_space_t = -1e9;
        if !self.caps {
            self.shift = false;
        }
        self.refresh_suggestions();
        self.emit(outs)
    }

    /// Returns the first key now and holds the rest for the next `tick`.
    fn emit(&mut self, mut outs: Vec<Out>) -> Option<Out> {
        if outs.is_empty() {
            return None;
        }
        let first = outs.remove(0);
        self.queue.extend(outs);
        Some(first)
    }

    fn with_pred<R>(&self, f: impl FnOnce(&mut Predictor) -> R) -> Option<R> {
        if !self.predictive {
            return None;
        }
        let mut guard = predict::shared().lock().ok()?;
        Some(f(&mut guard))
    }

    /// Follows typed text: letters extend the word, anything else ends it.
    fn track(&mut self, text: &str) {
        if !self.predictive {
            return;
        }
        self.undo_correction = None;
        let letter = self.page == Page::Letters && !text.is_empty() && text.chars().all(char::is_alphabetic);
        if letter {
            self.word.push_str(text);
        } else {
            self.finish_word();
            if text.chars().any(|c| matches!(c, '.' | '!' | '?')) {
                self.prev_word = None;
            }
        }
        self.refresh_suggestions();
    }

    /// The word being typed ends: learn it and the pair it made.
    fn finish_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let before = self.prev_word.clone();
        self.with_pred(|p| p.learn(before.as_deref().unwrap_or(""), &word));
        self.prev_word = Some(word.to_lowercase());
    }

    fn reset_typing(&mut self) {
        self.word.clear();
        self.prev_word = None;
        self.undo_correction = None;
        self.queue.clear();
        self.refresh_suggestions();
    }

    /// Recomputes the bar for the word so far (or the next word).
    fn refresh_suggestions(&mut self) {
        self.suggestions.clear();
        if self.emoji.searching() {
            self.suggestions = self.emoji.results().into_iter().take(3).collect();
            return;
        }
        if !self.predictive {
            return;
        }
        let prefix = self.word.to_lowercase();
        let before = self.prev_word.clone();
        let found = self.with_pred(|p| p.suggest(before.as_deref(), &prefix, 3)).unwrap_or_default();
        let typed = if self.word.is_empty() && (self.shift || self.caps) { "A".to_string() } else { self.word.clone() };
        self.suggestions = found.into_iter().map(|w| predict::match_case(&typed, &w)).collect();
        // the bar's labels are cached by text, so keep that cache bounded
        self.label_churn += 3;
        if self.label_churn > 400 {
            self.labels.clear();
            self.label_churn = 0;
        }
    }

    /// While the emoji search field has focus the letter page edits its query
    /// instead of typing; `Some` is the key's result.
    fn emoji_search_key(&mut self, kind: &Kind) -> Option<Option<Out>> {
        if !self.emoji.searching() {
            return None;
        }
        match kind {
            Kind::Char(c) => self.emoji.search_type(c),
            Kind::Space => self.emoji.search_type(" "),
            Kind::Backspace => self.emoji.search_backspace(),
            Kind::Enter | Kind::Globe => {
                self.emoji.end_search();
                return Some(self.switch(Page::Emoji));
            }
            Kind::Suggest(n) => {
                let e = self.suggestions.get(*n)?.clone();
                self.emoji.used(&e);
                return Some(Some(Out::Text(e)));
            }
            _ => return None,
        }
        self.refresh_suggestions();
        Some(None)
    }

    fn switch(&mut self, p: Page) -> Option<Out> {
        if p != Page::Letters {
            self.emoji.end_search();
        }
        self.finish_word();
        self.page = p;
        self.popup = None;
        self.pressed = None;
        if p == Page::Emoji {
            self.emoji_page = self.emoji.page();
        }
        self.layout();
        self.refresh_suggestions();
        None
    }

    /// Auto-repeat for a held backspace or arrow, and the end of a slide.
    /// Returns the keys to type now.
    pub fn tick(&mut self, t: f64) -> Vec<Out> {
        let mut outs: Vec<Out> = self.queue.drain(..).collect();
        if self.slide.is_some_and(|(start, _)| t - start >= SLIDE_MS) {
            self.slide = None;
        }
        self.open_popup(t);
        self.open_float_menu(t);
        if let Some(p) = self.pressed {
            if !self.trackpad.active() && self.keys[p.i].kind == Kind::Space && t - p.t0 >= kb_trackpad::HOLD_MS {
                self.trackpad.start(p.x0, p.y0, t);
            }
        }
        if let Some(p) = &mut self.pressed {
            if Self::repeats(&self.keys[p.i].kind) && t >= p.next_repeat {
                p.repeated = true;
                p.next_repeat = t + REPEAT_MS;
                let i = p.i;
                outs.extend(self.activate(i, t));
            }
        }
        outs
    }

    /// Holding the dismiss key opens the Dock / Split / Floating menu.
    fn open_float_menu(&mut self, t: f64) {
        if self.fl_menu.is_some() {
            return;
        }
        let Some(p) = self.pressed.as_mut() else { return };
        if p.dragging || t - p.t0 < LONG_PRESS_MS || self.keys[p.i].kind != Kind::Hide {
            return;
        }
        p.dragging = true;
        let k = &self.keys[p.i];
        self.fl_menu = Some(fl::Menu::new(k.x, k.y, k.w, self.w, self.s, self.mode));
    }

    /// Two-finger pinch (distance between the fingers in px): pinching in
    /// floats the keyboard, pinching out docks it again. `end` on release.
    pub fn pinch(&mut self, dist: f32) {
        if let Some(mode) = self.fl_pinch.update(dist, self.mode) {
            self.pressed = None;
            self.set_mode(mode);
        }
    }

    pub fn pinch_end(&mut self) {
        self.fl_pinch.end();
    }

    /// A letter held long enough slides out its accents, the letter itself
    /// first (under the finger), so releasing without moving types it.
    fn open_popup(&mut self, t: f64) {
        if self.popup.is_some() || self.page == Page::Emoji {
            return;
        }
        let Some(p) = self.pressed else { return };
        if p.flick || p.repeated || p.dragging || t - p.t0 < LONG_PRESS_MS {
            return;
        }
        if self.keys[p.i].kind == Kind::Globe {
            self.open_layout_popup(p.i);
            return;
        }
        if self.page != Page::Letters {
            return;
        }
        let Kind::Char(c) = self.keys[p.i].kind.clone() else { return };
        let variants: Vec<&str> = accents(c).iter().copied().filter(|v| springboard::shell::kb_layouts::accent_allowed(self.lay, v)).collect();
        if variants.is_empty() {
            return;
        }
        let key = &self.keys[p.i];
        let (cell_w, h, m) = (key.w, self.py(KEY_H_PT), self.px(MARGIN_PT));
        let mut cells: Vec<String> = std::iter::once(c).chain(variants.iter().copied()).map(str::to_string).collect();
        let total = cell_w * cells.len() as f32;
        let (x0, sel) = if key.x + total <= self.w - m {
            (key.x, 0)
        } else {
            cells.reverse();
            (key.x + key.w - total, cells.len() - 1)
        };
        self.popup = Some(Popup { cells, x0, y: key.y - h - self.pt(POPUP_GAP_PT), cell_w, h, sel, vertical: false });
    }

    /// Long-press globe: the keyboard list rises above the key.
    fn open_layout_popup(&mut self, i: usize) {
        let cells = springboard::shell::kb_layouts::menu();
        let key = &self.keys[i];
        let (h, cell_w) = (self.py(KEY_H_PT), self.pt(190.0));
        let y = key.y - self.pt(POPUP_GAP_PT) - h * cells.len() as f32;
        let x0 = key.x.min(self.w - self.px(MARGIN_PT) - cell_w);
        self.popup = Some(Popup { cells, x0, y, cell_w, h, sel: 0, vertical: true });
    }

    /// Switches to the named layout (or the Emoji page) and relays out.
    fn pick_layout(&mut self, name: &str) -> Option<Out> {
        match springboard::shell::kb_layouts::by_name(name) {
            Some(l) => {
                springboard::shell::kb_layouts::set_active(l);
                self.page = Page::Letters;
                self.switch(Page::Letters)
            }
            None if !springboard::emoji::available() => None,
            None => {
                if self.emoji.tab() == 0 && self.emoji.recent().is_empty() {
                    self.emoji.set_tab(1);
                }
                self.switch(Page::Emoji)
            }
        }
    }

    /// Slides the keyboard in or out over a quarter second.
    pub fn set_shown(&mut self, shown: bool, t: f64) {
        if shown != self.shown {
            self.shown = shown;
            self.slide = if self.hardware_input { None } else { Some((t, shown)) };
            self.pressed = None;
            self.popup = None;
            if shown {
                if springboard::shell::kb_layouts::active() != self.lay {
                    self.layout();
                }
                self.reset_typing();
                self.start_sentence();
            }
        }
    }

    /// Auto-Capitalisation (Settings › General › Keyboard): shift comes on
    /// when the keyboard appears and after a sentence ends.
    pub fn set_auto_caps(&mut self, on: bool) {
        self.auto_caps = on;
        self.last = None;
        if !on && !self.caps {
            self.shift = false;
        } else {
            self.start_sentence();
        }
    }

    /// Settings > General > Keyboard: Enable Caps Lock and "." Shortcut.
    pub fn set_kb_prefs(&mut self, p: springboard::shell::kb_capslock::KbPrefs) {
        self.kbp = p;
        if !p.enable_caps_lock {
            self.caps = false;
        }
    }

    fn start_sentence(&mut self) {
        if self.auto_caps && !self.caps && self.page == Page::Letters {
            self.shift = true;
            self.refresh_suggestions();
        }
    }

    /// Display Zoom: re-lays the keys out at `s` px per point.
    pub fn set_scale(&mut self, s: f32) {
        self.s = s;
        self.layout();
        self.labels.clear();
    }

    /// Drops cached labels so they re-render at a new text size.
    pub fn clear_labels(&mut self) {
        self.labels.clear();
    }

    /// Where the keyboard rests (ignoring the slide in progress).
    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn animating(&self) -> bool {
        // a held letter needs frames to notice the long press, and queued
        // corrections need a tick to go out
        let waiting = self.popup.is_none() && self.page != Page::Emoji && self.pressed.is_some_and(|p| matches!(self.keys[p.i].kind, Kind::Char(_)));
        let space = self.pressed.is_some_and(|p| self.keys[p.i].kind == Kind::Space);
        self.slide.is_some() || !self.queue.is_empty() || waiting || space || self.trackpad.animating()
    }

    /// How far below its resting place the keyboard is drawn at `t`: 0 when
    /// shown, its height when hidden, in between while sliding (ease-out).
    pub fn offset(&self, t: f64) -> f32 {
        match self.slide {
            Some((start, showing)) => {
                let p = ((t - start) / SLIDE_MS).clamp(0.0, 1.0) as f32;
                let e = 1.0 - (1.0 - p).powi(3);
                if showing {
                    self.h * (1.0 - e)
                } else {
                    self.h * e
                }
            }
            None if self.shown => 0.0,
            None => self.h,
        }
    }

    pub fn set_return_kind(&mut self, kind: ReturnKind) {
        self.return_kind = kind;
    }

    /// Logical name of a key, for tests and the accessory labels.
    fn name(&self, k: &Kind) -> String {
        match k {
            Kind::Char(c) if self.shift || self.caps => c.to_uppercase(),
            Kind::Char(c) => c.to_string(),
            Kind::Backspace => "⌫".into(),
            Kind::Enter => "return".into(),
            Kind::Shift => if self.caps { "⇪" } else { "⇧" }.into(),
            Kind::ToNumbers => "123".into(),
            Kind::ToSymbols => "#+=".into(),
            Kind::ToLetters => "ABC".into(),
            Kind::Space => String::new(),
            Kind::Globe => "globe".into(),
            Kind::Mic => "mic".into(),
            Kind::Hide => "hide".into(),
            Kind::Ctrl => "ctrl".into(),
            Kind::Suggest(n) => format!("suggest{n}"),
            Kind::Bar(n) => format!("bar{n}"),
            Kind::EmojiKey(n) => self.emoji_page.get(*n).cloned().unwrap_or_default(),
            Kind::EmojiTab(n) => format!("tab{n}"),
            Kind::EmojiSearch => "emoji-search".into(),
            Kind::Named("Escape") => "esc".into(),
            Kind::Named("Tab") => "tab".into(),
            Kind::Named("Paste") => "paste".into(),
            Kind::Named("More") => "more".into(),
            Kind::Named("Left") => "←".into(),
            Kind::Named("Up") => "↑".into(),
            Kind::Named("Down") => "↓".into(),
            Kind::Named("Right") => "→".into(),
            Kind::Named(n) => n.to_string(),
        }
    }

    /// Centre of the key named `name`, relative to the keyboard.
    #[cfg(test)]
    pub fn centre(&self, name: &str) -> Option<(f32, f32)> {
        self.keys.iter().find(|k| self.name(&k.kind) == name).map(|k| (k.x + k.w / 2.0, k.y + k.h / 2.0))
    }

    /// Draws the prediction bar and emoji page keys; false for any other key.
    fn draw_extra(&mut self, s: &mut Surface, ui: &Ui, k: &Key, oy: f32, down: bool, r: f32) -> bool {
        let y = oy + k.y;
        let label = match k.kind {
            Kind::Bar(n) => {
                if down {
                    s.fill_rrect(k.x, y, k.w, k.h, r, SPECIAL);
                }
                kb_shortcutbar::draw_glyph(s, n, (k.x + k.w / 2.0, y + k.h / 2.0), self.pt(8.0), self.pt(1.4), WHITE);
                return true;
            }
            Kind::Suggest(n) => {
                if down {
                    s.fill_rrect(k.x, y, k.w, k.h, r, SPECIAL);
                }
                if n > 0 {
                    s.fill_rect(k.x as usize, (y + k.h * 0.2).max(0.0) as usize, 1, (k.h * 0.6) as usize, SPECIAL);
                }
                self.suggestions.get(n).cloned().map(|t| (t, 18.0))
            }
            Kind::EmojiKey(n) => {
                if down {
                    s.fill_rrect(k.x, y, k.w, k.h, r, SPECIAL);
                }
                self.emoji_page.get(n).cloned().map(|t| (t, 30.0))
            }
            Kind::EmojiSearch => {
                s.fill_rrect(k.x, y, k.w, k.h, self.pt(6.0), SPECIAL);
                let q = self.emoji.query();
                let (text, color) = if q.is_empty() { ("Search Emoji".to_string(), LABEL_2.now()) } else { (q.to_string(), WHITE) };
                let ox = (k.x + self.px(MARGIN_PT)) as i32;
                let img = self.labels.get(ui, &text, MAC_TITLE2_PT, Weight::Regular, color);
                s.blit(img, ox, (y + k.h / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
                return true;
            }
            Kind::EmojiTab(n) => {
                if down || n == self.emoji.tab() {
                    s.fill_rrect(k.x, y, k.w, k.h, r, SPECIAL);
                }
                Some((Emoji::icon(n).to_string(), MAC_TITLE1_PT))
            }
            _ => return false,
        };
        if let Some((text, size)) = label {
            let img = self.labels.get(ui, &text, size, Weight::Regular, WHITE);
            s.blit(img, (k.x + k.w / 2.0 - img.w as f32 / 2.0) as i32, (y + k.h / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
        }
        true
    }

    /// The accent strip above a held letter, the chosen cell in blue.
    fn draw_popup(&mut self, s: &mut Surface, ui: &Ui, oy: f32) {
        let Some(pop) = &self.popup else { return };
        let (x0, y, cw, h, sel) = (pop.x0, oy + pop.y, pop.cell_w, pop.h, pop.sel);
        let (cells, vertical) = (pop.cells.clone(), pop.vertical);
        let r = self.pt(5.0).min(6.0);
        let upper = self.shift || self.caps;
        if vertical {
            let n = cells.len() as f32;
            s.fill_rrect(x0 - 2.0, y - 2.0, cw + 4.0, h * n + 4.0, r, BG);
            for (i, cell) in cells.iter().enumerate() {
                let ry = y + i as f32 * h;
                s.fill_rrect(x0, ry, cw, h, r, if i == sel { ACCENT_BLUE } else { KEY });
                let ox = (x0 + self.pt(14.0)) as i32;
                let img = self.labels.get(ui, cell, MAC_TITLE1_PT, Weight::Regular, WHITE);
                s.blit(img, ox, (ry + h / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
            }
            return;
        }
        s.fill_rrect(x0 - 2.0, y - 2.0, cw * cells.len() as f32 + 4.0, h + 4.0, r, BG);
        for (i, cell) in cells.iter().enumerate() {
            let x = x0 + i as f32 * cw;
            s.fill_rrect(x, y, cw, h, r, if i == sel { ACCENT_BLUE } else { KEY });
            let text = if upper { cell.to_uppercase() } else { cell.clone() };
            let img = self.labels.get(ui, &text, MAC_LARGE_TITLE_PT, Weight::Regular, WHITE);
            s.blit(img, (x + cw / 2.0 - img.w as f32 / 2.0) as i32, (y + h / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
        }
    }

    /// Draws with the keyboard's resting top-left at (0, oy), slid by
    /// `offset(t)`. Pressed keys swap shade (letters darken, specials
    /// lighten) as on the bifrost, which shows no popup.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, oy: f32, t: f64) {
        if self.hardware_input { return; }
        self.draw_inner(s, ui, oy, t);
        let oy = oy + self.offset(t);
        if self.mode == Mode::Floating {
            let (lo, hi) = (self.float_x, self.float_x + self.w * FLOAT_SCALE);
            let (x, y, w, h) = fl::grabber_rect(lo, hi, self.h - self.pt(fl::GRABBER_BAND_PT), self.s);
            s.fill_rrect(x, oy + y, w, h, h.min(self.pt(6.0) * 2.0) / 2.0, fade(LABEL_2.now(), 140));
        }
        if let Some(m) = self.fl_menu {
            let (x, y, w, h) = m.rect();
            let r = self.pt(6.0);
            s.fill_rrect(x, oy + y, w, h, r, BG);
            for (i, (label, mode)) in fl::ITEMS.iter().enumerate() {
                let ry = oy + y + i as f32 * m.row_h;
                if i == m.sel {
                    s.fill_rrect(x + 4.0, ry + 2.0, w - 8.0, m.row_h - 4.0, r * 0.7, ACCENT_BLUE);
                }
                let text = if *mode == self.mode { format!("{label}  \u{2713}") } else { label.to_string() };
                let ox = (x + self.pt(16.0)) as i32;
                let img = self.labels.get(ui, &text, MAC_TITLE2_PT, Weight::Regular, WHITE);
                s.blit(img, ox, (ry + m.row_h / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
            }
        }
    }

    fn draw_inner(&mut self, s: &mut Surface, ui: &Ui, oy: f32, t: f64) {
        let oy = oy + self.offset(t);
        s.fill_rect(0, oy.max(0.0) as usize, self.w as usize, self.h.ceil() as usize, BG);
        let r = self.pt(5.0).min(6.0);
        let pressed = self.pressed.map(|p| p.i);
        let pull = self.pressed.map_or(0.0, |p| p.pull);
        for i in 0..self.keys.len() {
            let k = self.keys[i].clone();
            if self.draw_extra(s, ui, &k, oy, pressed == Some(i), r) {
                continue;
            }
            let plain = matches!(k.kind, Kind::Char(_) | Kind::Space);
            let active = (k.kind == Kind::Shift && (self.shift || self.caps)) || (k.kind == Kind::Ctrl && self.ctrl);
            let down = pressed == Some(i);
            let mut color = if active { WHITE } else if plain != down { KEY } else { SPECIAL };
            if k.kind == Kind::Enter {
                color = kb_pressed::return_fill(self.return_kind, down).unwrap_or(color);
            }
            let ink = if active { BLACK } else { WHITE };
            let (y, cx) = (oy + k.y, k.x + k.w / 2.0);
            kb_pressed::key_shadow(s, k.x, y, k.w, k.h, r, self.ky);
            s.fill_rrect(k.x, y, k.w, k.h, r, color);
            let dim = fade(WHITE, 120);
            // pulling a key down slides its alternate into the middle while
            // the main label drops out below, as the bifrost's flick keys do
            let pull = if down && k.alt.is_some() { ease_out_cubic(pull) } else { 0.0 };
            let main_mid = y + k.h * if k.alt.is_some() { 0.58 } else { 0.5 };
            let alt_top = y + self.pt(3.0);
            if let Some(alt) = k.alt {
                let small = self.labels.get(ui, alt, MAC_SUBHEAD_PT, Weight::Regular, dim);
                let (sw, sh) = (small.w as f32, small.h as f32);
                let top = alt_top + sh / 2.0;
                let cy = top + (main_mid - top) * pull;
                let fade = ((1.0 - pull * 2.0).max(0.0) * 255.0) as u32;
                if fade > 0 {
                    s.blit(small, (cx - sw / 2.0) as i32, (cy - sh / 2.0) as i32, 255, fade);
                }
                if pull > 0.0 {
                    let big = self.labels.get(ui, alt, 24.0 * self.ky, Weight::Regular, ink);
                    let (bw, bh) = (big.w as f32, big.h as f32);
                    s.blit(big, (cx - bw / 2.0) as i32, (cy - bh / 2.0) as i32, 255, (pull * 255.0) as u32);
                }
            }
            let (text, size) = match &k.kind {
                Kind::Char(_) => (self.name(&k.kind), 24.0 * self.ky),
                Kind::Backspace => (self.name(&k.kind), 22.0 * self.ky),
                Kind::Shift => {
                    let st = springboard::shell::kb_capslock::ShiftState::of(self.shift, self.caps);
                    springboard::shell::kb_capslock::shift_glyph(s, (cx, y + k.h / 2.0), self.pt(19.0), self.pt(1.5), st, ink);
                    continue;
                }
                Kind::Enter if self.return_kind.blue() => (self.return_kind.label().into(), 17.0 * self.ky),
                Kind::Enter => ("⏎".into(), 22.0 * self.ky),
                Kind::Named("Left" | "Right" | "Up" | "Down") => (self.name(&k.kind), 20.0 * self.ky),
                Kind::Globe => {
                    glyphs::globe(s, (cx, y + k.h / 2.0), self.pt(11.0), self.pt(1.4), ink);
                    continue;
                }
                Kind::Mic => {
                    glyphs::mic(s, (cx, y + k.h / 2.0), self.pt(26.0), self.pt(1.5), ink);
                    continue;
                }
                Kind::Hide => {
                    glyphs::dismiss_keyboard(s, (cx, y + k.h / 2.0), self.pt(26.0), self.pt(1.5), ink);
                    continue;
                }
                Kind::Space => continue,
                _ => (self.name(&k.kind), 17.0 * self.ky),
            };
            let img = self.labels.get(ui, &text, size, Weight::Regular, ink);
            // keys with an alternate carry their main label a little lower
            let mid = main_mid + k.h * 0.35 * pull;
            let alpha = ((1.0 - pull) * 255.0) as u32;
            if alpha > 0 {
                s.blit(img, (cx - img.w as f32 / 2.0) as i32, (mid - img.h as f32 / 2.0) as i32, 255, alpha);
            }
        }
        let veil = self.trackpad.amount(t);
        if veil > 0.0 {
            s.fill_rrect(0.0, oy, self.w, self.h, 0.0, fade(BG, (veil * 235.0) as u8));
        }
        self.draw_popup(s, ui, oy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> Keyboard {
        Keyboard::new(1200.0, 1200.0 / 768.0, false)
    }

    fn key(k: &Keyboard, kind: &Kind) -> Key {
        k.keys.iter().find(|key| &key.kind == kind).unwrap().clone()
    }

    fn tap(k: &mut Keyboard, kind: &Kind, t: f64) -> Option<Out> {
        let key = key(k, kind);
        let (x, y) = (key.x + key.w / 2.0, key.y + key.h / 2.0);
        k.down(x, y, t);
        k.up(x, y, t)
    }

    #[test]
    fn layout_matches_the_bifrost_in_points() {
        let k = kb();
        let s = k.s;
        let q = key(&k, &Kind::Char("q"));
        assert_eq!((q.x / s).round(), 6.0);
        assert_eq!((q.w / s).round(), 49.0);
        assert_eq!((q.h / s).round(), 55.0);
        let w = key(&k, &Kind::Char("w"));
        assert_eq!(((w.x - q.x) / s).round(), 61.0, "key pitch");
        let a = key(&k, &Kind::Char("a"));
        assert_eq!((a.x / s).round(), 23.0, "second row inset");
        assert_eq!(((a.y - q.y) / s).round(), 66.0, "row pitch");
        let back = key(&k, &Kind::Backspace);
        assert_eq!((back.w / s).round(), 85.0);
        assert!(((back.x + back.w) - (k.w - 6.0 * s)).abs() < 1.0, "right margin");
        assert_eq!((key(&k, &Kind::Enter).w / s).round(), 68.0);
        let z = key(&k, &Kind::Char("z"));
        assert_eq!((z.x / s).round(), 67.0, "after the left shift");
        let space = key(&k, &Kind::Space);
        assert_eq!((space.x / s).round(), 207.0, "after .?123, globe and the mic key as on the bifrost");
        assert_eq!((space.w / s).round(), 351.0, "the mic key takes its share from space");
        for key in &k.keys {
            assert!(key.x >= 0.0 && key.x + key.w <= k.w + 0.5, "{key:?}");
        }
        assert_eq!((k.h / s).round(), 8.0 + 3.0 * 66.0 + 55.0 + 6.0);
    }

    #[test]
    fn letters_carry_the_number_page_as_alternates() {
        let k = kb();
        assert_eq!(key(&k, &Kind::Char("q")).alt, Some("1"));
        assert_eq!(key(&k, &Kind::Char("a")).alt, Some("@"));
        assert_eq!(key(&k, &Kind::Char("þ")).alt, Some("´"));
        assert_eq!(key(&k, &Kind::Space).alt, None);
        let mut k = kb();
        tap(&mut k, &Kind::ToNumbers, 0.0);
        assert_eq!(key(&k, &Kind::Char("@")).alt, Some("€"));
        assert_eq!(key(&k, &Kind::Char("1")).alt, None, "same on both pages");
        tap(&mut k, &Kind::ToSymbols, 1.0);
        assert!(k.keys.iter().all(|key| key.alt.is_none()));
    }

    #[test]
    fn typing_letters_and_shift() {
        let mut k = kb();
        assert_eq!(tap(&mut k, &Kind::Char("a"), 0.0), Some(Out::Text("a".into())));
        tap(&mut k, &Kind::Shift, 1000.0);
        assert_eq!(tap(&mut k, &Kind::Char("b"), 1100.0), Some(Out::Text("B".into())));
        assert_eq!(tap(&mut k, &Kind::Char("c"), 1200.0), Some(Out::Text("c".into())), "shift is one-shot");
        tap(&mut k, &Kind::Shift, 2000.0);
        tap(&mut k, &Kind::Shift, 2100.0);
        assert_eq!(tap(&mut k, &Kind::Char("d"), 2200.0), Some(Out::Text("D".into())));
        assert_eq!(tap(&mut k, &Kind::Char("ð"), 2300.0), Some(Out::Text("Ð".into())), "double tap = caps lock");
        tap(&mut k, &Kind::Shift, 3000.0);
        assert_eq!(tap(&mut k, &Kind::Char("e"), 3100.0), Some(Out::Text("e".into())), "caps lock released");
    }

    #[test]
    fn pages_and_special_keys() {
        let mut k = kb();
        assert_eq!(tap(&mut k, &Kind::ToNumbers, 0.0), None);
        assert_eq!(tap(&mut k, &Kind::Char("7"), 1.0), Some(Out::Text("7".into())));
        assert_eq!(tap(&mut k, &Kind::Char("kr"), 1.5), Some(Out::Text("kr".into())));
        tap(&mut k, &Kind::ToSymbols, 2.0);
        assert_eq!(tap(&mut k, &Kind::Char("€"), 3.0), Some(Out::Text("€".into())));
        tap(&mut k, &Kind::ToLetters, 4.0);
        assert_eq!(tap(&mut k, &Kind::Space, 5.0), Some(Out::Text(" ".into())));
        assert_eq!(tap(&mut k, &Kind::Backspace, 6.0), Some(Out::Backspace));
        assert_eq!(tap(&mut k, &Kind::Enter, 7.0), Some(Out::Enter));
        assert_eq!(tap(&mut k, &Kind::Hide, 8.0), Some(Out::Hide));
        assert_eq!(tap(&mut k, &Kind::Globe, 9.0), None);
    }

    #[test]
    fn gaps_resolve_to_nearest_key_and_drag_off_cancels() {
        let mut k = kb();
        let (a, s) = (key(&k, &Kind::Char("a")), key(&k, &Kind::Char("s")));
        let gap_x = a.x + a.w + 1.0;
        k.down(gap_x, a.y + 5.0, 0.0);
        assert_eq!(k.up(gap_x, a.y + 5.0, 0.0), Some(Out::Text("a".into())));
        k.down(a.x + 5.0, a.y + 5.0, 1.0);
        k.moved(s.x + s.w / 2.0, s.y + 5.0);
        assert_eq!(k.up(s.x + s.w / 2.0, s.y + 5.0, 1.0), Some(Out::Text("s".into())), "sliding sideways picks the key under the finger");
        k.down(a.x + 5.0, a.y + 5.0, 2.0);
        k.moved(-50.0, -50.0);
        assert_eq!(k.up(-50.0, -50.0, 2.0), None, "off the keyboard cancels");
    }

    #[test]
    fn a_downward_flick_types_the_alternate() {
        let mut k = kb();
        let q = key(&k, &Kind::Char("q"));
        let (x, y) = (q.x + q.w / 2.0, q.y + 5.0);
        k.down(x, y, 0.0);
        k.moved(x, y + k.pt(FLICK_PT) + 1.0);
        assert_eq!(k.up(x, y + k.pt(FLICK_PT) + 1.0, 50.0), Some(Out::Text("1".into())));
        k.down(x, y, 100.0);
        k.moved(x, y + k.pt(FLICK_PT) + 1.0);
        k.moved(x, y + 2.0);
        assert_eq!(k.up(x, y + 2.0, 150.0), Some(Out::Text("q".into())), "flicking back cancels the alternate");
    }

    #[test]
    fn a_partial_pull_tracks_progress_and_types_the_letter() {
        let mut k = kb();
        let q = key(&k, &Kind::Char("q"));
        let (x, y) = (q.x + q.w / 2.0, q.y + 5.0);
        k.down(x, y, 0.0);
        k.moved(x, y + k.pt(FLICK_PT) / 2.0);
        let pull = k.pressed.unwrap().pull;
        assert!((pull - 0.5).abs() < 0.01, "pull {pull}");
        assert_eq!(k.up(x, y + k.pt(FLICK_PT) / 2.0, 50.0), Some(Out::Text("q".into())));
    }

    #[test]
    fn holding_backspace_repeats_after_a_delay() {
        let mut k = kb();
        let b = key(&k, &Kind::Backspace);
        let (x, y) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        k.down(x, y, 1000.0);
        assert!(k.tick(1399.0).is_empty());
        assert_eq!(k.tick(1400.0), vec![Out::Backspace]);
        assert!(k.tick(1430.0).is_empty());
        assert_eq!(k.tick(1460.0), vec![Out::Backspace]);
        assert_eq!(k.up(x, y, 1470.0), None, "release after repeating types nothing more");
        let a = key(&k, &Kind::Char("a"));
        k.down(a.x + 5.0, a.y + 5.0, 2000.0);
        assert!(k.tick(5000.0).is_empty(), "letters never repeat");
        assert_eq!(k.up(a.x + 5.0, a.y + 5.0, 5000.0), Some(Out::Text("a".into())));
    }

    #[test]
    fn accessory_ctrl_makes_control_codes_and_arrows_repeat() {
        let mut k = Keyboard::new(1200.0, 1200.0 / 768.0, true);
        assert_eq!(tap(&mut k, &Kind::Named("Up"), 0.0), Some(Out::Named("Up")));
        tap(&mut k, &Kind::Ctrl, 1.0);
        assert_eq!(tap(&mut k, &Kind::Char("c"), 2.0), Some(Out::Text("\u{3}".into())));
        assert_eq!(tap(&mut k, &Kind::Char("c"), 3.0), Some(Out::Text("c".into())));
        tap(&mut k, &Kind::Ctrl, 4.0);
        assert_eq!(tap(&mut k, &Kind::Char("a"), 5.0), Some(Out::Text("\u{1}".into())), "Ctrl+A line start");
        tap(&mut k, &Kind::Ctrl, 6.0);
        assert_eq!(tap(&mut k, &Kind::Char("e"), 7.0), Some(Out::Text("\u{5}".into())), "Ctrl+E line end");
        tap(&mut k, &Kind::Ctrl, 8.0);
        assert_eq!(tap(&mut k, &Kind::Space, 9.0), Some(Out::Text("\0".into())), "Ctrl+Space is NUL");
        tap(&mut k, &Kind::Ctrl, 10.0);
        tap(&mut k, &Kind::ToNumbers, 10.5);
        assert_eq!(tap(&mut k, &Kind::Char("1"), 11.0), None, "Ctrl+1 is dropped, consuming Ctrl");
        tap(&mut k, &Kind::ToLetters, 11.5);
        assert_eq!(tap(&mut k, &Kind::Char("c"), 12.0), Some(Out::Text("c".into())), "Ctrl was consumed");
        let l = key(&k, &Kind::Named("Left"));
        k.down(l.x + 2.0, l.y + 2.0, 100.0);
        assert_eq!(k.tick(500.0), vec![Out::Named("Left")]);
        let esc = key(&k, &Kind::Named("Escape"));
        assert!(esc.y < key(&k, &Kind::Char("q")).y, "accessory row sits above the letters");
        assert!((esc.w / k.s - 65.0).abs() < 1.0);
        k.up(l.x + 2.0, l.y + 2.0, 550.0);
        assert_eq!(tap(&mut k, &Kind::Named("More"), 600.0), Some(Out::Named("More")));
    }

    #[test]
    fn slides_over_a_quarter_second() {
        let mut k = kb();
        assert_eq!(k.offset(0.0), 0.0);
        k.set_shown(false, 1000.0);
        assert!(k.animating() && !k.shown());
        assert_eq!(k.offset(1000.0), 0.0);
        let mid = k.offset(1100.0);
        assert!(mid > 0.0 && mid < k.h);
        assert!(k.offset(1200.0) > mid, "ease-out keeps moving");
        assert_eq!(k.offset(1250.0), k.h);
        k.tick(1250.0);
        assert!(!k.animating());
        assert_eq!(k.offset(5000.0), k.h);
        k.set_shown(true, 6000.0);
        assert!(k.offset(6010.0) < k.h && k.offset(6010.0) > 0.0);
        k.tick(6300.0);
        assert_eq!(k.offset(6300.0), 0.0);
        k.set_shown(true, 7000.0);
        assert!(!k.animating(), "no animation when already shown");
        let hidden = Keyboard::new_hidden(1200.0, 1200.0 / 768.0);
        assert!(!hidden.shown() && !hidden.animating());
        assert_eq!(hidden.offset(0.0), hidden.h);
    }

    #[test]
    fn a_landscape_keyboard_stretches_to_the_width_and_grows_a_third_taller() {
        let s = 1200.0 / 768.0;
        let p = kb();
        let mut k = Keyboard::new(1920.0, s, false);
        let (q, pq) = (key(&k, &Kind::Char("q")), key(&p, &Kind::Char("q")));
        let stretch = 1920.0 / 1200.0;
        assert!((q.x - pq.x * stretch).abs() < 0.01 && (q.w - pq.w * stretch).abs() < 0.01, "keys stretch with the width");
        assert!((q.h / pq.h - 4.0 / 3.0).abs() < 0.01, "keys grow a third taller, like the bifrost");
        assert!((k.h / p.h - 4.0 / 3.0).abs() < 0.02);
        let back = key(&k, &Kind::Backspace);
        assert!(((back.x + back.w) - (1920.0 - 6.0 * s * stretch)).abs() < 1.0, "right margin stretches too");
        for key in &k.keys {
            assert!(key.x >= 0.0 && key.x + key.w <= 1920.5, "{key:?}");
        }
        assert_eq!(k.hit(q.x + 1.0, q.y + 1.0), k.keys.iter().position(|x| x.kind == Kind::Char("q")));
        // turning back restores the portrait geometry and keeps the page
        tap(&mut k, &Kind::ToNumbers, 0.0);
        k.resize(1200.0);
        assert_eq!(key(&k, &Kind::Char("7")).w, key(&p, &Kind::Char("q")).w);
        assert_eq!(k.h, p.h);
        assert_eq!(k.page, Page::Numbers);
        assert!(k.pressed.is_none());
    }

    #[test]
    fn draws_every_page_without_panicking() {
        let ui = crate::testutil::ui();
        let mut k = Keyboard::new(1200.0, ui.s, true);
        let mut buf = vec![0u32; 1200 * 600];
        for page in [Kind::ToNumbers, Kind::ToSymbols, Kind::ToLetters] {
            let mut s = Surface { w: 1200, h: 600, stride: 1200, px: &mut buf, y0: 0 };
            k.shift = page == Kind::ToLetters;
            k.draw(&mut s, &ui, 0.0, 0.0);
            tap(&mut k, &page, 0.0);
        }
        assert!(buf.contains(&KEY) && buf.contains(&SPECIAL));
    }

    fn press(k: &mut Keyboard, c: &str, t: f64) -> Option<Out> {
        let key = k.keys.iter().find(|key| matches!(key.kind, Kind::Char(x) if x == c)).unwrap();
        let (x, y) = (key.x + key.w / 2.0, key.y + key.h / 2.0);
        k.down(x, y, t);
        k.up(x, y, t)
    }

    /// The text a field would hold after the keys a keyboard sends.
    #[derive(Default)]
    struct Field {
        buf: String,
    }

    impl Field {
        fn apply(&mut self, out: Out) {
            match out {
                Out::Text(s) => self.buf.push_str(&s),
                Out::Backspace => {
                    self.buf.pop();
                }
                _ => {}
            }
        }

        fn feed(&mut self, k: &mut Keyboard, out: Option<Out>, t: f64) {
            out.into_iter().for_each(|o| self.apply(o));
            k.tick(t).into_iter().for_each(|o| self.apply(o));
        }

        fn word(&mut self, k: &mut Keyboard, w: &str, t: f64) {
            for c in w.chars() {
                let out = press(k, &c.to_string(), t);
                self.feed(k, out, t);
            }
        }

        fn key(&mut self, k: &mut Keyboard, kind: Kind, t: f64) {
            let out = tap(k, &kind, t);
            self.feed(k, out, t);
        }
    }

    fn smart() -> Keyboard {
        let mut k = kb();
        k.set_predictive(true);
        k
    }

    #[test]
    fn autocorrect_fixes_on_space_and_backspace_undoes_it() {
        let (mut k, mut f) = (smart(), Field::default());
        f.word(&mut k, "becuase", 0.0);
        f.key(&mut k, Kind::Space, 100.0);
        assert_eq!(f.buf, "because ");
        f.key(&mut k, Kind::Backspace, 200.0);
        assert_eq!(f.buf, "becuase", "backspace restores what was typed");
        f.key(&mut k, Kind::Space, 300.0);
        assert_eq!(f.buf, "becuase ", "an undone correction is not repeated");
    }

    #[test]
    fn a_caret_move_ends_the_word_and_the_pending_correction() {
        let (mut k, mut f) = (smart(), Field::default());
        f.word(&mut k, "becuase", 0.0);
        f.key(&mut k, Kind::Space, 100.0);
        k.caret_moved();
        f.key(&mut k, Kind::Backspace, 200.0);
        assert_eq!(f.buf, "because", "backspace after a caret move is a plain delete");
        assert!(k.word.is_empty() && k.last.is_none() && k.undo_correction.is_none());
        f.word(&mut k, "ab", 300.0);
        k.caret_moved();
        assert!(k.word.is_empty(), "the word under the old caret is dropped");
    }

    #[test]
    fn autocorrect_leaves_words_acronyms_and_plain_keyboards_alone() {
        let (mut k, mut f) = (smart(), Field::default());
        f.word(&mut k, "hello", 0.0);
        f.key(&mut k, Kind::Space, 10.0);
        assert_eq!(f.buf, "hello ");
        let (mut k, mut f) = (kb(), Field::default());
        f.word(&mut k, "becuase", 0.0);
        f.key(&mut k, Kind::Space, 10.0);
        f.key(&mut k, Kind::Space, 20.0);
        assert_eq!(f.buf, "becuase  ", "smart typing is off by default");
        let (mut k, mut f) = (smart(), Field::default());
        f.word(&mut k, "i", 0.0);
        f.key(&mut k, Kind::Space, 10.0);
        assert_eq!(f.buf, "I ", "a lone i becomes I");
    }

    #[test]
    fn double_space_types_a_period_and_capitalises_the_next_word() {
        let (mut k, mut f) = (smart(), Field::default());
        k.set_auto_caps(true);
        f.word(&mut k, "hello", 0.0);
        f.key(&mut k, Kind::Space, 100.0);
        f.key(&mut k, Kind::Space, 200.0);
        assert_eq!(f.buf, "Hello. ");
        f.word(&mut k, "w", 300.0);
        assert_eq!(f.buf, "Hello. W", "auto-capitalised after the period");
        f.key(&mut k, Kind::Space, 400.0);
        f.key(&mut k, Kind::Space, 2000.0);
        assert!(f.buf.ends_with("W  ") || f.buf.ends_with("W "), "a slow second space is just a space");
        assert!(!f.buf.ends_with(". "), "{:?}", f.buf);
    }

    #[test]
    fn shift_is_one_shot_double_tap_locks_and_sentences_restart_it() {
        let mut k = kb();
        k.set_auto_caps(true);
        assert!(k.shift, "shift is on at the start of a sentence");
        assert_eq!(press(&mut k, "a", 0.0), Some(Out::Text("A".into())));
        assert!(!k.shift && !k.caps);
        press(&mut k, ".", 10.0);
        assert!(!k.shift);
        tap(&mut k, &Kind::Space, 20.0);
        assert!(k.shift, "a space after a period capitalises");
        tap(&mut k, &Kind::Shift, 1000.0);
        assert!(!k.shift, "tapping shift cancels the auto shift");
        tap(&mut k, &Kind::Shift, 2000.0);
        tap(&mut k, &Kind::Shift, 2100.0);
        assert!(k.caps);
        press(&mut k, "b", 2200.0);
        assert!(k.caps, "caps lock stays on");
        tap(&mut k, &Kind::Shift, 3000.0);
        assert!(!k.caps && !k.shift);
    }

    #[test]
    fn the_bar_suggests_completions_and_a_tap_replaces_the_word() {
        let (mut k, mut f) = (smart(), Field::default());
        let q = key(&k, &Kind::Char("q"));
        assert!(key(&k, &Kind::Suggest(0)).y < q.y, "the bar sits above the keys");
        f.word(&mut k, "th", 0.0);
        assert!(!k.suggestions.is_empty() && k.suggestions.iter().all(|s| s.starts_with("th")), "{:?}", k.suggestions);
        let first = k.suggestions[0].clone();
        f.key(&mut k, Kind::Suggest(0), 100.0);
        assert_eq!(f.buf, format!("{first} "));
        assert!(k.suggestions.len() <= 3);
    }

    #[test]
    fn holding_a_letter_slides_out_accents_to_pick_from() {
        let mut k = kb();
        let a = key(&k, &Kind::Char("a"));
        let (x, y) = (a.x + a.w / 2.0, a.y + a.h / 2.0);
        k.down(x, y, 0.0);
        assert!(k.animating(), "a held letter needs frames to notice the long press");
        k.tick(LONG_PRESS_MS - 1.0);
        assert!(k.popup.is_none());
        k.tick(LONG_PRESS_MS + 1.0);
        let pop = k.popup.as_ref().expect("accent strip");
        assert_eq!(pop.cells[0], "a");
        assert_eq!(pop.cells[1], "á");
        let at = pop.x0 + pop.cell_w * 1.5;
        k.moved(at, y);
        assert_eq!(k.up(at, y, 600.0), Some(Out::Text("á".into())));
        assert!(k.popup.is_none());
        // releasing in place types the letter itself
        k.down(x, y, 1000.0);
        k.tick(1500.0);
        assert_eq!(k.up(x, y, 1600.0), Some(Out::Text("a".into())));
        // a key near the right edge opens its strip leftwards, letter last
        let o = key(&k, &Kind::Char("o"));
        let (ox, oy) = (o.x + o.w / 2.0, o.y + o.h / 2.0);
        k.down(ox, oy, 2000.0);
        k.tick(2500.0);
        let pop = k.popup.as_ref().expect("strip");
        assert_eq!(pop.cells.last().unwrap(), "o");
        assert!(pop.x0 + pop.cell_w * pop.cells.len() as f32 <= k.w);
        let n = pop.cells.len();
        let at = pop.x0 + pop.cell_w * (n as f32 - 1.5);
        k.moved(at, oy);
        assert_eq!(k.up(at, oy, 2600.0), Some(Out::Text("ó".into())));
        // shift applies to accents; a tap on a letter without any needs no strip
        let q = key(&k, &Kind::Char("q"));
        k.down(q.x + 5.0, q.y + 5.0, 3000.0);
        k.tick(4000.0);
        assert!(k.popup.is_none());
    }

    #[test]
    fn the_emoji_page_types_emoji_and_remembers_recents() {
        springboard::emoji::FORCE.with(|f| f.set(Some(true)));
        let (mut k, mut f) = (kb(), Field::default());
        f.key(&mut k, Kind::Globe, 0.0);
        assert_eq!(k.page, Page::Emoji);
        f.key(&mut k, Kind::EmojiKey(0), 10.0);
        assert_eq!(f.buf, "😀");
        f.key(&mut k, Kind::EmojiTab(0), 20.0);
        assert_eq!(k.emoji_page, ["😀"], "recents lead");
        f.key(&mut k, Kind::EmojiKey(0), 30.0);
        assert_eq!(f.buf, "😀😀");
        f.key(&mut k, Kind::EmojiKey(5), 40.0);
        assert_eq!(f.buf, "😀😀", "empty cells type nothing");
        f.key(&mut k, Kind::ToLetters, 50.0);
        assert_eq!(k.page, Page::Letters);
        assert!(k.keys.iter().any(|key| key.kind == Kind::Space));
        for key in &k.keys {
            assert!(key.x >= 0.0 && key.x + key.w <= k.w + 0.5, "{key:?}");
        }
    }

    #[test]
    fn the_globe_skips_the_emoji_page_without_an_emoji_font() {
        springboard::emoji::FORCE.with(|f| f.set(Some(false)));
        let mut k = kb();
        for i in 0..6 {
            tap(&mut k, &Kind::Globe, i as f64);
            assert_ne!(k.page, Page::Emoji);
        }
        assert!(!springboard::shell::kb_layouts::menu().contains(&"Emoji".to_string()));
        springboard::emoji::FORCE.with(|f| f.set(None));
    }

    #[test]
    fn the_emoji_page_fits_the_screen() {
        springboard::emoji::FORCE.with(|f| f.set(Some(true)));
        let mut k = kb();
        let h = k.h;
        tap(&mut k, &Kind::Globe, 0.0);
        assert!((k.h - h - k.py(ROW_PT)).abs() <= 1.0, "letters height plus the search field row");
        for key in &k.keys {
            assert!(key.x >= 0.0 && key.x + key.w <= k.w + 0.5 && key.y + key.h <= k.h, "{key:?}");
        }
        assert_eq!(k.keys.iter().filter(|key| matches!(key.kind, Kind::EmojiTab(_))).count(), emoji::TABS);
    }

    #[test]
    fn dragging_the_dock_handle_floats_splits_and_docks() {
        let mut k = kb();
        let q0 = key(&k, &Kind::Char("q"));
        let hide = key(&k, &Kind::Hide);
        let (x, y) = (hide.x + hide.w / 2.0, hide.y + hide.h / 2.0);
        // sideways: split
        k.down(x, y, 0.0);
        k.moved(x - k.pt(DOCK_SIDE_PT) - 1.0, y);
        assert_eq!(k.mode(), Mode::Split);
        assert_eq!(k.up(x, y, 10.0), None, "a drag types nothing");
        let mut row: Vec<Key> = k.keys.iter().filter(|key| (key.y - q0.y).abs() < 1.0 && matches!(key.kind, Kind::Char(_))).cloned().collect();
        row.sort_by(|a, b| a.x.total_cmp(&b.x));
        let gap = row.windows(2).map(|w| w[1].x - (w[0].x + w[0].w)).fold(0.0, f32::max);
        assert!(gap > k.pt(60.0), "split leaves a gap in the middle: {gap}");
        for key in &k.keys {
            assert!(key.x >= 0.0 && key.x + key.w <= k.w + 0.5, "{key:?}");
        }
        // up: floating
        let hide = key(&k, &Kind::Hide);
        let (x, y) = (hide.x + hide.w / 2.0, hide.y + hide.h / 2.0);
        k.down(x, y, 100.0);
        k.moved(x, y - k.pt(DOCK_DRAG_PT) - 1.0);
        assert_eq!(k.mode(), Mode::Floating);
        k.up(x, y, 110.0);
        let q = key(&k, &Kind::Char("q"));
        assert!(k.keys.iter().all(|key| key.x >= k.float_x - 0.5 && key.x + key.w <= k.float_x + k.w * FLOAT_SCALE + 0.5));
        assert!(q.w < q0.w);
        // a tap beside the floating keyboard is not a key
        k.down(1.0, q.y + 5.0, 200.0);
        assert_eq!(k.up(1.0, q.y + 5.0, 210.0), None);
        // dragging the handle moves it sideways
        let hide = key(&k, &Kind::Hide);
        let (x, y) = (hide.x + hide.w / 2.0, hide.y + hide.h / 2.0);
        let before = k.float_x;
        k.down(x, y, 300.0);
        k.moved(x + k.pt(20.0), y);
        k.moved(x + k.pt(60.0), y);
        assert!(k.float_x > before);
        k.up(x + k.pt(60.0), y, 310.0);
        // down: docked again, exactly as before
        let hide = key(&k, &Kind::Hide);
        let (x, y) = (hide.x + hide.w / 2.0, hide.y + hide.h / 2.0);
        k.down(x, y, 400.0);
        k.moved(x, y + k.pt(DOCK_DRAG_PT) + 1.0);
        assert_eq!(k.mode(), Mode::Docked);
        k.up(x, y, 410.0);
        assert_eq!(key(&k, &Kind::Char("q")), q0);
    }

    #[test]
    fn a_plain_tap_on_the_handle_still_hides() {
        let mut k = kb();
        assert_eq!(tap(&mut k, &Kind::Hide, 0.0), Some(Out::Hide));
        assert_eq!(k.mode(), Mode::Docked);
    }
}

#[cfg(test)]
mod hardware_input_tests {
    use super::*;
    #[test]
    fn hardware_focus_has_no_keyboard_geometry_or_animation() {
        let mut keyboard = Keyboard::new_hidden(1000., 1.);
        keyboard.hardware_input = true;
        keyboard.layout();
        keyboard.set_shown(true, 10.);
        assert!(keyboard.shown());
        assert_eq!(keyboard.h, 0.);
        assert!(!keyboard.animating());
        keyboard.resize(900.);
        assert_eq!(keyboard.h, 0.);
        keyboard.set_shown(false, 20.);
        assert!(!keyboard.shown());
        assert!(!keyboard.animating());
    }
}
