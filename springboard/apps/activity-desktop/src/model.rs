//! What the window shows: the tabs and their columns, the process filter, sort and search, the graph history and the
//! number formats. Plain data with no drawing; `window.rs` holds the state and `draw.rs` paints it.

use crate::sample::{ProcSample, Snapshot, TICKS_PER_SEC};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// Samples kept for each graph (one per second, so one minute).
pub const HISTORY: usize = 60;

/// The six tabs above the graph. Each picks the graph and the table's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
    Gpu,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Cpu, Tab::Memory, Tab::Energy, Tab::Disk, Tab::Network, Tab::Gpu];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
            Tab::Memory => "Memory",
            Tab::Energy => "Energy",
            Tab::Disk => "Disk",
            Tab::Network => "Network",
            Tab::Gpu => "GPU",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Tab {
        Tab::ALL.get(i).copied().unwrap_or(Tab::Cpu)
    }

    /// The sort a tab starts with: the figure it is about, largest first.
    pub fn default_sort(self) -> Sort {
        match self {
            Tab::Cpu => Sort { key: SortKey::Cpu, descending: true },
            Tab::Memory => Sort { key: SortKey::Memory, descending: true },
            Tab::Energy => Sort { key: SortKey::Energy, descending: true },
            Tab::Disk => Sort { key: SortKey::DiskRead, descending: true },
            Tab::Network => Sort { key: SortKey::Name, descending: false },
            Tab::Gpu => Sort { key: SortKey::Gpu, descending: true },
        }
    }

    /// Every column the tab can show, in order. A width of 0 takes the space the fixed columns leave (the name column).
    pub fn all_columns(self) -> Vec<Column> {
        let name = || Column::new(Some(SortKey::Name), "Process", 0.0).required();
        let user = || Column::new(Some(SortKey::User), "User", 96.0);
        let threads = || Column::new(Some(SortKey::Threads), "Threads", 64.0);
        let pid = || Column::new(Some(SortKey::Pid), "PID", 56.0);
        match self {
            Tab::Cpu => vec![
                name(),
                user(),
                Column::new(Some(SortKey::Cpu), "% CPU", 72.0).required(),
                Column::new(Some(SortKey::CpuTime), "CPU Time", 88.0).off(),
                threads(),
                Column::new(Some(SortKey::Ppid), "Parent PID", 88.0).off(),
                pid(),
            ],
            Tab::Memory => vec![
                name(),
                user(),
                Column::new(Some(SortKey::Memory), "Memory", 88.0).required(),
                Column::new(Some(SortKey::Virtual), "Virtual Size", 104.0).off(),
                threads(),
                pid(),
            ],
            Tab::Energy => vec![
                name(),
                user(),
                Column::new(Some(SortKey::Energy), "Energy Impact", 112.0).required(),
                Column::new(Some(SortKey::EnergyAvg), "12 hr Average", 104.0).off(),
                Column::new(Some(SortKey::PreventingSleep), "Preventing Sleep", 124.0),
                threads(),
                pid(),
            ],
            Tab::Disk => vec![
                name(),
                user(),
                Column::new(Some(SortKey::DiskRead), "Read/s", 96.0).required(),
                Column::new(Some(SortKey::DiskWrite), "Written/s", 104.0),
                Column::new(Some(SortKey::BytesRead), "Bytes Read", 96.0).off(),
                Column::new(Some(SortKey::BytesWritten), "Bytes Written", 112.0).off(),
                pid(),
            ],
            Tab::Network => vec![
                Column::new(None, "Interface", 0.0).required(),
                Column::new(None, "Received/s", 104.0),
                Column::new(None, "Sent/s", 104.0),
                Column::new(None, "Total Received", 120.0),
                Column::new(None, "Total Sent", 120.0),
                Column::new(None, "Packets In", 96.0).off(),
                Column::new(None, "Packets Out", 104.0).off(),
            ],
            Tab::Gpu => vec![
                name(),
                user(),
                Column::new(Some(SortKey::Gpu), "% GPU", 72.0).required(),
                Column::new(Some(SortKey::GpuTime), "GPU Time", 96.0),
                pid(),
            ],
        }
    }

    /// The columns the table shows: all of them less the ones View > Columns turned off, plus the ones it turned on.
    pub fn columns(self, set: &ColumnSet) -> Vec<Column> {
        self.all_columns().into_iter().filter(|c| set.shown(self, c)).collect()
    }
}

/// Which optional columns differ from their defaults (View > Columns), remembered across launches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnSet {
    toggled: BTreeSet<String>,
}

impl ColumnSet {
    fn key(tab: Tab, col: &Column) -> String {
        format!("{}/{}", tab.label(), col.title)
    }

    /// Whether the table shows a column: the required ones always, the others by their default turned over once for
    /// each time the user toggled them.
    pub fn shown(&self, tab: Tab, col: &Column) -> bool {
        col.required || col.on_by_default != self.toggled.contains(&Self::key(tab, col))
    }

    /// Shows a hidden column or hides a shown one. Required columns stay.
    pub fn toggle(&mut self, tab: Tab, col: &Column) {
        if col.required {
            return;
        }
        let key = Self::key(tab, col);
        if !self.toggled.remove(&key) {
            self.toggled.insert(key);
        }
    }

    /// The set as one preference string.
    pub fn to_pref(&self) -> String {
        self.toggled.iter().cloned().collect::<Vec<_>>().join(",")
    }

    /// The set from a preference string; unknown names are kept, so a newer version's choices survive a downgrade.
    pub fn from_pref(text: &str) -> ColumnSet {
        ColumnSet { toggled: text.split(',').filter(|k| !k.is_empty()).map(str::to_string).collect() }
    }
}

/// A table column: its sort key (None when the column does not sort), its title and its width in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub key: Option<SortKey>,
    pub title: &'static str,
    pub width: f32,
    /// Always shown (the name, the figure the tab is about).
    pub required: bool,
    /// Shown until View > Columns turns it off.
    pub on_by_default: bool,
}

impl Column {
    pub fn new(key: Option<SortKey>, title: &'static str, width: f32) -> Self {
        Column { key, title, width, required: false, on_by_default: true }
    }

    /// Marks the column as one that cannot be hidden.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Marks the column as hidden until View > Columns turns it on.
    pub fn off(mut self) -> Self {
        self.on_by_default = false;
        self
    }
}

/// What the process table can sort by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    User,
    Cpu,
    Memory,
    Energy,
    EnergyAvg,
    PreventingSleep,
    DiskRead,
    DiskWrite,
    BytesRead,
    BytesWritten,
    Threads,
    Pid,
    Ppid,
    CpuTime,
    Virtual,
    Gpu,
    GpuTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Sort {
    /// Clicking a column: the same column flips the direction; a new one starts largest first for figures and A to Z
    /// for names and users.
    pub fn clicked(self, key: SortKey) -> Sort {
        if self.key == key {
            return Sort { key, descending: !self.descending };
        }
        let text = matches!(key, SortKey::Name | SortKey::User);
        Sort { key, descending: !text }
    }
}

/// The sidebar filter: every process, the signed-in user's own, the processes of other users, the system's, the
/// ones using / not using the CPU right now, the selected process with its children, or the ones using the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Mine,
    Others,
    System,
    Active,
    Inactive,
    /// The process selected when the filter was chosen, and everything it started. The scope is the caller's
    /// (`visible_within`); the filter itself keeps every row it is given.
    Selected,
    Gpu,
}

/// Accounts below this uid belong to the system, not to a person.
pub const FIRST_PERSON_UID: u32 = 1000;

impl Filter {
    pub const ALL: [Filter; 8] =
        [Filter::All, Filter::Mine, Filter::Others, Filter::System, Filter::Active, Filter::Inactive, Filter::Selected, Filter::Gpu];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All Processes",
            Filter::Mine => "My Processes",
            Filter::Others => "Other Users",
            Filter::System => "System Processes",
            Filter::Active => "Active Processes",
            Filter::Inactive => "Inactive Processes",
            Filter::Selected => "Selected Processes",
            Filter::Gpu => "GPU Processes",
        }
    }

    pub fn keep(self, p: &ProcSample, own_uid: u32) -> bool {
        match self {
            Filter::All => true,
            Filter::Mine => p.uid == own_uid,
            Filter::Others => p.uid != own_uid,
            Filter::System => p.uid < FIRST_PERSON_UID,
            Filter::Active => p.cpu > 0.0,
            Filter::Inactive => p.cpu <= 0.0,
            Filter::Selected => true,
            Filter::Gpu => p.gpu > 0.0,
        }
    }
}

/// How often the machine is read (View > Update Frequency).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateFrequency {
    VeryOften,
    Often,
    Normally,
}

impl UpdateFrequency {
    pub const ALL: [UpdateFrequency; 3] = [UpdateFrequency::VeryOften, UpdateFrequency::Often, UpdateFrequency::Normally];

    pub fn seconds(self) -> i64 {
        match self {
            UpdateFrequency::VeryOften => 1,
            UpdateFrequency::Often => 2,
            UpdateFrequency::Normally => 5,
        }
    }

    pub fn millis(self) -> f64 {
        self.seconds() as f64 * 1000.0
    }

    pub fn label(self) -> &'static str {
        match self {
            UpdateFrequency::VeryOften => "Very Often (1 sec)",
            UpdateFrequency::Often => "Often (2 sec)",
            UpdateFrequency::Normally => "Normally (5 sec)",
        }
    }

    /// The stored seconds back to a choice; anything unknown (or unset, 0) is the 1 s default.
    pub fn from_seconds(s: i64) -> UpdateFrequency {
        UpdateFrequency::ALL.into_iter().find(|f| f.seconds() == s).unwrap_or(UpdateFrequency::VeryOften)
    }
}

/// A signal that Process > Send Signal offers, with its Linux number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signal {
    pub name: &'static str,
    pub number: i32,
}

impl Signal {
    /// What the signal does, in a few words, for the signal sheet.
    pub fn describes(self) -> &'static str {
        match self.name {
            "HUP" => "Hang up: many programs reload their settings",
            "INT" => "Interrupt, as Control-C does",
            "KILL" => "Stop at once; the process cannot clean up",
            "TERM" => "Ask the process to quit",
            "USR1" | "USR2" => "Left to the program to define",
            "STOP" => "Pause the process",
            "CONT" => "Resume a paused process",
            _ => "",
        }
    }
}

pub const SIGNALS: [Signal; 8] = [
    Signal { name: "HUP", number: 1 },
    Signal { name: "INT", number: 2 },
    Signal { name: "KILL", number: 9 },
    Signal { name: "TERM", number: 15 },
    Signal { name: "USR1", number: 10 },
    Signal { name: "USR2", number: 12 },
    Signal { name: "STOP", number: 19 },
    Signal { name: "CONT", number: 18 },
];

/// CPU time as minutes, seconds and hundredths: "2:05.30".
pub fn cpu_time(ticks: u64) -> String {
    let secs = ticks / TICKS_PER_SEC as u64;
    format!("{}:{:02}.{:02}", secs / 60, secs % 60, ticks % TICKS_PER_SEC as u64)
}

/// GPU engine time in seconds: "1.25 s".
pub fn gpu_time(ns: u64) -> String {
    format!("{:.2} s", ns as f64 / 1e9)
}

/// The text of one table cell for a process row. With a baseline (Show Deltas), the cumulative figures (CPU time,
/// bytes read and written, GPU time) count from it instead of from the start of the process.
pub fn cell_text(p: &ProcSample, key: Option<SortKey>, base: Option<&ProcSample>) -> String {
    let since = |now: u64, then: Option<u64>| now.saturating_sub(then.unwrap_or(0));
    match key {
        Some(SortKey::Name) => p.name.clone(),
        Some(SortKey::User) => p.user.clone(),
        Some(SortKey::Cpu) => format!("{:.1}", p.cpu),
        Some(SortKey::CpuTime) => cpu_time(since(p.cpu_ticks, base.map(|b| b.cpu_ticks))),
        Some(SortKey::Memory) => bytes(p.rss),
        Some(SortKey::Virtual) => bytes(p.vsize),
        Some(SortKey::Energy) => format!("{:.1}", energy(p)),
        Some(SortKey::EnergyAvg) => format!("{:.1}", p.energy_avg),
        Some(SortKey::PreventingSleep) => (if p.prevents_sleep { "Yes" } else { "No" }).to_string(),
        Some(SortKey::DiskRead) => rate(p.read_bps),
        Some(SortKey::DiskWrite) => rate(p.write_bps),
        Some(SortKey::BytesRead) => bytes(since(p.read_total, base.map(|b| b.read_total))),
        Some(SortKey::BytesWritten) => bytes(since(p.write_total, base.map(|b| b.write_total))),
        Some(SortKey::Threads) => p.threads.to_string(),
        Some(SortKey::Pid) => p.pid.to_string(),
        Some(SortKey::Ppid) => p.ppid.to_string(),
        Some(SortKey::Gpu) => format!("{:.1}", p.gpu),
        Some(SortKey::GpuTime) => gpu_time(since(p.gpu_ns, base.map(|b| b.gpu_ns))),
        None => String::new(),
    }
}

/// One table row as tab-separated text (Edit > Copy): the columns the tab shows, in order. The CPU figure carries its
/// percent sign, as it does in the footer and the graph.
pub fn row_text(p: &ProcSample, tab: Tab, set: &ColumnSet, base: Option<&ProcSample>) -> String {
    if tab == Tab::Network {
        return [p.name.clone(), p.user.clone(), p.pid.to_string()].join("\t");
    }
    let fields: Vec<String> = tab
        .columns(set)
        .iter()
        .map(|c| if c.key == Some(SortKey::Cpu) { percent(p.cpu) } else { cell_text(p, c.key, base) })
        .collect();
    fields.join("\t")
}

/// The pids of a process and everything it started, children and their children.
pub fn descendants(procs: &[ProcSample], root: u32) -> HashSet<u32> {
    let mut set = HashSet::from([root]);
    loop {
        let before = set.len();
        for p in procs {
            if set.contains(&p.ppid) {
                set.insert(p.pid);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// A running mean of each process name's energy score, weighted towards the last 12 hours: a plain mean while the
/// monitor is young, then a moving average with a 12 hour time constant. Starts empty each launch.
#[derive(Clone, Debug, Default)]
pub struct EnergyAverages {
    by_name: HashMap<String, (f64, u64)>,
}

/// The time constant of the moving average, in seconds.
const TWELVE_HOURS: f64 = 12.0 * 3600.0;
/// Most names remembered; a longer list starts over rather than growing without end.
const MAX_NAMES: usize = 4096;

impl EnergyAverages {
    /// Folds one reading in: the energy score of each name (the sum over its processes) over `dt` seconds.
    pub fn record(&mut self, procs: &[ProcSample], dt: f64) {
        let mut now: HashMap<&str, f64> = HashMap::new();
        for p in procs {
            *now.entry(p.name.as_str()).or_insert(0.0) += energy(p);
        }
        if self.by_name.len() > MAX_NAMES {
            self.by_name.clear();
        }
        for (name, score) in now {
            let entry = self.by_name.entry(name.to_string()).or_insert((0.0, 0));
            entry.1 += 1;
            let alpha = (1.0 / entry.1 as f64).max(dt / TWELVE_HOURS).min(1.0);
            entry.0 += (score - entry.0) * alpha;
        }
    }

    pub fn mean(&self, name: &str) -> f64 {
        self.by_name.get(name).map_or(0.0, |e| e.0)
    }
}

/// An energy score for one process: its CPU share plus its disk traffic. Activity Monitor's Energy column is
/// not public; this is a heuristic that ranks the same busy processes first (one MiB/s of disk weighs as 2 %).
pub fn energy(p: &ProcSample) -> f64 {
    p.cpu as f64 + (p.read_bps + p.write_bps) / (1024.0 * 1024.0) * 2.0
}

fn compare(a: &ProcSample, b: &ProcSample, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortKey::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
        SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
        SortKey::Memory => a.rss.cmp(&b.rss),
        SortKey::Energy => energy(a).total_cmp(&energy(b)),
        SortKey::EnergyAvg => a.energy_avg.total_cmp(&b.energy_avg),
        SortKey::PreventingSleep => a.prevents_sleep.cmp(&b.prevents_sleep),
        SortKey::DiskRead => a.read_bps.total_cmp(&b.read_bps),
        SortKey::DiskWrite => a.write_bps.total_cmp(&b.write_bps),
        SortKey::BytesRead => a.read_total.cmp(&b.read_total),
        SortKey::BytesWritten => a.write_total.cmp(&b.write_total),
        SortKey::Threads => a.threads.cmp(&b.threads),
        SortKey::Pid => a.pid.cmp(&b.pid),
        SortKey::Ppid => a.ppid.cmp(&b.ppid),
        SortKey::CpuTime => a.cpu_ticks.cmp(&b.cpu_ticks),
        SortKey::Virtual => a.vsize.cmp(&b.vsize),
        SortKey::Gpu => a.gpu.total_cmp(&b.gpu),
        SortKey::GpuTime => a.gpu_ns.cmp(&b.gpu_ns),
    }
}

/// Whether a process matches the search text (already lower case): its name, user or PID contains it.
pub fn matches(p: &ProcSample, query: &str) -> bool {
    query.is_empty() || p.name.to_lowercase().contains(query) || p.user.to_lowercase().contains(query) || p.pid.to_string().contains(query)
}

/// The rows of the process table: filtered, searched and sorted. Ties keep PID order so rows do not swap places.
pub fn visible(procs: &[ProcSample], filter: Filter, own_uid: u32, query: &str, sort: Sort) -> Vec<ProcSample> {
    visible_within(procs, filter, own_uid, query, sort, None)
}

/// Like `visible`, limited to the pids in `only` when it is given (the Selected Processes scope).
pub fn visible_within(procs: &[ProcSample], filter: Filter, own_uid: u32, query: &str, sort: Sort, only: Option<&HashSet<u32>>) -> Vec<ProcSample> {
    let mut rows: Vec<ProcSample> =
        procs.iter().filter(|p| only.map_or(true, |set| set.contains(&p.pid)) && filter.keep(p, own_uid) && matches(p, query)).cloned().collect();
    rows.sort_by(|a, b| {
        let o = compare(a, b, sort.key);
        let o = if sort.descending { o.reverse() } else { o };
        o.then(a.pid.cmp(&b.pid))
    });
    rows
}

/// The last `HISTORY` values of one graph, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Series {
    values: VecDeque<f32>,
}

impl Series {
    pub fn push(&mut self, v: f32) {
        if self.values.len() == HISTORY {
            self.values.pop_front();
        }
        self.values.push_back(v.max(0.0));
    }

    pub fn last(&self) -> f32 {
        self.values.back().copied().unwrap_or(0.0)
    }

    pub fn peak(&self) -> f32 {
        self.values.iter().copied().fold(0.0, f32::max)
    }

    pub fn values(&self) -> impl Iterator<Item = f32> + '_ {
        self.values.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// The graphs' history: CPU and memory in percent, energy as the sum over processes, disk and network in bytes per
/// second.
#[derive(Clone, Debug, Default)]
pub struct History {
    pub cpu: Series,
    pub memory: Series,
    pub energy: Series,
    pub disk_read: Series,
    pub disk_write: Series,
    pub net_rx: Series,
    pub net_tx: Series,
    /// One series per core, in core order.
    pub cores: Vec<Series>,
    /// GPU busy share, recorded only while the sampler scans the GPU.
    pub gpu: Series,
}

impl History {
    pub fn record(&mut self, snap: &Snapshot) {
        self.cpu.push(snap.cpu.busy);
        self.memory.push(snap.memory.used_percent());
        self.energy.push(snap.procs.iter().map(energy).sum::<f64>() as f32);
        self.disk_read.push(snap.disk_read_bps as f32);
        self.disk_write.push(snap.disk_write_bps as f32);
        self.net_rx.push(snap.ifaces.iter().map(|i| i.rx_bps).sum::<f64>() as f32);
        self.net_tx.push(snap.ifaces.iter().map(|i| i.tx_bps).sum::<f64>() as f32);
        self.cores.resize_with(snap.cores.len(), Series::default);
        for (series, busy) in self.cores.iter_mut().zip(&snap.cores) {
            series.push(*busy);
        }
        if let Some(busy) = snap.gpu_busy {
            self.gpu.push(busy);
        }
    }
}

/// A byte count with a binary unit: "812 B", "4.2 MB", "120 GB".
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// A rate in bytes per second: "4.2 MB/s".
pub fn rate(bps: f64) -> String {
    format!("{}/s", bytes(bps.max(0.0) as u64))
}

/// A percentage with one decimal: "37.5%".
pub fn percent(v: f32) -> String {
    format!("{v:.1}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, name: &str, uid: u32, cpu: f32, rss: u64) -> ProcSample {
        ProcSample { pid, name: name.into(), user: if uid == 0 { "root".into() } else { "olibuijr".into() }, uid, cpu, rss, threads: 1, ..ProcSample::default() }
    }

    #[test]
    fn cpu_sort_is_largest_first_and_ties_keep_pid_order() {
        let procs = vec![proc(30, "a", 1000, 5.0, 1), proc(10, "b", 1000, 9.0, 1), proc(20, "c", 1000, 5.0, 1)];
        let rows = visible(&procs, Filter::All, 1000, "", Tab::Cpu.default_sort());
        let pids: Vec<u32> = rows.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![10, 20, 30]);
    }

    #[test]
    fn filters_and_search_apply_before_sorting() {
        let procs = vec![proc(1, "systemd", 0, 1.0, 1), proc(2, "firefox", 1000, 2.0, 1), proc(3, "Fire", 1000, 0.5, 1)];
        let mine = visible(&procs, Filter::Mine, 1000, "", Sort { key: SortKey::Name, descending: false });
        assert_eq!(mine.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![3, 2]);
        let others = visible(&procs, Filter::Others, 1000, "", Sort { key: SortKey::Pid, descending: false });
        assert_eq!(others.len(), 1);
        let found = visible(&procs, Filter::All, 1000, "fire", Sort { key: SortKey::Pid, descending: false });
        assert_eq!(found.len(), 2);
        assert!(matches(&procs[0], "root"));
    }

    #[test]
    fn system_active_and_inactive_filters() {
        let procs = vec![proc(1, "systemd", 0, 0.0, 1), proc(2, "firefox", 1000, 2.0, 1)];
        let sort = Sort { key: SortKey::Pid, descending: false };
        assert_eq!(visible(&procs, Filter::System, 1000, "", sort).len(), 1);
        assert_eq!(visible(&procs, Filter::Active, 1000, "", sort)[0].pid, 2);
        assert_eq!(visible(&procs, Filter::Inactive, 1000, "", sort)[0].pid, 1);
    }

    #[test]
    fn update_frequency_round_trips_and_defaults_to_one_second() {
        for f in UpdateFrequency::ALL {
            assert_eq!(UpdateFrequency::from_seconds(f.seconds()), f);
        }
        assert_eq!(UpdateFrequency::from_seconds(0), UpdateFrequency::VeryOften);
        assert_eq!(UpdateFrequency::Normally.millis(), 5000.0);
    }

    #[test]
    fn rows_copy_as_tab_separated_text() {
        let p = proc(7, "sh", 1000, 12.5, 2048);
        let set = ColumnSet::default();
        assert_eq!(row_text(&p, Tab::Cpu, &set, None), "sh\tolibuijr\t12.5%\t1\t7");
        assert_eq!(row_text(&p, Tab::Memory, &set, None), "sh\tolibuijr\t2.0 KB\t1\t7");
    }

    #[test]
    fn clicking_a_column_flips_or_starts_a_direction() {
        let s = Tab::Cpu.default_sort();
        assert_eq!(s.clicked(SortKey::Cpu).descending, false);
        assert_eq!(s.clicked(SortKey::Name).descending, false);
        assert_eq!(s.clicked(SortKey::Memory).descending, true);
    }

    #[test]
    fn series_keeps_the_last_sixty_values() {
        let mut s = Series::default();
        for i in 0..(HISTORY + 5) {
            s.push(i as f32);
        }
        assert_eq!(s.len(), HISTORY);
        assert_eq!(s.values().next(), Some(5.0));
        assert_eq!(s.last(), (HISTORY + 4) as f32);
        assert_eq!(s.peak(), (HISTORY + 4) as f32);
    }

    #[test]
    fn formats_bytes_and_rates() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(rate(2048.0), "2.0 KB/s");
        assert_eq!(percent(37.44), "37.4%");
    }

    #[test]
    fn every_tab_has_one_fill_column_and_a_sort_key_for_its_default() {
        for tab in Tab::ALL {
            let cols = tab.columns(&ColumnSet::default());
            assert_eq!(cols.iter().filter(|c| c.width == 0.0).count(), 1);
            if tab != Tab::Network {
                assert!(cols.iter().any(|c| c.key == Some(tab.default_sort().key)));
            }
        }
    }

    #[test]
    fn every_signal_has_a_description_and_a_distinct_number() {
        let mut numbers: Vec<i32> = SIGNALS.iter().map(|s| s.number).collect();
        numbers.sort_unstable();
        numbers.dedup();
        assert_eq!(numbers.len(), SIGNALS.len());
        assert!(SIGNALS.iter().all(|s| !s.describes().is_empty()));
        assert_eq!(SIGNALS[3].name, "TERM");
        assert_eq!(SIGNALS[3].describes(), "Ask the process to quit");
    }

    #[test]
    fn optional_columns_toggle_against_their_default_and_round_trip() {
        let mut set = ColumnSet::default();
        let titles = |set: &ColumnSet| Tab::Cpu.columns(set).iter().map(|c| c.title).collect::<Vec<_>>();
        assert_eq!(titles(&set), vec!["Process", "User", "% CPU", "Threads", "PID"]);
        let all = Tab::Cpu.all_columns();
        let cpu_time = all.iter().find(|c| c.title == "CPU Time").unwrap();
        let user = all.iter().find(|c| c.title == "User").unwrap();
        set.toggle(Tab::Cpu, cpu_time);
        set.toggle(Tab::Cpu, user);
        assert_eq!(titles(&set), vec!["Process", "% CPU", "CPU Time", "Threads", "PID"]);
        // the same title on another tab is its own choice
        assert!(Tab::Disk.columns(&set).iter().any(|c| c.title == "User"));
        let restored = ColumnSet::from_pref(&set.to_pref());
        assert_eq!(restored, set);
        set.toggle(Tab::Cpu, cpu_time);
        assert_eq!(set.to_pref(), "CPU/User");
        assert_eq!(ColumnSet::from_pref(""), ColumnSet::default());
    }

    #[test]
    fn required_columns_cannot_be_hidden() {
        let mut set = ColumnSet::default();
        for tab in Tab::ALL {
            for col in tab.all_columns().iter().filter(|c| c.required) {
                set.toggle(tab, col);
            }
        }
        assert_eq!(set, ColumnSet::default());
        for tab in Tab::ALL {
            assert!(tab.columns(&set).iter().any(|c| c.required));
        }
    }

    #[test]
    fn cpu_and_gpu_time_formats() {
        assert_eq!(cpu_time(0), "0:00.00");
        assert_eq!(cpu_time(12_530), "2:05.30");
        assert_eq!(gpu_time(1_250_000_000), "1.25 s");
    }

    #[test]
    fn cumulative_cells_count_from_the_baseline() {
        let mut p = proc(5, "x", 1000, 0.0, 0);
        p.cpu_ticks = 1000;
        p.read_total = 5 * 1024 * 1024;
        p.write_total = 2048;
        p.gpu_ns = 3_000_000_000;
        let mut base = p.clone();
        base.cpu_ticks = 400;
        base.read_total = 1024 * 1024;
        base.write_total = 2048;
        base.gpu_ns = 1_000_000_000;
        assert_eq!(cell_text(&p, Some(SortKey::CpuTime), None), "0:10.00");
        assert_eq!(cell_text(&p, Some(SortKey::CpuTime), Some(&base)), "0:06.00");
        assert_eq!(cell_text(&p, Some(SortKey::BytesRead), Some(&base)), "4.0 MB");
        assert_eq!(cell_text(&p, Some(SortKey::BytesWritten), Some(&base)), "0 B");
        assert_eq!(cell_text(&p, Some(SortKey::GpuTime), Some(&base)), "2.00 s");
        // a counter that went backwards (a recycled pid) does not underflow
        base.cpu_ticks = 5000;
        assert_eq!(cell_text(&p, Some(SortKey::CpuTime), Some(&base)), "0:00.00");
    }

    #[test]
    fn plain_cells() {
        let mut p = proc(5, "x", 1000, 3.25, 4096);
        p.ppid = 2;
        p.vsize = 1024 * 1024;
        p.prevents_sleep = true;
        p.gpu = 12.34;
        assert_eq!(cell_text(&p, Some(SortKey::Ppid), None), "2");
        assert_eq!(cell_text(&p, Some(SortKey::Virtual), None), "1.0 MB");
        assert_eq!(cell_text(&p, Some(SortKey::PreventingSleep), None), "Yes");
        assert_eq!(cell_text(&p, Some(SortKey::Gpu), None), "12.3");
        assert_eq!(cell_text(&p, None, None), "");
    }

    #[test]
    fn descendants_follow_children_of_children_and_stop_at_unrelated_trees() {
        let mut procs = vec![proc(1, "init", 0, 0.0, 0), proc(10, "a", 1000, 0.0, 0), proc(11, "b", 1000, 0.0, 0), proc(12, "c", 1000, 0.0, 0), proc(20, "d", 1000, 0.0, 0)];
        procs[1].ppid = 1;
        procs[2].ppid = 10;
        procs[3].ppid = 11;
        procs[4].ppid = 1;
        let family = descendants(&procs, 10);
        assert_eq!(family, HashSet::from([10, 11, 12]));
        assert_eq!(descendants(&procs, 99), HashSet::from([99]));
        let rows = visible_within(&procs, Filter::Selected, 1000, "", Sort { key: SortKey::Pid, descending: false }, Some(&family));
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![10, 11, 12]);
    }

    #[test]
    fn the_gpu_filter_keeps_processes_using_the_gpu() {
        let mut busy = proc(1, "game", 1000, 0.0, 0);
        busy.gpu = 20.0;
        let idle = proc(2, "shell", 1000, 0.0, 0);
        let rows = visible(&[busy, idle], Filter::Gpu, 1000, "", Tab::Gpu.default_sort());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 1);
    }

    #[test]
    fn energy_averages_start_as_a_mean_and_settle_towards_recent_use() {
        let mut busy = proc(1, "build", 1000, 100.0, 0);
        let mut avgs = EnergyAverages::default();
        avgs.record(&[busy.clone()], 1.0);
        assert!((avgs.mean("build") - 100.0).abs() < 1e-9);
        busy.cpu = 0.0;
        avgs.record(&[busy.clone()], 1.0);
        assert!((avgs.mean("build") - 50.0).abs() < 1e-9, "a plain mean while young");
        // processes of one name add up
        let twin = proc(2, "build", 1000, 10.0, 0);
        let mut other = EnergyAverages::default();
        other.record(&[proc(1, "build", 1000, 10.0, 0), twin], 1.0);
        assert!((other.mean("build") - 20.0).abs() < 1e-9);
        assert_eq!(other.mean("unknown"), 0.0);
        // once old, one sample moves the mean by dt / 12 hours, not by 1/n
        let mut old = EnergyAverages::default();
        old.by_name.insert("build".into(), (10.0, 1_000_000));
        old.record(&[proc(1, "build", 1000, 110.0, 0)], 60.0);
        let expected = 10.0 + 100.0 * 60.0 / TWELVE_HOURS;
        assert!((old.mean("build") - expected).abs() < 1e-6);
    }

    #[test]
    fn history_keeps_one_series_per_core_and_the_gpu_only_while_scanned() {
        use crate::sample::{CpuShare, Memory};
        let snap = |cores: Vec<f32>, gpu: Option<f32>| Snapshot {
            cpu: CpuShare::default(),
            memory: Memory::default(),
            disk_read_bps: 0.0,
            disk_write_bps: 0.0,
            ifaces: Vec::new(),
            procs: Vec::new(),
            cores,
            gpu_busy: gpu,
        };
        let mut h = History::default();
        h.record(&snap(vec![10.0, 20.0], None));
        h.record(&snap(vec![30.0, 40.0], Some(55.0)));
        assert_eq!(h.cores.len(), 2);
        assert_eq!(h.cores[1].last(), 40.0);
        assert_eq!(h.cores[0].len(), 2);
        assert_eq!(h.gpu.len(), 1);
        assert_eq!(h.gpu.last(), 55.0);
    }
}
