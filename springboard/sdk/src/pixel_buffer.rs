//! Pixel buffer layouts for video frames and images: the format, the planes, the row stride and the byte size an
//! application needs before it reads or fills a frame.
//!
//! The media layer decodes hardware video into buffers laid out by this model, and the frame conversion writes the same
//! layouts. No pixel data is held here. A layout is a plain description, checked once, that says where each byte of a
//! plane lives.

/// The pixel formats a buffer may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Four bytes per pixel in blue, green, red, alpha order.
    Bgra8888,
    /// Two planes in 4:2:0 layout: a full-size luma plane, then one interleaved chroma plane at half size in each
    /// direction. Both dimensions must be even.
    Nv12,
}

impl PixelFormat {
    /// The planes of a `width` by `height` buffer as (width, height, bytes per pixel, minimum bytes per row).
    fn planes(self, width: u32, height: u32) -> Vec<(u32, u32, usize, usize)> {
        match self {
            PixelFormat::Bgra8888 => vec![(width, height, 4, width as usize * 4)],
            PixelFormat::Nv12 => vec![
                (width, height, 1, width as usize),
                (width / 2, height / 2, 2, width as usize),
            ],
        }
    }
}

/// Where one plane sits in the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    /// Width in samples of this plane (chroma pairs for the chroma plane of NV12).
    pub width: u32,
    /// Height in rows of this plane.
    pub height: u32,
    /// Bytes of one sample in this plane.
    pub bytes_per_pixel: usize,
    /// Bytes from the start of one row to the start of the next, padding included.
    pub bytes_per_row: usize,
    /// Byte offset of the plane from the start of the buffer.
    pub offset: usize,
}

/// Why a layout was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PixelError {
    /// The width or height is zero.
    EmptySize,
    /// The row alignment is zero or not a power of two.
    BadAlignment(u32),
    /// The format needs even dimensions and the size is odd.
    OddSize,
    /// The buffer would not fit in memory addressing.
    TooLarge,
}

impl std::fmt::Display for PixelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PixelError::EmptySize => write!(f, "a pixel buffer needs a width and a height above zero"),
            PixelError::BadAlignment(a) => write!(f, "a row alignment must be a power of two, not {a}"),
            PixelError::OddSize => write!(f, "this format needs an even width and height"),
            PixelError::TooLarge => write!(f, "the pixel buffer is too large to address"),
        }
    }
}

impl std::error::Error for PixelError {}

/// A checked buffer layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Plane>,
    /// The size of the whole buffer in bytes.
    pub total_bytes: usize,
}

impl Layout {
    /// Lays out a buffer. Each row is padded up to a multiple of `row_alignment` bytes, which must be a power of two.
    pub fn new(format: PixelFormat, width: u32, height: u32, row_alignment: u32) -> Result<Layout, PixelError> {
        if width == 0 || height == 0 {
            return Err(PixelError::EmptySize);
        }
        if row_alignment == 0 || !row_alignment.is_power_of_two() {
            return Err(PixelError::BadAlignment(row_alignment));
        }
        if format == PixelFormat::Nv12 && (width % 2 != 0 || height % 2 != 0) {
            return Err(PixelError::OddSize);
        }
        let alignment = row_alignment as usize;
        let mut planes = Vec::new();
        let mut offset = 0usize;
        for (plane_width, plane_height, bytes_per_pixel, minimum_row) in format.planes(width, height) {
            let bytes_per_row = minimum_row.div_ceil(alignment).checked_mul(alignment).ok_or(PixelError::TooLarge)?;
            let len = bytes_per_row.checked_mul(plane_height as usize).ok_or(PixelError::TooLarge)?;
            planes.push(Plane { width: plane_width, height: plane_height, bytes_per_pixel, bytes_per_row, offset });
            offset = offset.checked_add(len).ok_or(PixelError::TooLarge)?;
        }
        Ok(Layout { format, width, height, planes, total_bytes: offset })
    }

    /// The byte offset of sample (`x`, `y`) in plane `plane`, or None when the sample is outside the plane.
    pub fn offset_of(&self, plane: usize, x: u32, y: u32) -> Option<usize> {
        let p = self.planes.get(plane)?;
        if x >= p.width || y >= p.height {
            return None;
        }
        Some(p.offset + y as usize * p.bytes_per_row + x as usize * p.bytes_per_pixel)
    }
}
