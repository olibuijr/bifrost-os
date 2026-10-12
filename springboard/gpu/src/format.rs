//! Pixel formats and their DRM fourcc and Vulkan equivalents.

use ash::vk;

/// The pixel formats BifrOSt draws and exchanges. Byte order follows the name (Vulkan convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit B, G, R, A in memory: DRM `ARGB8888` (little endian); the usual scanout and client format.
    Bgra8Unorm,
    /// 8-bit B, G, R, X: DRM `XRGB8888`; opaque scanout.
    Bgrx8Unorm,
    /// 8-bit R, G, B, A: DRM `ABGR8888`; the CPU raster's premultiplied RGBA8 (`gfx::pack`).
    Rgba8Unorm,
    /// One 8-bit channel: glyph atlas and masks.
    R8Unorm,
    /// 16-bit float RGBA: blur intermediates and wide-gamut content.
    Rgba16Float,
}

impl PixelFormat {
    /// The Vulkan format.
    pub fn vk(self) -> vk::Format {
        match self {
            PixelFormat::Bgra8Unorm | PixelFormat::Bgrx8Unorm => vk::Format::B8G8R8A8_UNORM,
            PixelFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
            PixelFormat::R8Unorm => vk::Format::R8_UNORM,
            PixelFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        }
    }

    /// The DRM fourcc, or `None` for formats that are never shared as dma-bufs.
    pub fn fourcc(self) -> Option<u32> {
        let code = |s: &[u8; 4]| u32::from_le_bytes(*s);
        match self {
            PixelFormat::Bgra8Unorm => Some(code(b"AR24")),
            PixelFormat::Bgrx8Unorm => Some(code(b"XR24")),
            PixelFormat::Rgba8Unorm => Some(code(b"AB24")),
            PixelFormat::R8Unorm => Some(code(b"R8  ")),
            PixelFormat::Rgba16Float => Some(code(b"AB4H")),
        }
    }

    /// The format a DRM fourcc names, if BifrOSt handles it.
    pub fn from_fourcc(fourcc: u32) -> Option<PixelFormat> {
        [PixelFormat::Bgra8Unorm, PixelFormat::Bgrx8Unorm, PixelFormat::Rgba8Unorm, PixelFormat::R8Unorm, PixelFormat::Rgba16Float]
            .into_iter()
            .find(|f| f.fourcc() == Some(fourcc))
    }

    /// Bytes per pixel.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::R8Unorm => 1,
            PixelFormat::Rgba16Float => 8,
            _ => 4,
        }
    }

    /// Whether the alpha channel is ignored (X formats).
    pub fn is_opaque(self) -> bool {
        self == PixelFormat::Bgrx8Unorm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_round_trips() {
        for f in [PixelFormat::Bgra8Unorm, PixelFormat::Bgrx8Unorm, PixelFormat::Rgba8Unorm, PixelFormat::R8Unorm, PixelFormat::Rgba16Float] {
            assert_eq!(PixelFormat::from_fourcc(f.fourcc().unwrap()), Some(f));
        }
    }

    #[test]
    fn argb8888_is_the_drm_code() {
        // DRM_FORMAT_ARGB8888 = fourcc_code('A', 'R', '2', '4') = 0x34325241
        assert_eq!(PixelFormat::Bgra8Unorm.fourcc(), Some(0x3432_5241));
    }
}
