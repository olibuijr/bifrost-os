//! The quick-event popover (Golden Gate Calendar): the toolbar's "+" and a double-click on an empty slot of the grid make a
//! new event and open a small popover beside it, with the title field focused, the time range, a location field, the
//! calendar pop-up and "Edit Details..." for the full event form. Return adds the event, Escape drops it, and a click
//! outside the popover adds it when it has a title (the popover is transient, `NativePopover.Behavior.transient`).
//! The panel is the kit's `widgets::popover`; its rows are laid out by `layout_in`, which drawing and hit testing share.
use super::intents::NewEvent;
use super::timeview::{ALLDAY_H, GRID0};
use super::*;
use bifrost_app::controls::geometry::Edge;
use bifrost_app::controls::{Node, Rect, Role, TextField as Line};
use bifrost_app::kit::ui::FIELD_BG;
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets::{self, POPOVER_ARROW_PT};

/// The popover's size and its rows (points).
pub(super) const POP_W: f32 = 300.0;
const PAD: f32 = 14.0;
const ROW: f32 = 34.0;
const ROWS: usize = 5;
pub(super) const POP_H: f32 = 2.0 * PAD + ROW * ROWS as f32;
/// The title of an event added with no title typed (as the stock app names it).
pub(super) const UNTITLED: &str = "New Event";
/// Accessibility ids of the popover's controls (clear of the cells 100.., the editor 200.. and the Undo bar 300).
const POP_NODE: u64 = 400;
/// Double-clicks on an empty slot of the Day or Week grid make an event on the half hour under the pointer.
const SLOT_SNAP: u32 = 30;

/// A part of the popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PopPart {
    Title,
    Time,
    Location,
    Calendar,
    Details,
}

/// The new event while its popover is open: nothing is saved until Return or a click outside.
pub(super) struct EventPop {
    pub event: CalendarEvent,
    pub title: Line,
    pub location: Line,
    /// The field that takes the keys: 0 the title, 1 the location.
    pub focus: usize,
    /// The calendar the event goes to (`extras.rs` id).
    pub calendar: u32,
}

/// The rows of a popover whose panel is `panel` (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PopLayout {
    pub panel: Rect,
    pub title: Rect,
    pub time: Rect,
    pub location: Rect,
    pub calendar: Rect,
    pub details: Rect,
}

impl PopLayout {
    /// The part under `(x, y)` (points), None for the panel's padding.
    pub fn part_at(&self, x: f32, y: f32) -> Option<PopPart> {
        [(self.title, PopPart::Title), (self.time, PopPart::Time), (self.location, PopPart::Location), (self.calendar, PopPart::Calendar), (self.details, PopPart::Details)]
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, p)| p)
    }
}

/// Lays the rows out in `panel`: title, time range, location, calendar pop-up (after its colour dot), Edit Details.
pub(super) fn layout_in(panel: Rect) -> PopLayout {
    let (x, w) = (panel.x + PAD, panel.w - 2.0 * PAD);
    let row = |i: usize| Rect { x, y: panel.y + PAD + ROW * i as f32 + 4.0, w, h: ROW - 8.0 };
    let dot = SPACE_L;
    PopLayout { panel, title: row(0), time: row(1), location: row(2), calendar: Rect { x: x + dot, w: w - dot, ..row(3) }, details: row(4) }
}

/// The minute a double-click at minute `m` of the grid starts an event on: the half hour it falls in, so an event of
/// the default length still ends within the day.
pub(super) fn slot_minute(m: u32) -> u16 {
    ((m / SLOT_SNAP) * SLOT_SNAP).min(1440 - SLOT_SNAP) as u16
}

/// "Oct 4, 2026  09:00 - 10:00", or the date alone for an all-day event.
pub(super) fn range_label(e: &CalendarEvent) -> String {
    let day = format!("{} {}, {}", &MONTHS[(e.date.month - 1) as usize][..3], e.date.day, e.date.year);
    if e.all_day {
        return day;
    }
    let end = (e.minute as u32 + e.duration as u32).min(1440) as u16;
    format!("{day}  {} - {}", time_label(e.minute), time_label(end))
}

impl Calendar {
    /// Opens the popover on a new event on `date` at `minute`, of the default length, on the default calendar.
    pub(super) fn open_event_pop(&mut self, date: Date, minute: u16) {
        let event = CalendarEvent {
            id: store::next_id(&self.events),
            date,
            minute,
            duration: self.prefs.duration,
            all_day: false,
            title: String::new(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: self.prefs.alert,
            uid: String::new(),
        };
        self.selected = date;
        self.search_focused = false;
        self.event_pop = Some(EventPop { event, title: Line::new(), location: Line::new(), focus: 0, calendar: self.extras.default_id() });
        self.dirty = true;
    }

    /// Whether the quick-event popover is open.
    pub fn event_pop_open(&self) -> bool {
        self.event_pop.is_some()
    }

    /// The new event's place in the grid (points, window coordinates): its block in the Day and Week grids, its day cell
    /// in the month grid, and the middle of the content otherwise. The popover points at it.
    pub(super) fn pop_anchor(&self, ui: &Ui, e: &CalendarEvent) -> Rect {
        let k = ui.pt(1.0);
        let x0 = self.x0 / k;
        let (w, bottom) = (self.w as f32 / k, self.bottom(ui) / k);
        if self.time_view() {
            if let Some(col) = self.view_days().iter().position(|d| *d == e.date) {
                let (gut, col_w) = self.col_geometry(ui);
                let top = ui.pt(GRID0 + ALLDAY_H) - self.scroll.offset();
                let hh = ui.pt(self.prefs.hour_h);
                let y = layout::minute_y(e.minute as f32, top, hh) / k;
                let h = (e.duration as f32 / 60.0 * hh / k).max(SPACE_L);
                let grid_top = GRID0 + ALLDAY_H;
                let y = y.clamp(grid_top, (bottom - h).max(grid_top));
                return Rect { x: x0 + (gut + col_w * col as f32) / k, y, w: col_w / k, h: h.min(bottom - y).max(1.0) };
            }
        }
        if self.view == View::Month {
            let (left, cell, top, rh) = self.grid_geom(ui);
            let n = (e.date.days() - self.grid_start().days()).clamp(0, 41) as f32;
            let (row, col) = ((n / 7.0).floor(), n % 7.0);
            return Rect { x: x0 + (left + cell * col) / k, y: (top + rh * row) / k, w: cell / k, h: rh / k };
        }
        Rect { x: x0 + w / 2.0, y: GRID0, w: 1.0, h: 1.0 }
    }

    /// Where the popover goes: the window below the toolbar band (points).
    fn pop_bounds(&self, ui: &Ui) -> Rect {
        let k = ui.pt(1.0);
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: self.full_w as f32 / k, h: (self.h as f32 / k - UNIFIED_BAR_PT).max(0.0) }
    }

    /// The popover's rows, while it is open (points, window coordinates). The panel is placed as `widgets::popover`
    /// places it: beside the new event, on its right unless there is no room there.
    pub(super) fn pop_layout(&self, ui: &Ui) -> Option<PopLayout> {
        let pop = self.event_pop.as_ref()?;
        let anchor = self.pop_anchor(ui, &pop.event);
        let (panel, _) = bifrost_app::controls::split::place_popover(anchor, (POP_W, POP_H), self.pop_bounds(ui), Edge::Right, POPOVER_ARROW_PT.1);
        Some(layout_in(panel))
    }

    /// What a press at window pixel `(x, y)` does while the popover is open: a part of it, or (outside it) closes it.
    pub(super) fn pop_target(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let k = ui.pt(1.0);
        let Some(l) = self.pop_layout(ui) else { return Target::PopDismiss };
        let (px, py) = (x / k, y / k);
        if !l.panel.contains(px, py) {
            return Target::PopDismiss;
        }
        // the panel's padding is part of the popover: a press there does nothing (Agenda is the no-op target)
        l.part_at(px, py).map_or(Target::Agenda, Target::Pop)
    }

    /// A click on a part of the popover.
    pub(super) fn pop_tap(&mut self, ui: &Ui, part: PopPart, t: f64) {
        match part {
            PopPart::Title => self.pop_focus(0),
            PopPart::Location => self.pop_focus(1),
            PopPart::Calendar => {
                let choices: Vec<u32> = self.extras.choices().into_iter().map(|(id, _)| id).collect();
                if let Some(pop) = &mut self.event_pop {
                    let at = choices.iter().position(|id| *id == pop.calendar).map_or(0, |i| i + 1);
                    pop.calendar = choices.get(at % choices.len().max(1)).copied().unwrap_or(pop.calendar);
                }
            }
            // the time is set in the full form, as are the repeat, alerts and invitees
            PopPart::Time | PopPart::Details => self.pop_details(ui),
        }
        let _ = t;
        self.dirty = true;
    }

    fn pop_focus(&mut self, field: usize) {
        if let Some(pop) = &mut self.event_pop {
            pop.focus = field;
        }
    }

    /// A key while the popover is open: text and Backspace edit the focused field, Left and Right move in it, Tab goes to
    /// the other field, Return adds the event.
    pub(super) fn pop_key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        if out == Out::Enter {
            self.pop_commit(ui, t);
            return;
        }
        let Some(pop) = &mut self.event_pop else { return };
        if out == Out::Named("Tab") {
            pop.focus = 1 - pop.focus.min(1);
            return;
        }
        let line = if pop.focus == 0 { &mut pop.title } else { &mut pop.location };
        match out {
            Out::Text(text) => line.push_str(text.lines().next().unwrap_or("")),
            Out::Backspace => line.pop(),
            Out::Named("Left") => line.move_caret(false),
            Out::Named("Right") => line.move_caret(true),
            _ => {}
        }
    }

    /// Escape: the popover closes and the new event is dropped. False when it is not open.
    pub(super) fn pop_cancel(&mut self) -> bool {
        self.dirty = true;
        self.event_pop.take().is_some()
    }

    /// Return: adds the event with the typed title ("New Event" when none was typed) and location, on the chosen
    /// calendar, and closes the popover. A save that fails keeps the popover open with the reason.
    pub(super) fn pop_commit(&mut self, ui: &Ui, _t: f64) {
        let Some(pop) = &self.event_pop else { return };
        let title = match pop.title.trim() {
            "" => UNTITLED.to_string(),
            typed => typed.to_string(),
        };
        let new = NewEvent {
            title,
            date: pop.event.date,
            minute: Some(pop.event.minute),
            minutes: Some(pop.event.duration),
            location: pop.location.to_string(),
            notes: String::new(),
            all_day: pop.event.all_day,
        };
        let calendar = pop.calendar;
        match self.create_event(new) {
            Ok(e) => {
                self.event_pop = None;
                self.extras.put(e.id, extras::Extra { calendar, ..Default::default() });
                self.save_extras();
                self.error = None;
                self.fit_scroll(ui);
            }
            Err(e) => self.error = Some(e),
        }
        self.dirty = true;
    }

    /// A click outside the popover: an event with a title typed is added, an untouched one is dropped.
    pub(super) fn pop_dismiss(&mut self, ui: &Ui, t: f64) {
        if self.event_pop.as_ref().is_some_and(|p| !p.title.trim().is_empty()) {
            self.pop_commit(ui, t);
        } else {
            self.event_pop = None;
        }
    }

    /// Edit Details...: the full event form on the new event, with what was typed and the chosen calendar.
    pub(super) fn pop_details(&mut self, ui: &Ui) {
        let Some(pop) = self.event_pop.take() else { return };
        let mut event = pop.event;
        event.title = pop.title.trim().to_string();
        event.location = pop.location.trim().to_string();
        self.open_draft(ui, event, false);
        if let Some(draft) = &mut self.editor {
            draft.extra.calendar = pop.calendar;
        }
    }

    /// Records a click on the empty slot `(date, minute)`; true when it is the second click of a double-click there.
    pub(super) fn slot_double(&mut self, date: Date, minute: u16, t: f64) -> bool {
        let double = self.last_slot.is_some_and(|(d, m, at)| d == date && m == minute && t - at <= window::DOUBLE_TAP_MS);
        self.last_slot = if double { None } else { Some((date, minute, t)) };
        double
    }

    /// The empty slot a click on `target` at content pixel `(x, y)` is on: the day and the half hour under it in the Day
    /// and Week grids, a day of the month grid (at the default 09:00). None for anything else.
    pub(super) fn slot_of(&self, ui: &Ui, target: Target, x: f32, y: f32) -> Option<(Date, u16)> {
        match target {
            Target::Agenda if self.time_view() => {
                let (col, minute) = self.grid_at(ui, x, y)?;
                Some((*self.view_days().get(col)?, slot_minute(minute)))
            }
            Target::Day(day) if self.view == View::Month => Some((Date { day, ..self.selected }, 9 * 60)),
            Target::Pick(date) if self.view == View::Month => Some((date, 9 * 60)),
            _ => None,
        }
    }

    /// The new event's block in the grid and the popover beside it, over everything else (`draw` calls it last).
    pub(super) fn draw_event_pop(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(l) = self.pop_layout(ui) else { return };
        let Some(pop) = &self.event_pop else { return };
        let (event, calendar, focus) = (pop.event.clone(), pop.calendar, pop.focus);
        let (title, location) = (pop.title.clone(), pop.location.clone());
        let anchor = self.pop_anchor(ui, &event);
        let colour = self.calendar_colour(calendar);
        if self.time_view() {
            // the new event's block, where it will be saved
            let (x, y, w, h) = (ui.pt(anchor.x + SPACE_XXS), ui.pt(anchor.y), ui.pt(anchor.w - 2.0 * SPACE_XXS), ui.pt(anchor.h));
            s.fill_rrect(x, y, w, h, ui.pt(SPACE_XS), FIELD_BG.now());
            s.fill_rrect(x, y, ui.pt(SPACE_XS), h, ui.pt(SPACE_XS) / 2.0, colour);
            let name = if title.trim().is_empty() { UNTITLED } else { title.trim() };
            let fit = ui.fit(name, MAC_SUBHEAD_PT, (w - ui.pt(SPACE_M)).max(0.0));
            self.text(s, ui, &fit, MAC_SUBHEAD_PT, LABEL.now(), x + ui.pt(SPACE_S), y + ui.pt(SPACE_S + 2.0));
        }
        let panel = widgets::popover(s, ui, anchor, (POP_W, POP_H), self.pop_bounds(ui), Edge::Right);
        let l = if panel == l.panel { l } else { layout_in(panel) };
        widgets::text_field(s, ui, &title, l.title, UNTITLED, focus == 0);
        let range = range_label(&event);
        self.text(s, ui, &range, MAC_BODY_PT, LABEL_2.now(), ui.pt(l.time.x + SPACE_XS), ui.pt(l.time.y + l.time.h / 2.0));
        widgets::text_field(s, ui, &location, l.location, "Add Location", focus == 1);
        let d = SPACE_S;
        s.fill_rrect(ui.pt(l.calendar.x - SPACE_L + (SPACE_L - d) / 2.0), ui.pt(l.calendar.y + (l.calendar.h - d) / 2.0), ui.pt(d), ui.pt(d), ui.pt(d / 2.0), colour);
        widgets::popup_button(s, ui, l.calendar, &self.extras.name_of(calendar), false, true);
        self.text(s, ui, "Edit Details...", MAC_BODY_PT, colors::LINK.now(), ui.pt(l.details.x + SPACE_XS), ui.pt(l.details.y + l.details.h / 2.0));
    }

    /// The popover's fields and buttons for assistive technology (points).
    pub(super) fn pop_nodes(&self, ui: &Ui) -> Vec<Node> {
        let (Some(l), Some(pop)) = (self.pop_layout(ui), &self.event_pop) else { return vec![] };
        let node = |i: u64, role: Role, label: &str, value: String, bounds: Rect| Node { id: POP_NODE + i, role, label: label.into(), value, bounds, enabled: true };
        vec![
            node(0, Role::TextField, "Title", pop.title.to_string(), l.title),
            node(1, Role::StaticText, "Time", range_label(&pop.event), l.time),
            node(2, Role::TextField, "Location", pop.location.to_string(), l.location),
            node(3, Role::Button, "Calendar", self.extras.name_of(pop.calendar), l.calendar),
            node(4, Role::Button, "Edit Details", String::new(), l.details),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_are_stacked_inside_the_panel_and_hit_where_they_are_drawn() {
        let l = layout_in(Rect { x: 100.0, y: 50.0, w: POP_W, h: POP_H });
        assert!(l.title.y < l.time.y && l.time.y < l.location.y && l.location.y < l.calendar.y && l.calendar.y < l.details.y);
        assert!(l.details.bottom() <= l.panel.bottom() - PAD + 0.01, "the last row stays inside the padding");
        assert!(l.calendar.x > l.title.x, "the calendar pop-up leaves room for its colour dot");
        assert_eq!(l.part_at(l.title.x + 5.0, l.title.y + 5.0), Some(PopPart::Title));
        assert_eq!(l.part_at(l.details.x + 5.0, l.details.y + 5.0), Some(PopPart::Details));
        assert_eq!(l.part_at(l.panel.x + 1.0, l.panel.y + 1.0), None, "the padding is no control");
    }

    #[test]
    fn a_double_clicked_slot_starts_on_the_half_hour() {
        assert_eq!(slot_minute(0), 0);
        assert_eq!(slot_minute(29), 0);
        assert_eq!(slot_minute(30), 30);
        assert_eq!(slot_minute(14 * 60 + 47), 14 * 60 + 30);
        assert_eq!(slot_minute(1439), 1410, "the last slot of the day");
    }

    #[test]
    fn the_time_row_reads_the_range() {
        let e = CalendarEvent {
            id: 1,
            date: Date { year: 2026, month: 9, day: 10 },
            minute: 18 * 60,
            duration: 180,
            all_day: false,
            title: String::new(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: 0,
            uid: String::new(),
        };
        assert_eq!(range_label(&e), "Sep 10, 2026  18:00 - 21:00");
        assert_eq!(range_label(&CalendarEvent { all_day: true, ..e }), "Sep 10, 2026");
    }
}
