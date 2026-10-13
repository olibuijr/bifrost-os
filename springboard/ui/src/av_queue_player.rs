//! Queue player (items played in order) and looper (an item repeated). Built on `Player`: the queue advances when
//! the current item plays to its end, keeping the rate.

use crate::av_asset::Asset;
use crate::av_player::{ActionAtItemEnd, Notification, Player, PlayerItem};

#[derive(Clone, Debug)]
pub struct QueuePlayer {
    pub player: Player,
    queue: Vec<PlayerItem>,
    loop_asset: Option<Asset>,
}

impl QueuePlayer {
    pub fn new(mut items: Vec<PlayerItem>) -> Self {
        let mut player = Player::new();
        player.action_at_item_end = ActionAtItemEnd::Advance;
        if !items.is_empty() {
            player.replace_current_item(Some(items.remove(0)));
        }
        QueuePlayer { player, queue: items, loop_asset: None }
    }
    /// A looper: plays `asset` over and over once playing starts.
    pub fn looper(asset: Asset) -> Self {
        let mut q = QueuePlayer::new(vec![PlayerItem::new(asset.clone()), PlayerItem::new(asset.clone())]);
        q.loop_asset = Some(asset);
        q
    }
    /// Items after the current one.
    pub fn items(&self) -> &[PlayerItem] {
        &self.queue
    }
    /// Inserts after the pending item at `after`, or at the front of the pending list for `None`.
    pub fn insert(&mut self, item: PlayerItem, after: Option<usize>) {
        let at = after.map_or(0, |i| (i + 1).min(self.queue.len()));
        self.queue.insert(at, item);
        if self.player.current_item().is_none() {
            self.advance_to_next_item();
        }
    }
    pub fn remove(&mut self, index: usize) -> Option<PlayerItem> {
        (index < self.queue.len()).then(|| self.queue.remove(index))
    }
    pub fn remove_all_items(&mut self) {
        self.queue.clear();
        self.player.replace_current_item(None);
    }
    pub fn advance_to_next_item(&mut self) {
        let rate = self.player.rate();
        let next = (!self.queue.is_empty()).then(|| self.queue.remove(0));
        self.player.replace_current_item(next);
        if let Some(a) = &self.loop_asset {
            self.queue.push(PlayerItem::new(a.clone()));
        }
        if self.player.current_item().is_some() {
            self.player.set_rate(rate);
        } else {
            self.player.pause();
        }
    }
    /// Advances the clock; moves to the next item when the current one finished.
    pub fn advance(&mut self, dt_us: i64) -> Vec<Notification> {
        self.player.advance(dt_us);
        let notes = self.player.take_notifications();
        if notes.contains(&Notification::DidPlayToEndTime) && self.player.action_at_item_end == ActionAtItemEnd::Advance {
            self.advance_to_next_item();
        }
        notes
    }
}
