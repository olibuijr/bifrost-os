//! Control Center: a swipe down from the top-right of the status bar drags a
//! cluster of bifrost 15-style modules over a frosted copy of the current
//! screen. Geometry follows the bifrost mini: 65 pt tiles, 10 pt gaps, the
//! cluster 34 pt from the right edge and 68 pt from the top.

use super::app_controls::{join_pref, pref_app_keys, AppControlEntry, AppControls};
use super::controls::{draw_switch, SWITCH_H_PT, SWITCH_W_PT};
use super::gestures::{ease_out_quartic, progress, Drag, SPRING_MS};
use bifrost_ui::control_widget::ControlKind;
use std::time::Duration;
use super::music::nowplaying::Target as Media;
use super::music::{draw_cc_controls, icons};
use super::services::now_playing;
use crate::shell::switcher::draw_scaled;
use bifrost_ui::material_request::ShellSurface;
use bifrost_ui::visual_effect::{MaterialKind, MaterialRequest, MaterialState};
use bifrost_ui::media_session::PlaybackState;
use bifrost_ui::symbol_effect::{Effect as SymbolFx, Options as SymbolOptions, SymbolEffect};
use super::AppId;
use super::ui::{Ui, Weight, SECONDARY, WHITE};
use super::{Effect, Event, Info, Screen, Shell, WifiAction};
use crate::gfx::{over, pack, rgba, scale, shade, Image, Surface};
use crate::layout::Rect;
use crate::scene::draw_status_right;
use crate::status;

const SMALL_PT: f32 = 65.5;
const GAP_PT: f32 = 10.5;
const BIG_PT: f32 = 2.0 * SMALL_PT + GAP_PT;
const PITCH_PT: f32 = SMALL_PT + GAP_PT;
const BIG_PITCH_PT: f32 = BIG_PT + GAP_PT;
const RIGHT_PT: f32 = 34.0;
const TOP_PT: f32 = 68.0;
const RADIUS_PT: f32 = 6.0;
const CIRCLE_PT: f32 = 52.0;
const CIRCLE_PAD_PT: f32 = 14.0;
/// finger travel for a full reveal
const REVEAL_PT: f32 = 300.0;
/// a touch that moves less than this is a tap
const TAP_PT: f32 = 8.0;
/// blur radius on the quarter-size copy, in pt
const BLUR_PT: f32 = 8.0;
pub const BLUE: u32 = pack(10, 132, 255, 255);
/// the Rotation Lock glyph while locked, on its white tile
const LOCK_RED: u32 = pack(255, 69, 58, 255);

/// Something a finger can land on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tile {
    Airplane,
    /// Nearby, top right of the connectivity block
    Nearby,
    Wifi,
    Bluetooth,
    /// Rotation Lock, first in the row below the network tile as on the bifrost
    RotationLock,
    /// screen mirroring, beside it
    Mirror,
    Focus,
    /// silent mode, first of the bottom row
    Silent,
    Timer,
    Camera,
    Brightness,
    Volume,
    /// the media tile's transport buttons
    MediaPrev,
    MediaPlay,
    MediaNext,
}

/// Top-left of the module cluster in px.
fn origin(ui: &Ui, w: i32) -> (f32, f32) {
    (w as f32 - ui.pt(RIGHT_PT + 2.0 * BIG_PT + GAP_PT), ui.pt(TOP_PT))
}

/// The whole cluster (x, y, w, h) in px.
pub fn cluster(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    (x, y, ui.pt(2.0 * BIG_PT + GAP_PT), ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT + SMALL_PT))
}

/// The network tile that holds the Airplane and Wi-Fi circles.
fn network_tile(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    (x, y, ui.pt(BIG_PT), ui.pt(BIG_PT))
}

/// The Now Playing tile beside it.
fn media_tile(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    (x + ui.pt(BIG_PITCH_PT), y, ui.pt(BIG_PT), ui.pt(BIG_PT))
}

/// App-control tiles per row, under the built-in modules: four square tiles span the cluster's width.
const APP_COLS: usize = 4;

/// How long an app control's request may take on its control socket.
const APP_CONTROL_WAIT: Duration = Duration::from_secs(8);

/// Where app control `index` sits under the built-in modules, (x, y, w, h) in px. Rows fill left to right.
pub fn app_tile_rect(ui: &Ui, w: i32, index: usize) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    let col = (index % APP_COLS) as f32;
    let row = (index / APP_COLS) as f32;
    (x + ui.pt(col * PITCH_PT), y + ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT + SMALL_PT + GAP_PT + row * PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT))
}

/// The app control tile under (x, y), of `count` tiles on the panel.
pub fn app_hit(ui: &Ui, w: i32, count: usize, x: f32, y: f32) -> Option<usize> {
    (0..count).find(|&i| inside(app_tile_rect(ui, w, i), x, y))
}

/// The Edit Controls button, under the rows of `count` app control tiles, (x, y, w, h) in px.
pub fn edit_button_rect(ui: &Ui, w: i32, count: usize) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    let rows = count.div_ceil(APP_COLS) as f32;
    (x, y + ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT + SMALL_PT + GAP_PT + rows * PITCH_PT), ui.pt(2.0 * BIG_PT + GAP_PT), ui.pt(32.0))
}

/// Row `index` of the Edit Controls list, which replaces the cluster while it is open: (x, y, w, h) in px.
pub fn gallery_row_rect(ui: &Ui, w: i32, index: usize) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    (x, y + ui.pt(ROW_PT) * index as f32, ui.pt(2.0 * BIG_PT + GAP_PT), ui.pt(ROW_PT))
}

/// The Edit Controls row under (x, y), of `rows` rows.
pub fn gallery_hit(ui: &Ui, w: i32, rows: usize, x: f32, y: f32) -> Option<usize> {
    (0..rows).find(|&i| inside(gallery_row_rect(ui, w, i), x, y))
}

/// Where a tile sits, (x, y, w, h) in px.
pub fn tile_rect(ui: &Ui, w: i32, tile: Tile) -> (f32, f32, f32, f32) {
    let (x, y) = origin(ui, w);
    let circle = |col: f32, row: f32| (x + ui.pt(CIRCLE_PAD_PT + col * (CIRCLE_PT + GAP_PT)), y + ui.pt(CIRCLE_PAD_PT + row * (CIRCLE_PT + GAP_PT)), ui.pt(CIRCLE_PT), ui.pt(CIRCLE_PT));
    match tile {
        Tile::Airplane => circle(0.0, 0.0),
        Tile::Nearby => circle(1.0, 0.0),
        Tile::Wifi => circle(0.0, 1.0),
        Tile::Bluetooth => circle(1.0, 1.0),
        Tile::RotationLock => (x, y + ui.pt(BIG_PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT)),
        Tile::Mirror => (x + ui.pt(PITCH_PT), y + ui.pt(BIG_PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT)),
        Tile::Focus => (x, y + ui.pt(BIG_PITCH_PT + PITCH_PT), ui.pt(BIG_PT), ui.pt(SMALL_PT)),
        Tile::Silent => (x, y + ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT)),
        Tile::Timer => (x + ui.pt(PITCH_PT), y + ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT)),
        Tile::Camera => (x + ui.pt(2.0 * PITCH_PT), y + ui.pt(BIG_PITCH_PT + 2.0 * PITCH_PT), ui.pt(SMALL_PT), ui.pt(SMALL_PT)),
        Tile::Brightness => (x + ui.pt(BIG_PITCH_PT), y + ui.pt(BIG_PITCH_PT), ui.pt(SMALL_PT), ui.pt(BIG_PT)),
        Tile::Volume => (x + ui.pt(BIG_PITCH_PT + PITCH_PT), y + ui.pt(BIG_PITCH_PT), ui.pt(SMALL_PT), ui.pt(BIG_PT)),
        Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => {
            let (mx, my, mw, mh) = media_tile(ui, w);
            let k = match tile {
                Tile::MediaPrev => 0.22,
                Tile::MediaPlay => 0.5,
                _ => 0.78,
            };
            let b = ui.pt(CIRCLE_PT);
            (mx + mw * k - b / 2.0, my + mh * 0.72 - b / 2.0, b, b)
        }
    }
}

fn inside((rx, ry, rw, rh): (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= rx && y >= ry && x < rx + rw && y < ry + rh
}

/// The tile under (x, y).
pub fn hit(ui: &Ui, w: i32, x: f32, y: f32) -> Option<Tile> {
    [Tile::Airplane, Tile::Nearby, Tile::Wifi, Tile::Bluetooth, Tile::RotationLock, Tile::Mirror, Tile::Focus, Tile::Silent, Tile::Timer, Tile::Camera, Tile::Brightness, Tile::Volume, Tile::MediaPrev, Tile::MediaPlay, Tile::MediaNext].into_iter().find(|&t| inside(tile_rect(ui, w, t), x, y))
}

/// A slider's value for a finger at height `y`: full at the top.
fn slider_value(ui: &Ui, w: i32, tile: Tile, y: f32) -> f32 {
    let (_, ty, _, th) = tile_rect(ui, w, tile);
    ((ty + th - y) / th).clamp(0.0, 1.0)
}

/// The sheet's position between hidden and shown.
#[derive(Clone, Copy, Debug)]
enum State {
    Hidden,
    /// following the finger
    Dragging(Drag),
    Settling { from: f32, to: f32, start: f64 },
    Shown,
}

/// A finger on the shown sheet.
#[derive(Clone, Copy, Debug)]
enum Touch {
    Tile(Tile),
    Slider(Tile),
    /// outside the modules: a tap dismisses, a drag up pulls the sheet away
    Outside(Drag),
    /// a finger on the expanded module: a big slider being dragged, or a tap target
    Panel(Option<Tile>),
    /// a tap on the Now Playing plate's progress bar: released there, it seeks the shown session
    Scrub,
    /// a finger on an app control tile (its index in the panel's order): released there, it uses the control
    App(usize),
    /// a finger on the Edit Controls button
    Edit,
    /// a finger on a row of the Edit Controls list (the last row is Done)
    Gallery(usize),
}

/// How long a finger rests on a module before it expands, like bifrost.
pub const EXPAND_MS: f64 = 400.0;
const ROW_PT: f32 = 44.0;
const HEADER_PT: f32 = 64.0;
const PANEL_FILL: u32 = pack(30, 30, 34, 238);

/// A module opened to full size by a long press.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expanded {
    /// the connectivity cluster with the Wi-Fi network list
    Connectivity,
    Brightness,
    Volume,
    NowPlaying,
}

impl Expanded {
    fn of(tile: Tile) -> Option<Expanded> {
        match tile {
            Tile::Airplane | Tile::Wifi | Tile::Nearby | Tile::Bluetooth => Some(Expanded::Connectivity),
            Tile::Brightness => Some(Expanded::Brightness),
            Tile::Volume => Some(Expanded::Volume),
            Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => Some(Expanded::NowPlaying),
            _ => None,
        }
    }

    fn slider(self) -> Option<Tile> {
        match self {
            Expanded::Brightness => Some(Tile::Brightness),
            Expanded::Volume => Some(Tile::Volume),
            _ => None,
        }
    }
}

/// Where an expanded module sits, (x, y, w, h) in px: over the cluster.
fn panel_rect(ui: &Ui, w: i32, e: Expanded) -> (f32, f32, f32, f32) {
    let (cx, cy, cw, ch) = cluster(ui, w);
    match e {
        Expanded::Connectivity => (cx, cy, cw, ch),
        Expanded::NowPlaying => (cx, cy, cw, ui.pt(BIG_PT * 1.25)),
        Expanded::Brightness | Expanded::Volume => {
            let pw = ui.pt(BIG_PT);
            (cx + (cw - pw) / 2.0, cy, pw, ch)
        }
    }
}

/// The transport buttons of the expanded Now Playing card.
fn panel_button(ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), tile: Tile) -> (f32, f32, f32, f32) {
    let k = match tile {
        Tile::MediaPrev => 0.22,
        Tile::MediaPlay => 0.5,
        _ => 0.78,
    };
    let b = ui.pt(CIRCLE_PT);
    (x + w * k - b / 2.0, y + h * 0.72 - b / 2.0, b, b)
}

/// How many network rows fit in the expanded connectivity panel.
fn visible_rows(ui: &Ui, w: i32, info: &Info) -> usize {
    let (_, _, _, ph) = panel_rect(ui, w, Expanded::Connectivity);
    info.networks.len().min(((ph - ui.pt(HEADER_PT)) / ui.pt(ROW_PT)).max(0.0) as usize)
}

/// The current screen, sharp and frosted, both full size.
pub struct Frost {
    pub sharp: Image,
    pub frosted: Image,
}

impl Frost {
    /// Blurs a quarter-size copy (radius `blur` px there), darkens it by
    /// `dim` / 255 and scales it back up.
    pub fn new(sharp: Image, blur: usize, dim: u32) -> Self {
        // Reduce Transparency: no see-through blur, a near-opaque dark backdrop
        let (blur, dim) = if super::accessibility::reduce_transparency() { (0, dim.max(235)) } else { (blur, dim) };
        let mut small = downscale4(&sharp);
        small.blur(blur);
        for p in small.px.iter_mut() {
            *p = shade(*p, dim);
        }
        let frosted = upscale(&small, sharp.w, sharp.h);
        Frost { sharp, frosted }
    }
}

/// Box-averages 4x4 blocks.
fn downscale4(img: &Image) -> Image {
    let (w, h) = (img.w / 4, img.h / 4);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for dy in 0..4 {
                for dx in 0..4 {
                    let p = img.px[(y * 4 + dy) * img.w + x * 4 + dx];
                    for (ch, s) in sum.iter_mut().enumerate() {
                        *s += (p >> (ch * 8)) & 255;
                    }
                }
            }
            out.px[y * w + x] = (0..4).fold(0, |c, ch| c | ((sum[ch] + 8) / 16) << (ch * 8));
        }
    }
    out
}

/// Separable bilinear upscale with 8-bit fixed-point weights.
fn upscale(img: &Image, w: usize, h: usize) -> Image {
    let taps = |n: usize, src: usize| -> Vec<(usize, usize, u32)> {
        (0..n)
            .map(|i| {
                let f = ((i as f32 + 0.5) * src as f32 / n as f32 - 0.5).clamp(0.0, (src - 1) as f32);
                let i0 = f as usize;
                (i0, (i0 + 1).min(src - 1), (f.fract() * 256.0) as u32)
            })
            .collect()
    };
    let mix = |a: u32, b: u32, f: u32| -> u32 {
        (0..4).fold(0, |c, ch| {
            let (va, vb) = ((a >> (ch * 8)) & 255, (b >> (ch * 8)) & 255);
            c | ((va * (256 - f) + vb * f + 128) >> 8) << (ch * 8)
        })
    };
    let cols = taps(w, img.w);
    let mut wide = Image::new(w, img.h);
    for y in 0..img.h {
        let row = &img.px[y * img.w..][..img.w];
        for (x, &(x0, x1, f)) in cols.iter().enumerate() {
            wide.px[y * w + x] = mix(row[x0], row[x1], f);
        }
    }
    let mut out = Image::new(w, h);
    for (y, &(y0, y1, f)) in taps(h, img.h).iter().enumerate() {
        let (a, b) = (&wide.px[y0 * w..][..w], &wide.px[y1 * w..][..w]);
        for x in 0..w {
            out.px[y * w + x] = mix(a[x], b[x], f);
        }
    }
    out
}

/// Text rendered for the current network and airplane state.
struct Labels {
    key: (String, bool),
    focus_off: Image,
    focus_on: Image,
}

impl Labels {
    fn key(info: &Info, airplane: bool) -> (String, bool) {
        (info.wifi.clone(), airplane)
    }

    fn new(ui: &Ui, info: &Info, airplane: bool) -> Self {
        Labels {
            key: Self::key(info, airplane),
            focus_off: ui.text("Focus", 15.0, Weight::Semibold, WHITE),
            focus_on: ui.text("On", 15.0, Weight::Semibold, WHITE),
        }
    }
}

pub struct ControlCenter {
    state: State,
    touch: Option<Touch>,
    pub airplane: bool,
    /// (wi-fi, bluetooth) as they were before Airplane Mode switched them off
    airplane_prior: Option<(bool, bool)>,
    /// 0..=1, applied through `Effect::Volume`
    pub volume: f32,
    /// where the sound goes; the volume slider's glyph shows it
    pub route: crate::media::route::Route,
    backdrop: Option<Frost>,
    /// the desktop panel shows Display only with a backlight to drive
    backlight: bool,
    labels: Option<Labels>,
    /// the module opened by a long press, if any
    expanded: Option<Expanded>,
    /// a finger resting on an expandable tile: (tile, down time, x, y)
    press: Option<(Tile, f64, f32, f32)>,
    /// the toggle whose glyph bounces after it changed, with the bounce effect (seconds on the shell clock)
    bounce: Option<(Tile, SymbolEffect)>,
    /// the app controls shown under the built-in modules, in panel order
    pub app_controls: AppControls,
    /// the Edit Controls list is showing in place of the modules
    pub editing: bool,
}

impl Default for ControlCenter {
    fn default() -> Self {
        ControlCenter { state: State::Hidden, touch: None, airplane: false, airplane_prior: None, volume: 0.6, route: Default::default(), backdrop: None, backlight: false, labels: None, expanded: None, press: None, bounce: None, app_controls: AppControls::new(), editing: false }
    }
}

impl ControlCenter {
    /// The toggles whose glyph bounces once when they change (a Bounce symbol effect, as on the toggles of a PC panel).
    const BOUNCE_TILES: [Tile; 6] = [Tile::Airplane, Tile::Wifi, Tile::Bluetooth, Tile::RotationLock, Tile::Focus, Tile::Silent];

    /// Starts the bounce of toggle `tile` at `t` ms. Nothing starts under Reduce Motion or for a tile that is not a toggle.
    pub fn start_toggle_bounce(&mut self, tile: Tile, t: f64) {
        if !Self::BOUNCE_TILES.contains(&tile) || bifrost_render::appearance::reduce_motion() {
            return;
        }
        self.bounce = Some((tile, SymbolEffect::new(SymbolFx::Bounce, SymbolOptions::default(), t / 1000.0)));
    }

    /// The glyph scale of `tile` at `t` ms: 1 unless its bounce is playing.
    pub fn glyph_scale(&self, tile: Tile, t: f64) -> f32 {
        match self.bounce {
            Some((k, effect)) if k == tile => effect.pose(t / 1000.0).scale,
            _ => 1.0,
        }
    }

    pub fn is_hidden(&self) -> bool {
        matches!(self.state, State::Hidden)
    }

    pub fn is_shown(&self) -> bool {
        matches!(self.state, State::Shown)
    }

    pub fn animating(&self) -> bool {
        matches!(self.state, State::Settling { .. }) || self.bounce.is_some()
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

    /// Drops the sheet and its backdrop at once (screen off, lock).
    pub fn hide(&mut self) {
        self.state = State::Hidden;
        self.touch = None;
        self.backdrop = None;
        self.expanded = None;
        self.press = None;
        self.editing = false;
    }

    /// When a resting finger turns into an expansion.
    pub fn next_wake(&self) -> Option<f64> {
        self.press.map(|(_, t0, _, _)| t0 + super::accessibility::long_press_ms(EXPAND_MS))
    }

    fn settle(&mut self, from: f32, shown: bool, t: f64) {
        self.touch = None;
        if !shown {
            self.editing = false;
        }
        self.state = State::Settling { from, to: if shown { 1.0 } else { 0.0 }, start: t };
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
}

impl Shell {
    /// True while the sheet covers the screen in any way.
    pub(super) fn cc_active(&self) -> bool {
        !self.cc.is_hidden()
    }

    /// Opens the same sheet used by a status-bar swipe, for AssistiveTouch.
    pub(crate) fn open_control_center(&mut self, t: f64) {
        if self.desk_active() {
            return self.open_desk_control_center(t);
        }
        if !self.screen_on || self.screen == Screen::Lock {
            return;
        }
        self.notify.center.hide();
        let sharp = self.snapshot_screen(t);
        self.cc.backdrop = Some(Frost::new(sharp, (self.ui.pt(BLUR_PT) / 4.0) as usize, 140));
        self.cc.settle(0.0, true, t);
        self.dirty = true;
    }

    /// Whether a touch at (x, y) starts a reveal: the right third of the status bar (on a wide
    /// desktop panel at most 320 pt of it, around the status icons).
    pub(super) fn cc_reveal_zone(&self, x: f32, y: f32) -> bool {
        let zone = if crate::layout::adaptive() { (self.w as f32 / 3.0).min(self.ui.pt(320.0)) } else { self.w as f32 / 3.0 };
        self.screen != Screen::Lock && y < self.layout.status_h as f32 && x > self.w as f32 - zone
    }

    /// Handles a touch that belongs to the sheet; None leaves it to the screen.
    pub(super) fn cc_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if self.desk_active() {
            return match self.cc.state {
                State::Hidden => None,
                State::Shown => Some(self.cc_desk_event(ev, t)),
                _ => Some(vec![]),
            };
        }
        let range = self.ui.pt(REVEAL_PT);
        match (self.cc.state, ev) {
            (State::Hidden, Event::Down(x, y)) if self.cc_reveal_zone(x, y) => {
                let candidates = self.cc_candidates();
                self.restore_app_controls(&candidates);
                let sharp = self.snapshot_screen(t);
                self.cc.backdrop = Some(Frost::new(sharp, (self.ui.pt(BLUR_PT) / 4.0) as usize, 140));
                self.cc.state = State::Dragging(Drag::new(y, t, 0.0, range));
                Some(vec![])
            }
            (State::Hidden, _) => None,
            (State::Dragging(mut d), Event::Move(_, y)) => {
                d.moved(y, t);
                self.cc.state = State::Dragging(d);
                Some(vec![])
            }
            (State::Dragging(mut d), Event::Up(_, y)) => {
                d.moved(y, t);
                self.cc.settle(d.progress(), d.release(t), t);
                Some(vec![])
            }
            (State::Shown, ev) if self.cc.expanded.is_some() => Some(self.cc_expanded_event(ev, t)),
            (State::Shown, Event::Down(x, y)) => Some(self.cc_down(x, y, t, range)),
            (State::Shown, Event::Move(x, y)) => Some(self.cc_move(x, y, t)),
            (State::Shown, Event::Up(x, y)) => Some(self.cc_up(x, y, t)),
            _ => Some(vec![]),
        }
    }

    /// The module list the user saved (`MODULES_PREF`), read as the cluster shows it.
    fn cc_slots(&self) -> Vec<ModuleSlot> {
        parse_slots(self.prefs.get(MODULES_PREF).unwrap_or(""))
    }

    /// Every app control the running apps declare, in the order the apps are running.
    fn cc_candidates(&self) -> Vec<AppControlEntry> {
        self.ext
            .iter()
            .flatten()
            .flat_map(|h| h.controls.iter().map(move |c| AppControlEntry { app_id: h.id.clone(), control: c.clone(), identifier: c.identifier(&h.id) }))
            .collect()
    }

    /// Puts back on the panel each app control the saved list names, once its app declares it.
    fn restore_app_controls(&mut self, candidates: &[AppControlEntry]) {
        for key in pref_app_keys(self.prefs.get(MODULES_PREF).unwrap_or("")) {
            if self.cc.app_controls.keys().contains(&key) {
                continue;
            }
            if let Some(c) = candidates.iter().find(|c| c.identifier == key) {
                let _ = self.cc.app_controls.add(&c.app_id, c.control.clone());
            }
        }
    }

    /// Adds an app control to the panel, or takes it off when it is already there, and saves the list.
    fn toggle_app_control(&mut self, c: &AppControlEntry) {
        if self.cc.app_controls.keys().contains(&c.identifier) {
            self.cc.app_controls.remove(&c.identifier);
        } else {
            let _ = self.cc.app_controls.add(&c.app_id, c.control.clone());
        }
        let slots = self.cc_slots();
        self.cc_save(&slots);
    }

    /// Writes the module list and the app controls on the panel to `MODULES_PREF`.
    fn cc_save(&mut self, slots: &[ModuleSlot]) {
        let text = save_text(slots, &self.cc.app_controls.keys());
        let _ = self.prefs.set(MODULES_PREF, &text);
    }

    /// Opens the Edit Controls list and brings back the app controls the saved list names.
    fn cc_open_edit(&mut self) {
        let candidates = self.cc_candidates();
        self.restore_app_controls(&candidates);
        self.cc.editing = true;
    }

    /// The Edit Controls row under (x, y), while the list is open.
    fn cc_gallery_at(&self, x: f32, y: f32) -> Option<usize> {
        gallery_hit(&self.ui, self.w, self.cc_gallery_rows(), x, y)
    }

    /// The rows of the Edit Controls list: every module, then every app control, then Done.
    fn cc_gallery_rows(&self) -> usize {
        DEFAULT_MODULES.len() + self.cc_candidates().len() + 1
    }

    /// Uses the Edit Controls row `index`: a module is shown or hidden, an app control is added or removed, Done closes.
    fn cc_gallery_tap(&mut self, index: usize) {
        let modules = DEFAULT_MODULES.len();
        if index < modules {
            let tile = DEFAULT_MODULES[index];
            let mut slots = self.cc_slots();
            if let Some(slot) = slots.iter_mut().find(|m| m.tile == tile) {
                slot.shown = !slot.shown;
            }
            self.cc_save(&slots);
        } else if index < self.cc_gallery_rows() - 1 {
            let candidates = self.cc_candidates();
            if let Some(c) = candidates.get(index - modules) {
                self.toggle_app_control(c);
            }
        } else {
            self.cc.editing = false;
        }
    }

    /// Draws the Edit Controls list over the frosted backdrop, in the cluster's place.
    fn draw_gallery(&self, s: &mut Surface, slide: f32, alpha: u32) {
        let ui = &self.ui;
        let a = |c: u32| scale(c, alpha);
        let candidates = self.cc_candidates();
        let rows = DEFAULT_MODULES.len() + candidates.len() + 1;
        let slots = self.cc_slots();
        let (x0, y0, w0, _) = gallery_row_rect(ui, self.w, 0);
        s.fill_rrect(x0, y0 + slide, w0, ui.pt(ROW_PT) * rows as f32, ui.pt(RADIUS_PT), a(rgba(118, 118, 128, 96)));
        for i in 0..rows {
            let (x, y, w, h) = gallery_row_rect(ui, self.w, i);
            let y = y + slide;
            if i == rows - 1 {
                let done = ui.text("Done", 15.0, Weight::Semibold, BLUE);
                s.blit(&done, (x + (w - done.w as f32) / 2.0) as i32, (y + (h - done.h as f32) / 2.0) as i32, 255, alpha);
                continue;
            }
            let (title, on) = if i < DEFAULT_MODULES.len() {
                let tile = DEFAULT_MODULES[i];
                (module_title(tile).to_string(), slots.iter().find(|m| m.tile == tile).is_none_or(|m| m.shown))
            } else {
                let c = &candidates[i - DEFAULT_MODULES.len()];
                (c.control.title.clone(), self.cc.app_controls.keys().contains(&c.identifier))
            };
            let label = ui.text(&title, 13.0, Weight::Regular, WHITE);
            s.blit(&label, (x + ui.pt(12.0)) as i32, (y + (h - label.h as f32) / 2.0) as i32, 255, alpha);
            let (sw, sh) = (ui.pt(SWITCH_W_PT), ui.pt(SWITCH_H_PT));
            draw_switch(s, ui, x + w - sw - ui.pt(12.0), y + (h - sh) / 2.0, if on { 1.0 } else { 0.0 });
        }
    }

    /// The tile under (x, y) that the cluster is showing: a module the user hid does not answer a finger.
    fn cc_tile_at(&self, x: f32, y: f32) -> Option<Tile> {
        let slots = self.cc_slots();
        hit(&self.ui, self.w, x, y).filter(|&t| tile_visible(&slots, t))
    }

    /// Whether (x, y) is on the Edit Controls button under the app control rows.
    fn cc_edit_at(&self, x: f32, y: f32) -> bool {
        inside(edit_button_rect(&self.ui, self.w, self.cc.app_controls.entries().len()), x, y)
    }

    /// The app control tile under (x, y), if any.
    fn cc_app_at(&self, x: f32, y: f32) -> Option<usize> {
        app_hit(&self.ui, self.w, self.cc.app_controls.entries().len(), x, y)
    }

    fn cc_down(&mut self, x: f32, y: f32, t: f64, range: f32) -> Vec<Effect> {
        if self.cc.editing {
            self.cc.touch = self.cc_gallery_at(x, y).map(Touch::Gallery);
            return vec![];
        }
        if self.cc_edit_at(x, y) {
            self.cc.touch = Some(Touch::Edit);
            return vec![];
        }
        if let Some(i) = self.cc_app_at(x, y) {
            self.cc.press = None;
            self.cc.touch = Some(Touch::App(i));
            return vec![];
        }
        let tile = self.cc_tile_at(x, y);
        self.cc.press = tile.filter(|&k| Expanded::of(k).is_some()).map(|k| (k, t, x, y));
        match tile {
            Some(tile @ (Tile::Brightness | Tile::Volume)) => {
                self.cc.touch = Some(Touch::Slider(tile));
                self.set_slider(tile, slider_value(&self.ui, self.w, tile, y))
            }
            Some(tile) => {
                self.cc.touch = Some(Touch::Tile(tile));
                vec![]
            }
            None => {
                self.cc.touch = Some(Touch::Outside(Drag::new(y, t, 1.0, range)));
                vec![]
            }
        }
    }

    fn cc_move(&mut self, x: f32, y: f32, t: f64) -> Vec<Effect> {
        if let Some((_, _, x0, y0)) = self.cc.press {
            if (x - x0).hypot(y - y0) > self.ui.pt(TAP_PT) {
                self.cc.press = None;
            }
        }
        match self.cc.touch {
            Some(Touch::Slider(tile)) => self.set_slider(tile, slider_value(&self.ui, self.w, tile, y)),
            Some(Touch::Outside(mut d)) => {
                d.moved(y, t);
                self.cc.touch = Some(Touch::Outside(d));
                vec![]
            }
            _ => vec![],
        }
    }

    fn cc_up(&mut self, x: f32, y: f32, t: f64) -> Vec<Effect> {
        self.cc.press = None;
        match self.cc.touch.take() {
            Some(Touch::Tile(tile)) if self.cc_tile_at(x, y) == Some(tile) => self.activate_tile(tile, t),
            Some(Touch::App(index)) if self.cc_app_at(x, y) == Some(index) => self.activate_app(index),
            Some(Touch::Edit) if self.cc_edit_at(x, y) => {
                self.cc_open_edit();
                vec![]
            }
            Some(Touch::Gallery(index)) if self.cc_gallery_at(x, y) == Some(index) => {
                self.cc_gallery_tap(index);
                vec![]
            }
            Some(Touch::Outside(mut d)) => {
                d.moved(y, t);
                let tap = d.travel().abs() < self.ui.pt(TAP_PT);
                self.cc.settle(d.progress(), !tap && d.release(t), t);
                vec![]
            }
            _ => vec![],
        }
    }

    fn set_slider(&mut self, tile: Tile, v: f32) -> Vec<Effect> {
        if tile == Tile::Brightness {
            self.info.brightness = v;
            vec![Effect::Brightness(v)]
        } else {
            self.cc.volume = v;
            vec![Effect::Volume(v)]
        }
    }

    /// The Control Center's Bluetooth switch: flips the radio and returns what the
    /// shell must carry out. Nothing happens while there is no controller. The tile
    /// itself is drawn by whoever lays out the grid; it calls this on a tap and reads
    /// `info.bt` (`on()`, `audio_sink()`) for its look and caption.
    pub fn toggle_bluetooth(&mut self) -> Vec<Effect> {
        use crate::bare::bluetooth::{Cmd, State};
        if self.info.bt.state == State::Unavailable {
            return vec![];
        }
        let on = !self.info.bt.on();
        self.info.bt.state = if on { State::On } else { State::Off };
        let mut fx = vec![Effect::Bluetooth(Cmd::Enable(on))];
        if on {
            fx.push(Effect::Bluetooth(Cmd::Scan));
        }
        fx
    }

    /// Airplane Mode: radios off (remembering what was on), and back to that when it ends.
    fn toggle_airplane(&mut self) -> Vec<Effect> {
        use crate::bare::bluetooth::{Cmd, State};
        let has_bt = self.info.bt.state != State::Unavailable;
        let mut fx = vec![];
        let (wifi, bt) = if self.cc.airplane {
            self.cc.airplane = false;
            self.cc.airplane_prior.take().unwrap_or((true, false))
        } else {
            self.cc.airplane = true;
            self.cc.airplane_prior = Some((self.info.wifi_on, self.info.bt.on()));
            (false, false)
        };
        self.info.wifi_on = wifi;
        fx.push(Effect::Wifi(WifiAction::Enable(wifi)));
        if has_bt && self.info.bt.on() != bt {
            self.info.bt.state = if bt { State::On } else { State::Off };
            fx.push(Effect::Bluetooth(Cmd::Enable(bt)));
            if bt {
                fx.push(Effect::Bluetooth(Cmd::Scan));
            }
        }
        fx
    }

    /// Uses app control `index`: a toggle flips in the registry, and its request goes to the app's control socket on
    /// a worker thread. The app's dialog text, if any, is not shown yet.
    fn activate_app(&mut self, index: usize) -> Vec<Effect> {
        let Some(entry) = self.cc.app_controls.entries().get(index).cloned() else { return vec![] };
        let Ok(line) = self.cc.app_controls.press(&entry.identifier) else { return vec![] };
        std::thread::spawn(move || {
            if let Err(e) = super::intents::exchange(&entry.app_id, &line, APP_CONTROL_WAIT).and_then(|reply| super::intents::parse_reply(&reply)) {
                eprintln!("control center: {}: {e}", entry.app_id);
            }
        });
        vec![]
    }

    fn activate_tile(&mut self, tile: Tile, t: f64) -> Vec<Effect> {
        self.cc.start_toggle_bounce(tile, t);
        match tile {
            Tile::RotationLock => self.toggle_rotation_lock(t),
            Tile::Airplane => {
                self.toggle_airplane()
            }
            Tile::Wifi => {
                self.cc.airplane = false;
                self.cc.airplane_prior = None;
                self.info.wifi_on = !self.info.wifi_on;
                vec![Effect::Wifi(WifiAction::Enable(self.info.wifi_on))]
            }
            Tile::Nearby | Tile::Mirror | Tile::Timer => vec![],
            Tile::Bluetooth => self.toggle_bluetooth(),
            Tile::Silent => {
                self.set_music_muted(!self.music_muted);
                vec![]
            }
            Tile::Camera => {
                self.cc.hide();
                self.open(AppId::Camera, t);
                vec![]
            }
            Tile::Focus => {
                self.set_dnd(!self.dnd());
                vec![]
            }
            Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => {
                self.media_control(media_target(tile).unwrap_or(Media::Play));
                vec![]
            }
            Tile::Brightness | Tile::Volume => vec![],
        }
    }

    /// Touches while a module is expanded: big sliders, transport buttons, the network list.
    fn cc_expanded_event(&mut self, ev: Event, _t: f64) -> Vec<Effect> {
        let Some(e) = self.cc.expanded else { return vec![] };
        let panel = panel_rect(&self.ui, self.w, e);
        match ev {
            Event::Down(x, y) => {
                if !inside(panel, x, y) {
                    self.cc.expanded = None;
                    self.cc.touch = None;
                    self.dirty = true;
                    return vec![];
                }
                match e.slider() {
                    Some(tile) => {
                        self.cc.touch = Some(Touch::Panel(Some(tile)));
                        self.set_slider(tile, ((panel.1 + panel.3 - y) / panel.3).clamp(0.0, 1.0))
                    }
                    None => {
                        self.cc.touch = Some(Touch::Panel(None));
                        vec![]
                    }
                }
            }
            Event::Move(_, y) => match self.cc.touch {
                Some(Touch::Panel(Some(tile))) => self.set_slider(tile, ((panel.1 + panel.3 - y) / panel.3).clamp(0.0, 1.0)),
                _ => vec![],
            },
            Event::Up(x, y) => {
                let was = self.cc.touch.take();
                if !matches!(was, Some(Touch::Panel(None))) {
                    return vec![];
                }
                self.dirty = true;
                match e {
                    Expanded::NowPlaying => {
                        for tile in [Tile::MediaPrev, Tile::MediaPlay, Tile::MediaNext] {
                            if inside(panel_button(&self.ui, panel, tile), x, y) {
                                self.media_control(media_target(tile).unwrap_or(Media::Play));
                            }
                        }
                        vec![]
                    }
                    Expanded::Connectivity => self.connectivity_tap(panel, y),
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }

    /// A tap inside the expanded connectivity panel: the Wi-Fi header or a network row.
    fn connectivity_tap(&mut self, panel: (f32, f32, f32, f32), y: f32) -> Vec<Effect> {
        let py = panel.1;
        let header = self.ui.pt(HEADER_PT);
        if y < py + header {
            return self.activate_tile(Tile::Wifi, 0.0);
        }
        let row = ((y - py - header) / self.ui.pt(ROW_PT)) as usize;
        if !self.info.wifi_on || row >= visible_rows(&self.ui, self.w, &self.info) {
            return vec![];
        }
        let net = self.info.networks[row].clone();
        if net.saved {
            vec![Effect::Wifi(WifiAction::Select(net.ssid))]
        } else if !net.secure {
            vec![Effect::Wifi(WifiAction::Join { ssid: net.ssid, psk: None })]
        } else {
            vec![]
        }
    }

    /// Finishes a settle animation, and expands a module the finger has rested on.
    pub(super) fn cc_tick(&mut self, t: f64) -> Vec<Effect> {
        if self.cc.bounce.is_some_and(|(_, effect)| effect.is_done(t / 1000.0)) {
            self.cc.bounce = None;
        }
        if let (State::Shown, Some((tile, _, _, _))) = (self.cc.state, self.cc.press) {
            if self.cc.next_wake().is_some_and(|w| t >= w) {
                self.cc.press = None;
                self.cc.touch = None;
                self.cc.expanded = Expanded::of(tile);
                if self.cc.expanded == Some(Expanded::Connectivity) {
                    self.dirty = true;
                    return vec![Effect::Wifi(WifiAction::Scan)];
                }
                self.dirty = true;
            }
        }
        if let State::Settling { to, start, .. } = self.cc.state {
            if progress(start, t, SPRING_MS) >= 1.0 {
                if to >= 1.0 {
                    self.cc.state = State::Shown;
                } else {
                    self.cc.hide();
                }
            }
        }
        vec![]
    }

    /// Draws the frosted screen and the module cluster for the sheet's progress at `t`.
    pub(super) fn draw_cc(&mut self, s: &mut Surface, t: f64) {
        if self.desk_active() {
            return self.draw_desk_cc(s, t);
        }
        let p = self.cc.progress(t);
        let Some(frost) = &self.cc.backdrop else { return };
        let fade = (p.clamp(0.0, 1.0) * 255.0) as u32;
        if fade < 255 {
            s.copy(frost.sharp.view(), 0, 0);
            s.blit(&frost.frosted, 0, 0, 255, fade);
        } else {
            s.copy(frost.frosted.view(), 0, 0);
        }
        if self.cc.labels.as_ref().is_none_or(|l| l.key != Labels::key(&self.info, self.cc.airplane)) {
            self.cc.labels = Some(Labels::new(&self.ui, &self.info, self.cc.airplane));
        }
        let slots = self.cc_slots();
        let ui = &self.ui;
        let alpha = (p * 2.0).clamp(0.0, 1.0);
        self.draw_bar(s, (alpha * 255.0) as u32);
        let (_, cy, _, ch) = cluster(ui, self.w);
        let slide = (p.min(1.3) - 1.0) * (cy + ch);
        let a = |c: u32| scale(c, (alpha * 255.0) as u32);
        let r = ui.pt(RADIUS_PT);
        // bifrost modules are lighter than the blurred backdrop (a light translucent fill), not darker
        let tile_bg = a(rgba(118, 118, 128, 96));
        let pressed = match self.cc.touch {
            Some(Touch::Tile(t)) => Some(t),
            _ => None,
        };
        let labels = self.cc.labels.as_ref().unwrap();
        if self.cc.editing {
            self.draw_gallery(s, slide, (alpha * 255.0) as u32);
            return;
        }

        // connectivity: airplane, nearby, wi-fi and bluetooth circles
        let (nx, ny, nw, nh) = network_tile(ui, self.w);
        s.fill_rrect(nx, ny + slide, nw, nh, r, tile_bg);
        let bt_on = self.info.bt.on();
        for tile in [Tile::Airplane, Tile::Nearby, Tile::Wifi, Tile::Bluetooth] {
            if !tile_visible(&slots, tile) {
                continue;
            }
            let (x, y, w, h) = tile_rect(ui, self.w, tile);
            let on = match tile {
                Tile::Airplane => self.cc.airplane,
                Tile::Nearby => false,
                Tile::Wifi => self.info.wifi_on && !self.cc.airplane,
                _ => bt_on,
            };
            let mut bg = if on { BLUE } else { rgba(255, 255, 255, 34) };
            if pressed == Some(tile) {
                bg = over(bg, rgba(255, 255, 255, 50));
            }
            s.fill_rrect(x, y + slide, w, h, ui.pt(RADIUS_PT), a(bg));
            let (cx, cy) = (x + w / 2.0, y + slide + h / 2.0);
            match tile {
                Tile::Airplane => draw_airplane(s, cx, cy, ui.pt(26.0), a(WHITE)),
                Tile::Nearby => draw_nearby(s, cx, cy, ui.s, a(WHITE)),
                Tile::Wifi => status::draw_wifi(s, cx - ui.pt(13.0), cy - ui.pt(7.0), ui.pt(13.0), a(WHITE)),
                _ => draw_bluetooth(s, cx, cy, ui.s, a(WHITE)),
            }
        }

        // Now Playing
        let (kx, ky, kw, kh) = media_tile(ui, self.w);
        s.fill_rrect(kx, ky + slide, kw, kh, r, tile_bg);
        self.draw_media_tile(s, (kx, ky + slide, kw, kh), (alpha * 255.0) as u32, pressed.and_then(media_target));

        // rotation lock: a white tile with a red glyph while held, as on the bifrost
        if tile_visible(&slots, Tile::RotationLock) {
        let (rx, ry, rw, rh) = tile_rect(ui, self.w, Tile::RotationLock);
        let locked = self.rotation_locked();
        let mut bg = if locked { a(rgba(255, 255, 255, 240)) } else { tile_bg };
        if pressed == Some(Tile::RotationLock) {
            bg = over(bg, a(rgba(255, 255, 255, 50)));
        }
        s.fill_rrect(rx, ry + slide, rw, rh, r, bg);
        draw_rotation_lock(s, rx + rw / 2.0, ry + slide + rh / 2.0, ui.s, a(if locked { LOCK_RED } else { WHITE }));
        }

        // screen mirroring
        if tile_visible(&slots, Tile::Mirror) {
        let (lx, ly, lw, lh) = tile_rect(ui, self.w, Tile::Mirror);
        let bg = if pressed == Some(Tile::Mirror) { over(tile_bg, a(rgba(255, 255, 255, 50))) } else { tile_bg };
        s.fill_rrect(lx, ly + slide, lw, lh, r, bg);
        draw_mirror(s, lx + lw / 2.0, ly + slide + lh / 2.0, ui.s, a(WHITE));
        }

        // Focus: the moon beside its caption in a wide capsule
        if tile_visible(&slots, Tile::Focus) {
        let (fx, fy, fw, fh) = tile_rect(ui, self.w, Tile::Focus);
        let mut bg = if self.dnd() { a(rgba(94, 92, 230, 230)) } else { tile_bg };
        if pressed == Some(Tile::Focus) {
            bg = over(bg, a(rgba(255, 255, 255, 50)));
        }
        s.fill_rrect(fx, fy + slide, fw, fh, r, bg);
        let (cx, cy, radius) = (fx + ui.pt(32.0), fy + slide + fh / 2.0, ui.pt(12.0));
        s.fill_rrect(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius, radius, a(WHITE));
        let moon = super::glyphs::disc((cx, cy), radius * 0.62);
        let cut = super::glyphs::disc((cx + radius * 0.35, cy - radius * 0.25), radius * 0.55);
        super::glyphs::paint(s, (cx - radius, cy - radius, 2.0 * radius, 2.0 * radius), a(rgba(88, 86, 214, 255)), |x, y| moon(x, y).max(-cut(x, y)));
        let text = if self.dnd() { &labels.focus_on } else { &labels.focus_off };
        s.blit(text, (fx + ui.pt(70.0)) as i32, (fy + slide + (fh - text.h as f32) / 2.0) as i32, 255, (alpha * 255.0) as u32);
        }

        // bottom row: silent mode, timer, camera
        for tile in [Tile::Silent, Tile::Timer, Tile::Camera] {
            if !tile_visible(&slots, tile) {
                continue;
            }
            let (x, y, w, h) = tile_rect(ui, self.w, tile);
            let silent = tile == Tile::Silent && self.music_muted;
            let mut bg = if silent { a(rgba(255, 255, 255, 240)) } else { tile_bg };
            if pressed == Some(tile) {
                bg = over(bg, a(rgba(255, 255, 255, 50)));
            }
            s.fill_rrect(x, y + slide, w, h, r, bg);
            let (cx, cy) = (x + w / 2.0, y + slide + h / 2.0);
            match tile {
                Tile::Silent => draw_bell(s, cx, cy, ui.s, a(if silent { LOCK_RED } else { WHITE }), silent),
                Tile::Timer => draw_timer(s, cx, cy, ui.s, a(WHITE)),
                _ => draw_camera(s, cx, cy, ui.s, a(WHITE)),
            }
        }

        // brightness and volume sliders
        for (tile, v) in [(Tile::Brightness, self.info.brightness), (Tile::Volume, self.cc.volume)] {
            if !tile_visible(&slots, tile) {
                continue;
            }
            let (x, y, w, h) = tile_rect(ui, self.w, tile);
            let y = y + slide;
            s.fill_rrect(x, y, w, h, r, tile_bg);
            let top = (y + h * (1.0 - v)).round().max(0.0) as usize;
            s.band(top).fill_rrect(x, y, w, h, r, a(rgba(255, 255, 255, 215)));
            let (cx, cy) = (x + w / 2.0, y + h - ui.pt(26.0));
            if tile == Tile::Brightness {
                draw_sun(s, cx, cy, ui.s, a(SECONDARY));
            } else if self.cc.route == crate::media::route::Route::Headphones {
                super::volume_hud::draw_headphones(s, cx, cy, ui.s, a(SECONDARY));
            } else {
                draw_speaker(s, cx, cy, ui.s, a(SECONDARY));
            }
        }

        // app controls: one tile per control the user added, under the built-in modules
        let app_pressed = match self.cc.touch {
            Some(Touch::App(i)) => Some(i),
            _ => None,
        };
        for (i, entry) in self.cc.app_controls.entries().iter().enumerate() {
            let (x, y, w, h) = app_tile_rect(ui, self.w, i);
            let y = y + slide;
            let on = entry.control.kind == ControlKind::Toggle && self.cc.app_controls.value(&entry.identifier);
            let mut bg = if on { BLUE } else { rgba(255, 255, 255, 34) };
            if app_pressed == Some(i) {
                bg = over(bg, rgba(255, 255, 255, 50));
            }
            s.fill_rrect(x, y, w, h, r, a(bg));
            let (cx, cy) = (x + w / 2.0, y + h / 2.0 - ui.pt(8.0));
            draw_app_symbol(s, &entry.control.symbol, cx, cy, ui.s, a(WHITE));
            let title = ui.text(&entry.control.title, 10.0, Weight::Semibold, WHITE);
            let tx = (x + (w - title.w as f32) / 2.0).max(x) as i32;
            s.blit(&title, tx, (y + h - ui.pt(16.0)) as i32, 255, (alpha * 255.0) as u32);
        }

        // Edit Controls: opens the list of modules and app controls
        let (ex, ey, ew, eh) = edit_button_rect(ui, self.w, self.cc.app_controls.entries().len());
        let mut bg = tile_bg;
        if matches!(self.cc.touch, Some(Touch::Edit)) {
            bg = over(bg, a(rgba(255, 255, 255, 50)));
        }
        s.fill_rrect(ex, ey + slide, ew, eh, eh / 2.0, bg);
        let label = ui.text("Edit Controls", 13.0, Weight::Semibold, BLUE);
        s.blit(&label, (ex + (ew - label.w as f32) / 2.0) as i32, (ey + slide + (eh - label.h as f32) / 2.0) as i32, 255, (alpha * 255.0) as u32);

        if let Some(e) = self.cc.expanded {
            self.draw_expanded(s, e, (alpha * 255.0) as u32);
        }
    }

    /// The module opened by a long press, over a dimmed sheet.
    fn draw_expanded(&self, s: &mut Surface, e: Expanded, alpha: u32) {
        let ui = &self.ui;
        let a = |c: u32| scale(c, alpha);
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, a(rgba(0, 0, 0, 110)));
        let panel = panel_rect(ui, self.w, e);
        let (x, y, w, h) = panel;
        let r = ui.pt(RADIUS_PT);
        s.fill_rrect(x, y, w, h, r, a(PANEL_FILL));
        match e {
            Expanded::Brightness | Expanded::Volume => {
                let v = if e == Expanded::Brightness { self.info.brightness } else { self.cc.volume };
                let top = (y + h * (1.0 - v)).round().max(0.0) as usize;
                s.band(top).fill_rrect(x, y, w, h, r, a(rgba(255, 255, 255, 225)));
                let (cx, cy) = (x + w / 2.0, y + h - ui.pt(44.0));
                if e == Expanded::Brightness {
                    draw_sun(s, cx, cy, ui.s * 1.6, a(SECONDARY));
                } else if self.cc.route == crate::media::route::Route::Headphones {
                    super::volume_hud::draw_headphones(s, cx, cy, ui.s * 1.6, a(SECONDARY));
                } else {
                    draw_speaker(s, cx, cy, ui.s * 1.6, a(SECONDARY));
                }
                let name = ui.text(if e == Expanded::Brightness { "Brightness" } else { "Volume" }, 17.0, Weight::Semibold, a(rgba(60, 60, 67, 255)));
                s.blit(&name, (x + ui.pt(16.0)) as i32, (y + ui.pt(16.0)) as i32, 255, alpha);
            }
            Expanded::NowPlaying => {
                self.draw_media_tile(s, panel, alpha, None);
            }
            Expanded::Connectivity => {
                let on = self.info.wifi_on && !self.cc.airplane;
                let title = ui.text("Wi-Fi", 17.0, Weight::Semibold, WHITE);
                s.blit(&title, (x + ui.pt(16.0)) as i32, (y + ui.pt(12.0)) as i32, 255, alpha);
                let sub = if self.cc.airplane { "Airplane Mode".to_string() } else { self.info.wifi.clone() };
                let sub = ui.text(&ui.fit(&sub, 13.0, w - ui.pt(32.0 + 60.0)), 13.0, Weight::Regular, SECONDARY);
                s.blit(&sub, (x + ui.pt(16.0)) as i32, (y + ui.pt(36.0)) as i32, 255, alpha);
                // the switch
                let (sw, sh) = (ui.pt(51.0), ui.pt(31.0));
                let (sx, sy) = (x + w - sw - ui.pt(16.0), y + ui.pt(16.0));
                s.fill_rrect(sx, sy, sw, sh, ui.pt(RADIUS_PT), a(if on { super::ui::SYSTEM_GREEN } else { super::ui::TRACK }));
                let knob = sh - ui.pt(4.0);
                let kx = if on { sx + sw - knob - ui.pt(2.0) } else { sx + ui.pt(2.0) };
                s.fill_rrect(kx, sy + ui.pt(2.0), knob, knob, ui.pt(RADIUS_PT) - ui.pt(2.0), a(WHITE));
                if on {
                    let sep = a(rgba(255, 255, 255, 40));
                    for (i, net) in self.info.networks.iter().take(visible_rows(ui, self.w, &self.info)).enumerate() {
                        let ry = y + ui.pt(HEADER_PT) + i as f32 * ui.pt(ROW_PT);
                        s.fill_rrect(x + ui.pt(16.0), ry, w - ui.pt(32.0), 1.0f32.max(ui.s * 0.5), 0.0, sep);
                        let cur = self.info.current_ssid.as_deref() == Some(net.ssid.as_str());
                        let name = ui.text(&ui.fit(&net.ssid, 17.0, w - ui.pt(32.0 + 70.0)), 17.0, Weight::Regular, WHITE);
                        s.blit(&name, (x + ui.pt(40.0)) as i32, (ry + (ui.pt(ROW_PT) - name.h as f32) / 2.0) as i32, 255, alpha);
                        if cur {
                            let tick = ui.text("\u{2713}", 17.0, Weight::Semibold, BLUE);
                            s.blit(&tick, (x + ui.pt(16.0)) as i32, (ry + (ui.pt(ROW_PT) - tick.h as f32) / 2.0) as i32, 255, alpha);
                        }
                        status::draw_wifi(s, x + w - ui.pt(36.0), ry + ui.pt(ROW_PT) / 2.0 - ui.pt(6.0), ui.pt(12.0), a(if net.bars >= 2 { WHITE } else { SECONDARY }));
                    }
                }
            }
        }
    }

    /// The status bar as the home screen shows it: time and date left,
    /// radios right (those only once fully faded in).
    pub(super) fn draw_bar(&self, s: &mut Surface, alpha: u32) {
        let ui = &self.ui;
        let mid = self.layout.status_h / 2;
        let mut x = ui.pt(20.0) as i32;
        for img in [&self.chrome.time, &self.chrome.date].into_iter().flatten() {
            s.blit(img, x, mid - img.h as i32 / 2, 255, alpha);
            x += img.w as i32 + ui.pt(5.0) as i32;
        }
        if alpha == 255 {
            draw_status_right(s, self.w, mid, ui.s, &self.chrome);
        }
    }

    /// Renders the current screen as it is, for a frosted backdrop.
    pub(super) fn snapshot_screen(&mut self, t: f64) -> Image {
        let mut img = Image::new(self.w as usize, self.h as usize);
        let mut s = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
        match self.screen {
            Screen::App(app) => self.draw_app(&mut s, app, t),
            Screen::Switcher => self.draw_switcher(&mut s, t),
            Screen::Home | Screen::Lock => self.draw_home(&mut s),
        }
        img
    }
}

/// The Now Playing plate's progress bar: a thin strip near the bottom edge, inset from the sides.
fn media_bar((x, y, w, h): (f32, f32, f32, f32), ui: &Ui) -> (f32, f32, f32, f32) {
    (x + ui.pt(14.0), y + h - ui.pt(16.0), w - ui.pt(28.0), ui.pt(3.0))
}

/// Where a touch at `x` lands along the bar, as a share of its length from 0 to 1.
fn scrub_fraction((bx, _, bw, _): (f32, f32, f32, f32), x: f32) -> f32 {
    if bw <= 0.0 {
        return 0.0;
    }
    ((x - bx) / bw).clamp(0.0, 1.0)
}

/// The transport control a media tile button stands for.
fn media_target(tile: Tile) -> Option<Media> {
    match tile {
        Tile::MediaPrev => Some(Media::Prev),
        Tile::MediaPlay => Some(Media::Play),
        Tile::MediaNext => Some(Media::Next),
        _ => None,
    }
}

/// An airplane pointing up-right, `size` px across its wings.
/// The glyph an app control shows for its manifest `symbol` name. Names the shell has no glyph for get a plain dot.
fn draw_app_symbol(s: &mut Surface, name: &str, cx: f32, cy: f32, pt: f32, color: u32) {
    match name {
        "moon" => status::draw_moon(s, cx - pt / 2.0, cy - pt / 2.0, pt, color),
        "bell" => draw_bell(s, cx, cy, pt, color, false),
        "timer" => draw_timer(s, cx, cy, pt, color),
        "camera" => draw_camera(s, cx, cy, pt, color),
        "sun.max" | "sun" => draw_sun(s, cx, cy, pt, color),
        "speaker.wave.2" | "speaker" => draw_speaker(s, cx, cy, pt, color),
        "wifi" => status::draw_wifi(s, cx - pt / 2.0, cy - pt / 2.0, pt, color),
        _ => s.fill_rrect(cx - pt / 4.0, cy - pt / 4.0, pt / 2.0, pt / 2.0, pt / 4.0, color),
    }
}

fn draw_airplane(s: &mut Surface, cx: f32, cy: f32, size: f32, color: u32) {
    const P: [(f32, f32); 18] = [
        (0.5, 0.0),
        (0.58, 0.1),
        (0.58, 0.38),
        (1.0, 0.64),
        (1.0, 0.74),
        (0.58, 0.6),
        (0.58, 0.82),
        (0.7, 0.9),
        (0.7, 0.98),
        (0.5, 0.93),
        (0.3, 0.98),
        (0.3, 0.9),
        (0.42, 0.82),
        (0.42, 0.6),
        (0.0, 0.74),
        (0.0, 0.64),
        (0.42, 0.38),
        (0.42, 0.1),
    ];
    let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
    let pts: Vec<(f32, f32)> = P
        .iter()
        .map(|&(x, y)| {
            let (dx, dy) = ((x - 0.5) * size, (y - 0.5) * size);
            (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
        })
        .collect();
    icons::fill_poly(s, &pts, color);
}

/// Rotation Lock: a small padlock inside a three-quarter circular arrow.
fn draw_rotation_lock(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    s.stroke_rrect(cx - 4.0 * pt, cy - 8.5 * pt, 8.0 * pt, 10.0 * pt, 4.0 * pt, 1.8 * pt, color);
    s.fill_rrect(cx - 6.0 * pt, cy - 1.5 * pt, 12.0 * pt, 8.5 * pt, 2.0 * pt, color);
    // the arc runs clockwise from the top around the right and bottom to the left
    let (radius, thick) = (13.0 * pt, 2.0 * pt);
    for py in ((cy - radius - thick).max(0.0) as usize).max(s.y0)..((cy + radius + thick).ceil() as usize).min(s.h) {
        for px in ((cx - radius - thick).max(0.0) as usize)..((cx + radius + thick).ceil() as usize).min(s.w) {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            // leave the top-left quarter open, where the arrowhead sits
            if dx < 0.0 && dy < 0.0 {
                continue;
            }
            let d = (dx * dx + dy * dy).sqrt();
            let cov = (thick / 2.0 - (d - radius).abs() + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let p = s.at(px, py);
                *p = over(*p, scale(color, (cov * 255.0) as u32));
            }
        }
    }
    let (ax, ay) = (cx - radius, cy);
    icons::fill_poly(s, &[(ax - 4.0 * pt, ay + 0.5 * pt), (ax + 4.0 * pt, ay + 0.5 * pt), (ax, ay - 5.5 * pt)], color);
}

fn draw_nearby(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    for r in [6.0, 11.0, 16.0] {
        let r = r * pt;
        s.stroke_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, 2.0 * pt, color);
    }
    s.fill_rrect(cx - 2.5 * pt, cy - 2.5 * pt, 5.0 * pt, 5.0 * pt, 2.5 * pt, color);
}

fn draw_bluetooth(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    let (hw, h) = (0.9 * pt, 11.0 * pt);
    let line = |s: &mut Surface, x0: f32, y0: f32, x1: f32, y1: f32| {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let l = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (-dy / l * hw, dx / l * hw);
        icons::fill_poly(s, &[(x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)], color);
    };
    let w = 6.5 * pt;
    line(s, cx, cy - h, cx, cy + h);
    line(s, cx, cy - h, cx + w, cy - h * 0.45);
    line(s, cx + w, cy - h * 0.45, cx - w, cy + h * 0.45);
    line(s, cx - w, cy - h * 0.45, cx + w, cy + h * 0.45);
    line(s, cx + w, cy + h * 0.45, cx, cy + h);
}

fn draw_mirror(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    s.stroke_rrect(cx - 15.0 * pt, cy - 12.0 * pt, 20.0 * pt, 15.0 * pt, 2.5 * pt, 2.0 * pt, color);
    s.stroke_rrect(cx - 5.0 * pt, cy - 3.0 * pt, 20.0 * pt, 15.0 * pt, 2.5 * pt, 2.0 * pt, color);
}

fn draw_bell(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32, slashed: bool) {
    s.fill_rrect(cx - 8.0 * pt, cy - 11.0 * pt, 16.0 * pt, 18.0 * pt, 8.0 * pt, color);
    s.fill_rrect(cx - 11.0 * pt, cy + 2.0 * pt, 22.0 * pt, 6.0 * pt, 2.0 * pt, color);
    s.fill_rrect(cx - 3.0 * pt, cy + 9.0 * pt, 6.0 * pt, 4.0 * pt, 2.0 * pt, color);
    if slashed {
        icons::fill_poly(s, &[(cx - 13.0 * pt, cy - 12.0 * pt), (cx - 10.5 * pt, cy - 14.0 * pt), (cx + 13.0 * pt, cy + 14.0 * pt), (cx + 10.5 * pt, cy + 16.0 * pt)], color);
    }
}

fn draw_timer(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    let r = 14.0 * pt;
    s.stroke_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, 2.5 * pt, color);
    icons::fill_poly(s, &[(cx - 1.2 * pt, cy), (cx + 1.2 * pt, cy), (cx - 5.0 * pt, cy - 9.0 * pt), (cx - 7.0 * pt, cy - 8.0 * pt)], color);
}

fn draw_camera(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    s.fill_rrect(cx - 14.0 * pt, cy - 8.0 * pt, 28.0 * pt, 20.0 * pt, 4.0 * pt, color);
    s.fill_rrect(cx - 6.0 * pt, cy - 12.0 * pt, 12.0 * pt, 6.0 * pt, 2.0 * pt, color);
    let r = 6.0 * pt;
    s.fill_rrect(cx - r, cy - r + 2.0 * pt, 2.0 * r, 2.0 * r, r, rgba(30, 30, 34, 255));
}

/// The brightness sun: a disc with eight rays.
fn draw_sun(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    let r = 5.0 * pt;
    s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, color);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = a.sin_cos();
        let (r0, r1, hw) = (7.5 * pt, 11.0 * pt, 1.2 * pt);
        let pts = [
            (cx + cos * r0 - sin * hw, cy + sin * r0 + cos * hw),
            (cx + cos * r1 - sin * hw, cy + sin * r1 + cos * hw),
            (cx + cos * r1 + sin * hw, cy + sin * r1 - cos * hw),
            (cx + cos * r0 + sin * hw, cy + sin * r0 - cos * hw),
        ];
        icons::fill_poly(s, &pts, color);
    }
}

/// A speaker with two sound arcs.
fn draw_speaker(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    draw_speaker_waves(s, cx, cy, pt, color, 2);
}

/// A speaker body with `waves` (0..=3) sound arcs; `pt` is px per point.
pub(super) fn draw_speaker_waves(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32, waves: usize) {
    let (x, y) = (cx - 12.0 * pt, cy - 10.0 * pt);
    let body = [(x, y + 6.5 * pt), (x + 5.0 * pt, y + 6.5 * pt), (x + 11.0 * pt, y + 1.0 * pt), (x + 11.0 * pt, y + 19.0 * pt), (x + 5.0 * pt, y + 13.5 * pt), (x, y + 13.5 * pt)];
    icons::fill_poly(s, &body, color);
    let (ax, ay) = (x + 11.0 * pt, cy);
    for radius in [5.5 * pt, 10.0 * pt, 14.5 * pt].into_iter().take(waves.min(3)) {
        let thick = 1.8 * pt;
        for py in ((ay - radius - thick).max(0.0) as usize).max(s.y0)..((ay + radius + thick).ceil() as usize).min(s.h) {
            for px in (ax as usize)..((ax + radius + thick).ceil() as usize).min(s.w) {
                let (dx, dy) = (px as f32 + 0.5 - ax, py as f32 + 0.5 - ay);
                if dx < dy.abs() {
                    continue;
                }
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (thick / 2.0 - (d - radius).abs() + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let p = s.at(px, py);
                    *p = over(*p, scale(color, (cov * 255.0) as u32));
                }
            }
        }
    }
}

/// Pixel rectangle of the cluster, for tests and previews.
pub fn cluster_rect(ui: &Ui, w: i32) -> Rect {
    let (x, y, cw, ch) = cluster(ui, w);
    Rect { x: x as i32, y: y as i32, w: cw as i32, h: ch as i32 }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{settle, shell, swipe, unlock};
    use super::super::{AppId, Effect, Event, Shell, WifiAction};
    use bifrost_core::json::Json;
    use super::*;
    use crate::gfx::Surface;

    fn open_cc(sh: &mut Shell, t: f64) {
        swipe(sh, 1100.0, 5.0, 500.0, t);
        settle(sh, t + 110.0);
        assert!(sh.cc.is_shown(), "cc={}", sh.cc.describe());
    }

    fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let mut fx = sh.event(Event::Down(x, y), t);
        fx.extend(sh.event(Event::Up(x, y), t + 30.0));
        fx
    }

    fn centre(sh: &Shell, tile: Tile) -> (f32, f32) {
        let (x, y, w, h) = tile_rect(&sh.ui, sh.w, tile);
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn app_control_tiles_sit_under_the_modules_and_hit_test() {
        let sh = shell();
        let module_bottom = {
            let (_, y, _, h) = cluster(&sh.ui, sh.w);
            y + h
        };
        let (x0, y0, w0, h0) = app_tile_rect(&sh.ui, sh.w, 0);
        assert!(y0 >= module_bottom, "app tiles start below the built-in modules");
        // four to a row: the fifth control wraps to the second row, under the first
        let (x4, y4, _, _) = app_tile_rect(&sh.ui, sh.w, 4);
        assert!((x4 - x0).abs() < 0.5 && y4 > y0 + h0 / 2.0);
        let centre = |i: usize| {
            let (x, y, w, h) = app_tile_rect(&sh.ui, sh.w, i);
            (x + w / 2.0, y + h / 2.0)
        };
        let (cx, cy) = centre(1);
        assert_eq!(app_hit(&sh.ui, sh.w, 2, cx, cy), Some(1));
        assert_eq!(app_hit(&sh.ui, sh.w, 1, cx, cy), None, "a tile past the count does not answer");
        assert_eq!(app_hit(&sh.ui, sh.w, 3, x0 + w0 / 2.0, y0 - 1.0), None, "the gap above a row is not a tile");
        // a built-in module's centre is not an app tile
        let (wx, wy) = centre(&sh, Tile::Wifi);
        assert_eq!(app_hit(&sh.ui, sh.w, 12, wx, wy), None);
    }

    fn notes_candidates() -> Vec<AppControlEntry> {
        let control = super::super::app_controls::ManifestControl { kind: ControlKind::Toggle, title: "Quiet Hours".into(), symbol: "moon".into(), intent: "set-quiet".into() };
        vec![AppControlEntry { app_id: "is.olibuijr.notes".into(), identifier: control.identifier("is.olibuijr.notes"), control }]
    }

    #[test]
    fn gallery_module_row_hides_and_shows_a_module_and_saves_it() {
        let mut sh = shell();
        let wifi = DEFAULT_MODULES.iter().position(|&t| t == Tile::Wifi).unwrap();
        sh.cc_gallery_tap(wifi);
        assert!(!tile_visible(&sh.cc_slots(), Tile::Wifi));
        assert!(sh.prefs.get(MODULES_PREF).unwrap_or("").contains("-wifi"));
        sh.cc_gallery_tap(wifi);
        assert!(tile_visible(&sh.cc_slots(), Tile::Wifi));
        assert!(!sh.prefs.get(MODULES_PREF).unwrap_or("").contains("-wifi"));
    }

    #[test]
    fn gallery_app_control_add_remove_persists_and_comes_back() {
        let mut sh = shell();
        let candidates = notes_candidates();
        sh.toggle_app_control(&candidates[0]);
        assert_eq!(sh.cc.app_controls.keys(), vec!["is.olibuijr.notes.set-quiet"]);
        assert!(sh.prefs.get(MODULES_PREF).unwrap_or("").contains("app:is.olibuijr.notes.set-quiet"));
        // a fresh panel gets the control back from the saved list, once its app declares it
        sh.cc.app_controls = AppControls::new();
        sh.restore_app_controls(&candidates);
        assert_eq!(sh.cc.app_controls.keys(), vec!["is.olibuijr.notes.set-quiet"]);
        sh.toggle_app_control(&candidates[0]);
        assert!(sh.cc.app_controls.keys().is_empty());
        assert!(!sh.prefs.get(MODULES_PREF).unwrap_or("").contains("app:"));
    }

    #[test]
    fn edit_controls_opens_the_list_and_done_closes_it() {
        let mut sh = shell();
        let (x, y, w, h) = edit_button_rect(&sh.ui, sh.w, 0);
        assert!(sh.cc_edit_at(x + w / 2.0, y + h / 2.0));
        sh.cc_open_edit();
        assert!(sh.cc.editing);
        let (rx, ry, rw, rh) = gallery_row_rect(&sh.ui, sh.w, 0);
        assert_eq!(sh.cc_gallery_at(rx + rw / 2.0, ry + rh / 2.0), Some(0));
        sh.cc_gallery_tap(sh.cc_gallery_rows() - 1);
        assert!(!sh.cc.editing);
    }

    #[test]
    fn a_toggle_bounces_its_glyph_once_and_only_toggles_bounce() {
        let mut cc = ControlCenter::default();
        cc.start_toggle_bounce(Tile::Wifi, 1000.0);
        // a 0.6 s bounce peaks halfway: 1 + 0.15 * sin(pi / 2) = 1.15
        assert!((cc.glyph_scale(Tile::Wifi, 1300.0) - 1.15).abs() < 1e-4);
        assert_eq!(cc.glyph_scale(Tile::Bluetooth, 1300.0), 1.0, "only the toggle that changed bounces");
        assert!(cc.animating(), "frames keep coming while the bounce plays");
        // at 0 s into the bounce sin(0) = 0, so the glyph is at rest
        assert!((cc.glyph_scale(Tile::Wifi, 1000.0) - 1.0).abs() < 1e-6);
        // Brightness is a slider, not a toggle: it starts nothing and keeps the Wi-Fi bounce
        cc.start_toggle_bounce(Tile::Brightness, 2000.0);
        assert!(matches!(cc.bounce, Some((Tile::Wifi, _))));
        // after 0.7 s the bounce is done, so the glyph rests and the tick drops it
        assert!((cc.glyph_scale(Tile::Wifi, 1700.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cluster_sits_at_the_top_right_like_the_bifrost() {
        let sh = shell();
        let (x, y, w, _) = cluster(&sh.ui, sh.w);
        assert_eq!((x + w).round(), sh.w as f32 - sh.ui.pt(34.0).round());
        assert_eq!(y, sh.ui.pt(68.0));
        assert_eq!(w, sh.ui.pt(293.5));
        let (_, _, bw, bh) = tile_rect(&sh.ui, sh.w, Tile::Brightness);
        assert_eq!((bw, bh), (sh.ui.pt(65.5), sh.ui.pt(141.5)));
        for tile in [Tile::Airplane, Tile::Nearby, Tile::Wifi, Tile::Bluetooth, Tile::RotationLock, Tile::Mirror, Tile::Focus, Tile::Silent, Tile::Timer, Tile::Camera, Tile::Brightness, Tile::Volume, Tile::MediaPrev, Tile::MediaPlay, Tile::MediaNext] {
            let (cx, cy) = centre(&sh, tile);
            assert_eq!(hit(&sh.ui, sh.w, cx, cy), Some(tile));
        }
        let (mx, my, mw, mh) = media_tile(&sh.ui, sh.w);
        let (px, py) = centre(&sh, Tile::MediaPlay);
        assert!((px - mx - mw / 2.0).abs() < 1.0 && py > my + mh / 2.0 && py < my + mh, "play sits low in the media tile");
        let (rx, ry, ..) = tile_rect(&sh.ui, sh.w, Tile::RotationLock);
        let (lx, ly, ..) = tile_rect(&sh.ui, sh.w, Tile::Mirror);
        assert_eq!((rx, ry), (cluster(&sh.ui, sh.w).0, sh.ui.pt(68.0 + 152.0)));
        assert_eq!((lx - rx, ly), (sh.ui.pt(76.0), ry), "mirroring sits beside rotation lock");
        assert_eq!(hit(&sh.ui, sh.w, 10.0, 10.0), None);
    }

    #[test]
    fn the_progress_bar_sits_inside_the_plate_and_a_touch_gives_its_share() {
        let sh = shell();
        let plate = (100.0, 50.0, 300.0, 141.5);
        let (bx, by, bw, bh) = media_bar(plate, &sh.ui);
        assert!(bx > plate.0 && bx + bw < plate.0 + plate.2, "the bar is inset from both sides");
        assert!(by > plate.1 && by + bh < plate.1 + plate.3, "the bar is near the bottom edge");
        assert_eq!(scrub_fraction((0.0, 0.0, 200.0, 4.0), 50.0), 0.25);
        assert_eq!(scrub_fraction((0.0, 0.0, 200.0, 4.0), -10.0), 0.0);
        assert_eq!(scrub_fraction((0.0, 0.0, 200.0, 4.0), 900.0), 1.0);
        assert_eq!(scrub_fraction((0.0, 0.0, 0.0, 4.0), 10.0), 0.0, "an empty bar gives no share");
    }

    #[test]
    fn an_app_session_takes_the_media_tile_and_its_taps_go_to_the_app() {
        use super::super::services::{now_playing, SystemService};
        let mut sh = shell();
        let body = bifrost_core::json::parse(r#"{"title":"Song","artist":"Band","state":"playing","commands":{"next":"enabled","play":"enabled"}}"#).unwrap();
        let mut service = now_playing::NowPlaying;
        service.call(&mut sh, "app.music", "publish", &body, 0.0).expect("published");
        assert!(sh.media_active(), "the desktop media tile appears for an app session");
        let shown = now_playing::current().expect("the session is shown");
        assert_eq!(shown.content().map(|c| c.title.as_str()), Some("Song"));
        assert_eq!(shown.snapshot().state, PlaybackState::Playing);
        sh.media_control(Media::Next);
        assert!(now_playing::current().is_some(), "a tap keeps the session until the owner clears it");
        service.call(&mut sh, "app.music", "clear", &Json::Null, 0.0).expect("cleared");
        assert!(!sh.media_active() || sh.svc_active(), "without a session only the audio service can show");
    }

    #[test]
    fn focus_tile_toggles_and_persists_without_platform_effects() {
        let mut sh = shell();
        unlock(&mut sh);
        open_cc(&mut sh, 1000.0);
        let (x, y) = centre(&sh, Tile::Focus);
        assert!(tap(&mut sh, x, y, 2000.0).is_empty());
        assert!(sh.dnd());
        assert_eq!(sh.prefs.get("dnd"), Some("1"));
        assert!(sh.cc.is_shown());
        assert!(tap(&mut sh, x, y, 3000.0).is_empty());
        assert!(!sh.dnd());
        assert_eq!(sh.prefs.get("dnd"), Some("0"));
    }

    #[test]
    fn programmatic_overlays_open_and_replace_each_other() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.open_control_center(1000.0);
        settle(&mut sh, 1000.0);
        assert!(sh.cc.is_shown());
        sh.open_notification_center(2000.0);
        settle(&mut sh, 2000.0);
        assert!(sh.cc.is_hidden());
        assert!(sh.notify.center.is_shown());
        sh.open_control_center(3000.0);
        settle(&mut sh, 3000.0);
        assert!(sh.notify.center.is_hidden());
        assert!(sh.cc.is_shown());
        sh.event(Event::Power, 4000.0);
        sh.open_control_center(4100.0);
        sh.open_notification_center(4100.0);
        assert!(sh.cc.is_hidden());
        assert!(sh.notify.center.is_hidden());
    }

    #[test]
    fn swipe_down_from_the_top_right_reveals_and_follows_the_finger() {
        let mut sh = shell();
        unlock(&mut sh);
        assert!(sh.event(Event::Down(1100.0, 5.0), 1000.0).is_empty());
        assert_eq!(sh.cc.describe(), "dragging");
        sh.event(Event::Move(1100.0, 5.0 + sh.ui.pt(150.0)), 1050.0);
        assert!((sh.cc.progress(1050.0) - 0.5).abs() < 0.01);
        sh.event(Event::Up(1100.0, 5.0 + sh.ui.pt(150.0)), 1400.0);
        assert_eq!(sh.cc.describe(), "opening");
        assert!(sh.needs_frame());
        settle(&mut sh, 1400.0);
        assert!(sh.cc.is_shown());
        assert!(sh.describe().contains("cc=shown"));
    }

    #[test]
    fn short_drag_springs_back_and_other_edges_do_nothing() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.event(Event::Down(1100.0, 5.0), 1000.0);
        sh.event(Event::Move(1100.0, 40.0), 1300.0);
        sh.event(Event::Up(1100.0, 40.0), 1600.0);
        settle(&mut sh, 1600.0);
        assert!(sh.cc.is_hidden());
        swipe(&mut sh, 100.0, 5.0, 500.0, 2000.0);
        settle(&mut sh, 2110.0);
        assert!(sh.cc.is_hidden(), "top-left is not control center");
        swipe(&mut sh, 1100.0, 100.0, 500.0, 3000.0);
        settle(&mut sh, 3110.0);
        assert!(sh.cc.is_hidden(), "below the status bar is not control center");
    }

    #[test]
    fn not_available_on_the_lock_screen() {
        let mut sh = shell();
        swipe(&mut sh, 1100.0, 5.0, 500.0, 0.0);
        settle(&mut sh, 110.0);
        assert!(sh.cc.is_hidden());
    }

    #[test]
    fn bluetooth_hook_flips_the_radio_only_with_a_controller() {
        use crate::bare::bluetooth::{Cmd, State};
        let mut sh = shell();
        assert!(sh.toggle_bluetooth().is_empty(), "no controller, nothing to do");
        sh.info.bt.state = State::Off;
        assert_eq!(sh.toggle_bluetooth(), vec![Effect::Bluetooth(Cmd::Enable(true)), Effect::Bluetooth(Cmd::Scan)]);
        assert!(sh.info.bt.on());
        assert_eq!(sh.toggle_bluetooth(), vec![Effect::Bluetooth(Cmd::Enable(false))]);
    }

    #[test]
    fn toggles_emit_wifi_effects() {
        let mut sh = shell();
        unlock(&mut sh);
        open_cc(&mut sh, 1000.0);
        let (wx, wy) = centre(&sh, Tile::Wifi);
        assert_eq!(tap(&mut sh, wx, wy, 3000.0), vec![Effect::Wifi(WifiAction::Enable(true))]);
        assert!(sh.info.wifi_on);
        let (ax, ay) = centre(&sh, Tile::Airplane);
        assert_eq!(tap(&mut sh, ax, ay, 3100.0), vec![Effect::Wifi(WifiAction::Enable(false))]);
        assert!(sh.cc.airplane && !sh.info.wifi_on);
        assert_eq!(tap(&mut sh, ax, ay, 3200.0), vec![Effect::Wifi(WifiAction::Enable(true))]);
        assert!(!sh.cc.airplane && sh.info.wifi_on);
        // Bluetooth that was on goes off with Airplane Mode and comes back after it
        sh.info.bt.state = crate::bare::bluetooth::State::On;
        let off = tap(&mut sh, ax, ay, 3250.0);
        assert!(off.contains(&Effect::Bluetooth(crate::bare::bluetooth::Cmd::Enable(false))) && !sh.info.bt.on());
        let back = tap(&mut sh, ax, ay, 3260.0);
        assert!(back.contains(&Effect::Bluetooth(crate::bare::bluetooth::Cmd::Enable(true))) && sh.info.bt.on());
        // a press that slides off the circle does nothing
        sh.event(Event::Down(wx, wy), 3300.0);
        assert!(sh.event(Event::Up(wx, wy + 400.0), 3330.0).is_empty());
        assert!(sh.cc.is_shown());
    }

    #[test]
    fn sliders_follow_the_finger_vertically() {
        let mut sh = shell();
        unlock(&mut sh);
        open_cc(&mut sh, 1000.0);
        let (x, y, w, h) = tile_rect(&sh.ui, sh.w, Tile::Brightness);
        let fx = sh.event(Event::Down(x + w / 2.0, y + h * 0.75), 3000.0);
        assert_eq!(fx, vec![Effect::Brightness(0.25)]);
        assert_eq!(sh.event(Event::Move(x + 300.0, y - 50.0), 3010.0), vec![Effect::Brightness(1.0)]);
        assert!(sh.event(Event::Up(x + 300.0, y - 50.0), 3020.0).is_empty());
        assert!(sh.cc.is_shown(), "a slider drag never dismisses");
        let (x, y, w, h) = tile_rect(&sh.ui, sh.w, Tile::Volume);
        assert_eq!(tap(&mut sh, x + w / 2.0, y + h * 0.25, 4000.0), vec![Effect::Volume(0.75)]);
        assert_eq!(sh.cc.volume, 0.75);
    }

    #[test]
    fn tap_outside_or_swipe_up_dismisses() {
        let mut sh = shell();
        unlock(&mut sh);
        open_cc(&mut sh, 1000.0);
        tap(&mut sh, 200.0, 1200.0, 3000.0);
        assert_eq!(sh.cc.describe(), "closing");
        settle(&mut sh, 3030.0);
        assert!(sh.cc.is_hidden());
        open_cc(&mut sh, 5000.0);
        swipe(&mut sh, 200.0, 1500.0, 900.0, 6000.0);
        assert_eq!(sh.cc.describe(), "closing");
        settle(&mut sh, 6110.0);
        assert!(sh.cc.is_hidden());
        // a short pull up snaps back open
        open_cc(&mut sh, 7000.0);
        sh.event(Event::Down(200.0, 1500.0), 9000.0);
        sh.event(Event::Move(200.0, 1470.0), 9200.0);
        assert!(sh.cc.progress(9200.0) < 1.0);
        sh.event(Event::Up(200.0, 1470.0), 9500.0);
        settle(&mut sh, 9500.0);
        assert!(sh.cc.is_shown());
    }

    #[test]
    fn works_over_an_app_and_power_drops_it() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.open(AppId::Settings, 500.0);
        settle(&mut sh, 500.0);
        open_cc(&mut sh, 1000.0);
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 2000.0);
        let (x, y, _, h) = tile_rect(&sh.ui, sh.w, Tile::Mirror);
        let row = (y + h / 2.0) as usize * 1200;
        let beside = buf[row + (x - 20.0) as usize];
        assert!(buf[row + x as usize..row + (x + 200.0) as usize].iter().any(|&p| p != beside), "a tile shows over the frosted app");
        assert_eq!(sh.event(Event::Power, 3000.0), vec![Effect::ScreenOff]);
        assert!(sh.cc.is_hidden());
    }

    #[test]
    fn draws_every_phase() {
        let mut sh = shell();
        unlock(&mut sh);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| {
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        };
        sh.event(Event::Down(1100.0, 5.0), 1000.0);
        draw(&mut sh, 1000.0);
        sh.event(Event::Move(1100.0, 300.0), 1050.0);
        draw(&mut sh, 1050.0);
        sh.event(Event::Up(1100.0, 900.0), 1100.0);
        draw(&mut sh, 1200.0);
        settle(&mut sh, 1100.0);
        draw(&mut sh, 2000.0);
        sh.cc.airplane = true;
        sh.event(Event::Down(200.0, 1500.0), 3000.0);
        sh.event(Event::Move(200.0, 1000.0), 3100.0);
        draw(&mut sh, 3100.0);
    }
}

// ---------------------------------------------------------------------------
// Golden Gate desktop Control Center: a column of glass modules under the
// menu bar at the right (27-Golden-Gate-Files-Control-Center.png, 2x).
// Measured in pt: modules 139 wide in two columns 12 apart, 15 from the right
// edge; capsule rows 62 high, 14 apart; an icon disc of 36 in each capsule;
// full-width Display and Sound sliders with an 18 pt radius. Only modules the
// PC can really drive are laid out: Wi-Fi (with an adapter), Bluetooth (with a
// controller), Focus, Display (with a backlight) and Sound.

const DK_COL_PT: f32 = 139.0;
const DK_COL_GAP_PT: f32 = 12.0;
const DK_ROW_PT: f32 = 62.0;
const DK_ROW_GAP_PT: f32 = 14.0;
const DK_RIGHT_PT: f32 = 15.0;
/// gap under the menu bar, as menus (Golden Gate)
const DK_TOP_GAP_PT: f32 = 6.0;
const DK_DISC_PT: f32 = 36.0;
const DK_PAD_PT: f32 = 12.0;
const DK_SLIDER_R_PT: f32 = 18.0;
const DK_TRACK_H_PT: f32 = 4.0;

/// Which transport button of the Now Playing plate a click at `x` lands on.
fn desk_media_button((rx, _, rw, _): (f32, f32, f32, f32), x: f32) -> Tile {
    match (x - rx) / rw {
        f if f < 1.0 / 3.0 => Tile::MediaPrev,
        f if f < 2.0 / 3.0 => Tile::MediaPlay,
        _ => Tile::MediaNext,
    }
}

fn desk_backlight() -> bool {
    std::fs::read_dir("/sys/class/backlight").is_ok_and(|mut d| d.next().is_some())
}

/// The presenter's glass shape for a shell surface's reference material. Surfaces that float over the
/// desktop (menus, panels, Search, the Dock) have no window active state to follow, so they are `Chrome`;
/// the window-background families keep their own kinds.
pub(super) fn shell_glass_rect(req: &MaterialRequest) -> bifrost_render::glass::Rect {
    use bifrost_render::glass::Kind;
    let kind = match req.kind {
        MaterialKind::Sidebar => Kind::Sidebar,
        MaterialKind::Titlebar | MaterialKind::HeaderView => Kind::Titlebar,
        _ => Kind::Chrome,
    };
    let active = match req.state {
        MaterialState::FollowsWindowActiveState => None,
        MaterialState::Active => Some(true),
        MaterialState::Inactive => Some(false),
    };
    let c = req.corners();
    let detail = bifrost_render::glass::Detail {
        kind: req.kind.window_server_wire(),
        within_window: !req.for_window_server(),
        emphasized: req.emphasized,
        radii: [c.tl, c.tr, c.br, c.bl],
    };
    let r = c.tl.max(c.tr).max(c.br).max(c.bl);
    bifrost_render::glass::Rect { x: req.rect.x, y: req.rect.y, w: req.rect.w, h: req.rect.h, r, kind, active, detail: Some(detail) }
}

/// Declare a shell surface's material: the presenter's blur under it on the GPU path (nothing under
/// Reduce Transparency or on the CPU fallback). Returns the fill colour for the surface, which is the
/// named material tint (`MENU_TINT`), the solid material with Reduce Transparency, or the plain tint.
pub(super) fn request_shell_material(req: MaterialRequest) -> u32 {
    use super::ui::{MENU_SOLID, MENU_TINT};
    if bifrost_render::glass::gpu() && bifrost_render::appearance::reduce_transparency() {
        MENU_SOLID.now()
    } else if bifrost_render::glass::gpu() {
        bifrost_render::glass::push(shell_glass_rect(&req));
        bifrost_render::glass::tint(MENU_TINT.now())
    } else {
        MENU_TINT.now()
    }
}

/// A glass plate for a shell surface (see `request_shell_material`), filled with `alpha`.
pub(super) fn desk_glass(s: &mut Surface, surface: ShellSurface, (x, y, w, h): (f32, f32, f32, f32), r: f32, alpha: u32) {
    let fill = request_shell_material(surface.request((x, y, w, h), r));
    s.fill_rrect(x, y, w, h, r, scale(fill, alpha));
}

#[cfg(test)]
mod shell_material_tests {
    use super::*;

    #[test]
    fn shell_surfaces_map_to_presenter_kinds() {
        use bifrost_render::glass::Kind;
        for (surface, kind) in [
            (ShellSurface::SettingsSidebar, Kind::Sidebar),
            (ShellSurface::SettingsToolbar, Kind::Titlebar),
            (ShellSurface::Dock, Kind::Chrome),
            (ShellSurface::Search, Kind::Chrome),
            (ShellSurface::MenuBarMenu, Kind::Chrome),
        ] {
            let r = shell_glass_rect(&surface.request((0.0, 0.0, 10.0, 10.0), 4.0));
            assert_eq!(r.kind, kind, "{surface:?}");
        }
    }
}

impl Shell {
    /// Whether the media tile has something to show: an app's published session or the audio service's track.
    fn media_active(&self) -> bool {
        now_playing::current().is_some() || self.svc_active()
    }

    /// A transport tap on the media tile. The newest app session gets the command (play or pause follows its state);
    /// without one the audio service gets it.
    fn media_control(&mut self, target: Media) {
        if let Some(session) = now_playing::current() {
            let playing = session.snapshot().state == PlaybackState::Playing;
            if let Some(command) = now_playing::transport_for(target, playing) {
                now_playing::send(command);
            }
            return;
        }
        self.music_control(target);
    }

    /// Paints the media tile: the app's published session (artwork placeholder, title, artist and the transport row)
    /// when there is one, else what the audio service plays.
    fn draw_media_tile(&self, s: &mut Surface, rect: (f32, f32, f32, f32), alpha: u32, pressed: Option<Media>) {
        let Some(session) = now_playing::current() else {
            return self.draw_music_tile(s, rect, alpha, pressed);
        };
        let ui = &self.ui;
        let (x, y, w, h) = rect;
        let size = ui.pt(44.0);
        let r = Rect { x: (x + ui.pt(14.0)) as i32, y: (y + ui.pt(14.0)) as i32, w: size as i32, h: size as i32 };
        draw_scaled(s, &icons::placeholder(64, 0.0), r, ui.pt(5.0), alpha);
        let title = session.content().map_or("Now Playing".to_string(), |c| c.title.clone());
        let artist = session.content().map(|c| c.subtitle()).unwrap_or_default();
        let text_w = ui.pt(130.0);
        let tx = (x + ui.pt(14.0) + size + ui.pt(10.0)) as i32;
        let title = self.fit_chrome(&title, 15.0, text_w);
        let artist = self.fit_chrome(&artist, 13.0, text_w);
        self.label(&title, 15.0, Weight::Semibold, WHITE, |im| s.blit(im, tx, (y + ui.pt(16.0)) as i32, 255, alpha));
        self.label(&artist, 13.0, Weight::Regular, rgba(255, 255, 255, 150), |im| s.blit(im, tx, (y + ui.pt(38.0)) as i32, 255, alpha));
        let playing = session.snapshot().state == PlaybackState::Playing;
        draw_cc_controls(s, ui, (x, y, w, h), playing, scale(WHITE, alpha), pressed);
        // progress: the elapsed share of a finite item, advanced while it plays
        if let Some(shown) = now_playing::progress().filter(|p| p.duration_ms.is_some_and(|d| d > 0)) {
            let total = shown.duration_ms.unwrap_or(1) as f32;
            let (bx, by, bw, bh) = media_bar(rect, ui);
            let share = (shown.position_ms as f32 / total).clamp(0.0, 1.0);
            s.fill_rrect(bx, by, bw, bh, bh / 2.0, scale(rgba(255, 255, 255, 60), alpha));
            s.fill_rrect(bx, by, bw * share, bh, bh / 2.0, scale(WHITE, alpha));
        }
    }

    fn desk_wifi(&self) -> Option<(bool, Option<String>)> {
        let n = self.info.desktop_network.as_ref()?;
        n.wifi_available.then(|| (n.wifi_on, n.ssid.clone()))
    }

    /// The desktop modules and where they sit, (x, y, w, h) in px.
    pub(super) fn cc_desk_tiles(&self) -> Vec<(Tile, (f32, f32, f32, f32))> {
        let ui = &self.ui;
        let full = ui.pt(2.0 * DK_COL_PT + DK_COL_GAP_PT);
        let x0 = self.w as f32 - ui.pt(DK_RIGHT_PT) - full;
        let mut y = self.desktop_bar_height() as f32 + ui.pt(DK_TOP_GAP_PT);
        let mut capsules = vec![];
        if self.desk_wifi().is_some() {
            capsules.push(Tile::Wifi);
        }
        if self.info.bt.state != crate::bare::bluetooth::State::Unavailable {
            capsules.push(Tile::Bluetooth);
        }
        capsules.push(Tile::Focus);
        let mut out = vec![];
        for row in capsules.chunks(2) {
            for (i, &tile) in row.iter().enumerate() {
                out.push((tile, (x0 + i as f32 * ui.pt(DK_COL_PT + DK_COL_GAP_PT), y, ui.pt(DK_COL_PT), ui.pt(DK_ROW_PT))));
            }
            y += ui.pt(DK_ROW_PT + DK_ROW_GAP_PT);
        }
        if self.cc.backlight {
            out.push((Tile::Brightness, (x0, y, full, ui.pt(DK_ROW_PT))));
            y += ui.pt(DK_ROW_PT + DK_ROW_GAP_PT);
        }
        out.push((Tile::Volume, (x0, y, full, ui.pt(DK_ROW_PT))));
        // Now Playing: only with a player session; the transport buttons sit at thirds of the plate
        if self.media_active() {
            y += ui.pt(DK_ROW_PT + DK_ROW_GAP_PT);
            out.push((Tile::MediaPlay, (x0, y, full, ui.pt(BIG_PT))));
        }
        out
    }

    /// The panel's bounds in px, for the presenter's opaque regions.
    #[allow(dead_code)] // for desk_regions_now (desktop_bar.rs)
    pub(crate) fn cc_desk_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if self.cc.is_hidden() || !self.desk_active() {
            return None;
        }
        let t = self.cc_desk_tiles();
        let (x, y, ..) = t.first()?.1;
        let (lx, ly, lw, lh) = t.last()?.1;
        Some((x, y, lx + lw - x, ly + lh - y))
    }

    /// A slider's track in its module: (left, right, centre y) in px.
    fn desk_track(&self, (x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32, f32) {
        let ui = &self.ui;
        (x + ui.pt(DK_PAD_PT + 22.0), x + w - ui.pt(DK_PAD_PT + 22.0), y + h * 0.66)
    }

    fn desk_slider_at(&mut self, tile: Tile, rect: (f32, f32, f32, f32), x: f32) -> Vec<Effect> {
        let (l, r, _) = self.desk_track(rect);
        self.set_slider(tile, ((x - l) / (r - l)).clamp(0.0, 1.0))
    }

    /// Opens the desktop panel (a menu bar status item, Control Center).
    pub(crate) fn open_desk_control_center(&mut self, t: f64) {
        if !self.screen_on || self.is_locked() {
            return;
        }
        self.notify.center.hide();
        self.cc.backlight = desk_backlight();
        // a live overlay over the desktop, like the menu-bar menus: no snapshot
        self.cc.backdrop = None;
        self.cc.settle(0.0, true, t);
        self.dirty = true;
    }

    /// Pointer input while the desktop panel is up: a click outside closes it
    /// (and is consumed, as a menu does), modules act on release.
    pub(super) fn cc_desk_event(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        let tiles = self.cc_desk_tiles();
        let at = |x: f32, y: f32| tiles.iter().copied().find(|&(_, r)| inside(r, x, y));
        match ev {
            Event::Down(x, y) => match at(x, y) {
                Some((Tile::MediaPlay, r)) => {
                    let (bx, by, bw, bh) = media_bar(r, &self.ui);
                    let zone = (bx - self.ui.pt(6.0), by - self.ui.pt(6.0), bw + self.ui.pt(12.0), bh + self.ui.pt(12.0));
                    self.cc.touch = if now_playing::seek_length_ms().is_some() && inside(zone, x, y) {
                        Some(Touch::Scrub)
                    } else {
                        Some(Touch::Tile(desk_media_button(r, x)))
                    };
                    vec![]
                }
                Some((tile @ (Tile::Brightness | Tile::Volume), r)) => {
                    self.cc.touch = Some(Touch::Slider(tile));
                    self.desk_slider_at(tile, r, x)
                }
                Some((tile, _)) => {
                    self.cc.touch = Some(Touch::Tile(tile));
                    vec![]
                }
                None => {
                    self.cc.settle(1.0, false, t);
                    vec![]
                }
            },
            Event::Move(x, _) => match self.cc.touch {
                Some(Touch::Slider(tile)) => match tiles.iter().find(|(k, _)| *k == tile) {
                    Some(&(_, r)) => self.desk_slider_at(tile, r, x),
                    None => vec![],
                },
                _ => vec![],
            },
            Event::Up(x, y) => match self.cc.touch.take() {
                Some(Touch::Scrub) => {
                    let bar = tiles.iter().find(|(k, _)| *k == Tile::MediaPlay).map(|&(_, r)| media_bar(r, &self.ui));
                    if let (Some(bar), Some(total)) = (bar, now_playing::seek_length_ms()) {
                        now_playing::seek(now_playing::scrub_target_ms(total, scrub_fraction(bar, x)));
                    }
                    vec![]
                }
                Some(Touch::Tile(tile @ (Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext)))
                    if at(x, y).is_some_and(|(k, r)| k == Tile::MediaPlay && desk_media_button(r, x) == tile) =>
                {
                    self.activate_tile(tile, t)
                }
                Some(Touch::Tile(tile)) if at(x, y).map(|a| a.0) == Some(tile) => {
                    self.cc.start_toggle_bounce(tile, t);
                    match tile {
                        Tile::Wifi => {
                            let on = !self.desk_wifi().is_some_and(|w| w.0);
                            if let Some(n) = self.info.desktop_network.as_mut() {
                                n.wifi_on = on;
                            }
                            vec![Effect::Wifi(WifiAction::Enable(on))]
                        }
                        other => self.activate_tile(other, t),
                    }
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// Draws the screen as it was and the glass modules over it.
    pub(super) fn draw_desk_cc(&mut self, s: &mut Surface, t: f64) {
        use super::ui::{FIELD_BG, LABEL, LABEL_2, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, SYSTEM_BLUE, TRACK_C};
        let p = self.cc.progress(t).clamp(0.0, 1.0);
        if let Some((x, y, w, h)) = self.cc_desk_rect().filter(|_| bifrost_render::glass::gpu()) {
            // the panel floats over the live desktop: clear only its own rect for the glass
            let pad = self.ui.pt(8.0);
            s.fill_rect((x - pad).max(0.0) as usize, (y - pad).max(0.0) as usize, (w + 2.0 * pad) as usize, (h + 2.0 * pad) as usize, 0);
        }
        let alpha = (p * 255.0) as u32;
        let a = |c: u32| scale(c, alpha);
        let ui = &self.ui;
        let lift = (1.0 - p) * ui.pt(8.0);
        let pressed = match self.cc.touch {
            Some(Touch::Tile(k)) => Some(k),
            _ => None,
        };
        let label = LABEL.now();
        for (tile, (x, y, w, h)) in self.cc_desk_tiles() {
            let y = y - lift;
            match tile {
                Tile::Brightness | Tile::Volume => {
                    desk_glass(s, ShellSurface::ControlCentre, (x, y, w, h), ui.pt(DK_SLIDER_R_PT), alpha);
                    let name = if tile == Tile::Brightness { "Display" } else { "Sound" };
                    self.label(name, MAC_HEADLINE_PT, Weight::Semibold, label, |im| s.blit(im, (x + ui.pt(DK_PAD_PT + 4.0)) as i32, (y + ui.pt(10.0)) as i32, 255, alpha));
                    let (l, r, cy) = self.desk_track((x, y, w, h));
                    let v = if tile == Tile::Brightness { self.info.brightness } else { self.cc.volume };
                    let th = ui.pt(DK_TRACK_H_PT);
                    s.fill_rrect(l, cy - th / 2.0, r - l, th, th / 2.0, a(TRACK_C.now()));
                    s.fill_rrect(l, cy - th / 2.0, (r - l) * v, th, th / 2.0, a(label));
                    let ends = (x + ui.pt(DK_PAD_PT + 9.0), x + w - ui.pt(DK_PAD_PT + 9.0));
                    if tile == Tile::Brightness {
                        draw_sun(s, ends.0, cy, ui.s * 0.6, a(label));
                        draw_sun(s, ends.1, cy, ui.s * 0.8, a(label));
                    } else {
                        draw_speaker(s, ends.0, cy, ui.s * 0.6, a(label));
                        draw_speaker_waves(s, ends.1, cy, ui.s * 0.6, a(label), 3);
                    }
                }
                Tile::MediaPlay => {
                    desk_glass(s, ShellSurface::ControlCentre, (x, y, w, h), ui.pt(DK_SLIDER_R_PT), alpha);
                    self.draw_media_tile(s, (x, y, w, h), alpha, None);
                }
                _ => {
                    desk_glass(s, ShellSurface::ControlCentre, (x, y, w, h), h / 2.0, alpha);
                    let (on, title, sub) = match tile {
                        Tile::Wifi => {
                            let (on, ssid) = self.desk_wifi().unwrap_or((false, None));
                            (on, "Wi-Fi", if on { ssid.unwrap_or_else(|| "Not Connected".into()) } else { "Off".into() })
                        }
                        Tile::Bluetooth => {
                            let on = self.info.bt.on();
                            (on, "Bluetooth", if on { "On" } else { "Off" }.into())
                        }
                        _ => (self.dnd(), "Focus", if self.dnd() { "Do Not Disturb" } else { "Off" }.into()),
                    };
                    let d = ui.pt(DK_DISC_PT);
                    let (dx, dy) = (x + (h - d) / 2.0, y + (h - d) / 2.0);
                    let mut disc = if on { SYSTEM_BLUE } else { FIELD_BG.now() };
                    if pressed == Some(tile) {
                        disc = over(disc, rgba(128, 128, 128, 60));
                    }
                    s.fill_rrect(dx, dy, d, d, d / 2.0, a(disc));
                    let ink = a(if on { WHITE } else { label });
                    let (cx, cy) = (dx + d / 2.0, dy + d / 2.0);
                    let k = self.cc.glyph_scale(tile, t);
                    match tile {
                        Tile::Wifi => status::draw_wifi(s, cx - ui.pt(9.0) * k, cy - ui.pt(5.0) * k, ui.pt(9.0) * k, ink),
                        Tile::Bluetooth => draw_bluetooth(s, cx, cy, ui.s * 0.8 * k, ink),
                        _ => {
                            let radius = d * 0.3 * k;
                            let moon = super::glyphs::disc((cx, cy), radius);
                            let cut = super::glyphs::disc((cx + radius * 0.55, cy - radius * 0.4), radius * 0.85);
                            super::glyphs::paint(s, (cx - radius, cy - radius, 2.0 * radius, 2.0 * radius), ink, |x, y| moon(x, y).max(-cut(x, y)));
                        }
                    }
                    let tx = dx + d + ui.pt(8.0);
                    let room = x + w - tx - ui.pt(DK_PAD_PT);
                    let (f1, f2) = (self.fit_chrome(title, MAC_HEADLINE_PT, room), self.fit_chrome(&sub, MAC_SUBHEAD_PT, room));
                    let sub_ink = LABEL_2.now();
                    let h1 = self.label(&f1, MAC_HEADLINE_PT, Weight::Semibold, label, |im| im.h);
                    let h2 = self.label(&f2, MAC_SUBHEAD_PT, Weight::Regular, sub_ink, |im| im.h);
                    let top = y + (h - (h1 + h2) as f32) / 2.0;
                    self.label(&f1, MAC_HEADLINE_PT, Weight::Semibold, label, |im| s.blit(im, tx as i32, top as i32, 255, alpha));
                    self.label(&f2, MAC_SUBHEAD_PT, Weight::Regular, sub_ink, |im| s.blit(im, tx as i32, (top + h1 as f32) as i32, 255, alpha));
                }
            }
        }
    }
}

/// The modules the Control Centre can show, in the default order they have on the cluster. Media transport
/// buttons are not listed: they belong to the media tile.
pub const DEFAULT_MODULES: [Tile; 12] = [
    Tile::Airplane,
    Tile::Nearby,
    Tile::Wifi,
    Tile::Bluetooth,
    Tile::RotationLock,
    Tile::Mirror,
    Tile::Focus,
    Tile::Silent,
    Tile::Timer,
    Tile::Camera,
    Tile::Brightness,
    Tile::Volume,
];

/// The stable word a module is stored under in prefs. `None` for tiles that are not user modules.
pub fn module_key(tile: Tile) -> Option<&'static str> {
    Some(match tile {
        Tile::Airplane => "airplane",
        Tile::Nearby => "nearby",
        Tile::Wifi => "wifi",
        Tile::Bluetooth => "bluetooth",
        Tile::RotationLock => "rotation-lock",
        Tile::Mirror => "mirror",
        Tile::Focus => "focus",
        Tile::Silent => "silent",
        Tile::Timer => "timer",
        Tile::Camera => "camera",
        Tile::Brightness => "brightness",
        Tile::Volume => "volume",
        Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => return None,
    })
}

/// One row of the customised module list: which module, and whether the cluster shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModuleSlot {
    pub tile: Tile,
    pub shown: bool,
}

/// The default list: every module, in the default order, all shown.
pub fn default_slots() -> Vec<ModuleSlot> {
    DEFAULT_MODULES.iter().map(|&tile| ModuleSlot { tile, shown: true }).collect()
}

/// Reads the prefs string: comma-separated keys in display order, a leading `-` hides a module.
/// Unknown keys and repeats are dropped, and any module the string leaves out is appended, shown, in
/// its default position among the rest. An empty string gives the default list.
pub fn parse_slots(text: &str) -> Vec<ModuleSlot> {
    let mut out: Vec<ModuleSlot> = Vec::new();
    for raw in text.split(',') {
        let raw = raw.trim();
        let (shown, key) = match raw.strip_prefix('-') {
            Some(rest) => (false, rest),
            None => (true, raw),
        };
        let Some(tile) = DEFAULT_MODULES.iter().copied().find(|&t| module_key(t) == Some(key)) else { continue };
        if out.iter().any(|s| s.tile == tile) {
            continue;
        }
        out.push(ModuleSlot { tile, shown });
    }
    for &tile in &DEFAULT_MODULES {
        if !out.iter().any(|s| s.tile == tile) {
            out.push(ModuleSlot { tile, shown: true });
        }
    }
    out
}

/// Writes the list back in the form `parse_slots` reads.
pub fn format_slots(slots: &[ModuleSlot]) -> String {
    slots
        .iter()
        .filter_map(|s| module_key(s.tile).map(|k| if s.shown { k.to_string() } else { format!("-{k}") }))
        .collect::<Vec<_>>()
        .join(",")
}

/// The tiles the cluster draws, in order.
pub fn shown_tiles(slots: &[ModuleSlot]) -> Vec<Tile> {
    slots.iter().filter(|s| s.shown).map(|s| s.tile).collect()
}

/// The prefs key that holds the module list (`format_slots` text).
pub const MODULES_PREF: &str = "control_center_modules";

/// Whether the cluster draws and answers `tile` under the list `slots`. The media transport buttons belong to the
/// Now Playing tile and always show; a module the list hides does not.
pub fn tile_visible(slots: &[ModuleSlot], tile: Tile) -> bool {
    match module_key(tile) {
        None => true,
        Some(_) => slots.iter().find(|s| s.tile == tile).is_none_or(|s| s.shown),
    }
}

/// The prefs key that holds the modules that get their own menu bar item (comma-separated module keys, in the
/// order they appear in the bar). Empty or absent means none.
pub const MENU_BAR_PREF: &str = "control_center_menu_bar";

/// The prefs key for the battery percentage beside the battery menu bar item: `"1"` shows it, anything else hides it.
pub const BATTERY_PERCENT_PREF: &str = "battery_show_percent";

/// Reads the menu bar pref: the modules with their own status item, in order. Unknown keys, media tiles and repeats
/// are dropped.
pub fn parse_menu_bar_items(text: &str) -> Vec<Tile> {
    let mut out: Vec<Tile> = Vec::new();
    for key in text.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let Some(tile) = DEFAULT_MODULES.iter().copied().find(|&t| module_key(t) == Some(key)) else { continue };
        if !out.contains(&tile) {
            out.push(tile);
        }
    }
    out
}

/// Writes the menu bar items back in the form `parse_menu_bar_items` reads.
pub fn format_menu_bar_items(items: &[Tile]) -> String {
    let mut keys: Vec<&str> = Vec::new();
    for key in items.iter().filter_map(|&t| module_key(t)) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.join(",")
}

/// Whether a module has its own menu bar item.
pub fn has_menu_bar_item(items: &[Tile], tile: Tile) -> bool {
    items.contains(&tile)
}

/// Turns a module's own menu bar item on or off, appending it at the end when turned on.
pub fn set_menu_bar_item(items: &mut Vec<Tile>, tile: Tile, on: bool) {
    items.retain(|&t| t != tile);
    if on && module_key(tile).is_some() {
        items.push(tile);
    }
}

/// Whether the battery item shows its percentage, from the pref text.
pub fn battery_percent_shown(pref: Option<&str>) -> bool {
    pref.map(str::trim) == Some("1")
}

/// The name a module has in the Edit Controls list. Media tiles have none (they are not listed).
pub fn module_title(tile: Tile) -> &'static str {
    match tile {
        Tile::Airplane => "Airplane Mode",
        Tile::Nearby => "Nearby Sharing",
        Tile::Wifi => "Wi-Fi",
        Tile::Bluetooth => "Bluetooth",
        Tile::RotationLock => "Rotation Lock",
        Tile::Mirror => "Screen Mirroring",
        Tile::Focus => "Focus",
        Tile::Silent => "Silent Mode",
        Tile::Timer => "Timer",
        Tile::Camera => "Camera",
        Tile::Brightness => "Brightness",
        Tile::Volume => "Volume",
        Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => "",
    }
}

/// The modules pref text for the module list and the app controls on the panel (`join_pref`).
pub fn save_text(slots: &[ModuleSlot], app_keys: &[String]) -> String {
    join_pref(&format_slots(slots), app_keys)
}

/// Moves the module at `from` to `to` (both indices into the list). Out-of-range indices change nothing.
pub fn move_slot(slots: &mut Vec<ModuleSlot>, from: usize, to: usize) {
    if from >= slots.len() || to >= slots.len() {
        return;
    }
    let s = slots.remove(from);
    slots.insert(to, s);
}

#[cfg(test)]
mod module_list_tests {
    use super::*;

    #[test]
    fn empty_text_is_the_default_list_with_everything_shown() {
        let slots = parse_slots("");
        assert_eq!(slots, default_slots());
        assert_eq!(shown_tiles(&slots), DEFAULT_MODULES.to_vec());
    }

    #[test]
    fn stored_order_and_hidden_modules_are_read_back() {
        let slots = parse_slots("focus,-wifi,airplane");
        assert_eq!(slots[0], ModuleSlot { tile: Tile::Focus, shown: true });
        assert_eq!(slots[1], ModuleSlot { tile: Tile::Wifi, shown: false });
        assert_eq!(slots[2], ModuleSlot { tile: Tile::Airplane, shown: true });
        // modules left out follow in default order, shown
        assert_eq!(slots[3].tile, Tile::Nearby);
        assert_eq!(slots.len(), DEFAULT_MODULES.len());
        assert!(!shown_tiles(&slots).contains(&Tile::Wifi));
    }

    #[test]
    fn unknown_keys_and_repeats_are_dropped() {
        let slots = parse_slots("bogus,wifi,wifi,-wifi, camera ");
        assert_eq!(slots.iter().filter(|s| s.tile == Tile::Wifi).count(), 1);
        assert_eq!(slots[0], ModuleSlot { tile: Tile::Wifi, shown: true });
        assert_eq!(slots[1], ModuleSlot { tile: Tile::Camera, shown: true });
        assert_eq!(slots.len(), DEFAULT_MODULES.len());
    }

    #[test]
    fn format_and_parse_round_trip() {
        let mut slots = default_slots();
        move_slot(&mut slots, 6, 0);
        slots[2].shown = false;
        let text = format_slots(&slots);
        assert_eq!(parse_slots(&text), slots);
    }

    #[test]
    fn media_tiles_have_no_key_and_are_not_listed() {
        assert_eq!(module_key(Tile::MediaPlay), None);
        assert!(!DEFAULT_MODULES.iter().any(|t| matches!(t, Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext)));
        assert!(!format_slots(&default_slots()).contains("media"));
    }

    #[test]
    fn hidden_modules_do_not_show_and_media_always_does() {
        let slots = parse_slots("-wifi");
        assert!(!tile_visible(&slots, Tile::Wifi));
        assert!(tile_visible(&slots, Tile::Airplane));
        assert!(tile_visible(&slots, Tile::MediaPlay));
        assert!(tile_visible(&[], Tile::Wifi), "no list means every module shows");
    }

    #[test]
    fn move_slot_ignores_out_of_range_indices() {
        let mut slots = default_slots();
        let before = slots.clone();
        move_slot(&mut slots, 0, 99);
        move_slot(&mut slots, 99, 0);
        assert_eq!(slots, before);
        move_slot(&mut slots, 0, 2);
        assert_eq!(slots[2].tile, Tile::Airplane);
    }

    #[test]
    fn menu_bar_items_read_in_order_and_drop_unknowns() {
        let items = parse_menu_bar_items("wifi,bogus,media,wifi,focus,");
        assert_eq!(items, vec![Tile::Wifi, Tile::Focus]);
        assert!(parse_menu_bar_items("").is_empty());
    }

    #[test]
    fn menu_bar_items_round_trip_and_toggle() {
        let mut items = parse_menu_bar_items("focus,wifi");
        set_menu_bar_item(&mut items, Tile::Bluetooth, true);
        assert!(has_menu_bar_item(&items, Tile::Bluetooth));
        set_menu_bar_item(&mut items, Tile::Focus, false);
        assert!(!has_menu_bar_item(&items, Tile::Focus));
        let text = format_menu_bar_items(&items);
        assert_eq!(text, "wifi,bluetooth");
        assert_eq!(parse_menu_bar_items(&text), items);
        // media tiles have no menu bar item of their own
        set_menu_bar_item(&mut items, Tile::MediaPlay, true);
        assert!(!has_menu_bar_item(&items, Tile::MediaPlay));
    }

    #[test]
    fn battery_percent_pref_is_on_only_for_one() {
        assert!(battery_percent_shown(Some("1")));
        assert!(battery_percent_shown(Some(" 1 ")));
        assert!(!battery_percent_shown(Some("0")));
        assert!(!battery_percent_shown(None));
    }
}
