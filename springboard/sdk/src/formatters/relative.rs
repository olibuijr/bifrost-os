//! `RelativeDateTimeFormatter` with `dateTimeStyle = .named`: whole days get words ("yesterday", "today",
//! "tomorrow"), everything else falls back to the numeric style of `locale::relative_date_time`.

use crate::locale::{relative_date_time, Lang, Locale};

/// `day_delta` is the number of calendar days from now to the date (`-1` is yesterday).
pub fn named_days(locale: &Locale, day_delta: i64) -> String {
    let is = locale.lang() == Lang::Is;
    match (day_delta, is) {
        (-1, false) => "yesterday".into(),
        (0, false) => "today".into(),
        (1, false) => "tomorrow".into(),
        (-1, true) => "í gær".into(),
        (0, true) => "í dag".into(),
        (1, true) => "á morgun".into(),
        (n, _) => relative_date_time(locale, n * 86_400),
    }
}
