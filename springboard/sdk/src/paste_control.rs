//! `UIPasteControl` for BifrOSt apps: the paste read behind a paste button. The button's look and enabled state come from
//! `bifrost_ui::paste_control`; this module reads the pasteboard when the person taps the button.
//!
//! No access prompt is shown. The tap is the consent, and the read takes only the items that conform to the control's
//! accepted types, so the app never sees data it did not ask for. Calling [`paste`] needs the [`Tap`] the toolkit creates
//! for the press, so a read cannot happen without a press.
//!
//! ```ignore
//! let config = Configuration::accepting(&[pasteboard::STRING]);
//! let enabled = paste_control::is_enabled(&NativePasteboard::general(), &config);
//! // on the tap:
//! let items = paste_control::paste(&NativePasteboard::general(), &config, tap);
//! ```

pub use bifrost_ui::paste_control::{Configuration, Display, Tap};

use crate::pasteboard::NativePasteboard;
use crate::pasteboard_item::PasteboardItem;

/// Whether the control is enabled: the board holds an item of an accepted type.
pub fn is_enabled(board: &NativePasteboard, config: &Configuration) -> bool {
    config.is_enabled(&board.types())
}

/// The items the control places into the app for a tap: the board's items conforming to an accepted type, in board order.
/// Nothing is returned for a board without an accepted type; the control is then disabled and never sends a tap.
pub fn paste(board: &NativePasteboard, config: &Configuration, _tap: Tap) -> Vec<PasteboardItem> {
    let types: Vec<&str> = config.accepted_types.iter().map(String::as_str).collect();
    board.read_items(&types)
}
