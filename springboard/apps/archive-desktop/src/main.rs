//! Archive as an installed BifrOSt app in the full-size window. Opening an archive (Files, Open With, or
//! double-click when it is the default) expands it beside itself; opening a folder compresses it to a .zip beside
//! it. Jobs run one at a time on a worker thread, and the window shows the running job's progress and a history
//! of finished ones. The logic is in `archive.rs`.

mod archive;

use archive::{Op, Outcome};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, Ctx, Input};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Layout in points: the window padding and the row height of the history list.
const PAD_PT: f32 = 16.0;
const ROW_PT: f32 = 22.0;
const HEADER_PT: f32 = 56.0;
const BAR_PT: f32 = 8.0;
/// Finished jobs kept in the history list.
const HISTORY: usize = 12;
/// How often the window polls the worker while a job runs, in ms.
const POLL_MS: f64 = 100.0;

struct Request {
    op: Op,
    path: PathBuf,
}

/// A job on the worker thread: its progress counters and the result once it finishes.
struct Running {
    req: Request,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    result: Arc<Mutex<Option<Result<Outcome, String>>>>,
}

#[derive(Default)]
struct ArchiveApp {
    queue: VecDeque<Request>,
    running: Option<Running>,
    history: Vec<String>,
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// The request an opened path asks for, or the reason it cannot be handled.
fn request_for(path: &Path) -> Result<Request, String> {
    if path.is_dir() {
        return Ok(Request { op: Op::Compress, path: path.to_path_buf() });
    }
    match springboard::files::archive::is_archive(&name_of(path)) {
        true => Ok(Request { op: Op::Expand, path: path.to_path_buf() }),
        false => Err(format!("{} is not an archive or a folder.", name_of(path))),
    }
}

/// The line the history shows for a finished job.
fn describe(req: &Request, result: &Result<Outcome, String>) -> String {
    let name = name_of(&req.path);
    match (req.op, result) {
        (Op::Expand, Ok(o)) if o.skipped > 0 => format!("Expanded {name} to {} ({} files, {} skipped).", name_of(&o.path), o.files, o.skipped),
        (Op::Expand, Ok(o)) => format!("Expanded {name} to {} ({} files).", name_of(&o.path), o.files),
        (Op::Compress, Ok(o)) => format!("Compressed {name} to {}.", name_of(&o.path)),
        (_, Err(e)) => format!("Could not handle {name}: {e}"),
    }
}

impl ArchiveApp {
    fn enqueue(&mut self, req: Request) {
        self.queue.push_back(req);
        self.start_next();
    }

    /// Starts the next queued job when the worker is idle.
    fn start_next(&mut self) {
        if self.running.is_some() {
            return;
        }
        let Some(req) = self.queue.pop_front() else { return };
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let result = Arc::new(Mutex::new(None));
        let (d, t, r) = (done.clone(), total.clone(), result.clone());
        let (op, path) = (req.op, req.path.clone());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let mut progress = |a: u64, b: u64| {
                d.store(a, Ordering::Relaxed);
                t.store(b, Ordering::Relaxed);
                true
            };
            let out = archive::run(op, &path, &mut progress).map_err(|e| e.to_string());
            *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
        });
        self.running = Some(Running { req, done, total, result });
    }

    /// Collects a finished job and starts the next one. True when the window needs a redraw.
    fn poll(&mut self) -> bool {
        let finished = match &self.running {
            Some(run) => run.result.lock().unwrap_or_else(|e| e.into_inner()).take(),
            None => return false,
        };
        let Some(result) = finished else { return false };
        if let Some(run) = self.running.take() {
            let line = describe(&run.req, &result);
            self.history.insert(0, line);
            self.history.truncate(HISTORY);
        }
        self.start_next();
        true
    }

    /// Progress of the running job, 0.0 to 1.0 (0 while the total is not known).
    fn fraction(&self) -> f32 {
        let Some(run) = &self.running else { return 0.0 };
        let (d, t) = (run.done.load(Ordering::Relaxed), run.total.load(Ordering::Relaxed));
        if t == 0 { 0.0 } else { (d as f32 / t as f32).clamp(0.0, 1.0) }
    }
}

impl App for ArchiveApp {
    fn id(&self) -> &str {
        "is.olibuijr.archive.desktop"
    }

    fn event(&mut self, _ctx: &Ctx, ev: Input) -> bool {
        let Input::Open(path) = ev else { return false };
        match request_for(Path::new(&path)) {
            Ok(req) => self.enqueue(req),
            Err(e) => {
                self.history.insert(0, e);
                self.history.truncate(HISTORY);
            }
        }
        true
    }

    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        self.poll() || self.running.is_some()
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.running.as_ref().map(|_| t + POLL_MS)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let scale = ui.s;
        let p = |v: f32| (v * scale) as usize;
        // positions for blit_colored, which takes signed pixels
        let pi = |v: f32| (v * scale) as i32;
        let (w, h) = (s.w, s.h);
        let width_pt = w as f32 / scale - 2.0 * PAD_PT;
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());

        let title = ui.text("Archive", ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&title, pi(PAD_PT), pi(PAD_PT), ui::LABEL.now(), 255);
        let hint = "Open an archive to expand it, or a folder to compress it to .zip.";
        let img = ui.text(&ui.fit(hint, ui::MAC_FOOTNOTE_PT, ui.pt(width_pt)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
        s.blit_colored(&img, pi(PAD_PT), pi(PAD_PT + 26.0), ui::LABEL_2.now(), 255);

        let mut y = HEADER_PT;
        if let Some(run) = &self.running {
            let verb = if run.req.op == Op::Expand { "Expanding" } else { "Compressing" };
            let line = format!("{verb} {}", name_of(&run.req.path));
            let img = ui.text(&ui.fit(&line, ui::MAC_BODY_PT, ui.pt(width_pt)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, pi(PAD_PT), pi(y), ui::LABEL.now(), 255);
            y += ROW_PT;
            let bar_w = (width_pt * scale) as usize;
            s.fill_rect(p(PAD_PT), p(y), bar_w, p(BAR_PT), ui::SEPARATOR);
            let fill = (bar_w as f32 * self.fraction()) as usize;
            s.fill_rect(p(PAD_PT), p(y), fill, p(BAR_PT), ui::LINK);
            y += BAR_PT + 10.0;
        }
        if !self.queue.is_empty() {
            let line = format!("{} waiting", self.queue.len());
            let img = ui.text(&line, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, pi(PAD_PT), pi(y), ui::LABEL_2.now(), 255);
            y += ROW_PT;
        }
        if self.history.is_empty() && self.running.is_none() {
            let img = ui.text("Nothing has been done yet.", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, pi(PAD_PT), pi(y), ui::LABEL_2.now(), 255);
        }
        for line in &self.history {
            if y > h as f32 / scale {
                break;
            }
            let img = ui.text(&ui.fit(line, ui::MAC_BODY_PT, ui.pt(width_pt)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, pi(PAD_PT), pi(y), ui::LABEL.now(), 255);
            y += ROW_PT;
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ArchiveApp::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_compress_and_archives_expand() {
        let dir = std::env::temp_dir();
        assert!(matches!(request_for(&dir), Ok(Request { op: Op::Compress, .. })));
        assert!(matches!(request_for(Path::new("/x/photos.zip")), Ok(Request { op: Op::Expand, .. })));
        assert!(matches!(request_for(Path::new("/x/photos.tar.gz")), Ok(Request { op: Op::Expand, .. })));
        assert!(request_for(Path::new("/x/notes.txt")).is_err());
    }

    #[test]
    fn history_lines_name_the_outcome() {
        let req = Request { op: Op::Expand, path: PathBuf::from("/a/b/data.zip") };
        let ok = Ok(Outcome { path: PathBuf::from("/a/b/data 2"), files: 3, skipped: 1 });
        assert_eq!(describe(&req, &ok), "Expanded data.zip to data 2 (3 files, 1 skipped).");
        let err: Result<Outcome, String> = Err("Damaged ZIP directory".into());
        assert_eq!(describe(&req, &err), "Could not handle data.zip: Damaged ZIP directory");
    }
}
