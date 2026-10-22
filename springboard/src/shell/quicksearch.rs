//! Search: a pull down on the home screen (clear of the status bar and
//! the home indicator) drops the bifrost 15 search card over the home
//! screen with the keyboard rising from the bottom. Empty, the card shows
//! "Assistant Suggestions": the four most recently opened apps. As the query is
//! typed it lists the Top Hit, then apps, Settings pages, World Clock
//! cities and Deildu titles cached on disk, with "Search in Terminal" last.
//! Return opens the Top Hit (or the row the Up and Down arrow keys are on); a tap outside, a swipe up or the go-home swipe
//! dismisses. Geometry is the bifrost mini's in points: a 556 pt card 41 pt
//! from the top with a 52 pt field, 60 pt suggestion icons on an 88 pt
//! pitch and 36 pt result rows.

use super::clock::tz::CITIES;
use super::gestures::{ease_out_quartic, progress, Drag, SPRING_MS};
use super::keyboard::{Keyboard, Out};
use super::quicksearch_math;
use super::prefs::Prefs;
use super::textfield::TextField;
use super::icons::{self, Icon};
use super::pages::PageId;
use super::row_glyphs::{self, Glyph};
use super::ui::{home_indicator, home_zone, Ui, Weight, CELL, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_LARGE_TITLE_PT, MAC_TITLE2_PT, MENU_SOLID, SEARCH_TINT, SECONDARY, SIDEBAR_PLATE, WHITE};
use super::{fuzzy, glyphs, settings, AppId, Effect, Event, Screen, Shell, apps};
use crate::gfx::{scale, Image, Surface};
use bifrost_ui::{search_rank, uttype};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CARD_W_PT: f32 = 556.0;
const CARD_TOP_PT: f32 = 41.0;
const RADIUS_PT: f32 = 6.0;
const FIELD_PT: f32 = 52.0;
const FIELD_TEXT_PT: f32 = 22.0;
/// where the query text starts, from the card's left edge
const TEXT_X_PT: f32 = 56.0;
const GLASS_X_PT: f32 = 28.0;
const GLASS_R_PT: f32 = 9.0;
const CLEAR_R_PT: f32 = 11.0;
const CLEAR_X_PT: f32 = 28.0;
const MIC_PT: f32 = 26.0;
/// section header band and where its text is centred
const HEADER_PT: f32 = 46.0;
const HEADER_MID_PT: f32 = 27.5;
const INSET_PT: f32 = 21.0;
const ROW_PT: f32 = 36.0;
const ROW_ICON_PT: f32 = 29.0;
const ROW_ICON_X_PT: f32 = 11.0;
const ROW_TEXT_X_PT: f32 = 52.0;
const BOTTOM_PAD_PT: f32 = 4.5;
/// the suggestions row: icons below the header, their labels under them
const SUGGEST_PT: f32 = 96.5;
const SUGGEST_ICON_PT: f32 = 60.0;
const SUGGEST_ICON_TOP_PT: f32 = 9.5;
const SUGGEST_PITCH_PT: f32 = 88.0;
const SUGGEST_INSET_PT: f32 = 28.0;
const SUGGEST_LABEL_GAP_PT: f32 = 5.0;
/// finger travel for a full reveal; a release past 35 % of it (40 pt) opens
pub const REVEAL_PT: f32 = 115.0;
/// travel before a home-screen touch is either a page swipe or a pull
pub const DIRECTION_PT: f32 = 7.0;
/// a touch that moves less than this is a tap
const TAP_PT: f32 = 8.0;
const RECENTS: usize = 6;
const SEARCHES: usize = 4;
const PER_GROUP: usize = 3;
/// The file beside quicksearch.ini that keeps how often each indexed result was opened (local only).
const USAGE_FILE: &str = "search-usage.txt";
/// The launch history the app prediction reads, beside quicksearch.ini (`bifrost_ui::app_prediction`).
const LAUNCHES_FILE: &str = "app-launches.txt";
/// The opens of indexed results, read by the Documents section; loaded by `QuickSearch::load`.
static OPENS: std::sync::Mutex<Option<bifrost_ui::search_rank::Usage>> = std::sync::Mutex::new(None);
/// How long a scan of the Files root is reused by later opens before the next opening scans again.
const FILES_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(20);
const QUERY_MAX: usize = 64;
/// the card over the wallpaper, as measured on the bifrost
const CARD_BG: u32 = rgba_const(38, 41, 45, 222);
/// Liquid Glass (regular) for the desktop Search card: a light dark tint over the blurred backdrop,
/// a bright 1 pt edge, and the solid card colour when Reduce Transparency is on.
const GLASS_TINT: u32 = rgba_const(30, 32, 36, 120);
const GLASS_EDGE: u32 = rgba_const(255, 255, 255, 46);
const GLASS_BLUR_PT: f32 = 18.0;
const SEPARATOR: u32 = rgba_const(255, 255, 255, 36);
const PRESSED: u32 = rgba_const(255, 255, 255, 30);
/// the keyboard's opacity over the wallpaper
const KB_ALPHA: u32 = 215;

/// Card geometry in points: the bifrost's bifrost card and the desktop's Golden Gate-style panel (`mac`).
struct Metrics {
    card_w: f32,
    card_top: f32,
    radius: f32,
    field: f32,
    text_x: f32,
    glass_x: f32,
    clear_x: f32,
    mic: bool,
    header: f32,
    header_mid: f32,
    inset: f32,
    row: f32,
    row_icon_x: f32,
    row_icon: f32,
    row_text_x: f32,
    bottom_pad: f32,
    label_pt: f32,
    /// the empty field shows Assistant Suggestions (bifrost) or nothing (Golden Gate)
    suggestions: bool,
    /// the band of filter chips above the results (Golden Gate; 0 on the bifrost, which has section headers)
    chips: f32,
}

const BIFROST: Metrics = Metrics {
    card_w: CARD_W_PT,
    card_top: CARD_TOP_PT,
    radius: RADIUS_PT,
    field: FIELD_PT,
    text_x: TEXT_X_PT,
    glass_x: GLASS_X_PT,
    clear_x: CLEAR_X_PT,
    mic: true,
    header: HEADER_PT,
    header_mid: HEADER_MID_PT,
    inset: INSET_PT,
    row: ROW_PT,
    row_icon_x: ROW_ICON_X_PT,
    row_icon: ROW_ICON_PT,
    row_text_x: ROW_TEXT_X_PT,
    bottom_pad: BOTTOM_PAD_PT,
    label_pt: 17.0,
    suggestions: true,
    chips: 0.0,
};

/// Golden Gate Search on the desktop, measured from the Golden Gate captures (27-Golden-Gate-Search-Search-v2):
/// a 520 pt wide, 84 pt tall glass capsule 139 pt from the top with the query in the large-title size, and the
/// results in a separate glass card 12 pt below it (radius 28): filter chips across its top (one per kind of result
/// present, in place of section headers), then rows of a 40 pt icon with a title and a second line, the top hit on
/// an inset neutral plate (Return opens it). With nothing typed only the capsule shows. That is 0.37 of a 1408 pt
/// wide screen: on a wider one everything grows with the screen (`QuickSearch::k`) so the proportions hold.
const MAC: Metrics = Metrics {
    card_w: 520.0,
    card_top: 139.0,
    radius: 28.0,
    field: 84.0,
    text_x: 38.0,
    glass_x: 0.0,
    clear_x: 30.0,
    mic: false,
    header: 0.0,
    header_mid: 16.0,
    inset: 26.0,
    row: 56.0,
    row_icon_x: 20.0,
    row_icon: 40.0,
    row_text_x: 76.0,
    bottom_pad: 12.0,
    label_pt: MAC_TITLE2_PT,
    suggestions: false,
    chips: 52.0,
};
/// Golden Gate: the card is this fraction of the screen's width (the reference's 520 pt of 1408 pt), never narrower than
/// its point size, and the whole layout grows by at most `MAC_K_MAX` with it.
const MAC_CARD_FRACTION: f32 = 0.37;
const MAC_K_MAX: f32 = 2.5;
/// Golden Gate: a filter chip's height, the gap between chips and the padding either side of its text
const MAC_CHIP_H_PT: f32 = 28.0;
const MAC_CHIP_GAP_PT: f32 = 8.0;
const MAC_CHIP_PAD_PT: f32 = 14.0;
/// Result groups that are not a kind of result a person filters to: the best match, the folder action and the Terminal row.
const NOT_A_KIND: [&str; 3] = ["Top Hit", "Show in Folder", "Search in Apps"];
/// Golden Gate: the gap between the search capsule and the results card
const MAC_GAP_PT: f32 = 12.0;
/// Golden Gate: the top-hit plate's inset from the card edge and its corner radius
const MAC_PLATE_INSET_PT: f32 = 10.0;
const MAC_PLATE_R_PT: f32 = 12.0;
/// Golden Gate: the gap kept between the bottom of the results card and the top of the Dock (or the screen's bottom edge)
const MAC_DOCK_MARGIN_PT: f32 = 12.0;
/// Golden Gate: the most result rows built for the scrolling list; the rest of the ranking is not shown
const MAC_MAX_ROWS: usize = 30;
/// How long after a wheel notch the touches the window server emulates for it (a down, a move, an up) are dropped
const WHEEL_DRAG_MS: f64 = 250.0;

const fn rgba_const(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let k = a as u32;
    ((r as u32 * k + 127) / 255) | (((g as u32 * k + 127) / 255) << 8) | (((b as u32 * k + 127) / 255) << 16) | (k << 24)
}

/// What a result opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    App(AppId),
    Contact { id: u64, name: String },
    /// a Settings page by its row name ("Wi-Fi", "Wallpaper", …)
    Settings(&'static str),
    /// a World Clock city
    City(&'static str),
    /// a Deildu title from the disk cache, searched for in the library
    Deildu(String),
    /// "Search in Terminal": opens the Terminal and types the query
    Terminal(String),
    /// an inline calculator or unit-conversion answer
    Answer(String),
    /// a file under the Files root, previewed in Files
    File { path: PathBuf, name: String },
    /// "Show in Folder": opens Files at `folder`, the folder that holds the file `name`
    Reveal { folder: PathBuf, name: String },
    /// an app shortcut (`AppShortcut`): an intent declared in the app's manifest; opens the app and runs `intent.<id>`
    Shortcut { app_id: String, app_name: String, intent: String, title: String },
    /// a Settings intent (`settings_intents`, "Open Keyboard"): opens the pane it names in Settings
    SettingsIntent { id: &'static str, title: &'static str },
    /// a shell action intent (`settings_actions`) with its parameters already set: runs the same effect the Settings pane does
    SettingsAction { intent: super::settings_actions::SettingsIntent, title: String },
    /// a transport command of the Now Playing session (`media_intents::TRANSPORT`): play or pause, next, previous.
    /// Sent to the session's owner; no window opens.
    MediaTransport { command: bifrost_ui::media_session::RemoteCommand, title: String },
    /// an item an app put into the Core Search index: opens the app and sends it a continue-activity message.
    /// `kind` is the file type's name ("PDF document", "" when unknown); `thumb` the thumbnail path in the app's data dir.
    Indexed { app_id: String, unique_id: String, domain: String, title: String, kind: String, thumb: String },
    /// a calendar event: opens the Calendar app
    Event { title: String },
}

#[cfg(test)]
mod app_section_title_tests {
    use super::app_section_title;

    #[test]
    fn an_app_section_is_titled_by_the_app_name_and_the_name_is_kept_once() {
        let a = app_section_title("Notes");
        assert_eq!(a, "Notes");
        assert!(std::ptr::eq(a, app_section_title("Notes")), "the same name is kept once");
        assert_eq!(app_section_title("Mail"), "Mail");
    }
}

/// The label of an indexed result: its title, then its content type when it has one ("Q1 report · PDF document").
fn indexed_label(title: &str, kind: &str) -> String {
    if kind.is_empty() {
        title.to_string()
    } else {
        format!("{title} · {kind}")
    }
}

#[cfg(test)]
mod indexed_label_tests {
    use super::indexed_label;

    #[test]
    fn an_indexed_result_shows_its_content_type_after_the_title() {
        assert_eq!(indexed_label("Q1 report", "PDF document"), "Q1 report · PDF document");
        assert_eq!(indexed_label("Notes", ""), "Notes");
    }
}

/// The symbol a Settings result draws: the tile its pane has in the Settings list, so a row in Search and the row in
/// Settings read as the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Symbol {
    Tile(Icon),
    Glyph(Glyph),
}

/// The symbol of a Settings pane.
fn pane_symbol(page: PageId) -> Symbol {
    use PageId as P;
    match page {
        P::Wifi | P::WifiDetail | P::KnownNetworks | P::Network | P::Ethernet | P::Vpn | P::Sharing | P::Firewall => Symbol::Tile(Icon::Wifi),
        P::Bluetooth | P::BluetoothDetail => Symbol::Tile(Icon::Bluetooth),
        P::Notifications | P::NotificationApp => Symbol::Tile(Icon::Notifications),
        P::Sounds => Symbol::Tile(Icon::Sounds),
        P::Focus => Symbol::Tile(Icon::Focus),
        P::ScreenTime => Symbol::Tile(Icon::ScreenTime),
        P::ControlCentre | P::MenuBar => Symbol::Tile(Icon::ControlCentre),
        P::Display | P::Appearance => Symbol::Tile(Icon::Display),
        P::HomeScreen | P::Overview | P::HotCorners => Symbol::Tile(Icon::HomeScreen),
        P::Accessibility | P::AccessibilityShortcut => Symbol::Tile(Icon::Accessibility),
        P::Wallpaper | P::ScreenSaver => Symbol::Tile(Icon::Wallpaper),
        P::Battery => Symbol::Tile(Icon::Battery),
        P::Privacy => Symbol::Tile(Icon::Privacy),
        P::Passcode | P::Passwords | P::LockScreen | P::Users | P::Account => Symbol::Tile(Icon::Passcode),
        P::About => Symbol::Glyph(Glyph::About),
        P::SoftwareUpdate => Symbol::Glyph(Glyph::Update),
        P::Storage | P::StartupDisk => Symbol::Glyph(Glyph::Storage),
        P::DateTime => Symbol::Glyph(Glyph::DateTime),
        P::Keyboard | P::Shortcuts => Symbol::Glyph(Glyph::Keyboard),
        P::Language => Symbol::Glyph(Glyph::Language),
        P::Backup => Symbol::Glyph(Glyph::Backup),
        P::LoginItems | P::DefaultApps => Symbol::Glyph(Glyph::DefaultApps),
        _ => Symbol::Tile(Icon::General),
    }
}

/// The symbol of a result that is not an app: a Settings page by its row name, a Settings intent by the pane it opens, a
/// shell action by what it changes, the Now Playing transport by sound. None for every other kind of result.
fn result_symbol(hit: &Hit) -> Option<Symbol> {
    use super::settings_actions::SettingsIntent as Action;
    match hit {
        Hit::Settings(name) => Some(match *name {
            "Wi-Fi" => Symbol::Tile(Icon::Wifi),
            "Display & Brightness" => Symbol::Tile(Icon::Display),
            "Wallpaper" => Symbol::Tile(Icon::Wallpaper),
            "About" => Symbol::Glyph(Glyph::About),
            "Battery" => Symbol::Tile(Icon::Battery),
            "Focus" | "Do Not Disturb" => Symbol::Tile(Icon::Focus),
            "Notifications" => Symbol::Tile(Icon::Notifications),
            "Accessibility" | "AssistiveTouch" => Symbol::Tile(Icon::Accessibility),
            _ => Symbol::Tile(Icon::General),
        }),
        Hit::SettingsIntent { id, .. } => Some(super::settings_intents::page_for(id).map_or(Symbol::Tile(Icon::General), pane_symbol)),
        Hit::SettingsAction { intent, .. } => Some(match intent {
            Action::WifiPower(_) | Action::WifiScan => Symbol::Tile(Icon::Wifi),
            Action::SetWallpaper(_) => Symbol::Tile(Icon::Wallpaper),
            Action::LockScreen => Symbol::Tile(Icon::Passcode),
        }),
        Hit::MediaTransport { .. } => Some(Symbol::Tile(Icon::Sounds)),
        _ => None,
    }
}

/// The second line of an action result.
fn action_line(intent: &super::settings_actions::SettingsIntent) -> &'static str {
    use super::settings_actions::SettingsIntent as Action;
    match intent {
        Action::WifiScan => "Look for Wi-Fi networks nearby",
        Action::WifiPower(true) => "Switch the Wi-Fi radio on",
        Action::WifiPower(false) => "Switch the Wi-Fi radio off",
        Action::SetWallpaper(_) => "Use this picture as the desktop wallpaper",
        Action::LockScreen => "Lock the screen now",
    }
}

/// A result as the Golden Gate panel shows it: its title and its second line (what kind of thing it is, where it is, or
/// what it does). The second line is empty when there is nothing more to say.
fn row_text(hit: &Hit, apps_name: impl Fn(AppId) -> String) -> (String, String) {
    let folder_of = |path: &Path| path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match hit {
        Hit::App(app) => (apps_name(*app), "Application".to_string()),
        Hit::Settings(name) => (name.to_string(), "Settings".to_string()),
        Hit::City(name) => (name.to_string(), "World Clock".to_string()),
        Hit::Deildu(title) => (title.clone(), "Deildu".to_string()),
        Hit::Contact { name, .. } => (name.clone(), "Contact".to_string()),
        Hit::Event { title } => (title.clone(), "Calendar event".to_string()),
        Hit::Terminal(query) => ("Search in Terminal".to_string(), query.clone()),
        Hit::Answer(a) => (format!("= {a}"), "Calculator".to_string()),
        Hit::File { path, name } => {
            let kind = crate::files::fs::kind_of(name, path.is_dir()).label();
            let folder = folder_of(path);
            (name.clone(), if folder.is_empty() { kind.to_string() } else { format!("{kind} \u{b7} {folder}") })
        }
        Hit::Reveal { folder, name } => (format!("Show {name} in Folder"), folder.display().to_string()),
        Hit::Shortcut { title, app_name, .. } => (title.clone(), format!("Shortcut \u{b7} {app_name}")),
        Hit::SettingsIntent { id, title } => (title.to_string(), super::settings_intents::find(id).map_or("Settings", |i| i.description).to_string()),
        Hit::SettingsAction { intent, title } => (title.clone(), action_line(intent).to_string()),
        Hit::MediaTransport { title, .. } => (title.clone(), "Control the playing media".to_string()),
        Hit::Indexed { title, kind, .. } => (title.clone(), kind.clone()),
    }
}

/// The filter chips of a result list: the title of each group that is a kind of result, once each, in list order.
fn chips_of(groups: &[Group]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    for g in groups {
        if !NOT_A_KIND.contains(&g.title) && !out.contains(&g.title) && !g.hits.is_empty() {
            out.push(g.title);
        }
    }
    out
}

/// The runnable `[[intent]]` shortcuts of the installed apps, with their folded search text.
fn shortcut_items() -> Vec<(Hit, String)> {
    super::intents::runnable_shortcuts(&super::apphost::installed())
        .into_iter()
        .map(|s| {
            let folded = fuzzy::fold(&format!("{} {}", s.intent.title, s.app_name));
            let hit = Hit::Shortcut { app_id: s.app_id, app_name: s.app_name, intent: s.intent.id, title: s.intent.title };
            (hit, folded)
        })
        .collect()
}

/// The Settings intents as Shortcuts rows: each opens its pane. Searched by title, listed with its description.
fn settings_intent_rows(hits: &mut BTreeMap<String, Hit>) -> Vec<(search_rank::Item, String)> {
    super::settings_intents::CATALOGUE
        .iter()
        .map(|intent| table_row(hits, format!("settings-intent:{}", intent.id), intent.title, intent.description.to_string(), search_rank::Kind::Settings, Hit::SettingsIntent { id: intent.id, title: intent.title }, fuzzy::fold(intent.title)))
        .collect()
}

/// The wallpaper catalogue as (id, name), for the "Set Wallpaper to" Search rows. The shell sets it whenever its
/// wallpaper list changes (`Shell::set_wallpapers`).
static WALLPAPERS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Records the wallpaper catalogue Search offers rows for.
pub fn set_wallpaper_catalogue(list: Vec<(String, String)>) {
    *WALLPAPERS.lock().unwrap_or_else(|e| e.into_inner()) = list;
}

/// The shell action intents (`settings_actions`) Search lists: the Wi-Fi scan, the two Wi-Fi switch states, and one
/// "Set Wallpaper to" row per picture in the catalogue.
fn settings_action_rows(hits: &mut BTreeMap<String, Hit>) -> Vec<(search_rank::Item, String)> {
    use super::settings_actions::SettingsIntent;
    let fixed: [(&str, &str, &str, SettingsIntent); 4] = [
        ("wifi-scan", "Scan for Wi-Fi Networks", "Look for Wi-Fi networks nearby", SettingsIntent::WifiScan),
        ("wifi-on", "Turn Wi-Fi On", "Switch the Wi-Fi radio on", SettingsIntent::WifiPower(true)),
        ("wifi-off", "Turn Wi-Fi Off", "Switch the Wi-Fi radio off", SettingsIntent::WifiPower(false)),
        ("lock-screen", "Lock Screen", "Lock the screen now", SettingsIntent::LockScreen),
    ];
    let mut rows: Vec<(search_rank::Item, String)> = fixed
        .into_iter()
        .map(|(key, title, text, intent)| table_row(hits, format!("settings-action:{key}"), title, text.to_string(), search_rank::Kind::Settings, Hit::SettingsAction { intent, title: title.to_string() }, fuzzy::fold(title)))
        .collect();
    let pictures = WALLPAPERS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for (id, name) in pictures {
        let title = format!("Set Wallpaper to {name}");
        let folded = fuzzy::fold(&title);
        let hit = Hit::SettingsAction { intent: SettingsIntent::SetWallpaper(id.clone()), title: title.clone() };
        rows.push(table_row(hits, format!("settings-action:wallpaper:{id}"), &title, "Use this picture as the desktop wallpaper".to_string(), search_rank::Kind::Settings, hit, folded));
    }
    rows
}

/// The Now Playing transport commands as Search rows: play or pause, next and previous. Each sends its command to the
/// session's owner, which decides whether it applies (`services::now_playing::send`).
fn media_transport_rows(hits: &mut BTreeMap<String, Hit>) -> Vec<(search_rank::Item, String)> {
    super::media_intents::TRANSPORT
        .iter()
        .map(|&(id, title, command)| {
            let hit = Hit::MediaTransport { command, title: title.to_string() };
            table_row(hits, format!("media-transport:{id}"), title, "Control the playing media".to_string(), search_rank::Kind::Settings, hit, fuzzy::fold(title))
        })
        .collect()
}

/// The row `step` places from `cur` in `rows` (section, row pairs in list order), clamped to the ends.
/// With no row selected, Down takes the first and Up the last. None when there are no rows.
fn step_row(rows: &[(usize, usize)], cur: Option<(usize, usize)>, step: isize) -> Option<(usize, usize)> {
    let last = rows.len().checked_sub(1)? as isize;
    let next = match cur.and_then(|c| rows.iter().position(|r| *r == c)) {
        Some(i) => (i as isize + step).clamp(0, last),
        None if step > 0 => 0,
        None => last,
    };
    Some(rows[next as usize])
}

#[cfg(test)]
mod step_row_tests {
    use super::step_row;

    #[test]
    fn arrows_walk_the_rows_and_stop_at_the_ends() {
        let rows = [(0, 0), (0, 1), (1, 0)];
        assert_eq!(step_row(&rows, None, 1), Some((0, 0)));
        assert_eq!(step_row(&rows, None, -1), Some((1, 0)));
        assert_eq!(step_row(&rows, Some((0, 0)), 1), Some((0, 1)));
        assert_eq!(step_row(&rows, Some((1, 0)), 1), Some((1, 0)));
        assert_eq!(step_row(&rows, Some((0, 0)), -1), Some((0, 0)));
    }

    #[test]
    fn no_rows_means_no_selection() {
        assert_eq!(step_row(&[], None, 1), None);
        assert_eq!(step_row(&[], Some((0, 0)), -1), None);
    }
}

#[cfg(test)]
mod symbol_and_line_tests {
    use super::*;
    use super::super::settings_actions::SettingsIntent as Action;

    #[test]
    fn settings_results_draw_the_symbol_their_pane_has_in_settings() {
        assert_eq!(result_symbol(&Hit::Settings("Wi-Fi")), Some(Symbol::Tile(Icon::Wifi)));
        assert_eq!(result_symbol(&Hit::Settings("About")), Some(Symbol::Glyph(Glyph::About)));
        assert_eq!(result_symbol(&Hit::Settings("Do Not Disturb")), Some(Symbol::Tile(Icon::Focus)));
        assert_eq!(pane_symbol(PageId::Wallpaper), Symbol::Tile(Icon::Wallpaper));
        assert_eq!(pane_symbol(PageId::Keyboard), Symbol::Glyph(Glyph::Keyboard));
        assert_eq!(pane_symbol(PageId::Root), Symbol::Tile(Icon::General), "an unnamed pane keeps the general symbol");
        // every Settings page and every Settings intent has a symbol: no blank square is left
        for page in settings::PAGES {
            assert!(result_symbol(&Hit::Settings(page)).is_some(), "{page}");
        }
        for intent in super::super::settings_intents::CATALOGUE {
            assert!(result_symbol(&Hit::SettingsIntent { id: intent.id, title: intent.title }).is_some(), "{}", intent.id);
        }
    }

    #[test]
    fn actions_and_transport_commands_have_a_symbol_and_other_results_use_their_app() {
        let wallpaper = Hit::SettingsAction { intent: Action::SetWallpaper("dusk".into()), title: "Set Wallpaper to Dusk".into() };
        assert_eq!(result_symbol(&wallpaper), Some(Symbol::Tile(Icon::Wallpaper)));
        assert_eq!(result_symbol(&Hit::SettingsAction { intent: Action::WifiPower(true), title: "x".into() }), Some(Symbol::Tile(Icon::Wifi)));
        assert_eq!(result_symbol(&Hit::SettingsAction { intent: Action::LockScreen, title: "x".into() }), Some(Symbol::Tile(Icon::Passcode)));
        assert!(result_symbol(&Hit::App(AppId::Files)).is_none(), "an app result is drawn with the app's icon");
        assert!(result_symbol(&Hit::Terminal("x".into())).is_none());
    }

    #[test]
    fn rows_have_a_title_and_a_second_line_saying_what_they_are() {
        let name = |_: AppId| "Files".to_string();
        assert_eq!(row_text(&Hit::App(AppId::Files), name), ("Files".to_string(), "Application".to_string()));
        assert_eq!(row_text(&Hit::Settings("Wi-Fi"), name), ("Wi-Fi".to_string(), "Settings".to_string()));
        let file = Hit::File { path: PathBuf::from("/home/u/Downloads/photo.jpg"), name: "photo.jpg".into() };
        assert_eq!(row_text(&file, name), ("photo.jpg".to_string(), "Image \u{b7} Downloads".to_string()));
        let shortcut = Hit::Shortcut { app_id: "a.b".into(), app_name: "Notes".into(), intent: "new".into(), title: "New Note".into() };
        assert_eq!(row_text(&shortcut, name), ("New Note".to_string(), "Shortcut \u{b7} Notes".to_string()));
        let wallpaper = Hit::SettingsAction { intent: Action::SetWallpaper("dusk".into()), title: "Set Wallpaper to Dusk".into() };
        assert_eq!(row_text(&wallpaper, name).1, "Use this picture as the desktop wallpaper");
        assert_eq!(row_text(&Hit::Answer("4".into()), name), ("= 4".to_string(), "Calculator".to_string()));
        assert_eq!(row_text(&Hit::Terminal("ls".into()), name), ("Search in Terminal".to_string(), "ls".to_string()));
        let indexed = Hit::Indexed { app_id: "a".into(), unique_id: "1".into(), domain: String::new(), title: "Q1 report".into(), kind: "PDF document".into(), thumb: String::new() };
        assert_eq!(row_text(&indexed, name), ("Q1 report".to_string(), "PDF document".to_string()));
        let intent = Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" };
        assert_eq!(row_text(&intent, name), ("Open Keyboard".to_string(), "Show the keyboard settings".to_string()));
    }

    #[test]
    fn one_chip_per_kind_of_result_and_none_for_the_best_match_or_the_actions() {
        let g = |title: &'static str, n: usize| Group { title, hits: (0..n).map(|i| Hit::City(if i == 0 { "A" } else { "B" })).collect() };
        let groups = vec![g("Top Hit", 1), g("Applications", 2), g("Settings", 1), g("Shortcuts", 3), g("Applications", 1), g("Files", 0), g("Show in Folder", 1), g("Search in Apps", 1)];
        assert_eq!(chips_of(&groups), vec!["Applications", "Settings", "Shortcuts"]);
        assert!(chips_of(&[]).is_empty());
    }
}

/// A titled section of results.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: &'static str,
    pub hits: Vec<Hit>,
}

/// Ranks every source for `query`: the best match overall as the Top Hit,
/// then up to three per source (apps, Settings, Clock, Deildu) and the
/// Terminal action last. Empty for a blank query.
pub fn results(query: &str, deildu: &[String]) -> Vec<Group> {
    results_with_contacts(query, deildu, &[])
}

fn results_with_contacts(query: &str, deildu: &[String], contacts: &[super::contactstore::Contact]) -> Vec<Group> {
    let folded: Vec<String> = deildu.iter().map(|t| fuzzy::fold(t)).collect();
    let contacts_folded: Vec<[String; 3]> = contacts.iter().map(folded_fields).collect();
    results_folded(query, deildu, &folded, contacts, &contacts_folded)
}

/// The name, phone and email-and-company of a contact, folded for matching (kept by the caller between keystrokes). The
/// company sits with the email so a person is found by where they work; notes and the address stay out of Search.
fn folded_fields(contact: &super::contactstore::Contact) -> [String; 3] {
    let mail_and_company = format!("{} {}", contact.email, contact.company);
    [contact.name.as_str(), contact.phone.as_str(), mail_and_company.as_str()].map(fuzzy::fold)
}

/// Candidates of the sources that never change while the shell runs, folded once.
struct Fixed {
    #[allow(dead_code)]
    settings: Vec<(Hit, String)>,
    #[allow(dead_code)]
    cities: Vec<(Hit, String)>,
    aliases: Vec<(Hit, String)>,
}

fn fixed() -> &'static Fixed {
    static FIXED: std::sync::OnceLock<Fixed> = std::sync::OnceLock::new();
    FIXED.get_or_init(|| Fixed {
        settings: settings::PAGES.iter().map(|p| (Hit::Settings(p), fuzzy::fold(p))).collect(),
        cities: CITIES.iter().map(|c| (Hit::City(c.name), fuzzy::fold(c.name))).collect(),
        aliases: quicksearch_math::settings_aliases()
            .iter()
            .filter_map(|(k, p)| settings::PAGES.iter().find(|n| *n == p).map(|p| (Hit::Settings(p), fuzzy::fold(k))))
            .collect(),
    })
}

/// Deildu titles, contacts, files and calendar events as a query searches them, each with its folded form (`fuzzy::fold`,
/// or the folded fields for a contact), kept by the caller between keystrokes.
struct Corpus<'a> {
    deildu: &'a [String],
    deildu_folded: &'a [String],
    contacts: &'a [super::contactstore::Contact],
    contacts_folded: &'a [[String; 3]],
    files: &'a [crate::files::fs::Entry],
    files_folded: &'a [String],
    events: &'a [super::calendar::store::CalendarEvent],
}

/// The usage key of an app: its opens are counted under it (see `QuickSearch::note_open`).
fn app_usage_key(app: AppId) -> String {
    format!("app:{}", app_key(app))
}

/// The usage key of a Settings page.
fn settings_usage_key(page: &str) -> String {
    format!("settings:{page}")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long a query waits for its sources before the panel shows what has come in (kept small: it runs on the UI
/// thread inside a frame). The rest streams in afterwards.
const RUN_GRACE: std::time::Duration = std::time::Duration::from_millis(4);
/// The longest a query waits for its slowest source before it goes on without it.
const RUN_DEADLINE: std::time::Duration = std::time::Duration::from_millis(300);
/// The wait of a query merged at once (`results`, used by the look-up popover and by tests).
const SYNC_DEADLINE: std::time::Duration = std::time::Duration::from_millis(300);

/// One section of results as a search source: its rows (the item the ranker sees, and the folded text the fuzzy matcher
/// tests). It owns its rows, so a query can run it on its own thread. The hit a row opens is kept by `build_tables`.
struct Table {
    name: &'static str,
    rows: Vec<(search_rank::Item, String)>,
}

impl search_rank::Source for Table {
    fn name(&self) -> &'static str {
        self.name
    }

    fn search(&self, query: &str) -> Vec<search_rank::Item> {
        let m = fuzzy::Matcher::new(query);
        self.rows.iter().filter(|(_, folded)| m.rank_folded(folded).is_some()).map(|(item, _)| item.clone()).collect()
    }
}

/// One row of a section: its item, and the hit it opens (kept under the item's id).
fn table_row(hits: &mut BTreeMap<String, Hit>, key: String, title: &str, text: String, kind: search_rank::Kind, hit: Hit, folded: String) -> (search_rank::Item, String) {
    hits.insert(key.clone(), hit);
    (search_rank::Item { id: key, title: title.to_string(), text, kind, modified: 0 }, folded)
}

/// The sections of a query in display order (apps, Settings, clock, Deildu, contacts, shortcuts, files, calendar), and
/// the hit each item id opens.
fn build_tables(c: &Corpus) -> (Vec<Table>, BTreeMap<String, Hit>) {
    let mut hits: BTreeMap<String, Hit> = BTreeMap::new();
    let fx = fixed();
    let apps_rows = apps().iter().map(|a| table_row(&mut hits, app_usage_key(a.id), a.name, String::new(), search_rank::Kind::App, Hit::App(a.id), fuzzy::fold(a.name))).collect();
    let settings_rows = settings::PAGES
        .iter()
        .map(|&page| {
            // the keyword aliases ("brightness", "dark mode") are part of the page's text
            let aliases: Vec<&str> = fx.aliases.iter().filter(|(h, _)| *h == Hit::Settings(page)).map(|(_, f)| f.as_str()).collect();
            let text = aliases.join(" ");
            let folded = format!("{} {}", fuzzy::fold(page), text);
            table_row(&mut hits, settings_usage_key(page), page, text, search_rank::Kind::Settings, Hit::Settings(page), folded)
        })
        .collect();
    let city_rows = CITIES.iter().map(|city| table_row(&mut hits, format!("city:{}", city.name), city.name, String::new(), search_rank::Kind::Other, Hit::City(city.name), fuzzy::fold(city.name))).collect();
    let deildu_rows = c.deildu.iter().zip(c.deildu_folded).map(|(title, f)| table_row(&mut hits, format!("deildu:{title}"), title, String::new(), search_rank::Kind::Other, Hit::Deildu(title.clone()), f.clone())).collect();
    let contact_rows = c
        .contacts
        .iter()
        .zip(c.contacts_folded)
        .map(|(contact, f)| {
            let name = contact.display_name().to_string();
            let text = format!("{} {}", contact.phone, contact.email);
            let hit = Hit::Contact { id: contact.id, name: name.clone() };
            table_row(&mut hits, format!("contact:{}", contact.id), &name, text, search_rank::Kind::Contact, hit, f.join(" "))
        })
        .collect();
    let mut shortcut_rows: Vec<(search_rank::Item, String)> = shortcut_items().into_iter().enumerate().map(|(i, (hit, folded))| table_row(&mut hits, format!("shortcut:{i}"), &folded, String::new(), search_rank::Kind::Other, hit, folded.clone())).collect();
    shortcut_rows.extend(settings_intent_rows(&mut hits));
    shortcut_rows.extend(settings_action_rows(&mut hits));
    shortcut_rows.extend(media_transport_rows(&mut hits));
    let file_rows = c.files.iter().zip(c.files_folded).map(|(e, f)| table_row(&mut hits, format!("file:{}", e.path.display()), &e.name, String::new(), search_rank::Kind::File, Hit::File { path: e.path.clone(), name: e.name.clone() }, f.clone())).collect();
    let event_rows = c
        .events
        .iter()
        .map(|e| {
            let text = format!("{} {}", e.location, e.notes);
            let folded = fuzzy::fold(&format!("{} {}", e.title, e.location));
            table_row(&mut hits, format!("event:{}", e.id), &e.title, text, search_rank::Kind::Other, Hit::Event { title: e.title.clone() }, folded)
        })
        .collect();
    let tables = vec![
        Table { name: "Applications", rows: apps_rows },
        Table { name: "Settings", rows: settings_rows },
        Table { name: "Clock", rows: city_rows },
        Table { name: "Deildu", rows: deildu_rows },
        Table { name: "Contacts", rows: contact_rows },
        Table { name: "Shortcuts", rows: shortcut_rows },
        Table { name: "Files", rows: file_rows },
        Table { name: "Calendar", rows: event_rows },
    ];
    (tables, hits)
}

/// The tables as the sources a query runs on.
fn sources_of(tables: Vec<Table>) -> Vec<Arc<dyn search_rank::Source>> {
    // Settings > Search switches a section off: its source is not queried
    let prefs = super::search_prefs::current();
    tables.into_iter().filter(|t| prefs.source_on(t.name)).map(|t| Arc::new(t) as Arc<dyn search_rank::Source>).collect()
}

/// The calculator answer for the query, as its group.
fn calculator_group(query: &str) -> Option<Group> {
    quicksearch_math::answer(query).map(|a| Group { title: "Calculator", hits: vec![Hit::Answer(a)] })
}

/// Splits a merged query into the Top Hit group, the sections, and the files group (with "Show in Folder" after it).
/// `top` is the hit shown as the top hit; it is taken out of the sections, so no row is listed twice.
fn split_merged(merged: &search_rank::Merged, hits: &BTreeMap<String, Hit>, top: Option<Hit>) -> (Option<Group>, Vec<Group>, Vec<Group>) {
    let top_group = top.clone().map(|h| Group { title: "Top Hit", hits: vec![h] });
    let mut sections = vec![];
    let mut files = vec![];
    for section in &merged.sections {
        let rows: Vec<Hit> = section.hits.iter().filter_map(|r| hits.get(&r.item.id).cloned()).filter(|h| top.as_ref() != Some(h)).collect();
        if rows.is_empty() {
            continue;
        }
        if section.source == "Files" {
            let show = match rows.first() {
                Some(Hit::File { path, name }) => Some(Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: path.parent().map_or_else(|| path.clone(), Path::to_path_buf), name: name.clone() }] }),
                _ => None,
            };
            files.push(Group { title: "Files", hits: rows });
            files.extend(show);
        } else {
            sections.push(Group { title: section.source, hits: rows });
        }
    }
    (top_group, sections, files)
}

/// The groups in display order: the calculator answer, the Top Hit, the sections, the Documents, the files, and the
/// "Search in Apps" action last.
fn compose(query: &str, calc: Option<Group>, top: Option<Group>, sections: Vec<Group>, documents: Vec<Group>, files: Vec<Group>) -> Vec<Group> {
    let mut groups: Vec<Group> = calc.into_iter().chain(top).collect();
    groups.extend(sections);
    groups.extend(documents);
    groups.extend(files);
    groups.push(Group { title: "Search in Apps", hits: vec![Hit::Terminal(query.to_string())] });
    scoped(groups, &super::spotlight_scope::current())
}

/// The category a hit is listed under in Settings > Search, and the path it names when it is a file. `None` for
/// the hits that no switch controls (the Search in Apps action, the Top Hit's own kind, the calculator answer).
fn hit_category(hit: &Hit) -> (Option<super::spotlight_scope::Category>, Option<&Path>) {
    use super::spotlight_scope::Category as C;
    match hit {
        Hit::App(_) | Hit::Shortcut { .. } => (Some(C::Applications), None),
        Hit::Settings(_) | Hit::SettingsIntent { .. } | Hit::SettingsAction { .. } | Hit::MediaTransport { .. } => (Some(C::Settings), None),
        Hit::Contact { .. } => (Some(C::Contacts), None),
        Hit::Event { .. } => (Some(C::CalendarEvents), None),
        Hit::File { path, .. } | Hit::Reveal { folder: path, .. } => (Some(C::Files), Some(path.as_path())),
        Hit::Indexed { .. } => (Some(C::Documents), None),
        Hit::City(_) => (Some(C::Cities), None),
        Hit::Answer(_) => (Some(C::Calculator), None),
        Hit::Deildu(_) | Hit::Terminal(_) => (None, None),
    }
}

/// Drops the hits Settings > Search hides, and any group left empty. A group with no hits stays only when it was
/// empty to begin with (the "Search in Apps" action is never hidden).
fn scoped(groups: Vec<Group>, scope: &super::spotlight_scope::Scope) -> Vec<Group> {
    groups
        .into_iter()
        .filter_map(|mut g| {
            let had = !g.hits.is_empty();
            g.hits.retain(|h| {
                let (cat, path) = hit_category(h);
                !cat.is_some_and(|c| scope.hides(c, path))
            });
            (!had || !g.hits.is_empty()).then_some(g)
        })
        .collect()
}

/// `deildu_folded` holds `fuzzy::fold` of each `deildu` title and `contacts_folded` the `folded_fields` of each contact,
/// both kept by the caller between keystrokes. The sources are merged at once here; the Search panel streams them.
fn results_folded(query: &str, deildu: &[String], deildu_folded: &[String], contacts: &[super::contactstore::Contact], contacts_folded: &[[String; 3]]) -> Vec<Group> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    let corpus = Corpus { deildu, deildu_folded, contacts, contacts_folded, files: &[], files_folded: &[], events: &[] };
    let (tables, hits) = build_tables(&corpus);
    let mut run = search_rank::Query::start(query, &sources_of(tables), SYNC_DEADLINE);
    run.wait();
    let merged = run.merge(PER_GROUP, &search_rank::Usage::default(), now_secs(), &search_rank::NoSemantic);
    let top = merged.top.as_ref().and_then(|t| hits.get(&t.item.id).cloned());
    let (top, sections, files) = split_merged(&merged, &hits, top);
    compose(query, calculator_group(query), top, sections, vec![], files)
}

impl QuickSearch {
    /// Starts `query` on every source. Waits a moment for the quick ones, so the first frame usually has them; the
    /// rest come in through `run_groups` as they answer.
    fn start_run(&mut self, query: &str) {
        let corpus = Corpus {
            deildu: &self.deildu_titles,
            deildu_folded: &self.deildu_folded,
            contacts: &self.contacts,
            contacts_folded: &self.contacts_folded,
            files: &self.files,
            files_folded: &self.files_folded,
            events: &self.events,
        };
        let (tables, hits) = build_tables(&corpus);
        let mut run = search_rank::Query::start(query, &sources_of(tables), RUN_DEADLINE);
        run.wait_for(RUN_GRACE);
        self.run = Some(run);
        self.run_hits = hits;
        self.run_query = query.to_string();
        self.frozen_top = None;
        self.shown.clear();
    }

    /// Takes the answers that have arrived and merges them into (Top Hit, sections, files). The top hit is frozen the
    /// first time one is shown for the query, so a late source cannot move the row the person is looking at.
    fn run_groups(&mut self) -> (Option<Group>, Vec<Group>, Vec<Group>) {
        let Some(run) = self.run.as_mut() else { return (None, vec![], vec![]) };
        run.poll();
        let guard = OPENS.lock().unwrap_or_else(|e| e.into_inner());
        let empty = search_rank::Usage::default();
        let usage = guard.as_ref().unwrap_or(&empty);
        let merged = run.merge(PER_GROUP, usage, now_secs(), &search_rank::NoSemantic);
        drop(guard);
        if self.frozen_top.is_none() {
            let hits = &self.run_hits;
            self.frozen_top = merged.top.as_ref().and_then(|t| hits.get(&t.item.id).cloned());
        }
        split_merged(&merged, &self.run_hits, self.frozen_top.clone())
    }

    /// The result groups for the query, as they stand now. Groups already on screen keep their order, and new ones
    /// follow them (before "Search in Apps"), so a late source adds rows below what is shown.
    fn groups_for(&mut self, query: &str) -> Vec<Group> {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.run = None;
            self.run_query.clear();
            self.frozen_top = None;
            self.shown.clear();
            return vec![];
        }
        if query != self.run_query {
            self.start_run(&query);
        }
        let (top, sections, files) = self.run_groups();
        let groups = compose(&query, calculator_group(&query), top, sections, indexed_groups(&query), files);
        let groups = self.keep_shown_order(groups);
        self.shown = groups.iter().map(|g| g.title).collect();
        groups
    }

    /// The groups already shown keep their order; new groups follow them, and "Search in Apps" stays last.
    fn keep_shown_order(&self, mut groups: Vec<Group>) -> Vec<Group> {
        let tail = groups.iter().position(|g| g.title == "Search in Apps").map(|i| groups.remove(i));
        let mut out: Vec<Group> = vec![];
        for title in &self.shown {
            if let Some(i) = groups.iter().position(|g| g.title == *title) {
                out.push(groups.remove(i));
            }
        }
        out.extend(groups);
        out.extend(tail);
        out
    }
}

/// The key an indexed result's opens are counted under.
fn indexed_key(app_id: &str, unique_id: &str) -> String {
    format!("{app_id}/{unique_id}")
}

/// The section title of an app's indexed items: the app's name. Names are kept for the rest of the run (an app name is
/// a few bytes and there are at most a few dozen installed apps), so a section title stays a `&'static str`.
fn app_section_title(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = names.iter().copied().find(|known| *known == name) {
        return known;
    }
    let kept: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(kept);
    kept
}

/// One section per app that has matching items, titled with the app's name. Items are ordered by the hybrid ranker
/// (`bifrost_ui::search_rank`) across all apps at once: title and word match, recency of indexing, and local opens.
/// Each section keeps its first `PER_GROUP` items in ranked order, and sections follow the best item of each app.
/// Matches the ranker does not score keep their index order after the ranked ones.
fn indexed_groups(query: &str) -> Vec<Group> {
    // Settings > Search: the Documents source or an app's items switched off are left out
    let prefs = super::search_prefs::current();
    if !prefs.source_on("Documents") {
        return vec![];
    }
    let found: Vec<super::spotindex::Found> = super::spotindex::search(query, PER_GROUP * 4).into_iter().filter(|f| prefs.app_on(&f.app_id)).collect();
    if found.is_empty() {
        return vec![];
    }
    let names: BTreeMap<String, String> = super::apphost::installed().iter().map(|a| (a.manifest.id.clone(), a.manifest.name.clone())).collect();
    let items: Vec<search_rank::Item> = found
        .iter()
        .map(|f| search_rank::Item {
            id: indexed_key(&f.app_id, &f.unique_id),
            title: f.title.clone(),
            text: f.text.clone(),
            kind: search_rank::Kind::Document,
            modified: f.modified,
        })
        .collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let guard = OPENS.lock().unwrap_or_else(|e| e.into_inner());
    let empty = search_rank::Usage::default();
    let usage = guard.as_ref().unwrap_or(&empty);
    let order = search_rank::order(query, &items, usage, now, &search_rank::NoSemantic);
    let mut groups: Vec<Group> = vec![];
    for i in order {
        let f = found[i].clone();
        let title = app_section_title(names.get(&f.app_id).map_or(f.app_id.as_str(), String::as_str));
        let hit = Hit::Indexed {
            kind: f.kind.map_or(String::new(), |k| uttype::description(k).to_string()),
            thumb: f.thumbnail,
            app_id: f.app_id,
            unique_id: f.unique_id,
            domain: f.domain,
            title: f.title,
        };
        match groups.iter_mut().find(|g| g.title == title) {
            Some(g) => {
                if g.hits.len() < PER_GROUP {
                    g.hits.push(hit);
                }
            }
            None => groups.push(Group { title, hits: vec![hit] }),
        }
    }
    groups
}

/// A release title from a torrent name: dots and underscores become
/// spaces and the quality tags are dropped.
fn title_of(name: &str) -> String {
    let spaced = name.replace(['.', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    let t = deildu_rust::listings::media_title(&spaced);
    if t.is_empty() {
        spaced
    } else {
        t
    }
}

/// Titles of the torrents Deildu has cached under `dir/cache`, sorted and
/// deduplicated; nothing is fetched.
pub fn cached_titles(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("cache")) else { return vec![] };
    let mut titles: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "torrent"))
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| deildu_rust::torrent::Torrent::parse(&b).ok())
        .map(|t| title_of(&t.name))
        .filter(|t| !t.is_empty())
        .collect();
    titles.sort();
    titles.dedup();
    titles
}

/// What the sources worker read: a fresh list only where its file changed (`None` where it did not),
/// with the stamps those reads were taken at.
struct SourcesRead {
    contacts: Option<Vec<super::contactstore::Contact>>,
    /// `folded_fields` of each contact, built on the worker with the contacts
    contacts_folded: Option<Vec<[String; 3]>>,
    contacts_stamp: Option<(PathBuf, std::time::SystemTime)>,
    titles: Option<Vec<String>>,
    deildu_stamp: Option<(PathBuf, std::time::SystemTime)>,
    events: Option<Vec<super::calendar::store::CalendarEvent>>,
    calendar_stamp: Option<(PathBuf, std::time::SystemTime)>,
}

/// What a touch on the home screen turned out to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pull {
    Undecided,
    /// sideways (or upwards): the pager's
    Page,
    /// mostly vertical and downwards: Search's
    Down,
}

/// Decides, once the finger has travelled `slop` px, whether a home-screen
/// touch pulls Search down or pages sideways.
#[derive(Clone, Copy, Debug)]
pub struct PullDown {
    x0: f32,
    y0: f32,
}

impl PullDown {
    pub fn new(x: f32, y: f32) -> Self {
        PullDown { x0: x, y0: y }
    }

    pub fn moved(&self, x: f32, y: f32, slop: f32) -> Pull {
        let (dx, dy) = (x - self.x0, y - self.y0);
        if dx.abs() < slop && dy.abs() < slop {
            Pull::Undecided
        } else if dy > 0.0 && dy >= dx.abs() {
            Pull::Down
        } else {
            Pull::Page
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum State {
    Hidden,
    Dragging(Drag),
    Settling { from: f32, to: f32, start: f64 },
    Shown,
}

/// Something a finger can land on while the card is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Keyboard,
    Field,
    Clear,
    Suggestion(usize),
    /// a recent search
    Search(usize),
    /// (section, row)
    Row(usize, usize),
    /// a filter chip above the results (Golden Gate): the index into the chips shown
    Chip(usize),
    Outside,
}

#[derive(Clone, Copy, Debug)]
enum Touch {
    Keyboard,
    Target(Target),
    /// a tap dismisses, a drag up pulls the card away
    Outside(Drag),
}

/// A px rectangle (x, y, w, h).
pub type Rect = (f32, f32, f32, f32);

/// A rendered section: header and rows.
struct Section {
    header: Image,
    rows: Vec<(Hit, Image)>,
    /// the second line of each row (Golden Gate), when it has one
    subs: Vec<Option<Image>>,
}

fn app_key(app: AppId) -> String {
    format!("{app:?}").to_lowercase()
}

fn app_named(key: &str) -> Option<AppId> {
    apps().iter().map(|a| a.id).find(|&id| app_key(id) == key)
}

pub struct QuickSearch {
    state: State,
    touch: Option<Touch>,
    pull: Option<PullDown>,
    field: TextField,
    keyboard: Keyboard,
    prefs: Prefs,
    /// Golden Gate presentation (desktop), see `MAC`
    mac: bool,
    /// the screen's width in px: the Golden Gate card is `MAC_CARD_FRACTION` of it
    screen_w: f32,
    /// the kind of result the list is filtered to (the title of a group), and the chips shown with their text
    chip: Option<&'static str>,
    chips: Vec<(&'static str, Image)>,
    /// the `k` the field and the chips were made for
    made_k: f32,
    recents: Vec<AppId>,
    /// recent queries, newest first, and their rendered rows
    searches: Vec<String>,
    search_imgs: Vec<Image>,
    /// decoded thumbnails of indexed results: (app/path key, icon size in pixels, image; None when unreadable)
    thumbs: Vec<(String, usize, Option<Image>)>,
    /// pictures of the files in the file rows, drawn by the system thumbnail providers on worker threads
    file_thumbs: super::search_thumbs::FileThumbs,
    searches_header: Image,
    /// (app, 60 pt icon, 29 pt icon, label)
    icons: Vec<(AppId, Image, Image, Image)>,
    /// the app icons resized to the Golden Gate row's icon square, so they match the pane tiles; rebuilt by `search`
    /// whenever the square changes (another scale) or the icons were reset
    row_icons: Vec<(AppId, Image)>,
    sections: Vec<Section>,
    /// the row the arrow keys moved to: (section, row); Return opens it instead of the Top Hit
    selected: Option<(usize, usize)>,
    /// Golden Gate: the top of the Dock in px while it stands along the bottom edge, None when it does not limit the card
    dock_top: Option<f32>,
    /// Golden Gate: the lowest y in px the results card may reach (a margin above the Dock or the screen's bottom)
    max_bottom: f32,
    /// Golden Gate: how far the rows are scrolled up inside the card, in px, and the query the offset belongs to
    scroll: f32,
    scroll_query: String,
    /// touches of the drag emulated for a wheel notch that are still to be dropped: (count, until ms)
    wheel_swallow: (u8, f64),
    deildu_titles: Vec<String>,
    /// `fuzzy::fold` of each Deildu title, kept so keystrokes do not re-fold them
    deildu_folded: Vec<String>,
    contacts: Vec<super::contactstore::Contact>,
    /// `folded_fields` of each contact
    contacts_folded: Vec<[String; 3]>,
    /// files under the Files root, newest first, and their folded names
    files: Vec<crate::files::fs::Entry>,
    files_folded: Vec<String>,
    /// the background file scan started at open, until its result is taken
    files_rx: Option<std::sync::mpsc::Receiver<Vec<crate::files::fs::Entry>>>,
    /// the background read of the contacts and Deildu titles, until its result is taken
    sources_rx: Option<std::sync::mpsc::Receiver<SourcesRead>>,
    /// the root the file list was scanned under, and when that scan finished
    files_root: Option<PathBuf>,
    files_at: Option<std::time::Instant>,
    /// modification times the contacts and Deildu title caches were read at
    contacts_stamp: Option<(PathBuf, std::time::SystemTime)>,
    deildu_stamp: Option<(PathBuf, std::time::SystemTime)>,
    /// where the indexed-result open counts are saved (beside quicksearch.ini); None until `load`
    usage_path: Option<PathBuf>,
    /// every app launch with its time, for the prediction of the next app; saved at `launches_path`
    launches: bifrost_ui::app_prediction::LaunchLedger,
    launches_path: Option<PathBuf>,
    /// the query being shown: its sources (started by `start_run`) and what they answer with
    run: Option<search_rank::Query>,
    /// the query `run` was started for ("" when none)
    run_query: String,
    /// the hit each item id of `run` opens
    run_hits: BTreeMap<String, Hit>,
    /// the top hit of `run_query`, kept once shown so a late source cannot replace it
    frozen_top: Option<Hit>,
    /// the titles of the groups on screen, in order
    shown: Vec<&'static str>,
    /// the calendar events, read on a worker (see `refresh_sources`)
    events: Vec<super::calendar::store::CalendarEvent>,
    calendar_stamp: Option<(PathBuf, std::time::SystemTime)>,
    suggestions: Image,
    chevron: Image,
}

impl QuickSearch {
    /// Taps on the field raise the on-screen keyboard on the bifrost only; the desktop types on its own keyboard.
    fn show_keyboard(&mut self, t: f64) {
        if !self.mac {
            self.keyboard.set_shown(true, t);
        }
    }

    fn m(&self) -> &'static Metrics {
        if self.mac {
            &MAC
        } else {
            &BIFROST
        }
    }

    /// Chooses the presentation before the card opens; the field placeholder follows it.
    pub fn set_mac(&mut self, ui: &Ui, mac: bool) {
        let k = if mac { self.mac_k(ui) } else { 1.0 };
        if self.mac == mac && (self.made_k - k).abs() < 0.005 {
            return;
        }
        self.mac = mac;
        self.made_k = k;
        self.chip = None;
        self.field = if mac { TextField::new(ui, "Search", MAC_LARGE_TITLE_PT * k, false, QUERY_MAX) } else { TextField::new(ui, "Search", FIELD_TEXT_PT, false, QUERY_MAX) };
        self.search_imgs.clear();
    }

    /// How much the Golden Gate layout is enlarged: the card is `MAC_CARD_FRACTION` of the screen's width, and when that is
    /// wider than the card's point size everything in it (the field, the rows, the text) grows by the same factor.
    fn mac_k(&self, ui: &Ui) -> f32 {
        (self.screen_w * MAC_CARD_FRACTION / ui.pt(MAC.card_w).max(1.0)).clamp(1.0, MAC_K_MAX)
    }

    /// The layout factor of the presentation on show: `mac_k` for Golden Gate, 1 on the bifrost.
    fn k(&self, ui: &Ui) -> f32 {
        if self.mac {
            self.mac_k(ui)
        } else {
            1.0
        }
    }

    /// `v` points, enlarged by the layout factor, in px.
    fn px(&self, ui: &Ui, v: f32) -> f32 {
        ui.pt(v * self.k(ui))
    }

    pub fn new(ui: &Ui, w: f32, s: f32) -> Self {
        QuickSearch {
            state: State::Hidden,
            touch: None,
            pull: None,
            field: TextField::new(ui, "Search", FIELD_TEXT_PT, false, QUERY_MAX),
            keyboard: {
                let mut k = Keyboard::new_hidden(w, s);
                k.set_predictive(true);
                k
            },
            prefs: Prefs::memory(),
            mac: false,
            screen_w: w,
            chip: None,
            chips: vec![],
            made_k: 1.0,
            recents: vec![],
            searches: vec![],
            search_imgs: vec![],
            searches_header: ui.text("Recent Searches", 20.0, Weight::Semibold, WHITE),
            icons: vec![],
            row_icons: vec![],
            sections: vec![],
            selected: None,
            dock_top: None,
            max_bottom: f32::MAX,
            scroll: 0.0,
            scroll_query: String::new(),
            wheel_swallow: (0, 0.0),
            thumbs: vec![],
            file_thumbs: super::search_thumbs::FileThumbs::default(),
            deildu_titles: vec![],
            deildu_folded: vec![],
            contacts: vec![],
            contacts_folded: vec![],
            files: vec![],
            files_folded: vec![],
            files_rx: None,
            sources_rx: None,
            files_root: None,
            files_at: None,
            contacts_stamp: None,
            deildu_stamp: None,
            usage_path: None,
            launches: Default::default(),
            launches_path: None,
            run: None,
            run_query: String::new(),
            run_hits: BTreeMap::new(),
            frozen_top: None,
            shown: vec![],
            events: vec![],
            calendar_stamp: None,
            suggestions: ui.text("Assistant Suggestions", 20.0, Weight::Semibold, WHITE),
            chevron: ui.text("›", 22.0, Weight::Semibold, SECONDARY),
        }
    }

    /// The screen turned: the keyboard takes the new width.
    pub fn resize(&mut self, w: f32) {
        self.screen_w = w;
        self.keyboard.resize(w);
    }

    /// Reads the recents from `path` (an ini file) and saves there from now on.
    pub fn load(&mut self, path: impl Into<PathBuf>) {
        let path: PathBuf = path.into();
        let usage_path = path.with_file_name(USAGE_FILE);
        let opens = std::fs::read_to_string(&usage_path).map(|t| search_rank::Usage::from_text(&t)).unwrap_or_default();
        if let Ok(mut g) = OPENS.lock() {
            *g = Some(opens);
        }
        self.usage_path = Some(usage_path);
        let launches_path = path.with_file_name(LAUNCHES_FILE);
        self.launches = std::fs::read_to_string(&launches_path).map(|t| bifrost_ui::app_prediction::LaunchLedger::from_text(&t)).unwrap_or_default();
        self.launches_path = Some(launches_path);
        self.prefs = Prefs::load(path);
        self.recents = self.prefs.get("recents").unwrap_or("").split(',').filter_map(app_named).take(RECENTS).collect();
        self.searches = self.prefs.get("searches").unwrap_or("").split(',').map(str::trim).filter(|q| !q.is_empty()).map(String::from).take(SEARCHES).collect();
        self.search_imgs.clear();
    }

    /// Counts an open of an indexed result (`indexed_key`) and saves the counts beside quicksearch.ini. Local only.
    pub fn note_open(&mut self, key: &str) {
        let text = match OPENS.lock() {
            Ok(mut g) => {
                let u = g.get_or_insert_with(Default::default);
                u.bump(key);
                u.to_text()
            }
            Err(_) => return,
        };
        if let Some(path) = &self.usage_path {
            if let Err(e) = std::fs::write(path, text) {
                eprintln!("springboard: {USAGE_FILE}: {e}");
            }
        }
    }

    /// Records a launch: `app` becomes the first suggestion.
    pub fn note_launch(&mut self, app: AppId) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.launches.record(&app_key(app), now);
        if let Some(path) = &self.launches_path {
            if let Err(e) = std::fs::write(path, self.launches.to_text()) {
                eprintln!("springboard: {LAUNCHES_FILE}: {e}");
            }
        }
        self.recents.retain(|&a| a != app);
        self.recents.insert(0, app);
        self.recents.truncate(RECENTS);
        let text = self.recents.iter().map(|&a| app_key(a)).collect::<Vec<_>>().join(",");
        if let Err(e) = self.prefs.set("recents", &text) {
            eprintln!("springboard: quicksearch.ini: {e}");
        }
    }

    /// The apps most likely to be opened next at Unix time `now` (`tz_offset` seconds east of UTC), best first: the launch
    /// history ranked by frequency, time of day and what usually follows the last app (`bifrost_ui::app_prediction`).
    pub fn predicted_apps(&self, now: u64, tz_offset: i64, limit: usize) -> Vec<AppId> {
        let previous = self.recents.first().map(|&a| app_key(a));
        let open = previous.as_deref().into_iter().collect::<Vec<_>>();
        self.launches.predict(now, tz_offset, previous.as_deref(), &open, limit).iter().filter_map(|k| app_named(k)).collect()
    }

    /// Remembers the current query as a recent search (call before a result opens).
    pub fn note_search(&mut self) {
        let q = self.field.text().replace([',', '\n'], " ").trim().to_string();
        if q.is_empty() {
            return;
        }
        self.searches.retain(|s| *s != q);
        self.searches.insert(0, q);
        self.searches.truncate(SEARCHES);
        self.search_imgs.clear();
        if let Err(e) = self.prefs.set("searches", &self.searches.join(",")) {
            eprintln!("springboard: quicksearch.ini: {e}");
        }
    }

    /// The recent searches, newest first.
    pub fn searches(&self) -> &[String] {
        &self.searches
    }

    /// The most recently opened apps, newest first.
    pub fn recents(&self) -> &[AppId] {
        &self.recents
    }

    /// Forgets the app icons, so the next pull takes them from the home screen's tiles again (an icon style change
    /// or an install).
    pub fn clear_icons(&mut self) {
        self.icons.clear();
        self.row_icons.clear();
    }

    /// Icons for the suggestion row and the result rows, from the home
    /// screen's tiles; built once by the shell before the first show.
    pub fn set_icons(&mut self, ui: &Ui, icons: Vec<(AppId, &Image)>) {
        let (big, small) = (ui.pt(SUGGEST_ICON_PT).round() as usize, ui.pt(ROW_ICON_PT).round() as usize);
        self.icons = icons
            .into_iter()
            .map(|(app, img)| {
                let name = apps().iter().find(|a| a.id == app).map_or("", |a| a.name);
                let label = ui.text(&ui.fit(name, 12.0, ui.pt(SUGGEST_PITCH_PT - 4.0)), 12.0, Weight::Regular, WHITE);
                (app, img.resized(big, big), img.resized(small, small), label)
            })
            .collect();
        self.row_icons.clear();
    }

    pub fn has_icons(&self) -> bool {
        !self.icons.is_empty()
    }

    /// The Deildu titles searched from now on.
    pub fn set_deildu_titles(&mut self, titles: Vec<String>) {
        self.deildu_folded = titles.iter().map(|t| fuzzy::fold(t)).collect();
        self.deildu_titles = titles;
    }

    /// Re-reads the contacts and cached Deildu titles only when their files changed, on a worker thread (a
    /// cache can hold many torrents). Scans `root` for files on a worker thread when the list is older than
    /// `FILES_MAX_AGE` (or was scanned under another root). The list shown meanwhile is the last one, so the card
    /// opens with results; a new list fills in on a later tick.
    pub fn refresh_sources(&mut self, contacts: &Path, deildu_dir: &Path, calendar: &Path, root: PathBuf) {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let contacts_stamp = mtime(contacts).map(|t| (contacts.to_path_buf(), t));
        let want_contacts = contacts_stamp.is_none() || contacts_stamp != self.contacts_stamp;
        let cache = deildu_dir.join("cache");
        let deildu_stamp = mtime(&cache).map(|t| (cache.clone(), t));
        let want_titles = deildu_stamp.is_none() || deildu_stamp != self.deildu_stamp;
        let calendar_stamp = mtime(calendar).map(|t| (calendar.to_path_buf(), t));
        let want_events = calendar_stamp.is_some() && calendar_stamp != self.calendar_stamp;
        if (want_contacts || want_titles || want_events) && self.sources_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let contacts = contacts.to_path_buf();
            let deildu_dir = deildu_dir.to_path_buf();
            let calendar = calendar.to_path_buf();
            crate::bare::sys::spawn_named("quicksearch-sources", move || {
                let loaded = want_contacts.then(|| super::contactstore::load_checked(&contacts).unwrap_or_default());
                let contacts_folded: Option<Vec<[String; 3]>> = loaded.as_ref().map(|c| c.iter().map(folded_fields).collect());
                let _ = tx.send(SourcesRead {
                    contacts: loaded,
                    contacts_folded,
                    contacts_stamp,
                    titles: want_titles.then(|| cached_titles(&deildu_dir)),
                    deildu_stamp,
                    events: want_events.then(|| super::calendar::store::load(&calendar).unwrap_or_default()),
                    calendar_stamp,
                });
            });
            self.sources_rx = Some(rx);
        }
        let fresh = self.files_root.as_ref() == Some(&root) && self.files_at.is_some_and(|at| at.elapsed() < FILES_MAX_AGE);
        if self.files_rx.is_none() && !fresh {
            self.files_root = Some(root.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            crate::bare::sys::spawn_named("quicksearch-files", move || {
                // Privacy: files in the folders Settings > Search excludes are never listed
                let excluded = super::search_prefs::current();
                let files: Vec<crate::files::fs::Entry> = crate::files::fs::recent_in(&[root], 2000).into_iter().filter(|e| !excluded.folder_excluded(&e.path)).collect();
                let _ = tx.send(files);
            });
            self.files_rx = Some(rx);
        }
    }

    /// Takes the contacts and Deildu titles a worker read; true when they changed.
    fn take_sources(&mut self) -> bool {
        let Some(rx) = &self.sources_rx else { return false };
        let read = match rx.try_recv() {
            Ok(read) => read,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.sources_rx = None;
                return false;
            }
        };
        self.sources_rx = None;
        let changed = read.contacts.is_some() || read.titles.is_some() || read.events.is_some();
        if let Some(events) = read.events {
            self.events = events;
        }
        self.calendar_stamp = read.calendar_stamp;
        if let Some(contacts) = read.contacts {
            self.contacts = contacts;
        }
        if let Some(folded) = read.contacts_folded {
            self.contacts_folded = folded;
        }
        if let Some(titles) = read.titles {
            self.set_deildu_titles(titles);
        }
        self.contacts_stamp = read.contacts_stamp;
        self.deildu_stamp = read.deildu_stamp;
        changed
    }

    /// Takes a finished file scan; true when the list changed.
    fn take_files(&mut self) -> bool {
        let Some(rx) = &self.files_rx else { return false };
        match rx.try_recv() {
            Ok(files) => {
                self.set_files(files);
                self.files_rx = None;
                self.files_at = Some(std::time::Instant::now());
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.files_rx = None;
                false
            }
        }
    }

    pub fn set_files(&mut self, files: Vec<crate::files::fs::Entry>) {
        self.files_folded = files.iter().map(|e| fuzzy::fold(&e.name)).collect();
        self.files = files;
    }

    /// True while the card covers the screen in any way.
    pub fn active(&self) -> bool {
        !matches!(self.state, State::Hidden)
    }

    pub fn is_shown(&self) -> bool {
        matches!(self.state, State::Shown)
    }

    /// The search field, for an input method's composition (`compose_window`).
    pub fn field_mut(&mut self) -> &mut TextField {
        &mut self.field
    }

    pub fn animating(&self) -> bool {
        matches!(self.state, State::Settling { .. }) || (self.active() && self.keyboard.animating())
    }

    pub fn query(&self) -> &str {
        self.field.text()
    }

    pub fn describe(&self) -> &'static str {
        match self.state {
            State::Hidden => "hidden",
            State::Dragging(_) => "dragging",
            State::Settling { to, .. } if to >= 1.0 => "opening",
            State::Settling { .. } => "closing",
            State::Shown => "shown",
        }
    }

    /// 0 hidden .. 1 shown, past the ends under the rubber band.
    pub fn progress(&self, t: f64) -> f32 {
        match (self.state, self.touch) {
            (State::Hidden, _) => 0.0,
            (State::Dragging(d), _) | (State::Shown, Some(Touch::Outside(d))) => d.progress(),
            (State::Settling { from, to, start }, _) => from + (to - from) * ease_out_quartic(progress(start, t, SPRING_MS)),
            (State::Shown, _) => 1.0,
        }
    }

    /// Drops the card at once (screen off, an app launch).
    pub fn hide(&mut self) {
        self.state = State::Hidden;
        self.touch = None;
        self.pull = None;
    }

    /// Starts following a pull that landed at `y0` and is now at `y`;
    /// `range` is the travel for a full reveal. The query starts empty.
    pub fn open(&mut self, ui: &Ui, h: i32, y0: f32, y: f32, t: f64, range: f32) {
        self.field.set_text(ui, "");
        self.field.focused = false;
        self.scroll = 0.0;
        self.search(ui, h);
        self.touch = None;
        let mut drag = Drag::new(y0, t, 0.0, range);
        drag.moved(y, t);
        self.state = State::Dragging(drag);
    }

    fn settle(&mut self, from: f32, shown: bool, t: f64) {
        self.touch = None;
        self.state = State::Settling { from, to: if shown { 1.0 } else { 0.0 }, start: t };
        if shown {
            if !self.mac {
                self.keyboard.appear(t);
            }
            self.field.focused = true;
        } else {
            self.keyboard.set_shown(false, t);
        }
    }

    /// Finishes a settle and repeats held keys; true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, h: i32, t: f64) -> bool {
        let mut dirty = false;
        let files = self.take_files();
        let sources = self.take_sources();
        // a new file scan or contacts read changes the rows, so the query starts again on them
        if files || sources {
            self.run_query.clear();
        }
        let answered = self.run.as_mut().is_some_and(|r| r.poll());
        if (files || sources || answered) && self.active() && !self.field.text().is_empty() {
            self.search(ui, h);
            dirty = true;
        }
        // a file row's picture finished drawing: the card redraws with it
        if self.file_thumbs.poll() && self.active() {
            dirty = true;
        }
        if let State::Settling { to, start, .. } = self.state {
            dirty = true;
            if progress(start, t, SPRING_MS) >= 1.0 {
                if to >= 1.0 {
                    self.state = State::Shown;
                } else {
                    self.hide();
                }
            }
        }
        if !self.active() {
            return dirty;
        }
        let outs = self.keyboard.tick(t);
        for out in &outs {
            // held keys only repeat backspace and arrows, which never submit
            self.key(ui, h, out.clone(), t);
        }
        let blinking = self.is_shown() && self.field.next_blink(t).is_some_and(|due| due <= t + 1.0);
        dirty || !outs.is_empty() || blinking || self.keyboard.animating()
    }

    /// When the card next needs a frame on its own (caret blink, slide).
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if !self.active() {
            return None;
        }
        if self.keyboard.animating() {
            return Some(t);
        }
        if self.files_rx.is_some() || self.sources_rx.is_some() || self.run.as_ref().is_some_and(|r| r.pending()) || self.file_thumbs.pending() {
            return Some(t + 50.0);
        }
        self.is_shown().then(|| self.field.next_blink(t)).flatten()
    }

    /// Applies a key: Return opens the Top Hit, the rest edit the query.
    pub fn key(&mut self, ui: &Ui, h: i32, out: Out, t: f64) -> Option<Hit> {
        if out == Out::Hide {
            self.keyboard.set_shown(false, t);
            self.field.focused = false;
            return None;
        }
        match out {
            Out::Named("Down") => {
                self.move_selection(ui, 1);
                return None;
            }
            Out::Named("Up") => {
                self.move_selection(ui, -1);
                return None;
            }
            // Space on an arrow-selected file previews it in Files instead of typing a space
            Out::Text(ref text) if text == " " => {
                if let Some(hit @ Hit::File { .. }) = self.selected_hit() {
                    return Some(hit);
                }
            }
            _ => {}
        }
        if self.field.edit(ui, &out, t) {
            return self.selected_hit().or_else(|| self.top_hit());
        }
        self.search(ui, h);
        None
    }

    /// The row the arrow keys are on, if any.
    pub fn selected_hit(&self) -> Option<Hit> {
        let (si, ri) = self.selected?;
        self.sections.get(si).and_then(|s| s.rows.get(ri)).map(|(h, _)| h.clone())
    }

    /// Moves the keyboard selection one row down (`step` 1) or up (-1) through the shown rows.
    fn move_selection(&mut self, ui: &Ui, step: isize) {
        let rows: Vec<(usize, usize)> = self.rows().into_iter().map(|(si, ri, _)| (si, ri)).collect();
        self.selected = step_row(&rows, self.selected, step);
        self.reveal_selected(ui);
    }

    /// Golden Gate: scrolls the list the least that brings the selected row fully into the card.
    fn reveal_selected(&mut self, ui: &Ui) {
        let Some((si, ri)) = self.selected.filter(|_| self.mac) else { return };
        let m = self.m();
        let rh = self.px(ui, m.row);
        let mut top = 0.0;
        for (i, sec) in self.sections.iter().enumerate() {
            top += self.px(ui, m.header);
            if i == si {
                top += rh * ri as f32;
                break;
            }
            top += rh * sec.rows.len() as f32;
        }
        let view = self.viewport_h(ui);
        if top < self.scroll {
            self.scroll = top;
        } else if top + rh > self.scroll + view {
            self.scroll = top + rh - view;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(ui));
    }

    /// Golden Gate: scrolls the list by `delta` px (positive: further down the results). True when it moved.
    pub fn scroll_by(&mut self, ui: &Ui, delta: f32) -> bool {
        let to = (self.scroll + delta).clamp(0.0, self.max_scroll(ui));
        let moved = (to - self.scroll).abs() > 0.01;
        self.scroll = to;
        moved
    }

    /// Golden Gate: sets the top of the Dock (px) the card must stay above, None when the Dock does not limit it.
    /// The rows are laid out again when it changed; true then.
    pub fn set_dock_top(&mut self, ui: &Ui, h: i32, top: Option<f32>) -> bool {
        let same = match (self.dock_top, top) {
            (Some(a), Some(b)) => (a - b).abs() < 0.5,
            (a, b) => a == b,
        };
        if same {
            return false;
        }
        self.dock_top = top;
        if self.mac && self.active() {
            self.search(ui, h);
            return true;
        }
        false
    }

    /// True for each touch of the drag emulated for a wheel notch Search took already.
    fn swallows_touch(&mut self, t: f64) -> bool {
        if self.wheel_swallow.0 == 0 || t > self.wheel_swallow.1 {
            self.wheel_swallow = (0, 0.0);
            return false;
        }
        self.wheel_swallow.0 -= 1;
        true
    }

    /// The first result: what Return opens.
    pub fn top_hit(&self) -> Option<Hit> {
        self.sections.first().and_then(|s| s.rows.first()).map(|(h, _)| h.clone())
    }

    /// Rebuilds the result rows for the query, keeping what fits above the keyboard.
    fn search(&mut self, ui: &Ui, h: i32) {
        let m = self.m();
        let k = self.k(ui);
        let query = self.field.text().to_string();
        let mut groups = self.groups_for(&query);
        // Golden Gate: one chip per kind of result present; a chosen one narrows the list to that kind
        if self.mac {
            let kinds = chips_of(&groups);
            if self.chip.is_some_and(|c| !kinds.contains(&c)) {
                self.chip = None;
            }
            self.chips = kinds.into_iter().map(|t| (t, ui.text(t, MAC_BODY_PT * k, Weight::Regular, WHITE))).collect();
            if let Some(c) = self.chip {
                groups.retain(|g| g.title == c);
            }
        } else {
            self.chips.clear();
        }
        // the row the person is on stays selected, wherever the late rows put it
        let keep = self.selected_hit();
        let mut sections = vec![];
        let mut y = self.content_top(ui) + self.px(ui, m.bottom_pad) + self.chips_band(ui);
        // Golden Gate: the card ends a margin above the Dock (or the screen) and the rows scroll inside it, so every
        // result is built up to a cap; the bifrost cuts the list at the keyboard
        self.max_bottom = if self.mac { self.dock_top.unwrap_or(h as f32) - ui.pt(MAC_DOCK_MARGIN_PT) } else { f32::MAX };
        let bottom = if self.mac { f32::MAX } else { self.keyboard_top(ui, h) - ui.pt(8.0) };
        let mut built = 0usize;
        // thumbnails are fitted to the row's icon square; a size change (another scale) reloads them
        let isz = self.px(ui, m.row_icon).round() as usize;
        self.thumbs.retain(|(_, s, _)| *s == isz);
        if self.mac && self.row_icons.first().map_or(!self.icons.is_empty(), |(_, img)| img.h != isz) {
            // the 60 pt icons are the sharpest source for the (larger) Golden Gate row square
            self.row_icons = self.icons.iter().map(|(app, big, ..)| (*app, big.resized(isz, isz))).collect();
        }
        if self.thumbs.len() > 64 {
            self.thumbs.clear();
        }
        'fill: for g in groups {
            let mut rows = vec![];
            let mut subs = vec![];
            for hit in g.hits {
                let need = self.px(ui, m.row) + if rows.is_empty() { self.px(ui, m.header) } else { 0.0 };
                if y + need > bottom || (self.mac && built >= MAC_MAX_ROWS) {
                    break 'fill;
                }
                built += 1;
                y += need;
                if let Hit::Indexed { app_id, thumb, .. } = &hit {
                    let key = format!("{app_id}/{thumb}");
                    if !thumb.is_empty() && !self.thumbs.iter().any(|(k, _, _)| *k == key) {
                        let img = super::spotindex::load_thumbnail(app_id, thumb, isz);
                        self.thumbs.push((key, isz, img));
                    }
                }
                // a file row shows a picture of the file: asked once here, drawn on a worker, shown by `draw`
                if let Hit::File { path, .. } = &hit {
                    self.file_thumbs.want(path, isz);
                }
                let name_of = |app: AppId| apps().iter().find(|a| a.id == app).map_or(String::new(), |a| a.name.to_string());
                let max_w = self.px(ui, m.card_w - m.row_text_x - m.inset);
                let label_pt = self.m().label_pt * k;
                if self.mac {
                    let (title, sub) = row_text(&hit, name_of);
                    rows.push((hit, ui.text(&ui.fit(&title, label_pt, max_w), label_pt, Weight::Regular, WHITE)));
                    subs.push((!sub.is_empty()).then(|| ui.text(&ui.fit(&sub, MAC_BODY_PT * k, max_w), MAC_BODY_PT * k, Weight::Regular, WHITE)));
                    continue;
                }
                let label = match &hit {
                    Hit::App(app) => name_of(*app),
                    Hit::Settings(name) | Hit::City(name) => name.to_string(),
                    Hit::Deildu(title) => title.clone(),
                    Hit::Contact { name, .. } => name.clone(),
                    Hit::Event { title } => title.clone(),
                    Hit::Terminal(_) => "Search in Terminal".to_string(),
                    Hit::Answer(a) => format!("= {a}"),
                    Hit::File { name, .. } => name.clone(),
                    Hit::Reveal { name, .. } => format!("Show {name} in Folder"),
                    Hit::Shortcut { title, app_name, .. } => format!("{title} ({app_name})"),
                    Hit::SettingsIntent { title, .. } => title.to_string(),
                    Hit::SettingsAction { title, .. } | Hit::MediaTransport { title, .. } => title.clone(),
                    Hit::Indexed { title, kind, .. } => indexed_label(title, kind),
                };
                rows.push((hit, ui.text(&ui.fit(&label, label_pt, max_w), label_pt, Weight::Regular, WHITE)));
                subs.push(None);
            }
            if !rows.is_empty() {
                // Golden Gate has chips where the bifrost has section headers
                let header = if self.mac { Image::new(1, 1) } else { ui.text(g.title, 20.0, Weight::Semibold, WHITE) };
                sections.push(Section { header, rows, subs });
            }
        }
        self.sections = sections;
        self.selected = keep.and_then(|wanted| self.sections.iter().enumerate().find_map(|(si, s)| s.rows.iter().position(|(r, _)| *r == wanted).map(|ri| (si, ri))));
        self.selected = None;
        // a new query starts at the top; the same one (late rows, another Dock) keeps its place within the new bounds
        if self.scroll_query != query {
            self.scroll = 0.0;
            self.scroll_query = query;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(ui));
    }

    /// Rows as the list shows them: (section, row, label text).
    pub fn rows(&self) -> Vec<(usize, usize, Hit)> {
        self.sections.iter().enumerate().flat_map(|(si, s)| s.rows.iter().enumerate().map(move |(ri, (h, _))| (si, ri, h.clone()))).collect()
    }

    // -- geometry --

    fn card_x(&self, ui: &Ui, w: i32) -> f32 {
        let m = self.m();
        ((w as f32 - self.px(ui, m.card_w)) / 2.0).round()
    }

    /// Below the field, where the suggestions or the results start.
    fn content_top(&self, ui: &Ui) -> f32 {
        let m = self.m();
        self.px(ui, m.card_top + m.field + if self.mac { MAC_GAP_PT } else { 0.0 })
    }

    /// The height of the band of filter chips over the results, in px: none without chips (or on the bifrost).
    fn chips_band(&self, ui: &Ui) -> f32 {
        if self.mac && !self.chips.is_empty() {
            self.px(ui, self.m().chips)
        } else {
            0.0
        }
    }

    /// Where the first result row starts: under the card's top padding and the chips. On the bifrost the first section's
    /// header follows `content_top` directly.
    fn rows_top(&self, ui: &Ui) -> f32 {
        if self.mac {
            self.content_top(ui) + self.px(ui, self.m().bottom_pad) + self.chips_band(ui)
        } else {
            self.content_top(ui)
        }
    }

    /// The filter chips (x, y, w, h) in px, left to right from the card's inset; chips that would run off the card are
    /// not shown. Empty on the bifrost and while no results are listed.
    fn chip_rects(&self, ui: &Ui, w: i32) -> Vec<Rect> {
        if self.chips_band(ui) == 0.0 {
            return vec![];
        }
        let m = self.m();
        let (cx, _, cw, _) = self.card(ui, w);
        let (h, gap, pad) = (self.px(ui, MAC_CHIP_H_PT), self.px(ui, MAC_CHIP_GAP_PT), self.px(ui, MAC_CHIP_PAD_PT));
        let y = self.content_top(ui) + self.px(ui, m.bottom_pad) + (self.chips_band(ui) - h) / 2.0;
        let (left, right) = (cx + self.px(ui, m.bottom_pad + 4.0), cx + cw - self.px(ui, m.bottom_pad + 4.0));
        let mut x = left;
        let mut out = vec![];
        for (_, img) in &self.chips {
            let chip_w = img.w as f32 + 2.0 * pad;
            if x + chip_w > right {
                break;
            }
            out.push((x, y, chip_w, h));
            x += chip_w + gap;
        }
        out
    }

    fn suggestions_shown(&self) -> bool {
        self.m().suggestions && self.field.is_empty() && (!self.recents.is_empty() || !self.searches.is_empty())
    }

    fn content_h(&self, ui: &Ui) -> f32 {
        let m = self.m();
        if self.suggestions_shown() {
            let apps = if self.recents.is_empty() { 0.0 } else { m.header + SUGGEST_PT };
            let found = if self.searches.is_empty() { 0.0 } else { m.header + m.row * self.searches.len() as f32 + m.bottom_pad };
            ui.pt(apps + found)
        } else if self.sections.is_empty() {
            0.0
        } else {
            let natural = self.list_fixed(ui) + self.rows_h(ui);
            if self.mac {
                // never taller than what ends a margin above the Dock, but always room for one row
                let room = self.max_bottom - self.px(ui, m.card_top + m.field);
                natural.min(room.max(self.list_fixed(ui) + self.px(ui, m.row)))
            } else {
                natural
            }
        }
    }

    /// The height of every result row together, in px.
    fn rows_h(&self, ui: &Ui) -> f32 {
        let m = self.m();
        self.sections.iter().map(|s| self.px(ui, m.header) + self.px(ui, m.row) * s.rows.len() as f32).sum()
    }

    /// The part of the results card that does not scroll, in px: the gap under the capsule, the padding above the chips
    /// and the chips (Golden Gate), and the padding under the last row.
    fn list_fixed(&self, ui: &Ui) -> f32 {
        let m = self.m();
        let gap = if self.mac { self.px(ui, MAC_GAP_PT + m.bottom_pad) + self.chips_band(ui) } else { 0.0 };
        gap + self.px(ui, m.bottom_pad)
    }

    /// The height of the window the rows scroll in, in px (the card's content less what does not scroll).
    fn viewport_h(&self, ui: &Ui) -> f32 {
        if self.sections.is_empty() || self.suggestions_shown() {
            return 0.0;
        }
        (self.content_h(ui) - self.list_fixed(ui)).max(0.0)
    }

    /// How far the rows can be scrolled, in px: 0 when they all fit in the card.
    fn max_scroll(&self, ui: &Ui) -> f32 {
        if !self.mac || self.sections.is_empty() || self.suggestions_shown() {
            return 0.0;
        }
        (self.rows_h(ui) - self.viewport_h(ui)).max(0.0)
    }

    /// The card (x, y, w, h) in px at rest.
    /// The desktop capsule and its results card as one overlay: (x, y, w, h) plus the alpha mask (capsule
    /// rounded to half its height, the card below it to the card radius). None unless shown on the desktop.
    pub fn desk_overlay(&self, ui: &Ui, w: i32) -> Option<((usize, usize, usize, usize), Vec<u8>)> {
        if !self.mac || !self.active() {
            return None;
        }
        let m = self.m();
        let (cx, cy, cw, ch) = self.card(ui, w);
        let (x, y, ow, oh) = (cx.max(0.0) as usize, cy.max(0.0) as usize, cw.ceil() as usize, ch.ceil() as usize);
        let fh = self.px(ui, m.field).ceil() as usize;
        let mut mask = vec![0u8; ow * oh];
        let mut put = |top: usize, h: usize, r: f32| {
            for (i, a) in crate::gfx::rrect_mask(ow, h, r).into_iter().enumerate() {
                let at = (top + i / ow) * ow + i % ow;
                if at < mask.len() {
                    mask[at] = mask[at].max(a);
                }
            }
        };
        put(0, fh.min(oh), fh as f32 / 2.0);
        let gap = self.px(ui, MAC_GAP_PT) as usize;
        if oh > fh + gap {
            put(fh + gap, oh - fh - gap, self.px(ui, m.radius));
        }
        Some(((x, y, ow, oh), mask))
    }

    pub fn card(&self, ui: &Ui, w: i32) -> Rect {
        let m = self.m();
        (self.card_x(ui, w), self.px(ui, m.card_top), self.px(ui, m.card_w), self.px(ui, m.field) + self.content_h(ui))
    }

    fn field_rect(&self, ui: &Ui, w: i32) -> Rect {
        let m = self.m();
        let (x, y, cw, _) = self.card(ui, w);
        (x, y, cw, self.px(ui, m.field))
    }

    /// Centre of the clear button at the field's right end.
    fn clear_centre(&self, ui: &Ui, w: i32) -> (f32, f32) {
        let m = self.m();
        let (x, y, cw, fh) = self.field_rect(ui, w);
        (x + cw - self.px(ui, m.clear_x), y + fh / 2.0)
    }

    /// The suggestion icon `i` (x, y, w, h) in px.
    pub fn suggestion_rect(&self, ui: &Ui, w: i32, i: usize) -> Rect {
        let m = self.m();
        let x = self.card_x(ui, w) + ui.pt(SUGGEST_INSET_PT + SUGGEST_PITCH_PT * i as f32);
        let y = self.content_top(ui) + ui.pt(m.header + SUGGEST_ICON_TOP_PT);
        (x, y, ui.pt(SUGGEST_ICON_PT), ui.pt(SUGGEST_ICON_PT))
    }

    /// The recent-search row `i` (x, y, w, h) in px.
    pub fn search_rect(&self, ui: &Ui, w: i32, i: usize) -> Rect {
        let m = self.m();
        let apps = if self.recents.is_empty() { 0.0 } else { m.header + SUGGEST_PT };
        let y = self.content_top(ui) + ui.pt(apps + m.header + m.row * i as f32);
        (self.card_x(ui, w), y, ui.pt(m.card_w), ui.pt(m.row))
    }

    /// Every result row's (section, row) and px rect, top to bottom.
    pub fn row_rects(&self, ui: &Ui, w: i32) -> Vec<((usize, usize), Rect)> {
        let m = self.m();
        let (x, _, cw, _) = self.card(ui, w);
        let top = self.rows_top(ui);
        let bottom = top + self.viewport_h(ui);
        let mut y = top - self.scroll;
        let mut out = vec![];
        for (si, s) in self.sections.iter().enumerate() {
            y += self.px(ui, m.header);
            for ri in 0..s.rows.len() {
                let rh = self.px(ui, m.row);
                if !self.mac {
                    out.push(((si, ri), (x, y, cw, rh)));
                } else {
                    // a row cut by the card's edge answers only where it shows
                    let (y0, y1) = (y.max(top), (y + rh).min(bottom));
                    if y1 > y0 {
                        out.push(((si, ri), (x, y0, cw, y1 - y0)));
                    }
                }
                y += rh;
            }
        }
        out
    }

    /// Top of the docked keyboard (where it rests when shown).
    pub fn keyboard_top(&self, ui: &Ui, h: i32) -> f32 {
        h as f32 - home_zone(ui) - self.keyboard.h()
    }

    /// The first row the keyboard covers at `t`, while it is up or sliding.
    pub fn keyboard_edge(&self, ui: &Ui, h: i32, t: f64) -> Option<f32> {
        let drawn = matches!(self.state, State::Shown | State::Settling { .. }) && (self.keyboard.shown() || self.keyboard.animating());
        drawn.then(|| self.keyboard_top(ui, h) + self.keyboard.offset(t))
    }

    /// What (x, y) lands on while the card is shown.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Target {
        let inside = |(rx, ry, rw, rh): Rect| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if self.keyboard.shown() && y >= self.keyboard_top(ui, h) && y < h as f32 - home_zone(ui) {
            return Target::Keyboard;
        }
        let (cx, cy) = self.clear_centre(ui, w);
        let r = ui.pt(CLEAR_R_PT + 8.0);
        if !self.field.is_empty() && (x - cx).abs() < r && (y - cy).abs() < r {
            return Target::Clear;
        }
        if inside(self.field_rect(ui, w)) {
            return Target::Field;
        }
        if self.suggestions_shown() {
            if let Some(i) = (0..self.recents.len()).find(|&i| inside(self.suggestion_rect(ui, w, i))) {
                return Target::Suggestion(i);
            }
        }
        if self.suggestions_shown() {
            if let Some(i) = (0..self.searches.len()).find(|&i| inside(self.search_rect(ui, w, i))) {
                return Target::Search(i);
            }
        }
        if let Some(i) = self.chip_rects(ui, w).into_iter().position(inside) {
            return Target::Chip(i);
        }
        if let Some((at, _)) = self.row_rects(ui, w).into_iter().find(|(_, r)| inside(*r)) {
            return Target::Row(at.0, at.1);
        }
        Target::Outside
    }

    /// A finger lands on the shown card; `range` is the travel that pulls it away.
    pub fn down(&mut self, ui: &Ui, w: i32, h: i32, (x, y): (f32, f32), t: f64, range: f32) {
        self.touch = Some(match self.hit(ui, w, h, x, y) {
            Target::Keyboard => {
                self.keyboard.down(x, y - self.keyboard_top(ui, h), t);
                Touch::Keyboard
            }
            Target::Outside => Touch::Outside(Drag::new(y, t, 1.0, range)),
            target => Touch::Target(target),
        });
    }

    pub fn moved(&mut self, ui: &Ui, h: i32, x: f32, y: f32, t: f64) {
        match self.touch {
            Some(Touch::Keyboard) => self.keyboard.moved(x, y - self.keyboard_top(ui, h)),
            Some(Touch::Outside(mut d)) => {
                d.moved(y, t);
                self.touch = Some(Touch::Outside(d));
            }
            _ => {}
        }
    }

    /// Lifts the finger; returns what to open, if anything.
    pub fn up(&mut self, ui: &Ui, w: i32, h: i32, x: f32, y: f32, t: f64) -> Option<Hit> {
        match self.touch.take()? {
            Touch::Keyboard => {
                let out = self.keyboard.up(x, y - self.keyboard_top(ui, h), t)?;
                self.key(ui, h, out, t)
            }
            Touch::Outside(mut d) => {
                d.moved(y, t);
                let tap = d.travel().abs() < ui.pt(TAP_PT);
                self.settle(d.progress(), !tap && d.release(t), t);
                None
            }
            Touch::Target(target) if self.hit(ui, w, h, x, y) == target => self.activate(ui, w, h, target, x, t),
            Touch::Target(_) => None,
        }
    }

    fn activate(&mut self, ui: &Ui, w: i32, h: i32, target: Target, x: f32, t: f64) -> Option<Hit> {
        let m = self.m();
        match target {
            Target::Field => {
                self.show_keyboard(t);
                self.field.tap(ui, x - self.card_x(ui, w) - self.px(ui, m.text_x), t);
                None
            }
            Target::Clear => {
                self.field.set_text(ui, "");
                self.field.focused = true;
                self.show_keyboard(t);
                self.search(ui, h);
                None
            }
            Target::Search(i) => {
                if let Some(q) = self.searches.get(i).cloned() {
                    self.field.set_text(ui, &q);
                    self.field.focused = true;
                    self.show_keyboard(t);
                    self.search(ui, h);
                }
                None
            }
            Target::Suggestion(i) => self.recents.get(i).map(|&a| Hit::App(a)),
            Target::Row(si, ri) => self.sections.get(si).and_then(|s| s.rows.get(ri)).map(|(hit, _)| hit.clone()),
            Target::Chip(i) => {
                // a chip narrows the results to its kind; the same chip again shows every kind
                let title = self.chips.get(i).map(|(t, _)| *t);
                self.chip = if self.chip == title { None } else { title };
                self.scroll = 0.0;
                self.search(ui, h);
                None
            }
            Target::Keyboard | Target::Outside => None,
        }
    }

    // -- drawing --

    /// Draws the card and keyboard over the home screen already in `s`.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        let m = self.m();
        let p = self.progress(t);
        let (cx, cy, cw, ch) = self.card(ui, w);
        let slide = if p < 1.0 { (p - 1.0) * (cy + ch) } else { (p - 1.0) * ui.pt(60.0) };
        let alpha = (p.clamp(0.0, 1.0) * 255.0) as u32;
        let a = |c: u32| scale(c, alpha);
        let cy = cy + slide;
        let fh = self.px(ui, m.field);
        if self.mac {
            // Golden Gate: a glass capsule for the field and, when there are results, a glass card under it of the
            // same material, both through the presenter's glass hook as the menus are.
            mac_glass(s, cx, cy, cw, fh, fh / 2.0, alpha);
            if ch > fh {
                let ry = cy + fh + self.px(ui, MAC_GAP_PT);
                mac_glass(s, cx, ry, cw, ch - fh - self.px(ui, MAC_GAP_PT), self.px(ui, m.radius), alpha);
            }
        } else {
            // Liquid Glass (regular): blur the wallpaper and dock beneath, tint it, and light the edge.
            s.glass_rrect(cx, cy, cw, ch, ui.pt(m.radius), a(GLASS_TINT), a(CARD_BG), ui.pt(GLASS_BLUR_PT) as usize);
            s.stroke_rrect(cx, cy, cw, ch, ui.pt(m.radius), ui.pt(1.0), a(GLASS_EDGE));
        }

        // the field: magnifier (bifrost), query or placeholder, clear button
        let mid = cy + fh / 2.0;
        let (gx, gr) = (cx + self.px(ui, m.glass_x), ui.pt(GLASS_R_PT));
        let stroke = ui.pt(2.2);
        let glass = glyphs::either(glyphs::ring((gx, mid - ui.pt(1.5)), gr, stroke), glyphs::segment((gx + gr * 0.72, mid - ui.pt(1.5) + gr * 0.72), (gx + gr * 1.55, mid - ui.pt(1.5) + gr * 1.55), stroke * 1.2));
        if !self.mac {
            glyphs::paint(s, (gx - gr - stroke, mid - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), a(SECONDARY), glass);
        }
        let tx = cx + self.px(ui, m.text_x);
        let text_w = cw - self.px(ui, m.text_x + m.clear_x + CLEAR_R_PT + 8.0);
        if alpha == 255 {
            self.field.draw(s, ui, (tx, cy, text_w, fh), t);
        }
        if self.field.is_empty() {
            let (kx, ky) = self.clear_centre(ui, w);
            if m.mic {
                glyphs::mic(s, (kx, ky + slide), ui.pt(MIC_PT), ui.pt(1.7), a(SECONDARY));
            }
        } else {
            let (kx, ky) = self.clear_centre(ui, w);
            let (ky, r) = (ky + slide, self.px(ui, CLEAR_R_PT));
            s.fill_rrect(kx - r, ky - r, 2.0 * r, 2.0 * r, r, a(SECONDARY));
            let d = r * 0.42;
            let x_mark = glyphs::either(glyphs::segment((kx - d, ky - d), (kx + d, ky + d), ui.pt(1.8)), glyphs::segment((kx - d, ky + d), (kx + d, ky - d), ui.pt(1.8)));
            glyphs::paint(s, (kx - r, ky - r, 2.0 * r, 2.0 * r), a(CELL), x_mark);
        }
        if ch > fh && !self.mac {
            s.fill_rrect(cx, cy + fh - 1.0, cw, 1.0, 0.0, a(SEPARATOR));
        }

        let pressed = match self.touch {
            Some(Touch::Target(t)) => Some(t),
            _ => None,
        };
        let header_x = (cx + self.px(ui, m.inset)) as i32;
        let mut y = cy + fh + if self.mac && ch > fh { self.px(ui, MAC_GAP_PT + m.bottom_pad) } else { 0.0 };
        // Golden Gate: the filter chips across the top of the results card
        if self.mac && ch > fh {
            let chosen = self.chip;
            for (i, (cxr, cyr, cwr, chr)) in self.chip_rects(ui, w).into_iter().enumerate() {
                let Some((title, img)) = self.chips.get(i) else { break };
                let on = chosen == Some(*title);
                let plate = if on { SIDEBAR_PLATE.now() } else { FIELD_BG.now() };
                let pressed_chip = pressed == Some(Target::Chip(i));
                s.fill_rrect(cxr, cyr + slide, cwr, chr, chr / 2.0, a(if on || pressed_chip { plate } else { scale(plate, 140) }));
                s.blit_colored(img, (cxr + (cwr - img.w as f32) / 2.0) as i32, (cyr + slide + (chr - img.h as f32) / 2.0) as i32, LABEL.now(), alpha);
            }
            y += self.chips_band(ui);
        }
        if self.suggestions_shown() {
            if self.search_imgs.len() != self.searches.len() {
                let max_w = ui.pt(m.card_w - m.row_text_x - m.inset);
                self.search_imgs = self.searches.iter().map(|q| ui.text(&ui.fit(q, 17.0, max_w), 17.0, Weight::Regular, WHITE)).collect();
            }
            if !self.recents.is_empty() {
            let hd = &self.suggestions;
            s.blit_colored(hd, header_x, (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32, LABEL_2.now(), alpha);
            let ch = &self.chevron;
            s.blit_colored(ch, (cx + cw - ui.pt(m.inset + 4.0) - ch.w as f32) as i32, (y + ui.pt(m.header_mid) - ch.h as f32 / 2.0) as i32, LABEL_2.now(), alpha);
            for (i, app) in self.recents.iter().enumerate() {
                let Some((_, big, _, label)) = self.icons.iter().find(|(a, ..)| a == app) else { continue };
                let (ix, iy, iw, _) = self.suggestion_rect(ui, w, i);
                let iy = iy + slide;
                let tint = if pressed == Some(Target::Suggestion(i)) { 150 } else { 255 };
                s.blit(big, ix as i32, iy as i32, tint, alpha);
                s.blit_colored(label, (ix + (iw - label.w as f32) / 2.0) as i32, (iy + iw + ui.pt(SUGGEST_LABEL_GAP_PT)) as i32, LABEL.now(), alpha);
            }
            y += ui.pt(m.header + SUGGEST_PT);
            }
            if !self.searches.is_empty() {
                let hd = &self.searches_header;
                s.blit_colored(hd, header_x, (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32, LABEL_2.now(), alpha);
                y += ui.pt(m.header);
                for (i, label) in self.search_imgs.iter().enumerate() {
                    let rh = ui.pt(m.row);
                    if pressed == Some(Target::Search(i)) {
                        s.fill_rrect(cx + ui.pt(6.0), y, cw - ui.pt(12.0), rh, ui.pt(8.0), a(PRESSED));
                    }
                    let (gx, gr) = (cx + ui.pt(m.row_icon_x + m.row_icon / 2.0 - 3.0), ui.pt(5.5));
                    let stroke = ui.pt(1.6);
                    let glass = glyphs::either(glyphs::ring((gx, y + rh / 2.0 - ui.pt(1.0)), gr, stroke), glyphs::segment((gx + gr * 0.72, y + rh / 2.0 - ui.pt(1.0) + gr * 0.72), (gx + gr * 1.5, y + rh / 2.0 - ui.pt(1.0) + gr * 1.5), stroke * 1.2));
                    glyphs::paint(s, (gx - gr - stroke, y + rh / 2.0 - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), a(LABEL_2.now()), glass);
                    s.blit_colored(label, (cx + ui.pt(m.row_text_x)) as i32, (y + rh / 2.0 - label.h as f32 / 2.0) as i32, LABEL.now(), alpha);
                    y += rh;
                }
            }
        } else {
            // Golden Gate: the rows scroll under the chips and are cut by the card above its bottom padding, as in the
            // reference where the last row runs out at the card's rounded edge
            let (clip_top, clip_bot) = if self.mac { (y, cy + ch - self.px(ui, m.bottom_pad)) } else { (0.0, h as f32) };
            let mut clipped = s.rows(clip_top.max(0.0).floor() as usize, clip_bot.max(0.0).ceil() as usize);
            let s = &mut clipped;
            if self.mac {
                y -= self.scroll;
            }
            for (si, sec) in self.sections.iter().enumerate() {
                if !self.mac {
                    let hd = &sec.header;
                    let hy = (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32;
                    s.blit_colored(hd, header_x, hy, LABEL_2.now(), alpha);
                }
                y += self.px(ui, m.header);
                for (ri, (hit, label)) in sec.rows.iter().enumerate() {
                    let rh = self.px(ui, m.row);
                    if self.mac && (y + rh <= clip_top || y >= clip_bot) {
                        y += rh;
                        continue;
                    }
                    let top = self.mac && si == 0 && ri == 0;
                    if top {
                        let pi = self.px(ui, MAC_PLATE_INSET_PT);
                        s.fill_rrect(cx + pi, y, cw - 2.0 * pi, rh, self.px(ui, MAC_PLATE_R_PT), a(SIDEBAR_PLATE.now())); // the reference: a neutral plate, not the accent
                    } else if pressed == Some(Target::Row(si, ri)) || self.selected == Some((si, ri)) {
                        s.fill_rrect(cx + ui.pt(6.0), y, cw - ui.pt(12.0), rh, ui.pt(8.0), a(PRESSED));
                    }
                    let (ix, isz) = (cx + self.px(ui, m.row_icon_x), self.px(ui, m.row_icon));
                    let iy = y + (rh - isz) / 2.0;
                    let app = match hit {
                        Hit::App(a) => Some(*a),
                        Hit::City(_) => super::apphost::slot_of(super::clock::APP_ID).map(|n| AppId::External(n as u8)),
                        Hit::Contact { .. } => super::apphost::slot_of("is.olibuijr.contacts").map(|n| AppId::External(n as u8)),
                        Hit::Deildu(_) => Some(AppId::Deildu),
                        Hit::Terminal(_) => Some(AppId::Terminal),
                        Hit::Answer(_) => super::apphost::slot_of(super::apphost::CALCULATOR).map(|n| AppId::External(n as u8)),
                        Hit::File { .. } | Hit::Reveal { .. } => Some(AppId::Files),
                        Hit::Shortcut { app_id, .. } | Hit::Indexed { app_id, .. } => super::apphost::slot_of(app_id).map(|n| AppId::External(n as u8)),
                        Hit::Event { .. } => super::apphost::slot_of(super::services::calendar::STORE_APP).map(|n| AppId::External(n as u8)),
                        Hit::Settings(_) | Hit::SettingsIntent { .. } | Hit::SettingsAction { .. } | Hit::MediaTransport { .. } => None,
                    };
                    // an indexed item with a readable thumbnail shows it in place of its app's icon
                    let thumb_img = match hit {
                        Hit::Indexed { app_id, thumb, .. } if !thumb.is_empty() => {
                            let key = format!("{app_id}/{thumb}");
                            self.thumbs.iter().find(|(k, s, _)| *k == key && *s == isz.round() as usize).and_then(|(_, _, img)| img.as_ref())
                        }
                        Hit::File { path, .. } => self.file_thumbs.ready(path, isz.round() as usize),
                        _ => None,
                    };
                    if let Some(img) = thumb_img {
                        s.blit(img, (ix + (isz - img.w as f32) / 2.0) as i32, (iy + (isz - img.h as f32) / 2.0) as i32, 255, alpha);
                    }
                    let sized = app.and_then(|app| self.row_icons.iter().find(|(a, _)| *a == app)).map(|(_, img)| img).filter(|img| self.mac && img.h == isz.round() as usize);
                    let icon = sized.or_else(|| app.and_then(|app| self.icons.iter().find(|(a, ..)| *a == app)).map(|(_, _, small, _)| small));
                    match (icon, result_symbol(hit)) {
                        // an app icon: the row-square copy fills the slot, the 29 pt one is centred in it
                        (Some(small), _) if thumb_img.is_none() => {
                            s.blit(small, (ix + (isz - small.w as f32) / 2.0) as i32, (iy + (isz - small.h as f32) / 2.0) as i32, 255, alpha);
                        }
                        // a Settings result, a shell action or a Now Playing command: the symbol of its pane
                        (None, Some(Symbol::Tile(icon))) if thumb_img.is_none() => {
                            icons::tile_sized(s, ui, icon, ix, iy, isz / ui.s);
                        }
                        (None, Some(Symbol::Glyph(glyph))) if thumb_img.is_none() => {
                            row_glyphs::tile(s, ui, glyph, ix, iy, isz / ui.s);
                        }
                        _ => {}
                    }
                    let lx = (cx + self.px(ui, m.row_text_x)) as i32;
                    let sub = sec.subs.get(ri).and_then(|i| i.as_ref());
                    match sub {
                        // two lines: the title over the kind, where it is or what it does, centred as a pair
                        Some(sub) => {
                            let gap = ui.pt(1.0);
                            let top = y + (rh - (label.h + sub.h) as f32 - gap) / 2.0;
                            s.blit_colored(label, lx, top as i32, LABEL.now(), alpha);
                            s.blit_colored(sub, lx, (top + label.h as f32 + gap) as i32, LABEL_2.now(), alpha);
                        }
                        None => s.blit_colored(label, lx, (y + rh / 2.0 - label.h as f32 / 2.0) as i32, LABEL.now(), alpha),
                    }
                    y += rh;
                }
            }
        }

        if self.keyboard_edge(ui, h, t).is_some() {
            self.draw_keyboard(s, ui, w, h, t);
        }
        if !self.mac {
            home_indicator(s, ui, w, h, 255);
        }
    }

    /// The keyboard, translucent over the wallpaper as on the bifrost: drawn
    /// opaque into a scratch image and blended in.
    fn draw_keyboard(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        let top = self.keyboard_top(ui, h);
        let kh = (h as f32 - top).ceil() as usize;
        let mut img = Image::new(w as usize, kh);
        let mut sub = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
        let off = self.keyboard.offset(t) as usize;
        sub.fill_rect(0, off.min(kh), w as usize, kh.saturating_sub(off), super::keyboard::BG);
        self.keyboard.draw(&mut sub, ui, 0.0, t);
        s.blit(&img, 0, top as i32, 255, KB_ALPHA);
    }
}

impl Shell {
    /// Handles a touch that belongs to Search: the pull on the home
    /// screen (decided against the page swipe once the finger has moved)
    /// and everything while the card is up. None leaves it to the screen.
    pub(super) fn quicksearch_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let range = self.ui.pt(REVEAL_PT);
        let (w, h) = (self.w, self.h);
        // the touches the window server emulates after a wheel notch must not tap the row under the pointer
        if matches!(self.quicksearch.state, State::Shown) && matches!(ev, Event::Down(..) | Event::Move(..) | Event::Up(..)) && self.quicksearch.swallows_touch(t) {
            return Some(vec![]);
        }
        match (self.quicksearch.state, ev) {
            (State::Hidden, Event::Down(x, y)) => {
                let zone = y >= self.layout.status_h as f32 && y < self.h as f32 - home_zone(&self.ui);
                self.quicksearch.pull = (self.screen == Screen::Home && zone).then(|| PullDown::new(x, y));
                None
            }
            (State::Hidden, Event::Move(x, y)) => {
                let p = self.quicksearch.pull?;
                match p.moved(x, y, self.ui.pt(DIRECTION_PT)) {
                    Pull::Undecided => None,
                    Pull::Page => {
                        self.quicksearch.pull = None;
                        None
                    }
                    Pull::Down => {
                        self.quicksearch.pull = None;
                        self.pager.cancel(t);
                        self.quicksearch_open(p.y0, y, t, range);
                        Some(vec![])
                    }
                }
            }
            (State::Hidden, _) => {
                self.quicksearch.pull = None;
                None
            }
            (State::Dragging(mut d), Event::Move(_, y)) => {
                d.moved(y, t);
                self.quicksearch.state = State::Dragging(d);
                Some(vec![])
            }
            (State::Dragging(mut d), Event::Up(_, y)) => {
                d.moved(y, t);
                self.quicksearch.settle(d.progress(), d.release(t), t);
                Some(vec![])
            }
            (State::Shown, Event::Down(x, y)) => {
                self.quicksearch.down(&self.ui, w, h, (x, y), t, range);
                Some(vec![])
            }
            (State::Shown, Event::Move(x, y)) => {
                self.quicksearch.moved(&self.ui, h, x, y, t);
                Some(vec![])
            }
            (State::Shown, Event::Up(x, y)) => {
                let hit = self.quicksearch.up(&self.ui, w, h, x, y, t);
                Some(hit.map_or(vec![], |hit| self.quicksearch_act(hit, t)))
            }
            _ => Some(vec![]),
        }
    }

    /// Where Search's results card must stop on the desktop, in px: the top of the Dock while it stands along the bottom
    /// edge and shows, else None (the screen's bottom edge limits it).
    fn quicksearch_dock_top(&self) -> Option<f32> {
        use super::desktop_dock::Position;
        if !self.desk_active() || self.dock_preferences().position != Position::Bottom || !self.dock_is_visible() {
            return None;
        }
        let m = self.desk_metrics();
        let (_, bar, _) = self.desk_dock(&m);
        Some((bar.y as f32).min(self.h as f32))
    }

    /// The wheel turned while Search is up on the desktop: the results scroll by `dy` px of content travel (negative
    /// for a wheel turned down) and the drag emulated for the notch is dropped. False when Search does not take it.
    pub fn quicksearch_wheel(&mut self, dy: f32, t: f64) -> bool {
        if !(self.quicksearch.mac && self.quicksearch.is_shown()) {
            return false;
        }
        if self.quicksearch.scroll_by(&self.ui, -dy) {
            self.dirty = true;
        }
        self.quicksearch.wheel_swallow = (3, t + WHEEL_DRAG_MS);
        true
    }

    /// Shows the card at once (the App Library's search field), as if pulled all the way down.
    pub(super) fn quicksearch_show(&mut self, t: f64) {
        let range = self.ui.pt(REVEAL_PT);
        self.quicksearch_open(0.0, range, t, range);
        if let State::Dragging(d) = self.quicksearch.state {
            self.quicksearch.settle(d.progress(), true, t);
        }
        self.dirty = true;
    }

    /// Starts the pull: icons and cached titles are refreshed first.
    fn quicksearch_open(&mut self, y0: f32, y: f32, t: f64, range: f32) {
        self.overview_dismiss(t);
        self.request_index_extensions();
        self.quicksearch.set_mac(&self.ui, self.desk_active());
        let dock_top = self.quicksearch_dock_top();
        self.quicksearch.dock_top = dock_top;
        if !self.quicksearch.has_icons() {
            let icons = apps().iter().enumerate().map(|(i, a)| (a.id, if i < super::IN_DOCK { &self.dock[i].icon } else { &self.grid[i - super::IN_DOCK].icon })).collect();
            self.quicksearch.set_icons(&self.ui, icons);
        }
        let root = self.files_locations.first().map_or_else(|| crate::files::fs::home(), |l| l.path.clone());
        self.quicksearch.refresh_sources(&super::contactstore::store_path(&self.data_dir), &self.deildu_dir, &super::services::calendar::store_path(&self.data_dir), root);
        self.quicksearch.open(&self.ui, self.h, y0, y, t, range);
    }

    /// Cmd/Meta+Space: opens Search fully from the home screen, and on the desktop over any app (Golden Gate).
    pub(super) fn open_quicksearch_hw(&mut self, t: f64) {
        if self.quicksearch.active() {
            self.quicksearch.hide();
            // the glass the panel declared goes with it: repaint everything so no edge of the capsule is left on the desktop
            self.force_full_redraw();
            self.dirty = true;
            return;
        }
        // locked, off or on the lock screen: no Search. On the desktop it opens over any app, whatever the screen is.
        if !self.screen_on || self.is_locked() || (self.screen != Screen::Home && !self.desk_active() && !self.form.is_desktop()) {
            return;
        }
        let range = self.ui.pt(REVEAL_PT);
        self.pager.cancel(t);
        self.quicksearch_open(0.0, range, t, range);
        self.quicksearch.settle(1.0, true, t);
    }

    /// Opens what a result stands for; the card drops at once.
    fn quicksearch_act(&mut self, hit: Hit, t: f64) -> Vec<Effect> {
        self.quicksearch.note_search();
        self.quicksearch.hide();
        match hit {
            Hit::App(app) => {
                self.quicksearch.note_open(&app_usage_key(app));
                self.open(app, t)
            }
            Hit::Settings(name) => {
                self.quicksearch.note_open(&settings_usage_key(name));
                return self.open_settings_page(name, t);
            }
            Hit::Event { .. } => {
                if let Some(n) = super::apphost::slot_of(super::services::calendar::STORE_APP) {
                    self.open(AppId::External(n as u8), t);
                }
            }
            Hit::Contact { id, .. } => {
                let _ = id;
                if let Some(n) = super::apphost::slot_of("is.olibuijr.contacts") {
                    self.open(AppId::External(n as u8), t);
                }
            }
            Hit::City(name) => {
                let _ = name;
                if let Some(n) = super::apphost::slot_of(super::clock::APP_ID) {
                    self.open(AppId::External(n as u8), t);
                }
            }
            Hit::Deildu(title) => {
                self.open(AppId::Deildu, t);
                if let Some(d) = &mut self.deildu {
                    d.search(&self.ui, &title);
                }
            }
            Hit::Answer(_) => {
                if let Some(n) = super::apphost::slot_of(super::apphost::CALCULATOR) {
                    self.open(AppId::External(n as u8), t)
                }
            }
            Hit::File { path, .. } => {
                self.open(AppId::Files, t);
                if let Some(entry) = self.quicksearch.files.iter().find(|e| e.path == path).cloned() {
                    self.preview_in_files(&entry, t);
                }
            }
            Hit::Reveal { folder, .. } => {
                self.open(AppId::Files, t);
                if let Some(f) = &mut self.files {
                    f.show_folder(&self.ui, &folder, t);
                }
            }
            Hit::SettingsAction { intent: super::settings_actions::SettingsIntent::LockScreen, .. } => {
                // locking changes the lock state too, so it goes through the same path as the menu item
                return self.lock_screen();
            }
            Hit::MediaTransport { command, .. } => {
                // the Now Playing owner decides whether the command applies; no window opens
                super::services::now_playing::send(command);
            }
            Hit::SettingsAction { intent, .. } => {
                // the same effect the Settings pane and the agent socket apply; no window opens
                return vec![intent.into_effect()];
            }
            Hit::SettingsIntent { id, .. } => {
                self.quicksearch.note_open(&format!("settings-intent:{id}"));
                if let Some(page) = super::settings_intents::page_for(id) {
                    return self.open_settings_intent_page(page, t);
                }
            }
            Hit::Shortcut { app_id, intent, .. } => {
                if let Some(n) = super::apphost::slot_of(&app_id) {
                    self.open(AppId::External(n as u8), t);
                    // the app may still be starting: retry the connection for a few seconds, off the UI thread
                    crate::bare::sys::spawn_named("intent-retry", move || {
                        if let Err(e) = super::intents::run(&app_id, &intent, std::time::Duration::from_secs(8)) {
                            eprintln!("quicksearch: intent {intent} of {app_id}: {e}");
                        }
                    });
                }
            }
            Hit::Indexed { app_id, unique_id, domain, .. } => {
                self.quicksearch.note_open(&indexed_key(&app_id, &unique_id));
                if let Some(n) = super::apphost::slot_of(&app_id) {
                    self.open(AppId::External(n as u8), t);
                    // the app may still be starting: the message waits in its socket until it reads it
                    let m = bifrost_proto::ToApp::ContinueActivity { activity_type: bifrost_proto::quicksearch::ACTION_TYPE.into(), unique_id, domain };
                    self.ext_send(n, &m);
                }
            }
            Hit::Terminal(query) => {
                self.open(AppId::Terminal, t);
                if let Some(term) = self.term.as_mut() {
                    term.write(query.as_bytes());
                }
            }
        }
        vec![]
    }

    /// Launches Settings on the page a row name stands for ("Wi-Fi",
    /// "Wallpaper", …; the rest show the root page).
    pub fn open_settings_page(&mut self, name: &str, t: f64) -> Vec<Effect> {
        self.open(AppId::Settings, t);
        let ui = &self.ui;
        self.settings.as_mut().map_or(vec![], |st| st.open_page(ui, name, t))
    }

    /// Launches Settings on the pane a Settings intent names (`settings_intents::page_for`).
    fn open_settings_intent_page(&mut self, page: super::pages::PageId, t: f64) -> Vec<Effect> {
        self.open(AppId::Settings, t);
        let ui = &self.ui;
        self.settings.as_mut().map_or(vec![], |st| st.open_page_id(ui, page, t))
    }

    /// Records a launch for the suggestions row (the shell does this on every open).
    pub fn note_launch(&mut self, app: AppId) {
        self.quicksearch.note_launch(app);
    }

    /// Types into the shown card as the keyboard would (remote control, previews).
    pub fn quicksearch_key(&mut self, out: Out, t: f64) -> Vec<Effect> {
        if !self.quicksearch.is_shown() {
            return vec![];
        }
        self.dirty = true;
        let dock_top = self.quicksearch_dock_top();
        self.quicksearch.set_dock_top(&self.ui, self.h, dock_top);
        match self.quicksearch.key(&self.ui, self.h, out, t) {
            Some(hit) => self.quicksearch_act(hit, t),
            None => vec![],
        }
    }

    /// Advances the card's settle and keyboard; true when a frame is needed.
    pub(super) fn quicksearch_tick(&mut self, t: f64) -> bool {
        // the Dock may show or hide under the open card: its top moves the card's limit
        let moved = self.quicksearch.active() && {
            let dock_top = self.quicksearch_dock_top();
            self.quicksearch.set_dock_top(&self.ui, self.h, dock_top)
        };
        self.quicksearch.tick(&self.ui, self.h, t) || moved
    }

    /// The home screen with the card and keyboard over it; the keyboard
    /// rises over bare wallpaper, the dock hidden under it as on the bifrost.
    pub(super) fn draw_quicksearch(&mut self, s: &mut Surface, t: f64) {
        self.draw_home(s);
        if let Some(edge) = self.quicksearch.keyboard_edge(&self.ui, self.h, t) {
            let wp = self.home_still();
            s.band(edge.max(0.0) as usize).copy(wp, (self.w - wp.w as i32) / 2, (self.h - wp.h as i32) / 2);
        }
        self.quicksearch.draw(s, &self.ui, self.w, self.h, t);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{settle, shell, swipe, unlock};
    use super::super::{AppId, Effect, Event, Screen, Shell, WifiAction};
    use super::*;
    use crate::gfx::Surface;

    fn ui() -> Ui {
        super::super::tests::ui()
    }

    /// A finger moving from y0 to y1 at x in ten steps `step_ms` apart.
    fn drag(sh: &mut Shell, x: f32, y0: f32, y1: f32, t: f64, step_ms: f64) -> f64 {
        sh.event(Event::Down(x, y0), t);
        for i in 1..=10 {
            sh.event(Event::Move(x, y0 + (y1 - y0) * i as f32 / 10.0), t + step_ms * i as f64);
        }
        let end = t + step_ms * 10.0 + 200.0;
        sh.event(Event::Up(x, y1), end);
        end
    }

    fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let mut fx = sh.event(Event::Down(x, y), t);
        fx.extend(sh.event(Event::Up(x, y), t + 30.0));
        fx
    }

    fn open(sh: &mut Shell, t: f64) -> f64 {
        let end = drag(sh, 600.0, 800.0, 800.0 + sh.ui.pt(60.0), t, 20.0);
        assert_eq!(sh.quicksearch.describe(), "opening");
        settle(sh, end);
        assert!(sh.quicksearch.is_shown());
        end + 1000.0
    }

    fn type_text(sh: &mut Shell, text: &str, t: f64) {
        sh.quicksearch.key(&sh.ui, sh.h, Out::Text(text.into()), t);
    }

    #[test]
    fn pull_decides_direction_once_past_the_slop() {
        let p = PullDown::new(100.0, 100.0);
        assert_eq!(p.moved(103.0, 105.0, 10.0), Pull::Undecided);
        assert_eq!(p.moved(100.0, 112.0, 10.0), Pull::Down);
        assert_eq!(p.moved(108.0, 112.0, 10.0), Pull::Down, "45 degrees still counts as a pull");
        assert_eq!(p.moved(114.0, 112.0, 10.0), Pull::Page, "more sideways than down pages");
        assert_eq!(p.moved(100.0, 88.0, 10.0), Pull::Page, "upwards is never a pull");
        assert_eq!(p.moved(-20.0, 100.0, 10.0), Pull::Page);
    }

    #[test]
    fn results_rank_a_top_hit_then_each_source_and_the_terminal_last() {
        let titles = vec!["Severance".to_string(), "Sex Education".to_string(), "Seinfeld".to_string(), "Secret Invasion".to_string()];
        let g = results("se", &titles);
        assert_eq!(g[0], Group { title: "Top Hit", hits: vec![Hit::App(AppId::Settings)] });
        assert_eq!(g[1].title, "Applications");
        assert!(g[1].hits.is_empty() || g[1].hits != vec![Hit::App(AppId::Settings)], "the top hit is not listed twice");
        let deildu = g.iter().find(|g| g.title == "Deildu").unwrap();
        assert_eq!(deildu.hits.len(), PER_GROUP, "each source stops at three");
        assert_eq!(deildu.hits[0], Hit::Deildu("Seinfeld".into()), "shorter prefix matches first");
        assert_eq!(g.last().unwrap(), &Group { title: "Search in Apps", hits: vec![Hit::Terminal("se".into())] });
        assert!(results("", &titles).is_empty());
        assert!(results("  ", &titles).is_empty());
        assert_eq!(results("wi", &[])[0].hits, vec![Hit::Settings("Wi-Fi")]);
        assert_eq!(results("tok", &[])[0].hits, vec![Hit::City("Tokyo")]);
        assert_eq!(results("francisco", &[])[0].hits, vec![Hit::City("San Francisco")]);
        let none = results("zzzz", &[]);
        assert_eq!(none.len(), 1, "only the terminal action: {none:?}");
        assert_eq!(none[0].hits, vec![Hit::Terminal("zzzz".into())]);
    }

    #[test]
    fn settings_intents_are_found_as_shortcuts_and_open_their_pane() {
        let groups = results("open keyboard", &[]);
        let wanted = Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" };
        assert!(groups.iter().flat_map(|g| g.hits.iter()).any(|h| *h == wanted), "{groups:?}");
        assert!(super::super::settings_intents::page_for("open-keyboard").is_some());
        assert!(super::super::settings_intents::page_for("no-such-intent").is_none());
    }

    #[test]
    fn wifi_action_intents_are_found_and_run_their_effect_without_a_window() {
        use super::super::settings_actions::SettingsIntent;
        let groups = results("turn wi-fi off", &[]);
        let wanted = Hit::SettingsAction { intent: SettingsIntent::WifiPower(false), title: "Turn Wi-Fi Off".into() };
        assert!(groups.iter().flat_map(|g| g.hits.iter()).any(|h| *h == wanted), "{groups:?}");
        let scan = Hit::SettingsAction { intent: SettingsIntent::WifiScan, title: "Scan for Wi-Fi Networks".into() };
        assert!(results("scan for wi-fi", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == scan));
        set_wallpaper_catalogue(vec![("kirkjufell_dusk".into(), "Kirkjufell Dusk".into())]);
        let picture = Hit::SettingsAction { intent: SettingsIntent::SetWallpaper("kirkjufell_dusk".into()), title: "Set Wallpaper to Kirkjufell Dusk".into() };
        assert!(results("kirkjufell", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == picture));
        set_wallpaper_catalogue(Vec::new());
        let lock = Hit::SettingsAction { intent: SettingsIntent::LockScreen, title: "Lock Screen".into() };
        assert!(results("lock screen", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == lock));
        let next = Hit::MediaTransport { command: bifrost_ui::media_session::RemoteCommand::Next, title: "Next Track".into() };
        assert!(results("next track", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == next));
    }

    #[test]
    fn spotlight_scope_hides_switched_off_kinds_and_excluded_folders() {
        use super::super::spotlight_scope::{Category, Scope};
        let mut scope = Scope::default();
        scope.set_allowed(Category::Settings, false);
        scope.exclude(Path::new("/home/u/Private"));
        let groups = vec![Group {
            title: "Top Hit",
            hits: vec![
                Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" },
                Hit::File { path: PathBuf::from("/home/u/Private/a.txt"), name: "a.txt".into() },
                Hit::File { path: PathBuf::from("/home/u/Work/b.txt"), name: "b.txt".into() },
            ],
        }, Group { title: "Settings", hits: vec![Hit::SettingsIntent { id: "open-sound", title: "Open Sound" }] }, Group { title: "Search in Apps", hits: vec![Hit::Terminal("x".into())] }];
        let kept = scoped(groups, &scope);
        assert_eq!(kept.len(), 2, "the Settings group empties and goes: {kept:?}");
        assert_eq!(kept[0].hits, vec![Hit::File { path: PathBuf::from("/home/u/Work/b.txt"), name: "b.txt".into() }]);
        assert_eq!(kept[1].title, "Search in Apps", "the action is never hidden");
    }

    /// A one-piece torrent named `name`.
    fn torrent(name: &str) -> Vec<u8> {
        let mut b = format!("d4:infod6:lengthi1e4:name{}:{name}12:piece lengthi16384e6:pieces20:", name.len()).into_bytes();
        b.extend([7u8; 20]);
        b.extend(b"ee");
        b
    }

    #[test]
    fn cached_titles_come_from_the_torrents_on_disk() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-cache"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(cached_titles(&dir).is_empty(), "no cache yet");
        std::fs::create_dir_all(dir.join("cache")).unwrap();
        std::fs::write(dir.join("cache/1.torrent"), torrent("The.Movie.2024.1080p.WEB-DL.x264")).unwrap();
        std::fs::write(dir.join("cache/2.torrent"), torrent("Another_Show_S01E02_720p")).unwrap();
        std::fs::write(dir.join("cache/3.torrent"), torrent("The.Movie.2024.720p")).unwrap();
        std::fs::write(dir.join("cache/4.torrent"), b"not a torrent").unwrap();
        std::fs::write(dir.join("cache/5.img"), b"art").unwrap();
        let titles = cached_titles(&dir);
        assert_eq!(titles, ["Another Show S01E02", "The Movie 2024"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recents_persist_newest_first_and_stop_at_six() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-recents/quicksearch.ini"));
        let _ = std::fs::remove_file(&path);
        let ui = ui();
        let mut sp = QuickSearch::new(&ui, 1200.0, ui.s);
        sp.load(&path);
        assert!(sp.recents().is_empty());
        for app in [AppId::Settings, AppId::Files, AppId::Settings, AppId::Terminal, AppId::Deildu, AppId::Files] {
            sp.note_launch(app);
        }
        assert_eq!(sp.recents(), [AppId::Files, AppId::Deildu, AppId::Terminal, AppId::Settings]);
        let mut again = QuickSearch::new(&ui, 1200.0, ui.s);
        again.load(&path);
        assert_eq!(again.recents(), [AppId::Files, AppId::Deildu, AppId::Terminal, AppId::Settings]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "recents=files,deildu,terminal,settings\n");
        std::fs::write(&path, "recents=files,bogus,settings\n").unwrap();
        again.load(&path);
        assert_eq!(again.recents(), [AppId::Files, AppId::Settings], "unknown names are skipped");
    }

    #[test]
    fn launches_are_kept_and_predict_the_next_app() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-launches"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quicksearch.ini");
        let ui = ui();
        let mut sp = QuickSearch::new(&ui, 1200.0, ui.s);
        sp.load(&path);
        assert!(sp.predicted_apps(1_000_000, 0, 3).is_empty(), "no history, no prediction");
        for app in [AppId::Files, AppId::Files, AppId::Terminal] {
            sp.note_launch(app);
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 60;
        // the last app used (Terminal) is already open, so Files is the prediction
        assert_eq!(sp.predicted_apps(now, 0, 3), [AppId::Files]);
        let mut again = QuickSearch::new(&ui, 1200.0, ui.s);
        again.load(&path);
        assert_eq!(again.launches.count("files"), 2, "the history survives a restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vertical_pull_opens_quicksearch_and_a_page_swipe_does_not() {
        let mut sh = shell();
        unlock(&mut sh);
        // sideways: the pager's, nothing opens
        drag(&mut sh, 900.0, 800.0, 800.0, 1000.0, 20.0);
        sh.event(Event::Down(900.0, 800.0), 2000.0);
        sh.event(Event::Move(700.0, 830.0), 2050.0);
        assert_eq!(sh.quicksearch.describe(), "hidden");
        sh.event(Event::Up(600.0, 840.0), 2100.0);
        settle(&mut sh, 2100.0);
        assert!(!sh.quicksearch.active());
        // a short slow pull springs back
        sh.event(Event::Down(600.0, 800.0), 3000.0);
        sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(20.0)), 3300.0);
        assert_eq!(sh.quicksearch.describe(), "dragging");
        sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(30.0)), 3600.0);
        sh.event(Event::Up(600.0, 800.0 + sh.ui.pt(30.0)), 3900.0);
        assert_eq!(sh.quicksearch.describe(), "closing");
        settle(&mut sh, 3900.0);
        assert!(!sh.quicksearch.active());
        // a pull past 40 pt opens, even when released slowly
        sh.event(Event::Down(600.0, 800.0), 5000.0);
        sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(25.0)), 5300.0);
        sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(50.0)), 5600.0);
        sh.event(Event::Up(600.0, 800.0 + sh.ui.pt(50.0)), 5900.0);
        assert_eq!(sh.quicksearch.describe(), "opening");
        assert!(sh.needs_frame());
        settle(&mut sh, 5900.0);
        assert!(sh.quicksearch.is_shown());
        assert!(sh.describe().contains("quicksearch=shown"));
        assert_eq!(sh.screen, Screen::Home);
        // the sideways drag paged to the App Library, the page after the last home page
        assert_eq!(sh.pager.page(), 1);
    }

    #[test]
    fn pulls_from_the_status_bar_the_home_zone_or_the_lock_screen_do_nothing() {
        let mut sh = shell();
        drag(&mut sh, 600.0, 800.0, 900.0, 0.0, 20.0);
        assert!(!sh.quicksearch.active(), "locked");
        unlock(&mut sh);
        drag(&mut sh, 300.0, 5.0, 300.0, 2000.0, 20.0);
        assert!(!sh.quicksearch.active(), "from the status bar");
        swipe(&mut sh, 600.0, 1915.0, 1500.0, 3000.0);
        settle(&mut sh, 3200.0);
        assert!(!sh.quicksearch.active(), "the home zone is the go-home swipe's");
        sh.open(AppId::Files, 4000.0);
        settle(&mut sh, 4000.0);
        drag(&mut sh, 600.0, 800.0, 900.0, 5000.0, 20.0);
        assert!(!sh.quicksearch.active(), "not inside an app");
    }

    #[test]
    fn typing_lists_results_and_rows_hit_test_in_order() {
        let mut sh = shell();
        unlock(&mut sh);
        let t = open(&mut sh, 1000.0);
        assert!(sh.quicksearch.rows().is_empty(), "empty query lists nothing");
        type_text(&mut sh, "fil", t);
        assert_eq!(sh.quicksearch.query(), "fil");
        let rows = sh.quicksearch.rows();
        assert_eq!(rows[0], (0, 0, Hit::App(AppId::Files)));
        assert_eq!(rows.last().unwrap().2, Hit::Terminal("fil".into()));
        assert_eq!(sh.quicksearch.top_hit(), Some(Hit::App(AppId::Files)));
        let rects = sh.quicksearch.row_rects(&sh.ui, sh.w);
        assert_eq!(rects.len(), rows.len());
        for (i, (at, (x, y, w, h))) in rects.iter().enumerate() {
            assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, x + w / 2.0, y + h / 2.0), Target::Row(at.0, at.1));
            if i > 0 {
                assert!(*y >= rects[i - 1].1 .1 + rects[i - 1].1 .3, "rows run top to bottom");
            }
        }
        let (_, _, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
        let last = rects.last().unwrap().1;
        assert!(last.1 + last.3 <= sh.ui.pt(CARD_TOP_PT) + ch, "the card covers every row");
        assert!(sh.ui.pt(CARD_TOP_PT) + ch < sh.quicksearch.keyboard_top(&sh.ui, sh.h), "and stops above the keyboard");
        let kb = sh.quicksearch.keyboard_top(&sh.ui, sh.h) + 10.0;
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, kb), Target::Keyboard);
        let (fx, fy, fw, fh) = sh.quicksearch.field_rect(&sh.ui, sh.w);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, fx + fw / 2.0, fy + fh / 2.0), Target::Field);
        let (kx, ky) = sh.quicksearch.clear_centre(&sh.ui, sh.w);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, kx, ky), Target::Clear);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, 1300.0), Target::Outside);
        // a long list is cut to what fits above the keyboard
        let titles: Vec<String> = (0..30).map(|i| format!("Clone Wars {i}")).collect();
        sh.quicksearch.set_deildu_titles(titles);
        type_text(&mut sh, "n", t + 10.0);
        let rects = sh.quicksearch.row_rects(&sh.ui, sh.w);
        let last = rects.last().unwrap().1;
        assert!(last.1 + last.3 < sh.quicksearch.keyboard_top(&sh.ui, sh.h));
        // clear empties the query and brings the suggestions back
        sh.quicksearch.note_launch(AppId::Files);
        tap(&mut sh, kx, ky, t + 100.0);
        assert_eq!(sh.quicksearch.query(), "");
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, kx, ky), Target::Field, "no clear button without text");
        let (sx, sy, sw, shh) = sh.quicksearch.suggestion_rect(&sh.ui, sh.w, 0);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, sx + sw / 2.0, sy + shh / 2.0), Target::Suggestion(0));
    }

    #[test]
    fn tapping_a_row_or_return_opens_the_hit_and_records_it() {
        let mut sh = shell();
        unlock(&mut sh);
        let t = open(&mut sh, 1000.0);
        type_text(&mut sh, "fil", t);
        let (_, (x, y, w, h)) = sh.quicksearch.row_rects(&sh.ui, sh.w)[0];
        sh.event(Event::Down(x + w / 2.0, y + h / 2.0), t + 100.0);
        assert!(sh.event(Event::Up(x + w / 2.0, y + 400.0), t + 130.0).is_empty(), "slid off the row");
        assert!(sh.quicksearch.is_shown());
        tap(&mut sh, x + w / 2.0, y + h / 2.0, t + 200.0);
        assert!(!sh.quicksearch.active());
        settle(&mut sh, t + 300.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        assert_eq!(sh.quicksearch.recents(), [AppId::Files]);
        swipe(&mut sh, 600.0, 1915.0, 1500.0, t + 2000.0);
        settle(&mut sh, t + 2200.0);
        let t = open(&mut sh, t + 4000.0);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, 1300.0), Target::Outside);
        type_text(&mut sh, "wi", t);
        let (kx, ky) = sh.quicksearch.keyboard.centre("return").unwrap();
        let top = sh.quicksearch.keyboard_top(&sh.ui, sh.h);
        let fx = tap(&mut sh, kx, top + ky, t + 100.0);
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Scan)], "return opens the top hit: Settings › Wi-Fi");
        settle(&mut sh, t + 200.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        assert_eq!(sh.settings.as_ref().unwrap().page(), crate::shell::pages::PageId::Wifi);
        assert_eq!(sh.quicksearch.recents(), [AppId::Settings, AppId::Files]);
    }

    #[test]
    fn keyboard_types_into_the_field_and_hide_keeps_the_card() {
        let mut sh = shell();
        unlock(&mut sh);
        let t = open(&mut sh, 1000.0);
        let top = sh.quicksearch.keyboard_top(&sh.ui, sh.h);
        for (i, k) in ["s", "e"].iter().enumerate() {
            let (x, y) = sh.quicksearch.keyboard.centre(k).unwrap();
            tap(&mut sh, x, top + y, t + 100.0 * i as f64);
        }
        assert_eq!(sh.quicksearch.query(), "se");
        assert_eq!(sh.quicksearch.top_hit(), Some(Hit::App(AppId::Settings)));
        let (x, y) = sh.quicksearch.keyboard.centre("hide").unwrap();
        tap(&mut sh, x, top + y, t + 500.0);
        assert!(sh.quicksearch.is_shown() && !sh.quicksearch.keyboard.shown());
        assert!(sh.needs_frame(), "the keyboard slides out");
        settle(&mut sh, t + 500.0);
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, top + 10.0), Target::Outside, "no keyboard to hit");
        let (fx, fy, fw, fh) = sh.quicksearch.field_rect(&sh.ui, sh.w);
        tap(&mut sh, fx + fw / 2.0, fy + fh / 2.0, t + 2000.0);
        assert!(sh.quicksearch.keyboard.shown(), "tapping the field brings it back");
        assert!(sh.next_wake(t + 2001.0).is_some(), "the caret blinks");
    }

    #[test]
    fn tap_outside_swipe_up_power_and_launch_dismiss() {
        let mut sh = shell();
        unlock(&mut sh);
        let t = open(&mut sh, 1000.0);
        tap(&mut sh, 600.0, 1300.0, t);
        assert_eq!(sh.quicksearch.describe(), "closing");
        settle(&mut sh, t + 30.0);
        assert!(!sh.quicksearch.active());
        let t = open(&mut sh, t + 2000.0);
        drag(&mut sh, 600.0, 1300.0, 700.0, t, 15.0);
        assert_eq!(sh.quicksearch.describe(), "closing");
        settle(&mut sh, t + 400.0);
        assert!(!sh.quicksearch.active());
        // a short tug snaps back
        let t = open(&mut sh, t + 3000.0);
        sh.event(Event::Down(600.0, 1300.0), t);
        sh.event(Event::Move(600.0, 1280.0), t + 200.0);
        assert!(sh.quicksearch.progress(t + 200.0) < 1.0);
        sh.event(Event::Up(600.0, 1280.0), t + 500.0);
        settle(&mut sh, t + 500.0);
        assert!(sh.quicksearch.is_shown());
        // the go-home swipe from the indicator dismisses too
        swipe(&mut sh, 600.0, 1915.0, 1400.0, t + 1000.0);
        assert_eq!(sh.quicksearch.describe(), "closing");
        settle(&mut sh, t + 1200.0);
        assert!(!sh.quicksearch.active() && sh.screen == Screen::Home);
        let t = open(&mut sh, t + 3000.0);
        assert_eq!(sh.event(Event::Power, t), vec![Effect::ScreenOff]);
        assert!(!sh.quicksearch.active() && sh.is_locked());
    }

    #[test]
    fn golden_gate_panel_is_centred_near_the_top_with_no_suggestions() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.quicksearch.note_launch(AppId::Deildu);
        sh.desk = Some(crate::shell::desk::Desk::new());
        open(&mut sh, 2000.0);
        let (x, y, w, h) = sh.quicksearch.card(&sh.ui, sh.w);
        let k = sh.quicksearch.k(&sh.ui);
        assert!((1.0..=MAC_K_MAX).contains(&k));
        assert_eq!(w, sh.ui.pt(520.0 * k), "520 pt, grown with the screen so the card is 0.37 of its width");
        assert!(w >= sh.ui.pt(520.0) && (w >= sh.w as f32 * 0.37 - 1.0 || k == MAC_K_MAX));
        assert!((x - (sh.w as f32 - w) / 2.0).abs() < 1.0);
        assert_eq!(y, sh.ui.pt(139.0 * k));
        assert_eq!(h, sh.ui.pt(sh.quicksearch.m().field * k), "the empty Golden Gate field shows no Assistant Suggestions, only the field");
    }

    #[test]
    fn golden_gate_card_grows_with_a_wide_screen_and_the_bifrost_does_not() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.desk = Some(crate::shell::desk::Desk::new());
        open(&mut sh, 2000.0);
        sh.quicksearch.resize(sh.ui.pt(520.0) * 4.0);
        assert_eq!(sh.quicksearch.k(&sh.ui), MAC_K_MAX.min(0.37 * sh.quicksearch.screen_w / sh.ui.pt(520.0)));
        assert!(sh.quicksearch.k(&sh.ui) > 1.0, "a screen four card widths wide enlarges the card");
        sh.quicksearch.resize(sh.ui.pt(520.0));
        assert_eq!(sh.quicksearch.k(&sh.ui), 1.0, "a narrow screen keeps the point size");
        sh.quicksearch.mac = false;
        sh.quicksearch.resize(sh.ui.pt(520.0) * 4.0);
        assert_eq!(sh.quicksearch.k(&sh.ui), 1.0, "the bifrost layout never scales");
    }

    #[test]
    fn golden_gate_results_have_chips_two_line_rows_and_filter_by_kind() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.desk = Some(crate::shell::desk::Desk::new());
        let t = open(&mut sh, 2000.0);
        assert!(sh.quicksearch.chip_rects(&sh.ui, sh.w).is_empty(), "no chips before anything is typed");
        type_text(&mut sh, "se", t);
        let qs = &sh.quicksearch;
        assert!(!qs.chips.is_empty(), "one chip per kind of result");
        assert!(qs.chips.iter().all(|(title, _)| !NOT_A_KIND.contains(title)), "the best match and the actions are not kinds");
        assert!(qs.sections.iter().all(|sec| sec.subs.len() == sec.rows.len()));
        assert!(qs.sections[0].subs[0].is_some(), "the top row has a second line");
        // the rows start under the chips, as drawn, and are the Golden Gate row height tall
        let chips = qs.chip_rects(&sh.ui, sh.w);
        assert_eq!(chips.len(), qs.chips.len());
        let rows = qs.row_rects(&sh.ui, sh.w);
        assert!(rows[0].1 .1 >= chips[0].1 + chips[0].3, "the first row is under the chips");
        assert_eq!(rows[0].1 .3, qs.px(&sh.ui, qs.m().row));
        let (cx, cy, cw, ch) = chips[0];
        assert_eq!(qs.hit(&sh.ui, sh.w, sh.h, cx + cw / 2.0, cy + ch / 2.0), Target::Chip(0));
        let title = qs.chips[0].0;
        let (x, y) = (cx + cw / 2.0, cy + ch / 2.0);
        // a tap on the chip narrows the list to that kind
        sh.quicksearch.down(&sh.ui, sh.w, sh.h, (x, y), t + 10.0, 0.0);
        assert_eq!(sh.quicksearch.up(&sh.ui, sh.w, sh.h, x, y, t + 40.0), None);
        assert_eq!(sh.quicksearch.chip, Some(title));
        assert_eq!(sh.quicksearch.sections.len(), 1, "one kind of result");
        // the same chip again shows every kind
        sh.quicksearch.down(&sh.ui, sh.w, sh.h, (x, y), t + 100.0, 0.0);
        sh.quicksearch.up(&sh.ui, sh.w, sh.h, x, y, t + 130.0);
        assert_eq!(sh.quicksearch.chip, None);
        assert!(sh.quicksearch.sections.len() >= 1);
    }

    #[test]
    fn golden_gate_card_is_drawn_with_chips_and_symbols_without_panicking() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.desk = Some(crate::shell::desk::Desk::new());
        let t = open(&mut sh, 2000.0);
        type_text(&mut sh, "wi", t);
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        sh.quicksearch.draw(&mut s, &sh.ui, sh.w, sh.h, t + 5000.0);
        let (cx, cy, cw, _) = sh.quicksearch.card(&sh.ui, sh.w);
        assert!(px.iter().any(|&p| p >> 24 != 0), "something was drawn over {cx} {cy} {cw}");
    }

    #[test]
    fn suggestions_show_the_recents_and_open_them() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.quicksearch.note_launch(AppId::Deildu);
        let t = open(&mut sh, 2000.0);
        assert_eq!(sh.quicksearch.recents(), [AppId::Deildu]);
        let (x, y, w, h) = sh.quicksearch.suggestion_rect(&sh.ui, sh.w, 0);
        assert!((x - (sh.quicksearch.card_x(&sh.ui, sh.w) + sh.ui.pt(SUGGEST_INSET_PT))).abs() < 1.0);
        let (_, _, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
        assert_eq!(ch, sh.ui.pt(FIELD_PT + HEADER_PT + SUGGEST_PT));
        sh.deildu_transport = crate::deildu::http::tests::Fake::new(vec![]);
        sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-deildu").into();
        tap(&mut sh, x + w / 2.0, y + h / 2.0, t);
        settle(&mut sh, t + 100.0);
        assert_eq!(sh.screen, Screen::App(AppId::Deildu));
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
    }

    #[test]
    fn deildu_city_and_terminal_hits_open_their_apps() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.term_cmd = &["/bin/sh"];
        sh.deildu_transport = crate::deildu::http::tests::Fake::new(vec![]);
        sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-hits").into();
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
        let t = open(&mut sh, 1000.0);
        assert!(sh.quicksearch_act(Hit::City("Tokyo"), t).is_empty());
        settle(&mut sh, t);
        sh.go_home(t + 1000.0);
        settle(&mut sh, t + 1000.0);
        sh.quicksearch_act(Hit::Deildu("Severance".into()), t + 2000.0);
        settle(&mut sh, t + 2000.0);
        assert_eq!(sh.screen, Screen::App(AppId::Deildu));
        assert!(sh.deildu.as_ref().unwrap().describe().contains("deildu_screen=library"));
        sh.go_home(t + 3000.0);
        settle(&mut sh, t + 3000.0);
        sh.quicksearch_act(Hit::Terminal("ls -la".into()), t + 4000.0);
        settle(&mut sh, t + 4000.0);
        assert_eq!(sh.screen, Screen::App(AppId::Terminal));
        assert!(sh.term.is_some() && sh.keyboard.shown());
        assert_eq!(sh.quicksearch.recents(), [AppId::Terminal, AppId::Deildu]);
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
    }

    #[test]
    fn draws_every_phase() {
        let mut sh = shell();
        unlock(&mut sh);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| {
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
            buf.clone()
        };
        sh.event(Event::Down(600.0, 800.0), 1000.0);
        sh.event(Event::Move(600.0, 850.0), 1050.0);
        let mid = draw(&mut sh, 1050.0);
        assert!(sh.quicksearch.active());
        sh.event(Event::Move(600.0, 900.0), 1100.0);
        sh.event(Event::Up(600.0, 900.0), 1120.0);
        draw(&mut sh, 1200.0);
        settle(&mut sh, 1120.0);
        sh.quicksearch.note_launch(AppId::Files);
        let shown = draw(&mut sh, 3000.0);
        let (cx, cy, _, _) = sh.quicksearch.card(&sh.ui, sh.w);
        let at = |buf: &[u32], x: f32, y: f32| buf[y as usize * 1200 + x as usize];
        assert_ne!(at(&shown, cx + 30.0, cy + 30.0), at(&mid, cx + 30.0, cy + 30.0), "the card settles into place");
        let kb_y = sh.quicksearch.keyboard_top(&sh.ui, sh.h) + 5.0;
        assert_ne!(at(&shown, 600.0, kb_y), at(&mid, 600.0, kb_y), "the keyboard is up");
        type_text(&mut sh, "se", 3000.0);
        sh.event(Event::Down(cx + 200.0, cy + sh.ui.pt(FIELD_PT + HEADER_PT + 10.0)), 3100.0);
        let pressed = draw(&mut sh, 3100.0);
        assert_ne!(pressed, shown);
        sh.event(Event::Up(cx + 200.0, cy + 2000.0), 3130.0);
        tap(&mut sh, 600.0, 1300.0, 3200.0);
        draw(&mut sh, 3300.0);
        settle(&mut sh, 3300.0);
        assert!(draw(&mut sh, 5000.0) != shown);
    }
    #[test]
    fn contact_results_match_names_phone_email_and_fold_icelandic_without_duplicates() {
        use super::super::contactstore::Contact;
        let people = vec![
            Contact { id: 7, name: "Þór Árnason".into(), phone: "5551234".into(), email: "thor@example.org".into(), notes: "private-only-value".into(), ..Contact::default() },
            Contact { id: 8, name: "Ægir".into(), phone: "5559876".into(), ..Contact::default() },
        ];
        for (query, id) in [("thor", 7), ("5551234", 7), ("example.org", 7), ("aegir", 8)] {
            let groups = results_with_contacts(query, &[], &people);
            assert_eq!(groups.first().unwrap().title, "Top Hit");
            assert!(matches!(groups[0].hits[0], Hit::Contact { id: got, .. } if got == id));
            assert_eq!(groups.iter().flat_map(|g| &g.hits).filter(|hit| matches!(hit, Hit::Contact { id: got, .. } if *got == id)).count(), 1);
        }
        assert!(results_with_contacts("", &[], &people).is_empty());
        assert!(!results_with_contacts("private-only-value", &[], &people).iter().flat_map(|g| &g.hits).any(|hit| matches!(hit, Hit::Contact { .. })));
    }

    #[test]
    fn a_contact_is_found_by_company_but_not_by_address_or_notes() {
        use super::super::contactstore::Contact;
        let people = vec![Contact {
            id: 9,
            name: "Ada King".into(),
            company: "Analytical Engines".into(),
            address: "12 Bryn Road".into(),
            notes: "private-only-value".into(),
            ..Contact::default()
        }];
        let found = |q: &str| results_with_contacts(q, &[], &people).iter().flat_map(|g| &g.hits).any(|hit| matches!(hit, Hit::Contact { id: 9, .. }));
        assert!(found("analytical"));
        assert!(!found("bryn road"));
        assert!(!found("private-only-value"));
    }

    #[test]
    fn quicksearch_refreshes_saved_contacts_and_opens_the_selected_person() {
        use super::super::contactstore::{self, Contact};
        let mut sh = shell();
        unlock(&mut sh);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-quicksearch-contacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("appdata/is.olibuijr.contacts/Contacts/contacts.tsv");
        let person = Contact { id: 42, name: "Þór".into(), phone: "5551234".into(), ..Contact::default() };
        contactstore::save(&path, &[person]).unwrap();
        sh.data_dir = dir.clone();
        let t = open(&mut sh, 1000.0);
        type_text(&mut sh, "thor", t);
        let mut image = crate::gfx::Image::new(sh.w as usize, sh.h as usize);
        sh.draw(&mut Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 }, t);
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/quicksearch-contacts.png"), super::super::screenshot::encode(&image)).unwrap();
        // contacts are read on a worker (`refresh_sources`): let the read land, as the frames would
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !matches!(sh.quicksearch.top_hit(), Some(Hit::Contact { .. })) && std::time::Instant::now() < end {
            let h = sh.h as i32;
            sh.quicksearch.tick(&sh.ui, h, t);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let hit = sh.quicksearch.top_hit().unwrap();
        assert!(matches!(hit, Hit::Contact { id: 42, .. }));
        sh.quicksearch_act(hit, t + 50.0);
        sh.go_home(t + 50.0);
        settle(&mut sh, t + 50.0);
        sh.go_home(t + 1000.0);
        settle(&mut sh, t + 1000.0);
        let t = open(&mut sh, t + 2000.0);
        type_text(&mut sh, "5551234", t);
        assert!(matches!(sh.quicksearch.top_hit(), Some(Hit::Contact { id: 42, .. })));
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The desktop Search material: the shared `HudWindow` material request (the presenter's blur under the named
/// menu tint, the solid colour with Reduce Transparency, the tint alone without a GPU presenter). The panel
/// fades in with `alpha`; the blur shape is declared only once it is fully opaque, so it never pops mid-fade.
pub(super) fn mac_glass(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, r: f32, alpha: u32) {
    panel_glass(s, bifrost_ui::material_request::ShellSurface::Search, x, y, w, h, r, alpha);
}

/// The material of the desktop's translucent panels (Search, the Apps grid, the Cmd+Tab strip): the request of
/// `surface` declares the presenter's blur, and the fill is the search tint, so the wallpaper shows through frosted.
// Rect, radius, alpha and the surface are independent draw parameters, mirroring `Surface::glass_rrect`.
#[allow(clippy::too_many_arguments)]
pub(super) fn panel_glass(s: &mut Surface, surface: bifrost_ui::material_request::ShellSurface, x: f32, y: f32, w: f32, h: f32, r: f32, alpha: u32) {
    // The field and the results card are one material: the presenter's blur under the search tint, which lets the
    // desktop show through (the menus' tint is nearly opaque); solid with Reduce Transparency.
    let fill = if bifrost_render::appearance::reduce_transparency() {
        MENU_SOLID.now()
    } else {
        if alpha == 255 && bifrost_render::glass::gpu() {
            bifrost_render::glass::push(super::control_center::shell_glass_rect(&surface.request((x, y, w, h), r)));
        }
        if bifrost_render::glass::gpu() {
            bifrost_render::glass::tint(SEARCH_TINT.now())
        } else {
            SEARCH_TINT.now()
        }
    };
    s.fill_rrect(x, y, w, h, r, scale(fill, alpha));
}

#[cfg(test)]
mod desktop_tests {
    use crate::shell::hwkeys::Shortcut;
    use crate::shell::keyboard::Out;
    use crate::shell::{AppId, Screen};

    /// Cmd+Space on the desktop opens Search over the app in front, and the keyboard types into it.
    #[test]
    fn cmd_space_opens_quicksearch_over_an_app_and_takes_typing() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::App(AppId::Settings);
        sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
        sh.quicksearch_tick(2000.0);
        assert!(sh.quicksearch.is_shown(), "Search opens over an app");
        sh.hw_type(Out::Text("cl".into()), 2100.0);
        assert_eq!(sh.quicksearch.query(), "cl");
    }

    /// Cmd+Space opens Search over an app the shell does not draw (an external window in front), and a second press closes it.
    #[test]
    fn cmd_space_opens_and_closes_quicksearch_over_an_external_app() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::App(AppId::External(3));
        sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
        sh.quicksearch_tick(2000.0);
        assert!(sh.quicksearch.is_shown(), "Search opens over an external app");
        sh.hw_shortcut(Shortcut::QuickSearch, 3000.0);
        assert!(!sh.quicksearch.active(), "the same chord closes it");
    }

    /// The Cmd release that ends the chord is not Home on the desktop: Search stays open and the app stays in front.
    #[test]
    fn a_lone_cmd_does_not_close_search_or_go_home_on_the_desktop() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::App(AppId::Settings);
        sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
        sh.quicksearch_tick(2000.0);
        assert!(sh.quicksearch.is_shown());
        sh.hw_shortcut(Shortcut::Home, 2100.0);
        assert!(sh.quicksearch.is_shown(), "Home from a stray Cmd release does not hide Search");
        assert_eq!(sh.screen, Screen::App(AppId::Settings), "and the windows stay where they are");
    }
}

#[cfg(test)]
mod file_actions_tests {
    use super::*;
    use crate::files::fs::{kind_of, Entry};

    fn ui() -> Ui {
        super::super::tests::ui()
    }

    fn entry(dir: &str, name: &str) -> Entry {
        Entry { kind: kind_of(name, false), name: name.into(), path: PathBuf::from(dir).join(name), is_dir: false, size: 1, mtime: 0, items: None }
    }

    /// The Files and Show in Folder groups of `query` over `files`, merged the way the panel merges them.
    fn file_groups(query: &str, files: &[crate::files::fs::Entry]) -> Vec<Group> {
        let folded: Vec<String> = files.iter().map(|e| fuzzy::fold(&e.name)).collect();
        let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files, files_folded: &folded, events: &[] };
        let (tables, hits) = build_tables(&corpus);
        let mut run = search_rank::Query::start(query, &sources_of(tables), SYNC_DEADLINE);
        run.wait();
        let merged = run.merge(PER_GROUP, &search_rank::Usage::default(), 0, &search_rank::NoSemantic);
        split_merged(&merged, &hits, None).2
    }

    #[test]
    fn groups_that_arrive_late_follow_the_ones_on_screen() {
        let u = ui();
        let mut q = QuickSearch::new(&u, 800.0, 1.0);
        q.shown = vec!["Applications", "Files"];
        let g = |title: &'static str| Group { title, hits: vec![] };
        let out = q.keep_shown_order(vec![g("Search in Apps"), g("Settings"), g("Applications"), g("Files")]);
        let titles: Vec<&str> = out.iter().map(|g| g.title).collect();
        assert_eq!(titles, vec!["Applications", "Files", "Settings", "Search in Apps"]);
    }

    #[test]
    fn a_file_match_comes_with_show_in_folder_for_the_top_match() {
        let files = vec![entry("/home/me/Docs", "Q1 report.pdf"), entry("/home/me/Pics", "Beach.png")];
        let groups = file_groups("report", &files);
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].title, "Files");
        assert_eq!(groups[0].hits, vec![Hit::File { path: PathBuf::from("/home/me/Docs/Q1 report.pdf"), name: "Q1 report.pdf".into() }]);
        assert_eq!(groups[1], Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/Docs"), name: "Q1 report.pdf".into() }] });
        assert!(file_groups("zzz", &files).is_empty());
        assert!(file_groups("  ", &files).is_empty());
    }

    #[test]
    fn space_previews_an_arrow_selected_file_and_types_otherwise() {
        let u = ui();
        let mut q = QuickSearch::new(&u, 800.0, 1.0);
        let file = Hit::File { path: PathBuf::from("/home/me/a.txt"), name: "a.txt".into() };
        let section = |u: &Ui| Section { header: u.text("Files", 20.0, Weight::Semibold, WHITE), rows: vec![(file.clone(), u.text("a.txt", 13.0, Weight::Regular, WHITE))], subs: vec![None] };
        q.sections = vec![section(&u)];
        assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), None, "no row selected: the space is typed");
        assert_eq!(q.query(), " ");
        q.sections = vec![section(&u)];
        q.selected = Some((0, 0));
        assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), Some(file), "Space previews the selected file");
        assert_eq!(q.query(), " ", "the query is untouched by the preview");
    }
}

#[cfg(test)]
mod dock_and_scroll_tests {
    use super::*;
    use crate::shell::keyboard::Out;
    use crate::shell::Event;

    /// A desktop shell with Search open and "se" typed (several kinds of result).
    fn typed() -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = Screen::Home;
        sh.open_quicksearch_hw(1000.0);
        sh.quicksearch_tick(2000.0);
        assert!(sh.quicksearch.is_shown());
        sh.quicksearch.key(&sh.ui, sh.h, Out::Text("se".into()), 2100.0);
        sh
    }

    /// The Dock top that leaves room for exactly `rows` result rows under the chips.
    fn dock_for_rows(sh: &Shell, rows: f32) -> f32 {
        let qs = &sh.quicksearch;
        let m = qs.m();
        qs.px(&sh.ui, m.card_top + m.field) + qs.list_fixed(&sh.ui) + rows * qs.px(&sh.ui, m.row) + sh.ui.pt(MAC_DOCK_MARGIN_PT)
    }

    fn row_h(sh: &Shell) -> f32 {
        sh.quicksearch.px(&sh.ui, sh.quicksearch.m().row)
    }

    #[test]
    fn the_dock_limits_the_card_on_the_desktop_and_the_screen_otherwise() {
        let mut sh = crate::shell::tests::desktop_shell();
        let top = sh.quicksearch_dock_top().expect("the Dock stands along the bottom and shows");
        assert!(top > 0.0 && top < sh.h as f32, "the Dock's top is on the screen");
        sh.desk.as_mut().unwrap().dock_preferences.position = crate::shell::desktop_dock::Position::Left;
        assert_eq!(sh.quicksearch_dock_top(), None, "a Dock on a side does not limit the height");
    }

    #[test]
    fn the_results_card_stays_above_the_dock_however_many_results() {
        let mut sh = typed();
        let dock = sh.quicksearch_dock_top().unwrap();
        let h = sh.h;
        let margin = sh.ui.pt(MAC_DOCK_MARGIN_PT);
        // from the real Dock to one that leaves four rows, three rows and one row
        for floor in [dock, dock_for_rows(&sh, 4.0), dock_for_rows(&sh, 3.0), dock_for_rows(&sh, 1.0)] {
            sh.quicksearch.set_dock_top(&sh.ui, h, Some(floor));
            let (_, cy, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
            assert!(cy + ch <= floor - margin + 0.5, "the card ends {} px above the Dock's top {floor}", floor - cy - ch);
            for (_, (_, ry, _, rh)) in sh.quicksearch.row_rects(&sh.ui, sh.w) {
                assert!(ry + rh <= cy + ch, "a row shows only inside the card");
            }
        }
        // the Dock hiding gives the screen's bottom back
        sh.quicksearch.set_dock_top(&sh.ui, h, None);
        let (_, cy, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
        assert!(cy + ch <= h as f32 - margin + 0.5);
    }

    #[test]
    fn rows_beyond_the_card_scroll_with_the_wheel_and_do_not_tap_through() {
        let mut sh = typed();
        let floor = dock_for_rows(&sh, 3.0);
        sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
        let row = row_h(&sh);
        let qs = &sh.quicksearch;
        assert!(qs.rows_h(&sh.ui) > qs.viewport_h(&sh.ui) + row, "more results than the card shows");
        assert!((qs.viewport_h(&sh.ui) - 3.0 * row).abs() < 1.0, "the card shows three rows");
        assert!(qs.row_rects(&sh.ui, sh.w).len() < qs.rows().len(), "rows below the card's edge cannot be hit");
        assert_eq!(qs.scroll, 0.0);
        // a wheel turned down (negative content travel) scrolls the list
        assert!(sh.quicksearch_wheel(-row, 3000.0));
        assert!((sh.quicksearch.scroll - row).abs() < 0.5);
        // the drag emulated for the notch lands on a row and must not open it
        let (_, (x, y, w, rh)) = sh.quicksearch.row_rects(&sh.ui, sh.w)[0];
        sh.event(Event::Down(x + w / 2.0, y + rh / 2.0), 3000.0);
        sh.event(Event::Move(x + w / 2.0, y + rh / 2.0 - 3.0), 3016.0);
        sh.event(Event::Up(x + w / 2.0, y + rh / 2.0 - 3.0), 3032.0);
        assert!(sh.quicksearch.is_shown(), "the emulated drag is dropped");
        // far past the end stops at the last row
        sh.quicksearch_wheel(-100_000.0, 4000.0);
        let qs = &sh.quicksearch;
        assert!((qs.scroll - qs.max_scroll(&sh.ui)).abs() < 0.5);
        let last = qs.row_rects(&sh.ui, sh.w).last().unwrap().0;
        assert_eq!(last, (qs.sections.len() - 1, qs.sections.last().unwrap().rows.len() - 1), "the last result shows at the end");
        // and back to the top
        sh.quicksearch_wheel(100_000.0, 5000.0);
        assert_eq!(sh.quicksearch.scroll, 0.0);
    }

    #[test]
    fn the_arrow_keys_keep_the_selected_row_in_view() {
        let mut sh = typed();
        let floor = dock_for_rows(&sh, 3.0);
        sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
        let (all, row) = (sh.quicksearch.rows().len(), row_h(&sh));
        for i in 0..all {
            sh.quicksearch.key(&sh.ui, sh.h, Out::Named("Down"), 3000.0 + i as f64);
            let qs = &sh.quicksearch;
            let at = qs.selected.expect("Down selects a row");
            let rect = qs.row_rects(&sh.ui, sh.w).into_iter().find(|(a, _)| *a == at).expect("the selected row is in view").1;
            assert!((rect.3 - row).abs() < 0.5, "the selected row shows whole");
        }
        assert!(sh.quicksearch.scroll > 0.0, "the list followed the selection down");
        for i in 0..all {
            sh.quicksearch.key(&sh.ui, sh.h, Out::Named("Up"), 4000.0 + i as f64);
            let qs = &sh.quicksearch;
            let at = qs.selected.expect("Up keeps a row selected");
            assert!(qs.row_rects(&sh.ui, sh.w).iter().any(|(a, r)| *a == at && (r.3 - row).abs() < 0.5), "the selected row shows whole");
        }
        assert_eq!(sh.quicksearch.scroll, 0.0, "the first row is back at the top");
    }

    #[test]
    fn a_new_query_scrolls_back_to_the_top() {
        let mut sh = typed();
        let floor = dock_for_rows(&sh, 3.0);
        sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
        sh.quicksearch_wheel(-200.0, 3000.0);
        assert!(sh.quicksearch.scroll > 0.0);
        sh.quicksearch.key(&sh.ui, sh.h, Out::Text("t".into()), 3100.0);
        assert_eq!(sh.quicksearch.scroll, 0.0);
    }
}
