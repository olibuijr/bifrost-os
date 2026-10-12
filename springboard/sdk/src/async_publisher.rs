//! `AsyncPublisher` and `AsyncThrowingPublisher`: a publisher's values exposed as a sequence the
//! caller pulls from. Std has no async runtime in this crate, so the sequence is pull-based and never
//! blocks: [`AsyncPublisher::next`] returns [`Step::Pending`] while nothing is buffered and the publisher
//! is still open. Values come out in the order they were sent, and the completion comes after the last
//! value. The sequence attaches to its publisher when it is created and detaches when it is dropped.
//!
//! A publisher whose failure type is `Never` gives `AsyncPublisher`; any other failure type gives
//! `AsyncThrowingPublisher` (the alias [`AsyncThrowingPublisher`] names the same type).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::observable::{Completion, Finite, PassthroughSubject, Subscription};

/// One pull from an async publisher sequence (`AsyncPublisher.Iterator.next()`).
#[derive(Clone, Debug, PartialEq)]
pub enum Step<T, E> {
    /// The next value, in send order.
    Value(T),
    /// The publisher finished. No more values come.
    Finished,
    /// The publisher failed with `E`. No more values come.
    Failed(E),
    /// Nothing is buffered yet and the publisher is still open.
    Pending,
}

/// A publisher's values as a pull-based sequence.
pub struct AsyncPublisher<T, E> {
    buffer: Rc<RefCell<VecDeque<T>>>,
    /// Set by the publisher when it completes; taken by `next` once the buffer is empty.
    completion: Rc<RefCell<Option<Completion<E>>>>,
    /// True after `next` has returned the completion.
    done: bool,
    /// Keeps the subject subscription alive; `None` for a finite publisher.
    _sub: Option<Subscription>,
}

/// `AsyncThrowingPublisher`: the same sequence for a publisher whose failure type is not `Never`.
pub type AsyncThrowingPublisher<T, E> = AsyncPublisher<T, E>;

impl<T: Clone + 'static, E: Clone + 'static> AsyncPublisher<T, E> {
    /// Attaches to a subject. The sequence gets each value sent after this call. A subject never
    /// completes, so the sequence only ends with `Pending` until it is dropped.
    pub fn from_subject(subject: &PassthroughSubject<T>) -> Self {
        let buffer = Rc::new(RefCell::new(VecDeque::new()));
        let b = buffer.clone();
        let sub = subject.sink(move |v: &T| b.borrow_mut().push_back(v.clone()));
        AsyncPublisher { buffer, completion: Rc::new(RefCell::new(None)), done: false, _sub: Some(sub) }
    }

    /// Replays a finite publisher into the sequence: every value, then its completion (if it has one).
    pub fn from_finite(publisher: &Finite<T, E>) -> Self {
        let buffer = Rc::new(RefCell::new(VecDeque::new()));
        let completion = Rc::new(RefCell::new(None));
        let (b, c) = (buffer.clone(), completion.clone());
        publisher.subscribe(
            move |v: &T| b.borrow_mut().push_back(v.clone()),
            move |done: Completion<E>| *c.borrow_mut() = Some(done),
        );
        AsyncPublisher { buffer, completion, done: false, _sub: None }
    }

    /// Takes the next step without blocking. Buffered values come first; the completion follows the last
    /// of them. After the completion, every call returns `Finished`.
    pub fn next(&mut self) -> Step<T, E> {
        if self.done {
            return Step::Finished;
        }
        let buffered = self.buffer.borrow_mut().pop_front();
        if let Some(v) = buffered {
            return Step::Value(v);
        }
        let completed = self.completion.borrow_mut().take();
        match completed {
            Some(Completion::Finished) => {
                self.done = true;
                Step::Finished
            }
            Some(Completion::Failed(e)) => {
                self.done = true;
                Step::Failed(e)
            }
            None => Step::Pending,
        }
    }

    /// Takes every buffered value at once, oldest first. It does not report the completion; call `next`
    /// for that.
    pub fn drain_ready(&mut self) -> Vec<T> {
        self.buffer.borrow_mut().drain(..).collect()
    }

    /// Number of values waiting to be taken.
    pub fn buffered(&self) -> usize {
        self.buffer.borrow().len()
    }

    /// True once `next` has returned the completion.
    pub fn is_finished(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observable::Never;

    #[test]
    fn subject_values_come_out_in_send_order() {
        let subject: PassthroughSubject<i32> = PassthroughSubject::new();
        let mut seq = AsyncPublisher::from_subject(&subject);
        subject.send(1);
        subject.send(2);
        assert_eq!(seq.buffered(), 2);
        assert_eq!(seq.next(), Step::Value(1));
        assert_eq!(seq.next(), Step::Value(2));
        assert_eq!(seq.next(), Step::Pending);
        assert!(!seq.is_finished());
    }

    #[test]
    fn values_sent_before_the_sequence_are_not_replayed() {
        let subject: PassthroughSubject<i32> = PassthroughSubject::new();
        subject.send(7);
        let mut seq: AsyncPublisher<i32, Never> = AsyncPublisher::from_subject(&subject);
        assert_eq!(seq.next(), Step::Pending);
        subject.send(8);
        assert_eq!(seq.next(), Step::Value(8));
    }

    #[test]
    fn dropping_the_sequence_detaches_from_the_subject() {
        let subject: PassthroughSubject<i32> = PassthroughSubject::new();
        let seq: AsyncPublisher<i32, Never> = AsyncPublisher::from_subject(&subject);
        assert_eq!(subject.subscriber_count(), 1);
        drop(seq);
        assert_eq!(subject.subscriber_count(), 0);
    }

    #[test]
    fn finite_publisher_yields_values_then_finishes_once() {
        let finite = Finite::record(vec![1, 2], Completion::Finished);
        let mut seq: AsyncPublisher<i32, Never> = AsyncPublisher::from_finite(&finite);
        assert_eq!(seq.next(), Step::Value(1));
        assert_eq!(seq.next(), Step::Value(2));
        assert_eq!(seq.next(), Step::Finished);
        assert!(seq.is_finished());
        assert_eq!(seq.next(), Step::Finished);
    }

    #[test]
    fn failure_is_reported_once_and_then_the_sequence_ends() {
        let failing: Finite<i32, &str> = Finite::fail("boom");
        let mut seq = AsyncPublisher::from_finite(&failing);
        assert_eq!(seq.next(), Step::Failed("boom"));
        assert_eq!(seq.next(), Step::Finished);
    }

    #[test]
    fn a_publisher_that_never_completes_stays_pending() {
        let open: Finite<i32, Never> = Finite::empty(false);
        let mut seq = AsyncPublisher::from_finite(&open);
        assert_eq!(seq.next(), Step::Pending);
        assert_eq!(seq.next(), Step::Pending);
        assert!(!seq.is_finished());
    }

    #[test]
    fn drain_ready_takes_all_buffered_values_in_order() {
        let finite = Finite::record(vec![3, 4, 5], Completion::Finished);
        let mut seq: AsyncPublisher<i32, Never> = AsyncPublisher::from_finite(&finite);
        assert_eq!(seq.drain_ready(), vec![3, 4, 5]);
        assert_eq!(seq.buffered(), 0);
        assert_eq!(seq.next(), Step::Finished);
    }
}
