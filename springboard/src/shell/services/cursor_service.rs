//! The `cursor` service: apps push and pop pointer shapes, hide the pointer and hide it until it moves. The state is
//! the `bifrost_ui::cursor::Cursors` model (a stack of shapes with a hide count). The pointer code in `bare/wayland.rs`
//! reads it through [`pointer_shape`] and [`pointer_visible`] and calls [`pointer_moved`] on every move.
//!
//! Methods (params are JSON; a shape is its `Kind::name`, for example `"i_beam"`):
//! `push(shape)`, `pop()` (returns the popped shape or null), `set(shape)`, `current()` (the shape name),
//! `depth()`, `hide()`, `unhide()`, `set_hidden_until_mouse_moves(bool)`.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::cursor::{Cursors, Kind};
use std::sync::Mutex;

/// The one cursor stack of the desktop. `None` until the first call, because `Cursors::new` is not `const`.
static STATE: Mutex<Option<Cursors>> = Mutex::new(None);

/// Runs `f` on the stack. A poisoned lock still gives the stack back: a cursor is never worth a panic.
pub fn with<R>(f: impl FnOnce(&mut Cursors) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Cursors::new))
}

/// The shape an app pushed or set, or `None` when no app has one (the shell then shows its own edge shapes).
pub fn pointer_shape() -> Option<Kind> {
    with(|c| (c.depth() > 0).then(|| c.current()))
}

/// Whether the pointer should be drawn at all.
pub fn pointer_visible() -> bool {
    with(|c| c.is_visible())
}

/// The pointer moved: a cursor that was hidden until the next move shows again.
pub fn pointer_moved() {
    with(|c| c.mouse_moved());
}

/// Applies one cursor method to `c`. Returns the JSON result, or an error message for a bad call.
pub fn apply(c: &mut Cursors, method: &str, params: &Json) -> Result<Json, String> {
    let shape = || -> Result<Kind, String> {
        let name = params.as_str().ok_or("the shape must be a string")?;
        Kind::from_name(name).ok_or_else(|| format!("no cursor shape {name}"))
    };
    let flag = || match params {
        Json::Bool(b) => Ok(*b),
        _ => Err("the flag must be true or false".to_string()),
    };
    match method {
        "push" => {
            c.push(shape()?);
            Ok(Json::Null)
        }
        "pop" => Ok(c.pop().map_or(Json::Null, |k| Json::Str(k.name().into()))),
        "set" => {
            c.set(shape()?);
            Ok(Json::Null)
        }
        "current" => Ok(Json::Str(c.current().name().into())),
        "depth" => Ok(Json::Num(c.depth() as f64)),
        "hide" => {
            c.hide();
            Ok(Json::Null)
        }
        "unhide" => {
            c.unhide();
            Ok(Json::Null)
        }
        "set_hidden_until_mouse_moves" => {
            c.set_hidden_until_mouse_moves(flag()?);
            Ok(Json::Null)
        }
        _ => Err(format!("cursor has no method {method}")),
    }
}

pub struct CursorService;

impl SystemService for CursorService {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        with(|c| apply(c, method, params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> Json {
        Json::Str(name.into())
    }

    #[test]
    fn push_and_pop_follow_the_stack_and_pop_returns_the_shape() {
        let mut c = Cursors::new();
        assert_eq!(apply(&mut c, "push", &s("i_beam")), Ok(Json::Null));
        apply(&mut c, "push", &s("drag_copy")).unwrap();
        assert_eq!(apply(&mut c, "current", &Json::Null), Ok(s("drag_copy")));
        assert_eq!(apply(&mut c, "depth", &Json::Null), Ok(Json::Num(2.0)));
        assert_eq!(apply(&mut c, "pop", &Json::Null), Ok(s("drag_copy")));
        assert_eq!(apply(&mut c, "current", &Json::Null), Ok(s("i_beam")));
        apply(&mut c, "pop", &Json::Null).unwrap();
        assert_eq!(apply(&mut c, "pop", &Json::Null), Ok(Json::Null), "an empty pop changes nothing");
    }

    #[test]
    fn set_replaces_the_top_and_hide_flags_work() {
        let mut c = Cursors::new();
        apply(&mut c, "set", &s("zoom_in")).unwrap();
        assert_eq!(apply(&mut c, "depth", &Json::Null), Ok(Json::Num(1.0)));
        apply(&mut c, "hide", &Json::Null).unwrap();
        assert!(!c.is_visible());
        apply(&mut c, "unhide", &Json::Null).unwrap();
        assert!(c.is_visible());
        apply(&mut c, "set_hidden_until_mouse_moves", &Json::Bool(true)).unwrap();
        assert!(!c.is_visible());
        c.mouse_moved();
        assert!(c.is_visible());
    }

    #[test]
    fn bad_calls_are_errors_not_panics() {
        let mut c = Cursors::new();
        assert!(apply(&mut c, "push", &s("text")).is_err(), "the protocol spelling is not a service name");
        assert!(apply(&mut c, "push", &Json::Null).is_err());
        assert!(apply(&mut c, "set_hidden_until_mouse_moves", &s("yes")).is_err());
        assert!(apply(&mut c, "wiggle", &Json::Null).is_err());
        assert_eq!(c.depth(), 0);
    }

    #[test]
    fn the_shared_stack_is_what_the_shell_reads() {
        with(|c| *c = Cursors::new());
        assert_eq!(pointer_shape(), None, "no app pushed a shape");
        assert!(pointer_visible());
        with(|c| apply(c, "push", &s("pointing_hand")).map(|_| ()).unwrap());
        assert_eq!(pointer_shape(), Some(Kind::PointingHand));
        with(|c| apply(c, "set_hidden_until_mouse_moves", &Json::Bool(true)).map(|_| ()).unwrap());
        assert!(!pointer_visible());
        pointer_moved();
        assert!(pointer_visible());
        with(|c| *c = Cursors::new());
    }
}
