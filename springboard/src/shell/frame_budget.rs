//! Frame-time budgets for the common desktop scenes. Run them with `tools/bench.py` (every `bench_*` test runs there),
//! or directly in `springboard/`: `cargo test --profile fast --features desktop-wayland,testkit --lib bench_frame_ -- --ignored --nocapture`.
//!
//! Each scene prints `BENCH frame_<scene>_median` and `BENCH frame_<scene>_p95` in ms, and fails when its 95th-percentile
//! frame is over `FRAME_BUDGET_MS` (one 60 Hz frame). A frame over budget is a dropped frame on screen, so a scene that
//! fails here is a smoothness regression. The Files listing scene lives in `files/bench.rs` and shares these helpers.
use super::tests::{settle, shell_sized, unlock};
use super::*;
use crate::gfx::Surface;
use std::time::Instant;

/// One 60 Hz frame. The 95th-percentile frame of every common scene must fit in it.
pub const FRAME_BUDGET_MS: f64 = 16.7;

/// The measured display size (1080p, the size `bench_desktop_frame` uses).
const W: usize = 1920;
const H: usize = 1080;
/// Untimed warm-up frames and timed frames per scene.
const WARM: usize = 3;
const RUNS: usize = 60;

/// (median, 95th percentile) in ms of `runs` timed calls of `f`, after `warm` untimed calls. `f` gets the call index.
pub(crate) fn frame_ms(warm: usize, runs: usize, mut f: impl FnMut(usize)) -> (f64, f64) {
    for k in 0..warm {
        f(k);
    }
    let mut t: Vec<f64> = (0..runs)
        .map(|k| {
            let s = Instant::now();
            f(warm + k);
            s.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    t.sort_by(f64::total_cmp);
    (t[t.len() / 2], t[(t.len() * 95 / 100).min(t.len() - 1)])
}

/// Prints the scene's BENCH lines and fails the test when its 95th-percentile frame is over `FRAME_BUDGET_MS`.
pub(crate) fn report(scene: &str, (median, p95): (f64, f64)) {
    println!("BENCH frame_{scene}_median {median:.3} ms");
    println!("BENCH frame_{scene}_p95 {p95:.3} ms");
    assert!(p95 <= FRAME_BUDGET_MS, "frame budget {FRAME_BUDGET_MS} ms exceeded: {scene} p95 is {p95:.2} ms");
}

/// Repaints the whole shell at time `t` (the frame is forced dirty, as a real change would be).
fn paint(sh: &mut Shell, buf: &mut [u32], t: f64) {
    sh.dirty = true;
    sh.draw(&mut Surface { w: W, h: H, stride: W, px: &mut *buf, y0: 0 }, t);
}

/// An unlocked desktop at `W`x`H` with `open` launched and settled, as `bench_desktop_frame` sets it up.
fn desktop(open: Option<AppId>) -> Shell {
    let mut sh = shell_sized(W, H);
    sh.desk = Some(desk::Desk::new());
    unlock(&mut sh);
    if let Some(app) = open {
        sh.desk_open(app, 0.);
    }
    settle(&mut sh, 0.0);
    sh
}

/// Desktop with no windows open, repainted every frame.
#[test]
#[ignore]
fn bench_frame_desktop_idle() {
    let mut sh = desktop(None);
    let mut buf = vec![0u32; W * H];
    report("desktop_idle", frame_ms(WARM, RUNS, |k| paint(&mut sh, &mut buf, 1e9 + k as f64 * 16.0)));
}

/// Desktop with the Settings window open (its page is drawn in the window body every frame).
#[test]
#[ignore]
fn bench_frame_settings_open() {
    let mut sh = desktop(Some(AppId::Settings));
    let mut buf = vec![0u32; W * H];
    report("settings_open", frame_ms(WARM, RUNS, |k| paint(&mut sh, &mut buf, 1e9 + k as f64 * 16.0)));
}

/// The Window menu open over the desktop (its popup rows and shadow are drawn every frame).
#[cfg(feature = "desktop-wayland")]
#[test]
#[ignore]
fn bench_frame_menu_open() {
    let mut sh = desktop(Some(AppId::Terminal));
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desktop_menu = Some(desktop_bar::Menu::Window);
    let mut buf = vec![0u32; W * H];
    report("menu_open", frame_ms(WARM, RUNS, |k| paint(&mut sh, &mut buf, 1e9 + k as f64 * 16.0)));
}

/// The frame an external app's new pixels ask for on a settled desktop: the windows are republished and nothing else is
/// drawn (`Shell::draw`, the windows-only path). It must cost a fraction of `bench_frame_desktop_idle`; this is the frame
/// an idle app that repaints every vblank costs the shell.
#[cfg(feature = "desktop-wayland")]
#[test]
#[ignore]
fn bench_frame_ext_pixels() {
    let _lock = crate::test_support::global_lock();
    let saved = std::env::var_os("SPRINGBOARD_WAYLAND");
    std::env::set_var("SPRINGBOARD_WAYLAND", "1");
    let mut sh = desktop(Some(AppId::Settings));
    sh.screen = Screen::Home;
    sh.anim = None;
    let mut buf = vec![0u32; W * H];
    // one whole frame first: the windows-only frame follows a drawn desktop
    paint(&mut sh, &mut buf, 1e9);
    report(
        "ext_pixels",
        frame_ms(WARM, RUNS, |k| {
            sh.ext_frame = true;
            sh.draw(&mut Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 }, 1e9 + 16.0 + k as f64 * 16.0);
        }),
    );
    if let Some(v) = saved {
        std::env::set_var("SPRINGBOARD_WAYLAND", v)
    } else {
        std::env::remove_var("SPRINGBOARD_WAYLAND")
    };
}

/// A Settings window dragged by its title bar: each timed frame is one pointer move (the event) plus the repaint.
#[cfg(feature = "desktop-wayland")]
#[test]
#[ignore]
fn bench_frame_window_drag() {
    let mut sh = desktop(Some(AppId::Settings));
    let r = sh.desk.as_ref().unwrap().wins.last().unwrap().r;
    let (x0, y0) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + 12.0);
    let t0 = 2000.0;
    sh.event(Event::Down(x0, y0), t0);
    let mut buf = vec![0u32; W * H];
    report(
        "window_drag",
        frame_ms(WARM, RUNS, |k| {
            // a diagonal path that stays on screen for the whole run (the window clamps at the screen edges)
            let (dx, dy) = (4.0 * (k % 100) as f32, 2.0 * (k % 50) as f32);
            let t = t0 + k as f64 * 16.0;
            sh.event(Event::Move(x0 + dx, y0 + dy), t);
            paint(&mut sh, &mut buf, t);
        }),
    );
    sh.event(Event::Up(x0 + 4.0, y0 + 2.0), t0 + 1000.0);
}
