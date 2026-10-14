//! Text Replacement: a short typed shortcut becomes its symbol when the word that holds it ends with a blank, as in
//! Edit > Substitutions > Text Replacement. The shortcuts are a fixed list; the replacements the system keeps for the
//! whole desktop are not read here. Pure: the rules are tested without a window.

/// The shortcuts and the text each one becomes.
pub const SHORTCUTS: [(&str, &str); 16] = [
    ("(c)", "\u{a9}"),
    ("(C)", "\u{a9}"),
    ("(r)", "\u{ae}"),
    ("(R)", "\u{ae}"),
    ("(tm)", "\u{2122}"),
    ("(TM)", "\u{2122}"),
    ("...", "\u{2026}"),
    ("1/2", "\u{bd}"),
    ("1/4", "\u{bc}"),
    ("3/4", "\u{be}"),
    ("+/-", "\u{b1}"),
    ("!=", "\u{2260}"),
    ("<=", "\u{2264}"),
    (">=", "\u{2265}"),
    ("->", "\u{2192}"),
    ("<-", "\u{2190}"),
];

/// The text a shortcut becomes, when the word is one.
pub fn for_shortcut(word: &str) -> Option<&'static str> {
    SHORTCUTS.iter().find(|(from, _)| *from == word).map(|(_, to)| *to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_become_their_symbols() {
        assert_eq!(for_shortcut("(c)"), Some("\u{a9}"));
        assert_eq!(for_shortcut("..."), Some("\u{2026}"));
        assert_eq!(for_shortcut("(c"), None, "only the whole word is replaced");
        assert_eq!(for_shortcut("hello"), None);
    }

    #[test]
    fn every_shortcut_is_unique() {
        for (i, (from, _)) in SHORTCUTS.iter().enumerate() {
            assert!(SHORTCUTS[i + 1..].iter().all(|(other, _)| other != from), "{from} is listed twice");
        }
    }
}
