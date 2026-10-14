//! The PDF worker. Each open PDF gets its own thread, and that thread owns the parsed file: the engine's objects are
//! reference counted, so they never leave it. The window sends jobs (draw a page, save an edited copy). A job runs on
//! the worker in order and returns a `Reply`, which goes into the document's inbox; the delivery runs on the app loop
//! and wakes it, so `App::tick` can take the reply and the window can draw it.

use crate::find::Hit;
use bifrost_app::dispatch::DispatchQueue;
use bifrost_app::kit::Image;
use crate::inspector;
use springboard::cg::pdf::doc::Doc;
use springboard::cg::pdf::obj::Obj;
use springboard::cg::pdf::draw;
use springboard::cg::pdf::page::{self, Page as PdfPage};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// Pages are fitted into this box (width, height) for the window; thumbnails are fitted into smaller boxes.
pub const PAGE_BOX: (usize, usize) = (1000, 1400);

/// A picture of one page the window asked for: the source page, the box it is fitted into, and whether it is a
/// sidebar thumbnail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub thumb: bool,
    pub page: usize,
    pub w: usize,
    pub h: usize,
}

/// One outline entry: its title, its depth and the source page it goes to, when it names one.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub title: String,
    pub level: u8,
    pub page: Option<usize>,
}

/// What the worker reports once the file is open: each page's size before rotation (fitted into `PAGE_BOX`) and the
/// outline.
pub struct Opened {
    pub sizes: Vec<(usize, usize)>,
    pub outline: Vec<Outline>,
    /// The document information the Inspector lists, as label and value.
    pub info: Vec<(&'static str, String)>,
    /// Whether the file is encrypted.
    pub encrypted: bool,
}

/// A result from the worker.
pub enum Reply {
    Opened(Result<Opened, String>),
    /// A page picture or thumbnail. None when the page could not be drawn.
    Rendered(Key, Option<Image>),
    /// A save or an export: the file written, or why it was not.
    Written(Result<PathBuf, String>),
    /// A print: the status line to show, which says whether the job was sent.
    Printed(String),
    /// A search: the text searched for and the lines that hold it, in page order.
    Found(String, Vec<Hit>),
    /// The text copied from a page: its lines, one to a line of the string.
    Copied(String),
}

/// A job for the worker: it runs on the worker thread with the parsed file and returns its reply.
pub type Job = Box<dyn FnOnce(&Pdf) -> Reply + Send>;

/// Replies waiting for the window. The worker fills it and the window empties it in `tick`.
pub type Inbox = Arc<Mutex<VecDeque<Reply>>>;

/// The parsed file, owned by the worker thread.
pub struct Pdf {
    pub doc: Doc,
    pub pages: Vec<PdfPage>,
}

impl Pdf {
    fn open(path: &Path, password: &[u8]) -> Result<Pdf, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let doc = Doc::open_with_password(data, password)?;
        let pages = page::pages(&doc);
        if pages.is_empty() {
            return Err("The document has no pages.".into());
        }
        Ok(Pdf { doc, pages })
    }

    fn summary(&self) -> Result<Opened, String> {
        let mut sizes = Vec::with_capacity(self.pages.len());
        for p in &self.pages {
            sizes.push(draw::fit_size(p, PAGE_BOX.0, PAGE_BOX.1).ok_or("A page has no size.")?);
        }
        let outline = page::outline(&self.doc)
            .into_iter()
            .map(|item| Outline {
                title: item.title,
                level: item.level,
                page: self.pages.iter().position(|p| p.num == Some(item.dest.page)),
            })
            .collect();
        Ok(Opened { sizes, outline, info: self.info(), encrypted: self.doc.is_encrypted() })
    }

    /// The title, author, subject and keywords the document information dictionary holds, when they are text.
    fn info(&self) -> Vec<(&'static str, String)> {
        let Some(Obj::Dict(info)) = self.doc.trailer.get("Info").map(|o| self.doc.get(o)) else { return Vec::new() };
        [("Title", "Title"), ("Author", "Author"), ("Subject", "Subject"), ("Keywords", "Keywords")]
            .into_iter()
            .filter_map(|(key, label)| match info.get(key) {
                Some(Obj::Str(bytes)) => Some((label, inspector::pdf_text(bytes))).filter(|(_, text)| !text.trim().is_empty()),
                _ => None,
            })
            .collect()
    }
}

/// Whether an open error means the file needs a user password (none was given, or the one given is wrong).
pub fn needs_password(error: &str) -> bool {
    error == springboard::cg::pdf::crypt::PASSWORD_NEEDED
}

/// Starts the worker for the PDF at `path`, opened with `password` (empty for a file without one). It opens the file and
/// reports it, then runs the jobs sent on the returned sender in order. Results collect in the returned inbox.
pub fn spawn(path: PathBuf, password: Vec<u8>) -> (Sender<Job>, Inbox) {
    let inbox: Inbox = Arc::default();
    let (tx, rx) = channel::<Job>();
    let sink = Arc::clone(&inbox);
    thread::spawn(move || match Pdf::open(&path, &password) {
        Ok(pdf) => {
            deliver(&sink, Reply::Opened(pdf.summary()));
            for job in rx {
                let reply = job(&pdf);
                deliver(&sink, reply);
            }
        }
        Err(e) => deliver(&sink, Reply::Opened(Err(e))),
    });
    (tx, inbox)
}

/// Queues `reply` on the app loop. The push runs there, so the loop wakes for it.
fn deliver(sink: &Inbox, reply: Reply) {
    let sink = Arc::clone(sink);
    DispatchQueue::main().async_(move || {
        sink.lock().unwrap_or_else(|e| e.into_inner()).push_back(reply);
    });
}

/// Takes every reply waiting in `inbox`, oldest first.
pub fn take(inbox: &Inbox) -> Vec<Reply> {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
}

/// A job that draws the page or thumbnail `key` names, fitted into its box.
pub fn render(key: Key) -> Job {
    Box::new(move |pdf: &Pdf| {
        let img = pdf.pages.get(key.page).and_then(|p| draw::render_page_fit(&pdf.doc, p, key.w, key.h));
        Reply::Rendered(key, img)
    })
}

/// A job that runs `f` on the worker and replies with what it returns.
pub fn run<F>(f: F) -> Job
where
    F: FnOnce(&Pdf) -> Reply + Send + 'static,
{
    Box::new(f)
}
