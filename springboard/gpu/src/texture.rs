//! Textures: a Vulkan image with its memory and view, made new, imported from a dma-buf or exported to one.
//!
//! Owner: wave G1 agent `gpu-texture`.

use crate::{DmaBuf, PixelFormat, Result};

/// What a texture is used for (combine with `|`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextureUsage(pub u32);

impl TextureUsage {
    /// Sampled in a shader.
    pub const SAMPLED: TextureUsage = TextureUsage(1);
    /// A render pass colour attachment.
    pub const RENDER_TARGET: TextureUsage = TextureUsage(2);
    /// A compute shader storage image.
    pub const STORAGE: TextureUsage = TextureUsage(4);
    /// The destination of uploads and copies.
    pub const COPY_DST: TextureUsage = TextureUsage(8);
    /// The source of copies and readback.
    pub const COPY_SRC: TextureUsage = TextureUsage(16);

    pub fn contains(self, other: TextureUsage) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for TextureUsage {
    type Output = TextureUsage;
    fn bitor(self, rhs: TextureUsage) -> TextureUsage {
        TextureUsage(self.0 | rhs.0)
    }
}

/// How to make a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub usage: TextureUsage,
    /// Allocate it exportable as a dma-buf (backing stores, scanout buffers).
    pub exportable: bool,
    /// The DRM modifiers the consumer accepts, most preferred first; empty means linear.
    pub modifiers: Vec<u64>,
}

/// A rectangle of texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    pub(crate) image: ash::vk::Image,
    pub(crate) view: ash::vk::ImageView,
    pub(crate) memory: Vec<ash::vk::DeviceMemory>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: PixelFormat,
    pub(crate) usage: TextureUsage,
    pub(crate) modifier: Option<u64>,
    /// The layout the last recorded use left the image in (tracked by the command module).
    pub(crate) layout: std::sync::Mutex<ash::vk::ImageLayout>,
    /// Whether the image belongs to another process or the display (`VK_QUEUE_FAMILY_FOREIGN_EXT` transfers).
    pub(crate) foreign: bool,
}

/// A GPU image. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct Texture {
    pub(crate) inner: std::sync::Arc<Inner>,
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    pub fn height(&self) -> u32 {
        self.inner.height
    }

    pub fn format(&self) -> PixelFormat {
        self.inner.format
    }

    /// The DRM modifier the image was created or imported with, when it is shareable.
    pub fn modifier(&self) -> Option<u64> {
        self.inner.modifier
    }

    /// Exports the texture as a dma-buf (it must have been made `exportable`).
    pub fn export_dmabuf(&self) -> Result<DmaBuf> {
        todo!("gpu-texture")
    }

    /// Uploads tightly packed (or `stride`-spaced) CPU pixels into `region`, through a staging buffer, and waits for
    /// the copy. For shm clients, the glyph atlas and decoded images.
    pub fn replace_region(&self, region: Region, bytes: &[u8], stride: usize) -> Result<()> {
        let _ = (region, bytes, stride);
        todo!("gpu-texture")
    }

    /// Reads `region` back to tightly packed CPU pixels (tests, screenshots, the bench). Waits for the GPU.
    pub fn read_region(&self, region: Region) -> Result<Vec<u8>> {
        let _ = region;
        todo!("gpu-texture")
    }
}
