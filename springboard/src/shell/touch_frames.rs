//! Touch frames for apps: the samples the shell feeds to an app's `TouchTracker` (`bifrost_ui::touch`).
//!
//! An app that receives `ToApp::Touches` gets whole frames (phases, previous locations, fingers that stayed down, tap
//! counts) and drops the plain `ToApp::Touch` stream. The shell makes the samples from its own input: a touchscreen
//! finger is a `TouchType::Direct` sample under its contact id (`Event::Contact`); a pointer that is not on the surface
//! (a mouse or trackpad) is one `TouchType::Indirect` sample under [`POINTER_ID`].
//!
//! The first finger of a touchscreen also arrives on the pointer stream (`Event::Down` / `Move` / `Up`). While a contact
//! is down, [`pointer_sample`] adds nothing, so that finger is not reported twice. The module is pure: the caller passes
//! the samples and the clock.

use crate::bare::input::TouchPhase;
use bifrost_proto::{TouchKind, TouchSource};
use bifrost_ui::touch::{Raw, Sample, TouchType};

/// The contact id of the pointer. Compositor and evdev contact ids are small, so this value never collides with one.
pub const POINTER_ID: u32 = u32::MAX;

/// The contact id of a tap the shell synthesizes for an accessibility press (`ax press`). Not a finger.
pub const SYNTHETIC_ID: u32 = u32::MAX - 1;

/// What produced the plain touch of a pointer-stream event: a press made while a touchscreen finger is down is that
/// finger's mirror (a finger); any other press is a mouse or trackpad (the pointer). The rest of the touch (moves, the
/// release) keeps the source its press had, since the contact lifts before its mirror release arrives.
pub fn plain_source(kind: TouchKind, contacts: &Contacts, held: TouchSource) -> TouchSource {
    match kind {
        TouchKind::Down if contacts.touching() => TouchSource::Finger,
        TouchKind::Down => TouchSource::Pointer,
        _ => held,
    }
}

/// The tracker's raw kind of a contact phase.
pub fn raw_of_phase(phase: TouchPhase) -> Raw {
    match phase {
        TouchPhase::Down => Raw::Down,
        TouchPhase::Move => Raw::Move,
        TouchPhase::Up => Raw::Up,
        TouchPhase::Cancel => Raw::Cancel,
    }
}

/// The tracker's raw kind of a pointer-stream event.
pub fn raw_of_kind(kind: TouchKind) -> Raw {
    match kind {
        TouchKind::Down => Raw::Down,
        TouchKind::Move => Raw::Move,
        TouchKind::Up => Raw::Up,
        TouchKind::Cancel => Raw::Cancel,
    }
}

/// The sample of one touchscreen contact, in the surface's pixels.
pub fn contact_sample(id: i32, x: f32, y: f32) -> Sample {
    Sample { id: id as u32, x, y, kind: TouchType::Direct, ..Sample::default() }
}

/// The sample of one synthesized tap (an accessibility press), in the surface's pixels.
pub fn synthetic_sample(x: f32, y: f32) -> Sample {
    Sample { id: SYNTHETIC_ID, x, y, kind: TouchType::Direct, ..Sample::default() }
}

/// The finger contacts that are down on the touchscreen. The pointer stream stands aside while one is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contacts {
    down: Vec<i32>,
}

impl Contacts {
    /// A contact went down.
    pub fn down(&mut self, id: i32) {
        if !self.down.contains(&id) {
            self.down.push(id);
        }
    }

    /// A contact lifted or was taken away.
    pub fn up(&mut self, id: i32) {
        self.down.retain(|&d| d != id);
    }

    /// True while at least one finger is on the touchscreen.
    pub fn touching(&self) -> bool {
        !self.down.is_empty()
    }

    /// Forgets every contact (the app lost the touches; the fingers themselves may still be down).
    pub fn clear(&mut self) {
        self.down.clear();
    }
}

/// The tracker sample of one pointer-stream event at (`x`, `y`), or `None` while a touchscreen finger is down: that
/// finger is reported by its own contact.
pub fn pointer_sample(kind: TouchKind, x: f32, y: f32, contacts: &Contacts) -> Option<(Raw, Sample)> {
    if contacts.touching() {
        return None;
    }
    Some((raw_of_kind(kind), Sample { id: POINTER_ID, x, y, kind: TouchType::Indirect, ..Sample::default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::touch::{TouchEvent, TouchPhase as Phase, TouchTracker};

    fn frame(tr: &mut TouchTracker, raw: Raw, s: Sample, t: f64) -> Option<TouchEvent> {
        tr.frame(&[(raw, s)], t)
    }

    #[test]
    fn phases_and_kinds_map_onto_the_tracker() {
        assert_eq!(raw_of_phase(TouchPhase::Down), Raw::Down);
        assert_eq!(raw_of_phase(TouchPhase::Move), Raw::Move);
        assert_eq!(raw_of_phase(TouchPhase::Up), Raw::Up);
        assert_eq!(raw_of_phase(TouchPhase::Cancel), Raw::Cancel);
        assert_eq!(raw_of_kind(TouchKind::Down), Raw::Down);
        assert_eq!(raw_of_kind(TouchKind::Move), Raw::Move);
        assert_eq!(raw_of_kind(TouchKind::Up), Raw::Up);
        assert_eq!(raw_of_kind(TouchKind::Cancel), Raw::Cancel);
    }

    #[test]
    fn contact_and_pointer_samples_carry_their_device_kind() {
        let c = contact_sample(3, 10.0, 20.0);
        assert_eq!((c.id, c.kind, c.x, c.y), (3, TouchType::Direct, 10.0, 20.0));
        let (raw, p) = pointer_sample(TouchKind::Down, 1.0, 2.0, &Contacts::default()).expect("a pointer with no finger down");
        assert_eq!(raw, Raw::Down);
        assert_eq!((p.id, p.kind), (POINTER_ID, TouchType::Indirect));
        assert_eq!(synthetic_sample(0.0, 0.0).kind, TouchType::Direct);
    }

    #[test]
    fn contacts_track_fingers_and_clear() {
        let mut c = Contacts::default();
        assert!(!c.touching());
        c.down(1);
        c.down(1);
        c.down(2);
        assert!(c.touching());
        c.up(1);
        assert!(c.touching(), "one finger is still down");
        c.up(2);
        assert!(!c.touching());
        c.down(5);
        c.clear();
        assert!(!c.touching());
        c.up(9); // an unknown contact is ignored
        assert!(!c.touching());
    }

    #[test]
    fn a_press_is_a_finger_when_a_contact_is_down_and_the_pointer_otherwise_and_keeps_that_to_its_release() {
        let mut c = Contacts::default();
        let none = TouchSource::default();
        // a mouse press, drag and release
        let s = plain_source(TouchKind::Down, &c, none);
        assert_eq!(s, TouchSource::Pointer);
        assert_eq!(plain_source(TouchKind::Move, &c, s), TouchSource::Pointer);
        assert_eq!(plain_source(TouchKind::Up, &c, s), TouchSource::Pointer);
        // a finger: the contact is down before its pointer mirror, and up again before the mirror release
        c.down(4);
        let s = plain_source(TouchKind::Down, &c, s);
        assert_eq!(s, TouchSource::Finger);
        c.up(4);
        assert_eq!(plain_source(TouchKind::Move, &c, s), TouchSource::Finger);
        assert_eq!(plain_source(TouchKind::Up, &c, s), TouchSource::Finger);
    }

    #[test]
    fn the_pointer_stands_aside_while_a_finger_is_down() {
        let mut c = Contacts::default();
        c.down(7);
        assert_eq!(pointer_sample(TouchKind::Move, 1.0, 1.0, &c), None);
        c.up(7);
        assert!(pointer_sample(TouchKind::Up, 1.0, 1.0, &c).is_some());
    }

    #[test]
    fn a_finger_is_reported_once_with_its_pointer_mirror_suppressed() {
        // the touchscreen order: the contact first, then the pointer mirror of the primary finger
        let mut tr = TouchTracker::new();
        let mut c = Contacts::default();
        c.down(4);
        let mirror = pointer_sample(TouchKind::Down, 50.0, 60.0, &c);
        assert_eq!(mirror, None);
        let ev = frame(&mut tr, Raw::Down, contact_sample(4, 50.0, 60.0), 100.0).expect("the contact begins a touch");
        assert_eq!(ev.touches.len(), 1);
        assert_eq!(ev.touches[0].phase, Phase::Began);
        assert_eq!(ev.touches[0].kind, TouchType::Direct);
        assert_eq!(tr.active_count(), 1);
    }

    #[test]
    fn a_mouse_click_is_an_indirect_tap_with_a_tap_count() {
        let mut tr = TouchTracker::new();
        let c = Contacts::default();
        let (raw, s) = pointer_sample(TouchKind::Down, 30.0, 40.0, &c).unwrap();
        let down = tr.frame(&[(raw, s)], 0.0).unwrap();
        assert_eq!(down.touches[0].kind, TouchType::Indirect);
        assert_eq!(down.touches[0].tap_count, 1);
        let (raw, s) = pointer_sample(TouchKind::Up, 30.0, 40.0, &c).unwrap();
        let up = tr.frame(&[(raw, s)], 60.0).unwrap();
        assert_eq!(up.touches[0].phase, Phase::Ended);
        assert_eq!(tr.active_count(), 0);
    }

    #[test]
    fn a_synthesized_press_makes_a_begin_and_an_end_frame() {
        let mut tr = TouchTracker::new();
        let s = synthetic_sample(12.0, 24.0);
        let down = frame(&mut tr, Raw::Down, s, 1000.0).expect("down frame");
        assert_eq!(down.touches[0].phase, Phase::Began);
        let up = frame(&mut tr, Raw::Up, s, 1040.0).expect("up frame");
        assert_eq!(up.touches[0].phase, Phase::Ended);
        assert_eq!(up.touches[0].tap_count, 1);
    }

    #[test]
    fn taking_the_touches_away_cancels_every_contact_once() {
        let mut tr = TouchTracker::new();
        frame(&mut tr, Raw::Down, contact_sample(1, 5.0, 5.0), 0.0);
        frame(&mut tr, Raw::Down, contact_sample(2, 9.0, 9.0), 0.0);
        let ev = tr.cancel_all(10.0).expect("two cancelled touches");
        let cancelled: Vec<u32> = ev.touches.iter().filter(|t| t.phase == Phase::Cancelled).map(|t| t.id).collect();
        assert_eq!(cancelled.len(), 2);
        assert_eq!(tr.active_count(), 0);
        assert_eq!(tr.cancel_all(11.0), None, "nothing left to cancel");
    }
}
