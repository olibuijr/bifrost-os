//! Math typesetting: a TeX-subset parser, a box model and TeX-style layout rules.
//!
//! Pipeline: `parse::parse_tex` or `parse::parse_linear` turn source text into a
//! [`Node`] tree; [`layout::layout`] measures that tree against a [`metrics::FontMetrics`]
//! provider (the OpenType MATH constants when the font has them, sensible defaults
//! otherwise) and returns a [`Layout`]: positioned glyphs and rules in pixels, ready for
//! the existing glyph drawing path. Nothing here touches the screen.
//!
//! Coordinates in [`Layout`] are relative to the origin on the baseline at the left edge
//! of the formula, with `y` growing downward (so a glyph raised above the baseline has a
//! negative `y`).

pub mod layout;
pub mod metrics;
pub mod parse;
pub mod paint;
pub mod system;

/// TeX atom classes, used for the spacing table between neighbouring atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// A parsed formula.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// One symbol (letter, digit, Greek letter, operator, delimiter character).
    Sym { ch: char, class: Class },
    /// Upright text, as produced by `\text{...}`, `\mathrm{...}` and operator names.
    Text(String),
    /// A sequence of atoms.
    Row(Vec<Node>),
    /// `\frac{num}{den}`, or `a/b` in linear input.
    Frac { num: Box<Node>, den: Box<Node> },
    /// `\sqrt[index]{body}`, or `sqrt(x)` in linear input.
    Radical { index: Option<Box<Node>>, body: Box<Node> },
    /// Superscript and subscript on an ordinary base.
    Scripts { base: Box<Node>, sub: Option<Box<Node>>, sup: Option<Box<Node>> },
    /// A large operator (`\sum`, `\prod`, `\int`) with optional limits.
    BigOp { ch: char, limits: bool, sub: Option<Box<Node>>, sup: Option<Box<Node>> },
    /// `\left(...\right)` and friends: a body between stretched delimiters.
    /// A `None` delimiter is the invisible `.`.
    Fenced { open: Option<char>, close: Option<char>, body: Box<Node> },
    /// A matrix environment; `rows[r][c]` are the cells.
    Matrix { open: Option<char>, close: Option<char>, rows: Vec<Vec<Node>> },
    /// Horizontal space in em (`\,`, `\;`, `\quad`, `\!`, ...).
    Space(f32),
}

impl Node {
    /// Wraps a node in a `Row` unless it already is one.
    pub fn into_row(self) -> Vec<Node> {
        match self {
            Node::Row(items) => items,
            other => vec![other],
        }
    }
}

/// Glyph and rule output of a layout, in pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Advance width of the whole formula.
    pub width: f32,
    /// Height above the baseline.
    pub ascent: f32,
    /// Depth below the baseline.
    pub depth: f32,
    pub glyphs: Vec<Glyph>,
    pub rules: Vec<Rule>,
}

/// One glyph placed on the baseline at `(x, y)`; `size` is its font size in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// A filled rectangle (fraction bars, radical overlines), top-left at `(x, y)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Layout options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Text font size in pixels (the size of the surrounding text).
    pub font_px: f32,
    /// Display style (`$$...$$`, a block of its own) rather than inline text style.
    pub display: bool,
}

impl Options {
    pub fn text(font_px: f32) -> Self {
        Options { font_px, display: false }
    }

    pub fn display(font_px: f32) -> Self {
        Options { font_px, display: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn into_row_keeps_rows_and_wraps_others() {
        let s = Node::Sym { ch: 'x', class: Class::Ord };
        assert_eq!(s.clone().into_row(), vec![s.clone()]);
        let row = Node::Row(vec![s.clone(), s.clone()]);
        assert_eq!(row.into_row().len(), 2);
    }

    #[test]
    fn options_constructors_set_display_flag() {
        assert!(!Options::text(16.0).display);
        assert!(Options::display(16.0).display);
    }
}
