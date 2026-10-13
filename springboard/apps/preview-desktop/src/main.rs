//! Viewer, a document window for images and PDFs: zoom and fit, rotation, page navigation with a
//! thumbnails sidebar or an outline, page moves and deletes, markup (highlight, note, rectangle, rounded
//! rectangle, freehand signature), Adjust Color through the image filter API, PNG and JPEG export at a
//! chosen size, and Export PDF, which saves the markup, page order and rotation into a new PDF. A PDF is
//! opened and drawn by its worker thread (`pdfjob`); pages and markup live in `document` and `canvas`, the
//! pure state and maths in `model`, and the file writing in `export` and `pdfmarks`.

mod canvas;
mod document;
mod export;
mod model;
mod pdfjob;
mod pdfmarks;
mod printjob;
mod recents;

use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use canvas::{Action, Layout, PictureCache, Scene, Side, Tool};
use document::Document;
use export::Raster;
use model::{Adjust, History, Mark, MarkKind, Norm, Snapshot, Trail, View, Zoom, HIGHLIGHT, MARK_BLUE};
use pdfjob::{Inbox, Job, Reply};
use recents::Recents;
use springboard::shell::books::engine;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// File > Open and the Open Recent items. The ids sit above every `Action` id.
const MENU_OPEN: u32 = 0x5001;
const MENU_CLEAR_RECENTS: u32 = 0x5002;
const MENU_RECENT: u32 = 0x5010;
/// The kinds of file the open panel offers.
const OPEN_TYPES: [&str; 4] = ["pdf", "png", "jpg", "jpeg"];

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
    /// The page number being typed after Go > Go to Page.
    goto: Option<String>,
    /// Whether the thumbnails and outline sidebar is shown.
    sidebar: bool,
    /// The files opened lately, for File > Open Recent.
    recents: Recents,
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
        }
    }

    /// The window layout: the sidebar takes no room while it is hidden.
    fn layout(&self, ui: &Ui, w: i32, h: i32) -> Layout {
        let mut lay = Layout::new(ui, w, h);
        if !self.sidebar {
            lay.side = 0;
        }
        lay
    }

    /// The document's editable state, or None with no document.
    fn snapshot(&self) -> Option<Snapshot> {
        let doc = self.doc.as_ref()?;
        Some(Snapshot { marks: doc.marks.clone(), order: doc.order.clone(), turns: doc.turns.clone(), adjust: self.adjust })
    }

    /// Puts a state from the undo history back.
    fn restore(&mut self, state: Snapshot) {
        self.editing = None;
        if let Some(doc) = &mut self.doc {
            doc.marks = state.marks;
            doc.order = state.order;
            doc.turns = state.turns;
        }
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
        self.stop_editing();
        self.doc = None;
        self.loading = None;
        self.cache = None;
        self.view = View::default();
        self.side = Side::Pages;
        self.history = History::default();
        self.trail = Trail::default();
        self.goto = None;
        self.status.clear();
        self.remember(path);
        if matches!(engine::format_of(path), Some(engine::Format::Pdf)) {
            let (jobs, inbox) = pdfjob::spawn(path.to_path_buf());
            self.loading = Some(Loading { path: path.to_path_buf(), jobs, inbox });
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

    /// Takes what the PDF worker has sent: the open result, drawn pages and the results of saves and exports.
    /// True when the window needs a new frame.
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
                    self.status.clear();
                    self.sync_turn();
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
                    Reply::Written(result) => {
                        self.status = match result {
                            Ok(path) if path.extension().is_some_and(|e| e == "pdf") => format!("Saved {}", path.display()),
                            Ok(path) => format!("Exported {}", path.display()),
                            Err(e) => format!("Save failed: {e}"),
                        };
                    }
                    Reply::Printed(status) => self.status = status,
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
                Some(Mark { kind: *kind, page: src, rect: Norm::from_corners(a, b), color: color_for(*kind), text: String::new(), ink: Vec::new() })
            }
            Gesture::Ink { points } => Some(stroke(src, points.iter().map(|&(x, y)| canvas::to_source(&p, q, x, y)).collect())),
            _ => None,
        }
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
            KeyInput::Code(27) => {
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
            | Action::ToggleSidebar => {}
            Action::ExportPng => self.export_raster(Raster::Png),
            Action::ExportJpeg => self.export_raster(Raster::Jpeg),
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
        }
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
            let job = printjob::pdf_job(doc.path.clone(), target, doc.order.clone(), doc.turns.clone(), doc.marks.clone());
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

    /// Writes the shown page as a PNG or JPEG at the export size. A PDF page is drawn by its worker first.
    fn export_raster(&mut self, raster: Raster) {
        let Some(doc) = &self.doc else { return };
        let page = self.view.page;
        let Some(src) = doc.source_of(page) else { return };
        let turns = doc.turn_of(page);
        let target = export::target(&doc.path, raster.ext());
        if doc.is_pdf {
            let job = export::pdf_page_job(src, turns, doc.marks.clone(), self.adjust, self.export_scale, raster, target);
            self.status = if doc.send(job) { "Exporting".into() } else { "The document is not open.".into() };
            return;
        }
        let Some(img) = doc.picture(page) else { return };
        let out = export::finish(&img, &doc.marks, src, turns, &self.adjust, self.export_scale);
        self.status = match export::write_image(&out, raster, &target) {
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
        let job = pdfmarks::save_job(doc.path.clone(), target, doc.order.clone(), doc.turns.clone(), doc.marks.clone());
        self.status = if doc.send(job) { "Saving the PDF".into() } else { "The document is not open.".into() };
    }

    fn touch(&mut self, ui: &Ui, width: i32, height: i32, kind: TouchKind, x: f32, y: f32) -> bool {
        let lay = self.layout(ui, width, height);
        match kind {
            TouchKind::Down => {
                // a press ends the note being typed (an empty note is dropped) before the edit is recorded
                self.stop_editing();
                self.track(|v| v.touch_down(ui, &lay, x, y))
            }
            TouchKind::Move => self.touch_move(ui, &lay, x, y),
            TouchKind::Up | TouchKind::Cancel => self.track(|v| v.touch_up(&lay, x, y)),
        }
    }

    fn touch_down(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        if let Some(action) = canvas::button_hit(ui, lay, x, y) {
            return self.act(ui, lay, action);
        }
        if y < lay.bar as f32 {
            return false;
        }
        if x < lay.side as f32 {
            if self.doc.is_none() {
                return false;
            }
            let rows = self.side_rows(ui);
            if let Some(i) = canvas::sidebar_hit(lay, &rows, self.view.sidebar_scroll, x, y) {
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
                }
            }
            self.gesture = Some(Gesture::Scroll { last: y });
            return true;
        }
        if self.doc.is_none() {
            return false;
        }
        self.stop_editing();
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
                self.view.sidebar_scroll = (self.view.sidebar_scroll - delta).clamp(0.0, max);
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
                    let rect = Norm::from_corners(a, b);
                    if moved >= 4.0 && rect.w > 0.0 && rect.h > 0.0 {
                        if let Some(doc) = &mut self.doc {
                            doc.marks.push(Mark { kind, page: src, rect, color: color_for(kind), text: String::new(), ink: Vec::new() });
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

    fn key(&mut self, ctx: &Ctx, key: KeyInput) -> bool {
        if self.goto.is_some() {
            return self.key_goto(&key);
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
        _ => m,
    };
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
        let tool_items: Vec<AppMenuItem> = [Tool::Pan, Tool::Highlight, Tool::Note, Tool::Rect, Tool::Rounded, Tool::Sign]
            .into_iter()
            .map(|t| item(Action::UseTool(t), open, self.tool == t))
            .collect();
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
        file_items.extend([AppMenuItem::separator(), item(Action::Print, open, false), AppMenuItem::separator()]);
        file_items.extend([item(Action::ExportPng, open, false), item(Action::ExportJpeg, open, false)]);
        file_items.push(AppMenuItem::separator());
        file_items.extend(scale_items);
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::ExportPdf, pdf, false));
        file_items.push(AppMenuItem::separator());
        file_items.push(item(Action::RevertToOriginal, open, false));
        let sidebar_label = if self.sidebar { "Hide Sidebar" } else { "Show Sidebar" };
        vec![
            AppMenu { title: "File".into(), items: file_items },
            AppMenu {
                title: "Edit".into(),
                items: vec![item(Action::Undo, self.history.can_undo(), false), item(Action::Redo, self.history.can_redo(), false)],
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
                    AppMenuItem::new(Action::ToggleSidebar.id(), sidebar_label).key("cmd+alt+s").enabled(open),
                    item(Action::ShowPages, open, self.side == Side::Pages),
                    item(Action::ShowOutline, outline, self.side == Side::Outline),
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
                    item(Action::ResetColour, open && !self.adjust.is_identity(), false),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.open_menu(ctx, id) {
            return true;
        }
        let Some(action) = Action::from_id(id) else { return false };
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

    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        self.pump()
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.stop_editing();
        self.save();
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let lay = self.layout(&ctx.ui, s.w as i32, s.h as i32);
        let draft = self.draft(&lay);
        let scene = Scene {
            doc: self.doc.as_ref(),
            view: self.view,
            tool: self.tool,
            adjust: self.adjust,
            side: self.side,
            draft,
            editing: self.editing,
            status: &self.status,
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
