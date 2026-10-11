//! Interaction boost: the stock governor (schedutil) ramps the clocks up over the first frames of an
//! animation, so opening an app or the switcher starts at 20+ ms per frame and settles near 6 ms.
//! While the UI is animating or was just touched the minimum clock of every cpufreq policy is raised
//! to the active profile's boost frequency (see `cpuprofile`), and lowered back to the hardware
//! minimum shortly after. The same controller applies the profile's governor and clock window.

use super::cpuprofile::{freq_at, Profile, Rule, STOCK_GOVERNOR};
use std::fs;
use std::path::PathBuf;

/// The Balanced profile's hold after the last touch or animated frame. Every animated frame
/// extends it, so it only has to bridge the gap between a finger lifting and the next touch
/// or the start of the response; longer keeps the clocks up for nothing.
pub const HOLD_MS: f64 = 350.0;

/// Pure hold logic: when to raise and when to lower. A hold of 0 never raises.
#[derive(Debug)]
pub struct Hold {
    on: bool,
    until: f64,
    hold_ms: f64,
}

impl Default for Hold {
    fn default() -> Self {
        Hold::new(HOLD_MS)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Raise,
    Lower,
}

impl Hold {
    pub fn new(hold_ms: f64) -> Self {
        Hold { on: false, until: 0.0, hold_ms }
    }

    /// `active` = animating or just touched.
    pub fn update(&mut self, active: bool, now: f64) -> Option<Change> {
        if active && self.hold_ms > 0.0 {
            self.until = now + self.hold_ms;
            return (!self.on).then(|| {
                self.on = true;
                Change::Raise
            });
        }
        (self.on && now >= self.until).then(|| {
            self.on = false;
            Change::Lower
        })
    }

    /// When `update` next needs to run to lower the clocks.
    pub fn deadline(&self) -> Option<f64> {
        self.on.then_some(self.until)
    }
}


/// A devfreq device (the GPU, the DDR controller) whose floor goes up with the cpu clocks:
/// a frame is as slow as its slowest unit, and these ramp as lazily as the cpus do.
struct Devfreq {
    dir: PathBuf,
    /// lowest and highest frequency the device offers, Hz
    low: u64,
    high: u64,
}

impl Devfreq {
    fn floor(&self, up: bool) {
        let _ = fs::write(self.dir.join("min_freq"), (if up { self.high } else { self.low }).to_string());
    }
}

/// The GPU and DDR devfreq nodes under `root` (those whose name says so), with their limits.
#[cfg(test)]
fn scan_devfreq(root: &str) -> Vec<Devfreq> {
    let mut out = Vec::new();
    for e in fs::read_dir(root).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !["gpu", "dmc", "ddr"].iter().any(|k| name.contains(k)) {
            continue;
        }
        let dir = e.path();
        let freqs: Vec<u64> = fs::read_to_string(dir.join("available_frequencies")).unwrap_or_default().split_whitespace().filter_map(|f| f.parse().ok()).collect();
        if let (Some(&low), Some(&high)) = (freqs.iter().min(), freqs.iter().max()) {
            out.push(Devfreq { dir, low, high });
        }
    }
    out
}

/// One cpufreq policy (a cluster) and its hardware limits in kHz.
struct Policy {
    dir: PathBuf,
    low: u32,
    high: u32,
    big: bool,
}

impl Policy {
    fn write(&self, file: &str, value: impl ToString) {
        let _ = fs::write(self.dir.join(file), value.to_string());
    }
}

pub struct Boost {
    hold: Hold,
    policies: Vec<Policy>,
    /// GPU and DDR clocks, raised with the cpus on the Balanced profile
    devfreq: Vec<Devfreq>,
    profile: Profile,
    /// the profile last written to sysfs; None until the first `set_profile`
    applied: Option<Profile>,
    /// desktop: no governor or clock writes (the firmware, intel_pstate or amd-pstate know better);
    /// the profile goes to power-profiles-daemon instead
    desktop: bool,
}

/// power-profiles-daemon's name for a shell profile.
pub fn ppd_name(p: Profile) -> &'static str {
    match p {
        Profile::Performance => "performance",
        Profile::Balanced => "balanced",
        Profile::Battery => "power-saver",
    }
}

impl Boost {
    /// Desktop: no governor or clock writes, only the power-profiles-daemon hand-off.
    pub fn new() -> Self {
        Boost { hold: Hold::default(), policies: Vec::new(), devfreq: Vec::new(), profile: Profile::default(), applied: None, desktop: true }
    }

    #[cfg(test)]
    fn at(root: &str) -> Self {
        let mut policies = Vec::new();
        for e in fs::read_dir(root).into_iter().flatten().flatten() {
            let dir = e.path();
            if !e.file_name().to_string_lossy().starts_with("policy") {
                continue;
            }
            let read = |f: &str| fs::read_to_string(dir.join(f)).ok().and_then(|s| s.trim().parse::<u32>().ok());
            if let (Some(low), Some(high)) = (read("cpuinfo_min_freq"), read("cpuinfo_max_freq")) {
                policies.push(Policy { dir, low, high, big: false });
            }
        }
        // the cluster that idles highest is the big one (the little cores reach the lowest clock)
        policies.sort_by_key(|p| (p.low, p.dir.clone()));
        let little = policies.first().map_or(0, |p| p.low);
        for p in &mut policies {
            p.big = p.low > little;
        }
        Boost { hold: Hold::default(), policies, devfreq: Vec::new(), profile: Profile::default(), applied: None, desktop: false }
    }

    /// The profile last asked for.
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Minimum time between presented frames under the active profile (0 = display paced).
    pub fn frame_interval_ms(&self) -> f64 {
        self.profile.rule().frame_ms
    }

    /// Switches profile and writes its governor, minimum and maximum right away; the same profile
    /// again is a no-op.
    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
        if self.applied == Some(profile) {
            return;
        }
        self.applied = Some(profile);
        if self.desktop {
            // best effort: no daemon, no profile switch
            crate::bare::sys::spawn_named("powerprofile", move || {
                let _ = std::process::Command::new("powerprofilesctl").args(["set", ppd_name(profile)]).status();
            });
            return;
        }
        let rule = profile.rule();
        self.hold = Hold::new(rule.boost_hold_ms);
        // the minimum goes down first and the maximum before the final minimum, so the kernel never
        // sees min above max
        for p in &self.policies {
            let c = rule.cluster(p.big);
            p.write("scaling_min_freq", p.low);
            p.write("scaling_governor", c.governor);
            p.write("scaling_max_freq", freq_at(p.low, p.high, c.max_pct));
        }
        self.apply(&rule, false);
        // best effort: the chip may be off or the interface absent
        #[cfg(unix)]
        let _ = super::nl80211::set_power_save(crate::net::wifi::IFACE, rule.wifi_power_save);
    }

    /// Call every loop iteration with whether the UI is animating or was just touched.
    pub fn update(&mut self, active: bool, now: f64) {
        let rule = self.profile.rule();
        match self.hold.update(active, now) {
            Some(Change::Raise) => self.apply(&rule, true),
            Some(Change::Lower) => self.apply(&rule, false),
            None => {}
        }
    }

    pub fn deadline(&self) -> Option<f64> {
        self.hold.deadline()
    }

    /// Puts every policy back to the stock governor and full range (clean exits; boot.sh also
    /// resets them when the shell dies).
    pub fn release(&mut self) {
        self.hold = Hold::new(0.0);
        self.applied = None;
        for p in &self.policies {
            p.write("scaling_min_freq", p.low);
            p.write("scaling_governor", STOCK_GOVERNOR);
            p.write("scaling_max_freq", p.high);
        }
        for d in &self.devfreq {
            d.floor(false);
        }
    }

    fn apply(&self, rule: &Rule, high: bool) {
        for p in &self.policies {
            let c = rule.cluster(p.big);
            let max = freq_at(p.low, p.high, c.max_pct);
            let min = if high { freq_at(p.low, p.high, c.boost_pct).min(max) } else { p.low };
            p.write("scaling_min_freq", min);
        }
        // Battery saves what Balanced spends; Performance never boosts
        let up = high && self.profile == Profile::Balanced;
        for d in &self.devfreq {
            d.floor(up);
        }
    }
}

impl Default for Boost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_profiles_map_to_power_profiles_daemon() {
        assert_eq!(ppd_name(Profile::Battery), "power-saver");
        assert_eq!(ppd_name(Profile::Balanced), "balanced");
        assert_eq!(ppd_name(Profile::Performance), "performance");
        let mut b = Boost::new();
        assert!(b.policies.is_empty());
        b.release(); // writes nothing anywhere
    }

    #[test]
    fn gpu_and_ddr_floors_follow_the_boost_on_balanced_only() {
        let dir = std::env::temp_dir().join(format!("sb-devfreq-{}", std::process::id()));
        for (name, freqs) in [("28000000.gpu", "320000000 512000000 850000000"), ("dmc", "533000000 1866000000"), ("sdio", "1 2")] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("available_frequencies"), freqs).unwrap();
            fs::write(dir.join(name).join("min_freq"), "0").unwrap();
        }
        let mut b = Boost::at(dir.join("none").to_str().unwrap());
        b.devfreq = scan_devfreq(dir.to_str().unwrap());
        assert_eq!(b.devfreq.len(), 2, "only the gpu and the ddr controller");
        let min = |n: &str| fs::read_to_string(dir.join(n).join("min_freq")).unwrap();
        b.set_profile(Profile::Balanced);
        assert_eq!(min("28000000.gpu"), "320000000");
        b.update(true, 0.0);
        assert_eq!(min("28000000.gpu"), "850000000");
        assert_eq!(min("dmc"), "1866000000");
        assert_eq!(min("sdio"), "0");
        b.update(false, 400.0);
        assert_eq!(min("dmc"), "533000000");
        b.set_profile(Profile::Battery);
        b.update(true, 0.0);
        assert_eq!(min("dmc"), "533000000", "Battery does not spend the bus");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn raises_once_when_activity_starts() {
        let mut h = Hold::default();
        assert_eq!(h.update(true, 0.0), Some(Change::Raise));
        assert_eq!(h.update(true, 16.0), None);
    }

    #[test]
    fn lowers_only_after_the_hold_has_passed() {
        let mut h = Hold::default();
        h.update(true, 100.0);
        assert_eq!(h.update(false, 100.0 + HOLD_MS - 1.0), None);
        assert_eq!(h.update(false, 100.0 + HOLD_MS), Some(Change::Lower));
        assert_eq!(h.update(false, 5000.0), None);
    }

    #[test]
    fn activity_extends_the_hold() {
        let mut h = Hold::default();
        h.update(true, 0.0);
        h.update(true, 400.0);
        assert_eq!(h.update(false, 600.0), None, "400 + HOLD is still ahead");
        assert_eq!(h.deadline(), Some(400.0 + HOLD_MS));
    }

    #[test]
    fn nothing_to_wake_for_while_idle() {
        let mut h = Hold::default();
        assert_eq!(h.deadline(), None);
        h.update(true, 0.0);
        h.update(false, 10_000.0);
        assert_eq!(h.deadline(), None);
    }

    #[test]
    fn a_zero_hold_never_raises() {
        let mut h = Hold::new(0.0);
        assert_eq!(h.update(true, 0.0), None);
        assert_eq!(h.deadline(), None);
    }

    fn fake_policy(dir: &std::path::Path, name: &str, low: u32, high: u32) -> PathBuf {
        let policy = dir.join(name);
        fs::create_dir_all(&policy).unwrap();
        fs::write(policy.join("cpuinfo_min_freq"), format!("{low}\n")).unwrap();
        fs::write(policy.join("cpuinfo_max_freq"), format!("{high}\n")).unwrap();
        fs::write(policy.join("scaling_min_freq"), format!("{low}\n")).unwrap();
        fs::write(policy.join("scaling_max_freq"), format!("{high}\n")).unwrap();
        fs::write(policy.join("scaling_governor"), "schedutil\n").unwrap();
        policy
    }

    fn read(policy: &std::path::Path, f: &str) -> String {
        fs::read_to_string(policy.join(f)).unwrap()
    }

    #[test]
    fn writes_the_cluster_limits() {
        let dir = std::env::temp_dir().join(format!("sb-boost-{}", std::process::id()));
        let little = fake_policy(&dir, "policy0", 600_000, 2_000_000);
        let big = fake_policy(&dir, "policy6", 1_200_000, 2_000_000);
        let mut b = Boost::at(dir.to_str().unwrap());
        b.set_profile(Profile::Balanced);
        b.update(true, 0.0);
        assert_eq!(read(&little, "scaling_min_freq"), "2000000");
        b.release();
        assert_eq!(read(&little, "scaling_min_freq"), "600000");
        assert_eq!(read(&big, "scaling_min_freq"), "1200000");
        assert_eq!(read(&big, "scaling_governor"), "schedutil");

        // Battery caps each cluster at its share of its own range and boosts to the middle
        b.set_profile(Profile::Battery);
        assert_eq!(read(&little, "scaling_max_freq"), "1580000", "70 % of 600000..2000000");
        assert_eq!(read(&big, "scaling_max_freq"), "1680000", "60 % of 1200000..2000000");
        assert_eq!(read(&big, "scaling_min_freq"), "1200000");
        b.update(true, 0.0);
        assert_eq!(read(&little, "scaling_min_freq"), "1300000");
        assert_eq!(b.deadline(), Some(300.0));
        b.update(false, 300.0);
        assert_eq!(read(&little, "scaling_min_freq"), "600000");

        // Performance pins the governor and never boosts
        b.set_profile(Profile::Performance);
        assert_eq!(read(&big, "scaling_governor"), "performance");
        assert_eq!(read(&big, "scaling_max_freq"), "2000000");
        b.update(true, 0.0);
        assert_eq!(b.deadline(), None);

        // a profile change while boosted drops the boost; the same profile again writes nothing
        b.set_profile(Profile::Balanced);
        fs::write(little.join("scaling_max_freq"), "1\n").unwrap();
        b.set_profile(Profile::Balanced);
        assert_eq!(read(&little, "scaling_max_freq"), "1\n");
        b.release();
        assert_eq!(read(&little, "scaling_max_freq"), "2000000");
        fs::remove_dir_all(&dir).unwrap();
    }
}
