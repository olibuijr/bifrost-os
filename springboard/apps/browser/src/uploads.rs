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
            WebViewEvent::EngineExited => *self = Self::default(),
            _ => {}
        }
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
        if !self.pending.as_ref().is_some_and(|request| request.document == document) { return false; }
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
