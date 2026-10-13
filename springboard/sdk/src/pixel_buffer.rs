//! Pixel buffers and their pool (Core Video's `CVPixelBuffer`, `CVPixelBufferLockBaseAddress`, `CVPixelBufferGetBaseAddress`,
//! `CVPixelBufferGetBytesPerRow`, `CVPixelBufferPool`).
//!
//! A [`PixelBuffer`] owns its planes in main memory. The layout comes from the format table in
//! `bifrost_render::pixel_format`: tight row strides, one plane per component group, odd chroma sizes rounded up.
//! Bytes are reached only under a lock, as in Core Video: [`PixelBuffer::lock`] counts the locks, [`PixelBuffer::unlock`]
//! releases one, and [`PixelBuffer::base_address`] returns None while no lock is held. A read-only lock denies
//! [`PixelBuffer::base_address_mut`]. [`PixelBufferPool`] recycles buffers of one format and size through
//! `bifrost_render::buffer_pool`. Nothing here touches the GPU: buffers the presenter renders into stay in the system layer.

use bifrost_render::buffer_attach::Rect;
use bifrost_render::pixel_format::{self, FourCc, PixelFormat};

pub use bifrost_render::buffer_attach::{Attachments, Primaries, Transfer, YcbcrMatrix};
pub use bifrost_render::buffer_pool::{Config as PoolConfig, PoolError};

/// Why a pixel buffer could not be made or unlocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelBufferError {
    /// The format code is not in the system's table (`kCVReturnInvalidPixelFormat`).
    UnknownFormat,
    /// A width or height is zero, or the buffer size overflows.
    InvalidSize,
    /// `unlock` with no lock of that kind held.
    NotLocked,
}

/// A buffer of pixels in main memory, one byte vector per plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelBuffer {
    layout: &'static PixelFormat,
    width: usize,
    height: usize,
    planes: Vec<Vec<u8>>,
    read_only_locks: usize,
    read_write_locks: usize,
    attachments: Attachments,
    revision: u64,
}

impl PixelBuffer {
    /// A zeroed buffer of `width` by `height` pixels in the format with this code.
    pub fn new(code: FourCc, width: usize, height: usize) -> Result<Self, PixelBufferError> {
        let layout = pixel_format::find(code).ok_or(PixelBufferError::UnknownFormat)?;
        if width == 0 || height == 0 || layout.buffer_len(width, height).is_none() {
            return Err(PixelBufferError::InvalidSize);
        }
        Ok(Self::alloc(layout, width, height))
    }

    /// Allocates a zeroed buffer. The caller has checked the size against the format (`buffer_len`).
    fn alloc(layout: &'static PixelFormat, width: usize, height: usize) -> Self {
        let planes: Vec<Vec<u8>> = (0..layout.planes.len())
            .map(|i| {
                let len = layout.plane_size(i, width, height).map_or(0, |(row, rows)| row * rows);
                vec![0u8; len]
            })
            .collect();
        PixelBuffer {
            layout,
            width,
            height,
            planes,
            read_only_locks: 0,
            read_write_locks: 0,
            attachments: Attachments::default(),
            revision: 0,
        }
    }

    /// The format code, such as NV12.
    pub fn code(&self) -> FourCc {
        self.layout.code
    }

    /// The format's name, such as "NV12".
    pub fn format_name(&self) -> &'static str {
        self.layout.name
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// True when the components sit in separate planes.
    pub fn is_planar(&self) -> bool {
        self.layout.is_planar()
    }

    /// Number of planes: 1 for packed formats, 2 for NV12 and NV16.
    pub fn plane_count(&self) -> usize {
        self.layout.planes.len()
    }

    /// Bytes per row of plane `plane` (plane 0 for packed formats). None when the plane does not exist.
    pub fn bytes_per_row(&self, plane: usize) -> Option<usize> {
        self.layout.plane_size(plane, self.width, self.height).map(|(row, _)| row)
    }

    /// Width of plane `plane` in its own samples; chroma planes round odd widths up. None when the plane does not exist.
    pub fn plane_width(&self, plane: usize) -> Option<usize> {
        let p = self.layout.planes.get(plane)?;
        self.width.checked_add(p.h_sub - 1).map(|w| w / p.h_sub)
    }

    /// Rows of plane `plane`; chroma planes round odd heights up. None when the plane does not exist.
    pub fn plane_height(&self, plane: usize) -> Option<usize> {
        self.layout.plane_size(plane, self.width, self.height).map(|(_, rows)| rows)
    }

    /// Total bytes of all planes.
    pub fn data_size(&self) -> usize {
        self.planes.iter().map(|p| p.len()).sum()
    }

    /// True while at least one lock of either kind is held.
    pub fn is_locked(&self) -> bool {
        self.read_only_locks + self.read_write_locks > 0
    }

    /// Takes a lock: read-only, or read-write when `read_only` is false. Locks nest.
    pub fn lock(&mut self, read_only: bool) {
        if read_only {
            self.read_only_locks += 1;
        } else {
            self.read_write_locks += 1;
        }
    }

    /// Releases one lock of the given kind. `NotLocked` when none of that kind is held. Releasing a read-write lock
    /// counts as a change of the pixels and advances [`PixelBuffer::revision`].
    pub fn unlock(&mut self, read_only: bool) -> Result<(), PixelBufferError> {
        let count = if read_only { &mut self.read_only_locks } else { &mut self.read_write_locks };
        if *count == 0 {
            return Err(PixelBufferError::NotLocked);
        }
        *count -= 1;
        if !read_only {
            self.revision += 1;
        }
        Ok(())
    }

    /// Counts the read-write unlocks so far. A cache that holds a copy of the pixels compares this value to know the
    /// copy is stale.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The colour metadata attached to the buffer (`CVBufferCopyAttachments`).
    pub fn attachments(&self) -> &Attachments {
        &self.attachments
    }

    /// Replaces the colour metadata (`CVBufferSetAttachment`, for each key at once). Attachments do not change the
    /// pixels, so the revision stays the same.
    pub fn set_attachments(&mut self, attachments: Attachments) {
        self.attachments = attachments;
    }

    /// The working colour space the attachments name, when the renderer has it (`CVImageBufferGetColorSpace`).
    pub fn color_space(&self) -> Option<bifrost_render::colorspace::Space> {
        self.attachments.color_space()
    }

    /// The clean aperture clipped to the buffer (`CVImageBufferGetCleanRect`).
    pub fn clean_rect(&self) -> Rect {
        self.attachments.clean_rect(self.width, self.height)
    }

    /// The bytes of plane `plane`, readable only while locked. None when unlocked or the plane does not exist.
    pub fn base_address(&self, plane: usize) -> Option<&[u8]> {
        if !self.is_locked() {
            return None;
        }
        self.planes.get(plane).map(|p| p.as_slice())
    }

    /// The bytes of plane `plane` for writing: needs a read-write lock. None otherwise.
    pub fn base_address_mut(&mut self, plane: usize) -> Option<&mut [u8]> {
        if self.read_write_locks == 0 {
            return None;
        }
        self.planes.get_mut(plane).map(|p| p.as_mut_slice())
    }
}

/// A pool of pixel buffers of one format and size (`CVPixelBufferPool`). Released buffers are reused; the limits come
/// from [`PoolConfig`]. Times are tick counts the caller supplies, as in `bifrost_render::buffer_pool`.
#[derive(Debug)]
pub struct PixelBufferPool {
    layout: &'static PixelFormat,
    width: usize,
    height: usize,
    pool: bifrost_render::buffer_pool::Pool<PixelBuffer>,
}

impl PixelBufferPool {
    /// An empty pool. The format and size are checked once here, so later allocations cannot fail.
    pub fn new(code: FourCc, width: usize, height: usize, config: PoolConfig) -> Result<Self, PixelBufferError> {
        let probe = PixelBuffer::new(code, width, height)?;
        Ok(PixelBufferPool {
            layout: probe.layout,
            width,
            height,
            pool: bifrost_render::buffer_pool::Pool::new(config),
        })
    }

    /// Buffers created and not dropped, free or in use.
    pub fn live(&self) -> usize {
        self.pool.live()
    }

    /// Buffers free for reuse right now.
    pub fn free_count(&self) -> usize {
        self.pool.free_count()
    }

    /// Creates buffers up to the minimum count at tick `now`. Returns how many were created.
    pub fn prewarm(&mut self, now: u64) -> usize {
        let (layout, width, height) = (self.layout, self.width, self.height);
        self.pool.prewarm(now, || PixelBuffer::alloc(layout, width, height))
    }

    /// A buffer: a free one when available, else a new one while under the threshold.
    pub fn acquire(&mut self) -> Result<PixelBuffer, PoolError> {
        let (layout, width, height) = (self.layout, self.width, self.height);
        self.pool.acquire(|| PixelBuffer::alloc(layout, width, height))
    }

    /// Returns a buffer to the pool at tick `now`. Buffers of another format or size are not checked here.
    pub fn release(&mut self, buf: PixelBuffer, now: u64) {
        self.pool.release(buf, now);
    }

    /// Drops free buffers unused for longer than the maximum age at tick `now`. Returns how many were dropped.
    pub fn age(&mut self, now: u64) -> usize {
        self.pool.age(now)
    }

    /// Drops every free buffer. Returns how many were dropped.
    pub fn flush(&mut self) -> usize {
        self.pool.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &str) -> FourCc {
        FourCc::parse(s).unwrap()
    }

    #[test]
    fn nv12_planes_round_odd_chroma_up() {
        // 5 x 3 picture: luma rows of 5 bytes by 3 rows = 15; chroma 3 samples of 2 bytes = 6 by 2 rows = 12.
        let buf = PixelBuffer::new(code("NV12"), 5, 3).unwrap();
        assert!(buf.is_planar());
        assert_eq!(buf.plane_count(), 2);
        assert_eq!(buf.bytes_per_row(0), Some(5));
        assert_eq!(buf.bytes_per_row(1), Some(6));
        assert_eq!(buf.plane_width(1), Some(3));
        assert_eq!(buf.plane_height(0), Some(3));
        assert_eq!(buf.plane_height(1), Some(2));
        assert_eq!(buf.data_size(), 27);
        assert_eq!(buf.bytes_per_row(2), None);
    }

    #[test]
    fn unknown_format_and_empty_size_are_refused() {
        assert_eq!(PixelBuffer::new(code("XX99"), 4, 4), Err(PixelBufferError::UnknownFormat));
        assert_eq!(PixelBuffer::new(code("NV12"), 0, 4), Err(PixelBufferError::InvalidSize));
        assert_eq!(PixelBuffer::new(code("NV12"), 4, 0), Err(PixelBufferError::InvalidSize));
    }

    #[test]
    fn bytes_are_hidden_until_locked() {
        let mut buf = PixelBuffer::new(code("AR24"), 3, 2).unwrap();
        assert_eq!(buf.bytes_per_row(0), Some(12));
        assert!(buf.base_address(0).is_none());
        buf.lock(true);
        assert_eq!(buf.base_address(0).map(|b| b.len()), Some(24));
        // A read-only lock does not allow writing.
        assert!(buf.base_address_mut(0).is_none());
    }

    #[test]
    fn write_under_read_write_lock_is_seen_by_the_next_read() {
        let mut buf = PixelBuffer::new(code("AR24"), 3, 2).unwrap();
        buf.lock(false);
        buf.base_address_mut(0).unwrap()[0] = 0xff;
        buf.unlock(false).unwrap();
        buf.lock(true);
        assert_eq!(buf.base_address(0).unwrap()[0], 0xff);
        assert_eq!(buf.base_address(0).unwrap()[1], 0);
    }

    #[test]
    fn locks_nest_and_unlock_without_a_lock_fails() {
        let mut buf = PixelBuffer::new(code("RG16"), 2, 1).unwrap();
        assert_eq!(buf.unlock(false), Err(PixelBufferError::NotLocked));
        buf.lock(false);
        buf.lock(false);
        buf.unlock(false).unwrap();
        assert!(buf.is_locked());
        buf.unlock(false).unwrap();
        assert!(!buf.is_locked());
        assert_eq!(buf.unlock(false), Err(PixelBufferError::NotLocked));
    }

    #[test]
    fn pool_reuses_released_buffers_and_stops_at_the_threshold() {
        let config = PoolConfig { min_count: 1, max_age: None, threshold: Some(2) };
        let mut pool = PixelBufferPool::new(code("AR24"), 2, 2, config).unwrap();
        assert_eq!(pool.prewarm(0), 1);
        assert_eq!(pool.free_count(), 1);
        let a = pool.acquire().unwrap();
        let b = pool.acquire().unwrap();
        assert_eq!(pool.live(), 2);
        assert_eq!(pool.acquire(), Err(PoolError::WouldExceedThreshold));
        assert_eq!(a.data_size(), 16);
        pool.release(a, 5);
        assert_eq!(pool.free_count(), 1);
        assert!(pool.acquire().is_ok());
        assert_eq!(pool.live(), 2);
        pool.release(b, 6);
        assert_eq!(pool.flush(), 1);
        assert_eq!(pool.live(), 1);
    }

    #[test]
    fn revision_counts_read_write_unlocks_only() {
        let mut buf = PixelBuffer::new(code("AR24"), 2, 2).unwrap();
        assert_eq!(buf.revision(), 0);
        buf.lock(true);
        buf.unlock(true).unwrap();
        assert_eq!(buf.revision(), 0);
        buf.lock(false);
        buf.unlock(false).unwrap();
        assert_eq!(buf.revision(), 1);
        assert_eq!(buf.unlock(false), Err(PixelBufferError::NotLocked));
        assert_eq!(buf.revision(), 1);
    }

    #[test]
    fn attachments_default_to_none_and_full_clean_rect() {
        let buf = PixelBuffer::new(code("NV12"), 1920, 1080).unwrap();
        assert!(buf.attachments().is_empty());
        assert_eq!(buf.color_space(), None);
        assert_eq!(buf.clean_rect(), Rect { x: 0, y: 0, w: 1920, h: 1080 });
    }

    #[test]
    fn attachments_name_the_colour_space_and_clip_the_aperture() {
        let mut buf = PixelBuffer::new(code("NV12"), 12, 8).unwrap();
        let before = buf.revision();
        buf.set_attachments(Attachments {
            primaries: Some(Primaries::DisplayP3),
            transfer: Some(Transfer::Srgb),
            matrix: Some(YcbcrMatrix::Bt709),
            clean: Some(Rect { x: 4, y: 2, w: 10, h: 10 }),
        });
        assert_eq!(buf.color_space(), Some(bifrost_render::colorspace::Space::DisplayP3));
        assert_eq!(buf.clean_rect(), Rect { x: 4, y: 2, w: 8, h: 6 });
        // Attachments do not touch the pixels.
        assert_eq!(buf.revision(), before);
    }
}
