//! Linux dma-buf descriptors for window surfaces: the layout a client sends with a shared buffer (format, modifier,
//! plane offset and stride) and the bounds check the system makes before it wraps the buffer. The file descriptor
//! itself is passed by the system's window protocol; this module holds only the layout the descriptor must satisfy.
//!
//! Only linear buffers of one plane in a 32-bit RGB format are accepted, so the stride and size can be checked without
//! a tiling agreement.

/// Format codes are DRM fourcc values: four ASCII letters read little-endian.
pub const fn fourcc(code: [u8; 4]) -> u32 {
    u32::from_le_bytes(code)
}

/// `DRM_FORMAT_ARGB8888`.
pub const ARGB8888: u32 = fourcc(*b"AR24");
/// `DRM_FORMAT_XRGB8888`.
pub const XRGB8888: u32 = fourcc(*b"XR24");
/// `DRM_FORMAT_ABGR8888`.
pub const ABGR8888: u32 = fourcc(*b"AB24");
/// `DRM_FORMAT_XBGR8888`.
pub const XBGR8888: u32 = fourcc(*b"XB24");
/// `DRM_FORMAT_MOD_LINEAR`: no tiling.
pub const MODIFIER_LINEAR: u64 = 0;

/// Why a descriptor was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DmaBufError {
    /// A width or height is zero.
    EmptySize,
    /// The format is not one of the four 32-bit RGB formats.
    UnsupportedFormat(u32),
    /// The modifier is not linear.
    UnsupportedModifier(u64),
    /// The number of planes is not one.
    PlaneCount(usize),
    /// The row stride is smaller than one row of pixels.
    StrideTooSmall { stride: u32, minimum: u64 },
    /// The plane does not fit in the file: `needed` bytes are required, `size` are present.
    OutOfBounds { needed: u64, size: u64 },
}

/// One plane of a buffer: where its first byte is and how many bytes separate two rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    pub offset: u32,
    pub stride: u32,
}

/// The parameters a client sends with a dma-buf buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub modifier: u64,
    pub planes: Vec<Plane>,
}

/// Bytes per pixel of a supported format.
pub fn bytes_per_pixel(format: u32) -> Option<u32> {
    match format {
        ARGB8888 | XRGB8888 | ABGR8888 | XBGR8888 => Some(4),
        _ => None,
    }
}

impl Descriptor {
    /// Checks the descriptor against the size of the file that carries the buffer. Returns the number of bytes the
    /// plane covers when it is valid.
    pub fn validate(&self, file_size: u64) -> Result<u64, DmaBufError> {
        if self.width == 0 || self.height == 0 {
            return Err(DmaBufError::EmptySize);
        }
        let bpp = bytes_per_pixel(self.format).ok_or(DmaBufError::UnsupportedFormat(self.format))?;
        if self.modifier != MODIFIER_LINEAR {
            return Err(DmaBufError::UnsupportedModifier(self.modifier));
        }
        let [plane] = self.planes.as_slice() else {
            return Err(DmaBufError::PlaneCount(self.planes.len()));
        };
        let minimum = u64::from(self.width) * u64::from(bpp);
        if u64::from(plane.stride) < minimum {
            return Err(DmaBufError::StrideTooSmall { stride: plane.stride, minimum });
        }
        let needed = u64::from(plane.offset) + u64::from(plane.stride) * u64::from(self.height);
        if needed > file_size {
            return Err(DmaBufError::OutOfBounds { needed, size: file_size });
        }
        Ok(needed)
    }
}
