//! Capture to playback at 64 frames. Usage: `passthrough [seconds] [frames]` (default 600 s, 64 frames).
//! Prints the xruns and overload notices every 10 s and at the end; the Midget acceptance is 0 over 10 minutes.

use bifrost_audio_hal::{IoProcConfig, Role, System};
use std::time::{Duration, Instant};

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(600);
    let frames: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(64);
    let sys = System::connect("passthrough").expect("connect to bifrost-audiod");
    let dev = sys.default_device(Role::Output).expect("default output device");
    let id = dev
        .add_io_proc(
            IoProcConfig { input_channels: 2, output_channels: 2, buffer_frames: frames },
            Box::new(|_ctx, _now, input, _it, output, _ot| {
                for (o, i) in output.buffers.iter_mut().zip(input.buffers.iter()) {
                    o.data.copy_from_slice(&i.data);
                }
            }),
        )
        .expect("add io proc");
    dev.start(id).expect("start");
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(10));
        println!("{:>5} s  xruns {}  overloads {}", t0.elapsed().as_secs(), dev.io_proc_xruns(id).unwrap_or(0), sys.overloads(id));
    }
    let (x, o) = (dev.io_proc_xruns(id).unwrap_or(0), sys.overloads(id));
    dev.stop(id).expect("stop");
    dev.remove_io_proc(id).expect("remove");
    println!("done: xruns {x}, overloads {o}");
    std::process::exit(if x == 0 && o == 0 { 0 } else { 1 });
}
