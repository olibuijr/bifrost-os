//! Observable values (Combine `@Published` / `CurrentValueSubject`, `ObservableObject`).
//! Pure and single-threaded: no shell or UI dependency.
//!
//! `set`/`update` fire each subscriber once per change; `sink` returns a [`Subscription`]
//! (`AnyCancellable`) that cancels on drop; `map` and `remove_duplicates` derive new observables.
//! A `set` made from inside a subscriber is queued and delivered after the running change, in order.
//! `with_observation_tracking` records the observables its closure reads and calls `on_change` once when
//! one of them changes; `Observable::observations` pulls the changes in order instead of calling back.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

type Callback<T> = Box<dyn FnMut(&T)>;

struct Inner<T> {
    value: T,
    next: u64,
    subs: Vec<(u64, Callback<T>)>,
    /// Changes waiting for delivery. A `set` made during delivery queues here (see `drain`).
    queue: VecDeque<T>,
    /// True while `drain` runs; the subscribers are taken out of `subs` for that time.
    draining: bool,
    /// Subscriptions cancelled during a delivery pass; removed when the pass ends.
    cancelled: Vec<u64>,
}

/// Attaches one change callback to an observable (recorded when the observable is read in a tracking scope).
type Attach = Rc<dyn Fn(Box<dyn FnMut()>) -> Subscription>;

thread_local! {
    /// One frame per active `with_observation_tracking` call: the observables read so far, keyed by identity.
    static TRACKING: RefCell<Vec<Vec<(usize, Attach)>>> = RefCell::new(Vec::new());
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

/// Runs `apply` and records every observable it reads. When one of those observables next changes,
/// `on_change` runs once and the tracking ends (the other reads are dropped). Reads made by `on_change`
/// itself are not tracked. If `apply` reads no observable, `on_change` never runs.
///
/// Limits against the Observation framework: the closure returns only `apply`'s value (no token or
/// options), there is no property-level macro, and tracking is not re-armed after it fires.
pub fn with_observation_tracking<R>(apply: impl FnOnce() -> R, on_change: impl FnOnce() + 'static) -> R {
    TRACKING.with(|t| t.borrow_mut().push(Vec::new()));
    let result = apply();
    let reads = TRACKING.with(|t| t.borrow_mut().pop()).unwrap_or_default();
    let fired: Rc<RefCell<Option<Box<dyn FnOnce()>>>> =
        Rc::new(RefCell::new(Some(Box::new(on_change) as Box<dyn FnOnce()>)));
    let subs: Rc<RefCell<Vec<Subscription>>> = Rc::new(RefCell::new(Vec::new()));
    for (_, attach) in reads {
        let fired = fired.clone();
        let subs_for_change = subs.clone();
        let sub = attach(Box::new(move || {
            let cb = fired.borrow_mut().take();
            if let Some(cb) = cb {
                // The first change ends the tracking: release every other subscription, then notify.
                let others = std::mem::take(&mut *subs_for_change.borrow_mut());
                drop(others);
                cb();
            }
        }));
        subs.borrow_mut().push(sub);
    }
    result
}

impl<T: Clone + 'static> Observable<T> {
    pub fn new(value: T) -> Self {
        Observable {
            inner: Rc::new(RefCell::new(Inner {
                value,
                next: 0,
                subs: Vec::new(),
                queue: VecDeque::new(),
                draining: false,
                cancelled: Vec::new(),
            })),
        }
    }

    /// Current value (`CurrentValueSubject.value`). Inside `with_observation_tracking` the read is recorded.
    pub fn get(&self) -> T {
        self.record_read();
        self.peek()
    }

    /// Current value without recording a tracking read (used by the internal delivery paths).
    fn peek(&self) -> T {
        self.inner.borrow().value.clone()
    }

    fn record_read(&self) {
        let key = Rc::as_ptr(&self.inner) as *const () as usize;
        TRACKING.with(|t| {
            let mut frames = t.borrow_mut();
            let Some(frame) = frames.last_mut() else { return };
            if frame.iter().any(|(k, _)| *k == key) {
                return;
            }
            let o = self.clone();
            let attach: Attach = Rc::new(move |mut cb: Box<dyn FnMut()>| o.subscribe(move |_: &T| cb()));
            frame.push((key, attach));
        });
    }

    /// Replaces the value and notifies once. A `set` made from a subscriber is not lost: the value
    /// changes at once and its notification is delivered after the running one, in order.
    pub fn set(&self, value: T) {
        {
            let mut i = self.inner.borrow_mut();
            i.value = value.clone();
            i.queue.push_back(value);
            if i.draining {
                return;
            }
        }
        self.drain();
    }

    /// Mutates in place and notifies once.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        {
            let mut i = self.inner.borrow_mut();
            f(&mut i.value);
            let v = i.value.clone();
            i.queue.push_back(v);
            if i.draining {
                return;
            }
        }
        self.drain();
    }

    /// Delivers the queued changes in order. The subscribers are taken out of `inner` while callbacks run,
    /// so callbacks may subscribe, cancel, or set the value without a double borrow.
    fn drain(&self) {
        let mut subs = {
            let mut i = self.inner.borrow_mut();
            i.draining = true;
            std::mem::take(&mut i.subs)
        };
        loop {
            let next = self.inner.borrow_mut().queue.pop_front();
            let Some(v) = next else { break };
            for (id, cb) in subs.iter_mut() {
                let id = *id;
                if self.inner.borrow().cancelled.contains(&id) {
                    continue;
                }
                cb(&v);
            }
        }
        let mut i = self.inner.borrow_mut();
        i.draining = false;
        let cancelled = std::mem::take(&mut i.cancelled);
        let added = std::mem::take(&mut i.subs);
        subs.extend(added);
        subs.retain(|(id, _)| !cancelled.contains(id));
        i.subs = subs;
    }

    /// `sink(receiveValue:)`: delivers the current value immediately (as `CurrentValueSubject`), then each change.
    pub fn sink(&self, mut f: impl FnMut(&T) + 'static) -> Subscription {
        let v = self.peek();
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
                        if i.draining {
                            i.cancelled.push(id);
                        } else {
                            i.subs.retain(|(k, _)| *k != id);
                        }
                    }
                }
            })),
        }
    }

    /// Number of live subscribers. Reads 0 for the duration of a delivery pass (subscribers are taken out).
    pub fn subscriber_count(&self) -> usize {
        self.inner.borrow().subs.len()
    }

    /// Pull-based sequence of the changes made after this call, oldest first. Dropping it stops recording.
    /// This is the synchronous counterpart of the Observation framework's `Observations` sequence.
    pub fn observations(&self) -> Observations<T> {
        let pending = Rc::new(RefCell::new(VecDeque::new()));
        let p = pending.clone();
        let sub = self.subscribe(move |v: &T| p.borrow_mut().push_back(v.clone()));
        Observations { pending, _sub: sub }
    }

    /// `map`: a derived observable that updates when this one does. The returned `Subscription` links them.
    pub fn map<U: Clone + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> (Observable<U>, Subscription) {
        let out = Observable::new(f(&self.peek()));
        let o = out.clone();
        let sub = self.subscribe(move |v| o.set(f(v)));
        (out, sub)
    }
}

/// Pull-based change sequence created by [`Observable::observations`].
pub struct Observations<T> {
    pending: Rc<RefCell<VecDeque<T>>>,
    _sub: Subscription,
}

impl<T> Observations<T> {
    /// The oldest change not yet taken, or `None` when nothing has changed since the last call.
    pub fn try_next(&mut self) -> Option<T> {
        self.pending.borrow_mut().pop_front()
    }

    /// Changes waiting to be taken.
    pub fn pending_len(&self) -> usize {
        self.pending.borrow().len()
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
        let out = Observable::new(self.peek());
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
    use std::cell::Cell;

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

    #[test]
    fn set_from_a_subscriber_is_delivered_after_the_running_change() {
        let o = Observable::new(0);
        let inner = o.clone();
        let (log, w) = counter();
        let _s = o.subscribe(move |v| {
            w.borrow_mut().push(*v);
            if *v == 1 {
                inner.set(2);
            }
        });
        o.set(1);
        assert_eq!(*log.borrow(), vec![1, 2]);
        assert_eq!(o.get(), 2);
    }

    #[test]
    fn cancelling_another_subscriber_during_a_change_is_durable() {
        let o = Observable::new(0);
        let (log, w) = counter();
        // The canceller is subscribed first, so it runs before the subscriber it cancels.
        let slot: Rc<RefCell<Option<Subscription>>> = Rc::new(RefCell::new(None));
        let s2 = slot.clone();
        let _a = o.subscribe(move |v| {
            if *v == 1 {
                let taken = s2.borrow_mut().take();
                if let Some(b) = taken {
                    b.cancel();
                }
            }
        });
        *slot.borrow_mut() = Some(o.subscribe(move |v| w.borrow_mut().push(*v)));
        o.set(1);
        o.set(2);
        assert_eq!(*log.borrow(), Vec::<i32>::new());
        assert_eq!(o.subscriber_count(), 1);
    }

    #[test]
    fn tracking_fires_once_for_a_value_that_was_read() {
        let a = Observable::new(1);
        let b = Observable::new(10);
        let fired = Rc::new(Cell::new(0));
        let f = fired.clone();
        let read = with_observation_tracking(|| a.get() + 1, move || f.set(f.get() + 1));
        assert_eq!(read, 2);
        b.set(11);
        assert_eq!(fired.get(), 0, "an observable the closure did not read must not fire");
        a.set(2);
        assert_eq!(fired.get(), 1);
        a.set(3);
        assert_eq!(fired.get(), 1, "tracking ends after the first change");
    }

    #[test]
    fn tracking_fires_when_any_of_several_reads_changes() {
        let a = Observable::new(1);
        let b = Observable::new(2);
        let fired = Rc::new(Cell::new(0));
        let f = fired.clone();
        let sum = with_observation_tracking(|| a.get() + b.get(), move || f.set(f.get() + 1));
        assert_eq!(sum, 3);
        b.set(5);
        assert_eq!(fired.get(), 1);
        a.set(9);
        assert_eq!(fired.get(), 1);
    }

    #[test]
    fn tracking_without_reads_never_fires() {
        let a = Observable::new(1);
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();
        let _ = with_observation_tracking(|| 0, move || f.set(true));
        a.set(2);
        assert!(!fired.get());
    }

    #[test]
    fn observations_pull_changes_in_order() {
        let o = Observable::new(0);
        o.set(9);
        let mut seq = o.observations();
        assert_eq!(seq.pending_len(), 0, "changes before the call are not included");
        o.set(1);
        o.set(2);
        assert_eq!(seq.pending_len(), 2);
        assert_eq!(seq.try_next(), Some(1));
        assert_eq!(seq.try_next(), Some(2));
        assert_eq!(seq.try_next(), None);
        drop(seq);
        assert_eq!(o.subscriber_count(), 0);
    }
}
