//! The capture readback on the headless backend: a solid scene must come back as RGBA with red still red, and the
//! requested region only. Guards the red/blue swap of the first screenshots.

use bifrost_wm::{headless::Compositor, session_protocols::CaptureSpec, wm::Rect};

#[test]
fn a_red_scene_reads_back_red_in_rgba_order() {
    let (w, h) = (64, 48);
    let wm = Compositor::start("wayland-bfwm-capture-test", w, h).expect("headless compositor");
    let spec = CaptureSpec { output: "HEADLESS-1".into(), region: Rect::new(8, 4, 16, 8), cursor: false };
    let px = wm
        .run(move |state, backend| backend.capture(state, &spec, [1.0, 0.0, 0.0, 1.0]))
        .expect("capture");
    assert_eq!(px.len(), 16 * 8 * 4, "only the requested region comes back");
    for p in px.chunks_exact(4) {
        assert_eq!(p, [255, 0, 0, 255], "R, G, B, A in memory: red stays red");
    }
    let other = CaptureSpec { output: "NOPE".into(), region: Rect::new(0, 0, 4, 4), cursor: false };
    assert!(wm.run(move |state, backend| backend.capture(state, &other, [0.0; 4])).is_none());
}
