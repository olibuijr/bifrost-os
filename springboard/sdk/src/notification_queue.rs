//! `NotificationQueue` parity: posts held back and merged before they reach [`NotificationCenter`].
//!
//! A queue keeps its pending notifications in the order they were queued. Each post names a [`PostingStyle`] and a
//! [`Coalescing`] rule:
//! - `Coalescing::OnName` or `OnSender` or `OnNameAndSender`: a queued notification that matches on those keys is
//!   removed, and the newer one takes its place at the end (the latest state wins, one delivery).
//! - [`PostingStyle::Now`] is not queued: [`NotificationQueue::enqueue`] hands it back for an immediate post.
//! - [`PostingStyle::Asap`] and [`PostingStyle::WhenIdle`] wait for [`NotificationQueue::deliver_pending`], which posts
//!   everything queued on the main queue. Gap: there is no idle detection yet, so `WhenIdle` delivers on the same
//!   drain as `Asap`.
//!
//! The queue is plain data (owned by the app), so the merge rules are testable without a running app.

use crate::dispatch::DispatchQueue;
use crate::notification::{Notification, NotificationCenter};

/// When a queued post reaches observers (the posting style).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostingStyle {
    /// Posted at once; never held in the queue.
    Now,
    /// Posted at the next main-queue drain.
    Asap,
    /// Posted at the next drain. Idle detection is not implemented yet.
    WhenIdle,
}

/// Which queued posts a new post replaces (the coalescing mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coalescing {
    None,
    OnName,
    OnSender,
    OnNameAndSender,
}

/// Pending notifications, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotificationQueue {
    pending: Vec<Notification>,
}

fn matches(a: &Notification, b: &Notification, rule: Coalescing) -> bool {
    match rule {
        Coalescing::None => false,
        Coalescing::OnName => a.name == b.name,
        Coalescing::OnSender => a.object == b.object,
        Coalescing::OnNameAndSender => a.name == b.name && a.object == b.object,
    }
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `n` with the given style and rule. Returns `Some(n)` for [`PostingStyle::Now`] so the caller posts it
    /// at once; `None` when it is held. A coalescing rule first removes the queued posts that match `n`.
    pub fn enqueue(&mut self, n: Notification, style: PostingStyle, coalescing: Coalescing) -> Option<Notification> {
        if style == PostingStyle::Now {
            return Some(n);
        }
        self.pending.retain(|q| !matches(q, &n, coalescing));
        self.pending.push(n);
        None
    }

    /// Removes the queued posts named `name` (and sent by `object`, when given), as `removeNotifications` does.
    pub fn remove(&mut self, name: Option<&str>, object: Option<u64>) {
        self.pending.retain(|q| {
            let name_hit = name.map_or(true, |n| q.name == n);
            let object_hit = object.map_or(true, |o| q.object == Some(o));
            !(name_hit && object_hit)
        });
    }

    /// Takes every queued post, oldest first, and empties the queue.
    pub fn take_pending(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.pending)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Posts everything queued to [`NotificationCenter`] on the main queue, oldest first. The queue is emptied now, so
    /// a post made by an observer during delivery is held for the next drain.
    pub fn deliver_pending(&mut self) {
        let batch = self.take_pending();
        if batch.is_empty() {
            return;
        }
        DispatchQueue::main().async_(move || {
            let center = NotificationCenter;
            for n in batch {
                center.post(&n.name, n.object, n.user_info);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn note(name: &str, object: Option<u64>) -> Notification {
        Notification { name: name.to_string(), object, user_info: HashMap::new() }
    }

    #[test]
    fn now_is_handed_back_and_not_held() {
        let mut q = NotificationQueue::new();
        assert_eq!(q.enqueue(note("a", None), PostingStyle::Now, Coalescing::None), Some(note("a", None)));
        assert!(q.is_empty());
    }

    #[test]
    fn held_posts_keep_their_order() {
        let mut q = NotificationQueue::new();
        assert!(q.enqueue(note("a", None), PostingStyle::Asap, Coalescing::None).is_none());
        assert!(q.enqueue(note("b", Some(2)), PostingStyle::WhenIdle, Coalescing::None).is_none());
        assert_eq!(q.len(), 2);
        assert_eq!(q.take_pending(), vec![note("a", None), note("b", Some(2))]);
        assert!(q.is_empty());
    }

    #[test]
    fn coalescing_on_name_keeps_only_the_newest() {
        let mut q = NotificationQueue::new();
        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::OnName);
        q.enqueue(note("b", None), PostingStyle::Asap, Coalescing::OnName);
        q.enqueue(note("a", Some(9)), PostingStyle::Asap, Coalescing::OnName);
        assert_eq!(q.take_pending(), vec![note("b", None), note("a", Some(9))]);
    }

    #[test]
    fn coalescing_on_sender_and_on_both() {
        let mut q = NotificationQueue::new();
        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::OnSender);
        q.enqueue(note("b", Some(1)), PostingStyle::Asap, Coalescing::OnSender);
        assert_eq!(q.take_pending(), vec![note("b", Some(1))]);

        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::OnNameAndSender);
        q.enqueue(note("a", Some(2)), PostingStyle::Asap, Coalescing::OnNameAndSender);
        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::OnNameAndSender);
        assert_eq!(q.take_pending(), vec![note("a", Some(2)), note("a", Some(1))]);
    }

    #[test]
    fn no_coalescing_keeps_duplicates() {
        let mut q = NotificationQueue::new();
        q.enqueue(note("a", None), PostingStyle::Asap, Coalescing::None);
        q.enqueue(note("a", None), PostingStyle::Asap, Coalescing::None);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn remove_by_name_sender_or_both() {
        let mut q = NotificationQueue::new();
        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::None);
        q.enqueue(note("a", Some(2)), PostingStyle::Asap, Coalescing::None);
        q.enqueue(note("b", Some(1)), PostingStyle::Asap, Coalescing::None);
        q.remove(Some("a"), Some(1));
        assert_eq!(q.take_pending(), vec![note("a", Some(2)), note("b", Some(1))]);

        q.enqueue(note("a", Some(1)), PostingStyle::Asap, Coalescing::None);
        q.enqueue(note("b", Some(1)), PostingStyle::Asap, Coalescing::None);
        q.remove(None, Some(1));
        assert!(q.is_empty());
        q.enqueue(note("c", None), PostingStyle::Asap, Coalescing::None);
        q.remove(Some("c"), None);
        assert!(q.is_empty());
    }
}
