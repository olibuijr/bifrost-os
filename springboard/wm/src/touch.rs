//! Touchscreen input: multi-touch contacts reach clients as `wl_touch`, and a client that never bound `wl_touch`
//! gets a pointer emulated from the first contact.
//!
//! The pieces:
//! - [`map_touch`] and [`to_logical`] are pure: they turn a device position in 0..1 into a point in the output's
//!   logical space. [`output_transform`] is the output's rotation (`crate::transform`): the normalized device
//!   position goes through it before it is scaled to the output.
//! - [`Contacts`] is a pure book of the fingers that are down and of the one that drives the emulated pointer.
//! - [`WmState`] methods deliver the events: touch down, motion, up, cancel and frame go to the surface under the
//!   contact through Smithay's touch handle (surface-local coordinates are computed there from the surface origin
//!   given at touch down). The emulated pointer uses the same pointer functions as a mouse.
//!
//! The seat has the touch capability only while a touchscreen exists (`touch_device_added` and
//! `touch_device_removed`, called by the DRM backend), so a desktop without one does not advertise it.

use smithay::{
    backend::input::{AbsolutePositionEvent, Event, InputBackend, InputEvent, TouchEvent, TouchSlot},
    input::{
        pointer::MotionEvent as PointerMotion,
        touch::{DownEvent, MotionEvent, UpEvent},
    },
    output::Output,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{Logical, Point, Transform, SERIAL_COUNTER},
};

use crate::state::WmState;

/// The evdev code of the left mouse button, which the emulated pointer presses.
pub const BTN_LEFT: u32 = 0x110;

/// How finely a normalized position is sampled from the device: the backend scales its range to this many units.
const NORM_SCALE: i32 = 1 << 16;

/// How far outside the surface a cancelled emulated press is dragged before it is released, so that toolkits do
/// not count the release as a click.
const CANCEL_OFFSET: f64 = 100_000.0;

/// A device position with both axes in 0..1, in the panel's own orientation.
pub type Norm = (f64, f64);

/// The transform from the panel's orientation to `output`'s current orientation (its rotation, see
/// `crate::transform`); every touch position passes through it.
pub fn output_transform(output: &Output) -> Transform {
    output.current_transform()
}

/// A normalized position scaled to the output rectangle at `origin` with `size`, kept inside it (the last pixel
/// is `origin + size - 1`, as for the pointer). A position outside 0..1, or not a number, is pulled to the edge.
pub fn to_logical(norm: Norm, origin: (i32, i32), size: (i32, i32)) -> (f64, f64) {
    let axis = |n: f64, origin: i32, size: i32| {
        let n = if n.is_finite() { n.clamp(0.0, 1.0) } else { 0.0 };
        let last = (size - 1).max(0) as f64;
        origin as f64 + (n * size as f64).min(last)
    };
    (axis(norm.0, origin.0, size.0), axis(norm.1, origin.1, size.1))
}

/// A normalized device position on the output at `origin` with `size`, after the output transform: the panel
/// fraction becomes a desktop fraction first, then it is scaled to the output's logical rectangle.
pub fn map_touch(norm: Norm, transform: Transform, origin: (i32, i32), size: (i32, i32)) -> (f64, f64) {
    to_logical(crate::transform::normalized_to_desktop_fraction(norm, transform), origin, size)
}

/// The position of an absolute-position event in 0..1 of the device's range.
pub fn normalized<B: InputBackend, E: AbsolutePositionEvent<B>>(event: &E) -> Norm {
    (event.x_transformed(NORM_SCALE) / NORM_SCALE as f64, event.y_transformed(NORM_SCALE) / NORM_SCALE as f64)
}

/// Whether a new contact drives an emulated pointer: only the first contact of a touch session, and only when the
/// client under it never bound `wl_touch`.
pub fn wants_pointer_emulation(first_contact: bool, client_has_touch: bool) -> bool {
    first_contact && !client_has_touch
}

/// Whether the pointer image is drawn. A finger has no cursor, so it is hidden while the last input was a touch
/// contact, and real pointer motion shows it again.
pub fn cursor_shown(last_input_was_touch: bool) -> bool {
    !last_input_was_touch
}

/// The touchscreens the backend has announced. The seat keeps the touch capability while at least one is present.
/// Only announced devices are counted: a touch from a source that was never announced (a nested window) gives the
/// seat the capability without a count, so no later removal can leave it behind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TouchDevices(usize);

impl TouchDevices {
    /// The number of announced touchscreens.
    pub fn count(self) -> usize {
        self.0
    }

    /// A touchscreen was announced.
    pub fn added(&mut self) {
        self.0 += 1;
    }

    /// A touchscreen went away. True when it was the last one, so the seat must lose the capability.
    pub fn removed(&mut self) -> bool {
        self.0 = self.0.saturating_sub(1);
        self.0 == 0
    }
}

/// The finger that drives the emulated pointer, and where it last was.
#[derive(Clone, Copy, Debug)]
struct Emulated {
    slot: TouchSlot,
    at: (f64, f64),
}

/// The fingers that are down, in the order they came, and which of them is the emulated pointer.
#[derive(Debug, Default)]
pub struct Contacts {
    active: Vec<TouchSlot>,
    pointer: Option<Emulated>,
}

/// A finger that was lifted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lifted {
    /// Where the emulated pointer was, when this finger drove it (its button is to be released there).
    pub pointer_at: Option<(f64, f64)>,
}

impl Contacts {
    /// No finger is down.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn is_active(&self, slot: TouchSlot) -> bool {
        self.active.contains(&slot)
    }

    /// A finger came down at `at`; `emulate` makes it the emulated pointer. A slot that is already down (a lost
    /// `up`) is replaced.
    pub fn down(&mut self, slot: TouchSlot, at: (f64, f64), emulate: bool) {
        self.active.retain(|s| *s != slot);
        self.active.push(slot);
        if emulate {
            self.pointer = Some(Emulated { slot, at });
        }
    }

    /// A finger moved to `at`. `None` when that slot is not down (the event is dropped); otherwise whether the
    /// emulated pointer follows it.
    pub fn motion(&mut self, slot: TouchSlot, at: (f64, f64)) -> Option<bool> {
        if !self.is_active(slot) {
            return None;
        }
        match self.pointer.as_mut().filter(|p| p.slot == slot) {
            Some(p) => {
                p.at = at;
                Some(true)
            }
            None => Some(false),
        }
    }

    /// A finger left the screen. `None` when that slot is not down.
    pub fn up(&mut self, slot: TouchSlot) -> Option<Lifted> {
        if !self.is_active(slot) {
            return None;
        }
        self.active.retain(|s| *s != slot);
        let pointer_at = match self.pointer {
            Some(p) if p.slot == slot => self.pointer.take().map(|p| p.at),
            _ => None,
        };
        Some(Lifted { pointer_at })
    }

    /// Every finger is gone without a lift. Returns where the emulated pointer was, if one was running.
    pub fn cancel(&mut self) -> Option<(f64, f64)> {
        self.active.clear();
        self.pointer.take().map(|p| p.at)
    }
}

impl WmState {
    /// Touch events from a backend. Anything that is not a touch event is ignored.
    pub(crate) fn process_touch_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::TouchDown { event } => {
                self.last_input_touch = true;
                // a backend that never announced a touchscreen (a nested window) gets the capability on first use
                self.ensure_touch_capability();
                self.touch_down(event.slot(), normalized::<I, _>(&event), event.time_msec());
            }
            InputEvent::TouchMotion { event } => {
                self.last_input_touch = true;
                self.touch_motion(event.slot(), normalized::<I, _>(&event), event.time_msec())
            }
            InputEvent::TouchUp { event } => {
                self.last_input_touch = true;
                self.touch_up(event.slot(), event.time_msec());
            }
            InputEvent::TouchCancel { event } => {
                self.last_input_touch = true;
                self.touch_cancel(event.time_msec())
            }
            InputEvent::TouchFrame { .. } => self.touch_frame(),
            _ => {}
        }
    }

    /// Whether the pointer image is drawn now (see [`cursor_shown`]).
    pub fn pointer_cursor_shown(&self) -> bool {
        cursor_shown(self.last_input_touch)
    }

    /// The seat has the touch capability: a touchscreen was announced, or a touch arrived from an unannounced source.
    fn ensure_touch_capability(&mut self) {
        if self.seat.get_touch().is_none() {
            self.seat.add_touch();
        }
    }

    /// A touchscreen appeared: the seat gets the touch capability with the first one.
    pub fn touch_device_added(&mut self) {
        self.touch_devices.added();
        self.ensure_touch_capability();
    }

    /// A touchscreen went away: with the last one every contact is cancelled and the seat loses the capability.
    pub fn touch_device_removed(&mut self) {
        if self.touch_devices.removed() {
            let now = self.start.elapsed().as_millis() as u32;
            self.touch_cancel(now);
            if self.seat.get_touch().is_some() {
                self.seat.remove_touch();
            }
        }
    }

    /// The point in the output's logical space for a normalized device position (through the output transform).
    /// Absolute pointer devices use it too, so they land where a touch at the same place lands.
    pub fn touch_position(&self, norm: Norm) -> Option<Point<f64, Logical>> {
        let output = self.output()?;
        let geo = self.space.output_geometry(&output)?;
        let (x, y) = map_touch(norm, output_transform(&output), (geo.loc.x, geo.loc.y), (geo.size.w, geo.size.h));
        Some(Point::from((x, y)))
    }

    /// Whether the client that owns `surface` has a `wl_touch` object.
    fn client_has_touch(&self, surface: &WlSurface) -> bool {
        let Ok(client) = self.dh.get_client(surface.id()) else { return false };
        let mut found = false;
        // the closure runs under the backend's lock: it only looks at the interface name
        let _ = self.dh.backend_handle().with_all_objects_for(client.id(), |object| {
            found |= object.interface().name == "wl_touch";
        });
        found
    }

    /// A finger came down. The first finger of a session raises and focuses the window under it, as a click does.
    pub fn touch_down(&mut self, slot: TouchSlot, norm: Norm, time: u32) {
        let Some(pos) = self.touch_position(norm) else { return };
        if self.contacts.is_active(slot) {
            // the lift of this slot was lost: end it before the new contact starts
            self.touch_up(slot, time);
            self.touch_frame();
        }
        let first = self.contacts.is_empty();
        let under = self.surface_under(pos);
        // only the first contact can start pointer emulation, so later fingers skip the object scan
        let has_touch = !first || under.as_ref().is_some_and(|(surface, _)| self.client_has_touch(surface));
        let emulate = wants_pointer_emulation(first, has_touch);
        self.contacts.down(slot, (pos.x, pos.y), emulate);

        let serial = SERIAL_COUNTER.next_serial();
        // xdg-activation tokens are checked against the serials of real touches
        self.note_input_serial(serial, under.as_ref().map(|(surface, _)| surface));
        if first && !emulate {
            // an emulated press focuses through `pointer_button`
            self.focus_at(pos, serial);
        }
        if let Some(touch) = self.seat.get_touch() {
            touch.down(self, under, &DownEvent { slot, location: pos, serial, time });
        }
        if emulate {
            self.pointer_motion(pos, time);
            self.pointer_button(BTN_LEFT, true, time);
        }
    }

    /// A finger moved. The surface that got the touch down keeps receiving it, whatever is under the finger now.
    pub fn touch_motion(&mut self, slot: TouchSlot, norm: Norm, time: u32) {
        let Some(pos) = self.touch_position(norm) else { return };
        let Some(drives_pointer) = self.contacts.motion(slot, (pos.x, pos.y)) else { return };
        let under = self.surface_under(pos);
        if let Some(touch) = self.seat.get_touch() {
            touch.motion(self, under, &MotionEvent { slot, location: pos, time });
        }
        if drives_pointer {
            self.pointer_motion(pos, time);
        }
    }

    /// A finger left the screen. When it drove the emulated pointer the button is released and the pointer leaves.
    pub fn touch_up(&mut self, slot: TouchSlot, time: u32) {
        let Some(lifted) = self.contacts.up(slot) else { return };
        if let Some(touch) = self.seat.get_touch() {
            touch.up(self, &UpEvent { slot, serial: SERIAL_COUNTER.next_serial(), time });
        }
        if let Some(at) = lifted.pointer_at {
            self.pointer_button(BTN_LEFT, false, time);
            self.pointer_leave(at.into(), time);
        }
    }

    /// The touch stream was cancelled (the compositor or the device took it over): clients get `cancel`, and an
    /// emulated press ends without a click.
    pub fn touch_cancel(&mut self, time: u32) {
        let pointer_at = self.contacts.cancel();
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
        }
        if let Some(at) = pointer_at {
            let pointer = self.seat.get_pointer().expect("pointer");
            // the press keeps its target; moving far away first makes the release land outside the widget
            let location = Point::from((at.0 - CANCEL_OFFSET, at.1 - CANCEL_OFFSET));
            let focus = self.surface_under(at.into());
            pointer.motion(self, focus, &PointerMotion { location, serial: SERIAL_COUNTER.next_serial(), time });
            pointer.frame(self);
            self.pointer_button(BTN_LEFT, false, time);
            self.pointer_leave(at.into(), time);
        }
    }

    /// The end of a group of touch events that belong together.
    pub fn touch_frame(&mut self) {
        if let Some(touch) = self.seat.get_touch() {
            touch.frame(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(n: u32) -> TouchSlot {
        TouchSlot::from(Some(n))
    }

    #[test]
    fn corners_map_to_the_output_corners() {
        assert_eq!(to_logical((0.0, 0.0), (0, 0), (1200, 1920)), (0.0, 0.0));
        assert_eq!(to_logical((1.0, 1.0), (0, 0), (1200, 1920)), (1199.0, 1919.0));
    }

    #[test]
    fn the_middle_maps_to_the_middle() {
        assert_eq!(to_logical((0.5, 0.25), (0, 0), (1200, 1920)), (600.0, 480.0));
    }

    #[test]
    fn an_output_away_from_the_origin_offsets_the_point() {
        assert_eq!(to_logical((0.5, 0.5), (1920, 100), (1000, 800)), (2420.0, 500.0));
        assert_eq!(to_logical((1.0, 1.0), (1920, 100), (1000, 800)), (2919.0, 899.0));
    }

    #[test]
    fn positions_outside_the_device_range_are_pulled_to_the_edge() {
        assert_eq!(to_logical((-0.2, 1.4), (0, 0), (100, 100)), (0.0, 99.0));
        assert_eq!(to_logical((f64::NAN, f64::INFINITY), (0, 0), (100, 100)), (0.0, 0.0));
    }

    #[test]
    fn an_empty_output_maps_to_its_origin() {
        assert_eq!(to_logical((0.7, 0.7), (10, 20), (0, 0)), (10.0, 20.0));
    }

    #[test]
    fn the_output_transform_applies_before_scaling() {
        use smithay::utils::Transform;
        assert_eq!(map_touch((0.25, 0.0), Transform::Normal, (0, 0), (100, 200)), (25.0, 0.0));
        for t in [Transform::_90, Transform::_180, Transform::_270] {
            let frac = crate::transform::normalized_to_desktop_fraction((0.25, 0.0), t);
            assert_eq!(map_touch((0.25, 0.0), t, (0, 0), (100, 200)), to_logical(frac, (0, 0), (100, 200)));
        }
        // a panel corner stays a desktop corner under every quarter turn
        for t in [Transform::_90, Transform::_180, Transform::_270] {
            let (x, y) = map_touch((0.0, 0.0), t, (0, 0), (100, 200));
            assert!((x == 0.0 || x == 99.0) && (y == 0.0 || y == 199.0), "{t:?} -> ({x}, {y})");
        }
    }

    #[test]
    fn an_absolute_pointer_lands_where_a_touch_at_the_same_place_lands() {
        use smithay::utils::Transform;
        // the panel's top-left corner is the desktop's top-right corner when the desktop is turned 90 degrees
        assert_eq!(map_touch((0.0, 0.0), Transform::_90, (0, 0), (1920, 1200)), (1919.0, 0.0));
        // the same fraction on an unrotated output is the plain scaled position
        assert_eq!(map_touch((0.5, 0.5), Transform::Normal, (0, 0), (1920, 1200)), (960.0, 600.0));
    }

    #[test]
    fn only_the_first_contact_of_a_pointer_only_client_emulates() {
        assert!(wants_pointer_emulation(true, false));
        assert!(!wants_pointer_emulation(true, true), "a touch-aware client gets touch events, not a pointer");
        assert!(!wants_pointer_emulation(false, false), "a second finger does not move the pointer");
    }

    #[test]
    fn the_emulated_pointer_follows_the_first_finger_only() {
        let mut c = Contacts::default();
        assert!(c.is_empty());
        c.down(slot(0), (10.0, 10.0), true);
        c.down(slot(1), (50.0, 50.0), false);
        assert_eq!(c.motion(slot(0), (12.0, 14.0)), Some(true));
        assert_eq!(c.motion(slot(1), (60.0, 60.0)), Some(false));
        assert_eq!(c.motion(slot(7), (1.0, 1.0)), None, "a slot that is not down is dropped");
        assert_eq!(c.up(slot(1)), Some(Lifted { pointer_at: None }));
        assert_eq!(c.up(slot(0)), Some(Lifted { pointer_at: Some((12.0, 14.0)) }), "the release is where the finger last was");
        assert!(c.is_empty());
    }

    #[test]
    fn a_lift_releases_the_pointer_once() {
        let mut c = Contacts::default();
        c.down(slot(2), (5.0, 6.0), true);
        assert_eq!(c.up(slot(2)), Some(Lifted { pointer_at: Some((5.0, 6.0)) }));
        assert_eq!(c.up(slot(2)), None, "a second lift of the same slot is dropped");
        assert_eq!(c.cancel(), None, "nothing is left to cancel");
    }

    #[test]
    fn the_pointer_stays_with_the_first_finger_after_it_lifts() {
        let mut c = Contacts::default();
        c.down(slot(0), (1.0, 1.0), true);
        c.down(slot(1), (2.0, 2.0), false);
        c.up(slot(0));
        assert_eq!(c.motion(slot(1), (3.0, 3.0)), Some(false), "the remaining finger does not take over the pointer");
        assert!(!c.is_empty());
    }

    #[test]
    fn a_cancel_clears_every_finger_and_reports_the_pointer() {
        let mut c = Contacts::default();
        c.down(slot(0), (1.0, 1.0), true);
        c.down(slot(1), (2.0, 2.0), false);
        c.motion(slot(0), (9.0, 8.0));
        assert_eq!(c.cancel(), Some((9.0, 8.0)));
        assert!(c.is_empty());
        assert_eq!(c.motion(slot(1), (0.0, 0.0)), None);
    }

    #[test]
    fn a_slot_that_comes_down_again_is_replaced() {
        let mut c = Contacts::default();
        c.down(slot(0), (1.0, 1.0), false);
        c.down(slot(0), (4.0, 4.0), false);
        assert_eq!(c.up(slot(0)), Some(Lifted { pointer_at: None }));
        assert!(c.is_empty(), "the slot is listed once");
    }

    #[test]
    fn the_pointer_image_hides_while_the_last_input_was_a_touch() {
        assert!(cursor_shown(false), "a desktop with no touch input always shows the pointer");
        assert!(!cursor_shown(true));
    }

    #[test]
    fn the_last_announced_touchscreen_is_the_one_that_takes_the_capability() {
        let mut d = TouchDevices::default();
        assert_eq!(d.count(), 0);
        d.added();
        d.added();
        assert!(!d.removed(), "one touchscreen is still present");
        assert!(d.removed(), "the last one went away");
        assert_eq!(d.count(), 0);
    }

    #[test]
    fn a_removal_without_an_announced_device_does_not_go_negative() {
        let mut d = TouchDevices::default();
        assert!(d.removed());
        assert_eq!(d.count(), 0);
        d.added();
        assert!(d.removed(), "the earlier stray removal did not leave a credit behind");
    }
}
