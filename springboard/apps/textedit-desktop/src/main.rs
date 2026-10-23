//! Text Editor as an installed BifrOSt app: plain and rich text documents, opened and saved as RTF, HTML or plain text
//! through the shell's document panels, with character and paragraph formatting, fonts, find and replace, spelling, a
//! tab ruler, printing with page setup, a word count, and undo and redo.
//!
//! The document model is in `text.rs`, the file formats in `rtf.rs`, `html.rs` and `document.rs`, the line breaking in
//! `layout.rs`, the spelling in `spell.rs`, the PDF for printing in `print.rs`, and the window in `view.rs`. This file
//! wires them to the SDK: the document controller, menus, keys, touches and the canvas.

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
use bifrost_app::document::{is_menu_id, item, DocId, DocumentController, DocumentHost, Notice};
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
            drop_sessions: Vec::new(),
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
        let ruler = self.opts.show_ruler && active_doc(&self.docs).is_some_and(|d| d.rich);
        view::geometry_with(width_px as f32 / ui.s, height_px as f32 / ui.s, self.find_open, ruler, self.opts.wrap_window)
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
        self.docs.active().and_then(|id| self.docs.window_title(id)).unwrap_or_else(|| "Text Editor".to_string())
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
                || self.plain_sheet;
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
                if (index as usize) >= text::FAMILIES.len() {
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
            Action::Undo => doc.text.undo(),
            Action::Redo => doc.text.redo(),
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
                // attachments are dropped only after the confirmation sheet says so
                if doc.rich && doc.text.has_attachments() {
                    self.plain_sheet = true;
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
                self.note = Some(format!("\"{typed}\" is not an address Text Editor can link to"));
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
            let window = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h).text_w;
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

    /// The Make Plain Text confirmation's touches: the buttons it shows.
    fn touch_plain(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let layout = properties::layout_rows(geom.width, geom.body_top, 1);
        let buttons = view::plain_buttons(&ctx.ui, &layout);
        match buttons.iter().find(|b| b.rect.contains(px, py)).and_then(|b| b.action) {
            Some(action) => self.perform(ctx, action),
            None => true,
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
        if printable.is_empty() {
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
        let width = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h).text_w;
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
    }

    /// A navigation key, with the modifiers a chord carries: Shift extends the selection, Alt or Ctrl move by word, Cmd by
    /// line (and to the ends of the text for Up and Down).
    fn navigate(&mut self, ctx: &Ctx, nav: select::Nav, mods: select::NavMods) -> bool {
        let width = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h).text_w;
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
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
                let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
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
                let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
                self.page_drag(&ctx.ui, &geom, px, py)
            }
            TouchKind::Up | TouchKind::Cancel => {
                self.drag = false;
                false
            }
        }
    }

    fn text_menu(&self) -> AppMenu {
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
        AppMenu { title: "Text".into(), items }
    }

    fn format_menu(&self) -> AppMenu {
        let status = self.status();
        let (style, align, rich) = (status.style, status.align, status.rich);
        let enabled = active_doc(&self.docs).is_some();
        let mut items = vec![
            menu_item(Action::Bold, "Bold", "cmd+b", rich).checked(style.bold),
            menu_item(Action::Italic, "Italic", "cmd+i", rich).checked(style.italic),
            menu_item(Action::Underline, "Underline", "cmd+u", rich).checked(style.underline),
            menu_item(Action::Strike, "Strikethrough", "", rich).checked(style.strike),
            AppMenuItem::separator(),
            menu_item(Action::Bigger, "Bigger", "cmd+=", rich),
            menu_item(Action::Smaller, "Smaller", "cmd+-", rich),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Baseline"),
            menu_item(Action::Superscript, "Superscript", "", rich).checked(style.script == 1).depth(1),
            menu_item(Action::Subscript, "Subscript", "", rich).checked(style.script == -1).depth(1),
            AppMenuItem::submenu("Highlight"),
        ];
        for (index, (name, _)) in text::COLORS.iter().enumerate() {
            let title = if index == 0 { "None" } else { *name };
            items.push(menu_item(Action::Highlight(index as u8), title, "", rich).checked(style.highlight == index as u8).depth(1));
        }
        items.push(AppMenuItem::submenu("Font"));
        items.push(menu_item(Action::ShowFonts, "Show Fonts", "cmd+t", enabled).checked(self.fonts.is_some()).depth(1));
        items.push(menu_item(Action::ShowColors, "Show Colors", "cmd+shift+c", enabled).checked(self.colors_panel).depth(1));
        items.push(AppMenuItem::separator().depth(1));
        for index in format_bar::font_menu_families(style.font) {
            items.push(menu_item(Action::Font(index), view::family_name(index), "", rich).checked(style.font == index).depth(1));
        }
        items.push(AppMenuItem::submenu("Text"));
        for (title, a) in [("Align Left", Align::Left), ("Center", Align::Center), ("Align Right", Align::Right), ("Justify", Align::Justified)] {
            items.push(menu_item(Action::Align(a), title, "", rich).checked(align == a).depth(1));
        }
        items.push(AppMenuItem::separator().depth(1));
        items.push(menu_item(Action::Direction(false), "Left to Right", "", rich).checked(!style.rtl).depth(1));
        items.push(menu_item(Action::Direction(true), "Right to Left", "", rich).checked(style.rtl).depth(1));
        items.push(AppMenuItem::separator().depth(1));
        for index in 0..text::SPACINGS.len() as u8 {
            let title = format!("Spacing {}", view::spacing_label(index));
            items.push(menu_item(Action::Spacing(index), &title, "", rich).checked(style.spacing == index).depth(1));
        }
        items.push(menu_item(Action::ShowSpacing, "Spacing\u{2026}", "", rich).depth(1));
        items.push(AppMenuItem::separator().depth(1));
        items.push(menu_item(Action::CopyRuler, "Copy Ruler", "", enabled && rich).depth(1));
        items.push(menu_item(Action::PasteRuler, "Paste Ruler", "", rich && self.ruler.is_some()).depth(1));
        items.push(menu_item(Action::List(text::ListKind::Bullet), "Bullets", "", rich).checked(style.list == text::ListKind::Bullet).depth(1));
        items.push(menu_item(Action::List(text::ListKind::Numbered), "Numbers", "", rich).checked(style.list == text::ListKind::Numbered).depth(1));
        items.push(AppMenuItem::submenu("Color"));
        for (index, (name, _)) in text::COLORS.iter().enumerate() {
            items.push(menu_item(Action::Color(index as u8), name, "", rich).checked(style.color == index as u8).depth(1));
        }
        items.push(AppMenuItem::separator());
        items.push(menu_item(Action::CopyStyle, "Copy Style", "cmd+alt+c", enabled));
        items.push(menu_item(Action::PasteStyle, "Paste Style", "cmd+alt+v", rich && self.copied_style.is_some()));
        items.push(AppMenuItem::separator());
        // the mode switch names the mode it switches to
        if rich {
            items.push(menu_item(Action::MakePlain, "Make Plain Text", "", enabled));
        } else {
            items.push(menu_item(Action::MakeRich, "Make Rich Text", "", enabled));
        }
        let read_only = active_doc(&self.docs).is_some_and(|d| d.read_only);
        items.push(menu_item(Action::PreventEditing, if read_only { "Allow Editing" } else { "Prevent Editing" }, "", enabled));
        // the text column is the page's width unless Wrap to Window is on (View has the same item)
        items.push(menu_item(Action::WrapToWindow, "Wrap to Page", "", enabled).checked(!self.opts.wrap_window));
        // the encoding is a plain text setting: it is saved with the file, and Reinterpret As repairs a wrong guess
        let current = active_doc(&self.docs).map(|d| d.encoding);
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::submenu("Text Encoding"));
        // the encodings listed in Preferences, and the document's own encoding whether it is listed or not
        for (index, e) in encoding::ALL.iter().enumerate().filter(|(i, e)| self.opts.lists_encoding(*i) || current == Some(**e)) {
            items.push(menu_item(Action::Encoding(index as u8), e.name(), "", enabled && !rich).checked(current == Some(*e)).depth(1));
        }
        items.push(AppMenuItem::submenu("Reinterpret As"));
        for (index, e) in encoding::ALL.iter().enumerate() {
            items.push(menu_item(Action::Reinterpret(index as u8), e.name(), "", enabled && !rich).depth(1));
        }
        AppMenu { title: "Format".into(), items }
    }

    /// View: the ruler, the wrapping width and the zoom.
    fn view_menu(&self) -> AppMenu {
        let doc = active_doc(&self.docs);
        let enabled = doc.is_some();
        let rich = doc.is_some_and(|d| d.rich);
        let ruler = if self.opts.show_ruler { "Hide Ruler" } else { "Show Ruler" };
        let wrap = if self.opts.wrap_window { "Wrap to Page" } else { "Wrap to Window" };
        let items = vec![
            menu_item(Action::ShowRuler, ruler, "cmd+r", rich),
            menu_item(Action::WrapToWindow, wrap, "", enabled),
            AppMenuItem::separator(),
            menu_item(Action::ZoomIn, "Zoom In", "cmd+alt+=", enabled && self.zoom < prefs::ZOOMS[prefs::ZOOMS.len() - 1]),
            menu_item(Action::ZoomOut, "Zoom Out", "cmd+alt+-", enabled && self.zoom > prefs::ZOOMS[0]),
            menu_item(Action::ActualSize, "Actual Size", "cmd+0", enabled && self.zoom != 1.0),
            AppMenuItem::separator(),
            menu_item(Action::DarkBackground, "Use Dark Background for Windows", "", true).checked(self.opts.dark_background),
        ];
        AppMenu { title: "View".into(), items }
    }

    /// Preferences: the format of new documents. The other options live in the menus they belong to (spelling and
    /// substitutions in Edit, the ruler and wrapping in View, the header and footer in File) and are kept in the
    /// preferences too.
    fn preferences_menu(&self) -> AppMenu {
        let mut items = vec![
            menu_item(Action::ShowSettings, "Preferences\u{2026}", "cmd+,", true),
            AppMenuItem::submenu("New Documents"),
            menu_item(Action::NewPlain(false), "Rich Text", "", true).checked(!self.opts.new_plain).depth(1),
            menu_item(Action::NewPlain(true), "Plain Text", "", true).checked(self.opts.new_plain).depth(1),
            AppMenuItem::submenu("Plain Text Encoding"),
        ];
        for (index, e) in encoding::ALL.iter().enumerate() {
            items.push(menu_item(Action::DefaultEncoding(index as u8), e.name(), "", true).checked(self.opts.default_encoding == *e).depth(1));
        }
        items.push(menu_item(Action::ShowEncodings, "Customize Encodings List\u{2026}", "", true));
        items.push(AppMenuItem::submenu("Default Font"));
        for index in 0..text::FAMILIES.len() as u8 {
            items.push(menu_item(Action::DefaultFont(index), view::family_name(index), "", true).checked(self.opts.default_font == index).depth(1));
        }
        items.push(menu_item(Action::MarkupAsCode, "Show RTF and HTML Files as Code", "", true).checked(self.opts.markup_as_code));
        AppMenu { title: "Preferences".into(), items }
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
        menu.items.push(menu_item(Action::Print, "Print\u{2026}", "cmd+p", enabled));
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
    /// A new untitled document holding `doc`, shown from its top (as `after_documents` does for a created document).
    fn open_untitled(&mut self, doc: TextDocument) {
        let id = self.docs.new_document();
        if let Some(d) = self.docs.get_mut(id) {
            *d = doc;
        }
        self.scroll = 0.0;
        self.follow = true;
        self.revision += 1;
    }
}

impl App for TextEditApp {
    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["service.new_window_with_text", "service.open_selected_file"]
    }

    /// Services menu: "New Window Containing Selection" (an untitled document with the text) and "Open Selected File"
    /// (the file's contents as an untitled document, so Save asks for a place to write it).
    fn rpc(&mut self, ctx: Option<&Ctx>, _data_dir: &std::path::Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        if ctx.is_none() {
            return Err(RpcError::new(INTERNAL_ERROR, "Text Editor is not running yet"));
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
        self.open_untitled(doc);
        Ok(Json::Obj(Vec::new()))
    }

    fn id(&self) -> &str {
        "is.olibuijr.textedit.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the learned words and the settings are read once the app has its preferences
        self.spelling.load(ctx);
        self.opts = Options::read(&|key| DocumentHost::pref(ctx, key));
        self.spelling.enabled = self.opts.spell_enabled;
        document::set_markup_as_code(self.opts.markup_as_code);
        // pictures dropped from Files or another app land in this window (see `drag`)
        dnd::register_types(ctx, &["public.image"]);
        // a window starts with an untitled document, as a new document would
        if self.docs.is_empty() {
            let id = self.docs.new_document();
            self.apply_new_defaults(id);
        }
    }

    /// The format bar's items are the window's controls (the shell moves the window by its title row, except over these).
    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let status = self.status();
        format_bar::nodes(&format_bar::layout(&ctx.ui, &geom, &status), &status)
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
                if self.plain_sheet {
                    return match code {
                        13 => self.perform(ctx, Action::PlainDone),
                        1 | 27 => self.perform(ctx, Action::PlainCancel),
                        _ => false,
                    };
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

    /// The scroll wheel, in place of the drag the shell would emulate for it (which placed the caret and selected text).
    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::WHEEL
    }

    fn pointer(&mut self, ctx: &Ctx, ev: &bifrost_app::pointer_events::PointerEvent) -> bool {
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
        vec![self.file_menu(), self.text_menu(), self.format_menu(), self.view_menu(), self.preferences_menu()]
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

    /// A picture dragged over the window is accepted when it is a picture and the page can take it (rich text, editable).
    /// On the drop the files arrive as documents (see `picture_event`); the drop is answered here.
    fn drag(&mut self, ctx: &Ctx, ev: dnd::Event) -> bool {
        let dnd::Event::Dest { session, phase, items, .. } = ev else { return false };
        let pictures = items.iter().any(|item| dnd::image_format_of(item).is_some());
        let takes = pictures && active_doc(&self.docs).is_some_and(|d| d.rich && !d.read_only);
        match phase {
            DragPhase::Entered | DragPhase::Updated => {
                if takes && !self.drop_sessions.contains(&session) {
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
        if let (Some(doc), Some(cache)) = (active_doc(&self.docs), self.cache.as_ref()) {
            let sel = doc.text.selection();
            let head = doc.text.head();
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
        }
        canvas.pop_clip();
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
        if self.plain_sheet {
            let layout = properties::layout_rows(geom.width, geom.body_top, 1);
            let buttons = view::plain_buttons(ui, &layout);
            view::draw_confirm(canvas, ui, &layout, "Make Plain Text", "Attachments will be removed from the document.", &buttons);
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
            view::draw_list(canvas, ui, &layout, "Preferences", &rows, &buttons);
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

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
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

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::document::Document;
    use text::{Cell, Style};

    fn doc(s: &str) -> TextDocument {
        let mut d = TextDocument::untitled();
        d.text.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
        d
    }

    #[test]
    fn an_untitled_document_holds_the_text_it_is_given_and_shows_from_the_top() {
        let mut app = TextEditApp::new();
        app.scroll = 40.0;
        app.open_untitled(doc("shared text"));
        assert_eq!(active_doc(&app.docs).unwrap().text.text(), "shared text");
        assert!(app.follow);
        assert_eq!(app.scroll, 0.0);
    }

    #[test]
    fn spelling_works_on_the_word_at_the_caret_or_a_single_word_selection() {
        let mut d = doc("helo world");
        d.text.select(2, 2);
        assert_eq!(word_here(&d), Some((0..4, "helo".to_string())));
        d.text.select(5, 5);
        assert_eq!(word_here(&d), Some((5..10, "world".to_string())));
        d.text.select(0, 10);
        assert_eq!(word_here(&d), None, "a selection of two words is not a word");
        d.text.select(4, 4);
        assert_eq!(word_here(&d), Some((0..4, "helo".to_string())), "the end of a word still belongs to it");
    }

    #[test]
    fn prevent_editing_refuses_text_changes_but_not_copying_or_view_commands() {
        assert!(edits_text(Action::Paste));
        assert!(edits_text(Action::Delete));
        assert!(edits_text(Action::Transform(Transform::Upper)));
        assert!(edits_text(Action::Strike));
        assert!(edits_text(Action::Undo));
        assert!(!edits_text(Action::Copy));
        assert!(!edits_text(Action::Find));
        assert!(!edits_text(Action::ZoomIn));
        assert!(!edits_text(Action::PreventEditing));
        assert!(!edits_text(Action::CopyStyle));
    }

    #[test]
    fn zooming_scales_sizes_only() {
        let cells = vec![Cell { ch: 'a', style: Style { size: 12.0, ..Style::default() } }];
        let big = zoomed(&cells, 2.0);
        assert_eq!(big[0].style.size, 24.0);
        assert_eq!(big[0].ch, 'a');
        assert_eq!(cells[0].style.size, 12.0, "the document keeps its own size");
    }

    #[test]
    fn formatting_commands_include_the_new_character_styles() {
        assert!(is_formatting(Action::Strike));
        assert!(is_formatting(Action::Superscript));
        assert!(is_formatting(Action::Highlight(2)));
        assert!(is_formatting(Action::PasteStyle));
        assert!(!is_formatting(Action::CopyStyle));
    }

    fn app_with_plain(bytes: &[u8]) -> TextEditApp {
        let mut app = TextEditApp::new();
        let id = app.docs.new_document();
        *app.docs.get_mut(id).unwrap() = TextDocument::read_bytes(bytes, false).unwrap();
        app
    }

    #[test]
    fn choosing_an_encoding_marks_a_plain_document_edited_and_only_when_it_changes() {
        let mut app = app_with_plain(b"abc");
        let id = app.docs.active().unwrap();
        assert!(!app.docs.is_edited(id));
        assert!(!app.set_encoding(Encoding::Utf8), "the encoding it already has changes nothing");
        assert!(!app.docs.is_edited(id));
        assert!(app.set_encoding(Encoding::Utf16Le));
        assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Utf16Le);
        assert!(app.docs.is_edited(id));
    }

    #[test]
    fn rich_text_and_read_only_documents_refuse_an_encoding() {
        let mut app = TextEditApp::new();
        app.docs.new_document();
        assert!(app.set_encoding(Encoding::Latin1));
        assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Utf8, "rich text keeps its encoding");
        assert!(app.note.as_deref().is_some_and(|n| n.contains("Make Plain Text")));
        let mut plain = app_with_plain(b"abc");
        active_doc_mut(&mut plain.docs).unwrap().read_only = true;
        assert!(plain.set_encoding(Encoding::Latin1));
        assert_eq!(active_doc(&plain.docs).unwrap().encoding, Encoding::Utf8);
    }

    #[test]
    fn reinterpreting_rereads_the_text_and_is_undoable() {
        // a classic-set file that was guessed as Windows text
        let mut app = app_with_plain(&[0x8e, b'x']);
        assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Windows1252);
        assert_eq!(active_doc(&app.docs).unwrap().text.text(), "\u{17d}x");
        assert!(app.reinterpret(Encoding::Roman));
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(doc.text.text(), "éx");
        assert_eq!(doc.encoding, Encoding::Roman);
        assert!(doc.text.can_undo());
        // text that is not bytes of the old set cannot be reread
        let mut app = app_with_plain("→".as_bytes());
        active_doc_mut(&mut app.docs).unwrap().encoding = Encoding::Latin1;
        assert!(app.reinterpret(Encoding::Roman));
        assert_eq!(active_doc(&app.docs).unwrap().text.text(), "→");
        assert!(app.note.as_deref().is_some_and(|n| n.contains("cannot be read")));
    }

    #[test]
    fn the_properties_panel_applies_on_done_and_discards_on_cancel() {
        let mut app = app_with_plain(b"abc");
        let id = app.docs.active().unwrap();
        assert!(app.open_properties());
        app.props.as_mut().unwrap().1.type_text("  Plan ");
        assert!(app.close_properties(false));
        assert!(app.props.is_none());
        assert!(active_doc(&app.docs).unwrap().properties.is_empty(), "Cancel keeps nothing");
        assert!(!app.docs.is_edited(id));

        app.open_properties();
        app.props.as_mut().unwrap().1.type_text("  Plan ");
        app.props.as_mut().unwrap().1.next_field();
        app.props.as_mut().unwrap().1.type_text("Ada");
        assert!(app.close_properties(true));
        let props = &active_doc(&app.docs).unwrap().properties;
        assert_eq!((props.title.as_str(), props.author.as_str()), ("Plan", "Ada"));
        assert!(app.docs.is_edited(id), "changed properties are unsaved changes");

        // unchanged properties are not an edit
        let mut quiet = app_with_plain(b"x");
        let qid = quiet.docs.active().unwrap();
        quiet.open_properties();
        quiet.close_properties(true);
        assert!(!quiet.docs.is_edited(qid));
    }

    #[test]
    fn properties_cannot_change_while_editing_is_prevented() {
        let mut app = app_with_plain(b"abc");
        active_doc_mut(&mut app.docs).unwrap().read_only = true;
        app.open_properties();
        app.props.as_mut().unwrap().1.type_text("Title");
        app.close_properties(true);
        assert!(active_doc(&app.docs).unwrap().properties.is_empty());
    }

    fn app_with_text(s: &str, select: Range<usize>) -> TextEditApp {
        let mut app = TextEditApp::new();
        let id = app.docs.new_document();
        let doc = app.docs.get_mut(id).unwrap();
        doc.text.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
        doc.text.select(select.start, select.end);
        app
    }

    #[test]
    fn adding_a_link_links_the_selection_and_remove_link_takes_it_off() {
        let mut app = app_with_text("see the docs", 4..12);
        let id = app.docs.active().unwrap();
        assert!(app.open_link_sheet());
        assert!(!app.link.as_ref().unwrap().had_link);
        app.link.as_mut().unwrap().url = "example.com/docs".into();
        assert!(app.finish_link(true));
        assert!(app.link.is_none());
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(links::url(doc.text.link_at(5)).as_deref(), Some("https://example.com/docs"));
        assert_eq!(doc.text.link_at(3), 0);
        assert!(app.docs.is_edited(id));

        // with the caret inside the link the panel opens on it, with its address
        active_doc_mut(&mut app.docs).unwrap().text.select(6, 6);
        assert!(app.open_link_sheet());
        let sheet = app.link.as_ref().unwrap();
        assert_eq!((sheet.url.as_str(), sheet.had_link, sheet.range.clone()), ("https://example.com/docs", true, 4..12));
        assert!(app.finish_link(false));
        assert_eq!(active_doc(&app.docs).unwrap().text.link_at(5), 0);
    }

    #[test]
    fn an_empty_address_removes_the_link_and_a_bad_one_stays_in_the_panel() {
        let mut app = app_with_text("a link here", 2..6);
        app.open_link_sheet();
        app.link.as_mut().unwrap().url = "two words".into();
        assert!(app.finish_link(true));
        assert!(app.link.is_some(), "the panel stays open for another try");
        assert!(app.note.as_deref().is_some_and(|n| n.contains("not an address")));
        assert_eq!(active_doc(&app.docs).unwrap().text.link_at(3), 0);
        app.link.as_mut().unwrap().url = "https://example.org".into();
        assert!(app.finish_link(true));
        assert_ne!(active_doc(&app.docs).unwrap().text.link_at(3), 0);
        app.open_link_sheet();
        app.link.as_mut().unwrap().url.clear();
        assert!(app.finish_link(true));
        assert_eq!(active_doc(&app.docs).unwrap().text.link_at(3), 0, "Done with no address removes the link");
    }

    #[test]
    fn links_need_a_selection_rich_text_and_an_editable_document() {
        let mut none = app_with_text("text", 2..2);
        assert!(none.open_link_sheet());
        assert!(none.link.is_none());
        assert!(none.note.as_deref().is_some_and(|n| n.contains("Select the text")));

        let mut plain = app_with_text("text", 0..4);
        active_doc_mut(&mut plain.docs).unwrap().rich = false;
        assert!(plain.open_link_sheet());
        assert!(plain.link.is_none());
        assert!(plain.note.as_deref().is_some_and(|n| n.contains("rich text")));

        let mut locked = app_with_text("text", 0..4);
        active_doc_mut(&mut locked.docs).unwrap().read_only = true;
        assert!(locked.open_link_sheet());
        assert!(locked.link.is_none());
    }

    #[test]
    fn smart_links_link_an_address_when_the_word_ends() {
        let mut app = app_with_text("", 0..0);
        app.opts.smart_links = true;
        assert!(app.insert_raw("see www.example.com"));
        assert_eq!(active_doc(&app.docs).unwrap().text.link_at(8), 0, "no link until the word ends");
        assert!(app.insert_raw(" "));
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(links::url(doc.text.link_at(8)).as_deref(), Some("https://www.example.com"));
        assert_eq!(doc.text.link_at(3), 0);
        assert_eq!(doc.text.link_at(19), 0, "the blank after it is not part of the link");
        assert_eq!(doc.text.style().link, 0, "typing goes on outside the link");
        // a plain word, and an address that is already a link, are left alone
        assert!(app.insert_raw("hello "));
        assert!(active_doc(&app.docs).unwrap().text.cells()[20..].iter().all(|c| c.style.link == 0));
        // with the option off nothing is linked
        let mut off = app_with_text("", 0..0);
        off.insert_raw("www.example.com ");
        assert_eq!(active_doc(&off.docs).unwrap().text.link_at(2), 0);
        // a plain text document is never linked
        let mut plain = app_with_text("", 0..0);
        plain.opts.smart_links = true;
        active_doc_mut(&mut plain.docs).unwrap().rich = false;
        plain.insert_raw("www.example.com ");
        assert_eq!(active_doc(&plain.docs).unwrap().text.link_at(2), 0);
    }

    #[test]
    fn typing_into_the_link_panel_takes_an_address_without_blanks() {
        let mut app = app_with_text("text", 0..4);
        app.open_link_sheet();
        assert!(app.type_into("https://exa mple.com"));
        assert_eq!(app.link.as_ref().unwrap().url, "https://example.com");
        assert!(app.backspace());
        assert_eq!(app.link.as_ref().unwrap().url, "https://example.co");
        assert_eq!(active_doc(&app.docs).unwrap().text.text(), "text", "the page is untouched");
    }

    #[test]
    fn only_rich_text_takes_formatting_commands() {
        assert!(is_formatting(Action::Bold));
        assert!(is_formatting(Action::Color(2)));
        assert!(is_formatting(Action::List(text::ListKind::Bullet)));
        assert!(!is_formatting(Action::MakePlain));
        assert!(!is_formatting(Action::Find));
        assert!(!is_formatting(Action::Print));
    }

    // ---- the page and the format bar, through a context with no shell behind it ----

    const SENTENCE: &str = "Hello from the BifrOSt E2E run.";

    fn window(w: u32, h: u32) -> Ctx {
        use bifrost_app::kit::{Font, Ui};
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = bifrost_app::Configure { w, h, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    /// The window point a quarter of the way into the character at `index` on the first line of the page: the caret nearest
    /// to it is the one before that character, and the character under it is that one.
    fn middle_of(app: &TextEditApp, ctx: &Ctx, text: &str, index: usize) -> (f32, f32) {
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let width = |ch: char| view::char_width(&ctx.ui, ch, &Style::default());
        let before: f32 = text.chars().take(index).map(width).sum();
        let this = text.chars().nth(index).map_or(0.0, width);
        (geom.text_x + before + this * 0.25, geom.body_top + 6.0)
    }

    fn press(app: &mut TextEditApp, ctx: &Ctx, kind: TouchKind, (x, y): (f32, f32), t: f64) -> bool {
        app.touch(ctx, kind, x * ctx.ui.s, y * ctx.ui.s, t)
    }

    fn selected(app: &TextEditApp) -> String {
        active_doc(&app.docs).unwrap().text.selected_text().unwrap_or_default()
    }

    fn wheel_at(app: &mut TextEditApp, ctx: &Ctx, (x, y): (f32, f32), dy: f32) -> bool {
        use bifrost_app::pointer_events::{Phase, PointerEvent};
        let ev = PointerEvent::Wheel { x: x * ctx.ui.s, y: y * ctx.ui.s, dx: 0.0, dy: dy * ctx.ui.s, precise: false, phase: Phase::None, momentum: Phase::None };
        app.pointer(ctx, &ev)
    }

    /// E2E: the wheel scrolls the page (the emulated drag it replaces placed the caret and selected text), stops at the
    /// ends of the text, and over the Fonts panel it goes to the family column, never to the page under the panel.
    #[test]
    fn the_wheel_scrolls_the_page_and_over_the_fonts_panel_its_families() {
        let ctx = window(1200, 780);
        assert_eq!(TextEditApp::new().pointer_interest(), bifrost_app::pointer_events::WHEEL);
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_with_text(&long, 0..0);
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        app.ensure_layout(&ctx.ui, geom.text_w);
        let page = (geom.text_x + 10.0, geom.body_top + 40.0);
        assert!(wheel_at(&mut app, &ctx, page, -40.0), "a wheel turned down shows the text below");
        assert_eq!(app.scroll, 40.0);
        assert_eq!(selected(&app), "", "no text is selected by the wheel");
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0, "and the caret stays");
        assert!(wheel_at(&mut app, &ctx, page, 15.0));
        assert_eq!(app.scroll, 25.0);
        wheel_at(&mut app, &ctx, page, 1.0e6);
        assert_eq!(app.scroll, 0.0, "the top of the text");
        assert!(!wheel_at(&mut app, &ctx, page, 30.0), "already at the top");
        wheel_at(&mut app, &ctx, page, -1.0e6);
        let bottom = app.scroll;
        assert!(bottom > 0.0 && bottom < app.cache.as_ref().unwrap().layout.height, "the end of the text: {bottom}");
        assert!(!wheel_at(&mut app, &ctx, (page.0, geom.body_top - 4.0), 30.0), "the format bar does not scroll");

        // the Fonts panel: its family column takes the wheel (three families fit, so nothing moves) and the page under
        // the panel keeps its place
        app.fonts = Some(font_panel::Fonts::new(0));
        let layout = font_panel::fonts_layout(geom.width, geom.body_top);
        let column = (layout.family.x + 4.0, layout.family.y + 4.0);
        assert!(!wheel_at(&mut app, &ctx, column, 30.0));
        assert!(!wheel_at(&mut app, &ctx, (layout.done.x + 1.0, layout.done.y + 1.0), 30.0));
        assert_eq!(app.scroll, bottom);
        // beside the panel the page still scrolls
        let beside = (layout.frame.x + layout.frame.w + 4.0, geom.body_bottom - 10.0);
        assert!(!layout.frame.contains(beside.0, beside.1));
        assert!(wheel_at(&mut app, &ctx, beside, 30.0));
        assert_eq!(app.scroll, bottom - 30.0);
    }

    #[test]
    fn a_press_on_the_first_line_places_the_caret_though_the_find_fields_overlap_it() {
        // the report: clicks never moved the caret, because the closed find bar's fields sat over the first line and took the press
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let point = middle_of(&app, &ctx, SENTENCE, 3);
        assert!(geom.find_field.contains(point.0, point.1), "the point is under the hidden find field: {point:?} {:?}", geom.find_field);
        assert!(press(&mut app, &ctx, TouchKind::Down, point, 0.0));
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(doc.text.head(), 3, "the caret is at the character pressed, not at the end");
        assert_eq!(app.focus, Focus::Body);
        // typing goes in at the caret
        press(&mut app, &ctx, TouchKind::Up, point, 10.0);
        app.type_into("X");
        let text = active_doc(&app.docs).unwrap().text.text();
        assert_eq!(text, "HelXlo from the BifrOSt E2E run.");
    }

    #[test]
    fn a_double_click_selects_the_word_and_typing_replaces_it() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
        let point = middle_of(&app, &ctx, SENTENCE, 7);
        press(&mut app, &ctx, TouchKind::Down, point, 0.0);
        press(&mut app, &ctx, TouchKind::Up, point, 40.0);
        assert_eq!(selected(&app), "");
        press(&mut app, &ctx, TouchKind::Down, point, 120.0);
        press(&mut app, &ctx, TouchKind::Up, point, 160.0);
        assert_eq!(selected(&app), "from");
        app.type_into("X");
        assert_eq!(active_doc(&app.docs).unwrap().text.text(), "Hello X the BifrOSt E2E run.");
    }

    #[test]
    fn a_triple_click_selects_the_paragraph_and_a_fourth_press_starts_over() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 0..0);
        let point = middle_of(&app, &ctx, SENTENCE, 7);
        for (k, t) in [0.0, 100.0, 200.0].into_iter().enumerate() {
            press(&mut app, &ctx, TouchKind::Down, point, t);
            press(&mut app, &ctx, TouchKind::Up, point, t + 30.0);
            let expected = [0, 4, SENTENCE.len()][k];
            assert_eq!(selected(&app).len(), expected, "press {}", k + 1);
        }
        press(&mut app, &ctx, TouchKind::Down, point, 300.0);
        assert_eq!(selected(&app), "", "the fourth press places the caret again");
    }

    #[test]
    fn dragging_selects_by_characters_and_by_words_after_a_double_click() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 0..0);
        // a plain drag from the middle of "from" to the middle of "BifrOSt"
        let (from, to) = (middle_of(&app, &ctx, SENTENCE, 7), middle_of(&app, &ctx, SENTENCE, 16));
        press(&mut app, &ctx, TouchKind::Down, from, 0.0);
        assert!(press(&mut app, &ctx, TouchKind::Move, to, 20.0));
        assert_eq!(selected(&app), "rom the B", "from the character pressed to the one under the pointer");
        press(&mut app, &ctx, TouchKind::Up, to, 30.0);
        assert!(!press(&mut app, &ctx, TouchKind::Move, from, 40.0), "a move with the button up does nothing");
        // press twice and hold: the drag takes whole words
        press(&mut app, &ctx, TouchKind::Down, from, 2000.0);
        press(&mut app, &ctx, TouchKind::Up, from, 2010.0);
        press(&mut app, &ctx, TouchKind::Down, from, 2100.0);
        press(&mut app, &ctx, TouchKind::Move, to, 2120.0);
        assert_eq!(selected(&app), "from the BifrOSt");
    }

    #[test]
    fn the_navigation_chords_extend_the_selection_and_plain_keys_collapse_it() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
        // Shift+Home (evdev 102) selects back to the start of the line, Shift+End forward again
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(102, CHORD_SHIFT))));
        assert_eq!(selected(&app), SENTENCE);
        // Shift+Right moves the head from the start; the anchor stays at the end of the text
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(106, CHORD_SHIFT))));
        assert_eq!(selected(&app), &SENTENCE[1..]);
        // Alt+Shift+Right moves the head one word on
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(106, CHORD_SHIFT | CHORD_ALT))));
        assert_eq!(selected(&app), &SENTENCE[5..]);
        // Left alone collapses to the start of the selection, then Home (plain code 36) goes to the line start
        assert!(app.event(&ctx, Input::Key(KeyInput::Code(37))));
        assert_eq!(active_doc(&app.docs).unwrap().text.selection(), 5..5);
        assert!(app.event(&ctx, Input::Key(KeyInput::Code(36))));
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0);
        assert!(app.event(&ctx, Input::Key(KeyInput::Code(35))));
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), SENTENCE.len());
        // Cmd+Left is the line start as well, and a chord on another key is not a move
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(105, CHORD_META))));
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0);
        assert!(!app.event(&ctx, Input::Key(KeyInput::chord(49, CHORD_META))), "a chord on another key is not a move");
        // Cmd+A as a chord (the shell sends it so when its last report of the commands is stale) selects everything
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(30, CHORD_META))));
        assert_eq!(active_doc(&app.docs).unwrap().text.selection(), 0..SENTENCE.len());
    }

    #[test]
    fn the_navigation_keys_leave_a_panel_or_the_find_field_alone() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 3..3);
        app.focus = Focus::Find;
        assert!(!app.event(&ctx, Input::Key(KeyInput::chord(102, CHORD_SHIFT))), "the find field has the keyboard");
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), 3);
        app.focus = Focus::Body;
        app.open_line_sheet();
        assert!(!app.event(&ctx, Input::Key(KeyInput::chord(107, CHORD_SHIFT))), "a panel is open over the page");
        assert_eq!(active_doc(&app.docs).unwrap().text.head(), 3);
    }

    #[test]
    fn the_editing_commands_act_on_the_focused_find_field_or_sheet_not_the_document() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 0..5);
        app.find_open = true;
        app.focus = Focus::Find;
        clipboard::set_text("needle\nsecond");
        assert!(app.event(&ctx, Input::Command(Command::Paste)));
        assert_eq!(app.find, "needle", "paste goes into the find field");
        assert_eq!(text_of(&app), SENTENCE, "the document is untouched");
        // Select All, Copy and Cut are the field's (QA W2-12: Cmd+A appended to the query instead); Undo does nothing
        assert!(app.event(&ctx, Input::Command(Command::SelectAll)), "Cmd+A selects the find field's text");
        assert!(app.find_sel);
        assert!(!app.event(&ctx, Input::Command(Command::Undo)));
        clipboard::set_text("");
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(45, CHORD_META))), "Cmd+X as a chord cuts the field's text");
        assert_eq!((app.find.as_str(), clipboard::text().as_deref()), ("", Some("needle")));
        assert_eq!(text_of(&app), SENTENCE, "cut and undo left the document alone");
        assert_eq!(caret_of(&app), 0..5, "select all left the selection alone");
        // a sheet open over the page: the same
        app.find_open = false;
        app.focus = Focus::Body;
        app.open_line_sheet();
        for command in [Command::Cut, Command::SelectAll, Command::Undo] {
            assert!(!app.event(&ctx, Input::Command(command)));
        }
        assert_eq!(text_of(&app), SENTENCE);
        assert_eq!(caret_of(&app), 0..5);
    }

    /// QA W2-12: Cmd+A in the find field, live matches while typing, the previous query selected on reopening, and no
    /// "not found" left in the status row after the bar closes.
    #[test]
    fn the_find_bar_searches_as_it_is_typed_and_reopens_with_the_query_selected() {
        let ctx = window(1200, 780);
        let mut app = app_with_text("the lazy dog and the lazy cat", 0..0);
        assert!(app.perform(&ctx, Action::Find));
        // the shared find pasteboard may hold another test's query
        app.find.clear();
        app.find_sel = false;
        for c in ["l", "a", "z"] {
            app.event(&ctx, Input::Key(KeyInput::Text(c.into())));
        }
        assert_eq!(caret_of(&app), 4..7, "the first match is selected while typing");
        assert_eq!(app.find_matches(), vec![4..7, 21..24], "every match is highlighted");
        app.event(&ctx, Input::Key(KeyInput::Text("q".into())));
        assert_eq!(app.note.as_deref(), Some("\"lazq\" not found"));
        assert!(app.find_matches().is_empty());
        app.backspace();
        assert_eq!((app.find.as_str(), app.note.as_deref()), ("laz", None));
        // closing clears the note and the highlight
        app.event(&ctx, Input::Key(KeyInput::Text("q".into())));
        assert!(app.perform(&ctx, Action::FindDone));
        assert_eq!(app.note, None);
        assert!(app.find_matches().is_empty(), "no highlight once the bar is closed");
        // reopening selects the previous query: typing replaces it, Cmd+A selects it again
        app.perform(&ctx, Action::Find);
        assert!(app.find_sel);
        app.event(&ctx, Input::Key(KeyInput::Text("cat".into())));
        assert_eq!(app.find, "cat");
        assert_eq!(caret_of(&app), 26..29);
        app.event(&ctx, Input::Command(Command::SelectAll));
        app.event(&ctx, Input::Key(KeyInput::Text("dog".into())));
        assert_eq!(app.find, "dog", "Cmd+A then typing replaces the query");
        app.event(&ctx, Input::Command(Command::SelectAll));
        app.backspace();
        assert_eq!(app.find, "", "Backspace over the selected query clears it");
        assert_eq!(text_of(&app), "the lazy dog and the lazy cat", "the page was never edited");
    }

    #[test]
    fn find_all_lists_every_match_without_overlap() {
        let app = app_with_text("aaaa Aa", 0..0);
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(doc.text.find_all("aa", false, 10), vec![0..2, 2..4, 5..7]);
        assert_eq!(doc.text.find_all("aa", true, 10), vec![0..2, 2..4]);
        assert_eq!(doc.text.find_all("aa", false, 1), vec![0..2], "the limit holds");
        assert!(doc.text.find_all("", false, 10).is_empty());
    }

    /// QA W2-11: Cmd+T did nothing and the Format menu had no Show Fonts or Show Colors.
    #[test]
    fn show_fonts_and_show_colors_open_panels_that_format_the_selection() {
        let ctx = window(1200, 780);
        let mut app = app_with_text("Hello world", 0..5);
        let format = app.format_menu();
        let item = |title: &str| format.items.iter().find(|i| i.title == title).cloned();
        assert_eq!(item("Show Fonts").map(|i| (i.id, i.key)), Some((Action::ShowFonts.id().unwrap(), "cmd+t".to_string())));
        assert_eq!(item("Show Colors").map(|i| (i.id, i.key)), Some((Action::ShowColors.id().unwrap(), "cmd+shift+c".to_string())));
        // the menu item (Cmd+T) opens the panel; a family, a typeface and a size apply to the selection at once
        assert!(app.menu_action(&ctx, Action::ShowFonts.id().unwrap()), "Cmd+T is the menu item's key equivalent");
        assert!(app.fonts.is_some() && !app.page_has_keys());
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let l = font_panel::fonts_layout(geom.width, geom.body_top);
        let row = |r: bifrost_app::controls::Rect, i: usize| (r.x + 6.0, r.y + font_panel::ROW_H * (i as f32 + 0.5));
        press(&mut app, &ctx, TouchKind::Down, row(l.family, 2), 0.0);
        press(&mut app, &ctx, TouchKind::Down, row(l.face, 1), 100.0);
        press(&mut app, &ctx, TouchKind::Down, row(l.size, 5), 200.0);
        let style = active_doc(&app.docs).unwrap().text.cells()[0].style;
        assert_eq!((style.font, style.bold, style.italic, style.size), (2, true, false, text::SIZES[5]));
        assert_eq!(active_doc(&app.docs).unwrap().text.cells()[6].style.font, 0, "only the selection");
        // Up steps to the previous family; typing a name jumps to it; Esc closes
        app.event(&ctx, Input::Key(KeyInput::Code(38)));
        assert_eq!(app.status().style.font, 1);
        app.event(&ctx, Input::Key(KeyInput::Text("hel".into())));
        assert_eq!(app.status().style.font, 0, "Helvetica");
        app.event(&ctx, Input::Key(KeyInput::Code(27)));
        assert!(app.fonts.is_none() && app.page_has_keys());
        // Show Colors: a swatch colours the selection, Done closes
        app.perform(&ctx, Action::ShowColors);
        let c = font_panel::colors_layout(geom.width, geom.body_top);
        let s = c.swatches[1];
        press(&mut app, &ctx, TouchKind::Down, (s.x + 4.0, s.y + 4.0), 300.0);
        assert_eq!(active_doc(&app.docs).unwrap().text.cells()[0].style.color, 1);
        press(&mut app, &ctx, TouchKind::Down, (c.done.x + 2.0, c.done.y + 2.0), 400.0);
        assert!(!app.colors_panel);
        assert!(app.docs.is_edited(app.docs.active().unwrap()), "formatting is an unsaved change");
    }

    #[test]
    fn an_installed_family_beyond_the_base_three_maps_to_a_base_face_and_keeps_its_name() {
        // the tests see only the base families; the mapping rules still hold for any index
        assert_eq!(text::base_family(1), 1);
        assert_eq!(text::base_family(200), 0, "an index past the list falls back to the default");
        assert_eq!(text::export_name(2), "Courier New");
        assert_eq!(text::family_index("times"), Some(1));
        assert_eq!(text::family_for_name("Liberation Mono"), 2, "a fixed-width name maps to Courier");
    }

    fn text_of(app: &TextEditApp) -> String {
        active_doc(&app.docs).unwrap().text.text()
    }

    fn caret_of(app: &TextEditApp) -> Range<usize> {
        active_doc(&app.docs).unwrap().text.selection()
    }

    /// The report: Cmd+A, C, X, V did nothing and Undo had nothing to undo. The shell sends them as the standard commands
    /// (`Input::Command`); each one acts on the page, and each edit is one undo step.
    #[test]
    fn select_all_copy_cut_paste_and_undo_act_on_the_page() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 5..5);
        assert!(app.event(&ctx, Input::Command(Command::SelectAll)));
        assert_eq!(caret_of(&app), 0..SENTENCE.len(), "Cmd+A selects everything");
        app.event(&ctx, Input::Command(Command::Copy));
        assert_eq!(clipboard::text().as_deref(), Some(SENTENCE), "Cmd+C reaches the clipboard");
        assert_eq!(text_of(&app), SENTENCE, "copy leaves the text");
        assert!(app.event(&ctx, Input::Command(Command::Cut)));
        assert_eq!(text_of(&app), "", "Cmd+X removes the selection");
        assert_eq!(clipboard::text().as_deref(), Some(SENTENCE));
        assert!(app.event(&ctx, Input::Command(Command::Undo)));
        assert_eq!(text_of(&app), SENTENCE, "Cmd+Z puts the cut back");
        assert!(app.event(&ctx, Input::Command(Command::Redo)));
        assert_eq!(text_of(&app), "", "Shift+Cmd+Z cuts again");
        assert!(app.event(&ctx, Input::Command(Command::Paste)));
        assert_eq!(text_of(&app), SENTENCE, "Cmd+V pastes the clipboard");
        assert!(app.event(&ctx, Input::Command(Command::Undo)));
        assert_eq!(text_of(&app), "", "the paste is an undo step of its own");
        // the app reports what it takes, so the shell sends the command (and not the chord)
        let commands = app.commands(&ctx);
        assert!(commands.supports(Command::SelectAll) && commands.supports(Command::Paste) && commands.supports(Command::Redo));
    }

    /// Option+Backspace deletes a word, Cmd+Backspace the line before the caret, Option+Delete a word forward, Fn+Delete a
    /// character forward, and Ctrl+K the rest of the paragraph; each is one chord and one undo step.
    #[test]
    fn the_delete_chords_remove_a_word_or_a_line() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 22..22);
        // Option+Backspace (evdev 14) after "BifrOSt" removes the word
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_ALT))));
        assert_eq!(text_of(&app), "Hello from the  E2E run.");
        assert_eq!(caret_of(&app), 15..15);
        // Cmd+Backspace removes everything back to the start of the line
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_META))));
        assert_eq!(text_of(&app), " E2E run.");
        assert_eq!(caret_of(&app), 0..0);
        // Option+Delete (evdev 111) removes the next word, blanks first; plain Delete one character
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(111, CHORD_ALT))));
        assert_eq!(text_of(&app), " run.");
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(111, 0))));
        assert_eq!(text_of(&app), "run.");
        // Ctrl+E is the line end, Ctrl+H a backspace, Ctrl+A the line start, Ctrl+D a forward delete, Ctrl+K the rest
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(18, CHORD_CTRL))));
        assert_eq!(caret_of(&app), 4..4);
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(35, CHORD_CTRL))));
        assert_eq!(text_of(&app), "run");
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(30, CHORD_CTRL))));
        assert_eq!(caret_of(&app), 0..0);
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(32, CHORD_CTRL))));
        assert_eq!(text_of(&app), "un");
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(37, CHORD_CTRL))));
        assert_eq!(text_of(&app), "");
        // with a selection each of them deletes the selection
        let mut app = app_with_text(SENTENCE, 6..10);
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_ALT))));
        assert_eq!(text_of(&app), "Hello  the BifrOSt E2E run.");
        // the undo stack has the steps
        assert!(app.event(&ctx, Input::Command(Command::Undo)));
        assert_eq!(text_of(&app), SENTENCE);
    }

    #[test]
    fn page_up_and_page_down_move_a_window_of_lines_and_extend_with_shift() {
        let ctx = window(1200, 780);
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_with_text(&long, 0..0);
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(109, 0))), "Page Down");
        let down = caret_of(&app).start;
        assert!(down > 0 && down < long.len(), "the caret moved down by a page, not to the end: {down}");
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(109, CHORD_SHIFT))), "Shift+Page Down");
        let sel = caret_of(&app);
        assert_eq!(sel.start, down, "the selection grows from where Page Down left the caret");
        assert!(sel.end > sel.start);
        assert!(app.event(&ctx, Input::Key(KeyInput::chord(104, 0))), "Page Up");
        assert_eq!(caret_of(&app).start, caret_of(&app).end, "a plain Page Up collapses the selection");
    }
    fn centre(r: bifrost_app::controls::Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn the_format_bar_buttons_format_the_selection_and_show_its_state() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 6..10);
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
        let bold = centre(placed.rect_of(format_bar::BOLD).unwrap());
        assert!(press(&mut app, &ctx, TouchKind::Down, bold, 0.0));
        let doc = active_doc(&app.docs).unwrap();
        assert_eq!(doc.text.selection(), 6..10, "the press on a button keeps the selection");
        assert!(doc.text.cells()[6..10].iter().all(|c| c.style.bold));
        assert!(!doc.text.cells()[0].style.bold && !doc.text.cells()[10].style.bold);
        assert!(app.status().style.bold, "the bar shows the style of the selection");
        // without a selection the button sets the typing style
        active_doc_mut(&mut app.docs).unwrap().text.select(31, 31);
        let italic = centre(placed.rect_of(format_bar::ITALIC).unwrap());
        press(&mut app, &ctx, TouchKind::Down, italic, 100.0);
        app.type_into("z");
        let typed = active_doc(&app.docs).unwrap().text.cells().last().copied().unwrap();
        assert!(typed.ch == 'z' && typed.style.italic && !typed.style.bold, "{typed:?}");
    }

    #[test]
    fn a_pop_up_opens_its_menu_and_the_choice_formats_the_selection() {
        use bifrost_app::proto::ToShell;
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 0..5);
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
        let size = centre(placed.rect_of(format_bar::SIZE).unwrap());
        assert!(!press(&mut app, &ctx, TouchKind::Down, size, 0.0));
        let sent = ctx.take_sent();
        let items = sent.iter().find_map(|m| match m {
            ToShell::ContextMenu { items, .. } => Some(items.clone()),
            _ => None,
        });
        let items = items.expect("the size pop-up opens a menu");
        assert_eq!(items.len(), text::SIZES.len());
        // the choice comes back with the menu id of its action
        let eighteen = items.iter().find(|i| i.label == "18").expect("a row for 18").id;
        assert!(app.event(&ctx, Input::ContextAction(eighteen)));
        assert!(active_doc(&app.docs).unwrap().text.cells()[..5].iter().all(|c| c.style.size == 18.0));
        assert_eq!(app.status().style.size, 18.0);
        // the style pop-up sets bold and italic together
        assert!(app.event(&ctx, Input::ContextAction(Action::FontStyle(3).id().unwrap())));
        let first = active_doc(&app.docs).unwrap().text.cells()[0].style;
        assert!(first.bold && first.italic);
        assert!(app.event(&ctx, Input::ContextAction(Action::FontStyle(0).id().unwrap())));
        let first = active_doc(&app.docs).unwrap().text.cells()[0].style;
        assert!(!first.bold && !first.italic);
    }

    #[test]
    fn the_overflow_button_of_a_narrow_window_opens_the_hidden_controls() {
        use bifrost_app::proto::ToShell;
        let ctx = window(520, 600);
        let mut app = app_with_text(SENTENCE, 0..0);
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
        let more = placed.overflow_button.expect("the bar overflows at 520 points");
        let nodes = app.controls(&ctx);
        assert!(nodes.iter().any(|n| n.label == "More Formatting" && n.bounds == more), "the button is a control");
        assert!(nodes.iter().all(|n| n.bounds.right() <= geom.width + 0.01), "no control is cut by the window's edge");
        press(&mut app, &ctx, TouchKind::Down, centre(more), 0.0);
        let menu = ctx.take_sent().into_iter().find_map(|m| match m {
            ToShell::ContextMenu { items, .. } => Some(items),
            _ => None,
        });
        assert!(menu.is_some_and(|items| !items.is_empty() && items.len() <= 32));
        // the same through the control: assistive technology activates it
        assert!(!app.event(&ctx, Input::Activate(format_bar::NODE_BASE + u64::from(format_bar::OVERFLOW))));
        assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::ContextMenu { .. })));
    }

    #[test]
    fn the_controls_the_window_publishes_are_the_format_bar() {
        let ctx = window(1200, 780);
        let app = app_with_text(SENTENCE, 0..0);
        let nodes = app.controls(&ctx);
        assert_eq!(nodes.len(), format_bar::ALL.len());
        assert!(nodes.iter().all(|n| !n.label.is_empty()));
        // none of the old text buttons (New, Open, Save, Undo, Redo, Find, Print) is a control any more
        for gone in ["New", "Open", "Save", "Undo", "Redo", "Find", "Print", "Bullets", "Numbers"] {
            assert!(!nodes.iter().any(|n| n.label == gone), "{gone}");
        }
    }

    #[test]
    fn the_title_row_names_the_document_and_says_when_it_has_unsaved_changes() {
        let mut app = app_with_text("", 0..0);
        assert_eq!(app.window_title(), "Untitled");
        app.type_into("x");
        assert_eq!(app.window_title(), "Untitled \u{2013} Edited");
        assert_eq!(TextEditApp::new().window_title(), "Text Editor", "with no document the app's own name shows");
    }

    #[test]
    fn a_plain_text_document_takes_no_press_on_the_format_bar() {
        let ctx = window(1200, 780);
        let mut app = app_with_text(SENTENCE, 0..5);
        active_doc_mut(&mut app.docs).unwrap().rich = false;
        let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
        let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
        let bold = centre(placed.rect_of(format_bar::BOLD).unwrap());
        assert!(!press(&mut app, &ctx, TouchKind::Down, bold, 0.0));
        assert!(!active_doc(&app.docs).unwrap().text.cells()[0].style.bold);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn the_size_and_style_actions_count_as_formatting() {
        assert!(is_formatting(Action::Size(3)));
        assert!(is_formatting(Action::FontStyle(1)));
    }
}
