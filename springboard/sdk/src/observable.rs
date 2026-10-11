//! Observable values (Combine `@Published` / `CurrentValueSubject`, `ObservableObject`).
//! Pure and single-threaded: no shell or UI dependency.
//!
//! `set`/`update` fire each subscriber once per change; `sink` returns a [`Subscription`]
//! (`AnyCancellable`) that cancels on drop; `map` and `remove_duplicates` derive new observables.

use std::cell::RefCell;
use std::rc::Rc;

type Callback<T> = Box<dyn FnMut(&T)>;

struct Inner<T> {
    value: T,
    next: u64,
    subs: Vec<(u64, Callback<T>)>,
}

/// A value plus subscribers. Clones share the same value (reference semantics, like a Combine subject).
pub struct Observable<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for Observable<T> {
    fn clone(&self) -> Self {
        Observable { inner: self.inner.clone() }
    }
}

/// `AnyCancellable`: dropping it (or calling `cancel`) removes the subscriber.
pub struct Subscription {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl Subscription {
    pub fn cancel(mut self) {
        if let Some(c) = self.cancel.take() {
            c();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(c) = self.cancel.take() {
            c();
        }
    }
}

/// `store(in:)` target: keeps subscriptions alive until it is dropped or cleared.
#[derive(Default)]
pub struct Cancellables(Vec<Subscription>);

impl Cancellables {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn store(&mut self, s: Subscription) {
        self.0.push(s);
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl Subscription {
    /// `AnyCancellable.store(in:)`.
    pub fn store(self, bag: &mut Cancellables) {
        bag.store(self);
    }
}

impl<T: Clone + 'static> Observable<T> {
    pub fn new(value: T) -> Self {
        Observable { inner: Rc::new(RefCell::new(Inner { value, next: 0, subs: Vec::new() })) }
    }

    /// Current value (`CurrentValueSubject.value`).
    pub fn get(&self) -> T {
        self.inner.borrow().value.clone()
    }

    /// Replaces the value and notifies once. Re-entrant `set` from a callback is not supported
    /// (the nested notification is skipped for the running subscriber set); see docs.
    pub fn set(&self, value: T) {
        self.inner.borrow_mut().value = value;
        self.notify();
    }

    /// Mutates in place and notifies once.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.inner.borrow_mut().value);
        self.notify();
    }

    fn notify(&self) {
        // Take the subscribers out so callbacks may subscribe or cancel without a double borrow.
        let mut subs = std::mem::take(&mut self.inner.borrow_mut().subs);
        let v = self.get();
        for (_, cb) in subs.iter_mut() {
            cb(&v);
        }
        let mut inner = self.inner.borrow_mut();
        let added = std::mem::take(&mut inner.subs);
        subs.extend(added);
        inner.subs = subs;
    }

    /// `sink(receiveValue:)`: delivers the current value immediately (as `CurrentValueSubject`), then each change.
    pub fn sink(&self, mut f: impl FnMut(&T) + 'static) -> Subscription {
        let v = self.get();
        f(&v);
        self.subscribe(f)
    }

    /// Like `sink` but without the initial delivery (`dropFirst()`).
    pub fn subscribe(&self, f: impl FnMut(&T) + 'static) -> Subscription {
        let id = {
            let mut i = self.inner.borrow_mut();
            i.next += 1;
            let id = i.next;
            i.subs.push((id, Box::new(f)));
            id
        };
        let weak = Rc::downgrade(&self.inner);
        Subscription {
            cancel: Some(Box::new(move || {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut i) = rc.try_borrow_mut() {
                        i.subs.retain(|(k, _)| *k != id);
                    }
                }
            })),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner.borrow().subs.len()
    }

    /// `map`: a derived observable that updates when this one does. The returned `Subscription` links them.
    pub fn map<U: Clone + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> (Observable<U>, Subscription) {
        let out = Observable::new(f(&self.get()));
        let o = out.clone();
        let sub = self.subscribe(move |v| o.set(f(v)));
        (out, sub)
    }
}

impl<T: Clone + PartialEq + 'static> Observable<T> {
    /// Sets only when the value differs (`removeDuplicates()` on the input side).
    pub fn set_if_changed(&self, value: T) -> bool {
        if self.inner.borrow().value == value {
            return false;
        }
        self.set(value);
        true
    }

    /// `removeDuplicates`: a derived observable that only fires when the value changes.
    pub fn remove_duplicates(&self) -> (Observable<T>, Subscription) {
        let out = Observable::new(self.get());
        let o = out.clone();
        let sub = self.subscribe(move |v| {
            o.set_if_changed(v.clone());
        });
        (out, sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> (Rc<RefCell<Vec<i32>>>, Rc<RefCell<Vec<i32>>>) {
        let l = Rc::new(RefCell::new(Vec::new()));
        (l.clone(), l)
    }

    #[test]
    fn one_notification_per_change() {
        let o = Observable::new(1);
        let (log, w) = counter();
        let _s = o.sink(move |v| w.borrow_mut().push(*v));
        o.set(2);
        o.update(|v| *v += 1);
        assert_eq!(*log.borrow(), vec![1, 2, 3]);
    }

    #[test]
    fn cancel_on_drop_and_store_in() {
        let o = Observable::new(0);
        let (log, w) = counter();
        let s = o.subscribe(move |v| w.borrow_mut().push(*v));
        o.set(1);
        drop(s);
        o.set(2);
        assert_eq!(*log.borrow(), vec![1]);
        assert_eq!(o.subscriber_count(), 0);

        let mut bag = Cancellables::new();
        o.subscribe(|_| {}).store(&mut bag);
        assert_eq!(o.subscriber_count(), 1);
        bag.clear();
        assert_eq!(o.subscriber_count(), 0);
    }

    #[test]
    fn remove_duplicates_and_map() {
        let o = Observable::new(1);
        let (d, _k) = o.remove_duplicates();
        let (log, w) = counter();
        let _s = d.subscribe(move |v| w.borrow_mut().push(*v));
        o.set(1);
        o.set(2);
        o.set(2);
        assert_eq!(*log.borrow(), vec![2]);
        let (m, _k2) = o.map(|v| v * 10);
        o.set(5);
        assert_eq!(m.get(), 50);
    }
}
