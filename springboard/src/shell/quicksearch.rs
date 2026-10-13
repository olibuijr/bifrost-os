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
use super::ui::{home_indicator, home_zone, Ui, Weight, CELL, LABEL, LABEL_2, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE2_PT, MENU_SOLID, MENU_TINT, SECONDARY, SIDEBAR_PLATE, WHITE};
use super::{fuzzy, glyphs, settings, AppId, Effect, Event, Screen, Shell, apps};
use crate::gfx::{rgba, scale, Image, Surface};
use bifrost_ui::uttype;
use std::path::{Path, PathBuf};

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
};

/// Golden Gate Search on the desktop, measured from the Golden Gate captures (27-Golden-Gate-Search-Search-v2):
/// a 520 pt wide, 84 pt tall glass capsule 139 pt from the top with the query in the large-title size, and the
/// results in a separate glass card 12 pt below it (radius 28) with 36 pt icons on 48 pt rows, the top hit on an
/// inset systemBlue plate (Return opens it). With nothing typed only the capsule shows.
const MAC: Metrics = Metrics {
    card_w: 520.0,
    card_top: 139.0,
    radius: 28.0,
    field: 84.0,
    text_x: 38.0,
    glass_x: 0.0,
    clear_x: 30.0,
    mic: false,
    header: 28.0,
    header_mid: 16.0,
    inset: 26.0,
    row: 48.0,
    row_icon_x: 20.0,
    row_icon: 36.0,
    row_text_x: 66.0,
    bottom_pad: 12.0,
    label_pt: MAC_TITLE2_PT,
    suggestions: false,
};
/// Golden Gate: the gap between the search capsule and the results card
const MAC_GAP_PT: f32 = 12.0;
/// Golden Gate: the top-hit plate's inset from the card edge and its corner radius
const MAC_PLATE_INSET_PT: f32 = 10.0;
const MAC_PLATE_R_PT: f32 = 12.0;

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
    /// an item an app put into the Core Search index: opens the app and sends it a continue-activity message.
    /// `kind` is the file type's name ("PDF document", "" when unknown); `thumb` the thumbnail path in the app's data dir.
    Indexed { app_id: String, unique_id: String, domain: String, title: String, kind: String, thumb: String },
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

/// The runnable `[[intent]]` shortcuts of the installed apps, with their folded search text.
fn shortcut_items() -> Vec<(Hit, String)> {
    let mut v = vec![];
    for app in super::apphost::installed().iter() {
        for i in app.manifest.intents.iter().filter(|i| i.runnable()) {
            let hit = Hit::Shortcut { app_id: app.manifest.id.clone(), app_name: app.manifest.name.clone(), intent: i.id.clone(), title: i.title.clone() };
            v.push((hit, fuzzy::fold(&format!("{} {}", i.title, app.manifest.name))));
        }
    }
    v
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
    results_folded(query, deildu, &folded, contacts)
}

/// Candidates of the sources that never change while the shell runs, folded once.
struct Fixed {
    settings: Vec<(Hit, String)>,
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

/// `deildu_folded` holds `fuzzy::fold` of each `deildu` title, kept by the caller between keystrokes.
fn results_folded(query: &str, deildu: &[String], deildu_folded: &[String], contacts: &[super::contactstore::Contact]) -> Vec<Group> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    let m = fuzzy::Matcher::new(query);
    let fx = fixed();
    let rank_all = |items: &[(Hit, String)]| fuzzy::sorted_folded(&m, items.iter().map(|(h, f)| (h.clone(), f.as_str())));
    let app_items: Vec<(Hit, String)> = apps().iter().map(|a| (Hit::App(a.id), fuzzy::fold(a.name))).collect();
    let deildu_items: Vec<(Hit, &str)> = deildu.iter().zip(deildu_folded).map(|(t, f)| (Hit::Deildu(t.clone()), f.as_str())).collect();
    let titles = ["Applications", "Settings", "Clock", "Deildu"];
    let mut ranked: Vec<Vec<(fuzzy::Rank, Hit)>> =
        vec![rank_all(&app_items), rank_all(&fx.settings), rank_all(&fx.cities), fuzzy::sorted_folded(&m, deildu_items)];
    // keyword aliases ("brightness", "dark mode") lead to their Settings page
    let mut seen: Vec<Hit> = vec![];
    ranked[1].retain(|(_, h)| !seen.contains(h) && { seen.push(h.clone()); true });
    for (rank, hit) in rank_all(&fx.aliases) {
        if !seen.contains(&hit) {
            seen.push(hit.clone());
            ranked[1].push((rank, hit));
        }
    }
    let mut people: Vec<_> = contacts.iter().filter_map(|contact| {
        [&contact.name, &contact.phone, &contact.email].into_iter()
            .filter_map(|field| m.rank(field)).min()
            .map(|rank| (rank, Hit::Contact { id: contact.id, name: contact.display_name().into() }))
    }).collect();
    people.sort_by_key(|(rank, _)| *rank);
    ranked.push(people);
    ranked.push(rank_all(&shortcut_items()));
    let mut groups = vec![];
    if let Some(a) = quicksearch_math::answer(query) {
        groups.push(Group { title: "Calculator", hits: vec![Hit::Answer(a)] });
    }
    if let Some(i) = ranked.iter().enumerate().filter_map(|(i, v)| v.first().map(|(r, _)| (r.kind, i, r.len))).min().map(|(_, i, _)| i) {
        let (_, hit) = ranked[i].remove(0);
        groups.push(Group { title: "Top Hit", hits: vec![hit] });
    }
    for (title, v) in titles.into_iter().chain(["Contacts", "Shortcuts"]).zip(ranked) {
        if !v.is_empty() {
            groups.push(Group { title, hits: v.into_iter().take(PER_GROUP).map(|(_, h)| h).collect() });
        }
    }
    groups.push(Group { title: "Search in Apps", hits: vec![Hit::Terminal(query.to_string())] });
    groups
}

/// "Documents" section: items apps indexed with `CSSearchableIndex`, up to `PER_GROUP` rows.
fn indexed_groups(query: &str) -> Vec<Group> {
    let hits: Vec<Hit> = super::spotindex::search(query, PER_GROUP)
        .into_iter()
        .map(|f| Hit::Indexed {
            kind: f.kind.map_or(String::new(), |k| uttype::description(k).to_string()),
            thumb: f.thumbnail,
            app_id: f.app_id,
            unique_id: f.unique_id,
            domain: f.domain,
            title: f.title,
        })
        .collect();
    if hits.is_empty() {
        vec![]
    } else {
        vec![Group { title: "Documents", hits }]
    }
}

/// "Files" section (names containing the query, newest first), up to three rows, then a
/// "Show in Folder" row for the top match. `folded` holds `fuzzy::fold` of each file's name,
/// so "bjork" finds "Björk.mp3".
fn file_note_groups(query: &str, files: &[crate::files::fs::Entry], folded: &[String]) -> Vec<Group> {
    let q = fuzzy::fold(query.trim());
    if q.is_empty() {
        return vec![];
    }
    let matches: Vec<&crate::files::fs::Entry> = files.iter().zip(folded).filter(|(_, f)| f.contains(&q)).map(|(e, _)| e).take(PER_GROUP).collect();
    let Some(top) = matches.first() else { return vec![] };
    let hits = matches.iter().map(|e| Hit::File { path: e.path.clone(), name: e.name.clone() }).collect();
    let folder = top.path.parent().map_or_else(|| top.path.clone(), Path::to_path_buf);
    vec![Group { title: "Files", hits }, Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder, name: top.name.clone() }] }]
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
    recents: Vec<AppId>,
    /// recent queries, newest first, and their rendered rows
    searches: Vec<String>,
    search_imgs: Vec<Image>,
    /// decoded thumbnails of indexed results: (app/path key, icon size in pixels, image; None when unreadable)
    thumbs: Vec<(String, usize, Option<Image>)>,
    searches_header: Image,
    /// (app, 60 pt icon, 29 pt icon, label)
    icons: Vec<(AppId, Image, Image, Image)>,
    sections: Vec<Section>,
    /// the row the arrow keys moved to: (section, row); Return opens it instead of the Top Hit
    selected: Option<(usize, usize)>,
    deildu_titles: Vec<String>,
    /// `fuzzy::fold` of each Deildu title, kept so keystrokes do not re-fold them
    deildu_folded: Vec<String>,
    contacts: Vec<super::contactstore::Contact>,
    /// files under the Files root, newest first, and their folded names
    files: Vec<crate::files::fs::Entry>,
    files_folded: Vec<String>,
    /// the background file scan started at open, until its result is taken
    files_rx: Option<std::sync::mpsc::Receiver<Vec<crate::files::fs::Entry>>>,
    /// modification times the contacts and Deildu title caches were read at
    contacts_stamp: Option<(PathBuf, std::time::SystemTime)>,
    deildu_stamp: Option<(PathBuf, std::time::SystemTime)>,
    suggestions: Image,
    chevron: Image,
    icon_colour: Vec<(&'static str, u32)>,
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
        if self.mac == mac {
            return;
        }
        self.mac = mac;
        self.field = if mac { TextField::new(ui, "Search Search", MAC_LARGE_TITLE_PT, false, QUERY_MAX) } else { TextField::new(ui, "Search", FIELD_TEXT_PT, false, QUERY_MAX) };
        self.search_imgs.clear();
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
            recents: vec![],
            searches: vec![],
            search_imgs: vec![],
            searches_header: ui.text("Recent Searches", 20.0, Weight::Semibold, WHITE),
            icons: vec![],
            sections: vec![],
            selected: None,
            thumbs: vec![],
            deildu_titles: vec![],
            deildu_folded: vec![],
            contacts: vec![],
            files: vec![],
            files_folded: vec![],
            files_rx: None,
            contacts_stamp: None,
            deildu_stamp: None,
            suggestions: ui.text("Assistant Suggestions", 20.0, Weight::Semibold, WHITE),
            chevron: ui.text("›", 22.0, Weight::Semibold, SECONDARY),
            icon_colour: vec![
                ("Wi-Fi", rgba(0, 122, 255, 255)),
                ("Display & Brightness", rgba(10, 132, 255, 255)),
                ("Wallpaper", rgba(50, 173, 230, 255)),
                ("About", rgba(142, 142, 147, 255)),
                ("Battery", rgba(52, 199, 89, 255)),
            ],
        }
    }

    /// The screen turned: the keyboard takes the new width.
    pub fn resize(&mut self, w: f32) {
        self.keyboard.resize(w);
    }

    /// Reads the recents from `path` (an ini file) and saves there from now on.
    pub fn load(&mut self, path: impl Into<PathBuf>) {
        self.prefs = Prefs::load(path);
        self.recents = self.prefs.get("recents").unwrap_or("").split(',').filter_map(app_named).take(RECENTS).collect();
        self.searches = self.prefs.get("searches").unwrap_or("").split(',').map(str::trim).filter(|q| !q.is_empty()).map(String::from).take(SEARCHES).collect();
        self.search_imgs.clear();
    }

    /// Records a launch: `app` becomes the first suggestion.
    pub fn note_launch(&mut self, app: AppId) {
        self.recents.retain(|&a| a != app);
        self.recents.insert(0, app);
        self.recents.truncate(RECENTS);
        let text = self.recents.iter().map(|&a| app_key(a)).collect::<Vec<_>>().join(",");
        if let Err(e) = self.prefs.set("recents", &text) {
            eprintln!("springboard: quicksearch.ini: {e}");
        }
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
    }

    pub fn has_icons(&self) -> bool {
        !self.icons.is_empty()
    }

    /// The Deildu titles searched from now on.
    pub fn set_deildu_titles(&mut self, titles: Vec<String>) {
        self.deildu_folded = titles.iter().map(|t| fuzzy::fold(t)).collect();
        self.deildu_titles = titles;
    }

    /// Re-reads the contacts and cached Deildu titles only when their files changed, and
    /// scans `root` for files on a worker thread; the list fills in on a later tick.
    pub fn refresh_sources(&mut self, contacts: &Path, deildu_dir: &Path, root: PathBuf) {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let stamp = mtime(contacts).map(|t| (contacts.to_path_buf(), t));
        if stamp.is_none() || stamp != self.contacts_stamp {
            self.contacts = super::contactstore::load_checked(contacts).unwrap_or_default();
            self.contacts_stamp = stamp;
        }
        let cache = deildu_dir.join("cache");
        let stamp = mtime(&cache).map(|t| (cache.clone(), t));
        if stamp.is_none() || stamp != self.deildu_stamp {
            self.set_deildu_titles(cached_titles(deildu_dir));
            self.deildu_stamp = stamp;
        }
        if self.files_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            crate::bare::sys::spawn_named("quicksearch-files", move || {
                let _ = tx.send(crate::files::fs::recent_in(&[root], 2000));
            });
            self.files_rx = Some(rx);
        }
    }

    /// Takes a finished file scan; true when the list changed.
    fn take_files(&mut self) -> bool {
        let Some(rx) = &self.files_rx else { return false };
        match rx.try_recv() {
            Ok(files) => {
                self.set_files(files);
                self.files_rx = None;
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
        if self.take_files() && self.active() && !self.field.text().is_empty() {
            self.search(ui, h);
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
        if self.files_rx.is_some() {
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
                self.move_selection(1);
                return None;
            }
            Out::Named("Up") => {
                self.move_selection(-1);
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
    fn move_selection(&mut self, step: isize) {
        let rows: Vec<(usize, usize)> = self.rows().into_iter().map(|(si, ri, _)| (si, ri)).collect();
        self.selected = step_row(&rows, self.selected, step);
    }

    /// The first result: what Return opens.
    pub fn top_hit(&self) -> Option<Hit> {
        self.sections.first().and_then(|s| s.rows.first()).map(|(h, _)| h.clone())
    }

    /// Rebuilds the result rows for the query, keeping what fits above the keyboard.
    fn search(&mut self, ui: &Ui, h: i32) {
        let m = self.m();
        let mut groups = results_folded(self.field.text(), &self.deildu_titles, &self.deildu_folded, &self.contacts);
        let mut extra = indexed_groups(self.field.text());
        extra.extend(file_note_groups(self.field.text(), &self.files, &self.files_folded));
        if !groups.is_empty() {
            let at = groups.len() - 1;
            groups.splice(at..at, extra);
        }
        let mut sections = vec![];
        let mut y = self.content_top(ui) + ui.pt(m.bottom_pad);
        let bottom = if self.mac { h as f32 - ui.pt(12.0) } else { self.keyboard_top(ui, h) - ui.pt(8.0) };
        // thumbnails are fitted to the row's icon square; a size change (another scale) reloads them
        let isz = ui.pt(m.row_icon).round() as usize;
        self.thumbs.retain(|(_, s, _)| *s == isz);
        if self.thumbs.len() > 64 {
            self.thumbs.clear();
        }
        'fill: for g in groups {
            let mut rows = vec![];
            for hit in g.hits {
                let need = ui.pt(m.row) + if rows.is_empty() { ui.pt(m.header) } else { 0.0 };
                if y + need > bottom {
                    break 'fill;
                }
                y += need;
                if let Hit::Indexed { app_id, thumb, .. } = &hit {
                    let key = format!("{app_id}/{thumb}");
                    if !thumb.is_empty() && !self.thumbs.iter().any(|(k, _, _)| *k == key) {
                        let img = super::spotindex::load_thumbnail(app_id, thumb, isz);
                        self.thumbs.push((key, isz, img));
                    }
                }
                let label = match &hit {
                    Hit::App(app) => apps().iter().find(|a| a.id == *app).map_or(String::new(), |a| a.name.to_string()),
                    Hit::Settings(name) | Hit::City(name) => name.to_string(),
                    Hit::Deildu(title) => title.clone(),
                    Hit::Contact { name, .. } => name.clone(),
                    Hit::Terminal(_) => "Search in Terminal".to_string(),
                    Hit::Answer(a) => format!("= {a}"),
                    Hit::File { name, .. } => name.clone(),
                    Hit::Reveal { name, .. } => format!("Show {name} in Folder"),
                    Hit::Shortcut { title, app_name, .. } => format!("{title} ({app_name})"),
                    Hit::Indexed { title, kind, .. } => indexed_label(title, kind),
                };
                let max_w = ui.pt(m.card_w - m.row_text_x - m.inset);
                rows.push((hit, ui.text(&ui.fit(&label, self.m().label_pt, max_w), self.m().label_pt, Weight::Regular, WHITE)));
            }
            if !rows.is_empty() {
                let header = if self.mac { ui.text(g.title, MAC_SUBHEAD_PT, Weight::Semibold, WHITE) } else { ui.text(g.title, 20.0, Weight::Semibold, WHITE) };
                sections.push(Section { header, rows });
            }
        }
        self.sections = sections;
        self.selected = None;
    }

    /// Rows as the list shows them: (section, row, label text).
    pub fn rows(&self) -> Vec<(usize, usize, Hit)> {
        self.sections.iter().enumerate().flat_map(|(si, s)| s.rows.iter().enumerate().map(move |(ri, (h, _))| (si, ri, h.clone()))).collect()
    }

    // -- geometry --

    fn card_x(&self, ui: &Ui, w: i32) -> f32 {
        let m = self.m();
        ((w as f32 - ui.pt(m.card_w)) / 2.0).round()
    }

    /// Below the field, where the suggestions or the results start.
    fn content_top(&self, ui: &Ui) -> f32 {
        let m = self.m();
        ui.pt(m.card_top + m.field + if self.mac { MAC_GAP_PT } else { 0.0 })
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
            let gap = if self.mac { ui.pt(MAC_GAP_PT + m.bottom_pad) } else { 0.0 };
            gap + self.sections.iter().map(|s| ui.pt(m.header) + ui.pt(m.row) * s.rows.len() as f32).sum::<f32>() + ui.pt(m.bottom_pad)
        }
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
        let fh = ui.pt(m.field).ceil() as usize;
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
        let gap = ui.pt(MAC_GAP_PT) as usize;
        if oh > fh + gap {
            put(fh + gap, oh - fh - gap, ui.pt(m.radius));
        }
        Some(((x, y, ow, oh), mask))
    }

    pub fn card(&self, ui: &Ui, w: i32) -> Rect {
        let m = self.m();
        (self.card_x(ui, w), ui.pt(m.card_top), ui.pt(m.card_w), ui.pt(m.field) + self.content_h(ui))
    }

    fn field_rect(&self, ui: &Ui, w: i32) -> Rect {
        let m = self.m();
        let (x, y, cw, _) = self.card(ui, w);
        (x, y, cw, ui.pt(m.field))
    }

    /// Centre of the clear button at the field's right end.
    fn clear_centre(&self, ui: &Ui, w: i32) -> (f32, f32) {
        let m = self.m();
        let (x, y, cw, fh) = self.field_rect(ui, w);
        (x + cw - ui.pt(m.clear_x), y + fh / 2.0)
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
        let mut y = self.content_top(ui);
        let mut out = vec![];
        for (si, s) in self.sections.iter().enumerate() {
            y += ui.pt(m.header);
            for ri in 0..s.rows.len() {
                out.push(((si, ri), (x, y, cw, ui.pt(m.row))));
                y += ui.pt(m.row);
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
                self.field.tap(ui, x - self.card_x(ui, w) - ui.pt(m.text_x), t);
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
        let fh = ui.pt(m.field);
        if self.mac {
            // Golden Gate: a glass capsule for the field and, when there are results, a glass card under it,
            // both through the presenter's glass hook as the menus are.
            mac_glass(s, cx, cy, cw, fh, fh / 2.0, alpha);
            if ch > fh {
                let ry = cy + fh + ui.pt(MAC_GAP_PT);
                mac_glass(s, cx, ry, cw, ch - fh - ui.pt(MAC_GAP_PT), ui.pt(m.radius), alpha);
            }
        } else {
            // Liquid Glass (regular): blur the wallpaper and dock beneath, tint it, and light the edge.
            s.glass_rrect(cx, cy, cw, ch, ui.pt(m.radius), a(GLASS_TINT), a(CARD_BG), ui.pt(GLASS_BLUR_PT) as usize);
            s.stroke_rrect(cx, cy, cw, ch, ui.pt(m.radius), ui.pt(1.0), a(GLASS_EDGE));
        }

        // the field: magnifier (bifrost), query or placeholder, clear button
        let mid = cy + fh / 2.0;
        let (gx, gr) = (cx + ui.pt(m.glass_x), ui.pt(GLASS_R_PT));
        let stroke = ui.pt(2.2);
        let glass = glyphs::either(glyphs::ring((gx, mid - ui.pt(1.5)), gr, stroke), glyphs::segment((gx + gr * 0.72, mid - ui.pt(1.5) + gr * 0.72), (gx + gr * 1.55, mid - ui.pt(1.5) + gr * 1.55), stroke * 1.2));
        if !self.mac {
            glyphs::paint(s, (gx - gr - stroke, mid - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), a(SECONDARY), glass);
        }
        let tx = cx + ui.pt(m.text_x);
        let text_w = cw - ui.pt(m.text_x + m.clear_x + CLEAR_R_PT + 8.0);
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
            let (ky, r) = (ky + slide, ui.pt(CLEAR_R_PT));
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
        let header_x = (cx + ui.pt(m.inset)) as i32;
        let mut y = cy + fh + if self.mac && ch > fh { ui.pt(MAC_GAP_PT + m.bottom_pad) } else { 0.0 };
        if self.suggestions_shown() {
            if self.search_imgs.len() != self.searches.len() {
                let max_w = ui.pt(m.card_w - m.row_text_x - m.inset);
                self.search_imgs = self.searches.iter().map(|q| ui.text(&ui.fit(q, 17.0, max_w), 17.0, Weight::Regular, WHITE)).collect();
            }
            if !self.recents.is_empty() {
            let hd = &self.suggestions;
            s.blit(hd, header_x, (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32, 255, alpha);
            let ch = &self.chevron;
            s.blit(ch, (cx + cw - ui.pt(m.inset + 4.0) - ch.w as f32) as i32, (y + ui.pt(m.header_mid) - ch.h as f32 / 2.0) as i32, 255, alpha);
            for (i, app) in self.recents.iter().enumerate() {
                let Some((_, big, _, label)) = self.icons.iter().find(|(a, ..)| a == app) else { continue };
                let (ix, iy, iw, _) = self.suggestion_rect(ui, w, i);
                let iy = iy + slide;
                let tint = if pressed == Some(Target::Suggestion(i)) { 150 } else { 255 };
                s.blit(big, ix as i32, iy as i32, tint, alpha);
                s.blit(label, (ix + (iw - label.w as f32) / 2.0) as i32, (iy + iw + ui.pt(SUGGEST_LABEL_GAP_PT)) as i32, 255, alpha);
            }
            y += ui.pt(m.header + SUGGEST_PT);
            }
            if !self.searches.is_empty() {
                let hd = &self.searches_header;
                s.blit(hd, header_x, (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32, 255, alpha);
                y += ui.pt(m.header);
                for (i, label) in self.search_imgs.iter().enumerate() {
                    let rh = ui.pt(m.row);
                    if pressed == Some(Target::Search(i)) {
                        s.fill_rrect(cx + ui.pt(6.0), y, cw - ui.pt(12.0), rh, ui.pt(8.0), a(PRESSED));
                    }
                    let (gx, gr) = (cx + ui.pt(m.row_icon_x + m.row_icon / 2.0 - 3.0), ui.pt(5.5));
                    let stroke = ui.pt(1.6);
                    let glass = glyphs::either(glyphs::ring((gx, y + rh / 2.0 - ui.pt(1.0)), gr, stroke), glyphs::segment((gx + gr * 0.72, y + rh / 2.0 - ui.pt(1.0) + gr * 0.72), (gx + gr * 1.5, y + rh / 2.0 - ui.pt(1.0) + gr * 1.5), stroke * 1.2));
                    glyphs::paint(s, (gx - gr - stroke, y + rh / 2.0 - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), a(SECONDARY), glass);
                    s.blit(label, (cx + ui.pt(m.row_text_x)) as i32, (y + rh / 2.0 - label.h as f32 / 2.0) as i32, 255, alpha);
                    y += rh;
                }
            }
        } else {
            for (si, sec) in self.sections.iter().enumerate() {
                let hd = &sec.header;
                let hy = (y + ui.pt(m.header_mid) - hd.h as f32 / 2.0) as i32;
                if self.mac { s.blit_colored(hd, header_x, hy, LABEL_2.now(), alpha) } else { s.blit(hd, header_x, hy, 255, alpha) }
                y += ui.pt(m.header);
                for (ri, (hit, label)) in sec.rows.iter().enumerate() {
                    let rh = ui.pt(m.row);
                    let top = self.mac && si == 0 && ri == 0;
                    if top {
                        let pi = ui.pt(MAC_PLATE_INSET_PT);
                        s.fill_rrect(cx + pi, y, cw - 2.0 * pi, rh, ui.pt(MAC_PLATE_R_PT), a(SIDEBAR_PLATE.now())); // the reference: a neutral plate, not the accent
                    } else if pressed == Some(Target::Row(si, ri)) || self.selected == Some((si, ri)) {
                        s.fill_rrect(cx + ui.pt(6.0), y, cw - ui.pt(12.0), rh, ui.pt(8.0), a(PRESSED));
                    }
                    let (ix, isz) = (cx + ui.pt(m.row_icon_x), ui.pt(m.row_icon));
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
                        Hit::Settings(_) => None,
                    };
                    // an indexed item with a readable thumbnail shows it in place of its app's icon
                    let thumb_img = match hit {
                        Hit::Indexed { app_id, thumb, .. } if !thumb.is_empty() => {
                            let key = format!("{app_id}/{thumb}");
                            self.thumbs.iter().find(|(k, s, _)| *k == key && *s == isz.round() as usize).and_then(|(_, _, img)| img.as_ref())
                        }
                        _ => None,
                    };
                    if let Some(img) = thumb_img {
                        s.blit(img, (ix + (isz - img.w as f32) / 2.0) as i32, (iy + (isz - img.h as f32) / 2.0) as i32, 255, alpha);
                    }
                    match (app, hit) {
                        (Some(app), _) if thumb_img.is_none() => {
                            if let Some((_, _, small, _)) = self.icons.iter().find(|(a, ..)| *a == app) {
                                s.blit(small, ix as i32, iy as i32, 255, alpha);
                            }
                        }
                        (None, Hit::Settings(name)) => {
                            let colour = self.icon_colour.iter().find(|(n, _)| n == name).map_or(SECONDARY, |(_, c)| *c);
                            s.fill_rrect(ix, iy, isz, isz, ui.pt(6.5), a(colour));
                        }
                        _ => {}
                    }
                    let (lx, ly) = ((cx + ui.pt(m.row_text_x)) as i32, (y + rh / 2.0 - label.h as f32 / 2.0) as i32);
                    if self.mac { s.blit_colored(label, lx, ly, LABEL.now(), alpha) } else { s.blit(label, lx, ly, 255, alpha) }
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
        self.quicksearch.set_mac(&self.ui, self.desk_active());
        if !self.quicksearch.has_icons() {
            let icons = apps().iter().enumerate().map(|(i, a)| (a.id, if i < super::IN_DOCK { &self.dock[i].icon } else { &self.grid[i - super::IN_DOCK].icon })).collect();
            self.quicksearch.set_icons(&self.ui, icons);
        }
        let root = self.files_locations.first().map_or_else(|| crate::files::fs::home(), |l| l.path.clone());
        self.quicksearch.refresh_sources(&super::contactstore::store_path(&self.data_dir), &self.deildu_dir, root);
        self.quicksearch.open(&self.ui, self.h, y0, y, t, range);
    }

    /// Cmd/Meta+Space: opens Search fully from the home screen, and on the desktop over any app (Golden Gate).
    pub(super) fn open_quicksearch_hw(&mut self, t: f64) {
        if (self.screen != Screen::Home && !self.desk_active()) || self.quicksearch.active() {
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
            Hit::App(app) => self.open(app, t),
            Hit::Settings(name) => return self.open_settings_page(name, t),
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
        match self.quicksearch.key(&self.ui, self.h, out, t) {
            Some(hit) => self.quicksearch_act(hit, t),
            None => vec![],
        }
    }

    /// Advances the card's settle and keyboard; true when a frame is needed.
    pub(super) fn quicksearch_tick(&mut self, t: f64) -> bool {
        self.quicksearch.tick(&self.ui, self.h, t)
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
        assert_eq!(w, sh.ui.pt(520.0));
        assert!((x - (sh.w as f32 - w) / 2.0).abs() < 1.0);
        assert_eq!(y, sh.ui.pt(139.0));
        assert_eq!(h, sh.ui.pt(sh.quicksearch.m().field), "the empty Golden Gate field shows no Assistant Suggestions, only the field");
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
fn mac_glass(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, r: f32, alpha: u32) {
    use bifrost_ui::material_request::ShellSurface;
    let req = ShellSurface::Spotlight.request((x, y, w, h), r);
    let fill = if alpha == 255 {
        super::control_center::request_shell_material(req)
    } else if bifrost_render::appearance::reduce_transparency() {
        MENU_SOLID.now()
    } else {
        bifrost_render::glass::tint(MENU_TINT.now())
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

    #[test]
    fn a_file_match_comes_with_show_in_folder_for_the_top_match() {
        let files = vec![entry("/home/me/Docs", "Q1 report.pdf"), entry("/home/me/Pics", "Beach.png")];
        let folded: Vec<String> = files.iter().map(|e| fuzzy::fold(&e.name)).collect();
        let groups = file_note_groups("report", &files, &folded);
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].title, "Files");
        assert_eq!(groups[0].hits, vec![Hit::File { path: PathBuf::from("/home/me/Docs/Q1 report.pdf"), name: "Q1 report.pdf".into() }]);
        assert_eq!(groups[1], Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/Docs"), name: "Q1 report.pdf".into() }] });
        assert!(file_note_groups("zzz", &files, &folded).is_empty());
        assert!(file_note_groups("  ", &files, &folded).is_empty());
    }

    #[test]
    fn space_previews_an_arrow_selected_file_and_types_otherwise() {
        let u = ui();
        let mut q = QuickSearch::new(&u, 800.0, 1.0);
        let file = Hit::File { path: PathBuf::from("/home/me/a.txt"), name: "a.txt".into() };
        let section = |u: &Ui| Section { header: u.text("Files", 20.0, Weight::Semibold, WHITE), rows: vec![(file.clone(), u.text("a.txt", 13.0, Weight::Regular, WHITE))] };
        q.sections = vec![section(&u)];
        assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), None, "no row selected: the space is typed");
        assert_eq!(q.query(), " ");
        q.sections = vec![section(&u)];
        q.selected = Some((0, 0));
        assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), Some(file), "Space previews the selected file");
        assert_eq!(q.query(), " ", "the query is untouched by the preview");
    }
}
