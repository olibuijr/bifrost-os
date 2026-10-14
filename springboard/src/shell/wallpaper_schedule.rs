//! Dynamic wallpaper: a day of stills that changes with the time of day. The
//! day (1440 minutes) is cut into equal slices, one per still, in order, so
//! the first still shows from midnight. This module is pure: the shell asks
//! which still is current and how long until the next change, and sets a timer.

/// Minutes in a day.
pub const DAY_MIN: u32 = 24 * 60;

/// The index of the still shown at minute `minute` (0..1440, larger values wrap) of a day cut into `n` slices.
/// None when `n` is zero.
pub fn slice_at(n: usize, minute: u32) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let m = (minute % DAY_MIN) as u64;
    Some(((m * n as u64) / DAY_MIN as u64) as usize)
}

/// The still shown at `minute` of the day, from `stills` in order.
pub fn still_at<'a>(stills: &'a [String], minute: u32) -> Option<&'a str> {
    slice_at(stills.len(), minute).map(|i| stills[i].as_str())
}

/// Minutes from `minute` until the still changes (1..=1440). None when `n` is zero.
///
/// Slice `k` runs from `ceil(k * 1440 / n)` up to the first minute of slice `k + 1`,
/// so the change lands on the first whole minute at or after the boundary.
pub fn minutes_until_change(n: usize, minute: u32) -> Option<u32> {
    let k = slice_at(n, minute)? as u64;
    let n = n as u64;
    let day = DAY_MIN as u64;
    let boundary = ((k + 1) * day + n - 1) / n;
    let now = (minute % DAY_MIN) as u64;
    Some((boundary - now) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stills(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_schedule_has_no_still() {
        assert_eq!(slice_at(0, 600), None);
        assert_eq!(minutes_until_change(0, 600), None);
        assert_eq!(still_at(&[], 600), None);
    }

    #[test]
    fn four_stills_change_every_six_hours() {
        let s = stills(&["a", "b", "c", "d"]);
        assert_eq!(still_at(&s, 0), Some("a"));
        assert_eq!(still_at(&s, 359), Some("a"));
        assert_eq!(still_at(&s, 360), Some("b"));
        assert_eq!(still_at(&s, 720), Some("c"));
        assert_eq!(still_at(&s, 1439), Some("d"));
        assert_eq!(minutes_until_change(4, 0), Some(360));
        assert_eq!(minutes_until_change(4, 359), Some(1));
        assert_eq!(minutes_until_change(4, 360), Some(360));
    }

    #[test]
    fn uneven_slices_put_the_boundary_on_a_whole_minute() {
        // 1440 / 7 = 205.7..., so slice 1 starts at minute 206 and slice 2 at 412.
        assert_eq!(slice_at(7, 205), Some(0));
        assert_eq!(slice_at(7, 206), Some(1));
        assert_eq!(minutes_until_change(7, 205), Some(1));
        assert_eq!(minutes_until_change(7, 206), Some(206));
    }

    #[test]
    fn a_single_still_never_changes_and_the_day_wraps() {
        let s = stills(&["only"]);
        assert_eq!(still_at(&s, 1000), Some("only"));
        assert_eq!(minutes_until_change(1, 0), Some(DAY_MIN));
        assert_eq!(minutes_until_change(1, 1000), Some(440));
    }

    #[test]
    fn minutes_past_a_day_wrap_to_its_start() {
        assert_eq!(slice_at(4, DAY_MIN + 400), slice_at(4, 400));
        assert_eq!(minutes_until_change(4, DAY_MIN + 100), Some(260));
    }

    #[test]
    fn the_last_slice_runs_to_midnight() {
        assert_eq!(minutes_until_change(6, 1439), Some(1));
        assert_eq!(slice_at(6, 1439), Some(5));
    }
}
