//! Observable values (Combine `@Published` / `CurrentValueSubject`, `ObservableObject`).
//! Pure and single-threaded: no shell or UI dependency.
//!
//! `set`/`update` fire each subscriber once per change; `sink` returns a [`Subscription`]
//! (`AnyCancellable`) that cancels on drop; `map` and `remove_duplicates` derive new observables.
//! A `set` made from inside a subscriber is queued and delivered after the running change, in order.
//! `with_observation_tracking` records the observables its closure reads and calls `on_change` once when
//! one of them changes; `Observable::observations` pulls the changes in order instead of calling back.
//!
//! The publisher side (Combine) adds: [`Completion`] and [`Never`] for how a stream stops; [`Finite`] for
//! the replay publishers `Just`, `Empty`, `Fail` and `Record`; [`Future`] and [`Promise`] for one later
//! result; [`Deferred`] for a publisher built per subscriber; [`Connectable`] and [`Autoconnect`] for
//! `multicast`, `connect`, `autoconnect` and `share`; and [`CombineIdentifier`] on each subject.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

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
    id: CombineIdentifier,
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
        let inner = PassInner {
            id: CombineIdentifier::new(),
            next: 0,
            subs: Vec::new(),
            cancelled: Vec::new(),
            depth: 0,
        };
        PassthroughSubject { inner: Rc::new(RefCell::new(inner)) }
    }

    /// The identifier of this subject. Clones share it; a new subject gets a new one.
    pub fn identifier(&self) -> CombineIdentifier {
        self.inner.borrow().id
    }

    /// Number of live subscribers. Reads 0 while a `send` is delivering (subscribers are taken out), as
    /// [`Observable::subscriber_count`] does.
    pub fn subscriber_count(&self) -> usize {
        self.inner.borrow().subs.len()
    }

    /// `multicast`: a connectable view of this publisher. Subscribers attach to the view; nothing flows
    /// from this publisher to them until [`Connectable::connect`] is called.
    pub fn multicast(&self) -> Connectable<T>
    where
        T: Clone,
    {
        Connectable::new(self)
    }

    /// `share()`: `multicast` that connects when the first subscriber arrives and stays connected.
    pub fn share(&self) -> Autoconnect<T>
    where
        T: Clone,
    {
        self.multicast().autoconnect()
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

impl<T: Clone + 'static> PassthroughSubject<T> {
    /// `merge(with:)`: a subject that sends every value sent on either this subject or `other`, in the order
    /// they are sent. The returned `Cancellables` keeps both links alive; dropping it detaches the merge.
    pub fn merge(&self, other: &PassthroughSubject<T>) -> (PassthroughSubject<T>, Cancellables) {
        let out = PassthroughSubject::new();
        let mut links = Cancellables::new();
        let o1 = out.clone();
        self.sink(move |v| o1.send(v.clone())).store(&mut links);
        let o2 = out.clone();
        other.sink(move |v| o2.send(v.clone())).store(&mut links);
        (out, links)
    }

    /// `flatMap(_:)` for inner publishers that replay synchronously: each value is turned into a [`Finite`]
    /// publisher and every value that publisher replays is sent on the result. The inner completion is not
    /// forwarded, as Combine's `flatMap` does not end the outer stream when an inner one finishes.
    pub fn flat_map<U, E>(&self, f: impl Fn(&T) -> Finite<U, E> + 'static) -> (PassthroughSubject<U>, Subscription)
    where
        U: Clone + 'static,
        E: Clone,
    {
        let out = PassthroughSubject::new();
        let o = out.clone();
        let sub = self.sink(move |v| {
            f(v).subscribe(|u| o.send(u.clone()), |_completion| {});
        });
        (out, sub)
    }
}

/// `Subscribers.Completion`: how a publisher stopped producing values.
#[derive(Clone, Debug, PartialEq)]
pub enum Completion<E> {
    Finished,
    Failed(E),
}

/// `Never`: the failure type of a publisher that cannot fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Never {}

/// A publisher that replays a fixed list of values, then finishes, fails, or never completes. It covers
/// `Just` (one value, then finish), `Empty` (no values; finish now or never), `Fail` (no values, then a
/// failure) and `Record` (a given list, then a given completion). Each subscriber gets every value and
/// then the completion, synchronously and in order.
#[derive(Clone, Debug)]
pub struct Finite<T, E> {
    values: Vec<T>,
    /// `None` means the publisher never completes.
    completion: Option<Completion<E>>,
}

impl<T: Clone, E: Clone> Finite<T, E> {
    /// `Record(output:completion:)`.
    pub fn record(values: Vec<T>, completion: Completion<E>) -> Self {
        Finite { values, completion: Some(completion) }
    }

    /// `Just(_:)`: emits `value` once, then finishes.
    pub fn just(value: T) -> Self {
        Self::record(vec![value], Completion::Finished)
    }

    /// `Empty(completeImmediately:)`: no values; finishes at once when `complete_immediately` is set,
    /// otherwise never completes.
    pub fn empty(complete_immediately: bool) -> Self {
        Finite { values: Vec::new(), completion: complete_immediately.then_some(Completion::Finished) }
    }

    /// `Fail(error:)`: no values, then fails at once.
    pub fn fail(error: E) -> Self {
        Self::record(Vec::new(), Completion::Failed(error))
    }

    /// Number of values this publisher replays.
    pub fn value_count(&self) -> usize {
        self.values.len()
    }

    /// Replays the values to `on_value`, then the completion to `on_completion` if there is one.
    pub fn subscribe(&self, mut on_value: impl FnMut(&T), on_completion: impl FnOnce(Completion<E>)) {
        for v in &self.values {
            on_value(v);
        }
        if let Some(c) = self.completion.clone() {
            on_completion(c);
        }
    }
}

struct FutureInner<T, E> {
    result: Option<Result<T, E>>,
    waiting: Vec<Box<dyn FnOnce(&Result<T, E>)>>,
}

/// `Future`: a publisher of one result that is produced later by a [`Promise`]. A subscriber that
/// arrives before the result waits for it; a later subscriber gets the stored result at once. Each
/// subscriber receives the result once. A waiting subscriber cannot be cancelled.
pub struct Future<T, E> {
    inner: Rc<RefCell<FutureInner<T, E>>>,
}

/// The handle a [`Future`] gives its producer. The first result wins; later ones are refused.
pub struct Promise<T, E> {
    inner: Rc<RefCell<FutureInner<T, E>>>,
}

impl<T: Clone + 'static, E: Clone + 'static> Future<T, E> {
    /// Runs `attempt` at once with the promise. The attempt may resolve it synchronously or keep the
    /// promise and resolve it later.
    pub fn new(attempt: impl FnOnce(Promise<T, E>)) -> Self {
        let inner = Rc::new(RefCell::new(FutureInner { result: None, waiting: Vec::new() }));
        attempt(Promise { inner: inner.clone() });
        Future { inner }
    }

    /// Calls `f` with the result: now if it is resolved, otherwise when the promise resolves.
    pub fn subscribe(&self, f: impl FnOnce(&Result<T, E>) + 'static) {
        let ready = self.inner.borrow().result.clone();
        match ready {
            Some(r) => f(&r),
            None => self.inner.borrow_mut().waiting.push(Box::new(f)),
        }
    }

    /// True once the promise has been resolved.
    pub fn is_resolved(&self) -> bool {
        self.inner.borrow().result.is_some()
    }
}

impl<T: Clone + 'static, E: Clone + 'static> Promise<T, E> {
    /// Resolves the future and calls the waiting subscribers in the order they subscribed. Returns
    /// `false` and changes nothing when the future was already resolved.
    pub fn resolve(&self, result: Result<T, E>) -> bool {
        let waiting = {
            let mut i = self.inner.borrow_mut();
            if i.result.is_some() {
                return false;
            }
            i.result = Some(result.clone());
            std::mem::take(&mut i.waiting)
        };
        for f in waiting {
            f(&result);
        }
        true
    }
}

/// `Deferred`: builds a fresh publisher for each subscriber, so the work runs only when someone subscribes.
#[derive(Clone)]
pub struct Deferred<T, E> {
    make: Rc<dyn Fn() -> Finite<T, E>>,
}

impl<T: Clone + 'static, E: Clone + 'static> Deferred<T, E> {
    pub fn new(make: impl Fn() -> Finite<T, E> + 'static) -> Self {
        Deferred { make: Rc::new(make) }
    }

    /// Calls the factory for this subscriber and replays its publisher.
    pub fn subscribe(&self, on_value: impl FnMut(&T), on_completion: impl FnOnce(Completion<E>)) {
        (self.make)().subscribe(on_value, on_completion);
    }
}

/// `ConnectablePublisher`: values from `upstream` reach the subscribers only while connected. Subscribers
/// attach to a shared subject; [`Connectable::connect`] links that subject to the upstream.
#[derive(Clone)]
pub struct Connectable<T> {
    upstream: PassthroughSubject<T>,
    subject: PassthroughSubject<T>,
}

impl<T: Clone + 'static> Connectable<T> {
    /// A connectable view of `upstream`, not yet connected.
    pub fn new(upstream: &PassthroughSubject<T>) -> Self {
        Connectable { upstream: upstream.clone(), subject: PassthroughSubject::new() }
    }

    /// `connect()`: forwards each upstream value to the subscribers until the returned `Subscription`
    /// is cancelled or dropped. Each call adds another link.
    pub fn connect(&self) -> Subscription {
        let out = self.subject.clone();
        self.upstream.sink(move |v| out.send(v.clone()))
    }

    /// Attaches a subscriber to the shared subject.
    pub fn sink(&self, f: impl FnMut(&T) + 'static) -> Subscription {
        self.subject.sink(f)
    }

    /// Number of subscribers on the shared subject.
    pub fn subscriber_count(&self) -> usize {
        self.subject.subscriber_count()
    }

    /// `autoconnect()`: connects when the first subscriber arrives and never disconnects afterwards.
    pub fn autoconnect(&self) -> Autoconnect<T> {
        Autoconnect { connectable: self.clone(), link: Rc::new(RefCell::new(None)) }
    }
}

/// `Publishers.Autoconnect`: a [`Connectable`] that connects on the first subscription. The connection
/// is kept for the life of the value, so later subscribers share it.
#[derive(Clone)]
pub struct Autoconnect<T> {
    connectable: Connectable<T>,
    link: Rc<RefCell<Option<Subscription>>>,
}

impl<T: Clone + 'static> Autoconnect<T> {
    /// Attaches a subscriber, connecting first if no connection exists yet.
    pub fn sink(&self, f: impl FnMut(&T) + 'static) -> Subscription {
        let connected = self.link.borrow().is_some();
        if !connected {
            let link = self.connectable.connect();
            *self.link.borrow_mut() = Some(link);
        }
        self.connectable.sink(f)
    }
}

/// `CombineIdentifier`: a process-unique id for a publisher stream. Values are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CombineIdentifier(u64);

static NEXT_IDENTIFIER: AtomicU64 = AtomicU64::new(1);

impl CombineIdentifier {
    /// A new identifier, distinct from every one made before it in this process.
    pub fn new() -> Self {
        CombineIdentifier(NEXT_IDENTIFIER.fetch_add(1, Ordering::Relaxed))
    }

    /// The numeric value of the identifier.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl Default for CombineIdentifier {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CombineIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CombineIdentifier({:#x})", self.0)
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

    #[test]
    fn just_emits_its_value_once_then_finishes() {
        let f: Finite<i32, Never> = Finite::just(3);
        let mut vals = Vec::new();
        let done = Rc::new(RefCell::new(None));
        let d = done.clone();
        f.subscribe(|v| vals.push(*v), |c| *d.borrow_mut() = Some(c));
        assert_eq!(vals, vec![3]);
        assert_eq!(*done.borrow(), Some(Completion::Finished));
    }

    #[test]
    fn empty_finishes_at_once_or_never() {
        let mut emitted = 0;
        let mut completed = false;
        Finite::<i32, &str>::empty(false).subscribe(|_| emitted += 1, |_| completed = true);
        assert_eq!(emitted, 0, "empty(false) emits nothing");
        assert!(!completed, "empty(false) never completes");
        let done = Rc::new(RefCell::new(None));
        let d = done.clone();
        let f = Finite::<i32, &str>::empty(true);
        f.subscribe(|_| panic!("no values"), |c| *d.borrow_mut() = Some(c));
        assert_eq!(f.value_count(), 0);
        assert_eq!(*done.borrow(), Some(Completion::Finished));
    }

    #[test]
    fn fail_and_record_replay_values_then_the_failure_in_order() {
        let (log, w) = counter();
        let done = Rc::new(RefCell::new(None));
        let d = done.clone();
        Finite::record(vec![1, 2], Completion::Failed("boom")).subscribe(
            move |v| w.borrow_mut().push(*v),
            |c| *d.borrow_mut() = Some(c),
        );
        assert_eq!(*log.borrow(), vec![1, 2]);
        assert_eq!(*done.borrow(), Some(Completion::Failed("boom")));

        let failed = Rc::new(RefCell::new(None));
        let f2 = failed.clone();
        Finite::<i32, &str>::fail("nope").subscribe(|_| panic!("no values"), |c| *f2.borrow_mut() = Some(c));
        assert_eq!(*failed.borrow(), Some(Completion::Failed("nope")));
    }

    #[test]
    fn future_waits_for_the_promise_then_keeps_the_first_result() {
        let slot: Rc<RefCell<Option<Promise<i32, &'static str>>>> = Rc::new(RefCell::new(None));
        let s = slot.clone();
        let fut: Future<i32, &'static str> = Future::new(move |p| *s.borrow_mut() = Some(p));
        let (log, w) = counter();
        fut.subscribe(move |r| w.borrow_mut().push(r.clone().unwrap_or(-1)));
        assert!(log.borrow().is_empty());
        assert!(!fut.is_resolved());
        let promise = slot.borrow_mut().take().unwrap();
        assert!(promise.resolve(Ok(5)));
        assert!(!promise.resolve(Ok(9)), "the first result wins");
        assert_eq!(*log.borrow(), vec![5]);
        // A subscriber after the result gets it at once.
        let late = Rc::new(RefCell::new(None));
        let l = late.clone();
        fut.subscribe(move |r| *l.borrow_mut() = Some(r.clone()));
        assert_eq!(*late.borrow(), Some(Ok(5)));
        assert!(fut.is_resolved());
    }

    #[test]
    fn future_resolved_during_creation_and_failure_result() {
        let fut: Future<i32, &'static str> = Future::new(|p| {
            p.resolve(Err("x"));
        });
        let got = Rc::new(RefCell::new(None));
        let g = got.clone();
        fut.subscribe(move |r| *g.borrow_mut() = Some(r.clone()));
        assert_eq!(*got.borrow(), Some(Err("x")));
    }

    #[test]
    fn deferred_builds_a_fresh_publisher_per_subscriber() {
        let made = Rc::new(Cell::new(0));
        let m = made.clone();
        let d: Deferred<u32, ()> = Deferred::new(move || {
            m.set(m.get() + 1);
            Finite::just(m.get())
        });
        assert_eq!(made.get(), 0, "nothing runs before a subscription");
        let (log, w) = counter_u32();
        let w2 = w.clone();
        d.subscribe(move |v| w.borrow_mut().push(*v), |_| {});
        d.subscribe(move |v| w2.borrow_mut().push(*v), |_| {});
        assert_eq!(made.get(), 2);
        assert_eq!(*log.borrow(), vec![1, 2]);
    }

    #[test]
    fn connectable_delivers_only_while_connected() {
        let up = PassthroughSubject::<i32>::new();
        let conn = up.multicast();
        let (log, w) = counter();
        let _s = conn.sink(move |v| w.borrow_mut().push(*v));
        up.send(1);
        assert!(log.borrow().is_empty(), "nothing flows before connect");
        let link = conn.connect();
        assert_eq!(up.subscriber_count(), 1);
        up.send(2);
        link.cancel();
        assert_eq!(up.subscriber_count(), 0);
        up.send(3);
        assert_eq!(*log.borrow(), vec![2]);
    }

    #[test]
    fn share_connects_on_first_subscriber_and_shares_one_link() {
        let up = PassthroughSubject::<i32>::new();
        let shared = up.share();
        assert_eq!(up.subscriber_count(), 0);
        let (a_log, aw) = counter();
        let (b_log, bw) = counter();
        let _a = shared.sink(move |v| aw.borrow_mut().push(*v));
        assert_eq!(up.subscriber_count(), 1, "the first subscriber connects");
        let _b = shared.sink(move |v| bw.borrow_mut().push(*v));
        assert_eq!(up.subscriber_count(), 1, "later subscribers share the connection");
        up.send(7);
        assert_eq!(*a_log.borrow(), vec![7]);
        assert_eq!(*b_log.borrow(), vec![7]);
    }

    #[test]
    fn combine_identifiers_are_unique_shared_by_clones_and_display_as_hex() {
        let a = PassthroughSubject::<i32>::new();
        let b = PassthroughSubject::<i32>::new();
        assert_ne!(a.identifier(), b.identifier());
        assert_eq!(a.clone().identifier(), a.identifier());
        let x = CombineIdentifier::new();
        let y = CombineIdentifier::new();
        assert!(y.raw() > x.raw());
        assert_eq!(CombineIdentifier(255).to_string(), "CombineIdentifier(0xff)");
    }

    fn counter_u32() -> (Rc<RefCell<Vec<u32>>>, Rc<RefCell<Vec<u32>>>) {
        let l = Rc::new(RefCell::new(Vec::new()));
        (l.clone(), l)
    }
}

#[cfg(test)]
mod merge_flat_map_tests {
    use super::*;

    fn log_of<T: Clone + 'static>(subject: &PassthroughSubject<T>) -> (Rc<RefCell<Vec<T>>>, Subscription) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let sub = subject.sink(move |v: &T| l.borrow_mut().push(v.clone()));
        (log, sub)
    }

    #[test]
    fn merge_forwards_values_from_both_subjects_in_send_order() {
        let a = PassthroughSubject::new();
        let b = PassthroughSubject::new();
        let (merged, _links) = a.merge(&b);
        let (log, _sub) = log_of(&merged);
        a.send(1);
        b.send(2);
        a.send(3);
        b.send(4);
        assert_eq!(*log.borrow(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn merge_stops_forwarding_when_links_are_cleared() {
        let a = PassthroughSubject::new();
        let b = PassthroughSubject::new();
        let (merged, mut links) = a.merge(&b);
        let (log, _sub) = log_of(&merged);
        a.send(1);
        links.clear();
        a.send(2);
        b.send(3);
        assert_eq!(*log.borrow(), vec![1]);
    }

    #[test]
    fn merge_with_no_values_sends_nothing() {
        let a: PassthroughSubject<i32> = PassthroughSubject::new();
        let b: PassthroughSubject<i32> = PassthroughSubject::new();
        let (merged, _links) = a.merge(&b);
        let (log, _sub) = log_of(&merged);
        assert!(log.borrow().is_empty());
        assert_eq!(merged.subscriber_count(), 1);
    }

    #[test]
    fn flat_map_sends_each_inner_value_in_order() {
        let src: PassthroughSubject<i32> = PassthroughSubject::new();
        let (out, _sub) = src.flat_map(|n: &i32| Finite::<i32, Never>::record(vec![*n, *n * 10], Completion::Finished));
        let (log, _log_sub) = log_of(&out);
        src.send(1);
        src.send(2);
        assert_eq!(*log.borrow(), vec![1, 10, 2, 20]);
    }

    #[test]
    fn flat_map_empty_inner_sends_nothing_and_does_not_end_the_outer_stream() {
        let src: PassthroughSubject<i32> = PassthroughSubject::new();
        let (out, _sub) = src.flat_map(|n: &i32| {
            if *n == 0 {
                Finite::<i32, Never>::empty(true)
            } else {
                Finite::<i32, Never>::just(*n)
            }
        });
        let (log, _log_sub) = log_of(&out);
        src.send(0);
        src.send(5);
        assert_eq!(*log.borrow(), vec![5]);
    }

    #[test]
    fn flat_map_cancelled_upstream_stops_inner_values() {
        let src: PassthroughSubject<i32> = PassthroughSubject::new();
        let (out, sub) = src.flat_map(|n: &i32| Finite::<i32, Never>::just(*n));
        let (log, _log_sub) = log_of(&out);
        src.send(1);
        sub.cancel();
        src.send(2);
        assert_eq!(*log.borrow(), vec![1]);
    }
}
