//! Managed background downloads: a queue of asset packs that the system fetches on an app's behalf, also while the app
//! is not running. The queue decides what may start now. A download made while the app is running is user-visible and
//! is not budgeted. A background download needs an unmetered connection and must fit the app's daily byte budget. Each
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
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// A pack with this id is already queued, downloading or done.
    Duplicate,
    /// The pack is larger than the whole daily budget, so it can never start in the background.
    ExceedsBudget,
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

    /// Adds a pack. Higher `priority` starts first; packs with equal priority start in request order.
    pub fn request(&mut self, pack: &str, bytes: u64, priority: u8) -> Result<(), RequestError> {
        if self.entries.iter().any(|e| e.pack == pack && e.state != PackState::Failed) {
            return Err(RequestError::Duplicate);
        }
        if bytes > self.daily_budget {
            return Err(RequestError::ExceedsBudget);
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
        });
        self.next_order += 1;
        Ok(())
    }

    /// The pack that should start now, or `None` when nothing queued is allowed under `conditions`.
    pub fn next_ready(&self, conditions: Conditions) -> Option<&str> {
        if !conditions.network_available {
            return None;
        }
        self.entries
            .iter()
            .filter(|e| e.state == PackState::Queued)
            .filter(|e| conditions.app_running || self.background_allowed(conditions, e.bytes))
            .max_by(|a, b| a.priority.cmp(&b.priority).then(b.order.cmp(&a.order)))
            .map(|e| e.pack.as_str())
    }

    fn background_allowed(&self, conditions: Conditions, bytes: u64) -> bool {
        conditions.unmetered && self.used_today.saturating_add(bytes) <= self.daily_budget
    }

    /// Marks a queued pack as downloading. Returns false if the pack is not queued.
    pub fn start(&mut self, pack: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.pack == pack && e.state == PackState::Queued) {
            Some(e) => {
                e.state = PackState::Downloading;
                e.attempts += 1;
                true
            }
            None => false,
        }
    }

    /// Marks a downloading pack as complete and charges its bytes to today's budget.
    pub fn finish(&mut self, pack: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.pack == pack && e.state == PackState::Downloading) {
            Some(e) => {
                e.state = PackState::Done;
                self.used_today = self.used_today.saturating_add(e.bytes);
                true
            }
            None => false,
        }
    }

    /// Records a failed transfer. The pack returns to the queue until it has used `MAX_ATTEMPTS` attempts.
    pub fn fail(&mut self, pack: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.pack == pack && e.state == PackState::Downloading) {
            Some(e) => {
                e.state = if e.attempts >= MAX_ATTEMPTS { PackState::Failed } else { PackState::Queued };
                true
            }
            None => false,
        }
    }

    /// The state of a pack, or `None` if it was never requested (or was requested and then replaced after failing).
    pub fn state(&self, pack: &str) -> Option<PackState> {
        self.entries.iter().find(|e| e.pack == pack).map(|e| e.state)
    }

    /// Bytes charged to the background budget today.
    pub fn used_today(&self) -> u64 {
        self.used_today
    }

    /// Starts a new day: the budget is available again.
    pub fn reset_day(&mut self) {
        self.used_today = 0;
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
        q.start("c");
        q.finish("c");
        assert_eq!(q.next_ready(FOREGROUND), Some("a"));
    }

    #[test]
    fn background_budget_blocks_packs_that_do_not_fit_today() {
        let mut q = ManagedDownloads::new(100);
        q.request("first", 60, 2).unwrap();
        q.request("second", 60, 1).unwrap();
        q.start("first");
        q.finish("first");
        assert_eq!(q.used_today(), 60);
        assert_eq!(q.next_ready(BACKGROUND_WIRED), None);
        q.reset_day();
        assert_eq!(q.next_ready(BACKGROUND_WIRED), Some("second"));
    }

    #[test]
    fn duplicate_and_oversized_requests_are_refused() {
        let mut q = ManagedDownloads::new(50);
        q.request("levels", 10, 1).unwrap();
        assert_eq!(q.request("levels", 10, 1), Err(RequestError::Duplicate));
        assert_eq!(q.request("huge", 51, 1), Err(RequestError::ExceedsBudget));
    }

    #[test]
    fn a_failing_pack_gets_three_attempts_then_fails() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert!(q.start("levels"));
            assert!(q.fail("levels"));
        }
        assert_eq!(q.state("levels"), Some(PackState::Failed));
        assert_eq!(q.next_ready(FOREGROUND), None);
        // A failed pack can be requested again with a fresh attempt count.
        q.request("levels", 10, 1).unwrap();
        assert_eq!(q.state("levels"), Some(PackState::Queued));
    }

    #[test]
    fn state_transitions_reject_out_of_order_calls() {
        let mut q = ManagedDownloads::new(100);
        q.request("levels", 10, 1).unwrap();
        assert!(!q.finish("levels"));
        assert!(!q.fail("levels"));
        assert!(q.start("levels"));
        assert!(!q.start("levels"));
        assert!(q.finish("levels"));
        assert_eq!(q.state("levels"), Some(PackState::Done));
        assert_eq!(q.state("missing"), None);
    }
}
