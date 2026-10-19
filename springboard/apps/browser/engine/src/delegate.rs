//! Translate Servo callbacks into engine events.
use super::*;

/// What the delegate saw; handled by the main loop, which knows the app's ids.
pub(super) enum Note {
    Frame(WebView),
    Title(WebView, String),
    Favicon(WebView),
    Clipboard(String),
    Status(WebView, String),
    Unload(WebView, controls::UnloadRequest),
    Url(WebView, String),
    Load(WebView, &'static str),
    History(WebView),
    Created(WebView, CreateNewWebViewRequest),
    Closed(WebView),
    Crashed(WebView, String),
    Console(WebView, &'static str, String),
    Eval(u32, String),
    Control(WebView, EmbedderControl),
    /// A navigation the app is asked about (`navpolicy on`).
    Nav(WebView, servo::NavigationRequest),
    /// A navigation the content rules turned into another URL.
    Redirect(WebView, Url),
}

#[derive(Default)]
pub(super) struct Delegate {
    pub(super) notes: RefCell<Vec<Note>>,
    pub(super) evals: Cell<usize>,
    /// Controls the page withdrew (navigation, script); the engine tells the app to drop them.
    pub(super) hidden: RefCell<Vec<EmbedderControlId>>,
    pub(super) clipboard: RefCell<String>,
}

impl servo::ClipboardDelegate for Delegate {
    fn clear(&self, _view: WebView) {
        self.clipboard.borrow_mut().clear();
        self.push(Note::Clipboard(String::new()));
    }

    fn get_text(&self, _view: WebView, request: servo::StringRequest) {
        request.success(self.clipboard.borrow().clone());
    }

    fn set_text(&self, _view: WebView, text: String) {
        *self.clipboard.borrow_mut() = text.clone();
        self.push(Note::Clipboard(text));
    }
}

impl Delegate {
    /// Queues a note, dropping noisy ones (console, frame, title, url, history) when the queue is long.
    pub(super) fn push(&self, n: Note) {
        let mut q = self.notes.borrow_mut();
        let len = q.len();
        let noisy = matches!(n, Note::Console(..) | Note::Frame(_) | Note::Title(..) | Note::Status(..) | Note::Favicon(_) | Note::Url(..) | Note::History(_));
        if (len >= guard::NOTES_SOFT && matches!(n, Note::Console(..))) || (len >= guard::NOTES_HARD && noisy) {
            return;
        }
        if let (Note::Frame(w), Some(Note::Frame(last))) = (&n, q.last()) {
            if w.id() == last.id() {
                return;
            }
        }
        q.push(n);
    }
}

impl WebViewDelegate for Delegate {
    fn notify_status_text_changed(&self, view: WebView, status: Option<String>) {
        self.push(Note::Status(view, guard::cap_text(status.as_deref().unwrap_or(""), 4096).to_string()));
    }
    fn request_unload(&self, view: WebView, request: servo::AllowOrDenyRequest) {
        let request = controls::UnloadRequest::new(request);
        if self.notes.borrow().len() >= guard::NOTES_HARD { return; }
        self.push(Note::Unload(view, request));
    }
    fn notify_new_frame_ready(&self, w: WebView) {
        self.push(Note::Frame(w));
    }
    fn notify_page_title_changed(&self, w: WebView, title: Option<String>) {
        self.push(Note::Title(w, title.unwrap_or_default()));
    }
    fn notify_favicon_changed(&self, view: WebView) {
        self.push(Note::Favicon(view));
    }
    fn notify_url_changed(&self, w: WebView, url: Url) {
        self.push(Note::Url(w, url.to_string()));
    }
    fn notify_load_status_changed(&self, w: WebView, s: LoadStatus) {
        let s = match s {
            LoadStatus::Started => "started",
            LoadStatus::HeadParsed => "head",
            LoadStatus::Complete => "complete",
        };
        self.push(Note::Load(w, s));
    }
    fn notify_history_changed(&self, w: WebView, _: Vec<Url>, _: usize) {
        self.push(Note::History(w));
    }
    fn request_create_new(&self, parent: WebView, request: CreateNewWebViewRequest) {
        self.push(Note::Created(parent, request));
    }
    fn notify_closed(&self, w: WebView) {
        self.push(Note::Closed(w));
    }
    fn show_console_message(&self, w: WebView, level: servo::ConsoleLogLevel, message: String) {
        use servo::ConsoleLogLevel as L;
        let level = match level {
            L::Warn => "warn",
            L::Error => "error",
            L::Info => "info",
            L::Debug | L::Trace => "debug",
            _ => "log",
        };
        self.push(Note::Console(w, level, guard::cap_text(&message, 4096).to_string()));
    }
    fn show_embedder_control(&self, w: WebView, control: EmbedderControl) {
        self.push(Note::Control(w, control));
    }
    fn hide_embedder_control(&self, _w: WebView, id: EmbedderControlId) {
        self.hidden.borrow_mut().push(id);
    }
    fn notify_crashed(&self, w: WebView, reason: String, _: Option<String>) {
        self.push(Note::Crashed(w, reason));
    }
    fn request_navigation(&self, w: WebView, request: servo::NavigationRequest) {
        match blocker::judge_navigation(&request.url) {
            blocker::Verdict::Block => request.deny(),
            blocker::Verdict::Https(url) => {
                request.deny();
                self.push(Note::Redirect(w, url));
            }
            blocker::Verdict::Pass if blocker::ask_app() => self.push(Note::Nav(w, request)),
            blocker::Verdict::Pass => request.allow(),
        }
    }
    fn load_web_resource(&self, w: WebView, load: servo::WebResourceLoad) {
        let top = if load.request().is_for_main_frame { None } else { w.url().map(|u| u.to_string()) };
        blocker::filter_load(load, top);
    }
}
