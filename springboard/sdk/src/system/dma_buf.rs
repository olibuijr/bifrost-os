//! Linux dma-buf descriptors for window surfaces: the layout a client sends with a shared buffer (format, modifier,
//! plane offset and stride) and the bounds check the system makes before it wraps the buffer. The file descriptor
//! itself is passed by the system's window protocol; this module holds only the layout the descriptor must satisfy.
//!
//! Only linear buffers are accepted, so the stride and size can be checked without a tiling agreement: one plane in a
//! 32-bit RGB format, or the two planes of NV12 (a luma plane and an interleaved half-size chroma plane, even width
//! and height) for video.

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
/// `DRM_FORMAT_NV12`: two planes, luma then interleaved chroma at half size in each direction.
pub const NV12: u32 = fourcc(*b"NV12");
/// `DRM_FORMAT_MOD_LINEAR`: no tiling.
pub const MODIFIER_LINEAR: u64 = 0;

/// Why a descriptor was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DmaBufError {
    /// A width or height is zero.
    EmptySize,
    /// The format is not one of the four 32-bit RGB formats or NV12.
    UnsupportedFormat(u32),
    /// NV12 needs an even width and height.
    OddSize { width: u32, height: u32 },
    /// The modifier is not linear.
    UnsupportedModifier(u64),
    /// The number of planes is not the format's (one for RGB, two for NV12).
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
        if self.format == NV12 {
            return self.validate_nv12(file_size);
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

    /// The NV12 case of [`Descriptor::validate`]: even size, linear, two planes with a stride of at least `width`
    /// bytes each (a chroma row holds `width / 2` pairs of two bytes), both inside the file.
    fn validate_nv12(&self, file_size: u64) -> Result<u64, DmaBufError> {
        if self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(DmaBufError::OddSize { width: self.width, height: self.height });
        }
        if self.modifier != MODIFIER_LINEAR {
            return Err(DmaBufError::UnsupportedModifier(self.modifier));
        }
        let [luma, chroma] = self.planes.as_slice() else {
            return Err(DmaBufError::PlaneCount(self.planes.len()));
        };
        let minimum = u64::from(self.width);
        let mut needed = 0u64;
        for (plane, rows) in [(luma, self.height), (chroma, self.height / 2)] {
            if u64::from(plane.stride) < minimum {
                return Err(DmaBufError::StrideTooSmall { stride: plane.stride, minimum });
            }
            needed = needed.max(u64::from(plane.offset) + u64::from(plane.stride) * u64::from(rows));
        }
        if needed > file_size {
            return Err(DmaBufError::OutOfBounds { needed, size: file_size });
        }
        Ok(needed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12(width: u32, height: u32, luma: Plane, chroma: Plane) -> Descriptor {
        Descriptor { width, height, format: NV12, modifier: MODIFIER_LINEAR, planes: vec![luma, chroma] }
    }

    #[test]
    fn nv12_planes_are_checked_against_the_file() {
        // 4 x 4: luma 4 rows of 4 bytes at 0 (16 bytes), chroma 2 rows of 4 bytes at 16 (8 bytes).
        let d = nv12(4, 4, Plane { offset: 0, stride: 4 }, Plane { offset: 16, stride: 4 });
        assert_eq!(d.validate(24), Ok(24));
        assert_eq!(d.validate(23), Err(DmaBufError::OutOfBounds { needed: 24, size: 23 }));
        // The planes may come in any order in the file: the chroma first still needs the luma's end.
        let swapped = nv12(4, 4, Plane { offset: 8, stride: 4 }, Plane { offset: 0, stride: 4 });
        assert_eq!(swapped.validate(24), Ok(24));
    }

    #[test]
    fn nv12_refuses_odd_sizes_short_strides_and_wrong_plane_counts() {
        let ok = Plane { offset: 0, stride: 4 };
        assert_eq!(nv12(3, 4, ok, ok).validate(100), Err(DmaBufError::OddSize { width: 3, height: 4 }));
        assert_eq!(nv12(4, 4, ok, Plane { offset: 16, stride: 2 }).validate(100), Err(DmaBufError::StrideTooSmall { stride: 2, minimum: 4 }));
        let mut one = nv12(4, 4, ok, ok);
        one.planes.pop();
        assert_eq!(one.validate(100), Err(DmaBufError::PlaneCount(1)));
        let mut tiled = nv12(4, 4, ok, ok);
        tiled.modifier = 7;
        assert_eq!(tiled.validate(100), Err(DmaBufError::UnsupportedModifier(7)));
    }

    #[test]
    fn rgb_descriptors_still_take_one_plane() {
        let d = Descriptor { width: 4, height: 2, format: ARGB8888, modifier: MODIFIER_LINEAR, planes: vec![Plane { offset: 0, stride: 16 }] };
        assert_eq!(d.validate(32), Ok(32));
    }
}
