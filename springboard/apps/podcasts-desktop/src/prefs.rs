//! Podcasts settings: the app settings (skip intervals, refresh period) and the per-show options (auto-download,
//! how many downloads to keep, episode order). Pure data with a tab-separated file; the app reads it on start and
//! writes it when a setting changes.

use springboard::shell::prefs::write_atomic;
use std::path::Path;

/// Skip intervals the settings offer, in seconds.
pub const SKIP_CHOICES: [u32; 5] = [10, 15, 30, 45, 60];
/// Refresh periods the settings offer, in minutes; 0 is manual refresh only.
pub const REFRESH_CHOICES: [u32; 5] = [0, 15, 30, 60, 360];
/// Downloads to keep per show; 0 keeps all.
pub const KEEP_CHOICES: [u32; 5] = [0, 1, 3, 5, 10];

/// The choice after `current` in `list`, wrapping; the first when `current` is not in the list.
pub fn next_choice(list: &[u32], current: u32) -> u32 {
    match list.iter().position(|&c| c == current) {
        Some(i) => list[(i + 1) % list.len()],
        None => list[0],
    }
}

/// App-wide settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub back_s: u32,
    pub forward_s: u32,
    pub refresh_min: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { back_s: crate::transport::BACK_SKIP_S as u32, forward_s: crate::transport::FORWARD_SKIP_S as u32, refresh_min: 60 }
    }
}

impl Settings {
    pub fn cycle_back(&mut self) {
        self.back_s = next_choice(&SKIP_CHOICES, self.back_s);
    }

    pub fn cycle_forward(&mut self) {
        self.forward_s = next_choice(&SKIP_CHOICES, self.forward_s);
    }

    pub fn cycle_refresh(&mut self) {
        self.refresh_min = next_choice(&REFRESH_CHOICES, self.refresh_min);
    }

    /// "Manually", "Every 15 minutes", "Every hour", "Every 6 hours".
    pub fn refresh_label(&self) -> String {
        match self.refresh_min {
            0 => "Manually".into(),
            60 => "Every hour".into(),
            m if m % 60 == 0 => format!("Every {} hours", m / 60),
            m => format!("Every {m} minutes"),
        }
    }

    /// The refresh period in milliseconds, None when refresh is manual.
    pub fn refresh_ms(&self) -> Option<f64> {
        (self.refresh_min > 0).then(|| self.refresh_min as f64 * 60_000.0)
    }
}

/// Options of one followed show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShowPrefs {
    /// Download new episodes when a refresh finds them.
    pub auto_download: bool,
    /// Downloaded episodes to keep, newest first; 0 keeps all.
    pub keep: u32,
    /// The episode list shows the oldest episode first.
    pub oldest_first: bool,
}

impl ShowPrefs {
    pub fn cycle_keep(&mut self) {
        self.keep = next_choice(&KEEP_CHOICES, self.keep);
    }

    /// "All Episodes" or "Latest 3".
    pub fn keep_label(&self) -> String {
        match self.keep {
            0 => "All Episodes".into(),
            n => format!("Latest {n}"),
        }
    }

    pub fn order_label(&self) -> &'static str {
        if self.oldest_first { "Oldest First" } else { "Newest First" }
    }
}

/// Everything the settings file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub settings: Settings,
    shows: Vec<(String, ShowPrefs)>,
}

impl Prefs {
    pub fn show(&self, url: &str) -> ShowPrefs {
        self.shows.iter().find(|(u, _)| u == url).map(|(_, p)| *p).unwrap_or_default()
    }

    pub fn set_show(&mut self, url: &str, prefs: ShowPrefs) {
        self.shows.retain(|(u, _)| u != url);
        if prefs != ShowPrefs::default() {
            self.shows.push((url.to_string(), prefs));
        }
    }

    pub fn forget_show(&mut self, url: &str) {
        self.shows.retain(|(u, _)| u != url);
    }

    pub fn parse(text: &str) -> Self {
        let mut prefs = Prefs::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["back", v] => prefs.settings.back_s = v.parse().ok().filter(|v| SKIP_CHOICES.contains(v)).unwrap_or(15),
                ["forward", v] => prefs.settings.forward_s = v.parse().ok().filter(|v| SKIP_CHOICES.contains(v)).unwrap_or(30),
                ["refresh", v] => prefs.settings.refresh_min = v.parse().ok().filter(|v| REFRESH_CHOICES.contains(v)).unwrap_or(60),
                ["show", url, auto, keep, oldest] => {
                    let p = ShowPrefs { auto_download: *auto == "1", keep: keep.parse().unwrap_or(0), oldest_first: *oldest == "1" };
                    prefs.set_show(url, p);
                }
                _ => {}
            }
        }
        prefs
    }

    pub fn encode(&self) -> String {
        let mut out = format!("back\t{}\nforward\t{}\nrefresh\t{}\n", self.settings.back_s, self.settings.forward_s, self.settings.refresh_min);
        for (url, p) in &self.shows {
            out.push_str(&format!("show\t{url}\t{}\t{}\t{}\n", u8::from(p.auto_download), p.keep, u8::from(p.oldest_first)));
        }
        out
    }

    fn file(dir: &Path) -> std::path::PathBuf {
        dir.join("prefs.tsv")
    }

    pub fn load(dir: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(Self::file(dir)).unwrap_or_default())
    }

    pub fn save(&self, dir: &Path) {
        let _ = write_atomic(&Self::file(dir), self.encode().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_cycle_and_wrap() {
        assert_eq!(next_choice(&SKIP_CHOICES, 10), 15);
        assert_eq!(next_choice(&SKIP_CHOICES, 60), 10);
        assert_eq!(next_choice(&SKIP_CHOICES, 7), 10, "an unknown value restarts at the first choice");
        let mut s = Settings::default();
        s.cycle_back();
        s.cycle_forward();
        s.cycle_refresh();
        assert_eq!((s.back_s, s.forward_s, s.refresh_min), (30, 45, 360));
    }

    #[test]
    fn refresh_labels_and_periods() {
        let mut s = Settings { refresh_min: 0, ..Settings::default() };
        assert_eq!((s.refresh_label().as_str(), s.refresh_ms()), ("Manually", None));
        s.refresh_min = 15;
        assert_eq!((s.refresh_label().as_str(), s.refresh_ms()), ("Every 15 minutes", Some(900_000.0)));
        s.refresh_min = 60;
        assert_eq!(s.refresh_label(), "Every hour");
        s.refresh_min = 360;
        assert_eq!(s.refresh_label(), "Every 6 hours");
    }

    #[test]
    fn show_options_cycle_and_label() {
        let mut p = ShowPrefs::default();
        assert_eq!((p.keep_label().as_str(), p.order_label()), ("All Episodes", "Newest First"));
        p.cycle_keep();
        p.oldest_first = true;
        assert_eq!((p.keep_label().as_str(), p.order_label()), ("Latest 1", "Oldest First"));
    }

    #[test]
    fn prefs_round_trip_and_drop_defaults() {
        let mut prefs = Prefs::default();
        prefs.settings = Settings { back_s: 10, forward_s: 60, refresh_min: 0 };
        prefs.set_show("https://a.test/rss", ShowPrefs { auto_download: true, keep: 3, oldest_first: false });
        prefs.set_show("https://b.test/rss", ShowPrefs::default());
        let back = Prefs::parse(&prefs.encode());
        assert_eq!(back, prefs);
        assert_eq!(back.show("https://a.test/rss").keep, 3);
        assert_eq!(back.show("https://b.test/rss"), ShowPrefs::default());
        let mut back = back;
        back.forget_show("https://a.test/rss");
        assert_eq!(back.show("https://a.test/rss"), ShowPrefs::default());
    }

    #[test]
    fn bad_values_fall_back_to_the_defaults() {
        let prefs = Prefs::parse("back\t7\nforward\tx\nrefresh\t5\nnonsense\nshow\tu\t1\t2\n");
        assert_eq!(prefs.settings, Settings::default());
    }

    #[test]
    fn the_file_is_written_and_read() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut prefs = Prefs::default();
        prefs.settings.back_s = 45;
        prefs.save(&dir);
        assert_eq!(Prefs::load(&dir).settings.back_s, 45);
        assert_eq!(Prefs::load(&dir.join("missing")), Prefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
