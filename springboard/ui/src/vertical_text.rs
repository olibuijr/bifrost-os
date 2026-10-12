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
