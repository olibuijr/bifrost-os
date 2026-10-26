//! Conversion rules between document and content types: which way an app can turn content of one type into another,
//! and whether the result loses anything. These are BifrOSt's own rules for the types in `bifrost_ui::uttype`, not
//! the reference catalogue of conversions, which is larger.
//!
//! A pair converts with [`method`]. Content that already conforms to the target (a PNG to `public.image`) is
//! `Identity`, and a listed rule applies to the exact pair. Pairs with no rule return None, and the app must refuse
//! the conversion.
//!
//! ```ignore
//! use bifrost_app::type_conversion::{method, Method};
//! assert_eq!(method("public.rtf", "public.plain-text"), Some(Method::Strip));
//! assert!(method("public.plain-text", "public.svg-image").is_none());
//! ```

use bifrost_ui::uttype::conforms;

/// How the content is turned into the target type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The bytes are already in the target type, or the target is an ancestor of the source. No work is needed.
    Identity,
    /// The content is kept and the target container adds the markup, such as plain text inside RTF.
    Wrap,
    /// The formatting is dropped, such as the styling of RTF or HTML when it becomes plain text. Lossy.
    Strip,
    /// The content is decoded and encoded again in the target format, such as PNG to JPEG. Lossy when the target
    /// is `public.jpeg`, since the encoder discards detail.
    Reencode,
}

impl Method {
    /// Whether the conversion can drop part of the content.
    pub fn loses_data(self, to: &str) -> bool {
        match self {
            Method::Identity | Method::Wrap => false,
            Method::Strip => true,
            Method::Reencode => to == "public.jpeg",
        }
    }
}

/// The rules BifrOSt implements for an exact pair of types. The first match wins.
const RULES: &[(&str, &str, Method)] = &[
    ("public.rtf", "public.plain-text", Method::Strip),
    ("public.html", "public.plain-text", Method::Strip),
    ("public.plain-text", "public.rtf", Method::Wrap),
    ("public.utf8-plain-text", "public.rtf", Method::Wrap),
    ("public.plain-text", "public.utf8-plain-text", Method::Identity),
    ("public.utf8-plain-text", "public.plain-text", Method::Identity),
    ("public.comma-separated-values-text", "public.tab-separated-values-text", Method::Reencode),
    ("public.tab-separated-values-text", "public.comma-separated-values-text", Method::Reencode),
    ("public.png", "public.jpeg", Method::Reencode),
    ("public.jpeg", "public.png", Method::Reencode),
    ("public.png", "public.webp", Method::Reencode),
    ("public.webp", "public.png", Method::Reencode),
    ("public.svg-image", "public.png", Method::Reencode),
];

/// How content of type `from` converts to type `to`, or None when no rule covers the pair.
pub fn method(from: &str, to: &str) -> Option<Method> {
    if let Some((_, _, m)) = RULES.iter().find(|(a, b, _)| *a == from && *b == to) {
        return Some(*m);
    }
    if from == to || conforms(from, to) {
        return Some(Method::Identity);
    }
    None
}

/// The distinct types that a rule converts content of type `from` into, in rule order. Ancestors are not listed.
pub fn targets(from: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (a, b, _) in RULES {
        if *a == from && !out.contains(b) {
            out.push(*b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_text_strips_its_formatting_to_plain_text() {
        let m = method("public.rtf", "public.plain-text").expect("rule");
        assert_eq!(m, Method::Strip);
        assert!(m.loses_data("public.plain-text"));
    }

    #[test]
    fn plain_text_wraps_into_rtf_without_losing_content() {
        let m = method("public.plain-text", "public.rtf").expect("rule");
        assert_eq!(m, Method::Wrap);
        assert!(!m.loses_data("public.rtf"));
    }

    #[test]
    fn a_type_converts_to_its_ancestor_by_identity() {
        assert_eq!(method("public.png", "public.image"), Some(Method::Identity));
        assert_eq!(method("public.png", "public.png"), Some(Method::Identity));
    }

    #[test]
    fn reencoding_to_jpeg_is_lossy_and_to_png_is_not() {
        assert_eq!(method("public.png", "public.jpeg"), Some(Method::Reencode));
        assert!(Method::Reencode.loses_data("public.jpeg"));
        assert!(!Method::Reencode.loses_data("public.png"));
    }

    #[test]
    fn a_pair_without_a_rule_is_refused() {
        assert_eq!(method("public.plain-text", "public.svg-image"), None);
        assert_eq!(method("public.zip-archive", "public.plain-text"), None);
    }

    #[test]
    fn targets_list_the_rule_destinations_of_a_source() {
        assert_eq!(targets("public.plain-text"), vec!["public.rtf", "public.utf8-plain-text"]);
        assert!(targets("public.zip-archive").is_empty());
    }
}
