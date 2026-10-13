//! Vertical text layout: the glyphs of a vertical run placed in one column, read top to bottom, and the columns of a
//! block, read right to left. Three forms of a glyph take part (see [`Forms`]): a glyph with a native upright form,
//! a horizontal glyph turned a quarter turn so it lies on its side, and a short horizontal group (tate-chu-yoko, such
//! as two digits) that sits upright inside one em box.
//!
//! Pure geometry. The caller supplies the horizontal advance of each glyph from its font, and gets back boxes in
//! column coordinates: `x` is the left edge inside the column (0 to `em`), `y` is measured down from the top of the
//! column, and `rotated` means the glyph is drawn turned a quarter turn clockwise. No font, no drawing.
//!
//! Rules used by [`layout_column`]:
//! * an upright glyph takes one em of height;
//! * a sideways glyph takes its horizontal advance as height;
//! * a run of consecutive horizontal-in-vertical glyphs takes one em of height when their advances add up to at most
//!   one em. They are centred side by side in that box. A wider run falls back to sideways glyphs.

/// How a glyph takes part in a vertical line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forms {
    /// The glyph's own upright form, one em tall.
    Vertical,
    /// A horizontal glyph turned a quarter turn, as Latin letters are set in a vertical line.
    Horizontal,
    /// A horizontal glyph kept horizontal inside an upright em box, as short numerals are set in a vertical line.
    HorizontalInVertical,
}

/// One glyph of a vertical run: its horizontal advance from the font and how it is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub advance: f32,
    pub forms: Forms,
}

/// The box of one glyph in column coordinates. `index` is the glyph's position in the input slice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotated: bool,
}

/// Lays `glyphs` out top to bottom in one column whose em box is `em` wide and tall, starting at `top`.
/// Returns the boxes in input order and the length the column takes. Negative or NaN sizes count as zero.
pub fn layout_column(glyphs: &[Glyph], em: f32, top: f32) -> (Vec<Placed>, f32) {
    let em = em.max(0.0);
    let mut out = Vec::with_capacity(glyphs.len());
    let mut y = top;
    let mut i = 0;
    while i < glyphs.len() {
        match glyphs[i].forms {
            Forms::Vertical => {
                out.push(Placed { index: i, x: 0.0, y, width: em, height: em, rotated: false });
                y += em;
                i += 1;
            }
            Forms::Horizontal => {
                let h = glyphs[i].advance.max(0.0);
                out.push(Placed { index: i, x: 0.0, y, width: em, height: h, rotated: true });
                y += h;
                i += 1;
            }
            Forms::HorizontalInVertical => {
                // the group is every consecutive glyph with this form
                let mut end = i;
                let mut total = 0.0_f32;
                while end < glyphs.len() && glyphs[end].forms == Forms::HorizontalInVertical {
                    total += glyphs[end].advance.max(0.0);
                    end += 1;
                }
                if total <= em {
                    let mut x = (em - total) / 2.0;
                    for k in i..end {
                        let w = glyphs[k].advance.max(0.0);
                        out.push(Placed { index: k, x, y, width: w, height: em, rotated: false });
                        x += w;
                    }
                    y += em;
                } else {
                    for k in i..end {
                        let h = glyphs[k].advance.max(0.0);
                        out.push(Placed { index: k, x: 0.0, y, width: em, height: h, rotated: true });
                        y += h;
                    }
                }
                i = end;
            }
        }
    }
    (out, y - top)
}

/// Left edge of each of `columns` columns of width `em`, separated by `gap`, in a block whose right edge is `right`.
/// Column 0 is the rightmost, because vertical text reads from right to left.
pub fn column_lefts(columns: usize, em: f32, gap: f32, right: f32) -> Vec<f32> {
    let em = em.max(0.0);
    let gap = gap.max(0.0);
    (0..columns).map(|c| right - (c as f32 + 1.0) * em - c as f32 * gap).collect()
}

/// How a character is oriented in a vertical line, by its script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Drawn upright (ideographs, kana, Hangul, full-width forms).
    Upright,
    /// Drawn turned a quarter turn clockwise (Latin, digits, most punctuation).
    Rotated,
}

/// The vertical orientation of a character: upright for East Asian scripts and symbols, rotated for everything else.
pub fn orientation(c: char) -> Orientation {
    let v = c as u32;
    let upright = matches!(v,
        0x1100..=0x11ff | 0x2e80..=0x2fdf | 0x2ff0..=0x303e | 0x3041..=0x33ff | 0x3400..=0x4dbf | 0x4e00..=0xa4cf
        | 0xa960..=0xa97f | 0xac00..=0xd7ff | 0xf900..=0xfaff | 0xfe10..=0xfe1f | 0xfe30..=0xfe6f | 0xff00..=0xff60
        | 0xffe0..=0xffe6 | 0x1b000..=0x1b16f | 0x1f200..=0x1f2ff | 0x20000..=0x3ffff
        | 0x2460..=0x24ff | 0x25a0..=0x25ff | 0x2600..=0x26ff | 0x2190..=0x21ff | 0xa5 | 0xa7 | 0xa8 | 0xa9
        | 0xae | 0xb1 | 0xbc..=0xbe | 0xd7 | 0xf7 | 0x1f300..=0x1faff);
    if upright {
        Orientation::Upright
    } else {
        Orientation::Rotated
    }
}

/// The form of a punctuation or bracket character used in a vertical line, when it differs from the horizontal one.
pub fn vertical_form(c: char) -> Option<char> {
    Some(match c {
        '\u{3001}' => '\u{fe11}',
        '\u{3002}' => '\u{fe12}',
        '\u{ff0c}' => '\u{fe10}',
        '\u{ff1a}' => '\u{fe13}',
        '\u{ff1b}' => '\u{fe14}',
        '\u{ff01}' => '\u{fe15}',
        '\u{ff1f}' => '\u{fe16}',
        '\u{2026}' => '\u{fe19}',
        '\u{2025}' => '\u{fe30}',
        '\u{2014}' => '\u{fe31}',
        '\u{2013}' => '\u{fe32}',
        '\u{ff3f}' => '\u{fe33}',
        '\u{ff08}' => '\u{fe35}',
        '\u{ff09}' => '\u{fe36}',
        '\u{ff5b}' => '\u{fe37}',
        '\u{ff5d}' => '\u{fe38}',
        '\u{3014}' => '\u{fe39}',
        '\u{3015}' => '\u{fe3a}',
        '\u{3010}' => '\u{fe3b}',
        '\u{3011}' => '\u{fe3c}',
        '\u{300a}' => '\u{fe3d}',
        '\u{300b}' => '\u{fe3e}',
        '\u{3008}' => '\u{fe3f}',
        '\u{3009}' => '\u{fe40}',
        '\u{300c}' => '\u{fe41}',
        '\u{300d}' => '\u{fe42}',
        '\u{300e}' => '\u{fe43}',
        '\u{300f}' => '\u{fe44}',
        '\u{ff3b}' => '\u{fe47}',
        '\u{ff3d}' => '\u{fe48}',
        _ => return None,
    })
}

/// Chooses the form of each character of `text` in a vertical line. Characters with an upright orientation are
/// [`Forms::Vertical`]; others are [`Forms::Horizontal`] (turned). A run of 2 to `max_group` ASCII digits that sits
/// between upright characters, or alone, is [`Forms::HorizontalInVertical`] (tate-chu-yoko). `explicit` lists
/// character ranges the caller marks horizontal-in-vertical regardless of the rule.
pub fn classify(text: &str, max_group: usize, explicit: &[std::ops::Range<usize>]) -> Vec<Forms> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Forms> = chars
        .iter()
        .map(|&c| if orientation(c) == Orientation::Upright { Forms::Vertical } else { Forms::Horizontal })
        .collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let s = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - s;
            let before_ok = s == 0 || out[s - 1] == Forms::Vertical;
            let after_ok = i == chars.len() || out[i] == Forms::Vertical;
            if len >= 2 && len <= max_group && before_ok && after_ok {
                out[s..i].fill(Forms::HorizontalInVertical);
            }
        } else {
            i += 1;
        }
    }
    for r in explicit {
        for k in r.start.min(out.len())..r.end.min(out.len()) {
            out[k] = Forms::HorizontalInVertical;
        }
    }
    out
}

/// Lays out `text` in one column: classifies it, takes the advances from `advance_of` (horizontal advance in px of a
/// character at size `em`), and places it. A convenience over [`classify`] and [`layout_column`].
pub fn layout_text(text: &str, em: f32, top: f32, advance_of: &dyn Fn(char) -> f32) -> (Vec<Placed>, f32) {
    let forms = classify(text, 4, &[]);
    let glyphs: Vec<Glyph> = text.chars().zip(forms).map(|(c, f)| Glyph { advance: advance_of(c), forms: f }).collect();
    layout_column(&glyphs, em, top)
}

#[cfg(test)]
mod form_tests {
    use super::*;

    #[test]
    fn scripts_get_their_orientation() {
        assert_eq!(orientation('日'), Orientation::Upright);
        assert_eq!(orientation('あ'), Orientation::Upright);
        assert_eq!(orientation('a'), Orientation::Rotated);
    }

    #[test]
    fn punctuation_has_vertical_forms() {
        assert_eq!(vertical_form('\u{3001}'), Some('\u{fe11}'));
        assert_eq!(vertical_form('\u{300c}'), Some('\u{fe41}'));
        assert_eq!(vertical_form('a'), None);
    }

    #[test]
    fn short_digit_runs_between_ideographs_go_horizontal_in_vertical() {
        let f = classify("日12日", 4, &[]);
        assert_eq!(f, vec![Forms::Vertical, Forms::HorizontalInVertical, Forms::HorizontalInVertical, Forms::Vertical]);
        // one digit and long runs are turned
        assert_eq!(classify("日1日", 4, &[])[1], Forms::Horizontal);
        assert_eq!(classify("日12345日", 4, &[])[1], Forms::Horizontal);
        // next to Latin letters they are turned
        assert_eq!(classify("a12", 4, &[])[1], Forms::Horizontal);
    }

    #[test]
    fn explicit_ranges_win() {
        assert_eq!(classify("日ab日", 4, &[1..3])[2], Forms::HorizontalInVertical);
    }

    #[test]
    fn layout_text_places_mixed_text() {
        let (b, len) = layout_text("日12", 10.0, 0.0, &|_| 5.0);
        assert_eq!(b.len(), 3);
        assert_eq!(len, 20.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(advance: f32, forms: Forms) -> Glyph {
        Glyph { advance, forms }
    }

    #[test]
    fn upright_glyphs_take_one_em_each() {
        let glyphs = [g(6.0, Forms::Vertical), g(6.0, Forms::Vertical), g(6.0, Forms::Vertical)];
        let (boxes, len) = layout_column(&glyphs, 10.0, 0.0);
        assert_eq!(len, 30.0);
        let ys: Vec<f32> = boxes.iter().map(|b| b.y).collect();
        assert_eq!(ys, vec![0.0, 10.0, 20.0]);
        assert!(boxes.iter().all(|b| !b.rotated && b.width == 10.0 && b.height == 10.0 && b.x == 0.0));
    }

    #[test]
    fn a_sideways_glyph_takes_its_advance_as_height_and_is_rotated() {
        let glyphs = [g(6.0, Forms::Horizontal), g(4.0, Forms::Vertical)];
        let (boxes, len) = layout_column(&glyphs, 10.0, 0.0);
        assert_eq!(boxes[0], Placed { index: 0, x: 0.0, y: 0.0, width: 10.0, height: 6.0, rotated: true });
        assert_eq!(boxes[1], Placed { index: 1, x: 0.0, y: 6.0, width: 10.0, height: 10.0, rotated: false });
        assert_eq!(len, 16.0);
    }

    #[test]
    fn a_short_horizontal_group_sits_centred_in_one_em_box() {
        // three glyphs of advance 3 make 9, which fits in one em of 10, so they start at (10 - 9) / 2 = 0.5
        let glyphs = [
            g(3.0, Forms::HorizontalInVertical),
            g(3.0, Forms::HorizontalInVertical),
            g(3.0, Forms::HorizontalInVertical),
            g(5.0, Forms::Vertical),
        ];
        let (boxes, len) = layout_column(&glyphs, 10.0, 0.0);
        assert_eq!(boxes.len(), 4);
        assert_eq!((boxes[0].x, boxes[1].x, boxes[2].x), (0.5, 3.5, 6.5));
        assert!(boxes[..3].iter().all(|b| b.y == 0.0 && b.height == 10.0 && !b.rotated));
        // the group used one em of height, so the upright glyph after it starts at 10
        assert_eq!(boxes[3].y, 10.0);
        // one em for the group and one em for the upright glyph
        assert_eq!(len, 20.0);
    }

    #[test]
    fn a_wide_horizontal_group_falls_back_to_sideways_glyphs() {
        // two advances of 6 make 12, more than one em of 10
        let glyphs = [g(6.0, Forms::HorizontalInVertical), g(6.0, Forms::HorizontalInVertical)];
        let (boxes, len) = layout_column(&glyphs, 10.0, 0.0);
        assert_eq!(boxes[0], Placed { index: 0, x: 0.0, y: 0.0, width: 10.0, height: 6.0, rotated: true });
        assert_eq!(boxes[1], Placed { index: 1, x: 0.0, y: 6.0, width: 10.0, height: 6.0, rotated: true });
        assert_eq!(len, 12.0);
    }

    #[test]
    fn a_group_between_upright_glyphs_keeps_the_indices_and_the_order() {
        let glyphs = [
            g(7.0, Forms::Vertical),
            g(2.0, Forms::HorizontalInVertical),
            g(2.0, Forms::HorizontalInVertical),
            g(7.0, Forms::Vertical),
        ];
        let (boxes, len) = layout_column(&glyphs, 10.0, 5.0);
        let indices: Vec<usize> = boxes.iter().map(|b| b.index).collect();
        assert_eq!(indices, vec![0, 1, 2, 3]);
        // group of 4 in one em: starts at (10 - 4) / 2 = 3 within the column
        assert_eq!((boxes[1].x, boxes[2].x), (3.0, 5.0));
        // upright at top 5, group at 15, upright at 25; the column is 30 long
        assert_eq!((boxes[0].y, boxes[1].y, boxes[3].y), (5.0, 15.0, 25.0));
        assert_eq!(len, 30.0);
    }

    #[test]
    fn an_empty_run_takes_no_space_and_bad_sizes_count_as_zero() {
        assert_eq!(layout_column(&[], 10.0, 4.0), (Vec::new(), 0.0));
        let glyphs = [g(-3.0, Forms::Horizontal)];
        let (boxes, len) = layout_column(&glyphs, -1.0, 0.0);
        assert_eq!(boxes[0].height, 0.0);
        assert_eq!(boxes[0].width, 0.0);
        assert_eq!(len, 0.0);
    }

    #[test]
    fn columns_run_from_the_right_edge_leftwards() {
        // columns of width 10 with a gap of 2 in a block whose right edge is 100
        assert_eq!(column_lefts(3, 10.0, 2.0, 100.0), vec![90.0, 78.0, 66.0]);
        assert_eq!(column_lefts(0, 10.0, 2.0, 100.0), Vec::<f32>::new());
    }
}
