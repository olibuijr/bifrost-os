//! Lightweight Markdown for notes: everything here is pure text logic on
//! char indices so it can be tested on the host. A note stays a plain `.md`
//! file; this module classifies lines (headings, bullets, numbered and
//! checklist items), splits a line into styled inline spans, continues lists
//! on Enter, toggles checklist items, wraps selections in `**`, `*` or
//! backticks, and counts words.

use std::ops::Range;

/// The paragraph style of one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Body,
    /// `# `
    Title,
    /// `## `
    Heading,
    /// `### `
    Subheading,
    /// `- ` or `* `
    Bullet,
    /// `1. `
    Numbered(u32),
    /// `- [ ] ` (false) or `- [x] ` (true)
    Check(bool),
    /// `– ` (an en dash and a space): the dashed list
    Dashed,
    /// a monostyled line, marked by a leading three-backtick fence and a space
    Mono,
    /// `> `: a block quote line
    Quote,
}

impl Block {
    fn same_kind(self, other: Block) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// A line's block style and the number of leading chars (indent and marker)
/// that belong to the marker rather than the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineInfo {
    pub block: Block,
    pub prefix: usize,
    pub indent: usize,
}

/// Classifies one line (no newline).
pub fn classify(line: &str) -> LineInfo {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|&&c| c == ' ').count();
    let rest = &chars[indent..];
    let starts = |p: &str| rest.len() >= p.chars().count() && rest.iter().zip(p.chars()).all(|(a, b)| *a == b);
    let info = |block, len: usize, ind: usize| LineInfo { block, prefix: ind + len, indent: ind };
    if starts("### ") {
        return info(Block::Subheading, 4, 0);
    }
    if starts("## ") {
        return info(Block::Heading, 3, 0);
    }
    if starts("# ") {
        return info(Block::Title, 2, 0);
    }
    if starts("``` ") {
        return LineInfo { block: Block::Mono, prefix: 4, indent: 0 };
    }
    if starts("> ") {
        return LineInfo { block: Block::Quote, prefix: 2, indent: 0 };
    }
    if starts("\u{2013} ") {
        return info(Block::Dashed, 2, indent);
    }
    if starts("- [ ] ") {
        return info(Block::Check(false), 6, indent);
    }
    if starts("- [x] ") || starts("- [X] ") {
        return info(Block::Check(true), 6, indent);
    }
    if starts("- ") || starts("* ") {
        return info(Block::Bullet, 2, indent);
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 6 && rest.get(digits) == Some(&'.') && rest.get(digits + 1) == Some(&' ') {
        let n: u32 = rest[..digits].iter().collect::<String>().parse().unwrap_or(1);
        return info(Block::Numbered(n), digits + 2, indent);
    }
    LineInfo { block: Block::Body, prefix: 0, indent: 0 }
}

/// Inline emphasis of a span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Bold,
    Italic,
    Code,
    /// `~~text~~`
    Strike,
    /// `++text++`
    Underline,
    /// `==text==`
    Highlight,
}

/// A run of chars (indices within the line) with one style. `marker` spans
/// are the `**`, `*` or backtick delimiters themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
    pub marker: bool,
}

fn find(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(pat.len() - 1)).find(|&i| chars[i..i + pat.len()] == *pat)
}

/// Splits `line[start..]` into spans that tile it exactly.
pub fn spans(line: &str, start: usize) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut out = vec![];
    let (mut i, mut run) = (start.min(n), start.min(n));
    let flush = |out: &mut Vec<Span>, from: usize, to: usize| {
        if to > from {
            out.push(Span { range: from..to, style: Style::Plain, marker: false });
        }
    };
    while i < n {
        let (open, style) = match chars[i] {
            '`' => (1, Style::Code),
            '*' if chars.get(i + 1) == Some(&'*') => (2, Style::Bold),
            '*' if chars.get(i + 1).is_some_and(|c| *c != ' ' && *c != '*') => (1, Style::Italic),
            '~' if chars.get(i + 1) == Some(&'~') => (2, Style::Strike),
            '+' if chars.get(i + 1) == Some(&'+') => (2, Style::Underline),
            '=' if chars.get(i + 1) == Some(&'=') => (2, Style::Highlight),
            _ => {
                i += 1;
                continue;
            }
        };
        let pat: Vec<char> = chars[i..i + open].to_vec();
        let close = find(&chars, i + open + 1, &pat).filter(|&j| style != Style::Italic || chars[j - 1] != ' ');
        let Some(j) = close else {
            i += open;
            continue;
        };
        flush(&mut out, run, i);
        out.push(Span { range: i..i + open, style, marker: true });
        out.push(Span { range: i + open..j, style, marker: false });
        out.push(Span { range: j..j + open, style, marker: true });
        i = j + open;
        run = i;
    }
    flush(&mut out, run, n);
    out
}

/// What Enter does at the end of a list line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Continue {
    /// not a list line: a plain newline
    None,
    /// insert this after the newline (next marker)
    Marker(String),
    /// the item is empty: remove its marker (this many chars) and end the list
    End(usize),
}

pub fn continue_list(line: &str) -> Continue {
    let info = classify(line);
    let chars: Vec<char> = line.chars().collect();
    let empty = chars[info.prefix.min(chars.len())..].iter().all(|c| c.is_whitespace());
    let pad = " ".repeat(info.indent);
    match info.block {
        Block::Bullet | Block::Check(_) | Block::Numbered(_) | Block::Dashed | Block::Quote | Block::Mono if empty => Continue::End(chars.len()),
        Block::Dashed => Continue::Marker(format!("{pad}\u{2013} ")),
        Block::Quote => Continue::Marker("> ".into()),
        Block::Mono => Continue::Marker("``` ".into()),
        Block::Bullet => Continue::Marker(format!("{pad}{} ", chars[info.indent])),
        Block::Check(_) => Continue::Marker(format!("{pad}- [ ] ")),
        Block::Numbered(n) => Continue::Marker(format!("{pad}{}. ", n + 1)),
        _ => Continue::None,
    }
}

/// The line with its checkbox flipped; None when it is not a checklist item.
pub fn toggle_check(line: &str) -> Option<String> {
    let info = classify(line);
    let Block::Check(done) = info.block else { return None };
    let mut chars: Vec<char> = line.chars().collect();
    chars[info.indent + 3] = if done { ' ' } else { 'x' };
    Some(chars.into_iter().collect())
}

/// Within every run of consecutive checklist lines, moves the checked ones
/// below the unchecked ones (stable).
pub fn checked_to_bottom(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if matches!(classify(lines[i]).block, Block::Check(_)) {
            let end = (i..lines.len()).find(|&j| !matches!(classify(lines[j]).block, Block::Check(_))).unwrap_or(lines.len());
            let run = &lines[i..end];
            out.extend(run.iter().filter(|l| classify(l).block == Block::Check(false)));
            out.extend(run.iter().filter(|l| classify(l).block == Block::Check(true)));
            i = end;
        } else {
            out.push(lines[i]);
            i += 1;
        }
    }
    out.join("\n")
}

/// Rewrites a line to `block` (a line already in that style goes back to
/// body text). Content and emphasis are kept.
pub fn set_block(line: &str, block: Block) -> String {
    let info = classify(line);
    let content: String = line.chars().skip(info.prefix).collect();
    let target = if info.block.same_kind(block) { Block::Body } else { block };
    let prefix = match target {
        Block::Body => String::new(),
        Block::Title => "# ".into(),
        Block::Heading => "## ".into(),
        Block::Subheading => "### ".into(),
        Block::Bullet => "- ".into(),
        Block::Numbered(_) => "1. ".into(),
        Block::Check(_) => "- [ ] ".into(),
        Block::Dashed => "\u{2013} ".into(),
        Block::Mono => "``` ".into(),
        Block::Quote => "> ".into(),
    };
    format!("{prefix}{content}")
}

/// `set_block` for every line of `text`; numbered lists count 1, 2, 3...
pub fn set_block_lines(text: &str, block: Block) -> String {
    let mut n = 0;
    text.split('\n')
        .map(|line| {
            let out = set_block(line, block);
            let info = classify(&out);
            if matches!(block, Block::Numbered(_)) && matches!(info.block, Block::Numbered(_)) {
                n += 1;
                let content: String = out.chars().skip(info.prefix).collect();
                return format!("{n}. {content}");
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The smallest single replacement turning `old` into `new`: the char range
/// of `old` to swap and the text to put there. Lets whole-note rewrites
/// (sorting checked items) land as one small undo step.
pub fn diff_range(old: &str, new: &str) -> (Range<usize>, String) {
    let (o, n): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let prefix = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let suffix = o[prefix..].iter().rev().zip(n[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    (prefix..o.len() - suffix, n[prefix..n.len() - suffix].iter().collect())
}

/// A text replacement: swap `range` (char indices) for `with` and select
/// `select` afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replace {
    pub range: Range<usize>,
    pub with: String,
    pub select: Range<usize>,
}

/// Wraps the selection in `marker` (`**`, `*`, `` ` ``), or unwraps it when
/// it is already wrapped. An empty selection inserts a pair around the caret.
pub fn toggle_wrap(text: &str, sel: Range<usize>, marker: &str) -> Replace {
    let chars: Vec<char> = text.chars().collect();
    let m: Vec<char> = marker.chars().collect();
    let ml = m.len();
    let (a, b) = (sel.start.min(chars.len()), sel.end.min(chars.len()));
    let before = a >= ml && chars[a - ml..a] == m[..];
    let after = b + ml <= chars.len() && chars[b..b + ml] == m[..];
    let guard = ml == 1 && marker == "*" && ((a >= 2 && chars[a - 2] == '*') || chars.get(b + 1) == Some(&'*'));
    if before && after && !guard {
        let inner: String = chars[a..b].iter().collect();
        return Replace { range: a - ml..b + ml, with: inner, select: a - ml..b - ml };
    }
    let inner: String = chars[a..b].iter().collect();
    Replace { range: a..b, with: format!("{marker}{inner}{marker}"), select: a + ml..b + ml }
}

/// Words in a note: runs of non-space chars holding a letter or digit
/// (list markers and rules do not count).
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// `(words, chars)` for the editor's counter.
pub fn counts(text: &str) -> (usize, usize) {
    (word_count(text), text.chars().filter(|c| !c.is_control()).count())
}

/// A line that is only an image: `![alt](file)`; returns the file name.
pub fn image_ref(line: &str) -> Option<&str> {
    let t = line.trim();
    let rest = t.strip_prefix("![")?;
    let (_, tail) = rest.split_once("](")?;
    let name = tail.strip_suffix(')')?;
    (!name.is_empty() && !name.contains('/') && !name.contains("..") && !name.contains(')')).then_some(name)
}

/// The Markdown for an attached image.
pub fn image_line(name: &str) -> String {
    format!("![image]({name})")
}

/// Plain text of a note: markers stripped (used for the text export).
pub fn to_plain(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let info = classify(line);
            let content: String = line.chars().skip(info.prefix).collect();
            let body: String = spans(&content, 0).into_iter().filter(|s| !s.marker).flat_map(|s| content.chars().skip(s.range.start).take(s.range.len()).collect::<Vec<_>>()).collect();
            match info.block {
                Block::Check(done) => format!("{} {body}", if done { "[x]" } else { "[ ]" }),
                Block::Bullet => format!("\u{2022} {body}"),
                Block::Dashed => format!("\u{2013} {body}"),
                Block::Numbered(n) => format!("{n}. {body}"),
                _ => body,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_block_styles() {
        assert_eq!(classify("plain").block, Block::Body);
        assert_eq!(classify("# T"), LineInfo { block: Block::Title, prefix: 2, indent: 0 });
        assert_eq!(classify("## H").block, Block::Heading);
        assert_eq!(classify("### S").prefix, 4);
        assert_eq!(classify("- item").block, Block::Bullet);
        assert_eq!(classify("* item").block, Block::Bullet);
        assert_eq!(classify("- [ ] todo"), LineInfo { block: Block::Check(false), prefix: 6, indent: 0 });
        assert_eq!(classify("- [x] done").block, Block::Check(true));
        assert_eq!(classify("  - [X] nested"), LineInfo { block: Block::Check(true), prefix: 8, indent: 2 });
        assert_eq!(classify("12. twelve"), LineInfo { block: Block::Numbered(12), prefix: 4, indent: 0 });
        assert_eq!(classify("-nospace").block, Block::Body);
        assert_eq!(classify("1.5 miles").block, Block::Body);
        assert_eq!(classify("#hashtag").block, Block::Body);
    }

    #[test]
    fn spans_tile_the_line_and_mark_delimiters() {
        let s = spans("a **b** *c* `d` e", 0);
        let styles: Vec<(Style, bool)> = s.iter().map(|x| (x.style, x.marker)).collect();
        assert_eq!(styles[0], (Style::Plain, false));
        assert_eq!(styles[1], (Style::Bold, true));
        assert_eq!(styles[2], (Style::Bold, false));
        assert_eq!(s[2].range, 4..5);
        assert!(s.iter().any(|x| x.style == Style::Italic && !x.marker && x.range == (9..10)));
        assert!(s.iter().any(|x| x.style == Style::Code && !x.marker));
        let total: usize = s.iter().map(|x| x.range.len()).sum();
        assert_eq!(total, "a **b** *c* `d` e".chars().count());
        for w in s.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start, "contiguous");
        }
    }

    #[test]
    fn unmatched_markers_stay_plain() {
        for line in ["2 * 3 = 6", "**open", "a * b * c", "`tick", "*", ""] {
            assert!(spans(line, 0).iter().all(|s| s.style == Style::Plain && !s.marker), "{line}");
        }
        let s = spans("- [ ] **x**", 6);
        assert_eq!(s[0].range.start, 6);
        assert_eq!(spans("abc", 9), vec![]);
    }

    #[test]
    fn enter_continues_lists_and_ends_empty_items() {
        assert_eq!(continue_list("- milk"), Continue::Marker("- ".into()));
        assert_eq!(continue_list("* milk"), Continue::Marker("* ".into()));
        assert_eq!(continue_list("  - sub"), Continue::Marker("  - ".into()));
        assert_eq!(continue_list("3. c"), Continue::Marker("4. ".into()));
        assert_eq!(continue_list("- [x] done"), Continue::Marker("- [ ] ".into()));
        assert_eq!(continue_list("- "), Continue::End(2));
        assert_eq!(continue_list("- [ ] "), Continue::End(6));
        assert_eq!(continue_list("2. "), Continue::End(3));
        assert_eq!(continue_list("just text"), Continue::None);
        assert_eq!(continue_list("## head"), Continue::None);
        assert_eq!(continue_list(""), Continue::None);
    }

    #[test]
    fn toggles_checkboxes_both_ways() {
        assert_eq!(toggle_check("- [ ] a").as_deref(), Some("- [x] a"));
        assert_eq!(toggle_check("- [x] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_check("- [X] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_check("  - [ ] a").as_deref(), Some("  - [x] a"));
        assert_eq!(toggle_check("- a"), None);
        assert_eq!(toggle_check("é"), None);
    }

    #[test]
    fn checked_items_sink_within_their_run_only() {
        let text = "Title\n- [x] a\n- [ ] b\n- [x] c\n- [ ] d\ntext\n- [x] e\n- [ ] f\n";
        assert_eq!(checked_to_bottom(text), "Title\n- [ ] b\n- [ ] d\n- [x] a\n- [x] c\ntext\n- [ ] f\n- [x] e\n");
        assert_eq!(checked_to_bottom("no lists"), "no lists");
        assert_eq!(checked_to_bottom(&checked_to_bottom(text)), checked_to_bottom(text), "idempotent");
    }

    #[test]
    fn set_block_swaps_prefixes_and_toggles_off() {
        assert_eq!(set_block("hello", Block::Heading), "## hello");
        assert_eq!(set_block("## hello", Block::Title), "# hello");
        assert_eq!(set_block("## hello", Block::Heading), "hello");
        assert_eq!(set_block("- [x] hi **b**", Block::Bullet), "- hi **b**");
        assert_eq!(set_block("- hi", Block::Check(false)), "- [ ] hi");
        assert_eq!(set_block("3. hi", Block::Numbered(1)), "hi");
        assert_eq!(set_block("hi", Block::Numbered(1)), "1. hi");
    }

    #[test]
    fn block_changes_apply_to_every_selected_line() {
        assert_eq!(set_block_lines("a\nb\nc", Block::Numbered(1)), "1. a\n2. b\n3. c");
        assert_eq!(set_block_lines("- a\n- b", Block::Bullet), "a\nb", "toggles off");
        assert_eq!(set_block_lines("a\n- b", Block::Check(false)), "- [ ] a\n- [ ] b");
        assert_eq!(set_block_lines("x", Block::Heading), "## x");
    }

    #[test]
    fn diff_range_is_the_minimal_replacement() {
        assert_eq!(diff_range("abc", "abc"), (3..3, String::new()));
        assert_eq!(diff_range("abcd", "abXd"), (2..3, "X".into()));
        assert_eq!(diff_range("ab", "abcd"), (2..2, "cd".into()));
        assert_eq!(diff_range("aXa", "aa"), (1..2, String::new()));
        assert_eq!(diff_range("", "x"), (0..0, "x".into()));
        let (r, with) = diff_range("T\n- [x] a\n- [ ] b", "T\n- [ ] b\n- [x] a");
        let mut chars: Vec<char> = "T\n- [x] a\n- [ ] b".chars().collect();
        chars.splice(r, with.chars());
        assert_eq!(chars.into_iter().collect::<String>(), "T\n- [ ] b\n- [x] a");
    }

    #[test]
    fn wrap_toggles_selection_markers() {
        let r = toggle_wrap("a word b", 2..6, "**");
        assert_eq!(r, Replace { range: 2..6, with: "**word**".into(), select: 4..8 });
        let r = toggle_wrap("a **word** b", 4..8, "**");
        assert_eq!(r, Replace { range: 2..10, with: "word".into(), select: 2..6 });
        let r = toggle_wrap("ab", 1..1, "`");
        assert_eq!(r, Replace { range: 1..1, with: "``".into(), select: 2..2 });
        let r = toggle_wrap("x *i* y", 3..4, "*");
        assert_eq!(r.with, "i");
        let r = toggle_wrap("**b**", 2..3, "*");
        assert_eq!(r.with, "*b*", "italic does not unwrap half of a bold pair");
    }

    #[test]
    fn counts_words_without_markers() {
        assert_eq!(word_count("# Title\n- [ ] buy milk\n- eggs ---"), 4);
        assert_eq!(word_count(""), 0);
        assert_eq!(counts("ab cd\n").0, 2);
        assert_eq!(counts("ab cd\n").1, 5);
    }

    #[test]
    fn image_lines_round_trip_and_reject_paths() {
        assert_eq!(image_ref("![image](1-2.png)"), Some("1-2.png"));
        assert_eq!(image_ref(&image_line("a.jpg")), Some("a.jpg"));
        assert_eq!(image_ref("![x](../../etc/passwd)"), None);
        assert_eq!(image_ref("![x](a/b.png)"), None);
        assert_eq!(image_ref("text ![x](a.png)"), None);
        assert_eq!(image_ref("![x]()"), None);
    }

    #[test]
    fn dashed_mono_and_quote_lines_are_blocks() {
        assert_eq!(classify("\u{2013} item"), LineInfo { block: Block::Dashed, prefix: 2, indent: 0 });
        assert_eq!(classify("  \u{2013} nested"), LineInfo { block: Block::Dashed, prefix: 4, indent: 2 });
        assert_eq!(classify("``` let x = 1;"), LineInfo { block: Block::Mono, prefix: 4, indent: 0 });
        assert_eq!(classify("> said"), LineInfo { block: Block::Quote, prefix: 2, indent: 0 });
        assert_eq!(classify(">no space").block, Block::Body);
        assert_eq!(classify("```rust").block, Block::Body, "a bare fence is not a monostyled line");
    }

    #[test]
    fn the_new_blocks_toggle_continue_and_export() {
        assert_eq!(set_block("hello", Block::Dashed), "\u{2013} hello");
        assert_eq!(set_block("\u{2013} hello", Block::Dashed), "hello");
        assert_eq!(set_block("- hello", Block::Quote), "> hello");
        assert_eq!(set_block("## hello", Block::Mono), "``` hello");
        assert_eq!(continue_list("\u{2013} a"), Continue::Marker("\u{2013} ".into()));
        assert_eq!(continue_list("> a"), Continue::Marker("> ".into()));
        assert_eq!(continue_list("> "), Continue::End(2));
        assert_eq!(continue_list("``` "), Continue::End(4));
        assert_eq!(to_plain("\u{2013} a\n> b\n``` c"), "\u{2013} a\nb\nc");
    }

    #[test]
    fn strike_underline_and_highlight_spans() {
        let s = spans("a ~~b~~ ++c++ ==d==", 0);
        let styles: Vec<Style> = s.iter().filter(|x| !x.marker && x.style != Style::Plain).map(|x| x.style).collect();
        assert_eq!(styles, vec![Style::Strike, Style::Underline, Style::Highlight]);
        let total: usize = s.iter().map(|x| x.range.len()).sum();
        assert_eq!(total, "a ~~b~~ ++c++ ==d==".chars().count());
        for line in ["~~open", "a == b", "1 ++ 2"] {
            assert!(spans(line, 0).iter().all(|x| x.style == Style::Plain), "{line}");
        }
    }

    #[test]
    fn plain_export_strips_markup() {
        assert_eq!(to_plain("# T\n- [x] **a** b\n- c\n2. d `e`"), "T\n[x] a b\n\u{2022} c\n2. d e");
    }
}
