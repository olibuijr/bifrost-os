//! The Golden Gate window (Golden Gate Notes): a full-size window with a source-list sidebar of folders under the
//! traffic lights, the note list column with its header, and the editor with the unified toolbar (compose,
//! format, checklist, list, photo, share, more, search). Geometry is in points for the kit and in pixels
//! (`ui.pt`) for the rest of the app.

use super::*;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use super::smart::Smart;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CONTENT_BG, LABEL, LABEL_2, SEPARATOR_C};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets::{self, CAPSULE_PT, TOOLBAR_GAP_PT};

/// Sidebar and note-list column widths (points), and the window width below which the sidebar folds away.
const SIDE_PT: f32 = 200.0;
/// The narrowest the editor's wrap column is kept.
const MIN_WRAP_PT: f32 = 40.0;
const LIST_PT: f32 = 270.0;
const SIDE_FOLD_PT: f32 = 720.0;
/// Source-list row and header heights, and the scroll that starts the rows under the toolbar band.
const SIDE_ROW_PT: f32 = 32.0;
const SIDE_HEADER_PT: f32 = 30.0;
const SIDE_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);

// Toolbar items (also the published control ids).
pub(super) const B_NEW_FOLDER: u32 = 1;
pub(super) const B_SIDEBAR: u32 = 2;
const B_LIST_TITLE: u32 = 3;
pub(super) const B_LIST_MORE: u32 = 4;
pub(super) const B_COMPOSE: u32 = 5;
pub(super) const B_STYLE: u32 = 6;
pub(super) const B_CHECK: u32 = 7;
pub(super) const B_BULLET: u32 = 8;
pub(super) const B_PHOTO: u32 = 9;
pub(super) const B_SHARE: u32 = 10;
pub(super) const B_MORE: u32 = 11;
pub(super) const B_SEARCH: u32 = 12;
/// the list's List / Gallery toggle
pub(super) const B_LAYOUT: u32 = 13;

/// Sidebar row ids: All Notes, Recently Deleted, then folder `i` as `FOLDER_ROW + i`.
const ROW_ALL: u32 = 100;
const ROW_BIN: u32 = 101;
const FOLDER_ROW: u32 = 200;
/// The smart folder rows (`smart::Smart::ALL` in order), above any folder row.
const SMART_ROW: u32 = 900;
/// The tag rows (`Notes::tag_list` in order), above the smart folder rows.
const TAG_ROW: u32 = 1000;

/// The pixel rect of a kit rect.
pub(super) fn px(ui: &Ui, r: Rect) -> (f32, f32, f32, f32) {
    (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h))
}

/// The kit rect of a pixel rect.
pub(super) fn pt_rect(ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) -> Rect {
    let k = ui.pt(1.0);
    Rect { x: x / k, y: y / k, w: w / k, h: h / k }
}

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

impl Notes {
    fn w_pt(&self, ui: &Ui) -> f32 {
        self.w as f32 / ui.pt(1.0)
    }

    /// The sidebar's width in points: 0 while hidden or in a narrow window.
    pub(super) fn side_pt(&self, ui: &Ui) -> f32 {
        if self.sidebar_hidden || self.w_pt(ui) < SIDE_FOLD_PT { 0.0 } else { SIDE_PT }
    }

    /// The note list column, `(x, w)` in points.
    pub(super) fn list_col(&self, ui: &Ui) -> (f32, f32) {
        let x = self.side_pt(ui);
        (x, LIST_PT.min((self.w_pt(ui) - x) * 0.45))
    }

    /// The editor column, `(x, w)` in points.
    pub(super) fn editor_col(&self, ui: &Ui) -> (f32, f32) {
        let (lx, lw) = self.list_col(ui);
        (lx + lw, (self.w_pt(ui) - lx - lw).max(0.0))
    }

    /// The editor column in pixels.
    pub(super) fn ed_px(&self, ui: &Ui) -> (f32, f32) {
        let (x, w) = self.editor_col(ui);
        (ui.pt(x), ui.pt(w))
    }

    /// The list column in pixels.
    pub(super) fn list_px(&self, ui: &Ui) -> (f32, f32) {
        let (x, w) = self.list_col(ui);
        (ui.pt(x), ui.pt(w))
    }

    pub(super) fn search_open(&self) -> bool {
        self.searching || self.search.focused || !self.search.is_empty()
    }

    /// The three toolbars of the unified band: over the sidebar, over the list, over the editor.
    pub(super) fn bars(&self, ui: &Ui) -> Vec<(Toolbar, Rect)> {
        let side = self.side_pt(ui);
        let lead = traffic_lights_trailing() + TOOLBAR_GAP_PT;
        let mut bars = vec![];
        if side > 0.0 {
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::flexible_space(),
                    ToolbarItem::icon(B_NEW_FOLDER, "New Folder", "folder.badge.plus"),
                    ToolbarItem::icon(B_SIDEBAR, "Hide Sidebar", "sidebar.left"),
                ]),
                band(lead, side - lead - 8.0),
            ));
        }
        let (lx, lw) = self.list_col(ui);
        let x0 = if side > 0.0 { lx + 12.0 } else { lead };
        let w = lx + lw - 8.0 - x0;
        let buttons = if side > 0.0 { 2.0 } else { 3.0 };
        let mut items = vec![];
        if side == 0.0 {
            items.push(ToolbarItem::icon(B_SIDEBAR, "Show Sidebar", "sidebar.left"));
        }
        items.push(ToolbarItem::custom(B_LIST_TITLE, "Title", (w - buttons * CAPSULE_PT - 2.0 * TOOLBAR_GAP_PT).max(40.0)));
        items.push(ToolbarItem::flexible_space());
        items.push(ToolbarItem::icon(B_LAYOUT, "View as Gallery", "square.grid.2x2"));
        items.push(ToolbarItem::icon(B_LIST_MORE, "View Options", "ellipsis"));
        bars.push((Toolbar::new(items), band(x0, w)));
        let (ex, ew) = self.editor_col(ui);
        let open = matches!(self.screen, Screen::Editor(_));
        let search_w = if self.search_open() { (ew * 0.3).clamp(120.0, 220.0) } else { CAPSULE_PT };
        bars.push((
            Toolbar::new(vec![
                ToolbarItem::button(B_COMPOSE, "New Note"),
                ToolbarItem::button(B_STYLE, "Aa").in_group(1).enabled(open),
                ToolbarItem::icon(B_CHECK, "Checklist", "checkmark.circle").in_group(1).enabled(open),
                ToolbarItem::icon(B_BULLET, "Bulleted List", "list.bullet").in_group(1).enabled(open),
                ToolbarItem::icon(B_PHOTO, "Add Photo", "photo").in_group(1).enabled(open),
                ToolbarItem::icon(B_SHARE, "Share", "square.and.arrow.up").in_group(2).enabled(open),
                ToolbarItem::icon(B_MORE, "More", "ellipsis").in_group(2).enabled(open),
                ToolbarItem::flexible_space(),
                ToolbarItem::custom(B_SEARCH, "Search", search_w),
            ]),
            band(ex + 12.0, ew - 24.0),
        ));
        bars
    }

    fn layouts(&self, ui: &Ui) -> Vec<(Toolbar, ToolbarLayout)> {
        self.bars(ui).into_iter().map(|(bar, r)| {
            let l = bar.layout_with(r, item_w(ui), CAPSULE_PT, TOOLBAR_GAP_PT);
            (bar, l)
        }).collect()
    }

    /// The rectangle (points) of toolbar item `id`, if it is on the bar.
    pub(super) fn bar_rect(&self, ui: &Ui, id: u32) -> Option<Rect> {
        self.layouts(ui).iter().find_map(|(_, l)| l.rect_of(id))
    }

    /// The enabled toolbar item under pixel `(x, y)`.
    pub(super) fn bar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        let (x, y) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if y >= UNIFIED_BAR_PT {
            return None;
        }
        self.layouts(ui).iter().find_map(|(bar, l)| {
            let id = l.hit(x, y)?;
            bar.items.iter().find(|i| i.id == id).filter(|i| i.enabled && id != B_LIST_TITLE).map(|_| id)
        })
    }

    /// Every toolbar button and the search field as accessible controls (`App::controls`), in points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = vec![];
        // the source list rows are buttons too: a tap at the centre of a row selects it
        if self.side_pt(ui) > 0.0 {
            let sidebar = self.sidebar();
            let bounds = self.sidebar_bounds(ui);
            for (i, item) in sidebar.items.iter().enumerate() {
                let SidebarItem::Row { id, label, enabled, .. } = item else { continue };
                let r = sidebar.item_rect(i, bounds, SIDE_SCROLL);
                if r.h <= 0.0 {
                    continue;
                }
                nodes.push(Node { id: *id as u64, role: Role::Row, label: label.clone(), value: String::new(), bounds: r, enabled: *enabled });
            }
        }
        for (bar, l) in self.layouts(ui) {
            for (id, r) in l.visible {
                let Some(item) = bar.items.iter().find(|i| i.id == id) else { continue };
                match id {
                    B_LIST_TITLE => {}
                    B_SEARCH => nodes.push(Node { id: id as u64, role: Role::TextField, label: "Search".into(), value: self.search.text().to_string(), bounds: r, enabled: true }),
                    _ => nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled }),
                }
            }
        }
        nodes
    }

    // --- sidebar ---

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.side_pt(ui), h: self.h as f32 / ui.pt(1.0) }
    }

    /// The notes a view shows (the count beside its name in the sidebar and the list menu).
    pub(super) fn count_in(&self, view: &View) -> u32 {
        let cutoff = self.smart_cutoff();
        self.notes.iter().filter(|n| {
            let loc = self.loc_of(&n.id);
            match view {
                View::All => loc != folders::TRASH,
                View::Trash => loc == folders::TRASH,
                View::Folder(f) => loc == f,
                View::Smart(s) => loc != folders::TRASH && s.matches(n, self.is_pinned(&n.id), cutoff),
                View::Tag(t) => loc != folders::TRASH && store::has_tag(&n.text, t),
            }
        }).count() as u32
    }

    /// The source list: All Notes, the folders, Recently Deleted, each with its count.
    pub(super) fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("On My Computer"), SidebarItem::row(ROW_ALL, "All Notes").with_symbol("folder").with_count(self.count_in(&View::All))];
        for (i, f) in self.folders.iter().enumerate() {
            items.push(SidebarItem::row(FOLDER_ROW + i as u32, f).with_symbol("folder").with_count(self.count_in(&View::Folder(f.clone()))));
        }
        items.push(SidebarItem::row(ROW_BIN, "Recently Deleted").with_symbol("trash").with_count(self.count_in(&View::Trash)));
        items.push(SidebarItem::header("Smart Folders"));
        for (i, s) in Smart::ALL.into_iter().enumerate() {
            items.push(SidebarItem::row(SMART_ROW + i as u32, s.label()).with_symbol(s.symbol()).with_count(self.count_in(&View::Smart(s))));
        }
        if !self.tag_list.is_empty() {
            items.push(SidebarItem::header("Tags"));
            for (i, (tag, count)) in self.tag_list.iter().enumerate() {
                items.push(SidebarItem::row(TAG_ROW + i as u32, &format!("#{tag}")).with_symbol("tag").with_count(*count));
            }
        }
        let mut list = SourceList::new(items, SIDE_ROW_PT, SIDE_HEADER_PT);
        list.set_selected(Some(match &self.view {
            View::All => ROW_ALL,
            View::Trash => ROW_BIN,
            View::Folder(f) => self.folders.iter().position(|x| x == f).map_or(ROW_ALL, |i| FOLDER_ROW + i as u32),
            View::Smart(s) => SMART_ROW + Smart::ALL.iter().position(|x| x == s).unwrap_or(0) as u32,
            View::Tag(t) => self.tag_list.iter().position(|(x, _)| x == t).map_or(ROW_ALL, |i| TAG_ROW + i as u32),
        }));
        list
    }

    /// The sidebar row id under pixel `(x, y)`.
    pub(super) fn sidebar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if x >= ui.pt(self.side_pt(ui)) || y < ui.pt(UNIFIED_BAR_PT) {
            return None;
        }
        self.sidebar().hit(self.sidebar_bounds(ui), SIDE_SCROLL, x / ui.pt(1.0), y / ui.pt(1.0))
    }

    /// The view a sidebar row stands for.
    pub(super) fn row_view(&self, id: u32) -> Option<View> {
        match id {
            ROW_ALL => Some(View::All),
            ROW_BIN => Some(View::Trash),
            id if (SMART_ROW..SMART_ROW + Smart::ALL.len() as u32).contains(&id) => Smart::ALL.get((id - SMART_ROW) as usize).map(|s| View::Smart(*s)),
            id if id >= TAG_ROW => self.tag_list.get((id - TAG_ROW) as usize).map(|(t, _)| View::Tag(t.clone())),
            id => self.folders.get(id.checked_sub(FOLDER_ROW)? as usize).map(|f| View::Folder(f.clone())),
        }
    }

    /// Shows `view` in the list (a sidebar click), keeping the open note open.
    pub(super) fn show_view(&mut self, ui: &Ui, view: View, t: f64) {
        if matches!(self.screen, Screen::Editor(_)) {
            self.close_note(ui, t);
        }
        self.view = view;
        self.search.set_text(ui, "");
        self.list_scroll.jump_top();
        self.reload(ui, t);
    }

    pub(super) fn toggle_sidebar(&mut self, ui: &Ui) {
        self.sidebar_hidden = !self.sidebar_hidden;
        self.sync_width(ui);
        self.dirty = true;
    }

    /// Keeps the editor's wrap width in step with its column.
    pub(super) fn sync_width(&mut self, ui: &Ui) {
        let w = (self.ed_px(ui).1 - 2.0 * ui.pt(TEXT_INSET_PT)).max(ui.pt(MIN_WRAP_PT));
        if (w - self.editor_w).abs() > 0.5 {
            self.editor_w = w;
            self.editor.set_width(ui, w);
            self.load_images(ui);
            self.fit_scroll(ui);
        }
    }

    // --- drawing ---

    /// The sidebar, the toolbar band over the list and editor, the toolbars, the list title and the search.
    pub(super) fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let side = self.side_pt(ui);
        if side > 0.0 {
            widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), SIDE_SCROLL, None);
            s.fill_rect(ui.pt(side) as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        }
        let (lx, _) = self.list_col(ui);
        widgets::toolbar_band(s, ui, Rect { x: lx, y: 0.0, w: self.w_pt(ui) - lx, h: UNIFIED_BAR_PT });
        let (ex, _) = self.ed_px(ui);
        s.fill_rect(ex as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
        for (bar, r) in self.bars(ui) {
            let placed = widgets::toolbar(s, ui, r, &bar, None);
            if let Some(t) = placed.rect_of(B_LIST_TITLE) {
                let title = ui.fit(&self.view_title(), MAC_BODY_PT, ui.pt(t.w));
                let n = self.shown.len();
                let count = if n == 1 { "1 note".to_string() } else { format!("{n} notes") };
                let a = self.labels.get(ui, &title, MAC_BODY_PT, Weight::Bold, LABEL.now()).clone();
                let b = self.labels.get(ui, &count, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now()).clone();
                let top = ui.pt(t.y + t.h / 2.0) - (a.h + b.h) as f32 / 2.0;
                s.blit(&a, ui.pt(t.x) as i32, top as i32, 255, 255);
                s.blit(&b, ui.pt(t.x) as i32, (top + a.h as f32) as i32, 255, 255);
            }
            if let Some(r) = placed.rect_of(B_SEARCH) {
                if self.search_open() {
                    let mut field = bifrost_app::controls::TextField::new();
                    field.push_str(self.search.text());
                    widgets::search_field(s, ui, &field, Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 }, "Search", self.search.focused);
                } else {
                    widgets::search_button(s, ui, (r.x, r.y), false);
                }
            }
        }
    }

    /// The background of the content columns.
    pub(super) fn draw_background(&self, s: &mut Surface) {
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
    }
}
