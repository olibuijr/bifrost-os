//! Concurrent countdown timers. A running timer remembers its absolute end
//! (Unix ms) so it survives leaving the app and a restart of the shell, and
//! its monotonic deadline for smooth countdowns while the shell lives.
//! Persisted in clock-timers.ini next to the alarms.

use super::store;

/// The quick-start durations under the picker.
pub const PRESETS: [(&str, i64); 6] = [("1 min", 60_000), ("3 min", 180_000), ("5 min", 300_000), ("10 min", 600_000), ("15 min", 900_000), ("30 min", 1_800_000)];

#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Running {
        /// absolute end, Unix ms (persisted)
        end_ms: i64,
        /// the same moment on the monotonic frame clock
        mono_end: f64,
    },
    Paused {
        left_ms: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: u32,
    pub label: String,
    pub duration_ms: i64,
    pub run: Run,
}

impl Item {
    pub fn running(&self) -> bool {
        matches!(self.run, Run::Running { .. })
    }

    /// ms left at monotonic `t` (never negative).
    pub fn remaining(&self, t: f64) -> f64 {
        match self.run {
            Run::Running { mono_end, .. } => (mono_end - t).max(0.0),
            Run::Paused { left_ms } => left_ms as f64,
        }
    }

    /// Fraction of the duration still to go, for the progress bar.
    pub fn fraction(&self, t: f64) -> f32 {
        if self.duration_ms <= 0 {
            return 0.0;
        }
        (self.remaining(t) / self.duration_ms as f64).clamp(0.0, 1.0) as f32
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timers {
    pub list: Vec<Item>,
    last_id: u32,
}

impl Timers {
    /// "Timer", then "Timer 2", "Timer 3"... the first unused name.
    pub fn default_label(&self) -> String {
        let taken = |l: &str| self.list.iter().any(|i| i.label == l);
        if !taken("Timer") {
            return "Timer".into();
        }
        (2..).map(|n| format!("Timer {n}")).find(|l| !taken(l)).unwrap()
    }

    /// Starts a timer of `duration_ms` at monotonic `t` / Unix ms `now_ms`;
    /// None for a zero duration. An empty label gets the default name.
    pub fn start(&mut self, label: &str, duration_ms: i64, t: f64, now_ms: i64) -> Option<u32> {
        if duration_ms <= 0 {
            return None;
        }
        let label = match label.trim() {
            "" => self.default_label(),
            l => l.to_string(),
        };
        self.last_id += 1;
        self.list.push(Item { id: self.last_id, label, duration_ms, run: Run::Running { end_ms: now_ms + duration_ms, mono_end: t + duration_ms as f64 } });
        Some(self.last_id)
    }

    /// Pauses a running timer or resumes a paused one.
    pub fn toggle(&mut self, id: u32, t: f64, now_ms: i64) {
        let Some(item) = self.list.iter_mut().find(|i| i.id == id) else { return };
        item.run = match item.run {
            Run::Running { mono_end, .. } => Run::Paused { left_ms: (mono_end - t).max(0.0).ceil() as i64 },
            Run::Paused { left_ms } => Run::Running { end_ms: now_ms + left_ms, mono_end: t + left_ms as f64 },
        };
    }

    pub fn rename(&mut self, id: u32, label: &str) {
        if let Some(item) = self.list.iter_mut().find(|i| i.id == id) {
            item.label = if label.trim().is_empty() { "Timer".into() } else { label.trim().to_string() };
        }
    }

    /// Adds `ms` to a timer (the "+1 min" button), extending its duration if needed.
    pub fn add_time(&mut self, id: u32, ms: i64) {
        let Some(item) = self.list.iter_mut().find(|i| i.id == id) else { return };
        item.run = match item.run {
            Run::Running { end_ms, mono_end } => Run::Running { end_ms: end_ms + ms, mono_end: mono_end + ms as f64 },
            Run::Paused { left_ms } => Run::Paused { left_ms: (left_ms + ms).max(0) },
        };
        let left = match item.run {
            Run::Running { end_ms, .. } => end_ms,
            Run::Paused { left_ms } => left_ms,
        };
        let _ = left;
        item.duration_ms = item.duration_ms.max(item.duration_ms + ms.max(0));
    }

    /// Restarts a timer from its full duration, paused or running.
    pub fn restart(&mut self, id: u32, t: f64, now_ms: i64) {
        if let Some(item) = self.list.iter_mut().find(|i| i.id == id) {
            item.run = Run::Running { end_ms: now_ms + item.duration_ms, mono_end: t + item.duration_ms as f64 };
        }
    }

    pub fn remove(&mut self, id: u32) {
        self.list.retain(|i| i.id != id);
    }

    pub fn any_running(&self) -> bool {
        self.list.iter().any(Item::running)
    }

    /// The timers that ran out by `t`, removed from the list, in order of finishing.
    pub fn poll(&mut self, t: f64) -> Vec<Item> {
        let mut done: Vec<Item> = self.list.iter().filter(|i| i.running() && i.remaining(t) <= 0.0).cloned().collect();
        self.list.retain(|i| !done.iter().any(|d| d.id == i.id));
        let end = |i: &Item| if let Run::Running { mono_end, .. } = i.run { mono_end } else { 0.0 };
        done.sort_by(|a, b| end(a).total_cmp(&end(b)));
        done
    }

    /// Monotonic ms of the next timer to finish.
    pub fn next_due(&self) -> Option<f64> {
        self.list.iter().filter_map(|i| if let Run::Running { mono_end, .. } = i.run { Some(mono_end) } else { None }).min_by(f64::total_cmp)
    }

    /// Unix seconds of the next timer to finish, for the RTC wake-up.
    pub fn next_due_unix(&self) -> Option<i64> {
        self.list.iter().filter_map(|i| if let Run::Running { end_ms, .. } = i.run { Some((end_ms + 999).div_euclid(1000)) } else { None }).min()
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# SpringBoard Clock timers\n");
        for (n, i) in self.list.iter().enumerate() {
            out += &format!("timer.{n}.id={}\ntimer.{n}.label={}\ntimer.{n}.duration={}\n", i.id, i.label.replace(['\n', '\r'], " "), i.duration_ms);
            match i.run {
                Run::Running { end_ms, .. } => out += &format!("timer.{n}.end={end_ms}\n"),
                Run::Paused { left_ms } => out += &format!("timer.{n}.left={left_ms}\n"),
            }
        }
        out
    }

    /// Restores the store at monotonic `t` / Unix ms `now_ms`: a timer whose
    /// end passed while the shell was down comes back already finished, so
    /// the next `poll` reports it.
    pub fn parse(text: &str, t: f64, now_ms: i64) -> Self {
        #[derive(Default)]
        struct Raw {
            id: Option<u32>,
            label: Option<String>,
            duration: Option<i64>,
            end: Option<i64>,
            left: Option<i64>,
        }
        let mut raws: Vec<(usize, Raw)> = vec![];
        for (key, value) in store::pairs(text) {
            let mut parts = key.splitn(3, '.');
            if parts.next() != Some("timer") {
                continue;
            }
            let (Some(Ok(n)), Some(field)) = (parts.next().map(str::parse::<usize>), parts.next()) else { continue };
            let pos = match raws.iter().position(|(i, _)| *i == n) {
                Some(p) => p,
                None => {
                    raws.push((n, Raw::default()));
                    raws.len() - 1
                }
            };
            let raw = &mut raws[pos].1;
            match field {
                "id" => raw.id = value.parse().ok(),
                "label" => raw.label = Some(value.to_string()),
                "duration" => raw.duration = value.parse().ok(),
                "end" => raw.end = value.parse().ok(),
                "left" => raw.left = value.parse().ok(),
                _ => {}
            }
        }
        raws.sort_by_key(|(n, _)| *n);
        let mut timers = Timers::default();
        for (_, raw) in raws {
            let Some(duration_ms) = raw.duration.filter(|&d| d > 0) else { continue };
            let run = match (raw.end, raw.left) {
                (Some(end_ms), _) => Run::Running { end_ms, mono_end: t + (end_ms - now_ms) as f64 },
                (None, Some(left)) => Run::Paused { left_ms: left.clamp(0, duration_ms) },
                _ => continue,
            };
            let id = raw.id.filter(|id| !timers.list.iter().any(|i| i.id == *id)).unwrap_or(timers.last_id + 1);
            timers.last_id = timers.last_id.max(id);
            timers.list.push(Item { id, label: raw.label.filter(|l| !l.is_empty()).unwrap_or_else(|| "Timer".into()), duration_ms, run });
        }
        timers
    }
}

/// "H:MM:SS" or "MM:SS" for the countdown, rounding up so the display
/// never shows zero before the timer fires.
pub fn format(ms: f64) -> String {
    let s = (ms / 1000.0).ceil() as i64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// (hours, minutes, seconds) of a duration in ms, for the pickers.
pub fn split(ms: f64) -> (u32, u32, u32) {
    let s = (ms / 1000.0).round() as u32;
    (s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    #[test]
    fn counts_down_pauses_resumes_and_fires_once() {
        let mut tm = Timers::default();
        assert_eq!(tm.start("", 0, 0.0, NOW), None, "nothing to count");
        let id = tm.start("Tea", 10_000, 1000.0, NOW).unwrap();
        assert_eq!(tm.list[0].remaining(4000.0), 7000.0);
        assert_eq!(tm.next_due(), Some(11_000.0));
        assert_eq!(tm.next_due_unix(), Some((NOW + 10_000) / 1000));
        tm.toggle(id, 4000.0, NOW + 3000);
        assert_eq!(tm.list[0].run, Run::Paused { left_ms: 7000 });
        assert_eq!(tm.list[0].remaining(9000.0), 7000.0, "frozen while paused");
        assert_eq!(tm.next_due(), None);
        assert!(!tm.any_running());
        tm.toggle(id, 9000.0, NOW + 8000);
        assert_eq!(tm.list[0].run, Run::Running { end_ms: NOW + 15_000, mono_end: 16_000.0 });
        assert!((tm.list[0].fraction(12_000.0) - 0.4).abs() < 1e-6);
        assert!(tm.poll(15_999.0).is_empty());
        let done = tm.poll(16_000.0);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].label, "Tea");
        assert!(tm.poll(16_001.0).is_empty(), "fires once");
        assert!(tm.list.is_empty());
    }

    #[test]
    fn several_timers_run_at_once_with_their_own_labels() {
        let mut tm = Timers::default();
        let a = tm.start("", 60_000, 0.0, NOW).unwrap();
        let b = tm.start("", 30_000, 0.0, NOW).unwrap();
        let c = tm.start("Pasta", 20_000, 0.0, NOW).unwrap();
        assert_eq!(tm.list.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Timer", "Timer 2", "Pasta"]);
        assert_eq!(tm.next_due(), Some(20_000.0));
        let done = tm.poll(31_000.0);
        assert_eq!(done.iter().map(|i| i.id).collect::<Vec<_>>(), [c, b], "in order of finishing");
        assert_eq!(tm.list.len(), 1);
        assert_eq!(tm.list[0].id, a);
        tm.rename(a, "  Laundry ");
        assert_eq!(tm.list[0].label, "Laundry");
        tm.rename(a, " ");
        assert_eq!(tm.list[0].label, "Timer");
        tm.remove(a);
        assert!(tm.list.is_empty());
        assert_eq!(tm.start("", 1000, 0.0, NOW), Some(4), "ids keep counting");
    }

    #[test]
    fn the_store_survives_a_restart_by_absolute_end_time() {
        let mut tm = Timers::default();
        tm.start("Eggs", 600_000, 5000.0, NOW).unwrap();
        let paused = tm.start("Bread", 3_600_000, 5000.0, NOW).unwrap();
        tm.toggle(paused, 65_000.0, NOW + 60_000);
        tm.start("Done soon", 30_000, 5000.0, NOW).unwrap();
        let text = tm.serialize();
        assert!(text.contains(&format!("timer.0.end={}\n", NOW + 600_000)));
        assert!(text.contains("timer.1.left=3540000\n"));
        // the shell restarts 90 s later; its monotonic clock starts over at 200 ms
        let mut back = Timers::parse(&text, 200.0, NOW + 90_000);
        assert_eq!(back.list.len(), 3);
        assert!((back.list[0].remaining(200.0) - 510_000.0).abs() < 1e-6, "ten minutes minus ninety seconds");
        assert_eq!(back.list[1].remaining(200.0), 3_540_000.0, "paused stays paused");
        let done = back.poll(200.0);
        assert_eq!(done.len(), 1, "the timer that ended while the shell was down reports at once");
        assert_eq!(done[0].label, "Done soon");
        assert_eq!(back.start("x", 1000, 200.0, NOW + 90_000), Some(4));
    }

    #[test]
    fn parse_drops_junk_and_repairs_ids() {
        assert!(Timers::parse("", 0.0, NOW).list.is_empty());
        let t = Timers::parse("junk\ntimer.0.duration=0\ntimer.0.left=5\ntimer.1.duration=1000\ntimer.2.duration=2000\ntimer.2.left=999999\ntimer.2.id=7\ntimer.3.duration=4000\ntimer.3.left=1\ntimer.3.id=7\n", 0.0, NOW);
        assert_eq!(t.list.len(), 2, "no duration and no state are dropped");
        assert_eq!(t.list[0].run, Run::Paused { left_ms: 2000 }, "left is clamped to the duration");
        assert_ne!(t.list[0].id, t.list[1].id);
    }

    #[test]
    fn formats_round_up() {
        assert_eq!(format(900_000.0), "15:00");
        assert_eq!(format(899_001.0), "15:00");
        assert_eq!(format(899_000.0), "14:59");
        assert_eq!(format(3_600_000.0), "1:00:00");
        assert_eq!(format(0.0), "00:00");
        assert_eq!(split(3_661_000.0), (1, 1, 1));
    }
}
