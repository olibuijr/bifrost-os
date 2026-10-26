//! The Settings search field: a capsule with a magnifier under the root
//! page's large title, its docked keyboard, and the index it searches.
//! The index lists every pane and every setting row of the pages Settings
//! reaches from its root (`index`), matched by name and keyword (`matches`);
//! while there is a query the root page shows the matches (`results`).
//! Other apps reach a pane by name through `find_page`.

use std::collections::{HashMap, HashSet, VecDeque};

use super::fuzzy;
use super::glyphs;
use super::keyboard::{self, Keyboard, Out};
use super::list::{Group, Item, Spec};
use super::pages::{self, Action, PageId};
use super::textfield::TextField;
use super::ui::{home_zone, Ui, FIELD_BG, MAC_BODY_PT, SECONDARY};
use super::Event;
use crate::gfx::Surface;

/// Height of the capsule.
pub const FIELD_PT: f32 = 36.0;
const GLASS_X_PT: f32 = 15.0;
const GLASS_R_PT: f32 = 5.0;
const TEXT_X_PT: f32 = 28.0;

pub struct Search {
    field: TextField,
    keyboard: Keyboard,
    /// the keyboard is up and keys go to the field
    pub active: bool,
    /// the finger went down on the capsule
    pressed_field: bool,
    /// the finger went down on the keyboard
    pressed_keys: bool,
}

impl Search {
    pub fn new(ui: &Ui, w: i32) -> Self {
        let mut field = TextField::new(ui, "Search", MAC_BODY_PT, false, 64);
        field.focused = false;
        let mut keyboard = Keyboard::new(w as f32, ui.s, false);
        keyboard.set_shown(false, 0.0);
        Search { field, keyboard, active: false, pressed_field: false, pressed_keys: false }
    }

    /// No keyboard up, rising or falling: the field and keyboard draw nothing that moves.
    pub fn idle(&self) -> bool {
        !self.active && !self.keyboard.shown() && !self.keyboard.animating()
    }

    pub fn query(&self) -> &str {
        self.field.text()
    }

    pub fn resize(&mut self, ui: &Ui, w: i32) {
        self.keyboard = Keyboard::new(w as f32, ui.s, false);
        self.keyboard.set_shown(self.active, 0.0);
    }

    fn show(&mut self, on: bool, t: f64) {
        self.active = on;
        self.field.focused = on;
        // a desktop has a physical keyboard: focusing the field never raises the on-screen one
        self.keyboard.set_shown(false, t);
    }

    /// Gives the field the keyboard (Tab into it) or takes it away (Tab out of it, leaving the query).
    pub fn focus(&mut self, on: bool, t: f64) {
        self.show(on, t);
    }

    /// Applies a key; true when the query changed.
    pub fn edit(&mut self, ui: &Ui, out: Out, t: f64) -> bool {
        match out {
            Out::Hide | Out::Enter => {
                self.show(false, t);
                false
            }
            out => {
                let before = self.field.text().to_string();
                self.field.edit(ui, &out, t);
                before != self.field.text()
            }
        }
    }

    fn keyboard_top(&self, ui: &Ui, h: i32) -> Option<f32> {
        self.keyboard.shown().then(|| h as f32 - home_zone(ui) - self.keyboard.h())
    }

    /// Routes a touch; `frame` is the capsule's rect on screen when the
    /// root page shows it. Returns (consumed, query changed).
    pub fn touch(&mut self, ui: &Ui, ev: Event, frame: Option<(f32, f32, f32, f32)>, h: i32, t: f64) -> (bool, bool) {
        let inside = |x: f32, y: f32| frame.is_some_and(|(fx, fy, fw, fh)| x >= fx && x < fx + fw && y >= fy && y < fy + fh);
        match ev {
            Event::Down(x, y) => {
                if let Some(top) = self.keyboard_top(ui, h).filter(|&top| y >= top) {
                    self.keyboard.down(x, y - top, t);
                    self.pressed_keys = true;
                    return (true, false);
                }
                self.pressed_field = inside(x, y);
                (self.pressed_field, false)
            }
            Event::Move(x, y) => {
                if self.pressed_keys {
                    let top = self.keyboard_top(ui, h).unwrap_or(0.0);
                    self.keyboard.moved(x, y - top);
                }
                (self.pressed_keys || self.pressed_field, false)
            }
            Event::Up(x, y) => {
                if std::mem::take(&mut self.pressed_keys) {
                    let top = self.keyboard_top(ui, h).unwrap_or(0.0);
                    let changed = self.keyboard.up(x, y - top, t).is_some_and(|o| self.edit(ui, o, t));
                    return (true, changed);
                }
                if std::mem::take(&mut self.pressed_field) {
                    if inside(x, y) {
                        let (fx, ..) = frame.expect("frame");
                        self.show(true, t);
                        self.field.tap(ui, x - fx - ui.pt(TEXT_X_PT), t);
                    }
                    return (true, false);
                }
                // a tap elsewhere puts the keyboard away
                if self.active {
                    self.show(false, t);
                }
                (false, false)
            }
            _ => (false, false),
        }
    }

    /// Repeats held keys and the keyboard slide; (query changed, frame needed).
    pub fn tick(&mut self, ui: &Ui, t: f64) -> (bool, bool) {
        let outs = self.keyboard.tick(t);
        let mut changed = false;
        for out in outs.iter().cloned() {
            changed |= self.edit(ui, out, t);
        }
        let blinking = self.field.next_blink(t).is_some_and(|due| due <= t + 1.0);
        (changed, !outs.is_empty() || blinking || self.keyboard.animating())
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.keyboard.animating() {
            return Some(t);
        }
        self.field.next_wake(t)
    }

    /// The capsule, drawn in `frame`.
    pub fn draw_field(&self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), t: f64) {
        // a capsule: the radius is half the height
        s.fill_rrect(x, y, w, h, h / 2.0, FIELD_BG.now());
        // the focus ring while the field has the keyboard, outside the capsule as the kit draws it (`sheet::focus_ring_rect`)
        if self.active {
            let g = ui.pt(bifrost_ui::sheet::FOCUS_RING_PT);
            s.stroke_rrect(x - g, y - g, w + 2.0 * g, h + 2.0 * g, h / 2.0 + g, ui.pt(2.0), bifrost_render::appearance::accent_now());
        }
        let mid = y + h / 2.0;
        let (gx, gr, stroke) = (x + ui.pt(GLASS_X_PT), ui.pt(GLASS_R_PT), ui.pt(1.8));
        let glass = glyphs::either(glyphs::ring((gx, mid - ui.pt(1.0)), gr, stroke), glyphs::segment((gx + gr * 0.72, mid - ui.pt(1.0) + gr * 0.72), (gx + gr * 1.5, mid - ui.pt(1.0) + gr * 1.5), stroke * 1.2));
        glyphs::paint(s, (gx - gr - stroke, mid - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), SECONDARY, glass);
        self.field.draw(s, ui, (x + ui.pt(TEXT_X_PT), y, w - ui.pt(TEXT_X_PT + 14.0), h), t);
    }

    /// The docked keyboard over the bottom of the page.
    pub fn draw_keyboard(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = h as f32 - home_zone(ui) - self.keyboard.h();
            self.keyboard.draw(s, ui, top, t);
            let below = top + self.keyboard.h() + self.keyboard.offset(t);
            s.fill_rrect(0.0, below, w as f32, h as f32 - below, 0.0, keyboard::BG);
        }
    }
}

/// One thing Search finds: a pane, or a setting on a pane. `page` is what a tap opens (the pane itself, or the page
/// the setting sits on); `place` is the title of the page the row is listed on.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    pub page: PageId,
    pub place: String,
    /// the row opens a pane of its own
    pub pane: bool,
    /// folded label, place and keywords, space separated
    words: String,
    /// `words` without spaces and punctuation, so "wifi" finds "Wi-Fi"
    squashed: String,
}

impl Entry {
    fn new(label: &str, page: PageId, place: &str, pane: bool) -> Entry {
        let words = fuzzy::fold(&format!("{label} {place} {}", pages::keywords(label)));
        let squashed = squash(&words);
        Entry { label: label.to_string(), page, place: place.to_string(), pane, words, squashed }
    }

    fn matches_word(&self, word: &str) -> bool {
        self.words.contains(word) || self.squashed.contains(&squash(word))
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A pane whose rows open a page with data of its own (a Wi-Fi network, a crash): found by its row, never listed.
fn detail(id: PageId) -> bool {
    matches!(id, PageId::WifiDetail | PageId::BluetoothDetail | PageId::NotificationApp | PageId::UserDetail | PageId::GroupDetail | PageId::GameController(_))
}

/// A pane Search lists but does not walk: its rows are personal (passwords, accounts), read from the disk, change
/// with every device, or the pane is a grid. It is still found by its own row on the page that links to it.
fn walked(id: PageId) -> bool {
    !matches!(
        id,
        PageId::Wallpaper
            | PageId::Passwords
            | PageId::InternetAccounts
            | PageId::Account
            | PageId::Diagnostics
            | PageId::Storage
            | PageId::Passcode
            | PageId::Developer
            | PageId::ScreenTime
            | PageId::StDowntime
            | PageId::StLimits
            | PageId::StAllowed
            | PageId::Users
            | PageId::Groups
            | PageId::GameControllers
            | PageId::FontPick(_)
            | PageId::Graphics
            | PageId::StartupDisk
    )
}

/// A group header as a result's label: "SHOW SCROLL BARS" reads "Show scroll bars" (the macOS sentence case of a
/// section title).
fn header_label(header: &str) -> String {
    let lower = header.trim().to_lowercase();
    let mut chars = lower.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// Every pane reachable from the root page, every group header on them and every setting row, walked from `spec_of` (a
/// page's live spec). A row that opens a pane is listed as that pane; a row without an action is information and is not
/// listed; a group header opens the page it heads, so a section is found by its title ("Highlight colour").
pub fn index(mut spec_of: impl FnMut(PageId) -> Spec) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut seen: HashSet<(String, PageId)> = HashSet::new();
    let mut walk: VecDeque<PageId> = VecDeque::from([PageId::Root]);
    let mut visited: HashSet<PageId> = HashSet::from([PageId::Root]);
    while let Some(id) = walk.pop_front() {
        let page = spec_of(id);
        if id != PageId::Root {
            for header in page.groups.iter().filter_map(|g| g.header.as_deref()) {
                let entry = Entry::new(&header_label(header), id, &page.title, false);
                if !entry.label.is_empty() && seen.insert((entry.label.clone(), entry.page)) {
                    out.push(entry);
                }
            }
        }
        for item in page.groups.iter().flat_map(|g| g.items.iter()) {
            let label = item.label.trim();
            if label.is_empty() {
                continue;
            }
            let entry = match item.action {
                Action::None => continue,
                Action::Push(target) => {
                    if detail(target) {
                        continue;
                    }
                    if walked(target) && visited.insert(target) {
                        walk.push_back(target);
                    }
                    Entry::new(label, target, &page.title, true)
                }
                _ => Entry::new(label, id, &page.title, false),
            };
            if seen.insert((entry.label.clone(), entry.page)) {
                out.push(entry);
            }
        }
    }
    out
}

/// The pages from a first-level pane (a row of the root page) down to `target`, the pane first and `target` last, found
/// by walking the pages from the root as `index` does (the shortest way there). None when no walked page links to it.
pub fn path_to(mut spec_of: impl FnMut(PageId) -> Spec, target: PageId) -> Option<Vec<PageId>> {
    if target == PageId::Root {
        return None;
    }
    let mut parent: HashMap<PageId, PageId> = HashMap::new();
    let mut walk: VecDeque<PageId> = VecDeque::from([PageId::Root]);
    let mut visited: HashSet<PageId> = HashSet::from([PageId::Root]);
    'walk: while let Some(id) = walk.pop_front() {
        let page = spec_of(id);
        for item in page.groups.iter().flat_map(|g| g.items.iter()) {
            let Action::Push(next) = item.action else { continue };
            if !visited.insert(next) {
                continue;
            }
            parent.insert(next, id);
            if next == target {
                break 'walk;
            }
            if walked(next) && !detail(next) {
                walk.push_back(next);
            }
        }
    }
    parent.get(&target)?;
    let mut path = vec![target];
    let mut at = target;
    while let Some(&up) = parent.get(&at) {
        if up == PageId::Root {
            break;
        }
        path.push(up);
        at = up;
    }
    path.reverse();
    Some(path)
}

/// The entries that match `query`: every word of it must match part of an entry. Entries whose label starts with the
/// whole query come first, then those whose label contains it; the rest keep the index order.
pub fn matches<'a>(index: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let q = fuzzy::fold(query.trim());
    let words: Vec<String> = q.split_whitespace().filter(|w| !squash(w).is_empty()).map(str::to_string).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, &Entry)> = index.iter().filter(|e| words.iter().all(|w| e.matches_word(w))).map(|e| (rank(e, &q), e)).collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, e)| e).collect()
}

fn rank(e: &Entry, query: &str) -> u8 {
    let label = fuzzy::fold(&e.label);
    if label.starts_with(query) {
        0
    } else if label.contains(query) {
        1
    } else {
        2
    }
}

/// The page Return opens while Search has a query: the first match's.
pub fn first_hit(index: &[Entry], query: &str) -> Option<PageId> {
    matches(index, query).first().map(|e| e.page)
}

/// The root page while Search has a query: one row per match (its name, and the page it sits on under it), or a
/// single "No Results" row. A tap opens the match's page. A label that more than one page has ("Natural Scrolling" on
/// Mouse and Trackpad) carries its page's name, since the sidebar shows labels alone.
pub fn results(index: &[Entry], query: &str) -> Spec {
    let hits = matches(index, query);
    let twice = |label: &str| hits.iter().filter(|e| e.label == label).count() > 1;
    let mut items: Vec<Item> = hits
        .iter()
        .map(|e| {
            let label = if twice(&e.label) && !e.place.is_empty() { format!("{} ({})", e.label, e.place) } else { e.label.clone() };
            let mut item = Item::new(label).value(e.place.clone());
            item.action = Action::Push(e.page);
            item
        })
        .collect();
    if items.is_empty() {
        items.push(Item::new("No Results"));
    }
    Spec { id: PageId::Root, title: "Settings".into(), large: true, groups: vec![Group::new(items)] }
}

/// Names other apps and menus use for a pane that no row is labelled with.
const ALIASES: &[(&str, PageId)] = &[
    ("Wi-Fi Settings", PageId::Wifi),
    ("Network Settings", PageId::Network),
    ("Bluetooth Settings", PageId::Bluetooth),
    ("Sound", PageId::Sounds),
    ("Displays", PageId::Display),
    ("Desktop & Dock", PageId::HomeScreen),
    ("Privacy & Security", PageId::Privacy),
    ("Users & Groups", PageId::Users),
    ("Print & Scan", PageId::PrintScan),
    ("Keyboard Shortcuts", PageId::Shortcuts),
    ("Lock Screen", PageId::LockScreen),
    ("Screen Time", PageId::ScreenTime),
    ("Trackpad", PageId::Trackpad),
    ("Mouse", PageId::Mouse),
    ("Control Centre", PageId::ControlCentre),
];

/// The pane an alias names ("Wi-Fi Settings"), if it is one.
pub fn alias(name: &str) -> Option<PageId> {
    let n = fuzzy::fold(name.trim());
    ALIASES.iter().find(|(a, _)| fuzzy::fold(a) == n).map(|(_, id)| *id)
}

/// The page a deep link names: an alias, else the pane whose label is the name, else the setting with that label.
pub fn find_page(index: &[Entry], name: &str) -> Option<PageId> {
    if let Some(id) = alias(name) {
        return Some(id);
    }
    let n = fuzzy::fold(name.trim());
    if n.is_empty() {
        return None;
    }
    index
        .iter()
        .find(|e| e.pane && fuzzy::fold(&e.label) == n)
        .or_else(|| index.iter().find(|e| fuzzy::fold(&e.label) == n))
        .map(|e| e.page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::pages::Pick;

    fn page(id: PageId, title: &str, items: Vec<Item>) -> Spec {
        Spec { id, title: title.into(), large: false, groups: vec![Group::new(items)] }
    }

    fn push(label: &str, to: PageId) -> Item {
        let mut item = Item::new(label);
        item.action = Action::Push(to);
        item
    }

    fn setting(label: &str) -> Item {
        let mut item = Item::new(label);
        item.action = Action::Pick(Pick::Nearby(0));
        item
    }

    /// A small settings tree: Root > Network > Wi-Fi (with a detail row and a setting), Root > Battery, Root > Passwords.
    fn tree(id: PageId) -> Spec {
        match id {
            PageId::Root => page(PageId::Root, "Settings", vec![push("Network", PageId::Network), push("Battery", PageId::Battery), push("Passwords", PageId::Passwords)]),
            PageId::Network => page(PageId::Network, "Network", vec![push("Wi-Fi", PageId::Wifi), Item::new("Version")]),
            PageId::Wifi => page(PageId::Wifi, "Wi-Fi", vec![setting("Ask to Join"), push("Cafe", PageId::WifiDetail)]),
            PageId::Battery => page(PageId::Battery, "Battery", vec![setting("Low Power Mode")]),
            other => panic!("walked a page Search must not read: {other:?}"),
        }
    }

    fn found(index: &[Entry]) -> Vec<(String, PageId, bool)> {
        index.iter().map(|e| (e.label.clone(), e.page, e.pane)).collect()
    }

    #[test]
    fn index_lists_panes_and_settings_with_the_page_they_open() {
        let idx = index(tree);
        let f = found(&idx);
        assert!(f.contains(&("Network".into(), PageId::Network, true)));
        assert!(f.contains(&("Wi-Fi".into(), PageId::Wifi, true)));
        assert!(f.contains(&("Ask to Join".into(), PageId::Wifi, false)), "a setting opens the page it is on");
        assert!(f.contains(&("Low Power Mode".into(), PageId::Battery, false)));
        assert!(f.contains(&("Passwords".into(), PageId::Passwords, true)), "a private pane is found by its row");
        assert!(!f.iter().any(|(l, ..)| l == "Cafe"), "a detail row is not a result");
        assert!(!f.iter().any(|(l, ..)| l == "Version"), "a row without an action is not a result");
        assert_eq!(idx.iter().find(|e| e.label == "Wi-Fi").map(|e| e.place.as_str()), Some("Network"));
    }

    #[test]
    fn search_matches_names_keywords_and_squashed_words() {
        let idx = index(tree);
        assert!(matches(&idx, "").is_empty());
        assert_eq!(matches(&idx, "wi-fi")[0].label, "Wi-Fi", "a label prefix ranks first");
        assert!(matches(&idx, "wifi").iter().any(|e| e.label == "Wi-Fi"), "punctuation is ignored");
        assert!(matches(&idx, "ask join").iter().any(|e| e.label == "Ask to Join"), "every word must match");
        assert!(matches(&idx, "wireless").iter().any(|e| e.label == "Wi-Fi"), "a keyword finds the pane");
        assert!(matches(&idx, "zzzz").is_empty());
    }

    #[test]
    fn results_show_matches_or_no_results() {
        let idx = index(tree);
        let spec = results(&idx, "battery");
        let first = &spec.groups[0].items[0];
        assert_eq!((first.label.as_str(), &first.action), ("Battery", &Action::Push(PageId::Battery)));
        let none = results(&idx, "zzzz");
        assert_eq!(none.groups[0].items.len(), 1);
        assert_eq!(none.groups[0].items[0].action, Action::None);
    }

    #[test]
    fn group_headers_are_found_and_open_their_page() {
        let tree_with_header = |id: PageId| {
            let mut s = tree(id);
            if id == PageId::Battery {
                s.groups[0].header = Some("BATTERY HEALTH".into());
            }
            s
        };
        let idx = index(tree_with_header);
        let hit = matches(&idx, "health");
        assert_eq!(hit.first().map(|e| (e.label.as_str(), e.page)), Some(("Battery health", PageId::Battery)), "a section is found by its title");
    }

    #[test]
    fn a_label_on_two_pages_shows_its_page() {
        let two = |id: PageId| match id {
            PageId::Root => page(PageId::Root, "Settings", vec![push("Mouse", PageId::Mouse), push("Trackpad", PageId::Trackpad)]),
            PageId::Mouse => page(PageId::Mouse, "Mouse", vec![setting("Natural Scrolling")]),
            PageId::Trackpad => page(PageId::Trackpad, "Trackpad", vec![setting("Natural Scrolling"), setting("Tap to Click")]),
            other => panic!("unexpected page {other:?}"),
        };
        let idx = index(two);
        let spec = results(&idx, "natural");
        let labels: Vec<&str> = spec.groups[0].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["Natural Scrolling (Mouse)", "Natural Scrolling (Trackpad)"]);
        let single = results(&idx, "tap to");
        assert_eq!(single.groups[0].items[0].label, "Tap to Click", "a label only one page has stays plain");
        assert_eq!(first_hit(&idx, "natural"), Some(PageId::Mouse), "Return opens the first match");
        assert_eq!(first_hit(&idx, "zzzz"), None);
    }

    #[test]
    fn a_path_leads_from_the_pane_to_the_page() {
        assert_eq!(path_to(tree, PageId::Wifi), Some(vec![PageId::Network, PageId::Wifi]));
        assert_eq!(path_to(tree, PageId::Battery), Some(vec![PageId::Battery]));
        assert_eq!(path_to(tree, PageId::WifiDetail), Some(vec![PageId::Network, PageId::Wifi, PageId::WifiDetail]));
        assert_eq!(path_to(tree, PageId::Root), None);
    }

    #[test]
    fn deep_links_resolve_aliases_and_labels() {
        let idx = index(tree);
        assert_eq!(find_page(&idx, "Wi-Fi Settings"), Some(PageId::Wifi));
        assert_eq!(find_page(&idx, "battery"), Some(PageId::Battery));
        assert_eq!(find_page(&idx, "Low Power Mode"), Some(PageId::Battery));
        assert_eq!(find_page(&idx, "Nope"), None);
        assert_eq!(find_page(&idx, "  "), None);
        assert_eq!(alias("Print & Scan"), Some(PageId::PrintScan));
    }
}
