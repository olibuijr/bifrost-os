//! The Maps window: a Golden Gate sidebar (search, Home and Work, Favorites, Collections, Recents), the map under a
//! unified toolbar, round map controls, and a card for the place, the directions, the measurement or the download.
//! Layout is computed from the window size in points; `buttons` is the one list that drawing, hit testing and the
//! accessibility controls all read, so they cannot disagree.

use crate::loader::TileLoader;
use crate::net::Net;
use crate::route::{self, Route};
use crate::source::{MapType, TRAFFIC_MAX_AGE};
use crate::state::{Act, Effect, Here, Maps, Panel};
use crate::tiles::{TileId, TILE_SIZE};
use crate::units::{metres_per_pt, scale_bar};
use bifrost_app::controls::location_auth::RequestOutcome;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Command, Node, Rect as KRect, Role};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LINK, SEPARATOR_C, SIDEBAR_BG, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED, SYSTEM_TEAL, WHITE};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE3_PT, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS};
use bifrost_app::kit::{Image, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::core_location::{AuthorizationStatus, Delegate, Location, LocationError, LocationManager};
use bifrost_app::{privacy_permissions, widgets, Ctx, KeyInput};
use bifrost_core::json::Json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SIDEBAR_PT: f32 = 260.0;
const COMPACT_PT: f32 = 640.0;
const ROW_PT: f32 = 32.0;
const FIELD_H: f32 = 32.0;
const MARGIN: f32 = 12.0;
const CARD_W: f32 = 300.0;
const PLATE: f32 = 32.0;
const BUTTON_H: f32 = 30.0;
/// Pixels a finger or pointer may move and still be a tap.
const TAP_SLOP_PT: f32 = 4.0;
/// Two taps within this time (ms) and distance (points) are a double tap.
const DOUBLE_TAP_MS: f64 = 350.0;
const DOUBLE_TAP_PT: f32 = 20.0;
/// Pan step of an arrow key, points.
const KEY_PAN_PT: f64 = 60.0;
const RECENTS_SHOWN: usize = 8;
const STEPS_SHOWN: usize = 80;

const HOME_ID: u32 = 1;
const WORK_ID: u32 = 2;
const FAV_ID: u32 = 100;
const COLL_ID: u32 = 200;
const REC_ID: u32 = 300;
const SUG_ID: u32 = 400;

/// How long (ms) the Allow sheet of the location permission is waited for.
const ALLOW_WAIT_MS: f64 = 90_000.0;
/// Pause (ms) between two looks at the permission while the sheet is up.
const STATUS_POLL_MS: f64 = 1_500.0;


#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    Act(Act),
    Row(u32),
    Search,
}

#[derive(Clone, Debug)]
struct Btn {
    id: u64,
    rect: KRect,
    label: String,
    role: Role,
    what: What,
    enabled: bool,
    selected: bool,
    /// drawn by the kit (toolbar, search field, sidebar row) rather than by the card or the map plates
    kit: bool,
}

/// One line or control of the card, laid out top to bottom.
#[derive(Clone, Debug)]
enum Item {
    Title(String),
    Sub(String),
    Line(String),
    Gap(f32),
    Rule,
    Buttons(Vec<(String, Act, bool)>),
    Route { index: usize, line: String, tag: String, selected: bool },
    Step { text: String, distance: String },
    Progress(f32),
}

impl Item {
    fn height(&self) -> f32 {
        match self {
            Item::Title(_) => 28.0,
            Item::Sub(_) | Item::Line(_) => 20.0,
            Item::Gap(h) => *h,
            Item::Rule => 9.0,
            Item::Buttons(_) => BUTTON_H + 8.0,
            Item::Route { .. } => 46.0,
            Item::Step { .. } => 32.0,
            Item::Progress(_) => 14.0,
        }
    }
}

enum Done {
    Search { query: String, result: Result<Vec<crate::geocode::Place>, String> },
    Route(Result<Vec<Route>, String>),
    Printed(Result<String, String>),
}

struct Drag {
    start: (f32, f32),
    last: (f32, f32),
    moved: bool,
    in_card: bool,
}

/// What a pending call to the location or privacy service was for.
#[derive(Clone, Copy, PartialEq)]
enum LocCall {
    /// the permission request: its reply says whether to ask the person, go on, or stop
    Authorize,
    /// anything else: a status, a position or a start or stop; the location manager reads the reply
    Other,
}

/// What the location manager reported, collected while it handles one reply or event.
#[derive(Default)]
struct Sink {
    updates: Vec<Location>,
    errors: Vec<LocationError>,
    statuses: Vec<AuthorizationStatus>,
}

impl Delegate for Sink {
    fn did_update_locations(&mut self, locations: &[Location]) {
        self.updates.extend_from_slice(locations);
    }
    fn did_fail_with_error(&mut self, error: LocationError) {
        self.errors.push(error);
    }
    fn did_change_authorization(&mut self, status: AuthorizationStatus) {
        self.statuses.push(status);
    }
}

/// The sentence for a location failure.
fn location_error_text(error: LocationError) -> &'static str {
    match error {
        LocationError::Denied | LocationError::PromptDeclined => "Maps may not use your location. Allow it in Settings, Privacy, App Permissions.",
        LocationError::LocationUnknown => "Your location is not known. Set one in Settings.",
        LocationError::Network => "Your location could not be looked up.",
        _ => "Your location is not available.",
    }
}

/// Draws a map tile at its place, scaled to the size on screen (the scaled copies are kept in `scaled`).
fn blit_tile(c: &mut Surface, scaled: &mut HashMap<(TileId, usize), Image>, id: TileId, img: &Image, x: i32, y: i32, tile_px: usize) {
    if img.w == tile_px && img.h == tile_px {
        c.blit(img, x, y, 255, 255);
        return;
    }
    if scaled.len() > 200 {
        scaled.clear();
    }
    let entry = scaled.entry((id, tile_px)).or_insert_with(|| img.resized(tile_px.max(1), tile_px.max(1)));
    c.blit(entry, x, y, 255, 255);
}

pub struct Window {
    pub maps: Maps,
    loader: TileLoader,
    net: Arc<dyn Net>,
    done: Arc<Mutex<Vec<Done>>>,
    /// the window in pixels
    w: i32,
    h: i32,
    pt_px: f32,
    focused: bool,
    drag: Option<Drag>,
    last_tap: Option<(f32, f32, f64)>,
    pressed: Option<u64>,
    scaled: HashMap<(TileId, usize), Image>,
    labels: TextCache,
    card_scroll: f32,
    gpx_request: Option<u32>,
    share_request: Option<u32>,
    dirty: bool,
    /// the traffic layer's tiles, when `traffic_url` is set
    traffic: Option<TileLoader>,
    scaled_traffic: HashMap<(TileId, usize), Image>,
    /// whether the traffic layer was on at the last tick, to ask for tiles again when it is switched on
    traffic_on: bool,
    loc: LocationManager,
    loc_calls: Vec<(u32, LocCall)>,
    /// the person is allowed to share their location with the app
    granted: bool,
    /// positions are wanted continuously (a route is followed)
    tracking: bool,
    /// while the Allow sheet is up: when to stop waiting, and when to look at the permission next
    allow_until: Option<f64>,
    next_status_poll: f64,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> KRect {
    KRect { x, y, w, h }
}

fn bar_table() -> Vec<(u32, &'static str, &'static str, Act, u32)> {
    vec![
        (1, "Directions", "arrow.triangle.turn.up.right.diamond", Act::Directions, 0),
        (2, "Drop Pin", "mappin", Act::DropPin, 0),
        (3, "Current Location", "location", Act::Locate, 0),
        (10, "Map", "map", Act::MapType(MapType::Standard), 1),
        (11, "Satellite", "globe", Act::MapType(MapType::Satellite), 1),
        (20, "Measure Distance", "ruler", Act::Measure, 0),
        (21, "Download Map", "arrow.down.circle", Act::Download, 0),
        (22, "Share", "square.and.arrow.up", Act::Share, 0),
    ]
}

impl Window {
    pub fn new(ui: &Ui, w: i32, h: i32, root: PathBuf, version: &str, net: Arc<dyn Net>) -> Window {
        let pt = ui.pt(1.0).max(0.01);
        let maps = Maps::new(&root, version, (w as f32 / pt) as f64, (h as f32 / pt) as f64);
        let loader = TileLoader::new(&root, maps.map_type.source(version), net.clone());
        let traffic = maps.traffic_source().cloned().map(|source| TileLoader::overlay(&root, source, net.clone(), TRAFFIC_MAX_AGE));
        Window {
            maps,
            loader,
            net,
            done: Arc::new(Mutex::new(Vec::new())),
            w,
            h,
            pt_px: pt,
            focused: false,
            drag: None,
            last_tap: None,
            pressed: None,
            scaled: HashMap::new(),
            labels: TextCache::default(),
            card_scroll: 0.0,
            gpx_request: None,
            share_request: None,
            dirty: true,
            traffic,
            scaled_traffic: HashMap::new(),
            traffic_on: false,
            loc: LocationManager::new(),
            loc_calls: Vec::new(),
            granted: false,
            tracking: false,
            allow_until: None,
            next_status_poll: 0.0,
        }
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    pub fn keyboard_shown(&self) -> bool {
        self.focused
    }

    fn pt(&self) -> f32 {
        self.pt_px
    }

    // ---- geometry, in points ----

    fn size_pt(&self) -> (f32, f32) {
        (self.w as f32 / self.pt(), self.h as f32 / self.pt())
    }

    fn side_pt(&self) -> f32 {
        if self.size_pt().0 < COMPACT_PT { 0.0 } else { SIDEBAR_PT }
    }

    /// The map area in window points.
    fn map_rect(&self) -> KRect {
        let (w, h) = self.size_pt();
        let side = self.side_pt();
        rect(side, UNIFIED_BAR_PT, (w - side).max(1.0), (h - UNIFIED_BAR_PT).max(1.0))
    }

    /// The search field: at the top of the sidebar, or floating over the map without one.
    fn search_rect(&self) -> KRect {
        if self.side_pt() > 0.0 {
            rect(SPACE_M, UNIFIED_BAR_PT, SIDEBAR_PT - 2.0 * SPACE_M, FIELD_H)
        } else {
            let m = self.map_rect();
            rect(m.x + MARGIN, m.y + MARGIN, (m.w - 2.0 * MARGIN).min(CARD_W), FIELD_H)
        }
    }

    /// The list under the search field.
    fn list_rect(&self) -> KRect {
        let s = self.search_rect();
        if self.side_pt() > 0.0 {
            let top = s.y + s.h + SPACE_S;
            rect(0.0, top, SIDEBAR_PT, (self.size_pt().1 - top).max(0.0))
        } else {
            let top = s.y + s.h + 6.0;
            rect(s.x, top, s.w, ((self.size_pt().1 - top - MARGIN).min(ROW_PT * 6.0)).max(0.0))
        }
    }

    fn card_rect(&self, content_h: f32) -> KRect {
        let m = self.map_rect();
        let w = CARD_W.min(m.w - 2.0 * MARGIN);
        let top = if self.side_pt() > 0.0 { m.y + MARGIN } else { self.search_rect().y + FIELD_H + MARGIN };
        let bottom_room = m.y + m.h - 2.0 * PLATE - 3.0 * MARGIN - top;
        rect(m.x + m.w - w - MARGIN, top, w, (content_h + 2.0 * MARGIN).min(bottom_room.max(80.0)))
    }

    fn plate_rects(&self) -> [KRect; 2] {
        let m = self.map_rect();
        let x = m.x + m.w - MARGIN - PLATE;
        let bottom = m.y + m.h - MARGIN - 20.0;
        [rect(x, bottom - 2.0 * PLATE - 1.0, PLATE, PLATE), rect(x, bottom - PLATE, PLATE, PLATE)]
    }

    // ---- sidebar and suggestions ----

    /// The sidebar's rows: suggestions while there is a query, else the saved places. The second list holds each
    /// item's row id (0 for a header).
    fn side_items(&self) -> (Vec<SidebarItem>, Vec<u32>) {
        let mut items = Vec::new();
        let mut ids = Vec::new();
        let mut push = |item: SidebarItem, id: u32| {
            items.push(item);
            ids.push(id);
        };
        let lib = &self.maps.library;
        if !self.maps.query.is_empty() {
            let sug = self.maps.suggestions();
            let head = if sug.is_empty() {
                if self.maps.searching { "Searching\u{2026}".to_string() } else { self.maps.note.clone().unwrap_or_else(|| "Press Return to search".to_string()) }
            } else {
                "Places".to_string()
            };
            push(SidebarItem::header(&head), 0);
            for (i, s) in sug.iter().enumerate() {
                let symbol = if s.local { "star" } else { "mappin" };
                let mut row = SidebarItem::row(SUG_ID + i as u32, &s.title).with_symbol(symbol);
                if !s.subtitle.is_empty() {
                    row = row.with_subtitle(&s.subtitle);
                }
                push(row, SUG_ID + i as u32);
            }
            return (items, ids);
        }
        push(SidebarItem::header("Places"), 0);
        let home = lib.home.as_ref().map_or("Not set".to_string(), |p| p.title().to_string());
        let work = lib.work.as_ref().map_or("Not set".to_string(), |p| p.title().to_string());
        push(SidebarItem::row(HOME_ID, "Home").with_symbol("house").with_subtitle(&home), HOME_ID);
        push(SidebarItem::row(WORK_ID, "Work").with_symbol("briefcase").with_subtitle(&work), WORK_ID);
        if !lib.favorites.is_empty() {
            push(SidebarItem::header("Favorites"), 0);
            for (i, p) in lib.favorites.iter().enumerate() {
                push(SidebarItem::row(FAV_ID + i as u32, p.title()).with_symbol("star"), FAV_ID + i as u32);
            }
        }
        if !lib.collections.is_empty() {
            push(SidebarItem::header("Collections"), 0);
            for (i, c) in lib.collections.iter().enumerate() {
                push(SidebarItem::row(COLL_ID + i as u32, &c.name).with_symbol("folder").with_count(c.places.len() as u32), COLL_ID + i as u32);
            }
        }
        if !lib.recents.is_empty() {
            push(SidebarItem::header("Recents"), 0);
            for (i, p) in lib.recents.iter().take(RECENTS_SHOWN).enumerate() {
                push(SidebarItem::row(REC_ID + i as u32, p.title()).with_symbol("clock"), REC_ID + i as u32);
            }
        }
        (items, ids)
    }

    fn side_list(&self) -> (SourceList, Vec<u32>) {
        let (items, ids) = self.side_items();
        (SourceList::new(items, ROW_PT, ROW_PT), ids)
    }

    /// Runs a sidebar or suggestion row.
    fn row_activate(&mut self, id: u32) {
        match id {
            HOME_ID => {
                if self.maps.library.home.is_some() {
                    self.maps.act(Act::GoHome, 0);
                } else {
                    self.maps.note = Some("Choose a place, then Set Home in the Go menu".to_string());
                }
            }
            WORK_ID => {
                if self.maps.library.work.is_some() {
                    self.maps.act(Act::GoWork, 0);
                } else {
                    self.maps.note = Some("Choose a place, then Set Work in the Go menu".to_string());
                }
            }
            i if i >= SUG_ID => {
                if let Some(s) = self.maps.suggestions().into_iter().nth((i - SUG_ID) as usize) {
                    self.maps.choose(s.place);
                    self.focused = false;
                }
            }
            i if i >= REC_ID => {
                if let Some(p) = self.maps.library.recents.get((i - REC_ID) as usize).cloned() {
                    self.maps.show_place(p.to_place());
                }
            }
            i if i >= COLL_ID => self.maps.show_collection((i - COLL_ID) as usize),
            i if i >= FAV_ID => {
                if let Some(p) = self.maps.library.favorites.get((i - FAV_ID) as usize).cloned() {
                    self.maps.show_place(p.to_place());
                }
            }
            _ => {}
        }
    }

    // ---- toolbar ----

    fn bar(&self) -> Toolbar {
        let mut items = Vec::new();
        for (id, label, symbol, act, group) in bar_table() {
            if id == 10 {
                items.push(ToolbarItem::flexible_space());
            }
            let mut it = ToolbarItem::icon(id, label, symbol);
            it.group = group;
            it.selected = group != 0 && self.maps.checked(act);
            it.enabled = self.maps.can(act);
            items.push(it);
        }
        Toolbar::new(items)
    }

    fn bar_bounds(&self) -> KRect {
        let side = self.side_pt();
        let x = if side == 0.0 { traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT } else { SPACE_M };
        rect(x, 0.0, (self.size_pt().0 - side - x - SPACE_M).max(0.0), UNIFIED_BAR_PT)
    }

    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        let h = widgets::CAPSULE_PT;
        let b = self.bar_bounds();
        self.bar().layout_with(KRect { y: b.y + (b.h - h) / 2.0, h, ..b }, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    // ---- the card ----

    fn card_items(&self) -> Vec<Item> {
        let m = &self.maps;
        let mut v = Vec::new();
        match m.panel {
            Panel::None => {}
            Panel::Place => {
                let Some(p) = &m.selected else { return v };
                v.push(Item::Title(p.title().to_string()));
                let sub = if !p.details.kind.is_empty() { p.details.kind.clone() } else { p.name.split(',').skip(1).collect::<Vec<_>>().join(",").trim().to_string() };
                if !sub.is_empty() {
                    v.push(Item::Sub(sub));
                }
                for line in &p.details.address {
                    v.push(Item::Line(line.clone()));
                }
                for (label, value) in [("Phone", &p.details.phone), ("Website", &p.details.website), ("Hours", &p.details.opening_hours)] {
                    if !value.is_empty() {
                        v.push(Item::Line(format!("{label}: {value}")));
                    }
                }
                v.push(Item::Sub(crate::links::coordinates_text(p.lat, p.lon)));
                v.push(Item::Gap(4.0));
                let fav = m.checked(Act::ToggleFavorite);
                v.push(Item::Buttons(vec![
                    ("Directions".into(), Act::Directions, false),
                    ((if fav { "Unfavorite" } else { "Favorite" }).to_string(), Act::ToggleFavorite, fav),
                    ("Share".into(), Act::Share, false),
                ]));
                let pin = if m.selected_pin.is_some() { ("Remove Pin", Act::RemovePin) } else { ("Drop Pin", Act::DropPin) };
                v.push(Item::Buttons(vec![(pin.0.into(), pin.1, false), ("Copy Link".into(), Act::CopyLink, false), ("Print".into(), Act::Print, false)]));
                v.push(Item::Buttons(vec![("Set Home".into(), Act::SetHome, false), ("Set Work".into(), Act::SetWork, false), ("New Collection".into(), Act::NewCollection, false)]));
                let colls: Vec<(String, Act, bool)> = m
                    .library
                    .collections
                    .iter()
                    .enumerate()
                    .take(3)
                    .map(|(i, c)| (c.name.clone(), Act::AddToCollection(i), c.places.iter().any(|q| (q.lat - p.lat).abs() < 2e-4 && (q.lon - p.lon).abs() < 2e-4)))
                    .collect();
                if !colls.is_empty() {
                    v.push(Item::Sub("Add to a collection".into()));
                    v.push(Item::Buttons(colls));
                }
            }
            Panel::Directions => {
                v.push(Item::Title("Directions".into()));
                if let Some((n, _)) = &m.route_from {
                    v.push(Item::Line(format!("From: {n}")));
                }
                if let Some((n, _)) = &m.route_to {
                    v.push(Item::Line(format!("To: {n}")));
                }
                v.push(Item::Buttons(vec![("Swap".into(), Act::SwapEnds, false), ("Print".into(), Act::Print, false), ("Copy Link".into(), Act::CopyLink, false)]));
                v.push(Item::Buttons(
                    [(route::Profile::Driving, "Drive"), (route::Profile::Walking, "Walk"), (route::Profile::Cycling, "Cycle"), (route::Profile::Transit, "Transit")]
                        .into_iter()
                        .map(|(p, l)| (l.to_string(), Act::Profile(p), m.profile == p))
                        .collect(),
                ));
                if m.can(Act::StartNavigation) {
                    v.push(Item::Buttons(vec![("Start Navigation".into(), Act::StartNavigation, false)]));
                }
                if m.routing {
                    v.push(Item::Sub("Finding routes\u{2026}".into()));
                }
                if let Some(n) = &m.note {
                    v.push(Item::Sub(n.clone()));
                }
                for (i, r) in m.routes.iter().enumerate() {
                    let tag = if r.approximate { "Estimate" } else if i == 0 { "Fastest" } else { "Alternative" };
                    v.push(Item::Route { index: i, line: m.route_line(r), tag: tag.into(), selected: i == m.route_index });
                }
                if let Some(r) = m.current_route() {
                    if !r.steps.is_empty() {
                        v.push(Item::Rule);
                    }
                    for s in r.steps.iter().take(STEPS_SHOWN) {
                        v.push(Item::Step { text: s.instruction(), distance: m.distance_text(s.distance_m) });
                    }
                }
            }
            Panel::Navigation => {
                v.push(Item::Title(m.nav_title()));
                v.push(Item::Sub(m.nav_subtitle()));
                if let Some(remaining) = m.nav_remaining() {
                    v.push(Item::Line(remaining));
                }
                if let Some(call) = &m.announcement {
                    v.push(Item::Line(call.clone()));
                }
                if let Some(n) = &m.note {
                    v.push(Item::Sub(n.clone()));
                }
                v.push(Item::Gap(4.0));
                v.push(Item::Buttons(vec![("End Navigation".into(), Act::EndNavigation, false)]));
            }
            Panel::Measure => {
                v.push(Item::Title("Measure Distance".into()));
                v.push(Item::Sub(m.measure_line()));
                v.push(Item::Gap(4.0));
                v.push(Item::Buttons(vec![("Undo".into(), Act::MeasureUndo, false), ("Clear".into(), Act::MeasureClear, false), ("Done".into(), Act::Measure, false)]));
            }
            Panel::Download => {
                v.push(Item::Title("Download Map".into()));
                v.push(Item::Sub(m.download_line()));
                if let Some((d, t)) = m.progress {
                    v.push(Item::Progress(d as f32 / t.max(1) as f32));
                    v.push(Item::Buttons(vec![("Cancel".into(), Act::CancelDownload, false)]));
                } else {
                    v.push(Item::Gap(4.0));
                    v.push(Item::Buttons(vec![("Download".into(), Act::StartDownload, false), ("Done".into(), Act::ClosePanel, false)]));
                }
            }
        }
        v
    }

    /// The card's items with their top offsets (points, from the card's content top) and the total height.
    fn flow(items: &[Item]) -> (Vec<(f32, f32)>, f32) {
        let mut y = 0.0;
        let placed = items
            .iter()
            .map(|i| {
                let at = (y, i.height());
                y += i.height();
                at
            })
            .collect();
        (placed, y)
    }

    // ---- the one list of controls ----

    fn buttons(&self, ui: &Ui) -> Vec<Btn> {
        let mut out: Vec<Btn> = Vec::new();
        let mut push = |rect: KRect, label: String, role: Role, what: What, enabled: bool, selected: bool, kit: bool| {
            let id = out.len() as u64 + 1;
            out.push(Btn { id, rect, label, role, what, enabled, selected, kit });
        };
        // the toolbar
        let side = self.side_pt();
        let bar = self.bar();
        let table = bar_table();
        for (id, r) in self.bar_layout(ui).visible {
            if let (Some(it), Some((_, _, _, act, _))) = (bar.items.iter().find(|i| i.id == id), table.iter().find(|t| t.0 == id)) {
                push(KRect { x: r.x + side, ..r }, it.label.clone(), Role::Button, What::Act(*act), it.enabled, it.selected, true);
            }
        }
        // the search field and its rows
        push(self.search_rect(), "Search".into(), Role::TextField, What::Search, true, false, true);
        let (list, ids) = self.side_list();
        let bounds = self.list_rect();
        if side > 0.0 || !self.maps.query.is_empty() {
            for (i, id) in ids.iter().enumerate() {
                if *id == 0 {
                    continue;
                }
                if let Some(SidebarItem::Row { label, .. }) = list.items.get(i) {
                    push(list.item_rect(i, bounds, 0.0), label.clone(), Role::Row, What::Row(*id), true, false, true);
                }
            }
        }
        // the map plates
        let [zin, zout] = self.plate_rects();
        push(zin, "Zoom In".into(), Role::Button, What::Act(Act::ZoomIn), true, false, false);
        push(zout, "Zoom Out".into(), Role::Button, What::Act(Act::ZoomOut), true, false, false);
        // the card
        let items = self.card_items();
        if !items.is_empty() {
            let (placed, total) = Self::flow(&items);
            let card = self.card_rect(total + 32.0);
            push(rect(card.x + card.w - 34.0, card.y + 6.0, 28.0, 28.0), "Close".into(), Role::Button, What::Act(Act::ClosePanel), true, false, false);
            let base = card.y + MARGIN - self.card_scroll;
            for (item, (y, h)) in items.iter().zip(placed) {
                let y = base + y;
                if y + h < card.y || y > card.y + card.h {
                    continue;
                }
                match item {
                    Item::Buttons(list) => {
                        let n = list.len() as f32;
                        let gap = 6.0;
                        let bw = (card.w - 2.0 * MARGIN - gap * (n - 1.0)) / n;
                        for (k, (label, act, selected)) in list.iter().enumerate() {
                            let enabled = self.maps.can(*act);
                            push(rect(card.x + MARGIN + k as f32 * (bw + gap), y, bw, BUTTON_H), label.clone(), Role::Button, What::Act(*act), enabled, *selected, false);
                        }
                    }
                    Item::Route { index, line, tag, selected } => {
                        push(rect(card.x + MARGIN, y, card.w - 2.0 * MARGIN, h - 4.0), format!("{tag}, {line}"), Role::Button, What::Act(Act::PickRoute(*index)), true, *selected, false);
                    }
                    _ => {}
                }
            }
        }
        out
    }

    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        self.buttons(ui)
            .into_iter()
            .map(|b| {
                let value = if b.role == Role::TextField { self.maps.query.to_string() } else if b.selected { "selected".to_string() } else { String::new() };
                Node { id: b.id, role: b.role, label: b.label, value, bounds: b.rect, enabled: b.enabled }
            })
            .collect()
    }

    // ---- input ----

    fn run(&mut self, ctx: &Ctx, what: What, now: f64) {
        let ms = now as u64;
        match what {
            What::Act(a) => {
                let fx = self.maps.act(a, ms);
                self.card_scroll = 0.0;
                self.effects(ctx, fx);
            }
            What::Row(id) => self.row_activate(id),
            What::Search => {
                self.focused = true;
            }
        }
        self.maps.persist();
        self.dirty = true;
    }

    /// Activates accessibility control `id`.
    pub fn activate(&mut self, ctx: &Ctx, ui: &Ui, id: u64, now: f64) -> bool {
        let Some(b) = self.buttons(ui).into_iter().find(|b| b.id == id && b.enabled) else { return false };
        self.run(ctx, b.what, now);
        true
    }

    fn to_map_pt(&self, x: f32, y: f32) -> (f32, f32) {
        let m = self.map_rect();
        (x - m.x, y - m.y)
    }

    fn pin_at(&self, mx: f32, my: f32) -> Option<usize> {
        self.maps.pins.visible(&self.maps.view).into_iter().rev().find(|&(_, x, y)| (x as f32 - mx).abs() < 14.0 && (y as f32 - 9.0 - my).abs() < 16.0).map(|(i, _, _)| i)
    }

    pub fn touch(&mut self, ctx: &Ctx, ui: &Ui, kind: bifrost_app::TouchKind, px: f32, py: f32, t: f64) {
        use bifrost_app::TouchKind::*;
        self.pt_px = ui.pt(1.0).max(0.01);
        let (x, y) = (px / self.pt(), py / self.pt());
        self.dirty = true;
        match kind {
            Down => {
                self.maps.note = None;
                let hit = self.buttons(ui).into_iter().rev().find(|b| b.rect.contains(x, y));
                match hit {
                    Some(b) => {
                        self.pressed = Some(b.id);
                        self.drag = None;
                        if !matches!(b.what, What::Search) {
                            // a press elsewhere ends typing, like a click outside the field in any app
                            self.focused = false;
                        }
                    }
                    None => {
                        self.focused = false;
                        let card_h = self.card_height();
                        let in_card = card_h > 0.0 && self.card_rect(card_h).contains(x, y);
                        if in_card || self.map_rect().contains(x, y) {
                            self.drag = Some(Drag { start: (x, y), last: (x, y), moved: false, in_card });
                        }
                    }
                }
            }
            Move => {
                if let Some(d) = &mut self.drag {
                    let dist = ((x - d.start.0).powi(2) + (y - d.start.1).powi(2)).sqrt();
                    if dist > TAP_SLOP_PT {
                        d.moved = true;
                    }
                    if d.moved {
                        let (dx, dy) = (x - d.last.0, y - d.last.1);
                        if d.in_card {
                            self.card_scroll = (self.card_scroll - dy).max(0.0);
                        } else {
                            self.maps.view.pan(dx as f64, dy as f64);
                            self.maps.touch_settings();
                        }
                    }
                    d.last = (x, y);
                }
            }
            Up | Cancel => {
                if let Some(id) = self.pressed.take() {
                    if kind == Up {
                        if let Some(b) = self.buttons(ui).into_iter().find(|b| b.id == id) {
                            if b.rect.contains(x, y) && b.enabled {
                                self.run(ctx, b.what, t);
                            }
                        }
                    }
                    return;
                }
                let Some(d) = self.drag.take() else { return };
                if d.moved || kind == Cancel {
                    self.maps.persist();
                    return;
                }
                if d.in_card {
                    return;
                }
                let (mx, my) = self.to_map_pt(x, y);
                let double = self.last_tap.is_some_and(|(lx, ly, lt)| t - lt < DOUBLE_TAP_MS && (lx - x).abs() < DOUBLE_TAP_PT && (ly - y).abs() < DOUBLE_TAP_PT);
                if double {
                    self.last_tap = None;
                    self.maps.double_tap(mx as f64, my as f64);
                } else {
                    self.last_tap = Some((x, y, t));
                    let (lat, lon) = self.maps.view.to_geo(mx as f64, my as f64);
                    let pin = self.pin_at(mx, my);
                    self.maps.tap(lat, lon, pin);
                    self.card_scroll = 0.0;
                }
                self.maps.persist();
            }
        }
    }

    fn card_height(&self) -> f32 {
        let items = self.card_items();
        if items.is_empty() {
            return 0.0;
        }
        Self::flow(&items).1 + 32.0
    }

    /// The pointer's context action on the map: Drop Pin at that position.
    pub fn context(&mut self, ui: &Ui, px: f32, py: f32) -> bool {
        self.pt_px = ui.pt(1.0).max(0.01);
        let (x, y) = (px / self.pt(), py / self.pt());
        if !self.map_rect().contains(x, y) || self.buttons(ui).iter().any(|b| !b.kit && b.rect.contains(x, y)) {
            return false;
        }
        let (mx, my) = self.to_map_pt(x, y);
        let (lat, lon) = self.maps.view.to_geo(mx as f64, my as f64);
        self.maps.drop_pin_at(lat, lon);
        self.maps.persist();
        self.dirty = true;
        true
    }

    pub fn key(&mut self, ctx: &Ctx, k: KeyInput, now: f64) -> bool {
        let ms = now as u64;
        self.dirty = true;
        match k {
            KeyInput::Text(s) if self.focused => {
                for c in s.chars().filter(|c| !c.is_control()) {
                    self.maps.query.push_str(&c.to_string());
                }
                self.maps.query_edited(ms);
                true
            }
            KeyInput::Text(s) => match s.as_str() {
                "+" | "=" => self.act(ctx, Act::ZoomIn, now),
                "-" | "_" => self.act(ctx, Act::ZoomOut, now),
                "/" => {
                    self.focused = true;
                    true
                }
                _ => false,
            },
            KeyInput::Code(8) if self.focused => {
                self.maps.query.pop();
                self.maps.query_edited(ms);
                true
            }
            KeyInput::Code(46) if self.focused => {
                self.maps.query.delete();
                self.maps.query_edited(ms);
                true
            }
            KeyInput::Code(13) if self.focused => {
                let fx = self.maps.submit_search(ms);
                self.effects(ctx, fx);
                if self.maps.query.is_empty() {
                    self.focused = false;
                }
                true
            }
            KeyInput::Code(27) => {
                if self.focused && !self.maps.query.is_empty() {
                    self.maps.query.clear();
                    self.maps.query_edited(ms);
                } else {
                    self.focused = false;
                    self.maps.act(Act::Escape, ms);
                }
                true
            }
            KeyInput::Code(37) | KeyInput::Code(39) if self.focused => {
                self.maps.query.move_caret(matches!(k, KeyInput::Code(39)));
                true
            }
            KeyInput::Code(c @ 37..=40) => {
                let (dx, dy) = match c {
                    37 => (KEY_PAN_PT, 0.0),
                    39 => (-KEY_PAN_PT, 0.0),
                    38 => (0.0, KEY_PAN_PT),
                    _ => (0.0, -KEY_PAN_PT),
                };
                self.maps.view.pan(dx, dy);
                self.maps.touch_settings();
                true
            }
            _ => false,
        }
    }

    /// An Edit menu command while the search field has the keyboard (Cut, Copy, Paste, Select All, Undo, Redo).
    pub fn command(&mut self, command: Command, now: f64) -> bool {
        if !self.focused {
            return false;
        }
        let clipboard = bifrost_app::kit::clipboard::text().unwrap_or_default();
        if let Some(text) = self.maps.query.command(command, &clipboard) {
            bifrost_app::kit::clipboard::set_text(&text);
        }
        self.maps.query_edited(now as u64);
        self.dirty = true;
        true
    }

    pub fn act(&mut self, ctx: &Ctx, act: Act, now: f64) -> bool {
        self.run(ctx, What::Act(act), now);
        true
    }

    pub fn open_link(&mut self, ctx: &Ctx, link: &str, now: f64) -> bool {
        let fx = self.maps.open_link(link, now as u64);
        self.effects(ctx, fx);
        self.maps.persist();
        self.dirty = true;
        true
    }

    /// The text of a GPX file the person opened.
    pub fn gpx_text(&mut self, text: &str) {
        self.maps.note = Some(match self.maps.gpx_loaded(text) {
            Ok(summary) => format!("Opened GPX: {summary}"),
            Err(e) => format!("Could not open the file: {e}"),
        });
        self.maps.persist();
        self.dirty = true;
    }

    pub fn take_gpx_request(&mut self, request: u32) -> bool {
        self.gpx_request.take() == Some(request)
    }

    // ---- effects and background work ----

    fn effects(&mut self, ctx: &Ctx, effects: Vec<Effect>) {
        for fx in effects {
            match fx {
                Effect::Search { query, wait_ms } => {
                    let (net, geocoder) = (self.net.clone(), self.maps.geocoder().clone());
                    self.spawn(move || {
                        if wait_ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                        }
                        let result = geocoder
                            .search_url(&query)
                            .and_then(|url| net.get(&url, geocoder.user_agent(), crate::net::REPLY_LIMIT).map_err(|e| e.to_string()))
                            .and_then(|body| geocoder.parse(&String::from_utf8_lossy(&body)));
                        Done::Search { query, result }
                    });
                }
                Effect::Route { url } => {
                    let net = self.net.clone();
                    let ua = crate::source::user_agent(self.maps.version());
                    self.spawn(move || Done::Route(net.get(&url, &ua, crate::net::REPLY_LIMIT).map_err(|e| e.to_string()).and_then(|b| route::parse_routes(&String::from_utf8_lossy(&b)))));
                }
                Effect::Copy(text) => {
                    bifrost_app::kit::clipboard::set_text(&text);
                    self.maps.note = Some("Copied".to_string());
                }
                Effect::Share(link) => {
                    let anchor = self.buttons_anchor(Act::Share);
                    self.share_request = Some(bifrost_app::share::show_picker(ctx, &[bifrost_app::share::Payload::Url(link)], anchor));
                }
                Effect::Print { title, text } => {
                    self.spawn(move || {
                        let result = bifrost_app::print::PrintOperation::text(title, text).run().map_err(|e| match e {
                            bifrost_app::print::PrintError::NoCups => "no print service is running".to_string(),
                            bifrost_app::print::PrintError::NoPrinter => "no printer is set up".to_string(),
                            bifrost_app::print::PrintError::Failed(m) => m,
                        });
                        Done::Printed(result)
                    });
                    self.maps.note = Some("Sending to the printer\u{2026}".to_string());
                }
                Effect::OpenGpx => {
                    self.gpx_request = Some(ctx.open_document(bifrost_app::proto::DocMode::Read, "Open GPX File", &["public.xml"]));
                }
                Effect::ReloadTiles => {
                    let root = self.maps.root().to_path_buf();
                    self.loader.switch_source(&root, self.maps.map_type.source(self.maps.version()));
                    self.scaled.clear();
                }
                Effect::StartDownload(tiles) => self.loader.start_download(tiles),
                Effect::CancelDownload => self.loader.cancel_download(),
                Effect::FocusSearch => self.focused = true,
                Effect::Plan { url } => {
                    let net = self.net.clone();
                    let ua = crate::source::user_agent(self.maps.version());
                    self.spawn(move || Done::Route(net.get(&url, &ua, crate::net::REPLY_LIMIT).map_err(|e| e.to_string()).and_then(|b| crate::transit::parse_itineraries(&String::from_utf8_lossy(&b)))));
                }
                Effect::Locate => self.ask_location(ctx),
                Effect::TrackLocation(on) => self.track_location(ctx, on),
            }
        }
    }

    // ---- the location service ----

    /// Makes sure the person has allowed location, asking them first when they have not answered. Once it is allowed
    /// (`authorized`), a position is asked for.
    fn ask_location(&mut self, ctx: &Ctx) {
        if self.granted {
            self.authorized(ctx);
            return;
        }
        let id = self.loc.request_when_in_use_authorization(ctx);
        self.loc_calls.push((id, LocCall::Authorize));
    }

    /// Starts or stops the live positions that guidance follows.
    fn track_location(&mut self, ctx: &Ctx, on: bool) {
        self.tracking = on;
        if on {
            self.ask_location(ctx);
        } else if self.loc.is_updating() {
            let id = self.loc.stop_updating_location(ctx);
            self.loc_calls.push((id, LocCall::Other));
        }
    }

    /// Location is allowed: ask for one position now and, while a route is followed, for every change after it.
    fn authorized(&mut self, ctx: &Ctx) {
        self.granted = true;
        self.allow_until = None;
        if self.maps.locating || self.tracking {
            let id = self.loc.request_location(ctx);
            self.loc_calls.push((id, LocCall::Other));
        }
        if self.tracking && !self.loc.is_updating() {
            let id = self.loc.start_updating_location(ctx);
            self.loc_calls.push((id, LocCall::Other));
        }
    }

    /// The person said no (or the system did): nothing more is asked, and the reason shows.
    fn refused(&mut self, why: &str) {
        self.granted = false;
        self.allow_until = None;
        self.tracking = false;
        self.maps.locate_failed(why);
    }

    /// A reply to a call this window made to the location or privacy service. False for any other reply.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(at) = self.loc_calls.iter().position(|(id, _)| *id == request) else { return false };
        let (_, call) = self.loc_calls.remove(at);
        let mut sink = Sink::default();
        match call {
            LocCall::Authorize => match result.as_ref().ok().and_then(privacy_permissions::decode_location_outcome) {
                Some(RequestOutcome::AlreadyGranted) => self.authorized(ctx),
                Some(RequestOutcome::Prompt) => {
                    // the Allow sheet is up; its answer is read from the permission
                    let now = bifrost_app::proto::sys::now_ms();
                    self.allow_until = Some(now + ALLOW_WAIT_MS);
                    self.next_status_poll = now + STATUS_POLL_MS;
                    self.maps.note = Some("Allow Maps to use your location to go on".to_string());
                }
                Some(RequestOutcome::Refused) => self.refused(location_error_text(LocationError::Denied)),
                None => self.refused(location_error_text(LocationError::LocationUnknown)),
            },
            LocCall::Other => {
                self.loc.handle_reply(result, &mut sink);
            }
        }
        self.apply_location(ctx, sink);
        self.dirty = true;
        true
    }

    /// A push from the location service (a changed position, a failure).
    pub fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let mut sink = Sink::default();
        if !self.loc.handle_event(service, event, body, &mut sink) {
            return false;
        }
        self.apply_location(ctx, sink);
        self.dirty = true;
        true
    }

    /// Carries out what the location manager reported: a permission change, failures, then the newest position.
    fn apply_location(&mut self, ctx: &Ctx, sink: Sink) {
        for status in sink.statuses {
            if status.is_authorized() {
                self.authorized(ctx);
            } else if matches!(status, AuthorizationStatus::Denied | AuthorizationStatus::Restricted) {
                self.refused(location_error_text(LocationError::Denied));
            }
        }
        for error in sink.errors {
            if error.needs_user_action() {
                self.granted = false;
            }
            self.maps.locate_failed(location_error_text(error));
        }
        if let Some(l) = sink.updates.last() {
            let here = Here { lat: l.coordinate.latitude, lon: l.coordinate.longitude, accuracy_m: l.horizontal_accuracy };
            let fx = self.maps.position_update(here, bifrost_app::proto::sys::now_ms() as u64);
            self.effects(ctx, fx);
            self.maps.persist();
        }
    }

    /// Where the share sheet anchors: under the Share toolbar button, else the map's top right.
    fn buttons_anchor(&self, act: Act) -> KRect {
        let m = self.map_rect();
        let _ = act;
        rect(m.x + m.w - 60.0, 0.0, 36.0, UNIFIED_BAR_PT)
    }

    fn spawn(&self, job: impl FnOnce() -> Done + Send + 'static) {
        let done = self.done.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = job();
            done.lock().unwrap_or_else(|e| e.into_inner()).push(result);
            DispatchQueue::main().async_(|| {});
        });
    }

    pub fn set_online(&mut self, online: bool) {
        self.loader.set_online(online);
        if let Some(t) = &mut self.traffic {
            t.set_online(online);
        }
    }

    /// Applies finished work and starts what is due. True when a frame is needed.
    pub fn tick(&mut self, ctx: &Ctx, ui: &Ui, t: f64) -> bool {
        self.pt_px = ui.pt(1.0).max(0.01);
        let m = self.map_rect();
        self.maps.resize(m.w as f64, m.h as f64);
        let mut changed = std::mem::take(&mut self.dirty);
        let finished = std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner()));
        for d in finished {
            changed = true;
            match d {
                Done::Search { query, result } => self.maps.search_done(&query, result),
                Done::Route(r) => self.maps.route_done(r),
                Done::Printed(r) => {
                    self.maps.note = Some(match r {
                        Ok(job) => format!("Printed ({job})"),
                        Err(e) => format!("Printing failed: {e}"),
                    })
                }
            }
        }
        if let Some(fx) = self.maps.due_search(t as u64) {
            self.effects(ctx, vec![fx]);
            changed = true;
        }
        if let Some(until) = self.allow_until {
            if t >= until {
                self.refused("Maps was not allowed to use your location.");
                changed = true;
            } else if t >= self.next_status_poll {
                self.next_status_poll = t + STATUS_POLL_MS;
                let id = self.loc.refresh_authorization(ctx);
                self.loc_calls.push((id, LocCall::Other));
            }
        }
        let wanted: Vec<TileId> = self.maps.view.tiles().into_iter().map(|(id, _, _)| id).collect();
        changed |= self.loader.want(&wanted);
        changed |= self.loader.pump();
        if self.maps.show_traffic != self.traffic_on {
            self.traffic_on = self.maps.show_traffic;
            if let (true, Some(layer)) = (self.traffic_on, self.traffic.as_mut()) {
                layer.retry_failed();
            }
            changed = true;
        }
        if self.maps.show_traffic {
            if let Some(layer) = &mut self.traffic {
                changed |= layer.want(&wanted);
                changed |= layer.pump();
            }
        }
        if let Some(p) = self.loader.download_progress() {
            if self.maps.progress != Some(p) {
                self.maps.progress = Some(p);
                changed = true;
            }
            if self.loader.download_finished() {
                self.loader.end_download();
                self.maps.download_done();
            }
        }
        changed
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let search = self.maps.next_search_wake(t as u64).map(|ms| ms as f64);
        let poll = self.allow_until.map(|until| self.next_status_poll.min(until));
        match (search, poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // ---- drawing ----

    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, align: Align, x: f32, mid: f32) {
        let img = self.labels.get(ui, text, size, weight, color);
        let left = match align {
            Align::Left => x,
            Align::Centre => x - img.w as f32 / 2.0,
            Align::Right => x - img.w as f32,
        };
        s.blit(img, left.round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }

    fn dot(s: &mut Surface, x: f32, y: f32, r: f32, color: u32) {
        s.fill_rrect(x - r, y - r, 2.0 * r, 2.0 * r, r, color);
    }

    /// A line through screen points (pixels), drawn as overlapping dots.
    fn polyline(s: &mut Surface, pts: &[(f32, f32)], width: f32, color: u32) {
        let r = width / 2.0;
        let mut prev: Option<(f32, f32)> = None;
        for &(x, y) in pts {
            if let Some((px, py)) = prev {
                let (dx, dy) = (x - px, y - py);
                let len = (dx * dx + dy * dy).sqrt();
                if len > 4000.0 {
                    prev = Some((x, y));
                    continue;
                }
                let steps = (len / (r.max(1.0))).ceil().max(1.0) as usize;
                for i in 1..=steps {
                    let k = i as f32 / steps as f32;
                    Self::dot(s, px + dx * k, py + dy * k, r, color);
                }
            }
            prev = Some((x, y));
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.pt_px = ui.pt(1.0).max(0.01);
        self.dirty = false;
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        let p = self.pt();
        let side = self.side_pt();
        if side > 0.0 {
            self.draw_sidebar(s, ui);
        }
        {
            let side_px = (side * p) as usize;
            let w = s.w.saturating_sub(side_px);
            let start = side_px.min(s.px.len());
            let mut d = Surface { w, h: s.h, stride: s.stride, px: &mut s.px[start..], y0: s.y0 };
            self.draw_map(&mut d, ui, side);
            self.draw_bar(&mut d, ui);
        }
        if side == 0.0 && !self.maps.query.is_empty() {
            let (list, _) = self.side_list();
            widgets::source_list(s, ui, self.list_rect(), &list, 0.0, None);
        }
        if side == 0.0 {
            widgets::search_field(s, ui, &self.maps.query, self.search_rect(), "Search", self.focused);
        }
        self.draw_card(s, ui);
        self.draw_plates(s, ui);
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        let p = self.pt();
        let (list, _) = self.side_list();
        let h = self.h as f32 / p;
        widgets::source_list(s, ui, rect(0.0, 0.0, SIDEBAR_PT, h), &SourceList::new(vec![], ROW_PT, ROW_PT), 0.0, None);
        widgets::search_field(s, ui, &self.maps.query, self.search_rect(), "Search", self.focused);
        let pressed = self.pressed.and_then(|id| self.buttons(ui).into_iter().find(|b| b.id == id)).and_then(|b| if let What::Row(r) = b.what { Some(r) } else { None });
        widgets::source_list(s, ui, self.list_rect(), &list, 0.0, pressed);
        s.fill_rect(((SIDEBAR_PT * p) - 1.0) as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
    }

    /// `d` is the detail pane (x = 0 at the sidebar's edge); the map starts under the toolbar.
    fn draw_map(&mut self, d: &mut Surface, ui: &Ui, _side: f32) {
        let p = self.pt();
        let top = (UNIFIED_BAR_PT * p) as usize;
        let mut c = d.rows(top, d.h);
        c.fill_rect(0, top, c.w, c.h.saturating_sub(top), SIDEBAR_BG.now());
        let zoom = self.maps.view.zoom();
        let (_, scale) = crate::tiles::tile_level(zoom);
        let tile_px = (TILE_SIZE * scale * p as f64).round() as usize;
        let oy = UNIFIED_BAR_PT * p;
        let tiles = self.maps.view.tiles();
        for &(id, tx, ty) in &tiles {
            let (x, y) = ((tx as f32 * p).round() as i32, (ty as f32 * p).round() as i32 + oy as i32);
            let Some(img) = self.loader.image(id) else { continue };
            blit_tile(&mut c, &mut self.scaled, id, img, x, y, tile_px);
        }
        // the traffic layer, over the base tiles
        if self.maps.show_traffic {
            if let Some(layer) = &self.traffic {
                for &(id, tx, ty) in &tiles {
                    let (x, y) = ((tx as f32 * p).round() as i32, (ty as f32 * p).round() as i32 + oy as i32);
                    let Some(img) = layer.image(id) else { continue };
                    blit_tile(&mut c, &mut self.scaled_traffic, id, img, x, y, tile_px);
                }
            }
        }
        // overlays: tracks, the route, the measurement, then markers
        let view = self.maps.view;
        let to_px = |lat: f64, lon: f64| {
            let (x, y) = view.to_screen(lat, lon);
            (x as f32 * p, y as f32 * p + oy)
        };
        for (_, pts) in &self.maps.tracks {
            let line: Vec<(f32, f32)> = pts.iter().map(|&(la, lo)| to_px(la, lo)).collect();
            Self::polyline(&mut c, &line, ui.pt(SPACE_XS), bifrost_app::kit::colors::SYSTEM_PURPLE.now());
        }
        if matches!(self.maps.panel, Panel::Directions | Panel::Navigation) {
            for (i, r) in self.maps.routes.iter().enumerate() {
                // while a route is followed only that one is drawn
                if i == self.maps.route_index || self.maps.panel == Panel::Navigation {
                    continue;
                }
                let line: Vec<(f32, f32)> = r.geometry.iter().map(|&(la, lo)| to_px(la, lo)).collect();
                Self::polyline(&mut c, &line, ui.pt(5.0), SYSTEM_TEAL);
            }
            if let Some(r) = self.maps.current_route() {
                let line: Vec<(f32, f32)> = r.geometry.iter().map(|&(la, lo)| to_px(la, lo)).collect();
                Self::polyline(&mut c, &line, ui.pt(m::ICON_RADIUS), if r.approximate { SYSTEM_ORANGE } else { LINK });
            }
            for (end, color) in [(&self.maps.route_from, SYSTEM_GREEN), (&self.maps.route_to, SYSTEM_RED)] {
                if let Some((_, (la, lo))) = end {
                    let (x, y) = to_px(*la, *lo);
                    Self::dot(&mut c, x, y, ui.pt(SPACE_S), WHITE);
                    Self::dot(&mut c, x, y, ui.pt(5.5), color);
                }
            }
        }
        if self.maps.panel == Panel::Measure {
            let pts: Vec<(f32, f32)> = self.maps.measure.points().iter().map(|&(la, lo)| to_px(la, lo)).collect();
            Self::polyline(&mut c, &pts, ui.pt(3.0), LINK);
            for &(x, y) in &pts {
                Self::dot(&mut c, x, y, ui.pt(m::ICON_RADIUS), WHITE);
                Self::dot(&mut c, x, y, ui.pt(SPACE_XS), LINK);
            }
        }
        if let Some(i) = self.maps.shown_collection {
            if let Some(coll) = self.maps.library.collections.get(i) {
                for pl in &coll.places {
                    let (x, y) = to_px(pl.lat, pl.lon);
                    Self::dot(&mut c, x, y, ui.pt(SPACE_S), WHITE);
                    Self::dot(&mut c, x, y, ui.pt(m::ICON_RADIUS), SYSTEM_ORANGE);
                }
            }
        }
        for (i, pin) in self.maps.pins.items().iter().enumerate() {
            let (x, y) = to_px(pin.lat, pin.lon);
            if x < -20.0 || y < oy - 20.0 || x > c.w as f32 + 20.0 || y > c.h as f32 + 40.0 {
                continue;
            }
            let on = self.maps.selected_pin == Some(i);
            let r = ui.pt(if on { 9.0 } else { 7.5 });
            // a teardrop: a disc on a short point
            c.fill_rrect(x - ui.pt(SPACE_XXS), y - ui.pt(m::ICON_RADIUS), ui.pt(SPACE_XS), ui.pt(m::ICON_RADIUS), 0.0, SYSTEM_RED);
            Self::dot(&mut c, x, y - ui.pt(10.0), r, WHITE);
            Self::dot(&mut c, x, y - ui.pt(10.0), r - ui.pt(1.5), if on { LINK } else { SYSTEM_RED });
        }
        if self.maps.selected_pin.is_none() {
            if let Some(sel) = &self.maps.selected {
                let (x, y) = to_px(sel.lat, sel.lon);
                Self::dot(&mut c, x, y, ui.pt(9.0), WHITE);
                Self::dot(&mut c, x, y, ui.pt(6.5), LINK);
            }
        }
        // the person's position: a dot, and a ring as wide as the fix is uncertain when that is wide enough to see
        if let Some(h) = self.maps.here {
            let (x, y) = to_px(h.lat, h.lon);
            let ring = (h.accuracy_m / metres_per_pt(h.lat, zoom).max(1e-9)) as f32 * p;
            if ring > ui.pt(14.0) {
                let ring = ring.min(ui.pt(2000.0));
                c.stroke_rrect(x - ring, y - ring, 2.0 * ring, 2.0 * ring, ring, 1.0, LINK);
            }
            Self::dot(&mut c, x, y, ui.pt(9.0), WHITE);
            Self::dot(&mut c, x, y, ui.pt(6.5), SYSTEM_TEAL);
        }
        // the scale bar and the data credit, bottom corners
        let bottom = c.h as f32 - ui.pt(MARGIN);
        let (lat, _) = self.maps.view.centre();
        if let Some(bar) = scale_bar(metres_per_pt(lat, zoom), 120.0, self.maps.units()) {
            let w = ui.pt(bar.length_pt as f32);
            let x = ui.pt(MARGIN);
            let plate_w = w + ui.pt(64.0);
            c.fill_rrect(x, bottom - ui.pt(SPACE_XXL), plate_w, ui.pt(SPACE_XXL), ui.pt(m::ICON_RADIUS), CARD_BG.now());
            c.fill_rect((x + ui.pt(SPACE_S)) as usize, (bottom - ui.pt(9.0)) as usize, w as usize, ui.pt(SPACE_XXS).max(1.0) as usize, LABEL.now());
            c.fill_rect((x + ui.pt(SPACE_S)) as usize, (bottom - ui.pt(14.0)) as usize, ui.pt(SPACE_XXS).max(1.0) as usize, ui.pt(7.0) as usize, LABEL.now());
            c.fill_rect((x + ui.pt(SPACE_S) + w) as usize, (bottom - ui.pt(14.0)) as usize, ui.pt(SPACE_XXS).max(1.0) as usize, ui.pt(7.0) as usize, LABEL.now());
            self.put(&mut c, ui, &bar.label, MAC_CAPTION1_PT, Weight::Regular, LABEL.now(), Align::Left, x + w + ui.pt(14.0), bottom - ui.pt(SPACE_M));
        }
        let credit = self.maps.map_type.attribution();
        let cw = ui.measure(credit, MAC_CAPTION1_PT, Weight::Regular);
        let right = c.w as f32 - ui.pt(MARGIN);
        c.fill_rrect(right - cw - ui.pt(SPACE_L), bottom - ui.pt(SPACE_XL), cw + ui.pt(SPACE_L), ui.pt(SPACE_XL), ui.pt(m::ICON_RADIUS), CARD_BG.now());
        self.put(&mut c, ui, credit, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, right - ui.pt(SPACE_S), bottom - ui.pt(10.0));
        // a line of news when no card shows it
        if self.maps.panel == Panel::None {
            if let Some(n) = self.maps.note.clone() {
                let text = ui.fit(&n, MAC_SUBHEAD_PT, c.w as f32 - ui.pt(80.0));
                let tw = ui.measure(&text, MAC_SUBHEAD_PT, Weight::Regular);
                let cx = c.w as f32 / 2.0;
                let y = oy + ui.pt(30.0);
                c.fill_rrect(cx - tw / 2.0 - ui.pt(SPACE_M), y - ui.pt(14.0), tw + ui.pt(SPACE_XXL), ui.pt(28.0), ui.pt(m::ICON_RADIUS), CARD_BG.now());
                self.put(&mut c, ui, &text, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), Align::Centre, cx, y);
            }
        }
    }

    fn draw_bar(&mut self, d: &mut Surface, ui: &Ui) {
        let p = self.pt();
        let bar = rect(0.0, 0.0, d.w as f32 / p, UNIFIED_BAR_PT);
        widgets::toolbar_band(d, ui, bar);
        let pressed = self.pressed.and_then(|id| self.buttons(ui).into_iter().find(|b| b.id == id)).and_then(|b| {
            let table = bar_table();
            match b.what {
                What::Act(a) => table.iter().find(|t| t.3 == a).map(|t| t.0),
                _ => None,
            }
        });
        widgets::toolbar(d, ui, self.bar_bounds(), &self.bar(), pressed);
    }

    fn draw_plates(&mut self, s: &mut Surface, ui: &Ui) {
        let p = self.pt();
        let side = self.side_pt();
        let _ = side;
        for (r, glyph) in self.plate_rects().into_iter().zip(["+", "\u{2212}"]) {
            let (x, y, w, h) = (r.x * p, r.y * p, r.w * p, r.h * p);
            s.fill_rrect(x, y, w, h, ui.pt(m::ICON_RADIUS), CARD_BG.now());
            s.stroke_rrect(x, y, w, h, ui.pt(m::ICON_RADIUS), 1.0, SEPARATOR_C.now());
            self.put(s, ui, glyph, MAC_TITLE1_PT, Weight::Regular, LABEL.now(), Align::Centre, x + w / 2.0, y + h / 2.0);
        }
    }

    fn draw_card(&mut self, s: &mut Surface, ui: &Ui) {
        let items = self.card_items();
        if items.is_empty() {
            return;
        }
        let p = self.pt();
        let (placed, total) = Self::flow(&items);
        let card = self.card_rect(total + 32.0);
        let (cx, cy, cw, ch) = (card.x * p, card.y * p, card.w * p, card.h * p);
        s.fill_rrect(cx, cy, cw, ch, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        s.stroke_rrect(cx, cy, cw, ch, ui.pt(m::CARD_RADIUS), 1.0, SEPARATOR_C.now());
        let buttons = self.buttons(ui);
        let pressed = self.pressed;
        let mut clip = s.rows(cy as usize + 1, (cy + ch) as usize - 1);
        let base = card.y + MARGIN - self.card_scroll;
        let left = (card.x + MARGIN) * p;
        let right = (card.x + card.w - MARGIN) * p;
        for (item, (y, h)) in items.iter().zip(placed) {
            let top = (base + y) * p;
            let mid = top + h * p / 2.0;
            if top + h * p < cy || top > cy + ch {
                continue;
            }
            match item {
                Item::Title(t) => {
                    let text = ui.fit(t, MAC_TITLE3_PT, right - left - ui.pt(36.0));
                    self.put(&mut clip, ui, &text, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), Align::Left, left, mid);
                }
                Item::Sub(t) => {
                    let text = ui.fit(t, MAC_FOOTNOTE_PT, right - left);
                    self.put(&mut clip, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, left, mid);
                }
                Item::Line(t) => {
                    let text = ui.fit(t, MAC_SUBHEAD_PT, right - left);
                    self.put(&mut clip, ui, &text, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), Align::Left, left, mid);
                }
                Item::Gap(_) => {}
                Item::Rule => clip.fill_rect(left as usize, mid as usize, (right - left) as usize, 1, SEPARATOR_C.now()),
                Item::Progress(f) => {
                    let (w, bh) = (right - left, ui.pt(m::ICON_RADIUS));
                    clip.fill_rrect(left, mid - bh / 2.0, w, bh, bh / 2.0, SEPARATOR_C.now());
                    clip.fill_rrect(left, mid - bh / 2.0, (w * f.clamp(0.0, 1.0)).max(bh), bh, bh / 2.0, LINK);
                }
                Item::Step { text, distance } => {
                    let t = ui.fit(text, MAC_SUBHEAD_PT, right - left - ui.pt(70.0));
                    self.put(&mut clip, ui, &t, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), Align::Left, left, mid);
                    self.put(&mut clip, ui, distance, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Right, right, mid);
                }
                Item::Route { .. } | Item::Buttons(_) => {}
            }
        }
        // buttons and route rows come from the shared list, so what is drawn is what is hit
        for b in buttons.iter().filter(|b| !b.kit && b.role == Role::Button && !matches!(b.what, What::Act(Act::ZoomIn) | What::Act(Act::ZoomOut))) {
            let (x, y, w, h) = (b.rect.x * p, b.rect.y * p, b.rect.w * p, b.rect.h * p);
            if y + h < cy || y > cy + ch {
                continue;
            }
            let down = pressed == Some(b.id);
            let is_close = matches!(b.what, What::Act(Act::ClosePanel)) && b.label == "Close";
            if is_close {
                self.put(&mut clip, ui, "\u{d7}", MAC_TITLE1_PT, Weight::Regular, LABEL_2.now(), Align::Centre, x + w / 2.0, y + h / 2.0);
                continue;
            }
            let route_row = matches!(b.what, What::Act(Act::PickRoute(_)));
            let fill = if b.selected { LINK } else if down { SEPARATOR_C.now() } else { CONTENT_BG.now() };
            clip.fill_rrect(x, y, w, h, ui.pt(m::ICON_RADIUS), fill);
            let (fg, fg2) = if b.selected { (WHITE, WHITE) } else if b.enabled { (LINK, LABEL_2.now()) } else { (LABEL_2.now(), LABEL_2.now()) };
            if route_row {
                let (tag, line) = b.label.split_once(", ").unwrap_or(("", &b.label));
                let (line, tag) = (line.to_string(), tag.to_string());
                let fg_main = if b.selected { WHITE } else { LABEL.now() };
                self.put(&mut clip, ui, &line, MAC_BODY_PT, Weight::Semibold, fg_main, Align::Left, x + ui.pt(10.0), y + h * 0.36);
                self.put(&mut clip, ui, &tag, MAC_FOOTNOTE_PT, Weight::Regular, fg2, Align::Left, x + ui.pt(10.0), y + h * 0.74);
            } else {
                let text = ui.fit(&b.label, MAC_SUBHEAD_PT, w - ui.pt(SPACE_S));
                self.put(&mut clip, ui, &text, MAC_SUBHEAD_PT, Weight::Medium, fg, Align::Centre, x + w / 2.0, y + h / 2.0);
            }
        }
    }

    /// The state dump the tests check.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        format!(
            "maps_panel={:?}\nmaps_type={}\nmaps_zoom={:.1}\nmaps_pins={}\nmaps_focused={}\n",
            self.maps.panel,
            self.maps.map_type.key(),
            self.maps.view.zoom(),
            self.maps.pins.items().len(),
            self.focused
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Offline;
    use bifrost_app::kit::Font;
    use bifrost_app::proto::Configure;
    use bifrost_app::TouchKind;

    fn ui() -> Ui {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let mut ui = Ui::new(Font::parse(data).unwrap(), 1.0);
        ui.symbols = springboard::symbols::Symbols::load(dir);
        ui
    }

    fn ctx(w: u32, h: u32) -> Ctx {
        let cfg = Configure { w, h, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(ui(), cfg)
    }

    fn window(name: &str, w: i32, h: i32) -> (Window, Ui, Ctx) {
        let root = std::env::temp_dir().join(format!("maps-window-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ui = ui();
        let mut win = Window::new(&ui, w, h, root, "1.0.0", Arc::new(Offline));
        win.set_online(false);
        (win, ui, ctx(w as u32, h as u32))
    }

    fn centre_of(b: &Btn) -> (f32, f32) {
        (b.rect.x + b.rect.w / 2.0, b.rect.y + b.rect.h / 2.0)
    }

    fn click(w: &mut Window, ctx: &Ctx, ui: &Ui, x: f32, y: f32) {
        w.touch(ctx, ui, TouchKind::Down, x, y, 1000.0);
        w.touch(ctx, ui, TouchKind::Up, x, y, 1010.0);
    }

    fn find(w: &Window, ui: &Ui, label: &str) -> Btn {
        w.buttons(ui).into_iter().find(|b| b.label == label).unwrap_or_else(|| panic!("no control named {label}"))
    }

    #[test]
    fn every_control_has_a_label_and_a_unique_id() {
        let (w, ui, _) = window("labels", 1200, 780);
        let nodes = w.controls(&ui);
        assert!(!nodes.is_empty());
        assert!(nodes.iter().all(|n| !n.label.is_empty()), "{nodes:?}");
        let mut ids: Vec<u64> = nodes.iter().map(|n| n.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), nodes.len());
    }

    #[test]
    fn the_zoom_buttons_zoom_the_map() {
        let (mut w, ui, ctx) = window("zoom", 1200, 780);
        let z = w.maps.view.zoom();
        let (x, y) = centre_of(&find(&w, &ui, "Zoom In"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.view.zoom(), z + 1.0);
        let (x, y) = centre_of(&find(&w, &ui, "Zoom Out"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.view.zoom(), z);
    }

    #[test]
    fn dragging_the_map_pans_it_and_a_tap_does_not() {
        let (mut w, ui, ctx) = window("pan", 1200, 780);
        let before = w.maps.view.centre();
        w.touch(&ctx, &ui, TouchKind::Down, 700.0, 400.0, 1000.0);
        w.touch(&ctx, &ui, TouchKind::Move, 650.0, 350.0, 1010.0);
        w.touch(&ctx, &ui, TouchKind::Move, 600.0, 300.0, 1020.0);
        w.touch(&ctx, &ui, TouchKind::Up, 600.0, 300.0, 1030.0);
        assert_ne!(w.maps.view.centre(), before);
        let panned = w.maps.view.centre();
        click(&mut w, &ctx, &ui, 700.0, 400.0);
        assert_eq!(w.maps.view.centre(), panned);
    }

    #[test]
    fn a_double_click_zooms_in_at_the_pointer() {
        let (mut w, ui, ctx) = window("double", 1200, 780);
        let z = w.maps.view.zoom();
        click(&mut w, &ctx, &ui, 700.0, 400.0);
        w.touch(&ctx, &ui, TouchKind::Down, 700.0, 400.0, 1100.0);
        w.touch(&ctx, &ui, TouchKind::Up, 700.0, 400.0, 1110.0);
        assert!(w.maps.view.zoom() > z);
    }

    #[test]
    fn tapping_a_pin_opens_its_card_with_directions() {
        let (mut w, ui, ctx) = window("pin", 1200, 780);
        let (lat, lon) = w.maps.view.centre();
        w.maps.pins.add("Camp", lat, lon).unwrap();
        let (lx, ly) = w.maps.view.to_screen(lat, lon);
        let m = w.map_rect();
        click(&mut w, &ctx, &ui, m.x + lx as f32, m.y + ly as f32 - 9.0);
        assert_eq!(w.maps.panel, Panel::Place);
        assert_eq!(w.maps.selected_pin, Some(0));
        let (x, y) = centre_of(&find(&w, &ui, "Directions"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.panel, Panel::Directions);
        assert!(w.maps.current_route().is_some(), "with no routing server the estimate shows");
    }

    #[test]
    fn typing_a_coordinate_and_pressing_return_shows_the_place() {
        let (mut w, ui, ctx) = window("typing", 1200, 780);
        let (x, y) = centre_of(&find(&w, &ui, "Search"));
        click(&mut w, &ctx, &ui, x, y);
        assert!(w.keyboard_shown());
        w.key(&ctx, KeyInput::Text("64.1, -21.9".into()), 2000.0);
        assert_eq!(w.maps.query.to_string(), "64.1, -21.9");
        let rows: Vec<Btn> = w.buttons(&ui).into_iter().filter(|b| b.role == Role::Row).collect();
        assert!(!rows.is_empty(), "the coordinate is the first suggestion");
        w.key(&ctx, KeyInput::Code(13), 2100.0);
        assert_eq!(w.maps.panel, Panel::Place);
        assert!(w.maps.query.is_empty());
    }

    #[test]
    fn escape_leaves_the_search_field_and_backspace_edits_it() {
        let (mut w, ui, ctx) = window("escape", 1200, 780);
        let (x, y) = centre_of(&find(&w, &ui, "Search"));
        click(&mut w, &ctx, &ui, x, y);
        w.key(&ctx, KeyInput::Text("abc".into()), 0.0);
        w.key(&ctx, KeyInput::Code(8), 0.0);
        assert_eq!(w.maps.query.to_string(), "ab");
        w.key(&ctx, KeyInput::Code(27), 0.0);
        assert!(w.maps.query.is_empty());
        w.key(&ctx, KeyInput::Code(27), 0.0);
        assert!(!w.keyboard_shown());
    }

    #[test]
    fn the_sidebar_lists_home_work_and_saved_places() {
        let (mut w, ui, ctx) = window("sidebar", 1200, 780);
        w.maps.library.toggle_favorite(crate::store::Saved::new("Perlan, Reykjavik", 64.1295, -21.9186));
        let names: Vec<String> = w.buttons(&ui).into_iter().filter(|b| b.role == Role::Row).map(|b| b.label).collect();
        assert_eq!(names, vec!["Home", "Work", "Perlan"]);
        let (x, y) = centre_of(&find(&w, &ui, "Perlan"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.panel, Panel::Place);
    }

    #[test]
    fn a_narrow_window_has_no_sidebar_and_the_search_floats_over_the_map() {
        let (w, ui, _) = window("narrow", 500, 700);
        assert_eq!(w.side_pt(), 0.0);
        let s = w.search_rect();
        assert!(s.x > 0.0 && s.y >= UNIFIED_BAR_PT);
        assert!(w.buttons(&ui).iter().all(|b| b.role != Role::Row), "no saved places without a sidebar");
    }

    #[test]
    fn activating_a_control_by_id_runs_it() {
        let (mut w, ui, ctx) = window("activate", 1200, 780);
        let z = w.maps.view.zoom();
        let id = find(&w, &ui, "Zoom In").id;
        assert!(w.activate(&ctx, &ui, id, 0.0));
        assert_eq!(w.maps.view.zoom(), z + 1.0);
        assert!(!w.activate(&ctx, &ui, 9999, 0.0));
    }

    #[test]
    fn the_card_buttons_follow_the_panel() {
        let (mut w, ui, ctx) = window("card", 1200, 780);
        w.maps.choose(crate::geocode::Place::new("Perlan, Reykjavik", 64.1295, -21.9186));
        for label in ["Directions", "Favorite", "Share", "Drop Pin", "Copy Link", "Print", "Set Home", "Set Work", "New Collection"] {
            find(&w, &ui, label);
        }
        let (x, y) = centre_of(&find(&w, &ui, "Favorite"));
        click(&mut w, &ctx, &ui, x, y);
        assert!(w.maps.library.is_favorite(64.1295, -21.9186));
        find(&w, &ui, "Unfavorite");
        let (x, y) = centre_of(&find(&w, &ui, "Close"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.panel, Panel::None);
    }

    #[test]
    fn measure_mode_collects_clicks_on_the_map() {
        let (mut w, ui, ctx) = window("measure", 1200, 780);
        w.act(&ctx, Act::Measure, 0.0);
        click(&mut w, &ctx, &ui, 600.0, 400.0);
        click(&mut w, &ctx, &ui, 800.0, 400.0);
        assert_eq!(w.maps.measure.points().len(), 2);
        assert!(w.maps.measure_line().contains("in total"));
    }

    #[test]
    fn it_draws_more_than_one_colour_at_two_sizes() {
        for (width, height) in [(1200usize, 780usize), (500, 700)] {
            let (mut w, ui, _) = window("draw", width as i32, height as i32);
            w.maps.choose(crate::geocode::Place::new("Perlan, Reykjavik", 64.1295, -21.9186));
            let mut px = vec![0u32; width * height];
            let mut s = Surface { w: width, h: height, stride: width, px: &mut px, y0: 0 };
            w.draw(&mut s, &ui, 0.0);
            let first = px[0];
            assert!(px.iter().any(|p| *p != first), "{width}x{height} is blank");
        }
    }

    #[test]
    fn describe_names_the_state() {
        let (w, _, _) = window("describe", 1200, 780);
        assert!(w.describe().contains("maps_panel=None"));
    }

    /// The service calls the window queued since the last look, as (request, method).
    fn calls(ctx: &Ctx) -> Vec<(u32, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                bifrost_app::proto::ToShell::ServiceCall { request, method, .. } => Some((request, method)),
                _ => None,
            })
            .collect()
    }

    fn methods(calls: &[(u32, String)]) -> Vec<&str> {
        calls.iter().map(|c| c.1.as_str()).collect()
    }

    fn json(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    fn fix_json(lat: f64, lon: f64, accuracy: f64) -> Json {
        json(&format!(
            r#"{{"latitude":{lat},"longitude":{lon},"altitude":null,"horizontal_accuracy":{accuracy},"timestamp":1790000000,"source":"manual","city":"Akureyri","country":"Iceland"}}"#
        ))
    }

    #[test]
    fn the_current_location_button_asks_permission_then_centres_on_the_position() {
        let (mut w, ui, ctx) = window("locate", 1200, 780);
        let (x, y) = centre_of(&find(&w, &ui, "Current Location"));
        click(&mut w, &ctx, &ui, x, y);
        assert!(w.maps.locating);
        let sent = calls(&ctx);
        assert_eq!(methods(&sent), vec!["location_request"]);
        // the person has not answered: the Allow sheet is up, and the window looks at the permission now and then
        assert!(w.service_reply(&ctx, sent[0].0, &Ok(json(r#"{"outcome":"prompt"}"#))));
        assert!(w.next_wake(0.0).is_some(), "the window wakes to look at the permission");
        w.tick(&ctx, &ui, bifrost_app::proto::sys::now_ms() + 2_000.0);
        let poll = calls(&ctx);
        assert_eq!(methods(&poll), vec!["location_status"]);
        assert!(w.service_reply(&ctx, poll[0].0, &Ok(json(r#"{"status":"authorized_when_in_use","authorized":true}"#))));
        let ask = calls(&ctx);
        assert_eq!(methods(&ask), vec!["current"]);
        assert!(w.service_reply(&ctx, ask[0].0, &Ok(fix_json(65.68, -18.1, 100.0))));
        assert!(!w.maps.locating);
        let (lat, lon) = w.maps.view.centre();
        assert!((lat - 65.68).abs() < 1e-6 && (lon + 18.1).abs() < 1e-6);
        assert!(w.next_wake(0.0).is_none(), "nothing is waited for any more");
    }

    #[test]
    fn an_allowed_app_is_not_asked_twice() {
        let (mut w, ui, ctx) = window("allowed", 1200, 780);
        let (x, y) = centre_of(&find(&w, &ui, "Current Location"));
        click(&mut w, &ctx, &ui, x, y);
        let sent = calls(&ctx);
        assert!(w.service_reply(&ctx, sent[0].0, &Ok(json(r#"{"outcome":"already_granted"}"#))));
        let ask = calls(&ctx);
        assert_eq!(methods(&ask), vec!["current"]);
        w.service_reply(&ctx, ask[0].0, &Ok(fix_json(65.68, -18.1, 100.0)));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(methods(&calls(&ctx)), vec!["current"], "the second Locate goes straight to the position");
    }

    #[test]
    fn a_refused_permission_stops_the_locate_and_says_why() {
        let (mut w, ui, ctx) = window("refused", 1200, 780);
        let (x, y) = centre_of(&find(&w, &ui, "Current Location"));
        click(&mut w, &ctx, &ui, x, y);
        let sent = calls(&ctx);
        assert!(w.service_reply(&ctx, sent[0].0, &Ok(json(r#"{"outcome":"refused"}"#))));
        assert!(!w.maps.locating);
        assert!(w.maps.note.as_deref().unwrap().contains("may not use your location"));
        assert!(w.maps.here.is_none());
    }

    #[test]
    fn replies_to_other_calls_are_left_alone() {
        let (mut w, _, ctx) = window("others", 1200, 780);
        assert!(!w.service_reply(&ctx, 987_654, &Ok(Json::Null)));
        assert!(!w.service_event(&ctx, "network_path", "pathUpdate", &Json::Null));
    }

    /// 1000 m north of (64, -21), a right turn, 1000 m east.
    fn l_route() -> Route {
        let b = (64.0 + 1000.0 / 111_194.9, -21.0);
        let c = (b.0, b.1 + 1000.0 / (111_194.9 * b.0.to_radians().cos()));
        let step = |name: &str, maneuver: &str, distance_m: f64| route::Step { name: name.into(), maneuver: maneuver.into(), distance_m };
        Route {
            distance_m: 2000.0,
            duration_s: 200.0,
            steps: vec![step("Main Street", "depart", 1000.0), step("Side Road", "turn right", 1000.0), step("", "arrive", 0.0)],
            approximate: false,
            geometry: vec![(64.0, -21.0), b, c],
        }
    }

    #[test]
    fn guidance_follows_positions_from_the_location_service_until_it_ends() {
        let (mut w, ui, ctx) = window("guidance", 1200, 780);
        let route = l_route();
        w.maps.route_from = Some(("A".into(), (64.0, -21.0)));
        w.maps.route_to = Some(("B".into(), *route.geometry.last().unwrap()));
        w.maps.panel = Panel::Directions;
        w.maps.route_done(Ok(vec![route]));
        let (x, y) = centre_of(&find(&w, &ui, "Start Navigation"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.panel, Panel::Navigation);
        let sent = calls(&ctx);
        assert_eq!(methods(&sent), vec!["location_request"]);
        w.service_reply(&ctx, sent[0].0, &Ok(json(r#"{"outcome":"already_granted"}"#)));
        let go = calls(&ctx);
        assert_eq!(methods(&go), vec!["current", "start"]);
        assert!(w.loc.is_updating());
        // a position 100 m along the route arrives as an update event
        let body = json(&format!(r#"{{"locations":[{}]}}"#, fix_json(64.0 + 100.0 / 111_194.9, -21.0, 10.0).encode()));
        assert!(w.service_event(&ctx, "location", "didUpdateLocations", &body));
        assert_eq!(w.maps.nav_title(), "Turn right onto Side Road");
        find(&w, &ui, "End Navigation");
        let (x, y) = centre_of(&find(&w, &ui, "End Navigation"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(methods(&calls(&ctx)), vec!["stop"]);
        assert_eq!(w.maps.panel, Panel::Directions);
        assert!(w.maps.nav.is_none());
    }

    #[test]
    fn the_transit_button_is_on_the_directions_card() {
        let (mut w, ui, ctx) = window("transit-button", 1200, 780);
        w.maps.choose(crate::geocode::Place::new("Perlan, Reykjavik", 64.1295, -21.9186));
        let (x, y) = centre_of(&find(&w, &ui, "Directions"));
        click(&mut w, &ctx, &ui, x, y);
        let (x, y) = centre_of(&find(&w, &ui, "Transit"));
        click(&mut w, &ctx, &ui, x, y);
        assert_eq!(w.maps.profile, route::Profile::Transit);
        assert!(w.maps.note.as_deref().unwrap().contains("transit_url"));
    }

    #[test]
    fn the_traffic_layer_loads_its_own_tiles_and_draws_over_the_map() {
        let root = std::env::temp_dir().join(format!("maps-window-traffic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let template = "https://traffic.example.org/{z}/{x}/{y}.png";
        std::fs::write(root.join("settings.json"), format!(r#"{{"traffic_url":"{template}","traffic":true}}"#)).unwrap();
        let ui = ui();
        let mut w = Window::new(&ui, 1200, 780, root.clone(), "1.0.0", Arc::new(Offline));
        w.set_online(false);
        let ctx = ctx(1200, 780);
        assert!(w.maps.show_traffic && w.traffic.is_some());
        // a cached traffic tile for the first tile on screen (once the map has its real size)
        w.tick(&ctx, &ui, 500.0);
        let (id, _, _) = w.maps.view.tiles()[0];
        let source = crate::source::TileSource::new(template, "ua").unwrap();
        let tile = springboard::pngenc::encode_rgb(4, 4, &[0xff80_4020; 16]);
        crate::cache::TileCache::new(&root, &source.overlay_cache_key()).write(id, &tile).unwrap();
        w.tick(&ctx, &ui, 1000.0);
        assert!(w.traffic.as_ref().unwrap().image(id).is_some(), "the traffic tile was read from its own folder");
        assert!(w.loader.image(id).is_none(), "and not as a base tile");
        let (width, height) = (1200usize, 780usize);
        let mut px = vec![0u32; width * height];
        let mut s = Surface { w: width, h: height, stride: width, px: &mut px, y0: 0 };
        w.draw(&mut s, &ui, 0.0);
        assert!(px.iter().any(|p| *p != px[0]));
    }

    #[test]
    fn the_position_and_the_guidance_card_are_drawn() {
        let (mut w, ui, _) = window("draw-here", 1200, 780);
        let (lat, lon) = w.maps.view.centre();
        w.maps.here = Some(Here { lat, lon, accuracy_m: 500.0 });
        let route = l_route();
        w.maps.route_from = Some(("A".into(), (64.0, -21.0)));
        w.maps.route_to = Some(("B".into(), *route.geometry.last().unwrap()));
        w.maps.panel = Panel::Directions;
        w.maps.route_done(Ok(vec![route]));
        w.maps.act(Act::StartNavigation, 0);
        let (width, height) = (1200usize, 780usize);
        let mut px = vec![0u32; width * height];
        let mut s = Surface { w: width, h: height, stride: width, px: &mut px, y0: 0 };
        w.draw(&mut s, &ui, 0.0);
        let first = px[0];
        assert!(px.iter().any(|p| *p != first));
    }
}
