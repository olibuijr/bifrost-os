//! `bifrost-webview`: the out-of-process web content engine behind `bifrost_app::webview` (WebView's
//! WebContent process). It embeds libservo with a hardware GL context (explicit CPU fallback), renders the visible web view
//! and copies each presented frame into a shared memory file the app maps, as `0xAABBGGRR` pixels (the SDK
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
//!   file-reply ID TOKEN JSON   selected absolute paths, or null to cancel
//!   color-reply ID TOKEN ok R G B | color-reply ID TOKEN cancel
//!   unload-reply ID TOKEN allow|cancel   answer a beforeunload navigation confirmation
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
//!   file-picker ID TOKEN JSON   {"multiple": bool, "filters": [extension, ...]}
//!   color-picker ID TOKEN R G B   native color input
//!   unload ID TOKEN         native beforeunload confirmation
//!   status ID TEXT          escaped hovered-link/status text
//!   evalresult ID TEXT      the result of `eval` (escaped like console TEXT)
//!   scriptmsg ID FLAG NAME BODY   a page posted BODY to the message handler NAME (FLAG 1 main frame; BODY escaped, up to 60 KB)
//!   navreq ID TOKEN URL     a navigation waits for `nav-reply`
//!   cookies ID TOKEN RECORDS   see `blocker::cookie_record`
//!   download ID URL         the app answered a `navreq` with `download`: the navigation was cancelled

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use dpi::PhysicalSize;
use euclid::Scale;
use servo::{
    Code, CreateNewWebViewRequest, DevicePoint, EventLoopWaker, InputEvent, Key, KeyState, KeyboardEvent, LoadStatus,
    Location, Modifiers, NamedKey, RenderingContext, Servo, ServoBuilder, SoftwareRenderingContext,
    ContextMenuAction, ContextMenuItem, EmbedderControl, EmbedderControlId, SelectElementOptionOrOptgroup, SimpleDialog,
    Theme, TouchEvent, TouchEventType, TouchId, TouchPointerType, WebView, WebViewBuilder, WebViewDelegate,
};
use url::Url;

mod commands;
mod input;
mod favicon;
mod find;
mod delegate;
mod events;
mod controls;
mod rendering;
mod runtime;
mod graphics;
mod configuration;
mod memory;
mod event_loop;
use delegate::{Delegate, Note};

mod blocker;
// the same source the SDK compiles, so the app and the engine agree on what a content rule matches
#[allow(dead_code)]
mod rule_match;

/// Default user agent: current Browser on Golden Gate (what most sites sniff for).
const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Golden Gate 10_15_7) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Browser/605.1.15";
mod devtools;
mod frame;
mod guard;
use frame::{Shm, escape_line, say, unescape_line};

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
    rc: Rc<dyn RenderingContext>,
    renderer_mode: &'static str,
    renderer_driver: String,
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
    unloads: HashMap<u32, (u32, controls::UnloadRequest)>,
}

enum Pending {
    Dialog(SimpleDialog),
    Menu(servo::ContextMenu, Vec<ContextMenuAction>),
    Select(servo::SelectElement),
    File(servo::FilePicker),
    Color(servo::ColorPicker),
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
        let w = builder.delegate(self.delegate.clone()).clipboard_delegate(self.delegate.clone())
            .hidpi_scale_factor(Scale::new(self.scale)).build();
        w.notify_theme_change(self.theme);
        w
    }
}

/// Tells the app why the engine cannot run, then exits.
fn die(reason: &str) -> ! {
    say(&format!("fatal {reason}"));
    std::process::exit(1)
}

fn main() {
    runtime::run();
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
