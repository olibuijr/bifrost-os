//! Press and hold, then drag, in the Day and Week grids: on empty space it draws out a new event (the editor opens on it
//! when the pointer is released), on an event it moves the event to the time and day under the pointer. A press that
//! moves at once still scrolls the grid, so a finger can scroll it; the geometry is `layout::drag_create` and
//! `layout::drag_move`.
use super::layout::{self, DragSpan};
use super::timeview::{ALLDAY_H, GRID0};
use super::*;
use bifrost_app::kit::ui::{FIELD_BG, MAC_CALLOUT_PT, MAC_CAPTION1_PT, SPACE_XXL};

/// How long a press must rest before moving the pointer drags instead of scrolling (ms).
pub(super) const HOLD_MS: f64 = 250.0;
/// Events snap to quarter hours.
const SNAP: u32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// a new event, drawn out from the minute where the press landed
    Create { press: u32 },
    /// a timed event, grabbed `grab` minutes after its start
    Move { id: u64, grab: u32, duration: u32 },
}

/// A press in the grid that may become a drag if it is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Press {
    pub t: f64,
    pub kind: Kind,
    /// the day column pressed
    pub day: Date,
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Drag {
    pub kind: Kind,
    /// the day column pressed, and the one the block is in now
    pub origin: Date,
    pub day: Date,
    /// the block in minutes of the day
    pub span: DragSpan,
}

/// The block for `kind` with the pointer at minute `now`.
pub(super) fn span_for(kind: Kind, now: u32) -> DragSpan {
    match kind {
        Kind::Create { press } => layout::drag_create(press, now, SNAP),
        Kind::Move { grab, duration, .. } => layout::drag_move(duration, grab, now, SNAP),
    }
}

/// "10:00 - 11:45".
pub(super) fn span_label(span: DragSpan) -> String {
    format!("{} - {}", time_label(span.start as u16), time_label(span.end as u16))
}

impl Calendar {
    /// The top of the hour grid (below the all-day band) and the height of an hour, in px.
    fn hours_geometry(&self, ui: &Ui) -> (f32, f32) {
        (ui.pt(GRID0 + ALLDAY_H) - self.scroll.offset(), ui.pt(self.prefs.hour_h))
    }

    /// The day column and the minute of the day under content pixel `(x, y)`, when it is in the day columns of the
    /// hour grid (not in the hour labels, the header or the all-day band).
    pub(super) fn grid_at(&self, ui: &Ui, x: f32, y: f32) -> Option<(usize, u32)> {
        if !self.time_view() {
            return None;
        }
        let (top, hh) = self.hours_geometry(ui);
        let (gut, col_w) = self.col_geometry(ui);
        if x < gut || y < ui.pt(GRID0).max(top) || y >= self.bottom(ui) {
            return None;
        }
        let days = self.view_days().len();
        Some((layout::column_at(x, gut, col_w, days), layout::y_minute(y, top, hh, 1)))
    }

    /// Remembers what a press at `(x, y)` would drag if it is held: the event under it, or empty space.
    pub(super) fn begin_press(&mut self, ui: &Ui, x: f32, y: f32, t: f64) {
        self.drag = None;
        self.press = None;
        if self.editor.is_some() || self.goto.is_some() {
            return;
        }
        let Some((col, minute)) = self.grid_at(ui, x, y) else { return };
        let day = self.view_days()[col];
        let under = self.time_rects(ui).into_iter().find(|r| within(x, y, (r.1, r.2, r.3, r.4)));
        let kind = match under {
            Some((id, ..)) => {
                let Some(e) = self.events.iter().find(|e| e.id == id).filter(|e| !e.all_day) else { return };
                Kind::Move { id, grab: minute.saturating_sub(e.minute as u32), duration: e.duration as u32 }
            }
            None => Kind::Create { press: minute },
        };
        self.press = Some(Press { t, kind, day });
    }

    /// Follows the pointer to `(x, y)` once the press has been held: starts the drag, and moves its block. True while a
    /// drag is going on, so the touch is not also a scroll or a tap.
    pub(super) fn track_drag(&mut self, ui: &Ui, x: f32, y: f32, t: f64) -> bool {
        if self.drag.is_none() {
            let Some(p) = self.press else { return false };
            if t - p.t < HOLD_MS || self.scroll.dragging() {
                return false;
            }
            // the scroll gesture that began with the press is given up
            if std::mem::take(&mut self.scrolling) {
                self.scroll.up(y, t);
            }
            self.pressed = None;
            self.drag = Some(Drag { kind: p.kind, origin: p.day, day: p.day, span: span_for(p.kind, 0) });
        }
        let Some(mut d) = self.drag else { return false };
        let (top, hh) = self.hours_geometry(ui);
        let (gut, col_w) = self.col_geometry(ui);
        let days = self.view_days();
        let now = layout::y_minute(y, top, hh, 1);
        // a new event stays in the day it began in; a moved one follows the pointer across the columns
        if matches!(d.kind, Kind::Move { .. }) {
            d.day = days[layout::column_at(x, gut, col_w, days.len())];
        }
        d.span = span_for(d.kind, now);
        self.drag = Some(d);
        true
    }

    /// The pointer was released: a drawn-out event opens in the editor, a moved one is saved in its new place.
    pub(super) fn finish_drag(&mut self, ui: &Ui, t: f64) {
        let Some(d) = self.drag.take() else { return };
        self.press = None;
        match d.kind {
            Kind::Create { .. } => {
                let event = CalendarEvent {
                    id: store::next_id(&self.events),
                    date: d.day,
                    minute: d.span.start as u16,
                    duration: (d.span.end - d.span.start).clamp(15, 1440) as u16,
                    all_day: false,
                    title: String::new(),
                    location: String::new(),
                    notes: String::new(),
                    repeat: 0,
                    alert: self.prefs.alert,
                };
                self.selected = d.day;
                self.open_draft(ui, event, false);
            }
            Kind::Move { id, .. } => self.move_event(ui, id, &d, t),
        }
    }

    /// Puts event `id` where the drag `d` ended: the same length, the new start, and (for a repeating event, the
    /// whole series) the new day.
    fn move_event(&mut self, ui: &Ui, id: u64, d: &Drag, _t: f64) {
        let Some(old) = self.events.iter().find(|e| e.id == id).cloned() else { return };
        let mut moved = old.clone();
        moved.date = old.date.shift_days((d.day.days() - d.origin.days()) as i32);
        moved.minute = d.span.start as u16;
        if moved == old {
            return;
        }
        if self.storage_blocked {
            self.error = Some("Saved events could not be read. Save is unavailable.".into());
            return;
        }
        let mut events = self.events.clone();
        events.retain(|e| e.id != id);
        events.push(moved.clone());
        store::sort(&mut events);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                self.selected = d.day;
                self.picked = Some(id);
                self.error = None;
                self.notice = Some(format!("Moved {} to {}", moved.title, time_label(moved.minute)));
                self.fit_scroll(ui);
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save. The event is where it was.".into());
            }
        }
    }

    /// Draws the block being dragged. `g` is the hour grid's surface; the geometry is the grid's, in px.
    pub(super) fn draw_drag(&mut self, g: &mut Surface, ui: &Ui, hours_top: f32, hh: f32, gut: f32, col_w: f32, days: &[Date]) {
        let Some(d) = self.drag else { return };
        let Some(col) = days.iter().position(|x| *x == d.day) else { return };
        let (x, w) = layout::slot_x(layout::Slot { col: 0, cols: 1 }, gut + col_w * col as f32, col_w, ui.pt(SPACE_XXS));
        let y = layout::minute_y(d.span.start as f32, hours_top, hh);
        let h = ((d.span.end - d.span.start) as f32 / 60.0 * hh - ui.pt(1.0)).max(ui.pt(SPACE_L));
        g.fill_rrect(x, y, w, h, ui.pt(SPACE_XS), FIELD_BG.now());
        g.fill_rrect(x, y, ui.pt(SPACE_XS), h, ui.pt(SPACE_XS) / 2.0, chrome::cal_colour());
        let title = match d.kind {
            Kind::Move { id, .. } => self.events.iter().find(|e| e.id == id).map(|e| e.title.clone()),
            Kind::Create { .. } => None,
        };
        let text_w = w - ui.pt(SPACE_M);
        if text_w > ui.pt(SPACE_L) {
            let mut line = ui.pt(SPACE_S);
            if let Some(title) = title {
                let fit = ui.fit(&title, MAC_CAPTION1_PT, text_w);
                self.text(g, ui, &fit, MAC_CALLOUT_PT, LABEL.now(), x + ui.pt(SPACE_S), y + line);
                line = ui.pt(SPACE_XXL);
            }
            if h >= line + ui.pt(SPACE_L) {
                let label = ui.fit(&span_label(d.span), MAC_SUBHEAD_PT, text_w);
                self.text(g, ui, &label, MAC_SUBHEAD_PT, LABEL_2.now(), x + ui.pt(SPACE_S), y + line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_event_is_drawn_out_in_either_direction() {
        let create = Kind::Create { press: 600 };
        assert_eq!(span_for(create, 690), DragSpan { start: 600, end: 705 });
        assert_eq!(span_for(create, 540), DragSpan { start: 540, end: 615 });
        assert_eq!(span_for(create, 600), DragSpan { start: 600, end: 615 });
    }

    #[test]
    fn a_moved_event_keeps_its_length_and_stays_in_the_day() {
        let mv = Kind::Move { id: 1, grab: 20, duration: 60 };
        assert_eq!(span_for(mv, 800), DragSpan { start: 780, end: 840 });
        assert_eq!(span_for(mv, 5), DragSpan { start: 0, end: 60 });
        assert_eq!(span_for(mv, 1439), DragSpan { start: 1380, end: 1440 });
    }

    #[test]
    fn the_block_is_labelled_with_its_times() {
        assert_eq!(span_label(DragSpan { start: 600, end: 705 }), "10:00 - 11:45");
        assert_eq!(span_label(DragSpan { start: 1380, end: 1440 }), "23:00 - 24:00");
    }
}
