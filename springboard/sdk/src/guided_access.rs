//! Guided Access for apps: a single-app session the user starts from the shell, with app-defined restrictions. Mirrors
//! `UIAccessibility.requestGuidedAccessSession` and the `UIGuidedAccessRestrictionDelegate` callbacks.
//!
//! An app calls [`Ctx::request_guided_access`] with its restrictions; the shell starts the session only when the user has
//! turned Guided Access on in Settings > Accessibility. The answer and every later change arrive at the `App` through
//! `App::guided_access`, as a [`GuidedAccessChange`]. The shell owns the session state: the app never sees a restriction it did
//! not declare, and a session ends when the user leaves it or the app exits.

use crate::proto::ToShell;
pub use crate::proto::GuidedRestriction;
use crate::runtime::Ctx;

/// What the shell tells the app about Guided Access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuidedAccessChange {
    /// A session started (`true`) or ended (`false`). A request the shell refused is answered with `false`.
    Session(bool),
    /// The user allowed (`true`) or denied (`false`) the restriction with this identifier during the session.
    Restriction { id: String, allowed: bool },
}

impl Ctx {
    /// Asks the shell for a Guided Access session with these restrictions. The answer is `GuidedAccessChange::Session`.
    pub fn request_guided_access(&self, restrictions: &[GuidedRestriction]) {
        self.send_to_shell(ToShell::GuidedAccessRequest { on: true, restrictions: restrictions.to_vec() });
    }

    /// Ends this app's Guided Access session, if it has one. The answer is `GuidedAccessChange::Session(false)`.
    pub fn end_guided_access(&self) {
        self.send_to_shell(ToShell::GuidedAccessRequest { on: false, restrictions: Vec::new() });
    }
}
