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
    /// an en dash and a space (`\u{2013} `)
    Dashed,
    /// four leading spaces (an indented code line)
    Mono,
    /// `> `
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
    // a line has one paragraph style: a checkbox marker after a heading marker (`## - [ ] Item`, as a checklist line
    // turned into a heading used to be written) belongs to the heading's marker, not its text (`stray_check`)
    let heading = |block, len: usize| info(block, len + check_marker_len(&rest[len..]), 0);
    if starts("### ") {
        return heading(Block::Subheading, 4);
    }
    if starts("## ") {
        return heading(Block::Heading, 3);
    }
    if starts("# ") {
        return heading(Block::Title, 2);
    }
    if starts("> ") && indent == 0 {
        return info(Block::Quote, 2, 0);
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
    if starts("\u{2013} ") {
        return info(Block::Dashed, 2, indent);
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 6 && rest.get(digits) == Some(&'.') && rest.get(digits + 1) == Some(&' ') {
        let n: u32 = rest[..digits].iter().collect::<String>().parse().unwrap_or(1);
        return info(Block::Numbered(n), digits + 2, indent);
    }
    if indent >= MONO_INDENT && rest.iter().any(|c| !c.is_whitespace()) {
        return LineInfo { block: Block::Mono, prefix: MONO_INDENT, indent: 0 };
    }
    LineInfo { block: Block::Body, prefix: 0, indent: 0 }
}

/// Leading spaces that make a line monostyled.
const MONO_INDENT: usize = 4;

/// The length of a checkbox marker (`- [ ] `, `- [x] `, `- [X] `) at the start of `rest`, else 0.
fn check_marker_len(rest: &[char]) -> usize {
    let is = |p: &str| rest.len() >= 6 && rest.iter().zip(p.chars()).all(|(a, b)| *a == b);
    if is("- [ ] ") || is("- [x] ") || is("- [X] ") {
        6
    } else {
        0
    }
}

/// The chars of a checkbox marker that follows a heading marker (`## - [ ] Item two`): the line is a heading (the
/// first marker decides, and the reference keeps a paragraph in one style, so a heading is never also a checklist
/// item), and this part of its marker is not drawn at all, so the heading reads "Item two". Empty for any other line.
pub fn stray_check(line: &str) -> Range<usize> {
    let info = classify(line);
    let marker = match info.block {
        Block::Title => 2,
        Block::Heading => 3,
        Block::Subheading => 4,
        _ => return 0..0,
    };
    marker.min(info.prefix)..info.prefix
}

/// The number of leading chars of a heading line (`# `, `## `, `### `, with a checkbox marker after it) that are its
/// marker: the editor does not draw them, as the reference's headings carry their style without marker characters.
/// 0 for any other line.
pub fn heading_marker(line: &str) -> usize {
    let info = classify(line);
    match info.block {
        Block::Title | Block::Heading | Block::Subheading => info.prefix,
        _ => 0,
    }
}

/// One line as the note list's snippet shows it: its block marker (heading, list, checklist, quote, code indent) and
/// its emphasis markers taken out, so `## - [ ] Item` and `> **said**` read `Item` and `said`.
pub fn plain_line(line: &str) -> String {
    let content: String = line.chars().skip(classify(line).prefix).collect();
    strip_inline(&content)
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
    /// `$tex$`: inline math, typeset in place of its source
    Math,
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
            // a lone dollar opens inline math; `$$` is display math, handled per paragraph
            '$' if chars.get(i + 1).is_some_and(|c| *c != '$' && *c != ' ') && (i == 0 || chars[i - 1] != '$') => {
                (1, Style::Math)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let pat: Vec<char> = chars[i..i + open].to_vec();
        let close = find(&chars, i + open + 1, &pat).filter(|&j| match style {
            Style::Italic => chars[j - 1] != ' ',
            // a pair of these marks must hug its text, so "i++ and j++" and "a == b" stay plain
            Style::Strike | Style::Underline | Style::Highlight => !chars[i + 2].is_whitespace() && !chars[j - 1].is_whitespace(),
            Style::Math => !chars[j - 1].is_whitespace(),
            _ => true,
        });
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

/// The run of `style` whose text holds the char boundary `c` of `line` (inline content from `start`, as `spans` reads
/// it), at either end of the text included: the text's char range and its marker's length. `None` when `c` is not in
/// such a run.
pub fn emphasis_at(line: &str, start: usize, c: usize, style: Style) -> Option<(Range<usize>, usize)> {
    let all = spans(line, start);
    let k = all.iter().position(|s| s.style == style && !s.marker && s.range.start <= c && c <= s.range.end)?;
    let marker = k.checked_sub(1).and_then(|m| all.get(m)).filter(|m| m.marker)?;
    Some((all[k].range.clone(), marker.range.len()))
}

/// One line's text with its emphasis markers taken out (`**b**` reads `b`), for titles and previews.
pub fn strip_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    spans(line, 0).into_iter().filter(|s| !s.marker).flat_map(|s| chars[s.range].to_vec()).collect()
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
        Block::Bullet | Block::Check(_) | Block::Numbered(_) if empty => Continue::End(chars.len()),
        Block::Dashed | Block::Quote if empty => Continue::End(chars.len()),
        Block::Bullet => Continue::Marker(format!("{pad}{} ", chars[info.indent])),
        Block::Dashed => Continue::Marker(format!("{pad}\u{2013} ")),
        Block::Quote => Continue::Marker("> ".to_string()),
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

/// Ticks (`done`) or unticks every checklist item of `text`; other lines are untouched.
pub fn set_all_checks(text: &str, done: bool) -> String {
    text.split('\n')
        .map(|line| {
            let info = classify(line);
            match info.block {
                Block::Check(now) if now != done => toggle_check(line).unwrap_or_else(|| line.to_string()),
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        Block::Mono => " ".repeat(MONO_INDENT),
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

/// Where `marker` already wraps `chars[s..e]`: the outer range (markers included) and the inner text's range. Each
/// marker may sit just outside the range or be its own first or last chars (a selection that runs across a line end
/// takes in the closing marker of that line). A lone `*` never matches half of `**`.
fn wrapped_by(chars: &[char], s: usize, e: usize, m: &[char]) -> Option<(Range<usize>, Range<usize>)> {
    let ml = m.len();
    let at = |i: usize| i + ml <= chars.len() && chars[i..i + ml] == *m;
    let star = ml == 1 && m[0] == '*';
    // for `*`: no star just before `l` or at `r`, so the marker at `l` is not one half of `**`
    let lone = |l: usize, r: usize| !star || ((l == 0 || chars[l - 1] != '*') && chars.get(r) != Some(&'*'));
    let left = if s >= ml && at(s - ml) && lone(s - ml, s) {
        (s - ml, s)
    } else if at(s) && lone(s, s + ml) {
        (s, s + ml)
    } else {
        return None;
    };
    let right = if at(e) && lone(e, e + ml) {
        (e, e + ml)
    } else if e >= ml && at(e - ml) && lone(e - ml, e) {
        (e - ml, e)
    } else {
        return None;
    };
    (left.1 <= right.0 && left.0 < right.0).then(|| (left.0..right.1, left.1..right.0))
}

/// Cmd+B (and the other inline styles) on a selection: Markdown emphasis lives inside one line, so a selection that
/// covers several lines is styled line by line. Each line's part of the selection, without its block marker (`# `,
/// `- [ ] `, ...) and without the spaces at its ends, is wrapped in `marker`; blank parts are left alone. When every
/// part is already wrapped the markers come off instead. `select` covers the styled text from the first part to the
/// last, so pressing the key again toggles back. Within one line this is `toggle_wrap` with the block marker kept
/// outside the emphasis.
pub fn toggle_wrap_lines(text: &str, sel: Range<usize>, marker: &str) -> Replace {
    let chars: Vec<char> = text.chars().collect();
    let m: Vec<char> = marker.chars().collect();
    let ml = m.len();
    let (a, b) = (sel.start.min(chars.len()), sel.end.min(chars.len()));
    let (a, b) = (a.min(b), a.max(b));
    if a == b {
        return toggle_wrap(text, a..b, marker);
    }
    // the lines the selection touches, and each line's styled part
    let first = chars[..a].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
    let last = chars[b..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| b + i);
    let mut parts = vec![];
    let mut ls = first;
    while ls <= last {
        let le = chars[ls..last].iter().position(|c| *c == '\n').map_or(last, |i| ls + i);
        let line: String = chars[ls..le].iter().collect();
        let mut s = a.max(ls + classify(&line).prefix.min(le - ls));
        let mut e = b.min(le);
        while s < e && chars[s].is_whitespace() {
            s += 1;
        }
        while e > s && chars[e - 1].is_whitespace() {
            e -= 1;
        }
        if s < e {
            parts.push((s, e));
        }
        ls = le + 1;
    }
    if parts.is_empty() {
        // nothing to style: hand back the touched lines unchanged, like every other result, and keep the selection
        return Replace { range: first..last, with: chars[first..last].iter().collect(), select: a..b };
    }
    let found: Vec<_> = parts.iter().map(|&(s, e)| wrapped_by(&chars, s, e, &m)).collect();
    let unwrap = found.iter().all(Option::is_some);
    // rebuild the touched lines, mapping the first part's start and the last part's end into the new text
    let mut with = String::new();
    let (mut at, mut len) = (first, 0usize);
    let (mut sel_a, mut sel_b) = (first, first);
    for (k, &(s, e)) in parts.iter().enumerate() {
        // a part that is already styled is left as it is when the others get the style
        let wrap = !unwrap && found[k].is_none();
        let (outer, inner) = match &found[k] {
            Some(f) if unwrap => f.clone(),
            _ => (s..e, s..e),
        };
        let keep: String = chars[at..outer.start].iter().collect();
        len += keep.chars().count();
        with.push_str(&keep);
        if k == 0 {
            sel_a = first + len + if wrap { ml } else { 0 };
        }
        let body: String = chars[inner.clone()].iter().collect();
        if wrap {
            len += inner.len() + 2 * ml;
            with.push_str(&format!("{marker}{body}{marker}"));
            sel_b = first + len - ml;
        } else {
            len += inner.len();
            with.push_str(&body);
            sel_b = first + len;
        }
        at = outer.end;
    }
    with.extend(chars[at..last].iter());
    Replace { range: first..last, with, select: sel_a..sel_b }
}

/// A web address as it goes into a link: a missing scheme becomes `https://`, spaces and brackets are escaped. `None`
/// when nothing is left after the scheme.
fn link_target(raw: &str) -> Option<String> {
    let t = raw.trim();
    let full = if t.contains("://") || t.starts_with("mailto:") { t.to_string() } else { format!("https://{t}") };
    let after = full.split_once("://").map_or_else(|| full.strip_prefix("mailto:").unwrap_or(""), |(_, rest)| rest);
    if after.trim().is_empty() {
        return None;
    }
    let mut out = String::new();
    for c in full.chars().filter(|c| !c.is_control()) {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            c => out.push(c),
        }
    }
    Some(out)
}

/// Add Link: wraps the selection as `[text](address)`. With nothing selected the address itself, without its scheme,
/// is the text. `select` covers the whole link. `None` when the address is empty.
pub fn link_markdown(text: &str, sel: Range<usize>, url: &str) -> Option<Replace> {
    let url = link_target(url)?;
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (sel.start.min(chars.len()), sel.end.min(chars.len()).max(sel.start.min(chars.len())));
    let inner: String = chars[a..b].iter().filter(|c| !matches!(**c, '[' | ']' | '\n')).collect();
    let label = if inner.trim().is_empty() { url.split_once("://").map_or_else(|| url.clone(), |(_, rest)| rest.to_string()) } else { inner };
    let with = format!("[{label}]({url})");
    let len = with.chars().count();
    Some(Replace { range: a..b, with, select: a..a + len })
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
            let body = strip_inline(&content);
            match info.block {
                Block::Check(done) => format!("{} {body}", if done { "[x]" } else { "[ ]" }),
                Block::Bullet => format!("\u{2022} {body}"),
                Block::Numbered(n) => format!("{n}. {body}"),
                Block::Dashed => format!("\u{2013} {body}"),
                _ => body,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E2E11 11: `## - [ ] Item two` drew as a heading with the raw checkbox marker in it.
    #[test]
    fn a_heading_is_never_also_a_checklist_item() {
        let line = "## - [ ] Item two";
        assert_eq!(classify(line), LineInfo { block: Block::Heading, prefix: 9, indent: 0 }, "the heading wins, the checkbox is marker");
        assert_eq!(stray_check(line), 3..9, "the checkbox part of the marker is hidden");
        assert_eq!(stray_check("# - [x] Done"), 2..8);
        assert_eq!(stray_check("### - [X] Sub"), 4..10);
        assert_eq!(stray_check("## Plain heading"), 3..3, "nothing hidden in a plain heading");
        assert_eq!(stray_check("- [ ] item"), 0..0, "a checklist item keeps its checkbox");
        assert_eq!(stray_check("## 1. Introduction").len(), 0, "a numbered heading keeps its number");
        assert_eq!(toggle_check(line), None, "no checkbox to tick on a heading");
        assert_eq!(continue_list(line), Continue::None, "Enter after a heading starts body text");
        assert_eq!(to_plain(line), "Item two");
        assert_eq!(set_block(line, Block::Body), "Item two", "the style change drops the whole marker");
        assert_eq!(set_block(line, Block::Check(false)), "- [ ] Item two");
    }

    #[test]
    fn heading_markers_are_hidden_and_snippets_drop_block_markers() {
        // E2E12 finding 3: the editor drew the `##` of every heading in grey and the list snippet showed it raw
        assert_eq!(heading_marker("## heading two"), 3);
        assert_eq!(heading_marker("# Title"), 2);
        assert_eq!(heading_marker("### Sub"), 4);
        assert_eq!(heading_marker("## - [ ] item"), 9, "a stray checkbox marker is part of the heading's marker");
        assert_eq!(heading_marker("- [ ] item"), 0);
        assert_eq!(heading_marker("#hashtag"), 0);
        assert_eq!(plain_line("## heading two"), "heading two");
        assert_eq!(plain_line("## - [ ] item"), "item");
        assert_eq!(plain_line("- [x] done"), "done");
        assert_eq!(plain_line("- milk"), "milk");
        assert_eq!(plain_line("3. third"), "third");
        assert_eq!(plain_line("> **said**"), "said");
        assert_eq!(plain_line("\u{2013} dash"), "dash");
        assert_eq!(plain_line("plain *it*"), "plain it");
    }

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
    fn wrapping_a_multi_line_selection_styles_each_line_inside_its_block_marker() {
        // Cmd+A, Cmd+B (E2E W2-2): every line becomes bold on its own, the title keeps its "# " outside the markers
        let text = "# Title\n\nbody text\n- item ";
        let r = toggle_wrap_lines(text, 0..text.chars().count(), "**");
        assert_eq!(r.range, 0..text.chars().count());
        assert_eq!(r.with, "# **Title**\n\n**body text**\n- **item** ");
        let chars: Vec<char> = r.with.chars().collect();
        assert_eq!(chars[r.select.clone()].iter().collect::<String>(), "Title**\n\n**body text**\n- **item");
        for line in r.with.lines().filter(|l| !l.trim().is_empty()) {
            let content: String = line.chars().skip(classify(line).prefix).collect();
            assert!(spans(&content, 0).iter().any(|s| s.style == Style::Bold && !s.marker), "{line} is bold");
        }
        assert_eq!(classify(r.with.lines().next().unwrap()).block, Block::Title, "the title is still a title");
        // pressing it again with that selection takes the markers off
        let back = toggle_wrap_lines(&r.with, r.select.clone(), "**");
        assert_eq!(back.with, text);
        // a part that is already bold stays as it is while the rest gets the style
        let mixed = toggle_wrap_lines("**a**\nb", 0..7, "**");
        assert_eq!(mixed.with, "**a**\n**b**");
    }

    #[test]
    fn emphasis_at_finds_the_styled_run_under_the_caret_and_strip_inline_drops_markers() {
        // "a **bold** c": the bold text is chars 4..8 with a 2-char marker
        assert_eq!(emphasis_at("a **bold** c", 0, 8, Style::Bold), Some((4..8, 2)), "at its end");
        assert_eq!(emphasis_at("a **bold** c", 0, 6, Style::Bold), Some((4..8, 2)), "inside");
        assert_eq!(emphasis_at("a **bold** c", 0, 2, Style::Bold), None, "before the markers");
        assert_eq!(emphasis_at("a **bold** c", 0, 10, Style::Bold), None, "after them");
        assert_eq!(emphasis_at("a **bold** c", 0, 6, Style::Italic), None, "another style");
        assert_eq!(emphasis_at("x *i* y", 0, 3, Style::Italic), Some((3..4, 1)));
        assert_eq!(strip_inline("**E2E bold test** plain"), "E2E bold test plain");
        assert_eq!(strip_inline("a ~~b~~ `c` ==d=="), "a b c d");
        assert_eq!(strip_inline("i++ and 2 * 3"), "i++ and 2 * 3", "unpaired marks stay");
    }

    #[test]
    fn wrapping_within_one_line_matches_toggle_wrap() {
        let r = toggle_wrap_lines("a word b", 2..6, "**");
        assert_eq!((r.range, r.with.as_str(), r.select), (0..8, "a **word** b", 4..8));
        let r = toggle_wrap_lines("a **word** b", 4..8, "**");
        assert_eq!((r.with.as_str(), r.select), ("a word b", 2..6));
        let r = toggle_wrap_lines("## Heading", 0..10, "*");
        assert_eq!(r.with, "## *Heading*", "the heading marker stays outside");
        let r = toggle_wrap_lines("x **b** y", 4..5, "*");
        assert_eq!(r.with, "x ***b*** y", "italic does not unwrap half of a bold pair");
        let r = toggle_wrap_lines("a\n\nb", 1..3, "**");
        assert_eq!((r.range, r.with.as_str(), r.select), (0..4, "a\n\nb", 1..3), "a selection of blank lines changes nothing");
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
    fn plain_export_strips_markup() {
        assert_eq!(to_plain("# T\n- [x] **a** b\n- c\n2. d `e`"), "T\n[x] a b\n\u{2022} c\n2. d e");
    }
    #[test]
    fn dashed_quote_and_mono_lines_classify_and_round_trip() {
        assert_eq!(classify("\u{2013} item"), LineInfo { block: Block::Dashed, prefix: 2, indent: 0 });
        assert_eq!(classify("  \u{2013} nested"), LineInfo { block: Block::Dashed, prefix: 4, indent: 2 });
        assert_eq!(classify("> said"), LineInfo { block: Block::Quote, prefix: 2, indent: 0 });
        assert_eq!(classify("    let x = 1;"), LineInfo { block: Block::Mono, prefix: 4, indent: 0 });
        assert_eq!(classify("    ").block, Block::Body, "a blank indented line is not code");
        assert_eq!(classify("   three spaces").block, Block::Body);
        assert_eq!(classify("    - nested bullet").block, Block::Bullet, "a list marker wins over the code indent");
        assert_eq!(set_block("plain", Block::Quote), "> plain");
        assert_eq!(set_block("> plain", Block::Quote), "plain", "the same style again returns to body text");
        assert_eq!(set_block("plain", Block::Mono), "    plain");
        assert_eq!(set_block("    plain", Block::Mono), "plain");
        assert_eq!(set_block("- a", Block::Dashed), "\u{2013} a");
        assert_eq!(to_plain("\u{2013} a\n> q"), "\u{2013} a\nq");
    }

    #[test]
    fn enter_continues_dashed_and_quote_lines_and_ends_them_when_empty() {
        assert_eq!(continue_list("\u{2013} a"), Continue::Marker("\u{2013} ".into()));
        assert_eq!(continue_list("  \u{2013} a"), Continue::Marker("  \u{2013} ".into()));
        assert_eq!(continue_list("\u{2013} "), Continue::End(2));
        assert_eq!(continue_list("> a"), Continue::Marker("> ".into()));
        assert_eq!(continue_list("> "), Continue::End(2));
        assert_eq!(continue_list("    code"), Continue::None);
    }

    #[test]
    fn strike_underline_and_highlight_need_a_hugging_pair() {
        let kinds = |line: &str| -> Vec<(Style, bool, String)> {
            let chars: Vec<char> = line.chars().collect();
            spans(line, 0).into_iter().map(|s| (s.style, s.marker, chars[s.range.clone()].iter().collect())).collect()
        };
        assert_eq!(kinds("a ~~b~~ c"), vec![
            (Style::Plain, false, "a ".into()),
            (Style::Strike, true, "~~".into()),
            (Style::Strike, false, "b".into()),
            (Style::Strike, true, "~~".into()),
            (Style::Plain, false, " c".into()),
        ]);
        assert!(kinds("x ++u++").iter().any(|(s, m, t)| *s == Style::Underline && !*m && t == "u"));
        assert!(kinds("==hi== there").iter().any(|(s, m, t)| *s == Style::Highlight && !*m && t == "hi"));
        assert!(kinds("e = $x^2$ now").iter().any(|(s, m, t)| *s == Style::Math && !*m && t == "x^2"));
        assert!(kinds("cost $5 or $6").iter().all(|(s, _, _)| *s == Style::Plain), "a dollar before a space stays plain");
        for plain in ["i++ and j++", "a == b == c", "~~ spaced ~~", "one ~~ two"] {
            assert!(kinds(plain).iter().all(|(s, _, _)| *s == Style::Plain), "{plain} stays plain");
        }
        // the spans still tile the line exactly
        let line = "x ~~s~~ ==h== ++u++ **b**";
        let total: usize = spans(line, 0).iter().map(|s| s.range.len()).sum();
        assert_eq!(total, line.chars().count());
    }

    #[test]
    fn add_link_wraps_the_selection_or_the_address_itself() {
        let r = link_markdown("See docs here", 4..8, "example.com/a b").unwrap();
        assert_eq!(r.range, 4..8);
        assert_eq!(r.with, "[docs](https://example.com/a%20b)");
        assert_eq!(r.select, 4..4 + r.with.chars().count());
        let r = link_markdown("x", 1..1, "https://rust-lang.org").unwrap();
        assert_eq!((r.range, r.with.as_str()), (1..1, "[rust-lang.org](https://rust-lang.org)"));
        assert_eq!(link_markdown("go", 0..2, "  http://a.b/(x)  ").unwrap().with, "[go](http://a.b/%28x%29)");
        assert_eq!(link_markdown("[a]", 0..3, "a.b").unwrap().with, "[a](https://a.b)", "brackets in the text would break the link");
        assert_eq!(link_markdown("m", 0..1, "mailto:me@x.y").unwrap().with, "[m](mailto:me@x.y)");
        for empty in ["", "   ", "https://", "http://  "] {
            assert!(link_markdown("t", 0..1, empty).is_none(), "{empty:?} is no address");
        }
        // a selection past the end is clamped
        assert_eq!(link_markdown("ab", 1..9, "c.d").unwrap().range, 1..2);
    }

    #[test]
    fn mark_all_and_unmark_all_flip_only_the_checklist_lines() {
        let t = "T\n- [ ] a\n- [x] b\n  - [ ] c\n- plain\nbody";
        assert_eq!(set_all_checks(t, true), "T\n- [x] a\n- [x] b\n  - [x] c\n- plain\nbody");
        assert_eq!(set_all_checks(t, false), "T\n- [ ] a\n- [ ] b\n  - [ ] c\n- plain\nbody");
        assert_eq!(set_all_checks("no list", true), "no list");
    }
}
