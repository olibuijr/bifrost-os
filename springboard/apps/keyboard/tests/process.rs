//! The keyboard as the shell sees it: the real `bifrost-keyboard` process, driven through the shell's `AppHost` over
//! the app socket (`ToApp::KeyboardConfig` and touches in, frames and `ToShell::KeyboardOut` back).
use bifrost_app::proto::{kbflags, Configure, KeyboardConfig, KeyboardOut, ToApp, ToShell, TouchKind};
use springboard::shell::apphost::{AppHost, KEYBOARD_APP_ID};
use std::path::Path;
use std::time::{Duration, Instant};

const W: u32 = 1200;
const SCALE: f32 = 1200.0 / 768.0;

fn spawn() -> AppHost {
    let data = Path::new(env!("CARGO_TARGET_TMPDIR")).join("keyboard-process");
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let cfg = Configure { w: W, h: (W as f32 * 0.7) as u32, scale: SCALE, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    AppHost::spawn(KEYBOARD_APP_ID, Path::new(env!("CARGO_BIN_EXE_bifrost-keyboard")), &data, &assets, cfg).unwrap()
}

/// Pumps for up to five seconds until `f` is satisfied by the keyboard's output so far.
fn pump_until(h: &mut AppHost, outs: &mut Vec<KeyboardOut>, mut f: impl FnMut(&[KeyboardOut]) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        for r in h.pump().requests {
            if let ToShell::KeyboardOut(o) = r {
                outs.push(o);
            }
        }
        if f(outs) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out; keyboard said {outs:?}");
}

fn height(outs: &[KeyboardOut]) -> Option<u32> {
    outs.iter().rev().find_map(|o| if let KeyboardOut::Height(h) = o { Some(*h) } else { None })
}

#[test]
fn the_keyboard_process_draws_reports_its_height_and_types() {
    // the app sandbox sets this for every app; the keyboard must still draw
    std::env::set_var("BIFROST_HARDWARE_KEYBOARD", "1");
    let mut h = spawn();
    let mut outs = Vec::new();

    // configured shown, plain: a frame and a height of a few hundred px
    h.send(&ToApp::KeyboardConfig(KeyboardConfig { flags: kbflags::SHOWN, ..Default::default() }));
    pump_until(&mut h, &mut outs, |o| height(o).is_some());
    assert!(h.frame().is_some(), "the keyboard drew a frame");
    let plain = height(&outs).unwrap();
    assert!((200..W / 2).contains(&plain), "plain height {plain}");
    let (_, fw, fh) = h.frame().unwrap();
    assert_eq!(fw as u32, W);
    assert!(fh as u32 >= plain, "the buffer holds the whole keyboard");

    // the shell's input language is applied, not echoed back
    h.send(&ToApp::KeyboardConfig(KeyboardConfig { flags: kbflags::SHOWN, languages: 3, language: 1, ..Default::default() }));
    std::thread::sleep(Duration::from_millis(150));
    pump_until(&mut h, &mut outs, |_| true);
    assert!(!outs.iter().any(|o| matches!(o, KeyboardOut::Language(_))), "{outs:?}");

    // the prediction bar makes it taller
    h.send(&ToApp::KeyboardConfig(KeyboardConfig { flags: kbflags::SHOWN | kbflags::PREDICTIVE, languages: 3, language: 1, ..Default::default() }));
    pump_until(&mut h, &mut outs, |o| height(o).is_some_and(|t| t > plain));
    h.send(&ToApp::KeyboardConfig(KeyboardConfig { flags: kbflags::SHOWN, languages: 3, language: 1, ..Default::default() }));
    pump_until(&mut h, &mut outs, |o| height(o) == Some(plain));

    // tapping along the top row of letters types a letter
    outs.clear();
    let y = (8.0 + 27.5) * SCALE;
    for (i, x) in (30..1170).step_by(40).enumerate() {
        let t = i as u32 * 100;
        h.touch(TouchKind::Down, 0, x as f32, y, t, bifrost_app::proto::TouchSource::Finger, bifrost_app::proto::ModifierFlags::EMPTY);
        h.touch(TouchKind::Up, 0, x as f32, y, t + 30, bifrost_app::proto::TouchSource::Finger, bifrost_app::proto::ModifierFlags::EMPTY);
    }
    pump_until(&mut h, &mut outs, |o| o.iter().any(|k| matches!(k, KeyboardOut::Text(t) if t.chars().all(char::is_alphabetic) && !t.is_empty())));
    h.kill();
}
