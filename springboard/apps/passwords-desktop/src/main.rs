//! Passwords as an installed BifrOSt app in the full-size window: the items in the shared passwords group, split into
//! Logins, Notes and Certificates tabs, with a search field and a detail column. A login has Reveal or Hide, Copy,
//! Generate (a new strong password for the login) and Delete; a note or certificate has Reveal or Hide, Copy and
//! Delete. Import reads a CSV with site, account and password columns (Open With on a .csv file); Export writes every
//! login to `~/Downloads/passwords.csv` with mode 0600. Pure logic (search, kinds, CSV, generator) is in `model.rs`.
//!
//! Secrets stay in the shell's keychain: the app reads one at a time, only when an item is revealed, copied,
//! exported or replaced, and it never writes a secret to the log or the status line. The shell asks for the passcode
//! before it shows a password in Settings; this app shows a revealed secret when the user asks for it.

mod model;

use bifrost_app::keychain::Error as KeychainError;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::passwords::{kind_of, name_of, Kind};
use bifrost_app::{keychain_groups, passwords, widgets, App, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use model::{Entry, Item};
use std::collections::HashMap;
use std::path::PathBuf;

/// Header and detail geometry in points.
const LIST_PT: f32 = 300.0;
const BAR_Y_PT: f32 = 12.0;
const BAR_H_PT: f32 = 28.0;
const TABS_Y_PT: f32 = 46.0;
const TABS_H_PT: f32 = 26.0;
const SEARCH_Y_PT: f32 = 80.0;
const HEADER_PT: f32 = 124.0;
const ROW_PT: f32 = 44.0;
const PAD_PT: f32 = 12.0;
/// Bar button hit boxes in the list header (x start, width) and in the detail column.
const IMPORT_BTN: (f32, f32) = (PAD_PT, 64.0);
const EXPORT_BTN: (f32, f32) = (PAD_PT + 72.0, 64.0);
/// The kind tabs under the bar: the kind, its label, and its hit box (x start, width).
const TABS: [(Kind, &str, f32, f32); 3] = [(Kind::Login, "Logins", PAD_PT, 80.0), (Kind::SecureNote, "Notes", PAD_PT + 84.0, 72.0), (Kind::Certificate, "Certificates", PAD_PT + 160.0, 108.0)];
const DETAIL_BTNS: [(&str, f32, f32); 4] = [("Reveal", 0.0, 80.0), ("Copy", 88.0, 64.0), ("Generate", 160.0, 84.0), ("Delete", 252.0, 72.0)];

/// What a service request was for, so its reply can be handled.
enum Pending {
    List,
    Reveal,
    Copy,
    Generate,
    Delete { service: String, account: String },
    Import { entry: Entry },
    ImportUpdate,
    Export { service: String, account: String },
}

#[derive(Default)]
struct PasswordsApp {
    items: Vec<Item>,
    listed: bool,
    tab: Kind,
    query: String,
    selected: Option<(String, String)>,
    revealed: Option<String>,
    pending: HashMap<u32, Pending>,
    export_rows: Vec<Entry>,
    export_left: usize,
    imported: usize,
    status: String,
}

impl PasswordsApp {
    fn ask_list(&mut self, ctx: &Ctx) {
        let id = passwords::list(ctx);
        self.pending.insert(id, Pending::List);
    }

    /// The items the list shows now: the current tab, filtered by the search, in order.
    fn shown(&self) -> Vec<Item> {
        model::filter(&self.items, self.tab, &self.query).into_iter().cloned().collect()
    }

    /// The filtered password rows, with their current selection, for the shared source-list widget.
    fn source_list(&self) -> (Vec<Item>, SourceList) {
        let shown = self.shown();
        let rows = shown.iter().enumerate().map(|(i, item)| {
            let symbol = match item.kind {
                Kind::Login => "person.crop.circle",
                Kind::SecureNote => "doc",
                Kind::Certificate => "checkmark.circle",
            };
            SidebarItem::row(i as u32, item.name()).with_symbol(symbol).with_subtitle(subtitle(item))
        }).collect();
        let mut list = SourceList::new(rows, ROW_PT - 16.0, 22.0);
        list.two_line_h = ROW_PT;
        let selected = self.selected.as_ref().and_then(|(service, account)| {
            shown.iter().position(|item| item.service == *service && item.account == *account)
        });
        list.set_selected(selected.map(|i| i as u32));
        (shown, list)
    }

    fn select(&mut self, service: String, account: String) {
        self.selected = Some((service, account));
        self.revealed = None;
    }

    fn set_tab(&mut self, kind: Kind) {
        self.tab = kind;
        self.selected = None;
        self.revealed = None;
    }

    fn import(&mut self, ctx: &Ctx, path: &str) {
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| model::parse_import(&t)) {
            Ok(got) => {
                self.imported = 0;
                let count = got.entries.len();
                for entry in got.entries {
                    let id = passwords::save(ctx, &entry.site, &entry.account, &entry.password);
                    self.pending.insert(id, Pending::Import { entry });
                }
                self.status = format!("Importing {count} logins{}.", skipped_text(got.skipped));
            }
            Err(e) => self.status = format!("Could not import: {e}"),
        }
    }

    /// Asks for the password of every login; the CSV is written once they have all come back.
    fn export(&mut self, ctx: &Ctx) {
        let logins: Vec<Item> = self.items.iter().filter(|i| i.kind == Kind::Login).cloned().collect();
        if logins.is_empty() {
            self.status = "No logins to export.".into();
            return;
        }
        self.export_rows.clear();
        self.export_left = logins.len();
        for l in logins {
            let id = passwords::fill(ctx, &l.service, &l.account);
            self.pending.insert(id, Pending::Export { service: l.service, account: l.account });
        }
        self.status = "Exporting logins.".into();
    }

    /// Writes the export once every password has come back; a failed write is shown in the status line.
    fn finish_export(&mut self) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let dir = home.join("Downloads");
        let text = model::render_export(&self.export_rows);
        let rows = self.export_rows.len();
        self.status = match write_private(&dir.join("passwords.csv"), text.as_bytes()) {
            Ok(()) => format!("Exported {rows} logins to Downloads."),
            Err(e) => format!("Could not export: {e}"),
        };
        self.export_rows.clear();
    }
}

fn skipped_text(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!(", {n} skipped")
    }
}

/// The label of a note or certificate, which has no user name to show.
fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Login => "",
        Kind::SecureNote => "Secure note",
        Kind::Certificate => "Certificate",
    }
}

/// The second line of a list row: the account for a login, the kind for a note or certificate.
fn subtitle(item: &Item) -> &str {
    if item.kind == Kind::Login {
        &item.account
    } else {
        kind_label(item.kind)
    }
}

/// What an empty list of `kind` says.
fn empty_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Login => "No Saved Passwords",
        Kind::SecureNote => "No Secure Notes",
        Kind::Certificate => "No Certificates",
    }
}

/// Creates the file for the owner only, then writes it (the export holds passwords in clear text).
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)
}

/// The kind tab at point `(x, y)` in the list column, if any.
fn tab_at(x: f32, y: f32) -> Option<Kind> {
    if !(TABS_Y_PT..TABS_Y_PT + TABS_H_PT).contains(&y) {
        return None;
    }
    TABS.iter().find(|(_, _, bx, bw)| (*bx..bx + bw).contains(&x)).map(|(kind, _, _, _)| *kind)
}

/// The bar button at point `(x, y)` in the detail column, if any.
fn detail_button(x: f32, y: f32) -> Option<&'static str> {
    if !(BAR_Y_PT..BAR_Y_PT + BAR_H_PT).contains(&y) {
        return None;
    }
    let base = LIST_PT + PAD_PT;
    DETAIL_BTNS.iter().find(|(_, bx, bw)| (base + bx..base + bx + bw).contains(&x)).map(|(name, _, _)| *name)
}

impl App for PasswordsApp {
    fn id(&self) -> &str {
        "is.olibuijr.passwords.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        if !self.listed {
            self.listed = true;
            self.ask_list(ctx);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                if x < LIST_PT {
                    if (BAR_Y_PT..BAR_Y_PT + BAR_H_PT).contains(&y) {
                        if (IMPORT_BTN.0..IMPORT_BTN.0 + IMPORT_BTN.1).contains(&x) {
                            self.status = "Open a .csv file with Passwords to import it.".into();
                            return true;
                        }
                        if (EXPORT_BTN.0..EXPORT_BTN.0 + EXPORT_BTN.1).contains(&x) {
                            self.export(ctx);
                            return true;
                        }
                        return false;
                    }
                    if let Some(kind) = tab_at(x, y) {
                        self.set_tab(kind);
                        return true;
                    }
                    let (shown, list) = self.source_list();
                    let bounds = Rect { x: 0.0, y: HEADER_PT, w: LIST_PT, h: ctx.cfg.h as f32 / ctx.ui.s - HEADER_PT };
                    if let Some(index) = list.hit(bounds, 0.0, x, y).map(|i| i as usize) {
                        if let Some(item) = shown.get(index) {
                            self.select(item.service.clone(), item.account.clone());
                        }
                        return true;
                    }
                    return false;
                }
                let Some((service, account)) = self.selected.clone() else { return false };
                let is_login = kind_of(&service) == Kind::Login;
                match detail_button(x, y) {
                    Some("Reveal") if self.revealed.is_some() => {
                        self.revealed = None;
                    }
                    Some("Reveal") => {
                        let id = passwords::fill(ctx, &service, &account);
                        self.pending.insert(id, Pending::Reveal);
                    }
                    Some("Copy") => match &self.revealed {
                        Some(p) => {
                            clipboard::set_text(p);
                            self.status = "Copied.".into();
                        }
                        None => {
                            let id = passwords::fill(ctx, &service, &account);
                            self.pending.insert(id, Pending::Copy);
                        }
                    },
                    Some("Generate") if is_login => match model::generate() {
                        Ok(p) => {
                            let id = passwords::update(ctx, &service, &account, &p);
                            self.pending.insert(id, Pending::Generate);
                            self.revealed = Some(p);
                        }
                        Err(e) => self.status = format!("Could not generate: {e}"),
                    },
                    Some("Delete") => {
                        let id = passwords::remove(ctx, &service, &account);
                        self.pending.insert(id, Pending::Delete { service, account });
                    }
                    _ => return false,
                }
                true
            }
            Input::Key(KeyInput::Text(text)) => {
                self.query.push_str(&text);
                self.selected = None;
                self.revealed = None;
                true
            }
            Input::Key(KeyInput::Code(8)) => {
                self.query.pop();
                true
            }
            Input::Key(KeyInput::Code(27)) => {
                self.query.clear();
                true
            }
            Input::Open(path) => {
                self.import(ctx, &path);
                true
            }
            _ => false,
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(job) = self.pending.remove(&request) else { return false };
        match job {
            Pending::List => {
                if let Some(rows) = passwords::decode_list(result) {
                    self.items = rows.into_iter().map(|r| Item::new(r.service, r.account)).collect();
                    model::sort(&mut self.items);
                }
            }
            Pending::Reveal => match passwords::decode_password(result) {
                Ok(p) => self.revealed = Some(p),
                Err(e) => self.status = format!("Could not show it: {e}"),
            },
            Pending::Copy => match passwords::decode_password(result) {
                Ok(p) => {
                    clipboard::set_text(&p);
                    self.status = "Copied.".into();
                }
                Err(e) => self.status = format!("Could not copy: {e}"),
            },
            Pending::Generate => match passwords::decode_password(result) {
                Ok(_) => self.status = "New password saved.".into(),
                Err(e) => self.status = format!("Could not save the new password: {e}"),
            },
            Pending::Delete { service, account } => match passwords::decode_password(result) {
                Ok(_) | Err(KeychainError::ItemNotFound) => {
                    self.items.retain(|i| !(i.service == service && i.account == account));
                    if self.selected.as_ref().is_some_and(|(s, a)| *s == service && *a == account) {
                        self.selected = None;
                        self.revealed = None;
                    }
                    self.status = "Deleted.".into();
                }
                Err(e) => self.status = format!("Could not delete: {e}"),
            },
            Pending::Import { entry } => match passwords::decode_password(result) {
                Ok(_) => self.imported += 1,
                // the login exists: replace its password with the imported one
                Err(KeychainError::DuplicateItem) => {
                    let id = passwords::update(ctx, &entry.site, &entry.account, &entry.password);
                    self.pending.insert(id, Pending::ImportUpdate);
                    return true;
                }
                Err(e) => self.status = format!("Could not import {}: {e}", entry.site),
            },
            Pending::ImportUpdate => match passwords::decode_password(result) {
                Ok(_) => self.imported += 1,
                Err(e) => self.status = format!("Could not update a login: {e}"),
            },
            Pending::Export { service, account } => {
                match passwords::decode_password(result) {
                    Ok(password) => self.export_rows.push(Entry { site: service, account, password }),
                    Err(e) => self.status = format!("Could not read {service}: {e}"),
                }
                self.export_left = self.export_left.saturating_sub(1);
                if self.export_left == 0 {
                    self.finish_export();
                }
                return true;
            }
        }
        if self.imported > 0 && self.pending.values().all(|p| !matches!(p, Pending::Import { .. } | Pending::ImportUpdate)) {
            self.status = format!("Imported {} logins.", self.imported);
            self.imported = 0;
            self.ask_list(ctx);
        }
        true
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        if keychain_groups::changed(service, event, body).is_some_and(|c| c.group == passwords::GROUP) {
            self.ask_list(ctx);
            return true;
        }
        false
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let scale = ui.s;
        let p = |v: f32| (v * scale) as usize;
        // positions for blit_colored, which takes signed pixels
        let pi = |v: f32| (v * scale) as i32;
        let (w, h) = (s.w, s.h);
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());
        // list column with its bar, kind tabs and search field
        s.fill_rect(0, 0, p(LIST_PT), h, ui::SIDEBAR_BG.now());
        for (label, x) in [("Import", IMPORT_BTN.0), ("Export", EXPORT_BTN.0)] {
            let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, ui::LINK);
            s.blit_colored(&img, pi(x + 4.0), pi(BAR_Y_PT + 6.0), ui::LINK, 255);
        }
        for (kind, label, x, width) in TABS {
            let on = kind == self.tab;
            let colour = if on { ui::LABEL.now() } else { ui::LINK };
            if on {
                s.fill_rect(p(x), p(TABS_Y_PT), p(width), p(TABS_H_PT), ui::ROW_PRESSED.now());
            }
            let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, colour);
            s.blit_colored(&img, pi(x + 6.0), pi(TABS_Y_PT + 5.0), colour, 255);
        }
        let search = if self.query.is_empty() { "Search".to_string() } else { self.query.clone() };
        let search_colour = if self.query.is_empty() { ui::LABEL_2.now() } else { ui::LABEL.now() };
        s.fill_rect(p(PAD_PT), p(SEARCH_Y_PT), p(LIST_PT - 2.0 * PAD_PT), p(26.0), ui::FIELD_BG.now());
        let img = ui.text(&ui.fit(&search, ui::MAC_BODY_PT, ui.pt(LIST_PT - 4.0 * PAD_PT)), ui::MAC_BODY_PT, Weight::Regular, search_colour);
        s.blit_colored(&img, pi(PAD_PT + 8.0), pi(SEARCH_Y_PT + 5.0), search_colour, 255);
        let (shown, list) = self.source_list();
        let bounds = Rect { x: 0.0, y: HEADER_PT, w: LIST_PT, h: h as f32 / scale - HEADER_PT };
        widgets::source_list(s, ui, bounds, &list, 0.0, None);
        if shown.is_empty() {
            let text = if self.items.iter().any(|i| i.kind == self.tab) { "No Matches" } else { empty_text(self.tab) };
            let img = ui.text(text, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, pi(PAD_PT), pi(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
        }
        s.fill_rect(p(LIST_PT - 1.0), 0, 1, h, ui::SEPARATOR_C.now());
        // detail column
        let px = LIST_PT + PAD_PT;
        match &self.selected {
            Some((service, account)) => {
                let kind = kind_of(service);
                for (name, bx, _) in DETAIL_BTNS {
                    if name == "Generate" && kind != Kind::Login {
                        continue;
                    }
                    let colour = match name {
                        "Delete" => ui::DESTRUCTIVE,
                        _ => ui::LINK,
                    };
                    let label = if name == "Reveal" && self.revealed.is_some() { "Hide" } else { name };
                    let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, colour);
                    s.blit_colored(&img, pi(px + bx + 2.0), pi(BAR_Y_PT + 6.0), colour, 255);
                }
                let width = w as f32 / scale - px - PAD_PT;
                let title = ui.text(&ui.fit(name_of(service), ui::MAC_TITLE2_PT, ui.pt(width)), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&title, pi(px), pi(HEADER_PT), ui::LABEL.now(), 255);
                let sub = if kind == Kind::Login { account.as_str() } else { kind_label(kind) };
                let user = ui.text(&ui.fit(sub, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&user, pi(px), pi(HEADER_PT + 36.0), ui::LABEL_2.now(), 255);
                // a note or certificate can be several lines: the detail shows its first line
                let shown_secret = match &self.revealed {
                    Some(v) if kind == Kind::Login => v.clone(),
                    Some(v) => v.lines().next().unwrap_or("").to_string(),
                    None => "••••••••••••".to_string(),
                };
                let pw = ui.text(&ui.fit(&shown_secret, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&pw, pi(px), pi(HEADER_PT + 72.0), ui::LABEL.now(), 255);
            }
            None => {
                let hint = ui.text("Select an item to see it", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&hint, pi(px), pi(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
            }
        }
        if !self.status.is_empty() {
            let width = w as f32 / scale - PAD_PT * 2.0;
            let img = ui.text(&ui.fit(&self.status, ui::MAC_FOOTNOTE_PT, ui.pt(width)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, pi(PAD_PT), pi(h as f32 / scale - 24.0), ui::LABEL_2.now(), 255);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PasswordsApp::default());
}
