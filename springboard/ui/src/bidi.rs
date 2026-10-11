//! Writing direction and bidirectional text (the writing direction attribute of a text run, and the visual order of
//! its runs). A pure model of the Unicode bidirectional algorithm for one paragraph: it resolves the embedding level
//! of every character and returns the runs in visual order, left to right on screen, for a drawing adapter.
//! Explicit embeddings and isolates (LRE, RLE, LRI, RLI and the rest) are not modelled, so the whole paragraph is
//! one isolating run sequence. Indices are character indices, not bytes.

use std::ops::Range;

/// The writing direction of a paragraph. `Natural` takes the direction of its first strong character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Natural,
    LeftToRight,
    RightToLeft,
}

/// Bidirectional class of a character, reduced to the classes this model tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Strong left to right (Latin, Greek, Cyrillic, most letters).
    L,
    /// Strong right to left (Hebrew, NKo, Samaritan).
    R,
    /// Strong right to left Arabic letter. Rule W3 turns it into `R`.
    AL,
    /// European digit.
    EN,
    /// Arabic-Indic digit.
    AN,
    /// European separator (`+`, `-`).
    ES,
    /// European terminator (`#`, `$`, `%`, currency signs).
    ET,
    /// Common separator (`,`, `.`, `/`, `:`).
    CS,
    /// Non-spacing mark. Rule W1 gives it the class of the character before it.
    NSM,
    /// Paragraph separator.
    B,
    /// Segment separator (tab).
    S,
    /// Whitespace.
    WS,
    /// Any other neutral.
    ON,
}

impl Class {
    fn is_neutral(self) -> bool {
        matches!(self, Class::B | Class::S | Class::WS | Class::ON)
    }
}

fn class_of(c: char) -> Class {
    match c {
        '\n' | '\r' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2029}' => Class::B,
        '\t' | '\u{0b}' | '\u{1f}' => Class::S,
        ' ' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{3000}' => Class::WS,
        '+' | '-' | '\u{2212}' => Class::ES,
        '#' | '$' | '%' | '\u{a2}'..='\u{a5}' | '\u{b0}' | '\u{b1}' | '\u{20a0}'..='\u{20cf}' | '\u{2030}'..='\u{2034}' => {
            Class::ET
        }
        ',' | '.' | '/' | ':' | '\u{a0}' | '\u{60c}' => Class::CS,
        '0'..='9' => Class::EN,
        '\u{200e}' => Class::L,
        '\u{200f}' => Class::R,
        '\u{300}'..='\u{36f}' | '\u{591}'..='\u{5bd}' | '\u{5bf}' | '\u{5c1}' | '\u{5c2}' | '\u{5c4}' | '\u{5c5}'
        | '\u{5c7}' | '\u{64b}'..='\u{65f}' | '\u{670}' => Class::NSM,
        '\u{600}'..='\u{605}' | '\u{660}'..='\u{669}' | '\u{66b}' | '\u{66c}' => Class::AN,
        '\u{590}'..='\u{5ff}' | '\u{7c0}'..='\u{85f}' | '\u{fb1d}'..='\u{fb4f}' | '\u{10800}'..='\u{10fff}' => Class::R,
        '\u{606}'..='\u{8ff}' | '\u{fb50}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}' => Class::AL,
        _ if c.is_alphabetic() => Class::L,
        _ if c.is_numeric() => Class::EN,
        _ => Class::ON,
    }
}

/// A run of characters that share one level. `chars` is a range of logical character indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub chars: Range<usize>,
    pub level: u8,
}

impl Run {
    /// Odd levels are right to left: a drawing adapter places the glyphs of such a run in reverse.
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

/// One paragraph laid out for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paragraph {
    /// The paragraph embedding level: 0 for left to right, 1 for right to left.
    pub level: u8,
    /// The resolved level of each character, in logical order.
    pub levels: Vec<u8>,
    /// The runs in visual order (left to right on screen).
    pub runs: Vec<Run>,
}

impl Paragraph {
    /// Resolves `text` (one paragraph; a line break is treated as a paragraph separator for levels only).
    pub fn new(text: &str, direction: Direction) -> Self {
        let orig: Vec<Class> = text.chars().map(class_of).collect();
        let level = paragraph_level(&orig, direction);
        let levels = resolve_levels(&orig, level);
        let runs = visual_runs(&levels);
        Paragraph { level, levels, runs }
    }

    /// Whether the paragraph itself is right to left.
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }

    /// Logical character indices in visual order.
    pub fn visual_order(&self) -> Vec<usize> {
        visual_order(&self.levels)
    }
}

/// P2 and P3: the first strong character decides a natural direction; none means left to right.
fn paragraph_level(classes: &[Class], direction: Direction) -> u8 {
    match direction {
        Direction::LeftToRight => 0,
        Direction::RightToLeft => 1,
        Direction::Natural => match classes.iter().find(|c| matches!(c, Class::L | Class::R | Class::AL)) {
            Some(Class::R) | Some(Class::AL) => 1,
            _ => 0,
        },
    }
}

/// Resolves the levels of one paragraph: the weak rules W1 to W7, the neutral rules N1 and N2, the implicit rules
/// I1 and I2, and the line rule L1 for separators and trailing whitespace.
fn resolve_levels(orig: &[Class], para: u8) -> Vec<u8> {
    let n = orig.len();
    let mut cls = orig.to_vec();
    // Without embeddings, the start and end of the sequence are both the paragraph direction.
    let edge = if para == 1 { Class::R } else { Class::L };

    // W1: a non-spacing mark takes the class of the character before it.
    let mut prev = edge;
    for c in cls.iter_mut() {
        if *c == Class::NSM {
            *c = prev;
        } else {
            prev = *c;
        }
    }

    // W2: a European digit after an Arabic letter is an Arabic-Indic digit. W3: an Arabic letter becomes R.
    let mut last_strong = edge;
    for c in cls.iter_mut() {
        match *c {
            Class::L | Class::R => last_strong = *c,
            Class::AL => {
                last_strong = Class::AL;
                *c = Class::R;
            }
            Class::EN if last_strong == Class::AL => *c = Class::AN,
            _ => {}
        }
    }

    // W4: a single separator between two numbers of the same kind joins them.
    for i in 1..n.saturating_sub(1) {
        let (p, q) = (cls[i - 1], cls[i + 1]);
        match cls[i] {
            Class::ES if p == Class::EN && q == Class::EN => cls[i] = Class::EN,
            Class::CS if p == q && matches!(p, Class::EN | Class::AN) => cls[i] = p,
            _ => {}
        }
    }

    // W5: a run of terminators next to a European digit becomes European digits.
    let mut i = 0;
    while i < n {
        if cls[i] != Class::ET {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && cls[i] == Class::ET {
            i += 1;
        }
        let next_en = i < n && cls[i] == Class::EN;
        if (start > 0 && cls[start - 1] == Class::EN) || next_en {
            cls[start..i].fill(Class::EN);
        }
    }

    // W6: the separators and terminators left over are neutral.
    for c in cls.iter_mut() {
        if matches!(c, Class::ES | Class::ET | Class::CS) {
            *c = Class::ON;
        }
    }

    // W7: a European digit after a left-to-right letter (or at a left-to-right start) is left to right.
    let mut last = edge;
    for c in cls.iter_mut() {
        match *c {
            Class::L | Class::R => last = *c,
            Class::EN if last == Class::L => *c = Class::L,
            _ => {}
        }
    }

    // N1 and N2: a run of neutrals takes the direction around it when both sides agree, else the paragraph's.
    // European and Arabic-Indic digits count as right to left here.
    let strong = |c: Class| if c == Class::L { Class::L } else { Class::R };
    let embedding = if para == 1 { Class::R } else { Class::L };
    let mut i = 0;
    while i < n {
        if !cls[i].is_neutral() {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && cls[i].is_neutral() {
            i += 1;
        }
        let before = if start == 0 { edge } else { strong(cls[start - 1]) };
        let after = if i == n { edge } else { strong(cls[i]) };
        let dir = if before == after { before } else { embedding };
        cls[start..i].fill(dir);
    }

    // I1 and I2: the implicit levels.
    let mut levels: Vec<u8> = cls.iter().map(|&c| implicit_level(c, para)).collect();

    // L1: separators, and whitespace before them or at the end of the line, take the paragraph level.
    let mut trailing = true;
    for i in (0..n).rev() {
        match orig[i] {
            Class::B | Class::S => {
                levels[i] = para;
                trailing = true;
            }
            Class::WS if trailing => levels[i] = para,
            _ => trailing = false,
        }
    }
    levels
}

fn implicit_level(c: Class, para: u8) -> u8 {
    if para == 0 {
        match c {
            Class::R => 1,
            Class::EN | Class::AN => 2,
            _ => 0,
        }
    } else {
        match c {
            Class::L | Class::EN | Class::AN => 2,
            _ => 1,
        }
    }
}

/// Rule L2: from the highest level down to the lowest odd level, reverse every run of characters at that level or
/// higher. Returns the logical character indices in visual order.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let min = match levels.iter().min() {
        Some(&m) => m,
        None => return order,
    };
    let max = levels.iter().copied().max().unwrap_or(0);
    let lowest_odd = if min % 2 == 1 { min } else { min + 1 };
    let mut k = max;
    while k >= lowest_odd {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= k {
                let start = i;
                while i < order.len() && levels[order[i]] >= k {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        k -= 1;
    }
    order
}

/// The maximal runs of equal level, in visual order.
fn visual_runs(levels: &[u8]) -> Vec<Run> {
    let mut blocks: Vec<Run> = Vec::new();
    for (i, &level) in levels.iter().enumerate() {
        let extend = blocks.last().is_some_and(|b| b.level == level);
        if extend {
            if let Some(b) = blocks.last_mut() {
                b.chars.end = i + 1;
            }
        } else {
            blocks.push(Run { chars: i..i + 1, level });
        }
    }
    // A run keeps its characters together, so reordering the runs as units gives the visual order of the runs.
    let block_levels: Vec<u8> = blocks.iter().map(|b| b.level).collect();
    visual_order(&block_levels).into_iter().map(|b| blocks[b].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebrew_is_right_to_left_under_the_natural_direction() {
        let p = Paragraph::new("שלום", Direction::Natural);
        assert_eq!(p.level, 1);
        assert!(p.is_rtl());
        assert_eq!(p.levels, vec![1, 1, 1, 1]);
        assert_eq!(p.runs, vec![Run { chars: 0..4, level: 1 }]);
    }

    #[test]
    fn digits_keep_a_left_to_right_paragraph() {
        let p = Paragraph::new("123 abc", Direction::Natural);
        assert_eq!(p.level, 0);
        assert_eq!(p.levels, vec![0; 7]);
    }

    #[test]
    fn a_neutral_between_opposite_directions_takes_the_paragraph_direction() {
        // The space sits between a right-to-left and a left-to-right letter.
        let p = Paragraph::new("א a", Direction::LeftToRight);
        assert_eq!(p.levels, vec![1, 0, 0]);
    }

    #[test]
    fn numbers_in_a_right_to_left_paragraph_run_left_to_right() {
        let p = Paragraph::new("שלום 123", Direction::RightToLeft);
        assert_eq!(p.levels, vec![1, 1, 1, 1, 1, 2, 2, 2]);
        // On screen the digits come first, then the Hebrew word with its space.
        assert_eq!(p.runs[0], Run { chars: 5..8, level: 2 });
        assert_eq!(p.runs[1].chars, 0..5);
    }

    #[test]
    fn visual_order_reverses_each_right_to_left_run() {
        assert_eq!(visual_order(&[0, 1, 1, 1, 0]), vec![0, 3, 2, 1, 4]);
        // Two levels of two are not reversed: no odd level is involved.
        assert_eq!(visual_order(&[2, 2]), vec![0, 1]);
    }

    #[test]
    fn arabic_indic_digits_stay_in_order() {
        let p = Paragraph::new("\u{661}\u{662}", Direction::LeftToRight);
        assert_eq!(p.levels, vec![2, 2]);
        assert_eq!(p.visual_order(), vec![0, 1]);
    }

    #[test]
    fn a_combining_mark_follows_its_base_letter() {
        // Alef followed by a Hebrew point, which is a non-spacing mark.
        let p = Paragraph::new("\u{5d0}\u{5b0}", Direction::LeftToRight);
        assert_eq!(p.levels, vec![1, 1]);
    }

    #[test]
    fn a_mixed_line_orders_its_runs_for_the_display() {
        let ltr = Paragraph::new("abc שלום", Direction::LeftToRight);
        assert_eq!(ltr.runs.len(), 2);
        assert_eq!(ltr.runs[0].chars, 0..4);
        assert_eq!(ltr.runs[1].chars, 4..8);
        assert!(ltr.runs[1].is_rtl());

        let rtl = Paragraph::new("abc שלום", Direction::RightToLeft);
        assert!(rtl.is_rtl());
        assert_eq!(rtl.levels, vec![2, 2, 2, 1, 1, 1, 1, 1]);
        assert_eq!(rtl.runs[0].chars, 3..8);
        assert_eq!(rtl.runs[1].chars, 0..3);
    }

    #[test]
    fn trailing_whitespace_takes_the_paragraph_level() {
        // A tab ends the line, so it takes the paragraph level (L1).
        let p = Paragraph::new("א\t", Direction::RightToLeft);
        assert_eq!(p.levels, vec![1, 1]);
    }
}
