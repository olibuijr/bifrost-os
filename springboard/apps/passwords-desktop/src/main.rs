//! Passwords as an installed BifrOSt app in the full-size window: the items in the shared passwords group, split into
//! Logins, Notes and Certificates tabs, with a search field and a detail column. A login has Reveal or Hide, Copy,
//! Generate (a new strong password for the login) and Delete; a note or certificate has Reveal or Hide, Copy and
//! Delete. Import reads a CSV with site, account and password columns (Open With on a .csv file); Export writes every
//! login to `~/Downloads/passwords.csv` with mode 0600. Pure logic (search, kinds, CSV, generator) is in `model.rs`.
//!
//! File > New Password (Cmd+N) opens a sheet that saves a login.
//!
//! Secrets stay in the shell's keychain: the app reads one at a time, only when an item is revealed, copied,
//! exported or replaced, and it never writes a secret to the log or the status line. The shell asks for the passcode
//! before it shows a password in Settings; this app shows a revealed secret when the user asks for it.

mod model;

use bifrost_app::keychain::Error as KeychainError;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::item_sheet::{ItemSheet, Outcome};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::pasteboard::NativePasteboard;
use bifrost_app::passwords::{kind_of, name_of, Kind};
use bifrost_app::{keychain_groups, passwords, widgets, App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use model::{Entry, Item};
use std::collections::HashMap;
use std::path::PathBuf;

/// Header and detail geometry in points.
const LIST_PT: f32 = 300.0;
/// The kind tabs sit just under the unified bar's window-drag band, so every part of them takes a click.
const TABS_Y_PT: f32 = UNIFIED_BAR_PT + 4.0;
const TABS_H_PT: f32 = 24.0;
const SEARCH_Y_PT: f32 = TABS_Y_PT + TABS_H_PT + 8.0;
const SEARCH_H_PT: f32 = 26.0;
const HEADER_PT: f32 = 124.0;
const ROW_PT: f32 = 44.0;
const PAD_PT: f32 = 12.0;
/// Toolbar button ids: the list column's bar (Import, Export, New Password) and the detail column's bar. They are also
/// the accessibility ids of the controls published through `App::controls`, because the whole unified bar is a window-drag
/// zone and a press there reaches the app only on a published control.
const B_IMPORT: u32 = 1;
const B_EXPORT: u32 = 2;
const B_NEW: u32 = 3;
const B_REVEAL: u32 = 10;
const B_COPY: u32 = 11;
const B_GENERATE: u32 = 12;
const B_DELETE: u32 = 13;
/// The kind tabs under the bar, in order: one segmented control across the list column.
const TABS: [(Kind, &str); 3] = [(Kind::Login, "Logins"), (Kind::SecureNote, "Notes"), (Kind::Certificate, "Certificates")];

/// File > New Password.
const MENU_NEW: u32 = 1;

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
    Add,
}

#[derive(Default)]
struct PasswordsApp {
    items: Vec<Item>,
    listed: bool,
    tab: Kind,
    query: TextField,
    selected: Option<(String, String)>,
    revealed: Option<String>,
    pending: HashMap<u32, Pending>,
    export_rows: Vec<Entry>,
    export_left: usize,
    export_failure: Option<String>,
    imported: usize,
    import_failed: usize,
    status: String,
    /// The New Password sheet while it is open; it blocks the window.
    adding: Option<ItemSheet>,
}

impl PasswordsApp {
    /// Opens the New Password sheet.
    fn open_add(&mut self) -> bool {
        if self.adding.is_some() {
            return false;
        }
        self.adding = Some(ItemSheet::new("New Password", &[("Website", "example.com", false), ("User Name", "Account name", false), ("Password", "", true)]));
        true
    }

    /// Input while the New Password sheet is open: it takes every press and key.
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
                let id = passwords::save(ctx, &values[0], &values[1], &values[2]);
                self.pending.insert(id, Pending::Add);
                true
            }
        }
    }

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

    /// The list column's toolbar: Import, Export and the New Password button, right of the traffic lights.
    fn list_toolbar() -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(B_IMPORT, "Import"),
            ToolbarItem::button(B_EXPORT, "Export"),
            ToolbarItem::icon(B_NEW, "New Password", "plus"),
        ])
    }

    /// The detail column's toolbar: the actions on the selected item (none while nothing is selected).
    fn detail_toolbar(&self) -> Toolbar {
        let Some((service, _)) = &self.selected else { return Toolbar::default() };
        let mut items = vec![
            ToolbarItem::button(B_REVEAL, if self.revealed.is_some() { "Hide" } else { "Reveal" }),
            ToolbarItem::button(B_COPY, "Copy"),
        ];
        if kind_of(service) == Kind::Login {
            items.push(ToolbarItem::button(B_GENERATE, "Generate"));
        }
        items.push(ToolbarItem::button(B_DELETE, "Delete"));
        Toolbar::new(items)
    }

    /// Both toolbars with where they sit, for a window `w_pt` wide: the unified bar's list-column half and detail half.
    fn toolbars(&self, w_pt: f32) -> [(Toolbar, Rect); 2] {
        [(Self::list_toolbar(), list_bar()), (self.detail_toolbar(), detail_bar(w_pt))]
    }

    /// The toolbar button under `(x, y)` (points), if any.
    fn button_at(&self, ui: &Ui, w_pt: f32, x: f32, y: f32) -> Option<u32> {
        self.toolbars(w_pt).iter().find_map(|(bar, bounds)| widgets::toolbar_layout(ui, *bounds, bar).hit(x, y))
    }

    /// The toolbar buttons as published controls, in window points: the shell leaves a press on them to the app.
    fn control_nodes(&self, ui: &Ui, w_pt: f32) -> Vec<Node> {
        let mut nodes = Vec::new();
        for (bar, bounds) in self.toolbars(w_pt) {
            for (id, bounds) in widgets::toolbar_layout(ui, bounds, &bar).visible {
                let Some(item) = bar.items.iter().find(|i| i.id == id) else { continue };
                nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds, enabled: item.enabled });
            }
        }
        nodes
    }

    /// A toolbar button was pressed (a click, or the published control through accessibility).
    fn press_button(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            B_IMPORT => {
                self.status = "Open a .csv file with Passwords to import it.".into();
                return true;
            }
            B_EXPORT => {
                self.export(ctx);
                return true;
            }
            B_NEW => return self.open_add(),
            _ => {}
        }
        let Some((service, account)) = self.selected.clone() else { return false };
        let is_login = kind_of(&service) == Kind::Login;
        match id {
            B_REVEAL if self.revealed.is_some() => {
                self.revealed = None;
            }
            B_REVEAL => {
                let id = passwords::fill(ctx, &service, &account);
                self.pending.insert(id, Pending::Reveal);
            }
            B_COPY => match &self.revealed {
                Some(p) => {
                    copy_secret(ctx, p);
                    self.status = "Copied.".into();
                }
                None => {
                    let id = passwords::fill(ctx, &service, &account);
                    self.pending.insert(id, Pending::Copy);
                }
            },
            B_GENERATE if is_login => match model::generate() {
                Ok(p) => {
                    let id = passwords::update(ctx, &service, &account, &p);
                    self.pending.insert(id, Pending::Generate);
                    self.revealed = Some(p);
                }
                Err(e) => self.status = format!("Could not generate: {e}"),
            },
            B_DELETE => {
                let id = passwords::remove(ctx, &service, &account);
                self.pending.insert(id, Pending::Delete { service, account });
            }
            _ => return false,
        }
        true
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
        if self.pending.values().any(|p| matches!(p, Pending::Import { .. } | Pending::ImportUpdate)) {
            self.status = "An import is already in progress.".into();
            return;
        }
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| model::parse_import(&t)) {
            Ok(got) => {
                self.imported = 0;
                self.import_failed = 0;
                let count = got.entries.len();
                if count == 0 {
                    self.status = format!("No logins to import{}.", skipped_text(got.skipped));
                    return;
                }
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
        if self.export_left > 0 {
            self.status = "An export is already in progress.".into();
            return;
        }
        let logins: Vec<Item> = self.items.iter().filter(|i| i.kind == Kind::Login).cloned().collect();
        if logins.is_empty() {
            self.status = "No logins to export.".into();
            return;
        }
        self.export_rows.clear();
        self.export_failure = None;
        self.export_left = logins.len();
        for l in logins {
            let id = passwords::fill(ctx, &l.service, &l.account);
            self.pending.insert(id, Pending::Export { service: l.service, account: l.account });
        }
        self.status = "Exporting logins.".into();
    }

    /// Writes the export once every password has come back; a failed write is shown in the status line.
    fn finish_export(&mut self) {
        let text = model::render_export(&self.export_rows);
        let rows = self.export_rows.len();
        self.status = if let Some(reason) = self.export_failure.take() {
            format!("Could not export; {reason}. No file was changed.")
        } else {
            match std::env::var_os("HOME") {
                Some(home) => match write_private(&PathBuf::from(home).join("Downloads/passwords.csv"), text.as_bytes()) {
                    Ok(()) => format!("Exported {rows} logins to Downloads."),
                    Err(e) => format!("Could not export: {e}"),
                },
                None => "Could not export: HOME is unavailable.".into(),
            }
        };
        self.export_rows.clear();
    }
}

/// Puts a password on the pasteboard as a concealed copy: the shell keeps it in memory only, hands it to the app in front
/// and not to apps in the background, marks it for other programs so clipboard managers leave it out, and clears it after
/// 45 seconds when nothing else was copied. The password is never put on the plain-text clipboard (that copy would be kept
/// on disk and shown to every app).
fn copy_secret(ctx: &Ctx, password: &str) {
    NativePasteboard::general().write_concealed_string(ctx, password);
}

/// Seconds since the Unix epoch, for one-time codes (0 if the clock is before the epoch).
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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

/// Atomically replaces the clear-text export with an owner-only file. A sibling temporary file means interruption
/// cannot truncate the previous export or leave a partially written CSV at the published path.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let tmp = dir.join(format!(".passwords-{}-{}.tmp", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    let written = (|| {
        f.write_all(data)?;
        f.sync_all()
    })();
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // Directory sync is best effort because some supported filesystems reject it.
    let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    Ok(())
}

/// The segmented control's bounds in the list column, in points.
fn tabs_bounds() -> Rect {
    Rect { x: PAD_PT, y: TABS_Y_PT, w: LIST_PT - 2.0 * PAD_PT, h: TABS_H_PT }
}

/// The search field's bounds in the list column, in points.
fn search_bounds() -> Rect {
    Rect { x: PAD_PT, y: SEARCH_Y_PT, w: LIST_PT - 2.0 * PAD_PT, h: SEARCH_H_PT }
}

/// The kind tab at point `(x, y)` in the list column, if any.
fn tab_at(x: f32, y: f32) -> Option<Kind> {
    bifrost_app::controls::sheet::segment_hit(tabs_bounds(), TABS.len(), x, y).map(|i| TABS[i].0)
}

/// The unified bar over the list column, in window points; the traffic lights sit at its left end.
fn list_bar() -> Rect {
    Rect { x: 0.0, y: 0.0, w: LIST_PT, h: UNIFIED_BAR_PT }
}

/// The unified bar over the detail column of a window `w_pt` wide, in window points.
fn detail_bar(w_pt: f32) -> Rect {
    Rect { x: LIST_PT, y: 0.0, w: (w_pt - LIST_PT).max(0.0), h: UNIFIED_BAR_PT }
}

impl App for PasswordsApp {
    fn id(&self) -> &str {
        "is.olibuijr.passwords.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // Keychain edits persist as they are made, and CSV export replaces its destination atomically.
        ctx.enable_sudden_termination();
        if !self.listed {
            self.listed = true;
            self.ask_list(ctx);
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        vec![AppMenu {
            title: "File".into(),
            items: vec![AppMenuItem::new(MENU_NEW, "New Password\u{2026}").key("cmd+n").enabled(self.adding.is_none())],
        }]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        id == MENU_NEW && self.open_add()
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        // a file opened from outside still imports behind the sheet
        if self.adding.is_some() && !matches!(ev, Input::Open(_)) {
            return self.adding_event(ctx, ev);
        }
        match ev {
            // a published toolbar control pressed (accessibility, `bifrost pc midget tap-window`)
            Input::Activate(id) => u32::try_from(id).is_ok_and(|id| self.press_button(ctx, id)),
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                let w_pt = ctx.cfg.w as f32 / ctx.ui.s;
                if y < UNIFIED_BAR_PT {
                    return match self.button_at(&ctx.ui, w_pt, x, y) {
                        Some(id) => self.press_button(ctx, id),
                        None => false,
                    };
                }
                if x >= LIST_PT {
                    return false;
                }
                if let Some(kind) = tab_at(x, y) {
                    self.set_tab(kind);
                    return true;
                }
                if bifrost_app::controls::search::hits_clear(search_bounds(), !self.query.is_empty(), x, y) {
                    self.query.clear();
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
                false
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
            Input::Key(KeyInput::Code(1 | 27)) => {
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

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.control_nodes(&ctx.ui, ctx.cfg.w as f32 / ctx.ui.s)
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
                    copy_secret(ctx, &p);
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
                Err(e) => {
                    self.import_failed += 1;
                    self.status = format!("Could not import {}: {e}", entry.site);
                }
            },
            Pending::ImportUpdate => match passwords::decode_password(result) {
                Ok(_) => self.imported += 1,
                Err(e) => {
                    self.import_failed += 1;
                    self.status = format!("Could not update a login: {e}");
                }
            },
            Pending::Add => match passwords::decode_password(result) {
                Ok(_) => {
                    self.set_tab(Kind::Login);
                    self.status = "Password added.".into();
                    self.ask_list(ctx);
                }
                Err(KeychainError::DuplicateItem) => self.status = "A login for that website and user name already exists.".into(),
                Err(e) => self.status = format!("Could not add the password: {e}"),
            },
            Pending::Export { service, account } => {
                match passwords::decode_password(result) {
                    Ok(password) => self.export_rows.push(Entry { site: service, account, password }),
                    Err(e) => self.export_failure = Some(format!("could not read {service}: {e}")),
                }
                self.export_left = self.export_left.saturating_sub(1);
                if self.export_left == 0 {
                    self.finish_export();
                }
                return true;
            }
        }
        if (self.imported > 0 || self.import_failed > 0)
            && self.pending.values().all(|p| !matches!(p, Pending::Import { .. } | Pending::ImportUpdate))
        {
            self.status = if self.import_failed == 0 {
                format!("Imported {} logins.", self.imported)
            } else {
                format!("Imported {}; {} failed. Keychain changes are not transactional.", self.imported, self.import_failed)
            };
            if self.imported > 0 {
                self.ask_list(ctx);
            }
            self.imported = 0;
            self.import_failed = 0;
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
        s.fill_rect(0, 0, w, h, ui::CONTENT_BG.now());
        // list column with its bar, kind tabs and search field
        s.fill_rect(0, 0, p(LIST_PT), h, ui::SIDEBAR_BG.now());
        // the unified bar: the detail half is a toolbar band, the list half is the sidebar; both clear the traffic lights
        widgets::toolbar_band(s, ui, detail_bar(w as f32 / scale));
        for (bar, bounds) in self.toolbars(w as f32 / scale) {
            widgets::toolbar(s, ui, bounds, &bar, None);
        }
        let labels: Vec<&str> = TABS.iter().map(|(_, label)| *label).collect();
        let selected = TABS.iter().position(|(kind, _)| *kind == self.tab).unwrap_or(0);
        widgets::segmented(s, ui, tabs_bounds(), &labels, selected, true);
        widgets::search_field(s, ui, &self.query, search_bounds(), "Search", true);
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
                let width = w as f32 / scale - px - PAD_PT;
                let title = ui.text(&ui.fit(name_of(service), ui::MAC_TITLE2_PT, ui.pt(width)), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&title, pi(px), pi(HEADER_PT), ui::LABEL.now(), 255);
                let sub = if kind == Kind::Login { account.as_str() } else { kind_label(kind) };
                let user = ui.text(&ui.fit(sub, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&user, pi(px), pi(HEADER_PT + 36.0), ui::LABEL_2.now(), 255);
                // a note or certificate can be several lines: the detail shows its first line
                // a login whose secret is an otpauth URI shows its current one-time code and the seconds it has left
                let code = match &self.revealed {
                    Some(v) if kind == Kind::Login => model::verification_code(v, unix_now()),
                    _ => None,
                };
                let shown_secret = match (&code, &self.revealed) {
                    (Some((digits, _)), _) => digits.clone(),
                    (None, Some(v)) if kind == Kind::Login => v.clone(),
                    (None, Some(v)) => v.lines().next().unwrap_or("").to_string(),
                    (None, None) => "••••••••••••".to_string(),
                };
                let pw = ui.text(&ui.fit(&shown_secret, ui::MAC_BODY_PT, ui.pt(width)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&pw, pi(px), pi(HEADER_PT + 72.0), ui::LABEL.now(), 255);
                if let Some((_, left)) = &code {
                    let note = ui.text(&format!("Verification code, changes in {left} s"), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
                    s.blit_colored(&note, pi(px), pi(HEADER_PT + 100.0), ui::LABEL_2.now(), 255);
                }
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
        if let Some(sheet) = &self.adding {
            sheet.draw(s, ui, Rect { x: 0.0, y: 0.0, w: w as f32 / scale, h: h as f32 / scale }, UNIFIED_BAR_PT);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PasswordsApp::default());
}

#[cfg(test)]
mod private_export_tests {
    use super::write_private;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn replaces_export_atomically_with_owner_only_permissions() {
        let dir = std::env::temp_dir().join(format!("bifrost-passwords-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("passwords.csv");
        std::fs::write(&path, b"old,complete,export\n").unwrap();

        write_private(&path, b"new,complete,export\n").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new,complete,export\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary export remains");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn tabs_sit_below_the_drag_band_and_hit_by_segment() {
        let b = tabs_bounds();
        assert!(b.y >= UNIFIED_BAR_PT, "the tabs must not sit in the window-drag band");
        let third = b.w / 3.0;
        let mid_y = b.y + b.h / 2.0;
        assert_eq!(tab_at(b.x + third * 0.5, mid_y), Some(Kind::Login));
        assert_eq!(tab_at(b.x + third * 1.5, mid_y), Some(Kind::SecureNote));
        assert_eq!(tab_at(b.x + third * 2.5, mid_y), Some(Kind::Certificate));
        assert_eq!(tab_at(b.x + third * 0.5, b.y - 2.0), None);
    }

    #[test]
    fn search_field_sits_between_the_tabs_and_the_list() {
        let (t, s) = (tabs_bounds(), search_bounds());
        assert!(s.y >= t.y + t.h && s.y + s.h <= HEADER_PT);
    }

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 1.0)
    }

    fn app_with_selection() -> PasswordsApp {
        PasswordsApp { selected: Some(("example.com".into(), "me".into())), ..Default::default() }
    }

    fn lights_trailing() -> f32 {
        bifrost_app::layout::traffic_lights_trailing()
    }

    #[test]
    fn list_bar_buttons_clear_the_traffic_lights() {
        let ui = ui();
        let layout = widgets::toolbar_layout(&ui, list_bar(), &PasswordsApp::list_toolbar());
        let ids: Vec<u32> = layout.visible.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![B_IMPORT, B_EXPORT, B_NEW], "Import, Export and New Password all fit in the list column");
        assert!(layout.overflow.is_empty());
        for (id, r) in &layout.visible {
            assert!(r.x >= lights_trailing(), "button {id} starts at {} under the traffic lights (ends at {})", r.x, lights_trailing());
            assert!(r.x + r.w <= LIST_PT, "button {id} fits in the list column");
            assert!(r.y >= 0.0 && r.y + r.h <= UNIFIED_BAR_PT, "button {id} sits inside the unified bar");
        }
        // the buttons do not overlap each other
        for (i, (_, a)) in layout.visible.iter().enumerate() {
            for (_, b) in layout.visible.iter().skip(i + 1) {
                assert!(a.x + a.w <= b.x || b.x + b.w <= a.x);
            }
        }
    }

    #[test]
    fn detail_bar_buttons_sit_in_the_detail_column_inside_the_bar() {
        let ui = ui();
        let app = app_with_selection();
        let layout = widgets::toolbar_layout(&ui, detail_bar(1200.0), &app.detail_toolbar());
        let ids: Vec<u32> = layout.visible.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![B_REVEAL, B_COPY, B_GENERATE, B_DELETE]);
        for (_, r) in &layout.visible {
            assert!(r.x >= LIST_PT && r.y + r.h <= UNIFIED_BAR_PT);
        }
        // nothing selected: no detail buttons
        assert!(PasswordsApp::default().detail_toolbar().items.is_empty());
    }

    #[test]
    fn every_bar_button_is_published_and_hits_where_published() {
        let ui = ui();
        let app = app_with_selection();
        let nodes = app.control_nodes(&ui, 1200.0);
        for id in [B_IMPORT, B_EXPORT, B_NEW, B_REVEAL, B_COPY, B_DELETE] {
            let node = nodes.iter().find(|n| n.id == id as u64).unwrap_or_else(|| panic!("button {id} is published"));
            assert_eq!(node.role, Role::Button);
            assert!(!node.label.is_empty() && node.enabled);
            assert!(node.bounds.y + node.bounds.h <= UNIFIED_BAR_PT, "in the band the shell moves the window unless the control is published");
            let (cx, cy) = (node.bounds.x + node.bounds.w / 2.0, node.bounds.y + node.bounds.h / 2.0);
            assert_eq!(app.button_at(&ui, 1200.0, cx, cy), Some(id));
        }
        // the traffic lights themselves hit nothing
        assert_eq!(app.button_at(&ui, 1200.0, 20.0, 20.0), None);
        // no selection: the detail buttons are not published
        let bare = PasswordsApp::default().control_nodes(&ui, 1200.0);
        assert!(bare.iter().all(|n| n.id == B_IMPORT as u64 || n.id == B_EXPORT as u64 || n.id == B_NEW as u64));
    }

    #[test]
    fn new_password_button_opens_the_sheet() {
        let mut app = PasswordsApp::default();
        assert!(app.open_add());
        assert!(app.adding.is_some());
        // a second press while the sheet is open does nothing
        assert!(!app.open_add());
    }
}
