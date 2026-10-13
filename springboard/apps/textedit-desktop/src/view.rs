//! What Text Editor draws and where it can be touched: the unified toolbar, the find bar, the page with its text, the
//! caret and selection, and the word count. Positions are in points derived from the window size; the canvas is
//! reached in device pixels through `Ui::pt`. Colours and type come from the shared tokens only.

use crate::layout::{self, Layout, Line, Marker};
use crate::text::{Align, Cell, ListKind, Style, Transform, COLORS, FAMILIES, SPACINGS};
use bifrost_app::controls::Rect;
use bifrost_app::document::item;
use bifrost_app::kit::canvas::{Canvas, ImageRef, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, CONTENT_BG, DETAIL_BG, FIELD_BG, LABEL, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, SELECTION,
    SEGMENT_ON, SEPARATOR_C,
};
use bifrost_app::kit::{pack, Image};
use bifrost_app::layout::{traffic_lights_trailing, MARGIN_PT, READABLE_MAX_PT, UNIFIED_BAR_PT};

/// Height of the format bar under the toolbar, and of the tab ruler under it.
pub const FORMAT_H: f32 = 36.0;
pub const RULER_H: f32 = 22.0;
/// Size of a colour swatch button.
const SWATCH: f32 = 18.0;
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
const CONTROL_RADIUS: f32 = 6.0;
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
    SpellAuto,
    /// Print: page header and footer
    HeaderFooter,
    /// Preferences: whether new documents are plain text (true) or rich text (false)
    NewPlain(bool),
}

/// The first menu id of Text Editor's own items. Ids are stable: they are the values a menu sends back.
const MENU_BASE: u32 = 0x4E00;

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
            Action::Font(i) => MENU_BASE + 0x20 + i as u32,
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
            Action::SpellAuto => MENU_BASE + 0x88,
            Action::HeaderFooter => MENU_BASE + 0x89,
            Action::NewPlain(false) => MENU_BASE + 0x8a,
            Action::NewPlain(true) => MENU_BASE + 0x8b,
            Action::Highlight(i) => MENU_BASE + 0x90 + i as u32,
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
            0x20..=0x27 => Action::Font((offset - 0x20) as u8),
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
            0x88 => Action::SpellAuto,
            0x89 => Action::HeaderFooter,
            0x8a => Action::NewPlain(false),
            0x8b => Action::NewPlain(true),
            0x90..=0x97 => Action::Highlight((offset - 0x90) as u8),
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
    /// A colour swatch instead of a label: the colour it fills with.
    pub swatch: Option<u32>,
}

/// Which text field has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Body,
    Find,
    Replace,
}

/// What the chrome shows about the document and the find bar.
pub struct Status {
    pub style: Style,
    pub align: Align,
    pub rich: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub find_open: bool,
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
        text_x,
        text_w,
        find_field: Rect { x: x0, y: row_y, w: FIELD_W, h: BUTTON_H },
        replace_field: Rect { x: x0 + FIELD_W + ITEM_GAP, y: row_y, w: FIELD_W, h: BUTTON_H },
    }
}

/// The width of one character in a style, in points.
pub fn char_width(ui: &Ui, ch: char, style: &Style) -> f32 {
    let mut buf = [0u8; 4];
    ui.measure(ch.encode_utf8(&mut buf), drawn_size(style), weight_of(style)) / ui.s
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
    FAMILIES.get(font as usize).copied().unwrap_or(FAMILIES[0])
}

fn label_button(ui: &Ui, label: &str, action: Option<Action>, weight: Weight) -> Button {
    let width = ui.measure(label, MAC_BODY_PT, weight) / ui.s + 2.0 * BUTTON_PAD;
    Button { rect: Rect { x: 0.0, y: 0.0, w: width, h: BUTTON_H }, label: label.to_string(), action, active: false, enabled: true, weight, swatch: None }
}

fn swatch_button(color: u32, action: Action) -> Button {
    Button {
        rect: Rect { x: 0.0, y: 0.0, w: SWATCH + 6.0, h: BUTTON_H },
        label: String::new(),
        action: Some(action),
        active: false,
        enabled: true,
        weight: Weight::Regular,
        swatch: Some(color),
    }
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

/// The toolbar's buttons, in the unified title row beside the traffic lights.
pub fn toolbar(ui: &Ui, status: &Status) -> Vec<Button> {
    let weight = Weight::Regular;
    let mut buttons = vec![
        label_button(ui, "New", Some(Action::Document(item::NEW)), weight),
        label_button(ui, "Open", Some(Action::Document(item::OPEN)), weight),
        label_button(ui, "Save", Some(Action::Document(item::SAVE)), weight),
        label_button(ui, "Undo", Some(Action::Undo), weight),
        label_button(ui, "Redo", Some(Action::Redo), weight),
        label_button(ui, "Find", Some(Action::Find), weight),
        label_button(ui, "Print", Some(Action::Print), weight),
    ];
    buttons[3].enabled = status.can_undo;
    buttons[4].enabled = status.can_redo;
    buttons[5].active = status.find_open;
    let groups: [usize; 4] = [3, 2, 1, 1];
    place_row(&mut buttons, &groups, traffic_lights_trailing() + BAR_GAP, (UNIFIED_BAR_PT - BUTTON_H) / 2.0);
    buttons
}

/// The format bar's buttons: character style, size, colour, alignment, line spacing, lists and the text mode. A plain
/// text document has only the mode switch enabled.
pub fn format_bar(ui: &Ui, status: &Status, geom: &Geometry) -> Vec<Button> {
    let weight = Weight::Regular;
    let spacing_next = (status.style.spacing + 1) % SPACINGS.len() as u8;
    let mut buttons = vec![
        label_button(ui, "B", Some(Action::Bold), Weight::Bold),
        label_button(ui, "I", Some(Action::Italic), weight),
        label_button(ui, "U", Some(Action::Underline), weight),
        label_button(ui, "A-", Some(Action::Smaller), weight),
        label_button(ui, &format!("{:.0} pt", status.style.size), None, weight),
        label_button(ui, "A+", Some(Action::Bigger), weight),
    ];
    let swatches = COLORS.len();
    for i in 0..swatches {
        let mut b = swatch_button(text_color(i as u8), Action::Color(i as u8));
        b.active = status.style.color as usize == i;
        buttons.push(b);
    }
    buttons.extend([
        label_button(ui, "Left", Some(Action::Align(Align::Left)), weight),
        label_button(ui, "Center", Some(Action::Align(Align::Center)), weight),
        label_button(ui, "Right", Some(Action::Align(Align::Right)), weight),
        label_button(ui, "Justify", Some(Action::Align(Align::Justified)), weight),
        label_button(ui, &format!("Spacing {}", spacing_label(status.style.spacing)), Some(Action::Spacing(spacing_next)), weight),
        label_button(ui, "Bullets", Some(Action::List(ListKind::Bullet)), weight),
        label_button(ui, "Numbers", Some(Action::List(ListKind::Numbered)), weight),
        label_button(ui, if status.rich { "Rich Text" } else { "Plain Text" }, Some(if status.rich { Action::MakePlain } else { Action::MakeRich }), weight),
    ]);
    let n = buttons.len();
    for b in &mut buttons[..n - 1] {
        b.enabled = status.rich;
    }
    buttons[0].active = status.style.bold;
    buttons[1].active = status.style.italic;
    buttons[2].active = status.style.underline;
    let a = 6 + swatches;
    for b in &mut buttons[a..a + 4] {
        if let Some(Action::Align(al)) = b.action {
            b.active = al == status.align;
        }
    }
    buttons[a + 5].active = status.style.list == ListKind::Bullet;
    buttons[a + 6].active = status.style.list == ListKind::Numbered;
    let groups = [3, 3, swatches, 4, 1, 2, 1];
    let y = geom.format_top + (FORMAT_H - BUTTON_H) / 2.0;
    place_row(&mut buttons, &groups, BAR_GAP + 4.0, y);
    buttons
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

/// The button or field under a point, in points.
pub fn hit(buttons: &[Button], geom: &Geometry, x: f32, y: f32) -> Option<Hit> {
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
fn blit(c: &mut dyn Canvas, img: &Image, x: f32, y: f32) {
    c.image(ImageRef { id: image_id(img), image: img }, RectF::new(x.floor(), y.floor(), img.w as f32, img.h as f32), 1.0);
}

/// Draws a text run with its top-left at `(x, y)` points.
fn text_at(c: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) -> f32 {
    let img = ui.text(text, pt, weight, color);
    let width = img.w as f32 / ui.s;
    blit(c, &img, ui.pt(x), ui.pt(y));
    width
}

/// Draws the whole window background and the toolbar band.
pub fn draw_bar(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry) {
    c.fill_rect(RectF::new(0.0, 0.0, ui.pt(geom.width).ceil(), ui.pt(geom.height).ceil()), DETAIL_BG.now());
    let sep = ui.pt(UNIFIED_BAR_PT).floor() - 1.0;
    c.fill_rect(RectF::new(0.0, sep.max(0.0), ui.pt(geom.width).ceil(), 1.0), SEPARATOR_C.now());
}

/// Draws a button: its plate when active, and its label centred.
pub fn draw_button(c: &mut dyn Canvas, ui: &Ui, b: &Button) {
    let r = to_px(ui, b.rect);
    if b.active {
        c.fill_rrect(r, ui.pt(CONTROL_RADIUS), SEGMENT_ON.now());
    }
    if let Some(fill) = b.swatch {
        let side = ui.pt(SWATCH);
        let sw = RectF::new(r.x + (r.w - side) / 2.0, r.y + (r.h - side) / 2.0, side, side);
        c.fill_rrect(sw, side / 2.0, if b.enabled { fill } else { LABEL_3.now() });
        return;
    }
    let color = if b.enabled { LABEL.now() } else { LABEL_3.now() };
    let img = ui.text(&b.label, MAC_BODY_PT, b.weight, color);
    let x = r.x + (r.w - img.w as f32) / 2.0;
    let y = r.y + (r.h - img.h as f32) / 2.0;
    blit(c, &img, x, y);
}

/// Draws the find bar: its two fields, their text, and its buttons.
pub fn draw_find(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, buttons: &[Button], find: &str, replace: &str, focus: Focus) {
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
        let (shown, color) = if text.is_empty() { (placeholder, LABEL_3.now()) } else { (text, LABEL.now()) };
        let inner = ui.pt(rect.w) - 2.0 * ui.pt(BUTTON_PAD);
        let fitted = ui.fit(shown, MAC_BODY_PT, inner.max(0.0));
        let img = ui.text(&fitted, MAC_BODY_PT, Weight::Regular, color);
        blit(c, &img, r.x + ui.pt(BUTTON_PAD), r.y + (r.h - img.h as f32) / 2.0);
    }
    for b in buttons {
        draw_button(c, ui, b);
    }
}

/// The page: its background and a clip to the body so text never draws over the bars.
pub fn page_clip(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry) -> RectF {
    let r = RectF::new(0.0, ui.pt(geom.body_top).floor(), ui.pt(geom.width).ceil(), ui.pt(geom.body_bottom - geom.body_top).ceil());
    c.fill_rect(r, CONTENT_BG.now());
    c.push_clip(r, 0.0);
    r
}

/// Draws the selection, the text and the caret at `head`. `scroll` is how far the body is scrolled, in points.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, lines: &Layout, cells: &[Cell], sel: std::ops::Range<usize>, head: usize, scroll: f32, caret: bool, misspelled: &[std::ops::Range<usize>]) {
    let origin_x = geom.text_x;
    let origin_y = geom.body_top - scroll;
    let measure = |ch: char, style: &Style| char_width(ui, ch, style);
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
        draw_line(c, ui, lines, line, cells, origin_x, origin_y);
        for word in misspelled.iter().filter(|w| w.start < line.end && w.end > line.start) {
            let x0 = lines.x_in_line(cells, line, word.start.max(line.start), &measure);
            let x1 = lines.x_in_line(cells, line, word.end.min(line.end), &measure);
            squiggle(c, ui, origin_x + x0, origin_x + x1, origin_y + line.top + line.height - 2.0);
        }
    }
    if caret {
        let (x, top, height) = lines.caret(cells, head, &measure);
        let rect = Rect { x: origin_x + x, y: origin_y + top, w: 1.0, h: height };
        c.fill_rect(to_px(ui, rect), LABEL.now());
    }
}

/// A wavy rule from `x0` to `x1` (points) at `y`: short segments alternating up and down, in the destructive colour.
fn squiggle(c: &mut dyn Canvas, ui: &Ui, x0: f32, x1: f32, y: f32) {
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        let w = 2.0f32.min(x1 - x);
        let rect = Rect { x, y: if up { y } else { y + 1.0 }, w, h: 1.0 };
        c.fill_rect(to_px(ui, rect), pack(255, 69, 58, 255));
        x += w;
        up = !up;
    }
}

fn draw_line(c: &mut dyn Canvas, ui: &Ui, lines: &Layout, line: &Line, cells: &[Cell], origin_x: f32, origin_y: f32) {
    let measure = |ch: char, style: &Style| char_width(ui, ch, style);
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
        text_at(c, ui, &m, size, Weight::Regular, LABEL.now(), origin_x + line.indent - 6.0 - w, y);
    }
    let base = origin_x + lines.x_offset(line);
    let mut rel = 0.0;
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    let mut run_x = 0.0;
    for cell in &cells[line.start..line.end] {
        let step = layout::advance(rel, cell, lines.tab, &measure);
        let breaks = cell.ch == '\t' || run_style.is_some_and(|s| !s.same_run(&cell.style));
        if breaks {
            if let Some(s) = run_style.take() {
                flush_run(c, ui, &run, &s, base + run_x, y, line.height);
            }
            run.clear();
        }
        if cell.ch != '\t' {
            if run_style.is_none() {
                run_style = Some(cell.style);
                run_x = rel;
            }
            run.push(cell.ch);
        }
        rel += step;
    }
    if let Some(s) = run_style {
        flush_run(c, ui, &run, &s, base + run_x, y, line.height);
    }
}

/// Draws one run and returns the x after it. Underlines and strikethrough are a rule through the run, a highlight is
/// a plate behind it, and raised or lowered text is smaller and shifted.
fn flush_run(c: &mut dyn Canvas, ui: &Ui, text: &str, style: &Style, x: f32, y: f32, line_height: f32) -> f32 {
    if text.is_empty() {
        return x;
    }
    let shown: String = text.chars().filter(|&ch| ch != '\n').collect();
    if shown.is_empty() {
        return x;
    }
    // text on a highlight is dark whatever the appearance, as the plate is light
    let color = if style.highlight > 0 && style.color == 0 { pack(0, 0, 0, 255) } else { text_color(style.color) };
    let size = drawn_size(style);
    let shift = match style.script {
        1 => 0.0,
        -1 => style.size * 0.3,
        _ => 0.0,
    };
    let width = ui.measure(&shown, size, weight_of(style)) / ui.s;
    if style.highlight > 0 {
        let plate = Rect { x, y, w: width, h: line_height };
        c.fill_rect(to_px(ui, plate), highlight_color(style.highlight));
    }
    text_at(c, ui, &shown, size, weight_of(style), color, x, y + shift);
    if style.underline {
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
            Action::SpellAuto,
            Action::HeaderFooter,
            Action::NewPlain(false),
            Action::NewPlain(true),
        ];
        let mut ids: Vec<u32> = all.iter().filter_map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "every action has its own menu id");
        for a in all {
            let id = a.id().expect("own action has an id");
            assert_eq!(Action::from_id(id), Some(a));
        }
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
}
