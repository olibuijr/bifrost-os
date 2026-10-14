//! What the window shows: the tabs and their columns, the process filter, sort and search, the graph history and the
//! number formats. Plain data with no drawing; `window.rs` holds the state and `draw.rs` paints it.

use crate::sample::{ProcSample, Snapshot, TICKS_PER_SEC};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// Samples kept for each graph (one per second, so one minute).
pub const HISTORY: usize = 60;
/// Floors for the disk and network graphs and the Dock readout, so a quiet machine does not look busy (bytes per second).
pub const DISK_FLOOR: f32 = 1024.0 * 1024.0;
pub const NET_FLOOR: f32 = 64.0 * 1024.0;

/// The title of the column that shows how long ago a process's Show Deltas baseline was taken. It has no sort key: the
/// figure is per process and only means something next to the deltas.
pub const SINCE_BASELINE: &str = "Since Baseline";

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
                Column::new(None, SINCE_BASELINE, 112.0).off(),
                threads(),
                Column::new(Some(SortKey::Ports), "Ports", 64.0).off(),
                Column::new(Some(SortKey::Ppid), "Parent PID", 88.0).off(),
                pid(),
            ],
            Tab::Memory => vec![
                name(),
                user(),
                Column::new(Some(SortKey::Memory), "Memory", 88.0).required(),
                Column::new(Some(SortKey::PrivateMem), "Private Memory", 112.0).off(),
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
                Column::new(Some(SortKey::WakeUps), "Idle Wake Ups", 104.0).off(),
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

/// Columns the View > Columns menu can hide, with the bit each takes in the stored mask.
pub const OPTIONAL_COLUMNS: [(SortKey, &str); 3] = [(SortKey::User, "User"), (SortKey::Threads, "Threads"), (SortKey::Pid, "PID")];

/// The columns of a tab without the ones whose bit is set in `hidden` (bit i = `OPTIONAL_COLUMNS[i]`). The name and
/// figure columns always stay.
pub fn visible_columns(tab: Tab, hidden: i64) -> Vec<Column> {
    tab.columns(&ColumnSet::default())
        .into_iter()
        .filter(|c| !OPTIONAL_COLUMNS.iter().enumerate().any(|(i, (k, _))| hidden & (1 << i) != 0 && c.key == Some(*k)))
        .collect()
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
    Ports,
    PrivateMem,
    WakeUps,
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
    /// The processes that own a window on the desktop. The owners come from the shell's window list, so the filter
    /// keeps every row it is given and the caller scopes the rows (`visible_within`), as for Selected.
    Windowed,
    /// The process selected when the filter was chosen, and everything it started. The scope is the caller's
    /// (`visible_within`); the filter itself keeps every row it is given.
    Selected,
    Gpu,
    /// The processes started in the last eight hours (Applications in the Last 8 Hours), by their start time.
    Recent,
}

/// Accounts below this uid belong to the system, not to a person.
pub const FIRST_PERSON_UID: u32 = 1000;
/// How far back the Recent filter looks, in seconds.
pub const RECENT_SECS: u64 = 8 * 3600;

impl Filter {
    pub const ALL: [Filter; 10] = [
        Filter::All,
        Filter::Mine,
        Filter::Others,
        Filter::System,
        Filter::Active,
        Filter::Inactive,
        Filter::Windowed,
        Filter::Selected,
        Filter::Gpu,
        Filter::Recent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All Processes",
            Filter::Mine => "My Processes",
            Filter::Others => "Other Users",
            Filter::System => "System Processes",
            Filter::Active => "Active Processes",
            Filter::Inactive => "Inactive Processes",
            Filter::Windowed => "Windowed Processes",
            Filter::Selected => "Selected Processes",
            Filter::Gpu => "GPU Processes",
            Filter::Recent => "Applications in the Last 8 Hours",
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
            Filter::Windowed | Filter::Selected => true,
            Filter::Gpu => p.gpu > 0.0,
            Filter::Recent => p.age_secs.map_or(false, |age| age <= RECENT_SECS),
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
        Some(SortKey::Ports) => p.ports.map_or_else(|| "-".to_string(), |n| n.to_string()),
        Some(SortKey::PrivateMem) => p.private.map_or_else(|| "-".to_string(), bytes),
        Some(SortKey::WakeUps) => format!("{:.1}/s", p.wakeups),
        None => String::new(),
    }
}

/// One table row as tab-separated text (Edit > Copy): the columns the tab shows, in order. The CPU figure carries its
/// percent sign, as it does in the footer and the graph. `since` is the Since Baseline column's text (empty without a
/// baseline).
pub fn row_text(p: &ProcSample, tab: Tab, set: &ColumnSet, base: Option<&ProcSample>, since: &str) -> String {
    if tab == Tab::Network {
        return [p.name.clone(), p.user.clone(), p.pid.to_string()].join("\t");
    }
    let fields: Vec<String> = tab
        .columns(set)
        .iter()
        .map(|c| {
            if c.title == SINCE_BASELINE {
                since.to_string()
            } else if c.key == Some(SortKey::Cpu) {
                percent(p.cpu)
            } else {
                cell_text(p, c.key, base)
            }
        })
        .collect();
    fields.join("\t")
}

/// How long ago a baseline was taken, in whole units: "45 s", "3 min 12 s" or "1 h 4 min".
pub fn elapsed_text(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min {} s", s / 60, s % 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

/// The sort keys whose figures Show Deltas counts from a baseline.
pub fn is_cumulative(key: SortKey) -> bool {
    matches!(key, SortKey::CpuTime | SortKey::BytesRead | SortKey::BytesWritten | SortKey::GpuTime)
}

/// The amount a row's cumulative figure has grown since its baseline, for sorting. A process without a baseline counts
/// as nothing, so the deltas sort puts those together.
fn delta_of(p: &ProcSample, key: SortKey, base: Option<&ProcSample>) -> u64 {
    let Some(b) = base else { return 0 };
    let since = |now: u64, then: u64| now.saturating_sub(then);
    match key {
        SortKey::CpuTime => since(p.cpu_ticks, b.cpu_ticks),
        SortKey::BytesRead => since(p.read_total, b.read_total),
        SortKey::BytesWritten => since(p.write_total, b.write_total),
        SortKey::GpuTime => since(p.gpu_ns, b.gpu_ns),
        _ => 0,
    }
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

    /// The averages as text, one name per line: its count, its mean, then the name. Line breaks and tabs in a name
    /// become spaces, so every line reads back.
    pub fn to_text(&self) -> String {
        let mut names: Vec<(&String, &(f64, u64))> = self.by_name.iter().collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        names.iter().map(|(name, (mean, count))| format!("{count}\t{mean}\t{}\n", name.replace(|c: char| c == '\n' || c == '\t', " "))).collect()
    }

    /// The averages written by `to_text`. Lines that do not read are left out.
    pub fn from_text(text: &str) -> EnergyAverages {
        let mut by_name = HashMap::new();
        for line in text.lines() {
            let mut f = line.splitn(3, '\t');
            let (Some(count), Some(mean), Some(name)) = (f.next(), f.next(), f.next()) else { continue };
            let (Ok(count), Ok(mean)) = (count.parse::<u64>(), mean.parse::<f64>()) else { continue };
            if !name.is_empty() && mean.is_finite() {
                by_name.insert(name.to_string(), (mean, count));
            }
        }
        EnergyAverages { by_name }
    }
}

/// An energy score for one process: its CPU share plus its disk traffic. Activity Monitor's Energy column is
/// not public; this is a heuristic that ranks the same busy processes first (one MiB/s of disk weighs as 2 %).
pub fn energy(p: &ProcSample) -> f64 {
    // compliance-allow: LAY-03 Converts byte rates to MiB/s; these are unit factors, not screen dimensions.
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
        SortKey::Ports => a.ports.unwrap_or(0).cmp(&b.ports.unwrap_or(0)),
        SortKey::PrivateMem => a.private.unwrap_or(0).cmp(&b.private.unwrap_or(0)),
        SortKey::WakeUps => a.wakeups.total_cmp(&b.wakeups),
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

/// Like `visible_within`, and with Show Deltas on (`baselines` not empty), a cumulative sort key orders the rows by
/// what each process has done since its baseline instead of by its whole history.
pub fn visible_scoped(
    procs: &[ProcSample],
    filter: Filter,
    own_uid: u32,
    query: &str,
    sort: Sort,
    only: Option<&HashSet<u32>>,
    baselines: &HashMap<u32, ProcSample>,
) -> Vec<ProcSample> {
    let mut rows = visible_within(procs, filter, own_uid, query, sort, only);
    if is_cumulative(sort.key) && !baselines.is_empty() {
        rows.sort_by(|a, b| {
            let o = delta_of(a, sort.key, baselines.get(&a.pid)).cmp(&delta_of(b, sort.key, baselines.get(&b.pid)));
            let o = if sort.descending { o.reverse() } else { o };
            o.then(a.pid.cmp(&b.pid))
        });
    }
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
    /// Memory pressure: the share of the last ten seconds some task waited for memory (PSI), in percent.
    pub pressure: Series,
}

/// How serious a memory pressure share is, for the strip in the Memory footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Elevated,
    Critical,
}

/// Normal below 5 %, elevated below 15 %, critical from 15 %: the share of time some task waited for memory.
pub fn pressure_level(percent: f32) -> Pressure {
    if percent < 5.0 {
        Pressure::Normal
    } else if percent < 15.0 {
        Pressure::Elevated
    } else {
        Pressure::Critical
    }
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

/// What the Dock tile shows (View > Dock Icon): nothing special, or a live figure as the badge with a bar under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockMode {
    Off,
    Cpu,
    Memory,
    Disk,
    Network,
}

impl DockMode {
    pub const ALL: [DockMode; 5] = [DockMode::Off, DockMode::Cpu, DockMode::Memory, DockMode::Disk, DockMode::Network];

    pub fn label(self) -> &'static str {
        match self {
            DockMode::Off => "Application Icon",
            DockMode::Cpu => "Show CPU Usage",
            DockMode::Memory => "Show Memory Usage",
            DockMode::Disk => "Show Disk Activity",
            DockMode::Network => "Show Network Usage",
        }
    }

    pub fn index(self) -> i64 {
        DockMode::ALL.iter().position(|m| *m == self).unwrap_or(0) as i64
    }

    /// The stored index back to a mode; anything unknown is the plain icon.
    pub fn from_index(i: i64) -> DockMode {
        usize::try_from(i).ok().and_then(|i| DockMode::ALL.get(i).copied()).unwrap_or(DockMode::Off)
    }
}

/// The badge text (at most four characters) and the bar fraction to put on the Dock tile.
#[derive(Clone, Debug, PartialEq)]
pub struct DockContent {
    pub badge: String,
    pub fraction: Option<f32>,
}

impl DockContent {
    /// Nothing on the tile.
    pub fn cleared() -> Self {
        DockContent { badge: String::new(), fraction: None }
    }
}

/// A rate in at most four characters: "512", "1.5K", "12M", "2.5G".
pub fn compact_rate(bps: f64) -> String {
    const UNITS: [&str; 5] = ["", "K", "M", "G", "T"];
    let mut v = bps.max(0.0);
    let mut i = 0;
    while v >= 999.5 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}")
    } else if v < 9.95 {
        format!("{v:.1}{}", UNITS[i])
    } else {
        format!("{v:.0}{}", UNITS[i])
    }
}

/// The Dock tile for a mode from the latest reading: CPU and memory as a percentage with a bar of the same size; disk
/// and network as the combined rate with a bar against the busiest second of the last minute.
pub fn dock_content(mode: DockMode, snap: &Snapshot, history: &History) -> DockContent {
    let scaled = |now: f64, peak: f32, floor: f32| (now as f32 / peak.max(floor)).clamp(0.0, 1.0);
    match mode {
        DockMode::Off => DockContent::cleared(),
        DockMode::Cpu => DockContent { badge: format!("{:.0}%", snap.cpu.busy), fraction: Some((snap.cpu.busy / 100.0).clamp(0.0, 1.0)) },
        DockMode::Memory => {
            let used = snap.memory.used_percent();
            DockContent { badge: format!("{used:.0}%"), fraction: Some((used / 100.0).clamp(0.0, 1.0)) }
        }
        DockMode::Disk => {
            let now = snap.disk_read_bps + snap.disk_write_bps;
            DockContent { badge: compact_rate(now), fraction: Some(scaled(now, history.disk_read.peak() + history.disk_write.peak(), DISK_FLOOR)) }
        }
        DockMode::Network => {
            let now: f64 = snap.ifaces.iter().map(|i| i.rx_bps + i.tx_bps).sum();
            DockContent { badge: compact_rate(now), fraction: Some(scaled(now, history.net_rx.peak() + history.net_tx.peak(), NET_FLOOR)) }
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
    // compliance-allow: LAY-03 Binary bytes-per-unit threshold for readable sizes, not window geometry.
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
    fn hidden_columns_drop_out_but_name_and_figure_stay() {
        assert_eq!(visible_columns(Tab::Cpu, 0).len(), 5);
        let titles: Vec<&str> = visible_columns(Tab::Cpu, 0b110).iter().map(|c| c.title).collect();
        assert_eq!(titles, vec!["Process", "User", "% CPU"]);
        assert_eq!(visible_columns(Tab::Cpu, 0b111).len(), 2);
        assert_eq!(visible_columns(Tab::Network, 0b111).len(), 5);
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
        assert_eq!(row_text(&p, Tab::Cpu, &set, None, ""), "sh\tolibuijr\t12.5%\t1\t7");
        assert_eq!(row_text(&p, Tab::Memory, &set, None, ""), "sh\tolibuijr\t2.0 KB\t1\t7");
    }

    #[test]
    fn the_since_baseline_column_copies_its_text_where_it_is_shown() {
        let p = proc(7, "sh", 1000, 12.5, 2048);
        let mut set = ColumnSet::default();
        let since = Tab::Cpu.all_columns().into_iter().find(|c| c.title == SINCE_BASELINE).unwrap();
        set.toggle(Tab::Cpu, &since);
        assert_eq!(row_text(&p, Tab::Cpu, &set, None, "3 min 2 s"), "sh\tolibuijr\t12.5%\t3 min 2 s\t1\t7");
        assert_eq!(row_text(&p, Tab::Cpu, &ColumnSet::default(), None, "3 min 2 s"), "sh\tolibuijr\t12.5%\t1\t7", "hidden, so not copied");
    }

    #[test]
    fn elapsed_time_reads_in_seconds_minutes_and_hours() {
        assert_eq!(elapsed_text(0.0), "0 s");
        assert_eq!(elapsed_text(45.9), "45 s");
        assert_eq!(elapsed_text(192.0), "3 min 12 s");
        assert_eq!(elapsed_text(3840.0), "1 h 4 min");
        assert_eq!(elapsed_text(-5.0), "0 s");
    }

    #[test]
    fn the_deltas_sort_orders_by_growth_since_the_baseline() {
        let mut big = proc(1, "big", 1000, 0.0, 0);
        big.cpu_ticks = 10_000;
        let mut busy = proc(2, "busy", 1000, 0.0, 0);
        busy.cpu_ticks = 500;
        let mut quiet = proc(3, "quiet", 1000, 0.0, 0);
        quiet.cpu_ticks = 100;
        let sort = Sort { key: SortKey::CpuTime, descending: true };
        // no baselines: the whole history decides
        let plain = visible_scoped(&[big.clone(), busy.clone(), quiet.clone()], Filter::All, 1000, "", sort, None, &HashMap::new());
        assert_eq!(plain.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![1, 2, 3]);
        // baselines taken: big has grown least, busy most
        let mut base = HashMap::new();
        let mut big0 = big.clone();
        big0.cpu_ticks = 9_900;
        let mut busy0 = busy.clone();
        busy0.cpu_ticks = 100;
        base.insert(1, big0);
        base.insert(2, busy0);
        let rows = visible_scoped(&[big, busy, quiet], Filter::All, 1000, "", sort, None, &base);
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 1, 3], "the process with no baseline counts as nothing");
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
    fn rates_fit_the_dock_badge() {
        assert_eq!(compact_rate(0.0), "0");
        assert_eq!(compact_rate(512.0), "512");
        assert_eq!(compact_rate(1_500.0), "1.5K");
        assert_eq!(compact_rate(12_000.0), "12K");
        assert_eq!(compact_rate(999_700.0), "1.0M");
        assert_eq!(compact_rate(2_500_000_000.0), "2.5G");
        assert_eq!(compact_rate(-5.0), "0");
        for v in [1.0, 999.0, 1e3, 9.9e3, 1e5, 9.99e5, 1e7, 5e8, 1e12] {
            assert!(compact_rate(v).len() <= 4, "{v} -> {}", compact_rate(v));
        }
    }

    #[test]
    fn the_dock_shows_the_chosen_figure_with_a_bar() {
        use crate::sample::{CpuShare, IfaceRate, Memory};
        let mut history = History::default();
        history.disk_read.push(1_000_000.0);
        history.disk_write.push(1_000_000.0);
        let snap = Snapshot {
            cpu: CpuShare { busy: 37.4, user: 20.0, system: 17.4, idle: 62.6 },
            memory: Memory { total: 1000, available: 250, ..Memory::default() },
            disk_read_bps: 1_500_000.0,
            disk_write_bps: 500_000.0,
            ifaces: vec![IfaceRate { name: "e".into(), rx_bps: 30_000.0, tx_bps: 2_000.0, rx_total: 0, tx_total: 0, rx_packets: 0, tx_packets: 0 }],
            procs: Vec::new(),
            cores: Vec::new(),
            gpu_busy: None,
        };
        assert_eq!(dock_content(DockMode::Off, &snap, &history), DockContent::cleared());
        let cpu = dock_content(DockMode::Cpu, &snap, &history);
        assert_eq!(cpu.badge, "37%");
        assert!((cpu.fraction.unwrap() - 0.374).abs() < 1e-4);
        let mem = dock_content(DockMode::Memory, &snap, &history);
        assert_eq!(mem.badge, "75%");
        assert!((mem.fraction.unwrap() - 0.75).abs() < 1e-4);
        let disk = dock_content(DockMode::Disk, &snap, &history);
        assert_eq!(disk.badge, "2.0M");
        assert!((disk.fraction.unwrap() - 1.0).abs() < 1e-4, "above the recorded peak is a full bar");
        let net = dock_content(DockMode::Network, &snap, &history);
        assert_eq!(net.badge, "32K");
        assert!((net.fraction.unwrap() - 32_000.0 / NET_FLOOR).abs() < 1e-3);
    }

    #[test]
    fn dock_modes_round_trip_through_the_stored_index() {
        for mode in DockMode::ALL {
            assert_eq!(DockMode::from_index(mode.index()), mode);
        }
        assert_eq!(DockMode::from_index(-1), DockMode::Off);
        assert_eq!(DockMode::from_index(99), DockMode::Off);
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
    fn the_windowed_filter_is_scoped_to_the_window_owners() {
        let procs = vec![proc(1, "init", 0, 0.0, 0), proc(2, "editor", 1000, 0.0, 0), proc(3, "daemon", 1000, 0.0, 0)];
        let owners = HashSet::from([2]);
        let sort = Sort { key: SortKey::Pid, descending: false };
        let rows = visible_within(&procs, Filter::Windowed, 1000, "", sort, Some(&owners));
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2]);
        assert_eq!(Filter::Windowed.label(), "Windowed Processes");
        assert!(Filter::ALL.contains(&Filter::Windowed));
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

    #[test]
    fn energy_averages_survive_a_restart() {
        let mut a = EnergyAverages::default();
        a.record(&[proc(1, "build", 1000, 40.0, 0), proc(2, "tab\tname", 1000, 5.0, 0)], 60.0);
        let text = a.to_text();
        let b = EnergyAverages::from_text(&text);
        assert!((b.mean("build") - a.mean("build")).abs() < 1e-12);
        assert_eq!(b.to_text(), text, "every line reads back to the same line");
        assert_eq!(EnergyAverages::from_text("garbage\n\n1\tx\ty\n").to_text(), "");
    }

    #[test]
    fn the_recent_filter_keeps_processes_started_in_the_last_eight_hours() {
        let mut young = proc(1, "young", 1000, 0.0, 0);
        young.age_secs = Some(3600);
        let mut old = proc(2, "old", 1000, 0.0, 0);
        old.age_secs = Some(RECENT_SECS + 1);
        let unknown = proc(3, "unknown", 1000, 0.0, 0);
        let sort = Sort { key: SortKey::Pid, descending: false };
        let rows = visible(&[young, old, unknown], Filter::Recent, 1000, "", sort);
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![1]);
        assert_eq!(Filter::Recent.label(), "Applications in the Last 8 Hours");
        assert!(Filter::ALL.contains(&Filter::Recent));
    }

    #[test]
    fn memory_pressure_is_normal_elevated_or_critical() {
        assert_eq!(pressure_level(0.0), Pressure::Normal);
        assert_eq!(pressure_level(4.9), Pressure::Normal);
        assert_eq!(pressure_level(5.0), Pressure::Elevated);
        assert_eq!(pressure_level(14.9), Pressure::Elevated);
        assert_eq!(pressure_level(15.0), Pressure::Critical);
    }
}
