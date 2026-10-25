//! Files and the desktop's system services. Space and the Preview item show the preview panel for the selection (the
//! `quick-look` service), and the arrows step it. Copy puts the selection on the shared pasteboard as file URLs and
//! names, and Paste reads the files another app put there. A document the user opens is indexed for desktop search
//! (the `CSSearchable` index). A copy or move shows its progress on the app's Dock tile.
//!
//! The code that sees a user action only queues an [`Effect`]. `SystemState::flush` sends the queue once the input or
//! tick has been handled, so the app calls a service on real events and never every frame. Each reply is matched to
//! what asked for it in `service_answer`. An older shell answers "no service", and then the app keeps its own fallback
//! (the in-app preview, or the Paste it already had).
use crate::*;
use bifrost_app::dock_tile;
use bifrost_app::pasteboard_types::{self, Board, Item, FILE_URL, STRING, URL};
use bifrost_app::proto::peek::PreviewItem;
use bifrost_app::quick_look;
use bifrost_app::quicksearch::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet};
use bifrost_app::proto::GrantId;
use bifrost_app::{Ctx, DocumentEvent};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The domain of this app's search items, so they can be deleted together.
pub(crate) const DOMAIN: &str = "is.olibuijr.files.desktop";
/// The most items one pasteboard write carries (the shell refuses more).
const MAX_BOARD_ITEMS: usize = 16;
/// The most calls waiting for a reply; older ones are dropped (their replies then find nothing to match).
const MAX_WAITING: usize = 32;
/// The most items one share menu carries (the protocol's limit).
const MAX_SHARE_ITEMS: usize = 8;
/// The smallest change of the Dock progress bar that is sent.
const DOCK_STEP: f32 = 0.01;

/// A service call the app asked for on an event, sent by `SystemState::flush`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    /// Copy: the paths go on the general pasteboard as file URLs with their names.
    CopyToBoard(Vec<PathBuf>),
    /// Paste: read the general pasteboard for files to copy into the folder.
    ReadBoard(PathBuf),
    /// Space: show the preview panel for these paths, starting at the index.
    ShowPreview(Vec<PathBuf>, usize),
    /// Space again: hide the preview panel.
    ClosePreview,
    /// A document was opened: index its name and folder for search.
    IndexDocument(PathBuf),
    /// The Dock tile's progress bar (None clears it).
    DockProgress(Option<f32>),
    /// OpenWith: ask the launch service which apps open this file.
    OpenWithApps(PathBuf),
    /// A toolbar pull-down: the menu opens at (x, y) window points, under its button.
    PopUp(f32, f32, Vec<ContextItem>),
    /// Share: the shell's share menu for these files, under the button at (x, y, w, h) window points.
    Share(Vec<PathBuf>, (f32, f32, f32, f32)),
    /// Open in the Applications place: launch the installed app (id, name), or bring it to the front
    /// (`workspace::open_application`).
    OpenApplication(String, String),
    /// Open (double-click, Cmd+O, the context menu's Open) and Open With: hand the files to the shell's workspace,
    /// which opens each in the default app for its type, or in the installed app named here
    /// (`workspace::open_file` / `open_with`, `NSWorkspace.open(_:)` parity). Files asks the shell for a read grant
    /// per file first (`Ctx::grant_path`): the workspace names files by grant, never by path.
    OpenFiles(Vec<PathBuf>, Option<String>),
}

/// The most files one Open hands to the workspace (the protocol's grant limit).
pub(crate) const MAX_OPEN_FILES: usize = bifrost_app::proto::workspace::MAX_GRANTS;
/// How long a grant made for an open is kept after the shell took the file: the shell may still need it when it
/// holds the open for the person's answer on the malware card, then it goes back.
const OPEN_GRANT_KEEP: std::time::Duration = std::time::Duration::from_secs(600);

/// The files on their way to the workspace (main thread only; the workspace answers in closures).
#[derive(Default)]
struct Opening {
    /// grant requests not yet answered: (request, the app for Open With or None for the default, file name)
    asking: Vec<(u32, Option<String>, String)>,
    /// files the shell could not open, by name, for the footer notice
    failed: Vec<String>,
    /// grants to hand back: (grant, when it may go)
    release: Vec<(GrantId, std::time::Instant)>,
}

thread_local! {
    static OPENING: std::cell::RefCell<Opening> = std::cell::RefCell::new(Opening::default());
}

/// Asks the shell for a read grant per file; `open_answer` sends the open once the grant arrives.
fn open_files(ctx: &Ctx, paths: &[PathBuf], app: Option<&str>) {
    for path in paths.iter().take(MAX_OPEN_FILES) {
        let Some(text) = path.to_str() else {
            OPENING.with(|o| o.borrow_mut().failed.push(file_name_of(path)));
            continue;
        };
        let request = ctx.grant_path(text);
        OPENING.with(|o| o.borrow_mut().asking.push((request, app.map(str::to_string), file_name_of(path))));
    }
}

/// A document answer for an open this app asked for: the grant goes to the workspace, a refusal becomes a notice.
/// `Err` hands back an answer that is not for an open (a drag's grant, a drop's files).
pub(crate) fn open_answer(ctx: &Ctx, ev: DocumentEvent) -> Result<bool, DocumentEvent> {
    let request = match &ev {
        DocumentEvent::Opened(doc) => doc.request,
        DocumentEvent::Failed { request, .. } => *request,
        _ => return Err(ev),
    };
    let Some((app, name)) = OPENING.with(|o| {
        let mut o = o.borrow_mut();
        let at = o.asking.iter().position(|(r, ..)| *r == request)?;
        let (_, app, name) = o.asking.remove(at);
        Some((app, name))
    }) else {
        return Err(ev);
    };
    let DocumentEvent::Opened(doc) = ev else {
        // the shell would not grant the file (outside the home folder, or gone)
        OPENING.with(|o| o.borrow_mut().failed.push(name));
        return Ok(true);
    };
    let grant = doc.grant;
    drop(doc.file);
    let done = move |ok: bool| {
        OPENING.with(|o| {
            let mut o = o.borrow_mut();
            if !ok {
                o.failed.push(name);
            }
            // a refused open needs the grant no more; a taken one is kept a while for a held open (malware card)
            let keep = if ok { OPEN_GRANT_KEEP } else { std::time::Duration::ZERO };
            o.release.push((grant, std::time::Instant::now() + keep));
        })
    };
    match app {
        Some(app) => {
            bifrost_app::workspace::open_with(ctx, &[grant], &app, move |status| done(status == bifrost_app::workspace::Status::Ok));
        }
        None => {
            bifrost_app::workspace::open_file(ctx, grant, done);
        }
    }
    Ok(false)
}

/// Hands back the open grants whose time is up.
fn release_open_grants(ctx: &Ctx, now: std::time::Instant) {
    let due: Vec<GrantId> = OPENING.with(|o| {
        let mut o = o.borrow_mut();
        let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut o.release).into_iter().partition(|(_, at)| *at <= now);
        o.release = keep;
        due.into_iter().map(|(g, _)| g).collect()
    });
    for grant in due {
        ctx.release_document(grant);
    }
}

/// What a worker thread hands to the main thread: a file to open (a provider's download), or a folder to show (a
/// disk image that was mounted). Opening needs the `Ctx`, which only the main thread has.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FromWorker {
    Open(PathBuf),
    Show(PathBuf),
}

static FROM_WORKERS: std::sync::Mutex<Vec<FromWorker>> = std::sync::Mutex::new(Vec::new());

/// Queues work for the main thread and wakes its loop.
pub(crate) fn from_worker(work: FromWorker) {
    FROM_WORKERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(work);
    bifrost_app::dispatch::DispatchQueue::main().async_(|| {});
}

/// The work the worker threads queued, oldest first.
pub(crate) fn take_from_workers() -> Vec<FromWorker> {
    std::mem::take(&mut *FROM_WORKERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// The name of a file the shell would not open since the last call, if any (the last one; the rest are dropped).
pub(crate) fn take_open_failure() -> Option<String> {
    OPENING.with(|o| {
        let mut o = o.borrow_mut();
        let last = o.failed.pop();
        o.failed.clear();
        last
    })
}

/// What a waiting reply is for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Waiting {
    Preview,
    Paste(PathBuf),
    /// The Open With panel's reply, for the file at this path.
    OpenWith(PathBuf),
}

/// The app's system-service state. Lives in `Files`.
#[derive(Default)]
pub(crate) struct SystemState {
    pub(crate) effects: Vec<Effect>,
    waiting: Vec<(u32, Waiting)>,
    /// The paths the shell's preview panel shows for this app, while it is open.
    pub(crate) preview: Option<Vec<PathBuf>>,
    /// The last progress fraction sent to the Dock tile; None when no bar is up.
    dock: Option<f32>,
    /// The names of the apps the shell would not open (the workspace answers in a closure; `tick` shows them).
    app_open_failed: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl SystemState {
    /// Sends the queued calls. Each call that expects a reply is remembered by its request id.
    pub(crate) fn flush(&mut self, ctx: &Ctx) {
        release_open_grants(ctx, std::time::Instant::now());
        for effect in std::mem::take(&mut self.effects) {
            match effect {
                Effect::CopyToBoard(paths) => {
                    let items: Vec<Item> = paths.iter().take(MAX_BOARD_ITEMS).map(board_item).collect();
                    Board::general().write(ctx, &items);
                }
                Effect::ReadBoard(dest) => {
                    let id = Board::general().read(ctx);
                    self.wait(id, Waiting::Paste(dest));
                }
                Effect::ShowPreview(paths, index) => {
                    let items: Vec<PreviewItem> = paths.iter().map(|p| PreviewItem { url: file_url(p), title: file_name_of(p) }).collect();
                    let id = quick_look::show(ctx, &items, index);
                    self.wait(id, Waiting::Preview);
                }
                Effect::ClosePreview => {
                    let _ = quick_look::close(ctx);
                }
                Effect::IndexDocument(path) => index_document(ctx, &path),
                Effect::DockProgress(fraction) => {
                    let _ = dock_tile::set_progress(ctx, fraction);
                }
                Effect::OpenWithApps(path) => {
                    let id = bifrost_app::launch_services::open_with(ctx, &file_name_of(&path));
                    self.wait(id, Waiting::OpenWith(path));
                }
                // the shell takes window pixels for both; the choice comes back as `Input::ContextAction`
                Effect::PopUp(x, y, items) => ctx.context_menu(x * ctx.ui.s, y * ctx.ui.s, items),
                Effect::Share(paths, (x, y, w, h)) => {
                    let items: Vec<bifrost_app::share::Payload> = paths.iter().take(MAX_SHARE_ITEMS).map(|p| bifrost_app::share::Payload::Url(file_url(p))).collect();
                    let s = ctx.ui.s;
                    let _ = bifrost_app::share::show_picker(ctx, &items, Rect { x: x * s, y: y * s, w: w * s, h: h * s });
                }
                Effect::OpenFiles(paths, app) => open_files(ctx, &paths, app.as_deref()),
                Effect::OpenApplication(id, name) => {
                    let failed = self.app_open_failed.clone();
                    bifrost_app::workspace::open_application(ctx, &id, move |status| {
                        if status != bifrost_app::workspace::Status::Ok {
                            failed.borrow_mut().push(name);
                        }
                    });
                }
            }
        }
    }

    /// The name of an app the shell would not open since the last call, if any.
    pub(crate) fn take_app_open_failure(&mut self) -> Option<String> {
        let mut failed = self.app_open_failed.borrow_mut();
        let last = failed.pop();
        failed.clear();
        last
    }

    fn wait(&mut self, request: u32, what: Waiting) {
        if self.waiting.len() >= MAX_WAITING {
            self.waiting.remove(0);
        }
        self.waiting.push((request, what));
    }

    /// Takes the waiting entry for `request`, if this app asked for it.
    pub(crate) fn answered(&mut self, request: u32) -> Option<Waiting> {
        let pos = self.waiting.iter().position(|(r, _)| *r == request)?;
        Some(self.waiting.remove(pos).1)
    }

    /// Queues the Dock progress for a running copy or move. Only a change of at least `DOCK_STEP` is sent.
    pub(crate) fn track_dock(&mut self, work: Option<&Work>) {
        let Some(work) = work else { return };
        if !matches!(work.kind, JobKind::Transfer(_) | JobKind::Duplicate | JobKind::Receive) {
            return;
        }
        let p = &work.progress;
        if p.items_total == 0 && p.bytes_total == 0 {
            return;
        }
        let fraction = p.fraction();
        if dock_step(self.dock, fraction) {
            self.dock = Some(fraction);
            self.effects.push(Effect::DockProgress(Some(fraction)));
        }
    }

    /// Clears the Dock progress bar when the job ends.
    pub(crate) fn end_dock(&mut self) {
        if self.dock.take().is_some() {
            self.effects.push(Effect::DockProgress(None));
        }
    }
}

/// Whether a Dock progress change from `last` to `now` is big enough to send.
pub(crate) fn dock_step(last: Option<f32>, now: f32) -> bool {
    match last {
        None => true,
        Some(prev) => (now - prev).abs() >= DOCK_STEP,
    }
}

/// The pasteboard item for one file: its file URL, and its name as plain text.
fn board_item(path: &PathBuf) -> Item {
    Item::default().with(FILE_URL, file_url(path).as_bytes()).with(STRING, file_name_of(path).as_bytes())
}

/// Indexes an opened document for search: its name is the title and its folder the description.
fn index_document(ctx: &Ctx, path: &Path) {
    let folder = path.parent().map(file_name_of).unwrap_or_default();
    let attributes = CSSearchableItemAttributeSet::new(&file_name_of(path)).content_description(&folder).content_url(&file_url(path));
    let item = CSSearchableItem::new(&path.to_string_lossy(), DOMAIN, attributes);
    let _ = CSSearchableIndex::default_index().index_checked(ctx, &[item]);
}

/// The last component of a path, as text.
fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A `file://` URL for an absolute path: every byte outside the unreserved set is percent-encoded.
pub(crate) fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path of a `file://` URL with an absolute path. None for other schemes, hosts or broken escapes.
pub(crate) fn path_from_file_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == b'%' {
            let hex = std::str::from_utf8(rest.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(rest[i]);
            i += 1;
        }
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(out));
    path.is_absolute().then_some(path)
}

/// The existing files that the pasteboard's items name, in order, without repeats.
pub(crate) fn board_files(items: &[Item]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for item in items {
        for rep in item.representations.iter().filter(|r| r.uti == FILE_URL || r.uti == URL) {
            let text = String::from_utf8_lossy(&rep.data);
            if let Some(path) = path_from_file_url(&text).filter(|p| p.exists()) {
                if !out.contains(&path) {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// What Paste copies or moves. When the pasteboard holds the files this app copied, the copy's own Copy or Cut applies
/// (a move stays a move). Other files on the pasteboard are copied. With no files, the app's own copy is used.
pub(crate) fn paste_plan(clip: Option<&(Vec<PathBuf>, bool)>, board: &[PathBuf]) -> Option<(Op, Vec<PathBuf>)> {
    let own = |cut: bool, paths: &[PathBuf]| (if cut { Op::Move } else { Op::Copy }, paths.to_vec());
    match clip {
        Some((paths, cut)) if board.is_empty() || same_paths(paths, board) => Some(own(*cut, paths.as_slice())),
        _ if !board.is_empty() => Some((Op::Copy, board.to_vec())),
        _ => None,
    }
}

fn same_paths(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.len() == b.len() && a.iter().all(|p| b.contains(p))
}

impl Files {
    /// Space and the Preview item: the preview panel for the selection, starting at the focused item. Pressed again
    /// while the panel is open, it closes. With nothing selected, nothing happens.
    pub(crate) fn quick_look(&mut self) {
        if self.sys.preview.take().is_some() {
            self.sys.effects.push(Effect::ClosePreview);
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        // the focus is the last selected row
        let index = paths.len() - 1;
        self.sys.preview = Some(paths.clone());
        self.sys.effects.push(Effect::ShowPreview(paths, index));
    }

    /// The panel stepped to `index`: the row for that item becomes the focus. Other selected rows stay selected.
    pub(crate) fn follow_preview(&mut self, index: usize) -> bool {
        let Some(path) = self.sys.preview.as_ref().and_then(|p| p.get(index)).cloned() else {
            return false;
        };
        let Some(row) = self.rows.iter().position(|e| e.path == path) else {
            return false;
        };
        self.selection.retain(|&i| i != row);
        self.selection.push(row);
        self.revision += 1;
        true
    }

    /// Queues the index entry for a document the user opened (not folders).
    pub(crate) fn index_opened(&mut self, e: &Entry) {
        if !e.is_dir {
            self.sys.effects.push(Effect::IndexDocument(e.path.clone()));
        }
    }

    /// Sends the queued service calls (see `SystemState::flush`).
    pub(crate) fn flush_services(&mut self, ctx: &Ctx) {
        self.sys.flush(ctx);
    }

    /// Handles a reply to a call this app made. Returns true when a frame is needed.
    pub(crate) fn service_answer(&mut self, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        let Some(what) = self.sys.answered(request) else {
            return false;
        };
        match what {
            Waiting::Preview => {
                // the shell cannot show the panel: the in-app preview takes its place
                if result.is_err() {
                    self.sys.preview = None;
                    self.peek();
                }
            }
            Waiting::Paste(dest) => {
                // an error reply reads as an empty board: the app's own copy, if any, is then pasted
                let board = match pasteboard_types::decode_read(result) {
                    Ok(snapshot) => board_files(&snapshot.items),
                    Err(_) => Vec::new(),
                };
                match paste_plan(self.clip.as_ref(), &board) {
                    Some((op, paths)) => {
                        let title = if matches!(op, Op::Move) { "Moving" } else { "Copying" };
                        self.start_job(title, JobKind::Transfer(op), paths, dest);
                    }
                    None => self.notice = Some("Nothing to paste. Copy files first.".into()),
                }
            }
            Waiting::OpenWith(path) => {
                // the panel may have closed or moved to another file while the reply was on its way
                if let Some(panel) = self.open_with.as_mut().filter(|p| p.path == path) {
                    panel.apps = Some(bifrost_app::launch_services::candidates(result));
                }
            }
        }
        self.revision += 1;
        true
    }

    /// The preview panel reports a step or a close (`quick-look` events). Returns true when a frame is needed.
    pub(crate) fn preview_event(&mut self, event: quick_look::Event) -> bool {
        match event {
            quick_look::Event::Index(i) => self.follow_preview(i),
            quick_look::Event::Closed => {
                self.sys.preview = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        let font = Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(font, 1.0), cfg)
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-system-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_urls_encode_spaces_and_non_ascii_and_come_back_as_paths() {
        let p = PathBuf::from("/home/u/Mín mappa/a b.txt");
        let url = file_url(&p);
        assert_eq!(url, "file:///home/u/M%C3%ADn%20mappa/a%20b.txt");
        assert_eq!(path_from_file_url(&url), Some(p));
    }

    #[test]
    fn other_schemes_hosts_and_broken_escapes_are_not_paths() {
        assert_eq!(path_from_file_url("https://example.com/a"), None);
        assert_eq!(path_from_file_url("file://host/share/a"), None);
        assert_eq!(path_from_file_url("file:///a%2"), None);
        assert_eq!(path_from_file_url("file:///a%zz"), None);
    }

    #[test]
    fn opening_an_application_asks_the_shell_by_id() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::OpenApplication("is.example.notes".into(), "Notes".into()));
        sys.flush(&ctx);
        match &ctx.take_sent()[..] {
            [ToShell::WorkspaceRequest { op, grants, arg, .. }] => {
                assert_eq!(*op, bifrost_app::proto::WorkspaceOp::OpenApplication);
                assert!(grants.is_empty(), "an app is named by id, not by a grant or a path");
                assert_eq!(arg, "is.example.notes");
            }
            other => panic!("expected a workspace request, got {other:?}"),
        }
        assert_eq!(sys.take_app_open_failure(), None, "no answer yet, no notice");
        sys.app_open_failed.borrow_mut().push("Notes".into());
        assert_eq!(sys.take_app_open_failure().as_deref(), Some("Notes"));
        assert_eq!(sys.take_app_open_failure(), None);
    }

    fn granted(request: u32, grant: GrantId, name: &str) -> DocumentEvent {
        let file = std::fs::File::open("/dev/null").unwrap();
        DocumentEvent::Opened(bifrost_app::Document { request, grant, name: name.into(), size: 0, mode: bifrost_app::proto::DocMode::Read, file })
    }

    fn grant_request(sent: &[ToShell]) -> (u32, String) {
        match sent {
            [ToShell::DocGrantPath { request, path }] => (*request, path.clone()),
            other => panic!("expected one grant request, got {other:?}"),
        }
    }

    #[test]
    fn open_asks_for_a_grant_then_hands_it_to_the_workspace_never_a_helper() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::OpenFiles(vec![PathBuf::from("/home/u/Pictures/a b.png")], None));
        sys.flush(&ctx);
        let (request, path) = grant_request(&ctx.take_sent());
        assert_eq!(path, "/home/u/Pictures/a b.png");
        let grant: GrantId = [7; 16];
        assert_eq!(open_answer(&ctx, granted(request, grant, "a b.png")).ok(), Some(false));
        match &ctx.take_sent()[..] {
            [ToShell::WorkspaceRequest { op, grants, arg, .. }] => {
                assert_eq!(*op, bifrost_app::proto::WorkspaceOp::OpenFile, "the shell picks the default app for the type");
                assert_eq!(grants, &vec![grant]);
                assert!(arg.is_empty());
            }
            other => panic!("expected a workspace open, got {other:?}"),
        }
        // the same request does not open twice
        assert!(open_answer(&ctx, granted(request, grant, "a b.png")).is_err());
    }

    #[test]
    fn open_with_names_the_chosen_app() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::OpenFiles(vec![PathBuf::from("/home/u/notes.txt")], Some("is.olibuijr.textedit.desktop".into())));
        sys.flush(&ctx);
        let (request, _) = grant_request(&ctx.take_sent());
        assert!(open_answer(&ctx, granted(request, [9; 16], "notes.txt")).is_ok());
        match &ctx.take_sent()[..] {
            [ToShell::WorkspaceRequest { op, arg, .. }] => {
                assert_eq!(*op, bifrost_app::proto::WorkspaceOp::OpenWith);
                assert_eq!(arg, "is.olibuijr.textedit.desktop");
            }
            other => panic!("expected a workspace open with, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_grant_is_a_notice_and_other_answers_pass_through() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        let _ = take_open_failure();
        sys.effects.push(Effect::OpenFiles(vec![PathBuf::from("/mnt/x/report.pdf")], None));
        sys.flush(&ctx);
        let (request, _) = grant_request(&ctx.take_sent());
        let refused = DocumentEvent::Failed { request, code: bifrost_app::proto::DocErrorCode::Denied };
        assert_eq!(open_answer(&ctx, refused).ok(), Some(true));
        assert!(ctx.take_sent().is_empty(), "nothing goes to the workspace without a grant");
        assert_eq!(take_open_failure().as_deref(), Some("report.pdf"));
        assert_eq!(take_open_failure(), None);
        // a grant this app did not ask for an open (a drag's) is not taken
        assert!(open_answer(&ctx, granted(request.wrapping_add(1000), [1; 16], "x")).is_err());
    }

    #[test]
    fn at_most_the_protocol_limit_of_files_is_opened_at_once() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        let paths: Vec<PathBuf> = (0..MAX_OPEN_FILES + 4).map(|i| PathBuf::from(format!("/home/u/{i}.txt"))).collect();
        sys.effects.push(Effect::OpenFiles(paths, None));
        sys.flush(&ctx);
        assert_eq!(ctx.take_sent().len(), MAX_OPEN_FILES);
    }

    #[test]
    fn workers_hand_files_and_folders_to_the_main_thread_in_order() {
        let _ = take_from_workers();
        from_worker(FromWorker::Open(PathBuf::from("/home/u/.cache/a.pdf")));
        from_worker(FromWorker::Show(PathBuf::from("/run/media/disk")));
        assert_eq!(take_from_workers(), vec![FromWorker::Open(PathBuf::from("/home/u/.cache/a.pdf")), FromWorker::Show(PathBuf::from("/run/media/disk"))]);
        assert!(take_from_workers().is_empty());
    }

    #[test]
    fn copy_writes_file_urls_and_names_to_the_general_board() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::CopyToBoard(vec![PathBuf::from("/d/a b.txt")]));
        sys.flush(&ctx);
        match &ctx.take_sent()[0] {
            ToShell::ServiceCall { service, method, body, .. } => {
                assert_eq!(service, "pasteboard");
                assert_eq!(method, "write");
                assert!(body.contains("public.file-url") && body.contains("file:///d/a%20b.txt"), "{body}");
                assert!(body.contains("a b.txt") && body.contains("public.utf8-plain-text"), "{body}");
            }
            other => panic!("expected a service call, got {other:?}"),
        }
    }

    #[test]
    fn show_preview_calls_the_panel_and_remembers_the_reply() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::ShowPreview(vec![PathBuf::from("/d/a.png"), PathBuf::from("/d/b.txt")], 1));
        sys.flush(&ctx);
        let request = match &ctx.take_sent()[0] {
            ToShell::ServiceCall { request, service, method, body } => {
                assert_eq!(service, "quick-look");
                assert_eq!(method, "show");
                assert!(body.contains("file:///d/b.txt") && body.contains("\"index\""), "{body}");
                *request
            }
            other => panic!("expected a service call, got {other:?}"),
        };
        assert_eq!(sys.answered(request), Some(Waiting::Preview));
        assert_eq!(sys.answered(request), None, "a reply is taken once");
    }

    /// The service calls to `quick-look` in `sent`, as (method, body).
    fn panel_calls(sent: Vec<ToShell>) -> Vec<(String, String)> {
        sent.into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } if service == "quick-look" => Some((method, body)),
                _ => None,
            })
            .collect()
    }

    fn two_files() -> Files {
        let file = |name: &str| {
            let mut e = crate::tests::folder(name);
            e.is_dir = false;
            e
        };
        crate::tests::fixture(vec![file("a.png"), file("b.txt")])
    }

    #[test]
    fn space_with_a_selection_asks_the_shell_for_the_panel_and_again_closes_it() {
        let ctx = test_ctx();
        let mut files = two_files();
        files.select_only(1);
        assert!(files.event(&ctx, Input::Key(KeyInput::Text(" ".into()))), "Space is taken");
        let calls = panel_calls(ctx.take_sent());
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].0, "show");
        assert!(calls[0].1.contains("file:///fixture/b.txt"), "the selected file: {}", calls[0].1);
        assert!(files.sys.preview.is_some(), "the panel is remembered as open");
        // Space again closes it
        assert!(files.event(&ctx, Input::Key(KeyInput::Text(" ".into()))));
        let calls = panel_calls(ctx.take_sent());
        assert_eq!(calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["close"]);
        assert!(files.sys.preview.is_none());
    }

    #[test]
    fn the_sdk_escape_code_cancels_a_rename_and_clears_the_search() {
        use bifrost_app::proto::KEY_ESCAPE;
        let ctx = test_ctx();
        let mut files = two_files();
        files.naming = Some((PathBuf::from("/fixture/a.png"), crate::TextField::new()));
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(KEY_ESCAPE))), "Escape is taken");
        assert!(files.naming.is_none(), "Escape cancels the rename");
        files.searching = true;
        files.query.push_str("ab");
        assert!(files.event(&ctx, Input::Key(KeyInput::Code(KEY_ESCAPE))));
        assert!(!files.searching && files.query.to_string().is_empty(), "Escape ends the search");
    }

    #[test]
    fn space_with_nothing_selected_or_in_a_text_field_asks_for_no_panel() {
        let ctx = test_ctx();
        let mut files = two_files();
        files.event(&ctx, Input::Key(KeyInput::Text(" ".into())));
        assert!(panel_calls(ctx.take_sent()).is_empty(), "nothing selected");
        files.select_only(0);
        files.searching = true;
        files.event(&ctx, Input::Key(KeyInput::Text(" ".into())));
        assert!(panel_calls(ctx.take_sent()).is_empty(), "the Search field takes the space");
        assert_eq!(files.query.to_string(), " ");
    }

    #[test]
    fn the_panel_stepping_moves_the_selection_and_closing_forgets_it() {
        let ctx = test_ctx();
        let mut files = two_files();
        files.selection = vec![0, 1];
        files.event(&ctx, Input::Key(KeyInput::Text(" ".into())));
        let _ = ctx.take_sent();
        assert!(files.preview_event(quick_look::Event::Index(0)));
        assert_eq!(files.selection.last(), Some(&0), "the focus follows the panel");
        files.preview_event(quick_look::Event::Closed);
        assert!(files.sys.preview.is_none());
    }

    #[test]
    fn a_closed_preview_sends_one_close_call() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::ClosePreview);
        sys.flush(&ctx);
        match &ctx.take_sent()[0] {
            ToShell::ServiceCall { service, method, .. } => {
                assert_eq!((service.as_str(), method.as_str()), ("quick-look", "close"));
            }
            other => panic!("expected a service call, got {other:?}"),
        }
    }

    #[test]
    fn an_opened_document_is_indexed_under_the_app_domain() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::IndexDocument(PathBuf::from("/d/notes.txt")));
        sys.flush(&ctx);
        match &ctx.take_sent()[0] {
            ToShell::QuickSearchIndex(items) => {
                assert_eq!(items[0].unique_id, "/d/notes.txt");
                assert_eq!(items[0].domain, DOMAIN);
                assert_eq!(items[0].title, "notes.txt");
                assert_eq!(items[0].content_url, "file:///d/notes.txt");
            }
            other => panic!("expected an index message, got {other:?}"),
        }
    }

    #[test]
    fn dock_progress_is_sent_on_a_real_change_and_cleared_at_the_end() {
        let ctx = test_ctx();
        let mut sys = SystemState::default();
        sys.effects.push(Effect::DockProgress(Some(0.5)));
        sys.flush(&ctx);
        match &ctx.take_sent()[0] {
            ToShell::ServiceCall { service, method, body, .. } => {
                assert_eq!((service.as_str(), method.as_str()), ("dock", "progress"));
                assert!(body.contains("0.5"), "{body}");
            }
            other => panic!("expected a service call, got {other:?}"),
        }
        sys.effects.push(Effect::DockProgress(None));
        sys.flush(&ctx);
        assert_eq!(ctx.take_sent().len(), 1);
    }

    #[test]
    fn dock_steps_are_throttled_to_one_percent() {
        assert!(dock_step(None, 0.0));
        assert!(!dock_step(Some(0.5), 0.505));
        assert!(dock_step(Some(0.5), 0.52));
    }

    #[test]
    fn paste_takes_our_own_copy_when_the_board_still_holds_it() {
        let ours = (vec![PathBuf::from("/a/x"), PathBuf::from("/a/y")], true);
        let plan = paste_plan(Some(&ours), &[PathBuf::from("/a/y"), PathBuf::from("/a/x")]);
        assert_eq!(plan, Some((Op::Move, vec![PathBuf::from("/a/x"), PathBuf::from("/a/y")])));
    }

    #[test]
    fn paste_copies_files_another_app_put_on_the_board() {
        let ours = (vec![PathBuf::from("/a/x")], false);
        let plan = paste_plan(Some(&ours), &[PathBuf::from("/b/z")]);
        assert_eq!(plan, Some((Op::Copy, vec![PathBuf::from("/b/z")])));
        assert_eq!(paste_plan(None, &[PathBuf::from("/b/z")]), Some((Op::Copy, vec![PathBuf::from("/b/z")])));
    }

    #[test]
    fn paste_falls_back_to_the_own_copy_and_otherwise_does_nothing() {
        let ours = (vec![PathBuf::from("/a/x")], false);
        assert_eq!(paste_plan(Some(&ours), &[]), Some((Op::Copy, vec![PathBuf::from("/a/x")])));
        assert_eq!(paste_plan(None, &[]), None);
    }

    #[test]
    fn board_files_keeps_existing_file_urls_once() {
        let dir = temp("board");
        let file = dir.join("note.txt");
        std::fs::write(&file, "hi").unwrap();
        let item = Item::default()
            .with(FILE_URL, file_url(&file).as_bytes())
            .with(URL, file_url(&file).as_bytes())
            .with(STRING, b"note.txt");
        let text = Item::default().with(STRING, b"https://example.com");
        let missing = Item::default().with(FILE_URL, file_url(&dir.join("gone.txt")).as_bytes());
        assert_eq!(board_files(&[item, text, missing]), vec![file]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
