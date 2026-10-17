//! Keychain Access logic that needs no window: the order of the items, the group titles and the time of the last
//! change. Pure, so it is tested alone.

use bifrost_app::keychain_groups::Row;

/// Items in order by service, then account.
pub fn sort(rows: &mut [Row]) {
    rows.sort_by(|a, b| a.service.cmp(&b.service).then_with(|| a.account.cmp(&b.account)));
}

/// The title of a group: its last dotted part with the first letter upper case (`is.bifrost.passwords` is
/// `Passwords`).
pub fn title(group: &str) -> String {
    let last = group.rsplit('.').next().unwrap_or(group);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => group.to_string(),
    }
}

/// Civil date from days since 1970-01-01 (proleptic Gregorian calendar).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The UTC time `YYYY-MM-DD HH:MM` for a Unix time.
pub fn stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, account: &str) -> Row {
        Row { service: service.into(), account: account.into(), modified: 0 }
    }

    #[test]
    fn items_sort_by_service_then_account() {
        let mut rows = vec![row("b", "2"), row("a", "z"), row("b", "1")];
        sort(&mut rows);
        let order: Vec<(&str, &str)> = rows.iter().map(|r| (r.service.as_str(), r.account.as_str())).collect();
        assert_eq!(order, vec![("a", "z"), ("b", "1"), ("b", "2")]);
    }

    #[test]
    fn group_titles_come_from_the_last_part() {
        assert_eq!(title("is.bifrost.passwords"), "Passwords");
        assert_eq!(title("is.bifrost.account"), "Account");
        assert_eq!(title("plain"), "Plain");
        assert_eq!(title(""), "");
    }

    #[test]
    fn stamps_are_utc_minutes() {
        assert_eq!(stamp(0), "1970-01-01 00:00");
        assert_eq!(stamp(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(stamp(951_782_400), "2000-02-29 00:00");
    }
}
