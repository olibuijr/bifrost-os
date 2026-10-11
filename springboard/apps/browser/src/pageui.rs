//! UI the page asks for: JavaScript alert / confirm / prompt sheets, the right-click context menu and `<select>`
//! popups. Browser shows script dialogs as a sheet titled with the page's host, so web content cannot pass for
//! browser UI; menus use the shared popover menu. Servo reports them as embedder controls (engine `dialog`, `menu`).
use std::collections::VecDeque;

use bifrost_app::controls::menu::{Menu, MenuItem};
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_SUBHEAD_PT, TINT, WHITE};
use bifrost_app::kit::{rgba, Surface, Ui};
use bifrost_app::webview::{DialogKind, MenuKind, WebProcessPool, WebViewEvent, WebViewId};
use bifrost_app::widgets::{self, MENU_PAD_PT, MENU_ROW_PT, MENU_SEPARATOR_PT};
use bifrost_app::KeyInput;

/// Sheet geometry, points.
const SHEET_W: f32 = 280.0;
const PAD: f32 = 16.0;
const LINE: f32 = 17.0;
const BTN_H: f32 = 28.0;
const FIELD_H: f32 = 24.0;
const MAX_LINES: usize = 12;
const MAX_QUEUE: usize = 8;
/// Row id added by the browser itself (the page cannot supply it).
const INSPECT: u32 = u32::MAX;

pub struct Dialog {
    view: WebViewId,
    token: u32,
    kind: DialogKind,
    host: String,
    message: String,
    field: TextField,
}

pub struct PageMenu {
    view: WebViewId,
    token: u32,
    kind: MenuKind,
    /// Top-left in window pixels.
    origin: (f32, f32),
    width: f32,
    menu: Menu,
    /// Menu item id (entry index + 1) to the engine index.
    picks: Vec<usize>,
}

/// What the browser should do after the user answered.
pub enum Outcome {
    None,
    Inspect,
}

#[derive(Default)]
pub struct PageUi {
    dialogs: VecDeque<Dialog>,
    menu: Option<PageMenu>,
    /// Where the last page click landed (window pixels), to place a context menu under the pointer.
    pub last_point: (f32, f32),
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

impl PageUi {
    pub fn active(&self) -> bool {
        !self.dialogs.is_empty() || self.menu.is_some()
    }

    /// True while a script dialog or menu belongs to `view`.
    pub fn forget(&mut self, view: WebViewId) {
        self.dialogs.retain(|d| d.view != view);
        if self.menu.as_ref().is_some_and(|m| m.view == view) {
            self.menu = None;
        }
    }

    /// Takes the events this module owns; returns true when it consumed one.
    pub fn on_event(&mut self, ev: &WebViewEvent, pool: &mut Option<WebProcessPool>, frame: (i32, i32, u32, u32), ui: &Ui) -> bool {
        match ev {
            WebViewEvent::Dialog { id, token, kind, message, default } => {
                if self.dialogs.len() >= MAX_QUEUE {
                    // a page flooding dialogs: refuse the extra ones (Browser offers to suppress them)
                    if let Some(p) = pool {
                        p.dialog_reply(*id, *token, false, "");
                    }
                    return true;
                }
                let host = pool.as_ref().and_then(|p| p.state(*id)).map(|s| host_of(&s.url)).unwrap_or_default();
                let mut field = TextField::new();
                field.push_str(default);
                self.dialogs.push_back(Dialog { view: *id, token: *token, kind: *kind, host, message: message.clone(), field });
                true
            }
            WebViewEvent::Menu { id, token, kind, rect, items } => {
                // a second menu replaces the first (the page opened another)
                self.dismiss_menu(pool);
                let one = ui.pt(1.0).max(0.5);
                let mut menu_items = Vec::new();
                let mut picks = Vec::new();
                for e in items.iter() {
                    if e.separator {
                        menu_items.push(MenuItem::separator());
                    } else if e.heading {
                        menu_items.push(MenuItem::new(0, &e.label).enabled(false));
                    } else {
                        picks.push(e.index);
                        menu_items.push(MenuItem::new(picks.len() as u32, &e.label).enabled(e.enabled).checked(e.checked));
                    }
                }
                let origin = match kind {
                    MenuKind::Context => {
                        menu_items.push(MenuItem::separator());
                        menu_items.push(MenuItem::new(INSPECT, "Inspect Element"));
                        self.last_point
                    }
                    MenuKind::Select => (frame.0 as f32 + rect.0 as f32, frame.1 as f32 + (rect.1 + rect.3) as f32),
                };
                let width = match kind {
                    MenuKind::Context => 200.0,
                    MenuKind::Select => (rect.2 as f32 / one).clamp(140.0, 360.0),
                };
                let mut menu = Menu::new(menu_items);
                if *kind == MenuKind::Select {
                    menu.highlighted = menu.items.iter().position(|i| i.checked);
                }
                self.menu = Some(PageMenu { view: *id, token: *token, kind: *kind, origin, width, menu, picks });
                true
            }
            WebViewEvent::ControlHidden(id, token) => {
                self.dialogs.retain(|d| !(d.view == *id && d.token == *token));
                if self.menu.as_ref().is_some_and(|m| m.view == *id && m.token == *token) {
                    self.menu = None;
                }
                true
            }
            _ => false,
        }
    }

    /// The menu's top-left in points, kept inside the window.
    fn menu_origin(m: &PageMenu, ui: &Ui, w: u32, h: u32) -> (f32, f32) {
        let pt = ui.pt(1.0);
        let (_, total) = m.menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        let (wpt, hpt) = (w as f32 / pt, h as f32 / pt);
        ((m.origin.0 / pt).min(wpt - m.width - 4.0).max(4.0), (m.origin.1 / pt).min(hpt - total - 4.0).max(4.0))
    }

    fn dismiss_menu(&mut self, pool: &mut Option<WebProcessPool>) {
        if let (Some(m), Some(p)) = (self.menu.take(), pool.as_mut()) {
            p.menu_reply(m.view, m.token, &[]);
        }
    }

    /// Sheet rectangles in window pixels: (card, field, cancel, ok).
    fn layout(&self, ui: &Ui, w: u32, h: u32) -> Option<(Rect, Option<Rect>, Option<Rect>, Rect, Vec<String>)> {
        let d = self.dialogs.front()?;
        let lines = self.wrap(ui, &d.message);
        let field_h = if d.kind == DialogKind::Prompt { FIELD_H + 10.0 } else { 0.0 };
        let ch = PAD + LINE + 8.0 + LINE * lines.len() as f32 + 8.0 + field_h + PAD + BTN_H + PAD;
        let (wpt, hpt) = (w as f32 / ui.pt(1.0), h as f32 / ui.pt(1.0));
        let card = Rect { x: ((wpt - SHEET_W) / 2.0).max(0.0), y: (hpt * 0.3 - ch / 2.0).max(8.0), w: SHEET_W, h: ch };
        let by = card.y + ch - PAD - BTN_H;
        let inner = SHEET_W - 2.0 * PAD;
        let field = (d.kind == DialogKind::Prompt).then(|| Rect { x: card.x + PAD, y: by - PAD - FIELD_H, w: inner, h: FIELD_H });
        let (cancel, ok) = if d.kind == DialogKind::Alert {
            (None, Rect { x: card.x + PAD, y: by, w: inner, h: BTN_H })
        } else {
            let bw = (inner - 8.0) / 2.0;
            (Some(Rect { x: card.x + PAD, y: by, w: bw, h: BTN_H }), Rect { x: card.x + PAD + bw + 8.0, y: by, w: bw, h: BTN_H })
        };
        Some((card, field, cancel, ok, lines))
    }

    fn wrap(&self, ui: &Ui, msg: &str) -> Vec<String> {
        // paragraphs keep their breaks; the sheet shows at most MAX_LINES lines
        let mut out = Vec::new();
        for para in msg.split('\n') {
            if para.trim().is_empty() {
                out.push(String::new());
            } else {
                out.extend(ui.wrap(para, MAC_BODY_PT, Weight::Regular, ui.pt(SHEET_W - 2.0 * PAD)));
            }
            if out.len() >= MAX_LINES {
                out.truncate(MAX_LINES);
                if let Some(l) = out.last_mut() {
                    l.push('\u{2026}');
                }
                break;
            }
        }
        out
    }

    fn answer(&mut self, pool: &mut Option<WebProcessPool>, ok: bool) {
        if let Some(d) = self.dialogs.pop_front() {
            if let Some(p) = pool {
                p.dialog_reply(d.view, d.token, ok, &d.field.to_string());
            }
        }
    }

    /// A tap at window pixel (x, y) while something is up. Returns what the browser should do; the tap is consumed.
    pub fn touch_up(&mut self, ui: &Ui, w: u32, h: u32, x: f32, y: f32, pool: &mut Option<WebProcessPool>) -> Outcome {
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let hit = |r: &Rect| px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h;
        if let Some((_, _, cancel, ok, _)) = self.layout(ui, w, h) {
            if hit(&ok) {
                self.answer(pool, true);
            } else if cancel.as_ref().is_some_and(hit) {
                self.answer(pool, false);
            }
            return Outcome::None;
        }
        let Some(m) = &self.menu else { return Outcome::None };
        let (ox, oy) = Self::menu_origin(m, ui, w, h);
        let inside = px >= ox && px < ox + m.width;
        let row = if inside { m.menu.hit(py - oy, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT) } else { None };
        match row {
            Some(i) => {
                let id = m.menu.items[i].id;
                self.choose(id, pool)
            }
            // a tap outside dismisses; a tap on padding or a disabled row keeps it open
            None if !inside || py < oy => {
                self.dismiss_menu(pool);
                Outcome::None
            }
            None => Outcome::None,
        }
    }

    pub fn touch_move(&mut self, ui: &Ui, w: u32, h: u32, x: f32, y: f32) {
        if let Some(m) = &mut self.menu {
            let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
            let (ox, oy) = Self::menu_origin(m, ui, w, h);
            if px >= ox && px < ox + m.width {
                if let Some(i) = m.menu.hit(py - oy, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT) {
                    m.menu.highlighted = Some(i);
                }
            }
        }
    }

    fn choose(&mut self, id: u32, pool: &mut Option<WebProcessPool>) -> Outcome {
        let Some(m) = self.menu.take() else { return Outcome::None };
        if id == INSPECT {
            if let Some(p) = pool.as_mut() {
                p.menu_reply(m.view, m.token, &[]);
            }
            return Outcome::Inspect;
        }
        if let (Some(&idx), Some(p)) = (m.picks.get(id as usize - 1), pool.as_mut()) {
            p.menu_reply(m.view, m.token, &[idx]);
        } else if let Some(p) = pool.as_mut() {
            p.menu_reply(m.view, m.token, &[]);
        }
        Outcome::None
    }

    /// Keys while something is up; always consumed. Return confirms, Escape cancels, arrows move in a menu.
    pub fn key(&mut self, k: &KeyInput, pool: &mut Option<WebProcessPool>) -> Outcome {
        if let Some(kind) = self.dialogs.front().map(|d| d.kind) {
            match k {
                KeyInput::Code(13) => self.answer(pool, true),
                KeyInput::Code(27) | KeyInput::Code(1) => self.answer(pool, kind == DialogKind::Alert),
                _ if kind == DialogKind::Prompt => {
                    if let Some(d) = self.dialogs.front_mut() {
                        match k {
                            KeyInput::Text(s) => d.field.push_str(s),
                            KeyInput::Code(8) => d.field.pop(),
                            KeyInput::Code(37) => d.field.move_caret(false),
                            KeyInput::Code(39) => d.field.move_caret(true),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return Outcome::None;
        }
        let Some(m) = &mut self.menu else { return Outcome::None };
        match k {
            KeyInput::Code(38) => {
                m.menu.step(-1);
            }
            KeyInput::Code(40) => {
                m.menu.step(1);
            }
            KeyInput::Code(13) => {
                if let Some(id) = m.menu.activate() {
                    return self.choose(id, pool);
                }
            }
            KeyInput::Code(27) | KeyInput::Code(1) => self.dismiss_menu(pool),
            KeyInput::Text(s) => {
                if let Some(c) = s.chars().next() {
                    m.menu.type_select(c, 0.0);
                }
            }
            _ => {}
        }
        Outcome::None
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: u32, h: u32) {
        if let (Some(d), Some((card, field, cancel, ok, lines))) = (self.dialogs.front(), self.layout(ui, w, h)) {
            s.fill_rect(0, 0, s.w, s.h, rgba(0, 0, 0, 110));
            s.fill_rrect(ui.pt(card.x), ui.pt(card.y), ui.pt(card.w), ui.pt(card.h), ui.pt(14.0), CARD_BG.now());
            let (lx, mut ty) = (ui.pt(card.x + PAD) as i32, ui.pt(card.y + PAD));
            let title = if d.host.is_empty() { "This page says".to_string() } else { format!("\u{201c}{}\u{201d} says", d.host) };
            let t = ui.text(&ui.fit(&title, MAC_BODY_PT, ui.pt(SHEET_W - 2.0 * PAD)), MAC_BODY_PT, Weight::Semibold, LABEL.now());
            s.blit(&t, lx, ty as i32, 255, 255);
            ty += ui.pt(LINE + 8.0);
            for l in &lines {
                if !l.is_empty() {
                    let img = ui.text(l, MAC_BODY_PT, Weight::Regular, LABEL.now());
                    s.blit(&img, lx, ty as i32, 255, 255);
                }
                ty += ui.pt(LINE);
            }
            if let Some(f) = field {
                widgets::text_field(s, ui, &d.field, f, "", true);
            }
            let button = |s: &mut Surface, r: Rect, label: &str, primary: bool| {
                s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(r.h / 2.0), if primary { TINT } else { FIELD_BG.now() });
                let img = ui.text(label, MAC_BODY_PT, Weight::Regular, if primary { WHITE } else { LABEL.now() });
                s.blit(&img, (ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y + r.h / 2.0) as i32) - img.h as i32 / 2, 255, 255);
            };
            if let Some(c) = cancel {
                button(s, c, "Cancel", false);
            }
            button(s, ok, "OK", true);
            if self.dialogs.len() > 1 {
                let more = ui.text(&format!("{} more dialogs", self.dialogs.len() - 1), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
                s.blit(&more, lx, ui.pt(card.y + card.h + 6.0) as i32, 255, 255);
            }
            return;
        }
        if let Some(m) = &self.menu {
            widgets::menu(s, ui, Self::menu_origin(m, ui, w, h), m.width, &m.menu);
        }
    }
}
