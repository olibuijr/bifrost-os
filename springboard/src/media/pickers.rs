//! Choice models behind the video player's pickers: the playback speed and the subtitle track. Pure: nothing here draws
//! a row or touches a player. The views list `SPEEDS` and `Subtitles::rows`, and the owner applies the choice (a rate
//! through `timestretch::clamp_rate`, a track through `subtitles::Track`).

use super::timestretch::clamp_rate;

/// The playback speeds the picker offers, slowest first. 1.0 is normal speed.
pub const SPEEDS: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0];

/// The row label for a speed: "1x", "1.25x". The rate is clamped first, so the label always names a speed that can play.
pub fn speed_label(rate: f32) -> String {
    format!("{}x", clamp_rate(rate))
}

/// The index in `SPEEDS` of the offered speed nearest `rate` (clamped first). A tie goes to the slower speed.
pub fn nearest_speed(rate: f32) -> usize {
    let r = clamp_rate(rate);
    let mut best = 0;
    for (i, s) in SPEEDS.iter().enumerate() {
        if (s - r).abs() < (SPEEDS[best] - r).abs() {
            best = i;
        }
    }
    best
}

/// The next offered speed above (`faster`) or below `rate`. It stops at the fastest and slowest offered speeds.
pub fn step_speed(rate: f32, faster: bool) -> f32 {
    let i = nearest_speed(rate);
    let j = if faster { (i + 1).min(SPEEDS.len() - 1) } else { i.saturating_sub(1) };
    SPEEDS[j]
}

/// The row that turns subtitles off.
pub const OFF_ROW: &str = "Off";

/// The subtitle picker: row 0 is Off, then one row per track in the order the file lists them. `selected` is the track
/// index, or `None` for Off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subtitles {
    names: Vec<String>,
    selected: Option<usize>,
}

impl Subtitles {
    /// A picker over the named tracks, with subtitles off.
    pub fn new(names: Vec<String>) -> Subtitles {
        Subtitles { names, selected: None }
    }

    /// The rows in order: Off, then the track names.
    pub fn rows(&self) -> Vec<String> {
        std::iter::once(OFF_ROW.to_string()).chain(self.names.iter().cloned()).collect()
    }

    /// The row that is checked: 0 for Off, otherwise the track's row.
    pub fn selected_row(&self) -> usize {
        self.selected.map_or(0, |i| i + 1)
    }

    /// Chooses row `row` (0 is Off). A row past the last track is refused, returns false and leaves the choice as it was.
    pub fn choose(&mut self, row: usize) -> bool {
        if row == 0 {
            self.selected = None;
            true
        } else if row <= self.names.len() {
            self.selected = Some(row - 1);
            true
        } else {
            false
        }
    }

    /// The track to load, or `None` while subtitles are off.
    pub fn track(&self) -> Option<usize> {
        self.selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_speed_label_names_the_clamped_rate() {
        assert_eq!(speed_label(1.0), "1x");
        assert_eq!(speed_label(1.25), "1.25x");
        assert_eq!(speed_label(0.1), "0.5x");
        assert_eq!(speed_label(9.0), "3x");
    }

    #[test]
    fn the_nearest_offered_speed_is_found_and_a_tie_goes_slower() {
        assert_eq!(nearest_speed(1.0), 2);
        assert_eq!(nearest_speed(1.1), 2);
        assert_eq!(nearest_speed(1.2), 3);
        // 0.625 is halfway between 0.5 and 0.75: the slower one wins.
        assert_eq!(nearest_speed(0.625), 0);
        assert_eq!(nearest_speed(f32::NAN), 2);
    }

    #[test]
    fn stepping_moves_one_offered_speed_and_stops_at_the_ends() {
        assert_eq!(step_speed(1.0, true), 1.25);
        assert_eq!(step_speed(1.0, false), 0.75);
        assert_eq!(step_speed(3.0, true), 3.0);
        assert_eq!(step_speed(0.5, false), 0.5);
        assert_eq!(step_speed(1.1, true), 1.25);
    }

    #[test]
    fn subtitles_start_off_and_list_off_first() {
        let subs = Subtitles::new(vec!["English".into(), "Islenska".into()]);
        assert_eq!(subs.rows(), vec!["Off", "English", "Islenska"]);
        assert_eq!(subs.selected_row(), 0);
        assert_eq!(subs.track(), None);
    }

    #[test]
    fn choosing_a_row_selects_that_track_and_zero_turns_subtitles_off() {
        let mut subs = Subtitles::new(vec!["English".into(), "Islenska".into()]);
        assert!(subs.choose(2));
        assert_eq!(subs.track(), Some(1));
        assert_eq!(subs.selected_row(), 2);
        assert!(subs.choose(0));
        assert_eq!(subs.track(), None);
    }

    #[test]
    fn a_row_past_the_last_track_is_refused_and_keeps_the_choice() {
        let mut subs = Subtitles::new(vec!["English".into()]);
        assert!(subs.choose(1));
        assert!(!subs.choose(2));
        assert_eq!(subs.track(), Some(0));
    }
}
