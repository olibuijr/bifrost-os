//! The desktop presentation of Photos: a full-size window (`NativeWindowStyleMaskFullSizeContentView`) whose source
//! list runs to the top edge under the traffic lights (Library, Pinned collections, Albums, Memories, Utilities), with
//! the photo pane beside it and the pane's toolbar in the unified band: title on the leading side, then the zoom
//! buttons, Select, the options menu and a search capsule on the trailing side, as the Golden Gate Photos window
//! (`goldengate_refimages/27-Golden-Gate-Photos.png`). The tablet keeps its push navigation (the album picker under
//! the navigation bar); this layout is only used on the desktop form factor (`bifrost_ui::form_factor`).
//!
//! Geometry is in points and comes from the shared split-window model (`bifrost_ui::prefs_layout`) that Settings and
//! the App Store use, so the sidebar width, row pitch, selection plate and traffic lights match theirs.

use super::*;
use super::{Album, Target};
use crate::shell::ui::{FIELD_BG, LABEL, LINK, MAC_BODY_PT, MAC_GROUP_HEADER_PT, MAC_TITLE3_PT, MATERIAL_SIDEBAR, MATERIAL_SIDEBAR_SOLID, MATERIAL_TITLEBAR_SOLID, SEPARATOR_C, SIDEBAR_SELECTED, TINT};
use bifrost_ui::prefs_layout::{self, m, Frame};
use bifrost_ui::Rect;
use std::cell::{Cell, RefCell};

/// The unified title and toolbar band at the top of the window (pt); the shell's drag zone uses the same height.
pub const BAR_PT: f32 = super::super::desk::UNIFIED_BAR_PT;
/// A group header ("Pinned", "Albums") above its rows (pt).
pub const HEADER_PT: f32 = 28.0;
/// The toolbar's search capsule (pt).
pub const SEARCH_W_PT: f32 = 160.0;
pub const SEARCH_H_PT: f32 = 28.0;
/// A round toolbar button (zoom, options) and the Select text button (pt).
pub const BUTTON_PT: f32 = 28.0;
pub const SELECT_W_PT: f32 = 60.0;
/// The gap between toolbar items and from the pane's trailing edge (pt).
pub const GAP_PT: f32 = 8.0;
pub const EDGE_PT: f32 = 12.0;

thread_local! {
    /// Photos runs in the desktop sidebar layout: the shell's window code makes its window full-size (`desk::full_size`).
    static SIDEBAR_LAYOUT: Cell<bool> = const { Cell::new(false) };
    /// The controls the window last drew in its toolbar band (window pt), for the shell's drag zone: a press on them
    /// reaches Photos, the rest of the band moves the window.
    static TOOLBAR: RefCell<Vec<(bool, &'static str, Rect)>> = const { RefCell::new(Vec::new()) };
}

/// Whether Photos' window uses the desktop sidebar layout (a full-size window without a title bar).
pub fn sidebar_layout() -> bool {
    SIDEBAR_LAYOUT.with(Cell::get)
}

pub(super) fn set_sidebar_layout(on: bool) {
    SIDEBAR_LAYOUT.with(|c| c.set(on));
    if !on {
        publish_toolbar(Vec::new());
    }
}

/// Keeps the toolbar controls (text field or not, label, window pt) the window drew.
pub(super) fn publish_toolbar(controls: Vec<(bool, &'static str, Rect)>) {
    TOOLBAR.with(|t| *t.borrow_mut() = controls);
}

/// The window's toolbar controls as semantic nodes in window points, as the shell's drag zone takes them
/// (`desk::drag_zone`), the way Settings and the App Store report theirs.
pub fn toolbar_controls() -> Vec<bifrost_proto::Node> {
    use bifrost_proto::{Node, Role};
    TOOLBAR.with(|t| {
        t.borrow()
            .iter()
            .enumerate()
            .map(|(i, (field, label, bounds))| Node {
                id: i as u64 + 1,
                role: if *field { Role::TextField } else { Role::Button },
                label: (*label).into(),
                value: String::new(),
                bounds: *bounds,
                enabled: true,
            })
            .collect()
    })
}

/// The sidebar's width (pt) for a window `w` x `h` pt: the split model's, never more than 40% of a narrow window.
pub fn width(w: f32, h: f32) -> f32 {
    Frame::new(w, h).sidebar.w
}

/// A row of the source list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Album(Album),
    /// "New Album" at the foot of the Albums group
    NewAlbum,
}

/// The source list laid out: the group headers (title, label centre y) and every row with its selection rect, in
/// window points.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub headers: Vec<(&'static str, f32)>,
    pub rows: Vec<(Row, Rect)>,
}

impl Layout {
    /// Lays `groups` out (an untitled group has no header; an empty one is left out) for a window `w` x `h` pt. The
    /// first row sits under the traffic lights' band, each group after a header and the split model's group gap.
    pub fn new(w: f32, h: f32, groups: &[(Option<&'static str>, Vec<Row>)]) -> Self {
        let frame = Frame::new(w, h);
        let (mut headers, mut rows) = (Vec::new(), Vec::new());
        let mut y = BAR_PT;
        for (title, items) in groups.iter().filter(|(_, items)| !items.is_empty()) {
            if let Some(title) = title {
                headers.push((*title, y + HEADER_PT / 2.0 + 2.0));
                y += HEADER_PT;
            }
            let (rects, next) = prefs_layout::sidebar_group(&frame, y, items.len());
            rows.extend(items.iter().copied().zip(rects));
            y = next;
        }
        Layout { width: frame.sidebar.w, headers, rows }
    }

    /// The row under window point (`x`, `y`), if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<Row> {
        self.rows.iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(row, _)| *row)
    }
}

/// The source list's groups for the albums Photos has (`menu`, in the picker's order) and whether another user album
/// can be made: Library alone, then Pinned (Favorites, Recently Saved, Map, Screenshots), Albums (the user's and the
/// smart albums, then New Album), Memories, and Utilities (Recently Deleted, Duplicates, Hidden).
pub fn groups(menu: &[Album], can_add: bool) -> Vec<(Option<&'static str>, Vec<Row>)> {
    let pick = |want: &[Album]| want.iter().filter(|a| menu.contains(*a)).map(|&a| Row::Album(a)).collect::<Vec<_>>();
    let mut albums: Vec<Row> = menu.iter().filter(|a| matches!(a, Album::User(_) | Album::Smart(_))).map(|&a| Row::Album(a)).collect();
    if can_add {
        albums.push(Row::NewAlbum);
    }
    let memories = menu.iter().filter(|a| matches!(a, Album::Memory(_))).map(|&a| Row::Album(a)).collect();
    vec![
        (None, vec![Row::Album(Album::All)]),
        (Some("Pinned"), pick(&[Album::Favorites, Album::Recents, Album::Places, Album::Screenshots])),
        (Some("Albums"), albums),
        (Some("Memories"), memories),
        (Some("Utilities"), pick(&[Album::Trash, Album::Duplicates, Album::Hidden])),
    ]
}

/// The name a fixed album goes by in the source list (the reference's names); user albums keep their own.
pub fn row_title(album: Album) -> Option<&'static str> {
    Some(match album {
        Album::All => "Library",
        Album::Recents => "Recently Saved",
        Album::Places => "Map",
        Album::User(_) | Album::Smart(_) | Album::Memory(_) => return None,
        other => other.title(),
    })
}

/// The pane's toolbar controls, trailing edge first laid out leading-ward, in pane points: the search capsule, the
/// options menu, Select and the zoom buttons. An empty library offers the search and options only (Import lives in
/// the options menu); a pane too narrow for an item drops it rather than overlap the title.
pub fn toolbar(pane_w: f32, blank: bool, title_end: f32) -> Vec<(Target, Rect)> {
    let mid = BAR_PT / 2.0;
    let mut right = pane_w - EDGE_PT;
    let mut out = Vec::new();
    let mut items = vec![(Target::SearchOpen, SEARCH_W_PT, SEARCH_H_PT), (Target::Album, BUTTON_PT, BUTTON_PT)];
    if !blank {
        items.extend([(Target::Select, SELECT_W_PT, BUTTON_PT), (Target::ZoomIn, BUTTON_PT, BUTTON_PT), (Target::ZoomOut, BUTTON_PT, BUTTON_PT)]);
    }
    for (target, w, h) in items {
        let x = right - w;
        if x < title_end + GAP_PT {
            break;
        }
        out.push((target, Rect { x, y: mid - h / 2.0, w, h }));
        right = x - GAP_PT;
    }
    out
}

/// Where the pane's toolbar title starts (pt from the pane's leading edge).
pub fn title_x() -> f32 {
    m::DETAIL_MARGIN
}

/// The label a toolbar control goes by for the shell's drag zone.
fn control_label(target: Target) -> &'static str {
    match target {
        Target::SearchOpen => "Search",
        Target::Album => "Options",
        Target::Select => "Select",
        Target::ZoomIn => "Zoom In",
        Target::ZoomOut => "Zoom Out",
        _ => "Title",
    }
}

/// Strokes `lines` (px) in `color`, `t` wide, inside the box of side `size` about (`cx`, `cy`).
fn stroke(s: &mut Surface, (cx, cy): (f32, f32), size: f32, t: f32, color: u32, lines: &[((f32, f32), (f32, f32))]) {
    let b = (cx - size / 2.0 - t, cy - size / 2.0 - t, size + 2.0 * t, size + 2.0 * t);
    for &(a, z) in lines {
        glyphs::paint(s, b, color, glyphs::segment(a, z, t));
    }
}

/// A source list row's symbol, about (`cx`, `cy`), `size` px across.
fn side_icon(s: &mut Surface, row: Row, (cx, cy): (f32, f32), size: f32, t: f32, color: u32) {
    let h = size / 2.0;
    match row {
        Row::Album(Album::Favorites) => heart_icon(s, cx, cy, h * 0.8, t, color, false),
        Row::Album(Album::Trash) => trash_icon(s, cx, cy, size * 0.9, t, color),
        Row::NewAlbum => stroke(s, (cx, cy), size, t, color, &[((cx - h * 0.7, cy), (cx + h * 0.7, cy)), ((cx, cy - h * 0.7), (cx, cy + h * 0.7))]),
        Row::Album(Album::Recents) => {
            glyphs::paint(s, (cx - h - t, cy - h - t, size + 2.0 * t, size + 2.0 * t), color, glyphs::ring((cx, cy), h * 0.8, t));
            stroke(s, (cx, cy), size, t, color, &[((cx, cy), (cx, cy - h * 0.5)), ((cx, cy), (cx + h * 0.4, cy))]);
        }
        Row::Album(Album::Places) => {
            // a folded map: three panels
            let (x0, x1, y0, y1) = (cx - h * 0.85, cx + h * 0.85, cy - h * 0.7, cy + h * 0.7);
            let (a, b, k) = (cx - h * 0.3, cx + h * 0.3, h * 0.15);
            let lines = [
                ((x0, y0 + k), (a, y0)),
                ((a, y0), (b, y0 + k)),
                ((b, y0 + k), (x1, y0)),
                ((x0, y1), (a, y1 - k)),
                ((a, y1 - k), (b, y1)),
                ((b, y1), (x1, y1 - k)),
                ((x0, y0 + k), (x0, y1)),
                ((x1, y0), (x1, y1 - k)),
                ((a, y0), (a, y1 - k)),
                ((b, y0 + k), (b, y1)),
            ];
            stroke(s, (cx, cy), size, t, color, &lines);
        }
        Row::Album(Album::Duplicates) => {
            let d = h * 0.3;
            for (ox, oy) in [(-d, -d), (d, d)] {
                let (x0, y0, x1, y1) = (cx + ox - h * 0.55, cy + oy - h * 0.55, cx + ox + h * 0.55, cy + oy + h * 0.55);
                stroke(s, (cx, cy), size, t, color, &[((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))]);
            }
        }
        _ => {
            // a photo: a frame with a hill and a sun
            let (x0, y0, x1, y1) = (cx - h * 0.85, cy - h * 0.65, cx + h * 0.85, cy + h * 0.65);
            let lines = [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0)), ((x0, y1), (cx - h * 0.1, cy)), ((cx - h * 0.1, cy), (x1, y1))];
            stroke(s, (cx, cy), size, t, color, &lines);
            glyphs::paint(s, (cx - h, cy - h, size, size), color, glyphs::disc((cx + h * 0.4, cy - h * 0.25), h * 0.15));
        }
    }
}

impl Photos {
    /// The source list for the current albums and window.
    pub(super) fn side_layout(&self, ui: &Ui) -> Layout {
        let pt = ui.pt(1.0).max(0.01);
        Layout::new(self.win_w as f32 / pt, self.h as f32 / pt, &groups(&self.menu_albums(), self.user_albums.len() < MAX_USER_ALBUMS))
    }

    /// The name of `album` in the source list and the toolbar.
    pub(super) fn side_title(&self, album: Album) -> String {
        row_title(album).map_or_else(|| self.album_title(album), str::to_string)
    }

    /// A text card, the editor or a slideshow is up: a click in the source list does not pull the pane away from it.
    fn side_blocked(&self) -> bool {
        self.entry.is_some() || self.editor.is_some() || self.slideshow.is_some()
    }

    /// A touch on the desktop window: one that began in the source list is the list's (None); the rest is returned
    /// in the pane's coordinates.
    pub(super) fn side_touch(&mut self, ui: &Ui, ev: Event) -> Option<Event> {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            other => return Some(other),
        };
        if matches!(ev, Event::Down(..)) {
            self.side_down = x < self.sidebar as f32;
        }
        if !self.side_down {
            let x = x - self.sidebar as f32;
            return Some(match ev {
                Event::Down(..) => Event::Down(x, y),
                Event::Move(..) => Event::Move(x, y),
                _ => Event::Up(x, y),
            });
        }
        self.dirty = true;
        let pt = ui.pt(1.0).max(0.01);
        let row = self.side_layout(ui).hit(x / pt, y / pt);
        match ev {
            Event::Down(..) => self.side_press = row,
            Event::Up(..) => {
                self.side_down = false;
                let pressed = self.side_press.take();
                if let Some(row) = pressed.filter(|p| Some(*p) == row) {
                    if !self.side_blocked() {
                        self.side_activate(ui, row);
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// A click on a source list row: its album's grid, or a new album.
    fn side_activate(&mut self, ui: &Ui, row: Row) {
        match row {
            Row::Album(album) => self.select_album(ui, album),
            Row::NewAlbum => {
                if let Some(i) = self.new_album() {
                    self.rebuild(ui);
                    self.select_album(ui, Album::User(i as u8));
                }
            }
        }
    }

    /// The toolbar's title: the selection, the search, the date filter or the album.
    fn toolbar_title(&self) -> String {
        if self.selection.active {
            return match self.selection.len() {
                0 => "Select Items".to_string(),
                n => format!("{n} Selected"),
            };
        }
        if !self.search.is_empty() {
            let shown: String = self.search.chars().take(18).collect();
            return format!("Search: {shown}{}", if self.search.chars().count() > 18 { "\u{2026}" } else { "" });
        }
        if self.showing_pick() {
            return "\u{2039} Map".to_string();
        }
        self.filter.as_ref().map_or_else(|| self.side_title(self.album), |f| f.0.clone())
    }

    /// The toolbar laid out (pane points): its title, what a click on the title does, where the title ends and the
    /// controls on the trailing side.
    fn toolbar_layout(&self, ui: &Ui) -> (String, Option<Target>, f32, Vec<(Target, Rect)>) {
        let pt = ui.pt(1.0).max(0.01);
        let title = ui.fit(&self.toolbar_title(), MAC_TITLE3_PT, self.w as f32 * 0.45);
        let title_end = title_x() + ui.measure(&title, MAC_TITLE3_PT, Weight::Semibold) / pt;
        let blank = self.blank_library();
        let target = if self.selection.active {
            Some(Target::SelectAll)
        } else if self.showing_pick() {
            Some(Target::PlacesBack)
        } else if blank {
            None
        } else {
            Some(Target::Filter)
        };
        (title, target, title_end, toolbar(self.w as f32 / pt, blank, title_end))
    }

    /// What a press in the toolbar band (pane px) lands on.
    pub(super) fn toolbar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y < 0.0 || y >= self.top(ui) {
            return None;
        }
        let pt = ui.pt(1.0).max(0.01);
        let (px, py) = (x / pt, y / pt);
        let (_, title_target, title_end, items) = self.toolbar_layout(ui);
        if let Some((t, _)) = items.iter().find(|(_, r)| px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h) {
            return Some(*t);
        }
        if px >= title_x() && px < title_end {
            return title_target;
        }
        None
    }

    /// The pane's toolbar in the unified band: the title on the leading side, the controls on the trailing side.
    pub(super) fn draw_toolbar(&mut self, s: &mut Surface, ui: &Ui) {
        let top = self.top(ui);
        let w = self.w.max(0) as usize;
        s.fill_rect(0, 0, w, top as usize, MATERIAL_TITLEBAR_SOLID.now());
        s.fill_rect(0, (top as usize).saturating_sub(1), w, 1, SEPARATOR_C.now());
        let (title, title_target, _, items) = self.toolbar_layout(ui);
        let mid = self.mid(ui);
        let pressed = self.pressed;
        let link = |t: Target| if pressed == Some(t) { rgba(10, 132, 255, 140) } else { LINK };
        let title_color = match title_target {
            Some(t @ (Target::SelectAll | Target::PlacesBack)) => link(t),
            _ if self.filter.is_some() || !self.search.is_empty() => link(Target::Filter),
            _ => LABEL.now(),
        };
        self.label_left(s, ui, &title, MAC_TITLE3_PT, Weight::Semibold, title_color, ui.pt(title_x()), mid);
        let (zoom_in, zoom_out) = (self.cols > ZOOMS[ZOOMS.len() - 1], self.cols < ZOOMS[0]);
        for (target, r) in items {
            let (x, y, rw, rh) = (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h));
            let (cx, cy) = (x + rw / 2.0, y + rh / 2.0);
            match target {
                Target::SearchOpen => {
                    // search fields are capsules
                    s.fill_rrect(x, y, rw, rh, rh / 2.0, FIELD_BG.now());
                    let (gx, u) = (x + ui.pt(14.0), ui.pt(1.0));
                    let b = (gx - 8.0 * u, cy - 8.0 * u, 16.0 * u, 16.0 * u);
                    glyphs::paint(s, b, SECONDARY, glyphs::ring((gx - u, cy - u), 4.5 * u, 1.4 * u));
                    glyphs::paint(s, b, SECONDARY, glyphs::segment((gx + 2.5 * u, cy + 2.5 * u), (gx + 6.0 * u, cy + 6.0 * u), 1.6 * u));
                    let (text, ink) = if self.search.is_empty() { ("Search".to_string(), SECONDARY) } else { (self.search.clone(), LABEL.now()) };
                    let text = ui.fit(&text, MAC_BODY_PT, (rw - ui.pt(34.0)).max(0.0));
                    self.label_left(s, ui, &text, MAC_BODY_PT, Weight::Regular, ink, x + ui.pt(26.0), cy);
                }
                Target::Select => {
                    let text = if self.selection.active { "Cancel" } else { "Select" };
                    self.label(s, ui, text, MAC_BODY_PT, Weight::Regular, link(Target::Select), cx, cy);
                }
                Target::ZoomIn | Target::ZoomOut => {
                    let on = if target == Target::ZoomIn { zoom_in } else { zoom_out };
                    self.zoom_button(s, ui, cx, cy, target == Target::ZoomIn, if on { link(target) } else { SEPARATOR });
                }
                Target::Album => {
                    // the options menu: the album picker's sorting, New Album and More rows
                    let ink = link(Target::Album);
                    for k in [-1.0f32, 0.0, 1.0] {
                        let dx = cx + k * ui.pt(5.0);
                        glyphs::paint(s, (dx - ui.pt(3.0), cy - ui.pt(3.0), ui.pt(6.0), ui.pt(6.0)), ink, glyphs::disc((dx, cy), ui.pt(1.6)));
                    }
                }
                _ => {}
            }
        }
    }

    /// The source list: the sidebar material, the group headers and the rows, the open album on the selection plate.
    pub(super) fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (self.sidebar.max(0) as usize, self.h.max(0) as usize);
        // with behind-window vibrancy the translucent tint is written over the pixels (the presenter blurs the desktop
        // under the `.sidebar` material the shell declares), else the solid sidebar colour
        let fill = if crate::shell::apphost::vibrancy_available() {
            let c = MATERIAL_SIDEBAR.now();
            rgba((c & 255) as u8, (c >> 8 & 255) as u8, (c >> 16 & 255) as u8, (c >> 24) as u8)
        } else {
            MATERIAL_SIDEBAR_SOLID.now()
        };
        s.fill_rect(0, 0, w, h, fill);
        s.fill_rect(w.saturating_sub(1), 0, 1, h, SEPARATOR_C.now());
        let layout = self.side_layout(ui);
        for &(title, y) in &layout.headers {
            self.label_left(s, ui, title, MAC_GROUP_HEADER_PT, Weight::Semibold, SECONDARY, ui.pt(m::SIDEBAR_INSET + 8.0), ui.pt(y));
        }
        let open = self.album;
        for (row, r) in layout.rows {
            let (x, y, rw, rh) = (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h));
            if y + rh > h as f32 {
                break;
            }
            let on = row == Row::Album(open);
            if on {
                s.fill_rrect(x, y, rw, rh, ui.pt(m::SELECTION_RADIUS), SIDEBAR_SELECTED);
            } else if self.side_press == Some(row) {
                s.fill_rrect(x, y, rw, rh, ui.pt(m::SELECTION_RADIUS), rgba(128, 128, 128, 50));
            }
            let cy = y + rh / 2.0;
            let icon_x = x + ui.pt(m::SIDEBAR_ICON_X - m::SIDEBAR_INSET + m::SIDEBAR_ICON / 2.0);
            side_icon(s, row, (icon_x, cy), ui.pt(m::SIDEBAR_ICON) * 0.8, ui.pt(1.3), if on { WHITE } else { TINT });
            let (text, ink) = match row {
                Row::Album(album) => (self.side_title(album), if on { WHITE } else { LABEL.now() }),
                Row::NewAlbum => ("New Album".to_string(), LINK),
            };
            let label_x = x + ui.pt(m::SIDEBAR_LABEL_X - m::SIDEBAR_INSET);
            let text = ui.fit(&text, MAC_BODY_PT, (x + rw - label_x - ui.pt(8.0)).max(0.0));
            self.label_left(s, ui, &text, MAC_BODY_PT, Weight::Regular, ink, label_x, cy);
        }
    }

    /// Hands the shell's drag zone the controls in the band (window pt): the title and the toolbar's controls, or the
    /// whole band while the viewer, the editor, a slideshow or an overlay draws its own controls there.
    pub(super) fn publish_toolbar(&self, ui: &Ui) {
        let pt = ui.pt(1.0).max(0.01);
        let side = self.sidebar as f32 / pt;
        let own_chrome = self.viewer.is_some() || self.editor.is_some() || self.slideshow.is_some() || self.album_menu || self.more_menu || self.keywords_open || self.entry.is_some();
        let controls = if own_chrome {
            vec![(false, "Toolbar", Rect { x: side, y: 0.0, w: self.w as f32 / pt, h: BAR_PT })]
        } else {
            let (_, title_target, title_end, items) = self.toolbar_layout(ui);
            let title = title_target.map(|_| (false, "Title", Rect { x: side + title_x(), y: 0.0, w: (title_end - title_x()).max(0.0), h: BAR_PT }));
            title.into_iter().chain(items.into_iter().map(|(t, r)| (t == Target::SearchOpen, control_label(t), Rect { x: side + r.x, ..r }))).collect()
        };
        publish_toolbar(controls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(l: &Layout) -> Vec<Row> {
        l.rows.iter().map(|(r, _)| *r).collect()
    }

    #[test]
    fn the_source_list_starts_under_the_traffic_lights_with_library_selected_first() {
        let menu = [Album::All, Album::Recents, Album::Favorites, Album::Screenshots, Album::Trash];
        let l = Layout::new(1000.0, 700.0, &groups(&menu, true));
        assert_eq!(l.width, m::SIDEBAR_W, "the split model's sidebar width");
        let (first, r) = l.rows[0];
        assert_eq!(first, Row::Album(Album::All));
        assert_eq!(r.y, BAR_PT, "the first row sits under the band the traffic lights are in");
        assert!(prefs_layout::Frame::traffic_light(0).1 < r.y, "the lights are above the rows");
        assert_eq!((r.x, r.w, r.h), (m::SIDEBAR_INSET, m::SIDEBAR_W - 2.0 * m::SIDEBAR_INSET, m::SIDEBAR_ROW_H));
        assert_eq!(
            rows_of(&l),
            vec![
                Row::Album(Album::All),
                Row::Album(Album::Favorites),
                Row::Album(Album::Recents),
                Row::Album(Album::Screenshots),
                Row::NewAlbum,
                Row::Album(Album::Trash),
            ]
        );
        assert_eq!(l.headers.iter().map(|h| h.0).collect::<Vec<_>>(), vec!["Pinned", "Albums", "Utilities"], "an empty group has no header");
    }

    #[test]
    fn groups_follow_one_another_with_a_header_and_the_group_gap() {
        let menu = [Album::All, Album::Recents, Album::Favorites, Album::Screenshots, Album::Trash, Album::User(0), Album::Memory(0)];
        let l = Layout::new(1000.0, 700.0, &groups(&menu, false));
        let y_of = |row: Row| l.rows.iter().find(|(r, _)| *r == row).map(|(_, r)| r.y).unwrap();
        let pinned = y_of(Row::Album(Album::Favorites));
        assert_eq!(pinned, BAR_PT + m::SIDEBAR_ROW_PITCH + m::SIDEBAR_GROUP_GAP + HEADER_PT);
        assert_eq!(y_of(Row::Album(Album::Recents)) - pinned, m::SIDEBAR_ROW_PITCH);
        let albums = y_of(Row::Album(Album::User(0)));
        assert_eq!(albums, pinned + 3.0 * m::SIDEBAR_ROW_PITCH + m::SIDEBAR_GROUP_GAP + HEADER_PT);
        assert!(!rows_of(&l).contains(&Row::NewAlbum), "no New Album once the albums are full");
        assert!(y_of(Row::Album(Album::Memory(0))) > albums);
        // every header sits above the first row of its group
        for (title, y) in &l.headers {
            assert!(l.rows.iter().any(|(_, r)| r.y > *y && r.y - *y <= HEADER_PT), "{title}");
        }
    }

    #[test]
    fn a_click_lands_on_the_row_under_it_and_nowhere_between_or_beside() {
        let l = Layout::new(1000.0, 700.0, &groups(&[Album::All, Album::Favorites, Album::Trash], true));
        let (_, lib) = l.rows[0];
        assert_eq!(l.hit(lib.x + 5.0, lib.y + 5.0), Some(Row::Album(Album::All)));
        assert_eq!(l.hit(lib.x + 5.0, lib.y + lib.h + 1.0), None, "the 2 pt between rows is no row");
        assert_eq!(l.hit(2.0, lib.y + 5.0), None, "outside the inset");
        assert_eq!(l.hit(lib.x + 5.0, 10.0), None, "the traffic lights' band is no row");
    }

    #[test]
    fn a_narrow_window_keeps_the_sidebar_to_its_share() {
        assert_eq!(width(500.0, 400.0), 200.0);
        assert_eq!(width(0.0, 0.0), 0.0);
    }

    #[test]
    fn the_toolbar_puts_search_on_the_trailing_edge_and_its_items_in_the_band() {
        let bar = toolbar(800.0, false, 120.0);
        assert_eq!(bar.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![Target::SearchOpen, Target::Album, Target::Select, Target::ZoomIn, Target::ZoomOut]);
        let search = bar[0].1;
        assert_eq!(search.x + search.w, 800.0 - EDGE_PT);
        assert_eq!(search.h, SEARCH_H_PT);
        for (t, r) in &bar {
            assert!(r.y >= 0.0 && r.y + r.h <= BAR_PT, "{t:?} is inside the band");
            assert!(r.x > 120.0, "{t:?} is right of the title");
        }
        for pair in bar.windows(2) {
            assert!(pair[1].1.x + pair[1].1.w <= pair[0].1.x - GAP_PT + 0.01, "no overlap");
        }
        assert_eq!(toolbar(800.0, true, 120.0).len(), 2, "an empty library: search and options");
        let narrow = toolbar(300.0, false, 100.0);
        assert!(narrow.len() < 5 && narrow.iter().all(|(_, r)| r.x >= 100.0 + GAP_PT), "a narrow pane drops what does not fit");
    }

    /// E2E finding 9: on the desktop Photos is a full-size window with the source list beside the pane and the
    /// toolbar in the unified band, not the tablet's "< Photos / Library" push navigation.
    #[test]
    fn the_desktop_window_puts_the_source_list_beside_the_pane_and_the_toolbar_in_the_band() {
        use super::super::tests::{frame, photos, tap};
        let (mut p, dir, ui) = photos("desktop-sidebar", 3);
        assert!(!p.desktop && p.sidebar_px() == 0, "a new Photos is the tablet layout");
        assert_eq!(p.top(&ui), content_top(&ui), "the tablet keeps its navigation bar");
        p.set_size(1200, 1920);
        p.set_desktop(&ui, true);
        assert!(sidebar_layout(), "the shell makes the window full-size");
        let pt = ui.pt(1.0);
        let side = p.sidebar_px();
        assert_eq!(side, ui.pt(width(1200.0 / pt, 1920.0 / pt)).round() as i32);
        assert_eq!(p.w, 1200 - side, "the pane is the window less the source list");
        assert_eq!((p.top(&ui), p.mid(&ui)), (ui.pt(BAR_PT), ui.pt(BAR_PT / 2.0)), "the grid starts under the unified band");

        // the frame: the source list column, then the pane with its toolbar band over the grid
        let buf = frame(&mut p, &ui);
        let at = |x: i32, y: f32| buf[y as usize * 1200 + x as usize];
        assert_ne!(at(4, ui.pt(400.0)), BLACK, "the source list is not the pane's black");
        assert_eq!(at(side + 4, ui.pt(4.0)), MATERIAL_TITLEBAR_SOLID.now(), "the pane's toolbar band");
        let controls = toolbar_controls();
        let search = controls.iter().find(|n| n.label == "Search").expect("the search capsule is a control");
        assert_eq!(search.role, bifrost_proto::Role::TextField);
        assert!(search.bounds.x >= side as f32 / pt && search.bounds.y + search.bounds.h <= BAR_PT, "in window points, in the band");

        // a click on a source list row opens that album; nothing in the pane moves
        let l = p.side_layout(&ui);
        let fav = l.rows.iter().find(|(row, _)| *row == Row::Album(Album::Favorites)).expect("Favorites is listed").1;
        tap(&mut p, &ui, ui.pt(fav.x + fav.w / 2.0), ui.pt(fav.y + fav.h / 2.0), 100.0);
        assert_eq!(p.album(), Album::Favorites);
        assert!(!p.album_menu, "no picker: the list is the navigation");
        let lib = l.rows[0].1;
        tap(&mut p, &ui, ui.pt(lib.x + lib.w / 2.0), ui.pt(lib.y + lib.h / 2.0), 200.0);
        assert_eq!(p.album(), Album::All);

        // the toolbar's controls take clicks at their place in the pane, right of the source list
        let (_, _, _, items) = p.toolbar_layout(&ui);
        let opts = items.iter().find(|(t, _)| *t == Target::Album).expect("the options button").1;
        tap(&mut p, &ui, side as f32 + ui.pt(opts.x + opts.w / 2.0), ui.pt(opts.y + opts.h / 2.0), 300.0);
        assert!(p.album_menu, "the options button opens the album picker's rows");
        p.album_menu = false;
        let field = items.iter().find(|(t, _)| *t == Target::SearchOpen).expect("the search capsule").1;
        tap(&mut p, &ui, side as f32 + ui.pt(field.x + field.w / 2.0), ui.pt(field.y + field.h / 2.0), 400.0);
        assert!(p.entry.is_some(), "the search capsule opens the search");
        tap(&mut p, &ui, ui.pt(fav.x + fav.w / 2.0), ui.pt(fav.y + fav.h / 2.0), 500.0);
        assert_eq!(p.album(), Album::All, "the list waits while the search is being typed");

        // off again (a tablet): the pane is the whole window and the window keeps its title bar
        p.set_desktop(&ui, false);
        assert_eq!((p.sidebar_px(), p.w), (0, 1200));
        assert!(!sidebar_layout() && toolbar_controls().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_fixed_albums_go_by_the_reference_names() {
        assert_eq!(row_title(Album::All), Some("Library"));
        assert_eq!(row_title(Album::Recents), Some("Recently Saved"));
        assert_eq!(row_title(Album::Trash), Some("Recently Deleted"));
        assert_eq!(row_title(Album::Places), Some("Map"));
        assert_eq!(row_title(Album::User(0)), None);
    }

    #[test]
    fn the_toolbar_controls_reach_the_drag_zone_as_nodes() {
        publish_toolbar(vec![(true, "Search", Rect { x: 600.0, y: 12.0, w: 160.0, h: 28.0 }), (false, "Select", Rect { x: 500.0, y: 12.0, w: 60.0, h: 28.0 })]);
        let nodes = toolbar_controls();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].role, bifrost_proto::Role::TextField);
        assert_eq!(nodes[1].role, bifrost_proto::Role::Button);
        assert_eq!(nodes[0].bounds.x, 600.0);
        set_sidebar_layout(false);
        assert!(toolbar_controls().is_empty() && !sidebar_layout());
        set_sidebar_layout(true);
        assert!(sidebar_layout());
        set_sidebar_layout(false);
    }
}
