//! A small pure-Rust synthesizer for Standard MIDI Files. It turns the channel messages that `smf::Smf` reads into
//! 16-bit stereo PCM with no sound bank and no device. A tone voice is additive: a few harmonics of a sine, chosen by
//! the General MIDI program number (0 to 31 soft, 32 to 55 warm, the rest bright), shaped by a linear ADSR envelope.
//! Channel 10 (index 9) plays General MIDI drums as short noise bursts that decay on their own. Pitch bend (plus or
//! minus two semitones), channel volume (CC 7), expression (CC 11) and pan (CC 10) are followed. Not modelled: the
//! sustain pedal, reverb, chorus and recorded instrument samples, so the sound is simpler than a sound bank's.
//!
//! `Renderer` produces the audio in blocks, so a player can stream a long file. `render` makes the whole file at once.

use crate::smf::{Message, Smf};
use std::f64::consts::{FRAC_PI_2, TAU};

/// The sample rate a player asks for by default (the audio output rate).
pub const DEFAULT_RATE: u32 = 48_000;
/// Output channels: interleaved left, right.
pub const CHANNELS: usize = 2;
/// Most voices sounding at once. A note past this takes the oldest voice.
pub const MAX_VOICES: usize = 32;

const DRUM_CHANNEL: usize = 9;
const BEND_RANGE_SEMITONES: f64 = 2.0;
const RELEASE_SECONDS: f64 = 0.2;
const MASTER_GAIN: f64 = 0.8;
const DRUM_ATTACK_SECONDS: f64 = 0.002;
const MIN_SHAPE_SECONDS: f64 = 0.001;

/// The tone family of a General MIDI program: its harmonic weights and its ADSR envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timbre {
    Soft,
    Warm,
    Bright,
}

impl Timbre {
    /// The family of a General MIDI program number (0 to 127).
    pub fn for_program(program: u8) -> Timbre {
        match program {
            0..=31 => Timbre::Soft,
            32..=55 => Timbre::Warm,
            _ => Timbre::Bright,
        }
    }

    /// Relative weights of the fundamental and its harmonics.
    fn harmonics(self) -> &'static [f64] {
        match self {
            Timbre::Soft => &[1.0, 0.5, 0.25],
            Timbre::Warm => &[1.0, 0.3, 0.15, 0.05],
            Timbre::Bright => &[1.0, 0.7, 0.5, 0.3, 0.2],
        }
    }

    /// Attack, decay (both in seconds) and the sustain level (0 to 1).
    fn envelope(self) -> (f64, f64, f64) {
        match self {
            Timbre::Soft => (0.01, 0.10, 0.7),
            Timbre::Warm => (0.08, 0.30, 0.8),
            Timbre::Bright => (0.02, 0.15, 0.6),
        }
    }
}

/// How long a General MIDI drum sound rings, by key: cymbals longest, closed hi-hats shortest.
fn drum_decay(key: u8) -> f64 {
    match key {
        35 | 36 => 0.25,
        42 | 44 | 46 => 0.05,
        49 | 51 | 52 | 53 | 55 | 57 | 59 => 0.6,
        _ => 0.15,
    }
}

/// The frequency of a key in hertz (key 69 is A4 at 440 Hz) with a bend in semitones.
pub fn key_hz(key: u8, bend: f64) -> f64 {
    440.0 * ((f64::from(key) - 69.0 + bend) / 12.0).exp2()
}

fn idx(channel: u8) -> usize {
    (channel & 0x0F) as usize
}

fn to_i16(x: f64) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[derive(Clone, Copy, Debug)]
struct Channel {
    volume: f64,
    expression: f64,
    pan_l: f64,
    pan_r: f64,
    bend: f64,
    program: u8,
}

impl Channel {
    /// CC 10: 0 is hard left, 64 is centre, 127 is hard right. Constant power keeps the loudness even.
    fn set_pan(&mut self, cc: u8) {
        let p = f64::from(cc.min(127)) / 127.0 * FRAC_PI_2;
        self.pan_l = p.cos();
        self.pan_r = p.sin();
    }
}

impl Default for Channel {
    fn default() -> Self {
        let mut c = Channel { volume: 100.0 / 127.0, expression: 1.0, pan_l: 0.0, pan_r: 0.0, bend: 0.0, program: 0 };
        c.set_pan(64);
        c
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone, Debug)]
struct Voice {
    channel: usize,
    key: u8,
    gain: f64,
    rate: f64,
    phase: f64,
    phase_inc: f64,
    harmonics: &'static [f64],
    norm: f64,
    noise: bool,
    noise_state: u32,
    stage: Stage,
    level: f64,
    attack_inc: f64,
    decay_inc: f64,
    sustain: f64,
    release_inc: f64,
}

impl Voice {
    fn new(channel: usize, key: u8, velocity: u8, ch: &Channel, rate: f64) -> Voice {
        let drum = channel == DRUM_CHANNEL;
        let empty: &'static [f64] = &[];
        let (harmonics, attack, decay, sustain) = if drum {
            (empty, DRUM_ATTACK_SECONDS, drum_decay(key), 0.0)
        } else {
            let t = Timbre::for_program(ch.program);
            let (a, d, s) = t.envelope();
            (t.harmonics(), a, d, s)
        };
        let norm = if drum { 1.0 } else { harmonics.iter().sum::<f64>() };
        let freq = key_hz(key, ch.bend);
        Voice {
            channel,
            key,
            gain: f64::from(velocity.min(127)) / 127.0,
            rate,
            phase: 0.0,
            phase_inc: freq / rate,
            harmonics,
            norm,
            noise: drum,
            noise_state: 0x2545_F491 ^ (u32::from(key)),
            stage: Stage::Attack,
            level: 0.0,
            attack_inc: 1.0 / (attack.max(MIN_SHAPE_SECONDS) * rate),
            decay_inc: (1.0 - sustain) / (decay.max(MIN_SHAPE_SECONDS) * rate),
            sustain,
            release_inc: 0.0,
        }
    }

    /// The raw wave for this instant, before the envelope and the channel gains.
    fn sample(&mut self) -> f64 {
        if self.noise {
            self.noise_state = self.noise_state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            return (self.noise_state >> 8) as f64 / 8_388_608.0 - 1.0;
        }
        let mut s = 0.0;
        for (k, h) in self.harmonics.iter().enumerate() {
            s += h * (TAU * (k as f64 + 1.0) * self.phase).sin();
        }
        s / self.norm
    }

    /// Moves the phase and the envelope on by one sample.
    fn advance(&mut self) {
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        match self.stage {
            Stage::Attack => {
                self.level += self.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= self.decay_inc;
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                if self.level <= 0.0 {
                    self.stage = Stage::Done;
                }
            }
            Stage::Release => {
                self.level -= self.release_inc;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => {}
        }
    }

    /// Note off: a tone fades out over the release time. A drum rings on, as the hardware does.
    fn release(&mut self) {
        if self.noise || matches!(self.stage, Stage::Release | Stage::Done) {
            return;
        }
        self.stage = Stage::Release;
        self.release_inc = self.level / (RELEASE_SECONDS * self.rate);
    }

    fn set_pitch(&mut self, bend: f64) {
        self.phase_inc = key_hz(self.key, bend) / self.rate;
    }

    fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }
}

/// Renders a parsed file to PCM a block at a time.
pub struct Renderer {
    rate: u32,
    /// (frame at which it sounds, message), in time order
    events: Vec<(u64, Message)>,
    next: usize,
    frame: u64,
    voices: Vec<Voice>,
    channels: [Channel; 16],
}

impl Renderer {
    /// A renderer at `rate` frames per second (clamped to 8 000 to 192 000).
    pub fn new(smf: &Smf, rate: u32) -> Renderer {
        let rate = rate.clamp(8_000, 192_000);
        let events = smf.events().iter().map(|e| ((e.seconds.max(0.0) * f64::from(rate)).round() as u64, e.message)).collect();
        Renderer { rate, events, next: 0, frame: 0, voices: Vec::with_capacity(MAX_VOICES), channels: [Channel::default(); 16] }
    }

    /// The frame rate of the output.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// True once every message has played and every voice has faded out.
    pub fn is_finished(&self) -> bool {
        self.next >= self.events.len() && self.voices.is_empty()
    }

    /// Writes up to `frames` stereo frames into `out` (replaced, interleaved left and right) and returns how many
    /// frames it wrote. Zero means the file is finished.
    pub fn next_block(&mut self, out: &mut Vec<i16>, frames: usize) -> usize {
        out.clear();
        let mut produced = 0;
        while produced < frames {
            self.apply_due();
            if self.is_finished() {
                break;
            }
            let (l, r) = self.mix();
            out.push(to_i16(l));
            out.push(to_i16(r));
            self.frame += 1;
            produced += 1;
        }
        produced
    }

    /// Moves to `seconds` from the start. Voices restart silent and the controls (program, volume, pan, bend) are
    /// replayed up to that point, so the sound after a seek matches a render that played through.
    pub fn seek(&mut self, seconds: f64) {
        let target = (seconds.max(0.0) * f64::from(self.rate)).round() as u64;
        self.voices.clear();
        self.channels = [Channel::default(); 16];
        let idx = self.events.partition_point(|&(due, _)| due < target);
        let controls: Vec<Message> = self.events[..idx].iter().map(|&(_, m)| m).filter(|m| is_control(m)).collect();
        for m in controls {
            self.apply(m);
        }
        self.next = idx;
        self.frame = target;
    }

    fn apply_due(&mut self) {
        while let Some(&(due, m)) = self.events.get(self.next) {
            if due > self.frame {
                break;
            }
            self.next += 1;
            self.apply(m);
        }
    }

    fn apply(&mut self, m: Message) {
        match m {
            Message::NoteOn { channel, key, velocity } => self.note_on(channel, key, velocity),
            Message::NoteOff { channel, key, .. } => self.note_off(channel, key),
            Message::ProgramChange { channel, program } => self.channels[idx(channel)].program = program,
            Message::ControlChange { channel, controller, value } => self.control_change(channel, controller, value),
            Message::PitchBend { channel, value } => self.pitch_bend(channel, value),
            Message::PolyPressure { .. } | Message::ChannelPressure { .. } => {}
        }
    }

    fn note_on(&mut self, channel: u8, key: u8, velocity: u8) {
        if velocity == 0 {
            return self.note_off(channel, key);
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        let c = idx(channel);
        let voice = Voice::new(c, key, velocity, &self.channels[c], f64::from(self.rate));
        self.voices.push(voice);
    }

    fn note_off(&mut self, channel: u8, key: u8) {
        let c = idx(channel);
        for v in self.voices.iter_mut().filter(|v| v.channel == c && v.key == key) {
            v.release();
        }
    }

    fn control_change(&mut self, channel: u8, controller: u8, value: u8) {
        let c = idx(channel);
        match controller {
            7 => self.channels[c].volume = f64::from(value.min(127)) / 127.0,
            10 => self.channels[c].set_pan(value),
            11 => self.channels[c].expression = f64::from(value.min(127)) / 127.0,
            120 => self.voices.retain(|v| v.channel != c),
            123 => {
                for v in self.voices.iter_mut().filter(|v| v.channel == c) {
                    v.release();
                }
            }
            _ => {}
        }
    }

    fn pitch_bend(&mut self, channel: u8, value: u16) {
        let c = idx(channel);
        let bend = (f64::from(value.min(16_383)) - 8192.0) / 8192.0 * BEND_RANGE_SEMITONES;
        self.channels[c].bend = bend;
        for v in self.voices.iter_mut().filter(|v| v.channel == c && !v.noise) {
            v.set_pitch(bend);
        }
    }

    /// One stereo sample from every voice, then the finished voices are dropped.
    fn mix(&mut self) -> (f64, f64) {
        let mut l = 0.0;
        let mut r = 0.0;
        let channels = &self.channels;
        for v in self.voices.iter_mut() {
            let c = &channels[v.channel];
            let s = v.sample() * v.level * v.gain * c.volume * c.expression;
            l += s * c.pan_l;
            r += s * c.pan_r;
            v.advance();
        }
        self.voices.retain(|v| !v.is_done());
        (l * MASTER_GAIN, r * MASTER_GAIN)
    }
}

fn is_control(m: &Message) -> bool {
    matches!(m, Message::ProgramChange { .. } | Message::ControlChange { .. } | Message::PitchBend { .. })
}

/// The whole file as interleaved stereo S16 at `rate`, from the start until the last voice has faded out.
pub fn render(smf: &Smf, rate: u32) -> Vec<i16> {
    let mut r = Renderer::new(smf, rate);
    let mut all = Vec::new();
    let mut block = Vec::new();
    while r.next_block(&mut block, 4096) > 0 {
        all.extend_from_slice(&block);
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A format 0 file with one track of the given bytes, at 96 ticks per quarter note (120 beats a minute: one
    /// quarter is half a second).
    fn smf(track: &[u8]) -> Smf {
        let mut b = b"MThd".to_vec();
        b.extend(6u32.to_be_bytes());
        b.extend(0u16.to_be_bytes());
        b.extend(1u16.to_be_bytes());
        b.extend(96u16.to_be_bytes());
        b.extend(b"MTrk");
        b.extend((track.len() as u32).to_be_bytes());
        b.extend(track);
        Smf::parse(&b).expect("a valid file")
    }

    fn drain(r: &mut Renderer) -> Vec<i16> {
        let mut all = Vec::new();
        let mut block = Vec::new();
        while r.next_block(&mut block, 512) > 0 {
            all.extend_from_slice(&block);
        }
        all
    }

    /// Note on key 69 at 0 s, note off at 0.5 s (96 ticks), end of track.
    const A4_HALF_SECOND: &[u8] = &[0x00, 0x90, 69, 100, 0x60, 0x80, 69, 0, 0x00, 0xFF, 0x2F, 0x00];

    #[test]
    fn a_note_sounds_then_fades_out_at_the_release() {
        // At 8 000 frames a second: note off at frame 4 000, then 0.2 s (1 600 frames) of release.
        let out = render(&smf(A4_HALF_SECOND), 8_000);
        let frames = out.len() / 2;
        assert!((5_590..=5_610).contains(&frames), "frames {frames}");
        // Frames 1 000 to 3 000 are at sustain: gain 0.787 * 0.787 * 0.70 * 0.7 * 0.8 is about 0.24, so the peak is
        // near 7 900 and certainly above 3 000.
        let peak = out[2_000..6_000].iter().map(|s| s.abs()).max().unwrap_or(0);
        assert!(peak > 3_000, "peak {peak}");
        assert!(out[out.len() - 2].abs() < 200, "the tail is silent");
    }

    #[test]
    fn a_drum_is_noise_that_rings_out_on_its_own() {
        // Channel 10 (index 9), key 36 (bass drum): 0.25 s of decay after a 2 ms attack, no note off needed.
        let track = [0x00, 0x99, 36, 100, 0x60, 0xFF, 0x2F, 0x00];
        let out = render(&smf(&track), 8_000);
        let frames = out.len() / 2;
        assert!((1_990..=2_030).contains(&frames), "frames {frames}");
        let peak = out.iter().map(|s| s.abs()).max().unwrap_or(0);
        assert!(peak > 1_000, "noise peak {peak}");
    }

    #[test]
    fn channel_volume_zero_is_silent() {
        // CC 7 (volume) = 0 on channel 0 before the note.
        let track = [0x00, 0xB0, 7, 0, 0x00, 0x90, 69, 100, 0x60, 0x80, 69, 0, 0x00, 0xFF, 0x2F, 0x00];
        let out = render(&smf(&track), 8_000);
        assert!(!out.is_empty());
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn a_file_without_notes_renders_nothing() {
        assert!(render(&smf(&[0x00, 0xFF, 0x2F, 0x00]), 8_000).is_empty());
    }

    #[test]
    fn key_frequencies_and_bend() {
        assert_eq!(key_hz(69, 0.0), 440.0);
        assert!((key_hz(81, 0.0) - 880.0).abs() < 1e-9);
        // +2 semitones from A4 is 440 * 2^(2/12) = 493.8833 Hz.
        assert!((key_hz(69, 2.0) - 493.8833).abs() < 0.001);
    }

    #[test]
    fn programs_map_to_their_families() {
        assert_eq!(Timbre::for_program(0), Timbre::Soft);
        assert_eq!(Timbre::for_program(40), Timbre::Warm);
        assert_eq!(Timbre::for_program(80), Timbre::Bright);
    }

    #[test]
    fn a_seek_back_to_the_start_renders_the_same_samples() {
        let s = smf(A4_HALF_SECOND);
        let mut r = Renderer::new(&s, 8_000);
        let first = drain(&mut r);
        assert!(r.is_finished());
        r.seek(0.0);
        assert_eq!(drain(&mut r), first);
        r.seek(0.25);
        let late = drain(&mut r);
        assert!(late.len() < first.len(), "a seek to 0.25 s leaves less to play");
    }
}
