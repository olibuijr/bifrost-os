//! Process event records and the subscription set that filters them: process signal, working-directory and
//! root-directory changes, interprocess events (trace, remote thread, suspend and resume), and mount, unmount and
//! remount events. A subscription set holds the kinds an app wants and a list of muted source processes. This is
//! the model only; no shell path produces these records yet, so every status that uses it is partial.

use std::collections::BTreeSet;

/// The kinds an app can subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    Signal,
    Chdir,
    Chroot,
    Trace,
    RemoteThreadCreate,
    SuspendResume,
    Mount,
    Unmount,
    Remount,
}

impl EventKind {
    pub const ALL: [EventKind; 9] = [
        EventKind::Signal,
        EventKind::Chdir,
        EventKind::Chroot,
        EventKind::Trace,
        EventKind::RemoteThreadCreate,
        EventKind::SuspendResume,
        EventKind::Mount,
        EventKind::Unmount,
        EventKind::Remount,
    ];

    /// Stable lower-case name, used in logs and in the control socket.
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Signal => "signal",
            EventKind::Chdir => "chdir",
            EventKind::Chroot => "chroot",
            EventKind::Trace => "trace",
            EventKind::RemoteThreadCreate => "remote-thread-create",
            EventKind::SuspendResume => "suspend-resume",
            EventKind::Mount => "mount",
            EventKind::Unmount => "unmount",
            EventKind::Remount => "remount",
        }
    }
}

/// One event record. Process fields are process ids; `source` is the process that acted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Signal { source: u32, target: u32, signal: i32 },
    Chdir { pid: u32, path: String },
    Chroot { pid: u32, path: String },
    Trace { source: u32, target: u32 },
    RemoteThreadCreate { source: u32, target: u32 },
    SuspendResume { source: u32, target: u32, suspended: bool },
    Mount { path: String, fs_type: String },
    Unmount { path: String },
    Remount { path: String },
}

impl ProcessEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            ProcessEvent::Signal { .. } => EventKind::Signal,
            ProcessEvent::Chdir { .. } => EventKind::Chdir,
            ProcessEvent::Chroot { .. } => EventKind::Chroot,
            ProcessEvent::Trace { .. } => EventKind::Trace,
            ProcessEvent::RemoteThreadCreate { .. } => EventKind::RemoteThreadCreate,
            ProcessEvent::SuspendResume { .. } => EventKind::SuspendResume,
            ProcessEvent::Mount { .. } => EventKind::Mount,
            ProcessEvent::Unmount { .. } => EventKind::Unmount,
            ProcessEvent::Remount { .. } => EventKind::Remount,
        }
    }

    /// The process that caused the event. Mount events have none.
    pub fn source(&self) -> Option<u32> {
        match self {
            ProcessEvent::Signal { source, .. }
            | ProcessEvent::Trace { source, .. }
            | ProcessEvent::RemoteThreadCreate { source, .. }
            | ProcessEvent::SuspendResume { source, .. } => Some(*source),
            ProcessEvent::Chdir { pid, .. } | ProcessEvent::Chroot { pid, .. } => Some(*pid),
            ProcessEvent::Mount { .. } | ProcessEvent::Unmount { .. } | ProcessEvent::Remount { .. } => None,
        }
    }
}

/// The kinds an app listens for, and the source processes whose events it does not want.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventSubscriptions {
    kinds: BTreeSet<EventKind>,
    muted: BTreeSet<u32>,
}

impl EventSubscriptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds kinds. Returns how many were not subscribed before.
    pub fn subscribe(&mut self, kinds: &[EventKind]) -> usize {
        kinds.iter().filter(|k| self.kinds.insert(**k)).count()
    }

    /// Removes one kind. Returns whether it was subscribed.
    pub fn unsubscribe(&mut self, kind: EventKind) -> bool {
        self.kinds.remove(&kind)
    }

    pub fn unsubscribe_all(&mut self) {
        self.kinds.clear();
    }

    /// The subscribed kinds, in declaration order.
    pub fn subscriptions(&self) -> Vec<EventKind> {
        self.kinds.iter().copied().collect()
    }

    /// Drops every event whose source is this process. Returns whether it was not muted before.
    pub fn mute_process(&mut self, pid: u32) -> bool {
        self.muted.insert(pid)
    }

    pub fn unmute_process(&mut self, pid: u32) -> bool {
        self.muted.remove(&pid)
    }

    pub fn unmute_all(&mut self) {
        self.muted.clear();
    }

    pub fn is_muted(&self, pid: u32) -> bool {
        self.muted.contains(&pid)
    }

    /// True when the event's kind is subscribed and its source process is not muted. An event with no source
    /// (a mount) is never dropped by a mute.
    pub fn accepts(&self, event: &ProcessEvent) -> bool {
        self.kinds.contains(&event.kind()) && !event.source().is_some_and(|pid| self.muted.contains(&pid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(source: u32, target: u32) -> ProcessEvent {
        ProcessEvent::Signal { source, target, signal: 15 }
    }

    #[test]
    fn subscribe_counts_only_new_kinds() {
        let mut s = EventSubscriptions::new();
        assert_eq!(s.subscribe(&[EventKind::Signal, EventKind::Chdir]), 2);
        assert_eq!(s.subscribe(&[EventKind::Signal, EventKind::Mount]), 1);
        assert_eq!(s.subscriptions(), vec![EventKind::Signal, EventKind::Chdir, EventKind::Mount]);
    }

    #[test]
    fn unsubscribed_kinds_are_not_accepted() {
        let mut s = EventSubscriptions::new();
        s.subscribe(&[EventKind::Signal]);
        assert!(s.accepts(&signal(7, 8)));
        assert!(!s.accepts(&ProcessEvent::Chdir { pid: 7, path: "/tmp".into() }));
        assert!(s.unsubscribe(EventKind::Signal));
        assert!(!s.accepts(&signal(7, 8)));
        assert!(!s.unsubscribe(EventKind::Signal));
    }

    #[test]
    fn muting_a_source_drops_its_events_only() {
        let mut s = EventSubscriptions::new();
        s.subscribe(&EventKind::ALL);
        assert!(s.mute_process(42));
        assert!(!s.mute_process(42));
        assert!(!s.accepts(&signal(42, 1)));
        assert!(s.accepts(&signal(7, 42)), "a muted target does not mute events from other sources");
        s.unmute_process(42);
        assert!(s.accepts(&signal(42, 1)));
    }

    #[test]
    fn mount_events_have_no_source_and_ignore_mutes() {
        let mut s = EventSubscriptions::new();
        s.subscribe(&[EventKind::Mount]);
        s.mute_process(0);
        let mount = ProcessEvent::Mount { path: "/mnt/usb".into(), fs_type: "vfat".into() };
        assert_eq!(mount.source(), None);
        assert!(s.accepts(&mount));
    }

    #[test]
    fn unsubscribe_all_and_unmute_all_clear_the_sets() {
        let mut s = EventSubscriptions::new();
        s.subscribe(&EventKind::ALL);
        s.mute_process(3);
        s.unsubscribe_all();
        s.unmute_all();
        assert!(s.subscriptions().is_empty());
        assert!(!s.is_muted(3));
    }

    #[test]
    fn events_report_their_kind_and_source() {
        let trace = ProcessEvent::Trace { source: 5, target: 6 };
        assert_eq!(trace.kind(), EventKind::Trace);
        assert_eq!(trace.source(), Some(5));
        let chroot = ProcessEvent::Chroot { pid: 9, path: "/jail".into() };
        assert_eq!(chroot.kind(), EventKind::Chroot);
        assert_eq!(chroot.source(), Some(9));
        assert_eq!(EventKind::SuspendResume.name(), "suspend-resume");
    }
}
