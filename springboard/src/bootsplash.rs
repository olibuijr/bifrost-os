//! The boot frame the EFI loader paints (tools/splash-efi), redrawn pixel for pixel so the first KMS frame of the
//! installer or desktop matches what simpledrm was still showing, then cross-fades into the UI. Also the shared boot
//! milestone log: every mark is milliseconds since the kernel started (CLOCK_BOOTTIME), one clock for all steps.

const LOGO: &[u8] = include_bytes!("../../tools/splash-efi/logo.bgrx");
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
