//! Locate: the model behind the device and people locator. It holds the account's devices with their last positions,
//! the lost mode and the queued device commands, location sharing grants, friends and notify rules, and the locator's
//! links. The model has no window, network or clock code: every function takes the time it needs.
//!
//! - `devices`: devices, position fixes, battery, lost mode and the command queue.
//! - `share`: location sharing grants with their durations and address checks.
//! - `friends`: friend distances and the rules that notify on arrival or departure.
//! - `links`: `locate:` links, and driving directions through the maps app.

pub mod devices;
pub mod friends;
pub mod links;
pub mod share;
