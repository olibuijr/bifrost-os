//! What Text Editor draws and where it can be touched: the unified toolbar, the find bar, the page with its text, the
//! caret and selection, and the word count. Positions are in points derived from the window size; the canvas is
//! reached in device pixels through `Ui::pt`. Colours and type come from the shared tokens only.

use crate::layout::{self, Layout, Line, Marker};
use crate::properties::{ListLayout, PanelLayout};
use crate::text::{Align, Attachment, Cell, ListKind, Style, Transform, COLORS, FAMILIES, OBJECT, SPACINGS};
use bifrost_app::graphics_imageio;
use bifrost_app::controls::Rect;
use bifrost_app::document::item;
use bifrost_app::kit::canvas::{Canvas, ImageRef, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, CONTENT_BG, DESTRUCTIVE, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT,
    SELECTION, SEGMENT_ON, SEPARATOR_C, WHITE,
};
use bifrost_app::glyphs::glyph;
use bifrost_app::kit::ui::system::FIND_HIGHLIGHT;
use bifrost_app::kit::{pack, Image};
use bifrost_app::layout::{traffic_lights_trailing, MARGIN_PT, READABLE_MAX_PT, UNIFIED_BAR_PT};
use bifrost_app::controls::prefs_layout::m;

/// Height of the format bar under the title row (a toolbar capsule and a little air), and of the tab ruler under it.
pub const FORMAT_H: f32 = 44.0;
pub const RULER_H: f32 = 22.0;
/// Size of the proxy icon in front of the document's name in the title row.
const PROXY_ICON_PT: f32 = 16.0;
/// Height of the find bar, when it is open.
pub const FIND_H: f32 = 36.0;
/// Height of the word count row at the bottom.
pub const STATUS_H: f32 = 24.0;
/// Width of each find field.
pub const FIELD_W: f32 = 200.0;
const BUTTON_H: f32 = 26.0;
const BUTTON_PAD: f32 = 10.0;
const ITEM_GAP: f32 = 4.0;
const GROUP_GAP: f32 = 14.0;
const CONTROL_RADIUS: f32 = m::ICON_RADIUS;
const BAR_GAP: f32 = 8.0;

/// What a control or a menu item does. Document commands (New, Open, Save...) are the document controller's and
/// travel as `Document` with its menu id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Document(u32),
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    FindPrevious,
    Replace,
    ReplaceAll,
    FindDone,
    FocusFind,
    FocusReplace,
    Bold,
    Italic,
    Underline,
    Bigger,
    Smaller,
    Font(u8),
    Align(Align),
    Color(u8),
    Spacing(u8),
    List(ListKind),
    Print,
    /// Page setup: a paper size (index into `PAPERS`), and the orientation
    Paper(u8),
    Landscape(bool),
    MakePlain,
    MakeRich,
    SpellToggle,
    SpellNext,
    SpellLearn,
    SpellIgnore,
    SpellCorrect,
    /// File: Save All and Export as PDF
    SaveAll,
    ExportPdf,
    /// Edit: Paste and Match Style, Delete, Complete, Use Selection for Find, Jump to Selection, Select Line
    PasteMatch,
    Delete,
    Complete,
    UseSelection,
    JumpToSelection,
    SelectLine,
    /// Edit > Speech: start speaking the selection (else the whole text), and stop speaking
    SpeakStart,
    SpeakStop,
    Transform(Transform),
    /// Format > Font: strikethrough, baseline shift, highlight colour (index into `COLORS`), Copy and Paste Style
    Strike,
    Superscript,
    Subscript,
    Highlight(u8),
    CopyStyle,
    PasteStyle,
    /// Format > Prevent Editing; View: ruler, wrap to window, zoom
    PreventEditing,
    ShowRuler,
    WrapToWindow,
    ZoomIn,
    ZoomOut,
    ActualSize,
    /// Substitutions and automatic spelling
    SmartQuotes,
    SmartDashes,
    SmartLinks,
    SpellAuto,
    /// Print: page header and footer
    HeaderFooter,
    /// Preferences: whether new documents are plain text (true) or rich text (false)
    NewPlain(bool),
    /// Edit > Insert: a page break, a line break (a new line in the paragraph) and a paragraph break
    InsertPage,
    InsertLine,
    InsertParagraph,
    /// Insert > Image...: an open panel for a PNG or JPEG file, which becomes an attachment
    InsertImage,
    /// The Make Plain Text confirmation sheet (when the document has attachments): remove them, or cancel
    PlainDone,
    PlainCancel,
    /// Edit > Add Link, Open Link, Remove Link, and the link panel's buttons
    AddLink,
    OpenLink,
    RemoveLink,
    LinkDone,
    LinkCancel,
    LinkRemove,
    /// File > Show Properties, and the panel's two buttons
    ShowProperties,
    PropsDone,
    PropsCancel,
    /// Preferences > Open and Save: show RTF and HTML files as code
    MarkupAsCode,
    /// Format > Text Encoding: save as (index into `encoding::ALL`), and read the text again as another encoding
    Encoding(u8),
    Reinterpret(u8),
    /// Preferences > Open and Save: the encoding of new plain text documents
    DefaultEncoding(u8),
    /// Format > Text > Copy Ruler and Paste Ruler (the tab stop distance)
    CopyRuler,
    PasteRuler,
    /// File > Print > Rewrap to Fit Page
    Rewrap,
    /// View > Use Dark Background for Windows
    DarkBackground,
    /// Edit > Substitutions > Text Replacement
    TextReplacement,
    /// Preferences > New Documents > Default Font (index into `text::families`, every installed family)
    DefaultFont(u8),
    /// Edit > Find > Select Line: the panel's Select and Cancel buttons
    LineDone,
    LineCancel,
    /// Edit > Spelling and Grammar > Show Spelling and Grammar, and the panel's buttons (Find Next is `SpellNext`)
    ShowSpelling,
    SpellingIgnore,
    SpellingChange,
    SpellingDone,
    /// Preferences: the panel's Done button, the item that opens it, and a row of it (index into `prefs::Setting::ALL`)
    ShowSettings,
    SettingsDone,
    Setting(u8),
    /// Format > Text > Spacing, and the panel's Done and Cancel buttons
    ShowSpacing,
    SpacingDone,
    SpacingCancel,
    /// Preferences > Customize Encodings List, its Done button, and a row (index into `encoding::ALL`)
    ShowEncodings,
    EncodingsDone,
    EncodingToggle(u8),
    /// Format > Text > Writing Direction: right to left (true) or left to right (false)
    Direction(bool),
    /// The format bar's style pop-up (0 Regular, 1 Bold, 2 Italic, 3 Bold Italic) and size pop-up (index into `SIZES`)
    FontStyle(u8),
    Size(u8),
    /// Format > Font > Show Fonts and Show Colors (the font and colour panels), and their Done buttons
    ShowFonts,
    ShowColors,
    FontsDone,
    ColorsDone,
}

/// The first menu id of Text Editor's own items. Ids are stable: they are the values a menu sends back.
const MENU_BASE: u32 = 0x4E00;
/// The ids of the families (`Action::Font`), one per `Style::font` index 0 to 255, after every other item's id.
const FONT_IDS: u32 = 0x100;
const FONT_IDS_END: u32 = FONT_IDS + u8::MAX as u32;
/// The ids of Preferences > Default Font (`Action::DefaultFont`), one per family index, after the families' ids.
const DEFAULT_FONT_IDS: u32 = FONT_IDS_END + 1;
const DEFAULT_FONT_IDS_END: u32 = DEFAULT_FONT_IDS + u8::MAX as u32;

impl Action {
    /// The menu id of this action, or `None` for document commands (they have the controller's ids).
    pub fn id(self) -> Option<u32> {
        Some(match self {
            Action::Document(_) => return None,
            Action::Undo => MENU_BASE + 1,
            Action::Redo => MENU_BASE + 2,
            Action::Cut => MENU_BASE + 3,
            Action::Copy => MENU_BASE + 4,
            Action::Paste => MENU_BASE + 5,
            Action::SelectAll => MENU_BASE + 6,
            Action::Find => MENU_BASE + 7,
            Action::FindNext => MENU_BASE + 8,
            Action::FindPrevious => MENU_BASE + 9,
            Action::Replace => MENU_BASE + 10,
            Action::ReplaceAll => MENU_BASE + 11,
            Action::Bold => MENU_BASE + 12,
            Action::Italic => MENU_BASE + 13,
            Action::Underline => MENU_BASE + 14,
            Action::Bigger => MENU_BASE + 15,
            Action::Smaller => MENU_BASE + 16,
            Action::FindDone => MENU_BASE + 17,
            Action::FocusFind => MENU_BASE + 18,
            Action::FocusReplace => MENU_BASE + 19,
            // every family a document can use (`text::families`, up to 256): ids of their own past the other items
            Action::Font(i) => MENU_BASE + FONT_IDS + i as u32,
            Action::Align(a) => MENU_BASE + 0x30 + align_index(a),
            Action::Color(i) => MENU_BASE + 0x40 + i as u32,
            Action::Spacing(i) => MENU_BASE + 0x50 + i as u32,
            Action::List(ListKind::Bullet) => MENU_BASE + 0x58,
            Action::List(_) => MENU_BASE + 0x59,
            Action::Print => MENU_BASE + 0x60,
            Action::Paper(i) => MENU_BASE + 0x61 + i as u32,
            Action::Landscape(true) => MENU_BASE + 0x66,
            Action::Landscape(false) => MENU_BASE + 0x67,
            Action::MakePlain => MENU_BASE + 0x68,
            Action::MakeRich => MENU_BASE + 0x69,
            Action::SpellToggle => MENU_BASE + 0x6a,
            Action::SpellNext => MENU_BASE + 0x6b,
            Action::SpellLearn => MENU_BASE + 0x6c,
            Action::SpellIgnore => MENU_BASE + 0x6d,
            Action::SpellCorrect => MENU_BASE + 0x6e,
            Action::SaveAll => MENU_BASE + 0x70,
            Action::ExportPdf => MENU_BASE + 0x71,
            Action::PasteMatch => MENU_BASE + 0x72,
            Action::Delete => MENU_BASE + 0x73,
            Action::Complete => MENU_BASE + 0x74,
            Action::UseSelection => MENU_BASE + 0x75,
            Action::JumpToSelection => MENU_BASE + 0x76,
            Action::SelectLine => MENU_BASE + 0x77,
            Action::SpeakStart => MENU_BASE + 0xe9,
            Action::SpeakStop => MENU_BASE + 0xea,
            Action::InsertImage => MENU_BASE + 0xeb,
            Action::PlainDone => MENU_BASE + 0xec,
            Action::PlainCancel => MENU_BASE + 0xed,
            Action::Transform(Transform::Upper) => MENU_BASE + 0x78,
            Action::Transform(Transform::Lower) => MENU_BASE + 0x79,
            Action::Transform(Transform::Capitalize) => MENU_BASE + 0x7a,
            Action::Strike => MENU_BASE + 0x7b,
            Action::Superscript => MENU_BASE + 0x7c,
            Action::Subscript => MENU_BASE + 0x7d,
            Action::CopyStyle => MENU_BASE + 0x7e,
            Action::PasteStyle => MENU_BASE + 0x7f,
            Action::ShowRuler => MENU_BASE + 0x80,
            Action::PreventEditing => MENU_BASE + 0x81,
            Action::WrapToWindow => MENU_BASE + 0x82,
            Action::ZoomIn => MENU_BASE + 0x83,
            Action::ZoomOut => MENU_BASE + 0x84,
            Action::ActualSize => MENU_BASE + 0x85,
            Action::SmartQuotes => MENU_BASE + 0x86,
            Action::SmartDashes => MENU_BASE + 0x87,
            Action::SmartLinks => MENU_BASE + 0xb8,
            Action::SpellAuto => MENU_BASE + 0x88,
            Action::HeaderFooter => MENU_BASE + 0x89,
            Action::NewPlain(false) => MENU_BASE + 0x8a,
            Action::NewPlain(true) => MENU_BASE + 0x8b,
            Action::InsertPage => MENU_BASE + 0x98,
            Action::InsertLine => MENU_BASE + 0x99,
            Action::InsertParagraph => MENU_BASE + 0x9a,
            Action::AddLink => MENU_BASE + 0x9b,
            Action::OpenLink => MENU_BASE + 0x9c,
            Action::RemoveLink => MENU_BASE + 0x9d,
            Action::LinkDone => MENU_BASE + 0x9e,
            Action::LinkCancel => MENU_BASE + 0x9f,
            Action::LinkRemove => MENU_BASE + 0xb7,
            Action::ShowProperties => MENU_BASE + 0x8c,
            Action::PropsDone => MENU_BASE + 0x8d,
            Action::PropsCancel => MENU_BASE + 0x8e,
            Action::MarkupAsCode => MENU_BASE + 0x8f,
            Action::Highlight(i) => MENU_BASE + 0x90 + i as u32,
            Action::Encoding(i) => MENU_BASE + 0xa0 + i as u32,
            Action::Reinterpret(i) => MENU_BASE + 0xa8 + i as u32,
            Action::DefaultEncoding(i) => MENU_BASE + 0xb0 + i as u32,
            Action::CopyRuler => MENU_BASE + 0xb9,
            Action::PasteRuler => MENU_BASE + 0xba,
            Action::Rewrap => MENU_BASE + 0xbb,
            Action::DarkBackground => MENU_BASE + 0xbc,
            Action::TextReplacement => MENU_BASE + 0xbd,
            Action::DefaultFont(i) => MENU_BASE + DEFAULT_FONT_IDS + i as u32,
            Action::LineDone => MENU_BASE + 0xbe,
            Action::LineCancel => MENU_BASE + 0xbf,
            Action::ShowSpelling => MENU_BASE + 0xc3,
            Action::SpellingIgnore => MENU_BASE + 0xc4,
            Action::SpellingChange => MENU_BASE + 0xc5,
            Action::SpellingDone => MENU_BASE + 0xc6,
            Action::ShowSettings => MENU_BASE + 0xc7,
            Action::SettingsDone => MENU_BASE + 0xc8,
            Action::Setting(i) => MENU_BASE + 0xd0 + i as u32,
            Action::Direction(rtl) => MENU_BASE + if rtl { 0xe8 } else { 0xe7 },
            Action::ShowSpacing => MENU_BASE + 0xc9,
            Action::SpacingDone => MENU_BASE + 0xca,
            Action::SpacingCancel => MENU_BASE + 0xcb,
            Action::ShowEncodings => MENU_BASE + 0xcc,
            Action::EncodingsDone => MENU_BASE + 0xcd,
            Action::EncodingToggle(i) => MENU_BASE + 0xe0 + i as u32,
            Action::FontStyle(i) => MENU_BASE + 0xee + i as u32,
            Action::Size(i) => MENU_BASE + 0xf2 + i as u32,
            Action::ShowFonts => MENU_BASE + 0xfb,
            Action::ShowColors => MENU_BASE + 0xfc,
            Action::FontsDone => MENU_BASE + 0xfd,
            Action::ColorsDone => MENU_BASE + 0xfe,
        })
    }

    /// The action a menu id stands for, when it is one of Text Editor's.
    pub fn from_id(id: u32) -> Option<Action> {
        let offset = id.checked_sub(MENU_BASE)?;
        Some(match offset {
            1 => Action::Undo,
            2 => Action::Redo,
            3 => Action::Cut,
            4 => Action::Copy,
            5 => Action::Paste,
            6 => Action::SelectAll,
            7 => Action::Find,
            8 => Action::FindNext,
            9 => Action::FindPrevious,
            10 => Action::Replace,
            11 => Action::ReplaceAll,
            12 => Action::Bold,
            13 => Action::Italic,
            14 => Action::Underline,
            15 => Action::Bigger,
            16 => Action::Smaller,
            17 => Action::FindDone,
            18 => Action::FocusFind,
            19 => Action::FocusReplace,
            0x30..=0x33 => Action::Align(align_of_index(offset - 0x30)),
            0x40..=0x47 => Action::Color((offset - 0x40) as u8),
            0x50..=0x54 => Action::Spacing((offset - 0x50) as u8),
            0x58 => Action::List(ListKind::Bullet),
            0x59 => Action::List(ListKind::Numbered),
            0x60 => Action::Print,
            0x61..=0x65 => Action::Paper((offset - 0x61) as u8),
            0x66 => Action::Landscape(true),
            0x67 => Action::Landscape(false),
            0x68 => Action::MakePlain,
            0x69 => Action::MakeRich,
            0x6a => Action::SpellToggle,
            0x6b => Action::SpellNext,
            0x6c => Action::SpellLearn,
            0x6d => Action::SpellIgnore,
            0x6e => Action::SpellCorrect,
            0x70 => Action::SaveAll,
            0x71 => Action::ExportPdf,
            0x72 => Action::PasteMatch,
            0x73 => Action::Delete,
            0x74 => Action::Complete,
            0x75 => Action::UseSelection,
            0x76 => Action::JumpToSelection,
            0x77 => Action::SelectLine,
            0xe9 => Action::SpeakStart,
            0xea => Action::SpeakStop,
            0xeb => Action::InsertImage,
            0xec => Action::PlainDone,
            0xed => Action::PlainCancel,
            0x78 => Action::Transform(Transform::Upper),
            0x79 => Action::Transform(Transform::Lower),
            0x7a => Action::Transform(Transform::Capitalize),
            0x7b => Action::Strike,
            0x7c => Action::Superscript,
            0x7d => Action::Subscript,
            0x7e => Action::CopyStyle,
            0x7f => Action::PasteStyle,
            0x80 => Action::ShowRuler,
            0x81 => Action::PreventEditing,
            0x82 => Action::WrapToWindow,
            0x83 => Action::ZoomIn,
            0x84 => Action::ZoomOut,
            0x85 => Action::ActualSize,
            0x86 => Action::SmartQuotes,
            0x87 => Action::SmartDashes,
            0xb8 => Action::SmartLinks,
            0x88 => Action::SpellAuto,
            0x89 => Action::HeaderFooter,
            0x8a => Action::NewPlain(false),
            0x8b => Action::NewPlain(true),
            0x98 => Action::InsertPage,
            0x99 => Action::InsertLine,
            0x9a => Action::InsertParagraph,
            0x9b => Action::AddLink,
            0x9c => Action::OpenLink,
            0x9d => Action::RemoveLink,
            0x9e => Action::LinkDone,
            0x9f => Action::LinkCancel,
            0xb7 => Action::LinkRemove,
            0x8c => Action::ShowProperties,
            0x8d => Action::PropsDone,
            0x8e => Action::PropsCancel,
            0x8f => Action::MarkupAsCode,
            0x90..=0x97 => Action::Highlight((offset - 0x90) as u8),
            0xa0..=0xa6 => Action::Encoding((offset - 0xa0) as u8),
            0xa8..=0xae => Action::Reinterpret((offset - 0xa8) as u8),
            0xb0..=0xb6 => Action::DefaultEncoding((offset - 0xb0) as u8),
            0xb9 => Action::CopyRuler,
            0xba => Action::PasteRuler,
            0xbb => Action::Rewrap,
            0xbc => Action::DarkBackground,
            0xbd => Action::TextReplacement,
            0xbe => Action::LineDone,
            0xbf => Action::LineCancel,
            0xc3 => Action::ShowSpelling,
            0xc4 => Action::SpellingIgnore,
            0xc5 => Action::SpellingChange,
            0xc6 => Action::SpellingDone,
            0xc7 => Action::ShowSettings,
            0xc8 => Action::SettingsDone,
            0xd0..=0xde => Action::Setting((offset - 0xd0) as u8),
            0xc9 => Action::ShowSpacing,
            0xca => Action::SpacingDone,
            0xcb => Action::SpacingCancel,
            0xcc => Action::ShowEncodings,
            0xcd => Action::EncodingsDone,
            0xe0..=0xe6 => Action::EncodingToggle((offset - 0xe0) as u8),
            0xe7 => Action::Direction(false),
            0xe8 => Action::Direction(true),
            0xee..=0xf1 => Action::FontStyle((offset - 0xee) as u8),
            0xf2..=0xfa => Action::Size((offset - 0xf2) as u8),
            0xfb => Action::ShowFonts,
            0xfc => Action::ShowColors,
            0xfd => Action::FontsDone,
            0xfe => Action::ColorsDone,
            FONT_IDS..=FONT_IDS_END => Action::Font((offset - FONT_IDS) as u8),
            DEFAULT_FONT_IDS..=DEFAULT_FONT_IDS_END => Action::DefaultFont((offset - DEFAULT_FONT_IDS) as u8),
            _ => return None,
        })
    }
}

fn align_index(align: Align) -> u32 {
    match align {
        Align::Left => 0,
        Align::Center => 1,
        Align::Right => 2,
        Align::Justified => 3,
    }
}

fn align_of_index(index: u32) -> Align {
    match index {
        1 => Align::Center,
        2 => Align::Right,
        3 => Align::Justified,
        _ => Align::Left,
    }
}

/// A push control in the toolbar or the find bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub rect: Rect,
    pub label: String,
    /// `None` for a readout (the size) that does nothing when touched.
    pub action: Option<Action>,
    pub active: bool,
    pub enabled: bool,
    pub weight: Weight,
}

/// Which text field has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Body,
    Find,
    Replace,
}

/// What the format bar shows about the document: the style at the caret (or of the first selected character), the
/// paragraph's alignment, and whether it is rich text (a plain text document has no formatting to change).
pub struct Status {
    pub style: Style,
    pub align: Align,
    pub rich: bool,
}

/// The window's regions in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    /// Where the body (the page) starts and ends; the find bar sits above it when it is open.
    pub body_top: f32,
    pub body_bottom: f32,
    /// The format bar, the ruler (zero height when the document is plain text) and the find bar's top edge.
    pub format_top: f32,
    pub ruler_top: f32,
    pub ruler_h: f32,
    pub find_top: f32,
    /// Whether the find bar is open: its fields take presses only then (they sit where the page starts otherwise).
    pub find_open: bool,
    /// The column of text: its left edge and width, centred and never wider than the readable width.
    pub text_x: f32,
    pub text_w: f32,
    /// The find fields, when the bar is open.
    pub find_field: Rect,
    pub replace_field: Rect,
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn geometry(width: f32, height: f32, find_open: bool, ruler: bool) -> Geometry {
    geometry_with(width, height, find_open, ruler, false)
}

/// The regions, with the text column as wide as the window (`wrap_to_window`) instead of the readable width.
pub fn geometry_with(width: f32, height: f32, find_open: bool, ruler: bool, wrap_to_window: bool) -> Geometry {
    let bar = UNIFIED_BAR_PT;
    let format_top = bar;
    let ruler_top = format_top + FORMAT_H;
    let ruler_h = if ruler { RULER_H } else { 0.0 };
    let find_top = ruler_top + ruler_h;
    let body_top = find_top + if find_open { FIND_H } else { 0.0 };
    let body_bottom = (height - STATUS_H).max(body_top);
    let text_w = if wrap_to_window { (width - 2.0 * MARGIN_PT).max(0.0) } else { (width - 2.0 * MARGIN_PT).min(READABLE_MAX_PT).max(0.0) };
    let text_x = ((width - text_w) / 2.0).max(0.0);
    let x0 = traffic_lights_trailing();
    let row_y = find_top + (FIND_H - BUTTON_H) / 2.0;
    Geometry {
        width,
        height,
        body_top,
        body_bottom,
        format_top,
        ruler_top,
        ruler_h,
        find_top,
        find_open,
        text_x,
        text_w,
        find_field: Rect { x: x0, y: row_y, w: FIELD_W, h: BUTTON_H },
        replace_field: Rect { x: x0 + FIELD_W + ITEM_GAP, y: row_y, w: FIELD_W, h: BUTTON_H },
    }
}

/// The pictures of a document's attachments: each one's aspect ratio (width over height) and its decoded pixels, `None`
/// where the file is not an image we can read. Indexed like the attachments (`Style::attachment` less one).
pub struct Art {
    pub aspects: Vec<f32>,
    pub images: Vec<Option<Image>>,
}

/// The aspect ratio of each attachment, the cheap part of `art` the layout needs.
pub fn aspects(attachments: &[Attachment]) -> Vec<f32> {
    attachments.iter().map(|a| if a.width > 0.0 && a.height > 0.0 { a.width / a.height } else { 1.0 }).collect()
}

/// The pictures of the attachments, decoded. Decoding is the costly part, so the window does it when the layout is
/// rebuilt, not on every draw.
pub fn art(attachments: &[Attachment]) -> Art {
    Art { aspects: aspects(attachments), images: attachments.iter().map(|a| graphics_imageio::decode(&a.data)).collect() }
}

/// The box an attachment is drawn in: as tall as its line, as wide as its aspect ratio, and narrowed (keeping the
/// shape) to `fit` when it would be wider than the text.
pub fn object_box(aspects: &[f32], style: &Style, fit: f32) -> (f32, f32) {
    let h = style.size * layout::LINE_FACTOR;
    let aspect = (style.attachment as usize).checked_sub(1).and_then(|i| aspects.get(i)).copied().unwrap_or(1.0);
    let w = h * aspect;
    if fit > 0.0 && w > fit {
        (fit, fit / aspect)
    } else {
        (w, h)
    }
}

/// The width of a character for layout: an attachment is its box's width, anything else its glyph's.
pub fn measure_in<'a>(ui: &'a Ui, aspects: &'a [f32], fit: f32) -> impl Fn(char, &Style) -> f32 + 'a {
    move |ch: char, style: &Style| if ch == OBJECT { object_box(aspects, style, fit).0 } else { char_width(ui, ch, style) }
}

/// The width of one character in a style, in points, in the style's family (`family_font`), as the page draws it.
pub fn char_width(ui: &Ui, ch: char, style: &Style) -> f32 {
    let mut buf = [0u8; 4];
    let text: &str = ch.encode_utf8(&mut buf);
    crate::family_font::with_family(ui, style.font, text, |u| u.measure(text, drawn_size(style), weight_of(style)) / u.s)
}

/// How much smaller superscript and subscript text is drawn.
pub const SCRIPT_SCALE: f32 = 0.65;

/// The size a style's text is drawn in: smaller when it is raised or lowered.
pub fn drawn_size(style: &Style) -> f32 {
    if style.script != 0 {
        style.size * SCRIPT_SCALE
    } else {
        style.size
    }
}

/// The fill behind highlighted text: the palette colour, lightened so dark text reads on it.
pub fn highlight_color(index: u8) -> u32 {
    let [r, g, b] = COLORS[(index as usize).min(COLORS.len() - 1)].1;
    let lighten = |c: u8| ((c as u32 * 2 + 255) / 3) as u8;
    pack(lighten(r), lighten(g), lighten(b), 255)
}

/// The weight a style draws in: bold text is heavier, everything else regular.
pub fn weight_of(style: &Style) -> Weight {
    if style.bold {
        Weight::Bold
    } else {
        Weight::Regular
    }
}

/// The face name a font index stands for, for the menu and the readout.
pub fn family_name(font: u8) -> &'static str {
    crate::text::families().get(font as usize).map_or(FAMILIES[0], String::as_str)
}

fn label_button(ui: &Ui, label: &str, action: Option<Action>, weight: Weight) -> Button {
    let width = ui.measure(label, MAC_BODY_PT, weight) / ui.s + 2.0 * BUTTON_PAD;
    Button { rect: Rect { x: 0.0, y: 0.0, w: width, h: BUTTON_H }, label: label.to_string(), action, active: false, enabled: true, weight }
}

/// The fill of a text colour: the window's label colour for the default, the palette colour otherwise.
pub fn text_color(index: u8) -> u32 {
    if index == 0 {
        LABEL.now()
    } else {
        let [r, g, b] = COLORS[(index as usize).min(COLORS.len() - 1)].1;
        pack(r, g, b, 255)
    }
}

/// Places a row of buttons from `x`, groups separated by `GROUP_GAP`, and returns the x after the row.
fn place_row(buttons: &mut [Button], groups: &[usize], mut x: f32, y: f32) -> f32 {
    let mut index = 0;
    for (g, &count) in groups.iter().enumerate() {
        if g > 0 {
            x += GROUP_GAP;
        }
        for k in 0..count {
            if k > 0 {
                x += ITEM_GAP;
            }
            let b = &mut buttons[index];
            b.rect.x = x;
            b.rect.y = y;
            x += b.rect.w;
            index += 1;
        }
    }
    x
}

/// The label of a line spacing multiple: "1.0", "1.15"...
pub fn spacing_label(index: u8) -> String {
    let m = SPACINGS.get(index as usize).copied().unwrap_or(1.0);
    if (m * 10.0).fract() == 0.0 {
        format!("{m:.1}")
    } else {
        format!("{m}")
    }
}

/// The find bar's buttons, to the right of the two fields.
pub fn find_buttons(ui: &Ui, geom: &Geometry) -> Vec<Button> {
    let mut buttons = vec![
        label_button(ui, "Next", Some(Action::FindNext), Weight::Regular),
        label_button(ui, "Previous", Some(Action::FindPrevious), Weight::Regular),
        label_button(ui, "Replace", Some(Action::Replace), Weight::Regular),
        label_button(ui, "Replace All", Some(Action::ReplaceAll), Weight::Regular),
        label_button(ui, "Done", Some(Action::FindDone), Weight::Regular),
    ];
    let x = geom.replace_field.x + geom.replace_field.w + GROUP_GAP;
    place_row(&mut buttons, &[5], x, geom.find_field.y);
    buttons
}

/// The find bar's button or field under a point, in points. The find bar takes presses only while it is open: its
/// fields are laid out where the page starts when it is closed, and must not take the presses meant for the text.
pub fn hit(buttons: &[Button], geom: &Geometry, x: f32, y: f32) -> Option<Hit> {
    if !geom.find_open {
        return None;
    }
    if let Some(b) = buttons.iter().find(|b| b.rect.contains(x, y)) {
        return Some(Hit::Button(b.clone()));
    }
    if geom.find_field.contains(x, y) {
        return Some(Hit::Field(Focus::Find));
    }
    if geom.replace_field.contains(x, y) {
        return Some(Hit::Field(Focus::Replace));
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Button(Button),
    Field(Focus),
}

fn to_px(ui: &Ui, r: Rect) -> RectF {
    RectF::new(ui.pt(r.x).floor(), ui.pt(r.y).floor(), ui.pt(r.w).ceil(), ui.pt(r.h).ceil())
}

/// A content hash of a rendered image (FNV-1a over its size and pixels), the key the GPU side caches it by.
fn image_id(img: &Image) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |word: u64| {
        h ^= word;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(img.w as u64);
    feed(img.h as u64);
    for &p in &img.px {
        feed(p as u64);
    }
    h
}

/// Draws a rendered image with its top-left at `(x, y)` device pixels.
pub fn blit(c: &mut dyn Canvas, img: &Image, x: f32, y: f32) {
    c.image(ImageRef { id: image_id(img), image: img }, RectF::new(x.floor(), y.floor(), img.w as f32, img.h as f32), 1.0);
}

/// Draws an attachment in its box at `(x, y)` points: its picture, or a grey plate where the file is not an image.
fn draw_object(c: &mut dyn Canvas, ui: &Ui, art: &Art, style: &Style, x: f32, y: f32, w: f32, h: f32) {
    let rect = RectF::new(ui.pt(x).floor(), ui.pt(y).floor(), ui.pt(w).ceil(), ui.pt(h).ceil());
    match (style.attachment as usize).checked_sub(1).and_then(|i| art.images.get(i)).and_then(|i| i.as_ref()) {
        Some(img) => c.image(ImageRef { id: image_id(img), image: img }, rect, 1.0),
        None => c.fill_rect(rect, LABEL_3.now()),
    }
}

/// Draws a text run with its top-left at `(x, y)` points.
fn text_at(c: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) -> f32 {
    let img = ui.text(text, pt, weight, color);
    let width = img.w as f32 / ui.s;
    blit(c, &img, ui.pt(x), ui.pt(y));
    width
}

/// Draws the whole window background and the hairline under the format bar. The title row above it has none: the title
/// and the format bar are one unified toolbar.
pub fn draw_bar(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry) {
    c.fill_rect(RectF::new(0.0, 0.0, ui.pt(geom.width).ceil(), ui.pt(geom.height).ceil()), DETAIL_BG.now());
    let sep = ui.pt(geom.ruler_top).floor() - 1.0;
    c.fill_rect(RectF::new(0.0, sep.max(0.0), ui.pt(geom.width).ceil(), 1.0), SEPARATOR_C.now());
}

/// Draws the document's name in the title row, centred between the traffic lights and the window's right edge, with the
/// proxy icon in front of it. `title` is the window title (the name, and "Edited" while there are unsaved changes).
pub fn draw_title(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, title: &str) {
    if title.is_empty() {
        return;
    }
    // the title keeps clear of the traffic lights on both sides, so it stays centred
    let room = (geom.width - 2.0 * (traffic_lights_trailing() + BAR_GAP) - PROXY_ICON_PT - BAR_GAP).max(0.0);
    let shown = ui.fit(title, MAC_BODY_PT, ui.pt(room));
    let img = ui.text(&shown, MAC_BODY_PT, Weight::Semibold, LABEL.now());
    let text_w = img.w as f32 / ui.s;
    let x0 = (geom.width - (PROXY_ICON_PT + BAR_GAP + text_w)) / 2.0;
    let mid = UNIFIED_BAR_PT / 2.0;
    glyph(c, ui, "doc", Rect { x: x0, y: mid - PROXY_ICON_PT / 2.0, w: PROXY_ICON_PT, h: PROXY_ICON_PT }, LABEL_2.now());
    blit(c, &img, ui.pt(x0 + PROXY_ICON_PT + BAR_GAP).floor(), (ui.pt(mid) - img.h as f32 / 2.0).floor());
}

/// Draws a button: its plate when active, and its label centred.
pub fn draw_button(c: &mut dyn Canvas, ui: &Ui, b: &Button) {
    let r = to_px(ui, b.rect);
    if b.active {
        c.fill_rrect(r, ui.pt(CONTROL_RADIUS), SEGMENT_ON.now());
    }
    let color = if b.enabled { LABEL.now() } else { LABEL_3.now() };
    let img = ui.text(&b.label, MAC_BODY_PT, b.weight, color);
    let x = r.x + (r.w - img.w as f32) / 2.0;
    let y = r.y + (r.h - img.h as f32) / 2.0;
    blit(c, &img, x, y);
}

/// Draws the find bar: its two fields, their text, and its buttons.
/// `selected`: the focused field's whole text is selected (typing replaces it), drawn on a selection plate.
pub fn draw_find(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, buttons: &[Button], find: &str, replace: &str, focus: Focus, selected: bool) {
    let top = geom.find_top;
    c.fill_rect(RectF::new(0.0, ui.pt(top), ui.pt(geom.width).ceil(), ui.pt(FIND_H).ceil()), DETAIL_BG.now());
    c.fill_rect(RectF::new(0.0, ui.pt(top + FIND_H).floor() - 1.0, ui.pt(geom.width).ceil(), 1.0), SEPARATOR_C.now());
    for (rect, text, placeholder, which) in [
        (geom.find_field, find, "Find", Focus::Find),
        (geom.replace_field, replace, "Replace with", Focus::Replace),
    ] {
        let r = to_px(ui, rect);
        c.fill_rrect(r, ui.pt(CONTROL_RADIUS), FIELD_BG.now());
        if focus == which {
            c.stroke_rrect(r, ui.pt(CONTROL_RADIUS), ui.pt(1.0).max(1.0), LINK);
        }
        let picked = selected && focus == which && !text.is_empty();
        let (shown, color) = if text.is_empty() { (placeholder, LABEL_3.now()) } else if picked { (text, WHITE) } else { (text, LABEL.now()) };
        let inner = ui.pt(rect.w) - 2.0 * ui.pt(BUTTON_PAD);
        let fitted = ui.fit(shown, MAC_BODY_PT, inner.max(0.0));
        let img = ui.text(&fitted, MAC_BODY_PT, Weight::Regular, color);
        if picked {
            c.fill_rrect(RectF::new(r.x + ui.pt(BUTTON_PAD) - 1.0, r.y + ui.pt(3.0), img.w as f32 + 2.0, r.h - ui.pt(6.0)), ui.pt(3.0), SELECTION.now());
        }
        blit(c, &img, r.x + ui.pt(BUTTON_PAD), r.y + (r.h - img.h as f32) / 2.0);
    }
    for b in buttons {
        draw_button(c, ui, b);
    }
}

/// The buttons of a panel, right-aligned on its bottom row: a label, what it does, and whether it is the default.
pub fn sheet_buttons(ui: &Ui, l: &PanelLayout, specs: &[(&str, Action, bool)]) -> Vec<Button> {
    let mut buttons: Vec<Button> = specs
        .iter()
        .map(|(label, action, default)| {
            let mut b = label_button(ui, label, Some(*action), if *default { Weight::Bold } else { Weight::Regular });
            b.active = *default;
            b
        })
        .collect();
    let total = buttons.iter().map(|b| b.rect.w).sum::<f32>() + ITEM_GAP * buttons.len().saturating_sub(1) as f32;
    place_row(&mut buttons, &[specs.len()], l.right - total, l.buttons_y);
    buttons
}

/// The document properties panel's buttons.
pub fn properties_buttons(ui: &Ui, l: &PanelLayout) -> Vec<Button> {
    sheet_buttons(ui, l, &[("Cancel", Action::PropsCancel, false), ("Done", Action::PropsDone, true)])
}

/// The Show Spelling and Grammar panel's buttons: Change is the default, Find Next moves to the next misspelled word.
pub fn spelling_buttons(ui: &Ui, l: &PanelLayout) -> Vec<Button> {
    sheet_buttons(
        ui,
        l,
        &[
            ("Done", Action::SpellingDone, false),
            ("Ignore", Action::SpellingIgnore, false),
            ("Learn", Action::SpellLearn, false),
            ("Find Next", Action::SpellNext, false),
            ("Change", Action::SpellingChange, true),
        ],
    )
}

/// The Spacing panel's buttons.
pub fn spacing_buttons(ui: &Ui, l: &PanelLayout) -> Vec<Button> {
    sheet_buttons(ui, l, &[("Cancel", Action::SpacingCancel, false), ("Done", Action::SpacingDone, true)])
}

/// The Select Line panel's buttons.
/// The buttons of the Make Plain Text confirmation: Remove Attachments is the default.
pub fn plain_buttons(ui: &Ui, l: &PanelLayout) -> Vec<Button> {
    sheet_buttons(ui, l, &[("Cancel", Action::PlainCancel, false), ("Remove Attachments", Action::PlainDone, true)])
}

/// The confirmation sheet: its title, a one-line message in the first row, and its buttons.
pub fn draw_confirm(c: &mut dyn Canvas, ui: &Ui, l: &PanelLayout, title: &str, message: &str, buttons: &[Button]) {
    draw_sheet(c, ui, l, title, &[], 0, buttons);
    text_at(c, ui, message, MAC_BODY_PT, Weight::Regular, LABEL.now(), l.labels_x, l.fields[0].y);
}

pub fn line_buttons(ui: &Ui, l: &PanelLayout) -> Vec<Button> {
    sheet_buttons(ui, l, &[("Cancel", Action::LineCancel, false), ("Select", Action::LineDone, true)])
}

/// The link panel's buttons; Remove Link is offered when the text already links somewhere.
pub fn link_buttons(ui: &Ui, l: &PanelLayout, can_remove: bool) -> Vec<Button> {
    let mut specs = vec![("Cancel", Action::LinkCancel, false)];
    if can_remove {
        specs.push(("Remove Link", Action::LinkRemove, false));
    }
    specs.push(("Done", Action::LinkDone, true));
    sheet_buttons(ui, l, &specs)
}

/// Draws a panel over the page: its title, its labelled fields (the focused one outlined) and its buttons.
pub fn draw_sheet(c: &mut dyn Canvas, ui: &Ui, l: &PanelLayout, title: &str, fields: &[(&str, &str)], focus: usize, buttons: &[Button]) {
    let frame = to_px(ui, l.frame);
    c.fill_rrect(frame, ui.pt(CONTROL_RADIUS), DETAIL_BG.now());
    c.stroke_rrect(frame, ui.pt(CONTROL_RADIUS), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
    text_at(c, ui, title, MAC_BODY_PT, Weight::Bold, LABEL.now(), l.labels_x, l.title_y);
    for (i, ((label, value), rect)) in fields.iter().zip(l.fields.iter()).enumerate() {
        let r = to_px(ui, *rect);
        c.fill_rrect(r, ui.pt(CONTROL_RADIUS), FIELD_BG.now());
        if focus == i {
            c.stroke_rrect(r, ui.pt(CONTROL_RADIUS), ui.pt(1.0).max(1.0), LINK);
        }
        let name = ui.text(label, MAC_BODY_PT, Weight::Regular, LABEL_3.now());
        blit(c, &name, ui.pt(l.labels_x), r.y + (r.h - name.h as f32) / 2.0);
        // the end of a long value is the part being typed, so it is the part shown
        let inner = (ui.pt(rect.w) - 2.0 * ui.pt(BUTTON_PAD)).max(0.0);
        let shown = tail_that_fits(ui, value, inner);
        let img = ui.text(&shown, MAC_BODY_PT, Weight::Regular, LABEL.now());
        blit(c, &img, r.x + ui.pt(BUTTON_PAD), r.y + (r.h - img.h as f32) / 2.0);
    }
    for b in buttons {
        draw_button(c, ui, b);
    }
}

/// A list panel's Done button, on its bottom row at the right; it does `action`.
pub fn list_buttons(ui: &Ui, l: &ListLayout, action: Action) -> Vec<Button> {
    let mut done = label_button(ui, "Done", Some(action), Weight::Bold);
    done.active = true;
    let width = done.rect.w;
    let mut buttons = vec![done];
    place_row(&mut buttons, &[1], l.right - width, l.buttons_y);
    buttons
}

/// Draws a list panel: its title, one row per item (the name at the left, the value at the right), and its buttons. The
/// rows are the panel's own touch targets (see `ListLayout::rows`).
pub fn draw_list(c: &mut dyn Canvas, ui: &Ui, l: &ListLayout, title: &str, rows: &[(&str, String)], buttons: &[Button]) {
    let frame = to_px(ui, l.frame);
    c.fill_rrect(frame, ui.pt(CONTROL_RADIUS), DETAIL_BG.now());
    c.stroke_rrect(frame, ui.pt(CONTROL_RADIUS), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
    text_at(c, ui, title, MAC_BODY_PT, Weight::Bold, LABEL.now(), l.labels_x, l.title_y);
    for ((label, value), rect) in rows.iter().zip(l.rows.iter()) {
        let r = to_px(ui, *rect);
        c.fill_rrect(r, ui.pt(CONTROL_RADIUS), FIELD_BG.now());
        let name = ui.text(label, MAC_BODY_PT, Weight::Regular, LABEL.now());
        blit(c, &name, r.x + ui.pt(BUTTON_PAD), r.y + (r.h - name.h as f32) / 2.0);
        let shown = ui.text(value, MAC_BODY_PT, Weight::Regular, LABEL_3.now());
        blit(c, &shown, r.x + r.w - ui.pt(BUTTON_PAD) - shown.w as f32, r.y + (r.h - shown.h as f32) / 2.0);
    }
    for b in buttons {
        draw_button(c, ui, b);
    }
}

/// The longest end of `text` that fits in `max_px`.
fn tail_that_fits(ui: &Ui, text: &str, max_px: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if ui.measure(&tail, MAC_BODY_PT, Weight::Regular) <= max_px {
            return tail;
        }
        // drop a chunk at a time so a very long address stays cheap
        start += ((chars.len() - start) / 8).max(1);
    }
    String::new()
}

/// The page's fill when View > Use Dark Background for Windows is on.
fn dark_page() -> u32 {
    DETAIL_BG.dark
}

/// The colour of text with no colour of its own: the window's label colour, or white on a dark page.
pub fn default_ink(dark: bool) -> u32 {
    if dark {
        LABEL.dark
    } else {
        LABEL.now()
    }
}

/// The page: its background and a clip to the body so text never draws over the bars.
pub fn page_clip(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, dark: bool) -> RectF {
    let r = RectF::new(0.0, ui.pt(geom.body_top).floor(), ui.pt(geom.width).ceil(), ui.pt(geom.body_bottom - geom.body_top).ceil());
    c.fill_rect(r, if dark { dark_page() } else { CONTENT_BG.now() });
    c.push_clip(r, 0.0);
    r
}

/// Draws the selection, the text and the caret at `head`. `scroll` is how far the body is scrolled, in points. `dark`
/// is the dark page: text with no colour of its own is white.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
/// `matches` are the find bar's matches, highlighted behind the text (`findHighlightColor`) while the bar is open.
pub fn draw_text(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, lines: &Layout, cells: &[Cell], art: &Art, sel: std::ops::Range<usize>, head: usize, scroll: f32, caret: bool, misspelled: &[std::ops::Range<usize>], matches: &[std::ops::Range<usize>], dark: bool) {
    let origin_x = geom.text_x;
    let origin_y = geom.body_top - scroll;
    let measure = measure_in(ui, &art.aspects, lines.width);
    // find matches: a plate per line each one crosses, under the selection (the current match is the selection)
    for m in matches {
        for line in lines.lines.iter().filter(|l| m.start <= l.end && m.end >= l.start) {
            let x0 = lines.x_in_line(cells, line, m.start.max(line.start), &measure);
            let x1 = lines.x_in_line(cells, line, m.end.min(line.end), &measure);
            if x1 > x0 {
                let rect = Rect { x: origin_x + x0, y: origin_y + line.top, w: x1 - x0, h: line.height };
                c.fill_rrect(to_px(ui, rect), ui.pt(2.0), FIND_HIGHLIGHT.now());
            }
        }
    }
    // selection: one plate per line the selection crosses; a newline inside it shows as the line's end
    if !sel.is_empty() {
        for line in &lines.lines {
            if sel.start > line.end || sel.end < line.start {
                continue;
            }
            let x0 = lines.x_in_line(cells, line, sel.start.max(line.start), &measure);
            let x1 = if sel.end > line.end {
                lines.x_offset(line) + line.width
            } else {
                lines.x_in_line(cells, line, sel.end, &measure)
            };
            if x1 > x0 {
                let rect = Rect { x: origin_x + x0, y: origin_y + line.top, w: x1 - x0, h: line.height };
                c.fill_rect(to_px(ui, rect), SELECTION.now());
            }
        }
    }
    // text: runs of one style per line
    for line in &lines.lines {
        draw_line(c, ui, lines, line, cells, art, origin_x, origin_y, dark);
        for word in misspelled.iter().filter(|w| w.start < line.end && w.end > line.start) {
            let x0 = lines.x_in_line(cells, line, word.start.max(line.start), &measure);
            let x1 = lines.x_in_line(cells, line, word.end.min(line.end), &measure);
            squiggle(c, ui, origin_x + x0, origin_x + x1, origin_y + line.top + line.height - 2.0);
        }
    }
    if caret {
        let (x, top, height) = lines.caret(cells, head, &measure);
        let rect = Rect { x: origin_x + x, y: origin_y + top, w: 1.0, h: height };
        c.fill_rect(to_px(ui, rect), default_ink(dark));
    }
}

/// A wavy rule from `x0` to `x1` (points) at `y`: short segments alternating up and down, in the destructive colour.
fn squiggle(c: &mut dyn Canvas, ui: &Ui, x0: f32, x1: f32, y: f32) {
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        let w = 2.0f32.min(x1 - x);
        let rect = Rect { x, y: if up { y } else { y + 1.0 }, w, h: 1.0 };
        c.fill_rect(to_px(ui, rect), DESTRUCTIVE);
        x += w;
        up = !up;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(c: &mut dyn Canvas, ui: &Ui, lines: &Layout, line: &Line, cells: &[Cell], art: &Art, origin_x: f32, origin_y: f32, dark: bool) {
    let measure = measure_in(ui, &art.aspects, lines.width);
    let y = origin_y + line.top;
    // the list marker hangs left of the text
    let marker = match line.marker {
        Marker::None => None,
        Marker::Bullet => Some("\u{2022}".to_string()),
        Marker::Number(n) => Some(format!("{n}.")),
    };
    if let Some(m) = marker {
        let size = cells.get(line.start).or(cells.last()).map_or(crate::text::DEFAULT_SIZE, |c| c.style.size);
        let w = ui.measure(&m, size, Weight::Regular) / ui.s;
        text_at(c, ui, &m, size, Weight::Regular, default_ink(dark), origin_x + line.indent - 6.0 - w, y);
    }
    let base = origin_x + lines.x_offset(line);
    let mut rel = 0.0;
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    let mut run_x = 0.0;
    for cell in &cells[line.start..line.end] {
        let step = layout::advance(rel, cell, lines.tab, &measure);
        if cell.ch == OBJECT {
            // an attachment is drawn on its own, between the runs of text either side of it
            if let Some(s) = run_style.take() {
                flush_run(c, ui, &run, &s, base + run_x, y, line.height, dark);
            }
            run.clear();
            let (w, h) = object_box(&art.aspects, &cell.style, lines.width);
            draw_object(c, ui, art, &cell.style, base + rel, y, w, h);
            rel += step;
            continue;
        }
        let breaks = cell.ch == '\t' || run_style.is_some_and(|s| !s.same_run(&cell.style));
        if breaks {
            if let Some(s) = run_style.take() {
                flush_run(c, ui, &run, &s, base + run_x, y, line.height, dark);
            }
            run.clear();
        }
        if cell.ch != '\t' && !crate::text::is_break(cell.ch) {
            if run_style.is_none() {
                run_style = Some(cell.style);
                run_x = rel;
            }
            run.push(cell.ch);
        }
        rel += step;
    }
    if let Some(s) = run_style {
        flush_run(c, ui, &run, &s, base + run_x, y, line.height, dark);
    }
    // a page break shows as a rule across the rest of its line
    if line.end > line.start && cells[line.end - 1].ch == crate::text::PAGE_BREAK {
        let from = base + line.width + 4.0;
        let to = origin_x + lines.width;
        if to > from {
            let rule = Rect { x: from, y: y + line.height / 2.0, w: to - from, h: 1.0 };
            c.fill_rect(to_px(ui, rule), LABEL_3.now());
        }
    }
}

/// Draws one run and returns the x after it. Underlines and strikethrough are a rule through the run, a highlight is
/// a plate behind it, and raised or lowered text is smaller and shifted.
#[allow(clippy::too_many_arguments)]
fn flush_run(c: &mut dyn Canvas, ui: &Ui, text: &str, style: &Style, x: f32, y: f32, line_height: f32, dark: bool) -> f32 {
    if text.is_empty() {
        return x;
    }
    let shown: String = text.chars().filter(|&ch| ch != '\n').collect();
    if shown.is_empty() {
        return x;
    }
    // text on a highlight is dark whatever the appearance, as the plate is light; a link is the link colour
    let color = if style.link != 0 {
        LINK
    } else if style.highlight > 0 && style.color == 0 {
        BLACK
    } else if style.color == 0 {
        default_ink(dark)
    } else {
        text_color(style.color)
    };
    let size = drawn_size(style);
    let shift = match style.script {
        1 => 0.0,
        -1 => style.size * 0.3,
        _ => 0.0,
    };
    // the run's own family, measured and drawn alike (the layout measured it the same way, `char_width`)
    let (width, img) = crate::family_font::with_family(ui, style.font, &shown, |u| (u.measure(&shown, size, weight_of(style)) / u.s, u.text(&shown, size, weight_of(style), color)));
    if style.highlight > 0 {
        let plate = Rect { x, y, w: width, h: line_height };
        c.fill_rect(to_px(ui, plate), highlight_color(style.highlight));
    }
    blit(c, &img, ui.pt(x), ui.pt(y + shift));
    if style.underline || style.link != 0 {
        let rule = Rect { x, y: y + line_height.min(style.size * 1.25) - 2.0, w: width, h: 1.0 };
        c.fill_rect(to_px(ui, rule), color);
    }
    if style.strike {
        let rule = Rect { x, y: y + line_height.min(style.size * 1.25) * 0.55, w: width, h: 1.0 };
        c.fill_rect(to_px(ui, rule), color);
    }
    x + width
}

/// The tab ruler above the page: a tick every 12 points, a number at every 72, and a marker at each tab stop. A click
/// on it moves the stops (see `ruler_tab`).
pub fn draw_ruler(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, tab: f32) {
    if geom.ruler_h <= 0.0 {
        return;
    }
    let top = geom.ruler_top;
    c.fill_rect(RectF::new(0.0, ui.pt(top), ui.pt(geom.width).ceil(), ui.pt(geom.ruler_h).ceil()), DETAIL_BG.now());
    c.fill_rect(RectF::new(0.0, ui.pt(top + geom.ruler_h).floor() - 1.0, ui.pt(geom.width).ceil(), 1.0), SEPARATOR_C.now());
    let mut mark = 0.0;
    while mark <= geom.text_w {
        let major = (mark / 72.0).fract() == 0.0;
        let h = if major { 8.0 } else { 4.0 };
        let rect = Rect { x: geom.text_x + mark, y: top + geom.ruler_h - 2.0 - h, w: 1.0, h };
        c.fill_rect(to_px(ui, rect), LABEL_3.now());
        if major && mark > 0.0 {
            let n = format!("{}", (mark / 72.0) as u32);
            text_at(c, ui, &n, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), geom.text_x + mark + 2.0, top + 1.0);
        }
        mark += 12.0;
    }
    if tab > 0.0 {
        let mut stop = tab;
        while stop <= geom.text_w {
            let rect = Rect { x: geom.text_x + stop - 2.0, y: top + geom.ruler_h - 8.0, w: 5.0, h: 6.0 };
            c.fill_rect(to_px(ui, rect), LINK);
            stop += tab;
        }
    }
}

/// The tab stop distance a click at `x`, `y` (points) on the ruler asks for, snapped to 6 points; `None` off the ruler.
pub fn ruler_tab(geom: &Geometry, x: f32, y: f32) -> Option<f32> {
    if geom.ruler_h <= 0.0 || y < geom.ruler_top || y >= geom.ruler_top + geom.ruler_h || x < geom.text_x || x > geom.text_x + geom.text_w {
        return None;
    }
    Some(((x - geom.text_x) / 6.0).round() * 6.0)
}

/// The word count at the bottom of the window, centred.
pub fn draw_status(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, words: usize, chars: usize, note: Option<&str>) {
    let top = geom.height - STATUS_H;
    c.fill_rect(RectF::new(0.0, ui.pt(top).floor(), ui.pt(geom.width).ceil(), ui.pt(STATUS_H).ceil()), DETAIL_BG.now());
    let text = match note {
        Some(note) => note.to_string(),
        None => format!("{words} words, {chars} characters"),
    };
    let img = ui.text(&text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now());
    let x = (ui.pt(geom.width) - img.w as f32) / 2.0;
    let y = ui.pt(top) + (ui.pt(STATUS_H) - img.h as f32) / 2.0;
    blit(c, &img, x, y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip() {
        let all = [
            Action::Undo,
            Action::Redo,
            Action::Cut,
            Action::Copy,
            Action::Paste,
            Action::SelectAll,
            Action::Find,
            Action::FindNext,
            Action::FindPrevious,
            Action::Replace,
            Action::ReplaceAll,
            Action::Bold,
            Action::Italic,
            Action::Underline,
            Action::Bigger,
            Action::Smaller,
            Action::FindDone,
            Action::FocusFind,
            Action::FocusReplace,
            Action::Font(2),
            Action::Align(Align::Justified),
            Action::Color(7),
            Action::Spacing(4),
            Action::List(ListKind::Bullet),
            Action::List(ListKind::Numbered),
            Action::Print,
            Action::Paper(4),
            Action::Landscape(true),
            Action::Landscape(false),
            Action::MakePlain,
            Action::MakeRich,
            Action::SpellToggle,
            Action::SpellNext,
            Action::SpellLearn,
            Action::SpellIgnore,
            Action::SpellCorrect,
            Action::SaveAll,
            Action::ExportPdf,
            Action::PasteMatch,
            Action::Delete,
            Action::Complete,
            Action::UseSelection,
            Action::JumpToSelection,
            Action::SelectLine,
            Action::SpeakStart,
            Action::SpeakStop,
            Action::Transform(Transform::Upper),
            Action::Transform(Transform::Lower),
            Action::Transform(Transform::Capitalize),
            Action::Strike,
            Action::Superscript,
            Action::Subscript,
            Action::Highlight(0),
            Action::Highlight(7),
            Action::CopyStyle,
            Action::PasteStyle,
            Action::PreventEditing,
            Action::ShowRuler,
            Action::WrapToWindow,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ActualSize,
            Action::SmartQuotes,
            Action::SmartDashes,
            Action::SmartLinks,
            Action::SpellAuto,
            Action::HeaderFooter,
            Action::NewPlain(false),
            Action::NewPlain(true),
            Action::InsertPage,
            Action::InsertLine,
            Action::InsertParagraph,
            Action::AddLink,
            Action::OpenLink,
            Action::RemoveLink,
            Action::LinkDone,
            Action::LinkCancel,
            Action::LinkRemove,
            Action::ShowProperties,
            Action::PropsDone,
            Action::PropsCancel,
            Action::MarkupAsCode,
            Action::Encoding(0),
            Action::Encoding(6),
            Action::Reinterpret(0),
            Action::Reinterpret(6),
            Action::DefaultEncoding(0),
            Action::DefaultEncoding(6),
            Action::CopyRuler,
            Action::PasteRuler,
            Action::Direction(false),
            Action::Direction(true),
            Action::Rewrap,
            Action::DarkBackground,
            Action::TextReplacement,
            Action::DefaultFont(1),
            Action::LineDone,
            Action::LineCancel,
            Action::ShowSpelling,
            Action::SpellingIgnore,
            Action::SpellingChange,
            Action::SpellingDone,
            Action::ShowSettings,
            Action::SettingsDone,
            Action::Setting(0),
            Action::Setting(14),
            Action::ShowSpacing,
            Action::SpacingDone,
            Action::SpacingCancel,
            Action::ShowEncodings,
            Action::EncodingsDone,
            Action::EncodingToggle(6),
            Action::FontStyle(0),
            Action::FontStyle(3),
            Action::Size(0),
            Action::Size(8),
            Action::ShowFonts,
            Action::ShowColors,
            Action::FontsDone,
            Action::ColorsDone,
        ];
        let mut ids: Vec<u32> = all.iter().filter_map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "every action has its own menu id");
        for a in all {
            let id = a.id().expect("own action has an id");
            assert_eq!(Action::from_id(id), Some(a));
        }
        // every family index has an id of its own, after all the other items' ids
        let others: Vec<u32> = all.iter().filter(|a| !matches!(a, Action::Font(_) | Action::DefaultFont(_))).filter_map(|a| a.id()).collect();
        for i in [0u8, 3, 8, 9, 100, 255] {
            let id = Action::Font(i).id().unwrap();
            assert_eq!(Action::from_id(id), Some(Action::Font(i)));
            assert!(others.iter().all(|other| *other < id), "family {i}");
            // Preferences > Default Font has an id for every family as well, after the families'
            let default = Action::DefaultFont(i).id().unwrap();
            assert_eq!(Action::from_id(default), Some(Action::DefaultFont(i)));
            assert!(default > Action::Font(255).id().unwrap(), "default font {i}");
        }
        assert_eq!(Action::from_id(MENU_BASE + DEFAULT_FONT_IDS_END + 1), None);
        assert_eq!(Action::Document(item::NEW).id(), None, "document commands use the controller's ids");
        assert_eq!(Action::from_id(item::SAVE), None);
    }

    #[test]
    fn the_geometry_follows_the_window_width() {
        let wide = geometry(1200.0, 780.0, false, true);
        assert!(wide.text_w <= READABLE_MAX_PT);
        assert!((wide.text_x * 2.0 + wide.text_w - 1200.0).abs() < 0.01, "the column is centred");
        let narrow = geometry(300.0, 400.0, true, false);
        assert_eq!(narrow.text_w, 300.0 - 2.0 * MARGIN_PT);
        assert_eq!(narrow.body_top, UNIFIED_BAR_PT + FORMAT_H + FIND_H);
        assert!(narrow.body_bottom <= 400.0 - STATUS_H);
    }

    #[test]
    fn a_closed_find_bar_takes_no_room() {
        assert_eq!(geometry(800.0, 600.0, false, false).body_top, UNIFIED_BAR_PT + FORMAT_H);
        assert_eq!(geometry(800.0, 600.0, false, true).body_top, UNIFIED_BAR_PT + FORMAT_H + RULER_H);
    }

    #[test]
    fn a_closed_find_bar_takes_no_presses_from_the_page() {
        // the find fields are laid out where the first line of the page is while the bar is closed: a press on that line
        // must reach the text (it once set the keyboard focus to a hidden field, so clicks never moved the caret)
        let closed = geometry(1200.0, 780.0, false, false);
        let (x, y) = (closed.find_field.x + 5.0, closed.find_field.y + 5.0);
        assert!(closed.find_field.contains(x, y) && y >= closed.body_top, "the hidden field overlaps the page");
        assert_eq!(hit(&[], &closed, x, y), None);
        assert_eq!(hit(&[], &closed, closed.replace_field.x + 5.0, closed.replace_field.y + 5.0), None);
        let open = geometry(1200.0, 780.0, true, false);
        assert_eq!(hit(&[], &open, open.find_field.x + 5.0, open.find_field.y + 5.0), Some(Hit::Field(Focus::Find)));
        assert_eq!(hit(&[], &open, open.replace_field.x + 5.0, open.replace_field.y + 5.0), Some(Hit::Field(Focus::Replace)));
    }

    #[test]
    fn the_format_bar_is_a_capsule_tall_and_the_page_starts_below_it() {
        let g = geometry(900.0, 700.0, false, false);
        assert!(FORMAT_H >= bifrost_app::widgets::CAPSULE_PT, "the toolbar capsules fit the bar");
        assert_eq!(g.ruler_top, UNIFIED_BAR_PT + FORMAT_H);
        assert_eq!(g.body_top, g.ruler_top, "no ruler and no find bar");
    }

    #[test]
    fn the_ruler_snaps_a_click_to_six_points() {
        let g = geometry(800.0, 600.0, false, true);
        assert_eq!(ruler_tab(&g, g.text_x + 38.0, g.ruler_top + 2.0), Some(36.0));
        assert_eq!(ruler_tab(&g, g.text_x + 38.0, g.body_top + 2.0), None, "the page is not the ruler");
        assert_eq!(ruler_tab(&geometry(800.0, 600.0, false, false), 100.0, 60.0), None);
    }

    #[test]
    fn spacing_labels_read_as_multiples() {
        assert_eq!(spacing_label(0), "1.0");
        assert_eq!(spacing_label(1), "1.15");
        assert_eq!(spacing_label(4), "2.5");
    }

    #[test]
    fn an_attachment_is_as_tall_as_its_line_and_narrows_to_fit_the_text() {
        let style = Style { attachment: 1, size: 40.0, ..Style::default() };
        // a 2:1 picture on a 50 point line is 100 wide, but a 60 point text width narrows it to 60 by 30
        assert_eq!(object_box(&[2.0], &style, 0.0), (100.0, 50.0));
        assert_eq!(object_box(&[2.0], &style, 60.0), (60.0, 30.0));
        // a character is its own box, not an attachment's
        assert_eq!(object_box(&[], &Style::default(), 0.0).1, Style::default().size * layout::LINE_FACTOR);
    }
}
