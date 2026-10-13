//! Navigation decisions and the back-forward list of a web view (`NavigationDelegate` policy callbacks and
//! `BackForwardList`).
//!
//! The engine asks the app before each request (`decidePolicyFor navigationAction`) and before each response is
//! shown (`decidePolicyFor navigationResponse`). The app answers allow, cancel or download. An answer that does not
//! arrive before its deadline counts as allow, so a slow delegate never stalls a page. [`BackForwardList`] is the
//! pure history model behind `backItem`, `forwardItem`, `item(at:)` and `go(to:)`.

use std::collections::HashMap;

/// The answer to a request or response decision (`NavigationActionPolicy`, `NavigationResponsePolicy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationActionPolicy {
    Cancel,
    Allow,
    /// The navigation becomes a download (see `web_download`).
    Download,
}

/// What caused a navigation (`NavigationType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationType {
    LinkActivated,
    FormSubmitted,
    BackForward,
    Reload,
    FormResubmitted,
    Other,
}

/// A request about to start (`NavigationAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationActionInfo {
    pub url: String,
    pub navigation_type: NavigationType,
    /// `targetFrame` is the main frame; false for a request from a frame inside the page.
    pub target_is_main_frame: bool,
}

/// A response whose headers arrived (`NavigationResponse`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationResponseInfo {
    pub url: String,
    pub status: u16,
    pub mime: String,
    pub is_for_main_frame: bool,
}

/// The app side of the policy callbacks. Both default to allow, like a view without a delegate.
pub trait NavigationDelegate {
    fn decide_policy_for_action(&mut self, _action: &NavigationActionInfo) -> NavigationActionPolicy {
        NavigationActionPolicy::Allow
    }
    fn decide_policy_for_response(&mut self, _response: &NavigationResponseInfo) -> NavigationActionPolicy {
        NavigationActionPolicy::Allow
    }
}

/// Default time the engine waits for a decision, in milliseconds.
pub const DECISION_TIMEOUT_MS: u64 = 5_000;

/// Decisions the engine is waiting on, by token. Times are caller-supplied milliseconds on any monotonic clock.
#[derive(Debug, Default)]
pub struct PolicyBroker {
    pending: HashMap<u32, u64>,
    answered: HashMap<u32, NavigationActionPolicy>,
}

impl PolicyBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers decision `token`, due at `now + timeout`.
    pub fn request(&mut self, token: u32, now: u64, timeout: u64) {
        self.pending.insert(token, now.saturating_add(timeout));
    }

    /// Records the app's answer. Returns false when the token is unknown, already answered or expired.
    pub fn answer(&mut self, token: u32, policy: NavigationActionPolicy) -> bool {
        if self.pending.remove(&token).is_some() {
            self.answered.insert(token, policy);
            true
        } else {
            false
        }
    }

    /// Gives up on every decision due at or before `now`; each resolves to allow. Returns their tokens, sorted.
    pub fn expire(&mut self, now: u64) -> Vec<u32> {
        let mut late: Vec<u32> = self.pending.iter().filter(|(_, due)| **due <= now).map(|(t, _)| *t).collect();
        late.sort_unstable();
        for t in &late {
            self.pending.remove(t);
            self.answered.insert(*t, NavigationActionPolicy::Allow);
        }
        late
    }

    /// Takes the resolved policy of `token`, if it has one.
    pub fn take(&mut self, token: u32) -> Option<NavigationActionPolicy> {
        self.answered.remove(&token)
    }

    pub fn is_pending(&self, token: u32) -> bool {
        self.pending.contains_key(&token)
    }
}

/// One entry of the history (`BackForwardListItem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackForwardItem {
    pub url: String,
    pub title: String,
}

/// The session history (`BackForwardList`): the entries, and which one is current.
#[derive(Clone, Debug, Default)]
pub struct BackForwardList {
    items: Vec<BackForwardItem>,
    current: usize,
}

impl BackForwardList {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new navigation: drops the forward entries and appends `url` as the current one.
    pub fn navigate(&mut self, url: &str) {
        if !self.items.is_empty() {
            self.items.truncate(self.current + 1);
        }
        self.items.push(BackForwardItem { url: url.into(), title: String::new() });
        self.current = self.items.len() - 1;
    }

    /// `location.replace`: the current entry takes `url` and keeps its place. Acts as `navigate` on an empty list.
    pub fn replace(&mut self, url: &str) {
        match self.items.get_mut(self.current) {
            Some(item) => {
                item.url = url.into();
                item.title.clear();
            }
            None => self.navigate(url),
        }
    }

    /// Sets the title of the current entry once the page reports it.
    pub fn set_title(&mut self, title: &str) {
        if let Some(item) = self.items.get_mut(self.current) {
            item.title = title.into();
        }
    }

    /// `currentItem`
    pub fn current_item(&self) -> Option<&BackForwardItem> {
        self.items.get(self.current)
    }

    /// `backItem`
    pub fn back_item(&self) -> Option<&BackForwardItem> {
        self.item_at(-1)
    }

    /// `forwardItem`
    pub fn forward_item(&self) -> Option<&BackForwardItem> {
        self.item_at(1)
    }

    /// `item(at:)`: 0 is the current entry, negative values go back, positive forward. `None` out of range.
    pub fn item_at(&self, offset: isize) -> Option<&BackForwardItem> {
        let index = self.current as isize + offset;
        if index < 0 {
            return None;
        }
        self.items.get(index as usize)
    }

    /// `backList`, oldest first.
    pub fn back_list(&self) -> &[BackForwardItem] {
        &self.items[..self.current.min(self.items.len())]
    }

    /// `forwardList`, nearest first.
    pub fn forward_list(&self) -> &[BackForwardItem] {
        if self.items.is_empty() {
            &[]
        } else {
            &self.items[self.current + 1..]
        }
    }

    /// `go(to:)`: makes the entry `offset` steps from the current one current. `None` and no change out of range.
    pub fn go_to(&mut self, offset: isize) -> Option<&BackForwardItem> {
        let index = self.current as isize + offset;
        if index < 0 || index as usize >= self.items.len() {
            return None;
        }
        self.current = index as usize;
        self.items.get(self.current)
    }

    pub fn go_back(&mut self) -> Option<&BackForwardItem> {
        self.go_to(-1)
    }

    pub fn go_forward(&mut self) -> Option<&BackForwardItem> {
        self.go_to(1)
    }

    pub fn can_go_back(&self) -> bool {
        self.current > 0
    }

    pub fn can_go_forward(&self) -> bool {
        !self.items.is_empty() && self.current + 1 < self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(items: &[BackForwardItem]) -> Vec<&str> {
        items.iter().map(|i| i.url.as_str()).collect()
    }

    #[test]
    fn a_delegate_without_overrides_allows_everything() {
        struct Plain;
        impl NavigationDelegate for Plain {}
        let action = NavigationActionInfo { url: "https://a.test/".into(), navigation_type: NavigationType::Other, target_is_main_frame: true };
        let response = NavigationResponseInfo { url: "https://a.test/".into(), status: 200, mime: "text/html".into(), is_for_main_frame: true };
        assert_eq!(Plain.decide_policy_for_action(&action), NavigationActionPolicy::Allow);
        assert_eq!(Plain.decide_policy_for_response(&response), NavigationActionPolicy::Allow);
    }

    #[test]
    fn an_answered_decision_is_returned_once() {
        let mut b = PolicyBroker::new();
        b.request(1, 0, DECISION_TIMEOUT_MS);
        assert!(b.is_pending(1));
        assert!(b.answer(1, NavigationActionPolicy::Cancel));
        assert_eq!(b.take(1), Some(NavigationActionPolicy::Cancel));
        assert_eq!(b.take(1), None);
        assert!(!b.answer(1, NavigationActionPolicy::Allow));
    }

    #[test]
    fn a_late_decision_defaults_to_allow() {
        let mut b = PolicyBroker::new();
        b.request(1, 100, 50);
        b.request(2, 100, 500);
        assert!(b.expire(149).is_empty());
        assert_eq!(b.expire(150), vec![1]);
        assert_eq!(b.take(1), Some(NavigationActionPolicy::Allow));
        assert!(b.is_pending(2));
        assert!(!b.answer(1, NavigationActionPolicy::Cancel));
    }

    #[test]
    fn download_is_a_valid_answer() {
        let mut b = PolicyBroker::new();
        b.request(7, 0, 10);
        b.answer(7, NavigationActionPolicy::Download);
        assert_eq!(b.take(7), Some(NavigationActionPolicy::Download));
    }

    #[test]
    fn navigate_back_forward_and_branch() {
        let mut l = BackForwardList::new();
        assert!(l.current_item().is_none() && !l.can_go_back() && !l.can_go_forward());
        for u in ["a", "b", "c"] {
            l.navigate(u);
        }
        assert_eq!(l.current_item().unwrap().url, "c");
        assert_eq!(l.back_item().unwrap().url, "b");
        assert!(l.forward_item().is_none());
        assert_eq!(l.go_back().unwrap().url, "b");
        assert_eq!(l.forward_item().unwrap().url, "c");
        assert!(l.can_go_back() && l.can_go_forward());
        l.navigate("d");
        assert_eq!(urls(l.back_list()), vec!["a", "b"]);
        assert!(l.forward_list().is_empty());
        assert!(l.go_forward().is_none());
    }

    #[test]
    fn item_at_and_go_to_use_offsets_from_the_current_entry() {
        let mut l = BackForwardList::new();
        for u in ["a", "b", "c", "d"] {
            l.navigate(u);
        }
        assert_eq!(l.item_at(-3).unwrap().url, "a");
        assert!(l.item_at(-4).is_none() && l.item_at(1).is_none());
        assert_eq!(l.go_to(-2).unwrap().url, "b");
        assert_eq!(l.item_at(0).unwrap().url, "b");
        assert!(l.go_to(5).is_none());
        assert_eq!(l.current_item().unwrap().url, "b");
        assert_eq!(urls(l.forward_list()), vec!["c", "d"]);
    }

    #[test]
    fn replace_keeps_the_position_and_titles_follow_the_current_entry() {
        let mut l = BackForwardList::new();
        l.replace("a");
        l.navigate("b");
        l.set_title("B");
        l.replace("b2");
        assert_eq!(urls(l.back_list()), vec!["a"]);
        let cur = l.current_item().unwrap();
        assert_eq!((cur.url.as_str(), cur.title.as_str()), ("b2", ""));
        l.set_title("B2");
        assert_eq!(l.current_item().unwrap().title, "B2");
    }
}
