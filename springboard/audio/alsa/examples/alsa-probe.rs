//! Midget check: plays 10 s of silence on the first analog device (card 0, device 0) at a 64-frame quantum, driven
//! by the DLL and a timerfd, and prints xruns, the DLL rate and the jack states.
//!
//! `cargo run --example alsa-probe [-- UID [SECONDS]]`; UID defaults to the first device with output.
//! Pass: `xruns 0` and `dll rate` within 50 ppm of 48000 Hz; the jack lines match `amixer -c0 contents`.

use bifrost_audio_alsa::timer::{wake_time, CycleTimer};
use bifrost_audio_alsa::{AlsaDriver, Ctl};
use bifrost_audio_driver::{Driver, IoConfig};
use bifrost_audio_types::{Buffer, BufferList};

fn main() {
    let mut args = std::env::args().skip(1);
    let uid_arg = args.next();
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let quantum = 64u32;
    let rate = 48_000.0;

    let mut driver = AlsaDriver::new();
    let mut devices = Vec::new();
    driver.enumerate(&mut devices);
    for d in &devices {
        println!("device {} \"{}\" [{}] in {} out {} rates {:?} buffer {:?}", d.uid, d.name, d.transport, d.input_channels, d.output_channels, d.rates, d.buffer_frames_range);
    }
    let Some(dev) = devices.iter().find(|d| d.output_channels > 0 && uid_arg.as_deref().is_none_or(|u| u == d.uid)) else {
        eprintln!("no output device");
        std::process::exit(1);
    };

    if let Ok(card_ctl) = Ctl::open(0) {
        for j in card_ctl.jacks().unwrap_or_default() {
            println!("jack {:<28} {}", j.name, if j.connected { "on" } else { "off" });
        }
        for d in card_ctl.pcm_devices().unwrap_or_default() {
            if let Some(n) = card_ctl.monitor_name(d) {
                println!("eld pcm {d}: {n}");
            }
        }
    }

    let mut io = driver.open(&dev.uid, &IoConfig { rate, quantum, input: false, output: true }).expect("open");
    let (_, out_fmt) = io.physical_formats();
    println!("opened {} output {:?}", dev.uid, out_fmt);
    let channels = out_fmt.map_or(2, |f| f.channels) as usize;
    let silence = BufferList { buffers: (0..channels).map(|_| Buffer { channels: 1, data: vec![0.0; quantum as usize] }).collect() };

    let zero = io.start().expect("start");
    let timer = CycleTimer::new().expect("timerfd");
    let cycles = seconds * rate as u64 / quantum as u64;
    let (mut xruns, mut late) = (0u64, 0u64);
    let prefill_frames = io.latency().output as u64;
    for n in 1..=cycles {
        // wake a quarter quantum before the hardware needs this cycle's data
        let frame = n * quantum as u64;
        let due = io.host_time_of(frame).unwrap_or(zero.host_ns + (frame as f64 * 1e9 / rate) as u64);
        let safety = (quantum as u64 * 1_000_000_000 / rate as u64) / 4;
        timer.arm_raw(wake_time(due.saturating_sub((prefill_frames * 1_000_000_000) / rate as u64), safety)).ok();
        timer.wait().ok();
        let pos = io.position();
        if !pos.running {
            late += 1;
        }
        if io.write_mix(quantum, &silence).is_err() {
            xruns += 1;
        }
    }
    println!("cycles {cycles} xruns {xruns} not-running {late}");
    println!("dll rate scalar {:.9} = {:.3} Hz ({:+.1} ppm)", io.rate_scalar(), rate * io.rate_scalar(), (io.rate_scalar() - 1.0) * 1e6);
    io.stop();
    if xruns > 0 {
        std::process::exit(2);
    }
}
