//! `bifrost-audio-bench`: see docs/audio/19-bench-instrument.md. Prints one JSON object with `--json`.

use bifrost_audio_bench::args::{self, Args, Cmd, USAGE};
use bifrost_audio_bench::json::Report;
use bifrost_audio_bench::rig::{Rig, RigOptions};
use bifrost_audio_bench::{live, midi_latency, roundtrip, sweep, throughput};

fn rig_options(a: &Args, input: bool) -> RigOptions {
    // The null driver is always on its manual clock; loopback and alsa run in real time.
    RigOptions { driver: a.driver.clone(), rate: a.rate, quantum: a.quantum, device: a.device.clone(), input, manual: false }
}

/// `--live`: the same benches against the running bifrost-audiod (see `live`).
fn run_live(a: &Args) -> Result<Report, String> {
    let dev = live::LiveDevice::connect(a.device.as_deref(), a.rate)?;
    match a.cmd {
        Cmd::MidiLatency => live::midi_latency(&dev, &midi_latency::Opts { count: a.count, interval_ms: a.interval_ms, lead: a.lead }, a.quantum),
        Cmd::AudioRoundtrip => live::roundtrip(&dev, &roundtrip::Opts { count: a.count, interval_ms: a.interval_ms, threshold: 0.25 }, a.quantum),
        Cmd::XrunSweep => live::sweep(&dev, &sweep::Opts { quanta: sweep::QUANTA.to_vec(), seconds: a.seconds, stress: a.stress }),
        Cmd::MidiThroughput => Err("midi-throughput has no --live mode".to_string()),
    }
}

fn run(a: &Args) -> Result<Report, String> {
    if a.live {
        return run_live(a);
    }
    match a.cmd {
        Cmd::MidiLatency => {
            let mut rig = Rig::open(&rig_options(a, false))?;
            let o = midi_latency::Opts { count: a.count, interval_ms: a.interval_ms, lead: a.lead };
            let out = midi_latency::run(&mut rig, &o)?;
            Ok(midi_latency::report(&rig, &o, &out))
        }
        Cmd::AudioRoundtrip => {
            let mut rig = Rig::open(&rig_options(a, true))?;
            let o = roundtrip::Opts { count: a.count, interval_ms: a.interval_ms, threshold: 0.25 };
            let out = roundtrip::run(&mut rig, &o)?;
            Ok(roundtrip::report(&rig, &o, &out))
        }
        Cmd::XrunSweep => {
            let base = rig_options(a, false);
            let o = sweep::Opts { quanta: sweep::QUANTA.to_vec(), seconds: a.seconds, stress: a.stress };
            let runs = sweep::run(&base, &o)?;
            Ok(sweep::report(&base, &o, &runs))
        }
        Cmd::MidiThroughput => {
            let out = throughput::run(a.rate, a.quantum, a.count)?;
            Ok(throughput::report(&a.driver.label(), a.rate, a.quantum, &out))
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }
    let parsed = match args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bifrost-audio-bench: {e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    // a run that fails is not a usage error: say why, without the usage text
    match run(&parsed).map(|r| (parsed, r)) {
        Ok((a, report)) => {
            if a.json {
                println!("{}", report.to_json().to_string());
            } else {
                println!("{}", report.to_text());
            }
        }
        Err(e) => {
            eprintln!("bifrost-audio-bench: {e}");
            std::process::exit(if e == live::NO_LOOPBACK { live::EXIT_NO_LOOPBACK } else { 1 });
        }
    }
}
