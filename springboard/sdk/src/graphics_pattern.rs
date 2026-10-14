//! `CGPattern` for `GraphicsContext`: a cell of pixels repeated across the plane. The pattern matrix maps pattern space
//! to user space, and the tiling mode decides how the step is fitted to whole device pixels. Cells are premultiplied
//! `0xAARRGGBB` pixels, and the painter reads them through [`CgPattern::shade`].
//!
//! ```ignore
//! let cell = vec![0xFF00_0000, 0xFF11_1111, 0xFF22_2222, 0xFF33_3333];
//! let pattern = CgPattern::from_pixels(2, 2, cell, (2.0, 2.0), (4.0, 4.0), Affine::IDENTITY, PatternTiling::NoDistortion);
//! let px = pattern.unwrap().shade(1.5, 0.5); // the cell pixel at column 1, row 0
//! ```

use crate::graphics_gradient::Affine;
use bifrost_render::cg::paint::{Pattern, Tiling};

/// `CGPatternTiling`: how the step is fitted to whole device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatternTiling {
    /// `kCGPatternTilingNoDistortion`: the step is used as given, so tiles may land on fractional pixels.
    #[default]
    NoDistortion,
    /// `kCGPatternTilingConstantSpacing`: the step is rounded to whole device pixels by changing the spacing only.
    ConstantSpacing,
    /// `kCGPatternTilingConstantSpacingMinimalDistortion`: the step is rounded and the cell is stretched by the same
    /// factor, so the spacing stays proportional to the cell.
    ConstantSpacingMinimalDistortion,
}

impl PatternTiling {
    fn to_render(self) -> Tiling {
        match self {
            PatternTiling::NoDistortion => Tiling::NoDistortion,
            PatternTiling::ConstantSpacing => Tiling::ConstantSpacing,
            PatternTiling::ConstantSpacingMinimalDistortion => Tiling::ConstantSpacingMinimalDistortion,
        }
    }
}

/// A tiled pattern (`CGPattern`).
#[derive(Clone, Debug, PartialEq)]
pub struct CgPattern {
    inner: Pattern,
    tiling: PatternTiling,
}

impl CgPattern {
    /// A pattern from a cell of `width` by `height` premultiplied pixels, rows from top to bottom. The cell covers
    /// `bounds` in pattern space and repeats every `step`; outside a cell the pattern is transparent. `None` for an
    /// empty cell, a pixel count that does not match the size, non-positive bounds or step, or a singular matrix.
    pub fn from_pixels(
        width: usize,
        height: usize,
        pixels: Vec<u32>,
        bounds: (f32, f32),
        step: (f32, f32),
        matrix: Affine,
        tiling: PatternTiling,
    ) -> Option<CgPattern> {
        let inner = Pattern::from_tile(width, height, pixels, bounds, step, matrix.to_array(), tiling.to_render())?;
        Some(CgPattern { inner, tiling })
    }

    pub fn tiling(&self) -> PatternTiling {
        self.tiling
    }

    /// The distance between cell origins, in pattern space.
    pub fn step(&self) -> (f32, f32) {
        self.inner.step()
    }

    /// The size of the drawn part of the cell, in pattern space.
    pub fn bounds(&self) -> (f32, f32) {
        self.inner.bounds()
    }

    /// The premultiplied pixel at the user-space point `(x, y)`, or 0 where no cell covers it.
    pub fn shade(&self, x: f32, y: f32) -> u32 {
        self.inner.shade(x, y)
    }

    /// This pattern with its step fitted to whole device pixels under `ctm`, the user-to-device transform, as its
    /// tiling mode asks. A pattern with `NoDistortion` is returned unchanged.
    pub fn fitted(&self, ctm: Affine) -> CgPattern {
        CgPattern { inner: self.inner.fitted(&ctm.to_array()), tiling: self.tiling }
    }
}
