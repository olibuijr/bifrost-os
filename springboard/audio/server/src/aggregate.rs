//! Aggregate and multi-output devices with drift compensation (task 11; docs/audio-architecture.md, "Hog mode,
//! aggregates and taps" and design decision 4).
//!
//! An aggregate is an ordered list of sub-device ids, one clock source, and a drift-compensation flag per
//! sub-device. The server runs one cycle per clock-source period; every other sub-device has its own hardware clock
//! that drifts against it by some tens or hundreds of ppm. Each such sub-device gets
//!
//! * a delay-locked loop (an alpha-beta tracker) that learns `d(sub frames) / d(clock frames)` from the hardware
//!   frame counters the drivers report (`HwPosition::hw_frames`), and
//! * a [`Varispeed`] resampler whose ratio is that rate, nudged by a slow correction on the sub-device's fill level
//!   so the transient of the loop never accumulates into an offset.
//!
//! Output: the mix (clock domain, `quantum` frames) is resampled into `count` frames for the sub-device, where
//! `count` is `quantum * ratio` plus a fractional carry; `count` varies by one frame between cycles but the ratio
//! the resampler sees is smooth. Input is the mirror image through a small FIFO.
//!
//! The same mix can go to several outputs ([`AggregateMode::Mirror`], a multi-output device); an aggregate proper
//! concatenates the sub-devices' channels ([`AggregateMode::Concat`]). Private aggregates belong to one client
//! (needed by taps) and disappear with it.
//!
//! Real-time rules: [`AggregateIo`] allocates only in `new`; `begin_cycle`, `render_output`, `push_input` and
//! `render_input` never allocate, lock or make a syscall. The registry ([`AggregateSet`]) is control-plane only.

use bifrost_audio_toolbox::varispeed::Varispeed;

/// The most sub-devices one aggregate holds.
pub const MAX_SUBS: usize = 16;
/// The most channels one sub-device can carry through the resampler.
pub const MAX_CHANNELS: usize = 64;
/// Loop gain of the rate tracker (position gain); the rate gain is the critically damped `a*a / (2 - a)`.
const DLL_ALPHA: f64 = 0.02;
/// The fill-level correction closes an offset over this many cycles.
const FILL_TAU_CYCLES: f64 = 256.0;
/// Rates outside this window are not drift but a different device; they are clamped.
const RATE_CLAMP: (f64, f64) = (0.95, 1.05);

/// How the sub-devices combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateMode {
    /// One device whose channels are the sub-devices' channels in order.
    Concat,
    /// A multi-output device: every sub-device plays the same mix.
    Mirror,
}

/// What an aggregate is.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateDesc {
    pub uid: String,
    pub name: String,
    /// Ordered sub-device ids.
    pub sub_uids: Vec<String>,
    /// Index into `sub_uids` of the device whose clock drives the cycle.
    pub clock_source: usize,
    /// Per sub-device: resample it against the clock source. The clock source itself ignores this.
    pub drift_comp: Vec<bool>,
    pub mode: AggregateMode,
    /// Some(pid): a private aggregate that only this client sees and that is removed when it exits.
    pub owner: Option<u32>,
}

impl AggregateDesc {
    /// A public aggregate with drift compensation on for every sub-device but the clock source.
    pub fn new(uid: &str, name: &str, sub_uids: &[&str], clock_source: usize, mode: AggregateMode) -> AggregateDesc {
        AggregateDesc {
            uid: uid.to_string(),
            name: name.to_string(),
            sub_uids: sub_uids.iter().map(|s| s.to_string()).collect(),
            clock_source,
            drift_comp: vec![true; sub_uids.len()],
            mode,
            owner: None,
        }
    }

    /// A private aggregate for `owner` (a client pid).
    pub fn private(uid: &str, sub_uids: &[&str], clock_source: usize, owner: u32) -> AggregateDesc {
        let mut d = AggregateDesc::new(uid, uid, sub_uids, clock_source, AggregateMode::Concat);
        d.owner = Some(owner);
        d
    }

    pub fn validate(&self) -> Result<(), AggregateError> {
        if self.uid.is_empty() {
            return Err(AggregateError::BadUid);
        }
        if self.sub_uids.is_empty() {
            return Err(AggregateError::Empty);
        }
        if self.sub_uids.len() > MAX_SUBS {
            return Err(AggregateError::TooMany);
        }
        if self.clock_source >= self.sub_uids.len() || self.drift_comp.len() != self.sub_uids.len() {
            return Err(AggregateError::BadClock);
        }
        for (i, u) in self.sub_uids.iter().enumerate() {
            if u == &self.uid {
                return Err(AggregateError::SelfReference);
            }
            if self.sub_uids[..i].contains(u) {
                return Err(AggregateError::DuplicateSub);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateError {
    BadUid,
    Empty,
    TooMany,
    BadClock,
    DuplicateSub,
    SelfReference,
    UidExists,
    NotFound,
    NotOwner,
}

/// The registry of aggregates (control plane).
#[derive(Debug, Default)]
pub struct AggregateSet {
    items: Vec<AggregateDesc>,
}

impl AggregateSet {
    pub fn new() -> AggregateSet {
        AggregateSet::default()
    }

    pub fn create(&mut self, desc: AggregateDesc) -> Result<(), AggregateError> {
        desc.validate()?;
        if self.items.iter().any(|a| a.uid == desc.uid) {
            return Err(AggregateError::UidExists);
        }
        // An aggregate may not contain another aggregate (the sub-device is a physical or virtual device).
        if self.items.iter().any(|a| desc.sub_uids.contains(&a.uid)) {
            return Err(AggregateError::SelfReference);
        }
        eprintln!("[is.olibuijr.audio/aggregate] create {} ({} sub-devices)", desc.uid, desc.sub_uids.len());
        self.items.push(desc);
        Ok(())
    }

    /// Removes an aggregate. A private one can be removed only by its owner (`requester` = its pid); `None` is the
    /// server itself.
    pub fn remove(&mut self, uid: &str, requester: Option<u32>) -> Result<AggregateDesc, AggregateError> {
        let idx = self.items.iter().position(|a| a.uid == uid).ok_or(AggregateError::NotFound)?;
        if let (Some(owner), Some(req)) = (self.items[idx].owner, requester) {
            if owner != req {
                return Err(AggregateError::NotOwner);
            }
        }
        eprintln!("[is.olibuijr.audio/aggregate] remove {uid}");
        Ok(self.items.remove(idx))
    }

    /// Drops every private aggregate of an exited client; returns their ids.
    pub fn remove_owner(&mut self, pid: u32) -> Vec<String> {
        let gone: Vec<String> = self.items.iter().filter(|a| a.owner == Some(pid)).map(|a| a.uid.clone()).collect();
        self.items.retain(|a| a.owner != Some(pid));
        gone
    }

    pub fn get(&self, uid: &str) -> Option<&AggregateDesc> {
        self.items.iter().find(|a| a.uid == uid)
    }

    /// The aggregates a client may see: public ones and its own private ones (`None`: all, for the server).
    pub fn visible_to(&self, pid: Option<u32>) -> Vec<&AggregateDesc> {
        self.items.iter().filter(|a| pid.is_none() || a.owner.is_none() || a.owner == pid).collect()
    }

    /// Whether `uid` is a sub-device of some aggregate (so it cannot be hogged out from under it silently, and the
    /// device list can mark it).
    pub fn is_member(&self, uid: &str) -> bool {
        self.items.iter().any(|a| a.sub_uids.iter().any(|u| u == uid))
    }
}

/// A second-order delay-locked loop (an alpha-beta tracker) learning `y` as a function of `x`: here the
/// sub-device's hardware frames against the clock source's.
#[derive(Debug, Clone)]
pub struct Dll {
    alpha: f64,
    beta: f64,
    x: f64,
    phase: f64,
    rate: f64,
    primed: bool,
}

impl Dll {
    pub fn new(alpha: f64) -> Dll {
        Dll { alpha, beta: alpha * alpha / (2.0 - alpha), x: 0.0, phase: 0.0, rate: 1.0, primed: false }
    }

    /// Starts tracking at `(x, y)` with the nominal rate 1.0.
    pub fn prime(&mut self, x: f64, y: f64) {
        self.x = x;
        self.phase = y;
        self.rate = 1.0;
        self.primed = true;
    }

    pub fn primed(&self) -> bool {
        self.primed
    }

    /// One observation. An `x` that did not advance is ignored.
    pub fn update(&mut self, x: f64, y: f64) {
        if !self.primed {
            self.prime(x, y);
            return;
        }
        let dx = x - self.x;
        if dx <= 0.0 {
            return;
        }
        let pred = self.phase + self.rate * dx;
        let e = y - pred;
        self.phase = pred + self.alpha * e;
        self.rate += self.beta * e / dx;
        self.x = x;
    }

    /// The learned `dy/dx`.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The smoothed `y` at the last observed `x`.
    pub fn phase(&self) -> f64 {
        self.phase
    }
}

/// Per sub-device state of the output path.
struct SubOut {
    channels: u32,
    drift: bool,
    dll: Dll,
    vs: Varispeed,
    /// Frames written to the device since start (same epoch as its `hw_frames`).
    written: u64,
    /// Fractional frames owed to the next cycle.
    carry: f64,
    /// Intended `written - hw` fill, set by the prefill.
    lead: f64,
    ratio: f64,
}

/// The real-time side of an aggregate for output: the drift accounting and the resamplers.
pub struct AggregateIo {
    quantum: u32,
    clock: usize,
    prefill: u32,
    subs: Vec<SubOut>,
    counts: Vec<u32>,
    cycles: u64,
}

impl AggregateIo {
    /// `channels[i]` is sub-device `i`'s output channel count. `prefill` is the number of frames the caller has
    /// already written to each sub-device before the first cycle (the device's buffer lead).
    pub fn new(desc: &AggregateDesc, quantum: u32, channels: &[u32], prefill: u32) -> AggregateIo {
        let n = desc.sub_uids.len().min(channels.len());
        let subs = (0..n)
            .map(|i| SubOut {
                channels: channels[i],
                drift: i != desc.clock_source && desc.drift_comp.get(i).copied().unwrap_or(false),
                dll: Dll::new(DLL_ALPHA),
                vs: Varispeed::new(channels[i]),
                written: 0,
                carry: 0.0,
                lead: f64::from(prefill),
                ratio: 1.0,
            })
            .collect();
        AggregateIo { quantum, clock: desc.clock_source, prefill, subs, counts: vec![quantum; n], cycles: 0 }
    }

    pub fn quantum(&self) -> u32 {
        self.quantum
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }

    /// The most frames one sub-device is ever given in one cycle (size scratch buffers with this).
    pub fn max_frames(&self) -> u32 {
        let q = f64::from(self.quantum);
        (q * RATE_CLAMP.1 * (1.0 + 1.0 / FILL_TAU_CYCLES)).ceil() as u32 + 2
    }

    /// Starts one cycle. `hw[i]` is sub-device `i`'s hardware frame count right now (`hw[clock]` defines the cycle).
    /// Returns the frames each sub-device is to receive this cycle.
    pub fn begin_cycle(&mut self, hw: &[u64]) -> &[u32] {
        let q = f64::from(self.quantum);
        let x = hw.get(self.clock).copied().unwrap_or(self.cycles * u64::from(self.quantum)) as f64;
        self.cycles += 1;
        for (i, s) in self.subs.iter_mut().enumerate() {
            let y = hw.get(i).copied().unwrap_or(0);
            if !s.dll.primed() {
                s.written = y + u64::from(self.prefill);
            }
            if i == self.clock || !s.drift {
                s.dll.update(x, y as f64);
                s.written += u64::from(self.quantum);
                s.ratio = 1.0;
                self.counts[i] = self.quantum;
                continue;
            }
            s.dll.update(x, y as f64);
            let rate = s.dll.rate().clamp(RATE_CLAMP.0, RATE_CLAMP.1);
            let expected = s.dll.phase() + s.lead;
            let err = expected - (s.written as f64 + s.carry);
            let ratio = (rate + err / (q * FILL_TAU_CYCLES)).clamp(RATE_CLAMP.0, RATE_CLAMP.1);
            let total = q * ratio + s.carry;
            let n = total.floor();
            s.carry = total - n;
            s.written += n as u64;
            s.ratio = ratio;
            self.counts[i] = n as u32;
        }
        &self.counts
    }

    /// The resampling ratio sub-device `i` runs at this cycle.
    pub fn ratio(&self, i: usize) -> f64 {
        self.subs[i].ratio
    }

    /// The frames sub-device `i` receives this cycle (after `begin_cycle`).
    pub fn frames_for(&self, i: usize) -> u32 {
        self.counts[i]
    }

    /// `written - hw` in fractional frames: the quantity compensation keeps constant.
    pub fn fill(&self, i: usize, hw: u64) -> f64 {
        self.subs[i].written as f64 + self.subs[i].carry - hw as f64
    }

    /// Resamples sub-device `i`'s channels of the mix (`mix[c]` is `quantum` frames) into `out[c]` (at least
    /// `frames_for(i)` long); returns the frames produced. The clock source and uncompensated devices are copied.
    pub fn render_output(&mut self, i: usize, mix: &[&[f32]], out: &mut [&mut [f32]]) -> u32 {
        let n = self.counts[i];
        let s = &mut self.subs[i];
        let ch = (s.channels as usize).min(mix.len()).min(out.len());
        if i == self.clock || !s.drift {
            for c in 0..ch {
                out[c][..n as usize].copy_from_slice(&mix[c][..n as usize]);
            }
            return n;
        }
        // The resampler's ratio is out/in with `quantum` input frames feeding `n` output frames: the cycle's
        // exact ratio is n / quantum (smooth over time because `carry` spreads the rounding).
        s.vs.set_ratio(f64::from(n) / f64::from(self.quantum));
        let mut ins: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for c in 0..ch.min(MAX_CHANNELS) {
            ins[c] = mix[c];
        }
        s.vs.process(&ins[..ch.min(MAX_CHANNELS)], &mut out[..ch], n);
        n
    }

    /// Processing delay of sub-device `i` in clock-domain frames, for latency reporting.
    pub fn latency(&self, i: usize) -> f64 {
        if i == self.clock || !self.subs[i].drift {
            0.0
        } else {
            self.subs[i].vs.latency()
        }
    }
}

/// Per sub-device state of the input path: a FIFO of captured frames and the resampler out of it.
struct SubIn {
    channels: u32,
    drift: bool,
    fifo: Vec<Vec<f32>>,
    /// Valid frames are `fifo[c][..len]`.
    len: usize,
    vs: Varispeed,
    dll: Dll,
    target: f64,
    underruns: u64,
}

/// The real-time side of an aggregate for input.
pub struct AggregateIn {
    quantum: u32,
    clock: usize,
    subs: Vec<SubIn>,
}

impl AggregateIn {
    /// `target_fill` is the number of frames each FIFO should hold at the start of a cycle.
    pub fn new(desc: &AggregateDesc, quantum: u32, channels: &[u32], target_fill: u32) -> AggregateIn {
        let n = desc.sub_uids.len().min(channels.len());
        let cap = (quantum as usize + target_fill as usize) * 4 + 16;
        let subs = (0..n)
            .map(|i| SubIn {
                channels: channels[i],
                drift: i != desc.clock_source && desc.drift_comp.get(i).copied().unwrap_or(false),
                fifo: vec![vec![0.0; cap]; channels[i] as usize],
                len: 0,
                vs: Varispeed::new(channels[i]),
                dll: Dll::new(DLL_ALPHA),
                target: f64::from(target_fill),
                underruns: 0,
            })
            .collect();
        AggregateIn { quantum, clock: desc.clock_source, subs }
    }

    /// Appends captured frames (`src[c]` per channel) of sub-device `i`. Frames that do not fit are dropped.
    pub fn push_input(&mut self, i: usize, src: &[&[f32]]) {
        let s = &mut self.subs[i];
        let frames = src.iter().map(|x| x.len()).min().unwrap_or(0);
        let room = s.fifo.first().map(|f| f.len() - s.len).unwrap_or(0);
        let n = frames.min(room);
        for (c, f) in s.fifo.iter_mut().enumerate() {
            if let Some(sc) = src.get(c) {
                f[s.len..s.len + n].copy_from_slice(&sc[..n]);
            }
        }
        s.len += n;
    }

    /// Feeds the rate tracker: `hw[i]` as in [`AggregateIo::begin_cycle`].
    pub fn observe(&mut self, hw: &[u64]) {
        let x = hw.get(self.clock).copied().unwrap_or(0) as f64;
        for (i, s) in self.subs.iter_mut().enumerate() {
            if let Some(&y) = hw.get(i) {
                s.dll.update(x, y as f64);
            }
        }
    }

    /// Produces `quantum` frames of sub-device `i` at the clock rate into `out[c]`. Underruns give silence.
    pub fn render_input(&mut self, i: usize, out: &mut [&mut [f32]]) {
        let q = self.quantum as usize;
        let s = &mut self.subs[i];
        let ch = (s.channels as usize).min(out.len());
        if i == self.clock || !s.drift {
            if s.len < q {
                s.underruns += 1;
                for c in 0..ch {
                    out[c][..q].iter_mut().for_each(|x| *x = 0.0);
                }
                return;
            }
            for c in 0..ch {
                out[c][..q].copy_from_slice(&s.fifo[c][..q]);
            }
            Self::consume(s, q);
            return;
        }
        // Output (clock) frames per input (sub) frame is clock/sub = 1 / rate; a FIFO fuller than its target is
        // drained slightly faster (a lower out/in ratio).
        let rate = s.dll.rate().clamp(RATE_CLAMP.0, RATE_CLAMP.1);
        let level = s.len as f64 - s.target;
        let ratio = ((1.0 / rate) * (1.0 - level / (f64::from(self.quantum) * FILL_TAU_CYCLES)))
            .clamp(RATE_CLAMP.0, RATE_CLAMP.1);
        s.vs.set_ratio(ratio);
        let need = s.vs.input_needed(self.quantum) as usize;
        if s.len < need {
            s.underruns += 1;
            for c in 0..ch {
                out[c][..q].iter_mut().for_each(|x| *x = 0.0);
            }
            return;
        }
        let mut ins: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for c in 0..ch.min(MAX_CHANNELS) {
            ins[c] = &s.fifo[c][..need];
        }
        let used = s.vs.process(&ins[..ch.min(MAX_CHANNELS)], &mut out[..ch], self.quantum) as usize;
        Self::consume(s, used);
    }

    fn consume(s: &mut SubIn, n: usize) {
        for f in &mut s.fifo {
            f.copy_within(n..s.len, 0);
        }
        s.len -= n;
    }

    /// Frames waiting in sub-device `i`'s FIFO.
    pub fn level(&self, i: usize) -> usize {
        self.subs[i].len
    }

    pub fn underruns(&self, i: usize) -> u64 {
        self.subs[i].underruns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;
    const Q: u32 = 64;

    #[test]
    fn registry_validates_and_scopes_private_aggregates() {
        let mut set = AggregateSet::new();
        let a = AggregateDesc::new("agg1", "Desk", &["hw:0", "hw:1"], 0, AggregateMode::Concat);
        assert_eq!(set.create(a.clone()), Ok(()));
        assert_eq!(set.create(a), Err(AggregateError::UidExists));
        assert_eq!(
            set.create(AggregateDesc::new("x", "x", &["a", "a"], 0, AggregateMode::Concat)),
            Err(AggregateError::DuplicateSub)
        );
        assert_eq!(
            set.create(AggregateDesc::new("x", "x", &["a", "b"], 2, AggregateMode::Concat)),
            Err(AggregateError::BadClock)
        );
        assert_eq!(
            set.create(AggregateDesc::new("x", "x", &[], 0, AggregateMode::Concat)),
            Err(AggregateError::Empty)
        );
        // An aggregate cannot contain an aggregate.
        assert_eq!(
            set.create(AggregateDesc::new("y", "y", &["agg1", "hw:2"], 0, AggregateMode::Mirror)),
            Err(AggregateError::SelfReference)
        );
        assert_eq!(set.create(AggregateDesc::private("tap-agg", &["hw:0"], 0, 4242)), Ok(()));
        assert_eq!(set.visible_to(Some(4242)).len(), 2);
        assert_eq!(set.visible_to(Some(7)).len(), 1);
        assert_eq!(set.visible_to(None).len(), 2);
        assert!(set.is_member("hw:1"));
        assert!(!set.is_member("hw:9"));
        assert_eq!(set.remove("tap-agg", Some(7)).unwrap_err(), AggregateError::NotOwner);
        assert_eq!(set.remove_owner(4242), vec!["tap-agg".to_string()]);
        assert!(set.get("tap-agg").is_none());
        assert_eq!(set.remove("agg1", None).unwrap().uid, "agg1");
        assert_eq!(set.remove("agg1", None).unwrap_err(), AggregateError::NotFound);
    }

    #[test]
    fn dll_learns_a_constant_rate_exactly() {
        // y = 1.0004 * x. After enough updates the rate is 1.0004 and the phase is on the line.
        let mut d = Dll::new(DLL_ALPHA);
        for k in 0..20_000u64 {
            let x = (k * 64) as f64;
            d.update(x, x * 1.0004);
        }
        assert!((d.rate() - 1.0004).abs() < 1e-9, "{}", d.rate());
        assert!((d.phase() - 19_999.0 * 64.0 * 1.0004).abs() < 1e-3);
    }

    /// Simulated hardware: sub-device `i` runs at `48000 * (1 + ppm_i * 1e-6)` frames per second. The clock source
    /// sets the cycle; the others' counters are read at the same instant (integer frames, as the drivers report).
    fn simulate(ppm: &[f64], clock: usize, seconds: f64, prefill: u32, warmup_s: f64) -> f64 {
        let subs: Vec<String> = (0..ppm.len()).map(|i| format!("d{i}")).collect();
        let refs: Vec<&str> = subs.iter().map(|s| s.as_str()).collect();
        let desc = AggregateDesc::new("agg", "agg", &refs, clock, AggregateMode::Concat);
        let mut io = AggregateIo::new(&desc, Q, &vec![2; ppm.len()], prefill);
        let cycles = (seconds * RATE / f64::from(Q)) as u64;
        let warm = (warmup_s * RATE / f64::from(Q)) as u64;
        let clock_rate = RATE * (1.0 + ppm[clock] * 1e-6);
        let mut hw = vec![0u64; ppm.len()];
        let mut worst = 0.0f64;
        let mut max_count = 0;
        for k in 1..=cycles {
            let t = (k * u64::from(Q)) as f64 / clock_rate; // seconds on the host
            let truth: Vec<f64> = ppm.iter().map(|p| t * RATE * (1.0 + p * 1e-6)).collect();
            for i in 0..ppm.len() {
                hw[i] = truth[i].floor() as u64;
            }
            hw[clock] = k * u64::from(Q);
            io.begin_cycle(&hw);
            for i in 0..ppm.len() {
                max_count = max_count.max(io.frames_for(i));
            }
            if k > warm {
                for i in 0..ppm.len() {
                    // Alignment is relative to the clock source (whose own fill is the exact `prefill + quantum`).
                    let e = (io.fill(i, 0) - truth[i]) - (io.fill(clock, 0) - truth[clock]);
                    worst = worst.max(e.abs());
                }
            }
        }
        assert!(max_count <= io.max_frames());
        worst
    }

    #[test]
    fn plus_200_and_minus_150_ppm_stay_aligned_within_one_frame_for_an_hour() {
        // Clock source = the +200 ppm device; the -150 ppm one is compensated (relative drift 350 ppm).
        let w = simulate(&[200.0, -150.0], 0, 3600.0, 128, 20.0);
        assert!(w < 1.0, "worst misalignment {w} frames");
        // And with the slow device as the clock source.
        let w = simulate(&[200.0, -150.0], 1, 3600.0, 128, 20.0);
        assert!(w < 1.0, "worst misalignment {w} frames");
        // Three devices, the middle one is the clock.
        let w = simulate(&[200.0, 0.0, -150.0], 1, 600.0, 128, 20.0);
        assert!(w < 1.0, "worst misalignment {w} frames");
    }

    #[test]
    fn the_clock_source_and_uncompensated_devices_get_exactly_one_quantum() {
        let mut desc = AggregateDesc::new("agg", "agg", &["a", "b", "c"], 0, AggregateMode::Mirror);
        desc.drift_comp[2] = false;
        let mut io = AggregateIo::new(&desc, Q, &[2, 2, 2], 0);
        for k in 1..100u64 {
            let hw = [k * 64, k * 64 + 3, k * 64];
            let c = io.begin_cycle(&hw).to_vec();
            assert_eq!(c[0], 64);
            assert_eq!(c[2], 64);
            assert!(c[1] == 64 || c[1] == 65 || c[1] == 63);
        }
    }

    #[test]
    fn render_output_copies_for_the_clock_and_resamples_for_a_drifting_device() {
        let desc = AggregateDesc::new("agg", "agg", &["a", "b"], 0, AggregateMode::Mirror);
        let mut io = AggregateIo::new(&desc, Q, &[1, 1], 0);
        let mix: Vec<f32> = (0..64).map(|x| x as f32).collect();
        // b runs 1 % fast: after the loop locks it receives about 64.6 frames per cycle.
        let mut counts = Vec::new();
        for k in 1..=3000u64 {
            let hw = [k * 64, (k as f64 * 64.0 * 1.01) as u64];
            io.begin_cycle(&hw);
            let mut oa = vec![0.0f32; 80];
            let mut ob = vec![0.0f32; 80];
            let na = io.render_output(0, &[&mix], &mut [&mut oa[..]]);
            let nb = io.render_output(1, &[&mix], &mut [&mut ob[..]]);
            assert_eq!(na, 64);
            assert_eq!(&oa[..64], &mix[..]);
            assert!(ob[nb as usize..].iter().all(|s| *s == 0.0));
            counts.push(nb);
        }
        let tail: u32 = counts[2000..].iter().sum();
        let mean = f64::from(tail) / 1000.0;
        assert!((mean - 64.64).abs() < 0.02, "mean {mean}");
        assert!(io.latency(0) == 0.0 && io.latency(1) > 1.0 && io.latency(1) < 2.0);
    }

    #[test]
    fn input_fifo_stays_bounded_with_a_fast_capture_device() {
        // Sub-device b captures 300 ppm faster than the clock source a. Its FIFO must neither grow nor run dry.
        let desc = AggregateDesc::new("agg", "agg", &["a", "b"], 0, AggregateMode::Concat);
        let mut inp = AggregateIn::new(&desc, Q, &[1, 1], 128);
        let silence = vec![0.0f32; 64];
        inp.push_input(1, &[&silence]);
        inp.push_input(1, &[&silence]);
        let cycles = (60.0 * RATE / f64::from(Q)) as u64;
        let (mut pushed, mut lo, mut hi) = (128u64, usize::MAX, 0usize);
        let mut out = vec![0.0f32; 64];
        for k in 1..=cycles {
            let target_total = 128 + (k as f64 * 64.0 * 1.0003) as u64;
            let n = (target_total - pushed) as usize;
            let chunk = vec![0.0f32; n];
            inp.push_input(1, &[&chunk]);
            pushed += n as u64;
            inp.observe(&[k * 64, pushed - 128]);
            inp.render_input(1, &mut [&mut out[..]]);
            if k > 3000 {
                lo = lo.min(inp.level(1));
                hi = hi.max(inp.level(1));
            }
        }
        assert_eq!(inp.underruns(1), 0);
        // The target is 128 frames at the start of a cycle, so 64 after the cycle consumed its quantum.
        assert!(lo >= 56 && hi <= 72, "fifo level {lo}..{hi}");
    }

    #[test]
    fn max_frames_covers_the_clamped_ratio() {
        let desc = AggregateDesc::new("agg", "agg", &["a"], 0, AggregateMode::Concat);
        let io = AggregateIo::new(&desc, 64, &[2], 0);
        // 64 * 1.05 * (1 + 1/256) = 67.46 -> 68, plus 2.
        assert_eq!(io.max_frames(), 70);
    }
}
