//! Preparing thumbnails ahead of need. A list that shows many photos asks the caching manager to prepare the ones
//! near the visible rows, and to drop the ones that scrolled away. A preparation is identified by the asset, the
//! target size, the content mode and the delivery mode, so the same photo at two sizes is two preparations. The
//! manager keeps the set of preparations it has started; the shell does the decoding. `preheat_window` and `changes`
//! turn a scroll position into the start and stop lists. Pure: no file or image access here.
//! Mirrors the caching image manager's start and stop calls, its delivery modes and its high-quality switch.

use std::collections::BTreeSet;
use std::ops::Range;

/// How fast and how good a delivered image is allowed to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeliveryMode {
    /// One or more results, balancing quality and speed. The default.
    #[default]
    Opportunistic,
    /// Only the best quality available, however long it takes.
    HighQualityFormat,
    /// Only a fast image, which may be lower quality.
    FastFormat,
}

/// How the image fills the target size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContentMode {
    AspectFit,
    AspectFill,
}

/// One preparation: an asset at a size, in a content mode, with a delivery mode.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub asset: String,
    pub width: u32,
    pub height: u32,
    pub content_mode: ContentMode,
    pub delivery: DeliveryMode,
}

impl Key {
    pub fn new(asset: &str, width: u32, height: u32) -> Key {
        Key { asset: asset.to_string(), width, height, content_mode: ContentMode::AspectFill, delivery: DeliveryMode::default() }
    }
}

/// The set of started preparations. Starting a preparation twice is one preparation.
#[derive(Clone, Debug)]
pub struct Manager {
    allows_high_quality: bool,
    started: BTreeSet<Key>,
}

impl Default for Manager {
    fn default() -> Self {
        Manager { allows_high_quality: true, started: BTreeSet::new() }
    }
}

impl Manager {
    pub fn new() -> Manager {
        Manager::default()
    }

    /// Whether high-quality preparations are allowed. When false, `start` skips keys that ask for the high-quality
    /// delivery mode. The default is true.
    pub fn allows_caching_high_quality_images(&self) -> bool {
        self.allows_high_quality
    }

    pub fn set_allows_caching_high_quality_images(&mut self, allowed: bool) {
        self.allows_high_quality = allowed;
    }

    /// Starts preparing each key that is not already started. Returns how many were newly started.
    pub fn start(&mut self, keys: &[Key]) -> usize {
        let mut added = 0;
        for key in keys {
            if key.delivery == DeliveryMode::HighQualityFormat && !self.allows_high_quality {
                continue;
            }
            if self.started.insert(key.clone()) {
                added += 1;
            }
        }
        added
    }

    /// Cancels the listed preparations. Keys that were never started are ignored. Returns how many were cancelled.
    pub fn stop(&mut self, keys: &[Key]) -> usize {
        keys.iter().filter(|k| self.started.remove(*k)).count()
    }

    /// Cancels every preparation.
    pub fn stop_all(&mut self) {
        self.started.clear();
    }

    pub fn is_started(&self, key: &Key) -> bool {
        self.started.contains(key)
    }

    pub fn len(&self) -> usize {
        self.started.len()
    }

    pub fn is_empty(&self) -> bool {
        self.started.is_empty()
    }
}

/// The indexes to prepare for a list of `count` rows when `visible` rows are on screen and `ahead` rows past each
/// edge are wanted too. The result is clamped to the list.
pub fn preheat_window(visible: Range<usize>, count: usize, ahead: usize) -> Range<usize> {
    let end = visible.end.min(count);
    let start = visible.start.min(end);
    start.saturating_sub(ahead)..end.saturating_add(ahead).min(count)
}

/// Going from the `old` window to the `new` one: the indexes to start (in `new` but not `old`) and the indexes to
/// stop (in `old` but not `new`), each in ascending order.
pub fn changes(old: Range<usize>, new: Range<usize>) -> (Vec<usize>, Vec<usize>) {
    let to_start = new.clone().filter(|i| !old.contains(i)).collect();
    let to_stop = old.filter(|i| !new.contains(i)).collect();
    (to_start, to_stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(asset: &str) -> Key {
        Key::new(asset, 96, 96)
    }

    #[test]
    fn starting_twice_is_one_preparation() {
        let mut m = Manager::new();
        assert_eq!(m.start(&[key("a"), key("b")]), 2);
        assert_eq!(m.start(&[key("b"), key("c")]), 1);
        assert_eq!(m.len(), 3);
        // the same asset at another size is another preparation
        let big = Key::new("a", 512, 512);
        assert_eq!(m.start(&[big.clone()]), 1);
        assert!(m.is_started(&big) && m.is_started(&key("a")));
    }

    #[test]
    fn stop_cancels_listed_keys_and_ignores_the_rest() {
        let mut m = Manager::new();
        m.start(&[key("a"), key("b"), key("c")]);
        assert_eq!(m.stop(&[key("b"), key("zzz")]), 1);
        assert!(!m.is_started(&key("b")));
        assert_eq!(m.len(), 2);
        m.stop_all();
        assert!(m.is_empty());
    }

    #[test]
    fn high_quality_keys_are_skipped_when_not_allowed() {
        let mut m = Manager::new();
        assert!(m.allows_caching_high_quality_images(), "allowed by default");
        m.set_allows_caching_high_quality_images(false);
        let mut hq = key("a");
        hq.delivery = DeliveryMode::HighQualityFormat;
        let mut fast = key("a");
        fast.delivery = DeliveryMode::FastFormat;
        assert_eq!(m.start(&[hq.clone(), fast.clone()]), 1);
        assert!(!m.is_started(&hq) && m.is_started(&fast));
    }

    #[test]
    fn the_default_delivery_mode_is_opportunistic() {
        assert_eq!(DeliveryMode::default(), DeliveryMode::Opportunistic);
        assert_eq!(key("a").delivery, DeliveryMode::Opportunistic);
        assert_eq!(key("a").content_mode, ContentMode::AspectFill);
    }

    #[test]
    fn preheat_window_adds_rows_on_both_edges_and_clamps() {
        assert_eq!(preheat_window(5..8, 20, 3), 2..11);
        assert_eq!(preheat_window(0..4, 20, 3), 0..7, "no rows before the first");
        assert_eq!(preheat_window(18..20, 20, 3), 15..20, "no rows past the last");
        assert_eq!(preheat_window(8..30, 20, 0), 8..20, "a visible end past the list is cut");
        assert_eq!(preheat_window(25..30, 20, 2), 18..20, "a window past the list is clamped to its last rows");
        assert!(preheat_window(0..0, 0, 5).is_empty());
    }

    #[test]
    fn changes_list_the_rows_that_entered_and_left() {
        assert_eq!(changes(2..5, 4..7), (vec![5, 6], vec![2, 3]));
        assert_eq!(changes(0..0, 1..3), (vec![1, 2], vec![]));
        assert_eq!(changes(1..3, 1..3), (vec![], vec![]));
        assert_eq!(changes(0..2, 5..6), (vec![5], vec![0, 1]), "a jump starts and stops whole windows");
    }
}
