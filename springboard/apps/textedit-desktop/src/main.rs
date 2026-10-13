//! Text Editor as an installed BifrOSt app: plain and rich text documents, opened and saved through the shell's document
//! panels, with bold, italic, underline, size, font and alignment, find and replace, a word count, and undo and redo.
//!
//! The document model is in `text.rs`, the file formats in `rtf.rs` and `document.rs`, the line breaking in
//! `layout.rs`, and the window in `view.rs`. This file wires them to the SDK: the document controller, menus, keys,
//! touches and the canvas.

mod document;
mod layout;
mod rtf;
mod text;
mod view;

use bifrost_app::controls::undo::ChangeCount;
use bifrost_app::document::{is_menu_id, DocId, DocumentController, Notice};
use bifrost_app::kit::canvas::Canvas;
use bifrost_app::kit::ui::Ui;
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::proto::CHORD_FLAG;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use document::TextDocument;
use layout::Layout;
use text::{Align, Kind, Style};
use view::{Action, Button, Focus, Hit};

/// The layout of the document as last drawn, kept until the text, the width or the document changes.
struct Cache {
    doc: DocId,
    revision: u64,
    width: f32,
    layout: Layout,
}

struct TextEditApp {
    docs: DocumentController<TextDocument>,
    /// the find bar: whether it is open, its two fields, and which one has the keyboard
    find_open: bool,
    find: String,
    replace: String,
    focus: Focus,
    /// the status row's note (a search that found nothing, the count of replacements), until the next edit
    note: Option<String>,
    /// how far the page is scrolled, in points, and whether the caret should be brought into view
    scroll: f32,
    follow: bool,
    /// a press in the page is extending a selection from its anchor
    drag: bool,
    /// bumped by every change to a document, so the cached layout is rebuilt
    revision: u64,
    cache: Option<Cache>,
}

/// The document of the active window, if any.
fn active_doc(docs: &DocumentController<TextDocument>) -> Option<&TextDocument> {
    docs.active().and_then(|id| docs.get(id))
}

fn active_doc_mut(docs: &mut DocumentController<TextDocument>) -> Option<&mut TextDocument> {
    let id = docs.active()?;
    docs.get_mut(id)
}

/// Pasted text uses the page's line break.
fn normalize_breaks(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

impl TextEditApp {
    fn new() -> Self {
        TextEditApp {
            docs: DocumentController::new(),
            find_open: false,
            find: String::new(),
            replace: String::new(),
            focus: Focus::Body,
            note: None,
            scroll: 0.0,
            follow: true,
            drag: false,
            revision: 0,
            cache: None,
        }
    }

    /// The window's regions for the current window size and find bar.
    fn geometry(&self, ui: &Ui, width_px: u32, height_px: u32) -> view::Geometry {
        view::geometry(width_px as f32 / ui.s, height_px as f32 / ui.s, self.find_open)
    }

    /// The toolbar's status: what the document has at the caret.
    fn status(&self) -> view::Status {
        match active_doc(&self.docs) {
            Some(doc) => view::Status {
                style: doc.text.style(),
                align: doc.text.align_at(doc.text.head()),
                can_undo: doc.text.can_undo(),
                can_redo: doc.text.can_redo(),
                find_open: self.find_open,
            },
            None => view::Status {
                style: Style::default(),
                align: Align::Left,
                can_undo: false,
                can_redo: false,
                find_open: self.find_open,
            },
        }
    }

    /// The toolbar's buttons and, with the find bar open, its buttons.
    fn buttons(&self, ui: &Ui, geom: &view::Geometry) -> (Vec<Button>, Vec<Button>) {
        let toolbar = view::toolbar(ui, &self.status());
        let find = if self.find_open { view::find_buttons(ui, geom) } else { Vec::new() };
        (toolbar, find)
    }

    /// Something in a document changed: the layout is stale, the caret should show, and the controller learns the
    /// new change count (the window's edited flag follows it).
    fn edited(&mut self) {
        self.revision += 1;
        self.follow = true;
        self.note = None;
        if let Some(id) = self.docs.active() {
            let history = self.docs.get(id).map(|d| d.text.history());
            if let Some(h) = history {
                self.docs.note_change(id, ChangeCount { position: h.position, epoch: h.epoch });
            }
        }
    }

    /// A document was opened or created: the page starts at the top.
    fn after_documents(&mut self, notices: &[Notice]) {
        for n in notices {
            if matches!(n, Notice::Created(_) | Notice::Opened(_) | Notice::Reverted(_)) {
                self.scroll = 0.0;
                self.follow = true;
                self.revision += 1;
            }
        }
    }

    /// Rebuilds the layout of the active document when it is stale.
    fn ensure_layout(&mut self, ui: &Ui, width: f32) {
        let Some(id) = self.docs.active() else { return };
        let fresh = self.cache.as_ref().is_some_and(|c| c.doc == id && c.revision == self.revision && c.width == width);
        if fresh {
            return;
        }
        let Some(doc) = self.docs.get(id) else { return };
        let measure = |ch: char, style: &Style| view::char_width(ui, ch, style);
        let layout = layout::layout(doc.text.cells(), width, &measure);
        self.cache = Some(Cache { doc: id, revision: self.revision, width, layout });
    }

    /// Runs a text command on the active document. Returns whether the window needs a frame.
    fn perform(&mut self, ctx: &Ctx, action: Action) -> bool {
        match action {
            Action::Document(id) => {
                let Some(notices) = self.docs.menu_action(ctx, id) else { return false };
                self.after_documents(&notices);
                return true;
            }
            Action::Find => {
                self.find_open = true;
                self.focus = Focus::Find;
                return true;
            }
            Action::FindDone => {
                self.find_open = false;
                self.focus = Focus::Body;
                return true;
            }
            Action::FocusFind => {
                self.focus = Focus::Find;
                return true;
            }
            Action::FocusReplace => {
                self.focus = Focus::Replace;
                return true;
            }
            Action::FindNext => return self.find_step(true),
            Action::FindPrevious => return self.find_step(false),
            Action::Replace => {
                let (query, replacement, case) = (self.find.clone(), self.replace.clone(), false);
                let replaced = active_doc_mut(&mut self.docs).is_some_and(|d| d.text.replace_selection(&query, &replacement, case));
                if replaced {
                    self.edited();
                }
                return self.find_step(true) || replaced;
            }
            Action::ReplaceAll => {
                let (query, replacement) = (self.find.clone(), self.replace.clone());
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                let count = doc.text.replace_all(&query, &replacement, false);
                let note = if count == 0 { format!("\"{query}\" not found") } else { format!("{count} replaced") };
                self.edited();
                self.note = Some(note);
                return true;
            }
            _ => {}
        }
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let changed = match action {
            Action::Undo => doc.text.undo(),
            Action::Redo => doc.text.redo(),
            Action::Cut => {
                let cut = doc.text.cut();
                if cut.is_empty() {
                    false
                } else {
                    clipboard::set_text(&cut);
                    true
                }
            }
            Action::Copy => {
                let sel = doc.text.selection();
                let copied: String = doc.text.cells()[sel].iter().map(|c| c.ch).collect();
                if !copied.is_empty() {
                    clipboard::set_text(&copied);
                }
                return false;
            }
            Action::Paste => match clipboard::text() {
                Some(s) if !s.is_empty() => {
                    doc.text.insert(&normalize_breaks(&s), Kind::Pasting);
                    true
                }
                _ => false,
            },
            Action::SelectAll => {
                doc.text.select_all();
                return true;
            }
            Action::Bold => {
                doc.text.toggle_bold();
                true
            }
            Action::Italic => {
                doc.text.toggle_italic();
                true
            }
            Action::Underline => {
                doc.text.toggle_underline();
                true
            }
            Action::Bigger => {
                doc.text.bigger();
                true
            }
            Action::Smaller => {
                doc.text.smaller();
                true
            }
            Action::Font(index) => {
                doc.text.set_font(index);
                true
            }
            Action::Align(align) => {
                doc.text.set_align(align);
                true
            }
            _ => false,
        };
        if changed {
            self.edited();
        }
        changed
    }

    /// Finds the next (or previous) match of the find field from the selection, and selects it.
    fn find_step(&mut self, forward: bool) -> bool {
        if self.find.is_empty() {
            return false;
        }
        let query = self.find.clone();
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let sel = doc.text.selection();
        let from = if forward { sel.end } else { sel.start };
        match doc.text.find(&query, from, forward, false) {
            Some(r) => {
                doc.text.select(r.start, r.end);
                self.note = None;
            }
            None => self.note = Some(format!("\"{query}\" not found")),
        }
        self.follow = true;
        true
    }

    /// Typed text: into the find field when it has the keyboard, otherwise into the page.
    fn type_into(&mut self, s: &str) -> bool {
        let printable: String = s.chars().filter(|c| !c.is_control()).collect();
        if printable.is_empty() {
            return false;
        }
        if self.find_open && self.focus != Focus::Body {
            match self.focus {
                Focus::Find => self.find.push_str(&printable),
                _ => self.replace.push_str(&printable),
            }
            return true;
        }
        self.insert_raw(&printable)
    }

    /// Inserts text into the page as typed, line breaks included.
    fn insert_raw(&mut self, s: &str) -> bool {
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.type_text(s);
        self.edited();
        true
    }

    /// Backspace: from the focused field, or the page.
    fn backspace(&mut self) -> bool {
        if self.find_open && self.focus != Focus::Body {
            let field = if self.focus == Focus::Find { &mut self.find } else { &mut self.replace };
            return field.pop().is_some();
        }
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.delete_backward();
        self.edited();
        true
    }

    /// A key that moves the caret or deletes forward (the page only).
    fn page_key(&mut self, ctx: &Ctx, code: u32) -> bool {
        let width = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h).text_w;
        self.ensure_layout(&ctx.ui, width);
        let measure = |ch: char, style: &Style| view::char_width(&ctx.ui, ch, style);
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let head = doc.text.head();
        let target = match code {
            37 => {
                if doc.text.has_selection() { doc.text.selection().start } else { head.saturating_sub(1) }
            }
            39 => {
                if doc.text.has_selection() { doc.text.selection().end } else { (head + 1).min(doc.text.len()) }
            }
            38 | 40 => {
                let Some(cache) = self.cache.as_ref() else { return false };
                cache.layout.move_vertical(doc.text.cells(), head, code == 40, &measure)
            }
            _ => return false,
        };
        doc.text.select(target, target);
        self.follow = true;
        true
    }

    /// Mouse or finger: a press places the caret (or presses a control), a drag extends the selection.
    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let (px, py) = (x / ctx.ui.s, y / ctx.ui.s);
        match kind {
            TouchKind::Down => {
                let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
                let (toolbar, find) = self.buttons(&ctx.ui, &geom);
                let all: Vec<Button> = toolbar.into_iter().chain(find).collect();
                match view::hit(&all, &geom, px, py) {
                    Some(Hit::Button(b)) => {
                        return match (b.enabled, b.action) {
                            (true, Some(action)) => self.perform(ctx, action),
                            _ => false,
                        };
                    }
                    Some(Hit::Field(field)) => {
                        self.focus = field;
                        return true;
                    }
                    None => {}
                }
                if py < geom.body_top || py >= geom.body_bottom {
                    return false;
                }
                self.focus = Focus::Body;
                let width = geom.text_w;
                self.ensure_layout(&ctx.ui, width);
                let measure = |ch: char, style: &Style| view::char_width(&ctx.ui, ch, style);
                let at = px - geom.text_x;
                let scroll_y = py - geom.body_top + self.scroll;
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                let Some(cache) = self.cache.as_ref() else { return false };
                let index = cache.layout.index_at(doc.text.cells(), at, scroll_y, &measure);
                doc.text.select(index, index);
                self.drag = true;
                self.follow = true;
                true
            }
            TouchKind::Move => {
                if !self.drag {
                    return false;
                }
                let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
                self.ensure_layout(&ctx.ui, geom.text_w);
                let measure = |ch: char, style: &Style| view::char_width(&ctx.ui, ch, style);
                let at = px - geom.text_x;
                let scroll_y = py - geom.body_top + self.scroll;
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                let Some(cache) = self.cache.as_ref() else { return false };
                let index = cache.layout.index_at(doc.text.cells(), at, scroll_y, &measure);
                let anchor = doc.text.anchor();
                doc.text.select(anchor, index);
                true
            }
            TouchKind::Up | TouchKind::Cancel => {
                self.drag = false;
                false
            }
        }
    }

    fn edit_menu(&self) -> AppMenu {
        let doc = active_doc(&self.docs);
        let (can_undo, can_redo, has_sel, undo, redo) = match doc {
            Some(d) => (
                d.text.can_undo(),
                d.text.can_redo(),
                d.text.has_selection(),
                d.text.undo_name().map_or("Undo".to_string(), |n| format!("Undo {n}")),
                d.text.redo_name().map_or("Redo".to_string(), |n| format!("Redo {n}")),
            ),
            None => (false, false, false, "Undo".into(), "Redo".into()),
        };
        let has_find = !self.find.is_empty();
        AppMenu {
            title: "Edit".into(),
            items: vec![
                menu_item(Action::Undo, &undo, "cmd+z", can_undo),
                menu_item(Action::Redo, &redo, "cmd+shift+z", can_redo),
                AppMenuItem::separator(),
                menu_item(Action::Cut, "Cut", "cmd+x", has_sel),
                menu_item(Action::Copy, "Copy", "cmd+c", has_sel),
                menu_item(Action::Paste, "Paste", "cmd+v", clipboard::text().is_some()),
                menu_item(Action::SelectAll, "Select All", "cmd+a", doc.is_some()),
                AppMenuItem::separator(),
                menu_item(Action::Find, "Find\u{2026}", "cmd+f", true),
                menu_item(Action::FindNext, "Find Next", "cmd+g", has_find),
                menu_item(Action::FindPrevious, "Find Previous", "cmd+shift+g", has_find),
                menu_item(Action::Replace, "Replace", "", self.find_open && has_find),
                menu_item(Action::ReplaceAll, "Replace All", "", self.find_open && has_find),
            ],
        }
    }

    fn format_menu(&self) -> AppMenu {
        let style = self.status().style;
        let align = self.status().align;
        let enabled = active_doc(&self.docs).is_some();
        let mut items = vec![
            menu_item(Action::Bold, "Bold", "cmd+b", enabled).checked(style.bold),
            menu_item(Action::Italic, "Italic", "cmd+i", enabled).checked(style.italic),
            menu_item(Action::Underline, "Underline", "cmd+u", enabled).checked(style.underline),
            AppMenuItem::separator(),
            menu_item(Action::Bigger, "Bigger", "cmd+=", enabled),
            menu_item(Action::Smaller, "Smaller", "cmd+-", enabled),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Font"),
        ];
        for index in 0..text::FAMILIES.len() as u8 {
            items.push(menu_item(Action::Font(index), view::family_name(index), "", enabled).checked(style.font == index).depth(1));
        }
        items.push(AppMenuItem::submenu("Text").depth(0));
        for (title, a) in [("Align Left", Align::Left), ("Center", Align::Center), ("Align Right", Align::Right), ("Justify", Align::Justified)] {
            items.push(menu_item(Action::Align(a), title, "", enabled).checked(align == a).depth(1));
        }
        AppMenu { title: "Format".into(), items }
    }
}

/// A menu item for an action; document commands never come through here.
fn menu_item(action: Action, title: &str, key: &str, enabled: bool) -> AppMenuItem {
    let item = AppMenuItem::new(action.id().unwrap_or(0), title).enabled(enabled);
    if key.is_empty() {
        item
    } else {
        item.key(key)
    }
}

impl App for TextEditApp {
    fn id(&self) -> &str {
        "is.olibuijr.textedit.desktop"
    }

    fn configure(&mut self, _ctx: &Ctx) {
        // a window starts with an untitled document, as a new document would
        if self.docs.is_empty() {
            self.docs.new_document();
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Command(command) => match command {
                Command::Cut => self.perform(ctx, Action::Cut),
                Command::Copy => self.perform(ctx, Action::Copy),
                Command::Paste => self.perform(ctx, Action::Paste),
                Command::SelectAll => self.perform(ctx, Action::SelectAll),
                Command::Undo => self.perform(ctx, Action::Undo),
                Command::Redo => self.perform(ctx, Action::Redo),
            },
            Input::Key(KeyInput::Text(s)) => self.type_into(&s),
            Input::Key(KeyInput::Code(code)) => {
                if code & CHORD_FLAG != 0 {
                    // chords are the menus' key equivalents, which the shell routes as menu actions
                    return false;
                }
                match code {
                    8 => self.backspace(),
                    13 => {
                        if self.find_open && self.focus != Focus::Body {
                            self.find_step(true)
                        } else {
                            self.insert_raw("\n")
                        }
                    }
                    27 => {
                        if self.find_open {
                            self.perform(ctx, Action::FindDone)
                        } else {
                            false
                        }
                    }
                    9 => {
                        if self.find_open {
                            self.focus = match self.focus {
                                Focus::Find => Focus::Replace,
                                _ => Focus::Find,
                            };
                            true
                        } else {
                            false
                        }
                    }
                    46 if self.focus == Focus::Body => match active_doc_mut(&mut self.docs) {
                        Some(doc) => {
                            doc.text.delete_forward();
                            self.edited();
                            true
                        }
                        None => false,
                    },
                    37 | 38 | 39 | 40 if self.focus == Focus::Body => self.page_key(ctx, code),
                    _ => false,
                }
            }
            Input::Touch { kind, x, y, .. } => self.touch(ctx, kind, x, y),
            _ => false,
        }
    }

    /// Copy and Paste are enabled only when they have something to act on.
    fn commands(&self, _ctx: &Ctx) -> Commands {
        let (has_sel, can_undo, can_redo) = match active_doc(&self.docs) {
            Some(d) => (d.text.has_selection(), d.text.can_undo(), d.text.can_redo()),
            None => (false, false, false),
        };
        Commands::default()
            .with(Command::Cut, has_sel)
            .with(Command::Copy, has_sel)
            .with(Command::Paste, clipboard::text().is_some())
            .with(Command::SelectAll, active_doc(&self.docs).is_some())
            .with(Command::Undo, can_undo)
            .with(Command::Redo, can_redo)
    }

    fn undo_titles(&self, _ctx: &Ctx) -> (String, String) {
        match active_doc(&self.docs) {
            Some(d) => (
                d.text.undo_name().map_or("Undo".into(), |n| format!("Undo {n}")),
                d.text.redo_name().map_or("Redo".into(), |n| format!("Redo {n}")),
            ),
            None => ("Undo".into(), "Redo".into()),
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        vec![self.docs.file_menu(), self.edit_menu(), self.format_menu()]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if is_menu_id(id) {
            return self.perform(ctx, Action::Document(id));
        }
        match Action::from_id(id) {
            Some(action) => self.perform(ctx, action),
            None => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match self.docs.handle(ctx, ev) {
            Some(notices) => {
                self.after_documents(&notices);
                true
            }
            None => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.docs.tick(ctx, t);
        false
    }

    fn next_wake(&self, _t: f64) -> Option<f64> {
        self.docs.next_wake()
    }

    fn suspend(&mut self, ctx: &Ctx) {
        self.docs.suspend(ctx);
    }

    /// The window draws through `Canvas`, so the same routine serves the CPU `Surface` and the GPU canvas.
    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (w, h) = canvas.size();
        let ui = &ctx.ui;
        let geom = self.geometry(ui, ctx.cfg.w, ctx.cfg.h);
        let (toolbar, find) = self.buttons(ui, &geom);
        view::draw_bar(canvas, ui, &geom);
        for b in &toolbar {
            view::draw_button(canvas, ui, b);
        }
        if self.find_open {
            view::draw_find(canvas, ui, &geom, &find, &self.find, &self.replace, self.focus);
        }
        self.ensure_layout(ui, geom.text_w);
        view::page_clip(canvas, ui, &geom);
        let view_h = geom.body_bottom - geom.body_top;
        if let (Some(doc), Some(cache)) = (active_doc(&self.docs), self.cache.as_ref()) {
            let sel = doc.text.selection();
            let head = doc.text.head();
            if self.follow {
                let (_, top, height) = cache.layout.caret(doc.text.cells(), head, &|ch: char, st: &Style| view::char_width(ui, ch, st));
                if top < self.scroll {
                    self.scroll = top;
                } else if top + height > self.scroll + view_h {
                    self.scroll = top + height - view_h;
                }
                self.follow = false;
            }
            let max_scroll = (cache.layout.height - view_h).max(0.0);
            self.scroll = self.scroll.clamp(0.0, max_scroll);
            view::draw_text(canvas, ui, &geom, &cache.layout, doc.text.cells(), sel, head, self.scroll, self.focus == Focus::Body);
        }
        canvas.pop_clip();
        let (words, chars) = active_doc(&self.docs).map_or((0, 0), |d| (d.text.word_count(), d.text.char_count()));
        view::draw_status(canvas, ui, &geom, words, chars, self.note.as_deref());
        let mut d = Damage::new(w as usize, h as usize);
        d.all();
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(TextEditApp::new());
}
