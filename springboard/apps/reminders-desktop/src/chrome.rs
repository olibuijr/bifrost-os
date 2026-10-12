//! The Golden Gate window (Golden Gate Reminders): a full-size window with the sidebar under the traffic lights
//! (the smart-list tiles two to a row, then My Lists as a source list, Add List at the bottom), the open list
//! beside it, and the unified toolbar (export, view options, list info, New Reminder, search). The content draws
//! in its own column (`x0` px from the window's left edge).

use super::*;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{Weight, CONTENT_BG, LABEL, LABEL_2, SEPARATOR_C, SIDEBAR_BG, GRAY_DIM, WHITE, MAC_BODY_PT, MAC_TITLE2_PT};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets::{self, CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::kit::Surface;
use bifrost_app::kit::ui::{SPACE_L, SPACE_XS, SPACE_XXS};

/// Sidebar width and the window width below which it folds away (points).
const SIDE_PT: f32 = 250.0;
const SIDE_FOLD_PT: f32 = 600.0;
/// Smart-list tiles: height, gap and the sidebar's side inset (points).
const TILE_H: f32 = 56.0;
const TILE_GAP: f32 = 8.0;
const SIDE_INSET: f32 = 10.0;
/// The Add List row at the sidebar's bottom (points).
const ADD_LIST_H: f32 = 36.0;
/// Source-list row ids of My Lists: `LIST_ROW + index`.
const LIST_ROW: u32 = 1000;
/// Accessibility ids of the sidebar's smart tiles and Add List (the toolbar uses 1 to 9).
const SIDEBAR_TILE: u64 = 100;
const SIDEBAR_ADD: u64 = 200;

const B_SIDEBAR: u32 = 1;
const B_EXPORT: u32 = 2;
const B_OPTIONS: u32 = 3;
const B_INFO: u32 = 4;
pub(super) const B_ADD: u32 = 5;
pub(super) const B_SEARCH: u32 = 6;
/// The toolbar search field's width range (points).
const SEARCH_MIN_PT: f32 = 120.0;
const SEARCH_MAX_PT: f32 = 220.0;
pub(super) const B_CANCEL: u32 = 7;
const B_TITLE: u32 = 8;
pub(super) const B_DONE: u32 = 9;

/// The width the kit's `toolbar` gives each button (glyph buttons are a capsule, titled ones their label).
fn item_w(ui: &Ui) -> impl Fn(&ToolbarItem) -> f32 + '_ {
    move |it| {
        if it.symbol.is_some() {
            CAPSULE_PT
        } else {
            (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(CAPSULE_PT)
        }
    }
}

fn band(x: f32, w: f32) -> Rect {
    Rect { x, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: w.max(0.0), h: CAPSULE_PT }
}

/// A smart list's tile colour (Golden Gate Reminders).
pub(super) fn smart_colour(s: Smart) -> u32 {
    match s {
        Smart::Today => bifrost_app::kit::ui::SYSTEM_BLUE,
        Smart::Scheduled => bifrost_app::kit::ui::SYSTEM_RED,
        Smart::Flagged => bifrost_app::kit::ui::SYSTEM_ORANGE,
        Smart::All | Smart::Completed => GRAY_DIM,
    }
}

impl Reminders {
    /// Splits the window into the sidebar and the content column (`x0`, `w` in px).
    pub(super) fn layout_columns(&mut self, ui: &Ui) {
        let full = self.full_w as f32;
        let side = if self.side_hidden || full / ui.pt(1.0) < SIDE_FOLD_PT { 0.0 } else { ui.pt(SIDE_PT).round() };
        self.x0 = side;
        self.w = (full - side) as i32;
    }

    fn k(ui: &Ui) -> f32 {
        ui.pt(1.0)
    }

    fn list_view(&self) -> bool {
        matches!(self.view, View::List(_)) && !self.searching
    }

    fn search_open(&self) -> bool {
        self.searching && self.screen == Screen::Items
    }

    /// The toolbars: over the sidebar, and the unified band over the content.
    pub(super) fn bars(&self, ui: &Ui) -> Vec<(Toolbar, Rect)> {
        let k = Self::k(ui);
        let (x0, w) = (self.x0 / k, self.w as f32 / k);
        let lead = traffic_lights_trailing() + TOOLBAR_GAP_PT;
        let mut bars = vec![];
        if x0 > 0.0 {
            bars.push((Toolbar::new(vec![ToolbarItem::flexible_space(), ToolbarItem::icon(B_SIDEBAR, "Hide Sidebar", "sidebar.left")]), band(lead, x0 - lead - 8.0)));
        }
        let start = if x0 > 0.0 { x0 + 12.0 } else { lead };
        let end = x0 + w - 12.0;
        if matches!(self.screen, Screen::Editor | Screen::ListEditor) {
            let title = if self.screen == Screen::Editor {
                if self.editor.as_ref().is_some_and(|e| e.id.is_some()) { "Details" } else { "New Reminder" }
            } else if self.list_edit.as_ref().is_some_and(|e| e.id.is_some()) {
                "List Info"
            } else {
                "New List"
            };
            let can_save = !self.title.text().trim().is_empty();
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_DONE, "Done").enabled(can_save),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        let mut items = vec![];
        if x0 == 0.0 {
            items.push(ToolbarItem::icon(B_SIDEBAR, "Show Sidebar", "sidebar.left"));
        }
        items.push(ToolbarItem::flexible_space());
        items.push(ToolbarItem::icon(B_EXPORT, "Export List", "square.and.arrow.up").in_group(1).enabled(self.file_export && self.list_view()));
        items.push(ToolbarItem::icon(B_OPTIONS, "View Options", "arrow.up.arrow.down").in_group(1).selected(self.options_open));
        items.push(ToolbarItem::icon(B_INFO, "Show List Info", "ellipsis").in_group(1).enabled(self.list_view()));
        items.push(ToolbarItem::icon(B_ADD, "New Reminder", "plus").in_group(1));
        // the search field is always in the toolbar (Golden Gate: a visible search field, not a collapsed button)
        let search_w = ((end - start) * 0.35).clamp(SEARCH_MIN_PT, SEARCH_MAX_PT);
        items.push(ToolbarItem::custom(B_SEARCH, "Search", search_w));
        bars.push((Toolbar::new(items), band(start, end - start)));
        bars
    }

    fn layouts(&self, ui: &Ui) -> Vec<(Toolbar, ToolbarLayout)> {
        self.bars(ui).into_iter().map(|(bar, r)| {
            let l = bar.layout_with(r, item_w(ui), CAPSULE_PT, TOOLBAR_GAP_PT);
            (bar, l)
        }).collect()
    }

    /// The pixel rect (window coordinates) of toolbar item `id`.
    pub(super) fn bar_px(&self, ui: &Ui, id: u32) -> Option<(f32, f32, f32, f32)> {
        let r = self.layouts(ui).iter().find_map(|(_, l)| l.rect_of(id))?;
        Some((ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)))
    }

    /// The text in the toolbar's search field: the query while searching, empty otherwise.
    fn search_text(&self) -> String {
        if self.search_open() { self.title.text().to_string() } else { String::new() }
    }

    /// The open search field's text area in window pixels.
    pub(super) fn search_px(&self, ui: &Ui) -> Option<(f32, f32, f32, f32)> {
        if !self.search_open() {
            return None;
        }
        let (x, y, w, h) = self.bar_px(ui, B_SEARCH)?;
        Some((x + ui.pt(28.0), y + ui.pt(SPACE_XS), (w - ui.pt(36.0)).max(1.0), h - ui.pt(SPACE_S)))
    }

    /// The target of the toolbar button under window pixel `(x, y)`.
    pub(super) fn bar_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let k = Self::k(ui);
        let id = self.layouts(ui).iter().find_map(|(bar, l)| {
            let id = l.hit(x / k, y / k)?;
            bar.items.iter().find(|i| i.id == id).filter(|i| i.enabled && id != B_TITLE).map(|_| id)
        })?;
        Some(match id {
            B_SIDEBAR => Target::Sidebar,
            B_EXPORT => Target::OptExport,
            B_OPTIONS => Target::Options,
            B_INFO => match self.view {
                View::List(l) => Target::ListInfo(l),
                View::Smart(_) => return None,
            },
            B_ADD => Target::New,
            B_SEARCH if self.search_open() => Target::Field(Field::Title),
            B_SEARCH => Target::Search,
            B_CANCEL => Target::Cancel,
            _ => Target::Save,
        })
    }

    /// Every toolbar button and the search field as accessible controls (`App::controls`), in points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = vec![];
        for (bar, l) in self.layouts(ui) {
            for (id, r) in l.visible {
                let Some(item) = bar.items.iter().find(|i| i.id == id && id != B_TITLE) else { continue };
                let (role, value) = if id == B_SEARCH { (Role::TextField, self.search_text()) } else { (Role::Button, String::new()) };
                nodes.push(Node { id: id as u64, role, label: item.label.clone(), value, bounds: r, enabled: item.enabled });
            }
        }
        // the sidebar: smart tiles (label, count), My Lists rows (name, count) and Add List
        if self.x0 > 0.0 {
            for (i, smart) in Smart::ALL.iter().enumerate() {
                let value = smart.count(&self.data.items, self.today).to_string();
                nodes.push(Node { id: SIDEBAR_TILE + i as u64, role: Role::Button, label: smart.label().to_string(), value, bounds: self.tile_rect(ui, i), enabled: true });
            }
            let lists = self.lists_source();
            let area = self.lists_bounds(ui);
            for (i, list) in self.data.lists.iter().enumerate() {
                // the header is item 0 of the source list
                let bounds = lists.item_rect(i + 1, area, 0.0);
                let value = model::list_count(&self.data.items, list.id).to_string();
                nodes.push(Node { id: LIST_ROW as u64 + i as u64, role: Role::Row, label: list.name.clone(), value, bounds, enabled: true });
            }
            nodes.push(Node { id: SIDEBAR_ADD, role: Role::Button, label: "Add List".into(), value: String::new(), bounds: self.add_list_rect(ui), enabled: true });
        }
        nodes
    }

    // ---- sidebar ---------------------------------------------------------

    /// Smart tile `i` in points (two to a row under the toolbar band).
    fn tile_rect(&self, ui: &Ui, i: usize) -> Rect {
        let w = (self.x0 / Self::k(ui) - 2.0 * SIDE_INSET - TILE_GAP) / 2.0;
        Rect { x: SIDE_INSET + (w + TILE_GAP) * (i % 2) as f32, y: self.top_pt + UNIFIED_BAR_PT + 4.0 + (TILE_H + TILE_GAP) * (i / 2) as f32, w, h: TILE_H }
    }

    fn lists_bounds(&self, ui: &Ui) -> Rect {
        let rows = Smart::ALL.len().div_ceil(2) as f32;
        let top = self.top_pt + UNIFIED_BAR_PT + 4.0 + (TILE_H + TILE_GAP) * rows + 4.0;
        Rect { x: 0.0, y: top, w: self.x0 / Self::k(ui), h: (self.h as f32 / Self::k(ui) - top - ADD_LIST_H).max(0.0) }
    }

    fn add_list_rect(&self, ui: &Ui) -> Rect {
        let k = Self::k(ui);
        Rect { x: SIDE_INSET, y: self.h as f32 / k - ADD_LIST_H, w: self.x0 / k - 2.0 * SIDE_INSET, h: ADD_LIST_H }
    }

    /// My Lists: each list with its colour dot and open count, the open list selected.
    fn lists_source(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("My Lists")];
        for (i, l) in self.data.lists.iter().enumerate() {
            let colour = view::COLORS[l.color as usize % view::COLORS.len()];
            items.push(SidebarItem::row(LIST_ROW + i as u32, &l.name).with_dot(colour).with_count(model::list_count(&self.data.items, l.id) as u32));
        }
        let mut list = SourceList::new(items, 32.0, 30.0);
        let open = match self.view {
            View::List(id) if !self.searching => self.data.lists.iter().position(|l| l.id == id).map(|i| LIST_ROW + i as u32),
            _ => None,
        };
        list.set_selected(open);
        list
    }

    /// The sidebar target under window pixel `(x, y)`.
    pub(super) fn sidebar_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let k = Self::k(ui);
        let (px, py) = (x / k, y / k);
        if py < self.top_pt + UNIFIED_BAR_PT {
            return None;
        }
        for (i, smart) in Smart::ALL.iter().enumerate() {
            if self.tile_rect(ui, i).contains(px, py) {
                return Some(Target::Smart(*smart));
            }
        }
        if self.add_list_rect(ui).contains(px, py) {
            return Some(Target::NewList);
        }
        let id = self.lists_source().hit(self.lists_bounds(ui), 0.0, px, py)?;
        self.data.lists.get((id - LIST_ROW) as usize).map(|l| Target::List(l.id))
    }

    /// The sidebar: the smart-list tiles in their colours (count top right, name bottom left, the selected one
    /// outlined), My Lists, and Add List at the bottom.
    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        let k = Self::k(ui);
        let side = self.x0;
        s.fill_rect(0, 0, side as usize, self.h as usize, SIDEBAR_BG.now());
        let today = self.today;
        for (i, smart) in Smart::ALL.iter().enumerate() {
            let r = self.tile_rect(ui, i);
            let (x, y, w, h) = (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h));
            let colour = smart_colour(*smart);
            s.fill_rrect(x, y, w, h, ui.pt(bifrost_app::controls::prefs_layout::m::SELECTION_RADIUS + 2.0), colour);
            if self.view == View::Smart(*smart) && !self.searching {
                s.stroke_rrect(x - ui.pt(SPACE_XXS), y - ui.pt(SPACE_XXS), w + ui.pt(SPACE_XS), h + ui.pt(SPACE_XS), ui.pt(bifrost_app::controls::prefs_layout::m::SELECTION_RADIUS + 4.0), ui.pt(SPACE_XXS), LABEL.now());
            }
            self.smart_glyph(s, ui, *smart, (x + ui.pt(SPACE_L + 2.0), y + ui.pt(SPACE_L + 2.0)), colour, today);
            let count = smart.count(&self.data.items, today).to_string();
            self.right(s, ui, &count, MAC_TITLE2_PT, Weight::Bold, WHITE, x + w - ui.pt(SPACE_M), y + ui.pt(SPACE_S));
            self.text(s, ui, smart.label(), MAC_BODY_PT, Weight::Semibold, WHITE, x + ui.pt(SPACE_M), y + h - ui.pt(SPACE_XL + 2.0));
        }
        widgets::source_list(s, ui, self.lists_bounds(ui), &self.lists_source(), 0.0, None);
        // the tiles' column above My Lists keeps the sidebar colour (source_list painted only its bounds)
        let add = self.add_list_rect(ui);
        let (ax, ay, ah) = (ui.pt(add.x), ui.pt(add.y), ui.pt(add.h));
        s.fill_rect(0, ay as usize, side as usize, (self.h as f32 - ay).max(0.0) as usize, SIDEBAR_BG.now());
        let g = 16.0;
        bifrost_app::glyphs::glyph(s, ui, "plus", Rect { x: add.x + SPACE_XS, y: add.y + (add.h - g) / 2.0, w: g, h: g }, LABEL_2.now());
        self.text(s, ui, "Add List", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), ax + ui.pt(SPACE_XS + g + SPACE_S), ay + ah / 2.0 - ui.pt(SPACE_S));
        s.fill_rect(side as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        let _ = k;
    }

    /// The window around the content: the sidebar, the toolbar band and the toolbars, the search field.
    pub(super) fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let k = Self::k(ui);
        if self.x0 > 0.0 {
            self.draw_sidebar(s, ui);
        }
        widgets::toolbar_band(s, ui, Rect { x: self.x0 / k, y: 0.0, w: self.w as f32 / k, h: self.top_pt + UNIFIED_BAR_PT });
        for (bar, r) in self.bars(ui) {
            let placed = widgets::toolbar(s, ui, r, &bar, None);
            if let Some(t) = placed.rect_of(B_TITLE) {
                let title = bar.items.iter().find(|i| i.id == B_TITLE).map(|i| i.label.clone()).unwrap_or_default();
                let img = self.labels.get(ui, &title, MAC_BODY_PT, Weight::Bold, LABEL.now());
                s.blit(img, ui.pt(t.x) as i32, (ui.pt(t.y + t.h / 2.0) - img.h as f32 / 2.0) as i32, 255, 255);
            }
            if let Some(r) = placed.rect_of(B_SEARCH) {
                let mut field = bifrost_app::controls::TextField::new();
                field.push_str(&self.search_text());
                let focused = self.search_open() && self.focus == Some(Field::Title);
                widgets::search_field(s, ui, &field, Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 }, "Search", focused);
            }
        }
    }

    /// The content column of `s`: the same pixels, starting `x0` px from the left.
    pub(super) fn column<'a>(&self, s: &'a mut Surface) -> Surface<'a> {
        let x0 = (self.x0 as usize).min(s.w);
        Surface { w: s.w - x0, h: s.h, stride: s.stride, px: &mut s.px[x0..], y0: s.y0 }
    }

    pub(super) fn background(&self, s: &mut Surface) {
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
    }
}
