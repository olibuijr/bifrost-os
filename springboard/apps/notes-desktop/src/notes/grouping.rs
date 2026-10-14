//! Group notes by date: the note list under Today, Yesterday, Previous 7 Days, Previous 30 Days, then one section
//! per month (and per year for older ones), newest first. Pure functions over the note modified times.
use super::store::Note;

/// A section of the grouped list: a header and the indices of its notes, in the order given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub notes: Vec<usize>,
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

/// Civil (year, month 1-12, day) of a day count since 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The header for a note changed on local day `day`, seen from local day `today`.
pub fn title_for(day: i64, today: i64) -> String {
    let age = today - day;
    if age <= 0 {
        return "Today".into();
    }
    match age {
        1 => "Yesterday".into(),
        2..=7 => "Previous 7 Days".into(),
        8..=30 => "Previous 30 Days".into(),
        _ => {
            let (y, m, _) = civil(day);
            let (ty, _, _) = civil(today);
            if y == ty {
                MONTHS[m as usize - 1].to_string()
            } else {
                format!("{y}")
            }
        }
    }
}

/// Splits `order` (indices into `notes`, already sorted by date, newest first) into dated sections. `offset_secs`
/// is the local offset from UTC and `now` the current Unix time. Notes with the same header stay together in order.
pub fn group(notes: &[Note], order: &[usize], now: i64, offset_secs: i64) -> Vec<Section> {
    let today = (now + offset_secs).div_euclid(86_400);
    let mut out: Vec<Section> = vec![];
    for &i in order {
        let day = (notes[i].modified + offset_secs).div_euclid(86_400);
        let title = title_for(day, today);
        match out.last_mut() {
            Some(s) if s.title == title => s.notes.push(i),
            _ => out.push(Section { title, notes: vec![i] }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(modified: i64) -> Note {
        Note { id: modified.to_string(), text: String::new(), modified }
    }

    const DAY: i64 = 86_400;
    // 2026-10-15 12:00 UTC
    const NOW: i64 = 1_792_065_600;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(NOW.div_euclid(DAY)), (2026, 10, 15));
        assert_eq!(civil(-1), (1969, 12, 31));
    }

    #[test]
    fn headers_by_age() {
        let today = NOW.div_euclid(DAY);
        assert_eq!(title_for(today, today), "Today");
        assert_eq!(title_for(today - 1, today), "Yesterday");
        assert_eq!(title_for(today - 5, today), "Previous 7 Days");
        assert_eq!(title_for(today - 20, today), "Previous 30 Days");
        assert_eq!(title_for(today - 45, today), "August");
        assert_eq!(title_for(today - 400, today), "2025");
    }

    #[test]
    fn groups_keep_order_and_merge_neighbours() {
        let notes = vec![note(NOW), note(NOW - 3600), note(NOW - DAY), note(NOW - 3 * DAY), note(NOW - 4 * DAY)];
        let g = group(&notes, &[0, 1, 2, 3, 4], NOW, 0);
        assert_eq!(
            g.iter().map(|s| (s.title.as_str(), s.notes.len())).collect::<Vec<_>>(),
            vec![("Today", 2), ("Yesterday", 1), ("Previous 7 Days", 2)]
        );
    }

    #[test]
    fn local_offset_moves_the_day_boundary() {
        // 23:00 UTC on the 14th is yesterday in UTC but 01:00 today at UTC+2
        let notes = vec![note(NOW - 13 * 3600)];
        assert_eq!(group(&notes, &[0], NOW, 0)[0].title, "Yesterday");
        assert_eq!(group(&notes, &[0], NOW, 2 * 3600)[0].title, "Today");
    }
}
