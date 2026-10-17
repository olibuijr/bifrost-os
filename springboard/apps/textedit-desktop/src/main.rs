//! Text Editor as an installed BifrOSt app: plain and rich text documents, opened and saved as RTF, HTML or plain text
//! through the shell's document panels, with character and paragraph formatting, fonts, find and replace, spelling, a
//! tab ruler, printing with page setup, a word count, and undo and redo.
//!
//! The document model is in `text.rs`, the file formats in `rtf.rs`, `html.rs` and `document.rs`, the line breaking in
//! `layout.rs`, the spelling in `spell.rs`, the PDF for printing in `print.rs`, and the window in `view.rs`. This file
//! wires them to the SDK: the document controller, menus, keys, touches and the canvas.

mod docx;
mod encoding;
mod document;
mod html;
mod layout;
mod links;
mod odt;
mod prefs;
mod print;
mod properties;
mod replace;
mod rtf;
mod rtfd;
mod spell;
mod text;
mod view;
mod webarchive;
mod xml;
mod zipdoc;

use bifrost_app::controls::undo::ChangeCount;
use bifrost_app::controls::{Node, Role};
use bifrost_app::document::{is_menu_id, item, DocId, DocumentController, DocumentHost, Notice};
use bifrost_app::kit::canvas::Canvas;
use bifrost_app::pasteboard::NativePasteboard;
use bifrost_app::kit::ui::Ui;
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::print::{Orientation, PrintError, PrintInfo, PrintOperation};
use bifrost_app::proto::CHORD_FLAG;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use document::TextDocument;
use encoding::Encoding;
use layout::Layout;
use prefs::Options;
use spell::Spelling;
use std::ops::Range;
use text::{Align, Cell, Kind, Style, Transform};
use view::{Action, Button, Focus, Hit};

/// The layout of the document as last drawn, and the misspelled words of its text, kept until the text, the width or
/// the document changes.
struct Cache {
    doc: DocId,
    revision: u64,
    width: f32,
    zoom: f32,
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
    /// Cut and Copy keep the copied cells with their formatting, for a paste of the same text. The text is the clipboard
    /// text they were copied with.
    rich_clip: Option<(String, Vec<Cell>)>,
    /// Format > Text > Copy Ruler: the tab stop distance copied, for Paste Ruler
    ruler: Option<f32>,
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
            note: None,
            scroll: 0.0,
            follow: true,
            drag: false,
            revision: 0,
            cache: None,
            spelling: Spelling::new(),
            print_info: PrintInfo::default(),
            opts: Options::default(),
            zoom: 1.0,
            copied_style: None,
            completion: None,
            export_request: None,
            props: None,
            link: None,
            line: None,
            spelling_panel: false,
            settings: false,
            encodings_panel: false,
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

    /// The toolbar's status: what the document has at the caret.
    fn status(&self) -> view::Status {
        match active_doc(&self.docs) {
            Some(doc) => view::Status {
                style: doc.text.style(),
                align: doc.text.align_at(doc.text.head()),
                rich: doc.rich,
                can_undo: doc.text.can_undo(),
                can_redo: doc.text.can_redo(),
                find_open: self.find_open,
            },
            None => view::Status {
                style: Style::default(),
                align: Align::Left,
                rich: false,
                can_undo: false,
                can_redo: false,
                find_open: self.find_open,
            },
        }
    }

    /// The toolbar and format bar buttons, and, with the find bar open, its buttons.
    fn buttons(&self, ui: &Ui, geom: &view::Geometry) -> (Vec<Button>, Vec<Button>) {
        let status = self.status();
        let mut bar = view::toolbar(ui, &status);
        bar.extend(view::format_bar(ui, &status, geom));
        let find = if self.find_open { view::find_buttons(ui, geom) } else { Vec::new() };
        (bar, find)
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
        let measure = |ch: char, style: &Style| view::char_width(ui, ch, style);
        let cells = doc.text.cells();
        let scaled = zoomed(cells, zoom);
        let layout = layout::layout_tabs(&scaled, width, &measure, doc.text.tab_width() * zoom);
        let misspelled = if self.spelling.enabled { self.spelling.misspelled(cells) } else { Vec::new() };
        self.cache = Some(Cache { doc: id, revision: self.revision, width, zoom, cells: scaled, layout, misspelled });
    }

    /// Runs a text command on the active document. Returns whether the window needs a frame.
    fn perform(&mut self, ctx: &Ctx, action: Action) -> bool {
        // a panel takes a paste as typing (the first line of the clipboard)
        let panel_open = self.link.is_some() || self.props.is_some() || self.line.is_some() || self.spacing_sheet.is_some();
        if matches!(action, Action::Paste | Action::PasteMatch) && panel_open {
            let line = clipboard::text().map(|s| s.lines().next().unwrap_or("").to_string()).unwrap_or_default();
            return self.type_into(&line);
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
                let cut = doc.text.cut();
                if cut.is_empty() {
                    false
                } else {
                    clipboard::set_text(&cut);
                    self.rich_clip = Some((cut, picked));
                    true
                }
            }
            Action::Copy => {
                let picked = doc.text.cells()[doc.text.selection()].to_vec();
                let copied: String = picked.iter().map(|c| c.ch).collect();
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
                // attachments are not dropped silently: the user deletes them first (the confirmation is still to come)
                if doc.rich && doc.text.has_attachments() {
                    self.note = Some("This document has attachments. Delete them before choosing Make Plain Text.".into());
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
        let measure = |ch: char, style: &Style| view::char_width(&ctx.ui, ch, style);
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
            let field = if self.focus == Focus::Find { &mut self.find } else { &mut self.replace };
            return field.pop().is_some();
        }
        if self.refuse_edit() {
            return true;
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
                cache.layout.move_vertical(&cache.cells, head, code == 40, &measure)
            }
            _ => return false,
        };
        doc.text.select(target, target);
        self.follow = true;
        true
    }

    /// Mouse or finger: a press places the caret (or presses a control, or sets a tab stop on the ruler), a drag
    /// extends the selection.
    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let (px, py) = (x / ctx.ui.s, y / ctx.ui.s);
        match kind {
            TouchKind::Down => {
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
                if self.line.is_some() {
                    return self.touch_line(ctx, px, py);
                }
                if self.link.is_some() {
                    return self.touch_link(ctx, px, py);
                }
                let geom = self.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
                let (bar, find) = self.buttons(&ctx.ui, &geom);
                let all: Vec<Button> = bar.into_iter().chain(find).collect();
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
                self.focus = Focus::Body;
                let width = geom.text_w;
                self.ensure_layout(&ctx.ui, width);
                let measure = |ch: char, style: &Style| view::char_width(&ctx.ui, ch, style);
                let at = px - geom.text_x;
                let scroll_y = py - geom.body_top + self.scroll;
                let Some(doc) = active_doc_mut(&mut self.docs) else { return false };
                let Some(cache) = self.cache.as_ref() else { return false };
                let index = cache.layout.index_at(&cache.cells, at, scroll_y, &measure);
                // in a document that cannot be edited a press on a link follows it
                if doc.read_only {
                    if let Some(url) = links::url(doc.text.link_near(index)) {
                        ctx.open_url(&url);
                        self.note = Some(format!("Opening {url}"));
                        return true;
                    }
                }
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
                let index = cache.layout.index_at(&cache.cells, at, scroll_y, &measure);
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
        for index in 0..text::FAMILIES.len() as u8 {
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

impl App for TextEditApp {
    fn id(&self) -> &str {
        "is.olibuijr.textedit.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the learned words and the settings are read once the app has its preferences
        self.spelling.load(ctx);
        self.opts = Options::read(&|key| DocumentHost::pref(ctx, key));
        self.spelling.enabled = self.opts.spell_enabled;
        document::set_markup_as_code(self.opts.markup_as_code);
        // a window starts with an untitled document, as a new document would
        if self.docs.is_empty() {
            let id = self.docs.new_document();
            self.apply_new_defaults(id);
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        view::toolbar(&ctx.ui, &self.status())
            .into_iter()
            .enumerate()
            .map(|(index, button)| Node {
                id: 0x5445_0000 + index as u64,
                role: Role::Button,
                label: button.label,
                value: if button.active { "selected".into() } else { String::new() },
                bounds: button.rect,
                enabled: button.enabled,
            })
            .collect()
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
                // the properties panel takes the keys while it is open
                if self.props.is_some() {
                    return match code {
                        8 => self.backspace(),
                        13 => self.perform(ctx, Action::PropsDone),
                        27 => self.perform(ctx, Action::PropsCancel),
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
                        27 => self.perform(ctx, Action::SpacingCancel),
                        _ => false,
                    };
                }
                if self.settings && code == 27 {
                    return self.perform(ctx, Action::SettingsDone);
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
                        27 => self.perform(ctx, Action::LineCancel),
                        _ => false,
                    };
                }
                if self.link.is_some() {
                    return match code {
                        8 => self.backspace(),
                        13 => self.perform(ctx, Action::LinkDone),
                        27 => self.perform(ctx, Action::LinkCancel),
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

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let ev = match self.export_event(ctx, ev) {
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
        let (bar, find) = self.buttons(ui, &geom);
        view::draw_bar(canvas, ui, &geom);
        for b in &bar {
            view::draw_button(canvas, ui, b);
        }
        let tab = active_doc(&self.docs).map_or(0.0, |d| d.text.tab_width());
        view::draw_ruler(canvas, ui, &geom, tab);
        if self.find_open {
            view::draw_find(canvas, ui, &geom, &find, &self.find, &self.replace, self.focus);
        }
        self.ensure_layout(ui, geom.text_w);
        view::page_clip(canvas, ui, &geom, self.opts.dark_background);
        let view_h = geom.body_bottom - geom.body_top;
        if let (Some(doc), Some(cache)) = (active_doc(&self.docs), self.cache.as_ref()) {
            let sel = doc.text.selection();
            let head = doc.text.head();
            if self.follow {
                let (_, top, height) = cache.layout.caret(&cache.cells, head, &|ch: char, st: &Style| view::char_width(ui, ch, st));
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
                sel,
                head,
                self.scroll,
                self.focus == Focus::Body,
                &cache.misspelled,
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
}
