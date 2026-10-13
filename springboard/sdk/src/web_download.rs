//! Downloads started by web content (WebKit's `WebDownload` and `WebDownloadDelegate`).
//!
//! A navigation becomes a download when the response carries `Content-Disposition: attachment` or a MIME type the
//! view cannot render (`navigation_response_policy`, `NavigationResponsePolicy.download`). The app then owns a
//! [`WebDownload`]: it asks the delegate where to save (`download(_:decideDestinationUsing:...)`), tracks
//! `progress`, and can `cancel` to get [`ResumeData`] (offset and ETag) that `resume_headers` turns into a Range
//! request. This module is pure state; the transfer itself runs in the URL client or the engine.

use std::path::{Path, PathBuf};

use crate::webview::WebViewId;

/// What the view does with a response (`NavigationResponsePolicy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationResponsePolicy {
    Cancel,
    Allow,
    /// `NavigationResponsePolicy.download`: the navigation becomes a `WebDownload`.
    Download,
}

/// MIME types a web view renders itself.
fn is_renderable(mime: &str) -> bool {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    m.is_empty()
        || m.starts_with("text/")
        || m.starts_with("image/")
        || m.starts_with("video/")
        || m.starts_with("audio/")
        || matches!(
            m.as_str(),
            "application/xhtml+xml" | "application/json" | "application/xml" | "application/pdf" | "application/javascript"
        )
}

/// Decides a response: an attachment or a non-renderable MIME type downloads, anything else is shown.
pub fn navigation_response_policy(content_disposition: Option<&str>, mime: &str) -> NavigationResponsePolicy {
    let attachment = content_disposition.map_or(false, |d| d.trim_start().to_ascii_lowercase().starts_with("attachment"));
    if attachment || !is_renderable(mime) {
        NavigationResponsePolicy::Download
    } else {
        NavigationResponsePolicy::Allow
    }
}

/// `suggestedFilename`: the `filename` of a Content-Disposition header, else the last URL path segment, else
/// `download`. Path separators are dropped so a server cannot write outside the chosen folder.
pub fn suggested_filename(content_disposition: Option<&str>, url: &str) -> String {
    let from_header = content_disposition.and_then(|d| {
        let at = d.to_ascii_lowercase().find("filename=")?;
        let v = d[at + "filename=".len()..].split(';').next()?.trim().trim_matches('"');
        Some(v.to_string())
    });
    let from_url = || {
        let path = url.split(['?', '#']).next().unwrap_or("");
        let path = path.split_once("://").map_or(path, |(_, r)| r.split_once('/').map_or("", |(_, p)| p));
        path.rsplit('/').next().unwrap_or("").to_string()
    };
    let raw = from_header.filter(|n| !n.is_empty()).unwrap_or_else(from_url);
    let clean: String = raw.rsplit(['/', '\\']).next().unwrap_or("").trim().trim_start_matches('.').to_string();
    if clean.is_empty() {
        "download".to_string()
    } else {
        clean
    }
}

/// The first free name in `dir`: `name.ext`, then `name (1).ext`, `name (2).ext`... `exists` reports taken paths.
pub fn unique_destination(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1u32..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !exists(p))
        .expect("an unbounded range always finds a free name")
}

/// `Progress`: bytes done of a total that may be unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadProgress {
    pub completed: u64,
    pub total: Option<u64>,
}

impl DownloadProgress {
    /// `fractionCompleted`, 0 to 1; 0 while the total is unknown.
    pub fn fraction(&self) -> f64 {
        match self.total {
            Some(t) if t > 0 => (self.completed as f64 / t as f64).min(1.0),
            _ => 0.0,
        }
    }
}

/// What `cancel(_:)` hands back to restart the transfer later (`resumeData`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeData {
    pub url: String,
    /// Bytes already on disk.
    pub offset: u64,
    pub etag: Option<String>,
}

/// Headers of the request that continues a download: `Range` from the offset, and `If-Range` so a changed file
/// restarts from zero instead of mixing two versions.
pub fn resume_headers(data: &ResumeData) -> Vec<(String, String)> {
    let mut h = vec![("Range".to_string(), format!("bytes={}-", data.offset))];
    if let Some(e) = &data.etag {
        h.push(("If-Range".to_string(), e.clone()));
    }
    h
}

/// A download's life. `Finished`, `Failed` and `Cancelled` never change again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebDownloadState {
    /// Waiting for the delegate's destination.
    AwaitingDestination,
    Running,
    Finished,
    Failed(String),
    Cancelled,
}

/// `WebDownload`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebDownload {
    /// `originalRequest` URL.
    pub original_request: String,
    /// `webView`
    pub web_view: WebViewId,
    /// `progress`
    pub progress: DownloadProgress,
    pub destination: Option<PathBuf>,
    pub etag: Option<String>,
    state: WebDownloadState,
}

impl WebDownload {
    pub fn new(web_view: WebViewId, url: &str) -> Self {
        Self {
            original_request: url.to_string(),
            web_view,
            progress: DownloadProgress::default(),
            destination: None,
            etag: None,
            state: WebDownloadState::AwaitingDestination,
        }
    }

    /// A download that continues `data`: progress starts at the saved offset.
    pub fn resuming(web_view: WebViewId, data: &ResumeData) -> Self {
        let mut d = Self::new(web_view, &data.url);
        d.progress.completed = data.offset;
        d.etag = data.etag.clone();
        d
    }

    pub fn state(&self) -> &WebDownloadState {
        &self.state
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, WebDownloadState::AwaitingDestination | WebDownloadState::Running)
    }

    /// The response arrived: records the length of the whole file (`content_length` is the remaining body, so the
    /// resumed offset is added) and the ETag.
    pub fn response(&mut self, content_length: Option<u64>, etag: Option<&str>) {
        if !self.is_active() {
            return;
        }
        self.progress.total = content_length.map(|l| l + self.progress.completed);
        if let Some(e) = etag {
            self.etag = Some(e.to_string());
        }
    }

    /// The delegate chose a destination (`completionHandler(url)`); `None` cancels, as passing nil does.
    pub fn set_destination(&mut self, dest: Option<PathBuf>) {
        if self.state != WebDownloadState::AwaitingDestination {
            return;
        }
        match dest {
            Some(p) => {
                self.destination = Some(p);
                self.state = WebDownloadState::Running;
            }
            None => self.state = WebDownloadState::Cancelled,
        }
    }

    /// `bytes` arrived. Ignored unless running.
    pub fn receive(&mut self, bytes: u64) {
        if self.state == WebDownloadState::Running {
            self.progress.completed += bytes;
        }
    }

    pub fn finish(&mut self) {
        if self.state == WebDownloadState::Running {
            self.state = WebDownloadState::Finished;
        }
    }

    /// Fails an active download and returns the resume data for `download(_:didFailWithError:resumeData:)`.
    pub fn fail(&mut self, reason: &str) -> Option<ResumeData> {
        if !self.is_active() {
            return None;
        }
        self.state = WebDownloadState::Failed(reason.to_string());
        self.resume_data()
    }

    /// `cancel(_:)`: stops the transfer and returns resume data, or `None` when nothing was received.
    pub fn cancel(&mut self) -> Option<ResumeData> {
        if !self.is_active() {
            return None;
        }
        self.state = WebDownloadState::Cancelled;
        self.resume_data()
    }

    fn resume_data(&self) -> Option<ResumeData> {
        (self.progress.completed > 0).then(|| ResumeData {
            url: self.original_request.clone(),
            offset: self.progress.completed,
            etag: self.etag.clone(),
        })
    }
}

/// The answer to `download(_:willPerformHTTPRedirection:newRequest:decisionHandler:)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectDecision {
    Allow,
    Cancel,
}

/// `WebDownloadDelegate`. Return values stand for the completion handlers.
pub trait WebDownloadDelegate {
    /// `download(_:decideDestinationUsing:suggestedFilename:completionHandler:)`: the file to write, or `None` to cancel.
    fn decide_destination(&mut self, download: &WebDownload, response_url: &str, suggested_filename: &str) -> Option<PathBuf>;
    /// `downloadDidFinish(_:)`
    fn download_did_finish(&mut self, _download: &WebDownload) {}
    /// `download(_:didFailWithError:resumeData:)`
    fn download_did_fail(&mut self, _download: &WebDownload, _error: &str, _resume_data: Option<ResumeData>) {}
    /// `download(_:willPerformHTTPRedirection:newRequest:decisionHandler:)`
    fn will_perform_redirection(&mut self, _download: &WebDownload, _response_code: u16, _new_url: &str) -> RedirectDecision {
        RedirectDecision::Allow
    }
}

/// Drives one download through its delegate: the glue the URL client calls as the transfer advances.
pub struct DownloadDriver<D: WebDownloadDelegate> {
    pub download: WebDownload,
    pub delegate: D,
}

impl<D: WebDownloadDelegate> DownloadDriver<D> {
    pub fn new(download: WebDownload, delegate: D) -> Self {
        Self { download, delegate }
    }

    /// Response headers arrived: records them and asks the delegate for a destination.
    pub fn on_response(&mut self, content_disposition: Option<&str>, content_length: Option<u64>, etag: Option<&str>) {
        self.download.response(content_length, etag);
        if self.download.state() != &WebDownloadState::AwaitingDestination {
            return;
        }
        let url = self.download.original_request.clone();
        let name = suggested_filename(content_disposition, &url);
        let dest = self.delegate.decide_destination(&self.download, &url, &name);
        self.download.set_destination(dest);
    }

    /// A redirect: the delegate may cancel it. Returns true when the transfer follows it.
    pub fn on_redirect(&mut self, code: u16, new_url: &str) -> bool {
        if self.delegate.will_perform_redirection(&self.download, code, new_url) == RedirectDecision::Allow {
            true
        } else {
            self.download.cancel();
            false
        }
    }

    pub fn on_bytes(&mut self, n: u64) {
        self.download.receive(n);
    }

    pub fn on_finished(&mut self) {
        let was_running = self.download.state() == &WebDownloadState::Running;
        self.download.finish();
        if was_running {
            self.delegate.download_did_finish(&self.download);
        }
    }

    pub fn on_error(&mut self, error: &str) {
        if self.download.is_active() {
            let resume = self.download.fail(error);
            self.delegate.download_did_fail(&self.download, error, resume);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rec {
        dest: Option<PathBuf>,
        asked: Vec<String>,
        finished: u32,
        failed: Vec<(String, Option<ResumeData>)>,
        redirect: Option<RedirectDecision>,
    }
    impl WebDownloadDelegate for Rec {
        fn decide_destination(&mut self, _d: &WebDownload, _u: &str, name: &str) -> Option<PathBuf> {
            self.asked.push(name.to_string());
            self.dest.clone()
        }
        fn download_did_finish(&mut self, _d: &WebDownload) {
            self.finished += 1;
        }
        fn download_did_fail(&mut self, _d: &WebDownload, e: &str, r: Option<ResumeData>) {
            self.failed.push((e.to_string(), r));
        }
        fn will_perform_redirection(&mut self, _d: &WebDownload, _c: u16, _u: &str) -> RedirectDecision {
            self.redirect.unwrap_or(RedirectDecision::Allow)
        }
    }

    fn driver(dest: Option<&str>) -> DownloadDriver<Rec> {
        let rec = Rec { dest: dest.map(PathBuf::from), ..Default::default() };
        DownloadDriver::new(WebDownload::new(3, "https://x.test/files/a.zip"), rec)
    }

    #[test]
    fn attachment_or_unrenderable_mime_becomes_a_download() {
        use NavigationResponsePolicy::*;
        assert_eq!(navigation_response_policy(Some("attachment; filename=a.txt"), "text/plain"), Download);
        assert_eq!(navigation_response_policy(None, "application/zip"), Download);
        assert_eq!(navigation_response_policy(None, "application/octet-stream"), Download);
        assert_eq!(navigation_response_policy(None, "text/html; charset=utf-8"), Allow);
        assert_eq!(navigation_response_policy(Some("inline"), "image/png"), Allow);
    }

    #[test]
    fn suggested_filename_prefers_header_then_url_and_strips_paths() {
        assert_eq!(suggested_filename(Some("attachment; filename=\"r.pdf\""), "https://x.test/a"), "r.pdf");
        assert_eq!(suggested_filename(None, "https://x.test/dir/b.tar.gz?x=1"), "b.tar.gz");
        assert_eq!(suggested_filename(Some("attachment; filename=../../etc/passwd"), "https://x.test/"), "passwd");
        assert_eq!(suggested_filename(None, "https://x.test/"), "download");
    }

    #[test]
    fn unique_destination_numbers_before_the_extension() {
        let taken = [PathBuf::from("/d/a.zip"), PathBuf::from("/d/a (1).zip")];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(unique_destination(Path::new("/d"), "a.zip", exists), PathBuf::from("/d/a (2).zip"));
        assert_eq!(unique_destination(Path::new("/d"), "b.zip", exists), PathBuf::from("/d/b.zip"));
        assert_eq!(unique_destination(Path::new("/d"), "README", |p| p == Path::new("/d/README")), PathBuf::from("/d/README (1)"));
    }

    #[test]
    fn delegate_picks_destination_and_progress_updates() {
        let mut d = driver(Some("/d/a.zip"));
        d.on_response(None, Some(100), Some("\"v1\""));
        assert_eq!(d.delegate.asked, ["a.zip"]);
        assert_eq!(d.download.state(), &WebDownloadState::Running);
        d.on_bytes(25);
        assert_eq!(d.download.progress.fraction(), 0.25);
        d.on_bytes(75);
        d.on_finished();
        assert_eq!(d.delegate.finished, 1);
        assert_eq!(d.download.state(), &WebDownloadState::Finished);
    }

    #[test]
    fn nil_destination_cancels() {
        let mut d = driver(None);
        d.on_response(None, None, None);
        assert_eq!(d.download.state(), &WebDownloadState::Cancelled);
    }

    #[test]
    fn cancel_gives_offset_and_etag_and_resume_sends_range() {
        let mut d = driver(Some("/d/a.zip"));
        d.on_response(None, Some(100), Some("\"v1\""));
        d.on_bytes(40);
        let data = d.download.cancel().expect("resume data");
        assert_eq!(data, ResumeData { url: "https://x.test/files/a.zip".into(), offset: 40, etag: Some("\"v1\"".into()) });
        assert_eq!(
            resume_headers(&data),
            [("Range".to_string(), "bytes=40-".to_string()), ("If-Range".to_string(), "\"v1\"".to_string())]
        );
        let mut again = WebDownload::resuming(3, &data);
        again.response(Some(60), None);
        assert_eq!(again.progress, DownloadProgress { completed: 40, total: Some(100) });
        assert_eq!(WebDownload::new(1, "u").cancel(), None);
    }

    #[test]
    fn failure_reports_error_with_resume_data() {
        let mut d = driver(Some("/d/a.zip"));
        d.on_response(None, Some(100), None);
        d.on_bytes(10);
        d.on_error("connection reset");
        assert_eq!(d.delegate.failed.len(), 1);
        assert_eq!(d.delegate.failed[0].0, "connection reset");
        assert_eq!(d.delegate.failed[0].1.as_ref().map(|r| r.offset), Some(10));
        d.on_error("again");
        assert_eq!(d.delegate.failed.len(), 1);
    }

    #[test]
    fn redirect_can_be_refused() {
        let mut d = driver(Some("/d/a.zip"));
        assert!(d.on_redirect(302, "https://y.test/a.zip"));
        d.delegate.redirect = Some(RedirectDecision::Cancel);
        assert!(!d.on_redirect(302, "https://z.test/a.zip"));
        assert_eq!(d.download.state(), &WebDownloadState::Cancelled);
    }
}
