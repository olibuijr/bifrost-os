//! `ObservableObject` parity (Combine): a model object announces its changes through one aggregate publisher,
//! `objectWillChange`, rather than through one publisher per property.
//!
//! [`ObservableObjectPublisher`] is the aggregate. `send` notifies its subscribers, `sink` receives the
//! notifications, and [`ObservableObjectPublisher::watch`] connects an [`Observable`] property so that each change
//! of that property sends one notification. Partial against the official protocol: the official publisher emits
//! before the object changes, while here the property already holds the new value when `sink` runs. Pure and
//! single-threaded, like [`Observable`].

use super::observable::{Observable, PassthroughSubject, Subscription};

/// `ObservableObjectPublisher`: the aggregate change publisher of one model object. Clones share the subscribers.
#[derive(Clone, Default)]
pub struct ObservableObjectPublisher {
    subject: PassthroughSubject<()>,
}

impl ObservableObjectPublisher {
    /// `init()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `send()`: notifies each current subscriber once.
    pub fn send(&self) {
        self.subject.send(());
    }

    /// `sink(receiveValue:)`: receives each later notification. Dropping the returned subscription stops it.
    pub fn sink(&self, mut f: impl FnMut() + 'static) -> Subscription {
        self.subject.sink(move |_| f())
    }

    /// Sends one notification per change of `value`, for as long as the returned subscription lives.
    pub fn watch<T: Clone + 'static>(&self, value: &Observable<T>) -> Subscription {
        let subject = self.subject.clone();
        value.subscribe(move |_| subject.send(()))
    }
}

/// The `ObservableObject` protocol: a type that exposes its aggregate change publisher.
pub trait ObservableObject {
    /// `objectWillChange`. Implementations return the publisher their properties are `watch`ed by.
    fn object_will_change(&self) -> &ObservableObjectPublisher;
}

/// Declares a model struct whose stored properties are each an [`Observable`], all linked to one aggregate
/// publisher. This is the declarative stand-in for an observable macro: Rust has no property-level macro without a
/// derive crate, so the properties are listed once here.
///
/// `observable_model! { pub struct Name { pub field: Type = initial_value, ... } }` generates:
/// - a struct whose fields are `Observable<Type>` (read with `get`, write with `set` or `update`),
/// - `Name::new()`, which sets each field to its initial value,
/// - an [`ObservableObject`] implementation, whose `object_will_change` fires once for every change of any field.
///
/// The struct keeps the subscriptions that connect each field to the publisher, so those links live as long as the
/// model. The names `changes` and `links` are reserved for the generated bookkeeping fields. Unlike the official
/// macro there is no per-property opt-out (no ignored properties) and no computed-property rule.
#[macro_export]
macro_rules! observable_model {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($fvis:vis $field:ident : $ty:ty = $init:expr),* $(,)? }) => {
        $(#[$meta])*
        $vis struct $name {
            changes: $crate::observable_object::ObservableObjectPublisher,
            links: $crate::observable::Cancellables,
            $($fvis $field: $crate::observable::Observable<$ty>,)*
        }

        impl $name {
            /// Creates the model with every property at its initial value.
            $vis fn new() -> Self {
                let changes = $crate::observable_object::ObservableObjectPublisher::new();
                let mut links = $crate::observable::Cancellables::new();
                $(
                    let $field: $crate::observable::Observable<$ty> = $crate::observable::Observable::new($init);
                    links.store(changes.watch(&$field));
                )*
                Self { changes, links, $($field,)* }
            }
        }

        impl $crate::observable_object::ObservableObject for $name {
            fn object_will_change(&self) -> &$crate::observable_object::ObservableObjectPublisher {
                &self.changes
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observable::Cancellables;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// A model with two observed properties, linked to one aggregate publisher.
    struct Person {
        changes: ObservableObjectPublisher,
        name: Observable<String>,
        age: Observable<u32>,
        links: Cancellables,
    }

    impl Person {
        fn new() -> Self {
            let changes = ObservableObjectPublisher::new();
            let name = Observable::new("John".to_string());
            let age = Observable::new(24u32);
            let mut links = Cancellables::new();
            links.store(changes.watch(&name));
            links.store(changes.watch(&age));
            Person { changes, name, age, links }
        }
    }

    impl ObservableObject for Person {
        fn object_will_change(&self) -> &ObservableObjectPublisher {
            &self.changes
        }
    }

    #[test]
    fn send_notifies_every_subscriber_once_per_send() {
        let p = ObservableObjectPublisher::new();
        let (a, b) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (ca, cb) = (a.clone(), b.clone());
        let _sa = p.sink(move || ca.set(ca.get() + 1));
        let _sb = p.sink(move || cb.set(cb.get() + 1));
        p.send();
        p.send();
        assert_eq!((a.get(), b.get()), (2, 2));
    }

    #[test]
    fn sink_receives_only_notifications_sent_after_it_subscribed() {
        let p = ObservableObjectPublisher::new();
        p.send();
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let _s = p.sink(move || c.set(c.get() + 1));
        p.send();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn dropping_the_subscription_stops_notifications() {
        let p = ObservableObjectPublisher::new();
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let s = p.sink(move || c.set(c.get() + 1));
        p.send();
        drop(s);
        p.send();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn watch_sends_one_notification_per_change_of_the_property() {
        let p = ObservableObjectPublisher::new();
        let value = Observable::new(1);
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let _w = p.watch(&value);
        let _s = p.sink(move || c.set(c.get() + 1));
        value.set(2);
        value.update(|v| *v += 1);
        assert_eq!((n.get(), value.get()), (2, 3));
    }

    #[test]
    fn watch_stops_when_its_link_is_dropped() {
        let p = ObservableObjectPublisher::new();
        let value = Observable::new(0);
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let _s = p.sink(move || c.set(c.get() + 1));
        let link = p.watch(&value);
        value.set(1);
        drop(link);
        value.set(2);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn a_model_object_announces_each_property_change_on_its_aggregate() {
        let person = Person::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (s, name, age) = (seen.clone(), person.name.clone(), person.age.clone());
        let _sub = person.object_will_change().sink(move || s.borrow_mut().push((name.get(), age.get())));
        person.name.set("Jane".to_string());
        person.age.set(25);
        assert_eq!(*seen.borrow(), vec![("Jane".to_string(), 24), ("Jane".to_string(), 25)]);
        assert_eq!(person.links.len(), 2);
    }

    observable_model! {
        struct Session {
            name: String = String::from("guest"),
            count: u32 = 0,
        }
    }

    #[test]
    fn observable_model_starts_at_the_declared_values() {
        let s = Session::new();
        assert_eq!(s.name.get(), "guest");
        assert_eq!(s.count.get(), 0);
        assert_eq!(s.links.len(), 2);
    }

    #[test]
    fn observable_model_announces_one_change_per_field_write() {
        let s = Session::new();
        let fired = Rc::new(Cell::new(0));
        let f = fired.clone();
        let _sink = s.object_will_change().sink(move || f.set(f.get() + 1));
        s.count.set(3);
        s.name.set(String::from("olibuijr"));
        assert_eq!(fired.get(), 2);
        assert_eq!((s.name.get(), s.count.get()), (String::from("olibuijr"), 3));
    }

    #[test]
    fn observable_model_stops_announcing_when_the_publisher_sink_is_dropped() {
        let s = Session::new();
        let fired = Rc::new(Cell::new(0));
        let f = fired.clone();
        let sink = s.object_will_change().sink(move || f.set(f.get() + 1));
        s.count.set(1);
        drop(sink);
        s.count.set(2);
        assert_eq!(fired.get(), 1);
    }
}
