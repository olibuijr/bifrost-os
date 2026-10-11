//! The fullscreen player: the picture letterboxed on black and an bifrost-style
//! HUD (Done, title, skip and play/pause buttons, scrubber with elapsed and
//! remaining time) that fades in on a tap and out after three idle seconds.
//! Deildu and Files share it; the host owns the `Player` and lends it to
//! every call, so the view holds no decoder state of its own.
//!
//! The logic (time text, scrub math, the HUD timer, double-tap zones and
//! hit-testing) is plain functions and structs at the top, tested on the
//! host; the drawing and gesture glue follow.

use super::play::{PlayState, Player};
use super::{within, BLACK, RED};
use crate::gfx::{over, rgba, Image, Surface};
use crate::shell::glyphs;
use crate::shell::icons;
use crate::shell::ui::{centred, Ui, Weight};
use crate::media::{convert, DmaPicture, Picture, VideoLayer};
use crate::shell::Event;
use std::cell::{Cell, RefCell};
use std::f32::consts::TAU;

pub type Rect = (f32, f32, f32, f32);

/// Skip back / forward step, as on TV.
pub const BACK_MS: f64 = 10_000.0;
pub const FWD_MS: f64 = 30_000.0;
/// The HUD hides after this long without interaction while playing.
pub const HIDE_MS: f64 = 3000.0;
pub const FADE_MS: f64 = 200.0;
/// Two taps this close in the same side third skip.
pub const DOUBLE_MS: f64 = 300.0;
pub const RIPPLE_MS: f64 = 500.0;
/// A full-width drag across the picture covers at most this much video.
pub const DRAG_SPAN_MS: f64 = 600_000.0;
/// Keep seeks this far behind the downloaded edge of a growing file.
pub const DOWNLOAD_MARGIN_MS: f64 = 3000.0;

// evdev key codes
const KEY_ENTER: u16 = 28;
const KEY_SPACE: u16 = 57;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_PLAYPAUSE: u16 = 164;

// --- time text ---

pub use crate::shell::music::nowplaying::clock as fmt_clock;

/// `-m:ss` of what is left (rounded up so elapsed + remaining = length), or `--:--`.
pub fn fmt_remaining(pos_ms: f64, duration_ms: Option<f64>) -> String {
    match duration_ms {
        Some(d) => format!("-{}", fmt_clock(((d - pos_ms).max(0.0) / 1000.0).ceil() * 1000.0)),
        None => "--:--".into(),
    }
}

// --- scrub math ---

/// The video time under `x` on a track spanning `x0..x1`.
pub fn ms_at(x: f32, x0: f32, x1: f32, duration_ms: f64) -> f64 {
    if x1 <= x0 {
        return 0.0;
    }
    ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64 * duration_ms
}

/// The x of video time `ms` on a track spanning `x0..x1`.
pub fn x_at(ms: f64, x0: f32, x1: f32, duration_ms: f64) -> f32 {
    if duration_ms <= 0.0 {
        return x0;
    }
    x0 + (x1 - x0) * (ms / duration_ms).clamp(0.0, 1.0) as f32
}

/// The latest time a seek may land on: the whole video once the file is
/// complete (`download` None or done), else the downloaded share of it
/// less a margin. `download` is (fraction downloaded, done).
pub fn seek_limit(duration_ms: Option<f64>, download: Option<(f32, bool)>) -> f64 {
    match (duration_ms, download) {
        (d, None | Some((_, true))) => d.unwrap_or(f64::MAX),
        (Some(d), Some((frac, false))) => (d * frac.clamp(0.0, 1.0) as f64 - DOWNLOAD_MARGIN_MS).max(0.0),
        (None, Some((_, false))) => 0.0,
    }
}

/// `ms` held inside the video and the seekable range.
pub fn clamp_seek(ms: f64, duration_ms: Option<f64>, limit_ms: f64) -> f64 {
    ms.max(0.0).min(limit_ms).min(duration_ms.unwrap_or(f64::MAX))
}

/// Where a horizontal drag of `dx` px across a `w` px picture lands, from `start_ms`.
pub fn drag_target(start_ms: f64, dx: f32, w: f32, duration_ms: f64, limit_ms: f64) -> f64 {
    let span = duration_ms.min(DRAG_SPAN_MS);
    let ms = start_ms + (dx / w.max(1.0)) as f64 * span;
    clamp_seek(ms, Some(duration_ms), limit_ms.max(start_ms.min(duration_ms)))
}

// --- HUD timer ---

/// HUD visibility: a target (shown / hidden), a 200 ms fade towards it and
/// the idle timer that hides it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hud {
    target: bool,
    from: f32,
    t0: f64,
    last: f64,
}

impl Hud {
    /// Shown, fading in from nothing at `t`.
    pub fn new(t: f64) -> Hud {
        Hud { target: true, from: 0.0, t0: t, last: t }
    }

    /// Opacity 0..1 at `t`.
    pub fn alpha(&self, t: f64) -> f32 {
        let k = ((t - self.t0) / FADE_MS).clamp(0.0, 1.0) as f32;
        let to = if self.target { 1.0 } else { 0.0 };
        self.from + (to - self.from) * k
    }

    /// Shown or fading in (what a tap can hit).
    pub fn visible(&self) -> bool {
        self.target
    }

    /// True while fading.
    pub fn animating(&self, t: f64) -> bool {
        t - self.t0 < FADE_MS
    }

    /// Anything of it still on screen at `t`.
    pub fn showing(&self, t: f64) -> bool {
        self.target || self.alpha(t) > 0.0
    }

    fn set(&mut self, on: bool, t: f64) {
        if on != self.target {
            self.from = self.alpha(t);
            self.t0 = t;
            self.target = on;
        }
    }

    /// Shows it and restarts the idle timer.
    pub fn poke(&mut self, t: f64) {
        self.last = t;
        self.set(true, t);
    }

    /// Restarts the idle timer of a shown HUD (a touch on it).
    pub fn touch(&mut self, t: f64) {
        if self.target {
            self.last = t;
        }
    }

    pub fn hide(&mut self, t: f64) {
        self.set(false, t);
    }

    pub fn toggle(&mut self, t: f64) {
        if self.target {
            self.hide(t);
        } else {
            self.poke(t);
        }
    }

    /// Advances the idle timer: `hold` (paused, scrubbing, buffering, a
    /// finger down) keeps it from running out. True if it just hid.
    pub fn update(&mut self, t: f64, hold: bool) -> bool {
        if !self.target {
            return false;
        }
        if hold {
            self.last = t;
        } else if t - self.last >= HIDE_MS {
            self.hide(t);
            return true;
        }
        false
    }
}

// --- taps ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Left,
    Middle,
    Right,
}

/// The third of a `w` px wide picture that `x` falls in.
pub fn zone_of(x: f32, w: f32) -> Zone {
    if x < w / 3.0 {
        Zone::Left
    } else if x >= w * 2.0 / 3.0 {
        Zone::Right
    } else {
        Zone::Middle
    }
}

/// The skip a double tap in `z` makes, in ms.
pub fn zone_delta(z: Zone) -> Option<f64> {
    match z {
        Zone::Left => Some(-BACK_MS),
        Zone::Right => Some(FWD_MS),
        Zone::Middle => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// the tap may be the first of a double: decide when `expire` says so
    Wait,
    /// toggle the HUD now
    Toggle,
    /// double tap: skip in this zone
    Skip(Zone),
}

/// Tells single taps from double taps in the side thirds. A tap in the
/// middle third toggles at once; a side tap waits `DOUBLE_MS` for a second,
/// and further taps in that zone keep skipping.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Taps {
    pending: Option<(f64, Zone)>,
    chain: Option<(f64, Zone)>,
}

impl Taps {
    pub fn tap(&mut self, x: f32, w: f32, t: f64) -> Tap {
        let zone = zone_of(x, w);
        if zone == Zone::Middle {
            self.pending = None;
            self.chain = None;
            return Tap::Toggle;
        }
        if self.chain.is_some_and(|(t0, z)| z == zone && t - t0 <= DOUBLE_MS) {
            self.chain = Some((t, zone));
            self.pending = None;
            return Tap::Skip(zone);
        }
        if let Some((t0, z)) = self.pending.take() {
            if z == zone && t - t0 <= DOUBLE_MS {
                self.chain = Some((t, zone));
                return Tap::Skip(zone);
            }
        }
        self.chain = None;
        self.pending = Some((t, zone));
        Tap::Wait
    }

    /// True once a waiting single tap has outlived the double-tap window.
    pub fn expire(&mut self, t: f64) -> bool {
        match self.pending {
            Some((t0, _)) if t - t0 > DOUBLE_MS => {
                self.pending = None;
                true
            }
            _ => false,
        }
    }

    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }
}

// --- layout and hit-testing ---

/// What a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Close,
    PlayPause,
    Back,
    Forward,
    Scrubber,
    /// Retry (after an error) or Play again (after the end)
    Action,
    /// the picture, with or without the HUD over it
    Video,
}

/// Where everything sits for a `w` x `h` px surface at `s` px per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub s: f32,
    pub margin: f32,
    pub done: Rect,
    pub play: Rect,
    pub back: Rect,
    pub fwd: Rect,
    pub scrub: Rect,
    pub action: Rect,
    pub track_x0: f32,
    pub track_x1: f32,
    pub track_y: f32,
    pub top_h: f32,
    pub bottom_h: f32,
}

fn square(cx: f32, cy: f32, r: f32) -> Rect {
    (cx - r, cy - r, 2.0 * r, 2.0 * r)
}

impl Layout {
    pub fn new(w: i32, h: i32, s: f32) -> Layout {
        let pt = |v: f32| v * s;
        let (wf, hf) = (w as f32, h as f32);
        let (cx, cy) = (wf / 2.0, hf / 2.0);
        let margin = pt(24.0);
        let label_w = pt(70.0);
        let (track_x0, track_x1) = (margin + label_w, (wf - margin - label_w).max(margin + label_w + 1.0));
        let track_y = hf - pt(64.0);
        let spread = pt(120.0).min(wf / 4.0);
        Layout {
            w: wf,
            h: hf,
            s,
            margin,
            done: (margin, pt(20.0), pt(84.0), pt(36.0)),
            play: square(cx, cy, pt(48.0)),
            back: square(cx - spread, cy, pt(36.0)),
            fwd: square(cx + spread, cy, pt(36.0)),
            scrub: (track_x0 - pt(12.0), track_y - pt(22.0), track_x1 - track_x0 + pt(24.0), pt(44.0)),
            action: (cx - pt(80.0), cy + pt(84.0), pt(160.0), pt(44.0)),
            track_x0,
            track_x1,
            track_y,
            top_h: pt(120.0),
            bottom_h: pt(150.0),
        }
    }

    /// What a touch at (x, y) lands on. A hidden HUD takes no touches.
    pub fn hit(&self, x: f32, y: f32, hud: bool, state: &PlayState) -> Hit {
        if !hud {
            return Hit::Video;
        }
        let failed = matches!(state, PlayState::Error(_));
        if matches!(state, PlayState::Error(_) | PlayState::Ended) && within(x, y, self.action) {
            return Hit::Action;
        }
        if within(x, y, self.done) {
            return Hit::Close;
        }
        if within(x, y, self.scrub) {
            return Hit::Scrubber;
        }
        if !failed {
            if within(x, y, self.play) {
                return Hit::PlayPause;
            }
            if within(x, y, self.back) {
                return Hit::Back;
            }
            if within(x, y, self.fwd) {
                return Hit::Forward;
            }
        }
        Hit::Video
    }
}

// --- the view ---

/// What the host does after a touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Done: stop playing and leave
    Close,
    /// open the file again
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Hit(Hit),
    Scrubber,
    Drag { start_ms: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    grab: Grab,
}

#[derive(Clone, Debug, Default)]
struct Snap {
    pos: f64,
    dur: Option<f64>,
    state: PlayState,
}

/// Text rendered once per change.
#[derive(Default)]
struct Label {
    text: String,
    img: Option<Image>,
}

impl Label {
    fn set(&mut self, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32) {
        if self.img.is_none() || self.text != text {
            self.img = Some(ui.text(text, pt, weight, color));
            self.text = text.to_string();
        }
    }
}

const WHITE: u32 = crate::gfx::pack(255, 255, 255, 255);

pub struct PlayerView {
    w: i32,
    h: i32,
    stale: bool,
    layout: Layout,
    title: String,
    subtitle: String,
    title_img: Option<Image>,
    sub_img: Option<Image>,
    done_img: Option<Image>,
    retry_img: Option<Image>,
    again_img: Option<Image>,
    ten: Option<Image>,
    thirty: Option<Image>,
    rip_back: Option<Image>,
    rip_fwd: Option<Image>,
    elapsed: Label,
    remaining: Label,
    bubble: Label,
    percent: Label,
    error: Label,
    frame: Option<Image>,
    /// the picture when it is still in the decoder's buffer (GPU path); replaces `frame`
    gpu_frame: Option<DmaPicture>,
    /// `gpu_frame` converted for a frame the CPU draws: ((seq, w, h), picture)
    cpu_copy: RefCell<Option<((u64, i32, i32), Image)>>,
    /// the next `draw` is an overlay over a GPU video layer (set by `video_layer`)
    layer: Cell<bool>,
    hud: Hud,
    /// the HUD's clock starts with the first `tick`
    started: bool,
    taps: Taps,
    touch: Option<Touch>,
    /// the time being scrubbed to, while a finger drags
    scrub: Option<f64>,
    snap: Snap,
    /// (fraction downloaded, done) of a file still arriving
    download: Option<(f32, bool)>,
    ripple: Option<(Zone, f64)>,
    dirty: bool,
}

impl PlayerView {
    pub fn new(ui: &Ui, w: i32, h: i32, title: &str, subtitle: &str) -> PlayerView {
        let mut v = PlayerView {
            w,
            h,
            stale: false,
            layout: Layout::new(w, h, ui.s),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            title_img: None,
            sub_img: None,
            done_img: None,
            retry_img: None,
            again_img: None,
            ten: None,
            thirty: None,
            rip_back: None,
            rip_fwd: None,
            elapsed: Label::default(),
            remaining: Label::default(),
            bubble: Label::default(),
            percent: Label::default(),
            error: Label::default(),
            frame: None,
            gpu_frame: None,
            cpu_copy: RefCell::new(None),
            layer: Cell::new(false),
            hud: Hud::new(0.0),
            started: false,
            taps: Taps::default(),
            touch: None,
            scrub: None,
            snap: Snap::default(),
            download: None,
            ripple: None,
            dirty: true,
        };
        v.build_static(ui);
        v
    }

    fn build_static(&mut self, ui: &Ui) {
        let side = self.layout.margin + ui.pt(130.0);
        let max = (self.w as f32 - 2.0 * side).max(ui.pt(100.0));
        self.title_img = Some(ui.text(&ui.fit(&self.title, 17.0, max), 17.0, Weight::Semibold, WHITE));
        self.sub_img = (!self.subtitle.is_empty()).then(|| ui.text(&ui.fit(&self.subtitle, 13.0, max), 13.0, Weight::Regular, crate::gfx::pack(200, 200, 205, 255)));
        self.done_img = Some(ui.text("Done", 17.0, Weight::Semibold, WHITE));
        self.retry_img = Some(ui.text("Retry", 17.0, Weight::Semibold, BLACK));
        self.again_img = Some(ui.text("Play again", 17.0, Weight::Semibold, BLACK));
        self.ten = Some(ui.text("10", 13.0, Weight::Bold, WHITE));
        self.thirty = Some(ui.text("30", 13.0, Weight::Bold, WHITE));
        self.rip_back = Some(ui.text("-10 s", 17.0, Weight::Semibold, WHITE));
        self.rip_fwd = Some(ui.text("+30 s", 17.0, Weight::Semibold, WHITE));
    }

    /// Lays the view out for a new surface size (rotation, Split View); the
    /// next `tick` rebuilds the text and tells the player.
    pub fn set_size(&mut self, w: i32, h: i32) {
        if (w, h) != (self.w, self.h) {
            self.w = w;
            self.h = h;
            self.stale = true;
            self.dirty = true;
        }
    }

    /// Where everything sits (the preview example and tests read it).
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// True once the player has delivered a picture.
    pub fn has_frame(&self) -> bool {
        self.frame.is_some() || self.gpu_frame.is_some()
    }

    /// The picture for the GPU, when there is one and `eligible` (the shell can compose this
    /// frame): the next `draw` then paints only the HUD, transparent where the picture shows.
    pub fn video_layer(&self, eligible: bool) -> Option<VideoLayer> {
        self.layer.set(false);
        let p = self.gpu_frame.as_ref().filter(|_| eligible)?;
        let dst = crate::gpu::quad::fit_rect(p.width, p.height, (0.0, 0.0, self.w as f32, self.h as f32));
        self.layer.set(true);
        Some(VideoLayer { frame: p.clone(), dst })
    }

    pub fn state(&self) -> &PlayState {
        &self.snap.state
    }

    pub fn hud_visible(&self) -> bool {
        self.hud.visible()
    }

    /// The download of a growing file: (fraction, done); None for a whole file.
    pub fn set_download(&mut self, ui: &Ui, d: Option<(f32, bool)>) {
        if self.download == d {
            return;
        }
        self.download = d;
        if let Some((f, false)) = d {
            self.percent.set(ui, &format!("{}%", (f.clamp(0.0, 1.0) * 100.0) as u32), 13.0, Weight::Semibold, WHITE);
        }
        self.dirty = true;
    }

    fn limit(&self) -> f64 {
        seek_limit(self.snap.dur, self.download).max(self.snap.pos)
    }

    fn can_seek(&self) -> bool {
        self.snap.dur.is_some_and(|d| d > 0.0)
    }

    fn spinning(&self) -> bool {
        match self.snap.state {
            PlayState::Buffering => true,
            PlayState::Playing | PlayState::Idle => !self.has_frame(),
            _ => false,
        }
    }

    /// Hardware keys: space/enter toggle, left/right skip. False = not ours.
    pub fn key(&mut self, player: &mut dyn Player, code: u16, t: f64) -> bool {
        match code {
            KEY_SPACE | KEY_ENTER | KEY_PLAYPAUSE => player.toggle(),
            KEY_LEFT => {
                self.skip(player, -BACK_MS, t);
                self.ripple = Some((Zone::Left, t));
            }
            KEY_RIGHT => {
                self.skip(player, FWD_MS, t);
                self.ripple = Some((Zone::Right, t));
            }
            _ => return false,
        }
        self.hud.poke(t);
        self.dirty = true;
        true
    }

    fn skip(&mut self, player: &mut dyn Player, delta: f64, _t: f64) {
        if self.snap.dur.is_none() && delta > 0.0 {
            return;
        }
        let target = clamp_seek(self.snap.pos + delta, self.snap.dur, self.limit());
        player.seek_ms(target);
        self.snap.pos = target;
        self.dirty = true;
    }

    fn scrub_target(&self, x: f32) -> f64 {
        let d = self.snap.dur.unwrap_or(0.0);
        clamp_seek(ms_at(x, self.layout.track_x0, self.layout.track_x1, d), Some(d), self.limit())
    }

    fn toggle_hud(&mut self, t: f64) {
        if matches!(self.snap.state, PlayState::Error(_) | PlayState::Ended) {
            self.hud.poke(t);
        } else {
            self.hud.toggle(t);
        }
    }

    /// The control under the finger, for the pressed look.
    fn pressed(&self) -> Option<Hit> {
        match self.touch?.grab {
            Grab::Hit(h) if h != Hit::Video => Some(h),
            _ => None,
        }
    }

    pub fn touch(&mut self, ui: &Ui, player: &mut dyn Player, ev: Event, t: f64) -> Outcome {
        self.dirty = true;
        let mut out = Outcome::None;
        let slop = ui.pt(14.0);
        match ev {
            Event::Down(x, y) => {
                self.hud.touch(t);
                let hit = self.layout.hit(x, y, self.hud.visible(), &self.snap.state);
                let grab = if hit == Hit::Scrubber && self.can_seek() {
                    self.scrub = Some(self.scrub_target(x));
                    Grab::Scrubber
                } else {
                    Grab::Hit(hit)
                };
                self.touch = Some(Touch { x0: x, y0: y, grab });
            }
            Event::Move(x, y) => {
                if let Some(mut tc) = self.touch {
                    let (dx, dy) = (x - tc.x0, y - tc.y0);
                    match tc.grab {
                        Grab::Scrubber => self.scrub = Some(self.scrub_target(x)),
                        Grab::Drag { start_ms } => {
                            let d = self.snap.dur.unwrap_or(0.0);
                            self.scrub = Some(drag_target(start_ms, dx, self.layout.w, d, self.limit()));
                        }
                        Grab::Hit(Hit::Video) => {
                            if self.can_seek() && dx.abs() > slop && dx.abs() > dy.abs() {
                                let start_ms = self.snap.pos;
                                tc.grab = Grab::Drag { start_ms };
                                self.touch = Some(tc);
                                self.taps = Taps::default();
                                let d = self.snap.dur.unwrap_or(0.0);
                                self.scrub = Some(drag_target(start_ms, dx, self.layout.w, d, self.limit()));
                            }
                        }
                        Grab::Hit(_) => {
                            if dx.abs() > slop || dy.abs() > slop {
                                self.touch = None;
                            }
                        }
                    }
                }
            }
            Event::Up(x, y) => {
                self.hud.touch(t);
                if let Some(tc) = self.touch.take() {
                    match tc.grab {
                        Grab::Scrubber | Grab::Drag { .. } => {
                            if let Some(ms) = self.scrub.take() {
                                player.seek_ms(ms);
                                self.snap.pos = ms;
                            }
                        }
                        Grab::Hit(h) => {
                            if h == self.layout.hit(x, y, self.hud.visible(), &self.snap.state) {
                                out = self.activate(player, h, x, t);
                            }
                        }
                    }
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {}
        }
        self.refresh_labels(ui);
        out
    }

    fn activate(&mut self, player: &mut dyn Player, hit: Hit, x: f32, t: f64) -> Outcome {
        match hit {
            Hit::Close => return Outcome::Close,
            Hit::PlayPause => player.toggle(),
            Hit::Back => self.skip(player, -BACK_MS, t),
            Hit::Forward => self.skip(player, FWD_MS, t),
            Hit::Action => match self.snap.state {
                PlayState::Error(_) => return Outcome::Retry,
                PlayState::Ended => {
                    player.seek_ms(0.0);
                    player.play();
                    self.snap.pos = 0.0;
                }
                _ => {}
            },
            Hit::Scrubber => {}
            Hit::Video => match self.taps.tap(x, self.layout.w, t) {
                Tap::Wait => {}
                Tap::Toggle => self.toggle_hud(t),
                Tap::Skip(z) => {
                    if let Some(d) = zone_delta(z) {
                        self.skip(player, d, t);
                        self.ripple = Some((z, t));
                    }
                }
            },
        }
        Outcome::None
    }

    fn relayout(&mut self, ui: &Ui, player: &mut dyn Player) {
        self.stale = false;
        self.layout = Layout::new(self.w, self.h, ui.s);
        self.build_static(ui);
        player.set_viewport(self.w, self.h);
        // a paused picture gets no new frame: fit the old one to the new box
        if let Some(f) = &self.frame {
            let k = (self.w as f32 / f.w as f32).min(self.h as f32 / f.h as f32);
            if (k - 1.0).abs() > 0.02 {
                let (nw, nh) = (((f.w as f32 * k) as usize).max(1), ((f.h as f32 * k) as usize).max(1));
                self.frame = Some(f.resized(nw, nh));
            }
        }
    }

    fn refresh_labels(&mut self, ui: &Ui) {
        let dur = self.snap.dur;
        let shown = self.scrub.unwrap_or(self.snap.pos);
        self.elapsed.set(ui, &fmt_clock(shown), 13.0, Weight::Medium, WHITE);
        self.remaining.set(ui, &fmt_remaining(shown, dur), 13.0, Weight::Medium, WHITE);
        if let Some(ms) = self.scrub {
            self.bubble.set(ui, &fmt_clock(ms), 17.0, Weight::Semibold, WHITE);
        }
        if let PlayState::Error(e) = &self.snap.state {
            self.error.set(ui, &ui.fit(&format!("Playback failed: {e}"), 15.0, (self.w as f32 - 2.0 * ui.pt(40.0)).max(ui.pt(100.0))), 15.0, Weight::Regular, RED);
        }
    }

    /// Polls the player and advances the HUD, the double-tap timer and the
    /// ripple; true when the picture needs redrawing. With the HUD hidden
    /// only a new picture (or the spinner) redraws: a changing time label is
    /// not drawn, so it costs nothing.
    pub fn tick(&mut self, ui: &Ui, player: &mut dyn Player, t: f64) -> bool {
        let mut dirty = std::mem::take(&mut self.dirty);
        if !self.started {
            self.started = true;
            self.hud = Hud::new(t);
            dirty = true;
        }
        if self.stale {
            self.relayout(ui, player);
            dirty = true;
        }
        match player.picture() {
            Some(Picture::Rgba(f)) => {
                self.frame = Some(f);
                self.gpu_frame = None;
                dirty = true;
            }
            Some(Picture::Gpu(p)) => {
                self.gpu_frame = Some(p);
                self.frame = None;
                dirty = true;
            }
            None => {}
        }
        let state = player.state();
        if state != self.snap.state {
            if matches!(state, PlayState::Error(_) | PlayState::Ended) {
                self.hud.poke(t);
            }
            self.snap.state = state;
            dirty = true;
        }
        let (pos, dur) = (player.position_ms(), player.duration_ms());
        let d = dur.unwrap_or(0.0);
        let moved = fmt_clock(pos) != fmt_clock(self.snap.pos)
            || dur != self.snap.dur
            || x_at(pos, 0.0, self.layout.track_x1 - self.layout.track_x0, d).round() != x_at(self.snap.pos, 0.0, self.layout.track_x1 - self.layout.track_x0, d).round();
        self.snap.pos = pos;
        self.snap.dur = dur;
        if moved && self.hud.showing(t) {
            dirty = true;
        }
        self.refresh_labels(ui);
        let hold = self.scrub.is_some() || self.touch.is_some() || self.snap.state != PlayState::Playing;
        dirty |= self.hud.update(t, hold) | self.hud.animating(t);
        if self.taps.expire(t) {
            self.toggle_hud(t);
            dirty = true;
        }
        if let Some((_, t0)) = self.ripple {
            dirty = true;
            if t - t0 > RIPPLE_MS {
                self.ripple = None;
            }
        }
        dirty || self.spinning()
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        let (w, h) = (self.w, self.h);
        if self.layer.replace(false) {
            // the GPU draws the picture (and the black bars) under this frame
            s.fill_rect(0, 0, w as usize, h as usize, 0);
        } else {
            s.fill_rect(0, 0, w as usize, h as usize, BLACK);
            if let Some(f) = &self.frame {
                s.blit(f, (w - f.w as i32) / 2, (h - f.h as i32) / 2, 255, 255);
            } else if let Some(p) = &self.gpu_frame {
                let mut copy = self.cpu_copy.borrow_mut();
                let key = (p.seq, w, h);
                if copy.as_ref().map(|c| c.0) != Some(key) {
                    let (ow, oh) = convert::fit(p.width, p.height, w.max(2) as u32, h.max(2) as u32);
                    *copy = Some((key, convert::dma_to_rgba(p, ow, oh, convert::matrix_for(p.height), 3)));
                }
                if let Some((_, f)) = copy.as_ref() {
                    s.blit(f, (w - f.w as i32) / 2, (h - f.h as i32) / 2, 255, 255);
                }
            }
        }
        self.draw_ripple(s, t);
        let a = self.hud.alpha(t);
        if a > 0.0 {
            self.draw_hud(s, ui, a);
        }
        if self.spinning() {
            spinner(s, (w as f32 / 2.0, h as f32 / 2.0), ui.pt(22.0), ui.pt(3.0), t);
        }
    }

    fn draw_ripple(&self, s: &mut Surface, t: f64) {
        let Some((zone, t0)) = self.ripple else { return };
        let k = ((t - t0) / RIPPLE_MS).clamp(0.0, 1.0) as f32;
        let third = self.layout.w / 3.0;
        let cx = if zone == Zone::Left { third / 2.0 } else { self.layout.w - third / 2.0 };
        let cy = self.layout.h / 2.0;
        let r = third * (0.35 + 0.65 * k);
        s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, rgba(255, 255, 255, (70.0 * (1.0 - k)) as u8));
        if let Some(img) = if zone == Zone::Left { &self.rip_back } else { &self.rip_fwd } {
            s.blit(img, cx as i32 - img.w as i32 / 2, cy as i32 - img.h as i32 / 2, 255, ((1.0 - k) * 255.0) as u32);
        }
    }

    fn draw_hud(&self, s: &mut Surface, ui: &Ui, a: f32) {
        let l = &self.layout;
        let al = (a * 255.0).round().clamp(0.0, 255.0) as u32;
        let white = |k: u32| rgba(255, 255, 255, (k * al / 255).min(255) as u8);
        let dim = |h: Hit| if self.pressed() == Some(h) { 140 } else { 255 };
        let pt = |v: f32| v * l.s;
        gradient(s, 0, l.top_h as i32, 170.0, 0.0, a);
        gradient(s, (l.h - l.bottom_h) as i32, l.h as i32, 0.0, 190.0, a);

        // top: Done, title, download
        let (dx, dy, dw, dh) = l.done;
        s.fill_rrect(dx, dy, dw, dh, dh / 2.0, rgba(70, 70, 74, (190 * al / 255) as u8));
        if let Some(img) = &self.done_img {
            s.blit(img, (dx + (dw - img.w as f32) / 2.0) as i32, (dy + (dh - img.h as f32) / 2.0) as i32, 255, al * dim(Hit::Close) / 255);
        }
        let mid_y = dy + dh / 2.0;
        if let Some(img) = &self.title_img {
            let y = if self.sub_img.is_some() { mid_y - img.h as f32 * 0.9 } else { mid_y - img.h as f32 / 2.0 };
            centred(s, img, (l.w / 2.0) as i32, y as i32);
            if let Some(sub) = &self.sub_img {
                centred(s, sub, (l.w / 2.0) as i32, (y + img.h as f32 * 0.95) as i32);
            }
        }
        if let (Some((frac, false)), Some(txt)) = (self.download, &self.percent.img) {
            let tx = l.w - l.margin - txt.w as f32;
            s.blit(txt, tx as i32, (mid_y - txt.h as f32 / 2.0) as i32, 255, al);
            let (r, th) = (pt(9.0), pt(2.5));
            let c = (tx - pt(10.0) - r, mid_y);
            glyphs::paint(s, bbox(c, r + th), white(80), glyphs::ring(c, r, th));
            if frac > 0.005 {
                glyphs::paint(s, bbox(c, r + th), white(255), glyphs::arc(c, r, th, 0.0, (TAU * frac.min(0.999)).max(0.05)));
            }
        }

        // centre
        let failed = matches!(self.snap.state, PlayState::Error(_));
        if failed {
            if let Some(img) = &self.error.img {
                s.blit(img, (l.w / 2.0) as i32 - img.w as i32 / 2, (l.h / 2.0 - pt(16.0)) as i32, 255, al);
            }
        } else {
            let c = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
            if let Some(ten) = &self.ten {
                skip_icon(s, c(l.back), pt(25.0), pt(3.0), true, white(dim(Hit::Back)), ten, al * dim(Hit::Back) / 255);
            }
            if let Some(thirty) = &self.thirty {
                skip_icon(s, c(l.fwd), pt(25.0), pt(3.0), false, white(dim(Hit::Forward)), thirty, al * dim(Hit::Forward) / 255);
            }
            if !self.spinning() {
                let (px, py) = c(l.play);
                let col = white(dim(Hit::PlayPause));
                if self.snap.state == PlayState::Playing {
                    pause_icon(s, (px, py), pt(26.0), col);
                } else {
                    play_icon(s, (px, py), pt(28.0), col);
                }
            }
        }
        let action = match self.snap.state {
            PlayState::Error(_) => self.retry_img.as_ref(),
            PlayState::Ended => self.again_img.as_ref(),
            _ => None,
        };
        if let Some(img) = action {
            let (x, y, w, h) = l.action;
            s.fill_rrect(x, y, w, h, pt(10.0), white(dim(Hit::Action)));
            s.blit(img, (x + (w - img.w as f32) / 2.0) as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, al);
        }

        // bottom: scrubber
        let dur = self.snap.dur.unwrap_or(0.0);
        let shown = self.scrub.unwrap_or(self.snap.pos);
        let scrubbing = self.scrub.is_some();
        let (x0, x1, y) = (l.track_x0, l.track_x1, l.track_y);
        let th = pt(if scrubbing { 9.0 } else { 5.0 });
        s.fill_rrect(x0, y - th / 2.0, x1 - x0, th, th / 2.0, white(70));
        let buffered = match self.download {
            Some((f, false)) => x0 + (x1 - x0) * f.clamp(0.0, 1.0),
            _ => x1,
        };
        s.fill_rrect(x0, y - th / 2.0, buffered - x0, th, th / 2.0, white(120));
        let px = x_at(shown, x0, x1, dur);
        if px > x0 {
            s.fill_rrect(x0, y - th / 2.0, px - x0, th, th / 2.0, white(255));
        }
        let tr = pt(if scrubbing { 10.0 } else { 7.0 });
        if self.can_seek() {
            s.fill_rrect(px - tr, y - tr, 2.0 * tr, 2.0 * tr, tr, white(255));
        }
        if let Some(img) = &self.elapsed.img {
            s.blit(img, l.margin as i32, (y - img.h as f32 / 2.0) as i32, 255, al);
        }
        if let Some(img) = &self.remaining.img {
            s.blit(img, (l.w - l.margin) as i32 - img.w as i32, (y - img.h as f32 / 2.0) as i32, 255, al);
        }
        if let (true, Some(img)) = (scrubbing, &self.bubble.img) {
            let (bw, bh) = (img.w as f32 + pt(24.0), pt(34.0));
            let bx = (px - bw / 2.0).clamp(l.margin, (l.w - l.margin - bw).max(l.margin));
            let by = y - pt(34.0) - bh;
            s.fill_rrect(bx, by, bw, bh, bh / 2.0, rgba(40, 40, 44, (235 * al / 255) as u8));
            s.blit(img, (bx + (bw - img.w as f32) / 2.0) as i32, (by + (bh - img.h as f32) / 2.0) as i32, 255, al);
        }
        let _ = ui;
    }
}

// --- drawing helpers ---

fn bbox((cx, cy): (f32, f32), r: f32) -> Rect {
    (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0)
}

/// Black fading from alpha `a0` at row `y0` to `a1` at row `y1` (0..255), times `k`.
fn gradient(s: &mut Surface, y0: i32, y1: i32, a0: f32, a1: f32, k: f32) {
    let span = (y1 - y0).max(1) as f32;
    for y in y0.max(s.y0 as i32)..y1.min(s.h as i32) {
        let f = (y - y0) as f32 / span;
        let al = ((a0 + (a1 - a0) * f) * k).clamp(0.0, 255.0) as u32;
        if al == 0 {
            continue;
        }
        let c = al << 24;
        let base = (y as usize - s.y0) * s.stride;
        for d in &mut s.px[base..base + s.w] {
            *d = over(*d, c);
        }
    }
}

fn play_icon(s: &mut Surface, (cx, cy): (f32, f32), r: f32, color: u32) {
    icons::polygon(s, &[(cx - r * 0.7, cy - r), (cx - r * 0.7, cy + r), (cx + r * 0.95, cy)], color);
}

fn pause_icon(s: &mut Surface, (cx, cy): (f32, f32), r: f32, color: u32) {
    let (bw, gap) = (r * 0.55, r * 0.5);
    s.fill_rrect(cx - gap / 2.0 - bw, cy - r, bw, 2.0 * r, bw * 0.25, color);
    s.fill_rrect(cx + gap / 2.0, cy - r, bw, 2.0 * r, bw * 0.25, color);
}

/// A circular arrow open at the top with its number inside; `back` mirrors it.
#[allow(clippy::too_many_arguments)]
fn skip_icon(s: &mut Surface, c: (f32, f32), r: f32, t: f32, back: bool, color: u32, label: &Image, label_alpha: u32) {
    let gap = 0.6f32;
    glyphs::paint(s, bbox(c, r + t), color, glyphs::arc(c, r, t, gap, TAU - gap));
    let m = if back { -1.0 } else { 1.0 };
    let a = TAU - gap;
    let p = (c.0 + m * r * a.sin(), c.1 - r * a.cos());
    let tan = (m * a.cos(), a.sin());
    let rad = (m * a.sin(), -a.cos());
    let (hl, hw) = (t * 2.4, t * 1.9);
    let tip = (p.0 + tan.0 * hl, p.1 + tan.1 * hl);
    let b1 = (p.0 + rad.0 * hw, p.1 + rad.1 * hw);
    let b2 = (p.0 - rad.0 * hw, p.1 - rad.1 * hw);
    icons::polygon(s, &[tip, b1, b2], color);
    s.blit(label, c.0 as i32 - label.w as i32 / 2, c.1 as i32 - label.h as i32 / 2, 255, label_alpha);
}

fn spinner(s: &mut Surface, c: (f32, f32), r: f32, t: f32, now: f64) {
    glyphs::paint(s, bbox(c, r + t), rgba(255, 255, 255, 60), glyphs::ring(c, r, t));
    let a0 = ((now / 1000.0).fract() as f32) * TAU;
    let a1 = (a0 + 1.2 * std::f32::consts::PI).rem_euclid(TAU);
    glyphs::paint(s, bbox(c, r + t), WHITE, glyphs::arc(c, r, t, a0, a1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Surface;
    use crate::shell::tests::ui;

    #[test]
    fn clock_and_remaining_text() {
        assert_eq!(fmt_clock(0.0), "0:00");
        assert_eq!(fmt_clock(65_999.0), "1:05");
        assert_eq!(fmt_clock(3_599_000.0), "59:59");
        assert_eq!(fmt_clock(3_600_000.0), "1:00:00");
        assert_eq!(fmt_clock(7_384_000.0), "2:03:04");
        assert_eq!(fmt_clock(-5.0), "0:00");
        assert_eq!(fmt_clock(f64::NAN), "0:00");
        assert_eq!(fmt_remaining(0.0, Some(65_000.0)), "-1:05");
        assert_eq!(fmt_remaining(1_500.0, Some(65_000.0)), "-1:04");
        assert_eq!(fmt_remaining(70_000.0, Some(65_000.0)), "-0:00");
        assert_eq!(fmt_remaining(0.0, Some(7_384_000.0)), "-2:03:04");
        assert_eq!(fmt_remaining(5.0, None), "--:--");
    }

    #[test]
    fn scrub_math_maps_and_clamps() {
        let (x0, x1, d) = (100.0, 1100.0, 600_000.0);
        assert_eq!(ms_at(100.0, x0, x1, d), 0.0);
        assert_eq!(ms_at(600.0, x0, x1, d), 300_000.0);
        assert_eq!(ms_at(5000.0, x0, x1, d), d);
        assert_eq!(ms_at(-5.0, x0, x1, d), 0.0);
        assert_eq!(ms_at(5.0, 10.0, 10.0, d), 0.0);
        assert_eq!(x_at(300_000.0, x0, x1, d), 600.0);
        assert_eq!(x_at(9e9, x0, x1, d), x1);
        assert_eq!(x_at(1.0, x0, x1, 0.0), x0);
        // whole file: the full length; growing: the downloaded share less the margin
        assert_eq!(seek_limit(Some(d), None), d);
        assert_eq!(seek_limit(Some(d), Some((0.2, true))), d);
        assert_eq!(seek_limit(Some(d), Some((0.5, false))), 300_000.0 - DOWNLOAD_MARGIN_MS);
        assert_eq!(seek_limit(Some(d), Some((0.0, false))), 0.0);
        assert_eq!(seek_limit(None, Some((0.5, false))), 0.0);
        assert_eq!(seek_limit(None, None), f64::MAX);
        assert_eq!(clamp_seek(-1.0, Some(d), d), 0.0);
        assert_eq!(clamp_seek(d + 5.0, Some(d), d), d);
        assert_eq!(clamp_seek(500_000.0, Some(d), 100_000.0), 100_000.0);
        assert_eq!(clamp_seek(50.0, None, f64::MAX), 50.0);
        // dragging half the width moves half of the (capped) span
        assert_eq!(drag_target(100_000.0, 600.0, 1200.0, d, d), 100_000.0 + 300_000.0);
        assert_eq!(drag_target(100_000.0, -9999.0, 1200.0, d, d), 0.0);
        assert_eq!(drag_target(100_000.0, 1200.0, 1200.0, 3_600_000.0, 200_000.0), 200_000.0);
        // never below where it already is, even past the download edge
        assert_eq!(drag_target(250_000.0, 100.0, 1200.0, d, 200_000.0), 250_000.0);
    }

    #[test]
    fn hud_fades_and_hides_after_idle_but_never_while_held() {
        let mut h = Hud::new(0.0);
        assert!(h.visible() && h.alpha(0.0) == 0.0 && (h.alpha(100.0) - 0.5).abs() < 1e-6 && h.alpha(500.0) == 1.0);
        assert!(h.animating(100.0) && !h.animating(250.0));
        // idle for just under 3 s: still shown; a hold restarts the clock
        assert!(!h.update(2999.0, false));
        assert!(!h.update(5000.0, true));
        assert!(!h.update(7999.0, false) && h.visible());
        assert!(h.update(8000.0, false));
        assert!(!h.visible());
        assert!(h.showing(8100.0) && !h.showing(8300.0));
        assert_eq!(h.alpha(8100.0), 0.5);
        assert_eq!(h.alpha(8200.0), 0.0);
        // a tap shows it again, from wherever the fade had got to
        h.poke(8100.0);
        assert!(h.visible() && (h.alpha(8100.0) - 0.5).abs() < 1e-6);
        assert!(h.alpha(8300.0) == 1.0);
        // held HUDs stay put however long, and a hidden one stays hidden
        assert!(!h.update(60_000.0, true));
        h.toggle(60_000.0);
        assert!(!h.visible() && !h.update(90_000.0, true));
        h.toggle(90_000.0);
        assert!(h.visible());
        // a touch restarts the timer
        h.touch(92_000.0);
        assert!(!h.update(94_999.0, false) && h.update(95_000.0, false));
    }

    #[test]
    fn double_taps_in_the_side_thirds_skip() {
        let w = 1200.0;
        assert_eq!(zone_of(0.0, w), Zone::Left);
        assert_eq!(zone_of(399.0, w), Zone::Left);
        assert_eq!(zone_of(400.0, w), Zone::Middle);
        assert_eq!(zone_of(799.0, w), Zone::Middle);
        assert_eq!(zone_of(800.0, w), Zone::Right);
        assert_eq!(zone_of(1919.0, 1920.0), Zone::Right);
        assert_eq!(zone_delta(Zone::Left), Some(-10_000.0));
        assert_eq!(zone_delta(Zone::Right), Some(30_000.0));
        assert_eq!(zone_delta(Zone::Middle), None);
        let mut t = Taps::default();
        assert_eq!(t.tap(600.0, w, 0.0), Tap::Toggle);
        // a lone side tap waits, then expires into a toggle
        assert_eq!(t.tap(100.0, w, 1000.0), Tap::Wait);
        assert!(t.waiting() && !t.expire(1200.0));
        assert!(t.expire(1301.0) && !t.waiting() && !t.expire(2000.0));
        // two quick taps skip, a third keeps skipping, another zone starts over
        assert_eq!(t.tap(1100.0, w, 3000.0), Tap::Wait);
        assert_eq!(t.tap(1000.0, w, 3200.0), Tap::Skip(Zone::Right));
        assert!(!t.expire(3600.0), "the first tap was consumed");
        assert_eq!(t.tap(1000.0, w, 3400.0), Tap::Skip(Zone::Right));
        assert_eq!(t.tap(100.0, w, 3500.0), Tap::Wait);
        // too slow for a double
        assert_eq!(t.tap(100.0, w, 4000.0), Tap::Wait);
        assert_eq!(t.tap(600.0, w, 4100.0), Tap::Toggle);
        assert!(!t.waiting());
    }

    fn layout_checks(w: i32, h: i32) {
        let l = Layout::new(w, h, w.min(h) as f32 / 768.0);
        let ok = PlayState::Playing;
        let centre = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (px, py) = centre(l.play);
        assert_eq!((px, py), (w as f32 / 2.0, h as f32 / 2.0));
        assert_eq!(l.hit(px, py, true, &ok), Hit::PlayPause);
        assert_eq!(l.hit(px, py, false, &ok), Hit::Video, "a hidden HUD takes no touches");
        let (bx, by) = centre(l.back);
        let (fx, fy) = centre(l.fwd);
        assert_eq!(l.hit(bx, by, true, &ok), Hit::Back);
        assert_eq!(l.hit(fx, fy, true, &ok), Hit::Forward);
        assert!(bx < px && px < fx);
        let (dx, dy) = centre(l.done);
        assert_eq!(l.hit(dx, dy, true, &ok), Hit::Close);
        assert_eq!(l.hit(l.track_x0, l.track_y, true, &ok), Hit::Scrubber);
        assert_eq!(l.hit(l.track_x1, l.track_y + 5.0, true, &ok), Hit::Scrubber);
        assert_eq!(l.hit(l.track_x0 + 1.0, l.track_y - l.s * 100.0, true, &ok), Hit::Video);
        assert_eq!(l.hit(w as f32 - 2.0, h as f32 / 4.0, true, &ok), Hit::Video);
        // everything sits inside the surface, the track has room and the controls do not overlap
        for r in [l.done, l.play, l.back, l.fwd, l.scrub, l.action] {
            assert!(r.0 >= 0.0 && r.1 >= 0.0 && r.0 + r.2 <= w as f32 && r.1 + r.3 <= h as f32, "{r:?} outside {w}x{h}");
        }
        assert!(l.track_x1 - l.track_x0 > l.s * 200.0);
        assert!(l.back.0 + l.back.2 <= l.play.0 && l.play.0 + l.play.2 <= l.fwd.0);
        // failed: only Retry and Done; ended: Play again as well as the buttons
        let (ax, ay) = centre(l.action);
        let err = PlayState::Error("x".into());
        assert_eq!(l.hit(ax, ay, true, &err), Hit::Action);
        assert_eq!(l.hit(px, py, true, &err), Hit::Video);
        assert_eq!(l.hit(ax, ay, true, &PlayState::Ended), Hit::Action);
        assert_eq!(l.hit(ax, ay, true, &ok), Hit::Video);
        assert_eq!(l.hit(px, py, true, &PlayState::Ended), Hit::PlayPause);
    }

    #[test]
    fn hit_testing_in_portrait_and_landscape() {
        layout_checks(1200, 1920);
        layout_checks(1920, 1200);
    }

    /// A player with a script: records what the view asks of it.
    struct Fake {
        state: PlayState,
        pos: f64,
        dur: Option<f64>,
        seeks: Vec<f64>,
        viewport: (i32, i32),
        frames: Vec<Image>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { state: PlayState::Playing, pos: 0.0, dur: Some(600_000.0), seeks: vec![], viewport: (0, 0), frames: vec![] }
        }
    }

    impl Player for Fake {
        fn open(&mut self, _: &std::path::Path, _: &str) -> std::io::Result<()> {
            Ok(())
        }
        fn stop(&mut self) {}
        fn status(&self) -> String {
            String::new()
        }
        fn frame(&mut self) -> Option<Image> {
            self.frames.pop()
        }
        fn play(&mut self) {
            self.state = PlayState::Playing;
        }
        fn pause(&mut self) {
            self.state = PlayState::Paused;
        }
        fn seek_ms(&mut self, ms: f64) {
            self.seeks.push(ms);
            self.pos = ms;
        }
        fn position_ms(&self) -> f64 {
            self.pos
        }
        fn duration_ms(&self) -> Option<f64> {
            self.dur
        }
        fn state(&self) -> PlayState {
            self.state.clone()
        }
        fn set_viewport(&mut self, w: i32, h: i32) {
            self.viewport = (w, h);
        }
    }

    fn tap(v: &mut PlayerView, ui: &Ui, p: &mut Fake, x: f32, y: f32, t: f64) -> Outcome {
        v.touch(ui, p, Event::Down(x, y), t);
        v.touch(ui, p, Event::Up(x, y), t + 30.0)
    }

    #[test]
    fn taps_buttons_scrubbing_and_the_hud_timer_drive_the_player() {
        let ui = ui();
        let (w, h) = (1920, 1200);
        let mut v = PlayerView::new(&ui, w, h, "Film", "film.mp4");
        let mut p = Fake::new();
        assert!(v.tick(&ui, &mut p, 10.0));
        let l = v.layout;
        // play/pause and skips
        assert_eq!(tap(&mut v, &ui, &mut p, w as f32 / 2.0, h as f32 / 2.0, 100.0), Outcome::None);
        assert_eq!(p.state, PlayState::Paused);
        v.tick(&ui, &mut p, 200.0);
        p.pos = 100_000.0;
        v.tick(&ui, &mut p, 300.0);
        tap(&mut v, &ui, &mut p, l.fwd.0 + 5.0, l.fwd.1 + 5.0, 400.0);
        tap(&mut v, &ui, &mut p, l.back.0 + 5.0, l.back.1 + 5.0, 500.0);
        assert_eq!(p.seeks, vec![130_000.0, 120_000.0]);
        // dragging the scrubber shows the target and seeks on release
        let mid = (l.track_x0 + l.track_x1) / 2.0;
        v.touch(&ui, &mut p, Event::Down(l.track_x0 + 1.0, l.track_y), 600.0);
        v.touch(&ui, &mut p, Event::Move(mid, l.track_y + 30.0), 650.0);
        assert!((v.scrub.unwrap() - 300_000.0).abs() < 1_000.0);
        assert_eq!(p.seeks.len(), 2, "seeks on release only");
        v.touch(&ui, &mut p, Event::Up(mid, l.track_y), 700.0);
        assert!((p.seeks[2] - 300_000.0).abs() < 1_000.0 && v.scrub.is_none());
        // a tap on the track jumps there
        tap(&mut v, &ui, &mut p, l.track_x1, l.track_y, 800.0);
        assert_eq!(p.seeks[3], 600_000.0);
        // Done closes
        assert_eq!(tap(&mut v, &ui, &mut p, l.done.0 + 5.0, l.done.1 + 5.0, 900.0), Outcome::Close);
        // playing: the HUD hides itself after 3 s of nothing, and a tap on the picture brings it back
        p.state = PlayState::Playing;
        v.tick(&ui, &mut p, 1000.0);
        assert!(v.hud_visible());
        v.tick(&ui, &mut p, 3900.0);
        assert!(v.hud_visible());
        assert!(v.tick(&ui, &mut p, 4100.0));
        assert!(!v.hud_visible());
        tap(&mut v, &ui, &mut p, w as f32 / 2.0, 100.0, 5000.0);
        assert!(v.hud_visible());
        // paused HUDs stay
        p.state = PlayState::Paused;
        v.tick(&ui, &mut p, 5100.0);
        v.tick(&ui, &mut p, 60_000.0);
        assert!(v.hud_visible());
    }

    #[test]
    fn dragging_across_the_picture_scrubs_and_double_tap_skips() {
        let ui = ui();
        let mut v = PlayerView::new(&ui, 1200, 1920, "Film", "");
        let mut p = Fake::new();
        p.pos = 60_000.0;
        v.tick(&ui, &mut p, 10.0);
        v.tick(&ui, &mut p, 4000.0);
        assert!(!v.hud_visible());
        v.touch(&ui, &mut p, Event::Down(300.0, 900.0), 5000.0);
        v.touch(&ui, &mut p, Event::Move(900.0, 910.0), 5050.0);
        assert_eq!(v.scrub, Some(60_000.0 + 300_000.0));
        v.touch(&ui, &mut p, Event::Up(900.0, 910.0), 5100.0);
        assert_eq!(p.seeks, vec![360_000.0]);
        assert!(!v.hud_visible(), "a drag is not a tap");
        // double tap on the right third skips 30 s and leaves the HUD alone
        tap(&mut v, &ui, &mut p, 1000.0, 900.0, 6000.0);
        tap(&mut v, &ui, &mut p, 1010.0, 900.0, 6150.0);
        assert_eq!(*p.seeks.last().unwrap(), 390_000.0);
        assert!(v.ripple.is_some());
        v.tick(&ui, &mut p, 7000.0);
        assert!(!v.hud_visible() && v.ripple.is_none());
        // a lone side tap toggles after the double-tap window
        tap(&mut v, &ui, &mut p, 100.0, 900.0, 8000.0);
        v.tick(&ui, &mut p, 8100.0);
        assert!(!v.hud_visible());
        v.tick(&ui, &mut p, 8400.0);
        assert!(v.hud_visible());
    }

    #[test]
    fn seeks_stop_at_the_downloaded_edge() {
        let ui = ui();
        let mut v = PlayerView::new(&ui, 1200, 1920, "Film", "");
        let mut p = Fake::new();
        v.set_download(&ui, Some((0.25, false)));
        v.tick(&ui, &mut p, 10.0);
        let l = v.layout;
        tap(&mut v, &ui, &mut p, l.track_x1, l.track_y, 100.0);
        assert_eq!(p.seeks, vec![150_000.0 - DOWNLOAD_MARGIN_MS]);
        p.pos = 140_000.0;
        v.tick(&ui, &mut p, 200.0);
        tap(&mut v, &ui, &mut p, l.fwd.0 + 5.0, l.fwd.1 + 5.0, 300.0);
        assert_eq!(*p.seeks.last().unwrap(), 147_000.0);
        v.set_download(&ui, Some((1.0, true)));
        tap(&mut v, &ui, &mut p, l.track_x1, l.track_y, 400.0);
        assert_eq!(*p.seeks.last().unwrap(), 600_000.0);
    }

    #[test]
    fn errors_retry_and_the_end_plays_again() {
        let ui = ui();
        let mut v = PlayerView::new(&ui, 1200, 1920, "Film", "");
        let mut p = Fake::new();
        v.tick(&ui, &mut p, 10.0);
        v.tick(&ui, &mut p, 4000.0);
        assert!(!v.hud_visible());
        p.state = PlayState::Error("no decoder".into());
        v.tick(&ui, &mut p, 4100.0);
        assert!(v.hud_visible(), "an error brings the HUD back");
        v.tick(&ui, &mut p, 20_000.0);
        assert!(v.hud_visible(), "and keeps it");
        let l = v.layout;
        assert_eq!(tap(&mut v, &ui, &mut p, l.action.0 + 5.0, l.action.1 + 5.0, 21_000.0), Outcome::Retry);
        p.state = PlayState::Ended;
        p.pos = 600_000.0;
        v.tick(&ui, &mut p, 21_100.0);
        assert_eq!(tap(&mut v, &ui, &mut p, l.action.0 + 5.0, l.action.1 + 5.0, 22_000.0), Outcome::None);
        assert_eq!((p.state.clone(), *p.seeks.last().unwrap()), (PlayState::Playing, 0.0));
    }

    #[test]
    fn keys_toggle_and_skip() {
        let ui = ui();
        let mut v = PlayerView::new(&ui, 1200, 1920, "Film", "");
        let mut p = Fake::new();
        p.pos = 50_000.0;
        v.tick(&ui, &mut p, 10.0);
        assert!(v.key(&mut p, KEY_SPACE, 100.0));
        assert_eq!(p.state, PlayState::Paused);
        assert!(v.key(&mut p, KEY_ENTER, 200.0));
        assert_eq!(p.state, PlayState::Playing);
        assert!(v.key(&mut p, KEY_RIGHT, 300.0) && v.key(&mut p, KEY_LEFT, 400.0));
        assert_eq!(p.seeks, vec![80_000.0, 70_000.0]);
        assert!(!v.key(&mut p, 30, 500.0), "other keys stay with the shell");
    }

    #[test]
    fn resizing_rotates_the_viewport_and_refits_the_picture() {
        let ui = ui();
        let mut v = PlayerView::new(&ui, 1200, 1920, "Film", "x");
        let mut p = Fake::new();
        p.frames.push(Image::new(1200, 675));
        v.tick(&ui, &mut p, 10.0);
        assert!(v.has_frame());
        v.set_size(1920, 1200);
        assert!(v.tick(&ui, &mut p, 20.0));
        assert_eq!(p.viewport, (1920, 1200));
        let f = v.frame.as_ref().unwrap();
        assert_eq!((f.w, f.h), (1920, 1080));
        assert_eq!(v.layout.w, 1920.0);
    }

    #[test]
    fn draws_the_hud_in_both_orientations() {
        let ui = ui();
        for (w, h) in [(1200usize, 1920usize), (1920, 1200)] {
            let mut v = PlayerView::new(&ui, w as i32, h as i32, "A very long film title that has to be cut to fit the bar", "Season 1 · film.mp4");
            let mut p = Fake::new();
            p.pos = 90_000.0;
            let mut img = Image::new(w, h * 9 / 16 * w / h);
            img.px.fill(crate::gfx::pack(30, 60, 90, 255));
            p.frames.push(img);
            v.set_download(&ui, Some((0.4, false)));
            v.tick(&ui, &mut p, 10.0);
            let mut buf = vec![0u32; w * h];
            for state in [PlayState::Playing, PlayState::Paused, PlayState::Buffering, PlayState::Ended, PlayState::Error("boom".into())] {
                p.state = state;
                v.tick(&ui, &mut p, 500.0);
                v.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 600.0);
            }
            v.touch(&ui, &mut p, Event::Down(v.layout.track_x0 + 100.0, v.layout.track_y), 700.0);
            v.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 800.0);
            assert!(buf.iter().any(|&p| p != 0));
        }
    }
}
