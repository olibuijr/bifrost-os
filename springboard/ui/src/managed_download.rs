//! Managed background downloads: a queue of asset packs that the system fetches on an app's behalf, also while the app
//! is not running. The queue decides what may start now. A download made while the app is running is user-visible and
//! is not budgeted. A background download needs an unmetered connection and must fit the app's daily byte budget; its
//! bytes are reserved when it starts, so downloads that run at the same time cannot overrun the budget together. Each
//! pack gets three attempts before it is marked failed. Pure state with no I/O: the transfer layer calls `start`,
//! `finish` and `fail`, and the day boundary calls `reset_day`.

/// Attempts a pack gets before it is marked failed.
pub const MAX_ATTEMPTS: u32 = 3;

/// What the system knows about the current network and the app at the moment a download could start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub network_available: bool,
    /// Whether the connection is not metered (wired or a plan without a data cap).
    pub unmetered: bool,
    /// Whether the owning app has a visible window or is in the foreground.
    pub app_running: bool,
}

/// Where one pack is in the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackState {
    Queued,
    Downloading,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    pack: String,
    bytes: u64,
    priority: u8,
    attempts: u32,
    state: PackState,
    order: u64,
    /// Bytes reserved against today's budget while this pack downloads in the background.
    reserved: u64,
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// A pack with this id is already queued, downloading or done.
    Duplicate,
}

/// The managed download queue for one app.
#[derive(Clone, Debug)]
pub struct ManagedDownloads {
    entries: Vec<Entry>,
    daily_budget: u64,
    used_today: u64,
    next_order: u64,
}

impl ManagedDownloads {
    /// A queue that may fetch at most `daily_budget` bytes per day in the background.
    pub fn new(daily_budget: u64) -> Self {
        Self { entries: Vec::new(), daily_budget, used_today: 0, next_order: 0 }
    }

    /// Adds a pack. Higher `priority` starts first; packs with equal priority start in request order. A pack larger
    /// than the daily budget is accepted, but it can only start while the app is running.
    pub fn request(&mut self, pack: &str, bytes: u64, priority: u8) -> Result<(), RequestError> {
        if self.entries.iter().any(|e| e.pack == pack && e.state != PackState::Failed) {
            return Err(RequestError::Duplicate);
        }
        // A failed pack that is requested again starts over with a fresh attempt count.
        self.entries.retain(|e| !(e.pack == pack && e.state == PackState::Failed));
        self.entries.push(Entry {
            pack: pack.to_string(),
            bytes,
            priority,
            attempts: 0,
            state: PackState::Queued,
            order: self.next_order,
            reserved: 0,
        });
        self.next_order += 1;
        Ok(())
    }

    /// The pack that should start now, or `None` when nothing queued is allowed under `conditions`.
    pub fn next_ready(&self, conditions: Conditions) -> Option<&str> {
        self.entries
            .iter()
            .filter(|e| e.state == PackState::Queued && self.may_start(conditions, e.bytes))
            .max_by(|a, b| a.priority.cmp(&b.priority).then(b.order.cmp(&a.order)))
            .map(|e| e.pack.as_str())
    }

    fn may_start(&self, conditions: Conditions, bytes: u64) -> bool {
        conditions.network_available
            && (conditions.app_running
                || (conditions.unmetered && self.used_today.saturating_add(bytes) <= self.daily_budget))
    }

    /// Marks a queued pack as downloading if `conditions` allow it. A background start reserves the pack's bytes
    /// against today's budget. Returns false if the pack is not queued or may not start under `conditions`.
    pub fn start(&mut self, pack: &str, conditions: Conditions) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.pack == pack && e.state == PackState::Queued) else {
            return false;
        };
        let bytes = self.entries[index].bytes;
        if !self.may_start(conditions, bytes) {
            return false;
        }
        let reserved = if conditions.app_running { 0 } else { bytes };
        self.used_today = self.used_today.saturating_add(reserved);
        let e = &mut self.entries[index];
        e.state = PackState::Downloading;
        e.attempts += 1;
        e.reserved = reserved;
        true
    }

    /// Marks a downloading pack as complete. Its reservation stays charged to today's budget.
    pub fn finish(&mut self, pack: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.pack == pack && e.state == PackState::Downloading) {
            Some(e) => {
                e.state = PackState::Done;
                e.reserved = 0;
                true
            }
            None => false,
        }
    }

    /// Records a failed transfer and releases the pack's reservation. The pack returns to the queue until it has used
    /// `MAX_ATTEMPTS` attempts.
    pub fn fail(&mut self, pack: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.pack == pack && e.state == PackState::Downloading) {
            Some(e) => {
                e.state = if e.attempts >= MAX_ATTEMPTS { PackState::Failed } else { PackState::Queued };
                self.used_today = self.used_today.saturating_sub(e.reserved);
                e.reserved = 0;
                true
            }
            None => false,
        }
    }

    /// The state of a pack, or `None` if it was never requested (or was requested and then replaced after failing).
    pub fn state(&self, pack: &str) -> Option<PackState> {
        self.entries.iter().find(|e| e.pack == pack).map(|e| e.state)
    }

    /// Bytes reserved or used by background downloads today.
    pub fn used_today(&self) -> u64 {
        self.used_today
    }

    /// Starts a new day: the budget is available again, except for downloads still running in the background.
    pub fn reset_day(&mut self) {
        self.used_today = self.entries.iter().map(|e| e.reserved).sum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOREGROUND: Conditions = Conditions { network_available: true, unmetered: false, app_running: true };
    const BACKGROUND_WIRED: Conditions = Conditions { network_available: true, unmetered: true, app_running: false };
    const BACKGROUND_METERED: Conditions = Conditions { network_available: true, unmetered: false, app_running: false };

    #[test]
    fn foreground_downloads_ignore_the_budget_and_metering() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 90, 1).unwrap();
        assert_eq!(q.next_ready(FOREGROUND), Some("levels"));
    }

    #[test]
    fn background_downloads_need_an_unmetered_connection() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        assert_eq!(q.next_ready(BACKGROUND_METERED), None);
        assert_eq!(q.next_ready(BACKGROUND_WIRED), Some("levels"));
    }

    #[test]
    fn nothing_starts_without_a_network() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        let offline = Conditions { network_available: false, ..FOREGROUND };
        assert_eq!(q.next_ready(offline), None);
    }

    #[test]
    fn higher_priority_starts_first_and_ties_keep_request_order() {
        let mut q = ManagedDownloads::new(1000);
        q.request("a", 1, 1).unwrap();
        q.request("b", 1, 1).unwrap();
        q.request("c", 1, 5).unwrap();
        assert_eq!(q.next_ready(FOREGROUND), Some("c"));
        assert!(q.start("c", FOREGROUND));
        assert!(q.finish("c"));
        assert_eq!(q.next_ready(FOREGROUND), Some("a"));
        assert!(q.start("a", FOREGROUND));
        assert_eq!(q.next_ready(FOREGROUND), Some("b"));
    }

    #[test]
    fn background_budget_blocks_packs_that_do_not_fit_today() {
        let mut q = ManagedDownloads::new(100);
        q.request("first", 60, 2).unwrap();
        q.request("second", 60, 1).unwrap();
        assert!(q.start("first", BACKGROUND_WIRED));
        assert!(q.finish("first"));
        assert_eq!(q.used_today(), 60);
        assert_eq!(q.next_ready(BACKGROUND_WIRED), None);
        assert!(!q.start("second", BACKGROUND_WIRED));
        q.reset_day();
        assert_eq!(q.next_ready(BACKGROUND_WIRED), Some("second"));
    }

    #[test]
    fn concurrent_background_downloads_cannot_overrun_the_budget() {
        let mut q = ManagedDownloads::new(100);
        q.request("first", 60, 2).unwrap();
        q.request("second", 60, 1).unwrap();
        assert!(q.start("first", BACKGROUND_WIRED));
        // The first pack is still downloading; its bytes already count.
        assert_eq!(q.used_today(), 60);
        assert_eq!(q.next_ready(BACKGROUND_WIRED), None);
        assert!(!q.start("second", BACKGROUND_WIRED));
    }

    #[test]
    fn foreground_downloads_are_not_charged_to_the_background_budget() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 90, 1).unwrap();
        assert!(q.start("levels", FOREGROUND));
        assert!(q.finish("levels"));
        assert_eq!(q.used_today(), 0);
    }

    #[test]
    fn start_refuses_what_next_ready_would_not_offer() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        assert!(!q.start("levels", BACKGROUND_METERED));
        let offline = Conditions { network_available: false, ..FOREGROUND };
        assert!(!q.start("levels", offline));
        assert_eq!(q.state("levels"), Some(PackState::Queued));
    }

    #[test]
    fn a_failed_transfer_releases_its_reservation() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 60, 1).unwrap();
        assert!(q.start("levels", BACKGROUND_WIRED));
        assert_eq!(q.used_today(), 60);
        assert!(q.fail("levels"));
        assert_eq!(q.used_today(), 0);
        assert_eq!(q.next_ready(BACKGROUND_WIRED), Some("levels"));
    }

    #[test]
    fn a_pack_larger_than_the_budget_waits_for_the_app_to_run() {
        let mut q = ManagedDownloads::new(50);
        q.request("huge", 51, 1).unwrap();
        assert_eq!(q.next_ready(BACKGROUND_WIRED), None);
        assert_eq!(q.next_ready(FOREGROUND), Some("huge"));
    }

    #[test]
    fn duplicate_requests_are_refused() {
        let mut q = ManagedDownloads::new(50);
        q.request("levels", 10, 1).unwrap();
        assert_eq!(q.request("levels", 10, 1), Err(RequestError::Duplicate));
    }

    #[test]
    fn a_failing_pack_gets_three_attempts_then_fails() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        for attempt in 1..=MAX_ATTEMPTS {
            assert!(q.start("levels", FOREGROUND));
            assert!(q.fail("levels"));
            let expected = if attempt < MAX_ATTEMPTS { PackState::Queued } else { PackState::Failed };
            assert_eq!(q.state("levels"), Some(expected));
        }
        assert_eq!(q.next_ready(FOREGROUND), None);
        assert!(!q.start("levels", FOREGROUND));
        // A failed pack can be requested again with a fresh attempt count.
        q.request("levels", 10, 1).unwrap();
        assert_eq!(q.state("levels"), Some(PackState::Queued));
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert!(q.start("levels", FOREGROUND));
            assert!(q.fail("levels"));
        }
        assert_eq!(q.state("levels"), Some(PackState::Queued));
    }

    #[test]
    fn state_transitions_reject_out_of_order_calls() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        assert!(!q.finish("levels"));
        assert!(!q.fail("levels"));
        assert!(q.start("levels", FOREGROUND));
        assert!(!q.start("levels", FOREGROUND));
        assert!(q.finish("levels"));
        assert_eq!(q.state("levels"), Some(PackState::Done));
        assert_eq!(q.state("missing"), None);
    }
}
