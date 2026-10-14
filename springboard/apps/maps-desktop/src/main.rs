//! Maps as an installed BifrOSt app: a Golden Gate window with a sidebar, the map under a unified toolbar, a place and
//! directions card, and the app's own menus, hosted through the SDK. The map model and every command live in the
//! library crate (`state`); this file only connects them to the shell.

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{network_path, App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input};
use bifrost_core::json::Json;
use bifrost_maps::geocode::{Geocoder, DEFAULT_BASE};
use bifrost_maps::net::{Live, Net, Offline};
use bifrost_maps::source::user_agent;
use bifrost_maps::state::{Act, MENU_COLLECTIONS};
use bifrost_maps::units::UnitsSetting;
use bifrost_maps::window::Window;
use bifrost_maps::rpc;
use bifrost_maps::route::Router;
use std::io::Read;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Largest GPX file read.
const GPX_LIMIT: u64 = 16 * 1024 * 1024;

struct MapsApp {
    w: Option<Window>,
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

/// The Go menu: finding places and routes, the saved places, and the collections. The collection slots are named after
/// the collections the person has made; a slot with no collection behind it is greyed out.
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
    for i in 0..MENU_COLLECTIONS {
        let title = w.maps.library.collections.get(i).map_or_else(|| format!("Collection {}", i + 1), |c| c.name.clone());
        items.push(sub(item(w, Act::AddToCollection(i), &title, "")));
    }
    items.push(AppMenuItem::separator());
    items.push(item(w, Act::ClearRecents, "Clear Recents", ""));
    items
}

impl MapsApp {
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.w.as_ref().is_some_and(|w| w.keyboard_shown());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
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
                self.w = Some(Window::new(&ctx.ui, w, h, ctx.data_dir.clone(), VERSION, self.net.clone()));
                self.path_request = Some(network_path::request_path(ctx));
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(w) = &mut self.w else { return false };
        let now = bifrost_app::proto::sys::now_ms();
        let handled = match ev {
            Input::Touch { kind, id, x, y, t } => {
                w.finger(ctx, &ctx.ui, id, kind, x, y, t);
                true
            }
            Input::Key(k) => w.key(ctx, k, now),
            Input::Activate(id) => w.activate(ctx, &ctx.ui, id, now),
            Input::Command(c) => w.command(c, now),
            Input::Context { x, y } => w.context(&ctx.ui, x, y),
            Input::Open(url) => w.open_link(ctx, &url, now),
            Input::ContextAction(_) => false,
            Input::QuickAction(_) => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let r = self.w.as_mut().is_some_and(|w| w.tick(ctx, &ctx.ui, t));
        self.sync_keyboard(ctx);
        r
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.as_ref().and_then(|w| w.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.w.as_ref().map(|w| w.controls(&ctx.ui)).unwrap_or_default()
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(w) = &self.w else { return Vec::new() };
        let sub = |it: AppMenuItem| AppMenuItem { depth: 1, ..it };
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
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
        let (Some(act), Some(w)) = (Act::from_menu_id(id), self.w.as_mut()) else { return false };
        let handled = w.act(ctx, act, bifrost_app::proto::sys::now_ms());
        self.sync_keyboard(ctx);
        handled
    }

    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        let Some(w) = &mut self.w else { return false };
        match ev {
            DocumentEvent::Opened(mut doc) if w.take_gpx_request(doc.request) => {
                let mut text = String::new();
                match (&mut doc.file).take(GPX_LIMIT).read_to_string(&mut text) {
                    Ok(_) => w.gpx_text(&text),
                    Err(e) => w.gpx_text(&format!("not readable: {e}")),
                }
                true
            }
            _ => false,
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.w.as_mut().is_some_and(|w| w.service_reply(ctx, request, result)) {
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
    bifrost_app::run(MapsApp { w: None, net: net(), kb: false, path_request: None });
}
