//! The Print Center's model: what the scheduler reported, which printer and jobs are selected, which toolbar actions
//! apply to them, and the words for their states. Pure: `worker.rs` talks to the scheduler and `window.rs` draws.

use bifrost_app::controls::cups::{Job, JobState, Printer, PrinterState, Supply};
use std::collections::BTreeSet;

/// A change the person asks for. Each one maps to one scheduler operation in `worker.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Pause,
    Resume,
    SetDefault,
    /// Removes the queue. The window asks twice before it sends this.
    Delete,
    /// Removes a printer pool; the printers in it stay.
    DeletePool,
    /// Offers the queue to other machines (`true`) or stops offering it (`false`).
    SetShared(bool),
    /// Changes the description and the location people see for the queue.
    SetDetails { info: String, location: String },
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
            Action::DeletePool => "Printer pool removed",
            Action::SetShared(true) => "Printer shared",
            Action::SetShared(false) => "Sharing stopped",
            Action::SetDetails { .. } => "Printer details saved",
            Action::CancelJob(_) => "Job canceled",
            Action::HoldJob(_) => "Job held",
            Action::ReleaseJob(_) => "Job released",
        }
    }
}

/// What the worker sends back for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The printers, the queue of `printer` when the request named one, and the names of the printers that did not
    /// answer a connection to their device address.
    Listed { printers: Vec<Printer>, printer: Option<String>, jobs: Vec<Job>, offline: Vec<String> },
    /// The options of `printer`, or `None` when the device did not report them.
    Options { printer: String, options: Option<Options> },
    /// An action was accepted.
    Done(&'static str),
    /// A printer was added under this queue name.
    Added(String),
    /// A document was sent to the printer; this is its name.
    Printed(String),
    /// The scheduler could not be reached or refused the request.
    Failed(String),
}

/// What the detail area (right of the printer list) shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The queue of the selected printer.
    Queue,
    /// The selected printer's details, supplies and sharing.
    Info,
    /// The printers found on the network, and the clipboard fallback.
    Discover,
    /// The help topics.
    Help,
}

/// The window's data: the printers, the selected printer and its queue, and the selected jobs.
#[derive(Default, Debug)]
pub struct Model {
    pub printers: Vec<Printer>,
    pub selected: Option<String>,
    pub jobs: Vec<Job>,
    /// The jobs chosen in the queue; the job actions apply to all of them.
    pub selected_jobs: BTreeSet<i32>,
    /// The last result, shown in the footer.
    pub status: String,
    /// The printer whose removal the person has asked for once; a second request removes it.
    pub confirm_delete: Option<String>,
    /// Printers whose device did not answer the last probe.
    pub offline: BTreeSet<String>,
    /// The info pane replaces the queue.
    pub show_info: bool,
    /// The options of the selected printer, once the device has reported them.
    pub options: Option<(String, Options)>,
    /// Jobs whose cancel was asked for, as (printer, job id), until the scheduler drops them from the queue.
    pub canceling: BTreeSet<(String, i32)>,
    /// The Add Printer pane (printers found on the network) replaces the queue.
    pub show_discover: bool,
    /// The Help pane replaces the queue.
    pub show_help: bool,
    /// A listing has arrived, so what the window knows is the scheduler's answer and not the empty start.
    pub listed: bool,
    /// `printer|problem` pairs already announced, so a problem is announced once until it clears.
    alerted: BTreeSet<String>,
    /// Announcements not yet posted: (printer, problem).
    alerts: Vec<(String, String)>,
}

impl Model {
    /// Takes a reply. A listing keeps the selection when the printer still exists; its queue is kept only when it is the
    /// queue that is selected now (a selection made while the request was in flight asks again).
    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Listed { printers, printer, jobs, offline } => {
                self.listed = true;
                self.printers = printers;
                self.offline = offline.into_iter().collect();
                self.collect_alerts();
                self.selected = pick_selection(&self.printers, self.selected.as_deref());
                if printer == self.selected {
                    self.jobs = jobs;
                    let ids: BTreeSet<i32> = self.jobs.iter().map(|j| j.id).collect();
                    self.selected_jobs.retain(|id| ids.contains(id));
                    let shown = self.selected.clone().unwrap_or_default();
                    self.canceling.retain(|(name, id)| *name != shown || ids.contains(id));
                }
            }
            // an answer for a printer that is no longer selected is out of date and is dropped
            Reply::Options { printer, options } => {
                if self.selected.as_deref() == Some(printer.as_str()) {
                    self.options = options.map(|o| (printer, o));
                }
            }
            Reply::Done(text) => self.status = text.to_string(),
            Reply::Added(name) => {
                self.status = format!("Added printer \"{name}\"");
                self.select_printer(name);
            }
            Reply::Printed(name) => self.status = format!("Sent \"{name}\" to the printer"),
            Reply::Failed(text) => self.status = text,
        }
    }

    /// Notes the error-level problems that were not announced yet, and forgets the ones that cleared.
    fn collect_alerts(&mut self) {
        let mut live = BTreeSet::new();
        for p in &self.printers {
            for problem in printer_problems(p) {
                if problem.level != Level::Error {
                    continue;
                }
                let key = format!("{}|{}", p.name, problem.text);
                if !self.alerted.contains(&key) {
                    self.alerts.push((p.name.clone(), problem.text.clone()));
                }
                live.insert(key);
            }
        }
        self.alerted = live;
    }

    /// The announcements to post now: a printer name and what is wrong. Each is returned once.
    pub fn take_alerts(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.alerts)
    }

    /// The pane the detail area shows now.
    pub fn pane(&self) -> Pane {
        if self.show_discover {
            Pane::Discover
        } else if self.show_help {
            Pane::Help
        } else if self.show_info {
            Pane::Info
        } else {
            Pane::Queue
        }
    }

    /// Shows `pane`, or the queue when it is the one showing. Only one pane is shown at a time.
    pub fn toggle_pane(&mut self, pane: Pane) {
        let next = if self.pane() == pane { Pane::Queue } else { pane };
        self.show_info = next == Pane::Info;
        self.show_discover = next == Pane::Discover;
        self.show_help = next == Pane::Help;
    }

    /// Selects a printer. Its queue is empty until the next listing.
    pub fn select_printer(&mut self, name: String) {
        self.selected = Some(name);
        self.jobs.clear();
        self.selected_jobs.clear();
        self.confirm_delete = None;
        self.options = None;
    }

    /// Marks a job of `printer` as being canceled, so its status reads Canceling until the scheduler drops it.
    pub fn mark_canceling(&mut self, printer: &str, id: i32) {
        self.canceling.insert((printer.to_string(), id));
    }

    /// The status words of a job in the selected printer's queue, with a cancel on its way.
    pub fn job_status(&self, job: &Job) -> String {
        let canceling = self.selected.as_ref().is_some_and(|name| self.canceling.contains(&(name.clone(), job.id)));
        job_status_text(job, canceling)
    }

    /// The options of `printer`, when the device reported them and it is the selected printer.
    pub fn options_for(&self, printer: &str) -> Option<&Options> {
        self.options.as_ref().filter(|(name, _)| name == printer).map(|(_, o)| o)
    }

    /// Chooses this job alone.
    pub fn select_job(&mut self, id: i32) {
        self.selected_jobs.clear();
        self.selected_jobs.insert(id);
    }

    /// Adds the job to the choice, or takes it out when it is in.
    pub fn toggle_job(&mut self, id: i32) {
        if !self.selected_jobs.remove(&id) {
            self.selected_jobs.insert(id);
        }
    }

    /// Chooses every job of the queue.
    pub fn select_all_jobs(&mut self) {
        self.selected_jobs = self.jobs.iter().map(|j| j.id).collect();
    }

    pub fn selected_printer(&self) -> Option<&Printer> {
        let name = self.selected.as_deref()?;
        self.printers.iter().find(|p| p.name == name)
    }

    /// The chosen jobs, in queue order.
    pub fn chosen_jobs(&self) -> Vec<&Job> {
        self.jobs.iter().filter(|j| self.selected_jobs.contains(&j.id)).collect()
    }

    /// The ids of the chosen jobs the rule `allows`.
    pub fn chosen_ids(&self, allows: fn(&Job) -> bool) -> Vec<i32> {
        self.chosen_jobs().into_iter().filter(|j| allows(j)).map(|j| j.id).collect()
    }

    pub fn is_offline(&self, name: &str) -> bool {
        self.offline.contains(name)
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

/// How serious a printer problem is, from `printer-state-reasons`: a `-report` keyword informs, a `-warning` asks for
/// attention soon, an `-error` stops the printer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Report,
    Warning,
    Error,
}

/// One problem in plain words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub text: String,
}

/// The sentence for one `printer-state-reasons` keyword, or `None` for a keyword that carries nothing a person can act on.
pub fn reason_problem(keyword: &str) -> Option<Problem> {
    let (base, level) = if let Some(b) = keyword.strip_suffix("-error") {
        (b, Level::Error)
    } else if let Some(b) = keyword.strip_suffix("-warning") {
        (b, Level::Warning)
    } else if let Some(b) = keyword.strip_suffix("-report") {
        (b, Level::Report)
    } else {
        (keyword, Level::Report)
    };
    let text = match base {
        "media-jam" => "Paper jam",
        "media-empty" => "Out of paper",
        "media-needed" => "Paper needed",
        "media-low" => "Paper is running low",
        "toner-low" => "Toner is running low",
        "toner-empty" => "Out of toner",
        "marker-supply-low" => "Ink or toner is running low",
        "marker-supply-empty" => "Out of ink or toner",
        "marker-waste-almost-full" => "Waste container is almost full",
        "marker-waste-full" => "Waste container is full",
        "door-open" | "cover-open" => "A door or cover is open",
        "input-tray-missing" => "Paper tray is missing",
        "output-tray-missing" => "Output tray is missing",
        "output-area-full" => "Output tray is full",
        "offline" => "Printer is offline",
        "timed-out" => "Printer is not responding",
        "connecting-to-device" => "Connecting to the printer",
        "spool-area-full" => "Print queue is full",
        "shutdown" => "Printer is shutting down",
        _ if level == Level::Error => {
            return Some(Problem { level, text: format!("Needs attention: {}", base.replace('-', " ")) });
        }
        _ => return None,
    };
    Some(Problem { level, text: text.to_string() })
}

/// The problems of a printer, most serious first, each sentence once.
pub fn printer_problems(p: &Printer) -> Vec<Problem> {
    let mut problems: Vec<Problem> = p.state_reasons.iter().filter_map(|r| reason_problem(r)).collect();
    problems.sort_by(|a, b| b.level.cmp(&a.level));
    let mut seen = BTreeSet::new();
    problems.retain(|p| seen.insert(p.text.clone()));
    problems
}

/// The line under a printer's name when something is wrong: the sentences joined, with the worst level.
pub fn problem_line(p: &Printer) -> Option<(Level, String)> {
    let problems = printer_problems(p);
    let level = problems.first()?.level;
    Some((level, problems.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(", ")))
}

/// True for a queue that hands its jobs to a queue on another computer: its device address is a print queue path
/// (`ipp://host/printers/NAME`). Such a queue is remote, so it is not offered for sharing.
pub fn forwards_to_queue(p: &Printer) -> bool {
    if p.is_class {
        return false;
    }
    let Some((_, rest)) = p.device_uri.split_once("://") else { return false };
    let Some(slash) = rest.find('/') else { return false };
    let (host, path) = rest.split_at(slash);
    !host.is_empty() && (path.starts_with("/printers/") || path.starts_with("/classes/"))
}

/// The second line of a printer row: its state (or Offline), whether it is a pool, remote, shared or the default, then how
/// many jobs wait.
pub fn row_summary(p: &Printer, offline: bool) -> String {
    let mut parts = vec![if offline { "Offline".to_string() } else { printer_state_name(p.state).to_string() }];
    if p.is_class {
        parts.push(format!("Pool of {}", p.members.len()));
    } else if forwards_to_queue(p) {
        parts.push("Remote".into());
    } else if p.shared {
        parts.push("Shared".into());
    }
    if p.is_default {
        parts.push("Default".into());
    }
    if p.queued_jobs > 0 {
        parts.push(format!("{} in queue", p.queued_jobs));
    }
    parts.join(", ")
}

/// The summary of a printer that answers.
pub fn printer_summary(p: &Printer) -> String {
    row_summary(p, false)
}

/// The line under the printer's name in the detail pane: the members of a pool, make and model, location and the
/// scheduler's message.
pub fn printer_detail(p: &Printer) -> String {
    let members = if p.is_class && !p.members.is_empty() { format!("Members: {}", p.members.join(", ")) } else { String::new() };
    [members.as_str(), p.make_and_model.as_str(), p.location.as_str(), p.state_message.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// The label and value pairs of the info pane: what the person can read about the queue.
pub fn info_lines(p: &Printer, offline: bool) -> Vec<(String, String)> {
    let mut lines = vec![("Status".to_string(), if offline { "Offline".to_string() } else { printer_state_name(p.state).to_string() })];
    if let Some((_, problem)) = problem_line(p) {
        lines.push(("Needs attention".into(), problem));
    }
    let mut add = |label: &str, value: &str| {
        if !value.is_empty() {
            lines.push((label.to_string(), value.to_string()));
        }
    };
    add("Description", &p.info);
    add("Location", &p.location);
    add("Model", &p.make_and_model);
    add("Address", &p.device_uri);
    if p.is_class {
        add("Members", &p.members.join(", "));
    } else if forwards_to_queue(p) {
        add("Queue", "Sends jobs to a print queue on another computer");
    } else {
        add("Sharing", if p.shared { "Shared with other computers" } else { "Not shared" });
    }
    add("Default", if p.is_default { "Yes" } else { "No" });
    lines
}

/// What the device reports about a printer's options: the default paper and whether it prints on both sides and in
/// colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub paper: String,
    pub two_sided: bool,
    pub color: bool,
}

/// The option lines of the info pane, after the printer's own lines.
pub fn option_lines(o: &Options) -> Vec<(String, String)> {
    vec![
        ("Default paper".to_string(), o.paper.clone()),
        ("Two-sided".to_string(), yes_no(o.two_sided)),
        ("Color printing".to_string(), yes_no(o.color)),
    ]
}

fn yes_no(yes: bool) -> String {
    String::from(if yes { "Yes" } else { "No" })
}

/// The words beside a supply bar: its name and level.
pub fn supply_text(s: &Supply) -> String {
    match s.level {
        Some(l) => format!("{} {l}%", s.name),
        None => s.name.clone(),
    }
}

/// The colour of a supply as red, green and blue, from `#RRGGBB`.
pub fn supply_rgb(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((part(0)?, part(2)?, part(4)?))
}

/// The host and port to try a connection to for a device address, or `None` for an address that names no network host
/// (USB, a local socket, a service name that is looked up by the scheduler).
pub fn probe_target(device_uri: &str) -> Option<(String, u16)> {
    let (scheme, rest) = device_uri.split_once("://")?;
    let default_port: u16 = match scheme {
        "ipp" | "ipps" => 631,
        "socket" => 9100,
        "http" => 80,
        "https" => 443,
        "lpd" => 515,
        _ => return None,
    };
    let authority = rest.split(['/', '?']).next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (h, tail) = inner.split_once(']')?;
        (h, tail.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(default_port))
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        (h, p.parse().ok()?)
    } else {
        (authority, default_port)
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

/// A job's size for its column: kilobytes, or "" when the scheduler did not report one.
pub fn size_text(kb: i32) -> String {
    if kb > 0 {
        format!("{kb} KB")
    } else {
        String::new()
    }
}

/// A job's page count for its column, or "" when the scheduler does not know it yet.
pub fn pages_text(j: &Job) -> String {
    if j.pages > 0 {
        j.pages.to_string()
    } else {
        String::new()
    }
}

/// The sentence for one `job-state-reasons` keyword, `None` when the state word already says it.
pub fn job_reason_text(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "cups-remote-canceled" => "Canceled on server",
        "cups-remote-stopped" => "Paused on server",
        "cups-remote-aborted" => "Failed on server",
        "cups-held-for-authentication" | "job-hold-until-authentication" => "Held for authentication",
        "job-canceled-by-user" => "Canceled by user",
        "job-canceled-at-device" => "Canceled at the printer",
        "aborted-by-system" => "Stopped by the system",
        "printer-stopped" => "Printer is paused",
        "document-format-error" => "Unsupported document",
        "job-completed-with-errors" => "Completed with errors",
        "resources-are-not-ready" => "Waiting for paper or supplies",
        _ => return None,
    })
}

/// The words of a job's Status column: the page being printed, a cancel on its way ("Canceling page 3 of 5"), a reason
/// such as "Canceled on server", else the state.
pub fn job_status_text(j: &Job, canceling: bool) -> String {
    if j.state == JobState::Processing && (canceling || j.pages > 0) {
        let page = (j.pages_done + 1).min(j.pages);
        return match (canceling, j.pages > 0) {
            (true, true) => format!("Canceling page {page} of {}", j.pages),
            (true, false) => "Canceling".to_string(),
            (false, _) => format!("Printing {page} of {}", j.pages),
        };
    }
    j.reasons.iter().find_map(|r| job_reason_text(r)).map_or_else(|| job_state_name(j.state).to_string(), str::to_string)
}

/// How far a printing job is, from 0 to 1: the scheduler's percent, else the pages done. `None` for a job that is not
/// printing.
pub fn job_progress(j: &Job) -> Option<f32> {
    if !matches!(j.state, JobState::Processing | JobState::ProcessingStopped) {
        return None;
    }
    if let Some(p) = j.progress {
        return Some(f32::from(p) / 100.0);
    }
    (j.pages > 0).then(|| (j.pages_done as f32 / j.pages as f32).clamp(0.0, 1.0))
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// When a job was submitted, for its column: the time today, else the date. `created` and `now` are Unix seconds and
/// `offset` is the zone's offset from UTC in seconds. "" when the scheduler gave no time.
pub fn created_text(created: i32, offset: i32, now: i64) -> String {
    if created <= 0 {
        return String::new();
    }
    let local = i64::from(created) + i64::from(offset);
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (hour, minute) = (secs / 3600, secs % 3600 / 60);
    let today = (now + i64::from(offset)).div_euclid(86_400);
    if days == today {
        return format!("{hour:02}:{minute:02}");
    }
    let (year, month, day) = bifrost_app::calendar_system::civil_from_days(days);
    let (this_year, _, _) = bifrost_app::calendar_system::civil_from_days(today);
    let name = MONTHS[(month.clamp(1, 12) - 1) as usize];
    if year == this_year {
        format!("{name} {day}, {hour:02}:{minute:02}")
    } else {
        format!("{name} {day}, {year}")
    }
}

/// The Dock badge: the jobs waiting on all printers, or "" for none. Pools are left out, their jobs sit on a member.
pub fn badge_text(printers: &[Printer]) -> String {
    let total: i32 = printers.iter().filter(|p| !p.is_class).map(|p| p.queued_jobs.max(0)).sum();
    if total > 0 {
        total.to_string()
    } else {
        String::new()
    }
}

/// The document type to send a file as, from its extension. A type the scheduler detects itself goes as octet-stream.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e).to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "ps" => "application/postscript",
        "txt" | "log" | "md" | "text" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
}

/// Milliseconds between two listings while nothing is waiting or printing.
pub const IDLE_POLL_MS: f64 = 5000.0;
/// Milliseconds between two listings while a job is waiting or printing, so a change shows within about a second.
pub const ACTIVE_POLL_MS: f64 = 1000.0;

/// How long to wait before listing again: short while any printer has a job queued or is printing.
pub fn poll_interval_ms(printers: &[Printer]) -> f64 {
    let busy = printers.iter().any(|p| p.queued_jobs > 0 || p.state == PrinterState::Processing);
    if busy {
        ACTIVE_POLL_MS
    } else {
        IDLE_POLL_MS
    }
}

/// "1 job", "3 jobs".
fn count_word(n: i32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The title and the text of the printers widget: the most important thing first. A printer error, then a printer
/// that does not answer, then a paused printer, then the jobs waiting, else that the printers are ready.
pub fn widget_text(printers: &[Printer], offline: &BTreeSet<String>) -> (String, String) {
    if printers.is_empty() {
        return ("Printers".into(), "No printers are set up".into());
    }
    let error = printers.iter().find_map(|p| printer_problems(p).into_iter().find(|x| x.level == Level::Error).map(|x| (p, x.text)));
    if let Some((p, text)) = error {
        return (format!("{} needs attention", p.name), text);
    }
    if let Some(name) = printers.iter().find(|p| offline.contains(&p.name)).map(|p| &p.name) {
        return (name.clone(), "Printer is offline".into());
    }
    let waiting: i32 = printers.iter().filter(|p| !p.is_class).map(|p| p.queued_jobs.max(0)).sum();
    if let Some(p) = printers.iter().find(|p| p.state == PrinterState::Stopped) {
        let text = if waiting > 0 { format!("Paused, {} waiting", count_word(waiting, "job", "jobs")) } else { "Paused".to_string() };
        return (p.name.clone(), text);
    }
    if waiting > 0 {
        let printing = printers.iter().filter(|p| !p.is_class && p.state == PrinterState::Processing).count();
        let text = if printing > 0 { format!("{} in the queue, printing", count_word(waiting, "job", "jobs")) } else { format!("{} waiting", count_word(waiting, "job", "jobs")) };
        return ("Printers".into(), text);
    }
    let text = match printers {
        [only] => format!("{} is ready", only.name),
        many => format!("All {} printers are ready", many.len()),
    };
    ("Printers".into(), text)
}

/// The sentence the List Printers action says: how many printers there are and how many jobs wait.
pub fn list_dialog(printers: &[Printer]) -> String {
    if printers.is_empty() {
        return "No printers are set up".into();
    }
    let waiting: i32 = printers.iter().filter(|p| !p.is_class).map(|p| p.queued_jobs.max(0)).sum();
    let count = count_word(printers.len() as i32, "printer", "printers");
    if waiting > 0 {
        format!("{count}, {} waiting", count_word(waiting, "job", "jobs"))
    } else {
        format!("{count}, no jobs waiting")
    }
}

/// The short label of a document's tile in the queue: its extension in capitals, else "DOC".
pub fn document_tag(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => ext.to_ascii_uppercase(),
        _ => "DOC".to_string(),
    }
}

/// A queue name from a printer's advertised label: spaces become underscores, characters the scheduler does not take
/// are dropped, and a name already in `taken` gets `-2`, `-3`. "Printer" when nothing is left.
pub fn queue_name_for(label: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in label.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
            base.push(c);
        } else if (c == ' ' || c == '_') && !base.ends_with('_') {
            base.push('_');
        }
    }
    let base: String = base.trim_matches(|c| c == '_' || c == '-' || c == '.').chars().take(60).collect();
    let base = if base.is_empty() { "Printer".to_string() } else { base };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// The help topics: a heading and the text under it.
pub const HELP: [(&str, &str); 6] = [
    (
        "Add a printer",
        "Choose Add Printer to list the printers on this network, then click one to add it with its name and location. For a printer that is not listed, copy its address, such as ipp://printer.local/ipp/print, and choose Use Address from Clipboard.",
    ),
    (
        "Print a document",
        "Select a printer, then choose Print Document in the Printer menu, or drop files on the window or on the Dock icon.",
    ),
    (
        "Manage jobs",
        "Drag over the rows, or choose Select All Jobs, to pick several jobs. Hold, Release and Cancel apply to every chosen job, and a secondary click on a job offers the same actions.",
    ),
    (
        "Printer problems",
        "A warning mark under a printer's name says what needs attention, such as a paper jam. A printer that stops answering is dimmed and marked Offline. Errors are announced as a notification and the Dock icon bounces.",
    ),
    (
        "Details, sharing and removal",
        "Printer Info edits the description and location, lists the supply levels and turns sharing on or off. Remove Printer asks for a second press.",
    ),
    (
        "Shortcuts and the widget",
        "List Printers, Pause Printer, Resume Printer, Set Default Printer, Cancel Print Job and Print File are Shortcuts actions. The printers widget in Notification Centre shows what needs attention first.",
    ),
];

/// Breaks `text` into lines of at most `max_chars` characters at word boundaries. A word longer than a line gets a line
/// of its own.
pub fn wrap_words(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let joined = if line.is_empty() { word.chars().count() } else { line.chars().count() + 1 + word.chars().count() };
        if joined > max_chars && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
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
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    fn job(id: i32, state: JobState) -> Job {
        Job { id, uri: String::new(), state, name: format!("doc{id}"), user: "olibuijr".into(), size_kb: 12, reasons: Vec::new(), created: 0, pages: 0, pages_done: 0, progress: None }
    }

    fn listed(printers: Vec<Printer>, printer: Option<&str>, jobs: Vec<Job>) -> Reply {
        Reply::Listed { printers, printer: printer.map(str::to_string), jobs, offline: vec![] }
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
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 1), printer("Lab", false, PrinterState::Idle, 0)], None, vec![]));
        assert_eq!(m.selected.as_deref(), Some("Office"));
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 2)], Some("Office"), vec![job(4, JobState::Pending), job(5, JobState::PendingHeld)]));
        assert_eq!(m.jobs.len(), 2);
        m.select_job(5);
        assert_eq!(m.chosen_jobs().first().map(|j| j.name.as_str()), Some("doc5"));
        // the job is released from the queue: the next listing clears the selection
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 1)], Some("Office"), vec![job(4, JobState::Pending)]));
        assert!(m.selected_jobs.is_empty());
    }

    #[test]
    fn several_jobs_can_be_chosen_and_the_actions_apply_to_each_that_allows_them() {
        let mut m = Model::default();
        m.apply(listed(
            vec![printer("Office", true, PrinterState::Idle, 3)],
            Some("Office"),
            vec![job(1, JobState::Pending), job(2, JobState::Pending), job(3, JobState::PendingHeld)],
        ));
        m.select_all_jobs();
        assert_eq!(m.selected_jobs.len(), 3);
        assert_eq!(m.chosen_ids(can_hold), vec![1, 2]);
        assert_eq!(m.chosen_ids(can_release), vec![3]);
        assert_eq!(m.chosen_ids(can_cancel), vec![1, 2, 3]);
        m.toggle_job(2);
        assert_eq!(m.chosen_ids(can_hold), vec![1]);
        m.toggle_job(2);
        assert!(m.selected_jobs.contains(&2));
        m.select_job(3);
        assert_eq!(m.selected_jobs.len(), 1, "choosing one job drops the others");
    }

    #[test]
    fn a_queue_listed_for_another_printer_is_ignored() {
        let mut m = Model::default();
        let two = || vec![printer("Office", true, PrinterState::Idle, 0), printer("Lab", false, PrinterState::Idle, 0)];
        m.apply(listed(two(), None, vec![]));
        m.select_printer("Lab".into());
        m.apply(listed(two(), Some("Office"), vec![job(1, JobState::Pending)]));
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
        m.apply(Reply::Printed("a.pdf".into()));
        assert_eq!(m.status, "Sent \"a.pdf\" to the printer");
        m.apply(Reply::Failed("printing service unreachable".into()));
        assert_eq!(m.status, "printing service unreachable");
    }

    #[test]
    fn reasons_become_sentences_with_a_level() {
        assert_eq!(reason_problem("media-jam-error"), Some(Problem { level: Level::Error, text: "Paper jam".into() }));
        assert_eq!(reason_problem("toner-low-warning").map(|p| p.level), Some(Level::Warning));
        assert_eq!(reason_problem("offline-report").map(|p| p.text), Some("Printer is offline".into()));
        assert_eq!(reason_problem("media-empty-error").map(|p| p.text), Some("Out of paper".into()));
        assert_eq!(reason_problem("door-open-error").map(|p| p.text), Some("A door or cover is open".into()));
        assert_eq!(reason_problem("cups-ipp-conformance-failure-report"), None);
        assert_eq!(reason_problem("odd-thing-error").map(|p| p.text), Some("Needs attention: odd thing".into()));
    }

    #[test]
    fn problems_are_sorted_worst_first_and_joined_for_the_line() {
        let mut p = printer("Lab", false, PrinterState::Idle, 0);
        assert_eq!(problem_line(&p), None);
        p.state_reasons = vec!["toner-low-warning".into(), "media-jam-error".into(), "media-jam-error".into(), "nothing-to-say-report".into()];
        let problems = printer_problems(&p);
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].text, "Paper jam");
        assert_eq!(problem_line(&p), Some((Level::Error, "Paper jam, Toner is running low".into())));
    }

    #[test]
    fn an_error_is_announced_once_until_it_clears() {
        let mut m = Model::default();
        let mut jammed = printer("Lab", false, PrinterState::Stopped, 0);
        jammed.state_reasons = vec!["media-jam-error".into(), "toner-low-warning".into()];
        m.apply(listed(vec![jammed.clone()], None, vec![]));
        assert_eq!(m.take_alerts(), vec![("Lab".to_string(), "Paper jam".to_string())], "warnings are not announced");
        m.apply(listed(vec![jammed.clone()], None, vec![]));
        assert!(m.take_alerts().is_empty());
        m.apply(listed(vec![printer("Lab", false, PrinterState::Idle, 0)], None, vec![]));
        m.apply(listed(vec![jammed], None, vec![]));
        assert_eq!(m.take_alerts().len(), 1, "a jam that returns is announced again");
    }

    #[test]
    fn rows_say_offline_pool_and_shared() {
        let mut p = printer("Lab", true, PrinterState::Idle, 1);
        assert_eq!(row_summary(&p, true), "Offline, Default, 1 in queue");
        p.shared = true;
        assert_eq!(row_summary(&p, false), "Idle, Shared, Default, 1 in queue");
        p.is_class = true;
        p.members = vec!["a".into(), "b".into()];
        assert_eq!(row_summary(&p, false), "Idle, Pool of 2, Default, 1 in queue");
        assert_eq!(printer_detail(&p), "Members: a, b");
        let mut m = Model::default();
        m.apply(Reply::Listed { printers: vec![p], printer: None, jobs: vec![], offline: vec!["Lab".into()] });
        assert!(m.is_offline("Lab"));
        assert!(!m.is_offline("Other"));
    }

    #[test]
    fn a_queue_that_forwards_to_another_computer_is_remote_not_shared() {
        let mut p = printer("Lab", false, PrinterState::Idle, 0);
        p.device_uri = "ipp://front.local:631/printers/Front".into();
        assert!(forwards_to_queue(&p));
        p.shared = true;
        assert_eq!(row_summary(&p, false), "Idle, Remote");
        assert!(info_lines(&p, false).iter().any(|(l, v)| l == "Queue" && v.contains("another computer")));
        p.device_uri = "socket://192.168.1.20:9100".into();
        assert!(!forwards_to_queue(&p));
        assert_eq!(row_summary(&p, false), "Idle, Shared");
        p.device_uri = "ipp:///printers/Front".into();
        assert!(!forwards_to_queue(&p), "a device address needs a host");
        p.device_uri = "ipp://front.local/ipp/print".into();
        assert!(!forwards_to_queue(&p), "an IPP Everywhere queue is not a forward");
        p.device_uri = "ipp://front.local/classes/Floor".into();
        assert!(forwards_to_queue(&p), "a class on another computer forwards too");
    }

    #[test]
    fn the_options_read_as_info_lines_and_wait_for_the_selected_printer() {
        let options = Options { paper: "Letter".into(), two_sided: true, color: false };
        assert_eq!(
            option_lines(&options),
            vec![
                ("Default paper".to_string(), "Letter".to_string()),
                ("Two-sided".to_string(), "Yes".to_string()),
                ("Color printing".to_string(), "No".to_string()),
            ]
        );
        let mut m = Model::default();
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 0), printer("Lab", false, PrinterState::Idle, 0)], Some("Office"), vec![]));
        m.apply(Reply::Options { printer: "Office".into(), options: Some(options.clone()) });
        assert_eq!(m.options_for("Office"), Some(&options));
        assert_eq!(m.options_for("Lab"), None);
        m.apply(Reply::Options { printer: "Lab".into(), options: Some(options.clone()) });
        assert_eq!(m.options_for("Lab"), None, "an answer for another printer is dropped");
        m.select_printer("Lab".into());
        assert_eq!(m.options_for("Office"), None, "choosing another printer forgets the options");
        m.apply(Reply::Options { printer: "Lab".into(), options: None });
        assert_eq!(m.options_for("Lab"), None, "a device that reports nothing has no options");
    }

    #[test]
    fn supplies_read_as_text_and_colour() {
        let black = Supply { name: "Black".into(), level: Some(80), color: "#000000".into() };
        assert_eq!(supply_text(&black), "Black 80%");
        assert_eq!(supply_text(&Supply { name: "Cyan".into(), level: None, color: String::new() }), "Cyan");
        assert_eq!(supply_rgb("#00FFFF"), Some((0, 255, 255)));
        assert_eq!(supply_rgb("00FFFF"), None);
        assert_eq!(supply_rgb("#00FF"), None);
        assert_eq!(supply_rgb("#00FFGG"), None);
    }

    #[test]
    fn the_info_pane_lists_what_is_known() {
        let mut p = printer("Lab", true, PrinterState::Idle, 0);
        p.make_and_model = "HP LaserJet".into();
        p.location = "Floor 2".into();
        p.shared = true;
        p.state_reasons = vec!["media-empty-error".into()];
        let lines = info_lines(&p, false);
        let get = |k: &str| lines.iter().find(|(l, _)| l == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Status"), Some("Idle"));
        assert_eq!(get("Needs attention"), Some("Out of paper"));
        assert_eq!(get("Model"), Some("HP LaserJet"));
        assert_eq!(get("Sharing"), Some("Shared with other computers"));
        assert_eq!(get("Description"), None, "empty values are left out");
        assert_eq!(info_lines(&p, true)[0].1, "Offline");
    }

    #[test]
    fn device_addresses_give_a_host_and_port_to_probe() {
        assert_eq!(probe_target("ipp://printer.local/ipp/print"), Some(("printer.local".into(), 631)));
        assert_eq!(probe_target("socket://192.168.1.20:9100"), Some(("192.168.1.20".into(), 9100)));
        assert_eq!(probe_target("socket://192.168.1.20"), Some(("192.168.1.20".into(), 9100)));
        assert_eq!(probe_target("ipps://user@host:8631/x"), Some(("host".into(), 8631)));
        assert_eq!(probe_target("ipp://[fe80::1]:632/ipp"), Some(("fe80::1".into(), 632)));
        assert_eq!(probe_target("usb://HP/LaserJet"), None);
        assert_eq!(probe_target("dnssd://Printer._ipp._tcp.local/"), None);
        assert_eq!(probe_target("ipp:///x"), None);
        assert_eq!(probe_target("ipp://host:bad/x"), None);
        assert_eq!(probe_target(""), None);
    }

    #[test]
    fn a_job_status_names_the_page_or_the_reason() {
        let mut j = job(1, JobState::Processing);
        assert_eq!(job_status_text(&j, false), "Printing");
        j.pages = 10;
        j.pages_done = 3;
        assert_eq!(job_status_text(&j, false), "Printing 4 of 10");
        j.pages_done = 10;
        assert_eq!(job_status_text(&j, false), "Printing 10 of 10");
        let mut c = job(2, JobState::Canceled);
        c.reasons = vec!["cups-remote-canceled".into()];
        assert_eq!(job_status_text(&c, false), "Canceled on server");
        let mut h = job(3, JobState::PendingHeld);
        h.reasons = vec!["job-hold-until-specified".into()];
        assert_eq!(job_status_text(&h, false), "Held");
        h.reasons = vec!["cups-held-for-authentication".into()];
        assert_eq!(job_status_text(&h, false), "Held for authentication");
    }

    #[test]
    fn a_cancel_on_its_way_shows_the_page_being_canceled() {
        let mut j = job(1, JobState::Processing);
        j.pages = 5;
        j.pages_done = 1;
        assert_eq!(job_status_text(&j, true), "Canceling page 2 of 5");
        assert_eq!(job_status_text(&j, false), "Printing 2 of 5");
        let unknown = job(2, JobState::Processing);
        assert_eq!(job_status_text(&unknown, true), "Canceling");
        let waiting = job(3, JobState::Pending);
        assert_eq!(job_status_text(&waiting, true), "Waiting", "only a printing job reads Canceling");
    }

    #[test]
    fn the_model_marks_a_cancel_until_the_job_leaves_the_queue() {
        let mut m = Model::default();
        m.apply(listed(vec![printer("Office", true, PrinterState::Processing, 1)], Some("Office"), vec![job(4, JobState::Processing)]));
        let mut canceled = job(4, JobState::Processing);
        canceled.pages = 3;
        canceled.pages_done = 0;
        m.mark_canceling("Office", 4);
        assert_eq!(m.job_status(&canceled), "Canceling page 1 of 3");
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 0)], Some("Office"), vec![]));
        assert!(m.canceling.is_empty(), "the job left the queue");
        m.mark_canceling("Lab", 4);
        m.apply(listed(vec![printer("Office", true, PrinterState::Idle, 0)], Some("Office"), vec![]));
        assert_eq!(m.canceling.len(), 1, "a job of another printer is kept");
    }

    #[test]
    fn progress_comes_from_the_percent_or_the_pages() {
        let mut j = job(1, JobState::Processing);
        assert_eq!(job_progress(&j), None);
        j.pages = 4;
        j.pages_done = 1;
        assert_eq!(job_progress(&j), Some(0.25));
        j.progress = Some(60);
        assert_eq!(job_progress(&j), Some(0.6));
        assert_eq!(job_progress(&job(2, JobState::Pending)), None);
        assert_eq!(pages_text(&j), "4");
        assert_eq!(pages_text(&job(3, JobState::Pending)), "");
    }

    #[test]
    fn submitted_times_show_the_clock_today_and_the_date_before() {
        // 2026-10-13 12:00:00 UTC
        let now = 1_791_892_800_i64;
        assert_eq!(created_text(0, 0, now), "");
        assert_eq!(created_text((now - 3600) as i32, 0, now), "11:00");
        assert_eq!(created_text((now - 3600) as i32, 3600, now), "12:00", "the zone offset moves the clock");
        assert_eq!(created_text((now - 86_400) as i32, 0, now), "Oct 12, 12:00");
        assert_eq!(created_text((now - 400 * 86_400) as i32, 0, now), "Sep 8, 2025");
    }

    #[test]
    fn the_badge_counts_the_waiting_jobs_but_not_those_of_pools() {
        let mut pool = printer("Floor", false, PrinterState::Idle, 5);
        pool.is_class = true;
        assert_eq!(badge_text(&[printer("a", false, PrinterState::Idle, 2), printer("b", false, PrinterState::Idle, 1), pool]), "3");
        assert_eq!(badge_text(&[printer("a", false, PrinterState::Idle, 0)]), "");
    }

    #[test]
    fn documents_are_sent_with_a_type_from_their_extension() {
        assert_eq!(mime_for("Report.PDF"), "application/pdf");
        assert_eq!(mime_for("notes.txt"), "text/plain");
        assert_eq!(mime_for("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_for("archive.xyz"), "application/octet-stream");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }

    #[test]
    fn the_listing_is_quick_while_a_job_waits_or_prints() {
        assert_eq!(poll_interval_ms(&[]), IDLE_POLL_MS);
        assert_eq!(poll_interval_ms(&[printer("a", false, PrinterState::Idle, 0)]), IDLE_POLL_MS);
        assert_eq!(poll_interval_ms(&[printer("a", false, PrinterState::Idle, 0), printer("b", false, PrinterState::Idle, 1)]), ACTIVE_POLL_MS);
        assert_eq!(poll_interval_ms(&[printer("a", false, PrinterState::Processing, 0)]), ACTIVE_POLL_MS);
    }

    #[test]
    fn the_widget_leads_with_the_most_important_thing() {
        let none = BTreeSet::new();
        assert_eq!(widget_text(&[], &none), ("Printers".to_string(), "No printers are set up".to_string()));
        let ready = printer("Office", true, PrinterState::Idle, 0);
        assert_eq!(widget_text(std::slice::from_ref(&ready), &none).1, "Office is ready");
        assert_eq!(widget_text(&[ready.clone(), printer("Lab", false, PrinterState::Idle, 0)], &none).1, "All 2 printers are ready");
        let waiting = printer("Office", true, PrinterState::Idle, 1);
        assert_eq!(widget_text(&[waiting], &none).1, "1 job waiting");
        let printing = printer("Office", true, PrinterState::Processing, 3);
        assert_eq!(widget_text(&[printing], &none).1, "3 jobs in the queue, printing");
        let paused = printer("Lab", false, PrinterState::Stopped, 2);
        assert_eq!(widget_text(&[ready.clone(), paused], &none), ("Lab".to_string(), "Paused, 2 jobs waiting".to_string()));
        let offline: BTreeSet<String> = ["Office".to_string()].into();
        assert_eq!(widget_text(std::slice::from_ref(&ready), &offline), ("Office".to_string(), "Printer is offline".to_string()));
        let mut jam = printer("Lab", false, PrinterState::Stopped, 0);
        jam.state_reasons = vec!["media-jam-error".into()];
        assert_eq!(widget_text(&[ready, jam], &offline), ("Lab needs attention".to_string(), "Paper jam".to_string()), "an error outranks an offline printer");
    }

    #[test]
    fn the_list_dialog_counts_printers_and_waiting_jobs() {
        assert_eq!(list_dialog(&[]), "No printers are set up");
        assert_eq!(list_dialog(&[printer("a", true, PrinterState::Idle, 0)]), "1 printer, no jobs waiting");
        assert_eq!(list_dialog(&[printer("a", true, PrinterState::Idle, 2), printer("b", false, PrinterState::Idle, 1)]), "2 printers, 3 jobs waiting");
    }

    #[test]
    fn a_document_tile_shows_the_extension() {
        assert_eq!(document_tag("report.pdf"), "PDF");
        assert_eq!(document_tag("photo.jpeg"), "JPEG");
        assert_eq!(document_tag("archive.tar.gz"), "GZ");
        assert_eq!(document_tag("notes"), "DOC");
        assert_eq!(document_tag(".hidden"), "DOC");
        assert_eq!(document_tag("long.extension"), "DOC");
        assert_eq!(document_tag("trailing."), "DOC");
    }

    #[test]
    fn a_queue_name_from_a_label_is_valid_and_never_replaces_a_queue() {
        assert_eq!(queue_name_for("HP LaserJet 400", &[]), "HP_LaserJet_400");
        assert_eq!(queue_name_for("  Front  desk (2) ", &[]), "Front_desk_2");
        assert_eq!(queue_name_for("Brother HL", &["Brother_HL".to_string()]), "Brother_HL-2");
        assert_eq!(queue_name_for("Brother HL", &["Brother_HL".to_string(), "Brother_HL-2".to_string()]), "Brother_HL-3");
        assert_eq!(queue_name_for("\u{1F5A8}", &[]), "Printer");
        assert!(bifrost_app::controls::cups::valid_printer_name(&queue_name_for("Büro Drucker #1", &[])));
        assert!(queue_name_for(&"x".repeat(200), &[]).len() <= 60);
    }

    #[test]
    fn one_pane_shows_at_a_time_and_a_second_choice_returns_to_the_queue() {
        let mut m = Model::default();
        assert_eq!(m.pane(), Pane::Queue);
        m.toggle_pane(Pane::Info);
        assert!(m.show_info && !m.show_discover && !m.show_help);
        m.toggle_pane(Pane::Discover);
        assert_eq!(m.pane(), Pane::Discover);
        assert!(!m.show_info, "the info pane gave way");
        m.toggle_pane(Pane::Help);
        assert_eq!(m.pane(), Pane::Help);
        m.toggle_pane(Pane::Help);
        assert_eq!(m.pane(), Pane::Queue);
    }

    #[test]
    fn help_topics_wrap_at_word_boundaries() {
        assert!(HELP.iter().all(|(heading, body)| !heading.is_empty() && !body.is_empty()));
        let lines = wrap_words("one two three four", 9);
        assert_eq!(lines, vec!["one two", "three", "four"]);
        assert_eq!(wrap_words("extraordinarily long", 5), vec!["extraordinarily", "long"]);
        assert!(wrap_words("   ", 10).is_empty());
        for (_, body) in HELP {
            assert!(wrap_words(body, 60).iter().all(|l| l.chars().count() <= 60), "no help line is wider than the limit");
        }
    }

    #[test]
    fn the_listing_flag_marks_the_first_answer() {
        let mut m = Model::default();
        assert!(!m.listed);
        m.apply(Reply::Failed("no scheduler".into()));
        assert!(!m.listed, "an error is not a listing");
        m.apply(listed(vec![], None, vec![]));
        assert!(m.listed);
    }
}
