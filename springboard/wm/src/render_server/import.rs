//! Client buffers to `bifrost_gpu::Texture`s: dma-buf `wl_buffer`s are imported without a copy (cached per buffer
//! until destroyed, re-imported never); shm buffers are uploaded into a per-surface texture, only the damaged rows.
//! Also answers linux-dmabuf v4 feedback with `Device::supported_dmabuf_formats` for the main device.
//!
//! Owner: wave G1 agent `wm-import`.

use bifrost_gpu::{Device, Result, Texture};

/// A surface's current content on the GPU.
pub struct SurfaceTexture {
    pub texture: Texture,
    /// Whether it is a client dma-buf (sampled directly) or the server's copy of an shm buffer.
    pub zero_copy: bool,
    pub width: u32,
    pub height: u32,
}

/// Caches imports per buffer and shm upload textures per surface.
pub struct Importer {
    pub(crate) device: Device,
}

impl Importer {
    pub fn new(device: &Device) -> Importer {
        Importer { device: device.clone() }
    }

    /// Imports a dma-buf (described with Smithay's `Dmabuf` by the caller, converted to `bifrost_gpu::DmaBuf`).
    pub fn import_dmabuf(&mut self, key: u64, dmabuf: &bifrost_gpu::DmaBuf) -> Result<SurfaceTexture> {
        let _ = (key, dmabuf);
        todo!("wm-import")
    }

    /// Uploads shm pixels (`stride` bytes per row, BGRA8 or BGRX8) into the surface's texture, only `damage` rows.
    pub fn upload_shm(&mut self, surface_key: u64, width: u32, height: u32, stride: usize, bytes: &[u8], damage: &[(i32, i32, i32, i32)]) -> Result<SurfaceTexture> {
        let _ = (surface_key, width, height, stride, bytes, damage);
        todo!("wm-import")
    }

    /// Drops everything cached for a buffer or a surface that was destroyed.
    pub fn forget(&mut self, key: u64) {
        let _ = key;
        todo!("wm-import")
    }
}
