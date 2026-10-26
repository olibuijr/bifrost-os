//! Keychain Access as an installed BifrOSt app in the full-size window: the keychain groups this app may read (its
//! `keychain_groups` in manifest.toml) in a sidebar, and the items of the selected group with the account and the time
//! each was last written. No secret is read or shown here. Delete removes the selected item after a second press.
//! File > New Item (Cmd+N) opens a sheet that adds an item to the selected group.
//! Pure logic (order, titles, dates) is in `model.rs`.

mod model;

use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::item_sheet::{ItemSheet, Outcome};
use bifrost_app::keychain::Error as KeychainError;
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::keychain_groups::{self, Row};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{widgets, App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use std::collections::HashMap;

/// Geometry in points. The sidebar is on the left; the header holds the group title and the Delete button.
const SIDE_PT: f32 = 220.0;
const HEADER_PT: f32 = 56.0;
const ROW_PT: f32 = 44.0;
const PAD_PT: f32 = 12.0;
const DELETE_W_PT: f32 = 72.0;
const DELETE_H_PT: f32 = 26.0;

/// File > New Item.
const MENU_NEW: u32 = 1;

/// The Delete button's top-left corner, relative to the content column of width `content_w`.
fn delete_at(content_w: f32) -> (f32, f32) {
    (content_w - PAD_PT - DELETE_W_PT, PAD_PT)
}

/// What a service request was for, so its reply can be handled.
enum Pending {
    Groups,
    List { group: String },
    Delete { group: String },
    Add { group: String },
}

#[derive(Default)]
struct KeychainAccessApp {
    started: bool,
    groups: Vec<String>,
    group: Option<usize>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// The first Delete press was made for the selected item; the second one removes it.
    armed: bool,
    pending: HashMap<u32, Pending>,
    status: String,
    /// The New Item sheet while it is open; it blocks the window.
    adding: Option<ItemSheet>,
}

impl KeychainAccessApp {
    fn current(&self) -> Option<&str> {
        self.group.and_then(|i| self.groups.get(i)).map(String::as_str)
    }

    fn ask_groups(&mut self, ctx: &Ctx) {
        let id = keychain_groups::groups(ctx);
        self.pending.insert(id, Pending::Groups);
    }

    fn ask_list(&mut self, ctx: &Ctx) {
        let Some(group) = self.current().map(str::to_string) else {
            self.rows.clear();
            return;
        };
        let id = keychain_groups::list(ctx, &group);
        self.pending.insert(id, Pending::List { group });
    }

    fn select_group(&mut self, ctx: &Ctx, index: usize) {
        if self.group == Some(index) {
            return;
        }
        self.group = Some(index);
        self.rows.clear();
        self.selected = None;
        self.armed = false;
        self.ask_list(ctx);
    }

    /// Opens the New Item sheet for the selected group.
    fn open_add(&mut self) -> bool {
        if self.adding.is_some() || self.current().is_none() {
            return false;
        }
        self.adding = Some(ItemSheet::new("New Item", &[("Name", "Service or website", false), ("Account", "User name", false), ("Password", "", true)]));
        true
    }

    /// Input while the New Item sheet is open: it takes every press and key.
    fn adding_event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let parent = Rect { x: 0.0, y: 0.0, w: ctx.cfg.w as f32 / ctx.ui.s, h: ctx.cfg.h as f32 / ctx.ui.s };
        let Some(sheet) = self.adding.as_mut() else { return false };
        let outcome = match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => sheet.press(parent, UNIFIED_BAR_PT, x / ctx.ui.s, y / ctx.ui.s),
            Input::Key(KeyInput::Text(text)) => sheet.type_text(&text),
            Input::Key(KeyInput::Code(code)) => sheet.code(code),
            _ => Outcome::None,
        };
        match outcome {
            Outcome::None => false,
            Outcome::Redraw => true,
            Outcome::Cancel => {
                self.adding = None;
                true
            }
            Outcome::Add(values) => {
                self.adding = None;
                if let Some(group) = self.current().map(str::to_string) {
                    let id = keychain_groups::add(ctx, &group, &values[0], &values[1], values[2].as_bytes());
                    self.pending.insert(id, Pending::Add { group });
                }
                true
            }
        }
    }

    fn sidebar(&self) -> SourceList {
        let rows = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| SidebarItem::row(i as u32, &model::title(g)).with_symbol("lock").with_subtitle(g))
            .collect();
        let mut list = SourceList::new(rows, ROW_PT - 16.0, 22.0);
        list.two_line_h = ROW_PT;
        list.set_selected(self.group.map(|i| i as u32));
        list
    }

    /// Removes the selected item. The first call only arms it and says what it will remove.
    fn delete_selected(&mut self, ctx: &Ctx) {
        let (Some(group), Some(index)) = (self.current().map(str::to_string), self.selected) else { return };
        let Some(row) = self.rows.get(index) else { return };
        if !self.armed {
            self.armed = true;
            self.status = format!("Press Delete again to remove {}.", row.service);
            return;
        }
        let id = keychain_groups::delete(ctx, &group, &row.service, &row.account);
        self.pending.insert(id, Pending::Delete { group });
        self.armed = false;
        self.selected = None;
    }
}

impl App for KeychainAccessApp {
    fn id(&self) -> &str {
        "is.olibuijr.keychain-access.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // nothing is kept in memory that is not in the shell's keychain
        ctx.enable_sudden_termination();
        if !self.started {
            self.started = true;
            self.ask_groups(ctx);
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        vec![AppMenu {
            title: "File".into(),
            items: vec![AppMenuItem::new(MENU_NEW, "New Item\u{2026}").key("cmd+n").enabled(self.adding.is_none() && self.current().is_some())],
        }]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        id == MENU_NEW && self.open_add()
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if self.adding.is_some() {
            return self.adding_event(ctx, ev);
        }
        let Input::Touch { kind: TouchKind::Down, x, y, .. } = ev else { return false };
        let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
        if x < SIDE_PT {
            let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: ctx.cfg.h as f32 / ctx.ui.s };
            return match self.sidebar().hit(bounds, 0.0, x, y) {
                Some(i) if (i as usize) < self.groups.len() => {
                    self.select_group(ctx, i as usize);
                    true
                }
                _ => false,
            };
        }
        let content_w = ctx.cfg.w as f32 / ctx.ui.s - SIDE_PT;
        let (dx, dy) = delete_at(content_w);
        let (bx, by) = (SIDE_PT + dx, dy);
        if self.selected.is_some() && (bx..bx + DELETE_W_PT).contains(&x) && (by..by + DELETE_H_PT).contains(&y) {
            self.delete_selected(ctx);
            return true;
        }
        if y >= HEADER_PT {
            let index = ((y - HEADER_PT) / ROW_PT) as usize;
            if index < self.rows.len() {
                if self.selected != Some(index) {
                    self.selected = Some(index);
                    self.armed = false;
                    self.status.clear();
                }
                return true;
            }
        }
        false
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(job) = self.pending.remove(&request) else { return false };
        match job {
            Pending::Groups => match keychain_groups::decode_groups(result) {
                Some(mut groups) => {
                    groups.sort();
                    self.groups = groups;
                    if self.group.is_none() && !self.groups.is_empty() {
                        self.select_group(ctx, 0);
                    }
                }
                None => self.status = "Could not read the keychain groups.".into(),
            },
            Pending::List { group } => {
                if self.current() == Some(group.as_str()) {
                    match keychain_groups::decode_list(result) {
                        Some(mut rows) => {
                            model::sort(&mut rows);
                            self.rows = rows;
                        }
                        None => self.status = "Could not list the items.".into(),
                    }
                }
            }
            Pending::Delete { group } => {
                match keychain_groups::decode(result) {
                    Ok(_) => self.status = "Item deleted.".into(),
                    Err(e) => self.status = format!("Could not delete: {e}"),
                }
                if self.current() == Some(group.as_str()) {
                    self.ask_list(ctx);
                }
            }
            Pending::Add { group } => {
                match keychain_groups::decode(result) {
                    Ok(_) => self.status = "Item added.".into(),
                    Err(KeychainError::DuplicateItem) => self.status = "An item with that name and account already exists.".into(),
                    Err(e) => self.status = format!("Could not add: {e}"),
                }
                if self.current() == Some(group.as_str()) {
                    self.ask_list(ctx);
                }
            }
        }
        true
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(change) = keychain_groups::changed(service, event, body) else { return false };
        if self.current() == Some(change.group.as_str()) {
            self.ask_list(ctx);
            return true;
        }
        false
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui: &Ui = &ctx.ui;
        let scale = ui.s;
        let p = |v: f32| (v * scale) as usize;
        // positions for blit_colored, which takes signed pixels
        let pi = |v: f32| (v * scale) as i32;
        let (w, h) = (s.w, s.h);
        let height_pt = h as f32 / scale;
        let content_w = w as f32 / scale - SIDE_PT;
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());
        // sidebar of groups
        s.fill_rect(0, 0, p(SIDE_PT), h, ui::SIDEBAR_BG.now());
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: height_pt };
        widgets::source_list(s, ui, bounds, &self.sidebar(), 0.0, None);
        s.fill_rect(p(SIDE_PT - 1.0), 0, 1, h, ui::SEPARATOR_C.now());
        // header: the group title and Delete
        let title = self.current().map(model::title).unwrap_or_else(|| "Keychain Access".to_string());
        let heading = ui.text(
            &ui.fit(&title, ui::MAC_TITLE2_PT, ui.pt(content_w - 2.0 * PAD_PT - DELETE_W_PT)),
            ui::MAC_TITLE2_PT,
            Weight::Regular,
            ui::LABEL.now(),
        );
        s.blit_colored(&heading, pi(SIDE_PT + PAD_PT), pi(PAD_PT), ui::LABEL.now(), 255);
        let (dx, dy) = delete_at(content_w);
        let colour = if self.selected.is_some() { ui::DESTRUCTIVE } else { ui::LABEL_2.now() };
        let label = ui.text("Delete", ui::MAC_BODY_PT, Weight::Regular, colour);
        s.blit_colored(&label, pi(SIDE_PT + dx + 8.0), pi(dy + 5.0), colour, 255);
        // items
        if self.rows.is_empty() {
            let text = if self.groups.is_empty() { "This app can read no keychain group." } else { "No items in this group." };
            let hint = ui.text(text, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&hint, pi(SIDE_PT + PAD_PT), pi(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
        }
        for (i, row) in self.rows.iter().enumerate() {
            let y = HEADER_PT + i as f32 * ROW_PT;
            if y + ROW_PT > height_pt {
                break;
            }
            if self.selected == Some(i) {
                s.fill_rect(p(SIDE_PT), p(y), w.saturating_sub(p(SIDE_PT)), p(ROW_PT), ui::ROW_PRESSED.now());
            }
            let service = ui.text(
                &ui.fit(&row.service, ui::MAC_BODY_PT, ui.pt(content_w * 0.6 - 2.0 * PAD_PT)),
                ui::MAC_BODY_PT,
                Weight::Regular,
                ui::LABEL.now(),
            );
            s.blit_colored(&service, pi(SIDE_PT + PAD_PT), pi(y + 6.0), ui::LABEL.now(), 255);
            let account = ui.text(
                &ui.fit(&row.account, ui::MAC_FOOTNOTE_PT, ui.pt(content_w * 0.6 - 2.0 * PAD_PT)),
                ui::MAC_FOOTNOTE_PT,
                Weight::Regular,
                ui::LABEL_2.now(),
            );
            s.blit_colored(&account, pi(SIDE_PT + PAD_PT), pi(y + 24.0), ui::LABEL_2.now(), 255);
            let when = ui.text(&model::stamp(row.modified), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&when, pi(SIDE_PT + content_w * 0.65), pi(y + 14.0), ui::LABEL_2.now(), 255);
            s.fill_rect(p(SIDE_PT + PAD_PT), p(y + ROW_PT) - 1, p(content_w - 2.0 * PAD_PT), 1, ui::SEPARATOR_C.now());
        }
        // status line
        if !self.status.is_empty() {
            let line = ui.text(
                &ui.fit(&self.status, ui::MAC_FOOTNOTE_PT, ui.pt(content_w - 2.0 * PAD_PT)),
                ui::MAC_FOOTNOTE_PT,
                Weight::Regular,
                ui::LABEL_2.now(),
            );
            s.blit_colored(&line, pi(SIDE_PT + PAD_PT), pi(height_pt - 24.0), ui::LABEL_2.now(), 255);
        }
        if let Some(sheet) = &self.adding {
            sheet.draw(s, ui, Rect { x: 0.0, y: 0.0, w: w as f32 / scale, h: height_pt }, UNIFIED_BAR_PT);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(KeychainAccessApp::default());
}
