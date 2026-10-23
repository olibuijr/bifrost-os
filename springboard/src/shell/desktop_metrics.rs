//! Desktop resource readings for the CPU and RAM menu extras, the way Golden Gate does it: cheap counter reads
//! (`bifrost_core::host`, the `host_statistics64` equivalents) taken on the main loop at a deadline. There is no
//! sampler thread. The shell reads at `interval_ms` (5 s idle wake budget, 1 s while a CPU/RAM menu is open: Activity Monitor
//! "Very often") with 10 % tolerance, lets the deadline slip to coalesce with another wake-up that is already
//! due, and does not read at all while the screen is off or locked (`Shell::metrics_active`).
use bifrost_core::host::{self, HostCpuLoadInfo, HostSampler};

#[derive(Clone, Default, PartialEq)]
pub struct Snapshot {
    pub cpu: f32,
    pub cores: usize,
    pub total: u64,
    pub available: u64,
    /// Golden Gate "Memory Used" in bytes: (active + wired + compressed) pages.
    pub used: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub load: String,
    pub uptime: u64,
    pub generation: u64,
    pub apps: Vec<AppUsage>,
}
/// One app group from /run/bifrost/app-resources.json (written by bare/app_manager.rs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppUsage {
    pub id: String,
    /// What the menus print: the app's manifest name (the id when no installed app has it).
    pub name: String,
    pub memory: u64,
    pub cpu_usec: u64,
    pub processes: u64,
}
fn number(text: &str, key: &str) -> u64 {
    let needle = format!("\"{key}\":");
    text.split(needle.as_str()).nth(1).and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
fn apps(text: &str) -> Vec<AppUsage> {
    text.split("{\"id\":\"")
        .skip(1)
        .filter_map(|chunk| {
            let id = chunk.split('"').next()?.to_owned();
            Some(AppUsage { name: id.clone(), id, memory: number(chunk, "memory_bytes"), cpu_usec: number(chunk, "usage_usec"), processes: number(chunk, "processes") })
        })
        .collect()
}
/// The app groups the CPU and RAM menus list: only those with a process in them (a quit app's group stays with its
/// page cache and CPU time, and listed Browser, Font Book and Disk Utility after they quit, W5-13), each named by
/// `name_of` (the manifest name) instead of its id.
pub fn running_apps(apps: Vec<AppUsage>, name_of: impl Fn(&str) -> Option<String>) -> Vec<AppUsage> {
    apps.into_iter()
        .filter(|a| a.processes > 0)
        .map(|mut a| {
            if let Some(name) = name_of(&a.id).filter(|n| !n.trim().is_empty()) {
                a.name = name;
            }
            a
        })
        .collect()
}
/// The installed app's manifest name for `id`.
fn manifest_name(id: &str) -> Option<String> {
    crate::shell::apphost::installed().iter().find(|a| a.manifest.id == id).map(|a| a.manifest.name.clone())
}
impl Snapshot {
    /// RAM % as Golden Gate "Memory Used": (active + wired + compressed) / physical memory.
    pub fn memory_percent(&self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            (100. * self.used.min(self.total) as f64 / self.total as f64) as f32
        }
    }
    /// Only the bar readouts ("CPU 3%", "RAM 41%"): what is visible while no CPU or RAM menu is open.
    pub fn bar_key(&self) -> String {
        format!("{:.0}|{:.0}", self.cpu, self.memory_percent())
    }
    /// What the open menus print (precision and units as in desktop_bar.rs); equal keys mean no visible change.
    pub fn shown(&self) -> String {
        let gb = |n: u64| format!("{:.2}", n as f64 / 1073741824.);
        format!(
            "{:.1}|{:.0}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.cpu,
            self.memory_percent(),
            self.cores,
            gb(self.total),
            gb(self.used),
            gb(self.available),
            gb(self.cached),
            gb(self.swap_total),
            gb(self.swap_free),
            self.load,
            self.uptime / 60
        )
    }
}

/// Seconds between reads with only the bar readouts showing.
pub const INTERVAL_MS: f64 = 5000.;
/// Seconds between reads while a CPU or RAM menu is open.
pub const MENU_INTERVAL_MS: f64 = 1000.;
/// Share of the interval a read may slip by (timer tolerance / dispatch leeway).
pub const TOLERANCE: f64 = 0.1;
/// The grid deadlines snap to when it is within tolerance: whole seconds, where the clock and other 1 Hz work wake.
const GRID_MS: f64 = 1000.;

pub fn interval_ms(menu_open: bool) -> f64 {
    if menu_open {
        MENU_INTERVAL_MS
    } else {
        INTERVAL_MS
    }
}
pub fn tolerance_ms(menu_open: bool) -> f64 {
    interval_ms(menu_open) * TOLERANCE
}
/// When the next read is due after one at `last` (monotonic ms): `last + interval`, moved later to the next second
/// boundary when that is within the tolerance.
pub fn deadline(last: f64, menu_open: bool) -> f64 {
    let d = last + interval_ms(menu_open);
    let snapped = (d / GRID_MS).ceil() * GRID_MS;
    if snapped - d <= tolerance_ms(menu_open) {
        snapped
    } else {
        d
    }
}
/// The wake to schedule for `deadline`: an `other` wake-up already planned inside `[deadline, deadline + tolerance]`
/// serves the read too (no extra wake); otherwise the deadline itself.
pub fn coalesce(deadline: f64, tolerance: f64, other: Option<f64>) -> f64 {
    match other {
        Some(o) if o >= deadline && o <= deadline + tolerance => o,
        _ => deadline,
    }
}
/// Whether a wake at `now` should read.
pub fn is_due(now: f64, last: f64, menu_open: bool) -> bool {
    now >= deadline(last, menu_open)
}

/// The reader: one `HostSampler` (reused buffers) and the previous CPU ticks. Not a thread.
#[derive(Default)]
pub struct Sampler {
    host: HostSampler,
    previous: Option<HostCpuLoadInfo>,
    generation: u64,
}

impl Sampler {
    /// Forget the CPU baseline (after a pause the ticks span the whole pause, which says nothing about now).
    pub fn pause(&mut self) {
        self.previous = None;
    }
    /// One read. `menu_open` also reads the menu-only values (load, uptime, per-app groups).
    pub fn sample(&mut self, out: &mut Snapshot, menu_open: bool) {
        if let Some(now) = self.host.host_cpu_load_info() {
            if let Some(old) = self.previous {
                out.cpu = HostCpuLoadInfo::usage_percent(&old, &now);
            }
            self.previous = Some(now);
            out.cores = self.host.processor_info().len();
        }
        if let Some(m) = self.host.memory() {
            out.total = m.physical;
            out.available = m.available;
            out.used = m.used_bytes();
            out.cached = m.cached;
            out.swap_total = m.swap_total;
            out.swap_free = m.swap_free;
        }
        self.generation += 1;
        out.generation = self.generation;
        if menu_open {
            let l = host::load_average();
            out.load = format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2]);
            out.uptime = self.host.system_uptime() as u64;
            #[cfg(feature = "desktop-wayland")]
            crate::bare::app_manager::want_resources();
            let groups = std::fs::read_to_string("/run/bifrost/app-resources.json").map(|t| apps(&t)).unwrap_or_default();
            out.apps = running_apps(groups, manifest_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_is_active_plus_wired_plus_compressed_not_total_minus_available() {
        let s = Snapshot { total: 100, available: 40, used: 30, ..Default::default() };
        assert_eq!(s.memory_percent(), 30.);
        assert_eq!(Snapshot::default().memory_percent(), 0.);
    }

    #[test]
    fn cadence_is_five_seconds_and_one_while_a_menu_is_open() {
        assert_eq!(interval_ms(false), 5000.);
        assert_eq!(interval_ms(true), 1000.);
        assert_eq!(tolerance_ms(false), 500.);
        assert!(!is_due(1500., 0., false));
        assert!(is_due(5000., 0., false));
        assert!(!is_due(500., 0., true));
        assert!(is_due(1000., 0., true));
    }

    #[test]
    fn deadline_snaps_to_the_next_second_only_inside_the_tolerance() {
        assert_eq!(deadline(100., false), 5100., "900 ms off the grid: no snap");
        assert_eq!(deadline(900., false), 6000., "5900 is 100 ms short of a boundary, within 500");
        assert_eq!(deadline(0., false), 5000.);
    }

    #[test]
    fn another_wake_inside_the_window_is_reused() {
        assert_eq!(coalesce(2000., 200., Some(2150.)), 2150.);
        assert_eq!(coalesce(2000., 200., Some(2300.)), 2000.);
        assert_eq!(coalesce(2000., 200., Some(1500.)), 2000., "an earlier wake does not serve a later deadline");
        assert_eq!(coalesce(2000., 200., None), 2000.);
    }

    #[test]
    fn sampler_reads_this_host_and_pauses() {
        let mut s = Sampler::default();
        let mut snap = Snapshot::default();
        s.sample(&mut snap, false);
        assert!(snap.total > 0 && snap.cores >= 1);
        assert_eq!(snap.generation, 1);
        s.sample(&mut snap, true);
        assert!(snap.cpu >= 0. && snap.cpu <= 100. && !snap.load.is_empty());
        s.pause();
        assert!(s.previous.is_none());
    }

    #[test]
    fn counts_app_groups() {
        assert_eq!(apps("[{\"id\":\"a\",\"memory_bytes\":5,\"usage_usec\":7,\"processes\":2}]"), vec![AppUsage { id: "a".into(), name: "a".into(), memory: 5, cpu_usec: 7, processes: 2 }]);
    }

    #[test]
    fn the_menus_list_running_apps_by_their_manifest_name() {
        let groups = apps(concat!(
            "[{\"id\":\"is.olibuijr.browser.desktop\",\"memory_bytes\":900,\"usage_usec\":1,\"processes\":0},",
            "{\"id\":\"is.olibuijr.notes.desktop\",\"memory_bytes\":50,\"usage_usec\":9,\"processes\":3},",
            "{\"id\":\"is.a.unknown\",\"memory_bytes\":10,\"usage_usec\":2,\"processes\":1}]"
        ));
        let name_of = |id: &str| (id == "is.olibuijr.notes.desktop").then(|| "Notes".to_string());
        let shown = running_apps(groups, name_of);
        let names: Vec<&str> = shown.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Notes", "is.a.unknown"], "the quit Browser is gone; an app without a manifest keeps its id");
        assert_eq!(shown[0].id, "is.olibuijr.notes.desktop", "the id stays for lookups");
    }
}
