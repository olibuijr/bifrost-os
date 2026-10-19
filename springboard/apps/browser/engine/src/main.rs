//! `bifrost-webview`: the out-of-process web content engine behind `bifrost_app::webview` (WebView's
//! WebContent process). It embeds libservo with a software (offscreen) GL context, renders the visible web view
//! and copies each presented frame into a shared memory file the app maps, as `0xAARRGGBB` pixels (the SDK
//! `Surface` format), so the app blits it straight into its window.
//!
//! The frame file is the descriptor named by `$BIFROST_WEBVIEW_FD`. The profile directory (cookies, storage) is
//! `$BIFROST_WEBVIEW_PROFILE`.
//!
//! Protocol (one line per message, fields separated by one space; the last field may hold spaces):
//!
//! app -> engine on stdin
//!   size W H SCALE          device pixels of the content rect; the frame file is at least W*H*4 bytes
//!   new ID URL              create web view ID (app-chosen) and load URL
//!   show ID | close ID      make ID the visible view | close it
//!   load ID URL | back ID | forward ID | reload ID | stop ID
//!   touch KIND ID X Y       KIND d|m|u|c, device pixels inside the content rect, sent to the visible view
//!   rclick X Y              secondary click at a point (device pixels inside the content rect): context menu
//!   wheel DX DY X Y         scroll by device pixels at a point
//!   text STRING             typed text;  key NAME  a named key (Enter, Backspace, ArrowLeft, ...)
//!   chord MODS CHAR         a key with modifiers (bits: 1 ctrl, 2 shift, 4 alt, 8 meta)
//!   theme light|dark        the system appearance (prefers-color-scheme)
//!   zoom ID FACTOR          page zoom (1.0 = 100%)
//!   find ID n|f|b QUERY | find ID c   new / next / previous match (injected script; Servo 0.7 has no find API) | clear
//!   focus 0|1               the window gained or lost key focus
//!   devtools on|off         forward console messages (default on)
//!   eval ID JS              run JS (one line) in view ID; answered by `evalresult`
//!   ping N                  heartbeat, answered by `pong N`;  suspend | resume  stop / restart painting
//!   dialog-reply ID TOKEN ok|cancel [TEXT]   answer an alert/confirm/prompt (TEXT escaped; prompt value)
//!   menu-reply ID TOKEN INDEX...   choose item(s) of a context menu / <select> (no INDEX = dismissed)
//!   ack                     the app copied the last frame; the next one may be written
//!   userscript NAME JS      keep JS (escaped like console text) as the document-start script NAME in every view; empty JS removes it
//!   rules-reset | rules-add TEXT | rules-commit   content blocker rules (`blocker.rs`), TEXT escaped like console text
//!   navpolicy on|off        ask the app before each navigation (`navreq`)
//!   nav-reply ID TOKEN allow|cancel|download   answer a `navreq`
//!   cookies ID TOKEN        the cookies of the page URL of view ID, answered by `cookies`
//!   clear-cookies           delete every cookie
//!
//! engine -> app on stdout
//!   frame W H ID               a frame of the visible view is in the shared memory
//!   title ID TEXT | url ID URL | load ID started|head|complete | history ID BACK FORWARD
//!   created ID PARENT URL   the page opened a new window (window.open, target=_blank): a new view ID
//!   found ID COUNT          match count of the last find
//!   closed ID | crashed ID REASON
//!   pong N                  heartbeat answer
//!   fatal REASON            the engine cannot run (setup failed); it exits right after
//!   console ID LEVEL SOURCE LINE TEXT   a console message (LEVEL log|info|debug|warn|error; SOURCE `-` if unknown;
//!                           TEXT escaped `\\n`, `\\r`, `\\\\`, at most 4 KB)
//!   dialog ID TOKEN alert|confirm|prompt DEFAULT\u{1}MESSAGE   script dialog (both escaped); answer with dialog-reply
//!   menu ID TOKEN context|select X Y W H ITEMS   ITEMS: records split by \u{1}, each `INDEX|FLAGS|LABEL`
//!                           (FLAGS 1 enabled, 2 checked, 4 separator, 8 group heading; LABEL escaped); X Y W H are
//!                           device pixels inside the page; answer with menu-reply
//!   control-hide ID TOKEN   the page withdrew a dialog / menu: close it without answering
//!   evalresult ID TEXT      the result of `eval` (escaped like console TEXT)
//!   scriptmsg ID FLAG NAME BODY   a page posted BODY to the message handler NAME (FLAG 1 main frame; BODY escaped, up to 60 KB)
//!   navreq ID TOKEN URL     a navigation waits for `nav-reply`
//!   cookies ID TOKEN RECORDS   see `blocker::cookie_record`
//!   download ID URL         the app answered a `navreq` with `download`: the navigation was cancelled

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use dpi::PhysicalSize;
use euclid::Scale;
use servo::{
    Code, CreateNewWebViewRequest, DevicePoint, DeviceVector2D, EventLoopWaker, InputEvent, Key, KeyState, KeyboardEvent, LoadStatus,
    Location, Modifiers, NamedKey, Opts, RenderingContext, Scroll, Servo, ServoBuilder, SoftwareRenderingContext,
    ContextMenuAction, ContextMenuItem, EmbedderControl, EmbedderControlId, SelectElementOptionOrOptgroup, SimpleDialog,
    Preferences, Theme, TouchEvent, TouchEventType, TouchId, TouchPointerType, WebView, WebViewBuilder, WebViewDelegate,
};
use url::Url;

mod compat;
mod blocker;
// the same source the SDK compiles, so the app and the engine agree on what a content rule matches
#[allow(dead_code)]
#[path = "../../../../sdk/src/rule_match.rs"]
mod rule_match;

/// Default user agent: current Browser on Golden Gate (what most sites sniff for).
const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Golden Gate 10_15_7) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Browser/605.1.15";
mod devtools;
mod frame;
mod guard;
use frame::{Shm, escape_line, say, unescape_line};

enum Msg {
    Line(String),
    Wake,
    Eof,
}

#[derive(Clone)]
struct Waker(Sender<Msg>);
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.send(Msg::Wake);
    }
}

/// What the delegate saw; handled by the main loop, which knows the app's ids.
enum Note {
    Frame(WebView),
    Title(WebView, String),
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
struct Delegate {
    notes: RefCell<Vec<Note>>,
    evals: Cell<usize>,
    /// Controls the page withdrew (navigation, script); the engine tells the app to drop them.
    hidden: RefCell<Vec<EmbedderControlId>>,
}

impl Delegate {
    /// Queues a note, dropping noisy ones (console, frame, title, url, history) when the queue is long.
    fn push(&self, n: Note) {
        let mut q = self.notes.borrow_mut();
        let len = q.len();
        let noisy = matches!(n, Note::Console(..) | Note::Frame(_) | Note::Title(..) | Note::Url(..) | Note::History(_));
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
    fn notify_new_frame_ready(&self, w: WebView) {
        self.push(Note::Frame(w));
    }
    fn notify_page_title_changed(&self, w: WebView, title: Option<String>) {
        self.push(Note::Title(w, title.unwrap_or_default()));
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
        self.push(Note::Console(w, level, message));
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

const MAX_DIM: u32 = 8192;
const MAX_VIEWS: usize = 256;

/// A pixel dimension from the app: 1..=MAX_DIM, garbage becomes 1.
fn dim(s: &str) -> u32 {
    s.parse::<u32>().unwrap_or(1).clamp(1, MAX_DIM)
}

/// A device scale factor: finite and within 0.25..=8, otherwise 1.
fn scale(s: &str) -> f32 {
    s.parse::<f32>().ok().filter(|v| v.is_finite()).map_or(1.0, |v| v.clamp(0.25, 8.0))
}

/// A pointer or scroll coordinate: finite, otherwise 0.
fn coord(s: &str) -> f32 {
    s.parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(0.0)
}

struct Engine {
    servo: Servo,
    rc: Rc<SoftwareRenderingContext>,
    delegate: Rc<Delegate>,
    views: HashMap<u32, WebView>,
    visible: Option<u32>,
    size: PhysicalSize<u32>,
    scale: f32,
    theme: Theme,
    shm: Shm,
    awaiting_ack: bool,
    dirty: bool,
    pending_size: Option<(PhysicalSize<u32>, f32)>,
    next_id: u32,
    shm_warned: bool,
    devtools: bool,
    suspended: bool,
    /// Script dialogs, context menus and <select> popups waiting for the app's answer, by token.
    controls: HashMap<u32, (u32, EmbedderControlId, Pending)>,
    next_token: u32,
}

enum Pending {
    Dialog(SimpleDialog),
    Menu(servo::ContextMenu, Vec<ContextMenuAction>),
    Select(servo::SelectElement),
}

/// Most controls waiting at once; older ones are answered with their default (dropped).
const MAX_CONTROLS: usize = 16;

impl Engine {
    fn id_of(&self, w: &WebView) -> Option<u32> {
        self.views.iter().find(|(_, v)| v.id() == w.id()).map(|(k, _)| *k)
    }
    fn visible_view(&self) -> Option<&WebView> {
        self.visible.and_then(|id| self.views.get(&id))
    }
    fn build(&self, builder: WebViewBuilder) -> WebView {
        let builder = devtools::collector_builder(&self.servo, builder);
        let w = builder.delegate(self.delegate.clone()).hidpi_scale_factor(Scale::new(self.scale)).build();
        w.notify_theme_change(self.theme);
        w
    }

    /// Runs one command line; a panic inside it is reported and the engine carries on.
    fn command(&mut self, line: &str) -> bool {
        let line = guard::cap_line(line);
        let verb = line.split(' ').next().unwrap_or("").to_string();
        match guard::guarded(|| self.command_inner(line)) {
            Ok(keep) => keep,
            Err(msg) => {
                self.dirty = true;
                say(&format!("crashed 4294967295 command {verb} panicked: {}", escape_line(&msg).chars().take(200).collect::<String>()));
                true
            }
        }
    }

    fn command_inner(&mut self, line: &str) -> bool {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let arg = |n: usize| rest.split(' ').nth(n).unwrap_or("");
        let id: u32 = arg(0).parse().unwrap_or(u32::MAX);
        let tail = |n: usize| rest.splitn(n + 1, ' ').nth(n).unwrap_or("").to_string();
        match verb {
            "size" => {
                let (w, h) = (dim(arg(0)), dim(arg(1)));
                // coalesced: only the last size of a burst is applied (apply_size)
                self.pending_size = Some((PhysicalSize::new(w, h), scale(arg(2))));
            }
            "new" => {
                if id == u32::MAX || self.views.contains_key(&id) || self.views.len() >= MAX_VIEWS {
                    return true;
                }
                // a new view takes its viewport from the context size when built (servo paint add_webview)
                self.apply_size();
                let Some(url) = Url::parse(&tail(1)).or_else(|_| Url::parse("about:blank")).ok() else { return true };
                let w = self.build(WebViewBuilder::new(&self.servo, self.rc.clone()).url(url));
                w.resize(self.size);
                // Servo builds views visible (paint webview_renderer hidden: false); every view draws into
                // the one shared scene, so only `show` may make one visible
                w.hide();
                self.next_id = self.next_id.max(id.saturating_add(1));
                self.views.insert(id, w);
            }
            "show" => {
                if let Some(old) = self.visible_view() {
                    old.hide();
                    old.set_focused(false);
                }
                if let Some(v) = self.views.get(&id) {
                    v.show();
                    v.set_focused(true);
                    v.resize(self.size);
                    self.visible = Some(id);
                    self.dirty = true;
                }
            }
            "close" => {
                self.views.remove(&id);
                if self.visible == Some(id) {
                    self.visible = None;
                    self.dirty = false;
                }
                // last view gone: nothing is in flight any more, so a later view paints at once
                if self.views.is_empty() {
                    self.awaiting_ack = false;
                    self.pending_size = None;
                }
            }
            "load" => {
                if let (Some(v), Ok(url)) = (self.views.get(&id), Url::parse(&tail(1))) {
                    v.load(url);
                }
            }
            "back" => {
                if let Some(v) = self.views.get(&id).filter(|v| v.can_go_back()) {
                    v.go_back(1);
                }
            }
            "forward" => {
                if let Some(v) = self.views.get(&id).filter(|v| v.can_go_forward()) {
                    v.go_forward(1);
                }
            }
            "reload" => {
                if let Some(v) = self.views.get(&id) {
                    v.reload();
                }
            }
            // Servo 0.7.0 has no API to cancel a load; Stop is accepted and does nothing.
            "stop" => {}
            "dialog-reply" => {
                let token: u32 = arg(1).parse().unwrap_or(0);
                if let Some((_, _, Pending::Dialog(d))) = self.controls.remove(&token) {
                    if arg(2) == "ok" {
                        match d {
                            SimpleDialog::Prompt(mut p) => {
                                p.set_current_value(&unescape_line(&tail(3)));
                                p.confirm();
                            }
                            d => d.confirm(),
                        }
                    } else {
                        d.dismiss();
                    }
                    self.dirty = true;
                }
            }
            "menu-reply" => {
                let token: u32 = arg(1).parse().unwrap_or(0);
                let picks: Vec<usize> = rest.split(' ').skip(2).filter_map(|v| v.parse().ok()).collect();
                match self.controls.remove(&token) {
                    Some((_, _, Pending::Menu(m, actions))) => match picks.first().and_then(|i| actions.get(*i)) {
                        Some(a) => m.select(*a),
                        None => m.dismiss(),
                    },
                    Some((_, _, Pending::Select(mut s))) => {
                        s.select(picks);
                        s.submit();
                    }
                    Some(_) | None => {}
                }
                self.dirty = true;
            }
            // Servo 0.7.0 has no cache-bypass reload flag; the plain reload runs (gap noted in the README).
            "reload-hard" => {
                // Servo 0.7.0 has no cache-bypassing reload (WebView::reload sends Reload only; components/servo/webview.rs).
                // NetworkManager::clear_cache empties the whole in-memory HTTP cache (no per-origin API), then reload.
                self.servo.network_manager().clear_cache();
                if let Some(v) = self.views.get(&id) {
                    v.reload();
                }
            }
            // Servo 0.7.0 reads the user agent at start-up only; the request is accepted and not applied yet.
            "ua" => {}
            "emulate" => return self.command_inner(&format!("size {rest}")),
            "touch" => {
                let kind = match arg(0) {
                    "d" => TouchEventType::Down,
                    "m" => TouchEventType::Move,
                    "u" => TouchEventType::Up,
                    _ => TouchEventType::Cancel,
                };
                let finger: i32 = arg(1).parse().unwrap_or(0);
                let (x, y) = (coord(arg(2)), coord(arg(3)));
                if let Some(v) = self.visible_view() {
                    let point = DevicePoint::new(x, y).into();
                    v.notify_input_event(InputEvent::Touch(TouchEvent::new(kind, TouchId(finger), point, TouchPointerType::Touch)));
                }
            }
            // secondary (right) click at a point of the visible view: opens the page's context menu
            "rclick" => {
                if let Some(v) = self.visible_view() {
                    let (px, py) = (coord(arg(0)), coord(arg(1)));
                    let point = || -> servo::WebViewPoint { DevicePoint::new(px, py).into() };
                    v.notify_input_event(InputEvent::MouseMove(servo::MouseMoveEvent::new(point())));
                    for action in [servo::MouseButtonAction::Down, servo::MouseButtonAction::Up] {
                        v.notify_input_event(InputEvent::MouseButton(servo::MouseButtonEvent::new(action, servo::MouseButton::Secondary, point())));
                    }
                }
            }
            "wheel" => {
                let n: Vec<f32> = rest.split(' ').map(coord).collect();
                if let (Some(v), [dx, dy, x, y]) = (self.visible_view(), n.as_slice()) {
                    v.notify_scroll_event(Scroll::Delta(DeviceVector2D::new(*dx, *dy).into()), DevicePoint::new(*x, *y).into());
                }
            }
            "text" => {
                if let Some(v) = self.visible_view() {
                    for c in rest.chars() {
                        let key = Key::Character(c.to_string());
                        v.notify_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(KeyState::Down, key.clone())));
                        v.notify_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(KeyState::Up, key)));
                    }
                }
            }
            "key" => {
                let (named, code) = match rest {
                    "Enter" => (NamedKey::Enter, Code::Enter),
                    "Backspace" => (NamedKey::Backspace, Code::Backspace),
                    "Tab" => (NamedKey::Tab, Code::Tab),
                    "Escape" => (NamedKey::Escape, Code::Escape),
                    "Delete" => (NamedKey::Delete, Code::Delete),
                    "ArrowLeft" => (NamedKey::ArrowLeft, Code::ArrowLeft),
                    "ArrowRight" => (NamedKey::ArrowRight, Code::ArrowRight),
                    "ArrowUp" => (NamedKey::ArrowUp, Code::ArrowUp),
                    "ArrowDown" => (NamedKey::ArrowDown, Code::ArrowDown),
                    "Home" => (NamedKey::Home, Code::Home),
                    "End" => (NamedKey::End, Code::End),
                    "PageUp" => (NamedKey::PageUp, Code::PageUp),
                    "PageDown" => (NamedKey::PageDown, Code::PageDown),
                    _ => return true,
                };
                if let Some(v) = self.visible_view() {
                    for state in [KeyState::Down, KeyState::Up] {
                        let e = KeyboardEvent::new_without_event(state, Key::Named(named.clone()), code, Location::Standard, Modifiers::empty(), false, false);
                        v.notify_input_event(InputEvent::Keyboard(e));
                    }
                }
            }
            "chord" => {
                let bits: u32 = arg(0).parse().unwrap_or(0);
                let ch = tail(1);
                let mut m = Modifiers::empty();
                for (bit, flag) in [(1, Modifiers::CONTROL), (2, Modifiers::SHIFT), (4, Modifiers::ALT), (8, Modifiers::META)] {
                    if bits & bit != 0 {
                        m |= flag;
                    }
                }
                if let Some(v) = self.visible_view() {
                    for state in [KeyState::Down, KeyState::Up] {
                        let e = KeyboardEvent::new_without_event(state, Key::Character(ch.clone()), Code::Unidentified, Location::Standard, m, false, false);
                        v.notify_input_event(InputEvent::Keyboard(e));
                    }
                }
            }
            "theme" => {
                self.theme = if rest == "dark" { Theme::Dark } else { Theme::Light };
                for v in self.views.values() {
                    v.notify_theme_change(self.theme);
                }
            }
            "focus" => {
                if let Some(v) = self.visible_view() {
                    v.set_focused(rest == "1");
                }
            }
            "zoom" => {
                if let Some(v) = self.views.get(&id) {
                    v.set_page_zoom(arg(1).parse::<f32>().unwrap_or(1.0).clamp(0.3, 5.0));
                    self.dirty = true;
                }
            }
            "find" => {
                if let Some(v) = self.views.get(&id) {
                    let (mode, q) = (arg(1), tail(2));
                    let quoted: String = q.chars().flat_map(|c| match c {
                        '\\' | '"' => vec!['\\', c],
                        c if (c as u32) < 32 => vec![' '],
                        c => vec![c],
                    }).collect();
                    let script = if mode == "c" {
                        "getSelection().removeAllRanges();0".to_string()
                    } else {
                        let back = mode == "b";
                        let new = mode == "n";
                        format!(
                            "(function(q,b,n){{var t=(document.body?document.body.innerText:'').toLowerCase(),l=q.toLowerCase(),c=0,i=0;\
                             if(!l)return 0;while((i=t.indexOf(l,i))>=0){{c++;i+=l.length}}\
                             if(n)getSelection().removeAllRanges();window.find(q,false,b,true);return c}})(\"{quoted}\",{back},{new})"
                        )
                    };
                    v.evaluate_javascript(script, move |r| {
                        if let Ok(servo::JSValue::Number(n)) = r {
                            say(&format!("found {id} {}", n as u64));
                        }
                    });
                }
            }
            "devtools" => self.devtools = rest == "on",
            "userscript" => devtools::set_user_script(&self.servo, rest),
            "rules-reset" => blocker::rules_reset(),
            "rules-add" => blocker::rules_add(&unescape_line(rest)),
            "rules-commit" => {
                blocker::rules_commit();
            }
            "navpolicy" => {
                blocker::set_ask_app(rest == "on");
                if rest != "on" {
                    blocker::release_all();
                }
            }
            "nav-reply" => {
                if let Some(url) = blocker::answer(arg(1).parse().unwrap_or(0), arg(2)) {
                    say(&format!("download {id} {url}"));
                }
            }
            "cookies" => {
                if let Some(url) = self.views.get(&id).and_then(|v| v.url()) {
                    let host = url.host_str().unwrap_or("").to_string();
                    let jar = self.servo.site_data_manager().cookies_for_url(url, servo::CookieSource::HTTP);
                    let records: Vec<String> = jar
                        .iter()
                        .filter_map(|c| {
                            let (name, value) = c.name_value();
                            let domain = c.domain().map(|d| d.to_string()).unwrap_or_else(|| host.clone());
                            let expires = c.expires_datetime().map(|t| t.unix_timestamp());
                            blocker::cookie_record(name, value, &domain, c.path().unwrap_or("/"), expires, c.secure().unwrap_or(false), c.http_only().unwrap_or(false))
                        })
                        .collect();
                    say(&blocker::cookies_line(id, arg(1), &records));
                }
            }
            "clear-cookies" => self.servo.site_data_manager().clear_cookies(None),
            // The downloads cookie probe (app downloads.rs COOKIE_JS) is answered from the cookie jar instead of
            // `document.cookie`, so HttpOnly cookies reach the download fetch. SiteDataManager::cookies_for_url with
            // CookieSource::HTTP (servo 0.7.0 components/servo/site_data_manager.rs) returns every cookie that matches the URL.
            "eval" if tail(1) == "'bifrost-ck:' + document.cookie" && self.views.get(&id).and_then(|v| v.url()).is_some() => {
                let url = self.views.get(&id).and_then(|v| v.url()).unwrap();
                let jar = self.servo.site_data_manager().cookies_for_url(url, servo::CookieSource::HTTP);
                let line: Vec<String> = jar.iter().map(|c| { let (n, v) = c.name_value(); format!("{n}={v}") }).collect();
                say(&format!("evalresult {id} \"bifrost-ck:{}\"", escape_line(&line.join("; ").replace('"', ""))));
            }
            "eval" => {
                if self.delegate.evals.get() >= guard::MAX_EVALS {
                    say(&format!("evalresult {id} Error: too many evals in flight"));
                } else if let Some(v) = self.views.get(&id) {
                    self.delegate.evals.set(self.delegate.evals.get() + 1);
                    let delegate = self.delegate.clone();
                    v.evaluate_javascript(tail(1), move |r| {
                        let text = match r {
                            Ok(val) => format!("{val:?}"),
                            Err(e) => format!("Error: {e:?}"),
                        };
                        delegate.evals.set(delegate.evals.get().saturating_sub(1));
                        delegate.push(Note::Eval(id, text));
                    });
                }
            }
            "ack" => self.awaiting_ack = false,
            // heartbeat: the app sends `ping N` every 2 s and restarts an engine that stops answering
            "ping" => say(&format!("pong {}", arg(0).chars().take(20).collect::<String>())),
            // window suspended / resumed: stop painting, keep the views
            "suspend" => {
                self.suspended = true;
                if let Some(v) = self.visible_view() {
                    v.set_focused(false);
                }
            }
            "resume" => {
                self.suspended = false;
                self.awaiting_ack = false;
                self.dirty = true;
            }
            "dev" => self.dev_command(rest),
            "quit" => return false,
            _ => {}
        }
        true
    }

    fn notes(&mut self) {
        // controls the page withdrew: forget them (Drop answers them) and tell the app to close its UI
        for cid in std::mem::take(&mut *self.delegate.hidden.borrow_mut()) {
            let tokens: Vec<u32> = self.controls.iter().filter(|(_, (_, id, _))| *id == cid).map(|(t, _)| *t).collect();
            for t in tokens {
                if let Some((view, _, _)) = self.controls.remove(&t) {
                    say(&format!("control-hide {view} {t}"));
                }
            }
        }
        let notes = std::mem::take(&mut *self.delegate.notes.borrow_mut());
        for note in notes {
            match note {
                Note::Frame(_) => {
                    // Any new frame repaints: after `show`, WebRender gets the new root display list
                    // asynchronously (servo paint/painter.rs set_webview_hidden), and the frame that carries
                    // it is not attributed to the shown view. Hidden views do not animate, so this is cheap.
                    if self.visible.is_some() {
                        self.dirty = true;
                    }
                }
                Note::Title(w, t) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("title {id} {t}"));
                    }
                }
                Note::Url(w, u) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("url {id} {u}"));
                    }
                }
                Note::Load(w, s) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("load {id} {s}"));
                    }
                }
                Note::History(w) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("history {id} {} {}", w.can_go_back() as u8, w.can_go_forward() as u8));
                    }
                }
                Note::Created(parent, request) => {
                    let Some(parent_id) = self.id_of(&parent) else { continue };
                    // engine-chosen ids count down from the top so they never meet the app's
                    let id = (u32::MAX / 2).saturating_add(self.next_id);
                    self.next_id = self.next_id.saturating_add(1);
                    let w = self.build(request.builder(self.rc.clone()));
                    w.resize(self.size);
                    w.hide();
                    let url = w.url().map(|u| u.to_string()).unwrap_or_default();
                    self.views.insert(id, w);
                    say(&format!("created {id} {parent_id} {url}"));
                }
                Note::Closed(w) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("closed {id}"));
                    }
                }
                Note::Console(w, level, text) => {
                    if let Some(id) = self.id_of(&w) {
                        if let Some(rest) = text.strip_prefix(blocker::MESSAGE_PREFIX) {
                            // a page message for an app handler: never console output, and sent whatever `devtools` says
                            if let Some(line) = blocker::script_message_line(id, rest) {
                                say(&line);
                            }
                        } else if self.devtools {
                            say(&format!("console {id} {level} - 0 {}", escape_line(&text)));
                        }
                    }
                }
                Note::Nav(w, request) => {
                    let Some(id) = self.id_of(&w) else { continue };
                    let token = self.next_token;
                    self.next_token = self.next_token.wrapping_add(1).max(1);
                    let url = request.url.to_string();
                    blocker::hold(token, request);
                    say(&format!("navreq {id} {token} {url}"));
                }
                Note::Redirect(w, url) => {
                    w.load(url);
                    self.dirty = true;
                }
                Note::Control(w, control) => {
                    let Some(view) = self.id_of(&w) else { continue };
                    self.show_control(view, control);
                }
                Note::Eval(id, text) => say(&format!("evalresult {id} {}", escape_line(&text))),
                Note::Crashed(w, reason) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("crashed {id} {reason}"));
                    }
                }
            }
        }
    }

    /// Forwards a script dialog, context menu or <select> popup to the app; anything else is dropped (answered by Drop).
    fn show_control(&mut self, view: u32, control: EmbedderControl) {
        // dropping the oldest answers it with the default (Servo's Drop impls), so a flood cannot pile up
        if self.controls.len() >= MAX_CONTROLS {
            if let Some(k) = self.controls.keys().min().copied() {
                self.controls.remove(&k);
            }
        }
        let cid = control.id();
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let esc = |s: &str| escape_line(s);
        match control {
            EmbedderControl::SimpleDialog(d) => {
                let (kind, default) = match &d {
                    SimpleDialog::Alert(_) => ("alert", String::new()),
                    SimpleDialog::Confirm(_) => ("confirm", String::new()),
                    SimpleDialog::Prompt(p) => ("prompt", p.current_value().to_string()),
                };
                say(&format!("dialog {view} {token} {kind} {}\u{1}{}", esc(&default), esc(d.message())));
                self.controls.insert(token, (view, cid, Pending::Dialog(d)));
            }
            EmbedderControl::ContextMenu(m) => {
                let r = m.position();
                let mut actions = Vec::new();
                let mut recs = Vec::new();
                for it in m.items() {
                    match it {
                        ContextMenuItem::Separator => recs.push("0|4|".to_string()),
                        ContextMenuItem::Item { label, action, enabled } => {
                            recs.push(format!("{}|{}|{}", actions.len(), if *enabled { 1 } else { 0 }, esc(label)));
                            actions.push(*action);
                        }
                    }
                }
                say(&format!("menu {view} {token} context {} {} {} {} {}", r.min.x, r.min.y, r.width(), r.height(), recs.join("\u{1}")));
                self.controls.insert(token, (view, cid, Pending::Menu(m, actions)));
            }
            EmbedderControl::SelectElement(sel) => {
                let r = sel.position();
                let chosen = sel.selected_options();
                let mut recs = Vec::new();
                for o in sel.options() {
                    match o {
                        SelectElementOptionOrOptgroup::Option(o) => {
                            let f = if o.is_disabled { 0 } else { 1 } | if chosen.contains(&o.id) { 2 } else { 0 };
                            recs.push(format!("{}|{}|{}", o.id, f, esc(&o.label)));
                        }
                        SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                            recs.push(format!("0|8|{}", esc(label)));
                            for o in options {
                                let f = if o.is_disabled { 0 } else { 1 } | if chosen.contains(&o.id) { 2 } else { 0 };
                                recs.push(format!("{}|{}|{}", o.id, f, esc(&o.label)));
                            }
                        }
                    }
                }
                say(&format!("menu {view} {token} select {} {} {} {} {}", r.min.x, r.min.y, r.width(), r.height(), recs.join("\u{1}")));
                self.controls.insert(token, (view, cid, Pending::Select(sel)));
            }
            // color and file pickers, input methods: dropped, which cancels them
            _ => {}
        }
    }

    /// Applies the last `size` of a burst, so a live window resize resizes Servo once per loop turn.
    fn apply_size(&mut self) {
        let Some((size, scale)) = self.pending_size.take() else { return };
        self.size = size;
        // WebView::resize resizes the shared RenderingContext and relayouts every view on it (servo
        // paint/painter.rs resize_rendering_context). It returns early when the context already has the
        // size, so the context is never resized directly while a view exists.
        match self.views.values().next() {
            Some(v) => v.resize(size),
            None => self.rc.resize(size),
        }
        for v in self.views.values() {
            if scale != self.scale {
                v.set_hidpi_scale_factor(Scale::new(scale));
            }
        }
        self.scale = scale;
        self.dirty = true;
    }

    /// Paints the visible view and hands the pixels to the app, at most one frame in flight.
    fn present(&mut self) {
        if !self.dirty || self.awaiting_ack || self.suspended {
            return;
        }
        let Some(v) = self.visible_view() else { return };
        v.paint();
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let rect = euclid::default::Box2D::new(euclid::point2(0, 0), euclid::point2(w as i32, h as i32)).cast_unit();
        if w == 0 || h == 0 {
            self.dirty = false;
            return;
        }
        let Some(img) = self.rc.read_to_image(rect) else { return };
        if img.width() as usize != w || img.height() as usize != h {
            self.dirty = false;
            return;
        }
        if w * h * 4 > MAX_DIM as usize * MAX_DIM as usize * 4 || !self.shm.ensure(w * h * 4) {
            // keep dirty: retry once the app has grown the frame file
            if !self.shm_warned {
                self.shm_warned = true;
                say(&format!("crashed 4294967295 frame file too small for {w}x{h}"));
            }
            return;
        }
        self.shm_warned = false;
        self.dirty = false;
        self.shm.write_argb(img.as_raw(), w * h);
        self.awaiting_ack = true;
        // the view id lets the app drop a frame that was in flight when the tab changed
        say(&format!("frame {w} {h} {}", self.visible.unwrap_or(u32::MAX)));
    }
}

/// Tells the app why the engine cannot run, then exits.
fn die(reason: &str) -> ! {
    say(&format!("fatal {reason}"));
    std::process::exit(1)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--content-process") {
        return servo::run_content_process(args.get(2).cloned().unwrap_or_default());
    }
    let profile = std::env::var_os("BIFROST_WEBVIEW_PROFILE").map(std::path::PathBuf::from);
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
    let reader = tx.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) => {
                    if reader.send(Msg::Line(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = reader.send(Msg::Eof);
    });
    let size = PhysicalSize::new(800, 600);
    let rc = match SoftwareRenderingContext::new(size) {
        Ok(rc) => Rc::new(rc),
        Err(e) => die(&format!("software rendering context: {e:?}")),
    };
    if let Err(e) = rc.make_current() {
        die(&format!("make current: {e:?}"));
    }
    let opts = Opts { config_dir: profile, multiprocess: std::env::var_os("BIFROST_WEBVIEW_SINGLE").is_none(), ..Opts::default() };
    // Sites gate on the UA string: send a Browser-compatible one by default (BIFROST_BROWSER_UA overrides, empty keeps Servo's).
    let mut prefs = Preferences::default();
    let ua = std::env::var("BIFROST_BROWSER_UA").unwrap_or_else(|_| DEFAULT_UA.to_string());
    if !ua.is_empty() {
        prefs.user_agent = ua;
    }
    let servo = ServoBuilder::default().opts(opts).preferences(prefs).event_loop_waker(Box::new(Waker(tx))).build();
    let mut e = Engine {
        servo,
        rc,
        delegate: Rc::new(Delegate::default()),
        views: HashMap::new(),
        visible: None,
        size,
        scale: 1.0,
        theme: Theme::Light,
        shm: Shm { fd: std::env::var("BIFROST_WEBVIEW_FD").ok().and_then(|v| v.parse().ok()).unwrap_or(3), ptr: std::ptr::null_mut(), len: 0 },
        awaiting_ack: false,
        dirty: false,
        pending_size: None,
        next_id: 0,
        shm_warned: false,
        devtools: true,
        suspended: false,
        controls: HashMap::new(),
        next_token: 1,
    };
    let animating = Cell::new(false);
    loop {
        // block until the app or Servo has something; tick at 60 Hz while a page animates
        // idle: block with no timer (Servo wakes us through the Waker)
        let mut first = if animating.get() {
            match rx.recv_timeout(Duration::from_millis(16)) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            }
        };
        let mut open = true;
        while let Some(m) = first.take().or_else(|| rx.try_recv().ok()) {
            match m {
                Msg::Line(l) => open &= e.command(&l),
                Msg::Wake => {}
                Msg::Eof => open = false,
            }
        }
        if !open {
            break;
        }
        e.apply_size();
        e.servo.spin_event_loop();
        e.notes();
        e.present();
        animating.set(e.visible_view().is_some_and(|v| v.animating()));
    }
    e.views.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_args_survive_garbage() {
        for s in ["", " ", "-1", "0", "nan", "NaN", "inf", "-inf", "1e99", "99999999999999999999", "\u{1F600}", "0x10", "1.5.2"] {
            assert!((1..=MAX_DIM).contains(&dim(s)));
            assert!((0.25..=8.0).contains(&scale(s)));
            assert!(coord(s).is_finite());
        }
        assert_eq!(dim("0"), 1);
        assert_eq!(dim("1000000"), MAX_DIM);
        assert_eq!(scale("nan"), 1.0);
    }
}
