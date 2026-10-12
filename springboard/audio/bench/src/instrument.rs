//! The virtual MIDI instrument for millisecond benchmarks.
//!
//! - `BenchInstrument` is an audio unit: for each note-on in the block it writes one full-scale sample (1.0, on every
//!   output channel) at the event's frame offset and nothing else.
//! - `BenchMidi` registers the "Bench Source" and "Bench Instrument" endpoints with the MIDI hub. The source sends
//!   note-ons stamped with host time (`CLOCK_MONOTONIC_RAW` ns); the instrument's destination queues them.
//! - `schedule::EventScheduler` turns the stamped events into per-cycle frame offsets (later events stay queued, late
//!   ones play at offset 0 and count as late).

use crate::schedule::EventScheduler;
use bifrost_audio_units::param::ParameterTree;
use bifrost_audio_units::unit::check_config;
use bifrost_audio_units::{AudioUnit, Block, MidiEvent, RenderConfig, UnitError, UnitInfo, UnitKind};
use bifrost_midi::endpoints::ObjectId;
use bifrost_midi::events::MidiEvent as WireEvent;
use bifrost_midi::hub::Hub;
use bifrost_midi::ump::{from_midi1, to_midi1};

/// The name of the virtual MIDI source.
pub const SOURCE_NAME: &str = "Bench Source";
/// The name of the virtual MIDI destination (and of the audio unit).
pub const INSTRUMENT_NAME: &str = "Bench Instrument";

/// Writes a single full-scale impulse per note-on, at the note's sample offset inside the block.
pub struct BenchInstrument {
    info: UnitInfo,
    tree: ParameterTree,
    ready: bool,
    impulses: u64,
    out_of_block: u64,
}

impl BenchInstrument {
    pub fn new() -> BenchInstrument {
        BenchInstrument {
            info: UnitInfo { name: INSTRUMENT_NAME.into(), kind: UnitKind::Instrument, accepts_midi: true, max_channels: 8 },
            tree: ParameterTree::new(Vec::new()).expect("an empty tree is valid"),
            ready: false,
            impulses: 0,
            out_of_block: 0,
        }
    }

    /// Note-ons rendered as impulses so far.
    pub fn impulses(&self) -> u64 {
        self.impulses
    }

    /// Note-ons whose offset was beyond the block (they are not rendered).
    pub fn out_of_block(&self) -> u64 {
        self.out_of_block
    }
}

impl Default for BenchInstrument {
    fn default() -> BenchInstrument {
        BenchInstrument::new()
    }
}

impl AudioUnit for BenchInstrument {
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
        self.ready = true;
        Ok(())
    }
    fn uninitialize(&mut self) {
        self.ready = false;
    }
    fn reset(&mut self) {}
    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        for out in block.outputs.iter_mut() {
            out[..n].iter_mut().for_each(|s| *s = 0.0);
        }
        if !self.ready {
            return;
        }
        for ev in block.midi.iter().filter(|e| e.is_note_on()) {
            let at = ev.offset as usize;
            if at >= n {
                self.out_of_block += 1;
                continue;
            }
            for out in block.outputs.iter_mut() {
                out[at] = 1.0;
            }
            self.impulses += 1;
        }
    }
}

/// Renders one cycle of `unit` into `planes` (one `Vec` per output channel, at least `frames` long).
pub fn render_cycle(unit: &mut dyn AudioUnit, frames: usize, sample_time: f64, midi: &[MidiEvent], planes: &mut [Vec<f32>]) {
    let mut outs: Vec<&mut [f32]> = planes.iter_mut().map(|p| &mut p[..frames]).collect();
    let mut block = Block { frames, sample_time, inputs: &[], outputs: &mut outs, midi };
    unit.render(&mut block);
}

/// A note-on or note-off of the wire format as a unit event (offset 0); other messages are ignored.
pub fn to_unit_event(e: WireEvent) -> Option<MidiEvent> {
    match e {
        WireEvent::NoteOn { channel, key, velocity } => Some(MidiEvent::note_on(0, channel, key, velocity)),
        WireEvent::NoteOff { channel, key, .. } => Some(MidiEvent::note_off(0, channel, key)),
        _ => None,
    }
}

/// The two virtual endpoints on a MIDI hub.
pub struct BenchMidi {
    pub hub: Hub,
    /// "Bench Source".
    pub source: ObjectId,
    /// "Bench Instrument".
    pub destination: ObjectId,
}

impl BenchMidi {
    pub fn new() -> BenchMidi {
        let mut hub = Hub::new();
        let (_src_client, source, _unused_dst) = hub.create_app(SOURCE_NAME);
        let (_dst_client, _unused_src, destination) = hub.create_app(INSTRUMENT_NAME);
        BenchMidi { hub, source, destination }
    }

    /// The source sends a note-on (channel 0, velocity 100) stamped `host_ns` to the instrument.
    /// `false` when the destination is gone.
    pub fn send_note_on(&mut self, host_ns: u64, key: u8) -> bool {
        let ump = from_midi1(0, WireEvent::NoteOn { channel: 0, key: key & 127, velocity: 100 });
        self.hub.send_to_destination(self.source, self.destination, host_ns, ump)
    }

    /// Moves the destination's events stamped at or before `until_ns` into the scheduler. Returns how many.
    pub fn drain_into(&mut self, until_ns: u64, sched: &mut EventScheduler) -> usize {
        let evs = self.take_events(until_ns);
        for (ns, ev) in &evs {
            sched.push(*ns, *ev);
        }
        evs.len()
    }

    /// Takes the destination's note events stamped at or before `until_ns` as `(host ns, unit event)`, in order.
    pub fn take_events(&mut self, until_ns: u64) -> Vec<(u64, MidiEvent)> {
        self.hub
            .take_destination(self.destination, until_ns)
            .into_iter()
            .filter_map(|t| to_midi1(&t.packet).and_then(to_unit_event).map(|ev| (t.host_ns, ev)))
            .collect()
    }
}

impl Default for BenchMidi {
    fn default() -> BenchMidi {
        BenchMidi::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::Timeline;
    use bifrost_midi::endpoints::Direction;

    fn cfg() -> RenderConfig {
        RenderConfig { sample_rate: 50_000.0, max_frames: 64, input_channels: 0, output_channels: 2 }
    }

    fn planes(frames: usize) -> Vec<Vec<f32>> {
        vec![vec![0.5; frames], vec![0.5; frames]]
    }

    #[test]
    fn impulse_at_the_exact_offset_on_every_channel() {
        let mut u = BenchInstrument::new();
        u.initialize(&cfg()).unwrap();
        let mut p = planes(8);
        render_cycle(&mut u, 8, 0.0, &[MidiEvent::note_on(3, 0, 60, 100)], &mut p);
        for ch in &p {
            let expected = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
            assert_eq!(ch.as_slice(), &expected);
        }
        assert_eq!(u.impulses(), 1);
    }

    #[test]
    fn only_note_ons_in_the_block_count() {
        let mut u = BenchInstrument::new();
        u.initialize(&cfg()).unwrap();
        let mut p = planes(8);
        let evs = [
            MidiEvent::note_off(1, 0, 60),
            MidiEvent::note_on(2, 0, 60, 0), // velocity 0 is a note-off
            MidiEvent::note_on(5, 1, 61, 90),
            MidiEvent::note_on(8, 0, 62, 90), // past the end of the block
            MidiEvent::control(0, 0, 7, 100),
        ];
        render_cycle(&mut u, 8, 0.0, &evs, &mut p);
        assert_eq!(p[0], vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!((u.impulses(), u.out_of_block()), (1, 1));
        // The next block starts silent again.
        render_cycle(&mut u, 8, 8.0, &[], &mut p);
        assert!(p[0].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn uninitialized_unit_is_silent() {
        let mut u = BenchInstrument::new();
        let mut p = planes(4);
        render_cycle(&mut u, 4, 0.0, &[MidiEvent::note_on(0, 0, 60, 100)], &mut p);
        assert!(p[0].iter().all(|s| *s == 0.0));
        assert_eq!(u.impulses(), 0);
        assert_eq!(u.info().kind, UnitKind::Instrument);
        assert!(u.info().accepts_midi);
    }

    #[test]
    fn endpoints_are_registered_by_name() {
        let m = BenchMidi::new();
        let s = m.hub.registry.endpoint(m.source).unwrap();
        assert_eq!((s.name.as_str(), s.direction), (SOURCE_NAME, Direction::Source));
        let d = m.hub.registry.endpoint(m.destination).unwrap();
        assert_eq!((d.name.as_str(), d.direction), (INSTRUMENT_NAME, Direction::Destination));
    }

    #[test]
    fn stamped_note_reaches_its_sample_through_the_hub() {
        // 50 kHz: 20 000 ns per sample; zero timestamp at 1 s.
        let tl = Timeline::new(0.0, 1_000_000_000, 50_000.0, 1.0);
        let stamp = |sample: u64| 1_000_000_000 + 20_000 * sample;
        let mut m = BenchMidi::new();
        assert!(m.send_note_on(stamp(70), 60));
        assert!(m.send_note_on(stamp(200), 61));
        let mut sched = EventScheduler::new();
        // Cycle 1 covers samples 64..128; its end in host time is stamp(128).
        assert_eq!(m.drain_into(stamp(128), &mut sched), 1);
        let mut evs = Vec::new();
        sched.take_cycle(&tl, 64, 64, &mut evs);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].offset, 6);
        let mut u = BenchInstrument::new();
        u.initialize(&cfg()).unwrap();
        let mut p = planes(64);
        render_cycle(&mut u, 64, 64.0, &evs, &mut p);
        let hot: Vec<usize> = p[0].iter().enumerate().filter(|(_, s)| **s != 0.0).map(|(i, _)| i).collect();
        assert_eq!(hot, vec![6]);
        // The second note is still in the hub for a later cycle.
        assert_eq!(m.drain_into(stamp(256), &mut sched), 1);
    }

    #[test]
    fn non_note_messages_are_dropped_on_conversion() {
        assert_eq!(to_unit_event(WireEvent::ControlChange { channel: 0, controller: 7, value: 1 }), None);
        assert_eq!(to_unit_event(WireEvent::NoteOff { channel: 2, key: 5, velocity: 9 }), Some(MidiEvent::note_off(0, 2, 5)));
    }
}
