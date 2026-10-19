//! BifrOSt Browser: a Golden Gate SDK app (full-size window, unified toolbar, Browser-style tabs) around the SDK's
//! web view (`bifrost_app::webview`). Pages render in the bundled Servo engine process; the app draws the chrome.
mod lifecycle;
mod profile_data;
mod navigation;
mod layout;
mod actions;
mod menus;
mod browser_menu;
mod events;
mod input;
mod keyboard;
mod pointer_input;
mod drawing;
mod app;
mod engine_loader;
use engine_loader::engine;

mod perf;
mod store;
mod suggest;

mod browser_ctl;
mod webbridge;
mod control;
mod devtools;
mod elements;
mod network;
mod pagetools;
mod resample;

mod devmenu;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_SUBHEAD_PT, ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, SPACE_S, SPACE_XXS, TINT};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::webview::{FindDirection, WebProcessPool, WebRenderer, WebViewEvent, WebViewId};
use bifrost_core::json::Json;
use bifrost_app::control::RpcError;
mod inspector;
mod pageui;
mod prefs;
mod system;
use inspector::Inspector;
use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
use bifrost_app::network_path::{self, Status};
use bifrost_app::{glyphs, widgets, App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};

/// The engine archive bundled in assets/servo.tar.gz (version and the first 12 hex digits of its SHA-256).
const ENGINE: &str = "servo-0.7.0-bifrost.15-5e041ae3e32a";
const START: &str = "about:blank";

/// Toolbar ids.
const BACK: u32 = 1;
const FORWARD: u32 = 2;
const ADDRESS: u32 = 3;
const RELOAD: u32 = 4;
const NEW_TAB: u32 = 5;
const DOWNLOADS: u32 = 6;
const MAIN_MENU: u32 = 7;
/// Accessibility ids of tabs start here (`TAB0 + index`).
const TAB0: u64 = 100;

/// Tab strip height (Browser's compact tab bar), points.
const TAB_STRIP_PT: f32 = 30.0;
/// The address field's widest width, points (Browser keeps it readable on wide windows).
const ADDRESS_MAX_PT: f32 = 560.0;
/// Below this movement (points) a touch on the chrome is a tap.
const TAP_SLOP_PT: f32 = 6.0;

/// evdev keys for the browser's shortcuts.
const KEY_L: u32 = 38;
const KEY_C: u32 = 46;
const KEY_K: u32 = 37;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_R: u32 = 19;
/// Main-menu item ids (`App::menu_action`): File > Share and Downloads, View > Show Reader, Zoom and Find.
const MENU_READER: u32 = 1;
const MENU_SHARE: u32 = 2;
const MENU_ZOOM_IN: u32 = 3;
const MENU_ZOOM_OUT: u32 = 4;
const MENU_ACTUAL_SIZE: u32 = 5;
const MENU_FIND: u32 = 6;
const MENU_FIND_NEXT: u32 = 7;
const MENU_FIND_PREV: u32 = 8;
const MENU_DOWNLOADS: u32 = 9;
const MENU_CLEAR_DOWNLOADS: u32 = 10;
const MENU_AUTOFILL: u32 = 11;
const MENU_OVERVIEW: u32 = 12;
const KEY_D: u32 = 32;
const KEY_Y: u32 = 21;
const KEY_B: u32 = 48;
const KEY_U: u32 = 22;
const KEY_LEFTBRACE: u32 = 26;
const KEY_RIGHTBRACE: u32 = 27;
const KEY_TAB: u32 = 15;
const KEY_COMMA: u32 = 51;
const KEY_N: u32 = 49;
/// Hold this long without moving on a page and the touch becomes a right-click (the context menu).
const LONG_PRESS_S: f64 = 0.5;
/// evdev KEY_1 ..= KEY_9 are 2 ..= 10.
const KEY_1: u32 = 2;
const KEY_9: u32 = 10;

mod downloads;
mod uploads;
mod findbar;
mod tabdiscard;
mod library;
mod crashbanner;
mod reader;
mod favicon;
mod inbox;
mod tokens;
mod session;
mod autofill;
mod overview;

const KEY_F: u32 = 33;
const KEY_BACKSLASH: u32 = 43;
const KEY_G: u32 = 34;
const KEY_0: u32 = 11;
const KEY_MINUS: u32 = 12;
const KEY_EQUAL: u32 = 13;

struct Tab {
    view: WebViewId,
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Bar(u32),
    Tab(usize),
    TabClose(usize),
    Sugg(usize),
    Web,
    BannerClose,
    BannerReload,
    Reader,
    /// The star at the address field's trailing edge: toggles the bookmark of the open page.
    Bookmark,
}

struct Browser {
    pool: Option<WebProcessPool>,
    error: Option<String>,
    tabs: Vec<Tab>,
    current: usize,
    address: TextField,
    editing: bool,
    /// Last pointer position in pixels (tab hover).
    hover: Option<(f32, f32)>,
    press: Option<(Press, f32, f32)>,
    bar_layout: Option<ToolbarLayout>,
    main_menu: browser_menu::MainMenu,
    initial_url: Option<String>,
    dark: Option<bool>,
    storm: session::recover::StormGuard,
    crash: crashbanner::CrashBanner,
    reader: reader::Reader,
    watchdog: session::recover::Watchdog,
    discard: tabdiscard::Discarder,
    clock: std::time::Instant,
    crash_log: Vec<String>,
    restore: Vec<String>,
    restore_sel: usize,
    /// Session only, never saved: the GPU engine could not start, so the engine runs with software rendering.
    renderer_fallback: bool,
    store: store::Store,
    sugg: Vec<suggest::Suggestion>,
    sel: Option<usize>,
    session: session::Session,
    find: findbar::FindBar,
    inspector: Inspector,
    dev: devmenu::DevMenu,
    ctl: browser_ctl::CtlState,
    /// Script dialogs, context menu and <select> popups.
    pageui: pageui::PageUi,
    /// View Source fetches running off the UI thread: (page URL, result).
    sources: Vec<(String, inbox::Slot<String>)>,
    prefs: prefs::Prefs,
    sheet: prefs::Sheet,
    /// Private window (`--private`): no history, no session, a throwaway profile deleted on exit.
    private: bool,
    /// A finger down on the page: (time, x, y, finger), for long-press.
    long: Option<(f64, f32, f32, u32)>,
    /// The long-press already fired: swallow the rest of that touch.
    swallow: bool,
    /// A tab is being dragged to a new place.
    drag: bool,
    /// System services (system.rs): the network state, the path request, a copy awaiting its answer, the pointer shape
    /// and the dock progress last sent.
    net: system::Network,
    net_request: Option<u32>,
    clip: Option<(u32, system::Clip)>,
    pointer: system::Pointer,
    dock: system::Dock,
    /// Downloads list and its popover.
    downloads: downloads::Downloads,
    uploads: uploads::Uploads,
    /// Bookmark popover, Favorites bar and the Bookmarks / History panel.
    lib: library::Library,
    /// Site icons for tabs, the address field and suggestions (favicon.rs).
    favicons: favicon::Favicons,
    /// Password AutoFill: which pages have a password field, and the request in flight (autofill.rs).
    autofill: autofill::AutoFill,
    /// Tab overview grid (overview.rs).
    overview: overview::Overview,
}

/// The bookmark star at the trailing edge of the address field (a square the height of the field).
fn star_rect(field: Rect) -> Rect {
    Rect { x: field.x + field.w - field.h, y: field.y, w: field.h, h: field.h }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}


/// The host of a URL without scheme, `www.` or path ("example.com"); non-URLs come back unchanged.
fn host_of(url: &str) -> String {
    let Some(host) = favicon::host_key(url) else { return url.to_string() };
    host.strip_prefix("www.").unwrap_or(&host).to_string()
}

/// The throwaway profile of this private window.
fn private_profile() -> PathBuf {
    home().join(format!(".cache/bifrost-browser/private-{}", std::process::id()))
}

/// Private profiles of windows that are gone (crashed or killed) are deleted at the next private launch.
fn remove_stale_private_profiles() {
    let Ok(dir) = fs::read_dir(home().join(".cache/bifrost-browser")) else { return };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(pid) = name.strip_prefix("private-").and_then(|p| p.parse::<u32>().ok()) {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
}


fn main() {
    app::run();
}

#[cfg(test)]
mod host_tests {
    use super::host_of;
    use super::{MENU_ACTUAL_SIZE, MENU_CLEAR_DOWNLOADS, MENU_DOWNLOADS, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREV, MENU_READER, MENU_SHARE, MENU_ZOOM_IN, MENU_ZOOM_OUT};
    use crate::tabdiscard::NAV_MENU_BASE;

    #[test]
    fn menu_ids_are_distinct_and_below_the_history_range() {
        let ids = [MENU_READER, MENU_SHARE, MENU_ZOOM_IN, MENU_ZOOM_OUT, MENU_ACTUAL_SIZE, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREV, MENU_DOWNLOADS, MENU_CLEAR_DOWNLOADS];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a > 0 && *a < NAV_MENU_BASE, "id {a} is outside 1..NAV_MENU_BASE");
            assert!(ids[i + 1..].iter().all(|b| b != a), "id {a} is used twice");
        }
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn address_host_drops_scheme_www_path_and_credentials() {
        assert_eq!(host_of("https://www.example.com/a?b#c"), "example.com");
        assert_eq!(host_of("http://user:pw@sub.example.com:8080/x"), "sub.example.com:8080");
        assert_eq!(host_of("https://example.com"), "example.com");
        assert_eq!(host_of("about:blank"), "about:blank");
        assert_eq!(host_of("hello world"), "hello world");
    }
}
