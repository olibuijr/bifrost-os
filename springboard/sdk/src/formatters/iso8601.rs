//! `ISO8601DateFormatter` parity: the internet date-time form `2026-10-14T12:30:05+00:00`, and its reading back.
//! Times are whole seconds since 1970-01-01T00:00:00Z. Fractional seconds are not written or read.

use crate::calendar_system::{civil_from_days, days_from_civil, is_gregorian_leap};

const DAY: i64 = 86_400;

/// Formats `unix` as local time at `offset_seconds` east of UTC. A zero offset is written as `Z`.
pub fn string(unix: i64, offset_seconds: i32) -> String {
    let local = unix + i64::from(offset_seconds);
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let secs = local.rem_euclid(DAY);
    let zone = if offset_seconds == 0 {
        "Z".to_string()
    } else {
        let sign = if offset_seconds < 0 { '-' } else { '+' };
        let minutes = offset_seconds.abs() / 60;
        format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
    };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{zone}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Reads `YYYY-MM-DDThh:mm:ss` followed by `Z` or `±hh:mm`, and returns the Unix time. None for any other text,
/// a date that does not exist, or a time outside the day.
pub fn parse(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[10] != b'T' {
        return None;
    }
    let y: i64 = field(text, 0, 4)?;
    let m: i64 = field(text, 5, 2)?;
    let d: i64 = field(text, 8, 2)?;
    let hh: i64 = field(text, 11, 2)?;
    let mm: i64 = field(text, 14, 2)?;
    let ss: i64 = field(text, 17, 2)?;
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    let offset = zone_offset(&text[19..])?;
    let local = days_from_civil(y, m, d) * DAY + hh * 3600 + mm * 60 + ss;
    Some(local - offset)
}

/// The digits of `text` from `start` to `start + len`, or None when they are not all digits.
fn field(text: &str, start: usize, len: usize) -> Option<i64> {
    let part = text.get(start..start + len)?;
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// The offset in seconds of a zone designator: `Z`, or `±hh:mm` with hours up to 23 and minutes up to 59.
fn zone_offset(zone: &str) -> Option<i64> {
    if zone == "Z" {
        return Some(0);
    }
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    if zone.len() != 6 || zone.as_bytes()[3] != b':' {
        return None;
    }
    let hours: i64 = field(zone, 1, 2)?;
    let minutes: i64 = field(zone, 4, 2)?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_gregorian_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
