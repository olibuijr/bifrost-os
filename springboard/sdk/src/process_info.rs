//! Foundation `ProcessInfo` parity: sudden termination (counted like's: an app starts with termination
//! disabled and opts in with one `enable`; the shell hears of it only when the state flips), `physicalMemory`,
//! `processorCount`, `systemUptime`, `thermalState`, `isLowPowerModeEnabled`, their change notifications, and
//! `beginActivity` / `endActivity`. The Mach-style counters live in `bifrost_core::host` (`bifrost_app::host`).
use crate::dispatch::{DispatchQueue, Qos};
use bifrost_core::host;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

/// The counter. Termination is allowed only while the count is zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuddenTermination {
    disabled: u32,
}

impl Default for SuddenTermination {
    fn default() -> Self {
        SuddenTermination { disabled: 1 }
    }
}

impl SuddenTermination {
    pub fn is_enabled(&self) -> bool {
        self.disabled == 0
    }
    /// Balances one `disable`. Returns the new state when it flipped. An unbalanced enable is ignored.
    pub fn enable(&mut self) -> Option<bool> {
        if self.disabled == 0 {
            return None;
        }
        self.disabled -= 1;
        (self.disabled == 0).then_some(true)
    }
    /// Returns the new state when it flipped.
    pub fn disable(&mut self) -> Option<bool> {
        self.disabled = self.disabled.saturating_add(1);
        (self.disabled == 1).then_some(false)
    }
}

// ---- thermal state --------------------------------------------------------------------------------------------

// The model lives in `bifrost_core::thermal` (the shell's `power` service reads it too).
pub use bifrost_core::thermal::{thermal_state_for, ThermalState, TripPoint};

/// The worst state over every `/sys/class/thermal/thermal_zone*`.
fn read_thermal_state() -> ThermalState {
    bifrost_core::thermal::worst_in(std::path::Path::new("/sys/class/thermal"))
}

// ---- low power mode --------------------------------------------------------------------------------------------

/// Where the shell publishes its Low Power Mode setting (one `low_power=0|1` line, rewritten when it changes).
pub const POWER_STATE_PATH: &str = "/run/bifrost/power-state";

pub fn parse_low_power(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "low_power=1")
}

fn read_low_power() -> bool {
    std::fs::read_to_string(POWER_STATE_PATH).map_or(false, |t| parse_low_power(&t))
}

// ---- activities ------------------------------------------------------------------------------------------------

/// `ProcessInfo.ActivityOptions` (bits in `bifrost_proto::activity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityOptions(pub u32);

impl ActivityOptions {
    pub const USER_INITIATED: ActivityOptions = ActivityOptions(bifrost_proto::activity::USER_INITIATED);
    pub const LATENCY_CRITICAL: ActivityOptions = ActivityOptions(bifrost_proto::activity::LATENCY_CRITICAL);
    pub const BACKGROUND: ActivityOptions = ActivityOptions(bifrost_proto::activity::BACKGROUND);
    pub const IDLE_SYSTEM_SLEEP_DISABLED: ActivityOptions = ActivityOptions(bifrost_proto::activity::IDLE_SYSTEM_SLEEP_DISABLED);
    pub const fn union(self, o: ActivityOptions) -> ActivityOptions {
        ActivityOptions(self.0 | o.0)
    }
}

/// The token `beginActivity` returns; pass it to `endActivity`.
#[derive(Debug, PartialEq, Eq)]
pub struct Activity(u32);

/// The live activities. The shell hears only of the union of their options, when it changes.
#[derive(Debug, Default)]
pub struct ActivityTracker {
    next: u32,
    live: BTreeMap<u32, (u32, String)>,
}

impl ActivityTracker {
    fn union(&self) -> u32 {
        self.live.values().fold(0, |a, (o, _)| a | o)
    }
    /// Returns the token and the new union when it changed.
    pub fn begin(&mut self, options: ActivityOptions, reason: &str) -> (Activity, Option<u32>) {
        let before = self.union();
        self.next += 1;
        self.live.insert(self.next, (options.0, reason.to_owned()));
        let after = self.union();
        (Activity(self.next), (after != before).then_some(after))
    }
    /// Returns the new union when it changed. An unknown token is ignored.
    pub fn end(&mut self, a: Activity) -> Option<u32> {
        let before = self.union();
        self.live.remove(&a.0)?;
        let after = self.union();
        (after != before).then_some(after)
    }
    pub fn count(&self) -> usize {
        self.live.len()
    }
}

// ---- change notifications --------------------------------------------------------------------------------------

type ThermalHandler = Arc<dyn Fn(ThermalState) + Send + Sync>;
type PowerHandler = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Default)]
struct Watch {
    thermal: Vec<(DispatchQueue, ThermalHandler)>,
    power: Vec<(DispatchQueue, PowerHandler)>,
    last_thermal: Option<ThermalState>,
    last_power: Option<bool>,
    armed: bool,
}

/// How often subscribers' state is re-read, and how far the timer may slip to coalesce with other timers.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(10);
pub const WATCH_LEEWAY: Duration = Duration::from_secs(5);

fn watch() -> MutexGuard<'static, Watch> {
    static STATE: OnceLock<Mutex<Watch>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Watch::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs only while someone subscribed: reads just the states that have subscribers, notifies on change, re-arms.
fn poll_watchers() {
    let mut w = watch();
    let mut jobs: Vec<(DispatchQueue, Box<dyn FnOnce() + Send>)> = Vec::new();
    if !w.thermal.is_empty() {
        let now = read_thermal_state();
        if w.last_thermal.is_some_and(|l| l != now) {
            for (q, h) in &w.thermal {
                let h = h.clone();
                jobs.push((q.clone(), Box::new(move || h(now))));
            }
        }
        w.last_thermal = Some(now);
    }
    if !w.power.is_empty() {
        let now = read_low_power();
        if w.last_power.is_some_and(|l| l != now) {
            for (q, h) in &w.power {
                let h = h.clone();
                jobs.push((q.clone(), Box::new(move || h(now))));
            }
        }
        w.last_power = Some(now);
    }
    w.armed = !w.thermal.is_empty() || !w.power.is_empty();
    let again = w.armed;
    drop(w);
    for (q, j) in jobs {
        q.async_(j);
    }
    if again {
        DispatchQueue::global(Qos::Utility).async_after_leeway(WATCH_INTERVAL, WATCH_LEEWAY, poll_watchers);
    }
}

fn arm(w: &mut Watch) {
    if !w.armed {
        w.armed = true;
        DispatchQueue::global(Qos::Utility).async_after_leeway(WATCH_INTERVAL, WATCH_LEEWAY, poll_watchers);
    }
}

/// Foundation `ProcessInfo.processInfo`: process-wide system facts. All reads are cheap procfs/sysfs reads.
pub struct ProcessInfo;

impl ProcessInfo {
    /// `physicalMemory`, bytes.
    pub fn physical_memory() -> u64 {
        host::physical_memory()
    }
    /// `processorCount`.
    pub fn processor_count() -> usize {
        host::processor_count()
    }
    /// `activeProcessorCount`.
    pub fn active_processor_count() -> usize {
        host::active_processor_count()
    }
    /// `systemUptime`, seconds.
    pub fn system_uptime() -> f64 {
        host::HostSampler::new().system_uptime()
    }
    /// `thermalState`.
    pub fn thermal_state() -> ThermalState {
        read_thermal_state()
    }
    /// `isLowPowerModeEnabled`: the shell's Low Power Mode setting.
    pub fn is_low_power_mode_enabled() -> bool {
        read_low_power()
    }
    /// `thermalStateDidChangeNotification`: `handler` runs on `queue` (usually the main queue) when the state
    /// changes. Nothing is read until someone subscribes; then one timer re-reads every [`WATCH_INTERVAL`] (leeway
    /// [`WATCH_LEEWAY`], so it coalesces with other timers). Handlers live for the process.
    pub fn observe_thermal_state(queue: &DispatchQueue, handler: impl Fn(ThermalState) + Send + Sync + 'static) {
        let mut w = watch();
        w.thermal.push((queue.clone(), Arc::new(handler)));
        if w.last_thermal.is_none() {
            w.last_thermal = Some(read_thermal_state());
        }
        arm(&mut w);
    }
    /// `powerStateDidChangeNotification`: `handler(is_low_power_mode_enabled)` runs on `queue` on a change.
    pub fn observe_low_power_mode(queue: &DispatchQueue, handler: impl Fn(bool) + Send + Sync + 'static) {
        let mut w = watch();
        w.power.push((queue.clone(), Arc::new(handler)));
        if w.last_power.is_none() {
            w.last_power = Some(read_low_power());
        }
        arm(&mut w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_starts_disabled_and_flips_only_at_the_edges() {
        let mut s = SuddenTermination::default();
        assert!(!s.is_enabled());
        assert_eq!(s.enable(), Some(true));
        assert_eq!(s.enable(), None, "unbalanced enable is ignored");
        assert_eq!(s.disable(), Some(false));
        assert_eq!(s.disable(), None);
        assert_eq!(s.enable(), None, "still one disable outstanding");
        assert_eq!(s.enable(), Some(true));
        assert!(s.is_enabled());
    }

    #[test]
    fn low_power_state_file_parses() {
        assert!(parse_low_power("low_power=1\n"));
        assert!(!parse_low_power("low_power=0\n"));
        assert!(!parse_low_power(""));
    }

    #[test]
    fn activities_report_only_changes_of_the_union() {
        let mut t = ActivityTracker::default();
        let (a, u) = t.begin(ActivityOptions::USER_INITIATED, "export");
        assert_eq!(u, Some(1));
        let (b, u) = t.begin(ActivityOptions::USER_INITIATED.union(ActivityOptions::LATENCY_CRITICAL), "audio");
        assert_eq!(u, Some(3));
        assert_eq!(t.end(a), None, "the second activity still holds the user-initiated bit");
        assert_eq!(t.count(), 1);
        assert_eq!(t.end(b), Some(0));
        assert_eq!(t.end(Activity(99)), None);
    }

    #[test]
    fn process_info_counters_are_sane() {
        assert!(ProcessInfo::physical_memory() > 0);
        assert!(ProcessInfo::processor_count() >= 1);
        assert!(ProcessInfo::system_uptime() > 0.0);
    }
}
