//! The custom rotors of the running apps, as the screen reader cycles them: "All", the standard rotors, then each rotor the
//! app declared with `ToShell::AxRotors`. The cycle is the shared model `bifrost_ui::ax_rotor::RotorCycle`; this module only
//! reads it from the app's host.

use crate::shell::Shell;
use bifrost_ui::ax_rotor::RotorCycle;

impl Shell {
    /// The rotor cycle of the app in slot `slot` (its index in `ext`). An empty or unknown slot has only the standard rotors.
    #[allow(dead_code)] // the reader does not switch to app rotors yet; the desk and the `ax` listing will read this
    pub(in crate::shell) fn app_rotor_cycle(&self, slot: usize) -> RotorCycle {
        let custom = self.ext.get(slot).and_then(|h| h.as_ref()).map(|h| h.custom_rotors.clone()).unwrap_or_default();
        RotorCycle::new(custom)
    }
}
