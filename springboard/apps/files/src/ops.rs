//! File operations (copy, move, duplicate, rename, trash, new folder, compress, extract) on the dispatch queue, with progress
//! and the name-conflict prompt (Files's copy engine).
use crate::*;
use std::ops::Range;

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

/// An item to select, and perhaps rename, once the open folder has been read again: the one a rename just named, or
/// the folder New Folder just made (`Files::apply_pending`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    pub(crate) path: PathBuf,
    /// Open its name for editing as well (a new folder), rather than only select it.
    pub(crate) rename: bool,
}

/// The part of `name` a rename selects first, as byte offsets: the base name without the extension, so typing
/// replaces the name and keeps the type ("report.pdf" selects "report", "a.tar.gz" selects "a.tar"). A folder, a
/// name without an extension, a dot file (".bashrc") and a name that ends in a dot select the whole name.
pub(crate) fn rename_selection(name: &str, is_dir: bool) -> Range<usize> {
    let whole = 0..name.len();
    if is_dir {
        return whole;
    }
    match name.rfind('.') {
        // a dot at the start names a hidden file, one at the end leaves no extension
        Some(dot) if dot > 0 && dot + 1 < name.len() => 0..dot,
        _ => whole,
    }
}

/// What a background job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    /// Paste: a taken name asks the user.
    Transfer(Op),
    /// Duplicate: a taken name keeps both, without asking.
    Duplicate,
    Trash,
    /// Empty Trash: every item in Recently Deleted is deleted for good.
    Empty,
    Compress(archive::Format),
    Extract,
    /// Files another app dropped on a window: copied in from the open files, streamed, one after another.
    Receive,
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
    // the destination folder is held for writing for the whole batch, so a file open in another coordinating app
    // is not replaced under it; a folder held by another app for longer than the timeout refuses the batch
    match crate::coordinate::writing_in(dest, || batch::transfer(op, sources, dest, &mut ask, &mut tick)) {
        Ok(report) => report,
        Err(e) => crate::coordinate::refused(sources, &e),
    }
}

/// Copies one file another app handed over into `dir` under `name` (a taken name gets a number), in chunks. `tick` gets the
/// bytes copied so far and returns false to cancel. A copy that fails or is cancelled leaves nothing behind.
pub(crate) fn write_copy(dir: &Path, name: &str, src: &mut std::fs::File, tick: &mut dyn FnMut(u64) -> bool) -> std::io::Result<PathBuf> {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if base.is_empty() {
        return Err(std::io::Error::other("the file has no name"));
    }
    let target = springboard::files::fs::unique_name(dir, &base);
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
    if let Err(e) = copy_chunks(src, &mut out, tick) {
        let _ = std::fs::remove_file(&target);
        return Err(e);
    }
    Ok(target)
}

/// Streams `src` into `out` in 256 KB chunks, asking `tick` after each one; a `false` answer is an `Interrupted` error.
fn copy_chunks(src: &mut std::fs::File, out: &mut std::fs::File, tick: &mut dyn FnMut(u64) -> bool) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        out.write_all(&buf[..n])?;
        done += n as u64;
        if !tick(done) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
    }
}

/// Copies the files another app dropped into `dest` on this thread, one after another, reporting bytes as they go. A file
/// that cannot be copied is reported and the rest go on; a cancel stops the batch.
pub(crate) fn receive_now(files: Vec<(String, std::fs::File)>, dest: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    let sizes: Vec<u64> = files.iter().map(|(_, f)| f.metadata().map(|m| m.len()).unwrap_or(0)).collect();
    let mut progress = Progress { items_total: files.len(), bytes_total: sizes.iter().sum(), ..Default::default() };
    let mut last = Instant::now();
    for (n, (name, mut file)) in files.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        progress.items_done = n;
        progress.current = name.clone();
        let before = progress.bytes_done;
        let mut tick = |copied: u64| {
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                let _ = tx.send(Event::Progress(Progress { bytes_done: before + copied, ..progress.clone() }));
            }
            !cancel.load(Ordering::Relaxed)
        };
        match write_copy(dest, &name, &mut file, &mut tick) {
            Ok(made) => {
                report.done.push(made);
                progress.bytes_done = before + sizes[n];
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                report.cancelled = true;
                break;
            }
            Err(e) => {
                progress.bytes_done = before + sizes[n];
                report.failed.push((PathBuf::from(&name), batch::friendly(&e)));
            }
        }
    }
    report
}

/// Writes an archive of `sources` in `format` into `dest` on this thread.
pub(crate) fn compress_now(sources: &[PathBuf], dest: &Path, format: archive::Format, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    let mut tick = |done: u64, total: u64| {
        let _ = tx.send(Event::Progress(Progress { bytes_done: done, bytes_total: total, items_total: sources.len(), ..Default::default() }));
        !cancel.load(Ordering::Relaxed)
    };
    // the archive is written into the folder, which is held for writing while it is made
    let made = crate::coordinate::writing_in(dest, || archive::create(sources, dest, format, &mut tick));
    match made {
        Ok(Ok(out)) => report.done.push(out),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => report.cancelled = true,
        Ok(Err(e)) => report.failed.push((sources.first().cloned().unwrap_or_default(), e.to_string())),
        Err(e) => return crate::coordinate::refused(sources, &e),
    }
    report
}

/// Expands an archive into a new folder beside it on this thread.
pub(crate) fn extract_now(archive: &Path, dest: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    let mut tick = |done: u64, total: u64| {
        let _ = tx.send(Event::Progress(Progress { bytes_done: done, bytes_total: total, items_total: 1, ..Default::default() }));
        !cancel.load(Ordering::Relaxed)
    };
    // the new folder is made in `dest`, so `dest` is held for writing while the archive unpacks
    match crate::coordinate::writing_in(dest, || archive::extract(archive, dest, &mut tick)) {
        Ok(Ok(out)) => report.done.push(out.dir),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => report.cancelled = true,
        Ok(Err(e)) => report.failed.push((archive.to_path_buf(), e.to_string())),
        Err(e) => return crate::coordinate::refused(&[archive.to_path_buf()], &e),
    }
    report
}

/// Deletes each item in the Recently Deleted folder for good, then drops the origins of those that are gone.
pub(crate) fn empty_now(items: &[PathBuf], bin: &Path, cancel: &AtomicBool, tx: &Feed<Event>) -> Report {
    let mut report = Report::default();
    for (n, path) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let current = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = tx.send(Event::Progress(Progress { items_done: n, items_total: items.len(), current, ..Default::default() }));
        match crate::coordinate::writing(path, || fs::delete(path)) {
            Ok(Ok(())) => report.done.push(path.clone()),
            Ok(Err(e)) => report.failed.push((path.clone(), batch::friendly(&e))),
            Err(e) => report.failed.push((path.clone(), crate::coordinate::message(&e))),
        }
    }
    prune_origins(bin);
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
        // the item's folder is held for writing, so another coordinating app never sees it half moved
        match crate::coordinate::writing(path, || trash::discard(path, bin, trash::now_secs())) {
            Ok(Ok(stored)) => {
                if let (Some(name), Some(parent)) = (stored.file_name(), path.parent()) {
                    record_origin(bin, &name.to_string_lossy(), parent);
                }
                report.moved.push((path.clone(), stored.clone()));
                report.done.push(path.clone());
            }
            Ok(Err(e)) => report.failed.push((path.clone(), batch::friendly(&e))),
            Err(e) => report.failed.push((path.clone(), crate::coordinate::message(&e))),
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
            self.sys.effects.push(Effect::CopyToBoard(paths.clone()));
            self.clip = Some((paths, cut));
        }
    }
    pub(crate) fn start_job(&mut self, title: &str, kind: JobKind, sources: Vec<PathBuf>, dest: PathBuf) {
        let (s, d, bin) = (sources.clone(), dest.clone(), self.bin());
        self.spawn_work(title, kind, sources, dest, move |flag, tx, answers| match kind {
            JobKind::Transfer(op) => transfer_now(op, &s, &d, flag, tx, Some(answers)),
            JobKind::Duplicate => transfer_now(Op::Copy, &s, &d, flag, tx, None),
            JobKind::Trash => trash_now(&s, &bin, flag, tx),
            JobKind::Empty => empty_now(&s, &bin, flag, tx),
            JobKind::Compress(format) => compress_now(&s, &d, format, flag, tx),
            JobKind::Extract => extract_now(&s[0], &d, flag, tx),
            JobKind::Receive => Report::default(),
        });
    }

    /// Copies the files another app dropped into `dest` as a job. False (and nothing copied) when another job is running.
    pub(crate) fn start_receive(&mut self, title: &str, files: Vec<(String, std::fs::File)>, dest: PathBuf) -> bool {
        let names = files.iter().map(|(name, _)| PathBuf::from(name)).collect();
        self.spawn_work(title, JobKind::Receive, names, dest.clone(), move |flag, tx, _| receive_now(files, &dest, flag, tx))
    }

    /// Runs `run` on a utility thread as the folder's job, with the progress and the answers of a name conflict. False when a
    /// job is already running (the notice says so).
    fn spawn_work<F>(&mut self, title: &str, kind: JobKind, sources: Vec<PathBuf>, dest: PathBuf, run: F) -> bool
    where
        F: FnOnce(&AtomicBool, &Feed<Event>, &Receiver<Answer>) -> Report + Send + 'static,
    {
        if self.work.is_some() {
            self.notice = Some("Wait for the current job to finish".into());
            return false;
        }
        let (tx, rx) = feed();
        let (answers, answer_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let report = run(&flag, &tx, &answer_rx);
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
        true
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
            JobKind::Empty => "deleted",
            JobKind::Compress(_) => "compressed",
            JobKind::Extract => "extracted",
            JobKind::Receive => "copied",
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
    /// A disk image file (`.iso` or `.img`, any case): opening it mounts it read-only.
    pub(crate) fn is_disk_image(entry: &Entry) -> bool {
        !entry.is_dir && matches!(entry.path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("iso" | "img"))
    }
    pub(crate) fn is_archive(entry: &Entry) -> bool {
        !entry.is_dir && archive::is_archive(&entry.name)
    }
    /// Compress the selection into an archive of `format` in the open folder.
    pub(crate) fn compress(&mut self, format: archive::Format) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dest = self.path.clone();
        self.start_job("Compressing", JobKind::Compress(format), paths, dest);
    }
    /// Expand the focused archive into a new folder beside it.
    pub(crate) fn extract(&mut self) {
        let Some(entry) = self.focus_entry().filter(|e| Self::is_archive(e)).map(|e| e.path.clone()) else {
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
    /// Empty Trash (⇧⌘⌫): deletes every item in Recently Deleted for good, as a job. Undo does not bring them back.
    pub(crate) fn empty_trash(&mut self) {
        let bin = self.bin();
        let items: Vec<PathBuf> = std::fs::read_dir(&bin)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name() != Some(std::ffi::OsStr::new(ORIGINS)))
            .collect();
        if items.is_empty() {
            self.notice = Some("The Trash is already empty".into());
            return;
        }
        self.start_job("Emptying the Trash", JobKind::Empty, items, bin);
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
    /// Make Alias (Cmd+L): a symbolic link beside each selected item, named "name alias". Links are made on the spot.
    pub(crate) fn make_alias(&mut self) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let mut failed = None;
        for path in self.selected_paths() {
            match springboard::files::fs::make_alias(&path) {
                Ok(link) => self.push_undo(Undo::Alias(link)),
                Err(e) => failed = Some(e.to_string()),
            }
        }
        let dir = self.path.clone();
        self.navigate(dir, false);
        self.notice = failed.map(|e| format!("Could not make an alias: {e}"));
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
        if self.in_recents() {
            self.notice = Some("Open a folder to paste into it".into());
            return;
        }
        // the shared pasteboard decides what is pasted; its reply starts the job (system.rs `service_answer`)
        self.sys.effects.push(Effect::ReadBoard(self.path.clone()));
    }
    /// New Folder (the toolbar button, File > New Folder, Cmd+Shift+N): makes "New Folder" in the open folder. When the
    /// folder has been read again the new item is selected, scrolled into view and its name is open for editing with
    /// the whole name selected, so typing names it (`apply_pending`).
    pub(crate) fn new_folder(&mut self) {
        if self.in_bin() || self.in_recents() {
            return;
        }
        let name = fs::unique_name(&self.path, "New Folder");
        let name = name.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "New Folder".into());
        let dir = self.path.clone();
        match fs::new_folder(&dir, &name) {
            Ok(made) => {
                self.push_undo(Undo::NewFolder(made.clone()));
                self.navigate(dir, false);
                self.fx.pending = Some(Pending { path: made, rename: true });
            }
            Err(e) => self.notice = Some(format!("Could not create the folder: {}", batch::friendly(&e))),
        }
    }
    /// Return, Rename in the menu: opens the in-place name field for the focused item (one item only).
    pub(crate) fn start_rename(&mut self) {
        if self.in_bin() || self.selection.len() != 1 {
            return;
        }
        if let Some(i) = self.focus() {
            self.begin_rename(i);
        }
    }
    /// Opens the in-place name field for row `i` with the base name selected (see `rename_selection`), so typing
    /// replaces it, and scrolls the row into view. The one way a rename starts: Return, the menu and New Folder.
    pub(crate) fn begin_rename(&mut self, i: usize) {
        let Some(entry) = self.rows.get(i) else {
            return;
        };
        let (path, name, is_dir) = (entry.path.clone(), entry.name.clone(), entry.is_dir);
        let mut field = TextField::new();
        field.push_str(&name);
        field.select(rename_selection(&name, is_dir));
        self.naming = Some((path, field));
        self.searching = false;
        self.preview = None;
        self.reveal(i);
        self.revision += 1;
    }
    /// Once the open folder has been read again: selects the item a rename or New Folder left pending, scrolls to it
    /// and, for a new folder, opens its name for editing. An item that is not shown (hidden, or not in the folder) is
    /// dropped, and so is a pending item whatever else happens, so it never leaks into a later listing.
    pub(crate) fn apply_pending(&mut self) {
        let Some(pending) = self.fx.pending.take() else {
            return;
        };
        let Some(i) = self.rows.iter().position(|e| e.path == pending.path) else {
            return;
        };
        if !self.visible().contains(&i) {
            return;
        }
        self.select_only(i);
        self.reveal(i);
        if pending.rename {
            self.begin_rename(i);
        }
        self.revision += 1;
    }
    /// File > Open (Cmd+O, Cmd+Down): opens the focused item as a double-click does.
    pub(crate) fn open_selected(&mut self) {
        // several selected files each open in their app, as Open on a selection does in the reference
        let files: Vec<usize> = self.selection.iter().copied().filter(|&i| self.rows.get(i).is_some_and(|e| !e.is_dir)).collect();
        if self.selection.len() > 1 && files.len() == self.selection.len() {
            for i in files {
                self.open(i);
            }
            return;
        }
        if let Some(i) = self.focus() {
            self.open(i);
        }
    }
    /// Scrolls the icon or list view just far enough that row `i` (an index into `rows`) is fully on screen. The
    /// columns and gallery views do not scroll.
    pub(crate) fn reveal(&mut self, i: usize) {
        if self.columns || self.vo.gallery {
            return;
        }
        let Some(n) = self.visible().iter().position(|&j| j == i) else {
            return;
        };
        // the line's top at scroll 0, its height, and the gap the icon view leaves above its first line
        let (top, height, gap) = if self.grid {
            let cols = ((self.w - (self.side() + 16.)) / self.grid_w()).floor().max(1.) as usize;
            (self.rows_top() + 16. + (n / cols) as f32 * self.grid_h(), self.grid_h(), 16.)
        } else {
            (self.rows_top() + n as f32 * 32., 32., 0.)
        };
        let (view_top, view_bottom) = (self.rows_top(), self.bottom());
        if top - gap - self.scroll < view_top {
            self.scroll = (top - gap - view_top).max(0.);
        } else if top + height - self.scroll > view_bottom {
            self.scroll = (top + height - view_bottom).max(0.);
        }
    }
    /// Draws the rename field for the open folder's column row of `e` (columns view) and returns true, or returns
    /// false when `e` is not the item being renamed so the caller draws the row as usual.
    pub(crate) fn paint_column_name_field(&self, ui: &Ui, s: &mut Surface, e: &Entry, x0: f32, y: f32) -> bool {
        let Some(field) = self.naming.as_ref().filter(|(p, _)| *p == e.path).map(|(_, f)| f) else {
            return false;
        };
        applications::entry_icon(s, (ui.pt(x0 + 8.), ui.pt(y + 5.), ui.pt(SPACE_XL), ui.pt(SPACE_XL)), e);
        bifrost_app::widgets::text_field(s, ui, field, Rect { x: x0 + 32., y: y + 2., w: (self.vo.col_w - 40.).max(40.), h: 24. }, "", true);
        true
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
                self.navigate(dir, false);
                // the renamed item stays selected once the folder is read again
                self.fx.pending = Some(Pending { path: to, rename: false });
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

#[cfg(test)]
mod receive_tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-receive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn source(path: &Path, bytes: &[u8]) -> std::fs::File {
        std::fs::write(path, bytes).unwrap();
        std::fs::File::open(path).unwrap()
    }

    #[test]
    fn a_copy_takes_a_free_name_and_leaves_nothing_on_failure() {
        let d = dir("name");
        std::fs::write(d.join("a.txt"), b"old").unwrap();
        let mut src = source(&d.join("source.bin"), b"new bytes");
        let made = write_copy(&d, "a.txt", &mut src, &mut |_| true).unwrap();
        assert_ne!(made, d.join("a.txt"), "the existing file is kept");
        assert_eq!(std::fs::read(&made).unwrap(), b"new bytes");
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"old");
        assert!(write_copy(&d, "", &mut src, &mut |_| true).is_err(), "a nameless file is refused");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancelled_copy_removes_its_partial_file() {
        let d = dir("cancel");
        let big = vec![7u8; 600 * 1024];
        let mut src = source(&d.join("source.bin"), &big);
        let mut calls = 0;
        let mut stop_after_first = |_: u64| {
            calls += 1;
            calls < 2
        };
        let err = write_copy(&d, "big.bin", &mut src, &mut stop_after_first).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert!(!d.join("big.bin").exists(), "the partial copy is removed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_batch_copies_each_file_in_order() {
        let d = dir("batch");
        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let files = vec![
            ("one.txt".to_string(), source(&d.join("s1"), b"hello")),
            ("two.txt".to_string(), source(&d.join("s2"), b"world!!")),
        ];
        let (tx, rx) = feed();
        let cancel = AtomicBool::new(false);
        let report = receive_now(files, &out, &cancel, &tx);
        assert_eq!(report.done, vec![out.join("one.txt"), out.join("two.txt")]);
        assert!(report.failed.is_empty() && !report.cancelled);
        assert_eq!(std::fs::read(out.join("two.txt")).unwrap(), b"world!!");
        drop(rx);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancelled_batch_stops_before_the_next_file() {
        let d = dir("stop");
        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let files = vec![("one.txt".to_string(), source(&d.join("s1"), b"hello"))];
        let (tx, _rx) = feed();
        let cancel = AtomicBool::new(true);
        let report = receive_now(files, &out, &cancel, &tx);
        assert!(report.cancelled);
        assert!(report.done.is_empty());
        assert!(!out.join("one.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::Configure;

    fn test_ctx() -> Ctx {
        let font = Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(font, 1.0), cfg)
    }

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file row in the fixture folder.
    fn file(name: &str) -> Entry {
        let mut e = crate::tests::folder(name);
        e.is_dir = false;
        e.kind = Kind::Text;
        e
    }

    /// A Files open on the real folder `d`, with its rows listed.
    fn files_in(d: &Path) -> Files {
        let mut files = crate::tests::fixture(vec![]);
        files.home = d.to_path_buf();
        files.path = d.to_path_buf();
        files.listing_arrives();
        files
    }

    fn row_of(files: &Files, name: &str) -> usize {
        files.rows.iter().position(|e| e.name == name).unwrap()
    }

    impl Files {
        /// What the listing worker's final message does in `tick`: the folder's rows, nothing selected, then the
        /// pending item applied.
        fn listing_arrives(&mut self) {
            self.listing = fs::list(&self.path).unwrap();
            self.refresh_rows();
            self.selection.clear();
            self.apply_pending();
        }
    }

    #[test]
    fn a_rename_selects_the_name_without_its_extension() {
        assert_eq!(rename_selection("report.pdf", false), 0..6);
        assert_eq!(rename_selection("a.tar.gz", false), 0..5, "only the last extension stays out: a.tar");
        assert_eq!(rename_selection("Makefile", false), 0..8, "no extension: the whole name");
        assert_eq!(rename_selection(".bashrc", false), 0..7, "a dot file is all name");
        assert_eq!(rename_selection(".config.json", false), 0..7, "a dot file with an extension keeps its leading dot");
        assert_eq!(rename_selection("notes.", false), 0..6, "a trailing dot is no extension");
        assert_eq!(rename_selection("My.Folder", true), 0..9, "a folder is all name, dots or not");
        assert_eq!(rename_selection("café.txt", false), 0..5, "offsets are bytes");
        assert_eq!(rename_selection("", false), 0..0);
    }

    #[test]
    fn begin_rename_selects_the_base_name_so_typing_replaces_it() {
        let mut files = crate::tests::fixture(vec![file("report.pdf"), crate::tests::folder("My.Folder")]);
        files.select_only(0);
        files.start_rename();
        let (path, field) = files.naming.as_mut().expect("the editor is open");
        assert_eq!(*path, PathBuf::from("/fixture/report.pdf"));
        assert_eq!(field.selected_text(), "report");
        field.push_str("Reports");
        assert_eq!(field.to_string(), "Reports.pdf", "the extension stays");
        // a folder selects its whole name
        files.naming = None;
        files.select_only(1);
        files.start_rename();
        assert_eq!(files.naming.as_ref().unwrap().1.selected_text(), "My.Folder");
        // two items are not renamed
        files.naming = None;
        files.select_all();
        files.start_rename();
        assert!(files.naming.is_none());
    }

    #[test]
    fn return_renames_the_item_and_commits_while_the_editor_is_open() {
        let ctx = test_ctx();
        let mut files = crate::tests::fixture(vec![file("a.txt"), file("b.txt")]);
        files.select_only(0);
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(13))));
        assert!(files.naming.is_some(), "Return starts the rename instead of opening the item");
        assert_eq!(files.naming.as_ref().unwrap().1.selected_text(), "a");
        // Escape cancels, and the item stays selected
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(KEY_ESCAPE))));
        assert!(files.naming.is_none() && files.selection == vec![0]);
        // Return again, then Return commits (the name is unchanged, so nothing is renamed)
        files.event(&ctx, Input::Key(KeyInput::Code(13)));
        assert!(files.naming.is_some());
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(13))));
        assert!(files.naming.is_none(), "Return in the editor commits it");
    }

    #[test]
    fn return_in_the_search_field_still_opens_the_hit() {
        let ctx = test_ctx();
        let mut files = crate::tests::fixture(vec![crate::tests::folder("Alpha"), crate::tests::folder("Beta")]);
        files.select_only(1);
        files.searching = true;
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(13))));
        assert!(files.naming.is_none(), "no rename from the search field");
        assert_eq!(files.path, PathBuf::from("/fixture/Beta"), "the selected hit opens");
    }

    #[test]
    fn cmd_o_and_cmd_down_open_and_cmd_up_goes_to_the_enclosing_folder() {
        let mut files = crate::tests::fixture(vec![crate::tests::folder("Alpha"), crate::tests::folder("Beta")]);
        files.select_only(0);
        assert!(files.chord(24, CHORD_META), "Cmd+O");
        assert_eq!(files.path, PathBuf::from("/fixture/Alpha"));
        assert!(files.chord(103, CHORD_META), "Cmd+Up");
        assert_eq!(files.path, PathBuf::from("/fixture"));
        files.rows = vec![crate::tests::folder("Beta")];
        files.select_only(0);
        assert!(files.chord(108, CHORD_META), "Cmd+Down");
        assert_eq!(files.path, PathBuf::from("/fixture/Beta"));
        // nothing selected: nothing to open
        files.selection.clear();
        assert!(files.chord(24, CHORD_META));
        assert_eq!(files.path, PathBuf::from("/fixture/Beta"));
        // the editor owns the keys while it is open
        files.rows = vec![file("c.txt")];
        files.select_only(0);
        files.start_rename();
        assert!(!files.chord(24, CHORD_META), "Cmd+O is not ours while a name is typed");
    }

    #[test]
    fn a_new_folder_is_selected_and_named_in_place_once_the_folder_is_listed() {
        let ctx = test_ctx();
        let d = dir("newfolder");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        let mut files = files_in(&d);
        files.new_folder();
        let made = d.join("New Folder");
        assert!(made.is_dir());
        assert_eq!(files.fx.pending, Some(Pending { path: made.clone(), rename: true }));
        // the worker's listing arrives
        files.listing_arrives();
        assert!(files.fx.pending.is_none(), "the pending item is used once");
        assert_eq!(files.selection, vec![row_of(&files, "New Folder")], "the new folder is selected");
        let (path, field) = files.naming.as_ref().expect("its name is open for editing");
        assert_eq!(*path, made);
        assert_eq!(field.selected_text(), "New Folder", "the whole name is selected");
        // typing replaces the name and Return makes it so; the renamed folder stays selected
        files.event(&ctx, Input::Key(KeyInput::Text("Reports".into())));
        assert_eq!(files.naming.as_ref().unwrap().1.to_string(), "Reports");
        files.event(&ctx, Input::Key(KeyInput::Code(13)));
        assert!(d.join("Reports").is_dir() && !made.exists());
        assert_eq!(files.fx.pending, Some(Pending { path: d.join("Reports"), rename: false }));
        files.listing_arrives();
        assert_eq!(files.selection, vec![row_of(&files, "Reports")], "the renamed item is selected after the commit");
        assert!(files.naming.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_renamed_file_stays_selected_and_keeps_its_extension() {
        let d = dir("keepsel");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("z.txt"), "z").unwrap();
        let mut files = files_in(&d);
        let at = row_of(&files, "a.txt");
        files.select_only(at);
        files.start_rename();
        assert_eq!(files.naming.as_ref().unwrap().1.selected_text(), "a");
        files.naming.as_mut().unwrap().1.push_str("m");
        files.commit_rename();
        assert!(d.join("m.txt").exists() && !d.join("a.txt").exists());
        files.listing_arrives();
        assert_eq!(files.selection, vec![row_of(&files, "m.txt")]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pending_item_that_is_not_listed_is_dropped() {
        let mut files = crate::tests::fixture(vec![file("a.txt")]);
        files.fx.pending = Some(Pending { path: PathBuf::from("/fixture/gone"), rename: true });
        files.apply_pending();
        assert!(files.naming.is_none() && files.selection.is_empty() && files.fx.pending.is_none());
        // a hidden item is not selected either
        files.rows = vec![file(".hidden")];
        files.fx.pending = Some(Pending { path: PathBuf::from("/fixture/.hidden"), rename: false });
        files.apply_pending();
        assert!(files.selection.is_empty() && files.fx.pending.is_none());
    }

    #[test]
    fn reveal_scrolls_a_row_into_view_and_no_further() {
        let mut files = crate::tests::fixture((0..100).map(|i| file(&format!("f{i:03}"))).collect());
        files.grid = false;
        files.reveal(0);
        assert_eq!(files.scroll, 0.);
        files.reveal(60);
        let (top, bottom) = (files.rows_top() + 60. * 32., files.bottom());
        assert_eq!(top + 32. - files.scroll, bottom, "a row below the view lands on its bottom edge");
        let seen = files.scroll;
        files.reveal(55);
        assert_eq!(files.scroll, seen, "a row already in view does not move it");
        files.reveal(0);
        assert_eq!(files.scroll, 0., "a row above the view scrolls back up");
        // the icon view scrolls too; the columns view does not
        files.grid = true;
        files.reveal(99);
        assert!(files.scroll > 0.);
        files.scroll = 0.;
        files.grid = false;
        files.columns = true;
        files.reveal(99);
        assert_eq!(files.scroll, 0.);
    }
}
