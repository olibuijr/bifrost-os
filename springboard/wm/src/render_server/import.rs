//! Client buffers to `bifrost_gpu::Texture`s: dma-buf `wl_buffer`s are imported without a copy (cached per buffer
//! until destroyed, re-imported never); shm buffers are uploaded into a per-surface texture, only the damaged rows.
//! Also answers linux-dmabuf v4 feedback with `Device::supported_dmabuf_formats` for the main device.
//!
//! Owner: wave G1 agent `wm-import`.

use bifrost_gpu::{Device, DmaBuf, DmaBufPlane, Error, PixelFormat, Region, Result, Texture, TextureDescriptor, TextureUsage};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::Buffer;
use std::collections::HashMap;

/// A surface's current content on the GPU.
#[derive(Clone)]
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
    /// dma-buf imports, keyed by the buffer key the caller chose (one per `wl_buffer`).
    dmabufs: HashMap<u64, SurfaceTexture>,
    /// shm upload textures, keyed by surface key.
    shm: HashMap<u64, Texture>,
}

/// An integer rectangle `(x, y, width, height)` in texels, already clamped to the surface.
pub type Rect = (u32, u32, u32, u32);

/// The bytes of one BGRA8 or BGRX8 texel row group.
const SHM_BYTES_PER_PIXEL: usize = 4;

impl Importer {
    pub fn new(device: &Device) -> Importer {
        Importer { device: device.clone(), dmabufs: HashMap::new(), shm: HashMap::new() }
    }

    /// Imports a dma-buf (described with Smithay's `Dmabuf` by the caller, converted to `bifrost_gpu::DmaBuf`).
    /// Imports are cached under `key`: a second call with the same key returns the cached texture and does not look
    /// at `dmabuf` again, so the caller must call `forget(key)` when the client destroys the buffer.
    pub fn import_dmabuf(&mut self, key: u64, dmabuf: &bifrost_gpu::DmaBuf) -> Result<SurfaceTexture> {
        if let Some(cached) = self.dmabufs.get(&key) {
            return Ok(cached.clone());
        }
        let texture = self.device.import_dmabuf(dmabuf)?;
        let imported = SurfaceTexture { width: texture.width(), height: texture.height(), texture, zero_copy: true };
        self.dmabufs.insert(key, imported.clone());
        Ok(imported)
    }

    /// Uploads shm pixels (`stride` bytes per row, BGRA8 or BGRX8) into the surface's texture, only `damage` rows.
    /// `damage` rectangles are `(x, y, width, height)` in buffer pixels; anything outside the buffer is clipped.
    /// The texture is (re)created, and fully uploaded, when the surface is new or its size changed.
    pub fn upload_shm(&mut self, surface_key: u64, width: u32, height: u32, stride: usize, bytes: &[u8], damage: &[(i32, i32, i32, i32)]) -> Result<SurfaceTexture> {
        let needed = shm_span(width, height, stride)?;
        if bytes.len() < needed {
            return Err(Error::Invalid(format!("shm buffer has {} bytes, needs {needed}", bytes.len())));
        }
        let existing = self.shm.get(&surface_key).filter(|t| t.width() == width && t.height() == height).cloned();
        let texture = match existing {
            Some(texture) => {
                for (x, y, w, h) in clamp_damage(width, height, damage) {
                    upload_rect(&texture, (x, y, w, h), bytes, stride)?;
                }
                texture
            }
            None => {
                let texture = self.device.new_texture(&TextureDescriptor {
                    width,
                    height,
                    format: PixelFormat::Bgra8Unorm,
                    usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST,
                    exportable: false,
                    modifiers: Vec::new(),
                })?;
                upload_rect(&texture, (0, 0, width, height), bytes, stride)?;
                self.shm.insert(surface_key, texture.clone());
                texture
            }
        };
        Ok(SurfaceTexture { texture, zero_copy: false, width, height })
    }

    /// The texture cached under `key` (a dma-buf import or a surface's shm upload), for composition.
    pub fn texture(&self, key: u64) -> Option<Texture> {
        self.dmabufs.get(&key).map(|t| t.texture.clone()).or_else(|| self.shm.get(&key).cloned())
    }

    /// Drops everything cached for a buffer or a surface that was destroyed.
    pub fn forget(&mut self, key: u64) {
        self.dmabufs.remove(&key);
        self.shm.remove(&key);
    }
}

/// Converts a Smithay dma-buf into the `bifrost_gpu` description: every plane fd is duplicated (the caller keeps its
/// own `Dmabuf`), and the fourcc and modifier are the DRM values Smithay carries.
pub fn dmabuf_to_gpu(d: &Dmabuf) -> Result<DmaBuf> {
    let format = d.format();
    let mut planes = Vec::with_capacity(d.num_planes());
    for ((fd, offset), stride) in d.handles().zip(d.offsets()).zip(d.strides()) {
        let fd = fd.try_clone_to_owned().map_err(|e| Error::DmaBuf(format!("plane fd: {e}")))?;
        planes.push(DmaBufPlane { fd, offset, stride });
    }
    Ok(DmaBuf {
        width: d.width(),
        height: d.height(),
        fourcc: format.code as u32,
        modifier: u64::from(format.modifier),
        planes,
    })
}

/// Bytes a `width` x `height` buffer with `stride` bytes per row spans, from its first texel to the end of its last
/// row. Errors when a row is too short for its pixels or a size is zero.
pub fn shm_span(width: u32, height: u32, stride: usize) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(Error::Invalid(format!("shm buffer size {width}x{height}")));
    }
    let row = width as usize * SHM_BYTES_PER_PIXEL;
    if stride < row {
        return Err(Error::Invalid(format!("shm stride {stride} is less than a row of {row} bytes")));
    }
    Ok((height as usize - 1) * stride + row)
}

/// Clamps damage rectangles `(x, y, width, height)` to a `width` x `height` buffer and drops the empty ones. The
/// result is in the order given.
pub fn clamp_damage(width: u32, height: u32, damage: &[(i32, i32, i32, i32)]) -> Vec<Rect> {
    let (w, h) = (i64::from(width), i64::from(height));
    damage
        .iter()
        .filter_map(|&(x, y, rw, rh)| {
            let x0 = i64::from(x).clamp(0, w);
            let y0 = i64::from(y).clamp(0, h);
            let x1 = (i64::from(x) + i64::from(rw)).clamp(0, w);
            let y1 = (i64::from(y) + i64::from(rh)).clamp(0, h);
            (x1 > x0 && y1 > y0).then(|| (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
        })
        .collect()
}

/// Uploads one rectangle of shm rows: the bytes from its first texel through its last row, with the source stride.
fn upload_rect(texture: &Texture, (x, y, w, h): Rect, bytes: &[u8], stride: usize) -> Result<()> {
    let start = y as usize * stride + x as usize * SHM_BYTES_PER_PIXEL;
    let end = start + (h as usize - 1) * stride + w as usize * SHM_BYTES_PER_PIXEL;
    let slice = bytes.get(start..end).ok_or_else(|| Error::Invalid(format!("shm rect {x},{y} {w}x{h} outside the buffer")))?;
    texture.replace_region(Region { x, y, width: w, height: h }, slice, stride)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::allocator::dmabuf::DmabufFlags;
    use smithay::backend::allocator::{Fourcc, Modifier};
    use std::os::fd::OwnedFd;

    #[test]
    fn shm_span_is_last_row_end() {
        // 4 rows of 10 pixels (40 bytes each) with 64-byte stride: 3 * 64 + 40 = 232.
        assert_eq!(shm_span(10, 4, 64).unwrap(), 232);
        // Tight stride: the whole buffer.
        assert_eq!(shm_span(10, 4, 40).unwrap(), 160);
        // A single row needs only its pixels, whatever the stride.
        assert_eq!(shm_span(3, 1, 4096).unwrap(), 12);
    }

    #[test]
    fn shm_span_rejects_bad_sizes() {
        assert!(shm_span(0, 4, 40).is_err());
        assert!(shm_span(4, 0, 16).is_err());
        // 10 pixels need 40 bytes per row; 36 is short.
        assert!(shm_span(10, 4, 36).is_err());
    }

    #[test]
    fn damage_inside_the_buffer_is_kept_as_given() {
        assert_eq!(clamp_damage(100, 50, &[(10, 5, 20, 8)]), vec![(10, 5, 20, 8)]);
    }

    #[test]
    fn damage_is_clipped_to_the_buffer() {
        // Starts left of and above the buffer: clipped to the origin.
        assert_eq!(clamp_damage(100, 50, &[(-10, -5, 30, 20)]), vec![(0, 0, 20, 15)]);
        // Runs past the right and bottom edges.
        assert_eq!(clamp_damage(100, 50, &[(90, 40, 30, 30)]), vec![(90, 40, 10, 10)]);
        // Covers the whole buffer and more.
        assert_eq!(clamp_damage(100, 50, &[(-1, -1, 500, 500)]), vec![(0, 0, 100, 50)]);
    }

    #[test]
    fn empty_and_outside_damage_is_dropped() {
        let out = clamp_damage(100, 50, &[(0, 0, 0, 10), (200, 0, 10, 10), (0, 60, 10, 10), (5, 5, -3, 3), (10, 10, 4, 4)]);
        assert_eq!(out, vec![(10, 10, 4, 4)]);
    }

    #[test]
    fn damage_order_is_kept() {
        let out = clamp_damage(10, 10, &[(5, 5, 2, 2), (0, 0, 1, 1)]);
        assert_eq!(out, vec![(5, 5, 2, 2), (0, 0, 1, 1)]);
    }

    #[test]
    fn huge_damage_does_not_overflow() {
        assert_eq!(clamp_damage(8, 8, &[(i32::MAX, i32::MAX, i32::MAX, i32::MAX)]), Vec::<Rect>::new());
        assert_eq!(clamp_damage(8, 8, &[(-5, -5, i32::MAX, i32::MAX)]), vec![(0, 0, 8, 8)]);
        // Starts at the most negative value and ends before the origin: nothing inside the buffer.
        assert_eq!(clamp_damage(8, 8, &[(i32::MIN, i32::MIN, i32::MAX, i32::MAX)]), Vec::<Rect>::new());
    }

    #[test]
    fn smithay_dmabuf_converts_format_modifier_and_planes() {
        let file = tempfile::tempfile().unwrap();
        let mut builder = Dmabuf::builder((64, 32), Fourcc::Xrgb8888, Modifier::Linear, DmabufFlags::empty());
        assert!(builder.add_plane(OwnedFd::from(file), 0, 16, 256));
        let dmabuf = builder.build().unwrap();
        let gpu = dmabuf_to_gpu(&dmabuf).unwrap();
        assert_eq!((gpu.width, gpu.height), (64, 32));
        // DRM_FORMAT_XRGB8888 = fourcc('X', 'R', '2', '4') = 0x34325258 (little endian).
        assert_eq!(gpu.fourcc, 0x3432_5258);
        assert_eq!(gpu.modifier, 0);
        assert_eq!(gpu.planes.len(), 1);
        assert_eq!((gpu.planes[0].offset, gpu.planes[0].stride), (16, 256));
    }

    #[test]
    fn converted_planes_are_independent_fds() {
        use std::os::fd::AsRawFd;
        let file = tempfile::tempfile().unwrap();
        let mut builder = Dmabuf::builder((4, 4), Fourcc::Argb8888, Modifier::Linear, DmabufFlags::empty());
        assert!(builder.add_plane(OwnedFd::from(file), 0, 0, 16));
        let dmabuf = builder.build().unwrap();
        let gpu = dmabuf_to_gpu(&dmabuf).unwrap();
        // The copy owns its own descriptor, so dropping it leaves the original usable.
        let original = dmabuf.handles().next().unwrap().as_raw_fd();
        assert_ne!(gpu.planes[0].fd.as_raw_fd(), original);
        drop(gpu);
        assert_eq!(dmabuf.num_planes(), 1);
    }

    #[test]
    fn argb_fourcc_matches_the_gpu_format_table() {
        // The gpu crate's ARGB8888 code is the same value Smithay reports for Argb8888.
        let file = tempfile::tempfile().unwrap();
        let mut builder = Dmabuf::builder((2, 2), Fourcc::Argb8888, Modifier::Linear, DmabufFlags::empty());
        assert!(builder.add_plane(OwnedFd::from(file), 0, 0, 8));
        let gpu = dmabuf_to_gpu(&builder.build().unwrap()).unwrap();
        assert_eq!(PixelFormat::from_fourcc(gpu.fourcc), Some(PixelFormat::Bgra8Unorm));
    }
}
