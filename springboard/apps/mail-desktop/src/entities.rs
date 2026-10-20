//! Character references in HTML text: `&amp;`, `&bull;`, `&#8202;`, `&#x2022;`. The named ones are the whole HTML list
//! (`entities_table`); numeric ones follow the HTML rules for the characters they may not name.

use crate::entities_table::{LEGACY, NAMED};

/// The longest name in the table (`CounterClockwiseContourIntegral`), with room to spare.
const MAX_NAME: usize = 32;

/// The longest name that is also accepted without its `;`.
const MAX_LEGACY: usize = 6;

/// Resolves every reference in `s`; text that is not a reference, and a reference that names nothing, stays as written.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match reference(rest) {
            Some((text, used)) => {
                out.push_str(&text);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The reference at the start of `rest` (which starts with `&`): its text and the bytes it takes.
fn reference(rest: &str) -> Option<(String, usize)> {
    let after = &rest[1..];
    if let Some(number) = after.strip_prefix('#') {
        let (digits, hex) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, true),
            None => (number, false),
        };
        let len = digits.bytes().take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }).count();
        if len == 0 {
            return None;
        }
        let prefix = 2 + usize::from(hex);
        let semicolon = usize::from(digits[len..].starts_with(';'));
        let code = u32::from_str_radix(&digits[..len], if hex { 16 } else { 10 }).unwrap_or(u32::MAX);
        return Some((numeric(code).to_string(), prefix + len + semicolon));
    }
    let run = after.bytes().take_while(u8::is_ascii_alphanumeric).count().min(MAX_NAME);
    let name = &after[..run];
    if after[run..].starts_with(';') {
        if let Ok(i) = NAMED.binary_search_by(|(n, _)| (*n).cmp(name)) {
            return Some((NAMED[i].1.to_string(), 1 + run + 1));
        }
    }
    // without a `;`, the legacy names still count: the longest one that starts the name
    (2..=name.len().min(MAX_LEGACY)).rev().find_map(|n| {
        let candidate = name.get(..n)?;
        LEGACY.binary_search(&candidate).ok()?;
        let i = NAMED.binary_search_by(|(k, _)| (*k).cmp(candidate)).ok()?;
        Some((NAMED[i].1.to_string(), 1 + n))
    })
}

/// The character a numeric reference names. Zero, surrogates and numbers past Unicode become U+FFFD; the numbers
/// 0x80 to 0x9F name the Windows-1252 characters, as browsers read them.
fn numeric(code: u32) -> char {
    match code {
        0x80..=0x9f => crate::codec::windows_1252(code as u8),
        _ => char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_references_from_the_evidence_decode() {
        assert_eq!(decode("a&#8202;b"), "a\u{200a}b");
        assert_eq!(decode("&bull; one &bull; two"), "\u{2022} one \u{2022} two");
        assert_eq!(decode("&#x2022; &#X2022; &#8226;"), "\u{2022} \u{2022} \u{2022}");
    }

    #[test]
    fn the_common_names_decode() {
        let cases = [
            ("&amp;", "&"), ("&lt;&gt;", "<>"), ("&quot;", "\""), ("&apos;", "'"), ("&nbsp;", "\u{a0}"), ("&copy;", "\u{a9}"),
            ("&reg;", "\u{ae}"), ("&trade;", "\u{2122}"), ("&hellip;", "\u{2026}"), ("&ndash;", "\u{2013}"), ("&mdash;", "\u{2014}"),
            ("&lsquo;&rsquo;", "\u{2018}\u{2019}"), ("&ldquo;&rdquo;", "\u{201c}\u{201d}"), ("&euro;", "\u{20ac}"), ("&pound;", "\u{a3}"),
            ("&eacute;", "\u{e9}"), ("&thorn;&eth;", "\u{fe}\u{f0}"), ("&AElig;&aelig;", "\u{c6}\u{e6}"), ("&szlig;", "\u{df}"),
            ("&larr;&rarr;", "\u{2190}\u{2192}"), ("&hairsp;", "\u{200a}"), ("&zwnj;", "\u{200c}"), ("&check;", "\u{2713}"),
            ("&hearts;", "\u{2665}"), ("&alpha;&Omega;", "\u{3b1}\u{3a9}"), ("&frac12;", "\u{bd}"), ("&times;", "\u{d7}"),
        ];
        for (input, expected) in cases {
            assert_eq!(decode(input), expected, "{input}");
        }
    }

    #[test]
    fn a_name_with_two_characters_decodes_to_both() {
        assert_eq!(decode("&NotEqualTilde;"), "\u{2242}\u{338}");
    }

    #[test]
    fn a_reference_is_read_once() {
        assert_eq!(decode("&amp;amp;"), "&amp;");
        assert_eq!(decode("&amp;lt;"), "&lt;");
        assert_eq!(decode("&#38;#38;"), "&#38;");
    }

    #[test]
    fn text_that_is_not_a_reference_stays() {
        for text in ["&", "a & b", "&;", "&#;", "&#x;", "&#xZZ;", "&unknownname;", "AT&T", "&&&", "x&", "&#"] {
            assert_eq!(decode(text), text, "{text}");
        }
    }

    #[test]
    fn legacy_names_work_without_the_semicolon() {
        assert_eq!(decode("&copy 2026"), "\u{a9} 2026");
        assert_eq!(decode("&amp&lt"), "&<");
        assert_eq!(decode("&copy2026"), "\u{a9}2026");
        assert_eq!(decode("&notit;"), "\u{ac}it;");
        // a name that was never a legacy one needs its semicolon
        assert_eq!(decode("&hellip and &bull"), "&hellip and &bull");
    }

    #[test]
    fn numeric_references_follow_the_html_rules() {
        assert_eq!(decode("&#0;&#xD800;&#x110000;&#99999999999;"), "\u{fffd}\u{fffd}\u{fffd}\u{fffd}");
        assert_eq!(decode("&#x80;&#150;&#x9f;"), "\u{20ac}\u{2013}\u{178}");
        assert_eq!(decode("&#65 &#66"), "A B", "the semicolon may be left out");
        assert_eq!(decode("&#x1F600;"), "\u{1f600}");
    }

    #[test]
    fn the_table_is_sorted_so_the_search_works() {
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(LEGACY.windows(2).all(|w| w[0] < w[1]));
        assert!(LEGACY.iter().all(|name| NAMED.binary_search_by(|(k, _)| k.cmp(name)).is_ok() && name.len() <= MAX_LEGACY));
        assert!(NAMED.len() > 2000);
    }
}
