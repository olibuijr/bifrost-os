//! Dated text inside a widget that stays current while the app is not running: the text shown at a moment for a
//! target date, and the instant the text can next change, so the owner of the widget's timeline asks for the next
//! entry in time. Pure model: dates are seconds since 1970-01-01 as `f64`, and callers pass `now`; nothing reads a
//! clock. The relative wording is the shared `format::relative`, and the clock wording is `format::duration`.

use crate::format;

/// How a dated value is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Wording such as "in 2 minutes" or "2 hours ago", from now to the target.
    Relative,
    /// The distance to the target as a clock, "m:ss" or "h:mm:ss", without a sign.
    Timer,
}

impl Style {
    /// The style an app names in a timeline entry: `"relative"` or `"timer"`.
    pub fn from_key(key: &str) -> Option<Style> {
        match key {
            "relative" => Some(Style::Relative),
            "timer" => Some(Style::Timer),
            _ => None,
        }
    }
}

/// The text shown at `now` for a target date. A fraction of a second is cut off towards zero, so the distance is
/// counted in whole seconds.
pub fn text(style: Style, target: f64, now: f64) -> String {
    let d = target - now;
    match style {
        Style::Relative => format::relative(d.trunc() as i64),
        Style::Timer => format::duration(d.abs().trunc() as u64),
    }
}

/// The grid step of relative wording at a distance: seconds below a minute, then minutes, hours and days.
fn step(distance: f64) -> f64 {
    if distance < 60.0 {
        1.0
    } else if distance < 3600.0 {
        60.0
    } else if distance < 86400.0 {
        3600.0
    } else {
        86400.0
    }
}

/// The instant after `now` at which the text for `target` can change. Relative wording changes when the distance
/// crosses a multiple of its step, so the result is never later than a real change; it may be earlier when the
/// wording stays the same (for example around the "now" cut-off). A clock changes every whole second.
pub fn next_change(style: Style, target: f64, now: f64) -> f64 {
    match style {
        Style::Timer => now.floor() + 1.0,
        Style::Relative => {
            let d = target - now;
            let a = d.abs();
            let s = step(a);
            let r = a % s;
            if d >= 0.0 {
                // The distance shrinks: the next crossing is the multiple of the step just below it.
                now + if r == 0.0 { s } else { r }
            } else {
                // The distance grows: the next crossing is the multiple of the step just above it.
                now + (s - r)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_wording_counts_from_now_to_the_target() {
        assert_eq!(text(Style::Relative, 130.0, 0.0), "in 2 minutes");
        assert_eq!(text(Style::Relative, -7200.0, 0.0), "2 hours ago");
        assert_eq!(text(Style::Relative, 59.9, 0.0), "in 59 seconds", "fraction cut off towards zero");
    }

    #[test]
    fn timer_wording_is_the_distance_as_a_clock() {
        assert_eq!(text(Style::Timer, 3725.4, 0.0), "1:02:05");
        assert_eq!(text(Style::Timer, -65.0, 0.0), "1:05", "the sign is dropped");
        assert_eq!(text(Style::Timer, 0.0, 0.0), "0:00");
    }

    #[test]
    fn a_clock_changes_on_the_next_whole_second() {
        assert_eq!(next_change(Style::Timer, 500.0, 100.25), 101.0);
        assert_eq!(next_change(Style::Timer, 500.0, 100.0), 101.0);
    }

    #[test]
    fn a_future_date_changes_at_the_multiple_just_below_it() {
        // 130 s away: "2 minutes" until the distance reaches 120 s, which is 10 s from now.
        assert_eq!(next_change(Style::Relative, 130.0, 0.0), 10.0);
        // Exactly 120 s away: the wording changes to "1 minute" at 60 s, which is 60 s from now.
        assert_eq!(next_change(Style::Relative, 120.0, 0.0), 60.0);
    }

    #[test]
    fn a_past_date_changes_at_the_multiple_just_above_it() {
        // 130 s ago: "2 minutes ago" until the distance reaches 180 s, which is 50 s from now.
        assert_eq!(next_change(Style::Relative, -130.0, 0.0), 50.0);
        // Exactly 120 s ago: the next change is at 180 s, 60 s from now.
        assert_eq!(next_change(Style::Relative, -120.0, 0.0), 60.0);
    }

    #[test]
    fn the_text_really_changes_at_the_reported_instant() {
        let target = 130.0;
        let before = text(Style::Relative, target, 9.9);
        let at = next_change(Style::Relative, target, 0.0);
        assert_eq!(before, "in 2 minutes");
        // At 10 s the distance is exactly 120 s, which still reads "2 minutes"; the wording drops to "1 minute" just after.
        assert_eq!(text(Style::Relative, target, at - 0.5), "in 2 minutes");
        assert_eq!(text(Style::Relative, target, at), "in 2 minutes");
        assert_eq!(text(Style::Relative, target, at + 0.5), "in 1 minute");
    }

    #[test]
    fn an_app_names_a_style_by_its_key() {
        assert_eq!(Style::from_key("relative"), Some(Style::Relative));
        assert_eq!(Style::from_key("timer"), Some(Style::Timer));
        assert_eq!(Style::from_key("offset"), None);
    }
}
