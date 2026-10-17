//! Publishers for what a control or the shell reports: the bridge between the event callbacks an app already
//! receives and the `PassthroughSubject` and `Observable` types of [`crate::observable`]. It covers the two jobs of the
//! reference framework that joins its control and notification classes to the reactive layer: control-event publishers
//! (a button's click, a slider's change, a field's edits) and notification publishers (one publisher per name).
//!
//! ```ignore
//! let mut clicks = ControlEvents::<()>::new();
//! let _bag = clicks.publisher(ControlEvent::Clicked).sink(|_| save());
//! // in the control's action callback:
//! clicks.fire(ControlEvent::Clicked, ());
//!
//! let mut hub = NotificationHub::new();
//! let _s = hub.publisher(notify_bus::APPEARANCE_CHANGE).sink(|n| relayout(&n.name));
//! hub.bind(token, notify_bus::APPEARANCE_CHANGE);              // after notify_bus::register answered
//! // App::service_event: if let Some(n) = notify_bus::notified(service, event, &body) { hub.deliver(&n) }
//! ```
//! A [`ControlProperty`] is the property form: it holds the current value, delivers it on subscribe, and sends only
//! real changes. Single-threaded like the rest of `observable`; the callbacks run on the app's event thread.

use std::collections::BTreeMap;

use crate::notify_bus::Notified;
use crate::observable::{Observable, PassthroughSubject, Subscription};

/// The events a control can publish, named for the user's action rather than a platform class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControlEvent {
    /// A press and release inside the control (a button click, a menu item pick).
    Clicked,
    /// The control's value changed (slider, switch, segmented control, stepper).
    ValueChanged,
    /// The text of an editable control changed.
    EditingChanged,
    /// An editable control gained focus.
    EditingDidBegin,
    /// An editable control lost focus, or the user pressed return.
    EditingDidEnd,
}

/// One publisher per [`ControlEvent`] for a control that reports values of type `T`.
pub struct ControlEvents<T> {
    subjects: BTreeMap<ControlEvent, PassthroughSubject<T>>,
}

impl<T: 'static> Default for ControlEvents<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> ControlEvents<T> {
    pub fn new() -> Self {
        ControlEvents { subjects: BTreeMap::new() }
    }

    /// The publisher for `event`; the same subject is returned on every call, so subscribers share it.
    pub fn publisher(&mut self, event: ControlEvent) -> PassthroughSubject<T> {
        self.subjects.entry(event).or_default().clone()
    }

    /// Reports `event` with its value to every subscriber of that event; others hear nothing.
    pub fn fire(&self, event: ControlEvent, value: T) {
        if let Some(subject) = self.subjects.get(&event) {
            subject.send(value);
        }
    }

    /// Live subscribers on `event`.
    pub fn subscriber_count(&self, event: ControlEvent) -> usize {
        self.subjects.get(&event).map_or(0, PassthroughSubject::subscriber_count)
    }
}

/// A control property as a publisher: it holds the current value, delivers it to a new subscriber at once, and sends
/// a change only when the value differs from the one it holds.
pub struct ControlProperty<T: Clone + PartialEq + 'static> {
    value: Observable<T>,
}

impl<T: Clone + PartialEq + 'static> ControlProperty<T> {
    pub fn new(initial: T) -> Self {
        ControlProperty { value: Observable::new(initial) }
    }

    pub fn get(&self) -> T {
        self.value.get()
    }

    /// Records the control's new value. Returns whether it differed (and so was published).
    pub fn set(&self, value: T) -> bool {
        self.value.set_if_changed(value)
    }

    /// Delivers the current value now, then each change.
    pub fn sink(&self, f: impl FnMut(&T) + 'static) -> Subscription {
        self.value.sink(f)
    }

    /// Delivers each change after this call, not the current value.
    pub fn changes(&self, f: impl FnMut(&T) + 'static) -> Subscription {
        self.value.subscribe(f)
    }
}

/// Notification publishers: one subject per notification name, fed by the tokens the app registered with
/// `notify_bus`.
#[derive(Default)]
pub struct NotificationHub {
    by_name: BTreeMap<String, PassthroughSubject<Notified>>,
    tokens: BTreeMap<u32, String>,
}

impl NotificationHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// The publisher for `name`, shared by every caller.
    pub fn publisher(&mut self, name: &str) -> PassthroughSubject<Notified> {
        self.by_name.entry(name.to_string()).or_default().clone()
    }

    /// Records that `token` (from `notify_bus::register`) delivers `name`.
    pub fn bind(&mut self, token: u32, name: &str) {
        self.tokens.insert(token, name.to_string());
    }

    /// Forgets `token` (after `notify_bus::cancel`).
    pub fn unbind(&mut self, token: u32) {
        self.tokens.remove(&token);
    }

    /// Sends a delivered notification to the publisher of its name. A token that was never bound falls back to the
    /// name the shell reported. Returns whether any subscriber was reached.
    pub fn deliver(&self, event: &Notified) -> bool {
        let name = self.tokens.get(&event.token).unwrap_or(&event.name);
        match self.by_name.get(name) {
            Some(subject) if subject.subscriber_count() > 0 => {
                subject.send(event.clone());
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_control_event_reaches_only_the_subscribers_of_that_event() {
        let mut events = ControlEvents::<i32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let changes = Rc::clone(&seen);
        let _a = events.publisher(ControlEvent::ValueChanged).sink(move |v| changes.borrow_mut().push(*v));
        let clicks = Rc::new(RefCell::new(0));
        let c = Rc::clone(&clicks);
        let _b = events.publisher(ControlEvent::Clicked).sink(move |_| *c.borrow_mut() += 1);
        events.fire(ControlEvent::ValueChanged, 3);
        events.fire(ControlEvent::ValueChanged, 5);
        events.fire(ControlEvent::EditingChanged, 9);
        assert_eq!(*seen.borrow(), vec![3, 5]);
        assert_eq!(*clicks.borrow(), 0, "a value change is not a click");
        events.fire(ControlEvent::Clicked, 0);
        assert_eq!(*clicks.borrow(), 1);
    }

    #[test]
    fn dropping_the_subscription_stops_delivery() {
        let mut events = ControlEvents::<()>::new();
        let n = Rc::new(RefCell::new(0));
        let counter = Rc::clone(&n);
        let sub = events.publisher(ControlEvent::Clicked).sink(move |_| *counter.borrow_mut() += 1);
        assert_eq!(events.subscriber_count(ControlEvent::Clicked), 1);
        events.fire(ControlEvent::Clicked, ());
        drop(sub);
        events.fire(ControlEvent::Clicked, ());
        assert_eq!(*n.borrow(), 1);
        assert_eq!(events.subscriber_count(ControlEvent::Clicked), 0);
    }

    #[test]
    fn a_control_property_delivers_its_value_then_only_real_changes() {
        let prop = ControlProperty::new("a".to_string());
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&seen);
        let _s = prop.sink(move |v| log.borrow_mut().push(v.clone()));
        assert!(!prop.set("a".into()), "same value: nothing published");
        assert!(prop.set("b".into()));
        assert!(prop.set("c".into()));
        assert_eq!(*seen.borrow(), vec!["a", "b", "c"]);
        assert_eq!(prop.get(), "c");
        let later = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&later);
        let _t = prop.changes(move |v| log.borrow_mut().push(v.clone()));
        prop.set("d".into());
        assert_eq!(*later.borrow(), vec!["d"], "changes skips the current value");
    }

    #[test]
    fn a_notification_goes_to_the_publisher_of_its_bound_name() {
        let mut hub = NotificationHub::new();
        let hits = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&hits);
        let _s = hub.publisher("system.appearance.change").sink(move |n| log.borrow_mut().push(n.token));
        let other = Rc::new(RefCell::new(0));
        let o = Rc::clone(&other);
        let _t = hub.publisher("system.network.change").sink(move |_| *o.borrow_mut() += 1);
        hub.bind(7, "system.appearance.change");
        assert!(hub.deliver(&Notified { token: 7, name: "ignored".into() }));
        assert_eq!(*hits.borrow(), vec![7]);
        assert_eq!(*other.borrow(), 0);
        // an unbound token falls back to the name the shell reported
        assert!(hub.deliver(&Notified { token: 9, name: "system.network.change".into() }));
        assert_eq!(*other.borrow(), 1);
        hub.unbind(7);
        assert!(!hub.deliver(&Notified { token: 7, name: "system.volume.mount".into() }), "nobody listens to that name");
    }
}
