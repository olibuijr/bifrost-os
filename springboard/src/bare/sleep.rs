//! Suspend to RAM once the screen has been off for a while.
//!
//! /sys/power belongs to logind, so `Host::Logind` suspends with `systemctl suspend` (logind's
//! Suspend, which also runs the sleep hooks and honours "delay" inhibitors) and refuses while
//! another program holds a "sleep" block inhibitor. A worker thread makes the attempt once the
//! screen has been off for `after_ms`; anything that wakes us gets `GRACE_MS` to do its work
//! before we sleep again. The wake-lock and wakeup_count protocol of `try_suspend` is kept for
//! hosts that expose /sys/power directly (and for its tests). The lid switch is logind's too;
//! `LidInhibitor` only turns it off when the `lid_close=ignore` setting asks for that, and
//! `AcWatch` reports mains plug changes.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Screen-off time before the first suspend attempt.
pub const SUSPEND_AFTER_MS: f64 = 30_000.0;
/// Prefs key for the Energy idle timeout: "never" (default) or minutes. Display off and Lock Screen
/// never suspend the system (Golden Gate parity); only this timeout after the display went off, or the
/// menu's Sleep, does.
pub const SYSTEM_SLEEP_PREF: &str = "system_sleep_minutes";

/// Parses the Energy system-sleep pref into a delay in ms; `None` means never.
pub fn system_sleep_after(pref: Option<&str>) -> Option<f64> {
    let m: f64 = pref?.trim().parse().ok()?;
    (m > 0.0).then_some(m * 60_000.0)
}
/// Awake time after a resume that did not turn the screen on.
pub const GRACE_MS: f64 = 5_000.0;
const RETRY_MS: f64 = 1_000.0;
const MAX_RETRY_MS: f64 = 60_000.0;
/// Longest we wait for wakeup_count to become readable before giving up on this attempt.
const COUNT_TIMEOUT_MS: u64 = 2_000;
const WAKE_LOCK: &str = "springboard";
/// Held instead of suspending while audio plays or a download runs.
const BUSY_LOCK: &str = "springboard-busy";

/// Files under /sys/power; injectable so the protocol is testable.
pub trait PowerSys {
    fn read(&mut self, _file: &str) -> io::Result<String>;
    fn write(&mut self, file: &str, value: &str) -> io::Result<()>;
    /// Called on the worker thread right after a suspend returned, before
    /// the result is reported.
    fn resumed(&mut self) {}
    /// wakeup_count, but gives up after `timeout_ms` (the sysfs read blocks while
    /// any wakeup source is active). Errors with `TimedOut` then.
    fn wakeup_count(&mut self, _timeout_ms: u64) -> io::Result<String> {
        self.read("wakeup_count")
    }
    /// Names of wakeup sources that are active now, for the log of a refused suspend.
    fn active_sources(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Makes sure the power key can wake the device.
    fn arm_wake_sources(&mut self) {}
    /// Whether this host uses the /sys/power protocol (wake locks, Wi-Fi power save).
    fn bifrost(&self) -> bool {
        true
    }
}

/// "sleep" block inhibitors in `systemd-inhibit --list --no-legend` output. Columns are
/// `WHO UID USER PID COMM WHAT WHY MODE`, but WHO and WHY hold spaces, so the row is read from
/// the right (MODE) and the first token that is a colon list of inhibitor kinds is WHAT.
pub fn parse_sleep_blockers(list: &str) -> Vec<String> {
    const KINDS: [&str; 6] = ["shutdown", "sleep", "idle", "handle-power-key", "handle-suspend-key", "handle-lid-switch"];
    list.lines()
        .filter_map(|l| {
            let tok: Vec<&str> = l.split_whitespace().collect();
            if tok.last() != Some(&"block") {
                return None;
            }
            let what = tok.iter().position(|t| t.split(':').all(|k| KINDS.contains(&k) || k.starts_with("handle-")))?;
            tok[what].split(':').any(|k| k == "sleep").then(|| tok[..what].first().copied().unwrap_or("?").to_string())
        })
        .collect()
}

fn sleep_blockers() -> Vec<String> {
    let out = std::process::Command::new("systemd-inhibit").args(["--list", "--no-legend", "--no-pager"]).output();
    out.map(|o| parse_sleep_blockers(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// The apps that hold `ProcessInfo.ActivityOptions.idleSystemSleepDisabled`, by app id. The shell
/// fills it from each app's `ToShell::Activity` and empties it when the app is dropped.
static APP_SLEEP_HOLDS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn app_sleep_holds() -> std::sync::MutexGuard<'static, std::collections::BTreeSet<String>> {
    APP_SLEEP_HOLDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Records the activity union an app reported: the app holds a system sleep block while the
/// `idleSystemSleepDisabled` bit is set.
pub fn note_app_activity(app: &str, options: u32) {
    let mut holds = app_sleep_holds();
    if options & bifrost_proto::activity::IDLE_SYSTEM_SLEEP_DISABLED != 0 {
        holds.insert(app.to_string());
    } else {
        holds.remove(app);
    }
}

/// Drops an app's sleep block when the app goes away.
pub fn forget_app(app: &str) {
    app_sleep_holds().remove(app);
}

/// One blocker entry per app holding a sleep block, in the same form as logind's lock list.
pub fn app_sleep_blockers() -> Vec<String> {
    app_sleep_holds().iter().map(|app| format!("app {app}")).collect()
}

/// Suspends through logind; returns once the machine has resumed.
fn logind_suspend() -> io::Result<()> {
    crate::privd_client::connect().power("suspend")
}

/// Keeps logind from acting on the lid switch for as long as it lives (`lid_close=ignore`):
/// a block inhibitor held by a child process that is killed on drop.
pub struct LidInhibitor(Option<std::process::Child>);

impl LidInhibitor {
    /// `setting` is the `lid_close` value: "ignore" inhibits, anything else leaves logind alone.
    pub fn apply(setting: Option<&str>) -> Self {
        if setting != Some("ignore") {
            return LidInhibitor(None);
        }
        let child = std::process::Command::new("systemd-inhibit")
            .args(["--what=handle-lid-switch", "--who=SpringBoard", "--why=lid_close is set to ignore", "--mode=block", "sleep", "infinity"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| eprintln!("springboard: lid inhibitor: {e}"))
            .ok();
        LidInhibitor(child)
    }
}

impl Drop for LidInhibitor {
    fn drop(&mut self) {
        if let Some(c) = &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Mains plug changes, found by polling (sysfs raises no event we could wait on).
#[derive(Default)]
pub struct AcWatch {
    last: Option<bool>,
    next_ms: f64,
}

impl AcWatch {
    const POLL_MS: f64 = 2_000.0;

    /// Feeds the current charging state; Some(plugged) only on a change.
    pub fn feed(&mut self, plugged: bool) -> Option<bool> {
        let changed = self.last.is_some_and(|l| l != plugged);
        self.last = Some(plugged);
        changed.then_some(plugged)
    }

    /// Reads power::battery() at most every 2 s; Some(plugged) when the charger came or went.
    pub fn poll(&mut self, now_ms: f64) -> Option<bool> {
        if now_ms < self.next_ms {
            return None;
        }
        self.next_ms = now_ms + Self::POLL_MS;
        self.feed(super::power::battery().1)
    }
}

/// Active sources from /sys/kernel/debug/wakeup_sources: columns are
/// `name active_count event_count wakeup_count expire_count active_since
/// total_time max_time last_change prevent_sleep_time`; a source is active
/// while `active_since` (column 6) is not 0.
pub fn parse_active_sources(table: &str) -> Vec<String> {
    table
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?;
            let active_since: u64 = f.nth(4)?.parse().ok()?;
            (active_since > 0).then(|| name.to_string())
        })
        .collect()
}

/// How this machine suspends, or not at all: an emulator instance runs on a host
/// that must never be suspended, so every attempt there fails without touching /sys/power.
#[derive(Clone, Copy)]
pub enum Host {
    /// Suspend through logind.
    Logind,
    Inert,
}

impl Host {
    pub fn new(emulated: bool) -> Self {
        if emulated { Host::Inert } else { Host::Logind }
    }
}

impl PowerSys for Host {
    fn read(&mut self, _file: &str) -> io::Result<String> {
        match self {
            Host::Logind => Ok("0".into()),
            Host::Inert => Err(io::Error::other("emulated: no power files")),
        }
    }
    fn write(&mut self, file: &str, _value: &str) -> io::Result<()> {
        match self {
            // no wake locks or wakeup_count handshake: logind is the arbiter
            Host::Logind if file == "state" => {
                let mut blockers = sleep_blockers();
                blockers.extend(app_sleep_blockers());
                if blockers.is_empty() {
                    logind_suspend()
                } else {
                    Err(io::Error::other(format!("sleep inhibited by {}", blockers.join(", "))))
                }
            }
            Host::Logind => Ok(()),
            Host::Inert => Err(io::Error::other("emulated: no power files")),
        }
    }
    fn wakeup_count(&mut self, _timeout_ms: u64) -> io::Result<String> {
        match self {
            Host::Logind => Ok("0".into()),
            Host::Inert => Err(io::Error::other("emulated: no power files")),
        }
    }
    fn active_sources(&mut self) -> Vec<String> {
        match self {
            Host::Logind => {
                let mut blockers = sleep_blockers();
                blockers.extend(app_sleep_blockers());
                blockers
            }
            Host::Inert => Vec::new(),
        }
    }
    fn bifrost(&self) -> bool {
        !matches!(self, Host::Logind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Resumed,
    Failed,
}

/// " (active wakeup sources: a, b)" for the log, or nothing when none are known.
fn blockers(sys: &mut impl PowerSys) -> String {
    let v = sys.active_sources();
    if v.is_empty() {
        String::new()
    } else {
        format!(" (active wakeup sources: {})", v.join(", "))
    }
}

/// One suspend attempt; returns once the device has resumed or the kernel
/// refused. `armed` is checked before each write so a screen-on while the
/// count read was blocked cancels the attempt.
pub fn try_suspend(sys: &mut impl PowerSys, armed: impl Fn() -> bool) -> Outcome {
    let read = sys.wakeup_count(COUNT_TIMEOUT_MS);
    // A blocked read means a wakeup event is in progress; the kernel refuses state=mem in that case
    // anyway, so try without the handshake instead of never suspending.
    let blocked = matches!(&read, Err(e) if e.kind() == io::ErrorKind::TimedOut);
    if blocked {
        eprintln!("springboard: suspend: wakeup_count blocked, suspending without the handshake{}", blockers(sys));
    } else {
        let count = match read.map_err(|e| e.to_string()).and_then(|c| c.trim().parse::<u64>().map_err(|e| e.to_string())) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("springboard: suspend: reading wakeup_count: {e}{}", blockers(sys));
                return Outcome::Failed;
            }
        };
        if !armed() {
            return Outcome::Failed;
        }
        if let Err(e) = sys.write("wakeup_count", &count.to_string()) {
            eprintln!("springboard: suspend: writing wakeup_count {count}: {e} (a wakeup event arrived since the read){}", blockers(sys));
            return Outcome::Failed;
        }
    }
    if !armed() {
        return Outcome::Failed;
    }
    if let Err(e) = sys.write("state", "mem") {
        eprintln!("springboard: suspend: writing state=mem: {e}{}", blockers(sys));
        return Outcome::Failed;
    }
    Outcome::Resumed
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Suspend,
}

/// When to attempt a suspend: pure timing, no I/O.
pub struct Policy {
    after_ms: Option<f64>,
    off: bool,
    due: f64,
    backoff: f64,
    /// bumped on every screen change; results of older attempts only free the slot
    epoch: u32,
    in_flight: Option<u32>,
    /// audio playing or a download running: never suspend
    busy: bool,
}

impl Policy {
    pub fn new(after_ms: Option<f64>) -> Self {
        Policy { after_ms, off: false, due: 0.0, backoff: RETRY_MS, epoch: 0, in_flight: None, busy: false }
    }

    /// While busy no attempt starts; when it ends the grace period applies again.
    pub fn set_busy(&mut self, busy: bool, now: f64) {
        if self.busy && !busy {
            self.due = self.due.max(now + GRACE_MS);
            self.backoff = RETRY_MS;
        }
        self.busy = busy;
    }

    /// Changes the delay for the next screen-off (and the pending one, if the screen is off now).
    pub fn set_after(&mut self, after_ms: Option<f64>, now: f64) {
        self.after_ms = after_ms;
        if self.off && self.in_flight.is_none() {
            self.due = after_ms.map_or(f64::INFINITY, |a| now + a);
        }
    }

    pub fn screen_on(&mut self) {
        self.off = false;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn screen_off(&mut self, now: f64) {
        self.off = true;
        self.epoch = self.epoch.wrapping_add(1);
        self.due = self.after_ms.map_or(f64::INFINITY, |a| now + a);
        self.backoff = RETRY_MS;
    }

    /// Explicit Sleep (menu): suspend as soon as the screen is off.
    pub fn sleep_now(&mut self, now: f64) {
        self.off = true;
        self.due = now;
    }

    pub fn tick(&mut self, now: f64) -> Option<Action> {
        if !self.off || self.busy || self.in_flight.is_some() || now < self.due || self.due.is_infinite() {
            return None;
        }
        self.in_flight = Some(self.epoch);
        Some(Action::Suspend)
    }

    pub fn done(&mut self, now: f64, outcome: Outcome) {
        if self.in_flight.take() != Some(self.epoch) {
            return;
        }
        match outcome {
            Outcome::Resumed => {
                self.due = now + GRACE_MS;
                self.backoff = RETRY_MS;
            }
            Outcome::Failed => {
                self.due = now + self.backoff;
                self.backoff = (self.backoff * 2.0).min(MAX_RETRY_MS);
            }
        }
    }

    /// When `tick` next wants to run, if anything is pending.
    pub fn deadline(&self) -> Option<f64> {
        (self.off && !self.busy && self.in_flight.is_none() && self.due.is_finite()).then_some(self.due)
    }
}

/// Holds the wake lock while the screen is on and suspends on a worker
/// thread once `Policy` says so (one short-lived thread per attempt: an idle shell holds none).
pub struct Sleeper<P: PowerSys> {
    sys: P,
    policy: Policy,
    armed: Arc<AtomicBool>,
    done: Sender<Outcome>,
    results: Receiver<Outcome>,
    busy: bool,
}

impl<P: PowerSys + Clone + Send + 'static> Sleeper<P> {
    /// Starts with the screen on, taking the wake lock.
    pub fn new(sys: P, after_ms: Option<f64>) -> Self {
        let armed = Arc::new(AtomicBool::new(false));
        let (done, results) = channel();
        let mut s = Sleeper { sys, policy: Policy::new(after_ms), armed, done, results, busy: false };
        s.sys.arm_wake_sources();
        s.wake_lock(true);
        s
    }

    fn wake_lock(&mut self, hold: bool) {
        self.lock(WAKE_LOCK, hold);
    }

    fn lock(&mut self, name: &str, hold: bool) {
        let file = if hold { "wake_lock" } else { "wake_unlock" };
        if let Err(e) = self.sys.write(file, name) {
            eprintln!("springboard: {file}: {e}");
        }
    }

    /// Reports whether audio is playing or a download runs; holds a second wake
    /// lock for as long as that lasts and keeps the policy from suspending.
    /// Call every loop iteration; only changes do any work.
    pub fn set_busy(&mut self, busy: bool, now: f64) {
        if busy == self.busy {
            return;
        }
        self.busy = busy;
        self.lock(BUSY_LOCK, busy);
        self.policy.set_busy(busy, now);
    }

    /// Applies a changed Energy timeout without a restart.
    pub fn set_after(&mut self, after_ms: Option<f64>, now: f64) {
        self.policy.set_after(after_ms, now);
    }

    /// Takes the wake lock first, so a racing attempt is refused by the kernel.
    pub fn screen_on(&mut self, _now: f64) {
        self.armed.store(false, Ordering::SeqCst);
        self.wake_lock(true);
        self.policy.screen_on();
    }

    /// Explicit Sleep: call after `screen_off`.
    pub fn sleep_now(&mut self, now: f64) {
        self.policy.sleep_now(now);
    }

    pub fn screen_off(&mut self, now: f64) {
        self.policy.screen_off(now);
        if self.sys.bifrost() {
            super::power::wifi_power_save(true);
        }
        self.wake_lock(false);
    }

    /// Starts a due attempt; returns the result of one that finished.
    pub fn tick(&mut self, now: f64) -> Option<Outcome> {
        let finished = self.results.try_recv().ok();
        if let Some(o) = finished {
            self.policy.done(now, o);
        }
        if self.policy.tick(now) == Some(Action::Suspend) {
            self.armed.store(true, Ordering::SeqCst);
            let (mut worker, flag, tx) = (self.sys.clone(), self.armed.clone(), self.done.clone());
            let spawned = std::thread::Builder::new().name("suspend".into()).spawn(move || {
                let outcome = try_suspend(&mut worker, || flag.load(Ordering::SeqCst));
                if outcome == Outcome::Resumed {
                    worker.resumed();
                }
                let _ = tx.send(outcome);
            });
            if spawned.is_err() {
                self.policy.done(now, Outcome::Failed);
            }
        }
        finished
    }

    pub fn deadline(&self) -> Option<f64> {
        self.policy.deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_sleep_pref_parses_never_and_minutes() {
        assert_eq!(system_sleep_after(None), None);
        assert_eq!(system_sleep_after(Some("never")), None);
        assert_eq!(system_sleep_after(Some("0")), None);
        assert_eq!(system_sleep_after(Some("30")), Some(1_800_000.0));
    }

    #[test]
    fn display_off_never_suspends_when_sleep_is_never() {
        let mut p = Policy::new(None);
        p.screen_off(0.0);
        assert_eq!(p.tick(1e12), None);
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn explicit_sleep_suspends_even_when_never() {
        let mut p = Policy::new(None);
        p.screen_off(0.0);
        p.sleep_now(10.0);
        assert_eq!(p.tick(10.0), Some(Action::Suspend));
    }

    #[test]
    fn idle_timeout_suspends_after_display_off() {
        let mut p = Policy::new(Some(60_000.0));
        p.screen_off(0.0);
        assert_eq!(p.tick(59_999.0), None);
        assert_eq!(p.tick(60_000.0), Some(Action::Suspend));
    }

    #[test]
    fn an_emulated_host_cannot_suspend() {
        let mut h = Host::new(true);
        assert!(h.write("state", "mem").is_err());
        assert!(h.write("wake_lock", "springboard").is_err());
        assert!(h.read("wakeup_count").is_err());
        assert!(h.wakeup_count(10).is_err());
        assert!(h.active_sources().is_empty());
        assert!(matches!(Host::new(false), Host::Logind));
    }

    #[test]
    fn only_sleep_block_inhibitors_refuse_a_suspend() {
        let list = "NetworkManager 0 root 800 NetworkManager sleep NetworkManager needs to turn off networks delay
Some App 1000 u 4242 app handle-lid-switch:sleep Playing video block
Other 1000 u 5 x idle Screen saver block
Delay Only 0 root 1 y sleep because delay
";
        assert_eq!(parse_sleep_blockers(list), vec!["Some".to_string()]);
        assert!(parse_sleep_blockers("").is_empty());
    }

    #[test]
    fn ac_watch_reports_changes_only() {
        let mut a = AcWatch::default();
        assert_eq!(a.feed(true), None, "the first reading is the baseline");
        assert_eq!(a.feed(true), None);
        assert_eq!(a.feed(false), Some(false));
        assert_eq!(a.feed(true), Some(true));
    }

    #[test]
    fn lid_inhibitor_only_for_ignore() {
        assert!(LidInhibitor::apply(None).0.is_none());
        assert!(LidInhibitor::apply(Some("sleep")).0.is_none());
    }
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        count: Option<&'static str>,
        refuse: Option<&'static str>,
        count_blocks: bool,
    }

    impl Fake {
        fn new(count: Option<&'static str>, refuse: Option<&'static str>) -> Self {
            Fake { log: Arc::default(), count, refuse, count_blocks: false }
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
        }
    }

    impl PowerSys for Fake {
        fn read(&mut self, file: &str) -> io::Result<String> {
            self.log.lock().unwrap_or_else(|p| p.into_inner()).push(format!("read {file}"));
            match (file, self.count) {
                ("wakeup_count", Some(c)) => Ok(c.into()),
                _ => Err(io::ErrorKind::NotFound.into()),
            }
        }
        fn write(&mut self, file: &str, value: &str) -> io::Result<()> {
            self.log.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{file}={value}"));
            if self.refuse == Some(file) {
                return Err(io::Error::from_raw_os_error(16));
            }
            Ok(())
        }
        fn resumed(&mut self) {
            self.log.lock().unwrap_or_else(|p| p.into_inner()).push("resumed".into());
        }
        fn wakeup_count(&mut self, _timeout_ms: u64) -> io::Result<String> {
            if self.count_blocks {
                self.log.lock().unwrap_or_else(|p| p.into_inner()).push("read wakeup_count (blocked)".into());
                return Err(io::ErrorKind::TimedOut.into());
            }
            self.read("wakeup_count")
        }
    }

    #[test]
    fn suspends_with_the_wakeup_count_handshake() {
        let mut f = Fake::new(Some("42\n"), None);
        assert_eq!(try_suspend(&mut f, || true), Outcome::Resumed);
        assert_eq!(f.log(), ["read wakeup_count", "wakeup_count=42", "state=mem"]);
    }

    #[test]
    fn unreadable_or_garbled_count_fails_without_writing() {
        let mut f = Fake::new(None, None);
        assert_eq!(try_suspend(&mut f, || true), Outcome::Failed);
        assert_eq!(f.log(), ["read wakeup_count"]);
        let mut f = Fake::new(Some("x"), None);
        assert_eq!(try_suspend(&mut f, || true), Outcome::Failed);
        assert_eq!(f.log(), ["read wakeup_count"]);
    }

    #[test]
    fn blocked_count_read_falls_back_to_suspending_without_the_handshake() {
        let mut f = Fake::new(None, None);
        f.count_blocks = true;
        assert_eq!(try_suspend(&mut f, || true), Outcome::Resumed);
        assert_eq!(f.log(), ["read wakeup_count (blocked)", "state=mem"]);
        let mut f = Fake::new(None, Some("state"));
        f.count_blocks = true;
        assert_eq!(try_suspend(&mut f, || true), Outcome::Failed);
    }

    #[test]
    fn wakeup_event_race_aborts_before_suspending() {
        let mut f = Fake::new(Some("7"), Some("wakeup_count"));
        assert_eq!(try_suspend(&mut f, || true), Outcome::Failed);
        assert_eq!(f.log(), ["read wakeup_count", "wakeup_count=7"]);
    }

    #[test]
    fn busy_kernel_fails() {
        let mut f = Fake::new(Some("7"), Some("state"));
        assert_eq!(try_suspend(&mut f, || true), Outcome::Failed);
        assert_eq!(f.log(), ["read wakeup_count", "wakeup_count=7", "state=mem"]);
    }

    #[test]
    fn disarmed_attempt_never_writes() {
        let mut f = Fake::new(Some("7"), None);
        assert_eq!(try_suspend(&mut f, || false), Outcome::Failed);
        assert_eq!(f.log(), ["read wakeup_count"]);
    }

    #[test]
    fn parses_active_wakeup_sources() {
        let t = "name\tactive_count\tevent_count\twakeup_count\texpire_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_sleep_time\n\
                 wlan_rx\t3\t3\t0\t0\t1200\t10\t5\t1200\t0\n\
                 gpio-keys\t1\t1\t0\t0\t0\t2\t2\t900\t0\n";
        assert_eq!(parse_active_sources(t), ["wlan_rx"]);
        assert!(parse_active_sources("").is_empty());
    }

    #[test]
    fn busy_blocks_suspend_and_grants_grace_afterwards() {
        let mut p = Policy::new(Some(1000.0));
        p.screen_off(0.0);
        p.set_busy(true, 500.0);
        assert_eq!(p.tick(1e6), None);
        assert_eq!(p.deadline(), None);
        p.set_busy(false, 100_000.0);
        assert_eq!(p.deadline(), Some(100_000.0 + GRACE_MS));
        assert_eq!(p.tick(100_000.0 + GRACE_MS), Some(Action::Suspend));
    }

    #[test]
    fn sleeper_takes_the_busy_lock_only_on_changes() {
        let f = Fake::new(Some("3"), None);
        let mut s = Sleeper::new(f.clone(), Some(100.0));
        s.set_busy(true, 0.0);
        s.set_busy(true, 1.0);
        s.set_busy(false, 2.0);
        assert_eq!(f.log(), ["wake_lock=springboard", "wake_lock=springboard-busy", "wake_unlock=springboard-busy"]);
    }

    #[test]
    fn never_suspends_while_screen_is_on() {
        let mut p = Policy::new(Some(30_000.0));
        assert_eq!(p.tick(1e9), None);
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn suspends_after_screen_off_delay() {
        let mut p = Policy::new(Some(30_000.0));
        p.screen_off(1000.0);
        assert_eq!(p.deadline(), Some(31_000.0));
        assert_eq!(p.tick(30_999.0), None);
        assert_eq!(p.tick(31_000.0), Some(Action::Suspend));
        // one attempt at a time
        assert_eq!(p.tick(40_000.0), None);
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn failures_back_off_exponentially_up_to_a_cap() {
        let mut p = Policy::new(Some(0.0));
        p.screen_off(0.0);
        let mut now = 0.0;
        let mut waits = Vec::new();
        for _ in 0..9 {
            assert_eq!(p.tick(now), Some(Action::Suspend));
            p.done(now, Outcome::Failed);
            let due = p.deadline().unwrap();
            waits.push(due - now);
            assert_eq!(p.tick(due - 1.0), None);
            now = due;
        }
        assert_eq!(waits, [1e3, 2e3, 4e3, 8e3, 16e3, 32e3, 60e3, 60e3, 60e3]);
    }

    #[test]
    fn resume_grants_a_grace_period_and_resets_backoff() {
        let mut p = Policy::new(Some(0.0));
        p.screen_off(0.0);
        p.tick(0.0);
        p.done(0.0, Outcome::Failed);
        p.tick(1000.0);
        p.done(1000.0, Outcome::Resumed);
        assert_eq!(p.deadline(), Some(1000.0 + GRACE_MS));
        assert_eq!(p.tick(1000.0 + GRACE_MS), Some(Action::Suspend));
        p.done(1000.0 + GRACE_MS, Outcome::Failed);
        assert_eq!(p.deadline(), Some(2000.0 + GRACE_MS));
    }

    #[test]
    fn screen_on_cancels_and_screen_off_restarts_the_timer() {
        let mut p = Policy::new(Some(30_000.0));
        p.screen_off(0.0);
        assert_eq!(p.tick(30_000.0), Some(Action::Suspend));
        p.screen_on();
        p.done(30_500.0, Outcome::Failed);
        assert_eq!(p.tick(1e9), None);
        p.screen_off(50_000.0);
        assert_eq!(p.deadline(), Some(80_000.0));
    }

    #[test]
    fn late_result_of_a_stale_attempt_blocks_a_second_attempt() {
        let mut p = Policy::new(Some(1000.0));
        p.screen_off(0.0);
        assert_eq!(p.tick(1000.0), Some(Action::Suspend));
        p.screen_on();
        p.screen_off(2000.0);
        // the first attempt is still blocked in the kernel
        assert_eq!(p.tick(3000.0), None);
        p.done(3500.0, Outcome::Failed);
        assert_eq!(p.tick(3500.0), Some(Action::Suspend));
    }

    #[test]
    fn sleeper_holds_the_wake_lock_while_on_and_suspends_when_off() {
        let f = Fake::new(Some("3"), None);
        let mut s = Sleeper::new(f.clone(), Some(100.0));
        assert_eq!(f.log(), ["wake_lock=springboard"]);
        s.screen_off(0.0);
        assert_eq!(s.deadline(), Some(100.0));
        assert_eq!(s.tick(50.0), None);
        s.tick(100.0);
        let mut outcome = None;
        for _ in 0..200 {
            outcome = s.tick(100.0);
            if outcome.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(outcome, Some(Outcome::Resumed));
        assert_eq!(s.deadline(), Some(100.0 + GRACE_MS));
        s.screen_on(200.0);
        assert_eq!(s.deadline(), None);
        let lock = "wake_lock=springboard";
        assert_eq!(f.log(), [lock, "wake_unlock=springboard", "read wakeup_count", "wakeup_count=3", "state=mem", "resumed", lock]);
    }
}
