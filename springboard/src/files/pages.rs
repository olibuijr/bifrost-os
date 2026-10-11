//! The pages of the Files navigation stack and their geometry in points,
//! as measured on the bifrost (nav bar, search field, crumb/sort row, list
//! rows, icon grid), with hit testing for every tappable part.

use super::fs::{self, Entry, Kind, Location, Sort};
use super::text::TextView;
use crate::deildu::scroll::Scroller;
use crate::gfx::Image;
pub use crate::scene::STATUS_PT;
use crate::shell::textfield::TextField;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use std::cell::{Ref, RefCell};
use std::path::{Path, PathBuf};

pub const MARGIN_PT: f32 = 20.0;
pub const NAV_PT: f32 = 44.0;
/// Vertical centre of the nav bar.
pub const NAV_MID_PT: f32 = STATUS_PT + NAV_PT / 2.0;
pub const SEARCH_TOP_PT: f32 = STATUS_PT + NAV_PT + 6.0;
pub const SEARCH_H_PT: f32 = 36.0;
/// The breadcrumb and sort row under the search field.
pub const CRUMB_TOP_PT: f32 = SEARCH_TOP_PT + SEARCH_H_PT + 8.0;
pub const CRUMB_H_PT: f32 = 28.0;
pub const CONTENT_TOP_PT: f32 = CRUMB_TOP_PT + CRUMB_H_PT + 4.0;
pub const ROW_PT: f32 = 60.0;
pub const LIST_ICON_PT: f32 = 36.0;
pub const GRID_COLS: usize = 5;
pub const CELL_H_PT: f32 = 130.0;
pub const FOOTER_PT: f32 = 36.0;
/// Rows of the sidebar list.
pub const LOC_ROW_PT: f32 = 44.0;
/// Extra space the "Tags" header takes between the locations and the tag rows.
pub const TAGS_GAP_PT: f32 = 48.0;
/// Where the "Locations" header sits (below the large title).
pub const LOC_HEADER_PT: f32 = STATUS_PT + 96.0;

/// Something a touch can land on, across every page and overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Back,
    /// the picker's Cancel
    Cancel,
    /// the picker's Move/Copy
    Confirm,
    ViewToggle,
    NewFolder,
    /// enters or leaves select mode
    Select,
    /// the select-mode bar: Copy, Move, Delete
    BulkCopy,
    BulkMove,
    BulkDelete,
    Search,
    SearchClear,
    SearchCancel,
    Sort(Sort),
    Crumb(usize),
    Location(usize),
    Entry(usize),
    /// scrollable area with nothing to activate
    Content,
    Keyboard,
    /// Preview's share and markup buttons
    Share,
    Markup,
}

/// The sidebar root: the locations.
pub struct LocationsPage {
    pub locations: Vec<Location>,
    pub title: Image,
    pub header: Image,
    pub labels: Vec<Image>,
    /// (used, total) bytes of this PC's storage and its caption
    pub storage: Option<(u64, u64, Image)>,
    /// index of the first colour-tag row (the "Tags" section starts there)
    pub tags_from: usize,
    pub tags_header: Image,
}

impl LocationsPage {
    pub fn new(ui: &Ui, locations: Vec<Location>) -> Self {
        let tags_from = locations.iter().position(|l| matches!(l.glyph, fs::Glyph::Tag(_))).unwrap_or(locations.len());
        let labels = locations.iter().map(|l| ui.text(l.name, 17.0, Weight::Regular, WHITE)).collect();
        let storage = fs::capacity(fs::home().parent().unwrap_or(Path::new("/"))).filter(|&(total, _)| total > 0).map(|(total, free)| {
            let used = total.saturating_sub(free);
            let text = format!("{} of {} used", crate::shell::storage::human(used), crate::shell::storage::human(total));
            (used, total, ui.text(&text, 13.0, Weight::Regular, SECONDARY))
        });
        LocationsPage { locations, title: ui.text("Files", 34.0, Weight::Bold, WHITE), header: ui.text("Locations", 20.0, Weight::Bold, WHITE), labels, storage, tags_from, tags_header: ui.text("Tags", 20.0, Weight::Bold, WHITE) }
    }

    /// Top of location row `i`, px.
    pub fn row_top(ui: &Ui, i: usize) -> f32 {
        ui.pt(LOC_HEADER_PT + 36.0) + i as f32 * ui.pt(LOC_ROW_PT)
    }

    /// Top of sidebar row `i`, px: the Tags section sits one header lower.
    pub fn top(&self, ui: &Ui, i: usize) -> f32 {
        Self::row_top(ui, i) + if i >= self.tags_from && self.tags_from < self.locations.len() { ui.pt(TAGS_GAP_PT) } else { 0.0 }
    }

    pub fn hit(&self, ui: &Ui, w: i32, x: f32, y: f32) -> Option<Target> {
        let m = ui.pt(MARGIN_PT);
        if x < m || x >= w as f32 - m {
            return None;
        }
        (0..self.locations.len()).find(|&i| y >= self.top(ui, i) && y < self.top(ui, i) + ui.pt(LOC_ROW_PT)).map(Target::Location)
    }
}

/// Rendered text of one entry, made when it first scrolls into view.
pub struct Label {
    pub name: Image,
    /// "date · size" for the list
    pub sub: Image,
    pub grid_name: Image,
    /// "size" or "n items" for the grid
    pub grid_sub: Image,
}

/// A folder listing.
pub struct DirPage {
    pub root: Location,
    pub path: PathBuf,
    pub title: Image,
    pub entries: Vec<Entry>,
    /// one per entry, rendered on first sight (so drawing needs no `&mut`)
    pub labels: RefCell<Vec<Option<Label>>>,
    /// indices into `entries` matching the search, in sort order
    pub shown: Vec<usize>,
    pub loading: bool,
    pub error: Option<Image>,
    pub scroll: Scroller,
    pub query: TextField,
    pub searching: bool,
    /// the breadcrumb from the root, rendered
    pub crumbs: Vec<(Image, PathBuf)>,
    pub footer: Option<Image>,
    pub available: Option<u64>,
    /// listing generation, so stale results are dropped
    pub gen: u64,
}

impl DirPage {
    pub fn new(ui: &Ui, root: &Location, path: &Path) -> Self {
        let trail = fs::breadcrumb(root, path);
        let n = trail.len();
        // the current folder (last crumb) is white, the links grey
        let crumbs = trail.into_iter().enumerate().map(|(i, (name, p))| (ui.text(&ui.fit(&name, 13.0, ui.pt(160.0)), 13.0, Weight::Regular, if i + 1 == n { WHITE } else { SECONDARY }), p)).collect();
        let name = if path == root.path { root.name.to_string() } else { fs::title_of(path) };
        let mut query = TextField::new(ui, "Search", 17.0, false, 64);
        query.focused = false;
        DirPage {
            root: root.clone(),
            path: path.to_path_buf(),
            title: ui.text(&ui.fit(&name, 17.0, ui.pt(360.0)), 17.0, Weight::Semibold, WHITE),
            entries: vec![],
            labels: RefCell::new(vec![]),
            shown: vec![],
            loading: true,
            error: None,
            scroll: Scroller::new(),
            query,
            searching: false,
            crumbs,
            footer: None,
            available: None,
            gen: 0,
        }
    }

    /// The display name shown in the nav bar and as the Back label above.
    pub fn name(&self) -> String {
        if self.path == self.root.path {
            self.root.name.to_string()
        } else {
            fs::title_of(&self.path)
        }
    }

    /// Installs a fresh listing, sorted and filtered.
    pub fn set_entries(&mut self, ui: &Ui, mut entries: Vec<Entry>, sort: Sort, available: Option<u64>) {
        fs::sort(&mut entries, sort);
        self.labels = RefCell::new((0..entries.len()).map(|_| None).collect());
        self.entries = entries;
        self.loading = false;
        self.error = None;
        self.available = available;
        self.refilter(ui);
    }

    pub fn set_error(&mut self, ui: &Ui, text: &str) {
        self.loading = false;
        self.entries.clear();
        self.labels.get_mut().clear();
        self.shown.clear();
        self.error = Some(ui.text(&ui.fit(text, 15.0, ui.pt(600.0)), 15.0, Weight::Regular, SECONDARY));
        self.footer = None;
    }

    pub fn resort(&mut self, ui: &Ui, sort: Sort) {
        fs::sort(&mut self.entries, sort);
        self.labels = RefCell::new((0..self.entries.len()).map(|_| None).collect());
        self.refilter(ui);
    }

    /// Re-applies the search query to the entries.
    pub fn refilter(&mut self, ui: &Ui) {
        self.shown = fs::search(&self.entries, self.query.text());
        let n = self.shown.len();
        let items = format!("{n} item{}", if n == 1 { "" } else { "s" });
        let text = match self.available {
            Some(b) => format!("{items}, {} available", fs::size_text(b)),
            None => items,
        };
        self.footer = Some(ui.text(&text, 15.0, Weight::Semibold, SECONDARY));
        self.scroll.jump_top();
    }

    /// The label of entry `i`, rendered on first use.
    pub fn label(&self, ui: &Ui, w: i32, i: usize, now: i64, offset: i32) -> Ref<'_, Label> {
        if self.labels.borrow()[i].is_none() {
            let e = &self.entries[i];
            let items = |n: usize| format!("{n} item{}", if n == 1 { "" } else { "s" });
            let sub = match (e.is_dir, e.items) {
                (true, Some(n)) => format!("{} · {}", fs::date_text(e.mtime, now, offset), items(n)),
                (true, None) => fs::date_text(e.mtime, now, offset),
                (false, _) => format!("{} · {}", fs::date_text(e.mtime, now, offset), fs::size_text(e.size)),
            };
            let grid_sub = match (e.is_dir, e.items) {
                (true, Some(n)) => items(n),
                (true, None) => String::new(),
                (false, _) => fs::size_text(e.size),
            };
            let name_w = w as f32 - ui.pt(MARGIN_PT + 72.0 + 16.0);
            let cell_w = Self::cell_w(ui, w) - ui.pt(8.0);
            self.labels.borrow_mut()[i] = Some(Label {
                name: ui.text(&ui.fit(&e.name, 17.0, name_w), 17.0, Weight::Regular, WHITE),
                sub: ui.text(&ui.fit(&sub, 13.0, name_w), 13.0, Weight::Regular, SECONDARY),
                grid_name: ui.text(&ui.fit(&e.name, 15.0, cell_w), 15.0, Weight::Regular, WHITE),
                grid_sub: ui.text(&grid_sub, 13.0, Weight::Regular, SECONDARY),
            });
        }
        Ref::map(self.labels.borrow(), |l| l[i].as_ref().unwrap())
    }

    pub fn cell_w(ui: &Ui, w: i32) -> f32 {
        (w as f32 - 2.0 * ui.pt(MARGIN_PT)) / GRID_COLS as f32
    }

    /// Height of the listing content in px for the given view.
    pub fn content_h(&self, ui: &Ui, grid: bool) -> f32 {
        if grid {
            self.shown.len().div_ceil(GRID_COLS) as f32 * ui.pt(CELL_H_PT)
        } else {
            self.shown.len() as f32 * ui.pt(ROW_PT)
        }
    }

    /// Keeps the scroll range in step with the content and viewport.
    pub fn fit_scroll(&mut self, ui: &Ui, grid: bool, bottom: f32) {
        let viewport = bottom - ui.pt(CONTENT_TOP_PT);
        self.scroll.set_max(self.content_h(ui, grid) - viewport);
    }

    /// The rect of shown item `k` (index into `shown`) in px, scrolled.
    pub fn item_rect(&self, ui: &Ui, w: i32, grid: bool, k: usize) -> (f32, f32, f32, f32) {
        let top = ui.pt(CONTENT_TOP_PT) - self.scroll.offset();
        if grid {
            let cw = Self::cell_w(ui, w);
            (ui.pt(MARGIN_PT) + (k % GRID_COLS) as f32 * cw, top + (k / GRID_COLS) as f32 * ui.pt(CELL_H_PT), cw, ui.pt(CELL_H_PT))
        } else {
            (0.0, top + k as f32 * ui.pt(ROW_PT), w as f32, ui.pt(ROW_PT))
        }
    }

    /// The items whose rects overlap the viewport, as `shown` indices.
    pub fn visible(&self, ui: &Ui, w: i32, grid: bool, bottom: f32) -> Vec<usize> {
        let top = ui.pt(CONTENT_TOP_PT);
        (0..self.shown.len())
            .filter(|&k| {
                let (_, y, _, h) = self.item_rect(ui, w, grid, k);
                y + h > top && y < bottom
            })
            .collect()
    }

    /// The listing region: below the crumb row, above `bottom`.
    pub fn in_content(ui: &Ui, bottom: f32, y: f32) -> bool {
        y >= ui.pt(CONTENT_TOP_PT) && y < bottom
    }

    pub fn hit(&self, ui: &Ui, w: i32, grid: bool, bottom: f32, x: f32, y: f32) -> Option<Target> {
        if let Some(t) = search_hit(ui, w, self.searching, !self.query.is_empty(), x, y) {
            return Some(t);
        }
        if y >= ui.pt(CRUMB_TOP_PT) && y < ui.pt(CRUMB_TOP_PT + CRUMB_H_PT) {
            if let Some(s) = sort_hit(ui, w, x) {
                return Some(Target::Sort(s));
            }
            let mut cx = ui.pt(MARGIN_PT);
            for (i, (img, _)) in self.crumbs.iter().enumerate() {
                let cw = img.w as f32 + ui.pt(8.0);
                if x >= cx - ui.pt(4.0) && x < cx + cw && i + 1 < self.crumbs.len() {
                    return Some(Target::Crumb(i));
                }
                cx += cw + ui.pt(14.0);
            }
            return None;
        }
        if !Self::in_content(ui, bottom, y) {
            return None;
        }
        for k in self.visible(ui, w, grid, bottom) {
            let (rx, ry, rw, rh) = self.item_rect(ui, w, grid, k);
            if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                return Some(Target::Entry(self.shown[k]));
            }
        }
        Some(Target::Content)
    }
}

/// Px rect of the search field.
pub fn search_rect(ui: &Ui, w: i32, searching: bool) -> (f32, f32, f32, f32) {
    let m = ui.pt(MARGIN_PT);
    let cancel = if searching { ui.pt(70.0) } else { 0.0 };
    (m, ui.pt(SEARCH_TOP_PT), w as f32 - 2.0 * m - cancel, ui.pt(SEARCH_H_PT))
}

/// Centre of the clear (x) button inside the search field.
pub fn clear_centre(ui: &Ui, w: i32, searching: bool) -> (f32, f32) {
    let (x, y, fw, fh) = search_rect(ui, w, searching);
    (x + fw - ui.pt(18.0), y + fh / 2.0)
}

fn search_hit(ui: &Ui, w: i32, searching: bool, has_text: bool, x: f32, y: f32) -> Option<Target> {
    let (sx, sy, sw, sh) = search_rect(ui, w, searching);
    if y < sy || y >= sy + sh {
        return None;
    }
    if x >= sx && x < sx + sw {
        let (cx, _) = clear_centre(ui, w, searching);
        return Some(if has_text && x >= cx - ui.pt(20.0) { Target::SearchClear } else { Target::Search });
    }
    (searching && x >= sx + sw).then_some(Target::SearchCancel)
}

/// Width of one sort label slot.
pub const SORT_SLOT_PT: f32 = 64.0;

/// Left edge of sort option `i`'s slot, px.
pub fn sort_x(ui: &Ui, w: i32, i: usize) -> f32 {
    w as f32 - ui.pt(MARGIN_PT) - (Sort::ALL.len() - i) as f32 * ui.pt(SORT_SLOT_PT)
}

fn sort_hit(ui: &Ui, w: i32, x: f32) -> Option<Sort> {
    Sort::ALL.iter().enumerate().find(|(i, _)| x >= sort_x(ui, w, *i) && x < sort_x(ui, w, *i) + ui.pt(SORT_SLOT_PT)).map(|(_, s)| *s)
}

/// The nav bar's buttons (shared by every page): back/cancel on the left,
/// the new-folder and view toggles plus confirm on the right.
pub fn nav_hit(ui: &Ui, w: i32, left: Option<Target>, right: &[Target], x: f32, y: f32) -> Option<Target> {
    let top = ui.pt(STATUS_PT);
    if y < top || y >= top + ui.pt(NAV_PT) {
        return None;
    }
    if x < ui.pt(MARGIN_PT + 110.0) {
        return left;
    }
    let mut edge = w as f32 - ui.pt(MARGIN_PT);
    for &t in right {
        let bw = ui.pt(if matches!(t, Target::Confirm | Target::Select) { 72.0 } else { 44.0 });
        if x >= edge - bw && x < edge + ui.pt(10.0) {
            return Some(t);
        }
        edge -= bw + ui.pt(12.0);
    }
    None
}

/// The x of right-side nav button `i` (0 = rightmost), px.
pub fn nav_right_x(ui: &Ui, w: i32, i: usize) -> f32 {
    w as f32 - ui.pt(MARGIN_PT) - ui.pt(44.0) - i as f32 * ui.pt(56.0)
}

/// Centre x of the text button (Select/Done) in right slot `i`, px; it
/// matches the hit range `nav_hit` gives such a button.
pub fn nav_text_x(ui: &Ui, w: i32, i: usize) -> f32 {
    w as f32 - ui.pt(MARGIN_PT) - i as f32 * ui.pt(56.0) - ui.pt(36.0) + ui.pt(5.0)
}

/// The select-mode bar in the footer: which button `x` is over.
pub fn bulk_hit(ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Target> {
    if y < bottom || y >= bottom + ui.pt(FOOTER_PT) {
        return None;
    }
    Some(match (x / (w as f32 / 3.0)) as usize {
        0 => Target::BulkCopy,
        1 => Target::BulkMove,
        _ => Target::BulkDelete,
    })
}

/// What a preview page shows.
pub enum Body {
    Loading,
    Text(TextView),
    Image(Image),
    /// label/value rows of the info sheet
    Info(Vec<(Image, Image)>),
    Error(Image),
}

/// A file preview or info sheet.
pub struct PreviewPage {
    pub entry: Entry,
    pub path: PathBuf,
    pub kind: Kind,
    pub title: Image,
    pub body: Body,
    pub scroll: Scroller,
}

impl PreviewPage {
    pub fn new(ui: &Ui, entry: &Entry) -> Self {
        PreviewPage { entry: entry.clone(), path: entry.path.clone(), kind: entry.kind, title: ui.text(&ui.fit(&entry.name, 17.0, ui.pt(360.0)), 17.0, Weight::Semibold, WHITE), body: Body::Loading, scroll: Scroller::new() }
    }

    /// The info sheet rows for an entry.
    pub fn info_rows(ui: &Ui, w: i32, e: &Entry, kind: Kind, now: i64, offset: i32) -> Vec<(Image, Image)> {
        let value_w = w as f32 - 2.0 * ui.pt(MARGIN_PT) - ui.pt(140.0);
        let size = if e.is_dir { e.items.map_or(String::from("—"), |n| format!("{n} items")) } else { fs::size_text(e.size) };
        let where_ = e.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        [("Name", e.name.clone()), ("Kind", fs::kind_description(&e.name, e.is_dir, kind).to_string()), ("Size", size), ("Modified", fs::date_text(e.mtime, now, offset)), ("Where", where_)]
            .into_iter()
            .map(|(k, v)| (ui.text(k, 17.0, Weight::Regular, WHITE), ui.text(&ui.fit(&v, 17.0, value_w), 17.0, Weight::Regular, SECONDARY)))
            .collect()
    }

    pub fn content_h(&self, ui: &Ui) -> f32 {
        match &self.body {
            Body::Text(t) => t.content_h(ui),
            Body::Info(rows) => rows.len() as f32 * ui.pt(44.0) + ui.pt(40.0),
            _ => 0.0,
        }
    }

    pub fn fit_scroll(&mut self, ui: &Ui, bottom: f32) {
        let h = self.content_h(ui);
        self.scroll.set_max(h - (bottom - ui.pt(STATUS_PT + NAV_PT)));
    }
}

/// One page of the stack. A `DirPage` is much larger than the others, but
/// a stack holds a handful of pages, so they are not boxed.
#[allow(clippy::large_enum_variant)]
pub enum Page {
    Locations(LocationsPage),
    Dir(DirPage),
    Preview(PreviewPage),
}

impl Page {
    /// The nav-bar title's text image.
    pub fn title(&self) -> Option<&Image> {
        match self {
            Page::Locations(_) => None,
            Page::Dir(d) => Some(&d.title),
            Page::Preview(p) => Some(&p.title),
        }
    }

    /// The name a Back button to this page shows.
    pub fn back_label(&self) -> String {
        match self {
            Page::Locations(_) => "Files".into(),
            Page::Dir(d) => d.name(),
            Page::Preview(p) => fs::title_of(&p.path),
        }
    }

    pub fn scroller(&mut self) -> Option<&mut Scroller> {
        match self {
            Page::Locations(_) => None,
            Page::Dir(d) => Some(&mut d.scroll),
            Page::Preview(p) => Some(&mut p.scroll),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry { kind: fs::kind_of(name, is_dir), name: name.into(), path: PathBuf::from("/x").join(name), is_dir, size, mtime: 0, items: is_dir.then_some(3) }
    }

    fn page(ui: &Ui, n: usize) -> DirPage {
        let root = fs::locations().remove(6);
        let mut d = DirPage::new(ui, &root, Path::new("/system/bin"));
        let entries = (0..n).map(|i| entry(&format!("f{i}.txt"), i % 3 == 0, i as u64)).collect();
        d.set_entries(ui, entries, Sort::Name, Some(5_000_000_000));
        d
    }

    #[test]
    fn locations_rows_hit() {
        let ui = ui();
        let p = LocationsPage::new(&ui, fs::locations());
        let y0 = LocationsPage::row_top(&ui, 0);
        assert_eq!(p.hit(&ui, 1200, 600.0, y0 + 5.0), Some(Target::Location(0)));
        assert_eq!(p.hit(&ui, 1200, 600.0, y0 + ui.pt(LOC_ROW_PT) * 4.5), Some(Target::Location(4)));
        assert_eq!(p.hit(&ui, 1200, 600.0, y0 + ui.pt(LOC_ROW_PT) * 7.5), None);
        assert_eq!(p.hit(&ui, 1200, 5.0, y0 + 5.0), None);
        assert_eq!(p.hit(&ui, 1200, 600.0, y0 - 5.0), None);
    }

    #[test]
    fn dir_page_lists_filters_and_hits() {
        let ui = ui();
        let mut d = page(&ui, 40);
        assert_eq!(d.crumbs.len(), 3, "Root › system › bin");
        assert_eq!(d.name(), "bin");
        assert_eq!(d.shown.len(), 40);
        assert!(d.footer.is_some() && !d.loading);
        let bottom = 1920.0 - ui.pt(28.0 + FOOTER_PT);
        // list: rows of ROW_PT from CONTENT_TOP
        let top = ui.pt(CONTENT_TOP_PT);
        assert_eq!(d.hit(&ui, 1200, false, bottom, 600.0, top + 5.0), Some(Target::Entry(d.shown[0])));
        assert_eq!(d.hit(&ui, 1200, false, bottom, 600.0, top + ui.pt(ROW_PT) * 2.5), Some(Target::Entry(d.shown[2])));
        assert_eq!(d.hit(&ui, 1200, false, bottom, 600.0, bottom + 5.0), None, "below the content");
        let vis = d.visible(&ui, 1200, false, bottom);
        assert_eq!(vis[0], 0);
        assert!(vis.len() < 40 && vis.len() > 10);
        // grid: 5 across
        let (x, y, w, h) = d.item_rect(&ui, 1200, true, 7);
        assert_eq!((x, y), (ui.pt(MARGIN_PT) + 2.0 * DirPage::cell_w(&ui, 1200), top + ui.pt(CELL_H_PT)));
        assert_eq!(d.hit(&ui, 1200, true, bottom, x + w / 2.0, y + h / 2.0), Some(Target::Entry(d.shown[7])));
        // past the last item in the grid is plain content
        d.fit_scroll(&ui, true, bottom);
        assert_eq!(d.scroll.offset(), 0.0);
        assert!(d.content_h(&ui, true) == 8.0 * ui.pt(CELL_H_PT));
        d.fit_scroll(&ui, false, bottom);
        assert!(d.content_h(&ui, false) > bottom);
        // search field and sort row
        let (sx, sy, sw, sh) = search_rect(&ui, 1200, false);
        assert_eq!(d.hit(&ui, 1200, false, bottom, sx + 10.0, sy + sh / 2.0), Some(Target::Search));
        assert_eq!(d.hit(&ui, 1200, false, bottom, sx + sw + 10.0, sy + sh / 2.0), None);
        let cy = ui.pt(CRUMB_TOP_PT + CRUMB_H_PT / 2.0);
        assert_eq!(d.hit(&ui, 1200, false, bottom, sort_x(&ui, 1200, 1) + 5.0, cy), Some(Target::Sort(Sort::Date)));
        assert_eq!(d.hit(&ui, 1200, false, bottom, ui.pt(MARGIN_PT) + 5.0, cy), Some(Target::Crumb(0)));
        let last_x = ui.pt(MARGIN_PT) + d.crumbs[..2].iter().map(|(i, _)| i.w as f32 + ui.pt(8.0 + 14.0)).sum::<f32>() + 5.0;
        assert_eq!(d.hit(&ui, 1200, false, bottom, last_x, cy), None, "the current folder is not a link");
        // filtering
        d.query.set_text(&ui, "f1");
        d.refilter(&ui);
        assert_eq!(d.shown.len(), 11, "f1, f10..f19");
        d.searching = true;
        let (sx, sy, sw, sh) = search_rect(&ui, 1200, true);
        assert_eq!(d.hit(&ui, 1200, false, bottom, sx + sw + 20.0, sy + sh / 2.0), Some(Target::SearchCancel));
        assert_eq!(d.hit(&ui, 1200, false, bottom, sx + sw - 10.0, sy + sh / 2.0), Some(Target::SearchClear));
        {
            let l = d.label(&ui, 1200, 0, 0, 0);
            assert!(l.name.w > 0 && l.sub.w > 0 && l.grid_name.w > 0 && l.grid_sub.w > 0);
        }
        d.set_error(&ui, "Permission denied");
        assert!(d.error.is_some() && d.shown.is_empty());
        let empty = page(&ui, 0);
        assert_eq!(empty.hit(&ui, 1200, false, bottom, 600.0, top + 5.0), Some(Target::Content));
    }

    #[test]
    fn nav_bar_hits_left_and_right_buttons() {
        let ui = ui();
        let mid = ui.pt(NAV_MID_PT);
        let right = [Target::ViewToggle, Target::NewFolder];
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Back), &right, 40.0, mid), Some(Target::Back));
        assert_eq!(nav_hit(&ui, 1200, None, &right, 40.0, mid), None);
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Back), &right, nav_right_x(&ui, 1200, 0) + 10.0, mid), Some(Target::ViewToggle));
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Back), &right, nav_right_x(&ui, 1200, 1) + 10.0, mid), Some(Target::NewFolder));
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Back), &right, 600.0, mid), None);
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Back), &right, 40.0, mid + 500.0), None);
        assert_eq!(nav_hit(&ui, 1200, Some(Target::Cancel), &[Target::Confirm], 1150.0, mid), Some(Target::Confirm));
    }

    #[test]
    fn preview_page_rows_and_scroll() {
        let ui = ui();
        let e = entry("a.bin", false, 1234);
        let mut p = PreviewPage::new(&ui, &e);
        assert!(matches!(p.body, Body::Loading));
        let rows = PreviewPage::info_rows(&ui, 1200, &e, Kind::Other, 0, 0);
        assert_eq!(rows.len(), 5);
        p.body = Body::Info(rows);
        p.fit_scroll(&ui, 1800.0);
        assert_eq!(p.scroll.offset(), 0.0);
        assert!(p.content_h(&ui) > 0.0);
        let mut pg = Page::Preview(p);
        assert_eq!(pg.back_label(), "a.bin");
        assert!(pg.title().is_some() && pg.scroller().is_some());
        let mut root = Page::Locations(LocationsPage::new(&ui, fs::locations()));
        assert_eq!(root.back_label(), "Files");
        assert!(root.title().is_none() && root.scroller().is_none());
    }
}
