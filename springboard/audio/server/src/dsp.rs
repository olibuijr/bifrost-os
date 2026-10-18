//! The processing stages a device runs: the audio-thread side of the DSP plan (`bifrost_audio_policy::dsp`).
//!
//! The policy crate decides which stages run for a device and in what order (`DspManager::assignment`). Here that
//! decision becomes [`Stages`], a chain of processors with every buffer and filter state allocated up front (control
//! thread), so [`Stages::process`] on the IO thread never allocates, locks or makes a syscall. The chain is handed to the
//! IO thread through [`DspMailbox`], the same way the client list is: the control thread posts, the IO thread picks the
//! new chain up with `try_lock` when the generation changed, and the chain it replaced goes back to the mailbox so the
//! control thread frees it.
//!
//! Stage parameters (all optional, defaults in brackets):
//! - `gain`: `db` [0]
//! - `eq`: a peaking filter, `freq` [1000] Hz, `q` [0.707], `gain_db` [0]
//! - `compressor`: `threshold_db` [-18], `ratio` [4], `attack_ms` [10], `release_ms` [100], `makeup_db` [0]
//! - `noise-gate`: `threshold_db` [-60], `release_ms` [100]
//! - `delay`: `frames` [0], at most `MAX_DELAY_FRAMES`
//! - `limiter`: `ceiling_db` [-0.3], `release_ms` [50], `lookahead` [0] frames, at most `MAX_LOOKAHEAD`
//!
//! A stage that would change nothing (0 dB gain, a flat filter, no delay) is left out of the chain. Detectors (compressor,
//! gate, limiter) are linked across channels, so the stereo image does not move.

use bifrost_audio_policy::dsp::{Assignment, Kind, Stage};
use bifrost_audio_types::Buffer;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// The longest delay a `delay` stage accepts, in frames (one second at 48 kHz).
pub const MAX_DELAY_FRAMES: usize = 48_000;
/// The longest limiter lookahead, in frames (10 ms at 48 kHz).
pub const MAX_LOOKAHEAD: usize = 480;

fn param(stage: &Stage, name: &str, default: f32) -> f32 {
    stage.params.get(name).copied().unwrap_or(default)
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The one-pole coefficient that reaches 63% of a step in `ms` milliseconds (1 when `ms` is not positive).
fn smooth(ms: f32, rate: f32) -> f32 {
    if ms <= 0.0 || rate <= 0.0 {
        1.0
    } else {
        1.0 - (-1.0 / (ms * 0.001 * rate)).exp()
    }
}

fn peak_of(bufs: &[Buffer], nch: usize, f: usize) -> f32 {
    let mut level = 0.0f32;
    for b in bufs.iter().take(nch) {
        level = level.max(b.data[f].abs());
    }
    level
}

enum Proc {
    Gain(f32),
    Eq(Peaking),
    Compressor(Compressor),
    Gate(Gate),
    Delay(Delay),
    Limiter(Limiter),
}

struct Peaking {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: Vec<f64>,
    z2: Vec<f64>,
}

impl Peaking {
    /// The peaking filter of the audio EQ cookbook, normalised, with one state pair per channel.
    fn new(rate: f64, freq: f64, q: f64, gain_db: f64, channels: usize) -> Peaking {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Peaking {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * w0.cos() / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha / a) / a0,
            z1: vec![0.0; channels],
            z2: vec![0.0; channels],
        }
    }

    fn process(&mut self, bufs: &mut [Buffer], nch: usize, frames: usize) {
        for (c, b) in bufs.iter_mut().take(nch).enumerate() {
            let (mut z1, mut z2) = (self.z1[c], self.z2[c]);
            for x in b.data[..frames].iter_mut() {
                let input = *x as f64;
                let out = self.b0 * input + z1;
                z1 = self.b1 * input - self.a1 * out + z2;
                z2 = self.b2 * input - self.a2 * out;
                *x = out as f32;
            }
            self.z1[c] = z1;
            self.z2[c] = z2;
        }
    }
}

struct Compressor {
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup_db: f32,
    env: f32,
}

impl Compressor {
    fn process(&mut self, bufs: &mut [Buffer], nch: usize, frames: usize) {
        for f in 0..frames {
            let level = peak_of(bufs, nch, f);
            let k = if level > self.env { self.attack } else { self.release };
            self.env += (level - self.env) * k;
            let env_db = 20.0 * self.env.max(1e-9).log10();
            let over = env_db - self.threshold_db;
            let reduction = if over > 0.0 { over * (1.0 - 1.0 / self.ratio) } else { 0.0 };
            let g = db_to_lin(self.makeup_db - reduction);
            for b in bufs.iter_mut().take(nch) {
                b.data[f] *= g;
            }
        }
    }
}

struct Gate {
    threshold: f32,
    attack: f32,
    release: f32,
    smooth: f32,
    env: f32,
    gain: f32,
}

impl Gate {
    fn process(&mut self, bufs: &mut [Buffer], nch: usize, frames: usize) {
        for f in 0..frames {
            let level = peak_of(bufs, nch, f);
            let k = if level > self.env { self.attack } else { self.release };
            self.env += (level - self.env) * k;
            let target = if self.env >= self.threshold { 1.0 } else { 0.0 };
            self.gain += (target - self.gain) * self.smooth;
            for b in bufs.iter_mut().take(nch) {
                b.data[f] *= self.gain;
            }
        }
    }
}

struct Delay {
    line: Vec<f32>,
    len: usize,
    pos: usize,
}

impl Delay {
    fn process(&mut self, bufs: &mut [Buffer], nch: usize, frames: usize) {
        for f in 0..frames {
            for (c, b) in bufs.iter_mut().take(nch).enumerate() {
                let slot = &mut self.line[c * self.len + self.pos];
                std::mem::swap(slot, &mut b.data[f]);
            }
            self.pos += 1;
            if self.pos == self.len {
                self.pos = 0;
            }
        }
    }
}

struct Limiter {
    ceiling: f32,
    release: f32,
    gain: f32,
    /// `lookahead` frames per channel; empty without lookahead.
    line: Vec<f32>,
    look: usize,
    pos: usize,
}

impl Limiter {
    fn process(&mut self, bufs: &mut [Buffer], nch: usize, frames: usize) {
        for f in 0..frames {
            // the gain follows the incoming signal; the (possibly delayed) signal it is applied to leaves a little later
            let level = peak_of(bufs, nch, f);
            let target = if level > self.ceiling { self.ceiling / level } else { 1.0 };
            if target < self.gain {
                self.gain = target;
            } else {
                self.gain += (target - self.gain) * self.release;
            }
            for (c, b) in bufs.iter_mut().take(nch).enumerate() {
                if self.look > 0 {
                    std::mem::swap(&mut self.line[c * self.look + self.pos], &mut b.data[f]);
                }
                b.data[f] = (b.data[f] * self.gain).clamp(-self.ceiling, self.ceiling);
            }
            if self.look > 0 {
                self.pos += 1;
                if self.pos == self.look {
                    self.pos = 0;
                }
            }
        }
    }
}

/// A compiled chain of stages for one direction of one device.
#[derive(Default)]
pub struct Stages {
    procs: Vec<Proc>,
    channels: usize,
    latency: u32,
}

impl Stages {
    /// A chain that does nothing (no allocation).
    pub fn empty() -> Stages {
        Stages::default()
    }

    /// Compiles the stages of `plan` for `channels` planes at `rate` hertz. Runs on the control thread.
    pub fn compile(plan: &Assignment, rate: f32, channels: usize) -> Stages {
        let mut procs = Vec::new();
        let mut latency = 0u32;
        for planned in &plan.stages {
            let s = &planned.stage;
            let p = match s.kind {
                Kind::Gain => {
                    let db = param(s, "db", 0.0);
                    if db == 0.0 || !db.is_finite() {
                        continue;
                    }
                    Proc::Gain(db_to_lin(db))
                }
                Kind::Eq => {
                    let gain_db = param(s, "gain_db", 0.0);
                    let freq = param(s, "freq", 1000.0);
                    let q = param(s, "q", 0.707);
                    if gain_db == 0.0 || !(gain_db.is_finite() && freq > 0.0 && freq < rate * 0.5 && q > 0.0) {
                        continue;
                    }
                    Proc::Eq(Peaking::new(rate as f64, freq as f64, q as f64, gain_db as f64, channels))
                }
                Kind::Compressor => Proc::Compressor(Compressor {
                    threshold_db: param(s, "threshold_db", -18.0),
                    ratio: param(s, "ratio", 4.0).max(1.0),
                    attack: smooth(param(s, "attack_ms", 10.0), rate),
                    release: smooth(param(s, "release_ms", 100.0), rate),
                    makeup_db: param(s, "makeup_db", 0.0),
                    env: 0.0,
                }),
                Kind::NoiseGate => Proc::Gate(Gate {
                    threshold: db_to_lin(param(s, "threshold_db", -60.0)),
                    attack: smooth(1.0, rate),
                    release: smooth(param(s, "release_ms", 100.0), rate),
                    smooth: smooth(5.0, rate),
                    env: 0.0,
                    gain: 1.0,
                }),
                Kind::Delay => {
                    let frames = (param(s, "frames", 0.0).max(0.0) as usize).min(MAX_DELAY_FRAMES);
                    if frames == 0 {
                        continue;
                    }
                    latency += frames as u32;
                    Proc::Delay(Delay { line: vec![0.0; frames * channels], len: frames, pos: 0 })
                }
                Kind::Limiter => {
                    let look = (param(s, "lookahead", 0.0).max(0.0) as usize).min(MAX_LOOKAHEAD);
                    latency += look as u32;
                    Proc::Limiter(Limiter {
                        ceiling: db_to_lin(param(s, "ceiling_db", -0.3)).clamp(1e-4, 1.0),
                        release: smooth(param(s, "release_ms", 50.0), rate),
                        gain: 1.0,
                        line: vec![0.0; look * channels],
                        look,
                        pos: 0,
                    })
                }
            };
            procs.push(p);
        }
        Stages { procs, channels, latency }
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.procs.len()
    }

    /// Frames of latency the compiled stages add.
    pub fn latency_frames(&self) -> u32 {
        self.latency
    }

    /// Runs the chain over the first `frames` frames of `bufs`, in place. Audio thread: no allocation, no locks.
    pub fn process(&mut self, bufs: &mut [Buffer], frames: usize) {
        let nch = bufs.len().min(self.channels);
        if nch == 0 {
            return;
        }
        for p in self.procs.iter_mut() {
            match p {
                Proc::Gain(g) => {
                    for b in bufs.iter_mut().take(nch) {
                        for x in b.data[..frames].iter_mut() {
                            *x *= *g;
                        }
                    }
                }
                Proc::Eq(e) => e.process(bufs, nch, frames),
                Proc::Compressor(c) => c.process(bufs, nch, frames),
                Proc::Gate(g) => g.process(bufs, nch, frames),
                Proc::Delay(d) => d.process(bufs, nch, frames),
                Proc::Limiter(l) => l.process(bufs, nch, frames),
            }
        }
    }
}

/// What a device runs: the chain for what it plays and the one for what it captures.
#[derive(Default)]
pub struct Plan {
    pub output: Stages,
    pub input: Stages,
}

#[derive(Default)]
struct Slot {
    pending: Option<Plan>,
    /// The plan the IO thread replaced, waiting for the control thread to free it.
    retired: Option<Plan>,
}

/// Hands plans from the control thread to a device's IO thread. The IO thread never blocks on it and never frees what
/// it replaced.
#[derive(Default)]
pub struct DspMailbox {
    generation: AtomicU32,
    slot: Mutex<Slot>,
}

impl DspMailbox {
    pub fn new() -> DspMailbox {
        DspMailbox::default()
    }

    /// Control thread: offers a plan. A plan the IO thread has not picked up yet is replaced, and the one it retired is
    /// freed here.
    pub fn post(&self, plan: Plan) {
        let mut s = self.slot.lock().unwrap();
        s.retired = None;
        s.pending = Some(plan);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// IO thread: swaps `current` for the posted plan. Returns the generation now in force when it did; `None` when
    /// nothing is new, the control thread holds the lock, or the previous retired plan has not been freed yet (it is
    /// tried again next cycle).
    pub fn take_into(&self, seen: u32, current: &mut Plan) -> Option<u32> {
        let g = self.generation.load(Ordering::Acquire);
        if g == seen {
            return None;
        }
        let mut s = self.slot.try_lock().ok()?;
        if s.retired.is_some() {
            return None;
        }
        let new = s.pending.take()?;
        s.retired = Some(std::mem::replace(current, new));
        Some(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_policy::dsp::{Chain, DspManager, Stage};
    use bifrost_audio_policy::sessions::Direction;

    const RATE: f32 = 48_000.0;

    fn planes(n: usize, frames: usize, v: f32) -> Vec<Buffer> {
        (0..n).map(|_| Buffer { channels: 1, data: vec![v; frames] }).collect()
    }

    fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn compile(stages: Vec<Stage>, channels: usize) -> Stages {
        let mut m = DspManager::new();
        m.set_device_chain("dev", Direction::Output, Chain::new(stages)).unwrap();
        Stages::compile(&m.assignment("dev", Direction::Output, None, None), RATE, channels)
    }

    #[test]
    fn a_gain_stage_scales_every_channel_and_zero_db_is_left_out() {
        let mut s = compile(vec![Stage::new("g", Kind::Gain).with("db", -6.0206)], 2);
        let mut b = planes(2, 64, 1.0);
        s.process(&mut b, 64);
        for p in &b {
            assert!(p.data.iter().all(|x| (x - 0.5).abs() < 1e-3));
        }
        assert!(compile(vec![Stage::new("g", Kind::Gain)], 2).is_empty());
        assert!(compile(vec![Stage::new("e", Kind::Eq)], 2).is_empty(), "a flat filter changes nothing");
        assert!(compile(vec![Stage::new("d", Kind::Delay)], 2).is_empty(), "a zero delay changes nothing");
    }

    #[test]
    fn a_delay_keeps_its_state_across_blocks() {
        let mut s = compile(vec![Stage::new("d", Kind::Delay).with("frames", 3.0)], 1);
        assert_eq!(s.latency_frames(), 3);
        let mut a = vec![Buffer { channels: 1, data: vec![1.0, 2.0, 3.0, 4.0] }];
        s.process(&mut a, 4);
        assert_eq!(a[0].data, vec![0.0, 0.0, 0.0, 1.0]);
        let mut b = vec![Buffer { channels: 1, data: vec![5.0, 6.0, 0.0, 0.0] }];
        s.process(&mut b, 2);
        assert_eq!(&b[0].data[..2], &[2.0, 3.0]);
    }

    #[test]
    fn the_limiter_never_passes_its_ceiling_with_or_without_lookahead() {
        for look in [0.0, 24.0] {
            let mut s = compile(vec![Stage::new("l", Kind::Limiter).with("ceiling_db", -6.0206).with("lookahead", look)], 2);
            assert_eq!(s.latency_frames(), look as u32);
            let loud = sine(440.0, 3.0, 4800);
            let mut b = vec![Buffer { channels: 1, data: loud.clone() }, Buffer { channels: 1, data: loud }];
            s.process(&mut b, 4800);
            let peak = b.iter().flat_map(|p| p.data.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak <= 0.5 + 1e-4, "lookahead {look}: peak {peak}");
            assert!(peak > 0.3, "the limiter lowers the signal, it does not mute it: {peak}");
        }
    }

    #[test]
    fn a_quiet_signal_passes_the_limiter_untouched() {
        let mut s = compile(vec![Stage::new("l", Kind::Limiter)], 1);
        let quiet = sine(440.0, 0.25, 480);
        let mut b = vec![Buffer { channels: 1, data: quiet.clone() }];
        s.process(&mut b, 480);
        assert_eq!(b[0].data, quiet);
    }

    #[test]
    fn the_gate_closes_on_a_quiet_signal_and_opens_on_a_loud_one() {
        let mut s = compile(vec![Stage::new("n", Kind::NoiseGate).with("threshold_db", -40.0)], 1);
        let mut quiet = vec![Buffer { channels: 1, data: sine(300.0, 0.003, 9600) }];
        s.process(&mut quiet, 9600);
        assert!(rms(&quiet[0].data[8000..]) < 0.0002, "the hiss is gated: {}", rms(&quiet[0].data[8000..]));
        let mut loud = vec![Buffer { channels: 1, data: sine(300.0, 0.5, 9600) }];
        s.process(&mut loud, 9600);
        assert!((rms(&loud[0].data[8000..]) - 0.5 / 2f32.sqrt()).abs() < 0.02, "the signal passes: {}", rms(&loud[0].data[8000..]));
    }

    #[test]
    fn the_compressor_lowers_a_loud_steady_signal_and_leaves_a_quiet_one() {
        let stage = Stage::new("c", Kind::Compressor).with("threshold_db", -20.0).with("ratio", 4.0).with("attack_ms", 1.0);
        let mut s = compile(vec![stage.clone()], 1);
        let mut loud = vec![Buffer { channels: 1, data: vec![0.8; 9600] }];
        s.process(&mut loud, 9600);
        let out = loud[0].data[9599];
        // 0.8 is -1.9 dB: 18.1 dB over the threshold, reduced by 3/4 of that
        assert!((out - 0.8 * db_to_lin(-18.1 * 0.75)).abs() < 0.02, "steady state {out}");
        let mut s = compile(vec![stage], 1);
        let mut quiet = vec![Buffer { channels: 1, data: vec![0.05; 9600] }];
        s.process(&mut quiet, 9600);
        assert!((quiet[0].data[9599] - 0.05).abs() < 1e-4);
    }

    #[test]
    fn a_peaking_filter_boosts_its_own_frequency_only() {
        let mut s = compile(vec![Stage::new("e", Kind::Eq).with("freq", 1000.0).with("gain_db", 12.0).with("q", 1.0)], 1);
        let mut at = vec![Buffer { channels: 1, data: sine(1000.0, 0.1, 9600) }];
        s.process(&mut at, 9600);
        let gain = rms(&at[0].data[4800..]) / rms(&sine(1000.0, 0.1, 4800));
        assert!((gain - 3.98).abs() < 0.15, "12 dB at the centre: {gain}");
        let mut s = compile(vec![Stage::new("e", Kind::Eq).with("freq", 1000.0).with("gain_db", 12.0).with("q", 1.0)], 1);
        let mut far = vec![Buffer { channels: 1, data: sine(100.0, 0.1, 9600) }];
        s.process(&mut far, 9600);
        let gain = rms(&far[0].data[4800..]) / rms(&sine(100.0, 0.1, 4800));
        assert!((gain - 1.0).abs() < 0.1, "a decade away it is nearly flat: {gain}");
    }

    #[test]
    fn the_plan_order_and_latency_come_from_the_policy_and_the_limiter_runs_last() {
        let mut m = DspManager::new();
        let chain = Chain::new(vec![
            Stage::new("g", Kind::Gain).with("db", -6.0),
            Stage::new("d", Kind::Delay).with("frames", 10.0),
            Stage::new("l", Kind::Limiter).with("lookahead", 5.0),
        ]);
        m.set_device_chain("dev", Direction::Output, chain).unwrap();
        let plan = m.assignment("dev", Direction::Output, None, None);
        let s = Stages::compile(&plan, RATE, 2);
        assert_eq!((s.len(), s.latency_frames()), (3, plan.latency_frames));
        assert_eq!(s.latency_frames(), 15);
    }

    #[test]
    fn a_chain_ignores_planes_beyond_its_channels_and_empty_input() {
        let mut s = compile(vec![Stage::new("g", Kind::Gain).with("db", -6.0206)], 1);
        let mut b = planes(2, 8, 1.0);
        s.process(&mut b, 8);
        assert!((b[0].data[0] - 0.5).abs() < 1e-3 && b[1].data[0] == 1.0);
        let mut none: Vec<Buffer> = Vec::new();
        s.process(&mut none, 8);
        let mut e = Stages::empty();
        e.process(&mut b, 8);
    }

    #[test]
    fn the_mailbox_hands_a_plan_over_once_and_returns_the_old_one_to_the_control_thread() {
        let mb = DspMailbox::new();
        let mut current = Plan::default();
        assert_eq!(mb.take_into(0, &mut current), None, "nothing posted");
        mb.post(Plan { output: compile(vec![Stage::new("g", Kind::Gain).with("db", -3.0)], 2), input: Stages::empty() });
        let g = mb.take_into(0, &mut current).expect("a new plan");
        assert_eq!(current.output.len(), 1);
        assert_eq!(mb.take_into(g, &mut current), None, "taken once");
        // a second plan cannot be picked up until the control thread has freed the retired one by posting
        mb.post(Plan::default());
        let g2 = mb.take_into(g, &mut current).expect("the retired slot was emptied by the post");
        assert!(current.output.is_empty());
        assert_ne!(g, g2);
        mb.post(Plan::default());
        assert_eq!(mb.generation(), g2 + 1);
    }
}
