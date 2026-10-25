//! The format bar under the title row: one unified toolbar of grouped controls built from the kit. A capsule of the
//! character styles (bold, italic, underline, strikethrough), the text colour well, the font family, style and size
//! pop-ups, a capsule of the four alignments, the list pull-down and the line spacing with its stepper. The kit lays the
//! items out; whatever does not fit the window moves behind the `>>` button, whose menu lists what is hidden.
//!
//! This file is the model (what the bar has, what a press does, the menu rows) and the drawing of the slots the app
//! draws itself. The window (`main.rs`) hit-tests with `hit`, asks `press` what to do and opens the menus.

use crate::text::{families, Align, ListKind, COLORS, SIZES, SPACINGS};
use crate::view::{blit, family_name, spacing_label, text_color, Action, Geometry, Status, FORMAT_H};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sheet::{stepper_hit, stepper_rect, StepperPart};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{Ui, Weight, HAIRLINE_C, LABEL, LABEL_3, MAC_BODY_PT};
use bifrost_app::layout::MARGIN_PT;
use bifrost_app::widgets::{self, CAPSULE_PT};

/// The ids of the bar's items. They are also the low bits of the controls the window publishes.
pub const BOLD: u32 = 1;
pub const ITALIC: u32 = 2;
pub const UNDERLINE: u32 = 3;
pub const STRIKE: u32 = 4;
pub const COLOR: u32 = 5;
pub const FONT: u32 = 6;
pub const STYLE: u32 = 7;
pub const SIZE: u32 = 8;
pub const LEFT: u32 = 9;
pub const CENTER: u32 = 10;
pub const RIGHT: u32 = 11;
pub const JUSTIFY: u32 = 12;
pub const LIST: u32 = 13;
pub const SPACING: u32 = 14;

/// Every item of the bar, left to right.
pub const ALL: [u32; 14] = [BOLD, ITALIC, UNDERLINE, STRIKE, COLOR, FONT, STYLE, SIZE, LEFT, CENTER, RIGHT, JUSTIFY, LIST, SPACING];

/// The id of the `>>` button among the published controls (the items' ids are below it).
pub const OVERFLOW: u32 = 99;

/// The widths of the slots the app draws, in points.
const COLOR_W: f32 = 40.0;
const FONT_W: f32 = 128.0;
const STYLE_W: f32 = 104.0;
const SIZE_W: f32 = 68.0;
const SPACING_W: f32 = 84.0;
/// The height of a pop-up button (`widgets::popup_button`'s standard) and the side of the colour well.
const POPUP_H: f32 = 22.0;
const WELL_PT: f32 = 22.0;
/// The gap between a button and the menu it opens, and the most rows a menu may have.
const MENU_GAP: f32 = 2.0;
const MAX_MENU_ROWS: usize = 32;

/// A row of a menu the bar opens: its title and what choosing it does (`None` for a separator).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub title: String,
    pub action: Option<Action>,
}

impl Row {
    fn new(title: &str, action: Action) -> Row {
        Row { title: title.to_string(), action: Some(action) }
    }

    fn separator() -> Row {
        Row { title: String::new(), action: None }
    }
}

/// What a press on the bar asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// Do this now.
    Do(Action),
    /// Open a menu with its top-left corner at `anchor` (points).
    Menu { anchor: (f32, f32), rows: Vec<Row> },
}

/// What is under a point of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Item(u32),
    Overflow,
}

/// The label of the style pop-up for a style: the face it has.
pub fn face_label(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

/// A size as the size pop-up shows it: whole sizes without a fraction.
pub fn size_text(size: f32) -> String {
    if size.fract() == 0.0 {
        format!("{size:.0}")
    } else {
        format!("{size:.1}")
    }
}

/// The accessibility label of an item.
pub fn label_of(id: u32) -> &'static str {
    match id {
        BOLD => "Bold",
        ITALIC => "Italic",
        UNDERLINE => "Underline",
        STRIKE => "Strikethrough",
        COLOR => "Text Color",
        FONT => "Font",
        STYLE => "Font Style",
        SIZE => "Font Size",
        LEFT => "Align Left",
        CENTER => "Center",
        RIGHT => "Align Right",
        JUSTIFY => "Justify",
        LIST => "List",
        SPACING => "Line Spacing",
        OVERFLOW => "More Formatting",
        _ => "",
    }
}

/// The bar as a kit toolbar for the document's state. A plain text document has no formatting to change, so every item
/// is dimmed.
pub fn toolbar(status: &Status) -> Toolbar {
    let style = &status.style;
    let rich = status.rich;
    let list_symbol = if style.list == ListKind::Numbered { "list.number" } else { "list.bullet" };
    Toolbar::new(vec![
        ToolbarItem::icon(BOLD, label_of(BOLD), "bold").in_group(1).selected(style.bold).enabled(rich),
        ToolbarItem::icon(ITALIC, label_of(ITALIC), "italic").in_group(1).selected(style.italic).enabled(rich),
        ToolbarItem::icon(UNDERLINE, label_of(UNDERLINE), "underline").in_group(1).selected(style.underline).enabled(rich),
        ToolbarItem::icon(STRIKE, label_of(STRIKE), "strikethrough").in_group(1).selected(style.strike).enabled(rich),
        ToolbarItem::custom(COLOR, label_of(COLOR), COLOR_W).enabled(rich),
        ToolbarItem::custom(FONT, label_of(FONT), FONT_W).enabled(rich),
        ToolbarItem::custom(STYLE, label_of(STYLE), STYLE_W).enabled(rich),
        ToolbarItem::custom(SIZE, label_of(SIZE), SIZE_W).enabled(rich),
        ToolbarItem::icon(LEFT, label_of(LEFT), "text.alignleft").in_group(2).selected(status.align == Align::Left).enabled(rich),
        ToolbarItem::icon(CENTER, label_of(CENTER), "text.aligncenter").in_group(2).selected(status.align == Align::Center).enabled(rich),
        ToolbarItem::icon(RIGHT, label_of(RIGHT), "text.alignright").in_group(2).selected(status.align == Align::Right).enabled(rich),
        ToolbarItem::icon(JUSTIFY, label_of(JUSTIFY), "text.justify").in_group(2).selected(status.align == Align::Justified).enabled(rich),
        ToolbarItem::menu(LIST, label_of(LIST), Some(list_symbol)).selected(style.list != ListKind::None).enabled(rich),
        ToolbarItem::custom(SPACING, label_of(SPACING), SPACING_W).enabled(rich),
    ])
}

/// Where the bar sits: the window's width less the margins, one capsule tall, in the middle of its band.
pub fn bounds(geom: &Geometry) -> Rect {
    Rect { x: MARGIN_PT, y: geom.format_top + (FORMAT_H - CAPSULE_PT) / 2.0, w: (geom.width - 2.0 * MARGIN_PT).max(0.0), h: CAPSULE_PT }
}

/// The bar's items: none when the window shows no format bar (a plain text document, `Geometry::format_h` 0).
fn shown_toolbar(geom: &Geometry, status: &Status) -> Toolbar {
    if geom.format_h > 0.0 { toolbar(status) } else { Toolbar::new(Vec::new()) }
}

/// Where the kit puts each item for this window and document. A window with no format bar has no items, so nothing is
/// hit, published or drawn there.
pub fn layout(ui: &Ui, geom: &Geometry, status: &Status) -> ToolbarLayout {
    widgets::toolbar_layout(ui, bounds(geom), &shown_toolbar(geom, status))
}

/// The items the window is too narrow for: they are behind the `>>` button.
pub fn hidden(layout: &ToolbarLayout) -> Vec<u32> {
    ALL.iter().copied().filter(|id| layout.rect_of(*id).is_none()).collect()
}

/// What is under a point (points): an item on the bar, or the `>>` button.
pub fn hit(layout: &ToolbarLayout, x: f32, y: f32) -> Option<Hit> {
    if layout.overflow_button.is_some_and(|r| r.contains(x, y)) {
        return Some(Hit::Overflow);
    }
    layout.hit(x, y).map(Hit::Item)
}

/// The font family menu: every family a document can use (`text::families`, the base ones and the installed ones).
pub fn font_rows() -> Vec<Row> {
    family_rows(families().len())
}

/// The rows for the first `count` families. A menu holds at most `MAX_MENU_ROWS` rows (the context menu's wire
/// limit), so past that the last rows are a separator and Show Fonts, whose panel lists every family.
fn family_rows(count: usize) -> Vec<Row> {
    let count = count.min(256);
    let row = |i: usize| Row::new(family_name(i as u8), Action::Font(i as u8));
    if count <= MAX_MENU_ROWS {
        return (0..count).map(row).collect();
    }
    let mut rows: Vec<Row> = (0..MAX_MENU_ROWS - 2).map(row).collect();
    rows.push(Row::separator());
    rows.push(Row::new("Show Fonts\u{2026}", Action::ShowFonts));
    rows
}

/// The style menu: the four faces.
pub fn style_rows() -> Vec<Row> {
    (0..4u8).map(|i| Row::new(face_label(i & 1 != 0, i & 2 != 0), Action::FontStyle(i))).collect()
}

/// The size menu: the sizes Bigger and Smaller step through.
pub fn size_rows() -> Vec<Row> {
    SIZES.iter().enumerate().map(|(i, size)| Row::new(&size_text(*size), Action::Size(i as u8))).collect()
}

/// The colour menu: the document's palette.
pub fn color_rows() -> Vec<Row> {
    COLORS.iter().enumerate().map(|(i, (name, _))| Row::new(name, Action::Color(i as u8))).collect()
}

/// The list menu. Choosing the kind the paragraph already has takes the list off, as the Format menu does.
pub fn list_rows() -> Vec<Row> {
    vec![Row::new("Bulleted List", Action::List(ListKind::Bullet)), Row::new("Numbered List", Action::List(ListKind::Numbered))]
}

/// The line spacing menu.
pub fn spacing_rows() -> Vec<Row> {
    (0..SPACINGS.len() as u8).map(|i| Row::new(&format!("Spacing {}", spacing_label(i)), Action::Spacing(i))).collect()
}

/// The menu behind the `>>` button: one group of rows for each hidden item (the character styles and the alignments
/// share a group), in the bar's order, at most what a context menu holds.
pub fn overflow_rows(hidden: &[u32]) -> Vec<Row> {
    let has = |id: u32| hidden.contains(&id);
    let pick = |items: &[(u32, &str, Action)]| -> Vec<Row> { items.iter().filter(|(id, _, _)| has(*id)).map(|(_, title, action)| Row::new(title, *action)).collect() };
    let mut groups = vec![pick(&[(BOLD, "Bold", Action::Bold), (ITALIC, "Italic", Action::Italic), (UNDERLINE, "Underline", Action::Underline), (STRIKE, "Strikethrough", Action::Strike)])];
    for (id, rows) in [(COLOR, color_rows as fn() -> Vec<Row>), (FONT, font_rows), (STYLE, style_rows), (SIZE, size_rows)] {
        groups.push(if has(id) { rows() } else { Vec::new() });
    }
    groups.push(pick(&[
        (LEFT, "Align Left", Action::Align(Align::Left)),
        (CENTER, "Center", Action::Align(Align::Center)),
        (RIGHT, "Align Right", Action::Align(Align::Right)),
        (JUSTIFY, "Justify", Action::Align(Align::Justified)),
    ]));
    for (id, rows) in [(LIST, list_rows as fn() -> Vec<Row>), (SPACING, spacing_rows)] {
        groups.push(if has(id) { rows() } else { Vec::new() });
    }
    let mut rows = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !rows.is_empty() {
            rows.push(Row::separator());
        }
        rows.extend(group);
    }
    rows.truncate(MAX_MENU_ROWS);
    rows
}

/// Where the line spacing's stepper is inside its slot: at the right edge, centred in the band.
fn stepper_origin(slot: Rect) -> (f32, f32) {
    let size = stepper_rect((0.0, 0.0));
    (slot.right() - size.w, slot.y + (slot.h - size.h) / 2.0)
}

/// What a press at (`x`, `y`) points on `hit` asks for. Nothing for a plain text document (the items are dimmed). The
/// line spacing's stepper steps the spacing by one; the rest of its slot opens the spacing menu.
pub fn press(status: &Status, layout: &ToolbarLayout, hit: Hit, x: f32, y: f32) -> Option<Press> {
    let menu = |id: u32, rows: Vec<Row>| layout.menu_anchor(id, MENU_GAP).map(|anchor| Press::Menu { anchor, rows });
    match hit {
        Hit::Overflow => {
            let r = layout.overflow_button?;
            Some(Press::Menu { anchor: (r.x, r.bottom() + MENU_GAP), rows: overflow_rows(&hidden(layout)) })
        }
        Hit::Item(id) => {
            if !status.rich {
                return None;
            }
            match id {
                BOLD => Some(Press::Do(Action::Bold)),
                ITALIC => Some(Press::Do(Action::Italic)),
                UNDERLINE => Some(Press::Do(Action::Underline)),
                STRIKE => Some(Press::Do(Action::Strike)),
                LEFT => Some(Press::Do(Action::Align(Align::Left))),
                CENTER => Some(Press::Do(Action::Align(Align::Center))),
                RIGHT => Some(Press::Do(Action::Align(Align::Right))),
                JUSTIFY => Some(Press::Do(Action::Align(Align::Justified))),
                COLOR => menu(id, color_rows()),
                FONT => menu(id, font_rows()),
                STYLE => menu(id, style_rows()),
                SIZE => menu(id, size_rows()),
                LIST => menu(id, list_rows()),
                SPACING => {
                    let slot = layout.rect_of(id)?;
                    let current = status.style.spacing;
                    match stepper_hit(stepper_origin(slot), x, y) {
                        Some(StepperPart::Up) => Some(Press::Do(Action::Spacing((current + 1).min(SPACINGS.len() as u8 - 1)))),
                        Some(StepperPart::Down) => Some(Press::Do(Action::Spacing(current.saturating_sub(1)))),
                        None => menu(id, spacing_rows()),
                    }
                }
                _ => None,
            }
        }
    }
}

/// The controls the bar publishes: every item on it and the `>>` button, with the state each shows.
pub fn nodes(layout: &ToolbarLayout, status: &Status) -> Vec<Node> {
    let style = &status.style;
    let mut out: Vec<Node> = layout
        .visible
        .iter()
        .map(|(id, rect)| {
            let selected = match *id {
                BOLD => style.bold,
                ITALIC => style.italic,
                UNDERLINE => style.underline,
                STRIKE => style.strike,
                LEFT => status.align == Align::Left,
                CENTER => status.align == Align::Center,
                RIGHT => status.align == Align::Right,
                JUSTIFY => status.align == Align::Justified,
                LIST => style.list != ListKind::None,
                _ => false,
            };
            let value = match *id {
                COLOR => COLORS.get(style.color as usize).map_or("", |c| c.0).to_string(),
                FONT => family_name(style.font).to_string(),
                STYLE => face_label(style.bold, style.italic).to_string(),
                SIZE => size_text(style.size),
                SPACING => spacing_label(style.spacing),
                _ if selected => "selected".to_string(),
                _ => String::new(),
            };
            Node { id: NODE_BASE + u64::from(*id), role: Role::Button, label: label_of(*id).to_string(), value, bounds: *rect, enabled: status.rich }
        })
        .collect();
    if let Some(rect) = layout.overflow_button {
        out.push(Node { id: NODE_BASE + u64::from(OVERFLOW), role: Role::Button, label: label_of(OVERFLOW).to_string(), value: String::new(), bounds: rect, enabled: true });
    }
    out
}

/// The first id of the controls the bar publishes.
pub const NODE_BASE: u64 = 0x5445_0000;

/// The item a published control's id stands for (`Input::Activate`): an item id, or `OVERFLOW`.
pub fn item_of_node(id: u64) -> Option<u32> {
    let item = id.checked_sub(NODE_BASE)?;
    let item = u32::try_from(item).ok()?;
    (ALL.contains(&item) || item == OVERFLOW).then_some(item)
}

fn popup_rect(slot: Rect) -> Rect {
    Rect { x: slot.x, y: slot.y + (slot.h - POPUP_H) / 2.0, w: slot.w, h: POPUP_H }
}

/// The colour well: a rounded square in the text colour at the caret.
fn draw_well(c: &mut dyn Canvas, ui: &Ui, slot: Rect, color: u32, enabled: bool) {
    let rect = RectF::new(ui.pt(slot.x + (slot.w - WELL_PT) / 2.0), ui.pt(slot.y + (slot.h - WELL_PT) / 2.0), ui.pt(WELL_PT), ui.pt(WELL_PT));
    c.fill_rrect(rect, ui.pt(m::ICON_RADIUS), if enabled { color } else { LABEL_3.now() });
    c.stroke_rrect(rect, ui.pt(m::ICON_RADIUS), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
}

/// Draws the bar: the kit's toolbar (capsules, glyphs, the `>>` button) and then the slots the app draws itself. Returns
/// where everything is, for hit-testing.
pub fn draw(c: &mut dyn Canvas, ui: &Ui, geom: &Geometry, status: &Status, pressed: Option<u32>) -> ToolbarLayout {
    // plain text: no bar at all (the reference shows none), so nothing of it is drawn
    if geom.format_h <= 0.0 {
        return layout(ui, geom, status);
    }
    let bar = toolbar(status);
    let placed = widgets::toolbar(c, ui, bounds(geom), &bar, pressed);
    let style = &status.style;
    let enabled = status.rich;
    for (id, slot) in &placed.visible {
        let popup = |c: &mut dyn Canvas, title: &str| widgets::popup_button(c, ui, popup_rect(*slot), title, false, enabled);
        match *id {
            COLOR => draw_well(c, ui, *slot, text_color(style.color), enabled),
            FONT => popup(c, family_name(style.font)),
            STYLE => popup(c, face_label(style.bold, style.italic)),
            SIZE => popup(c, &size_text(style.size)),
            SPACING => {
                let ink = if enabled { LABEL.now() } else { LABEL_3.now() };
                let label = ui.text(&spacing_label(style.spacing), MAC_BODY_PT, Weight::Regular, ink);
                blit(c, &label, ui.pt(slot.x + 4.0).floor(), (ui.pt(slot.y + slot.h / 2.0) - label.h as f32 / 2.0).floor());
                widgets::stepper(c, ui, stepper_origin(*slot), None, enabled);
            }
            _ => {}
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Style;
    use crate::view::geometry;
    use bifrost_app::kit::Font;

    #[test]
    fn the_family_pop_up_lists_every_family_and_past_a_menu_ends_with_show_fonts() {
        let rows = family_rows(12);
        assert_eq!(rows.len(), 12, "more than the eight families menu ids once allowed");
        assert_eq!(rows[11].action, Some(Action::Font(11)));
        assert_eq!(family_rows(MAX_MENU_ROWS).len(), MAX_MENU_ROWS);
        let many = family_rows(200);
        assert_eq!(many.len(), MAX_MENU_ROWS, "the context menu's limit");
        assert_eq!(many[MAX_MENU_ROWS - 3].action, Some(Action::Font(MAX_MENU_ROWS as u8 - 3)));
        assert_eq!(many[MAX_MENU_ROWS - 2].action, None, "a separator");
        assert_eq!(many[MAX_MENU_ROWS - 1].action, Some(Action::ShowFonts));
        // every row's action has a menu id the app reads back
        for row in &many {
            if let Some(action) = row.action {
                assert_eq!(Action::from_id(action.id().unwrap()), Some(action));
            }
        }
    }

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn status() -> Status {
        Status { style: Style::default(), align: Align::Left, rich: true }
    }

    #[test]
    fn the_items_have_their_own_ids_and_the_bar_holds_all_of_them() {
        let mut ids: Vec<u32> = ALL.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
        assert!(!ALL.contains(&OVERFLOW));
        let bar = toolbar(&status());
        assert_eq!(bar.items.iter().map(|i| i.id).collect::<Vec<_>>(), ALL.to_vec());
        for id in ALL {
            assert!(!label_of(id).is_empty(), "item {id} has an accessibility label");
        }
    }

    #[test]
    fn the_groups_are_the_character_styles_and_the_alignments() {
        let bar = toolbar(&status());
        let group = |id: u32| bar.items.iter().find(|i| i.id == id).map(|i| i.group);
        assert!([BOLD, ITALIC, UNDERLINE, STRIKE].iter().all(|id| group(*id) == Some(1)));
        assert!([LEFT, CENTER, RIGHT, JUSTIFY].iter().all(|id| group(*id) == Some(2)));
        assert!([COLOR, FONT, STYLE, SIZE, LIST, SPACING].iter().all(|id| group(*id) == Some(0)));
    }

    #[test]
    fn the_buttons_show_the_style_of_the_selection() {
        let mut s = status();
        s.style.bold = true;
        s.style.underline = true;
        s.align = Align::Center;
        s.style.list = ListKind::Numbered;
        let bar = toolbar(&s);
        let on = |id: u32| bar.items.iter().find(|i| i.id == id).is_some_and(|i| i.selected);
        assert!(on(BOLD) && on(UNDERLINE) && on(CENTER) && on(LIST));
        assert!(!on(ITALIC) && !on(STRIKE) && !on(LEFT) && !on(JUSTIFY));
        let list = bar.items.iter().find(|i| i.id == LIST).unwrap();
        assert_eq!(list.symbol.as_deref(), Some("list.number"));
    }

    #[test]
    fn a_plain_text_document_dims_the_bar_and_presses_do_nothing() {
        let mut s = status();
        s.rich = false;
        assert!(toolbar(&s).items.iter().all(|i| !i.enabled));
        let ui = ui();
        let geom = geometry(1200.0, 780.0, false, false);
        let placed = layout(&ui, &geom, &s);
        assert_eq!(press(&s, &placed, Hit::Item(BOLD), 0.0, 0.0), None);
        assert!(nodes(&placed, &s).iter().all(|n| !n.enabled || n.label == label_of(OVERFLOW)));
    }

    #[test]
    fn a_wide_window_shows_every_item_inside_its_edges() {
        let ui = ui();
        let geom = geometry(1200.0, 780.0, false, false);
        let placed = layout(&ui, &geom, &status());
        assert_eq!(placed.visible.len(), ALL.len());
        assert!(placed.overflow_button.is_none() && hidden(&placed).is_empty());
        let b = bounds(&geom);
        for (id, r) in &placed.visible {
            assert!(r.x >= b.x && r.right() <= b.right() + 0.01, "item {id} is inside the window");
            assert!(r.y >= geom.format_top && r.bottom() <= geom.format_top + FORMAT_H, "item {id} is inside the bar's band");
        }
    }

    #[test]
    fn a_narrow_window_moves_the_end_of_the_bar_behind_the_overflow_button() {
        let ui = ui();
        for width in [320.0, 480.0, 640.0, 760.0] {
            let geom = geometry(width, 600.0, false, false);
            let placed = layout(&ui, &geom, &status());
            let b = bounds(&geom);
            // nothing is cut by the window's edge
            for (id, r) in &placed.visible {
                assert!(r.right() <= b.right() + 0.01, "item {id} fits at {width}");
            }
            if let Some(more) = placed.overflow_button {
                assert!(more.right() <= b.right() + 0.01, "the >> button fits at {width}");
                let hidden = hidden(&placed);
                assert!(!hidden.is_empty(), "the button is there because something is hidden");
                // the visible items are the front of the bar, the hidden ones its end
                let first_hidden = ALL.iter().position(|id| hidden.contains(id)).unwrap();
                assert!(ALL[..first_hidden].iter().all(|id| placed.rect_of(*id).is_some()));
                let rows = overflow_rows(&hidden);
                assert!(!rows.is_empty() && rows.len() <= MAX_MENU_ROWS);
                assert!(rows.iter().all(|r| r.action.is_some_and(|a| a.id().is_some()) || r.action.is_none()));
            } else {
                assert_eq!(placed.visible.len(), ALL.len(), "no button means nothing is hidden at {width}");
            }
        }
        // a window as narrow as the bar's first group still shows the capsule of character styles whole
        let geom = geometry(240.0, 600.0, false, false);
        let placed = layout(&ui, &geom, &status());
        let styles = [BOLD, ITALIC, UNDERLINE, STRIKE].iter().filter(|id| placed.rect_of(**id).is_some()).count();
        assert!(styles == 0 || styles == 4, "a group moves behind the button as a whole");
    }

    #[test]
    fn the_overflow_menu_lists_what_is_hidden_and_stays_within_a_context_menu() {
        // everything hidden: all groups, trimmed to what a context menu holds
        let rows = overflow_rows(&ALL);
        assert_eq!(rows.len(), MAX_MENU_ROWS);
        assert_eq!(rows[0], Row::new("Bold", Action::Bold));
        assert!(rows.iter().filter(|r| r.action.is_none()).count() >= 3, "groups are separated");
        // only the end of the bar hidden: the spacing rows alone, without a leading separator
        let end = overflow_rows(&[SPACING]);
        assert_eq!(end.len(), SPACINGS.len());
        assert_eq!(end[0], Row::new("Spacing 1.0", Action::Spacing(0)));
        // the alignments and the list are separate groups
        let tail = overflow_rows(&[JUSTIFY, LIST]);
        assert_eq!(tail.len(), 1 + 1 + 2);
        assert_eq!(tail[0].action, Some(Action::Align(Align::Justified)));
        assert!(tail[1].action.is_none());
        assert!(overflow_rows(&[]).is_empty());
    }

    #[test]
    fn a_press_on_a_button_acts_and_a_press_on_a_pop_up_opens_its_menu_below_it() {
        let ui = ui();
        let geom = geometry(1200.0, 780.0, false, false);
        let s = status();
        let placed = layout(&ui, &geom, &s);
        assert_eq!(press(&s, &placed, Hit::Item(BOLD), 0.0, 0.0), Some(Press::Do(Action::Bold)));
        assert_eq!(press(&s, &placed, Hit::Item(STRIKE), 0.0, 0.0), Some(Press::Do(Action::Strike)));
        assert_eq!(press(&s, &placed, Hit::Item(RIGHT), 0.0, 0.0), Some(Press::Do(Action::Align(Align::Right))));
        let font = placed.rect_of(FONT).unwrap();
        match press(&s, &placed, Hit::Item(FONT), font.x + 4.0, font.y + 4.0) {
            Some(Press::Menu { anchor, rows }) => {
                assert_eq!(anchor, (font.x, font.bottom() + MENU_GAP));
                assert_eq!(rows.len(), crate::text::FAMILIES.len());
                assert_eq!(rows[1], Row::new("Times", Action::Font(1)));
            }
            other => panic!("the font pop-up opens a menu, got {other:?}"),
        }
        for (id, count) in [(COLOR, COLORS.len()), (STYLE, 4), (SIZE, SIZES.len()), (LIST, 2)] {
            let r = placed.rect_of(id).unwrap();
            assert!(matches!(press(&s, &placed, Hit::Item(id), r.x + 2.0, r.y + 2.0), Some(Press::Menu { rows, .. }) if rows.len() == count), "item {id}");
        }
    }

    #[test]
    fn the_line_spacing_steps_with_its_stepper_and_opens_a_menu_elsewhere() {
        let ui = ui();
        let geom = geometry(1200.0, 780.0, false, false);
        let mut s = status();
        s.style.spacing = 2;
        let placed = layout(&ui, &geom, &s);
        let slot = placed.rect_of(SPACING).unwrap();
        let (sx, sy) = stepper_origin(slot);
        let size = stepper_rect((0.0, 0.0));
        let item = Hit::Item(SPACING);
        assert_eq!(press(&s, &placed, item, sx + size.w / 2.0, sy + 2.0), Some(Press::Do(Action::Spacing(3))), "the upper half steps up");
        assert_eq!(press(&s, &placed, item, sx + size.w / 2.0, sy + size.h - 2.0), Some(Press::Do(Action::Spacing(1))), "the lower half steps down");
        assert!(matches!(press(&s, &placed, item, slot.x + 4.0, slot.y + 4.0), Some(Press::Menu { .. })));
        // the steps stop at the ends of the list
        s.style.spacing = 0;
        assert_eq!(press(&s, &placed, item, sx + size.w / 2.0, sy + size.h - 2.0), Some(Press::Do(Action::Spacing(0))));
        s.style.spacing = SPACINGS.len() as u8 - 1;
        assert_eq!(press(&s, &placed, item, sx + size.w / 2.0, sy + 2.0), Some(Press::Do(Action::Spacing(SPACINGS.len() as u8 - 1))));
    }

    #[test]
    fn the_overflow_button_opens_the_hidden_items() {
        let ui = ui();
        let geom = geometry(520.0, 600.0, false, false);
        let s = status();
        let placed = layout(&ui, &geom, &s);
        let more = placed.overflow_button.expect("a 520 point window overflows");
        assert_eq!(hit(&placed, more.x + 1.0, more.y + 1.0), Some(Hit::Overflow));
        match press(&s, &placed, Hit::Overflow, more.x + 1.0, more.y + 1.0) {
            Some(Press::Menu { anchor, rows }) => {
                assert_eq!(anchor, (more.x, more.bottom() + MENU_GAP));
                assert_eq!(rows, overflow_rows(&hidden(&placed)));
            }
            other => panic!("the overflow button opens a menu, got {other:?}"),
        }
        // a point on a shown item is that item, and a point outside the bar is nothing
        let (id, r) = placed.visible[0];
        assert_eq!(hit(&placed, r.x + 1.0, r.y + 1.0), Some(Hit::Item(id)));
        assert_eq!(hit(&placed, 1.0, 1.0), None);
    }

    #[test]
    fn the_published_controls_carry_the_state_the_bar_shows() {
        let ui = ui();
        let geom = geometry(1200.0, 780.0, false, false);
        let mut s = status();
        s.style.bold = true;
        s.style.size = 14.0;
        s.style.font = 1;
        s.style.color = 5;
        let placed = layout(&ui, &geom, &s);
        let nodes = nodes(&placed, &s);
        assert_eq!(nodes.len(), ALL.len());
        let find = |id: u32| nodes.iter().find(|n| n.id == NODE_BASE + u64::from(id)).unwrap();
        assert_eq!(find(BOLD).value, "selected");
        assert_eq!(find(ITALIC).value, "");
        assert_eq!(find(SIZE).value, "14");
        assert_eq!(find(FONT).value, "Times");
        assert_eq!(find(COLOR).value, "Blue");
        assert_eq!(find(STYLE).value, "Bold");
        assert!(nodes.iter().all(|n| !n.label.trim().is_empty() && n.role == Role::Button));
        // a node's id leads back to its item
        assert_eq!(item_of_node(NODE_BASE + u64::from(SIZE)), Some(SIZE));
        assert_eq!(item_of_node(NODE_BASE + u64::from(OVERFLOW)), Some(OVERFLOW));
        assert_eq!(item_of_node(NODE_BASE + 77), None);
        assert_eq!(item_of_node(5), None);
    }

    #[test]
    fn faces_and_sizes_read_as_the_pop_ups_show_them() {
        assert_eq!(face_label(false, false), "Regular");
        assert_eq!(face_label(true, true), "Bold Italic");
        assert_eq!(size_text(12.0), "12");
        assert_eq!(size_text(10.5), "10.5");
        let styles: Vec<Option<Action>> = style_rows().iter().map(|r| r.action).collect();
        assert_eq!(styles, [Action::FontStyle(0), Action::FontStyle(1), Action::FontStyle(2), Action::FontStyle(3)].map(Some));
    }
}
