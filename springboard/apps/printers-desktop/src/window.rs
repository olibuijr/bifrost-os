//! The Print Center window's state and geometry: the toolbar, the printer list on the left, the queue of the selected
//! printer on the right, and the footer. Layout is in points and derives from the window size; the drawing and the taps
//! read the same rectangles. The scheduler is reached through `worker.rs`; the rules are in `model.rs`.

use crate::model::{self, Action, Model, Reply};
use crate::worker::{Link, Request};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{widgets, TouchKind};

/// The printer list's width (points).
pub const SIDE_PT: f32 = 240.0;
/// One printer row (points).
pub const ROW_PT: f32 = 50.0;
/// The gap above the first printer row (points).
const LIST_PAD_PT: f32 = 8.0;
/// The queue's header: the printer's name and its detail line (points).
pub const HEAD_PT: f32 = 84.0;
/// The queue's column headings (points).
pub const COLS_PT: f32 = 28.0;
/// One job row (points).
pub const JOB_ROW_PT: f32 = 30.0;
/// The footer (points).
pub const FOOT_PT: f32 = 30.0;
/// Padding inside the queue pane (points).
pub const PAD_PT: f32 = 20.0;
/// How often the printers are listed again while the window is in the foreground (milliseconds).
pub const POLL_MS: f64 = 5000.0;
/// How soon the loop comes back while a request is in flight (milliseconds).
const BUSY_WAKE_MS: f64 = 50.0;

pub const B_PAUSE: u32 = 1;
pub const B_DEFAULT: u32 = 2;
pub const B_HOLD: u32 = 3;
pub const B_RELEASE: u32 = 4;
pub const B_CANCEL: u32 = 5;
pub const B_REFRESH: u32 = 6;

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The printer at this index of the list.
    Printer(usize),
    /// The job at this index of the selected printer's queue.
    Job(usize),
}

pub struct PrintCenter {
    pub model: Model,
    link: Link,
    /// Requests sent whose replies have not come back.
    in_flight: usize,
    /// When the printers are listed next (loop milliseconds).
    next_poll: f64,
    /// The window size, in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target a press started on; the release acts only when it lands on the same one.
    pub press: Target,
}

impl PrintCenter {
    pub fn new() -> PrintCenter {
        PrintCenter::with_link(Link::start())
    }

    fn with_link(link: Link) -> PrintCenter {
        PrintCenter {
            model: Model::default(),
            link,
            in_flight: 0,
            next_poll: 0.0,
            size: (1000.0, 640.0),
            labels: TextCache::default(),
            press: Target::Nothing,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = (w, h);
    }

    fn send(&mut self, request: Request) {
        self.link.send(request);
        self.in_flight += 1;
    }

    /// Lists the printers, and the queue of the selected one.
    fn list(&mut self) {
        let printer = self.model.selected.clone();
        self.send(Request::List { printer });
    }

    /// Performs `action` on the selected printer.
    fn act(&mut self, action: Action) {
        if let Some(printer) = self.model.selected.clone() {
            self.send(Request::Act { printer, action });
        }
    }

    /// Takes the replies that arrived. A finished action lists again so the window shows the result. True when a frame
    /// is needed.
    fn poll(&mut self) -> bool {
        let replies = self.link.drain();
        if replies.is_empty() {
            return false;
        }
        self.in_flight = self.in_flight.saturating_sub(replies.len());
        for reply in replies {
            let done = matches!(reply, Reply::Done(_));
            self.model.apply(reply);
            if done {
                self.list();
            }
        }
        true
    }

    /// One loop turn in the foreground: takes the replies and lists the printers every `POLL_MS`. True when a frame is
    /// needed.
    pub fn step(&mut self, t: f64) -> bool {
        let changed = self.poll();
        if t >= self.next_poll {
            self.next_poll = t + POLL_MS;
            self.list();
        }
        changed
    }

    /// When the loop should come back: soon while a request is in flight, else at the next listing.
    pub fn wake_at(&self, t: f64) -> f64 {
        if self.in_flight > 0 {
            t + BUSY_WAKE_MS
        } else {
            self.next_poll.max(t + BUSY_WAKE_MS)
        }
    }

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    fn body_h(&self) -> f32 {
        (self.size.1 - UNIFIED_BAR_PT - FOOT_PT).max(0.0)
    }

    /// The printer list, under the toolbar on the left.
    pub fn side_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT.min(self.size.0), h: self.body_h() }
    }

    /// The queue pane, on the right of the printer list.
    pub fn detail_rect(&self) -> Rect {
        let x = SIDE_PT.min(self.size.0);
        Rect { x, y: UNIFIED_BAR_PT, w: (self.size.0 - x).max(0.0), h: self.body_h() }
    }

    pub fn footer_rect(&self) -> Rect {
        Rect { x: 0.0, y: (self.size.1 - FOOT_PT).max(0.0), w: self.size.0, h: FOOT_PT }
    }

    pub fn printer_row(&self, i: usize) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT + LIST_PAD_PT + i as f32 * ROW_PT, w: SIDE_PT, h: ROW_PT }
    }

    /// The top of the queue's column headings.
    pub fn queue_top(&self) -> f32 {
        UNIFIED_BAR_PT + HEAD_PT
    }

    /// The top of the first job row.
    pub fn job_row_top(&self) -> f32 {
        self.queue_top() + COLS_PT
    }

    pub fn job_row(&self, i: usize) -> Rect {
        let d = self.detail_rect();
        Rect { x: d.x + 8.0, y: self.job_row_top() + i as f32 * JOB_ROW_PT, w: (d.w - 16.0).max(0.0), h: JOB_ROW_PT }
    }

    /// The left edges of the queue's columns: job, document, owner, size and status.
    pub fn columns(&self) -> [f32; 5] {
        let d = self.detail_rect();
        let x0 = d.x + PAD_PT;
        let w = (d.w - 2.0 * PAD_PT).max(0.0);
        let (owner_w, size_w, status_w) = (150.0, 80.0, 130.0);
        [x0 + 8.0, x0 + 64.0, x0 + w - status_w - size_w - owner_w, x0 + w - status_w - size_w, x0 + w - status_w]
    }

    /// The toolbar. Each button is enabled only when the selection allows its action.
    pub fn toolbar(&self) -> Toolbar {
        let printer = self.model.selected_printer();
        let job = self.model.selected_job();
        let pause_label = printer.map_or("Pause", |p| model::toggle(p).1);
        Toolbar::new(vec![
            ToolbarItem::button(B_PAUSE, pause_label).enabled(printer.is_some()),
            ToolbarItem::button(B_DEFAULT, "Set Default").enabled(printer.is_some_and(|p| !p.is_default)),
            ToolbarItem::button(B_HOLD, "Hold").enabled(job.is_some_and(model::can_hold)),
            ToolbarItem::button(B_RELEASE, "Release").enabled(job.is_some_and(model::can_release)),
            ToolbarItem::button(B_CANCEL, "Cancel Job").enabled(job.is_some_and(model::can_cancel)),
            ToolbarItem::button(B_REFRESH, "Refresh"),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The footer's words: the last result, or the number of printers.
    pub fn footer_text(&self) -> String {
        if !self.model.status.is_empty() {
            return self.model.status.clone();
        }
        match self.model.printers.len() {
            0 => "No printers are set up".into(),
            1 => "1 printer".into(),
            n => format!("{n} printers"),
        }
    }

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return self.hit_toolbar(ui, x, y);
        }
        self.hit_body(x, y)
    }

    /// What is under `(x, y)` below the toolbar: a printer row or a job row.
    pub fn hit_body(&self, x: f32, y: f32) -> Target {
        if x < SIDE_PT {
            let top = UNIFIED_BAR_PT + LIST_PAD_PT;
            if y >= top {
                let i = ((y - top) / ROW_PT) as usize;
                if i < self.model.printers.len() {
                    return Target::Printer(i);
                }
            }
            return Target::Nothing;
        }
        if y >= self.job_row_top() {
            let i = ((y - self.job_row_top()) / JOB_ROW_PT) as usize;
            if i < self.model.jobs.len() {
                return Target::Job(i);
            }
        }
        Target::Nothing
    }

    fn hit_toolbar(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let bar = self.toolbar();
        self.toolbar_layout(ui)
            .visible
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| Target::Toolbar(i.id)))
            .unwrap_or(Target::Nothing)
    }

    /// A touch in points. A tap is a press and a release on the same target.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32) -> bool {
        match kind {
            TouchKind::Down => {
                self.press = self.hit(ui, x, y);
                false
            }
            TouchKind::Move => false,
            TouchKind::Up => {
                let released = self.hit(ui, x, y);
                let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                if released != pressed {
                    return false;
                }
                self.activate(released)
            }
            TouchKind::Cancel => {
                self.press = Target::Nothing;
                false
            }
        }
    }

    /// Performs what a target asks for. True when the window should redraw.
    pub fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Toolbar(id) => {
                self.toolbar_action(id);
                true
            }
            Target::Printer(i) => {
                let Some(name) = self.model.printers.get(i).map(|p| p.name.clone()) else { return false };
                self.model.select_printer(name);
                self.list();
                true
            }
            Target::Job(i) => {
                let Some(id) = self.model.jobs.get(i).map(|j| j.id) else { return false };
                self.model.select_job(id);
                true
            }
            Target::Nothing => false,
        }
    }

    fn toolbar_action(&mut self, id: u32) {
        let action = match id {
            B_PAUSE => self.model.selected_printer().map(|p| model::toggle(p).0),
            B_DEFAULT => self.model.selected_printer().filter(|p| !p.is_default).map(|_| Action::SetDefault),
            B_HOLD => self.model.selected_job().filter(|j| model::can_hold(j)).map(|j| Action::HoldJob(j.id)),
            B_RELEASE => self.model.selected_job().filter(|j| model::can_release(j)).map(|j| Action::ReleaseJob(j.id)),
            B_CANCEL => self.model.selected_job().filter(|j| model::can_cancel(j)).map(|j| Action::CancelJob(j.id)),
            B_REFRESH => {
                self.list();
                None
            }
            _ => None,
        };
        if let Some(action) = action {
            self.act(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::cups::{Job, JobState, Printer, PrinterState};

    fn printer(name: &str, is_default: bool) -> Printer {
        Printer {
            name: name.into(),
            uri: String::new(),
            state: PrinterState::Idle,
            state_message: String::new(),
            accepting_jobs: true,
            is_default,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: 0,
        }
    }

    fn window() -> PrintCenter {
        let mut w = PrintCenter::with_link(Link::detached());
        w.resize(1000.0, 640.0);
        w.model.printers = vec![printer("Office", true), printer("Lab", false)];
        w.model.selected = Some("Office".into());
        w.model.jobs = vec![Job { id: 9, uri: String::new(), state: JobState::Pending, name: "a.pdf".into(), user: "u".into(), size_kb: 4 }];
        w
    }

    #[test]
    fn the_printer_rows_and_the_job_rows_are_hit_where_they_are_drawn() {
        let w = window();
        let r = w.printer_row(1);
        assert_eq!(w.hit_body(r.x + 5.0, r.y + 5.0), Target::Printer(1));
        let j = w.job_row(0);
        assert_eq!(w.hit_body(j.x + 5.0, j.y + 5.0), Target::Job(0));
        assert_eq!(w.hit_body(j.x + 5.0, j.y + 5.0 + JOB_ROW_PT * 3.0), Target::Nothing);
    }

    #[test]
    fn a_tap_selects_a_printer_and_asks_for_its_queue() {
        let mut w = window();
        assert!(w.activate(Target::Printer(1)));
        assert_eq!(w.model.selected.as_deref(), Some("Lab"));
        assert_eq!(w.in_flight, 1);
        assert!(!w.activate(Target::Printer(7)));
    }

    #[test]
    fn job_buttons_follow_the_selected_job() {
        let mut w = window();
        w.model.select_job(9);
        let bar = w.toolbar();
        let enabled = |id: u32| bar.items.iter().find(|i| i.id == id).map(|i| i.enabled);
        assert_eq!(enabled(B_HOLD), Some(true));
        assert_eq!(enabled(B_RELEASE), Some(false));
        assert_eq!(enabled(B_CANCEL), Some(true));
        assert_eq!(enabled(B_DEFAULT), Some(false));
        assert_eq!(enabled(B_PAUSE), Some(true));
    }

    #[test]
    fn a_hold_sends_the_job_and_no_action_without_a_job() {
        let mut w = window();
        w.toolbar_action(B_HOLD);
        assert_eq!(w.in_flight, 0, "no job is selected, so nothing is sent");
        w.model.select_job(9);
        w.toolbar_action(B_HOLD);
        assert_eq!(w.in_flight, 1);
    }

    #[test]
    fn the_loop_lists_on_a_timer_and_comes_back_soon_while_busy() {
        let mut w = window();
        w.step(0.0);
        assert_eq!(w.in_flight, 1);
        assert_eq!(w.wake_at(0.0), BUSY_WAKE_MS);
        w.in_flight = 0;
        assert_eq!(w.wake_at(10.0), POLL_MS);
        assert!(w.next_poll > 0.0);
    }

    #[test]
    fn the_footer_shows_the_last_result_or_the_printer_count() {
        let mut w = window();
        assert_eq!(w.footer_text(), "2 printers");
        w.model.status = "Printer paused".into();
        assert_eq!(w.footer_text(), "Printer paused");
    }

    #[test]
    fn the_columns_stay_inside_the_queue_pane() {
        let w = window();
        let d = w.detail_rect();
        let cols = w.columns();
        assert!(cols.windows(2).all(|p| p[0] <= p[1]));
        assert!(cols[4] < d.x + d.w);
    }
}
