//! Notes > Settings: a pane over the window with the Notes settings (text size, what a new note starts with, the dark
//! note background) and the two list options that live in the preferences (sort order, oldest first, group by date).
//!
//! A tap on a row changes its value at once and saves it; Done, or a tap outside the pane, closes it. The pane takes
//! the taps while it is open, as the replace card does.

use super::*;

/// The pane's width at most, and the heights of its header and rows, in points.
const PANE_W_PT: f32 = 420.0;
const PANE_HEAD_PT: f32 = 48.0;
const PANE_ROW_PT: f32 = 44.0;
/// The width of the Done button, in points.
const PANE_DONE_PT: f32 = 64.0;

/// A row of the pane, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    TextSize,
    NewNoteStart,
    Sort,
    OldestFirst,
    GroupByDate,
    DarkBackground,
}

pub const ITEMS: [Item; 6] = [Item::TextSize, Item::NewNoteStart, Item::Sort, Item::OldestFirst, Item::GroupByDate, Item::DarkBackground];

impl Item {
    /// The row's name, on the left of the row.
    pub fn title(self) -> &'static str {
        match self {
            Item::TextSize => "Text Size",
            Item::NewNoteStart => "New Notes Start With",
            Item::Sort => "Sort Notes By",
            Item::OldestFirst => "Oldest First",
            Item::GroupByDate => "Group by Date",
            Item::DarkBackground => "Dark Note Background",
        }
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

/// The index of the row a point is on, if any.
pub fn row_index(rows: usize, row_at: impl Fn(usize) -> bool) -> Option<usize> {
    (0..rows).find(|&i| row_at(i))
}

/// The next sort key in the pane's cycle (Date Edited, Date Created, Title, then Date Edited again).
pub fn next_sort(current: store::SortKey) -> store::SortKey {
    let all = store::SortKey::ALL;
    let i = all.iter().position(|k| *k == current).unwrap_or(0);
    all[(i + 1) % all.len()]
}

impl Notes {
    /// Notes > Settings: opens the pane over the window.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.dirty = true;
    }

    /// The pane: centred under the toolbar, at most `PANE_W_PT` wide.
    fn pane_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let pad = ui.pt(SPACE_S);
        let w = (self.w as f32 - 2.0 * pad).min(ui.pt(PANE_W_PT)).max(1.0);
        let x = (self.w as f32 - w) / 2.0;
        let h = ui.pt(PANE_HEAD_PT) + ui.pt(PANE_ROW_PT) * ITEMS.len() as f32 + pad;
        (x, ui.pt(nav_bottom_pt()) + pad, w, h)
    }

    fn pane_done(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.pane_rect(ui);
        (x + w - ui.pt(PANE_DONE_PT), y, ui.pt(PANE_DONE_PT), ui.pt(PANE_HEAD_PT))
    }

    fn pane_row(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.pane_rect(ui);
        (x, y + ui.pt(PANE_HEAD_PT) + ui.pt(PANE_ROW_PT) * i as f32, w, ui.pt(PANE_ROW_PT))
    }

    /// The value a row shows.
    fn setting_value(&self, item: Item) -> &'static str {
        match item {
            Item::TextSize => self.note_settings.text_size.label(),
            Item::NewNoteStart => self.note_settings.new_note_start.label(),
            Item::Sort => self.prefs.sort.label(),
            Item::OldestFirst => on_off(self.prefs.oldest_first),
            Item::GroupByDate => on_off(self.prefs.group_by_date),
            Item::DarkBackground => on_off(self.note_settings.dark_background),
        }
    }

    /// A tap while the pane is open: Done closes it, a row changes its value, and a tap outside the pane closes it.
    pub(super) fn settings_tap(&mut self, ui: &Ui, x: f32, y: f32, t: f64) {
        self.dirty = true;
        if within(x, y, self.pane_done(ui)) || !within(x, y, self.pane_rect(ui)) {
            self.settings_open = false;
            return;
        }
        if let Some(i) = row_index(ITEMS.len(), |i| within(x, y, self.pane_row(ui, i))) {
            self.change_setting(ui, ITEMS[i], t);
        }
    }

    fn change_setting(&mut self, ui: &Ui, item: Item, t: f64) {
        match item {
            Item::TextSize => {
                self.note_settings.text_size = self.note_settings.text_size.next();
                self.save_note_settings();
                self.remake_editor(ui, t);
            }
            Item::NewNoteStart => {
                self.note_settings.new_note_start = self.note_settings.new_note_start.next();
                self.save_note_settings();
            }
            Item::DarkBackground => {
                self.note_settings.dark_background = !self.note_settings.dark_background;
                self.save_note_settings();
                self.remake_editor(ui, t);
            }
            Item::Sort => {
                self.prefs.sort = next_sort(self.prefs.sort);
                self.save_prefs();
                self.rebuild(ui, t);
            }
            Item::OldestFirst => {
                self.prefs.oldest_first = !self.prefs.oldest_first;
                self.save_prefs();
                self.rebuild(ui, t);
            }
            Item::GroupByDate => {
                self.prefs.group_by_date = !self.prefs.group_by_date;
                self.save_prefs();
                self.rebuild(ui, t);
            }
        }
    }

    fn save_note_settings(&mut self) {
        if let Err(error) = settings::save(&self.dir, self.note_settings) {
            self.notice = Some(format!("Unable to save settings: {error}"));
        }
    }

    /// The pane over the window, drawn last (above the replace card and the menus). The geometry and the values are
    /// read first, so the label cache borrow does not overlap them.
    pub(super) fn draw_settings(&mut self, s: &mut Surface, ui: &Ui) {
        if !self.settings_open {
            return;
        }
        let radius = ui.pt(m::CARD_RADIUS);
        let pad = ui.pt(SPACE_S);
        let head = ui.pt(PANE_HEAD_PT);
        let (x, y, w, h) = self.pane_rect(ui);
        let done_rect = self.pane_done(ui);
        let rows: Vec<(f32, f32, f32, f32)> = (0..ITEMS.len()).map(|i| self.pane_row(ui, i)).collect();
        let values: Vec<&'static str> = ITEMS.iter().map(|&item| self.setting_value(item)).collect();
        s.fill_rrect(x, y, w, h, radius, CARD_BG.now());
        s.stroke_rrect(x, y, w, h, radius, 1.0, SEPARATOR_C.now());
        let title = self.labels.get(ui, "Settings", MAC_BODY_PT, Weight::Semibold, LABEL.now());
        s.blit(title, (x + pad) as i32, (y + head / 2.0 - title.h as f32 / 2.0) as i32, 255, 255);
        let (dx, dy, dw, dh) = done_rect;
        let done = self.labels.get(ui, "Done", MAC_BODY_PT, Weight::Semibold, LINK);
        s.blit(done, (dx + dw / 2.0 - done.w as f32 / 2.0) as i32, (dy + dh / 2.0 - done.h as f32 / 2.0) as i32, 255, 255);
        for (i, item) in ITEMS.iter().enumerate() {
            let (rx, ry, rw, rh) = rows[i];
            if i > 0 {
                s.fill_rect((rx + pad).max(0.0) as usize, ry as usize, (rw - 2.0 * pad).max(0.0) as usize, 1, SEPARATOR_C.now());
            }
            let label = self.labels.get(ui, item.title(), MAC_BODY_PT, Weight::Regular, LABEL.now());
            s.blit(label, (rx + pad) as i32, (ry + rh / 2.0 - label.h as f32 / 2.0) as i32, 255, 255);
            let value = self.labels.get(ui, values[i], MAC_BODY_PT, Weight::Regular, LINK);
            s.blit(value, (rx + rw - pad - value.w as f32) as i32, (ry + rh / 2.0 - value.h as f32 / 2.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sort_cycle_visits_every_key_and_comes_back() {
        let first = next_sort(store::SortKey::DateEdited);
        assert_eq!(first, store::SortKey::DateCreated);
        assert_eq!(next_sort(first), store::SortKey::Title);
        assert_eq!(next_sort(store::SortKey::Title), store::SortKey::DateEdited);
    }

    #[test]
    fn a_point_selects_the_row_it_is_on_and_no_other() {
        let rows = ITEMS.len();
        assert_eq!(row_index(rows, |i| i == 2), Some(2));
        assert_eq!(row_index(rows, |_| false), None);
    }

    #[test]
    fn the_pane_lists_the_six_settings_in_order() {
        assert_eq!(ITEMS.len(), 6);
        assert_eq!(ITEMS[0].title(), "Text Size");
        assert_eq!(ITEMS[5].title(), "Dark Note Background");
    }
}
