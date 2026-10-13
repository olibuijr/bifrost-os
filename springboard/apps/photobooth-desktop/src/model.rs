//! Photo Booth's model, free of the window: the capture modes and the countdown, the effect presets (chains of the
//! Core Image style filters), the layout in points, the file names in the library, and the 4-up contact sheet.

use bifrost_app::image_filter::{Filter, FilterChain};
use bifrost_app::kit::{pack, Image};
use springboard::shell::photos::library::Date;
use std::path::{Path, PathBuf};

/// The library folder under the user's Pictures folder, where every capture is saved.
pub const LIBRARY_DIR: &str = "Photo Booth Library";
/// Burst shots taken for one burst, and the gap between them in ms.
pub const BURST_SHOTS: usize = 4;
pub const BURST_GAP_MS: f64 = 350.0;
/// A clip runs this many seconds at this many frames per second.
pub const CLIP_SECS: f64 = 3.0;
pub const CLIP_FPS: u32 = 10;
/// The flash overlay lasts this long, in ms.
pub const FLASH_MS: f64 = 260.0;
/// The widest the live preview is drawn, in pixels; the viewfinder never asks the camera for more.
pub const PREVIEW_MAX_W: usize = 960;
/// Bar heights from the bottom of the window, in points.
pub const CONTROLS_PT: f32 = 64.0;
pub const STRIP_PT: f32 = 84.0;
pub const EFFECTS_PT: f32 = 44.0;
/// Side of a strip thumbnail, in points (the longer side).
pub const THUMB_PT: f32 = 72.0;

/// What the shutter takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Photo,
    Burst,
    Clip,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Photo => Mode::Burst,
            Mode::Burst => Mode::Clip,
            Mode::Clip => Mode::Photo,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Mode::Photo => "Photo",
            Mode::Burst => "Burst",
            Mode::Clip => "Clip",
        }
    }
}

/// The self-timer before the shutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    Off,
    Three,
    Ten,
}

impl Timer {
    pub fn next(self) -> Timer {
        match self {
            Timer::Off => Timer::Three,
            Timer::Three => Timer::Ten,
            Timer::Ten => Timer::Off,
        }
    }

    /// Seconds to count down; zero when the shutter is immediate.
    pub fn secs(self) -> u32 {
        match self {
            Timer::Off => 0,
            Timer::Three => 3,
            Timer::Ten => 10,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Timer::Off => "Timer Off",
            Timer::Three => "3 s",
            Timer::Ten => "10 s",
        }
    }
}

/// The effect applied to the viewfinder and to every capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Mono,
    Noir,
    Sepia,
    Warm,
    Cool,
    Vivid,
    Posterize,
    Vignette,
    Invert,
    Bulge,
    Twirl,
    Mirror,
}

impl Effect {
    pub const ALL: [Effect; 13] = [
        Effect::None,
        Effect::Mono,
        Effect::Noir,
        Effect::Sepia,
        Effect::Warm,
        Effect::Cool,
        Effect::Vivid,
        Effect::Posterize,
        Effect::Vignette,
        Effect::Invert,
        Effect::Bulge,
        Effect::Twirl,
        Effect::Mirror,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Effect::None => "Original",
            Effect::Mono => "Mono",
            Effect::Noir => "Noir",
            Effect::Sepia => "Sepia",
            Effect::Warm => "Warm",
            Effect::Cool => "Cool",
            Effect::Vivid => "Vivid",
            Effect::Posterize => "Posterize",
            Effect::Vignette => "Vignette",
            Effect::Invert => "Invert",
            Effect::Bulge => "Bulge",
            Effect::Twirl => "Twirl",
            Effect::Mirror => "Mirror",
        }
    }

    /// The filters of this preset in order: name, then (input key, value) pairs.
    fn steps(self) -> &'static [(&'static str, &'static [(&'static str, f32)])] {
        match self {
            Effect::None | Effect::Bulge | Effect::Twirl | Effect::Mirror => &[],
            Effect::Mono => &[("saturation", &[("amount", 0.0)])],
            Effect::Noir => &[("saturation", &[("amount", 0.0)]), ("contrast", &[("amount", 1.6)])],
            Effect::Sepia => &[("sepia", &[("amount", 1.0)])],
            Effect::Warm => &[("temperatureTint", &[("temperature", 40.0), ("tint", 0.0)])],
            Effect::Cool => &[("temperatureTint", &[("temperature", -40.0), ("tint", 0.0)])],
            Effect::Vivid => &[("saturation", &[("amount", 1.6)]), ("contrast", &[("amount", 1.2)])],
            Effect::Posterize => &[("posterize", &[("levels", 5.0)])],
            Effect::Vignette => &[("vignette", &[("strength", 0.6)])],
            Effect::Invert => &[("invert", &[])],
        }
    }

    /// The geometric warp of this preset, if it is a distortion rather than a colour chain.
    pub fn warp(self) -> Option<Warp> {
        match self {
            Effect::Bulge => Some(Warp::Bulge),
            Effect::Twirl => Some(Warp::Twirl),
            Effect::Mirror => Some(Warp::Mirror),
            _ => None,
        }
    }

    /// The preset applied to `img`: the colour chain, then the warp.
    pub fn apply(self, img: &Image) -> Image {
        let coloured = if self.steps().is_empty() { img.clone() } else { self.chain().apply(img).unwrap_or_else(|_| img.clone()) };
        match self.warp() {
            Some(warp) => warp_image(&coloured, warp),
            None => coloured,
        }
    }

    /// The filter chain for this preset. Every name resolves in the renderer's filter table (a test checks it).
    pub fn chain(self) -> FilterChain {
        let mut chain = FilterChain::new();
        for (name, inputs) in self.steps() {
            if let Some(mut filter) = Filter::named(name) {
                for (key, value) in inputs.iter() {
                    filter.set(key, *value);
                }
                chain.push(filter);
            }
        }
        chain
    }
}

/// A distortion of the picture geometry (the effects browser's warps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warp {
    /// magnifies the middle like a fisheye lens
    Bulge,
    /// rotates the picture more the closer it is to the middle
    Twirl,
    /// the left half is reflected onto the right half
    Mirror,
}

/// `img` with `warp` applied: each output pixel samples its source by inverse mapping (nearest), so the size is kept.
pub fn warp_image(img: &Image, warp: Warp) -> Image {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() < w * h {
        return img.clone();
    }
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let radius = cx.min(cy).max(1.0);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = match warp {
                Warp::Mirror => (if (x as f32 + 0.5) >= cx { (w - 1 - x) as f32 } else { x as f32 }, y as f32),
                Warp::Bulge | Warp::Twirl => {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    let r = (dx * dx + dy * dy).sqrt() / radius;
                    if r >= 1.0 {
                        (x as f32, y as f32)
                    } else if warp == Warp::Bulge {
                        let k = r.powf(0.8);
                        (cx + dx * k, cy + dy * k)
                    } else {
                        let a = (1.0 - r) * (1.0 - r) * 2.0;
                        let (s, c) = a.sin_cos();
                        (cx + dx * c - dy * s, cy + dx * s + dy * c)
                    }
                }
            };
            let ix = (sx as i64).clamp(0, w as i64 - 1) as usize;
            let iy = (sy as i64).clamp(0, h as i64 - 1) as usize;
            px.push(img.px[iy * w + ix]);
        }
    }
    Image { w, h, px }
}

/// `img` reflected left to right (Auto Flip, so the picture reads like a mirror).
pub fn flip_horizontal(img: &Image) -> Image {
    let mut px = Vec::with_capacity(img.px.len());
    if img.w > 0 {
        for row in img.px.chunks(img.w) {
            px.extend(row.iter().rev());
        }
    }
    Image { w: img.w, h: img.h, px }
}

/// Where the slideshow goes after `index` among `total` captures, wrapping; `forward` false steps back.
pub fn slide_step(index: usize, total: usize, forward: bool) -> usize {
    if total == 0 {
        return 0;
    }
    if forward { (index + 1) % total } else { (index + total - 1) % total }
}

/// The selection after the capture at `removed` leaves a strip of `remaining` captures: the one that took its place,
/// else the new last one, else none.
pub fn selection_after_delete(removed: usize, remaining: usize) -> Option<usize> {
    if remaining == 0 { None } else { Some(removed.min(remaining - 1)) }
}

/// A rectangle in points, from the top-left of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// The window split into the viewfinder, the effects row, the strip and the controls bar. Every rectangle derives
/// from the window size; the viewfinder takes what the bars leave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub viewfinder: Rect,
    pub effects: Rect,
    pub strip: Rect,
    pub controls: Rect,
}

pub fn layout(w: f32, h: f32) -> Layout {
    let w = w.max(0.0);
    let h = h.max(0.0);
    let controls = Rect { x: 0.0, y: (h - CONTROLS_PT).max(0.0), w, h: CONTROLS_PT.min(h) };
    let strip = Rect { x: 0.0, y: (controls.y - STRIP_PT).max(0.0), w, h: STRIP_PT.min(controls.y) };
    let effects = Rect { x: 0.0, y: (strip.y - EFFECTS_PT).max(0.0), w, h: EFFECTS_PT.min(strip.y) };
    let viewfinder = Rect { x: 0.0, y: 0.0, w, h: effects.y };
    Layout { viewfinder, effects, strip, controls }
}

/// The equal-width cells of the effects row, one per preset in `Effect::ALL`.
pub fn effect_cells(row: Rect) -> Vec<(Effect, Rect)> {
    let n = Effect::ALL.len() as f32;
    let cell = row.w / n;
    Effect::ALL.iter().enumerate().map(|(i, e)| (*e, Rect { x: row.x + cell * i as f32, y: row.y, w: cell, h: row.h })).collect()
}

/// The controls bar's buttons: mode on the left, the shutter in the middle, timer and export on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Mode,
    Shutter,
    Timer,
    Export,
}

pub fn button_rect(bar: Rect, button: Button) -> Rect {
    let slot_w = 96.0;
    let shutter: f32 = 52.0;
    let y = bar.y + (bar.h - shutter.min(bar.h)) / 2.0;
    match button {
        Button::Mode => Rect { x: 12.0, y: bar.y + (bar.h - 28.0) / 2.0, w: slot_w, h: 28.0 },
        Button::Timer => Rect { x: 12.0 + slot_w + 8.0, y: bar.y + (bar.h - 28.0) / 2.0, w: slot_w, h: 28.0 },
        Button::Export => Rect { x: (bar.w - 12.0 - slot_w).max(0.0), y: bar.y + (bar.h - 28.0) / 2.0, w: slot_w, h: 28.0 },
        Button::Shutter => Rect { x: bar.w / 2.0 - shutter / 2.0, y, w: shutter, h: shutter.min(bar.h) },
    }
}

/// The name of a capture without its extension: "Photo Booth 2026-10-13 at 14.05.12" plus the kind's suffix
/// ("", " Burst 2", " Burst Sheet", " Clip").
pub fn capture_stem(date: &Date, suffix: &str) -> String {
    format!(
        "Photo Booth {:04}-{:02}-{:02} at {:02}.{:02}.{:02}{}",
        date.year, date.month, date.day, date.hour, date.min, date.sec, suffix
    )
}

/// The first of `stem.ext`, `stem 2.ext`, `stem 3.ext`... that `exists` does not report as taken.
pub fn free_name_with(exists: impl Fn(&str) -> bool, stem: &str, ext: &str) -> String {
    let mut name = format!("{stem}.{ext}");
    let mut n = 2;
    while exists(&name) {
        name = format!("{stem} {n}.{ext}");
        n += 1;
    }
    name
}

/// `free_name_with` against the files in `dir`.
pub fn free_name(dir: &Path, stem: &str, ext: &str) -> String {
    free_name_with(|name| dir.join(name).exists(), stem, ext)
}

/// The library folder inside `home`'s Pictures folder.
pub fn library_dir(home: &Path) -> PathBuf {
    home.join("Pictures").join(LIBRARY_DIR)
}

/// A 2 by 2 contact sheet of up to four images, each scaled into its cell of a `w` by `h` sheet. Empty cells stay black.
pub fn contact_sheet(frames: &[Image], w: usize, h: usize) -> Image {
    let mut px = vec![pack(0, 0, 0, 255); w * h];
    let (cw, ch) = (w / 2, h / 2);
    for (i, frame) in frames.iter().take(4).enumerate() {
        if cw == 0 || ch == 0 {
            break;
        }
        let (ox, oy) = ((i % 2) * cw, (i / 2) * ch);
        let scaled = frame.resized(cw, ch);
        for y in 0..ch.min(scaled.h) {
            let src = &scaled.px[y * scaled.w..y * scaled.w + cw.min(scaled.w)];
            let dst_start = (oy + y) * w + ox;
            px[dst_start..dst_start + src.len()].copy_from_slice(src);
        }
    }
    Image { w, h, px }
}

/// The size an image of `w` by `h` takes when it fits in `max_w` by `max_h` (never enlarged), at least one pixel each way.
pub fn fit_size(w: usize, h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (max_w.max(1), max_h.max(1));
    }
    let r = (max_w as f32 / w as f32).min(max_h as f32 / h as f32).min(1.0);
    (((w as f32 * r) as usize).max(1), ((h as f32 * r) as usize).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    fn date() -> Date {
        Date { year: 2026, month: 10, day: 13, hour: 14, min: 5, sec: 9, wday: 2 }
    }

    #[test]
    fn every_effect_preset_resolves_all_its_filters() {
        for effect in Effect::ALL {
            let names: Vec<&str> = effect.steps().iter().map(|(n, _)| *n).collect();
            assert_eq!(effect.chain().len(), names.len(), "{effect:?}");
            for (name, inputs) in effect.steps() {
                let filter = Filter::named(name).expect("filter name");
                for (key, _) in inputs.iter() {
                    assert!(filter.input_keys().contains(key), "{name} has no input {key}");
                }
            }
        }
    }

    #[test]
    fn original_is_an_empty_chain_and_the_others_are_not() {
        assert!(Effect::None.chain().is_empty());
        for effect in Effect::ALL.iter().skip(1).filter(|e| e.warp().is_none()) {
            assert!(!effect.chain().is_empty(), "{effect:?}");
        }
    }

    fn ramp(w: usize, h: usize) -> Image {
        Image { w, h, px: (0..w * h).map(|i| pack((i % 251) as u8, (i / 3 % 251) as u8, 7, 255)).collect() }
    }

    #[test]
    fn warps_keep_the_size_and_change_the_picture() {
        let img = ramp(16, 12);
        for effect in [Effect::Bulge, Effect::Twirl, Effect::Mirror] {
            let out = effect.apply(&img);
            assert_eq!((out.w, out.h, out.px.len()), (16, 12, 192), "{effect:?}");
            assert_ne!(out, img, "{effect:?}");
        }
        assert_eq!(Effect::None.apply(&img), img);
    }

    #[test]
    fn mirror_reflects_the_left_half_onto_the_right() {
        let img = ramp(8, 2);
        let out = warp_image(&img, Warp::Mirror);
        assert_eq!(out.px[0], img.px[0]);
        assert_eq!(out.px[7], img.px[0]);
        assert_eq!(out.px[4], img.px[3]);
    }

    #[test]
    fn bulge_and_twirl_leave_the_corners_alone() {
        let img = ramp(20, 20);
        for warp in [Warp::Bulge, Warp::Twirl] {
            let out = warp_image(&img, warp);
            assert_eq!(out.px[0], img.px[0]);
            assert_eq!(out.px[399], img.px[399]);
        }
    }

    #[test]
    fn flipping_twice_is_the_identity_and_once_reverses_rows() {
        let img = ramp(5, 3);
        let f = flip_horizontal(&img);
        assert_eq!(f.px[0], img.px[4]);
        assert_eq!(flip_horizontal(&f), img);
    }

    #[test]
    fn slideshow_steps_wrap_both_ways() {
        assert_eq!(slide_step(2, 3, true), 0);
        assert_eq!(slide_step(0, 3, false), 2);
        assert_eq!(slide_step(0, 0, true), 0);
    }

    #[test]
    fn deleting_selects_the_neighbour_or_nothing() {
        assert_eq!(selection_after_delete(1, 2), Some(1));
        assert_eq!(selection_after_delete(2, 2), Some(1));
        assert_eq!(selection_after_delete(0, 0), None);
    }

    #[test]
    fn mono_keeps_the_size_and_drops_colour() {
        let img = solid(4, 3, pack(200, 40, 40, 255));
        let out = Effect::Mono.chain().apply(&img).unwrap();
        assert_eq!((out.w, out.h, out.px.len()), (4, 3, 12));
        let p = out.px[0];
        let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
        assert!(r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2, "{r} {g} {b}");
    }

    #[test]
    fn invert_twice_is_the_identity() {
        let img = solid(2, 2, pack(10, 20, 30, 255));
        let chain = Effect::Invert.chain();
        let once = chain.apply(&img).unwrap();
        assert_ne!(once, img);
        assert_eq!(chain.apply(&once).unwrap(), img);
    }

    #[test]
    fn the_timer_and_mode_cycle_back_to_the_start() {
        assert_eq!(Mode::Photo.next().next().next(), Mode::Photo);
        assert_eq!(Timer::Off.next().next().next(), Timer::Off);
        assert_eq!(Timer::Off.secs(), 0);
        assert_eq!(Timer::Ten.secs(), 10);
    }

    #[test]
    fn capture_names_use_the_local_time_and_the_suffix() {
        assert_eq!(capture_stem(&date(), ""), "Photo Booth 2026-10-13 at 14.05.09");
        assert_eq!(capture_stem(&date(), " Burst Sheet"), "Photo Booth 2026-10-13 at 14.05.09 Burst Sheet");
    }

    #[test]
    fn a_taken_name_gets_the_next_free_number() {
        let taken = ["a.jpg", "a 2.jpg"];
        assert_eq!(free_name_with(|n| taken.contains(&n), "a", "jpg"), "a 3.jpg");
        assert_eq!(free_name_with(|_| false, "a", "jpg"), "a.jpg");
    }

    #[test]
    fn the_library_sits_in_pictures() {
        assert_eq!(library_dir(Path::new("/home/x")), PathBuf::from("/home/x/Pictures/Photo Booth Library"));
    }

    #[test]
    fn layout_bars_stack_under_the_viewfinder_and_fill_the_window() {
        let l = layout(1200.0, 780.0);
        assert_eq!(l.controls.y + l.controls.h, 780.0);
        assert_eq!(l.strip.y + l.strip.h, l.controls.y);
        assert_eq!(l.effects.y + l.effects.h, l.strip.y);
        assert_eq!(l.viewfinder.h, l.effects.y);
        assert!(l.viewfinder.h > 500.0);
    }

    #[test]
    fn a_tiny_window_never_gets_negative_rectangles() {
        let l = layout(100.0, 40.0);
        for r in [l.viewfinder, l.effects, l.strip, l.controls] {
            assert!(r.h >= 0.0 && r.y >= 0.0);
        }
    }

    #[test]
    fn effect_cells_split_the_row_evenly() {
        let cells = effect_cells(Rect { x: 0.0, y: 0.0, w: 1000.0, h: 40.0 });
        assert_eq!(cells.len(), Effect::ALL.len());
        let last = cells.last().unwrap().1;
        assert!((last.x + last.w - 1000.0).abs() < 0.01);
    }

    #[test]
    fn the_shutter_is_centred_in_the_controls_bar() {
        let bar = layout(1200.0, 780.0).controls;
        let s = button_rect(bar, Button::Shutter);
        assert!((s.x + s.w / 2.0 - 600.0).abs() < 0.01);
        assert!(button_rect(bar, Button::Mode).x < s.x);
        assert!(button_rect(bar, Button::Export).x > s.x + s.w);
    }

    #[test]
    fn contact_sheet_places_each_frame_in_its_cell() {
        let a = solid(4, 4, pack(255, 0, 0, 255));
        let b = solid(4, 4, pack(0, 255, 0, 255));
        let sheet = contact_sheet(&[a, b], 8, 8);
        assert_eq!((sheet.w, sheet.h, sheet.px.len()), (8, 8, 64));
        assert_eq!(sheet.px[0], pack(255, 0, 0, 255));
        assert_eq!(sheet.px[4], pack(0, 255, 0, 255));
        // the cells below are empty and stay black
        assert_eq!(sheet.px[4 * 8], pack(0, 0, 0, 255));
    }

    #[test]
    fn fit_size_keeps_the_aspect_and_never_enlarges() {
        assert_eq!(fit_size(1280, 720, 640, 480), (640, 360));
        assert_eq!(fit_size(100, 50, 640, 480), (100, 50));
        assert_eq!(fit_size(4, 3, 2, 100), (2, 1));
    }

    #[test]
    fn contact_sheet_of_nothing_is_black() {
        let sheet = contact_sheet(&[], 4, 4);
        assert!(sheet.px.iter().all(|p| *p == pack(0, 0, 0, 255)));
    }
}
