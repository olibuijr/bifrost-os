//! Controls shared by the Settings pages: the bifrost switch with its slide
//! and colour cross-fade, sliders and the disclosure chevron.

use super::icons::disc;
use super::ui::{Ui, SECONDARY, TRACK_C, WHITE};
use crate::gfx::{over, Image, Surface};

/// Switch geometry in points (NativeSwitch).
pub const SWITCH_W_PT: f32 = 38.0;
pub const SWITCH_H_PT: f32 = 22.0;
/// How long the knob takes to slide across.
pub const SWITCH_MS: f64 = 200.0;
/// Slider knob and track in points (NativeSlider).
pub const KNOB_PT: f32 = 21.0;
pub const SLIDER_TRACK_PT: f32 = 4.0;

/// Where a switch is between off (0) and on (1) at `t`: `on` is its
/// resting state, `since` when it last flipped.
pub fn switch_progress(on: bool, since: f64, t: f64) -> f32 {
    let p = ((t - since) / SWITCH_MS).clamp(0.0, 1.0) as f32;
    let e = 1.0 - (1.0 - p).powi(3);
    if on {
        e
    } else {
        1.0 - e
    }
}

/// Mixes two premultiplied colours channel by channel.
fn mix(a: u32, b: u32, k: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 255) as f32, ((b >> sh) & 255) as f32);
        ((x + (y - x) * k).round() as u32) << sh
    };
    ch(0) | ch(8) | ch(16) | ch(24)
}

/// bifrost switch, 51 x 31 pt with (x, y) its top-left, `k` of the way to on.
pub fn draw_switch(s: &mut Surface, ui: &Ui, x: f32, y: f32, k: f32) {
    let (w, h, knob) = (ui.pt(SWITCH_W_PT), ui.pt(SWITCH_H_PT), ui.pt(18.0));
    s.fill_rrect(x, y, w, h, h / 2.0, mix(TRACK_C.now(), bifrost_render::appearance::accent_now(), k));
    disc(s, x + ui.pt(2.0) + knob / 2.0 + (w - ui.pt(4.0) - knob) * k, y + ui.pt(2.0) + knob / 2.0 + ui.pt(0.7), knob / 2.0 + ui.pt(0.4), crate::gfx::rgba(0, 0, 0, 40));
    let kx = x + ui.pt(2.0) + (w - ui.pt(4.0) - knob) * k;
    s.fill_rrect(kx, y + ui.pt(2.0), knob, knob, knob / 2.0, WHITE);
}

/// A slider track from `x` to `x + w` centred on `y`, filled to `value`
/// (0..=1) in blue with the white knob on top.
pub fn draw_slider(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, value: f32) {
    let th = ui.pt(SLIDER_TRACK_PT);
    s.fill_rrect(x, y - th / 2.0, w, th, th / 2.0, TRACK_C.now());
    s.fill_rrect(x, y - th / 2.0, w * value, th, th / 2.0, bifrost_render::appearance::accent_now());
    let k = ui.pt(KNOB_PT);
    // the knob's soft shadow, as NativeSlider draws it
    disc(s, x + w * value, y + ui.pt(1.0), k / 2.0 + ui.pt(0.5), crate::gfx::rgba(0, 0, 0, 60));
    disc(s, x + w * value, y, k / 2.0, WHITE);
}

/// Slider value for a touch at `x` along the track.
pub fn slider_value(x: f32, track_x: f32, track_w: f32) -> f32 {
    ((x - track_x) / track_w).clamp(0.0, 1.0)
}

/// Snaps a 0..=1 slider value to the nearest of `n` notches, as an index.
pub fn notch(value: f32, n: usize) -> usize {
    ((value * (n as f32 - 1.0)).round() as usize).min(n - 1)
}

/// The disclosure chevron ›, drawn as two strokes in the separator grey.
pub fn chevron(ui: &Ui) -> Image {
    let (w, h) = (ui.pt(8.0).ceil() as usize + 2, ui.pt(14.0).ceil() as usize + 2);
    let mut img = Image::new(w, h);
    let mut s = Surface { w, h, stride: w, px: &mut img.px, y0: 0 };
    let (x0, x1, mid) = (ui.pt(1.5), ui.pt(1.5) + ui.pt(6.0), h as f32 / 2.0);
    let t = ui.pt(1.7);
    super::icons::line(&mut s, (x0, mid - ui.pt(6.0)), (x1, mid), t, SECONDARY);
    super::icons::line(&mut s, (x1, mid), (x0, mid + ui.pt(6.0)), t, SECONDARY);
    img
}

/// Blends `src` over `dst` with `alpha` (0..=255) on top of the source alpha.
pub fn blend(dst: u32, src: u32, alpha: u32) -> u32 {
    over(dst, crate::gfx::scale(src, alpha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::icons::GREEN;
    use crate::shell::ui::TRACK;

    #[test]
    fn switch_slides_over_two_hundred_ms() {
        assert_eq!(switch_progress(true, 0.0, -1.0), 0.0);
        assert_eq!(switch_progress(true, 0.0, SWITCH_MS), 1.0);
        let mid = switch_progress(true, 0.0, 60.0);
        assert!(mid > 0.3 && mid < 1.0, "ease-out starts fast: {mid}");
        assert_eq!(switch_progress(false, 0.0, 0.0), 1.0);
        assert_eq!(switch_progress(false, 0.0, 500.0), 0.0);
        assert_eq!(mix(TRACK, GREEN, 0.0), TRACK);
        assert_eq!(mix(TRACK, GREEN, 1.0), GREEN);
    }

    #[test]
    fn drawn_switch_moves_its_knob() {
        let ui = crate::shell::tests::ui();
        let knob_at = |k: f32| {
            let mut buf = vec![0u32; 120 * 60];
            draw_switch(&mut Surface { w: 120, h: 60, stride: 120, px: &mut buf, y0: 0 }, &ui, 5.0, 5.0, k);
            (0..120).filter(|&x| buf[30 * 120 + x] == WHITE).sum::<usize>() as f32 / (0..120).filter(|&x| buf[30 * 120 + x] == WHITE).count().max(1) as f32
        };
        assert!(knob_at(1.0) > knob_at(0.5) && knob_at(0.5) > knob_at(0.0));
    }

    #[test]
    fn slider_value_and_notches() {
        assert_eq!(slider_value(100.0, 100.0, 400.0), 0.0);
        assert_eq!(slider_value(300.0, 100.0, 400.0), 0.5);
        assert_eq!(slider_value(900.0, 100.0, 400.0), 1.0);
        assert_eq!(notch(0.0, 7), 0);
        assert_eq!(notch(0.5, 7), 3);
        assert_eq!(notch(0.99, 7), 6);
        assert_eq!(notch(0.6, 2), 1);
        let ui = crate::shell::tests::ui();
        let mut buf = vec![0u32; 400 * 60];
        draw_slider(&mut Surface { w: 400, h: 60, stride: 400, px: &mut buf, y0: 0 }, &ui, 20.0, 30.0, 300.0, 0.5);
        assert_eq!(buf[30 * 400 + 100], bifrost_render::appearance::accent_now(), "filled left of the knob");
        assert_eq!(buf[30 * 400 + 300], TRACK_C.now(), "empty right of the knob");
        assert_eq!(buf[30 * 400 + 170], WHITE, "the knob");
        let ch = chevron(&ui);
        assert!(ch.px.iter().any(|&p| p >> 24 > 128));
    }
}
