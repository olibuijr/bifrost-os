//! Toolbar main-menu presentation. The SDK owns menu rows, keyboard navigation, type selection and drawing.
use bifrost_app::controls::menu::{Menu, MenuItem};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::widgets::{self, MENU_PAD_PT, MENU_ROW_PT, MENU_SEPARATOR_PT};
use bifrost_app::{KeyInput, TouchKind};

pub(super) const BACK: u32 = 2000;
pub(super) const NEW: u32 = 2001;
pub(super) const PRIVATE: u32 = 2002;
pub(super) const REOPEN: u32 = 2003;
pub(super) const BOOKMARKS: u32 = 2004;
pub(super) const HISTORY: u32 = 2005;
pub(super) const BOOKMARK: u32 = 2006;
pub(super) const SETTINGS: u32 = 2007;
pub(super) const INSPECTOR: u32 = 2008;
pub(super) const PAGE: u32 = 2009;
pub(super) const DEVELOP: u32 = 2010;
pub(super) const CLOSE: u32 = 2011;
pub(super) const NODE_BASE: u64 = 0x100000;
const WIDTH: f32 = 264.0;
const EDGE: f32 = 6.0;

#[derive(Default)]
pub(super) struct MainMenu {
    menu: Option<Menu>,
    scroll: f32,
    drag: Option<(f32, f32, bool)>,
    swallow_release: bool,
}

impl MainMenu {
    pub fn open(&self) -> bool { self.menu.is_some() }
    pub fn captures_touch(&self) -> bool { self.swallow_release }

    pub fn allows(&self, id: u32) -> bool {
        self.menu.as_ref().is_some_and(|menu| menu.items.iter().any(|item| item.id == id && item.enabled && !item.separator))
    }

    pub fn close(&mut self) {
        self.menu = None;
        self.drag = None;
        self.scroll = 0.0;
        self.swallow_release = false;
    }

    pub fn show(&mut self, items: Vec<MenuItem>) {
        self.menu = Some(Menu::new(items));
        self.scroll = 0.0;
        self.drag = None;
    }

    fn bounds(&self, ui: &Ui, w: u32, h: u32, anchor: Rect) -> Rect {
        let one = ui.pt(1.0);
        let (w, h) = (w as f32 / one, h as f32 / one);
        let total = self.menu.as_ref().map_or(0.0, |menu| menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT).1);
        let width = WIDTH.min((w - EDGE * 2.0).max(0.0));
        let y = (anchor.y + anchor.h + EDGE).min((h - EDGE).max(0.0));
        Rect { x: (anchor.x + anchor.w - width).clamp(EDGE.min(w), (w - width - EDGE).max(EDGE.min(w))), y, w: width, h: total.min((h - y - EDGE).max(0.0)) }
    }

    fn limit(&self, bounds: Rect) -> f32 {
        (self.menu.as_ref().map_or(0.0, |menu| menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT).1) - bounds.h).max(0.0)
    }

    pub fn draw(&self, surface: &mut Surface, ui: &Ui, w: u32, h: u32, anchor: Rect) {
        let Some(menu) = &self.menu else { return };
        let bounds = self.bounds(ui, w, h, anchor);
        if bounds.w <= 0.0 || bounds.h <= 0.0 { return; }
        let top = ui.pt(bounds.y).max(0.0) as usize;
        let bottom = (ui.pt(bounds.y + bounds.h).ceil() as usize).min(surface.h);
        let mut clipped = surface.rows(top.min(bottom), bottom);
        widgets::menu(&mut clipped, ui, (bounds.x, bounds.y - self.scroll.min(self.limit(bounds))), bounds.w, menu);
    }

    pub fn hover(&mut self, ui: &Ui, w: u32, h: u32, anchor: Rect, x: f32, y: f32) {
        let bounds = self.bounds(ui, w, h, anchor);
        let (x, y) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if let Some(menu) = &mut self.menu {
            menu.highlighted = bounds.contains(x, y).then(|| menu.hit(y - bounds.y + self.scroll, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT)).flatten();
        }
    }

    pub fn wheel(&mut self, ui: &Ui, w: u32, h: u32, anchor: Rect, dy: f32) {
        let bounds = self.bounds(ui, w, h, anchor);
        self.scroll = (self.scroll - dy / ui.pt(1.0)).clamp(0.0, self.limit(bounds));
    }

    pub fn touch(&mut self, ui: &Ui, w: u32, h: u32, anchor: Rect, kind: TouchKind, x: f32, y: f32) -> Option<u32> {
        if self.swallow_release {
            if matches!(kind, TouchKind::Up | TouchKind::Cancel) { self.swallow_release = false; }
            return None;
        }
        let bounds = self.bounds(ui, w, h, anchor);
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        match kind {
            TouchKind::Down => {
                if !bounds.contains(px, py) { self.close(); self.swallow_release = true; }
                else { self.drag = Some((py, self.scroll, false)); self.hover(ui, w, h, anchor, x, y); }
            }
            TouchKind::Move => {
                if let Some((start, scroll, dragging)) = self.drag {
                    if dragging || (py - start).abs() > EDGE {
                        self.scroll = (scroll + start - py).clamp(0.0, self.limit(bounds));
                        self.drag = Some((start, scroll, true));
                    }
                }
            }
            TouchKind::Up => {
                let dragged = self.drag.take().is_some_and(|drag| drag.2);
                if !dragged && bounds.contains(px, py) {
                    self.hover(ui, w, h, anchor, x, y);
                    return self.menu.as_ref().and_then(Menu::activate);
                }
            }
            TouchKind::Cancel => self.close(),
        }
        None
    }

    pub fn key(&mut self, ui: &Ui, w: u32, h: u32, anchor: Rect, key: &KeyInput, now: f64) -> Option<u32> {
        let bounds = self.bounds(ui, w, h, anchor);
        let Some(menu) = &mut self.menu else { return None };
        match key {
            KeyInput::Code(1 | 27) => { self.close(); return None; }
            KeyInput::Code(13) => return menu.activate(),
            KeyInput::Code(38) => { menu.step(-1); }
            KeyInput::Code(40) => { menu.step(1); }
            KeyInput::Code(36) => { menu.first(); }
            KeyInput::Code(35) => { menu.last(); }
            KeyInput::Text(text) => { for ch in text.chars().filter(|ch| !ch.is_control()) { menu.type_select(ch, now); } }
            _ => {}
        }
        if let Some(i) = menu.highlighted {
            let (rows, total) = menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
            if let Some(&(top, height)) = rows.get(i) {
                if top < self.scroll { self.scroll = top; }
                else if top + height > self.scroll + bounds.h { self.scroll = top + height - bounds.h; }
                self.scroll = self.scroll.clamp(0.0, (total - bounds.h).max(0.0));
            }
        }
        None
    }

    pub fn controls(&self, ui: &Ui, w: u32, h: u32, anchor: Rect) -> Vec<Node> {
        let Some(menu) = &self.menu else { return Vec::new() };
        let bounds = self.bounds(ui, w, h, anchor);
        let (rows, _) = menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        menu.items.iter().zip(rows).filter_map(|(item, (top, height))| {
            let y = bounds.y + top - self.scroll;
            (!item.separator && item.id != 0 && y >= bounds.y && y + height <= bounds.y + bounds.h).then(|| Node {
                id: NODE_BASE + item.id as u64, role: Role::MenuItem, label: item.title.clone(), value: if item.checked { "selected".into() } else { String::new() },
                bounds: Rect { x: bounds.x, y, w: bounds.w, h: height }, enabled: item.enabled,
            })
        }).collect()
    }
}
