//! Tracking areas (`NativeTrackingArea`): rectangles that report pointer enter, exit and move events. Pure model:
//! the caller reports each pointer position in view points, in the same space as the area rectangles, and gets back
//! the events to deliver, in order: exits, then enters, then moves.

use crate::Rect;

/// Report enter and exit events for the area.
pub const ENTERED_EXITED: u32 = 1 << 0;
/// Report every pointer position inside the area.
pub const MOVED: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub id: u64,
    pub rect: Rect,
    /// `ENTERED_EXITED` and `MOVED`, combined.
    pub options: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Entered(u64),
    Exited(u64),
    Moved(u64, f32, f32),
}

/// Keeps the set of areas the pointer is inside, so each enter and exit is reported once.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    areas: Vec<Area>,
    inside: Vec<u64>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the areas. An area that disappears while the pointer is in it gets its exit now, so the owner's
    /// hover state cannot stay stuck on. An area that stays keeps its inside state.
    pub fn update(&mut self, areas: Vec<Area>) -> Vec<Event> {
        let mut events = Vec::new();
        let gone: Vec<u64> = self
            .inside
            .iter()
            .copied()
            .filter(|id| !areas.iter().any(|a| a.id == *id))
            .collect();
        for id in gone {
            if self.reports(id, ENTERED_EXITED) {
                events.push(Event::Exited(id));
            }
        }
        self.inside.retain(|id| areas.iter().any(|a| a.id == *id));
        self.areas = areas;
        events
    }

    /// The pointer is at (x, y). Overlapping areas each get their own events.
    pub fn pointer(&mut self, x: f32, y: f32) -> Vec<Event> {
        let now: Vec<u64> = self
            .areas
            .iter()
            .filter(|a| a.rect.contains(x, y))
            .map(|a| a.id)
            .collect();
        let mut events = Vec::new();
        for id in &self.inside {
            if !now.contains(id) && self.reports(*id, ENTERED_EXITED) {
                events.push(Event::Exited(*id));
            }
        }
        for id in &now {
            if !self.inside.contains(id) && self.reports(*id, ENTERED_EXITED) {
                events.push(Event::Entered(*id));
            }
        }
        for id in &now {
            if self.reports(*id, MOVED) {
                events.push(Event::Moved(*id, x, y));
            }
        }
        self.inside = now;
        events
    }

    /// The pointer left the view: every area it was in is exited.
    pub fn pointer_left(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self
            .inside
            .iter()
            .filter(|id| self.reports(**id, ENTERED_EXITED))
            .map(|id| Event::Exited(*id))
            .collect();
        self.inside.clear();
        events
    }

    fn reports(&self, id: u64, flag: u32) -> bool {
        self.areas.iter().any(|a| a.id == id && a.options & flag != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn area(id: u64, r: Rect, options: u32) -> Area {
        Area { id, rect: r, options }
    }

    #[test]
    fn enter_move_then_exit_reports_each_once() {
        let mut t = Tracker::new();
        t.update(vec![area(1, rect(0., 0., 100., 100.), ENTERED_EXITED | MOVED)]);
        assert_eq!(t.pointer(5., 5.), vec![Event::Entered(1), Event::Moved(1, 5., 5.)]);
        assert_eq!(t.pointer(6., 5.), vec![Event::Moved(1, 6., 5.)], "no second enter while inside");
        assert_eq!(t.pointer(150., 5.), vec![Event::Exited(1)]);
        assert_eq!(t.pointer(160., 5.), vec![], "outside and no longer tracked");
    }

    #[test]
    fn overlapping_areas_each_get_their_own_events() {
        let mut t = Tracker::new();
        t.update(vec![
            area(1, rect(0., 0., 100., 100.), ENTERED_EXITED),
            area(2, rect(0., 0., 10., 10.), ENTERED_EXITED),
        ]);
        assert_eq!(t.pointer(5., 5.), vec![Event::Entered(1), Event::Entered(2)]);
        assert_eq!(
            t.pointer(50., 50.),
            vec![Event::Exited(2)],
            "leaving the small area while staying in the big one exits only the small one"
        );
    }

    #[test]
    fn move_only_area_never_enters_or_exits() {
        let mut t = Tracker::new();
        t.update(vec![area(7, rect(0., 0., 10., 10.), MOVED)]);
        assert_eq!(t.pointer(3., 3.), vec![Event::Moved(7, 3., 3.)]);
        assert_eq!(t.pointer(50., 50.), vec![], "no exit was reported, so none is owed");
        assert_eq!(t.pointer_left(), vec![]);
    }

    #[test]
    fn pointer_leaving_the_view_exits_every_entered_area() {
        let mut t = Tracker::new();
        t.update(vec![
            area(1, rect(0., 0., 100., 100.), ENTERED_EXITED),
            area(2, rect(0., 0., 50., 50.), MOVED),
        ]);
        t.pointer(10., 10.);
        assert_eq!(t.pointer_left(), vec![Event::Exited(1)]);
        assert_eq!(t.pointer_left(), vec![], "nothing left to exit");
    }

    #[test]
    fn removing_a_hovered_area_exits_it_and_a_kept_area_is_not_re_entered() {
        let mut t = Tracker::new();
        t.update(vec![area(1, rect(0., 0., 100., 100.), ENTERED_EXITED | MOVED)]);
        t.pointer(5., 5.);
        assert_eq!(t.update(vec![]), vec![Event::Exited(1)]);

        t.update(vec![area(3, rect(0., 0., 100., 100.), ENTERED_EXITED)]);
        assert_eq!(t.pointer(5., 5.), vec![Event::Entered(3)]);
        assert_eq!(
            t.update(vec![area(3, rect(0., 0., 100., 100.), ENTERED_EXITED)]),
            vec![],
            "an area that stays keeps its state, so no duplicate enter"
        );
        assert_eq!(t.pointer(6., 6.), vec![], "still inside, nothing new to report");
    }

    #[test]
    fn right_and_bottom_edges_are_outside() {
        let mut t = Tracker::new();
        t.update(vec![area(1, rect(0., 0., 100., 100.), ENTERED_EXITED)]);
        assert_eq!(t.pointer(100., 50.), vec![], "the rectangle is half-open");
        assert_eq!(t.pointer(99.5, 99.5), vec![Event::Entered(1)]);
    }
}
