//! Whether a drag pans content. On a desktop the scroll wheel, the trackpad and the scroll bar scroll; a mouse press and
//! drag never pans a list or a page (it drags the item out or extends the selection, or does nothing). A finger pans, and
//! so does everything on a tablet, whatever made the drag. The form factor is `crate::form_factor`, chosen at run time.
//!
//! The shell tells every app what produced a plain touch ([`TouchSource`], on `Input::Touch`). The SDK also records it for
//! the touch it is delivering ([`set_source`]), so the kit's scroll views and [`Scroller`](crate::scroll::Scroller) decide
//! at the press without each app passing it down; an app that pans on its own asks [`pans_on_drag`].

use crate::form_factor::FormFactor;
use std::cell::Cell;

/// What produced a plain touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TouchSource {
    /// A mouse or trackpad button press.
    Pointer,
    /// A finger on a touchscreen; also a synthesized press, which stands for one, and the drag the shell emulates for a
    /// wheel notch, which is a scroll.
    #[default]
    Finger,
}

impl TouchSource {
    /// The source of a touch frame sample: an indirect touch is the pointer; a finger or a pencil is direct.
    pub fn of_touch_type(kind: crate::touch::TouchType) -> Self {
        match kind {
            crate::touch::TouchType::Indirect => Self::Pointer,
            _ => Self::Finger,
        }
    }
}

/// The rule, on a given form factor: a drag pans on a tablet, and on a desktop only when a finger made it.
pub fn pans_on_drag_for(form: FormFactor, source: TouchSource) -> bool {
    form.is_tablet() || source == TouchSource::Finger
}

/// The rule on the form factor of this process (`FormFactor::current`).
pub fn pans_on_drag(source: TouchSource) -> bool {
    pans_on_drag_for(form(), source)
}

thread_local! {
    static SOURCE: Cell<TouchSource> = const { Cell::new(TouchSource::Finger) };
    static FORM: Cell<Option<FormFactor>> = const { Cell::new(None) };
}

fn form() -> FormFactor {
    FORM.with(Cell::get).unwrap_or_else(FormFactor::current)
}

/// Records what produced the touch being delivered now (the SDK calls it before it hands the touch to the app).
pub fn set_source(source: TouchSource) {
    SOURCE.with(|s| s.set(source));
}

/// What produced the touch being delivered now; a finger until the SDK says otherwise.
pub fn source() -> TouchSource {
    SOURCE.with(Cell::get)
}

/// Whether a press delivered now starts a pan if it drags: `pans_on_drag(source())`.
pub fn pans_now() -> bool {
    pans_on_drag(source())
}

/// Runs `f` as if the machine had form factor `form` and the touch came from `source`, for tests; both are restored after.
pub fn with_touch<R>(form: FormFactor, source: TouchSource, f: impl FnOnce() -> R) -> R {
    let (old_form, old_source) = (FORM.with(|c| c.replace(Some(form))), SOURCE.with(|c| c.replace(source)));
    let r = f();
    FORM.with(|c| c.set(old_form));
    SOURCE.with(|c| c.set(old_source));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_pans_only_for_a_finger_and_a_tablet_for_both() {
        assert!(!pans_on_drag_for(FormFactor::Desktop, TouchSource::Pointer));
        assert!(pans_on_drag_for(FormFactor::Desktop, TouchSource::Finger));
        assert!(pans_on_drag_for(FormFactor::Tablet, TouchSource::Pointer));
        assert!(pans_on_drag_for(FormFactor::Tablet, TouchSource::Finger));
    }

    #[test]
    fn the_recorded_source_and_form_are_restored_after_a_scoped_run() {
        assert_eq!(source(), TouchSource::Finger);
        with_touch(FormFactor::Desktop, TouchSource::Pointer, || {
            assert!(!pans_now());
            set_source(TouchSource::Finger);
            assert!(pans_now());
        });
        assert_eq!(source(), TouchSource::Finger);
    }

    #[test]
    fn a_touch_frame_sample_maps_to_its_source() {
        use crate::touch::TouchType;
        assert_eq!(TouchSource::of_touch_type(TouchType::Indirect), TouchSource::Pointer);
        assert_eq!(TouchSource::of_touch_type(TouchType::Direct), TouchSource::Finger);
        assert_eq!(TouchSource::of_touch_type(TouchType::Pencil), TouchSource::Finger);
    }
}
