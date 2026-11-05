//! TextEdit as an installed BifrOSt app: plain and rich text documents, opened and saved as RTF, HTML or plain text
//! through the shell's document panels, with character and paragraph formatting, fonts, find and replace, spelling, a
//! tab ruler, printing with page setup, a word count, and undo and redo.
//!
//! The document model is in `text.rs`, the file formats in `rtf.rs`, `html.rs` and `document.rs`, the line breaking in
//! `layout.rs`, the spelling in `spell.rs`, the PDF for printing in `print.rs`, and the window in `view.rs`. This file
//! wires them to the SDK: the document controller, menus, keys, touches and the canvas.

mod doc_windows;
mod docx;
mod encoding;
mod family_font;
mod document;
mod font_panel;
mod format_bar;
mod html;
mod layout;
mod links;
mod odt;
mod office;
mod prefs;
mod print;
mod properties;
mod replace;
mod rtf;
mod rtfd;
mod select;
mod spell;
mod text;
mod view;
mod webarchive;
mod xml;
mod zipdoc;

use bifrost_app::controls::undo::ChangeCount;
use bifrost_app::controls::nav::ToolbarLayout;
use bifrost_app::controls::Node;
use bifrost_app::document::{is_menu_id, item, AlertKey, DocId, DocumentController, DocumentHost, Notice};
use bifrost_app::kit::canvas::Canvas;
use bifrost_app::pasteboard::NativePasteboard;
use bifrost_app::kit::ui::Ui;
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::print::{Orientation, PrintError, PrintInfo, PrintOperation};
use bifrost_app::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::proto::{ContextItem, CHORD_ALT, CHORD_CTRL, CHORD_FLAG, CHORD_META, CHORD_SHIFT};
use bifrost_app::services::ServiceCall;
use bifrost_core::json::Json;
use bifrost_app::dnd::{self, dragop, DragPhase};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use document::TextDocument;
use encoding::Encoding;
use layout::Layout;
use prefs::Options;
use spell::Spelling;
use std::ops::Range;
use text::{Align, Cell, Kind, Style, Transform, OBJECT};
use bifrost_app::graphics_imageio;
use view::{Action, Button, Focus, Hit};

/// A millisecond clock for type-select in the font panel (keys typed within a second continue a family's name).
fn clock_ms() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// The find bar highlights at most this many matches (a long document full of one letter stays quick to draw).
const MAX_HIGHLIGHTS: usize = 2000;

/// The layout of the document as last drawn, and the misspelled words of its text, kept until the text, the width or
/// the document changes.
struct Cache {
    doc: DocId,
    revision: u64,
    width: f32,
    zoom: f32,
    /// the attachments' aspect ratios and decoded pictures, indexed like the attachments
    art: view::Art,
    /// the document's cells at the zoom: sizes scaled, so the layout, the caret and the drawing agree
    cells: Vec<Cell>,
    layout: Layout,
    misspelled: Vec<Range<usize>>,
}

struct TextEditApp {
    docs: DocumentController<TextDocument>,
    /// the find bar: whether it is open, its two fields, and which one has the keyboard
    find_open: bool,
    find: String,
    replace: String,
    focus: Focus,
    /// the whole text of the focused find field is selected (Cmd+A, reopening the bar, Tab into it): typing replaces it
    find_sel: bool,
    /// the status row's note (a search that found nothing, the count of replacements), until the next edit
    note: Option<String>,
    /// how far the page is scrolled, in points, and whether the caret should be brought into view
    scroll: f32,
    follow: bool,
    /// a press in the page is extending a selection: by what unit (a caret, a word after a double click, a paragraph after a
    /// triple click), from which range, and the presses that count the clicks
    drag: bool,
    drag_unit: select::Unit,
    drag_origin: Range<usize>,
    clicks: select::Clicks,
    /// bumped by every change to a document, so the cached layout is rebuilt
    revision: u64,
    cache: Option<Cache>,
    /// the spell checker, and the page setup that Print uses
    spelling: Spelling,
    print_info: PrintInfo,
    /// the settings the menus toggle (kept in the preferences), and the view zoom
    opts: Options,
    zoom: f32,
    /// Copy Style: the character formatting waiting to be pasted
    copied_style: Option<Style>,
    /// Edit > Complete: the word being completed, so a second Complete offers the next candidate
    completion: Option<Completion>,
    /// File > Export as PDF: the save panel's request, until its file is written
    export_request: Option<u32>,
    /// the open panel of Insert > Image, and the drag sessions this window accepted images from (kept after the drop,
    /// so the files that arrive after `Perform` still land)
    image_request: Option<u32>,
    /// Format > Make Plain Text is asking to remove the attachments first
    plain_sheet: bool,
    /// the confirmation's button that has the focus ring (Tab moves it); `None` until Tab
    plain_focus: Option<u32>,
    drop_sessions: Vec<u32>,
    /// File > Show Properties: the document being edited and the panel's draft, while the panel is open
    props: Option<(DocId, properties::Panel)>,
    /// Edit > Add Link: the address being typed for a range of text, while the panel is open
    link: Option<LinkSheet>,
    /// Edit > Find > Select Line: the line number or range being typed, while the panel is open
    line: Option<String>,
    /// Edit > Spelling and Grammar > Show Spelling and Grammar: the panel is open over the page
    spelling_panel: bool,
    /// Preferences: the settings panel is open over the page, and the encodings list panel
    settings: bool,
    encodings_panel: bool,
    /// Format > Text > Spacing: the panel's fields, while it is open
    spacing_sheet: Option<SpacingSheet>,
    /// Format > Font > Show Fonts: the font panel (its family list scroll), and Show Colors: the colour panel
    fonts: Option<font_panel::Fonts>,
    colors_panel: bool,
    /// Cut and Copy keep the copied cells with their formatting, for a paste of the same text. The text is the clipboard
    /// text they were copied with.
    rich_clip: Option<(String, Vec<Cell>)>,
    /// Format > Text > Copy Ruler: the tab stop distance copied, for Paste Ruler
    ruler: Option<f32>,
    /// the document windows (`doc_windows`): the window being handled, its document and size in px (None for the main
    /// window, which is the configured size), the other windows' parked state, the document notices not yet placed in
    /// windows, and whether the input being handled was already routed to its window
    current: bifrost_app::app_windows::WindowId,
    current_doc: Option<DocId>,
    size: Option<(u32, u32)>,
    views: Vec<doc_windows::View>,
    notices: Vec<Notice>,
    routed: bool,
    /// The text state last reported to the shell (`ctx.text_state`): the active document's selection and caret, which
    /// the Services submenu and Look Up read, as an SDK text field reports its own.
    text_published: Option<bifrost_app::proto::TextState>,
    /// The caret rectangle last reported to the shell (`ctx.caret_rect`) and the window it is in: Look Up opens under
    /// it, an input method's candidates beside it.
    caret_published: Option<(bifrost_app::app_windows::WindowId, (f32, f32, f32, f32))>,
}

/// What a press in the page did.
enum PageDown {
    /// nothing to place the caret in (no document)
    Ignored,
    /// the caret or the selection moved
    Placed,
    /// a press on a link in a document that cannot be edited: open this address
    Link(String),
}

/// The link panel: which text it links, the address typed so far, and whether that text already links somewhere (the
/// panel then offers to remove the link).
struct LinkSheet {
    doc: DocId,
    range: Range<usize>,
    url: String,
    had_link: bool,
}

/// The Spacing panel: the line spacing, the space before and the space after as typed, and the field with the keyboard.
struct SpacingSheet {
    fields: [String; 3],
    focus: usize,
}

impl SpacingSheet {
    /// The fields' labels, in order.
    const LABELS: [&'static str; 3] = ["Line", "Before", "After"];
}

/// A completion in progress: the typed prefix, the cells of the word as completed, and which candidate it shows.
struct Completion {
    prefix: String,
    range: Range<usize>,
    index: usize,
}

/// The document of the active window, if any.
/// The data of the first attachment among the cells, if any (a copy of a picture takes its file's bytes).
fn picture_of(doc: &TextDocument, cells: &[Cell]) -> Option<Vec<u8>> {
    cells.iter().filter(|c| c.ch == OBJECT).find_map(|c| doc.text.attachment_of(&c.style)).map(|a| a.data.clone()).filter(|d| !d.is_empty())
}

/// Puts a picture on the general pasteboard as its own type (PNG or JPEG); other formats are not put on it.
fn put_picture(ctx: &Ctx, data: &[u8]) {
    let Some(format) = graphics_imageio::detect(data) else { return };
    let uti = format.type_identifier();
    if matches!(uti, "public.png" | "public.jpeg") {
        NativePasteboard::general().set_data(ctx, data, uti);
    }
}

/// The picture on the general pasteboard (PNG or JPEG), if there is one.
fn picture_on_board() -> Option<Vec<u8>> {
    let board = NativePasteboard::general();
    let uti = board.available_type(&["public.png", "public.jpeg"])?;
    board.data(&uti).filter(|d| !d.is_empty())
}

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

/// The word the caret is in, or the selection when there is one: its cell range and its text. A selection that is not
/// one word (it holds a space, or nothing) has none.
fn word_here(doc: &TextDocument) -> Option<(Range<usize>, String)> {
    let range = if doc.text.has_selection() { doc.text.selection() } else { spell::word_at(doc.text.cells(), doc.text.head())? };
    let word: String = doc.text.cells()[range.clone()].iter().map(|c| c.ch).collect();
    (!word.is_empty() && !word.chars().any(char::is_whitespace)).then_some((range, word))
}

/// Characters of context reported on each side of the selection (`TextState::text` is a window of the document).
const TEXT_STATE_CONTEXT: usize = 400;

/// The document's selection as the text state an SDK text field reports (`ctx.text_state`): a window of the text around
/// the selection, at most `MAX_SURROUNDING` bytes, with the caret (`cursor`) and the selection's other end (`anchor`) as
/// byte offsets into it. A selection longer than the window is cut at the window's end.
fn text_state_of(doc: &TextDocument) -> bifrost_app::proto::TextState {
    let cells = doc.text.cells();
    let len = cells.len();
    let (anchor, head) = (doc.text.anchor().min(len), doc.text.head().min(len));
    let (from, to) = (anchor.min(head), anchor.max(head));
    let limit = bifrost_app::proto::MAX_SURROUNDING;
    let w = |i: usize| cells[i].ch.len_utf8();
    // the selection first, then context on both sides while it fits
    let mut used = 0;
    let mut hi = from;
    while hi < to && used + w(hi) <= limit {
        used += w(hi);
        hi += 1;
    }
    let (mut lo, mut end) = (from, hi);
    loop {
        let mut grew = false;
        if lo > 0 && from - lo < TEXT_STATE_CONTEXT && used + w(lo - 1) <= limit {
            lo -= 1;
            used += w(lo);
            grew = true;
        }
        if hi == to && end < len && end - to < TEXT_STATE_CONTEXT && used + w(end) <= limit {
            used += w(end);
            end += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    let text: String = cells[lo..end].iter().map(|c| c.ch).collect();
    let offset = |i: usize| cells[lo..i.clamp(lo, hi)].iter().map(|c| c.ch.len_utf8()).sum::<usize>() as u32;
    bifrost_app::proto::TextState { text, cursor: offset(head), anchor: offset(anchor), marked: None }
}

/// The cells with every size scaled by the view zoom; the text itself is unchanged.
fn zoomed(cells: &[Cell], zoom: f32) -> Vec<Cell> {
    cells
        .iter()
        .map(|c| {
            let mut c = *c;
            c.style.size *= zoom;
            c
        })
        .collect()
}

/// Whether an action changes the document's text or formatting, which Prevent Editing refuses.
fn edits_text(action: Action) -> bool {
    is_formatting(action)
        || matches!(
            action,
            Action::Undo
                | Action::Redo
                | Action::Cut
                | Action::Paste
                | Action::PasteMatch
                | Action::Delete
                | Action::Complete
                | Action::Transform(_)
                | Action::MakePlain
                | Action::MakeRich
                | Action::PasteRuler
                | Action::Direction(_)
        )
}

/// Whether an action changes how text looks, which only rich text does.
fn is_formatting(action: Action) -> bool {
    matches!(
        action,
        Action::Bold
            | Action::Italic
            | Action::Underline
            | Action::Bigger
            | Action::Smaller
            | Action::Font(_)
            | Action::Align(_)
            | Action::Color(_)
            | Action::Spacing(_)
            | Action::List(_)
            | Action::Strike
            | Action::Superscript
            | Action::Subscript
            | Action::Highlight(_)
            | Action::PasteStyle
            | Action::FontStyle(_)
            | Action::Size(_)
    )
}

impl TextEditApp {
    fn new() -> Self {
        TextEditApp {
            docs: DocumentController::new(),
            find_open: false,
            find: String::new(),
            replace: String::new(),
            focus: Focus::Body,
            find_sel: false,
            note: None,
            scroll: 0.0,
            follow: true,
            drag: false,
            drag_unit: select::Unit::Char,
            drag_origin: 0..0,
            clicks: select::Clicks::default(),
            revision: 0,
            cache: None,
            spelling: Spelling::new(),
            print_info: PrintInfo::default(),
            opts: Options::default(),
            zoom: 1.0,
            copied_style: None,
            completion: None,
            export_request: None,
            image_request: None,
            plain_sheet: false,
            plain_focus: None,
            drop_sessions: Vec::new(),
            text_published: None,
            caret_published: None,
            props: None,
            link: None,
            line: None,
            spelling_panel: false,
            settings: false,
            encodings_panel: false,
            fonts: None,
            colors_panel: false,
            spacing_sheet: None,
            rich_clip: None,
            ruler: None,
            current: bifrost_app::app_windows::WindowId::MAIN,
            current_doc: None,
            size: None,
            views: Vec::new(),
            notices: Vec::new(),
            routed: false,
        }
    }

    /// The documents a launch starts with: the untitled ones autosaved when the app last quit (Cmd+Q saves nothing else
    /// and asks nothing, so the text comes back edited, as the reference's Untitled documents do), or one new Untitled
    /// when there are none.
    fn start_documents(&mut self, host: &dyn DocumentHost) {
        let restored = self.docs.restore(host);
        if restored.is_empty() {
            let id = self.docs.new_document();
            self.apply_new_defaults(id);
        } else {
            self.after_documents(&restored);
        }
    }

    /// Writes the settings to the preferences.
    fn save_options(&self, ctx: &Ctx) {
        for (key, value) in self.opts.entries() {
            DocumentHost::set_pref(ctx, key, &value);
        }
    }

    /// A setting changed in the Preferences panel: it is saved, and the parts that read it directly take it now (the
    /// reader's markup choice, the spelling checker, and the page, which is laid out again).
    fn apply_options(&mut self, ctx: &Ctx) {
        document::set_markup_as_code(self.opts.markup_as_code);
        self.spelling.enabled = self.opts.spell_enabled;
        self.revision += 1;
        self.save_options(ctx);
    }

    /// Whether the active document is read-only; says so in the status row when it is.
    fn refuse_edit(&mut self) -> bool {
        if active_doc(&self.docs).is_some_and(|d| d.read_only) {
            self.note = Some("Editing is prevented. Choose Format > Allow Editing.".into());
            true
        } else {
            false
        }
    }

    /// The window's regions for the current window size, find bar and document. The ruler shows for rich text only.
    fn geometry(&self, ui: &Ui, width_px: u32, height_px: u32) -> view::Geometry {
        let rich = active_doc(&self.docs).is_none_or(|d| d.rich);
        let ruler = self.opts.show_ruler && active_doc(&self.docs).is_some_and(|d| d.rich);
        // a plain text document has no format bar (E2E9 11): the page starts under the title row
        view::geometry_with(width_px as f32 / ui.s, height_px as f32 / ui.s, self.find_open, ruler, self.opts.wrap_window, rich)
    }

    /// The format bar's status: what the document has at the caret, or in the first selected character.
    fn status(&self) -> view::Status {
        match active_doc(&self.docs) {
            Some(doc) => view::Status { style: doc.text.style(), align: doc.text.align_at(doc.text.head()), rich: doc.rich },
            None => view::Status { style: Style::default(), align: Align::Left, rich: false },
        }
    }

    /// The window's title: the document's name, with "Edited" after it while it has unsaved changes. The window has no
    /// title row of its own (the shell draws none for a full-size window), so the app draws the title in its toolbar.
    fn window_title(&self) -> String {
        self.docs.active().and_then(|id| self.docs.window_title(id)).unwrap_or_else(|| "TextEdit".to_string())
    }

    /// The find bar's buttons: none while it is closed.
    fn find_buttons(&self, ui: &Ui, geom: &view::Geometry) -> Vec<Button> {
        if self.find_open {
            view::find_buttons(ui, geom)
        } else {
            Vec::new()
        }
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

    /// A document was reverted: the page starts at the top. A created or opened one is placed in a window by
    /// `sync_windows`, whose page then starts at the top.
    /// Reports the active document's selection and caret to the shell when they changed (`ctx.text_state`), so the
    /// Services submenu lists the services for a selection and Look Up finds the word at the caret. No document reports
    /// an empty state.
    pub(crate) fn sync_text_state(&mut self, ctx: &Ctx) {
        let now = active_doc(&self.docs).map(text_state_of).unwrap_or_default();
        if self.text_published.as_ref() != Some(&now) {
            // an empty state clears the shell's caret too: the next caret is reported again
            if now == bifrost_app::proto::TextState::default() {
                self.caret_published = None;
            }
            ctx.text_state(now.clone());
            self.text_published = Some(now);
        }
    }

    fn after_documents(&mut self, notices: &[Notice]) {
        for n in notices {
            if matches!(n, Notice::Reverted(_)) {
                self.scroll = 0.0;
                self.follow = true;
                self.revision += 1;
            }
        }
        self.notices.extend(notices.iter().cloned());
    }

    /// Rebuilds the layout and the misspelled words of the active document when they are stale.
    fn ensure_layout(&mut self, ui: &Ui, width: f32) {
        let Some(id) = self.docs.active() else { return };
        let zoom = self.zoom;
        let fresh = self.cache.as_ref().is_some_and(|c| c.doc == id && c.revision == self.revision && c.width == width && c.zoom == zoom);
        if fresh {
            return;
        }
        let Some(doc) = self.docs.get(id) else { return };
        let art = view::art(doc.text.attachments());
        let measure = view::measure_in(ui, &art.aspects, width);
        let cells = doc.text.cells();
        let scaled = zoomed(cells, zoom);
        let layout = layout::layout_tabs(&scaled, width, &measure, doc.text.tab_width() * zoom);
        let misspelled = if self.spelling.enabled { self.spelling.misspelled(cells) } else { Vec::new() };
        // the measure borrows the pictures' sizes; it ends before the pictures move into the cache
        drop(measure);
        self.cache = Some(Cache { doc: id, revision: self.revision, width, zoom, art, cells: scaled, layout, misspelled });
    }

    /// Runs a text command on the active document. Returns whether the window needs a frame.
    fn perform(&mut self, ctx: &Ctx, action: Action) -> bool {
        // the standard editing commands belong to the field or sheet that has the keyboard, never to the document behind it
        if matches!(action, Action::Cut | Action::Copy | Action::Paste | Action::PasteMatch | Action::SelectAll | Action::Undo | Action::Redo) {
            let find_field = self.find_open && self.focus != Focus::Body;
            let sheet_open = self.link.is_some()
                || self.props.is_some()
                || self.line.is_some()
                || self.spacing_sheet.is_some()
                || self.settings
                || self.encodings_panel
                || self.spelling_panel
                || self.plain_sheet
                || self.close_alert_here();
            if find_field && !sheet_open && matches!(action, Action::Paste | Action::PasteMatch) {
                let line = clipboard::text().map(|s| s.lines().next().unwrap_or("").to_string()).unwrap_or_default();
                return self.type_into(&line);
            }
            // the find field's own Select All, Copy and Cut (QA W2-12: Cmd+A appended to the query instead)
            if find_field && !sheet_open && matches!(action, Action::SelectAll | Action::Copy | Action::Cut) {
                return self.find_field_command(action);
            }
            let panel_paste = matches!(action, Action::Paste | Action::PasteMatch)
                && (self.link.is_some() || self.props.is_some() || self.line.is_some() || self.spacing_sheet.is_some());
            if (find_field || sheet_open) && !panel_paste {
                return false;
            }
        }
        // a panel takes a paste as typing (the first line of the clipboard)
        let panel_open = self.link.is_some() || self.props.is_some() || self.line.is_some() || self.spacing_sheet.is_some();
        if matches!(action, Action::Paste | Action::PasteMatch) && panel_open {
            let line = clipboard::text().map(|s| s.lines().next().unwrap_or("").to_string()).unwrap_or_default();
            return self.type_into(&line);
        }
        if let Action::CloseAnswer(choice) = action {
            return self.answer_close(ctx, choice);
        }
        if matches!(action, Action::PlainDone) {
            self.plain_sheet = false;
            return self.make_plain_now();
        }
        if matches!(action, Action::PlainCancel) {
            let was = self.plain_sheet;
            self.plain_sheet = false;
            return was;
        }
        // a paste with no text on the clipboard takes the picture on the pasteboard, if there is one
        if matches!(action, Action::Paste) && clipboard::text().filter(|s| !s.is_empty()).is_none() {
            if let Some(data) = picture_on_board() {
                return self.insert_picture("Picture", data);
            }
        }
        match action {
            Action::Document(id) => {
                let Some(notices) = self.docs.menu_action(ctx, id) else { return false };
                // File > New follows the Preferences' document format
                if id == item::NEW {
                    for n in &notices {
                        if let Notice::Created(created) = n {
                            self.apply_new_defaults(*created);
                        }
                    }
                }
                // Open Recent on a document that is already open: its window comes forward
                if id >= item::RECENT {
                    if let Some(open) = self.docs.active().filter(|a| self.shown(*a)) {
                        self.raise_doc(ctx, open);
                    }
                }
                self.after_documents(&notices);
                return true;
            }
            Action::SaveAll => {
                for id in self.docs.ids() {
                    if self.docs.is_edited(id) {
                        self.docs.save(ctx, id);
                    }
                }
                self.docs.sync(ctx);
                return true;
            }
            Action::ExportPdf => return self.export_pdf(ctx),
            Action::InsertPage | Action::InsertLine | Action::InsertParagraph => {
                // a plain text document has no line break apart from the paragraph break
                let rich = active_doc(&self.docs).is_some_and(|d| d.rich);
                let s = match action {
                    Action::InsertPage => text::PAGE_BREAK,
                    Action::InsertLine if rich => text::LINE_BREAK,
                    _ => '\n',
                };
                return self.insert_raw(&s.to_string());
            }
            Action::InsertImage => return self.open_image_panel(ctx),
            Action::AddLink => return self.open_link_sheet(),
            Action::LinkDone => return self.finish_link(true),
            Action::LinkRemove => return self.finish_link(false),
            Action::LinkCancel => return self.link.take().is_some(),
            Action::RemoveLink => return self.remove_link_here(),
            Action::OpenLink => return self.open_link(ctx),
            Action::ShowProperties => return self.open_properties(),
            Action::PropsDone => return self.close_properties(true),
            Action::PropsCancel => return self.close_properties(false),
            Action::Encoding(index) => return Encoding::from_index(index).is_some_and(|e| self.set_encoding(e)),
            Action::Reinterpret(index) => return Encoding::from_index(index).is_some_and(|e| self.reinterpret(e)),
            Action::DefaultEncoding(index) => {
                let Some(e) = Encoding::from_index(index) else { return false };
                self.opts.default_encoding = e;
                self.save_options(ctx);
                return true;
            }
            Action::MarkupAsCode => {
                self.opts.markup_as_code = !self.opts.markup_as_code;
                document::set_markup_as_code(self.opts.markup_as_code);
                self.save_options(ctx);
                let note = if self.opts.markup_as_code { "RTF and HTML files now open as code" } else { "RTF and HTML files now open formatted" };
                self.note = Some(note.into());
                return true;
            }
            Action::Complete => return self.complete(),
            Action::SpeakStart => {
                let Some(doc) = active_doc(&self.docs) else { return false };
                let text = doc.text.selected_text().unwrap_or_else(|| doc.text.text());
                if !text.trim().is_empty() {
                    bifrost_app::speech::speak(ctx, &bifrost_app::speech::Utterance::new(&text));
                }
                return false;
            }
            Action::SpeakStop => {
                bifrost_app::speech::stop(ctx);
                return false;
            }
            Action::UseSelection => {
                let Some(doc) = active_doc(&self.docs) else { return false };
                let picked: String = doc.text.cells()[doc.text.selection()].iter().map(|c| c.ch).collect();
                if picked.is_empty() || picked.contains('\n') {
                    return false;
                }
                // the selection goes on the find pasteboard, which other apps' find bars read too
                NativePasteboard::find().set_string(ctx, &picked);
                self.note = Some(format!("Finding \"{picked}\""));
                self.find = picked;
                return true;
            }
            Action::JumpToSelection => {
                self.follow = true;
                return true;
            }
            Action::SelectLine => return self.open_line_sheet(),
            Action::LineDone => return self.finish_line(),
            Action::LineCancel => return self.line.take().is_some(),
            Action::CopyStyle => {
                let Some(doc) = active_doc(&self.docs) else { return false };
                self.copied_style = Some(doc.text.char_style());
                self.note = Some("Style copied".into());
                return true;
            }
            Action::PreventEditing => {
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                doc.read_only = !doc.read_only;
                return true;
            }
            Action::ShowRuler => {
                self.opts.show_ruler = !self.opts.show_ruler;
                self.save_options(ctx);
                return true;
            }
            Action::WrapToWindow => {
                self.opts.wrap_window = !self.opts.wrap_window;
                self.save_options(ctx);
                self.revision += 1;
                return true;
            }
            Action::ZoomIn | Action::ZoomOut | Action::ActualSize => {
                self.zoom = match action {
                    Action::ZoomIn => prefs::zoom_in(self.zoom),
                    Action::ZoomOut => prefs::zoom_out(self.zoom),
                    _ => 1.0,
                };
                self.revision += 1;
                self.follow = true;
                return true;
            }
            Action::Rewrap => {
                self.opts.rewrap = !self.opts.rewrap;
                self.save_options(ctx);
                return true;
            }
            Action::DarkBackground => {
                self.opts.dark_background = !self.opts.dark_background;
                self.save_options(ctx);
                return true;
            }
            Action::TextReplacement => {
                self.opts.text_replacement = !self.opts.text_replacement;
                self.save_options(ctx);
                return true;
            }
            Action::DefaultFont(index) => {
                if (index as usize) >= text::families().len() {
                    return false;
                }
                self.opts.default_font = index;
                self.save_options(ctx);
                return true;
            }
            Action::SmartQuotes => {
                self.opts.smart_quotes = !self.opts.smart_quotes;
                self.save_options(ctx);
                return true;
            }
            Action::SmartLinks => {
                self.opts.smart_links = !self.opts.smart_links;
                self.save_options(ctx);
                return true;
            }
            Action::SmartDashes => {
                self.opts.smart_dashes = !self.opts.smart_dashes;
                self.save_options(ctx);
                return true;
            }
            Action::SpellAuto => {
                self.opts.spell_auto = !self.opts.spell_auto;
                self.save_options(ctx);
                return true;
            }
            Action::HeaderFooter => {
                self.opts.header_footer = !self.opts.header_footer;
                self.save_options(ctx);
                return true;
            }
            Action::NewPlain(plain) => {
                self.opts.new_plain = plain;
                self.save_options(ctx);
                return true;
            }
            Action::Find => {
                // the find field starts with the find pasteboard's text, as the shared find field does
                if let Some(shared) = NativePasteboard::find().string().filter(|s| !s.is_empty() && !s.contains('\n')) {
                    self.find = shared;
                }
                self.find_open = true;
                self.focus = Focus::Find;
                // the previous query comes back selected, so typing starts a new one (QA W2-12)
                self.find_sel = !self.find.is_empty();
                return true;
            }
            Action::FindDone => {
                self.find_open = false;
                self.focus = Focus::Body;
                self.find_sel = false;
                // the bar's "not found" or "replaced" note goes with it (QA W2-12)
                self.note = None;
                return true;
            }
            Action::FocusFind => {
                self.focus = Focus::Find;
                self.find_sel = false;
                return true;
            }
            Action::FocusReplace => {
                self.focus = Focus::Replace;
                self.find_sel = false;
                return true;
            }
            Action::FindNext => return self.find_step(true),
            Action::FindPrevious => return self.find_step(false),
            Action::Replace => {
                if self.refuse_edit() {
                    return true;
                }
                let (query, replacement, case) = (self.find.clone(), self.replace.clone(), false);
                let replaced = active_doc_mut(&mut self.docs).is_some_and(|d| d.text.replace_selection(&query, &replacement, case));
                if replaced {
                    self.edited();
                }
                return self.find_step(true) || replaced;
            }
            Action::ReplaceAll => {
                if self.refuse_edit() {
                    return true;
                }
                let (query, replacement) = (self.find.clone(), self.replace.clone());
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                let count = doc.text.replace_all(&query, &replacement, false);
                let note = if count == 0 { format!("\"{query}\" not found") } else { format!("{count} replaced") };
                self.edited();
                self.note = Some(note);
                return true;
            }
            Action::Print => return self.print_document(ctx),
            Action::Paper(index) => {
                if let Some(paper) = print::PAPERS.get(index as usize) {
                    self.print_info.paper = *paper;
                }
                return true;
            }
            Action::Landscape(on) => {
                self.print_info.orientation = if on { Orientation::Landscape } else { Orientation::Portrait };
                return true;
            }
            Action::SpellToggle => {
                self.spelling.enabled = !self.spelling.enabled;
                self.opts.spell_enabled = self.spelling.enabled;
                self.save_options(ctx);
                self.revision += 1;
                return true;
            }
            Action::SpellNext => return self.spell_next(),
            Action::SpellCorrect => return self.spell_correct(),
            Action::SpellLearn => {
                let learned = self.spell_learn(ctx);
                // the panel moves on from a word it has learned
                if self.spelling_panel {
                    self.spell_next();
                }
                return learned;
            }
            Action::SpellIgnore => return self.spell_ignore(),
            Action::ShowSpelling => {
                if active_doc(&self.docs).is_none() {
                    return false;
                }
                self.props = None;
                self.link = None;
                self.line = None;
                self.spelling_panel = true;
                // the panel opens on the next misspelled word, selected
                self.spell_next();
                return true;
            }
            Action::SpellingIgnore => {
                if self.spell_ignore() {
                    self.spell_next();
                }
                return true;
            }
            Action::SpellingChange => {
                if self.spell_correct() {
                    self.spell_next();
                }
                return true;
            }
            Action::SpellingDone => {
                self.spelling_panel = false;
                return true;
            }
            Action::ShowSettings => {
                self.settings = true;
                self.props = None;
                self.link = None;
                self.line = None;
                self.spelling_panel = false;
                return true;
            }
            Action::SettingsDone => {
                self.settings = false;
                return true;
            }
            Action::ShowEncodings => {
                self.encodings_panel = true;
                self.settings = false;
                return true;
            }
            Action::EncodingsDone => {
                self.encodings_panel = false;
                return true;
            }
            Action::EncodingToggle(index) => {
                let index = index as usize;
                if index >= encoding::ALL.len() {
                    return false;
                }
                self.opts.encodings = self.opts.with_encoding(index, !self.opts.lists_encoding(index));
                self.save_options(ctx);
                return true;
            }
            Action::ShowSpacing => return self.open_spacing_sheet(),
            Action::ShowFonts => {
                // Cmd+T toggles the panel, as the shared font panel's menu item does
                self.fonts = match self.fonts.take() {
                    Some(_) => None,
                    None => Some(font_panel::Fonts::new(self.status().style.font)),
                };
                self.colors_panel = false;
                return true;
            }
            Action::ShowColors => {
                self.colors_panel = !self.colors_panel;
                self.fonts = None;
                return true;
            }
            Action::FontsDone => return self.fonts.take().is_some(),
            Action::ColorsDone => return std::mem::take(&mut self.colors_panel),
            Action::SpacingDone => return self.finish_spacing(),
            Action::SpacingCancel => return self.spacing_sheet.take().is_some(),
            Action::Setting(index) => {
                let Some(setting) = prefs::Setting::ALL.get(index as usize).copied() else { return false };
                setting.change(&mut self.opts);
                self.apply_options(ctx);
                return true;
            }
            _ => {}
        }
        if edits_text(action) && self.refuse_edit() {
            return true;
        }
        let formatting = is_formatting(action);
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        // a plain text document has no character or paragraph formatting to change
        if formatting && !doc.rich {
            return false;
        }
        let changed = match action {
            Action::Undo => doc.undo(),
            Action::Redo => doc.redo(),
            Action::Cut => {
                let picked = doc.text.cells()[doc.text.selection()].to_vec();
                let picture = picture_of(doc, &picked);
                let cut = doc.text.cut();
                // the clipboard text leaves the attachment characters out: they mean nothing to another application
                let text: String = cut.chars().filter(|&c| c != OBJECT).collect();
                if let Some(data) = picture {
                    put_picture(ctx, &data);
                }
                if cut.is_empty() {
                    false
                } else {
                    clipboard::set_text(&text);
                    self.rich_clip = Some((text, picked));
                    true
                }
            }
            Action::Copy => {
                let picked = doc.text.cells()[doc.text.selection()].to_vec();
                let copied: String = picked.iter().filter(|c| c.ch != OBJECT).map(|c| c.ch).collect();
                if let Some(data) = picture_of(doc, &picked) {
                    put_picture(ctx, &data);
                    // a picture alone: the text clipboard is emptied so a paste takes the picture
                    if copied.is_empty() {
                        clipboard::set_text("");
                    }
                }
                if !copied.is_empty() {
                    clipboard::set_text(&copied);
                    self.rich_clip = Some((copied, picked));
                }
                return false;
            }
            // a paste of this application's own copy keeps the formatting it was copied with; any other text is
            // pasted as plain text in the style at the caret. Paste and Match Style always takes the caret's style.
            Action::Paste => match clipboard::text() {
                Some(s) if !s.is_empty() => {
                    match self.rich_clip.as_ref().filter(|(text, _)| *text == s) {
                        Some((_, cells)) => doc.text.insert_cells(cells, Kind::Pasting),
                        None => doc.text.insert(&normalize_breaks(&s), Kind::Pasting),
                    }
                    true
                }
                _ => false,
            },
            Action::PasteMatch => match clipboard::text() {
                Some(s) if !s.is_empty() => {
                    doc.text.insert(&normalize_breaks(&s), Kind::Pasting);
                    true
                }
                _ => false,
            },
            Action::CopyRuler => {
                self.ruler = Some(doc.text.tab_width());
                self.note = Some("Ruler copied".into());
                return true;
            }
            Action::PasteRuler => match self.ruler {
                Some(width) => {
                    doc.text.set_tab_width(width);
                    if let Some(id) = self.docs.active() {
                        self.docs.mark_edited(id);
                    }
                    self.revision += 1;
                    true
                }
                None => false,
            },
            Action::SelectAll => {
                doc.text.select_all();
                return true;
            }
            Action::Delete => {
                if doc.text.has_selection() {
                    doc.text.delete_backward();
                    true
                } else {
                    false
                }
            }
            Action::Transform(how) => doc.text.transform(how),
            Action::Direction(rtl) => {
                doc.text.set_writing_direction(rtl);
                true
            }
            Action::Strike => {
                doc.text.toggle_strike();
                true
            }
            Action::Superscript => {
                doc.text.toggle_script(1);
                true
            }
            Action::Subscript => {
                doc.text.toggle_script(-1);
                true
            }
            Action::Highlight(index) => {
                doc.text.set_highlight(index);
                true
            }
            // Paste Style gives the character formatting and the paragraph's alignment, spacing and list
            Action::PasteStyle => match self.copied_style {
                Some(style) => {
                    doc.text.apply_style(&style);
                    true
                }
                None => false,
            },
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
            Action::FontStyle(face) => {
                doc.text.set_face(face & 1 != 0, face & 2 != 0);
                true
            }
            Action::Size(index) => match text::SIZES.get(index as usize) {
                Some(size) => {
                    doc.text.set_size(*size);
                    true
                }
                None => false,
            },
            Action::Align(align) => {
                doc.text.set_align(align);
                true
            }
            Action::Color(index) => {
                doc.text.set_color(index);
                true
            }
            Action::Spacing(index) => {
                doc.text.set_spacing(index);
                true
            }
            Action::List(kind) => {
                doc.text.toggle_list(kind);
                true
            }
            Action::MakePlain => {
                // a rich document with anything in it is converted only after the confirmation sheet says so (its
                // styles and attachments are lost), as the reference asks "Convert this document to plain text?"
                if doc.rich && !doc.text.cells().is_empty() {
                    self.plain_sheet = true;
                    self.plain_focus = None;
                    return true;
                }
                let was_rich = doc.rich;
                doc.set_rich(false);
                was_rich
            }
            Action::MakeRich => {
                let was_rich = doc.rich;
                doc.set_rich(true);
                !was_rich
            }
            _ => false,
        };
        if changed {
            self.edited();
        }
        changed
    }

    /// A new document takes the format and the plain text encoding chosen in the Preferences.
    fn apply_new_defaults(&mut self, id: DocId) {
        let (plain, encoding, font) = (self.opts.new_plain, self.opts.default_encoding, self.opts.default_font);
        if let Some(d) = self.docs.get_mut(id) {
            if plain {
                d.rich = false;
            }
            d.encoding = encoding;
            d.text.set_default_font(font);
        }
    }

    /// Format > Text Encoding: the encoding a plain text document is saved in. Only a change marks the document edited.
    fn set_encoding(&mut self, encoding: Encoding) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let Some(id) = self.docs.active() else { return false };
        let Some(doc) = self.docs.get_mut(id) else { return false };
        if doc.rich {
            self.note = Some("Only plain text has an encoding. Choose Format > Make Plain Text first.".into());
            return true;
        }
        if doc.encoding == encoding {
            return false;
        }
        doc.encoding = encoding;
        self.docs.mark_edited(id);
        self.note = Some(format!("Saves as {}", encoding.name()));
        true
    }

    /// Format > Reinterpret As: the text was read in the wrong encoding, so its characters go back to bytes and are read
    /// again in `to`. One undo entry.
    fn reinterpret(&mut self, to: Encoding) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        if doc.rich {
            self.note = Some("Only plain text has an encoding. Choose Format > Make Plain Text first.".into());
            return true;
        }
        let Some(again) = encoding::reinterpret(&doc.text.text(), doc.encoding, to) else {
            self.note = Some(format!("The text cannot be read as {}", to.name()));
            return true;
        };
        let len = doc.text.len();
        doc.text.replace_range(0..len, &normalize_breaks(&again));
        let end = doc.text.len();
        doc.text.select(end, end);
        doc.encoding = to;
        self.edited();
        self.note = Some(format!("Read as {}", to.name()));
        true
    }

    /// File > Show Properties: opens the panel on the active document's properties.
    fn open_properties(&mut self) -> bool {
        let Some(id) = self.docs.active() else { return false };
        let Some(doc) = self.docs.get(id) else { return false };
        self.props = Some((id, properties::Panel::new(&doc.properties)));
        self.link = None;
        true
    }

    /// The text a link command acts on: the selection, or the link around the caret.
    fn link_target(&self) -> Option<Range<usize>> {
        let doc = active_doc(&self.docs)?;
        if doc.text.has_selection() {
            Some(doc.text.selection())
        } else {
            doc.text.link_range(doc.text.head())
        }
    }

    /// Edit > Add Link: opens the panel on the selected text (or the link around the caret) with its address.
    fn open_link_sheet(&mut self) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let Some(id) = self.docs.active() else { return false };
        let Some(range) = self.link_target() else {
            self.note = Some("Select the text to link first".into());
            return true;
        };
        let Some(doc) = self.docs.get(id) else { return false };
        if !doc.rich {
            self.note = Some("Links need rich text. Choose Format > Make Rich Text first.".into());
            return true;
        }
        let current = links::url(doc.text.link_at(range.start)).unwrap_or_default();
        let had_link = doc.text.cells()[range.clone()].iter().any(|c| c.style.link != 0);
        self.link = Some(LinkSheet { doc: id, range, url: current, had_link });
        self.props = None;
        true
    }

    /// Done in the link panel (`apply`) links the text to the typed address, or removes the link when the address is
    /// empty; Remove Link takes the link off. An address that is not one stays in the panel with a note.
    fn finish_link(&mut self, apply: bool) -> bool {
        let Some(sheet) = self.link.take() else { return false };
        let mut link = 0;
        let typed = sheet.url.trim().to_string();
        if apply && !typed.is_empty() {
            link = links::normalize(&typed).map_or(0, |url| links::intern(&url));
            if link == 0 {
                self.note = Some(format!("\"{typed}\" is not an address TextEdit can link to"));
                self.link = Some(sheet);
                return true;
            }
        }
        let Some(doc) = self.docs.get_mut(sheet.doc) else { return true };
        if doc.text.set_link(sheet.range, link) {
            self.edited();
            let note = if link == 0 { "Link removed" } else { "Link added" };
            self.note = Some(note.into());
        }
        true
    }

    /// Edit > Remove Link: takes the link off the selection or the link around the caret.
    fn remove_link_here(&mut self) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let Some(range) = self.link_target() else { return false };
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        if doc.text.set_link(range, 0) {
            self.edited();
            return true;
        }
        false
    }

    /// Edit > Open Link: the address of the link at the caret (or the start of the selection) goes to the system.
    fn open_link(&mut self, ctx: &Ctx) -> bool {
        let Some(doc) = active_doc(&self.docs) else { return false };
        let at = if doc.text.has_selection() { doc.text.selection().start } else { doc.text.head() };
        let Some(url) = links::url(doc.text.link_near(at)) else { return false };
        ctx.open_url(&url);
        self.note = Some(format!("Opening {url}"));
        true
    }

    /// A press while the link panel is open: a button, or nothing (the page behind it is not reachable).
    fn touch_link(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::layout_rows(geom.width, geom.body_top, 1);
        let can_remove = self.link.as_ref().is_some_and(|s| s.had_link);
        let buttons = view::link_buttons(&ctx.ui, &layout, can_remove);
        if let Some(b) = buttons.iter().find(|b| b.rect.contains(px, py)) {
            return match b.action {
                Some(action) => self.perform(ctx, action),
                None => false,
            };
        }
        true
    }

    /// `scrollWheel:` at (`x`, `y`) window pixels: the content moved `dy` pixels (negative for a wheel turned down, which
    /// shows what is below). Over the Fonts panel's family column the families scroll; elsewhere on a floating panel, or
    /// while a sheet is open, it is used up; over the page the page scrolls, within the text. True when something moved.
    fn wheel(&mut self, ctx: &Ctx, x: f32, y: f32, dy: f32) -> bool {
        if !(dy.is_finite() && x.is_finite() && y.is_finite()) || dy == 0.0 {
            return false;
        }
        let (px, py, dy) = (x / ctx.ui.s, y / ctx.ui.s, dy / ctx.ui.s);
        let geom = self.geom(ctx);
        if let Some(panel) = self.fonts.as_mut() {
            let layout = font_panel::fonts_layout(geom.width, geom.body_top);
            if layout.family.contains(px, py) {
                return panel.wheel(dy);
            }
            if layout.frame.contains(px, py) {
                return false;
            }
        }
        if self.colors_panel && font_panel::colors_layout(geom.width, geom.body_top).frame.contains(px, py) {
            return false;
        }
        let sheet = self.props.is_some()
            || self.spacing_sheet.is_some()
            || self.encodings_panel
            || self.settings
            || self.spelling_panel
            || self.plain_sheet
            || self.close_alert_here()
            || self.line.is_some()
            || self.link.is_some();
        if sheet || py < geom.body_top || py >= geom.body_bottom {
            return false;
        }
        let view_h = geom.body_bottom - geom.body_top;
        let most = self.cache.as_ref().map_or(0.0, |c| (c.layout.height - view_h).max(0.0));
        let to = (self.scroll - dy).clamp(0.0, most);
        let moved = to != self.scroll;
        self.scroll = to;
        moved
    }

    /// A press while the font panel is open: a family, typeface or size applies to the selection at once; the arrows
    /// scroll the families; Done closes. A press elsewhere is used up (the panel floats over the page).
    fn touch_fonts(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = font_panel::fonts_layout(geom.width, geom.body_top);
        let Some(panel) = self.fonts.as_mut() else { return false };
        match font_panel::fonts_hit(&layout, panel, px, py) {
            Some(font_panel::FontsHit::Family(i)) => self.perform(ctx, Action::Font(i)),
            Some(font_panel::FontsHit::Face(i)) => self.perform(ctx, Action::FontStyle(i)),
            Some(font_panel::FontsHit::Size(i)) => self.perform(ctx, Action::Size(i)),
            Some(font_panel::FontsHit::Up) => {
                panel.scroll(-(font_panel::ROWS as i32 - 1));
                true
            }
            Some(font_panel::FontsHit::Down) => {
                panel.scroll(font_panel::ROWS as i32 - 1);
                true
            }
            Some(font_panel::FontsHit::Done) => self.perform(ctx, Action::FontsDone),
            None => true,
        }
    }

    /// A press while the colour panel is open: a swatch colours the selection, Done closes.
    fn touch_colors(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = font_panel::colors_layout(geom.width, geom.body_top);
        match font_panel::colors_hit(&layout, px, py) {
            Some(Some(i)) => self.perform(ctx, Action::Color(i)),
            Some(None) => self.perform(ctx, Action::ColorsDone),
            None => true,
        }
    }

    /// Up and Down in the font panel: the next or previous family, applied at once.
    fn fonts_step(&mut self, ctx: &Ctx, by: i32) -> bool {
        let current = self.status().style.font as i32;
        let next = (current + by).clamp(0, text::families().len() as i32 - 1) as u8;
        if let Some(panel) = self.fonts.as_mut() {
            panel.reveal(next as usize);
        }
        self.perform(ctx, Action::Font(next))
    }

    /// A press while the Preferences panel is open: Done, a row (which changes that setting), or nothing.
    fn touch_settings(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::list_layout(geom.width, geom.body_top, prefs::Setting::ALL.len());
        let buttons = view::list_buttons(&ctx.ui, &layout, Action::SettingsDone);
        if let Some(action) = buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            return self.perform(ctx, action);
        }
        match layout.rows.iter().position(|r| r.contains(px, py)) {
            Some(index) => self.perform(ctx, Action::Setting(index as u8)),
            None => true,
        }
    }

    /// A press while the Customize Encodings List panel is open: Done, a row (which lists or unlists that encoding), or
    /// nothing.
    fn touch_encodings(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::list_layout(geom.width, geom.body_top, encoding::ALL.len());
        let buttons = view::list_buttons(&ctx.ui, &layout, Action::EncodingsDone);
        if let Some(action) = buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            return self.perform(ctx, action);
        }
        match layout.rows.iter().position(|r| r.contains(px, py)) {
            Some(index) => self.perform(ctx, Action::EncodingToggle(index as u8)),
            None => true,
        }
    }

    /// A press while the Show Spelling and Grammar panel is open: a button, or nothing (the page is not reachable).
    fn touch_spelling(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::layout_rows(geom.width, geom.body_top, 2);
        let buttons = view::spelling_buttons(&ctx.ui, &layout);
        match buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            Some(action) => self.perform(ctx, action),
            None => true,
        }
    }

    /// The word the Spelling panel shows (the word at the caret or the selection) and its best correction.
    fn spelling_fields(&self) -> (String, String) {
        let Some(doc) = active_doc(&self.docs) else { return (String::new(), String::new()) };
        match word_here(doc) {
            Some((range, word)) => {
                let guess = self.spelling.correction(doc.text.cells(), range).unwrap_or_default();
                (word, guess)
            }
            None => (String::new(), String::new()),
        }
    }

    /// A press while the Select Line panel is open: a button, or nothing (the page behind it is not reachable).
    fn touch_line(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::layout_rows(geom.width, geom.body_top, 1);
        let buttons = view::line_buttons(&ctx.ui, &layout);
        match buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            Some(action) => self.perform(ctx, action),
            None => true,
        }
    }

    /// Closes the properties panel; Done (`apply`) stores the draft on the document and marks it edited when it changed.
    fn close_properties(&mut self, apply: bool) -> bool {
        let Some((id, panel)) = self.props.take() else { return false };
        if !apply {
            return true;
        }
        let new = panel.result();
        let Some(doc) = self.docs.get_mut(id) else { return true };
        if doc.properties == new {
            return true;
        }
        if doc.read_only {
            self.note = Some("Editing is prevented. Choose Format > Allow Editing.".into());
            return true;
        }
        doc.properties = new;
        self.docs.mark_edited(id);
        self.note = Some("Properties changed".into());
        true
    }

    /// A press while the properties panel is open: a button, a field, or nothing (the page behind it is not reachable).
    fn touch_properties(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::layout(geom.width, geom.body_top);
        let buttons = view::properties_buttons(&ctx.ui, &layout);
        if let Some(b) = buttons.iter().find(|b| b.rect.contains(px, py)) {
            return match b.action {
                Some(action) => self.perform(ctx, action),
                None => false,
            };
        }
        if let (Some(field), Some((_, panel))) = (properties::field_at(&layout, px, py), self.props.as_mut()) {
            panel.focus = field;
        }
        true
    }

    /// Select All, Copy and Cut while a find field has the keyboard: they act on the field, never on the page.
    fn find_field_command(&mut self, action: Action) -> bool {
        let text = if self.focus == Focus::Find { self.find.clone() } else { self.replace.clone() };
        match action {
            Action::SelectAll => self.find_sel = !text.is_empty(),
            Action::Copy | Action::Cut if self.find_sel => {
                clipboard::set_text(&text);
                if matches!(action, Action::Cut) {
                    self.find_sel = false;
                    if self.focus == Focus::Find {
                        self.find.clear();
                        self.find_live();
                    } else {
                        self.replace.clear();
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// The live search as the query changes: the first match at or after the start of the selection is selected
    /// (wrapping), so the page follows the typing; the page highlights every match while the bar is open.
    fn find_live(&mut self) -> bool {
        if self.find.is_empty() {
            self.note = None;
            return true;
        }
        let query = self.find.clone();
        let Some(doc) = active_doc_mut(&mut self.docs) else { return true };
        let from = doc.text.selection().start;
        match doc.text.find(&query, from, true, false) {
            Some(r) => {
                doc.text.select(r.start, r.end);
                self.note = None;
            }
            None => self.note = Some(format!("\"{query}\" not found")),
        }
        self.follow = true;
        true
    }

    /// Every match of the find field in the active document, for the highlight (none while the bar is closed).
    fn find_matches(&self) -> Vec<Range<usize>> {
        if !self.find_open || self.find.is_empty() {
            return Vec::new();
        }
        active_doc(&self.docs).map_or_else(Vec::new, |d| d.text.find_all(&self.find, false, MAX_HIGHLIGHTS))
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

    /// Check Document Now: selects the next misspelled word after the selection, wrapping around.
    fn spell_next(&mut self) -> bool {
        let Some(doc) = active_doc(&self.docs) else { return false };
        let ranges = self.spelling.misspelled(doc.text.cells());
        let next = spell::next_misspelled(&ranges, doc.text.selection().end);
        let Some(range) = next else {
            self.note = Some("No misspelled words".into());
            return true;
        };
        if let Some(doc) = active_doc_mut(&mut self.docs) {
            doc.text.select(range.start, range.end);
        }
        self.note = None;
        self.follow = true;
        true
    }

    /// Correct Spelling: replaces the word at the caret with the checker's best guess, keeping its style.
    fn spell_correct(&mut self) -> bool {
        let Some(doc) = active_doc(&self.docs) else { return false };
        let Some((range, word)) = word_here(doc) else { return false };
        let Some(guess) = self.spelling.correction(doc.text.cells(), range.clone()) else {
            self.note = Some(format!("No correction for \"{word}\""));
            return true;
        };
        if let Some(doc) = active_doc_mut(&mut self.docs) {
            doc.text.select(range.start, range.end);
            doc.text.replace_selection(&word, &guess, true);
        }
        self.edited();
        true
    }

    /// Learn Spelling: adds the word at the caret to the user's dictionary, kept in the preferences.
    fn spell_learn(&mut self, ctx: &Ctx) -> bool {
        let Some((_, word)) = active_doc(&self.docs).and_then(word_here) else { return false };
        self.spelling.learn(ctx, &word);
        self.revision += 1;
        self.note = Some(format!("Learned \"{word}\""));
        true
    }

    /// Ignore Spelling: stops flagging the word at the caret until the app quits.
    fn spell_ignore(&mut self) -> bool {
        let Some((_, word)) = active_doc(&self.docs).and_then(word_here) else { return false };
        self.spelling.ignore(&word);
        self.revision += 1;
        true
    }

    /// Edit > Find > Select Line: opens the panel that takes a line number or a range ("12", "12-20").
    fn open_line_sheet(&mut self) -> bool {
        if active_doc(&self.docs).is_none() {
            return false;
        }
        self.line = Some(String::new());
        self.props = None;
        self.link = None;
        true
    }

    /// Format > Text > Spacing: opens the panel on the paragraph at the caret (or the first one the selection touches).
    fn open_spacing_sheet(&mut self) -> bool {
        let Some(doc) = active_doc(&self.docs) else { return false };
        if !doc.rich {
            self.note = Some("Spacing needs rich text. Choose Format > Make Rich Text first.".into());
            return true;
        }
        let style = doc.text.paragraph_style_at(doc.text.selection().start);
        let (before, after) = doc.text.paragraph_space_at(doc.text.selection().start);
        let line = view::spacing_label(style.spacing);
        self.spacing_sheet = Some(SpacingSheet { fields: [line, before.to_string(), after.to_string()], focus: 0 });
        self.props = None;
        self.link = None;
        self.line = None;
        true
    }

    /// Done in the Spacing panel: the line spacing is the preset nearest the number typed (1.0 to 3.0), and the spaces
    /// are whole points. A value that is not a number keeps the panel open with a note.
    fn finish_spacing(&mut self) -> bool {
        let Some(sheet) = self.spacing_sheet.as_ref() else { return false };
        let line: Option<f32> = sheet.fields[0].trim().parse::<f32>().ok().filter(|m| (1.0..=3.0).contains(m));
        let before: Option<u8> = sheet.fields[1].trim().parse::<u8>().ok().filter(|&p| p <= text::MAX_PARAGRAPH_SPACE);
        let after: Option<u8> = sheet.fields[2].trim().parse::<u8>().ok().filter(|&p| p <= text::MAX_PARAGRAPH_SPACE);
        let (Some(line), Some(before), Some(after)) = (line, before, after) else {
            self.note = Some(format!("Line spacing is 1 to 3, and the spaces are 0 to {} points", text::MAX_PARAGRAPH_SPACE));
            return true;
        };
        if self.refuse_edit() {
            return true;
        }
        let spacing = rtf::nearest_spacing(line);
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.set_paragraph_format(spacing, before, after);
        self.spacing_sheet = None;
        self.edited();
        true
    }

    /// A press while the Spacing panel is open: a button or a field, or nothing (the page is not reachable).
    fn touch_spacing(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let layout = properties::layout_rows(geom.width, geom.body_top, 3);
        let buttons = view::spacing_buttons(&ctx.ui, &layout);
        if let Some(action) = buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            return self.perform(ctx, action);
        }
        if let (Some(field), Some(sheet)) = (properties::field_at(&layout, px, py), self.spacing_sheet.as_mut()) {
            sheet.focus = field;
        }
        true
    }

    /// Done in the Select Line panel: selects the lines it names and closes the panel. A line that is not in the document
    /// keeps the panel open with a note, so the number can be corrected.
    fn finish_line(&mut self) -> bool {
        let Some(typed) = self.line.clone() else { return false };
        let Some((first, last)) = text::parse_line_spec(&typed) else {
            self.note = Some("Type a line number or a range such as 12-20".into());
            return true;
        };
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let found = doc.text.select_lines(first, last);
        let count = doc.text.line_count();
        if !found {
            self.note = Some(format!("There is no line {first}; the document has {count}"));
            return true;
        }
        self.line = None;
        self.note = Some(format!("Selected line {first}"));
        self.follow = true;
        true
    }

    /// Edit > Complete: replaces the word before the caret with the first word of the document that starts with it;
    /// choosing it again steps through the other candidates.
    fn complete(&mut self) -> bool {
        let previous = self.completion.take();
        if self.refuse_edit() {
            return true;
        }
        let Some(doc) = active_doc(&self.docs) else { return false };
        let sel = doc.text.selection();
        let (prefix, start, index) = match previous {
            Some(c) if c.range == sel && !sel.is_empty() => (c.prefix, c.range.start, c.index + 1),
            _ => {
                let head = doc.text.head();
                let Some(range) = spell::word_at(doc.text.cells(), head).filter(|r| r.end == head && !doc.text.has_selection()) else {
                    return false;
                };
                let prefix: String = doc.text.cells()[range.clone()].iter().map(|c| c.ch).collect();
                (prefix, range.start, 0)
            }
        };
        let options = text::completions(doc.text.cells(), &prefix);
        if options.is_empty() {
            self.note = Some(format!("No completions for \"{prefix}\""));
            return true;
        }
        let index = index % options.len();
        let choice = options[index].clone();
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.replace_range(start..sel.end, &choice);
        let range = doc.text.selection();
        self.edited();
        self.completion = Some(Completion { prefix, range, index });
        true
    }

    /// The print operation for the active document: laid out for the page setup, with the title and page number in
    /// the margins when the header and footer are on.
    fn print_job(&self, ctx: &Ctx) -> Option<PrintOperation> {
        let info = self.print_info.clone();
        let doc = active_doc(&self.docs)?;
        let (_, _, width, _) = info.imageable();
        let aspects = view::aspects(doc.text.attachments());
        let measure = view::measure_in(&ctx.ui, &aspects, width);
        let (cells, layout) = if self.opts.rewrap {
            // wrapped to the page: the lines break at the printable width
            let cells = doc.text.cells().to_vec();
            let layout = layout::layout_tabs(&cells, width, &measure, doc.text.tab_width());
            (cells, layout)
        } else {
            // kept as on screen: the lines break where the window breaks them, and the whole column is scaled down to
            // the printable width (never up)
            let window = self.geom(ctx).text_w;
            let scale = if window > 0.0 { (width / window).min(1.0) } else { 1.0 };
            let cells = zoomed(doc.text.cells(), scale);
            let layout = layout::layout_tabs(&cells, window * scale, &measure, doc.text.tab_width() * scale);
            (cells, layout)
        };
        let title = self.docs.active().and_then(|id| self.docs.display_name(id)).unwrap_or_else(|| "Untitled".into());
        let header = self.opts.header_footer.then_some(title.as_str());
        let pages = print::document_with(&cells, &layout, &info, &measure, header);
        Some(PrintOperation::pages(title.clone(), pages, print::FACES.to_vec()).with_info(info))
    }

    /// File > Export as PDF: asks where to save; the PDF is written when the save panel answers (see `export_event`).
    fn export_pdf(&mut self, ctx: &Ctx) -> bool {
        let Some(id) = self.docs.active() else { return false };
        let name = self.docs.display_name(id).unwrap_or_else(|| "Untitled".into());
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
        self.export_request = Some(ctx.save_document(&format!("{stem}.pdf"), &["pdf"]));
        true
    }

    /// The save panel's answers for Export as PDF: the file to write, then the commit. `Err` hands back an event that is
    /// not the export's, for the document controller.
    fn export_event(&mut self, ctx: &Ctx, ev: DocumentEvent) -> Result<bool, DocumentEvent> {
        let Some(request) = self.export_request else { return Err(ev) };
        match ev {
            DocumentEvent::SaveTo(file) if file.request == request => {
                let written = match self.print_job(ctx).map(|job| job.document()) {
                    Some(Ok((_, bytes))) => {
                        use std::io::Write;
                        let mut f = &file.file;
                        f.set_len(0).and_then(|_| f.write_all(&bytes)).and_then(|_| f.sync_all()).map_err(|e| e.to_string())
                    }
                    Some(Err(e)) => Err(format!("{e:?}")),
                    None => Err("no document".to_string()),
                };
                match written {
                    Ok(()) => ctx.commit_document(request),
                    Err(message) => {
                        ctx.abort_document(request);
                        self.export_request = None;
                        self.note = Some(format!("Export failed: {message}"));
                    }
                }
                Ok(true)
            }
            DocumentEvent::Saved(file) if file.request == request => {
                self.export_request = None;
                self.note = Some("Exported as PDF".into());
                Ok(true)
            }
            DocumentEvent::Failed { request: failed, .. } if failed == request => {
                self.export_request = None;
                Ok(true)
            }
            other => Err(other),
        }
    }

    /// Makes the active document plain text now (the confirmed case); one undo entry, edited when it changed.
    fn make_plain_now(&mut self) -> bool {
        let was_rich = match active_doc_mut(&mut self.docs) {
            Some(doc) => {
                let was = doc.rich;
                doc.set_rich(false);
                was
            }
            None => false,
        };
        if was_rich {
            self.edited();
        }
        was_rich
    }

    /// A quit waiting on the documents (`quit_requested`) goes on after every answer: the next edited document asks on its own
    /// window, which comes forward, and with none left the app exits (`Ctx::terminate`). True when a sheet was raised.
    fn continue_quit(&mut self, ctx: &Ctx) -> bool {
        if !self.docs.quitting() {
            return false;
        }
        if self.docs.advance_quit(ctx) {
            ctx.terminate();
            return false;
        }
        match self.docs.alert_doc() {
            Some(doc) if self.current_doc != Some(doc) => self.raise_doc(ctx, doc),
            _ => false,
        }
    }

    /// Whether the window being handled shows the close confirmation: its document is the one waiting to close.
    fn close_alert_here(&self) -> bool {
        self.current_doc.is_some() && self.docs.alert_doc() == self.current_doc
    }

    /// The close confirmation was answered: Save saves and then closes, Don't Save closes now (`sync_windows` closes the
    /// window), Cancel keeps the document and its window as they are.
    fn answer_close(&mut self, ctx: &Ctx, choice: u8) -> bool {
        if self.docs.alert_doc().is_none() {
            return false;
        }
        let notices = self.docs.answer_alert(ctx, choice as u32);
        self.after_documents(&notices);
        true
    }

    /// The close confirmation's touches: the buttons it shows; the rest of the window is blocked while it is up.
    fn touch_close_alert(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let Some(sheet) = self.close_sheet() else { return false };
        let (parent, bar) = close_sheet_parent(&geom);
        match sheet.hit(parent, bar, 1.0, px, py) {
            bifrost_app::controls::sheet::Hit::Button(id) => self.perform(ctx, Action::CloseAnswer(id as u8)),
            _ => true,
        }
    }

    /// The close confirmation of this window's document as the kit's alert sheet (`Alert::sheet`, E2E9 6): it hangs
    /// from the title bar, its title wraps instead of being cut, and Save is the accent-filled default (Return).
    fn close_sheet(&self) -> Option<bifrost_app::controls::sheet::Sheet> {
        self.docs.alert_sheet().filter(|_| self.close_alert_here())
    }

    /// The Make Plain Text confirmation as the kit's alert sheet (E2E11 6), hung from the title bar like the close
    /// confirmation: stacked full-width buttons, OK the default on top. `None` while it is not up.
    fn plain_alert_sheet(&self) -> Option<bifrost_app::controls::sheet::Sheet> {
        self.plain_sheet.then(|| view::plain_alert(active_doc(&self.docs).is_some_and(|d| d.text.has_attachments())).sheet().with_focus(self.plain_focus))
    }

    /// A key press while the Make Plain Text confirmation is up (`Sheet::handle_key`); it takes every key.
    fn plain_key(&mut self, ctx: &Ctx, key: &KeyInput) -> bool {
        let Some(mut sheet) = self.plain_alert_sheet() else { return false };
        match bifrost_app::document::sheet_key(&mut sheet, key) {
            AlertKey::Answer(view::PLAIN_OK) => self.perform(ctx, Action::PlainDone),
            AlertKey::Answer(_) => self.perform(ctx, Action::PlainCancel),
            AlertKey::Focus => {
                self.plain_focus = sheet.focus;
                true
            }
            AlertKey::Ignored => false,
        }
    }

    /// The Make Plain Text confirmation's touches: its buttons; the rest of the window is blocked while it is up.
    fn touch_plain(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geom(ctx);
        let Some(sheet) = self.plain_alert_sheet() else { return false };
        let (parent, bar) = close_sheet_parent(&geom);
        match sheet.hit(parent, bar, 1.0, px, py) {
            bifrost_app::controls::sheet::Hit::Button(view::PLAIN_OK) => self.perform(ctx, Action::PlainDone),
            bifrost_app::controls::sheet::Hit::Button(_) => self.perform(ctx, Action::PlainCancel),
            _ => true,
        }
    }

    /// Insert > Image...: the open panel for PNG and JPEG files; the file it returns is inserted by `picture_event`.
    fn open_image_panel(&mut self, ctx: &Ctx) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let options = OpenOptions { title: "Insert Image".into(), filters: vec!["public.png".into(), "public.jpeg".into()], ..Default::default() };
        self.image_request = Some(ctx.open_panel(&options));
        true
    }

    /// The answers for pictures: the Insert Image panel's file, and the files of an image drop (they arrive as documents
    /// with the drag session's number). `Err` hands back an event that is not a picture's, for the document controller.
    fn picture_event(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> Result<bool, DocumentEvent> {
        match ev {
            DocumentEvent::Opened(file) if Some(file.request) == self.image_request => {
                self.image_request = None;
                self.insert_picture_files(vec![file]);
                Ok(true)
            }
            DocumentEvent::Opened(file) if self.drop_sessions.contains(&file.request) => {
                self.insert_picture_files(vec![file]);
                Ok(true)
            }
            DocumentEvent::OpenedMany { request, docs } if self.drop_sessions.contains(&request) => {
                self.insert_picture_files(docs);
                Ok(true)
            }
            DocumentEvent::Failed { request, .. } if Some(request) == self.image_request => {
                self.image_request = None;
                Ok(true)
            }
            other => Err(other),
        }
    }

    /// Reads each picture file and inserts it as an attachment. A file that is not a picture is named in the note.
    fn insert_picture_files(&mut self, files: Vec<bifrost_app::Document>) {
        use std::io::Read;
        for mut file in files {
            let mut data = Vec::new();
            let read = file.file.read_to_end(&mut data).map(|_| ()).map_err(|e| e.to_string());
            match read {
                Ok(()) => {
                    let name = file.name.clone();
                    self.insert_picture(&name, data);
                }
                Err(message) => self.note = Some(format!("{} could not be read: {message}", file.name)),
            }
        }
    }

    /// Inserts one picture at the caret, as an attachment in the active rich text document (one undo entry).
    fn insert_picture(&mut self, name: &str, data: Vec<u8>) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let rich = active_doc(&self.docs).is_some_and(|d| d.rich);
        if !rich {
            self.note = Some("Pictures need rich text. Choose Format > Make Rich Text first.".into());
            return true;
        }
        if bifrost_app::graphics_imageio::pixel_size(&data).is_none() {
            self.note = Some(format!("{name} is not a picture this app can read."));
            return true;
        }
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.insert_attachment(rtfd::attachment(name.to_string(), 0.0, 0.0, data), Kind::Inserting);
        self.edited();
        true
    }

    /// Print: the document is laid out for the page setup and sent to the default printer through the print operation.
    /// The answer is the status row's note.
    fn print_document(&mut self, ctx: &Ctx) -> bool {
        let Some(job) = self.print_job(ctx) else { return false };
        self.note = Some(match job.run() {
            Ok(_) => "Sent to the printer".into(),
            Err(PrintError::NoCups) => "The printing service is not running".into(),
            Err(PrintError::NoPrinter) => "No printer is set up".into(),
            Err(PrintError::Failed(message)) => format!("Printing failed: {message}"),
        });
        true
    }

    /// Typed text: into the find field when it has the keyboard, otherwise into the page.
    fn type_into(&mut self, s: &str) -> bool {
        let printable: String = s.chars().filter(|c| !c.is_control()).collect();
        if printable.is_empty() || self.close_alert_here() {
            return false;
        }
        // the font panel jumps to the family whose name is typed (and applies it, as clicking it would)
        if let Some(panel) = self.fonts.as_mut() {
            let Some(i) = panel.type_select(&printable, clock_ms()) else { return true };
            let rich = active_doc(&self.docs).is_some_and(|d| d.rich);
            if let Some(doc) = active_doc_mut(&mut self.docs).filter(|_| rich) {
                doc.text.set_font(i);
                self.edited();
            }
            return true;
        }
        if let Some((_, panel)) = self.props.as_mut() {
            panel.type_text(&printable);
            return true;
        }
        if let Some(sheet) = self.spacing_sheet.as_mut() {
            // a number: digits and the point, up to a few characters
            for ch in printable.chars().filter(|c| c.is_ascii_digit() || *c == '.') {
                let field = &mut sheet.fields[sheet.focus];
                if field.len() < 6 {
                    field.push(ch);
                }
            }
            return true;
        }
        if let Some(typed) = self.line.as_mut() {
            // a line number or a range: digits and the dash between the two numbers
            for ch in printable.chars().filter(|c| c.is_ascii_digit() || *c == '-') {
                if typed.len() < 12 {
                    typed.push(ch);
                }
            }
            return true;
        }
        if let Some(sheet) = self.link.as_mut() {
            for ch in printable.chars().filter(|c| !c.is_whitespace()) {
                if sheet.url.chars().count() < links::MAX_LEN {
                    sheet.url.push(ch);
                }
            }
            return true;
        }
        if self.find_open && self.focus != Focus::Body {
            // typing over a selected field replaces it
            let over = std::mem::take(&mut self.find_sel);
            let field = if self.focus == Focus::Find { &mut self.find } else { &mut self.replace };
            if over {
                field.clear();
            }
            field.push_str(&printable);
            if self.focus == Focus::Find {
                self.find_live();
            }
            return true;
        }
        self.insert_raw(&printable)
    }

    /// Inserts text into the page as typed, line breaks included.
    fn insert_raw(&mut self, s: &str) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let (quotes, dashes) = (self.opts.smart_quotes, self.opts.smart_dashes);
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        if quotes || dashes {
            doc.text.type_smart(s, quotes, dashes);
        } else {
            doc.text.type_text(s);
        }
        self.edited();
        // a blank ends a word: a shortcut becomes its symbol, and Smart Links turns an address into a link
        if s.chars().last().is_some_and(char::is_whitespace) {
            if self.opts.text_replacement {
                self.replace_shortcut();
            }
            if self.opts.smart_links {
                self.autolink();
            }
        }
        // a space or punctuation ends a word, which Correct Spelling Automatically then checks
        if self.opts.spell_auto && s.chars().last().is_some_and(|c| !c.is_alphanumeric() && c != '\'') {
            self.autocorrect();
        }
        true
    }

    /// Text Replacement: the word before the blank just typed becomes its symbol when it is a shortcut. The symbol must
    /// be one the document's encoding can save, so a plain text document in ASCII keeps the shortcut as typed. The caret
    /// stays after the blank.
    fn replace_shortcut(&mut self) {
        let Some(doc) = active_doc(&self.docs) else { return };
        let head = doc.text.head();
        let Some(boundary) = head.checked_sub(1) else { return };
        let cells = doc.text.cells();
        if !cells[boundary].ch.is_whitespace() {
            return;
        }
        let start = cells[..boundary].iter().rposition(|c| c.ch.is_whitespace()).map_or(0, |i| i + 1);
        let word: String = cells[start..boundary].iter().map(|c| c.ch).collect();
        let Some(symbol) = replace::for_shortcut(&word) else { return };
        if !doc.rich && encoding::encode(symbol, doc.encoding).is_err() {
            return;
        }
        let shift = symbol.chars().count() as isize - word.chars().count() as isize;
        let caret = (head as isize + shift).max(0) as usize;
        let Some(doc) = active_doc_mut(&mut self.docs) else { return };
        doc.text.replace_range(start..boundary, symbol);
        doc.text.select(caret, caret);
        self.edited();
    }

    /// Smart Links: the word before the blank just typed becomes a link when it is a web or mail address (and is not
    /// linked already). Rich text only.
    fn autolink(&mut self) {
        let Some(doc) = active_doc(&self.docs) else { return };
        let Some(boundary) = doc.text.head().checked_sub(1) else { return };
        let cells = doc.text.cells();
        if !doc.rich || !cells[boundary].ch.is_whitespace() {
            return;
        }
        let start = cells[..boundary].iter().rposition(|c| c.ch.is_whitespace()).map_or(0, |i| i + 1);
        let word: String = cells[start..boundary].iter().map(|c| c.ch).collect();
        let Some((offset, len, url)) = links::smart_link(&word) else { return };
        let range = start + offset..start + offset + len;
        if cells[range.clone()].iter().any(|c| c.style.link != 0) {
            return;
        }
        let id = links::intern(&url);
        if id == 0 {
            return;
        }
        if active_doc_mut(&mut self.docs).is_some_and(|d| d.text.set_link(range, id)) {
            self.edited();
        }
    }

    /// Correct Spelling Automatically: the word just ended by the typed boundary is replaced by the checker's guess, and
    /// the caret stays after the boundary.
    fn autocorrect(&mut self) {
        let Some(doc) = active_doc(&self.docs) else { return };
        let head = doc.text.head();
        let Some(boundary) = head.checked_sub(1) else { return };
        let Some((range, guess)) = self.spelling.autocorrect(doc.text.cells(), boundary) else { return };
        let word: String = doc.text.cells()[range.clone()].iter().map(|c| c.ch).collect();
        let shift = guess.chars().count() as isize - word.chars().count() as isize;
        let Some(doc) = active_doc_mut(&mut self.docs) else { return };
        doc.text.select(range.start, range.end);
        doc.text.replace_selection(&word, &guess, true);
        let caret = (head as isize + shift).max(0) as usize;
        doc.text.select(caret, caret);
        self.edited();
    }

    /// Backspace: from the focused field, or the page.
    fn backspace(&mut self) -> bool {
        if let Some((_, panel)) = self.props.as_mut() {
            panel.backspace();
            return true;
        }
        if let Some(sheet) = self.spacing_sheet.as_mut() {
            sheet.fields[sheet.focus].pop();
            return true;
        }
        if let Some(typed) = self.line.as_mut() {
            typed.pop();
            return true;
        }
        if let Some(sheet) = self.link.as_mut() {
            sheet.url.pop();
            return true;
        }
        if self.find_open && self.focus != Focus::Body {
            let over = std::mem::take(&mut self.find_sel);
            let field = if self.focus == Focus::Find { &mut self.find } else { &mut self.replace };
            let changed = if over { !std::mem::take(field).is_empty() } else { field.pop().is_some() };
            if changed && self.focus == Focus::Find {
                self.find_live();
            }
            return changed;
        }
        if self.refuse_edit() {
            return true;
        }
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.delete_backward();
        self.edited();
        true
    }

    /// A delete key: with a selection it deletes the selection, without one the character, word or line the key names.
    fn delete_key(&mut self, ctx: &Ctx, key: select::DeleteKey) -> bool {
        if self.refuse_edit() {
            return true;
        }
        let width = self.geom(ctx).text_w;
        self.ensure_layout(&ctx.ui, width);
        let Some(cache) = self.cache.as_ref() else { return false };
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        if !doc.text.has_selection() {
            let Some(range) = select::delete_range(key, &cache.cells, &cache.layout, doc.text.head()) else { return true };
            doc.text.select(range.start, range.end);
        }
        doc.text.delete_backward();
        self.edited();
        true
    }

    /// Page Up or Page Down: the caret goes a window's height of lines up or down, and with Shift the selection grows to it.
    fn page_move(&mut self, ctx: &Ctx, down: bool, shift: bool) -> bool {
        let geom = self.geom(ctx);
        self.ensure_layout(&ctx.ui, geom.text_w);
        let (aspects, fit) = self.cache.as_ref().map_or((Vec::new(), 0.0), |c| (c.art.aspects.clone(), c.width));
        let measure = view::measure_in(&ctx.ui, &aspects, fit);
        let Some(cache) = self.cache.as_ref() else { return false };
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let lines = select::page_lines(geom.body_bottom - geom.body_top, &cache.layout);
        let to = select::page_target(down, select::NavMods::default(), &cache.cells, &cache.layout, doc.text.head(), &measure, lines);
        let anchor = if shift { doc.text.anchor() } else { to };
        doc.text.select(anchor, to);
        self.follow = true;
        true
    }

    /// A plain key that moves the caret (the page only).
    fn page_key(&mut self, ctx: &Ctx, code: u32) -> bool {
        match select::nav_of_code(code) {
            Some(nav) => self.navigate(ctx, nav, select::NavMods::default()),
            None => false,
        }
    }

    /// Whether the keys belong to the page: no panel is open over it and it has the keyboard.
    fn page_has_keys(&self) -> bool {
        self.focus == Focus::Body
            && self.props.is_none()
            && self.spacing_sheet.is_none()
            && self.line.is_none()
            && self.link.is_none()
            && !self.settings
            && !self.encodings_panel
            && !self.spelling_panel
            && !self.plain_sheet
            && self.fonts.is_none()
            && !self.colors_panel
            && !self.close_alert_here()
    }

    /// A navigation key, with the modifiers a chord carries: Shift extends the selection, Alt or Ctrl move by word, Cmd by
    /// line (and to the ends of the text for Up and Down).
    fn navigate(&mut self, ctx: &Ctx, nav: select::Nav, mods: select::NavMods) -> bool {
        let width = self.geom(ctx).text_w;
        self.move_caret(&ctx.ui, width, nav, mods)
    }

    /// Moves the caret (or, with Shift, the moving end of the selection) for a navigation key.
    fn move_caret(&mut self, ui: &Ui, width: f32, nav: select::Nav, mods: select::NavMods) -> bool {
        self.ensure_layout(ui, width);
        let (aspects, fit) = self.cache.as_ref().map_or((Vec::new(), 0.0), |c| (c.art.aspects.clone(), c.width));
        let measure = view::measure_in(ui, &aspects, fit);
        let Some(cache) = self.cache.as_ref() else { return false };
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        let selection = doc.text.selection();
        let to = select::target(nav, mods, &cache.cells, &cache.layout, doc.text.head(), &selection, &measure);
        let anchor = if mods.shift { doc.text.anchor() } else { to };
        doc.text.select(anchor, to);
        self.follow = true;
        true
    }

    /// A press in the page: it places the caret, or selects the word (a double click) or the paragraph (a triple click)
    /// under the pointer. `px` and `py` are window points. A press on a link in a document that cannot be edited follows
    /// the link instead.
    fn page_down(&mut self, ui: &Ui, geom: &view::Geometry, px: f32, py: f32, t: f64) -> PageDown {
        self.focus = Focus::Body;
        self.ensure_layout(ui, geom.text_w);
        let (aspects, fit) = self.cache.as_ref().map_or((Vec::new(), 0.0), |c| (c.art.aspects.clone(), c.width));
        let measure = view::measure_in(ui, &aspects, fit);
        let Some(cache) = self.cache.as_ref() else { return PageDown::Ignored };
        let (index, under) = select::locate(geom, self.scroll, &cache.layout, &cache.cells, &measure, px, py);
        let unit = select::unit_of(self.clicks.press(t, px, py));
        let Some(doc) = active_doc_mut(&mut self.docs) else { return PageDown::Ignored };
        if doc.read_only {
            if let Some(url) = links::url(doc.text.link_near(index)) {
                return PageDown::Link(url);
            }
        }
        let origin = select::unit_range(&cache.cells, unit, index, under);
        doc.text.select(origin.start, origin.end);
        self.drag = true;
        self.drag_unit = unit;
        self.drag_origin = origin;
        self.follow = true;
        PageDown::Placed
    }

    /// The pointer moved with the button down: the selection grows from where the press began, by the unit it selected.
    fn page_drag(&mut self, ui: &Ui, geom: &view::Geometry, px: f32, py: f32) -> bool {
        self.ensure_layout(ui, geom.text_w);
        let (aspects, fit) = self.cache.as_ref().map_or((Vec::new(), 0.0), |c| (c.art.aspects.clone(), c.width));
        let measure = view::measure_in(ui, &aspects, fit);
        let Some(cache) = self.cache.as_ref() else { return false };
        let (index, under) = select::locate(geom, self.scroll, &cache.layout, &cache.cells, &measure, px, py);
        let (anchor, head) = select::drag_selection(&cache.cells, &self.drag_origin, self.drag_unit, index, under);
        let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
        doc.text.select(anchor, head);
        self.follow = true;
        true
    }

    /// A press on the format bar: a button acts now, a pop-up or the `>>` button opens its menu under itself (the choice
    /// comes back as `Input::ContextAction`, with the menu id of its action).
    fn press_format(&mut self, ctx: &Ctx, status: &view::Status, placed: &ToolbarLayout, hit: format_bar::Hit, x: f32, y: f32) -> bool {
        match format_bar::press(status, placed, hit, x, y) {
            Some(format_bar::Press::Do(action)) => self.perform(ctx, action),
            Some(format_bar::Press::Menu { anchor, rows }) => {
                let items = rows
                    .iter()
                    .map(|row| match row.action {
                        Some(action) => ContextItem::new(action.id().unwrap_or(0), &row.title),
                        None => ContextItem::separator(),
                    })
                    .collect();
                ctx.context_menu(ctx.ui.pt(anchor.0), ctx.ui.pt(anchor.1), items);
                false
            }
            None => false,
        }
    }

    /// A control the window published was activated (assistive technology): the same as pressing it in its middle.
    fn activate(&mut self, ctx: &Ctx, node: u64) -> bool {
        let Some(item) = format_bar::item_of_node(node) else { return false };
        let geom = self.geom(ctx);
        let status = self.status();
        let placed = format_bar::layout(&ctx.ui, &geom, &status);
        let (hit, rect) = if item == format_bar::OVERFLOW {
            (format_bar::Hit::Overflow, placed.overflow_button)
        } else {
            (format_bar::Hit::Item(item), placed.rect_of(item))
        };
        match rect {
            Some(r) => self.press_format(ctx, &status, &placed, hit, r.x + r.w / 2.0, r.y + r.h / 2.0),
            None => false,
        }
    }

    /// Mouse or finger: a press places the caret (or presses a control, or sets a tab stop on the ruler), a drag
    /// extends the selection. `t` is the event time in ms, which tells a double or triple click from separate presses.
    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let (px, py) = (x / ctx.ui.s, y / ctx.ui.s);
        match kind {
            TouchKind::Down => {
                // the close confirmation is a sheet over the whole window: it takes the touches first
                if self.close_alert_here() {
                    return self.touch_close_alert(ctx, px, py);
                }
                if self.fonts.is_some() {
                    return self.touch_fonts(ctx, px, py);
                }
                if self.colors_panel {
                    return self.touch_colors(ctx, px, py);
                }
                if self.props.is_some() {
                    return self.touch_properties(ctx, px, py);
                }
                if self.spacing_sheet.is_some() {
                    return self.touch_spacing(ctx, px, py);
                }
                if self.encodings_panel {
                    return self.touch_encodings(ctx, px, py);
                }
                if self.settings {
                    return self.touch_settings(ctx, px, py);
                }
                if self.spelling_panel {
                    return self.touch_spelling(ctx, px, py);
                }
                if self.plain_sheet {
                    return self.touch_plain(ctx, px, py);
                }
                if self.line.is_some() {
                    return self.touch_line(ctx, px, py);
                }
                if self.link.is_some() {
                    return self.touch_link(ctx, px, py);
                }
                let geom = self.geom(ctx);
                let status = self.status();
                let placed = format_bar::layout(&ctx.ui, &geom, &status);
                if let Some(hit) = format_bar::hit(&placed, px, py) {
                    return self.press_format(ctx, &status, &placed, hit, px, py);
                }
                let find = self.find_buttons(&ctx.ui, &geom);
                match view::hit(&find, &geom, px, py) {
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
                if let Some(tab) = view::ruler_tab(&geom, px, py) {
                    if let Some(doc) = active_doc_mut(&mut self.docs) {
                        doc.text.set_tab_width(tab);
                        self.edited();
                        return true;
                    }
                    return false;
                }
                if py < geom.body_top || py >= geom.body_bottom {
                    return false;
                }
                match self.page_down(&ctx.ui, &geom, px, py, t) {
                    PageDown::Ignored => false,
                    PageDown::Placed => true,
                    PageDown::Link(url) => {
                        ctx.open_url(&url);
                        self.note = Some(format!("Opening {url}"));
                        true
                    }
                }
            }
            TouchKind::Move => {
                if !self.drag {
                    return false;
                }
                let geom = self.geom(ctx);
                self.page_drag(&ctx.ui, &geom, px, py)
            }
            TouchKind::Up | TouchKind::Cancel => {
                self.drag = false;
                false
            }
        }
    }

    /// Edit: the items the shell's own Edit menu (Undo, Cut, Copy, Paste, Select All) does not have. The shell merges this
    /// menu into its Edit, so the bar reads File, Edit, Format, View, Window, Help as the reference's does.
    fn edit_menu(&self) -> AppMenu {
        let doc = active_doc(&self.docs);
        let (has_sel, enabled) = doc.map_or((false, false), |d| (d.text.has_selection(), true));
        let has_find = !self.find.is_empty();
        let rich = doc.is_some_and(|d| d.rich);
        let linked = doc.is_some_and(|d| d.text.link_near(if d.text.has_selection() { d.text.selection().start } else { d.text.head() }) != 0);
        let items = vec![
            menu_item(Action::PasteMatch, "Paste and Match Style", "cmd+alt+shift+v", clipboard::text().is_some()),
            menu_item(Action::Delete, "Delete", "", has_sel),
            AppMenuItem::separator(),
            menu_item(Action::Find, "Find\u{2026}", "cmd+f", true),
            menu_item(Action::FindNext, "Find Next", "cmd+g", has_find),
            menu_item(Action::FindPrevious, "Find Previous", "cmd+shift+g", has_find),
            menu_item(Action::UseSelection, "Use Selection for Find", "cmd+e", has_sel),
            menu_item(Action::JumpToSelection, "Jump to Selection", "cmd+j", enabled),
            menu_item(Action::SelectLine, "Select Line\u{2026}", "cmd+l", enabled),
            menu_item(Action::Replace, "Replace", "", self.find_open && has_find),
            menu_item(Action::ReplaceAll, "Replace All", "", self.find_open && has_find),
            AppMenuItem::separator(),
            menu_item(Action::AddLink, "Add Link\u{2026}", "cmd+k", enabled && rich),
            menu_item(Action::OpenLink, "Open Link", "", linked),
            menu_item(Action::RemoveLink, "Remove Link", "", linked && rich),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Spelling and Grammar"),
            menu_item(Action::SpellNext, "Check Document Now", "cmd+;", enabled).depth(1),
            menu_item(Action::SpellToggle, "Check Spelling While Typing", "", true).checked(self.spelling.enabled).depth(1),
            menu_item(Action::SpellAuto, "Correct Spelling Automatically", "", true).checked(self.opts.spell_auto).depth(1),
            AppMenuItem::separator().depth(1),
            menu_item(Action::SpellCorrect, "Correct Spelling", "", enabled).depth(1),
            menu_item(Action::SpellLearn, "Learn Spelling", "", enabled).depth(1),
            menu_item(Action::SpellIgnore, "Ignore Spelling", "", enabled).depth(1),
            AppMenuItem::separator().depth(1),
            menu_item(Action::ShowSpelling, "Show Spelling and Grammar", "", enabled).depth(1),
            AppMenuItem::submenu("Substitutions"),
            menu_item(Action::SmartQuotes, "Smart Quotes", "", true).checked(self.opts.smart_quotes).depth(1),
            menu_item(Action::SmartDashes, "Smart Dashes", "", true).checked(self.opts.smart_dashes).depth(1),
            menu_item(Action::SmartLinks, "Smart Links", "", true).checked(self.opts.smart_links).depth(1),
            menu_item(Action::TextReplacement, "Text Replacement", "", true).checked(self.opts.text_replacement).depth(1),
            AppMenuItem::submenu("Transformations"),
            menu_item(Action::Transform(Transform::Upper), "Make Upper Case", "", has_sel).depth(1),
            menu_item(Action::Transform(Transform::Lower), "Make Lower Case", "", has_sel).depth(1),
            menu_item(Action::Transform(Transform::Capitalize), "Capitalize", "", has_sel).depth(1),
            AppMenuItem::submenu("Insert"),
            menu_item(Action::InsertPage, "Page Break", "", enabled).depth(1),
            menu_item(Action::InsertLine, "Line Break", "", enabled).depth(1),
            menu_item(Action::InsertParagraph, "Paragraph Break", "", enabled).depth(1),
            menu_item(Action::InsertImage, "Image...", "", enabled).depth(1),
            AppMenuItem::submenu("Speech"),
            menu_item(Action::SpeakStart, "Start Speaking", "", enabled).depth(1),
            menu_item(Action::SpeakStop, "Stop Speaking", "", true).depth(1),
            AppMenuItem::separator(),
            menu_item(Action::Complete, "Complete", "alt+escape", enabled),
        ];
        AppMenu { title: "Edit".into(), items }
    }

    /// Format, laid out as the reference TextEdit's: Font and Text submenus, then the document switches (Make Plain Text,
    /// Prevent Editing, Wrap to Page, Allow Hyphenation) and, for plain text, the encoding. The Font submenu ends with every
    /// installed family (`text::families`); a submenu has an item budget of its own (`proto::MAX_APP_MENU_ITEMS`) and the
    /// shell scrolls a menu taller than the screen. Items the editor cannot do yet (Outline, Styles, Kern, Ligatures,
    /// Tables, Allow Hyphenation) are shown disabled so the menu matches the reference.
    fn format_menu(&self) -> AppMenu {
        let status = self.status();
        let (style, align, rich) = (status.style, status.align, status.rich);
        let enabled = active_doc(&self.docs).is_some();
        let unavailable = |title: &str| AppMenuItem::new(0, title).enabled(false);
        let mut items = vec![
            AppMenuItem::submenu("Font"),
            menu_item(Action::ShowFonts, "Show Fonts", "cmd+t", enabled).checked(self.fonts.is_some()).depth(1),
            menu_item(Action::Bold, "Bold", "cmd+b", rich).checked(style.bold).depth(1),
            menu_item(Action::Italic, "Italic", "cmd+i", rich).checked(style.italic).depth(1),
            menu_item(Action::Underline, "Underline", "cmd+u", rich).checked(style.underline).depth(1),
            menu_item(Action::Strike, "Strikethrough", "", rich).checked(style.strike).depth(1),
            unavailable("Outline").depth(1),
            unavailable("Styles\u{2026}").depth(1),
            AppMenuItem::separator().depth(1),
            menu_item(Action::Bigger, "Bigger", "cmd+=", rich).depth(1),
            menu_item(Action::Smaller, "Smaller", "cmd+-", rich).depth(1),
            AppMenuItem::separator().depth(1),
            AppMenuItem::submenu("Kern").enabled(false).depth(1),
            unavailable("Use Default").depth(2),
            unavailable("Use None").depth(2),
            unavailable("Tighten").depth(2),
            unavailable("Loosen").depth(2),
            AppMenuItem::submenu("Ligatures").enabled(false).depth(1),
            unavailable("Use Default").depth(2),
            unavailable("Use None").depth(2),
            unavailable("Use All").depth(2),
            AppMenuItem::submenu("Baseline").depth(1),
            menu_item(Action::Superscript, "Superscript", "", rich).checked(style.script == 1).depth(2),
            menu_item(Action::Subscript, "Subscript", "", rich).checked(style.script == -1).depth(2),
            AppMenuItem::separator().depth(1),
            menu_item(Action::ShowColors, "Show Colors", "cmd+shift+c", enabled).checked(self.colors_panel).depth(1),
            AppMenuItem::submenu("Color").depth(1),
        ];
        for (index, (name, _)) in text::COLORS.iter().enumerate() {
            items.push(menu_item(Action::Color(index as u8), name, "", rich).checked(style.color == index as u8).depth(2));
        }
        items.push(AppMenuItem::submenu("Highlight").depth(1));
        for (index, (name, _)) in text::COLORS.iter().enumerate() {
            let title = if index == 0 { "None" } else { *name };
            items.push(menu_item(Action::Highlight(index as u8), title, "", rich).checked(style.highlight == index as u8).depth(2));
        }
        items.push(AppMenuItem::separator().depth(1));
        items.push(menu_item(Action::CopyStyle, "Copy Style", "cmd+alt+c", enabled).depth(1));
        items.push(menu_item(Action::PasteStyle, "Paste Style", "cmd+alt+v", rich && self.copied_style.is_some()).depth(1));
        // every installed family, the selection's checked (the Fonts panel and the format bar list them too)
        items.push(AppMenuItem::separator().depth(1));
        for index in 0..text::families().len().min(256) {
            let index = index as u8;
            items.push(menu_item(Action::Font(index), view::family_name(index), "", rich).checked(style.font == index).depth(1));
        }
        items.push(AppMenuItem::submenu("Text"));
        for (title, a) in [("Align Left", Align::Left), ("Center", Align::Center), ("Justify", Align::Justified), ("Align Right", Align::Right)] {
            items.push(menu_item(Action::Align(a), title, "", rich).checked(align == a).depth(1));
        }
        items.push(AppMenuItem::separator().depth(1));
        items.push(AppMenuItem::submenu("Writing Direction").depth(1));
        items.push(menu_item(Action::Direction(false), "Left to Right", "", rich).checked(!style.rtl).depth(2));
        items.push(menu_item(Action::Direction(true), "Right to Left", "", rich).checked(style.rtl).depth(2));
        items.push(AppMenuItem::separator().depth(1));
        let ruler = if self.opts.show_ruler { "Hide Ruler" } else { "Show Ruler" };
        items.push(menu_item(Action::ShowRuler, ruler, "cmd+r", rich).depth(1));
        items.push(menu_item(Action::CopyRuler, "Copy Ruler", "", enabled && rich).depth(1));
        items.push(menu_item(Action::PasteRuler, "Paste Ruler", "", rich && self.ruler.is_some()).depth(1));
        items.push(AppMenuItem::separator().depth(1));
        items.push(menu_item(Action::ShowSpacing, "Spacing\u{2026}", "", rich).depth(1));
        items.push(AppMenuItem::submenu("Lists").depth(1));
        items.push(menu_item(Action::List(text::ListKind::Bullet), "Bullets", "", rich).checked(style.list == text::ListKind::Bullet).depth(2));
        items.push(menu_item(Action::List(text::ListKind::Numbered), "Numbers", "", rich).checked(style.list == text::ListKind::Numbered).depth(2));
        items.push(unavailable("Tables\u{2026}").depth(1));
        items.push(AppMenuItem::separator());
        // the mode switch names the mode it switches to
        if rich {
            items.push(menu_item(Action::MakePlain, "Make Plain Text", "cmd+shift+t", enabled));
        } else {
            items.push(menu_item(Action::MakeRich, "Make Rich Text", "cmd+shift+t", enabled));
        }
        let read_only = active_doc(&self.docs).is_some_and(|d| d.read_only);
        items.push(menu_item(Action::PreventEditing, if read_only { "Allow Editing" } else { "Prevent Editing" }, "", enabled));
        // the text column is the page's width unless Wrap to Window is on; the item names the width it switches to
        items.push(menu_item(Action::WrapToWindow, if self.opts.wrap_window { "Wrap to Page" } else { "Wrap to Window" }, "", enabled));
        items.push(unavailable("Allow Hyphenation"));
        // the encoding is a plain text setting: it is saved with the file, and Reinterpret As repairs a wrong guess
        let current = active_doc(&self.docs).map(|d| d.encoding);
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::submenu("Text Encoding"));
        // the encodings listed in Preferences, and the document's own encoding whether it is listed or not
        for (index, e) in encoding::ALL.iter().enumerate().filter(|(i, e)| self.opts.lists_encoding(*i) || current == Some(**e)) {
            items.push(menu_item(Action::Encoding(index as u8), e.name(), "", enabled && !rich).checked(current == Some(*e)).depth(1));
        }
        items.push(AppMenuItem::separator().depth(1));
        items.push(menu_item(Action::ShowEncodings, "Customize Encodings List\u{2026}", "", true).depth(1));
        items.push(AppMenuItem::submenu("Reinterpret As"));
        for (index, e) in encoding::ALL.iter().enumerate() {
            items.push(menu_item(Action::Reinterpret(index as u8), e.name(), "", enabled && !rich).depth(1));
        }
        AppMenu { title: "Format".into(), items }
    }

    /// View: the zoom and the window background. The ruler and the wrapping width are in Format, as in the reference.
    fn view_menu(&self) -> AppMenu {
        let doc = active_doc(&self.docs);
        let enabled = doc.is_some();
        let items = vec![
            menu_item(Action::ZoomIn, "Zoom In", "cmd+alt+=", enabled && self.zoom < prefs::ZOOMS[prefs::ZOOMS.len() - 1]),
            menu_item(Action::ZoomOut, "Zoom Out", "cmd+alt+-", enabled && self.zoom > prefs::ZOOMS[0]),
            menu_item(Action::ActualSize, "Actual Size", "cmd+0", enabled && self.zoom != 1.0),
            AppMenuItem::separator(),
            menu_item(Action::DarkBackground, "Use Dark Background for Windows", "", true).checked(self.opts.dark_background),
        ];
        AppMenu { title: "View".into(), items }
    }

    /// File: the document controller's items, then Print and Page Setup.
    fn file_menu(&self) -> AppMenu {
        let mut menu = self.docs.file_menu();
        let enabled = active_doc(&self.docs).is_some();
        menu.items.push(AppMenuItem::separator());
        let any_edited = self.docs.ids().into_iter().any(|id| self.docs.is_edited(id));
        menu.items.push(menu_item(Action::SaveAll, "Save All", "cmd+alt+s", any_edited));
        menu.items.push(menu_item(Action::ExportPdf, "Export as PDF\u{2026}", "", enabled));
        menu.items.push(menu_item(Action::ShowProperties, "Show Properties\u{2026}", "cmd+alt+p", enabled));
        menu.items.push(AppMenuItem::separator());
        menu.items.push(AppMenuItem::print(Action::Print.id().unwrap_or(0)).enabled(enabled));
        menu.items.push(menu_item(Action::HeaderFooter, "Print Header and Footer", "", enabled).checked(self.opts.header_footer));
        menu.items.push(menu_item(Action::Rewrap, "Rewrap to Fit Page", "", enabled).checked(self.opts.rewrap));
        menu.items.push(AppMenuItem::submenu("Paper Size"));
        for (index, paper) in print::PAPERS.iter().enumerate() {
            let checked = self.print_info.paper == *paper;
            menu.items.push(menu_item(Action::Paper(index as u8), print::paper_name(*paper), "", true).checked(checked).depth(1));
        }
        menu.items.push(AppMenuItem::submenu("Orientation"));
        let portrait = self.print_info.orientation == Orientation::Portrait;
        menu.items.push(menu_item(Action::Landscape(false), "Portrait", "", true).checked(portrait).depth(1));
        menu.items.push(menu_item(Action::Landscape(true), "Landscape", "", true).checked(!portrait).depth(1));
        // the shell shows this one in the application menu (a Settings\u{2026} item on Cmd+, is not listed in File)
        menu.items.push(menu_item(Action::ShowSettings, "Settings\u{2026}", "cmd+,", true));
        menu
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

impl TextEditApp {
    /// A new untitled document holding `doc`, in a window of its own (`sync_windows` places it).
    fn open_untitled(&mut self, doc: TextDocument) {
        let id = self.docs.new_document();
        if let Some(d) = self.docs.get_mut(id) {
            *d = doc;
        }
        self.revision += 1;
        self.notices.push(Notice::Created(id));
    }
}

/// Whether a dragged item is a file TextEdit opens as a document: a text, rich text or other text-typed file (a picture
/// is inserted into the page instead).
fn is_document_file(item: &dnd::DragItem) -> bool {
    matches!(item.payload, dnd::Payload::File { .. })
        && dnd::image_format_of(item).is_none()
        && item.types.iter().any(|t| bifrost_app::controls::uttype::conforms(t, "public.text"))
}

impl App for TextEditApp {
    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["service.new_window_with_text", "service.open_selected_file"]
    }

    /// Services menu: "New Window Containing Selection" (an untitled document with the text) and "Open Selected File"
    /// (the file's contents as an untitled document, so Save asks for a place to write it).
    fn rpc(&mut self, ctx: Option<&Ctx>, _data_dir: &std::path::Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        if ctx.is_none() {
            return Err(RpcError::new(INTERNAL_ERROR, "TextEdit is not running yet"));
        }
        let call = ServiceCall::from_params(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service call has no text or path"))?;
        let doc = match method {
            "service.new_window_with_text" => {
                let text = call.text.ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service needs text"))?;
                TextDocument::read_bytes(text.as_bytes(), false).map_err(|e| RpcError::new(INTERNAL_ERROR, e.0))?
            }
            "service.open_selected_file" => {
                const MAX_OPEN_BYTES: u64 = 16 << 20;
                let path = call.paths.first().ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service needs a path"))?;
                let len = std::fs::metadata(path).map_err(|e| RpcError::new(INVALID_PARAMS, format!("{path}: {e}")))?.len();
                if len > MAX_OPEN_BYTES {
                    return Err(RpcError::new(INVALID_PARAMS, format!("{path} is too large to open here")));
                }
                let bytes = std::fs::read(path).map_err(|e| RpcError::new(INTERNAL_ERROR, format!("{path}: {e}")))?;
                TextDocument::read_bytes(&bytes, false).map_err(|e| RpcError::new(INTERNAL_ERROR, e.0))?
            }
            _ => return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
        };
        if let Some(ctx) = ctx {
            self.enter(ctx.key_window());
        }
        self.open_untitled(doc);
        if let Some(ctx) = ctx {
            self.sync_windows(ctx);
        }
        Ok(Json::Obj(Vec::new()))
    }

    fn id(&self) -> &str {
        "is.olibuijr.textedit.desktop"
    }

    /// Look Up and the Services menu act on the document's selection (the text is drawn here, not in an SDK field);
    /// with only a caret, on the word at the caret, as Look Up defines it. `Some("")` when there is neither.
    fn selected_text(&mut self, _ctx: &Ctx) -> Option<String> {
        let Some(doc) = active_doc(&self.docs) else { return Some(String::new()) };
        Some(doc.text.selected_text().unwrap_or_else(|| text_state_of(doc).word_at_cursor().to_string()))
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the learned words and the settings are read once the app has its preferences
        self.spelling.load(ctx);
        self.opts = Options::read(&|key| DocumentHost::pref(ctx, key));
        self.spelling.enabled = self.opts.spell_enabled;
        document::set_markup_as_code(self.opts.markup_as_code);
        // pictures dropped from Files or another app land in this window, and text documents open (see `drag`)
        dnd::register_types(ctx, &["public.image", "public.text"]);
        // a window starts with an untitled document, as a new document would. Not while the main window is ordered out
        // after the last document closed: a later configure (appearance, scale) must not bring a window back
        if self.docs.is_empty() && ctx.windows().main_shown() {
            self.start_documents(ctx);
        }
        self.sync_windows(ctx);
    }

    /// The red button and Cmd+W on the main window ask the app instead of quitting it: the main window's document closes as
    /// File > Close closes it. A clean one closes at once, an edited one raises the confirmation on the main window and
    /// closes when it is answered with Save or Don't Save, and with the last document closed the window is ordered out
    /// (`sync_windows`). The other windows close through `window_closed`.
    fn handles_close_request(&self) -> bool {
        true
    }

    fn close_requested(&mut self, ctx: &Ctx) -> bool {
        self.enter(bifrost_app::app_windows::WindowId::MAIN);
        self.routed = true;
        self.menu_action(ctx, item::CLOSE);
        true
    }

    /// The red button on a document window asks as Cmd+W in it does: that window's document closes as File > Close closes
    /// it (a clean one with its window, an edited one raises the confirmation on that window and keeps both until it is
    /// answered). A window this app does not know is ignored, never answered with the main window's document.
    fn window_close_requested(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId) -> bool {
        if window.is_main() {
            return self.close_requested(ctx);
        }
        self.enter(window);
        if self.current != window {
            return false;
        }
        self.routed = true;
        self.menu_action(ctx, item::CLOSE);
        true
    }

    /// Activated (Dock, the app switcher, Search) with no window open, after the last document closed: the shell has put
    /// the main window back, and it shows a new Untitled. The reference shows its open panel here when the cloud-storage
    /// document picker is on; this editor starts with an Untitled on launch, so a reopen does the same (E2E12 2).
    fn reopen(&mut self, ctx: &Ctx) -> bool {
        if self.docs.is_empty() {
            let id = self.docs.new_document();
            self.apply_new_defaults(id);
            // the window shows it from its top (`sync_windows`)
            self.notices.push(Notice::Created(id));
        }
        self.sync_windows(ctx);
        true
    }

    /// The format bar's items are the window's controls (the shell moves the window by its title row, except over these).
    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let geom = self.geom(ctx);
        let status = self.status();
        format_bar::nodes(&format_bar::layout(&ctx.ui, &geom, &status), &status)
    }

    /// Input for the main window, or (`routed`) for the window `window_event` entered. Touches belong to the main window;
    /// keys and commands to the key window.
    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if !std::mem::take(&mut self.routed) {
            let target = if matches!(ev, Input::Touch { .. }) { bifrost_app::app_windows::WindowId::MAIN } else { ctx.key_window() };
            self.enter(target);
        }
        let changed = self.input(ctx, ev);
        self.sync_windows(ctx);
        changed
    }

    fn window_event(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId, ev: Input) -> bool {
        self.enter(window);
        if self.current != window {
            return false;
        }
        // a document window's touches arrive in window points; the page takes px, as the main window's
        let ev = if window.is_main() { ev } else { bifrost_app::app_windows::window_touch_in_px(ev, ctx.ui.pt(1.0)) };
        self.routed = true;
        self.event(ctx, ev)
    }

    /// A menu item chosen while a document window is key acts on that window's document (the default sends it to the
    /// main window only).
    fn window_input(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId, input: bifrost_app::proto::WindowInput) -> bool {
        if let bifrost_app::proto::WindowInput::MenuAction(id) = input {
            self.enter(window);
            self.routed = true;
            return self.menu_action(ctx, id);
        }
        bifrost_app::app_windows::default_window_input(self, ctx, window, input)
    }

    /// The wheel over a document window scrolls that window's page (its location comes in window points).
    fn window_pointer(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
        if window.is_main() {
            return self.pointer(ctx, ev);
        }
        self.enter(window);
        if self.current != window {
            return false;
        }
        match *ev {
            bifrost_app::pointer_events::PointerEvent::Wheel { x, y, dy, .. } => {
                let s = ctx.ui.s;
                self.wheel(ctx, x * s, y * s, dy * s)
            }
            _ => false,
        }
    }

    fn draw_window(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        self.draw_doc_window(ctx, window, s)
    }

    fn window_closed(&mut self, ctx: &Ctx, window: bifrost_app::app_windows::WindowId) -> bool {
        self.doc_window_closed(ctx, window)
    }

    /// The scroll wheel, in place of the drag the shell would emulate for it (which placed the caret and selected text).
    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::WHEEL
    }

    fn pointer(&mut self, ctx: &Ctx, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
        self.enter(bifrost_app::app_windows::WindowId::MAIN);
        match *ev {
            bifrost_app::pointer_events::PointerEvent::Wheel { x, y, dy, .. } => self.wheel(ctx, x, y, dy),
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
        vec![self.file_menu(), self.edit_menu(), self.format_menu(), self.view_menu()]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if !std::mem::take(&mut self.routed) {
            self.enter(ctx.key_window());
        }
        let changed = if is_menu_id(id) {
            self.perform(ctx, Action::Document(id))
        } else {
            match Action::from_id(id) {
                Some(action) => self.perform(ctx, action),
                None => false,
            }
        };
        self.sync_windows(ctx);
        changed
    }

    /// A picture dragged over the window is accepted when it is a picture and the page can take it (rich text, editable);
    /// a text or rich text file is accepted always and opens as a document of its own, as a drop on the Dock icon does.
    /// On the drop the files arrive as documents (see `picture_event`, else the document controller opens them as inbound
    /// files); the drop is answered here.
    fn drag(&mut self, ctx: &Ctx, ev: dnd::Event) -> bool {
        let dnd::Event::Dest { session, phase, items, .. } = ev else { return false };
        let pictures = items.iter().any(|item| dnd::image_format_of(item).is_some());
        let pictures = pictures && active_doc(&self.docs).is_some_and(|d| d.rich && !d.read_only);
        let takes_documents = items.iter().any(is_document_file);
        let takes = pictures || takes_documents;
        match phase {
            DragPhase::Entered | DragPhase::Updated => {
                // only a picture drop is inserted here; a document drop is left to the document controller
                if pictures && !self.drop_sessions.contains(&session) {
                    if self.drop_sessions.len() >= 16 {
                        self.drop_sessions.remove(0);
                    }
                    self.drop_sessions.push(session);
                }
                dnd::reply(ctx, session, if takes { dragop::COPY } else { 0 });
            }
            DragPhase::Exited => self.drop_sessions.retain(|s| *s != session),
            DragPhase::Perform => {
                // the files follow as documents (`picture_event` inserts them); the drop is accepted here
                dnd::performed(ctx, session, takes);
            }
        }
        true
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.enter(ctx.key_window());
        let changed = self.document_event(ctx, ev);
        self.sync_windows(ctx);
        changed
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.docs.tick(ctx, t);
        let asking = self.continue_quit(ctx);
        self.sync_titles(ctx);
        asking
    }

    /// Cmd+Q, the application menu's and the Dock's Quit ask for every edited document (the close confirmation, Save, Don't
    /// Save or Cancel) before the app exits; logout and shutdown still send `Quit` and only autosave.
    fn quit_requested(&mut self, ctx: &Ctx) -> bool {
        let quit = self.docs.request_quit(ctx);
        if !quit {
            self.continue_quit(ctx);
        }
        quit
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
        self.enter(bifrost_app::app_windows::WindowId::MAIN);
        let d = self.paint(ctx, canvas);
        self.enter(ctx.key_window());
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

impl TextEditApp {
    /// The input of the window being handled.
    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Command(command) => match command {
                Command::Cut => self.perform(ctx, Action::Cut),
                Command::Copy => self.perform(ctx, Action::Copy),
                Command::Paste => self.perform(ctx, Action::Paste),
                Command::SelectAll => self.perform(ctx, Action::SelectAll),
                Command::Undo => self.perform(ctx, Action::Undo),
                Command::Redo => self.perform(ctx, Action::Redo),
            },
            // the close confirmation: Return saves, Escape cancels, Cmd+D is Don't Save, Tab moves the focus ring and Space
            // presses the focused button; other keys do nothing behind the sheet
            Input::Key(key) if self.close_alert_here() => match self.docs.alert_key(&key) {
                Some(AlertKey::Answer(choice)) => self.perform(ctx, Action::CloseAnswer(choice as u8)),
                Some(AlertKey::Focus) => true,
                _ => false,
            },
            // the Make Plain Text confirmation takes the same sheet keys: Return is OK, Escape and Cmd+. cancel, Tab moves
            // the focus ring and Space presses the focused button; other keys do nothing behind it
            Input::Key(key) if self.plain_sheet => self.plain_key(ctx, &key),
            Input::Key(KeyInput::Text(s)) => self.type_into(&s),
            Input::Key(KeyInput::Code(code)) => {
                if code & CHORD_FLAG != 0 {
                    // chords are the menus' key equivalents, which the shell routes as menu actions. The editing keys come as
                    // chords too (the manifest's `edit_keys`): the arrows, Home, End and Page Up and Down with Shift, Alt, Ctrl
                    // or Cmd move or extend the selection, Backspace and Delete with Alt, Ctrl or Cmd delete a word or a line,
                    // and Ctrl with A, E, F, B, N, P, D, H, K are the emacs keys (`select::chord_action`).
                    let Some((key, bits)) = KeyInput::Code(code).as_chord() else { return false };
                    let (shift, alt, ctrl, meta) = (bits & CHORD_SHIFT != 0, bits & CHORD_ALT != 0, bits & CHORD_CTRL != 0, bits & CHORD_META != 0);
                    // Cmd (or Ctrl) with A, C, X, V, Z is the standard command, which the shell sends as `Input::Command` when it
                    // knows the app takes it; the chord is what arrives when the app's last report is stale
                    if let Some(action) = chord_edit_action(key, shift, alt, ctrl, meta) {
                        return self.perform(ctx, action);
                    }
                    if !self.page_has_keys() {
                        return false;
                    }
                    return match select::chord_action(key, shift, alt, ctrl, meta) {
                        Some(select::ChordAction::Move(nav, mods)) => self.navigate(ctx, nav, mods),
                        Some(select::ChordAction::Page { down, shift }) => self.page_move(ctx, down, shift),
                        Some(select::ChordAction::Delete(key)) => self.delete_key(ctx, key),
                        None => false,
                    };
                }
                // the properties panel takes the keys while it is open
                if self.props.is_some() {
                    return match code {
                        8 => self.backspace(),
                        13 => self.perform(ctx, Action::PropsDone),
                        1 | 27 => self.perform(ctx, Action::PropsCancel),
                        9 => {
                            if let Some((_, panel)) = self.props.as_mut() {
                                panel.next_field();
                            }
                            true
                        }
                        _ => false,
                    };
                }
                if self.spacing_sheet.is_some() {
                    return match code {
                        8 => self.backspace(),
                        9 => {
                            if let Some(sheet) = self.spacing_sheet.as_mut() {
                                sheet.focus = (sheet.focus + 1) % SpacingSheet::LABELS.len();
                            }
                            true
                        }
                        13 => self.perform(ctx, Action::SpacingDone),
                        1 | 27 => self.perform(ctx, Action::SpacingCancel),
                        _ => false,
                    };
                }
                if self.settings && code == 27 {
                    return self.perform(ctx, Action::SettingsDone);
                }
                // the font panel: Up and Down step through the families, Esc and Return close it
                if self.fonts.is_some() {
                    match code {
                        38 => return self.fonts_step(ctx, -1),
                        40 => return self.fonts_step(ctx, 1),
                        1 | 27 | 13 => return self.perform(ctx, Action::FontsDone),
                        _ => {}
                    }
                }
                if self.colors_panel && matches!(code, 1 | 27 | 13) {
                    return self.perform(ctx, Action::ColorsDone);
                }
                if self.encodings_panel && code == 27 {
                    return self.perform(ctx, Action::EncodingsDone);
                }
                if self.spelling_panel && code == 27 {
                    return self.perform(ctx, Action::SpellingDone);
                }
                if self.line.is_some() {
                    return match code {
                        8 => self.backspace(),
                        13 => self.perform(ctx, Action::LineDone),
                        1 | 27 => self.perform(ctx, Action::LineCancel),
                        _ => false,
                    };
                }
                if self.link.is_some() {
                    return match code {
                        8 => self.backspace(),
                        13 => self.perform(ctx, Action::LinkDone),
                        1 | 27 => self.perform(ctx, Action::LinkCancel),
                        _ => false,
                    };
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
                    1 | 27 => {
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
                            // Tab into a field selects its text, as the native fields do
                            self.find_sel = !(if self.focus == Focus::Find { &self.find } else { &self.replace }).is_empty();
                            true
                        } else {
                            false
                        }
                    }
                    46 if self.focus == Focus::Body => {
                        if self.refuse_edit() {
                            return true;
                        }
                        match active_doc_mut(&mut self.docs) {
                            Some(doc) => {
                                doc.text.delete_forward();
                                self.edited();
                                true
                            }
                            None => false,
                        }
                    }
                    35..=40 if self.focus == Focus::Body => self.page_key(ctx, code),
                    _ => false,
                }
            }
            // shared text (a drop on the Dock icon, or the share sheet) is a `data:` URL and becomes an untitled document.
            // Files do not come here: Files, the Dock and the share sheet hand a file over as a grant, which the document
            // controller opens (`DocumentEvent::Opened`, folders of the RTFD type included). A file URL has no grant, so it is refused.
            Input::Open(url) => match bifrost_app::share::text_from_url(&url).map(|text| TextDocument::read_bytes(text.as_bytes(), false)) {
                Some(Ok(doc)) => {
                    self.open_untitled(doc);
                    true
                }
                _ => false,
            },
            Input::Touch { kind, x, y, t, .. } => self.touch(ctx, kind, x, y, t),
            // the choice in a menu the format bar opened (a font, a size, a colour...), by the menu id of its action
            Input::ContextAction(id) => match Action::from_id(id) {
                Some(action) => self.perform(ctx, action),
                None => false,
            },
            Input::Activate(node) => self.activate(ctx, node),
            _ => false,
        }
    }

    fn document_event(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let ev = match self.export_event(ctx, ev) {
            Ok(handled) => return handled,
            Err(ev) => ev,
        };
        let ev = match self.picture_event(ctx, ev) {
            Ok(handled) => return handled,
            Err(ev) => ev,
        };
        match self.docs.handle(ctx, ev) {
            Some(notices) => {
                self.after_documents(&notices);
                true
            }
            None => false,
        }
    }

    /// Draws the window being handled (`doc_windows`): the main window from `draw_canvas`, a document window from
    /// `draw_window`.
    pub(crate) fn paint(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas) -> Damage {
        let (w, h) = canvas.size();
        let ui = &ctx.ui;
        let geom = self.geom(ctx);
        let find = self.find_buttons(ui, &geom);
        view::draw_bar(canvas, ui, &geom);
        view::draw_title(canvas, ui, &geom, &self.window_title());
        format_bar::draw(canvas, ui, &geom, &self.status(), None);
        let tab = active_doc(&self.docs).map_or(0.0, |d| d.text.tab_width());
        view::draw_ruler(canvas, ui, &geom, tab);
        if self.find_open {
            view::draw_find(canvas, ui, &geom, &find, &self.find, &self.replace, self.focus, self.find_sel);
        }
        self.ensure_layout(ui, geom.text_w);
        let matches = self.find_matches();
        view::page_clip(canvas, ui, &geom, self.opts.dark_background);
        let view_h = geom.body_bottom - geom.body_top;
        let mut caret_now = None;
        if let (Some(doc), Some(cache)) = (active_doc(&self.docs), self.cache.as_ref()) {
            let sel = doc.text.selection();
            let head = doc.text.head();
            let caret_at = if sel.is_empty() { head } else { sel.start };
            if self.follow {
                let (_, top, height) = cache.layout.caret(&cache.cells, head, &view::measure_in(ui, &cache.art.aspects, cache.layout.width));
                if top < self.scroll {
                    self.scroll = top;
                } else if top + height > self.scroll + view_h {
                    self.scroll = top + height - view_h;
                }
                self.follow = false;
            }
            let max_scroll = (cache.layout.height - view_h).max(0.0);
            self.scroll = self.scroll.clamp(0.0, max_scroll);
            view::draw_text(
                canvas,
                ui,
                &geom,
                &cache.layout,
                &cache.cells,
                &cache.art,
                sel,
                head,
                self.scroll,
                self.focus == Focus::Body,
                &cache.misspelled,
                &matches,
                self.opts.dark_background,
            );
            // where the caret (or the selection's start) is in the window, in points, as the page draws it
            let (x, top, height) = cache.layout.caret(&cache.cells, caret_at, &view::measure_in(ui, &cache.art.aspects, cache.layout.width));
            caret_now = Some(caret_in_window(&geom, self.scroll, (x, top, height)));
        }
        canvas.pop_clip();
        // the key window's caret goes to the shell (`firstRectForCharacterRange:`): Look Up opens under the selection
        if let Some(rect) = caret_now.filter(|_| ctx.key_window() == self.current) {
            if self.caret_published != Some((self.current, rect)) {
                ctx.caret_rect(rect.0, rect.1, rect.2, rect.3);
                self.caret_published = Some((self.current, rect));
            }
        }
        if let Some((_, panel)) = &self.props {
            let layout = properties::layout(geom.width, geom.body_top);
            let buttons = view::properties_buttons(ui, &layout);
            let rows: Vec<(&str, &str)> = properties::LABELS.iter().enumerate().map(|(i, label)| (*label, panel.draft.get(i))).collect();
            view::draw_sheet(canvas, ui, &layout, "Document Properties", &rows, panel.focus, &buttons);
        }
        if let Some(sheet) = &self.link {
            let layout = properties::layout_rows(geom.width, geom.body_top, 1);
            let buttons = view::link_buttons(ui, &layout, sheet.had_link);
            view::draw_sheet(canvas, ui, &layout, "Link", &[("Address", sheet.url.as_str())], 0, &buttons);
        }
        if let Some(sheet) = self.plain_alert_sheet() {
            let (parent, bar) = close_sheet_parent(&geom);
            bifrost_app::widgets::sheet(canvas, ui, parent, bar, &sheet, 1.0, None);
        }
        // the close confirmation of this window's document, over everything else in the window
        if let Some(sheet) = self.close_sheet() {
            let (parent, bar) = close_sheet_parent(&geom);
            bifrost_app::widgets::sheet(canvas, ui, parent, bar, &sheet, 1.0, None);
        }
        if let Some(typed) = &self.line {
            let layout = properties::layout_rows(geom.width, geom.body_top, 1);
            let buttons = view::line_buttons(ui, &layout);
            view::draw_sheet(canvas, ui, &layout, "Select Line", &[("Line", typed.as_str())], 0, &buttons);
        }
        if self.spelling_panel {
            let layout = properties::layout_rows(geom.width, geom.body_top, 2);
            let buttons = view::spelling_buttons(ui, &layout);
            let (word, guess) = self.spelling_fields();
            let rows = [("Misspelled", word.as_str()), ("Change to", guess.as_str())];
            view::draw_sheet(canvas, ui, &layout, "Spelling and Grammar", &rows, usize::MAX, &buttons);
        }
        if let Some(sheet) = &self.spacing_sheet {
            let layout = properties::layout_rows(geom.width, geom.body_top, 3);
            let buttons = view::spacing_buttons(ui, &layout);
            let rows = [
                (SpacingSheet::LABELS[0], sheet.fields[0].as_str()),
                (SpacingSheet::LABELS[1], sheet.fields[1].as_str()),
                (SpacingSheet::LABELS[2], sheet.fields[2].as_str()),
            ];
            view::draw_sheet(canvas, ui, &layout, "Spacing", &rows, sheet.focus, &buttons);
        }
        if self.settings {
            let layout = properties::list_layout(geom.width, geom.body_top, prefs::Setting::ALL.len());
            let buttons = view::list_buttons(ui, &layout, Action::SettingsDone);
            let rows: Vec<(&str, String)> = prefs::Setting::ALL.iter().map(|s| (s.label(), s.value(&self.opts))).collect();
            view::draw_list(canvas, ui, &layout, "Settings", &rows, &buttons);
        }
        if self.encodings_panel {
            let layout = properties::list_layout(geom.width, geom.body_top, encoding::ALL.len());
            let buttons = view::list_buttons(ui, &layout, Action::EncodingsDone);
            let rows: Vec<(&str, String)> = encoding::ALL
                .iter()
                .enumerate()
                .map(|(i, e)| (e.name(), (if self.opts.lists_encoding(i) { "Listed" } else { "Not listed" }).to_string()))
                .collect();
            view::draw_list(canvas, ui, &layout, "Customize Encodings List", &rows, &buttons);
        }
        if let Some(panel) = &self.fonts {
            let layout = font_panel::fonts_layout(geom.width, geom.body_top);
            let style = self.status().style;
            font_panel::draw_fonts(canvas, ui, &layout, panel, style.font, style.bold as u8 + 2 * style.italic as u8, style.size);
        }
        if self.colors_panel {
            let layout = font_panel::colors_layout(geom.width, geom.body_top);
            font_panel::draw_colors(canvas, ui, &layout, self.status().style.color);
        }
        let (words, chars) = active_doc(&self.docs).map_or((0, 0), |d| (d.text.word_count(), d.text.char_count()));
        view::draw_status(canvas, ui, &geom, words, chars, self.note.as_deref());
        let mut d = Damage::new(w as usize, h as usize);
        d.all();
        d
    }
}

/// The standard editing command a Cmd (or Ctrl) chord names: A select all, C copy, X cut, V paste, Z undo (Shift redo).
/// Ctrl+A is the line start (`select::chord_action`), not Select All.
fn chord_edit_action(key: u32, shift: bool, alt: bool, ctrl: bool, meta: bool) -> Option<Action> {
    if alt || !(meta || ctrl) {
        return None;
    }
    match key {
        46 if !shift => Some(Action::Copy),
        45 if !shift => Some(Action::Cut),
        47 if !shift => Some(Action::Paste),
        30 if meta && !ctrl && !shift => Some(Action::SelectAll),
        44 => Some(if shift { Action::Redo } else { Action::Undo }),
        _ => None,
    }
}

fn main() {
    bifrost_app::run(TextEditApp::new());
}

/// The window a document-modal sheet covers (points) and the height of the title bar it hangs from: the unified title
/// row, above the format bar.
fn close_sheet_parent(geom: &view::Geometry) -> (bifrost_app::controls::Rect, f32) {
    (bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: geom.width, h: geom.height }, bifrost_app::layout::UNIFIED_BAR_PT)
}

/// The caret `(x, top, height)` of the page's layout as a rectangle in the window, in points: the text column starts at
/// `text_x` and the page at `body_top`, scrolled by `scroll`. The caret is one point wide.
fn caret_in_window(geom: &view::Geometry, scroll: f32, (x, top, height): (f32, f32, f32)) -> (f32, f32, f32, f32) {
    (geom.text_x + x, geom.body_top - scroll + top, 1.0, height)
}

#[cfg(test)]
mod tests;
