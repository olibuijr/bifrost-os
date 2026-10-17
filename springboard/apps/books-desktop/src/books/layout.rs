//! Pagination: flows a book's blocks into screen-sized pages at a given
//! font size. Pure arithmetic over a text-measuring callback, so it is
//! testable without fonts. Positions are `(block, char offset)` pairs, which
//! stay valid when the font size changes and the pages are rebuilt.

use super::engine::Block;
use super::store::Pos;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// text column width, px
    pub width: f32,
    /// text column height, px
    pub height: f32,
    /// body font size, px (already including Dynamic Type)
    pub body: f32,
    /// line height as a multiple of the font size (the Display panel's line spacing)
    pub line_height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Body,
    /// heading level 1..=6, drawn bold at `Metrics::body * heading_scale`
    Heading(u8),
    /// a picture shown at `w` x `h` px
    Image { w: f32, h: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// top of the line within the page column, px
    pub y: f32,
    pub kind: Kind,
    pub block: usize,
    /// char offset of the line's first char in the block's text
    pub offset: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub lines: Vec<Line>,
}

impl Page {
    pub fn start(&self) -> Option<Pos> {
        self.lines.first().map(|l| Pos { block: l.block, offset: l.offset })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
}

#[cfg(test)]
pub const LINE_HEIGHT: f32 = 1.42;
const PARA_GAP: f32 = 0.55;
const HEADING_BEFORE: f32 = 1.0;
const HEADING_AFTER: f32 = 0.45;

/// Columns in a wrapped line (whose whitespace runs are collapsed to one space) that
/// correspond to the original char range `a..b` of `text`, where the line starts at
/// original char `line_off`. Keeps highlights aligned when the source repeats whitespace.
pub fn line_cols(text: &str, line_off: usize, line: &str, a: usize, b: usize) -> (usize, usize) {
    let orig: Vec<char> = text.chars().skip(line_off).collect();
    let line: Vec<char> = line.chars().collect();
    let col = |pos: usize| {
        let target = pos.saturating_sub(line_off);
        let (mut o, mut l) = (0, 0);
        while l < line.len() && o < target && o < orig.len() {
            let was_space = line[l] == ' ';
            l += 1;
            o += 1;
            while was_space && o < orig.len() && orig[o].is_whitespace() && orig[o] != '\u{a0}' {
                o += 1;
            }
        }
        l
    };
    (col(a), col(b))
}

/// True when `line`, a wrapped line of `text` that starts at char `line_off`, is the last line of its paragraph: only
/// white space follows it before the next explicit line break or the end of the text. A justified page leaves such a
/// line ragged.
pub fn ends_paragraph(text: &str, line_off: usize, line: &str) -> bool {
    // walk the original text across the line the way `line_cols` does: a space in the line stands for a run of white space
    let mut orig = text.chars().skip(line_off).peekable();
    for c in line.chars() {
        if orig.next().is_none() {
            break;
        }
        if c == ' ' {
            while orig.next_if(|w| w.is_whitespace() && *w != '\u{a0}' && *w != '\n').is_some() {}
        }
    }
    orig.take_while(|c| *c != '\n').all(|c| c.is_whitespace())
}

/// The extra width each of the `spaces` of a justified line takes so that the line, `natural` px wide, fills `max` px.
/// A line that is already full, or has no space, gets none; so does a line so short that the spaces would gape (more
/// than twice a normal space wider, which happens before a picture or at the end of a block).
pub fn justify_extra(natural: f32, max: f32, spaces: usize, space_w: f32) -> f32 {
    if spaces == 0 || max <= natural {
        return 0.0;
    }
    let extra = (max - natural) / spaces as f32;
    if extra > space_w * 2.0 {
        0.0
    } else {
        extra
    }
}

pub fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 1.7,
        2 => 1.45,
        3 => 1.25,
        _ => 1.1,
    }
}

/// Wraps `text` to `max` px, returning `(char offset, line)` pairs. Explicit
/// `\n` forces a break; a word wider than a line is split by characters.
pub fn wrap(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> Vec<(usize, String)> {
    let space = measure(" ");
    let mut out: Vec<(usize, String)> = vec![];
    let mut at = 0usize; // char offset of the start of the current segment
    for segment in text.split('\n') {
        let mut line = String::new();
        let mut line_w = 0.0f32;
        let mut line_off = at;
        let mut pos = at;
        let chars: Vec<char> = segment.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() && chars[i] != '\u{a0}' {
                i += 1;
                pos += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !(chars[i].is_whitespace() && chars[i] != '\u{a0}') {
                i += 1;
            }
            let mut word: String = chars[start..i].iter().collect();
            let mut word_off = pos;
            pos += i - start;
            let mut w = measure(&word);
            // a word that cannot fit even alone: peel off what fits
            while w > max && word.chars().count() > 1 {
                if !line.is_empty() {
                    out.push((line_off, std::mem::take(&mut line)));
                    line_w = 0.0;
                }
                let mut cut = 0;
                let mut acc = String::new();
                for (k, c) in word.chars().enumerate() {
                    acc.push(c);
                    if measure(&acc) > max && k > 0 {
                        acc.pop();
                        break;
                    }
                    cut = k + 1;
                }
                let cut = cut.max(1);
                let head: String = word.chars().take(cut).collect();
                out.push((word_off, head));
                word = word.chars().skip(cut).collect();
                word_off += cut;
                w = measure(&word);
            }
            if line.is_empty() {
                line = word;
                line_w = w;
                line_off = word_off;
            } else if line_w + space + w <= max {
                line.push(' ');
                line.push_str(&word);
                line_w += space + w;
            } else {
                out.push((line_off, std::mem::take(&mut line)));
                line = word;
                line_w = w;
                line_off = word_off;
            }
        }
        if !line.is_empty() {
            out.push((line_off, line));
        }
        at += segment.chars().count() + 1;
    }
    out
}

/// Flows `blocks` into pages. `measure(text, px, bold)` gives a width in px.
pub fn paginate(blocks: &[Block], m: Metrics, measure: &dyn Fn(&str, f32, bool) -> f32) -> Layout {
    let body_lh = m.body * m.line_height;
    let mut pages: Vec<Page> = vec![];
    let mut cur = Page::default();
    let mut y = 0.0f32;

    let new_page = |cur: &mut Page, y: &mut f32, pages: &mut Vec<Page>| {
        if !cur.lines.is_empty() {
            pages.push(std::mem::take(cur));
        }
        *y = 0.0;
    };

    for (bi, b) in blocks.iter().enumerate() {
        match b {
            Block::Image(img) => {
                if img.w == 0 || img.h == 0 {
                    continue;
                }
                let s = (m.width / img.w as f32).min(m.height * 0.92 / img.h as f32).min(2.0);
                let (w, h) = (img.w as f32 * s, img.h as f32 * s);
                let gap = if cur.lines.is_empty() { 0.0 } else { m.body * PARA_GAP };
                if y + gap + h > m.height && !cur.lines.is_empty() {
                    new_page(&mut cur, &mut y, &mut pages);
                }
                let gap = if cur.lines.is_empty() { 0.0 } else { m.body * PARA_GAP };
                cur.lines.push(Line { y: y + gap, kind: Kind::Image { w, h }, block: bi, offset: 0, text: String::new() });
                y += gap + h;
            }
            Block::Heading(_, text) | Block::Para(text) => {
                let (kind, px, bold) = match b {
                    Block::Heading(l, _) => (Kind::Heading(*l), m.body * heading_scale(*l), true),
                    _ => (Kind::Body, m.body, false),
                };
                let lines = wrap(text, m.width, &|s| measure(s, px, bold));
                if lines.is_empty() {
                    continue;
                }
                let lh = px * m.line_height;
                let before = match kind {
                    Kind::Heading(_) => m.body * HEADING_BEFORE,
                    _ => m.body * PARA_GAP,
                };
                // a heading keeps company with the first lines under it
                let need = match kind {
                    Kind::Heading(_) => lh * lines.len() as f32 + before + 2.0 * body_lh,
                    _ => lh + before,
                };
                if !cur.lines.is_empty() && y + need > m.height {
                    new_page(&mut cur, &mut y, &mut pages);
                }
                if !cur.lines.is_empty() {
                    y += before;
                }
                for (off, line) in lines {
                    if y + lh > m.height && !cur.lines.is_empty() {
                        new_page(&mut cur, &mut y, &mut pages);
                    }
                    cur.lines.push(Line { y, kind, block: bi, offset: off, text: line });
                    y += lh;
                }
                if matches!(kind, Kind::Heading(_)) {
                    y += m.body * HEADING_AFTER;
                }
            }
        }
    }
    if !cur.lines.is_empty() || pages.is_empty() {
        pages.push(cur);
    }
    Layout { pages }
}

impl Layout {
    /// The page showing `pos`: the last page that starts at or before it.
    pub fn page_of(&self, pos: Pos) -> usize {
        let key = (pos.block, pos.offset);
        let after = self.pages.partition_point(|p| p.start().map_or(true, |s| (s.block, s.offset) <= key));
        after.saturating_sub(1)
    }

    /// Where page `i` starts (the book's start for an empty page).
    pub fn pos_of(&self, i: usize) -> Pos {
        self.pages.get(i).and_then(Page::start).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::gfx::Image;

    /// 10 px per char at 20 px font size, scaling with the size.
    fn measure(s: &str, px: f32, _bold: bool) -> f32 {
        s.chars().count() as f32 * px / 2.0
    }

    fn m(width: f32, height: f32) -> Metrics {
        Metrics { width, height, body: 20.0, line_height: LINE_HEIGHT }
    }

    #[test]
    fn wrap_tracks_char_offsets() {
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        let lines = wrap("aaa bbb  ccc dd", 70.0, &w);
        assert_eq!(lines, vec![(0, "aaa bbb".to_string()), (9, "ccc dd".to_string())]);
        let lines = wrap("ab\ncd ef", 1000.0, &w);
        assert_eq!(lines, vec![(0, "ab".to_string()), (3, "cd ef".to_string())]);
        assert!(wrap("   ", 100.0, &w).is_empty());
    }

    #[test]
    fn line_cols_skip_collapsed_whitespace() {
        let text = "one   two \t three";
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        let lines = wrap(text, 500.0, &w);
        assert_eq!(lines[0].1, "one two three");
        // original "three" is chars 12..17
        let (a, b) = line_cols(text, lines[0].0, &lines[0].1, 12, 17);
        assert_eq!(&lines[0].1[a..b], "three");
    }

    #[test]
    fn only_the_last_line_of_a_paragraph_ends_it() {
        let text = "one   two three\nfour five";
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        let lines = wrap(text, 100.0, &w);
        assert_eq!(lines.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["one two", "three", "four five"]);
        let ends: Vec<bool> = lines.iter().map(|(off, line)| ends_paragraph(text, *off, line)).collect();
        assert_eq!(ends, [false, true, true], "a forced break ends the line before it");
        // a long word cut into pieces: only the last piece ends the paragraph
        let cut = wrap("abcdefghij", 40.0, &w);
        let ends: Vec<bool> = cut.iter().map(|(off, line)| ends_paragraph("abcdefghij", *off, line)).collect();
        assert_eq!(ends, [false, false, true]);
        assert!(ends_paragraph("tail   ", 0, "tail"), "trailing white space does not continue the paragraph");
    }

    #[test]
    fn justified_lines_share_the_slack_between_their_spaces() {
        assert_eq!(justify_extra(90.0, 100.0, 5, 4.0), 2.0);
        assert_eq!(justify_extra(100.0, 100.0, 5, 4.0), 0.0, "a full line is left alone");
        assert_eq!(justify_extra(120.0, 100.0, 5, 4.0), 0.0, "so is one that does not fit");
        assert_eq!(justify_extra(50.0, 100.0, 5, 4.0), 0.0, "a short line would gape, so it stays ragged");
        assert_eq!(justify_extra(50.0, 100.0, 0, 4.0), 0.0, "one word has no space to widen");
        assert_eq!(justify_extra(92.0, 100.0, 4, 4.0), 2.0);
    }

    #[test]
    fn line_spacing_changes_how_many_lines_fit_a_page() {
        let blocks: Vec<Block> = (0..20).map(|i| Block::Para(format!("para{i} one two three four five six seven eight"))).collect();
        let tight = paginate(&blocks, Metrics { width: 200.0, height: 300.0, body: 20.0, line_height: 1.25 }, &measure);
        let loose = paginate(&blocks, Metrics { width: 200.0, height: 300.0, body: 20.0, line_height: 1.7 }, &measure);
        assert!(loose.pages.len() > tight.pages.len(), "{} loose pages against {} tight", loose.pages.len(), tight.pages.len());
        for (m, layout) in [(1.25f32, &tight), (1.7, &loose)] {
            for p in &layout.pages {
                let last = p.lines.last().unwrap();
                assert!(last.y + 20.0 * m <= 300.0 + 0.01, "no line overflows at spacing {m}");
            }
        }
    }

    #[test]
    fn long_words_are_split() {
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        let lines = wrap("abcdefghij kl", 40.0, &w);
        assert_eq!(lines.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["abcd", "efgh", "ij", "kl"]);
        assert_eq!(lines[1].0, 4);
        assert_eq!(lines[3].0, 11);
    }

    #[test]
    fn paragraphs_fill_pages_and_keep_positions() {
        // 20 px font => line height 28.4; 5 lines per 150 px page
        let blocks: Vec<Block> = (0..4).map(|i| Block::Para(format!("para{i} one two three four five six seven eight nine ten"))).collect();
        let l = paginate(&blocks, m(200.0, 150.0), &measure);
        assert!(l.pages.len() > 1);
        for p in &l.pages {
            let last = p.lines.last().unwrap();
            assert!(last.y + 28.4 <= 150.0 + 0.01, "no line overflows the page");
        }
        // every block's text appears exactly once across the pages
        let total: usize = l.pages.iter().flat_map(|p| &p.lines).filter(|x| x.block == 0).map(|x| x.text.split(' ').count()).sum();
        assert_eq!(total, 11);
        for (i, p) in l.pages.iter().enumerate() {
            assert_eq!(l.page_of(p.start().unwrap()), i);
        }
    }

    #[test]
    fn font_size_change_keeps_the_reading_position() {
        let blocks: Vec<Block> = (0..30).map(|i| Block::Para(format!("block {i} lorem ipsum dolor sit amet consectetur adipiscing elit"))).collect();
        let small = paginate(&blocks, Metrics { width: 300.0, height: 300.0, body: 16.0, line_height: LINE_HEIGHT }, &measure);
        let big = paginate(&blocks, Metrics { width: 300.0, height: 300.0, body: 28.0, line_height: LINE_HEIGHT }, &measure);
        assert!(big.pages.len() > small.pages.len());
        let pos = small.pos_of(small.pages.len() / 2);
        let page = big.page_of(pos);
        let start = big.pos_of(page);
        assert!((start.block, start.offset) <= (pos.block, pos.offset));
        let next = big.pos_of(page + 1);
        assert!((next.block, next.offset) > (pos.block, pos.offset) || page + 1 >= big.pages.len());
    }

    #[test]
    fn headings_stay_with_following_text_and_images_fit() {
        let img = Image::new(400, 200);
        let blocks = vec![Block::Para("a b c d e f g h i j k l m n o p q r s t".into()), Block::Heading(1, "Chapter".into()), Block::Para("after the heading".into()), Block::Image(img)];
        let l = paginate(&blocks, m(200.0, 160.0), &measure);
        let heading_page = l.pages.iter().position(|p| p.lines.iter().any(|x| x.block == 1)).unwrap();
        assert!(l.pages[heading_page].lines.iter().any(|x| x.block == 2), "the heading is not stranded at the page bottom");
        let image = l.pages.iter().flat_map(|p| &p.lines).find(|x| x.block == 3).unwrap();
        match image.kind {
            Kind::Image { w, h } => {
                assert!(w <= 200.0 + 0.01 && h <= 160.0);
                assert!((w / h - 2.0).abs() < 0.01);
            }
            _ => panic!("image line"),
        }
    }

    #[test]
    fn empty_book_has_one_empty_page() {
        let l = paginate(&[], m(100.0, 100.0), &measure);
        assert_eq!(l.pages.len(), 1);
        assert_eq!(l.page_of(Pos { block: 5, offset: 5 }), 0);
        assert_eq!(l.pos_of(3), Pos::default());
    }

    /// A face's measure: the same text is wider in a wider face, so the same book breaks into more pages in it.
    #[test]
    fn the_measure_of_the_chosen_face_sets_the_page_count() {
        let blocks: Vec<Block> = (0..30).map(|i| Block::Para(format!("Paragraph {i} runs on for a while so that it wraps across several lines of the column."))).collect();
        let narrow_face = |s: &str, px: f32, bold: bool| measure(s, px, bold);
        let wide_face = |s: &str, px: f32, bold: bool| measure(s, px, bold) * 1.35;
        let narrow = paginate(&blocks, m(400.0, 300.0), &narrow_face);
        let wide = paginate(&blocks, m(400.0, 300.0), &wide_face);
        assert!(wide.pages.len() > narrow.pages.len(), "a wider face needs more pages: {} vs {}", wide.pages.len(), narrow.pages.len());
        // the same face gives the same pages again
        assert_eq!(paginate(&blocks, m(400.0, 300.0), &wide_face).pages.len(), wide.pages.len());
    }
}
