//! Photo Booth's model, free of the window: the capture modes and the countdown, the effect presets (chains of the
//! Core Image style filters), the layout in points, the file names in the library, and the 4-up contact sheet.

use bifrost_app::image_filter::{Filter, FilterChain};
use bifrost_app::kit::{pack, Image};
use crate::warp;
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
/// While no camera is open the camera list is read again this often, in ms, so a camera plugged in later appears.
pub const CAMERA_RETRY_MS: f64 = 2000.0;
/// The widest the live preview is drawn, in pixels; the viewfinder never asks the camera for more.
pub const PREVIEW_PIXEL_LIMIT: usize = 960;
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

    /// True for the effects that move pixels (`warp.rs`) instead of changing their colour.
    pub fn is_distortion(self) -> bool {
        matches!(self, Effect::Bulge | Effect::Twirl | Effect::Mirror)
    }

    /// The filters of this preset in order: name, then (input key, value) pairs.
    fn steps(self) -> &'static [(&'static str, &'static [(&'static str, f32)])] {
        match self {
            Effect::None => &[],
            Effect::Mono => &[("saturation", &[("amount", 0.0)])],
            Effect::Noir => &[("saturation", &[("amount", 0.0)]), ("contrast", &[("amount", 1.6)])],
            Effect::Sepia => &[("sepia", &[("amount", 1.0)])],
            Effect::Warm => &[("temperatureTint", &[("temperature", 40.0), ("tint", 0.0)])],
            Effect::Cool => &[("temperatureTint", &[("temperature", -40.0), ("tint", 0.0)])],
            Effect::Vivid => &[("saturation", &[("amount", 1.6)]), ("contrast", &[("amount", 1.2)])],
            Effect::Posterize => &[("posterize", &[("levels", 5.0)])],
            Effect::Vignette => &[("vignette", &[("strength", 0.6)])],
            Effect::Invert => &[("invert", &[])],
            Effect::Bulge | Effect::Twirl | Effect::Mirror => &[],
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

    /// The picture with this effect: the colour filters first, then the distortion if the effect has one, pulled as far
    /// as `strength` says. Mirror has no strength.
    pub fn apply(self, img: Image, strength: warp::Strength) -> Image {
        let img = if self.steps().is_empty() { img } else { self.chain().apply(&img).unwrap_or(img) };
        match self {
            Effect::Bulge => warp::bulge(&img, strength),
            Effect::Twirl => warp::twirl(&img, strength),
            Effect::Mirror => warp::mirror_left(&img),
            _ => img,
        }
    }
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

/// The layout while the viewfinder fills the window: the other bars have no area, so nothing in them can be hit.
pub fn layout_full_screen(w: f32, h: f32) -> Layout {
    let w = w.max(0.0);
    let h = h.max(0.0);
    let none = Rect { x: 0.0, y: h, w: 0.0, h: 0.0 };
    Layout { viewfinder: Rect { x: 0.0, y: 0.0, w, h }, effects: none, strip: none, controls: none }
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
    const SLOT_MAX: f32 = 96.0;
    const EDGE_GAP: f32 = 12.0;
    const BUTTON_GAP: f32 = 8.0;
    const SHUTTER_MAX: f32 = 52.0;
    let shutter = SHUTTER_MAX.min(bar.w * 0.24);
    let y = bar.y + (bar.h - shutter.min(bar.h)) / 2.0;
    let centre = bar.x + bar.w / 2.0;
    let left = bar.x + EDGE_GAP.min(bar.w / 10.0);
    let right = bar.x + bar.w - EDGE_GAP.min(bar.w / 10.0);
    let left_end = (centre - shutter / 2.0 - BUTTON_GAP).max(left);
    let right_start = (centre + shutter / 2.0 + BUTTON_GAP).min(right);
    let left_width = (left_end - left - BUTTON_GAP).max(0.0) / 2.0;
    let left_width = left_width.min(SLOT_MAX);
    let mode_x = left;
    let timer_x = (left + left_width + BUTTON_GAP).min(left_end);
    let export_width = (right - right_start).max(0.0).min(SLOT_MAX);
    let export_x = right_start + ((right - right_start - export_width).max(0.0) / 2.0);
    match button {
        Button::Mode => Rect { x: mode_x, y: bar.y + (bar.h - 28.0).max(0.0) / 2.0, w: left_width, h: 28.0.min(bar.h) },
        Button::Timer => Rect { x: timer_x, y: bar.y + (bar.h - 28.0).max(0.0) / 2.0, w: left_width, h: 28.0.min(bar.h) },
        Button::Export => Rect { x: export_x, y: bar.y + (bar.h - 28.0).max(0.0) / 2.0, w: export_width, h: 28.0.min(bar.h) },
        Button::Shutter => Rect { x: centre - shutter / 2.0, y, w: shutter, h: shutter.min(bar.h) },
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

/// True once the screen flash that began at `started` (ms on the app clock) has lasted `FLASH_MS` at time `t`.
pub fn flash_over(started: f64, t: f64) -> bool {
    t - started >= FLASH_MS
}

/// True when the camera list is due to be read again: `CAMERA_RETRY_MS` after the last read, or later.
pub fn camera_retry_due(last_read: f64, now: f64) -> bool {
    now - last_read >= CAMERA_RETRY_MS
}

/// The position of the camera at device node `chosen` in a freshly read list, or none once it has been unplugged.
pub fn position_of_camera(chosen: &str, listed: &[&str]) -> Option<usize> {
    listed.iter().position(|path| *path == chosen)
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
    let mut px = vec![bifrost_app::kit::ui::BLACK; w * h];
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

/// The size an image of `w` by `h` takes when it is scaled, up or down, to fill as much of `max_w` by `max_h` as its
/// aspect allows; at least one pixel each way.
pub fn fit_inside(w: usize, h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (max_w.max(1), max_h.max(1));
    }
    let r = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    (((w as f32 * r) as usize).clamp(1, max_w.max(1)), ((h as f32 * r) as usize).clamp(1, max_h.max(1)))
}

/// Which capture is selected after the one at `deleted` left the strip, which then holds `remaining` captures: the one
/// that took its place, or the last one when the newest was deleted, or none when the strip is empty.
pub fn selection_after_delete(deleted: usize, remaining: usize) -> Option<usize> {
    remaining.checked_sub(1).map(|last| deleted.min(last))
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
    fn original_is_an_empty_chain_and_the_colour_effects_are_not() {
        assert!(Effect::None.chain().is_empty());
        for effect in Effect::ALL.iter().skip(1).filter(|e| !e.is_distortion()) {
            assert!(!effect.chain().is_empty(), "{effect:?}");
        }
    }

    #[test]
    fn the_distortion_effects_have_no_colour_filters_and_change_the_picture() {
        let mut px = Vec::new();
        for y in 0..40usize {
            for x in 0..60usize {
                px.push(pack(x as u8 * 4, y as u8 * 6, 90, 255));
            }
        }
        let img = Image { w: 60, h: 40, px };
        let distortions: Vec<Effect> = Effect::ALL.iter().copied().filter(|e| e.is_distortion()).collect();
        assert_eq!(distortions, [Effect::Bulge, Effect::Twirl, Effect::Mirror]);
        for effect in distortions {
            assert!(effect.chain().is_empty(), "{effect:?}");
            let out = effect.apply(img.clone(), warp::Strength::Normal);
            assert_eq!((out.w, out.h), (60, 40), "{effect:?}");
            assert_ne!(out, img, "{effect:?}");
        }
    }

    #[test]
    fn the_strength_changes_the_bulge_and_the_twirl_but_not_the_mirror() {
        let mut px = Vec::new();
        for y in 0..40usize {
            for x in 0..40usize {
                px.push(pack(x as u8 * 6, y as u8 * 6, 90, 255));
            }
        }
        let ramp = Image { w: 40, h: 40, px };
        assert_ne!(Effect::Bulge.apply(ramp.clone(), warp::Strength::Subtle), Effect::Bulge.apply(ramp.clone(), warp::Strength::Strong));
        assert_ne!(Effect::Twirl.apply(ramp.clone(), warp::Strength::Subtle), Effect::Twirl.apply(ramp.clone(), warp::Strength::Strong));
        assert_eq!(Effect::Mirror.apply(ramp.clone(), warp::Strength::Subtle), Effect::Mirror.apply(ramp, warp::Strength::Strong));
    }

    #[test]
    fn apply_runs_the_colour_chain_and_leaves_the_original_alone() {
        let img = solid(3, 3, pack(10, 20, 30, 255));
        assert_eq!(Effect::None.apply(img.clone(), warp::Strength::Normal), img);
        assert_ne!(Effect::Invert.apply(img.clone(), warp::Strength::Normal), img);
    }

    #[test]
    fn effect_titles_are_distinct() {
        let mut titles: Vec<&str> = Effect::ALL.iter().map(|e| e.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), Effect::ALL.len());
    }

    #[test]
    fn the_full_screen_layout_gives_the_viewfinder_the_whole_window_and_hides_the_bars() {
        let l = layout_full_screen(1200.0, 780.0);
        assert_eq!(l.viewfinder, Rect { x: 0.0, y: 0.0, w: 1200.0, h: 780.0 });
        for bar in [l.effects, l.strip, l.controls] {
            assert!(!bar.contains(10.0, 779.0) && !bar.contains(0.0, 780.0) && !bar.contains(600.0, 400.0));
        }
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
    fn controls_stay_inside_the_bar_and_do_not_overlap_at_compact_widths() {
        for width in [160.0, 240.0, 360.0, 640.0] {
            let bar = layout(width, 420.0).controls;
            let mode = button_rect(bar, Button::Mode);
            let timer = button_rect(bar, Button::Timer);
            let shutter = button_rect(bar, Button::Shutter);
            let export = button_rect(bar, Button::Export);
            for r in [mode, timer, shutter, export] {
                assert!(r.x >= bar.x && r.x + r.w <= bar.x + bar.w);
                assert!(r.y >= bar.y && r.y + r.h <= bar.y + bar.h);
            }
            assert!(mode.x + mode.w <= timer.x);
            assert!(timer.x + timer.w <= shutter.x);
            assert!(shutter.x + shutter.w <= export.x);
        }
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
    fn fit_inside_scales_up_and_down_and_keeps_the_aspect() {
        assert_eq!(fit_inside(960, 540, 1920, 1200), (1920, 1080));
        assert_eq!(fit_inside(960, 540, 480, 480), (480, 270));
        assert_eq!(fit_inside(100, 100, 100, 100), (100, 100));
        assert_eq!(fit_inside(0, 0, 50, 40), (50, 40));
    }

    #[test]
    fn deleting_selects_the_capture_that_moves_up_or_the_last_one() {
        assert_eq!(selection_after_delete(1, 3), Some(1));
        assert_eq!(selection_after_delete(0, 3), Some(0));
        assert_eq!(selection_after_delete(3, 3), Some(2));
        assert_eq!(selection_after_delete(0, 0), None);
    }

    #[test]
    fn the_screen_flash_is_on_until_its_time_runs_out() {
        assert!(!flash_over(1000.0, 1000.0));
        assert!(!flash_over(1000.0, 1000.0 + FLASH_MS - 1.0));
        assert!(flash_over(1000.0, 1000.0 + FLASH_MS));
        assert!(flash_over(1000.0, 1000.0 + FLASH_MS * 4.0));
    }

    #[test]
    fn the_camera_list_is_read_again_every_retry_interval() {
        assert!(!camera_retry_due(1000.0, 1000.0 + CAMERA_RETRY_MS - 1.0));
        assert!(camera_retry_due(1000.0, 1000.0 + CAMERA_RETRY_MS));
        assert!(camera_retry_due(0.0, 1_000_000.0));
    }

    #[test]
    fn a_chosen_camera_is_found_only_while_it_is_still_listed() {
        let listed = ["/dev/video0", "/dev/video2"];
        assert_eq!(position_of_camera("/dev/video2", &listed), Some(1));
        assert_eq!(position_of_camera("/dev/video1", &listed), None);
        assert_eq!(position_of_camera("/dev/video0", &[]), None);
    }

    #[test]
    fn contact_sheet_of_nothing_is_black() {
        let sheet = contact_sheet(&[], 4, 4);
        assert!(sheet.px.iter().all(|p| *p == pack(0, 0, 0, 255)));
    }
}
