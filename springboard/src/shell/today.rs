//! The Today View (bifrost 15): page "-1" of the home pager, reached with a
//! swipe right from the first page or the lock screen. A frosted copy of
//! the wallpaper fades in while the page slides over it, carrying a search
//! bar, a scrolling column of widget tiles and an Edit button that lets the
//! user switch widgets off and drag them into a new order (kept in
//! `today.ini`). Tiles are pre-rendered images (see `widgets`), refreshed
//! when the information behind them changes and once a minute.

use super::control_center::Frost;
use super::prefs::Prefs;
use super::ui::{Ui, Weight, WHITE};
use super::weather::data::Summary;
use super::widgets::{self, battery, calendar, clock, deildu, photos, terminal, weather, wifi, Kind, SMALL_PT};
use super::Info;
use crate::deildu::scroll::Scroller;
use crate::font::Font;
use crate::gfx::{rgba, Image, Surface, View};
use crate::layout::{Layout, Rect};
use std::collections::HashMap;
use std::path::PathBuf;

/// The widget column is 329 pt wide and centred.
pub const COL_PT: f32 = widgets::MEDIUM_PT;
/// Two small widgets fill the column with this gap between them.
pub const GAP_X_PT: f32 = COL_PT - 2.0 * SMALL_PT;
pub const GAP_PT: f32 = 16.0;
pub const SEARCH_TOP_PT: f32 = 32.0;
pub const SEARCH_H_PT: f32 = 36.0;
pub const EDIT_W_PT: f32 = 100.0;
pub const EDIT_H_PT: f32 = 44.0;
const EDIT_GAP_PT: f32 = 24.0;
const BOTTOM_PAD_PT: f32 = 16.0;
const BADGE_PT: f32 = 26.0;
/// blur radius on the quarter-size wallpaper copy, in pt
const BLUR_PT: f32 = 14.0;
/// A touch that moves this far (px) is a drag, not a tap.
pub const SLOP: f32 = 12.0;
const REFRESH_MS: f64 = 60_000.0;
const PREFS_FILE: &str = "today.ini";

/// Something a finger can land on, in page coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Search,
    /// index into `placed()`
    Widget(usize),
    /// the on/off badge of that tile (edit mode)
    Badge(usize),
    Edit,
}

/// What a tap asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenQuickSearch,
    OpenCalendar,
    OpenWeather,
    /// the Photos widget was tapped: show the photo picker to choose its picture
    ChoosePhoto,
    /// open the last played Deildu title
    OpenDeildu(deildu::Played),
}

/// A widget's place on the page (px, page coordinates before scrolling).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub kind: Kind,
    pub rect: Rect,
    /// shown live; false for the switched-off tiles listed in edit mode
    pub on: bool,
}

/// Flows `kinds` into the column: medium tiles take a row, small ones pair
/// up left to right. The off tiles (edit mode) follow after a section gap.
pub fn layout(s: f32, w: i32, kinds: &[(Kind, bool)]) -> Vec<Placed> {
    let px = |pt: f32| (pt * s).round() as i32;
    let x0 = (w - px(COL_PT)) / 2;
    let (small, medium, gap) = (px(SMALL_PT), px(COL_PT), px(GAP_PT));
    let mut out = Vec::new();
    let mut y = px(SEARCH_TOP_PT + SEARCH_H_PT + GAP_PT);
    let mut left_open = false;
    let mut section = true;
    for &(kind, on) in kinds {
        if section != on {
            section = on;
            if left_open {
                y += small + gap;
                left_open = false;
            }
            y += gap;
        }
        if kind.medium() {
            if left_open {
                y += small + gap;
                left_open = false;
            }
            out.push(Placed { kind, rect: Rect { x: x0, y, w: medium, h: small }, on });
            y += small + gap;
        } else if left_open {
            out.push(Placed { kind, rect: Rect { x: x0 + medium - small, y, w: small, h: small }, on });
            y += small + gap;
            left_open = false;
        } else {
            out.push(Placed { kind, rect: Rect { x: x0, y, w: small, h: small }, on });
            left_open = true;
        }
    }
    out
}

/// Bottom edge of the laid-out tiles.
fn tiles_bottom(s: f32, placed: &[Placed]) -> i32 {
    placed.iter().map(|p| p.rect.y + p.rect.h).max().unwrap_or((((SEARCH_TOP_PT + SEARCH_H_PT) * s).round()) as i32)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Claim {
    Undecided,
    /// a vertical drag: scrolling or, in edit mode on a tile, reordering
    Vertical,
    /// the pager has it
    Horizontal,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    y: f32,
    claim: Claim,
    target: Option<Target>,
    /// the tile being dragged into a new place
    drag: Option<Kind>,
    /// a vertical drag on the Smart Stack pages it instead of scrolling
    pan: bool,
}

/// Everything the Today View needs from the shell to refresh its tiles.
pub struct Sources<'a> {
    pub info: &'a Info,
    /// the terminal's last lines, None while it is not running
    pub term: Option<Vec<String>>,
    pub unix: i64,
}

pub struct Today {
    s: f32,
    w: i32,
    h: i32,
    prefs: Prefs,
    order: Vec<Kind>,
    off: Vec<Kind>,
    editing: bool,
    tiles: HashMap<Kind, Image>,
    keys: HashMap<Kind, String>,
    search: Image,
    edit: Image,
    done: Image,
    minus: Image,
    plus: Image,
    frost: Option<Image>,
    scroll: Scroller,
    touch: Option<Touch>,
    gen: u32,
    played: Option<deildu::Played>,
    poster: Option<Image>,
    photos: Option<Image>,
    /// the thumbnail key of the picture the person chose for the Photos widget (`photos.pin`); None: a new one each minute
    pinned: Option<String>,
    mono: Font<'static>,
    deildu_dir: PathBuf,
    thumbs_dir: PathBuf,
    weather_dir: PathBuf,
    /// the Weather app's saved summary, None until it has run
    weather: Option<Summary>,
    last_refresh: f64,
    /// Smart Stack: the shown member and the finger's vertical pull (px)
    stack: usize,
    stack_dy: f32,
}

impl Today {
    /// `mono` is the terminal font's data (SFUIMono.ttf).
    pub fn new(ui: &Ui, w: i32, h: i32, mono: &'static [u8]) -> Self {
        let badge = |t: &str| ui.text(t, 20.0, Weight::Semibold, WHITE);
        Today {
            s: ui.s,
            w,
            h,
            prefs: Prefs::memory(),
            order: Kind::ALL.to_vec(),
            off: Vec::new(),
            editing: false,
            tiles: HashMap::new(),
            keys: HashMap::new(),
            search: ui.text("Search", 17.0, Weight::Regular, rgba(235, 235, 245, 150)),
            edit: ui.text("Edit", 17.0, Weight::Semibold, WHITE),
            done: ui.text("Done", 17.0, Weight::Semibold, WHITE),
            minus: badge("\u{2212}"),
            plus: badge("+"),
            frost: None,
            scroll: Scroller::new(),
            touch: None,
            gen: 1,
            played: None,
            poster: None,
            photos: None,
            pinned: None,
            mono: Font::parse(mono).expect("mono font"),
            deildu_dir: crate::deildu::data_dir(),
            thumbs_dir: photos::thumbs_dir().into(),
            weather_dir: super::weather::data_path().into(),
            weather: None,
            last_refresh: f64::NEG_INFINITY,
            stack: 0,
            stack_dy: 0.0,
        }
    }

    /// Reads the widget order, the switched-off set and the last played
    /// title from `<dir>/today.ini`; saves there from now on.
    pub fn load(&mut self, dir: &str) {
        self.prefs = Prefs::load(format!("{dir}/{PREFS_FILE}"));
        self.weather_dir = PathBuf::from(dir).join("appdata/is.olibuijr.weather");
        self.read_prefs();
    }

    /// Loads from an in-memory store (tests, previews).
    pub fn load_prefs(&mut self, prefs: Prefs) {
        self.prefs = prefs;
        self.read_prefs();
    }

    fn read_prefs(&mut self) {
        let list = |v: Option<&str>| -> Vec<Kind> { v.unwrap_or("").split(',').filter_map(Kind::from_key).collect() };
        let mut order = list(self.prefs.get("order"));
        for k in Kind::ALL {
            if !order.contains(&k) {
                order.push(k);
            }
        }
        order.dedup();
        self.order = order;
        self.off = list(self.prefs.get("off"));
        self.off.dedup();
        self.played = self.prefs.get("deildu.last").and_then(deildu::Played::decode);
        self.pinned = self.prefs.get("photos.pin").filter(|k| !k.is_empty()).map(str::to_string);
        self.gen += 1;
    }

    /// Pins the Photos widget to the picture whose thumbnail key is `key`, or lets it rotate again with `None`. Saved.
    pub fn pin_photo(&mut self, key: Option<&str>) {
        self.pinned = key.filter(|k| !k.is_empty()).map(str::to_string);
        if let Err(e) = self.prefs.set("photos.pin", self.pinned.as_deref().unwrap_or("")) {
            eprintln!("springboard: {PREFS_FILE}: {e}");
        }
        self.keys.remove(&Kind::Photos);
        self.last_refresh = f64::NEG_INFINITY;
    }

    /// The thumbnail key the Photos widget is pinned to.
    pub fn pinned_photo(&self) -> Option<&str> {
        self.pinned.as_deref()
    }

    fn save(&mut self) {
        let join = |v: &[Kind]| v.iter().map(|k| k.key()).collect::<Vec<_>>().join(",");
        let (order, off) = (join(&self.order), join(&self.off));
        for (k, v) in [("order", order), ("off", off)] {
            if let Err(e) = self.prefs.set(k, &v) {
                eprintln!("springboard: {PREFS_FILE}: {e}");
            }
        }
    }

    /// Where Deildu keeps its artwork cache and the Photos app its thumbnails.
    pub fn set_dirs(&mut self, deildu: impl Into<PathBuf>, thumbs: impl Into<PathBuf>) {
        self.deildu_dir = deildu.into();
        self.thumbs_dir = thumbs.into();
        self.keys.clear();
    }

    /// Frees the frosted backdrop (rebuilt by `set_wallpaper` when the page is next shown).
    pub fn drop_frost(&mut self) {
        self.frost = None;
    }

    /// Rebuilds the frosted backdrop from the wallpaper (centred like the home screen).
    pub fn set_wallpaper(&mut self, wp: View) {
        let (w, h) = (self.w as usize, self.h as usize);
        let sharp = wp.crop(wp.w.saturating_sub(w) / 2, wp.h.saturating_sub(h) / 2, w, h);
        self.frost = Some(Frost::new(sharp, (BLUR_PT * self.s / 4.0) as usize, 120).frosted);
        self.gen += 1;
    }

    /// Remembers what Deildu last played (persisted); true if it changed.
    pub fn note_played(&mut self, p: &deildu::Played) -> bool {
        if self.played.as_ref() == Some(p) {
            return false;
        }
        if let Err(e) = self.prefs.set("deildu.last", &p.encode()) {
            eprintln!("springboard: {PREFS_FILE}: {e}");
        }
        self.played = Some(p.clone());
        true
    }

    /// The Weather app's saved summary (the first place), refreshed with the tiles; None until the app has run.
    pub fn weather(&self) -> Option<&Summary> {
        self.weather.as_ref()
    }

    pub fn played(&self) -> Option<&deildu::Played> {
        self.played.as_ref()
    }

    /// Bumped whenever the page looks different (tiles, order, scroll, edit mode).
    pub fn gen(&self) -> u32 {
        self.gen
    }

    /// The rendered tile of a widget (None until the first refresh), for the home-screen widgets.
    pub fn tile(&self, kind: Kind) -> Option<&Image> {
        self.tiles.get(&kind)
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    pub fn order(&self) -> &[Kind] {
        &self.order
    }

    pub fn off(&self) -> &[Kind] {
        &self.off
    }

    /// The widgets shown live, in order.
    pub fn enabled(&self) -> Vec<Kind> {
        self.order.iter().copied().filter(|k| !self.off.contains(k) && self.available(*k)).collect()
    }

    /// Photos has nothing to show without the thumbnail cache.
    fn available(&self, k: Kind) -> bool {
        (k != Kind::Photos || self.photos.is_some()) && (k != Kind::Weather || self.weather.is_some())
    }

    /// Whether a refresh is due: once a minute.
    pub fn due(&self, t: f64) -> bool {
        t - self.last_refresh >= REFRESH_MS
    }

    /// Re-renders every tile whose data changed; true if any did.
    pub fn refresh(&mut self, ui: &Ui, src: &Sources, t: f64) -> bool {
        self.last_refresh = t;
        let before = self.weather.clone();
        self.weather = Summary::load(&self.weather_dir);
        let mut changed = before != self.weather;
        let weather_now = self.weather.clone();
        let mut update = |kind: Kind, key: String, tiles: &mut HashMap<Kind, Image>, keys: &mut HashMap<Kind, String>, render: &mut dyn FnMut() -> Option<Image>| {
            if keys.get(&kind) == Some(&key) {
                return;
            }
            match render() {
                Some(img) => {
                    tiles.insert(kind, img);
                }
                None => {
                    tiles.remove(&kind);
                }
            }
            keys.insert(kind, key);
            changed = true;
        };
        let (tiles, keys) = (&mut self.tiles, &mut self.keys);
        update(Kind::Clock, clock::key(src.unix), tiles, keys, &mut || Some(clock::render(ui, src.unix)));
        update(Kind::Battery, battery::key(src.info.battery, src.info.charging), tiles, keys, &mut || (src.info.desktop_network.is_none() || src.info.battery_present).then(|| battery::render(ui, src.info.battery, src.info.charging)));
        update(Kind::Wifi, wifi::key(src.info), tiles, keys, &mut || Some(wifi::render(ui, src.info)));
        update(Kind::Calendar, calendar::key(src.info), tiles, keys, &mut || Some(calendar::render(ui, src.info)));
        update(Kind::Weather, weather::key(weather_now.as_ref()), tiles, keys, &mut || weather_now.as_ref().map(|s| weather::render(ui, s)));
        let mono = &self.mono;
        update(Kind::Terminal, terminal::key(src.term.as_deref()), tiles, keys, &mut || Some(terminal::render(ui, mono, src.term.as_deref())));
        // the poster is looked up again whenever the title changes or the art was missing
        let played = self.played.clone();
        let poster_key = format!("poster:{}", played.as_ref().map(|p| p.id.as_str()).unwrap_or(""));
        if self.keys.get(&Kind::Deildu).is_none_or(|k| !k.starts_with(&poster_key)) || (played.is_some() && self.poster.is_none()) {
            self.poster = played.as_ref().and_then(|p| deildu::poster(ui, &self.deildu_dir, &p.id));
        }
        let (poster, dkey) = (&self.poster, format!("{poster_key}:{}", deildu::key(played.as_ref(), self.poster.is_some())));
        update(Kind::Deildu, dkey, &mut self.tiles, &mut self.keys, &mut || Some(deildu::render(ui, played.as_ref(), poster.as_ref())));
        let thumbs = photos::thumbs(&self.thumbs_dir);
        // the picture the person chose, while its thumbnail exists; else a new one every minute
        let pinned = self.pinned.as_deref().and_then(|k| photos::pinned(&self.thumbs_dir, k));
        let pick = pinned.or_else(|| photos::pick(&thumbs, src.unix).cloned());
        let s = self.s;
        let mut img = None;
        update(Kind::Photos, photos::key(pick.as_deref()), &mut self.tiles, &mut self.keys, &mut || {
            img = pick.as_deref().and_then(|p| photos::render(s, p));
            img.clone()
        });
        if self.keys.get(&Kind::Photos) == Some(&photos::key(pick.as_deref())) && changed {
            self.photos = self.tiles.get(&Kind::Photos).cloned();
        }
        if changed {
            self.gen += 1;
            self.fit_scroll();
        }
        changed
    }

    /// The tiles on the page: the live ones, then (in edit mode) the switched-off ones.
    pub fn placed(&self) -> Vec<Placed> {
        let mut kinds: Vec<(Kind, bool)> = self.enabled().into_iter().map(|k| (k, true)).collect();
        if self.editing {
            kinds.extend(self.order.iter().copied().filter(|k| self.off.contains(k) && self.available(*k)).map(|k| (k, false)));
        }
        layout(self.s, self.w, &kinds)
    }

    fn px(&self, pt: f32) -> f32 {
        pt * self.s
    }

    /// The search bar, in page coordinates.
    pub fn search_rect(&self) -> Rect {
        let w = self.px(COL_PT).round() as i32;
        Rect { x: (self.w - w) / 2, y: self.px(SEARCH_TOP_PT).round() as i32, w, h: self.px(SEARCH_H_PT).round() as i32 }
    }

    /// The Edit/Done button under the tiles, in page coordinates.
    pub fn edit_rect(&self) -> Rect {
        let placed = self.placed();
        let (w, h) = (self.px(EDIT_W_PT).round() as i32, self.px(EDIT_H_PT).round() as i32);
        Rect { x: (self.w - w) / 2, y: tiles_bottom(self.s, &placed) + self.px(EDIT_GAP_PT).round() as i32, w, h }
    }

    /// Height of everything on the page.
    pub fn content_h(&self) -> i32 {
        let e = self.edit_rect();
        e.y + e.h + self.px(BOTTOM_PAD_PT).round() as i32
    }

    /// The rows the page may draw in: below the status bar, above the dots.
    pub fn viewport(&self, l: &Layout) -> (i32, i32) {
        (l.status_h, l.dots_y - l.dot - l.pt(10.0) as i32)
    }

    fn fit_scroll(&mut self) {
        let l = Layout::new(self.w, self.h, 0);
        let (top, bottom) = self.viewport(&l);
        self.scroll.set_max(self.content_h() as f32 - (bottom - top) as f32);
    }

    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset()
    }

    /// The target under (x, y) on screen, honouring the scroll position.
    pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
        let sy = y + self.scroll.offset();
        if self.search_rect().contains(x, sy) {
            return Some(Target::Search);
        }
        if self.edit_rect().contains(x, sy) {
            return Some(Target::Edit);
        }
        let badge = self.px(BADGE_PT);
        for (i, p) in self.placed().iter().enumerate() {
            let (bx, by) = badge_centre(p.rect, badge);
            if self.editing && (x - bx).abs() <= badge * 0.75 && (sy - by).abs() <= badge * 0.75 {
                return Some(Target::Badge(i));
            }
            if p.rect.contains(x, sy) {
                return Some(Target::Widget(i));
            }
        }
        None
    }

    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.scroll.down(y, t);
        self.touch = Some(Touch { x0: x, y0: y, y, claim: Claim::Undecided, target: self.hit(x, y), drag: None, pan: false });
    }

    /// True while this page owns the current touch (it has not turned out horizontal).
    pub fn owns(&self) -> bool {
        self.touch.is_some_and(|t| t.claim != Claim::Horizontal)
    }

    /// Follows the finger; returns true the moment the page claims the touch
    /// as vertical (the caller then stops feeding the pager).
    pub fn moved(&mut self, x: f32, y: f32, t: f64) -> bool {
        let Some(mut tc) = self.touch else { return false };
        let mut claimed = false;
        if tc.claim == Claim::Undecided {
            let (dx, dy) = (x - tc.x0, y - tc.y0);
            if dx.abs() > SLOP || dy.abs() > SLOP {
                tc.claim = if dy.abs() >= dx.abs() { Claim::Vertical } else { Claim::Horizontal };
                claimed = tc.claim == Claim::Vertical;
                if claimed {
                    if let (false, Some(Target::Widget(i))) = (self.editing, tc.target) {
                        tc.pan = self.placed().get(i).is_some_and(|p| p.kind == Kind::Stack);
                    }
                    if let (true, Some(Target::Widget(i))) = (self.editing, tc.target) {
                        if let Some(p) = self.placed().get(i).filter(|p| p.on) {
                            tc.drag = Some(p.kind);
                        }
                    }
                }
            }
        }
        tc.y = y;
        if tc.claim == Claim::Vertical {
            if tc.pan {
                self.stack_dy = y - tc.y0;
            }
            match tc.drag {
                _ if tc.pan => {}
                Some(kind) => self.reorder(kind, x, y),
                None => self.scroll.moved(y, t),
            }
            self.gen += 1;
        }
        self.touch = Some(tc);
        claimed
    }

    /// Moves `kind` into the slot of whichever live tile the finger is over.
    fn reorder(&mut self, kind: Kind, x: f32, y: f32) {
        let sy = y + self.scroll.offset();
        let placed = self.placed();
        let Some(over) = placed.iter().find(|p| p.on && p.kind != kind && p.rect.contains(x, sy)) else { return };
        let (from, to) = (self.order.iter().position(|&k| k == kind), self.order.iter().position(|&k| k == over.kind));
        if let (Some(from), Some(to)) = (from, to) {
            let k = self.order.remove(from);
            self.order.insert(to, k);
        }
    }

    /// Ends the touch: a tap acts on its target, a drag settles.
    pub fn up(&mut self, x: f32, y: f32, t: f64) -> Option<Action> {
        let tc = self.touch.take()?;
        self.gen += 1;
        match tc.claim {
            Claim::Horizontal => {
                self.scroll.up(tc.y0, t);
                None
            }
            Claim::Vertical => {
                if tc.pan {
                    self.scroll.up(tc.y0, t);
                    self.page_stack(y - tc.y0);
                } else if tc.drag.is_some() {
                    self.scroll.up(tc.y0, t);
                    self.save();
                } else {
                    self.scroll.up(y, t);
                }
                None
            }
            Claim::Undecided => {
                self.scroll.up(tc.y0, t);
                let target = tc.target.filter(|&tg| self.hit(x, y) == Some(tg))?;
                self.tap(target)
            }
        }
    }

    fn tap(&mut self, target: Target) -> Option<Action> {
        match target {
            Target::Search => Some(Action::OpenQuickSearch),
            Target::Edit => {
                self.editing = !self.editing;
                self.fit_scroll();
                None
            }
            Target::Badge(i) => {
                let p = *self.placed().get(i)?;
                if p.on {
                    self.off.push(p.kind);
                } else {
                    self.off.retain(|&k| k != p.kind);
                }
                self.save();
                self.fit_scroll();
                None
            }
            Target::Widget(i) if !self.editing => {
                let p = *self.placed().get(i)?;
                match p.kind {
                    Kind::Deildu => self.played.clone().map(Action::OpenDeildu),
                    Kind::Calendar => Some(Action::OpenCalendar),
                    Kind::Weather => Some(Action::OpenWeather),
                    Kind::Photos => Some(Action::ChoosePhoto),
                    _ => None,
                }
            }
            Target::Widget(_) => None,
        }
    }

    /// Smart Stack members: the small widgets that have a tile, in order.
    pub fn stack_members(&self) -> Vec<Kind> {
        self.enabled().into_iter().filter(|k| !k.medium() && *k != Kind::Stack && self.tiles.contains_key(k)).collect()
    }

    /// Releases a vertical pull of `dy` px: past a quarter of the tile it turns the page
    /// (up shows the next widget, down the previous), wrapping round.
    fn page_stack(&mut self, dy: f32) {
        let n = self.stack_members().len();
        self.stack_dy = 0.0;
        if n > 1 && dy.abs() > self.px(widgets::SMALL_PT) / 4.0 {
            self.stack = if dy < 0.0 { (self.stack + 1) % n } else { (self.stack + n - 1) % n };
        }
    }

    /// The tile being dragged, if any.
    fn dragging(&self) -> Option<(Kind, f32)> {
        self.touch.and_then(|t| t.drag.map(|k| (k, t.y - t.y0)))
    }

    /// True while the column is still coasting.
    pub fn animating(&self) -> bool {
        self.scroll.animating()
    }

    /// Advances the scroll inertia; true while it still moves.
    pub fn tick(&mut self, t: f64) -> bool {
        let moving = self.scroll.tick(t);
        if moving {
            self.gen += 1;
        }
        moving
    }

    /// The screen turned: lay the page out for `w` x `h`. The tiles are rebuilt at the next
    /// refresh.
    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.tiles.clear();
        self.keys.clear();
        self.frost = None;
        self.touch = None;
        self.reset();
        self.last_refresh = f64::NEG_INFINITY;
    }

    /// Drops any touch (screen off, lock) and leaves edit mode.
    pub fn reset(&mut self) {
        self.touch = None;
        self.editing = false;
        self.scroll.jump_top();
        self.gen += 1;
    }

    /// Draws the page for pager `offset` (0 = hidden, -w = fully in): the
    /// frost fades with the progress and the content slides with the finger.
    pub fn draw(&self, s: &mut Surface, l: &Layout, offset: f32) {
        let w = self.w as f32;
        let p = (-offset / w).clamp(0.0, 1.0);
        if p <= 0.0 {
            return;
        }
        if let Some(f) = &self.frost {
            s.blit(f, 0, 0, 255, (p * 255.0) as u32);
        }
        let dx = -(w + offset).round() as i32;
        let (top, bottom) = self.viewport(l);
        let mut band = s.rows(top as usize, bottom as usize);
        let scroll = self.scroll.offset().round() as i32;
        let at = |r: Rect| Rect { x: r.x + dx, y: r.y - scroll, w: r.w, h: r.h };
        let sr = at(self.search_rect());
        band.fill_rrect(sr.x as f32, sr.y as f32, sr.w as f32, sr.h as f32, self.px(10.0), rgba(255, 255, 255, 40));
        draw_magnifier(&mut band, sr.x as f32 + self.px(12.0), sr.y as f32 + sr.h as f32 / 2.0, self.s, rgba(235, 235, 245, 150));
        band.blit(&self.search, sr.x + self.px(36.0) as i32, sr.y + (sr.h - self.search.h as i32) / 2, 255, 255);
        let drag = self.dragging();
        let badge = self.px(BADGE_PT);
        let mut lifted = None;
        for p in self.placed() {
            let r = at(p.rect);
            if drag.is_some_and(|(k, _)| k == p.kind) {
                lifted = Some((p, r));
                continue;
            }
            self.draw_tile(&mut band, &p, r, badge, 255);
        }
        if let (Some((p, r)), Some((_, dy))) = (lifted, drag) {
            let r = Rect { y: r.y + dy.round() as i32, ..r };
            band.fill_rrect(r.x as f32, r.y as f32 + self.px(6.0), r.w as f32, r.h as f32, self.px(widgets::RADIUS_PT), rgba(0, 0, 0, 90));
            self.draw_tile(&mut band, &p, r, badge, 235);
        }
        let er = at(self.edit_rect());
        band.fill_rrect(er.x as f32, er.y as f32, er.w as f32, er.h as f32, er.h as f32 / 2.0, rgba(255, 255, 255, 46));
        let label = if self.editing { &self.done } else { &self.edit };
        band.blit(label, er.x + (er.w - label.w as i32) / 2, er.y + (er.h - label.h as i32) / 2, 255, 255);
    }

    fn draw_tile(&self, s: &mut Surface, p: &Placed, r: Rect, badge: f32, alpha: u32) {
        let alpha = if p.on { alpha } else { alpha * 140 / 255 };
        if p.kind == Kind::Stack {
            self.draw_stack(s, r, alpha);
        } else {
            self.draw_plain(s, p, r, alpha);
        }
        if self.editing {
            let (bx, by) = badge_centre(r, badge);
            let (color, glyph) = if p.on { (battery::RED, &self.minus) } else { (battery::GREEN, &self.plus) };
            s.fill_rrect(bx - badge / 2.0, by - badge / 2.0, badge, badge, badge / 2.0, color);
            s.blit(glyph, (bx - glyph.w as f32 / 2.0).round() as i32, (by - glyph.h as f32 / 2.0).round() as i32, 255, 255);
        }
    }

    /// The shown member (and its neighbour sliding in) clipped to the stack, plus the page dots.
    fn draw_stack(&self, s: &mut Surface, r: Rect, alpha: u32) {
        let members = self.stack_members();
        let n = members.len();
        s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, self.px(widgets::RADIUS_PT), crate::gfx::scale(widgets::background(), alpha));
        if n == 0 {
            return;
        }
        let cur = self.stack.min(n - 1);
        let dy = self.stack_dy.round() as i32;
        let mut clip = s.rows(r.y.max(0) as usize, (r.y + r.h).max(0) as usize);
        let neighbour = if dy < 0 { (cur + 1) % n } else { (cur + n - 1) % n };
        if n > 1 && dy != 0 {
            let off = if dy < 0 { r.h } else { -r.h };
            if let Some(img) = self.tiles.get(&members[neighbour]) {
                clip.blit(img, r.x, r.y + off + dy, 255, alpha);
            }
        }
        if let Some(img) = self.tiles.get(&members[cur]) {
            clip.blit(img, r.x, r.y + dy, 255, alpha);
        }
        if n > 1 {
            let (d, gap) = (self.px(5.0), self.px(7.0));
            let total = n as f32 * d + (n - 1) as f32 * gap;
            let x = (r.x + r.w) as f32 - self.px(10.0) - d;
            let y0 = r.y as f32 + (r.h as f32 - total) / 2.0;
            for i in 0..n {
                let a = if i == cur { 235 } else { 90 };
                s.fill_rrect(x, y0 + i as f32 * (d + gap), d, d, d / 2.0, rgba(255, 255, 255, (a * alpha as u32 / 255) as u8));
            }
        }
    }

    fn draw_plain(&self, s: &mut Surface, p: &Placed, r: Rect, alpha: u32) {
        match self.tiles.get(&p.kind) {
            Some(img) => s.blit(img, r.x, r.y, 255, alpha),
            None => s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, self.px(widgets::RADIUS_PT), crate::gfx::scale(widgets::background(), alpha)),
        }
    }
}

/// The badge sits over the tile's top-left corner.
fn badge_centre(r: Rect, badge: f32) -> (f32, f32) {
    (r.x as f32 + badge * 0.35, r.y as f32 + badge * 0.35)
}

/// A stroked circle with a handle, `cx` its left edge.
pub(super) fn draw_magnifier(s: &mut Surface, x: f32, mid: f32, scale: f32, color: u32) {
    let r = 6.5 * scale;
    let c = (x + r, mid - 1.0 * scale);
    let shape = super::glyphs::either(super::glyphs::ring(c, r, 1.8 * scale), super::glyphs::segment((c.0 + r * 0.7, c.1 + r * 0.7), (c.0 + r * 1.45, c.1 + r * 1.45), 2.0 * scale));
    super::glyphs::paint(s, (x - 2.0, c.1 - r - 2.0, 2.0 * r * 1.6 + 4.0, 2.0 * r * 1.6 + 4.0), color, shape);
}

/// The Today View's state as a preferences line for remote inspection.
pub fn describe(t: &Today) -> String {
    format!("today_editing={}\ntoday_order={}\ntoday_off={}\n", t.editing, t.order.iter().map(|k| k.key()).collect::<Vec<_>>().join(","), t.off.iter().map(|k| k.key()).collect::<Vec<_>>().join(","))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::gfx::pack;

    const W: i32 = 1200;
    const H: i32 = 1920;

    pub(crate) fn today() -> (Ui, Today) {
        let ui = crate::shell::tests::ui();
        let mono: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice());
        let mut t = Today::new(&ui, W, H, mono);
        let missing = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-today-missing");
        t.set_dirs(missing, missing);
        (ui, t)
    }

    fn info() -> Info {
        Info { hour: 9, min: 41, wday: 0, mday: 4, mon: 9, battery: 80, online: true, wifi: "Home".into(), ip: "10.0.0.2".into(), wifi_bars: 3, wifi_on: true, ..Info::default() }
    }

    fn refresh(ui: &Ui, t: &mut Today, info: &Info, unix: i64, at: f64) -> bool {
        t.refresh(ui, &Sources { info, term: None, unix }, at)
    }

    #[test]
    fn layout_pairs_small_tiles_and_gives_medium_ones_a_row() {
        let s = 2.0;
        let p = layout(s, W, &[(Kind::Clock, true), (Kind::Battery, true), (Kind::Wifi, true), (Kind::Calendar, true), (Kind::Deildu, true), (Kind::Photos, true)]);
        assert_eq!(p.len(), 6);
        let col_x = (W - (COL_PT * s) as i32) / 2;
        assert_eq!(p[0].rect, Rect { x: col_x, y: ((SEARCH_TOP_PT + SEARCH_H_PT + GAP_PT) * s) as i32, w: (COL_PT * s) as i32, h: (SMALL_PT * s) as i32 });
        assert_eq!(p[1].rect.y, p[0].rect.y + p[0].rect.h + (GAP_PT * s) as i32);
        assert_eq!((p[1].rect.x, p[1].rect.w), (col_x, (SMALL_PT * s) as i32));
        assert_eq!(p[2].rect.y, p[1].rect.y, "two smalls share a row");
        assert_eq!(p[2].rect.x, col_x + (COL_PT * s) as i32 - (SMALL_PT * s) as i32);
        assert_eq!(p[2].rect.x - (p[1].rect.x + p[1].rect.w), (GAP_X_PT * s) as i32);
        assert_eq!(p[3].rect.x, col_x, "a third small starts a new row");
        assert_eq!(p[4].rect.y, p[3].rect.y + p[3].rect.h + (GAP_PT * s) as i32, "a medium after a lone small moves down a row");
        assert_eq!(p[5].rect.y, p[4].rect.y + p[4].rect.h + (GAP_PT * s) as i32);
        // the off section follows after a gap, even mid-row
        let q = layout(s, W, &[(Kind::Battery, true), (Kind::Wifi, false), (Kind::Calendar, false)]);
        assert_eq!(q[1].rect.y, q[0].rect.y + q[0].rect.h + 2 * (GAP_PT * s) as i32);
        assert_eq!(q[2].rect.y, q[1].rect.y);
        assert!(!q[1].on && q[0].on);
        assert!(layout(s, W, &[]).is_empty());
    }

    #[test]
    fn prefs_round_trip_order_off_and_last_played() {
        let (_, mut t) = today();
        let mut prefs = Prefs::memory();
        prefs.set("order", "wifi,clock,bogus,clock").unwrap();
        prefs.set("off", "terminal,bogus").unwrap();
        prefs.set("deildu.last", "9|https://d/9|Film 2024").unwrap();
        t.load_prefs(prefs);
        assert_eq!(t.order()[..2], [Kind::Wifi, Kind::Clock]);
        assert_eq!(t.order().len(), Kind::ALL.len(), "unknown keys dropped, missing ones appended");
        assert_eq!(t.off(), [Kind::Terminal]);
        assert_eq!(t.played().map(|p| p.id.as_str()), Some("9"));
        assert!(!t.enabled().contains(&Kind::Terminal));
        assert!(!t.enabled().contains(&Kind::Photos), "no thumbnail cache, no Photos tile");
        let p = deildu::Played { id: "10".into(), title: "Other".into(), url: String::new() };
        assert!(t.note_played(&p));
        assert!(!t.note_played(&p), "same title again changes nothing");
        assert_eq!(t.prefs.get("deildu.last"), Some("10||Other"));
    }

    #[test]
    fn refresh_renders_once_per_change_and_is_due_every_minute() {
        let (ui, mut t) = today();
        let info = info();
        assert!(t.due(0.0));
        assert!(refresh(&ui, &mut t, &info, 1_000_020, 0.0), "first refresh renders everything");
        let g = t.gen();
        assert!(!t.due(30_000.0) && t.due(60_000.0));
        assert!(!refresh(&ui, &mut t, &info, 1_000_050, 30_000.0), "same minute, same info: nothing to do");
        assert_eq!(t.gen(), g);
        assert!(refresh(&ui, &mut t, &info, 1_000_080, 60_000.0), "a new minute moves the clock hands");
        let mut low = info.clone();
        low.battery = 20;
        assert!(refresh(&ui, &mut t, &low, 1_000_080, 61_000.0), "a battery change re-renders");
        assert!(t.tiles.contains_key(&Kind::Battery) && t.tiles.contains_key(&Kind::Clock));
        assert!(!t.tiles.contains_key(&Kind::Photos));
        let lines = Some(vec!["$ ls".to_string()]);
        assert!(t.refresh(&ui, &Sources { info: &low, term: lines.clone(), unix: 1_000_080 }, 62_000.0), "terminal output re-renders");
        assert!(!t.refresh(&ui, &Sources { info: &low, term: lines, unix: 1_000_080 }, 63_000.0));
    }

    #[test]
    fn photos_tile_appears_once_the_cache_exists() {
        let (ui, mut t) = today();
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-today-thumbs")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        t.set_dirs(&dir, &dir);
        refresh(&ui, &mut t, &info(), 0, 0.0);
        assert!(!t.enabled().contains(&Kind::Photos));
        std::fs::create_dir_all(&dir).unwrap();
        let mut b = b"RIMG".to_vec();
        b.extend(2u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        for _ in 0..4 {
            b.extend(pack(200, 100, 50, 255).to_le_bytes());
        }
        std::fs::write(dir.join("p.raw"), b).unwrap();
        assert!(refresh(&ui, &mut t, &info(), 60, 60_000.0));
        assert!(t.enabled().contains(&Kind::Photos));
        assert_eq!(t.placed().last().map(|p| p.kind), Some(Kind::Photos));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_photos_widget_asks_for_a_picture_and_shows_the_pinned_one() {
        let (ui, mut t) = today();
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-today-pin")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let raw = |color: u32| {
            let mut b = b"RIMG".to_vec();
            b.extend(2u32.to_le_bytes());
            b.extend(2u32.to_le_bytes());
            for _ in 0..4 {
                b.extend(color.to_le_bytes());
            }
            b
        };
        std::fs::write(dir.join("aaaa.raw"), raw(pack(200, 100, 50, 255))).unwrap();
        std::fs::write(dir.join("bbbb.raw"), raw(pack(10, 20, 30, 255))).unwrap();
        t.set_dirs(&dir, &dir);
        refresh(&ui, &mut t, &info(), 0, 0.0);
        let photos = t.placed().into_iter().find(|p| p.kind == Kind::Photos).expect("the Photos widget is on the page");
        let (px, py) = centre(photos.rect);
        assert_eq!(tap(&mut t, px, py, 100.0), Some(Action::ChoosePhoto), "a tap opens the picker");
        // the person chose the second picture: the widget shows it, every minute
        t.pin_photo(Some("bbbb"));
        assert_eq!(t.pinned_photo(), Some("bbbb"));
        assert_eq!(t.prefs.get("photos.pin"), Some("bbbb"), "the choice is saved");
        for (i, minute) in [0i64, 1, 2, 3].into_iter().enumerate() {
            refresh(&ui, &mut t, &info(), minute * 60, 120_000.0 + i as f64 * 60_000.0);
            let tile = t.tile(Kind::Photos).expect("the tile is drawn");
            assert_eq!(tile.px[tile.w * tile.h / 2 + tile.w / 2], pack(10, 20, 30, 255), "minute {minute}: still the pinned picture");
        }
        // a pinned picture whose thumbnail is gone falls back to the rotation
        std::fs::remove_file(dir.join("bbbb.raw")).unwrap();
        refresh(&ui, &mut t, &info(), 600, 900_000.0);
        let tile = t.tile(Kind::Photos).expect("the tile is drawn");
        assert_eq!(tile.px[tile.w * tile.h / 2 + tile.w / 2], pack(200, 100, 50, 255));
        t.pin_photo(None);
        assert_eq!(t.pinned_photo(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0)
    }

    fn tap(t: &mut Today, x: f32, y: f32, at: f64) -> Option<Action> {
        t.down(x, y, at);
        t.up(x, y, at + 30.0)
    }

    #[test]
    fn taps_hit_search_tiles_and_edit() {
        let (ui, mut t) = today();
        refresh(&ui, &mut t, &info(), 0, 0.0);
        let sr = t.search_rect();
        let (sx, sy) = centre(sr);
        assert_eq!(t.hit(sx, sy), Some(Target::Search));
        assert_eq!(tap(&mut t, sx, sy, 100.0), Some(Action::OpenQuickSearch));
        let placed = t.placed();
        assert_eq!(placed.len(), 7, "all but Photos, plus the Smart Stack");
        let (cx, cy) = centre(placed[0].rect);
        assert_eq!(t.hit(cx, cy), Some(Target::Widget(0)));
        assert_eq!(tap(&mut t, cx, cy, 200.0), None, "the clock tile does nothing");
        let deildu = placed.iter().position(|p| p.kind == Kind::Deildu).unwrap();
        let (dx, dy) = centre(placed[deildu].rect);
        assert_eq!(tap(&mut t, dx, dy, 300.0), None, "nothing played yet");
        t.note_played(&deildu::Played { id: "5".into(), title: "T".into(), url: "u".into() });
        assert!(matches!(tap(&mut t, dx, dy, 400.0), Some(Action::OpenDeildu(p)) if p.id == "5"));
        let calendar = placed.iter().find(|p| p.kind == Kind::Calendar).unwrap();
        let (cx, cy) = centre(calendar.rect);
        assert_eq!(tap(&mut t, cx, cy, 450.0), Some(Action::OpenCalendar));
        let (ex, ey) = centre(t.edit_rect());
        assert_eq!(t.hit(ex, ey), Some(Target::Edit));
        assert_eq!(t.hit(5.0, 5.0), None);
        // a press that moves away is not a tap
        t.down(sx, sy, 500.0);
        assert!(!t.moved(sx + 2.0 * SLOP, sy, 510.0), "a sideways drag is the pager's");
        assert!(!t.owns());
        assert_eq!(t.up(sx + 2.0 * SLOP, sy, 520.0), None);
    }

    #[test]
    fn edit_mode_toggles_tiles_reorders_by_drag_and_persists() {
        let (ui, mut t) = today();
        refresh(&ui, &mut t, &info(), 0, 0.0);
        let (ex, ey) = centre(t.edit_rect());
        assert_eq!(tap(&mut t, ex, ey, 100.0), None);
        assert!(t.editing());
        let before = t.placed();
        // switch the battery off: it moves to the off section at the bottom
        let bi = before.iter().position(|p| p.kind == Kind::Battery).unwrap();
        let badge = t.px(BADGE_PT);
        let (bx, by) = badge_centre(before[bi].rect, badge);
        assert_eq!(t.hit(bx, by), Some(Target::Badge(bi)));
        assert_eq!(tap(&mut t, bx, by, 200.0), None);
        assert_eq!(t.off(), [Kind::Battery]);
        let after = t.placed();
        let off = after.iter().find(|p| p.kind == Kind::Battery).unwrap();
        assert!(!off.on && off.rect.y > after.iter().filter(|p| p.on).map(|p| p.rect.y).max().unwrap());
        assert_eq!(t.prefs.get("off"), Some("battery"));
        // and back on
        let (bx, by) = badge_centre(off.rect, badge);
        tap(&mut t, bx, by, 300.0);
        assert!(t.off().is_empty());
        // drag the clock (first row) onto the deildu tile (third row)
        let placed = t.placed();
        let clock = placed.iter().find(|p| p.kind == Kind::Clock).unwrap().rect;
        let deildu = placed.iter().find(|p| p.kind == Kind::Deildu).unwrap().rect;
        let (cx, cy) = centre(clock);
        let (_, dy) = centre(deildu);
        t.down(cx, cy, 400.0);
        assert!(t.moved(cx, cy + 2.0 * SLOP, 410.0), "a vertical drag is ours");
        assert!(t.owns());
        t.moved(cx, dy, 450.0);
        assert_eq!(t.up(cx, dy, 500.0), None);
        let order = t.enabled();
        assert!(order.iter().position(|&k| k == Kind::Clock) > order.iter().position(|&k| k == Kind::Deildu), "clock now follows deildu: {order:?}");
        assert_eq!(t.prefs.get("order"), Some(t.order().iter().map(|k| k.key()).collect::<Vec<_>>().join(",").as_str()));
        let now = t.placed();
        let (clock_now, deildu_now) = (now.iter().find(|p| p.kind == Kind::Clock).unwrap().rect, now.iter().find(|p| p.kind == Kind::Deildu).unwrap().rect);
        assert!(deildu_now.y < deildu.y && clock_now.y > deildu_now.y, "deildu moved up, the clock sits below it");
        // Done leaves edit mode; the stored order survives a reload
        let (ex, ey) = centre(t.edit_rect());
        tap(&mut t, ex, ey, 600.0);
        assert!(!t.editing());
        let saved = t.prefs.clone();
        let (_, mut again) = today();
        again.load_prefs(saved);
        assert_eq!(again.order(), t.order());
        // tapping a tile in edit mode never opens anything
        tap(&mut t, ex, ey, 700.0);
        t.note_played(&deildu::Played { id: "1".into(), title: "T".into(), url: String::new() });
        let r = t.placed().iter().find(|p| p.kind == Kind::Deildu).unwrap().rect;
        let (x, y) = centre(r);
        assert_eq!(tap(&mut t, x, y, 800.0), None);
    }

    #[test]
    fn vertical_drag_scrolls_when_the_page_is_tall() {
        let (ui, mut t) = today();
        refresh(&ui, &mut t, &info(), 0, 0.0);
        let l = Layout::new(W, H, 3);
        let (top, bottom) = t.viewport(&l);
        assert!(t.content_h() > bottom - top, "the default set with the Smart Stack is taller than the page");
        assert_eq!(t.scroll_offset(), 0.0);
        t.down(100.0, 900.0, 0.0);
        t.moved(100.0, 700.0, 100.0);
        assert!(t.scroll_offset() > 0.0, "the tall page follows the finger");
        t.up(100.0, 700.0, 200.0);
        t.tick(5000.0);
        let max = t.content_h() as f32 - (bottom - top) as f32;
        assert!(t.scroll_offset() > 0.0 && t.scroll_offset() <= max + 0.5, "settles inside the page");
        t.reset();
        assert_eq!(t.scroll_offset(), 0.0);
        // edit mode lists more and may need scrolling; the hit test follows the offset
        let (ex, ey) = centre(t.edit_rect());
        tap(&mut t, ex, ey, 300.0);
        t.scroll.set_max(300.0);
        // beside the column, so the drag scrolls instead of reordering a tile
        t.down(100.0, 900.0, 400.0);
        t.moved(100.0, 700.0, 500.0);
        t.up(100.0, 700.0, 600.0);
        t.tick(600.0);
        let off = t.scroll_offset();
        assert!(off > 0.0);
        let first = t.placed()[0].rect;
        assert_eq!(t.hit(first.x as f32 + 50.0, first.y as f32 + 50.0 - off), Some(Target::Widget(0)));
        t.reset();
        assert_eq!(t.scroll_offset(), 0.0);
        assert!(!t.editing());
    }

    #[test]
    fn draws_the_page_sliding_in_over_the_frost() {
        let (ui, mut t) = today();
        refresh(&ui, &mut t, &info(), 0, 0.0);
        let wp = Image { w: W as usize, h: H as usize, px: vec![pack(200, 40, 40, 255); (W * H) as usize] };
        t.set_wallpaper(wp.view());
        let l = Layout::new(W, H, 3);
        let mut buf = vec![pack(0, 0, 255, 255); (W * H) as usize];
        t.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 }, &l, 0.0);
        assert!(buf.iter().all(|&p| p == pack(0, 0, 255, 255)), "hidden page draws nothing");
        t.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 }, &l, -(W as f32));
        let sr = t.search_rect();
        let (sx, sy) = centre(sr);
        let bar = buf[sy as usize * W as usize + sx as usize];
        assert_ne!(bar, pack(0, 0, 255, 255), "search bar drawn");
        let r = t.placed()[0].rect;
        let tile = buf[(r.y + 10) as usize * W as usize + (r.x + r.w / 2) as usize];
        assert!(tile & 0xff < 60 && tile >> 24 == 255, "a tile sits over the frost: {tile:08x}");
        let above = buf[5 * W as usize + 5];
        assert!(above & 0xff > 40, "outside the viewport only the frost shows: {above:08x}");
        // halfway in: the content is shifted left by half a page
        let mut half = vec![pack(0, 0, 255, 255); (W * H) as usize];
        t.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut half, y0: 0 }, &l, -(W as f32) / 2.0);
        let shifted = half[(r.y + 10) as usize * W as usize + (r.x + r.w / 2 - W / 2) as usize];
        assert!(shifted & 0xff < 60, "tile slid left: {shifted:08x}");
        // edit mode shows badges, and a dragged tile is drawn lifted
        let (ex, ey) = centre(t.edit_rect());
        tap(&mut t, ex, ey, 100.0);
        let (bx, by) = badge_centre(r, t.px(BADGE_PT));
        t.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 }, &l, -(W as f32));
        let badge = t.px(BADGE_PT);
        assert_eq!(buf[by as usize * W as usize + (bx - badge * 0.4) as usize], battery::RED, "the minus badge is red");
        let (cx, cy) = centre(r);
        t.down(cx, cy, 200.0);
        t.moved(cx, cy + 40.0, 210.0);
        t.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 }, &l, -(W as f32));
        assert!(describe(&t).starts_with("today_editing=true"));
    }
}
