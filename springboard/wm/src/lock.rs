//! `ext-session-lock-v1`: while a client holds the lock, only its lock surface is drawn and takes input. A lock whose
//! client dies stays locked (the protocol's guarantee): the screen stays black until a new lock client unlocks.

use smithay::{
    delegate_session_lock,
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::SERIAL_COUNTER,
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};

use crate::state::WmState;

impl SessionLockHandler for WmState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState { &mut self.lock_state }

    fn lock(&mut self, confirmation: SessionLocker) {
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

delegate_session_lock!(WmState);
