//! Settings › General › Date & Time: the Time Zone picker. A card with a
//! search field over a scrolling list of every zone in the tz database
//! (the same pattern as the World Clock city picker). The page's own rows
//! are built by `pages::spec`; Settings opens this card from the Time Zone
//! row and applies the zone it returns.

use crate::layout::in_rect as within;
use super::fuzzy::fold;
use super::keyboard::{self, Keyboard, Out};
use super::textfield::TextField;
use super::ui::{home_zone, Ui, Weight, CARD_BG, LABEL, SECONDARY, SEPARATOR_C};
use super::wifi_page::BLUE;
use super::Event;
use crate::bare::timezone;
use crate::deildu::scroll::Scroller;
use crate::gfx::{pack, rgba, Image, Surface};
use std::collections::HashMap;

const CARD_W_PT: f32 = 500.0;
const CARD_TOP_PT: f32 = 69.0;
const BAR_PT: f32 = 44.0;
const SEARCH_TOP_PT: f32 = 48.0;
const SEARCH_H_PT: f32 = 36.0;
const RESULTS_TOP_PT: f32 = 98.0;
const ROW_PT: f32 = 48.0;
const FIELD: u32 = pack(44, 44, 46, 255);

/// Zones matching a search: every word of the query must appear in the
/// zone's name, ignoring case, accents and the underscores in "New_York".
pub fn filter(zones: &[String], query: &str) -> Vec<usize> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    zones
        .iter()
        .enumerate()
        .filter(|(_, z)| {
            let name = fold(z).replace('_', " ");
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// "America/New_York" as shown in the list.
pub fn display(zone: &str) -> String {
    zone.replace('_', " ")
}

/// What a touch did to the picker.
#[derive(Clone, Debug, PartialEq)]
pub enum PickerEvent {
    None,
    Cancel,
    Pick(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Keyboard,
    Row(usize),
    Cancel,
    Outside,
    Query,
}

pub struct ZonePicker {
    w: i32,
    h: i32,
    zones: Vec<String>,
    current: String,
    query: TextField,
    keyboard: Keyboard,
    scroll: Scroller,
    pressed: Option<Press>,
    /// rendered row labels, by zone name
    labels: HashMap<String, Image>,
    title: Image,
    cancel: Image,
    empty: Image,
    check: Image,
}

impl ZonePicker {
    /// Opens at `t` with the keyboard sliding in; `current` is marked.
    pub fn new(ui: &Ui, w: i32, h: i32, current: &str, t: f64) -> Self {
        let mut keyboard = Keyboard::new_hidden(w as f32, ui.s);
        keyboard.set_shown(true, t);
        let mut picker = ZonePicker {
            w,
            h,
            zones: timezone::list_zones(),
            current: current.to_string(),
            query: TextField::new(ui, "Search Time Zones", 17.0, false, 80),
            keyboard,
            scroll: Scroller::new(),
            pressed: None,
            labels: HashMap::new(),
            title: ui.text("Time Zone", 17.0, Weight::Semibold, LABEL.now()),
            cancel: ui.text("Cancel", 17.0, Weight::Regular, BLUE),
            empty: ui.text("No Time Zones Found", 20.0, Weight::Regular, SECONDARY),
            check: ui.text("Current", 14.0, Weight::Regular, SECONDARY),
        };
        picker.fit(ui);
        picker
    }

    fn matches(&self) -> Vec<usize> {
        filter(&self.zones, self.query.text())
    }

    fn keyboard_top(&self, ui: &Ui) -> f32 {
        let docked = self.keyboard.shown().then_some(self.keyboard.h()).unwrap_or(0.0);
        self.h as f32 - home_zone(ui) - docked
    }

    /// The card (x, y, w, h), above the keyboard.
    fn card(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let width = ui.pt(CARD_W_PT).min(self.w as f32 - ui.pt(32.0));
        let top = ui.pt(CARD_TOP_PT);
        ((self.w as f32 - width) / 2.0, top, width, (self.keyboard_top(ui) - top - ui.pt(16.0)).min(ui.pt(620.0)).max(ui.pt(150.0)))
    }

    fn query_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.card(ui);
        (x + ui.pt(16.0), y + ui.pt(SEARCH_TOP_PT), w - ui.pt(32.0), ui.pt(SEARCH_H_PT))
    }

    /// The box the query text is drawn in.
    fn field_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.query_rect(ui);
        (x + ui.pt(12.0), y, w - ui.pt(24.0), h)
    }

    fn cancel_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.card(ui);
        (x + w - ui.pt(90.0), y, ui.pt(90.0), ui.pt(BAR_PT))
    }

    /// Resizes the scroll range to the matches in view.
    fn fit(&mut self, ui: &Ui) {
        let (_, _, _, ch) = self.card(ui);
        let rows = self.matches().len() as f32;
        self.scroll.set_max(rows * ui.pt(ROW_PT) - (ch - ui.pt(RESULTS_TOP_PT + 8.0)));
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Press> {
        if self.keyboard.shown() && y >= self.keyboard_top(ui) {
            return Some(Press::Keyboard);
        }
        let card = self.card(ui);
        if !within(x, y, card) {
            return Some(Press::Outside);
        }
        if within(x, y, self.cancel_rect(ui)) {
            return Some(Press::Cancel);
        }
        if within(x, y, self.query_rect(ui)) {
            return Some(Press::Query);
        }
        if y >= card.1 + ui.pt(RESULTS_TOP_PT) {
            let row = ((y - card.1 - ui.pt(RESULTS_TOP_PT) + self.scroll.offset()) / ui.pt(ROW_PT)).floor() as usize;
            return (row < self.matches().len()).then_some(Press::Row(row));
        }
        None
    }

    /// Typed input for the search field (the on-screen or a hardware keyboard).
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        match out {
            Out::Hide | Out::Enter => {
                self.keyboard.set_shown(false, t);
                self.query.focused = false;
            }
            out if self.query.focused => {
                self.query.edit(ui, &out, t);
                self.scroll.jump_top();
            }
            _ => {}
        }
        self.fit(ui);
    }

    /// Handles a touch; the picker is modal, so every touch is its own.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> PickerEvent {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return PickerEvent::None,
        };
        let frame = self.field_frame(ui);
        if self.query.touch(ui, ev, frame, self.w as f32, t) {
            if self.query.focused {
                self.keyboard.set_shown(true, t);
            }
            self.fit(ui);
            return PickerEvent::None;
        }
        let top = self.keyboard_top(ui);
        let hit = self.hit(ui, x, y);
        match ev {
            Event::Down(..) => {
                self.pressed = hit;
                match hit {
                    Some(Press::Keyboard) => self.keyboard.down(x, y - top, t),
                    Some(Press::Row(_)) => self.scroll.down(y, t),
                    _ => {}
                }
            }
            Event::Move(..) => match self.pressed {
                Some(Press::Keyboard) => self.keyboard.moved(x, y - top),
                Some(Press::Row(_)) => self.scroll.moved(y, t),
                _ => {}
            },
            _ => match self.pressed.take() {
                Some(Press::Keyboard) => {
                    if let Some(out) = self.keyboard.up(x, y - top, t) {
                        self.key(ui, out, t);
                    }
                }
                Some(Press::Row(row)) => {
                    let tap = self.scroll.up(y, t);
                    if tap && hit == Some(Press::Row(row)) {
                        let picked = self.matches().get(row).map(|&i| self.zones[i].clone());
                        return picked.map_or(PickerEvent::None, PickerEvent::Pick);
                    }
                }
                Some(p @ (Press::Cancel | Press::Outside)) if hit == Some(p) => return PickerEvent::Cancel,
                Some(Press::Query) => {
                    self.query.focused = true;
                    self.keyboard.set_shown(true, t);
                    self.fit(ui);
                }
                _ => {}
            },
        }
        PickerEvent::None
    }

    /// Repeats held keys, scrolls and blinks the caret; true when a frame
    /// is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut dirty = self.scroll.tick(t) | self.keyboard.animating() | self.query.tick(ui, t);
        for out in self.keyboard.tick(t) {
            self.key(ui, out, t);
            dirty = true;
        }
        dirty | self.query.next_wake(t).is_some_and(|due| due <= t + 1.0)
    }

    /// When the next frame is due on its own.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.scroll.animating() || self.keyboard.animating() {
            return Some(t);
        }
        self.query.next_wake(t)
    }

    /// Renders a row's label once.
    fn label(&mut self, ui: &Ui, zone: &str) {
        self.labels.entry(zone.to_string()).or_insert_with(|| ui.text(&display(zone), 20.0, Weight::Regular, LABEL.now()));
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 130));
        let (x, y, w, h) = self.card(ui);
        s.fill_rrect(x, y, w, h, ui.pt(6.0), CARD_BG.now());
        let mid = y + ui.pt(BAR_PT) / 2.0;
        s.blit(&self.title, (x + (w - self.title.w as f32) / 2.0) as i32, (mid - self.title.h as f32 / 2.0) as i32, 255, 255);
        s.blit(&self.cancel, (x + w - ui.pt(16.0) - self.cancel.w as f32) as i32, (mid - self.cancel.h as f32 / 2.0) as i32, 255, 255);
        let q = self.query_rect(ui);
        s.fill_rrect(q.0, q.1, q.2, q.3, ui.pt(6.0), FIELD);

        let matches = self.matches();
        let offset = self.scroll.offset();
        let list_top = y + ui.pt(RESULTS_TOP_PT);
        let row_h = ui.pt(ROW_PT);
        let mut content = s.rows(list_top as usize, (y + h - ui.pt(8.0)) as usize);
        if matches.is_empty() {
            content.blit(&self.empty, (x + (w - self.empty.w as f32) / 2.0) as i32, (list_top + ui.pt(44.0) - self.empty.h as f32 / 2.0) as i32, 255, 255);
        }
        for (row, &i) in matches.iter().enumerate() {
            let ry = list_top + row as f32 * row_h - offset;
            if ry + row_h < list_top || ry >= y + h {
                continue;
            }
            let zone = self.zones[i].clone();
            let rmid = ry + row_h / 2.0;
            self.label(ui, &zone);
            let label = &self.labels[&zone];
            content.blit(&label, (x + ui.pt(20.0)) as i32, (rmid - label.h as f32 / 2.0) as i32, 255, 255);
            if zone == self.current {
                content.blit(&self.check, (x + w - ui.pt(20.0) - self.check.w as f32) as i32, (rmid - self.check.h as f32 / 2.0) as i32, 255, 255);
            }
            content.fill_rrect(x + ui.pt(20.0), ry + row_h - 1.0, w - ui.pt(40.0), 1.0, 0.0, SEPARATOR_C.now());
        }
        self.query.draw(s, ui, self.field_frame(ui), t);
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = self.h as f32 - home_zone(ui) - self.keyboard.h();
            self.keyboard.draw(s, ui, top, t);
            let bottom = top + self.keyboard.h() + self.keyboard.offset(t);
            s.fill_rrect(0.0, bottom, self.w as f32, self.h as f32 - bottom, 0.0, keyboard::BG);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zones() -> Vec<String> {
        ["Africa/Cairo", "America/New_York", "America/Los_Angeles", "Atlantic/Reykjavik", "Europe/Paris"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn search_matches_every_word_ignoring_case_and_underscores() {
        let z = zones();
        assert_eq!(filter(&z, ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(filter(&z, "new york"), vec![1]);
        assert_eq!(filter(&z, "AMERICA"), vec![1, 2]);
        assert_eq!(filter(&z, "reykjavík"), vec![3]);
        assert_eq!(filter(&z, "los america"), vec![2]);
        assert!(filter(&z, "atlantis").is_empty());
    }

    #[test]
    fn zone_names_are_shown_without_underscores() {
        assert_eq!(display("America/New_York"), "America/New York");
    }
}
