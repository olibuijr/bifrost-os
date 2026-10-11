//! Text input client, shell side (`NativeTextInputContext`, `zwp_text_input_v3` semantics): what the focused app's
//! `TextState` / `CaretRect` / `InputTraits` mean for the keyboard, how the keyboard app's composition becomes
//! `SetMarkedText` / `UnmarkText`, and where a candidate popup goes. Pure functions; `apphost.rs` wires them.
//!
//! The shell is a Wayland client of the host compositor and serves no `wl_seat` text-input of its own, so
//! `zwp_text_input_v3` / `input_method` for third-party Wayland clients has no server to hang on (docs/bifrost-ui.md,
//! "Text input client", gaps). The same mapping applies the day it does: `preedit_string` is `Composition::Preedit`
//! (cursor_begin/end are `sel_start`/`sel_len`), `commit_string` is a `KeyInput::Text`, `delete_surrounding_text` is
//! `ToApp::DeleteSurrounding`, and `set_surrounding_text` / `set_content_type` / `set_cursor_rectangle` come from
//! `TextState` / `InputTraits` / `CaretRect`.
use super::keyboard::Composition;
use bifrost_proto::{InputTraits, ReturnKey, ToApp};

/// The keyboard app's return key label for a field: the nearest of its six (`ReturnKeyType::basic`).
pub fn return_key(t: InputTraits) -> ReturnKey {
    t.return_key.basic()
}

/// The app message for a composition event from the keyboard app.
pub fn to_app(c: Composition) -> ToApp {
    match c {
        Composition::Preedit { text, sel_start, sel_len } => ToApp::SetMarkedText { text, sel_start, sel_len },
        Composition::Unmark { commit } => ToApp::UnmarkText { commit },
    }
}

/// Top-left of a popup `w` x `h` (candidate window, correction bubble) for the caret rectangle `caret` in a screen of
/// `screen_w` x `screen_h`: just below the caret, flipped above when it would not fit, and kept inside the screen
/// horizontally. All values in px.
pub fn popup_origin(caret: (f32, f32, f32, f32), w: f32, h: f32, screen_w: f32, screen_h: f32, gap: f32) -> (f32, f32) {
    let (cx, cy, _, ch) = caret;
    let below = cy + ch + gap;
    let y = if below + h <= screen_h { below } else { (cy - gap - h).max(0.0) };
    let x = cx.min(screen_w - w).max(0.0);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::ReturnKeyType;

    #[test]
    fn traits_choose_the_nearest_return_key() {
        let t = |r| InputTraits { return_key: r, ..InputTraits::default() };
        assert_eq!(return_key(t(ReturnKeyType::Search)), ReturnKey::Search);
        assert_eq!(return_key(t(ReturnKeyType::Continue)), ReturnKey::Next);
        assert_eq!(return_key(InputTraits::default()), ReturnKey::Return);
    }

    #[test]
    fn keyboard_composition_becomes_marked_text_messages() {
        assert_eq!(
            to_app(Composition::Preedit { text: "にほ".into(), sel_start: 0, sel_len: 6 }),
            ToApp::SetMarkedText { text: "にほ".into(), sel_start: 0, sel_len: 6 }
        );
        assert_eq!(to_app(Composition::Unmark { commit: true }), ToApp::UnmarkText { commit: true });
    }

    #[test]
    fn a_popup_sits_below_the_caret_flips_above_and_stays_on_screen() {
        let caret = (100.0, 200.0, 1.0, 20.0);
        assert_eq!(popup_origin(caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 224.0));
        // not enough room below: above the caret
        assert_eq!(popup_origin((100.0, 560.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 476.0));
        // near the right edge: pushed left
        assert_eq!(popup_origin((780.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0).0, 650.0);
    }
}
