//! Host test: the shell's AppHost drives a real app process over a socketpair.
use bifrost_app::proto::{Configure, ToShell, TouchKind};
use springboard::gfx::{pack, Surface};
use springboard::shell::apphost::{AppHost, Status};
use std::path::Path;
use std::time::{Duration, Instant};

const BLUE: u32 = pack(0, 0, 255, 255);
const WHITE: u32 = pack(255, 255, 255, 255);

fn cfg() -> Configure {
    Configure { w: 64, h: 48, scale: 1.5, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false }
}

fn spawn(dir: &str) -> AppHost {
    let data = Path::new(env!("CARGO_TARGET_TMPDIR")).join(dir);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
    AppHost::spawn("is.olibuijr.testapp", Path::new(env!("CARGO_BIN_EXE_bifrost-testapp")), &data, &assets, cfg()).unwrap()
}

/// Pumps until `f` says stop or two seconds pass; returns the collected requests.
fn pump_until(h: &mut AppHost, mut f: impl FnMut(&AppHost, bool) -> bool) -> Vec<ToShell> {
    let end = Instant::now() + Duration::from_secs(5);
    let mut reqs = Vec::new();
    while Instant::now() < end {
        let p = h.pump();
        reqs.extend(p.requests);
        if f(h, p.frame) {
            return reqs;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

#[test]
fn frames_touches_damage_and_exit() {
    let mut h = spawn("apps-sdk-test-a");
    let mut screen = vec![0u32; 64 * 48];
    let mut s = Surface { w: 64, h: 48, stride: 64, px: &mut screen, y0: 0 };

    // first frame: the whole surface, blue
    pump_until(&mut h, |_, frame| frame);
    assert!(h.hello);
    let d = h.blit(&mut s, false);
    assert!(d.is_all());
    assert!(s.px.iter().all(|&p| p == BLUE));

    // three touches in a row: each frame carries only its box, earlier boxes survive
    // the double-buffer swap (the SDK brings the stale buffer up to date)
    for (i, (x, y)) in [(3, 4), (20, 30), (50, 10)].into_iter().enumerate() {
        h.touch(TouchKind::Down, 0, x as f32, y as f32, 0);
        pump_until(&mut h, |_, frame| frame);
        let d = h.blit(&mut s, false);
        assert_eq!(d.areas().len(), 1, "touch {i}");
        assert_eq!(d.pixels(), 16, "only the box is damaged");
        for (bx, by) in [(3, 4), (20, 30), (50, 10)].into_iter().take(i + 1) {
            assert_eq!(s.px[by * 64 + bx], WHITE, "box at {bx},{by} after touch {i}");
        }
        assert_eq!(s.px[40 * 64 + 5], BLUE);
    }
    // the last frame is kept for the switcher
    let (px, w, hh) = h.frame().unwrap();
    assert_eq!((w, hh, px[10 * 64 + 50]), (64, 48, WHITE));

    // suspend then kill: the app saved its state on Quit
    h.suspend();
    h.kill();
    assert!(matches!(h.status(), Status::Exited(_)));
    let saved = std::fs::read_to_string(Path::new(env!("CARGO_TARGET_TMPDIR")).join("apps-sdk-test-a/suspended")).unwrap();
    assert_eq!(saved, "3");
}

#[test]
fn a_crash_is_reported_and_the_last_frame_stays() {
    let mut h = spawn("apps-sdk-test-b");
    pump_until(&mut h, |_, frame| frame);
    h.touch(TouchKind::Down, 0, -1.0, 0.0, 0);
    pump_until(&mut h, |h, _| !h.running());
    assert_eq!(h.status(), Status::Exited(3));
    assert!(h.frame().is_some(), "the crash screen is the last frame");
}
