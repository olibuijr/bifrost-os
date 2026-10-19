//! Pixel buffers and their pool (Core Video's `CVPixelBuffer`, `CVPixelBufferLockBaseAddress`, `CVPixelBufferGetBaseAddress`,
//! `CVPixelBufferGetBytesPerRow`, `CVPixelBufferPool`), and the plane layouts that media and frame conversion write.
//!
//! A [`PixelBuffer`] owns its planes in main memory. The layout comes from the format table in
//! `bifrost_render::pixel_format`: tight row strides, one plane per component group, odd chroma sizes rounded up.
//! Bytes are reached only under a lock, as in Core Video: [`PixelBuffer::lock`] counts the locks, [`PixelBuffer::unlock`]
//! releases one, and [`PixelBuffer::base_address`] returns None while no lock is held. A read-only lock denies
//! [`PixelBuffer::base_address_mut`]. [`PixelBufferPool`] recycles buffers of one format and size through
//! `bifrost_render::buffer_pool`. Nothing here touches the GPU: buffers the presenter renders into stay in the system layer.
//!
//! [`Layout`] is the separate plain description of where each byte of a BGRA or NV12 frame lives. It holds no pixels,
//! and the media layer uses it to address frames it writes.

use bifrost_render::buffer_attach::Rect;
use bifrost_render::pixel_format::{self, FourCc, PixelFormat as RenderFormat};

pub use bifrost_render::buffer_attach::{Attachments, Primaries, Transfer, YcbcrMatrix};
pub use bifrost_render::buffer_pool::{Config as PoolConfig, PoolError};

/// The pixel formats a [`Layout`] may use.
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
            PixelFormat::Nv12 => vec![(width, height, 1, width as usize), (width / 2, height / 2, 2, width as usize)],
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
    layout: &'static RenderFormat,
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
    fn alloc(layout: &'static RenderFormat, width: usize, height: usize) -> Self {
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

impl PixelBuffer {
    /// A buffer in the format with this code holding `img`, converted by `bifrost_render::pixel_convert` (the
    /// 32-bit RGB layouts, RGB565, NV12 and NV16). The buffer is unlocked and its attachments are empty; YCbCr
    /// formats use the BT.709 matrix, video range. Fails with `UnknownFormat` for a format without a conversion and
    /// `InvalidSize` for an empty image or one whose pixel count does not match its size.
    pub fn from_image(img: &bifrost_render::gfx::Image, code: FourCc) -> Result<Self, PixelBufferError> {
        let mut buf = Self::new(code, img.w, img.h)?;
        let planes = bifrost_render::pixel_convert::encode(img, buf.layout, YcbcrMatrix::Bt709).ok_or_else(|| {
            if img.w == 0 || img.h == 0 || img.px.len() != img.w * img.h {
                PixelBufferError::InvalidSize
            } else {
                PixelBufferError::UnknownFormat
            }
        })?;
        buf.planes = planes;
        Ok(buf)
    }

    /// A buffer that takes ownership of already-decoded `planes` (one tightly packed byte vector per plane, the sizes
    /// `PixelFormat::plane_size` gives) and their colour `attachments`: the way a decoder or a media app hands a
    /// frame over. Fails with `UnknownFormat` or `InvalidSize` like [`PixelBuffer::new`], and with `InvalidSize` when
    /// the plane count or any plane length is not what the format needs for this size.
    pub fn from_planes(code: FourCc, width: usize, height: usize, planes: Vec<Vec<u8>>, attachments: Attachments) -> Result<Self, PixelBufferError> {
        let mut buf = Self::new(code, width, height)?;
        let sizes_match = planes.len() == buf.planes.len() && planes.iter().zip(&buf.planes).all(|(given, own)| given.len() == own.len());
        if !sizes_match {
            return Err(PixelBufferError::InvalidSize);
        }
        buf.planes = planes;
        buf.attachments = attachments;
        Ok(buf)
    }

    /// The pixels as an image, for the image filters. Like any read of the bytes it needs a lock: `None` while the
    /// buffer is unlocked or when the format has no conversion. The YCbCr matrix is the one in the attachments,
    /// BT.709 when unset. The image is opaque for the formats without alpha.
    pub fn to_image(&self) -> Option<bifrost_render::gfx::Image> {
        if !self.is_locked() {
            return None;
        }
        let matrix = self.attachments.matrix.unwrap_or(YcbcrMatrix::Bt709);
        bifrost_render::pixel_convert::decode(self.layout, &self.planes, self.width, self.height, matrix)
    }

    /// Replaces the pixels with `img`, converted to this buffer's format. Needs a read-write lock, and the image must
    /// be the buffer's size. The revision advances when the read-write lock is released, as for any write. Returns
    /// false when the buffer is not write-locked, the size differs, or the format has no conversion.
    pub fn write_image(&mut self, img: &bifrost_render::gfx::Image) -> bool {
        if self.read_write_locks == 0 || img.w != self.width || img.h != self.height {
            return false;
        }
        let matrix = self.attachments.matrix.unwrap_or(YcbcrMatrix::Bt709);
        match bifrost_render::pixel_convert::encode(img, self.layout, matrix) {
            Some(planes) => {
                self.planes = planes;
                true
            }
            None => false,
        }
    }
}

/// A pool of pixel buffers of one format and size (`CVPixelBufferPool`). Released buffers are reused; the limits come
/// from [`PoolConfig`]. Times are tick counts the caller supplies, as in `bifrost_render::buffer_pool`.
#[derive(Debug)]
pub struct PixelBufferPool {
    layout: &'static RenderFormat,
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

    fn image(w: usize, h: usize, px: u32) -> bifrost_render::gfx::Image {
        bifrost_render::gfx::Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn an_image_goes_into_a_buffer_and_comes_back_only_under_a_lock() {
        let img = image(3, 2, 0xff11_2233);
        let mut buf = PixelBuffer::from_image(&img, code("AR24")).unwrap();
        assert_eq!((buf.width(), buf.height(), buf.data_size()), (3, 2, 24));
        assert!(buf.to_image().is_none(), "unlocked");
        buf.lock(true);
        assert_eq!(buf.to_image().unwrap(), img);
        assert_eq!(buf.base_address(0).unwrap()[..4], [0x11, 0x22, 0x33, 0xff]);
    }

    #[test]
    fn nv12_buffers_convert_with_the_attached_matrix() {
        let img = image(4, 4, 0xff00_00ff);
        let mut buf = PixelBuffer::from_image(&img, code("NV12")).unwrap();
        assert_eq!(buf.data_size(), 24);
        buf.lock(true);
        let back = buf.to_image().unwrap();
        assert!((back.px[0] & 0xff) >= 250 && (back.px[0] >> 8 & 0xff) <= 4);
        buf.unlock(true).unwrap();
        // Reading with a different matrix gives different colours: the attachment is honoured.
        buf.set_attachments(Attachments { matrix: Some(YcbcrMatrix::Bt601), ..Attachments::default() });
        buf.lock(true);
        assert_ne!(buf.to_image().unwrap().px[0], back.px[0]);
    }

    #[test]
    fn write_image_needs_a_write_lock_and_the_same_size() {
        let mut buf = PixelBuffer::new(code("AB24"), 2, 2).unwrap();
        let white = image(2, 2, 0xffff_ffff);
        assert!(!buf.write_image(&white), "no lock");
        buf.lock(true);
        assert!(!buf.write_image(&white), "read-only lock");
        buf.unlock(true).unwrap();
        buf.lock(false);
        assert!(!buf.write_image(&image(3, 2, 0)), "wrong size");
        assert!(buf.write_image(&white));
        assert_eq!(buf.to_image().unwrap(), white);
        buf.unlock(false).unwrap();
        assert_eq!(buf.revision(), 1);
    }

    #[test]
    fn decoded_planes_become_a_buffer_with_their_attachments() {
        let attachments = Attachments { matrix: Some(YcbcrMatrix::Bt601), ..Attachments::default() };
        let planes = vec![vec![126u8; 6], vec![128u8; 4]];
        let mut buf = PixelBuffer::from_planes(code("NV12"), 3, 2, planes.clone(), attachments).unwrap();
        assert_eq!(buf.attachments().matrix, Some(YcbcrMatrix::Bt601));
        buf.lock(true);
        assert_eq!(buf.base_address(0).unwrap(), &planes[0][..]);
        let grey = buf.to_image().unwrap();
        assert!((grey.px[0] & 0xff).abs_diff(128) <= 3);
        assert_eq!(
            PixelBuffer::from_planes(code("NV12"), 3, 2, vec![vec![0u8; 6]], Attachments::default()),
            Err(PixelBufferError::InvalidSize)
        );
        assert_eq!(
            PixelBuffer::from_planes(code("NV12"), 3, 2, vec![vec![0u8; 6], vec![0u8; 5]], Attachments::default()),
            Err(PixelBufferError::InvalidSize)
        );
    }

    #[test]
    fn from_image_refuses_bad_input() {
        assert_eq!(PixelBuffer::from_image(&image(2, 2, 0), code("XX99")), Err(PixelBufferError::UnknownFormat));
        let short = bifrost_render::gfx::Image { w: 2, h: 2, px: vec![0; 3] };
        assert_eq!(PixelBuffer::from_image(&short, code("AR24")), Err(PixelBufferError::InvalidSize));
        assert_eq!(PixelBuffer::from_image(&image(0, 0, 0), code("AR24")), Err(PixelBufferError::InvalidSize));
    }
}
