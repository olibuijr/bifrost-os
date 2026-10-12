//! `bootweave-frame`: the single description and renderer of the boot frame (docs/boot-architecture.md, "One
//! frame"), the hand-off from the frame to the first screen, and the boot timing record behind the progress bar
//! ("The progress bar"). `no_std` plus `alloc`, no dependencies, so the UEFI loader (`springboard/bootweave/efi`),
//! Bootweave, the installer and the window server all draw the same pixels from the same code.
//!
//! Owners (boot team): `render`, the screens, and the hand-off (task 02); `progress` and `timing` (task 03; task 06
//! writes records).

#![no_std]
extern crate alloc;

pub mod progress;
pub mod render;
pub mod timing;

use alloc::vec::Vec;

pub use progress::Progress;
pub use render::{bar_rect, mark_box, palette, render, render_bar, PixelOrder, Palette, Rgb};
pub use timing::{BootTiming, Stage};

/// Light or Dark, as the user last chose (stored with the timing record so the loader knows it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// Which screen the frame shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Screen {
    /// The mark (and the progress bar when `Frame::progress` is set).
    #[default]
    Boot,
    /// Pre-boot unlock: the user's name, a password field with `dots` dots, and whether the last try failed.
    Unlock { name: alloc::string::String, dots: u32, failed: bool },
    /// The boot options (held Shift): entries and the selected one.
    Options { entries: alloc::vec::Vec<alloc::string::String>, selected: u32 },
    /// Starting the previous version after a failed update.
    Rollback,
    /// The session could not start: Restart and Recovery, the selected button.
    StartFailed { selected: u32 },
    /// After a kernel panic: "Your computer restarted because of a problem".
    RestartedAfterProblem,
    ShuttingDown,
}

/// One boot frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub appearance: Appearance,
    /// 0..1 when the progress bar shows, `None` to hide it.
    pub progress: Option<f32>,
    pub screen: Screen,
}

impl Frame {
    /// The loader's frame: the mark and the bar at 0 (the first thing every stage shows), in Dark.
    pub fn boot(width: u32, height: u32) -> Frame {
        Frame { width, height, appearance: Appearance::Dark, progress: Some(0.0), screen: Screen::Boot }
    }
}

/// How long the bar takes to fill to the end once the first screen is ready, before the fade (ms).
pub const FILL_MS: f32 = 300.0;
/// How long the boot frame takes to fade into the first screen (ms).
pub const FADE_MS: f32 = 450.0;

/// How full the bar is when the first screen is ready: the share of the boot already behind (boot so far against
/// the fill still to come), so a slow boot starts nearer the end. At most 95 %.
pub fn first_fill(boot_ms: f32) -> f32 {
    (boot_ms / (boot_ms + FILL_MS * 2.0)).clamp(0.0, 0.95)
}

/// The smoothstep the fades use: 0 at 0, 1 at 1, slow at both ends.
fn smoothstep(k: f32) -> f32 {
    let e = k.clamp(0.0, 1.0);
    e * e * (3.0 - 2.0 * e)
}

/// The boot frame's opacity over the screen at fade progress `k` (0..1): 1 at the start, 0 at the end.
pub fn fade_weight(k: f32) -> f32 {
    1.0 - smoothstep(k)
}

/// Blends the boot frame `splash` over `frame` with weight `fade_weight(k)` (k = fade progress 0..1). Per channel,
/// so it is the same for both pixel orders.
pub fn fade(frame: &mut [u32], splash: &[u32], k: f32) {
    let a = (255.0 * fade_weight(k)) as u32;
    if a == 0 {
        return;
    }
    let ia = 255 - a;
    for (o, &s) in frame.iter_mut().zip(splash) {
        let f = *o;
        let ch = |sh: u32| ((((s >> sh) & 255) * a + ((f >> sh) & 255) * ia) / 255) << sh;
        *o = 0xff00_0000 | ch(16) | ch(8) | ch(0);
    }
}

/// The boot frame's hand-off to the first screen: the bar fills to the end over `FILL_MS`, then the frame
/// cross-fades into the screen over `FADE_MS`, the reference desktop's boot-to-login order. Frames are `w` pixels
/// per row (stride `w`).
pub struct Handoff {
    splash: Vec<u32>,
    /// `splash` with the bar full, the frame the fade starts from (kept so the fade allocates nothing per tick)
    full: Vec<u32>,
    w: u32,
    h: u32,
    appearance: Appearance,
    order: PixelOrder,
    /// (first-screen time, the bar's fill then), both in ms
    start: Option<(f32, f32)>,
}

impl Handoff {
    /// The frame to hand over from: the loader's frame (`Frame::boot`) rendered at this size.
    pub fn new(width: u32, height: u32, appearance: Appearance, order: PixelOrder) -> Handoff {
        let mut splash = alloc::vec![0u32; (width as usize) * (height as usize)];
        let frame = Frame { width, height, appearance, progress: Some(0.0), screen: Screen::Boot };
        render(&frame, &mut splash, width as usize, order);
        let mut full = splash.clone();
        render_bar(width, height, 1.0, appearance, full.as_mut_slice(), width as usize, order);
        Handoff { splash, full, w: width, h: height, appearance, order, start: None }
    }

    /// True once the first screen arrived (the first `compose`).
    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    /// Writes the boot frame with the bar at its hand-off fill: what the screen shows while the desktop is not ready
    /// yet (reveal once). Does not start the hand-off.
    pub fn hold(&self, out: &mut Vec<u32>, boot_ms: f32) {
        out.clear();
        out.extend_from_slice(&self.splash);
        render_bar(self.w, self.h, first_fill(boot_ms), self.appearance, out.as_mut_slice(), self.w as usize, self.order);
    }

    /// Writes into `out` what to show at `now` (ms) over the screen `ui`, `boot_ms` the boot clock at the first call.
    /// Returns false once the hand-off is over: show `ui` as it is.
    pub fn compose(&mut self, ui: &[u32], out: &mut Vec<u32>, now: f32, boot_ms: f32) -> bool {
        let (from, fill0) = *self.start.get_or_insert((now, first_fill(boot_ms)));
        let t = now - from;
        if t >= FILL_MS + FADE_MS {
            return false;
        }
        out.clear();
        let stride = self.w as usize;
        if t < FILL_MS {
            // ease-out: the bar arrives at the end gently
            let r = 1.0 - t / FILL_MS;
            let e = 1.0 - r * r * r;
            out.extend_from_slice(&self.splash);
            render_bar(self.w, self.h, fill0 + (1.0 - fill0) * e, self.appearance, out.as_mut_slice(), stride, self.order);
        } else {
            out.extend_from_slice(ui);
            fade(out.as_mut_slice(), &self.full, (t - FILL_MS) / FADE_MS);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn hold_shows_the_boot_frame_without_starting_the_handoff() {
        let (w, h) = (320u32, 180u32);
        let hand = Handoff::new(w, h, Appearance::Dark, PixelOrder::Bgrx);
        let mut held = Vec::new();
        hand.hold(&mut held, 1700.0);
        assert_eq!(held.len(), (w * h) as usize);
        assert!(!hand.started());
        let mut expect = hand.splash.clone();
        render_bar(w, h, first_fill(1700.0), Appearance::Dark, expect.as_mut_slice(), w as usize, PixelOrder::Bgrx);
        assert_eq!(held, expect);
    }

    #[test]
    fn first_fill_follows_the_boot_so_far() {
        assert_eq!(first_fill(0.0), 0.0);
        // 1700 / (1700 + 600) = 0.739
        assert!((first_fill(1700.0) - 1700.0 / 2300.0).abs() < 1e-6);
        // 60000 / 60600 = 0.99, clamped to 0.95
        assert_eq!(first_fill(60_000.0), 0.95);
    }

    #[test]
    fn fade_weight_is_smoothstep_from_one_to_zero() {
        assert_eq!(fade_weight(0.0), 1.0);
        assert_eq!(fade_weight(1.0), 0.0);
        assert!((fade_weight(0.5) - 0.5).abs() < 1e-6, "symmetric");
        // at 0.25: smoothstep = 3*0.0625 - 2*0.015625 = 0.15625, weight 0.84375
        assert!((fade_weight(0.25) - 0.84375).abs() < 1e-6);
        assert_eq!(fade_weight(3.0), 0.0, "clamped");
    }

    #[test]
    fn fade_starts_on_the_boot_frame_and_ends_on_the_screen() {
        let splash = vec![0xff00_0000u32; 4];
        let mut f = vec![0xffff_ffffu32; 4];
        fade(&mut f, &splash, 0.0);
        assert_eq!(f[0], 0xff00_0000, "k = 0 is the boot frame");
        let mut f = vec![0xffff_ffffu32; 4];
        fade(&mut f, &splash, 1.0);
        assert_eq!(f[0], 0xffff_ffff, "k = 1 is the screen");
    }

    #[test]
    fn frame_boot_is_the_loader_frame_in_dark() {
        let f = Frame::boot(1920, 1080);
        assert_eq!(f.appearance, Appearance::Dark);
        assert_eq!(f.progress, Some(0.0));
        assert_eq!(f.screen, Screen::Boot);
        assert_eq!(Appearance::default(), Appearance::Dark);
    }

    #[test]
    fn handoff_fills_the_bar_then_fades_into_the_screen() {
        let (w, h) = (640u32, 360u32);
        let ui = vec![0xff10_2030u32; (w * h) as usize];
        let mut hand = Handoff::new(w, h, Appearance::Dark, PixelOrder::Bgrx);
        let mut out = Vec::new();
        assert!(!hand.started());
        // the bar at 640x360 is x 290..350, rows 207..209 (bh clamped to 2)
        let (bx, by, bw, _) = bar_rect(w, h);
        assert_eq!((bx, by, bw), (290, 207, 60));
        let bar_px = |out: &[u32], frac: f32| out[by as usize * w as usize + bx as usize + (bw as f32 * frac) as usize];
        assert!(hand.compose(&ui, &mut out, 1000.0, 1700.0));
        let f0 = first_fill(1700.0);
        assert!(f0 > 0.6 && f0 < 0.95, "{f0}");
        assert_eq!(bar_px(&out, f0 - 0.05), 0xffe5_e5ea, "already filled up to the boot so far");
        assert_eq!(bar_px(&out, 0.99), 0xff3a_3a3c, "the end still empty");
        assert!(hand.compose(&ui, &mut out, 1000.0 + FILL_MS - 1.0, 0.0));
        assert_eq!(bar_px(&out, 0.99), 0xffe5_e5ea, "full by the end of the fill");
        assert_eq!(out[0], 0xff00_0000, "still the boot frame, no screen yet");
        assert!(hand.compose(&ui, &mut out, 1000.0 + FILL_MS + FADE_MS - 1.0, 0.0));
        assert_ne!(out[0], 0xff00_0000, "fading into the screen");
        assert!(!hand.compose(&ui, &mut out, 1000.0 + FILL_MS + FADE_MS, 0.0), "done: show the screen");
    }

    #[test]
    fn handoff_frame_is_the_same_as_the_loader_frame_at_the_handover() {
        // the first hand-off frame at zero fill is exactly the boot frame rendered by `render`
        let (w, h) = (640u32, 360u32);
        let mut hand = Handoff::new(w, h, Appearance::Light, PixelOrder::Bgrx);
        let ui = vec![0u32; (w * h) as usize];
        let mut out = Vec::new();
        hand.compose(&ui, &mut out, 0.0, 0.0);
        let mut boot = vec![0u32; (w * h) as usize];
        render(&Frame { width: w, height: h, appearance: Appearance::Light, progress: Some(0.0), screen: Screen::Boot }, &mut boot, w as usize, PixelOrder::Bgrx);
        // at t = 0 the fill is first_fill(0) = 0, so the bar matches the track only
        assert_eq!(out[0], boot[0]);
        assert_eq!(out[(w * h / 2) as usize], boot[(w * h / 2) as usize]);
    }
}
