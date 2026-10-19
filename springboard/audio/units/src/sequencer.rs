//! Sample-accurate MIDI sequencing inside the render callback.
//!
//! A `SequencerUnit` wraps an instrument unit and a `Sequence` (timed channel messages). On every block it works out
//! which messages fall inside the block from its own position, gives each the exact frame offset it lands on, merges
//! any live MIDI the host sent, and renders the instrument with the result. Because the timeline lives on the render
//! thread, a note sounds on the very frame its time maps to, whatever the block size, with no control-thread jitter.
//!
//! The control side holds a `SequencerHandle` (atomics only, any thread): play, pause, stop, seek, loop region, rate
//! and the position. A pause, a seek, a loop wrap and the end of the sequence all release the notes still held, so
//! nothing hangs. Rates scale the timeline (2.0 plays twice as fast).
//!
//! `render` allocates nothing and takes no lock: the event schedule and the two MIDI buffers are made in
//! `initialize`, and a buffer that is full drops the extra messages instead of growing.

use crate::midi::MidiEvent;
use crate::param::ParameterTree;
use crate::unit::{AudioUnit, Block, RenderConfig, UnitError, UnitInfo};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Most messages one block can carry (sequence, released notes and live MIDI together); more are dropped.
pub const BLOCK_MIDI_CAPACITY: usize = 4096;
/// Slowest and fastest rate.
pub const MIN_RATE: f32 = 0.25;
pub const MAX_RATE: f32 = 4.0;

/// One channel message at a time in seconds from the start of the sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeqEvent {
    pub seconds: f64,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

impl SeqEvent {
    pub fn note_on(seconds: f64, channel: u8, key: u8, velocity: u8) -> SeqEvent {
        SeqEvent { seconds, status: 0x90 | (channel & 15), data1: key & 127, data2: velocity & 127 }
    }

    pub fn note_off(seconds: f64, channel: u8, key: u8) -> SeqEvent {
        SeqEvent { seconds, status: 0x80 | (channel & 15), data1: key & 127, data2: 0 }
    }

    fn is_note_on(&self) -> bool {
        self.status & 0xF0 == 0x90 && self.data2 != 0
    }

    fn is_note_off(&self) -> bool {
        self.status & 0xF0 == 0x80 || (self.status & 0xF0 == 0x90 && self.data2 == 0)
    }
}

/// Timed channel messages in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    events: Vec<SeqEvent>,
}

impl Sequence {
    /// Sorts by time (messages with the same time keep their order). Negative and non-finite times are dropped.
    pub fn new(mut events: Vec<SeqEvent>) -> Sequence {
        events.retain(|e| e.seconds.is_finite() && e.seconds >= 0.0);
        events.sort_by(|a, b| a.seconds.partial_cmp(&b.seconds).unwrap_or(std::cmp::Ordering::Equal));
        Sequence { events }
    }

    /// Converts messages timed in ticks with a tempo map: `tempos` are (tick, microseconds per quarter note), and the
    /// tempo is 500000 (120 beats a minute) until the first one. `ppq` is ticks per quarter note.
    pub fn from_ticks(ppq: u32, tempos: &[(u64, u32)], events: &[(u64, [u8; 3])]) -> Sequence {
        let ppq = ppq.max(1) as f64;
        let mut map: Vec<(u64, f64, u32)> = vec![(0, 0.0, 500_000)];
        let mut sorted: Vec<(u64, u32)> = tempos.iter().copied().filter(|t| t.1 > 0).collect();
        sorted.sort_by_key(|t| t.0);
        for (tick, us) in sorted {
            let (t0, s0, us0) = *map.last().expect("the map starts with one segment");
            let seconds = s0 + (tick - t0) as f64 * (us0 as f64 / 1e6) / ppq;
            if tick == t0 {
                map.pop();
                map.push((tick, s0, us));
            } else {
                map.push((tick, seconds, us));
            }
        }
        let out = events
            .iter()
            .map(|(tick, b)| {
                let i = map.partition_point(|m| m.0 <= *tick).saturating_sub(1);
                let (t0, s0, us) = map[i];
                let seconds = s0 + (tick - t0) as f64 * (us as f64 / 1e6) / ppq;
                SeqEvent { seconds, status: b[0], data1: b[1] & 127, data2: b[2] & 127 }
            })
            .collect();
        Sequence::new(out)
    }

    pub fn events(&self) -> &[SeqEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The time of the last message (0 for an empty sequence).
    pub fn length_seconds(&self) -> f64 {
        self.events.last().map_or(0.0, |e| e.seconds)
    }
}

const NO_SEEK: u64 = u64::MAX;

struct Control {
    playing: AtomicBool,
    finished: AtomicBool,
    /// Seconds as f64 bits, or `NO_SEEK`.
    seek: AtomicU64,
    loop_on: AtomicBool,
    loop_start: AtomicU64,
    loop_end: AtomicU64,
    rate: AtomicU32,
    position: AtomicU64,
}

/// The control side of a `SequencerUnit`: cheap to clone, any thread, wait-free.
#[derive(Clone)]
pub struct SequencerHandle(Arc<Control>);

impl SequencerHandle {
    /// Starts or resumes. After the end of the sequence it starts again from the beginning.
    pub fn play(&self) {
        if self.0.finished.load(Ordering::Acquire) {
            self.seek(0.0);
        }
        self.0.playing.store(true, Ordering::Release);
    }

    /// Stops where it is; held notes are released and `play` resumes from here.
    pub fn pause(&self) {
        self.0.playing.store(false, Ordering::Release);
    }

    /// Pauses and returns to the start.
    pub fn stop(&self) {
        self.pause();
        self.seek(0.0);
    }

    /// Moves to `seconds` (negative and non-finite values go to 0). Held notes are released.
    pub fn seek(&self, seconds: f64) {
        let s = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };
        self.0.finished.store(false, Ordering::Release);
        self.0.seek.store(s.to_bits(), Ordering::Release);
    }

    /// Loops between two times (the end must be after the start), or turns looping off with `None`.
    pub fn set_loop(&self, region: Option<(f64, f64)>) {
        match region {
            Some((a, b)) if a.is_finite() && b.is_finite() && a >= 0.0 && b > a => {
                self.0.loop_start.store(a.to_bits(), Ordering::Relaxed);
                self.0.loop_end.store(b.to_bits(), Ordering::Relaxed);
                self.0.loop_on.store(true, Ordering::Release);
            }
            _ => self.0.loop_on.store(false, Ordering::Release),
        }
    }

    /// Sets the rate, clamped to `MIN_RATE..=MAX_RATE`; a value that is not finite is ignored.
    pub fn set_rate(&self, rate: f32) {
        if rate.is_finite() {
            self.0.rate.store(rate.clamp(MIN_RATE, MAX_RATE).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn rate(&self) -> f32 {
        f32::from_bits(self.0.rate.load(Ordering::Relaxed))
    }

    /// Where the timeline is, in seconds, as of the last block.
    pub fn position_seconds(&self) -> f64 {
        f64::from_bits(self.0.position.load(Ordering::Acquire))
    }

    pub fn is_playing(&self) -> bool {
        self.0.playing.load(Ordering::Acquire)
    }

    /// True once the timeline ran off the end (not while looping); `play` starts again.
    pub fn is_finished(&self) -> bool {
        self.0.finished.load(Ordering::Acquire)
    }
}

/// An instrument unit driven by a sequence.
pub struct SequencerUnit {
    inner: Box<dyn AudioUnit>,
    info: UnitInfo,
    seq: Sequence,
    ctl: Arc<Control>,
    /// Message times in sample frames at the initialized rate.
    frames_at: Vec<f64>,
    rate_hz: f64,
    /// The timeline position in sample frames.
    pos: f64,
    next: usize,
    /// Held notes per channel, one bit per key.
    held: [u128; 16],
    was_playing: bool,
    scratch: Vec<MidiEvent>,
    merged: Vec<MidiEvent>,
}

impl SequencerUnit {
    /// Wraps `inner` (an instrument or generator) and returns the unit with its control handle. The unit accepts
    /// live MIDI too, merged with the sequence.
    pub fn new(inner: Box<dyn AudioUnit>, seq: Sequence) -> (SequencerUnit, SequencerHandle) {
        let ctl = Arc::new(Control {
            playing: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            seek: AtomicU64::new(NO_SEEK),
            loop_on: AtomicBool::new(false),
            loop_start: AtomicU64::new(0),
            loop_end: AtomicU64::new(0),
            rate: AtomicU32::new(1.0f32.to_bits()),
            position: AtomicU64::new(0),
        });
        let mut info = inner.info().clone();
        info.name = format!("Sequenced {}", info.name);
        info.accepts_midi = true;
        let unit = SequencerUnit {
            inner,
            info,
            seq,
            ctl: ctl.clone(),
            frames_at: Vec::new(),
            rate_hz: 0.0,
            pos: 0.0,
            next: 0,
            held: [0; 16],
            was_playing: false,
            scratch: Vec::new(),
            merged: Vec::new(),
        };
        (unit, SequencerHandle(ctl))
    }

    fn push(v: &mut Vec<MidiEvent>, e: MidiEvent) {
        if v.len() < v.capacity() {
            v.push(e);
        }
    }

    /// Releases every held note at `offset`.
    fn release_all(held: &mut [u128; 16], out: &mut Vec<MidiEvent>, offset: u32) {
        for (ch, bits) in held.iter_mut().enumerate() {
            let mut b = *bits;
            while b != 0 {
                let key = b.trailing_zeros() as u8;
                b &= b - 1;
                SequencerUnit::push(out, MidiEvent::note_off(offset, ch as u8, key));
            }
            *bits = 0;
        }
    }

    fn track(held: &mut [u128; 16], e: &SeqEvent) {
        let ch = (e.status & 15) as usize;
        let bit = 1u128 << (e.data1 & 127);
        if e.is_note_on() {
            held[ch] |= bit;
        } else if e.is_note_off() {
            held[ch] &= !bit;
        }
    }
}

impl AudioUnit for SequencerUnit {
    fn info(&self) -> &UnitInfo {
        &self.info
    }

    fn tree(&self) -> &ParameterTree {
        self.inner.tree()
    }

    fn tree_mut(&mut self) -> &mut ParameterTree {
        self.inner.tree_mut()
    }

    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        self.inner.initialize(cfg)?;
        self.rate_hz = cfg.sample_rate;
        self.frames_at = self.seq.events.iter().map(|e| e.seconds * cfg.sample_rate).collect();
        self.scratch = Vec::with_capacity(BLOCK_MIDI_CAPACITY);
        self.merged = Vec::with_capacity(BLOCK_MIDI_CAPACITY);
        self.pos = 0.0;
        self.next = 0;
        self.held = [0; 16];
        self.was_playing = false;
        Ok(())
    }

    fn uninitialize(&mut self) {
        self.inner.uninitialize();
        self.frames_at = Vec::new();
        self.scratch = Vec::new();
        self.merged = Vec::new();
    }

    /// Clears the instrument. The sequencer keeps its own timeline: a jump in the host's sample time is not a seek.
    fn reset(&mut self) {
        self.held = [0; 16];
        self.inner.reset();
    }

    fn latency_frames(&self) -> u32 {
        self.inner.latency_frames()
    }

    fn tail_frames(&self) -> u32 {
        self.inner.tail_frames()
    }

    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        self.scratch.clear();
        let want_play = self.ctl.playing.load(Ordering::Acquire);
        let seek = self.ctl.seek.swap(NO_SEEK, Ordering::AcqRel);
        if seek != NO_SEEK {
            SequencerUnit::release_all(&mut self.held, &mut self.scratch, 0);
            self.pos = f64::from_bits(seek) * self.rate_hz;
            let pos = self.pos;
            self.next = self.frames_at.partition_point(|f| *f < pos);
        }
        if self.was_playing && !want_play {
            SequencerUnit::release_all(&mut self.held, &mut self.scratch, 0);
        }
        self.was_playing = want_play;

        if want_play && n > 0 && self.rate_hz > 0.0 {
            let rate = (f32::from_bits(self.ctl.rate.load(Ordering::Relaxed)) as f64).clamp(MIN_RATE as f64, MAX_RATE as f64);
            let end_of_sequence = self.seq.length_seconds() * self.rate_hz + 1.0;
            let mut at = 0usize;
            let mut remaining = n;
            let mut guard = 0;
            while remaining > 0 && guard < 64 {
                guard += 1;
                let looping = self.ctl.loop_on.load(Ordering::Acquire);
                let (lo, hi) = (
                    f64::from_bits(self.ctl.loop_start.load(Ordering::Relaxed)) * self.rate_hz,
                    f64::from_bits(self.ctl.loop_end.load(Ordering::Relaxed)) * self.rate_hz,
                );
                let wraps = looping && hi > lo && self.pos < hi;
                let limit = if wraps { hi } else { end_of_sequence };
                let seg_end = (self.pos + remaining as f64 * rate).min(limit).max(self.pos);
                while self.next < self.frames_at.len() && self.frames_at[self.next] < seg_end {
                    let e = self.seq.events[self.next];
                    let into = ((self.frames_at[self.next] - self.pos).max(0.0) / rate) as usize;
                    let offset = (at + into).min(n - 1) as u32;
                    SequencerUnit::push(&mut self.scratch, MidiEvent { offset, status: e.status, data1: e.data1, data2: e.data2 });
                    SequencerUnit::track(&mut self.held, &e);
                    self.next += 1;
                }
                let used = (((seg_end - self.pos) / rate).round() as usize).min(remaining);
                at += used;
                remaining -= used;
                self.pos = seg_end;
                if self.pos >= limit {
                    let offset = at.min(n - 1) as u32;
                    SequencerUnit::release_all(&mut self.held, &mut self.scratch, offset);
                    if wraps {
                        self.pos = lo;
                        let pos = self.pos;
                        self.next = self.frames_at.partition_point(|f| *f < pos);
                    } else {
                        self.ctl.finished.store(true, Ordering::Release);
                        self.ctl.playing.store(false, Ordering::Release);
                        self.was_playing = false;
                        break;
                    }
                }
            }
        }
        self.ctl.position.store((self.pos / self.rate_hz.max(1.0)).to_bits(), Ordering::Release);

        // live MIDI from the host merges in by frame offset (live first when the offsets are equal)
        if !block.midi.is_empty() {
            self.merged.clear();
            let (mut i, mut j) = (0, 0);
            while i < block.midi.len() || j < self.scratch.len() {
                let take_live = j >= self.scratch.len() || (i < block.midi.len() && block.midi[i].offset <= self.scratch[j].offset);
                if take_live {
                    SequencerUnit::push(&mut self.merged, block.midi[i]);
                    i += 1;
                } else {
                    SequencerUnit::push(&mut self.merged, self.scratch[j]);
                    j += 1;
                }
            }
            std::mem::swap(&mut self.scratch, &mut self.merged);
        }

        let mut inner_block = Block { frames: n, sample_time: block.sample_time, inputs: block.inputs, outputs: &mut *block.outputs, midi: &self.scratch };
        self.inner.render(&mut inner_block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::UnitHost;

    /// A power of two, so every frame time below is an exact binary fraction of a second.
    const RATE: f64 = 8_192.0;

    fn fr(frames: u32) -> f64 {
        frames as f64 / RATE
    }

    /// An instrument that writes an impulse at each message: the key / 127 for a note-on, -1 for a note-off, and
    /// records how many messages each block carried.
    struct Marker {
        info: UnitInfo,
        tree: ParameterTree,
    }

    impl Marker {
        fn new() -> Marker {
            Marker {
                info: UnitInfo { name: "Marker".into(), kind: crate::unit::UnitKind::Instrument, accepts_midi: true, max_channels: 1 },
                tree: ParameterTree::new(vec![]).unwrap(),
            }
        }
    }

    impl AudioUnit for Marker {
        fn info(&self) -> &UnitInfo {
            &self.info
        }
        fn tree(&self) -> &ParameterTree {
            &self.tree
        }
        fn tree_mut(&mut self) -> &mut ParameterTree {
            &mut self.tree
        }
        fn initialize(&mut self, _: &RenderConfig) -> Result<(), UnitError> {
            Ok(())
        }
        fn reset(&mut self) {}
        fn render(&mut self, block: &mut Block<'_, '_>) {
            let n = block.frames;
            block.outputs[0][..n].iter_mut().for_each(|s| *s = 0.0);
            for e in block.midi {
                if e.is_note_on() {
                    block.outputs[0][e.offset as usize] = e.data1 as f32 / 127.0;
                } else if e.is_note_off() {
                    block.outputs[0][e.offset as usize] = -1.0;
                }
            }
        }
    }

    fn host(seq: Sequence, frames: u32) -> (UnitHost, SequencerHandle) {
        let (unit, handle) = SequencerUnit::new(Box::new(Marker::new()), seq);
        let mut h = UnitHost::new(Box::new(unit));
        h.initialize(RenderConfig { sample_rate: RATE, max_frames: frames, input_channels: 0, output_channels: 1 }).unwrap();
        (h, handle)
    }

    fn block(h: &mut UnitHost, frames: usize, at: f64, live: &[MidiEvent]) -> Vec<f32> {
        let mut o = vec![9.0f32; frames];
        h.render(&[], &mut [o.as_mut_slice()], frames, at, live).unwrap();
        o
    }

    fn impulses(o: &[f32]) -> Vec<(usize, f32)> {
        o.iter().enumerate().filter(|(_, s)| **s != 0.0).map(|(i, s)| (i, *s)).collect()
    }

    fn two_notes() -> Sequence {
        // on at frame 8, off at 20, on at 40, off at 48
        Sequence::new(vec![
            SeqEvent::note_on(fr(8), 0, 127, 100),
            SeqEvent::note_off(fr(20), 0, 127),
            SeqEvent::note_on(fr(40), 0, 127, 100),
            SeqEvent::note_off(fr(48), 0, 127),
        ])
    }

    #[test]
    fn messages_land_on_their_exact_frames_whatever_the_block_size() {
        for size in [16usize, 7, 64] {
            let (mut h, ctl) = host(two_notes(), 64);
            ctl.play();
            let mut got = Vec::new();
            let mut t = 0usize;
            while t < 64 {
                let o = block(&mut h, size.min(64 - t), t as f64, &[]);
                got.extend(impulses(&o).into_iter().map(|(i, s)| (t + i, s)));
                t += size;
            }
            assert_eq!(got, vec![(8, 1.0), (20, -1.0), (40, 1.0), (48, -1.0)], "block size {size}");
        }
    }

    #[test]
    fn nothing_plays_until_play_and_pause_releases_held_notes() {
        let (mut h, ctl) = host(two_notes(), 16);
        assert!(impulses(&block(&mut h, 16, 0.0, &[])).is_empty());
        assert!(!ctl.is_playing());
        ctl.play();
        let o = block(&mut h, 16, 16.0, &[]);
        assert_eq!(impulses(&o), vec![(8, 1.0)]);
        // paused with the note held: it is released at the start of the next block, and the timeline stays
        ctl.pause();
        let o = block(&mut h, 16, 32.0, &[]);
        assert_eq!(impulses(&o), vec![(0, -1.0)]);
        let at = ctl.position_seconds();
        assert!(at > 0.0);
        assert!(impulses(&block(&mut h, 16, 48.0, &[])).is_empty());
        assert_eq!(ctl.position_seconds(), at);
    }

    #[test]
    fn seek_releases_notes_and_skips_earlier_messages() {
        let (mut h, ctl) = host(two_notes(), 16);
        ctl.play();
        block(&mut h, 16, 0.0, &[]);
        block(&mut h, 8, 16.0, &[]);
        // the first note was on at 8 and off at 20, so nothing is held at frame 24; seek to frame 30
        ctl.seek(fr(30));
        let o = block(&mut h, 16, 24.0, &[]);
        // the block covers sequence frames 30..46: the note-on at 40 lands at offset 10, the off at 48 is later
        assert_eq!(impulses(&o), vec![(10, 1.0)]);
        assert!((ctl.position_seconds() - fr(46)).abs() < 1e-12);
    }

    #[test]
    fn a_seek_with_a_note_held_releases_it_first() {
        let (mut h, ctl) = host(two_notes(), 16);
        ctl.play();
        block(&mut h, 16, 0.0, &[]); // note on at 8, held
        ctl.seek(0.0);
        let o = block(&mut h, 16, 16.0, &[]);
        // release at 0, then the sequence restarts: its note-on at 8 again
        assert_eq!(impulses(&o), vec![(0, -1.0), (8, 1.0)]);
    }

    #[test]
    fn the_end_of_the_sequence_stops_releases_and_play_restarts() {
        let seq = Sequence::new(vec![SeqEvent::note_on(fr(8), 0, 100, 90)]);
        let (mut h, ctl) = host(seq, 16);
        ctl.play();
        let o = block(&mut h, 16, 0.0, &[]);
        // on at 8; the sequence ends one frame after its last message (frame 9), where the held note is released
        assert_eq!(impulses(&o), vec![(8, 100.0 / 127.0), (9, -1.0)]);
        assert!(ctl.is_finished() && !ctl.is_playing());
        assert!(impulses(&block(&mut h, 16, 16.0, &[])).is_empty());
        // play after the end starts again from the beginning
        ctl.play();
        assert!(!ctl.is_finished());
        let o = block(&mut h, 16, 32.0, &[]);
        assert_eq!(impulses(&o), vec![(8, 100.0 / 127.0), (9, -1.0)]);
    }

    #[test]
    fn a_loop_wraps_on_the_exact_frame_and_releases_held_notes() {
        // a note on at frame 8 with no off; loop over frames 0..24
        let seq = Sequence::new(vec![SeqEvent::note_on(fr(8), 0, 127, 100)]);
        let (mut h, ctl) = host(seq, 64);
        ctl.set_loop(Some((0.0, fr(24))));
        ctl.play();
        let o = block(&mut h, 64, 0.0, &[]);
        // on at 8, released at the wrap (24), on again at 32 (8 after the wrap), released at 48, on at 56
        assert_eq!(impulses(&o), vec![(8, 1.0), (24, -1.0), (32, 1.0), (48, -1.0), (56, 1.0)]);
        assert!(!ctl.is_finished());
        ctl.set_loop(None);
        block(&mut h, 64, 64.0, &[]);
        assert!(ctl.is_finished());
    }

    #[test]
    fn a_rate_of_two_plays_twice_as_fast() {
        let (mut h, ctl) = host(two_notes(), 32);
        ctl.set_rate(2.0);
        ctl.play();
        let o = block(&mut h, 32, 0.0, &[]);
        // on at 8 -> frame 4, off at 20 -> 10, on at 40 -> 20, off at 48 -> 24
        assert_eq!(impulses(&o), vec![(4, 1.0), (10, -1.0), (20, 1.0), (24, -1.0)]);
        assert_eq!(ctl.rate(), 2.0);
    }

    #[test]
    fn rate_is_clamped_and_junk_is_ignored() {
        let (_h, ctl) = host(two_notes(), 16);
        ctl.set_rate(100.0);
        assert_eq!(ctl.rate(), MAX_RATE);
        ctl.set_rate(0.0);
        assert_eq!(ctl.rate(), MIN_RATE);
        ctl.set_rate(f32::NAN);
        assert_eq!(ctl.rate(), MIN_RATE);
        ctl.set_loop(Some((2.0, 1.0)));
        ctl.seek(f64::NAN);
        ctl.seek(-3.0);
    }

    #[test]
    fn live_midi_merges_with_the_sequence_by_offset() {
        let (mut h, ctl) = host(two_notes(), 16);
        ctl.play();
        // live note-on at 2 and at 8 (the same frame as the sequence's note-on)
        let live = [MidiEvent::note_on(2, 0, 64, 100), MidiEvent::note_on(8, 0, 32, 100)];
        let o = block(&mut h, 16, 0.0, &live);
        assert_eq!(impulses(&o).len(), 2);
        assert_eq!(o[2], 64.0 / 127.0);
        // at frame 8 both write the impulse; the later one in the merged order (the sequence's) wins
        assert_eq!(o[8], 1.0);
    }

    #[test]
    fn tempo_maps_convert_ticks_to_seconds() {
        // 480 ticks per quarter: 120 bpm for the first beat, then 240 bpm
        let seq = Sequence::from_ticks(
            480,
            &[(480, 250_000)],
            &[(0, [0x90, 60, 100]), (480, [0x80, 60, 0]), (960, [0x90, 62, 100]), (1440, [0x80, 62, 0])],
        );
        let t: Vec<f64> = seq.events().iter().map(|e| e.seconds).collect();
        assert_eq!(t, vec![0.0, 0.5, 0.75, 1.0]);
        assert_eq!(seq.length_seconds(), 1.0);
        assert_eq!(seq.len(), 4);
    }

    #[test]
    fn a_tempo_at_tick_zero_replaces_the_default() {
        let seq = Sequence::from_ticks(96, &[(0, 1_000_000)], &[(96, [0x90, 60, 100])]);
        assert_eq!(seq.events()[0].seconds, 1.0);
    }

    #[test]
    fn sequences_sort_and_drop_bad_times() {
        let seq = Sequence::new(vec![
            SeqEvent::note_on(2.0, 0, 60, 1),
            SeqEvent::note_on(-1.0, 0, 61, 1),
            SeqEvent::note_on(f64::NAN, 0, 62, 1),
            SeqEvent::note_on(1.0, 0, 63, 1),
            SeqEvent::note_off(1.0, 0, 63),
        ]);
        let keys: Vec<(f64, u8, u8)> = seq.events().iter().map(|e| (e.seconds, e.status & 0xF0, e.data1)).collect();
        assert_eq!(keys, vec![(1.0, 0x90, 63), (1.0, 0x80, 63), (2.0, 0x90, 60)]);
        assert!(Sequence::new(vec![]).is_empty());
        assert_eq!(Sequence::new(vec![]).length_seconds(), 0.0);
    }

    #[test]
    fn the_unit_reports_the_instrument_underneath() {
        let (unit, _ctl) = SequencerUnit::new(Box::new(Marker::new()), two_notes());
        assert_eq!(unit.info().name, "Sequenced Marker");
        assert!(unit.info().accepts_midi);
        assert_eq!(unit.tree().specs().len(), 0);
        assert_eq!(unit.latency_frames(), 0);
    }

    #[test]
    fn a_burst_of_messages_beyond_the_capacity_is_dropped_not_allocated() {
        let events: Vec<SeqEvent> = (0..(BLOCK_MIDI_CAPACITY + 500)).map(|i| SeqEvent::note_on(0.0, (i % 16) as u8, (i / 16 % 128) as u8, 90)).collect();
        let (mut h, ctl) = host(Sequence::new(events), 16);
        ctl.play();
        // renders without panicking or growing the buffers
        let o = block(&mut h, 16, 0.0, &[]);
        assert_eq!(o.len(), 16);
    }
}
