//! Playback model: player item, player with rate and time-control status, seeking with tolerance, periodic and
//! boundary time observers, the did-play-to-end notification and the action at item end.
//! Driven by `advance(dt_us)` on a caller clock, so tests use a fake clock. Nothing starts playing until `play()`
//! or a non-zero `set_rate`: creating a player or setting an item never produces sound.

use crate::av_asset::Asset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemStatus {
    Unknown,
    ReadyToPlay,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerStatus {
    Unknown,
    ReadyToPlay,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeControlStatus {
    Paused,
    WaitingToPlayAtSpecifiedRate,
    Playing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionAtItemEnd {
    Advance,
    Pause,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification {
    /// Posted when an item plays to its end.
    DidPlayToEndTime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetadataItem {
    pub identifier: String,
    pub value: String,
}

/// Metadata items that apply from `start_us`.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedMetadataGroup {
    pub start_us: i64,
    pub items: Vec<MetadataItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerItem {
    asset: Asset,
    pub metadata_groups: Vec<TimedMetadataGroup>,
}

impl PlayerItem {
    pub fn new(asset: Asset) -> Self {
        PlayerItem { asset, metadata_groups: Vec::new() }
    }
    pub fn asset(&self) -> &Asset {
        &self.asset
    }
    pub fn status(&self) -> ItemStatus {
        if self.asset.pcm().is_some() { ItemStatus::ReadyToPlay } else { ItemStatus::Failed }
    }
    pub fn duration_us(&self) -> i64 {
        self.asset.duration_us().unwrap_or(0)
    }
    pub fn loaded_time_ranges(&self) -> Vec<(i64, i64)> {
        if self.duration_us() > 0 { vec![(0, self.duration_us())] } else { Vec::new() }
    }
    pub fn seekable_time_ranges(&self) -> Vec<(i64, i64)> {
        self.loaded_time_ranges()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverEvent {
    pub id: ObserverId,
    pub time_us: i64,
}

#[derive(Clone, Debug)]
enum Observer {
    Periodic { id: u32, interval_us: i64 },
    Boundary { id: u32, times: Vec<i64> },
}

#[derive(Clone, Debug)]
pub struct Player {
    item: Option<PlayerItem>,
    rate: f32,
    default_rate: f32,
    current_us: i64,
    pub volume: f32,
    pub is_muted: bool,
    pub action_at_item_end: ActionAtItemEnd,
    observers: Vec<Observer>,
    next_id: u32,
    events: Vec<ObserverEvent>,
    notifications: Vec<Notification>,
    metadata: Vec<TimedMetadataGroup>,
    ended: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Player {
            item: None,
            rate: 0.0,
            default_rate: 1.0,
            current_us: 0,
            volume: 1.0,
            is_muted: false,
            action_at_item_end: ActionAtItemEnd::Pause,
            observers: Vec::new(),
            next_id: 1,
            events: Vec::new(),
            notifications: Vec::new(),
            metadata: Vec::new(),
            ended: false,
        }
    }
    pub fn with_item(item: PlayerItem) -> Self {
        let mut p = Self::new();
        p.replace_current_item(Some(item));
        p
    }
    pub fn replace_current_item(&mut self, item: Option<PlayerItem>) {
        self.item = item;
        self.current_us = 0;
        self.ended = false;
    }
    pub fn current_item(&self) -> Option<&PlayerItem> {
        self.item.as_ref()
    }
    pub fn status(&self) -> PlayerStatus {
        match self.item.as_ref().map(PlayerItem::status) {
            Some(ItemStatus::ReadyToPlay) => PlayerStatus::ReadyToPlay,
            Some(ItemStatus::Failed) => PlayerStatus::Failed,
            _ => PlayerStatus::Unknown,
        }
    }
    pub fn rate(&self) -> f32 {
        self.rate
    }
    pub fn default_rate(&self) -> f32 {
        self.default_rate
    }
    pub fn set_default_rate(&mut self, r: f32) {
        self.default_rate = r;
    }
    /// Sets the rate. Playing from the end of an item restarts it.
    pub fn set_rate(&mut self, r: f32) {
        self.rate = r;
        if r > 0.0 && self.ended && self.current_us >= self.duration() {
            self.current_us = 0;
            self.ended = false;
        }
    }
    pub fn play(&mut self) {
        self.set_rate(self.default_rate);
    }
    pub fn pause(&mut self) {
        self.rate = 0.0;
    }
    pub fn time_control_status(&self) -> TimeControlStatus {
        if self.rate == 0.0 {
            return TimeControlStatus::Paused;
        }
        match self.status() {
            PlayerStatus::ReadyToPlay => TimeControlStatus::Playing,
            PlayerStatus::Failed => TimeControlStatus::Paused,
            PlayerStatus::Unknown => TimeControlStatus::WaitingToPlayAtSpecifiedRate,
        }
    }
    pub fn current_time_us(&self) -> i64 {
        self.current_us
    }
    fn duration(&self) -> i64 {
        self.item.as_ref().map_or(0, PlayerItem::duration_us)
    }
    /// Seeks near `to_us`. The result is a sample boundary within both tolerances when one exists, else the
    /// nearest sample. Returns the time landed on.
    pub fn seek(&mut self, to_us: i64, tolerance_before_us: i64, tolerance_after_us: i64) -> i64 {
        let dur = self.duration();
        let to = to_us.clamp(0, dur);
        let rate = self.item.as_ref().and_then(|i| i.asset().pcm()).map_or(1_000_000, |p| p.sample_rate as i64);
        let lo_f = to * rate / 1_000_000;
        let lo = lo_f * 1_000_000 / rate;
        let hi = ((lo_f + 1) * 1_000_000 / rate).min(dur);
        let nearest = if to - lo <= hi - to { lo } else { hi };
        let ok = |t: i64| (t <= to && to - t <= tolerance_before_us) || (t >= to && t - to <= tolerance_after_us);
        let landed = if ok(nearest) {
            nearest
        } else if ok(lo) {
            lo
        } else if ok(hi) {
            hi
        } else {
            nearest
        };
        self.current_us = landed.clamp(0, dur);
        self.ended = false;
        self.current_us
    }
    /// A seek with zero tolerance.
    pub fn seek_exact(&mut self, to_us: i64) -> i64 {
        self.seek(to_us, 0, 0)
    }
    pub fn add_periodic_time_observer(&mut self, interval_us: i64) -> ObserverId {
        let id = self.alloc();
        self.observers.push(Observer::Periodic { id, interval_us: interval_us.max(1) });
        ObserverId(id)
    }
    pub fn add_boundary_time_observer(&mut self, times_us: &[i64]) -> ObserverId {
        let id = self.alloc();
        self.observers.push(Observer::Boundary { id, times: times_us.to_vec() });
        ObserverId(id)
    }
    pub fn remove_time_observer(&mut self, id: ObserverId) {
        self.observers.retain(|o| match o {
            Observer::Periodic { id: i, .. } | Observer::Boundary { id: i, .. } => *i != id.0,
        });
    }
    fn alloc(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id - 1
    }
    pub fn take_events(&mut self) -> Vec<ObserverEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.notifications)
    }
    /// Timed metadata groups whose start was crossed since the last call.
    pub fn take_metadata(&mut self) -> Vec<TimedMetadataGroup> {
        std::mem::take(&mut self.metadata)
    }
    /// Moves the clock on by `dt_us` of wall time. Only a playing player moves.
    pub fn advance(&mut self, dt_us: i64) {
        if self.time_control_status() != TimeControlStatus::Playing || self.ended {
            return;
        }
        let dur = self.duration();
        let prev = self.current_us;
        let target = prev + (dt_us as f64 * self.rate as f64) as i64;
        if target <= prev {
            self.current_us = target.max(0);
            return;
        }
        let reach = target.min(dur);
        self.fire(prev, reach);
        self.current_us = reach;
        if target >= dur {
            self.ended = true;
            self.notifications.push(Notification::DidPlayToEndTime);
            if self.action_at_item_end == ActionAtItemEnd::Pause {
                self.rate = 0.0;
            }
        }
    }
    fn fire(&mut self, from: i64, to: i64) {
        for o in &self.observers {
            match o {
                Observer::Periodic { id, interval_us } => {
                    let mut t = (from / interval_us + 1) * interval_us;
                    while t <= to {
                        self.events.push(ObserverEvent { id: ObserverId(*id), time_us: t });
                        t += interval_us;
                    }
                }
                Observer::Boundary { id, times } => {
                    // One callback per crossing, carrying the first boundary crossed.
                    if let Some(t) = times.iter().copied().filter(|t| *t > from && *t <= to).min() {
                        self.events.push(ObserverEvent { id: ObserverId(*id), time_us: t });
                    }
                }
            }
        }
        if let Some(item) = &self.item {
            for g in &item.metadata_groups {
                if g.start_us > from && g.start_us <= to {
                    self.metadata.push(g.clone());
                }
            }
        }
    }
}
