//! Midget verification: a player, an EQ, a mixer and the output node running on the default output device at 128
//! frames, with a tap on the output. Run on Midget (needs `bifrost-audiod`):
//!
//! `cargo run --profile fast -p bifrost-audio-engine --example midget_engine -- 128 30`
//!
//! Prints the frames the tap saw and the dropped count every second; xruns are read from the server's profiler
//! (`bifrost pc log` category `io`). The pass criterion is 0 xruns over the run.

use bifrost_audio_engine::{Engine, EngineConfig, OUTPUT};
use bifrost_audio_hal::{Role, System};
use bifrost_audio_units::builtin::{Biquad, BQ_FREQ, BQ_LOW_PASS, BQ_TYPE};

fn main() {
    let mut args = std::env::args().skip(1);
    let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(128);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);

    let system = System::connect("is.olibuijr.audio.example").expect("connect to bifrost-audiod");
    let device = system.default_device(Role::Output).expect("default output device");

    let mut engine = Engine::new(EngineConfig { sample_rate: 48_000.0, channels: 2, max_frames: frames.max(128) }).unwrap();
    // A two-second 220 Hz / 330 Hz stereo tone, looped.
    let tone: Vec<f32> = (0..96_000)
        .flat_map(|i| {
            let t = i as f32 / 48_000.0;
            [0.2 * (t * 220.0 * std::f32::consts::TAU).sin(), 0.2 * (t * 330.0 * std::f32::consts::TAU).sin()]
        })
        .collect();
    let (player, transport) = engine.attach_player(48_000.0, 2, &tone).unwrap();
    transport.set_looping(true);
    transport.play();
    let (eq, params, _) = engine.attach_unit(Box::new(Biquad::new())).unwrap();
    params.set(BQ_TYPE, BQ_LOW_PASS).unwrap();
    params.set(BQ_FREQ, 4_000.0).unwrap();
    let mixer = engine.attach_mixer().unwrap();
    engine.connect(player, eq).unwrap();
    engine.connect(eq, mixer).unwrap();
    engine.connect(mixer, OUTPUT).unwrap();
    let tap = engine.attach_tap(OUTPUT, 48_000).unwrap();

    engine.start_on_device(&device, frames).expect("start on device");
    let mut sink = vec![Vec::new(), Vec::new()];
    for s in 1..=seconds {
        std::thread::sleep(std::time::Duration::from_secs(1));
        sink.iter_mut().for_each(|v| v.clear());
        let n = tap.read(&mut sink, 48_000);
        println!("t={s}s tap frames={n} dropped={} position={}", tap.dropped(), transport.position());
    }
    engine.stop().unwrap();
}
