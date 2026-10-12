//! Suspension behaviours for cross-app broadcast posts: what happens to a post whose receiver is suspended or
//! not running.
//!
//! The broadcast center in [`notification`](crate::notification) relays a post to running apps and nothing else.
//! This module is the model that decides the rest. [`SuspensionBehavior::Drop`] loses the post.
//! [`SuspensionBehavior::Coalesce`] keeps only the latest post of each name. [`SuspensionBehavior::Hold`] keeps up
//! to [`MAX_HELD`] posts in arrival order and discards the oldest when full. [`SuspensionBehavior::DeliverImmediately`]
//! reaches a suspended receiver at once. A receiver that is not running is treated like a suspended one, except that
//! `DeliverImmediately` has no process to reach and drops the post. [`PostQueue::resume`] hands back what was held
//! when the receiver runs again.
//!
//! The model is pure: the shell owns the receivers' states and calls [`PostQueue::send`] and
//! [`PostQueue::resume`]. Nothing calls it yet, so a post still reaches running apps only and a post to a stopped
//! app is lost. The queue size of 32 and the value limits are BifrOSt's choices; the official pages do not give a
//! queue size for `hold`.
//!
//! ```ignore
//! let mut q = PostQueue::new();
//! let p = Post::new("Theme.changed", BTreeMap::new()).unwrap();
//! assert_eq!(q.send("notes", ReceiverState::Suspended, SuspensionBehavior::Hold, p), Outcome::Held);
//! assert_eq!(q.resume("notes").len(), 1);
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Posts held for one receiver before the oldest is discarded (`Hold`) or the older post of the same name is
/// replaced (`Coalesce`, when the queue is full of other names).
pub const MAX_HELD: usize = 32;
/// Entries allowed in one post's user info.
pub const MAX_USER_INFO_ENTRIES: usize = 32;
/// Characters allowed in one user-info value.
pub const MAX_VALUE_LEN: usize = 1024;

/// What a sender asks to happen to a post whose receiver cannot take it now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuspensionBehavior {
    /// The post is lost. Nothing is queued until the receiver resumes.
    Drop,
    /// Only the latest post of each name is queued; an earlier one of the same name is replaced.
    Coalesce,
    /// Every post is queued in arrival order, up to [`MAX_HELD`].
    Hold,
    /// A suspended receiver gets the post at once. A receiver that is not running has nothing to reach, so the
    /// post is lost.
    DeliverImmediately,
}

/// The state of a receiving app as the shell knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverState {
    Running,
    Suspended,
    NotRunning,
}

/// What became of one post.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The receiver got the post now.
    Delivered,
    /// The post waits in the queue for the receiver to resume.
    Held,
    /// The post is gone.
    Dropped,
}

/// A post was refused before it was queued: too many user-info entries, a value too long, or an empty name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostError(pub String);

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PostError {}

/// A cross-app post: a name and string-to-string user info, within the limits above.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    name: String,
    user_info: BTreeMap<String, String>,
}

impl Post {
    /// Checks the name and the user info. Fails on an empty name, more than [`MAX_USER_INFO_ENTRIES`] entries, or a
    /// value longer than [`MAX_VALUE_LEN`] characters.
    pub fn new(name: &str, user_info: BTreeMap<String, String>) -> Result<Post, PostError> {
        if name.is_empty() {
            return Err(PostError("post name is empty".into()));
        }
        if user_info.len() > MAX_USER_INFO_ENTRIES {
            return Err(PostError(format!("{} user info entries, the limit is {MAX_USER_INFO_ENTRIES}", user_info.len())));
        }
        if let Some((k, _)) = user_info.iter().find(|(_, v)| v.chars().count() > MAX_VALUE_LEN) {
            return Err(PostError(format!("value of {k:?} is longer than {MAX_VALUE_LEN} characters")));
        }
        Ok(Post { name: name.to_string(), user_info })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn user_info(&self) -> &BTreeMap<String, String> {
        &self.user_info
    }
}

/// Held posts for each receiver, keyed by the receiver's app id.
#[derive(Debug, Default)]
pub struct PostQueue {
    held: BTreeMap<String, VecDeque<Post>>,
}

impl PostQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `post` to `receiver`, whose state is `state`, under `behavior`, and says what became of it.
    pub fn send(&mut self, receiver: &str, state: ReceiverState, behavior: SuspensionBehavior, post: Post) -> Outcome {
        if state == ReceiverState::Running {
            return Outcome::Delivered;
        }
        match behavior {
            SuspensionBehavior::DeliverImmediately if state == ReceiverState::Suspended => Outcome::Delivered,
            SuspensionBehavior::DeliverImmediately | SuspensionBehavior::Drop => Outcome::Dropped,
            SuspensionBehavior::Coalesce | SuspensionBehavior::Hold => {
                let queue = self.held.entry(receiver.to_string()).or_default();
                if behavior == SuspensionBehavior::Coalesce {
                    queue.retain(|p| p.name != post.name);
                }
                if queue.len() >= MAX_HELD {
                    queue.pop_front();
                }
                queue.push_back(post);
                Outcome::Held
            }
        }
    }

    /// Posts waiting for `receiver`.
    pub fn pending(&self, receiver: &str) -> usize {
        self.held.get(receiver).map_or(0, VecDeque::len)
    }

    /// Takes everything held for `receiver`, oldest first. Call it when the receiver runs again.
    pub fn resume(&mut self, receiver: &str) -> Vec<Post> {
        self.held.remove(receiver).map(Vec::from).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(name: &str) -> Post {
        Post::new(name, BTreeMap::new()).unwrap()
    }

    fn names(posts: &[Post]) -> Vec<&str> {
        posts.iter().map(Post::name).collect()
    }

    #[test]
    fn a_running_receiver_gets_every_post_whatever_the_behaviour() {
        let mut q = PostQueue::new();
        for b in [SuspensionBehavior::Drop, SuspensionBehavior::Coalesce, SuspensionBehavior::Hold] {
            assert_eq!(q.send("notes", ReceiverState::Running, b, post("a")), Outcome::Delivered);
        }
        assert_eq!(q.pending("notes"), 0);
    }

    #[test]
    fn drop_loses_the_post_while_suspended() {
        let mut q = PostQueue::new();
        assert_eq!(q.send("notes", ReceiverState::Suspended, SuspensionBehavior::Drop, post("a")), Outcome::Dropped);
        assert_eq!(q.pending("notes"), 0);
        assert!(q.resume("notes").is_empty());
    }

    #[test]
    fn coalesce_keeps_only_the_latest_post_of_each_name() {
        let mut q = PostQueue::new();
        let s = ReceiverState::Suspended;
        let c = SuspensionBehavior::Coalesce;
        assert_eq!(q.send("notes", s, c, post("a")), Outcome::Held);
        assert_eq!(q.send("notes", s, c, post("b")), Outcome::Held);
        assert_eq!(q.send("notes", s, c, post("a")), Outcome::Held);
        // Three posts, two names: the first "a" is replaced and the newest "a" sits behind "b".
        assert_eq!(q.pending("notes"), 2);
        assert_eq!(names(&q.resume("notes")), vec!["b", "a"]);
    }

    #[test]
    fn hold_keeps_arrival_order_and_discards_the_oldest_when_full() {
        let mut q = PostQueue::new();
        let names_in: Vec<String> = (0..=MAX_HELD).map(|i| format!("n{i}")).collect();
        for n in &names_in {
            q.send("notes", ReceiverState::Suspended, SuspensionBehavior::Hold, post(n));
        }
        // 33 posts into a queue of 32: "n0" was discarded, so the first held post is "n1".
        assert_eq!(q.pending("notes"), MAX_HELD);
        let held = q.resume("notes");
        assert_eq!(held.first().map(Post::name), Some("n1"));
        assert_eq!(held.last().map(Post::name), Some("n32"));
        assert_eq!(q.pending("notes"), 0);
    }

    #[test]
    fn deliver_immediately_reaches_only_a_running_process() {
        let mut q = PostQueue::new();
        let d = SuspensionBehavior::DeliverImmediately;
        assert_eq!(q.send("notes", ReceiverState::Suspended, d, post("a")), Outcome::Delivered);
        assert_eq!(q.send("notes", ReceiverState::NotRunning, d, post("b")), Outcome::Dropped);
        assert_eq!(q.pending("notes"), 0);
    }

    #[test]
    fn a_stopped_receiver_holds_posts_until_it_runs_again() {
        let mut q = PostQueue::new();
        let h = SuspensionBehavior::Hold;
        q.send("editor", ReceiverState::NotRunning, h, post("x"));
        q.send("editor", ReceiverState::NotRunning, h, post("y"));
        // Another receiver's queue is separate.
        q.send("mail", ReceiverState::Suspended, h, post("z"));
        assert_eq!(q.pending("editor"), 2);
        assert_eq!(q.pending("mail"), 1);
        assert_eq!(names(&q.resume("editor")), vec!["x", "y"]);
        assert_eq!(q.pending("editor"), 0);
        assert_eq!(q.pending("mail"), 1);
    }

    #[test]
    fn post_limits_are_checked_before_queueing() {
        let entries = |n: usize| -> BTreeMap<String, String> { (0..n).map(|i| (format!("k{i:02}"), "v".to_string())).collect() };
        assert!(Post::new("a", entries(MAX_USER_INFO_ENTRIES)).is_ok());
        assert!(Post::new("a", entries(MAX_USER_INFO_ENTRIES + 1)).is_err());
        assert!(Post::new("", BTreeMap::new()).is_err());

        let mut long = BTreeMap::new();
        long.insert("k".to_string(), "é".repeat(MAX_VALUE_LEN));
        assert!(Post::new("a", long.clone()).is_ok());
        long.insert("k".to_string(), "é".repeat(MAX_VALUE_LEN + 1));
        // Counted in characters, not bytes: 1025 two-byte characters are refused.
        assert!(Post::new("a", long).is_err());
    }
}
