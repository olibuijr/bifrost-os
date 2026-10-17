//! Locate: the model behind the device and people locator. It holds the account's devices with their last positions,
//! the lost mode and the queued device commands, location sharing grants, friends and notify rules, and the locator's
//! links. The model has no window, network or clock code: every function takes the time it needs.
//!
//! - `devices`: devices, position fixes, battery, lost mode and the command queue.
//! - `share`: location sharing grants with their durations and address checks.
//! - `friends`: friend distances and the rules that notify on arrival or departure.
//! - `links`: `locate:` links, and driving directions through the maps app.
//! - `panel`: the window's model (sections, device list, first-run sheet, Share My Location switch).
//! - `control`: the control methods (`locate.locate`, `locate.play_sound`, `locate.share`) parsed from JSON.
//!
//! The window and the app's entry point are in `src/window.rs` and `src/main.rs`.

pub mod control;
pub mod devices;
pub mod friends;
pub mod links;
pub mod pairing;
pub mod panel;
pub mod reporting;
pub mod sealing;
pub mod share;
