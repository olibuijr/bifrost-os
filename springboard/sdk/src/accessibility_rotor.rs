//! Custom rotors for apps: named lists of elements the screen reader steps through ("Unread", "Errors"), beside the standard
//! rotors. Mirrors `UIAccessibilityCustomRotor`. An app declares its rotors with [`Ctx::set_custom_rotors`]; each item is a
//! `Node::id` the app reported in `controls`, in the order the user should reach them. The shell keeps the last list, cleaned
//! to the caps in `bifrost_ui::ax_rotor`, and the screen reader walks it.

use crate::proto::ToShell;
use crate::runtime::Ctx;

/// A custom rotor: the name the user hears and the `Node::id`s to step through, in order.
pub use bifrost_ui::ax::AxCustomRotor as CustomRotor;

impl Ctx {
    /// Replaces the app's custom rotors. Call it when the set of rotors or their items changes; an empty list removes them all.
    pub fn set_custom_rotors(&self, rotors: Vec<CustomRotor>) {
        self.send_to_shell(ToShell::AxRotors(rotors));
    }
}
