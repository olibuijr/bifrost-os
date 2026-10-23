//! User-selected World Clock cities. Names refer only to the supported `tz` dataset.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_TITLE2_PT, MAC_TITLE3_PT, LINK};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::kit::ui::{MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT};
use super::*;
use springboard::deildu::scroll::Scroller;
use springboard::shell::prefs::Prefs;
use springboard::shell::fuzzy::fold;
use std::path::Path;

const SHEET_W_PT: f32 = 500.0;
/// The city sheet's height is capped at the maximum and never drops below the minimum.
const SHEET_MAX_H_PT: f32 = 620.0;
const SHEET_MIN_H_PT: f32 = 150.0;
const SEARCH_TOP_PT: f32 = 48.0;
const RESULTS_TOP_PT: f32 = 98.0;
const CITY_ROW_PT: f32 = 48.0;

pub(super) struct World {
    pub selected: Vec<usize>,
    pub scroll: Scroller,
    prefs: Prefs,
    sheet: Option<CitySheet>,
    touched: bool,
    delete: Option<usize>,
    /// the card whose move-earlier arrow is pressed
    up: Option<usize>,
    error: Option<String>,
}

impl Default for World {
    fn default() -> Self {
        Self { selected: (0..tz::DEFAULT_CITIES).collect(), scroll: Scroller::new(), prefs: Prefs::memory(), sheet: None, touched: false, delete: None, up: None, error: None }
    }
}

impl World {
    /// Whether the Choose a City sheet is open (it covers the window, so a partial repaint must not run under it).
    pub fn sheet_open(&self) -> bool {
        self.sheet.is_some()
    }

    pub fn load(&mut self, alarm_path: &str) {
        self.prefs = Prefs::load(Path::new(alarm_path).with_file_name("world-cities.ini"));
        self.selected = match self.prefs.get("cities") {
            None => (0..tz::DEFAULT_CITIES).collect(),
            Some(names) => {
                let mut indices = Vec::new();
                for name in names.split('|') {
                    if let Some(index) = CITIES.iter().position(|city| city.name == name.trim()) {
                        if !indices.contains(&index) {
                            indices.push(index);
                        }
                    }
                }
                indices
            }
        };
        self.scroll.jump_top();
        self.sheet = None;
    }

    fn save(&mut self) -> bool {
        let previous = self.prefs.clone();
        let names = self.selected.iter().map(|&i| CITIES[i].name).collect::<Vec<_>>().join("|");
        if let Err(error) = self.prefs.set("cities", &names) {
            eprintln!("springboard: world clock: {error}");
            self.prefs = previous;
            self.error = Some("Could not save World Clocks. Try again.".into());
            return false;
        }
        self.error = None;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Cancel,
    Query,
    City(usize),
    Outside,
}

struct CitySheet {
    query: TextField,
    scroll: Scroller,
    pressed: Option<Hit>,
    drawn_query_wake: Option<f64>,
}

/// Supported cities matching a case-insensitive name search, in dataset order.
pub fn search(query: &str) -> Vec<usize> {
    let query = fold(query.trim());
    let mut found: Vec<usize> = CITIES.iter().enumerate().filter_map(|(i, city)| fold(city.name).contains(&query).then_some(i)).collect();
    // names that start with the query come first; dataset order otherwise
    found.sort_by_key(|&i| !fold(CITIES[i].name).starts_with(&query));
    found
}

/// Moves the entry at `from` to `to` (both clamped into the list).
pub fn move_city(list: &mut Vec<usize>, from: usize, to: usize) {
    if list.is_empty() || from >= list.len() {
        return;
    }
    let city = list.remove(from);
    list.insert(to.min(list.len()), city);
}

impl Clock {
    /// The user's selected cities, in card order.
    pub fn cities(&self) -> Vec<&'static tz::City> {
        self.world.selected.iter().map(|&i| &CITIES[i]).collect()
    }

    /// Adds a supported city if necessary and reveals its card, for Search.
    /// Returns false for an unsupported name or a failed save; failed changes are rolled back.
    pub fn show_city(&mut self, ui: &Ui, name: &str) -> bool {
        let Some(city) = CITIES.iter().position(|c| fold(c.name) == fold(name.trim())) else { return false };
        let position = match self.world.selected.iter().position(|&i| i == city) {
            Some(position) => position,
            None => {
                self.world.selected.push(city);
                if !self.world.save() { self.world.selected.pop(); return false }
                self.world.selected.len() - 1
            }
        };
        self.tab = Tab::World;
        self.editing = false;
        self.add = None;
        self.world.sheet = None;
        self.fit_world(ui);
        let top = ui.pt(SPACE_M) + (position / 3) as f32 * ui.pt(WORLD_CARD_H_PT + CARD_MARGIN_PT);
        self.world.scroll.jump_to(top);
        true
    }

    pub(super) fn fit_world(&mut self, ui: &Ui) {
        let rows = self.world.selected.len().div_ceil(3);
        let height = ui.pt(SPACE_M) + rows as f32 * ui.pt(WORLD_CARD_H_PT + CARD_MARGIN_PT);
        self.world.scroll.set_max(height - (self.tab_top(ui) - Self::content_top(ui)));
    }

    pub(super) fn add_city_sheet(&mut self, ui: &Ui) {
        // the search field has focus as the sheet opens: typing filters the cities straight away
        let mut query = TextField::new(ui, "Search Cities", MAC_TITLE2_PT, false, 80);
        query.focused = true;
        self.world.sheet = Some(CitySheet { query, scroll: Scroller::new(), pressed: None, drawn_query_wake: None });
        self.editing = false;
        self.fit_city_sheet(ui);
    }

    fn city_sheet_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let width = ui.pt(SHEET_W_PT).min(self.w as f32 - ui.pt(SPACE_XXXL));
        let top = ui.pt(POP_Y_PT);
        ((self.w as f32 - width) / 2.0, top, width, (self.h as f32 - home_zone(ui) - top - ui.pt(SPACE_L)).min(ui.pt(SHEET_MAX_H_PT)).max(ui.pt(SHEET_MIN_H_PT)))
    }

    fn city_query_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.city_sheet_rect(ui);
        (x + ui.pt(SPACE_L), y + ui.pt(SEARCH_TOP_PT), w - ui.pt(SPACE_XXXL), ui.pt(36.0))
    }

    fn fit_city_sheet(&mut self, ui: &Ui) {
        let (_, _, _, height) = self.city_sheet_rect(ui);
        if let Some(sheet) = &mut self.world.sheet {
            sheet.scroll.set_max(search(sheet.query.text()).len() as f32 * ui.pt(CITY_ROW_PT) - (height - ui.pt(RESULTS_TOP_PT + 8.0)));
        }
    }

    fn city_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        let sheet = self.world.sheet.as_ref()?;
        let (sx, sy, sw, sh) = self.city_sheet_rect(ui);
        if !Self::within(x, y, (sx, sy, sw, sh)) {
            return Some(Hit::Outside);
        }
        if y < sy + ui.pt(ROW_PT) && x > sx + sw - ui.pt(90.0) {
            return Some(Hit::Cancel);
        }
        if Self::within(x, y, self.city_query_rect(ui)) {
            return Some(Hit::Query);
        }
        if y >= sy + ui.pt(RESULTS_TOP_PT) {
            let row = ((y - sy - ui.pt(RESULTS_TOP_PT) + sheet.scroll.offset()) / ui.pt(CITY_ROW_PT)).floor() as usize;
            return search(sheet.query.text()).get(row).copied().map(Hit::City);
        }
        None
    }

    fn city_key(&mut self, ui: &Ui, out: Out, t: f64) {
        let Some(sheet) = &mut self.world.sheet else { return };
        match out {
            Out::Hide | Out::Enter => {
                sheet.query.focused = false;
            }
            out if sheet.query.focused => { sheet.query.edit(ui, &out, t); sheet.scroll.jump_top(); }
            _ => {}
        }
        self.fit_city_sheet(ui);
    }

    /// Modal city-picker input, or scrolling/removal in World Clock content.
    pub(super) fn world_event(&mut self, ui: &Ui, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let (x, y) = match ev { Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y), _ => return None };
        if self.world.sheet.is_some() {
            let (qx, qy, qw, qh) = self.city_query_rect(ui);
            let field = (qx + ui.pt(SPACE_M), qy, qw - ui.pt(SPACE_XXL), qh);
            if self.world.sheet.as_mut().is_some_and(|s| s.query.touch(ui, ev, field, self.w as f32, t)) {
                self.fit_city_sheet(ui);
                return Some(vec![]);
            }
            let hit = self.city_hit(ui, x, y);
            match ev {
                Event::Down(..) => {
                    let sheet = self.world.sheet.as_mut().unwrap();
                    sheet.pressed = hit;
                    if matches!(hit, Some(Hit::City(_))) { sheet.scroll.down(y, t); }
                }
                Event::Move(..) => {
                    let sheet = self.world.sheet.as_mut().unwrap();
                    if matches!(sheet.pressed, Some(Hit::City(_))) { sheet.scroll.moved(y, t); }
                }
                Event::Up(..) => {
                    let pressed = self.world.sheet.as_mut().unwrap().pressed.take();
                    match pressed {
                        Some(Hit::City(city)) => {
                            let tap = self.world.sheet.as_mut().unwrap().scroll.up(y, t);
                            if tap && hit == pressed { self.show_city(ui, CITIES[city].name); }
                        }
                        Some(Hit::Cancel | Hit::Outside) if hit == pressed => self.world.sheet = None,
                        Some(Hit::Query) => {
                            let sheet = self.world.sheet.as_mut().unwrap();
                            sheet.query.focused = true;
                            self.fit_city_sheet(ui);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            return Some(vec![]);
        }
        if self.tab != Tab::World { return None }
        self.fit_world(ui);
        match ev {
            Event::Down(..) if y >= Self::content_top(ui) && y < self.tab_top(ui) => {
                self.world.touched = true;
                self.world.delete = (0..self.world.selected.len()).find(|&i| {
                    let (cx, cy, _, _) = self.world_card(ui, i);
                    self.editing && Self::near((cx + ui.pt(SPACE_XL), cy + ui.pt(SPACE_XL)), ui.pt(SPACE_XXL), x, y)
                });
                self.world.up = (0..self.world.selected.len()).find(|&i| {
                    let (cx, cy, cw, _) = self.world_card(ui, i);
                    self.editing && i > 0 && Self::near((cx + cw - ui.pt(SPACE_XXL), cy + ui.pt(SPACE_XXL)), ui.pt(SPACE_XXL), x, y)
                });
                self.world.scroll.down(y, t);
                Some(vec![])
            }
            Event::Move(..) if self.world.touched => { self.world.scroll.moved(y, t); Some(vec![]) }
            Event::Up(..) if self.world.touched => {
                self.world.touched = false;
                let tap = self.world.scroll.up(y, t);
                if let Some(i) = self.world.up.take().filter(|_| tap) {
                    let (cx, cy, cw, _) = self.world_card(ui, i);
                    if Self::near((cx + cw - ui.pt(SPACE_XXL), cy + ui.pt(SPACE_XXL)), ui.pt(SPACE_XXL), x, y) {
                        let before = self.world.selected.clone();
                        move_city(&mut self.world.selected, i, i - 1);
                        if !self.world.save() { self.world.selected = before; }
                    }
                }
                if let Some(i) = self.world.delete.take().filter(|_| tap) {
                    let (cx, cy, _, _) = self.world_card(ui, i);
                    if Self::near((cx + ui.pt(SPACE_XL), cy + ui.pt(SPACE_XL)), ui.pt(SPACE_XXL), x, y) {
                        let city = self.world.selected.remove(i);
                        if !self.world.save() { self.world.selected.insert(i, city); }
                        self.editing &= !self.world.selected.is_empty();
                        self.fit_world(ui);
                    }
                }
                Some(vec![])
            }
            _ => None,
        }
    }

    pub(super) fn world_tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut dirty = self.world.scroll.tick(t);
        if let Some(sheet) = &mut self.world.sheet {
            dirty |= sheet.scroll.tick(t) | sheet.query.tick(ui, t);
            dirty |= sheet.drawn_query_wake.is_some_and(|at| at <= t);
        }
        dirty
    }

    pub(super) fn world_next_wake(&self, t: f64) -> Option<f64> {
        let mut due = self.world.scroll.animating().then_some(t);
        if let Some(sheet) = &self.world.sheet {
            if sheet.scroll.animating() { return Some(t) }
            if let Some(at) = sheet.query.next_wake(t) { due = Some(due.map_or(at, |d| d.min(at))); }
        }
        due
    }

    pub(super) fn draw_world_notice(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(error) = &self.world.error else { return };
        let text = ui.text(error, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now());
        let width = text.w as f32 + ui.pt(SPACE_XXXL);
        let (x, y) = ((self.w as f32 - width) / 2.0, self.tab_top(ui) - ui.pt(54.0));
        s.fill_rrect(x, y, width, ui.pt(SPACE_HUGE), ui.pt(m::CARD_RADIUS), FIELD_BG.now());
        s.blit(&text, (x + ui.pt(SPACE_L)) as i32, (y + (ui.pt(SPACE_HUGE) - text.h as f32) / 2.0) as i32, 255, 255);
    }

    pub(super) fn draw_world_sheet(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.world.sheet.is_none() { return }
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, fade(SCRIM, 130));
        let (x, y, w, h) = self.city_sheet_rect(ui);
        s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        self.label(s, ui, "Choose a City", MAC_BODY_PT, Weight::Semibold, LABEL.now(), x + w / 2.0, y + ui.pt(SPACE_XXL), 0.5);
        self.label(s, ui, "Cancel", MAC_BODY_PT, Weight::Regular, LINK, x + w - ui.pt(SPACE_L), y + ui.pt(SPACE_XXL), 1.0);
        let query = self.city_query_rect(ui);
        s.fill_rrect(query.0, query.1, query.2, query.3, 1.0e4, FIELD_BG.now());
        let matches = search(self.world.sheet.as_ref().unwrap().query.text());
        let offset = self.world.sheet.as_ref().unwrap().scroll.offset();
        let list_top = y + ui.pt(RESULTS_TOP_PT);
        let mut content = s.rows(list_top as usize, (y + h - ui.pt(SPACE_S)) as usize);
        if matches.is_empty() {
            self.label(&mut content, ui, "No Cities Found", MAC_TITLE3_PT, Weight::Regular, LABEL_2.now(), x + w / 2.0, list_top + ui.pt(44.0), 0.5);
        }
        for (row, city) in matches.into_iter().enumerate() {
            let ry = list_top + row as f32 * ui.pt(CITY_ROW_PT) - offset;
            if ry + ui.pt(CITY_ROW_PT) < list_top || ry >= y + h { continue }
            self.label(&mut content, ui, CITIES[city].name, MAC_TITLE3_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_XL), ry + ui.pt(CITY_ROW_PT / 2.0), 0.0);
            if self.world.selected.contains(&city) {
                self.label(&mut content, ui, "Added", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + w - ui.pt(SPACE_XL), ry + ui.pt(CITY_ROW_PT / 2.0), 1.0);
            }
            content.fill_rrect(x + ui.pt(SPACE_XL), ry + ui.pt(CITY_ROW_PT) - 1.0, w - ui.pt(SPACE_HUGE), 1.0, 0.0, SEPARATOR_C.now());
        }
        let field = (query.0 + ui.pt(SPACE_M), query.1, query.2 - ui.pt(SPACE_XXL), query.3);
        let sheet = self.world.sheet.as_mut().unwrap();
        sheet.query.draw(s, ui, field, t);
        sheet.drawn_query_wake = sheet.query.next_wake(t);
        if let Some(error) = &self.world.error {
            let image = ui.text(error, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(&image, (x + (w - image.w as f32) / 2.0) as i32, (y + h - ui.pt(SPACE_XXL)) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::tests::{clock, tap, ui};
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-world").join(std::process::id().to_string()).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn remove(c: &mut Clock, ui: &Ui, i: usize, t: f64) {
        c.editing = true;
        let (x, y, _, _) = c.world_card(ui, i);
        tap(c, ui, x + ui.pt(20.0), y + ui.pt(20.0), t);
    }

    #[test]
    fn move_city_reorders_and_clamps() {
        let mut l = vec![1, 2, 3, 4];
        move_city(&mut l, 2, 0);
        assert_eq!(l, [3, 1, 2, 4]);
        move_city(&mut l, 0, 99);
        assert_eq!(l, [1, 2, 4, 3]);
        move_city(&mut l, 9, 0);
        assert_eq!(l, [1, 2, 4, 3], "out of range is ignored");
        move_city(&mut Vec::new(), 0, 0);
    }

    #[test]
    fn search_ranks_prefix_matches_first() {
        let names: Vec<&str> = search("ber").into_iter().map(|i| CITIES[i].name).collect();
        assert_eq!(names.first(), Some(&"Berlin"));
        let s = search("o");
        let first_non_prefix = s.iter().position(|&i| !CITIES[i].name.to_lowercase().starts_with('o')).unwrap();
        assert!(s[..first_non_prefix].iter().all(|&i| CITIES[i].name.to_lowercase().starts_with('o')));
        assert!(s[first_non_prefix..].iter().all(|&i| !CITIES[i].name.to_lowercase().starts_with('o')));
    }

    #[test]
    fn the_arrow_moves_a_card_earlier_in_edit_mode() {
        let ui = ui();
        let mut c = clock(&ui);
        c.editing = true;
        let (x, y, w, _) = c.world_card(&ui, 2);
        tap(&mut c, &ui, x + w - ui.pt(24.0), y + ui.pt(24.0), 1000.0);
        assert_eq!(&c.world.selected[..3], &[0, 2, 1]);
        let (x, y, w, _) = c.world_card(&ui, 0);
        tap(&mut c, &ui, x + w - ui.pt(24.0), y + ui.pt(24.0), 2000.0);
        assert_eq!(&c.world.selected[..3], &[0, 2, 1], "the first card has no arrow");
    }

    #[test]
    fn selection_removal_restart_and_alarm_data_are_preserved() {
        let ui = ui();
        let dir = dir("persist");
        let path = dir.join("clock.ini");
        let alarms = "alarm.0.time=07:30\nalarm.0.on=1\nalarm.0.label=Breakfast\n";
        std::fs::write(&path, alarms).unwrap();
        let mut c = clock(&ui);
        c.load(path.to_str().unwrap(), 1000.0);
        assert_eq!(c.cities().len(), tz::DEFAULT_CITIES, "legacy installs keep the existing cities");
        remove(&mut c, &ui, 1, 1100.0);
        assert!(!c.cities().iter().any(|city| city.name == "London"));
        assert!(c.show_city(&ui, "london"));
        assert_eq!(c.cities().last().unwrap().name, "London");
        assert!(c.show_city(&ui, "London"));
        assert_eq!(c.cities().len(), tz::DEFAULT_CITIES, "adding an existing city never duplicates it");
        assert!(!c.show_city(&ui, "Atlantis"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), alarms);
        let mut again = clock(&ui);
        again.load(path.to_str().unwrap(), 2000.0);
        assert_eq!(again.cities(), c.cities());
        assert_eq!(again.alarms().list[0].label, "Breakfast");
    }

    #[test]
    fn an_empty_selection_survives_restart_and_can_be_added_to() {
        let ui = ui();
        let dir = dir("empty");
        std::fs::write(dir.join("world-cities.ini"), "cities=Tokyo\n").unwrap();
        let path = dir.join("clock.ini");
        let mut c = clock(&ui);
        c.load(path.to_str().unwrap(), 0.0);
        remove(&mut c, &ui, 0, 1000.0);
        assert!(c.cities().is_empty());
        assert!(!c.editing);
        let mut again = clock(&ui);
        again.load(path.to_str().unwrap(), 2000.0);
        assert!(again.cities().is_empty());
        assert!(again.show_city(&ui, "Sydney"));
        assert_eq!(again.cities()[0].name, "Sydney");
    }

    #[test]
    fn city_search_picker_selects_and_blocks_cards_and_navigation_behind_it() {
        let ui = ui();
        let mut c = clock(&ui);
        c.world.selected = vec![0];
        let nav = ui.pt(UNIFIED_BAR_PT / 2.0);
        tap(&mut c, &ui, 1200.0 - ui.pt(20.0), nav, 1000.0);
        assert!(c.world.sheet.is_some());
        c.city_key(&ui, Out::Text("  yOrK  ".into()), 1100.0);
        assert_eq!(search(c.world.sheet.as_ref().unwrap().query.text()), vec![5]);
        assert!(search("not a city").is_empty());
        assert_eq!(search("Reykjavík"), vec![0]);
        let (x, y, _, _) = c.city_sheet_rect(&ui);
        tap(&mut c, &ui, x + ui.pt(40.0), y + ui.pt(RESULTS_TOP_PT + CITY_ROW_PT / 2.0), 1200.0);
        assert_eq!(c.cities().iter().map(|city| city.name).collect::<Vec<_>>(), ["Reykjavik", "New York"]);
        assert!(c.world.sheet.is_none());
        c.add_city_sheet(&ui);
        let (tab_x, tab_y) = crate::clock::tests::bar_point(&c, &ui, BAR_TAB + 1);
        tap(&mut c, &ui, tab_x, tab_y, 2100.0);
        assert_eq!(c.tab, Tab::World, "modal input never switches the underlying tab");
        assert_eq!(c.cities().len(), 2);
    }

    #[test]
    fn scroll_gestures_reveal_last_cards_and_do_not_delete_them() {
        let ui = ui();
        let mut c = clock(&ui);
        c.h = 1300;
        c.fit_world(&ui);
        c.editing = true;
        let (x, y, _, _) = c.world_card(&ui, 0);
        let (x, y) = (x + ui.pt(20.0), y + ui.pt(20.0));
        c.touch(&ui, Event::Down(x, y), 1000.0);
        c.touch(&ui, Event::Move(x, y - ui.pt(180.0)), 1200.0);
        c.touch(&ui, Event::Up(x, y - ui.pt(180.0)), 1400.0);
        c.tick(&ui, 5000.0);
        assert_eq!(c.cities().len(), tz::DEFAULT_CITIES);
        assert!(c.world.scroll.offset() > 0.0);
        assert!(c.show_city(&ui, "Sydney"));
        let (_, y, _, h) = c.world_card(&ui, 8);
        assert!(y >= Clock::content_top(&ui) && y + h <= c.tab_top(&ui));
    }

    #[test]
    fn persistence_failure_rolls_back_changes_and_reports_an_error() {
        let ui = ui();
        let dir = dir("failure");
        let city_path = dir.join("world-cities.ini");
        std::fs::write(&city_path, "cities=Reykjavik|London\n").unwrap();
        let path = dir.join("clock.ini");
        let mut c = clock(&ui);
        c.load(path.to_str().unwrap(), 0.0);
        std::fs::remove_file(&city_path).unwrap();
        std::fs::create_dir(&city_path).unwrap();
        assert!(!c.show_city(&ui, "Tokyo"));
        assert_eq!(c.cities().len(), 2);
        assert!(c.world.error.is_some());
        remove(&mut c, &ui, 1, 1000.0);
        assert_eq!(c.cities().len(), 2);
        std::fs::remove_dir(&city_path).unwrap();
        assert!(c.show_city(&ui, "Tokyo"));
        assert!(c.world.error.is_none());
        let mut again = clock(&ui);
        again.load(path.to_str().unwrap(), 0.0);
        assert_eq!(again.cities(), c.cities());
    }

    #[test]
    fn selected_cities_use_supported_dst_and_cross_midnight_correctly() {
        let ui = ui();
        let mut c = clock(&ui);
        c.world.selected.clear();
        assert!(c.show_city(&ui, "London"));
        assert!(c.show_city(&ui, "Sydney"));
        let unix = tz::days_from_civil(2026, 10, 3) * 86_400 + 23 * 3600 + 32 * 60;
        let cities = c.cities();
        assert_eq!(cities[0].local(unix).hour, 0);
        assert_eq!(cities[1].local(unix).hour, 10);
        assert_eq!(tz::relative(cities[0], unix, 0), ("Tomorrow", 1));
        assert_eq!(tz::relative(cities[1], unix, 0), ("Tomorrow", 11));
        let winter = tz::days_from_civil(2026, 1, 15) * 86_400;
        assert_eq!(cities[0].offset_at(winter), 0);
        assert_eq!(cities[1].offset_at(winter), 11 * 3600);
    }
}
