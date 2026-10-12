//! dma-bufs: the cross-process GPU buffer (the reference platform's shared surface object).
//!
//! Owner: wave G1 agent `gpu-texture` (with `texture.rs`).

use std::os::fd::OwnedFd;

/// One plane of a dma-buf.
#[derive(Debug)]
pub struct DmaBufPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// A dma-buf as Wayland's linux-dmabuf and KMS describe it.
#[derive(Debug)]
pub struct DmaBuf {
    pub width: u32,
    pub height: u32,
    /// DRM fourcc (`PixelFormat::fourcc`).
    pub fourcc: u32,
    /// DRM format modifier (`DRM_FORMAT_MOD_LINEAR` = 0, `DRM_FORMAT_MOD_INVALID` = 0x00ff_ffff_ffff_ffff).
    pub modifier: u64,
    pub planes: Vec<DmaBufPlane>,
}

/// The DRM linear modifier.
pub const MOD_LINEAR: u64 = 0;
/// The DRM "implicit modifier" value.
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// A fourcc and modifier pair a device supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaBufFormat {
    pub fourcc: u32,
    pub modifier: u64,
}

impl DmaBuf {
    /// Duplicates every plane fd, so the same buffer can be imported twice (the compositor keeps a copy while a
    /// client reuses its `wl_buffer`).
    pub fn try_clone(&self) -> std::io::Result<DmaBuf> {
        let planes = self
            .planes
            .iter()
            .map(|p| Ok(DmaBufPlane { fd: p.fd.try_clone()?, offset: p.offset, stride: p.stride }))
            .collect::<std::io::Result<Vec<_>>>()?;
        Ok(DmaBuf { width: self.width, height: self.height, fourcc: self.fourcc, modifier: self.modifier, planes })
    }
}
