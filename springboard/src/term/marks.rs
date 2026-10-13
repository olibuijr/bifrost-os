//! Marks: the scrollback lines where a prompt starts (the shell's OSC 133;A sequence) or the user set a mark.
//! Lines are counted from the oldest scrollback line (line 0), so a mark keeps its line while the history grows.

/// The bytes that open a "prompt starts here" report.
const PROMPT: &[u8] = b"\x1b]133;A";

/// Marked lines, oldest first, without duplicates.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Marks {
    lines: Vec<usize>,
}

impl Marks {
    /// Marks `line`; false when it was marked already.
    pub fn add(&mut self, line: usize) -> bool {
        match self.lines.binary_search(&line) {
            Ok(_) => false,
            Err(at) => {
                self.lines.insert(at, line);
                true
            }
        }
    }

    /// Marks `line`, or removes its mark; true when the line is marked afterwards.
    pub fn toggle(&mut self, line: usize) -> bool {
        if self.add(line) {
            return true;
        }
        self.lines.retain(|&l| l != line);
        false
    }

    pub fn contains(&self, line: usize) -> bool {
        self.lines.binary_search(&line).is_ok()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Drops the marks on lines `limit` and after, which no longer exist.
    pub fn forget_from(&mut self, limit: usize) {
        self.lines.retain(|&l| l < limit);
    }

    /// The nearest mark above `line`.
    pub fn previous(&self, line: usize) -> Option<usize> {
        self.lines.iter().rev().copied().find(|&l| l < line)
    }

    /// The nearest mark below `line`.
    pub fn next(&self, line: usize) -> Option<usize> {
        self.lines.iter().copied().find(|&l| l > line)
    }

    /// The view offset (lines back from the live screen) that puts the previous or next mark on the top row, given
    /// `history` scrollback lines and the current `view` offset. None when there is no such mark or the view would not move.
    pub fn jump(&self, history: usize, view: usize, forward: bool) -> Option<usize> {
        let top = history.saturating_sub(view);
        let line = if forward { self.next(top)? } else { self.previous(top)? };
        let target = history.saturating_sub(line);
        (target != view).then_some(target)
    }
}

/// Offsets into `chunk` where a prompt report begins. `carry` is the tail of the bytes before it (see `tail`), so a
/// report split between two reads is found once, at offset 0 of the chunk that completes it.
pub fn prompt_starts(carry: &[u8], chunk: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    // a report that began in the carry and ends in this chunk (the carry is too short to hold a whole one)
    if !carry.is_empty() {
        let mut head = carry.to_vec();
        head.extend_from_slice(&chunk[..chunk.len().min(PROMPT.len() - 1)]);
        if (0..carry.len()).any(|i| head[i..].starts_with(PROMPT)) {
            out.push(0);
        }
    }
    // reports inside the chunk: only the bytes that open an escape sequence are looked at
    let mut from = 0;
    while let Some(found) = chunk[from..].iter().position(|&b| b == 0x1b) {
        let at = from + found;
        if chunk[at..].starts_with(PROMPT) {
            out.push(at);
        }
        from = at + 1;
    }
    out
}

/// The bytes to carry into the next read: the last few of `carry` and `chunk`, too short to hold a whole report.
pub fn tail(carry: &[u8], chunk: &[u8]) -> Vec<u8> {
    let keep = PROMPT.len() - 1;
    if chunk.len() >= keep {
        return chunk[chunk.len() - keep..].to_vec();
    }
    let from_carry = (keep - chunk.len()).min(carry.len());
    let mut out = carry[carry.len() - from_carry..].to_vec();
    out.extend_from_slice(chunk);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_stay_sorted_unique_and_toggle() {
        let mut m = Marks::default();
        assert!(m.add(50) && m.add(10) && m.add(90));
        assert!(!m.add(50), "a line is marked once");
        assert_eq!((m.len(), m.contains(10), m.contains(11)), (3, true, false));
        assert_eq!(m.previous(50), Some(10));
        assert_eq!(m.next(50), Some(90));
        assert_eq!((m.previous(10), m.next(90)), (None, None));
        assert!(!m.toggle(50), "a second toggle removes the mark");
        assert!(m.toggle(70));
        assert_eq!(m.next(50), Some(70));
        m.forget_from(70);
        assert_eq!((m.contains(70), m.contains(10)), (false, true));
        m.clear();
        assert!(m.is_empty());
    }

    #[test]
    fn jumps_put_the_mark_on_the_top_row() {
        let mut m = Marks::default();
        for line in [10, 50, 90] {
            m.add(line);
        }
        // live view of 100 history lines: the top row is line 100
        assert_eq!(m.jump(100, 0, false), Some(10), "previous mark is line 90, ten lines back");
        assert_eq!(m.jump(100, 10, false), Some(50), "from the top at 90 the next one up is line 50");
        assert_eq!(m.jump(100, 50, true), Some(10), "forward from line 50 lands on line 90");
        assert_eq!(m.jump(100, 10, true), None, "no mark below the last one");
        assert_eq!(m.jump(100, 90, false), None, "no mark above the first one");
        assert_eq!(Marks::default().jump(100, 0, false), None);
        // a mark on the live screen cannot be scrolled to; the view only returns to live
        let mut live = Marks::default();
        live.add(104);
        assert_eq!(live.jump(100, 30, true), Some(0));
        assert_eq!(live.jump(100, 0, true), None, "already live");
    }

    #[test]
    fn prompt_reports_are_found_in_a_chunk_and_across_two() {
        assert_eq!(prompt_starts(b"", b"ab\x1b]133;A\x07cd\x1b]133;A"), vec![2, 12]);
        assert!(prompt_starts(b"", b"\x1b]133;B\x07").is_empty(), "only the prompt start is a mark");
        let first = b"out\x1b]13";
        assert!(prompt_starts(b"", first).is_empty());
        let carry = tail(b"", first);
        assert_eq!(prompt_starts(&carry, b"3;A\x07$ "), vec![0], "the report completes in the next read");
        let carry = tail(&carry, b"3;A\x07$ ");
        assert!(prompt_starts(&carry, b"ls").is_empty(), "a found report is not counted twice");
    }

    #[test]
    fn the_carry_is_shorter_than_a_report() {
        assert_eq!(tail(b"", b"abc"), b"abc");
        assert_eq!(tail(b"xy", b"0123456789"), b"456789");
        assert!(tail(b"", b"").is_empty());
    }
}
