//! The Print Center's model: what the scheduler reported, which printer and job are selected, which toolbar actions
//! apply to them, and the words for their states. Pure: `worker.rs` talks to the scheduler and `window.rs` draws.

use bifrost_app::controls::cups::{Job, JobState, Printer, PrinterState};

/// A change the person asks for. Each one maps to one scheduler operation in `worker.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Pause,
    Resume,
    SetDefault,
    /// Removes the queue. The window asks twice before it sends this.
    Delete,
    CancelJob(i32),
    HoldJob(i32),
    ReleaseJob(i32),
}

impl Action {
    /// The line shown in the footer when the scheduler accepted the action.
    pub fn done(&self) -> &'static str {
        match self {
            Action::Pause => "Printer paused",
            Action::Resume => "Printer resumed",
            Action::SetDefault => "Default printer changed",
            Action::Delete => "Printer deleted",
            Action::CancelJob(_) => "Job canceled",
            Action::HoldJob(_) => "Job held",
            Action::ReleaseJob(_) => "Job released",
        }
    }
}

/// What the worker sends back for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The printers, and the queue of `printer` when the request named one.
    Listed { printers: Vec<Printer>, printer: Option<String>, jobs: Vec<Job> },
    /// An action was accepted.
    Done(&'static str),
    /// A printer was added under this queue name.
    Added(String),
    /// The scheduler could not be reached or refused the request.
    Failed(String),
}

/// The window's data: the printers, the selected printer and its queue, and the selected job.
#[derive(Default, Debug)]
pub struct Model {
    pub printers: Vec<Printer>,
    pub selected: Option<String>,
    pub jobs: Vec<Job>,
    pub selected_job: Option<i32>,
    /// The last result, shown in the footer.
    pub status: String,
    /// The printer whose removal the person has asked for once; a second request removes it.
    pub confirm_delete: Option<String>,
}

impl Model {
    /// Takes a reply. A listing keeps the selection when the printer still exists; its queue is kept only when it is the
    /// queue that is selected now (a selection made while the request was in flight asks again).
    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Listed { printers, printer, jobs } => {
                self.printers = printers;
                self.selected = pick_selection(&self.printers, self.selected.as_deref());
                if printer == self.selected {
                    self.jobs = jobs;
                    if self.selected_job.is_some_and(|id| !self.jobs.iter().any(|j| j.id == id)) {
                        self.selected_job = None;
                    }
                }
            }
            Reply::Done(text) => self.status = text.to_string(),
            Reply::Added(name) => {
                self.status = format!("Added printer \"{name}\"");
                self.select_printer(name);
            }
            Reply::Failed(text) => self.status = text,
        }
    }

    /// Selects a printer. Its queue is empty until the next listing.
    pub fn select_printer(&mut self, name: String) {
        self.selected = Some(name);
        self.jobs.clear();
        self.selected_job = None;
        self.confirm_delete = None;
    }

    pub fn select_job(&mut self, id: i32) {
        self.selected_job = Some(id);
    }

    pub fn selected_printer(&self) -> Option<&Printer> {
        let name = self.selected.as_deref()?;
        self.printers.iter().find(|p| p.name == name)
    }

    pub fn selected_job(&self) -> Option<&Job> {
        let id = self.selected_job?;
        self.jobs.iter().find(|j| j.id == id)
    }
}

/// The printer to show: `current` when it is still listed, else the default, else the first one.
pub fn pick_selection(printers: &[Printer], current: Option<&str>) -> Option<String> {
    if let Some(c) = current {
        if printers.iter().any(|p| p.name == c) {
            return Some(c.to_string());
        }
    }
    printers.iter().find(|p| p.is_default).or(printers.first()).map(|p| p.name.clone())
}

/// The pause or resume action for a printer, with the label of the toolbar button that does it.
pub fn toggle(p: &Printer) -> (Action, &'static str) {
    if p.state == PrinterState::Stopped {
        (Action::Resume, "Resume")
    } else {
        (Action::Pause, "Pause")
    }
}

/// The address the person copied, for Add Printer: trimmed, and only when the scheduler takes it. Otherwise the words
/// the footer shows.
pub fn device_from_clipboard(text: Option<&str>) -> Result<String, &'static str> {
    let device = text.unwrap_or("").trim();
    if bifrost_app::controls::cups::valid_device_uri(device) {
        Ok(device.to_string())
    } else {
        Err("Copy a printer address, such as ipp://printer.local/ipp/print, then choose Add Printer")
    }
}

/// A waiting job can be held.
pub fn can_hold(j: &Job) -> bool {
    j.state == JobState::Pending
}

/// A held job can be released.
pub fn can_release(j: &Job) -> bool {
    j.state == JobState::PendingHeld
}

/// A job that still occupies the queue can be canceled.
pub fn can_cancel(j: &Job) -> bool {
    j.state.is_active()
}

pub fn printer_state_name(state: PrinterState) -> &'static str {
    match state {
        PrinterState::Idle => "Idle",
        PrinterState::Processing => "Printing",
        PrinterState::Stopped => "Paused",
        PrinterState::Other(_) => "Unknown",
    }
}

pub fn job_state_name(state: JobState) -> &'static str {
    match state {
        JobState::Pending => "Waiting",
        JobState::PendingHeld => "Held",
        JobState::Processing => "Printing",
        JobState::ProcessingStopped => "Stopped",
        JobState::Canceled => "Canceled",
        JobState::Aborted => "Failed",
        JobState::Completed => "Completed",
        JobState::Other(_) => "Unknown",
    }
}

/// The second line of a printer row: its state, then whether it is the default, then how many jobs wait.
pub fn printer_summary(p: &Printer) -> String {
    let mut parts = vec![printer_state_name(p.state).to_string()];
    if p.is_default {
        parts.push("Default".into());
    }
    if p.queued_jobs > 0 {
        parts.push(format!("{} in queue", p.queued_jobs));
    }
    parts.join(", ")
}

/// The line under the printer's name in the detail pane: make and model, location and the scheduler's message.
pub fn printer_detail(p: &Printer) -> String {
    [p.make_and_model.as_str(), p.location.as_str(), p.state_message.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// A job's size for its column: kilobytes, or "" when the scheduler did not report one.
pub fn size_text(kb: i32) -> String {
    if kb > 0 {
        format!("{kb} KB")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer(name: &str, is_default: bool, state: PrinterState, queued: i32) -> Printer {
        Printer {
            name: name.into(),
            uri: format!("ipp://localhost/printers/{name}"),
            state,
            state_message: String::new(),
            accepting_jobs: true,
            is_default,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: queued,
        }
    }

    fn job(id: i32, state: JobState) -> Job {
        Job { id, uri: String::new(), state, name: format!("doc{id}"), user: "olibuijr".into(), size_kb: 12 }
    }

    #[test]
    fn selection_keeps_the_current_printer_then_the_default_then_the_first() {
        let list = vec![printer("Lab", false, PrinterState::Idle, 0), printer("Office", true, PrinterState::Idle, 0)];
        assert_eq!(pick_selection(&list, Some("Lab")), Some("Lab".into()));
        assert_eq!(pick_selection(&list, Some("Gone")), Some("Office".into()));
        assert_eq!(pick_selection(&list, None), Some("Office".into()));
        let no_default = vec![printer("Lab", false, PrinterState::Idle, 0)];
        assert_eq!(pick_selection(&no_default, None), Some("Lab".into()));
        assert_eq!(pick_selection(&[], Some("Lab")), None);
    }

    #[test]
    fn the_pause_toggle_follows_the_printer_state() {
        assert_eq!(toggle(&printer("a", false, PrinterState::Stopped, 0)), (Action::Resume, "Resume"));
        assert_eq!(toggle(&printer("a", false, PrinterState::Idle, 0)), (Action::Pause, "Pause"));
        assert_eq!(toggle(&printer("a", false, PrinterState::Processing, 0)), (Action::Pause, "Pause"));
    }

    #[test]
    fn job_actions_follow_the_job_state() {
        assert!(can_hold(&job(1, JobState::Pending)));
        assert!(!can_hold(&job(1, JobState::PendingHeld)));
        assert!(can_release(&job(1, JobState::PendingHeld)));
        assert!(!can_release(&job(1, JobState::Pending)));
        assert!(can_cancel(&job(1, JobState::Processing)));
        assert!(!can_cancel(&job(1, JobState::Completed)));
    }

    #[test]
    fn states_have_their_words() {
        assert_eq!(job_state_name(JobState::PendingHeld), "Held");
        assert_eq!(job_state_name(JobState::Other(42)), "Unknown");
        assert_eq!(printer_state_name(PrinterState::Stopped), "Paused");
        assert_eq!(printer_summary(&printer("a", true, PrinterState::Idle, 2)), "Idle, Default, 2 in queue");
        assert_eq!(size_text(0), "");
        assert_eq!(size_text(120), "120 KB");
    }

    #[test]
    fn a_listing_keeps_the_selected_printer_and_its_queue() {
        let mut m = Model::default();
        m.apply(Reply::Listed {
            printers: vec![printer("Office", true, PrinterState::Idle, 1), printer("Lab", false, PrinterState::Idle, 0)],
            printer: None,
            jobs: vec![],
        });
        assert_eq!(m.selected.as_deref(), Some("Office"));
        m.apply(Reply::Listed {
            printers: vec![printer("Office", true, PrinterState::Idle, 2)],
            printer: Some("Office".into()),
            jobs: vec![job(4, JobState::Pending), job(5, JobState::PendingHeld)],
        });
        assert_eq!(m.jobs.len(), 2);
        m.select_job(5);
        assert_eq!(m.selected_job().map(|j| j.name.as_str()), Some("doc5"));
        // the job is released from the queue: the next listing clears the selection
        m.apply(Reply::Listed {
            printers: vec![printer("Office", true, PrinterState::Idle, 1)],
            printer: Some("Office".into()),
            jobs: vec![job(4, JobState::Pending)],
        });
        assert_eq!(m.selected_job, None);
    }

    #[test]
    fn a_queue_listed_for_another_printer_is_ignored() {
        let mut m = Model::default();
        m.apply(Reply::Listed { printers: vec![printer("Office", true, PrinterState::Idle, 0), printer("Lab", false, PrinterState::Idle, 0)], printer: None, jobs: vec![] });
        m.select_printer("Lab".into());
        m.apply(Reply::Listed { printers: vec![printer("Office", true, PrinterState::Idle, 0), printer("Lab", false, PrinterState::Idle, 0)], printer: Some("Office".into()), jobs: vec![job(1, JobState::Pending)] });
        assert!(m.jobs.is_empty());
        assert_eq!(m.selected.as_deref(), Some("Lab"));
    }

    #[test]
    fn an_added_printer_is_named_in_the_footer_and_selected() {
        let mut m = Model::default();
        m.apply(Reply::Added("HP".into()));
        assert_eq!(m.status, "Added printer \"HP\"");
        assert_eq!(m.selected.as_deref(), Some("HP"));
        assert_eq!(Action::Delete.done(), "Printer deleted");
    }

    #[test]
    fn the_address_for_add_printer_is_trimmed_and_must_be_one_the_scheduler_takes() {
        assert_eq!(device_from_clipboard(Some(" ipp://printer.local/ipp/print \n")), Ok("ipp://printer.local/ipp/print".to_string()));
        assert!(device_from_clipboard(Some("hello")).is_err());
        assert!(device_from_clipboard(None).is_err());
    }

    #[test]
    fn done_and_failed_replies_set_the_footer() {
        let mut m = Model::default();
        m.apply(Reply::Done(Action::HoldJob(3).done()));
        assert_eq!(m.status, "Job held");
        m.apply(Reply::Failed("printing service unreachable".into()));
        assert_eq!(m.status, "printing service unreachable");
    }
}
