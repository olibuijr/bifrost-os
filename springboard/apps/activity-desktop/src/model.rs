//! What the window shows: the tabs and their columns, the process filter, sort and search, the graph history and the
//! number formats. Plain data with no drawing; `window.rs` holds the state and `draw.rs` paints it.

use crate::sample::{ProcSample, Snapshot};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

/// Samples kept for each graph (one per second, so one minute).
pub const HISTORY: usize = 60;

/// The six tabs above the graph. Each picks the graph and the table's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Gpu,
    Memory,
    Energy,
    Disk,
    Network,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Cpu, Tab::Gpu, Tab::Memory, Tab::Energy, Tab::Disk, Tab::Network];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
            Tab::Gpu => "GPU",
            Tab::Memory => "Memory",
            Tab::Energy => "Energy",
            Tab::Disk => "Disk",
            Tab::Network => "Network",
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
            Tab::Gpu => Sort { key: SortKey::Gpu, descending: true },
            Tab::Memory => Sort { key: SortKey::Memory, descending: true },
            Tab::Energy => Sort { key: SortKey::Energy, descending: true },
            Tab::Disk => Sort { key: SortKey::DiskRead, descending: true },
            Tab::Network => Sort { key: SortKey::Name, descending: false },
        }
    }

    /// The table's columns. A width of 0 takes the space the fixed columns leave (the name column).
    pub fn columns(self) -> Vec<Column> {
        let process_head = |figure: Column| {
            vec![Column::new(Some(SortKey::Name), "Process", 0.0), Column::new(Some(SortKey::User), "User", 96.0), figure, Column::new(Some(SortKey::Threads), "Threads", 64.0), Column::new(Some(SortKey::Pid), "PID", 56.0)]
        };
        match self {
            Tab::Cpu => process_head(Column::new(Some(SortKey::Cpu), "% CPU", 72.0)),
            Tab::Gpu => process_head(Column::new(Some(SortKey::Gpu), "% GPU", 72.0)),
            Tab::Memory => process_head(Column::new(Some(SortKey::Memory), "Memory", 88.0)),
            Tab::Energy => {
                let mut cols = process_head(Column::new(Some(SortKey::Energy), "Energy Impact", 112.0));
                // the twelve hour mean and Preventing Sleep sit beside the impact, before Threads and PID
                cols.insert(3, Column::new(Some(SortKey::EnergyAvg), "12 hr Power", 96.0));
                cols.insert(4, Column::new(Some(SortKey::PreventsSleep), "Preventing Sleep", 128.0));
                cols
            }
            Tab::Disk => vec![
                Column::new(Some(SortKey::Name), "Process", 0.0),
                Column::new(Some(SortKey::User), "User", 96.0),
                Column::new(Some(SortKey::DiskRead), "Read/s", 96.0),
                Column::new(Some(SortKey::DiskWrite), "Written/s", 104.0),
                Column::new(Some(SortKey::Pid), "PID", 56.0),
            ],
            Tab::Network => vec![
                Column::new(None, "Interface", 0.0),
                Column::new(None, "Received/s", 104.0),
                Column::new(None, "Sent/s", 104.0),
                Column::new(None, "Total Received", 120.0),
                Column::new(None, "Total Sent", 120.0),
                Column::new(None, "Packets In", 96.0),
                Column::new(None, "Packets Out", 100.0),
            ],
        }
    }
}

/// Columns the View > Columns menu can hide, with the bit each takes in the stored mask.
pub const OPTIONAL_COLUMNS: [(SortKey, &str); 3] = [(SortKey::User, "User"), (SortKey::Threads, "Threads"), (SortKey::Pid, "PID")];

/// Columns the View > Columns menu can add to the process tabs, hidden until chosen: the key, the title and the
/// width. Bit i of the stored mask shows `EXTRA_COLUMNS[i]`.
pub const EXTRA_COLUMNS: [(SortKey, &str, f32); 11] = [
    (SortKey::Ppid, "Parent PID", 88.0),
    (SortKey::State, "State", 72.0),
    (SortKey::RealMem, "Real Mem", 88.0),
    (SortKey::PrivateMem, "Private Mem", 96.0),
    (SortKey::Compressed, "Compressed Mem", 120.0),
    (SortKey::CpuTime, "CPU Time", 96.0),
    (SortKey::Wakeups, "Idle Wake Ups", 104.0),
    (SortKey::BytesRead, "Bytes Read", 92.0),
    (SortKey::BytesWritten, "Bytes Written", 104.0),
    (SortKey::Gpu, "% GPU", 72.0),
    (SortKey::PreventsSleep, "Preventing Sleep", 128.0),
];

/// The columns of a tab without the ones whose bit is set in `hidden` (bit i = `OPTIONAL_COLUMNS[i]`) and with the
/// extra ones whose bit is set in `extras` (bit i = `EXTRA_COLUMNS[i]`). The name and figure columns always stay; an
/// extra column goes before PID, or last, and never repeats a column the tab already has. The Network tab has none.
pub fn visible_columns(tab: Tab, hidden: i64, extras: i64) -> Vec<Column> {
    let mut cols: Vec<Column> =
        tab.columns().into_iter().filter(|c| !OPTIONAL_COLUMNS.iter().enumerate().any(|(i, (k, _))| hidden & (1 << i) != 0 && c.key == Some(*k))).collect();
    if tab == Tab::Network {
        return cols;
    }
    for (i, (key, title, width)) in EXTRA_COLUMNS.iter().enumerate() {
        // Real Mem is the same reading as the Memory tab's own column
        let repeats = cols.iter().any(|c| c.key == Some(*key) || (*key == SortKey::RealMem && c.key == Some(SortKey::Memory)));
        if extras & (1 << i) == 0 || repeats {
            continue;
        }
        let at = cols.iter().position(|c| c.key == Some(SortKey::Pid)).unwrap_or(cols.len());
        cols.insert(at, Column::new(Some(*key), *title, *width));
    }
    cols
}

/// A table column: its sort key (None when the column does not sort), its title and its width in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub key: Option<SortKey>,
    pub title: &'static str,
    pub width: f32,
}

impl Column {
    pub fn new(key: Option<SortKey>, title: &'static str, width: f32) -> Self {
        Column { key, title, width }
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
    DiskRead,
    DiskWrite,
    Threads,
    Pid,
    Ppid,
    State,
    RealMem,
    PrivateMem,
    Compressed,
    CpuTime,
    Wakeups,
    BytesRead,
    BytesWritten,
    Gpu,
    PreventsSleep,
    EnergyAvg,
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
/// ones using / not using the CPU right now, the ones with a window, the ones using the GPU, or the ones that were
/// selected when the filter was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Mine,
    Others,
    System,
    Active,
    Inactive,
    Windowed,
    Gpu,
    Selected,
}

/// Accounts below this uid belong to the system, not to a person.
pub const FIRST_PERSON_UID: u32 = 1000;

impl Filter {
    pub const ALL: [Filter; 9] =
        [Filter::All, Filter::Mine, Filter::Others, Filter::System, Filter::Active, Filter::Inactive, Filter::Windowed, Filter::Gpu, Filter::Selected];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All Processes",
            Filter::Mine => "My Processes",
            Filter::Others => "Other Users",
            Filter::System => "System Processes",
            Filter::Active => "Active Processes",
            Filter::Inactive => "Inactive Processes",
            Filter::Windowed => "Windowed Processes",
            Filter::Gpu => "GPU Processes",
            Filter::Selected => "Selected Processes",
        }
    }

    pub fn keep(self, p: &ProcSample, scope: &Scope) -> bool {
        match self {
            Filter::All => true,
            Filter::Mine => p.uid == scope.own_uid,
            Filter::Others => p.uid != scope.own_uid,
            Filter::System => p.uid < FIRST_PERSON_UID,
            Filter::Active => p.cpu > 0.0,
            Filter::Inactive => p.cpu <= 0.0,
            Filter::Windowed => scope.windowed.contains(&p.pid),
            Filter::Gpu => p.gpu > 0.0,
            Filter::Selected => scope.pinned.contains(&p.pid),
        }
    }
}

/// What the table needs besides the processes: the filter and its inputs, the search, the sort and the Show Deltas
/// baseline.
pub struct Scope<'a> {
    pub filter: Filter,
    pub own_uid: u32,
    pub query: &'a str,
    pub sort: Sort,
    /// The pids Selected Processes keeps.
    pub pinned: &'a [u32],
    /// The pids that own a window on the desktop.
    pub windowed: &'a HashSet<u32>,
    pub baseline: Option<&'a Baseline>,
}

/// The counters of one process that Show Deltas reports relative to a starting point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub ticks: u64,
    pub wakeups: u64,
    pub read: u64,
    pub written: u64,
}

impl Counters {
    pub fn of(p: &ProcSample) -> Counters {
        Counters { ticks: p.cpu_ticks, wakeups: p.wakeups, read: p.read_total, written: p.write_total }
    }
}

/// The counters of every process when View > Show Deltas for Process was switched on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    map: HashMap<u32, Counters>,
}

impl Baseline {
    pub fn capture(procs: &[ProcSample]) -> Baseline {
        Baseline { map: procs.iter().map(|p| (p.pid, Counters::of(p))).collect() }
    }

    /// The counters since the baseline. A process that started later counts from zero, so all of its counters show.
    pub fn since(&self, p: &ProcSample) -> Counters {
        let now = Counters::of(p);
        let Some(b) = self.map.get(&p.pid) else { return now };
        Counters { ticks: now.ticks.saturating_sub(b.ticks), wakeups: now.wakeups.saturating_sub(b.wakeups), read: now.read.saturating_sub(b.read), written: now.written.saturating_sub(b.written) }
    }
}

/// The counters a row shows: since the baseline when Show Deltas is on, otherwise the totals.
pub fn counters(p: &ProcSample, baseline: Option<&Baseline>) -> Counters {
    baseline.map_or_else(|| Counters::of(p), |b| b.since(p))
}

/// CPU time as `m:ss.cc`, or `h:mm:ss.cc` from an hour on, from ticks of 1/100 second.
pub fn cpu_time(ticks: u64) -> String {
    let cs = ticks % 100;
    let secs = ticks / 100;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{cs:02}")
    } else {
        format!("{m}:{s:02}.{cs:02}")
    }
}

/// The text of one process-table cell, by the column's key. `baseline` turns the counter columns into deltas.
pub fn cell_text(p: &ProcSample, key: Option<SortKey>, baseline: Option<&Baseline>) -> String {
    let c = counters(p, baseline);
    match key {
        Some(SortKey::Name) => p.name.clone(),
        Some(SortKey::User) => p.user.clone(),
        Some(SortKey::Cpu) => format!("{:.1}", p.cpu),
        Some(SortKey::Memory) | Some(SortKey::RealMem) => bytes(p.rss),
        Some(SortKey::Energy) => format!("{:.1}", energy(p)),
        Some(SortKey::EnergyAvg) => format!("{:.1}", p.energy_avg),
        Some(SortKey::DiskRead) => rate(p.read_bps),
        Some(SortKey::DiskWrite) => rate(p.write_bps),
        Some(SortKey::Threads) => p.threads.to_string(),
        Some(SortKey::Pid) => p.pid.to_string(),
        Some(SortKey::Ppid) => p.ppid.to_string(),
        Some(SortKey::State) => state_name(p.state).to_string(),
        Some(SortKey::PrivateMem) => bytes(p.private_mem),
        Some(SortKey::Compressed) => bytes(p.swapped),
        Some(SortKey::CpuTime) => cpu_time(c.ticks),
        Some(SortKey::Wakeups) => c.wakeups.to_string(),
        Some(SortKey::BytesRead) => bytes(c.read),
        Some(SortKey::BytesWritten) => bytes(c.written),
        Some(SortKey::Gpu) => format!("{:.1}", p.gpu),
        Some(SortKey::PreventsSleep) => if p.prevents_sleep { "Yes" } else { "No" }.to_string(),
        None => String::new(),
    }
}

/// The scheduler state letter of `stat` as a word.
pub fn state_name(state: char) -> &'static str {
    match state {
        'R' => "Running",
        'S' | 'I' => "Sleeping",
        'D' => "Waiting",
        'T' | 't' => "Stopped",
        'Z' => "Zombie",
        'X' => "Dead",
        _ => "Unknown",
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

/// One table row as tab-separated text (Edit > Copy): the columns of the tab, in order.
pub fn row_text(p: &ProcSample, tab: Tab) -> String {
    let fields: Vec<String> = match tab {
        Tab::Cpu => vec![p.name.clone(), p.user.clone(), percent(p.cpu), p.threads.to_string(), p.pid.to_string()],
        Tab::Gpu => vec![p.name.clone(), p.user.clone(), percent(p.gpu), p.threads.to_string(), p.pid.to_string()],
        Tab::Memory => vec![p.name.clone(), p.user.clone(), bytes(p.rss), p.threads.to_string(), p.pid.to_string()],
        Tab::Energy => vec![
            p.name.clone(),
            p.user.clone(),
            format!("{:.1}", energy(p)),
            format!("{:.1}", p.energy_avg),
            if p.prevents_sleep { "Yes" } else { "No" }.to_string(),
            p.threads.to_string(),
            p.pid.to_string(),
        ],
        Tab::Disk => vec![p.name.clone(), p.user.clone(), rate(p.read_bps), rate(p.write_bps), p.pid.to_string()],
        Tab::Network => vec![p.name.clone(), p.user.clone(), p.pid.to_string()],
    };
    fields.join("\t")
}

/// An energy score for one process: its CPU share plus its disk traffic. Activity Monitor's Energy column is
/// not public; this is a heuristic that ranks the same busy processes first (one MiB/s of disk weighs as 2 %).
pub fn energy(p: &ProcSample) -> f64 {
    p.cpu as f64 + (p.read_bps + p.write_bps) / (1024.0 * 1024.0) * 2.0
}

fn compare(a: &ProcSample, b: &ProcSample, key: SortKey, baseline: Option<&Baseline>) -> Ordering {
    let (ca, cb) = (counters(a, baseline), counters(b, baseline));
    match key {
        SortKey::Ppid => a.ppid.cmp(&b.ppid),
        SortKey::State => state_name(a.state).cmp(state_name(b.state)),
        SortKey::RealMem => a.rss.cmp(&b.rss),
        SortKey::PrivateMem => a.private_mem.cmp(&b.private_mem),
        SortKey::Compressed => a.swapped.cmp(&b.swapped),
        SortKey::CpuTime => ca.ticks.cmp(&cb.ticks),
        SortKey::Wakeups => ca.wakeups.cmp(&cb.wakeups),
        SortKey::BytesRead => ca.read.cmp(&cb.read),
        SortKey::BytesWritten => ca.written.cmp(&cb.written),
        SortKey::Gpu => a.gpu.total_cmp(&b.gpu),
        SortKey::PreventsSleep => a.prevents_sleep.cmp(&b.prevents_sleep),
        SortKey::EnergyAvg => a.energy_avg.total_cmp(&b.energy_avg),
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortKey::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
        SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
        SortKey::Memory => a.rss.cmp(&b.rss),
        SortKey::Energy => energy(a).total_cmp(&energy(b)),
        SortKey::DiskRead => a.read_bps.total_cmp(&b.read_bps),
        SortKey::DiskWrite => a.write_bps.total_cmp(&b.write_bps),
        SortKey::Threads => a.threads.cmp(&b.threads),
        SortKey::Pid => a.pid.cmp(&b.pid),
    }
}

/// Whether a process matches the search text (already lower case): its name, user or PID contains it.
pub fn matches(p: &ProcSample, query: &str) -> bool {
    query.is_empty() || p.name.to_lowercase().contains(query) || p.user.to_lowercase().contains(query) || p.pid.to_string().contains(query)
}

/// The rows of the process table: filtered, searched and sorted. Ties keep PID order so rows do not swap places.
pub fn visible(procs: &[ProcSample], scope: &Scope) -> Vec<ProcSample> {
    let mut rows: Vec<ProcSample> = procs.iter().filter(|p| scope.filter.keep(p, scope) && matches(p, scope.query)).cloned().collect();
    rows.sort_by(|a, b| {
        let o = compare(a, b, scope.sort.key, scope.baseline);
        let o = if scope.sort.descending { o.reverse() } else { o };
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
    pub gpu: Series,
    pub pressure: Series,
    /// One series per core, in core order.
    pub cores: Vec<Series>,
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
        self.gpu.push(snap.gpu_busy);
        self.pressure.push(snap.pressure.unwrap_or(0.0));
        if self.cores.len() != snap.cores.len() {
            self.cores.resize_with(snap.cores.len(), Series::default);
        }
        for (series, busy) in self.cores.iter_mut().zip(&snap.cores) {
            series.push(*busy);
        }
    }
}

/// What the Dock tile shows (View > Dock Icon): the app's icon, or a live reading as the tile's progress bar and
/// badge text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockIcon {
    Application,
    Cpu,
    Memory,
    Disk,
    Network,
}

impl DockIcon {
    pub const ALL: [DockIcon; 5] = [DockIcon::Application, DockIcon::Cpu, DockIcon::Memory, DockIcon::Disk, DockIcon::Network];

    pub fn label(self) -> &'static str {
        match self {
            DockIcon::Application => "Application Icon",
            DockIcon::Cpu => "Show CPU Usage",
            DockIcon::Memory => "Show Memory Usage",
            DockIcon::Disk => "Show Disk Activity",
            DockIcon::Network => "Show Network Usage",
        }
    }

    /// The stored index back to a choice; anything unknown is the application icon.
    pub fn from_index(i: i64) -> DockIcon {
        usize::try_from(i).ok().and_then(|i| DockIcon::ALL.get(i).copied()).unwrap_or(DockIcon::Application)
    }

    pub fn index(self) -> i64 {
        DockIcon::ALL.iter().position(|d| *d == self).unwrap_or(0) as i64
    }

    /// The badge text (empty for none; the Dock keeps four characters) and the progress fraction (0 to 1) for a reading.
    pub fn tile(self, snap: &Snapshot) -> (String, Option<f32>) {
        let short = |bps: f64| {
            let mb = bps / (1024.0 * 1024.0);
            if mb >= 10.0 {
                format!("{mb:.0}M")
            } else {
                format!("{mb:.1}M")
            }
        };
        match self {
            DockIcon::Application => (String::new(), None),
            DockIcon::Cpu => (format!("{:.0}%", snap.cpu.busy), Some(snap.cpu.busy / 100.0)),
            DockIcon::Memory => (format!("{:.0}%", snap.memory.used_percent()), Some(snap.memory.used_percent() / 100.0)),
            DockIcon::Disk => (short(snap.disk_read_bps + snap.disk_write_bps), None),
            DockIcon::Network => (short(snap.ifaces.iter().map(|i| i.rx_bps + i.tx_bps).sum::<f64>()), None),
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

    fn scope<'a>(filter: Filter, query: &'a str, sort: Sort, windowed: &'a HashSet<u32>, pinned: &'a [u32]) -> Scope<'a> {
        Scope { filter, own_uid: 1000, query, sort, pinned, windowed, baseline: None }
    }

    fn pid_sort() -> Sort {
        Sort { key: SortKey::Pid, descending: false }
    }

    #[test]
    fn hidden_columns_drop_out_but_name_and_figure_stay() {
        assert_eq!(visible_columns(Tab::Cpu, 0, 0).len(), 5);
        let titles: Vec<&str> = visible_columns(Tab::Cpu, 0b110, 0).iter().map(|c| c.title).collect();
        assert_eq!(titles, vec!["Process", "User", "% CPU"]);
        assert_eq!(visible_columns(Tab::Cpu, 0b111, 0).len(), 2);
        assert_eq!(visible_columns(Tab::Network, 0b111, 0).len(), 7);
    }

    #[test]
    fn extra_columns_appear_before_pid_and_never_twice() {
        let titles = |tab, extras| visible_columns(tab, 0, extras).iter().map(|c| c.title).collect::<Vec<_>>();
        // bit 5 = CPU Time, bit 2 = Real Mem
        assert_eq!(titles(Tab::Cpu, 1 << 5), vec!["Process", "User", "% CPU", "Threads", "CPU Time", "PID"]);
        // Real Mem would repeat the Memory column, so the Memory tab does not get a second one
        assert_eq!(titles(Tab::Memory, 1 << 2).len(), 5);
        // without PID the extra column goes last
        let no_pid = visible_columns(Tab::Cpu, 0b100, 1 << 5);
        assert_eq!(no_pid.last().map(|c| c.title), Some("CPU Time"));
        assert_eq!(visible_columns(Tab::Network, 0, 0x7ff).len(), 7);
    }

    #[test]
    fn the_energy_tab_has_the_average_and_preventing_sleep_columns() {
        let titles: Vec<&str> = Tab::Energy.columns().iter().map(|c| c.title).collect();
        assert_eq!(titles, vec!["Process", "User", "Energy Impact", "12 hr Power", "Preventing Sleep", "Threads", "PID"]);
    }

    #[test]
    fn windowed_gpu_and_selected_filters() {
        let mut gpu_proc = proc(3, "mpv", 1000, 1.0, 1);
        gpu_proc.gpu = 12.0;
        let procs = vec![proc(1, "systemd", 0, 0.0, 1), proc(2, "firefox", 1000, 2.0, 1), gpu_proc];
        let windowed: HashSet<u32> = [2].into_iter().collect();
        let pids = |f: Filter, pinned: &[u32]| visible(&procs, &scope(f, "", pid_sort(), &windowed, pinned)).iter().map(|p| p.pid).collect::<Vec<_>>();
        assert_eq!(pids(Filter::Windowed, &[]), vec![2]);
        assert_eq!(pids(Filter::Gpu, &[]), vec![3]);
        assert_eq!(pids(Filter::Selected, &[1, 3]), vec![1, 3]);
        assert_eq!(pids(Filter::Selected, &[]), Vec::<u32>::new());
    }

    #[test]
    fn deltas_count_from_the_baseline_and_new_processes_from_zero() {
        let mut p = proc(5, "x", 1000, 0.0, 1);
        p.cpu_ticks = 100;
        p.wakeups = 10;
        p.read_total = 1000;
        let base = Baseline::capture(&[p.clone()]);
        p.cpu_ticks = 350;
        p.wakeups = 25;
        p.read_total = 5000;
        assert_eq!(base.since(&p), Counters { ticks: 250, wakeups: 15, read: 4000, written: 0 });
        let newcomer = ProcSample { pid: 9, cpu_ticks: 40, ..ProcSample::default() };
        assert_eq!(base.since(&newcomer).ticks, 40);
        assert_eq!(cell_text(&p, Some(SortKey::CpuTime), Some(&base)), "0:02.50");
        assert_eq!(cell_text(&p, Some(SortKey::CpuTime), None), "0:03.50");
        assert_eq!(cell_text(&p, Some(SortKey::Wakeups), Some(&base)), "15");
    }

    #[test]
    fn sorting_by_a_counter_uses_the_deltas_when_they_are_on() {
        let mut a = proc(1, "a", 1000, 0.0, 1);
        let mut b = proc(2, "b", 1000, 0.0, 1);
        a.cpu_ticks = 1000;
        b.cpu_ticks = 500;
        let base = Baseline::capture(&[a.clone(), b.clone()]);
        a.cpu_ticks = 1010;
        b.cpu_ticks = 900;
        let windowed = HashSet::new();
        let mut sc = scope(Filter::All, "", Sort { key: SortKey::CpuTime, descending: true }, &windowed, &[]);
        assert_eq!(visible(&[a.clone(), b.clone()], &sc)[0].pid, 1);
        sc.baseline = Some(&base);
        assert_eq!(visible(&[a, b], &sc)[0].pid, 2);
    }

    #[test]
    fn cpu_time_and_states_read_as_words() {
        assert_eq!(cpu_time(0), "0:00.00");
        assert_eq!(cpu_time(6_123), "1:01.23");
        assert_eq!(cpu_time(360_005), "1:00:00.05");
        assert_eq!(state_name('R'), "Running");
        assert_eq!(state_name('Z'), "Zombie");
        assert_eq!(state_name('?'), "Unknown");
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            cpu: Default::default(),
            memory: Default::default(),
            disk_read_bps: 0.0,
            disk_write_bps: 0.0,
            ifaces: Vec::new(),
            procs: Vec::new(),
            cores: Vec::new(),
            pressure: None,
            gpu_busy: 0.0,
        }
    }

    #[test]
    fn history_keeps_one_series_per_core() {
        let mut h = History::default();
        let snap = Snapshot { cores: vec![10.0, 90.0], pressure: Some(3.0), gpu_busy: 40.0, ..snapshot() };
        h.record(&snap);
        h.record(&snap);
        assert_eq!(h.cores.len(), 2);
        assert_eq!(h.cores[1].last(), 90.0);
        assert_eq!((h.gpu.last(), h.pressure.last(), h.cores[0].len()), (40.0, 3.0, 2));
    }

    #[test]
    fn the_dock_tile_shows_the_chosen_reading() {
        let mut snap = Snapshot {
            cpu: crate::sample::CpuShare { busy: 42.0, ..Default::default() },
            memory: crate::sample::Memory { total: 1000, available: 250, ..Default::default() },
            disk_read_bps: 2.0 * 1024.0 * 1024.0,
            ..snapshot()
        };
        assert_eq!(DockIcon::Application.tile(&snap), (String::new(), None));
        assert_eq!(DockIcon::Cpu.tile(&snap).0, "42%");
        assert_eq!(DockIcon::Memory.tile(&snap).0, "75%");
        assert_eq!(DockIcon::Disk.tile(&snap).0, "2.0M");
        snap.disk_read_bps = 0.0;
        assert_eq!(DockIcon::Network.tile(&snap).0, "0.0M");
        assert_eq!(DockIcon::from_index(DockIcon::Disk.index()), DockIcon::Disk);
        assert_eq!(DockIcon::from_index(-1), DockIcon::Application);
    }

    #[test]
    fn cpu_sort_is_largest_first_and_ties_keep_pid_order() {
        let procs = vec![proc(30, "a", 1000, 5.0, 1), proc(10, "b", 1000, 9.0, 1), proc(20, "c", 1000, 5.0, 1)];
        let none = HashSet::new();
        let rows = visible(&procs, &scope(Filter::All, "", Tab::Cpu.default_sort(), &none, &[]));
        let pids: Vec<u32> = rows.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![10, 20, 30]);
    }

    #[test]
    fn filters_and_search_apply_before_sorting() {
        let procs = vec![proc(1, "systemd", 0, 1.0, 1), proc(2, "firefox", 1000, 2.0, 1), proc(3, "Fire", 1000, 0.5, 1)];
        let none = HashSet::new();
        let mine = visible(&procs, &scope(Filter::Mine, "", Sort { key: SortKey::Name, descending: false }, &none, &[]));
        assert_eq!(mine.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![3, 2]);
        let others = visible(&procs, &scope(Filter::Others, "", pid_sort(), &none, &[]));
        assert_eq!(others.len(), 1);
        let found = visible(&procs, &scope(Filter::All, "fire", pid_sort(), &none, &[]));
        assert_eq!(found.len(), 2);
        assert!(matches(&procs[0], "root"));
    }

    #[test]
    fn system_active_and_inactive_filters() {
        let procs = vec![proc(1, "systemd", 0, 0.0, 1), proc(2, "firefox", 1000, 2.0, 1)];
        let none = HashSet::new();
        let by = |f: Filter| visible(&procs, &scope(f, "", pid_sort(), &none, &[]));
        assert_eq!(by(Filter::System).len(), 1);
        assert_eq!(by(Filter::Active)[0].pid, 2);
        assert_eq!(by(Filter::Inactive)[0].pid, 1);
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
        assert_eq!(row_text(&p, Tab::Cpu), "sh\tolibuijr\t12.5%\t1\t7");
        assert_eq!(row_text(&p, Tab::Memory), "sh\tolibuijr\t2.0 KB\t1\t7");
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
            let cols = tab.columns();
            assert_eq!(cols.iter().filter(|c| c.width == 0.0).count(), 1);
            if tab != Tab::Network {
                assert!(cols.iter().any(|c| c.key == Some(tab.default_sort().key)));
            }
        }
    }
}
