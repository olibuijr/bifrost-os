//! Exclusive input ownership for apps (the open options of a device client). Which requester may take an input device
//! away from the rest of the system: the shell grabs the keyboards, pointers and touchscreens it drives, so that
//! their events reach the shell alone. An app never grabs a device. Power buttons and lid switches are never grabbed,
//! because the system service owns suspend and shutdown.
//!
//! Rules of this model:
//! - An app requester is always refused.
//! - The shell may grab a device unless the device has a power key or a lid switch ([`Roles::grabbable`]).

use springboard::bare::input::Roles;

/// Who asks for the grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requester {
    /// An app: apps never own an input device.
    App,
    /// The desktop shell.
    Shell,
}

/// Why a grab is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Apps never grab a device.
    AppRequester,
    /// Power buttons and lid switches stay shared with the system service.
    SystemDevice,
}

/// Decides whether `requester` may grab a device with these roles.
pub fn may_grab(roles: Roles, requester: Requester) -> Result<(), Refusal> {
    match requester {
        Requester::App => Err(Refusal::AppRequester),
        Requester::Shell if !roles.grabbable() => Err(Refusal::SystemDevice),
        Requester::Shell => Ok(()),
    }
}
