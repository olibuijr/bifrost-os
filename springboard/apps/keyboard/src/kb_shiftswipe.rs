//! bifrost keyboard quick slides: dragging from shift onto a letter types it
//! capitalised, dragging from 123 onto a symbol types it and returns to the
//! letters page. Pure decision logic; `keyboard.rs` applies the result.

/// The key a slide started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Shift,
    Numbers,
}

/// What releasing the finger should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// not a slide: handle the release normally
    Normal,
    /// type the key under the finger capitalised, leaving shift alone
    TypeUpper,
    /// type the key under the finger, then go back to the letters page
    TypeThenLetters,
    /// the finger came back to where it started: the page stays as it is
    Stay,
}

/// `target_is_char`: the released key is a letter or symbol; `same_key`: it is
/// the key the touch started on.
pub fn on_release(origin: Option<Origin>, target_is_char: bool, same_key: bool) -> Action {
    match origin {
        Some(Origin::Shift) if target_is_char && !same_key => Action::TypeUpper,
        Some(Origin::Numbers) if target_is_char => Action::TypeThenLetters,
        Some(Origin::Numbers) if same_key => Action::Stay,
        _ => Action::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slides_resolve() {
        assert_eq!(on_release(Some(Origin::Shift), true, false), Action::TypeUpper);
        assert_eq!(on_release(Some(Origin::Shift), false, true), Action::Normal);
        assert_eq!(on_release(Some(Origin::Numbers), true, false), Action::TypeThenLetters);
        assert_eq!(on_release(Some(Origin::Numbers), false, true), Action::Stay);
        assert_eq!(on_release(None, true, false), Action::Normal);
    }
}
