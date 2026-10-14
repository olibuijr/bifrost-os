//! Automator's window: the workflow being edited, the selected step and its focused field, the run and its log, and the
//! geometry that the drawing and the taps share. Three columns under the toolbar: the library of actions, the sequence
//! of steps, and the editor for the selected step with the log under it. `draw.rs` paints this; `main.rs` performs the
//! requests that need the shell (the open and save panels).

use crate::model::{self, Kind, LogLine, Step, Value, Workflow, KINDS, MAX_FIELDS};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{widgets, TouchKind};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The library column (points).
pub const LIB_PT: f32 = 220.0;
/// The sequence column (points).
pub const SEQ_PT: f32 = 300.0;
/// The footer under the columns (points).
pub const FOOT_PT: f32 = 30.0;
/// A column's header row (points).
pub const HEAD_PT: f32 = 36.0;
/// One library or sequence row (points).
pub const ROW_PT: f32 = 44.0;
/// One field row in the editor (points).
pub const FIELD_PT: f32 = 34.0;
/// Lines of the editor's hint under the fields.
pub const HINT_LINES: usize = 3;
/// One hint line (points).
pub const HINT_LINE_PT: f32 = 15.0;
/// One log line (points).
pub const LOG_LINE_PT: f32 = 16.0;
/// Padding between the blocks of the editor (points).
pub const PAD_PT: f32 = 8.0;
/// Fallback only if the worker wake socket cannot be created (milliseconds).
const RUN_WAKE_FALLBACK_MS: f64 = 50.0;

pub const B_NEW: u32 = 1;
pub const B_OPEN: u32 = 2;
pub const B_SAVE: u32 = 3;
pub const B_UP: u32 = 4;
pub const B_DOWN: u32 = 5;
pub const B_REMOVE: u32 = 6;
pub const B_RUN: u32 = 7;

/// Accessibility ids: toolbar buttons are 1 to 7, library rows 100 and up, sequence rows 200 and up.
pub const LIBRARY_ID: u64 = 100;
pub const STEP_ID: u64 = 200;

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The action at this index of the library.
    Library(usize),
    /// The step at this index of the sequence.
    Step(usize),
    /// The field at this index of the selected step.
    Field(usize),
}

/// A request that needs the shell, for the app to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    None,
    Open,
    Save,
}

/// What the run thread reports back.
pub enum Message {
    Log(LogLine),
    Done(Result<Value, String>),
}

pub struct Automator {
    pub workflow: Workflow,
    /// The selected step of the sequence.
    pub selected: Option<usize>,
    /// The focused field of the selected step; typing goes to it when it is a free text field.
    pub focus: Option<usize>,
    /// The log of the last run.
    pub log: Vec<LogLine>,
    /// The messages of the run in progress, while one runs.
    running: Option<Receiver<Message>>,
    /// Worker progress signals wake the shell through this nonblocking socket pair.
    wake_rx: Option<UnixStream>,
    wake_tx: Option<UnixStream>,
    /// The last result as the footer says it: an error, or how the run finished.
    pub status: String,
    /// The window size, in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target a press started on; the release acts only when it lands on the same one.
    pub press: Target,
    /// The folder Find starts in by default.
    pub home: String,
}

impl Default for Automator {
    fn default() -> Self {
        Automator::new(home_folder())
    }
}

/// The folder a new Find step starts in.
fn home_folder() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

impl Automator {
    pub fn new(home: String) -> Automator {
        let (wake_rx, wake_tx) = UnixStream::pair()
            .ok()
            .filter(|(rx, tx)| rx.set_nonblocking(true).is_ok() && tx.set_nonblocking(true).is_ok())
            .map_or((None, None), |(rx, tx)| (Some(rx), Some(tx)));
        Automator {
            workflow: Workflow::default(),
            selected: None,
            focus: None,
            log: Vec::new(),
            running: None,
            wake_rx,
            wake_tx,
            status: String::new(),
            // The app host configures the actual client size before the first frame.
            size: (0.0, 0.0),
            labels: TextCache::default(),
            press: Target::Nothing,
            home,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = (w, h);
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    // ----- geometry, in points -----

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    pub fn footer_rect(&self) -> Rect {
        Rect { x: 0.0, y: (self.size.1 - FOOT_PT).max(0.0), w: self.size.0, h: FOOT_PT }
    }

    /// The height between the toolbar and the footer.
    fn body_h(&self) -> f32 {
        (self.size.1 - UNIFIED_BAR_PT - FOOT_PT).max(0.0)
    }

    pub fn library_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: LIB_PT.min(self.size.0), h: self.body_h() }
    }

    pub fn library_row(&self, i: usize) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT + HEAD_PT + i as f32 * ROW_PT, w: LIB_PT.min(self.size.0), h: ROW_PT }
    }

    pub fn sequence_rect(&self) -> Rect {
        let x = LIB_PT.min(self.size.0);
        Rect { x, y: UNIFIED_BAR_PT, w: SEQ_PT.min((self.size.0 - x).max(0.0)), h: self.body_h() }
    }

    /// How many step rows fit under the sequence header.
    pub fn visible_steps(&self) -> usize {
        ((self.sequence_rect().h - HEAD_PT).max(0.0) / ROW_PT).floor() as usize
    }

    pub fn step_row(&self, i: usize) -> Rect {
        let s = self.sequence_rect();
        Rect { x: s.x, y: UNIFIED_BAR_PT + HEAD_PT + i as f32 * ROW_PT, w: s.w, h: ROW_PT }
    }

    pub fn detail_rect(&self) -> Rect {
        let x = (LIB_PT + SEQ_PT).min(self.size.0);
        Rect { x, y: UNIFIED_BAR_PT, w: (self.size.0 - x).max(0.0), h: self.body_h() }
    }

    pub fn field_row(&self, k: usize) -> Rect {
        let d = self.detail_rect();
        Rect { x: d.x, y: UNIFIED_BAR_PT + HEAD_PT + k as f32 * FIELD_PT, w: d.w, h: FIELD_PT }
    }

    /// The top of the hint under the editor's fields.
    pub fn hint_top(&self) -> f32 {
        UNIFIED_BAR_PT + HEAD_PT + MAX_FIELDS as f32 * FIELD_PT + PAD_PT
    }

    /// The top of the log block under the hint.
    pub fn log_top(&self) -> f32 {
        self.hint_top() + HINT_LINES as f32 * HINT_LINE_PT + 2.0 * PAD_PT
    }

    pub fn log_rect(&self) -> Rect {
        let d = self.detail_rect();
        let top = self.log_top();
        Rect { x: d.x, y: top, w: d.w, h: (UNIFIED_BAR_PT + self.body_h() - top).max(0.0) }
    }

    /// How many log lines fit under the log header.
    pub fn visible_log_lines(&self) -> usize {
        ((self.log_rect().h - HEAD_PT).max(0.0) / LOG_LINE_PT).floor() as usize
    }

    /// The toolbar. Edits are offered only while nothing runs; Run needs at least one step.
    pub fn toolbar(&self) -> Toolbar {
        let idle = !self.is_running();
        let steps = self.workflow.steps.len();
        let up = self.selected.is_some_and(|i| i > 0);
        let down = self.selected.is_some_and(|i| i + 1 < steps);
        Toolbar::new(vec![
            ToolbarItem::button(B_NEW, "New").enabled(idle),
            ToolbarItem::button(B_OPEN, "Open").enabled(idle),
            ToolbarItem::button(B_SAVE, "Save").enabled(idle),
            ToolbarItem::button(B_UP, "Move Up").enabled(idle && up),
            ToolbarItem::button(B_DOWN, "Move Down").enabled(idle && down),
            ToolbarItem::button(B_REMOVE, "Remove").enabled(idle && self.selected.is_some()),
            ToolbarItem::button(B_RUN, "Run").enabled(idle && steps > 0),
        ])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The footer's words: the last status, or a hint for what to do next.
    pub fn footer_text(&self) -> String {
        if self.is_running() {
            return "Running…".into();
        }
        if !self.status.is_empty() {
            return self.status.clone();
        }
        if self.workflow.steps.is_empty() {
            return "Add actions from the library to build a workflow".into();
        }
        format!("{} · {} actions", self.workflow.name, self.workflow.steps.len())
    }

    // ----- hit testing -----

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return self.hit_toolbar(ui, x, y);
        }
        self.hit_body(x, y)
    }

    /// What is under `(x, y)` below the toolbar.
    pub fn hit_body(&self, x: f32, y: f32) -> Target {
        let top = UNIFIED_BAR_PT + HEAD_PT;
        if y < top || y >= UNIFIED_BAR_PT + self.body_h() {
            return Target::Nothing;
        }
        let row = ((y - top) / ROW_PT) as usize;
        if x < LIB_PT {
            return if row < KINDS.len() { Target::Library(row) } else { Target::Nothing };
        }
        if x < LIB_PT + SEQ_PT {
            return if row < self.workflow.steps.len() && row < self.visible_steps() {
                Target::Step(row)
            } else {
                Target::Nothing
            };
        }
        let field = ((y - top) / FIELD_PT) as usize;
        match self.selected_step() {
            Some(step) if field < step.kind.fields().len() && field < MAX_FIELDS => Target::Field(field),
            _ => Target::Nothing,
        }
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

    // ----- input -----

    /// A touch in points. A tap is a press and a release on the same target.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32) -> Request {
        match kind {
            TouchKind::Down => {
                self.press = self.hit(ui, x, y);
                Request::None
            }
            TouchKind::Move => Request::None,
            TouchKind::Up => {
                let released = self.hit(ui, x, y);
                let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                if released != pressed {
                    return Request::None;
                }
                self.activate(released)
            }
            TouchKind::Cancel => {
                self.press = Target::Nothing;
                Request::None
            }
        }
    }

    /// An accessibility press on a published control.
    pub fn activate_id(&mut self, id: u64) -> Request {
        let target = match id {
            1..=7 => Target::Toolbar(id as u32),
            _ if (LIBRARY_ID..LIBRARY_ID + KINDS.len() as u64).contains(&id) => Target::Library((id - LIBRARY_ID) as usize),
            _ if (STEP_ID..STEP_ID + 100).contains(&id) => Target::Step((id - STEP_ID) as usize),
            _ => Target::Nothing,
        };
        self.activate(target)
    }

    /// Performs what a tap on `target` means. Requests that need the shell come back to the app.
    pub fn activate(&mut self, target: Target) -> Request {
        match target {
            Target::Nothing => Request::None,
            Target::Toolbar(id) => match id {
                B_NEW => {
                    self.new_workflow();
                    Request::None
                }
                B_OPEN => Request::Open,
                B_SAVE => Request::Save,
                B_UP => {
                    self.move_selected(-1);
                    Request::None
                }
                B_DOWN => {
                    self.move_selected(1);
                    Request::None
                }
                B_REMOVE => {
                    self.remove_selected();
                    Request::None
                }
                B_RUN => {
                    self.start_run();
                    Request::None
                }
                _ => Request::None,
            },
            Target::Library(i) => {
                if let Some(kind) = KINDS.get(i).copied() {
                    self.add_step(kind);
                }
                Request::None
            }
            Target::Step(i) => {
                self.select(i);
                Request::None
            }
            Target::Field(k) => {
                self.field_tapped(k);
                Request::None
            }
        }
    }

    /// A tap on a field: a choice field moves to its next choice, a text field takes the focus.
    fn field_tapped(&mut self, k: usize) {
        let Some(i) = self.selected else { return };
        let Some(step) = self.workflow.steps.get_mut(i) else { return };
        if step.kind.fields().get(k).is_some_and(|f| !f.choices.is_empty()) {
            step.cycle(k);
            self.focus = None;
        } else {
            self.focus = Some(k);
        }
    }

    /// Typed text goes to the focused text field.
    pub fn type_text(&mut self, text: &str) {
        if let Some(value) = self.focused_text() {
            value.extend(text.chars().filter(|c| !c.is_control()));
        }
    }

    /// Backspace in the focused text field.
    pub fn backspace(&mut self) {
        if let Some(value) = self.focused_text() {
            value.pop();
        }
    }

    /// Tab and Enter move the focus to the next field of the selected step (and wrap).
    pub fn next_field(&mut self) {
        let Some(step) = self.selected_step() else { return };
        let count = step.kind.fields().len();
        if count == 0 {
            return;
        }
        self.focus = Some(self.focus.map_or(0, |k| (k + 1) % count));
    }

    /// Escape gives up the focus.
    pub fn unfocus(&mut self) {
        self.focus = None;
    }

    /// The value of the focused field, when the focus is a free text field.
    fn focused_text(&mut self) -> Option<&mut String> {
        let k = self.focus?;
        let i = self.selected?;
        let step = self.workflow.steps.get_mut(i)?;
        if !step.kind.fields().get(k)?.choices.is_empty() {
            return None;
        }
        step.values.get_mut(k)
    }

    // ----- the workflow -----

    pub fn selected_step(&self) -> Option<&Step> {
        self.selected.and_then(|i| self.workflow.steps.get(i))
    }

    /// Appends a step of `kind` with its default values, and selects it.
    pub fn add_step(&mut self, kind: Kind) {
        self.workflow.steps.push(Step::new(kind, &self.home));
        self.selected = Some(self.workflow.steps.len() - 1);
        self.focus = None;
    }

    pub fn select(&mut self, i: usize) {
        if i < self.workflow.steps.len() {
            self.selected = Some(i);
            self.focus = None;
        }
    }

    /// Removes the selected step; the selection moves to the step that takes its place.
    pub fn remove_selected(&mut self) {
        let Some(i) = self.selected else { return };
        if i >= self.workflow.steps.len() {
            return;
        }
        self.workflow.steps.remove(i);
        self.focus = None;
        self.selected = if self.workflow.steps.is_empty() { None } else { Some(i.min(self.workflow.steps.len() - 1)) };
    }

    /// Moves the selected step up (`-1`) or down (`1`) one place; the selection follows it.
    pub fn move_selected(&mut self, delta: isize) {
        let Some(i) = self.selected else { return };
        let target = i as isize + delta;
        if target < 0 || target as usize >= self.workflow.steps.len() {
            return;
        }
        self.workflow.steps.swap(i, target as usize);
        self.selected = Some(target as usize);
    }

    /// File > New: an empty workflow with the default name. Nothing runs while the run is in progress.
    pub fn new_workflow(&mut self) {
        if self.is_running() {
            return;
        }
        self.workflow = Workflow::default();
        self.selected = None;
        self.focus = None;
        self.log.clear();
        self.status = "New workflow".into();
    }

    /// Replaces the workflow with one read from a file.
    pub fn open_text(&mut self, text: &str) -> Result<(), String> {
        if self.is_running() {
            return Err("wait for the run to finish".into());
        }
        let workflow = Workflow::from_file(text)?;
        self.status = format!("Opened {}", workflow.name);
        self.selected = if workflow.steps.is_empty() { None } else { Some(0) };
        self.focus = None;
        self.log.clear();
        self.workflow = workflow;
        Ok(())
    }

    /// The workflow as the bytes of a file, for Save.
    pub fn save_bytes(&self) -> Vec<u8> {
        self.workflow.to_file().into_bytes()
    }

    /// The file name Save suggests: the workflow's name, without characters a file name cannot take.
    pub fn suggested_file_name(&self) -> String {
        let clean: String = self
            .workflow
            .name
            .chars()
            .map(|c| if c == '/' || c.is_control() { ' ' } else { c })
            .collect();
        let clean = clean.trim();
        format!("{}.automator", if clean.is_empty() { model::UNTITLED } else { clean })
    }

    /// The workflow takes the name of the file it was saved to.
    pub fn named_after(&mut self, file_name: &str) {
        let stem = file_name.strip_suffix(".automator").unwrap_or(file_name).trim();
        if !stem.is_empty() {
            self.workflow.name = stem.to_string();
        }
        self.status = format!("Saved {}", self.workflow.name);
    }

    // ----- running -----

    /// Starts the workflow on a thread. Its log lines and result come back through `poll`.
    pub fn start_run(&mut self) {
        if self.is_running() || self.workflow.steps.is_empty() {
            return;
        }
        self.log.clear();
        self.status.clear();
        let steps = self.workflow.steps.clone();
        let (tx, rx) = mpsc::channel();
        let wake_tx = self.wake_tx.as_ref().and_then(|wake| wake.try_clone().ok());
        DispatchQueue::global(Qos::UserInitiated).async_(move || {
            let result = model::execute(&steps, &mut |line: LogLine| {
                let _ = tx.send(Message::Log(line));
                signal_worker(&wake_tx);
            });
            let _ = tx.send(Message::Done(result));
            signal_worker(&wake_tx);
        });
        self.running = Some(rx);
    }

    /// Takes what the run has reported since the last look. True when the screen needs a frame.
    pub fn poll(&mut self) -> bool {
        if let Some(wake) = &mut self.wake_rx {
            let mut bytes = [0u8; 64];
            while wake.read(&mut bytes).is_ok_and(|n| n > 0) {}
        }
        let Some(rx) = &self.running else { return false };
        let mut changed = false;
        let mut finished = None;
        loop {
            match rx.try_recv() {
                Ok(Message::Log(line)) => {
                    self.log.push(line);
                    changed = true;
                }
                Ok(Message::Done(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err("the run stopped unexpectedly".into()));
                    break;
                }
            }
        }
        if let Some(result) = finished {
            self.running = None;
            self.status = match result {
                Ok(_) => "Finished".into(),
                Err(error) => error,
            };
            changed = true;
        }
        changed
    }

    /// The shell polls this descriptor for worker progress and completion.
    pub fn wake_fd(&self) -> Option<i32> {
        self.wake_rx.as_ref().map(AsRawFd::as_raw_fd)
    }

    /// When the loop should come back: only use a timer if wake-fd setup failed.
    pub fn wake_at(&self, t: f64) -> Option<f64> {
        (self.is_running() && self.wake_rx.is_none()).then_some(t + RUN_WAKE_FALLBACK_MS)
    }
}

fn signal_worker(wake: &Option<UnixStream>) {
    if let Some(wake) = wake {
        let _ = (&*wake).write(&[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Automator {
        let mut app = Automator::new("/home/u".into());
        app.resize(1000.0, 640.0);
        app
    }

    /// Points on the middle of the sequence row `i`.
    fn on_step(a: &Automator, i: usize) -> (f32, f32) {
        let r = a.step_row(i);
        (r.x + 10.0, r.y + 10.0)
    }

    #[test]
    fn library_rows_hit_their_action() {
        let a = window();
        let r = a.library_row(2);
        assert_eq!(a.hit_body(r.x + 5.0, r.y + 5.0), Target::Library(2));
        // the header is not a row
        assert_eq!(a.hit_body(5.0, UNIFIED_BAR_PT + 5.0), Target::Nothing);
    }

    #[test]
    fn library_rows_fit_in_the_window() {
        let a = window();
        let last = a.library_row(KINDS.len() - 1);
        assert!(last.y + last.h <= UNIFIED_BAR_PT + a.body_h());
    }

    #[test]
    fn adding_steps_selects_the_new_one() {
        let mut a = window();
        a.add_step(Kind::Find);
        a.add_step(Kind::Transform);
        assert_eq!(a.workflow.steps.len(), 2);
        assert_eq!(a.selected, Some(1));
        assert_eq!(a.workflow.steps[0].values[0], "/home/u");
    }

    #[test]
    fn steps_hit_in_the_sequence_column() {
        let mut a = window();
        a.add_step(Kind::Find);
        a.add_step(Kind::Combine);
        let (x, y) = on_step(&a, 1);
        assert_eq!(a.hit_body(x, y), Target::Step(1));
        // past the last step is nothing
        let r = a.step_row(2);
        assert_eq!(a.hit_body(r.x + 5.0, r.y + 5.0), Target::Nothing);
    }

    #[test]
    fn remove_and_move_keep_the_selection_sensible() {
        let mut a = window();
        a.add_step(Kind::Find);
        a.add_step(Kind::Rename);
        a.add_step(Kind::Combine);
        a.select(0);
        a.move_selected(1);
        assert_eq!(a.workflow.steps[1].kind, Kind::Find);
        assert_eq!(a.selected, Some(1));
        a.move_selected(-1);
        assert_eq!(a.selected, Some(0));
        a.move_selected(-1);
        assert_eq!(a.selected, Some(0));
        a.select(2);
        a.remove_selected();
        assert_eq!(a.selected, Some(1));
        a.remove_selected();
        a.remove_selected();
        assert!(a.workflow.steps.is_empty());
        assert_eq!(a.selected, None);
    }

    #[test]
    fn typing_goes_to_the_focused_text_field_only() {
        let mut a = window();
        a.add_step(Kind::Copy);
        // no focus: typing does nothing
        a.type_text("x");
        assert_eq!(a.workflow.steps[0].values[0], "");
        a.focus = Some(0);
        a.type_text("/data/out\u{7}");
        assert_eq!(a.workflow.steps[0].values[0], "/data/out");
        a.backspace();
        assert_eq!(a.workflow.steps[0].values[0], "/data/ou");
    }

    #[test]
    fn choice_fields_cycle_on_tap_and_ignore_typing() {
        let mut a = window();
        a.add_step(Kind::Convert);
        assert_eq!(a.workflow.steps[0].values[0], "png");
        a.activate(Target::Field(0));
        assert_eq!(a.workflow.steps[0].values[0], "bmp");
        assert_eq!(a.focus, None);
        a.focus = Some(0);
        a.type_text("zzz");
        assert_eq!(a.workflow.steps[0].values[0], "bmp");
    }

    #[test]
    fn tab_moves_focus_through_the_fields_and_wraps() {
        let mut a = window();
        a.add_step(Kind::Transform);
        a.next_field();
        assert_eq!(a.focus, Some(0));
        a.next_field();
        a.next_field();
        a.next_field();
        assert_eq!(a.focus, Some(0));
        a.unfocus();
        assert_eq!(a.focus, None);
    }

    #[test]
    fn field_taps_need_a_selected_step() {
        let mut a = window();
        assert_eq!(a.hit_body(600.0, UNIFIED_BAR_PT + HEAD_PT + 5.0), Target::Nothing);
        a.add_step(Kind::Find);
        assert_eq!(a.hit_body(600.0, UNIFIED_BAR_PT + HEAD_PT + 5.0), Target::Field(0));
        // Find has three fields; a fourth row is nothing
        assert_eq!(a.hit_body(600.0, UNIFIED_BAR_PT + HEAD_PT + 3.0 * FIELD_PT + 5.0), Target::Nothing);
    }

    #[test]
    fn run_messages_are_collected_and_finish_the_run() {
        let mut a = window();
        a.add_step(Kind::Transform);
        let (tx, rx) = mpsc::channel();
        a.running = Some(rx);
        tx.send(Message::Log(LogLine::info("Step 1 of 1"))).unwrap();
        tx.send(Message::Done(Err("stopped at step 1".into()))).unwrap();
        assert!(a.poll());
        assert_eq!(a.log.len(), 1);
        assert!(!a.is_running());
        assert_eq!(a.status, "stopped at step 1");
        assert!(!a.poll());
    }

    #[test]
    fn a_run_cannot_start_without_steps_or_twice() {
        let mut a = window();
        a.start_run();
        assert!(!a.is_running());
        let (_tx, rx) = mpsc::channel::<Message>();
        a.running = Some(rx);
        a.add_step(Kind::Transform);
        a.start_run();
        assert!(a.is_running());
        assert_eq!(a.wake_at(100.0), None);
    }

    #[test]
    fn opening_replaces_the_workflow_and_save_names_the_file() {
        let mut a = window();
        a.add_step(Kind::Find);
        let text = Workflow { name: "Shots".into(), steps: vec![Step::new(Kind::Resize, "/")] }.to_file();
        a.open_text(&text).unwrap();
        assert_eq!(a.workflow.name, "Shots");
        assert_eq!(a.selected, Some(0));
        assert_eq!(a.suggested_file_name(), "Shots.automator");
        a.named_after("Thumbs.automator");
        assert_eq!(a.workflow.name, "Thumbs");
        assert!(a.open_text("garbage").is_err());
    }

    #[test]
    fn accessibility_ids_reach_the_same_targets() {
        let mut a = window();
        assert_eq!(a.activate_id(LIBRARY_ID + 1), Request::None);
        assert_eq!(a.workflow.steps.len(), 1);
        assert_eq!(a.workflow.steps[0].kind, KINDS[1]);
        assert_eq!(a.activate_id(B_SAVE as u64), Request::Save);
        assert_eq!(a.activate_id(B_OPEN as u64), Request::Open);
        assert_eq!(a.activate_id(999), Request::None);
    }
}
