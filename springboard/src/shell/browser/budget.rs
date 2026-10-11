//! Memory accounting and the discard policy for browser tabs.
//!
//! Low memory use is the browser's top requirement. This module reads the
//! process's own footprint (`/proc/self/status`), the system's headroom
//! (`/proc/meminfo`, `/proc/pressure/memory`), and decides which background
//! tabs to shrink, and how far, when the budget is exceeded. It never touches
//! the visible tab. Everything except the `read*` helpers is pure and tested
//! on the host.

/// Identifies a tab for the policy (the browser's own tab handle maps onto it).
pub type TabId = u64;

/// Approximate heap cost of one DOM node (node + attrs + text), bytes.
pub const DOM_NODE_BYTES: usize = 256;
/// Bytes a discarded tab still holds (url, title, scroll, form state).
pub const DISCARDED_FLOOR_BYTES: usize = 2 * 1024;
/// Background tabs used more recently than this are spared unless at hard pressure.
pub const RECENT_GRACE_MS: u64 = 5_000;

// ---------------------------------------------------------------- MemStats

/// Process and system memory figures, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemStats {
    /// Resident set size now (VmRSS).
    pub rss: u64,
    /// Peak resident set size (VmHWM).
    pub hwm: u64,
    /// System MemAvailable, if known.
    pub mem_available: Option<u64>,
}

fn kb_field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        let n: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(n * 1024)
    })
}

/// Parses `/proc/self/status` text into (VmRSS, VmHWM) bytes.
pub fn parse_status(text: &str) -> (u64, u64) {
    (kb_field(text, "VmRSS").unwrap_or(0), kb_field(text, "VmHWM").unwrap_or(0))
}

/// Parses `/proc/meminfo` text into MemAvailable bytes.
pub fn parse_meminfo_available(text: &str) -> Option<u64> {
    kb_field(text, "MemAvailable")
}

impl MemStats {
    /// Builds stats from the text of the two proc files.
    pub fn parse(status: &str, meminfo: &str) -> MemStats {
        let (rss, hwm) = parse_status(status);
        MemStats { rss, hwm, mem_available: parse_meminfo_available(meminfo) }
    }

    /// Reads the live figures; missing files give zeros / None.
    pub fn read() -> MemStats {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        MemStats::parse(&status, &meminfo)
    }
}

// --------------------------------------------------------------------- PSI

/// One `/proc/pressure/memory` line's averages (percent of time stalled).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PsiLine {
    pub avg10: f32,
    pub avg60: f32,
    pub avg300: f32,
    pub total_us: u64,
}

/// Parsed `/proc/pressure/memory`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Psi {
    pub some: PsiLine,
    pub full: PsiLine,
}

fn psi_line(rest: &str) -> PsiLine {
    let mut l = PsiLine::default();
    for kv in rest.split_whitespace() {
        let Some((k, v)) = kv.split_once('=') else { continue };
        match k {
            "avg10" => l.avg10 = v.parse().unwrap_or(0.0),
            "avg60" => l.avg60 = v.parse().unwrap_or(0.0),
            "avg300" => l.avg300 = v.parse().unwrap_or(0.0),
            "total" => l.total_us = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    l
}

/// Parses PSI text; None when there is no `some` line (kernel without PSI).
pub fn parse_psi(text: &str) -> Option<Psi> {
    let mut psi = Psi::default();
    let mut seen = false;
    for line in text.lines() {
        if let Some(r) = line.strip_prefix("some ") {
            psi.some = psi_line(r);
            seen = true;
        } else if let Some(r) = line.strip_prefix("full ") {
            psi.full = psi_line(r);
        }
    }
    seen.then_some(psi)
}

/// Reads `/proc/pressure/memory` (absent on kernels without PSI).
pub fn read_psi() -> Option<Psi> {
    parse_psi(&std::fs::read_to_string("/proc/pressure/memory").ok()?)
}

// ------------------------------------------------------------ budget/policy

/// Byte budgets for the whole browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemBudget {
    /// Above this the browser starts shrinking background tabs.
    pub soft_limit: usize,
    /// Above this it discards aggressively.
    pub hard_limit: usize,
    /// No single background tab should keep more than this.
    pub per_tab_cap: usize,
}

impl Default for MemBudget {
    fn default() -> Self {
        MemBudget { soft_limit: 160 << 20, hard_limit: 256 << 20, per_tab_cap: 48 << 20 }
    }
}

/// System-level MemAvailable floors (Android low-memory hook thresholds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvailThresholds {
    pub soft_below: u64,
    pub hard_below: u64,
}

impl Default for AvailThresholds {
    fn default() -> Self {
        AvailThresholds { soft_below: 400 << 20, hard_below: 200 << 20 }
    }
}

/// How urgently memory must be given back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    Normal,
    Soft,
    Hard,
}

impl Pressure {
    /// Combines own RSS, system MemAvailable and PSI into one level.
    pub fn assess(stats: &MemStats, psi: Option<&Psi>, budget: &MemBudget, avail: &AvailThresholds) -> Pressure {
        let rss = stats.rss as usize;
        let mut p = Pressure::Normal;
        if rss >= budget.hard_limit {
            p = Pressure::Hard;
        } else if rss >= budget.soft_limit {
            p = Pressure::Soft;
        }
        if let Some(a) = stats.mem_available {
            if a < avail.hard_below {
                p = p.max(Pressure::Hard);
            } else if a < avail.soft_below {
                p = p.max(Pressure::Soft);
            }
        }
        if let Some(psi) = psi {
            if psi.full.avg10 >= 5.0 || psi.some.avg10 >= 40.0 {
                p = p.max(Pressure::Hard);
            } else if psi.some.avg10 >= 10.0 {
                p = p.max(Pressure::Soft);
            }
        }
        p
    }
}

/// How much of a background tab is thrown away; each level includes the ones before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiscardLevel {
    DropTiles,
    DropImages,
    DropLayoutAndStyle,
    DropJsContext,
    /// Keeps only url/title/scroll/form state (see `tabmem::DiscardState`).
    DropDom,
}

impl DiscardLevel {
    pub const ALL: [DiscardLevel; 5] = [
        DiscardLevel::DropTiles,
        DiscardLevel::DropImages,
        DiscardLevel::DropLayoutAndStyle,
        DiscardLevel::DropJsContext,
        DiscardLevel::DropDom,
    ];
}

/// Per-tab byte estimates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabMem {
    pub id: TabId,
    pub visible: bool,
    /// Last user interaction, milliseconds on any monotonic clock shared with `now`.
    pub last_used_ms: u64,
    pub dom_nodes: usize,
    pub style_bytes: usize,
    pub layout_bytes: usize,
    pub tile_bytes: usize,
    pub image_bytes: usize,
    pub js_bytes: usize,
    /// Deepest discard already applied, if any.
    pub level: Option<DiscardLevel>,
}

impl TabMem {
    pub fn dom_bytes(&self) -> usize {
        self.dom_nodes * DOM_NODE_BYTES
    }

    pub fn total(&self) -> usize {
        self.dom_bytes() + self.style_bytes + self.layout_bytes + self.tile_bytes + self.image_bytes + self.js_bytes
    }

    /// Bytes released by discarding down to `level` (cumulative).
    pub fn freed_by(&self, level: DiscardLevel) -> usize {
        let mut n = self.tile_bytes;
        if level >= DiscardLevel::DropImages {
            n += self.image_bytes;
        }
        if level >= DiscardLevel::DropLayoutAndStyle {
            n += self.layout_bytes + self.style_bytes;
        }
        if level >= DiscardLevel::DropJsContext {
            n += self.js_bytes;
        }
        if level >= DiscardLevel::DropDom {
            n += self.dom_bytes().saturating_sub(DISCARDED_FLOOR_BYTES);
        }
        n
    }
}

/// Sum of all tabs' estimates.
pub fn total_bytes(tabs: &[TabMem]) -> usize {
    tabs.iter().map(TabMem::total).sum()
}

/// Chooses background tabs to shrink so the total fits `budget.soft_limit` and
/// no background tab exceeds `per_tab_cap`. Oldest-used tabs go first, larger
/// first among equals; shallow levels are exhausted across all candidates
/// before a deeper level is used. The visible tab is never touched. Tabs used
/// within `RECENT_GRACE_MS` of `now` are spared unless the total is over the
/// hard limit. Returns one entry per affected tab with its final level.
pub fn plan_discards(tabs: &[TabMem], budget: &MemBudget, now: u64) -> Vec<(TabId, DiscardLevel)> {
    let mut total = total_bytes(tabs);
    let hard = total > budget.hard_limit;
    let max_level = if hard { DiscardLevel::DropDom } else { DiscardLevel::DropJsContext };
    let mut cand: Vec<usize> = (0..tabs.len())
        .filter(|&i| !tabs[i].visible && (hard || now.saturating_sub(tabs[i].last_used_ms) >= RECENT_GRACE_MS))
        .collect();
    cand.sort_by(|&a, &b| tabs[a].last_used_ms.cmp(&tabs[b].last_used_ms).then(tabs[b].total().cmp(&tabs[a].total())));
    let mut level: Vec<Option<DiscardLevel>> = tabs.iter().map(|t| t.level).collect();
    let mut left: Vec<usize> = tabs.iter().map(TabMem::total).collect();
    let mut touched = vec![false; tabs.len()];
    let mut apply = |i: usize, lv: DiscardLevel, level: &mut Vec<Option<DiscardLevel>>, left: &mut Vec<usize>, total: &mut usize| {
        if level[i].is_some_and(|c| c >= lv) {
            return;
        }
        let after = tabs[i].total() - tabs[i].freed_by(lv);
        let freed = left[i].saturating_sub(after);
        if freed == 0 {
            return;
        }
        level[i] = Some(lv);
        left[i] -= freed;
        *total -= freed;
        touched[i] = true;
    };
    // Per-tab cap: escalate each oversized background tab until it fits.
    for &i in &cand {
        for lv in DiscardLevel::ALL {
            if lv > max_level || left[i] <= budget.per_tab_cap {
                break;
            }
            apply(i, lv, &mut level, &mut left, &mut total);
        }
    }
    // Global target: shallow levels first, LRU order within a level.
    'outer: for lv in DiscardLevel::ALL {
        if lv > max_level {
            break;
        }
        for &i in &cand {
            if total <= budget.soft_limit {
                break 'outer;
            }
            apply(i, lv, &mut level, &mut left, &mut total);
        }
    }
    cand.iter().filter(|&&i| touched[i]).filter_map(|&i| level[i].map(|l| (tabs[i].id, l))).collect()
}

/// Like `plan_discards`, but also acts under system pressure even when the
/// browser's own total is within budget: Soft squeezes the soft limit to 3/4,
/// Hard to 1/2.
pub fn plan_for_pressure(pressure: Pressure, tabs: &[TabMem], budget: &MemBudget, now: u64) -> Vec<(TabId, DiscardLevel)> {
    let quarters = match pressure {
        Pressure::Normal => return plan_discards(tabs, budget, now),
        Pressure::Soft => 3,
        Pressure::Hard => 2,
    };
    let mut b = *budget;
    b.soft_limit = budget.soft_limit * quarters / 4;
    if pressure == Pressure::Hard {
        b.hard_limit = b.hard_limit.min(total_bytes(tabs).saturating_sub(1));
    }
    plan_discards(tabs, &b, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tspringboard\nVmPeak:\t  900000 kB\nVmHWM:\t  123456 kB\nVmRSS:\t   65432 kB\nThreads:\t4\n";
    const MEMINFO: &str = "MemTotal:        3800000 kB\nMemFree:          100000 kB\nMemAvailable:    1500000 kB\n";

    #[test]
    fn parses_status() {
        assert_eq!(parse_status(STATUS), (65432 * 1024, 123456 * 1024));
        assert_eq!(parse_status("nothing"), (0, 0));
    }

    #[test]
    fn parses_meminfo() {
        assert_eq!(parse_meminfo_available(MEMINFO), Some(1_500_000 * 1024));
        assert_eq!(parse_meminfo_available("MemFree: 1 kB\n"), None);
        let s = MemStats::parse(STATUS, MEMINFO);
        assert_eq!((s.rss, s.hwm, s.mem_available), (65432 * 1024, 123456 * 1024, Some(1_500_000 * 1024)));
    }

    #[test]
    fn parses_psi() {
        let t = "some avg10=12.50 avg60=3.00 avg300=1.25 total=987654\nfull avg10=6.00 avg60=1.00 avg300=0.50 total=1234\n";
        let p = parse_psi(t).unwrap();
        assert_eq!(p.some.avg10, 12.5);
        assert_eq!(p.some.total_us, 987654);
        assert_eq!(p.full.avg10, 6.0);
        assert!(parse_psi("").is_none());
        assert!(parse_psi("garbage").is_none());
    }

    fn psi(some: f32, full: f32) -> Psi {
        Psi { some: PsiLine { avg10: some, ..Default::default() }, full: PsiLine { avg10: full, ..Default::default() } }
    }

    #[test]
    fn pressure_levels() {
        let b = MemBudget { soft_limit: 100, hard_limit: 200, per_tab_cap: 50 };
        let a = AvailThresholds { soft_below: 1000, hard_below: 500 };
        let st = |rss, av| MemStats { rss, hwm: rss, mem_available: av };
        assert_eq!(Pressure::assess(&st(10, Some(5000)), None, &b, &a), Pressure::Normal);
        assert_eq!(Pressure::assess(&st(100, None), None, &b, &a), Pressure::Soft);
        assert_eq!(Pressure::assess(&st(200, None), None, &b, &a), Pressure::Hard);
        assert_eq!(Pressure::assess(&st(10, Some(900)), None, &b, &a), Pressure::Soft);
        assert_eq!(Pressure::assess(&st(10, Some(400)), None, &b, &a), Pressure::Hard);
        assert_eq!(Pressure::assess(&st(10, None), Some(&psi(15.0, 0.0)), &b, &a), Pressure::Soft);
        assert_eq!(Pressure::assess(&st(10, None), Some(&psi(1.0, 7.0)), &b, &a), Pressure::Hard);
        assert_eq!(Pressure::assess(&st(10, None), Some(&psi(1.0, 0.0)), &b, &a), Pressure::Normal);
    }

    fn tab(id: TabId, last: u64, mb: usize) -> TabMem {
        TabMem { id, last_used_ms: last, tile_bytes: mb << 20, image_bytes: mb << 20, layout_bytes: mb << 20, js_bytes: mb << 20, ..Default::default() }
    }

    #[test]
    fn freed_by_is_cumulative() {
        let t = TabMem { dom_nodes: 10_000, style_bytes: 10, layout_bytes: 20, tile_bytes: 1, image_bytes: 2, js_bytes: 100, ..Default::default() };
        assert_eq!(t.freed_by(DiscardLevel::DropTiles), 1);
        assert_eq!(t.freed_by(DiscardLevel::DropImages), 3);
        assert_eq!(t.freed_by(DiscardLevel::DropLayoutAndStyle), 33);
        assert_eq!(t.freed_by(DiscardLevel::DropJsContext), 133);
        assert_eq!(t.freed_by(DiscardLevel::DropDom), 133 + 10_000 * DOM_NODE_BYTES - DISCARDED_FLOOR_BYTES);
    }

    #[test]
    fn nothing_to_do_within_budget() {
        let b = MemBudget { soft_limit: 100 << 20, hard_limit: 200 << 20, per_tab_cap: 50 << 20 };
        let tabs = [tab(1, 0, 2), tab(2, 0, 2)];
        assert!(plan_discards(&tabs, &b, 100_000).is_empty());
    }

    #[test]
    fn never_discards_visible_tab() {
        let b = MemBudget { soft_limit: 1 << 20, hard_limit: 2 << 20, per_tab_cap: 1 << 20 };
        let mut v = tab(1, 0, 50);
        v.visible = true;
        assert!(plan_discards(&[v], &b, 1_000_000).is_empty());
        let plan = plan_discards(&[v, tab(2, 0, 10)], &b, 1_000_000);
        assert!(plan.iter().all(|(id, _)| *id == 2));
    }

    #[test]
    fn lru_first_with_shallowest_level() {
        // Two idle tabs of 40 MB each; soft limit 70 MB: dropping the oldest tab's tiles (10 MB) suffices.
        let b = MemBudget { soft_limit: 70 << 20, hard_limit: 300 << 20, per_tab_cap: 100 << 20 };
        let tabs = [tab(1, 5_000, 10), tab(2, 1_000, 10)];
        let plan = plan_discards(&tabs, &b, 100_000);
        assert_eq!(plan, vec![(2, DiscardLevel::DropTiles)]);
    }

    #[test]
    fn shallow_levels_spread_before_deeper() {
        // 3 tabs x 40 MB = 120 MB, target 80: tiles of all (30 MB) are not enough alone? 120-30=90 > 80, so images too.
        let b = MemBudget { soft_limit: 80 << 20, hard_limit: 300 << 20, per_tab_cap: 100 << 20 };
        let tabs = [tab(1, 1, 10), tab(2, 2, 10), tab(3, 3, 10)];
        let plan = plan_discards(&tabs, &b, 100_000);
        // After tiles of 1,2,3 (90) the images of the oldest (80) fit.
        assert_eq!(plan, vec![(1, DiscardLevel::DropImages), (2, DiscardLevel::DropTiles), (3, DiscardLevel::DropTiles)]);
    }

    #[test]
    fn per_tab_cap_forces_deeper_discard() {
        let b = MemBudget { soft_limit: 500 << 20, hard_limit: 900 << 20, per_tab_cap: 15 << 20 };
        let tabs = [tab(1, 0, 10)]; // 40 MB > cap 15 MB
        let plan = plan_discards(&tabs, &b, 100_000);
        // tiles 30, images 20, layout 10 (<=15) stops at layout/style
        assert_eq!(plan, vec![(1, DiscardLevel::DropLayoutAndStyle)]);
    }

    #[test]
    fn recent_tabs_spared_unless_hard() {
        let b = MemBudget { soft_limit: 10 << 20, hard_limit: 1000 << 20, per_tab_cap: 1000 << 20 };
        let tabs = [tab(1, 99_000, 10)];
        assert!(plan_discards(&tabs, &b, 100_000).is_empty());
        let b2 = MemBudget { hard_limit: 20 << 20, ..b };
        assert!(!plan_discards(&tabs, &b2, 100_000).is_empty());
    }

    #[test]
    fn hard_allows_dom_drop_soft_does_not() {
        let mut t = tab(1, 0, 0);
        t.dom_nodes = 200_000; // ~49 MB
        let soft = MemBudget { soft_limit: 1 << 20, hard_limit: 500 << 20, per_tab_cap: 500 << 20 };
        assert!(plan_discards(&[t], &soft, 100_000).is_empty());
        let hard = MemBudget { hard_limit: 10 << 20, ..soft };
        assert_eq!(plan_discards(&[t], &hard, 100_000), vec![(1, DiscardLevel::DropDom)]);
    }

    #[test]
    fn already_discarded_levels_are_not_repeated() {
        let b = MemBudget { soft_limit: 1, hard_limit: 1 << 30, per_tab_cap: 1 << 30 };
        let mut t = tab(1, 0, 10);
        t.level = Some(DiscardLevel::DropLayoutAndStyle);
        t.tile_bytes = 0;
        t.image_bytes = 0;
        t.layout_bytes = 0;
        let plan = plan_discards(&[t], &b, 100_000);
        assert_eq!(plan, vec![(1, DiscardLevel::DropJsContext)]);
    }

    #[test]
    fn pressure_plan_acts_below_limits() {
        let b = MemBudget { soft_limit: 100 << 20, hard_limit: 200 << 20, per_tab_cap: 100 << 20 };
        let tabs = [tab(1, 0, 10), tab(2, 0, 10)]; // 80 MB
        assert!(plan_for_pressure(Pressure::Normal, &tabs, &b, 100_000).is_empty());
        // 80 MB > 3/4 of 100 MB
        assert!(!plan_for_pressure(Pressure::Soft, &tabs, &b, 100_000).is_empty());
        assert!(!plan_for_pressure(Pressure::Hard, &tabs, &b, 100_000).is_empty());
    }
}
