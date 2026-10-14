//! Everything the Maps window shows and every command it runs, with no drawing and no network of its own. A command
//! (`Act`) changes the state and returns the `Effect`s the window must carry out (a request, the clipboard, a panel of
//! the shell). Finished requests come back through `search_done`, `route_done` and `gpx_loaded`.

use crate::cache::{download_estimate, OfflineArea, MAX_OFFLINE_TILES};
use crate::geocode::{parse_coordinates, Geocoder, Place, Suggestor, DEFAULT_BASE};
use crate::gpx;
use crate::links::{self, Endpoint, LinkAction};
use crate::measure::{path_length_m, Measure};
use crate::nav::{self, Guidance, Navigator};
use crate::route::{self, Profile, Route, Router};
use crate::source::{user_agent, MapType, TileSource};
use crate::store::{self, Library, Saved};
use crate::tiles::TileId;
use crate::transit::Planner;
use crate::units::{format_distance, format_duration, DistanceUnits, UnitsSetting};
use crate::view::{MapView, Pins};
use bifrost_app::controls::TextField;
use bifrost_core::json::{self, Json};
use std::path::{Path, PathBuf};

/// Reykjavik, until the person has moved the map.
pub const START: (f64, f64, f64) = (64.1466, -21.9426, 11.0);
/// Zoom a searched place is shown at (street level), unless the map is already closer.
pub const PLACE_ZOOM: f64 = 15.0;
/// Zoom levels a Download Map adds to the level on screen.
pub const DOWNLOAD_LEVELS: u8 = 2;
/// Zoom step of the buttons and the menu.
pub const ZOOM_STEP: f64 = 1.0;
/// Fewest milliseconds between two requests for a new route while the person is off the route.
pub const REROUTE_MIN_MS: u64 = 15_000;
/// A position this accurate (metres) or better is shown at street level; a coarser one at a wider zoom.
const STREET_FIX_M: f64 = 200.0;

/// The card beside the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    None,
    Place,
    Directions,
    Navigation,
    Measure,
    Download,
}

/// Where the person is, as the shell's location service reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Here {
    pub lat: f64,
    pub lon: f64,
    /// radius of uncertainty, metres
    pub accuracy_m: f64,
}

impl Here {
    pub fn lat_lon(&self) -> (f64, f64) {
        (self.lat, self.lon)
    }
}

/// The zoom that shows a position of the given accuracy: street level for a good fix, wider for a coarse one (an IP
/// lookup is good to a city, not a street).
pub fn zoom_for_accuracy(accuracy_m: f64) -> f64 {
    match accuracy_m {
        a if a <= STREET_FIX_M => PLACE_ZOOM,
        a if a <= 2_000.0 => 13.0,
        a if a <= 30_000.0 => 11.0,
        _ => 9.0,
    }
}

/// Every command. Menu items, toolbar buttons and the card's buttons all run one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    ZoomIn,
    ZoomOut,
    MapType(MapType),
    Units(UnitsSetting),
    Search,
    Directions,
    DropPin,
    RemovePin,
    Measure,
    MeasureUndo,
    MeasureClear,
    Download,
    StartDownload,
    CancelDownload,
    ClosePanel,
    Profile(Profile),
    PickRoute(usize),
    SwapEnds,
    ToggleFavorite,
    SetHome,
    SetWork,
    GoHome,
    GoWork,
    CopyLink,
    CopyCoordinates,
    Share,
    Print,
    NewCollection,
    AddToCollection(usize),
    OpenGpx,
    ClearRecents,
    Escape,
    /// centre the map on the person's position
    Locate,
    /// draw the traffic layer over the map
    ToggleTraffic,
    /// follow the route on the card, turn by turn
    StartNavigation,
    EndNavigation,
}

/// The commands that have a menu item, in menu order; an item's id is its position plus one.
pub const MENU_ACTS: &[Act] = &[
    Act::OpenGpx,
    Act::Print,
    Act::CopyLink,
    Act::CopyCoordinates,
    Act::Share,
    Act::MapType(MapType::Standard),
    Act::MapType(MapType::Satellite),
    Act::ZoomIn,
    Act::ZoomOut,
    Act::Units(UnitsSetting::Automatic),
    Act::Units(UnitsSetting::Kilometres),
    Act::Units(UnitsSetting::Miles),
    Act::Measure,
    Act::Download,
    Act::Search,
    Act::Directions,
    Act::DropPin,
    Act::RemovePin,
    Act::ToggleFavorite,
    Act::GoHome,
    Act::GoWork,
    Act::SetHome,
    Act::SetWork,
    Act::NewCollection,
    Act::ClearRecents,
    Act::ToggleTraffic,
    Act::Locate,
    Act::StartNavigation,
    Act::EndNavigation,
];

impl Act {
    pub fn menu_id(self) -> Option<u32> {
        MENU_ACTS.iter().position(|a| *a == self).map(|i| i as u32 + 1)
    }

    pub fn from_menu_id(id: u32) -> Option<Act> {
        MENU_ACTS.get((id as usize).checked_sub(1)?).copied()
    }
}

/// What the window has to do besides redrawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// ask the geocoder; the worker waits `wait_ms` first to keep to one request a second
    Search { query: String, wait_ms: u64 },
    /// ask the routing server
    Route { url: String },
    Copy(String),
    /// the share sheet with a link
    Share(String),
    Print { title: String, text: String },
    OpenGpx,
    /// another map type: the tile loader changes source
    ReloadTiles,
    StartDownload(Vec<TileId>),
    CancelDownload,
    FocusSearch,
    /// ask the transit planner; the reply is a list of journeys
    Plan { url: String },
    /// ask the location service for the person's position (asking permission first when needed)
    Locate,
    /// keep receiving positions while a route is followed, or stop
    TrackLocation(bool),
}

/// One row under the search field.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub title: String,
    pub subtitle: String,
    pub place: Place,
    /// from the saved places or a typed coordinate, not from the geocoder
    pub local: bool,
}

pub struct Maps {
    root: PathBuf,
    pub view: MapView,
    pub pins: Pins,
    pub map_type: MapType,
    pub units_setting: UnitsSetting,
    pub region_metric: bool,
    pub library: Library,
    pub panel: Panel,
    pub query: TextField,
    suggestor: Suggestor,
    pub results: Vec<Place>,
    pub searching: bool,
    pub selected: Option<Place>,
    /// the pin the card shows, when it is one of the dropped pins
    pub selected_pin: Option<usize>,
    pub profile: Profile,
    pub route_from: Option<(String, (f64, f64))>,
    pub route_to: Option<(String, (f64, f64))>,
    pub routes: Vec<Route>,
    pub route_index: usize,
    pub routing: bool,
    pub measure: Measure,
    pub tracks: Vec<(String, Vec<(f64, f64)>)>,
    pub offline_area: Option<OfflineArea>,
    pub progress: Option<(usize, usize)>,
    /// the collection whose places show on the map
    pub shown_collection: Option<usize>,
    /// one line of news under the card title ("Routing unavailable", "No places found")
    pub note: Option<String>,
    /// the person's position, once the location service has reported it
    pub here: Option<Here>,
    /// a Locate is waiting for the first position
    pub locating: bool,
    /// the traffic layer is drawn over the map
    pub show_traffic: bool,
    /// the route being followed turn by turn
    pub nav: Option<Navigator>,
    pub guidance: Option<Guidance>,
    /// the latest call of the guidance ("In 150 m, turn left onto Main Street")
    pub announcement: Option<String>,
    rerouting: bool,
    last_reroute_ms: Option<u64>,
    router: Router,
    planner: Planner,
    traffic: Option<TileSource>,
    geocoder: Geocoder,
    version: String,
    dirty_settings: bool,
}

/// True unless the language setting names a region that measures in miles.
pub fn region_is_metric() -> bool {
    let lang = ["LC_MEASUREMENT", "LC_ALL", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default();
    !["_US", "_LR", "_MM"].iter().any(|r| lang.contains(r)) && !lang.contains("_GB")
}

impl Maps {
    pub fn new(root: &Path, version: &str, width: f64, height: f64) -> Maps {
        let settings = std::fs::read_to_string(root.join("settings.json")).ok().and_then(|t| json::parse(&t).ok());
        let s = settings.as_ref();
        let num = |k: &str| s.and_then(|j| j.get(k)).and_then(Json::number);
        let text = |k: &str| s.and_then(|j| j.str_field(k)).unwrap_or_default();
        let (lat, lon, zoom) = (num("lat").unwrap_or(START.0), num("lon").unwrap_or(START.1), num("zoom").unwrap_or(START.2));
        let mut pins = Pins::default();
        if let Some(list) = s.and_then(|j| j.get("pins")).and_then(Json::as_array) {
            for p in list {
                if let (Some(la), Some(lo)) = (p.get("lat").and_then(Json::number), p.get("lon").and_then(Json::number)) {
                    let _ = pins.add(&p.str_field("label").unwrap_or_default(), la, lo);
                }
            }
        }
        let geocoder_base = Some(text("geocoder_url")).filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_BASE.to_string());
        let geocoder = Geocoder::new(&geocoder_base, &user_agent(version)).or_else(|_| Geocoder::new(DEFAULT_BASE, &user_agent(version))).expect("the default geocoder is valid");
        let routing = Some(text("routing_url")).filter(|u| !u.is_empty());
        let router = Router::new(routing.as_deref()).unwrap_or_else(|_| Router::new(None).expect("no routing server is valid"));
        let transit_base = Some(text("transit_url")).filter(|u| !u.is_empty());
        let planner = Planner::new(transit_base.as_deref()).unwrap_or_else(|_| Planner::new(None).expect("no transit planner is valid"));
        let traffic = Some(text("traffic_url")).filter(|u| !u.is_empty()).and_then(|u| TileSource::new(&u, &user_agent(version)).ok());
        let show_traffic = traffic.is_some() && matches!(s.and_then(|j| j.get("traffic")), Some(Json::Bool(true)));
        Maps {
            root: root.to_path_buf(),
            view: MapView::new(lat, lon, zoom, width, height),
            pins,
            map_type: MapType::from_key(&text("map_type")),
            units_setting: UnitsSetting::from_key(&text("units")),
            region_metric: region_is_metric(),
            library: store::load(root),
            panel: Panel::None,
            query: TextField::new(),
            suggestor: Suggestor::default(),
            results: Vec::new(),
            searching: false,
            selected: None,
            selected_pin: None,
            profile: Profile::Driving,
            route_from: None,
            route_to: None,
            routes: Vec::new(),
            route_index: 0,
            routing: false,
            measure: Measure::default(),
            tracks: Vec::new(),
            offline_area: None,
            progress: None,
            shown_collection: None,
            note: None,
            here: None,
            locating: false,
            show_traffic,
            nav: None,
            guidance: None,
            announcement: None,
            rerouting: false,
            last_reroute_ms: None,
            router,
            planner,
            traffic,
            geocoder,
            version: version.to_string(),
            dirty_settings: false,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn geocoder(&self) -> &Geocoder {
        &self.geocoder
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    /// The tile source of the traffic layer, when `traffic_url` is set in settings.json and valid.
    pub fn traffic_source(&self) -> Option<&TileSource> {
        self.traffic.as_ref()
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.view.resize(width, height);
    }

    /// The view moved or a setting changed: the next `persist` writes the settings.
    pub fn touch_settings(&mut self) {
        self.dirty_settings = true;
    }

    /// Writes the settings and the saved places when they changed.
    pub fn persist(&mut self) {
        let _ = store::save(&self.root, &self.library);
        if !self.dirty_settings {
            return;
        }
        self.dirty_settings = false;
        let (lat, lon) = self.view.centre();
        let pins = Json::Arr(
            self.pins
                .items()
                .iter()
                .map(|p| Json::Obj(vec![("label".into(), Json::Str(p.label.clone())), ("lat".into(), Json::Num(p.lat)), ("lon".into(), Json::Num(p.lon))]))
                .collect(),
        );
        let mut fields = vec![
            ("map_type".to_string(), Json::Str(self.map_type.key().into())),
            ("units".to_string(), Json::Str(self.units_setting.key().into())),
            ("lat".to_string(), Json::Num(lat)),
            ("lon".to_string(), Json::Num(lon)),
            ("zoom".to_string(), Json::Num(self.view.zoom())),
            ("pins".to_string(), pins),
            ("traffic".to_string(), Json::Bool(self.show_traffic)),
        ];
        // the servers are kept as the person set them in settings.json
        if let Some(old) = std::fs::read_to_string(self.root.join("settings.json")).ok().and_then(|t| json::parse(&t).ok()) {
            for k in ["geocoder_url", "routing_url", "transit_url", "traffic_url"] {
                if let Some(v) = old.str_field(k) {
                    fields.push((k.to_string(), Json::Str(v)));
                }
            }
        }
        let _ = std::fs::create_dir_all(&self.root);
        let tmp = self.root.join("settings.json.part");
        if std::fs::write(&tmp, Json::Obj(fields).encode()).is_ok() {
            let _ = std::fs::rename(&tmp, self.root.join("settings.json"));
        }
    }

    // ---- units ----

    pub fn units(&self) -> DistanceUnits {
        self.units_setting.resolve(self.region_metric)
    }

    pub fn distance_text(&self, metres: f64) -> String {
        format_distance(metres, self.units())
    }

    // ---- search ----

    /// The search field changed: the suggestions are asked for after a pause.
    pub fn query_edited(&mut self, now_ms: u64) {
        let q = self.query.to_string();
        self.suggestor.typed(&q, now_ms);
        if q.trim().is_empty() {
            self.results.clear();
            self.searching = false;
            self.note = None;
        }
    }

    /// The geocoder request due now, if any (suggestions while typing).
    pub fn due_search(&mut self, now_ms: u64) -> Option<Effect> {
        let query = self.suggestor.due(now_ms)?;
        self.suggestor.sent(&query, now_ms);
        self.searching = true;
        Some(Effect::Search { query, wait_ms: 0 })
    }

    /// When `due_search` next has something to send (ms), so the app wakes for it.
    pub fn next_search_wake(&self, now_ms: u64) -> Option<u64> {
        self.suggestor.next_due(now_ms)
    }

    /// Return in the search field: a coordinate is shown at once, a first suggestion is chosen, else the text is searched.
    pub fn submit_search(&mut self, now_ms: u64) -> Vec<Effect> {
        let q = self.query.to_string();
        let q = q.trim();
        if q.is_empty() {
            return vec![];
        }
        if let Some((lat, lon)) = parse_coordinates(q) {
            self.choose(Place::new(&links::coordinates_text(lat, lon), lat, lon));
            return vec![];
        }
        if let Some(first) = self.suggestions().into_iter().next().filter(|s| s.local) {
            self.choose(first.place);
            return vec![];
        }
        let wait_ms = self.suggestor.wait_for_request(now_ms);
        self.suggestor.sent(q, now_ms + wait_ms);
        self.searching = true;
        self.note = None;
        vec![Effect::Search { query: q.to_string(), wait_ms }]
    }

    /// A geocoder reply for `query`; replies for text that is no longer in the field are dropped.
    pub fn search_done(&mut self, query: &str, result: Result<Vec<Place>, String>) {
        if query.trim() != self.query.trim() {
            return;
        }
        self.searching = false;
        match result {
            Ok(places) => {
                self.note = places.is_empty().then(|| "No places found".to_string());
                self.results = places;
            }
            Err(_) => {
                self.results.clear();
                self.note = Some("Search is unavailable".to_string());
            }
        }
    }

    /// The rows under the search field: a typed coordinate, matching saved places, then the geocoder's places.
    pub fn suggestions(&self) -> Vec<Suggestion> {
        let q = self.query.to_string();
        let q = q.trim();
        if q.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        if let Some((lat, lon)) = parse_coordinates(q) {
            let name = links::coordinates_text(lat, lon);
            out.push(Suggestion { title: name.clone(), subtitle: "Coordinates".into(), place: Place::new(&name, lat, lon), local: true });
        }
        let saved = self.library.all_places();
        for p in route::search_offline(&saved, q, 3) {
            out.push(Suggestion { title: p.title().to_string(), subtitle: "Saved place".into(), place: p.clone(), local: true });
        }
        for p in &self.results {
            if out.iter().any(|s| (s.place.lat - p.lat).abs() < 1e-4 && (s.place.lon - p.lon).abs() < 1e-4) {
                continue;
            }
            let subtitle = if p.details.kind.is_empty() { p.name.split(',').skip(1).collect::<Vec<_>>().join(",").trim().to_string() } else { p.details.kind.clone() };
            out.push(Suggestion { title: p.title().to_string(), subtitle, place: p.clone(), local: false });
        }
        out
    }

    /// Shows a place chosen from the suggestions or the saved places: the map centres on it and its card opens.
    pub fn choose(&mut self, place: Place) {
        self.query.clear();
        self.results.clear();
        self.searching = false;
        self.suggestor = Suggestor::default();
        self.show_place(place);
    }

    pub fn show_place(&mut self, place: Place) {
        self.view.centre_to(place.lat, place.lon);
        if self.view.zoom() < PLACE_ZOOM - 1.0 {
            self.view.set_zoom(PLACE_ZOOM);
        }
        self.library.add_recent(Saved::new(&place.name, place.lat, place.lon));
        self.selected = Some(place);
        self.selected_pin = None;
        self.panel = Panel::Place;
        self.note = None;
        self.shown_collection = None;
        self.touch_settings();
    }

    // ---- taps on the map ----

    /// A tap on the map at (`lat`, `lon`); `pin` is the dropped pin under the finger, if any.
    pub fn tap(&mut self, lat: f64, lon: f64, pin: Option<usize>) {
        if self.panel == Panel::Measure {
            self.measure.add(lat, lon);
            return;
        }
        match pin {
            Some(i) => self.select_pin(i),
            None => {
                if self.panel == Panel::Place {
                    self.panel = Panel::None;
                }
                self.selected = None;
                self.selected_pin = None;
            }
        }
    }

    /// Drop Pin at a position (the context menu of the map): the pin is added and its card opens.
    pub fn drop_pin_at(&mut self, lat: f64, lon: f64) {
        let label = "Dropped Pin";
        if self.pins.add(label, lat, lon).is_err() {
            return;
        }
        let i = self.pins.items().len() - 1;
        self.select_pin(i);
        self.touch_settings();
    }

    fn select_pin(&mut self, i: usize) {
        let Some(p) = self.pins.items().get(i).cloned() else { return };
        self.selected = Some(Place::new(&p.label, p.lat, p.lon));
        self.selected_pin = Some(i);
        self.panel = Panel::Place;
    }

    pub fn double_tap(&mut self, ax: f64, ay: f64) {
        self.view.zoom_at(2.0, ax, ay);
        self.touch_settings();
    }

    // ---- commands ----

    /// The position commands act on: the place on the card, else the middle of the map.
    pub fn target(&self) -> (String, (f64, f64)) {
        match &self.selected {
            Some(p) => (p.title().to_string(), (p.lat, p.lon)),
            None => {
                let (lat, lon) = self.view.centre();
                ("Map centre".to_string(), (lat, lon))
            }
        }
    }

    pub fn can(&self, act: Act) -> bool {
        let has_place = self.selected.is_some();
        match act {
            Act::Print => self.selected.is_some() || !self.routes.is_empty(),
            Act::ToggleFavorite => has_place,
            Act::CopyLink | Act::CopyCoordinates | Act::Share => true,
            Act::RemovePin => self.selected_pin.is_some(),
            Act::GoHome => self.library.home.is_some(),
            Act::GoWork => self.library.work.is_some(),
            Act::MeasureUndo | Act::MeasureClear => !self.measure.points().is_empty(),
            Act::PickRoute(i) => i < self.routes.len(),
            Act::AddToCollection(i) => has_place && i < self.library.collections.len(),
            Act::StartDownload => self.offline_area.is_some_and(|a| a.count().1) && self.progress.is_none(),
            Act::ClearRecents => !self.library.recents.is_empty(),
            Act::StartNavigation => self.panel == Panel::Directions && self.current_route().is_some_and(|r| nav::can_follow(r, self.profile)),
            Act::EndNavigation => self.nav.is_some(),
            _ => true,
        }
    }

    pub fn checked(&self, act: Act) -> bool {
        match act {
            Act::ToggleTraffic => self.show_traffic,
            Act::MapType(m) => self.map_type == m,
            Act::Units(u) => self.units_setting == u,
            Act::Measure => self.panel == Panel::Measure,
            Act::Profile(p) => self.profile == p,
            Act::ToggleFavorite => self.selected.as_ref().is_some_and(|p| self.library.is_favorite(p.lat, p.lon)),
            _ => false,
        }
    }

    pub fn act(&mut self, act: Act, now_ms: u64) -> Vec<Effect> {
        if !self.can(act) {
            return vec![];
        }
        match act {
            Act::ZoomIn => self.zoom_by(ZOOM_STEP),
            Act::ZoomOut => self.zoom_by(-ZOOM_STEP),
            Act::MapType(m) => {
                if self.map_type != m {
                    self.map_type = m;
                    self.touch_settings();
                    return vec![Effect::ReloadTiles];
                }
            }
            Act::Units(u) => {
                self.units_setting = u;
                self.touch_settings();
            }
            Act::Search => return vec![Effect::FocusSearch],
            Act::Directions => return self.open_directions(),
            Act::DropPin => {
                let (_, (lat, lon)) = self.target();
                match self.selected_pin {
                    Some(_) => {}
                    None => self.drop_pin_at(lat, lon),
                }
            }
            Act::RemovePin => {
                if let Some(i) = self.selected_pin.take() {
                    self.pins.remove(i);
                    self.selected = None;
                    self.panel = Panel::None;
                    self.touch_settings();
                }
            }
            Act::Measure => {
                if self.panel == Panel::Measure {
                    self.panel = Panel::None;
                } else {
                    self.panel = Panel::Measure;
                    self.measure.clear();
                }
            }
            Act::MeasureUndo => {
                self.measure.undo();
            }
            Act::MeasureClear => self.measure.clear(),
            Act::Download => {
                self.panel = Panel::Download;
                self.offline_area = Some(OfflineArea::of_view(&self.view, DOWNLOAD_LEVELS));
            }
            Act::StartDownload => {
                let tiles = self.offline_area.map(|a| a.tiles()).unwrap_or_default();
                if tiles.is_empty() {
                    return vec![];
                }
                self.progress = Some((0, tiles.len()));
                return vec![Effect::StartDownload(tiles)];
            }
            Act::CancelDownload => {
                self.progress = None;
                return vec![Effect::CancelDownload];
            }
            Act::ClosePanel => return self.close_panel(),
            Act::Escape => {
                if self.panel != Panel::None {
                    return self.close_panel();
                }
                self.selected = None;
                self.selected_pin = None;
            }
            Act::Profile(p) => {
                self.profile = p;
                return self.request_route();
            }
            Act::PickRoute(i) => {
                self.route_index = i;
                if let Some(r) = self.routes.get(i) {
                    let pts = r.geometry.clone();
                    self.view.fit(&pts, 80.0);
                }
            }
            Act::SwapEnds => {
                std::mem::swap(&mut self.route_from, &mut self.route_to);
                return self.request_route();
            }
            Act::ToggleFavorite => {
                if let Some(p) = &self.selected {
                    let saved = Saved::new(&p.name, p.lat, p.lon);
                    self.library.toggle_favorite(saved);
                }
            }
            Act::SetHome => {
                let (name, (lat, lon)) = self.target();
                let full = self.selected.as_ref().map_or(name, |p| p.name.clone());
                self.library.set_home(Some(Saved::new(&full, lat, lon)));
            }
            Act::SetWork => {
                let (name, (lat, lon)) = self.target();
                let full = self.selected.as_ref().map_or(name, |p| p.name.clone());
                self.library.set_work(Some(Saved::new(&full, lat, lon)));
            }
            Act::GoHome => {
                if let Some(h) = self.library.home.clone() {
                    self.show_place(h.to_place());
                }
            }
            Act::GoWork => {
                if let Some(w) = self.library.work.clone() {
                    self.show_place(w.to_place());
                }
            }
            Act::CopyLink => return vec![Effect::Copy(self.link())],
            Act::CopyCoordinates => {
                let (_, (lat, lon)) = self.target();
                return vec![Effect::Copy(links::coordinates_text(lat, lon))];
            }
            Act::Share => return vec![Effect::Share(self.link())],
            Act::Print => {
                let (title, text) = self.print_text();
                return vec![Effect::Print { title, text }];
            }
            Act::NewCollection => {
                let name = (1..)
                    .map(|n| format!("Collection {n}"))
                    .find(|n| !self.library.collections.iter().any(|c| c.name.eq_ignore_ascii_case(n)))
                    .unwrap_or_else(|| "Collection".to_string());
                if let Ok(i) = self.library.new_collection(&name) {
                    if let Some(p) = &self.selected {
                        self.library.add_to_collection(i, Saved::new(&p.name, p.lat, p.lon));
                    }
                }
            }
            Act::AddToCollection(i) => {
                if let Some(p) = &self.selected {
                    self.library.add_to_collection(i, Saved::new(&p.name, p.lat, p.lon));
                }
            }
            Act::OpenGpx => return vec![Effect::OpenGpx],
            Act::ClearRecents => self.library.clear_recents(),
            Act::Locate => {
                self.locating = true;
                self.note = Some("Finding your location\u{2026}".to_string());
                return vec![Effect::Locate];
            }
            Act::ToggleTraffic => {
                if self.traffic.is_none() {
                    self.note = Some("No traffic layer is set up: add traffic_url to settings.json".to_string());
                } else {
                    self.show_traffic = !self.show_traffic;
                    self.touch_settings();
                }
            }
            Act::StartNavigation => return self.start_navigation(now_ms),
            Act::EndNavigation => return self.end_navigation(),
        }
        let _ = now_ms;
        vec![]
    }

    fn zoom_by(&mut self, step: f64) {
        self.view.set_zoom(self.view.zoom() + step);
        self.touch_settings();
    }

    fn close_panel(&mut self) -> Vec<Effect> {
        if self.panel == Panel::Navigation {
            // the card closes back to the directions it was started from
            return self.end_navigation();
        }
        let was = self.panel;
        self.panel = if was == Panel::Place || self.selected.is_none() { Panel::None } else { Panel::Place };
        if was == Panel::Download && self.progress.is_some() {
            self.progress = None;
            return vec![Effect::CancelDownload];
        }
        if was == Panel::Place {
            self.selected = None;
            self.selected_pin = None;
        }
        if was == Panel::Directions {
            self.routes.clear();
        }
        vec![]
    }

    /// Shows a saved collection's places on the map.
    pub fn show_collection(&mut self, index: usize) {
        let Some(c) = self.library.collections.get(index) else { return };
        let pts: Vec<(f64, f64)> = c.places.iter().map(|p| (p.lat, p.lon)).collect();
        self.shown_collection = Some(index);
        if !pts.is_empty() {
            self.view.fit(&pts, 80.0);
        }
        self.touch_settings();
    }

    // ---- directions ----

    fn open_directions(&mut self) -> Vec<Effect> {
        let (name, to) = self.target();
        // the start: one the person chose, else where they are, else Home, else the middle of the map
        let from = match (&self.library.home, self.route_from.as_ref(), self.here) {
            (_, Some(f), _) if f.1 != to => f.clone(),
            (_, _, Some(h)) if (h.lat, h.lon) != to => ("Current location".to_string(), (h.lat, h.lon)),
            (Some(h), _, _) if (h.lat, h.lon) != to => (h.title().to_string(), (h.lat, h.lon)),
            _ => {
                let (lat, lon) = self.view.centre();
                ("Map centre".to_string(), (lat, lon))
            }
        };
        self.route_from = Some(from);
        self.route_to = Some((name, to));
        self.panel = Panel::Directions;
        self.request_route()
    }

    /// Asks the routing server, or shows the labelled straight-line estimate when none is configured.
    pub fn request_route(&mut self) -> Vec<Effect> {
        let (Some((_, from)), Some((_, to))) = (self.route_from.clone(), self.route_to.clone()) else { return vec![] };
        // a new request replaces the route that was being followed
        let mut effects: Vec<Effect> = self.stop_navigation().into_iter().collect();
        self.routes.clear();
        self.route_index = 0;
        self.note = None;
        self.rerouting = false;
        if self.profile == Profile::Transit {
            match self.planner.plan_url(from, to) {
                Ok(url) => {
                    self.routing = true;
                    effects.push(Effect::Plan { url });
                }
                Err(_) => self.note = Some("No transit planner is set up: add transit_url to settings.json".to_string()),
            }
            return effects;
        }
        match self.router.route_url(from, to, self.profile) {
            Ok(url) => {
                self.routing = true;
                effects.push(Effect::Route { url });
            }
            Err(_) => {
                self.routes = vec![route::estimate(from, to, self.profile)];
                self.note = Some("No routing server is set up: this is a straight-line estimate".to_string());
                self.fit_route();
            }
        }
        effects
    }

    pub fn route_done(&mut self, result: Result<Vec<Route>, String>) {
        self.routing = false;
        if self.rerouting {
            self.reroute_done(result);
            return;
        }
        match result {
            Ok(routes) if !routes.is_empty() => {
                self.routes = routes;
                self.route_index = 0;
                self.note = None;
            }
            Err(why) if self.profile == Profile::Transit => {
                // there is no straight-line stand-in for a timetable
                self.routes.clear();
                self.note = Some(format!("No journey found ({why})"));
                return;
            }
            other => {
                let (Some((_, from)), Some((_, to))) = (&self.route_from, &self.route_to) else { return };
                let why = other.err().unwrap_or_else(|| "no route found".into());
                self.routes = vec![route::estimate(*from, *to, self.profile)];
                self.route_index = 0;
                self.note = Some(format!("Directions are unavailable ({why}): a straight-line estimate"));
            }
        }
        self.fit_route();
    }

    fn fit_route(&mut self) {
        if let Some(r) = self.routes.get(self.route_index) {
            let pts = r.geometry.clone();
            self.view.fit(&pts, 80.0);
        }
    }

    pub fn current_route(&self) -> Option<&Route> {
        self.routes.get(self.route_index)
    }

    // ---- current location and turn-by-turn guidance ----

    /// The location service could not give a position (refused, none set up): Locate stops waiting and the reason shows.
    pub fn locate_failed(&mut self, why: &str) {
        self.locating = false;
        self.note = Some(why.to_string());
    }

    /// A position from the location service. A Locate that was waiting centres the map on it; while a route is
    /// followed the guidance moves on. Returns what the window must do (a request for a new route).
    pub fn position_update(&mut self, here: Here, now_ms: u64) -> Vec<Effect> {
        self.here = Some(here);
        if self.locating {
            self.locating = false;
            self.view.centre_to(here.lat, here.lon);
            let zoom = zoom_for_accuracy(here.accuracy_m);
            let zoom = if here.accuracy_m <= STREET_FIX_M { self.view.zoom().max(zoom) } else { zoom };
            self.view.set_zoom(zoom);
            let note = (here.accuracy_m > 1_000.0).then(|| format!("Approximate location, within about {}", self.distance_text(here.accuracy_m)));
            self.note = note;
            self.touch_settings();
        }
        self.follow(here, now_ms)
    }

    /// Moves the guidance to a position: the call that is due is kept for the card, the map follows the person, and
    /// a position off the route asks for a new route.
    fn follow(&mut self, here: Here, now_ms: u64) -> Vec<Effect> {
        let units = self.units();
        let Some(nav) = self.nav.as_mut() else { return vec![] };
        let guidance = nav.update((here.lat, here.lon), here.accuracy_m);
        let spoken = guidance.announce.and_then(|a| {
            let step = nav.route().steps.get(a.step)?;
            Some(nav::announcement_text(&step.instruction(), &format_distance(a.distance_m, units), a.now))
        });
        if let Some(text) = spoken {
            self.announcement = Some(text);
        }
        if !guidance.arrived {
            self.view.centre_to(here.lat, here.lon);
        }
        let off_route = guidance.off_route;
        self.guidance = Some(guidance);
        if off_route {
            self.reroute(here, now_ms)
        } else {
            vec![]
        }
    }

    /// Asks the routing server for a way from where the person is to the same destination. At most one request every
    /// `REROUTE_MIN_MS`, and none while another is running.
    fn reroute(&mut self, here: Here, now_ms: u64) -> Vec<Effect> {
        if self.routing || self.last_reroute_ms.is_some_and(|t| now_ms < t + REROUTE_MIN_MS) {
            return vec![];
        }
        let Some((_, to)) = self.route_to.clone() else { return vec![] };
        let from = (here.lat, here.lon);
        let Ok(url) = self.router.route_url(from, to, self.profile) else { return vec![] };
        self.last_reroute_ms = Some(now_ms);
        self.routing = true;
        self.rerouting = true;
        self.route_from = Some(("Current location".to_string(), from));
        self.note = Some("Finding a new route\u{2026}".to_string());
        vec![Effect::Route { url }]
    }

    /// The reply to a request for a new route: followed from the next position on, or the old route is kept.
    fn reroute_done(&mut self, result: Result<Vec<Route>, String>) {
        self.rerouting = false;
        let profile = self.profile;
        let followed = result.ok().and_then(|routes| {
            let nav = Navigator::new(routes.first()?.clone(), profile)?;
            Some((routes, nav))
        });
        match followed {
            Some((routes, nav)) => {
                self.routes = routes;
                self.route_index = 0;
                self.nav = Some(nav);
                self.guidance = None;
                self.announcement = Some("Following a new route".to_string());
                self.note = None;
            }
            None => self.note = Some("No new route was found".to_string()),
        }
    }

    fn start_navigation(&mut self, _now_ms: u64) -> Vec<Effect> {
        let Some(route) = self.current_route().cloned() else { return vec![] };
        let Some(nav) = Navigator::new(route, self.profile) else { return vec![] };
        self.nav = Some(nav);
        self.guidance = None;
        self.announcement = None;
        self.last_reroute_ms = None;
        self.rerouting = false;
        self.panel = Panel::Navigation;
        self.note = None;
        // the first position comes from the location service, never from an earlier Locate, which may be old
        vec![Effect::TrackLocation(true)]
    }

    fn end_navigation(&mut self) -> Vec<Effect> {
        self.stop_navigation().into_iter().collect()
    }

    /// Leaves guidance; the card goes back to the directions. `Some` when positions were being followed.
    fn stop_navigation(&mut self) -> Option<Effect> {
        let was = self.nav.take().is_some() || self.panel == Panel::Navigation;
        self.guidance = None;
        self.announcement = None;
        self.rerouting = false;
        if self.panel == Panel::Navigation {
            self.panel = Panel::Directions;
        }
        was.then_some(Effect::TrackLocation(false))
    }

    /// The large line of the guidance card: the next manoeuvre.
    pub fn nav_title(&self) -> String {
        match (&self.nav, &self.guidance) {
            (_, Some(g)) if g.arrived => "You have arrived".to_string(),
            (_, Some(g)) if g.off_route => "Off route".to_string(),
            (Some(nav), Some(g)) => match g.ahead.and_then(|i| nav.route().steps.get(i)) {
                Some(step) => step.instruction(),
                None => "Continue to your destination".to_string(),
            },
            _ => "Starting navigation".to_string(),
        }
    }

    /// The line under it: how far away the manoeuvre is.
    pub fn nav_subtitle(&self) -> String {
        match &self.guidance {
            None => "Waiting for your location".to_string(),
            Some(g) if g.arrived => self.route_to.as_ref().map(|(n, _)| n.clone()).unwrap_or_default(),
            Some(g) if g.off_route => "Finding a new route".to_string(),
            Some(g) => format!("in {}", self.distance_text(g.distance_m)),
        }
    }

    /// The time and distance left, once there is a position and the route is not finished.
    pub fn nav_remaining(&self) -> Option<String> {
        let g = self.guidance.as_ref().filter(|g| !g.arrived)?;
        Some(format!("{} \u{b7} {} remaining", format_duration(g.remaining_s), self.distance_text(g.remaining_m)))
    }

    // ---- links, files ----

    /// The link Copy Link and Share send: the place on the card, else the middle of the map; in Directions the route.
    pub fn link(&self) -> String {
        if self.panel == Panel::Directions {
            if let (Some((_, f)), Some((_, t))) = (&self.route_from, &self.route_to) {
                return links::route_link(*f, *t, self.profile);
            }
        }
        let (name, (lat, lon)) = self.target();
        let label = if self.selected.is_some() { name } else { String::new() };
        links::place_link(lat, lon, &label)
    }

    /// Opens a `maps:` or `geo:` link (also a shared URL). Text that needs a lookup goes to the search field.
    pub fn open_link(&mut self, link: &str, now_ms: u64) -> Vec<Effect> {
        match links::parse_link(link) {
            None => vec![],
            Some(LinkAction::Search(q)) => {
                self.query.clear();
                self.query.push_str(&q);
                self.query_edited(now_ms);
                self.submit_search(now_ms)
            }
            Some(LinkAction::Show { lat, lon, zoom, label }) => {
                let name = label.unwrap_or_else(|| links::coordinates_text(lat, lon));
                self.show_place(Place::new(&name, lat, lon));
                if let Some(z) = zoom {
                    self.view.set_zoom(z);
                }
                vec![]
            }
            Some(LinkAction::Route { from, to, profile }) => {
                let at = |e: &Endpoint| match e {
                    Endpoint::Coordinate(la, lo) => Some((links::coordinates_text(*la, *lo), (*la, *lo))),
                    Endpoint::Text(_) => None,
                };
                match (to, from) {
                    (Endpoint::Text(t), _) | (_, Some(Endpoint::Text(t))) => {
                        // a name has to be looked up before it can be routed
                        self.query.clear();
                        self.query.push_str(&t);
                        self.query_edited(now_ms);
                        self.submit_search(now_ms)
                    }
                    (to, from) => {
                        self.profile = profile;
                        let (_, centre) = self.target();
                        self.route_from = from.as_ref().and_then(at).or(Some(("Map centre".into(), centre)));
                        self.route_to = at(&to);
                        self.panel = Panel::Directions;
                        self.request_route()
                    }
                }
            }
        }
    }

    /// Reads a GPX file: waypoints become pins, tracks become lines. The text is the file's contents.
    pub fn gpx_loaded(&mut self, text: &str) -> Result<String, String> {
        let g = gpx::parse(text)?;
        if g.is_empty() {
            return Err("the file has no waypoints or tracks".into());
        }
        for (name, lat, lon) in &g.waypoints {
            let _ = self.pins.add(name, *lat, *lon);
        }
        let mut km = 0.0;
        for (name, pts) in &g.tracks {
            if pts.len() > 1 {
                km += path_length_m(pts);
                self.tracks.push((name.clone(), pts.clone()));
            }
        }
        self.view.fit(&g.all_points(), 80.0);
        self.touch_settings();
        let tracks = g.tracks.iter().filter(|(_, p)| p.len() > 1).count();
        Ok(format!("{} waypoint(s), {} track(s), {}", g.waypoints.len(), tracks, self.distance_text(km)))
    }

    // ---- text for the cards and Print ----

    /// "12 min" / "3.2 km" line of a route.
    pub fn route_line(&self, r: &Route) -> String {
        format!("{} \u{b7} {}", format_duration(r.duration_s), self.distance_text(r.distance_m))
    }

    pub fn measure_line(&self) -> String {
        let n = self.measure.points().len();
        match n {
            0 => "Click the map to place the first point".to_string(),
            1 => "Click the map to add the next point".to_string(),
            _ => format!("{} in total, last leg {}", self.distance_text(self.measure.total_m()), self.distance_text(self.measure.last_leg_m())),
        }
    }

    pub fn download_line(&self) -> String {
        match (self.offline_area, self.progress) {
            (_, Some((d, t))) => format!("Downloading {d} of {t} tiles"),
            (Some(a), None) => {
                let (n, ok) = a.count();
                if ok {
                    download_estimate(n)
                } else {
                    format!("This area is too large to keep offline (most {MAX_OFFLINE_TILES} tiles). Zoom in and try again.")
                }
            }
            _ => String::new(),
        }
    }

    /// The text Print sends: the route with its steps, else the place.
    pub fn print_text(&self) -> (String, String) {
        if self.panel == Panel::Directions {
            if let (Some((fname, _)), Some((tname, _)), Some(r)) = (&self.route_from, &self.route_to, self.current_route()) {
                let mut t = format!("Directions from {fname} to {tname}\n{}\n", self.route_line(r));
                if r.approximate {
                    t.push_str("A straight-line estimate, not a road route.\n");
                }
                t.push('\n');
                for (i, s) in r.steps.iter().enumerate() {
                    t.push_str(&format!("{}. {} ({})\n", i + 1, s.instruction(), self.distance_text(s.distance_m)));
                }
                return (format!("Directions to {tname}"), t);
            }
        }
        let (name, (lat, lon)) = self.target();
        let mut t = format!("{name}\n{}\n", links::coordinates_text(lat, lon));
        if let Some(p) = &self.selected {
            for line in &p.details.address {
                t.push_str(line);
                t.push('\n');
            }
            for (label, v) in [("Phone", &p.details.phone), ("Website", &p.details.website), ("Hours", &p.details.opening_hours)] {
                if !v.is_empty() {
                    t.push_str(&format!("{label}: {v}\n"));
                }
            }
        }
        t.push_str(&format!("\n{}\n", self.link()));
        (name, t)
    }

    /// The finished download: the card goes back to the map.
    pub fn download_done(&mut self) {
        self.progress = None;
        self.note = Some("The area is saved for offline use".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("maps-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn maps(name: &str) -> Maps {
        let mut m = Maps::new(&dir(name), "1.0.0", 800.0, 600.0);
        m.region_metric = true;
        m
    }

    fn type_query(m: &mut Maps, text: &str, now: u64) {
        m.query.clear();
        m.query.push_str(text);
        m.query_edited(now);
    }

    #[test]
    fn starts_in_reykjavik_with_no_panel() {
        let m = maps("start");
        assert_eq!(m.panel, Panel::None);
        assert!((m.view.centre().0 - START.0).abs() < 0.01);
    }

    #[test]
    fn menu_ids_round_trip() {
        for a in MENU_ACTS {
            assert_eq!(Act::from_menu_id(a.menu_id().unwrap()), Some(*a));
        }
        assert_eq!(Act::from_menu_id(0), None);
        assert_eq!(Act::from_menu_id(10_000), None);
    }

    #[test]
    fn zoom_commands_step_one_level() {
        let mut m = maps("zoom");
        let z = m.view.zoom();
        m.act(Act::ZoomIn, 0);
        assert_eq!(m.view.zoom(), z + 1.0);
        m.act(Act::ZoomOut, 0);
        m.act(Act::ZoomOut, 0);
        assert_eq!(m.view.zoom(), z - 1.0);
    }

    #[test]
    fn a_typed_coordinate_is_shown_without_a_request() {
        let mut m = maps("coord");
        type_query(&mut m, "64.1, -21.9", 0);
        assert!(m.due_search(5_000).is_none(), "a coordinate is never sent to the geocoder");
        assert!(m.submit_search(5_000).is_empty());
        assert_eq!(m.panel, Panel::Place);
        assert!((m.view.centre().0 - 64.1).abs() < 1e-6);
        assert!(m.query.is_empty());
    }

    #[test]
    fn suggestions_wait_for_a_pause_and_for_three_characters() {
        let mut m = maps("suggest");
        type_query(&mut m, "ha", 0);
        assert!(m.due_search(5_000).is_none());
        type_query(&mut m, "hallgr", 1_000);
        assert!(m.due_search(1_100).is_none(), "still typing");
        match m.due_search(2_000) {
            Some(Effect::Search { query, wait_ms }) => {
                assert_eq!(query, "hallgr");
                assert_eq!(wait_ms, 0);
            }
            other => panic!("{other:?}"),
        }
        assert!(m.due_search(2_100).is_none(), "the same text is not asked twice");
    }

    #[test]
    fn return_searches_at_once_but_keeps_to_a_request_a_second() {
        let mut m = maps("return");
        type_query(&mut m, "kringlan", 10_000);
        let fx = m.submit_search(10_000);
        assert_eq!(fx, vec![Effect::Search { query: "kringlan".into(), wait_ms: 0 }]);
        type_query(&mut m, "perlan", 10_100);
        match &m.submit_search(10_100)[..] {
            [Effect::Search { wait_ms, .. }] => assert!(*wait_ms > 0 && *wait_ms <= 1000, "{wait_ms}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_reply_for_old_text_is_dropped() {
        let mut m = maps("stale");
        type_query(&mut m, "perlan", 0);
        m.search_done("kringlan", Ok(vec![Place::new("Kringlan", 64.1, -21.9)]));
        assert!(m.results.is_empty());
        m.search_done("perlan", Ok(vec![Place::new("Perlan, Reykjavik", 64.12, -21.92)]));
        assert_eq!(m.suggestions().len(), 1);
        assert_eq!(m.suggestions()[0].title, "Perlan");
    }

    #[test]
    fn an_empty_reply_and_a_failed_one_leave_a_note() {
        let mut m = maps("note");
        type_query(&mut m, "zzzz", 0);
        m.search_done("zzzz", Ok(vec![]));
        assert_eq!(m.note.as_deref(), Some("No places found"));
        m.search_done("zzzz", Err("offline".into()));
        assert_eq!(m.note.as_deref(), Some("Search is unavailable"));
    }

    #[test]
    fn saved_places_are_suggested_before_the_geocoder_ones() {
        let mut m = maps("saved");
        m.library.toggle_favorite(Saved::new("Hallgrimskirkja, Reykjavik", 64.1417, -21.9266));
        type_query(&mut m, "hallgrims", 0);
        m.search_done("hallgrims", Ok(vec![Place::new("Hallgrimskirkja, Reykjavik", 64.1417, -21.9266), Place::new("Hallgrims Street", 65.0, -20.0)]));
        let s = m.suggestions();
        assert_eq!(s.len(), 2, "the repeat is left out: {s:?}");
        assert!(s[0].local && !s[1].local);
        assert_eq!(s[0].subtitle, "Saved place");
    }

    #[test]
    fn choosing_a_place_centres_opens_its_card_and_remembers_it() {
        let mut m = maps("choose");
        m.choose(Place::new("Perlan, Reykjavik", 64.1295, -21.9186));
        assert_eq!(m.panel, Panel::Place);
        assert_eq!(m.view.zoom(), PLACE_ZOOM);
        assert_eq!(m.library.recents[0].title(), "Perlan");
    }

    #[test]
    fn a_tap_on_empty_map_closes_the_place_card() {
        let mut m = maps("tapclose");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.tap(64.0, -21.0, None);
        assert_eq!(m.panel, Panel::None);
        assert!(m.selected.is_none());
    }

    #[test]
    fn dropping_a_pin_selects_it_and_remove_pin_takes_it_away() {
        let mut m = maps("pin");
        m.drop_pin_at(64.2, -21.8);
        assert_eq!(m.pins.items().len(), 1);
        assert_eq!(m.selected_pin, Some(0));
        assert_eq!(m.panel, Panel::Place);
        m.act(Act::RemovePin, 0);
        assert!(m.pins.items().is_empty());
        assert_eq!(m.panel, Panel::None);
    }

    #[test]
    fn tapping_a_pin_opens_its_card() {
        let mut m = maps("tappin");
        m.pins.add("Camp", 64.5, -21.5).unwrap();
        m.tap(64.5, -21.5, Some(0));
        assert_eq!(m.selected.as_ref().unwrap().name, "Camp");
        assert!(m.can(Act::RemovePin));
    }

    #[test]
    fn pins_and_the_view_survive_a_restart() {
        let root = dir("persist");
        {
            let mut m = Maps::new(&root, "1.0.0", 800.0, 600.0);
            m.drop_pin_at(64.2, -21.8);
            m.act(Act::MapType(MapType::Satellite), 0);
            m.act(Act::Units(UnitsSetting::Miles), 0);
            m.view.centre_to(65.0, -18.0);
            m.act(Act::ToggleFavorite, 0);
            m.persist();
        }
        let m = Maps::new(&root, "1.0.0", 800.0, 600.0);
        assert_eq!(m.pins.items().len(), 1);
        assert_eq!(m.map_type, MapType::Satellite);
        assert_eq!(m.units_setting, UnitsSetting::Miles);
        assert!((m.view.centre().0 - 65.0).abs() < 1e-6);
        assert_eq!(m.library.favorites.len(), 1);
    }

    #[test]
    fn changing_the_map_type_reloads_the_tiles_once() {
        let mut m = maps("type");
        assert_eq!(m.act(Act::MapType(MapType::Satellite), 0), vec![Effect::ReloadTiles]);
        assert!(m.act(Act::MapType(MapType::Satellite), 0).is_empty());
        assert!(m.checked(Act::MapType(MapType::Satellite)));
    }

    #[test]
    fn units_follow_the_setting_over_the_region() {
        let mut m = maps("units");
        assert_eq!(m.distance_text(1500.0), format_distance(1500.0, DistanceUnits::Metric));
        m.act(Act::Units(UnitsSetting::Miles), 0);
        assert_eq!(m.distance_text(1500.0), format_distance(1500.0, DistanceUnits::Imperial));
    }

    #[test]
    fn directions_without_a_server_show_a_labelled_estimate() {
        let mut m = maps("estimate");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        let fx = m.act(Act::Directions, 0);
        assert!(fx.is_empty());
        assert_eq!(m.panel, Panel::Directions);
        let r = m.current_route().unwrap();
        assert!(r.approximate);
        assert!(m.note.as_deref().unwrap().contains("estimate"));
    }

    #[test]
    fn directions_with_a_server_ask_it_and_keep_the_alternatives() {
        let root = dir("server");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("settings.json"), r#"{"routing_url":"https://router.example.org"}"#).unwrap();
        let mut m = Maps::new(&root, "1.0.0", 800.0, 600.0);
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        let fx = m.act(Act::Directions, 0);
        match &fx[..] {
            [Effect::Route { url }] => assert!(url.starts_with("https://router.example.org/route/v1/driving/") && url.contains("alternatives=true")),
            other => panic!("{other:?}"),
        }
        assert!(m.routing);
        let body = r#"{"code":"Ok","routes":[
            {"distance":3000,"duration":420,"geometry":{"coordinates":[[-21.9,64.1],[-21.92,64.13]]},"legs":[{"steps":[]}]},
            {"distance":3500,"duration":480,"geometry":{"coordinates":[[-21.9,64.1],[-21.93,64.13]]},"legs":[{"steps":[]}]}]}"#;
        m.route_done(route::parse_routes(body));
        assert_eq!(m.routes.len(), 2);
        m.act(Act::PickRoute(1), 0);
        assert_eq!(m.current_route().unwrap().duration_s, 480.0);
        assert!(!m.act(Act::Profile(Profile::Walking), 0).is_empty(), "a new mode asks again");
    }

    #[test]
    fn a_failed_route_falls_back_to_the_estimate_and_says_so() {
        let mut m = maps("fallback");
        m.route_from = Some(("A".into(), (64.1, -21.9)));
        m.route_to = Some(("B".into(), (64.2, -21.8)));
        m.route_done(Err("timeout".into()));
        assert!(m.current_route().unwrap().approximate);
        assert!(m.note.as_deref().unwrap().contains("timeout"));
    }

    #[test]
    fn swapping_the_ends_reverses_the_route() {
        let mut m = maps("swap");
        m.route_from = Some(("A".into(), (64.1, -21.9)));
        m.route_to = Some(("B".into(), (64.2, -21.8)));
        m.act(Act::SwapEnds, 0);
        assert_eq!(m.route_from.as_ref().unwrap().0, "B");
    }

    #[test]
    fn home_and_work_are_set_from_the_card_and_visited() {
        let mut m = maps("home");
        assert!(!m.can(Act::GoHome));
        m.choose(Place::new("Laugavegur 1, Reykjavik", 64.1466, -21.9426));
        m.act(Act::SetHome, 0);
        m.tap(0.0, 0.0, None);
        assert!(m.can(Act::GoHome));
        m.act(Act::GoHome, 0);
        assert_eq!(m.selected.as_ref().unwrap().title(), "Laugavegur 1");
    }

    #[test]
    fn collections_take_the_place_on_the_card() {
        let mut m = maps("coll");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::NewCollection, 0);
        assert_eq!(m.library.collections[0].name, "Collection 1");
        assert_eq!(m.library.collections[0].places.len(), 1);
        m.act(Act::NewCollection, 0);
        assert_eq!(m.library.collections[1].name, "Collection 2");
        m.choose(Place::new("Harpa", 64.15, -21.93));
        m.act(Act::AddToCollection(1), 0);
        assert_eq!(m.library.collections[1].places.len(), 2);
        m.show_collection(1);
        assert_eq!(m.shown_collection, Some(1));
    }

    #[test]
    fn copy_and_share_make_a_maps_link() {
        let mut m = maps("link");
        m.choose(Place::new("Perlan, Reykjavik", 64.1295, -21.9186));
        match &m.act(Act::CopyLink, 0)[..] {
            [Effect::Copy(l)] => assert!(l.starts_with("maps:?ll=64.12950,-21.91860&q=Perlan"), "{l}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&m.act(Act::Share, 0)[..], [Effect::Share(_)]));
        assert_eq!(m.act(Act::CopyCoordinates, 0), vec![Effect::Copy("64.12950, -21.91860".into())]);
    }

    #[test]
    fn a_directions_link_carries_both_ends_and_the_mode() {
        let mut m = maps("dirlink");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Directions, 0);
        m.act(Act::Profile(Profile::Walking), 0);
        assert!(m.link().contains("dirflg=w"));
    }

    #[test]
    fn opening_links_shows_searches_and_routes() {
        let mut m = maps("open");
        assert!(m.open_link("geo:64.1,-21.9?z=12", 0).is_empty());
        assert_eq!(m.view.zoom(), 12.0);
        let fx = m.open_link("maps:?q=Perlan", 0);
        assert!(matches!(&fx[..], [Effect::Search { query, .. }] if query == "Perlan"));
        m.open_link("maps:?saddr=64.1,-21.9&daddr=64.2,-21.8&dirflg=b", 0);
        assert_eq!(m.panel, Panel::Directions);
        assert_eq!(m.profile, Profile::Cycling);
        assert!(m.open_link("https://example.org", 0).is_empty());
    }

    #[test]
    fn measure_mode_collects_taps_and_reports_the_length() {
        let mut m = maps("measure");
        m.act(Act::Measure, 0);
        assert_eq!(m.panel, Panel::Measure);
        m.tap(64.0, -21.0, None);
        m.tap(64.0, -20.0, None);
        assert_eq!(m.measure.points().len(), 2);
        assert!(m.measure_line().contains("km"));
        m.act(Act::MeasureUndo, 0);
        assert_eq!(m.measure.points().len(), 1);
        m.act(Act::MeasureClear, 0);
        assert!(!m.can(Act::MeasureUndo));
        m.act(Act::Measure, 0);
        assert_eq!(m.panel, Panel::None);
    }

    #[test]
    fn download_map_counts_tiles_and_starts_within_the_limit() {
        let mut m = maps("download");
        m.view.set_zoom(13.0);
        m.act(Act::Download, 0);
        assert_eq!(m.panel, Panel::Download);
        let line = m.download_line();
        assert!(line.contains("tiles"), "{line}");
        let fx = m.act(Act::StartDownload, 0);
        assert!(matches!(&fx[..], [Effect::StartDownload(t)] if !t.is_empty()));
        assert!(m.progress.is_some());
        assert!(!m.can(Act::StartDownload), "one download at a time");
        assert_eq!(m.act(Act::CancelDownload, 0), vec![Effect::CancelDownload]);
    }

    #[test]
    fn a_world_view_is_too_big_to_download() {
        let mut m = maps("bigdl");
        m.resize(1600.0, 1200.0);
        m.act(Act::Download, 0);
        assert!(m.download_line().contains("too large"));
        assert!(!m.can(Act::StartDownload));
    }

    #[test]
    fn a_gpx_file_adds_pins_and_a_track() {
        let mut m = maps("gpx");
        let text = r#"<gpx><wpt lat="64.1" lon="-21.9"><name>Start</name></wpt>
            <trk><name>Walk</name><trkseg><trkpt lat="64.1" lon="-21.9"/><trkpt lat="64.2" lon="-21.8"/></trkseg></trk></gpx>"#;
        let summary = m.gpx_loaded(text).unwrap();
        assert!(summary.contains("1 waypoint") && summary.contains("1 track"), "{summary}");
        assert_eq!(m.pins.items().len(), 1);
        assert_eq!(m.tracks.len(), 1);
        assert!(m.gpx_loaded("<gpx></gpx>").is_err());
        assert!(m.gpx_loaded("not xml at all").is_err());
    }

    #[test]
    fn print_text_describes_the_route_with_its_steps() {
        let mut m = maps("print");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        assert!(matches!(&m.act(Act::Print, 0)[..], [Effect::Print { .. }]));
        m.act(Act::Directions, 0);
        let (title, text) = m.print_text();
        assert_eq!(title, "Directions to Perlan");
        assert!(text.contains("Directions from") && text.contains("straight-line"));
    }

    #[test]
    fn print_needs_something_to_print() {
        let m = maps("noprint");
        assert!(!m.can(Act::Print));
    }

    #[test]
    fn escape_closes_the_card_then_clears_the_place() {
        let mut m = maps("esc");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Escape, 0);
        assert_eq!(m.panel, Panel::None);
        assert!(m.selected.is_none());
    }

    #[test]
    fn closing_directions_goes_back_to_the_place() {
        let mut m = maps("closedir");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Directions, 0);
        m.act(Act::ClosePanel, 0);
        assert_eq!(m.panel, Panel::Place);
        assert!(m.routes.is_empty());
    }

    #[test]
    fn the_region_decides_automatic_units() {
        let mut m = maps("region");
        m.region_metric = false;
        assert_eq!(m.units(), DistanceUnits::Imperial);
        m.act(Act::Units(UnitsSetting::Kilometres), 0);
        assert_eq!(m.units(), DistanceUnits::Metric);
    }

    fn with_settings(name: &str, settings: &str) -> Maps {
        let root = dir(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("settings.json"), settings).unwrap();
        let mut m = Maps::new(&root, "1.0.0", 800.0, 600.0);
        m.region_metric = true;
        m
    }

    fn fix(p: (f64, f64)) -> Here {
        Here { lat: p.0, lon: p.1, accuracy_m: 10.0 }
    }

    #[test]
    fn locate_waits_for_a_position_then_centres_on_it() {
        let mut m = maps("locate");
        assert!(m.can(Act::Locate));
        assert_eq!(m.act(Act::Locate, 0), vec![Effect::Locate]);
        assert!(m.locating);
        assert!(m.position_update(Here { lat: 65.68, lon: -18.1, accuracy_m: 100.0 }, 10).is_empty());
        assert!(!m.locating);
        let (lat, lon) = m.view.centre();
        assert!((lat - 65.68).abs() < 1e-6 && (lon + 18.1).abs() < 1e-6);
        assert!((m.view.zoom() - PLACE_ZOOM).abs() < 1e-9);
        assert_eq!(m.here, Some(Here { lat: 65.68, lon: -18.1, accuracy_m: 100.0 }));
        assert!(m.note.is_none());
    }

    #[test]
    fn a_city_level_fix_is_shown_wider_and_called_approximate() {
        let mut m = maps("approx");
        m.act(Act::Locate, 0);
        m.position_update(Here { lat: 64.15, lon: -21.94, accuracy_m: 25_000.0 }, 0);
        assert!((m.view.zoom() - 11.0).abs() < 1e-9);
        assert!(m.note.as_deref().unwrap().starts_with("Approximate location"));
        assert!(zoom_for_accuracy(50.0) > zoom_for_accuracy(500.0) && zoom_for_accuracy(500.0) > zoom_for_accuracy(5_000.0));
    }

    #[test]
    fn a_position_nobody_asked_for_does_not_move_the_map() {
        let mut m = maps("quiet");
        let before = m.view.centre();
        m.position_update(fix((65.0, -18.0)), 0);
        assert_eq!(m.view.centre(), before);
        assert!(m.here.is_some());
    }

    #[test]
    fn a_refused_locate_stops_waiting_and_says_why() {
        let mut m = maps("refused");
        m.act(Act::Locate, 0);
        m.locate_failed("Location is off for Maps");
        assert!(!m.locating);
        assert_eq!(m.note.as_deref(), Some("Location is off for Maps"));
    }

    #[test]
    fn directions_start_from_the_current_location_when_it_is_known() {
        let mut m = maps("here-origin");
        m.library.set_home(Some(Saved::new("Home", 64.2, -21.8)));
        m.position_update(fix((64.14, -21.9)), 0);
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Directions, 0);
        let (name, at) = m.route_from.clone().unwrap();
        assert_eq!(name, "Current location");
        assert_eq!(at, (64.14, -21.9));
    }

    #[test]
    fn traffic_needs_a_layer_url_and_the_choice_is_kept() {
        let mut m = maps("traffic-none");
        assert!(m.traffic_source().is_none());
        m.act(Act::ToggleTraffic, 0);
        assert!(!m.show_traffic);
        assert!(m.note.as_deref().unwrap().contains("traffic_url"));

        let mut m = with_settings("traffic", r#"{"traffic_url":"https://traffic.example.org/{z}/{x}/{y}.png"}"#);
        assert!(m.traffic_source().is_some());
        assert!(!m.checked(Act::ToggleTraffic));
        m.act(Act::ToggleTraffic, 0);
        assert!(m.checked(Act::ToggleTraffic));
        m.persist();
        assert!(std::fs::read_to_string(m.root().join("settings.json")).unwrap().contains("traffic_url"));
        let again = Maps::new(m.root(), "1.0.0", 800.0, 600.0);
        assert!(again.show_traffic, "the choice is remembered");
        m.act(Act::ToggleTraffic, 0);
        assert!(!m.show_traffic);

        let bad = with_settings("traffic-bad", r#"{"traffic_url":"https://x/{z}/{x}.png","traffic":true}"#);
        assert!(bad.traffic_source().is_none() && !bad.show_traffic, "a template without a y is ignored");
    }

    #[test]
    fn transit_directions_ask_the_planner_and_never_fall_back_to_an_estimate() {
        let mut m = with_settings("transit", r#"{"transit_url":"https://otp.example.org/otp/routers/default"}"#);
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Directions, 0);
        let fx = m.act(Act::Profile(Profile::Transit), 0);
        match &fx[..] {
            [Effect::Plan { url }] => assert!(url.starts_with("https://otp.example.org/otp/routers/default/plan?fromPlace="), "{url}"),
            other => panic!("{other:?}"),
        }
        assert!(m.routing);
        let body = r#"{"plan":{"itineraries":[{"duration":900,"legs":[{"mode":"BUS","routeShortName":"1","distance":3000,
            "from":{"name":"A","lat":64.1,"lon":-21.9},"to":{"name":"B","lat":64.13,"lon":-21.92}}]}]}}"#;
        m.route_done(crate::transit::parse_itineraries(body));
        assert_eq!(m.routes.len(), 1);
        assert!(!m.routes[0].approximate);
        assert!(!m.can(Act::StartNavigation), "a timetable journey is not followed turn by turn");
        m.route_done(Err("timeout".into()));
        assert!(m.routes.is_empty(), "no straight-line stand-in for a bus");
        assert!(m.note.as_deref().unwrap().contains("No journey"));
    }

    #[test]
    fn transit_without_a_planner_says_so() {
        let mut m = maps("transit-none");
        m.choose(Place::new("Perlan", 64.1295, -21.9186));
        m.act(Act::Directions, 0);
        assert!(m.act(Act::Profile(Profile::Transit), 0).is_empty());
        assert!(m.routes.is_empty());
        assert!(m.note.as_deref().unwrap().contains("transit_url"));
        assert!(!m.routing);
    }

    const MPD: f64 = 111_194.9;
    const A: (f64, f64) = (64.0, -21.0);

    fn north_of(p: (f64, f64), metres: f64) -> (f64, f64) {
        (p.0 + metres / MPD, p.1)
    }

    fn east_of(p: (f64, f64), metres: f64) -> (f64, f64) {
        (p.0, p.1 + metres / (MPD * p.0.to_radians().cos()))
    }

    fn step(name: &str, maneuver: &str, distance_m: f64) -> route::Step {
        route::Step { name: name.into(), maneuver: maneuver.into(), distance_m }
    }

    /// 1000 m north, a right turn, 1000 m east.
    fn guide_route() -> Route {
        let b = north_of(A, 1000.0);
        let c = east_of(b, 1000.0);
        Route {
            distance_m: 2000.0,
            duration_s: 200.0,
            steps: vec![step("Main Street", "depart", 1000.0), step("Side Road", "turn right", 1000.0), step("", "arrive", 0.0)],
            approximate: false,
            geometry: vec![A, b, c],
        }
    }

    /// A map with a routing server whose directions card shows `guide_route`.
    fn with_route(name: &str) -> Maps {
        let mut m = with_settings(name, r#"{"routing_url":"https://router.example.org"}"#);
        let route = guide_route();
        let end = *route.geometry.last().unwrap();
        m.route_from = Some(("A".into(), A));
        m.route_to = Some(("B".into(), end));
        m.panel = Panel::Directions;
        m.route_done(Ok(vec![route]));
        m
    }

    #[test]
    fn only_a_computed_route_can_be_started() {
        let mut estimate = maps("nav-estimate");
        estimate.choose(Place::new("Perlan", 64.1295, -21.9186));
        estimate.act(Act::Directions, 0);
        assert!(estimate.current_route().unwrap().approximate);
        assert!(!estimate.can(Act::StartNavigation));
        assert!(!estimate.can(Act::EndNavigation));
        assert!(with_route("nav-can").can(Act::StartNavigation));
    }

    #[test]
    fn starting_waits_for_a_position_and_then_shows_the_first_manoeuvre() {
        let mut m = with_route("nav-start");
        assert_eq!(m.act(Act::StartNavigation, 0), vec![Effect::TrackLocation(true)]);
        assert_eq!(m.panel, Panel::Navigation);
        assert_eq!(m.nav_title(), "Starting navigation");
        assert_eq!(m.nav_subtitle(), "Waiting for your location");
        assert!(m.nav_remaining().is_none());
        assert!(m.position_update(fix(north_of(A, 100.0)), 1_000).is_empty());
        assert_eq!(m.nav_title(), "Turn right onto Side Road");
        assert!(m.nav_subtitle().starts_with("in "), "{}", m.nav_subtitle());
        assert!(m.nav_remaining().unwrap().ends_with("remaining"));
        let (lat, _) = m.view.centre();
        assert!((lat - north_of(A, 100.0).0).abs() < 1e-6, "the map follows the person");
    }

    #[test]
    fn the_next_manoeuvre_is_announced_as_it_comes_near() {
        let mut m = with_route("nav-announce");
        m.act(Act::StartNavigation, 0);
        m.position_update(fix(north_of(A, 300.0)), 1_000);
        assert!(m.announcement.is_none(), "700 m is too far");
        m.position_update(fix(north_of(A, 520.0)), 2_000);
        let far = m.announcement.clone().unwrap();
        assert!(far.starts_with("In ") && far.ends_with("turn right onto Side Road"), "{far}");
        m.position_update(fix(north_of(A, 990.0)), 3_000);
        assert_eq!(m.announcement.as_deref(), Some("Turn right onto Side Road"));
    }

    #[test]
    fn leaving_the_route_asks_for_a_new_one_once_in_a_while() {
        let mut m = with_route("nav-reroute");
        m.act(Act::StartNavigation, 0);
        m.position_update(fix(north_of(A, 300.0)), 1_000);
        let stray = fix(east_of(north_of(A, 500.0), -300.0));
        let fx = m.position_update(stray, 20_000);
        match &fx[..] {
            [Effect::Route { url }] => assert!(url.starts_with("https://router.example.org/route/v1/driving/"), "{url}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(m.nav_title(), "Off route");
        assert_eq!(m.route_from.as_ref().unwrap().0, "Current location");
        assert!(m.routing);
        assert!(m.position_update(stray, 25_000).is_empty(), "one request at a time");
        // the reply: a route from where the person is, followed from the next position on
        let end = north_of(stray.lat_lon(), 400.0);
        let fresh = Route {
            geometry: vec![stray.lat_lon(), end],
            steps: vec![step("Main Street", "depart", 400.0), step("", "arrive", 0.0)],
            distance_m: 400.0,
            duration_s: 40.0,
            approximate: false,
        };
        m.route_done(Ok(vec![fresh]));
        assert!(!m.routing);
        assert_eq!(m.announcement.as_deref(), Some("Following a new route"));
        assert!(m.guidance.is_none());
        m.position_update(stray, 30_000);
        assert!(!m.guidance.as_ref().unwrap().off_route);
    }

    #[test]
    fn a_new_route_that_cannot_be_found_keeps_the_old_one() {
        let mut m = with_route("nav-noroute");
        m.act(Act::StartNavigation, 0);
        let stray = fix(east_of(north_of(A, 500.0), -300.0));
        m.position_update(stray, 20_000);
        m.route_done(Err("timeout".into()));
        assert!(m.nav.is_some());
        assert_eq!(m.note.as_deref(), Some("No new route was found"));
        assert!(!m.position_update(stray, 40_000).is_empty(), "it tries again after the pause");
    }

    #[test]
    fn arriving_says_so() {
        let mut m = with_route("nav-arrive");
        m.act(Act::StartNavigation, 0);
        let end = *guide_route().geometry.last().unwrap();
        m.position_update(fix(east_of(end, -10.0)), 1_000);
        assert_eq!(m.nav_title(), "You have arrived");
        assert_eq!(m.nav_subtitle(), "B");
        assert_eq!(m.announcement.as_deref(), Some("Arrive at your destination"));
        assert!(m.nav_remaining().is_none());
    }

    #[test]
    fn ending_or_closing_the_guidance_goes_back_to_the_directions() {
        let mut m = with_route("nav-end");
        m.act(Act::StartNavigation, 0);
        assert!(m.can(Act::EndNavigation));
        assert_eq!(m.act(Act::EndNavigation, 0), vec![Effect::TrackLocation(false)]);
        assert_eq!(m.panel, Panel::Directions);
        assert!(m.nav.is_none() && m.guidance.is_none());
        assert!(!m.can(Act::EndNavigation));
        m.act(Act::StartNavigation, 0);
        assert_eq!(m.act(Act::ClosePanel, 0), vec![Effect::TrackLocation(false)]);
        assert_eq!(m.panel, Panel::Directions, "the card closes back to the directions");
    }

    #[test]
    fn asking_for_other_directions_while_following_stops_the_guidance() {
        let mut m = with_route("nav-profile");
        m.act(Act::StartNavigation, 0);
        let fx = m.act(Act::Profile(Profile::Walking), 0);
        assert_eq!(fx.first(), Some(&Effect::TrackLocation(false)));
        assert!(matches!(fx.get(1), Some(Effect::Route { .. })));
        assert!(m.nav.is_none());
    }
}
