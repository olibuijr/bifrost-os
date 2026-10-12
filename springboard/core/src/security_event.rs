//! Security events for sockets, pseudoterminals, the system clock and kernel extensions (std only).
//!
//! Each event has a kind and a phase. A phase says whether the event asks for a verdict before it happens
//! (`Authorize`) or reports that it already happened (`Notify`). Each kind belongs to one category, and only some kinds
//! have both phases. A subscription names the (kind, phase) pairs a client wants, and answers whether a given event
//! is wanted. The module reads no kernel feed, delivers no events and records no verdicts; it is the contract the
//! event source would use. The category names are BifrOSt's own.

use std::collections::BTreeSet;
use std::fmt;

/// The group a kind belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// Unix domain socket bind and connect, and suspend, resume or shutdown of another process's sockets.
    Socket,
    /// Pseudoterminal grant and close.
    Pseudoterminal,
    /// Changes to the system time.
    Clock,
    /// Kernel extension load and unload.
    Kernel,
}

/// What the event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    UnixSocketBind,
    UnixSocketConnect,
    SocketSuspendResume,
    PtyGrant,
    PtyClose,
    SetTime,
    KextLoad,
    KextUnload,
}

/// Whether the event asks for a verdict first or reports a change that already happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Authorize,
    Notify,
}

/// One security event: its kind and phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    pub kind: Kind,
    pub phase: Phase,
}

/// Why a subscription was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscribeError {
    /// The kind has no event for this phase, for example a pseudoterminal grant cannot be authorized.
    NoSuchEvent,
}

impl fmt::Display for SubscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscribeError::NoSuchEvent => write!(f, "no such security event for this phase"),
        }
    }
}

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: [Kind; 8] = [
        Kind::UnixSocketBind,
        Kind::UnixSocketConnect,
        Kind::SocketSuspendResume,
        Kind::PtyGrant,
        Kind::PtyClose,
        Kind::SetTime,
        Kind::KextLoad,
        Kind::KextUnload,
    ];

    /// The category this kind belongs to.
    pub fn category(self) -> Category {
        match self {
            Kind::UnixSocketBind | Kind::UnixSocketConnect | Kind::SocketSuspendResume => Category::Socket,
            Kind::PtyGrant | Kind::PtyClose => Category::Pseudoterminal,
            Kind::SetTime => Category::Clock,
            Kind::KextLoad | Kind::KextUnload => Category::Kernel,
        }
    }

    /// The phases this kind has. Pseudoterminal events and kernel extension unload are notify-only.
    pub fn phases(self) -> &'static [Phase] {
        match self {
            Kind::PtyGrant | Kind::PtyClose | Kind::KextUnload => &[Phase::Notify],
            _ => &[Phase::Authorize, Phase::Notify],
        }
    }
}

impl Event {
    /// Whether the event is valid: its kind has this phase.
    pub fn exists(self) -> bool {
        self.kind.phases().contains(&self.phase)
    }

    /// Whether the source must wait for a verdict on this event before the action proceeds.
    pub fn needs_verdict(self) -> bool {
        self.phase == Phase::Authorize
    }
}

/// The set of events one client has subscribed to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subscription {
    events: BTreeSet<Event>,
}

impl Subscription {
    /// An empty subscription, which wants nothing.
    pub fn new() -> Self {
        Subscription::default()
    }

    /// Subscribes to one event. Subscribing twice is harmless.
    pub fn subscribe(&mut self, kind: Kind, phase: Phase) -> Result<(), SubscribeError> {
        let ev = Event { kind, phase };
        if !ev.exists() {
            return Err(SubscribeError::NoSuchEvent);
        }
        self.events.insert(ev);
        Ok(())
    }

    /// Subscribes to every valid (kind, phase) pair in a category. Returns how many pairs were newly added.
    pub fn subscribe_category(&mut self, category: Category) -> usize {
        let mut added = 0;
        for kind in Kind::ALL.iter().copied().filter(|k| k.category() == category) {
            for &phase in kind.phases() {
                if self.events.insert(Event { kind, phase }) {
                    added += 1;
                }
            }
        }
        added
    }

    /// Removes one event. Returns whether it had been subscribed.
    pub fn unsubscribe(&mut self, kind: Kind, phase: Phase) -> bool {
        self.events.remove(&Event { kind, phase })
    }

    /// Whether this subscription wants the event.
    pub fn wants(&self, event: Event) -> bool {
        self.events.contains(&event)
    }

    /// Number of (kind, phase) pairs subscribed to.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_belongs_to_its_category() {
        assert_eq!(Kind::UnixSocketBind.category(), Category::Socket);
        assert_eq!(Kind::SocketSuspendResume.category(), Category::Socket);
        assert_eq!(Kind::PtyGrant.category(), Category::Pseudoterminal);
        assert_eq!(Kind::SetTime.category(), Category::Clock);
        assert_eq!(Kind::KextUnload.category(), Category::Kernel);
    }

    #[test]
    fn pseudoterminal_and_kext_unload_are_notify_only() {
        assert_eq!(Kind::PtyGrant.phases(), &[Phase::Notify]);
        assert_eq!(Kind::KextUnload.phases(), &[Phase::Notify]);
        assert_eq!(Kind::KextLoad.phases(), &[Phase::Authorize, Phase::Notify]);
        assert!(!Event { kind: Kind::PtyClose, phase: Phase::Authorize }.exists());
    }

    #[test]
    fn subscribing_to_a_phase_the_kind_lacks_is_refused() {
        let mut s = Subscription::new();
        assert_eq!(s.subscribe(Kind::PtyGrant, Phase::Authorize), Err(SubscribeError::NoSuchEvent));
        assert!(s.subscribe(Kind::PtyGrant, Phase::Notify).is_ok());
        assert_eq!(s.len(), 1);
        assert_eq!(format!("{}", SubscribeError::NoSuchEvent), "no such security event for this phase");
    }

    #[test]
    fn subscription_wants_only_the_pairs_it_holds() {
        let mut s = Subscription::new();
        s.subscribe(Kind::SetTime, Phase::Authorize).unwrap();
        assert!(s.wants(Event { kind: Kind::SetTime, phase: Phase::Authorize }));
        assert!(!s.wants(Event { kind: Kind::SetTime, phase: Phase::Notify }));
        assert!(!s.is_empty());
    }

    #[test]
    fn subscribing_to_a_category_adds_every_valid_pair_once() {
        let mut s = Subscription::new();
        // Socket: bind (2) + connect (2) + suspend-resume (2) = 6 pairs.
        assert_eq!(s.subscribe_category(Category::Socket), 6);
        // Pseudoterminal: grant (1) + close (1) = 2 pairs.
        assert_eq!(s.subscribe_category(Category::Pseudoterminal), 2);
        assert_eq!(s.subscribe_category(Category::Socket), 0);
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn only_authorize_events_need_a_verdict_and_unsubscribe_reports_removal() {
        let auth = Event { kind: Kind::UnixSocketConnect, phase: Phase::Authorize };
        let note = Event { kind: Kind::UnixSocketConnect, phase: Phase::Notify };
        assert!(auth.needs_verdict());
        assert!(!note.needs_verdict());
        let mut s = Subscription::new();
        s.subscribe(Kind::UnixSocketConnect, Phase::Authorize).unwrap();
        assert!(s.unsubscribe(Kind::UnixSocketConnect, Phase::Authorize));
        assert!(!s.unsubscribe(Kind::UnixSocketConnect, Phase::Authorize));
        assert!(s.is_empty());
    }
}
