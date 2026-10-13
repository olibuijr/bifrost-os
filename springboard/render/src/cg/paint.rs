//! STUB (cg-context branch): the paint agent owns the real module (gradients, shadings, patterns) and extends
//! `Paint`; the context only needs `Solid` and matches with a catch-all-free `match`, so the lead adds arms there.
use super::color::Color;

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
    }
}
