//! Fonts as an installed BifrOSt app in the full-size window: a sidebar of lists (All, Disabled, collections, smart
//! collections, New Collection), the family list with search, and a preview with three modes (Sample, Repertoire,
//! Info) plus a validation result, the on/off switch, Remove Font for the user's own installs, and collection
//! membership. Fonts are added from the Font menu (Add Fonts), by Open With, or by a `fontbook:` link; a file is shown
//! in an install preview first and refused when it fails validation (a setting). The model is in `model.rs`; the font
//! checks are in `validate.rs`. Libraries (named sets of font folders) switch what the list reads; Export Font saves a
//! copy of the selected font through the save panel, Export Fonts to Folder copies every family the list shows into a
//! folder (`export.rs`), Validate File checks a font file from the panel without installing it, Show in Files reveals
//! a font of the user's, and Print Sample Sheet sends the font's details and sample line to the printer.

mod export;
mod model;
mod validate;

use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{Damage, Font, Surface};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{ax, Rect, Role, TextField};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::print::{PrintError, PrintOperation};
use bifrost_app::proto::DocErrorCode;
use bifrost_app::workspace;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use bifrost_render::fontprefs;
use model::{FontSet, Library, Prefs, Rule, Source};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The shell's seeded asset folder: its font library is `<dir>/fonts`, read by Settings > Fonts.
const SHELL_ASSETS: &str = "/var/lib/springboard/assets";
/// Sidebar width, list column width and the header height, in points.
const SIDEBAR_PT: f32 = 200.0;
const LIST_PT: f32 = 260.0;
const ROW_PT: f32 = 28.0;
const HEADER_PT: f32 = 56.0;
/// The family list starts under its count line.
const LIST_TOP_PT: f32 = HEADER_PT + 22.0;
/// Height of the toolbar's buttons and of the search capsule, and the preview buttons' top: both centred in the
/// unified bar (`UNIFIED_BAR_PT`), so the bar's controls keep an equal inset above and below.
const BAR_BUTTON_H_PT: f32 = 28.0;
const BAR_BUTTON_Y_PT: f32 = (UNIFIED_BAR_PT - BAR_BUTTON_H_PT) / 2.0;
const PAD_PT: f32 = 12.0;
/// Width of the preview's buttons (On/Off, Remove Font), and the gap from the switch to the remove button.
const BUTTON_PT: f32 = 120.0;
const BUTTON_GAP_PT: f32 = 130.0;
/// The mode tabs under the family name, and where a mode's content starts.
const TABS_TOP_PT: f32 = HEADER_PT + 44.0;
const TAB_PT: f32 = 100.0;
const TAB_H_PT: f32 = 24.0;
const CONTENT_TOP_PT: f32 = HEADER_PT + 84.0;
/// The sample field and its size buttons (Sample mode).
const FIELD_W_PT: f32 = 360.0;
const FIELD_H_PT: f32 = 26.0;
const SIZE_BTN_PT: f32 = 26.0;
/// The collection membership block in Info mode, below the info rows.
const MEMBERSHIP_TOP_PT: f32 = CONTENT_TOP_PT + 210.0;
const MEMBERSHIP_ROW_PT: f32 = 22.0;
/// Sizes of the ladder under the sample, in points; only those smaller than the sample size are drawn.
const LADDER_SIZES: [f32; 3] = [12.0, 18.0, 24.0];
/// The size step of Bigger and Smaller.
const SIZE_STEP: f32 = 4.0;
/// Columns and cell size of the repertoire grid.
const GRID_COLS: usize = 16;
const GRID_CELL_PT: f32 = 28.0;

// main menu items
const MENU_ADD: u32 = 1;
const MENU_NEW_COLLECTION: u32 = 2;
const MENU_SMART_FIXED: u32 = 3;
const MENU_SMART_VARIABLE: u32 = 4;
const MENU_SMART_STATIC: u32 = 5;
const MENU_SMART_TTF: u32 = 6;
const MENU_SMART_OTF: u32 = 7;
const MENU_SMART_NAME: u32 = 8;
const MENU_RENAME: u32 = 9;
const MENU_DELETE: u32 = 10;
const MENU_VALIDATE: u32 = 11;
const MENU_DUPLICATES: u32 = 12;
const MENU_SAMPLE: u32 = 13;
const MENU_REPERTOIRE: u32 = 14;
const MENU_INFO: u32 = 15;
const MENU_EDIT_TEXT: u32 = 16;
const MENU_BIGGER: u32 = 17;
const MENU_SMALLER: u32 = 18;
const MENU_VALIDATE_FIRST: u32 = 19;
const MENU_TOGGLE: u32 = 20;
const MENU_REMOVE: u32 = 21;
const MENU_LIBRARY_ADD: u32 = 22;
const MENU_LIBRARY_REMOVE: u32 = 23;
const MENU_REVEAL: u32 = 24;
const MENU_EXPORT: u32 = 25;
const MENU_PRINT: u32 = 26;
const MENU_VALIDATE_FILE: u32 = 27;
const MENU_EXPORT_FOLDER: u32 = 28;
const MENU_SMART_ITALIC: u32 = 29;
/// The library items of the menu: this id plus the library's index in `model::library_names`.
const MENU_LIBRARY_BASE: u32 = 100;

/// What the preview column shows for the selected family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sample,
    Repertoire,
    Info,
    Validation,
}

/// A text field being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Edit {
    None,
    /// the sample line
    Sample(String),
    /// a collection's name (`smart` for a smart collection)
    Rename { old: String, smart: bool, text: String },
}

/// A font file waiting in the install preview.
struct Pending {
    /// the file to install (a staged copy when it came from the open panel)
    path: PathBuf,
    name: String,
    family: Option<String>,
    report: validate::Report,
    font: Option<Ui>,
}

struct FontBookApp {
    library: Option<Library>,
    /// the user's own libraries (named folder sets), saved in the data dir
    sets: Vec<FontSet>,
    home: PathBuf,
    user_dir: PathBuf,
    source: Source,
    query: String,
    /// the toolbar's search field was the last thing pressed
    search_focused: bool,
    selected: Option<String>,
    mode: Mode,
    prefs: Prefs,
    edit: Edit,
    pending: Option<Pending>,
    /// duplicates found by the first Resolve Duplicates, removed by the second
    duplicates: Vec<model::Duplicate>,
    /// the sample font for `selected`, drawn at the current scale
    preview: Option<(String, Ui)>,
    /// the validation and repertoire of the selected font's file, with the family it belongs to
    report: Option<(String, validate::Report)>,
    status: String,
    /// the request of the folder panel that adds a library
    add_library: Option<u32>,
    /// the request that grants the selected font's file so it can be shown in Files
    reveal: Option<u32>,
    /// the save panel request of an export, with the font file it copies
    export: Option<(u32, PathBuf)>,
    /// the folder panel request of Export Fonts to Folder, with the font files it copies
    export_folder: Option<(u32, Vec<PathBuf>)>,
    /// the open panel request of Validate File
    validate_file: Option<u32>,
    /// the file chosen in Validate File, checked and not installed, shown in place of the selected family
    checked: Option<(String, validate::Report)>,
    /// a message from a completion closure (the workspace answers later), shown at the next draw
    notice: Rc<RefCell<String>>,
}

impl FontBookApp {
    /// Reads the families from disk and the user's settings from the data dir.
    fn reload(&mut self, data_dir: &Path) {
        self.home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        self.user_dir = self.home.join(model::USER_FONTS_DIR);
        self.prefs = Prefs::parse(&model::read_text(data_dir, model::PREFS_FILE));
        self.sets = model::parse_font_sets(&model::read_text(data_dir, model::LIBRARIES_FILE));
        let dirs = model::library_dirs(&self.prefs.library, &self.sets, &self.home, &fontprefs::library(SHELL_ASSETS));
        let families = fontprefs::installed(&dirs);
        let disabled = model::parse_disabled(&model::read_text(data_dir, model::DISABLED_FILE));
        let collections = model::parse_collections(&model::read_text(data_dir, model::COLLECTIONS_FILE));
        let smart = model::parse_smart(&model::read_text(data_dir, model::SMART_FILE));
        self.library = Some(Library::new(families, disabled, collections).with_smart(smart));
        self.preview = None;
        self.report = None;
    }

    /// Saves the disabled list, the collections and the smart collections; a failed write is shown in the status line.
    fn save(&mut self, data_dir: &Path) {
        let Some(lib) = &self.library else { return };
        let a = model::write_text(data_dir, model::DISABLED_FILE, &lib.disabled_file());
        let b = model::write_text(data_dir, model::COLLECTIONS_FILE, &lib.collections_file());
        let c = model::write_text(data_dir, model::SMART_FILE, &lib.smart_file());
        if let Err(e) = a.and(b).and(c) {
            self.status = format!("Could not save: {e}");
        }
    }

    fn save_sets(&mut self, data_dir: &Path) {
        if let Err(e) = model::write_text(data_dir, model::LIBRARIES_FILE, &model::font_sets_file(&self.sets)) {
            self.status = format!("Could not save: {e}");
        }
    }

    /// Switches the list to the library `name` and rereads its folders.
    fn switch_library(&mut self, data_dir: &Path, name: &str) {
        self.prefs.library = name.to_string();
        self.save_prefs(data_dir);
        self.selected = None;
        self.source = Source::All;
        self.reload(data_dir);
        self.status = format!("Library: {name}.");
    }

    /// Remove Library: forgets the current library of the user's (its fonts stay on disk) and returns to Computer.
    fn remove_library(&mut self, data_dir: &Path) {
        let name = self.prefs.library.clone();
        if model::remove_font_set(&mut self.sets, &name) {
            self.save_sets(data_dir);
            self.switch_library(data_dir, model::LIBRARY_COMPUTER);
            self.status = format!("Removed library {name}. Its fonts are still on disk.");
        }
    }

    /// The folder panel answered: the chosen folder becomes a library and the list switches to it. The app gets the
    /// open folder, so its path is read from the descriptor's link.
    fn add_library_folder(&mut self, ctx: &Ctx, doc: bifrost_app::Document) {
        use std::os::fd::AsRawFd;
        let link = std::fs::read_link(format!("/proc/self/fd/{}", doc.file.as_raw_fd()));
        let dir = link.ok().and_then(|l| model::dir_from_link(&l));
        ctx.release_document(doc.grant);
        let Some(dir) = dir else {
            self.status = "Cannot find that folder's path.".to_string();
            return;
        };
        match model::add_font_set(&mut self.sets, &dir) {
            Ok(name) => {
                self.save_sets(&ctx.data_dir);
                self.switch_library(&ctx.data_dir, &name);
            }
            Err(e) => self.status = e,
        }
    }

    /// The selected family's font file, when it lies in the user's home (Show in Files needs a grant for it).
    fn revealable(&self, name: &str) -> bool {
        self.library.as_ref().and_then(|l| l.find(name)).is_some_and(|f| model::can_reveal(Path::new(&f.path), &self.home))
    }

    /// Show in Files: asks the shell for a grant on the font's file; the answer arrives in `document`.
    fn show_in_files(&mut self, ctx: &Ctx, name: &str) {
        let Some(path) = self.library.as_ref().and_then(|l| l.find(name)).map(|f| f.path.clone()) else { return };
        if !model::can_reveal(Path::new(&path), &self.home) {
            self.status = "Only fonts in your home folder can be shown in Files.".to_string();
            return;
        }
        self.reveal = Some(ctx.grant_path(&path));
    }

    /// The grant for the font's file arrived: reveal it, then let go of the grant.
    fn reveal_granted(&mut self, ctx: &Ctx, doc: bifrost_app::Document) {
        let notice = self.notice.clone();
        let grant = doc.grant;
        workspace::activate_file_viewer_selecting(ctx, &[grant], move |status| {
            if status != workspace::Status::Ok {
                *notice.borrow_mut() = "Files could not show the font.".to_string();
            }
        });
        ctx.release_document(grant);
    }

    /// Export Font: the save panel for a copy of the family's font file, written when the panel answers.
    fn export_font(&mut self, ctx: &Ctx, name: &str) {
        let Some(path) = self.library.as_ref().and_then(|l| l.find(name)).map(|f| f.path.clone()) else { return };
        let request = ctx.save_document(&model::export_name(&path), &["ttf", "otf"]);
        self.export = Some((request, PathBuf::from(path)));
    }

    /// The save panel answered an export: copy the font file into the file the panel made, then put it in place.
    fn write_export(&mut self, ctx: &Ctx, mut doc: bifrost_app::Document) {
        let Some((request, source)) = self.export.take() else {
            ctx.abort_document(doc.request);
            return;
        };
        if request != doc.request {
            self.export = Some((request, source));
            return;
        }
        match std::fs::read(&source).map_err(|e| e.to_string()).and_then(|bytes| doc.file.write_all(&bytes).map_err(|e| e.to_string())) {
            Ok(()) => {
                ctx.commit_document(request);
                self.status = format!("Exported {}.", doc.name);
            }
            Err(e) => {
                ctx.abort_document(request);
                self.status = format!("Could not export the font: {e}");
            }
        }
    }

    /// The font files of the families the list shows now, in list order.
    fn shown_paths(&self) -> Vec<PathBuf> {
        self.library.as_ref().map(|l| l.visible(&self.source, &self.query).iter().map(|f| PathBuf::from(&f.path)).collect()).unwrap_or_default()
    }

    /// The folder panel answered Export Fonts to Folder: each font file in `files` is copied into the folder, under a
    /// name no file there uses. The folder is reached through its descriptor, so the copies are made inside it.
    fn export_into_folder(&mut self, ctx: &Ctx, doc: bifrost_app::Document, files: Vec<PathBuf>) {
        use std::os::fd::AsRawFd;
        let dest = PathBuf::from(format!("/proc/self/fd/{}", doc.file.as_raw_fd()));
        ctx.release_document(doc.grant);
        let existing: Vec<String> = std::fs::read_dir(&dest)
            .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let failed = export::plan(&files, &dest, &existing).iter().filter(|(src, target)| export::copy_new(src, target).is_err()).count();
        self.status = if failed == 0 {
            format!("Exported {} fonts to {}.", files.len(), doc.name)
        } else {
            format!("Exported {} of {} fonts; {failed} could not be copied.", files.len() - failed, files.len())
        };
    }

    /// The file chosen in Validate File is read and checked. Nothing is installed.
    fn check_chosen_file(&mut self, doc: bifrost_app::Document) {
        let mut bytes = Vec::new();
        match (&doc.file).read_to_end(&mut bytes) {
            Ok(_) => {
                self.status = format!("Checked {}; it is not installed.", doc.name);
                self.checked = Some((doc.name, validate::check(&bytes)));
            }
            Err(e) => self.status = format!("Cannot read {}: {e}", doc.name),
        }
    }

    /// Print Sample Sheet: sends the family's details and sample line to the default printer.
    fn print_sheet(&mut self, name: &str) {
        let Some(lib) = &self.library else { return };
        let sheet = model::sample_sheet(name, &lib.info(name), self.prefs.sample());
        self.status = match PrintOperation::text(name.to_string(), sheet).run() {
            Ok(_) => format!("Sent the sample sheet for {name} to the printer."),
            Err(PrintError::NoCups) => "The printing service is not running.".to_string(),
            Err(PrintError::NoPrinter) => "No printer is set up.".to_string(),
            Err(PrintError::Failed(why)) => format!("Printing failed: {why}"),
        };
    }

    fn save_prefs(&mut self, data_dir: &Path) {
        if let Err(e) = model::write_text(data_dir, model::PREFS_FILE, &self.prefs.to_text()) {
            self.status = format!("Could not save: {e}");
        }
    }

    /// The families the list shows now.
    fn shown(&self) -> Vec<String> {
        self.library.as_ref().map(|l| l.visible(&self.source, &self.query).iter().map(|f| f.family.clone()).collect()).unwrap_or_default()
    }

    /// Loads the preview font and its validation for the selected family, once per selection.
    fn ensure_preview(&mut self, scale: f32) {
        let Some(name) = self.selected.clone() else {
            self.preview = None;
            self.report = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|(n, _)| *n == name) {
            return;
        }
        let path = self.library.as_ref().and_then(|l| l.find(&name)).map(|f| f.path.clone());
        let bytes = path.and_then(|p| std::fs::read(&p).ok());
        self.report = bytes.as_deref().map(|b| (name.clone(), validate::check(b)));
        self.preview = bytes.and_then(|bytes| font_ui(bytes, scale).map(|ui| (name.clone(), ui)));
    }

    /// True when the family's file is one this user installed (the only fonts Remove Font may delete).
    fn is_user_font(&self, name: &str) -> bool {
        self.library.as_ref().and_then(|l| l.find(name)).is_some_and(|f| Path::new(&f.path).starts_with(&self.user_dir))
    }

    /// Opens the install preview for the font file at `path` (named `name`): its validation, family and a sample.
    fn begin_install(&mut self, ctx: &Ctx, path: PathBuf, name: &str) {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !model::INSTALLABLE.contains(&ext.as_str()) {
            self.status = "Only .ttf, .otf and .ttc fonts can be installed.".to_string();
            return;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Cannot read the font: {e}");
                return;
            }
        };
        let report = validate::check(&bytes);
        let family = fontprefs::family(&path.to_string_lossy());
        let font = font_ui(bytes, ctx.ui.s);
        self.status.clear();
        self.pending = Some(Pending { path, name: name.to_string(), family, report, font });
    }

    /// Install in the preview: copies the file into the user's fonts (checked first when the setting asks for it).
    fn install_pending(&mut self, ctx: &Ctx) {
        let Some(p) = self.pending.take() else { return };
        match model::install(&p.path, &self.user_dir, self.prefs.validate_before_install) {
            Ok((_, family)) => {
                self.status = format!("Installed {family}.");
                Self::refresh_font_cache(&self.user_dir);
                self.reload(&ctx.data_dir);
                self.source = Source::All;
                self.selected = Some(family);
            }
            Err(e) => {
                self.status = e;
                self.pending = Some(p);
                return;
            }
        }
        if p.path.starts_with(ctx.data_dir.join("staging")) {
            let _ = std::fs::remove_file(&p.path);
        }
    }

    fn cancel_install(&mut self, ctx: &Ctx) {
        if let Some(p) = self.pending.take() {
            if p.path.starts_with(ctx.data_dir.join("staging")) {
                let _ = std::fs::remove_file(&p.path);
            }
        }
    }

    /// Rebuilds the fontconfig cache of the user's fonts so other apps see the change. Best effort: a refresh inside the
    /// rate limit, or a missing fc-cache, leaves the install in place and the list still reads the folder.
    fn refresh_font_cache(user_dir: &Path) {
        let _ = fontprefs::refresh_user_cache(user_dir);
    }

/// Remove Font: deletes the selected family's file if the user installed it, then rereads the list.
    fn remove(&mut self, data_dir: &Path, name: &str) {
        let Some(path) = self.library.as_ref().and_then(|l| l.find(name)).map(|f| PathBuf::from(&f.path)) else { return };
        match model::uninstall(&path, &self.user_dir) {
            Ok(()) => {
                self.status = format!("Removed {name}.");
                self.selected = None;
                Self::refresh_font_cache(&self.user_dir);
                self.reload(data_dir);
            }
            Err(e) => self.status = e,
        }
    }

    /// Turns the family on or off and saves.
    fn toggle_enabled(&mut self, data_dir: &Path, name: &str) {
        if let Some(lib) = &mut self.library {
            let on = lib.is_enabled(name);
            lib.set_enabled(name, !on);
        }
        self.save(data_dir);
    }

    /// Adds the family to the collection, or takes it out when it is already a member, then saves.
    fn toggle_membership(&mut self, data_dir: &Path, collection: &str, family: &str) {
        let Some(lib) = &mut self.library else { return };
        let member = lib.members(collection).is_some_and(|m| m.iter().any(|x| x.eq_ignore_ascii_case(family)));
        if member {
            lib.remove_from_collection(collection, family);
        } else {
            lib.add_to_collection(collection, family);
        }
        self.save(data_dir);
    }

    /// Every collection name, plain and smart.
    fn all_collection_names(&self) -> Vec<String> {
        self.library.as_ref().map(|l| l.collection_names().into_iter().chain(l.smart_names()).collect()).unwrap_or_default()
    }

    /// New Collection: adds an empty collection with a free name, shows it in the list and starts renaming it.
    fn new_collection(&mut self, data_dir: &Path) {
        let name = model::unique_name(&self.all_collection_names(), "New Collection");
        let Some(lib) = &mut self.library else { return };
        if lib.create_collection(&name) {
            self.source = Source::Collection(name.clone());
            self.edit = Edit::Rename { old: name.clone(), smart: false, text: name };
            self.save(data_dir);
        }
    }

    /// New Smart Collection with `rule`: named after the rule, shown in the list and ready to rename.
    fn new_smart(&mut self, data_dir: &Path, rule: Rule) {
        let name = model::unique_name(&self.all_collection_names(), &rule.title());
        let Some(lib) = &mut self.library else { return };
        if lib.create_smart(&name, rule) {
            self.source = Source::Smart(name.clone());
            self.edit = Edit::Rename { old: name.clone(), smart: true, text: name };
            self.save(data_dir);
        }
    }

    /// The collection the sidebar shows, with whether it is smart; None for All Fonts and Disabled.
    fn current_collection(&self) -> Option<(String, bool)> {
        match &self.source {
            Source::Collection(n) => Some((n.clone(), false)),
            Source::Smart(n) => Some((n.clone(), true)),
            _ => None,
        }
    }

    fn delete_current(&mut self, data_dir: &Path) {
        let Some((name, smart)) = self.current_collection() else { return };
        let Some(lib) = &mut self.library else { return };
        let done = if smart { lib.delete_smart(&name) } else { lib.delete_collection(&name) };
        if done {
            self.status = format!("Deleted {name}.");
            self.source = Source::All;
            self.save(data_dir);
        }
    }

    fn start_rename(&mut self) {
        if let Some((name, smart)) = self.current_collection() {
            self.edit = Edit::Rename { old: name.clone(), smart, text: name };
        }
    }

    /// Ends the text edit, keeping what was typed.
    fn commit_edit(&mut self, data_dir: &Path) {
        match std::mem::replace(&mut self.edit, Edit::None) {
            Edit::None => {}
            Edit::Sample(text) => {
                self.prefs.sample_text = text;
                self.save_prefs(data_dir);
            }
            Edit::Rename { old, smart, text } => {
                let Some(lib) = &mut self.library else { return };
                if lib.rename_collection(&old, &text) {
                    let new = text.trim().to_string();
                    if self.source == Source::Collection(old.clone()) || self.source == Source::Smart(old.clone()) {
                        self.source = if smart { Source::Smart(new) } else { Source::Collection(new) };
                    }
                    self.save(data_dir);
                } else if text.trim() != old {
                    self.status = format!("Cannot rename to \"{}\": the name is empty or already used.", text.trim());
                }
            }
        }
    }

    /// Resolve Duplicates: the first call lists the families with several files, the second removes the older files.
    fn resolve_duplicates(&mut self, data_dir: &Path) {
        if self.duplicates.is_empty() {
            self.duplicates = model::find_duplicates(&model::scan_files(&self.user_dir));
            if self.duplicates.is_empty() {
                self.status = "No duplicate fonts found among your installs.".to_string();
            } else {
                let older: usize = self.duplicates.iter().map(|d| d.remove.len()).sum();
                self.status = format!("{} families have duplicates ({older} older files). Choose Resolve Duplicates again to keep the newest.", self.duplicates.len());
            }
            return;
        }
        let dups = std::mem::take(&mut self.duplicates);
        let removed = model::resolve_duplicates(&dups, &self.user_dir);
        self.status = format!("Removed {removed} older files.");
        self.reload(data_dir);
    }

    fn change_size(&mut self, data_dir: &Path, delta: f32) {
        let size = self.prefs.sample_size + delta;
        self.prefs.set_size(size);
        self.save_prefs(data_dir);
    }

    /// A press in the sidebar, the list or the preview column (window points, scaled by the caller).
    fn touch_down(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        self.commit_edit(&ctx.data_dir);
        self.search_focused = false;
        if self.library.is_none() {
            return false;
        }
        let preview_x = SIDEBAR_PT + LIST_PT + PAD_PT;
        if self.pending.is_some() {
            return self.pending_touch(ctx, x, y, preview_x);
        }
        if x < SIDEBAR_PT + LIST_PT {
            self.checked = None;
        }
        if x < SIDEBAR_PT {
            let rows = self.library.as_ref().map(sidebar_sources).unwrap_or_default();
            let index = ((y - HEADER_PT) / ROW_PT).floor();
            if y >= HEADER_PT && index >= 0.0 {
                let index = index as usize;
                if let Some((_, source)) = rows.get(index) {
                    self.source = source.clone();
                    self.query.clear();
                    return true;
                }
                if index == rows.len() {
                    self.new_collection(&ctx.data_dir);
                    return true;
                }
            }
            return false;
        }
        if x < SIDEBAR_PT + LIST_PT {
            self.search_focused = y < UNIFIED_BAR_PT;
            if self.search_focused {
                // the search field in the toolbar: its clear button empties the query
                if bifrost_app::controls::search::hits_clear(search_bounds(), !self.query.is_empty(), x, y) {
                    self.query.clear();
                    self.search_focused = false;
                }
                return true;
            }
            let index = ((y - LIST_TOP_PT) / ROW_PT).floor();
            if y >= LIST_TOP_PT && index >= 0.0 {
                if let Some(name) = self.shown().get(index as usize).cloned() {
                    self.selected = Some(name);
                    return true;
                }
            }
            return false;
        }
        let Some(name) = self.selected.clone() else { return false };
        let remove_x = preview_x + BUTTON_GAP_PT;
        // the on/off switch and Remove Font sit at the top of the preview column
        if (BAR_BUTTON_Y_PT..BAR_BUTTON_Y_PT + BAR_BUTTON_H_PT).contains(&y) {
            if (preview_x..preview_x + BUTTON_PT).contains(&x) {
                self.toggle_enabled(&ctx.data_dir, &name);
                return true;
            }
            if (remove_x..remove_x + BUTTON_PT).contains(&x) && self.is_user_font(&name) {
                self.remove(&ctx.data_dir, &name);
                return true;
            }
        }
        // the mode tabs
        if (TABS_TOP_PT..TABS_TOP_PT + TAB_H_PT).contains(&y) && x >= preview_x {
            let index = ((x - preview_x) / TAB_PT).floor() as usize;
            if let Some(mode) = [Mode::Sample, Mode::Repertoire, Mode::Info].get(index) {
                self.mode = *mode;
                return true;
            }
        }
        match self.mode {
            Mode::Sample => self.sample_touch(ctx, x, y, preview_x),
            Mode::Info => {
                // one row per collection: a press adds the family to it or takes it out
                let rows_top = MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT;
                if (preview_x..preview_x + 260.0).contains(&x) && y >= rows_top {
                    let index = ((y - rows_top) / MEMBERSHIP_ROW_PT).floor() as usize;
                    let collection = self.library.as_ref().and_then(|l| l.collection_names().get(index).cloned());
                    if let Some(collection) = collection {
                        self.toggle_membership(&ctx.data_dir, &collection, &name);
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// A press in Sample mode: the sample field starts editing, the buttons change the size.
    fn sample_touch(&mut self, ctx: &Ctx, x: f32, y: f32, preview_x: f32) -> bool {
        if !(CONTENT_TOP_PT..CONTENT_TOP_PT + FIELD_H_PT).contains(&y) {
            return false;
        }
        if (preview_x..preview_x + FIELD_W_PT).contains(&x) {
            self.edit = Edit::Sample(self.prefs.sample_text.clone());
            return true;
        }
        let smaller_x = preview_x + FIELD_W_PT + 12.0;
        if (smaller_x..smaller_x + SIZE_BTN_PT).contains(&x) {
            self.change_size(&ctx.data_dir, -SIZE_STEP);
            return true;
        }
        let bigger_x = smaller_x + SIZE_BTN_PT + 4.0;
        if (bigger_x..bigger_x + SIZE_BTN_PT).contains(&x) {
            self.change_size(&ctx.data_dir, SIZE_STEP);
            return true;
        }
        false
    }

    /// A press while the install preview is up: Install or Cancel.
    fn pending_touch(&mut self, ctx: &Ctx, x: f32, y: f32, preview_x: f32) -> bool {
        if !(BAR_BUTTON_Y_PT..BAR_BUTTON_Y_PT + BAR_BUTTON_H_PT).contains(&y) {
            return false;
        }
        if (preview_x..preview_x + BUTTON_PT).contains(&x) {
            let can = self.pending.as_ref().is_some_and(|p| p.family.is_some() && (!self.prefs.validate_before_install || p.report.is_valid()));
            if can {
                self.install_pending(ctx);
            } else {
                self.status = "This font fails validation and cannot be installed.".to_string();
            }
            return true;
        }
        if (preview_x + BUTTON_GAP_PT..preview_x + BUTTON_GAP_PT + BUTTON_PT).contains(&x) {
            self.cancel_install(ctx);
            return true;
        }
        false
    }

    /// Selects the family a `fontbook:` link names.
    fn open_link(&mut self, url: &str) -> bool {
        let Some(wanted) = model::family_from_url(url) else { return false };
        let found = self.library.as_ref().and_then(|l| l.find(&wanted)).map(|f| f.family.clone());
        match found {
            Some(family) => {
                self.source = Source::All;
                self.query.clear();
                self.selected = Some(family);
                self.mode = Mode::Sample;
                self.status.clear();
            }
            None => self.status = format!("No font named {wanted} is installed."),
        }
        true
    }
}

/// A drawing context for the font in `bytes`, at `scale`. The bytes live for the whole run: one per selection or
/// install preview, so the leak is bounded by what was viewed.
fn font_ui(bytes: Vec<u8>, scale: f32) -> Option<Ui> {
    let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    Font::parse(data).map(|font| Ui::new(font, scale))
}

/// The sidebar rows in order: All Fonts, Disabled, the collections, then the smart collections.
fn sidebar_sources(lib: &Library) -> Vec<(String, Source)> {
    let mut rows = vec![("All Fonts".to_string(), Source::All), ("Disabled".to_string(), Source::Disabled)];
    rows.extend(lib.collection_names().into_iter().map(|n| (n.clone(), Source::Collection(n))));
    rows.extend(lib.smart_names().into_iter().map(|n| (n.clone(), Source::Smart(n))));
    rows
}

/// The toolbar's search capsule in window points: the list column's width less its padding, centred in the unified bar.
fn search_bounds() -> Rect {
    Rect { x: SIDEBAR_PT + PAD_PT, y: BAR_BUTTON_Y_PT, w: LIST_PT - 2.0 * PAD_PT, h: BAR_BUTTON_H_PT }
}

/// Draws `text` at (`x`, `y`) points, cut to `max_w` points.
fn put(s: &mut Surface, ui: &Ui, text: &str, pt: f32, colour: u32, x: f32, y: f32, max_w: f32) {
    let img = ui.text(&ui.fit(text, pt, ui.pt(max_w)), pt, Weight::Regular, colour);
    s.blit_colored(&img, (x * ui.s) as i32, (y * ui.s) as i32, colour, 255);
}

/// A filled button of `w` points with its label.
fn button(s: &mut Surface, ui: &Ui, label: &str, fill: u32, x: f32, y: f32, w: f32, h: f32) {
    let p = |v: f32| (v * ui.s) as usize;
    // published as a control: in a full-size window the unified bar is a drag zone that only controls take presses in
    ax::record(Role::Button, label, "", Rect { x, y, w, h }, true);
    s.fill_rect(p(x), p(y), p(w), p(h), fill);
    let colour = ui::LABEL.now();
    let img = ui.text(&ui.fit(label, ui::MAC_BODY_PT, ui.pt(w - 16.0)), ui::MAC_BODY_PT, Weight::Regular, colour);
    s.blit_colored(&img, ((x + 8.0) * ui.s) as i32, ((y + 6.0) * ui.s) as i32, colour, 255);
}

impl App for FontBookApp {
    fn id(&self) -> &str {
        "is.olibuijr.fontbook.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // all changes are saved as they are made
        if self.library.is_none() {
            self.reload(&ctx.data_dir);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                self.touch_down(ctx, x, y)
            }
            Input::Key(KeyInput::Text(text)) => {
                match &mut self.edit {
                    Edit::Sample(buf) => buf.push_str(&text),
                    Edit::Rename { text: buf, .. } => buf.push_str(&text),
                    Edit::None => {
                        self.query.push_str(&text);
                        self.selected = None;
                    }
                }
                true
            }
            Input::Key(KeyInput::Code(8)) => {
                match &mut self.edit {
                    Edit::Sample(buf) => {
                        buf.pop();
                    }
                    Edit::Rename { text: buf, .. } => {
                        buf.pop();
                    }
                    Edit::None => {
                        self.query.pop();
                    }
                }
                true
            }
            Input::Key(KeyInput::Code(13)) | Input::Key(KeyInput::Code(10)) if self.edit != Edit::None => {
                self.commit_edit(&ctx.data_dir);
                true
            }
            Input::Key(KeyInput::Code(1 | 27)) => {
                if self.edit != Edit::None {
                    self.edit = Edit::None;
                } else if self.pending.is_some() {
                    self.cancel_install(ctx);
                } else if self.checked.is_some() {
                    self.checked = None;
                } else {
                    self.query.clear();
                }
                true
            }
            Input::Open(path) => {
                if self.open_link(&path) {
                    return true;
                }
                let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.begin_install(ctx, PathBuf::from(&path), &name);
                true
            }
            _ => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let doc = match ev {
            DocumentEvent::Opened(doc) if self.add_library == Some(doc.request) => {
                self.add_library = None;
                self.add_library_folder(ctx, doc);
                return true;
            }
            DocumentEvent::Opened(doc) if self.reveal == Some(doc.request) => {
                self.reveal = None;
                self.reveal_granted(ctx, doc);
                return true;
            }
            DocumentEvent::Opened(doc) if self.export_folder.as_ref().is_some_and(|(r, _)| *r == doc.request) => {
                if let Some((_, files)) = self.export_folder.take() {
                    self.export_into_folder(ctx, doc, files);
                }
                return true;
            }
            DocumentEvent::Opened(doc) if self.validate_file == Some(doc.request) => {
                self.validate_file = None;
                self.check_chosen_file(doc);
                return true;
            }
            DocumentEvent::SaveTo(doc) => {
                self.write_export(ctx, doc);
                return true;
            }
            DocumentEvent::Failed { request, code } => {
                if self.add_library == Some(request) {
                    self.add_library = None;
                }
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export = None;
                }
                if self.export_folder.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export_folder = None;
                }
                if self.validate_file == Some(request) {
                    self.validate_file = None;
                }
                if self.reveal == Some(request) {
                    self.reveal = None;
                    self.status = "Files cannot show that font.".to_string();
                    return true;
                }
                if code != DocErrorCode::Cancelled {
                    self.status = format!("The file panel refused: {code:?}.");
                    return true;
                }
                return false;
            }
            DocumentEvent::Opened(doc) => doc,
            DocumentEvent::OpenedMany { docs, .. } => match docs.into_iter().next() {
                Some(doc) => doc,
                None => return false,
            },
            _ => return false,
        };
        let mut bytes = Vec::new();
        if let Err(e) = (&doc.file).read_to_end(&mut bytes) {
            self.status = format!("Cannot read {}: {e}", doc.name);
            return true;
        }
        match model::stage(&bytes, &doc.name, &ctx.data_dir) {
            Ok(path) => self.begin_install(ctx, path, &doc.name),
            Err(e) => self.status = format!("Cannot prepare {}: {e}", doc.name),
        }
        true
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let has_font = self.selected.is_some() && self.pending.is_none();
        let is_user = self.selected.as_deref().is_some_and(|n| self.is_user_font(n));
        let on = self.selected.as_deref().and_then(|n| self.library.as_ref().map(|l| l.is_enabled(n)));
        let collection = self.current_collection().is_some();
        let has_query = !self.query.trim().is_empty();
        let reveal = self.selected.as_deref().is_some_and(|n| self.revealable(n));
        let has_list = !self.shown_paths().is_empty();
        let mut items = vec![
                AppMenuItem::new(MENU_ADD, "Add Fonts...").key("cmd+o"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_NEW_COLLECTION, "New Collection").key("cmd+shift+n"),
                AppMenuItem::submenu("New Smart Collection"),
                AppMenuItem::new(MENU_SMART_FIXED, "Fixed Width Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_VARIABLE, "Variable Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_STATIC, "Static Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_TTF, "TrueType Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_OTF, "OpenType Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_ITALIC, "Italic Fonts").depth(1),
                AppMenuItem::new(MENU_SMART_NAME, "Name Contains Search Text").enabled(has_query).depth(1),
                AppMenuItem::new(MENU_RENAME, "Rename Collection").enabled(collection),
                AppMenuItem::new(MENU_DELETE, "Delete Collection").enabled(collection),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_TOGGLE, if on == Some(false) { "Turn Font On" } else { "Turn Font Off" }).enabled(has_font),
                AppMenuItem::new(MENU_REMOVE, "Remove Font").enabled(has_font && is_user),
                AppMenuItem::new(MENU_VALIDATE, "Validate Font").key("cmd+shift+v").enabled(has_font),
                AppMenuItem::new(MENU_VALIDATE_FILE, "Validate File..."),
                AppMenuItem::new(MENU_DUPLICATES, if self.duplicates.is_empty() { "Resolve Duplicates" } else { "Resolve Duplicates (Confirm)" }),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_SAMPLE, "Sample").key("cmd+1").checked(self.mode == Mode::Sample),
                AppMenuItem::new(MENU_REPERTOIRE, "Repertoire").key("cmd+2").checked(self.mode == Mode::Repertoire),
                AppMenuItem::new(MENU_INFO, "Info").key("cmd+3").checked(self.mode == Mode::Info),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_EDIT_TEXT, "Edit Sample Text").key("cmd+t").enabled(has_font),
                AppMenuItem::new(MENU_BIGGER, "Bigger").key("cmd++"),
                AppMenuItem::new(MENU_SMALLER, "Smaller").key("cmd+-"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_VALIDATE_FIRST, "Validate Fonts Before Installing").checked(self.prefs.validate_before_install),
                AppMenuItem::separator(),
                AppMenuItem::submenu("Library"),
        ];
        for (i, name) in model::library_names(&self.sets).iter().enumerate() {
            let on = name.eq_ignore_ascii_case(&self.prefs.library);
            items.push(AppMenuItem::new(MENU_LIBRARY_BASE + i as u32, name).checked(on).depth(1));
        }
        items.extend([
            AppMenuItem::new(MENU_LIBRARY_ADD, "Add Library Folder..."),
            AppMenuItem::new(MENU_LIBRARY_REMOVE, "Remove Library").enabled(model::is_custom_library(&self.sets, &self.prefs.library)),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_REVEAL, "Show in Files").enabled(has_font && reveal),
            AppMenuItem::new(MENU_EXPORT, "Export Font...").key("cmd+e").enabled(has_font),
            AppMenuItem::new(MENU_EXPORT_FOLDER, "Export Fonts to Folder...").enabled(has_list),
            AppMenuItem::new(MENU_PRINT, "Print Sample Sheet").key("cmd+p").enabled(has_font),
        ]);
        vec![AppMenu { title: "Font".into(), items }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let data = ctx.data_dir.clone();
        self.commit_edit(&data);
        match id {
            MENU_ADD => {
                let options = OpenOptions { title: "Add Fonts".into(), filters: vec!["ttf".into(), "otf".into(), "ttc".into()], ..OpenOptions::default() };
                let _ = ctx.open_panel(&options);
            }
            MENU_NEW_COLLECTION => self.new_collection(&data),
            MENU_SMART_FIXED => self.new_smart(&data, Rule::Fixed),
            MENU_SMART_VARIABLE => self.new_smart(&data, Rule::Variable),
            MENU_SMART_STATIC => self.new_smart(&data, Rule::Static),
            MENU_SMART_TTF => self.new_smart(&data, Rule::TrueType),
            MENU_SMART_OTF => self.new_smart(&data, Rule::OpenType),
            MENU_SMART_ITALIC => self.new_smart(&data, Rule::Italic),
            MENU_SMART_NAME => {
                let text = self.query.trim().to_string();
                if !text.is_empty() {
                    self.new_smart(&data, Rule::NameContains(text));
                }
            }
            MENU_RENAME => self.start_rename(),
            MENU_DELETE => self.delete_current(&data),
            MENU_VALIDATE => {
                self.checked = None;
                self.mode = Mode::Validation;
            }
            MENU_VALIDATE_FILE => {
                let options = OpenOptions { title: "Validate Font File".into(), filters: vec!["ttf".into(), "otf".into(), "ttc".into()], ..OpenOptions::default() };
                self.validate_file = Some(ctx.open_panel(&options));
            }
            MENU_EXPORT_FOLDER => {
                let files = self.shown_paths();
                let options = OpenOptions { title: "Export Fonts".into(), files: false, directories: true, create_directories: true, ..OpenOptions::default() };
                self.export_folder = Some((ctx.open_panel(&options), files));
            }
            MENU_DUPLICATES => self.resolve_duplicates(&data),
            MENU_SAMPLE => self.mode = Mode::Sample,
            MENU_REPERTOIRE => self.mode = Mode::Repertoire,
            MENU_INFO => self.mode = Mode::Info,
            MENU_EDIT_TEXT => {
                self.mode = Mode::Sample;
                self.edit = Edit::Sample(self.prefs.sample_text.clone());
            }
            MENU_BIGGER => self.change_size(&data, SIZE_STEP),
            MENU_SMALLER => self.change_size(&data, -SIZE_STEP),
            MENU_VALIDATE_FIRST => {
                self.prefs.validate_before_install = !self.prefs.validate_before_install;
                self.save_prefs(&data);
            }
            MENU_TOGGLE => {
                if let Some(name) = self.selected.clone() {
                    self.toggle_enabled(&data, &name);
                }
            }
            MENU_REMOVE => {
                if let Some(name) = self.selected.clone() {
                    if self.is_user_font(&name) {
                        self.remove(&data, &name);
                    }
                }
            }
            MENU_LIBRARY_ADD => {
                let options = OpenOptions { title: "Add Library Folder".into(), files: false, directories: true, create_directories: true, ..OpenOptions::default() };
                self.add_library = Some(ctx.open_panel(&options));
            }
            MENU_LIBRARY_REMOVE => self.remove_library(&data),
            MENU_REVEAL => {
                if let Some(name) = self.selected.clone() {
                    self.show_in_files(ctx, &name);
                }
            }
            MENU_EXPORT => {
                if let Some(name) = self.selected.clone() {
                    self.export_font(ctx, &name);
                }
            }
            MENU_PRINT => {
                if let Some(name) = self.selected.clone() {
                    self.print_sheet(&name);
                }
            }
            id if id >= MENU_LIBRARY_BASE => {
                let names = model::library_names(&self.sets);
                match names.get((id - MENU_LIBRARY_BASE) as usize) {
                    Some(name) => self.switch_library(&data, name),
                    None => return false,
                }
            }
            _ => return false,
        }
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let scale = ui.s;
        let notice = std::mem::take(&mut *self.notice.borrow_mut());
        if !notice.is_empty() {
            self.status = notice;
        }
        self.ensure_preview(scale);
        let p = |v: f32| (v * scale) as usize;
        let (w, h) = (s.w, s.h);
        let (wp, hp) = (w as f32 / scale, h as f32 / scale);
        s.fill_rect(0, 0, w, h, ui::CONTENT_BG.now());
        // Sidebar source list uses the shared Golden Gate neutral selection and row treatment.
        let rows = self.library.as_ref().map(sidebar_sources).unwrap_or_default();
        let entries = rows.iter().enumerate().map(|(i, (label, source))| {
            let editing = match (&self.edit, source) {
                (Edit::Rename { old, smart, text }, Source::Collection(n)) if !*smart && old == n => Some(text.as_str()),
                (Edit::Rename { old, smart, text }, Source::Smart(n)) if *smart && old == n => Some(text.as_str()),
                _ => None,
            };
            let shown = editing.map_or(label.as_str(), |text| text);
            let symbol = match i { 0 => "textformat", 1 => "eye.slash", _ => "folder" };
            SidebarItem::row(i as u32 + 1, shown).with_symbol(symbol)
        }).collect();
        let mut entries: Vec<SidebarItem> = entries;
        entries.push(SidebarItem::row(rows.len() as u32 + 1, "New Collection").with_symbol("plus"));
        let mut sidebar = SourceList::new(entries, ROW_PT, ROW_PT);
        sidebar.set_selected(rows.iter().position(|(_, source)| *source == self.source).map(|i| i as u32 + 1));
        widgets::source_list(s, ui, bifrost_app::controls::Rect { x: 0.0, y: HEADER_PT, w: SIDEBAR_PT, h: (hp - HEADER_PT).max(0.0) }, &sidebar, 0.0, None);
        // list column
        let x0 = SIDEBAR_PT;
        // the unified toolbar over the list and the preview, with the family search in its list half
        widgets::toolbar_band(s, ui, Rect { x: x0, y: 0.0, w: (wp - x0).max(0.0), h: UNIFIED_BAR_PT });
        let mut query = TextField::new();
        query.set_text(&self.query);
        widgets::search_field(s, ui, &query, search_bounds(), "Search", self.search_focused || !self.query.is_empty());
        s.fill_rect(p(x0 + LIST_PT - 1.0), p(UNIFIED_BAR_PT), 1, h.saturating_sub(p(UNIFIED_BAR_PT)), ui::SEPARATOR_C.now());
        let shown = self.shown();
        let heading = match (self.query.is_empty(), self.prefs.library.eq_ignore_ascii_case(model::LIBRARY_COMPUTER)) {
            (true, true) => format!("{} families", shown.len()),
            (true, false) => format!("{} families in {}", shown.len(), self.prefs.library),
            (false, _) => format!("{} families matching \"{}\"", shown.len(), self.query),
        };
        put(s, ui, &heading, ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), x0 + PAD_PT, HEADER_PT + 2.0, LIST_PT - 2.0 * PAD_PT);
        for (i, name) in shown.iter().enumerate() {
            let y = LIST_TOP_PT + i as f32 * ROW_PT;
            if y > hp {
                break;
            }
            let on = self.library.as_ref().is_none_or(|l| l.is_enabled(name));
            if self.selected.as_deref() == Some(name.as_str()) {
                s.fill_rect(p(x0 + 6.0), p(y), p(LIST_PT - 12.0), p(ROW_PT), ui::SIDEBAR_PLATE.now());
            }
            let colour = if on { ui::LABEL.now() } else { ui::LABEL_2.now() };
            put(s, ui, name, ui::MAC_BODY_PT, colour, x0 + PAD_PT, y + 6.0, LIST_PT - 2.0 * PAD_PT);
        }
        // preview column
        let px = SIDEBAR_PT + LIST_PT + PAD_PT;
        let avail = wp - px - PAD_PT;
        if self.pending.is_some() {
            self.draw_pending(ui, s, px, avail);
        } else if let Some((name, report)) = &self.checked {
            put(s, ui, name, ui::MAC_TITLE2_PT, ui::LABEL.now(), px, HEADER_PT + PAD_PT, avail);
            put(s, ui, "Checked, not installed. Select a font to leave this view.", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, HEADER_PT + 44.0, avail);
            self.draw_validation(ui, s, Some(report), px, avail);
        } else if let Some(name) = self.selected.clone() {
            self.draw_selected(ui, s, &name, px, avail, hp);
        } else {
            put(s, ui, "Select a font to preview it", ui::MAC_BODY_PT, ui::LABEL_2.now(), px, HEADER_PT + PAD_PT, avail);
        }
        if !self.status.is_empty() {
            put(s, ui, &self.status, ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, hp - 24.0, avail);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

impl FontBookApp {
    /// The install preview: Install and Cancel, the validation verdict, and a sample in the candidate font.
    fn draw_pending(&self, ui: &Ui, s: &mut Surface, px: f32, avail: f32) {
        let Some(pending) = &self.pending else { return };
        let can = pending.family.is_some() && (!self.prefs.validate_before_install || pending.report.is_valid());
        button(s, ui, "Install", if can { ui::LINK } else { ui::SEPARATOR_C.now() }, px, BAR_BUTTON_Y_PT, BUTTON_PT, BAR_BUTTON_H_PT);
        button(s, ui, "Cancel", ui::SEPARATOR_C.now(), px + BUTTON_GAP_PT, BAR_BUTTON_Y_PT, BUTTON_PT, BAR_BUTTON_H_PT);
        let title = pending.family.clone().unwrap_or_else(|| pending.name.clone());
        put(s, ui, &title, ui::MAC_TITLE2_PT, ui::LABEL.now(), px, HEADER_PT + PAD_PT, avail);
        let verdict = match pending.report.verdict() {
            validate::Verdict::Pass => "Validation: passed".to_string(),
            validate::Verdict::Warn => format!("Validation: passed with {} warnings", pending.report.warnings.len()),
            validate::Verdict::Fail => "Validation: failed".to_string(),
        };
        put(s, ui, &format!("{} - {verdict}", pending.name), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, HEADER_PT + 44.0, avail);
        let mut y = CONTENT_TOP_PT;
        if let Some(font) = &pending.font {
            let size = self.prefs.sample_size;
            let img = font.text(&font.fit(self.prefs.sample(), size, font.pt(avail)), size, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, (px * ui.s) as i32, (y * ui.s) as i32, ui::LABEL.now(), 255);
            y += size * 1.6 + 8.0;
        }
        let mut rows = model::repertoire_rows(&pending.report);
        if pending.family.is_none() {
            rows.push(("Family".to_string(), "This desktop cannot draw the file".to_string()));
        }
        for (k, v) in rows {
            put(s, ui, &format!("{k}: {v}"), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, y, avail);
            y += 18.0;
        }
        if !can {
            put(s, ui, "Install is off for a font that fails validation (Font > Validate Fonts Before Installing).", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, y + 8.0, avail);
        }
    }

    /// The selected family: switch and Remove Font, the name, the mode tabs and the mode's content.
    fn draw_selected(&self, ui: &Ui, s: &mut Surface, name: &str, px: f32, avail: f32, hp: f32) {
        let on = self.library.as_ref().is_none_or(|l| l.is_enabled(name));
        button(s, ui, if on { "Turn Off" } else { "Turn On" }, if on { ui::LINK } else { ui::SEPARATOR_C.now() }, px, BAR_BUTTON_Y_PT, BUTTON_PT, BAR_BUTTON_H_PT);
        if self.is_user_font(name) {
            button(s, ui, "Remove Font", ui::DESTRUCTIVE, px + BUTTON_GAP_PT, BAR_BUTTON_Y_PT, BUTTON_PT, BAR_BUTTON_H_PT);
        }
        put(s, ui, name, ui::MAC_TITLE2_PT, ui::LABEL.now(), px, HEADER_PT + PAD_PT, avail);
        for (i, (label, mode)) in [("Sample", Mode::Sample), ("Repertoire", Mode::Repertoire), ("Info", Mode::Info)].iter().enumerate() {
            let x = px + i as f32 * TAB_PT;
            let active = self.mode == *mode;
            let p = |v: f32| (v * ui.s) as usize;
            if active {
                s.fill_rect(p(x), p(TABS_TOP_PT), p(TAB_PT - 4.0), p(TAB_H_PT), ui::SIDEBAR_PLATE.now());
            }
            put(s, ui, label, ui::MAC_BODY_PT, ui::LABEL.now(), x + 10.0, TABS_TOP_PT + 4.0, TAB_PT - 20.0);
        }
        let preview = self.preview.as_ref().filter(|(n, _)| n == name).map(|(_, u)| u);
        let report = self.report.as_ref().filter(|(n, _)| n == name).map(|(_, r)| r);
        match self.mode {
            Mode::Sample => self.draw_sample(ui, s, preview, px, avail, hp),
            Mode::Repertoire => self.draw_repertoire(ui, s, preview, report, px, hp),
            Mode::Info => self.draw_info(ui, s, name, report, px, avail),
            Mode::Validation => self.draw_validation(ui, s, report, px, avail),
        }
    }

    /// Sample mode: the editable sample field with the size buttons, the sample at that size, and a smaller ladder.
    fn draw_sample(&self, ui: &Ui, s: &mut Surface, preview: Option<&Ui>, px: f32, avail: f32, hp: f32) {
        let p = |v: f32| (v * ui.s) as usize;
        s.fill_rect(p(px), p(CONTENT_TOP_PT), p(FIELD_W_PT), p(FIELD_H_PT), ui::FIELD_FILL);
        let editing = if let Edit::Sample(buf) = &self.edit { Some(format!("{buf}|")) } else { None };
        let (text, colour) = match (&editing, self.prefs.sample_text.trim().is_empty()) {
            (Some(t), _) => (t.clone(), ui::LABEL.now()),
            (None, true) => ("Click to type your own text".to_string(), ui::LABEL_2.now()),
            (None, false) => (self.prefs.sample_text.clone(), ui::LABEL.now()),
        };
        put(s, ui, &text, ui::MAC_BODY_PT, colour, px + 8.0, CONTENT_TOP_PT + 5.0, FIELD_W_PT - 16.0);
        let smaller_x = px + FIELD_W_PT + 12.0;
        let bigger_x = smaller_x + SIZE_BTN_PT + 4.0;
        button(s, ui, "-", ui::SEPARATOR_C.now(), smaller_x, CONTENT_TOP_PT, SIZE_BTN_PT, FIELD_H_PT);
        button(s, ui, "+", ui::SEPARATOR_C.now(), bigger_x, CONTENT_TOP_PT, SIZE_BTN_PT, FIELD_H_PT);
        put(s, ui, &format!("{} pt", self.prefs.sample_size.round() as i32), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), bigger_x + SIZE_BTN_PT + 8.0, CONTENT_TOP_PT + 6.0, 60.0);
        let Some(font) = preview else { return };
        let mut y = CONTENT_TOP_PT + FIELD_H_PT + 14.0;
        let mut sizes = vec![self.prefs.sample_size];
        sizes.extend(LADDER_SIZES.iter().rev().copied().filter(|v| *v < self.prefs.sample_size));
        for size in sizes {
            if y > hp {
                break;
            }
            let img = font.text(&font.fit(self.prefs.sample(), size, font.pt(avail)), size, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, (px * ui.s) as i32, (y * ui.s) as i32, ui::LABEL.now(), 255);
            y += size * 1.6;
        }
    }

    /// Repertoire mode: the coverage numbers and a grid of the Latin characters in the font.
    fn draw_repertoire(&self, ui: &Ui, s: &mut Surface, preview: Option<&Ui>, report: Option<&validate::Report>, px: f32, hp: f32) {
        let mut y = CONTENT_TOP_PT;
        if let Some(r) = report {
            let line = format!("{} characters, {} glyphs", r.characters, r.glyphs);
            put(s, ui, &line, ui::MAC_BODY_PT, ui::LABEL.now(), px, y, 400.0);
            y += 22.0;
            let blocks = if r.blocks.is_empty() { "No Unicode blocks covered".to_string() } else { r.blocks.join(", ") };
            put(s, ui, &blocks, ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, y, 520.0);
            y += 26.0;
        }
        let Some(font) = preview else { return };
        let chars: Vec<char> = (0x21u32..0x7f).chain(0xa1..0x100).filter_map(char::from_u32).collect();
        for (i, c) in chars.iter().enumerate() {
            let cx = px + (i % GRID_COLS) as f32 * GRID_CELL_PT;
            let cy = y + (i / GRID_COLS) as f32 * GRID_CELL_PT;
            if cy > hp {
                break;
            }
            let img = font.text(&c.to_string(), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, (cx * ui.s) as i32, (cy * ui.s) as i32, ui::LABEL.now(), 255);
        }
    }

    /// Info mode: the info rows, then one row per collection to add the family to it or take it out.
    fn draw_info(&self, ui: &Ui, s: &mut Surface, name: &str, report: Option<&validate::Report>, px: f32, avail: f32) {
        let Some(lib) = &self.library else { return };
        let mut y = CONTENT_TOP_PT;
        let mut info = lib.info(name);
        if let Some(r) = report {
            info.extend(model::repertoire_rows(r));
        }
        for (k, v) in info {
            put(s, ui, &format!("{k}: {v}"), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, y, avail);
            y += 18.0;
        }
        put(s, ui, "Collections", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, MEMBERSHIP_TOP_PT, avail);
        let names = lib.collection_names();
        if names.is_empty() {
            put(s, ui, "No collections yet. Choose New Collection in the sidebar.", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT, avail);
        }
        for (i, collection) in names.iter().enumerate() {
            let member = lib.members(collection).is_some_and(|m| m.iter().any(|x| x.eq_ignore_ascii_case(name)));
            let label = if member { format!("Remove from {collection}") } else { format!("Add to {collection}") };
            put(s, ui, &label, ui::MAC_BODY_PT, ui::LINK, px, MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT * (i + 1) as f32, 260.0);
        }
    }

    /// The validation result of the selected font: pass or fail, then each problem.
    fn draw_validation(&self, ui: &Ui, s: &mut Surface, report: Option<&validate::Report>, px: f32, avail: f32) {
        let mut y = CONTENT_TOP_PT;
        let Some(r) = report else {
            put(s, ui, "The font file could not be read.", ui::MAC_BODY_PT, ui::LABEL_2.now(), px, y, avail);
            return;
        };
        let (verdict, colour) = match r.verdict() {
            validate::Verdict::Pass => ("Passed", ui::LABEL.now()),
            validate::Verdict::Warn => ("Passed with warnings", ui::LABEL.now()),
            validate::Verdict::Fail => ("Failed", ui::DESTRUCTIVE),
        };
        put(s, ui, &format!("Validation: {verdict}"), ui::MAC_TITLE3_PT, colour, px, y, avail);
        y += 30.0;
        for problem in &r.problems {
            put(s, ui, problem, ui::MAC_BODY_PT, ui::DESTRUCTIVE, px, y, avail);
            y += 22.0;
        }
        for warning in &r.warnings {
            put(s, ui, warning, ui::MAC_BODY_PT, ui::LABEL_2.now(), px, y, avail);
            y += 22.0;
        }
        if r.is_valid() {
            for (k, v) in model::repertoire_rows(r).into_iter().filter(|(k, _)| k != "Validation") {
                put(s, ui, &format!("{k}: {v}"), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), px, y, avail);
                y += 18.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn toolbar_controls_are_centred_in_the_unified_bar_and_the_search_sits_over_the_list() {
        // the Turn On/Off and Remove Font buttons and the search capsule keep the same inset above and below
        assert_eq!(BAR_BUTTON_Y_PT, UNIFIED_BAR_PT - BAR_BUTTON_Y_PT - BAR_BUTTON_H_PT);
        assert!(BAR_BUTTON_Y_PT > 0.0);
        let r = search_bounds();
        assert!(r.x >= SIDEBAR_PT && r.right() <= SIDEBAR_PT + LIST_PT, "the field stays over the list column");
        assert!(r.bottom() < UNIFIED_BAR_PT);
        // the rows and the preview start below the bar
        assert!(LIST_TOP_PT > UNIFIED_BAR_PT && HEADER_PT >= UNIFIED_BAR_PT);
    }

    #[test]
    fn sidebar_lists_all_off_collections_then_smart_collections() {
        let mut cols = BTreeMap::new();
        cols.insert("Work".to_string(), Vec::new());
        let smart = BTreeMap::from([("Mono".to_string(), Rule::Fixed)]);
        let lib = Library::new(Vec::new(), BTreeSet::new(), cols).with_smart(smart);
        let rows = sidebar_sources(&lib);
        let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["All Fonts", "Disabled", "Work", "Mono"]);
        assert_eq!(rows[3].1, Source::Smart("Mono".to_string()));
    }
}

fn main() {
    bifrost_app::run(FontBookApp {
        library: None,
        sets: Vec::new(),
        home: PathBuf::new(),
        user_dir: PathBuf::new(),
        source: Source::All,
        query: String::new(),
        search_focused: false,
        selected: None,
        mode: Mode::Sample,
        prefs: Prefs::default(),
        edit: Edit::None,
        pending: None,
        duplicates: Vec::new(),
        preview: None,
        report: None,
        status: String::new(),
        add_library: None,
        reveal: None,
        export: None,
        export_folder: None,
        validate_file: None,
        checked: None,
        notice: Rc::new(RefCell::new(String::new())),
    });
}
