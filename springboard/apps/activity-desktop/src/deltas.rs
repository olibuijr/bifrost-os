//! Activity Monitor extras: Show Deltas for Process (counters relative to a stored baseline) and Sample Process
//! (a text report built from repeated reads of a process's kernel stack and wait channel). Pure std; the window
//! reads `/proc` and feeds the numbers in.
#![allow(dead_code)] // the Show Deltas toggle and Add Process > Sample Process wire these in next

use std::collections::BTreeMap;

/// A per-process counter baseline: the value each pid had when deltas were switched on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    values: BTreeMap<u32, u64>,
}

impl Baseline {
    /// Records the current counters (`pid`, value) as the baseline.
    pub fn capture(counters: &[(u32, u64)]) -> Baseline {
        Baseline { values: counters.iter().copied().collect() }
    }

    /// The change since the baseline. A process that started later has its whole value as the change; a counter
    /// that went backwards (pid reuse) counts from zero.
    pub fn delta(&self, pid: u32, now: u64) -> u64 {
        match self.values.get(&pid) {
            Some(base) if now >= *base => now - base,
            _ => now,
        }
    }

    /// Forgets processes that are gone.
    pub fn prune(&mut self, live: &[u32]) {
        self.values.retain(|pid, _| live.contains(pid));
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

/// One read of a process: the symbol the main thread waits in (`wchan`) and its kernel stack lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub wchan: String,
    pub stack: Vec<String>,
}

/// Reads `/proc/<pid>/stack` text: lines like `[<0>] futex_wait+0x12/0x30`. Returns the symbols, top first.
pub fn parse_stack(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let sym = l.trim().strip_prefix("[<0>]").unwrap_or(l.trim()).trim();
            let name = sym.split(['+', ' ']).next().unwrap_or("");
            (!name.is_empty() && name != "0").then(|| name.to_string())
        })
        .collect()
}

/// A sampled call: how many probes saw it at the top of the stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub symbol: String,
    pub count: usize,
}

/// Summarises probes: the `wchan` values and top-of-stack symbols with how often each was seen, most first.
pub fn summarize(probes: &[Probe]) -> Vec<Hit> {
    let mut counts: Vec<Hit> = vec![];
    for p in probes {
        let top = p.stack.first().cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            if p.wchan.is_empty() || p.wchan == "0" { "running".to_string() } else { p.wchan.clone() }
        });
        match counts.iter_mut().find(|h| h.symbol == top) {
            Some(h) => h.count += 1,
            None => counts.push(Hit { symbol: top, count: 1 }),
        }
    }
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.symbol.cmp(&b.symbol)));
    counts
}

/// The report text saved by Sample Process.
pub fn report(name: &str, pid: u32, seconds: u32, interval_ms: u32, probes: &[Probe]) -> String {
    let mut out = format!("Sampling {name} (pid {pid}) for {seconds} seconds, every {interval_ms} ms, {} samples\n\n", probes.len());
    for h in summarize(probes) {
        let pct = if probes.is_empty() { 0.0 } else { 100.0 * h.count as f32 / probes.len() as f32 };
        out.push_str(&format!("{:>5}  {:>5.1}%  {}\n", h.count, pct, h.symbol));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_against_a_baseline() {
        let b = Baseline::capture(&[(1, 100), (2, 50)]);
        assert_eq!(b.delta(1, 130), 30);
        assert_eq!(b.delta(2, 10), 10); // counter went backwards
        assert_eq!(b.delta(3, 7), 7); // new process
        let mut b = b;
        b.prune(&[1]);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn stack_lines() {
        let text = "[<0>] futex_wait+0x12/0x30\n[<0>] do_syscall_64+0x5a/0x90\n[<0>] 0\n";
        assert_eq!(parse_stack(text), vec!["futex_wait", "do_syscall_64"]);
        assert!(parse_stack("").is_empty());
    }

    #[test]
    fn summary_orders_by_count() {
        let a = Probe { wchan: "x".into(), stack: vec!["futex_wait".into()] };
        let b = Probe { wchan: "do_poll".into(), stack: vec![] };
        let c = Probe { wchan: "0".into(), stack: vec![] };
        let hits = summarize(&[a.clone(), b.clone(), a.clone(), c]);
        assert_eq!(hits[0], Hit { symbol: "futex_wait".into(), count: 2 });
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[1].symbol, "do_poll");
        assert_eq!(hits[2].symbol, "running");
    }

    #[test]
    fn report_lists_percentages() {
        let p = Probe { wchan: "".into(), stack: vec!["epoll_wait".into()] };
        let text = report("shell", 42, 3, 10, &[p.clone(), p]);
        assert!(text.starts_with("Sampling shell (pid 42) for 3 seconds, every 10 ms, 2 samples"));
        assert!(text.contains("100.0%  epoll_wait"));
    }
}
