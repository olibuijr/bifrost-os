//! Number formatting with the stock bifrost Calculator rules: at most nine
//! digits, scientific notation for very large or small values, thousands
//! separators and an optional decimal comma (Icelandic).

/// Digits the display and the entry line hold, as on bifrost in portrait.
pub const MAX_DIGITS: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Locale {
    /// 1,234.5
    #[default]
    En,
    /// 1.234,5
    Is,
}

impl Locale {
    pub fn decimal(self) -> char {
        match self {
            Self::En => '.',
            Self::Is => ',',
        }
    }

    pub fn group(self) -> char {
        match self {
            Self::En => ',',
            Self::Is => '.',
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Is => "is",
        }
    }

    pub fn from_code(code: &str) -> Self {
        if code.trim().eq_ignore_ascii_case("is") {
            Self::Is
        } else {
            Self::En
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::En => Self::Is,
            Self::Is => Self::En,
        }
    }
}

fn trim_zeros(text: String) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

fn scientific(value: f64) -> String {
    let text = format!("{value:.8e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    format!(
        "{}e{}",
        trim_zeros(mantissa.to_string()),
        exponent.parse::<i32>().unwrap_or(0)
    )
}

/// The canonical text of `value` (ASCII, `.` decimal, no separators):
/// up to nine significant digits in normal notation, `1.5e10` otherwise.
pub fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "Error".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    let a = value.abs();
    if !(1e-5..1e9).contains(&a) {
        return scientific(value);
    }
    let places = (8 - a.log10().floor() as i32).clamp(0, 15) as usize;
    let text = trim_zeros(format!("{value:.places$}"));
    let integer = text.trim_start_matches('-').split('.').next().unwrap_or("");
    if integer.len() > MAX_DIGITS {
        return scientific(value);
    }
    text
}

fn localize_number(raw: &str, locale: Locale) -> String {
    let (sign, rest) = match raw.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", raw),
    };
    if rest.contains('e') {
        return format!("{sign}{}", rest.replace('.', &locale.decimal().to_string()));
    }
    let (integer, fraction) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut out = String::new();
    let n = integer.chars().count();
    for (k, ch) in integer.chars().enumerate() {
        if k > 0 && (n - k) % 3 == 0 {
            out.push(locale.group());
        }
        out.push(ch);
    }
    if let Some(fraction) = fraction {
        out.push(locale.decimal());
        out.push_str(fraction);
    }
    format!("{sign}{out}")
}

/// Rewrites every number in `text` (an entry such as `-1234.5`, a result
/// such as `1.5e10` or a whole expression) with the locale's separators.
/// Everything that is not a digit run passes through untouched.
pub fn localize(text: &str, locale: Locale) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            run.push(ch);
        } else {
            if !run.is_empty() {
                out.push_str(&localize_number(&run, locale));
                run.clear();
            }
            out.push(ch);
        }
    }
    if !run.is_empty() {
        out.push_str(&localize_number(&run, locale));
    }
    out
}

/// `value` as shown on the display.
pub fn display(value: f64, locale: Locale) -> String {
    localize(&format_number(value), locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_numbers_keep_nine_digits_and_trim_zeros() {
        for (v, s) in [
            (0.0, "0"),
            (-0.0, "0"),
            (15.0, "15"),
            (-6.0, "-6"),
            (2.25, "2.25"),
            (1.0 / 3.0, "0.333333333"),
            (2.0 / 3.0, "0.666666667"),
            (0.1 + 0.2, "0.3"),
            (123456789.0, "123456789"),
            (12345.6789012, "12345.6789"),
            (0.00001234, "0.00001234"),
        ] {
            assert_eq!(format_number(v), s, "{v}");
        }
    }

    #[test]
    fn large_and_tiny_values_switch_to_scientific() {
        for (v, s) in [
            (1e9, "1e9"),
            (1.5e10, "1.5e10"),
            (-2.5e15, "-2.5e15"),
            (1e-15, "1e-15"),
            (1.234e-7, "1.234e-7"),
            (999999999.7, "1e9"),
            (f64::MAX, "1.79769313e308"),
        ] {
            assert_eq!(format_number(v), s, "{v}");
        }
        assert_eq!(format_number(f64::INFINITY), "Error");
        assert_eq!(format_number(f64::NAN), "Error");
    }

    #[test]
    fn thousands_separators_follow_the_locale() {
        assert_eq!(localize("1234567", Locale::En), "1,234,567");
        assert_eq!(localize("1234567.891", Locale::En), "1,234,567.891");
        assert_eq!(localize("1234567.891", Locale::Is), "1.234.567,891");
        assert_eq!(localize("-1234.5", Locale::Is), "-1.234,5");
        assert_eq!(localize("123", Locale::En), "123");
        assert_eq!(localize("1000", Locale::En), "1,000");
        assert_eq!(localize("12.", Locale::Is), "12,");
        assert_eq!(localize("0.5", Locale::Is), "0,5");
        assert_eq!(localize("1.5e10", Locale::Is), "1,5e10");
        assert_eq!(localize("1e-15", Locale::En), "1e-15");
        assert_eq!(localize("Error", Locale::Is), "Error");
    }

    #[test]
    fn expressions_are_localised_number_by_number() {
        assert_eq!(
            localize("sin(1234.5)+2000×π", Locale::En),
            "sin(1,234.5)+2,000×π"
        );
        assert_eq!(localize("1234.5+0.5", Locale::Is), "1.234,5+0,5");
    }

    #[test]
    fn display_combines_format_and_locale() {
        assert_eq!(display(1234567.5, Locale::En), "1,234,567.5");
        assert_eq!(display(1234567.5, Locale::Is), "1.234.567,5");
        assert_eq!(display(1e15, Locale::Is), "1e15");
    }

    #[test]
    fn locale_codes_round_trip() {
        assert_eq!(Locale::from_code("is"), Locale::Is);
        assert_eq!(Locale::from_code("IS\n"), Locale::Is);
        assert_eq!(Locale::from_code("xx"), Locale::En);
        assert_eq!(Locale::Is.toggled(), Locale::En);
        assert_eq!(Locale::from_code(Locale::Is.code()), Locale::Is);
    }
}
