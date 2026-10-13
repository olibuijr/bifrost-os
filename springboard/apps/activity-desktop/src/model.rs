//! What the window shows: the tabs and their columns, the process filter, sort and search, the graph history and the
//! number formats. Plain data with no drawing; `window.rs` holds the state and `draw.rs` paints it.

use crate::sample::{ProcSample, Snapshot};
use std::cmp::Ordering;
use std::collections::VecDeque;

/// Samples kept for each graph (one per second, so one minute).
pub const HISTORY: usize = 60;

/// The five tabs above the graph. Each picks the graph and the table's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Cpu, Tab::Memory, Tab::Energy, Tab::Disk, Tab::Network];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
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
            Tab::Memory => process_head(Column::new(Some(SortKey::Memory), "Memory", 88.0)),
            Tab::Energy => process_head(Column::new(Some(SortKey::Energy), "Energy Impact", 112.0)),
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
            ],
        }
    }
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

/// The sidebar filter: every process, the signed-in user's own, or the processes of other users.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Mine,
    Others,
}

impl Filter {
    pub fn keep(self, p: &ProcSample, own_uid: u32) -> bool {
        match self {
            Filter::All => true,
            Filter::Mine => p.uid == own_uid,
            Filter::Others => p.uid != own_uid,
        }
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
pub fn visible(procs: &[ProcSample], filter: Filter, own_uid: u32, query: &str, sort: Sort) -> Vec<ProcSample> {
    let mut rows: Vec<ProcSample> = procs.iter().filter(|p| filter.keep(p, own_uid) && matches(p, query)).cloned().collect();
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
        ProcSample { pid, name: name.into(), user: if uid == 0 { "root".into() } else { "olibuijr".into() }, uid, cpu, rss, threads: 1, read_bps: 0.0, write_bps: 0.0 }
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
