//! `GmSynth`: a polyphonic General MIDI instrument that needs no sound bank. It is an `AudioUnit` (so it sits in an
//! `Engine`, behind a `SequencerUnit`, or in a `UnitHost`), driven by sample-accurate MIDI.
//!
//! * 32 voices of an additive tone (a few harmonics of a sine, chosen by the General MIDI program: 0 to 31 soft,
//!   32 to 55 warm, the rest bright) shaped by a linear attack, decay, sustain and release envelope.
//! * Channel 10 (index 9) plays drums as noise bursts that decay on their own, longer for lower keys.
//! * Per channel: program change, volume (CC 7), pan (CC 10, constant power), expression (CC 11), sustain pedal
//!   (CC 64), pitch bend (plus or minus two semitones), all notes off (CC 123), all sound off (CC 120) and reset
//!   controllers (CC 121).
//! * One parameter: `Level` (0 to 1), ramped.
//!
//! Not modelled: recorded instrument samples, reverb and chorus, soft pedal, the modulation wheel. The sound is
//! simpler than a sound bank's; the point is that MIDI reaches the speakers through the same graph as every other
//! unit. `render` allocates nothing: the level curve is made in `initialize` and the voices are a fixed array.

use crate::midi::{note_to_hz, MidiEvent};
use crate::param::{ParamSpec, ParamUnit, ParameterTree};
use crate::unit::{check_config, AudioUnit, Block, RenderConfig, UnitError, UnitInfo, UnitKind};
use std::f64::consts::TAU;

pub const SYNTH_LEVEL: u32 = 0;
const VOICES: usize = 32;
const DRUM_CHANNEL: usize = 9;

const ATTACK_S: f64 = 0.005;
const DECAY_S: f64 = 0.1;
const SUSTAIN: f32 = 0.7;
const RELEASE_S: f64 = 0.15;

/// Harmonic weights per timbre and the factor that keeps each at the same peak level.
const HARMONICS: [[f32; 4]; 3] = [[1.0, 0.2, 0.0, 0.0], [1.0, 0.5, 0.25, 0.0], [1.0, 0.6, 0.4, 0.3]];
const NORMAL: [f32; 3] = [1.0 / 1.2, 1.0 / 1.75, 1.0 / 2.3];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    channel: u8,
    key: u8,
    phase: f64,
    base_step: f64,
    vel: f32,
    stage: Stage,
    level: f32,
    release_step: f32,
    /// Note-off arrived while the sustain pedal was down.
    held_by_pedal: bool,
    drum: bool,
    noise: u32,
    decay_mul: f32,
    age: u32,
}

impl Voice {
    const IDLE: Voice = Voice {
        active: false,
        channel: 0,
        key: 0,
        phase: 0.0,
        base_step: 0.0,
        vel: 0.0,
        stage: Stage::Release,
        level: 0.0,
        release_step: 0.0,
        held_by_pedal: false,
        drum: false,
        noise: 1,
        decay_mul: 0.999,
        age: 0,
    };
}

#[derive(Clone, Copy)]
struct Channel {
    program: u8,
    volume: f32,
    expression: f32,
    pan: f32,
    bend: f32,
    pedal: bool,
}

impl Channel {
    const DEFAULT: Channel = Channel { program: 0, volume: 100.0 / 127.0, expression: 1.0, pan: 64.0 / 127.0, bend: 0.0, pedal: false };

    fn timbre(&self) -> usize {
        match self.program {
            0..=31 => 0,
            32..=55 => 1,
            _ => 2,
        }
    }

    /// Left and right gains: volume times expression times a constant-power pan.
    fn gains(&self) -> (f32, f32) {
        let g = self.volume * self.expression;
        let a = self.pan.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
        (g * a.cos(), g * a.sin())
    }

    fn bend_ratio(&self) -> f64 {
        2f64.powf(self.bend as f64 * 2.0 / 12.0)
    }
}

pub struct GmSynth {
    info: UnitInfo,
    tree: ParameterTree,
    rate: f64,
    voices: [Voice; VOICES],
    channels: [Channel; 16],
    curve: Vec<f32>,
    ready: bool,
    counter: u32,
    attack_step: f32,
    decay_step: f32,
}

impl GmSynth {
    pub fn new() -> GmSynth {
        let specs = vec![ParamSpec::new(SYNTH_LEVEL, "Level", ParamUnit::Ratio, 0.0, 1.0, 0.8).expect("static spec").with_ramp(64)];
        GmSynth {
            info: UnitInfo { name: "General MIDI Synth".into(), kind: UnitKind::Instrument, accepts_midi: true, max_channels: 2 },
            tree: ParameterTree::new(specs).expect("static tree"),
            rate: 48_000.0,
            voices: [Voice::IDLE; VOICES],
            channels: [Channel::DEFAULT; 16],
            curve: Vec::new(),
            ready: false,
            counter: 0,
            attack_step: 0.0,
            decay_step: 0.0,
        }
    }

    /// The number of voices sounding (including ones in their release).
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    fn start_note(&mut self, channel: usize, key: u8, velocity: u8) {
        // a free voice, else a releasing one (the quietest), else the quietest of all
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active)
            .or_else(|| self.pick(|v| v.stage == Stage::Release))
            .or_else(|| self.pick(|_| true))
            .unwrap_or(0);
        self.counter = self.counter.wrapping_add(1);
        let drum = channel == DRUM_CHANNEL;
        let hz = note_to_hz(key);
        // lower drum keys ring longer: 0.05 s to 0.3 s to fall by 60 dB
        let drum_len = 0.05 + (127 - key.min(127)) as f64 / 127.0 * 0.25;
        self.voices[slot] = Voice {
            active: true,
            channel: channel as u8,
            key,
            phase: 0.0,
            base_step: hz / self.rate,
            vel: velocity as f32 / 127.0,
            stage: Stage::Attack,
            level: if drum { 1.0 } else { 0.0 },
            release_step: 0.0,
            held_by_pedal: false,
            drum,
            noise: (0x9E37_79B9 ^ (key as u32).wrapping_mul(2_654_435_761) ^ self.counter) | 1,
            decay_mul: (0.001f64).powf(1.0 / (drum_len * self.rate)) as f32,
            age: self.counter,
        };
    }

    /// The index of the quietest voice (oldest on ties) that `want` accepts.
    fn pick(&self, want: impl Fn(&Voice) -> bool) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active || !want(v) {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) => {
                    let w = &self.voices[b];
                    if v.level < w.level || (v.level == w.level && v.age < w.age) {
                        Some(i)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        best
    }

    fn release(&mut self, i: usize) {
        let v = &mut self.voices[i];
        if v.drum {
            return; // drums decay on their own
        }
        v.stage = Stage::Release;
        v.held_by_pedal = false;
        v.release_step = (v.level.max(0.001)) / (RELEASE_S * self.rate) as f32;
    }

    fn stop_note(&mut self, channel: usize, key: u8) {
        for i in 0..VOICES {
            let v = self.voices[i];
            if v.active && v.channel as usize == channel && v.key == key && v.stage != Stage::Release && !v.held_by_pedal {
                if self.channels[channel].pedal {
                    self.voices[i].held_by_pedal = true;
                } else {
                    self.release(i);
                }
            }
        }
    }

    fn controller(&mut self, channel: usize, cc: u8, value: u8) {
        let v = value as f32 / 127.0;
        match cc {
            7 => self.channels[channel].volume = v,
            10 => self.channels[channel].pan = v,
            11 => self.channels[channel].expression = v,
            64 => {
                let down = value >= 64;
                self.channels[channel].pedal = down;
                if !down {
                    for i in 0..VOICES {
                        let vo = self.voices[i];
                        if vo.active && vo.channel as usize == channel && vo.held_by_pedal {
                            self.release(i);
                        }
                    }
                }
            }
            120 => {
                for vo in self.voices.iter_mut().filter(|vo| vo.channel as usize == channel) {
                    vo.active = false;
                }
            }
            121 => {
                let program = self.channels[channel].program;
                self.channels[channel] = Channel { program, ..Channel::DEFAULT };
            }
            123 => {
                for i in 0..VOICES {
                    let vo = self.voices[i];
                    if vo.active && vo.channel as usize == channel && vo.stage != Stage::Release {
                        self.release(i);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle(&mut self, e: &MidiEvent) {
        let ch = (e.status & 15) as usize;
        match e.status & 0xF0 {
            0x90 if e.data2 > 0 => self.start_note(ch, e.data1 & 127, e.data2 & 127),
            0x80 | 0x90 => self.stop_note(ch, e.data1 & 127),
            0xB0 => self.controller(ch, e.data1 & 127, e.data2 & 127),
            0xC0 => self.channels[ch].program = e.data1 & 127,
            0xE0 => {
                let raw = ((e.data2 & 127) as i32) << 7 | (e.data1 & 127) as i32;
                self.channels[ch].bend = (raw - 8192) as f32 / 8192.0;
            }
            _ => {}
        }
    }

    /// Adds the voices' output for frames `from..to` of the block.
    fn mix(&mut self, outs: &mut [&mut [f32]], from: usize, to: usize) {
        let (first, rest) = outs.split_at_mut(1);
        let left: &mut [f32] = &mut *first[0];
        let mut right: Option<&mut [f32]> = rest.first_mut().map(|r| &mut **r);
        let (attack_step, decay_step) = (self.attack_step, self.decay_step);
        for v in self.voices.iter_mut() {
            if !v.active {
                continue;
            }
            let ch = self.channels[v.channel as usize];
            let (gl, gr) = ch.gains();
            let step = v.base_step * ch.bend_ratio();
            let timbre = ch.timbre();
            let (h, norm) = (HARMONICS[timbre], NORMAL[timbre]);
            for i in from..to {
                let s = if v.drum {
                    // a xorshift noise burst
                    v.noise ^= v.noise << 13;
                    v.noise ^= v.noise >> 17;
                    v.noise ^= v.noise << 5;
                    let n = (v.noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    v.level *= v.decay_mul;
                    if v.level < 1e-4 {
                        v.active = false;
                    }
                    n * v.level * v.vel * 0.5
                } else {
                    match v.stage {
                        Stage::Attack => {
                            v.level += attack_step;
                            if v.level >= 1.0 {
                                v.level = 1.0;
                                v.stage = Stage::Decay;
                            }
                        }
                        Stage::Decay => {
                            v.level -= decay_step;
                            if v.level <= SUSTAIN {
                                v.level = SUSTAIN;
                                v.stage = Stage::Sustain;
                            }
                        }
                        Stage::Sustain => {}
                        Stage::Release => {
                            v.level -= v.release_step;
                            if v.level <= 0.0 {
                                v.level = 0.0;
                                v.active = false;
                            }
                        }
                    }
                    let p = v.phase * TAU;
                    let mut tone = h[0] * p.sin() as f32;
                    for (k, w) in h.iter().enumerate().skip(1) {
                        if *w > 0.0 {
                            tone += *w * (p * (k + 1) as f64).sin() as f32;
                        }
                    }
                    v.phase += step;
                    if v.phase >= 1.0 {
                        v.phase -= 1.0;
                    }
                    tone * norm * v.level * v.vel * 0.25
                };
                match right.as_deref_mut() {
                    Some(r) => {
                        left[i] += s * gl;
                        r[i] += s * gr;
                    }
                    None => left[i] += s * (gl + gr) * 0.5,
                }
                if !v.active {
                    break;
                }
            }
        }
    }
}

impl Default for GmSynth {
    fn default() -> GmSynth {
        GmSynth::new()
    }
}

impl AudioUnit for GmSynth {
    fn info(&self) -> &UnitInfo {
        &self.info
    }

    fn tree(&self) -> &ParameterTree {
        &self.tree
    }

    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }

    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        check_config(&self.info, cfg)?;
        self.rate = cfg.sample_rate;
        self.curve = vec![0.0; cfg.max_frames as usize];
        self.attack_step = (1.0 / (ATTACK_S * cfg.sample_rate)) as f32;
        self.decay_step = ((1.0 - SUSTAIN as f64) / (DECAY_S * cfg.sample_rate)) as f32;
        self.ready = true;
        self.reset();
        Ok(())
    }

    fn uninitialize(&mut self) {
        self.curve = Vec::new();
        self.ready = false;
    }

    fn reset(&mut self) {
        self.voices = [Voice::IDLE; VOICES];
        self.channels = [Channel::DEFAULT; 16];
    }

    fn tail_frames(&self) -> u32 {
        (RELEASE_S * self.rate) as u32
    }

    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        self.tree.sync();
        for o in block.outputs.iter_mut() {
            o[..n].iter_mut().for_each(|s| *s = 0.0);
        }
        if !self.ready || n > self.curve.len() || block.outputs.is_empty() {
            return;
        }
        self.tree.fill(SYNTH_LEVEL, &mut self.curve[..n]);
        let mut at = 0;
        for e in block.midi.iter() {
            let off = (e.offset as usize).min(n);
            if off > at {
                self.mix(block.outputs, at, off);
                at = off;
            }
            self.handle(e);
        }
        if at < n {
            self.mix(block.outputs, at, n);
        }
        for o in block.outputs.iter_mut() {
            for (s, g) in o[..n].iter_mut().zip(self.curve[..n].iter()) {
                *s *= *g;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::UnitHost;

    const RATE: f64 = 48_000.0;

    fn host(frames: u32) -> UnitHost {
        let mut h = UnitHost::new(Box::new(GmSynth::new()));
        h.initialize(RenderConfig { sample_rate: RATE, max_frames: frames, input_channels: 0, output_channels: 2 }).unwrap();
        h
    }

    /// Renders `frames` frames in blocks of at most `max`, feeding `events` (offsets relative to the start) to the
    /// block they fall in. Returns the two channels.
    fn run(h: &mut UnitHost, frames: usize, events: &[MidiEvent], max: usize, start: &mut f64) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut done = 0;
        while done < frames {
            let n = max.min(frames - done);
            let local: Vec<MidiEvent> = events
                .iter()
                .filter(|e| (e.offset as usize) >= done && (e.offset as usize) < done + n)
                .map(|e| MidiEvent { offset: e.offset - done as u32, ..*e })
                .collect();
            let (mut a, mut b) = (vec![0.0f32; n], vec![0.0f32; n]);
            h.render(&[], &mut [a.as_mut_slice(), b.as_mut_slice()], n, *start, &local).unwrap();
            *start += n as f64;
            l.extend(a);
            r.extend(b);
            done += n;
        }
        (l, r)
    }

    fn render_events(events: &[MidiEvent], frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut h = host(512);
        let mut t = 0.0;
        run(&mut h, frames, events, 512, &mut t)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn rising_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn silence_before_the_note_and_sound_after_it_at_the_exact_frame() {
        let (l, r) = render_events(&[MidiEvent::note_on(100, 0, 69, 100)], 400);
        assert!(l[..=100].iter().chain(r[..=100].iter()).all(|s| *s == 0.0));
        assert!(rms(&l[200..400]) > 0.005);
        assert!(rms(&r[200..400]) > 0.005);
    }

    #[test]
    fn a_440_hertz_note_has_440_cycles_a_second() {
        let (l, _) = render_events(&[MidiEvent::note_on(0, 0, 69, 100)], 4800 + 480);
        let n = rising_crossings(&l[480..]);
        // 0.1 s at 440 Hz is 44 cycles
        assert!((43..=45).contains(&n), "{n} crossings");
    }

    #[test]
    fn an_octave_up_doubles_the_frequency() {
        let (a, _) = render_events(&[MidiEvent::note_on(0, 0, 57, 100)], 9600);
        let (b, _) = render_events(&[MidiEvent::note_on(0, 0, 69, 100)], 9600);
        let (ca, cb) = (rising_crossings(&a[480..]) as f32, rising_crossings(&b[480..]) as f32);
        assert!((cb / ca - 2.0).abs() < 0.1, "{ca} {cb}");
    }

    #[test]
    fn pitch_bend_up_two_semitones_raises_the_pitch_by_that_much() {
        let plain = render_events(&[MidiEvent::note_on(0, 0, 69, 100)], 9600).0;
        // bend value 16383 (maximum): data1 = low 7 bits, data2 = high 7 bits
        let bend = MidiEvent { offset: 0, status: 0xE0, data1: 127, data2: 127 };
        let bent = render_events(&[bend, MidiEvent::note_on(0, 0, 69, 100)], 9600).0;
        let ratio = rising_crossings(&bent[480..]) as f32 / rising_crossings(&plain[480..]) as f32;
        assert!((ratio - 1.1225).abs() < 0.04, "ratio {ratio}");
    }

    #[test]
    fn velocity_scales_the_level() {
        let loud = rms(&render_events(&[MidiEvent::note_on(0, 0, 69, 127)], 4800).0[960..]);
        let soft = rms(&render_events(&[MidiEvent::note_on(0, 0, 69, 64)], 4800).0[960..]);
        assert!((loud / soft - 127.0 / 64.0).abs() < 0.1, "{loud} {soft}");
    }

    #[test]
    fn a_released_note_falls_silent_after_its_release_time() {
        let ev = [MidiEvent::note_on(0, 0, 60, 100), MidiEvent::note_off(4800, 0, 60)];
        let (l, _) = render_events(&ev, 4800 + 7200 + 2400);
        assert!(rms(&l[2400..4800]) > 0.01);
        // 150 ms release is 7200 frames; the last 2400 frames are silent
        assert!(l[4800 + 7200 + 100..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn the_sustain_pedal_holds_a_released_note_until_it_is_lifted() {
        let ev = [
            MidiEvent::control(0, 0, 64, 127),
            MidiEvent::note_on(0, 0, 60, 100),
            MidiEvent::note_off(2400, 0, 60),
            MidiEvent::control(14_400, 0, 64, 0),
        ];
        let (l, _) = render_events(&ev, 14_400 + 9600);
        // still sounding 8000 frames after the note-off, silent well after the pedal came up
        assert!(rms(&l[10_000..12_000]) > 0.01);
        assert!(l[14_400 + 7200 + 200..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn pan_puts_the_sound_on_one_side() {
        let left = render_events(&[MidiEvent::control(0, 0, 10, 0), MidiEvent::note_on(0, 0, 60, 100)], 2400);
        assert!(rms(&left.0) > 0.005);
        assert!(left.1.iter().all(|s| *s == 0.0));
        let right = render_events(&[MidiEvent::control(0, 0, 10, 127), MidiEvent::note_on(0, 0, 60, 100)], 2400);
        assert!(rms(&right.1) > 0.005);
        assert!(rms(&right.0) < 1e-6);
        let centre = render_events(&[MidiEvent::note_on(0, 0, 60, 100)], 2400);
        assert!((rms(&centre.0) / rms(&centre.1) - 1.0).abs() < 0.05);
    }

    #[test]
    fn channel_volume_and_expression_scale_the_level() {
        let full = rms(&render_events(&[MidiEvent::control(0, 0, 7, 127), MidiEvent::note_on(0, 0, 60, 100)], 2400).0);
        let half = rms(&render_events(&[MidiEvent::control(0, 0, 7, 127), MidiEvent::control(0, 0, 11, 64), MidiEvent::note_on(0, 0, 60, 100)], 2400).0);
        assert!((half / full - 64.0 / 127.0).abs() < 0.02, "{full} {half}");
    }

    #[test]
    fn polyphony_adds_voices_and_all_notes_off_releases_them() {
        let mut h = host(512);
        let mut t = 0.0;
        let chord = [MidiEvent::note_on(0, 0, 60, 100), MidiEvent::note_on(0, 0, 64, 100), MidiEvent::note_on(0, 0, 67, 100)];
        run(&mut h, 512, &chord, 512, &mut t);
        // three voices are sounding; the synth is behind the host, so ask through a fresh instance as well
        let (one, _) = render_events(&[MidiEvent::note_on(0, 0, 60, 100)], 2400);
        let (three, _) = render_events(&chord, 2400);
        assert!(rms(&three[960..]) > rms(&one[960..]) * 1.3);
        let off = [MidiEvent::control(0, 0, 123, 0)];
        let (l, _) = run(&mut h, 9000, &off, 512, &mut t);
        assert!(l[8000..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn all_sound_off_is_immediate() {
        let ev = [MidiEvent::note_on(0, 0, 60, 100), MidiEvent::control(1000, 0, 120, 0)];
        let (l, _) = render_events(&ev, 2000);
        assert!(rms(&l[500..1000]) > 0.01);
        assert!(l[1000..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn drums_are_noise_that_decays_by_itself() {
        let (l, _) = render_events(&[MidiEvent::note_on(0, 9, 38, 120)], 48_000);
        assert!(rms(&l[..2400]) > 0.01);
        assert!(rms(&l[..2400]) > rms(&l[12_000..14_400]) * 3.0);
        assert!(l[40_000..].iter().all(|s| s.abs() < 1e-3));
        // a note-off does not cut a drum
        let (k, _) = render_events(&[MidiEvent::note_on(0, 9, 38, 120), MidiEvent::note_off(10, 9, 38)], 2400);
        assert!(rms(&k[100..]) > 0.01);
    }

    #[test]
    fn more_notes_than_voices_steal_without_trouble() {
        let events: Vec<MidiEvent> = (0..(VOICES as u8 + 10)).map(|k| MidiEvent::note_on(k as u32, 0, 36 + k, 100)).collect();
        let (l, r) = render_events(&events, 512);
        assert!(l.iter().chain(r.iter()).all(|s| s.is_finite() && s.abs() < 4.0));
        let mut s = GmSynth::new();
        s.initialize(&RenderConfig { sample_rate: RATE, max_frames: 64, input_channels: 0, output_channels: 2 }).unwrap();
        for k in 0..(VOICES as u8 + 10) {
            s.handle(&MidiEvent::note_on(0, 0, 36 + k, 100));
        }
        assert_eq!(s.active_voices(), VOICES);
    }

    #[test]
    fn the_level_parameter_scales_the_output_and_zero_is_silence() {
        let mut h = host(512);
        h.unit().params().set_ramped(SYNTH_LEVEL, 0.0, 0).unwrap();
        let mut t = 0.0;
        let (l, r) = run(&mut h, 1024, &[MidiEvent::note_on(0, 0, 60, 100)], 512, &mut t);
        assert!(l.iter().chain(r.iter()).all(|s| *s == 0.0));
    }

    #[test]
    fn mono_output_gets_the_sum_of_both_sides() {
        let mut h = UnitHost::new(Box::new(GmSynth::new()));
        h.initialize(RenderConfig { sample_rate: RATE, max_frames: 512, input_channels: 0, output_channels: 1 }).unwrap();
        let mut o = vec![0.0f32; 512];
        h.render(&[], &mut [o.as_mut_slice()], 512, 0.0, &[MidiEvent::note_on(0, 0, 60, 100)]).unwrap();
        assert!(rms(&o[100..]) > 0.005);
    }

    #[test]
    fn program_changes_pick_a_timbre_with_more_harmonics() {
        // same note, same level target: the bright timbre has a different waveform from the soft one
        let soft = render_events(&[MidiEvent::note_on(0, 0, 60, 100)], 2400).0;
        let bright = render_events(&[MidiEvent { offset: 0, status: 0xC0, data1: 80, data2: 0 }, MidiEvent::note_on(0, 0, 60, 100)], 2400).0;
        let diff: f32 = soft[960..].iter().zip(bright[960..].iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "{diff}");
    }

    #[test]
    fn tail_and_info() {
        let s = GmSynth::new();
        assert_eq!(s.info().kind, UnitKind::Instrument);
        assert!(s.info().accepts_midi);
        let mut s = s;
        s.initialize(&RenderConfig { sample_rate: RATE, max_frames: 64, input_channels: 0, output_channels: 2 }).unwrap();
        assert_eq!(s.tail_frames(), 7200);
    }
}
