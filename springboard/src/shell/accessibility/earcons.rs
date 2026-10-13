//! Screen reader earcons: short synthesised cues the reader plays next to its speech, one per event family (focus moved,
//! boundary, wrap, group entered or left, item selected, error, mode on or off, page turn, ...). Each cue is one to three
//! pitched blips or sweeps, mono i16 PCM at `kb_clicks::RATE`, under `MAX_MS`. This module is pure: it only maps events to
//! cues and synthesises samples; `Reader` queues the requests and the shell's sound player plays them. The cues are
//! synthesised stand-ins, not the original sounds.

use crate::shell::kb_clicks::RATE;

/// No cue lasts longer than this.
pub const MAX_MS: f32 = 400.0;

/// The accessibility notifications that carry a cue (`NSAccessibility.Notification`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification {
    FocusedUIElementChanged,
    LayoutChanged,
    AnnouncementRequested,
    SelectedTextChanged,
    RowExpanded,
    RowCollapsed,
}

/// What happened in the reader, before it is turned into a cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderEvent {
    /// the cursor moved to another element
    FocusMoved,
    /// the cursor ran off an end of the list without wrapping
    Boundary,
    /// the cursor wrapped from one end of the list to the other
    Wrapped,
    /// the rotor changed
    RotorChanged,
    /// the element under the cursor was pressed
    Activated,
    /// nothing to read, or the element is unavailable
    Failed,
    /// the screen reader (or a reader mode) turned on or off
    ModeToggled(bool),
    /// a post of `NSAccessibility.post(element:notification:)`
    Posted(Notification),
}

/// One cue per event family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Earcon {
    Focus,
    Boundary,
    Wrap,
    EnterGroup,
    ExitGroup,
    ItemSelected,
    Error,
    ModeOn,
    ModeOff,
    PageTurn,
    Announcement,
    TextSelected,
}

impl Earcon {
    /// Every cue, in a stable order.
    pub const ALL: [Earcon; 12] = [
        Earcon::Focus,
        Earcon::Boundary,
        Earcon::Wrap,
        Earcon::EnterGroup,
        Earcon::ExitGroup,
        Earcon::ItemSelected,
        Earcon::Error,
        Earcon::ModeOn,
        Earcon::ModeOff,
        Earcon::PageTurn,
        Earcon::Announcement,
        Earcon::TextSelected,
    ];

    /// The cue for a reader event. Total: every event has one.
    pub fn for_event(e: ReaderEvent) -> Earcon {
        match e {
            ReaderEvent::FocusMoved => Earcon::Focus,
            ReaderEvent::Boundary => Earcon::Boundary,
            ReaderEvent::Wrapped => Earcon::Wrap,
            ReaderEvent::RotorChanged => Earcon::PageTurn,
            ReaderEvent::Activated => Earcon::ItemSelected,
            ReaderEvent::Failed => Earcon::Error,
            ReaderEvent::ModeToggled(true) => Earcon::ModeOn,
            ReaderEvent::ModeToggled(false) => Earcon::ModeOff,
            ReaderEvent::Posted(n) => match n {
                Notification::FocusedUIElementChanged => Earcon::Focus,
                Notification::LayoutChanged => Earcon::PageTurn,
                Notification::AnnouncementRequested => Earcon::Announcement,
                Notification::SelectedTextChanged => Earcon::TextSelected,
                Notification::RowExpanded => Earcon::EnterGroup,
                Notification::RowCollapsed => Earcon::ExitGroup,
            },
        }
    }

    /// The segments of this cue.
    pub fn segments(self) -> &'static [Segment] {
        match self {
            Earcon::Focus => &[seg(1200.0, 1200.0, 0.0, 35.0, 0.35)],
            Earcon::Boundary => &[seg(300.0, 300.0, 0.0, 90.0, 0.45)],
            Earcon::Wrap => &[seg(1500.0, 600.0, 0.0, 140.0, 0.4)],
            Earcon::EnterGroup => &[seg(700.0, 1000.0, 0.0, 70.0, 0.35), seg(1000.0, 1400.0, 70.0, 70.0, 0.35)],
            Earcon::ExitGroup => &[seg(1400.0, 1000.0, 0.0, 70.0, 0.35), seg(1000.0, 700.0, 70.0, 70.0, 0.35)],
            Earcon::ItemSelected => &[seg(900.0, 900.0, 0.0, 50.0, 0.4), seg(1350.0, 1350.0, 55.0, 90.0, 0.4)],
            Earcon::Error => &[seg(180.0, 140.0, 0.0, 120.0, 0.5), seg(180.0, 140.0, 160.0, 120.0, 0.5)],
            Earcon::ModeOn => &[seg(600.0, 600.0, 0.0, 70.0, 0.4), seg(800.0, 800.0, 80.0, 70.0, 0.4), seg(1000.0, 1000.0, 160.0, 120.0, 0.4)],
            Earcon::ModeOff => &[seg(1000.0, 1000.0, 0.0, 70.0, 0.4), seg(800.0, 800.0, 80.0, 70.0, 0.4), seg(600.0, 600.0, 160.0, 120.0, 0.4)],
            Earcon::PageTurn => &[seg(400.0, 1800.0, 0.0, 200.0, 0.3)],
            Earcon::Announcement => &[seg(2000.0, 2000.0, 0.0, 40.0, 0.3), seg(2000.0, 2000.0, 90.0, 40.0, 0.3)],
            Earcon::TextSelected => &[seg(500.0, 500.0, 0.0, 60.0, 0.35), seg(1500.0, 1500.0, 0.0, 60.0, 0.2)],
        }
    }

    /// Length of the cue in ms.
    pub fn len_ms(self) -> f32 {
        self.segments().iter().map(|s| s.at_ms + s.len_ms).fold(0.0, f32::max)
    }

    /// Mono i16 PCM of the cue at `RATE`, scaled by `volume` (0..=1).
    pub fn pcm(self, volume: f32) -> Vec<i16> {
        let n = (RATE as f32 * self.len_ms() / 1000.0).ceil() as usize;
        let gain = volume.clamp(0.0, 1.0);
        let mut mix = vec![0f32; n];
        for s in self.segments() {
            let start = (RATE as f32 * s.at_ms / 1000.0) as usize;
            let len = (RATE as f32 * s.len_ms / 1000.0) as usize;
            let mut phase = 0f32;
            for i in 0..len.min(n.saturating_sub(start)) {
                let k = i as f32 / len as f32;
                let hz = s.from_hz + (s.to_hz - s.from_hz) * k;
                phase += hz / RATE as f32;
                // a 3 ms attack, then a quadratic decay to silence
                let attack = (i as f32 / (RATE as f32 * 0.003)).min(1.0);
                mix[start + i] += (phase * std::f32::consts::TAU).sin() * s.peak * attack * (1.0 - k).powi(2);
            }
        }
        mix.iter().map(|v| (v.clamp(-1.0, 1.0) * gain * i16::MAX as f32) as i16).collect()
    }
}

/// A pitched blip or sweep: frequency from `from_hz` to `to_hz` over `len_ms`, starting `at_ms` into the cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from_hz: f32,
    pub to_hz: f32,
    pub at_ms: f32,
    pub len_ms: f32,
    pub peak: f32,
}

const fn seg(from_hz: f32, to_hz: f32, at_ms: f32, len_ms: f32, peak: f32) -> Segment {
    Segment { from_hz, to_hz, at_ms, len_ms, peak }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_earcon_is_audible_and_under_400_ms() {
        for e in Earcon::ALL {
            let pcm = e.pcm(1.0);
            assert!(pcm.iter().any(|&s| s != 0), "{e:?} is silent");
            assert!(e.len_ms() <= MAX_MS, "{e:?} is too long");
            assert_eq!(pcm.len(), (RATE as f32 * e.len_ms() / 1000.0).ceil() as usize);
        }
    }

    #[test]
    fn every_earcon_differs_from_the_others() {
        for (i, a) in Earcon::ALL.iter().enumerate() {
            for b in &Earcon::ALL[i + 1..] {
                assert_ne!(a.segments(), b.segments(), "{a:?} and {b:?} share a note sequence");
                assert_ne!(a.pcm(1.0), b.pcm(1.0), "{a:?} and {b:?} sound the same");
            }
        }
    }

    #[test]
    fn zero_volume_is_silent() {
        assert!(Earcon::Focus.pcm(0.0).iter().all(|&s| s == 0));
    }

    #[test]
    fn the_event_mapping_is_total_and_reaches_every_earcon() {
        use Notification::*;
        use ReaderEvent::*;
        let events = [
            FocusMoved, Boundary, Wrapped, RotorChanged, Activated, Failed, ModeToggled(true), ModeToggled(false),
            Posted(FocusedUIElementChanged), Posted(LayoutChanged), Posted(AnnouncementRequested), Posted(SelectedTextChanged),
            Posted(RowExpanded), Posted(RowCollapsed),
        ];
        let reached: Vec<Earcon> = events.iter().map(|&e| Earcon::for_event(e)).collect();
        for e in Earcon::ALL {
            assert!(reached.contains(&e), "no event plays {e:?}");
        }
        assert_eq!(Earcon::for_event(Posted(RowExpanded)), Earcon::EnterGroup);
        assert_eq!(Earcon::for_event(Posted(RowCollapsed)), Earcon::ExitGroup);
    }
}
