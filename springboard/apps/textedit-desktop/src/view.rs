//! What Text Editor draws and where it can be touched: the unified toolbar, the find bar, the page with its text, the
//! caret and selection, and the word count. Positions are in points derived from the window size; the canvas is
//! reached in device pixels through `Ui::pt`. Colours and type come from the shared tokens only.

use crate::layout::{Layout, Line};
use crate::text::{Align, Cell, Style, FAMILIES};
use bifrost_app::controls::Rect;
use bifrost_app::document::item;
use bifrost_app::kit::canvas::{Canvas, ImageRef, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, CONTENT_BG, DETAIL_BG, FIELD_BG, LABEL, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, SELECTION,
    SEGMENT_ON, SEPARATOR_C,
};
use bifrost_app::kit::Image;
use bifrost_app::layout::{traffic_lights_trailing, MARGIN_PT, READABLE_MAX_PT, UNIFIED_BAR_PT};

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

/// What the chrome shows about the document and the find bar.
pub struct Status {
    pub style: Style,
    pub align: Align,
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
    /// The column of text: its left edge and width, centred and never wider than the readable width.
    pub text_x: f32,
    pub text_w: f32,
    /// The find fields, when the bar is open.
    pub find_field: Rect,
    pub replace_field: Rect,
}

pub fn geometry(width: f32, height: f32, find_open: bool) -> Geometry {
    let bar = UNIFIED_BAR_PT;
    let body_top = bar + if find_open { FIND_H } else { 0.0 };
    let body_bottom = (height - STATUS_H).max(body_top);
    let text_w = (width - 2.0 * MARGIN_PT).min(READABLE_MAX_PT).max(0.0);
    let text_x = ((width - text_w) / 2.0).max(0.0);
    let x0 = traffic_lights_trailing();
    let row_y = bar + (FIND_H - BUTTON_H) / 2.0;
    Geometry {
        width,
        height,
        body_top,
        body_bottom,
        text_x,
        text_w,
        find_field: Rect { x: x0, y: row_y, w: FIELD_W, h: BUTTON_H },
        replace_field: Rect { x: x0 + FIELD_W + ITEM_GAP, y: row_y, w: FIELD_W, h: BUTTON_H },
    }
}

/// The width of one character in a style, in points.
pub fn char_width(ui: &Ui, ch: char, style: &Style) -> f32 {
    let mut buf = [0u8; 4];
    ui.measure(ch.encode_utf8(&mut buf), style.size, weight_of(style)) / ui.s
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
    Button { rect: Rect { x: 0.0, y: 0.0, w: width, h: BUTTON_H }, label: label.to_string(), action, active: false, enabled: true, weight }
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
        label_button(ui, "B", Some(Action::Bold), Weight::Bold),
        label_button(ui, "I", Some(Action::Italic), Weight::Regular),
        label_button(ui, "U", Some(Action::Underline), Weight::Regular),
        label_button(ui, "A-", Some(Action::Smaller), weight),
        label_button(ui, &format!("{:.0} pt", status.style.size), None, weight),
        label_button(ui, "A+", Some(Action::Bigger), weight),
        label_button(ui, "Left", Some(Action::Align(Align::Left)), weight),
        label_button(ui, "Center", Some(Action::Align(Align::Center)), weight),
        label_button(ui, "Right", Some(Action::Align(Align::Right)), weight),
        label_button(ui, "Justify", Some(Action::Align(Align::Justified)), weight),
        label_button(ui, "Find", Some(Action::Find), weight),
    ];
    buttons[3].enabled = status.can_undo;
    buttons[4].enabled = status.can_redo;
    buttons[5].active = status.style.bold;
    buttons[6].active = status.style.italic;
    buttons[7].active = status.style.underline;
    for b in &mut buttons[11..15] {
        let align = match b.action {
            Some(Action::Align(a)) => a,
            _ => Align::Left,
        };
        b.active = align == status.align;
    }
    buttons[15].active = status.find_open;
    let groups: [usize; 6] = [3, 2, 3, 3, 4, 1];
    place_row(&mut buttons, &groups, traffic_lights_trailing() + BAR_GAP, (UNIFIED_BAR_PT - BUTTON_H) / 2.0);
    buttons
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
    let color = if b.enabled { LABEL.now() } else { LABEL_3.now() };
    let img = ui.text(&b.label, MAC_BODY_PT, b.weight, color);
    let x = r.x + (r.w - img.w as f32) / 2.0;
    let y = r.y + (r.h - img.h as f32) / 2.0;
    blit(c, &img, x, y);
}

/// Draws the find bar: its two fields, their text, and its buttons.
pub fn draw_find(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, buttons: &[Button], find: &str, replace: &str, focus: Focus) {
    let top = UNIFIED_BAR_PT;
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
pub fn draw_text(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, lines: &Layout, cells: &[Cell], sel: std::ops::Range<usize>, head: usize, scroll: f32, caret: bool) {
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
    }
    if caret {
        let (x, top, height) = lines.caret(cells, head, &measure);
        let rect = Rect { x: origin_x + x, y: origin_y + top, w: 1.0, h: height };
        c.fill_rect(to_px(ui, rect), LABEL.now());
    }
}

fn draw_line(c: &mut dyn Canvas, ui: &Ui, lines: &Layout, line: &Line, cells: &[Cell], origin_x: f32, origin_y: f32) {
    let mut x = origin_x + lines.x_offset(line);
    let y = origin_y + line.top;
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    let mut run_x = x;
    for cell in &cells[line.start..line.end] {
        let same = run_style.is_some_and(|s| same_run(&s, &cell.style));
        if !same {
            if let Some(s) = run_style {
                x = flush_run(c, ui, &run, &s, run_x, y, line.height);
            }
            run.clear();
            run_style = Some(cell.style);
            run_x = x;
        }
        run.push(cell.ch);
    }
    if let Some(s) = run_style {
        flush_run(c, ui, &run, &s, run_x, y, line.height);
    }
}

/// Two characters share a run when they draw alike.
fn same_run(a: &Style, b: &Style) -> bool {
    a.bold == b.bold && a.underline == b.underline && a.size == b.size && a.font == b.font
}

/// Draws one run and returns the x after it. Underlines are a rule under the run.
fn flush_run(c: &mut dyn Canvas, ui: &Ui, text: &str, style: &Style, x: f32, y: f32, line_height: f32) -> f32 {
    if text.is_empty() {
        return x;
    }
    let shown: String = text.chars().filter(|&ch| ch != '\n').collect();
    if shown.is_empty() {
        return x;
    }
    let width = text_at(c, ui, &shown, style.size, weight_of(style), LABEL.now(), x, y);
    if style.underline {
        let rule = Rect { x, y: y + line_height - 2.0, w: width, h: 1.0 };
        c.fill_rect(to_px(ui, rule), LABEL.now());
    }
    x + width
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
        ];
        for a in all {
            let id = a.id().expect("own action has an id");
            assert_eq!(Action::from_id(id), Some(a));
        }
        assert_eq!(Action::Document(item::NEW).id(), None, "document commands use the controller's ids");
        assert_eq!(Action::from_id(item::SAVE), None);
    }

    #[test]
    fn the_geometry_follows_the_window_width() {
        let wide = geometry(1200.0, 780.0, false);
        assert!(wide.text_w <= READABLE_MAX_PT);
        assert!((wide.text_x * 2.0 + wide.text_w - 1200.0).abs() < 0.01, "the column is centred");
        let narrow = geometry(300.0, 400.0, true);
        assert_eq!(narrow.text_w, 300.0 - 2.0 * MARGIN_PT);
        assert_eq!(narrow.body_top, UNIFIED_BAR_PT + FIND_H);
        assert!(narrow.body_bottom <= 400.0 - STATUS_H);
    }

    #[test]
    fn a_closed_find_bar_takes_no_room() {
        assert_eq!(geometry(800.0, 600.0, false).body_top, UNIFIED_BAR_PT);
    }
}
