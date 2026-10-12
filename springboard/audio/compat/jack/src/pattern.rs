//! A small regular-expression matcher for `jack_get_ports` (the reference uses POSIX extended expressions and
//! searches for a match anywhere in the name). Supported: literals, `.`, `*`, `+`, `?`, `[..]` with ranges and
//! `^` negation, `^` and `$` anchors, `\` escapes and top-level `|`. Groups are not supported.

#[derive(Debug, Clone)]
enum Atom {
    Any,
    Lit(char),
    Class(Vec<(char, char)>, bool),
}

#[derive(Debug, Clone)]
struct Item {
    atom: Atom,
    min: usize,
    max: usize,
}

#[derive(Debug, Clone)]
struct Alt {
    start: bool,
    end: bool,
    items: Vec<Item>,
}

fn parse_alt(p: &str) -> Alt {
    let cs: Vec<char> = p.chars().collect();
    let mut i = 0;
    let mut alt = Alt { start: false, end: false, items: vec![] };
    if cs.first() == Some(&'^') {
        alt.start = true;
        i = 1;
    }
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        let atom = match c {
            '$' if i == cs.len() => {
                alt.end = true;
                continue;
            }
            '.' => Atom::Any,
            '\\' if i < cs.len() => {
                i += 1;
                Atom::Lit(cs[i - 1])
            }
            '[' => {
                let neg = cs.get(i) == Some(&'^');
                if neg {
                    i += 1;
                }
                let mut ranges = vec![];
                let mut first = true;
                while i < cs.len() && (cs[i] != ']' || first) {
                    first = false;
                    let lo = cs[i];
                    if i + 2 < cs.len() && cs[i + 1] == '-' && cs[i + 2] != ']' {
                        ranges.push((lo, cs[i + 2]));
                        i += 3;
                    } else {
                        ranges.push((lo, lo));
                        i += 1;
                    }
                }
                i += 1; // the closing bracket
                Atom::Class(ranges, neg)
            }
            c => Atom::Lit(c),
        };
        let (min, max) = match cs.get(i) {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            _ => (1, 1),
        };
        if (min, max) != (1, 1) {
            i += 1;
        }
        alt.items.push(Item { atom, min, max });
    }
    alt
}

fn atom_matches(a: &Atom, c: char) -> bool {
    match a {
        Atom::Any => true,
        Atom::Lit(l) => *l == c,
        Atom::Class(r, neg) => r.iter().any(|&(lo, hi)| c >= lo && c <= hi) != *neg,
    }
}

/// Matches `items` at the start of `s`; `end` requires consuming all of `s`.
fn match_here(items: &[Item], s: &[char], end: bool) -> bool {
    let Some((first, rest)) = items.split_first() else {
        return !end || s.is_empty();
    };
    let mut n = 0;
    while n < first.max && n < s.len() && atom_matches(&first.atom, s[n]) {
        n += 1;
    }
    loop {
        if n >= first.min && match_here(rest, &s[n..], end) {
            return true;
        }
        if n <= first.min {
            return false;
        }
        n -= 1;
    }
}

/// True when `pattern` matches somewhere in `text`. An empty pattern matches everything.
pub fn matches(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let t: Vec<char> = text.chars().collect();
    pattern.split('|').any(|p| {
        let a = parse_alt(p);
        if a.start {
            match_here(&a.items, &t, a.end)
        } else {
            (0..=t.len()).any(|i| match_here(&a.items, &t[i..], a.end))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn empty_matches_all_and_plain_is_substring() {
        assert!(matches("", "system:capture_1"));
        assert!(matches("capture", "system:capture_1"));
        assert!(!matches("playback", "system:capture_1"));
    }

    #[test]
    fn anchors_and_wildcards() {
        assert!(matches("^system:", "system:capture_1"));
        assert!(!matches("^capture", "system:capture_1"));
        assert!(matches("capture_.$", "system:capture_1"));
        assert!(!matches("capture$", "system:capture_1"));
        assert!(matches("sys.*_1", "system:capture_1"));
    }

    #[test]
    fn classes_and_alternation() {
        assert!(matches("capture_[12]", "system:capture_2"));
        assert!(!matches("capture_[^12]", "system:capture_2"));
        assert!(matches("^foo|^system", "system:playback_1"));
        assert!(matches("a+b", "xaaab"));
        assert!(!matches("^a+b", "xaaab"));
    }

    #[test]
    fn type_strings() {
        assert!(matches("8 bit raw midi", "8 bit raw midi"));
        assert!(!matches("8 bit raw midi", "32 bit float mono audio"));
    }
}
