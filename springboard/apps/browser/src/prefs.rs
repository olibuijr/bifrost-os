//! Browser settings (Browser: Settings, Cmd+,): homepage, default search engine, restore-on-launch, and the
//! clear-history / clear-website-data actions. The values live in the app's UserDefaults (`Ctx::defaults`); the sheet is
//! drawn over the window like the script dialogs, because SDK apps have a single window.
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::{AlertCard, MAC_FOOTNOTE_PT, Weight, CARD_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_SUBHEAD_PT, MAC_TITLE2_PT, SEGMENT_ON, SEPARATOR_C, TINT};
use crate::tokens;
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::webview::url_for_input;
use bifrost_app::{Ctx, KeyInput};

const PREF_HOME: &str = "browser.homepage";
const PREF_ENGINE: &str = "browser.searchEngine";
const PREF_RESTORE: &str = "browser.restoreSession";
/// The SDK's built-in search URL prefix (DuckDuckGo); other engines replace it.
const DDG: &str = "https://duckduckgo.com/?q=";
const MAX_HOME: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    DuckDuckGo,
    Google,
    Bing,
    Startpage,
}

impl Engine {
    pub const ALL: [Engine; 4] = [Engine::DuckDuckGo, Engine::Google, Engine::Bing, Engine::Startpage];

    pub fn name(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Google => "Google",
            Engine::Bing => "Bing",
            Engine::Startpage => "Startpage",
        }
    }

    pub fn parse(s: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.name().eq_ignore_ascii_case(s.trim()))
    }

    /// The URL up to and including `q=`.
    pub fn prefix(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => DDG,
            Engine::Google => "https://www.google.com/search?q=",
            Engine::Bing => "https://www.bing.com/search?q=",
            Engine::Startpage => "https://www.startpage.com/do/search?q=",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// Empty means a blank start page.
    pub homepage: String,
    pub engine: Engine,
    pub restore: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { homepage: String::new(), engine: Engine::DuckDuckGo, restore: true }
    }
}

impl Prefs {
    pub fn load(ctx: &Ctx) -> Prefs {
        let d = ctx.defaults();
        let mut p = Prefs::default();
        if let Some(h) = d.string(PREF_HOME) {
            p.homepage = h.chars().take(MAX_HOME).collect();
        }
        if let Some(e) = d.string(PREF_ENGINE).and_then(|s| Engine::parse(&s)) {
            p.engine = e;
        }
        if d.contains(PREF_RESTORE) {
            p.restore = d.bool(PREF_RESTORE);
        }
        p
    }

    pub fn save(&self, ctx: &Ctx) {
        let mut d = ctx.defaults_mut();
        d.set_string(PREF_HOME, &self.homepage);
        d.set_string(PREF_ENGINE, self.engine.name());
        d.set_bool(PREF_RESTORE, self.restore);
    }

    /// What a new window opens: the homepage, or `blank`.
    pub fn start_url(&self, blank: &str) -> String {
        let h = self.homepage.trim();
        if h.is_empty() {
            blank.to_string()
        } else {
            url_for_input(h)
        }
    }

    /// Address-field text to URL, searching with the chosen engine (the SDK searches with DuckDuckGo).
    pub fn url_for_input(&self, text: &str) -> String {
        let url = url_for_input(text);
        if self.engine != Engine::DuckDuckGo && !text.contains("://") {
            if let Some(q) = url.strip_prefix(DDG) {
                return format!("{}{q}", self.engine.prefix());
            }
        }
        url
    }
}

/// What the browser should do after the sheet handled input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// A value changed: save the prefs.
    Changed,
    ClearHistory,
    ClearData,
    /// Settings > Import Bookmarks: the newest bookmark export in Downloads.
    ImportBookmarks,
    Close,
}

#[derive(Default)]
pub struct Sheet {
    pub open: bool,
    home: TextField,
    focus_home: bool,
    status: String,
    /// A destructive action waiting for the confirmation alert.
    confirm: Option<(Action, AlertCard)>,
    /// Vertical scroll in pt when the window is shorter than the panel, and the drag that drives it.
    scroll: f32,
    drag: Option<(f32, f32, bool)>,
}

/// Panel width, padding and row height in pt. The panel hangs under the toolbar so the window chrome stays visible.
const W: f32 = 460.0;
const PAD: f32 = 16.0;
const ROW: f32 = 40.0;
const ROW_PAD: f32 = 12.0;
const LABEL_W: f32 = 110.0;
const FOOT_H: f32 = 18.0;
const GROUP_GAP: f32 = 8.0;
const TITLE_H: f32 = 22.0;
const FOOTER_HOME: &str = "New windows open the homepage. Private windows never restore tabs.";
const FOOTER_CLEAR: &str = "Clearing history or website data cannot be undone.";

struct Geo {
    panel: Rect,
    title_y: f32,
    group1: Rect,
    group2: Rect,
    home: Rect,
    segs: [Rect; 4],
    restore_row: Rect,
    toggle: Rect,
    import_row: Rect,
    clear_history: Rect,
    clear_data: Rect,
    done: Rect,
    foot1_y: f32,
    foot2_y: f32,
    /// Panel content height in pt and the largest scroll.
    full_h: f32,
    max_scroll: f32,
}

impl Sheet {
    pub fn show(&mut self, p: &Prefs) {
        self.open = true;
        self.home.clear();
        self.home.push_str(&p.homepage);
        self.focus_home = false;
        self.status.clear();
        self.confirm = None;
        self.scroll = 0.0;
        self.drag = None;
    }

    pub fn set_status(&mut self, s: &str) {
        self.status = s.to_string();
    }

    fn geo(ui: &Ui, w: u32, h: u32, scroll: f32) -> Geo {
        let wpt = w as f32 / ui.pt(1.0);
        let pw = W.min(wpt - 16.0).max(0.0);
        let hpt = h as f32 / ui.pt(1.0);
        let full_h = PAD + TITLE_H + PAD + 4.0 * ROW + FOOT_H + GROUP_GAP + 2.0 * ROW + FOOT_H + PAD;
        let top = UNIFIED_BAR_PT + 6.0;
        // a short window shrinks the panel to fit and the content scrolls inside it
        let ph = full_h.min((hpt - top - 8.0).max(0.0));
        let max_scroll = (full_h - ph).max(0.0);
        let scroll = scroll.clamp(0.0, max_scroll);
        let panel = Rect { x: ((wpt - pw) / 2.0).max(0.0), y: top, w: pw, h: ph };
        let gx = panel.x + PAD;
        let gw = panel.w - 2.0 * PAD;
        let title_y = panel.y + PAD - scroll;
        let g1y = title_y + TITLE_H + PAD;
        let group1 = Rect { x: gx, y: g1y, w: gw, h: 4.0 * ROW };
        let row = |g: &Rect, i: f32| Rect { x: g.x, y: g.y + ROW * i, w: g.w, h: ROW };
        let r0 = row(&group1, 0.0);
        let fx = r0.x + ROW_PAD + LABEL_W.min(gw * 0.3);
        let home = Rect { x: fx, y: r0.y + (ROW - 28.0) / 2.0, w: (r0.x + r0.w - ROW_PAD - fx).max(0.0), h: 28.0 };
        let r1 = row(&group1, 1.0);
        let sw = ((r1.x + r1.w - ROW_PAD - fx) / 4.0).max(0.0);
        let segs = [0, 1, 2, 3].map(|i| Rect { x: fx + sw * i as f32, y: r1.y + (ROW - 28.0) / 2.0, w: sw, h: 28.0 });
        let restore_row = row(&group1, 2.0);
        let toggle = Rect { x: restore_row.x + restore_row.w - ROW_PAD - 40.0, y: restore_row.y + (ROW - 24.0) / 2.0, w: 40.0, h: 24.0 };
        let import_row = row(&group1, 3.0);
        let foot1_y = group1.y + group1.h + 3.0;
        let group2 = Rect { x: gx, y: group1.y + group1.h + FOOT_H + GROUP_GAP, w: gw, h: 2.0 * ROW };
        let clear_history = row(&group2, 0.0);
        let clear_data = row(&group2, 1.0);
        let foot2_y = group2.y + group2.h + 3.0;
        let done = Rect { x: panel.x + panel.w - PAD - 56.0, y: title_y - 3.0, w: 56.0, h: TITLE_H + 6.0 };
        Geo { panel, title_y, group1, group2, home, segs, restore_row, toggle, import_row, clear_history, clear_data, done, foot1_y, foot2_y, full_h, max_scroll }
    }

    fn ask(&mut self, ui: &Ui, action: Action, title: &str, message: &str, confirm: &str) {
        let buttons = vec![ui.text("Cancel", MAC_BODY_PT, Weight::Regular, TINT), ui.text(confirm, MAC_BODY_PT, Weight::Semibold, DESTRUCTIVE)];
        self.confirm = Some((action, AlertCard::new(ui, title, message, buttons)));
    }

    /// Finger down or move while the sheet is up: a vertical drag scrolls a panel that does not fit.
    pub fn touch_drag(&mut self, ui: &Ui, w: u32, h: u32, down: bool, x: f32, y: f32) {
        let _ = x;
        let g = Self::geo(ui, w, h, self.scroll);
        self.scroll = self.scroll.clamp(0.0, g.max_scroll);
        let py = y / ui.pt(1.0);
        if down {
            self.drag = Some((py, self.scroll, false));
        } else if let Some((y0, s0, _)) = self.drag {
            if (py - y0).abs() > 4.0 && g.max_scroll > 0.0 {
                self.scroll = (s0 - (py - y0)).clamp(0.0, g.max_scroll);
                self.drag = Some((y0, s0, true));
            }
        }
    }

    /// A tap at window pixels while the sheet is up; always consumed.
    pub fn touch_up(&mut self, ui: &Ui, w: u32, h: u32, x: f32, y: f32, p: &mut Prefs) -> Action {
        if let Some((action, card)) = self.confirm.take() {
            match card.hit(ui, w as i32, h as i32, x, y) {
                Some(1) => return action,
                Some(_) => {}
                None => self.confirm = Some((action, card)), // only a button answers
            }
            return Action::None;
        }
        if self.drag.take().is_some_and(|d| d.2) {
            return Action::None; // the end of a scroll drag is not a tap
        }
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let g = Self::geo(ui, w, h, self.scroll);
        self.focus_home = false;
        if !g.panel.contains(px, py) || g.done.contains(px, py) {
            return Action::Close;
        }
        if g.home.contains(px, py) {
            self.focus_home = true;
            return Action::None;
        }
        if let Some(i) = g.segs.iter().position(|r| r.contains(px, py)) {
            p.engine = Engine::ALL[i];
            return Action::Changed;
        }
        if g.restore_row.contains(px, py) {
            p.restore = !p.restore;
            return Action::Changed;
        }
        if g.import_row.contains(px, py) {
            return Action::ImportBookmarks;
        }
        if g.clear_history.contains(px, py) {
            self.ask(ui, Action::ClearHistory, "Clear History?", "Every page you visited is removed from this browser. This cannot be undone.", "Clear History");
        } else if g.clear_data.contains(px, py) {
            self.ask(ui, Action::ClearData, "Clear Website Data?", "Cookies, site storage and the cache are deleted and your open pages reload. This cannot be undone.", "Clear Data");
        }
        Action::None
    }

    /// Keys while the sheet is up; always consumed. Escape closes (or cancels the alert); Return ends editing the homepage.
    pub fn key(&mut self, k: &KeyInput, p: &mut Prefs) -> Action {
        fn edit(s: &Sheet, p: &mut Prefs) -> Action {
            p.homepage = s.home.to_string().chars().take(MAX_HOME).collect();
            Action::Changed
        }
        if self.confirm.is_some() {
            if matches!(k, KeyInput::Code(1) | KeyInput::Code(27) | KeyInput::Code(13)) {
                self.confirm = None;
            }
            return Action::None;
        }
        match k {
            KeyInput::Code(1) | KeyInput::Code(27) => Action::Close,
            KeyInput::Code(13) if self.focus_home => {
                self.focus_home = false;
                Action::None
            }
            KeyInput::Code(13) => Action::Close,
            _ if !self.focus_home => Action::None,
            KeyInput::Text(s) => {
                self.home.push_str(s);
                edit(self, p)
            }
            KeyInput::Code(8) => {
                self.home.pop();
                edit(self, p)
            }
            KeyInput::Code(46) => {
                self.home.delete();
                edit(self, p)
            }
            KeyInput::Code(37) => {
                self.home.move_caret(false);
                Action::None
            }
            KeyInput::Code(39) => {
                self.home.move_caret(true);
                Action::None
            }
            _ => Action::None,
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: u32, h: u32, p: &Prefs) {
        if !self.open {
            return;
        }
        let g = Self::geo(ui, w, h, self.scroll);
        // dim only the page: the toolbar stays visible above the sheet
        let top = ui.pt(UNIFIED_BAR_PT);
        s.fill_rrect(0.0, top, s.w as f32, s.h as f32 - top, 0.0, tokens::scrim(110));
        s.fill_rrect(ui.pt(g.panel.x), ui.pt(g.panel.y), ui.pt(g.panel.w), ui.pt(g.panel.h), ui.pt(m::WINDOW_RADIUS), CARD_BG.now());
        // everything below draws into the panel's rows only, so scrolled content never leaves the card
        let (clip_top, clip_bot) = (ui.pt(g.panel.y) as usize, ui.pt(g.panel.y + g.panel.h) as usize);
        let s0 = s;
        let mut sv = s0.rows(clip_top, clip_bot);
        let s = &mut sv;
        let text = |s: &mut Surface, t: &str, pt: f32, wt: Weight, color: u32, x: f32, cy: f32| {
            let img = ui.text(t, pt, wt, color);
            s.blit(&img, ui.pt(x) as i32, ui.pt(cy) as i32 - img.h as i32 / 2, 255, 255);
        };
        let group = |s: &mut Surface, r: &Rect, rows: u32| {
            s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(m::CARD_RADIUS), FIELD_BG.now());
            for i in 1..rows {
                s.fill_rect(ui.pt(r.x + ROW_PAD) as usize, ui.pt(r.y + ROW * i as f32) as usize, ui.pt(r.w - ROW_PAD) as usize, 1, SEPARATOR_C.now());
            }
        };
        text(s, "Settings", MAC_TITLE2_PT, Weight::Semibold, LABEL.now(), g.panel.x + PAD, g.title_y + TITLE_H / 2.0);
        text(s, "Done", MAC_BODY_PT, Weight::Semibold, TINT, g.done.x + 8.0, g.done.y + g.done.h / 2.0);

        group(s, &g.group1, 4);
        let lx = g.group1.x + ROW_PAD;
        text(s, "Homepage", MAC_BODY_PT, Weight::Regular, LABEL.now(), lx, g.home.y + g.home.h / 2.0);
        bifrost_app::widgets::text_field(s, ui, &self.home, g.home, "Blank page", self.focus_home);
        let seg = g.segs[0];
        text(s, "Search engine", MAC_BODY_PT, Weight::Regular, LABEL.now(), lx, seg.y + seg.h / 2.0);
        let track = Rect { x: seg.x, y: seg.y, w: g.segs[3].x + g.segs[3].w - seg.x, h: seg.h };
        s.fill_rrect(ui.pt(track.x), ui.pt(track.y), ui.pt(track.w), ui.pt(track.h), ui.pt(m::SELECTION_RADIUS), CARD_BG.now());
        for (i, r) in g.segs.iter().enumerate() {
            let on = p.engine == Engine::ALL[i];
            if on {
                s.fill_rrect(ui.pt(r.x + 2.0), ui.pt(r.y + 2.0), ui.pt(r.w - 4.0), ui.pt(r.h - 4.0), ui.pt(m::SELECTION_RADIUS), SEGMENT_ON.now());
            }
            let img = ui.text(Engine::ALL[i].name(), MAC_SUBHEAD_PT, if on { Weight::Semibold } else { Weight::Regular }, LABEL.now());
            s.blit(&img, ui.pt(r.x + r.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(r.y + r.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
        }
        text(s, "Restore tabs on launch", MAC_BODY_PT, Weight::Regular, LABEL.now(), lx, g.toggle.y + g.toggle.h / 2.0);
        let t = g.toggle;
        s.fill_rrect(ui.pt(t.x), ui.pt(t.y), ui.pt(t.w), ui.pt(t.h), ui.pt(t.h / 2.0), if p.restore { TINT } else { CARD_BG.now() });
        let knob = t.h - 4.0;
        let kx = if p.restore { t.x + t.w - knob - 2.0 } else { t.x + 2.0 };
        s.fill_rrect(ui.pt(kx), ui.pt(t.y + 2.0), ui.pt(knob), ui.pt(knob), ui.pt(knob / 2.0), tokens::KNOB);
        text(s, "Import Bookmarks\u{2026}", MAC_BODY_PT, Weight::Regular, LINK, lx, g.import_row.y + ROW / 2.0);
        let fx = g.group1.x + ROW_PAD;
        left_footer(s, ui, FOOTER_HOME, fx, g.group1.w - 2.0 * ROW_PAD, g.foot1_y);

        group(s, &g.group2, 2);
        text(s, "Clear History\u{2026}", MAC_BODY_PT, Weight::Regular, DESTRUCTIVE, lx, g.clear_history.y + ROW / 2.0);
        text(s, "Clear Website Data\u{2026}", MAC_BODY_PT, Weight::Regular, DESTRUCTIVE, lx, g.clear_data.y + ROW / 2.0);
        left_footer(s, ui, if self.status.is_empty() { FOOTER_CLEAR } else { &self.status }, fx, g.group1.w - 2.0 * ROW_PAD, g.foot2_y);

        if let Some((_, card)) = &self.confirm {
            card.draw(s0, ui, w as i32, h as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_round_trip_by_name() {
        for e in Engine::ALL {
            assert_eq!(Engine::parse(e.name()), Some(e));
        }
        assert_eq!(Engine::parse("nope"), None);
    }

    #[test]
    fn search_uses_the_chosen_engine_but_urls_are_untouched() {
        let p = Prefs { engine: Engine::Google, ..Prefs::default() };
        assert_eq!(p.url_for_input("rust lang"), "https://www.google.com/search?q=rust+lang");
        assert_eq!(p.url_for_input("example.com"), "https://example.com");
        assert_eq!(p.url_for_input("https://duckduckgo.com/?q=x"), "https://duckduckgo.com/?q=x");
        assert_eq!(Prefs::default().url_for_input("a b"), "https://duckduckgo.com/?q=a+b");
    }

    #[test]
    fn start_url_falls_back_to_blank() {
        assert_eq!(Prefs::default().start_url("about:blank"), "about:blank");
        let p = Prefs { homepage: "example.com".into(), ..Prefs::default() };
        assert_eq!(p.start_url("about:blank"), "https://example.com");
    }
}

/// A footnote line in secondary grey, left-aligned at `x` (pt) per the HIG for grouped-form footers (Settings only;
/// the shared `footer()` stays centred). Text wider than `max_w` is cut with an ellipsis, measured with `ui.measure`.
fn left_footer(s: &mut Surface, ui: &Ui, text: &str, x: f32, max_w: f32, y: f32) {
    let mut t = text.to_string();
    if ui.measure(&t, MAC_FOOTNOTE_PT, Weight::Regular) > ui.pt(max_w) {
        while t.pop().is_some() && ui.measure(&format!("{t}\u{2026}"), MAC_FOOTNOTE_PT, Weight::Regular) > ui.pt(max_w) {}
        t.push('\u{2026}');
    }
    let img = ui.text(&t, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    s.blit(&img, ui.pt(x) as i32, ui.pt(y) as i32, 255, 255);
}
