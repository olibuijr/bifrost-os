//! STUB (cg-context branch): the cg-raster agent owns the real module; drop this file on merge. Only the
//! contract the context uses is declared. `Coverage` is an 8-bit mask whose top-left is device pixel (x, y);
//! row-major `data` of `w * h` bytes. `fill` returns coverage with `clip` already multiplied in.
use super::path::{Affine, Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash: Vec<f32>,
    pub dash_phase: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

pub fn fill(_path: &Path, _rule: FillRule, _xf: &Affine, _clip: Option<&Coverage>, _w: usize, _h: usize) -> Coverage {
    todo!()
}

pub fn stroke_path(_path: &Path, _style: &StrokeStyle) -> Path {
    todo!()
}
