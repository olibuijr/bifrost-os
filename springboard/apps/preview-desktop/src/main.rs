//! Viewer, a document window for images and PDFs: zoom and fit, rotation, page navigation with a
//! thumbnails sidebar, an outline, a markup list, bookmarks or search hits, page moves and deletes, markup
//! (highlight, note, shapes, underline, strike-through, freehand signature), Adjust Color through the image
//! filter API, picture edits (select and crop, flip, instant alpha, background removal), a contact sheet, a
//! slideshow, a magnifier, an Inspector, Find in PDF, PNG and JPEG export at a chosen size, and Export PDF,
//! which saves the markup, page order and rotation into a new PDF. A PDF is opened and drawn by its worker
//! thread (`pdfjob`); pages and markup live in `document` and `canvas`, the pure state and maths in `model`
//! and `pixels`, and the file writing in `export` and `pdfmarks`.

mod picture_tools;
mod canvas;
mod document;
mod export;
mod export_panel;
mod find;
mod forms;
mod inspector;
mod model;
mod password;
mod pdfjob;
mod pdfmarks;
mod pixels;
mod signature_panel;
mod prefs;
mod printjob;
mod recents;
mod signature;

use bifrost_app::controls::{Node, Role};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::services::{is_viewable_path, ServiceCall};
use bifrost_app::pasteboard::{NativePasteboard, PNG};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use bifrost_app::{screenshot, speech};
use bifrost_core::json::Json;
use canvas::{Action, Layout, Mode, PictureCache, Scene, Side, Tool};
use document::Document;
use export::Raster;
use model::{Adjust, History, Mark, MarkKind, Norm, Pixels, Snapshot, Trail, View, Zoom, HIGHLIGHT, MARK_BLUE, MARK_RED};
use password::Ask;
use pdfjob::{Inbox, Job, Jobs, Reply};
use prefs::Prefs;
use recents::Recents;
use signature::Signature;
use springboard::shell::books::engine;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// File > Open and the Open Recent items. The ids sit above every `Action` id.
const MENU_OPEN: u32 = 0x5001;
const MENU_CLEAR_RECENTS: u32 = 0x5002;
const MENU_RECENT: u32 = 0x5010;
/// Pages > Insert: a page from another PDF, and a blank page, both placed after the shown page.
const MENU_INSERT_FILE: u32 = 0x5003;
const MENU_INSERT_BLANK: u32 = 0x5004;
/// Tools > Insert Signature: one item per saved signature, from this id on.
const MENU_SIGNATURE: u32 = 0x5100;
/// Edit > Speech (speaks the selection, else the page) and File > Take Screenshot (selection, window, whole screen).
const MENU_SPEAK_START: u32 = 0x5005;
const MENU_SPEAK_STOP: u32 = 0x5006;
const MENU_SHOT_SELECTION: u32 = 0x5007;
const MENU_SHOT_WINDOW: u32 = 0x5008;
const MENU_SHOT_SCREEN: u32 = 0x5009;
/// The size a text box is dragged out to, as a share of the page, when a click places it.
const TEXT_BOX: (f32, f32) = (0.3, 0.06);
/// The width an inserted signature takes, as a share of the page.
const SIGNATURE_WIDTH: f32 = 0.3;
/// The kinds of file the open panel offers.
const OPEN_TYPES: [&str; 4] = ["pdf", "png", "jpg", "jpeg"];
/// The type a JPEG has on the pasteboard (the PNG type is `bifrost_app::pasteboard::PNG`).
const JPEG: &str = "public.jpeg";

/// A pointer gesture in progress. The coordinates are canvas or sidebar pixels.
enum Gesture {
    /// Dragging the picture or the text.
    Pan { last: (f32, f32) },
    /// Dragging the sidebar up or down.
    Scroll { last: f32 },
    /// Dragging out a highlight or a shape.
    Drag { start: (f32, f32), now: (f32, f32), kind: MarkKind },
    /// Drawing a stroke: its points so far.
    Ink { points: Vec<(f32, f32)> },
    /// Dragging out the selection rectangle.
    Select { start: (f32, f32), now: (f32, f32) },
    /// Dragging the contact sheet up or down; a short press picks the page under it.
    Sheet { last: f32, start: (f32, f32) },
    /// Dragging an Adjust Color slider. `before` is the document as it was when the drag began, for Undo.
    Slider { which: usize, before: Option<Snapshot> },
}

/// An export that goes to a file the user picks in the save panel (`NativeSavePanel`), so the user chooses the folder and
/// the name. The panel is asked for on the next frame; what is written goes into the file it answers with.
struct PendingExport {
    /// The name the panel suggests: `<name>-markup.<ext>`.
    name: String,
    ext: &'static str,
    /// The panel's request, once it is shown.
    request: Option<u32>,
    what: Export,
}

/// What an export writes into the chosen file.
enum Export {
    /// A picture, already encoded.
    Picture(Vec<u8>),
    /// A PDF, or a PDF page as a picture: the worker draws it into `temp` (in the data folder) and the copy goes into the
    /// chosen file. `job` is sent once the panel has answered with `file`.
    Worker { job: Option<Job>, temp: PathBuf, file: Option<std::fs::File> },
}

/// A PDF on its way from its utility queue: submitted jobs and the inbox its replies come back in.
struct Loading {
    path: PathBuf,
    /// The password the file is opened with; empty when none was asked for.
    password: Vec<u8>,
    jobs: Jobs,
    inbox: Inbox,
}

struct Viewer {
    data_dir: PathBuf,
    doc: Option<Document>,
    loading: Option<Loading>,
    view: View,
    tool: Tool,
    side: Side,
    adjust: Adjust,
    /// The size exports are written at, as a factor of the page.
    export_scale: f32,
    gesture: Option<Gesture>,
    /// The index of the note whose text the keyboard is typing.
    editing: Option<usize>,
    /// The last message: an open error, a save or an export result.
    status: String,
    cache: Option<PictureCache>,
    /// Undo and redo of the document's edits.
    history: History,
    /// The pages visited, for Go > Back and Forward.
    trail: Trail,
    /// The page number being typed after Go > Go to Page.
    goto: Option<String>,
    /// Whether the thumbnails and outline sidebar is shown.
    sidebar: bool,
    /// The files opened lately, for File > Open Recent.
    recents: Recents,
    /// One page, the contact sheet, or the slideshow.
    mode: Mode,
    /// The zoom to go back to when the slideshow ends.
    slideshow_zoom: Zoom,
    contact_scroll: f32,
    /// The selected rectangle and the source page it is on.
    selection: Option<(usize, Norm)>,
    /// Whether the magnifier is on, and where the pointer last was over the canvas.
    loupe_on: bool,
    loupe: Option<(f32, f32)>,
    /// Whether the Inspector is shown, and what is known about the open file.
    inspector: bool,
    facts: inspector::Facts,
    /// The search text being typed after Edit > Find, the text last searched for and its hits.
    finding: Option<String>,
    query: String,
    found: Vec<find::Hit>,
    /// The size being typed after Tools > Adjust Size.
    resizing: Option<String>,
    /// The quality of an exported JPEG, 1 to 100.
    jpeg_quality: u8,
    /// Whether the toolbar is shown (View > Hide Toolbar).
    toolbar: bool,
    /// The Settings menu's choices, and the signature library.
    prefs: Prefs,
    signatures: Vec<Signature>,
    /// The file a save is written for, until the worker has written the copy that replaces it.
    pending_save: Option<PathBuf>,
    /// The export waiting for the save panel's file (Export As, File > Export), until it is written or cancelled.
    pending_export: Option<PendingExport>,
    /// The source page to show once the document is open, from the last time it was shown.
    restore_page: Option<usize>,
    /// The author's name being typed after Settings > Annotation Author.
    authoring: Option<String>,
    /// The status line to show once the file that is being opened again has arrived.
    after_open: Option<String>,
    /// Whether the Adjust Color sliders are shown on the right side.
    color_panel: bool,
    /// Whether the Export As panel is shown on the right, and the format it exports to.
    export_shown: bool,
    /// Whether the signature picker is shown on the right.
    signatures_shown: bool,
    /// The screenshot request the shell has not answered yet.
    shot_request: Option<u32>,
    export_format: export_panel::Format,
    /// A PDF that needs a password, and the password being typed for it.
    asking: Option<Ask>,
    /// The password a PDF was opened with this session, so it is not asked for again (Revert to Saved, Open Recent).
    password: Option<(PathBuf, Vec<u8>)>,
    /// The display place a PDF goes at, while the open panel chooses the file to insert there.
    insert_at: Option<usize>,
    /// The file an insert writes, opened when the worker has written it.
    inserted: Option<PathBuf>,
    /// The form widget that the keyboard is typing into.
    form_focus: Option<usize>,
}

impl Viewer {
    fn new() -> Viewer {
        Viewer {
            data_dir: PathBuf::new(),
            doc: None,
            loading: None,
            view: View::default(),
            tool: Tool::Pan,
            side: Side::Pages,
            adjust: Adjust::default(),
            export_scale: 1.0,
            gesture: None,
            editing: None,
            status: String::new(),
            cache: None,
            history: History::default(),
            trail: Trail::default(),
            goto: None,
            sidebar: true,
            recents: Recents::default(),
            mode: Mode::Page,
            slideshow_zoom: Zoom::Fit,
            contact_scroll: 0.0,
            selection: None,
            loupe_on: false,
            loupe: None,
            inspector: false,
            facts: inspector::Facts::default(),
            finding: None,
            query: String::new(),
            found: Vec::new(),
            resizing: None,
            jpeg_quality: export::JPEG_QUALITY,
            toolbar: true,
            prefs: Prefs::default(),
            signatures: Vec::new(),
            pending_save: None,
            pending_export: None,
            restore_page: None,
            authoring: None,
            after_open: None,
            color_panel: false,
            export_shown: false,
            signatures_shown: false,
            shot_request: None,
            export_format: export_panel::Format::Png,
            asking: None,
            password: None,
            insert_at: None,
            inserted: None,
            form_focus: None,
        }
    }

    /// The window layout: the sidebar takes no room while it is hidden, the Inspector takes room while it is shown, and
    /// the slideshow shows the page and nothing else.
    fn layout(&self, ui: &Ui, w: i32, h: i32) -> Layout {
        if self.mode == Mode::Slideshow {
            return Layout::bare(w, h);
        }
        let mut lay = Layout::new(ui, w, h);
        if !self.sidebar {
            lay.side = 0;
        }
        if !self.toolbar {
            lay.bar = 0;
        }
        if self.inspector || self.color_panel || self.export_shown || self.signatures_shown {
            lay.inspector = Layout::inspector_width(ui, w);
        }
        lay
    }

    /// The document's editable state, or None with no document.
    fn snapshot(&self) -> Option<Snapshot> {
        let doc = self.doc.as_ref()?;
        Some(Snapshot {
            marks: doc.marks.clone(),
            order: doc.order.clone(),
            turns: doc.turns.clone(),
            adjust: self.adjust,
            pixels: doc.picture_rc().map(Pixels),
        })
    }

    /// Puts a state from the undo history back.
    fn restore(&mut self, state: Snapshot) {
        self.editing = None;
        if let Some(doc) = &mut self.doc {
            doc.marks = state.marks;
            doc.order = state.order;
            doc.turns = state.turns;
            if let Some(Pixels(picture)) = state.pixels {
                doc.replace_picture(picture);
            }
        }
        self.selection = None;
        self.adjust = state.adjust;
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let keep = self.view.page;
        self.view.go(keep, count);
        self.sync_turn();
        self.save();
    }

    /// Runs an edit and, when it changed the document, records the state before it for Undo.
    fn track<R>(&mut self, edit: impl FnOnce(&mut Viewer) -> R) -> R {
        let before = self.snapshot();
        let out = edit(self);
        if let Some(before) = before {
            if self.snapshot().as_ref() != Some(&before) {
                self.history.record(before);
            }
        }
        out
    }

    fn open_path(&mut self, path: &Path) {
        let password = match &self.password {
            Some((known, password)) if known == path => password.clone(),
            _ => Vec::new(),
        };
        self.open_document(path, password);
    }

    /// Opens `path` with `password` (empty for a file without one). A PDF that the password does not open asks for it.
    fn open_document(&mut self, path: &Path, password: Vec<u8>) {
        self.stop_editing();
        self.save_position();
        self.asking = None;
        self.authoring = None;
        self.pending_save = None;
        // an export's job belongs to the document it was asked for; the panel's answer then finds nothing to write
        self.pending_export = None;
        self.after_open = None;
        self.doc = None;
        self.loading = None;
        self.cache = None;
        self.view = View::default();
        self.side = Side::Pages;
        self.history = History::default();
        self.trail = Trail::default();
        self.goto = None;
        self.finding = None;
        self.resizing = None;
        self.found.clear();
        self.query.clear();
        self.mode = Mode::Page;
        self.contact_scroll = 0.0;
        self.selection = None;
        self.loupe = None;
        self.facts = inspector::read(path);
        self.status.clear();
        self.remember(path);
        self.restore_page = if self.prefs.remember_page { document::load_position(&self.data_dir, path) } else { None };
        if matches!(engine::format_of(path), Some(engine::Format::Pdf)) {
            let (jobs, inbox) = pdfjob::spawn(path.to_path_buf(), password.clone());
            self.loading = Some(Loading { path: path.to_path_buf(), password, jobs, inbox });
            self.status = format!("Opening {}", name_of(path));
            return;
        }
        match document::open_picture(path, &self.data_dir) {
            Ok(doc) => self.doc = Some(doc),
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    /// Puts `path` first in Open Recent and saves the list.
    fn remember(&mut self, path: &Path) {
        if self.data_dir.as_os_str().is_empty() || !path.exists() {
            return;
        }
        self.recents.add(path);
        let _ = recents::save(&self.data_dir, &self.recents);
    }

    /// Writes the page the document is shown on, so it opens there again while Remember Last Page is on.
    fn save_position(&self) {
        if !self.prefs.remember_page || self.data_dir.as_os_str().is_empty() {
            return;
        }
        if let (Some(doc), Some(src)) = (&self.doc, self.src()) {
            let _ = document::save_position(&self.data_dir, &doc.path, src);
        }
    }

    /// Saves the Settings menu's choices under the app's data folder.
    fn save_prefs(&self) {
        if !self.data_dir.as_os_str().is_empty() {
            let _ = prefs::save(&self.data_dir, &self.prefs);
        }
    }

    /// The author written into new markup: the name from Settings, or none.
    fn author(&self) -> String {
        self.prefs.author_name().unwrap_or("").to_string()
    }

    /// The file a save is written to before it replaces the document's file. It lives in the app's data folder.
    fn saving_file(&self, path: &Path, ext: &str) -> PathBuf {
        let dir = self.data_dir.join("save");
        let _ = std::fs::create_dir_all(&dir);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
        dir.join(format!("{stem}.{ext}"))
    }

    /// File > Save: the markup, page order and turns are written into the document's file, and the file is opened again.
    /// A picture is written as a picture in the format of its file. A PDF is written by its worker and then copied over.
    /// The copy is the one the window opened, for a file chosen in the open panel.
    fn save_document(&mut self) {
        self.stop_editing();
        let Some(doc) = &self.doc else {
            self.status = "There is nothing to save.".into();
            return;
        };
        let path = doc.path.clone();
        if doc.is_pdf {
            let temp = self.saving_file(&path, "pdf");
            let job = pdfmarks::save_job(path.clone(), temp, doc.order.clone(), doc.turns.clone(), doc.marks.clone(), self.author(), pdfmarks::Insertion::None, doc.field_edits());
            if doc.send(job) {
                self.pending_save = Some(path);
                self.status = "Saving".into();
            } else {
                self.status = "The document is not open.".into();
            }
            return;
        }
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let Some(img) = doc.picture(page) else { return };
        let raster = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") => Raster::Jpeg(self.jpeg_quality),
            _ => Raster::Png,
        };
        let out = export::finish(&img, &doc.marks, src, doc.turn_of(page), &self.adjust, 1.0);
        let temp = self.saving_file(&path, raster.ext());
        let written = export::write_image(&out, raster, &temp).and_then(|()| std::fs::copy(&temp, &path).map(|_| ()).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                let message = format!("Saved {}", name_of(&path));
                self.reopen(&path, message);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// The PDF worker wrote the saved copy: it replaces the file the document was opened from.
    fn finish_save(&mut self, original: PathBuf, result: Result<PathBuf, String>) {
        let copied = result.and_then(|temp| std::fs::copy(&temp, &original).map(|_| ()).map_err(|e| e.to_string()));
        match copied {
            Ok(()) => {
                let message = format!("Saved {}", name_of(&original));
                self.reopen(&original, message);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Opens `path` again. The markup, page order and turns are in the file now, so the document's markup is dropped
    /// (the sidecar is emptied first) and the message is shown once the file is open.
    fn reopen(&mut self, path: &Path, message: String) {
        if let Some(doc) = &mut self.doc {
            doc.marks.clear();
        }
        self.save();
        self.open_path(path);
        self.status = message.clone();
        self.after_open = Some(message);
    }

    /// File > Revert to Saved: the file is opened again from disk, so every change since it was last saved goes.
    fn revert_to_saved(&mut self) {
        let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) else {
            self.status = "There is nothing to revert.".into();
            return;
        };
        let message = format!("Reverted to the saved {}", name_of(&path));
        self.reopen(&path, message);
    }

    /// Edit > Copy: the text of a PDF page (only the lines in the selection when there is one) or a picture as PNG. The
    /// PDF worker reads the text and the text reaches the clipboard in `finish_copy`.
    fn copy(&mut self, ctx: &Ctx) {
        let Some(doc) = &self.doc else { return };
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        if doc.is_pdf {
            let band = self.selection_on_page().map(|sel| (sel.y, sel.y + sel.h));
            let sent = doc.send(find::copy_job(src, band));
            self.status = if sent { "Copying the text".into() } else { "The document is not open.".into() };
            return;
        }
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, doc.turn_of(page), &self.adjust, 1.0);
        match export::png_bytes(&out) {
            Some(bytes) => {
                NativePasteboard::general().set_data(ctx, &bytes, PNG);
                self.status = "Copied the picture".into();
            }
            None => self.status = "The picture could not be copied.".into(),
        }
    }

    /// Edit > Speech > Start Speaking: the selected PDF text, else the shown page's text. The PDF worker reads both and
    /// `finish_speak` starts the speech when they arrive. A picture has no text to speak.
    fn speak(&mut self) -> bool {
        let Some(doc) = &self.doc else { return false };
        let Some(src) = doc.source_of(self.view.page) else { return false };
        if !doc.is_pdf {
            self.status = "There is no text to speak here.".into();
            return true;
        }
        let band = self.selection_on_page().map(|sel| (sel.y, sel.y + sel.h));
        let sent = doc.send(find::speech_job(src, band));
        self.status = if sent { "Reading the text".into() } else { "The document is not open.".into() };
        true
    }

    /// Speaks the text the worker read for `speak`: the selection when it has text, else the page.
    fn finish_speak(&mut self, ctx: &Ctx, selected: &str, page: &str) {
        match find::speech_text(selected, page) {
            Some((text, what)) => {
                speech::speak(ctx, &speech::Utterance::new(text));
                self.status = format!("Speaking {what}");
            }
            None => self.status = "There is no text to speak here.".into(),
        }
    }

    /// File > Take Screenshot: asks the shell's screenshot service. The shell answers the request, then sends the
    /// picture's path as a `captured` event, and `service_event` opens it.
    fn take_shot(&mut self, ctx: &Ctx, pick: screenshot::Pick) -> bool {
        self.shot_request = Some(screenshot::take(ctx, pick));
        self.status = match pick {
            screenshot::Pick::Screen => "Taking a screenshot of the screen".into(),
            _ => "Choose what to capture".into(),
        };
        true
    }

    /// Puts the text the worker copied on the clipboard.
    fn finish_copy(&mut self, ctx: &Ctx, text: String) {
        if text.trim().is_empty() {
            self.status = "There is no text to copy here.".into();
            return;
        }
        ctx.set_pasteboard_string(&text);
        let lines = text.lines().count();
        self.status = format!("Copied {lines} {}", if lines == 1 { "line" } else { "lines" });
    }

    /// Edit > Select All: the whole shown page is selected.
    fn select_all(&mut self) {
        self.selection = self.src().map(|src| (src, Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }));
    }

    /// Tools > Save Signature: the drawings on the shown page become a signature in the library.
    fn save_signature(&mut self) {
        let Some(src) = self.src() else { return };
        let strokes: Vec<Vec<(f32, f32)>> = self
            .doc
            .as_ref()
            .map(|d| d.marks.iter().filter(|m| m.page == src && m.kind == MarkKind::Ink).map(|m| m.ink.clone()).collect())
            .unwrap_or_default();
        match signature::from_strokes(&strokes) {
            Some(sig) => {
                signature::add(&mut self.signatures, sig);
                self.status = match signature::save(&self.data_dir, &self.signatures) {
                    Ok(()) => format!("Saved signature {}", self.signatures.len()),
                    Err(e) => format!("Could not save the signature: {e}"),
                };
            }
            None => self.status = "Draw a signature with the Sign tool on this page first.".into(),
        }
    }

    /// Tools > Clear Signatures: the library is emptied.
    fn clear_signatures(&mut self) {
        self.signatures.clear();
        self.status = match signature::save(&self.data_dir, &self.signatures) {
            Ok(()) => "Cleared the saved signatures.".into(),
            Err(e) => format!("Could not clear the signatures: {e}"),
        };
    }

    /// Tools > Insert Signature: the saved signature `index` goes in the middle of the shown page, as drawn strokes.
    fn insert_signature(&mut self, index: usize) {
        let Some(sig) = self.signatures.get(index).cloned() else { return };
        let Some(src) = self.src() else {
            self.status = "Open a page to insert a signature on.".into();
            return;
        };
        if let Some(doc) = &mut self.doc {
            for points in signature::centred(&sig, 0.5, 0.5, SIGNATURE_WIDTH) {
                doc.marks.push(stroke(src, points));
            }
        }
        self.save();
        self.status = format!("Inserted signature {}", index + 1);
    }

    /// Settings > Annotation Author: the status line takes the name; Return keeps it and Escape leaves it as it was.
    fn show_authoring(&mut self) {
        self.status = format!("Annotation author: {}|", self.authoring.as_deref().unwrap_or(""));
    }

    /// A key while the author's name is being typed.
    fn key_author(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                if let Some(name) = &mut self.authoring {
                    name.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            KeyInput::Code(8) => {
                if let Some(name) = &mut self.authoring {
                    name.pop();
                }
            }
            KeyInput::Code(13) => {
                let name = self.authoring.take().unwrap_or_default();
                self.prefs.author = prefs::sanitize(&name);
                self.save_prefs();
                self.status = if self.prefs.author.is_empty() { "New markup has no author name.".into() } else { format!("Annotation author: {}", self.prefs.author) };
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.authoring = None;
                self.status.clear();
                return true;
            }
            _ => return false,
        }
        self.show_authoring();
        true
    }

    /// Shows the page a PDF was last shown on (Remember Last Page), or the contact sheet when that is the default. Called
    /// once a PDF has opened.
    fn restore_view(&mut self) {
        let restored = self.restore_page.take().and_then(|src| self.doc.as_ref()?.display_of(src));
        if let Some(page) = restored {
            let count = self.doc.as_ref().map_or(0, |d| d.count());
            self.view.go(page, count);
        }
        self.sync_turn();
        if self.prefs.contact_sheet {
            self.mode = Mode::Contact;
            self.contact_scroll = 0.0;
        }
    }

    /// Shows the open panel for PDFs and pictures.
    fn show_open_panel(&self, ctx: &Ctx) {
        let options = OpenOptions {
            mode: DocMode::Read,
            title: "Open".into(),
            filters: OPEN_TYPES.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        ctx.open_panel(&options);
    }

    /// File > Open, Open Recent and Clear Menu. False when `id` is not one of those.
    fn open_menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id != MENU_INSERT_FILE {
            self.insert_at = None;
        }
        match id {
            MENU_INSERT_FILE => {
                self.insert_at = Some(self.view.page + 1);
                self.show_insert_panel(ctx);
            }
            MENU_INSERT_BLANK => self.insert_blank(),
            MENU_OPEN => self.show_open_panel(ctx),
            MENU_CLEAR_RECENTS => {
                self.recents.clear();
                let _ = recents::save(&self.data_dir, &self.recents);
            }
            _ if (MENU_RECENT..MENU_RECENT + recents::MAX_RECENTS as u32).contains(&id) => {
                let Some(path) = self.recents.get((id - MENU_RECENT) as usize).map(Path::to_path_buf) else { return true };
                if path.exists() {
                    self.open_path(&path);
                } else {
                    self.recents.prune();
                    self.status = format!("{} is no longer there.", name_of(&path));
                }
            }
            _ => return false,
        }
        true
    }

    /// The file an insert writes: a copy of the document under the app's data folder, which is opened when it is done.
    fn inserting_file(&self, path: &Path) -> PathBuf {
        let dir = self.data_dir.join("insert");
        let _ = std::fs::create_dir_all(&dir);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
        dir.join(format!("{stem}.pdf"))
    }

    /// Writes the document with `insertion` added to its pages, to the insert folder. The worker writes it, and the
    /// new file is opened once it is written.
    fn insert_pages(&mut self, insertion: pdfmarks::Insertion) {
        self.stop_editing();
        let Some(doc) = &self.doc else { return };
        if !doc.is_pdf {
            self.status = "Pages can only be inserted into a PDF.".into();
            return;
        }
        let target = self.inserting_file(&doc.path);
        let job = pdfmarks::save_job(
            doc.path.clone(),
            target.clone(),
            doc.order.clone(),
            doc.turns.clone(),
            doc.marks.clone(),
            self.author(),
            insertion,
            doc.field_edits(),
        );
        if doc.send(job) {
            self.inserted = Some(target);
            self.status = "Inserting".into();
        } else {
            self.status = "The document is not open.".into();
        }
    }

    /// Pages > Insert > Blank Page: a blank page the size of the shown page, after it.
    fn insert_blank(&mut self) {
        let Some(src) = self.src() else { return };
        let at = self.view.page + 1;
        self.insert_pages(pdfmarks::Insertion::Blank { at, src });
    }

    /// The worker wrote the document with pages inserted: the new file is opened in place of the old one.
    fn finish_insert(&mut self, result: Result<PathBuf, String>) {
        self.inserted = None;
        match result {
            Ok(path) => {
                self.after_open = Some(format!("Inserted pages into {}", name_of(&path)));
                self.open_path(&path);
            }
            Err(e) => self.status = format!("Insert failed: {e}"),
        }
    }

    /// Pages > Insert > Page from File: the open panel, offering PDFs.
    fn show_insert_panel(&self, ctx: &Ctx) {
        let options = OpenOptions { mode: DocMode::Read, title: "Insert".into(), filters: vec!["pdf".into()], ..Default::default() };
        ctx.open_panel(&options);
    }

    /// The form widget under the canvas point (x, y) of the shown page, if there is one.
    fn form_widget_at(&self, lay: &Layout, x: f32, y: f32) -> Option<usize> {
        let doc = self.doc.as_ref()?;
        let p = self.placed(lay)?;
        let src = self.src()?;
        let inside = x >= p.x as f32 && y >= p.y as f32 && x <= (p.x + p.w as i32) as f32 && y <= (p.y + p.h as i32) as f32;
        if !inside {
            return None;
        }
        doc.widget_at(src, canvas::to_source(&p, self.view.quarters, x, y))
    }

    /// A click on a form widget: a check box toggles, and a text field takes the keyboard.
    fn click_widget(&mut self, widget: usize) {
        let Some(doc) = &mut self.doc else { return };
        let kind = match doc.widgets.get(widget) {
            Some(w) => w.kind,
            None => return,
        };
        match kind {
            forms::Kind::Check => {
                doc.toggle_field(widget);
                self.form_focus = None;
            }
            forms::Kind::Text => self.form_focus = Some(widget),
        }
    }

    /// The keyboard while a form text field has focus: typing adds to its text, Backspace removes, and Return or Escape
    /// leaves the field. False for keys the field does not take.
    fn key_form(&mut self, widget: usize, key: &KeyInput) -> bool {
        let Some(doc) = &mut self.doc else { return false };
        match key {
            KeyInput::Text(text) => {
                let next = format!("{}{}", doc.field_text(widget).unwrap_or(""), text);
                doc.set_field_text(widget, &next);
            }
            KeyInput::Code(8) => {
                let mut next = doc.field_text(widget).unwrap_or("").to_string();
                next.pop();
                doc.set_field_text(widget, &next);
            }
            KeyInput::Code(13) | KeyInput::Code(1 | 27) => self.form_focus = None,
            _ => return false,
        }
        true
    }

    /// Takes what the PDF worker has sent: the open result, drawn pages and the results of saves and exports.
    /// True when the window needs a new frame.
    fn pump(&mut self, ctx: &Ctx) -> bool {
        let mut changed = false;
        if let Some(loading) = self.loading.take() {
            let mut opened = None;
            for reply in pdfjob::take(&loading.inbox) {
                if let Reply::Opened(result) = reply {
                    opened = Some(result);
                }
            }
            match opened {
                None => self.loading = Some(loading),
                Some(Ok(info)) => {
                    changed = true;
                    let Loading { path, jobs, inbox, .. } = loading;
                    self.doc = Some(document::from_pdf(&path, &self.data_dir, info, jobs, inbox));
                    self.status = self.after_open.take().unwrap_or_default();
                    self.restore_view();
                }
                Some(Err(e)) if pdfjob::needs_password(&e) => {
                    changed = true;
                    let retry = !loading.password.is_empty();
                    self.password = None;
                    self.asking = Some(Ask::new(&loading.path, retry));
                    self.show_asking();
                }
                Some(Err(e)) => {
                    changed = true;
                    self.status = format!("Could not open {}: {e}", loading.path.display());
                }
            }
        }
        // an export asks for its file once: the save panel answers with `DocumentEvent::SaveTo`
        if let Some(pending) = self.pending_export.as_mut() {
            if pending.request.is_none() {
                let request = ctx.save_document(&pending.name, &[pending.ext]);
                pending.request = Some(request);
            }
        }
        let mut results = None;
        let mut saved = None;
        let mut exported = None;
        let mut inserted = None;
        let mut copied = None;
        let mut spoken = None;
        if let Some(doc) = &self.doc {
            let (arrived, replies) = doc.take_replies();
            changed |= arrived;
            for reply in replies {
                match reply {
                    // the copy File > Save made is the one that replaces the document's file
                    Reply::Written(result) if self.inserted.is_some() => inserted = Some(result),
                    Reply::Written(result) if self.pending_save.is_some() => saved = Some(result),
                    Reply::Written(result) if self.export_is_drawn() => exported = Some(result),
                    Reply::Written(result) => {
                        self.status = match result {
                            Ok(path) if path.extension().is_some_and(|e| e == "pdf") => format!("Saved {}", path.display()),
                            Ok(path) => format!("Exported {}", path.display()),
                            Err(e) => format!("Save failed: {e}"),
                        };
                    }
                    Reply::Printed(status) => self.status = status,
                    Reply::Found(query, hits) => results = Some((query, hits)),
                    Reply::Copied(text) => copied = Some(text),
                    Reply::Spoken(selected, page) => spoken = Some((selected, page)),
                    _ => {}
                }
            }
        }
        if let Some(result) = inserted {
            self.finish_insert(result);
        }
        if let (Some(result), Some(original)) = (saved, self.pending_save.take()) {
            self.finish_save(original, result);
        }
        if let Some(result) = exported {
            changed = true;
            self.finish_drawn_export(ctx, result);
        }
        if let Some(text) = copied {
            self.finish_copy(ctx, text);
        }
        if let Some((selected, page)) = spoken {
            self.finish_speak(ctx, &selected, &page);
        }
        if let Some((query, hits)) = results {
            self.show_results(query, hits);
        }
        changed
    }

    /// Takes the hits of a search: lists them in the sidebar and goes to the first one on or after the shown page.
    fn show_results(&mut self, query: String, hits: Vec<find::Hit>) {
        self.status = find::summary(&query, hits.len());
        self.found = hits;
        if self.found.is_empty() {
            return;
        }
        self.side = Side::Search;
        self.sidebar = true;
        self.view.sidebar_scroll = 0.0;
        if let Some(page) = find::first_from(&self.hit_places(), self.view.page) {
            self.go_page(page);
        }
    }

    /// The shown pages that hold a hit, in hit order. A hit on a deleted page has none.
    fn hit_places(&self) -> Vec<usize> {
        match &self.doc {
            Some(doc) => self.found.iter().filter_map(|h| doc.display_of(h.page)).collect(),
            None => Vec::new(),
        }
    }

    fn save(&self) {
        if let Some(doc) = &self.doc {
            let _ = document::save_marks(&self.data_dir, &doc.path, &doc.marks);
            let _ = document::save_bookmarks(&self.data_dir, &doc.path, &doc.bookmarks);
        }
    }

    /// The source page the window is showing, or None with no document.
    fn src(&self) -> Option<usize> {
        self.doc.as_ref()?.source_of(self.view.page)
    }

    /// Makes the view's turn match the shown page's saved turn, after the page changes.
    fn sync_turn(&mut self) {
        if let Some(doc) = &self.doc {
            self.view.quarters = doc.turn_of(self.view.page);
        }
    }

    /// The picture geometry for the current page and view, or `None` with no document.
    fn placed(&self, lay: &Layout) -> Option<canvas::Placed> {
        let doc = self.doc.as_ref()?;
        let (w, h) = doc.page_size(self.view.page)?;
        let (cx, cy, cw, ch) = lay.canvas();
        let (rw, rh) = model::turn_size(w, h, self.view.quarters);
        let scale = self.view.zoom.scale(rw, rh, cw as usize, ch as usize);
        Some(canvas::place((cx, cy, cw, ch), rw, rh, scale, self.view.pan))
    }

    /// The mark a drag in progress would make.
    fn draft(&self, lay: &Layout) -> Option<Mark> {
        let p = self.placed(lay)?;
        let src = self.src()?;
        let q = self.view.quarters;
        match self.gesture.as_ref()? {
            Gesture::Drag { start, now, kind } => {
                let a = canvas::to_source(&p, q, start.0, start.1);
                let b = canvas::to_source(&p, q, now.0, now.1);
                Some(model::drag_mark(*kind, src, a, b, color_for(*kind)))
            }
            Gesture::Ink { points } => Some(stroke(src, points.iter().map(|&(x, y)| canvas::to_source(&p, q, x, y)).collect())),
            _ => None,
        }
    }

    /// The selection on the shown page, if there is one.
    fn selection_on_page(&self) -> Option<Norm> {
        let (page, rect) = self.selection?;
        (self.src()? == page).then_some(rect)
    }

    /// The selection to draw: the one being dragged out, else the saved one.
    fn selection_now(&self, lay: &Layout) -> Option<Norm> {
        if let Some(Gesture::Select { start, now }) = &self.gesture {
            let p = self.placed(lay)?;
            let q = self.view.quarters;
            let (a, b) = (canvas::to_source(&p, q, start.0, start.1), canvas::to_source(&p, q, now.0, now.1));
            return Some(Norm::from_corners(a, b));
        }
        self.selection_on_page()
    }

    /// What the Export As panel shows: the chosen format, quality and size, and whether a document is open.
    fn export_choices(&self) -> export_panel::Choices {
        export_panel::Choices { format: self.export_format, quality: self.jpeg_quality, scale: self.export_scale, open: self.doc.is_some() }
    }

    /// The rows the Inspector lists for the open file.
    fn inspector_rows(&self) -> Vec<(String, String)> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let shown = inspector::Shown {
            pages: doc.count(),
            is_pdf: doc.is_pdf,
            size: doc.sizes.first().copied(),
            marks: &doc.marks,
            bookmarks: doc.bookmarks.len(),
            info: &doc.info,
            encrypted: doc.encrypted,
        };
        inspector::rows(&self.facts, &shown)
    }

    /// The size the zoom steps work from: the turned picture, or the canvas with no document.
    fn page_dims(&self, lay: &Layout) -> (usize, usize) {
        let (_, _, cw, ch) = lay.canvas();
        let picture = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)).map(|(w, h)| model::turn_size(w, h, self.view.quarters));
        picture.unwrap_or((cw as usize, ch as usize))
    }

    /// The thumbnail rows or outline rows of the sidebar, whichever it shows.
    fn side_rows(&self, ui: &Ui) -> Vec<(i32, i32)> {
        match (&self.doc, self.side) {
            (Some(doc), Side::Pages) => canvas::rows(ui, doc),
            (Some(doc), Side::Outline) => canvas::outline_rows(ui, doc),
            (Some(doc), side) => canvas::list_rows(ui, canvas::side_items(doc, side, &self.found).len()),
            (None, _) => Vec::new(),
        }
    }

    fn scroll_max(&self, ui: &Ui, lay: &Layout) -> f32 {
        (canvas::rows_height(&self.side_rows(ui)) - (lay.h - lay.bar)).max(0) as f32
    }

    /// Shows `page` without noting the move in the trail.
    fn jump(&mut self, page: usize) {
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        if self.view.go(page, count) {
            self.stop_editing();
        }
        self.sync_turn();
    }

    /// Shows `page` and notes where the view came from, for Go > Back.
    fn go_page(&mut self, page: usize) {
        let from = self.view.page;
        self.jump(page);
        if self.view.page != from {
            self.trail.visit(from);
        }
    }

    /// Shows what has been typed after Go to Page in the status line.
    fn show_goto(&mut self) {
        self.status = format!("Go to page: {}|", self.goto.as_deref().unwrap_or(""));
    }

    /// A key while a page number is being typed.
    fn key_goto(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Some(g) = &mut self.goto {
                    g.push_str(&digits);
                }
            }
            KeyInput::Code(8) => {
                if let Some(g) = &mut self.goto {
                    g.pop();
                }
            }
            KeyInput::Code(13) => {
                let text = self.goto.take().unwrap_or_default();
                let count = self.doc.as_ref().map_or(0, |d| d.count());
                self.status.clear();
                match model::parse_page(&text, count) {
                    Some(page) => self.go_page(page),
                    None if !text.is_empty() => self.status = format!("There is no page {text}."),
                    None => {}
                }
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.goto = None;
                self.status.clear();
                return true;
            }
            _ => return false,
        }
        self.show_goto();
        true
    }

    /// Toggles the bookmark on the shown page.
    fn toggle_bookmark(&mut self) {
        let Some(src) = self.src() else { return };
        let shown = self.view.page + 1;
        if let Some(doc) = &mut self.doc {
            let on = model::toggle_bookmark(&mut doc.bookmarks, src);
            self.status = if on { format!("Bookmarked page {shown}") } else { format!("Removed the bookmark on page {shown}") };
        }
        self.save();
    }

    /// Goes to the next or previous bookmarked page.
    fn go_bookmark(&mut self, forward: bool) {
        let places = self.doc.as_ref().map(|d| d.bookmark_places()).unwrap_or_default();
        match model::neighbour(&places, self.view.page, forward) {
            Some(page) => self.go_page(page),
            None => self.status = "There is no bookmark that way.".into(),
        }
    }

    /// Ends the note being typed. A note left empty is removed.
    fn stop_editing(&mut self) {
        let Some(i) = self.editing.take() else { return };
        if let Some(doc) = &mut self.doc {
            if doc.marks.get(i).is_some_and(|m| m.text.is_empty()) {
                doc.marks.remove(i);
            }
        }
        self.save();
    }

    /// Turns the shown page by `delta` quarter turns, and keeps the turn with the page.
    fn rotate(&mut self, delta: i32) {
        if self.doc.is_none() {
            return;
        }
        self.view.turn(delta);
        let (page, quarters) = (self.view.page, self.view.quarters);
        if let Some(doc) = &mut self.doc {
            doc.set_turn(page, quarters);
        }
    }

    /// Does what a menu item or toolbar button says. Edits are recorded for Undo; Undo, Redo and Go are not edits.
    fn act(&mut self, ui: &Ui, lay: &Layout, action: Action) -> bool {
        match action {
            Action::Undo | Action::Redo => {
                self.stop_editing();
                let Some(current) = self.snapshot() else { return false };
                let state = if action == Action::Undo { self.history.undo(current) } else { self.history.redo(current) };
                if let Some(state) = state {
                    self.restore(state);
                }
                true
            }
            Action::GoBack | Action::GoForward => {
                let current = self.view.page;
                let page = if action == Action::GoBack { self.trail.back(current) } else { self.trail.forward(current) };
                if let Some(page) = page {
                    self.jump(page);
                }
                true
            }
            Action::FirstPage => {
                self.go_page(0);
                true
            }
            Action::LastPage => {
                let count = self.doc.as_ref().map_or(0, |d| d.count());
                self.go_page(count.saturating_sub(1));
                true
            }
            Action::GoToPage => {
                if self.doc.is_some() {
                    self.stop_editing();
                    self.goto = Some(String::new());
                    self.show_goto();
                }
                true
            }
            Action::AddBookmark => {
                self.toggle_bookmark();
                true
            }
            Action::NextBookmark | Action::PreviousBookmark => {
                self.go_bookmark(action == Action::NextBookmark);
                true
            }
            Action::ToggleSidebar => {
                self.sidebar = !self.sidebar;
                true
            }
            Action::ToggleMagnifier => {
                self.loupe_on = !self.loupe_on;
                if !self.loupe_on {
                    self.loupe = None;
                }
                true
            }
            Action::Inspector => {
                self.inspector = !self.inspector;
                true
            }
            Action::ShowLocation => {
                self.show_location();
                true
            }
            Action::ContactSheet => {
                self.toggle_contact(ui, lay);
                true
            }
            Action::Slideshow => {
                self.toggle_slideshow();
                true
            }
            Action::ZoomToSelection => {
                self.zoom_to_selection(lay);
                true
            }
            Action::NewFromClipboard => {
                self.new_from_clipboard();
                true
            }
            Action::Find => {
                self.start_find();
                true
            }
            Action::AdjustSize => {
                self.start_resize();
                true
            }
            Action::FindNext | Action::FindPrevious => {
                self.find_step(action == Action::FindNext);
                true
            }
            Action::Save => {
                self.save_document();
                true
            }
            Action::RevertToSaved => {
                self.revert_to_saved();
                true
            }
            // Copy needs the window's context to reach the clipboard, so menu_action runs it
            Action::CopyText => true,
            Action::SelectAll => {
                self.select_all();
                true
            }
            Action::SaveSignature => {
                self.save_signature();
                true
            }
            Action::ClearSignatures => {
                self.clear_signatures();
                true
            }
            Action::ToggleToolbar => {
                self.toolbar = !self.toolbar;
                true
            }
            Action::ToggleRememberPage => {
                self.prefs.remember_page = !self.prefs.remember_page;
                self.save_prefs();
                true
            }
            Action::ToggleContactDefault => {
                self.prefs.contact_sheet = !self.prefs.contact_sheet;
                self.save_prefs();
                true
            }
            Action::SetAuthor => {
                self.authoring = Some(self.prefs.author.clone());
                self.show_authoring();
                true
            }
            Action::ShowColorPanel => {
                self.color_panel = !self.color_panel;
                if self.color_panel {
                    self.export_shown = false;
                    self.signatures_shown = false;
                }
                true
            }
            Action::ExportAs => {
                self.export_shown = !self.export_shown;
                if self.export_shown {
                    self.color_panel = false;
                    self.signatures_shown = false;
                }
                true
            }
            Action::ShowSignatures => {
                self.signatures_shown = !self.signatures_shown;
                if self.signatures_shown {
                    self.color_panel = false;
                    self.export_shown = false;
                }
                true
            }
            Action::ChooseFormat(format) => {
                self.export_format = format;
                true
            }
            _ => self.track(|v| v.act_edit(ui, lay, action)),
        }
    }

    /// Tools > Adjust Size: the status line takes the new size. Only a picture can be resampled.
    fn start_resize(&mut self) {
        if !self.doc.as_ref().is_some_and(|d| !d.is_pdf) {
            self.status = "Adjust Size works on pictures, not PDFs.".into();
            return;
        }
        self.stop_editing();
        self.resizing = Some(String::new());
        self.show_resizing();
    }

    /// Shows the current size and what has been typed after Adjust Size in the status line.
    fn show_resizing(&mut self) {
        let (w, h) = self.doc.as_ref().and_then(|d| d.sizes.first().copied()).unwrap_or((0, 0));
        self.status = format!("Adjust Size ({w} x {h}): {}|  width, width x height or percent", self.resizing.as_deref().unwrap_or(""));
    }

    /// A key while the new size is being typed.
    fn key_resize(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                if let Some(r) = &mut self.resizing {
                    r.extend(t.chars().filter(|c| c.is_ascii_digit() || matches!(c, 'x' | 'X' | '%' | '.' | ' ')));
                }
            }
            KeyInput::Code(8) => {
                if let Some(r) = &mut self.resizing {
                    r.pop();
                }
            }
            KeyInput::Code(13) => {
                let text = self.resizing.take().unwrap_or_default();
                self.status.clear();
                if !text.trim().is_empty() {
                    self.track(|v| v.resize_to(&text));
                }
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.resizing = None;
                self.status.clear();
                return true;
            }
            _ => return false,
        }
        self.show_resizing();
        true
    }

    /// Resamples the picture to the size `text` names (see `pixels::parse_size`).
    fn resize_to(&mut self, text: &str) {
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else { return };
        match pixels::parse_size(text, picture.w, picture.h) {
            Some((w, h)) if (w, h) != (picture.w, picture.h) => {
                doc.replace_picture(Rc::new(picture.resized(w, h)));
                self.selection = None;
                self.status = format!("Resized to {w} x {h}");
            }
            Some(_) => self.status = "The picture already has that size.".into(),
            None => self.status = format!("\u{201c}{}\u{201d} is not a size the picture can be resampled to.", text.trim()),
        }
    }

    /// Edit > Find: the status line takes the search text. Only a PDF has text to search.
    fn start_find(&mut self) {
        if !self.doc.as_ref().is_some_and(|d| d.is_pdf) {
            self.status = "Find works in PDF documents.".into();
            return;
        }
        self.stop_editing();
        self.finding = Some(self.query.clone());
        self.show_finding();
    }

    /// Shows what has been typed after Find in the status line.
    fn show_finding(&mut self) {
        self.status = format!("Find: {}|", self.finding.as_deref().unwrap_or(""));
    }

    /// A key while the search text is being typed.
    fn key_find(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                if let Some(f) = &mut self.finding {
                    f.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            KeyInput::Code(8) => {
                if let Some(f) = &mut self.finding {
                    f.pop();
                }
            }
            KeyInput::Code(13) => {
                let query = self.finding.take().unwrap_or_default().trim().to_string();
                self.status.clear();
                if !query.is_empty() {
                    self.query = query.clone();
                    let sent = self.doc.as_ref().is_some_and(|d| d.send(find::find_job(query)));
                    self.status = if sent { "Searching".into() } else { "The document is not open.".into() };
                }
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.finding = None;
                self.status.clear();
                return true;
            }
            _ => return false,
        }
        self.show_finding();
        true
    }

    /// Edit > Find Next and Find Previous: goes to the next page that holds a hit, wrapping round. With nothing searched
    /// for yet, asks for the text.
    fn find_step(&mut self, forward: bool) {
        if self.found.is_empty() {
            if self.query.is_empty() {
                self.start_find();
            } else {
                self.status = find::summary(&self.query, 0);
            }
            return;
        }
        match find::next_page(&self.hit_places(), self.view.page, forward) {
            Some(page) => self.go_page(page),
            None => self.status = find::summary(&self.query, 0),
        }
    }

    /// Tools > Show Location Info: the place a picture was taken, from its EXIF block, in the status line and the Inspector.
    fn show_location(&mut self) {
        match inspector::location(&self.facts) {
            Some(place) => {
                self.status = format!("Location: {place}");
                self.inspector = true;
            }
            None => self.status = "This file has no location information.".into(),
        }
    }

    /// View > Contact Sheet: all the pages in a grid, or back to one page. The sheet opens scrolled to the shown page.
    fn toggle_contact(&mut self, ui: &Ui, lay: &Layout) {
        if self.mode == Mode::Contact {
            self.mode = Mode::Page;
            return;
        }
        self.stop_editing();
        let Some(doc) = &self.doc else { return };
        let (_, _, cw, ch) = lay.canvas();
        let sheet = canvas::contact_sheet(ui, doc, cw);
        let top = sheet.origin(self.view.page).1;
        let most = (sheet.height(doc.count()) - ch).max(0) as f32;
        self.contact_scroll = ((top - ch / 3) as f32).clamp(0.0, most);
        self.mode = Mode::Contact;
    }

    /// View > Slideshow: the pages one after another with nothing else on screen, or back to the window.
    fn toggle_slideshow(&mut self) {
        if self.mode == Mode::Slideshow {
            self.leave_slideshow();
            return;
        }
        if self.doc.is_none() {
            return;
        }
        self.stop_editing();
        self.slideshow_zoom = self.view.zoom;
        self.view.zoom = Zoom::Fit;
        self.view.pan = (0.0, 0.0);
        self.mode = Mode::Slideshow;
    }

    fn leave_slideshow(&mut self) {
        self.mode = Mode::Page;
        self.view.zoom = self.slideshow_zoom;
    }

    /// Shows the page `delta` pages on in the slideshow. It stops at the first and last page.
    fn slide_by(&mut self, delta: i64) {
        let target = (self.view.page as i64 + delta).max(0) as usize;
        self.go_page(target);
    }

    /// A key during the slideshow: arrows, space and Return move through the pages, Escape ends it.
    fn key_slideshow(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Code(1 | 27) => self.leave_slideshow(),
            KeyInput::Code(37) | KeyInput::Code(38) | KeyInput::Code(8) => self.slide_by(-1),
            KeyInput::Code(39) | KeyInput::Code(40) | KeyInput::Code(13) => self.slide_by(1),
            KeyInput::Text(t) if t == " " => self.slide_by(1),
            _ => return false,
        }
        true
    }

    /// View > Zoom to Selection: the zoom and pan that fill the canvas with the selected rectangle.
    fn zoom_to_selection(&mut self, lay: &Layout) {
        let Some(sel) = self.selection_on_page() else {
            self.status = "Select an area first.".into();
            return;
        };
        let (w, h) = self.page_dims(lay);
        let (_, _, cw, ch) = lay.canvas();
        if let Some((scale, pan)) = model::zoom_to_rect(sel, self.view.quarters, w, h, cw as usize, ch as usize) {
            self.view.zoom = Zoom::Factor(scale);
            self.view.pan = pan;
        }
    }

    /// File > New from Clipboard: opens the picture on the clipboard. It is kept as a file in the app's data folder,
    /// because the markup and the exports belong to a file.
    fn new_from_clipboard(&mut self) {
        let board = NativePasteboard::general();
        let Some(bytes) = board.data(PNG).or_else(|| board.data(JPEG)) else {
            self.status = "The clipboard holds no picture.".into();
            return;
        };
        if let Err(e) = document::decode_picture(&bytes) {
            self.status = format!("The clipboard picture could not be read: {e}");
            return;
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = document::clipboard_file_name(&bytes, stamp);
        match recents::keep_copy(&self.data_dir, &name, &bytes) {
            Ok(path) => self.open_path(&path),
            Err(e) => self.status = format!("Could not keep the clipboard picture: {e}"),
        }
    }

    /// Tools > Crop: the picture keeps the selected rectangle, and the markup moves with it.
    fn crop(&mut self) {
        self.stop_editing();
        let Some((page, sel)) = self.selection else {
            self.status = "Select an area to crop first.".into();
            return;
        };
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else {
            self.status = "Crop works on pictures, not PDFs.".into();
            return;
        };
        match pixels::crop(&picture, sel) {
            Some(cropped) => {
                doc.marks = pixels::crop_marks(&doc.marks, page, sel);
                doc.replace_picture(Rc::new(cropped));
                self.selection = None;
                self.view.pan = (0.0, 0.0);
                self.status.clear();
            }
            None => self.status = "The selection is empty.".into(),
        }
        self.save();
    }

    /// Tools > Flip Horizontal and Flip Vertical: mirrors the shown picture, and the markup with it.
    fn flip(&mut self, horizontal: bool) {
        self.stop_editing();
        let axis = pixels::source_horizontal(horizontal, self.view.quarters);
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else {
            self.status = "Flip works on pictures, not PDFs.".into();
            return;
        };
        doc.marks = pixels::flip_marks(&doc.marks, 0, axis);
        doc.replace_picture(Rc::new(pixels::flip(&picture, axis)));
        self.selection = None;
        self.save();
    }

    /// Tools > Remove Background: the background that reaches the picture's edge becomes transparent.
    fn remove_background(&mut self) {
        self.stop_editing();
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else {
            self.status = "Remove Background works on pictures, not PDFs.".into();
            return;
        };
        let cleared = pixels::remove_background(&picture, pixels::TOLERANCE);
        if cleared.px == picture.px {
            self.status = "There is no background to remove.".into();
        } else {
            doc.replace_picture(Rc::new(cleared));
            self.status = "Background removed. Export as PNG to keep the transparency.".into();
        }
    }

    /// Tools > Lift Subject: the picture becomes the subject of its plain backdrop, cropped and transparent outside it.
    fn lift_subject(&mut self) {
        self.stop_editing();
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else {
            self.status = "Lift Subject works on pictures, not PDFs.".into();
            return;
        };
        match pixels::lift_subject(&picture, pixels::TOLERANCE) {
            Some(subject) => {
                doc.replace_picture(Rc::new(subject));
                self.selection = None;
                self.status = "Subject lifted. Export as PNG to keep the transparency.".into();
            }
            None => self.status = "No subject stands out from the background.".into(),
        }
    }

    /// The Instant Alpha tool: the region of one colour around the point clicked becomes transparent.
    fn instant_alpha(&mut self, lay: &Layout, x: f32, y: f32) {
        let Some(placed) = self.placed(lay) else { return };
        let (u, v) = canvas::to_source(&placed, self.view.quarters, x, y);
        let Some(doc) = &mut self.doc else { return };
        let Some(picture) = doc.picture_rc() else {
            self.status = "Instant Alpha works on pictures, not PDFs.".into();
            return;
        };
        if picture.w == 0 || picture.h == 0 {
            return;
        }
        let px = ((u * picture.w as f32) as usize).min(picture.w - 1);
        let py = ((v * picture.h as f32) as usize).min(picture.h - 1);
        let cleared = pixels::instant_alpha(&picture, px, py, pixels::TOLERANCE);
        if cleared.px != picture.px {
            doc.replace_picture(Rc::new(cleared));
            self.status = "Export as PNG to keep the transparency.".into();
        }
    }

    /// The actions that change the document or the view, run inside `track`.
    fn act_edit(&mut self, _ui: &Ui, lay: &Layout, action: Action) -> bool {
        match action {
            Action::Previous | Action::Next => {
                let delta = if action == Action::Previous { -1 } else { 1 };
                let count = self.doc.as_ref().map_or(0, |d| d.count());
                let from = self.view.page;
                if count > 0 && self.view.step(delta, count) {
                    self.stop_editing();
                    self.trail.visit(from);
                }
                self.sync_turn();
            }
            Action::ZoomIn | Action::ZoomOut => {
                let (w, h) = self.page_dims(lay);
                let (_, _, cw, ch) = lay.canvas();
                self.view.zoom = self.view.zoom.stepped(w, h, cw as usize, ch as usize, action == Action::ZoomIn);
            }
            Action::ActualSize => self.view.zoom = Zoom::Factor(1.0),
            Action::FitWindow => self.view.zoom = Zoom::Fit,
            Action::RotateLeft => self.rotate(-1),
            Action::RotateRight => self.rotate(1),
            Action::UseTool(tool) => {
                self.stop_editing();
                self.tool = tool;
            }
            Action::ShowPages | Action::ShowOutline | Action::ShowNotes | Action::ShowBookmarks | Action::ShowSearch => {
                let side = match action {
                    Action::ShowPages => Side::Pages,
                    Action::ShowOutline => Side::Outline,
                    Action::ShowNotes => Side::Notes,
                    Action::ShowBookmarks => Side::Bookmarks,
                    _ => Side::Search,
                };
                self.sidebar = true;
                if self.side != side {
                    self.side = side;
                    self.view.sidebar_scroll = 0.0;
                }
            }
            Action::Crop => self.crop(),
            Action::FlipHorizontal => self.flip(true),
            Action::FlipVertical => self.flip(false),
            Action::RemoveBackground => self.remove_background(),
            Action::LiftSubject => self.lift_subject(),
            Action::MovePageUp | Action::MovePageDown => {
                let delta = if action == Action::MovePageUp { -1 } else { 1 };
                if let Some(doc) = &mut self.doc {
                    if doc.is_pdf {
                        self.view.page = doc.move_page(self.view.page, delta);
                    }
                }
                self.sync_turn();
            }
            Action::DeletePage => {
                let page = self.view.page;
                let deleted = self.doc.as_mut().is_some_and(|d| d.is_pdf && d.delete_page(page));
                if deleted {
                    let count = self.doc.as_ref().map_or(0, |d| d.count());
                    let keep = self.view.page;
                    self.view.go(keep, count);
                    self.sync_turn();
                }
            }
            Action::Brighter => self.adjust.exposure = (self.adjust.exposure + 0.5).clamp(-2.0, 2.0),
            Action::Darker => self.adjust.exposure = (self.adjust.exposure - 0.5).clamp(-2.0, 2.0),
            Action::Saturate => self.adjust.saturation = (self.adjust.saturation + 0.25).clamp(0.0, 2.0),
            Action::Desaturate => self.adjust.saturation = (self.adjust.saturation - 0.25).clamp(0.0, 2.0),
            Action::Sharpen => self.adjust.sharpness = (self.adjust.sharpness + 0.25).clamp(0.0, 2.0),
            Action::Soften => self.adjust.sharpness = (self.adjust.sharpness - 0.25).clamp(0.0, 2.0),
            Action::ResetColour => self.adjust = Adjust::default(),
            Action::Print => self.print(),
            Action::RevertToOriginal => self.revert(),
            Action::Undo
            | Action::Redo
            | Action::GoBack
            | Action::GoForward
            | Action::FirstPage
            | Action::LastPage
            | Action::GoToPage
            | Action::AddBookmark
            | Action::NextBookmark
            | Action::PreviousBookmark
            | Action::ToggleSidebar
            | Action::ToggleMagnifier
            | Action::Inspector
            | Action::ShowLocation
            | Action::ContactSheet
            | Action::Slideshow
            | Action::ZoomToSelection
            | Action::NewFromClipboard
            | Action::Find
            | Action::FindNext
            | Action::FindPrevious
            | Action::AdjustSize
            | Action::Save
            | Action::RevertToSaved
            | Action::CopyText
            | Action::SelectAll
            | Action::SaveSignature
            | Action::ClearSignatures
            | Action::ToggleToolbar
            | Action::ToggleRememberPage
            | Action::ToggleContactDefault
            | Action::SetAuthor
            | Action::ShowColorPanel
            | Action::ExportAs
            | Action::ChooseFormat(_)
            | Action::ShowSignatures => {}
            Action::ExportNow => match self.export_format {
                export_panel::Format::Png => self.export_raster(Raster::Png),
                export_panel::Format::Jpeg => self.export_raster(Raster::Jpeg(self.jpeg_quality)),
                export_panel::Format::Pdf => self.export_pdf(),
            },
            Action::QualityHigh | Action::QualityMedium | Action::QualityLow => {
                if let Some(quality) = action.jpeg_quality() {
                    self.jpeg_quality = quality;
                }
            }
            Action::ExportPng => self.export_raster(Raster::Png),
            Action::ExportJpeg => self.export_raster(Raster::Jpeg(self.jpeg_quality)),
            Action::ExportPdf => self.export_pdf(),
            Action::ExportAt25 | Action::ExportAt50 | Action::ExportAt100 | Action::ExportAt200 => {
                if let Some(scale) = action.export_scale() {
                    self.export_scale = scale;
                }
            }
        }
        true
    }

    /// File > Revert to Original: drops the markup, restores the page order and turns and resets the colour.
    fn revert(&mut self) {
        self.editing = None;
        if let Some(doc) = &mut self.doc {
            let n = doc.sizes.len();
            doc.marks.clear();
            doc.order = (0..n).collect();
            doc.turns = vec![0; n];
            doc.restore_original();
        }
        self.selection = None;
        self.adjust = Adjust::default();
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let keep = self.view.page;
        self.view.go(keep, count);
        self.sync_turn();
        self.save();
    }

    /// Prints the document as it is marked up: a PDF's edited copy (the worker writes and sends it), or a picture's
    /// marked up page written as a PNG and sent from here.
    fn print(&mut self) {
        let Some(doc) = &self.doc else { return };
        if doc.is_pdf {
            let target = printjob::scratch_file(&self.data_dir, &doc.path, "pdf");
            let job = printjob::pdf_job(doc.path.clone(), target, doc.order.clone(), doc.turns.clone(), doc.marks.clone(), self.author(), doc.field_edits());
            self.status = if doc.send(job) { "Printing".into() } else { "The document is not open.".into() };
            return;
        }
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, doc.turn_of(page), &self.adjust, 1.0);
        let target = printjob::scratch_file(&self.data_dir, &doc.path, "png");
        let written = match target.parent() {
            Some(dir) => std::fs::create_dir_all(dir).map_err(|e| e.to_string()),
            None => Ok(()),
        }
        .and_then(|()| export::write_image(&out, Raster::Png, &target));
        self.status = match written {
            Ok(()) => printjob::print_file(target),
            Err(e) => format!("Printing failed: {e}"),
        };
    }

    /// Export PNG or JPEG: the shown page at the export size goes into a file the user picks in the save panel. A PDF
    /// page is drawn by its worker first.
    fn export_raster(&mut self, raster: Raster) {
        let Some(doc) = &self.doc else { return };
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let turns = doc.turn_of(page);
        let target = export::target(&doc.path, raster.ext());
        if doc.is_pdf {
            let temp = self.saving_file(&doc.path, raster.ext());
            let job = export::pdf_page_job(src, turns, doc.marks.clone(), self.adjust, self.export_scale, raster, temp.clone());
            self.ask_export(&target, raster.ext(), Export::Worker { job: Some(job), temp, file: None });
            return;
        }
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, turns, &self.adjust, self.export_scale);
        match export::encode(&out, raster) {
            Ok(bytes) => self.ask_export(&target, raster.ext(), Export::Picture(bytes)),
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    /// Export PDF: the PDF with its markup, page order and turns goes into a file the user picks in the save panel. The
    /// worker draws it first.
    fn export_pdf(&mut self) {
        let Some(doc) = &self.doc else { return };
        if !doc.is_pdf {
            return;
        }
        let target = export::target(&doc.path, "pdf");
        let temp = self.saving_file(&doc.path, "pdf");
        let job = pdfmarks::save_job(doc.path.clone(), temp.clone(), doc.order.clone(), doc.turns.clone(), doc.marks.clone(), self.author(), pdfmarks::Insertion::None, doc.field_edits());
        self.ask_export(&target, "pdf", Export::Worker { job: Some(job), temp, file: None });
    }

    /// Holds `what` until the save panel answers. The panel is asked for on the next frame, suggesting `target`'s name.
    fn ask_export(&mut self, target: &Path, ext: &'static str, what: Export) {
        self.pending_export = Some(PendingExport { name: name_of(target), ext, request: None, what });
    }

    /// Whether the export waiting for its file is drawn by the PDF worker, so the worker's reply is its result.
    fn export_is_drawn(&self) -> bool {
        self.pending_export.as_ref().is_some_and(|p| matches!(p.what, Export::Worker { file: Some(_), .. }))
    }

    /// The save panel answered with the file to write (`SaveTo`). A picture goes into it at once; a PDF is sent to the
    /// worker, and the copy goes into the file when the worker has written it. Either way the save is then committed.
    fn write_export(&mut self, ctx: &Ctx, mut doc: bifrost_app::Document) -> bool {
        let request = doc.request;
        if !self.pending_export.as_ref().is_some_and(|p| p.request == Some(request)) {
            ctx.abort_document(request);
            return false;
        }
        let Some(PendingExport { name, ext, what, .. }) = self.pending_export.take() else { return false };
        match what {
            Export::Picture(bytes) => match doc.file.write_all(&bytes) {
                Ok(()) => ctx.commit_document(request),
                Err(e) => {
                    self.status = format!("Export failed: {e}");
                    ctx.abort_document(request);
                }
            },
            Export::Worker { job, temp, .. } => {
                let sent = job.is_some_and(|job| self.doc.as_ref().is_some_and(|d| d.send(job)));
                if sent {
                    let what = Export::Worker { job: None, temp, file: Some(doc.file) };
                    self.pending_export = Some(PendingExport { name, ext, request: Some(request), what });
                } else {
                    self.status = "The document is not open.".into();
                    ctx.abort_document(request);
                }
            }
        }
        false
    }

    /// The worker has drawn the export into its temporary file: the copy goes into the chosen file and the save commits.
    fn finish_drawn_export(&mut self, ctx: &Ctx, result: Result<PathBuf, String>) {
        let Some(PendingExport { request: Some(request), what: Export::Worker { file: Some(mut file), .. }, .. }) = self.pending_export.take() else {
            return;
        };
        let copied = result.and_then(|drawn| {
            let mut from = std::fs::File::open(&drawn).map_err(|e| e.to_string())?;
            std::io::copy(&mut from, &mut file).map(|_| ()).map_err(|e| e.to_string())
        });
        match copied {
            Ok(()) => ctx.commit_document(request),
            Err(e) => {
                self.status = format!("Export failed: {e}");
                ctx.abort_document(request);
            }
        }
    }

    fn touch(&mut self, ui: &Ui, width: i32, height: i32, kind: TouchKind, x: f32, y: f32) -> bool {
        let lay = self.layout(ui, width, height);
        let (cx, cy, cw, ch) = lay.canvas();
        let over_canvas = x >= cx as f32 && x < (cx + cw) as f32 && y >= cy as f32 && y < (cy + ch) as f32;
        if self.loupe_on && over_canvas {
            self.loupe = Some((x, y));
        }
        match kind {
            TouchKind::Down => {
                // a press ends the note being typed (an empty note is dropped) before the edit is recorded
                self.stop_editing();
                self.track(|v| v.touch_down(ui, &lay, x, y)) || self.loupe_on
            }
            TouchKind::Move => self.touch_move(ui, &lay, x, y) || (self.loupe_on && over_canvas),
            // a slider drag records its own undo step when it ends, from the state it began with
            TouchKind::Up | TouchKind::Cancel if matches!(self.gesture, Some(Gesture::Slider { .. })) => self.touch_up(ui, &lay, x, y),
            TouchKind::Up | TouchKind::Cancel => self.track(|v| v.touch_up(ui, &lay, x, y)),
        }
    }

    fn touch_down(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        if self.mode == Mode::Slideshow {
            self.slide_by(1);
            return true;
        }
        if let Some(action) = canvas::button_hit(ui, lay, self.doc.is_some(), self.tool, self.side, x, y) {
            return self.act(ui, lay, action);
        }
        // the signature picker takes the clicks on its side; a click on a signature places it on the shown page
        if self.signatures_shown && lay.inspector > 0 && y >= lay.bar as f32 && x >= (lay.w - lay.inspector) as f32 {
            let metrics = canvas::export_metrics(ui, lay);
            // the press is already inside the undo record that touch() opens around this call
            if let Some(index) = signature_panel::hit(&metrics, self.signatures.len(), x, y) {
                self.insert_signature(index);
            }
            return true;
        }
        // the Export As panel takes the clicks on its side; a click on one of its buttons runs that button's action
        if self.export_shown && lay.inspector > 0 && y >= lay.bar as f32 && x >= (lay.w - lay.inspector) as f32 {
            let panel = export_panel::panel(&canvas::export_metrics(ui, lay), &self.export_choices());
            return match export_panel::hit(&panel, x, y) {
                Some(action) => self.act(ui, lay, action),
                None => true,
            };
        }
        // the Adjust Color panel takes the clicks on its side; its value changes while the finger moves
        if self.color_panel && lay.inspector > 0 && y >= lay.bar as f32 && x >= (lay.w - lay.inspector) as f32 {
            if let Some(which) = canvas::slider_hit(ui, lay, self.adjust.sliders().len(), x, y) {
                self.gesture = Some(Gesture::Slider { which, before: self.snapshot() });
            }
            return true;
        }
        if y < lay.bar as f32 {
            return false;
        }
        if x < lay.side as f32 {
            if self.doc.is_none() {
                return false;
            }
            let rows = self.side_rows(ui);
            let hit = if self.side == Side::Pages {
                canvas::sidebar_hit(lay, &rows, self.view.sidebar_scroll, x, y)
            } else {
                self.doc.as_ref().and_then(|doc| {
                    canvas::source_list_hit(ui, lay, doc, self.side, &self.found, self.view.page, self.view.sidebar_scroll, x, y)
                })
            };
            if let Some(i) = hit {
                match self.side {
                    Side::Pages => self.go_page(i),
                    Side::Outline => {
                        // an outline entry goes to its page, when it names one that is still shown
                        let target = self.doc.as_ref().and_then(|d| {
                            let src = d.outline.get(i)?.page?;
                            d.display_of(src)
                        });
                        if let Some(page) = target {
                            self.go_page(page);
                        }
                    }
                    side => {
                        // a markup, bookmark or search entry goes to the page it names
                        let target = self.doc.as_ref().and_then(|d| canvas::side_items(d, side, &self.found).get(i).and_then(|item| item.1));
                        if let Some(page) = target {
                            self.go_page(page);
                        }
                    }
                }
            }
            self.gesture = Some(Gesture::Scroll { last: y });
            return true;
        }
        let (cx, _, cw, _) = lay.canvas();
        if self.doc.is_none() || x >= (cx + cw) as f32 {
            return false;
        }
        self.stop_editing();
        // a click on a form field fills it, unless a markup tool is chosen
        if self.tool == Tool::Pan && self.mode != Mode::Contact {
            if let Some(widget) = self.form_widget_at(lay, x, y) {
                self.click_widget(widget);
                return true;
            }
        }
        self.form_focus = None;
        if self.mode == Mode::Contact {
            self.gesture = Some(Gesture::Sheet { last: y, start: (x, y) });
            return true;
        }
        if self.tool == Tool::Redact && self.doc.as_ref().is_some_and(|d| d.is_pdf) {
            self.status = "Redact works on pictures. A PDF page keeps what is under a box.".into();
            return true;
        }
        match self.tool {
            Tool::Select => {
                self.gesture = Some(Gesture::Select { start: (x, y), now: (x, y) });
                return true;
            }
            Tool::Alpha => {
                self.instant_alpha(lay, x, y);
                return true;
            }
            _ => {}
        }
        match self.tool.mark_kind() {
            None => self.gesture = Some(Gesture::Pan { last: (x, y) }),
            Some(MarkKind::Note) => {
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let a = canvas::to_source(&p, self.view.quarters, x, y);
                    let rect = Norm { x: a.0, y: a.1, w: 0.0, h: 0.0 };
                    if let Some(doc) = &mut self.doc {
                        doc.marks.push(Mark { kind: MarkKind::Note, page: src, rect, color: MARK_BLUE, text: String::new(), ink: Vec::new() });
                        self.editing = Some(doc.marks.len() - 1);
                    }
                    self.save();
                }
            }
            Some(MarkKind::Text) => {
                // a click places a text box at the point and the typing that follows fills it
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let a = canvas::to_source(&p, self.view.quarters, x, y);
                    let rect = Norm { x: a.0, y: a.1, w: TEXT_BOX.0, h: TEXT_BOX.1 };
                    if let Some(doc) = &mut self.doc {
                        doc.marks.push(Mark { kind: MarkKind::Text, page: src, rect, color: MARK_BLUE, text: String::new(), ink: Vec::new() });
                        self.editing = Some(doc.marks.len() - 1);
                    }
                    self.save();
                }
            }
            Some(MarkKind::Ink) => self.gesture = Some(Gesture::Ink { points: vec![(x, y)] }),
            Some(kind) => self.gesture = Some(Gesture::Drag { start: (x, y), now: (x, y), kind }),
        }
        true
    }

    fn touch_move(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        match &mut self.gesture {
            Some(Gesture::Pan { last }) => {
                self.view.pan.0 += x - last.0;
                self.view.pan.1 += y - last.1;
                *last = (x, y);
                if let Some(p) = self.placed(lay) {
                    self.view.pan = p.pan;
                }
                true
            }
            Some(Gesture::Scroll { last }) => {
                let delta = y - *last;
                *last = y;
                let max = self.scroll_max(ui, lay);
                if bifrost_app::pans_now() {
                    self.view.sidebar_scroll = (self.view.sidebar_scroll - delta).clamp(0.0, max);
                }
                true
            }
            Some(Gesture::Drag { now, .. }) => {
                *now = (x, y);
                true
            }
            Some(Gesture::Ink { points }) => {
                // a point per pixel of movement is plenty for a stroke
                let near = points.last().is_some_and(|p| (p.0 - x).abs() + (p.1 - y).abs() < 1.0);
                if !near {
                    points.push((x, y));
                }
                true
            }
            Some(Gesture::Select { now, .. }) => {
                *now = (x, y);
                true
            }
            Some(Gesture::Sheet { last, .. }) => {
                let delta = y - *last;
                *last = y;
                let most = self.contact_max(ui, lay);
                if bifrost_app::pans_now() {
                    self.contact_scroll = (self.contact_scroll - delta).clamp(0.0, most);
                }
                true
            }
            Some(Gesture::Slider { which, .. }) => {
                let which = *which;
                self.drag_slider(ui, lay, which, x);
                true
            }
            None => false,
        }
    }

    /// Sets Adjust Color slider `which` from the point `x` across its track.
    fn drag_slider(&mut self, ui: &Ui, lay: &Layout, which: usize, x: f32) {
        let sliders = self.adjust.sliders();
        if let Some(slider) = sliders.get(which) {
            let value = canvas::slider_value(ui, lay, slider, x);
            self.adjust.set_slider(which, value);
        }
    }

    /// How far the contact sheet scrolls.
    fn contact_max(&self, ui: &Ui, lay: &Layout) -> f32 {
        let Some(doc) = &self.doc else { return 0.0 };
        let (_, _, cw, ch) = lay.canvas();
        (canvas::contact_sheet(ui, doc, cw).height(doc.count()) - ch).max(0) as f32
    }

    fn touch_up(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        let Some(gesture) = self.gesture.take() else { return false };
        match gesture {
            Gesture::Drag { start, kind, .. } => {
                let moved = (x - start.0).abs() + (y - start.1).abs();
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let q = self.view.quarters;
                    let a = canvas::to_source(&p, q, start.0, start.1);
                    let b = canvas::to_source(&p, q, x, y);
                    let mark = model::drag_mark(kind, src, a, b, color_for(kind));
                    if model::drag_counts(kind, moved, mark.rect, q) {
                        if let Some(doc) = &mut self.doc {
                            doc.marks.push(mark);
                        }
                        self.save();
                    }
                }
            }
            Gesture::Select { start, .. } => {
                let moved = (x - start.0).abs() + (y - start.1).abs();
                self.selection = None;
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let q = self.view.quarters;
                    let rect = Norm::from_corners(canvas::to_source(&p, q, start.0, start.1), canvas::to_source(&p, q, x, y));
                    if moved >= 4.0 && rect.w > 0.0 && rect.h > 0.0 {
                        self.selection = Some((src, rect));
                    }
                }
            }
            Gesture::Slider { which, before } => {
                self.drag_slider(ui, lay, which, x);
                if let Some(before) = before {
                    if self.snapshot().as_ref() != Some(&before) {
                        self.history.record(before);
                    }
                }
            }
            Gesture::Sheet { start, .. } => {
                let moved = (x - start.0).abs() + (y - start.1).abs();
                if moved < 4.0 {
                    let (cx, cy, cw, _) = lay.canvas();
                    let hit = self.doc.as_ref().and_then(|doc| {
                        let sheet = canvas::contact_sheet(ui, doc, cw);
                        sheet.hit(doc.count(), self.contact_scroll, x - cx as f32, y - cy as f32)
                    });
                    if let Some(page) = hit {
                        self.go_page(page);
                        self.mode = Mode::Page;
                    }
                }
            }
            Gesture::Ink { mut points } => {
                points.push((x, y));
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let q = self.view.quarters;
                    let ink: Vec<(f32, f32)> = points.iter().map(|&(px, py)| canvas::to_source(&p, q, px, py)).collect();
                    if ink.len() >= 2 {
                        if let Some(doc) = &mut self.doc {
                            doc.marks.push(stroke(src, ink));
                        }
                        self.save();
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// Shows the password request, with the typing as stars.
    fn show_asking(&mut self) {
        if let Some(ask) = &self.asking {
            self.status = ask.message(&name_of(&ask.path));
        }
    }

    /// A key while the password of a PDF is being typed. Return opens the file with what was typed, and Escape leaves
    /// the file unopened.
    fn key_password(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                if let Some(ask) = &mut self.asking {
                    ask.type_text(t);
                }
            }
            KeyInput::Code(8) => {
                if let Some(ask) = &mut self.asking {
                    ask.delete();
                }
            }
            KeyInput::Code(13) => {
                let Some(ask) = self.asking.take() else { return false };
                let password = ask.password();
                self.password = Some((ask.path.clone(), password.clone()));
                self.open_document(&ask.path, password);
                return true;
            }
            KeyInput::Code(1 | 27) => {
                self.asking = None;
                self.status = "The document was not opened.".into();
                return true;
            }
            _ => return false,
        }
        self.show_asking();
        true
    }

    fn key(&mut self, ctx: &Ctx, key: KeyInput) -> bool {
        if self.asking.is_some() {
            return self.key_password(&key);
        }
        if self.authoring.is_some() {
            return self.key_author(&key);
        }
        if self.mode == Mode::Slideshow {
            return self.key_slideshow(&key);
        }
        if self.goto.is_some() {
            return self.key_goto(&key);
        }
        if self.finding.is_some() {
            return self.key_find(&key);
        }
        if self.resizing.is_some() {
            return self.key_resize(&key);
        }
        if self.mode == Mode::Contact && matches!(key, KeyInput::Code(1 | 27)) {
            self.mode = Mode::Page;
            return true;
        }
        if let Some(widget) = self.form_focus {
            if self.key_form(widget, &key) {
                return true;
            }
        }
        match &key {
            KeyInput::Text(text) => {
                let Some(i) = self.editing else { return false };
                if let Some(m) = self.doc.as_mut().and_then(|d| d.marks.get_mut(i)) {
                    m.text.push_str(text);
                }
                true
            }
            KeyInput::Code(8) => {
                let Some(i) = self.editing else { return false };
                if let Some(m) = self.doc.as_mut().and_then(|d| d.marks.get_mut(i)) {
                    m.text.pop();
                }
                true
            }
            KeyInput::Code(13) | KeyInput::Code(1 | 27) => {
                if self.editing.is_none() {
                    return false;
                }
                self.stop_editing();
                true
            }
            KeyInput::Code(37) => self.key_page(ctx, Action::Previous),
            KeyInput::Code(39) => self.key_page(ctx, Action::Next),
            _ => false,
        }
    }

    fn key_page(&mut self, ctx: &Ctx, action: Action) -> bool {
        let (w, h) = self.size(ctx);
        let lay = self.layout(&ctx.ui, w, h);
        self.act(&ctx.ui, &lay, action)
    }

    /// The window size the last frame was drawn at. Layout is recomputed from it for input.
    fn size(&self, ctx: &Ctx) -> (i32, i32) {
        (ctx.cfg.w as i32, ctx.cfg.h as i32)
    }
}

/// A stroke on source page `page`, with its bounding box as its rectangle.
fn stroke(page: usize, ink: Vec<(f32, f32)>) -> Mark {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &ink {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let rect = if ink.is_empty() { Norm { x: 0.0, y: 0.0, w: 0.0, h: 0.0 } } else { Norm { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } };
    Mark { kind: MarkKind::Ink, page, rect, color: MARK_BLUE, text: String::new(), ink }
}

/// The colour a mark of this kind is drawn in.
fn color_for(kind: MarkKind) -> u32 {
    match kind {
        MarkKind::Highlight => HIGHLIGHT,
        MarkKind::Strike => MARK_RED,
        _ => MARK_BLUE,
    }
}

/// The file name of `path`, for messages.
fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The local path for an open request: a `file:` URL or an absolute path.
fn path_from_url(url: &str) -> Option<PathBuf> {
    if url.starts_with("file:") {
        return url::Url::parse(url).ok()?.to_file_path().ok();
    }
    let path = Path::new(url);
    path.is_absolute().then(|| path.to_path_buf())
}

fn item(action: Action, enabled: bool, checked: bool) -> AppMenuItem {
    item_labelled(action, action.label(), enabled, checked)
}

/// A menu item for `action` under a different label, for the items that switch between two (Show and Hide).
fn item_labelled(action: Action, label: &str, enabled: bool, checked: bool) -> AppMenuItem {
    let mut m = AppMenuItem::new(action.id(), label).enabled(enabled).checked(checked);
    m = match action {
        Action::NewFromClipboard => m.key("cmd+n"),
        Action::Find => m.key("cmd+f"),
        Action::FindNext => m.key("cmd+g"),
        Action::FindPrevious => m.key("cmd+shift+g"),
        Action::Crop => m.key("cmd+k"),
        Action::Inspector => m.key("cmd+i"),
        Action::ContactSheet => m.key("cmd+2"),
        Action::Slideshow => m.key("cmd+shift+f"),
        Action::AdjustSize => m.key("cmd+alt+i"),
        Action::ZoomIn => m.key("cmd+="),
        Action::ZoomOut => m.key("cmd+-"),
        Action::ActualSize => m.key("cmd+0"),
        Action::FitWindow => m.key("cmd+9"),
        Action::RotateLeft => m.key("cmd+l"),
        Action::RotateRight => m.key("cmd+r"),
        Action::ExportPng => m.key("cmd+shift+e"),
        Action::Print => m.key("cmd+p"),
        Action::Undo => m.key("cmd+z"),
        Action::Redo => m.key("cmd+shift+z"),
        Action::GoBack => m.key("cmd+alt+b"),
        Action::GoForward => m.key("cmd+alt+f"),
        Action::GoToPage => m.key("cmd+alt+g"),
        Action::AddBookmark => m.key("cmd+d"),
        Action::Save => m.key("cmd+s"),
        Action::ExportAs => m.key("cmd+shift+s"),
        Action::CopyText => m.key("cmd+c"),
        Action::SelectAll => m.key("cmd+a"),
        Action::ToggleToolbar => m.key("cmd+alt+t"),
        _ => m,
    };
    m
}

impl App for Viewer {
    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["service.open_images"]
    }

    /// Services menu: "Open Images in Viewer" on a picture or PDF selection. The first picture or PDF of the paths
    /// opens, since the Viewer shows one document at a time.
    fn rpc(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        if method != "service.open_images" {
            return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}")));
        }
        if ctx.is_none() {
            return Err(RpcError::new(INTERNAL_ERROR, "Viewer is not running yet"));
        }
        let call = ServiceCall::from_params(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service call has no paths"))?;
        let path = call
            .paths
            .iter()
            .find(|p| is_viewable_path(p.as_str()) && Path::new(p.as_str()).is_file())
            .map(PathBuf::from)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "no picture or PDF in the selection"))?;
        self.open_path(&path);
        Ok(Json::Obj(Vec::new()))
    }

    fn id(&self) -> &str {
        "is.olibuijr.preview.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the app's private storage holds the markup sidecars
        if self.data_dir.as_os_str().is_empty() {
            self.data_dir = ctx.data_dir.clone();
            self.recents = recents::load(&self.data_dir);
            self.prefs = prefs::load(&self.data_dir);
            self.signatures = signature::load(&self.data_dir);
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let open = self.doc.is_some();
        let pdf = self.doc.as_ref().is_some_and(|d| d.is_pdf);
        // rotation needs a page with a picture to turn
        let picture = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)).is_some();
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let bookmarks = self.doc.as_ref().map(|d| d.bookmark_places()).unwrap_or_default();
        let outline = self.doc.as_ref().is_some_and(|d| pdf && !d.outline.is_empty());
        let is_picture = open && !pdf;
        let has_selection = self.selection_on_page().is_some();
        let tools = [
            Tool::Pan,
            Tool::Select,
            Tool::Highlight,
            Tool::Underline,
            Tool::Strike,
            Tool::Note,
            Tool::Rect,
            Tool::Rounded,
            Tool::Oval,
            Tool::Line,
            Tool::Arrow,
            Tool::Sign,
            Tool::Alpha,
            Tool::Text,
            Tool::Star,
            Tool::Polygon,
            Tool::Bubble,
            Tool::Redact,
        ];
        // Instant Alpha and Redact change the pixels of a picture, so they are for pictures only
        let mut tool_items: Vec<AppMenuItem> = tools
            .into_iter()
            .map(|t| item(Action::UseTool(t), if matches!(t, Tool::Alpha | Tool::Redact) { is_picture } else { open }, self.tool == t))
            .collect();
        tool_items.push(AppMenuItem::separator());
        tool_items.extend([
            item(Action::Crop, is_picture && has_selection, false),
            item(Action::FlipHorizontal, is_picture, false),
            item(Action::FlipVertical, is_picture, false),
            item(Action::AdjustSize, is_picture, false),
            item(Action::RemoveBackground, is_picture, false),
            item(Action::LiftSubject, is_picture, false),
            AppMenuItem::separator(),
            item(Action::SaveSignature, open, false),
            AppMenuItem::submenu("Insert Signature").enabled(open && !self.signatures.is_empty()),
        ]);
        for i in 0..self.signatures.len() {
            tool_items.push(AppMenuItem::new(MENU_SIGNATURE + i as u32, &format!("Signature {}", i + 1)).enabled(open).depth(1));
        }
        tool_items.push(item(Action::ClearSignatures, !self.signatures.is_empty(), false));
        tool_items.push(item_labelled(
            Action::ShowSignatures,
            if self.signatures_shown { "Hide Signatures" } else { "Show Signatures" },
            open,
            self.signatures_shown,
        ));
        tool_items.extend([
            AppMenuItem::separator(),
            item_labelled(Action::ToggleMagnifier, if self.loupe_on { "Hide Magnifier" } else { "Show Magnifier" }, open, self.loupe_on),
            item_labelled(Action::Inspector, if self.inspector { "Hide Inspector" } else { "Show Inspector" }, open, self.inspector),
            item(Action::ShowLocation, open, false),
        ]);
        let scale_items: Vec<AppMenuItem> = [Action::ExportAt25, Action::ExportAt50, Action::ExportAt100, Action::ExportAt200]
            .into_iter()
            .map(|a| item(a, open, a.export_scale() == Some(self.export_scale)))
            .collect();
        let mut file_items = vec![
            item(Action::NewFromClipboard, true, false),
            AppMenuItem::new(MENU_OPEN, "Open\u{2026}").key("cmd+o"),
            AppMenuItem::submenu("Open Recent"),
        ];
        for (i, path) in self.recents.paths().iter().enumerate() {
            file_items.push(AppMenuItem::new(MENU_RECENT + i as u32, &name_of(path)).depth(1));
        }
        if !self.recents.is_empty() {
            file_items.push(AppMenuItem::separator().depth(1));
        }
        file_items.push(AppMenuItem::new(MENU_CLEAR_RECENTS, "Clear Menu").enabled(!self.recents.is_empty()).depth(1));
        file_items.extend([AppMenuItem::separator(), item(Action::Print, open, false), AppMenuItem::separator()]);
        file_items.extend([item(Action::ExportPng, open, false), item(Action::ExportJpeg, open, false)]);
        file_items.push(AppMenuItem::separator());
        file_items.extend(scale_items);
        file_items.push(AppMenuItem::separator());
        file_items.extend(
            [Action::QualityHigh, Action::QualityMedium, Action::QualityLow].map(|a| item(a, open, a.jpeg_quality() == Some(self.jpeg_quality))),
        );
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::ExportPdf, pdf, false));
        file_items.push(item(Action::ExportAs, open, self.export_shown));
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::Save, open, false));
        file_items.push(item(Action::RevertToSaved, open, false));
        file_items.push(item(Action::RevertToOriginal, open, false));
        file_items.push(AppMenuItem::separator());
        file_items.push(AppMenuItem::submenu("Take Screenshot"));
        file_items.push(AppMenuItem::new(MENU_SHOT_SELECTION, "From Selection\u{2026}").depth(1));
        file_items.push(AppMenuItem::new(MENU_SHOT_WINDOW, "From Window\u{2026}").depth(1));
        file_items.push(AppMenuItem::new(MENU_SHOT_SCREEN, "From Entire Screen").depth(1));
        let sidebar_label = if self.sidebar { "Hide Sidebar" } else { "Show Sidebar" };
        vec![
            AppMenu { title: "File".into(), items: file_items },
            AppMenu {
                title: "Preview".into(),
                items: vec![
                    item(Action::Undo, self.history.can_undo(), false),
                    item(Action::Redo, self.history.can_redo(), false),
                    AppMenuItem::separator(),
                    item(Action::Find, pdf, false),
                    item(Action::FindNext, pdf, false),
                    item(Action::FindPrevious, pdf, false),
                    AppMenuItem::separator(),
                    item(Action::CopyText, open, false),
                    item(Action::SelectAll, open, false),
                    AppMenuItem::separator(),
                    AppMenuItem::submenu("Speech"),
                    AppMenuItem::new(MENU_SPEAK_START, "Start Speaking").enabled(open).depth(1),
                    AppMenuItem::new(MENU_SPEAK_STOP, "Stop Speaking").depth(1),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    item(Action::ZoomIn, open, false),
                    item(Action::ZoomOut, open, false),
                    item(Action::ActualSize, open, self.view.zoom == Zoom::Factor(1.0)),
                    item(Action::FitWindow, open, self.view.zoom == Zoom::Fit),
                    AppMenuItem::separator(),
                    item(Action::RotateLeft, picture, false),
                    item(Action::RotateRight, picture, false),
                    AppMenuItem::separator(),
                    item(Action::ZoomToSelection, has_selection, false),
                    AppMenuItem::separator(),
                    item_labelled(Action::ContactSheet, if self.mode == Mode::Contact { "Single Page" } else { "Contact Sheet" }, open, self.mode == Mode::Contact),
                    item(Action::Slideshow, open, false),
                    AppMenuItem::separator(),
                    AppMenuItem::new(Action::ToggleSidebar.id(), sidebar_label).key("cmd+alt+s").enabled(open),
                    item_labelled(Action::ToggleToolbar, if self.toolbar { "Hide Toolbar" } else { "Show Toolbar" }, open, false),
                    item(Action::ShowPages, open, self.side == Side::Pages),
                    item(Action::ShowOutline, outline, self.side == Side::Outline),
                    item(Action::ShowNotes, open, self.side == Side::Notes),
                    item(Action::ShowBookmarks, open, self.side == Side::Bookmarks),
                    item(Action::ShowSearch, !self.found.is_empty(), self.side == Side::Search),
                ],
            },
            AppMenu {
                title: "Go".into(),
                items: vec![
                    item(Action::Previous, count > 1 && self.view.page > 0, false),
                    item(Action::Next, count > 1 && self.view.page + 1 < count, false),
                    item(Action::FirstPage, count > 1 && self.view.page > 0, false),
                    item(Action::LastPage, count > 1 && self.view.page + 1 < count, false),
                    item(Action::GoToPage, count > 1, false),
                    AppMenuItem::separator(),
                    item(Action::GoBack, self.trail.can_back(), false),
                    item(Action::GoForward, self.trail.can_forward(), false),
                    AppMenuItem::separator(),
                    item(Action::AddBookmark, open, self.src().is_some_and(|s| self.doc.as_ref().is_some_and(|d| d.bookmarks.contains(&s)))),
                    item(Action::PreviousBookmark, !bookmarks.is_empty(), false),
                    item(Action::NextBookmark, !bookmarks.is_empty(), false),
                ],
            },
            AppMenu {
                title: "Pages".into(),
                items: vec![
                    AppMenuItem::submenu("Insert").enabled(pdf),
                    AppMenuItem::new(MENU_INSERT_FILE, "Page from File\u{2026}").enabled(pdf).depth(1),
                    AppMenuItem::new(MENU_INSERT_BLANK, "Blank Page").enabled(pdf).depth(1),
                    AppMenuItem::separator(),
                    item(Action::MovePageUp, pdf && self.view.page > 0, false),
                    item(Action::MovePageDown, pdf && self.view.page + 1 < count, false),
                    item(Action::DeletePage, pdf && count > 1, false),
                ],
            },
            AppMenu { title: "Tools".into(), items: tool_items },
            AppMenu {
                title: "Adjust".into(),
                items: vec![
                    item(Action::Brighter, open, false),
                    item(Action::Darker, open, false),
                    item(Action::Saturate, open, false),
                    item(Action::Desaturate, open, false),
                    item(Action::Sharpen, open, false),
                    item(Action::Soften, open && self.adjust.sharpness > 0.0, false),
                    item(Action::ResetColour, open && !self.adjust.is_identity(), false),
                    AppMenuItem::separator(),
                    item_labelled(
                        Action::ShowColorPanel,
                        if self.color_panel { "Hide Adjust Color" } else { "Show Adjust Color" },
                        open,
                        self.color_panel,
                    ),
                ],
            },
            AppMenu {
                title: "Settings".into(),
                items: vec![
                    item(Action::ToggleRememberPage, true, self.prefs.remember_page),
                    item(Action::ToggleContactDefault, true, self.prefs.contact_sheet),
                    AppMenuItem::separator(),
                    item(Action::SetAuthor, true, false),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.open_menu(ctx, id) {
            return true;
        }
        match id {
            MENU_SPEAK_START => return self.speak(),
            MENU_SPEAK_STOP => {
                speech::stop(ctx);
                return false;
            }
            MENU_SHOT_SELECTION => return self.take_shot(ctx, screenshot::Pick::Selection),
            MENU_SHOT_WINDOW => return self.take_shot(ctx, screenshot::Pick::Window),
            MENU_SHOT_SCREEN => return self.take_shot(ctx, screenshot::Pick::Screen),
            _ => {}
        }
        if (MENU_SIGNATURE..MENU_SIGNATURE + signature::MAX_SIGNATURES as u32).contains(&id) {
            let index = (id - MENU_SIGNATURE) as usize;
            self.track(|v| v.insert_signature(index));
            return true;
        }
        let Some(action) = Action::from_id(id) else { return false };
        // Copy reaches the clipboard through the window's context, which only this entry point has
        if action == Action::CopyText {
            self.copy(ctx);
            return true;
        }
        let lay = self.layout(&ctx.ui, ctx.cfg.w as i32, ctx.cfg.h as i32);
        self.act(&ctx.ui, &lay, action)
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Open(url) => match path_from_url(&url) {
                Some(path) => {
                    self.open_path(&path);
                    true
                }
                None => false,
            },
            Input::Key(key) => self.key(ctx, key),
            Input::Activate(id) => self.track(|viewer| viewer.menu_action(ctx, id as u32)),
            Input::Touch { kind, x, y, .. } => {
                let (w, h) = self.size(ctx);
                self.touch(&ctx.ui, w, h, kind, x, y)
            }
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let lay = self.layout(&ctx.ui, ctx.cfg.w as i32, ctx.cfg.h as i32);
        let (bar, layout) = canvas::toolbar(&ctx.ui, &lay, self.doc.is_some(), self.tool, self.side);
        layout
            .visible
            .into_iter()
            .filter_map(|(id, bounds)| {
                let item = bar.items.iter().find(|item| item.id == id)?;
                Some(Node {
                    id: id as u64,
                    role: Role::Button,
                    label: item.label.clone(),
                    value: String::new(),
                    bounds,
                    enabled: item.enabled,
                })
            })
            .collect()
    }

    /// The answer to the open panel: the chosen file is copied into the app's data folder and opened from there.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let doc = match ev {
            DocumentEvent::Opened(doc) => doc,
            DocumentEvent::OpenedMany { docs, .. } => match docs.into_iter().next() {
                Some(doc) => doc,
                None => return false,
            },
            DocumentEvent::SaveTo(doc) => return self.write_export(ctx, doc),
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                self.status = format!("Exported {}", doc.name);
                return true;
            }
            DocumentEvent::Failed { request, code } => {
                let ours = self.pending_export.as_ref().is_some_and(|p| p.request == Some(request));
                if ours {
                    self.pending_export = None;
                    if code != DocErrorCode::Cancelled {
                        self.status = "The export could not be saved.".into();
                    }
                }
                return ours;
            }
            _ => return false,
        };
        let mut bytes = Vec::new();
        if let Err(e) = (&doc.file).read_to_end(&mut bytes) {
            self.status = format!("Could not read {}: {e}", doc.name);
            return true;
        }
        // a file chosen by Pages > Insert > Page from File is merged into the document instead of opened
        if let Some(at) = self.insert_at.take() {
            match recents::keep_copy(&self.data_dir, &doc.name, &bytes) {
                Ok(path) => self.insert_pages(pdfmarks::Insertion::File { path, at }),
                Err(e) => self.status = format!("Could not insert {}: {e}", doc.name),
            }
            return true;
        }
        match recents::keep_copy(&self.data_dir, &doc.name, &bytes) {
            Ok(path) => self.open_path(&path),
            Err(e) => self.status = format!("Could not open {}: {e}", doc.name),
        }
        true
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.shot_request != Some(request) {
            return false;
        }
        self.shot_request = None;
        if let Err(e) = screenshot::started_from_reply(result) {
            self.status = format!("Could not take a screenshot: {e}");
        }
        true
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match screenshot::captured_path(service, event, body) {
            Some(path) => {
                self.open_path(&path);
                true
            }
            None => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, _t: f64) -> bool {
        self.pump(ctx)
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.stop_editing();
        self.save();
        self.save_position();
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let lay = self.layout(&ctx.ui, s.w as i32, s.h as i32);
        let draft = self.draft(&lay);
        let rows = if self.inspector && self.mode != Mode::Slideshow { self.inspector_rows() } else { Vec::new() };
        let sliders = (self.color_panel && lay.inspector > 0 && self.mode != Mode::Slideshow).then(|| self.adjust.sliders());
        let export = (self.export_shown && lay.inspector > 0 && self.mode != Mode::Slideshow)
            .then(|| export_panel::panel(&canvas::export_metrics(&ctx.ui, &lay), &self.export_choices()));
        let signatures = (self.signatures_shown && lay.inspector > 0 && self.mode != Mode::Slideshow).then(|| self.signatures.as_slice());
        let scene = Scene {
            doc: self.doc.as_ref(),
            view: self.view,
            tool: self.tool,
            adjust: self.adjust,
            side: self.side,
            draft,
            editing: self.editing,
            form_focus: self.form_focus,
            status: &self.status,
            mode: self.mode,
            selection: self.selection_now(&lay),
            loupe: if self.loupe_on { self.loupe } else { None },
            inspector: (lay.inspector > 0).then_some(rows.as_slice()),
            contact_scroll: self.contact_scroll,
            found: &self.found,
            color: sliders.as_ref().map(|s| s.as_slice()),
            export,
            signatures,
        };
        canvas::draw(s, &ctx.ui, &lay, &scene, &mut self.cache);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(Viewer::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{pack, Image};

    #[test]
    fn open_requests_take_file_urls_and_absolute_paths_only() {
        assert_eq!(path_from_url("/tmp/a.png"), Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(path_from_url("file:///tmp/a.pdf"), Some(PathBuf::from("/tmp/a.pdf")));
        assert_eq!(path_from_url("relative/a.png"), None);
    }

    #[test]
    fn highlight_and_shapes_have_their_colours() {
        assert_eq!(color_for(MarkKind::Highlight), HIGHLIGHT);
        assert_eq!(color_for(MarkKind::Rect), MARK_BLUE);
    }

    /// A viewer with a picture open and a data folder of its own, so saving the markup leaves the working folder alone.
    fn viewer_with(img: Image) -> Viewer {
        let mut v = Viewer::new();
        v.data_dir = std::env::temp_dir().join(format!("viewer-main-{}", std::process::id()));
        v.doc = Some(document::tests_support::picture_of(img));
        v
    }

    fn pixels_of(v: &Viewer) -> Vec<u32> {
        v.doc.as_ref().and_then(|d| d.picture_rc()).map(|i| i.px.clone()).unwrap_or_default()
    }

    fn cleanup(v: &Viewer) {
        let _ = std::fs::remove_dir_all(&v.data_dir);
    }

    fn rect_mark(rect: Norm) -> Mark {
        Mark { kind: MarkKind::Rect, page: 0, rect, color: MARK_BLUE, text: String::new(), ink: Vec::new() }
    }

    /// Undoes the last edit the way Edit > Undo does.
    fn undo(v: &mut Viewer) {
        let current = v.snapshot().expect("a document");
        let previous = v.history.undo(current).expect("an edit to undo");
        v.restore(previous);
    }

    #[test]
    fn flipping_mirrors_the_picture_and_its_markup_and_can_be_undone() {
        let mut v = viewer_with(Image { w: 2, h: 2, px: vec![pack(1, 0, 0, 255), pack(2, 0, 0, 255), pack(3, 0, 0, 255), pack(4, 0, 0, 255)] });
        let original = pixels_of(&v);
        v.doc.as_mut().unwrap().marks.push(rect_mark(Norm { x: 0.0, y: 0.0, w: 0.25, h: 0.5 }));
        v.track(|v| v.flip(true));
        let flipped = pixels_of(&v);
        assert_eq!(flipped, vec![original[1], original[0], original[3], original[2]]);
        let mark = &v.doc.as_ref().unwrap().marks[0];
        assert!((mark.rect.x - 0.75).abs() < 1e-6 && (mark.rect.w - 0.25).abs() < 1e-6);
        // shown turned a quarter, Flip Horizontal mirrors the rows of the stored picture instead
        v.view.quarters = 1;
        v.track(|v| v.flip(true));
        assert_eq!(pixels_of(&v), vec![flipped[2], flipped[3], flipped[0], flipped[1]]);
        undo(&mut v);
        undo(&mut v);
        assert_eq!(pixels_of(&v), original);
        assert!((v.doc.as_ref().unwrap().marks[0].rect.x).abs() < 1e-6);
        cleanup(&v);
    }

    #[test]
    fn cropping_keeps_the_selection_moves_the_markup_and_can_be_undone() {
        let mut v = viewer_with(Image { w: 2, h: 2, px: vec![1, 2, 3, 4].into_iter().map(|n| pack(n, 0, 0, 255)).collect() });
        v.doc.as_mut().unwrap().marks.push(rect_mark(Norm { x: 0.5, y: 0.0, w: 0.5, h: 0.5 }));
        v.track(|v| v.crop());
        assert!(v.status.contains("Select an area"), "{}", v.status);
        v.selection = Some((0, Norm { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }));
        v.track(|v| v.crop());
        let doc = v.doc.as_ref().unwrap();
        assert_eq!(doc.sizes, vec![(1, 2)]);
        assert_eq!(pixels_of(&v), vec![pack(2, 0, 0, 255), pack(4, 0, 0, 255)]);
        let mark = &doc.marks[0];
        assert!((mark.rect.x).abs() < 1e-6 && (mark.rect.w - 1.0).abs() < 1e-6 && (mark.rect.h - 0.5).abs() < 1e-6);
        assert!(v.selection.is_none());
        undo(&mut v);
        assert_eq!(v.doc.as_ref().unwrap().sizes, vec![(2, 2)]);
        assert_eq!(pixels_of(&v).len(), 4);
        cleanup(&v);
    }

    #[test]
    fn removing_the_background_and_instant_alpha_clear_to_transparent_and_revert_restores() {
        let white = pack(255, 255, 255, 255);
        let black = pack(0, 0, 0, 255);
        // a white page with a black pixel in the middle of a 3 by 3 picture
        let mut px = vec![white; 9];
        px[4] = black;
        let mut v = viewer_with(Image { w: 3, h: 3, px: px.clone() });
        v.track(|v| v.remove_background());
        let cleared = pixels_of(&v);
        assert_eq!(cleared[0], 0);
        assert_eq!(cleared[4], black);
        assert!(v.status.contains("PNG"));
        undo(&mut v);
        assert_eq!(pixels_of(&v), px);
        // Instant Alpha on the black pixel clears just it
        let lay = Layout::bare(100, 100);
        let placed = v.placed(&lay).expect("a placed picture");
        v.track(|v| v.instant_alpha(&lay, placed.x as f32 + 1.5 * placed.w as f32 / 3.0, placed.y as f32 + 1.5 * placed.h as f32 / 3.0));
        assert_eq!(pixels_of(&v)[4], 0);
        assert_eq!(pixels_of(&v)[0], white);
        v.track(|v| v.revert());
        assert_eq!(pixels_of(&v), px);
        // a picture without an edge to remove says so
        v.track(|v| v.remove_background());
        v.track(|v| v.remove_background());
        assert!(v.status.contains("no background") || v.status.contains("PNG"));
        cleanup(&v);
    }

    #[test]
    fn search_results_list_the_hits_and_the_find_commands_step_through_them() {
        let mut v = Viewer::new();
        v.data_dir = std::env::temp_dir().join(format!("viewer-find-{}", std::process::id()));
        v.doc = Some(document::tests_support::pictures(4));
        v.view.page = 1;
        v.show_results("x".into(), vec![find::Hit { page: 0, snippet: "a".into() }, find::Hit { page: 3, snippet: "b".into() }]);
        assert_eq!(v.side, Side::Search);
        assert_eq!(v.view.page, 3, "the first hit on or after the shown page");
        assert!(v.status.starts_with("2 matches"));
        v.find_step(true);
        assert_eq!(v.view.page, 0, "next wraps round");
        v.find_step(false);
        assert_eq!(v.view.page, 3);
        v.show_results("zzz".into(), Vec::new());
        assert!(v.status.starts_with("Nothing found"));
        // a picture has no text to search
        v.start_find();
        assert!(v.finding.is_none() && v.status.contains("PDF"));
        cleanup(&v);
    }

    #[test]
    fn typing_a_search_edits_the_text_and_escape_ends_it() {
        let mut v = Viewer::new();
        v.finding = Some(String::new());
        assert!(v.key_find(&KeyInput::Text("ab".into())));
        assert!(v.key_find(&KeyInput::Code(8)));
        assert!(v.key_find(&KeyInput::Text("c\n".into())));
        assert_eq!(v.finding.as_deref(), Some("ac"));
        assert_eq!(v.status, "Find: ac|");
        assert!(v.key_find(&KeyInput::Code(27)));
        assert!(v.finding.is_none() && v.status.is_empty());
        assert!(!v.key_find(&KeyInput::Code(65)));
    }

    #[test]
    fn adjust_size_resamples_the_picture_and_can_be_undone() {
        let mut v = viewer_with(Image { w: 4, h: 2, px: vec![pack(9, 9, 9, 255); 8] });
        v.start_resize();
        assert!(v.resizing.is_some() && v.status.contains("4 x 2"), "{}", v.status);
        assert!(v.key_resize(&KeyInput::Text("8 junk".into())));
        assert_eq!(v.resizing.as_deref(), Some("8 "), "only digits, x, percent, dot and space are taken");
        assert!(v.key_resize(&KeyInput::Code(13)));
        assert!(v.resizing.is_none());
        let doc = v.doc.as_ref().unwrap();
        assert_eq!(doc.sizes, vec![(8, 4)]);
        assert_eq!((doc.picture_rc().unwrap().w, doc.picture_rc().unwrap().h), (8, 4));
        assert!(v.status.contains("8 x 4"));
        undo(&mut v);
        assert_eq!(v.doc.as_ref().unwrap().sizes, vec![(4, 2)]);
        // text that is not a size changes nothing and says so
        v.start_resize();
        v.key_resize(&KeyInput::Text("0".into()));
        v.key_resize(&KeyInput::Code(13));
        assert!(v.status.contains("not a size"), "{}", v.status);
        assert_eq!(v.doc.as_ref().unwrap().sizes, vec![(4, 2)]);
        cleanup(&v);
    }

    #[test]
    fn a_pdf_is_not_resampled() {
        let mut v = Viewer::new();
        v.start_resize();
        assert!(v.resizing.is_none() && v.status.contains("pictures"));
    }

    #[test]
    fn the_slideshow_hides_everything_steps_through_pages_and_restores_the_zoom() {
        let mut v = Viewer::new();
        v.doc = Some(document::tests_support::pictures(3));
        v.view.zoom = Zoom::Factor(2.0);
        v.toggle_slideshow();
        assert_eq!(v.mode, Mode::Slideshow);
        assert_eq!(v.view.zoom, Zoom::Fit);
        assert!(v.key_slideshow(&KeyInput::Code(39)));
        assert!(v.key_slideshow(&KeyInput::Text(" ".into())));
        assert!(v.key_slideshow(&KeyInput::Code(39)));
        assert_eq!(v.view.page, 2, "it stops at the last page");
        assert!(v.key_slideshow(&KeyInput::Code(37)));
        assert_eq!(v.view.page, 1);
        assert!(!v.key_slideshow(&KeyInput::Code(65)));
        assert!(v.key_slideshow(&KeyInput::Code(27)));
        assert_eq!(v.mode, Mode::Page);
        assert_eq!(v.view.zoom, Zoom::Factor(2.0));
        // with no document there is nothing to show
        let mut empty = Viewer::new();
        empty.toggle_slideshow();
        assert_eq!(empty.mode, Mode::Page);
    }

    #[test]
    fn the_inspector_lists_the_open_files_facts() {
        let mut v = Viewer::new();
        assert!(v.inspector_rows().is_empty());
        v.doc = Some(document::tests_support::pictures(1));
        v.facts = inspector::Facts { name: "p.jpg".into(), kind: "JPEG image", bytes: 10, exif: vec![("Location", "1.0000 N, 2.0000 E".into())] };
        let rows = v.inspector_rows();
        assert!(rows.contains(&("Name".to_string(), "p.jpg".to_string())));
        assert!(rows.contains(&("Dimensions".to_string(), "4 x 4 pixels".to_string())));
        v.show_location();
        assert!(v.inspector && v.status.contains("1.0000 N"));
        v.facts.exif.clear();
        v.show_location();
        assert!(v.status.contains("no location"));
    }

    #[test]
    fn strike_through_is_red_and_the_other_new_marks_blue() {
        assert_eq!(color_for(MarkKind::Strike), MARK_RED);
        assert_eq!(color_for(MarkKind::Underline), MARK_BLUE);
        assert_eq!(color_for(MarkKind::Arrow), MARK_BLUE);
    }

    #[test]
    fn a_saved_signature_keeps_every_stroke_and_goes_back_in_as_strokes() {
        let mut v = viewer_with(Image { w: 40, h: 40, px: vec![0xff00_0000; 1600] });
        v.data_dir = std::env::temp_dir().join(format!("viewer-sign-{}", std::process::id()));
        if let Some(doc) = &mut v.doc {
            doc.marks.push(stroke(0, vec![(0.1, 0.1), (0.3, 0.2)]));
            doc.marks.push(stroke(0, vec![(0.2, 0.5), (0.4, 0.6), (0.6, 0.5)]));
        }
        v.save_signature();
        assert_eq!(v.signatures.len(), 1);
        assert_eq!(v.signatures[0].strokes.len(), 2, "both strokes are kept apart");
        v.track(|v| v.insert_signature(0));
        let marks = &v.doc.as_ref().unwrap().marks;
        assert_eq!(marks.len(), 4, "the two drawn strokes and the two inserted ones");
        assert!(marks[2..].iter().all(|m| m.kind == MarkKind::Ink && m.page == 0));
        assert_eq!(marks[3].ink.len(), 3);
        undo(&mut v);
        assert_eq!(v.doc.as_ref().unwrap().marks.len(), 2, "insert is one edit");
        cleanup(&v);
    }

    #[test]
    fn the_author_is_typed_into_the_status_line_and_kept_on_return() {
        let mut v = Viewer::new();
        v.data_dir = std::env::temp_dir().join(format!("viewer-author-{}", std::process::id()));
        v.authoring = Some(String::new());
        assert!(v.key_author(&KeyInput::Text("Ana".into())));
        assert!(v.key_author(&KeyInput::Code(8)));
        assert!(v.key_author(&KeyInput::Text("a Lind".into())));
        assert_eq!(v.status, "Annotation author: Ana Lind|");
        assert!(v.key_author(&KeyInput::Code(13)));
        assert!(v.authoring.is_none());
        assert_eq!(v.author(), "Ana Lind");
        let _ = std::fs::remove_dir_all(&v.data_dir);
    }

    #[test]
    fn a_document_reopens_on_the_page_it_was_last_shown_on_and_the_contact_sheet_can_be_the_default() {
        let mut v = Viewer::new();
        v.doc = Some(document::tests_support::pictures(3));
        v.restore_page = Some(2);
        v.restore_view();
        assert_eq!(v.view.page, 2);
        assert!(v.restore_page.is_none());
        v.prefs.contact_sheet = true;
        v.restore_view();
        assert_eq!(v.mode, Mode::Contact);
    }

    #[test]
    fn export_as_holds_the_picture_for_the_save_panel_instead_of_writing_beside_the_document() {
        let mut v = viewer_with(Image { w: 2, h: 2, px: vec![0xff00_00ff; 4] });
        v.export_raster(Raster::Jpeg(export::JPEG_QUALITY));
        // the save panel names the file, so nothing is written beside the document (/a/b-markup.jpg)
        assert!(!Path::new("/a/b-markup.jpg").exists());
        let pending = v.pending_export.as_ref().expect("the export waits for the panel");
        assert_eq!((pending.name.as_str(), pending.ext, pending.request), ("b-markup.jpg", "jpg", None));
        assert!(matches!(pending.what, Export::Picture(ref bytes) if bytes.starts_with(&[0xff, 0xd8])));
        cleanup(&v);
    }

    #[test]
    fn a_stroke_is_boxed_by_its_points() {
        let m = stroke(3, vec![(0.25, 0.5), (0.75, 0.25)]);
        assert_eq!(m.kind, MarkKind::Ink);
        assert_eq!(m.page, 3);
        assert_eq!(m.rect, Norm { x: 0.25, y: 0.25, w: 0.5, h: 0.25 });
        assert_eq!(m.ink.len(), 2);
    }
}
