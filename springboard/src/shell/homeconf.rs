//! What the home screen is made of, as data: which grid cells hold widgets,
//! how apps flow around them into pages, and which pages are hidden or moved.
//! Pure logic; `Shell` turns a `Home` into tiles and `prefs` persists a
//! `HomeConf`. With no widgets and the default page order the result is the
//! plain sequential grid the home screen always had.

use super::prefs::Prefs;
use super::widgets::Kind;
use crate::layout::{COLS, PER_PAGE, ROWS};

/// A corrupt preference can never create more pages than this.
pub const MAX_PAGES: usize = 16;

/// Widgets the gallery offers (the Today renderers they reuse).
pub const GALLERY: [Kind; 4] = [Kind::Clock, Kind::Battery, Kind::Calendar, Kind::Deildu];

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Clock => "Clock",
        Kind::Battery => "Batteries",
        Kind::Calendar => "Calendar",
        Kind::Deildu => "Now Playing",
        Kind::Wifi => "Wi-Fi",
        Kind::Terminal => "Terminal",
        Kind::Photos => "Photos",
        Kind::Weather => "Weather",
        Kind::Stack => "Smart Stack",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 2 x 2 cells
    Small,
    /// 4 x 2 cells
    Medium,
}

impl Size {
    pub fn cells(self) -> (usize, usize) {
        match self {
            Size::Small => (2, 2),
            Size::Medium => (4, 2),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Medium => "medium",
        }
    }

    pub fn from_key(k: &str) -> Option<Size> {
        [Size::Small, Size::Medium].into_iter().find(|s| s.key() == k)
    }

    pub fn toggled(self) -> Size {
        match self {
            Size::Small => Size::Medium,
            Size::Medium => Size::Small,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Size::Small => "Small",
            Size::Medium => "Medium",
        }
    }
}

/// A widget on a page's cell grid (`page` is the flow page id, before reordering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub kind: Kind,
    pub size: Size,
    pub page: usize,
    pub col: usize,
    pub row: usize,
}

impl Placed {
    pub fn covers(&self, col: usize, row: usize) -> bool {
        let (w, h) = self.size.cells();
        (self.col..self.col + w).contains(&col) && (self.row..self.row + h).contains(&row)
    }

    fn overlaps(&self, o: &Placed) -> bool {
        let ((aw, ah), (bw, bh)) = (self.size.cells(), o.size.cells());
        self.page == o.page && self.col < o.col + bw && o.col < self.col + aw && self.row < o.row + bh && o.row < self.row + ah
    }
}

/// True if `p` stays on its page's grid and overlaps no widget in `others`.
pub fn fits(others: &[Placed], p: &Placed) -> bool {
    let (w, h) = p.size.cells();
    p.col + w <= COLS && p.row + h <= ROWS && p.page < MAX_PAGES && !others.iter().any(|o| o.overlaps(p))
}

/// The first free spot for a widget on `page`, scanning rows then columns.
pub fn find_spot(widgets: &[Placed], page: usize, kind: Kind, size: Size) -> Option<Placed> {
    (0..ROWS).flat_map(|row| (0..COLS).map(move |col| Placed { kind, size, page, col, row })).find(|p| fits(widgets, p))
}

/// Switches widget `i` to its other size where it fits; true if it did.
pub fn resize(widgets: &mut [Placed], i: usize) -> bool {
    let Some(&w) = widgets.get(i) else { return false };
    let bigger = Placed { size: w.size.toggled(), ..w };
    let others: Vec<Placed> = widgets.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, p)| *p).collect();
    if fits(&others, &bigger) {
        widgets[i] = bigger;
        true
    } else {
        false
    }
}

/// Persisted home-screen edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HomeConf {
    pub widgets: Vec<Placed>,
    /// flow page ids in display order (ids it does not list follow in id order)
    pub order: Vec<usize>,
    /// flow page ids kept off the home screen
    pub hidden: Vec<usize>,
}

const WIDGETS_KEY: &str = "home.widgets";
const ORDER_KEY: &str = "home.order";
const HIDDEN_KEY: &str = "home.hidden";

fn join_ids(v: &[usize]) -> String {
    v.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",")
}

fn parse_ids(v: Option<&str>) -> Vec<usize> {
    v.unwrap_or("").split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

impl HomeConf {
    pub fn load(prefs: &Prefs) -> HomeConf {
        let mut widgets: Vec<Placed> = Vec::new();
        for rec in prefs.get(WIDGETS_KEY).unwrap_or("").split(';') {
            let f: Vec<&str> = rec.split(':').collect();
            let [kind, size, page, col, row] = f.as_slice() else { continue };
            let (Some(kind), Some(size), Ok(page), Ok(col), Ok(row)) = (Kind::from_key(kind), Size::from_key(size), page.parse(), col.parse(), row.parse()) else { continue };
            let p = Placed { kind, size, page, col, row };
            if fits(&widgets, &p) {
                widgets.push(p);
            }
        }
        HomeConf { widgets, order: parse_ids(prefs.get(ORDER_KEY)), hidden: parse_ids(prefs.get(HIDDEN_KEY)) }
    }

    pub fn save(&self, prefs: &mut Prefs) {
        let widgets = self.widgets.iter().map(|w| format!("{}:{}:{}:{}:{}", w.kind.key(), w.size.key(), w.page, w.col, w.row)).collect::<Vec<_>>().join(";");
        for (k, v) in [(WIDGETS_KEY, widgets), (ORDER_KEY, join_ids(&self.order)), (HIDDEN_KEY, join_ids(&self.hidden))] {
            if let Err(e) = prefs.set(k, &v) {
                eprintln!("springboard: {k}: {e}");
            }
        }
    }
}

/// One flow page: grid app index per cell (None under a widget or past the last app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub slots: Vec<Option<usize>>,
}

/// Lays `apps` out page by page, skipping cells covered by widgets. A page
/// that holds a widget exists even when no app reaches it.
pub fn flow(apps: usize, widgets: &[Placed]) -> Vec<Page> {
    let last = widgets.iter().map(|w| w.page).max();
    let mut pages = Vec::new();
    let mut next = 0;
    while next < apps || last.is_some_and(|l| pages.len() <= l) || pages.is_empty() {
        let p = pages.len();
        let mut slots = vec![None; PER_PAGE];
        for (i, slot) in slots.iter_mut().enumerate() {
            if next < apps && !widgets.iter().any(|w| w.page == p && w.covers(i % COLS, i / COLS)) {
                *slot = Some(next);
                next += 1;
            }
        }
        pages.push(Page { slots });
    }
    pages
}

/// Every page id exactly once: `order` first (unknown ids dropped), the rest after.
pub fn full_order(total: usize, order: &[usize]) -> Vec<usize> {
    let mut seq: Vec<usize> = Vec::new();
    for &p in order {
        if p < total && !seq.contains(&p) {
            seq.push(p);
        }
    }
    let rest: Vec<usize> = (0..total).filter(|p| !seq.contains(p)).collect();
    seq.extend(rest);
    seq
}

/// The pages shown, in order; at least one page always stays.
pub fn visible(total: usize, order: &[usize], hidden: &[usize]) -> Vec<usize> {
    let seq = full_order(total, order);
    let shown: Vec<usize> = seq.iter().copied().filter(|p| !hidden.contains(p)).collect();
    if shown.is_empty() {
        seq.into_iter().take(1).collect()
    } else {
        shown
    }
}

/// Moves the element at `from` to index `to`.
pub fn move_to(v: &mut Vec<usize>, from: usize, to: usize) {
    if from < v.len() && to < v.len() {
        let x = v.remove(from);
        v.insert(to, x);
    }
}

/// The home screen as the shell draws it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Home {
    /// grid app index per cell of the visible pages, trailing empty cells cut
    pub slots: Vec<Option<usize>>,
    /// visible page count
    pub pages: usize,
    /// every flow page id in display order
    pub full: Vec<usize>,
    /// flow page ids shown, in order
    pub shown: Vec<usize>,
    /// (visible page, widget) for the widgets on shown pages
    pub widgets: Vec<(usize, Placed)>,
    /// the flow pages by flow id, for the page editor's thumbnails
    pub flow: Vec<Page>,
}

pub fn build(apps: usize, conf: &HomeConf) -> Home {
    let flowed = flow(apps, &conf.widgets);
    let shown = visible(flowed.len(), &conf.order, &conf.hidden);
    let mut slots: Vec<Option<usize>> = shown.iter().flat_map(|&p| flowed[p].slots.iter().copied()).collect();
    while slots.last() == Some(&None) {
        slots.pop();
    }
    let widgets = conf.widgets.iter().filter_map(|w| shown.iter().position(|&p| p == w.page).map(|v| (v, *w))).collect();
    Home { slots, pages: shown.len(), full: full_order(flowed.len(), &conf.order), shown, widgets, flow: flowed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(kind: Kind, size: Size, page: usize, col: usize, row: usize) -> Placed {
        Placed { kind, size, page, col, row }
    }

    #[test]
    fn without_widgets_apps_fill_pages_in_order() {
        let h = build(PER_PAGE + 3, &HomeConf::default());
        assert_eq!(h.pages, 2);
        assert_eq!(h.slots.len(), PER_PAGE + 3);
        assert!(h.slots.iter().enumerate().all(|(i, s)| *s == Some(i)));
        assert_eq!(build(0, &HomeConf::default()).pages, 1);
    }

    #[test]
    fn apps_flow_around_widgets_onto_later_pages() {
        let conf = HomeConf { widgets: vec![w(Kind::Clock, Size::Medium, 0, 0, 0)], ..HomeConf::default() };
        let pages = flow(PER_PAGE, &conf.widgets);
        assert_eq!(pages.len(), 2);
        assert!(pages[0].slots[..4].iter().all(Option::is_none));
        assert_eq!(pages[0].slots[4], Some(0));
        assert_eq!(pages[0].slots.iter().flatten().count(), PER_PAGE - 8);
        assert_eq!(pages[1].slots[0], Some(PER_PAGE - 8));
    }

    #[test]
    fn a_widget_page_exists_without_apps() {
        let pages = flow(2, &[w(Kind::Battery, Size::Small, 2, 0, 0)]);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[2].slots.iter().flatten().count(), 0);
    }

    #[test]
    fn fits_rejects_overlap_and_overflow() {
        let a = w(Kind::Clock, Size::Medium, 0, 0, 0);
        assert!(!fits(&[a], &w(Kind::Battery, Size::Small, 0, 3, 1)), "overlaps the medium widget");
        assert!(fits(&[a], &w(Kind::Battery, Size::Small, 1, 3, 1)), "other page is free");
        assert!(fits(&[a], &w(Kind::Battery, Size::Small, 0, 0, 2)));
        assert!(!fits(&[], &w(Kind::Clock, Size::Medium, 0, 3, 0)), "4 wide from column 3 leaves the grid");
        assert!(!fits(&[], &w(Kind::Clock, Size::Small, 0, 0, 5)));
    }

    #[test]
    fn find_spot_takes_the_first_free_place() {
        let a = w(Kind::Clock, Size::Medium, 0, 0, 0);
        let p = find_spot(&[a], 0, Kind::Battery, Size::Small).unwrap();
        assert_eq!((p.col, p.row), (0, 2), "a 2-wide widget does not fit beside the medium one on the bifrost's 5-column grid: below it");
        assert_eq!(find_spot(&[], 0, Kind::Clock, Size::Medium).map(|p| (p.col, p.row)), Some((0, 0)));
    }

    #[test]
    fn resize_grows_only_where_there_is_room() {
        let mut v = vec![w(Kind::Battery, Size::Small, 0, 0, 0)];
        assert!(resize(&mut v, 0));
        assert_eq!(v[0].size, Size::Medium);
        let mut blocked = vec![w(Kind::Battery, Size::Small, 0, 0, 0), w(Kind::Calendar, Size::Small, 0, 2, 0)];
        assert!(!resize(&mut blocked, 0));
        assert_eq!(blocked[0].size, Size::Small);
        assert!(resize(&mut v, 0), "and back down");
        assert_eq!(v[0].size, Size::Small);
    }

    #[test]
    fn hidden_pages_drop_out_but_one_always_stays() {
        assert_eq!(visible(3, &[], &[1]), [0, 2]);
        assert_eq!(visible(3, &[2, 0, 1], &[0]), [2, 1]);
        assert_eq!(visible(2, &[], &[0, 1]), [0]);
        assert_eq!(full_order(3, &[2, 9, 2]), [2, 0, 1]);
    }

    #[test]
    fn reordered_and_hidden_pages_change_the_slots() {
        let conf = HomeConf { order: vec![1, 0], hidden: vec![], widgets: vec![] };
        let h = build(PER_PAGE + 1, &conf);
        assert_eq!(h.slots[0], Some(PER_PAGE));
        assert_eq!(h.slots[PER_PAGE], Some(0));
        let hid = build(PER_PAGE + 1, &HomeConf { hidden: vec![0], ..HomeConf::default() });
        assert_eq!(hid.pages, 1);
        assert_eq!(hid.slots, [Some(PER_PAGE)]);
        assert_eq!(hid.flow[0].slots.iter().flatten().count(), PER_PAGE, "hidden apps are still listed for the library");
    }

    #[test]
    fn widgets_follow_their_page_to_its_new_position() {
        let conf = HomeConf { widgets: vec![w(Kind::Battery, Size::Small, 1, 0, 0)], order: vec![1, 0], hidden: vec![] };
        let h = build(PER_PAGE, &conf);
        assert_eq!(h.widgets.len(), 1);
        assert_eq!(h.widgets[0].0, 0);
        let hidden = build(PER_PAGE, &HomeConf { hidden: vec![1], ..conf });
        assert!(hidden.widgets.is_empty());
    }

    #[test]
    fn conf_round_trips_through_prefs_and_ignores_junk() {
        let mut prefs = Prefs::memory();
        let conf = HomeConf { widgets: vec![w(Kind::Clock, Size::Medium, 0, 0, 0), w(Kind::Battery, Size::Small, 1, 2, 2)], order: vec![1, 0], hidden: vec![2] };
        conf.save(&mut prefs);
        assert_eq!(HomeConf::load(&prefs), conf);
        prefs.set("home.widgets", "clock:medium:0:0:0;clock:medium:0:1:0;bogus:small:0:0:0;battery:small:x:0:0;battery:small:0:4:4").unwrap();
        let loaded = HomeConf::load(&prefs);
        assert_eq!(loaded.widgets, [w(Kind::Clock, Size::Medium, 0, 0, 0)], "overlapping, unknown and out-of-grid records are dropped");
    }

    #[test]
    fn move_to_reorders() {
        let mut v = vec![0, 1, 2, 3];
        move_to(&mut v, 3, 1);
        assert_eq!(v, [0, 3, 1, 2]);
        move_to(&mut v, 9, 0);
        assert_eq!(v, [0, 3, 1, 2]);
    }
}
