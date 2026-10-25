//! The open and save panel (`NativeOpenPanel` / `NativeSavePanel`), phase 3 of `docs/file-portal-design.md`.
//!
//! An app asks with `ToShell::DocOpen` or `DocSave`; the shell shows this panel as a sheet that hangs from under the
//! window's title bar (or toolbar band) and dims the window. The app never sees the listing. When the user chooses, the
//! broker (`bare::docbroker`) opens the file as the account and the app receives one descriptor (`ToApp::DocGranted`).
//!
//! Behaviour follows the Golden Gate panels: it starts in Documents, lists names alphabetically (case-insensitive), files the
//! filters do not allow are shown dimmed and cannot be chosen, Return or a double click is the default button, Esc
//! cancels, Left goes up a folder (Cmd+Up in a save panel, whose Left and Right move the caret in the name field).
//!
//! The toolbar row has back and forward (the folders visited in this panel), the list or icon view switcher, the location
//! pop-up (the folder and the folders that enclose it) and a search field that filters the folder shown by name.
//! The list and the icon grid scroll with the wheel (`Panel::wheel`, `Shell::doc_wheel`) and show each item's file-type
//! icon (`FileKind`, from its content type). The panel is as wide as its window allows (`fit_width`: the window's
//! width less a margin, between `MIN_W` and `MAX_W`), and follows the window as it is resized.
//! The save panel's name field is a real text field (the kit's `TextBuffer`): caret, selection, Shift and Option
//! movement, Cmd+A, Cut, Copy and Paste.
//!
//! `NativeOpenPanel` options (`bifrost_proto::docopt`): several files (`allowsMultipleSelection`; a click toggles a row),
//! folders (`canChooseDirectories`, `canChooseFiles`) and New Folder (`canCreateDirectories`, always on in a save
//! panel). The sidebar lists Recents (the app's recent grants, newest first), then Favorites (the common folders that
//! exist) and Locations (Home). The folder last used by the app is where its next panel starts.

use crate::bare::docbroker::{self, valid_name, Grants, Opener, PendingSave};
use bifrost_proto::{DocErrorCode, DocMode, GrantId, ToApp, ToShell};
use bifrost_ui::text::TextBuffer;
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

/// The sidebar group a place belongs to: Recents stands alone at the top, then Favorites and Locations under headings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Top,
    Favorites,
    Locations,
}

impl Section {
    pub fn heading(self) -> Option<&'static str> {
        match self {
            Section::Top => None,
            Section::Favorites => Some("Favorites"),
            Section::Locations => Some("Locations"),
        }
    }
}

/// A sidebar row: Recents (`path` is `None`) or a folder in the home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: Option<PathBuf>,
    pub section: Section,
}

/// A row of the sidebar as drawn: a group heading or the place at this index of `places`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideRow {
    Heading(&'static str),
    Place(usize),
}

/// Where the panel is: Recents or a folder of the home. Back and forward step through these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    Recents,
    Dir(PathBuf),
}

/// The text field with the keyboard: the save panel's name, the search field, or none (the list takes the keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Name,
    Search,
    List,
}

/// Recents shows at most this many documents.
pub const MAX_RECENTS: usize = 20;
/// The `Panel::slot` of a panel the shell itself asks for (Settings > Wallpaper > Add Photo…): no app slot is this high.
/// Its answer goes to the shell (`Shell::wallpaper_panel_done`), and the sheet hangs from the Settings window.
pub const SHELL_SLOT: u8 = u8::MAX;

/// The window a panel of `slot` hangs from: the app's, or Settings' for a shell-owned panel.
pub fn parent_app(slot: u8) -> super::AppId {
    if slot == SHELL_SLOT { super::AppId::Settings } else { super::AppId::External(slot) }
}

/// How many folders back and forward remember.
const MAX_HISTORY: usize = 50;

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
/// One item of the icon view, in points.
pub const CELL_W: f32 = 96.0;
pub const CELL_H: f32 = 92.0;
/// The file-type icon of an icon-view cell and of a list row, in points.
const ICON_PT: f32 = 52.0;
const ROW_ICON_PT: f32 = 18.0;
/// Clicks on the same row closer than this are a double click (ms).
const DOUBLE_CLICK_MS: f64 = 400.0;
/// The panel's width in points before the shell fits it to its window (`fit_width`), and the range it is fitted to:
/// narrow enough to leave the window's edges in view, wide enough for the sidebar, the toolbar row and the buttons.
pub const DEFAULT_W: f32 = 720.0;
pub const MIN_W: f32 = 620.0;
pub const MAX_W: f32 = 960.0;
/// How much narrower than its window the panel is, both edges together (points).
const WINDOW_INSET: f32 = 48.0;
/// The drag the shell emulates for a wheel event ends this soon after it (ms); the panel drops its release.
const WHEEL_DRAG_MS: f64 = 250.0;

/// What an item's icon shows, from its content type (`uttype`): the kinds `workspace::icon_for_content_type` names
/// glyphs for (folder, photo, film, music, PDF, archive, text), else a plain document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Image,
    Movie,
    Audio,
    Pdf,
    Archive,
    Text,
    Document,
}

impl FileKind {
    /// The kind of `e`: a folder, else the kind its name's content type conforms to.
    pub fn of(e: &Entry) -> FileKind {
        if e.dir {
            return FileKind::Folder;
        }
        const TABLE: [(&str, FileKind); 6] = [
            ("public.image", FileKind::Image),
            ("public.movie", FileKind::Movie),
            ("public.audio", FileKind::Audio),
            ("com.adobe.pdf", FileKind::Pdf),
            ("public.archive", FileKind::Archive),
            ("public.text", FileKind::Text),
        ];
        let Some(ty) = bifrost_ui::uttype::type_for_extension(&e.name) else { return FileKind::Document };
        TABLE.iter().find(|(t, _)| bifrost_ui::uttype::conforms(ty, t)).map_or(FileKind::Document, |(_, k)| *k)
    }

    /// The colour of the folder, or of the band across a document's page (system colours; red stays for destructive
    /// actions).
    pub fn tint(self) -> bifrost_render::appearance::Pair {
        use super::ui::system::{SYSTEM_BLUE, SYSTEM_BROWN, SYSTEM_GRAY, SYSTEM_GREEN, SYSTEM_INDIGO, SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE};
        match self {
            FileKind::Folder => SYSTEM_BLUE,
            FileKind::Image => SYSTEM_GREEN,
            FileKind::Movie => SYSTEM_PURPLE,
            FileKind::Audio => SYSTEM_PINK,
            FileKind::Pdf => SYSTEM_ORANGE,
            FileKind::Archive => SYSTEM_BROWN,
            FileKind::Text => SYSTEM_INDIGO,
            FileKind::Document => SYSTEM_GRAY,
        }
    }
}

/// The panel's width for a parent window `parent` points wide on a screen `screen` points wide: the window's width less
/// `WINDOW_INSET`, kept between `MIN_W` and `MAX_W`, and never wider than the screen.
pub fn fit_width(parent: f32, screen: f32) -> f32 {
    let w = if parent.is_finite() { (parent - WINDOW_INSET).max(MIN_W).min(MAX_W) } else { DEFAULT_W };
    if screen.is_finite() && screen > 0.0 {
        w.min(screen)
    } else {
        w
    }
}

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
    pub forward: R,
    /// the view switcher: list, then icons
    pub view_list: R,
    pub view_icons: R,
    /// the location pop-up
    pub location: R,
    pub search: R,
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
    /// what the list shows: the folder's rows that match the search
    pub entries: Vec<Entry>,
    /// every row of the folder, before the search
    all: Vec<Entry>,
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
    /// the save panel's file name, with its caret and selection
    pub name: TextBuffer,
    /// the search field's text: the list shows the rows whose name contains it (any case)
    pub search: TextBuffer,
    /// which field has the keyboard
    pub focus: Focus,
    /// the icon view instead of the list
    pub icons: bool,
    /// the location pop-up is open
    pub loc_menu: bool,
    /// where back and forward go, nearest last
    back_stack: Vec<Loc>,
    fwd_stack: Vec<Loc>,
    /// first row shown (the first item of a row of icons in the icon view)
    pub top: usize,
    last_click: Option<(usize, f64)>,
    /// the last click in a text field (field, time), for a double click that selects a word
    last_field_click: Option<(Focus, f64)>,
    /// the panel's width in points (`fit_width`, from its window)
    pub width: f32,
    /// a wheel event came at this time (ms): the release of the drag the shell emulates for it is not a click
    wheel_at: Option<f64>,
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
        let name: String = if kind == Kind::Save { suggested.chars().filter(|c| *c != '/' && !c.is_control()).take(255).collect() } else { String::new() };
        let mut p = Panel {
            slot,
            request,
            kind,
            title,
            filters,
            dir: PathBuf::new(),
            entries: Vec::new(),
            all: Vec::new(),
            selected: None,
            marked: BTreeSet::new(),
            opts: Options::default(),
            packages: Vec::new(),
            places: Vec::new(),
            in_recents: false,
            recents: Vec::new(),
            creating: None,
            name: TextBuffer::from_text(&name),
            search: TextBuffer::new(),
            focus: if kind == Kind::Save { Focus::Name } else { Focus::List },
            icons: false,
            loc_menu: false,
            back_stack: Vec::new(),
            fwd_stack: Vec::new(),
            top: 0,
            last_click: None,
            last_field_click: None,
            width: DEFAULT_W,
            wheel_at: None,
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

    /// The save panel's file name.
    pub fn name(&self) -> &str {
        self.name.text()
    }

    /// The selected part of the name (byte offsets), when something is selected.
    pub fn name_sel(&self) -> Option<std::ops::Range<usize>> {
        self.name.has_selection().then(|| self.name.selection())
    }

    /// Selects the base name in the name field (everything before the last dot, or the whole name when there is no
    /// extension), as a save panel opens on it: typing replaces the name and keeps the extension.
    pub fn select_base_name(&mut self) {
        if self.kind != Kind::Save {
            return;
        }
        let name = self.name.text();
        let end = name.rfind('.').filter(|i| *i > 0).unwrap_or(name.len());
        self.name.set_selection(0, end);
    }

    /// Replaces the whole name (a file chosen in the list), with its base name selected.
    fn set_name(&mut self, name: &str) {
        self.name.set_text(name);
        self.select_base_name();
    }

    /// Recents, then Favorites (the common folders that exist) and Locations (Home).
    fn places_in(opener: &Opener) -> Vec<Place> {
        let mut v = vec![Place { label: "Recents".into(), path: None, section: Section::Top }];
        let top = opener.list(Path::new("")).unwrap_or_default();
        for name in ["Desktop", "Documents", "Downloads"] {
            if top.iter().any(|(n, d)| *d && n == name) {
                v.push(Place { label: name.into(), path: Some(name.into()), section: Section::Favorites });
            }
        }
        v.push(Place { label: "Home".into(), path: Some(PathBuf::new()), section: Section::Locations });
        v
    }

    /// The sidebar as drawn: each group's heading before its first place.
    pub fn sidebar_rows(&self) -> Vec<SideRow> {
        let mut rows = Vec::new();
        let mut last = None;
        for (i, pl) in self.places.iter().enumerate() {
            if last != Some(pl.section) {
                if let Some(h) = pl.section.heading() {
                    rows.push(SideRow::Heading(h));
                }
                last = Some(pl.section);
            }
            rows.push(SideRow::Place(i));
        }
        rows
    }

    /// The sidebar row (from the top) where place `i` is drawn.
    pub fn sidebar_row_of(&self, i: usize) -> Option<usize> {
        self.sidebar_rows().iter().position(|r| *r == SideRow::Place(i))
    }

    /// What an open panel may choose (`allowsMultipleSelection`, `canChooseDirectories`, ...).
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

    /// Shows folder `dir` when it can be read (the folder the app used last). Not a step back can return to.
    pub fn go_to(&mut self, dir: &Path, opener: &Opener) -> bool {
        if opener.list(dir).is_err() {
            return false;
        }
        self.dir = dir.to_path_buf();
        self.in_recents = false;
        self.load(opener);
        true
    }

    /// Where the panel is now.
    pub fn loc(&self) -> Loc {
        if self.in_recents {
            Loc::Recents
        } else {
            Loc::Dir(self.dir.clone())
        }
    }

    fn show(&mut self, loc: Loc, opener: &Opener) {
        match loc {
            Loc::Recents => self.in_recents = true,
            Loc::Dir(d) => {
                self.in_recents = false;
                self.dir = d;
            }
        }
        self.creating = None;
        self.loc_menu = false;
        // a search belongs to the folder it was typed in
        self.search.set_text("");
        if self.focus == Focus::Search {
            self.focus = if self.kind == Kind::Save { Focus::Name } else { Focus::List };
        }
        self.load(opener);
    }

    /// Goes to `loc`, remembering where the panel was for Back. Going where it already is does nothing.
    pub fn navigate(&mut self, loc: Loc, opener: &Opener) {
        let here = self.loc();
        if loc == here {
            self.loc_menu = false;
            return;
        }
        self.back_stack.push(here);
        if self.back_stack.len() > MAX_HISTORY {
            self.back_stack.remove(0);
        }
        self.fwd_stack.clear();
        self.show(loc, opener);
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.fwd_stack.is_empty()
    }

    /// Back: the folder shown before (folders that went away are skipped).
    pub fn go_back(&mut self, opener: &Opener) -> bool {
        while let Some(loc) = self.back_stack.pop() {
            if let Loc::Dir(d) = &loc {
                if opener.list(d).is_err() {
                    continue;
                }
            }
            self.fwd_stack.push(self.loc());
            self.show(loc, opener);
            return true;
        }
        false
    }

    /// Forward: undoes a Back.
    pub fn go_forward(&mut self, opener: &Opener) -> bool {
        while let Some(loc) = self.fwd_stack.pop() {
            if let Loc::Dir(d) = &loc {
                if opener.list(d).is_err() {
                    continue;
                }
            }
            self.back_stack.push(self.loc());
            self.show(loc, opener);
            return true;
        }
        false
    }

    /// The sidebar row that matches what is shown.
    pub fn active_place(&self) -> Option<usize> {
        self.places.iter().position(|pl| if self.in_recents { pl.path.is_none() } else { pl.path.as_deref() == Some(self.dir.as_path()) })
    }

    pub fn go_place(&mut self, i: usize, opener: &Opener) {
        let Some(pl) = self.places.get(i).cloned() else { return };
        self.creating = None;
        self.navigate(pl.path.map_or(Loc::Recents, Loc::Dir), opener);
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
        self.all = entries;
        self.filter();
    }

    /// Shows the rows that match the search; the selection and the scroll start over.
    fn filter(&mut self) {
        let q = self.search.text().trim().to_lowercase();
        self.entries = self.all.iter().filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q)).cloned().collect();
        self.marked.clear();
        self.selected = None;
        self.top = 0;
        self.last_click = None;
    }

    /// The folder's name for the location pop-up ("Home" at the top).
    pub fn location(&self) -> String {
        if self.in_recents {
            return "Recents".into();
        }
        self.dir.file_name().map_or_else(|| "Home".into(), |n| n.to_string_lossy().into_owned())
    }

    /// The location pop-up's rows: the folder shown, then each folder that encloses it, up to Home.
    pub fn location_items(&self) -> Vec<(String, Loc)> {
        if self.in_recents {
            return vec![("Recents".into(), Loc::Recents)];
        }
        let mut v = Vec::new();
        let mut d = self.dir.clone();
        loop {
            let label = d.file_name().map_or_else(|| "Home".into(), |n| n.to_string_lossy().into_owned());
            v.push((label, Loc::Dir(d.clone())));
            if !d.pop() {
                break;
            }
        }
        v
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
        let mut parent = self.dir.clone();
        parent.pop();
        self.navigate(Loc::Dir(parent), opener);
        if let Some(i) = self.entries.iter().position(|e| e.name == left) {
            self.select(i);
        }
    }

    fn enter(&mut self, i: usize, opener: &Opener) {
        if let Some(e) = self.entries.get(i).filter(|e| e.dir) {
            let to = self.path_of(e);
            self.navigate(Loc::Dir(to), opener);
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

    /// Icons per row of the icon view (1 in the list).
    pub fn columns(&self) -> usize {
        if !self.icons {
            return 1;
        }
        ((self.layout().list.w / CELL_W).floor() as usize).max(1)
    }

    /// Items the list or the icon grid shows at once.
    pub fn visible(&self) -> usize {
        let offset = self.row_offset();
        if !self.icons {
            return VISIBLE_ROWS - offset;
        }
        let l = self.layout();
        let rows = (((l.list.h - offset as f32 * l.row_h) / CELL_H).floor() as usize).max(1);
        rows * self.columns()
    }

    /// Selects row `i` and scrolls it into view. In a save panel a file's name goes into the field.
    pub fn select(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        self.selected = Some(i);
        self.marked = BTreeSet::from([i]);
        let (cols, vis) = (self.columns(), self.visible());
        if i < self.top {
            self.top = i - i % cols;
        } else if i >= self.top + vis {
            let row = i / cols;
            let rows = vis / cols;
            self.top = (row + 1 - rows) * cols;
        }
        if self.kind == Kind::Save && !self.entries[i].dir {
            let name = self.entries[i].name.clone();
            self.set_name(&name);
        }
    }

    /// Up/Down: the next row (a row of icons in the icon view), stopping at the ends.
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

    /// Fits the panel to a new width (points), keeping the first item shown at the start of a row of icons.
    pub fn set_width(&mut self, w: f32) {
        if (w - self.width).abs() < 0.5 || !w.is_finite() || w <= 0.0 {
            return;
        }
        self.width = w;
        self.top -= self.top % self.columns();
    }

    /// `scrollWheel:` at (`x`, `y`) points in the panel at `t` ms: the content moved `dy` points (negative for a wheel
    /// turned down, which shows the items below). Over the list or the icon grid it scrolls a row (of icons) for each
    /// row height of travel, at least one a notch, between the first item and the last. The release of the drag the
    /// shell emulates for the wheel is then not a click (`swallows`). True when the items moved.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32, t: f64) -> bool {
        self.wheel_at = Some(t);
        let l = self.layout();
        if self.loc_menu || !l.list.contains(x, y) || !dy.is_finite() || dy == 0.0 {
            return false;
        }
        let cols = self.columns();
        let step = if self.icons { CELL_H } else { l.row_h };
        let rows = (-dy / step).round();
        let rows = if rows == 0.0 { -dy.signum() } else { rows };
        let shown = (self.visible() / cols).max(1);
        let most = self.entries.len().div_ceil(cols).saturating_sub(shown) as i64;
        let row = ((self.top / cols) as i64 + rows as i64).clamp(0, most) as usize;
        let before = self.top;
        self.top = row * cols;
        self.top != before
    }

    /// True for the release of the drag the shell emulated for a wheel event (`wheel`) at `t` ms: it is dropped, not
    /// taken as a click. Asked once per release.
    pub fn swallows(&mut self, t: f64) -> bool {
        matches!(self.wheel_at.take(), Some(at) if t >= at && t - at <= WHEEL_DRAG_MS)
    }

    /// Whether the default button (Open or Save) would do something now.
    pub fn can_confirm(&self) -> bool {
        if self.creating.is_some() {
            return true;
        }
        match self.kind {
            Kind::Save => self.selected.and_then(|i| self.entries.get(i)).is_some_and(|e| e.dir) || valid_name(self.name()),
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
            (Kind::Save, _) => Some(Outcome::Save { dir: self.dir.clone(), name: self.name().to_string() }),
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

    /// The text field with the keyboard, if any (the New Folder field is handled apart).
    fn field_mut(&mut self) -> Option<&mut TextBuffer> {
        match self.focus {
            Focus::Name if self.kind == Kind::Save => Some(&mut self.name),
            Focus::Search => Some(&mut self.search),
            _ => None,
        }
    }

    /// A text field has the keyboard: Left, Right, Home, End and Backspace edit it instead of moving in the list.
    pub fn field_focused(&self) -> bool {
        self.creating.is_none() && match self.focus {
            Focus::Name => self.kind == Kind::Save,
            Focus::Search => true,
            Focus::List => false,
        }
    }

    /// Typing (`/` and control characters are dropped; at most 255 bytes). It goes to the New Folder field while that is
    /// open, else to the field with the keyboard; in an open panel with no field focused it starts a search.
    pub fn type_text(&mut self, text: &str) -> bool {
        if let Some(n) = self.creating.as_mut() {
            for c in text.chars().filter(|c| *c != '/' && !c.is_control()) {
                if n.len() + c.len_utf8() <= 255 {
                    n.push(c);
                }
            }
            return true;
        }
        let typed: String = text.chars().filter(|c| *c != '/' && !c.is_control()).collect();
        if typed.is_empty() {
            return !text.is_empty() && self.field_focused();
        }
        if !self.field_focused() {
            if self.kind == Kind::Save {
                self.focus = Focus::Name;
            } else {
                self.focus = Focus::Search;
            }
        }
        let search = self.focus == Focus::Search;
        let Some(f) = self.field_mut() else { return false };
        let room = 255usize.saturating_sub(f.len() - f.selection().len());
        let mut fit = String::new();
        for c in typed.chars() {
            if fit.len() + c.len_utf8() > room {
                break;
            }
            fit.push(c);
        }
        f.insert_text(&fit);
        if search {
            self.filter();
        }
        true
    }

    /// Backspace: the New Folder field, else the focused field (the selection, or the character before the caret).
    pub fn backspace(&mut self) -> bool {
        if let Some(n) = self.creating.as_mut() {
            n.pop();
            return true;
        }
        let search = self.focus == Focus::Search;
        let Some(f) = self.field_mut() else { return false };
        let changed = f.delete_backward().is_some();
        if search && changed {
            self.filter();
        }
        changed
    }

    /// An editing key for the focused field, as the shell's text fields name them (`keyboard::Out`): movement with
    /// Shift and Option, Home and End, Delete, Select All, Cut, Copy and Paste. True when the panel took it.
    pub fn edit(&mut self, out: &super::keyboard::Out) -> bool {
        use super::keyboard::Out;
        match out {
            Out::Text(t) => return self.type_text(t),
            Out::Backspace => {
                self.backspace();
                return true;
            }
            _ => {}
        }
        if self.creating.is_some() {
            return false;
        }
        let Out::Named(name) = out else { return false };
        let name = *name;
        let search = self.focus == Focus::Search;
        // Select All in the list of a panel that allows several files
        if name == "SelectAll" && !self.field_focused() {
            self.select_all();
            return true;
        }
        let Some(f) = self.field_mut() else { return false };
        let mut changed = false;
        match name {
            "Left" => f.move_char(false, false),
            "Right" => f.move_char(true, false),
            "ShiftLeft" => f.move_char(false, true),
            "ShiftRight" => f.move_char(true, true),
            "WordLeft" => f.move_word(false, false),
            "WordRight" => f.move_word(true, false),
            "ShiftWordLeft" => f.move_word(false, true),
            "ShiftWordRight" => f.move_word(true, true),
            "Home" => f.move_document(false, false),
            "End" => f.move_document(true, false),
            "ShiftHome" => f.move_document(false, true),
            "ShiftEnd" => f.move_document(true, true),
            "SelectAll" => f.select_all(),
            "DeleteForward" => changed = f.delete_forward().is_some(),
            "DeleteWord" => changed = f.delete_word_backward().is_some(),
            "DeleteWordForward" => changed = f.delete_word_forward().is_some(),
            "DeleteLine" => {
                let caret = f.caret();
                changed = f.replace_range(0..caret, "").is_some();
            }
            "Copy" => {
                if f.has_selection() {
                    super::clipboard::set_text(f.selected_text());
                }
            }
            "Cut" => {
                if f.has_selection() {
                    super::clipboard::set_text(f.selected_text());
                    changed = f.insert_text("").is_some();
                }
            }
            "Paste" => {
                // one line, without the characters a file name cannot hold
                let text: String = super::clipboard::text().unwrap_or_default().lines().next().unwrap_or("").chars().filter(|c| *c != '/' && !c.is_control()).collect();
                let room = 255usize.saturating_sub(f.len() - f.selection().len());
                let mut fit = String::new();
                for c in text.chars() {
                    if fit.len() + c.len_utf8() > room {
                        break;
                    }
                    fit.push(c);
                }
                changed = f.insert_text(&fit).is_some();
            }
            _ => return false,
        }
        if search && changed {
            self.filter();
        }
        true
    }

    /// A click in a text field placed the caret at byte `at` (the shell measures the text), at time `t` ms. A double
    /// click selects the word there.
    pub fn click_field(&mut self, field: Focus, at: usize, t: f64) {
        if self.creating.is_some() || (field == Focus::Name && self.kind != Kind::Save) {
            return;
        }
        self.loc_menu = false;
        self.focus = field;
        let double = self.last_field_click.is_some_and(|(f, t0)| f == field && t - t0 <= DOUBLE_CLICK_MS);
        self.last_field_click = if double { None } else { Some((field, t)) };
        let Some(f) = self.field_mut() else { return };
        if double {
            f.select_word_at(at);
        } else {
            f.set_selection(at, at);
        }
    }

    /// The panel's geometry in points. The width follows the window the panel hangs from (`width`, `fit_width`): the
    /// location pop-up and the list take what the width leaves; the row count is a fixed token.
    pub fn layout(&self) -> Layout {
        use super::ui::{SPACE_M, SPACE_S, SPACE_XL, SPACE_XXS};
        let w = if self.width.is_finite() && self.width > 0.0 { self.width } else { DEFAULT_W };
        let (row_h, bar_h, button_w, side_w, search_w) = (28.0, 28.0, 96.0, 150.0, 150.0);
        let pad = SPACE_XL;
        let title = R { x: pad, y: pad, w: w - 2.0 * pad, h: 20.0 };
        let mut y = title.y + title.h + SPACE_M;
        let field = (self.kind == Kind::Save).then(|| {
            let f = R { x: pad + 72.0, y, w: w - 2.0 * pad - 72.0, h: bar_h };
            y += bar_h + SPACE_M;
            f
        });
        // the toolbar row: back and forward, the view switcher, the location pop-up and the search field; the sidebar
        // runs along the left from this row down to the end of the list
        let x0 = pad + side_w + SPACE_M;
        let back = R { x: x0, y, w: bar_h, h: bar_h };
        let forward = R { x: back.x + bar_h + SPACE_XXS, y, w: bar_h, h: bar_h };
        let view_list = R { x: forward.x + bar_h + SPACE_S, y, w: bar_h, h: bar_h };
        let view_icons = R { x: view_list.x + bar_h + SPACE_XXS, y, w: bar_h, h: bar_h };
        let search = R { x: w - pad - search_w, y, w: search_w, h: bar_h };
        let location = R { x: view_icons.x + bar_h + SPACE_S, y, w: search.x - SPACE_S - (view_icons.x + bar_h + SPACE_S), h: bar_h };
        let top = y;
        y += bar_h + SPACE_S;
        let list = R { x: x0, y, w: w - pad - x0, h: row_h * VISIBLE_ROWS as f32 };
        let sidebar = R { x: pad, y: top, w: side_w, h: list.y + list.h - top };
        y += list.h + SPACE_XL;
        let ok = R { x: w - pad - button_w, y, w: button_w, h: bar_h };
        let cancel = R { x: ok.x - SPACE_M - button_w, y, w: button_w, h: bar_h };
        let new_folder = R { x: pad, y, w: 120.0, h: bar_h };
        Layout { w, h: y + bar_h + pad, title, field, sidebar, sidebar_row_h: row_h, new_folder, back, forward, view_list, view_icons, location, search, list, row_h, cancel, ok }
    }

    /// The location pop-up's row `i`, under the location control.
    pub fn location_row(&self, l: &Layout, i: usize) -> R {
        R { x: l.location.x, y: l.location.y + l.location.h + 2.0 + l.row_h * i as f32, w: l.location.w, h: l.row_h }
    }

    /// Rows the list leaves for entries: the New Folder field takes the first row while it is open.
    fn row_offset(&self) -> usize {
        usize::from(self.creating.is_some())
    }

    /// Where entry `i` is drawn (a list row or an icon cell), when it is in view.
    pub fn item_rect(&self, l: &Layout, i: usize) -> Option<R> {
        if i < self.top || i >= self.top + self.visible() {
            return None;
        }
        let k = i - self.top;
        let y0 = l.list.y + l.row_h * self.row_offset() as f32;
        if !self.icons {
            return Some(R { x: l.list.x + 4.0, y: y0 + l.row_h * k as f32, w: l.list.w - 8.0, h: l.row_h });
        }
        let cols = self.columns();
        let gap = (l.list.w - CELL_W * cols as f32) / 2.0;
        Some(R { x: l.list.x + gap + CELL_W * (k % cols) as f32, y: y0 + CELL_H * (k / cols) as f32, w: CELL_W, h: CELL_H })
    }

    /// The entry at (x, y) in the list or the grid.
    fn item_at(&self, l: &Layout, x: f32, y: f32) -> Option<usize> {
        (self.top..self.entries.len().min(self.top + self.visible())).find(|i| self.item_rect(l, *i).is_some_and(|r| r.contains(x, y)))
    }

    /// Switches between the list and the icon view, keeping the selection in view.
    pub fn set_icons(&mut self, on: bool) {
        self.icons = on;
        self.top = 0;
        if let Some(i) = self.selected {
            let marked = self.marked.clone();
            let name = self.name.clone();
            self.select(i);
            self.marked = marked;
            self.name = name;
        }
    }

    /// A click at (x, y) points inside the panel, at time `t` ms.
    pub fn click(&mut self, x: f32, y: f32, t: f64, opener: &Opener) -> Option<Outcome> {
        let l = self.layout();
        // the location pop-up takes the click: a row goes there, anywhere else closes it
        if self.loc_menu {
            self.loc_menu = false;
            if let Some((_, loc)) = self.location_items().into_iter().enumerate().find(|(i, _)| self.location_row(&l, *i).contains(x, y)).map(|(_, it)| it) {
                self.navigate(loc, opener);
            }
            return None;
        }
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
        // a field clicked without a measured caret: the caret goes to the end
        if l.field.is_some_and(|f| f.contains(x, y)) {
            let end = self.name.len();
            self.click_field(Focus::Name, end, t);
            return None;
        }
        if l.search.contains(x, y) {
            let end = self.search.len();
            self.click_field(Focus::Search, end, t);
            return None;
        }
        if l.back.contains(x, y) {
            self.go_back(opener);
            return None;
        }
        if l.forward.contains(x, y) {
            self.go_forward(opener);
            return None;
        }
        if l.view_list.contains(x, y) {
            self.set_icons(false);
            return None;
        }
        if l.view_icons.contains(x, y) {
            self.set_icons(true);
            return None;
        }
        if l.location.contains(x, y) {
            self.loc_menu = true;
            return None;
        }
        if l.sidebar.contains(x, y) {
            let row = ((y - l.sidebar.y) / l.sidebar_row_h) as usize;
            if let Some(SideRow::Place(i)) = self.sidebar_rows().get(row).copied() {
                self.go_place(i, opener);
            }
            return None;
        }
        if l.list.contains(x, y) {
            // a click in the list gives the keyboard back to the list (a save panel keeps typing in its name)
            if self.focus == Focus::Search {
                self.focus = if self.kind == Kind::Save { Focus::Name } else { Focus::List };
            }
            match self.item_at(&l, x, y) {
                Some(i) => {
                    let double = self.last_click.is_some_and(|(j, t0)| j == i && t - t0 <= DOUBLE_CLICK_MS);
                    self.last_click = Some((i, t));
                    if double {
                        self.last_click = None;
                        return self.activate(i, opener);
                    }
                    self.toggle(i);
                }
                None => {
                    self.selected = None;
                    self.marked.clear();
                }
            }
        }
        None
    }

    /// A key while the panel is open (evdev codes). `Some` when the panel took it; text is typed through `type_text`.
    pub fn key(&mut self, code: u16, opener: &Opener) -> Option<Option<Outcome>> {
        self.key_with(code, false, opener)
    }

    /// `key` with Cmd held or not: Cmd+Up goes to the enclosing folder, Cmd+Down opens the selection, Cmd+[ and Cmd+]
    /// are back and forward. While a text field has the keyboard, Left, Right, Home, End, Delete and Backspace are left
    /// for the typing path (`edit`), which moves the caret.
    pub fn key_with(&mut self, code: u16, cmd: bool, opener: &Opener) -> Option<Option<Outcome>> {
        let cols = self.columns() as i32;
        let field = self.field_focused();
        Some(match code {
            1 if self.loc_menu => {
                self.loc_menu = false;
                None
            }
            1 if self.creating.is_some() => {
                self.creating = None;
                None
            }
            1 => Some(Outcome::Cancel),
            28 | 96 => self.confirm(opener),
            103 if cmd => {
                self.up(opener);
                None
            }
            108 if cmd => {
                match self.selected {
                    Some(i) if self.entries.get(i).is_some_and(|e| e.dir) => self.enter(i, opener),
                    Some(_) => return Some(self.confirm(opener)),
                    None => {}
                }
                None
            }
            26 if cmd => {
                self.go_back(opener);
                None
            }
            27 if cmd => {
                self.go_forward(opener);
                None
            }
            103 => {
                self.step(-cols);
                None
            }
            108 => {
                self.step(cols);
                None
            }
            // a text field moves its caret with these; the typing path brings them
            105 | 106 | 102 | 107 | 111 | 14 if field => return None,
            105 if self.icons => {
                self.step(-1);
                None
            }
            106 if self.icons => {
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
        self.doc_fit_width();
        self.desktop_menu = Some(super::desktop_bar::Menu::Document);
        self.dirty = true;
    }

    /// The panel stays up until it is answered, whatever else closed the popup (a menu-bar click, Esc elsewhere), and
    /// keeps the width of the window it hangs from as that window is resized.
    pub(super) fn doc_keep_shown(&mut self) {
        if self.doc_panel.is_some() && self.desktop_menu != Some(super::desktop_bar::Menu::Document) {
            self.desktop_menu = Some(super::desktop_bar::Menu::Document);
            self.dirty = true;
        }
        self.doc_fit_width();
    }

    /// Fits the panel's width to its window (`fit_width`): the app's main window, or the screen when it has none.
    pub(super) fn doc_fit_width(&mut self) {
        let Some(slot) = self.doc_panel.as_ref().map(|p| p.slot) else { return };
        let pt = self.ui.pt(1.0).max(f32::EPSILON);
        let m = self.desk_metrics();
        let parent = self.desk.as_ref().and_then(|d| d.index_of(parent_app(slot)).map(|i| d.wins[i].body(&m).w as f32 / pt));
        let screen = self.w as f32 / pt;
        let w = fit_width(parent.unwrap_or(screen), screen);
        if let Some(p) = self.doc_panel.as_mut() {
            if (p.width - w).abs() >= 0.5 {
                p.set_width(w);
                self.dirty = true;
            }
        }
    }

    /// The wheel at screen point (`x`, `y`) px while the panel is up, moving the content `dy` px at `t` ms: over the
    /// panel's list or icon grid the items scroll. The panel is modal, so the wheel never reaches the window under it;
    /// true while a panel is up.
    pub(super) fn doc_wheel(&mut self, x: f32, y: f32, dy: f32, t: f64) -> bool {
        if self.doc_panel.is_none() {
            return false;
        }
        let pt = self.ui.pt(1.0).max(f32::EPSILON);
        let (left, top) = self.doc_panel_rect().map_or((0.0, 0.0), |(l, t, _, _)| (l as f32, t as f32));
        if let Some(p) = self.doc_panel.as_mut() {
            if p.wheel((x - left) / pt, (y - top) / pt, dy / pt, t) {
                self.dirty = true;
            }
        }
        true
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
        if p.slot == SHELL_SLOT {
            return self.wallpaper_panel_done(p.request, out);
        }
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
        let cmd = self.key_mods.contains(bifrost_proto::ModifierFlags::COMMAND);
        let taken = p.key_with(code, cmd, &b.opener);
        self.doc_panel = Some(p);
        let took = taken.is_some();
        self.dirty |= took;
        if let Some(Some(out)) = taken {
            self.doc_finish(out);
        }
        took
    }

    /// What the keyboard typed while the panel is open (`hw_type`): text, Backspace, and the field keys (caret movement,
    /// Select All, Cut, Copy, Paste) for the name or search field.
    pub(super) fn doc_edit(&mut self, out: &super::keyboard::Out) {
        if let Some(p) = self.doc_panel.as_mut() {
            self.dirty |= p.edit(out);
        }
    }

    /// The byte offset in `text` nearest `x` px from its left edge, as the panel draws it (body size, regular).
    fn doc_caret_at(&self, text: &str, x: f32) -> usize {
        use super::ui::{Weight, MAC_BODY_PT};
        let mut best = (0, f32::MAX);
        for i in text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())) {
            let d = (self.ui.measure(&text[..i], MAC_BODY_PT, Weight::Regular) - x).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    /// A click at (x, y) px from the panel's top-left corner.
    pub(super) fn doc_click(&mut self, x: f32, y: f32, t: f64) {
        // the release of the drag the shell emulates for a wheel event is not a click (`doc_wheel`)
        if self.doc_panel.as_mut().is_some_and(|p| p.swallows(t)) {
            return;
        }
        // a click in the name or search field puts the caret under the pointer (measured as the field draws its text)
        let caret = self.doc_panel.as_ref().and_then(|p| {
            let l = p.layout();
            let (px, py) = (x / self.ui.pt(1.0), y / self.ui.pt(1.0));
            if p.creating.is_some() || p.loc_menu {
                return None;
            }
            let hit = |r: &R, field: Focus, text: &str| r.contains(px, py).then(|| (field, self.doc_caret_at(text, x - self.ui.pt(r.x + super::ui::SPACE_S))));
            l.field.as_ref().and_then(|f| hit(f, Focus::Name, p.name())).or_else(|| hit(&l.search, Focus::Search, p.search.text()))
        });
        if let (Some((field, at)), Some(p)) = (caret, self.doc_panel.as_mut()) {
            p.click_field(field, at, t);
            self.dirty = true;
            return;
        }
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
        let m = self.desk_metrics();
        let win = self.desk.as_ref().and_then(|d| d.index_of(parent_app(p.slot)).map(|i| (d.wins[i].body(&m), d.wins[i].full)));
        // a sheet drops from under the title bar; a full-size window's title bar is its unified toolbar band, which the
        // body includes, so the sheet starts below that band (QA W2-13: it covered the window's toolbar)
        let band = if super::desk::full_size(parent_app(p.slot)) && !win.as_ref().is_some_and(|(_, full)| *full) { self.ui.pt(super::desk::UNIFIED_BAR_PT).round() as i32 } else { 0 };
        let (cx, top) = win.map_or((self.w / 2, (self.h - h) / 2), |(b, _)| (b.x + b.w / 2, b.y + band));
        let bar = self.desktop_bar_height() as i32;
        let x = (cx - w / 2).clamp(0, (self.w - w).max(0));
        let y = top.max(bar).min((self.h - h).max(bar));
        Some((x as usize, y as usize, w as usize, h as usize))
    }

    /// A sheet blocks the window it belongs to: the window under the panel is dimmed, as a sheet dims its parent. The panel
    /// itself hangs from the top of that window (`doc_panel_rect`). Drawn before the panel's material.
    pub(super) fn draw_doc_dim(&self, s: &mut crate::gfx::Surface) {
        let Some(p) = self.doc_panel.as_ref() else { return };
        let Some(body) = self.desk.as_ref().and_then(|d| d.index_of(parent_app(p.slot)).map(|i| d.wins[i].body(&self.desk_metrics()))) else { return };
        let radius = self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS);
        s.fill_rrect(body.x as f32, body.y as f32, body.w as f32, body.h as f32, radius, crate::gfx::rgba(0, 0, 0, SHEET_DIM_ALPHA));
    }

    /// Draws the icon of item `e` centred in a square of `side` px whose top-left is (`x`, `y`): a folder in the folder
    /// colour, or a white page with a folded corner and a band in its kind's colour (`FileKind`) that carries the
    /// extension (`thumb_service::badge_for`) when the icon is large enough to read it. `bg` is what the page's cut
    /// corner shows; a row that cannot be chosen is drawn in grey.
    fn draw_file_icon(&self, s: &mut crate::gfx::Surface, e: &Entry, x: f32, y: f32, side: f32, bg: u32) {
        use super::ui::{Weight, MAC_CAPTION2_PT, SECONDARY, SEPARATOR_C, WHITE};
        let kind = FileKind::of(e);
        let tint = if e.enabled { kind.tint().now() } else { SECONDARY };
        let hair = self.ui.pt(1.0).max(1.0);
        if kind == FileKind::Folder {
            // the tab, then the body over its lower half
            let (bw, bh) = (side * 0.9, side * 0.66);
            let (bx, by) = (x + (side - bw) / 2.0, y + side * 0.24);
            s.fill_rrect(bx, by - side * 0.1, bw * 0.42, side * 0.2, side * 0.06, tint);
            s.fill_rrect(bx, by, bw, bh, side * 0.08, tint);
            return;
        }
        let (pw, ph) = (side * 0.74, side * 0.94);
        let (px0, py0) = (x + (side - pw) / 2.0, y + (side - ph) / 2.0);
        let page = if e.enabled { WHITE } else { SEPARATOR_C.now() };
        s.fill_rrect(px0, py0, pw, ph, side * 0.05, page);
        s.stroke_rrect(px0, py0, pw, ph, side * 0.05, hair, SEPARATOR_C.now());
        // the folded corner: the cut shows what is behind the page, the fold is a grey triangle
        let fold = pw * 0.3;
        let right = px0 + pw;
        super::icons::polygon(s, &[(right - fold, py0 - hair), (right + hair, py0 - hair), (right + hair, py0 + fold)], bg);
        super::icons::polygon(s, &[(right - fold, py0), (right - fold, py0 + fold), (right, py0 + fold)], SEPARATOR_C.now());
        // the band in the kind's colour, with the extension when it can be read
        let badge = bifrost_ui::thumb_service::badge_for(&e.name);
        let band_h = if side >= self.ui.pt(32.0) { ph * 0.22 } else { ph * 0.16 };
        let (band_x, band_y, band_w) = (px0 + pw * 0.08, py0 + ph * 0.6, pw * 0.84);
        s.fill_rrect(band_x, band_y, band_w, band_h, band_h * 0.25, tint);
        if let Some(badge) = badge.filter(|_| side >= self.ui.pt(32.0)) {
            let fitted = self.ui.fit(&badge, MAC_CAPTION2_PT, band_w);
            self.label(&fitted, MAC_CAPTION2_PT, Weight::Bold, WHITE, |im| {
                s.blit(im, (band_x + (band_w - im.w as f32) / 2.0) as i32, (band_y + (band_h - im.h as f32) / 2.0) as i32, 255, 255)
            });
        }
    }

    /// Draws the panel's content over the popup background (`draw_desktop_popup` drew the material).
    pub(super) fn draw_doc_panel(&self, s: &mut crate::gfx::Surface, x: usize, y: usize, _w: usize, _h: usize) {
        use super::ui::{Weight, DETAIL_BG, FIELD_BG, LABEL, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SECONDARY, SELECTION, SEPARATOR_C, SPACE_S, WHITE};
        let Some(p) = self.doc_panel.as_ref() else { return };
        // the focus rings and the default button follow the user's accent colour
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
        // text centred in `r`
        let centred = |s: &mut crate::gfx::Surface, r: &R, t: &str, size: f32, color: u32| {
            let (rx, ry, rw, rh) = px(r);
            self.label(t, size, Weight::Regular, color, |im| s.blit(im, (rx + (rw - im.w as f32) / 2.0) as i32, (ry + (rh - im.h as f32) / 2.0) as i32, 255, 255));
        };
        let radius = ui.pt(6.0);
        // a text field: its background, the focus ring when it has the keyboard, the selection plate with its text in
        // white, and the caret when nothing is selected. `inset` is where the text starts (the click maps x the same way).
        let field = |s: &mut crate::gfx::Surface, r: &R, buf: &bifrost_ui::text::TextBuffer, focused: bool, placeholder: &str, capsule: bool| {
            let (fx, fy, fw, fh) = px(r);
            let rad = if capsule { fh / 2.0 } else { radius };
            s.fill_rrect(fx, fy, fw, fh, rad, if capsule { FIELD_BG.now() } else { DETAIL_BG.now() });
            if focused {
                s.stroke_rrect(fx, fy, fw, fh, rad, ui.pt(1.0), accent);
            }
            let left = fx + ui.pt(SPACE_S);
            let t = buf.text();
            if t.is_empty() {
                if !placeholder.is_empty() && !focused {
                    text(s, r, SPACE_S, placeholder, MAC_BODY_PT, Weight::Regular, SECONDARY);
                }
            } else if ui.measure(t, MAC_BODY_PT, Weight::Regular) > fw - ui.pt(2.0 * SPACE_S) {
                // longer than the field: shown cut, without the caret
                text(s, r, SPACE_S, t, MAC_BODY_PT, Weight::Regular, LABEL.now());
                return;
            } else {
                let sel = if focused { buf.selection() } else { 0..0 };
                if !sel.is_empty() {
                    let before = ui.measure(&t[..sel.start], MAC_BODY_PT, Weight::Regular);
                    let picked = ui.measure(&t[sel.clone()], MAC_BODY_PT, Weight::Regular);
                    s.fill_rrect(left + before, fy + ui.pt(4.0), picked, fh - ui.pt(8.0), ui.pt(3.0), bifrost_render::ui::highlight_now());
                }
                let mut cx = left;
                for (part, color) in [(&t[..sel.start], LABEL.now()), (&t[sel.clone()], WHITE), (&t[sel.end..], LABEL.now())] {
                    if part.is_empty() {
                        continue;
                    }
                    self.label(part, MAC_BODY_PT, Weight::Regular, color, |im| s.blit(im, cx as i32, (fy + (fh - im.h as f32) / 2.0) as i32, 255, 255));
                    cx += ui.measure(part, MAC_BODY_PT, Weight::Regular);
                }
            }
            if focused && !buf.has_selection() {
                let at = left + ui.measure(&t[..buf.caret().min(t.len())], MAC_BODY_PT, Weight::Regular);
                s.fill_rrect(at, fy + ui.pt(5.0), ui.pt(1.0).max(1.0), fh - ui.pt(10.0), 0.0, LABEL.now());
            }
        };
        text(s, &l.title, 0.0, &p.title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now());

        if let Some(f) = &l.field {
            let label = R { x: l.title.x, y: f.y, w: f.x - l.title.x, h: f.h };
            text(s, &label, 0.0, "Save As:", MAC_BODY_PT, Weight::Regular, LABEL.now());
            field(s, f, &p.name, p.focus == Focus::Name && p.creating.is_none(), "", false);
        }

        // the toolbar row: back and forward, the view switcher, the location pop-up and the search field
        let button = |s: &mut crate::gfx::Surface, r: &R, glyph: &str, on: bool, enabled: bool| {
            let (bx, by, bw, bh) = px(r);
            if on {
                s.fill_rrect(bx, by, bw, bh, radius, FIELD_BG.now());
            }
            centred(s, r, glyph, MAC_HEADLINE_PT, if enabled { LABEL.now() } else { SECONDARY });
        };
        button(s, &l.back, "\u{2039}", false, p.can_go_back());
        button(s, &l.forward, "\u{203a}", false, p.can_go_forward());
        button(s, &l.view_list, "\u{2261}", !p.icons, true);
        button(s, &l.view_icons, "\u{229e}", p.icons, true);
        let (lx0, ly0, lw0, lh0) = px(&l.location);
        s.fill_rrect(lx0, ly0, lw0, lh0, radius, FIELD_BG.now());
        text(s, &R { w: l.location.w - 20.0, ..l.location }, SPACE_S, &p.location(), MAC_BODY_PT, Weight::Regular, LABEL.now());
        text(s, &R { x: l.location.x + l.location.w - 20.0, w: 20.0, ..l.location }, 4.0, "\u{2195}", MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY);
        field(s, &l.search, &p.search, p.focus == Focus::Search && p.creating.is_none(), "Search", true);

        // sidebar: group headings in footnote grey, plates of 9 pt (the sidebar selection radius) behind the active place
        let plate = ui.pt(9.0);
        let active = p.active_place();
        for (row, item) in p.sidebar_rows().into_iter().enumerate() {
            let r = R { x: l.sidebar.x, y: l.sidebar.y + l.sidebar_row_h * row as f32, w: l.sidebar.w, h: l.sidebar_row_h };
            match item {
                SideRow::Heading(h) => text(s, &R { y: r.y + 6.0, h: r.h - 6.0, ..r }, SPACE_S, h, MAC_FOOTNOTE_PT, Weight::Bold, SECONDARY),
                SideRow::Place(i) => {
                    if active == Some(i) {
                        let (rx, ry, rw, rh) = px(&r);
                        s.fill_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), plate, SELECTION.now());
                    }
                    text(s, &r, SPACE_S, &p.places[i].label, MAC_BODY_PT, Weight::Regular, if active == Some(i) { WHITE } else { LABEL.now() });
                }
            }
        }

        let (lx, ly, lw, lh) = px(&l.list);
        s.fill_rrect(lx, ly, lw, lh, radius, DETAIL_BG.now());
        s.stroke_rrect(lx, ly, lw, lh, radius, ui.pt(1.0), SEPARATOR_C.now());
        // New Folder: the name field takes the first row while it is open
        if let Some(name) = &p.creating {
            let r = R { x: l.list.x + 4.0, y: l.list.y, w: l.list.w - 8.0, h: l.row_h };
            let (rx, ry, rw, rh) = px(&r);
            s.stroke_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), radius, ui.pt(1.0), accent);
            text(s, &r, SPACE_S, &format!("{name}|"), MAC_BODY_PT, Weight::Regular, LABEL.now());
        }
        for (i, e) in p.entries.iter().enumerate().skip(p.top).take(p.visible()) {
            let Some(r) = p.item_rect(&l, i) else { continue };
            let selected = p.marked.contains(&i);
            let color = if selected { WHITE } else if e.enabled { LABEL.now() } else { SECONDARY };
            if p.icons {
                // the item's file-type icon over the name; the name sits on a selection plate when chosen
                let (rx, ry, rw, _) = px(&r);
                let side = ui.pt(ICON_PT);
                self.draw_file_icon(s, e, rx + (rw - side) / 2.0, ry + ui.pt(6.0), side, DETAIL_BG.now());
                let name_r = R { x: r.x + 2.0, y: r.y + 62.0, w: r.w - 4.0, h: 22.0 };
                if selected {
                    let (nx, ny, nw, nh) = px(&name_r);
                    s.fill_rrect(nx, ny, nw, nh, radius, SELECTION.now());
                }
                let fitted = ui.fit(&e.name, MAC_FOOTNOTE_PT, ui.pt(name_r.w - 4.0));
                centred(s, &name_r, &fitted, MAC_FOOTNOTE_PT, color);
            } else {
                if selected {
                    let (rx, ry, rw, rh) = px(&r);
                    s.fill_rrect(rx, ry + ui.pt(2.0), rw, rh - ui.pt(4.0), radius, SELECTION.now());
                }
                // a small file-type icon, then the name after it
                let (rx, ry, _, rh) = px(&r);
                let side = ui.pt(ROW_ICON_PT);
                self.draw_file_icon(s, e, rx + ui.pt(SPACE_S), ry + (rh - side) / 2.0, side, if selected { SELECTION.now() } else { DETAIL_BG.now() });
                let after = SPACE_S + ROW_ICON_PT;
                text(s, &R { x: r.x + after, w: r.w - 24.0 - after, ..r }, SPACE_S, &e.name, MAC_BODY_PT, Weight::Regular, color);
                if e.dir {
                    text(s, &R { x: r.x + r.w - 24.0, w: 24.0, ..r }, SPACE_S, "\u{203a}", MAC_BODY_PT, Weight::Regular, if selected { WHITE } else { SECONDARY });
                }
            }
        }
        if p.entries.is_empty() && p.creating.is_none() {
            let empty = if !p.search.is_empty() {
                "No matches"
            } else if p.in_recents {
                "No recent documents"
            } else {
                "No items"
            };
            text(s, &R { y: l.list.y + l.row_h, h: l.row_h, ..l.list }, SPACE_S, empty, MAC_BODY_PT, Weight::Regular, SECONDARY);
        }

        // Cancel, then the default button (Open or Save) in the tint; the default is dimmed when it would do nothing
        if p.can_create_dir() {
            let (nx, ny, nw, nh) = px(&l.new_folder);
            s.fill_rrect(nx, ny, nw, nh, radius, FIELD_BG.now());
            centred(s, &l.new_folder, "New Folder", MAC_BODY_PT, LABEL.now());
        }
        let (cx, cy, cw, ch) = px(&l.cancel);
        s.fill_rrect(cx, cy, cw, ch, radius, FIELD_BG.now());
        centred(s, &l.cancel, "Cancel", MAC_BODY_PT, LABEL.now());
        let (bx, by, bw, bh) = px(&l.ok);
        let enabled = p.can_confirm();
        s.fill_rrect(bx, by, bw, bh, radius, if enabled { accent } else { FIELD_BG.now() });
        centred(s, &l.ok, if p.kind == Kind::Save { "Save" } else { "Open" }, MAC_BODY_PT, if enabled { WHITE } else { SECONDARY });

        // the location pop-up, over everything: the folder shown (checked), then the folders that enclose it
        if p.loc_menu {
            let items = p.location_items();
            let first = p.location_row(&l, 0);
            let all = R { h: l.row_h * items.len() as f32, ..first };
            let (mx, my, mw, mh) = px(&all);
            s.fill_rrect(mx - ui.pt(2.0), my - ui.pt(2.0), mw + ui.pt(4.0), mh + ui.pt(4.0), radius, FIELD_BG.now());
            s.stroke_rrect(mx - ui.pt(2.0), my - ui.pt(2.0), mw + ui.pt(4.0), mh + ui.pt(4.0), radius, ui.pt(1.0), SEPARATOR_C.now());
            for (i, (label, _)) in items.iter().enumerate() {
                let r = p.location_row(&l, i);
                let mark = if i == 0 { "\u{2713} " } else { "    " };
                text(s, &r, SPACE_S, &format!("{mark}{label}"), MAC_BODY_PT, Weight::Regular, LABEL.now());
            }
        }
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

    /// A panel on a Documents folder of `n` text files (f00.txt, f01.txt, ...).
    fn many(tag: &str, n: usize) -> (PathBuf, Opener, Panel) {
        let d = std::env::temp_dir().join(format!("docpanel-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/Documents")).unwrap();
        for i in 0..n {
            fs::write(d.join(format!("home/Documents/f{i:02}.txt")), "x").unwrap();
        }
        let m = fs::metadata(&d).unwrap();
        let o = Opener::new(&d.join("home"), m.uid(), m.gid()).unwrap();
        let p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        (d, o, p)
    }

    #[test]
    fn the_wheel_scrolls_the_list_by_rows_and_the_grid_by_rows_of_icons() {
        let (d, _o, mut p) = many("wheel", 30);
        assert_eq!(p.entries.len(), 30);
        let l = p.layout();
        let (x, y) = (l.list.x + 10.0, l.list.y + 10.0);
        // a notch turned down shows the next row; more travel, more rows
        assert!(p.wheel(x, y, -3.0, 0.0));
        assert_eq!(p.top, 1);
        assert!(p.wheel(x, y, -l.row_h * 4.0, 10.0));
        assert_eq!(p.top, 5);
        assert!(p.wheel(x, y, l.row_h * 2.0, 20.0), "turned up, the rows above");
        assert_eq!(p.top, 3);
        p.wheel(x, y, -1.0e5, 30.0);
        assert_eq!(p.top, 30 - VISIBLE_ROWS, "the last item stays on the last row");
        assert!(!p.wheel(x, y, -30.0, 40.0), "the end");
        assert!(!p.wheel(l.ok.x + 1.0, l.ok.y + 1.0, 30.0, 50.0), "only the list scrolls");
        assert_eq!(p.top, 30 - VISIBLE_ROWS);
        // the icon grid moves by whole rows of icons
        p.set_icons(true);
        let cols = p.columns();
        assert!(cols > 1);
        assert!(p.wheel(x, y, -3.0, 60.0));
        assert_eq!(p.top, cols);
        p.wheel(x, y, -1.0e5, 70.0);
        assert_eq!(p.top % cols, 0);
        assert!(p.top + p.visible() >= 30 && p.top < 30, "the last row is in view: top {}", p.top);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_release_of_the_drag_a_wheel_emulates_is_not_a_click() {
        let (d, o, mut p) = many("wheel-click", 3);
        let l = p.layout();
        let row = |i: usize| l.list.y + l.row_h * (i as f32 + 0.5);
        p.wheel(l.list.x + 10.0, row(1), -3.0, 1000.0);
        assert!(p.swallows(1100.0), "the release that follows the wheel");
        assert!(!p.swallows(1150.0), "only that one");
        p.wheel(l.list.x + 10.0, row(1), -3.0, 2000.0);
        assert!(!p.swallows(2000.0 + WHEEL_DRAG_MS + 1.0), "a release long after the wheel is a click");
        assert_eq!(p.click(l.list.x + 10.0, row(1), 3000.0, &o), None);
        assert_eq!(p.selected, Some(1));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_panel_width_follows_its_window_within_a_range_and_the_list_takes_the_rest() {
        assert_eq!(fit_width(800.0, 1920.0), 800.0 - WINDOW_INSET);
        assert_eq!(fit_width(300.0, 1920.0), MIN_W, "a small window still gets a usable panel");
        assert_eq!(fit_width(3000.0, 3840.0), MAX_W);
        assert_eq!(fit_width(500.0, 600.0), 600.0f32.min(MIN_W), "never wider than the screen");
        assert_eq!(fit_width(f32::NAN, 1920.0), DEFAULT_W);
        let (d, _o, mut p) = many("width", 30);
        assert_eq!(p.layout().w, DEFAULT_W);
        p.set_icons(true);
        p.wheel(p.layout().list.x + 1.0, p.layout().list.y + 1.0, -3.0, 0.0);
        let narrow_cols = p.columns();
        p.set_width(MAX_W);
        let l = p.layout();
        assert_eq!(l.w, MAX_W);
        assert!((l.ok.x + l.ok.w - (MAX_W - super::super::ui::SPACE_XL)).abs() < 0.01, "the buttons keep to the right edge");
        assert!(l.list.x + l.list.w <= l.w && l.search.x + l.search.w <= l.w);
        assert!(p.columns() > narrow_cols, "a wider panel shows more icons a row");
        assert_eq!(p.top % p.columns(), 0, "the first item shown starts a row");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_icon_follows_its_content_type() {
        let entry = |name: &str, dir: bool| Entry { name: name.into(), dir, enabled: true, path: None };
        let kinds: Vec<FileKind> = [("Drafts", true), ("photo.png", false), ("b.txt", false), ("A.md", false), ("a.pdf", false), ("x.zip", false), ("song.mp3", false), ("clip.mov", false), ("data.xyz9", false), ("README", false)]
            .iter()
            .map(|(n, dir)| FileKind::of(&entry(n, *dir)))
            .collect();
        use FileKind as K;
        assert_eq!(kinds, [K::Folder, K::Image, K::Text, K::Text, K::Pdf, K::Archive, K::Audio, K::Movie, K::Document, K::Document]);
        assert_ne!(K::Pdf.tint(), super::super::ui::system::SYSTEM_RED, "red is for destructive actions");
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
        assert_eq!((p.name(), p.title.as_str()), ("Untitled1.md", "Export"), "a slash cannot be part of the name");
        assert!(p.layout().field.is_some());
        assert_eq!(p.name_sel(), Some(0..9), "the base name is selected as the panel opens, the extension is not");
        p.type_text("x\n");
        assert_eq!(p.name(), "x.md", "typing replaces the selected base name and keeps the extension");
        assert_eq!(p.name_sel(), None);
        p.type_text("y");
        assert_eq!(p.name(), "xy.md", "the caret stays after what was typed (QA W2-10: it jumped to the end)");
        p.backspace();
        assert_eq!(p.name(), "x.md", "Backspace deletes before the caret");
        p.select(1);
        assert_eq!(p.name_sel(), Some(0..1), "choosing a file selects its base name too");
        assert_eq!(p.name(), "b.txt");
        assert_eq!(p.key(28, &o), Some(Some(Outcome::Save { dir: "Documents".into(), name: "b.txt".into() })));
        p.name.set_text("");
        p.selected = None;
        assert!(!p.can_confirm(), "an empty name cannot be saved");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_name_field_selects_the_base_name_and_editing_replaces_or_deletes_it() {
        let (d, o) = scratch("namesel");
        let save = |name: &str| Panel::new(0, 1, Kind::Save, "", vec![], name, &o);
        // a name with an extension selects what is before the last dot; one without selects all of it
        assert_eq!(save("Report.final.rtf").name_sel(), Some(0..12));
        assert_eq!(save("Untitled").name_sel(), Some(0..8));
        assert_eq!(save(".profile").name_sel(), Some(0..8), "a leading dot is part of the name");
        assert_eq!(save("").name_sel(), None, "an empty name has nothing to select");
        let open = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "Untitled.rtf", &o);
        assert_eq!(open.name_sel(), None, "only a save panel has a name field");
        // Backspace over the selection deletes it and keeps the extension
        let mut p = save("Untitled.rtf");
        assert!(p.backspace());
        assert_eq!((p.name(), p.name_sel()), (".rtf", None));
        assert!(!p.backspace(), "the caret is at the start: nothing before it to delete");
        assert!(p.edit(&crate::shell::keyboard::Out::Named("End")));
        assert!(p.backspace(), "at the end Backspace deletes the last character");
        assert_eq!(p.name(), ".rt");
        // a slash is not typeable: it leaves the selection alone
        let mut p = save("Untitled.rtf");
        p.type_text("/");
        assert_eq!((p.name(), p.name_sel()), ("Untitled.rtf", Some(0..8)));
        // typing replaces the selection in the middle of text and the limit still holds
        p.type_text("Résumé");
        assert_eq!(p.name(), "Résumé.rtf");
        let mut long = save(&format!("{}.md", "a".repeat(300)));
        assert_eq!(long.name.len(), 255);
        long.type_text("b");
        assert!(long.name.len() <= 255 && long.name().starts_with('b'));
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

    /// A signed-in desktop shell with an app on a test channel in slot 0 and a broker over a scratch home. The test shell
    /// starts at the lock screen, which owns the keyboard (E2E N0: `hw_menu_key` and `hw_type` give a locked screen's
    /// keys to the login window), so the person using the app is signed in first.
    fn shell_with_app(tag: &str) -> (PathBuf, super::super::Shell, bifrost_proto::sys::Channel) {
        let (d, o) = scratch(tag);
        let mut sh = super::super::tests::desktop_shell();
        sh.screen = super::super::Screen::Home;
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

    /// E2E: the wheel over an open panel is the panel's (it is modal), and the release of the drag the shell emulates
    /// for it does not click the row under the pointer; a click afterwards does.
    #[test]
    fn the_wheel_over_the_panel_never_clicks_and_never_reaches_the_window_under_it() {
        ALLOW.with(|a| a.set(true));
        let (d, mut sh, peer) = shell_with_app("e2e-wheel");
        ask(&peer, &mut sh, ToShell::DocOpen { request: 3, mode: DocMode::Read, title: String::new(), filters: vec![] });
        let (left, top, w, _) = sh.doc_panel_rect().expect("the panel is up");
        let p = sh.doc_panel.as_ref().unwrap();
        let l = p.layout();
        assert!((sh.ui.pt(l.w) - w as f32).abs() <= 1.0, "the rect is the fitted layout's width");
        assert!(l.w >= MIN_W.min(sh.w as f32 / sh.ui.pt(1.0)) && l.w <= MAX_W);
        let pt = sh.ui.pt(1.0);
        let (x, y) = (left as f32 + pt * (l.list.x + 10.0), top as f32 + pt * (l.list.y + l.row_h * 1.5));
        assert!(sh.pointer_wheel(x, y, 0.0, -3.0, 5000.0), "the panel takes the wheel");
        sh.desktop_bar_event(super::super::Event::Down(x, y), 5010.0);
        sh.desktop_bar_event(super::super::Event::Up(x, y), 5030.0);
        assert_eq!(sh.doc_panel.as_ref().unwrap().selected, None, "the emulated release is no click");
        sh.desktop_bar_event(super::super::Event::Down(x, y), 9000.0);
        sh.desktop_bar_event(super::super::Event::Up(x, y), 9010.0);
        assert_eq!(sh.doc_panel.as_ref().unwrap().selected, Some(1), "a real click selects the row");
        // outside the panel the wheel is used up too: the panel is modal
        assert!(sh.pointer_wheel(1.0, sh.h as f32 - 2.0, 0.0, -3.0, 9500.0));
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
        assert_eq!(labels, ["Recents", "Documents", "Downloads", "Home"], "folders that do not exist are not listed");
        assert_eq!(p.sidebar_rows(), [SideRow::Place(0), SideRow::Heading("Favorites"), SideRow::Place(1), SideRow::Place(2), SideRow::Heading("Locations"), SideRow::Place(3)]);
        assert_eq!(p.active_place(), Some(1));
        let l = p.layout();
        let at = |p: &Panel, i: usize| l.sidebar.y + (p.sidebar_row_of(i).unwrap() as f32 + 0.5) * l.sidebar_row_h;
        assert_eq!(p.click(l.sidebar.x + 5.0, l.sidebar.y + 1.5 * l.sidebar_row_h, 0.0, &o), None, "a heading is not a place");
        assert_eq!(p.dir, Path::new("Documents"));
        p.click(l.sidebar.x + 5.0, at(&p, 0), 0.0, &o);
        assert!(p.in_recents && p.active_place() == Some(0));
        assert_eq!((p.location(), p.can_go_up()), ("Recents".into(), false));
        assert_eq!(names(&p), ["b.txt", "Drafts"], "newest first; a document that is gone is left out");
        p.select(0);
        assert_eq!(p.confirm(&o), Some(Outcome::Open("Documents/b.txt".into())));
        p.select(1);
        assert_eq!(p.confirm(&o), None, "a folder in Recents opens it");
        assert!(!p.in_recents && p.dir == Path::new("Documents/Drafts"));
        p.click(l.sidebar.x + 5.0, at(&p, 2), 1000.0, &o);
        assert_eq!(p.dir, Path::new("Downloads"));
        p.click(l.sidebar.x + 5.0, at(&p, 3), 2000.0, &o);
        assert_eq!((p.location(), p.active_place()), ("Home".into(), Some(3)));
        assert!(p.go_to(Path::new("Documents"), &o) && !p.go_to(Path::new("Nowhere"), &o));
        fs::remove_dir_all(&d).unwrap();
    }

    /// QA W2-10: the first character replaced the selected base name, the rest went after the extension ("e.rtf2e-test").
    #[test]
    fn typing_a_name_one_key_at_a_time_keeps_the_caret_after_the_typing() {
        let (d, o) = scratch("caret");
        let mut p = Panel::new(0, 1, Kind::Save, "", vec![], "Untitled.rtf", &o);
        for c in "e2e-test".chars() {
            p.type_text(&c.to_string());
        }
        assert_eq!(p.name(), "e2e-test.rtf");
        assert_eq!(p.key(28, &o), Some(Some(Outcome::Save { dir: "Documents".into(), name: "e2e-test.rtf".into() })));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_name_field_moves_selects_and_edits_like_a_text_field() {
        use crate::shell::keyboard::Out;
        let (d, o) = scratch("fieldkeys");
        let mut p = Panel::new(0, 1, Kind::Save, "", vec![], "Report.rtf", &o);
        // the caret keys are the field's: the panel's own key handler leaves them to the typing path
        for code in [105, 106, 102, 107, 111, 14] {
            assert_eq!(p.key(code, &o), None, "key {code} goes to the field");
        }
        assert!(p.edit(&Out::Named("Right")));
        assert_eq!((p.name.caret(), p.name_sel()), (6, None), "Right collapses the selection to its end");
        p.type_text("s");
        assert_eq!(p.name(), "Reports.rtf");
        p.edit(&Out::Named("Home"));
        p.edit(&Out::Named("ShiftWordRight"));
        assert_eq!(p.name_sel(), Some(0..7), "Shift+Option+Right selects the word");
        p.edit(&Out::Named("SelectAll"));
        assert_eq!(p.name_sel(), Some(0..11), "Cmd+A selects the whole name");
        p.type_text("a.md");
        assert_eq!(p.name(), "a.md");
        p.edit(&Out::Named("Left"));
        p.edit(&Out::Named("Left"));
        p.edit(&Out::Named("Left"));
        p.edit(&Out::Named("DeleteForward"));
        assert_eq!(p.name(), "amd", "Delete removes the character after the caret");
        p.edit(&Out::Named("End"));
        p.edit(&Out::Named("DeleteLine"));
        assert_eq!(p.name(), "", "Cmd+Backspace deletes everything before the caret");
        assert!(!p.can_confirm());
        // a click places the caret where the shell measured it; a double click selects the word
        p.name.set_text("one two.txt");
        p.click_field(Focus::Name, 2, 0.0);
        assert_eq!((p.name.caret(), p.name_sel()), (2, None));
        p.click_field(Focus::Name, 5, 100.0);
        assert_eq!(p.name_sel(), Some(4..7));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_search_field_filters_the_folder_and_is_cleared_on_navigation() {
        use crate::shell::keyboard::Out;
        let (d, o) = scratch("search");
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        assert_eq!(p.focus, Focus::List);
        assert_eq!(p.key(105, &o), Some(None), "with the list focused Left still goes up");
        assert_eq!(p.dir, Path::new(""));
        assert!(p.go_back(&o));
        assert_eq!(p.dir, Path::new("Documents"));
        // typing in an open panel starts a search
        p.type_text("T");
        assert_eq!((p.focus, p.search.text()), (Focus::Search, "T"));
        assert_eq!(names(&p), ["b.txt", "Drafts", "photo.png"], "any case, anywhere in the name");
        p.type_text("xt");
        assert_eq!(names(&p), ["b.txt"]);
        assert_eq!(p.key(105, &o), None, "Left moves the search field's caret");
        p.edit(&Out::Backspace);
        p.edit(&Out::Backspace);
        p.edit(&Out::Backspace);
        assert_eq!(names(&p), ["A.md", "b.txt", "Drafts", "photo.png"]);
        p.type_text("zzz");
        assert!(p.entries.is_empty());
        // a click in the list hands the keys back to it; going into a folder clears the search
        p.search.set_text("dra");
        p.edit(&Out::Named("End"));
        p.type_text("f");
        assert_eq!(names(&p), ["Drafts"]);
        let l = p.layout();
        p.click(l.list.x + 10.0, l.list.y + l.row_h * 0.5, 0.0, &o);
        assert_eq!(p.focus, Focus::List);
        p.key(28, &o);
        assert_eq!((p.dir.as_path(), p.search.text()), (Path::new("Documents/Drafts"), ""));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn back_and_forward_step_through_the_folders_visited() {
        let (d, o) = scratch("history");
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        assert!(!p.can_go_back() && !p.can_go_forward());
        let l = p.layout();
        p.select(2);
        p.key(28, &o);
        assert_eq!(p.dir, Path::new("Documents/Drafts"));
        p.click(l.back.x + 1.0, l.back.y + 1.0, 0.0, &o);
        assert_eq!(p.dir, Path::new("Documents"));
        assert!(p.can_go_forward());
        p.click(l.forward.x + 1.0, l.forward.y + 1.0, 1000.0, &o);
        assert_eq!(p.dir, Path::new("Documents/Drafts"));
        // Cmd+Up goes up and Cmd+[ comes back; a new place drops what forward remembered
        assert_eq!(p.key_with(103, true, &o), Some(None));
        assert_eq!(p.dir, Path::new("Documents"));
        p.key_with(26, true, &o);
        assert_eq!(p.dir, Path::new("Documents/Drafts"));
        p.go_place(0, &o);
        assert!(p.in_recents && !p.can_go_forward());
        // a folder that went away is skipped
        fs::remove_dir_all(d.join("home/Documents/Drafts")).unwrap();
        assert!(p.go_back(&o));
        assert_eq!(p.dir, Path::new("Documents"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_location_pop_up_lists_the_enclosing_folders_and_goes_there() {
        let (d, o) = scratch("location");
        let mut p = Panel::new(0, 1, Kind::Save, "", vec![], "x.md", &o);
        p.go_to(Path::new("Documents/Drafts"), &o);
        let labels: Vec<String> = p.location_items().into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["Drafts", "Documents", "Home"]);
        let l = p.layout();
        p.click(l.location.x + 5.0, l.location.y + 5.0, 0.0, &o);
        assert!(p.loc_menu);
        assert_eq!(p.key(1, &o), Some(None), "Esc closes the pop-up, not the panel");
        assert!(!p.loc_menu);
        p.click(l.location.x + 5.0, l.location.y + 5.0, 0.0, &o);
        let r = p.location_row(&l, 2);
        p.click(r.x + 5.0, r.y + 5.0, 100.0, &o);
        assert_eq!((p.dir.as_path(), p.loc_menu), (Path::new(""), false));
        assert!(p.can_go_back());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_icon_view_lays_items_in_rows_and_arrows_move_through_them() {
        let (d, o) = scratch("icons");
        for i in 0..20 {
            fs::write(d.join(format!("home/Documents/n{i:02}.txt")), "x").unwrap();
        }
        let mut p = Panel::new(0, 1, Kind::Open(DocMode::Read), "", vec![], "", &o);
        let l = p.layout();
        p.click(l.view_icons.x + 1.0, l.view_icons.y + 1.0, 0.0, &o);
        assert!(p.icons);
        let cols = p.columns();
        assert!(cols >= 3, "{cols} columns");
        assert_eq!(p.visible() % cols, 0);
        let second = p.item_rect(&l, 1).unwrap();
        assert_eq!(second.y, p.item_rect(&l, 0).unwrap().y, "side by side");
        assert_eq!(p.item_rect(&l, cols).unwrap().x, p.item_rect(&l, 0).unwrap().x, "the next row starts under the first");
        p.click(second.x + 5.0, second.y + 5.0, 1000.0, &o);
        assert_eq!(p.selected, Some(1));
        p.key(108, &o);
        assert_eq!(p.selected, Some(1 + cols), "Down moves a row of icons");
        p.key(106, &o);
        assert_eq!(p.selected, Some(2 + cols), "Right moves one icon");
        p.select(p.entries.len() - 1);
        assert!(p.top > 0 && p.top % cols == 0, "scrolled by whole rows");
        assert!(p.item_rect(&l, p.entries.len() - 1).is_some());
        p.click(l.view_list.x + 1.0, l.view_list.y + 1.0, 2000.0, &o);
        assert!(!p.icons && p.item_rect(&l, p.selected.unwrap()).is_some(), "the selection stays in view in the list");
        fs::remove_dir_all(&d).unwrap();
    }

    /// A save panel in a shell: the caret keys come through `hw_type` as named keys and edit the name, not the folder.
    #[test]
    fn the_shell_routes_caret_keys_and_select_all_to_the_name_field() {
        use crate::shell::keyboard::Out;
        ALLOW.with(|a| a.set(true));
        let (d, mut sh, peer) = shell_with_app("e2e-caret");
        ask(&peer, &mut sh, ToShell::DocSave { request: 4, suggested_name: "Untitled.rtf".into(), filters: vec![] });
        assert_eq!(sh.hw_menu_key(105, true, 1.0), None, "Left is the field's, not the panel's Back");
        for out in [Out::Text("e".into()), Out::Text("2".into()), Out::Named("Left"), Out::Text("x".into()), Out::Named("End"), Out::Backspace] {
            sh.hw_type(out, 1.0);
        }
        assert_eq!(sh.doc_panel.as_ref().unwrap().name(), "ex2.rt");
        sh.hw_type(Out::Named("SelectAll"), 1.0);
        sh.hw_type(Out::Text("final.rtf".into()), 1.0);
        sh.hw_menu_key(28, true, 1.0);
        let got = replies(&peer);
        assert!(matches!(&got[..], [(ToApp::DocGranted { request: 4, name, .. }, Some(_))] if name == "final.rtf"), "{got:?}");
        ALLOW.with(|a| a.set(false));
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

    /// Opens the file at `path` (in the account's home) read-only in the installed app `key`: the app comes up and gets
    /// a grant and the descriptor as an inbound document, as a file dropped on it does. False when `key` is not an
    /// installed app or the file cannot be granted (the Dock's stack then shows the file in Files).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn open_path_with(&mut self, key: &str, path: &Path, t: f64) -> bool {
        use super::apps::{apps, AppId};
        let Some(AppId::External(slot)) = apps().iter().find(|a| a.icon == key).map(|a| a.id) else { return false };
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let Some(broker) = self.doc_broker() else { return false };
        let Ok(rel) = path.strip_prefix(broker.opener.home()).map(Path::to_path_buf) else { return false };
        let Ok(file) = broker.opener.open_any(&rel, DocMode::Read) else { return false };
        let size = file.metadata().map_or(0, |m| m.len());
        let Ok(grant) = broker.grants.add(key, &rel, DocMode::Read, &file) else { return false };
        self.desk_open_or_start(AppId::External(slot), t);
        let request = self.app_share.next_inbound();
        self.doc_send(slot, ToApp::DocGranted { request, grant, mode: DocMode::Read, name, size }, Some(file));
        true
    }

    /// Image > Edit With for the photo open in the viewer.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn photo_edit_with_viewed(&mut self, key: &str, t: f64) {
        let Some(path) = self.photos.as_ref().and_then(|p| p.viewing()).map(|p| p.path.clone()) else { return };
        self.photo_edit_with(key, &path, t);
    }
}
