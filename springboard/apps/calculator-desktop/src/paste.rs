//! Parses pasted text into a number: thousands separators in either
//! convention, currency marks, `(12)` negatives, trailing `%` and the
//! programmer's `0x`/`0b`/`0o` and `1_000` forms.

use super::format::Locale;

fn strip_noise(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                ' ' | '\u{a0}' | '\u{2009}' | '\u{202f}' | '_' | '\'' | '’' | '$' | '€' | '£'
                    | '¥' | '₹'
            )
        })
        .collect()
}

/// Resolves `,` and `.` into a plain `1234.56` (or `None` when the digits
/// are not a well-formed number). `locale` breaks ties such as `1,234`.
fn resolve_separators(digits: &str, locale: Locale) -> Option<String> {
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
        return None;
    }
    if !digits.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let commas = digits.matches(',').count();
    let dots = digits.matches('.').count();
    let groups_ok = |sep: char| {
        let mut parts = digits.split(sep);
        let first = parts.next().unwrap_or("");
        (1..=3).contains(&first.len()) && parts.all(|p| p.len() == 3)
    };
    if commas > 0 && dots > 0 {
        let last_comma = digits.rfind(',')?;
        let last_dot = digits.rfind('.')?;
        let (decimal, group) = if last_comma > last_dot { (',', '.') } else { ('.', ',') };
        if digits.matches(decimal).count() != 1 {
            return None;
        }
        let (integer, fraction) = digits.split_once(decimal)?;
        let mut parts = integer.split(group);
        let first = parts.next().unwrap_or("");
        let grouped = integer.contains(group);
        if fraction.contains(group)
            || first.is_empty()
            || (grouped && !((1..=3).contains(&first.len()) && parts.all(|p| p.len() == 3)))
        {
            return None;
        }
        let integer: String = integer.chars().filter(|&c| c != group).collect();
        return Some(format!("{integer}.{fraction}"));
    }
    let sep = if commas > 0 { ',' } else { '.' };
    let n = commas + dots;
    if n == 0 {
        return Some(digits.to_string());
    }
    if n > 1 {
        return groups_ok(sep).then(|| digits.replace(sep, ""));
    }
    let (a, b) = digits.split_once(sep)?;
    let thousands = sep != locale.decimal() && b.len() == 3 && (1..=3).contains(&a.len()) && a != "0";
    if thousands {
        Some(format!("{a}{b}"))
    } else {
        Some(format!("{}{}{b}", if a.is_empty() { "0" } else { a }, "."))
    }
}

/// The value of `text`, or `None` when it is not a number.
pub fn parse(text: &str, locale: Locale) -> Option<f64> {
    let mut s = text.trim().to_string();
    let mut negative = false;
    if s.starts_with('(') && s.ends_with(')') && s.len() > 2 {
        negative = true;
        s = s[1..s.len() - 1].to_string();
    }
    let mut percent = false;
    if let Some(rest) = s.trim_end().strip_suffix('%') {
        percent = true;
        s = rest.to_string();
    }
    let lower = s.trim_end().to_lowercase();
    for suffix in ["isk", "kr.", "kr", "usd", "eur"] {
        if let Some(rest) = lower.strip_suffix(suffix) {
            s = s.get(..rest.len())?.to_string();
            break;
        }
    }
    let mut s = strip_noise(&s);
    if let Some(rest) = s.strip_prefix('-').or_else(|| s.strip_prefix('−')) {
        negative = !negative;
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix('+') {
        s = rest.to_string();
    }
    let lower = s.to_lowercase();
    let radix = [("0x", 16), ("0b", 2), ("0o", 8)]
        .into_iter()
        .find(|(prefix, _)| lower.starts_with(prefix));
    let mut value = if let Some((prefix, radix)) = radix {
        i64::from_str_radix(&lower[prefix.len()..], radix).ok()? as f64
    } else {
        let plain = resolve_separators(&s, locale)?;
        plain.parse::<f64>().ok()?
    };
    if percent {
        value /= 100.0;
    }
    if negative {
        value = -value;
    }
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Locale::{En, Is};

    #[test]
    fn plain_numbers() {
        assert_eq!(parse("42", En), Some(42.0));
        assert_eq!(parse("  -3.5 \n", En), Some(-3.5));
        assert_eq!(parse("+7", En), Some(7.0));
        assert_eq!(parse("−7", En), Some(-7.0));
        assert_eq!(parse(".5", En), Some(0.5));
        assert_eq!(parse("1e3", En), None);
    }

    #[test]
    fn thousands_separators_in_both_conventions() {
        assert_eq!(parse("1,234,567", En), Some(1234567.0));
        assert_eq!(parse("1,234,567.89", En), Some(1234567.89));
        assert_eq!(parse("1.234.567,89", Is), Some(1234567.89));
        assert_eq!(parse("1.234.567,89", En), Some(1234567.89));
        assert_eq!(parse("1,234.5", Is), Some(1234.5));
        assert_eq!(parse("1 234 567", En), Some(1234567.0));
        assert_eq!(parse("1\u{a0}234,5", Is), Some(1234.5));
        assert_eq!(parse("1'234'567", En), Some(1234567.0));
        assert_eq!(parse("1_000_000", En), Some(1000000.0));
    }

    #[test]
    fn a_single_separator_uses_the_locale_to_break_ties() {
        assert_eq!(parse("1,234", En), Some(1234.0));
        assert_eq!(parse("1,234", Is), Some(1.234));
        assert_eq!(parse("1.234", En), Some(1.234));
        assert_eq!(parse("1.234", Is), Some(1234.0));
        assert_eq!(parse("1,5", En), Some(1.5));
        assert_eq!(parse("0,500", En), Some(0.5));
        assert_eq!(parse("12,34", Is), Some(12.34));
        assert_eq!(parse("1234,567", En), Some(1234.567));
    }

    #[test]
    fn malformed_separators_are_rejected() {
        for bad in ["1,23,456", "1..2", "1,2,3.4,5", ",", ".", "", "abc", "12abc", "1.2.3,4", "--1"] {
            assert_eq!(parse(bad, En), None, "{bad:?}");
        }
    }

    #[test]
    fn currency_negatives_and_percent() {
        assert_eq!(parse("$1,234.50", En), Some(1234.5));
        assert_eq!(parse("€ 1.234,50", Is), Some(1234.5));
        assert_eq!(parse("12.500 kr.", Is), Some(12500.0));
        assert_eq!(parse("1.500 ISK", Is), Some(1500.0));
        assert_eq!(parse("(1,234)", En), Some(-1234.0));
        assert_eq!(parse("-(5)", En), None);
        assert_eq!(parse("50%", En), Some(0.5));
    }

    #[test]
    fn programmer_radix_prefixes() {
        assert_eq!(parse("0xFF", En), Some(255.0));
        assert_eq!(parse("0Xff", En), Some(255.0));
        assert_eq!(parse("-0x10", En), Some(-16.0));
        assert_eq!(parse("0b1010", En), Some(10.0));
        assert_eq!(parse("0b1111_0000", En), Some(240.0));
        assert_eq!(parse("0o17", En), Some(15.0));
        assert_eq!(parse("0xZZ", En), None);
        assert_eq!(parse("0x", En), None);
    }
}
