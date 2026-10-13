//! Writing direction and bidirectional text: the full Unicode bidirectional algorithm for a paragraph. It resolves
//! the embedding level of every character (explicit embeddings, overrides and isolates, weak and neutral rules,
//! paired brackets, implicit levels), applies the line rule per line, reorders a line for display, and mirrors
//! characters in right to left runs. A writing direction attribute over a range of text (embedding or override,
//! either way) is applied by [`Paragraph::with_attributes`].
//!
//! Pure functions, no fonts and no drawing. The entry points a line layout calls are [`Paragraph::new`],
//! [`Paragraph::line_levels`], [`reorder_line`], [`Paragraph::line_runs`] and [`mirrored`]. Indices are character
//! indices, not bytes. The class table covers the scripts in everyday use (Latin, Greek, Cyrillic, Hebrew, Arabic and
//! its relatives, Syriac, Thaana, NKo, common symbols and digits); characters it does not list are left to right when
//! alphabetic and neutral otherwise.

use std::ops::Range;

/// The writing direction of a paragraph. `Natural` takes the direction of its first strong character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Natural,
    LeftToRight,
    RightToLeft,
}

/// A writing direction attribute value for a range of text: an embedding keeps the natural order of the characters
/// inside it, an override forces every character inside to the given direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritingDirection {
    LeftToRightEmbedding,
    RightToLeftEmbedding,
    LeftToRightOverride,
    RightToLeftOverride,
}

impl WritingDirection {
    fn open(self) -> char {
        match self {
            WritingDirection::LeftToRightEmbedding => '\u{202a}',
            WritingDirection::RightToLeftEmbedding => '\u{202b}',
            WritingDirection::LeftToRightOverride => '\u{202d}',
            WritingDirection::RightToLeftOverride => '\u{202e}',
        }
    }
}

/// Bidirectional character class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BidiClass {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

impl BidiClass {
    fn is_isolate_initiator(self) -> bool {
        matches!(self, BidiClass::LRI | BidiClass::RLI | BidiClass::FSI)
    }

    /// Characters the algorithm removes (rule X9): explicit embeddings, overrides and boundary neutrals.
    fn is_removed(self) -> bool {
        matches!(
            self,
            BidiClass::BN | BidiClass::LRE | BidiClass::LRO | BidiClass::RLE | BidiClass::RLO | BidiClass::PDF
        )
    }

    fn is_neutral(self) -> bool {
        matches!(
            self,
            BidiClass::B
                | BidiClass::S
                | BidiClass::WS
                | BidiClass::ON
                | BidiClass::LRI
                | BidiClass::RLI
                | BidiClass::FSI
                | BidiClass::PDI
        )
    }
}

fn in_ranges(v: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(a, b)| v >= a && v <= b)
}

/// The bidirectional class of a character.
pub fn bidi_class(c: char) -> BidiClass {
    use BidiClass::*;
    let v = c as u32;
    match v {
        0x202a => return LRE,
        0x202b => return RLE,
        0x202c => return PDF,
        0x202d => return LRO,
        0x202e => return RLO,
        0x2066 => return LRI,
        0x2067 => return RLI,
        0x2068 => return FSI,
        0x2069 => return PDI,
        0x200e => return L,
        0x200f => return R,
        0x061c => return AL,
        _ => {}
    }
    if v < 0x80 {
        return match c {
            '\n' | '\r' | '\u{1c}'..='\u{1e}' => B,
            '\t' | '\u{0b}' | '\u{1f}' => S,
            ' ' | '\u{0c}' => WS,
            '\0'..='\u{08}' | '\u{0e}'..='\u{1b}' | '\u{7f}' => BN,
            '+' | '-' => ES,
            '#' | '$' | '%' => ET,
            ',' | '.' | '/' | ':' => CS,
            '0'..='9' => EN,
            'a'..='z' | 'A'..='Z' => L,
            _ => ON,
        };
    }
    // Specific classes first, then the script blocks.
    if in_ranges(
        v,
        &[
            (0x80, 0x84),
            (0x86, 0x9f),
            (0xad, 0xad),
            (0x200b, 0x200d),
            (0x2060, 0x2064),
            (0x206a, 0x206f),
            (0xfeff, 0xfeff),
            (0x180e, 0x180e),
            (0xe0001, 0xe0001),
            (0xe0020, 0xe007f),
        ],
    ) {
        return BN;
    }
    if in_ranges(v, &[(0x85, 0x85), (0x2029, 0x2029)]) {
        return B;
    }
    if in_ranges(v, &[(0x1680, 0x1680), (0x2000, 0x200a), (0x2028, 0x2028), (0x205f, 0x205f), (0x3000, 0x3000)]) {
        return WS;
    }
    if in_ranges(
        v,
        &[(0x207a, 0x207b), (0x208a, 0x208b), (0x2212, 0x2212), (0xfb29, 0xfb29), (0xfe62, 0xfe63), (0xff0b, 0xff0b), (0xff0d, 0xff0d)],
    ) {
        return ES;
    }
    if in_ranges(
        v,
        &[
            (0xa2, 0xa5),
            (0xb0, 0xb1),
            (0x58f, 0x58f),
            (0x609, 0x60a),
            (0x66a, 0x66a),
            (0x9f2, 0x9f3),
            (0x9fb, 0x9fb),
            (0xe3f, 0xe3f),
            (0x17db, 0x17db),
            (0x2030, 0x2034),
            (0x20a0, 0x20cf),
            (0x212e, 0x212e),
            (0x2213, 0x2213),
            (0xfe5f, 0xfe5f),
            (0xfe69, 0xfe6a),
            (0xff03, 0xff05),
            (0xffe0, 0xffe1),
            (0xffe5, 0xffe6),
        ],
    ) {
        return ET;
    }
    if in_ranges(
        v,
        &[
            (0xa0, 0xa0),
            (0x60c, 0x60c),
            (0x202f, 0x202f),
            (0x2044, 0x2044),
            (0xfe50, 0xfe50),
            (0xfe52, 0xfe52),
            (0xfe55, 0xfe55),
            (0xff0c, 0xff0c),
            (0xff0e, 0xff0f),
            (0xff1a, 0xff1a),
        ],
    ) {
        return CS;
    }
    if in_ranges(
        v,
        &[
            (0xb2, 0xb3),
            (0xb9, 0xb9),
            (0x6f0, 0x6f9),
            (0x2070, 0x2070),
            (0x2074, 0x2079),
            (0x2080, 0x2089),
            (0x2488, 0x249b),
            (0xff10, 0xff19),
            (0x1d7ce, 0x1d7ff),
        ],
    ) {
        return EN;
    }
    if in_ranges(
        v,
        &[(0x600, 0x605), (0x660, 0x669), (0x66b, 0x66c), (0x6dd, 0x6dd), (0x890, 0x891), (0x8e2, 0x8e2), (0x10d30, 0x10d39), (0x10e60, 0x10e7e)],
    ) {
        return AN;
    }
    if in_ranges(
        v,
        &[
            (0x300, 0x36f),
            (0x483, 0x489),
            (0x591, 0x5bd),
            (0x5bf, 0x5bf),
            (0x5c1, 0x5c2),
            (0x5c4, 0x5c5),
            (0x5c7, 0x5c7),
            (0x610, 0x61a),
            (0x64b, 0x65f),
            (0x670, 0x670),
            (0x6d6, 0x6dc),
            (0x6df, 0x6e4),
            (0x6e7, 0x6e8),
            (0x6ea, 0x6ed),
            (0x711, 0x711),
            (0x730, 0x74a),
            (0x7a6, 0x7b0),
            (0x7eb, 0x7f3),
            (0x816, 0x819),
            (0x81b, 0x823),
            (0x825, 0x827),
            (0x829, 0x82d),
            (0x859, 0x85b),
            (0x898, 0x89f),
            (0x8ca, 0x8e1),
            (0x8e3, 0x8ff),
            (0x900, 0x902),
            (0x93a, 0x93a),
            (0x93c, 0x93c),
            (0x941, 0x948),
            (0x94d, 0x94d),
            (0x1ab0, 0x1aff),
            (0x1dc0, 0x1dff),
            (0x20d0, 0x20ff),
            (0x2cef, 0x2cf1),
            (0x2de0, 0x2dff),
            (0x302a, 0x302d),
            (0x3099, 0x309a),
            (0xfb1e, 0xfb1e),
            (0xfe00, 0xfe0f),
            (0xfe20, 0xfe2f),
        ],
    ) {
        return NSM;
    }
    // Neutral symbols inside the Arabic block.
    if in_ranges(v, &[(0x606, 0x607), (0x60e, 0x60f), (0x6de, 0x6de), (0x6e9, 0x6e9)]) {
        return ON;
    }
    if in_ranges(
        v,
        &[
            (0x590, 0x5ff),
            (0x7c0, 0x7ea),
            (0x7f4, 0x7f5),
            (0x7fa, 0x7ff),
            (0x800, 0x815),
            (0x81a, 0x81a),
            (0x824, 0x824),
            (0x828, 0x828),
            (0x82e, 0x858),
            (0x85c, 0x85f),
            (0xfb1d, 0xfb28),
            (0xfb2a, 0xfb4f),
            (0x10800, 0x10cff),
            (0x10d40, 0x10ebf),
            (0x10f00, 0x10f2f),
            (0x10f70, 0x10fff),
            (0x1e800, 0x1edff),
            (0x1ef00, 0x1efff),
        ],
    ) {
        return R;
    }
    if in_ranges(
        v,
        &[
            (0x608, 0x608),
            (0x60b, 0x60b),
            (0x60d, 0x60d),
            (0x61b, 0x64a),
            (0x66d, 0x66f),
            (0x671, 0x6d5),
            (0x6e5, 0x6e6),
            (0x6ee, 0x6ef),
            (0x6fa, 0x710),
            (0x712, 0x72f),
            (0x74b, 0x7a5),
            (0x7b1, 0x7bf),
            (0x860, 0x88f),
            (0x8a0, 0x8c9),
            (0xfb50, 0xfdcf),
            (0xfdf0, 0xfdff),
            (0xfe70, 0xfefe),
            (0x10d00, 0x10d2f),
            (0x10ec0, 0x10eff),
            (0x10f30, 0x10f6f),
            (0x1ee00, 0x1eeff),
        ],
    ) {
        return AL;
    }
    if c.is_alphabetic() || c.is_numeric() {
        L
    } else {
        ON
    }
}

/// Opening and closing characters of bracket pairs that take part in rule N0.
const BRACKETS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('\u{f3a}', '\u{f3b}'),
    ('\u{f3c}', '\u{f3d}'),
    ('\u{169b}', '\u{169c}'),
    ('\u{2045}', '\u{2046}'),
    ('\u{207d}', '\u{207e}'),
    ('\u{208d}', '\u{208e}'),
    ('\u{2308}', '\u{2309}'),
    ('\u{230a}', '\u{230b}'),
    ('\u{3008}', '\u{3009}'),
    ('\u{2768}', '\u{2769}'),
    ('\u{276a}', '\u{276b}'),
    ('\u{276c}', '\u{276d}'),
    ('\u{276e}', '\u{276f}'),
    ('\u{2770}', '\u{2771}'),
    ('\u{2772}', '\u{2773}'),
    ('\u{2774}', '\u{2775}'),
    ('\u{27c5}', '\u{27c6}'),
    ('\u{27e6}', '\u{27e7}'),
    ('\u{27e8}', '\u{27e9}'),
    ('\u{27ea}', '\u{27eb}'),
    ('\u{27ec}', '\u{27ed}'),
    ('\u{27ee}', '\u{27ef}'),
    ('\u{2983}', '\u{2984}'),
    ('\u{2985}', '\u{2986}'),
    ('\u{2987}', '\u{2988}'),
    ('\u{2989}', '\u{298a}'),
    ('\u{298b}', '\u{298c}'),
    ('\u{2991}', '\u{2992}'),
    ('\u{2993}', '\u{2994}'),
    ('\u{2995}', '\u{2996}'),
    ('\u{2997}', '\u{2998}'),
    ('\u{29d8}', '\u{29d9}'),
    ('\u{29da}', '\u{29db}'),
    ('\u{29fc}', '\u{29fd}'),
    ('\u{2e22}', '\u{2e23}'),
    ('\u{2e24}', '\u{2e25}'),
    ('\u{2e26}', '\u{2e27}'),
    ('\u{2e28}', '\u{2e29}'),
    ('\u{300a}', '\u{300b}'),
    ('\u{300c}', '\u{300d}'),
    ('\u{300e}', '\u{300f}'),
    ('\u{3010}', '\u{3011}'),
    ('\u{3014}', '\u{3015}'),
    ('\u{3016}', '\u{3017}'),
    ('\u{3018}', '\u{3019}'),
    ('\u{301a}', '\u{301b}'),
    ('\u{fe59}', '\u{fe5a}'),
    ('\u{fe5b}', '\u{fe5c}'),
    ('\u{fe5d}', '\u{fe5e}'),
    ('\u{ff08}', '\u{ff09}'),
    ('\u{ff3b}', '\u{ff3d}'),
    ('\u{ff5b}', '\u{ff5d}'),
    ('\u{ff5f}', '\u{ff60}'),
    ('\u{ff62}', '\u{ff63}'),
];

/// Characters that mirror without being brackets for rule N0.
const MIRROR_EXTRA: &[(char, char)] = &[('<', '>'), ('\u{ab}', '\u{bb}'), ('\u{2039}', '\u{203a}'), ('\u{2264}', '\u{2265}')];

fn canonical_bracket(c: char) -> char {
    match c {
        '\u{2329}' => '\u{3008}',
        '\u{232a}' => '\u{3009}',
        _ => c,
    }
}

/// For a bracket character: the canonical opening character of its pair, and whether it opens.
fn bracket_of(c: char) -> Option<(char, bool)> {
    let c = canonical_bracket(c);
    BRACKETS.iter().find_map(|&(o, cl)| {
        if o == c {
            Some((o, true))
        } else if cl == c {
            Some((o, false))
        } else {
            None
        }
    })
}

/// The mirrored form of a character (rule L4): the opposite bracket or comparison sign, or the character itself.
pub fn mirrored(c: char) -> char {
    let c2 = canonical_bracket(c);
    for &(a, b) in BRACKETS.iter().chain(MIRROR_EXTRA) {
        if c2 == a {
            return b;
        }
        if c2 == b {
            return a;
        }
    }
    c
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
    /// The paragraph embedding level of the first paragraph: 0 for left to right, 1 for right to left.
    pub level: u8,
    /// The resolved level of each character, in logical order, with rule L1 applied as if the text were one line.
    pub levels: Vec<u8>,
    /// The runs in visual order (left to right on screen) of the whole text as one line.
    pub runs: Vec<Run>,
    raw_levels: Vec<u8>,
    classes: Vec<BidiClass>,
    para_levels: Vec<u8>,
    chars: Vec<char>,
}

impl Paragraph {
    /// Resolves `text`. A paragraph separator inside the text starts a new paragraph, each with its own level.
    pub fn new(text: &str, direction: Direction) -> Self {
        let chars: Vec<char> = text.chars().collect();
        Self::build(chars, direction, None)
    }

    /// Resolves `text` with writing direction attributes over character ranges. Ranges may nest; ranges outside the
    /// text are clamped.
    pub fn with_attributes(text: &str, direction: Direction, attrs: &[(Range<usize>, WritingDirection)]) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut sorted: Vec<(usize, usize, WritingDirection)> = attrs
            .iter()
            .map(|(r, d)| (r.start.min(n), r.end.min(n), *d))
            .filter(|t| t.0 < t.1)
            .collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut out: Vec<char> = Vec::with_capacity(n + 2 * sorted.len());
        let mut keep: Vec<bool> = Vec::with_capacity(n + 2 * sorted.len());
        let mut open_ends: Vec<usize> = Vec::new();
        let mut next = 0;
        for pos in 0..=n {
            while open_ends.last().is_some_and(|&e| e == pos) {
                open_ends.pop();
                out.push('\u{202c}');
                keep.push(false);
            }
            while next < sorted.len() && sorted[next].0 == pos {
                out.push(sorted[next].2.open());
                keep.push(false);
                open_ends.push(sorted[next].1);
                next += 1;
            }
            if pos < n {
                out.push(chars[pos]);
                keep.push(true);
            }
        }
        Self::build(out, direction, Some(keep))
    }

    fn build(chars: Vec<char>, direction: Direction, keep: Option<Vec<bool>>) -> Self {
        let r = resolve_text(&chars, direction);
        fn filter<T: Copy>(v: &[T], keep: &Option<Vec<bool>>) -> Vec<T> {
            match keep {
                Some(k) => v.iter().zip(k).filter(|(_, &k)| k).map(|(&x, _)| x).collect(),
                None => v.to_vec(),
            }
        }
        let raw_levels = filter(&r.levels, &keep);
        let para_levels = filter(&r.para_levels, &keep);
        let classes = filter(&r.classes, &keep);
        let chars = filter(&chars, &keep);
        let level = para_levels.first().copied().unwrap_or(match direction {
            Direction::RightToLeft => 1,
            _ => 0,
        });
        let mut p = Paragraph { level, levels: Vec::new(), runs: Vec::new(), raw_levels, classes, para_levels, chars };
        p.levels = p.line_levels(0..p.chars.len());
        p.runs = visual_runs(&p.levels);
        p
    }

    /// Whether the first paragraph is right to left.
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }

    /// Number of characters.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Logical character indices in visual order, the whole text as one line.
    pub fn visual_order(&self) -> Vec<usize> {
        reorder_line(&self.levels)
    }

    /// Rule L1 for the line `line` (logical character indices): separators and the whitespace and isolate
    /// formatting characters before them or at the end of the line take the paragraph level. Returns one level per
    /// character of the line.
    pub fn line_levels(&self, line: Range<usize>) -> Vec<u8> {
        let end = line.end.min(self.chars.len());
        let start = line.start.min(end);
        let mut lv: Vec<u8> = self.raw_levels[start..end].to_vec();
        let mut trailing = true;
        for i in (start..end).rev() {
            let c = self.classes[i];
            match c {
                BidiClass::B | BidiClass::S => {
                    lv[i - start] = self.para_levels[i];
                    trailing = true;
                }
                BidiClass::WS | BidiClass::FSI | BidiClass::LRI | BidiClass::RLI | BidiClass::PDI => {
                    if trailing {
                        lv[i - start] = self.para_levels[i];
                    }
                }
                c if c.is_removed() => {
                    if trailing {
                        lv[i - start] = self.para_levels[i];
                    }
                }
                _ => trailing = false,
            }
        }
        lv
    }

    /// The runs of the line `line` in visual order. `chars` of each run are logical character indices of the text.
    pub fn line_runs(&self, line: Range<usize>) -> Vec<Run> {
        let start = line.start;
        visual_runs(&self.line_levels(line))
            .into_iter()
            .map(|r| Run { chars: r.chars.start + start..r.chars.end + start, level: r.level })
            .collect()
    }

    /// The logical character indices of the line `line` in visual order.
    pub fn line_order(&self, line: Range<usize>) -> Vec<usize> {
        let start = line.start;
        reorder_line(&self.line_levels(line)).into_iter().map(|i| i + start).collect()
    }

    /// The characters of the line in visual order, mirrored where they sit in a right to left run (rule L4).
    pub fn display_chars(&self, line: Range<usize>) -> Vec<char> {
        let levels = self.line_levels(line.clone());
        let start = line.start;
        reorder_line(&levels)
            .into_iter()
            .map(|i| {
                let c = self.chars[start + i];
                if levels[i] % 2 == 1 {
                    mirrored(c)
                } else {
                    c
                }
            })
            .collect()
    }

    /// The character at logical index `i`, mirrored when its level is odd.
    pub fn display_char(&self, i: usize) -> char {
        let c = self.chars[i];
        if self.levels[i] % 2 == 1 {
            mirrored(c)
        } else {
            c
        }
    }
}

struct Resolved {
    levels: Vec<u8>,
    classes: Vec<BidiClass>,
    para_levels: Vec<u8>,
}

/// Resolves all paragraphs of `chars`. The levels have no L1 applied.
fn resolve_text(chars: &[char], direction: Direction) -> Resolved {
    let classes: Vec<BidiClass> = chars.iter().map(|&c| bidi_class(c)).collect();
    let mut levels = Vec::with_capacity(chars.len());
    let mut para_levels = Vec::with_capacity(chars.len());
    let mut start = 0;
    while start < chars.len() {
        let mut end = start;
        while end < chars.len() && classes[end] != BidiClass::B {
            end += 1;
        }
        if end < chars.len() {
            end += 1;
        }
        let (pl, lv) = resolve_paragraph(&chars[start..end], &classes[start..end], direction);
        levels.extend(lv);
        para_levels.extend(std::iter::repeat(pl).take(end - start));
        start = end;
    }
    Resolved { levels, classes, para_levels }
}

/// Indices of the matching PDI of each isolate initiator, and of the initiator of each matched PDI (BD9).
fn match_isolates(cls: &[BidiClass]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = cls.len();
    let mut pdi_of = vec![None; n];
    let mut init_of = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        if cls[i].is_isolate_initiator() {
            stack.push(i);
        } else if cls[i] == BidiClass::PDI {
            if let Some(j) = stack.pop() {
                pdi_of[j] = Some(i);
                init_of[i] = Some(j);
            }
        }
    }
    (pdi_of, init_of)
}

/// P2 and P3: the level (0 or 1) of the first strong character in `start..end`, skipping isolates.
fn first_strong(cls: &[BidiClass], pdi_of: &[Option<usize>], start: usize, end: usize) -> Option<u8> {
    let mut i = start;
    while i < end {
        match cls[i] {
            BidiClass::L => return Some(0),
            BidiClass::R | BidiClass::AL => return Some(1),
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => match pdi_of[i] {
                Some(j) => i = j,
                None => return None,
            },
            _ => {}
        }
        i += 1;
    }
    None
}

const MAX_DEPTH: u8 = 125;

#[derive(Clone, Copy)]
struct Status {
    level: u8,
    over: Option<BidiClass>,
    isolate: bool,
}

fn next_odd(l: u8) -> u8 {
    (l + 1) | 1
}

fn next_even(l: u8) -> u8 {
    (l + 2) & !1
}

/// Resolves one paragraph. Returns its paragraph level and the level of each character.
fn resolve_paragraph(chars: &[char], orig: &[BidiClass], direction: Direction) -> (u8, Vec<u8>) {
    use BidiClass::*;
    let n = orig.len();
    let (pdi_of, init_of) = match_isolates(orig);
    let para = match direction {
        Direction::LeftToRight => 0,
        Direction::RightToLeft => 1,
        Direction::Natural => first_strong(orig, &pdi_of, 0, n).unwrap_or(0),
    };

    // X1 to X8: explicit levels. `cls` holds the classes after directional overrides.
    let mut cls = orig.to_vec();
    let mut levels = vec![para; n];
    let base = Status { level: para, over: None, isolate: false };
    let mut stack = vec![base];
    let (mut overflow_isolate, mut overflow_embedding, mut valid_isolate) = (0usize, 0usize, 0usize);
    for i in 0..n {
        let top = *stack.last().unwrap_or(&base);
        let c = orig[i];
        match c {
            RLE | LRE | RLO | LRO => {
                levels[i] = top.level;
                let new = if matches!(c, RLE | RLO) { next_odd(top.level) } else { next_even(top.level) };
                if new <= MAX_DEPTH && overflow_isolate == 0 && overflow_embedding == 0 {
                    let over = match c {
                        RLO => Some(R),
                        LRO => Some(L),
                        _ => None,
                    };
                    stack.push(Status { level: new, over, isolate: false });
                } else if overflow_isolate == 0 {
                    overflow_embedding += 1;
                }
            }
            RLI | LRI | FSI => {
                levels[i] = top.level;
                if let Some(o) = top.over {
                    cls[i] = o;
                }
                let rtl = match c {
                    RLI => true,
                    LRI => false,
                    _ => first_strong(orig, &pdi_of, i + 1, pdi_of[i].unwrap_or(n)) == Some(1),
                };
                let new = if rtl { next_odd(top.level) } else { next_even(top.level) };
                if new <= MAX_DEPTH && overflow_isolate == 0 && overflow_embedding == 0 {
                    valid_isolate += 1;
                    stack.push(Status { level: new, over: None, isolate: true });
                } else {
                    overflow_isolate += 1;
                }
            }
            PDI => {
                if overflow_isolate > 0 {
                    overflow_isolate -= 1;
                } else if valid_isolate > 0 {
                    overflow_embedding = 0;
                    while stack.len() > 1 && stack.last().is_some_and(|s| !s.isolate) {
                        stack.pop();
                    }
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    valid_isolate -= 1;
                }
                let top = *stack.last().unwrap_or(&base);
                levels[i] = top.level;
                if let Some(o) = top.over {
                    cls[i] = o;
                }
            }
            PDF => {
                levels[i] = top.level;
                if overflow_isolate > 0 {
                    // an overflowing isolate swallows the embedding end
                } else if overflow_embedding > 0 {
                    overflow_embedding -= 1;
                } else if !top.isolate && stack.len() >= 2 {
                    stack.pop();
                }
            }
            B => levels[i] = para,
            BN => levels[i] = top.level,
            _ => {
                levels[i] = top.level;
                if let Some(o) = top.over {
                    cls[i] = o;
                }
            }
        }
    }

    // X10: isolating run sequences over the characters that were not removed.
    let kept: Vec<usize> = (0..n).filter(|&i| !orig[i].is_removed()).collect();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut run_of = vec![usize::MAX; n];
    for &i in &kept {
        let new_run = match runs.last() {
            Some(r) => levels[r[0]] != levels[i],
            None => true,
        };
        if new_run {
            runs.push(Vec::new());
        }
        let r = runs.len() - 1;
        runs[r].push(i);
        run_of[i] = r;
    }
    let mut final_levels = levels.clone();
    for r in 0..runs.len() {
        let first = runs[r][0];
        // A run that starts with the matching PDI of an initiator that ends a run belongs to that initiator's
        // sequence.
        if orig[first] == PDI {
            if let Some(j) = init_of[first] {
                if run_of[j] != usize::MAX && runs[run_of[j]].last() == Some(&j) {
                    continue;
                }
            }
        }
        let mut seq = runs[r].clone();
        loop {
            let last = *seq.last().unwrap_or(&first);
            if !orig[last].is_isolate_initiator() {
                break;
            }
            let Some(p) = pdi_of[last] else { break };
            let r2 = run_of[p];
            if r2 == usize::MAX || runs[r2][0] != p {
                break;
            }
            seq.extend(runs[r2].iter().copied());
        }
        let level = levels[first];
        let before = (0..first).rev().find(|&k| !orig[k].is_removed()).map(|k| levels[k]).unwrap_or(para);
        let last = *seq.last().unwrap_or(&first);
        let after = if orig[last].is_isolate_initiator() {
            para
        } else {
            (last + 1..n).find(|&k| !orig[k].is_removed()).map(|k| levels[k]).unwrap_or(para)
        };
        let sos = if level.max(before) % 2 == 1 { R } else { L };
        let eos = if level.max(after) % 2 == 1 { R } else { L };
        resolve_sequence(&seq, sos, eos, level, &cls, orig, chars, &mut final_levels);
    }

    // Removed characters follow the level of the character before them.
    for i in 0..n {
        if orig[i].is_removed() {
            final_levels[i] = if i == 0 { para } else { final_levels[i - 1] };
        }
    }
    (para, final_levels)
}

/// Rules W1 to W7, N0 to N2 and I1, I2 on one isolating run sequence.
#[allow(clippy::too_many_arguments)]
fn resolve_sequence(
    idxs: &[usize],
    sos: BidiClass,
    eos: BidiClass,
    level: u8,
    cls: &[BidiClass],
    orig: &[BidiClass],
    chars: &[char],
    out: &mut [u8],
) {
    use BidiClass::*;
    let n = idxs.len();
    let mut t: Vec<BidiClass> = idxs.iter().map(|&i| cls[i]).collect();

    // W1
    let mut prev = sos;
    for k in 0..n {
        if t[k] == NSM {
            let after_isolate_mark = k > 0 && {
                let o = orig[idxs[k - 1]];
                o.is_isolate_initiator() || o == PDI
            };
            t[k] = if after_isolate_mark { ON } else { prev };
        }
        prev = t[k];
    }
    // W2 and W3
    let mut last_strong = sos;
    for k in 0..n {
        match t[k] {
            L | R | AL => last_strong = t[k],
            EN if last_strong == AL => t[k] = AN,
            _ => {}
        }
    }
    for c in t.iter_mut() {
        if *c == AL {
            *c = R;
        }
    }
    // W4
    for k in 1..n.saturating_sub(1) {
        let (p, q) = (t[k - 1], t[k + 1]);
        match t[k] {
            ES if p == EN && q == EN => t[k] = EN,
            CS if p == q && (p == EN || p == AN) => t[k] = p,
            _ => {}
        }
    }
    // W5
    let mut k = 0;
    while k < n {
        if t[k] != ET {
            k += 1;
            continue;
        }
        let s = k;
        while k < n && t[k] == ET {
            k += 1;
        }
        if (s > 0 && t[s - 1] == EN) || (k < n && t[k] == EN) {
            t[s..k].fill(EN);
        }
    }
    // W6
    for c in t.iter_mut() {
        if matches!(*c, ES | ET | CS) {
            *c = ON;
        }
    }
    // W7
    let mut last = sos;
    for c in t.iter_mut() {
        match *c {
            L | R => last = *c,
            EN if last == L => *c = L,
            _ => {}
        }
    }

    let e = if level % 2 == 1 { R } else { L };
    let opp = if e == L { R } else { L };
    let strong_n0 = |c: BidiClass| match c {
        L => Some(L),
        R | EN | AN => Some(R),
        _ => None,
    };

    // N0: paired brackets (BD16).
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for k in 0..n {
        if t[k] != ON {
            continue;
        }
        match bracket_of(chars[idxs[k]]) {
            Some((id, true)) => {
                if open.len() >= 63 {
                    break;
                }
                open.push((id, k));
            }
            Some((id, false)) => {
                if let Some(p) = open.iter().rposition(|&(i, _)| i == id) {
                    pairs.push((open[p].1, k));
                    open.truncate(p);
                }
            }
            None => {}
        }
    }
    pairs.sort();
    for (o, c) in pairs {
        let mut found_e = false;
        let mut found_o = false;
        for k in o + 1..c {
            match strong_n0(t[k]) {
                Some(d) if d == e => {
                    found_e = true;
                    break;
                }
                Some(_) => found_o = true,
                None => {}
            }
        }
        let new = if found_e {
            Some(e)
        } else if found_o {
            let ctx = (0..o).rev().find_map(|k| strong_n0(t[k])).unwrap_or(sos);
            Some(if ctx == opp { opp } else { e })
        } else {
            None
        };
        if let Some(d) = new {
            for &b in &[o, c] {
                t[b] = d;
                let mut k = b + 1;
                while k < n && orig[idxs[k]] == NSM {
                    t[k] = d;
                    k += 1;
                }
            }
        }
    }

    // N1 and N2
    let side = |c: BidiClass| if c == L { L } else { R };
    let mut k = 0;
    while k < n {
        if !t[k].is_neutral() {
            k += 1;
            continue;
        }
        let s = k;
        while k < n && t[k].is_neutral() {
            k += 1;
        }
        let before = if s == 0 { side(sos) } else { side(t[s - 1]) };
        let after = if k == n { side(eos) } else { side(t[k]) };
        let dir = if before == after { before } else { e };
        t[s..k].fill(dir);
    }

    // I1 and I2
    for k in 0..n {
        let add = if level % 2 == 0 {
            match t[k] {
                R => 1,
                AN | EN => 2,
                _ => 0,
            }
        } else {
            match t[k] {
                L | EN | AN => 1,
                _ => 0,
            }
        };
        out[idxs[k]] = level + add;
    }
}

/// Rule L2: from the highest level down to the lowest odd level, reverse every run of characters at that level or
/// higher. Takes the levels of one line and returns the logical indices (into the line) in visual order.
pub fn reorder_line(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let min = match levels.iter().min() {
        Some(&m) => m,
        None => return order,
    };
    let max = levels.iter().copied().max().unwrap_or(0);
    let lowest_odd = if min % 2 == 1 { min } else { min + 1 };
    let mut k = max;
    while k >= lowest_odd && k > 0 {
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

/// The same as [`reorder_line`]; the older name.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    reorder_line(levels)
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
    reorder_line(&block_levels).into_iter().map(|b| blocks[b].clone()).collect()
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

    #[test]
    fn explicit_embedding_raises_the_level() {
        let p = Paragraph::new("a\u{202b}b\u{202c}c", Direction::LeftToRight);
        assert_eq!(p.levels[0], 0);
        assert_eq!(p.levels[2], 2);
        assert_eq!(p.levels[4], 0);
    }

    #[test]
    fn override_forces_direction() {
        let p = Paragraph::new("\u{202e}ab\u{202c}", Direction::LeftToRight);
        assert_eq!(p.levels[1], 1);
        assert_eq!(p.levels[2], 1);
    }

    #[test]
    fn isolate_keeps_its_content_apart() {
        let p = Paragraph::new("a \u{2067}\u{5d0}\u{5d1}\u{2069} b", Direction::LeftToRight);
        assert_eq!(p.levels, vec![0, 0, 0, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn first_strong_isolate_picks_its_own_direction() {
        let p = Paragraph::new("\u{2068}\u{5d0}\u{5d1}\u{2069}", Direction::LeftToRight);
        assert_eq!(p.levels, vec![0, 1, 1, 0]);
    }

    #[test]
    fn natural_direction_skips_isolates() {
        let p = Paragraph::new("\u{2067}\u{5d0}\u{2069}a", Direction::Natural);
        assert_eq!(p.level, 0);
    }

    #[test]
    fn digits_after_an_arabic_letter_become_arabic_numbers() {
        let p = Paragraph::new("\u{628}1", Direction::LeftToRight);
        assert_eq!(p.levels, vec![1, 2]);
    }

    #[test]
    fn paired_brackets_follow_their_content() {
        // The brackets enclose right to left text that follows right to left text, so they are right to left.
        let p = Paragraph::new("\u{5d0}\u{5d1}(\u{5d2}\u{5d3})e", Direction::LeftToRight);
        assert_eq!(p.levels, vec![1, 1, 1, 1, 1, 1, 0]);
    }

    #[test]
    fn line_levels_reset_trailing_space_per_line() {
        let p = Paragraph::new("\u{5d0} \u{5d1} a", Direction::LeftToRight);
        assert_eq!(p.line_levels(0..2), vec![1, 0]);
        assert_eq!(p.line_levels(0..3), vec![1, 1, 1]);
    }

    #[test]
    fn writing_direction_attribute_overrides_a_range() {
        let p = Paragraph::with_attributes("abc", Direction::LeftToRight, &[(0..2, WritingDirection::RightToLeftOverride)]);
        assert_eq!(p.levels, vec![1, 1, 0]);
        assert_eq!(p.len(), 3);
        assert_eq!(p.visual_order(), vec![1, 0, 2]);
    }

    #[test]
    fn embedding_attribute_raises_the_level_of_its_range() {
        let p = Paragraph::with_attributes("ab", Direction::LeftToRight, &[(0..2, WritingDirection::RightToLeftEmbedding)]);
        assert_eq!(p.levels, vec![2, 2]);
    }

    #[test]
    fn mirroring_swaps_brackets_in_right_to_left_runs() {
        assert_eq!(mirrored('('), ')');
        assert_eq!(mirrored('>'), '<');
        assert_eq!(mirrored('a'), 'a');
        let p = Paragraph::new("\u{5d0}(", Direction::RightToLeft);
        assert_eq!(p.display_char(1), ')');
        assert_eq!(p.display_chars(0..2), vec![')', '\u{5d0}']);
    }

    #[test]
    fn reorder_line_nested_levels() {
        assert_eq!(reorder_line(&[0, 1, 1, 0]), vec![0, 2, 1, 3]);
        assert_eq!(reorder_line(&[1, 2, 2]), vec![1, 2, 0]);
        assert!(reorder_line(&[]).is_empty());
    }

    #[test]
    fn separate_paragraphs_get_their_own_level() {
        let p = Paragraph::new("a\n\u{5d0}", Direction::Natural);
        assert_eq!(p.levels, vec![0, 0, 1]);
    }

    #[test]
    fn deep_nesting_does_not_panic() {
        let mut s = String::new();
        for _ in 0..200 {
            s.push('\u{202b}');
        }
        s.push('x');
        let p = Paragraph::new(&s, Direction::LeftToRight);
        assert!(p.levels.iter().all(|&l| l <= MAX_DEPTH + 2));
    }
}
