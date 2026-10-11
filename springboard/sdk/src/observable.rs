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

/// `combineLatest` for two values: a derived observable holding `(a, b)` that updates whenever either
/// source changes. Both sources always have a value, so the pair is available at once. The returned
/// `Cancellables` keeps both links alive; clearing it or dropping it detaches the pair.
pub fn combine_latest<A, B>(a: &Observable<A>, b: &Observable<B>) -> (Observable<(A, B)>, Cancellables)
where
    A: Clone + 'static,
    B: Clone + 'static,
{
    let out = Observable::new((a.get(), b.get()));
    let mut links = Cancellables::new();
    let (o1, b1) = (out.clone(), b.clone());
    a.subscribe(move |av| o1.set((av.clone(), b1.get()))).store(&mut links);
    let (o2, a2) = (out.clone(), a.clone());
    b.subscribe(move |bv| o2.set((a2.get(), bv.clone()))).store(&mut links);
    (out, links)
}

struct PassInner<T> {
    next: u64,
    subs: Vec<(u64, Callback<T>)>,
    /// Subscribers cancelled while a `send` is delivering; applied when the outermost send returns.
    cancelled: Vec<u64>,
    /// Number of `send` calls in progress.
    depth: usize,
}

/// `PassthroughSubject`: broadcasts each `send` to the current subscribers and keeps no value, so a
/// subscriber only receives what is sent after it subscribed. Clones share the subscriber list.
pub struct PassthroughSubject<T> {
    inner: Rc<RefCell<PassInner<T>>>,
}

impl<T> Clone for PassthroughSubject<T> {
    fn clone(&self) -> Self {
        PassthroughSubject { inner: self.inner.clone() }
    }
}

impl<T: 'static> Default for PassthroughSubject<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> PassthroughSubject<T> {
    pub fn new() -> Self {
        let inner = PassInner { next: 0, subs: Vec::new(), cancelled: Vec::new(), depth: 0 };
        PassthroughSubject { inner: Rc::new(RefCell::new(inner)) }
    }

    /// `send(_:)`: delivers `value` to each subscriber in subscription order. A subscriber cancelled
    /// during the send is skipped for the rest of it. Subscribers are not borrowed during delivery,
    /// so a callback may subscribe, cancel or send again.
    pub fn send(&self, value: T) {
        let mut subs = {
            let mut i = self.inner.borrow_mut();
            i.depth += 1;
            std::mem::take(&mut i.subs)
        };
        for (id, cb) in subs.iter_mut() {
            if self.inner.borrow().cancelled.contains(&*id) {
                continue;
            }
            cb(&value);
        }
        let mut i = self.inner.borrow_mut();
        i.depth -= 1;
        subs.extend(std::mem::take(&mut i.subs));
        if i.depth == 0 {
            let cancelled = std::mem::take(&mut i.cancelled);
            subs.retain(|(id, _)| !cancelled.contains(id));
        }
        i.subs = subs;
    }

    /// `sink(receiveValue:)`: delivers each later `send`. Dropping the returned `Subscription` (or
    /// calling `cancel`) removes the subscriber.
    pub fn sink(&self, f: impl FnMut(&T) + 'static) -> Subscription {
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
                        if i.depth > 0 {
                            i.cancelled.push(id);
                        } else {
                            i.subs.retain(|(k, _)| *k != id);
                        }
                    }
                }
            })),
        }
    }

    /// `map`: a derived subject that sends `f` of each value. The returned `Subscription` links them.
    pub fn map<U: 'static>(&self, f: impl Fn(&T) -> U + 'static) -> (PassthroughSubject<U>, Subscription) {
        let out = PassthroughSubject::new();
        let o = out.clone();
        let sub = self.sink(move |v| o.send(f(v)));
        (out, sub)
    }
}

impl<T: Clone + 'static> PassthroughSubject<T> {
    /// `filter(_:)`: a derived subject that forwards only the values `pred` accepts. The returned
    /// `Subscription` links it to this subject.
    pub fn filter(&self, pred: impl Fn(&T) -> bool + 'static) -> (PassthroughSubject<T>, Subscription) {
        let out = PassthroughSubject::new();
        let o = out.clone();
        let sub = self.sink(move |v| {
            if pred(v) {
                o.send(v.clone());
            }
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
    fn passthrough_delivers_only_later_sends_in_subscription_order() {
        let p = PassthroughSubject::<i32>::new();
        p.send(0);
        let (log, w) = counter();
        let w2 = w.clone();
        let _a = p.sink(move |v| w.borrow_mut().push(*v));
        let _b = p.sink(move |v| w2.borrow_mut().push(*v * 10));
        p.send(1);
        p.send(2);
        assert_eq!(*log.borrow(), vec![1, 10, 2, 20]);
    }

    #[test]
    fn passthrough_cancel_during_send_skips_the_cancelled_subscriber() {
        let p = PassthroughSubject::<i32>::new();
        let slot: Rc<RefCell<Option<Subscription>>> = Rc::new(RefCell::new(None));
        let slot2 = slot.clone();
        let (log_a, wa) = counter();
        let _a = p.sink(move |v| {
            wa.borrow_mut().push(*v);
            let taken = slot2.borrow_mut().take();
            if let Some(s) = taken {
                s.cancel();
            }
        });
        let (log_b, wb) = counter();
        let b = p.sink(move |v| wb.borrow_mut().push(*v));
        *slot.borrow_mut() = Some(b);
        p.send(1);
        p.send(2);
        assert_eq!(*log_a.borrow(), vec![1, 2]);
        assert!(log_b.borrow().is_empty());
    }

    #[test]
    fn passthrough_filter_and_map_forward_derived_values() {
        let p = PassthroughSubject::<i32>::new();
        let (evens, _k) = p.filter(|v| v % 2 == 0);
        let (tens, _m) = p.map(|v| v * 10);
        let (log, w) = counter();
        let _s = evens.sink(move |v| w.borrow_mut().push(*v));
        let (log2, w2) = counter();
        let _t = tens.sink(move |v| w2.borrow_mut().push(*v));
        for v in 1..=4 {
            p.send(v);
        }
        assert_eq!(*log.borrow(), vec![2, 4]);
        assert_eq!(*log2.borrow(), vec![10, 20, 30, 40]);
    }

    #[test]
    fn passthrough_dropping_the_subscription_stops_delivery() {
        let p = PassthroughSubject::<i32>::new();
        let (log, w) = counter();
        let s = p.sink(move |v| w.borrow_mut().push(*v));
        p.send(1);
        drop(s);
        p.send(2);
        assert_eq!(*log.borrow(), vec![1]);
    }

    #[test]
    fn combine_latest_emits_once_per_change_and_detaches_on_clear() {
        let a = Observable::new(1);
        let b = Observable::new("x");
        let (pair, mut links) = combine_latest(&a, &b);
        assert_eq!(pair.get(), (1, "x"));
        let seen = Rc::new(RefCell::new(Vec::<(i32, &str)>::new()));
        let s2 = seen.clone();
        let _sub = pair.sink(move |v| s2.borrow_mut().push(*v));
        a.set(2);
        b.set("y");
        assert_eq!(*seen.borrow(), vec![(1, "x"), (2, "x"), (2, "y")]);
        assert_eq!(links.len(), 2);
        links.clear();
        a.set(3);
        assert_eq!(pair.get(), (2, "y"));
        assert_eq!(a.subscriber_count(), 0);
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
