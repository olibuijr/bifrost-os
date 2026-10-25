//! The Look Up popover: the dictionary definition and the quick search hits for a selected word, with two actions,
//! Search for the term and Open in Dictionary. `bifrost_ui::look_up` holds the content and its rows; this module
//! measures, draws and hit tests them. Opened by the Services menu's Look Up and by Ctrl+Cmd+D; Escape or a tap
//! outside closes it, and a tap on an action runs it.
//!
//! The popover also hosts the suggested actions for the items the selection holds (a link, an email address, a date):
//! `bifrost_ui::suggested_actions` finds them and says what each action does (`Route`), and a tap on one runs that
//! route here: copy to the clipboard, open the link in the browser, start a mail, or add an event to the calendar.

use super::quicksearch::{self, dict, Hit};
use super::{apphost, Drawn, Effect, Event, Shell};
use crate::gfx::Surface;
use crate::shell::ui::{Ui, Weight, CARD_BG, LABEL, LINK, SECONDARY, SEPARATOR_C, WHITE};
use bifrost_ui::look_up::{LookUp, Row, HITS_SHOWN};
use bifrost_ui::services_menu::{Selection, DICTIONARY_APP};
use bifrost_ui::suggested_actions::{self, Route, ALL_KINDS};

/// The Calendar app, which an Add to Calendar suggestion opens.
const CALENDAR_APP: &str = "is.olibuijr.calendar.desktop";
/// The popover's overlay surface on the desktop (`desktop_chrome_frames`): the menus' and panels' rank (`chrome_z`).
pub const CHROME_ID: u32 = super::chrome_ids::LOOK_UP;
/// The most selection text the suggestions look through, in characters.
const SELECTION_SCAN_MAX: usize = 4000;
/// The most suggested actions the model gathers (the popover lists `look_up::SUGGESTIONS_SHOWN` of them).
const SUGGESTIONS_GATHERED: usize = 8;

const WIDTH_PT: f32 = 320.0;
const RADIUS_PT: f32 = 6.0;
const PAD_PT: f32 = 12.0;
/// gap between the menu bar and the popover's top edge
const GAP_PT: f32 = 8.0;
/// a section title band
const HEAD_PT: f32 = 22.0;
const HEAD_SIZE: f32 = 11.0;
/// one line of text in a section (definitions and hits wrap onto as many lines as they need)
const TEXT_PT: f32 = 13.0;
const LINE_PT: f32 = 17.0;
/// an action row (Search, Open in Dictionary)
const ACTION_PT: f32 = 30.0;

/// What a tap asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// inside the popover, on no action: it stays open
    None,
    /// outside the popover: it closes
    Close,
    /// Search for the term: the popover closes and Quick Search opens with it
    Search(String),
    /// Open in Dictionary: the popover closes and the Dictionary app looks the word up
    Dictionary(String),
    /// A suggested action: the popover closes and the route runs
    Suggest(Route),
}

/// The popover on screen: its content and where it opens (px). It drops below `anchor`, centred on its x. On the desktop
/// it is drawn into the shell's frame and shown through an overlay surface of its own above the app windows
/// (`desktop_chrome_frames`, [`CHROME_ID`]), as Search and the menus are: the shell's frame lies under every window.
pub struct Popover {
    pub look: LookUp,
    pub anchor: (f32, f32),
}

/// One laid out row: its top and height in pt below the top padding, and the wrapped lines of a text row.
struct Band {
    row: Row,
    top: f32,
    h: f32,
    lines: Vec<String>,
}

/// The popover's box in px and its rows.
struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bands: Vec<Band>,
}

/// The rows of `look`, with text wrapped to `max` px.
fn bands(look: &LookUp, ui: &Ui, max: f32) -> Vec<Band> {
    let mut top = 0.0;
    look.rows()
        .into_iter()
        .map(|row| {
            let (h, lines) = match &row {
                Row::Heading(_) => (HEAD_PT, Vec::new()),
                Row::Text(t) => {
                    let lines = ui.wrap(t, TEXT_PT, Weight::Regular, max);
                    (LINE_PT * lines.len().max(1) as f32, lines)
                }
                Row::Search { .. } | Row::OpenInDictionary { .. } | Row::Suggest { .. } => (ACTION_PT, Vec::new()),
            };
            let band = Band { row, top, h, lines };
            top += h;
            band
        })
        .collect()
}

/// A band's rectangle in px (x, y, w, h).
fn band_rect(f: &Frame, ui: &Ui, b: &Band) -> (f32, f32, f32, f32) {
    let pad = ui.pt(PAD_PT);
    (f.x + pad, f.y + pad + ui.pt(b.top), f.w - pad * 2.0, ui.pt(b.h))
}

impl Popover {
    pub fn new(look: LookUp, anchor: (f32, f32)) -> Self {
        Popover { look, anchor }
    }

    /// The box and rows for a `w` x `h` px screen: the box is narrowed to fit the screen and pulled inside it.
    fn frame(&self, ui: &Ui, w: i32, h: i32) -> Frame {
        let m = ui.pt(PAD_PT);
        let pw = ui.pt(WIDTH_PT).min(w as f32 - m * 2.0).max(0.0);
        let bands = bands(&self.look, ui, pw - m * 2.0);
        let content = bands.last().map_or(0.0, |b| b.top + b.h);
        let ph = ui.pt(content + PAD_PT * 2.0);
        let x = (self.anchor.0 - pw / 2.0).min(w as f32 - m - pw).max(m);
        let y = (self.anchor.1 + ui.pt(GAP_PT)).min(h as f32 - m - ph).max(m);
        Frame { x, y, w: pw, h: ph, bands }
    }

    /// The box on a `w` x `h` px screen (x, y, w, h in px).
    pub fn rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let f = self.frame(ui, w, h);
        (f.x, f.y, f.w, f.h)
    }

    /// What a tap at (`px`, `py`) does.
    pub fn tap(&self, ui: &Ui, w: i32, h: i32, px: f32, py: f32) -> Outcome {
        let f = self.frame(ui, w, h);
        if !(px >= f.x && px < f.x + f.w && py >= f.y && py < f.y + f.h) {
            return Outcome::Close;
        }
        for b in &f.bands {
            let (_, y, _, bh) = band_rect(&f, ui, b);
            if py >= y && py < y + bh {
                return match &b.row {
                    Row::Search { query } => Outcome::Search(query.clone()),
                    Row::OpenInDictionary { word } => Outcome::Dictionary(word.clone()),
                    Row::Suggest { route, .. } => Outcome::Suggest(route.clone()),
                    _ => Outcome::None,
                };
            }
        }
        Outcome::None
    }

    pub fn draw(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32) {
        let f = self.frame(ui, w, h);
        s.fill_rrect(f.x, f.y, f.w, f.h, ui.pt(RADIUS_PT), CARD_BG.now());
        for b in &f.bands {
            let (x, y, bw, bh) = band_rect(&f, ui, b);
            match &b.row {
                Row::Heading(title) => {
                    let t = ui.text(title, HEAD_SIZE, Weight::Semibold, WHITE);
                    s.blit_colored(&t, x as i32, (y + ui.pt(4.0)) as i32, SECONDARY, 255);
                }
                Row::Text(_) => {
                    for (i, line) in b.lines.iter().enumerate() {
                        let t = ui.text(line, TEXT_PT, Weight::Regular, WHITE);
                        s.blit_colored(&t, x as i32, (y + ui.pt(LINE_PT * i as f32)) as i32, LABEL.now(), 255);
                    }
                }
                Row::Search { .. } | Row::OpenInDictionary { .. } | Row::Suggest { .. } => {
                    let label = match &b.row {
                        Row::Search { query } => format!("Search for {query}"),
                        Row::OpenInDictionary { word } => format!("Open {word} in Dictionary"),
                        Row::Suggest { title, item, .. } => suggestion_label(title, item),
                        _ => String::new(),
                    };
                    s.fill_rect(x.max(0.0) as usize, y.max(0.0) as usize, bw.max(0.0) as usize, 1, SEPARATOR_C.now());
                    // text is rendered white and tinted on the way in, with the named colours in the surface's channel
                    // order: a literal ARGB value here drew the links orange
                    let t = ui.text(&ui.fit(&label, TEXT_PT, bw), TEXT_PT, Weight::Regular, WHITE);
                    s.blit_colored(&t, x as i32, (y + (bh - t.h as f32) / 2.0) as i32, LINK, 255);
                }
            }
        }
    }
}

/// The text of a suggested action row: the action, then the item it acts on.
fn suggestion_label(title: &str, item: &str) -> String {
    format!("{title}: {item}")
}

/// The suggested actions for the selection `text`: the actions of every link, email address and date in it that
/// something in BifrOSt performs (`suggested_actions::offers`).
fn suggestions_for(text: &str) -> Vec<suggested_actions::Offer> {
    let scan: String = text.chars().take(SELECTION_SCAN_MAX).collect();
    suggested_actions::offers(&scan, &ALL_KINDS, SUGGESTIONS_GATHERED)
}

/// The popover's content for `term`: the first definition the installed dictionaries give, and the quick search hits.
/// The dictionaries are the library Search holds (`quicksearch_dict`), read once per run on a worker thread: this
/// starts that read when it has not run yet, and until it is in the popover says it is looking the word up
/// (`look_up_tick` fills the definition in), so a large dictionary never stalls the shell.
fn content(term: &str) -> LookUp {
    dict::preload(apphost::root().join("appdata").join(DICTIONARY_APP));
    let definition = dict::definition(term);
    let loading = definition.is_none();
    let hits: Vec<String> = quicksearch::results(term, &[])
        .iter()
        .flat_map(|g| g.hits.iter())
        .filter_map(title_of)
        .take(HITS_SHOWN)
        .collect();
    LookUp::new(term, definition.flatten(), hits).with_loading(loading)
}

/// The text the suggestions look through: the whole text selection when there is one, else the term itself.
fn suggestion_source(term: &str, selection: Option<Selection>) -> String {
    match selection {
        Some(Selection::Text(text)) if text.contains(term) => text,
        _ => term.to_string(),
    }
}

/// The point the popover drops from: the bottom centre of the caret rectangle `(x, y, w, h)` (screen px).
fn caret_anchor((cx, cy, cw, ch): (f32, f32, f32, f32)) -> (f32, f32) {
    (cx + cw / 2.0, cy + ch)
}

/// An app's caret rectangle (`ToShell::CaretRect`, window points) on the screen in px: from the top left of the body of
/// the window `body` (px) at `k` px a point.
pub(super) fn caret_on_screen(body: (f32, f32), k: f32, (cx, cy, cw, ch): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (body.0 + cx * k, body.1 + cy * k, cw * k, ch * k)
}

/// The text a quick search result is listed by. A Terminal or Show in Folder result has no title to list.
fn title_of(hit: &Hit) -> Option<String> {
    Some(match hit {
        Hit::App(id) => super::apps::apps().iter().find(|a| a.id == *id)?.name.to_string(),
        Hit::Settings(p) | Hit::City(p) => p.to_string(),
        Hit::Deildu(t) | Hit::Answer(t) => t.clone(),
        Hit::Contact { name, .. } | Hit::File { name, .. } => name.clone(),
        Hit::Shortcut { title, .. } | Hit::Indexed { title, .. } | Hit::Event { title } => title.clone(),
        Hit::SettingsIntent { title, .. } => title.to_string(),
        Hit::SettingsAction { title, .. } | Hit::MediaTransport { title, .. } => title.clone(),
        // the popover shows the definition itself: a Search definition row would repeat it
        Hit::Terminal(_) | Hit::Reveal { .. } | Hit::Definition { .. } => return None,
    })
}

impl Shell {
    /// Opens the Look Up popover for `term`, taken from `selection` when the front window's selection gave it (its
    /// links, addresses and dates become suggested actions). It drops below the selection when the front app reported
    /// where its caret is (`look_up_anchor`), else below the menu bar, centred.
    pub(super) fn open_look_up(&mut self, term: String, selection: Option<Selection>) {
        let anchor = self.look_up_anchor();
        let source = suggestion_source(&term, selection);
        let look = content(&term).with_suggestions(suggestions_for(&source));
        self.look_up = Some(Popover::new(look, anchor));
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Where the popover opens (px): under the caret of the external app in front, which sits at the selection it
    /// reported (`ToShell::CaretRect`), else under the menu bar's centre.
    fn look_up_anchor(&self) -> (f32, f32) {
        self.front_ext_slot()
            .and_then(|slot| self.ext_caret_on_screen(slot))
            .map(caret_anchor)
            .unwrap_or((self.w as f32 / 2.0, self.desktop_bar_height() as f32))
    }

    /// The caret rectangle app `slot` reported (`ToShell::CaretRect`, points in its key window's content) on the screen
    /// in px. The app reports the caret of its key window: the app's front desk window when that is one of its own (a
    /// second document window), else its main window.
    pub(super) fn ext_caret_on_screen(&self, slot: u8) -> Option<(f32, f32, f32, f32)> {
        let caret = self.ext.get(slot as usize)?.as_ref()?.caret_rect?;
        let desk = self.desk.as_ref()?;
        let main = super::AppId::External(slot);
        let win = desk.front().filter(|w| w.app.parent() == main).or_else(|| desk.wins.iter().find(|w| w.app == main))?;
        let body = win.body(&self.desk_metrics());
        Some(caret_on_screen((body.x as f32, body.y as f32), self.ui.pt(1.0), caret))
    }

    /// The popover's box on the screen in px, clipped to it: its overlay surface and a region the GPU presenter keeps.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn look_up_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let (x, y, w, h) = self.look_up.as_ref()?.rect(&self.ui, self.w, self.h);
        let (x, y) = (x.max(0.0) as usize, y.max(0.0) as usize);
        let w = (w.max(0.0) as usize).min((self.w.max(0) as usize).saturating_sub(x));
        let h = (h.max(0.0) as usize).min((self.h.max(0) as usize).saturating_sub(y));
        (w > 0 && h > 0).then_some((x, y, w, h))
    }

    /// The popover as an overlay surface above the windows: its box and the alpha mask of its rounded corners.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn look_up_overlay(&self) -> Option<((usize, usize, usize, usize), Vec<u8>)> {
        let (x, y, w, h) = self.look_up_rect()?;
        Some(((x, y, w, h), crate::gfx::rrect_mask(w, h, self.ui.pt(RADIUS_PT))))
    }

    /// The dictionaries came in while the popover waited for them: its definition is filled in. True when it changed.
    pub(super) fn look_up_tick(&mut self) -> bool {
        let Some(p) = self.look_up.as_mut().filter(|p| p.look.loading) else { return false };
        let Some(definition) = dict::definition(&p.look.term) else { return false };
        p.look.definition = definition;
        p.look.loading = false;
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        true
    }

    /// True while the popover waits for the dictionaries (the shell wakes to fill the definition in).
    pub(super) fn look_up_waiting(&self) -> bool {
        self.look_up.as_ref().is_some_and(|p| p.look.loading)
    }

    /// Escape closes the popover. False when none is up, so Escape goes on to its other uses.
    pub(super) fn look_up_escape(&mut self) -> bool {
        if self.look_up.take().is_none() {
            return false;
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        true
    }

    /// Touches while the popover is up: a tap acts or closes it, and it takes every touch so none reaches the desktop
    /// under it. None when it is not up, or for a non-touch event.
    pub(super) fn look_up_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        self.look_up.as_ref()?;
        let (x, y) = match ev {
            Event::Down(..) | Event::Move(..) => return Some(Vec::new()),
            Event::Up(x, y) => (x, y),
            _ => return None,
        };
        let (w, h) = (self.w, self.h);
        let outcome = self.look_up.as_ref()?.tap(&self.ui, w, h, x, y);
        match outcome {
            Outcome::None => {}
            Outcome::Close => self.look_up = None,
            Outcome::Search(query) => {
                self.look_up = None;
                self.search_for(&query, t);
            }
            Outcome::Dictionary(word) => {
                self.look_up = None;
                self.run_intent(DICTIONARY_APP, "look-up", "word", &word, t);
            }
            Outcome::Suggest(route) => {
                self.look_up = None;
                self.run_suggestion(route, t);
            }
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        Some(Vec::new())
    }

    /// Carries out the route of a suggested action with the shell's own open paths.
    fn run_suggestion(&mut self, route: Route, t: f64) {
        match route {
            Route::Copy(text) => {
                if let Some(plain) = super::pasteboard::set(vec![(bifrost_ui::app_services::PLAIN_TEXT.to_string(), text.into_bytes())]) {
                    bifrost_render::clipboard::set_text(&plain);
                }
            }
            Route::OpenUrl(url) => {
                if let Err(e) = self.desktop_open_url(&url, t) {
                    eprintln!("look up: {e}");
                }
            }
            Route::ComposeMail(url) => {
                if let Err(e) = self.mail_compose_deliver("", &url, &[], t) {
                    eprintln!("look up: {e}");
                }
            }
            Route::CreateEvent { title, day } => {
                let mut params = vec![("title", title)];
                params.extend(day.map(|d| ("start", d)));
                self.run_intent_with(CALENDAR_APP, "create-event", params, t);
            }
        }
    }

    pub(super) fn draw_look_up(&self, s: &mut Surface<'_>) {
        if let Some(p) = &self.look_up {
            p.draw(s, &self.ui, self.w, self.h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::apps::AppId;
    use super::*;

    fn look() -> LookUp {
        LookUp::new("cat", Some("a small domesticated feline".into()), vec!["Cat photos".into()])
    }

    /// The centre of the first band that holds `row` (px), in a `w` x `h` screen.
    fn centre_of(p: &Popover, ui: &Ui, w: i32, h: i32, row: &Row) -> (f32, f32) {
        let f = p.frame(ui, w, h);
        let b = f.bands.iter().find(|b| &b.row == row).expect("row");
        let (x, y, bw, bh) = band_rect(&f, ui, b);
        (x + bw / 2.0, y + bh / 2.0)
    }

    #[test]
    fn a_tap_outside_closes_and_inside_on_text_keeps_it_open() {
        let ui = crate::shell::tests::ui();
        let p = Popover::new(look(), (600.0, 40.0));
        let (w, h) = (1200, 800);
        assert_eq!(p.tap(&ui, w, h, 1.0, 700.0), Outcome::Close);
        let (cx, cy) = centre_of(&p, &ui, w, h, &Row::Heading("Search"));
        assert_eq!(p.tap(&ui, w, h, cx, cy), Outcome::None);
    }

    #[test]
    fn the_actions_search_and_open_in_dictionary() {
        let ui = crate::shell::tests::ui();
        let p = Popover::new(look(), (600.0, 40.0));
        let (w, h) = (1200, 800);
        let (sx, sy) = centre_of(&p, &ui, w, h, &Row::Search { query: "cat".into() });
        assert_eq!(p.tap(&ui, w, h, sx, sy), Outcome::Search("cat".into()));
        let (dx, dy) = centre_of(&p, &ui, w, h, &Row::OpenInDictionary { word: "cat".into() });
        assert_eq!(p.tap(&ui, w, h, dx, dy), Outcome::Dictionary("cat".into()));
    }

    #[test]
    fn the_box_stays_inside_a_small_screen() {
        let ui = crate::shell::tests::ui();
        let p = Popover::new(look(), (5000.0, 5000.0));
        // a 300 x 200 px screen is shorter than the popover's content can ever be, so the test uses a small phone-sized screen
        let f = p.frame(&ui, 300, 600);
        assert!(f.x >= 0.0 && f.x + f.w <= 300.0, "x {} w {}", f.x, f.w);
        assert!(f.y >= 0.0 && f.y + f.h <= 600.0, "y {} h {}", f.y, f.h);
    }

    #[test]
    fn a_definition_wraps_onto_several_lines() {
        let ui = crate::shell::tests::ui();
        let long = "a small domesticated feline kept as a pet, which hunts mice and sleeps most of the day ".repeat(3);
        let rows = bands(&LookUp::new("cat", Some(long), vec![]), &ui, ui.pt(WIDTH_PT - PAD_PT * 2.0));
        let text = rows.iter().find(|b| matches!(b.row, Row::Text(_))).expect("definition band");
        assert!(text.lines.len() > 1, "{:?}", text.lines);
    }

    #[test]
    fn a_selection_with_links_lists_suggested_actions_that_run_on_a_tap() {
        let ui = crate::shell::tests::ui();
        let offers = suggestions_for("read https://example.com/a on 9.10.2026");
        let look = look().with_suggestions(offers);
        let p = Popover::new(look, (600.0, 40.0));
        let (w, h) = (1200, 1600);
        let row = Row::Suggest { title: "Open Link", item: "https://example.com/a".into(), route: Route::OpenUrl("https://example.com/a".into()) };
        let (x, y) = centre_of(&p, &ui, w, h, &row);
        assert_eq!(p.tap(&ui, w, h, x, y), Outcome::Suggest(Route::OpenUrl("https://example.com/a".into())));
        let event = Row::Suggest {
            title: "Add to Calendar",
            item: "9.10.2026".into(),
            route: Route::CreateEvent { title: "9.10.2026".into(), day: Some("2026-10-09".into()) },
        };
        let (ex, ey) = centre_of(&p, &ui, w, h, &event);
        assert_eq!(p.tap(&ui, w, h, ex, ey), Outcome::Suggest(Route::CreateEvent { title: "9.10.2026".into(), day: Some("2026-10-09".into()) }));
    }

    #[test]
    fn the_suggestions_look_through_the_whole_selection_only_when_it_holds_the_term() {
        let sel = Some(Selection::Text("see https://example.com now".into()));
        assert_eq!(suggestion_source("see https://example.com now", sel.clone()), "see https://example.com now");
        assert_eq!(suggestion_source("cat", sel), "cat", "a selection the term was not taken from is not scanned");
        assert_eq!(suggestion_source("cat", None), "cat");
        assert_eq!(suggestion_label("Copy Link", "https://x.is"), "Copy Link: https://x.is");
        assert!(suggestions_for("a plain word").is_empty());
    }

    /// The share of `px` (a `w` px wide frame) inside `(x, y, w, h)` that is clearly blue and clearly orange.
    fn blue_and_orange(px: &[u32], stride: usize, (x, y, w, h): (usize, usize, usize, usize)) -> (usize, usize) {
        let (mut blue, mut orange) = (0, 0);
        for row in y..y + h {
            for c in &px[row * stride + x..row * stride + x + w] {
                let (r, b) = (c & 255, (c >> 16) & 255);
                if b > r + 80 {
                    blue += 1;
                } else if r > b + 80 {
                    orange += 1;
                }
            }
        }
        (blue, orange)
    }

    /// E2E8 3: the action rows are drawn in the named link colour (systemBlue), in the surface's channel order; a literal
    /// ARGB value drew them orange.
    #[test]
    fn the_action_rows_are_link_blue_not_orange() {
        let ui = crate::shell::tests::ui();
        let p = Popover::new(look(), (300.0, 0.0));
        let (w, h) = (600usize, 900usize);
        let mut px = vec![0u32; w * h];
        p.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, w as i32, h as i32);
        let f = p.frame(&ui, w as i32, h as i32);
        for row in [Row::Search { query: "cat".into() }, Row::OpenInDictionary { word: "cat".into() }] {
            let b = f.bands.iter().find(|b| b.row == row).expect("action row");
            let (x, y, bw, bh) = band_rect(&f, &ui, b);
            // below the divider at the band's top
            let (blue, orange) = blue_and_orange(&px, w, (x as usize, y as usize + 2, bw as usize, bh as usize - 2));
            assert!(blue > 0, "{row:?}: the label is drawn in blue");
            assert_eq!(orange, 0, "{row:?}: nothing of it is orange");
        }
        assert_eq!(LINK, crate::shell::ui::TINT, "the link colour is systemBlue");
    }

    #[test]
    fn the_popover_drops_from_the_bottom_centre_of_the_caret() {
        assert_eq!(caret_anchor((120.0, 80.0, 2.0, 16.0)), (121.0, 96.0));
        // E2E9: the caret an app reports is in points; at 2 px a point it lands twice as far into the window
        assert_eq!(caret_on_screen((100.0, 50.0), 2.0, (20.0, 30.0, 1.0, 16.0)), (140.0, 110.0, 2.0, 32.0));
    }

    /// E2E8 2: on the desktop the popover is an overlay surface above the app windows (a menu's rank), on its own box,
    /// listed only while it is up; drawn only into the shell's frame it lay under the windows.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_popover_is_an_overlay_above_the_windows_only_while_it_is_up() {
        let _lock = crate::test_support::global_lock();
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 500.0);
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != CHROME_ID), "no popover, no overlay");
        sh.look_up = Some(Popover::new(look(), (sh.w as f32 / 2.0, sh.desktop_bar_height() as f32)));
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
        let frames = sh.desktop_chrome_frames(&px, w, h);
        let f = frames.iter().find(|f| f.id == CHROME_ID).expect("the popover is a surface above the windows");
        assert_eq!(Some((f.x as usize, f.y as usize, f.width, f.height)), sh.look_up_rect(), "on the popover's own box");
        assert_eq!(crate::bare::wayland::chrome_z(CHROME_ID), crate::bare::wayland::chrome_rank::PANEL, "over the windows, with the menus");
        let (blue, _) = blue_and_orange(&f.pixels, f.width, (0, 0, f.width, f.height));
        assert!(blue > 0, "the surface carries the popover's link rows");
        assert!(sh.look_up_escape());
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 2000.0);
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != CHROME_ID), "closed: the overlay is dropped");
    }

    #[test]
    fn quick_search_titles_name_the_results() {
        assert_eq!(title_of(&Hit::Settings("Wi-Fi")), Some("Wi-Fi".into()));
        assert_eq!(title_of(&Hit::City("Reykjavík")), Some("Reykjavík".into()));
        assert_eq!(title_of(&Hit::Answer("4".into())), Some("4".into()));
        assert_eq!(title_of(&Hit::Terminal("x".into())), None);
        assert!(title_of(&Hit::App(AppId::Terminal)).is_some());
    }
}
