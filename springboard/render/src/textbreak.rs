//! Text segmentation: grapheme cluster boundaries and line break opportunities.
//!
//! Both work on a slice of characters and answer per boundary: index `i` of the result is the boundary before
//! `chars[i]`, and index `chars.len()` is the end of the text.
//!
//! * `grapheme_boundaries` follows the extended grapheme cluster rules (CR LF, controls, combining marks, joiners,
//!   emoji sequences, regional indicator pairs, Hangul syllable sequences). Prepend characters and the full spacing
//!   mark table are not modelled.
//! * `line_breaks` follows the line breaking rules with a compact class table. Not modelled: dictionary breaks for
//!   Thai and similar scripts (they are treated as letters and never break), East Asian width distinctions, and the
//!   emoji base/modifier classes (emoji count as ideographs).

/// What may happen at a boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brk {
    /// The line must not break here.
    No,
    /// The line may break here.
    Allowed,
    /// The line must break here (after a hard line or paragraph break).
    Mandatory,
}

fn is_extend(c: char) -> bool {
    matches!(c as u32,
        0x300..=0x36f | 0x483..=0x489 | 0x591..=0x5bd | 0x5bf | 0x5c1 | 0x5c2 | 0x5c4 | 0x5c5 | 0x5c7
        | 0x610..=0x61a | 0x64b..=0x65f | 0x670 | 0x6d6..=0x6dc | 0x6df..=0x6e4 | 0x6e7 | 0x6e8 | 0x6ea..=0x6ed
        | 0x900..=0x902 | 0x93a | 0x93c | 0x941..=0x948 | 0x94d | 0xe31 | 0xe34..=0xe3a | 0xe47..=0xe4e
        | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x200c | 0x20d0..=0x20ff | 0x3099 | 0x309a
        | 0xfe00..=0xfe0f | 0xfe20..=0xfe2f | 0x1f3fb..=0x1f3ff | 0xe0020..=0xe007f | 0xe0100..=0xe01ef)
}

fn is_spacing_mark(c: char) -> bool {
    matches!(c as u32, 0x903 | 0x93b | 0x93e..=0x940 | 0x949..=0x94c | 0xe33 | 0xeb3)
}

fn is_regional(c: char) -> bool {
    matches!(c as u32, 0x1f1e6..=0x1f1ff)
}

fn is_ext_pict(c: char) -> bool {
    matches!(c as u32,
        0xa9 | 0xae | 0x203c | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x2199 | 0x21a9 | 0x21aa | 0x231a | 0x231b
        | 0x2328 | 0x23cf | 0x23e9..=0x23f3 | 0x23f8..=0x23fa | 0x24c2 | 0x25aa | 0x25ab | 0x25b6 | 0x25c0
        | 0x25fb..=0x25fe | 0x2600..=0x27bf | 0x2934 | 0x2935 | 0x2b05..=0x2b07 | 0x2b1b | 0x2b1c | 0x2b50
        | 0x2b55 | 0x3030 | 0x303d | 0x3297 | 0x3299 | 0x1f000..=0x1faff)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum G {
    Cr,
    Lf,
    Control,
    Extend,
    Zwj,
    Regional,
    Spacing,
    L,
    V,
    T,
    Lv,
    Lvt,
    Pict,
    Other,
}

fn g_class(c: char) -> G {
    let u = c as u32;
    match c {
        '\r' => G::Cr,
        '\n' => G::Lf,
        '\u{200d}' => G::Zwj,
        _ if c.is_control() || matches!(u, 0x200b | 0x200e | 0x200f | 0x2028 | 0x2029 | 0xfeff) => G::Control,
        _ if is_regional(c) => G::Regional,
        _ if is_extend(c) => G::Extend,
        _ if is_spacing_mark(c) => G::Spacing,
        _ if matches!(u, 0x1100..=0x115f | 0xa960..=0xa97c) => G::L,
        _ if matches!(u, 0x1160..=0x11a7 | 0xd7b0..=0xd7c6) => G::V,
        _ if matches!(u, 0x11a8..=0x11ff | 0xd7cb..=0xd7fb) => G::T,
        _ if (0xac00..=0xd7a3).contains(&u) => {
            if (u - 0xac00) % 28 == 0 {
                G::Lv
            } else {
                G::Lvt
            }
        }
        _ if is_ext_pict(c) => G::Pict,
        _ => G::Other,
    }
}

/// Grapheme cluster boundaries. The result has `chars.len() + 1` entries; the first and last are true for text that
/// is not empty (and the single entry of an empty text is true).
pub fn grapheme_boundaries(chars: &[char]) -> Vec<bool> {
    let n = chars.len();
    let mut out = vec![false; n + 1];
    out[0] = true;
    out[n] = true;
    // Whether the text so far ends in `Pict Extend*` (with an optional joiner after it).
    let mut pict_seq = n > 0 && g_class(chars[0]) == G::Pict;
    for i in 1..n {
        let (p, c) = (g_class(chars[i - 1]), g_class(chars[i]));
        let brk = if p == G::Cr && c == G::Lf {
            false
        } else if matches!(p, G::Cr | G::Lf | G::Control) || matches!(c, G::Cr | G::Lf | G::Control) {
            true
        } else if matches!((p, c), (G::L, G::L | G::V | G::Lv | G::Lvt))
            || matches!((p, c), (G::Lv | G::V, G::V | G::T))
            || matches!((p, c), (G::Lvt | G::T, G::T))
        {
            false
        } else if matches!(c, G::Extend | G::Zwj | G::Spacing) {
            false
        } else if p == G::Zwj && c == G::Pict && pict_seq {
            false
        } else if p == G::Regional && c == G::Regional {
            false
        } else {
            true
        };
        out[i] = brk;
        // Track state for the next boundary.
        pict_seq = match c {
            G::Pict => true,
            G::Extend => pict_seq,
            G::Zwj => pict_seq,
            _ => false,
        };
    }
    // The regional indicator state is recomputed simply: pairs from the start of each run.
    let mut run = 0usize;
    for i in 0..n {
        if is_regional(chars[i]) {
            run += 1;
            if i > 0 && is_regional(chars[i - 1]) {
                out[i] = run % 2 == 1;
            }
        } else {
            run = 0;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum L {
    Bk,
    Cr,
    Lf,
    Nl,
    Sp,
    Zw,
    Wj,
    Gl,
    Cm,
    Zwj,
    Op,
    Cl,
    Cp,
    Qu,
    Ex,
    Is,
    Ns,
    Ba,
    Bb,
    Hy,
    B2,
    Cb,
    Id,
    Al,
    Nu,
    Pr,
    Po,
    Hl,
    Sy,
    Ri,
}

fn l_class(c: char) -> L {
    let u = c as u32;
    match c {
        '\n' => L::Lf,
        '\r' => L::Cr,
        '\u{85}' => L::Nl,
        '\u{b}' | '\u{c}' | '\u{2028}' | '\u{2029}' => L::Bk,
        ' ' => L::Sp,
        '\u{200b}' => L::Zw,
        '\u{2060}' | '\u{feff}' => L::Wj,
        '\u{a0}' | '\u{202f}' | '\u{2007}' | '\u{2011}' | '\u{34f}' => L::Gl,
        '\u{200d}' => L::Zwj,
        '\u{fffc}' => L::Cb,
        '\u{2014}' => L::B2,
        '\u{b4}' | '\u{2c8}' => L::Bb,
        '-' => L::Hy,
        '\t' | '\u{ad}' | '\u{58a}' | '\u{2010}' | '\u{2012}' | '\u{2013}' | '|' => L::Ba,
        '/' => L::Sy,
        ',' | '.' | ':' | ';' | '\u{37e}' | '\u{589}' | '\u{60c}' | '\u{2044}' => L::Is,
        '!' | '?' | '\u{ff01}' | '\u{ff1f}' | '\u{203c}' | '\u{2047}'..='\u{2049}' => L::Ex,
        '(' | '[' | '{' | '\u{a1}' | '\u{bf}' | '\u{201a}' | '\u{201e}' | '\u{2045}' | '\u{3008}' | '\u{300a}'
        | '\u{300c}' | '\u{300e}' | '\u{3010}' | '\u{3014}' | '\u{ff08}' | '\u{ff3b}' | '\u{ff5b}' => L::Op,
        '}' | '\u{3001}' | '\u{3002}' | '\u{ff0c}' | '\u{ff0e}' | '\u{3009}' | '\u{300b}' | '\u{300d}'
        | '\u{300f}' | '\u{3011}' | '\u{3015}' => L::Cl,
        ')' | ']' | '\u{ff09}' | '\u{ff3d}' => L::Cp,
        '"' | '\'' | '\u{ab}' | '\u{bb}' | '\u{2018}'..='\u{201f}' | '\u{2039}' | '\u{203a}' => L::Qu,
        '$' | '+' | '\\' | '\u{a3}' | '\u{a5}' | '\u{20ac}' | '\u{2212}' => L::Pr,
        '%' | '\u{a2}' | '\u{b0}' | '\u{2030}'..='\u{2034}' => L::Po,
        '\u{3005}' | '\u{30fb}' | '\u{30fc}' | '\u{3041}' | '\u{3043}' | '\u{3045}' | '\u{3047}' | '\u{3049}'
        | '\u{3063}' | '\u{3083}' | '\u{3085}' | '\u{3087}' | '\u{308e}' | '\u{30a1}' | '\u{30a3}' | '\u{30a5}'
        | '\u{30a7}' | '\u{30a9}' | '\u{30c3}' | '\u{30e3}' | '\u{30e5}' | '\u{30e7}' | '\u{30ee}' | '\u{ff1a}'
        | '\u{ff1b}' => L::Ns,
        '\u{5d0}'..='\u{5ea}' => L::Hl,
        _ if is_extend(c) || is_spacing_mark(c) => L::Cm,
        _ if is_regional(c) => L::Ri,
        _ if c.is_numeric() && c.is_alphanumeric() && !c.is_alphabetic() => L::Nu,
        _ if matches!(u, 0x2e80..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe30..=0xfe4f | 0xff00..=0xffef
            | 0x1f000..=0x1faff | 0x20000..=0x3fffd) => L::Id,
        _ => L::Al,
    }
}

/// Line break opportunities. The result has `chars.len() + 1` entries. The entry for 0 is `No`, the entry for the end
/// is `Mandatory` after a hard break and `Allowed` otherwise.
pub fn line_breaks(chars: &[char]) -> Vec<Brk> {
    let n = chars.len();
    let mut out = vec![Brk::No; n + 1];
    if n == 0 {
        return out;
    }
    let cls: Vec<L> = chars.iter().map(|&c| l_class(c)).collect();
    // `prev` is the class of the previous character after attaching combining marks; `core` is the class of the last
    // character that is not a space (`None` at the start).
    let mut prev = if matches!(cls[0], L::Cm | L::Zwj) { L::Al } else { cls[0] };
    let mut core = if prev == L::Sp { None } else { Some(prev) };
    let mut prev_prev: Option<L> = None;
    let mut ri_run = if prev == L::Ri { 1usize } else { 0 };
    for i in 1..n {
        let mut cur = cls[i];
        if matches!(cur, L::Cm | L::Zwj) {
            if matches!(prev, L::Bk | L::Cr | L::Lf | L::Nl | L::Sp | L::Zw) {
                cur = L::Al;
            } else {
                // Attached to the character before: no break, state unchanged.
                out[i] = Brk::No;
                continue;
            }
        }
        out[i] = decide(prev, cur, core, prev_prev, ri_run);
        ri_run = if cur == L::Ri { if prev == L::Ri { ri_run + 1 } else { 1 } } else { 0 };
        prev_prev = Some(prev);
        prev = cur;
        if cur != L::Sp {
            core = Some(cur);
        }
    }
    // A joiner keeps the characters on both sides of it together.
    for i in 1..n {
        if chars[i - 1] == '\u{200d}' && out[i] == Brk::Allowed {
            out[i] = Brk::No;
        }
    }
    out[n] = if matches!(prev, L::Bk | L::Lf | L::Nl | L::Cr) { Brk::Mandatory } else { Brk::Allowed };
    out
}

fn decide(prev: L, cur: L, core: Option<L>, prev_prev: Option<L>, ri_run: usize) -> Brk {
    use L::*;
    // Hard breaks.
    if prev == Bk || prev == Lf || prev == Nl {
        return Brk::Mandatory;
    }
    if prev == Cr {
        return if cur == Lf { Brk::No } else { Brk::Mandatory };
    }
    if matches!(cur, Bk | Cr | Lf | Nl | Sp | Zw) {
        return Brk::No;
    }
    if core == Some(Zw) {
        return Brk::Allowed;
    }
    if cur == Wj || prev == Wj || prev == Gl {
        return Brk::No;
    }
    if cur == Gl && !matches!(prev, Sp | Ba | Hy) {
        return Brk::No;
    }
    if matches!(cur, Cl | Cp | Ex | Is | Sy) {
        return Brk::No;
    }
    if core == Some(Op) {
        return Brk::No;
    }
    if core == Some(Qu) && cur == Op {
        return Brk::No;
    }
    if matches!(core, Some(Cl) | Some(Cp)) && cur == Ns {
        return Brk::No;
    }
    if core == Some(B2) && cur == B2 {
        return Brk::No;
    }
    if prev == Sp {
        return Brk::Allowed;
    }
    if cur == Qu || prev == Qu {
        return Brk::No;
    }
    if cur == Cb || prev == Cb {
        return Brk::Allowed;
    }
    if matches!(cur, Ba | Hy | Ns) || prev == Bb {
        return Brk::No;
    }
    if prev_prev == Some(Hl) && matches!(prev, Hy | Ba) {
        return Brk::No;
    }
    if prev == Sy && cur == Hl {
        return Brk::No;
    }
    let letter = |c: L| matches!(c, Al | Hl);
    if (letter(prev) && cur == Nu) || (prev == Nu && letter(cur)) {
        return Brk::No;
    }
    if (matches!(prev, Pr | Po) && letter(cur)) || (letter(prev) && matches!(cur, Pr | Po)) {
        return Brk::No;
    }
    let pair = (prev, cur);
    if matches!(pair, (Cl, Po) | (Cp, Po) | (Cl, Pr) | (Cp, Pr) | (Nu, Po) | (Nu, Pr) | (Po, Op) | (Po, Nu)
        | (Pr, Op) | (Pr, Nu) | (Hy, Nu) | (Is, Nu) | (Nu, Nu) | (Sy, Nu))
    {
        return Brk::No;
    }
    if letter(prev) && letter(cur) {
        return Brk::No;
    }
    if prev == Is && letter(cur) {
        return Brk::No;
    }
    if (letter(prev) || prev == Nu) && cur == Op {
        return Brk::No;
    }
    if prev == Cp && (letter(cur) || cur == Nu) {
        return Brk::No;
    }
    if prev == Ri && cur == Ri && ri_run % 2 == 1 {
        return Brk::No;
    }
    Brk::Allowed
}

/// Greedy line wrap of one paragraph into lines no wider than `max_w` as `measure` reports it. Runs of white space
/// (newlines included) collapse to one space and no line starts or ends with one. Lines end at the line break
/// opportunities of [`line_breaks`]: after spaces and hyphens, between ideographs, kana and hangul, never before
/// closing punctuation (`、。」）`) nor after opening punctuation (`「（`). As a last resort a run with no opportunity
/// that is wider than the line is split between grapheme clusters, so every line fits unless one cluster alone is
/// wider than `max_w`.
pub fn wrap_paragraph(text: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = norm.chars().collect();
    let breaks = line_breaks(&chars);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut start = 0;
    for i in 1..=chars.len() {
        if i < chars.len() && breaks[i] == Brk::No {
            continue;
        }
        let seg: String = chars[start..i].iter().collect();
        start = i;
        let candidate = format!("{line}{seg}");
        if measure(candidate.trim_end()) <= max_w {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(line.trim_end().to_string());
        }
        line = seg;
        if measure(line.trim_end()) > max_w {
            line = split_clusters(&line, max_w, measure, &mut lines);
        }
    }
    if !line.trim_end().is_empty() {
        lines.push(line.trim_end().to_string());
    }
    lines
}

/// Splits `run` between grapheme clusters: every full line goes to `lines`, the last, partly filled one is returned.
fn split_clusters(run: &str, max_w: f32, measure: &dyn Fn(&str) -> f32, lines: &mut Vec<String>) -> String {
    let chars: Vec<char> = run.chars().collect();
    let bounds = grapheme_boundaries(&chars);
    let mut cur = String::new();
    let mut from = 0;
    for i in 1..=chars.len() {
        if !bounds[i] {
            continue;
        }
        let cluster: String = chars[from..i].iter().collect();
        from = i;
        let next = format!("{cur}{cluster}");
        if !cur.is_empty() && measure(next.trim_end()) > max_w {
            lines.push(std::mem::replace(&mut cur, cluster));
        } else {
            cur = next;
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn clusters(s: &str) -> usize {
        let c = cs(s);
        grapheme_boundaries(&c).iter().filter(|b| **b).count() - 1
    }

    #[test]
    fn graphemes_join_marks_and_crlf() {
        assert_eq!(clusters("abc"), 3);
        assert_eq!(clusters("e\u{301}x"), 2);
        assert_eq!(clusters("a\r\nb"), 3);
        assert_eq!(clusters(""), 0);
    }

    #[test]
    fn graphemes_join_emoji_and_flags_and_hangul() {
        // man ZWJ woman is one cluster; a skin tone modifier joins its base.
        assert_eq!(clusters("\u{1f468}\u{200d}\u{1f469}"), 1);
        assert_eq!(clusters("\u{1f44d}\u{1f3fd}"), 1);
        // Two flags are two clusters, not one or four.
        assert_eq!(clusters("\u{1f1ee}\u{1f1f8}\u{1f1e9}\u{1f1ea}"), 2);
        // L V T jamo form one syllable.
        assert_eq!(clusters("\u{1100}\u{1161}\u{11a8}"), 1);
    }

    fn allowed(s: &str) -> Vec<usize> {
        let c = cs(s);
        line_breaks(&c).iter().enumerate().filter(|(i, b)| *i > 0 && *i < c.len() && **b != Brk::No).map(|(i, _)| i).collect()
    }

    #[test]
    fn breaks_after_spaces_and_hyphens() {
        assert_eq!(allowed("aa bb cc"), vec![3, 6]);
        assert_eq!(allowed("well-known"), vec![5]);
        assert_eq!(allowed("abcdef"), Vec::<usize>::new());
    }

    #[test]
    fn hard_breaks_are_mandatory() {
        let c = cs("ab\ncd");
        let b = line_breaks(&c);
        assert_eq!(b[3], Brk::Mandatory);
        assert_eq!(b[2], Brk::No);
        assert_eq!(*line_breaks(&cs("ab\n")).last().unwrap(), Brk::Mandatory);
    }

    #[test]
    fn punctuation_and_numbers_stay_attached() {
        assert_eq!(allowed("end. Next"), vec![5]);
        assert_eq!(allowed("$100"), Vec::<usize>::new());
        assert_eq!(allowed("(a b)"), vec![3]);
        // no-break space does not break
        assert_eq!(allowed("a\u{a0}b c"), vec![4]);
    }

    /// Monospace: 10 points per character.
    fn m(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wrap_paragraph_breaks_words_at_spaces() {
        assert_eq!(wrap_paragraph("aa bb cc dd", 50.0, &m), vec!["aa bb", "cc dd"]);
        assert_eq!(wrap_paragraph("  aa \n bb  ", 100.0, &m), vec!["aa bb"]);
        assert!(wrap_paragraph("   ", 100.0, &m).is_empty());
    }

    #[test]
    fn wrap_paragraph_breaks_between_ideographs_kana_and_hangul() {
        // Four characters fit on a 40 point line.
        assert_eq!(wrap_paragraph("不想再收到我们的邮件", 40.0, &m), vec!["不想再收", "到我们的", "邮件"]);
        assert_eq!(wrap_paragraph("ひらがなカタカナ", 40.0, &m), vec!["ひらがな", "カタカナ"]);
        assert_eq!(wrap_paragraph("한국어문장입니다", 40.0, &m), vec!["한국어문", "장입니다"]);
    }

    #[test]
    fn wrap_paragraph_keeps_cjk_punctuation_with_its_neighbour() {
        // 、 and 。 never start a line: the ideograph before them moves down with them.
        assert_eq!(wrap_paragraph("日本語、日本", 30.0, &m), vec!["日本", "語、日", "本"]);
        assert_eq!(wrap_paragraph("日本語。", 30.0, &m), vec!["日本", "語。"]);
        // 「 and （ never end a line: they move down with the character after them.
        assert_eq!(wrap_paragraph("日本「語」", 30.0, &m), vec!["日本", "「語」"]);
        assert_eq!(wrap_paragraph("日本（語）", 30.0, &m), vec!["日本", "（語）"]);
        let lines = wrap_paragraph("（或点击「回复」）。不想再收到我们的邮件？", 50.0, &m);
        for l in &lines {
            assert!(!l.ends_with('「') && !l.ends_with('（'), "{lines:?}");
            assert!(!l.starts_with('」') && !l.starts_with('）') && !l.starts_with('。') && !l.starts_with('？'), "{lines:?}");
            assert!(m(l) <= 50.0, "{lines:?}");
        }
        assert_eq!(lines.concat(), "（或点击「回复」）。不想再收到我们的邮件？");
    }

    #[test]
    fn wrap_paragraph_splits_an_overlong_run_at_the_width() {
        assert_eq!(wrap_paragraph("abcdefg", 30.0, &m), vec!["abc", "def", "g"]);
        assert_eq!(wrap_paragraph("ab cdefgh ij", 30.0, &m), vec!["ab", "cde", "fgh", "ij"]);
        // a cluster wider than the line still takes a line of its own
        assert_eq!(wrap_paragraph("ab", 5.0, &m), vec!["a", "b"]);
        // a combining mark stays with its base
        assert_eq!(wrap_paragraph("e\u{301}e\u{301}", 15.0, &m), vec!["e\u{301}", "e\u{301}"]);
    }

    #[test]
    fn ideographs_break_between_characters() {
        assert_eq!(allowed("日本語"), vec![1, 2]);
        // a closing punctuation mark does not start a line
        assert_eq!(allowed("日。本"), vec![2]);
    }
}
