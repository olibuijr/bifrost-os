//! Viewer, a document window for images and PDFs: zoom and fit, rotation and mirroring, page navigation with a
//! thumbnails sidebar, outline, search results, notes and bookmarks, a contact sheet and a slideshow, page moves and
//! deletes, markup (highlight, note, text box, shapes, arrows, underline, strike through, redaction, freehand
//! signature with a saved signature library), Find, Copy, rectangular selection with Crop and Zoom to Selection,
//! Adjust Color with sliders, Adjust Size, Instant Alpha and Remove Background for pictures, the Inspector with the
//! GPS position, a magnifier, Save in place with a backup, and PNG, JPEG and PDF export. A PDF is opened and drawn by
//! its worker thread (`pdfjob`); pages and markup live in `document` and `canvas`, the pure state and maths in
//! `model`, `picture` and `find`, settings in `prefs`, and the file writing in `export` and `pdfmarks`.

mod canvas;
mod document;
mod exif;
mod export;
mod find;
mod model;
mod pdfjob;
mod pdfmarks;
mod picture;
mod prefs;
mod printjob;
mod recents;

use bifrost_app::kit::{clipboard, Damage, Surface, Ui};
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use canvas::{Action, Layout, PictureCache, Scene, Side, Tool};
use document::Document;
use export::Raster;
use find::Hit;
use model::{
    Adjust, History, InspectorInfo, Mark, MarkKind, Norm, Snapshot, Trail, View, Zoom, ADJUST_ROWS, FLIP_H, FLIP_V, HIGHLIGHT, MARK_BLUE, REDACT,
};
use pdfjob::{Inbox, Job, Reply};
use prefs::Prefs;
use recents::Recents;
use springboard::shell::books::engine;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// File > Open and the Open Recent items. The ids sit above every `Action` id.
const MENU_OPEN: u32 = 0x5001;
const MENU_CLEAR_RECENTS: u32 = 0x5002;
const MENU_RECENT: u32 = 0x5010;
/// Tools > Signatures: one item per saved signature.
const MENU_SIGNATURE: u32 = 0x5200;
/// The kinds of file the open panel offers.
const OPEN_TYPES: [&str; 4] = ["pdf", "png", "jpg", "jpeg"];
/// The colour distance Instant Alpha clears around the clicked colour, and Remove Background clears from the corners.
const MASK_TOLERANCE: i32 = 32;
const BACKGROUND_TOLERANCE: i32 = 24;
/// A new text box's size, as fractions of the page.
const TEXT_BOX: (f32, f32) = (0.3, 0.05);
/// A placed signature's width as a fraction of the page, and where its centre goes.
const SIGNATURE_WIDTH: f32 = 0.3;
const SIGNATURE_CENTRE: (f32, f32) = (0.5, 0.8);

/// A pointer gesture in progress. The coordinates are canvas or sidebar pixels.
enum Gesture {
    /// Dragging the picture or the text.
    Pan { last: (f32, f32) },
    /// Dragging the sidebar, or the contact sheet, up or down.
    Scroll { last: f32 },
    /// Dragging out a highlight or a shape.
    Drag { start: (f32, f32), now: (f32, f32), kind: MarkKind },
    /// Drawing a stroke: its points so far.
    Ink { points: Vec<(f32, f32)> },
    /// Dragging out the rectangular selection.
    Select { start: (f32, f32), now: (f32, f32) },
    /// Dragging a slider of the Adjust Color panel: its row.
    Slider { row: usize },
}

/// What the status line is asking for while the keyboard types into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptKind {
    GoTo,
    Find,
    Size,
    Author,
}

impl PromptKind {
    fn label(self) -> &'static str {
        match self {
            PromptKind::GoTo => "Go to page",
            PromptKind::Find => "Find",
            PromptKind::Size => "Size (width, xheight or widthxheight)",
            PromptKind::Author => "Annotation author",
        }
    }

    /// Whether the character may be typed for this prompt.
    fn accepts(self, c: char) -> bool {
        match self {
            PromptKind::GoTo => c.is_ascii_digit(),
            PromptKind::Size => c.is_ascii_digit() || matches!(c, 'x' | 'X' | ' '),
            PromptKind::Find | PromptKind::Author => !c.is_control(),
        }
    }
}

/// A line being typed in the status line.
struct Prompt {
    kind: PromptKind,
    text: String,
}

/// The state of Find: the query, its hits and the hit the view is on.
#[derive(Default)]
struct FindState {
    query: String,
    hits: Vec<Hit>,
    current: Option<usize>,
}

/// A PDF on its way from its worker: the worker's job sender and the inbox its replies come back in.
struct Loading {
    path: PathBuf,
    jobs: Sender<Job>,
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
    /// The line being typed in the status line, if any.
    prompt: Option<Prompt>,
    /// Whether the sidebar is shown.
    sidebar: bool,
    /// Whether the toolbar is shown.
    toolbar: bool,
    /// The files opened lately, for File > Open Recent.
    recents: Recents,
    /// The settings.
    prefs: Prefs,
    /// The search in progress or last made.
    find: FindState,
    /// The rectangular selection: its source page and rectangle.
    selection: Option<(usize, Norm)>,
    /// Whether every page is shown as a thumbnail grid.
    contact: bool,
    /// Whether the page is shown alone on black.
    slideshow: bool,
    /// Whether the Adjust Color panel is shown.
    adjust_panel: bool,
    /// Whether the Inspector is shown.
    inspector: bool,
    /// The GPS position text, once Show Location Info asked for it.
    location: Option<String>,
    /// The size of the open file in bytes.
    file_bytes: Option<u64>,
    /// Whether the magnifier is on, and the last pointer position it follows.
    magnifier: bool,
    pointer: (f32, f32),
    /// The signature library.
    signatures: Vec<Mark>,
    /// Text a PDF worker read for Copy, waiting for the pasteboard.
    copied: Option<String>,
    /// Whether a save in place is running on the worker, and what to add to its message.
    saving: bool,
    save_note: &'static str,
    /// The page last written to the last-page file.
    saved_page: Option<usize>,
    /// A message to show once the PDF being opened has arrived (the result of a save).
    after_open: String,
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
            prompt: None,
            sidebar: true,
            toolbar: true,
            recents: Recents::default(),
            prefs: Prefs::default(),
            find: FindState::default(),
            selection: None,
            contact: false,
            slideshow: false,
            adjust_panel: false,
            inspector: false,
            location: None,
            file_bytes: None,
            magnifier: false,
            pointer: (0.0, 0.0),
            signatures: Vec::new(),
            copied: None,
            saving: false,
            save_note: "",
            saved_page: None,
            after_open: String::new(),
        }
    }

    /// The window layout: the sidebar takes no room while it is hidden, in the contact sheet or the slideshow; the
    /// toolbar takes none while it is hidden or in the slideshow.
    fn layout(&self, ui: &Ui, w: i32, h: i32) -> Layout {
        let mut lay = Layout::new(ui, w, h);
        if !self.sidebar || self.contact || self.slideshow {
            lay.side = 0;
        }
        if !self.toolbar || self.slideshow {
            lay.bar = 0;
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
            flips: doc.flips.clone(),
            adjust: self.adjust,
            pic: doc.pic_state(),
        })
    }

    /// Puts a state from the undo history back.
    fn restore(&mut self, state: Snapshot) {
        self.editing = None;
        if let Some(doc) = &mut self.doc {
            doc.marks = state.marks;
            doc.order = state.order;
            doc.turns = state.turns;
            doc.flips = state.flips;
            doc.restore_pic(&state.pic);
        }
        self.adjust = state.adjust;
        self.cache = None;
        self.selection = None;
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
        self.stop_editing();
        self.doc = None;
        self.loading = None;
        self.cache = None;
        self.view = View::default();
        self.side = Side::Pages;
        self.history = History::default();
        self.trail = Trail::default();
        self.prompt = None;
        self.selection = None;
        self.contact = false;
        self.slideshow = false;
        self.find = FindState::default();
        self.location = None;
        self.saved_page = None;
        self.after_open.clear();
        self.file_bytes = std::fs::metadata(path).ok().map(|m| m.len());
        self.sidebar = self.prefs.sidebar_on_open;
        self.status.clear();
        self.remember(path);
        if matches!(engine::format_of(path), Some(engine::Format::Pdf)) {
            let (jobs, inbox) = pdfjob::spawn(path.to_path_buf());
            self.loading = Some(Loading { path: path.to_path_buf(), jobs, inbox });
            self.status = format!("Opening {}", name_of(path));
            return;
        }
        match document::open_picture(path, &self.data_dir) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.arrived();
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    /// Applies the settings that depend on the document just opened: the actual size for pictures and the page it was
    /// last left on.
    fn arrived(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (is_pdf, count, path) = (doc.is_pdf, doc.count(), doc.path.clone());
        if !is_pdf && self.prefs.actual_size {
            self.view.zoom = Zoom::Factor(1.0);
        }
        if self.prefs.remember_page {
            if let Some(page) = document::load_last_page(&self.data_dir, &path) {
                self.view.go(page, count);
            }
        }
        self.saved_page = Some(self.view.page);
        self.sync_turn();
    }

    /// Puts `path` first in Open Recent and saves the list.
    fn remember(&mut self, path: &Path) {
        if self.data_dir.as_os_str().is_empty() || !path.exists() {
            return;
        }
        self.recents.add(path);
        let _ = recents::save(&self.data_dir, &self.recents);
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
        match id {
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

    /// Takes what the PDF worker has sent: the open result, drawn pages and the results of saves, searches and
    /// exports. True when the window needs a new frame.
    fn pump(&mut self) -> bool {
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
                    let Loading { path, jobs, inbox } = loading;
                    self.doc = Some(document::from_pdf(&path, &self.data_dir, info, jobs, inbox));
                    self.status = std::mem::take(&mut self.after_open);
                    self.arrived();
                }
                Some(Err(e)) => {
                    changed = true;
                    self.status = format!("Could not open {}: {e}", loading.path.display());
                }
            }
        }
        if let Some(doc) = &self.doc {
            let (arrived, replies) = doc.take_replies();
            changed |= arrived;
            for reply in replies {
                match reply {
                    Reply::Written(result) if self.saving => {
                        self.saving = false;
                        self.finish_save(result);
                    }
                    Reply::Written(result) => {
                        self.status = match result {
                            Ok(path) if path.extension().is_some_and(|e| e == "pdf") => format!("Saved {}", path.display()),
                            Ok(path) => format!("Exported {}", path.display()),
                            Err(e) => format!("Save failed: {e}"),
                        };
                    }
                    Reply::Printed(status) => self.status = status,
                    Reply::Found(query, hits) => self.find_done(query, hits),
                    Reply::Text(text) => self.copied = Some(text),
                    _ => {}
                }
            }
        }
        changed
    }

    fn save(&self) {
        if let Some(doc) = &self.doc {
            let _ = document::save_marks(&self.data_dir, &doc.path, &doc.marks);
            let _ = document::save_bookmarks(&self.data_dir, &doc.path, &doc.bookmarks);
        }
    }

    /// Writes the shown page to the last-page file when it changed since the last write.
    fn save_page(&mut self) {
        let Some(doc) = &self.doc else { return };
        if self.saved_page == Some(self.view.page) || self.data_dir.as_os_str().is_empty() {
            return;
        }
        let _ = document::save_last_page(&self.data_dir, &doc.path, self.view.page);
        self.saved_page = Some(self.view.page);
    }

    /// The source page the window is showing, or None with no document.
    fn src(&self) -> Option<usize> {
        self.doc.as_ref()?.source_of(self.view.page)
    }

    /// Makes the view's orientation match the shown page's saved turns and mirroring, after the page changes.
    fn sync_turn(&mut self) {
        if let Some(doc) = &self.doc {
            self.view.quarters = doc.orient_of(self.view.page);
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
                Some(drag_mark(*kind, src, a, b))
            }
            Gesture::Ink { points } => Some(stroke(src, points.iter().map(|&(x, y)| canvas::to_source(&p, q, x, y)).collect())),
            _ => None,
        }
    }

    /// The selection to draw: the one being dragged out, or the stored one when it is on the shown page.
    fn selection_now(&self, lay: &Layout) -> Option<Norm> {
        if let Some(Gesture::Select { start, now }) = &self.gesture {
            let p = self.placed(lay)?;
            let q = self.view.quarters;
            return Some(Norm::from_corners(canvas::to_source(&p, q, start.0, start.1), canvas::to_source(&p, q, now.0, now.1)));
        }
        let (src, rect) = self.selection?;
        (Some(src) == self.src()).then_some(rect)
    }

    /// The size the zoom steps work from: the turned picture, or the canvas with no document.
    fn page_dims(&self, lay: &Layout) -> (usize, usize) {
        let (_, _, cw, ch) = lay.canvas();
        let picture = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)).map(|(w, h)| model::turn_size(w, h, self.view.quarters));
        picture.unwrap_or((cw as usize, ch as usize))
    }

    /// The rows of the sidebar list, whichever it shows.
    fn side_rows(&self, ui: &Ui) -> Vec<(i32, i32)> {
        match (&self.doc, self.side) {
            (Some(doc), Side::Pages) => canvas::rows(ui, doc),
            (Some(doc), Side::Outline) => canvas::outline_rows(ui, doc),
            (Some(doc), side) => canvas::list_rows(ui, canvas::list_entries(doc, side, &self.find.hits).len()),
            (None, _) => Vec::new(),
        }
    }

    /// How far the sidebar, or the contact sheet, can scroll.
    fn scroll_max(&self, ui: &Ui, lay: &Layout) -> f32 {
        if self.contact {
            let count = self.doc.as_ref().map_or(0, |d| d.count());
            let (_, _, _, ch) = lay.canvas();
            return (canvas::contact_height(&canvas::contact_layout(ui, lay, count)) - ch).max(0) as f32;
        }
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

    /// Starts typing into the status line.
    fn start_prompt(&mut self, kind: PromptKind) {
        if self.doc.is_none() && kind != PromptKind::Author {
            return;
        }
        self.stop_editing();
        let text = match kind {
            PromptKind::Find => self.find.query.clone(),
            PromptKind::Author => self.prefs.author.clone(),
            _ => String::new(),
        };
        self.prompt = Some(Prompt { kind, text });
        self.show_prompt();
    }

    /// Shows what has been typed in the status line.
    fn show_prompt(&mut self) {
        if let Some(p) = &self.prompt {
            self.status = format!("{}: {}|", p.kind.label(), p.text);
        }
    }

    /// A key while the status line is being typed into. Return does what the prompt asked, Escape drops it.
    fn key_prompt(&mut self, ctx: &Ctx, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(t) => {
                if let Some(p) = &mut self.prompt {
                    let kind = p.kind;
                    p.text.extend(t.chars().filter(|c| kind.accepts(*c)));
                }
            }
            KeyInput::Code(8) => {
                if let Some(p) = &mut self.prompt {
                    p.text.pop();
                }
            }
            KeyInput::Code(13) => {
                let Some(prompt) = self.prompt.take() else { return true };
                self.status.clear();
                self.submit(ctx, prompt);
                return true;
            }
            KeyInput::Code(27) => {
                self.prompt = None;
                self.status.clear();
                return true;
            }
            _ => return false,
        }
        self.show_prompt();
        true
    }

    /// Does what a finished prompt asked.
    fn submit(&mut self, ctx: &Ctx, prompt: Prompt) {
        let text = prompt.text.trim().to_string();
        match prompt.kind {
            PromptKind::GoTo => {
                let count = self.doc.as_ref().map_or(0, |d| d.count());
                match model::parse_page(&text, count) {
                    Some(page) => self.go_page(page),
                    None if !text.is_empty() => self.status = format!("There is no page {text}."),
                    None => {}
                }
            }
            PromptKind::Find => {
                if !text.is_empty() {
                    self.start_find(text);
                }
            }
            PromptKind::Size => {
                let size = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)).and_then(|(w, h)| model::parse_size(&text, w, h));
                match size {
                    Some((w, h)) => {
                        self.track(|v| v.resize_picture(w, h));
                    }
                    None if !text.is_empty() => self.status = format!("{text} is not a size."),
                    None => {}
                }
            }
            PromptKind::Author => {
                self.prefs.author = text;
                self.prefs.save(&mut ctx.defaults_mut());
                self.status = if self.prefs.author.is_empty() { "Annotations have no author.".into() } else { format!("Annotations are by {}.", self.prefs.author) };
            }
        }
    }

    /// Searches every page of a PDF for `query`; the hits come back as a reply.
    fn start_find(&mut self, query: String) {
        let Some(doc) = &self.doc else { return };
        if !doc.is_pdf {
            self.status = "Find works in PDF documents.".into();
            return;
        }
        let sent = doc.send(find::search_job(query.clone()));
        self.find = FindState { query: query.clone(), hits: Vec::new(), current: None };
        self.status = if sent { format!("Searching for {query}") } else { "The document is not open.".into() };
        self.side = Side::Search;
        self.sidebar = true;
        self.view.sidebar_scroll = 0.0;
    }

    /// The search finished: keep the hits and go to the first one on or after the shown page.
    fn find_done(&mut self, query: String, hits: Vec<Hit>) {
        if query != self.find.query {
            return;
        }
        self.status = match hits.len() {
            0 => format!("No results for {query}"),
            1 => format!("1 result for {query}"),
            n => format!("{n} results for {query}"),
        };
        let places: Vec<Option<usize>> = hits.iter().map(|h| self.doc.as_ref().and_then(|d| d.display_of(h.page))).collect();
        self.find.hits = hits;
        if let Some(i) = find::first_from(&places, self.view.page) {
            self.find.current = Some(i);
            if let Some(page) = places[i] {
                self.go_page(page);
            }
        }
    }

    /// Find Next and Find Previous: the next hit on a page that is still shown.
    fn find_step(&mut self, forward: bool) {
        if self.find.hits.is_empty() {
            if self.find.query.is_empty() {
                self.start_prompt(PromptKind::Find);
            } else {
                self.status = format!("No results for {}", self.find.query);
            }
            return;
        }
        let count = self.find.hits.len();
        let mut current = self.find.current;
        for _ in 0..count {
            current = find::step(count, current, forward);
            let page = current.and_then(|i| self.doc.as_ref().and_then(|d| d.display_of(self.find.hits[i].page)));
            if let Some(page) = page {
                self.find.current = current;
                self.go_page(page);
                self.status = format!("Result {} of {}", current.map_or(0, |i| i + 1), count);
                return;
            }
        }
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

    /// Turns the shown page by `delta` quarter turns, and keeps the turn with the page. A mirrored page turns the other
    /// way round in the saved turns, because the turn is applied before the mirroring.
    fn rotate(&mut self, delta: i32) {
        let page = self.view.page;
        let Some(doc) = &self.doc else { return };
        let orient = doc.orient_of(page);
        let mirrored = (orient & FLIP_H != 0) != (orient & FLIP_V != 0);
        self.view.turn(if mirrored { -delta } else { delta });
        let quarters = self.view.quarters & 3;
        if let Some(doc) = &mut self.doc {
            doc.set_turn(page, quarters);
        }
        self.sync_turn();
    }

    /// Mirrors the shown page.
    fn flip(&mut self, horizontal: bool) {
        let page = self.view.page;
        if let Some(doc) = &mut self.doc {
            doc.toggle_flip(page, horizontal);
        }
        self.sync_turn();
    }

    /// The editable picture of the shown page: its pixels and the source page, for a picture document.
    fn picture_page(&self) -> Option<(usize, std::rc::Rc<bifrost_app::kit::Image>)> {
        let doc = self.doc.as_ref()?;
        if doc.is_pdf {
            return None;
        }
        let page = self.view.page;
        Some((doc.source_of(page)?, doc.picture(page)?))
    }

    /// Crop: the picture keeps only the selection. Marks move with it.
    fn crop_selection(&mut self) {
        let Some((src, rect)) = self.selection else {
            self.status = "Choose Rectangular Selection and drag out the part to keep.".into();
            return;
        };
        let Some((page_src, img)) = self.picture_page() else {
            self.status = "Crop works on pictures.".into();
            return;
        };
        if src != page_src {
            return;
        }
        let Some(cropped) = picture::crop(&img, rect) else { return };
        if let Some(doc) = &mut self.doc {
            doc.marks = model::crop_marks(&doc.marks, rect);
            doc.replace_picture(cropped);
        }
        self.selection = None;
        self.cache = None;
        self.view.zoom = Zoom::Fit;
        self.view.pan = (0.0, 0.0);
        self.save();
    }

    /// Adjust Size: the picture is resampled to `w` by `h` pixels.
    fn resize_picture(&mut self, w: usize, h: usize) {
        let Some((_, img)) = self.picture_page() else {
            self.status = "Adjust Size works on pictures.".into();
            return;
        };
        let resized = img.resized(w, h);
        if let Some(doc) = &mut self.doc {
            doc.replace_picture(resized);
        }
        self.cache = None;
        self.status = format!("Resized to {w} \u{d7} {h} pixels");
    }

    /// Remove Background: the regions that touch the corners and match their colour become transparent.
    fn remove_background(&mut self) {
        let Some((_, img)) = self.picture_page() else {
            self.status = "Remove Background works on pictures.".into();
            return;
        };
        let mut copy = (*img).clone();
        let cleared = picture::remove_background(&mut copy, BACKGROUND_TOLERANCE);
        if cleared == 0 {
            self.status = "No background was found at the corners.".into();
            return;
        }
        if let Some(doc) = &mut self.doc {
            doc.replace_picture(copy);
        }
        self.cache = None;
        self.status = "Background removed. Save as PNG to keep the transparency.".into();
    }

    /// Instant Alpha: the region of one colour around the click becomes transparent.
    fn mask_at(&mut self, lay: &Layout, x: f32, y: f32) {
        let Some((_, img)) = self.picture_page() else {
            self.status = "Instant Alpha works on pictures.".into();
            return;
        };
        let Some(p) = self.placed(lay) else { return };
        let (u, v) = canvas::to_source(&p, self.view.quarters, x, y);
        let seed = (((u * img.w as f32) as usize).min(img.w - 1), ((v * img.h as f32) as usize).min(img.h - 1));
        let mut copy = (*img).clone();
        if picture::clear_region(&mut copy, seed, MASK_TOLERANCE) == 0 {
            return;
        }
        if let Some(doc) = &mut self.doc {
            doc.replace_picture(copy);
        }
        self.cache = None;
    }

    /// Edit > Select All: the whole shown page is the selection.
    fn select_all(&mut self) {
        if let Some(src) = self.src() {
            self.selection = Some((src, Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }));
        }
    }

    /// View > Zoom to Selection.
    fn zoom_to_selection(&mut self, lay: &Layout) {
        let Some((src, rect)) = self.selection else {
            self.status = "Choose Rectangular Selection and drag out an area first.".into();
            return;
        };
        if Some(src) != self.src() {
            return;
        }
        let Some((w, h)) = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)) else { return };
        let q = self.view.quarters;
        let (rw, rh) = model::turn_size(w, h, q);
        let (_, _, cw, ch) = lay.canvas();
        let (scale, pan) = model::zoom_to_rect(rect, q, rw, rh, cw as usize, ch as usize);
        self.view.zoom = Zoom::Factor(scale);
        self.view.pan = pan;
    }

    /// Edit > Copy: a picture's selection (or the whole picture) goes to the pasteboard as an image; a PDF page's text
    /// is read by the worker and copied when it arrives.
    fn copy(&mut self) {
        let Some(doc) = &self.doc else { return };
        if doc.is_pdf {
            let Some(src) = self.src() else { return };
            self.status = if doc.send(find::text_job(vec![src])) { "Copying the page text".into() } else { "The document is not open.".into() };
            return;
        }
        let Some((src, img)) = self.picture_page() else { return };
        let part = match self.selection {
            Some((s, rect)) if s == src => picture::crop(&img, rect).unwrap_or_else(|| (*img).clone()),
            _ => (*img).clone(),
        };
        let done = match clipboard::global().as_mut() {
            Some(c) => {
                c.set_image(part);
                true
            }
            None => false,
        };
        self.status = if done { "Copied the picture".into() } else { "The pasteboard is not available.".into() };
    }

    /// File > New from Clipboard: the picture on the pasteboard is written into the data folder and opened.
    fn new_from_clipboard(&mut self) {
        let image = clipboard::global().as_ref().and_then(|c| c.image().cloned());
        let Some(img) = image else {
            self.status = "The pasteboard has no picture.".into();
            return;
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let dir = self.data_dir.join("clipboard");
        let path = dir.join(format!("clipboard-{stamp}.png"));
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, springboard::pngenc::encode_rgb(img.w, img.h, &img.px)));
        match written {
            Ok(()) => self.open_path(&path),
            Err(e) => self.status = format!("Could not keep the picture: {e}"),
        }
    }

    /// File > Save: the edits are written into the file, after the original is copied to the backups folder. A PDF is
    /// written by its worker; a picture is written here. Either way the saved file is opened again, with no edits left.
    fn save_in_place(&mut self) {
        let Some(doc) = &self.doc else { return };
        let path = doc.path.clone();
        if let Err(e) = export::back_up(&self.data_dir, &path) {
            self.status = format!("Save failed: {e}");
            return;
        }
        if doc.is_pdf {
            self.save_note = if doc.marks.iter().any(|m| m.kind == MarkKind::Redact) {
                " Redactions cover the page only; the text under them stays in the file."
            } else {
                ""
            };
            let job = pdfmarks::save_in_place_job(path, doc.order.clone(), doc.turns.clone(), doc.marks.clone(), self.prefs.author.clone());
            self.saving = doc.send(job);
            self.status = if self.saving { "Saving".into() } else { "The document is not open.".into() };
            return;
        }
        let page = self.view.page;
        let outcome = (|| -> Result<PathBuf, String> {
            let src = doc.source_of(page).ok_or("There is no page to save.")?;
            let img = doc.picture(page).ok_or("The picture is not ready.")?;
            let out = export::finish(&img, &doc.marks, src, doc.orient_of(page), &self.adjust, 1.0);
            export::write_image(&out, export::raster_for(&path), &path, self.prefs.jpeg_quality)?;
            Ok(path.clone())
        })();
        self.finish_save(outcome);
    }

    /// The saved file is in place: the edits are in it, so they are dropped, and the file is opened again.
    fn finish_save(&mut self, result: Result<PathBuf, String>) {
        let path = match result {
            Ok(path) => path,
            Err(e) => {
                self.status = format!("Save failed: {e}");
                return;
            }
        };
        let mut places = self.doc.as_ref().map(|d| d.bookmark_places()).unwrap_or_default();
        places.sort_unstable();
        let page = self.view.page;
        let _ = document::save_marks(&self.data_dir, &path, &[]);
        let _ = document::save_bookmarks(&self.data_dir, &path, &places);
        let _ = document::save_last_page(&self.data_dir, &path, page);
        self.adjust = Adjust::default();
        let note = self.save_note;
        self.save_note = "";
        self.open_path(&path);
        let message = format!("Saved {}.{note}", name_of(&path));
        if self.loading.is_some() {
            self.after_open = message;
        } else {
            self.status = message;
        }
    }

    /// The lines the Inspector shows, or None while it is hidden.
    fn inspector_lines(&self) -> Option<Vec<(String, String)>> {
        if !self.inspector {
            return None;
        }
        let doc = self.doc.as_ref()?;
        let info = InspectorInfo {
            name: name_of(&doc.path),
            kind: if doc.is_pdf { "PDF document" } else { picture_kind(&doc.path) },
            bytes: self.file_bytes,
            pixels: if doc.is_pdf { None } else { doc.page_size(self.view.page) },
            pages: doc.count(),
            pdf_version: doc.pdf_info.as_ref().map(|i| i.version.clone()),
            encrypted: doc.pdf_info.as_ref().map(|i| i.encrypted),
            marks: doc.marks.len(),
            bookmarks: doc.bookmarks.len(),
            location: self.location.clone(),
        };
        Some(model::inspector_lines(&info))
    }

    /// Show Location Info: reads the picture's GPS position and shows it in the Inspector.
    fn show_location(&mut self) {
        self.inspector = true;
        let Some(doc) = &self.doc else { return };
        self.location = Some(if doc.is_pdf {
            "Not available for PDF documents".into()
        } else {
            match std::fs::read(&doc.path).ok().and_then(|bytes| exif::gps(&bytes)) {
                Some(position) => position.label(),
                None => "No location data".into(),
            }
        });
    }

    /// Places saved signature `index` on the shown page.
    fn use_signature(&mut self, index: usize) {
        let Some(entry) = self.signatures.get(index).cloned() else { return };
        let Some(src) = self.src() else { return };
        let Some((w, h)) = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)) else { return };
        let mark = prefs::place_signature(&entry, src, SIGNATURE_CENTRE, SIGNATURE_WIDTH, w as f32 / h.max(1) as f32);
        if let Some(doc) = &mut self.doc {
            doc.marks.push(mark);
        }
        self.save();
    }

    /// Keeps a stroke just signed in the signature library.
    fn keep_signature(&mut self, ink: &[(f32, f32)]) {
        let Some((w, h)) = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)) else { return };
        let Some(entry) = prefs::library_entry(ink, w as f32 / h.max(1) as f32) else { return };
        prefs::add_signature(&mut self.signatures, entry);
        let _ = prefs::save_signatures(&self.data_dir, &self.signatures);
    }

    /// The setting actions, which need the app's defaults. False when `action` is not one of them.
    fn pref_action(&mut self, ctx: &Ctx, action: Action) -> bool {
        match action {
            Action::PrefRememberPage => self.prefs.remember_page = !self.prefs.remember_page,
            Action::PrefActualSize => self.prefs.actual_size = !self.prefs.actual_size,
            Action::PrefSidebarOnOpen => self.prefs.sidebar_on_open = !self.prefs.sidebar_on_open,
            Action::QualityHigh | Action::QualityMedium | Action::QualityLow => {
                let index = match action {
                    Action::QualityHigh => 0,
                    Action::QualityMedium => 1,
                    _ => 2,
                };
                self.prefs.jpeg_quality = prefs::QUALITIES[index].1;
            }
            Action::SetAuthor => {
                self.start_prompt(PromptKind::Author);
                return true;
            }
            _ => return false,
        }
        self.prefs.save(&mut ctx.defaults_mut());
        true
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
                self.start_prompt(PromptKind::GoTo);
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
            Action::ToggleToolbar => {
                self.toolbar = !self.toolbar;
                true
            }
            Action::Save => {
                self.stop_editing();
                self.save_in_place();
                true
            }
            Action::NewFromClipboard => {
                self.new_from_clipboard();
                true
            }
            Action::Copy => {
                self.copy();
                true
            }
            Action::SelectAll => {
                self.select_all();
                true
            }
            Action::Find => {
                self.start_prompt(PromptKind::Find);
                true
            }
            Action::FindNext | Action::FindPrevious => {
                self.find_step(action == Action::FindNext);
                true
            }
            Action::ZoomToSelection => {
                self.zoom_to_selection(lay);
                true
            }
            Action::AdjustSize => {
                if self.picture_page().is_some() {
                    self.start_prompt(PromptKind::Size);
                } else {
                    self.status = "Adjust Size works on pictures.".into();
                }
                true
            }
            Action::AdjustColor => {
                self.adjust_panel = !self.adjust_panel;
                true
            }
            Action::ToggleInspector => {
                self.inspector = !self.inspector;
                true
            }
            Action::ShowLocation => {
                self.show_location();
                true
            }
            Action::ToggleMagnifier => {
                self.magnifier = !self.magnifier;
                true
            }
            Action::ShowSearch | Action::ShowNotes | Action::ShowBookmarks => {
                self.side = match action {
                    Action::ShowSearch => Side::Search,
                    Action::ShowNotes => Side::Notes,
                    _ => Side::Bookmarks,
                };
                self.sidebar = true;
                self.view.sidebar_scroll = 0.0;
                true
            }
            Action::ContactSheet => {
                self.stop_editing();
                self.contact = !self.contact;
                self.slideshow = false;
                self.view.sidebar_scroll = 0.0;
                true
            }
            Action::Slideshow => {
                self.stop_editing();
                self.slideshow = !self.slideshow;
                self.contact = false;
                true
            }
            Action::ClearSignatures => {
                self.signatures.clear();
                let _ = prefs::save_signatures(&self.data_dir, &self.signatures);
                true
            }
            Action::PrefRememberPage
            | Action::PrefActualSize
            | Action::PrefSidebarOnOpen
            | Action::QualityHigh
            | Action::QualityMedium
            | Action::QualityLow
            | Action::SetAuthor => false,
            _ => self.track(|v| v.act_edit(ui, lay, action)),
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
            Action::FlipHorizontal => self.flip(true),
            Action::FlipVertical => self.flip(false),
            Action::Crop => self.crop_selection(),
            Action::RemoveBackground => self.remove_background(),
            Action::UseTool(tool) => {
                self.stop_editing();
                self.tool = tool;
            }
            Action::ShowPages | Action::ShowOutline => {
                let side = if action == Action::ShowPages { Side::Pages } else { Side::Outline };
                if self.side != side {
                    self.side = side;
                    self.view.sidebar_scroll = 0.0;
                }
            }
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
            Action::ResetColour => self.adjust = Adjust::default(),
            Action::Print => self.print(),
            Action::RevertToOriginal => self.revert(),
            Action::ExportPng => self.export_raster(Raster::Png),
            Action::ExportJpeg => self.export_raster(Raster::Jpeg),
            Action::ExportPdf => self.export_pdf(),
            Action::ExportAt25 | Action::ExportAt50 | Action::ExportAt100 | Action::ExportAt200 => {
                if let Some(scale) = action.export_scale() {
                    self.export_scale = scale;
                }
            }
            // everything else is handled in `act` and never reaches here
            _ => {}
        }
        true
    }

    /// File > Revert to Saved: drops all markup and unsaved picture edits, restores the page order, turns and mirroring and
    /// resets the colour. The saved file is opened again, so this can be undone with Undo.
    fn revert(&mut self) {
        self.editing = None;
        if let Some(doc) = &mut self.doc {
            let n = doc.sizes.len();
            doc.marks.clear();
            doc.order = (0..n).collect();
            doc.turns = vec![0; n];
            doc.flips = vec![0; n];
        }
        self.adjust = Adjust::default();
        self.selection = None;
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
            let job = printjob::pdf_job(doc.path.clone(), target, doc.order.clone(), doc.turns.clone(), doc.marks.clone());
            self.status = if doc.send(job) { "Printing".into() } else { "The document is not open.".into() };
            return;
        }
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, doc.orient_of(page), &self.adjust, 1.0);
        let target = printjob::scratch_file(&self.data_dir, &doc.path, "png");
        let written = match target.parent() {
            Some(dir) => std::fs::create_dir_all(dir).map_err(|e| e.to_string()),
            None => Ok(()),
        }
        .and_then(|()| export::write_image(&out, Raster::Png, &target, self.prefs.jpeg_quality));
        self.status = match written {
            Ok(()) => printjob::print_file(target),
            Err(e) => format!("Printing failed: {e}"),
        };
    }

    /// Writes the shown page as a PNG or JPEG at the export size. A PDF page is drawn by its worker first.
    fn export_raster(&mut self, raster: Raster) {
        let Some(doc) = &self.doc else { return };
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let orient = doc.orient_of(page);
        let target = export::target(&doc.path, raster.ext());
        let quality = self.prefs.jpeg_quality;
        if doc.is_pdf {
            let job = export::pdf_page_job(src, orient, doc.marks.clone(), self.adjust, self.export_scale, raster, quality, target);
            self.status = if doc.send(job) { "Exporting".into() } else { "The document is not open.".into() };
            return;
        }
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, orient, &self.adjust, self.export_scale);
        self.status = match export::write_image(&out, raster, &target, quality) {
            Ok(()) => format!("Exported {}", target.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Saves the PDF with its markup, page order and turns as `<name>-markup.pdf`. The worker writes it.
    fn export_pdf(&mut self) {
        let Some(doc) = &self.doc else { return };
        if !doc.is_pdf {
            return;
        }
        let target = export::target(&doc.path, "pdf");
        let job = pdfmarks::save_job(doc.path.clone(), target, doc.order.clone(), doc.turns.clone(), doc.marks.clone(), self.prefs.author.clone());
        self.status = if doc.send(job) { "Saving the PDF".into() } else { "The document is not open.".into() };
    }

    fn touch(&mut self, ui: &Ui, width: i32, height: i32, kind: TouchKind, x: f32, y: f32) -> bool {
        let lay = self.layout(ui, width, height);
        self.pointer = (x, y);
        match kind {
            TouchKind::Down => {
                // a press ends the note being typed (an empty note is dropped) before the edit is recorded
                self.stop_editing();
                self.track(|v| v.touch_down(ui, &lay, x, y))
            }
            TouchKind::Move => self.touch_move(ui, &lay, x, y) || self.magnifier,
            TouchKind::Up | TouchKind::Cancel => self.track(|v| v.touch_up(&lay, x, y)),
        }
    }

    /// A press on the contact sheet: a thumbnail opens its page, empty space starts a scroll.
    fn touch_contact(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let cells = canvas::contact_layout(ui, lay, count);
        match canvas::contact_hit(&cells, lay, self.view.sidebar_scroll, x, y) {
            Some(page) => {
                self.go_page(page);
                self.contact = false;
            }
            None => self.gesture = Some(Gesture::Scroll { last: y }),
        }
        true
    }

    fn touch_down(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        if let Some(action) = canvas::button_hit(ui, lay, x, y) {
            return self.act(ui, lay, action);
        }
        if y < lay.bar as f32 {
            return false;
        }
        if self.slideshow {
            return self.act(ui, lay, Action::Next);
        }
        if self.contact && self.doc.is_some() {
            return self.touch_contact(ui, lay, x, y);
        }
        if self.adjust_panel && self.doc.is_some() && canvas::in_adjust_panel(ui, lay, x, y) {
            if let Some((row, t)) = canvas::adjust_hit(ui, lay, x, y) {
                self.set_adjust_row(row, t);
                self.gesture = Some(Gesture::Slider { row });
            }
            return true;
        }
        if x < lay.side as f32 {
            if self.doc.is_none() {
                return false;
            }
            let rows = self.side_rows(ui);
            if let Some(i) = canvas::sidebar_hit(lay, &rows, self.view.sidebar_scroll, x, y) {
                self.sidebar_pick(i);
            }
            self.gesture = Some(Gesture::Scroll { last: y });
            return true;
        }
        if self.doc.is_none() {
            return false;
        }
        self.stop_editing();
        match self.tool {
            Tool::Select => {
                self.gesture = Some(Gesture::Select { start: (x, y), now: (x, y) });
                return true;
            }
            Tool::Mask => {
                self.mask_at(lay, x, y);
                return true;
            }
            _ => {}
        }
        match self.tool.mark_kind() {
            None => self.gesture = Some(Gesture::Pan { last: (x, y) }),
            Some(kind @ (MarkKind::Note | MarkKind::Text)) => {
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let a = canvas::to_source(&p, self.view.quarters, x, y);
                    let rect = if kind == MarkKind::Note {
                        Norm { x: a.0, y: a.1, w: 0.0, h: 0.0 }
                    } else {
                        Norm { x: a.0.min(1.0 - TEXT_BOX.0), y: a.1.min(1.0 - TEXT_BOX.1), w: TEXT_BOX.0, h: TEXT_BOX.1 }
                    };
                    if let Some(doc) = &mut self.doc {
                        doc.marks.push(Mark { kind, page: src, rect, color: MARK_BLUE, text: String::new(), ink: Vec::new() });
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

    /// A click on row `i` of the sidebar list: the page it names is shown.
    fn sidebar_pick(&mut self, i: usize) {
        let target = match self.side {
            Side::Pages => Some(i),
            // an outline entry goes to its page, when it names one that is still shown
            Side::Outline => self.doc.as_ref().and_then(|d| {
                let src = d.outline.get(i)?.page?;
                d.display_of(src)
            }),
            side => {
                if side == Side::Search {
                    self.find.current = Some(i);
                }
                self.doc.as_ref().and_then(|d| canvas::list_entries(d, side, &self.find.hits).get(i).and_then(|e| e.page))
            }
        };
        if let Some(page) = target {
            self.go_page(page);
        }
    }

    /// Sets Adjust Color slider `row` to `t` (0 to 1) along its range.
    fn set_adjust_row(&mut self, row: usize, t: f32) {
        if let Some(&(_, lo, hi)) = ADJUST_ROWS.get(row) {
            self.adjust = self.adjust.with(row, lo + (hi - lo) * t);
        }
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
                self.view.sidebar_scroll = (self.view.sidebar_scroll - delta).clamp(0.0, max);
                true
            }
            Some(Gesture::Drag { now, .. }) | Some(Gesture::Select { now, .. }) => {
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
            Some(Gesture::Slider { row }) => {
                let row = *row;
                self.set_adjust_row(row, canvas::adjust_track(ui, lay, x));
                true
            }
            None => false,
        }
    }

    fn touch_up(&mut self, lay: &Layout, x: f32, y: f32) -> bool {
        let Some(gesture) = self.gesture.take() else { return false };
        match gesture {
            Gesture::Drag { start, kind, .. } => {
                let moved = (x - start.0).abs() + (y - start.1).abs();
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let q = self.view.quarters;
                    let a = canvas::to_source(&p, q, start.0, start.1);
                    let b = canvas::to_source(&p, q, x, y);
                    let mark = drag_mark(kind, src, a, b);
                    let flat_ok = matches!(kind, MarkKind::Line | MarkKind::Arrow);
                    if moved >= 4.0 && (flat_ok || (mark.rect.w > 0.0 && mark.rect.h > 0.0)) {
                        if let Some(doc) = &mut self.doc {
                            doc.marks.push(mark);
                        }
                        self.save();
                    }
                }
            }
            Gesture::Ink { mut points } => {
                points.push((x, y));
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    let q = self.view.quarters;
                    let ink: Vec<(f32, f32)> = points.iter().map(|&(px, py)| canvas::to_source(&p, q, px, py)).collect();
                    if ink.len() >= 2 {
                        if self.tool == Tool::Sign {
                            self.keep_signature(&ink);
                        }
                        if let Some(doc) = &mut self.doc {
                            doc.marks.push(stroke(src, ink));
                        }
                        self.save();
                    }
                }
            }
            Gesture::Select { start, .. } => {
                let moved = (x - start.0).abs() + (y - start.1).abs();
                self.selection = None;
                if let (Some(p), Some(src)) = (self.placed(lay), self.src()) {
                    if moved >= 4.0 {
                        let q = self.view.quarters;
                        let rect = Norm::from_corners(canvas::to_source(&p, q, start.0, start.1), canvas::to_source(&p, q, x, y));
                        if rect.w > 0.0 && rect.h > 0.0 {
                            self.selection = Some((src, rect));
                        }
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn key(&mut self, ctx: &Ctx, key: KeyInput) -> bool {
        if self.prompt.is_some() {
            return self.key_prompt(ctx, &key);
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
            KeyInput::Code(27) if self.editing.is_none() && (self.slideshow || self.contact) => {
                self.slideshow = false;
                self.contact = false;
                true
            }
            KeyInput::Code(32) if self.slideshow => self.key_page(ctx, Action::Next),
            KeyInput::Code(13) | KeyInput::Code(27) => {
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

/// The mark a drag from `a` to `b` (page coordinates) makes. A line or an arrow keeps its two ends, because its
/// rectangle does not say which way it points.
fn drag_mark(kind: MarkKind, page: usize, a: (f32, f32), b: (f32, f32)) -> Mark {
    let ink = if matches!(kind, MarkKind::Line | MarkKind::Arrow) { vec![a, b] } else { Vec::new() };
    Mark { kind, page, rect: Norm::from_corners(a, b), color: color_for(kind), text: String::new(), ink }
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
        MarkKind::Redact => REDACT,
        _ => MARK_BLUE,
    }
}

/// The file name of `path`, for messages.
fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What kind of picture `path` is, from its extension.
fn picture_kind(path: &Path) -> &'static str {
    match export::raster_for(path) {
        Raster::Jpeg => "JPEG image",
        Raster::Png => "PNG image",
    }
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
    let mut m = AppMenuItem::new(action.id(), action.label()).enabled(enabled).checked(checked);
    m = match action {
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
        Action::NewFromClipboard => m.key("cmd+n"),
        Action::Copy => m.key("cmd+c"),
        Action::SelectAll => m.key("cmd+a"),
        Action::Find => m.key("cmd+f"),
        Action::FindNext => m.key("cmd+g"),
        Action::FindPrevious => m.key("cmd+shift+g"),
        Action::Crop => m.key("cmd+k"),
        Action::ToggleInspector => m.key("cmd+i"),
        Action::AdjustColor => m.key("cmd+alt+c"),
        Action::Slideshow => m.key("cmd+shift+f"),
        _ => m,
    };
    m
}

/// An item with a title of its own (a toggle that names its next state).
fn titled(action: Action, title: &str, enabled: bool) -> AppMenuItem {
    let mut m = item(action, enabled, false);
    m.title = title.into();
    m
}

impl App for Viewer {
    fn id(&self) -> &str {
        "is.olibuijr.preview.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the app's private storage holds the markup sidecars
        if self.data_dir.as_os_str().is_empty() {
            self.data_dir = ctx.data_dir.clone();
            self.recents = recents::load(&self.data_dir);
            self.prefs = Prefs::load(&ctx.defaults());
            self.signatures = prefs::load_signatures(&self.data_dir);
            self.sidebar = self.prefs.sidebar_on_open;
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let open = self.doc.is_some();
        let pdf = self.doc.as_ref().is_some_and(|d| d.is_pdf);
        let picture_doc = open && !pdf;
        // rotation needs a page with a picture to turn
        let picture = self.doc.as_ref().and_then(|d| d.page_size(self.view.page)).is_some();
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let bookmarks = self.doc.as_ref().map(|d| d.bookmark_places()).unwrap_or_default();
        let outline = self.doc.as_ref().is_some_and(|d| pdf && !d.outline.is_empty());
        let has_selection = self.selection.is_some_and(|(src, _)| Some(src) == self.src());
        let tool_item = |t: Tool, depth: u8| item(Action::UseTool(t), open, self.tool == t).depth(depth);
        let scale_items: Vec<AppMenuItem> = [Action::ExportAt25, Action::ExportAt50, Action::ExportAt100, Action::ExportAt200]
            .into_iter()
            .map(|a| item(a, open, a.export_scale() == Some(self.export_scale)))
            .collect();
        let mut file_items = vec![AppMenuItem::new(MENU_OPEN, "Open\u{2026}").key("cmd+o"), AppMenuItem::submenu("Open Recent")];
        for (i, path) in self.recents.paths().iter().enumerate() {
            file_items.push(AppMenuItem::new(MENU_RECENT + i as u32, &name_of(path)).depth(1));
        }
        if !self.recents.is_empty() {
            file_items.push(AppMenuItem::separator().depth(1));
        }
        file_items.push(AppMenuItem::new(MENU_CLEAR_RECENTS, "Clear Menu").enabled(!self.recents.is_empty()).depth(1));
        file_items.push(item(Action::NewFromClipboard, true, false));
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::Save, open, false));
        file_items.push(item(Action::RevertToOriginal, open, false));
        file_items.extend([AppMenuItem::separator(), item(Action::Print, open, false), AppMenuItem::separator()]);
        file_items.extend([item(Action::ExportPng, open, false), item(Action::ExportJpeg, open, false)]);
        file_items.push(AppMenuItem::separator());
        file_items.extend(scale_items);
        file_items.push(AppMenuItem::separator());
        file_items.extend(
            [(Action::QualityHigh, 0), (Action::QualityMedium, 1), (Action::QualityLow, 2)]
                .into_iter()
                .map(|(a, i)| item(a, true, self.prefs.jpeg_quality == prefs::QUALITIES[i].1)),
        );
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::ExportPdf, pdf, false));
        let sidebar_label = if self.sidebar { "Hide Sidebar" } else { "Show Sidebar" };
        let toolbar_label = if self.toolbar { "Hide Toolbar" } else { "Show Toolbar" };
        let inspector_label = if self.inspector { "Hide Inspector" } else { "Show Inspector" };
        let magnifier_label = if self.magnifier { "Hide Magnifier" } else { "Show Magnifier" };
        let mut tools = vec![
            tool_item(Tool::Pan, 0),
            tool_item(Tool::Select, 0),
            AppMenuItem::submenu("Annotate"),
        ];
        for t in [
            Tool::Highlight,
            Tool::Note,
            Tool::Text,
            Tool::Rect,
            Tool::Rounded,
            Tool::Oval,
            Tool::Line,
            Tool::Arrow,
            Tool::Star,
            Tool::Bubble,
            Tool::Underline,
            Tool::Strike,
        ] {
            tools.push(tool_item(t, 1));
        }
        tools.extend([tool_item(Tool::Sign, 0), tool_item(Tool::Redact, 0), tool_item(Tool::Mask, 0)]);
        tools.push(AppMenuItem::submenu("Signatures"));
        for i in 0..self.signatures.len() {
            tools.push(AppMenuItem::new(MENU_SIGNATURE + i as u32, &format!("Signature {}", i + 1)).enabled(open).depth(1));
        }
        tools.push(item(Action::ClearSignatures, !self.signatures.is_empty(), false).depth(1));
        tools.push(AppMenuItem::separator());
        tools.extend([
            item(Action::Crop, picture_doc && has_selection, false),
            item(Action::FlipHorizontal, picture, false),
            item(Action::FlipVertical, picture, false),
            item(Action::AdjustSize, picture_doc, false),
            item(Action::AdjustColor, open, self.adjust_panel),
            item(Action::RemoveBackground, picture_doc, false),
            AppMenuItem::separator(),
            titled(Action::ToggleInspector, inspector_label, open),
            item(Action::ShowLocation, picture_doc, false),
            titled(Action::ToggleMagnifier, magnifier_label, open),
        ]);
        let mut settings = vec![AppMenuItem::submenu("Settings")];
        settings.extend([
            item(Action::PrefRememberPage, true, self.prefs.remember_page).depth(1),
            item(Action::PrefActualSize, true, self.prefs.actual_size).depth(1),
            item(Action::PrefSidebarOnOpen, true, self.prefs.sidebar_on_open).depth(1),
            item(Action::SetAuthor, true, false).depth(1),
        ]);
        let mut view = vec![
            item(Action::ZoomIn, open, false),
            item(Action::ZoomOut, open, false),
            item(Action::ActualSize, open, self.view.zoom == Zoom::Factor(1.0)),
            item(Action::FitWindow, open, self.view.zoom == Zoom::Fit),
            item(Action::ZoomToSelection, has_selection, false),
            AppMenuItem::separator(),
            item(Action::RotateLeft, picture, false),
            item(Action::RotateRight, picture, false),
            AppMenuItem::separator(),
            AppMenuItem::new(Action::ToggleSidebar.id(), sidebar_label).key("cmd+alt+s").enabled(open),
            item(Action::ShowPages, open, self.side == Side::Pages),
            item(Action::ShowOutline, outline, self.side == Side::Outline),
            item(Action::ShowSearch, open, self.side == Side::Search),
            item(Action::ShowNotes, open, self.side == Side::Notes),
            item(Action::ShowBookmarks, open, self.side == Side::Bookmarks),
            AppMenuItem::separator(),
            titled(Action::ToggleToolbar, toolbar_label, true).key("cmd+alt+t"),
            item(Action::ContactSheet, open, self.contact),
            item(Action::Slideshow, open, self.slideshow),
            AppMenuItem::separator(),
        ];
        view.extend(settings);
        vec![
            AppMenu { title: "File".into(), items: file_items },
            AppMenu {
                title: "Edit".into(),
                items: vec![
                    item(Action::Undo, self.history.can_undo(), false),
                    item(Action::Redo, self.history.can_redo(), false),
                    AppMenuItem::separator(),
                    item(Action::Copy, open, false),
                    item(Action::SelectAll, open, false),
                    AppMenuItem::separator(),
                    item(Action::Find, pdf, false),
                    item(Action::FindNext, pdf, false),
                    item(Action::FindPrevious, pdf, false),
                ],
            },
            AppMenu { title: "View".into(), items: view },
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
                    item(Action::MovePageUp, pdf && self.view.page > 0, false),
                    item(Action::MovePageDown, pdf && self.view.page + 1 < count, false),
                    item(Action::DeletePage, pdf && count > 1, false),
                ],
            },
            AppMenu { title: "Tools".into(), items: tools },
            AppMenu {
                title: "Adjust".into(),
                items: vec![
                    item(Action::Brighter, open, false),
                    item(Action::Darker, open, false),
                    item(Action::Saturate, open, false),
                    item(Action::Desaturate, open, false),
                    item(Action::ResetColour, open && !self.adjust.is_identity(), false),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.open_menu(ctx, id) {
            return true;
        }
        if (MENU_SIGNATURE..MENU_SIGNATURE + prefs::MAX_SIGNATURES as u32).contains(&id) {
            self.track(|v| v.use_signature((id - MENU_SIGNATURE) as usize));
            return true;
        }
        let Some(action) = Action::from_id(id) else { return false };
        if self.pref_action(ctx, action) {
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
            Input::Touch { kind, x, y, .. } => {
                let (w, h) = self.size(ctx);
                self.touch(&ctx.ui, w, h, kind, x, y)
            }
            _ => false,
        }
    }

    /// The answer to the open panel: the chosen file is copied into the app's data folder and opened from there.
    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        let doc = match ev {
            DocumentEvent::Opened(doc) => doc,
            DocumentEvent::OpenedMany { docs, .. } => match docs.into_iter().next() {
                Some(doc) => doc,
                None => return false,
            },
            _ => return false,
        };
        let mut bytes = Vec::new();
        if let Err(e) = (&doc.file).read_to_end(&mut bytes) {
            self.status = format!("Could not read {}: {e}", doc.name);
            return true;
        }
        match recents::keep_copy(&self.data_dir, &doc.name, &bytes) {
            Ok(path) => self.open_path(&path),
            Err(e) => self.status = format!("Could not open {}: {e}", doc.name),
        }
        true
    }

    fn tick(&mut self, ctx: &Ctx, _t: f64) -> bool {
        let mut changed = self.pump();
        if let Some(text) = self.copied.take() {
            self.status = if text.trim().is_empty() {
                "This page has no text to copy.".into()
            } else {
                ctx.set_pasteboard_string(&text);
                "Copied the page text".into()
            };
            changed = true;
        }
        self.save_page();
        changed
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.stop_editing();
        self.save();
        self.save_page();
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let lay = self.layout(&ctx.ui, s.w as i32, s.h as i32);
        let draft = self.draft(&lay);
        let selection = self.selection_now(&lay);
        let inspector = self.inspector_lines();
        let scene = Scene {
            doc: self.doc.as_ref(),
            view: self.view,
            tool: self.tool,
            adjust: self.adjust,
            side: self.side,
            draft,
            editing: self.editing,
            status: &self.status,
            hits: &self.find.hits,
            selection,
            contact: self.contact,
            slideshow: self.slideshow,
            adjust_panel: self.adjust_panel,
            inspector: inspector.as_deref(),
            magnifier: self.magnifier.then_some(self.pointer),
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
        assert_eq!(color_for(MarkKind::Redact), REDACT);
    }

    #[test]
    fn a_stroke_is_boxed_by_its_points() {
        let m = stroke(3, vec![(0.25, 0.5), (0.75, 0.25)]);
        assert_eq!(m.kind, MarkKind::Ink);
        assert_eq!(m.page, 3);
        assert_eq!(m.rect, Norm { x: 0.25, y: 0.25, w: 0.5, h: 0.25 });
        assert_eq!(m.ink.len(), 2);
    }

    #[test]
    fn a_dragged_arrow_keeps_which_way_it_points() {
        let m = drag_mark(MarkKind::Arrow, 1, (0.8, 0.2), (0.1, 0.6));
        assert_eq!(m.ink, vec![(0.8, 0.2), (0.1, 0.6)]);
        assert_eq!(m.rect, Norm { x: 0.1, y: 0.2, w: 0.7, h: 0.4 });
        assert!(drag_mark(MarkKind::Rect, 0, (0.0, 0.0), (0.5, 0.5)).ink.is_empty());
    }

    #[test]
    fn prompts_take_only_the_characters_they_use() {
        assert!(PromptKind::GoTo.accepts('7') && !PromptKind::GoTo.accepts('x'));
        assert!(PromptKind::Size.accepts('x') && PromptKind::Size.accepts('4') && !PromptKind::Size.accepts('q'));
        assert!(PromptKind::Find.accepts('q') && !PromptKind::Find.accepts('\n'));
    }

    #[test]
    fn picture_kinds_follow_the_extension() {
        assert_eq!(picture_kind(Path::new("/a/b.jpeg")), "JPEG image");
        assert_eq!(picture_kind(Path::new("/a/b.png")), "PNG image");
    }
}
