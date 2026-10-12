//! Pixel formats and plane layout for a shared buffer: element size, row stride and where each plane starts.
//! Pure model: sizes follow from the format rules and the row alignment only. Formats are named by their
//! four-character codes, as the graphics stack names them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Argb8888,
    Xrgb8888,
    Rgb565,
    /// Luma plane, then one interleaved chroma plane at half resolution.
    Nv12,
    /// Luma plane, then two separate chroma planes at half resolution.
    I420,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    ZeroSize,
    /// A subsampled format needs even width and height.
    OddSize,
    ZeroAlignment,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plane {
    /// In elements, not bytes.
    pub width: usize,
    pub height: usize,
    pub bytes_per_element: usize,
    /// Row stride in bytes, padded to the requested alignment.
    pub bytes_per_row: usize,
    /// Byte offset of the plane from the start of the buffer.
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceLayout {
    format: PixelFormat,
    width: usize,
    height: usize,
    planes: Vec<Plane>,
    total: usize,
}

const ALL_FORMATS: [PixelFormat; 5] = [
    PixelFormat::Argb8888,
    PixelFormat::Xrgb8888,
    PixelFormat::Rgb565,
    PixelFormat::Nv12,
    PixelFormat::I420,
];

impl PixelFormat {
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            PixelFormat::Argb8888 => *b"AR24",
            PixelFormat::Xrgb8888 => *b"XR24",
            PixelFormat::Rgb565 => *b"RG16",
            PixelFormat::Nv12 => *b"NV12",
            PixelFormat::I420 => *b"YU12",
        }
    }

    pub fn from_fourcc(code: [u8; 4]) -> Option<Self> {
        ALL_FORMATS.iter().copied().find(|f| f.fourcc() == code)
    }

    pub fn plane_count(self) -> usize {
        match self {
            PixelFormat::Nv12 => 2,
            PixelFormat::I420 => 3,
            _ => 1,
        }
    }

    pub fn is_subsampled(self) -> bool {
        matches!(self, PixelFormat::Nv12 | PixelFormat::I420)
    }
}

fn round_up(n: usize, align: usize) -> Result<usize, LayoutError> {
    let padded = n.checked_add(align - 1).ok_or(LayoutError::Overflow)?;
    Ok(padded / align * align)
}

impl SurfaceLayout {
    /// Lays out a buffer of `width` by `height` pixels whose rows start on multiples of `row_alignment` bytes.
    pub fn new(
        format: PixelFormat,
        width: usize,
        height: usize,
        row_alignment: usize,
    ) -> Result<Self, LayoutError> {
        if width == 0 || height == 0 {
            return Err(LayoutError::ZeroSize);
        }
        if row_alignment == 0 {
            return Err(LayoutError::ZeroAlignment);
        }
        if format.is_subsampled() && (width % 2 != 0 || height % 2 != 0) {
            return Err(LayoutError::OddSize);
        }
        // (width in elements, height, bytes per element) for each plane, in buffer order.
        let dims: Vec<(usize, usize, usize)> = match format {
            PixelFormat::Argb8888 | PixelFormat::Xrgb8888 => vec![(width, height, 4)],
            PixelFormat::Rgb565 => vec![(width, height, 2)],
            PixelFormat::Nv12 => vec![(width, height, 1), (width / 2, height / 2, 2)],
            PixelFormat::I420 => vec![
                (width, height, 1),
                (width / 2, height / 2, 1),
                (width / 2, height / 2, 1),
            ],
        };
        let mut planes = Vec::with_capacity(dims.len());
        let mut offset = 0usize;
        for (w, h, bpe) in dims {
            let row_bytes = w.checked_mul(bpe).ok_or(LayoutError::Overflow)?;
            let bytes_per_row = round_up(row_bytes, row_alignment)?;
            let size = bytes_per_row.checked_mul(h).ok_or(LayoutError::Overflow)?;
            planes.push(Plane {
                width: w,
                height: h,
                bytes_per_element: bpe,
                bytes_per_row,
                offset,
                size,
            });
            offset = offset.checked_add(size).ok_or(LayoutError::Overflow)?;
        }
        Ok(SurfaceLayout {
            format,
            width,
            height,
            planes,
            total: offset,
        })
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }

    pub fn plane(&self, index: usize) -> Option<&Plane> {
        self.planes.get(index)
    }

    /// Bytes needed for the whole buffer, all planes included.
    pub fn total_size(&self) -> usize {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_format_has_one_plane_padded_to_the_alignment() {
        // 100 pixels of 4 bytes is 400 bytes; 400 rounded up to 64 is 448.
        let l = SurfaceLayout::new(PixelFormat::Argb8888, 100, 50, 64).unwrap();
        assert_eq!(l.planes().len(), 1);
        let p = l.plane(0).unwrap();
        assert_eq!(p.bytes_per_row, 448);
        assert_eq!(p.size, 448 * 50);
        assert_eq!(l.total_size(), 22400);
    }

    #[test]
    fn a_sixteen_bit_format_with_no_padding_is_two_bytes_per_pixel() {
        let l = SurfaceLayout::new(PixelFormat::Rgb565, 3, 2, 1).unwrap();
        let p = l.plane(0).unwrap();
        assert_eq!(p.bytes_per_element, 2);
        assert_eq!(p.bytes_per_row, 6);
        assert_eq!(l.total_size(), 12);
    }

    #[test]
    fn nv12_full_hd_has_a_luma_plane_then_an_interleaved_chroma_plane() {
        let l = SurfaceLayout::new(PixelFormat::Nv12, 1920, 1080, 16).unwrap();
        assert_eq!(l.format().plane_count(), 2);
        let luma = l.plane(0).unwrap();
        assert_eq!(luma.size, 1920 * 1080);
        assert_eq!(luma.offset, 0);
        let chroma = l.plane(1).unwrap();
        assert_eq!((chroma.width, chroma.height), (960, 540));
        assert_eq!(chroma.bytes_per_element, 2);
        assert_eq!(chroma.bytes_per_row, 1920);
        assert_eq!(chroma.offset, 2073600);
        assert_eq!(chroma.size, 1036800);
        // Three halves of a luma plane: 1.5 * 1920 * 1080.
        assert_eq!(l.total_size(), 3110400);
    }

    #[test]
    fn i420_has_three_planes_and_the_chroma_planes_follow_one_another() {
        let l = SurfaceLayout::new(PixelFormat::I420, 10, 4, 1).unwrap();
        assert_eq!(l.planes().len(), 3);
        assert_eq!(l.plane(1).unwrap().offset, 40);
        assert_eq!(l.plane(1).unwrap().size, 10);
        assert_eq!(l.plane(2).unwrap().offset, 50);
        assert_eq!(l.total_size(), 60);
    }

    #[test]
    fn chroma_rows_are_padded_like_luma_rows() {
        // Width 100 gives chroma rows of 100 bytes, padded to 128 at alignment 64.
        let l = SurfaceLayout::new(PixelFormat::Nv12, 100, 50, 64).unwrap();
        assert_eq!(l.plane(0).unwrap().bytes_per_row, 128);
        assert_eq!(l.plane(0).unwrap().size, 6400);
        assert_eq!(l.plane(1).unwrap().bytes_per_row, 128);
        assert_eq!(l.plane(1).unwrap().size, 3200);
        assert_eq!(l.total_size(), 9600);
    }

    #[test]
    fn invalid_sizes_and_alignments_are_refused() {
        assert_eq!(
            SurfaceLayout::new(PixelFormat::Argb8888, 0, 4, 1),
            Err(LayoutError::ZeroSize)
        );
        assert_eq!(
            SurfaceLayout::new(PixelFormat::Argb8888, 4, 4, 0),
            Err(LayoutError::ZeroAlignment)
        );
        assert_eq!(
            SurfaceLayout::new(PixelFormat::Nv12, 3, 4, 1),
            Err(LayoutError::OddSize)
        );
        // An odd width is fine for a packed format.
        assert!(SurfaceLayout::new(PixelFormat::Argb8888, 3, 4, 1).is_ok());
    }

    #[test]
    fn fourcc_codes_round_trip_and_unknown_codes_are_none() {
        assert_eq!(PixelFormat::from_fourcc(*b"NV12"), Some(PixelFormat::Nv12));
        assert_eq!(PixelFormat::I420.fourcc(), *b"YU12");
        assert_eq!(PixelFormat::from_fourcc(*b"ZZZZ"), None);
    }
}
