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
use bifrost_app::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::ContextItem;
use bifrost_core::json::Json;
use bifrost_app::controls::chart_marks::Frame;
use bifrost_app::controls::location_auth::{AuthorizationStatus, LocationError, RequestOutcome};
use bifrost_app::core_location::{Delegate, Location as Fix, LocationManager};
use bifrost_app::{privacy_permissions, Ctx};
use crate::control;
use crate::insights::{self, wind_text, Column};
use crate::notify::{self, Notice, RainWatch};
use bifrost_app::controls::weather_model::{illuminated_fraction, MoonPhase, WeatherError};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bifrost_app::kit::ui::SYSTEM_GREEN as GREEN;
use bifrost_app::kit::ui::SYSTEM_ORANGE as ORANGE;
use bifrost_app::kit::ui::SYSTEM_YELLOW as YELLOW;
use bifrost_app::kit::ui::SYSTEM_RED as RED;
use bifrost_app::kit::ui::SYSTEM_PURPLE as PURPLE;
use bifrost_app::kit::ui::SYSTEM_TEAL as TEAL;
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
const BAR_SHARE: u32 = 5;
/// Per-place Notifications switch on the Locations page (`NOTIFY_ID + index`).
const NOTIFY_ID: u64 = 5000;
/// Context menu items of a place in the sidebar or the Locations page.
const CTX_REMOVE: u32 = 1;
const CTX_NOTIFY: u32 = 2;
/// The Notifications switch's width (points) between the temperature and the remove button.
const NOTIFY_W: f32 = 64.0;
/// Extent of the two compact weather dials (not UI spacing).
const DIAL_EXTENT_PT: f32 = 32.0;
/// A finger must move this far (points) on a sidebar row before the row is dragged instead of tapped.
const DRAG_START_PT: f32 = 6.0;
/// The leading glyph slot of a sidebar row: its centre, from the row's left edge and middle (points). The list
/// reserves the slot with an invisible dot; the weather glyph is drawn over it.
const SIDE_GLYPH_X: f32 = 28.0;
const SIDE_GLYPH_R: f32 = 9.0;

/// The translucent scrim under the forecast cards: black at 25% over the sky artwork (a fixed colour, not chrome).
const SKY_SCRIM: u32 = 64 << 24;

/// A premultiplied colour at `alpha` of its opacity (white text and plates over the sky).
fn fade(c: u32, alpha: u8) -> u32 {
    let m = |sh: u32| (((c >> sh) & 0xff) * alpha as u32 / 255) << sh;
    m(0) | m(8) | m(16) | m(24)
}
const MARGIN: f32 = 20.0;
/// How often a fetch in flight is looked for, in milliseconds.
const FETCH_POLL_MS: f64 = 120.0;

fn places_field_top_pt() -> f32 { nav_bottom_pt() + 54.0 }
fn places_list_top_pt() -> f32 { nav_bottom_pt() + 100.0 }
fn stocks_field_top_pt() -> f32 { nav_bottom_pt() + 10.0 }
fn stocks_list_top_pt() -> f32 { nav_bottom_pt() + 58.0 }
const FIELD_H: f32 = 36.0;
const ROW: f32 = 56.0;
const HEAD: f32 = 28.0;
const QUOTE_ROW: f32 = 60.0;
const REMOVE_W: f32 = 48.0;
/// The Next Hour card (minute-by-minute rain) above the hourly strip.
const NEXT_TOP: f32 = 296.0;
const NEXT_H: f32 = 100.0;
const HOURLY_TOP: f32 = NEXT_TOP + NEXT_H + 12.0;
const HOURLY_H: f32 = 136.0;
const HOUR_W: f32 = 60.0;
const DAILY_TOP: f32 = HOURLY_TOP + HOURLY_H + 12.0;
const DAY_ROW: f32 = 44.0;
const DAILY_H: f32 = 28.0 + 10.0 * DAY_ROW + 8.0;
const DETAILS_TOP: f32 = DAILY_TOP + DAILY_H + 16.0;
const TILE_H: f32 = 104.0;
const TILE_GAP: f32 = 12.0;
/// Detail tiles in two columns: UV, sunrise, wind, humidity, feels like, precipitation, visibility, pressure,
/// averages and the moon.
const TILE_ROWS: f32 = 5.0;
const DETAILS_H: f32 = TILE_ROWS * TILE_H + (TILE_ROWS - 1.0) * TILE_GAP;
/// A tile at least this wide (points) also draws the sun's day arc or the wind dial.
const TILE_GRAPHIC_MIN_W: f32 = 260.0;
/// The room (points) the sun arc or the wind dial takes at the right of its tile.
const TILE_GRAPHIC_W: f32 = 100.0;
/// The air quality card, below the detail tiles.
const AIR_TOP: f32 = DETAILS_TOP + DETAILS_H + 16.0;
const AIR_H: f32 = 132.0;
/// The temperature chart card, below the air quality card: its plot area and the axis labels' room, in points.
const CHART_TOP: f32 = AIR_TOP + AIR_H + 16.0;
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
const FOOTER_TOP: f32 = MAP_TOP + MAP_H + 16.0;
const FOOTER_LINE: f32 = 18.0;
const CONTENT_H: f32 = FOOTER_TOP + 2.0 * FOOTER_LINE + 16.0;
/// My Location: its row id (sidebar and controls; no saved place gets it), how long an Allow sheet is waited for (ms)
/// and how often the permission is read meanwhile (ms).
const HERE_ROW_ID: u32 = 99;
const ALLOW_WAIT_MS: f64 = 90_000.0;
const STATUS_POLL_MS: f64 = 1_500.0;
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
    Share,
    /// the Notifications switch of a place on the Locations page
    Notify(usize),
    /// the My Location entry
    Here,
}

enum Done {
    Forecast { key: String, result: Result<Forecast, WeatherError> },
    /// a place named by a `weather:` link, looked up by name
    #[allow(dead_code)]
    Linked { name: String, result: Result<Vec<Location>, String> },
    Places { query: String, result: Result<Vec<Location>, String> },
    Quotes(Result<Vec<Quote>, String>),
}

enum Row {
    Head(String),
    /// the My Location entry, listed above the saved places
    Here,
    Place(usize),
    Result(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideTarget {
    Search,
    Place(usize),
    Here,
}

/// What a pending call to the location or privacy service was for.
#[derive(Clone, Copy, PartialEq)]
enum LocCall {
    /// the permission request: its reply says whether to show the Allow sheet, go on or stop
    Authorize,
    /// a status, a position or a start or stop: the location manager reads the reply
    Other,
}

/// What the location manager reported while it handled one reply or event.
#[derive(Default)]
struct LocSink {
    fixes: Vec<Fix>,
    errors: Vec<LocationError>,
    statuses: Vec<AuthorizationStatus>,
}

impl Delegate for LocSink {
    fn did_update_locations(&mut self, locations: &[Fix]) {
        self.fixes.extend_from_slice(locations);
    }
    fn did_fail_with_error(&mut self, error: LocationError) {
        self.errors.push(error);
    }
    fn did_change_authorization(&mut self, status: AuthorizationStatus) {
        self.statuses.push(status);
    }
}

/// What the My Location entry says before it has a fix: a tablet taps it, a desktop clicks it.
fn here_prompt(form: bifrost_app::form_factor::FormFactor) -> &'static str {
    if form.is_tablet() { "Tap to find this place" } else { "Click to find this place" }
}

/// The My Location place for a fix: its coordinates rounded to two decimals (about 1 km) and the name "My Location".
/// Open-Meteo has no reverse lookup, so the coordinates are the only label.
fn here_place(fix: &Fix) -> Location {
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let (lat, lon) = (round(fix.coordinate.latitude), round(fix.coordinate.longitude));
    Location { name: "My Location".into(), region: format!("{lat:.2}, {lon:.2}"), lat, lon }
}

/// A finger that went down on a sidebar place row: a tap, or after `DRAG_START_PT` a drag to reorder.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SideDrag {
    from: usize,
    start_y: f32,
    /// where the finger is now (pixels)
    y: f32,
    active: bool,
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
    side_drag: Option<SideDrag>,
    /// the place a context menu was opened for
    ctx_place: Option<usize>,
    h: i32,
    store: Store,
    fetcher: Arc<dyn Fetch>,
    /// Finished fetches, filled on the global queue and applied by `pump` on the app loop.
    done: Arc<Mutex<Vec<Done>>>,
    places: Vec<Location>,
    selected: usize,
    /// location: the manager, its pending calls, the My Location place once the shell gave a fix, and the error that
    /// last stopped it (`loc_error`)
    loc: LocationManager,
    loc_calls: Vec<(u32, LocCall)>,
    my_location: Option<Location>,
    loc_error: Option<LocationError>,
    /// the person picked My Location: show it when a fix arrives (`here_pick`), and ask for location now (`locate_wanted`)
    here_pick: bool,
    locate_wanted: bool,
    /// My Location is the selected place (only with a fix)
    here_selected: bool,
    /// when an Allow sheet stops being waited for (ms), and when its answer is read next (ms)
    allow_until: Option<f64>,
    next_status_poll: f64,
    units: Units,
    forecasts: HashMap<String, Forecast>,
    /// when each place was last asked for (ms), and the asks in flight
    tried: HashMap<String, f64>,
    pending: HashSet<String>,
    error: Option<WeatherError>,
    /// whether the network path lets the app reach the forecast service (true until the shell says otherwise)
    online: bool,
    /// place keys with the rain Notifications switch on, and the rain spells already announced
    notify_on: HashSet<String>,
    rain: RainWatch,
    /// notifications waiting for the app to post them (`take_notices`)
    notices: Vec<Notice>,
    /// the Share button was pressed (`take_share`)
    share_pending: bool,
    /// a place a `weather:` link asked for, selected on the next tick
    pending_open: Option<Location>,
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
        let notify_on = notify::load(store.dir());
        Weather {
            w,
            full_w: w,
            side: 0.0,
            pt_px: ui.pt(1.0),
            side_query: SearchQuery::new(),
            side_pressed: None,
            side_drag: None,
            ctx_place: None,
            h,
            watch: store.load_watchlist(),
            selected: selected.min(places.len() - 1),
            store,
            fetcher,
            done: Arc::new(Mutex::new(Vec::new())),
            places,
            loc: LocationManager::new(),
            loc_calls: Vec::new(),
            my_location: None,
            loc_error: None,
            here_pick: false,
            locate_wanted: false,
            here_selected: false,
            allow_until: None,
            next_status_poll: 0.0,
            units,
            forecasts,
            tried: HashMap::new(),
            pending: HashSet::new(),
            error: None,
            online: true,
            notify_on,
            rain: RainWatch::default(),
            notices: vec![],
            share_pending: false,
            pending_open: None,
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

    /// The sidebar's rows: the place, its sky in words under the name, and its temperature at the right. The slot
    /// before the name is reserved by an invisible dot; `draw_sidebar` draws the sky glyph in it. A place with no
    /// forecast yet shows its region and no temperature.
    fn sidebar(&self) -> SourceList {
        let mut items: Vec<SidebarItem> = vec![];
        if self.has_here_row() {
            let hint = self.here_hint();
            let row = SidebarItem::row(HERE_ROW_ID, "My Location").with_dot(0);
            items.push(match self.my_location.as_ref().and_then(|p| self.forecasts.get(&p.key())) {
                Some(f) => row.with_subtitle(Sky::from_code(f.current.code).label()).with_trailing(&self.units.degrees(f.current.temp)),
                None => row.with_subtitle(&hint),
            });
        }
        let places = self.places.iter().enumerate().map(|(i, p)| {
            let row = SidebarItem::row(PLACE_ID + i as u32, &p.name).with_dot(0);
            match self.forecasts.get(&p.key()) {
                Some(f) => row.with_subtitle(Sky::from_code(f.current.code).label()).with_trailing(&self.units.degrees(f.current.temp)),
                None => row.with_subtitle(p.subtitle()),
            }
        });
        items.extend(places);
        let mut list = SourceList::new(items, SIDEBAR_ROW_PT, SIDEBAR_ROW_PT);
        let selected = if self.here_shown() { HERE_ROW_ID } else { PLACE_ID + self.selected as u32 };
        list.set_selected((self.screen != Screen::Stocks).then_some(selected));
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
        self.sidebar().hit(self.side_list(), 0.0, x, y).map(|id| if id == HERE_ROW_ID { SideTarget::Here } else { SideTarget::Place((id - PLACE_ID) as usize) })
    }

    /// The detail pane's toolbar: Locations (only without the sidebar) and Share on the main page, Done on
    /// Locations, Back on Stocks. The main page has no Stocks button: the watchlist belongs to the Stocks app, so
    /// Weather's own controls stay about the forecast (the watchlist page and its saved symbols remain for now).
    fn bar(&self) -> Toolbar {
        let mut items = vec![];
        match self.screen {
            Screen::Main => {
                if self.side == 0.0 {
                    items.push(ToolbarItem::icon(BAR_LOCATIONS, "Locations", "list.bullet"));
                }
                items.push(ToolbarItem::flexible_space());
                items.push(ToolbarItem::icon(BAR_SHARE, "Share", "square.and.arrow.up").enabled(self.shareable()));
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
        let o = self.here_offset();
        let here = self.has_here_row().then(|| Node {
            id: HERE_ROW_ID as u64,
            role: Role::Row,
            label: "My Location".into(),
            value: if list.selected() == Some(HERE_ROW_ID) { "selected".into() } else { String::new() },
            bounds: list.item_rect(0, bounds, 0.0),
            enabled: true,
        });
        let places = self.places.iter().enumerate().map(|(i, p)| {
            let id = PLACE_ID + i as u32;
            Node {
                id: id as u64,
                role: Role::Row,
                label: p.name.clone(),
                value: if list.selected() == Some(id) { "selected".into() } else { String::new() },
                bounds: list.item_rect(i + o, bounds, 0.0),
                enabled: true,
            }
        });
        here.into_iter().chain(places).collect()
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
                        Row::Here => {
                            let value = if self.here_shown() { "selected".to_string() } else { String::new() };
                            nodes.push(node(HERE_ROW_ID as u64, Role::Button, "My Location".into(), value, rect(x, y, card_w, rh - 4.0)));
                        }
                        Row::Place(i) => {
                            let name = self.places[i].name.clone();
                            let value = if i == self.selected && !self.here_shown() { "selected".to_string() } else { String::new() };
                            nodes.push(node(PLACE_ROW_ID + i as u64, Role::Button, name.clone(), value, rect(x, y, card_w, rh - 4.0)));
                            let remove_w = if self.places.len() > 1 { REMOVE_W } else { 0.0 };
                            let on = self.notify_on.contains(&self.places[i].key());
                            let state = if on { "on" } else { "off" };
                            nodes.push(node(NOTIFY_ID + i as u64, Role::Button, format!("Notifications for {name}"), state.to_string(), rect(x + card_w - remove_w - NOTIFY_W, y, NOTIFY_W, rh - 4.0)));
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
            _ if id == BAR_SHARE as u64 => Target::Share,
            _ if id == BAR_DONE as u64 || id == BAR_BACK as u64 => Target::Back,
            _ if id == HERE_ROW_ID as u64 => Target::Here,
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
            _ if id >= NOTIFY_ID => Target::Notify((id - NOTIFY_ID) as usize),
            _ if id >= QUOTE_REMOVE_ID => Target::RemoveQuote((id - QUOTE_REMOVE_ID) as usize),
            _ if id >= RESULT_ID => Target::Result((id - RESULT_ID) as usize),
            _ if id >= REMOVE_ID => Target::Remove((id - REMOVE_ID) as usize),
            _ if id >= PLACE_ROW_ID => Target::Place((id - PLACE_ROW_ID) as usize),
            _ if id >= PLACE_ID as u64 => Target::Place((id - PLACE_ID as u64) as usize),
            _ => return false,
        };
        if let Target::Place(i) | Target::Notify(i) = target {
            if i >= self.places.len() {
                return false;
            }
        }
        self.activate(ui, target, at, t);
        true
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        let p = ui.pt(1.0);
        let pressed = match self.side_pressed {
            Some(SideTarget::Place(i)) => Some(PLACE_ID + i as u32),
            Some(SideTarget::Here) => Some(HERE_ROW_ID),
            _ => None,
        };
        widgets::source_list(s, ui, KRect { x: 0.0, y: 0.0, w: self.side / p, h: self.h as f32 / p }, &SourceList::new(vec![], SIDEBAR_ROW_PT, SIDEBAR_ROW_PT), 0.0, None);
        widgets::search_field(s, ui, &self.side_query, self.side_search(), "Search", false);
        let (list, bounds) = (self.sidebar(), self.side_list());
        widgets::source_list(s, ui, bounds, &list, 0.0, pressed);
        // the sky glyph of each place in the slot its row reserves (My Location first, when listed)
        let o = self.here_offset();
        let here = self.my_location.as_ref().filter(|_| o == 1).map(|p| (0usize, p));
        let rows = here.into_iter().chain(self.places.iter().enumerate().map(|(i, p)| (i + o, p)));
        for (row, place) in rows {
            let Some(f) = self.forecasts.get(&place.key()) else { continue };
            let r = list.item_rect(row, bounds, 0.0);
            if r.y + r.h > bounds.y + bounds.h {
                continue;
            }
            glyph::draw(s, Sky::from_code(f.current.code), f.current.day, (ui.pt(r.x + SIDE_GLYPH_X), ui.pt(r.y + r.h / 2.0)), ui.pt(SIDE_GLYPH_R));
        }
        // a row being dragged: a line where it will be dropped
        if let Some(drag) = self.side_drag.filter(|d| d.active) {
            let to = insights::drop_index(&self.side_row_spans(), drag.y / p);
            let r = list.item_rect(to + self.here_offset(), bounds, 0.0);
            let y = if to > drag.from { r.y + r.h } else { r.y };
            s.fill_rect(ui.pt(r.x + SPACE_M) as usize, (ui.pt(y) as usize).saturating_sub(1), ui.pt(r.w - 2.0 * SPACE_M).max(0.0) as usize, 2, LINK);
        }
        s.fill_rect((self.side - 1.0) as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
    }

    /// The `(top, height)` in points of each place's sidebar row.
    fn side_row_spans(&self) -> Vec<(f32, f32)> {
        let (list, bounds) = (self.sidebar(), self.side_list());
        let o = self.here_offset();
        (0..self.places.len()).map(|i| list.item_rect(i + o, bounds, 0.0)).map(|r| (r.y, r.h)).collect()
    }

    fn draw_bar(&mut self, s: &mut Surface, ui: &Ui) {
        if self.screen != Screen::Main {
            // an opaque band with a hairline under it (E2E W4-13): the material band asked for its rectangle in this
            // pane's coordinates, which the shell reads as the window's, so the window behind showed through the bar
            let band = ui.pt(UNIFIED_BAR_PT).round() as usize;
            s.fill_rect(0, 0, s.w, band.min(s.h), CONTENT_BG.now());
            s.fill_rect(0, band.saturating_sub(1).min(s.h), s.w, 1, SEPARATOR_C.now());
            let title = if self.screen == Screen::Places { "Locations" } else { "Stocks" };
            self.put(s, ui, title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), Align::Centre, self.w as f32 / 2.0, ui.pt(UNIFIED_BAR_PT / 2.0));
        }
        let pressed = match self.pressed {
            Some(Target::Places) => Some(BAR_LOCATIONS),
            Some(Target::Stocks) => Some(BAR_STOCKS),
            Some(Target::Share) => Some(BAR_SHARE),
            Some(Target::Back) => Some(if self.screen == Screen::Places { BAR_DONE } else { BAR_BACK }),
            _ => None,
        };
        widgets::toolbar_in(s, ui, self.bar_bounds(ui), &self.bar(), pressed, self.side / ui.pt(1.0));
    }

    pub fn keyboard_shown(&self) -> bool {
        self.kb_on
    }

    /// The state dump the tests check (`weather_*` keys); `tick` already reports when a frame is needed.
    #[cfg(test)]
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
        if let (true, Some(here)) = (self.here_shown(), self.my_location.as_ref()) {
            return here;
        }
        &self.places[self.selected.min(self.places.len() - 1)]
    }

    /// Whether the My Location row is listed: not while location is off for Weather (denied, restricted or refused).
    fn has_here_row(&self) -> bool {
        let off = matches!(self.loc.authorization_status(), AuthorizationStatus::Denied | AuthorizationStatus::Restricted)
            || matches!(self.loc_error, Some(LocationError::Denied | LocationError::PromptDeclined));
        !off
    }

    /// The rows the My Location entry takes above the saved places: 1 when it is listed, else 0.
    fn here_offset(&self) -> usize {
        self.has_here_row() as usize
    }

    /// My Location is the selected place (it has a fix and the person chose it).
    fn here_shown(&self) -> bool {
        self.here_selected && self.my_location.is_some()
    }

    /// The My Location entry's second line: its rounded coordinates once a fix came, else what to do.
    fn here_hint(&self) -> String {
        match &self.my_location {
            Some(p) => p.region.clone(),
            None => here_prompt(bifrost_app::form_factor::FormFactor::current()).into(),
        }
    }

    fn save_summary(&mut self) {
        if let Some(s) = self.summary() {
            self.store.save_summary(&s);
            // the next hours go beside it, for Notification Center's Weather card
            if let Some(f) = self.forecasts.get(&self.place().key()) {
                self.store.save_glance(&s.place, f.glance());
            }
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
                            if let Some(place) = self.places.iter().find(|p| p.key() == key) {
                                let enabled = self.notify_on.contains(&key);
                                if let Some(notice) = self.rain.check(&key, &place.name, &f.minutes, enabled) {
                                    self.notices.push(notice);
                                }
                                let tomorrow = f.days.get(1).map_or(0, |d| d.precip_prob);
                                if let Some(notice) = self.rain.check_tomorrow(&key, &place.name, &f.date, tomorrow, enabled) {
                                    self.notices.push(notice);
                                }
                            }
                            self.forecasts.insert(key.clone(), f);
                            self.error = None;
                            if key == self.place().key() {
                                self.save_summary();
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Done::Linked { result, .. } => {
                    // the first match of the name in a `weather:` link is shown on the next tick
                    if let Ok(list) = result {
                        self.pending_open = list.into_iter().next();
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
        let mut all: Vec<Location> = self.my_location.iter().cloned().collect();
        all.extend(self.places.iter().cloned());
        for p in &all {
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
        if self.has_here_row() {
            rows.push((Row::Here, top, ROW));
            top += ROW;
        }
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
        rows.push((Row::Head(insights::location_note(self.loc.authorization_status(), self.loc_error)), top, HEAD));
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
        let columns = self.forecasts.get(&self.place().key()).map_or(0, |f| insights::strip_columns(&f.hours).len());
        self.hscroll.set_max((columns as f32 * ui.pt(HOUR_W) + ui.pt(SPACE_L) - self.card_w(ui)).max(0.0));
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
                BAR_SHARE => self.shareable().then_some(Target::Share),
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
                    Row::Place(i) if x >= self.notify_left(ui) && x < self.notify_left(ui) + ui.pt(NOTIFY_W) => Some(Target::Notify(i)),
                    Row::Place(i) => Some(Target::Place(i)),
                    Row::Here => Some(Target::Here),
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
            Target::Here => {
                // the fix shows My Location when it comes (`set_fix`), after the Allow sheet if one is needed
                self.here_pick = true;
                self.locate_wanted = true;
            }
            Target::Place(i) => {
                self.here_selected = false;
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
                    if self.notify_on.remove(&gone.key()) {
                        notify::save(self.store.dir(), &self.notify_on);
                    }
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
            Target::Share => self.share_pending = self.shareable(),
            Target::Notify(i) => self.toggle_notifications(i),
            Target::Hourly => {}
        }
    }

    /// Whether the Share button has a forecast to send.
    fn shareable(&self) -> bool {
        self.forecasts.contains_key(&self.place().key())
    }

    /// True once after the Share button was pressed with a forecast to send.
    pub fn take_share(&mut self) -> bool {
        std::mem::take(&mut self.share_pending)
    }

    /// The dialog of the current-conditions intent for the selected place; None until its forecast has loaded.
    pub fn intent_dialog(&self) -> Option<String> {
        let place = self.place();
        Some(control::intent_dialog(&place.name, self.forecasts.get(&place.key())?, self.units))
    }

    /// The text to share for the selected place (see `control::share_text`).
    pub fn share_text(&self) -> Option<String> {
        let place = self.place();
        Some(control::share_text(&place.name, self.forecasts.get(&place.key())?, self.units))
    }

    /// Where the Share button is, in window points, for the share menu to open under it.
    pub fn share_anchor(&self, ui: &Ui) -> KRect {
        let dx = self.side / ui.pt(1.0);
        self.bar_layout(ui).rect_of(BAR_SHARE).map_or(KRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, |r| KRect { x: r.x + dx, ..r })
    }

    /// Notifications waiting to be posted; the app posts them to the shell.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// Switches the rain Notifications of place `i` on or off and saves the choice.
    fn toggle_notifications(&mut self, i: usize) {
        let Some(key) = self.places.get(i).map(Location::key) else { return };
        if !self.notify_on.remove(&key) {
            self.notify_on.insert(key);
        }
        notify::save(self.store.dir(), &self.notify_on);
    }

    /// The left edge (pixels) of the Notifications switch on a Locations row.
    fn notify_left(&self, ui: &Ui) -> f32 {
        let remove = if self.places.len() > 1 { ui.pt(REMOVE_W) } else { 0.0 };
        self.w as f32 - ui.pt(MARGIN) - remove - ui.pt(NOTIFY_W)
    }

    /// Moves the place at `from` to position `to` in the list, keeps the same place selected and saves the order.
    fn reorder_places(&mut self, from: usize, to: usize) -> bool {
        if !insights::move_item(&mut self.places, from, to) {
            return false;
        }
        self.selected = insights::follow_selection(self.selected, from, to);
        self.store.save_locations(&self.places);
        self.store.save_settings(self.units, self.selected);
        self.dirty = true;
        true
    }

    /// The place under a secondary click at window pixel `(x, y)`: a sidebar row, or a row of the Locations page.
    fn place_at(&self, ui: &Ui, x: f32, y: f32) -> Option<usize> {
        let found = if x < self.side {
            match self.side_hit(ui, x, y) {
                Some(SideTarget::Place(i)) => Some(i),
                _ => None,
            }
        } else if self.screen == Screen::Places {
            match self.hit(ui, x - self.side, y) {
                Some(Target::Place(i) | Target::Remove(i) | Target::Notify(i)) => Some(i),
                _ => None,
            }
        } else {
            None
        };
        found.filter(|i| *i < self.places.len())
    }

    /// The context menu of the place at `(x, y)`: its Notifications switch and Remove. Empty when no place is there.
    pub fn context_items(&mut self, ui: &Ui, x: f32, y: f32) -> Vec<ContextItem> {
        self.ctx_place = self.place_at(ui, x, y);
        let Some(i) = self.ctx_place else { return vec![] };
        let label = if self.notify_on.contains(&self.places[i].key()) { "Turn Off Notifications" } else { "Turn On Notifications" };
        vec![
            ContextItem { id: CTX_NOTIFY, label: label.to_string(), enabled: true },
            ContextItem { id: CTX_REMOVE, label: "Remove".to_string(), enabled: self.places.len() > 1 },
        ]
    }

    /// The user chose item `id` of the menu `context_items` returned. True when a frame is needed.
    pub fn context_action(&mut self, ui: &Ui, id: u32, t: f64) -> bool {
        let Some(i) = self.ctx_place.take() else { return false };
        match id {
            CTX_REMOVE => self.activate(ui, Target::Remove(i), 0.0, t),
            CTX_NOTIFY => self.toggle_notifications(i),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// Shows `place`: selects it when it is saved, and adds it first when there is room. A full list leaves the
    /// selection alone.
    fn show_place(&mut self, ui: &Ui, place: Location, t: f64) {
        let index = match self.places.iter().position(|q| q.key() == place.key()) {
            Some(i) => i,
            None if self.places.len() < data::MAX_LOCATIONS => {
                self.places.push(place);
                self.store.save_locations(&self.places);
                self.places.len() - 1
            }
            None => return,
        };
        self.activate(ui, Target::Place(index), 0.0, t);
    }

    /// A `weather:` link asks for a place (see `insights::place_from_url`). A saved place of that name, or a link
    /// with a position, is shown at once; any other name is looked up first and the first match shown. True when
    /// the link names a place.
    pub fn open_link(&mut self, ui: &Ui, url: &str, t: f64) -> bool {
        let Some(link) = insights::place_from_url(url) else { return false };
        self.maintain_at = f64::NEG_INFINITY;
        self.dirty = true;
        let wanted = link.name.to_lowercase();
        if let Some(saved) = self.places.iter().find(|p| p.name.to_lowercase() == wanted).cloned() {
            self.show_place(ui, saved, t);
        } else if let Some((lat, lon)) = link.coordinate {
            self.show_place(ui, Location { name: link.name, region: String::new(), lat, lon }, t);
        } else {
            let net = self.fetcher.clone();
            let name = link.name;
            self.spawn(move || Done::Linked { result: data::search_places(&name, &*net).map_err(|_| "Search unavailable".to_string()), name });
        }
        true
    }

    /// Answers a control method (`control::METHODS`) from the forecasts the app holds. `place` in `params` names a
    /// saved place; without it the selected place answers.
    pub fn rpc(&self, method: &str, params: &Json) -> Result<Json, RpcError> {
        if method == "weather.places" {
            return Ok(Json::Arr(self.places.iter().map(|p| Json::Str(p.name.clone())).collect()));
        }
        if !control::METHODS.contains(&method) {
            return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}")));
        }
        let named = params.str_field("place").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let place = match &named {
            Some(name) => self
                .places
                .iter()
                .find(|p| p.name.to_lowercase() == name.to_lowercase())
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("no saved place named {name}")))?,
            None => self.place(),
        };
        let forecast = self.forecasts.get(&place.key()).ok_or_else(|| RpcError::new(INTERNAL_ERROR, format!("no forecast for {} yet", place.name)))?;
        Ok(match method {
            "weather.current" => control::current_json(&place.name, forecast, self.units),
            _ => control::forecast_json(&place.name, forecast, self.units),
        })
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.maintain_at = f64::NEG_INFINITY;
        self.dirty = true;
        // Delete with no field being typed in removes the selected place; so does Backspace on the forecast page
        if !self.field.focused && !self.here_shown() && self.screen != Screen::Stocks && (out == Out::Named("Delete") || (self.screen == Screen::Main && out == Out::Backspace)) {
            self.activate(ui, Target::Remove(self.selected), 0.0, t);
            return;
        }
        // Return in Locations searches for what the field holds, focused or not (E2E W4-12)
        if self.screen == Screen::Places && out == Out::Enter && !self.field.is_empty() {
            self.field.focused = false;
            self.kb_on = false;
            self.search();
            self.fit_scroll(ui);
            return;
        }
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
            // a press on a row can become a drag that reorders the places
            self.side_drag = match self.side_pressed {
                Some(SideTarget::Place(from)) if self.places.len() > 1 => Some(SideDrag { from, start_y: y, y, active: false }),
                _ => None,
            };
            return;
        }
        if let Some(pressed) = self.side_pressed {
            match ev {
                Event::Move(..) => {
                    if let Some(d) = &mut self.side_drag {
                        d.y = y;
                        d.active |= (y - d.start_y).abs() > ui.pt(DRAG_START_PT);
                    }
                }
                Event::Up(..) => {
                    self.side_pressed = None;
                    if let Some(d) = self.side_drag.take().filter(|d| d.active) {
                        let to = insights::drop_index(&self.side_row_spans(), y / ui.pt(1.0));
                        self.reorder_places(d.from, to);
                    } else if self.side_hit(ui, x, y) == Some(pressed) {
                        match pressed {
                            SideTarget::Search => {
                                if self.screen != Screen::Places {
                                    self.activate(ui, Target::Places, 0.0, t);
                                }
                                self.activate(ui, Target::Field, self.field_rect(ui).0 + ui.pt(SPACE_M), t);
                            }
                            SideTarget::Place(i) => self.activate(ui, Target::Place(i), 0.0, t),
                            SideTarget::Here => self.activate(ui, Target::Here, 0.0, t),
                        }
                    }
                }
                _ => {}
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

    /// The scroll wheel at (`x`, `y`) in window pixels (`scrollWheel`, E2E W4-11): over the hourly strip a sideways
    /// wheel scrolls the hours, anywhere else in the detail pane the wheel scrolls the page (forecast, Locations,
    /// Stocks). `dx` and `dy` are the content's travel in pixels, positive `dy` moving it down. True when it moved.
    pub fn wheel(&mut self, ui: &Ui, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        self.relayout(ui);
        if x < self.side || self.scrolling.is_some() {
            return false;
        }
        let x = x - self.side;
        self.fit_scroll(ui);
        let (hx, hy, hw, hh) = self.hourly_rect(ui);
        let over_hours = self.screen == Screen::Main && x >= hx && x < hx + hw && y >= hy && y < hy + hh;
        let moved = if over_hours && dx.abs() > dy.abs() {
            self.hscroll.wheel(dx)
        } else if y >= ui.pt(self.list_top()) && y < self.bottom(ui) {
            self.scroll.wheel(dy)
        } else {
            false
        };
        self.dirty |= moved;
        moved
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.relayout(ui);
        self.pump();
        if let Some(place) = self.pending_open.take() {
            self.show_place(ui, place, t);
        }
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
        if let Some(until) = self.allow_until {
            let poll = self.next_status_poll.min(until);
            due = Some(due.map_or(poll, |d| d.min(poll)));
        }
        // a search, forecast or quote fetch in flight: look for its result soon, so it shows when it lands and not on
        // the next click (E2E W4-12; the main-queue wake from the worker does not always reach the run loop)
        if self.fetching() {
            let poll = t + FETCH_POLL_MS;
            due = Some(due.map_or(poll, |d| d.min(poll)));
        }
        due
    }

    /// Whether a fetch runs on the utility queue whose result has not been applied yet.
    fn fetching(&self) -> bool {
        self.searching || self.quotes_pending || !self.pending.is_empty() || !self.done.lock().map_or(true, |d| d.is_empty())
    }

    // ---- My Location: the shell's location service, asked only once the person picks My Location ----

    /// Reads the location permission once at start. This asks nothing: no Allow sheet is shown, and an earlier answer
    /// lists the entry.
    pub fn read_location_status(&mut self, ctx: &Ctx) {
        let id = self.loc.refresh_authorization(ctx);
        self.loc_calls.push((id, LocCall::Other));
    }

    /// Acts on the pick of My Location (called from the app loop, which has the context): the Allow sheet is shown when
    /// the person has not answered yet, and a position is asked for once location is allowed.
    pub fn locate(&mut self, ctx: &Ctx) {
        if !std::mem::take(&mut self.locate_wanted) {
            return;
        }
        if self.loc.authorization_status().is_authorized() {
            self.ask_fix(ctx);
        } else {
            let id = self.loc.request_when_in_use_authorization(ctx);
            self.loc_calls.push((id, LocCall::Authorize));
        }
    }

    /// One position now, and every change after it (the place follows the shell's fixes while the window lives).
    fn ask_fix(&mut self, ctx: &Ctx) {
        let id = self.loc.request_location(ctx);
        self.loc_calls.push((id, LocCall::Other));
        if !self.loc.is_updating() {
            let id = self.loc.start_updating_location(ctx);
            self.loc_calls.push((id, LocCall::Other));
        }
    }

    /// Location is allowed: a position is asked for when the person picked My Location.
    fn authorized(&mut self, ctx: &Ctx) {
        self.loc_error = None;
        self.allow_until = None;
        if self.here_pick {
            self.ask_fix(ctx);
        }
    }

    /// Location is off or was refused: My Location goes away with its fix, and the reason shows in the note.
    fn stop_here(&mut self, why: LocationError) {
        self.loc_error = Some(why);
        self.allow_until = None;
        self.here_pick = false;
        self.locate_wanted = false;
        self.my_location = None;
        self.here_selected = false;
    }

    /// A fix arrived: My Location takes it, and when the person picked it the window shows it.
    fn set_fix(&mut self, ctx: &Ctx, fix: &Fix) {
        self.my_location = Some(here_place(fix));
        self.loc_error = None;
        self.maintain_at = f64::NEG_INFINITY;
        if self.here_pick {
            self.here_pick = false;
            self.here_selected = true;
            self.hscroll.jump_top();
            let t = bifrost_app::proto::sys::now_ms();
            self.go(&ctx.ui, Screen::Main, t);
            self.save_summary();
        } else if self.here_selected {
            self.save_summary();
        }
    }

    /// The Allow sheet's answer is read from the permission while it is waited for (`next_wake` schedules it).
    pub fn location_tick(&mut self, ctx: &Ctx, t: f64) {
        let Some(until) = self.allow_until else { return };
        if t >= until {
            self.stop_here(LocationError::PromptDeclined);
            self.dirty = true;
        } else if t >= self.next_status_poll {
            self.next_status_poll = t + STATUS_POLL_MS;
            let id = self.loc.refresh_authorization(ctx);
            self.loc_calls.push((id, LocCall::Other));
        }
    }

    /// A reply to a call this window made to the location or privacy service. False for any other reply.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(at) = self.loc_calls.iter().position(|(id, _)| *id == request) else { return false };
        let (_, call) = self.loc_calls.remove(at);
        match call {
            LocCall::Authorize => match result.as_ref().ok().and_then(privacy_permissions::decode_location_outcome) {
                Some(RequestOutcome::AlreadyGranted) => self.authorized(ctx),
                Some(RequestOutcome::Prompt) => {
                    // the Allow sheet is up; its answer is read from the permission
                    let now = bifrost_app::proto::sys::now_ms();
                    self.allow_until = Some(now + ALLOW_WAIT_MS);
                    self.next_status_poll = now + STATUS_POLL_MS;
                }
                Some(RequestOutcome::Refused) | None => self.stop_here(LocationError::PromptDeclined),
            },
            LocCall::Other => {
                let mut sink = LocSink::default();
                self.loc.handle_reply(result, &mut sink);
                self.apply_location(ctx, sink);
            }
        }
        self.dirty = true;
        true
    }

    /// A push from the location service (a changed position, a failure). False for any other event.
    pub fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let mut sink = LocSink::default();
        if !self.loc.handle_event(service, event, body, &mut sink) {
            return false;
        }
        self.apply_location(ctx, sink);
        self.dirty = true;
        true
    }

    /// Carries out what the location manager reported: permission changes, failures, then the newest fix.
    fn apply_location(&mut self, ctx: &Ctx, sink: LocSink) {
        for status in sink.statuses {
            if status.is_authorized() {
                self.authorized(ctx);
            } else if matches!(status, AuthorizationStatus::Denied | AuthorizationStatus::Restricted) {
                self.stop_here(LocationError::Denied);
            }
        }
        for error in sink.errors {
            if error.needs_user_action() {
                self.stop_here(error);
            } else {
                self.loc_error = Some(error);
            }
        }
        if let Some(fix) = sink.fixes.last() {
            self.set_fix(ctx, fix);
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
        let rain = rain_blue();
        // next hour: the minute-by-minute rain as one bar per 15-minute step, under a line that says what is coming
        c.fill_rrect(card_x, p(NEXT_TOP), card_w, ui.pt(NEXT_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "NEXT HOUR", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(NEXT_TOP + 16.0));
        let summary = ui.fit(&insights::next_hour_summary(&f.minutes), MAC_BODY_PT, card_w - ui.pt(2.0 * SPACE_L));
        self.put(&mut c, ui, &summary, MAC_BODY_PT, Weight::Medium, WHITE, Align::Left, card_x + ui.pt(SPACE_L), p(NEXT_TOP + 38.0));
        if !f.minutes.is_empty() {
            let area_x = card_x + ui.pt(SPACE_L);
            let slot = (card_w - ui.pt(2.0 * SPACE_L)) / f.minutes.len() as f32;
            let bar_w = (slot - ui.pt(SPACE_S)).max(1.0);
            let (base, tallest) = (p(NEXT_TOP + NEXT_H - 24.0), ui.pt(28.0));
            for (k, mm) in f.minutes.iter().enumerate() {
                let h = (insights::rain_bar(*mm) * tallest).max(ui.pt(SPACE_XXS));
                let color = if *mm >= insights::WET_STEP_MM { rain } else { fade(WHITE, 60) };
                let left = area_x + k as f32 * slot;
                c.fill_rrect(left, base - h, bar_w, h, ui.pt(SPACE_XXS), color);
                let label = if k == 0 { "Now".to_string() } else { format!("{} min", k * 15) };
                self.put(&mut c, ui, &label, MAC_CAPTION1_PT, Weight::Regular, dim, Align::Left, left, base + ui.pt(SPACE_M));
            }
        }
        // hourly strip: drawn into its own image so the columns clip at the card. An hour with a sunrise or sunset
        // is followed by a column for it.
        c.fill_rrect(card_x, p(HOURLY_TOP), card_w, ui.pt(HOURLY_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "HOURLY FORECAST", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(HOURLY_TOP + 16.0));
        let mut strip = Image { w: card_w as usize, h: ui.pt(HOURLY_H - 30.0) as usize, px: vec![0; card_w as usize * ui.pt(HOURLY_H - 30.0) as usize] };
        {
            let mut ss = Surface { w: strip.w, h: strip.h, stride: strip.w, px: &mut strip.px, y0: 0 };
            for (ci, column) in insights::strip_columns(&f.hours).into_iter().enumerate() {
                let mid = ui.pt(SPACE_S) + (ci as f32 + 0.5) * ui.pt(HOUR_W) - self.hscroll.offset();
                if mid < -ui.pt(HOUR_W) || mid > card_w + ui.pt(HOUR_W) {
                    continue;
                }
                match column {
                    Column::Hour(i) => {
                        let hr = &f.hours[i];
                        let label = if i == 0 { "Now".to_string() } else { hr.label.clone() };
                        self.put(&mut ss, ui, &label, MAC_BODY_PT, Weight::Medium, WHITE, Align::Centre, mid, ui.pt(SPACE_M));
                        glyph::draw(&mut ss, Sky::from_code(hr.code), hr.day, (mid, ui.pt(SPACE_HUGE)), ui.pt(SPACE_L));
                        if let Some(chance) = insights::chance_text(hr.precip_prob) {
                            self.put(&mut ss, ui, &chance, MAC_CAPTION1_PT, Weight::Semibold, rain, Align::Centre, mid, ui.pt(64.0));
                        }
                        self.put(&mut ss, ui, &units.degrees(hr.temp), MAC_BODY_PT, Weight::Semibold, WHITE, Align::Centre, mid, ui.pt(88.0));
                    }
                    Column::Sun(i) => {
                        if let Some(event) = &f.hours[i].sun {
                            let name = if event.rise { "Sunrise" } else { "Sunset" };
                            self.put(&mut ss, ui, name, MAC_BODY_PT, Weight::Medium, WHITE, Align::Centre, mid, ui.pt(SPACE_M));
                            glyph::draw(&mut ss, Sky::Clear, true, (mid, ui.pt(SPACE_HUGE)), ui.pt(SPACE_M));
                            self.put(&mut ss, ui, &event.time, MAC_BODY_PT, Weight::Semibold, WHITE, Align::Centre, mid, ui.pt(88.0));
                        }
                    }
                }
            }
        }
        c.blit(&strip, card_x as i32, p(HOURLY_TOP + 30.0) as i32, 255, 255);
        // seven days
        c.fill_rrect(card_x, p(DAILY_TOP), card_w, ui.pt(DAILY_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "10-DAY FORECAST", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(DAILY_TOP + 16.0));
        let lo = f.days.iter().map(|d| d.min).fold(f64::MAX, f64::min);
        let hi = f.days.iter().map(|d| d.max).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(1.0) as f32;
        let (bar_x, bar_w) = (card_x + card_w * 0.36, card_w * 0.48);
        for (i, d) in f.days.iter().enumerate() {
            let mid = p(28.0 + DAILY_TOP + (i as f32 + 0.5) * DAY_ROW);
            if i > 0 {
                c.fill_rrect(card_x + ui.pt(SPACE_L), mid - ui.pt(DAY_ROW / 2.0), card_w - ui.pt(28.0), 1.0, 0.0, fade(LABEL.now(), 40));
            }
            let label = if i == 0 { "Today" } else { d.label.as_str() };
            self.put(&mut c, ui, label, MAC_BODY_PT, Weight::Medium, WHITE, Align::Left, card_x + ui.pt(SPACE_L), mid);
            // the chance of precipitation sits under the glyph, which moves up to make room
            let chance = insights::chance_text(d.precip_prob);
            let glyph_y = if chance.is_some() { mid - ui.pt(5.0) } else { mid };
            glyph::draw(&mut c, Sky::from_code(d.code), true, (card_x + card_w * 0.20, glyph_y), ui.pt(SPACE_M));
            if let Some(chance) = chance {
                self.put(&mut c, ui, &chance, MAC_CAPTION1_PT, Weight::Semibold, rain, Align::Centre, card_x + card_w * 0.20, mid + ui.pt(15.0));
            }
            self.put(&mut c, ui, &units.degrees(d.min), MAC_BODY_PT, Weight::Medium, dim, Align::Right, bar_x - ui.pt(SPACE_M), mid);
            c.fill_rrect(bar_x, mid - ui.pt(SPACE_XXS), bar_w, ui.pt(SPACE_XS), ui.pt(SPACE_XXS), fade(LABEL.now(), 60));
            let (x0, x1) = (((d.min - lo as f64) as f32 / span).clamp(0.0, 1.0), ((d.max - lo as f64) as f32 / span).clamp(0.0, 1.0));
            c.fill_rrect(bar_x + bar_w * x0, mid - ui.pt(SPACE_XXS), (bar_w * (x1 - x0)).max(ui.pt(SPACE_XS)), ui.pt(SPACE_XS), ui.pt(SPACE_XXS), ORANGE);
            if i == 0 {
                let k = ((f.current.temp - lo) as f32 / span).clamp(0.0, 1.0);
                c.fill_rrect(bar_x + bar_w * k - ui.pt(SPACE_XS), mid - ui.pt(SPACE_XS), ui.pt(SPACE_S), ui.pt(SPACE_S), ui.pt(SPACE_XS), LABEL.now());
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
        let next_24h_mm: f64 = f.hours.iter().map(|h| h.precip_mm).sum();
        let snow_today = today.map_or(0.0, |d| d.snow_sum);
        let precip_note = if snow_today >= 0.05 {
            format!("Snow: {} of it", insights::snow_text(snow_today, units))
        } else if next_24h_mm >= 0.05 {
            format!("{} in the next 24 hours", insights::precipitation_text(next_24h_mm, units))
        } else {
            "None expected in the next 24 hours".to_string()
        };
        // the coming days with rain, after the 24 hours the note already gives
        let ahead: Vec<(String, f64)> = f.days.iter().map(|d| (d.label.clone(), d.precip_sum)).collect();
        let precip_note = match insights::rain_ahead_text(&ahead) {
            Some(later) => format!("{precip_note}. {later}"),
            None => precip_note,
        };
        let visibility = f.hours.first().and_then(|h| h.visibility_m);
        let pressure = f.current.pressure_hpa.or_else(|| f.hours.first().and_then(|h| h.pressure_hpa));
        let (average_value, average_note) = insights::average_text(today.map_or(f.current.temp, |d| d.max), f.average_high, units);
        let tiles = [
            ("UV INDEX", format!("{}", uv.round() as i32), uv_note.to_string()),
            ("SUNRISE", sun(today, true), format!("Sunset: {}", sun(today, false))),
            ("WIND", wind, insights::wind_note(f.current.wind_gust_kmh, f.current.wind_dir, units)),
            ("HUMIDITY", format!("{}%", f.current.humidity), insights::humidity_note(f.current.dew_point, units)),
            ("FEELS LIKE", units.degrees(f.current.feels), feels_note.to_string()),
            ("PRECIPITATION", insights::precipitation_text(today.map_or(0.0, |d| d.precip_sum), units), precip_note),
            ("VISIBILITY", visibility.map_or("--".to_string(), |m| insights::visibility_text(m, units)), visibility.map_or("Not reported".to_string(), |m| insights::visibility_note(m).to_string())),
            ("PRESSURE", pressure.map_or("--".to_string(), |h| insights::pressure_text(h, units)), insights::pressure_note(&f.hours)),
            ("AVERAGES", average_value, average_note),
            ("MOON", moon_value, moon_note),
        ];
        let tile_w = (card_w - ui.pt(TILE_GAP)) / 2.0;
        let graphic = tile_w > ui.pt(TILE_GRAPHIC_MIN_W);
        for (i, (name, value, note)) in tiles.iter().enumerate() {
            let tx = card_x + (i % 2) as f32 * (tile_w + ui.pt(TILE_GAP));
            let ty = p(DETAILS_TOP + (i / 2) as f32 * (TILE_H + TILE_GAP));
            c.fill_rrect(tx, ty, tile_w, ui.pt(TILE_H), ui.pt(RADIUS), card);
            self.put(&mut c, ui, name, MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(SPACE_XL));
            self.put(&mut c, ui, value, MAC_LARGE_TITLE_PT, Weight::Medium, WHITE, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(54.0));
            // the sun's day arc and the wind dial take the right of their tile, so the note stops short of them
            let has_graphic = graphic && matches!(*name, "SUNRISE" | "WIND" | "PRESSURE");
            let note_w = tile_w - ui.pt(28.0) - if has_graphic { ui.pt(TILE_GRAPHIC_W) } else { 0.0 };
            let note = ui.fit(note, MAC_FOOTNOTE_PT, note_w);
            self.put(&mut c, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, WHITE, Align::Left, tx + ui.pt(SPACE_L), ty + ui.pt(86.0));
            if has_graphic {
                let centre = (tx + tile_w - ui.pt(TILE_GRAPHIC_W / 2.0), ty);
                match *name {
                    "SUNRISE" => {
                        let (rise, set) = today.map_or((String::new(), String::new()), |d| (d.sunrise.clone(), d.sunset.clone()));
                        draw_sun_arc(&mut c, ui, centre, insights::sun_progress(&f.time, &rise, &set));
                    }
                    "WIND" => {
                        draw_wind_dial(&mut c, ui, centre, f.current.wind_dir);
                        self.put(&mut c, ui, "N", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Centre, centre.0, ty + ui.pt(15.0));
                    }
                    _ => draw_pressure_dial(&mut c, ui, centre, insights::pressure_dial_angle(&f.hours)),
                }
            }
        }
        // air quality: the index and its word, over a scale from good to extremely poor with a marker
        c.fill_rrect(card_x, p(AIR_TOP), card_w, ui.pt(AIR_H), ui.pt(RADIUS), card);
        self.put(&mut c, ui, "AIR QUALITY", MAC_CAPTION1_PT, Weight::Semibold, dim, Align::Left, card_x + ui.pt(SPACE_L), p(AIR_TOP + 16.0));
        match &f.air {
            Some(air) => {
                let value = format!("{}", air.aqi);
                let after = card_x + ui.pt(SPACE_L) + ui.measure(&value, MAC_LARGE_TITLE_PT, Weight::Medium) + ui.pt(SPACE_S);
            let bands = [GREEN, YELLOW, ORANGE, RED, PURPLE, bifrost_app::kit::ui::system::SYSTEM_INDIGO.now()];
                self.put(&mut c, ui, &value, MAC_LARGE_TITLE_PT, Weight::Medium, WHITE, Align::Left, card_x + ui.pt(SPACE_L), p(AIR_TOP + 44.0));
                // the band's colour as a dot before its word
                let dot = ui.pt(SPACE_S);
                c.fill_rrect(after, p(AIR_TOP + 44.0) - dot / 2.0, dot, dot, dot / 2.0, bands[insights::air_band(air.aqi)]);
                self.put(&mut c, ui, air.category(), MAC_TITLE3_PT, Weight::Semibold, WHITE, Align::Left, after + dot + ui.pt(SPACE_S), p(AIR_TOP + 44.0));
                let (pollutant, concentration) = air.main_pollutant();
                let detail = ui.fit(&format!("Main pollutant: {} {:.0} \u{b5}g/m\u{b3}", pollutant.name(), concentration), MAC_FOOTNOTE_PT, card_w - ui.pt(2.0 * SPACE_L));
                self.put(&mut c, ui, &detail, MAC_FOOTNOTE_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(SPACE_L), p(AIR_TOP + 66.0));
                let advice = ui.fit(air.advice(), MAC_FOOTNOTE_PT, card_w - ui.pt(2.0 * SPACE_L));
                self.put(&mut c, ui, &advice, MAC_FOOTNOTE_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(SPACE_L), p(AIR_TOP + 84.0));
                let (bar_x, bar_w, bar_y) = (card_x + ui.pt(SPACE_L), card_w - ui.pt(2.0 * SPACE_L), p(AIR_TOP + AIR_H - 20.0));
                let seg = bar_w / bands.len() as f32;
                for (b, color) in bands.iter().enumerate() {
                    c.fill_rrect(bar_x + b as f32 * seg, bar_y, seg + 1.0, ui.pt(SPACE_XS), 0.0, *color);
                }
                let marker = ui.pt(SPACE_M);
                let mx = bar_x + bar_w * insights::air_position(air.aqi);
                c.fill_rrect(mx - marker / 2.0, bar_y + ui.pt(SPACE_XS) / 2.0 - marker / 2.0, marker, marker, marker / 2.0, LABEL.now());
            }
            None => {
                let text = ui.fit("Air quality is not available for this place.", MAC_BODY_PT, card_w - ui.pt(2.0 * SPACE_L));
                self.put(&mut c, ui, &text, MAC_BODY_PT, Weight::Regular, WHITE, Align::Left, card_x + ui.pt(SPACE_L), p(AIR_TOP + 50.0));
            }
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
                c.fill_rrect(card_x + ui.pt(CHART_AXIS), p(CHART_TOP + y), ui.pt(frame.w as f32), 1.0, 0.0, fade(LABEL.now(), 40));
                self.put(&mut c, ui, &units.degrees(tick), MAC_CAPTION1_PT, Weight::Regular, dim, Align::Right, card_x + ui.pt(CHART_AXIS - SPACE_S), p(CHART_TOP + y));
            }
            // area mark: the fill from the line down to the plot's bottom edge, one column per point
            let line = insights::temperature_line(&plot, &f.hours);
            let base = CHART_PLOT_TOP + CHART_PLOT_H;
            let col = ui.pt(1.0) + 1.0;
            for (x, top) in insights::area_columns(&line, 1.0) {
                let top_px = p(CHART_TOP + top as f32);
                let bottom_px = p(CHART_TOP + base);
                c.fill_rrect(card_x + ui.pt(x as f32), top_px, col, bottom_px - top_px, 0.0, fade(LABEL.now(), 40));
            }
            let d = ui.pt(SPACE_XXS);
            for pair in line.windows(2) {
                let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                let steps = ((bx - ax).abs().max((by - ay).abs()) / 0.5).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    let (x, y) = ((ax + (bx - ax) * t) as f32, (ay + (by - ay) * t) as f32);
                    c.fill_rrect(card_x + ui.pt(x) - d / 2.0, p(CHART_TOP + y) - d / 2.0, d, d, d / 2.0, LABEL.now());
                }
            }
            for (x, y) in insights::temperature_points(&f.hours) {
                let (px, py) = plot.position(x, y);
                let dot = ui.pt(5.0);
                c.fill_rrect(card_x + ui.pt(px as f32) - dot / 2.0, p(CHART_TOP + py as f32) - dot / 2.0, dot, dot, dot / 2.0, LABEL.now());
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
            c.fill_rrect(ax, ay, aw, ah, ui.pt(SPACE_S), fade(LABEL.now(), 24));
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
            c.fill_rrect(ax + aw / 2.0 - mark / 2.0, ay + ah / 2.0 - mark / 2.0, mark, mark, mark / 2.0, LABEL.now());
            if let Some(bar) = view.camera().scale_bar(insights::SCALE_MAX_PT) {
                let bw = ui.pt(bar.width_px as f32);
                let (bx, by) = (ax + aw - ui.pt(SPACE_S) - bw, ay + ah - ui.pt(SPACE_M));
                c.fill_rrect(bx, by, bw, ui.pt(3.0), 0.0, LABEL.now());
                let text = insights::scale_text(bar.length_m, self.units);
                self.put(&mut c, ui, &text, MAC_CAPTION1_PT, Weight::Regular, WHITE, Align::Right, bx - ui.pt(SPACE_XS), by + ui.pt(1.5));
            }
            let note = ui.fit(&insights::map_note(view.map_type), MAC_FOOTNOTE_PT, card_w - ui.pt(2.0 * SPACE_L));
            self.put(&mut c, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(SPACE_L), p(MAP_TOP + MAP_NOTE_TOP));
        }
        // data footer: alert availability for the place and the data attribution, under the last card
        for (i, line) in insights::footer_lines().iter().enumerate() {
            let text = ui.fit(line, MAC_FOOTNOTE_PT, card_w - ui.pt(2.0 * SPACE_L));
            let mid = p(FOOTER_TOP + FOOTER_LINE / 2.0 + i as f32 * FOOTER_LINE);
            self.put(&mut c, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, dim, Align::Left, card_x + ui.pt(SPACE_L), mid);
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
                    Row::Here => {
                        c.fill_rrect(x, y, w, ui.pt(rh - 4.0), ui.pt(RADIUS), CARD_BG.now());
                        if self.here_shown() {
                            c.fill_rect(x as usize, (y + ui.pt(SPACE_S)) as usize, ui.pt(SPACE_XS) as usize, ui.pt(rh - 20.0) as usize, LINK);
                        }
                        let room = (w - ui.pt(150.0)).max(ui.pt(SPACE_HUGE));
                        self.put(&mut c, ui, "My Location", MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                        let hint = ui.fit(&self.here_hint(), MAC_FOOTNOTE_PT, room);
                        self.put(&mut c, ui, &hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(37.0));
                        let temp = match self.my_location.as_ref().and_then(|p| self.forecasts.get(&p.key())) {
                            Some(f) => self.units.degrees(f.current.temp),
                            None => "\u{2013}".to_string(),
                        };
                        self.put(&mut c, ui, &temp, MAC_TITLE1_PT, Weight::Regular, LABEL.now(), Align::Right, x + w - ui.pt(SPACE_L), y + ui.pt(26.0));
                    }
                    Row::Place(i) => {
                        let p = self.places[i].clone();
                        c.fill_rrect(x, y, w, ui.pt(rh - 4.0), ui.pt(RADIUS), CARD_BG.now());
                        if i == self.selected && !self.here_shown() {
                            c.fill_rect(x as usize, (y + ui.pt(SPACE_S)) as usize, ui.pt(SPACE_XS) as usize, ui.pt(rh - 20.0) as usize, LINK);
                        }
                        let room = (w - ui.pt(150.0 + NOTIFY_W)).max(ui.pt(SPACE_HUGE));
                        let name = ui.fit(&p.name, MAC_BODY_PT, room);
                        self.put(&mut c, ui, &name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                        let region = ui.fit(&self.subtitle_with_distance(&p), MAC_FOOTNOTE_PT, room);
                        self.put(&mut c, ui, &region, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, x + ui.pt(SPACE_L), y + ui.pt(37.0));
                        let right = if self.places.len() > 1 { x + w - ui.pt(REMOVE_W) } else { x + w - ui.pt(SPACE_L) };
                        // the Notifications switch: a flat chip, filled when rain alerts are on for the place
                        let on = self.notify_on.contains(&p.key());
                        let chip_w = ui.pt(NOTIFY_W - SPACE_S);
                        let chip_x = right - ui.pt(NOTIFY_W) + ui.pt(SPACE_XS);
                        let chip_y = y + ui.pt((rh - 4.0 - 24.0) / 2.0);
                        c.fill_rrect(chip_x, chip_y, chip_w, ui.pt(SPACE_XXL), ui.pt(SPACE_XS), if on { LINK } else { SEPARATOR_C.now() });
                        self.put(&mut c, ui, "Notify", MAC_FOOTNOTE_PT, Weight::Semibold, if on { WHITE } else { LABEL_2.now() }, Align::Centre, chip_x + chip_w / 2.0, chip_y + ui.pt(SPACE_M));
                        let right = right - ui.pt(NOTIFY_W);
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

/// The colour of the precipitation chances and bars: a light blue that reads on every sky.
fn rain_blue() -> u32 {
    TEAL
}

/// The sun's day arc in a detail tile. `origin` is the arc's centre x and the tile's top, in pixels. A horizon, an arc
/// of dots from sunrise on the left to sunset on the right and, while the sun is up (`progress` is its share of
/// the day), the sun at its place on the arc.
fn draw_sun_arc(c: &mut Surface, ui: &Ui, origin: (f32, f32), progress: Option<f32>) {
    let (cx, top) = origin;
    let (radius, horizon, dot) = (ui.pt(34.0), top + ui.pt(70.0), ui.pt(SPACE_XXS));
    c.fill_rrect(cx - radius - ui.pt(SPACE_S), horizon, 2.0 * radius + ui.pt(2.0 * SPACE_S), 1.0, 0.0, fade(LABEL.now(), 110));
    for step in 0..=24 {
        let (x, y) = insights::arc_point(step as f32 / 24.0, cx, horizon, radius);
        c.fill_rrect(x - dot / 2.0, y - dot / 2.0, dot, dot, dot / 2.0, fade(LABEL.now(), 110));
    }
    if let Some(k) = progress {
        let (x, y) = insights::arc_point(k, cx, horizon, radius);
        let sun = ui.pt(10.0);
        c.fill_rrect(x - sun / 2.0, y - sun / 2.0, sun, sun, sun / 2.0, YELLOW);
    }
}

/// The wind dial in a detail tile, centred on `origin.0` and 56 points under the tile's top (`origin.1`): sixteen
/// ticks around the rim with the four compass points larger, and a pointer from the centre toward the bearing
/// the wind comes from (`from`, degrees clockwise from north) when the service gave one.
fn draw_wind_dial(c: &mut Surface, ui: &Ui, origin: (f32, f32), from: Option<f64>) {
    let (cx, top) = origin;
    let (cy, radius) = (top + ui.pt(56.0), ui.pt(DIAL_EXTENT_PT));
    for k in 0..16 {
        let (x, y) = insights::dial_point(k as f64 * 22.5, cx, cy, radius);
        let (size, alpha) = if k % 4 == 0 { (ui.pt(3.5), 200) } else { (ui.pt(SPACE_XXS), 100) };
        c.fill_rrect(x - size / 2.0, y - size / 2.0, size, size, size / 2.0, fade(LABEL.now(), alpha));
    }
    if let Some(degrees) = from {
        let size = ui.pt(SPACE_XXS);
        for step in 1..=10 {
            let (x, y) = insights::dial_point(degrees, cx, cy, radius * 0.8 * step as f32 / 10.0);
            c.fill_rrect(x - size / 2.0, y - size / 2.0, size, size, size / 2.0, LABEL.now());
        }
        let (x, y) = insights::dial_point(degrees, cx, cy, radius * 0.8);
        let tip = ui.pt(SPACE_S);
        c.fill_rrect(x - tip / 2.0, y - tip / 2.0, tip, tip, tip / 2.0, ORANGE);
    }
    let hub = ui.pt(SPACE_XS);
    c.fill_rrect(cx - hub / 2.0, cy - hub / 2.0, hub, hub, hub / 2.0, LABEL.now());
}

/// The pressure trend dial in a detail tile, centred on `origin.0` and 56 points under the tile's top (`origin.1`):
/// seven ticks over the half circle from falling (left) through steady (straight up) to rising (right), and a
/// needle at `angle` degrees from straight up when the service gave the change (see `insights::pressure_dial_angle`).
fn draw_pressure_dial(c: &mut Surface, ui: &Ui, origin: (f32, f32), angle: Option<f64>) {
    let (cx, top) = origin;
    let (cy, radius) = (top + ui.pt(56.0), ui.pt(DIAL_EXTENT_PT));
    for k in 0..7 {
        let (x, y) = insights::dial_point(-90.0 + k as f64 * 30.0, cx, cy, radius);
        let (size, alpha) = if k % 3 == 0 { (ui.pt(3.5), 200) } else { (ui.pt(SPACE_XXS), 100) };
        c.fill_rrect(x - size / 2.0, y - size / 2.0, size, size, size / 2.0, fade(LABEL.now(), alpha));
    }
    if let Some(degrees) = angle {
        let size = ui.pt(SPACE_XXS);
        for step in 1..=10 {
            let (x, y) = insights::dial_point(degrees, cx, cy, radius * 0.8 * step as f32 / 10.0);
            c.fill_rrect(x - size / 2.0, y - size / 2.0, size, size, size / 2.0, LABEL.now());
        }
        let (x, y) = insights::dial_point(degrees, cx, cy, radius * 0.8);
        let tip = ui.pt(SPACE_S);
        c.fill_rrect(x - tip / 2.0, y - tip / 2.0, tip, tip, tip / 2.0, ORANGE);
    }
    let hub = ui.pt(SPACE_XS);
    c.fill_rrect(cx - hub / 2.0, cy - hub / 2.0, hub, hub, hub / 2.0, LABEL.now());
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

    #[test]
    fn the_my_location_hint_follows_the_form_factor() {
        use bifrost_app::form_factor::FormFactor;
        assert_eq!(here_prompt(FormFactor::Tablet), "Tap to find this place");
        assert_eq!(here_prompt(FormFactor::Desktop), "Click to find this place");
    }

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

    /// An app over `names` as saved places (the first selected), with the window wide enough for the sidebar.
    pub(super) fn app_with(tag: &str, names: &[&str]) -> (Weather, Ui, PathBuf) {
        let dir = temp(tag);
        let places: Vec<Location> = names.iter().enumerate().map(|(i, n)| place(n, 60.0 + i as f64)).collect();
        Store::new(dir.clone()).save_locations(&places);
        let ui = test_ui();
        let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        app.relayout(&ui);
        (app, ui, dir)
    }

    #[test]
    fn the_drawn_toolbar_sits_where_clicks_land_with_the_sidebar_shown() {
        let (app, ui, _) = app_with("bar-drawn", &["A"]);
        assert!(app.side > 0.0, "the 1200 px window shows the sidebar");
        let drawn = widgets::toolbar_layout_in(&ui, app.bar_bounds(&ui), &app.bar(), app.side / ui.pt(1.0));
        let hit = app.bar_layout(&ui);
        assert_eq!(drawn.visible.len(), hit.visible.len());
        for ((id, d), (hid, h)) in drawn.visible.iter().zip(hit.visible.iter()) {
            assert_eq!(id, hid);
            assert!((d.x - h.x).abs() < 0.01 && (d.w - h.w).abs() < 0.01, "item {id} drawn at {} but hit-tested at {}", d.x, h.x);
        }
    }

    fn names(app: &Weather) -> Vec<&str> {
        app.places.iter().map(|p| p.name.as_str()).collect()
    }

    /// A forecast whose minute block has these precipitation values for the steps from 13:15.
    fn forecast_with_minutes(minutes: &str) -> Forecast {
        let text = format!(
            r#"{{"current":{{"time":"2026-10-04T13:20","temperature_2m":7.4,"weather_code":3,"is_day":1}},
            "hourly":{{"time":["2026-10-04T13:00"],"temperature_2m":[7.0],"weather_code":[3]}},
            "daily":{{"time":["2026-10-04"],"weather_code":[3],"temperature_2m_max":[8.0],"temperature_2m_min":[2.0]}},
            "minutely_15":{{"time":["2026-10-04T13:15","2026-10-04T13:30","2026-10-04T13:45","2026-10-04T14:00"],"precipitation":{minutes}}}}}"#
        );
        data::parse_forecast(&text).unwrap()
    }

    #[test]
    fn dragging_a_sidebar_row_reorders_the_places_and_keeps_the_selection() {
        let (mut app, ui, dir) = app_with("drag", &["A", "B", "C"]);
        assert!(app.side > 0.0, "the window is wide enough for the sidebar");
        let spans = app.side_row_spans();
        let mid = |i: usize| ui.pt(spans[i].0 + spans[i].1 / 2.0);
        let x = ui.pt(40.0);
        app.touch(&ui, Event::Down(x, mid(0)), 0.0);
        app.touch(&ui, Event::Move(x, mid(1)), 0.1);
        assert!(app.side_drag.is_some_and(|d| d.active), "a move past the threshold becomes a drag");
        app.touch(&ui, Event::Up(x, mid(2)), 0.2);
        assert_eq!(names(&app), ["B", "C", "A"]);
        assert_eq!(app.selected, 2, "A stays selected where it was dropped");
        assert_eq!(app.screen, Screen::Main, "a drop is not a tap on the row");
        let store = Store::new(dir.clone());
        assert_eq!(store.load_locations().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B", "C", "A"], "the order is saved");
        assert_eq!(store.load_settings().1, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tap_on_a_sidebar_row_still_selects_it() {
        let (mut app, ui, dir) = app_with("tap", &["A", "B", "C"]);
        let spans = app.side_row_spans();
        let (x, y) = (ui.pt(40.0), ui.pt(spans[1].0 + spans[1].1 / 2.0));
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Up(x, y), 0.1);
        assert_eq!((app.selected, names(&app)), (1, vec!["A", "B", "C"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_delete_key_removes_the_selected_place_but_never_the_last() {
        let (mut app, ui, dir) = app_with("delete", &["A", "B"]);
        app.key(&ui, Out::Named("Delete"), 0.0);
        assert_eq!(names(&app), ["B"]);
        assert_eq!(Store::new(dir.clone()).load_locations().len(), 1, "the removal is saved");
        app.key(&ui, Out::Named("Delete"), 0.1);
        assert_eq!(names(&app), ["B"], "the last place stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_does_not_remove_a_place_while_a_field_is_being_typed_in() {
        let (mut app, ui, dir) = app_with("delete-field", &["A", "B"]);
        app.activate(&ui, Target::Places, 0.0, 0.0);
        app.activate(&ui, Target::Field, 0.0, 0.0);
        assert!(app.field.focused);
        app.key(&ui, Out::Named("Delete"), 0.1);
        assert_eq!(names(&app), ["A", "B"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_context_menu_of_a_sidebar_row_offers_notifications_and_remove() {
        let (mut app, ui, dir) = app_with("context", &["A", "B"]);
        let spans = app.side_row_spans();
        let (x, y) = (ui.pt(40.0), ui.pt(spans[1].0 + spans[1].1 / 2.0));
        let items = app.context_items(&ui, x, y);
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Turn On Notifications", "Remove"]);
        assert!(items.iter().all(|i| i.enabled));
        assert!(app.context_action(&ui, CTX_NOTIFY, 0.0));
        assert!(app.notify_on.contains(&app.places[1].key()), "the menu switched notifications on for the row it opened on");
        let again = app.context_items(&ui, x, y);
        assert_eq!(again[0].label, "Turn Off Notifications");
        assert!(app.context_action(&ui, CTX_REMOVE, 0.0));
        assert_eq!(names(&app), ["A"]);
        assert!(app.notify_on.is_empty(), "a removed place keeps no notification switch");
        assert!(app.context_items(&ui, x, ui.pt(700.0)).is_empty(), "nothing is under the finger below the rows");
        assert!(!app.context_action(&ui, CTX_REMOVE, 0.0), "no menu is open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_place_cannot_be_removed_from_its_context_menu() {
        let (mut app, ui, dir) = app_with("context-last", &["A"]);
        let spans = app.side_row_spans();
        let items = app.context_items(&ui, ui.pt(40.0), ui.pt(spans[0].0 + spans[0].1 / 2.0));
        assert_eq!(items[1].label, "Remove");
        assert!(!items[1].enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notifications_switch_is_saved_per_place() {
        let (mut app, ui, dir) = app_with("notify", &["A", "B"]);
        let key = app.places[1].key();
        app.activate(&ui, Target::Notify(1), 0.0, 0.0);
        assert!(app.notify_on.contains(&key));
        assert!(notify::load(&dir).contains(&key), "saved");
        let restarted = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
        assert!(restarted.notify_on.contains(&key), "and loaded on the next start");
        app.activate(&ui, Target::Notify(1), 0.0, 0.0);
        assert!(notify::load(&dir).is_empty());
        app.activate(&ui, Target::Notify(9), 0.0, 0.0);
        assert!(app.notify_on.is_empty(), "an index past the list changes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rain_starting_within_the_hour_posts_one_notice_for_a_place_with_notifications_on() {
        let (mut app, ui, dir) = app_with("rain", &["A", "B"]);
        let (a, b) = (app.places[0].key(), app.places[1].key());
        app.activate(&ui, Target::Notify(0), 0.0, 0.0);
        let soon = forecast_with_minutes("[0.0,0.0,0.4,0.4]");
        assert_eq!(soon.minutes, vec![0.0, 0.0, 0.4, 0.4]);
        for key in [&a, &b] {
            app.done.lock().unwrap().push(Done::Forecast { key: key.clone(), result: Ok(soon.clone()) });
        }
        app.pump();
        let notices = app.take_notices();
        assert_eq!(notices.len(), 1, "only the place with the switch on: {notices:?}");
        assert_eq!(notices[0].title, "Rain soon in A");
        assert_eq!(notices[0].body, "Rain is expected to start in about 30 minutes.");
        app.done.lock().unwrap().push(Done::Forecast { key: a.clone(), result: Ok(soon) });
        app.pump();
        assert!(app.take_notices().is_empty(), "the next refresh does not repeat it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sidebar_shows_each_places_sky_and_temperature_once_its_forecast_is_known() {
        let (mut app, _ui, dir) = app_with("rows", &["A", "B"]);
        let row = |app: &Weather, i: usize| match &app.sidebar().items[i + app.here_offset()] {
            SidebarItem::Row { subtitle, trailing, .. } => (subtitle.clone(), trailing.clone()),
            other => panic!("a row, not {other:?}"),
        };
        assert_eq!(row(&app, 0), (Some("Test".to_string()), None), "no forecast yet: the region, no temperature");
        let key = app.places[0].key();
        app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
        assert_eq!(row(&app, 0), (Some("Cloudy".to_string()), Some("7\u{b0}".to_string())));
        app.units = Units::Fahrenheit;
        assert_eq!(row(&app, 0).1.as_deref(), Some("45\u{b0}"), "the temperature follows the unit setting");
        assert_eq!(row(&app, 1).1, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_weather_link_selects_a_saved_place_or_adds_one_by_position() {
        let (mut app, ui, dir) = app_with("link", &["Akureyri", "Reykjavik"]);
        assert!(app.open_link(&ui, "weather://reykjavik", 0.0), "names match without regard to case");
        assert_eq!(app.selected, 1);
        assert_eq!(names(&app), ["Akureyri", "Reykjavik"]);
        assert!(app.open_link(&ui, "weather://?lat=63.4&lon=-20.3&name=Vestmannaeyjar", 0.1));
        assert_eq!(names(&app), ["Akureyri", "Reykjavik", "Vestmannaeyjar"]);
        assert_eq!(app.selected, 2);
        assert_eq!(Store::new(dir.clone()).load_locations().len(), 3, "the new place is saved");
        assert!(app.open_link(&ui, "weather://?lat=63.4&lon=-20.3&name=Vestmannaeyjar", 0.2));
        assert_eq!(app.places.len(), 3, "the same position is not added twice");
        assert!(!app.open_link(&ui, "https://example.com", 0.3), "other links are not ours");
        assert!(!app.open_link(&ui, "weather://", 0.4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_weather_link_to_an_unknown_name_is_looked_up_and_the_first_match_shown() {
        let (mut app, ui, dir) = app_with("link-search", &["Akureyri"]);
        assert!(app.open_link(&ui, "weather://Oslo", 0.0), "the lookup starts");
        assert_eq!(names(&app), ["Akureyri"], "nothing is added before the answer");
        let found = Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 };
        app.done.lock().unwrap().push(Done::Linked { name: "Oslo".into(), result: Ok(vec![found, place("Oslo, Texas", 30.0)]) });
        app.tick(&ui, 1.0);
        assert_eq!(names(&app), ["Akureyri", "Oslo"]);
        assert_eq!(app.selected, 1);
        app.done.lock().unwrap().push(Done::Linked { name: "Nowhere".into(), result: Ok(vec![]) });
        app.tick(&ui, 2.0);
        assert_eq!(names(&app), ["Akureyri", "Oslo"], "no match, no change");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_list_ignores_a_link_to_a_new_place() {
        let names_list: Vec<String> = (0..data::MAX_LOCATIONS).map(|i| format!("P{i}")).collect();
        let refs: Vec<&str> = names_list.iter().map(String::as_str).collect();
        let (mut app, ui, dir) = app_with("link-full", &refs);
        assert!(app.open_link(&ui, "weather://?lat=1.5&lon=2.5&name=Extra", 0.0));
        assert_eq!(app.places.len(), data::MAX_LOCATIONS);
        assert_eq!(app.selected, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn control_methods_answer_for_the_selected_or_a_named_place() {
        let (mut app, _ui, dir) = app_with("rpc", &["A", "B"]);
        let none = Json::Obj(vec![]);
        let named = |n: &str| Json::Obj(vec![("place".to_string(), Json::Str(n.to_string()))]);
        let places = app.rpc("weather.places", &none).unwrap();
        assert_eq!(places, Json::Arr(vec![Json::Str("A".into()), Json::Str("B".into())]));
        assert_eq!(app.rpc("weather.current", &none).unwrap_err().code, INTERNAL_ERROR, "no forecast loaded yet");
        assert_eq!(app.rpc("weather.nope", &none).unwrap_err().code, METHOD_NOT_FOUND);
        let key = app.places[1].key();
        app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
        let current = app.rpc("weather.current", &named("b")).unwrap();
        assert_eq!(current.str_field("place").as_deref(), Some("B"));
        assert_eq!(current.str_field("condition").as_deref(), Some("Cloudy"));
        assert_eq!(app.rpc("weather.current", &none).unwrap_err().code, INTERNAL_ERROR, "the selected place A has none");
        assert_eq!(app.rpc("weather.forecast", &named("Z")).unwrap_err().code, INVALID_PARAMS);
        app.selected = 1;
        let forecast = app.rpc("weather.forecast", &none).unwrap();
        assert_eq!(forecast.get("days").and_then(Json::as_array).map(|d| d.len()), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_share_button_needs_a_forecast_and_sends_its_text_once() {
        let (mut app, ui, dir) = app_with("share", &["A"]);
        assert!(!app.shareable());
        app.activate(&ui, Target::Share, 0.0, 0.0);
        assert!(!app.take_share(), "nothing to share yet");
        assert_eq!(app.share_text(), None);
        let key = app.place().key();
        app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
        assert!(app.shareable());
        app.activate(&ui, Target::Share, 0.0, 0.0);
        assert!(app.take_share());
        assert!(!app.take_share(), "the request is taken once");
        assert_eq!(app.share_text().as_deref(), Some("A: 7\u{b0} Cloudy, H:8\u{b0} L:2\u{b0}\nWeather data from Open-Meteo"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_toolbar_has_a_share_button_on_the_forecast_page_only() {
        let (mut app, ui, dir) = app_with("share-bar", &["A"]);
        let labels = |app: &Weather| app.bar().items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        assert!(labels(&app).contains(&"Share".to_string()));
        assert!(!labels(&app).contains(&"Stocks".to_string()), "the forecast page has no Stocks button");
        app.activate(&ui, Target::Places, 0.0, 0.0);
        assert!(!labels(&app).contains(&"Share".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_locations_page_has_a_notifications_hit_zone_between_the_temperature_and_remove() {
        let (mut app, ui, dir) = app_with("zone", &["A", "B"]);
        app.activate(&ui, Target::Places, 0.0, 0.0);
        let y = ui.pt(places_list_top_pt() + ROW * (app.here_offset() as f32 + 0.5));
        let left = app.notify_left(&ui);
        assert_eq!(app.hit(&ui, left + ui.pt(10.0), y), Some(Target::Notify(0)));
        assert_eq!(app.hit(&ui, left - ui.pt(10.0), y), Some(Target::Place(0)));
        assert_eq!(app.hit(&ui, app.w as f32 - ui.pt(MARGIN + 10.0), y), Some(Target::Remove(0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E W4-11: the wheel over the detail pane scrolls the forecast, within its range; the sidebar leaves it alone.
    #[test]
    fn the_wheel_scrolls_the_forecast_page() {
        let (mut app, ui, dir) = app_with("wheel", &["A"]);
        app.fit_scroll(&ui);
        let (x, y) = (app.side + ui.pt(200.0), 700.0);
        assert!(app.wheel(&ui, x, y, 0.0, -120.0), "a wheel turned down scrolls the page");
        assert!(app.scroll.offset() > 0.0);
        assert!(app.wheel(&ui, x, y, 0.0, 10_000.0));
        assert_eq!(app.scroll.offset(), 0.0, "clamped at the top");
        assert!(!app.wheel(&ui, x, y, 0.0, 120.0), "nothing above the top");
        if app.side > 0.0 {
            assert!(!app.wheel(&ui, 2.0, y, 0.0, -120.0), "the sidebar does not scroll the forecast");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E W4-12: Return in Locations searches, and while the answer is on its way the app asks to be woken for it.
    #[test]
    fn return_in_locations_searches_and_the_answer_shows_without_more_input() {
        let (mut app, ui, dir) = app_with("enter", &["A"]);
        app.activate(&ui, Target::Places, 0.0, 0.0);
        assert_eq!(app.screen, Screen::Places);
        app.field.set_text(&ui, "London");
        app.field.focused = false;
        app.key(&ui, Out::Enter, 10.0);
        assert!(app.searching, "Return starts the search");
        assert!(app.next_wake(20.0).is_some_and(|w| w <= 20.0 + FETCH_POLL_MS), "polls for the answer");
        // the offline answer lands on the utility queue and a plain tick applies it
        let mut landed = false;
        for i in 0..400 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.tick(&ui, 100.0 + i as f64);
            if !app.searching {
                landed = true;
                break;
            }
        }
        assert!(landed, "the search finished");
        assert_eq!(app.note.as_deref(), Some("Search unavailable"));
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
    fn my_location_is_named_and_its_coordinates_rounded_to_two_decimals() {
        let fix = Fix {
            coordinate: bifrost_app::core_location::Coordinate::new(64.146_57, -21.942_61),
            altitude: None,
            horizontal_accuracy: 100.0,
            timestamp: 0.0,
            source: "manual".into(),
            city: String::new(),
            country: String::new(),
        };
        let here = here_place(&fix);
        assert_eq!(here.name, "My Location");
        assert_eq!(here.region, "64.15, -21.94", "the label is the rounded position");
        assert_eq!((here.lat, here.lon), (64.15, -21.94), "the forecast is asked for the rounded position too");
    }

    #[test]
    fn my_location_leads_the_sidebar_until_location_is_refused() {
        let (mut app, _ui, dir) = super::tests::app_with("here-row", &["A", "B"]);
        assert_eq!(app.here_offset(), 1, "listed while location is not asked, so the person can pick it");
        assert_eq!(app.sidebar().items.len(), 3);
        app.stop_here(LocationError::Denied);
        assert_eq!(app.here_offset(), 0, "gone once location is off for Weather");
        assert_eq!(app.sidebar().items.len(), 2);
        assert!(!app.here_shown());
        let _ = std::fs::remove_dir_all(&dir);
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
