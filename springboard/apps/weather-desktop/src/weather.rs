//! The Weather app: current conditions with a big temperature and a drawn
//! condition glyph, an hourly strip, seven days, a list of places with
//! search and a C/F switch, and a Stocks-lite watchlist page.
//!
//! Open-Meteo answers are fetched on the global dispatch queue, cached on disk
//! (see `data`) and the UI thread only ever reads memory: with no network
//! the last forecast shows. The first place's summary is written to
//! `summary.txt` for the Today widget and the lock screen hook.

use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect as KRect, Role, TextField as SearchQuery};
use bifrost_app::widgets;
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS};
use bifrost_app::kit::ui::{DISPLAY_LARGE_PT};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE3_PT};
use springboard::layout::in_rect as within;
use bifrost_app::kit::ui::{home_zone, CARD_BG, CONTENT_BG, DESTRUCTIVE, LABEL, LABEL_2, LINK, SEPARATOR_C, WHITE};
use bifrost_app::kit::{pack, Event, Image, Surface, TextCache, Ui, Weight};
use springboard::deildu::scroll::Scroller;
use springboard::shell::keyboard::Out;
use springboard::shell::music::radio::Fetch;
use springboard::shell::textfield::TextField;
use springboard::shell::weather::data::{self, Forecast, Location, Quote, Sky, Store, Summary, Units};
use springboard::shell::weather::glyph;
use bifrost_app::controls::geo::{format_distance, Coordinate, DistanceUnits};
use bifrost_app::controls::chart_marks::Frame;
use bifrost_app::controls::location_auth::AuthorizationStatus;
use crate::insights;
use bifrost_app::controls::weather_model::{illuminated_fraction, MoonPhase, Wind, WeatherError};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bifrost_app::kit::ui::SYSTEM_GREEN as GREEN;
use bifrost_app::kit::ui::SYSTEM_ORANGE as ORANGE;
/// The content starts under the unified toolbar band (full-size window).
fn nav_bottom_pt() -> f32 { UNIFIED_BAR_PT }
/// Width of the Locations sidebar, and the window width below which it hides (points).
const SIDEBAR_PT: f32 = 260.0;
const COMPACT_PT: f32 = 640.0;
/// Sidebar rows (points) and their ids (`PLACE_ID + index`).
const SIDEBAR_ROW_PT: f32 = 32.0;
const PLACE_ID: u32 = 100;
/// Accessibility ids of the detail pane's controls (the toolbar uses 1 to 4, the sidebar search 10).
const SIDE_SEARCH: u64 = 10;
const UNITS_CELSIUS: u64 = 20;
const UNITS_FAHRENHEIT: u64 = 21;
const FIELD_ID: u64 = 30;
const PLACE_ROW_ID: u64 = 1000;
const REMOVE_ID: u64 = 2000;
const RESULT_ID: u64 = 3000;
const QUOTE_REMOVE_ID: u64 = 4000;
/// Toolbar item ids (also the semantic control ids).
const BAR_LOCATIONS: u32 = 1;
const BAR_STOCKS: u32 = 2;
const BAR_DONE: u32 = 3;
const BAR_BACK: u32 = 4;

/// The translucent scrim under the forecast cards: black at 25% over the sky artwork (a fixed colour, not chrome).
const SKY_SCRIM: u32 = 64 << 24;

/// A premultiplied colour at `alpha` of its opacity (white text and plates over the sky).
fn fade(c: u32, alpha: u8) -> u32 {
    let m = |sh: u32| (((c >> sh) & 0xff) * alpha as u32 / 255) << sh;
    m(0) | m(8) | m(16) | m(24)
}
const MARGIN: f32 = 20.0;
fn places_field_top_pt() -> f32 { nav_bottom_pt() + 54.0 }
fn places_list_top_pt() -> f32 { nav_bottom_pt() + 100.0 }
fn stocks_field_top_pt() -> f32 { nav_bottom_pt() + 10.0 }
fn stocks_list_top_pt() -> f32 { nav_bottom_pt() + 58.0 }
const FIELD_H: f32 = 36.0;
const ROW: f32 = 56.0;
const HEAD: f32 = 28.0;
const QUOTE_ROW: f32 = 60.0;
const REMOVE_W: f32 = 48.0;
const HOURLY_TOP: f32 = 296.0;
const HOURLY_H: f32 = 116.0;
const HOUR_W: f32 = 60.0;
const DAILY_TOP: f32 = 424.0;
const DAY_ROW: f32 = 44.0;
const DAILY_H: f32 = 28.0 + 10.0 * DAY_ROW + 8.0;
const DETAILS_TOP: f32 = DAILY_TOP + DAILY_H + 16.0;
const TILE_H: f32 = 104.0;
const TILE_GAP: f32 = 12.0;
const DETAILS_H: f32 = 3.0 * TILE_H + 2.0 * TILE_GAP;
/// The temperature chart card, below the detail tiles: its plot area and the axis labels' room, in points.
const CHART_TOP: f32 = DETAILS_TOP + DETAILS_H + 16.0;
const CHART_H: f32 = 176.0;
const CHART_AXIS: f32 = 40.0;
const CHART_PLOT_TOP: f32 = 36.0;
const CHART_PLOT_H: f32 = 116.0;
/// The map card, below the chart: the map area and the note under it, in points.
const MAP_TOP: f32 = CHART_TOP + CHART_H + 16.0;
const MAP_H: f32 = 200.0;
const MAP_AREA_TOP: f32 = 36.0;
const MAP_AREA_H: f32 = 124.0;
const MAP_NOTE_TOP: f32 = 176.0;
const CONTENT_H: f32 = MAP_TOP + MAP_H + 32.0;
/// Location access for Weather. The shell gives apps no location service yet, so the status stays "not asked".
const LOCATION: AuthorizationStatus = AuthorizationStatus::NotDetermined;
/// Grouped cards and fields: the system card radius (`prefs_layout`).
const RADIUS: f32 = m::CARD_RADIUS;
/// a shown forecast is fetched again after this long (ms), a missing one after `RETRY_MS`
const REFRESH_MS: f64 = 15.0 * 60_000.0;
const RETRY_MS: f64 = 30_000.0;
const QUOTES_MS: f64 = 60_000.0;
const MIN_REFRESH_MS: f64 = 5_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Main,
    Places,
    Stocks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Places,
    Stocks,
    Back,
    Units(Units),
    Field,
    Place(usize),
    Remove(usize),
    Result(usize),
    RemoveQuote(usize),
    Hourly,
}

enum Done {
    Forecast { key: String, result: Result<Forecast, WeatherError> },
    Places { query: String, result: Result<Vec<Location>, String> },
    Quotes(Result<Vec<Quote>, String>),
}

enum Row {
    Head(String),
    Place(usize),
    Result(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideTarget {
    Search,
    Place(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

type Rect = (f32, f32, f32, f32);

/// The (top, bottom) colours of the sky behind the main page.
fn backdrop(sky: Sky, day: bool) -> ((u8, u8, u8), (u8, u8, u8)) {
    match (sky, day) {
        (Sky::Clear | Sky::PartlyCloudy, true) => ((38, 108, 214), (110, 186, 248)),
        (Sky::Clear | Sky::PartlyCloudy, false) => ((8, 14, 38), (28, 40, 82)),
        (Sky::Cloudy | Sky::Fog, true) => ((88, 104, 124), (150, 164, 180)),
        (Sky::Cloudy | Sky::Fog, false) => ((26, 32, 44), (52, 60, 76)),
        (Sky::Drizzle | Sky::Rain, _) => ((52, 66, 90), (92, 110, 134)),
        (Sky::Snow, _) => ((100, 118, 142), (176, 190, 206)),
        (Sky::Thunder, _) => ((34, 32, 58), (70, 66, 96)),
    }
}

pub struct Weather {
    /// width of the detail pane (the window less the sidebar), px
    w: i32,
    /// the whole window's width and the sidebar's, px
    full_w: i32,
    side: f32,
    /// one point in pixels at the last `relayout`
    pt_px: f32,
    /// the sidebar's search capsule (it opens the Locations search)
    side_query: SearchQuery,
    /// a sidebar target under the finger
    side_pressed: Option<SideTarget>,
    h: i32,
    store: Store,
    fetcher: Arc<dyn Fetch>,
    /// Finished fetches, filled on the global queue and applied by `pump` on the app loop.
    done: Arc<Mutex<Vec<Done>>>,
    places: Vec<Location>,
    selected: usize,
    units: Units,
    forecasts: HashMap<String, Forecast>,
    /// when each place was last asked for (ms), and the asks in flight
    tried: HashMap<String, f64>,
    pending: HashSet<String>,
    error: Option<WeatherError>,
    /// whether the network path lets the app reach the forecast service (true until the shell says otherwise)
    online: bool,
    /// set when the saved summary changed (a forecast for the selected place, a new selection or units); the app
    /// reads it with `take_summary_changed` and reloads the widget timeline
    summary_changed: bool,
    results: Vec<Location>,
    searching: bool,
    note: Option<String>,
    watch: Vec<String>,
    quotes: Vec<Quote>,
    quotes_at: f64,
    /// when `maintain` next has anything to do (ms); reset by input and finished jobs
    maintain_at: f64,
    /// the sky backdrop, drawn once per (sky, day, size)
    backdrop_img: Option<((Sky, bool, usize, usize), Image)>,
    quotes_pending: bool,
    quotes_error: Option<String>,
    screen: Screen,
    field: TextField,
    kb_on: bool,
    scroll: Scroller,
    hscroll: Scroller,
    pressed: Option<Target>,
    /// (x, y, place) where a finger went down on a place row; a left swipe deletes it
    swipe: Option<(f32, f32, usize)>,
    /// Some(horizontal) while a finger drags a scroller
    scrolling: Option<bool>,
    labels: TextCache,
    dirty: bool,
}

impl Weather {
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, fetcher: Arc<dyn Fetch>) -> Self {
        let store = Store::new(dir);
        let places = store.load_locations();
        let (units, selected) = store.load_settings();
        let forecasts = places.iter().filter_map(|p| Some((p.key(), store.load_forecast(p)?.0))).collect();
        let mut field = TextField::new(ui, "Search city", MAC_BODY_PT, false, 64);
        field.focused = false;
        Weather {
            w,
            full_w: w,
            side: 0.0,
            pt_px: ui.pt(1.0),
            side_query: SearchQuery::new(),
            side_pressed: None,
            h,
            watch: store.load_watchlist(),
            selected: selected.min(places.len() - 1),
            store,
            fetcher,
            done: Arc::new(Mutex::new(Vec::new())),
            places,
            units,
            forecasts,
            tried: HashMap::new(),
            pending: HashSet::new(),
            error: None,
            online: true,
            summary_changed: false,
            results: vec![],
            searching: false,
            note: None,
            quotes: vec![],
            quotes_at: f64::NEG_INFINITY,
            maintain_at: f64::NEG_INFINITY,
            backdrop_img: None,
            quotes_pending: false,
            quotes_error: None,
            screen: Screen::Main,
            field,
            kb_on: false,
            scroll: Scroller::new(),
            hscroll: Scroller::new(),
            pressed: None,
            swipe: None,
            scrolling: None,
            labels: TextCache::default(),
            dirty: true,
        }
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.full_w = w;
        self.w = (w as f32 - self.side) as i32;
        self.h = h;
        self.dirty = true;
    }

    /// Splits the window into the Locations sidebar and the detail pane (no sidebar in a narrow window).
    fn relayout(&mut self, ui: &Ui) {
        self.pt_px = ui.pt(1.0);
        let side = if (self.full_w as f32) < ui.pt(COMPACT_PT) { 0.0 } else { ui.pt(SIDEBAR_PT).round() };
        if side != self.side || self.w != self.full_w - side as i32 {
            self.side = side;
            self.w = self.full_w - side as i32;
            self.dirty = true;
        }
    }

    // ---- Golden Gate chrome: sidebar and toolbar ----

    fn sidebar(&self) -> SourceList {
        let items = self.places.iter().enumerate().map(|(i, p)| SidebarItem::row(PLACE_ID + i as u32, &p.name)).collect();
        let mut list = SourceList::new(items, SIDEBAR_ROW_PT, SIDEBAR_ROW_PT);
        list.set_selected((self.screen != Screen::Stocks).then_some(PLACE_ID + self.selected as u32));
        list
    }

    /// The search capsule and the list, in points.
    fn side_search(&self) -> KRect {
        KRect { x: SPACE_M, y: UNIFIED_BAR_PT, w: self.side / self.pt() - 2.0 * SPACE_M, h: FIELD_H - 4.0 }
    }

    fn side_list(&self) -> KRect {
        let top = UNIFIED_BAR_PT + FIELD_H + SPACE_S;
        KRect { x: 0.0, y: top, w: self.side / self.pt(), h: (self.h as f32 / self.pt() - top).max(0.0) }
    }

    /// Points per pixel inverse: one point in pixels (kept from the last `relayout`).
    fn pt(&self) -> f32 {
        self.pt_px
    }

    fn side_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<SideTarget> {
        let p = ui.pt(1.0);
        let (x, y) = (x / p, y / p);
        if self.side_search().contains(x, y) {
            return Some(SideTarget::Search);
        }
        self.sidebar().hit(self.side_list(), 0.0, x, y).map(|id| SideTarget::Place((id - PLACE_ID) as usize))
    }

    /// The detail pane's toolbar: Locations (only without the sidebar) and Stocks on the main page, Done on
    /// Locations, Back on Stocks.
    fn bar(&self) -> Toolbar {
        let mut items = vec![];
        match self.screen {
            Screen::Main => {
                if self.side == 0.0 {
                    items.push(ToolbarItem::icon(BAR_LOCATIONS, "Locations", "list.bullet"));
                }
                items.push(ToolbarItem::flexible_space());
                items.push(ToolbarItem::button(BAR_STOCKS, "Stocks"));
            }
            Screen::Places => {
                items.push(ToolbarItem::flexible_space());
                items.push(ToolbarItem::button(BAR_DONE, "Done"));
            }
            Screen::Stocks => {
                items.push(ToolbarItem::icon(BAR_BACK, "Weather", "chevron.left"));
                items.push(ToolbarItem::flexible_space());
            }
        }
        Toolbar::new(items)
    }

    /// The toolbar's rectangle in detail-pane points; it starts after the traffic lights when there is no sidebar.
    fn bar_bounds(&self, ui: &Ui) -> KRect {
        let x = if self.side == 0.0 { traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT } else { SPACE_M };
        KRect { x, y: 0.0, w: (self.w as f32 / ui.pt(1.0) - x - SPACE_M).max(0.0), h: UNIFIED_BAR_PT }
    }

    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        let h = widgets::CAPSULE_PT;
        let b = self.bar_bounds(ui);
        self.bar().layout_with(KRect { y: b.y + (b.h - h) / 2.0, h, ..b }, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Toolbar buttons and the sidebar search as semantic controls, in window points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let dx = self.side / ui.pt(1.0);
        let bar = self.bar();
        let mut nodes: Vec<Node> = self.bar_layout(ui).visible.iter().filter_map(|(id, r)| {
            let item = bar.items.iter().find(|i| i.id == *id)?;
            Some(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: KRect { x: r.x + dx, ..*r }, enabled: item.enabled })
        }).collect();
        if self.side > 0.0 {
            nodes.push(Node { id: SIDE_SEARCH, role: Role::TextField, label: "Search".into(), value: String::new(), bounds: self.side_search(), enabled: true });
            nodes.extend(self.side_rows());
        }
        nodes.extend(self.detail_controls(ui, dx));
        nodes
    }

    /// The Locations sidebar's rows, in window points.
    fn side_rows(&self) -> Vec<Node> {
        let list = self.sidebar();
        let bounds = self.side_list();
        self.places.iter().enumerate().map(|(i, p)| {
            let id = PLACE_ID + i as u32;
            Node {
                id: id as u64,
                role: Role::Row,
                label: p.name.clone(),
                value: if list.selected() == Some(id) { "selected".into() } else { String::new() },
                bounds: list.item_rect(i, bounds, 0.0),
                enabled: true,
            }
        }).collect()
    }

    /// The detail pane's controls (units, the search or symbol field, place and stock rows with their buttons), in
    /// window points; `dx` is the sidebar width. Rows scrolled out of the list are left out.
    fn detail_controls(&self, ui: &Ui, dx: f32) -> Vec<Node> {
        let p = ui.pt(1.0);
        let card_w = self.w as f32 / p - 2.0 * MARGIN;
        let x = MARGIN + dx;
        let off = self.scroll.offset() / p;
        let (lo, hi) = (self.list_top(), self.bottom(ui) / p);
        let node = |id: u64, role: Role, label: String, value: String, bounds: KRect| Node { id, role, label, value, bounds, enabled: true };
        let rect = |x: f32, y: f32, w: f32, h: f32| KRect { x, y, w, h };
        let mut nodes = vec![];
        match self.screen {
            Screen::Main => {}
            Screen::Places => {
                let half = card_w / 2.0;
                for (id, u, label) in [(UNITS_CELSIUS, Units::Celsius, "Celsius"), (UNITS_FAHRENHEIT, Units::Fahrenheit, "Fahrenheit")] {
                    let bx = x + if u == Units::Celsius { 0.0 } else { half };
                    let value = if self.units == u { "selected".to_string() } else { String::new() };
                    nodes.push(node(id, Role::Button, label.into(), value, rect(bx, nav_bottom_pt() + 10.0, half, 34.0)));
                }
                nodes.push(node(FIELD_ID, Role::TextField, "Search city".into(), self.field.text().to_string(), rect(x, places_field_top_pt(), card_w, FIELD_H)));
                for (row, rt, rh) in self.place_rows() {
                    let y = self.list_top() + rt - off;
                    if y + rh < lo || y > hi {
                        continue;
                    }
                    match row {
                        Row::Place(i) => {
                            let name = self.places[i].name.clone();
                            let value = if i == self.selected { "selected".to_string() } else { String::new() };
                            nodes.push(node(PLACE_ROW_ID + i as u64, Role::Button, name.clone(), value, rect(x, y, card_w, rh - 4.0)));
                            if self.places.len() > 1 {
                                nodes.push(node(REMOVE_ID + i as u64, Role::Button, format!("Remove {name}"), String::new(), rect(x + card_w - REMOVE_W, y, REMOVE_W, rh - 4.0)));
                            }
                        }
                        Row::Result(i) => {
                            if let Some(r) = self.results.get(i) {
                                nodes.push(node(RESULT_ID + i as u64, Role::Button, format!("Add {}", r.name), String::new(), rect(x, y, card_w, rh - 4.0)));
                            }
                        }
                        Row::Head(_) => {}
                    }
                }
            }
            Screen::Stocks => {
                nodes.push(node(FIELD_ID, Role::TextField, "Add symbol, e.g. AAPL".into(), self.field.text().to_string(), rect(x, stocks_field_top_pt(), card_w, FIELD_H)));
                for (i, sym) in self.watch.iter().enumerate() {
                    let y = self.list_top() + i as f32 * QUOTE_ROW - off;
                    if y + QUOTE_ROW < lo || y > hi {
                        continue;
                    }
                    let label = format!("Remove {}", data::display_symbol(sym));
                    nodes.push(node(QUOTE_REMOVE_ID + i as u64, Role::Button, label, String::new(), rect(x + card_w - REMOVE_W, y, REMOVE_W, QUOTE_ROW - 6.0)));
                }
            }
        }
        nodes
    }

    /// Presses accessibility control `id` (`Input::Activate`), the same action as a tap on it. True when a frame is needed.
    pub fn activate_control(&mut self, ui: &Ui, id: u64, t: f64) -> bool {
        let at = self.field_rect(ui).0 + ui.pt(SPACE_M);
        let target = match id {
            _ if id == BAR_LOCATIONS as u64 => Target::Places,
            _ if id == BAR_STOCKS as u64 => Target::Stocks,
            _ if id == BAR_DONE as u64 || id == BAR_BACK as u64 => Target::Back,
            UNITS_CELSIUS => Target::Units(Units::Celsius),
            UNITS_FAHRENHEIT => Target::Units(Units::Fahrenheit),
            FIELD_ID => Target::Field,
            SIDE_SEARCH => {
                if self.screen != Screen::Places {
                    self.activate(ui, Target::Places, 0.0, t);
                }
                self.activate(ui, Target::Field, at, t);
                return true;
            }
            _ if id >= QUOTE_REMOVE_ID => Target::RemoveQuote((id - QUOTE_REMOVE_ID) as usize),
            _ if id >= RESULT_ID => Target::Result((id - RESULT_ID) as usize),
            _ if id >= REMOVE_ID => Target::Remove((id - REMOVE_ID) as usize),
            _ if id >= PLACE_ROW_ID => Target::Place((id - PLACE_ROW_ID) as usize),
            _ if id >= PLACE_ID as u64 => Target::Place((id - PLACE_ID as u64) as usize),
            _ => return false,
        };
        if let Target::Place(i) = target {
            if i >= self.places.len() {
                return false;
            }
        }
        self.activate(ui, target, at, t);
        true
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        let p = ui.pt(1.0);
        let pressed = match self.side_pressed { Some(SideTarget::Place(i)) => Some(PLACE_ID + i as u32), _ => None };
        widgets::source_list(s, ui, KRect { x: 0.0, y: 0.0, w: self.side / p, h: self.h as f32 / p }, &SourceList::new(vec![], SIDEBAR_ROW_PT, SIDEBAR_ROW_PT), 0.0, None);
        widgets::search_field(s, ui, &self.side_query, self.side_search(), "Search", false);
        widgets::source_list(s, ui, self.side_list(), &self.sidebar(), 0.0, pressed);
        s.fill_rect((self.side - 1.0) as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
    }

    fn draw_bar(&mut self, s: &mut Surface, ui: &Ui) {
        if self.screen != Screen::Main {
            widgets::toolbar_band(s, ui, KRect { x: 0.0, y: 0.0, w: self.w as f32 / ui.pt(1.0), h: UNIFIED_BAR_PT });
            let title = if self.screen == Screen::Places { "Locations" } else { "Stocks" };
            self.put(s, ui, title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), Align::Centre, self.w as f32 / 2.0, ui.pt(UNIFIED_BAR_PT / 2.0));
        }
        let pressed = match self.pressed {
            Some(Target::Places) => Some(BAR_LOCATIONS),
            Some(Target::Stocks) => Some(BAR_STOCKS),
            Some(Target::Back) => Some(if self.screen == Screen::Places { BAR_DONE } else { BAR_BACK }),
            _ => None,
        };
        widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(), pressed);
    }

    pub fn needs_frame(&self) -> bool {
        self.dirty
    }

    pub fn keyboard_shown(&self) -> bool {
        self.kb_on
    }

    pub fn describe(&self) -> String {
        format!(
            "weather_screen={:?}\nweather_place={}\nweather_units={}\nweather_places={}\nweather_loaded={}\nweather_watch={}\nweather_quotes={}\nweather_error={}\n",
            self.screen,
            self.place().name,
            self.units.key(),
            self.places.len(),
            self.forecasts.len(),
            self.watch.len(),
            self.quotes.len(),
            self.error.map_or("", |e| e.description())
        )
    }

    /// The summary of the selected place, if its forecast is known.
    pub fn summary(&self) -> Option<Summary> {
        let place = self.place();
        Some(self.forecasts.get(&place.key())?.summary(&place.name, self.units))
    }

    fn place(&self) -> &Location {
        &self.places[self.selected.min(self.places.len() - 1)]
    }

    fn save_summary(&mut self) {
        if let Some(s) = self.summary() {
            self.store.save_summary(&s);
            self.summary_changed = true;
        }
    }

    /// True once after the saved summary changed (see `summary_changed`).
    pub fn take_summary_changed(&mut self) -> bool {
        std::mem::take(&mut self.summary_changed)
    }

    /// The network path changed. Going offline stops new fetches and shows the error screen when no forecast is
    /// cached for the selected place. Coming back online retries every place at once, not after the retry delay.
    pub fn set_online(&mut self, online: bool) {
        if self.online == online {
            return;
        }
        self.online = online;
        self.dirty = true;
        if online {
            self.tried.clear();
            self.maintain_at = f64::NEG_INFINITY;
        } else {
            self.maintain_at = f64::INFINITY;
            if !self.forecasts.contains_key(&self.place().key()) {
                self.error = Some(WeatherError::Unknown);
            }
        }
    }

    // ---- fetching: everything on the global queue, never on the UI thread ----

    /// Applies finished fetches; true if the screen changed.
    pub fn pump(&mut self) -> bool {
        let done = std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner()));
        let changed = !done.is_empty();
        if changed {
            self.maintain_at = f64::NEG_INFINITY;
        }
        for d in done {
            match d {
                Done::Forecast { key, result } => {
                    self.pending.remove(&key);
                    match result {
                        Ok(f) => {
                            self.forecasts.insert(key.clone(), f);
                            self.error = None;
                            if key == self.place().key() {
                                self.save_summary();
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Done::Places { query, result } => {
                    if query == self.field.text().trim() {
                        self.searching = false;
                        match result {
                            Ok(list) => {
                                self.note = list.is_empty().then(|| "No places found".to_string());
                                self.results = list;
                            }
                            Err(e) => {
                                self.results.clear();
                                self.note = Some(e);
                            }
                        }
                    }
                }
                Done::Quotes(result) => {
                    self.quotes_pending = false;
                    match result {
                        Ok(q) => {
                            self.quotes = q;
                            self.quotes_error = None;
                        }
                        Err(e) => self.quotes_error = Some(e),
                    }
                }
            }
        }
        self.dirty |= changed;
        changed
    }

    /// Runs `job` on the global utility queue. The result waits in `done`; a main-queue wake makes the loop call
    /// `tick`, which applies it. Nothing polls for it.
    fn spawn(&self, job: impl FnOnce() -> Done + Send + 'static) {
        let done = self.done.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = job();
            done.lock().unwrap_or_else(|e| e.into_inner()).push(result);
            DispatchQueue::main().async_(|| {});
        });
    }

    /// Asks for every forecast that is missing or old, then records when to look again
    /// (`next_wake` reports it, so nothing polls every tick).
    fn maintain(&mut self, t: f64) {
        if t < self.maintain_at {
            return;
        }
        if !self.online {
            // no path: nothing to ask for until `set_online(true)` (which resets `maintain_at`)
            if !self.forecasts.contains_key(&self.place().key()) {
                self.error = Some(WeatherError::Unknown);
            }
            self.maintain_at = f64::INFINITY;
            return;
        }
        let mut next = f64::INFINITY;
        let mut due: Vec<Location> = vec![];
        for p in &self.places {
            let key = p.key();
            if self.pending.contains(&key) {
                next = next.min(t + RETRY_MS);
                continue;
            }
            let wait = if self.forecasts.contains_key(&key) { REFRESH_MS } else { RETRY_MS }.max(MIN_REFRESH_MS);
            match self.tried.get(&key) {
                Some(&at) if t - at < wait => next = next.min(at + wait),
                _ => {
                    next = next.min(t + wait);
                    due.push(p.clone());
                }
            }
        }
        for p in due {
            let key = p.key();
            self.tried.insert(key.clone(), t);
            self.pending.insert(key.clone());
            let (store, net) = (self.store.clone(), self.fetcher.clone());
            self.spawn(move || Done::Forecast { key, result: store.fetch_forecast(&p, &*net).map_err(|e| WeatherError::from_io(&e)) });
        }
        if self.screen == Screen::Stocks && !self.watch.is_empty() {
            if !self.quotes_pending && t - self.quotes_at >= QUOTES_MS {
                self.refresh_quotes(t);
            }
            next = next.min(self.quotes_at + QUOTES_MS);
        }
        self.maintain_at = next;
    }

    fn refresh_quotes(&mut self, t: f64) {
        self.quotes_at = t;
        self.quotes_pending = true;
        let (list, net) = (self.watch.clone(), self.fetcher.clone());
        self.spawn(move || Done::Quotes(data::fetch_quotes(&list, &*net).map_err(|_| "Quotes unavailable".to_string())));
    }

    fn search(&mut self) {
        let query = self.field.text().trim().to_string();
        if query.is_empty() {
            return;
        }
        self.searching = true;
        self.note = None;
        self.results.clear();
        let net = self.fetcher.clone();
        self.spawn(move || Done::Places { result: data::search_places(&query, &*net).map_err(|_| "Search unavailable".to_string()), query });
    }

    // ---- layout ----

    fn bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }

    fn list_top(&self) -> f32 {
        match self.screen {
            Screen::Main => nav_bottom_pt(),
            Screen::Places => places_list_top_pt(),
            Screen::Stocks => stocks_list_top_pt(),
        }
    }

    fn card_w(&self, ui: &Ui) -> f32 {
        self.w as f32 - ui.pt(2.0 * MARGIN)
    }

    fn hourly_rect(&self, ui: &Ui) -> Rect {
        (ui.pt(MARGIN), ui.pt(nav_bottom_pt() + HOURLY_TOP) - self.scroll.offset(), self.card_w(ui), ui.pt(HOURLY_H))
    }

    fn field_rect(&self, ui: &Ui) -> Rect {
        let top = if self.screen == Screen::Places { places_field_top_pt() } else { stocks_field_top_pt() };
        (ui.pt(MARGIN), ui.pt(top), self.card_w(ui), ui.pt(FIELD_H))
    }

    fn units_rect(&self, ui: &Ui, u: Units) -> Rect {
        let half = self.card_w(ui) / 2.0;
        (ui.pt(MARGIN) + if u == Units::Celsius { 0.0 } else { half }, ui.pt(nav_bottom_pt() + 10.0), half, ui.pt(34.0))
    }

    /// The place's region, with its distance from the selected place when it is another place.
    fn subtitle_with_distance(&self, p: &Location) -> String {
        match away_text(self.place(), p, self.units) {
            Some(d) => format!("{} \u{b7} {}", p.subtitle(), d),
            None => p.subtitle().to_string(),
        }
    }

    /// The rows of the Locations list with their top and height in points.
    fn place_rows(&self) -> Vec<(Row, f32, f32)> {
        let mut rows = vec![];
        let mut top = 0.0;
        for i in 0..self.places.len() {
            rows.push((Row::Place(i), top, ROW));
            top += ROW;
        }
        let head = if self.searching {
            Some("Searching\u{2026}".to_string())
        } else if !self.results.is_empty() {
            Some("Results".to_string())
        } else {
            self.note.clone()
        };
        if let Some(head) = head {
            top += 12.0;
            rows.push((Row::Head(head), top, HEAD));
            top += HEAD;
        }
        for i in 0..self.results.len() {
            rows.push((Row::Result(i), top, ROW));
            top += ROW;
        }
        // the location footer: plain words about what Weather may read (footnote size, secondary grey)
        top += 12.0;
        rows.push((Row::Head(insights::location_note(LOCATION, None)), top, HEAD));
        rows
    }

    fn content_h(&self) -> f32 {
        match self.screen {
            Screen::Main => CONTENT_H,
            Screen::Places => self.place_rows().last().map_or(0.0, |&(_, t, h)| t + h) + 24.0,
            Screen::Stocks => self.watch.len() as f32 * QUOTE_ROW + 48.0,
        }
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let view = (self.bottom(ui) - ui.pt(self.list_top())).max(0.0);
        self.scroll.set_max((ui.pt(self.content_h()) - view).max(0.0));
        let hours = self.forecasts.get(&self.place().key()).map_or(0, |f| f.hours.len());
        self.hscroll.set_max((hours as f32 * ui.pt(HOUR_W) + ui.pt(SPACE_L) - self.card_w(ui)).max(0.0));
    }

    fn go(&mut self, ui: &Ui, screen: Screen, t: f64) {
        self.screen = screen;
        self.kb_on = false;
        self.results.clear();
        self.note = None;
        self.searching = false;
        self.scroll.jump_top();
        self.field = {
            let mut f = TextField::new(ui, if screen == Screen::Places { "Search city" } else { "Add symbol, e.g. AAPL" }, MAC_BODY_PT, false, 64);
            f.focused = false;
            f
        };
        self.maintain(t);
        self.fit_scroll(ui);
        self.dirty = true;
    }

    // ---- input ----

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let nav = ui.pt(nav_bottom_pt());
        if y < nav {
            let p = ui.pt(1.0);
            return match self.bar_layout(ui).hit(x / p, y / p)? {
                BAR_LOCATIONS => Some(Target::Places),
                BAR_STOCKS => Some(Target::Stocks),
                BAR_DONE | BAR_BACK => Some(Target::Back),
                _ => None,
            };
        }
        match self.screen {
            Screen::Main => within(x, y, self.hourly_rect(ui)).then_some(Target::Hourly),
            Screen::Places => {
                for u in [Units::Celsius, Units::Fahrenheit] {
                    if within(x, y, self.units_rect(ui, u)) {
                        return Some(Target::Units(u));
                    }
                }
                if within(x, y, self.field_rect(ui)) {
                    return Some(Target::Field);
                }
                if y < ui.pt(places_list_top_pt()) || y >= self.bottom(ui) {
                    return None;
                }
                let at = (y - ui.pt(places_list_top_pt()) + self.scroll.offset()) / ui.s;
                let at = at.max(0.0);
                self.place_rows().into_iter().find(|&(_, t, h)| at >= t && at < t + h).and_then(|(row, _, _)| match row {
                    Row::Place(i) if x >= self.w as f32 - ui.pt(MARGIN + REMOVE_W) && self.places.len() > 1 => Some(Target::Remove(i)),
                    Row::Place(i) => Some(Target::Place(i)),
                    Row::Result(i) => Some(Target::Result(i)),
                    Row::Head(_) => None,
                })
            }
            Screen::Stocks => {
                if within(x, y, self.field_rect(ui)) {
                    return Some(Target::Field);
                }
                if y < ui.pt(stocks_list_top_pt()) || y >= self.bottom(ui) || x < self.w as f32 - ui.pt(MARGIN + REMOVE_W) {
                    return None;
                }
                let i = ((y - ui.pt(stocks_list_top_pt()) + self.scroll.offset()) / ui.pt(QUOTE_ROW)) as usize;
                (i < self.watch.len()).then_some(Target::RemoveQuote(i))
            }
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, t: f64) {
        self.dirty = true;
        match target {
            Target::Places => self.go(ui, Screen::Places, t),
            Target::Stocks => self.go(ui, Screen::Stocks, t),
            Target::Back => self.go(ui, Screen::Main, t),
            Target::Units(u) => {
                self.units = u;
                self.store.save_settings(self.units, self.selected);
                self.save_summary();
            }
            Target::Field => {
                self.field.focused = true;
                self.kb_on = true;
                let r = self.field_rect(ui);
                self.field.tap(ui, x - r.0 - ui.pt(SPACE_M), t);
                self.fit_scroll(ui);
            }
            Target::Place(i) => {
                self.selected = i;
                self.store.save_settings(self.units, i);
                self.save_summary();
                self.hscroll.jump_top();
                self.go(ui, Screen::Main, t);
            }
            Target::Remove(i) => {
                if self.places.len() > 1 && i < self.places.len() {
                    let gone = self.places.remove(i);
                    self.forecasts.remove(&gone.key());
                    if self.selected > i || self.selected >= self.places.len() {
                        self.selected = self.selected.saturating_sub(1);
                    }
                    self.store.save_locations(&self.places);
                    self.store.save_settings(self.units, self.selected);
                    self.save_summary();
                    self.fit_scroll(ui);
                }
            }
            Target::Result(i) => {
                if let Some(p) = self.results.get(i).cloned() {
                    if self.places.len() < data::MAX_LOCATIONS && !self.places.iter().any(|q| q.key() == p.key()) {
                        self.places.push(p);
                        self.store.save_locations(&self.places);
                        self.maintain(t);
                    }
                    self.results.clear();
                    self.note = None;
                    self.field.set_text(ui, "");
                    self.field.focused = false;
                    self.kb_on = false;
                    self.fit_scroll(ui);
                }
            }
            Target::RemoveQuote(i) => {
                if i < self.watch.len() {
                    let gone = self.watch.remove(i);
                    self.quotes.retain(|q| q.symbol != gone);
                    self.store.save_watchlist(&self.watch);
                    self.fit_scroll(ui);
                }
            }
            Target::Hourly => {}
        }
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.maintain_at = f64::NEG_INFINITY;
        self.dirty = true;
        if out == Out::Hide || !self.field.focused {
            if out == Out::Hide {
                self.field.focused = false;
                self.kb_on = false;
                self.fit_scroll(ui);
            }
            return;
        }
        match (self.screen, &out) {
            (Screen::Places, Out::Enter) => {
                self.field.focused = false;
                self.kb_on = false;
                self.search();
            }
            (Screen::Stocks, Out::Enter) => {
                if let Some(sym) = data::normalise_symbol(self.field.text()) {
                    if !self.watch.contains(&sym) && self.watch.len() < data::MAX_WATCH {
                        self.watch.push(sym);
                        self.store.save_watchlist(&self.watch);
                        self.refresh_quotes(t);
                    }
                    self.field.set_text(ui, "");
                }
            }
            (Screen::Places | Screen::Stocks, _) => {
                self.field.edit(ui, &out, t);
                if self.screen == Screen::Places && self.field.is_empty() {
                    self.results.clear();
                    self.note = None;
                    self.searching = false;
                }
            }
            _ => {}
        }
        self.fit_scroll(ui);
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.maintain_at = f64::NEG_INFINITY;
        self.relayout(ui);
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.dirty = true;
        // the sidebar: the search capsule opens Locations with its field focused; a row shows that place
        if matches!(ev, Event::Down(..)) && x < self.side {
            self.side_pressed = self.side_hit(ui, x, y);
            return;
        }
        if let Some(pressed) = self.side_pressed {
            if let Event::Up(..) = ev {
                self.side_pressed = None;
                if self.side_hit(ui, x, y) == Some(pressed) {
                    match pressed {
                        SideTarget::Search => {
                            if self.screen != Screen::Places {
                                self.activate(ui, Target::Places, 0.0, t);
                            }
                            self.activate(ui, Target::Field, self.field_rect(ui).0 + ui.pt(SPACE_M), t);
                        }
                        SideTarget::Place(i) => self.activate(ui, Target::Place(i), 0.0, t),
                    }
                }
            }
            return;
        }
        // the detail pane works in its own coordinates
        let x = x - self.side;
        let ev = match ev {
            Event::Down(_, y) => Event::Down(x, y),
            Event::Move(_, y) => Event::Move(x, y),
            Event::Up(_, y) => Event::Up(x, y),
            e => e,
        };
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                self.swipe = match self.pressed {
                    Some(Target::Place(i) | Target::Remove(i)) if self.screen == Screen::Places => Some((x, y, i)),
                    _ => None,
                };
                if self.pressed == Some(Target::Hourly) {
                    self.scrolling = Some(true);
                    self.hscroll.down(x, t);
                } else if y >= ui.pt(self.list_top()) && y < self.bottom(ui) {
                    self.scrolling = Some(false);
                    self.scroll.down(y, t);
                }
            }
            Event::Move(..) => {
                if let Some(horizontal) = self.scrolling {
                    let s = if horizontal { &mut self.hscroll } else { &mut self.scroll };
                    s.moved(if horizontal { x } else { y }, t);
                    if s.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) => {
                if let Some((x0, y0, i)) = self.swipe.take() {
                    if x0 - x > ui.pt(80.0) && (x0 - x) > 2.0 * (y - y0).abs() {
                        self.scrolling = None;
                        self.pressed = None;
                        self.activate(ui, Target::Remove(i), x, t);
                        return;
                    }
                }
                let tap = match self.scrolling.take() {
                    Some(true) => self.hscroll.up(x, t),
                    Some(false) => self.scroll.up(y, t),
                    None => true,
                };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|p| self.hit(ui, x, y) == Some(*p)) {
                        self.activate(ui, target, x, t);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.relayout(ui);
        self.pump();
        self.maintain(t);
        self.fit_scroll(ui);
        let was_moving = self.scroll.animating() || self.hscroll.animating();
        let moving = self.scroll.tick(t) | self.hscroll.tick(t);
        self.dirty |= was_moving || moving;
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.scroll.animating() || self.hscroll.animating() {
            return Some(t);
        }
        let mut due = if self.field.focused { self.field.next_blink(t) } else { None };
        if self.maintain_at.is_finite() {
            due = Some(due.map_or(self.maintain_at, |d| d.min(self.maintain_at)));
        }
        due
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

    fn draw_field(&self, s: &mut Surface, ui: &Ui, t: f64) {
        let (x, y, w, h) = self.field_rect(ui);
        s.fill_rrect(x, y, w, h, ui.pt(RADIUS), CARD_BG.now());
        self.field.draw(s, ui, (x + ui.pt(SPACE_M), y, w - ui.pt(SPACE_XXL), h), t);
    }

    /// Draws the selected place; its forecast is taken out of the map for the frame so the
    /// body can borrow it while `put` borrows `self` mutably, without cloning it.
    fn draw_main(&mut self, s: &mut Surface, ui: &Ui) {
        let (key, name) = (self.place().key(), self.place().name.clone());
        let forecast = self.forecasts.remove(&key);
        self.draw_main_with(s, ui, &name, forecast.as_ref());
        if let Some(f) = forecast {
            self.forecasts.insert(key, f);
        }
    }

    fn draw_main_with(&mut self, s: &mut Surface, ui: &Ui, place_name: &str, forecast: Option<&Forecast>) {
        let (top, bottom) = (ui.pt(nav_bottom_pt()) as usize, self.bottom(ui) as usize);
        let (sky, day) = forecast.map_or((Sky::Cloudy, true), |f| (Sky::from_code(f.current.code), f.current.day));
        let key = (sky, day, self.w.max(0) as usize, self.h.max(0) as usize);
        if self.backdrop_img.as_ref().map(|(k, _)| *k) != Some(key) {
            let (a, b) = backdrop(sky, day);
            let mut img = Image::new(key.2, key.3);
            for y in 0..key.3 {
                let k = y as f32 / self.h.max(1) as f32;
                let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * k) as u8;
                let row = pack(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2), 255);
                img.px[y * key.2..(y + 1) * key.2].fill(row);
            }
            self.backdrop_img = Some((key, img));
        }
        if let Some((_, img)) = &self.backdrop_img {
            s.blit(img, 0, 0, 255, 255);
        }
        let _ = place_name;
        let cx = self.w as f32 / 2.0;
        let Some(f) = forecast else {
            let text = self.error.map_or("Loading\u{2026}", |e| e.description());
            self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, WHITE, Align::Centre, cx, ui.pt(nav_bottom_pt() + 200.0));
            if let Some(e) = self.error {
                self.put(s, ui, e.recovery_suggestion(), MAC_FOOTNOTE_PT, Weight::Regular, fade(WHITE, 160), Align::Centre, cx, ui.pt(nav_bottom_pt() + 226.0));
            }
            return;
        };
        let mut c = s.rows(top, bottom);
        let oy = ui.pt(nav_bottom_pt()) - self.scroll.offset();
        let p = |pt: f32| oy + ui.pt(pt);
        let units = self.units;
        glyph::draw(&mut c, sky, day, (cx, p(64.0)), ui.pt(SPACE_HUGE));
        self.put(&mut c, ui, &units.degrees(f.current.temp), DISPLAY_LARGE_PT, Weight::Regular, WHITE, Align::Centre, cx, p(160.0));
        self.put(&mut c, ui, sky.label(), MAC_TITLE3_PT, Weight::Medium, WHITE, Align::Centre, cx, p(232.0));
        if let Some(d) = f.days.first() {
            let hl = format!("H:{}  L:{}", units.degrees(d.max), units.degrees(d.min));
            self.put(&mut c, ui, &hl, MAC_BODY_PT, Weight::Medium, WHITE, Align::Centre, cx, p(262.0));
        }
        let dim = fade(WHITE, 160);
        let (card_x, card_w) = (ui.pt(MARGIN), self.card_w(ui));
        let card = SKY_SCRIM;
        // hourly strip: drawn into its own image so the columns clip at the card
        c.fill_rrect(card_x, p(HOURLY_TOP), card_w, ui.pt(HOURLY_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "HOURLY FORECAST", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(HOURLY_TOP + 16.0));
        let mut strip = Image { w: card_w as usize, h: ui.pt(HOURLY_H - 30.0) as usize, px: vec![0; card_w as usize * ui.pt(HOURLY_H - 30.0) as usize] };
        {
            let mut ss = Surface { w: strip.w, h: strip.h, stride: strip.w, px: &mut strip.px, y0: 0 };
            for (i, hr) in f.hours.iter().enumerate() {
                let mid = ui.pt(SPACE_S) + (i as f32 + 0.5) * ui.pt(HOUR_W) - self.hscroll.offset();
                if mid < -ui.pt(HOUR_W) || mid > card_w + ui.pt(HOUR_W) {
                    continue;
                }
                let label = if i == 0 { "Now".to_string() } else { hr.label.clone() };
                self.put(&mut ss, ui, &label, MAC_BODY_PT, Weight::Medium, WHITE, Align::Centre, mid, ui.pt(SPACE_M));
                glyph::draw(&mut ss, Sky::from_code(hr.code), hr.day, (mid, ui.pt(SPACE_HUGE)), ui.pt(SPACE_L));
                self.put(&mut ss, ui, &units.degrees(hr.temp), MAC_BODY_PT, Weight::Semibold, WHITE, Align::Centre, mid, ui.pt(68.0));
            }
        }
        c.blit(&strip, card_x as i32, p(HOURLY_TOP + 30.0) as i32, 255, 255);
        // seven days
        c.fill_rrect(card_x, p(DAILY_TOP), card_w, ui.pt(DAILY_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "10-DAY FORECAST", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(DAILY_TOP + 16.0));
        let lo = f.days.iter().map(|d| d.min).fold(f64::MAX, f64::min);
        let hi = f.days.iter().map(|d| d.max).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(1.0) as f32;
        let (bar_x, bar_w) = (card_x + ui.pt(176.0), card_w - ui.pt(176.0 + 80.0));
        for (i, d) in f.days.iter().enumerate() {
            let mid = p(28.0 + DAILY_TOP + (i as f32 + 0.5) * DAY_ROW);
            if i > 0 {
                c.fill_rrect(card_x + ui.pt(SPACE_L), mid - ui.pt(DAY_ROW / 2.0), card_w - ui.pt(28.0), 1.0, 0.0, fade(WHITE, 40));
            }
            let label = if i == 0 { "Today" } else { d.label.as_str() };
            self.put(&mut c, ui, label, MAC_BODY_PT, Weight::Medium, WHITE, Align::Left, card_x + ui.pt(SPACE_L), mid);
            glyph::draw(&mut c, Sky::from_code(d.code), true, (card_x + ui.pt(100.0), mid), ui.pt(SPACE_M));
            self.put(&mut c, ui, &units.degrees(d.min), MAC_BODY_PT, Weight::Medium, dim, Align::Right, bar_x - ui.pt(SPACE_M), mid);
            c.fill_rrect(bar_x, mid - ui.pt(SPACE_XXS), bar_w, ui.pt(SPACE_XS), ui.pt(SPACE_XXS), fade(WHITE, 60));
            let (x0, x1) = (((d.min - lo as f64) as f32 / span).clamp(0.0, 1.0), ((d.max - lo as f64) as f32 / span).clamp(0.0, 1.0));
            c.fill_rrect(bar_x + bar_w * x0, mid - ui.pt(SPACE_XXS), (bar_w * (x1 - x0)).max(ui.pt(SPACE_XS)), ui.pt(SPACE_XS), ui.pt(SPACE_XXS), ORANGE);
            if i == 0 {
                let k = ((f.current.temp - lo) as f32 / span).clamp(0.0, 1.0);
                c.fill_rrect(bar_x + bar_w * k - ui.pt(SPACE_XS), mid - ui.pt(SPACE_XS), ui.pt(SPACE_S), ui.pt(SPACE_S), ui.pt(SPACE_XS), WHITE);
            }
            self.put(&mut c, ui, &units.degrees(d.max), MAC_BODY_PT, Weight::Semibold, WHITE, Align::Right, card_x + card_w - ui.pt(SPACE_L), mid);
        }
        // detail tiles: two columns of rounded cards, a caption, a big value and a note
        let today = f.days.first();
        let wind = wind_text(f.current.wind_kmh, self.units);
        let (moon_value, moon_note) = moon_text(now_unix());
        let uv = today.map_or(0.0, |d| d.uv);
        let uv_note = match uv.round() as i32 {
            0..=2 => "Low",
            3..=5 => "Moderate",
            6..=7 => "High",
            8..=10 => "Very High",
            _ => "Extreme",
        };
        let sun = |d: Option<&data::Day>, rise: bool| d.map(|d| if rise { d.sunrise.clone() } else { d.sunset.clone() }).filter(|s| !s.is_empty()).unwrap_or_else(|| "--:--".into());
        let delta = f.current.feels.round() - f.current.temp.round();
        let feels_note = if delta.abs() < 1.0 { "Similar to the actual temperature" } else if delta < 0.0 { "Wind makes it feel colder" } else { "Humidity makes it feel warmer" };
        let tiles = [
            ("UV INDEX", format!("{}", uv.round() as i32), uv_note.to_string()),
            ("SUNRISE", sun(today, true), format!("Sunset: {}", sun(today, false))),
            ("WIND", wind, "Current speed".to_string()),
            ("HUMIDITY", format!("{}%", f.current.humidity), "Relative humidity".to_string()),
            ("FEELS LIKE", units.degrees(f.current.feels), feels_note.to_string()),
            ("MOON", moon_value, moon_note),
        ];
        let tile_w = (card_w - ui.pt(TILE_GAP)) / 2.0;
        for (i, (name, value, note)) in tiles.iter().enumerate() {
            let tx = card_x + (i % 2) as f32 * (tile_w + ui.pt(TILE_GAP));
            let ty = p(DETAILS_TOP + (i / 2) as f32 * (TILE_H + TILE_GAP));
            c.fill_rrect(tx, ty, tile_w, ui.pt(TILE_H), ui.pt(RADIUS), card);
            self.put(&mut c, ui, name, MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(SPACE_XL));
            self.put(&mut c, ui, value, MAC_LARGE_TITLE_PT, Weight::Medium, WHITE, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(54.0));
            let note = ui.fit(note, MAC_FOOTNOTE_PT, tile_w - ui.pt(28.0));
            self.put(&mut c, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, WHITE, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(86.0));
        }
        // temperature chart: the hours as a line over labelled rules (points are in the card's own frame)
        let card_pt = self.w as f32 / ui.pt(1.0) - 2.0 * MARGIN;
        c.fill_rrect(card_x, p(CHART_TOP), card_w, ui.pt(CHART_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "TEMPERATURE", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(CHART_TOP + 16.0));
        if let Some(scale) = insights::temperature_scale(&f.hours) {
            let frame = Frame::new(CHART_AXIS as f64, CHART_PLOT_TOP as f64, (card_pt - CHART_AXIS - SPACE_L) as f64, CHART_PLOT_H as f64);
            let plot = insights::temperature_plot(&scale, f.hours.len(), frame);
            for &tick in &scale.ticks {
                let y = plot.y.map(tick) as f32;
                c.fill_rrect(card_x + ui.pt(CHART_AXIS), p(CHART_TOP + y), ui.pt(frame.w as f32), 1.0, 0.0, fade(WHITE, 40));
                self.put(&mut c, ui, &units.degrees(tick), MAC_CAPTION1_PT, Weight::Regular, dim, Align::Right, card_x + ui.pt(CHART_AXIS - SPACE_S), p(CHART_TOP + y));
            }
            let d = ui.pt(2.0);
            for pair in insights::temperature_line(&plot, &f.hours).windows(2) {
                let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                let steps = ((bx - ax).abs().max((by - ay).abs()) / 0.5).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    let (x, y) = ((ax + (bx - ax) * t) as f32, (ay + (by - ay) * t) as f32);
                    c.fill_rrect(card_x + ui.pt(x) - d / 2.0, p(CHART_TOP + y) - d / 2.0, d, d, d / 2.0, WHITE);
                }
            }
            for (x, y) in insights::temperature_points(&f.hours) {
                let (px, py) = plot.position(x, y);
                let dot = ui.pt(5.0);
                c.fill_rrect(card_x + ui.pt(px as f32) - dot / 2.0, p(CHART_TOP + py as f32) - dot / 2.0, dot, dot, dot / 2.0, WHITE);
            }
            let n = f.hours.len();
            if n > 1 {
                let (x0, _) = plot.position(0.0, scale.lo);
                let (x1, _) = plot.position((n - 1) as f64, scale.lo);
                let label_y = p(CHART_TOP + CHART_PLOT_TOP + CHART_PLOT_H + 14.0);
                self.put(&mut c, ui, "Now", MAC_CAPTION1_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(x0 as f32), label_y);
                self.put(&mut c, ui, &f.hours[n - 1].label, MAC_CAPTION1_PT, Weight::Regular, dim, Align::Right, card_x + ui.pt(x1 as f32), label_y);
            }
        }
        // map: a coordinate grid around the place with a centre mark and a scale bar; no map tiles are loaded
        c.fill_rrect(card_x, p(MAP_TOP), card_w, ui.pt(MAP_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "MAP", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(MAP_TOP + 16.0));
        let (plat, plon) = (self.place().lat, self.place().lon);
        let area_w = card_pt - 2.0 * SPACE_L;
        if let Some(view) = insights::map_view(plat, plon, area_w as f64, MAP_AREA_H as f64) {
            let (ax, ay) = (card_x + ui.pt(SPACE_L), p(MAP_TOP + MAP_AREA_TOP));
            let (aw, ah) = (ui.pt(area_w), ui.pt(MAP_AREA_H));
            c.fill_rrect(ax, ay, aw, ah, ui.pt(SPACE_S), fade(WHITE, 24));
            let grid = insights::grid(&view);
            let line = fade(WHITE, 90);
            for &(y, lat) in &grid.rows {
                let yy = ay + ui.pt(y as f32);
                c.fill_rrect(ax, yy, aw, 1.0, 0.0, line);
                let text = insights::coordinate_label(lat, 'N', 'S');
                self.put(&mut c, ui, &text, MAC_CAPTION1_PT, Weight::Regular, WHITE, Align::Left, ax + ui.pt(SPACE_S), yy + ui.pt(SPACE_S));
            }
            for &(x, lon) in &grid.cols {
                let xx = ax + ui.pt(x as f32);
                c.fill_rrect(xx, ay, 1.0, ah, 0.0, line);
                let text = insights::coordinate_label(lon, 'E', 'W');
                self.put(&mut c, ui, &text, MAC_CAPTION1_PT, Weight::Regular, WHITE, Align::Left, xx + ui.pt(SPACE_XS), ay + ah - ui.pt(SPACE_L));
            }
            let mark = ui.pt(10.0);
            c.fill_rrect(ax + aw / 2.0 - mark / 2.0, ay + ah / 2.0 - mark / 2.0, mark, mark, mark / 2.0, WHITE);
            if let Some(bar) = view.camera().scale_bar(insights::SCALE_MAX_PT) {
                let bw = ui.pt(bar.width_px as f32);
                let (bx, by) = (ax + aw - ui.pt(SPACE_S) - bw, ay + ah - ui.pt(SPACE_M));
                c.fill_rrect(bx, by, bw, ui.pt(3.0), 0.0, WHITE);
                let text = insights::scale_text(bar.length_m, self.units);
                self.put(&mut c, ui, &text, MAC_CAPTION1_PT, Weight::Regular, WHITE, Align::Right, bx - ui.pt(SPACE_XS), by + ui.pt(1.5));
            }
            let note = ui.fit(&insights::map_note(view.map_type), MAC_FOOTNOTE_PT, card_w - ui.pt(2.0 * SPACE_L));
            self.put(&mut c, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(SPACE_L), p(MAP_TOP + MAP_NOTE_TOP));
        }
    }

    fn draw_places(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (top, bottom) = (ui.pt(places_list_top_pt()) as usize, self.bottom(ui) as usize);
        let rows = self.place_rows();
        let (x, w) = (ui.pt(MARGIN), self.card_w(ui));
        {
            let mut c = s.rows(top, bottom);
            let oy = ui.pt(places_list_top_pt()) - self.scroll.offset();
            for (row, rt, rh) in rows {
                let y = oy + ui.pt(rt);
                if y + ui.pt(rh) < top as f32 || y > bottom as f32 {
                    continue;
                }
                let mid = y + ui.pt(rh) / 2.0;
                match row {
                    Row::Head(text) => {
                        let color = LABEL_2.now();
                        self.put(&mut c, ui, &text, MAC_FOOTNOTE_PT, Weight::Semibold, color, Align::Left, x + ui.pt(SPACE_XS), mid);
                    }
                    Row::Place(i) => {
                        let p = self.places[i].clone();
                        c.fill_rrect(x, y, w, ui.pt(rh - 4.0), ui.pt(RADIUS), CARD_BG.now());
                        if i == self.selected {
                            c.fill_rect(x as usize, (y + ui.pt(SPACE_S)) as usize, ui.pt(SPACE_XS) as usize, ui.pt(rh - 20.0) as usize, LINK);
                        }
                        let name = ui.fit(&p.name, MAC_BODY_PT, w - ui.pt(150.0));
                        self.put(&mut c, ui, &name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                        let region = ui.fit(&self.subtitle_with_distance(&p), MAC_FOOTNOTE_PT, w - ui.pt(150.0));
                        self.put(&mut c, ui, &region, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(37.0));
                        let right = if self.places.len() > 1 { x + w - ui.pt(REMOVE_W) } else { x + w - ui.pt(SPACE_L) };
                        let temp = self.forecasts.get(&p.key()).map_or("\u{2013}".to_string(), |f| self.units.degrees(f.current.temp));
                        self.put(&mut c, ui, &temp, MAC_TITLE1_PT, Weight::Regular, LABEL.now(), Align::Right, right - ui.pt(SPACE_S), y + ui.pt(26.0));
                        if self.places.len() > 1 {
                            self.put(&mut c, ui, "\u{d7}", MAC_TITLE1_PT, Weight::Regular, DESTRUCTIVE, Align::Centre, x + w - ui.pt(REMOVE_W / 2.0), y + ui.pt(26.0));
                        }
                    }
                    Row::Result(i) => {
                        let p = self.results[i].clone();
                        c.fill_rrect(x, y, w, ui.pt(rh - 4.0), ui.pt(RADIUS), CARD_BG.now());
                        let name = ui.fit(&p.name, MAC_BODY_PT, w - ui.pt(80.0));
                        self.put(&mut c, ui, &name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                        let region = ui.fit(&self.subtitle_with_distance(&p), MAC_FOOTNOTE_PT, w - ui.pt(80.0));
                        self.put(&mut c, ui, &region, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(37.0));
                        self.put(&mut c, ui, "+", MAC_LARGE_TITLE_PT, Weight::Regular, LINK, Align::Centre, x + w - ui.pt(REMOVE_W / 2.0), y + ui.pt(26.0));
                    }
                }
            }
        }
        s.fill_rect(0, 0, self.w as usize, top, CONTENT_BG.now());
        for (u, label) in [(Units::Celsius, "\u{b0}C"), (Units::Fahrenheit, "\u{b0}F")] {
            let r = self.units_rect(ui, u);
            let on = self.units == u;
            s.fill_rrect(r.0, r.1, r.2, r.3, ui.pt(RADIUS), if on { LINK } else { CARD_BG.now() });
            self.put(s, ui, label, MAC_SUBHEAD_PT, Weight::Semibold, if on { WHITE } else { LABEL.now() }, Align::Centre, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        }
        s.fill_rect(ui.pt(MARGIN) as usize + (self.card_w(ui) / 2.0) as usize, ui.pt(nav_bottom_pt() + 10.0) as usize, 1, ui.pt(34.0) as usize, SEPARATOR_C.now());
        self.draw_field(s, ui, t);
    }

    fn draw_stocks(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (top, bottom) = (ui.pt(stocks_list_top_pt()) as usize, self.bottom(ui) as usize);
        let (x, w) = (ui.pt(MARGIN), self.card_w(ui));
        {
            let mut c = s.rows(top, bottom);
            let oy = ui.pt(stocks_list_top_pt()) - self.scroll.offset();
            if self.watch.is_empty() {
                self.put(&mut c, ui, "Your watchlist is empty", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), Align::Centre, self.w as f32 / 2.0, oy + ui.pt(SPACE_HUGE));
            }
            for (i, sym) in self.watch.clone().iter().enumerate() {
                let y = oy + i as f32 * ui.pt(QUOTE_ROW);
                if y + ui.pt(QUOTE_ROW) < top as f32 || y > bottom as f32 {
                    continue;
                }
                c.fill_rrect(x, y, w, ui.pt(QUOTE_ROW - 6.0), ui.pt(RADIUS), CARD_BG.now());
                self.put(&mut c, ui, &data::display_symbol(sym), MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(27.0));
                let right = x + w - ui.pt(REMOVE_W);
                match self.quotes.iter().find(|q| &q.symbol == sym).cloned() {
                    Some(q) => {
                        let pct = q.change_pct();
                        let color = if pct < 0.0 { bifrost_app::kit::ui::SYSTEM_RED } else { GREEN };
                        self.put(&mut c, ui, &format!("{:.2}", q.close), MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Right, right, y + ui.pt(SPACE_L));
                        self.put(&mut c, ui, &format!("{:+.2}%", pct), MAC_BODY_PT, Weight::Medium, color, Align::Right, right, y + ui.pt(38.0));
                    }
                    None => self.put(&mut c, ui, "\u{2013}", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Right, right, y + ui.pt(27.0)),
                }
                self.put(&mut c, ui, "\u{d7}", MAC_TITLE1_PT, Weight::Regular, DESTRUCTIVE, Align::Centre, x + w - ui.pt(REMOVE_W / 2.0), y + ui.pt(27.0));
            }
            if let Some(e) = self.quotes_error.clone().filter(|_| !self.watch.is_empty()) {
                let y = oy + self.watch.len() as f32 * ui.pt(QUOTE_ROW) + ui.pt(SPACE_M);
                self.put(&mut c, ui, &e, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Centre, self.w as f32 / 2.0, y);
            }
        }
        s.fill_rect(0, 0, self.w as usize, top, CONTENT_BG.now());
        self.draw_field(s, ui, t);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.relayout(ui);
        self.dirty = false;
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        if self.side > 0.0 {
            self.draw_sidebar(s, ui);
        }
        // the detail pane draws in its own coordinates, right of the sidebar
        let side = (self.side as usize).min(s.w);
        let mut d = Surface { w: s.w - side, h: s.h, stride: s.stride, px: &mut s.px[side..], y0: s.y0 };
        match self.screen {
            Screen::Main => self.draw_main(&mut d, ui),
            Screen::Places => self.draw_places(&mut d, ui, t),
            Screen::Stocks => self.draw_stocks(&mut d, ui, t),
        }
        self.draw_bar(&mut d, ui);
    }
}

/// Wind speed as the detail tile shows it: km/h with the Celsius setting, mph with Fahrenheit (the imperial set).
fn wind_text(kmh: f64, units: Units) -> String {
    match units {
        Units::Celsius => format!("{} km/h", kmh.round() as i32),
        Units::Fahrenheit => format!("{} mph", Wind::new(kmh, 0.0, None).speed_mph().round() as i32),
    }
}

/// The distance from one saved or found place to another, such as "343 km away", in the unit system of the
/// temperature setting. None for the same place or for coordinates that are out of range.
fn away_text(from: &Location, to: &Location, units: Units) -> Option<String> {
    if from.key() == to.key() {
        return None;
    }
    let a = Coordinate::new(from.lat, from.lon)?;
    let b = Coordinate::new(to.lat, to.lon)?;
    let system = match units {
        Units::Celsius => DistanceUnits::Metric,
        Units::Fahrenheit => DistanceUnits::Imperial,
    };
    Some(format!("{} away", format_distance(a.distance_m(b), system)))
}

/// The Moon tile at a Unix time: the lit share of the disk as a value and the phase name as the note.
fn moon_text(unix: f64) -> (String, String) {
    let lit = (illuminated_fraction(unix) * 100.0).round() as i32;
    (format!("{lit}%"), MoonPhase::at_unix(unix).label().to_string())
}

fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ui() -> Ui {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
        ui.symbols = springboard::symbols::Symbols::load(dir);
        ui
    }

    /// A network that is always down: the app must keep showing what it has.
    struct Offline;

    impl Fetch for Offline {
        fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "offline"))
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("weather-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn place(name: &str, lat: f64) -> Location {
        Location { name: name.into(), region: "Test".into(), lat, lon: -20.0 }
    }

    fn luma((r, g, b): (u8, u8, u8)) -> u32 {
        r as u32 + g as u32 + b as u32
    }

    #[test]
    fn night_sky_is_darker_than_day_sky() {
        let day = backdrop(Sky::Clear, true);
        let night = backdrop(Sky::Clear, false);
        assert!(luma(night.0) < luma(day.0));
        assert!(luma(night.1) < luma(day.1));
    }

    #[test]
    fn storm_and_rain_skies_ignore_the_time_of_day() {
        assert_eq!(backdrop(Sky::Thunder, true), backdrop(Sky::Thunder, false));
        assert_eq!(backdrop(Sky::Rain, true), backdrop(Sky::Rain, false));
    }

    #[test]
    fn saved_places_settings_and_watchlist_load_on_start() {
        let dir = temp("load");
        let store = Store::new(dir.clone());
        store.save_locations(&[place("Akureyri", 65.68), place("Reykjavik", 64.14)]);
        store.save_settings(Units::Fahrenheit, 1);
        store.save_watchlist(&["aapl.us".to_string()]);

        let ui = test_ui();
        let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        let text = app.describe();
        assert!(text.contains("weather_units=F\n"), "{text}");
        assert!(text.contains("weather_places=2\n"), "{text}");
        assert!(text.contains("weather_place=Reykjavik\n"), "{text}");
        assert!(text.contains("weather_watch=1\n"), "{text}");
        assert!(text.contains("weather_loaded=0\n"), "{text}");
        assert!(app.summary().is_none(), "no cached forecast yet");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn going_offline_with_nothing_cached_shows_the_error_and_coming_back_clears_the_wait() {
        let dir = temp("offline");
        let ui = test_ui();
        let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        assert!(app.online, "the app starts online until the shell says otherwise");
        app.set_online(false);
        assert!(!app.online);
        assert!(app.describe().contains("weather_error=The weather could not be loaded.\n"), "{}", app.describe());
        app.set_online(true);
        assert!(app.online);
        app.set_online(true);
        assert!(app.online, "a repeated state changes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_app_has_no_summary_change_to_report() {
        let dir = temp("summary-flag");
        let ui = test_ui();
        let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        assert!(!app.take_summary_changed(), "nothing saved yet");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_launch_starts_with_reykjavik_in_celsius() {
        let dir = temp("first");
        let ui = test_ui();
        let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        let text = app.describe();
        assert!(text.contains("weather_units=C\n"), "{text}");
        assert!(text.contains("weather_place=Reykjav"), "{text}");
        assert!(text.contains("weather_places=1\n"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_size_updates_the_window_and_marks_it_dirty() {
        let dir = temp("size");
        let ui = test_ui();
        let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        app.dirty = false;
        app.set_size(900, 700);
        assert_eq!(app.full_w, 900);
        assert_eq!(app.h, 700);
        assert!(app.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn places_end_with_the_location_note() {
        let dir = temp("location-note");
        let ui = test_ui();
        let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        match app.place_rows().last() {
            Some((Row::Head(text), _, _)) => assert!(text.starts_with("Location: not asked"), "{text}"),
            _ => panic!("the list should end with the location note"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}


#[cfg(test)]
mod wire_tests {
    use super::*;

    fn loc(name: &str, lat: f64, lon: f64) -> Location {
        Location { name: name.into(), region: "Iceland".into(), lat, lon }
    }

    #[test]
    fn wind_follows_the_temperature_setting_unit_system() {
        assert_eq!(wind_text(20.4, Units::Celsius), "20 km/h");
        assert_eq!(wind_text(100.0, Units::Fahrenheit), "62 mph");
    }

    #[test]
    fn distance_is_metric_or_imperial_and_omitted_for_the_same_place() {
        let reykjavik = loc("Reykjavik", 64.1466, -21.9426);
        let paris = loc("Paris", 48.8566, 2.3522);
        let metric = away_text(&reykjavik, &paris, Units::Celsius).unwrap();
        assert!(metric.ends_with(" km away"), "{metric}");
        assert!(!metric.starts_with("0"), "{metric}");
        let imperial = away_text(&reykjavik, &paris, Units::Fahrenheit).unwrap();
        assert!(imperial.ends_with(" mi away"), "{imperial}");
        assert_eq!(away_text(&reykjavik, &reykjavik, Units::Celsius), None);
        assert_eq!(away_text(&reykjavik, &loc("Off", 95.0, 0.0), Units::Celsius), None);
    }

    #[test]
    fn moon_tile_shows_new_and_full_moon_at_the_cycle_marks() {
        let reference_new_moon = 947_182_440.0;
        assert_eq!(moon_text(reference_new_moon), ("0%".to_string(), "New Moon".to_string()));
        let half = reference_new_moon + bifrost_app::controls::weather_model::SYNODIC_MONTH_DAYS / 2.0 * 86_400.0;
        assert_eq!(moon_text(half), ("100%".to_string(), "Full Moon".to_string()));
    }

    #[test]
    fn subtitle_gets_distance_only_for_other_places() {
        let here = loc("Here", 64.1466, -21.9426);
        let other = loc("There", 65.6835, -18.1262);
        assert_eq!(here.subtitle(), "Iceland");
        assert!(away_text(&here, &other, Units::Celsius).is_some());
        assert!(away_text(&here, &here, Units::Celsius).is_none());
    }
}
