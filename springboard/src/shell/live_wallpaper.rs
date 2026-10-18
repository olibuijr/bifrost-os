//! Live wallpaper: the animated and time-of-day looks of Settings › Wallpaper, drawn by the desktop under the
//! windows. The kernels are `bifrost_render::wallpaper_shaders` (drift, waves, dither, time-of-day poster); this
//! module chooses the look (the `wallpaper_look` pref), decides which frame is due, renders it small and scales it to
//! the screen, and keeps the finished frame so a redraw of the desktop only copies it.
//!
//! Cost rules: an animated look makes at most `FPS` frames a second, a frame is computed at `PROXY_SIDE` pixels on
//! its longer side, and the time-of-day poster is recomputed once a minute. Reduce Motion and Low Power Mode freeze
//! the animated looks on one fixed frame and slow the poster to once every `STILL_POSTER_MIN` minutes.

use crate::gfx::{pack, Image, View};
use bifrost_render::colorfx::Rgb;
use bifrost_render::gradient::Ramp;
use bifrost_render::wallpaper_shaders::{dither_frame, drift_frame, dynamic_frame, waves_frame, WaveLayer};

/// The pref that stores the look.
pub const LOOK_PREF: &str = "wallpaper_look";
/// Animated looks make this many frames a second.
pub const FPS: f64 = 4.0;
/// Milliseconds between animated frames.
pub const FRAME_MS: f64 = 1000.0 / FPS;
/// A frame is computed with its longer side this many pixels, then scaled up.
pub const PROXY_SIDE: usize = 240;
/// The poster re-renders this often (ms) when it is allowed to move.
pub const POSTER_MS: f64 = 60_000.0;
/// Minutes between poster updates while motion is reduced.
pub const STILL_POSTER_MIN: u16 = 15;

/// What the desktop draws under the windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    /// The picked still photo (or the Dynamic Wallpaper stills).
    #[default]
    Still,
    /// Slowly flowing clouds of colour.
    Drift,
    /// Layered ridges travelling over a sky.
    Waves,
    /// A scrolling one-bit dither.
    Dither,
    /// A gradient poster that follows the time of day.
    TimeOfDay,
}

impl Look {
    pub const ALL: [Look; 5] = [Look::Still, Look::Drift, Look::Waves, Look::Dither, Look::TimeOfDay];

    /// The value stored in `wallpaper_look`.
    pub fn pref(self) -> &'static str {
        match self {
            Look::Still => "still",
            Look::Drift => "drift",
            Look::Waves => "waves",
            Look::Dither => "dither",
            Look::TimeOfDay => "time_of_day",
        }
    }

    /// The name on the Settings page.
    pub fn label(self) -> &'static str {
        match self {
            Look::Still => "Still",
            Look::Drift => "Drift",
            Look::Waves => "Waves",
            Look::Dither => "Dither",
            Look::TimeOfDay => "Time of Day",
        }
    }

    /// The look a stored value names; anything unknown (or empty) is `Still`.
    pub fn parse(s: &str) -> Look {
        Look::ALL.into_iter().find(|l| l.pref() == s.trim()).unwrap_or(Look::Still)
    }

    /// True for the looks that move every few hundred milliseconds.
    pub fn animated(self) -> bool {
        matches!(self, Look::Drift | Look::Waves | Look::Dither)
    }
}

/// Which frame of `look` is due. Equal keys are the same picture, so the cache re-renders only when this changes.
/// `still` freezes the animated looks on frame 0 and slows the poster to `STILL_POSTER_MIN`.
pub fn frame_key(look: Look, t_ms: f64, minute: u16, still: bool) -> u64 {
    match look {
        Look::Still => 0,
        Look::TimeOfDay => u64::from(if still { minute / STILL_POSTER_MIN } else { minute }),
        _ if still => 0,
        _ => (t_ms.max(0.0) / FRAME_MS) as u64,
    }
}

/// When (monotonic ms) the next frame of `look` is due after `t_ms`; None while nothing will change.
pub fn next_due(look: Look, t_ms: f64, still: bool) -> Option<f64> {
    match look {
        Look::Still => None,
        Look::TimeOfDay => Some((t_ms / POSTER_MS).floor() * POSTER_MS + POSTER_MS),
        _ if still => None,
        _ => Some((t_ms / FRAME_MS).floor() * FRAME_MS + FRAME_MS),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

fn ramp(stops: &[(f32, Rgb)]) -> Ramp {
    Ramp::new(stops).expect("the built-in ramps are ordered")
}

/// The poster's key hours (night, dawn, day, dusk) and their colours, top-left to bottom-right.
const POSTER_STOPS: [f32; 4] = [0.0, 6.0, 12.0, 18.0];

fn poster_ramps() -> [Ramp; 4] {
    [
        ramp(&[(0.0, rgb(8, 12, 36)), (1.0, rgb(38, 30, 78))]),
        ramp(&[(0.0, rgb(96, 108, 168)), (1.0, rgb(250, 176, 120))]),
        ramp(&[(0.0, rgb(70, 150, 230)), (1.0, rgb(176, 224, 250))]),
        ramp(&[(0.0, rgb(86, 62, 150)), (1.0, rgb(244, 120, 96))]),
    ]
}

fn to_px(c: Rgb) -> u32 {
    let c = c.clamped();
    pack((c.r * 255.0 + 0.5) as u8, (c.g * 255.0 + 0.5) as u8, (c.b * 255.0 + 0.5) as u8, 255)
}

/// The proxy size for a `w` x `h` screen: the same shape, the longer side `PROXY_SIDE` (never larger than the screen).
pub fn proxy_size(w: usize, h: usize) -> (usize, usize) {
    let long = w.max(h).max(1);
    let side = PROXY_SIDE.min(long);
    ((w * side).div_ceil(long).max(1), (h * side).div_ceil(long).max(1))
}

/// The kernel output for `look` at `proxy` size, `time` seconds into an animation or `hour` o'clock. None for `Still`.
fn kernel(look: Look, pw: usize, ph: usize, time: f32, hour: f32) -> Option<Vec<Rgb>> {
    match look {
        Look::Still => None,
        Look::Drift => {
            let r = ramp(&[(0.0, rgb(12, 24, 64)), (0.45, rgb(40, 96, 168)), (0.8, rgb(110, 190, 220)), (1.0, rgb(230, 244, 250))]);
            Some(drift_frame(pw, ph, time, &r, 3.0, 7))
        }
        Look::Waves => {
            let sky = ramp(&[(0.0, rgb(250, 190, 140)), (0.6, rgb(120, 170, 220)), (1.0, rgb(40, 90, 160))]);
            let layers = [
                WaveLayer { base: 0.55, amplitude: 0.05, wavelength: 0.9, speed: 0.04, phase: 0.0, color: rgb(70, 130, 190) },
                WaveLayer { base: 0.68, amplitude: 0.06, wavelength: 0.7, speed: 0.06, phase: 0.3, color: rgb(40, 96, 160) },
                WaveLayer { base: 0.82, amplitude: 0.07, wavelength: 0.5, speed: 0.08, phase: 0.6, color: rgb(18, 56, 112) },
            ];
            Some(waves_frame(pw, ph, time, &sky, &layers))
        }
        Look::Dither => Some(dither_frame(pw, ph, time, rgb(14, 18, 30), rgb(190, 214, 236), (ph / 60).max(1), 1.5)),
        Look::TimeOfDay => dynamic_frame(pw, ph, hour, &POSTER_STOPS, &poster_ramps()),
    }
}

/// `src` (opaque) scaled to `w` x `h`: bilinear in 8-bit fixed point, or nearest for the one-bit look.
fn upscale(src: &Image, w: usize, h: usize, smooth: bool) -> Image {
    let mut out = Image::new(w, h);
    if src.w == 0 || src.h == 0 || w == 0 || h == 0 {
        return out;
    }
    // per destination column: the two source columns and the weight of the second (0..=256)
    let cols: Vec<(usize, usize, u32)> = (0..w)
        .map(|x| {
            let f = if smooth { (((x as f32 + 0.5) * src.w as f32 / w as f32) - 0.5).clamp(0.0, (src.w - 1) as f32) } else { (x * src.w / w) as f32 };
            let x0 = f as usize;
            (x0, (x0 + 1).min(src.w - 1), (f.fract() * 256.0) as u32)
        })
        .collect();
    let mix = |a: u32, b: u32, t: u32| {
        let (rb, g) = (((a & 0x00ff_00ff) * (256 - t) + (b & 0x00ff_00ff) * t) >> 8 & 0x00ff_00ff, ((a & 0x0000_ff00) * (256 - t) + (b & 0x0000_ff00) * t) >> 8 & 0x0000_ff00);
        rb | g | 0xff00_0000
    };
    for y in 0..h {
        let f = if smooth { (((y as f32 + 0.5) * src.h as f32 / h as f32) - 0.5).clamp(0.0, (src.h - 1) as f32) } else { (y * src.h / h) as f32 };
        let (y0, ty) = (f as usize, (f.fract() * 256.0) as u32);
        let y1 = (y0 + 1).min(src.h - 1);
        let (r0, r1) = (&src.px[y0 * src.w..(y0 + 1) * src.w], &src.px[y1 * src.w..(y1 + 1) * src.w]);
        for (o, &(x0, x1, tx)) in out.px[y * w..(y + 1) * w].iter_mut().zip(&cols) {
            *o = if smooth { mix(mix(r0[x0], r0[x1], tx), mix(r1[x0], r1[x1], tx), ty) } else { r0[x0] };
        }
    }
    out
}

/// One finished frame of `look` for a `w` x `h` screen at animation `time` seconds or `hour` o'clock. None for `Still`.
pub fn render(look: Look, w: usize, h: usize, time: f32, hour: f32) -> Option<Image> {
    let (pw, ph) = proxy_size(w, h);
    let small = kernel(look, pw, ph, time, hour)?;
    let proxy = Image { w: pw, h: ph, px: small.into_iter().map(to_px).collect() };
    Some(upscale(&proxy, w, h, look != Look::Dither))
}

/// The look in force and its cached frame.
#[derive(Default)]
pub struct Live {
    look: Look,
    key: Option<u64>,
    image: Option<Image>,
}

impl Live {
    pub fn look(&self) -> Look {
        self.look
    }

    /// Chooses the look. Drops the cached frame; true when the look changed.
    pub fn set_look(&mut self, look: Look) -> bool {
        let changed = look != self.look;
        self.look = look;
        self.key = None;
        self.image = None;
        changed
    }

    /// The cached frame as a view; None for `Still` or before the first `update`.
    pub fn view(&self) -> Option<View<'_>> {
        self.image.as_ref().map(Image::view)
    }

    /// The frame number the cache holds.
    pub fn key(&self) -> Option<u64> {
        self.key
    }

    /// Brings the cache to the frame due at `t_ms` (monotonic) and local `minute` of the day for a `w` x `h` screen.
    /// True when the picture changed, so the desktop must redraw.
    pub fn update(&mut self, t_ms: f64, minute: u16, still: bool, w: usize, h: usize) -> bool {
        if self.look == Look::Still || w == 0 || h == 0 {
            return false;
        }
        let key = frame_key(self.look, t_ms, minute, still);
        if self.key == Some(key) && self.image.as_ref().is_some_and(|i| (i.w, i.h) == (w, h)) {
            return false;
        }
        let time = (key as f64 * FRAME_MS / 1000.0) as f32;
        let hour = if self.look == Look::TimeOfDay { f32::from(if still { minute - minute % STILL_POSTER_MIN } else { minute }) / 60.0 } else { 0.0 };
        self.image = render(self.look, w, h, time, hour);
        self.key = Some(key);
        self.image.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_value_round_trips_and_unknown_means_still() {
        for look in Look::ALL {
            assert_eq!(Look::parse(look.pref()), look);
        }
        assert_eq!(Look::parse(""), Look::Still);
        assert_eq!(Look::parse("sparkles"), Look::Still);
        assert!(Look::Drift.animated() && !Look::TimeOfDay.animated() && !Look::Still.animated());
    }

    #[test]
    fn animated_looks_step_at_the_frame_rate_and_freeze_when_still() {
        assert_eq!(frame_key(Look::Drift, 0.0, 600, false), 0);
        assert_eq!(frame_key(Look::Drift, FRAME_MS - 1.0, 600, false), 0);
        assert_eq!(frame_key(Look::Drift, FRAME_MS, 600, false), 1);
        assert_eq!(frame_key(Look::Waves, 10_000.0, 600, false), 40);
        assert_eq!(frame_key(Look::Dither, 10_000.0, 600, true), 0, "reduced motion: one fixed frame");
        assert_eq!(frame_key(Look::Still, 10_000.0, 600, false), 0);
        assert_eq!(next_due(Look::Drift, 100.0, false), Some(FRAME_MS));
        assert_eq!(next_due(Look::Drift, 100.0, true), None);
        assert_eq!(next_due(Look::Still, 100.0, false), None);
        assert_eq!(next_due(Look::TimeOfDay, 100.0, true), Some(POSTER_MS));
    }

    #[test]
    fn the_poster_follows_the_minute_and_slows_when_still() {
        assert_eq!(frame_key(Look::TimeOfDay, 0.0, 600, false), 600);
        assert_eq!(frame_key(Look::TimeOfDay, 0.0, 601, false), 601);
        assert_eq!(frame_key(Look::TimeOfDay, 0.0, 600, true), frame_key(Look::TimeOfDay, 0.0, 614, true));
        assert_ne!(frame_key(Look::TimeOfDay, 0.0, 600, true), frame_key(Look::TimeOfDay, 0.0, 615, true));
    }

    #[test]
    fn the_proxy_keeps_the_shape_and_never_exceeds_the_screen() {
        assert_eq!(proxy_size(1200, 1920), (150, 240));
        assert_eq!(proxy_size(1920, 1080), (240, 135));
        assert_eq!(proxy_size(100, 50), (100, 50));
    }

    #[test]
    fn every_look_renders_a_full_opaque_frame_and_still_renders_none() {
        assert!(render(Look::Still, 64, 48, 0.0, 12.0).is_none());
        for look in [Look::Drift, Look::Waves, Look::Dither, Look::TimeOfDay] {
            let img = render(look, 64, 48, 1.0, 12.0).unwrap();
            assert_eq!((img.w, img.h), (64, 48));
            assert!(img.px.iter().all(|p| p >> 24 == 0xff), "{look:?} is opaque");
        }
    }

    #[test]
    fn the_poster_is_dark_at_night_and_bright_at_noon() {
        let lum = |img: &Image| img.px.iter().map(|p| (p & 0xff) + (p >> 8 & 0xff) + (p >> 16 & 0xff)).sum::<u32>() / img.px.len() as u32;
        let night = render(Look::TimeOfDay, 32, 32, 0.0, 0.0).unwrap();
        let noon = render(Look::TimeOfDay, 32, 32, 0.0, 12.0).unwrap();
        assert!(lum(&noon) > lum(&night) + 150, "noon {} night {}", lum(&noon), lum(&night));
    }

    #[test]
    fn the_cache_renders_only_when_the_due_frame_changes() {
        let mut live = Live::default();
        assert!(!live.update(0.0, 0, false, 64, 48), "still draws nothing");
        assert!(live.view().is_none());
        assert!(live.set_look(Look::Drift));
        assert!(live.update(0.0, 0, false, 64, 48), "first frame");
        let first = live.view().unwrap().px.to_vec();
        assert!(!live.update(100.0, 0, false, 64, 48), "same frame number: cached");
        assert!(live.update(FRAME_MS * 8.0, 0, false, 64, 48), "a later frame");
        assert_ne!(live.view().unwrap().px, &first[..], "the drift moved");
        assert!(live.update(FRAME_MS * 8.0, 0, false, 32, 24), "a new size re-renders");
        assert_eq!(live.view().unwrap().w, 32);
        assert!(!live.set_look(Look::Drift), "same look");
        assert!(live.set_look(Look::Still));
        assert!(live.view().is_none(), "back to still frees the frame");
    }

    #[test]
    fn a_still_cache_holds_one_frame_for_ever() {
        let mut live = Live::default();
        live.set_look(Look::Waves);
        assert!(live.update(0.0, 0, true, 48, 48));
        assert!(!live.update(60_000.0, 0, true, 48, 48), "reduced motion or low power: no new frame");
        assert_eq!(live.key(), Some(0));
    }
}
