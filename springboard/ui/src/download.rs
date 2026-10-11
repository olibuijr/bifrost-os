//! Downloads started by web content: the state of one file a page asks to save, the name it is saved under (from the
//! response's Content-Disposition header) and the progress shown while it arrives. Pure state; the transfer itself
//! runs in the URL client (`bifrost_app::url`) or the engine.

/// Where a download is in its life. `Finished`, `Failed` and `Cancelled` never change again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    /// The navigation became a download; no response headers or bytes yet.
    Requested,
    /// Bytes are arriving. `expected` is `None` when the server sent no length.
    Running { received: u64, expected: Option<u64> },
    /// Every byte arrived.
    Finished { bytes: u64 },
    Failed(String),
    Cancelled,
}

/// One download: where it comes from, the file name it will be saved under, and its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    pub file_name: String,
    state: DownloadState,
}

impl Download {
    /// A download of `url`, saved as `suggested` until the response names the file.
    pub fn new(url: &str, suggested: &str) -> Self {
        Self { url: url.to_string(), file_name: sanitize_file_name(suggested), state: DownloadState::Requested }
    }

    pub fn state(&self) -> &DownloadState {
        &self.state
    }

    /// True until the download reaches a terminal state.
    pub fn is_active(&self) -> bool {
        matches!(self.state, DownloadState::Requested | DownloadState::Running { .. })
    }

    /// Records the response headers. A server file name (Content-Disposition) replaces the suggested one; this is
    /// ignored once bytes have arrived, so the name cannot change under a running transfer.
    pub fn response(&mut self, content_disposition: Option<&str>, content_length: Option<u64>) {
        if self.state != DownloadState::Requested {
            return;
        }
        if let Some(name) = content_disposition.and_then(file_name_from_disposition) {
            self.file_name = sanitize_file_name(&name);
        }
        self.state = DownloadState::Running { received: 0, expected: content_length };
    }

    /// Adds `bytes` received. Ignored unless the download is running.
    pub fn receive(&mut self, bytes: u64) {
        if let DownloadState::Running { received, .. } = &mut self.state {
            *received += bytes;
        }
    }

    /// Completes a running download. Ignored in any other state.
    pub fn finish(&mut self) {
        if let DownloadState::Running { received, .. } = self.state {
            self.state = DownloadState::Finished { bytes: received };
        }
    }

    /// Fails an active download with `reason`. Ignored once the download is terminal.
    pub fn fail(&mut self, reason: &str) {
        if self.is_active() {
            self.state = DownloadState::Failed(reason.to_string());
        }
    }

    /// Cancels an active download. Ignored once the download is terminal.
    pub fn cancel(&mut self) {
        if self.is_active() {
            self.state = DownloadState::Cancelled;
        }
    }

    /// Progress from 0 to 1 when the length is known and non-zero; `None` otherwise (show an indeterminate bar).
    pub fn fraction(&self) -> Option<f32> {
        match self.state {
            DownloadState::Running { received, expected: Some(total) } if total > 0 => {
                Some((received as f32 / total as f32).min(1.0))
            }
            _ => None,
        }
    }
}

/// The file name from a Content-Disposition header value, such as `attachment; filename="report.pdf"`.
/// `filename*` (RFC 5987: `charset'language'percent-encoded`) wins over `filename` when both are present.
/// Parameters are split on `;`, so a quoted name containing `;` is not supported.
pub fn file_name_from_disposition(header: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for param in header.split(';').skip(1) {
        let Some((key, value)) = param.split_once('=') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let encoded = value.splitn(3, '\'').nth(2).unwrap_or(value);
                extended = Some(percent_decode(encoded));
            }
            "filename" => plain = Some(value.trim_matches('"').replace("\\\"", "\"")),
            _ => {}
        }
    }
    extended.filter(|n| !n.trim().is_empty()).or(plain).filter(|n| !n.trim().is_empty())
}

/// A safe single file name: the last path component only, with control characters and `:*?"<>|` replaced by `_`,
/// no leading dots or spaces, at most 255 bytes. An empty result becomes `download`.
pub fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let replaced: String = last.chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    let mut out = replaced.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end().to_string();
    while out.len() > 255 {
        out.pop();
    }
    if out.is_empty() {
        return "download".to_string();
    }
    out
}

/// Decodes `%XX` escapes as UTF-8 bytes; a malformed escape is kept as written.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let byte = if b[i] == b'%' && i + 2 < b.len() {
            std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        } else {
            None
        };
        match byte {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_runs_from_request_to_finish() {
        let mut d = Download::new("https://example.test/a.zip", "a.zip");
        assert!(d.is_active());
        d.response(None, Some(100));
        assert_eq!(d.state(), &DownloadState::Running { received: 0, expected: Some(100) });
        d.receive(40);
        d.receive(60);
        d.finish();
        assert_eq!(d.state(), &DownloadState::Finished { bytes: 100 });
        assert!(!d.is_active());
    }

    #[test]
    fn terminal_states_do_not_change() {
        let mut failed = Download::new("https://example.test/x", "x");
        failed.response(None, None);
        failed.fail("connection reset");
        failed.finish();
        failed.cancel();
        assert_eq!(failed.state(), &DownloadState::Failed("connection reset".into()));

        let mut done = Download::new("https://example.test/y", "y");
        done.response(None, None);
        done.finish();
        done.cancel();
        assert_eq!(done.state(), &DownloadState::Finished { bytes: 0 });
    }

    #[test]
    fn progress_is_a_fraction_only_when_the_length_is_known() {
        let mut d = Download::new("https://example.test/big", "big");
        d.response(None, Some(200));
        d.receive(50);
        assert_eq!(d.fraction(), Some(0.25));
        d.receive(500);
        assert_eq!(d.fraction(), Some(1.0));

        let mut unknown = Download::new("https://example.test/stream", "stream");
        unknown.response(None, None);
        unknown.receive(10);
        assert_eq!(unknown.fraction(), None);
    }

    #[test]
    fn the_server_file_name_wins_and_extended_names_are_decoded() {
        assert_eq!(file_name_from_disposition("attachment; filename=\"report 1.pdf\"").as_deref(), Some("report 1.pdf"));
        assert_eq!(
            file_name_from_disposition("attachment; filename=\"a.txt\"; filename*=UTF-8''%C3%A1rni.txt").as_deref(),
            Some("\u{e1}rni.txt")
        );
        assert_eq!(file_name_from_disposition("inline"), None);

        let mut d = Download::new("https://example.test/dl?id=7", "dl");
        d.response(Some("attachment; filename=\"../../evil.sh\""), Some(5));
        assert_eq!(d.file_name, "evil.sh");
    }

    #[test]
    fn the_name_does_not_change_once_bytes_arrive() {
        let mut d = Download::new("https://example.test/q", "first.bin");
        d.response(None, Some(10));
        d.receive(3);
        d.response(Some("attachment; filename=\"second.bin\""), Some(99));
        assert_eq!(d.file_name, "first.bin");
        assert_eq!(d.state(), &DownloadState::Running { received: 3, expected: Some(10) });
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("a:b*?.txt"), "a_b__.txt");
        assert_eq!(sanitize_file_name("...hidden"), "hidden");
        assert_eq!(sanitize_file_name("   "), "download");
        assert_eq!(sanitize_file_name(&"x".repeat(300)).len(), 255);
    }
}
