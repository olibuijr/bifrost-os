//! `ext-session-lock-v1`: while a client holds the lock, only its lock surface is drawn and takes input. A lock whose
//! client dies stays locked (the protocol's guarantee): the screen stays black until a new lock client unlocks.
//!
//! Only privileged clients (the shell, `privileged.rs`) can see the manager global: a sandboxed app can neither lock nor
//! unlock the session. While a live client holds the lock a second `lock` request is refused (`finished`), so nothing
//! can replace the lock screen or its surface.

use smithay::{
    delegate_session_lock,
    output::Output,
    reexports::wayland_server::{protocol::wl_output::WlOutput, Resource},
    utils::SERIAL_COUNTER,
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};

use crate::state::WmState;

impl SessionLockHandler for WmState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState { &mut self.lock_state }

    fn lock(&mut self, confirmation: SessionLocker) {
        let holder_alive = self.lock_holder.as_ref().is_some_and(|holder| holder.is_alive());
        if !accepts_lock(self.locked, holder_alive) {
            // dropping the locker tells the client `finished`; the lock screen in place is untouched
            return;
        }
        self.lock_holder = Some(confirmation.ext_session_lock().clone());
        self.locked = true;
        self.lock_surface = None;
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        // the next frame draws nothing but the lock (black until its surface arrives)
        confirmation.lock();
    }

    fn unlock(&mut self) {
        self.locked = false;
        self.lock_holder = None;
        self.lock_surface = None;
        self.focus(None, SERIAL_COUNTER.next_serial());
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let size = Output::from_resource(&output).or_else(|| self.output()).and_then(|o| crate::transform::output_size(&o));
        if let Some((w, h)) = size {
            surface.with_pending_state(|s| s.size = Some((w as u32, h as u32).into()));
        }
        surface.send_configure();
        let wl = surface.wl_surface().clone();
        self.lock_surface = Some(surface);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(wl), SERIAL_COUNTER.next_serial());
        }
    }
}

/// Whether a new `lock` request is taken: the session is not locked, or it is locked by a client that died (the new
/// client then recovers the session, and may unlock it). A live holder is never replaced.
pub fn accepts_lock(locked: bool, holder_alive: bool) -> bool { !locked || !holder_alive }

delegate_session_lock!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_not_stolen_but_a_dead_holders_lock_is_recovered() {
        assert!(accepts_lock(false, false), "an unlocked session takes the lock");
        assert!(!accepts_lock(true, true), "a live holder keeps the lock; the second request gets `finished`");
        assert!(accepts_lock(true, false), "a lock whose client died can be taken over by a new lock client");
    }
}
