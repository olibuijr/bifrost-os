//! Texture descriptors for GPU resources: the kind, size, pixel format, mipmap count and sample count of a texture,
//! as a pure model. It checks that a descriptor can exist and counts the bytes its mip chain takes. It creates no GPU
//! object and uploads nothing.
//!
//! Rules: width and height are at least 1 and at most the device limit `max_dimension`. A 3D texture has a depth of at
//! least 1; other kinds have depth 1. A cube has equal sides and six faces. The mip count is at least 1 and at most the
//! number of halvings from the largest side down to 1 (`full_mip_count`). Each level halves every side, never below 1.
//! A multisampled texture is 2D, has one mip level and 2, 4 or 8 samples per pixel.

/// The dimension and arrangement of a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Texture2D,
    Texture3D,
    Cube,
}

/// The size and layout of one pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    R8Unorm,
    Rgba8Unorm,
    Rgba16Float,
    Rgba32Float,
    Depth32Float,
}

impl PixelFormat {
    /// Bytes in one pixel of one sample.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::R8Unorm => 1,
            PixelFormat::Rgba8Unorm => 4,
            PixelFormat::Rgba16Float => 8,
            PixelFormat::Rgba32Float => 16,
            PixelFormat::Depth32Float => 4,
        }
    }
}

/// Why a descriptor cannot describe a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureError {
    /// Width, height or depth is zero, or the array length is zero.
    ZeroSize,
    /// Width or height is above the device limit.
    TooLarge { max: usize },
    /// A cube has sides that differ.
    CubeNotSquare,
    /// A kind other than 2D has an array length above 1, or a 3D texture has an array length.
    ArrayNotAllowed,
    /// The mip count is zero or above the full chain for the size.
    MipCountOutOfRange { max: usize },
    /// The sample count is not 1, 2, 4 or 8.
    BadSampleCount { samples: usize },
    /// A multisampled texture has more than one mip level or is not 2D.
    MultisampleNeedsSingleLevel,
}

/// A full description of one texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureDescriptor {
    pub kind: Kind,
    pub format: PixelFormat,
    pub width: usize,
    pub height: usize,
    pub depth: usize,
    pub mip_levels: usize,
    pub samples: usize,
    pub array_length: usize,
}

/// The number of mip levels in a full chain for a size: the halvings from the largest side down to 1, plus the base.
pub fn full_mip_count(width: usize, height: usize, depth: usize) -> usize {
    let mut side = width.max(height).max(depth).max(1);
    let mut levels = 1;
    while side > 1 {
        side = (side + 1) / 2;
        levels += 1;
    }
    levels
}

/// One axis at one level: halved `level` times, never below 1.
fn shrink(value: usize, level: usize) -> usize {
    if level >= usize::BITS as usize {
        1
    } else {
        (value >> level).max(1)
    }
}

impl TextureDescriptor {
    /// A 2D texture. With `mipmapped` the chain is full, otherwise it has one level.
    pub fn texture_2d(format: PixelFormat, width: usize, height: usize, mipmapped: bool) -> Self {
        let mip_levels = if mipmapped { full_mip_count(width, height, 1) } else { 1 };
        TextureDescriptor {
            kind: Kind::Texture2D,
            format,
            width,
            height,
            depth: 1,
            mip_levels,
            samples: 1,
            array_length: 1,
        }
    }

    /// A 3D texture of `width` by `height` by `depth`, with a full chain when `mipmapped`.
    pub fn texture_3d(format: PixelFormat, width: usize, height: usize, depth: usize, mipmapped: bool) -> Self {
        let mip_levels = if mipmapped { full_mip_count(width, height, depth) } else { 1 };
        TextureDescriptor {
            kind: Kind::Texture3D,
            format,
            width,
            height,
            depth,
            mip_levels,
            samples: 1,
            array_length: 1,
        }
    }

    /// A cube of `size` by `size` pixels per face.
    pub fn cube(format: PixelFormat, size: usize, mipmapped: bool) -> Self {
        let mip_levels = if mipmapped { full_mip_count(size, size, 1) } else { 1 };
        TextureDescriptor {
            kind: Kind::Cube,
            format,
            width: size,
            height: size,
            depth: 1,
            mip_levels,
            samples: 1,
            array_length: 1,
        }
    }

    /// Checks the descriptor against the device's largest side. Returns the first rule it breaks.
    pub fn validate(&self, max_dimension: usize) -> Result<(), TextureError> {
        if self.width == 0 || self.height == 0 || self.depth == 0 || self.array_length == 0 {
            return Err(TextureError::ZeroSize);
        }
        if self.width > max_dimension || self.height > max_dimension {
            return Err(TextureError::TooLarge { max: max_dimension });
        }
        match self.kind {
            Kind::Cube => {
                if self.width != self.height {
                    return Err(TextureError::CubeNotSquare);
                }
                if self.array_length != 1 {
                    return Err(TextureError::ArrayNotAllowed);
                }
            }
            Kind::Texture3D => {
                if self.array_length != 1 {
                    return Err(TextureError::ArrayNotAllowed);
                }
            }
            Kind::Texture2D => {}
        }
        let max_levels = full_mip_count(self.width, self.height, self.depth);
        if self.mip_levels == 0 || self.mip_levels > max_levels {
            return Err(TextureError::MipCountOutOfRange { max: max_levels });
        }
        match self.samples {
            1 => Ok(()),
            2 | 4 | 8 => {
                if self.kind != Kind::Texture2D || self.mip_levels != 1 {
                    Err(TextureError::MultisampleNeedsSingleLevel)
                } else {
                    Ok(())
                }
            }
            samples => Err(TextureError::BadSampleCount { samples }),
        }
    }

    /// Size in pixels of one mip level as (width, height, depth).
    pub fn level_size(&self, level: usize) -> (usize, usize, usize) {
        (shrink(self.width, level), shrink(self.height, level), shrink(self.depth, level))
    }

    /// Total bytes for all mip levels, faces, array elements and samples. Saturates instead of overflowing.
    pub fn bytes(&self) -> usize {
        let faces = if self.kind == Kind::Cube { 6 } else { 1 };
        let mut total: usize = 0;
        for level in 0..self.mip_levels {
            let (w, h, d) = self.level_size(level);
            let pixels = w.saturating_mul(h).saturating_mul(d);
            total = total.saturating_add(pixels);
        }
        total
            .saturating_mul(self.format.bytes_per_pixel())
            .saturating_mul(self.samples)
            .saturating_mul(faces)
            .saturating_mul(self.array_length)
    }

    /// Bytes of one row of the base level, rounded up to a multiple of `align`. None for an alignment of zero or an
    /// overflow.
    pub fn bytes_per_row(&self, align: usize) -> Option<usize> {
        if align == 0 {
            return None;
        }
        let row = self.width.checked_mul(self.format.bytes_per_pixel())?;
        let padded = row.checked_add(align - 1)?;
        Some((padded / align) * align)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_chain_counts_halvings_down_to_one() {
        assert_eq!(full_mip_count(256, 64, 1), 9);
        assert_eq!(full_mip_count(5, 1, 1), 4);
        assert_eq!(full_mip_count(1, 1, 1), 1);
    }

    #[test]
    fn flat_rgba8_bytes_are_width_height_and_four() {
        let t = TextureDescriptor::texture_2d(PixelFormat::Rgba8Unorm, 64, 64, false);
        assert_eq!(t.bytes(), 64 * 64 * 4);
        assert_eq!(t.validate(16384), Ok(()));
    }

    #[test]
    fn mip_chain_adds_each_level() {
        // 4x4 has levels 4x4, 2x2 and 1x1: 16 + 4 + 1 pixels at 4 bytes each.
        let t = TextureDescriptor::texture_2d(PixelFormat::Rgba8Unorm, 4, 4, true);
        assert_eq!(t.mip_levels, 3);
        assert_eq!(t.bytes(), 21 * 4);
        assert_eq!(t.level_size(2), (1, 1, 1));
    }

    #[test]
    fn cube_counts_six_faces() {
        let t = TextureDescriptor::cube(PixelFormat::Rgba16Float, 16, false);
        assert_eq!(t.bytes(), 16 * 16 * 8 * 6);
        assert_eq!(t.validate(16384), Ok(()));
    }

    #[test]
    fn multisample_multiplies_bytes_and_needs_one_level() {
        let mut t = TextureDescriptor::texture_2d(PixelFormat::R8Unorm, 8, 8, false);
        t.samples = 4;
        assert_eq!(t.bytes(), 64 * 4);
        assert_eq!(t.validate(16384), Ok(()));
        t.mip_levels = 2;
        assert_eq!(t.validate(16384), Err(TextureError::MultisampleNeedsSingleLevel));
        t.mip_levels = 1;
        t.samples = 3;
        assert_eq!(t.validate(16384), Err(TextureError::BadSampleCount { samples: 3 }));
    }

    #[test]
    fn oversize_and_mip_overflow_are_refused() {
        let big = TextureDescriptor::texture_2d(PixelFormat::Rgba8Unorm, 20000, 4, false);
        assert_eq!(big.validate(16384), Err(TextureError::TooLarge { max: 16384 }));
        let mut t = TextureDescriptor::texture_2d(PixelFormat::Rgba8Unorm, 4, 4, false);
        t.mip_levels = 4;
        assert_eq!(t.validate(16384), Err(TextureError::MipCountOutOfRange { max: 3 }));
    }

    #[test]
    fn rows_are_padded_to_the_alignment() {
        let t = TextureDescriptor::texture_2d(PixelFormat::Rgba8Unorm, 100, 1, false);
        assert_eq!(t.bytes_per_row(1), Some(400));
        assert_eq!(t.bytes_per_row(256), Some(512));
        assert_eq!(t.bytes_per_row(0), None);
    }
}
