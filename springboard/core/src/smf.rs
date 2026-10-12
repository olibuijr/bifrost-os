//! Standard MIDI File reader and playback timeline. It parses a format 0, 1 or 2 file with ticks per quarter note,
//! merges every track into one list of channel messages, and converts each tick to seconds with the file's tempo
//! changes (default 120 beats per minute until the first tempo event). `Playback` walks that list at a rate.
//! This is the sequencing half of MIDI playback. It produces no sound: a sound bank would turn the messages into
//! audio, and none is loaded here. SMPTE time divisions are refused.

use std::fmt;

/// A channel message from a track. A note-on with velocity 0 is reported as a note-off, as the file format says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    NoteOff { channel: u8, key: u8, velocity: u8 },
    NoteOn { channel: u8, key: u8, velocity: u8 },
    PolyPressure { channel: u8, key: u8, pressure: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    ChannelPressure { channel: u8, pressure: u8 },
    /// The 14-bit bend value: 8192 is centred.
    PitchBend { channel: u8, value: u16 },
}

/// One message with its time in seconds from the start of the file and the index of its track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub seconds: f64,
    pub track: usize,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmfError {
    /// The data does not start with a header chunk.
    NotMidi,
    /// The data ends before a chunk or event is complete.
    Truncated,
    /// The file uses a feature this reader does not handle.
    Unsupported(&'static str),
}

impl fmt::Display for SmfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmfError::NotMidi => write!(f, "not a Standard MIDI File"),
            SmfError::Truncated => write!(f, "MIDI file ends in the middle of a chunk or event"),
            SmfError::Unsupported(what) => write!(f, "unsupported MIDI file feature: {what}"),
        }
    }
}

/// A parsed file: its events in time order and its length.
#[derive(Clone, Debug, PartialEq)]
pub struct Smf {
    format: u16,
    division: u16,
    events: Vec<Event>,
    duration: f64,
}

/// Tempo in microseconds per quarter note when a file sets none.
const DEFAULT_TEMPO_US: u32 = 500_000;

impl Smf {
    /// Parses a whole file. Returns an error for data that is not a file, for a file that ends too early, and for
    /// the SMPTE time division.
    pub fn parse(bytes: &[u8]) -> Result<Smf, SmfError> {
        let mut r = Reader { data: bytes, pos: 0 };
        if r.take(4).map_err(|_| SmfError::NotMidi)? != b"MThd" {
            return Err(SmfError::NotMidi);
        }
        let len = r.u32()? as usize;
        if len < 6 {
            return Err(SmfError::NotMidi);
        }
        let header = r.take(len)?;
        let format = u16::from_be_bytes([header[0], header[1]]);
        let tracks = u16::from_be_bytes([header[2], header[3]]) as usize;
        let division = u16::from_be_bytes([header[4], header[5]]);
        if format > 2 {
            return Err(SmfError::Unsupported("format above 2"));
        }
        if division & 0x8000 != 0 {
            return Err(SmfError::Unsupported("SMPTE time division"));
        }
        if division == 0 {
            return Err(SmfError::Unsupported("zero ticks per quarter note"));
        }

        let mut raw: Vec<(u64, usize, Message)> = Vec::new();
        let mut tempos: Vec<(u64, u32)> = Vec::new();
        let mut ends: Vec<u64> = Vec::new();
        let mut found = 0;
        while found < tracks {
            let id = r.take(4)?;
            let len = r.u32()? as usize;
            let body = r.take(len)?;
            if id == b"MTrk" {
                parse_track(body, found, &mut raw, &mut tempos, &mut ends)?;
                found += 1;
            }
        }

        raw.sort_by_key(|e| e.0);
        tempos.sort_by_key(|t| t.0);
        let map = tempo_map(division, &tempos);
        let last_tick = ends.iter().copied().chain(raw.iter().map(|e| e.0)).max().unwrap_or(0);
        let duration = seconds_at(&map, division, last_tick);
        let events = raw
            .into_iter()
            .map(|(tick, track, message)| Event { seconds: seconds_at(&map, division, tick), track, message })
            .collect();
        Ok(Smf { format, division, events, duration })
    }

    /// The file format: 0 (one track), 1 (tracks played together) or 2 (independent sequences).
    pub fn format(&self) -> u16 {
        self.format
    }

    /// Ticks per quarter note.
    pub fn division(&self) -> u16 {
        self.division
    }

    /// Every channel message, sorted by time. Messages at the same time keep their track order.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Seconds from the start to the last end-of-track event (or the last message, if later).
    pub fn duration(&self) -> f64 {
        self.duration
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, SmfError> {
        let b = *self.data.get(self.pos).ok_or(SmfError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], SmfError> {
        let end = self.pos.checked_add(n).ok_or(SmfError::Truncated)?;
        let s = self.data.get(self.pos..end).ok_or(SmfError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, SmfError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// A variable-length quantity: seven bits per byte, high bit set on all but the last. At most four bytes.
    fn vlq(&mut self) -> Result<u32, SmfError> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.byte()?;
            v = (v << 7) | (b & 0x7F) as u32;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(SmfError::Unsupported("variable-length value longer than four bytes"))
    }
}

fn parse_track(
    body: &[u8],
    track: usize,
    raw: &mut Vec<(u64, usize, Message)>,
    tempos: &mut Vec<(u64, u32)>,
    ends: &mut Vec<u64>,
) -> Result<(), SmfError> {
    let mut r = Reader { data: body, pos: 0 };
    let mut tick: u64 = 0;
    let mut running: Option<u8> = None;
    while r.pos < body.len() {
        tick += r.vlq()? as u64;
        let first = r.byte()?;
        let status = if first & 0x80 != 0 {
            first
        } else {
            // A data byte reuses the last channel status (running status).
            r.pos -= 1;
            running.ok_or(SmfError::Unsupported("data byte without running status"))?
        };
        match status {
            0xFF => {
                let kind = r.byte()?;
                let len = r.vlq()? as usize;
                let data = r.take(len)?;
                match kind {
                    0x2F => {
                        ends.push(tick);
                        break;
                    }
                    0x51 if len == 3 => {
                        tempos.push((tick, u32::from_be_bytes([0, data[0], data[1], data[2]])));
                    }
                    _ => {}
                }
            }
            0xF0 | 0xF7 => {
                let len = r.vlq()? as usize;
                r.take(len)?;
                running = None;
            }
            0x80..=0xEF => {
                running = Some(status);
                let channel = status & 0x0F;
                let a = r.byte()?;
                let message = match status & 0xF0 {
                    0xC0 => Message::ProgramChange { channel, program: a },
                    0xD0 => Message::ChannelPressure { channel, pressure: a },
                    kind => {
                        let b = r.byte()?;
                        match kind {
                            0x80 => Message::NoteOff { channel, key: a, velocity: b },
                            0x90 if b == 0 => Message::NoteOff { channel, key: a, velocity: 0 },
                            0x90 => Message::NoteOn { channel, key: a, velocity: b },
                            0xA0 => Message::PolyPressure { channel, key: a, pressure: b },
                            0xB0 => Message::ControlChange { channel, controller: a, value: b },
                            _ => Message::PitchBend { channel, value: ((b as u16) << 7) | (a as u16 & 0x7F) },
                        }
                    }
                };
                raw.push((tick, track, message));
            }
            _ => return Err(SmfError::Unsupported("system common or real-time message in a track")),
        }
    }
    Ok(())
}

/// A tempo in force from `tick` on, with the time in seconds at which it starts.
#[derive(Clone, Copy, Debug)]
struct TempoPoint {
    tick: u64,
    us: u32,
    at: f64,
}

/// The tempo points of a sorted tempo list. The first point is at tick 0 with the default tempo unless the file sets
/// one there. Two changes at the same tick: the later one in the list wins.
fn tempo_map(division: u16, tempos: &[(u64, u32)]) -> Vec<TempoPoint> {
    let div = division as f64;
    let mut points = vec![TempoPoint { tick: 0, us: DEFAULT_TEMPO_US, at: 0.0 }];
    for &(tick, us) in tempos {
        let last = *points.last().unwrap();
        if tick == last.tick {
            points.last_mut().unwrap().us = us;
        } else {
            let at = last.at + (tick - last.tick) as f64 * last.us as f64 / (div * 1_000_000.0);
            points.push(TempoPoint { tick, us, at });
        }
    }
    points
}

fn seconds_at(points: &[TempoPoint], division: u16, tick: u64) -> f64 {
    let p = points.iter().rev().find(|p| p.tick <= tick).unwrap_or(&points[0]);
    p.at + (tick - p.tick) as f64 * p.us as f64 / (division as f64 * 1_000_000.0)
}

/// Walks an event list at a playback rate. `advance` returns the events whose time has been reached since the
/// last call, so each event is returned once. Nothing plays while the walk is stopped.
#[derive(Debug)]
pub struct Playback<'a> {
    smf: &'a Smf,
    position: f64,
    rate: f64,
    playing: bool,
    next: usize,
}

impl<'a> Playback<'a> {
    /// A stopped walk at the start of the file, at rate 1.
    pub fn new(smf: &'a Smf) -> Self {
        Self { smf, position: 0.0, rate: 1.0, playing: false, next: 0 }
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Position in seconds of the file's own clock.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Sets the rate. Only a positive finite rate is accepted; returns false and keeps the old rate otherwise.
    pub fn set_rate(&mut self, rate: f64) -> bool {
        if rate.is_finite() && rate > 0.0 {
            self.rate = rate;
            true
        } else {
            false
        }
    }

    /// Moves to `seconds` (clamped to 0 or later). Events before that point count as already returned.
    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        self.position = seconds.max(0.0);
        let events = self.smf.events();
        self.next = events.partition_point(|e| e.seconds <= self.position);
    }

    /// Moves the walk on by `dt` seconds of wall time, scaled by the rate, and returns the events reached.
    /// Returns nothing while stopped or for a non-positive or non-finite `dt`.
    pub fn advance(&mut self, dt: f64) -> &'a [Event] {
        if !self.playing || !dt.is_finite() || dt <= 0.0 {
            return &[];
        }
        let smf: &'a Smf = self.smf;
        let events = smf.events();
        let start = self.next;
        self.position += dt * self.rate;
        while self.next < events.len() && events[self.next].seconds <= self.position {
            self.next += 1;
        }
        &events[start..self.next]
    }

    /// True once every event has been returned.
    pub fn is_finished(&self) -> bool {
        self.next >= self.smf.events().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps one track body in a format 0 file with one track.
    fn file(division: u16, track: &[u8]) -> Vec<u8> {
        let mut v = b"MThd".to_vec();
        v.extend_from_slice(&6u32.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&division.to_be_bytes());
        v.extend_from_slice(b"MTrk");
        v.extend_from_slice(&(track.len() as u32).to_be_bytes());
        v.extend_from_slice(track);
        v
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// Note 60 on at tick 0 and off at tick 480, with the default tempo: 480 ticks at 500000 us per 480 ticks is 0.5 s.
    fn one_note() -> Vec<u8> {
        file(
            480,
            &[0x00, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 64, 0x00, 0xFF, 0x2F, 0x00],
        )
    }

    #[test]
    fn note_off_lands_at_half_a_second_with_the_default_tempo() {
        let smf = Smf::parse(&one_note()).unwrap();
        assert_eq!(smf.format(), 0);
        assert_eq!(smf.division(), 480);
        let e = smf.events();
        assert_eq!(e.len(), 2);
        assert!(close(e[0].seconds, 0.0));
        assert_eq!(e[0].message, Message::NoteOn { channel: 0, key: 60, velocity: 100 });
        assert!(close(e[1].seconds, 0.5));
        assert_eq!(e[1].message, Message::NoteOff { channel: 0, key: 60, velocity: 64 });
        assert!(close(smf.duration(), 0.5));
    }

    #[test]
    fn tempo_change_at_tick_96_moves_the_later_note_off() {
        // Division 96. Tick 96: tempo set to 1000000 us per quarter (1 beat per second). Note off at tick 192.
        // Ticks 0..96 at the default 0.5 s per 96 ticks take 0.5 s. Ticks 96..192 at 1 s per 96 ticks take 1 s.
        let track = [
            0x00, 0x90, 60, 100, // note on at tick 0
            0x60, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40, // tempo at tick 96
            0x60, 0x80, 60, 0, // note off at tick 192
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let smf = Smf::parse(&file(96, &track)).unwrap();
        let e = smf.events();
        assert!(close(e[0].seconds, 0.0));
        assert!(close(e[1].seconds, 1.5));
        assert!(close(smf.duration(), 1.5));
    }

    #[test]
    fn running_status_reuses_the_last_channel_status() {
        // Second note-on has no status byte: 0x3E (key 62) and 80 (velocity) follow 0x90 from the first event.
        // Ten ticks at 0.5 s per 480 ticks is 10 / 960 s.
        let track = [0x00, 0x90, 60, 100, 0x0A, 0x3E, 80, 0x00, 0xFF, 0x2F, 0x00];
        let smf = Smf::parse(&file(480, &track)).unwrap();
        let e = smf.events();
        assert_eq!(e.len(), 2);
        assert_eq!(e[1].message, Message::NoteOn { channel: 0, key: 62, velocity: 80 });
        assert!(close(e[1].seconds, 10.0 / 960.0));
    }

    #[test]
    fn note_on_with_velocity_zero_is_a_note_off() {
        let track = [0x00, 0x90, 60, 0x00, 0x00, 0xFF, 0x2F, 0x00];
        let smf = Smf::parse(&file(480, &track)).unwrap();
        assert_eq!(smf.events().len(), 1);
        assert_eq!(smf.events()[0].message, Message::NoteOff { channel: 0, key: 60, velocity: 0 });
    }

    #[test]
    fn pitch_bend_and_program_change_are_decoded() {
        // Pitch bend on channel 2: lsb 0, msb 0x40 gives (0x40 << 7) = 8192, the centre value.
        // Program change on channel 1 to program 5.
        let track = [
            0x00, 0xE2, 0x00, 0x40, //
            0x00, 0xC1, 0x05, //
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let smf = Smf::parse(&file(480, &track)).unwrap();
        let e = smf.events();
        assert_eq!(e[0].message, Message::PitchBend { channel: 2, value: 8192 });
        assert_eq!(e[1].message, Message::ProgramChange { channel: 1, program: 5 });
    }

    #[test]
    fn bad_headers_and_truncated_data_are_errors() {
        assert_eq!(Smf::parse(b"RIFF\0\0\0\x06....").unwrap_err(), SmfError::NotMidi);
        // The note-on needs a second data byte that is not there.
        assert_eq!(Smf::parse(&file(480, &[0x00, 0x90, 60])).unwrap_err(), SmfError::Truncated);
        // Bit 15 of the division selects SMPTE timing, which is refused.
        assert!(matches!(
            Smf::parse(&file(0xE728, &[0x00, 0xFF, 0x2F, 0x00])),
            Err(SmfError::Unsupported(_))
        ));
    }

    #[test]
    fn playback_returns_each_event_once_as_time_passes() {
        let smf = Smf::parse(&one_note()).unwrap();
        let mut p = Playback::new(&smf);
        // Stopped: nothing is returned and the position does not move.
        assert!(p.advance(1.0).is_empty());
        p.play();
        // The note on is at 0 s, so the first step returns it.
        let first = p.advance(0.25);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].message, Message::NoteOn { channel: 0, key: 60, velocity: 100 });
        // The note off is at 0.5 s, reached at a position of 0.5.
        let second = p.advance(0.25);
        assert_eq!(second.len(), 1);
        assert!(p.is_finished());
        assert!(p.advance(1.0).is_empty());
    }

    #[test]
    fn rate_scales_the_walk_and_seek_skips_passed_events() {
        let smf = Smf::parse(&one_note()).unwrap();
        let mut p = Playback::new(&smf);
        assert!(p.set_rate(2.0));
        assert!(!p.set_rate(0.0));
        assert!(!p.set_rate(f64::NAN));
        assert!(close(p.rate(), 2.0));
        p.play();
        // At rate 2, a 0.25 s step covers 0.5 s of the file, so both events are reached.
        assert_eq!(p.advance(0.25).len(), 2);
        // Seeking past the end leaves nothing to return.
        p.seek(1.0);
        assert!(p.is_finished());
        assert!(p.advance(0.1).is_empty());
    }
}
