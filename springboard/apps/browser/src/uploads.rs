//! Native file inputs use the document portal. Only files explicitly granted by the user are
//! streamed to private staging files, because Servo 0.7's FilePicker accepts paths rather than FDs.
//! Staging survives while the document can reference its File objects; navigation and close remove it.
use std::collections::HashMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Seek};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use bifrost_app::{Ctx, Document, DocumentEvent, OpenOptions as PanelOptions};
use bifrost_app::webview::{WebProcessPool, WebViewEvent, WebViewId};

use crate::inbox;

const MAX_FILES: usize = 64;
const MAX_BATCH_BYTES: u64 = 1 << 30;
const MAX_RETAINED_BYTES: u64 = 2 << 30;

struct Staged {
    directory: PathBuf,
    paths: Vec<PathBuf>,
    bytes: u64,
}

impl Drop for Staged {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.directory); }
}

struct Request {
    view: WebViewId,
    token: u32,
    document: u32,
    multiple: bool,
}

struct Copying {
    request: Request,
    result: inbox::Slot<Result<Staged, String>>,
}

#[derive(Default)]
pub(super) struct Uploads {
    pending: Option<Request>,
    copying: Option<Copying>,
    retained: HashMap<WebViewId, Vec<Staged>>,
    /// Private copies of the handed files (a double click in Files, Open With) the browser cannot read where they are.
    /// A tab may load its file again at any time, so they stay until the window closes.
    opened: Vec<Staged>,
    /// What each private copy is shown as: the copy's `file:` URL and the address of the file it was made from.
    copies: Vec<(String, String)>,
    /// The `file:` URL of the folder the private copies are under, ending in a slash.
    private: Option<String>,
    /// `file:` URLs of handed files that no tab shows yet (`take_handed`).
    handed: Vec<String>,
    pub(super) error: Option<String>,
}

impl Uploads {
    pub(super) fn active(&self) -> bool { self.pending.is_some() || self.copying.is_some() }

    pub(super) fn copying(&self) -> bool { self.copying.is_some() }

    fn bytes(&self) -> u64 { self.retained.values().flatten().map(|batch| batch.bytes).sum() }

    pub(super) fn on_event(&mut self, event: &WebViewEvent, ctx: &Ctx, pool: &mut Option<WebProcessPool>, visible: Option<WebViewId>) {
        match event {
            WebViewEvent::FilePicker { id, token, multiple, filters } => {
                self.error = None;
                if self.active() || visible != Some(*id) || self.bytes() >= MAX_RETAINED_BYTES {
                    if self.bytes() >= MAX_RETAINED_BYTES { self.error = Some("Close pages with selected files before choosing more files (2 GiB window limit).".into()); }
                    if let Some(pool) = pool { pool.file_picker_reply(*id, *token, None); }
                    return;
                }
                let origin = pool.as_ref().and_then(|pool| pool.state(*id))
                    .and_then(|state| url::Url::parse(&state.url).ok())
                    .map(|url| url.origin().ascii_serialization()).unwrap_or_else(|| "this page".into());
                let document = ctx.open_panel(&PanelOptions {
                    title: format!("Choose files for {origin}"),
                    filters: filters.clone(),
                    multiple: *multiple,
                    ..Default::default()
                });
                self.pending = Some(Request { view: *id, token: *token, document, multiple: *multiple });
            }
            WebViewEvent::ControlHidden(view, token) => {
                if self.pending.as_ref().is_some_and(|request| request.view == *view && request.token == *token) { self.pending = None; }
                if self.copying.as_ref().is_some_and(|copy| copy.request.view == *view && copy.request.token == *token) { self.copying = None; }
            }
            WebViewEvent::LoadCommitted(view) | WebViewEvent::Closed(view) | WebViewEvent::ContentProcessTerminated(view, _) => self.forget(*view, pool),
            // the picker requests die with the engine; the handed files are still on disk for the tabs that come back
            WebViewEvent::EngineExited => self.reset(),
            _ => {}
        }
    }

    /// Forgets the file choosers of an engine that is gone. The handed files stay: their tabs come back and load them
    /// again, and the private copies keep the address they are shown under.
    pub(super) fn reset(&mut self) {
        *self = Self {
            opened: std::mem::take(&mut self.opened),
            copies: std::mem::take(&mut self.copies),
            private: self.private.take(),
            handed: std::mem::take(&mut self.handed),
            ..Self::default()
        };
    }

    pub(super) fn cancel_pending(&mut self, pool: &mut Option<WebProcessPool>) {
        if let Some(request) = self.pending.take() { Self::cancel(pool, &request); }
        if let Some(copy) = self.copying.take() { Self::cancel(pool, &copy.request); }
    }

    fn cancel(pool: &mut Option<WebProcessPool>, request: &Request) {
        if let Some(pool) = pool { pool.file_picker_reply(request.view, request.token, None); }
    }

    pub(super) fn forget(&mut self, view: WebViewId, pool: &mut Option<WebProcessPool>) {
        self.error = None;
        if self.pending.as_ref().is_some_and(|request| request.view == view) {
            if let Some(request) = self.pending.take() { Self::cancel(pool, &request); }
        }
        if self.copying.as_ref().is_some_and(|copy| copy.request.view == view) {
            if let Some(copy) = self.copying.take() { Self::cancel(pool, &copy.request); }
        }
        self.retained.remove(&view);
    }

    pub(super) fn document(&mut self, ctx: &Ctx, event: DocumentEvent, pool: &mut Option<WebProcessPool>) -> bool {
        let (document, docs) = match event {
            DocumentEvent::Opened(doc) => (doc.request, vec![doc]),
            DocumentEvent::OpenedMany { request, docs } => (request, docs),
            DocumentEvent::Failed { request, code } => {
                if self.pending.as_ref().is_some_and(|pending| pending.document == request) {
                    let pending = self.pending.take().unwrap();
                    Self::cancel(pool, &pending);
                    if code != bifrost_app::proto::DocErrorCode::Cancelled { self.error = Some("The file chooser is unavailable.".into()); }
                    return true;
                }
                return false;
            }
            _ => return false,
        };
        // The open descriptors remain valid after releasing their reusable grants.
        for doc in &docs { ctx.release_document(doc.grant); }
        if !self.pending.as_ref().is_some_and(|request| request.document == document) {
            // not an answer to a file chooser: a file somebody handed the browser is opened in a tab
            if !bifrost_app::share::is_inbound(document) || docs.is_empty() { return false; }
            self.hand_over(&ctx.data_dir.join("opened-files"), docs);
            return true;
        }
        let request = self.pending.take().unwrap();
        if docs.is_empty() || docs.len() > MAX_FILES || (!request.multiple && docs.len() != 1) {
            Self::cancel(pool, &request);
            if docs.len() > MAX_FILES { self.error = Some("Choose at most 64 files at a time.".into()); }
            return true;
        }
        let base = ctx.data_dir.join("upload-staging");
        let budget = MAX_BATCH_BYTES.min(MAX_RETAINED_BYTES.saturating_sub(self.bytes()));
        self.copying = Some(Copying { request, result: inbox::spawn(move || stage(&base, docs, budget).map_err(|_| "Could not prepare the selected files (limit: 1 GiB per selection, 2 GiB per window).".into())) });
        true
    }

    /// Queues a `file:` URL for each handed file. A file the browser can read where it is (the account's home is part
    /// of its sandbox) is loaded from there, so the address is the file's own and the pictures and style sheets beside
    /// it load. Any other file reaches the browser only as an open descriptor, and the engine loads paths: it is
    /// copied into a private folder under `base` and shown under the address it came from (`shown`).
    fn hand_over(&mut self, base: &Path, docs: Vec<Document>) {
        let mut apart = Vec::new();
        for doc in docs {
            match readable_in_place(&doc.file).and_then(|path| url::Url::from_file_path(path).ok()) {
                Some(url) => self.handed.push(url.into()),
                None => apart.push(doc),
            }
        }
        if apart.is_empty() { return; }
        let shown: Vec<String> = apart.iter().map(|doc| shown_address(opened_from(&doc.file).as_deref(), &doc.name)).collect();
        match stage(base, apart, MAX_BATCH_BYTES) {
            Ok(batch) => {
                self.private = url::Url::from_directory_path(base).ok().map(String::from);
                for (path, shown) in batch.paths.iter().zip(shown) {
                    let Some(url) = url::Url::from_file_path(path).ok().map(String::from) else { continue };
                    self.copies.push((url.clone(), shown));
                    self.handed.push(url);
                }
                self.opened.push(batch);
            }
            Err(_) => self.error = Some("The file could not be opened.".into()),
        }
    }

    /// The address to show for the page at `url` when it is one of the private copies: the place the file was opened
    /// from, or its name when that is not known. Only the exact URL of a copy this window made is mapped (a query or
    /// fragment is carried over). Anything else in the private folder (a link to a file that was beside the original)
    /// is shown by its name alone, never as the folder. `None` for every other page.
    pub(super) fn shown(&self, url: &str) -> Option<String> {
        let (page, rest) = split_suffix(url);
        if let Some((_, shown)) = self.copies.iter().rev().find(|(copy, _)| copy == page) {
            return Some(format!("{shown}{rest}"));
        }
        let private = self.private.as_deref()?;
        if !page.starts_with(private) { return None; }
        let name = url::Url::parse(page).ok().and_then(|u| u.to_file_path().ok()).and_then(|p| Some(p.file_name()?.to_string_lossy().into_owned()));
        Some(name.unwrap_or_default())
    }

    /// The private copy an address typed or pasted into the address field names: the reverse of `shown`, by exact
    /// match, so Return on the shown address loads the same document again.
    pub(super) fn copy_for(&self, address: &str) -> Option<String> {
        let (page, rest) = split_suffix(address.trim());
        self.copies.iter().rev().find(|(_, shown)| shown == page).map(|(copy, _)| format!("{copy}{rest}"))
    }

    /// The URLs of the handed files that are ready, each to be shown in a new tab.
    pub(super) fn take_handed(&mut self) -> Vec<String> { std::mem::take(&mut self.handed) }

    pub(super) fn tick(&mut self, pool: &mut Option<WebProcessPool>) -> bool {
        let Some(result) = self.copying.as_ref().and_then(|copy| inbox::take(&copy.result)) else { return false };
        let copy = self.copying.take().unwrap();
        match result {
            Ok(batch) if pool.as_ref().is_some_and(|pool| pool.state(copy.request.view).is_some()) => {
                if let Some(pool) = pool { pool.file_picker_reply(copy.request.view, copy.request.token, Some(&batch.paths)); }
                self.retained.entry(copy.request.view).or_default().push(batch);
            }
            Ok(_) => {}
            Err(message) => { Self::cancel(pool, &copy.request); self.error = Some(message); }
        }
        true
    }
}

/// `url` without its query and fragment, and that rest.
fn split_suffix(url: &str) -> (&str, &str) {
    url.split_at(url.find(['?', '#']).unwrap_or(url.len()))
}

/// The path a handed descriptor was opened from, as the kernel reports it (`/proc/self/fd`). It follows a rename; a
/// removed file has none that names it.
fn opened_from(file: &fs::File) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;
    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok().filter(|path| path.is_absolute())
}

/// The path of a handed file when the browser can read that very file there: the path opens and gives the same device
/// and inode as the descriptor. In a sandbox that shows something else at the path, or nothing, this is `None`.
fn readable_in_place(file: &fs::File) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let path = opened_from(file)?;
    path.to_str()?;
    let (handed, there) = (file.metadata().ok()?, fs::File::open(&path).ok()?.metadata().ok()?);
    (there.is_file() && handed.dev() == there.dev() && handed.ino() == there.ino()).then_some(path)
}

/// The address a private copy is shown under: the `file:` URL of the place it was opened from when the kernel's path
/// ends in the name the shell gave the file, else that name alone.
fn shown_address(from: Option<&Path>, name: &str) -> String {
    let name = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    from.filter(|path| path.is_absolute() && path.to_str().is_some() && path.file_name().is_some_and(|n| n.to_string_lossy() == name))
        .and_then(|path| url::Url::from_file_path(path).ok())
        .map_or(name, String::from)
}

fn stage(base: &Path, docs: Vec<Document>, budget: u64) -> io::Result<Staged> {
    DirBuilder::new().recursive(true).mode(0o700).create(base)?;
    // Recover our private copies after an interrupted browser; never remove another live process's files.
    for entry in fs::read_dir(base)?.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) { continue; }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((pid, nonce)) = name.split_once('-') else { continue };
        if pid.parse::<u32>().is_ok() && nonce.parse::<u128>().is_ok() && !Path::new("/proc").join(pid).exists() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let directory = base.join(format!("{}-{nonce}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&directory)?;
    let mut batch = Staged { directory, paths: Vec::new(), bytes: 0 };
    for (index, mut doc) in docs.into_iter().enumerate() {
        let metadata = doc.file.metadata()?;
        if !metadata.is_file() { return Err(io::Error::other("not a regular file")); }
        let name = Path::new(&doc.name).file_name().filter(|name| !name.is_empty()).ok_or_else(|| io::Error::other("invalid file name"))?;
        let parent = batch.directory.join(index.to_string());
        DirBuilder::new().mode(0o700).create(&parent)?;
        let path = parent.join(name);
        let mut target = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        doc.file.rewind()?;
        let remaining = budget.saturating_sub(batch.bytes);
        let count = io::copy(&mut doc.file.take(remaining + 1), &mut target)?;
        if count > remaining { return Err(io::Error::other("selection exceeds staging budget")); }
        // File.lastModified should describe the chosen file, not the time we prepared its copy.
        if let Ok(modified) = metadata.modified() {
            target.set_times(fs::FileTimes::new().set_modified(modified))?;
        }
        batch.bytes += count;
        batch.paths.push(path);
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handed(dir: &Path, name: &str, bytes: &[u8]) -> Document {
        let source = dir.join(name);
        fs::write(&source, bytes).unwrap();
        Document { request: bifrost_app::share::INBOUND | 1, grant: [7; 16], name: name.into(), size: bytes.len() as u64, mode: bifrost_app::proto::DocMode::Read, file: fs::File::open(&source).unwrap() }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("browser-handed-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_handed_file_the_browser_can_read_loads_from_its_own_place() {
        let dir = scratch("place");
        let mut uploads = Uploads::default();
        uploads.hand_over(&dir.join("opened-files"), vec![handed(&dir, "ok.svg", b"<svg/>"), handed(&dir, "page one.html", b"<p>")]);
        let urls = uploads.take_handed();
        let own = |name: &str| String::from(url::Url::from_file_path(fs::canonicalize(&dir).unwrap().join(name)).unwrap());
        assert_eq!(urls, [own("ok.svg"), own("page one.html")], "one tab per file, at the file's own address");
        assert!(urls[1].ends_with("/page%20one.html"), "{urls:?}");
        assert!(!dir.join("opened-files").exists(), "nothing is copied");
        assert_eq!(uploads.shown(&urls[0]), None, "the address is shown as it is");
        assert!(uploads.take_handed().is_empty(), "a file opens once");
        assert!(uploads.error.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_handed_file_out_of_reach_is_copied_and_shown_by_its_name_not_the_private_path() {
        let dir = scratch("copy");
        let doc = handed(&dir, "page one.html", b"<p>");
        // no path names the file any more: only the descriptor reads it
        fs::remove_file(dir.join("page one.html")).unwrap();
        let mut uploads = Uploads::default();
        uploads.hand_over(&dir.join("opened-files"), vec![doc]);
        let urls = uploads.take_handed();
        assert_eq!(urls.len(), 1);
        let copy = url::Url::parse(&urls[0]).unwrap().to_file_path().unwrap();
        assert!(copy.starts_with(dir.join("opened-files")), "the engine reads a private copy: {copy:?}");
        assert_eq!(fs::read(&copy).unwrap(), b"<p>");
        assert_eq!(uploads.shown(&urls[0]).as_deref(), Some("page one.html"));
        assert_eq!(uploads.shown(&format!("{}#top", urls[0])).as_deref(), Some("page one.html#top"));
        // Return on the shown address loads the same copy
        assert_eq!(uploads.copy_for(" page one.html ").as_deref(), Some(urls[0].as_str()));
        assert_eq!(uploads.copy_for("page one.html#top"), Some(format!("{}#top", urls[0])));
        assert_eq!(uploads.copy_for("other.html"), None);
        // the engine going away keeps the copy and its address
        uploads.reset();
        assert!(copy.exists());
        assert_eq!(uploads.shown(&urls[0]).as_deref(), Some("page one.html"));
        // the copies go with the window
        drop(uploads);
        assert!(!copy.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_exact_copy_is_shown_under_the_original_address() {
        let private = "file:///data/opened-files/";
        let copy = format!("{private}7-1/0/page.html");
        let uploads = Uploads { copies: vec![(copy.clone(), "file:///home/u/page.html".into())], private: Some(private.into()), ..Uploads::default() };
        assert_eq!(uploads.shown(&copy).as_deref(), Some("file:///home/u/page.html"));
        assert_eq!(uploads.shown(&format!("{copy}?a=1#b")).as_deref(), Some("file:///home/u/page.html?a=1#b"));
        assert_eq!(uploads.copy_for("file:///home/u/page.html").as_deref(), Some(copy.as_str()));
        // a file beside the copy (a relative link) is not the original: its name alone, never the private folder
        assert_eq!(uploads.shown(&format!("{private}7-1/0/other.html")).as_deref(), Some("other.html"));
        assert_eq!(uploads.shown(&format!("{private}7-1/0/")).as_deref(), Some("0"));
        // longer, shorter and look-alike addresses are not the copy
        for other in [format!("{copy}/x"), format!("{copy}x"), "file:///data/opened-files-2/page.html".to_string(), "file:///home/u/page.html".to_string(), "https://a.test/file:///data/opened-files/7-1/0/page.html".to_string()] {
            assert!(uploads.shown(&other).map_or(true, |shown| !shown.contains("/home/u/")), "{other}");
        }
        assert_eq!(uploads.shown("file:///home/u/page.html"), None);
        assert_eq!(uploads.shown("https://a.test/"), None);
        assert_eq!(uploads.copy_for("file:///home/u/../u/page.html"), None, "the shown address is matched as text");
    }

    #[test]
    fn a_copy_is_shown_under_the_place_it_came_from_only_when_the_names_agree() {
        assert_eq!(shown_address(Some(Path::new("/home/u/My Files/page one.html")), "page one.html"), "file:///home/u/My%20Files/page%20one.html");
        assert_eq!(shown_address(Some(Path::new("/home/u/page.html (deleted)")), "page.html"), "page.html");
        assert_eq!(shown_address(Some(Path::new("u/page.html")), "page.html"), "page.html");
        assert_eq!(shown_address(None, "page.html"), "page.html");
        assert_eq!(shown_address(None, "../../etc/page.html"), "page.html", "a name is never a path");
    }
}
