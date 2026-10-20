//! The open and save panel (`NativeOpenPanel` / `NativeSavePanel`), phase 3 of `docs/file-portal-design.md`.
//!
//! An app asks with `ToShell::DocOpen` or `DocSave`; the shell shows this panel over the desktop (it is a desktop popup,
//! so it sits on the menu layer above every window). The app never sees the listing. When the user chooses, the broker
//! (`bare::docbroker`) opens the file as the account and the app receives one descriptor (`ToApp::DocGranted`).
//!
//! Behaviour follows the Golden Gate panels: it starts in Documents, lists names alphabetically (case-insensitive), files the
//! filters do not allow are shown dimmed and cannot be chosen, Return or a double click is the default button, Esc
//! cancels, Left goes up a folder. In a save panel the name field takes typing and a click on a file copies its name.
//!
//! `NativeOpenPanel` options (`bifrost_proto::docopt`): several files (`allowsMultipleSelection`; a click toggles a row),
//! folders (`canChooseDirectories`, `canChooseFiles`) and New Folder (`canCreateDirectories`, always on in a save
//! panel). The sidebar lists Recents (the app's recent grants, newest first), Home and the common folders that exist.
//! The folder last used by the app is where its next panel starts.

use crate::bare::docbroker::{self, valid_name, Grants, Opener, PendingSave};
use bifrost_proto::{DocErrorCode, DocMode, GrantId, ToApp, ToShell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open(DocMode),
    Save,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// the filters allow it (folders always); a file is dimmed when the panel cannot choose files
    pub enabled: bool,
    /// the path in the home when it is not `dir/name` (the Recents list)
    pub path: Option<PathBuf>,
}

/// What an open panel may choose (`NativeOpenPanel`). The default is the classic panel: one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub multiple: bool,
    pub files: bool,
    pub dirs: bool,
    pub create_dirs: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { multiple: false, files: true, dirs: false, create_dirs: false }
    }
}

impl Options {
    /// From the wire bits (`docopt`). A panel that can choose nothing chooses files.
    pub fn from_bits(bits: u32) -> Self {
        use bifrost_proto::docopt::*;
        let dirs = bits & DIRECTORIES != 0;
        Options { multiple: bits & MULTIPLE != 0, files: bits & FILES != 0 || !dirs, dirs, create_dirs: bits & CREATE_DIRS != 0 }
    }
}

/// A sidebar row: Recents (`path` is `None`) or a folder in the home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: Option<PathBuf>,
}

/// Recents shows at most this many documents.
pub const MAX_RECENTS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open(PathBuf),
    /// several files or folders, in the list's order (a panel with `allowsMultipleSelection`, even for one)
    OpenMany(Vec<PathBuf>),
    Save { dir: PathBuf, name: String },
    Cancel,
}

/// Rows the list shows at once; it scrolls to keep the selection in view.
pub const VISIBLE_ROWS: usize = 10;
/// Clicks on the same row closer than this are a double click (ms).
const DOUBLE_CLICK_MS: f64 = 400.0;

/// How much the window under a panel is dimmed (the alpha of black): 28%, as the kit's sheets dim their parent.
const SHEET_DIM_ALPHA: u8 = 71;

/// Panel geometry in points, relative to the panel's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub title: R,
    /// the "Save As:" field (save panels only)
    pub field: Option<R>,
    pub sidebar: R,
    pub sidebar_row_h: f32,
    pub new_folder: R,
    pub back: R,
    pub location: R,
    pub list: R,
    pub row_h: f32,
    pub cancel: R,
    pub ok: R,
}

pub struct Panel {
    pub slot: u8,
    pub request: u32,
    pub kind: Kind,
    pub title: String,
    filters: Vec<String>,
    /// the folder shown, relative to the home ("" is the home)
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    /// every chosen row (`selected` is the one with focus); one row unless the panel allows several
    pub marked: BTreeSet<usize>,
    pub opts: Options,
    /// the package extensions the requesting app declared (`package_types`, without the dot): a folder with one
    /// of them is a file row, chosen whole and never opened
    packages: Vec<String>,
    pub places: Vec<Place>,
    /// Recents is shown instead of a folder
    pub in_recents: bool,
    recents: Vec<PathBuf>,
    /// the name being typed for a new folder
    pub creating: Option<String>,
    /// the save panel's file name
    pub name: String,
    /// the part of the name that is selected (byte offsets): the base name without its extension when the panel opens or a
    /// file is chosen, so typing replaces it and keeps the extension. Cleared by the first edit.
    pub name_sel: Option<std::ops::Range<usize>>,
    /// first row shown
    pub top: usize,
    last_click: Option<(usize, f64)>,
}

impl Panel {
    pub fn new(slot: u8, request: u32, kind: Kind, title: &str, filters: Vec<String>, suggested: &str, opener: &Opener) -> Self {
        let title = if !title.is_empty() {
            title.to_string()
        } else if kind == Kind::Save {
            "Save".into()
        } else {
            "Open".into()
        };
        let mut p = Panel {
            slot,
            request,
            kind,
            title,
            filters,
            dir: PathBuf::new(),
            entries: Vec::new(),
            selected: None,
            marked: BTreeSet::new(),
            opts: Options::default(),
            packages: Vec::new(),
            places: Vec::new(),
            in_recents: false,
            recents: Vec::new(),
            creating: None,
            name: if kind == Kind::Save { suggested.chars().filter(|c| *c != '/' && !c.is_control()).take(255).collect() } else { String::new() },
            name_sel: None,
            top: 0,
            last_click: None,
        };
        // Golden Gate starts in the last folder, else Documents (the shell moves the panel to the app's last folder after this)
        if opener.list(Path::new("Documents")).is_ok() {
            p.dir = "Documents".into();
        }
        p.places = Self::places_in(opener);
        p.load(opener);
        p.select_base_name();
        p
    }

    /// Selects the base name in the name field (everything before the last dot, or the whole name when there is no
    /// extension), as a save panel opens on it: typing replaces the name and keeps the extension.
    pub fn select_base_name(&mut self) {
        if self.kind != Kind::Save || self.name.is_empty() {
            self.name_sel = None;
            return;
        }
        let end = self.name.rfind('.').filter(|i| *i > 0).unwrap_or(self.name.len());
        self.name_sel = Some(0..end);
    }

    /// Recents, Home and the common folders that exist.
    fn places_in(opener: &Opener) -> Vec<Place> {
        let mut v = vec![Place { label: "Recents".into(), path: None }, Place { label: "Home".into(), path: Some(PathBuf::new()) }];
        let top = opener.list(Path::new("")).unwrap_or_default();
        for name in ["Desktop", "Documents", "Downloads"] {
            if top.iter().any(|(n, d)| *d && n == name) {
                v.push(Place { label: name.into(), path: Some(name.into()) });
            }
        }
        v
    }

    /// What the panel may choose (`allowsMultipleSelection`, `canChooseDirectories`, ...).
    pub fn with_options(mut self, opts: Options, opener: &Opener) -> Self {
        self.opts = opts;
        self.load(opener);
        self
    }

    /// The package types the requesting app declared (`package_types`, extensions without the dot). A folder of one of
    /// those types is a file row: it can be chosen (or replaced by a save) but not navigated.
    pub fn with_packages(mut self, types: Vec<String>, opener: &Opener) -> Self {
        self.packages = types;
        self.load(opener);
        self
    }

    /// Whether `name` is a folder of a package type the app declared.
    fn is_declared_package(&self, name: &str) -> bool {
        let Some((_, ext)) = name.rsplit_once('.') else { return false };
        self.packages.iter().any(|p| p.eq_ignore_ascii_case(ext)) && bifrost_ui::uttype::is_package_name(name)
    }

    /// One row of the listing. A declared package is a file (dimmed only when the panel chooses no files); any other
    /// folder stays a folder.
    fn row(&self, name: String, dir: bool, path: Option<PathBuf>) -> Entry {
        let package = dir && self.is_declared_package(&name);
        let enabled = if package { self.opts.files } else { dir || (self.opts.files && self.wanted(&name)) };
        Entry { name, dir: dir && !package, enabled, path }
    }

    /// The app's recent documents (paths in the home, newest first) for the Recents place.
    pub fn with_recents(mut self, recents: Vec<PathBuf>, opener: &Opener) -> Self {
        self.recents = recents.into_iter().take(MAX_RECENTS).collect();
        self.load(opener);
        self
    }

    /// Shows folder `dir` when it can be read (the folder the app used last).
    pub fn go_to(&mut self, dir: &Path, opener: &Opener) -> bool {
        if opener.list(dir).is_err() {
            return false;
        }
        self.dir = dir.to_path_buf();
        self.in_recents = false;
        self.load(opener);
        true
    }

    /// The sidebar row that matches what is shown.
    pub fn active_place(&self) -> Option<usize> {
        self.places.iter().position(|pl| if self.in_recents { pl.path.is_none() } else { pl.path.as_deref() == Some(self.dir.as_path()) })
    }

    pub fn go_place(&mut self, i: usize, opener: &Opener) {
        let Some(pl) = self.places.get(i).cloned() else { return };
        self.creating = None;
        match pl.path {
            None => self.in_recents = true,
            Some(path) => {
                self.in_recents = false;
                self.dir = path;
            }
        }
        self.load(opener);
    }

    /// The filters allow `name`: an extension equal to a filter, or a type that conforms to a filter UTI.
    fn wanted(&self, name: &str) -> bool {
        if self.filters.is_empty() {
            return true;
        }
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let ty = bifrost_ui::uttype::type_for_extension(name);
        self.filters.iter().any(|f| ext.as_deref() == Some(f.to_ascii_lowercase().as_str()) || ty.is_some_and(|t| bifrost_ui::uttype::conforms(t, f)))
    }

    /// Reads the folder shown. A folder that cannot be read shows empty (the panel stays usable: Back still works).
    pub fn load(&mut self, opener: &Opener) {
        let entries: Vec<Entry> = if self.in_recents {
            // newest first, as recorded; documents that are gone are left out
            self.recents
                .iter()
                .filter_map(|path| {
                    let f = opener.open_any(path, DocMode::Read).ok()?;
                    let dir = f.metadata().ok()?.is_dir();
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    Some(self.row(name, dir, Some(path.clone())))
                })
                .collect()
        } else {
            let mut v: Vec<Entry> = opener
                .list(&self.dir)
                .unwrap_or_default()
                .into_iter()
                .filter(|(n, _)| !n.starts_with('.'))
                .map(|(name, dir)| self.row(name, dir, None))
                .collect();
            v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
            v
        };
        self.entries = entries;
        self.marked.clear();
        self.selected = None;
        self.top = 0;
        self.last_click = None;
    }

    /// The folder's name for the location bar ("Home" at the top).
    pub fn location(&self) -> String {
        if self.in_recents {
            return "Recents".into();
        }
        self.dir.file_name().map_or_else(|| "Home".into(), |n| n.to_string_lossy().into_owned())
    }

    pub fn can_go_up(&self) -> bool {
        !self.in_recents && !self.dir.as_os_str().is_empty()
    }

    /// Up one folder, with the folder we left selected (as Files and the panels do).
    pub fn up(&mut self, opener: &Opener) {
        if self.in_recents {
            return;
        }
        let Some(left) = self.dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
        self.dir.pop();
        self.load(opener);
        if let Some(i) = self.entries.iter().position(|e| e.name == left) {
            self.select(i);
        }
    }

    fn enter(&mut self, i: usize, opener: &Opener) {
        if let Some(e) = self.entries.get(i).filter(|e| e.dir) {
            self.dir = self.path_of(e);
            self.in_recents = false;
            self.load(opener);
        }
    }

    /// The path of `e` in the home.
    fn path_of(&self, e: &Entry) -> PathBuf {
        e.path.clone().unwrap_or_else(|| self.dir.join(&e.name))
    }

    /// The panel can choose this row (a folder only when `canChooseDirectories`).
    fn chooseable(&self, e: &Entry) -> bool {
        if e.dir {
            self.opts.dirs
        } else {
            e.enabled
        }
    }

    /// Row indices that are chosen, in list order.
    pub fn chosen(&self) -> Vec<usize> {
        self.marked.iter().copied().filter(|i| *i < self.entries.len()).collect()
    }

    /// With `allowsMultipleSelection` a click adds the row to the selection or takes it out (no modifier keys reach the
    /// panel yet); otherwise it is `select`.
    pub fn toggle(&mut self, i: usize) {
        if !self.opts.multiple || i >= self.entries.len() || self.kind == Kind::Save {
            return self.select(i);
        }
        if !self.marked.remove(&i) {
            self.marked.insert(i);
        }
        self.selected = Some(i);
    }

    /// Selects every row the panel can choose (multiple selection only).
    pub fn select_all(&mut self) {
        if self.opts.multiple && self.kind != Kind::Save {
            self.marked = (0..self.entries.len()).filter(|i| self.chooseable(&self.entries[*i])).collect();
        }
    }

    /// New Folder is offered in a save panel and in an open panel that asks for it; not inside Recents.
    pub fn can_create_dir(&self) -> bool {
        (self.kind == Kind::Save || self.opts.create_dirs) && !self.in_recents
    }

    /// Starts typing the new folder's name (`untitled folder`, as Files does).
    pub fn begin_new_folder(&mut self) -> bool {
        if !self.can_create_dir() || self.creating.is_some() {
            return false;
        }
        self.creating = Some("untitled folder".into());
        true
    }

    /// Makes the folder named in the field and selects it. The field stays open when the name is refused (it exists).
    pub fn commit_new_folder(&mut self, opener: &Opener) -> bool {
        let Some(name) = self.creating.clone() else { return false };
        if opener.make_dir(&self.dir, &name).is_err() {
            return false;
        }
        self.creating = None;
        self.load(opener);
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.select(i);
        }
        true
    }

    /// Selects row `i` and scrolls it into view. In a save panel a file's name goes into the field.
    pub fn select(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        self.selected = Some(i);
        self.marked = BTreeSet::from([i]);
        if i < self.top {
            self.top = i;
        } else if i >= self.top + VISIBLE_ROWS {
            self.top = i + 1 - VISIBLE_ROWS;
        }
        if self.kind == Kind::Save && !self.entries[i].dir {
            self.name = self.entries[i].name.clone();
            self.select_base_name();
        }
    }

    /// Up/Down: the next row, stopping at the ends.
    pub fn step(&mut self, by: i32) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as i32 - 1;
        let to = match self.selected {
            None if by > 0 => 0,
            None => last,
            Some(i) => (i as i32 + by).clamp(0, last),
        };
        self.select(to as usize);
    }

    /// Whether the default button (Open or Save) would do something now.
    pub fn can_confirm(&self) -> bool {
        if self.creating.is_some() {
            return true;
        }
        match self.kind {
            Kind::Save => self.selected.and_then(|i| self.entries.get(i)).is_some_and(|e| e.dir) || valid_name(&self.name),
            Kind::Open(_) => {
                let rows: Vec<&Entry> = self.chosen().into_iter().map(|i| &self.entries[i]).collect();
                match rows[..] {
                    // nothing selected: a panel that can choose folders chooses the one shown
                    [] => self.opts.dirs && !self.in_recents,
                    // a single folder the panel cannot choose is opened instead
                    [e] if e.dir => true,
                    _ => rows.iter().all(|e| self.chooseable(e)),
                }
            }
        }
    }

    /// The default button, Return or the Open button: a folder the panel cannot choose opens; otherwise the panel finishes.
    pub fn confirm(&mut self, opener: &Opener) -> Option<Outcome> {
        if self.creating.is_some() {
            self.commit_new_folder(opener);
            return None;
        }
        if !self.can_confirm() {
            return None;
        }
        let sel = self.selected.and_then(|i| self.entries.get(i)).cloned();
        match (self.kind, sel) {
            (Kind::Save, Some(e)) if e.dir => {
                self.enter(self.selected.unwrap(), opener);
                None
            }
            (Kind::Save, _) => Some(Outcome::Save { dir: self.dir.clone(), name: self.name.clone() }),
            (Kind::Open(_), _) => {
                let rows = self.chosen();
                if rows.is_empty() {
                    let dir = self.dir.clone();
                    return Some(if self.opts.multiple { Outcome::OpenMany(vec![dir]) } else { Outcome::Open(dir) });
                }
                if rows.len() == 1 && self.entries[rows[0]].dir && !self.opts.dirs {
                    self.enter(rows[0], opener);
                    return None;
                }
                let mut paths: Vec<PathBuf> = rows.iter().map(|i| self.path_of(&self.entries[*i])).collect();
                Some(if self.opts.multiple { Outcome::OpenMany(paths) } else { Outcome::Open(paths.remove(0)) })
            }
        }
    }

    /// A double click on row `i`: a folder opens, a file is chosen.
    fn activate(&mut self, i: usize, opener: &Opener) -> Option<Outcome> {
        if self.entries.get(i).is_some_and(|e| e.dir) {
            self.enter(i, opener);
            return None;
        }
        if !self.marked.contains(&i) {
            self.toggle(i);
        }
        self.confirm(opener)
    }

    /// Typing into the save panel's name field (`/` and control characters are dropped; at most 255 bytes).
    pub fn type_text(&mut self, text: &str) -> bool {
        if let Some(n) = self.creating.as_mut() {
            for c in text.chars().filter(|c| *c != '/' && !c.is_control()) {
                if n.len() + c.len_utf8() <= 255 {
                    n.push(c);
                }
            }
            return true;
        }
        if self.kind != Kind::Save {
            return false;
        }
        let typed: Vec<char> = text.chars().filter(|c| *c != '/' && !c.is_control()).collect();
        if typed.is_empty() {
            return true;
        }
        // typing over a selection replaces it; otherwise it goes in at the end
        let mut at = self.name.len();
        if let Some(sel) = self.name_sel.take() {
            let sel = sel.start.min(self.name.len())..sel.end.min(self.name.len());
            at = sel.start;
            self.name.replace_range(sel, "");
        }
        for c in typed {
            if self.name.len() + c.len_utf8() <= 255 {
                self.name.insert(at, c);
                at += c.len_utf8();
            }
        }
        true
    }

    pub fn backspace(&mut self) -> bool {
        if let Some(n) = self.creating.as_mut() {
            n.pop();
            return true;
        }
        if self.kind != Kind::Save {
            return false;
        }
        // Backspace over a selection deletes it and keeps the rest (the extension)
        if let Some(sel) = self.name_sel.take() {
            let sel = sel.start.min(self.name.len())..sel.end.min(self.name.len());
            let had = !sel.is_empty();
            self.name.replace_range(sel, "");
            return had;
        }
        self.name.pop().is_some()
    }

    /// The panel's geometry in points. Width and row count are fixed tokens; nothing depends on the screen size
    /// except where the shell centres it.
    pub fn layout(&self) -> Layout {
        use super::ui::{SPACE_M, SPACE_S, SPACE_XL};
        let (w, row_h, bar_h, button_w, side_w) = (720.0, 28.0, 28.0, 96.0, 150.0);
        let pad = SPACE_XL;
        let title = R { x: pad, y: pad, w: w - 2.0 * pad, h: 20.0 };
        let mut y = title.y + title.h + SPACE_M;
        let field = (self.kind == Kind::Save).then(|| {
            let f = R { x: pad + 72.0, y, w: w - 2.0 * pad - 72.0, h: bar_h };
            y += bar_h + SPACE_M;
            f
        });
        // the sidebar runs along the left from the location bar down to the end of the list
        let x0 = pad + side_w + SPACE_M;
        let back = R { x: x0, y, w: bar_h, h: bar_h };
        let location = R { x: x0 + bar_h + SPACE_S, y, w: w - pad - x0 - bar_h - SPACE_S, h: bar_h };
        let top = y;
        y += bar_h + SPACE_S;
        let list = R { x: x0, y, w: w - pad - x0, h: row_h * VISIBLE_ROWS as f32 };
        let sidebar = R { x: pad, y: top, w: side_w, h: list.y + list.h - top };
        y += list.h + SPACE_XL;
        let ok = R { x: w - pad - button_w, y, w: button_w, h: bar_h };
        let cancel = R { x: ok.x - SPACE_M - button_w, y, w: button_w, h: bar_h };
        let new_folder = R { x: pad, y, w: 120.0, h: bar_h };
        Layout { w, h: y + bar_h + pad, title, field, sidebar, sidebar_row_h: row_h, new_folder, back, location, list, row_h, cancel, ok }
    }

    /// Rows the list leaves for entries: the New Folder field takes the first row while it is open.
    fn row_offset(&self) -> usize {
        usize::from(self.creating.is_some())
    }

    /// A click at (x, y) points inside the panel, at time `t` ms.
    pub fn click(&mut self, x: f32, y: f32, t: f64, opener: &Opener) -> Option<Outcome> {
        let l = self.layout();
        if l.cancel.contains(x, y) {
            // Cancel closes the New Folder field first, as Esc does
            if self.creating.take().is_some() {
                return None;
            }
            return Some(Outcome::Cancel);
        }
        if l.ok.contains(x, y) {
            return self.confirm(opener);
        }
        if self.can_create_dir() && l.new_folder.contains(x, y) {
            self.begin_new_folder();
            return None;
        }
        if self.creating.is_some() {
            return None;
        }
        if l.back.contains(x, y) {
            self.up(opener);
            return None;
        }
        if l.sidebar.contains(x, y) {
            let i = ((y - l.sidebar.y) / l.sidebar_row_h) as usize;
            self.go_place(i, opener);
            return None;
        }
        if l.list.contains(x, y) {
            let i = self.top + ((y - l.list.y) / l.row_h) as usize;
            if i < self.entries.len() {
                let double = self.last_click.is_some_and(|(j, t0)| j == i && t - t0 <= DOUBLE_CLICK_MS);
                self.last_click = Some((i, t));
                if double {
                    self.last_click = None;
                    return self.activate(i, opener);
                }
                self.toggle(i);
            } else {
                self.selected = None;
                self.marked.clear();
            }
        }
        None
    }

    /// A key while the panel is open (evdev codes). `Some` when the panel took it; text is typed through `type_text`.
    pub fn key(&mut self, code: u16, opener: &Opener) -> Option<Option<Outcome>> {
        Some(match code {
            1 if self.creating.is_some() => {
                self.creating = None;
                None
            }
            1 => Some(Outcome::Cancel),
            28 | 96 => self.confirm(opener),
            103 => {
                self.step(-1);
                None
            }
            108 => {
                self.step(1);
                None
            }
            105 => {
                self.up(opener);
                None
            }
            106 => {
                if let Some(i) = self.selected {
                    self.enter(i, opener);
                }
                None
            }
            14 => {
                self.backspace();
                None
            }
            _ => return None,
        })
    }
}

/// The broker the shell holds: opener for the account, grant records and saves waiting for `DocCommit`.
pub struct Broker {
    pub opener: Opener,
    pub grants: Grants,
    saves: Vec<(String, u32, GrantId, PendingSave)>,
    /// app id -> the folder its last panel ended in (`NativeOpenPanel` remembers it per app)
    pub last_dir: std::collections::HashMap<String, PathBuf>,
}

impl Broker {
    /// A file `from` holds a grant for, handed to `to` read-only (share sheet, drag and drop): `to` gets a grant of its
    /// own and the descriptor, as the `DocGranted` for `request`. The sender's grant is checked like a reopen.
    pub fn transfer(&mut self, from: &str, grant: &GrantId, to: &str, request: u32) -> Result<(ToApp, std::fs::File), DocErrorCode> {
        let (f, g) = self.grants.reopen(&self.opener, from, grant)?;
        let path = g.path.clone();
        let size = f.metadata().map_or(0, |m| m.len());
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let id = self.grants.add(to, &path, DocMode::Read, &f).map_err(|_| DocErrorCode::Denied)?;
        Ok((ToApp::DocGranted { request, grant: id, mode: DocMode::Read, name, size }, f))
    }

    pub fn new(opener: Opener, grants_file: &Path) -> Self {
        Broker { opener, grants: Grants::load(grants_file), saves: Vec::new(), last_dir: Default::default() }
    }

    /// The broker for the installed account, or `None` when there is none (the host, a fresh install before setup).
    pub fn installed() -> Option<Self> {
        let a = crate::bare::app_policy::Account::installed().ok()?;
        let opener = Opener::new(&a.home, a.uid, a.gid).ok()?;
        Some(Broker::new(opener, &super::apphost::root().join("documents/grants.json")))
    }

    /// The app's recently opened or saved documents, newest first, each once (the panel's Recents).
    pub fn recents(&self, app_id: &str) -> Vec<PathBuf> {
        let mut g: Vec<&docbroker::Grant> = self.grants.of_app(app_id).collect();
        // grants are recorded oldest first; reversing before the stable sort puts the latest first within one second
        g.reverse();
        g.sort_by(|a, b| b.created.cmp(&a.created));
        let mut out: Vec<PathBuf> = Vec::new();
        for x in g {
            if !out.contains(&x.path) {
                out.push(x.path.clone());
            }
        }
        out.truncate(MAX_RECENTS);
        out
    }

    /// `finish` for any outcome: the replies in order, each with its descriptor. A choice of several files is a
    /// `DocBatch` followed by one `DocGranted` per file that could be opened.
    pub fn finish_many(&mut self, app_id: &str, request: u32, kind: Kind, out: Outcome) -> Vec<(ToApp, Option<std::fs::File>)> {
        let paths = match out {
            Outcome::OpenMany(paths) => paths,
            other => return vec![self.finish(app_id, request, kind, other)],
        };
        let Kind::Open(mode) = kind else { return vec![(ToApp::DocError { request, code: DocErrorCode::Unsupported }, None)] };
        let mut granted = Vec::new();
        let mut first_err = DocErrorCode::Denied;
        for path in paths.into_iter().take(bifrost_proto::MAX_DOC_BATCH as usize) {
            match self.open_chosen(app_id, request, &path, mode) {
                (m @ ToApp::DocGranted { .. }, f) => granted.push((m, f)),
                (ToApp::DocError { code, .. }, _) => first_err = code,
                _ => {}
            }
        }
        if granted.is_empty() {
            return vec![(ToApp::DocError { request, code: first_err }, None)];
        }
        let mut v = vec![(ToApp::DocBatch { request, count: granted.len() as u32 }, None)];
        v.extend(granted);
        v
    }

    /// Opens one chosen file or folder and records its grant.
    fn open_chosen(&mut self, app_id: &str, request: u32, path: &Path, mode: DocMode) -> (ToApp, Option<std::fs::File>) {
        let fail = |code| (ToApp::DocError { request, code }, None);
        match self.opener.open_any(path, mode) {
            Ok(f) => {
                let size = f.metadata().map_or(0, |m| m.len());
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                match self.grants.add(app_id, path, mode, &f) {
                    Ok(grant) => (ToApp::DocGranted { request, grant, mode, name, size }, Some(f)),
                    Err(_) => fail(DocErrorCode::Denied),
                }
            }
            Err(code) => fail(code),
        }
    }

    /// Carries out what the user chose for `app_id`'s `request`: the reply and the descriptor to send with it.
    /// (A choice of several files answers through `finish_many`; here it is the first.)
    pub fn finish(&mut self, app_id: &str, request: u32, kind: Kind, out: Outcome) -> (ToApp, Option<std::fs::File>) {
        let fail = |code| (ToApp::DocError { request, code }, None);
        match (kind, out) {
            (_, Outcome::Cancel) => fail(DocErrorCode::Cancelled),
            (Kind::Open(mode), Outcome::OpenMany(paths)) => match paths.first() {
                Some(path) => self.open_chosen(app_id, request, path, mode),
                None => fail(DocErrorCode::Cancelled),
            },
            (Kind::Open(mode), Outcome::Open(path)) => match self.opener.open_any(&path, mode) {
                Ok(f) => {
                    let size = f.metadata().map_or(0, |m| m.len());
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    match self.grants.add(app_id, &path, mode, &f) {
                        Ok(grant) => (ToApp::DocGranted { request, grant, mode, name, size }, Some(f)),
                        Err(_) => fail(DocErrorCode::Denied),
                    }
                }
                Err(code) => fail(code),
            },
            (Kind::Save, Outcome::Save { dir, name }) => match self.opener.begin_save(&dir, &name) {
                Ok((pending, f)) => {
                    let grant = docbroker::new_grant_id();
                    // a second save on the same request replaces the first, whose temporary file goes away
                    if let Some(i) = self.saves.iter().position(|(a, r, ..)| a == app_id && *r == request) {
                        let (.., old) = self.saves.remove(i);
                        old.abort(&self.opener);
                    }
                    self.saves.push((app_id.to_string(), request, grant, pending));
                    (ToApp::DocGranted { request, grant, mode: DocMode::Write, name, size: 0 }, Some(f))
                }
                Err(code) => fail(code),
            },
            _ => fail(DocErrorCode::Unsupported),
        }
    }

    /// `DocCommit`, `DocAbort`, `DocReopen`, `DocRelease` from `app_id`. The reply, if any, and its descriptor.
    pub fn request(&mut self, app_id: &str, msg: ToShell) -> Option<(ToApp, Option<std::fs::File>)> {
        match msg {
            ToShell::DocCommit { request } => {
                let Some(i) = self.saves.iter().position(|(a, r, ..)| a == app_id && *r == request) else {
                    return Some((ToApp::DocError { request, code: DocErrorCode::Denied }, None));
                };
                let (_, _, grant, pending) = self.saves.remove(i);
                // a save in place keeps its grant id: the record for the replaced inode goes before the new one is added
                let _ = self.grants.release(app_id, &grant);
                Some(match pending.commit(&self.opener) {
                    Ok((path, f)) => {
                        let _ = self.grants.insert(grant, app_id, &path, DocMode::Write, &f);
                        let size = f.metadata().map_or(0, |m| m.len());
                        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        // the answer to a commit is the saved file itself, the same grant id the app already holds
                        (ToApp::DocGranted { request, grant, mode: DocMode::Write, name, size }, Some(f))
                    }
                    Err(code) => (ToApp::DocError { request, code }, None),
                })
            }
            ToShell::DocAbort { request } => {
                if let Some(i) = self.saves.iter().position(|(a, r, ..)| a == app_id && *r == request) {
                    let (.., pending) = self.saves.remove(i);
                    pending.abort(&self.opener);
                }
                None
            }
            ToShell::DocReopen { request, grant } => Some(match self.grants.reopen(&self.opener, app_id, &grant) {
                Ok((f, g)) => {
                    let size = f.metadata().map_or(0, |m| m.len());
                    let name = g.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    (ToApp::DocGranted { request, grant, mode: g.mode, name, size }, Some(f))
                }
                Err(code) => (ToApp::DocError { request, code }, None),
            }),
            ToShell::DocSaveGrant { request, grant } => Some(match self.grants.get(app_id, &grant).map(|g| (g.path.clone(), g.mode)) {
                Ok((path, DocMode::Write)) => {
                    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    match self.opener.begin_save(&dir, &name) {
                        Ok((pending, f)) => {
                            if let Some(i) = self.saves.iter().position(|(a, r, ..)| a == app_id && *r == request) {
                                let (.., old) = self.saves.remove(i);
                                old.abort(&self.opener);
                            }
                            self.saves.push((app_id.to_string(), request, grant, pending));
                            (ToApp::DocGranted { request, grant, mode: DocMode::Write, name, size: 0 }, Some(f))
                        }
                        Err(code) => (ToApp::DocError { request, code }, None),
                    }
                }
                Ok(_) => (ToApp::DocError { request, code: DocErrorCode::Denied }, None),
                Err(code) => (ToApp::DocError { request, code }, None),
            }),
            ToShell::DocRelease { grant } => {
                let _ = self.grants.release(app_id, &grant);
                None
            }
            ToShell::DocGrantPath { request, path } => Some(self.drag_grant(app_id, request, &path)),
            _ => None,
        }
    }

    /// A read grant for `path` (absolute, under the home) for a drag `app_id` begins: the file items of the drag carry it,
    /// and the shell releases it when the drag ends (`dnd`). Paths outside the home are `Denied`, as are symlinks, and
    /// so is every path the person did not hand to a `documents`-only app: only a file or folder it already holds a grant
    /// for (chosen in an open or save panel, dropped or shared onto it) can be granted again, so it cannot name its way
    /// to the rest of the home folder. An app that holds `storage` already has the whole home bound read-write, so a
    /// descriptor gives it nothing new and the grant-list requirement is skipped for it (Files drags what it browses).
    fn drag_grant(&mut self, app_id: &str, request: u32, path: &str) -> (ToApp, Option<std::fs::File>) {
        let refuse = |code| (ToApp::DocError { request, code }, None);
        let Ok(rel) = Path::new(path).strip_prefix(self.opener.home()) else { return refuse(DocErrorCode::Denied) };
        if rel.as_os_str().is_empty() {
            return refuse(DocErrorCode::Denied);
        }
        let rel = rel.to_path_buf();
        if !super::services::app_has_permission(app_id, "storage") && !self.grants.of_app(app_id).any(|g| g.path == rel) {
            return refuse(DocErrorCode::Denied);
        }
        let f = match self.opener.open_any(&rel, DocMode::Read) {
            Ok(f) => f,
            Err(code) => return refuse(code),
        };
        let size = f.metadata().map_or(0, |m| m.len());
        let name = rel.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match self.grants.add(app_id, &rel, DocMode::Read, &f) {
            Ok(grant) => (ToApp::DocGranted { request, grant, mode: DocMode::Read, name, size }, Some(f)),
            Err(_) => refuse(DocErrorCode::Denied),
        }
    }

    /// The app quit or was removed: its unfinished saves go away (grants stay; removal revokes them separately).
    pub fn app_gone(&mut self, app_id: &str) {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.saves).into_iter().partition(|(a, ..)| a == app_id);
        self.saves = keep;
        for (.., p) in gone {
            p.abort(&self.opener);
        }
    }
}


/// Only apps whose manifest asks for `documents` may open panels (the app policy accepts it; it binds nothing).
fn permitted(slot: u8) -> bool {
    #[cfg(test)]
    if tests::ALLOW.with(|a| a.get()) {
        return true;
    }
    super::apphost::installed().get(slot as usize).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "documents"))
}

impl super::Shell {
    fn doc_app_id(&self, slot: u8) -> Option<String> {
        self.ext.get(slot as usize).and_then(Option::as_ref).map(|h| h.id.clone())
    }

    fn doc_send(&mut self, slot: u8, reply: ToApp, file: Option<std::fs::File>) {
        if let Some(h) = self.ext.get_mut(slot as usize).and_then(Option::as_mut) {
            match &file {
                Some(f) => h.send_file(&reply, f),
                None => h.send(&reply),
            }
        }
    }

    /// Files `from` shares or drops on the app `to_app` in `to_slot`: one `DocGranted` per file (after a `DocBatch` for
    /// several), all with `request`. Files whose grant is not the sender's, or is stale, are left out. Returns
    /// (sender grant, new grant) for each file handed over.
    pub(super) fn inbound_files(&mut self, from: &str, to_app: &str, to_slot: u8, request: u32, grants: &[GrantId]) -> Vec<(GrantId, GrantId)> {
        let mut out = Vec::new();
        let mut mapping = Vec::new();
        if let Some(b) = self.doc_broker() {
            for g in grants.iter().take(bifrost_proto::MAX_DOC_BATCH as usize) {
                if let Ok(r) = b.transfer(from, g, to_app, request) {
                    if let ToApp::DocGranted { grant, .. } = &r.0 {
                        mapping.push((*g, *grant));
                    }
                    out.push(r);
                }
            }
        }
        if out.len() > 1 {
            self.doc_send(to_slot, ToApp::DocBatch { request, count: out.len() as u32 }, None);
        }
        for (m, f) in out {
            self.doc_send(to_slot, m, Some(f));
        }
        mapping
    }

    /// The account's home and, for each of `grants`, its absolute path and whether it is a write grant. `app_id` must own
    /// every grant (`Denied` otherwise). For the workspace: reveal and recycle act on the path; the app never sees it.
    pub(super) fn grant_files(&mut self, app_id: &str, grants: &[GrantId]) -> Result<(PathBuf, Vec<(PathBuf, bool)>), DocErrorCode> {
        let b = self.doc_broker().ok_or(DocErrorCode::Unsupported)?;
        let home = b.opener.home().to_path_buf();
        let files = grants.iter().map(|g| b.grants.get(app_id, g).map(|g| (home.join(&g.path), g.mode == DocMode::Write))).collect::<Result<Vec<_>, _>>()?;
        Ok((home, files))
    }

    pub(super) fn doc_broker(&mut self) -> Option<&mut Broker> {
        self.doc_broker.get_or_insert_with(Broker::installed).as_mut()
    }

    /// A `Doc*` request from the app in `slot` (`ext_tick` collects them).
    pub(super) fn doc_request(&mut self, slot: u8, msg: ToShell) {
        let request = match &msg {
            ToShell::DocOpen { request, .. } | ToShell::DocOpenEx { request, .. } | ToShell::DocSave { request, .. } | ToShell::DocCommit { request } | ToShell::DocAbort { request } | ToShell::DocReopen { request, .. } | ToShell::DocSaveGrant { request, .. } | ToShell::DocGrantPath { request, .. } => Some(*request),
            _ => None,
        };
        let refuse = |sh: &mut Self, code| {
            if let Some(request) = request {
                sh.doc_send(slot, ToApp::DocError { request, code }, None);
            }
        };
        let Some(app_id) = self.doc_app_id(slot) else { return };
        if !permitted(slot) || self.doc_broker().is_none() {
            return refuse(self, DocErrorCode::Unsupported);
        }
        let (kind, request, title, filters, suggested, opts) = match msg {
            ToShell::DocOpen { request, mode, title, filters } => (Kind::Open(mode), request, title, filters, String::new(), Options::default()),
            ToShell::DocOpenEx { request, mode, title, filters, options } => (Kind::Open(mode), request, title, filters, String::new(), Options::from_bits(options)),
            ToShell::DocSave { request, suggested_name, filters } => (Kind::Save, request, String::new(), filters, suggested_name, Options::default()),
            other => {
                if let Some((reply, file)) = self.doc_broker().and_then(|b| b.request(&app_id, other)) {
                    self.doc_send(slot, reply, file);
                }
                return;
            }
        };
        // one panel at a time, as one sheet per window: a second request is cancelled, not queued
        if self.doc_panel.is_some() {
            return refuse(self, DocErrorCode::Cancelled);
        }
        let packages = super::apphost::installed().get(slot as usize).map(|a| a.manifest.package_types.clone()).unwrap_or_default();
        let b = self.doc_broker().unwrap();
        let mut panel = Panel::new(slot, request, kind, &title, filters, &suggested, &b.opener).with_options(opts, &b.opener).with_packages(packages, &b.opener).with_recents(b.recents(&app_id), &b.opener);
        if let Some(last) = b.last_dir.get(&app_id).cloned() {
            panel.go_to(&last, &b.opener);
        }
        self.doc_panel = Some(panel);
        self.desktop_menu = Some(super::desktop_bar::Menu::Document);
        self.dirty = true;
    }

    /// The panel stays up until it is answered, whatever else closed the popup (a menu-bar click, Esc elsewhere).
    pub(super) fn doc_keep_shown(&mut self) {
        if self.doc_panel.is_some() && self.desktop_menu != Some(super::desktop_bar::Menu::Document) {
            self.desktop_menu = Some(super::desktop_bar::Menu::Document);
            self.dirty = true;
        }
    }

    /// The app in `slot` exited: close its panel and throw away its unfinished saves.
    pub(super) fn doc_app_gone(&mut self, slot: u8) {
        if self.doc_panel.as_ref().is_some_and(|p| p.slot == slot) {
            self.doc_panel = None;
            self.desktop_menu = None;
            self.dirty = true;
        }
        if let Some(id) = self.doc_app_id(slot) {
            if let Some(b) = self.doc_broker.as_mut().and_then(Option::as_mut) {
                b.app_gone(&id);
            }
        }
    }

    fn doc_finish(&mut self, out: Outcome) {
        let Some(p) = self.doc_panel.take() else { return };
        self.desktop_menu = None;
        self.dirty = true;
        let Some(app_id) = self.doc_app_id(p.slot) else { return };
        let Some(b) = self.doc_broker() else { return };
        if out != Outcome::Cancel && !p.in_recents {
            b.last_dir.insert(app_id.clone(), p.dir.clone());
        }
        for (reply, file) in b.finish_many(&app_id, p.request, p.kind, out) {
            self.doc_send(p.slot, reply, file);
        }
    }

    /// A key while the panel is open; true when the panel took it.
    pub(super) fn doc_key(&mut self, code: u16, _t: f64) -> bool {
        let (Some(mut p), Some(b)) = (self.doc_panel.take(), self.doc_broker.as_mut().and_then(Option::as_mut)) else { return false };
        let taken = p.key(code, &b.opener);
        self.doc_panel = Some(p);
        let took = taken.is_some();
        self.dirty |= took;
        if let Some(Some(out)) = taken {
            self.doc_finish(out);
        }
        took
    }

    pub(super) fn doc_type(&mut self, text: &str) {
        if let Some(p) = self.doc_panel.as_mut() {
            self.dirty |= p.type_text(text);
        }
    }

    /// A click at (x, y) px from the panel's top-left corner.
    pub(super) fn doc_click(&mut self, x: f32, y: f32, t: f64) {
        let (Some(mut p), Some(b)) = (self.doc_panel.take(), self.doc_broker.as_mut().and_then(Option::as_mut)) else { return };
        let out = p.click(x / self.ui.pt(1.0), y / self.ui.pt(1.0), t, &b.opener);
        self.doc_panel = Some(p);
        self.dirty = true;
        if let Some(out) = out {
            self.doc_finish(out);
        }
    }

    /// Where the panel sits, in px: a sheet under its window's title bar, centred on the window (on the screen when the
    /// app has no window), always below the menu bar and inside the screen.
    pub(super) fn doc_panel_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let p = self.doc_panel.as_ref()?;
        let l = p.layout();
        let (w, h) = (self.ui.pt(l.w).ceil() as i32, self.ui.pt(l.h).ceil() as i32);
        let body = self.desk.as_ref().and_then(|d| d.index_of(super::AppId::External(p.slot)).map(|i| d.wins[i].body(&self.desk_metrics())));
        let (cx, top) = body.map_or((self.w / 2, (self.h - h) / 2), |b| (b.x + b.w / 2, b.y));
        let bar = self.desktop_bar_height() as i32;
        let x = (cx - w / 2).clamp(0, (self.w - w).max(0));
        let y = top.max(bar).min((self.h - h).max(bar));
        Some((x as usize, y as usize, w as usize, h as usize))
    }

    /// A sheet blocks the window it belongs to: the window under the panel is dimmed, as a sheet dims its parent. The panel
    /// itself hangs from the top of that window (`doc_panel_rect`). Drawn before the panel's material.
    pub(super) fn draw_doc_dim(&self, s: &mut crate::gfx::Surface) {
        let Some(p) = self.doc_panel.as_ref() else { return };
        let Some(body) = self.desk.as_ref().and_then(|d| d.index_of(super::AppId::External(p.slot)).map(|i| d.wins[i].body(&self.desk_metrics()))) else { return };
        let radius = self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS);
        s.fill_rrect(body.x as f32, body.y as f32, body.w as f32, body.h as f32, radius, crate::gfx::rgba(0, 0, 0, SHEET_DIM_ALPHA));
    }

    /// Draws the panel's content over the popup background (`draw_desktop_popup` drew the material).
    pub(super) fn draw_doc_panel(&self, s: &mut crate::gfx::Surface, x: usize, y: usize, _w: usize, _h: usize) {
        use super::ui::{Weight, DETAIL_BG, FIELD_BG, LABEL, LINK, MAC_BODY_PT, MAC_HEADLINE_PT, SECONDARY, SELECTION, SEPARATOR_C, SPACE_S, WHITE};
        let Some(p) = self.doc_panel.as_ref() else { return };
        // the focus rings and the default button follow the user's accent colour; the back control stays a link
        let accent = bifrost_render::appearance::accent_now();
        let l = p.layout();
        let ui = &self.ui;
        let (ox, oy) = (x as f32, y as f32);
        let px = |r: &R| (ox + ui.pt(r.x), oy + ui.pt(r.y), ui.pt(r.w), ui.pt(r.h));
        // text vertically centred in `r`, starting `inset` pt from its left edge, cut to fit
        let text = |s: &mut crate::gfx::Surface, r: &R, inset: f32, t: &str, size: f32, weight: Weight, color: u32| {
            let (rx, ry, rw, rh) = px(r);
            let t = ui.fit(t, size, rw - ui.pt(2.0 * inset));
            self.label(&t, size, weight, color, |im| s.blit(im, (rx + ui.pt(inset)) as i32, (ry + (rh - im.h as f32) / 2.0) as i32, 255, 255));
        };
        let radius = ui.pt(6.0);
        text(s, &l.title, 0.0, &p.title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now());

        if let Some(f) = &l.field {
            let label = R { x: l.title.x, y: f.y, w: f.x - l.title.x, h: f.h };
            text(s, &label, 0.0, "Save As:", MAC_BODY_PT, Weight::Regular, LABEL.now());
            let (fx, fy, fw, fh) = px(f);
            // the focused field: content background and the focus ring in the tint
            s.fill_rrect(fx, fy, fw, fh, radius, DETAIL_BG.now());
            s.stroke_rrect(fx, fy, fw, fh, radius, ui.pt(1.0), accent);
            match p.name_sel.as_ref().filter(|r| !r.is_empty() && ui.measure(&p.name, MAC_BODY_PT, Weight::Regular) <= fw - ui.pt(2.0 * SPACE_S)) {
                // the base name is selected: a plate behind it, its text in white, and no caret until the first edit
                Some(sel) => {
                    let (start, end) = (sel.start.min(p.name.len()), sel.end.min(p.name.len()));
                    let before = ui.measure(&p.name[..start], MAC_BODY_PT, Weight::Regular);
                    let picked = ui.measure(&p.name[start..end], MAC_BODY_PT, Weight::Regular);
                    let left = fx + ui.pt(SPACE_S);
                    s.fill_rrect(left + before, fy + ui.pt(3.0), picked, fh - ui.pt(6.0), ui.pt(3.0), SELECTION.now());
                    let mut x = left;
                    for (part, color) in [(&p.name[..start], LABEL.now()), (&p.name[start..end], WHITE), (&p.name[end..], LABEL.now())] {
                        if part.is_empty() {
                            continue;
                        }
                        self.label(part, MAC_BODY_PT, Weight::Regular, color, |im| s.blit(im, x as i32, (fy + (fh - im.h as f32) / 2.0) as i32, 255, 255));
                        x += ui.measure(part, MAC_BODY_PT, Weight::Regular);
                    }
                }
                None => {
                    let shown = format!("{}|", p.name);
                    text(s, f, SPACE_S, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now());
                }
            }
        }

        // back control and the folder's name (detail navigation: back in LINK, disabled at the top)
        text(s, &l.back, SPACE_S, "\u{2039}", MAC_HEADLINE_PT, Weight::Bold, if p.can_go_up() { LINK } else { SECONDARY });
        text(s, &l.location, 0.0, &p.location(), MAC_BODY_PT, Weight::Bold, LABEL.now());

        // sidebar: plates of 9 pt (the sidebar selection radius) behind the active place
        let plate = ui.pt(9.0);
        let active = p.active_place();
        for (i, place) in p.places.iter().enumerate() {
            let r = R { x: l.sidebar.x, y: l.sidebar.y + l.sidebar_row_h * i as f32, w: l.sidebar.w, h: l.sidebar_row_h };
            if active == Some(i) {
                let (rx, ry, rw, rh) = px(&r);
                s.fill_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), plate, SELECTION.now());
            }
            text(s, &r, SPACE_S, &place.label, MAC_BODY_PT, Weight::Regular, if active == Some(i) { WHITE } else { LABEL.now() });
        }

        let (lx, ly, lw, lh) = px(&l.list);
        s.fill_rrect(lx, ly, lw, lh, radius, DETAIL_BG.now());
        s.stroke_rrect(lx, ly, lw, lh, radius, ui.pt(1.0), SEPARATOR_C.now());
        // New Folder: the name field takes the first row while it is open
        let offset = p.row_offset();
        if let Some(name) = &p.creating {
            let r = R { x: l.list.x + 4.0, y: l.list.y, w: l.list.w - 8.0, h: l.row_h };
            let (rx, ry, rw, rh) = px(&r);
            s.stroke_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), radius, ui.pt(1.0), accent);
            text(s, &r, SPACE_S, &format!("{name}|"), MAC_BODY_PT, Weight::Regular, LABEL.now());
        }
        for (row, e) in p.entries.iter().enumerate().skip(p.top).take(VISIBLE_ROWS - offset) {
            let r = R { x: l.list.x + 4.0, y: l.list.y + l.row_h * (row - p.top + offset) as f32, w: l.list.w - 8.0, h: l.row_h };
            let selected = p.marked.contains(&row);
            if selected {
                let (rx, ry, rw, rh) = px(&r);
                s.fill_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), radius, SELECTION.now());
            }
            let color = if selected { WHITE } else if e.enabled { LABEL.now() } else { SECONDARY };
            text(s, &R { w: r.w - 24.0, ..r }, SPACE_S, &e.name, MAC_BODY_PT, Weight::Regular, color);
            if e.dir {
                text(s, &R { x: r.x + r.w - 24.0, w: 24.0, ..r }, SPACE_S, "\u{203a}", MAC_BODY_PT, Weight::Regular, if selected { WHITE } else { SECONDARY });
            }
        }
        if p.entries.is_empty() && p.creating.is_none() {
            text(s, &R { y: l.list.y + l.row_h, h: l.row_h, ..l.list }, SPACE_S, if p.in_recents { "No recent documents" } else { "No items" }, MAC_BODY_PT, Weight::Regular, SECONDARY);
        }

        // Cancel, then the default button (Open or Save) in the tint; the default is dimmed when it would do nothing
        if p.can_create_dir() {
            let (nx, ny, nw, nh) = px(&l.new_folder);
            s.fill_rrect(nx, ny, nw, nh, radius, FIELD_BG.now());
            self.label("New Folder", MAC_BODY_PT, Weight::Regular, LABEL.now(), |im| s.blit(im, (nx + (nw - im.w as f32) / 2.0) as i32, (ny + (nh - im.h as f32) / 2.0) as i32, 255, 255));
        }
        let (cx, cy, cw, ch) = px(&l.cancel);
        s.fill_rrect(cx, cy, cw, ch, radius, FIELD_BG.now());
        self.label("Cancel", MAC_BODY_PT, Weight::Regular, LABEL.now(), |im| s.blit(im, (cx + (cw - im.w as f32) / 2.0) as i32, (cy + (ch - im.h as f32) / 2.0) as i32, 255, 255));
        let (bx, by, bw, bh) = px(&l.ok);
        let enabled = p.can_confirm();
        s.fill_rrect(bx, by, bw, bh, radius, if enabled { accent } else { FIELD_BG.now() });
        let ok = if p.kind == Kind::Save { "Save" } else { "Open" };
        self.label(ok, MAC_BODY_PT, Weight::Regular, if enabled { WHITE } else { SECONDARY }, |im| s.blit(im, (bx + (bw - im.w as f32) / 2.0) as i32, (by + (bh - im.h as f32) / 2.0) as i32, 255, 255));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    thread_local! {
        /// Tests stand in for an installed app with the `documents` permission.
        pub(crate) static ALLOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub(crate) fn scratch(tag: &str) -> (PathBuf, Opener) {
        let d = std::env::temp_dir().join(format!("docpanel-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/Documents/Drafts")).unwrap();
        for f in ["b.txt", "A.md", "photo.png", ".hidden"] {
            fs::write(d.join("home/Documents").join(f), f).unwrap();
        }
        let m = fs::metadata(&d).unwrap();
        let o = Opener::new(&d.join("home"), m.uid(), m.gid()).unwrap();
        (d, o)
    }

    fn names(p: &Panel) -> Vec<&str> {
        p.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn starts_in_documents_sorted_without_hidden_files_and_dims_what_the_filters_refuse() {
        let (d, o) = scratch("list");
        let p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec!["public.text".into()], "", &o);
        assert_eq!((p.dir.as_path(), p.title.as_str(), p.location()), (Path::new("Documents"), "Open", "Documents".into()));
        assert_eq!(names(&p), ["A.md", "b.txt", "Drafts", "photo.png"]);
        let enabled: Vec<bool> = p.entries.iter().map(|e| e.enabled).collect();
        assert_eq!(enabled, [true, true, true, false], "markdown and plain text conform to public.text; a PNG does not");
        let ext = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec!["PNG".into()], "", &o);
        assert_eq!(ext.entries.iter().filter(|e| e.enabled && !e.dir).map(|e| e.name.as_str()).collect::<Vec<_>>(), ["photo.png"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn keys_move_open_folders_go_up_and_choose() {
        let (d, o) = scratch("keys");
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        assert_eq!(p.key(28, &o), Some(None), "Return with nothing selected does nothing");
        p.key(108, &o);
        p.key(108, &o);
        p.key(108, &o);
        assert_eq!(p.selected, Some(2));
        assert_eq!(p.key(28, &o), Some(None), "Return on a folder opens it");
        assert_eq!(p.dir, Path::new("Documents/Drafts"));
        p.key(105, &o);
        assert_eq!((p.dir.as_path(), p.selected), (Path::new("Documents"), Some(2)), "Left goes up with the folder selected");
        p.key(103, &o);
        assert_eq!(p.key(28, &o), Some(Some(Outcome::Open("Documents/b.txt".into()))));
        p.key(105, &o);
        p.key(105, &o);
        assert_eq!((p.location(), p.can_go_up()), ("Home".into(), false));
        assert_eq!(p.key(1, &o), Some(Some(Outcome::Cancel)));
        assert_eq!(p.key(30, &o), None, "letters are not the panel's keys in an open panel");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clicks_select_double_click_opens_and_the_buttons_answer() {
        let (d, o) = scratch("click");
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec!["public.text".into()], "", &o);
        let l = p.layout();
        let row = |i: usize| l.list.y + l.row_h * (i as f32 + 0.5);
        assert_eq!(p.click(l.list.x + 10.0, row(3), 0.0, &o), None);
        assert!(!p.can_confirm(), "a dimmed file cannot be chosen");
        assert_eq!(p.click(l.ok.x + 1.0, l.ok.y + 1.0, 10.0, &o), None);
        p.click(l.list.x + 10.0, row(1), 1000.0, &o);
        assert_eq!(p.click(l.list.x + 10.0, row(1), 1200.0, &o), Some(Outcome::Open("Documents/b.txt".into())));
        p.click(l.list.x + 10.0, row(0), 5000.0, &o);
        assert_eq!(p.click(l.list.x + 10.0, row(0), 6000.0, &o), None, "two slow clicks are not a double click");
        assert_eq!(p.click(l.cancel.x + 1.0, l.cancel.y + 1.0, 7000.0, &o), Some(Outcome::Cancel));
        assert!(l.list.y + l.list.h < l.ok.y && l.h > l.ok.y + l.ok.h, "buttons sit under the list, inside the panel");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_save_panel_types_a_name_and_a_click_on_a_file_copies_its_name() {
        let (d, o) = scratch("save");
        let mut p = Panel::new(0, 1, Kind::Save, "Export", vec![], "Untitled/1.md", &o);
        assert_eq!((p.name.as_str(), p.title.as_str()), ("Untitled1.md", "Export"), "a slash cannot be part of the name");
        assert!(p.layout().field.is_some());
        assert_eq!(p.name_sel, Some(0..9), "the base name is selected as the panel opens, the extension is not");
        p.type_text("x\n");
        assert_eq!(p.name, "x.md", "typing replaces the selected base name and keeps the extension");
        assert_eq!(p.name_sel, None);
        p.type_text("y");
        assert_eq!(p.name, "x.mdy", "with nothing selected typing goes in at the end");
        p.backspace();
        assert_eq!(p.name, "x.md");
        p.select(1);
        assert_eq!(p.name_sel, Some(0..1), "choosing a file selects its base name too");
        assert_eq!(p.name, "b.txt");
        assert_eq!(p.key(28, &o), Some(Some(Outcome::Save { dir: "Documents".into(), name: "b.txt".into() })));
        p.name.clear();
        p.selected = None;
        assert!(!p.can_confirm(), "an empty name cannot be saved");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_name_field_selects_the_base_name_and_editing_replaces_or_deletes_it() {
        let (d, o) = scratch("namesel");
        let save = |name: &str| Panel::new(0, 1, Kind::Save, "", vec![], name, &o);
        // a name with an extension selects what is before the last dot; one without selects all of it
        assert_eq!(save("Report.final.rtf").name_sel, Some(0..12));
        assert_eq!(save("Untitled").name_sel, Some(0..8));
        assert_eq!(save(".profile").name_sel, Some(0..8), "a leading dot is part of the name");
        assert_eq!(save("").name_sel, None, "an empty name has nothing to select");
        let open = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "Untitled.rtf", &o);
        assert_eq!(open.name_sel, None, "only a save panel has a name field");
        // Backspace over the selection deletes it and keeps the extension
        let mut p = save("Untitled.rtf");
        assert!(p.backspace());
        assert_eq!((p.name.as_str(), p.name_sel.clone()), (".rtf", None));
        assert!(p.backspace(), "the next Backspace deletes a character as usual");
        assert_eq!(p.name, ".rt");
        // a slash is not typeable: it leaves the selection alone
        let mut p = save("Untitled.rtf");
        p.type_text("/");
        assert_eq!((p.name.as_str(), p.name_sel.clone()), ("Untitled.rtf", Some(0..8)));
        // typing replaces the selection in the middle of text and the limit still holds
        p.type_text("Résumé");
        assert_eq!(p.name, "Résumé.rtf");
        let mut long = save(&format!("{}.md", "a".repeat(300)));
        assert_eq!(long.name.len(), 255);
        long.type_text("b");
        assert!(long.name.len() <= 255 && long.name.starts_with('b'));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_broker_grants_opens_and_saves_with_commit_and_abort() {
        use std::io::{Read, Write};
        let (d, o) = scratch("broker");
        let mut b = Broker::new(o, &d.join("state/grants.json"));
        let (reply, f) = b.finish("app.a", 7, Kind::Open(DocMode::Read), Outcome::Open("Documents/b.txt".into()));
        let ToApp::DocGranted { request: 7, grant, name, size: 5, .. } = reply else { panic!("{reply:?}") };
        assert_eq!(name, "b.txt");
        let mut s = String::new();
        f.unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "b.txt");
        assert!(matches!(b.request("app.a", ToShell::DocReopen { request: 8, grant }), Some((ToApp::DocGranted { request: 8, .. }, Some(_)))));
        assert!(matches!(b.request("app.b", ToShell::DocReopen { request: 9, grant }), Some((ToApp::DocError { code: DocErrorCode::Denied, .. }, None))));
        assert_eq!(b.finish("app.a", 1, Kind::Open(DocMode::Read), Outcome::Cancel).0, ToApp::DocError { request: 1, code: DocErrorCode::Cancelled });

        let (reply, f) = b.finish("app.a", 10, Kind::Save, Outcome::Save { dir: "Documents".into(), name: "new.md".into() });
        let ToApp::DocGranted { grant: save_grant, .. } = reply else { panic!("{reply:?}") };
        f.unwrap().write_all(b"saved").unwrap();
        assert!(!d.join("home/Documents/new.md").exists());
        assert!(matches!(b.request("app.b", ToShell::DocCommit { request: 10 }), Some((ToApp::DocError { code: DocErrorCode::Denied, .. }, _))), "only the app that asked commits");
        let Some((ToApp::DocGranted { grant: g, size: 5, .. }, Some(_))) = b.request("app.a", ToShell::DocCommit { request: 10 }) else { panic!() };
        assert_eq!((g, fs::read_to_string(d.join("home/Documents/new.md")).unwrap().as_str()), (save_grant, "saved"));
        assert!(b.request("app.a", ToShell::DocReopen { request: 11, grant: save_grant }).is_some_and(|(r, _)| matches!(r, ToApp::DocGranted { .. })));

        let (_, f) = b.finish("app.a", 12, Kind::Save, Outcome::Save { dir: "Documents".into(), name: "gone.md".into() });
        f.unwrap().write_all(b"x").unwrap();
        b.app_gone("app.a");
        let left: Vec<String> = fs::read_dir(d.join("home/Documents")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.contains("gone")).collect();
        assert!(left.is_empty(), "a quit app's temporary file is removed: {left:?}");
        fs::remove_dir_all(&d).unwrap();
    }

    /// A desktop shell with an app on a test channel in slot 0 and a broker over a scratch home.
    fn shell_with_app(tag: &str) -> (PathBuf, super::super::Shell, bifrost_proto::sys::Channel) {
        let (d, o) = scratch(tag);
        let mut sh = super::super::tests::desktop_shell();
        let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
        let cfg = sh.ext_cfg();
        sh.ext = vec![Some(super::super::apphost::AppHost::with_channel("is.example.editor", mine, None, cfg))];
        sh.doc_broker = Some(Some(Broker::new(o, &d.join("state/grants.json"))));
        (d, sh, peer)
    }

    /// The app side: every `Doc*` reply waiting on the channel, with its descriptor.
    fn replies(peer: &bifrost_proto::sys::Channel) -> Vec<(ToApp, Option<std::fs::File>)> {
        use std::os::fd::FromRawFd;
        let mut out = Vec::new();
        while let Ok(bifrost_proto::sys::Recv::Packet(p, mut fds)) = peer.recv(false) {
            let m = ToApp::decode(&p).unwrap();
            if matches!(m, ToApp::DocGranted { .. } | ToApp::DocError { .. }) {
                // SAFETY: the descriptor arrived with this packet and nothing else owns it
                let f = fds.pop().map(|o| unsafe { std::fs::File::from_raw_fd(std::mem::ManuallyDrop::new(o).0) });
                out.push((m, f));
            }
        }
        out
    }

    fn ask(peer: &bifrost_proto::sys::Channel, sh: &mut super::super::Shell, m: ToShell) {
        peer.send(&m.encode(), &[]).unwrap();
        sh.ext_tick(1.0);
    }

    #[test]
    fn an_app_opens_a_file_through_the_panel_and_reads_it_by_descriptor() {
        use std::io::Read;
        ALLOW.with(|a| a.set(true));
        let (d, mut sh, peer) = shell_with_app("e2e-open");
        ask(&peer, &mut sh, ToShell::DocOpen { request: 3, mode: DocMode::Read, title: "Import".into(), filters: vec!["public.text".into()] });
        assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Document));
        assert!(sh.desktop_popup_rect().is_some(), "the panel has a place on screen");
        // a press outside does not dismiss it; Down twice selects b.txt, Return opens
        sh.desktop_bar_event(super::super::Event::Down(1.0, 600.0), 2.0);
        sh.desktop_bar_event(super::super::Event::Up(1.0, 600.0), 2.0);
        assert!(sh.doc_panel.is_some(), "modal: a click elsewhere does not close it");
        for code in [108, 108, 28] {
            assert_eq!(sh.hw_menu_key(code, true, 3.0), Some(vec![]));
        }
        assert!(sh.doc_panel.is_none() && sh.desktop_menu.is_none());
        let got = replies(&peer);
        let [(ToApp::DocGranted { request: 3, ref name, mode: DocMode::Read, size: 5, .. }, Some(ref f))] = got[..] else { panic!("{got:?}") };
        assert_eq!(name, "b.txt");
        let mut s = String::new();
        (&*f).read_to_string(&mut s).unwrap();
        assert_eq!(s, "b.txt", "the app reads the chosen file through the descriptor");
        ALLOW.with(|a| a.set(false));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_path_grant_is_only_for_a_file_the_person_already_gave_the_app() {
        ALLOW.with(|a| a.set(true));
        let (d, mut sh, peer) = shell_with_app("e2e-pathgrant");
        let (chosen, other) = (d.join("home/Documents/b.txt"), d.join("home/Documents/A.md"));
        // nothing was chosen yet: the shell hands out no descriptor for a named path
        ask(&peer, &mut sh, ToShell::DocGrantPath { request: 1, path: chosen.to_string_lossy().into_owned() });
        assert!(matches!(replies(&peer)[..], [(ToApp::DocError { request: 1, code: DocErrorCode::Denied }, None)]));
        ask(&peer, &mut sh, ToShell::DocOpen { request: 3, mode: DocMode::Read, title: String::new(), filters: vec!["public.text".into()] });
        for code in [108, 108, 28] {
            sh.hw_menu_key(code, true, 3.0);
        }
        let got = replies(&peer);
        assert!(matches!(got[..], [(ToApp::DocGranted { request: 3, .. }, Some(_))]), "{got:?}");
        // the file the person chose can go into a drag; any other file of the home folder still cannot
        ask(&peer, &mut sh, ToShell::DocGrantPath { request: 4, path: chosen.to_string_lossy().into_owned() });
        assert!(matches!(replies(&peer)[..], [(ToApp::DocGranted { request: 4, mode: DocMode::Read, .. }, Some(_))]));
        ask(&peer, &mut sh, ToShell::DocGrantPath { request: 5, path: other.to_string_lossy().into_owned() });
        assert!(matches!(replies(&peer)[..], [(ToApp::DocError { request: 5, code: DocErrorCode::Denied }, None)]));
        ALLOW.with(|a| a.set(false));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_app_saves_through_the_panel_typing_the_name_and_commits() {
        use std::io::Write;
        ALLOW.with(|a| a.set(true));
        let (d, mut sh, peer) = shell_with_app("e2e-save");
        ask(&peer, &mut sh, ToShell::DocSave { request: 4, suggested_name: "Untitled.md".into(), filters: vec![] });
        // the base name "Untitled" is selected on open, so typing replaces it and keeps ".md"; then Return
        assert_eq!(sh.hw_menu_key(18, true, 1.0), None, "a letter is not a panel key: it falls through to typing");
        sh.hw_type(super::super::keyboard::Out::Text("Report".into()), 1.0);
        sh.hw_menu_key(28, true, 1.0);
        let got = replies(&peer);
        let [(ToApp::DocGranted { request: 4, grant, ref name, .. }, Some(ref f))] = got[..] else { panic!("{got:?}") };
        assert_eq!(name, "Report.md");
        (&*f).write_all(b"# notes").unwrap();
        assert!(!d.join("home/Documents/Report.md").exists(), "nothing replaced before the commit");
        ask(&peer, &mut sh, ToShell::DocCommit { request: 4 });
        let got = replies(&peer);
        assert!(matches!(got[..], [(ToApp::DocGranted { request: 4, grant: g, size: 7, .. }, Some(_))] if g == grant), "{got:?}");
        assert_eq!(fs::read_to_string(d.join("home/Documents/Report.md")).unwrap(), "# notes");
        ALLOW.with(|a| a.set(false));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_app_without_the_documents_permission_gets_no_panel() {
        ALLOW.with(|a| a.set(false));
        let (d, mut sh, peer) = shell_with_app("e2e-deny");
        ask(&peer, &mut sh, ToShell::DocOpen { request: 5, mode: DocMode::Read, title: String::new(), filters: vec![] });
        assert!(sh.doc_panel.is_none());
        assert!(matches!(replies(&peer)[..], [(ToApp::DocError { request: 5, code: DocErrorCode::Unsupported }, None)]));
        fs::remove_dir_all(&d).unwrap();
    }

    /// Developer preview: `PREVIEW_DIR=/tmp cargo test preview_doc_panel -- --ignored` writes docpanel-{open,save}-{dark,light}.png.
    #[test]
    #[ignore]
    fn preview_doc_panel() {
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        ALLOW.with(|a| a.set(true));
        for (name, light) in [("dark", false), ("light", true)] {
            for save in [false, true] {
                bifrost_render::appearance::jump_to(light);
                let (d, mut sh, peer) = shell_with_app("preview");
                sh.w = 1800;
                sh.h = 1100;
                for i in 0..14 {
                    fs::write(d.join(format!("home/Documents/Chapter {i:02}.md")), "x").unwrap();
                }
                let m = if save {
                    ToShell::DocSave { request: 1, suggested_name: "Untitled.md".into(), filters: vec![] }
                } else {
                    ToShell::DocOpen { request: 1, mode: DocMode::Read, title: String::new(), filters: vec!["public.text".into()] }
                };
                ask(&peer, &mut sh, m);
                for code in [108, 108, 108] {
                    sh.hw_menu_key(code, true, 1.0);
                }
                let (w, h) = (sh.w as usize, sh.h as usize);
                let mut img = crate::gfx::Image::new(w, h);
                sh.draw_desktop_background(&mut crate::gfx::Surface { w, h, stride: w, px: &mut img.px, y0: 0 });
                let kind = if save { "save" } else { "open" };
                fs::write(Path::new(&dir).join(format!("docpanel-{kind}-{name}.png")), super::super::screenshot::encode(&img)).unwrap();
                fs::remove_dir_all(&d).unwrap();
            }
        }
        ALLOW.with(|a| a.set(false));
    }

    fn open_panel(o: &Opener, opts: Options, filters: Vec<String>) -> Panel {
        Panel::new(0, 1, Kind::Open(DocMode::Read), "", filters, "", o).with_options(opts, o)
    }

    #[test]
    fn options_come_from_the_wire_bits_and_a_panel_that_chooses_nothing_chooses_files() {
        use bifrost_proto::docopt::*;
        assert_eq!(Options::from_bits(0), Options::default());
        assert_eq!(Options::from_bits(DIRECTORIES), Options { multiple: false, files: false, dirs: true, create_dirs: false });
        assert_eq!(Options::from_bits(MULTIPLE | FILES | DIRECTORIES | CREATE_DIRS), Options { multiple: true, files: true, dirs: true, create_dirs: true });
    }

    #[test]
    fn several_files_are_marked_by_clicks_and_chosen_in_list_order() {
        let (d, o) = scratch("multi");
        let mut p = open_panel(&o, Options { multiple: true, ..Options::default() }, vec![]);
        let l = p.layout();
        let row = |i: usize| l.list.y + l.row_h * (i as f32 + 0.5);
        let x = l.list.x + 10.0;
        p.click(x, row(1), 0.0, &o);
        p.click(x, row(0), 1000.0, &o);
        assert_eq!(p.chosen(), [0, 1]);
        p.click(x, row(1), 3000.0, &o);
        assert_eq!(p.chosen(), [0], "a second click takes the row out");
        p.click(x, row(1), 5000.0, &o);
        assert_eq!(p.key(28, &o), Some(Some(Outcome::OpenMany(vec!["Documents/A.md".into(), "Documents/b.txt".into()]))));
        p.marked.insert(2);
        assert!(!p.can_confirm(), "a folder among files cannot be chosen when folders are not");
        p.marked.remove(&2);
        p.select_all();
        assert_eq!(p.chosen(), [0, 1, 3], "select all skips folders the panel cannot choose");
        // one file in a panel that allows several is still a list
        p.select(1);
        assert_eq!(p.confirm(&o), Some(Outcome::OpenMany(vec!["Documents/b.txt".into()])));
        // a single-selection panel ignores the toggle
        let mut one = open_panel(&o, Options::default(), vec![]);
        one.toggle(0);
        one.toggle(1);
        assert_eq!(one.chosen(), [1]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn folders_can_be_chosen_when_the_panel_allows_it() {
        let (d, o) = scratch("dirs");
        let mut p = open_panel(&o, Options { files: false, dirs: true, ..Options::default() }, vec![]);
        assert!(p.entries.iter().filter(|e| !e.dir).all(|e| !e.enabled), "files are dimmed in a folders-only panel");
        assert!(p.can_confirm(), "nothing selected chooses the folder shown");
        assert_eq!(p.confirm(&o), Some(Outcome::Open("Documents".into())));
        p.select(2);
        assert_eq!(p.confirm(&o), Some(Outcome::Open("Documents/Drafts".into())));
        p.select(3);
        assert!(!p.can_confirm(), "a dimmed file");
        let l = p.layout();
        let at = |i: usize| (l.list.x + 10.0, l.list.y + l.row_h * (i as f32 + 0.5));
        p.click(at(2).0, at(2).1, 0.0, &o);
        assert_eq!(p.click(at(2).0, at(2).1, 100.0, &o), None, "a double click goes into the folder");
        assert_eq!(p.dir, Path::new("Documents/Drafts"));
        // files and folders together
        let mut both = open_panel(&o, Options { dirs: true, ..Options::default() }, vec![]);
        both.select(2);
        assert_eq!(both.confirm(&o), Some(Outcome::Open("Documents/Drafts".into())));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_package_the_app_declares_is_one_file_row_that_is_chosen_not_opened() {
        let (d, o) = scratch("pkg");
        fs::create_dir_all(d.join("home/Documents/Notes.rtfd")).unwrap();
        fs::write(d.join("home/Documents/Notes.rtfd/TXT.rtf"), "{\\rtf1}").unwrap();
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o).with_packages(vec!["rtfd".into()], &o);
        let i = p.entries.iter().position(|e| e.name == "Notes.rtfd").unwrap();
        assert!(!p.entries[i].dir && p.entries[i].enabled, "a package is a file to the panel");
        assert!(p.entries.iter().find(|e| e.name == "Drafts").unwrap().dir, "other folders stay folders");
        p.select(i);
        assert!(p.can_confirm());
        assert_eq!(p.confirm(&o), Some(Outcome::Open("Documents/Notes.rtfd".into())));
        // a double click chooses it too, instead of going in
        let l = p.layout();
        let at = |i: usize| (l.list.x + 10.0, l.list.y + l.row_h * (i as f32 + 0.5));
        p.click(at(i).0, at(i).1, 0.0, &o);
        assert_eq!(p.click(at(i).0, at(i).1, 100.0, &o), Some(Outcome::Open("Documents/Notes.rtfd".into())));
        assert_eq!(p.dir, Path::new("Documents"), "never navigated into");
        // an app that declares no package types navigates the folder as before
        let mut plain = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        assert!(plain.entries[i].dir);
        plain.select(i);
        assert_eq!(plain.confirm(&o), None);
        assert_eq!(plain.dir, Path::new("Documents/Notes.rtfd"));
        // a folder-only panel cannot choose a package, which stays dimmed
        let dirs = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o).with_options(Options { files: false, dirs: true, ..Options::default() }, &o).with_packages(vec!["rtfd".into()], &o);
        assert!(!dirs.entries.iter().find(|e| e.name == "Notes.rtfd").unwrap().enabled);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn new_folder_takes_a_name_and_selects_the_folder() {
        let (d, o) = scratch("newdir");
        let plain = open_panel(&o, Options::default(), vec![]);
        assert!(!plain.can_create_dir(), "an open panel offers New Folder only when asked");
        let mut p = open_panel(&o, Options { create_dirs: true, ..Options::default() }, vec![]);
        let l = p.layout();
        p.click(l.new_folder.x + 1.0, l.new_folder.y + 1.0, 0.0, &o);
        assert_eq!(p.creating.as_deref(), Some("untitled folder"));
        p.type_text("/2");
        p.backspace();
        p.type_text("b");
        assert_eq!(p.creating.as_deref(), Some("untitled folderb"));
        assert_eq!(p.key(1, &o), Some(None), "Esc closes the field, not the panel");
        assert!(p.creating.is_none());
        p.begin_new_folder();
        p.creating = Some("Drafts".into());
        assert!(!p.commit_new_folder(&o), "the name exists");
        assert!(p.creating.is_some());
        p.creating = Some("Taxes".into());
        assert_eq!(p.key(28, &o), Some(None));
        assert!(d.join("home/Documents/Taxes").is_dir());
        assert_eq!(p.entries[p.selected.unwrap()].name, "Taxes");
        // a save panel always has it
        let save = Panel::new(0, 1, Kind::Save, "", vec![], "x.md", &o);
        assert!(save.can_create_dir());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_sidebar_lists_places_and_recents_shows_the_apps_documents() {
        let (d, o) = scratch("places");
        fs::create_dir_all(d.join("home/Downloads")).unwrap();
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o).with_recents(vec!["Documents/b.txt".into(), "Gone/x.txt".into(), "Documents/Drafts".into()], &o);
        let labels: Vec<&str> = p.places.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["Recents", "Home", "Documents", "Downloads"], "folders that do not exist are not listed");
        assert_eq!(p.active_place(), Some(2));
        let l = p.layout();
        p.click(l.sidebar.x + 5.0, l.sidebar.y + 0.5 * l.sidebar_row_h, 0.0, &o);
        assert!(p.in_recents && p.active_place() == Some(0));
        assert_eq!((p.location(), p.can_go_up()), ("Recents".into(), false));
        assert_eq!(names(&p), ["b.txt", "Drafts"], "newest first; a document that is gone is left out");
        p.select(0);
        assert_eq!(p.confirm(&o), Some(Outcome::Open("Documents/b.txt".into())));
        p.select(1);
        assert_eq!(p.confirm(&o), None, "a folder in Recents opens it");
        assert!(!p.in_recents && p.dir == Path::new("Documents/Drafts"));
        p.click(l.sidebar.x + 5.0, l.sidebar.y + 3.5 * l.sidebar_row_h, 1000.0, &o);
        assert_eq!(p.dir, Path::new("Downloads"));
        p.click(l.sidebar.x + 5.0, l.sidebar.y + 1.5 * l.sidebar_row_h, 2000.0, &o);
        assert_eq!((p.location(), p.active_place()), ("Home".into(), Some(1)));
        assert!(p.go_to(Path::new("Documents"), &o) && !p.go_to(Path::new("Nowhere"), &o));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_broker_answers_several_files_as_a_batch_and_remembers_recents() {
        let (d, o) = scratch("batch");
        let mut b = Broker::new(o, &d.join("state/grants.json"));
        let out = Outcome::OpenMany(vec!["Documents/A.md".into(), "Documents/Nope".into(), "Documents/Drafts".into()]);
        let got = b.finish_many("app.a", 6, Kind::Open(DocMode::Read), out);
        assert!(matches!(got[0], (ToApp::DocBatch { request: 6, count: 2 }, None)), "{:?}", got.iter().map(|g| &g.0).collect::<Vec<_>>());
        assert_eq!(got.len(), 3);
        let ToApp::DocGranted { ref name, .. } = got[2].0 else { panic!() };
        assert!(got[2].1.as_ref().unwrap().metadata().unwrap().is_dir() && name == "Drafts", "a folder is granted as its descriptor");
        let all_bad = b.finish_many("app.a", 7, Kind::Open(DocMode::Read), Outcome::OpenMany(vec!["Nope".into()]));
        assert!(matches!(all_bad[..], [(ToApp::DocError { request: 7, code: DocErrorCode::NotFound }, None)]));
        assert_eq!(b.recents("app.a").len(), 2);
        assert!(b.recents("app.b").is_empty(), "recents are per app");
        let single = b.finish_many("app.a", 8, Kind::Open(DocMode::Read), Outcome::Open("Documents/b.txt".into()));
        assert!(matches!(single[..], [(ToApp::DocGranted { request: 8, .. }, Some(_))]));
        assert_eq!(b.recents("app.a")[0], Path::new("Documents/b.txt"));
        fs::remove_dir_all(&d).unwrap();
    }
}

impl super::Shell {
    /// Image > Edit With: opens the photo at `path` in the app `key` with a write grant, so the app saves the original
    /// file in place. Photos picks the change up on its next scan. False when the app or the file cannot be opened.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn photo_edit_with(&mut self, key: &str, path: &Path, t: f64) -> bool {
        use super::apps::{apps, AppId};
        let Some(def) = apps().iter().find(|a| a.icon == key).map(|a| a.id) else { return false };
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let Some(broker) = self.doc_broker() else { return false };
        // grants name files relative to the account's home
        let Ok(rel) = path.strip_prefix(broker.opener.home()).map(Path::to_path_buf) else { return false };
        let Ok(file) = broker.opener.open_any(&rel, DocMode::Write) else { return false };
        let size = file.metadata().map_or(0, |m| m.len());
        let Ok(grant) = broker.grants.add(key, &rel, DocMode::Write, &file) else { return false };
        self.desk_open_or_start(def, t);
        if let AppId::External(slot) = def {
            let request = self.app_share.next_inbound();
            self.doc_send(slot, ToApp::DocGranted { request, grant, mode: DocMode::Write, name, size }, Some(file));
        }
        true
    }

    /// Image > Edit With for the photo open in the viewer.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn photo_edit_with_viewed(&mut self, key: &str, t: f64) {
        let Some(path) = self.photos.as_ref().and_then(|p| p.viewing()).map(|p| p.path.clone()) else { return };
        self.photo_edit_with(key, &path, t);
    }
}
