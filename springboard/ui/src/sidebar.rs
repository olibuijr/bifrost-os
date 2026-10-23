//! A source list (`NativeOutlineView` source-list style, a `NativeTableView` sidebar): section headers and selectable rows with
//! optional counts, scrolled inside a column. Pure model: it lays rows out, hit-tests them and moves the selection.
use crate::Rect;

#[derive(Clone, Debug, PartialEq)]
pub enum SidebarItem {
    Header(String),
    /// `symbol` is an the system symbol set name drawn before the label in the accent colour (Files's blue sidebar glyphs).
    /// `dot` is a filled colour circle drawn in place of the glyph (Files Tags), packed like the renderer's colours.
    Row {
        id: u32,
        label: String,
        count: Option<u32>,
        enabled: bool,
        symbol: Option<String>,
        dot: Option<u32>,
        /// A second, secondary line under the label (a two-line `NativeTableCellView`); the row is `two_line_h` tall.
        subtitle: Option<String>,
        /// Trailing text in place of or before a count ("12°", "09:41").
        trailing: Option<String>,
        /// A round monogram avatar leading the row (Contacts, Messages): initials on a filled circle, packed colour.
        avatar: Option<(String, u32)>,
        /// The glyph is drawn in the secondary grey, not the accent (Files's Locations rows: home, computer, trash).
        neutral: bool,
    },
    /// A collapsible section header (`NativeOutlineView` group row with a disclosure button). `collapsed` is the app's
    /// state: rows under a collapsed section are hidden until the next header or section.
    Section { id: u32, label: String, collapsed: bool },
}

impl SidebarItem {
    pub fn header(label: &str) -> Self {
        SidebarItem::Header(label.into())
    }
    pub fn row(id: u32, label: &str) -> Self {
        SidebarItem::Row { id, label: label.into(), count: None, enabled: true, symbol: None, dot: None, subtitle: None, trailing: None, avatar: None, neutral: false }
    }
    fn map_row(mut self, f: impl FnOnce(&mut Option<String>, &mut Option<u32>, &mut Option<u32>, &mut bool, &mut Option<String>, &mut Option<String>, &mut Option<(String, u32)>)) -> Self {
        if let SidebarItem::Row { symbol, dot, count, enabled, subtitle, trailing, avatar, .. } = &mut self {
            f(symbol, dot, count, enabled, subtitle, trailing, avatar);
        }
        self
    }
    /// A row with a glyph (`NativeImage(systemSymbolName:)`).
    pub fn with_symbol(self, name: &str) -> Self {
        self.map_row(|sy, _, _, _, _, _, _| *sy = Some(name.into()))
    }
    /// A row led by a filled colour dot instead of a glyph (Files's Tags section).
    pub fn with_dot(self, color: u32) -> Self {
        self.map_row(|_, d, _, _, _, _, _| *d = Some(color))
    }
    pub fn with_count(self, n: u32) -> Self {
        self.map_row(|_, _, c, _, _, _, _| *c = Some(n))
    }
    pub fn disabled(self) -> Self {
        self.map_row(|_, _, _, e, _, _, _| *e = false)
    }
    /// A second line under the label (two-line rows: Mail, Podcasts, Weather locations).
    pub fn with_subtitle(self, text: &str) -> Self {
        self.map_row(|_, _, _, _, st, _, _| *st = Some(text.into()))
    }
    /// Trailing secondary text ("12°", a time), drawn where a count goes (before it when both are set).
    pub fn with_trailing(self, text: &str) -> Self {
        self.map_row(|_, _, _, _, _, t, _| *t = Some(text.into()))
    }
    /// A monogram avatar (`initials` on a `color` disc) leading the row, in place of a glyph or dot.
    pub fn with_avatar(self, initials: &str, color: u32) -> Self {
        self.map_row(|_, _, _, _, _, _, a| *a = Some((initials.into(), color)))
    }
    /// Draws the row's glyph in the secondary grey instead of the accent. Files tints Favorites and keeps Locations grey.
    pub fn neutral(mut self) -> Self {
        if let SidebarItem::Row { neutral, .. } = &mut self {
            *neutral = true;
        }
        self
    }
    /// A collapsible section header with disclosure (`NativeOutlineView`); the app owns `collapsed`.
    pub fn section(id: u32, label: &str, collapsed: bool) -> Self {
        SidebarItem::Section { id, label: label.into(), collapsed }
    }
}

/// A source-list row length `pt` at sidebar icon `scale` (Settings > Appearance > Sidebar Icon Size,
/// `bifrost_render::ui::sidebar_icon_scale`; medium is 1): rows, their glyphs and selection plates scale together.
pub fn scaled_row(pt: f32, scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { pt * scale } else { pt }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceList {
    pub items: Vec<SidebarItem>,
    selected: Option<u32>,
    pub row_h: f32,
    pub header_h: f32,
    /// Height of a row with a subtitle (`with_subtitle`); `new` sets `row_h + 16`.
    pub two_line_h: f32,
}

impl SourceList {
    pub fn new(items: Vec<SidebarItem>, row_h: f32, header_h: f32) -> Self {
        SourceList { items, selected: None, row_h, header_h, two_line_h: row_h + 16.0 }
    }
    pub fn selected(&self) -> Option<u32> {
        self.selected
    }
    /// Sets the selection without validation, or clears it with `None` (the shown place is not in the list).
    pub fn set_selected(&mut self, id: Option<u32>) {
        self.selected = id;
    }
    /// Selects the row with `id` if it exists and is enabled; true when the selection changed.
    pub fn select(&mut self, id: u32) -> bool {
        let ok = self.items.iter().any(|i| matches!(i, SidebarItem::Row { id: r, enabled: true, .. } if *r == id));
        if !ok || self.selected == Some(id) {
            return false;
        }
        self.selected = Some(id);
        true
    }
    fn height_of(&self, item: &SidebarItem) -> f32 {
        match item {
            SidebarItem::Header(_) | SidebarItem::Section { .. } => self.header_h,
            // rows follow Sidebar Icon Size live (headers keep their height); drawing and hit-testing both read here
            SidebarItem::Row { subtitle: Some(_), .. } => scaled_row(self.two_line_h, bifrost_render::ui::sidebar_icon_scale()),
            SidebarItem::Row { .. } => scaled_row(self.row_h, bifrost_render::ui::sidebar_icon_scale()),
        }
    }
    /// False for a row under a collapsed section.
    pub fn is_visible(&self, index: usize) -> bool {
        if !matches!(self.items[index], SidebarItem::Row { .. }) {
            return true;
        }
        for item in self.items[..index].iter().rev() {
            match item {
                SidebarItem::Header(_) => return true,
                SidebarItem::Section { collapsed, .. } => return !collapsed,
                _ => {}
            }
        }
        true
    }
    /// The laid-out height of item `index` (0 when hidden in a collapsed section).
    fn height_at(&self, index: usize) -> f32 {
        if self.is_visible(index) { self.height_of(&self.items[index]) } else { 0.0 }
    }
    /// Total height of all items: the scrollable content size.
    pub fn content_height(&self) -> f32 {
        (0..self.items.len()).map(|i| self.height_at(i)).sum()
    }
    /// Rectangle of item `index` for a column `bounds`, scrolled by `scroll` points (zero height when hidden).
    pub fn item_rect(&self, index: usize, bounds: Rect, scroll: f32) -> Rect {
        let y: f32 = (0..index).map(|i| self.height_at(i)).sum();
        Rect { x: bounds.x, y: bounds.y + y - scroll, w: bounds.w, h: self.height_at(index) }
    }
    /// The collapsible section whose header is under `(x, y)` (a click there toggles its disclosure).
    pub fn hit_section(&self, bounds: Rect, scroll: f32, x: f32, y: f32) -> Option<u32> {
        if !bounds.contains(x, y) {
            return None;
        }
        (0..self.items.len()).find_map(|i| match &self.items[i] {
            SidebarItem::Section { id, .. } if self.item_rect(i, bounds, scroll).contains(x, y) => Some(*id),
            _ => None,
        })
    }
    /// Sets section `id` collapsed or expanded; true when it changed. A selected row that becomes hidden stays
    /// selected (Files keeps it), but the arrows skip hidden rows.
    pub fn set_collapsed(&mut self, id: u32, on: bool) -> bool {
        for item in self.items.iter_mut() {
            if let SidebarItem::Section { id: s, collapsed, .. } = item {
                if *s == id && *collapsed != on {
                    *collapsed = on;
                    return true;
                }
            }
        }
        false
    }
    /// Flips section `id`; returns whether it is now collapsed (`None` when there is no such section).
    pub fn toggle_section(&mut self, id: u32) -> Option<bool> {
        let now = self.items.iter().find_map(|i| match i {
            SidebarItem::Section { id: s, collapsed, .. } if *s == id => Some(!*collapsed),
            _ => None,
        })?;
        self.set_collapsed(id, now);
        Some(now)
    }
    /// The enabled row under `(x, y)`; headers and disabled rows do not hit.
    pub fn hit(&self, bounds: Rect, scroll: f32, x: f32, y: f32) -> Option<u32> {
        if !bounds.contains(x, y) {
            return None;
        }
        let mut top = bounds.y - scroll;
        for (k, item) in self.items.iter().enumerate() {
            let h = self.height_at(k);
            if y >= top && y < top + h {
                return match item {
                    SidebarItem::Row { id, enabled: true, .. } => Some(*id),
                    _ => None,
                };
            }
            top += h;
        }
        None
    }
    /// Up and Down arrows: moves to the next enabled row in `delta`'s direction, jumping over headers and disabled
    /// rows, without wrapping. Starts from the first (or last) row when nothing is selected.
    pub fn step(&mut self, delta: isize) -> bool {
        let dir = delta.signum();
        if dir == 0 {
            return false;
        }
        let n = self.items.len() as isize;
        let current = self.selected.and_then(|s| self.items.iter().position(|i| matches!(i, SidebarItem::Row { id, .. } if *id == s)));
        // Scan in place from the current row (no allocation): the first enabled row in the direction of travel.
        let mut i = match current {
            Some(c) => c as isize + dir,
            None if dir > 0 => 0,
            None => n - 1,
        };
        while i >= 0 && i < n {
            if let SidebarItem::Row { id, enabled: true, .. } = &self.items[i as usize] {
                if !self.is_visible(i as usize) {
                    i += dir;
                    continue;
                }
                return self.select(*id);
            }
            i += dir;
        }
        false
    }
    /// The scroll offset that brings the selected row fully into a viewport of `height`, from the current `scroll`.
    pub fn scroll_to_selected(&self, height: f32, scroll: f32) -> f32 {
        let Some(sel) = self.selected else { return scroll };
        let Some(index) = self.items.iter().position(|i| matches!(i, SidebarItem::Row { id, .. } if *id == sel)) else { return scroll };
        let top: f32 = (0..index).map(|i| self.height_at(i)).sum();
        let bottom = top + self.height_at(index);
        if top < scroll { top } else if bottom > scroll + height { bottom - height } else { scroll }
            .clamp(0.0, (self.content_height() - height).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const COL: Rect = Rect { x: 0.0, y: 40.0, w: 200.0, h: 300.0 };

    fn list() -> SourceList {
        SourceList::new(
            vec![
                SidebarItem::header("Library"),
                SidebarItem::row(1, "Today").with_count(3),
                SidebarItem::row(2, "All"),
                SidebarItem::header("My Lists"),
                SidebarItem::row(3, "Work"),
                SidebarItem::row(4, "Archive").disabled(),
                SidebarItem::row(5, "Home"),
            ],
            28.0,
            22.0,
        )
    }

    #[test]
    fn sidebar_icon_size_scales_rows_and_ignores_a_bad_factor() {
        assert_eq!(scaled_row(28.0, 1.0), 28.0, "medium is the measured size");
        assert!(scaled_row(28.0, 0.85) < 28.0 && scaled_row(28.0, 1.2) > 28.0);
        assert_eq!(scaled_row(28.0, 0.0), 28.0);
        assert_eq!(scaled_row(28.0, f32::NAN), 28.0);
    }

    #[test]
    fn rows_lay_out_in_order_with_header_and_row_heights() {
        let l = list();
        assert_eq!(l.content_height(), 2.0 * 22.0 + 5.0 * 28.0);
        assert_eq!(l.item_rect(0, COL, 0.0), Rect { x: 0.0, y: 40.0, w: 200.0, h: 22.0 });
        assert_eq!(l.item_rect(1, COL, 0.0).y, 62.0);
        assert_eq!(l.item_rect(1, COL, 10.0).y, 52.0, "scrolling moves rows up");
    }

    #[test]
    fn hit_finds_enabled_rows_only_and_respects_the_column_and_scroll() {
        let l = list();
        assert_eq!(l.hit(COL, 0.0, 50.0, 70.0), Some(1));
        assert_eq!(l.hit(COL, 0.0, 50.0, 45.0), None, "a header is not selectable");
        assert_eq!(l.hit(COL, 0.0, 50.0, 40.0 + 22.0 + 28.0 * 2.0 + 22.0 + 28.0 + 5.0), None, "a disabled row does not hit");
        assert_eq!(l.hit(COL, 0.0, 250.0, 70.0), None, "outside the column");
        assert_eq!(l.hit(COL, 28.0, 50.0, 62.0), Some(2), "scrolled by one row");
    }

    #[test]
    fn select_ignores_headers_unknown_ids_and_disabled_rows() {
        let mut l = list();
        assert!(l.select(2));
        assert!(!l.select(2), "already selected");
        assert!(!l.select(99) && !l.select(4));
        assert_eq!(l.selected(), Some(2));
    }

    #[test]
    fn arrows_skip_headers_and_disabled_rows_and_do_not_wrap() {
        let mut l = list();
        assert!(l.step(1));
        assert_eq!(l.selected(), Some(1), "nothing selected: Down starts at the first row");
        assert!(l.step(1) && l.step(1));
        assert_eq!(l.selected(), Some(3), "jumped over the My Lists header");
        assert!(l.step(1));
        assert_eq!(l.selected(), Some(5), "jumped over the disabled Archive");
        assert!(!l.step(1), "no wrap at the bottom");
        assert!(l.step(-1) && l.step(-1) && l.step(-1), "5, then 3, 2, 1: the disabled row is skipped");
        assert_eq!(l.selected(), Some(1));
        assert!(!l.step(-1));
        let mut up = list();
        assert!(up.step(-1));
        assert_eq!(up.selected(), Some(5), "nothing selected: Up starts at the last row");
        assert!(!SourceList::new(vec![SidebarItem::header("Empty")], 28.0, 22.0).step(1));
    }

    #[test]
    fn two_line_rows_are_taller_and_carry_subtitle_trailing_and_avatar() {
        let l = SourceList::new(
            vec![SidebarItem::row(1, "Reykjavík").with_subtitle("Cloudy").with_trailing("12°"), SidebarItem::row(2, "Anna").with_avatar("AB", 0xff00_80ff)],
            28.0,
            22.0,
        );
        assert_eq!(l.item_rect(0, COL, 0.0).h, 44.0);
        assert_eq!(l.item_rect(1, COL, 0.0).y, 84.0);
        assert_eq!(l.content_height(), 72.0);
        assert!(matches!(&l.items[0], SidebarItem::Row { subtitle: Some(s), trailing: Some(t), .. } if s == "Cloudy" && t == "12°"));
        assert!(matches!(&l.items[1], SidebarItem::Row { avatar: Some((a, _)), .. } if a == "AB"));
        assert_eq!(SidebarItem::header("H").with_subtitle("x"), SidebarItem::header("H"), "headers ignore row builders");
    }

    #[test]
    fn a_neutral_row_keeps_its_glyph_grey_and_headers_ignore_it() {
        let accent = SidebarItem::row(1, "Desktop").with_symbol("doc");
        let grey = SidebarItem::row(2, "Trash").with_symbol("trash").neutral();
        assert!(matches!(&accent, SidebarItem::Row { neutral: false, .. }), "a row is accent-tinted unless asked");
        assert!(matches!(&grey, SidebarItem::Row { neutral: true, symbol: Some(s), .. } if s == "trash"));
        assert_eq!(SidebarItem::header("H").neutral(), SidebarItem::header("H"), "headers ignore row builders");
    }

    #[test]
    fn a_collapsed_section_hides_its_rows_until_the_next_header() {
        let mut l = SourceList::new(
            vec![
                SidebarItem::section(10, "Favourites", false),
                SidebarItem::row(1, "A"),
                SidebarItem::row(2, "B"),
                SidebarItem::header("Other"),
                SidebarItem::row(3, "C"),
            ],
            28.0,
            22.0,
        );
        assert_eq!(l.hit_section(COL, 0.0, 50.0, 45.0), Some(10));
        assert_eq!(l.toggle_section(10), Some(true));
        assert!(!l.set_collapsed(10, true), "no change");
        assert!(!l.is_visible(1) && !l.is_visible(2) && l.is_visible(4));
        assert_eq!(l.content_height(), 22.0 + 22.0 + 28.0);
        assert_eq!(l.item_rect(1, COL, 0.0).h, 0.0);
        assert_eq!(l.hit(COL, 0.0, 50.0, 40.0 + 44.0 + 5.0), Some(3), "C moved up under the Other header");
        assert!(l.step(1));
        assert_eq!(l.selected(), Some(3), "Down skips the hidden rows");
        assert_eq!(l.toggle_section(10), Some(false));
        assert_eq!(l.toggle_section(99), None);
        assert_eq!(l.hit(COL, 0.0, 50.0, 40.0 + 22.0 + 5.0), Some(1));
    }

    #[test]
    fn scroll_to_selected_brings_the_row_into_view_and_never_overscrolls() {
        let mut l = list();
        l.select(5);
        let off = l.scroll_to_selected(100.0, 0.0);
        let r = l.item_rect(6, Rect { x: 0.0, y: 0.0, w: 10.0, h: 100.0 }, off);
        assert!(r.y >= 0.0 && r.bottom() <= 100.0 + 1e-3, "{r:?}");
        l.select(1);
        assert_eq!(l.scroll_to_selected(100.0, off), 22.0, "scrolls back up to the first row");
        assert_eq!(l.scroll_to_selected(10_000.0, 50.0), 0.0, "a tall viewport needs no scroll");
    }
}
