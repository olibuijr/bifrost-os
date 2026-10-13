//! Passwords as an installed BifrOSt app in the full-size window: the saved website logins in the shared passwords
//! group (the logins Settings > Passwords lists), a search field, and a detail column with Reveal or Hide, Copy,
//! Generate (a new strong password for the login) and Delete. Import reads a CSV with site, account and password
//! columns (Open With on a .csv file); Export writes every login to `~/Downloads/passwords.csv` with mode 0600.
//! Pure logic (search, CSV, generator) is in `model.rs`.
//!
//! Secrets stay in the shell's keychain: the app reads one at a time, only when a login is revealed, copied,
//! exported or replaced, and it never writes a password to the log or the status line.

mod model;

use bifrost_app::keychain::Error as KeychainError;
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::{keychain_groups, passwords, App, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use model::{Entry, Login};
use std::collections::HashMap;
use std::path::PathBuf;

/// Header and detail geometry in points.
const LIST_PT: f32 = 300.0;
const HEADER_PT: f32 = 88.0;
const ROW_PT: f32 = 44.0;
const PAD_PT: f32 = 12.0;
const BAR_Y_PT: f32 = 12.0;
const BAR_H_PT: f32 = 28.0;
/// Bar button hit boxes in the list header (x start, width) and in the detail column.
const IMPORT_BTN: (f32, f32) = (PAD_PT, 64.0);
const EXPORT_BTN: (f32, f32) = (PAD_PT + 72.0, 64.0);
const DETAIL_BTNS: [(&str, f32, f32); 4] = [("Reveal", 0.0, 80.0), ("Copy", 88.0, 64.0), ("Generate", 160.0, 84.0), ("Delete", 252.0, 72.0)];

/// What a service request was for, so its reply can be handled.
enum Pending {
    List,
    Reveal,
    Copy,
    Generate,
    Delete { site: String, account: String },
    Import { entry: Entry },
    ImportUpdate,
    Export { site: String, account: String },
}

#[derive(Default)]
struct PasswordsApp {
    logins: Vec<Login>,
    listed: bool,
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

    /// The logins the list shows now, in order.
    fn shown(&self) -> Vec<Login> {
        model::filter(&self.logins, &self.query).into_iter().cloned().collect()
    }

    fn select(&mut self, site: String, account: String) {
        self.selected = Some((site, account));
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

    fn export(&mut self, ctx: &Ctx) {
        if self.logins.is_empty() {
            self.status = "No logins to export.".into();
            return;
        }
        self.export_rows.clear();
        self.export_left = self.logins.len();
        for l in self.logins.clone() {
            let id = passwords::fill(ctx, &l.site, &l.account);
            self.pending.insert(id, Pending::Export { site: l.site, account: l.account });
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
                    if y < HEADER_PT {
                        return false;
                    }
                    let index = ((y - HEADER_PT) / ROW_PT).floor() as usize;
                    if let Some(l) = self.shown().get(index).cloned() {
                        self.select(l.site, l.account);
                        return true;
                    }
                    return false;
                }
                let Some((site, account)) = self.selected.clone() else { return false };
                match detail_button(x, y) {
                    Some("Reveal") if self.revealed.is_some() => {
                        self.revealed = None;
                    }
                    Some("Reveal") => {
                            let id = passwords::fill(ctx, &site, &account);
                        self.pending.insert(id, Pending::Reveal);
                    }
                    Some("Copy") => match &self.revealed {
                        Some(p) => {
                            clipboard::set_text(p);
                            self.status = "Password copied.".into();
                        }
                        None => {
                            let id = passwords::fill(ctx, &site, &account);
                            self.pending.insert(id, Pending::Copy);
                        }
                    },
                    Some("Generate") => match model::generate() {
                        Ok(p) => {
                            let id = passwords::update(ctx, &site, &account, &p);
                            self.pending.insert(id, Pending::Generate);
                            self.revealed = Some(p);
                        }
                        Err(e) => self.status = format!("Could not generate: {e}"),
                    },
                    Some("Delete") => {
                        let id = passwords::remove(ctx, &site, &account);
                        self.pending.insert(id, Pending::Delete { site, account });
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
                    self.logins = rows.into_iter().map(|r| Login { site: r.service, account: r.account }).collect();
                    model::sort(&mut self.logins);
                }
            }
            Pending::Reveal => match passwords::decode_password(result) {
                Ok(p) => self.revealed = Some(p),
                Err(e) => self.status = format!("Could not show the password: {e}"),
            },
            Pending::Copy => match passwords::decode_password(result) {
                Ok(p) => {
                    clipboard::set_text(&p);
                    self.status = "Password copied.".into();
                }
                Err(e) => self.status = format!("Could not copy: {e}"),
            },
            Pending::Generate => match passwords::decode_password(result) {
                Ok(_) => self.status = "New password saved.".into(),
                Err(e) => self.status = format!("Could not save the new password: {e}"),
            },
            Pending::Delete { site, account } => match passwords::decode_password(result) {
                Ok(_) | Err(KeychainError::ItemNotFound) => {
                    self.logins.retain(|l| !(l.site == site && l.account == account));
                    if self.selected.as_ref().is_some_and(|(s, a)| *s == site && *a == account) {
                        self.selected = None;
                        self.revealed = None;
                    }
                    self.status = "Login deleted.".into();
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
            Pending::Export { site, account } => {
                match passwords::decode_password(result) {
                    Ok(password) => self.export_rows.push(Entry { site, account, password }),
                    Err(e) => self.status = format!("Could not read {site}: {e}"),
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
        let (w, h) = (s.w, s.h);
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());
        // list column with its bar and search field
        s.fill_rect(0, 0, p(LIST_PT), h, ui::SIDEBAR_BG.now());
        for (label, x) in [("Import", IMPORT_BTN.0), ("Export", EXPORT_BTN.0)] {
            let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, ui::LINK);
            s.blit_colored(&img, p(x + 4.0), p(BAR_Y_PT + 6.0), ui::LINK, 255);
        }
        let search = if self.query.is_empty() { "Search".to_string() } else { self.query.clone() };
        let search_colour = if self.query.is_empty() { ui::LABEL_2.now() } else { ui::LABEL.now() };
        s.fill_rect(p(PAD_PT), p(HEADER_PT - 36.0), p(LIST_PT - 2.0 * PAD_PT), p(26.0), ui::SEPARATOR);
        let img = ui.text(&ui.fit(&search, ui::MAC_BODY_PT, ui.pt(LIST_PT - 4.0 * PAD_PT)), ui::MAC_BODY_PT, Weight::Regular, search_colour);
        s.blit_colored(&img, p(PAD_PT + 8.0), p(HEADER_PT - 31.0), search_colour, 255);
        let shown = self.shown();
        for (i, l) in shown.iter().enumerate() {
            let y = HEADER_PT + i as f32 * ROW_PT;
            if y > h as f32 / scale {
                break;
            }
            let selected = self.selected.as_ref().is_some_and(|(site, acc)| *site == l.site && *acc == l.account);
            if selected {
                s.fill_rect(p(6.0), p(y), p(LIST_PT - 12.0), p(ROW_PT - 4.0), ui::SIDEBAR_SELECTED);
            }
            let img = ui.text(&ui.fit(&l.site, ui::MAC_BODY_PT, ui.pt(LIST_PT - 2.0 * PAD_PT)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, p(PAD_PT), p(y + 5.0), ui::LABEL.now(), 255);
            let img = ui.text(&ui.fit(&l.account, ui::MAC_FOOTNOTE_PT, ui.pt(LIST_PT - 2.0 * PAD_PT)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, p(PAD_PT), p(y + 24.0), ui::LABEL_2.now(), 255);
        }
        if shown.is_empty() {
            let text = if self.logins.is_empty() { "No Saved Passwords" } else { "No Matches" };
            let img = ui.text(text, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, p(PAD_PT), p(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
        }
        s.fill_rect(p(LIST_PT - 1.0), 0, 1, h, ui::SEPARATOR);
        // detail column
        let px = LIST_PT + PAD_PT;
        match &self.selected {
            Some((site, account)) => {
                for (name, bx, _) in DETAIL_BTNS {
                    let colour = match name {
                        "Delete" => ui::DESTRUCTIVE,
                        _ => ui::LINK,
                    };
                    let label = if name == "Reveal" && self.revealed.is_some() { "Hide" } else { name };
                    let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, colour);
                    s.blit_colored(&img, p(px + bx + 2.0), p(BAR_Y_PT + 6.0), colour, 255);
                }
                let width = w as f32 / scale - px - PAD_PT;
                let title = ui.text(&ui.fit(site, ui::MAC_TITLE2_PT, ui.pt(width)), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&title, p(px), p(HEADER_PT), ui::LABEL.now(), 255);
                let user = ui.text(&ui.fit(account, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&user, p(px), p(HEADER_PT + 36.0), ui::LABEL_2.now(), 255);
                let shown_pw = match &self.revealed {
                    Some(pw) => pw.clone(),
                    None => "••••••••••••".to_string(),
                };
                let pw = ui.text(&ui.fit(&shown_pw, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&pw, p(px), p(HEADER_PT + 72.0), ui::LABEL.now(), 255);
            }
            None => {
                let hint = ui.text("Select a login to see its password", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&hint, p(px), p(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
            }
        }
        if !self.status.is_empty() {
            let width = w as f32 / scale - PAD_PT * 2.0;
            let img = ui.text(&ui.fit(&self.status, ui::MAC_FOOTNOTE_PT, ui.pt(width)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, p(PAD_PT), p(h as f32 / scale - 24.0), ui::LABEL_2.now(), 255);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PasswordsApp::default());
}
