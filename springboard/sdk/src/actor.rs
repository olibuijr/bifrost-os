//! Actor isolation: a value that only one thread ever touches. Callers never reach the value; they queue messages
//! (`tell`, fire and forget) or ask a question and wait for the answer (`ask`). Messages run one at a time, in the
//! order they were sent, on the actor's own thread, so the state needs no lock.
//!
//! A message that panics stops the actor: the panic is not caught, the thread ends, `ask` returns `None` from then
//! on, and later `tell` calls are dropped. Dropping the `Actor` lets the thread finish the messages already queued
//! and then exit. This models the reference isolation rules as far as a Rust std-only SDK can: there is no
//! compiler-checked isolation and no shared executor, so a value must not be reached except through its messages.

use std::sync::mpsc::{channel, Sender};
use std::thread;

type Job<S> = Box<dyn FnOnce(&mut S) + Send + 'static>;

/// A value owned by its own thread. Create one with [`Actor::new`].
pub struct Actor<S> {
    tx: Sender<Job<S>>,
}

impl<S: Send + 'static> Actor<S> {
    /// Starts the actor's thread with `state` as its isolated value.
    pub fn new(state: S) -> Actor<S> {
        let (tx, rx) = channel::<Job<S>>();
        thread::spawn(move || {
            let mut state = state;
            for job in rx {
                job(&mut state);
            }
        });
        Actor { tx }
    }

    /// Queues `f` to run on the actor's thread and returns at once.
    pub fn tell<F>(&self, f: F)
    where
        F: FnOnce(&mut S) + Send + 'static,
    {
        let _ = self.tx.send(Box::new(f));
    }

    /// Queues `f`, waits for it to run and returns its result. `None` means the actor has stopped.
    pub fn ask<R, F>(&self, f: F) -> Option<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut S) -> R + Send + 'static,
    {
        let (reply_tx, reply_rx) = channel::<R>();
        self.tell(move |state| {
            let _ = reply_tx.send(f(state));
        });
        reply_rx.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn tell_messages_run_in_the_order_sent() {
        let actor = Actor::new(Vec::<u32>::new());
        for n in 1..=5 {
            actor.tell(move |v| v.push(n));
        }
        assert_eq!(actor.ask(|v| v.clone()), Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn ask_returns_the_result_of_each_message_in_turn() {
        let actor = Actor::new(0i64);
        assert_eq!(actor.ask(|c| { *c += 3; *c }), Some(3));
        assert_eq!(actor.ask(|c| { *c += 4; *c }), Some(7));
        assert_eq!(actor.ask(|c| *c * 10), Some(70));
    }

    #[test]
    fn ask_reads_the_initial_state() {
        let actor = Actor::new(String::from("abc"));
        assert_eq!(actor.ask(|s| s.len()), Some(3));
    }

    #[test]
    fn a_panicking_message_stops_the_actor() {
        let actor = Actor::new(0i32);
        let first: Option<i32> = actor.ask(|_| panic!("message failed"));
        assert_eq!(first, None);
        // The thread is gone: later asks get no answer and later tells are dropped without a panic.
        assert_eq!(actor.ask(|c| *c), None);
        actor.tell(|c| *c += 1);
    }

    #[test]
    fn concurrent_senders_are_serialised_without_lost_updates() {
        let actor = Arc::new(Actor::new(0u64));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let actor = actor.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        actor.tell(|n| *n += 1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(actor.ask(|n| *n), Some(400));
    }
}
