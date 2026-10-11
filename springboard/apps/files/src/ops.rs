//! File operations (copy, move, duplicate, rename, trash, new folder, compress, extract) on the dispatch queue, with progress
//! and the name-conflict prompt (Files's copy engine).
use crate::*;

/// File in the Recently Deleted folder that records where each item came from.
pub(crate) const ORIGINS: &str = ".origins";

/// Buttons of the conflict prompt: (label, choice, left edge, width), in points.
pub(crate) const PROMPT_TOP: f32 = 168.;

pub(crate) const PROMPT_BUTTONS: [(&str, Resolution, f32, f32); 3] = [
    ("Keep Both", Resolution::KeepBoth, 244., 110.),
    ("Replace", Resolution::Replace, 364., 90.),
    ("Skip", Resolution::Skip, 464., 70.),
];

pub(crate) const PROMPT_ALL_X: f32 = 544.;

/// What a background job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    /// Paste: a taken name asks the user.
    Transfer(Op),
    /// Duplicate: a taken name keeps both, without asking.
    Duplicate,
    Trash,
    Compress,
    Extract,
}

/// The sending end of a worker's channel. Each send also wakes the app loop through the main queue, so the loop runs
/// `tick` and reads the message: no timer is needed while a job or a search runs (docs/bifrost-ui.md "Dispatch").
pub(crate) struct Feed<T>(Sender<T>);

/// A channel from a worker to the UI: the worker's `Feed` and the UI's receiver.
pub(crate) fn feed<T>() -> (Feed<T>, Receiver<T>) {
    let (tx, rx) = mpsc::channel();
    (Feed(tx), rx)
}

/// Messages from a job thread to the UI.
pub(crate) enum Event {
    Progress(Progress),
    /// A name is taken: the job waits for the UI's answer.
    Conflict(String),
    Done(Report),
}

/// A copy, move, duplicate or trash running on its own thread.
pub(crate) struct Work {
    pub(crate) rx: Receiver<Event>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) title: String,
    pub(crate) kind: JobKind,
    pub(crate) dest: PathBuf,
    pub(crate) sources: Vec<PathBuf>,
    pub(crate) progress: Progress,
    /// The taken name the job is waiting on, while the prompt shows.
    pub(crate) conflict: Option<String>,
    /// Apply the next answer to the rest of the batch.
    pub(crate) apply_all: bool,
    /// The UI's answers to the job (None cancels it).
    pub(crate) answers: Sender<Answer>,
}

/// Records that the trashed item `stored` in `bin` came from `origin`.
pub(crate) fn record_origin(bin: &Path, stored: &str, origin: &Path) {
    let Some(origin) = origin.to_str().filter(|o| !o.contains('\t') && !o.contains('\n')) else { return };
    let mut lines = read_origins(bin);
    lines.retain(|(name, _)| name != stored);
    lines.push((stored.to_string(), origin.to_string()));
    write_origins(bin, &lines);
}

/// Removes and returns the recorded origin of `stored` in `bin`.
pub(crate) fn take_origin(bin: &Path, stored: &str) -> Option<PathBuf> {
    let mut lines = read_origins(bin);
    let at = lines.iter().position(|(name, _)| name == stored)?;
    let (_, origin) = lines.remove(at);
    write_origins(bin, &lines);
    Some(PathBuf::from(origin))
}

/// Drops origins for items that are no longer in `bin` (purged or restored by hand).
pub(crate) fn prune_origins(bin: &Path) {
    let lines: Vec<_> = read_origins(bin).into_iter().filter(|(name, _)| bin.join(name).symlink_metadata().is_ok()).collect();
    write_origins(bin, &lines);
}

pub(crate) fn read_origins(bin: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(bin.join(ORIGINS))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string())))
        .collect()
}

pub(crate) fn write_origins(bin: &Path, lines: &[(String, String)]) {
    let text: String = lines.iter().map(|(a, b)| format!("{a}\t{b}\n")).collect();
    let _ = springboard::shell::fsutil::write_atomic(&bin.join(ORIGINS), text);
}

/// Copies or moves `sources` into `dest` on this thread, reporting progress to the UI.
/// A name already in use gets a numbered copy: a paste never overwrites.
pub(crate) fn transfer_now(op: Op, sources: &[PathBuf], dest: &Path, cancel: &AtomicBool, tx: &Feed<Event>, answers: Option<&Receiver<Answer>>) -> Report {
    // a taken name waits for the UI's choice; a closed channel cancels the batch.
    // Without a channel (duplicate) a taken name keeps both.
    let mut ask = |target: &Path| -> Answer {
        let Some(answers) = answers else {
            return Some((Resolution::KeepBoth, true));
        };
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if tx.send(Event::Conflict(name)).is_err() {
            return None;
        }
        answers.recv().unwrap_or(None)
    };
    let mut last = Instant::now();
    let mut tick = |p: &Progress| {
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            let _ = tx.send(Event::Progress(p.clone()));
        }
        !cancel.load(Ordering::Relaxed)
    };
    batch::transfer(op, sources, dest, &mut ask, &mut tick)
}

/// Writes a .zip of `sources` into `dest` on this thread.
pub(crate) fn compress_now(sources: &[PathBuf], dest: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    let mut tick = |done: u64, total: u64| {
        let _ = tx.send(Event::Progress(Progress { bytes_done: done, bytes_total: total, items_total: sources.len(), ..Default::default() }));
        !cancel.load(Ordering::Relaxed)
    };
    match zip::create(sources, dest, &mut tick) {
        Ok(out) => report.done.push(out),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => report.cancelled = true,
        Err(e) => report.failed.push((sources.first().cloned().unwrap_or_default(), e.to_string())),
    }
    report
}

/// Unpacks a .zip into a new folder beside it on this thread.
pub(crate) fn extract_now(archive: &Path, dest: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    let mut tick = |done: u64, total: u64| {
        let _ = tx.send(Event::Progress(Progress { bytes_done: done, bytes_total: total, items_total: 1, ..Default::default() }));
        !cancel.load(Ordering::Relaxed)
    };
    match zip::extract(archive, dest, &mut tick) {
        Ok(out) => report.done.push(out.dir),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => report.cancelled = true,
        Err(e) => report.failed.push((archive.to_path_buf(), e.to_string())),
    }
    report
}

/// Moves each item into the Recently Deleted folder, recording where it came from.
pub(crate) fn trash_now(sources: &[PathBuf], bin: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    trash::purge(bin, trash::now_secs());
    for (n, path) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let current = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = tx.send(Event::Progress(Progress { items_done: n, items_total: sources.len(), current, ..Default::default() }));
        match trash::discard(path, bin, trash::now_secs()) {
            Ok(stored) => {
                if let (Some(name), Some(parent)) = (stored.file_name(), path.parent()) {
                    record_origin(bin, &name.to_string_lossy(), parent);
                }
                report.moved.push((path.clone(), stored.clone()));
                report.done.push(path.clone());
            }
            Err(e) => report.failed.push((path.clone(), batch::friendly(&e))),
        }
    }
    prune_origins(bin);
    report
}

impl<T> Feed<T> {
    pub(crate) fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        let sent = self.0.send(value);
        DispatchQueue::main().async_(|| {});
        sent
    }
}

impl Files {
    /// Copies or cuts the selected items for Paste.
    pub(crate) fn copy_selected(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            self.clip = Some((paths, cut));
        }
    }
    pub(crate) fn start_job(&mut self, title: &str, kind: JobKind, sources: Vec<PathBuf>, dest: PathBuf) {
        if self.work.is_some() {
            self.notice = Some("Wait for the current job to finish".into());
            return;
        }
        let (tx, rx) = feed();
        let (answers, answer_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (s, d, bin) = (sources.clone(), dest.clone(), self.bin());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let report = match kind {
                JobKind::Transfer(op) => transfer_now(op, &s, &d, &flag, &tx, Some(&answer_rx)),
                JobKind::Duplicate => transfer_now(Op::Copy, &s, &d, &flag, &tx, None),
                JobKind::Trash => trash_now(&s, &bin, &flag, &tx),
                JobKind::Compress => compress_now(&s, &d, &flag, &tx),
                JobKind::Extract => extract_now(&s[0], &d, &flag, &tx),
            };
            let _ = tx.send(Event::Done(report));
        });
        self.work = Some(Work {
            rx,
            cancel,
            title: title.into(),
            kind,
            dest,
            progress: Progress { items_total: sources.len(), ..Default::default() },
            sources,
            conflict: None,
            apply_all: false,
            answers,
        });
    }
    /// Called on the UI thread when a job reports done.
    pub(crate) fn finish_job(&mut self, work: Work, report: Report) {
        if work.kind == JobKind::Transfer(Op::Move) && report.failed.is_empty() && !report.cancelled {
            self.clip = None;
        }
        let verb = match work.kind {
            JobKind::Transfer(Op::Copy) => "copied",
            JobKind::Duplicate => "duplicated",
            JobKind::Transfer(Op::Move) => "moved",
            JobKind::Trash => "moved to the Trash",
            JobKind::Compress => "compressed",
            JobKind::Extract => "extracted",
        };
        let notice = if report.stopped {
            let name = report.failed.first().and_then(|(p, _)| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(format!("Stopped: the disk is full. {} of {} items {verb}; {name} and the rest were not", report.done.len(), work.sources.len()))
        } else if report.cancelled {
            Some(format!("Cancelled after {} of {} items", report.done.len(), work.sources.len()))
        } else if let Some((path, why)) = report.failed.first() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(format!("{} of {} items could not be {verb}. {name}: {why}", report.failed.len(), work.sources.len()))
        } else {
            None
        };
        if !report.moved.is_empty() {
            match work.kind {
                JobKind::Transfer(Op::Move) => self.push_undo(Undo::Move(report.moved.clone())),
                JobKind::Trash => self.push_undo(Undo::Trash(report.moved.clone())),
                _ => {}
            }
        }
        // follow the items that moved, were copied, or went to Recently Deleted
        let done = report.done.clone();
        match work.kind {
            JobKind::Transfer(op) => {
                for src in &work.sources {
                    let Some(name) = src.file_name() else { continue };
                    let to = work.dest.join(name);
                    if done.contains(&to) {
                        match op {
                            Op::Move => self.with_tags(|index| index.moved(src, &to)),
                            Op::Copy => self.with_tags(|index| index.copied(src, &to)),
                        }
                    }
                }
            }
            JobKind::Trash => {
                for src in done.iter() {
                    self.with_tags(|index| index.removed(src));
                }
            }
            _ => {}
        }
        let affected = self.path == work.dest
            || self.in_bin()
            || work.sources.iter().any(|s| s.parent() == Some(self.path.as_path()));
        if affected {
            let dir = self.path.clone();
            self.navigate(dir, false);
        }
        self.notice = notice;
    }
    pub(crate) fn cancel_job(&mut self) {
        if let Some(work) = &self.work {
            work.cancel.store(true, Ordering::Relaxed);
            // a job waiting on the prompt must hear that it is cancelled
            let _ = work.answers.send(None);
        }
    }
    /// Sends the prompt's choice to the job and hides the prompt.
    pub(crate) fn answer_conflict(&mut self, choice: Resolution) {
        if let Some(work) = self.work.as_mut() {
            if work.conflict.take().is_some() {
                let answer = Some((choice, work.apply_all));
                let _ = work.answers.send(answer);
                self.revision += 1;
            }
        }
    }
    pub(crate) fn is_zip(entry: &Entry) -> bool {
        !entry.is_dir && entry.name.to_lowercase().ends_with(".zip")
    }
    /// Compress the selection into a .zip in the open folder.
    pub(crate) fn compress(&mut self) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dest = self.path.clone();
        self.start_job("Compressing", JobKind::Compress, paths, dest);
    }
    /// Extract the focused .zip into a new folder beside it.
    pub(crate) fn extract(&mut self) {
        let Some(entry) = self.focus_entry().filter(|e| Self::is_zip(e)).map(|e| e.path.clone()) else {
            return;
        };
        let dest = self.path.clone();
        self.start_job("Extracting", JobKind::Extract, vec![entry], dest);
    }
    pub(crate) fn trash_selected(&mut self) {
        if self.in_bin() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dest = self.path.clone();
        self.start_job("Moving to the Trash", JobKind::Trash, paths, dest);
    }
    /// Puts trashed items back where they came from (Home when the folder is gone).
    pub(crate) fn put_back(&mut self) {
        let bin = self.bin();
        let home = self.home.clone();
        let mut failed = None;
        for path in self.selected_paths() {
            let stored = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let target = take_origin(&bin, &stored).filter(|o| o.is_dir()).unwrap_or_else(|| home.clone());
            if let Err(e) = trash::restore(&path, &target) {
                failed = Some(e.to_string());
            }
        }
        let dir = self.path.clone();
        self.navigate(dir, false);
        self.notice = failed.map(|e| format!("Could not put back: {e}"));
    }
    /// Copies the selected items into their own folder under new names.
    pub(crate) fn duplicate(&mut self) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dest = self.path.clone();
        self.start_job("Duplicating", JobKind::Duplicate, paths, dest);
    }
    pub(crate) fn paste(&mut self) {
        let Some((sources, cut)) = self.clip.clone() else {
            return;
        };
        if self.in_recents() {
            self.notice = Some("Open a folder to paste into it".into());
            return;
        }
        let op = if cut { Op::Move } else { Op::Copy };
        let title = if cut { "Moving" } else { "Copying" };
        let dest = self.path.clone();
        self.start_job(title, JobKind::Transfer(op), sources, dest);
    }
    pub(crate) fn new_folder(&mut self) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let name = fs::unique_name(&self.path, "New Folder");
        let name = name.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "New Folder".into());
        let dir = self.path.clone();
        match fs::new_folder(&dir, &name) {
            Ok(made) => {
                self.push_undo(Undo::NewFolder(made));
                self.navigate(dir, false)
            }
            Err(e) => self.notice = Some(format!("Could not create the folder: {}", batch::friendly(&e))),
        }
    }
    /// Opens the in-place name field for the focused item (one item only).
    pub(crate) fn start_rename(&mut self) {
        if self.in_bin() || self.selection.len() != 1 {
            return;
        }
        let Some(entry) = self.focus_entry() else {
            return;
        };
        let mut field = TextField::new();
        field.push_str(&entry.name);
        let path = entry.path.clone();
        self.naming = Some((path, field));
        self.searching = false;
        self.preview = None;
    }
    pub(crate) fn commit_rename(&mut self) {
        let Some((path, field)) = self.naming.take() else {
            return;
        };
        let name = field.to_string();
        let current = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.trim().is_empty() || name.trim() == current {
            return;
        }
        match fs::rename(&path, &name) {
            Ok(to) => {
                self.with_tags(|index| index.moved(&path, &to));
                self.push_undo(Undo::Move(vec![(path.clone(), to.clone())]));
                let dir = self.path.clone();
                self.navigate(dir, false)
            }
            Err(e) => self.notice = Some(format!("Could not rename: {}", batch::friendly(&e))),
        }
    }
    /// The name-conflict prompt of a paste, while a job waits for the answer.
    pub(crate) fn paint_prompt(&self, ui: &Ui, s: &mut Surface) {
        let w = self.w;
        if let Some(name) = self.work.as_ref().and_then(|w| w.conflict.clone()) {
            let (x, y) = (self.side() + 16., 72.);
            rect(s, ui, x, y, w - x - 16., 170., CARD_BG.now());
            text(s, ui, &format!("“{name}” already exists here"), 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
            text(s, ui, "Keep both numbers the new copy. Replace overwrites it. Skip leaves it alone.", 13., LABEL_2.now(), x + 20., y + 52., w - x - 56.);
            for (label, _, left, _) in PROMPT_BUTTONS {
                text(s, ui, label, 13., LINK, left, PROMPT_TOP, 110.);
            }
            let all = self.work.as_ref().is_some_and(|w| w.apply_all);
            text(s, ui, if all { "Apply to all: On" } else { "Apply to all: Off" }, 13., LINK, PROMPT_ALL_X, PROMPT_TOP, 130.);
            text(s, ui, "k, r, s choose; capitals apply to all. Esc cancels.", 11., LABEL_2.now(), x + 20., y + 134., w - x - 56.);
        }
    }
}
