//! `NumberFormatter` styles `spellOut` and `ordinal`, and a single entry point over all five styles.
//! Spell-out is English for every locale (other languages are not in the table yet); ordinals are
//! English suffixes ("1st") or a trailing dot ("1.") for Icelandic and German.

use crate::locale::{Locale, NumberFormatter, NumberStyle};

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const SCALES: [&str; 7] = ["", "thousand", "million", "billion", "trillion", "quadrillion", "quintillion"];

/// `NumberFormatter.Style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Decimal,
    Percent,
    Currency,
    SpellOut,
    Ordinal,
}

fn below_thousand(n: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if n >= 100 {
        parts.push(format!("{} hundred", ONES[(n / 100) as usize]));
    }
    let rest = n % 100;
    if rest >= 20 {
        let tens = TENS[(rest / 10) as usize];
        parts.push(if rest % 10 == 0 { tens.to_string() } else { format!("{tens}-{}", ONES[(rest % 10) as usize]) });
    } else if rest > 0 {
        parts.push(ONES[rest as usize].to_string());
    }
    parts.join(" ")
}

/// An unsigned integer in English words: 1234 is "one thousand two hundred thirty-four".
pub fn spell_out_unsigned(n: u64) -> String {
    if n == 0 {
        return ONES[0].into();
    }
    let mut groups = Vec::new();
    let mut rest = n;
    while rest > 0 {
        groups.push(rest % 1000);
        rest /= 1000;
    }
    let mut parts = Vec::new();
    for (scale, &g) in groups.iter().enumerate().rev() {
        if g == 0 {
            continue;
        }
        let words = below_thousand(g);
        parts.push(if scale == 0 { words } else { format!("{words} {}", SCALES[scale]) });
    }
    parts.join(" ")
}

/// `NumberFormatter` with `numberStyle = .spellOut`: "minus twenty-one", "one point five".
pub fn spell_out(_locale: &Locale, v: f64) -> String {
    if v.is_nan() {
        return "not a number".into();
    }
    let sign = if v < 0.0 { "minus " } else { "" };
    if v.is_infinite() {
        return format!("{sign}infinity");
    }
    let a = v.abs();
    if a >= 1e18 {
        return format!("{sign}{a}");
    }
    let mut out = format!("{sign}{}", spell_out_unsigned(a.trunc() as u64));
    let text = format!("{a}");
    if let Some((_, frac)) = text.split_once('.') {
        out.push_str(" point");
        for c in frac.chars().filter_map(|c| c.to_digit(10)) {
            out.push(' ');
            out.push_str(ONES[c as usize]);
        }
    }
    out
}

/// `NumberFormatter` with `numberStyle = .ordinal`: "1st", "2nd", "11th", "23rd" in English; "1." in
/// Icelandic and German.
pub fn ordinal(locale: &Locale, n: i64) -> String {
    if matches!(locale.language.as_str(), "is" | "de") {
        return format!("{n}.");
    }
    let a = n.unsigned_abs();
    let suffix = match (a % 10, a % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// `NumberFormatter(numberStyle:).string(from:)` for every style.
pub fn format(locale: &Locale, style: Style, v: f64) -> String {
    let plain = |s| NumberFormatter::new(locale.clone(), s).string(v);
    match style {
        Style::Decimal => plain(NumberStyle::Decimal),
        Style::Percent => plain(NumberStyle::Percent),
        Style::Currency => plain(NumberStyle::Currency),
        Style::SpellOut => spell_out(locale, v),
        Style::Ordinal => ordinal(locale, v.round() as i64),
    }
}
