//! The play queue an app keeps for playback requests. A request's plan (play, add to queue, or show results) is applied
//! to the queue: play replaces the queue and starts at its first item, add to queue appends without interrupting the
//! current item, and show leaves the queue alone. The queue moves forward through its items and stops at the end; it
//! does not wrap or shuffle, so the same requests always leave the same queue.

use crate::media_request::PlaybackAction;
use crate::media_search::MediaItem;

/// The items to play, in order, and the position of the item now playing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackQueue {
    items: Vec<MediaItem>,
    current: Option<usize>,
}

impl PlaybackQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies the plan of a request. Returns the item now playing after the plan, or `None` when nothing plays.
    pub fn apply(&mut self, action: &PlaybackAction<'_>) -> Option<&MediaItem> {
        match action {
            PlaybackAction::Play(items) => {
                self.items = items.iter().map(|item| (*item).clone()).collect();
                self.current = if self.items.is_empty() { None } else { Some(0) };
            }
            PlaybackAction::Enqueue(items) => {
                let was_empty = self.items.is_empty();
                self.items.extend(items.iter().map(|item| (*item).clone()));
                if was_empty && !self.items.is_empty() {
                    self.current = Some(0);
                }
            }
            PlaybackAction::Show(_) | PlaybackAction::NoMatch => return None,
        }
        self.current()
    }

    /// The item now playing.
    pub fn current(&self) -> Option<&MediaItem> {
        self.current.and_then(|i| self.items.get(i))
    }

    /// The items after the one now playing, in play order.
    pub fn upcoming(&self) -> &[MediaItem] {
        match self.current {
            Some(i) => &self.items[i + 1..],
            None => &[],
        }
    }

    /// Moves to the next item. Returns it, or `None` and stays on the last item when the queue has ended.
    pub fn advance(&mut self) -> Option<&MediaItem> {
        let next = self.current? + 1;
        if next >= self.items.len() {
            return None;
        }
        self.current = Some(next);
        self.current()
    }

    /// Removes every item and stops.
    pub fn clear(&mut self) {
        self.items.clear();
        self.current = None;
    }

    /// The number of items, including the one now playing.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
