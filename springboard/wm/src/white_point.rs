//! Night Shift for the whole output (`bifrost_window_level_v1` version 5, `set_white_point`; docs/gpu-first.md "Night
//! Shift"). The shell sends the warm white point it wants; the window server fades to it here, in one place, and shows
//! everything with it: the shell's surfaces and every app's window alike.
//!
//! Two ways, chosen per display (`Mode`): the CRTC's gamma ramp (`ramp`, programmed by `drm.rs`; the scanout does the
//! multiply, no GPU work), or, when the CRTC has no ramp or refuses one, a final multiply pass in the GLES compositor
//! (`MultiplyElement`, drawn over the whole scene with blend DST_COLOR, ZERO). The Vulkan scanout has no such pass yet,
//! so without a ramp it reports `Mode::None` and the shell keeps tinting its own surfaces.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement},
        gles::{ffi, GlesError, GlesFrame, GlesRenderer},
        utils::{CommitCounter, OpaqueRegions},
        Color32F,
    },
    utils::{Buffer, Physical, Rectangle, Scale},
};

/// The multiplier that is 1.0 on the wire (`night_shift::multipliers` in the shell uses the same scale).
pub const ONE: u32 = 256;
/// The fade from one white point to the next covers the whole range (1.0) in this long, as the shell's own did.
pub const FADE_MS: f32 = 1_500.0;
/// A fade step reaches the gamma ramp at most this often: each legacy gamma set is a blocking commit in the kernel,
/// and a slow warm fade at 30 steps a second is as smooth as one at 60.
pub const RAMP_STEP: Duration = Duration::from_millis(33);
/// Off: every channel at 1.0.
pub const WHITE: [f32; 3] = [1.0; 3];

/// How the window server applies the white point (`white_point_mode` in the protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Not applied: the backend has neither a gamma ramp nor a pass for it (nested, headless, Vulkan without a ramp).
    #[default]
    None,
    /// The CRTC's gamma ramp.
    Hardware,
    /// A final multiply pass of the GLES compositor.
    Composited,
}

impl Mode {
    /// The protocol value (`white_point_mode` enum).
    pub fn wire(self) -> u32 {
        match self {
            Mode::None => 0,
            Mode::Hardware => 1,
            Mode::Composited => 2,
        }
    }
}

/// The wire multipliers (256 = 1.0) as factors, clamped to 1.0: Night Shift only takes light away.
pub fn factors(m: [u32; 3]) -> [f32; 3] {
    m.map(|v| v.min(ONE) as f32 / ONE as f32)
}

/// One step of the fade: `cur` moves towards `target` along a straight line in RGB, so the white point does not change
/// hue on the way, covering the whole range (1.0) in `FADE_MS`. `dt_ms` is the time since the previous step.
pub fn step(cur: [f32; 3], target: [f32; 3], dt_ms: f32) -> [f32; 3] {
    let d = (0..3).map(|i| (target[i] - cur[i]).abs()).fold(0.0f32, f32::max);
    let step = dt_ms.max(0.0) / FADE_MS;
    if d <= step || d < 1e-4 {
        return target;
    }
    let k = step / d;
    [cur[0] + (target[0] - cur[0]) * k, cur[1] + (target[1] - cur[1]) * k, cur[2] + (target[2] - cur[2]) * k]
}

/// The output's white point: the shell's target, the value in force while it fades there, and how it is applied.
#[derive(Debug, Clone, Copy)]
pub struct WhitePoint {
    target: [f32; 3],
    current: [f32; 3],
    /// When the fade last stepped (`advance`).
    at: Option<Instant>,
    /// Set by the backend when it starts (and when a ramp is refused); reported to the shell on bind.
    pub mode: Mode,
}

impl Default for WhitePoint {
    fn default() -> Self {
        WhitePoint { target: WHITE, current: WHITE, at: None, mode: Mode::None }
    }
}

impl WhitePoint {
    /// The shell's new target (`set_white_point`); the fade towards it starts on the next `advance`.
    pub fn set_target(&mut self, m: [u32; 3]) {
        self.target = factors(m);
    }

    pub fn target(&self) -> [f32; 3] {
        self.target
    }

    /// The white point in force.
    pub fn current(&self) -> [f32; 3] {
        self.current
    }

    /// True while the white point is still on its way to the target.
    pub fn fading(&self) -> bool {
        self.current != self.target
    }

    /// Steps the fade to `now`; true when the white point in force changed. A long gap since the last step (nothing
    /// was drawn) counts as one frame, so a target that arrives on a quiet desktop still fades in.
    pub fn advance(&mut self, now: Instant) -> bool {
        let dt = self.at.map_or(16.0, |t| (now.saturating_duration_since(t).as_secs_f32() * 1000.0).min(50.0));
        self.at = Some(now);
        if !self.fading() {
            return false;
        }
        self.current = step(self.current, self.target, dt);
        true
    }
}

/// True when the ramp read back from the CRTC can serve as the base the white point multiplies: as long as asked for,
/// and not all zero (a driver that keeps no ramp reads back zeros, which would black the display).
pub fn usable_base(base: &[u16], len: usize) -> bool {
    len > 1 && base.len() >= len && base[..len].iter().any(|&v| v != 0)
}

/// True when the read-back ramps look like a tint left on the CRTC (a window server that died warm) rather than the
/// display's calibration: the channels' top entries differ by more than a tenth. Such a base is replaced by the
/// identity, so a restart does not stack a second warm white point on the first.
pub fn leftover_tint(r: &[u16], g: &[u16], b: &[u16]) -> bool {
    let top = [r, g, b].map(|c| c.last().copied().unwrap_or(0) as f32);
    let (lo, hi) = (top.iter().copied().fold(f32::MAX, f32::min), top.iter().copied().fold(0.0f32, f32::max));
    hi > 0.0 && (hi - lo) / hi > 0.1
}

/// The identity ramp of `len` entries (0 to 0xFFFF).
pub fn linear(len: usize) -> Vec<u16> {
    let last = len.saturating_sub(1).max(1) as u32;
    (0..len as u32).map(|i| (i * 0xFFFF / last) as u16).collect()
}

/// One channel's ramp: the display's own ramp `base` (its calibration is kept) times `factor`.
pub fn ramp(base: &[u16], factor: f32) -> Vec<u16> {
    let f = factor.clamp(0.0, 1.0);
    base.iter().map(|&v| (v as f32 * f).round() as u16).collect()
}

/// Whether a fade step should reach the ramp now: always for the last step (the target lands exactly), else at most
/// once per `RAMP_STEP`.
pub fn ramp_due(last: Option<Instant>, now: Instant, landed: bool) -> bool {
    landed || last.is_none_or(|t| now.saturating_duration_since(t) >= RAMP_STEP)
}

/// The final pass of `Mode::Composited`: one rectangle over the whole output whose colour multiplies what is under it
/// (blend DST_COLOR, ZERO; alpha is multiplied by 1 and kept). It is the frontmost element, so nothing under it is put
/// on a plane of its own past the multiply, and the pointer is tinted too. Its id is kept between frames, and its commit
/// moves only when the colour or the size changes, so a still white point adds no damage.
#[derive(Debug, Clone)]
pub struct MultiplyElement {
    id: Id,
    commit: CommitCounter,
    rect: Rectangle<i32, Physical>,
    color: [f32; 3],
}

impl Default for MultiplyElement {
    fn default() -> Self {
        MultiplyElement { id: Id::new(), commit: CommitCounter::default(), rect: Rectangle::default(), color: WHITE }
    }
}

impl MultiplyElement {
    /// Updates the element to `color` over an output of `size` physical px (the panel's mode in the output's
    /// orientation, so the pass covers it whatever the scale); a change damages the whole output.
    pub fn update(&mut self, size: (i32, i32), color: [f32; 3]) {
        let rect = Rectangle::from_size(size.into());
        if rect != self.rect || color != self.color {
            self.rect = rect;
            self.color = color;
            self.commit.increment();
        }
    }

    /// True when the pass would change nothing (the white point is off).
    pub fn is_identity(&self) -> bool {
        self.color == WHITE
    }

    pub fn color(&self) -> [f32; 3] {
        self.color
    }
}

impl Element for MultiplyElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        self.commit
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((self.rect.size.w as f64, self.rect.size.h as f64).into())
    }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.rect
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // it shows what is under it, multiplied
        OpaqueRegions::default()
    }
    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for MultiplyElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let [r, g, b] = self.color;
        // SAFETY: plain GL state calls on the frame's current context; the renderer's blend state is put back after
        frame.with_context(|gl| unsafe { gl.BlendFunc(ffi::DST_COLOR, ffi::ZERO) })?;
        // the inherent draw_solid keeps the blend state set above (the trait method would disable blending for an
        // opaque colour)
        let drawn = frame.draw_solid(dst, damage, Color32F::new(r, g, b, 1.0));
        frame.with_context(|gl| unsafe { gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA) })?;
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_scale_is_256_and_more_is_white() {
        assert_eq!(factors([256, 256, 256]), WHITE);
        assert_eq!(factors([256, 128, 64]), [1.0, 0.5, 0.25]);
        assert_eq!(factors([999, 300, 0]), [1.0, 1.0, 0.0], "Night Shift never brightens");
    }

    #[test]
    fn the_modes_are_the_protocol_values() {
        let xml = include_str!("../protocol/bifrost-window-level-v1.xml");
        for (mode, name) in [(Mode::None, "none"), (Mode::Hardware, "hardware"), (Mode::Composited, "composited")] {
            assert!(xml.contains(&format!("<entry name=\"{name}\" value=\"{}\"", mode.wire())), "{name}");
        }
        assert_eq!(Mode::default(), Mode::None, "a backend that does nothing reports none");
    }

    #[test]
    fn the_fade_runs_along_a_straight_line_and_lands_on_the_target() {
        let warm = factors([256, 220, 160]);
        let mut cur = WHITE;
        let mut steps = 0;
        while cur != warm {
            let next = step(cur, warm, 16.0);
            assert_eq!(next[0], 1.0, "red stays");
            let (dg, db) = (1.0 - next[1], 1.0 - next[2]);
            if db > 1e-3 {
                assert!((dg / db - 36.0 / 96.0).abs() < 1e-3, "no hue shift on the way: {next:?}");
            }
            cur = next;
            steps += 1;
            assert!(steps < 1000);
        }
        assert!(steps > 20 && steps < 50, "a fade, not a jump: {steps}");
        assert_eq!(step(warm, WHITE, FADE_MS), WHITE, "a long gap lands on the target");
    }

    #[test]
    fn the_white_point_fades_from_set_target_and_settles() {
        let mut wp = WhitePoint::default();
        let t0 = Instant::now();
        assert!(!wp.advance(t0), "nothing to do while off");
        wp.set_target([256, 200, 120]);
        assert!(wp.fading());
        let mut t = t0;
        let mut n = 0;
        while wp.advance(t) {
            t += Duration::from_millis(16);
            n += 1;
            assert!(n < 500);
        }
        assert_eq!(wp.current(), factors([256, 200, 120]));
        assert!(!wp.fading());
        wp.set_target([256, 256, 256]);
        assert!(wp.fading(), "turning it off fades back too");
    }

    #[test]
    fn a_long_quiet_gap_counts_as_one_frame() {
        let mut wp = WhitePoint::default();
        let t0 = Instant::now();
        wp.advance(t0);
        wp.set_target([256, 0, 0]);
        assert!(wp.advance(t0 + Duration::from_secs(60)));
        assert!(wp.fading(), "one 50 ms step, not the whole fade at once");
    }

    #[test]
    fn the_ramp_multiplies_the_displays_own_ramp() {
        let base = linear(256);
        assert_eq!((base[0], base[255]), (0, 0xFFFF));
        assert_eq!(ramp(&base, 1.0), base, "white keeps the calibration as it is");
        let half = ramp(&base, 0.5);
        assert_eq!(half[255], 0x8000);
        assert!(half.windows(2).all(|w| w[0] <= w[1]), "still monotonic");
        let curve: Vec<u16> = (0..256u32).map(|i| ((i * i) * 0xFFFF / (255 * 255)) as u16).collect();
        assert_eq!(ramp(&curve, 0.25)[255], 0x4000, "a calibrated curve is scaled, not replaced");
        assert!(ramp(&base, 7.0) == base && ramp(&base, -1.0).iter().all(|&v| v == 0), "factors are clamped");
    }

    #[test]
    fn a_zero_or_short_read_back_is_not_a_base() {
        assert!(usable_base(&linear(1024), 1024));
        assert!(!usable_base(&vec![0; 1024], 1024), "a driver without a ramp reads zeros");
        assert!(!usable_base(&linear(256), 1024));
        assert!(!usable_base(&[], 0), "no ramp at all");
    }

    #[test]
    fn a_warm_ramp_left_on_the_crtc_is_not_taken_as_calibration() {
        let base = linear(256);
        assert!(!leftover_tint(&base, &base, &base));
        let calibrated = ramp(&base, 0.97);
        assert!(!leftover_tint(&base, &base, &calibrated), "a few percent between channels is calibration");
        assert!(leftover_tint(&base, &ramp(&base, 0.86), &ramp(&base, 0.62)), "a Night Shift ramp is a leftover");
    }

    #[test]
    fn ramp_steps_are_throttled_but_the_last_one_always_lands() {
        let t = Instant::now();
        assert!(ramp_due(None, t, false), "the first step goes out");
        assert!(!ramp_due(Some(t), t + Duration::from_millis(10), false));
        assert!(ramp_due(Some(t), t + RAMP_STEP, false));
        assert!(ramp_due(Some(t), t + Duration::from_millis(1), true), "the target is never skipped");
    }

    #[test]
    fn the_multiply_pass_damages_only_on_a_change() {
        let mut e = MultiplyElement::default();
        assert!(e.is_identity());
        let c0 = e.current_commit();
        e.update((1920, 1200), WHITE);
        let c1 = e.current_commit();
        assert_ne!(c0, c1, "the first size is a change");
        e.update((1920, 1200), WHITE);
        assert_eq!(e.current_commit(), c1, "a still white point adds no damage");
        e.update((1920, 1200), [1.0, 0.8, 0.6]);
        assert_ne!(e.current_commit(), c1);
        assert!(!e.is_identity());
        assert_eq!(e.geometry(Scale::from(1.0)), Rectangle::from_size((1920, 1200).into()));
    }
}
