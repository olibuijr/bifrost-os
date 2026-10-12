//! The boot frame the EFI loader paints (springboard/bootweave/efi), redrawn pixel for pixel so the first KMS frame of the
//! installer or desktop matches what simpledrm was still showing, then cross-fades into the UI. Also the shared boot
//! milestone log: every mark is milliseconds since the kernel started (CLOCK_BOOTTIME), one clock for all steps.

const LOGO: &[u8] = include_bytes!("../bootweave/efi/logo.bgrx");
const LOGO_PX: usize = 160;
/// How long the boot frame takes to fade into the first screen.
pub const FADE_MS: f64 = 450.0;
pub const MARKS: &str = "/run/bifrost/boot-marks";

/// Paints the loader's frame into `px` (0xAARRGGBB, `w`x`h`, stride `w`). Same arithmetic as the loader's `paint`.
pub fn paint(px: &mut [u32], w: usize, h: usize) {
    px[..w * h].fill(0xff00_0000);
    let l = (LOGO_PX * h / 1440).max(16);
    let (x, y) = ((w.saturating_sub(l)) / 2, (h / 2).saturating_sub(110 * h / 1440));
    for j in 0..l.min(h - y) {
        for i in 0..l.min(w - x) {
            let (sx, sy) = (i * LOGO_PX / l, j * LOGO_PX / l);
            let c = &LOGO[(sy * LOGO_PX + sx) * 4..][..4];
            px[(y + j) * w + x + i] = 0xff00_0000 | (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32;
        }
    }
    let (bw, bh) = (240 * h / 1440, (4 * h / 1440).max(2));
    let (bx, by) = ((w.saturating_sub(bw)) / 2, h / 2 + 110 * h / 1440);
    for row in px[by * w..].chunks_mut(w).take(bh) {
        row[bx..bx + bw].fill(0xff3c_3c3e);
    }
}

/// How long the loader's bar takes to fill to the end once the first screen is ready, before the fade.
pub const FILL_MS: f64 = 300.0;
/// The bar's fill colour (the track is the loader's grey).
const BAR_FILL: u32 = 0xffe5_e5ea;

/// Fills the loader frame's bar to `fill` (0..1) from the left, over `px` as `paint` left it.
pub fn paint_bar(px: &mut [u32], w: usize, h: usize, fill: f64) {
    let (bw, bh) = (240 * h / 1440, (4 * h / 1440).max(2));
    let (bx, by) = ((w.saturating_sub(bw)) / 2, h / 2 + 110 * h / 1440);
    let n = (bw as f64 * fill.clamp(0.0, 1.0)).round() as usize;
    for row in px[by * w..].chunks_mut(w).take(bh) {
        row[bx..bx + n].fill(BAR_FILL);
    }
}

/// How full the bar is when the first screen is ready: the share of the boot already behind (boot so far against
/// the fill still to come), so a slow boot starts nearer the end. At most 95 %.
pub fn first_fill(boot_ms: f64) -> f64 {
    (boot_ms / (boot_ms + FILL_MS * 2.0)).clamp(0.0, 0.95)
}

/// The boot frame's hand-off to the first screen: the bar fills to the end over `FILL_MS`, then the frame
/// cross-fades into the screen over `FADE_MS`, the reference desktop's boot-to-login order.
pub struct Handoff {
    splash: Vec<u32>,
    w: usize,
    h: usize,
    /// (first-screen time, the bar's fill then)
    start: Option<(f64, f64)>,
}

impl Handoff {
    pub fn new(w: usize, h: usize) -> Self {
        let mut splash = vec![0; w * h];
        paint(&mut splash, w, h);
        Handoff { splash, w, h, start: None }
    }

    /// True once the first screen arrived (the first `compose`).
    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    /// Writes into `out` what to show at `now` (ms) over the screen `ui`, `boot_ms` the boot clock at the first call.
    /// Returns false once the hand-off is over: show `ui` as it is.
    pub fn compose(&mut self, ui: &[u32], out: &mut Vec<u32>, now: f64, boot_ms: f64) -> bool {
        let (from, fill0) = *self.start.get_or_insert((now, first_fill(boot_ms)));
        let t = now - from;
        if t >= FILL_MS + FADE_MS {
            return false;
        }
        out.clear();
        if t < FILL_MS {
            // ease-out: the bar arrives at the end gently
            let e = 1.0 - (1.0 - t / FILL_MS).powi(3);
            out.extend_from_slice(&self.splash);
            paint_bar(out, self.w, self.h, fill0 + (1.0 - fill0) * e);
        } else {
            let mut full = self.splash.clone();
            paint_bar(&mut full, self.w, self.h, 1.0);
            out.extend_from_slice(ui);
            fade(out, &full, (t - FILL_MS) / FADE_MS);
        }
        true
    }
}

/// Blends the boot frame over `frame` with weight `1 - k` (k = fade progress 0..1, eased).
pub fn fade(frame: &mut [u32], splash: &[u32], k: f64) {
    let e = k.clamp(0.0, 1.0);
    let a = (255.0 * (1.0 - e * e * (3.0 - 2.0 * e))) as u32;
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

/// Milliseconds since the kernel started, including suspend (the clock systemd-analyze also counts from).
pub fn boottime_ms() -> f64 {
    let mut ts = libc_timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(7 /* CLOCK_BOOTTIME */, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}
#[repr(C)]
struct libc_timespec {
    tv_sec: i64,
    tv_nsec: i64,
}
extern "C" {
    fn clock_gettime(clk: i32, ts: *mut libc_timespec) -> i32;
}

/// Appends `ms event` to the boot milestone log; the first mark of an event wins (a restart does not overwrite it).
pub fn mark(event: &str) {
    use std::io::Write;
    if !event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return;
    }
    let _ = std::fs::create_dir_all("/run/bifrost");
    let seen = std::fs::read_to_string(MARKS).unwrap_or_default();
    if seen.lines().any(|l| l.split_whitespace().nth(1) == Some(event)) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(MARKS) {
        let _ = writeln!(f, "{:.1} {event}", boottime_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar_px(px: &[u32], w: usize, h: usize, fill: f64) -> u32 {
        let (bw, by) = (240 * h / 1440, h / 2 + 110 * h / 1440);
        let x = (w - bw) / 2 + ((bw as f64 * fill) as usize).min(bw - 1);
        px[by * w + x]
    }

    #[test]
    fn handoff_fills_the_bar_then_fades_into_the_screen() {
        let (w, h) = (640, 360);
        let ui = vec![0xff10_2030u32; w * h];
        let mut hand = Handoff::new(w, h);
        let mut out = Vec::new();
        assert!(!hand.started());
        assert!(hand.compose(&ui, &mut out, 1000.0, 1700.0));
        let f0 = first_fill(1700.0);
        assert!(f0 > 0.6 && f0 < 0.95, "{f0}");
        assert_eq!(bar_px(&out, w, h, f0 - 0.05), BAR_FILL, "already filled up to the boot so far");
        assert_eq!(bar_px(&out, w, h, 0.99), 0xff3c_3c3e, "the end still empty");
        assert!(hand.compose(&ui, &mut out, 1000.0 + FILL_MS - 1.0, 0.0));
        assert_eq!(bar_px(&out, w, h, 0.99), BAR_FILL, "full by the end of the fill");
        assert_eq!(out[0], 0xff00_0000, "still the boot frame, no screen yet");
        assert!(hand.compose(&ui, &mut out, 1000.0 + FILL_MS + FADE_MS - 1.0, 0.0));
        assert_ne!(out[0], 0xff00_0000, "fading into the screen");
        assert!(!hand.compose(&ui, &mut out, 1000.0 + FILL_MS + FADE_MS, 0.0), "done: show the screen");
    }
    #[test]
    fn frame_matches_the_loader_layout_and_fades_out() {
        let (w, h) = (1280, 720);
        let mut s = vec![0; w * h];
        paint(&mut s, w, h);
        assert_eq!(s[0], 0xff00_0000, "black outside the mark");
        assert_eq!(s[(h / 2 + 110 * h / 1440) * w + w / 2], 0xff3c_3c3e, "the progress bar sits 110/1440 below the middle");
        let mut f = vec![0xffff_ffff; w * h];
        fade(&mut f, &s, 0.0);
        assert_eq!(f[0], 0xff00_0000, "the fade starts on the boot frame");
        let mut f = vec![0xffff_ffff; w * h];
        fade(&mut f, &s, 1.0);
        assert_eq!(f[0], 0xffff_ffff, "and ends on the UI");
    }
}
