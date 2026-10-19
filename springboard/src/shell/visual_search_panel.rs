//! Look Up in Visual Search: the panel that sends a picture to the apps that answer visual searches and lists what they
//! found.
//!
//! The person asks for it: the Markup editor (the screenshot thumbnail's result, and Preview's Markup) has a Look Up
//! button, and the Photos viewer's Info sheet has a "Look Up in Visual Search" row. The shell builds a scene from the
//! picture (`bifrost_ui::visual_search::Scene`, shrunk for the wire by `visual_search_wire`), and a worker thread
//! delivers it through `intents::visual_search` to every installed app that declares the `visual-search` intent and is
//! running. The matches come back merged, best first, and the panel lists them: a title, a subtitle and the app that
//! found it. A tap on a match opens that app and runs the intent the match names.
//!
//! Privacy: nothing is sent until the person taps Look Up, the picture goes only to apps that declare the intent and
//! are already open (an app is never started for a search), and the panel says so. The scene carries no labels: the
//! shell has no classifier, so an app matches on the pixels. The panel is drawn over everything, takes every touch
//! while it is up, and Escape or a tap outside closes it.

use super::apphost;
use super::intents;
use super::ui::{Ui, Weight, LABEL, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MENU_SOLID, SECONDARY, SEPARATOR_C, WHITE};
use super::{Drawn, Effect, Event, Shell};
use crate::gfx::{Image, Surface};
use crate::layout::Rect;
use bifrost_ui::prefs_layout::m::CARD_RADIUS;
use bifrost_ui::visual_search::{Match, Scene, SceneKind};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// The panel's width, in points.
const WIDTH_PT: f32 = 360.0;
const PAD_PT: f32 = 14.0;
/// Gap between the menu bar and the panel's top edge.
const GAP_PT: f32 = 8.0;
const HEADER_PT: f32 = 26.0;
/// The picture band.
const PREVIEW_PT: f32 = 108.0;
/// A line of the status text and of the footnote.
const LINE_PT: f32 = 17.0;
const FOOT_LINE_PT: f32 = 14.0;
/// One match: a title over a subtitle.
const ROW_PT: f32 = 46.0;
const GAP_BAND_PT: f32 = 10.0;
/// The longest side of the picture kept for the panel's preview, in px.
const PREVIEW_PX: usize = 320;
/// How long the shell waits for an app to start answering when a match is opened.
const INTENT_WAIT: Duration = Duration::from_secs(8);
/// What the panel tells the person about where the picture goes.
pub const PRIVACY_NOTE: &str = "The picture goes only to apps that answer visual searches and are open now. Nothing leaves this computer.";

/// What the search has produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Searching,
    Found(Vec<Match>),
    /// The picture could not be made into a scene (too large, empty).
    Failed(String),
}

/// What a tap on the panel does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Inside the panel, on nothing that acts.
    None,
    /// Outside the panel.
    Close,
    /// The match at this index of the list.
    Open(usize),
}

/// The panel on screen.
pub struct Panel {
    /// The picture searched for, shrunk to a preview.
    preview: Image,
    state: State,
    /// The installed apps that answer visual searches, as (app id, name).
    providers: Vec<(String, String)>,
}

/// The panel's boxes in px.
struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    header: (f32, f32),
    picture: (f32, f32, f32, f32),
    message: (f32, Vec<String>),
    rows: Vec<(f32, f32, f32, f32)>,
    note: (f32, Vec<String>),
}

impl Panel {
    pub fn new(preview: Image, providers: Vec<(String, String)>) -> Panel {
        Panel { preview, state: State::Searching, providers }
    }

    #[cfg(test)]
    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn matches(&self) -> &[Match] {
        match &self.state {
            State::Found(m) => m,
            _ => &[],
        }
    }

    /// The text of the status band, or None while matches are listed.
    pub fn message(&self) -> Option<String> {
        match &self.state {
            State::Searching => Some("Searching…".to_string()),
            State::Failed(why) => Some(why.clone()),
            State::Found(m) if m.is_empty() => Some(if self.providers.is_empty() {
                "No installed app answers visual searches.".to_string()
            } else {
                "No app found a match. Apps are asked only while they are open.".to_string()
            }),
            State::Found(_) => None,
        }
    }

    /// The name of the app with this id, else the id.
    pub fn app_name(&self, id: &str) -> String {
        self.providers.iter().find(|(i, _)| i == id).map_or_else(|| id.to_string(), |(_, n)| n.clone())
    }

    /// The box and bands for a `w` x `h` px screen whose menu bar is `top` px high.
    fn frame(&self, ui: &Ui, w: i32, h: i32, top: f32) -> Frame {
        let pad = ui.pt(PAD_PT);
        let pw = ui.pt(WIDTH_PT).min(w as f32 - pad * 2.0).max(0.0);
        let inner = (pw - pad * 2.0).max(0.0);
        let mut y = pad;
        let header = (y, ui.pt(HEADER_PT));
        y += header.1;
        let band = (inner, ui.pt(PREVIEW_PT));
        let k = (band.0 / self.preview.w.max(1) as f32).min(band.1 / self.preview.h.max(1) as f32).min(1.0);
        let (pic_w, pic_h) = ((self.preview.w as f32 * k).round(), (self.preview.h as f32 * k).round());
        let picture = (pad + (inner - pic_w) / 2.0, y + (band.1 - pic_h) / 2.0, pic_w, pic_h);
        y += band.1 + ui.pt(GAP_BAND_PT);
        let mut rows = Vec::new();
        let mut message = (y, Vec::new());
        match self.message() {
            Some(text) => {
                message = (y, ui.wrap(&text, MAC_BODY_PT, Weight::Regular, inner));
                y += ui.pt(LINE_PT) * message.1.len().max(1) as f32;
            }
            None => {
                for _ in self.matches() {
                    rows.push((pad, y, inner, ui.pt(ROW_PT)));
                    y += ui.pt(ROW_PT);
                }
            }
        }
        y += ui.pt(GAP_BAND_PT);
        let lines = ui.wrap(PRIVACY_NOTE, MAC_FOOTNOTE_PT, Weight::Regular, inner);
        let note = (y, lines);
        y += ui.pt(FOOT_LINE_PT) * note.1.len() as f32;
        let ph = y + pad;
        let x = ((w as f32 - pw) / 2.0).max(0.0);
        let y0 = (top + ui.pt(GAP_PT)).min(h as f32 - pad - ph).max(0.0);
        // everything above was laid out from the panel's own top; move it to its place
        let shift = |(rx, ry, rw, rh): (f32, f32, f32, f32)| (x + rx, y0 + ry, rw, rh);
        Frame {
            x,
            y: y0,
            w: pw,
            h: ph,
            header: (y0 + header.0, header.1),
            picture: shift(picture),
            message: (y0 + message.0, message.1),
            rows: rows.into_iter().map(shift).collect(),
            note: (y0 + note.0, note.1),
        }
    }

    /// What a tap at (`px`, `py`) does on a `w` x `h` px screen whose menu bar is `top` px high.
    pub fn tap(&self, ui: &Ui, w: i32, h: i32, top: f32, px: f32, py: f32) -> Outcome {
        let f = self.frame(ui, w, h, top);
        if !(px >= f.x && px < f.x + f.w && py >= f.y && py < f.y + f.h) {
            return Outcome::Close;
        }
        match f.rows.iter().position(|&(rx, ry, rw, rh)| px >= rx && px < rx + rw && py >= ry && py < ry + rh) {
            Some(i) => Outcome::Open(i),
            None => Outcome::None,
        }
    }

    pub fn draw(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32, top: f32) {
        let f = self.frame(ui, w, h, top);
        let radius = ui.pt(CARD_RADIUS);
        s.fill_rrect(f.x, f.y, f.w, f.h, radius, MENU_SOLID.now());
        s.stroke_rrect(f.x, f.y, f.w, f.h, radius, ui.pt(1.0).max(1.0), SEPARATOR_C.now());
        let pad = ui.pt(PAD_PT);
        let text = |s: &mut Surface<'_>, t: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32| {
            let img = ui.text(t, pt, weight, WHITE);
            s.blit_colored(&img, x as i32, y as i32, color, 255);
        };
        // the title, centred in its band
        let title = ui.text("Visual Search", MAC_HEADLINE_PT, Weight::Semibold, WHITE);
        s.blit_colored(&title, (f.x + (f.w - title.w as f32) / 2.0) as i32, (f.header.0 + (f.header.1 - title.h as f32) / 2.0) as i32, LABEL.now(), 255);
        let (px, py, pw, ph) = f.picture;
        if pw > 0.0 && ph > 0.0 {
            super::switcher::draw_scaled(s, &self.preview, Rect { x: px as i32, y: py as i32, w: pw as i32, h: ph as i32 }, ui.pt(4.0), 255);
        }
        for (i, line) in f.message.1.iter().enumerate() {
            text(s, line, MAC_BODY_PT, Weight::Regular, SECONDARY, f.x + pad, f.message.0 + ui.pt(LINE_PT) * i as f32);
        }
        for (m, &(rx, ry, rw, rh)) in self.matches().iter().zip(&f.rows) {
            s.fill_rect(rx as usize, ry as usize, rw as usize, 1, SEPARATOR_C.now());
            let app = ui.text(&self.app_name(&m.app), MAC_SUBHEAD_PT, Weight::Regular, WHITE);
            let room = (rw - app.w as f32 - ui.pt(8.0)).max(0.0);
            text(s, &ui.fit(&m.title, MAC_BODY_PT, room), MAC_BODY_PT, Weight::Medium, LABEL.now(), rx, ry + ui.pt(6.0));
            if !m.subtitle.is_empty() {
                text(s, &ui.fit(&m.subtitle, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, SECONDARY, rx, ry + ui.pt(24.0));
            }
            s.blit_colored(&app, (rx + rw - app.w as f32) as i32, (ry + (rh - app.h as f32) / 2.0) as i32, SECONDARY, 255);
        }
        for (i, line) in f.note.1.iter().enumerate() {
            text(s, line, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, f.x + pad, f.note.0 + ui.pt(FOOT_LINE_PT) * i as f32);
        }
    }
}

/// A photo being read and decoded on a worker, and the search to run with it once it has landed.
pub struct Load {
    rx: Receiver<Option<Image>>,
    search: Box<dyn FnOnce(&Scene) -> Vec<Match> + Send>,
}

/// A search in progress or finished: the panel and the channel the worker answers on.
pub struct Session {
    pub panel: Panel,
    rx: Option<Receiver<Vec<Match>>>,
}

impl Session {
    pub fn searching(&self) -> bool {
        self.rx.is_some()
    }
}

/// The scene for `img`: its pixels as RGBA, kind `kind`. Err names why it cannot be one.
pub fn scene_of(img: &Image, kind: SceneKind) -> Result<Scene, String> {
    bifrost_ui::visual_search_wire::scene_from_pixels(kind, img.w as u32, img.h as u32, &img.px).map_err(|e| match e {
        bifrost_ui::visual_search::SceneError::Size => "This picture is empty or too large to look up.".to_string(),
        bifrost_ui::visual_search::SceneError::PixelCount { .. } => "This picture could not be read.".to_string(),
    })
}

impl Shell {
    /// Looks the photo file at `path` up in the apps that answer visual searches: the file is read and decoded on a worker
    /// (a large picture takes a while), and the panel opens when it has landed.
    pub(super) fn visual_search_photo(&mut self, path: PathBuf) {
        let apps = apphost::installed();
        self.visual_search_photo_with(path, move |scene| intents::visual_search(&apps, scene));
    }

    /// As [`Shell::visual_search_photo`], with the delivery given (see [`Shell::visual_search_with`]).
    pub(super) fn visual_search_photo_with<F>(&mut self, path: PathBuf, search: F)
    where
        F: FnOnce(&Scene) -> Vec<Match> + Send + 'static,
    {
        let (tx, rx) = channel();
        crate::bare::sys::spawn_named("visual-search-load", move || {
            crate::gfx::affinity::background_thread();
            let _ = tx.send(std::fs::read(&path).ok().and_then(|bytes| crate::cg::imageio::decode(&bytes)));
        });
        self.visual_search_load = Some(Load { rx, search: Box::new(search) });
    }

    /// Opens the panel on a failure: no search is started.
    fn visual_search_fail(&mut self, why: &str) {
        let mut panel = Panel::new(Image::new(1, 1), self.visual_search_providers());
        panel.state = State::Failed(why.to_string());
        self.visual_search = Some(Session { panel, rx: None });
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// The installed apps that answer visual searches, as (app id, name).
    fn visual_search_providers(&self) -> Vec<(String, String)> {
        apphost::installed()
            .iter()
            .filter(|a| a.manifest.intents.iter().any(|i| i.id == bifrost_ui::visual_search_wire::PROVIDER_INTENT))
            .map(|a| (a.manifest.id.clone(), a.manifest.name.clone()))
            .collect()
    }

    /// Looks `img` up in the apps that answer visual searches: opens the panel and delivers the scene on a worker. The
    /// installed apps are asked through `intents::visual_search`.
    pub(super) fn visual_search_start(&mut self, img: &Image, kind: SceneKind) {
        let apps = apphost::installed();
        self.visual_search_with(img, kind, move |scene| intents::visual_search(&apps, scene));
    }

    /// As [`Shell::visual_search_start`], with the delivery given: `search` runs on a worker thread with the scene and
    /// returns the matches. Tests give it a fixed answer.
    pub(super) fn visual_search_with<F>(&mut self, img: &Image, kind: SceneKind, search: F)
    where
        F: FnOnce(&Scene) -> Vec<Match> + Send + 'static,
    {
        let providers = self.visual_search_providers();
        let preview = if img.w == 0 || img.h == 0 || img.px.len() < img.w * img.h {
            Image::new(1, 1)
        } else {
            let k = (PREVIEW_PX as f32 / img.w.max(img.h) as f32).min(1.0);
            img.resized(((img.w as f32 * k).round() as usize).max(1), ((img.h as f32 * k).round() as usize).max(1))
        };
        let mut panel = Panel::new(preview, providers);
        let mut rx = None;
        match scene_of(img, kind) {
            Ok(scene) => {
                let (tx, r) = channel();
                rx = Some(r);
                crate::bare::sys::spawn_named("visual-search", move || {
                    crate::gfx::affinity::background_thread();
                    let _ = tx.send(search(&scene));
                });
            }
            Err(why) => panel.state = State::Failed(why),
        }
        self.visual_search = Some(Session { panel, rx });
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Takes the worker's answer when it has landed.
    pub(super) fn visual_search_tick(&mut self) {
        let landed = match self.visual_search_load.as_ref().map(|l| l.rx.try_recv()) {
            Some(Ok(img)) => Some(img),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            Some(Err(TryRecvError::Empty)) | None => None,
        };
        if let Some(img) = landed {
            if let Some(load) = self.visual_search_load.take() {
                match img {
                    Some(img) => self.visual_search_with(&img, SceneKind::Photo, load.search),
                    None => self.visual_search_fail("This picture could not be opened."),
                }
            }
        }
        let Some(session) = self.visual_search.as_mut() else { return };
        let Some(rx) = &session.rx else { return };
        let found = match rx.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return,
            // the worker died without answering: nothing was found
            Err(TryRecvError::Disconnected) => Vec::new(),
        };
        session.rx = None;
        session.panel.state = State::Found(found);
        self.dirty = true;
    }

    /// True while a worker has not answered yet: the shell polls for it.
    pub(super) fn visual_search_pending(&self) -> bool {
        self.visual_search_load.is_some() || self.visual_search.as_ref().is_some_and(Session::searching)
    }

    /// Escape closes the panel. False when it is not up, so Escape goes on to its other uses.
    pub(super) fn visual_search_escape(&mut self) -> bool {
        if self.visual_search.take().is_none() {
            return false;
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        true
    }

    /// Touches while the panel is up: a tap on a match opens its app, a tap outside closes the panel, and the panel takes
    /// every touch so none reaches what is under it. None when it is not up, or for a non-touch event.
    pub(super) fn visual_search_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        self.visual_search.as_ref()?;
        let (x, y) = match ev {
            Event::Down(..) | Event::Move(..) => return Some(Vec::new()),
            Event::Up(x, y) => (x, y),
            _ => return None,
        };
        let (w, h, top) = (self.w, self.h, self.visual_search_top());
        let outcome = self.visual_search.as_ref()?.panel.tap(&self.ui, w, h, top, x, y);
        match outcome {
            Outcome::None => {}
            Outcome::Close => self.visual_search = None,
            Outcome::Open(i) => {
                let found = self.visual_search.as_ref().and_then(|s| s.panel.matches().get(i).cloned());
                self.visual_search = None;
                if let Some(m) = found {
                    self.visual_search_open(&m, t);
                }
            }
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        Some(Vec::new())
    }

    /// Opens the app that found `m` and runs the intent it names, as a shortcut does: on a worker, retrying the
    /// connection while the app comes to the front.
    fn visual_search_open(&mut self, m: &Match, t: f64) {
        let Some(slot) = apphost::slot_of(&m.app) else {
            eprintln!("visual search: {} is not installed", m.app);
            return;
        };
        self.open(super::AppId::External(slot as u8), t);
        let (app, intent) = (m.app.clone(), m.intent.clone());
        crate::bare::sys::spawn_named("intent-retry", move || {
            if let Err(e) = intents::run(&app, &intent, INTENT_WAIT) {
                eprintln!("visual search: intent {intent} of {app}: {e}");
            }
        });
    }

    /// The menu bar's height in px, which the panel drops below.
    fn visual_search_top(&self) -> f32 {
        #[cfg(feature = "desktop-wayland")]
        {
            self.desktop_bar_height() as f32
        }
        #[cfg(not(feature = "desktop-wayland"))]
        {
            0.0
        }
    }

    pub(super) fn draw_visual_search(&self, s: &mut Surface<'_>) {
        if let Some(v) = &self.visual_search {
            v.panel.draw(s, &self.ui, self.w, self.h, self.visual_search_top());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn picture() -> Image {
        Image { w: 40, h: 30, px: vec![pack(200, 40, 40, 255); 40 * 30] }
    }

    fn found(app: &str, title: &str, score: u32) -> Match {
        Match { app: app.into(), intent: "open-plant".into(), title: title.into(), subtitle: "Genus".into(), score }
    }

    fn panel(state: State) -> Panel {
        let mut p = Panel::new(picture(), vec![("is.x.plants".into(), "Plants".into())]);
        p.state = state;
        p
    }

    #[test]
    fn the_status_band_says_what_is_going_on() {
        assert_eq!(panel(State::Searching).message().as_deref(), Some("Searching…"));
        assert!(panel(State::Found(vec![])).message().is_some_and(|m| m.contains("open")), "no match: apps are asked only while open");
        let mut none = Panel::new(picture(), vec![]);
        none.state = State::Found(vec![]);
        assert!(none.message().is_some_and(|m| m.contains("No installed app")));
        assert_eq!(panel(State::Found(vec![found("is.x.plants", "Fern", 90)])).message(), None, "matches replace the status");
        assert_eq!(panel(State::Failed("too big".into())).message().as_deref(), Some("too big"));
    }

    #[test]
    fn a_tap_outside_closes_a_tap_on_a_match_opens_it_and_a_tap_elsewhere_does_nothing() {
        let ui = crate::shell::tests::ui();
        let p = panel(State::Found(vec![found("is.x.plants", "Fern", 90), found("is.x.plants", "Moss", 40)]));
        let (w, h) = (1200, 800);
        assert_eq!(p.tap(&ui, w, h, 0.0, 2.0, 790.0), Outcome::Close);
        let f = p.frame(&ui, w, h, 0.0);
        assert_eq!(f.rows.len(), 2);
        for (i, &(x, y, rw, rh)) in f.rows.iter().enumerate() {
            assert_eq!(p.tap(&ui, w, h, 0.0, x + rw / 2.0, y + rh / 2.0), Outcome::Open(i));
        }
        let (px, py, _, _) = f.picture;
        assert_eq!(p.tap(&ui, w, h, 0.0, px + 1.0, py + 1.0), Outcome::None, "the picture is not a button");
    }

    #[test]
    fn the_panel_stays_inside_a_small_screen() {
        let ui = crate::shell::tests::ui();
        let p = panel(State::Found((0..8).map(|i| found("is.x.plants", &format!("Plant {i}"), 50)).collect()));
        let f = p.frame(&ui, 300, 600, 0.0);
        assert!(f.x >= 0.0 && f.x + f.w <= 300.0, "x {} w {}", f.x, f.w);
        assert!(f.y >= 0.0, "y {}", f.y);
    }

    #[test]
    fn an_app_is_named_by_its_manifest_name() {
        let p = panel(State::Searching);
        assert_eq!(p.app_name("is.x.plants"), "Plants");
        assert_eq!(p.app_name("is.x.gone"), "is.x.gone");
    }

    #[test]
    fn a_picture_becomes_a_scene_of_its_pixels_or_says_why_not() {
        let scene = scene_of(&picture(), SceneKind::Screenshot).expect("a scene");
        assert_eq!((scene.width, scene.height), (40, 30));
        assert_eq!(scene.pixel(0, 0), Some([200, 40, 40, 255]), "red, green, blue, alpha in order");
        assert!(scene.labels.is_empty(), "the shell has no classifier");
        assert!(scene_of(&Image { w: 0, h: 0, px: vec![] }, SceneKind::Photo).is_err());
    }

    // --- the wired path: the shell delivers a scene, lists the matches and opens one ------------------------------------

    use crate::shell::tests::desktop_shell;
    use std::sync::{Arc, Mutex};

    /// Waits for the worker's answer, as frames would.
    fn settle(sh: &mut Shell) {
        let began = std::time::Instant::now();
        while sh.visual_search_pending() && began.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
            sh.visual_search_tick();
        }
    }

    #[test]
    fn a_look_up_delivers_the_scene_and_lists_the_matches_best_first() {
        let mut sh = desktop_shell();
        let seen = Arc::new(Mutex::new(None));
        let got = Arc::clone(&seen);
        sh.visual_search_with(&picture(), SceneKind::Photo, move |scene| {
            *got.lock().unwrap() = Some((scene.kind, scene.width, scene.height, scene.pixel(1, 1)));
            vec![found("is.x.plants", "Fern", 90)]
        });
        assert!(sh.visual_search_pending(), "the search is out");
        assert_eq!(sh.visual_search.as_ref().unwrap().panel.state(), &State::Searching);
        settle(&mut sh);
        assert_eq!(*seen.lock().unwrap(), Some((SceneKind::Photo, 40, 30, Some([200, 40, 40, 255]))), "the app got the picture's pixels");
        let session = sh.visual_search.as_ref().unwrap();
        assert!(!session.searching());
        assert_eq!(session.panel.matches(), &[found("is.x.plants", "Fern", 90)]);
        assert!(sh.dirty, "the answer repaints the panel");
    }

    #[test]
    fn the_panel_is_drawn_over_the_screen_and_takes_every_touch() {
        let mut sh = desktop_shell();
        sh.visual_search_with(&picture(), SceneKind::Screenshot, |_| vec![found("is.x.plants", "Fern", 90)]);
        settle(&mut sh);
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; w * h];
        sh.draw_visual_search(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        assert!(buf.iter().any(|&p| p != 0), "the card is drawn");
        // a touch that lands outside the card closes it and goes no further
        assert_eq!(sh.visual_search_event(Event::Down(2.0, h as f32 - 2.0), 1.0), Some(Vec::new()));
        assert!(sh.visual_search.is_some(), "a press alone does nothing");
        assert_eq!(sh.visual_search_event(Event::Up(2.0, h as f32 - 2.0), 2.0), Some(Vec::new()));
        assert!(sh.visual_search.is_none(), "a tap outside closes it");
        assert_eq!(sh.visual_search_event(Event::Up(2.0, 2.0), 3.0), None, "with no panel the touch goes on");
    }

    #[test]
    fn a_tap_on_a_match_closes_the_panel_and_escape_closes_it_too() {
        let mut sh = desktop_shell();
        sh.visual_search_with(&picture(), SceneKind::Screenshot, |_| vec![found("is.x.not-installed", "Fern", 90)]);
        settle(&mut sh);
        let top = sh.visual_search_top();
        let (x, y, rw, rh) = sh.visual_search.as_ref().unwrap().panel.frame(&sh.ui, sh.w, sh.h, top).rows[0];
        sh.visual_search_event(Event::Up(x + rw / 2.0, y + rh / 2.0), 1.0).expect("taken");
        assert!(sh.visual_search.is_none(), "opening a match closes the panel (an app that is not installed opens nothing)");
        sh.visual_search_with(&picture(), SceneKind::Screenshot, |_| Vec::new());
        assert!(sh.visual_search_escape());
        assert!(!sh.visual_search_escape(), "nothing left to close");
    }

    #[test]
    fn a_picture_that_cannot_be_a_scene_fails_in_the_panel_and_sends_nothing() {
        let mut sh = desktop_shell();
        let sent = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&sent);
        sh.visual_search_with(&Image { w: 0, h: 0, px: vec![] }, SceneKind::Screenshot, move |_| {
            *flag.lock().unwrap() = true;
            Vec::new()
        });
        assert!(!sh.visual_search_pending(), "no worker was started");
        assert!(matches!(sh.visual_search.as_ref().unwrap().panel.state(), State::Failed(_)));
        assert!(!*sent.lock().unwrap());
    }

    #[test]
    fn a_photo_file_is_decoded_on_a_worker_and_its_pixels_reach_the_search() {
        let mut sh = desktop_shell();
        let dir = std::env::temp_dir().join(format!("bifrost-visual-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("fern.png");
        std::fs::write(&file, crate::shell::screenshot::encode(&picture())).unwrap();
        let seen = Arc::new(Mutex::new(None));
        let got = Arc::clone(&seen);
        sh.visual_search_photo_with(file.clone(), move |scene| {
            *got.lock().unwrap() = Some((scene.kind, scene.width, scene.height));
            vec![found("is.x.plants", "Fern", 90)]
        });
        assert!(sh.visual_search_pending(), "the decode is out and the panel waits for it");
        assert!(sh.visual_search.is_none(), "the panel opens when the picture has landed");
        settle(&mut sh);
        assert_eq!(*seen.lock().unwrap(), Some((SceneKind::Photo, 40, 30)), "the decoded photo went to the search");
        assert_eq!(sh.visual_search.as_ref().unwrap().panel.matches().len(), 1);
        // a file that is not a picture says so and sends nothing
        std::fs::write(&file, b"not a picture").unwrap();
        let sent = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&sent);
        sh.visual_search_photo_with(file, move |_| {
            *flag.lock().unwrap() = true;
            Vec::new()
        });
        settle(&mut sh);
        assert!(matches!(sh.visual_search.as_ref().unwrap().panel.state(), State::Failed(_)));
        assert!(!*sent.lock().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
