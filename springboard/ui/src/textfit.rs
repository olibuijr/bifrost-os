//! Line breaking and truncation (`NativeLineBreakMode`, `NativeParagraphStyle`) against a caller-supplied measure
//! function, so it works with any font. `measure` returns the width of a string in points.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truncate {
    Head,
    Middle,
    Tail,
}

const ELLIPSIS: char = '…';

/// Shortens `text` to fit `width`, replacing the removed part with an ellipsis. Binary search on the kept length.
pub fn truncate(text: &str, width: f32, mode: Truncate, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| -> String {
        match mode {
            Truncate::Tail => chars[..keep].iter().collect::<String>() + &ELLIPSIS.to_string(),
            Truncate::Head => ELLIPSIS.to_string() + &chars[chars.len() - keep..].iter().collect::<String>(),
            Truncate::Middle => {
                let a = keep.div_ceil(2);
                let b = keep - a;
                chars[..a].iter().collect::<String>() + &ELLIPSIS.to_string() + &chars[chars.len() - b..].iter().collect::<String>()
            }
        }
    };
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&build(mid)) <= width { lo = mid } else { hi = mid - 1 }
    }
    build(lo)
}

/// Greedy word wrap (`NativeLineBreakByWordWrapping`). A word wider than the line is split by characters.
/// Explicit `\n` always breaks. Returns the lines without their trailing spaces.
pub fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let trial = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&trial) <= width {
                line = trial;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Split an over-long word.
            let mut piece = String::new();
            for ch in word.chars() {
                let t = format!("{piece}{ch}");
                if measure(&t) > width && !piece.is_empty() {
                    lines.push(std::mem::take(&mut piece));
                }
                piece.push(ch);
            }
            line = piece;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }
    #[test]
    fn fits_unchanged() {
        assert_eq!(truncate("hello", 50.0, Truncate::Tail, &m), "hello");
    }
    #[test]
    fn truncation_modes() {
        assert_eq!(truncate("hello world", 60.0, Truncate::Tail, &m), "hello…");
        assert_eq!(truncate("hello world", 60.0, Truncate::Head, &m), "…world");
        assert_eq!(truncate("hello world", 60.0, Truncate::Middle, &m), "hel…ld");
        assert_eq!(truncate("hello", 5.0, Truncate::Tail, &m), "…");
    }
    #[test]
    fn wraps_words_and_newlines() {
        assert_eq!(wrap("aa bb cc dd", 50.0, &m), vec!["aa bb", "cc dd"]);
        assert_eq!(wrap("a\nb", 100.0, &m), vec!["a", "b"]);
    }
    #[test]
    fn long_words_split() {
        assert_eq!(wrap("abcdefg", 30.0, &m), vec!["abc", "def", "g"]);
    }
}
