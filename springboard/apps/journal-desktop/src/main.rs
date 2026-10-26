//! Journal as an installed BifrOSt app: private dated entries with text and photos. The timeline on the left lists
//! the entries newest first; the editor on the right holds the text of the selected entry, its bookmark and its
//! photos. Everything is kept in the app's data directory (see `store`).

mod layout;
mod model;
mod store;

use bifrost_app::controls::text::TextBuffer;
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT,
    MENU_HIGHLIGHT_TEXT, SEPARATOR_C, SYSTEM_ORANGE, SYSTEM_RED,
};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::widgets::{self, Material};
use bifrost_app::controls::{Node, Role};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use layout::{Hit, Layout, Rect};
use model::{Day, Journal};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Corner radius of buttons, fields and rows (points). The design rules keep corners at 6 points or less.
const RADIUS_PT: f32 = m::ICON_RADIUS;
/// Distance between lines of the entry text (points).
const LINE_PT: f32 = 18.0;
/// How far a touch must move before it scrolls the timeline instead of selecting a row (points).
const DRAG_PT: f32 = 6.0;

/// Ids of the published controls (`App::controls`).
const CTL_SEARCH: u64 = 1;
const CTL_FILTER: u64 = 2;
const CTL_NEW: u64 = 3;
const CTL_BOOKMARK: u64 = 4;
const CTL_DELETE: u64 = 5;
const CTL_PHOTO_FIELD: u64 = 6;
const CTL_ATTACH: u64 = 7;

/// `AppMenuItem` ids for `menu_action` (0 is reserved for separators).
const MENU_NEW: u32 = 1;
const MENU_BOOKMARK: u32 = 2;
const MENU_DELETE: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Editor,
    Search,
    PhotoField,
}

/// A touch that started on the timeline and may still become a tap or a scroll.
#[derive(Clone, Copy, Debug)]
struct Press {
    y: f32,
    top: usize,
    row: Option<usize>,
    dragged: bool,
}

struct JournalApp {
    journal: Journal,
    data_dir: PathBuf,
    loaded: bool,
    editor: TextBuffer,
    search: TextBuffer,
    photo_path: TextBuffer,
    focus: Focus,
    /// Timeline index of the first visible row.
    top: usize,
    /// Rows of the timeline that fit on screen, from the last layout.
    visible: usize,
    press: Option<Press>,
    status: String,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Window size in points, from the configuration the shell sent.
fn layout_of(ctx: &Ctx) -> Layout {
    let scale = ctx.ui.pt(1.0).max(0.001);
    Layout::new(ctx.cfg.w as f32 / scale, ctx.cfg.h as f32 / scale)
}

fn to_points(ctx: &Ctx, x: f32, y: f32) -> (f32, f32) {
    let scale = ctx.ui.pt(1.0).max(0.001);
    (x / scale, y / scale)
}

/// A layout rectangle in device pixels.
fn px(ui: &Ui, r: Rect) -> RectF {
    RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h))
}

/// A layout rectangle as the control type the widgets take (points).
fn controls_rect(r: Rect) -> bifrost_app::controls::Rect {
    bifrost_app::controls::Rect { x: r.x, y: r.y, w: r.w, h: r.h }
}

/// Draws one line of text with its top-left corner at (`x`, `y`) in points.
fn text_at(c: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, colour: u32, x: f32, y: f32) {
    let img = ui.text(text, pt, weight, colour);
    widgets::blit(c, &img, ui.pt(x) as i32, ui.pt(y) as i32);
}

/// Draws text centred in a rectangle given in points.
fn text_centred(c: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, colour: u32, r: Rect) {
    let img = ui.text(text, pt, weight, colour);
    let x = ui.pt(r.x + r.w / 2.0) - img.w as f32 / 2.0;
    let y = ui.pt(r.y + r.h / 2.0) - img.h as f32 / 2.0;
    widgets::blit(c, &img, x as i32, y as i32);
}

/// A filled button or field plate with flat corners.
fn plate(c: &mut dyn Canvas, ui: &Ui, r: Rect, colour: u32) {
    c.fill_rrect(px(ui, r), ui.pt(RADIUS_PT), colour);
}

/// The text to show in an editable field: the text, with a caret when the field has focus.
fn with_caret(buffer: &TextBuffer, focused: bool) -> String {
    if !focused {
        return buffer.text().to_string();
    }
    let text = buffer.text();
    let mut at = buffer.caret().min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}|{}", &text[..at], &text[at..])
}

/// The text of an entry as shown in the timeline: its first non-empty line.
fn preview(text: &str) -> &str {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// Text typed into a field: control characters are dropped.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

impl JournalApp {
    fn new() -> Self {
        JournalApp {
            journal: Journal::default(),
            data_dir: PathBuf::new(),
            loaded: false,
            editor: TextBuffer::new(),
            search: TextBuffer::new(),
            photo_path: TextBuffer::new(),
            focus: Focus::Editor,
            top: 0,
            visible: 0,
            press: None,
            status: String::new(),
        }
    }

    fn timeline(&self) -> Vec<u64> {
        self.journal.timeline()
    }

    fn selected_entry(&self) -> Option<&model::Entry> {
        self.journal.selected.and_then(|id| self.journal.entry(id))
    }

    /// Writes the journal file. A failure is shown in the status line and the change stays in memory.
    fn save(&mut self) -> bool {
        match store::save(&self.data_dir, &self.journal) {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("Could not save the journal: {e}");
                false
            }
        }
    }

    /// Puts the selected entry's text into the editor.
    fn load_editor(&mut self) {
        let text = self.selected_entry().map(|e| e.text.clone()).unwrap_or_default();
        self.editor.set_text(&text);
    }

    fn select(&mut self, id: Option<u64>) {
        if self.journal.select(id) {
            self.load_editor();
            self.keep_selection_visible();
        }
    }

    /// Scrolls the timeline so the selected row is on screen.
    fn keep_selection_visible(&mut self) {
        let Some(id) = self.journal.selected else { return };
        let Some(at) = self.timeline().iter().position(|e| *e == id) else { return };
        let visible = self.visible;
        if at < self.top {
            self.top = at;
        } else if visible > 0 && at >= self.top + visible {
            self.top = at + 1 - visible;
        }
    }

    /// Re-applies the search and the bookmarks filter after either changed.
    fn refilter(&mut self) {
        self.journal.query = self.search.text().to_string();
        self.journal.reconcile_selection();
        self.top = 0;
        self.load_editor();
        self.keep_selection_visible();
    }

    fn clamp_top(&mut self, visible: usize) {
        let count = self.timeline().len();
        self.top = self.top.min(count.saturating_sub(visible));
    }

    fn new_entry(&mut self) {
        // a new entry must be visible, so the search and the filter are cleared
        self.search.set_text("");
        self.journal.query.clear();
        self.journal.bookmarks_only = false;
        let now = now_secs();
        self.journal.new_entry(Day::from_unix(now), now);
        self.top = 0;
        self.load_editor();
        self.keep_selection_visible();
        self.focus = Focus::Editor;
        self.status = "New entry for today.".into();
        self.save();
    }

    /// The header buttons, the search and filter fields and the photo bar as controls, in points. The header buttons
    /// sit in the top band of the full-size window, where the shell moves the window on a press unless the press lands
    /// on a published control: without these nodes New Entry never received its click.
    fn control_nodes(&self, layout: &Layout) -> Vec<Node> {
        let has = self.journal.selected.is_some();
        let bookmarked = self.selected_entry().is_some_and(|e| e.bookmarked);
        let [new, bookmark, delete] = layout.buttons();
        let node = |id: u64, role: Role, label: &str, value: String, r: Rect, enabled: bool| Node {
            id,
            role,
            label: label.into(),
            value,
            bounds: controls_rect(r),
            enabled,
        };
        vec![
            node(CTL_SEARCH, Role::TextField, "Search", self.search.text().to_string(), layout.search(), true),
            node(CTL_FILTER, Role::Button, if self.journal.bookmarks_only { "Showing Bookmarks" } else { "All Entries" }, String::new(), layout.filter(), true),
            node(CTL_NEW, Role::Button, "New Entry", String::new(), new, true),
            node(CTL_BOOKMARK, Role::Button, if bookmarked { "Bookmarked" } else { "Bookmark" }, String::new(), bookmark, has),
            node(CTL_DELETE, Role::Button, "Delete", String::new(), delete, has),
            node(CTL_PHOTO_FIELD, Role::TextField, "Photo path", self.photo_path.text().to_string(), layout.photo_field(), true),
            node(CTL_ATTACH, Role::Button, "Attach Photo", String::new(), layout.attach(), has),
        ]
    }

    /// A published control was pressed (accessibility, `bifrost pc midget tap-window`).
    fn activate(&mut self, id: u64) -> bool {
        match id {
            CTL_SEARCH => self.focus = Focus::Search,
            CTL_FILTER => {
                self.journal.bookmarks_only = !self.journal.bookmarks_only;
                self.refilter();
            }
            CTL_NEW => self.new_entry(),
            CTL_BOOKMARK => self.toggle_bookmark(),
            CTL_DELETE => self.delete_selected(),
            CTL_PHOTO_FIELD => self.focus = Focus::PhotoField,
            CTL_ATTACH => self.attach_photo(),
            _ => return false,
        }
        true
    }

    /// The File menu; it merges into the shell's File menu. Cmd+N reaches the app only through an item with that key.
    fn file_menu(&self) -> AppMenu {
        let has = self.journal.selected.is_some();
        let bookmarked = self.selected_entry().is_some_and(|e| e.bookmarked);
        AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(MENU_NEW, "New Entry").key("cmd+n"),
                AppMenuItem::new(MENU_BOOKMARK, if bookmarked { "Remove Bookmark" } else { "Bookmark Entry" }).key("cmd+d").enabled(has),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_DELETE, "Delete Entry").key("cmd+backspace").enabled(has),
            ],
        }
    }

    fn toggle_bookmark(&mut self) {
        let Some(id) = self.journal.selected else { return };
        if let Some(on) = self.journal.toggle_bookmark(id) {
            self.status = if on { "Bookmarked.".into() } else { "Bookmark removed.".into() };
            self.journal.reconcile_selection();
            self.load_editor();
            self.save();
        }
    }

    fn delete_selected(&mut self) {
        let Some(id) = self.journal.selected else { return };
        let Some(removed) = self.journal.remove(id) else { return };
        self.load_editor();
        self.keep_selection_visible();
        if self.save() {
            // the photos go only once the entry without them is saved
            for name in &removed.photos {
                let _ = store::delete_photo(&self.data_dir, name);
            }
            self.status = "Entry deleted.".into();
        }
    }

    fn attach_photo(&mut self) {
        let path = self.photo_path.text().trim().to_string();
        if path.is_empty() {
            self.status = "Type the path of an image file first.".into();
            return;
        }
        let Some(id) = self.journal.selected else {
            self.status = "Choose or make an entry first.".into();
            return;
        };
        let Some(ext) = model::file_extension(&path) else {
            self.status = "That file has no image extension.".into();
            return;
        };
        let Some(name) = self.journal.photo_name(id, &ext) else {
            self.status = "That is not a supported image type.".into();
            return;
        };
        match store::import_photo(&self.data_dir, Path::new(&path), &name) {
            Ok(_) => {
                self.journal.add_photo(id, &name, now_secs());
                self.photo_path.set_text("");
                if self.save() {
                    self.status = format!("Added photo {name}.");
                }
            }
            Err(e) => self.status = format!("Could not copy that file: {e}"),
        }
    }

    fn edit_text(&mut self, text: &str) -> bool {
        if self.journal.selected.is_none() {
            return false;
        }
        let _ = self.editor.insert_text(text);
        self.store_editor_text();
        true
    }

    fn store_editor_text(&mut self) {
        if let Some(id) = self.journal.selected {
            let text = self.editor.text().to_string();
            self.journal.set_text(id, &text, now_secs());
            self.save();
        }
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        if let KeyInput::Code(9) = key {
            self.focus = match self.focus {
                Focus::Editor => Focus::Search,
                Focus::Search => Focus::PhotoField,
                Focus::PhotoField => Focus::Editor,
            };
            return true;
        }
        match self.focus {
            Focus::Editor => match key {
                KeyInput::Text(t) => self.edit_text(&printable(t)),
                KeyInput::Code(13) => self.edit_text("\n"),
                KeyInput::Code(8) => {
                    if self.journal.selected.is_none() {
                        return false;
                    }
                    let _ = self.editor.delete_backward();
                    self.store_editor_text();
                    true
                }
                KeyInput::Code(37) => self.move_editor(|e| e.move_char(false, false)),
                KeyInput::Code(39) => self.move_editor(|e| e.move_char(true, false)),
                KeyInput::Code(38) => self.move_editor(|e| e.move_vertical(false, false)),
                KeyInput::Code(40) => self.move_editor(|e| e.move_vertical(true, false)),
                _ => false,
            },
            Focus::Search => match key {
                KeyInput::Text(t) => {
                    let _ = self.search.insert_text(&printable(t));
                    self.refilter();
                    true
                }
                KeyInput::Code(8) => {
                    let _ = self.search.delete_backward();
                    self.refilter();
                    true
                }
                KeyInput::Code(1 | 27) => {
                    self.search.set_text("");
                    self.refilter();
                    true
                }
                _ => false,
            },
            Focus::PhotoField => match key {
                KeyInput::Text(t) => {
                    let _ = self.photo_path.insert_text(&printable(t));
                    true
                }
                KeyInput::Code(8) => {
                    let _ = self.photo_path.delete_backward();
                    true
                }
                KeyInput::Code(13) => {
                    self.attach_photo();
                    true
                }
                _ => false,
            },
        }
    }

    fn move_editor(&mut self, f: impl FnOnce(&mut TextBuffer)) -> bool {
        if self.journal.selected.is_none() {
            return false;
        }
        f(&mut self.editor);
        true
    }

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let (px_, py) = to_points(ctx, x, y);
        let layout = layout_of(ctx);
        let timeline = self.timeline();
        let visible = layout.visible_rows();
        self.visible = visible;
        match kind {
            TouchKind::Down => {
                let hit = layout.hit(px_, py, self.top, timeline.len());
                self.press = None;
                match hit {
                    Hit::Row(index) => {
                        self.press = Some(Press { y: py, top: self.top, row: Some(index), dragged: false });
                        true
                    }
                    Hit::Search => {
                        self.focus = Focus::Search;
                        true
                    }
                    Hit::Filter => {
                        self.journal.bookmarks_only = !self.journal.bookmarks_only;
                        self.refilter();
                        true
                    }
                    Hit::New => {
                        self.new_entry();
                        true
                    }
                    Hit::Bookmark => {
                        self.toggle_bookmark();
                        true
                    }
                    Hit::Delete => {
                        self.delete_selected();
                        true
                    }
                    Hit::Editor => {
                        self.focus = Focus::Editor;
                        true
                    }
                    Hit::PhotoField => {
                        self.focus = Focus::PhotoField;
                        true
                    }
                    Hit::Attach => {
                        self.attach_photo();
                        true
                    }
                    Hit::None => false,
                }
            }
            TouchKind::Move => {
                let Some(press) = self.press.as_mut() else { return false };
                let dy = press.y - py;
                if dy.abs() > DRAG_PT {
                    press.dragged = true;
                }
                if press.dragged {
                    let top = (press.top as f32 + dy / layout::ROW_H_PT).max(0.0) as usize;
                    self.top = top;
                    self.clamp_top(visible);
                    return true;
                }
                false
            }
            TouchKind::Up | TouchKind::Cancel => {
                let Some(press) = self.press.take() else { return false };
                if !press.dragged {
                    if let Some(index) = press.row {
                        if let Some(id) = timeline.get(index).copied() {
                            self.focus = Focus::Editor;
                            self.select(Some(id));
                            return true;
                        }
                    }
                }
                true
            }
        }
    }

    fn draw_navigation(&self, c: &mut dyn Canvas, ui: &Ui, layout: &Layout, timeline: &[u64]) {
        let sidebar = Rect { x: 0.0, y: 0.0, w: layout::SIDEBAR_PT, h: layout.h };
        widgets::material(c, ui, controls_rect(sidebar), 0.0, Material::Sidebar);

        // search field
        let search = layout.search();
        plate(c, ui, search, FIELD_BG.now());
        let shown = with_caret(&self.search, self.focus == Focus::Search);
        if self.search.text().is_empty() && self.focus != Focus::Search {
            text_at(c, ui, "Search", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), search.x + 8.0, search.y + 6.0);
        } else {
            let fit = ui.fit(&shown, MAC_BODY_PT, ui.pt(search.w - 16.0));
            text_at(c, ui, &fit, MAC_BODY_PT, Weight::Regular, LABEL.now(), search.x + 8.0, search.y + 6.0);
        }

        // bookmarks filter
        let filter = layout.filter();
        let on = self.journal.bookmarks_only;
        plate(c, ui, filter, if on { SYSTEM_ORANGE } else { FIELD_BG.now() });
        let label = if on { "Showing Bookmarks" } else { "All Entries" };
        text_centred(c, ui, label, MAC_FOOTNOTE_PT, Weight::Semibold, if on { MENU_HIGHLIGHT_TEXT } else { LABEL.now() }, filter);

        if timeline.is_empty() {
            let message = if self.journal.entries.is_empty() {
                "No entries yet. Choose New Entry to start."
            } else {
                "No entries match."
            };
            text_at(c, ui, message, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 16.0, layout::LIST_TOP_PT + 12.0);
            return;
        }

        let selected = self.journal.selected.and_then(|id| timeline.iter().position(|entry| *entry == id));
        let items = timeline.iter().enumerate().filter_map(|(i, id)| {
            let entry = self.journal.entry(*id)?;
            let mut subtitle = if preview(&entry.text).is_empty() { "No text".to_string() } else { preview(&entry.text).to_string() };
            if entry.bookmarked { subtitle.push_str(" · Bookmarked"); }
            if !entry.photos.is_empty() {
                subtitle.push_str(&format!(" · {} photo{}", entry.photos.len(), if entry.photos.len() == 1 { "" } else { "s" }));
            }
            Some(SidebarItem::row(i as u32 + 1, &entry.day.long()).with_subtitle(&subtitle))
        }).collect();
        let mut list = SourceList::new(items, layout::ROW_H_PT, layout::ROW_H_PT);
        list.set_selected(selected.map(|i| i as u32 + 1));
        widgets::source_list(
            c,
            ui,
            bifrost_app::controls::Rect { x: 0.0, y: layout::LIST_TOP_PT, w: layout::SIDEBAR_PT, h: (layout.h - layout::LIST_TOP_PT).max(0.0) },
            &list,
            self.top as f32 * layout::ROW_H_PT,
            None,
        );
    }

    fn draw_header(&self, c: &mut dyn Canvas, ui: &Ui, layout: &Layout) {
        let title = match self.selected_entry() {
            Some(entry) => entry.day.long(),
            None => "Journal".to_string(),
        };
        let width = ui.pt(layout.pane_w() - 2.0 * layout::PAD_PT - 2.0 * layout::BUTTON_W_PT - 2.0 * layout::BUTTON_GAP_PT - 24.0);
        let fit = ui.fit(&title, MAC_TITLE3_PT, width.max(0.0));
        text_at(c, ui, &fit, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), layout::SIDEBAR_PT + layout::PAD_PT, 14.0);

        let [new, bookmark, delete] = layout.buttons();
        let has = self.journal.selected.is_some();
        let bookmarked = self.selected_entry().is_some_and(|e| e.bookmarked);
        plate(c, ui, new, FIELD_BG.now());
        text_centred(c, ui, "New Entry", MAC_FOOTNOTE_PT, Weight::Semibold, LABEL.now(), new);
        plate(c, ui, bookmark, if bookmarked { SYSTEM_ORANGE } else { FIELD_BG.now() });
        let label = if bookmarked { "Bookmarked" } else { "Bookmark" };
        let ink = if has { LABEL.now() } else { LABEL_2.now() };
        text_centred(c, ui, label, MAC_FOOTNOTE_PT, Weight::Semibold, if bookmarked { MENU_HIGHLIGHT_TEXT } else { ink }, bookmark);
        plate(c, ui, delete, FIELD_BG.now());
        text_centred(c, ui, "Delete", MAC_FOOTNOTE_PT, Weight::Semibold, if has { SYSTEM_RED } else { LABEL_2.now() }, delete);
    }

    fn draw_editor(&self, c: &mut dyn Canvas, ui: &Ui, layout: &Layout) {
        let area = layout.editor();
        if self.journal.selected.is_none() {
            text_at(c, ui, "Make an entry with New Entry to write about today.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), area.x, area.y);
            return;
        }
        let text = with_caret(&self.editor, self.focus == Focus::Editor);
        let max = ui.pt(area.w);
        let clip = px(ui, area);
        c.push_clip(clip, 0.0);
        let mut y = area.y;
        for paragraph in text.split('\n') {
            let mut lines = ui.wrap(paragraph, MAC_BODY_PT, Weight::Regular, max);
            if lines.is_empty() {
                lines.push(String::new());
            }
            for line in lines {
                text_at(c, ui, &line, MAC_BODY_PT, Weight::Regular, LABEL.now(), area.x, y);
                y += LINE_PT;
            }
        }
        c.pop_clip();
    }

    fn draw_photo_bar(&self, c: &mut dyn Canvas, ui: &Ui, layout: &Layout) {
        let bar = layout.photo_bar();
        c.fill_rect(RectF::new(ui.pt(bar.x), ui.pt(bar.y), ui.pt(bar.w), ui.pt(1.0)), SEPARATOR_C.now());
        let names = self.selected_entry().map(|e| e.photos.join(", ")).unwrap_or_default();
        let list = if names.is_empty() { "No photos yet.".to_string() } else { format!("Photos: {names}") };
        let width = ui.pt(bar.w - 2.0 * layout::PAD_PT);
        text_at(c, ui, &ui.fit(&list, MAC_SUBHEAD_PT, width), MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), bar.x + layout::PAD_PT, bar.y + 10.0);

        let field = layout.photo_field();
        plate(c, ui, field, FIELD_BG.now());
        let focused = self.focus == Focus::PhotoField;
        if self.photo_path.text().is_empty() && !focused {
            text_at(c, ui, "Path to an image file (jpg, png, gif, webp, heic)", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), field.x + 8.0, field.y + 7.0);
        } else {
            let shown = with_caret(&self.photo_path, focused);
            text_at(c, ui, &ui.fit(&shown, MAC_BODY_PT, ui.pt(field.w - 16.0)), MAC_BODY_PT, Weight::Regular, LABEL.now(), field.x + 8.0, field.y + 6.0);
        }

        let attach = layout.attach();
        plate(c, ui, attach, FIELD_BG.now());
        text_centred(c, ui, "Attach Photo", MAC_FOOTNOTE_PT, Weight::Semibold, LABEL.now(), attach);

        if !self.status.is_empty() {
            text_at(c, ui, &ui.fit(&self.status, MAC_FOOTNOTE_PT, width), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), bar.x + layout::PAD_PT, bar.y + 72.0);
        }
    }
}

impl App for JournalApp {
    fn id(&self) -> &str {
        "is.olibuijr.journal.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let visible = layout_of(ctx).visible_rows();
        self.visible = visible;
        if !self.loaded {
            self.data_dir = ctx.data_dir.clone();
            self.journal = store::load(&self.data_dir);
            self.loaded = true;
            let first = self.timeline().first().copied();
            self.select(first);
        }
        self.clamp_top(visible);
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.control_nodes(&layout_of(ctx))
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        vec![self.file_menu()]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.new_entry(),
            MENU_BOOKMARK => self.toggle_bookmark(),
            MENU_DELETE => self.delete_selected(),
            _ => return false,
        }
        true
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Activate(id) => self.activate(id),
            Input::Touch { kind, x, y, .. } => self.touch(ctx, kind, x, y),
            Input::Key(key) => self.key(&key),
            _ => false,
        }
    }

    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (w, h) = canvas.size();
        let layout = layout_of(ctx);
        self.visible = layout.visible_rows();
        let timeline = self.timeline();
        let ui = &ctx.ui;
        // an opaque ground first: a pixel the app leaves transparent shows the window behind and, because the shell
        // shapes a window to what it drew, passes clicks through to it (New Entry never got the click)
        canvas.fill_rect(RectF::new(0.0, 0.0, w as f32, h as f32), CONTENT_BG.now());
        self.draw_navigation(canvas, ui, &layout, &timeline);
        canvas.fill_rect(RectF::new(ui.pt(layout::SIDEBAR_PT) - 1.0, 0.0, 1.0, h as f32), SEPARATOR_C.now());
        self.draw_header(canvas, ui, &layout);
        self.draw_editor(canvas, ui, &layout);
        self.draw_photo_bar(canvas, ui, &layout);
        let mut d = Damage::new(w as usize, h as usize);
        d.all();
        d
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(JournalApp::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_in(name: &str) -> JournalApp {
        let dir = std::env::temp_dir().join(format!("bifrost-journal-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = JournalApp::new();
        app.data_dir = dir;
        app.loaded = true;
        app
    }

    #[test]
    fn new_entry_shows_in_the_timeline_and_is_selected() {
        let mut app = app_in("new");
        assert!(app.timeline().is_empty());
        app.new_entry();
        let timeline = app.timeline();
        assert_eq!(timeline.len(), 1);
        assert_eq!(app.journal.selected, Some(timeline[0]));
        assert!(app.selected_entry().is_some_and(|e| e.text.is_empty()));
        // a second entry is added, not merged, and becomes the selection
        app.new_entry();
        assert_eq!(app.timeline().len(), 2);
        let _ = std::fs::remove_dir_all(&app.data_dir);
    }

    #[test]
    fn new_entry_clears_a_search_and_the_bookmarks_filter_that_would_hide_it() {
        let mut app = app_in("filter");
        app.search.set_text("harbour");
        app.journal.query = "harbour".into();
        app.journal.bookmarks_only = true;
        app.new_entry();
        assert_eq!(app.timeline().len(), 1);
        assert!(app.search.text().is_empty());
        assert!(!app.journal.bookmarks_only);
        let _ = std::fs::remove_dir_all(&app.data_dir);
    }

    #[test]
    fn new_entry_is_reachable_by_click_control_and_cmd_n() {
        let layout = Layout::new(1280.0, 720.0);
        let mut app = app_in("paths");
        // the click: the published control covers the drawn button, and that point hit-tests as New
        let nodes = app.control_nodes(&layout);
        let node = nodes.iter().find(|n| n.id == CTL_NEW).expect("a New Entry control");
        let b = node.bounds;
        assert_eq!(layout.hit(b.x + b.w / 2.0, b.y + b.h / 2.0, 0, 0), Hit::New);
        assert!(node.enabled);
        assert!(b.y + b.h <= layout::BAND_PT, "in the band the shell moves the window unless the control is published");
        assert!(app.activate(CTL_NEW));
        assert_eq!(app.timeline().len(), 1);
        // Cmd+N: File > New Entry carries the key
        let file = app.file_menu();
        assert_eq!(file.title, "File");
        let item = file.items.iter().find(|i| i.id == MENU_NEW).expect("File > New Entry");
        assert_eq!(item.title, "New Entry");
        assert_eq!(item.key, "cmd+n");
        let _ = std::fs::remove_dir_all(&app.data_dir);
    }

    #[test]
    fn every_header_button_is_published() {
        let layout = Layout::new(1280.0, 720.0);
        let app = app_in("published");
        let nodes = app.control_nodes(&layout);
        let [new, bookmark, delete] = layout.buttons();
        for (id, rect) in [(CTL_NEW, new), (CTL_BOOKMARK, bookmark), (CTL_DELETE, delete), (CTL_SEARCH, layout.search())] {
            let node = nodes.iter().find(|n| n.id == id).expect("published");
            assert_eq!((node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h), (rect.x, rect.y, rect.w, rect.h));
        }
        // with no entry, Bookmark and Delete are disabled but New Entry is not
        assert!(nodes.iter().find(|n| n.id == CTL_NEW).unwrap().enabled);
        assert!(!nodes.iter().find(|n| n.id == CTL_DELETE).unwrap().enabled);
    }
}
