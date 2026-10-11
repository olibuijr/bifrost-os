//! End to end: the shell's `Keyboard` client drives the real keyboard process (show, frame, touch, typed output).
use springboard::gfx::Surface;
use springboard::shell::keyboard::{self, Keyboard, Out, BG};
use std::time::{Duration, Instant};

const W: usize = 1200;
const SCALE: f32 = 1200.0 / 768.0;

fn pump_until(kb: &mut Keyboard, outs: &mut Vec<Out>, mut done: impl FnMut(&Keyboard, &[Out]) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        keyboard::pump();
        outs.extend(kb.tick(10_000.0));
        if done(kb, outs) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out; typed {outs:?}");
}

#[test]
fn the_shell_client_shows_draws_and_types_through_the_keyboard_app() {
    // the shell root doubles as the apps' asset directory: link the repo's assets into a scratch root
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("keyboard-client-root");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for e in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")).unwrap().flatten() {
        let _ = std::os::unix::fs::symlink(e.path().canonicalize().unwrap(), root.join(e.file_name()));
    }
    springboard::shell::apphost::register(&root);
    keyboard::use_process_for_test("is.olibuijr.keyboard", env!("CARGO_BIN_EXE_bifrost-keyboard").into());
    let mut kb = Keyboard::new_hidden(W as f32, SCALE);
    assert!(!kb.shown());
    let _shown_but_never_raised = Keyboard::new(W as f32, SCALE, true);
    assert!(keyboard::poll_fd().is_none(), "creating keyboards starts no process");
    kb.set_shown(true, 0.0);
    let mut outs = Vec::new();

    // the process reports a real height and its frame arrives
    let estimate = kb.h();
    pump_until(&mut kb, &mut outs, |kb, _| kb.animating() && kb.h() > 0.0);
    let h = kb.h();
    assert!(h > 100.0 && h < W as f32 / 2.0, "height {h} (estimate was {estimate})");

    // drawn at the bottom of a screen: the keyboard's pixels replace the background
    let screen_h = 1920usize;
    let mut px = vec![0u32; W * screen_h];
    let mut s = Surface { w: W, h: screen_h, stride: W, px: &mut px, y0: 0 };
    let ui = springboard::shell::ui::Ui::new(springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap(), SCALE);
    let top = screen_h as f32 - h;
    kb.draw(&mut s, &ui, top, 10_000.0);
    let drawn = &px[(top as usize + 20) * W..(top as usize + 40) * W];
    assert!(drawn.iter().any(|&p| p != 0), "the keyboard drew into the screen");
    assert!(drawn.iter().any(|&p| p != BG), "keys, not only the background");

    // a tap on the letter rows types; it arrives from `tick`
    let y = (8.0 + 27.5) * SCALE;
    for (i, x) in (30..1170).step_by(40).enumerate() {
        kb.down(x as f32, y, i as f64 * 100.0);
        kb.up(x as f32, y, i as f64 * 100.0 + 30.0);
    }
    pump_until(&mut kb, &mut outs, |_, o| o.iter().any(|k| matches!(k, Out::Text(t) if t.chars().all(char::is_alphabetic))));

    // hiding slides it away and stops it taking touches
    kb.set_shown(false, 20_000.0);
    assert!(!kb.shown());
    assert!(kb.offset(20_300.0) >= h, "fully off screen after the slide");
}
