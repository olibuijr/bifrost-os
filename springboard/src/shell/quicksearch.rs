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

#[path = "quicksearch_dict.rs"]
pub(super) mod dict;
#[path = "quicksearch_files.rs"]
mod files_walk;

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

/// A shortcut run from Search (`Hit::Shortcut`) that finished on its thread: the app's slot, the shortcut's title and
/// what the run gave (its dialog text, or why it failed).
struct ShortcutRun {
    slot: u8,
    title: String,
    outcome: Result<Option<String>, String>,
}

/// The shortcut runs a panel started that are still on their threads, and the finished ones whose banner the shell has
/// not posted yet (`Shell::shortcut_results_tick`). Shared between the panel and its run threads only, so one shell
/// never polls for (or posts) the runs of another in the same process.
type ShortcutRuns = Arc<std::sync::Mutex<(usize, Vec<ShortcutRun>)>>;

fn shortcut_runs(runs: &ShortcutRuns) -> std::sync::MutexGuard<'_, (usize, Vec<ShortcutRun>)> {
    runs.lock().unwrap_or_else(|e| e.into_inner())
}

/// One shortcut run counted in its panel's `ShortcutRuns` from before its thread starts until the thread ends, however
/// it ends: a panic in the app's intent call unwinds through the drop, and a thread that could not be started drops its
/// closure (and the guard in it) at once. A count that never dropped kept `next_wake` polling every 50 ms forever.
struct ShortcutRunGuard(ShortcutRuns);

impl ShortcutRunGuard {
    fn new(runs: &ShortcutRuns) -> ShortcutRunGuard {
        shortcut_runs(runs).0 += 1;
        ShortcutRunGuard(Arc::clone(runs))
    }
}

impl Drop for ShortcutRunGuard {
    fn drop(&mut self) {
        let mut g = shortcut_runs(&self.0);
        g.0 = g.0.saturating_sub(1);
    }
}

/// Runs `f` on a thread of its own, counted in `runs` as a shortcut run while it is out (`ShortcutRunGuard`). False
/// when the thread could not be started (the count is back where it was).
fn spawn_shortcut_run(runs: &ShortcutRuns, f: impl FnOnce() + Send + 'static) -> bool {
    let guard = ShortcutRunGuard::new(runs);
    let started = std::thread::Builder::new().name("intent-retry".into()).spawn(move || {
        let _guard = guard;
        f();
    });
    match started {
        Ok(_) => true,
        Err(e) => {
            bifrost_core::log::shell_logger("quicksearch").error(format!("shortcut run thread: {e}"));
            false
        }
    }
}


/// The banner of a finished shortcut, as the reference shows a shortcut's result: its name over its output, "Done" when
/// it gave none, or the problem when it failed.
fn shortcut_banner(title: &str, outcome: &Result<Option<String>, String>) -> (String, String) {
    match outcome {
        Ok(Some(said)) if !said.trim().is_empty() => (title.to_string(), said.trim().to_string()),
        Ok(_) => (title.to_string(), "Done".to_string()),
        Err(e) => (title.to_string(), format!("There was a problem running the shortcut: {e}")),
    }
}
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
/// wide screen; on a wider one the card keeps this size (`QuickSearch::k` is 1).
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
/// Golden Gate: the card keeps its measured point size on every screen (owner, 2026-11-11: a card that grew with the
/// screen was too big). `MAC_CARD_FRACTION` is the share of a 1408 pt wide screen the 520 pt card takes; with
/// `MAC_K_MAX` at 1 the layout never grows past its point size.
const MAC_CARD_FRACTION: f32 = 0.37;
const MAC_K_MAX: f32 = 1.0;
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
    /// a file under the Files root or the home folder, previewed in Files; `modified` is its modification time (Unix
    /// seconds, 0 when unknown), shown on the row
    File { path: PathBuf, name: String, modified: i64 },
    /// a folder under the Files root or the home folder, opened in Files; Cmd+Return shows it in the folder that holds
    /// it. `modified` as for a file
    Folder { path: PathBuf, name: String, modified: i64 },
    /// a dictionary definition of `word` (`quicksearch_dict`): opens the word in Dictionary; Space shows the whole
    /// definition in the Look Up popover. `dictionary` is empty for the "look it up" row of a word no dictionary has.
    Definition { word: String, dictionary: String, preview: String },
    /// "Show in Folder": opens Files at `folder`, the folder that holds the file (or folder) `name`
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
    /// `kind` is the file type's name ("PDF document", "" when unknown); `thumb` the thumbnail path in the app's data dir;
    /// `snippet` the indexed text around the match when the title does not match (a mail found by its body), else "".
    Indexed { app_id: String, unique_id: String, domain: String, title: String, kind: String, thumb: String, snippet: String },
    /// a calendar event: opens the Calendar app
    Event { title: String },
}

#[cfg(test)]
mod app_section_title_tests;

/// The label of an indexed result: its title, then its content type when it has one ("Q1 report · PDF document").
fn indexed_label(title: &str, kind: &str) -> String {
    if kind.is_empty() {
        title.to_string()
    } else {
        format!("{title} · {kind}")
    }
}

#[cfg(test)]
mod indexed_label_tests;

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
        P::Display => Symbol::Tile(Icon::Display),
        P::Appearance => Symbol::Tile(Icon::Appearance),
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
fn row_text(hit: &Hit, apps_name: impl Fn(AppId) -> String, (now, offset, h24): (i64, i32, bool)) -> (String, String) {
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
        Hit::File { path, name, modified } => {
            // the kind, where it is and when it last changed ("Image · Downloads · Today at 09:41")
            let mut parts = vec![crate::files::fs::kind_of(name, false).label().to_string()];
            parts.extend(Some(folder_of(path)).filter(|f| !f.is_empty()));
            if *modified > 0 {
                parts.push(crate::files::fs::date_text_in(*modified, now, offset, h24));
            }
            (name.clone(), parts.join(" \u{b7} "))
        }
        Hit::Folder { path, name, modified } => {
            // "Folder · olibuijr · Today at 09:41"
            let mut parts = vec![crate::files::fs::Kind::Folder.label().to_string()];
            parts.extend(Some(folder_of(path)).filter(|f| !f.is_empty()));
            if *modified > 0 {
                parts.push(crate::files::fs::date_text_in(*modified, now, offset, h24));
            }
            (name.clone(), parts.join(" \u{b7} "))
        }
        Hit::Definition { word, dictionary, .. } if dictionary.is_empty() => (word.clone(), "Look up in Dictionary".to_string()),
        Hit::Definition { word, preview, .. } => (word.clone(), if preview.is_empty() { "Dictionary".to_string() } else { preview.clone() }),
        Hit::Reveal { folder, name } => (format!("Show {name} in Folder"), folder.display().to_string()),
        Hit::Shortcut { title, app_name, .. } => (title.clone(), format!("Shortcut \u{b7} {app_name}")),
        Hit::SettingsIntent { id, title } => (title.to_string(), super::settings_intents::find(id).map_or("Settings", |i| i.description).to_string()),
        Hit::SettingsAction { intent, title } => (title.clone(), action_line(intent).to_string()),
        Hit::MediaTransport { title, .. } => (title.clone(), "Control the playing media".to_string()),
        // a match in the body shows the text around it, so the row says why it is listed
        Hit::Indexed { title, snippet, .. } if !snippet.is_empty() => (title.clone(), snippet.clone()),
        Hit::Indexed { title, kind, .. } => (title.clone(), kind.clone()),
    }
}

/// The filter chips of a result list: the title of each group that is a kind of result, once each, in list order. The
/// Top Hit counts as its own kind ("settings" with the Settings app on top shows an Applications chip, as in the
/// reference), since the row is taken out of its section.
fn chips_of(groups: &[Group]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    for g in groups {
        let kind = if g.title == "Top Hit" { g.hits.first().and_then(kind_of_hit) } else { Some(g.title) };
        let Some(kind) = kind else { continue };
        if !NOT_A_KIND.contains(&kind) && !out.contains(&kind) && !g.hits.is_empty() {
            out.push(kind);
        }
    }
    out
}

/// The section a hit is listed in when it is not the Top Hit (the group titles `build_tables` and the other sources
/// give), or None for a hit whose section is not fixed (an app's indexed items are titled with the app's name).
fn kind_of_hit(hit: &Hit) -> Option<&'static str> {
    Some(match hit {
        Hit::App(_) => "Applications",
        Hit::Settings(_) => "Settings",
        Hit::Contact { .. } => "Contacts",
        Hit::City(_) => "Clock",
        Hit::Deildu(_) => "Deildu",
        Hit::File { .. } | Hit::Folder { .. } => "Files",
        Hit::Event { .. } => "Calendar",
        Hit::Definition { .. } => "Dictionary",
        Hit::Answer(_) => "Calculator",
        Hit::Shortcut { .. } | Hit::SettingsIntent { .. } | Hit::SettingsAction { .. } | Hit::MediaTransport { .. } => "Shortcuts",
        _ => return None,
    })
}

/// The list filtered to the chip `chip`: that kind's sections, with the Top Hit leading them when it is of that kind
/// (it was taken out of its section, so the filtered list would miss its best row otherwise). The Files chip keeps
/// the "Show in Folder" action of its first file, after the files.
fn filter_to_chip(groups: Vec<Group>, chip: &'static str) -> Vec<Group> {
    let mut lead: Vec<Hit> = vec![];
    let mut out: Vec<Group> = vec![];
    let mut action: Option<Group> = None;
    for g in groups {
        if g.title == "Top Hit" && g.hits.first().and_then(kind_of_hit) == Some(chip) {
            lead.extend(g.hits);
        } else if g.title == chip {
            out.push(g);
        } else if g.title == "Show in Folder" && chip == "Files" {
            action = Some(g);
        }
    }
    if !lead.is_empty() {
        if let Some(first) = out.first_mut() {
            lead.append(&mut first.hits);
            first.hits = lead;
        } else {
            out.push(Group { title: chip, hits: lead });
        }
    }
    if !out.is_empty() {
        out.extend(action);
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

/// The Settings intents as Shortcuts rows: each opens its pane. Searched by title, listed with its description. A pane
/// that already has its own Settings row (`settings::PAGES`) gets no second, intent row for the same target.
fn settings_intent_rows(hits: &mut BTreeMap<String, Hit>) -> Vec<(search_rank::Item, String)> {
    super::settings_intents::CATALOGUE
        .iter()
        .filter(|intent| !settings::PAGES.iter().any(|name| settings::named_page(name) == Some(intent.page)))
        .map(|intent| table_row(hits, format!("settings-intent:{}", intent.id), intent.title, intent.description.to_string(), search_rank::Kind::Settings, Hit::SettingsIntent { id: intent.id, title: intent.title }, fuzzy::fold(intent.title)))
        .collect()
}

/// The shell action intents (`settings_actions`) Search lists: the Wi-Fi scan, the two Wi-Fi switch states, and one
/// "Set Wallpaper to" row per picture of `wallpapers` (the catalogue as (id, name)).
fn settings_action_rows(hits: &mut BTreeMap<String, Hit>, wallpapers: &[(String, String)]) -> Vec<(search_rank::Item, String)> {
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
    for (id, name) in wallpapers {
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
mod step_row_tests;

#[cfg(test)]
mod symbol_and_line_tests;

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
    results_folded(query, deildu, &folded, contacts, &contacts_folded, &[])
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
    /// the wallpaper catalogue as (id, name), for the "Set Wallpaper to" rows
    wallpapers: &'a [(String, String)],
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
///
/// In the shell's own tests a query waits for every source (`TEST_DEADLINE`): a test types and looks at the rows at
/// once, with no frames in between, so with the 4 ms of the panel a source whose thread was not scheduled in time on a
/// busy machine was simply missing from what the test saw (a contact typed by its number gave "Search in Terminal" as
/// the first row in one run of four of the whole test binary). The wait ends as soon as the last source answers, so
/// the tests are no slower for it.
const RUN_GRACE: std::time::Duration = if cfg!(test) { TEST_DEADLINE } else { std::time::Duration::from_millis(4) };
/// The longest a query waits for its slowest source before it goes on without it.
const RUN_DEADLINE: std::time::Duration = if cfg!(test) { TEST_DEADLINE } else { std::time::Duration::from_millis(300) };
/// The wait of a query merged at once (`results`, used by the look-up popover and by tests).
const SYNC_DEADLINE: std::time::Duration = if cfg!(test) { TEST_DEADLINE } else { std::time::Duration::from_millis(300) };
/// The deadline of a query in the shell's own tests: long enough that every source's thread has run, however busy the
/// machine is.
const TEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// One section of results as a search source: its rows (the item the ranker sees, and the folded text the fuzzy matcher
/// tests). It owns its rows, so a query can run it on its own thread. The hit a row opens is kept by `build_tables`.
struct Table {
    name: &'static str,
    rows: Vec<(search_rank::Item, String)>,
    /// Per row (same index): folded other names the row is found by only when the query starts one of their words (an
    /// app's aliases, `apps::search_aliases`), never by the fuzzy matcher's initials or scattered letters, so "wi" does
    /// not find the browser through "web internet" and "se" does not find Term through "shell". Empty: none.
    prefixes: Vec<String>,
    /// Per row (same index): the digits of its phone number (`phone_digits`), which a query of digits alone finds
    /// anywhere inside, as a number is searched by any part of it ("1234" finds 555 1234). Empty: none.
    phones: Vec<String>,
    /// Rows match only when the query is contiguous in their text or starts one of its words, never by scattered
    /// letters: every section but Applications and Deildu. The Settings panes' text carries many keyword aliases, so
    /// "lantern" does not find "Display & Brightness" through letters spread over them, and "aapl" does not find the
    /// Shortcuts row "Set Wallpaper to ..." (E2E12 1).
    strict: bool,
    /// Rows that are not `strict` match as a name does (`name_match`): by a run of the name or its words' initials, not
    /// by letters scattered through it. The Applications section; Deildu's titles keep the loose match.
    names: bool,
}

/// Whether the folded query `q` matches the folded name `name` of an app: the name or one of its words starts with
/// it, it is a run of the name anywhere ("book" in "Font Book", "color m" in "Digital Color Meter"), or it is the
/// initials of the name's words in order from the first ("dcm", "as" for App Store). Letters scattered through the
/// name do not match: "tall" found "Digital Color Meter" through digiTAL coLor (E2E6 5).
fn name_match(q: &str, name: &str) -> bool {
    if q.is_empty() {
        return false;
    }
    if name.contains(q) {
        return true;
    }
    let initials: String = name.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next()).collect();
    q.chars().count() > 1 && initials.starts_with(q)
}

/// Whether the folded query `q` matches folded `text` well enough for a strict table: a contiguous run of the text
/// that starts at a word start (the text's start or after a non-alphanumeric character), so a prefix ("bright"), a
/// later word ("wallp") and a run over words ("dark m" in "... dark mode ...") match, but a run inside a word does
/// not ("words" does not find "Open Passwords", E2E13 4).
fn strict_match(q: &str, text: &str) -> bool {
    !q.is_empty()
        && text.char_indices().any(|(i, _)| text[i..].starts_with(q) && text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric()))
}

/// The digits of a phone number, its spaces, `+`, `-` and other punctuation dropped.
fn phone_digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// The digits of a query made only of digits, spaces, `+` and `-` (with at least one digit): a phone number search.
fn phone_query(q: &str) -> Option<String> {
    let q = q.trim();
    (q.chars().any(|c| c.is_ascii_digit()) && q.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '+' | '-'))).then(|| phone_digits(q))
}

impl search_rank::Source for Table {
    fn name(&self) -> &'static str {
        self.name
    }

    fn search(&self, query: &str) -> Vec<search_rank::Item> {
        let m = fuzzy::Matcher::new(query);
        let q = fuzzy::fold(query.trim());
        let digits = phone_query(query);
        self.rows
            .iter()
            .enumerate()
            .filter(|(i, (_, folded))| {
                let hit = if self.strict {
                    strict_match(&q, folded)
                } else if self.names {
                    name_match(&q, folded)
                } else {
                    m.rank_folded(folded).is_some()
                };
                hit || self.prefixes.get(*i).is_some_and(|p| starts_a_word(p, &q))
                    || digits.as_deref().is_some_and(|d| self.phones.get(*i).is_some_and(|p| !p.is_empty() && p.contains(d)))
            })
            .map(|(_, (item, _))| item.clone())
            .collect()
    }
}

/// Whether `q` (folded) is the start of a word of `words` (folded, space separated), running on over later words
/// ("font b" starts "font book").
fn starts_a_word(words: &str, q: &str) -> bool {
    !q.is_empty() && (words.starts_with(q) || words.match_indices(' ').any(|(i, _)| words[i + 1..].starts_with(q)))
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
    // an app is found by its name (`name_match`) and by its aliases ("terminal" for Term, "preview" for Viewer) as word prefixes
    let mut app_prefixes = Vec::new();
    let apps_rows = apps()
        .iter()
        .map(|a| {
            let aliases = super::apps::search_aliases(a.icon);
            app_prefixes.push(fuzzy::fold(aliases));
            table_row(&mut hits, app_usage_key(a.id), a.name, aliases.to_string(), search_rank::Kind::App, Hit::App(a.id), fuzzy::fold(a.name))
        })
        .collect();
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
    let contact_phones = c.contacts.iter().map(|contact| phone_digits(&contact.phone)).collect();
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
    shortcut_rows.extend(settings_action_rows(&mut hits, c.wallpapers));
    shortcut_rows.extend(media_transport_rows(&mut hits));
    let file_rows = c
        .files
        .iter()
        .zip(c.files_folded)
        .map(|(e, f)| {
            let (path, name, modified) = (e.path.clone(), e.name.clone(), e.mtime);
            let hit = if e.is_dir { Hit::Folder { path, name, modified } } else { Hit::File { path, name, modified } };
            table_row(&mut hits, format!("file:{}", e.path.display()), &e.name, String::new(), search_rank::Kind::File, hit, f.clone())
        })
        .collect();
    let event_rows = c
        .events
        .iter()
        .map(|e| {
            let text = format!("{} {}", e.location, e.notes);
            let folded = fuzzy::fold(&format!("{} {}", e.title, e.location));
            table_row(&mut hits, format!("event:{}", e.id), &e.title, text, search_rank::Kind::Other, Hit::Event { title: e.title.clone() }, folded)
        })
        .collect();
    // an app's name is matched by a run anywhere in it or by its initials (its aliases by word prefix), a Deildu title
    // by letters in order; Settings, Clock, Contacts, Shortcuts, Files and Calendar need a prefix, a word start or a
    // contiguous run from one, so "aapl" does not list "Set Wallpaper to ..." in Shortcuts (E2E12 1)
    let tables = vec![
        Table { name: "Applications", rows: apps_rows, prefixes: app_prefixes, phones: Vec::new(), strict: false, names: true },
        Table { name: "Settings", rows: settings_rows, prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false },
        Table { name: "Clock", rows: city_rows, prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false },
        Table { name: "Deildu", rows: deildu_rows, prefixes: Vec::new(), phones: Vec::new(), strict: false, names: false },
        Table { name: "Contacts", rows: contact_rows, prefixes: Vec::new(), phones: contact_phones, strict: true, names: false },
        Table { name: "Shortcuts", rows: shortcut_rows, prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false },
        Table { name: "Files", rows: file_rows, prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false },
        Table { name: "Calendar", rows: event_rows, prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false },
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

/// What "Show in Folder" hands to Files for the file `name` in `folder`: the file, so it is shown selected, or the
/// folder itself when the file is gone since the search listed it (or `name` is not a plain file name).
fn reveal_target(folder: &Path, name: &str) -> PathBuf {
    let plain = !name.is_empty() && Path::new(name).file_name().is_some_and(|n| n == name);
    let file = folder.join(name);
    if plain && file.exists() { file } else { folder.to_path_buf() }
}

/// The banner of a file action that failed: the file's name with what did not happen, over the reason.
fn file_problem_banner(path: &Path, what: &str, why: &str) -> (String, String) {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    (format!("\u{201c}{name}\u{201d} {what}"), why.to_string())
}

/// Splits a merged query into the Top Hit group, the sections, and the files group (with "Show in Folder" after it,
/// `show_in_folder`). `top` is the hit shown as the top hit; it is taken out of the sections, so no row is listed twice.
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
            files.push(Group { title: "Files", hits: rows });
        } else {
            sections.push(Group { title: section.source, hits: rows });
        }
    }
    let files = show_in_folder(top_group.as_ref(), files);
    (top_group, sections, files)
}

/// The path and name of a file or folder row.
fn file_of(hit: &Hit) -> Option<(&Path, &str)> {
    match hit {
        Hit::File { path, name, .. } | Hit::Folder { path, name, .. } => Some((path.as_path(), name.as_str())),
        _ => None,
    }
}

/// `files` (the Files group, if any) with the "Show in Folder" action after it: one row, for the first file or folder
/// of the list as it is shown, which is the Top Hit `top` when that is a file and else the first row of the Files
/// group. That is the file Cmd+Return shows (`QuickSearch::reveal_hit`), so the row says what the chord does.
///
/// The action used to be built from the Files group's own rows alone, after the Top Hit had been taken out of them. A
/// query whose only file match was the Top Hit (the usual case: "note.txt", "file-37") left that group empty, and so
/// no "Show ... in Folder" row was listed at all (E2E6 3). An action already in `files` is replaced.
fn show_in_folder(top: Option<&Group>, mut files: Vec<Group>) -> Vec<Group> {
    files.retain(|g| g.title != "Show in Folder");
    let listed = files.iter().filter(|g| g.title == "Files").flat_map(|g| &g.hits);
    let action = top.iter().flat_map(|g| &g.hits).chain(listed).find_map(file_of).map(|(path, name)| reveal_of(path, name));
    files.extend(action.map(|hit| Group { title: "Show in Folder", hits: vec![hit] }));
    files
}

/// The name the folder icon is asked for and kept under (`file_icons_wanted`, `set_file_icon`): no file is named so.
const FOLDER_ICON: &str = "/";

/// What the document icon of a file depends on: its name's extension, lower case ("" for a name without one); the
/// folder icon has a key of its own (`FOLDER_ICON`).
fn file_icon_key(name: &str) -> String {
    if name == FOLDER_ICON {
        return FOLDER_ICON.to_string();
    }
    Path::new(name).extension().map_or_else(String::new, |e| e.to_string_lossy().to_lowercase())
}

/// The name a file or folder row asks its icon by: the file's name, or `FOLDER_ICON` for a folder.
fn icon_name(hit: &Hit) -> Option<&str> {
    match hit {
        Hit::File { name, .. } => Some(name.as_str()),
        Hit::Folder { .. } => Some(FOLDER_ICON),
        _ => None,
    }
}

/// The "Show in Folder" hit of the file at `path` named `name`.
fn reveal_of(path: &Path, name: &str) -> Hit {
    Hit::Reveal { folder: path.parent().map_or_else(|| path.to_path_buf(), Path::to_path_buf), name: name.to_string() }
}

/// What Search finds at the path a document of the Files app is indexed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnDisk {
    /// a plain file Search may list, with its modification time (Unix seconds, at least 1)
    File(i64),
    /// nothing is there any more: the file was deleted, moved or renamed since it was indexed
    Gone,
    /// a file Search does not list: one in a folder Settings > Search excludes, or any file while the Files section is off
    Hidden,
    /// something Search cannot say more about (a folder, a path that could not be looked at): the item stays as it is
    Unknown,
}

/// What is at `path` now, for a document of the Files app (`OnDisk`). One `stat` of the path.
fn indexed_file_on_disk(path: &Path) -> OnDisk {
    let prefs = super::search_prefs::current();
    if !prefs.source_on("Files") || prefs.folder_excluded(path) {
        return OnDisk::Hidden;
    }
    match std::fs::metadata(path) {
        Ok(md) if md.is_file() => OnDisk::File(md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(1, |d| (d.as_secs() as i64).max(1))),
        Ok(_) => OnDisk::Unknown,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnDisk::Gone,
        Err(_) => OnDisk::Unknown,
    }
}

/// `path` without the parts that do not change what it names (`.`, doubled and trailing separators), so two spellings
/// of one path compare equal. Links are not resolved: nothing is read from the disk.
fn plain_path(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Turns the documents the Files app put into the index (a file it opened; the item's id is the file's path) into
/// file rows. As an indexed item such a file read as a row of the Files app with its kind alone: no folder or date,
/// the app's icon, and no "Show in Folder" (E2E5 2 to 4: the only row "note.txt" gave). `on_disk` says what is at an
/// item's path now (`indexed_file_on_disk`):
///
/// - a file Search may list becomes a file row, unless the walk (or the Top Hit `top`) lists that file already: one
///   file gives one row, the walk's;
/// - a file that is gone is not listed, and its id is returned (third) so the caller drops it from the index. In
///   0.10.51 such an item stayed as the app's row, so the index's leftovers of deleted files were listed for ever, and
///   one named like a file that exists read as that file listed twice (E2E6 1 and 2: "note.txt" beside the stale
///   "note.txt" of an earlier folder, "pic.png" twice, "e2e7-notes.txt", "bad-huge.gif");
/// - a file Search does not list (an excluded folder, the Files section off) is not listed as the app's row either;
/// - anything else (another app's item, an id that is no absolute path, a folder) stays as it is.
///
/// Each section keeps at most `PER_GROUP` rows after this, and at most `PER_GROUP` files join the walk's.
fn fold_indexed_files(documents: Vec<Group>, mut files: Vec<Group>, top: Option<&Group>, on_disk: impl Fn(&Path) -> OnDisk) -> (Vec<Group>, Vec<Group>, Vec<String>) {
    let mut found: Vec<Hit> = vec![];
    let mut gone: Vec<String> = vec![];
    let documents: Vec<Group> = documents
        .into_iter()
        .filter_map(|mut g| {
            g.hits.retain(|h| {
                let Hit::Indexed { app_id, unique_id, .. } = h else { return true };
                let path = Path::new(unique_id);
                if app_id != super::workspace::FILES_APP || !path.is_absolute() {
                    return true;
                }
                let Some(name) = path.file_name() else { return true };
                match on_disk(path) {
                    OnDisk::File(at) => found.push(Hit::File { path: path.to_path_buf(), name: name.to_string_lossy().into_owned(), modified: at }),
                    OnDisk::Gone => gone.push(unique_id.clone()),
                    OnDisk::Hidden => {}
                    OnDisk::Unknown => return true,
                }
                false
            });
            g.hits.truncate(PER_GROUP);
            (!g.hits.is_empty()).then_some(g)
        })
        .collect();
    let path_of = |h: &Hit| file_of(h).map(|(path, _)| plain_path(path));
    let mut listed: Vec<PathBuf> = top.iter().flat_map(|g| &g.hits).chain(files.iter().flat_map(|g| &g.hits)).filter_map(path_of).collect();
    found.retain(|h| {
        let new = path_of(h).is_some_and(|p| !listed.contains(&p));
        listed.extend(path_of(h).filter(|_| new));
        new
    });
    found.truncate(PER_GROUP);
    if !found.is_empty() {
        match files.iter_mut().find(|g| g.title == "Files") {
            Some(g) => g.hits.extend(found),
            None => files.insert(0, Group { title: "Files", hits: found }),
        }
    }
    (documents, show_in_folder(top, files), gone)
}

/// Drops the documents `gone` (their ids) of the Files app from the index: files that were deleted since the app
/// indexed them (`fold_indexed_files`). On a thread of its own, since the index is saved after it (`spotindex::flush`
/// makes the app's whole list into text); the rows are left out of the list on show whether or not this has run yet.
fn forget_gone_files(gone: Vec<String>) {
    if gone.is_empty() {
        return;
    }
    let forget = move || {
        super::spotindex::apply(super::workspace::FILES_APP, bifrost_proto::ToShell::QuickSearchDelete { kind: bifrost_proto::quicksearch::DELETE_IDS, values: gone });
        // the tests keep the index in memory: the saver would write under the real data dir
        if !cfg!(test) {
            super::spotindex::flush();
        }
    };
    if let Err(e) = std::thread::Builder::new().name("quicksearch-forget".into()).spawn(forget) {
        bifrost_core::log::shell_logger("quicksearch").error(format!("index clean-up thread: {e}"));
    }
}

/// The definition row of `query` as its group ("Dictionary"), and whether the query asked for it ("define x").
fn dictionary_group(query: &str) -> Option<(Group, bool)> {
    dict::lookup(query).map(|(d, explicit)| (Group { title: "Dictionary", hits: vec![Hit::Definition { word: d.word, dictionary: d.dictionary, preview: d.preview }] }, explicit))
}

/// Whether the definition group `g` is for the query's own word (an exact headword) while the Top Hit `top` was
/// matched only by scattered letters: the headword ranks first then. A Top Hit found by an alias (an app's
/// `search_aliases`, a Settings pane's keywords), a prefix, a word start or a contiguous run of its title keeps its
/// place, so "terminal" stays the Term app and "preview" the Viewer.
fn headword_beats_top(query: &str, g: &Group, top: Option<&Group>) -> bool {
    let q = fuzzy::fold(query.trim());
    let exact = g.hits.iter().any(|h| matches!(h, Hit::Definition { word, dictionary, .. } if !dictionary.is_empty() && fuzzy::fold(word) == q));
    let Some(top_hit) = top.and_then(|t| t.hits.first()) else { return false };
    exact && !top_matched_well(&q, top_hit)
}

/// Whether the folded query `q` matches `hit` better than by scattered letters: its title by a contiguous run from a
/// word start (`strict_match`), or one of its aliases (an app's aliases as word prefixes, a Settings pane's keywords strictly).
fn top_matched_well(q: &str, hit: &Hit) -> bool {
    if q.is_empty() {
        return false;
    }
    let name_of = |app: AppId| apps().iter().find(|a| a.id == app).map_or(String::new(), |a| a.name.to_string());
    let (title, _) = row_text(hit, name_of, (0, 0, true));
    if strict_match(q, &fuzzy::fold(&title)) {
        return true;
    }
    match hit {
        Hit::App(id) => apps().iter().find(|a| a.id == *id).is_some_and(|a| starts_a_word(&fuzzy::fold(super::apps::search_aliases(a.icon)), q)),
        Hit::Settings(page) => fixed().aliases.iter().any(|(h, k)| *h == Hit::Settings(*page) && strict_match(q, k)),
        _ => false,
    }
}

/// The groups in display order: the calculator answer, the Top Hit, the sections, the Documents, the files, and the
/// "Search in Apps" action last. A definition the query asked for ("define x") comes first, so Return opens it, and so
/// does the exact headword of a plain word when the Top Hit matched only loosely (`headword_beats_top`); any other
/// definition of a plain word follows the sections.
fn compose(query: &str, calc: Option<Group>, top: Option<Group>, sections: Vec<Group>, documents: Vec<Group>, files: Vec<Group>) -> Vec<Group> {
    let (asked, plain) = match dictionary_group(query) {
        Some((g, true)) => (Some(g), None),
        Some((g, false)) if headword_beats_top(query, &g, top.as_ref()) => (Some(g), None),
        Some((g, false)) => (None, Some(g)),
        None => (None, None),
    };
    let mut groups: Vec<Group> = asked.into_iter().chain(calc).chain(top).collect();
    groups.extend(sections);
    groups.extend(plain);
    groups.extend(documents);
    groups.extend(files);
    groups.push(Group { title: "Search in Apps", hits: vec![Hit::Terminal(query.to_string())] });
    scoped(groups, &super::search_scope::current())
}

/// The category a hit is listed under in Settings > Search, and the path it names when it is a file. `None` for
/// the hits that no switch controls (the Search in Apps action, the Top Hit's own kind, the calculator answer).
fn hit_category(hit: &Hit) -> (Option<super::search_scope::Category>, Option<&Path>) {
    use super::search_scope::Category as C;
    match hit {
        Hit::App(_) | Hit::Shortcut { .. } => (Some(C::Applications), None),
        Hit::Settings(_) | Hit::SettingsIntent { .. } | Hit::SettingsAction { .. } | Hit::MediaTransport { .. } => (Some(C::Settings), None),
        Hit::Contact { .. } => (Some(C::Contacts), None),
        Hit::Event { .. } => (Some(C::CalendarEvents), None),
        Hit::File { path, .. } | Hit::Folder { path, .. } | Hit::Reveal { folder: path, .. } => (Some(C::Files), Some(path.as_path())),
        Hit::Indexed { .. } => (Some(C::Documents), None),
        Hit::City(_) => (Some(C::Cities), None),
        Hit::Answer(_) => (Some(C::Calculator), None),
        // no Settings > Search switch covers definitions (gap: the page lists no Dictionary category yet)
        Hit::Deildu(_) | Hit::Terminal(_) | Hit::Definition { .. } => (None, None),
    }
}

/// Drops the hits Settings > Search hides, and any group left empty. A group with no hits stays only when it was
/// empty to begin with (the "Search in Apps" action is never hidden).
fn scoped(groups: Vec<Group>, scope: &super::search_scope::Scope) -> Vec<Group> {
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
/// both kept by the caller between keystrokes; `wallpapers` is the catalogue as (id, name). The sources are merged at
/// once here; the Search panel streams them.
fn results_folded(query: &str, deildu: &[String], deildu_folded: &[String], contacts: &[super::contactstore::Contact], contacts_folded: &[[String; 3]], wallpapers: &[(String, String)]) -> Vec<Group> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    let corpus = Corpus { deildu, deildu_folded, contacts, contacts_folded, files: &[], files_folded: &[], events: &[], wallpapers };
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
            wallpapers: &self.wallpapers,
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
        let merged = run.merge(PER_GROUP, &self.opens, now_secs(), &search_rank::NoSemantic);
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
            self.forgotten.clear();
            return vec![];
        }
        if query != self.run_query {
            self.start_run(&query);
        }
        let (top, sections, files) = self.run_groups();
        let (documents, files, gone) = fold_indexed_files(indexed_groups(&query, &self.opens), files, top.as_ref(), indexed_file_on_disk);
        // a query is merged again for every answer that streams in: each stale id is handed over once
        let gone: Vec<String> = gone.into_iter().filter(|id| !self.forgotten.contains(id)).collect();
        self.forgotten.extend(gone.iter().cloned());
        forget_gone_files(gone);
        let groups = compose(&query, calculator_group(&query), top, sections, documents, files);
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
/// Matches the ranker does not score keep their index order after the ranked ones. `usage` holds the panel's open counts.
/// The Files app's section keeps every match: `fold_indexed_files` looks at each (its documents are files, some of
/// them gone since) and cuts what is left to `PER_GROUP`. Cut here, three leftovers of deleted files hid the documents
/// behind them.
fn indexed_groups(query: &str, usage: &search_rank::Usage) -> Vec<Group> {
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
    // free document text is where a local semantic score helps; app and settings titles are too short for it
    let order = search_rank::order(query, &items, usage, now, &bifrost_ui::semantic_hash::HashedSemantic::new());
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
            snippet: f.snippet,
        };
        let whole = matches!(&hit, Hit::Indexed { app_id, .. } if app_id == super::workspace::FILES_APP);
        match groups.iter_mut().find(|g| g.title == title) {
            Some(g) => {
                if whole || g.hits.len() < PER_GROUP {
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
    /// the document icons of the file rows, by (`file_icon_key` of the name, icon size in pixels): the icon of the
    /// file's type as the open panel and the Dock's stack draw it. The shell draws them (`Shell::quicksearch_file_icons`)
    /// before the card is drawn; a row whose icon is not here yet shows the Files icon
    file_icons: Vec<(String, usize, Image)>,
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
    deildu_titles: Vec<String>,
    /// `fuzzy::fold` of each Deildu title, kept so keystrokes do not re-fold them
    deildu_folded: Vec<String>,
    contacts: Vec<super::contactstore::Contact>,
    /// `folded_fields` of each contact
    contacts_folded: Vec<[String; 3]>,
    /// files and folders under the Files root and the home folder, newest first, and their folded names
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
    /// how often each result was opened from this panel (`note_open`): the ranker's local usage signal. Owned by the
    /// panel, so two panels in one process (the tests' shells) never rank by each other's opens
    opens: search_rank::Usage,
    /// where the open counts are saved (beside quicksearch.ini); None until `load`
    usage_path: Option<PathBuf>,
    /// the wallpaper catalogue as (id, name), for the "Set Wallpaper to" rows (`set_wallpapers`)
    wallpapers: Vec<(String, String)>,
    /// a home folder to walk besides the Files root in place of the account's, which `refresh_sources` looks up at
    /// every scan (`files_walk::search_home`); None outside tests
    home: Option<PathBuf>,
    /// the shortcut runs this panel started (`ShortcutRuns`)
    shortcut_runs: ShortcutRuns,
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
    /// the ids of the Files app's indexed documents whose file was found gone and handed to `forget_gone_files`, so
    /// the merges that follow do not hand them over again; emptied with the query
    forgotten: Vec<String>,
    /// the calendar events, read on a worker (see `refresh_sources`)
    events: Vec<super::calendar::store::CalendarEvent>,
    calendar_stamp: Option<(PathBuf, std::time::SystemTime)>,
    /// the wall clock (Unix seconds, zone offset) the file rows' dates are written against, set when Search opens;
    /// (0, 0) until then, when the system clock is read
    wall: (i64, i32),
    /// 24-Hour Time of Settings > Date & Time, for the file rows' dates (set when the panel opens)
    clock_24h: bool,
    /// set by Space on a definition row: the next act shows the whole definition instead of opening Dictionary
    space_preview: bool,
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
        // Golden Gate: the empty field shows only its "Search" placeholder; the caret appears with the first character
        self.field = if mac { TextField::new(ui, "Search", MAC_LARGE_TITLE_PT * k, false, QUERY_MAX).without_empty_caret() } else { TextField::new(ui, "Search", FIELD_TEXT_PT, false, QUERY_MAX) };
        self.search_imgs.clear();
    }

    /// How much the Golden Gate layout is enlarged: at most `MAC_K_MAX`, which is 1, so the card, the field, the rows and
    /// the text keep their point sizes on a screen of any width.
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
            thumbs: vec![],
            file_thumbs: super::search_thumbs::FileThumbs::default(),
            file_icons: vec![],
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
            opens: Default::default(),
            usage_path: None,
            wallpapers: Vec::new(),
            home: None,
            shortcut_runs: Default::default(),
            launches: Default::default(),
            launches_path: None,
            run: None,
            run_query: String::new(),
            run_hits: BTreeMap::new(),
            frozen_top: None,
            shown: vec![],
            forgotten: vec![],
            events: vec![],
            calendar_stamp: None,
            wall: (0, 0),
            clock_24h: true,
            space_preview: false,
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
        self.opens = std::fs::read_to_string(&usage_path).map(|t| search_rank::Usage::from_text(&t)).unwrap_or_default();
        self.usage_path = Some(usage_path);
        let launches_path = path.with_file_name(LAUNCHES_FILE);
        self.launches = std::fs::read_to_string(&launches_path).map(|t| bifrost_ui::app_prediction::LaunchLedger::from_text(&t)).unwrap_or_default();
        self.launches_path = Some(launches_path);
        self.prefs = Prefs::load(path);
        self.recents = self.prefs.get("recents").unwrap_or("").split(',').filter_map(app_named).take(RECENTS).collect();
        self.searches = self.prefs.get("searches").unwrap_or("").split(',').map(str::trim).filter(|q| !q.is_empty()).map(String::from).take(SEARCHES).collect();
        self.search_imgs.clear();
    }

    /// The wallpaper catalogue as (id, name) Search offers "Set Wallpaper to" rows for. The shell sets it whenever its
    /// wallpaper list changes (`Shell::set_wallpapers`).
    pub fn set_wallpapers(&mut self, list: Vec<(String, String)>) {
        self.wallpapers = list;
    }

    /// What `query` finds at once outside the panel (the Look Up popover's Search section): the fixed sources and
    /// this panel's wallpaper catalogue, merged as `results` does.
    pub fn results_for(&self, query: &str) -> Vec<Group> {
        results_folded(query, &[], &[], &[], &[], &self.wallpapers)
    }

    /// True while a shortcut run from this panel is out or its result is not shown yet: the shell polls for it.
    pub(super) fn shortcut_runs_pending(&self) -> bool {
        let g = shortcut_runs(&self.shortcut_runs);
        g.0 > 0 || !g.1.is_empty()
    }

    /// Counts an open of an indexed result (`indexed_key`) and saves the counts beside quicksearch.ini. Local only.
    pub fn note_open(&mut self, key: &str) {
        self.opens.bump(key);
        if let Some(path) = &self.usage_path {
            if let Err(e) = std::fs::write(path, self.opens.to_text()) {
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
            let home = self.home.clone();
            crate::bare::sys::spawn_named("quicksearch-files", move || {
                // Privacy: files in the folders Settings > Search excludes are never listed
                let excluded = super::search_prefs::current();
                // the Files root and the account's home folder, in a bounded walk (`quicksearch_files`). Tests never
                // walk the home folder of the machine they run on (`search_home` gives none there): its files would
                // come in as results whenever the walk happened to finish inside a test
                let home = home.or_else(files_walk::search_home);
                let roots = files_walk::roots(&root, home.as_deref());
                let files: Vec<crate::files::fs::Entry> = files_walk::walk(&roots, files_walk::LIMIT, files_walk::VISIT_BUDGET).into_iter().filter(|e| !excluded.folder_excluded(&e.path)).collect();
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
        // the document icons carry the appearance's colours: drawn again for each showing
        self.file_icons.clear();
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
        // the dictionaries finished loading: the query may have a definition now
        let defined = dict::take_fresh();
        if (files || sources || answered || defined) && self.active() && !self.field.text().is_empty() {
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
            // Space on an arrow-selected file previews it in Files instead of typing a space; on a definition it shows
            // the whole definition (Look Up)
            Out::Text(ref text) if text == " " => match self.selected_hit() {
                Some(hit @ Hit::File { .. }) => return Some(hit),
                Some(hit @ Hit::Definition { .. }) => {
                    self.space_preview = true;
                    return Some(hit);
                }
                _ => {}
            },
            _ => {}
        }
        if self.field.edit(ui, &out, t) {
            return self.selected_hit().or_else(|| self.top_hit());
        }
        self.search(ui, h);
        None
    }

    /// The file rows whose document icon is not drawn yet, as (file name, icon size in pixels), one per kind of file
    /// (`file_icon_key`); a folder row asks for the folder icon as `FOLDER_ICON`.
    pub(super) fn file_icons_wanted(&self, ui: &Ui) -> Vec<(String, usize)> {
        let px = self.px(ui, self.m().row_icon).round() as usize;
        let mut out: Vec<(String, usize)> = vec![];
        for (hit, _) in self.sections.iter().flat_map(|s| &s.rows) {
            let Some(name) = icon_name(hit) else { continue };
            let key = file_icon_key(name);
            if px > 0 && self.file_icon(name, px).is_none() && !out.iter().any(|(n, _)| file_icon_key(n) == key) {
                out.push((name.to_string(), px));
            }
        }
        out
    }

    /// Keeps the document icon `icon` (a square of `px` pixels) for files named like `name`.
    pub(super) fn set_file_icon(&mut self, name: &str, px: usize, icon: Image) {
        // another scale leaves icons of a size no row asks for; a long session many kinds
        self.file_icons.retain(|(_, s, _)| *s == px);
        if self.file_icons.len() >= 64 {
            self.file_icons.clear();
        }
        self.file_icons.push((file_icon_key(name), px, icon));
    }

    /// The document icon kept for files named like `name` at `px` pixels.
    fn file_icon(&self, name: &str, px: usize) -> Option<&Image> {
        let key = file_icon_key(name);
        self.file_icons.iter().find(|(k, s, _)| *k == key && *s == px).map(|(_, _, img)| img)
    }

    /// The row the arrow keys are on, if any.
    pub fn selected_hit(&self) -> Option<Hit> {
        let (si, ri) = self.selected?;
        self.sections.get(si).and_then(|s| s.rows.get(ri)).map(|(h, _)| h.clone())
    }

    /// Cmd+Return: "Show in Folder" for the file or folder row the arrow keys are on, or else the first such row. None
    /// when no file is listed.
    pub fn reveal_hit(&self) -> Option<Hit> {
        let file = |h: &Hit| match h {
            Hit::File { path, name, .. } | Hit::Folder { path, name, .. } => Some(reveal_of(path, name)),
            Hit::Reveal { .. } => Some(h.clone()),
            _ => None,
        };
        self.selected_hit().as_ref().and_then(file).or_else(|| self.rows().iter().find_map(|(_, _, h)| file(h)))
    }

    /// The wall clock the rows' dates are written against.
    fn wall_now(&self) -> (i64, i32) {
        if self.wall.0 > 0 {
            self.wall
        } else {
            (now_secs() as i64, 0)
        }
    }

    /// Moves the keyboard selection one row down (`step` 1) or up (-1) through the shown rows.
    fn move_selection(&mut self, ui: &Ui, step: isize) {
        let rows: Vec<(usize, usize)> = self.rows().into_iter().map(|(si, ri, _)| (si, ri)).collect();
        // Golden Gate: the Top Hit is highlighted before any arrow key, so Down leaves it for the second row
        let cur = self.selected.or_else(|| rows.first().copied().filter(|_| self.mac));
        self.selected = step_row(&rows, cur, step);
        self.reveal_selected(ui);
    }

    /// Cmd+Down / Cmd+Up: jumps to the first row of the next (`step` 1) or previous (-1) section, clamped to the ends.
    /// True when Search is showing rows to jump through.
    pub fn jump_section(&mut self, ui: &Ui, step: isize) -> bool {
        let rows = self.rows();
        let Some(&(first, _, _)) = rows.first() else { return false };
        let cur = self.selected.map_or(first, |(si, _)| si);
        let last = rows.last().map_or(first, |r| r.0);
        let to = if step > 0 { rows.iter().map(|r| r.0).find(|&si| si > cur).unwrap_or(last) } else { rows.iter().map(|r| r.0).rev().find(|&si| si < cur).unwrap_or(first) };
        self.selected = rows.iter().find(|r| r.0 == to).map(|r| (r.0, r.1));
        self.reveal_selected(ui);
        true
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
                groups = filter_to_chip(groups, c);
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
                    let (now, offset) = self.wall_now();
                    let (title, sub) = row_text(&hit, name_of, (now, offset, self.clock_24h));
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
                    Hit::File { name, .. } | Hit::Folder { name, .. } => name.clone(),
                    Hit::Reveal { name, .. } => format!("Show {name} in Folder"),
                    Hit::Shortcut { title, app_name, .. } => format!("{title} ({app_name})"),
                    Hit::SettingsIntent { title, .. } => title.to_string(),
                    Hit::SettingsAction { title, .. } | Hit::MediaTransport { title, .. } => title.clone(),
                    Hit::Indexed { title, snippet, .. } if !snippet.is_empty() => indexed_label(title, snippet),
                    Hit::Indexed { title, kind, .. } => indexed_label(title, kind),
                    Hit::Definition { word, preview, .. } if !preview.is_empty() => format!("{word}: {preview}"),
                    Hit::Definition { word, .. } => format!("Look up \u{201c}{word}\u{201d}"),
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
        // a new query puts the highlight back on the Top Hit; a rebuild for the same query (late rows, another Dock) keeps
        // the row the arrow keys are on
        self.selected = keep.filter(|_| self.scroll_query == query).and_then(|wanted| self.sections.iter().enumerate().find_map(|(si, s)| s.rows.iter().position(|(r, _)| *r == wanted).map(|ri| (si, ri))));
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
                    // the Top Hit's plate gives way to the row the arrow keys are on
                    let top = self.mac && si == 0 && ri == 0 && self.selected.map_or(true, |at| at == (0, 0));
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
                        Hit::File { .. } | Hit::Folder { .. } | Hit::Reveal { .. } => Some(AppId::Files),
                        Hit::Shortcut { app_id, .. } | Hit::Indexed { app_id, .. } => super::apphost::slot_of(app_id).map(|n| AppId::External(n as u8)),
                        Hit::Event { .. } => super::apphost::slot_of(super::services::calendar::STORE_APP).map(|n| AppId::External(n as u8)),
                        Hit::Definition { .. } => super::apphost::slot_of(bifrost_ui::services_menu::DICTIONARY_APP).map(|n| AppId::External(n as u8)),
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
                    // a file shows the document icon of its type, as Files and the Dock's stack do, not the Files app's
                    let doc_icon = icon_name(hit).and_then(|name| self.file_icon(name, isz.round() as usize));
                    let icon = doc_icon.or(icon);
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
        self.swallow_wheel_drag(t);
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
        // the dictionaries load once, off the UI thread; the file rows' dates use the shell's wall clock
        dict::preload(super::apphost::root().join("appdata").join(bifrost_ui::services_menu::DICTIONARY_APP));
        self.quicksearch.wall = self.wall_now(t);
        self.quicksearch.clock_24h = self.clock_style.use_24h;
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
        if !matches!(hit, Hit::Definition { .. }) {
            self.quicksearch.space_preview = false;
        }
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
            Hit::File { path, name, modified } => self.quicksearch_open_file(&path, &name, modified, t),
            Hit::Folder { path, .. } => {
                // the folder opens in Files: the Files app on the desktop, the built-in view elsewhere
                let shown = if self.desk_active() {
                    self.desktop_open_folder(super::workspace::FILES_APP, &path.to_string_lossy(), t)
                } else {
                    self.open(AppId::Files, t);
                    let shown = match &mut self.files {
                        Some(f) => f.show_folder(&self.ui, &path, t),
                        None => false,
                    };
                    if shown { Ok(()) } else { Err("No location in Files holds it".to_string()) }
                };
                if let Err(e) = shown {
                    self.file_action_failed(&path, "could not be opened in Files", &e);
                }
            }
            Hit::Definition { word, .. } => {
                if std::mem::take(&mut self.quicksearch.space_preview) {
                    // Space: the whole definition in the Look Up popover
                    self.open_look_up(word, None);
                } else {
                    self.run_intent(bifrost_ui::services_menu::DICTIONARY_APP, "look-up", "word", &word, t);
                }
            }
            Hit::Reveal { folder, name } => {
                // on the desktop the Files app shows the folder with the file selected, as the Services menu and the
                // Downloads stack do, and a failure is said (the built-in view is not what the desktop shows: opening
                // it there left the person with no window and no word, E2E5 2); elsewhere the built-in view opens at
                // the folder
                let target = reveal_target(&folder, &name);
                if self.desk_active() {
                    // the item selected in the folder that holds it, also when the item is a folder itself (which
                    // `desktop_show_in_files` would open instead); the folder alone when the item is gone
                    let files_app = super::workspace::FILES_APP;
                    let shown = if target == folder { self.desktop_open_folder(files_app, &folder.to_string_lossy(), t) } else { self.desktop_reveal_file(files_app, &target.to_string_lossy(), t) };
                    if let Err(e) = shown {
                        self.file_action_failed(&target, "could not be shown in Files", &e);
                    }
                } else {
                    self.open(AppId::Files, t);
                    let shown = match &mut self.files {
                        Some(f) => f.show_folder(&self.ui, &folder, t),
                        None => false,
                    };
                    if !shown {
                        self.file_action_failed(&target, "could not be shown in Files", "No location in Files holds its folder");
                    }
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
            Hit::Shortcut { app_id, intent, title, .. } => {
                if let Some(n) = super::apphost::slot_of(&app_id) {
                    self.open(AppId::External(n as u8), t);
                    // the app may still be starting: retry the connection for a few seconds, off the UI thread; the
                    // result comes back as a banner with the shortcut's name (E2E12 5)
                    // counted while it is out, also when the intent call panics (`ShortcutRunGuard`)
                    let runs = Arc::clone(&self.quicksearch.shortcut_runs);
                    let done = Arc::clone(&runs);
                    spawn_shortcut_run(&runs, move || {
                        let outcome = super::intents::run(&app_id, &intent, std::time::Duration::from_secs(8));
                        match &outcome {
                            Ok(_) => bifrost_core::log::shell_logger("quicksearch").info(format!("shortcut {intent} of {app_id} ran")),
                            Err(e) => bifrost_core::log::shell_logger("quicksearch").error(format!("shortcut {intent} of {app_id}: {e}")),
                        }
                        shortcut_runs(&done).1.push(ShortcutRun { slot: n as u8, title, outcome });
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

    /// Return on a file row. On the desktop the file opens the way the Dock's Downloads stack opens one: with the app
    /// its type opens with after the malware check, else shown selected in Files (`stack_open_item`); a failure of
    /// that last step is said (`file_action_failed`). Elsewhere the built-in Files view previews it.
    fn quicksearch_open_file(&mut self, path: &Path, name: &str, modified: i64, t: f64) {
        #[cfg(feature = "desktop-wayland")]
        if self.desk_active() {
            let item = super::dock_stack::Item { name: name.to_string(), path: path.to_path_buf(), dir: false };
            return self.stack_open_item(&item, t);
        }
        self.open(AppId::Files, t);
        // a row that came from the index (`fold_indexed_files`) is not in the walk's list
        let entry = self.quicksearch.files.iter().find(|e| e.path == path).cloned().unwrap_or_else(|| crate::files::fs::Entry {
            kind: crate::files::fs::kind_of(name, false),
            name: name.to_string(),
            path: path.to_path_buf(),
            is_dir: false,
            size: std::fs::metadata(path).map_or(0, |m| m.len()),
            mtime: modified,
            items: None,
        });
        self.preview_in_files(&entry, t);
    }

    /// Says that something asked of the file (or folder) at `path` did not happen: a banner "<name> <what>" with the
    /// reason under it, and a line in the log. Search and the Downloads stack close before they act, so without this a
    /// failure looked like a key that did nothing.
    pub(super) fn file_action_failed(&mut self, path: &Path, what: &str, why: &str) {
        let (title, body) = file_problem_banner(path, what, why);
        bifrost_core::log::shell_logger("quicksearch").error(format!("{}: {what}: {why}", path.display()));
        let time = self.notify.now();
        self.notify(super::notify::Notification::new(AppId::Files, title, body, time));
        self.dirty = true;
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

    /// Cmd+Down / Cmd+Up in Search: jumps between the result categories. True when Search took the chord.
    pub(super) fn quicksearch_jump(&mut self, step: isize) -> bool {
        if !self.quicksearch.is_shown() {
            return false;
        }
        self.quicksearch.jump_section(&self.ui, step);
        self.dirty = true;
        true
    }

    /// Cmd+Return in Search: shows the selected (or first) file in its folder in Files, as "Show in Folder" does. True
    /// when Search was open and took the chord (it does even with no file listed, so the app behind never gets it).
    pub(super) fn quicksearch_reveal(&mut self, t: f64) -> bool {
        if !self.quicksearch.is_shown() {
            return false;
        }
        match self.quicksearch.reveal_hit() {
            Some(hit) => {
                let _ = self.quicksearch_act(hit, t);
            }
            // nothing to show in a folder: the panel stays as it is, and the log says why the chord did nothing
            None => bifrost_core::log::shell_logger("quicksearch").info("Cmd+Return: no file among the results to show in its folder"),
        }
        self.dirty = true;
        true
    }

    /// Draws the document icons the card's file rows still lack, with the icon the open panel and the Dock's stack use
    /// for a file (`draw_file_icon`), each into a transparent square the card then blits. Called before the card is drawn.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn quicksearch_file_icons(&mut self) {
        for (name, px) in self.quicksearch.file_icons_wanted(&self.ui) {
            let mut img = Image::new(px, px);
            {
                let mut sub = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
                let entry = super::docpanel::Entry { name: name.clone(), dir: name == FOLDER_ICON, enabled: true, path: None };
                self.draw_file_icon(&mut sub, &entry, 0.0, 0.0, px as f32, 0);
            }
            self.quicksearch.set_file_icon(&name, px, img);
        }
    }

    /// Advances the card's settle and keyboard; true when a frame is needed.
    pub(super) fn quicksearch_tick(&mut self, t: f64) -> bool {
        // the Dock may show or hide under the open card: its top moves the card's limit
        let moved = self.quicksearch.active() && {
            let dock_top = self.quicksearch_dock_top();
            self.quicksearch.set_dock_top(&self.ui, self.h, dock_top)
        };
        let posted = self.shortcut_results_tick();
        self.quicksearch.tick(&self.ui, self.h, t) || moved || posted
    }

    /// Posts the banner of every shortcut run from Search that finished since the last tick, through the ordinary
    /// notification path (the app's notification settings apply). True when one was posted.
    pub(super) fn shortcut_results_tick(&mut self) -> bool {
        let done = std::mem::take(&mut shortcut_runs(&self.quicksearch.shortcut_runs).1);
        self.post_shortcut_runs(done)
    }

    fn post_shortcut_runs(&mut self, done: Vec<ShortcutRun>) -> bool {
        let posted = !done.is_empty();
        for run in done {
            let (title, body) = shortcut_banner(&run.title, &run.outcome);
            let time = self.notify.now();
            self.notify(super::notify::Notification::new(AppId::External(run.slot), title, body, time));
        }
        posted
    }

    /// The home screen with the card and keyboard over it; the keyboard
    /// rises over bare wallpaper, the dock hidden under it as on the bifrost.
    pub(super) fn draw_quicksearch(&mut self, s: &mut Surface, t: f64) {
        self.draw_home(s);
        if let Some(edge) = self.quicksearch.keyboard_edge(&self.ui, self.h, t) {
            let wp = self.home_still();
            s.band(edge.max(0.0) as usize).copy(wp, (self.w - wp.w as i32) / 2, (self.h - wp.h as i32) / 2);
        }
        #[cfg(feature = "desktop-wayland")]
        self.quicksearch_file_icons();
        self.quicksearch.draw(s, &self.ui, self.w, self.h, t);
    }
}

#[cfg(test)]
mod tests;

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
mod desktop_tests;

#[cfg(test)]
mod file_actions_tests;

#[cfg(test)]
mod dock_and_scroll_tests;
