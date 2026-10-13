//! Paragraph attributes (`NativeParagraphStyle`): alignment, indents, tab stops and line spacing, as geometry for a
//! container width, a line width and a natural line height. Measuring text stays with the caller's font; line
//! breaking and truncation are in `typeset`.

/// Horizontal placement of a line (`NativeTextAlignment`). `Natural` is the leading edge; text runs left to right, so
/// it places like `Left`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Natural,
    Left,
    Right,
    Center,
    Justified,
}

/// Where a run sits on a tab stop (`NativeTextTab`). `Justified` and `Natural` are not tab alignments and act as `Left`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabAlignment {
    #[default]
    Left,
    Right,
    Center,
}

/// A tab stop: an alignment and a distance from the leading margin, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextTab {
    pub alignment: TabAlignment,
    pub location: f32,
}

/// How a paragraph is cut into lines and what happens to text that does not fit (`NativeLineBreakMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineBreakMode {
    /// Break at line break opportunities (word wrap).
    #[default]
    WordWrap,
    /// Break between any two characters.
    CharWrap,
    /// One line per paragraph, cut off at the edge.
    Clip,
    /// One line per paragraph, the start replaced by an ellipsis.
    TruncateHead,
    /// One line per paragraph, the end replaced by an ellipsis.
    TruncateTail,
    /// One line per paragraph, the middle replaced by an ellipsis.
    TruncateMiddle,
}

/// The base writing direction of a paragraph. `Natural` follows the first strong character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BaseDirection {
    #[default]
    Natural,
    LeftToRight,
    RightToLeft,
}

/// The distance between default tab stops when a style sets none, in points.
pub const DEFAULT_TAB_INTERVAL: f32 = 28.0;

/// Paragraph attributes (`NativeParagraphStyle`). The default value is the plain style: natural alignment, no indents,
/// no tab stops and no spacing. Zero in a line-height or spacing field means unset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphStyle {
    pub alignment: Alignment,
    /// Indent of the first line of a paragraph, from the leading margin.
    pub first_line_head_indent: f32,
    /// Indent of the other lines, from the leading margin.
    pub head_indent: f32,
    /// A positive value is the distance from the leading margin. Zero is the full width. A negative value is measured
    /// back from the trailing margin.
    pub tail_indent: f32,
    pub tab_stops: Vec<TextTab>,
    /// Limits on the line height in points.
    pub minimum_line_height: f32,
    pub maximum_line_height: f32,
    /// Multiplies the natural line height (ascent + descent + leading).
    pub line_height_multiple: f32,
    /// Space before the paragraph and after it, in points.
    pub paragraph_spacing_before: f32,
    pub paragraph_spacing: f32,
    /// Extra space added below every line, in points.
    pub line_spacing: f32,
    /// Distance between the default tab stops past the last explicit stop. Zero means `DEFAULT_TAB_INTERVAL`.
    pub default_tab_interval: f32,
    pub line_break_mode: LineBreakMode,
    pub base_direction: BaseDirection,
}

impl ParagraphStyle {
    /// The left indent of a line: the first-line indent for the first line of a paragraph, the head indent otherwise.
    pub fn indent(&self, first_line: bool) -> f32 {
        if first_line {
            self.first_line_head_indent
        } else {
            self.head_indent
        }
    }

    /// The x of the trailing margin in a container `width` wide.
    pub fn trailing_edge(&self, width: f32) -> f32 {
        if self.tail_indent > 0.0 {
            self.tail_indent
        } else {
            width + self.tail_indent
        }
    }

    /// Height of a line with natural height `natural`: the multiple first, then the minimum and the maximum.
    pub fn line_height(&self, natural: f32) -> f32 {
        let mut h = if self.line_height_multiple > 0.0 { natural * self.line_height_multiple } else { natural };
        if self.minimum_line_height > 0.0 {
            h = h.max(self.minimum_line_height);
        }
        if self.maximum_line_height > 0.0 {
            h = h.min(self.maximum_line_height);
        }
        h
    }

    /// The x where a line `line_w` wide starts inside a container `width` wide, placed by the alignment. `Justified`
    /// starts at the left edge; `extra_per_space` stretches it.
    pub fn line_x(&self, first_line: bool, width: f32, line_w: f32) -> f32 {
        let left = self.indent(first_line);
        let free = (self.trailing_edge(width) - left - line_w).max(0.0);
        left + match self.alignment {
            Alignment::Right => free,
            Alignment::Center => free / 2.0,
            Alignment::Natural | Alignment::Left | Alignment::Justified => 0.0,
        }
    }

    /// The extra width given to each of the `spaces` gaps so that a justified line fills the container. Other
    /// alignments, the last line of a paragraph and lines with no gaps get zero.
    pub fn extra_per_space(&self, first_line: bool, width: f32, line_w: f32, spaces: usize, last_line: bool) -> f32 {
        if self.alignment != Alignment::Justified || last_line || spaces == 0 {
            return 0.0;
        }
        let free = self.trailing_edge(width) - self.indent(first_line) - line_w;
        free.max(0.0) / spaces as f32
    }

    /// The nearest tab stop past `x`. None when no stop lies beyond `x`; the caller then uses its default interval.
    /// The stops may be given in any order.
    pub fn next_tab(&self, x: f32) -> Option<TextTab> {
        self.tab_stops.iter().copied().filter(|t| t.location > x).min_by(|a, b| a.location.total_cmp(&b.location))
    }

    /// Where a run `run_w` wide, reaching the next tab stop after `x`, starts: the run is aligned on the stop.
    pub fn tab_start(&self, x: f32, run_w: f32) -> Option<f32> {
        let tab = self.next_tab(x)?;
        Some(match tab.alignment {
            TabAlignment::Left => tab.location,
            TabAlignment::Right => tab.location - run_w,
            TabAlignment::Center => tab.location - run_w / 2.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(alignment: Alignment) -> ParagraphStyle {
        ParagraphStyle { alignment, ..Default::default() }
    }

    #[test]
    fn plain_style_places_lines_at_the_leading_edge() {
        let s = ParagraphStyle::default();
        assert_eq!(s.line_x(true, 200.0, 50.0), 0.0);
        assert_eq!(s.line_x(false, 200.0, 50.0), 0.0);
        assert_eq!(style(Alignment::Left).line_x(true, 200.0, 50.0), 0.0);
    }

    #[test]
    fn right_and_center_alignment_use_the_free_width() {
        assert_eq!(style(Alignment::Right).line_x(true, 200.0, 50.0), 150.0);
        assert_eq!(style(Alignment::Center).line_x(true, 200.0, 50.0), 75.0);
        // a line wider than the container does not go negative
        assert_eq!(style(Alignment::Right).line_x(true, 200.0, 260.0), 0.0);
    }

    #[test]
    fn indents_and_tail_indent_move_the_margins() {
        let s = ParagraphStyle { first_line_head_indent: 30.0, head_indent: 10.0, ..Default::default() };
        assert_eq!(s.line_x(true, 200.0, 50.0), 30.0);
        assert_eq!(s.line_x(false, 200.0, 50.0), 10.0);
        let positive = ParagraphStyle { tail_indent: 150.0, ..style(Alignment::Right) };
        assert_eq!(positive.trailing_edge(200.0), 150.0);
        assert_eq!(positive.line_x(true, 200.0, 50.0), 100.0);
        let negative = ParagraphStyle { tail_indent: -20.0, ..style(Alignment::Right) };
        assert_eq!(negative.trailing_edge(200.0), 180.0);
        assert_eq!(negative.line_x(false, 200.0, 50.0), 130.0);
    }

    #[test]
    fn only_justified_lines_stretch_and_not_the_last_line() {
        let j = style(Alignment::Justified);
        assert_eq!(j.extra_per_space(true, 200.0, 150.0, 5, false), 10.0);
        assert_eq!(j.extra_per_space(true, 200.0, 150.0, 5, true), 0.0, "last line stays natural");
        assert_eq!(j.extra_per_space(true, 200.0, 150.0, 0, false), 0.0, "no gaps, no stretch");
        assert_eq!(style(Alignment::Left).extra_per_space(true, 200.0, 150.0, 5, false), 0.0);
    }

    #[test]
    fn tab_stops_are_found_in_any_order_and_align_the_run() {
        let s = ParagraphStyle {
            tab_stops: vec![
                TextTab { alignment: TabAlignment::Right, location: 100.0 },
                TextTab { alignment: TabAlignment::Left, location: 50.0 },
                TextTab { alignment: TabAlignment::Center, location: 150.0 },
            ],
            ..Default::default()
        };
        assert_eq!(s.next_tab(0.0).map(|t| t.location), Some(50.0));
        assert_eq!(s.next_tab(50.0).map(|t| t.location), Some(100.0), "a stop at x is not past x");
        assert_eq!(s.next_tab(150.0), None);
        assert_eq!(s.tab_start(0.0, 20.0), Some(50.0), "left stop: the run starts on it");
        assert_eq!(s.tab_start(60.0, 20.0), Some(80.0), "right stop: the run ends on it");
        assert_eq!(s.tab_start(120.0, 20.0), Some(140.0), "center stop: the run is centred on it");
        assert_eq!(s.tab_start(160.0, 20.0), None);
    }

    #[test]
    fn line_height_applies_multiple_then_limits() {
        let plain = ParagraphStyle::default();
        assert_eq!(plain.line_height(20.0), 20.0);
        let multiple = ParagraphStyle { line_height_multiple: 1.5, ..Default::default() };
        assert_eq!(multiple.line_height(20.0), 30.0);
        let min = ParagraphStyle { minimum_line_height: 40.0, ..Default::default() };
        assert_eq!(min.line_height(20.0), 40.0);
        let max = ParagraphStyle { line_height_multiple: 1.5, maximum_line_height: 25.0, ..Default::default() };
        assert_eq!(max.line_height(20.0), 25.0, "the maximum wins over the multiple");
    }
}
