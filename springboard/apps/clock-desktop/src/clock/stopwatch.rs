//! Stopwatch on the monotonic frame clock: start/stop, laps, reset and
//! the bifrost's "MM:SS,hh" formatting.

#[derive(Debug, Default, PartialEq)]
pub struct Stopwatch {
    /// ms accumulated before the current run
    banked: f64,
    /// when the current run started
    running_since: Option<f64>,
    /// total elapsed ms at each lap press
    lap_marks: Vec<f64>,
}

/// One row of the lap table: (lap number, split ms, total ms), newest first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lap {
    pub number: usize,
    pub split: f64,
    pub total: f64,
}

impl Stopwatch {
    pub fn running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Total elapsed ms at `t`.
    pub fn elapsed(&self, t: f64) -> f64 {
        self.banked + self.running_since.map_or(0.0, |s| (t - s).max(0.0))
    }

    /// True once started, until reset.
    pub fn started(&self, t: f64) -> bool {
        self.running() || self.elapsed(t) > 0.0
    }

    pub fn toggle(&mut self, t: f64) {
        match self.running_since.take() {
            Some(s) => self.banked += (t - s).max(0.0),
            None => self.running_since = Some(t),
        }
    }

    /// Lap while running, reset while stopped: the left button on the bifrost.
    pub fn lap_or_reset(&mut self, t: f64) {
        if self.running() {
            self.lap_marks.push(self.elapsed(t));
        } else {
            *self = Stopwatch::default();
        }
    }

    /// Finished laps plus the lap in progress, newest first.
    pub fn laps(&self, t: f64) -> Vec<Lap> {
        if !self.started(t) {
            return vec![];
        }
        let mut marks = self.lap_marks.clone();
        marks.push(self.elapsed(t));
        let mut prev = 0.0;
        let mut laps: Vec<Lap> = marks
            .iter()
            .enumerate()
            .map(|(i, &total)| {
                let lap = Lap { number: i + 1, split: total - prev, total };
                prev = total;
                lap
            })
            .collect();
        laps.reverse();
        laps
    }

    /// The lap table as plain text for the pasteboard: a header line, then one line per lap, oldest first, with the
    /// lap number, the split and the total as `format` writes them (tab separated). `None` before the watch has started.
    pub fn export_text(&self, t: f64) -> Option<String> {
        let mut laps = self.laps(t);
        if laps.is_empty() {
            return None;
        }
        laps.reverse();
        let mut out = String::from("Lap\tSplit\tTotal\n");
        for lap in laps {
            out += &format!("{}\t{}\t{}\n", lap.number, format(lap.split), format(lap.total));
        }
        Some(out)
    }
}

/// The (best, worst) lap numbers by split time among the finished laps
/// (`laps` newest first as `Stopwatch::laps` returns them; while `running`
/// the newest is still in progress and not counted). None below two
/// finished laps or when every split is equal.
pub fn extremes(laps: &[Lap], running: bool) -> Option<(usize, usize)> {
    let done: Vec<&Lap> = laps.iter().skip(running as usize).collect();
    if done.len() < 2 {
        return None;
    }
    let best = done.iter().min_by(|a, b| a.split.total_cmp(&b.split).then(a.number.cmp(&b.number)))?;
    let worst = done.iter().max_by(|a, b| a.split.total_cmp(&b.split).then(b.number.cmp(&a.number)))?;
    (best.split != worst.split).then_some((best.number, worst.number))
}

impl Stopwatch {
    /// The persisted form at monotonic `t` / Unix ms `now_ms`: time banked,
    /// the absolute start of the run in progress and the lap marks, so laps
    /// and a running watch survive leaving the app and a shell restart.
    pub fn to_text(&self, t: f64, now_ms: i64) -> String {
        let mut out = format!("# SpringBoard Clock stopwatch\nbanked={}\n", self.banked.round() as i64);
        if let Some(s) = self.running_since {
            out += &format!("since={}\n", now_ms - (t - s).max(0.0).round() as i64);
        }
        let marks: Vec<String> = self.lap_marks.iter().map(|m| (m.round() as i64).to_string()).collect();
        out += &format!("laps={}\n", marks.join(","));
        out
    }

    /// Restores `to_text` at monotonic `t` / Unix ms `now_ms`; a running
    /// watch kept counting while the shell was down.
    pub fn from_text(text: &str, t: f64, now_ms: i64) -> Stopwatch {
        let mut sw = Stopwatch::default();
        for (key, value) in springboard::shell::clock::store::pairs(text) {
            match key {
                "banked" => sw.banked = value.parse::<i64>().map_or(0.0, |v| v.max(0) as f64),
                "since" => {
                    if let Ok(since) = value.parse::<i64>() {
                        sw.running_since = Some(t - (now_ms - since).max(0) as f64);
                    }
                }
                "laps" => sw.lap_marks = value.split(',').filter_map(|m| m.parse::<i64>().ok()).map(|m| m as f64).collect(),
                _ => {}
            }
        }
        // marks must not run backwards or past the elapsed total
        let total = sw.elapsed(t);
        let mut prev = 0.0;
        sw.lap_marks.retain(|&m| {
            let ok = m >= prev && m <= total;
            if ok {
                prev = m;
            }
            ok
        });
        sw
    }
}

/// "MM:SS,hh", growing to "H:MM:SS,hh" past an hour.
pub fn format(ms: f64) -> String {
    let cs = (ms / 10.0).floor() as i64;
    let (h, m, s, c) = (cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100);
    if h > 0 {
        format!("{h}:{m:02}:{s:02},{c:02}")
    } else {
        format!("{m:02}:{s:02},{c:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_while_running() {
        let mut sw = Stopwatch::default();
        assert!(!sw.started(0.0));
        assert_eq!(sw.elapsed(500.0), 0.0);
        sw.toggle(1000.0);
        assert!(sw.running());
        assert_eq!(sw.elapsed(1500.0), 500.0);
        sw.toggle(2000.0);
        assert_eq!(sw.elapsed(9000.0), 1000.0, "frozen while stopped");
        assert!(sw.started(9000.0));
        sw.toggle(10_000.0);
        assert_eq!(sw.elapsed(10_250.0), 1250.0);
    }

    #[test]
    fn laps_split_the_total_and_reset_clears() {
        let mut sw = Stopwatch::default();
        assert!(sw.laps(0.0).is_empty());
        sw.toggle(0.0);
        sw.lap_or_reset(3610.0);
        let laps = sw.laps(5040.0);
        assert_eq!(laps, vec![Lap { number: 2, split: 1430.0, total: 5040.0 }, Lap { number: 1, split: 3610.0, total: 3610.0 }]);
        sw.toggle(5040.0);
        sw.lap_or_reset(6000.0);
        assert_eq!(sw, Stopwatch::default());
        assert!(sw.laps(6000.0).is_empty());
    }

    #[test]
    fn export_lists_laps_oldest_first_under_a_header() {
        let mut sw = Stopwatch::default();
        assert_eq!(sw.export_text(0.0), None, "nothing to copy before a start");
        sw.toggle(0.0);
        sw.lap_or_reset(3610.0);
        // lap 1 ends at 3610 ms (00:03,61); lap 2 is 1430 ms (00:01,43) and the total is 5040 ms (00:05,04)
        assert_eq!(sw.export_text(5040.0).unwrap(), "Lap\tSplit\tTotal\n1\t00:03,61\t00:03,61\n2\t00:01,43\t00:05,04\n");
    }

    #[test]
    fn best_and_worst_laps_need_two_finished_laps() {
        let lap = |number, split| Lap { number, split, total: 0.0 };
        assert_eq!(extremes(&[], false), None);
        assert_eq!(extremes(&[lap(2, 1.0), lap(1, 5.0)], true), None, "one finished lap while running");
        assert_eq!(extremes(&[lap(3, 9.0), lap(2, 1.0), lap(1, 5.0)], true), Some((2, 1)), "the lap in progress is ignored");
        assert_eq!(extremes(&[lap(3, 9.0), lap(2, 1.0), lap(1, 5.0)], false), Some((2, 3)), "stopped: every lap counts");
        assert_eq!(extremes(&[lap(2, 4.0), lap(1, 4.0)], false), None, "all equal");
        assert_eq!(extremes(&[lap(3, 2.0), lap(2, 7.0), lap(1, 2.0)], false), Some((1, 2)), "ties go to the earlier lap");
    }

    #[test]
    fn laps_persist_while_stopped_and_a_running_watch_keeps_counting() {
        const NOW: i64 = 1_790_000_000_000;
        let mut sw = Stopwatch::default();
        sw.toggle(1000.0);
        sw.lap_or_reset(3610.0);
        sw.toggle(5000.0);
        sw.toggle(8000.0);
        let text = sw.to_text(9000.0, NOW);
        assert!(text.contains("banked=4000\n") && text.contains("laps=2610\n"), "{text}");
        assert!(text.contains(&format!("since={}\n", NOW - 1000)));
        // restart 10 s later, monotonic clock back at 0: still running, 4 s banked + 11 s
        let back = Stopwatch::from_text(&text, 0.0, NOW + 10_000);
        assert!(back.running());
        assert_eq!(back.elapsed(0.0), 15_000.0);
        assert_eq!(back.laps(0.0).len(), 2);
        // stopped watch: nothing changes with the time away
        sw.toggle(10_000.0);
        let stopped = Stopwatch::from_text(&sw.to_text(10_000.0, NOW), 0.0, NOW + 99_000);
        assert!(!stopped.running());
        assert_eq!(stopped.elapsed(500.0), sw.elapsed(10_000.0));
        assert_eq!(stopped.laps(0.0), sw.laps(10_000.0));
        assert_eq!(Stopwatch::from_text("", 0.0, NOW), Stopwatch::default());
        let junk = Stopwatch::from_text("banked=1000\nlaps=500,200,x,9999\nsince=oops\n", 0.0, NOW);
        assert_eq!(junk.laps(0.0).len(), 2, "backward and overlong marks are dropped");
    }

    #[test]
    fn formats_like_the_bifrost() {
        assert_eq!(format(0.0), "00:00,00");
        assert_eq!(format(5040.0), "00:05,04");
        assert_eq!(format(61_999.0), "01:01,99");
        assert_eq!(format(3_600_000.0 + 1500.0), "1:00:01,50");
        assert_eq!(format(9.9), "00:00,00", "truncates, never rounds up");
    }
}
