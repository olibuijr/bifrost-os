//! Maps as an installed BifrOSt app: a Golden Gate window with a sidebar, the map under a unified toolbar, a place and
//! directions card, and the app's own menus, hosted through the SDK. The map model and every command live in the
//! library crate (`state`); this file only connects them to the shell.

use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::dock_tile::{self, MenuItem};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{network_path, App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input};
use bifrost_core::json::Json;
use bifrost_maps::geocode::{Geocoder, DEFAULT_BASE};
use bifrost_maps::net::{Live, Net, Offline};
use bifrost_maps::source::user_agent;
use bifrost_maps::state::{Act, Shared, SharedData, MENU_COLLECTIONS};
use bifrost_maps::units::UnitsSetting;
use bifrost_maps::window::Window;
use bifrost_maps::rpc;
use bifrost_maps::route::Router;
use std::io::Read;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Largest GPX file read.
const GPX_LIMIT: u64 = 16 * 1024 * 1024;
/// The File > New Window item. Not a menu `Act`: the menu ids of `Act` stop before it.
const NEW_WINDOW_MENU: u32 = 10_000;
/// The greyed item Add Place to Collection holds while there are no collections. Not a menu `Act`.
const NO_COLLECTIONS_MENU: u32 = 10_001;
/// The Dock menu item that opens a window, chosen as `Input::ContextAction`.
const DOCK_NEW_WINDOW: u32 = 1;
/// The size of a window opened with New Window, in points.
const NEW_WINDOW_PT: (f32, f32) = (960.0, 640.0);

/// A window opened with New Window: its map state of its own, and the pixel size it was last laid out at.
struct ExtraWindow {
    id: WindowId,
    win: Window,
    px: (i32, i32),
}

struct MapsApp {
    w: Option<Window>,
    /// the windows the person opened, front to back of the app's registry; each one has its own map state
    extra: Vec<ExtraWindow>,
    /// the data every window shares (library, pins, units, areas), made when the first window is configured
    shared: Option<SharedData>,
    /// the window that has the focus: the one a touch last reached. Keys, commands, the toolbar and the menus go to it.
    focus: WindowId,
    net: Arc<dyn Net>,
    kb: bool,
    path_request: Option<u32>,
}

/// The network, except under the testing kit (`BIFROST_TEST_PIN=1`): offline, so a snapshot never shows live data.
fn net() -> Arc<dyn Net> {
    if bifrost_app::testing::pinned() { Arc::new(Offline) } else { Arc::new(Live::default()) }
}

fn item(w: &Window, act: Act, title: &str, key: &str) -> AppMenuItem {
    let id = act.menu_id().expect("menu items come from MENU_ACTS");
    let mut it = AppMenuItem::new(id, title).enabled(w.maps.can(act));
    if !key.is_empty() {
        it = it.key(key);
    }
    it.checked = w.maps.checked(act);
    it
}

/// The Go menu: finding places and routes, the saved places, and the collections. Add Place to Collection lists the
/// collections the person has made; with none it holds one greyed No Collections item, not placeholder slots named
/// "Collection 1/2/3" (E2E W4-15).
fn go_items(w: &Window) -> Vec<AppMenuItem> {
    let sub = |it: AppMenuItem| AppMenuItem { depth: 1, ..it };
    let mut items = vec![
        item(w, Act::Search, "Search", "cmd+f"),
        item(w, Act::Directions, "Directions", "cmd+r"),
        item(w, Act::StartNavigation, "Start Navigation", ""),
        item(w, Act::EndNavigation, "End Navigation", ""),
        AppMenuItem::separator(),
        item(w, Act::Locate, "Current Location", "cmd+l"),
        item(w, Act::DropPin, "Drop Pin", "cmd+shift+p"),
        item(w, Act::RemovePin, "Remove Pin", "cmd+delete"),
        AppMenuItem::separator(),
        item(w, Act::GoHome, "Go to Home", "cmd+shift+h"),
        item(w, Act::GoWork, "Go to Work", ""),
        item(w, Act::SetHome, "Set Home", ""),
        item(w, Act::SetWork, "Set Work", ""),
        AppMenuItem::separator(),
        item(w, Act::ToggleFavorite, "Favorite", "cmd+d"),
        item(w, Act::NewCollection, "New Collection", "cmd+shift+n"),
        AppMenuItem::submenu("Add Place to Collection"),
    ];
    let names: Vec<String> = w.maps.lib().collections.iter().take(MENU_COLLECTIONS).map(|c| c.name.clone()).collect();
    if names.is_empty() {
        items.push(sub(AppMenuItem::new(NO_COLLECTIONS_MENU, "No Collections").enabled(false)));
    }
    for (i, name) in names.iter().enumerate() {
        items.push(sub(item(w, Act::AddToCollection(i), name, "")));
    }
    items.push(AppMenuItem::separator());
    items.push(item(w, Act::ClearRecents, "Clear Recents", ""));
    items
}

impl MapsApp {
    /// The window with this id, the main window included.
    fn window(&self, id: WindowId) -> Option<&Window> {
        if id.is_main() { self.w.as_ref() } else { self.extra.iter().find(|e| e.id == id).map(|e| &e.win) }
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        if id.is_main() { self.w.as_mut() } else { self.extra.iter_mut().find(|e| e.id == id).map(|e| &mut e.win) }
    }

    /// The focused window: keys, commands, the toolbar and the menus act on it.
    fn focused_ref(&self) -> Option<&Window> {
        self.window(self.focus)
    }

    fn focused_mut(&mut self) -> Option<&mut Window> {
        let id = self.focus;
        self.window_mut(id)
    }

    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.focused_ref().is_some_and(|w| w.keyboard_shown());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
    }

    /// Opens a window with its own map state, which starts from the saved view and places like the main window.
    fn open_new_window(&mut self, ctx: &Ctx) -> bool {
        let (pw, ph) = NEW_WINDOW_PT;
        let Ok(id) = ctx.open_window(WindowSpec::new("Maps", pw, ph)) else { return false };
        let pt = ctx.ui.pt(1.0);
        let px = ((pw * pt) as i32, (ph * pt) as i32);
        let Some(shared) = self.shared.clone() else { return false };
        let win = Window::with_shared(&ctx.ui, px.0, px.1, ctx.data_dir.clone(), VERSION, self.net.clone(), shared);
        self.extra.push(ExtraWindow { id, win, px });
        // a new window is in front, so it has the focus
        self.focus = id;
        true
    }
}

impl App for MapsApp {
    fn id(&self) -> &str {
        "is.olibuijr.maps.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // places and settings are saved as they change
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.w {
            Some(win) => win.set_size(w, h),
            None => {
                let shared = Shared::open(&ctx.data_dir);
                self.shared = Some(shared.clone());
                self.w = Some(Window::with_shared(&ctx.ui, w, h, ctx.data_dir.clone(), VERSION, self.net.clone(), shared));
                self.path_request = Some(network_path::request_path(ctx));
                dock_tile::set_menu(ctx, &[MenuItem { id: DOCK_NEW_WINDOW, title: "New Window".into(), enabled: true }]);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if matches!(ev, Input::ContextAction(id) if id == DOCK_NEW_WINDOW) {
            return self.open_new_window(ctx);
        }
        // a touch on the main window gives it the focus
        if matches!(ev, Input::Touch { kind: bifrost_app::TouchKind::Down, .. }) {
            self.focus = WindowId::MAIN;
        }
        let now = bifrost_app::proto::sys::now_ms();
        let Some(w) = self.focused_mut() else { return false };
        let handled = match ev {
            Input::Touch { kind, id, x, y, t, .. } => {
                w.finger(ctx, &ctx.ui, id, kind, x, y, t);
                true
            }
            Input::Key(k) => w.key(ctx, k, now),
            Input::Activate(id) => w.activate(ctx, &ctx.ui, id, now),
            Input::Command(c) => w.command(c, now),
            // a Recents row has its own menu; anywhere else on the map a right click drops a pin
            Input::Context { x, y } => match w.row_context(&ctx.ui, x, y) {
                Some(items) => {
                    ctx.context_menu(x, y, items);
                    true
                }
                None => w.context(&ctx.ui, x, y),
            },
            Input::Open(url) => w.open_link(ctx, &url, now),
            Input::ContextAction(id) => w.context_action(id),
            Input::QuickAction(_) => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut r = self.w.as_mut().is_some_and(|w| w.tick(ctx, &ctx.ui, t));
        for e in &mut self.extra {
            r |= e.win.tick(ctx, &ctx.ui, t);
        }
        self.sync_keyboard(ctx);
        r
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let main = self.w.as_ref().and_then(|w| w.next_wake(t));
        self.extra.iter().filter_map(|e| e.win.next_wake(t)).chain(main).reduce(f64::min)
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        // a touch on a window gives it the focus; keys, commands and the menus then go to it
        if matches!(ev, Input::Touch { kind: bifrost_app::TouchKind::Down, .. }) {
            self.focus = window;
        }
        let Some(e) = self.extra.iter_mut().find(|e| e.id == window) else { return false };
        match ev {
            // an extra window's touches arrive in window points (`WindowTouch`); `finger` takes px
            Input::Touch { kind, id, x, y, t, .. } => {
                let k = ctx.ui.pt(1.0);
                e.win.finger(ctx, &ctx.ui, id, kind, x * k, y * k, t);
                true
            }
            // keys, commands and the menu bar belong to the main window
            _ => false,
        }
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        if let Some(e) = self.extra.iter_mut().find(|e| e.id == window) {
            let size = (s.w as i32, s.h as i32);
            if e.px != size {
                e.win.set_size(size.0, size.1);
                e.px = size;
            }
            e.win.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn window_closed(&mut self, ctx: &Ctx, window: WindowId) -> bool {
        let Some(at) = self.extra.iter().position(|e| e.id == window) else { return false };
        let mut gone = self.extra.remove(at);
        gone.win.leave(ctx);
        if self.focus == window {
            self.focus = WindowId::MAIN;
        }
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.focused_ref().map(|w| w.controls(&ctx.ui)).unwrap_or_default()
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(w) = self.focused_ref() else { return Vec::new() };
        let sub = |it: AppMenuItem| AppMenuItem { depth: 1, ..it };
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(NEW_WINDOW_MENU, "New Window").key("cmd+n"),
                    AppMenuItem::separator(),
                    item(w, Act::OpenGpx, "Open GPX File\u{2026}", "cmd+o"),
                    AppMenuItem::separator(),
                    AppMenuItem::submenu("Share"),
                    sub(item(w, Act::CopyLink, "Copy Link", "cmd+shift+c")),
                    sub(item(w, Act::CopyCoordinates, "Copy Coordinates", "cmd+alt+c")),
                    sub(item(w, Act::Share, "Share\u{2026}", "")),
                    AppMenuItem::separator(),
                    item(w, Act::Print, "Print\u{2026}", "cmd+p"),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    item(w, Act::MapType(bifrost_maps::source::MapType::Standard), "Map", "cmd+1"),
                    item(w, Act::MapType(bifrost_maps::source::MapType::Satellite), "Satellite", "cmd+2"),
                    item(w, Act::MapType(bifrost_maps::source::MapType::Hybrid), "Hybrid", "cmd+3"),
                    item(w, Act::ToggleTraffic, "Show Traffic", "cmd+t"),
                    AppMenuItem::separator(),
                    item(w, Act::ZoomIn, "Zoom In", "cmd++"),
                    item(w, Act::ZoomOut, "Zoom Out", "cmd+-"),
                    AppMenuItem::separator(),
                    AppMenuItem::submenu("Distance Units"),
                    sub(item(w, Act::Units(UnitsSetting::Automatic), "Automatic", "")),
                    sub(item(w, Act::Units(UnitsSetting::Kilometres), "Kilometres", "")),
                    sub(item(w, Act::Units(UnitsSetting::Miles), "Miles", "")),
                    AppMenuItem::separator(),
                    item(w, Act::Measure, "Measure Distance", "cmd+shift+m"),
                    item(w, Act::Download, "Download Map\u{2026}", "cmd+shift+d"),
                ],
            },
            AppMenu { title: "Go".into(), items: go_items(w) },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == NEW_WINDOW_MENU {
            return self.open_new_window(ctx);
        }
        let (Some(act), Some(w)) = (Act::from_menu_id(id), self.focused_mut()) else { return false };
        let handled = w.act(ctx, act, bifrost_app::proto::sys::now_ms());
        self.sync_keyboard(ctx);
        handled
    }

    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        let DocumentEvent::Opened(mut doc) = ev else { return false };
        let request = doc.request;
        // the window that asked for the file gets it
        let owner = if self.w.as_mut().is_some_and(|w| w.take_gpx_request(request)) {
            WindowId::MAIN
        } else {
            match self.extra.iter_mut().find_map(|e| if e.win.take_gpx_request(request) { Some(e.id) } else { None }) {
                Some(id) => id,
                None => return false,
            }
        };
        let mut text = String::new();
        let body = match (&mut doc.file).take(GPX_LIMIT).read_to_string(&mut text) {
            Ok(_) => text,
            Err(e) => format!("not readable: {e}"),
        };
        if let Some(w) = self.window_mut(owner) {
            w.gpx_text(&body);
        }
        true
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.w.as_mut().is_some_and(|w| w.service_reply(ctx, request, result)) {
            return true;
        }
        if self.extra.iter_mut().any(|e| e.win.service_reply(ctx, request, result)) {
            return true;
        }
        if Some(request) != self.path_request {
            return false;
        }
        // an error (for example an older shell) keeps the app online
        if let (Ok(path), Some(w)) = (network_path::decode_reply(result.clone()), self.w.as_mut()) {
            w.set_online(path.status == network_path::Status::Satisfied);
        }
        true
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        if self.w.as_mut().is_some_and(|w| w.service_event(ctx, service, event, body)) {
            return true;
        }
        if self.extra.iter_mut().any(|e| e.win.service_event(ctx, service, event, body)) {
            return true;
        }
        match (network_path::path_event(service, event, body), self.w.as_mut()) {
            (Some(Ok(path)), Some(w)) => {
                w.set_online(path.status == network_path::Status::Satisfied);
                true
            }
            _ => false,
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        rpc::methods()
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, params: &Json) -> Result<Json, bifrost_app::control::RpcError> {
        let agent = user_agent(VERSION);
        match method {
            "maps.search" => {
                let geocoder = Geocoder::new(DEFAULT_BASE, &agent).map_err(|e| bifrost_app::control::RpcError::new(bifrost_app::control::INTERNAL_ERROR, e))?;
                rpc::search(&*self.net, &geocoder, params)
            }
            "maps.route" => {
                // the routing server the person set in settings.json, if any
                let base = std::fs::read_to_string(data_dir.join("settings.json")).ok().and_then(|t| bifrost_core::json::parse(&t).ok()).and_then(|j| j.str_field("routing_url"));
                let router = Router::new(base.as_deref()).unwrap_or_else(|_| Router::new(None).expect("no routing server is valid"));
                rpc::route(&*self.net, &router, &agent, params)
            }
            _ => Err(bifrost_app::control::RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(w) = &mut self.w {
            w.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(MapsApp { w: None, extra: Vec::new(), shared: None, focus: WindowId::MAIN, net: net(), kb: false, path_request: None });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_maps::state::MENU_ACTS;

    #[test]
    fn new_window_menu_id_is_not_a_map_action() {
        assert!(MENU_ACTS.len() < NEW_WINDOW_MENU as usize);
        assert!(Act::from_menu_id(NEW_WINDOW_MENU).is_none());
        assert!(Act::from_menu_id(NO_COLLECTIONS_MENU).is_none());
    }
}
