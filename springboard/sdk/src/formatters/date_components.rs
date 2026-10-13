//! `DateComponentsFormatter`: a duration as days, hours, minutes and seconds. English wording.

/// `DateComponentsFormatter.UnitsStyle` (the styles that need no per-language grammar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitsStyle {
    /// "1:02:03", "5:07"
    Positional,
    /// "1h 2m 3s"
    Abbreviated,
    /// "1 hr, 2 min, 3 sec"
    Short,
    /// "1 hour, 2 minutes, 3 seconds"
    Full,
}

/// A duration in whole seconds; a negative duration formats as its magnitude.
pub fn string(seconds: i64, style: UnitsStyle) -> String {
    let total = seconds.unsigned_abs();
    let (d, h, m, s) = (total / 86_400, total / 3600 % 24, total / 60 % 60, total % 60);
    if style == UnitsStyle::Positional {
        let hours = total / 3600;
        return if hours > 0 { format!("{hours}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") };
    }
    let fields = [(d, "d", "day"), (h, "h", "hour"), (m, "m", "min"), (s, "s", "sec")];
    let mut parts: Vec<String> = fields
        .iter()
        .filter(|f| f.0 > 0)
        .map(|&(n, abbr, name)| match style {
            UnitsStyle::Abbreviated => format!("{n}{abbr}"),
            UnitsStyle::Short => match name {
                "day" => format!("{n} {}", if n == 1 { "day" } else { "days" }),
                "hour" => format!("{n} hr"),
                other => format!("{n} {other}"),
            },
            _ => {
                let word = match name {
                    "min" => "minute",
                    "sec" => "second",
                    other => other,
                };
                format!("{n} {word}{}", if n == 1 { "" } else { "s" })
            }
        })
        .collect();
    if parts.is_empty() {
        parts.push(match style {
            UnitsStyle::Abbreviated => "0s".into(),
            UnitsStyle::Short => "0 sec".into(),
            _ => "0 seconds".into(),
        });
    }
    parts.join(if style == UnitsStyle::Abbreviated { " " } else { ", " })
}
