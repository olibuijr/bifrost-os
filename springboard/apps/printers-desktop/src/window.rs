//! The Print Center window's state and geometry: the toolbar, the printer list on the left, the queue of the selected
//! printer on the right, and the footer. Layout is in points and derives from the window size; the drawing and the taps
//! read the same rectangles. The scheduler is reached through `worker.rs`; the rules are in `model.rs`.

use crate::model::{self, Action, Model, Reply};
use crate::worker::{Link, Request};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::proto::ContextItem;
use bifrost_app::{widgets, AppMenu, AppMenuItem, Command, TouchKind};
use std::io::Read;

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
pub const B_ADD: u32 = 7;
pub const B_DELETE: u32 = 8;
pub const B_INFO: u32 = 9;
pub const B_SHARE: u32 = 10;
pub const B_SELECT_ALL: u32 = 11;
/// Menu only: asks for a document to print.
pub const B_PRINT_FILE: u32 = 12;
/// Info pane: saves the description and location.
pub const B_SAVE: u32 = 13;

/// A text field of the info pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Description,
    Location,
}

/// The largest document the window reads to print (bytes).
const MAX_DOCUMENT: u64 = 64 * 1024 * 1024;

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The printer at this index of the list.
    Printer(usize),
    /// The job at this index of the selected printer's queue.
    Job(usize),
    /// A text field of the info pane.
    Field(Field),
    /// The Save button of the info pane.
    Save,
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
    /// The time now (Unix seconds) and the zone's offset from UTC (seconds), for the Created column. Read again with
    /// each listing.
    pub now_secs: i64,
    pub tz_offset: i32,
    /// The Dock badge last set.
    badge: String,
    /// The open panel asked for a document to print.
    pub open_request: Option<u32>,
    /// The info pane's fields and which one has the keyboard.
    pub description: TextField,
    pub location: TextField,
    pub focus: Option<Field>,
    /// The printer the fields were filled from, so a listing never overwrites what is being typed.
    loaded_for: Option<String>,
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
            now_secs: 0,
            tz_offset: 0,
            badge: String::new(),
            open_request: None,
            description: TextField::new(),
            location: TextField::new(),
            focus: None,
            loaded_for: None,
        }
    }

    /// Reads the clock and the zone for the Created column.
    fn refresh_clock(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        self.now_secs = now;
        self.tz_offset = bifrost_app::time_zone::TimeZone::current().seconds_from_gmt(bifrost_app::coding::Date(now as f64));
    }

    /// A window with no scheduler behind it, for tests of the code that drives it.
    #[cfg(test)]
    pub fn detached() -> PrintCenter {
        PrintCenter::with_link(Link::detached())
    }

    /// How many requests were sent whose replies have not come back.
    #[cfg(test)]
    pub fn requests_sent(&self) -> usize {
        self.in_flight
    }

    /// Performs `action` on the named printer (a control method, not the selection).
    pub fn request(&mut self, printer: String, action: Action) {
        self.send(Request::Act { printer, action });
    }

    /// Fills the info fields from the selected printer.
    fn load_fields(&mut self) {
        let (info, location, name) = match self.model.selected_printer() {
            Some(p) => (p.info.clone(), p.location.clone(), Some(p.name.clone())),
            None => (String::new(), String::new(), None),
        };
        self.description.clear();
        self.description.push_str(&info);
        self.location.clear();
        self.location.push_str(&location);
        self.loaded_for = name;
        self.focus = None;
    }

    /// Fills the fields again when the info pane shows another printer than they were filled from.
    fn sync_fields(&mut self) {
        if self.model.show_info && self.loaded_for != self.model.selected {
            self.load_fields();
        }
    }

    /// Sends the edited description and location of the selected printer.
    fn save_details(&mut self) {
        if self.model.selected_printer().is_some_and(|p| !p.is_class) {
            let info = self.description.to_string().trim().to_string();
            let location = self.location.to_string().trim().to_string();
            self.act(Action::SetDetails { info, location });
        }
    }

    /// True when the info pane offers the fields: a printer that is not a pool.
    pub fn fields_shown(&self) -> bool {
        self.model.show_info && self.model.selected_printer().is_some_and(|p| !p.is_class)
    }

    fn field_mut(&mut self) -> Option<&mut TextField> {
        match self.focus? {
            Field::Description => Some(&mut self.description),
            Field::Location => Some(&mut self.location),
        }
    }

    /// Typed text goes to the focused field. True when a frame is needed.
    pub fn text(&mut self, text: &str) -> bool {
        match self.field_mut() {
            Some(f) => {
                f.push_str(text);
                true
            }
            None => false,
        }
    }

    /// Editing keys (shell key codes) for the focused field: backspace, delete, arrows, home, end, tab, return and escape.
    pub fn key(&mut self, code: u32) -> bool {
        let Some(focus) = self.focus else { return false };
        match code {
            13 => self.save_details(),
            9 => self.focus = Some(if focus == Field::Description { Field::Location } else { Field::Description }),
            27 | 1 => self.focus = None,
            8 | 46 | 37 | 39 | 36 | 35 => {
                let Some(f) = self.field_mut() else { return false };
                match code {
                    8 => f.pop(),
                    46 => f.delete(),
                    37 | 39 => f.move_caret(code == 39),
                    36 => f.home(),
                    _ => f.end(),
                }
            }
            _ => return false,
        }
        true
    }

    /// A copy, cut, paste or select command for the focused field. True when it was taken.
    pub fn command(&mut self, command: Command) -> bool {
        let clipboard = bifrost_app::kit::clipboard::text().unwrap_or_default();
        let Some(f) = self.field_mut() else { return false };
        if let Some(text) = f.command(command, &clipboard) {
            bifrost_app::kit::clipboard::set_text(&text);
        }
        true
    }

    /// The announcements for new printer problems, each returned once: a title and a body.
    pub fn take_alerts(&mut self) -> Vec<(String, String)> {
        self.model.take_alerts().into_iter().map(|(printer, problem)| (format!("{printer} needs attention"), problem)).collect()
    }

    /// The new Dock badge text when the number of waiting jobs changed ("" clears it).
    pub fn badge_change(&mut self) -> Option<String> {
        let text = model::badge_text(&self.model.printers);
        if text == self.badge {
            return None;
        }
        self.badge = text.clone();
        Some(text)
    }

    /// Sends a document to the selected printer. The footer says why when it cannot.
    pub fn print_reader(&mut self, name: &str, reader: &mut dyn Read) {
        let Some(printer) = self.model.selected.clone() else {
            self.model.status = "Choose a printer, then print the document".into();
            return;
        };
        let mut data = Vec::new();
        match reader.take(MAX_DOCUMENT + 1).read_to_end(&mut data) {
            Ok(_) if data.len() as u64 > MAX_DOCUMENT => self.model.status = format!("\"{name}\" is too large to print from here"),
            Ok(0) => self.model.status = format!("\"{name}\" is empty"),
            Ok(_) => {
                self.model.status = format!("Printing \"{name}\"…");
                self.send(Request::Print { printer, name: name.to_string(), mime: model::mime_for(name).to_string(), data });
            }
            Err(e) => self.model.status = format!("Could not read \"{name}\": {e}"),
        }
    }

    /// Prints the file at `path` (a drop on the window or the Dock icon).
    pub fn print_path(&mut self, path: &str) {
        let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        match std::fs::File::open(path) {
            Ok(mut file) => self.print_reader(&name, &mut file),
            Err(e) => self.model.status = format!("Could not open \"{name}\": {e}"),
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

    /// Add Printer: the address on the clipboard is added; anything else leaves the footer saying how to add one.
    fn add_from_clipboard(&mut self) {
        let text = bifrost_app::kit::clipboard::text();
        match model::device_from_clipboard(text.as_deref()) {
            Ok(device) => {
                self.model.status = "Adding printer…".into();
                self.send(Request::Add { device });
            }
            Err(message) => self.model.status = message.into(),
        }
    }

    /// Delete Printer takes two requests: the first asks the person to press again, the second removes the queue.
    fn delete_selected(&mut self) {
        let Some(name) = self.model.selected.clone() else { return };
        if self.model.confirm_delete.as_deref() == Some(name.as_str()) {
            self.model.confirm_delete = None;
            let pool = self.model.selected_printer().is_some_and(|p| p.is_class);
            self.act(if pool { Action::DeletePool } else { Action::Delete });
        } else {
            self.model.status = format!("Press Delete Printer again to remove \"{name}\"");
            self.model.confirm_delete = Some(name);
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
        self.refresh_clock();
        for reply in replies {
            let done = matches!(reply, Reply::Done(_) | Reply::Added(_) | Reply::Printed(_));
            self.model.apply(reply);
            if done {
                self.list();
            }
        }
        self.sync_fields();
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

    /// The left edge of the info pane's values (points).
    fn info_x(&self) -> f32 {
        self.detail_rect().x + PAD_PT + 110.0
    }

    /// The info pane's text field.
    pub fn field_rect(&self, field: Field) -> Rect {
        let d = self.detail_rect();
        let w = (d.w - 2.0 * PAD_PT - 110.0).clamp(0.0, 320.0);
        let row = if field == Field::Description { 0.0 } else { 1.0 };
        Rect { x: self.info_x(), y: self.queue_top() + 8.0 + row * 36.0, w, h: 26.0 }
    }

    /// The info pane's Save button.
    pub fn save_rect(&self) -> Rect {
        Rect { x: self.info_x(), y: self.queue_top() + 80.0, w: 80.0, h: 26.0 }
    }

    /// The top of the info pane's read-only lines: under the fields when there are fields.
    pub fn info_lines_top(&self) -> f32 {
        self.queue_top() + if self.fields_shown() { 124.0 } else { 8.0 }
    }

    /// The left edges of the queue's columns: job, document, owner, pages, size, created and status.
    pub fn columns(&self) -> [f32; 7] {
        let d = self.detail_rect();
        let x0 = d.x + PAD_PT;
        let w = (d.w - 2.0 * PAD_PT).max(0.0);
        let (owner_w, pages_w, size_w, created_w, status_w) = (100.0, 52.0, 70.0, 100.0, 130.0);
        let status = x0 + w - status_w;
        let created = status - created_w;
        let size = created - size_w;
        let pages = size - pages_w;
        [x0 + 8.0, x0 + 64.0, pages - owner_w, pages, size, created, status]
    }

    /// The toolbar. Each button is enabled only when the selection allows its action.
    pub fn toolbar(&self) -> Toolbar {
        let printer = self.model.selected_printer();
        let pause_label = printer.map_or("Pause", |p| model::toggle(p).1);
        let confirming = printer.is_some_and(|p| self.model.confirm_delete.as_deref() == Some(p.name.as_str()));
        let delete_label = if confirming { "Confirm Delete" } else { "Delete Printer" };
        Toolbar::new(vec![
            ToolbarItem::button(B_PAUSE, pause_label).enabled(printer.is_some()),
            ToolbarItem::button(B_DEFAULT, "Set Default").enabled(printer.is_some_and(|p| !p.is_default)),
            ToolbarItem::button(B_HOLD, "Hold").enabled(!self.model.chosen_ids(model::can_hold).is_empty()),
            ToolbarItem::button(B_RELEASE, "Release").enabled(!self.model.chosen_ids(model::can_release).is_empty()),
            ToolbarItem::button(B_CANCEL, "Cancel Job").enabled(!self.model.chosen_ids(model::can_cancel).is_empty()),
            ToolbarItem::button(B_DELETE, delete_label).enabled(printer.is_some()),
            ToolbarItem::button(B_INFO, if self.model.show_info { "Show Queue" } else { "Info" }).enabled(printer.is_some()),
            ToolbarItem::button(B_ADD, "Add Printer"),
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
        if self.fields_shown() {
            for field in [Field::Description, Field::Location] {
                if self.field_rect(field).contains(x, y) {
                    return Target::Field(field);
                }
            }
            if self.save_rect().contains(x, y) {
                return Target::Save;
            }
        }
        if y >= self.job_row_top() && !self.model.show_info {
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
                if let (Target::Job(a), Target::Job(b)) = (pressed, released) {
                    if a != b {
                        return self.select_job_range(a, b);
                    }
                }
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

    /// Chooses the jobs from row `a` to row `b`, both included, in either order (a drag over the rows).
    pub fn select_job_range(&mut self, a: usize, b: usize) -> bool {
        let (lo, hi) = (a.min(b), a.max(b));
        let ids: Vec<i32> = self.model.jobs.iter().enumerate().filter(|(i, _)| (lo..=hi).contains(i)).map(|(_, j)| j.id).collect();
        if ids.is_empty() {
            return false;
        }
        self.model.selected_jobs = ids.into_iter().collect();
        true
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
                self.sync_fields();
                self.list();
                true
            }
            Target::Field(f) => {
                self.focus = Some(f);
                true
            }
            Target::Save => {
                self.save_details();
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

    /// Runs `make` for each job id.
    fn act_jobs(&mut self, ids: Vec<i32>, make: fn(i32) -> Action) {
        for id in ids {
            self.act(make(id));
        }
    }

    /// Runs the command a toolbar button, a menu item or a context-menu item stands for.
    pub fn toolbar_action(&mut self, id: u32) {
        match id {
            B_PAUSE => {
                if let Some(action) = self.model.selected_printer().map(|p| model::toggle(p).0) {
                    self.act(action);
                }
            }
            B_DEFAULT => {
                if self.model.selected_printer().is_some_and(|p| !p.is_default) {
                    self.act(Action::SetDefault);
                }
            }
            B_HOLD => {
                let ids = self.model.chosen_ids(model::can_hold);
                self.act_jobs(ids, Action::HoldJob);
            }
            B_RELEASE => {
                let ids = self.model.chosen_ids(model::can_release);
                self.act_jobs(ids, Action::ReleaseJob);
            }
            B_CANCEL => {
                let ids = self.model.chosen_ids(model::can_cancel);
                self.act_jobs(ids, Action::CancelJob);
            }
            B_REFRESH => self.list(),
            B_ADD => self.add_from_clipboard(),
            B_DELETE => self.delete_selected(),
            B_INFO => {
                self.model.show_info = !self.model.show_info;
                if self.model.show_info {
                    self.load_fields();
                }
                self.focus = None;
            }
            B_SAVE => self.save_details(),
            B_SHARE => {
                let shared = self.model.selected_printer().filter(|p| !p.is_class).map(|p| !p.shared);
                if let Some(shared) = shared {
                    self.act(Action::SetShared(shared));
                }
            }
            B_SELECT_ALL => self.model.select_all_jobs(),
            _ => {}
        }
    }

    /// The items of the context menu for a secondary click on `target`, after the click chose what it landed on (a job
    /// outside the chosen ones becomes the only chosen job; a printer is selected). Empty when it landed on nothing.
    pub fn context_items(&mut self, target: Target) -> Vec<ContextItem> {
        let item = |id: u32, label: &str, enabled: bool| ContextItem { id, label: label.to_string(), enabled };
        match target {
            Target::Job(i) => {
                let Some(id) = self.model.jobs.get(i).map(|j| j.id) else { return Vec::new() };
                if !self.model.selected_jobs.contains(&id) {
                    self.model.select_job(id);
                }
                let many = self.model.selected_jobs.len() > 1;
                let (hold, release, cancel) = if many { ("Hold Jobs", "Release Jobs", "Cancel Jobs") } else { ("Hold Job", "Release Job", "Cancel Job") };
                vec![
                    item(B_HOLD, hold, !self.model.chosen_ids(model::can_hold).is_empty()),
                    item(B_RELEASE, release, !self.model.chosen_ids(model::can_release).is_empty()),
                    item(B_CANCEL, cancel, !self.model.chosen_ids(model::can_cancel).is_empty()),
                    item(B_SELECT_ALL, "Select All Jobs", true),
                ]
            }
            Target::Printer(i) => {
                let Some(name) = self.model.printers.get(i).map(|p| p.name.clone()) else { return Vec::new() };
                if self.model.selected.as_deref() != Some(name.as_str()) {
                    self.model.select_printer(name);
                    self.list();
                }
                let Some(p) = self.model.selected_printer() else { return Vec::new() };
                let (resume, default_enabled, pool, shared) = (model::toggle(p).1 == "Resume", !p.is_default, p.is_class, p.shared);
                let mut items = vec![
                    item(B_PAUSE, if resume { "Resume Printer" } else { "Pause Printer" }, true),
                    item(B_DEFAULT, "Set as Default Printer", default_enabled),
                    item(B_INFO, "Printer Info", true),
                ];
                if !pool {
                    items.push(item(B_SHARE, if shared { "Stop Sharing" } else { "Share Printer" }, true));
                }
                items.push(item(B_DELETE, if pool { "Remove Printer Pool…" } else { "Remove Printer…" }, true));
                items
            }
            Target::Toolbar(_) | Target::Nothing | Target::Field(_) | Target::Save => Vec::new(),
        }
    }

    /// The window's own menus in the menu bar.
    pub fn menus(&self) -> Vec<AppMenu> {
        let printer = self.model.selected_printer();
        let resume = printer.is_some_and(|p| model::toggle(p).1 == "Resume");
        let shared = printer.is_some_and(|p| p.shared);
        let plural = self.model.selected_jobs.len() > 1;
        let word = |one: &'static str, many: &'static str| if plural { many } else { one };
        vec![
            AppMenu {
                title: "Printer".into(),
                items: vec![
                    AppMenuItem::new(B_ADD, "Add Printer from Clipboard").key("cmd+n"),
                    AppMenuItem::new(B_PRINT_FILE, "Print Document…").key("cmd+p").enabled(printer.is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(B_PAUSE, if resume { "Resume Printer" } else { "Pause Printer" }).enabled(printer.is_some()),
                    AppMenuItem::new(B_DEFAULT, "Set as Default Printer").enabled(printer.is_some_and(|p| !p.is_default)),
                    AppMenuItem::new(B_INFO, "Printer Info").key("cmd+i").checked(self.model.show_info).enabled(printer.is_some()),
                    AppMenuItem::new(B_SHARE, if shared { "Stop Sharing" } else { "Share Printer" }).enabled(printer.is_some_and(|p| !p.is_class)),
                    AppMenuItem::separator(),
                    AppMenuItem::new(B_DELETE, "Remove Printer…").enabled(printer.is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(B_REFRESH, "Refresh").key("cmd+r"),
                ],
            },
            AppMenu {
                title: "Jobs".into(),
                items: vec![
                    AppMenuItem::new(B_HOLD, word("Pause Selected Job", "Pause Selected Jobs")).enabled(!self.model.chosen_ids(model::can_hold).is_empty()),
                    AppMenuItem::new(B_RELEASE, word("Resume Selected Job", "Resume Selected Jobs")).enabled(!self.model.chosen_ids(model::can_release).is_empty()),
                    AppMenuItem::new(B_CANCEL, word("Cancel Selected Job", "Cancel Selected Jobs")).key("cmd+.").enabled(!self.model.chosen_ids(model::can_cancel).is_empty()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(B_SELECT_ALL, "Select All Jobs").key("cmd+a").enabled(!self.model.jobs.is_empty()),
                ],
            },
        ]
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
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    fn job(id: i32, state: JobState) -> Job {
        Job { id, uri: String::new(), state, name: format!("{id}.pdf"), user: "u".into(), size_kb: 4, reasons: Vec::new(), created: 0, pages: 0, pages_done: 0, progress: None }
    }

    fn window() -> PrintCenter {
        let mut w = PrintCenter::with_link(Link::detached());
        w.resize(1000.0, 640.0);
        w.model.printers = vec![printer("Office", true), printer("Lab", false)];
        w.model.selected = Some("Office".into());
        w.model.jobs = vec![job(9, JobState::Pending)];
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
    fn delete_printer_asks_once_more_before_it_removes_the_queue() {
        let mut w = window();
        w.toolbar_action(B_DELETE);
        assert_eq!(w.in_flight, 0, "the first press only asks");
        assert_eq!(w.model.confirm_delete.as_deref(), Some("Office"));
        assert!(w.model.status.contains("Office"));
        w.toolbar_action(B_DELETE);
        assert_eq!(w.in_flight, 1, "the second press removes it");
        assert_eq!(w.model.confirm_delete, None);
    }

    #[test]
    fn choosing_another_printer_drops_the_pending_delete() {
        let mut w = window();
        w.toolbar_action(B_DELETE);
        assert!(w.activate(Target::Printer(1)));
        assert_eq!(w.model.confirm_delete, None);
    }

    #[test]
    fn add_printer_without_an_address_on_the_clipboard_says_how() {
        let mut w = window();
        w.toolbar_action(B_ADD);
        assert_eq!(w.in_flight, 0);
        assert!(w.model.status.starts_with("Copy a printer address"));
    }

    #[test]
    fn a_job_action_is_sent_for_every_chosen_job_that_allows_it() {
        let mut w = window();
        w.model.jobs = vec![job(1, JobState::Pending), job(2, JobState::Pending), job(3, JobState::PendingHeld)];
        w.model.select_all_jobs();
        w.toolbar_action(B_HOLD);
        assert_eq!(w.in_flight, 2, "the held job is skipped");
        w.toolbar_action(B_RELEASE);
        assert_eq!(w.in_flight, 3);
        w.toolbar_action(B_CANCEL);
        assert_eq!(w.in_flight, 6);
    }

    #[test]
    fn select_all_jobs_chooses_the_whole_queue() {
        let mut w = window();
        w.model.jobs = vec![job(1, JobState::Pending), job(2, JobState::Pending)];
        w.toolbar_action(B_SELECT_ALL);
        assert_eq!(w.model.selected_jobs.len(), 2);
        let bar = w.toolbar();
        assert!(bar.items.iter().find(|i| i.id == B_CANCEL).is_some_and(|i| i.enabled));
    }

    #[test]
    fn a_secondary_click_on_a_job_chooses_it_and_offers_the_job_actions() {
        let mut w = window();
        w.model.jobs = vec![job(1, JobState::Pending), job(2, JobState::PendingHeld)];
        let items = w.context_items(Target::Job(1));
        assert_eq!(w.model.selected_jobs.iter().copied().collect::<Vec<_>>(), vec![2]);
        let enabled = |id: u32| items.iter().find(|i| i.id == id).map(|i| i.enabled);
        assert_eq!(enabled(B_HOLD), Some(false));
        assert_eq!(enabled(B_RELEASE), Some(true));
        assert_eq!(enabled(B_CANCEL), Some(true));
        // a click inside the chosen jobs keeps them all, and the labels turn plural
        w.model.select_all_jobs();
        let items = w.context_items(Target::Job(0));
        assert_eq!(w.model.selected_jobs.len(), 2);
        assert_eq!(items.iter().find(|i| i.id == B_CANCEL).map(|i| i.label.as_str()), Some("Cancel Jobs"));
        assert!(w.context_items(Target::Job(9)).is_empty());
        assert!(w.context_items(Target::Nothing).is_empty());
    }

    #[test]
    fn a_secondary_click_on_a_printer_selects_it_and_offers_its_actions() {
        let mut w = window();
        let items = w.context_items(Target::Printer(1));
        assert_eq!(w.model.selected.as_deref(), Some("Lab"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["Pause Printer", "Set as Default Printer", "Printer Info", "Share Printer", "Remove Printer…"]);
        w.model.printers[1].is_class = true;
        let items = w.context_items(Target::Printer(1));
        assert!(items.iter().any(|i| i.label == "Remove Printer Pool…"));
        assert!(!items.iter().any(|i| i.id == B_SHARE), "a pool cannot be shared");
    }

    #[test]
    fn the_menus_follow_the_selection_and_use_the_job_menu_words() {
        let mut w = window();
        w.model.jobs = vec![job(1, JobState::Pending), job(2, JobState::Pending)];
        let find = |menus: &[AppMenu], id: u32| menus.iter().flat_map(|m| m.items.iter()).find(|i| i.id == id).cloned();
        let menus = w.menus();
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["Printer", "Jobs"]);
        assert_eq!(find(&menus, B_HOLD).map(|i| (i.title, i.enabled)), Some(("Pause Selected Job".to_string(), false)));
        w.model.select_all_jobs();
        let menus = w.menus();
        assert_eq!(find(&menus, B_HOLD).map(|i| (i.title, i.enabled)), Some(("Pause Selected Jobs".to_string(), true)));
        assert_eq!(find(&menus, B_CANCEL).map(|i| i.key), Some("cmd+.".to_string()));
        w.toolbar_action(B_INFO);
        assert!(find(&w.menus(), B_INFO).is_some_and(|i| i.checked));
    }

    #[test]
    fn the_info_pane_hides_the_job_rows() {
        let mut w = window();
        let j = w.job_row(0);
        assert_eq!(w.hit_body(j.x + 5.0, j.y + 5.0), Target::Job(0));
        w.toolbar_action(B_INFO);
        assert_eq!(w.hit_body(j.x + 5.0, j.y + 5.0), Target::Nothing);
        w.toolbar_action(B_INFO);
        assert_eq!(w.hit_body(j.x + 5.0, j.y + 5.0), Target::Job(0));
    }

    #[test]
    fn sharing_flips_and_a_pool_is_removed_as_a_pool() {
        let mut w = window();
        w.toolbar_action(B_SHARE);
        assert_eq!(w.in_flight, 1);
        w.model.printers[0].is_class = true;
        w.toolbar_action(B_SHARE);
        assert_eq!(w.in_flight, 1, "a pool is not shared");
        w.toolbar_action(B_DELETE);
        w.toolbar_action(B_DELETE);
        assert_eq!(w.in_flight, 2);
    }

    #[test]
    fn a_document_is_sent_to_the_selected_printer_and_the_footer_says_why_not() {
        let mut w = window();
        w.print_reader("a.pdf", &mut std::io::Cursor::new(b"%PDF-1.4".to_vec()));
        assert_eq!(w.in_flight, 1);
        assert_eq!(w.model.status, "Printing \"a.pdf\"…");
        w.print_reader("empty.pdf", &mut std::io::Cursor::new(Vec::new()));
        assert_eq!(w.in_flight, 1);
        assert_eq!(w.model.status, "\"empty.pdf\" is empty");
        w.print_path("/no/such/file.pdf");
        assert!(w.model.status.starts_with("Could not open \"file.pdf\""));
        w.model.selected = None;
        w.print_reader("a.pdf", &mut std::io::Cursor::new(b"x".to_vec()));
        assert_eq!(w.in_flight, 1);
        assert!(w.model.status.starts_with("Choose a printer"));
    }

    #[test]
    fn the_badge_changes_only_when_the_count_does_and_alerts_are_titled_by_printer() {
        let mut w = window();
        assert_eq!(w.badge_change(), None);
        w.model.printers[0].queued_jobs = 2;
        assert_eq!(w.badge_change(), Some("2".to_string()));
        assert_eq!(w.badge_change(), None);
        w.model.printers[0].queued_jobs = 0;
        assert_eq!(w.badge_change(), Some(String::new()));
        let mut jam = printer("Lab", false);
        jam.state_reasons = vec!["media-jam-error".into()];
        w.model.apply(Reply::Listed { printers: vec![jam], printer: None, jobs: vec![], offline: vec![] });
        assert_eq!(w.take_alerts(), vec![("Lab needs attention".to_string(), "Paper jam".to_string())]);
        assert!(w.take_alerts().is_empty());
    }

    #[test]
    fn dragging_over_job_rows_chooses_the_range() {
        let mut w = window();
        w.model.jobs = vec![job(1, JobState::Pending), job(2, JobState::Pending), job(3, JobState::Pending)];
        assert!(w.select_job_range(2, 0));
        assert_eq!(w.model.selected_jobs.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(w.select_job_range(1, 1));
        assert_eq!(w.model.selected_jobs.len(), 1);
        w.model.jobs.clear();
        assert!(!w.select_job_range(0, 1));
    }

    #[test]
    fn the_info_fields_load_edit_and_save_the_details() {
        let mut w = window();
        w.model.printers[0].info = "Front desk".into();
        w.model.printers[0].location = "Floor 2".into();
        w.toolbar_action(B_INFO);
        assert_eq!(w.description.to_string(), "Front desk");
        assert_eq!(w.location.to_string(), "Floor 2");
        assert!(!w.text("x"), "no field has the keyboard yet");
        let r = w.field_rect(Field::Location);
        assert_eq!(w.hit_body(r.x + 4.0, r.y + 4.0), Target::Field(Field::Location));
        assert!(w.activate(Target::Field(Field::Location)));
        assert!(w.text(" B"));
        assert_eq!(w.location.to_string(), "Floor 2 B");
        assert!(w.key(8));
        assert_eq!(w.location.to_string(), "Floor 2 ");
        assert!(w.key(9), "tab moves to the other field");
        assert_eq!(w.focus, Some(Field::Description));
        let s = w.save_rect();
        assert_eq!(w.hit_body(s.x + 4.0, s.y + 4.0), Target::Save);
        assert!(w.activate(Target::Save));
        assert_eq!(w.in_flight, 1);
        assert!(w.key(27));
        assert_eq!(w.focus, None);
        assert!(!w.key(8));
    }

    #[test]
    fn a_pool_has_no_fields_and_saves_nothing() {
        let mut w = window();
        w.model.printers[0].is_class = true;
        w.toolbar_action(B_INFO);
        assert!(!w.fields_shown());
        let r = w.field_rect(Field::Description);
        assert_eq!(w.hit_body(r.x + 4.0, r.y + 4.0), Target::Nothing);
        w.toolbar_action(B_SAVE);
        assert_eq!(w.in_flight, 0);
    }

    #[test]
    fn the_fields_follow_the_selected_printer_but_keep_an_edit_across_listings() {
        let mut w = window();
        w.model.printers[1].info = "Lab printer".into();
        w.toolbar_action(B_INFO);
        w.activate(Target::Field(Field::Description));
        w.text("!");
        w.sync_fields();
        assert!(w.description.to_string().ends_with('!'), "a listing does not overwrite the edit");
        w.activate(Target::Printer(1));
        assert_eq!(w.description.to_string(), "Lab printer");
    }

    #[test]
    fn the_columns_stay_inside_the_queue_pane() {
        let w = window();
        let d = w.detail_rect();
        let cols = w.columns();
        assert!(cols.windows(2).all(|p| p[0] <= p[1]));
        assert!(cols[6] < d.x + d.w);
    }
}
