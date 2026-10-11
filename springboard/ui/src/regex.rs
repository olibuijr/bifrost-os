//! Regex parity (`Regex`, `Regex.Match`): a small pure-Rust pattern matcher for text search and extraction.
//!
//! Syntax: literals and the escapes `\n \t \r` and escaped metacharacters; `.`; the anchors `^` and `$`, which match
//! the start and end of the whole input; classes `[a-z]` and `[^...]`; the shorthand classes `\d \w \s` and their
//! negations, ASCII only; groups `( )` and `(?: )` (grouping only, captures are not reported yet); alternation `|`;
//! and the quantifiers `* + ? {n} {n,} {n,m}`, greedy by default and lazy with a trailing `?`.
//! Matching is a backtracking search over characters. `Match::range` holds byte offsets into the searched text.
//!
//! Not covered yet: the result builder, typed and named captures, line-anchor mode, word-boundary kinds, and
//! Unicode-aware shorthand classes.

use std::fmt;
use std::ops::Range;

/// A pattern that does not parse. `position` is the character index in the pattern where the problem was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.message, self.position)
    }
}

/// `Regex.Match`: where a match lies in the searched text, and the matched substring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub range: Range<usize>,
    pub output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassItem {
    Range(char, char),
    /// `\d` (false) or `\D` (true).
    Digit(bool),
    /// `\w` (false) or `\W` (true).
    Word(bool),
    /// `\s` (false) or `\S` (true).
    Space(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Class {
    items: Vec<ClassItem>,
    negated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rep {
    min: usize,
    max: Option<usize>,
    greedy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    Group(Vec<Vec<Node>>),
    Repeat { node: Box<Node>, rep: Rep },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Any match, starting at the first position that has one.
    First,
    /// A match that starts at the beginning of the text.
    Prefix,
    /// A match that covers the whole text.
    Whole,
}

/// A compiled regular expression. Options are set with the builder methods before matching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regex {
    pattern: String,
    branches: Vec<Vec<Node>>,
    ignore_case: bool,
    dot_matches_newlines: bool,
}

impl Regex {
    /// Compiles `pattern`, or reports the first syntax error.
    pub fn new(pattern: &str) -> Result<Regex, RegexError> {
        let mut parser = Parser { chars: pattern.chars().collect(), pos: 0 };
        let branches = parser.alternatives(0)?;
        Ok(Regex { pattern: pattern.to_string(), branches, ignore_case: false, dot_matches_newlines: false })
    }

    /// A regex that matches `text` exactly, with every metacharacter taken literally.
    pub fn verbatim(text: &str) -> Regex {
        Regex {
            pattern: text.to_string(),
            branches: vec![text.chars().map(Node::Char).collect()],
            ignore_case: false,
            dot_matches_newlines: false,
        }
    }

    /// The pattern as written (for `verbatim`, the text itself).
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns a regex that ignores letter case when matching.
    pub fn ignores_case(mut self, on: bool) -> Self {
        self.ignore_case = on;
        self
    }

    /// Returns a regex where `.` also matches a newline.
    pub fn dot_matches_newlines(mut self, on: bool) -> Self {
        self.dot_matches_newlines = on;
        self
    }

    /// The first match anywhere in `text`, if there is one.
    pub fn first_match(&self, text: &str) -> Option<Match> {
        self.search(text, Mode::First)
    }

    /// A match that starts at the beginning of `text`, if there is one.
    pub fn prefix_match(&self, text: &str) -> Option<Match> {
        self.search(text, Mode::Prefix)
    }

    /// A match that covers all of `text`, if there is one.
    pub fn whole_match(&self, text: &str) -> Option<Match> {
        self.search(text, Mode::Whole)
    }

    fn search(&self, text: &str, mode: Mode) -> Option<Match> {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        // offsets[i] is the byte offset of character i; the last entry is the end of the text.
        let mut offsets: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        offsets.push(text.len());
        let last_start = if mode == Mode::First { n } else { 0 };
        for start in 0..=last_start {
            let mut end = None;
            self.match_alt(&self.branches, &chars, start, &mut |j| {
                if mode == Mode::Whole && j != n {
                    return false;
                }
                end = Some(j);
                true
            });
            if let Some(e) = end {
                let range = offsets[start]..offsets[e];
                return Some(Match { output: text[range.clone()].to_string(), range });
            }
        }
        None
    }

    fn same_char(&self, a: char, b: char) -> bool {
        a == b || (self.ignore_case && a.to_lowercase().eq(b.to_lowercase()))
    }

    fn class_matches(&self, class: &Class, c: char) -> bool {
        let mut found = in_class(&class.items, c);
        if self.ignore_case && !found {
            found = c.to_lowercase().any(|l| in_class(&class.items, l)) || c.to_uppercase().any(|u| in_class(&class.items, u));
        }
        found != class.negated
    }

    /// Tries each branch at `i`; `k` receives the end position of every way the branch can match.
    fn match_alt(&self, branches: &[Vec<Node>], s: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        branches.iter().any(|seq| self.match_seq(seq, s, i, &mut *k))
    }

    fn match_seq(&self, seq: &[Node], s: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match seq.split_first() {
            None => k(i),
            Some((first, rest)) => self.match_node(first, s, i, &mut |j| self.match_seq(rest, s, j, &mut *k)),
        }
    }

    fn match_node(&self, node: &Node, s: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match node {
            Node::Char(c) => i < s.len() && self.same_char(s[i], *c) && k(i + 1),
            Node::Any => i < s.len() && (self.dot_matches_newlines || s[i] != '\n') && k(i + 1),
            Node::Class(class) => i < s.len() && self.class_matches(class, s[i]) && k(i + 1),
            Node::Start => i == 0 && k(i),
            Node::End => i == s.len() && k(i),
            Node::Group(branches) => self.match_alt(branches, s, i, k),
            Node::Repeat { node, rep } => self.match_repeat(node, *rep, 0, s, i, k),
        }
    }

    /// `count` repetitions have matched so far. An iteration that matches the empty string is not repeated,
    /// which keeps patterns such as `(a*)*` from looping.
    fn match_repeat(&self, node: &Node, rep: Rep, count: usize, s: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        let can_more = rep.max.map_or(true, |m| count < m);
        if rep.greedy {
            if can_more
                && self.match_node(node, s, i, &mut |j| {
                    (j != i || count < rep.min) && self.match_repeat(node, rep, count + 1, s, j, &mut *k)
                })
            {
                return true;
            }
            count >= rep.min && k(i)
        } else {
            if count >= rep.min && k(i) {
                return true;
            }
            can_more
                && self.match_node(node, s, i, &mut |j| {
                    (j != i || count < rep.min) && self.match_repeat(node, rep, count + 1, s, j, &mut *k)
                })
        }
    }
}

fn in_class(items: &[ClassItem], c: char) -> bool {
    items.iter().any(|item| match *item {
        ClassItem::Range(lo, hi) => lo <= c && c <= hi,
        ClassItem::Digit(negated) => c.is_ascii_digit() != negated,
        ClassItem::Word(negated) => (c.is_ascii_alphanumeric() || c == '_') != negated,
        ClassItem::Space(negated) => matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') != negated,
    })
}

fn shorthand(c: char) -> Option<ClassItem> {
    match c {
        'd' => Some(ClassItem::Digit(false)),
        'D' => Some(ClassItem::Digit(true)),
        'w' => Some(ClassItem::Word(false)),
        'W' => Some(ClassItem::Word(true)),
        's' => Some(ClassItem::Space(false)),
        'S' => Some(ClassItem::Space(true)),
        _ => None,
    }
}

/// The character an escape stands for when it is not a shorthand class.
fn literal_escape(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        other => other,
    }
}

/// Reads a decimal number at `*i`, advancing past it; `None` when there are no digits.
fn number_at(chars: &[char], i: &mut usize) -> Option<usize> {
    let start = *i;
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i == start {
        return None;
    }
    chars[start..*i].iter().collect::<String>().parse().ok()
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn err<T>(&self, message: &str, position: usize) -> Result<T, RegexError> {
        Err(RegexError { message: message.to_string(), position })
    }

    /// Branches separated by `|`, up to the end of the pattern or the `)` that closes the enclosing group.
    fn alternatives(&mut self, depth: usize) -> Result<Vec<Vec<Node>>, RegexError> {
        let mut branches = vec![self.sequence(depth)?];
        while self.peek() == Some('|') {
            self.pos += 1;
            branches.push(self.sequence(depth)?);
        }
        Ok(branches)
    }

    fn sequence(&mut self, depth: usize) -> Result<Vec<Node>, RegexError> {
        let mut seq = Vec::new();
        loop {
            match self.peek() {
                None | Some('|') => break,
                Some(')') => {
                    if depth == 0 {
                        return self.err("unmatched ')'", self.pos);
                    }
                    break;
                }
                Some(_) => {}
            }
            let atom = self.atom(depth)?;
            seq.push(self.quantified(atom)?);
        }
        Ok(seq)
    }

    fn atom(&mut self, depth: usize) -> Result<Node, RegexError> {
        let start = self.pos;
        let Some(c) = self.bump() else {
            return self.err("unexpected end of pattern", start);
        };
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => {
                if self.chars[self.pos..].starts_with(&['?', ':']) {
                    self.pos += 2;
                }
                let inner = self.alternatives(depth + 1)?;
                if self.peek() != Some(')') {
                    return self.err("unclosed group", start);
                }
                self.pos += 1;
                Node::Group(inner)
            }
            '[' => Node::Class(self.class(start)?),
            '\\' => self.escape(start)?,
            '*' | '+' | '?' => return self.err("nothing to repeat", start),
            other => Node::Char(other),
        })
    }

    fn escape(&mut self, start: usize) -> Result<Node, RegexError> {
        let Some(c) = self.bump() else {
            return self.err("trailing backslash", start);
        };
        if let Some(item) = shorthand(c) {
            return Ok(Node::Class(Class { items: vec![item], negated: false }));
        }
        if c.is_ascii_alphanumeric() && !matches!(c, 'n' | 't' | 'r') {
            return self.err("unknown escape", start);
        }
        Ok(Node::Char(literal_escape(c)))
    }

    /// A bracket class; `start` is the position of the opening `[`.
    fn class(&mut self, start: usize) -> Result<Class, RegexError> {
        let negated = self.peek() == Some('^');
        if negated {
            self.pos += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let at = self.pos;
            let Some(c) = self.bump() else {
                return self.err("unterminated character class", start);
            };
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let Some(e) = self.bump() else {
                    return self.err("trailing backslash", at);
                };
                if let Some(item) = shorthand(e) {
                    items.push(item);
                    continue;
                }
                literal_escape(e)
            } else {
                c
            };
            // `a-z` is a range; a `-` before `]` or at the end is a plain character.
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|&next| next != ']') {
                self.pos += 1;
                let hi_at = self.pos;
                let Some(mut hi) = self.bump() else {
                    return self.err("unterminated character class", start);
                };
                if hi == '\\' {
                    let Some(e) = self.bump() else {
                        return self.err("unterminated character class", start);
                    };
                    hi = literal_escape(e);
                }
                if hi < lo {
                    return self.err("invalid range in character class", hi_at);
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
        Ok(Class { items, negated })
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, RegexError> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => match self.braces() {
                Some(bounds) => bounds,
                // Not a repetition: the brace is an ordinary character, read by the next atom.
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        if let Some(m) = max {
            if m < min {
                return self.err("invalid repetition range", self.pos);
            }
        }
        let greedy = if self.peek() == Some('?') {
            self.pos += 1;
            false
        } else {
            true
        };
        Ok(Node::Repeat { node: Box::new(atom), rep: Rep { min, max, greedy } })
    }

    /// `{n}`, `{n,}` or `{n,m}` at the current position. Consumes it and returns the bounds, or consumes nothing.
    fn braces(&mut self) -> Option<(usize, Option<usize>)> {
        let mut i = self.pos + 1;
        let min = number_at(&self.chars, &mut i)?;
        let max = if self.chars.get(i) == Some(&',') {
            i += 1;
            number_at(&self.chars, &mut i)
        } else {
            Some(min)
        };
        if self.chars.get(i) != Some(&'}') {
            return None;
        }
        self.pos = i + 1;
        Some((min, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(pattern: &str, text: &str) -> Option<String> {
        Regex::new(pattern).unwrap().first_match(text).map(|m| m.output)
    }

    #[test]
    fn first_match_reports_the_range_and_the_substring() {
        let m = Regex::new("needle").unwrap().first_match("hay needle hay").unwrap();
        assert_eq!(m.range, 4..10);
        assert_eq!(m.output, "needle");
        assert!(Regex::new("absent").unwrap().first_match("hay").is_none());
    }

    #[test]
    fn classes_shorthands_and_bounded_repetition() {
        assert_eq!(find(r"\d{2,3}-[a-z]+", "id 7 x12-abc").as_deref(), Some("12-abc"));
        assert_eq!(find(r"\d{3}", "12 345").as_deref(), Some("345"));
        assert_eq!(find(r"[^\s]+", "  word ").as_deref(), Some("word"));
        assert_eq!(find(r"\w+", "--ok_1--").as_deref(), Some("ok_1"));
    }

    #[test]
    fn anchors_select_prefix_and_whole_matches() {
        let re = Regex::new("ab").unwrap();
        assert_eq!(re.prefix_match("abc").map(|m| m.range), Some(0..2));
        assert!(re.prefix_match("xab").is_none());
        assert!(re.whole_match("abc").is_none());
        assert!(Regex::new("^ab$").unwrap().whole_match("ab").is_some());
        assert!(Regex::new("^b").unwrap().first_match("ab").is_none());
    }

    #[test]
    fn alternation_and_groups_repeat_as_a_unit() {
        assert_eq!(find("(cat|dog)s?", "two dogs").as_deref(), Some("dogs"));
        assert_eq!(find("(?:ab)+c", "xababc").as_deref(), Some("ababc"));
    }

    #[test]
    fn ignores_case_only_when_asked() {
        assert!(Regex::new("hello").unwrap().first_match("HeLLo").is_none());
        assert!(Regex::new("hello").unwrap().ignores_case(true).first_match("HeLLo").is_some());
        assert!(Regex::new("[a-c]+").unwrap().ignores_case(true).first_match("BCA").is_some());
    }

    #[test]
    fn dot_skips_newlines_unless_enabled() {
        assert!(Regex::new("a.b").unwrap().first_match("a\nb").is_none());
        assert!(Regex::new("a.b").unwrap().dot_matches_newlines(true).first_match("a\nb").is_some());
    }

    #[test]
    fn lazy_quantifiers_take_the_shortest_match() {
        assert_eq!(find("<.+>", "<a><b>").as_deref(), Some("<a><b>"));
        assert_eq!(find("<.+?>", "<a><b>").as_deref(), Some("<a>"));
    }

    #[test]
    fn invalid_patterns_are_reported_with_a_position() {
        assert_eq!(Regex::new("[abc").unwrap_err().message, "unterminated character class");
        assert_eq!(Regex::new("(ab").unwrap_err().message, "unclosed group");
        assert_eq!(Regex::new("*a").unwrap_err().message, "nothing to repeat");
        assert_eq!(Regex::new("a{3,1}").unwrap_err().message, "invalid repetition range");
        assert_eq!(Regex::new("ab)").unwrap_err().message, "unmatched ')'");
    }

    #[test]
    fn verbatim_takes_every_character_literally() {
        let re = Regex::verbatim("a.b*");
        assert!(re.whole_match("a.b*").is_some());
        assert!(re.first_match("axbb").is_none());
        assert_eq!(re.pattern(), "a.b*");
    }

    #[test]
    fn match_offsets_are_byte_offsets() {
        let m = Regex::new("b+").unwrap().first_match("aébbb").unwrap();
        assert_eq!(m.range, 3..6);
        assert_eq!(m.output, "bbb");
    }

    #[test]
    fn empty_iterations_do_not_loop() {
        assert_eq!(find("(a*)*", "b").as_deref(), Some(""));
        assert_eq!(find("(a|)+x", "aax").as_deref(), Some("aax"));
    }
}
