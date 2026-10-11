//! Foundation-style formatters with fixed, locale-independent English output: byte counts, durations,
//! relative times and grouped numbers. Pure functions; apps pass in the numbers.

/// `ByteCountFormatter` (decimal units, as Golden Gate shows them): "0 bytes", "1 byte", "999 bytes", "1.5 MB", "12 GB".
pub fn byte_count(bytes: u64) -> String {
    if bytes == 1 {
        return "1 byte".into();
    }
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 999.5 && u < units.len() {
        v /= 1000.0;
        u += 1;
    }
    // Three significant digits: KB whole, MB and up with one decimal under 100 ("1.5 MB", "22.0 GB", "812 MB").
    if u > 1 && v < 99.95 { format!("{:.1} {}", v, units[u - 1]) } else { format!("{:.0} {}", v, units[u - 1]) }
}

/// `DateComponentsFormatter` positional style: "0:07", "3:25", "1:02:03".
pub fn duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// `RelativeDateTimeFormatter`: `delta_seconds` is `then - now` (negative = past).
pub fn relative(delta_seconds: i64) -> String {
    let a = delta_seconds.unsigned_abs();
    if a < 5 {
        return "now".into();
    }
    let (n, unit) = match a {
        0..=59 => (a, "second"),
        60..=3599 => (a / 60, "minute"),
        3600..=86_399 => (a / 3600, "hour"),
        86_400..=2_591_999 => (a / 86_400, "day"),
        2_592_000..=31_535_999 => (a / 2_592_000, "month"),
        _ => (a / 31_536_000, "year"),
    };
    let s = if n == 1 { "" } else { "s" };
    if delta_seconds < 0 { format!("{n} {unit}{s} ago") } else { format!("in {n} {unit}{s}") }
}

/// `NumberFormatter` decimal style with grouping: 1234567 -> "1,234,567".
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bytes() {
        assert_eq!(byte_count(0), "0 bytes");
        assert_eq!(byte_count(1), "1 byte");
        assert_eq!(byte_count(999), "999 bytes");
        assert_eq!(byte_count(1_000), "1 KB");
        assert_eq!(byte_count(1_500_000), "1.5 MB");
        assert_eq!(byte_count(12_300_000_000), "12.3 GB");
        assert_eq!(byte_count(999_999), "1.0 MB");
        assert_eq!(byte_count(117_000_000_000), "117 GB");
    }
    #[test]
    fn durations() {
        assert_eq!((duration(7).as_str(), duration(205).as_str(), duration(3723).as_str()), ("0:07", "3:25", "1:02:03"));
    }
    #[test]
    fn relatives() {
        assert_eq!(relative(-1), "now");
        assert_eq!(relative(-90), "1 minute ago");
        assert_eq!(relative(7200), "in 2 hours");
        assert_eq!(relative(-86_400 * 3), "3 days ago");
    }
    #[test]
    fn grouping() {
        assert_eq!((grouped(0).as_str(), grouped(999).as_str(), grouped(1000).as_str()), ("0", "999", "1,000"));
        assert_eq!(grouped(-1234567), "-1,234,567");
    }
}
