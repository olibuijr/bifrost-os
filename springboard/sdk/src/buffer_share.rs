//! Pixel buffers shared with the GPU: the layout a buffer must have to cross into the GPU as a dma-buf, and the choice of
//! the fourcc and modifier a device imports it with.
//!
//! A [`BufferLayout`] fixes the size, the pixel format and the row stride, and checks that a byte buffer has exactly that
//! length. [`choose_import_format`] picks, from the list a device reports, the fourcc and modifier pair that matches the
//! layout. Nothing here opens a device or a file: the device list is passed in, so the checks run with no GPU.

use bifrost_gpu::dmabuf::DmaBufFormat;
use bifrost_gpu::format::PixelFormat;
use std::fmt;

/// Largest width or height a shared buffer may have, in pixels.
pub const MAX_SIDE: u32 = 16_384;

/// Largest row stride a shared buffer may have, in bytes: one full row of the widest format at the largest side. It keeps
/// `stride * height` inside `usize` on every target.
pub const MAX_STRIDE: usize = MAX_SIDE as usize * 8;

/// Why a layout or an import format is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// The width or the height is zero.
    EmptySize,
    /// The width or the height is above [`MAX_SIDE`].
    SideTooLarge { side: u32 },
    /// The row stride cannot hold one row of pixels.
    StrideTooSmall { stride: usize, minimum: usize },
    /// The row stride is above [`MAX_STRIDE`].
    StrideTooLarge { stride: usize },
    /// The row stride is not a whole number of pixels.
    StrideNotPixelAligned { stride: usize, bytes_per_pixel: usize },
    /// The buffer length does not match the layout.
    LengthMismatch { expected: usize, actual: usize },
    /// The pixel format has no DRM fourcc, so it is never shared as a dma-buf.
    NotShareable(PixelFormat),
    /// The device lists no format for the fourcc (and modifier, when one was asked for).
    NoImportFormat { fourcc: u32, modifier: u64 },
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareError::EmptySize => f.write_str("A shared buffer needs a width and a height above zero"),
            ShareError::SideTooLarge { side } => write!(f, "A shared buffer side may be at most {MAX_SIDE} pixels, not {side}"),
            ShareError::StrideTooSmall { stride, minimum } => write!(f, "The row stride {stride} is below one row, {minimum} bytes"),
            ShareError::StrideTooLarge { stride } => write!(f, "The row stride {stride} is above the {MAX_STRIDE} byte limit"),
            ShareError::StrideNotPixelAligned { stride, bytes_per_pixel } => {
                write!(f, "The row stride {stride} is not a multiple of the {bytes_per_pixel}-byte pixel")
            }
            ShareError::LengthMismatch { expected, actual } => write!(f, "The buffer holds {actual} bytes, the layout needs {expected}"),
            ShareError::NotShareable(format) => write!(f, "{format:?} has no dma-buf format"),
            ShareError::NoImportFormat { fourcc, modifier } => {
                write!(f, "The device cannot import fourcc {fourcc:#010x} with modifier {modifier:#018x}")
            }
        }
    }
}

impl std::error::Error for ShareError {}

/// The size, pixel format and row stride of a buffer that is shared with the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferLayout {
    width: u32,
    height: u32,
    format: PixelFormat,
    stride: usize,
}

impl BufferLayout {
    /// A layout with rows packed one after another, no padding.
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Result<BufferLayout, ShareError> {
        let minimum = width as usize * format.bytes_per_pixel();
        BufferLayout::with_stride(width, height, format, minimum)
    }

    /// A layout with an explicit row stride. The stride must hold one whole row and be a whole number of pixels.
    pub fn with_stride(width: u32, height: u32, format: PixelFormat, stride: usize) -> Result<BufferLayout, ShareError> {
        if width == 0 || height == 0 {
            return Err(ShareError::EmptySize);
        }
        for side in [width, height] {
            if side > MAX_SIDE {
                return Err(ShareError::SideTooLarge { side });
            }
        }
        let bytes_per_pixel = format.bytes_per_pixel();
        let minimum = width as usize * bytes_per_pixel;
        if stride < minimum {
            return Err(ShareError::StrideTooSmall { stride, minimum });
        }
        if stride > MAX_STRIDE {
            return Err(ShareError::StrideTooLarge { stride });
        }
        if stride % bytes_per_pixel != 0 {
            return Err(ShareError::StrideNotPixelAligned { stride, bytes_per_pixel });
        }
        Ok(BufferLayout { width, height, format, stride })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Bytes from the start of one row to the start of the next.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The length of the whole buffer: one stride for every row.
    pub fn byte_len(&self) -> usize {
        self.stride * self.height as usize
    }

    /// Checks that `bytes` has exactly the length of this layout.
    pub fn check_bytes(&self, bytes: &[u8]) -> Result<(), ShareError> {
        let expected = self.byte_len();
        if bytes.len() == expected { Ok(()) } else { Err(ShareError::LengthMismatch { expected, actual: bytes.len() }) }
    }

    /// The DRM fourcc of the pixel format.
    pub fn fourcc(&self) -> Result<u32, ShareError> {
        self.format.fourcc().ok_or(ShareError::NotShareable(self.format))
    }
}

/// The fourcc and modifier pair a device imports this layout with. The first pair in the device's list that has the
/// layout's fourcc wins, and a named `modifier` must match exactly. With no modifier named, the first pair the device
/// lists for the fourcc is taken.
pub fn choose_import_format(layout: &BufferLayout, supported: &[DmaBufFormat], modifier: Option<u64>) -> Result<DmaBufFormat, ShareError> {
    let fourcc = layout.fourcc()?;
    supported
        .iter()
        .copied()
        .find(|format| format.fourcc == fourcc && modifier.map_or(true, |wanted| format.modifier == wanted))
        .ok_or(ShareError::NoImportFormat { fourcc, modifier: modifier.unwrap_or(bifrost_gpu::dmabuf::MOD_INVALID) })
}
