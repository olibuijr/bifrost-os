//! Records the resampler's THD+N: a 1 kHz sine at -6 dBFS through each quality and a set of rate pairs.
//! Run on the host (lead only): `cargo run --release --example thdn -p bifrost-audio-toolbox`. Prints one line
//! per case for docs/audio/13-toolbox.md.
use bifrost_audio_toolbox::resample::{resample, thd_n_db, Quality};

fn main() {
    for (from, to) in [(44100u32, 48000u32), (48000, 44100), (44100, 96000), (96000, 44100), (48000, 192000)] {
        for q in [Quality::Fast, Quality::Standard, Quality::High] {
            let n = from as usize / 2;
            let input: Vec<f32> =
                (0..n).map(|i| 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / from as f64).sin() as f32).collect();
            let out = resample(&input, from, to, 1, q).unwrap();
            // a window of whole 10 ms units (ten cycles of 1 kHz each) away from both edges
            let unit = to as usize / 100;
            let units = (out.len() / unit).saturating_sub(4).min(50);
            let w = &out[unit * 2..unit * (2 + units)];
            println!("{from} -> {to} {q:?}: THD+N {:.1} dB", thd_n_db(w, to as f64, 1000.0));
        }
    }
}
