//! Pixel format descriptions for buffers that the system allocates and other code reads: the four-character
//! type code (the DRM fourcc values the GPU path already uses), the planes of the buffer, the bytes per pixel of
//! each plane and its chroma subsampling. Layout only: nothing is converted or allocated here. Row strides are
//! tight, with no padding.

/// A four-character code naming a pixel format. The first character sits in the low byte, as in DRM fourccs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FourCc(pub u32);

impl FourCc {
    /// The code for four ASCII bytes.
    pub const fn new(code: [u8; 4]) -> Self {
        FourCc(u32::from_le_bytes(code))
    }

    /// The four bytes, first character first.
    pub fn bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    /// Parses a code such as "NV12". None unless the text is exactly four printable ASCII bytes.
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 4 || !b.iter().all(|c| c.is_ascii_graphic()) {
            return None;
        }
        Some(FourCc::new([b[0], b[1], b[2], b[3]]))
    }
}

impl std::fmt::Display for FourCc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in self.bytes() {
            write!(f, "{}", c as char)?;
        }
        Ok(())
    }
}

/// One plane of a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    /// Bytes stored for one sample of this plane. A chroma pair in a packed plane counts as one sample.
    pub bytes_per_pixel: usize,
    /// Horizontal subsampling: one stored sample covers this many picture columns (1 or 2).
    pub h_sub: usize,
    /// Vertical subsampling: one stored sample covers this many picture rows (1 or 2).
    pub v_sub: usize,
}

/// A pixel format: its code, its name and its planes. Non-planar formats have one plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub code: FourCc,
    pub name: &'static str,
    pub planes: &'static [Plane],
}

const RGB8: Plane = Plane { bytes_per_pixel: 4, h_sub: 1, v_sub: 1 };
const RGB16: Plane = Plane { bytes_per_pixel: 2, h_sub: 1, v_sub: 1 };
const LUMA8: Plane = Plane { bytes_per_pixel: 1, h_sub: 1, v_sub: 1 };
const CHROMA_420: Plane = Plane { bytes_per_pixel: 2, h_sub: 2, v_sub: 2 };
const CHROMA_422: Plane = Plane { bytes_per_pixel: 2, h_sub: 2, v_sub: 1 };

/// The formats this system knows, with the fourcc values of the DRM format list.
pub const TABLE: &[PixelFormat] = &[
    PixelFormat { code: FourCc::new(*b"AR24"), name: "ARGB8888", planes: &[RGB8] },
    PixelFormat { code: FourCc::new(*b"XR24"), name: "XRGB8888", planes: &[RGB8] },
    PixelFormat { code: FourCc::new(*b"AB24"), name: "ABGR8888", planes: &[RGB8] },
    PixelFormat { code: FourCc::new(*b"XB24"), name: "XBGR8888", planes: &[RGB8] },
    PixelFormat { code: FourCc::new(*b"RG16"), name: "RGB565", planes: &[RGB16] },
    PixelFormat { code: FourCc::new(*b"NV12"), name: "NV12", planes: &[LUMA8, CHROMA_420] },
    PixelFormat { code: FourCc::new(*b"NV16"), name: "NV16", planes: &[LUMA8, CHROMA_422] },
];

/// The format with this code, if the system knows it.
pub fn find(code: FourCc) -> Option<&'static PixelFormat> {
    TABLE.iter().find(|f| f.code == code)
}

impl PixelFormat {
    /// True when the samples of each component sit in their own plane.
    pub fn is_planar(&self) -> bool {
        self.planes.len() > 1
    }

    /// Bytes per row and row count of plane `index` for a picture of `width` by `height` pixels. Odd sizes round
    /// the subsampled planes up. None when the plane does not exist or the size overflows.
    pub fn plane_size(&self, index: usize, width: usize, height: usize) -> Option<(usize, usize)> {
        let p = self.planes.get(index)?;
        let cols = width.checked_add(p.h_sub - 1)? / p.h_sub;
        let rows = height.checked_add(p.v_sub - 1)? / p.v_sub;
        Some((cols.checked_mul(p.bytes_per_pixel)?, rows))
    }

    /// Total bytes of a buffer of `width` by `height` pixels, all planes together. None on overflow.
    pub fn buffer_len(&self, width: usize, height: usize) -> Option<usize> {
        let mut total = 0usize;
        for i in 0..self.planes.len() {
            let (row, rows) = self.plane_size(i, width, height)?;
            total = total.checked_add(row.checked_mul(rows)?)?;
        }
        Some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_parses_and_prints_its_text() {
        let nv12 = FourCc::parse("NV12").unwrap();
        assert_eq!(nv12.bytes(), [b'N', b'V', b'1', b'2']);
        assert_eq!(nv12.to_string(), "NV12");
        assert_eq!(FourCc::parse("NV1"), None);
        assert_eq!(FourCc::parse("NV12X"), None);
        assert_eq!(FourCc::parse("NV\u{e9}"), None);
    }

    #[test]
    fn argb_code_is_found_by_its_drm_name() {
        let f = find(FourCc::parse("AR24").unwrap()).unwrap();
        assert_eq!(f.name, "ARGB8888");
        assert!(!f.is_planar());
        assert_eq!(find(FourCc::parse("XX99").unwrap()), None);
    }

    #[test]
    fn rgba_buffer_is_four_bytes_per_pixel() {
        let f = find(FourCc::parse("AR24").unwrap()).unwrap();
        assert_eq!(f.buffer_len(3, 2), Some(24));
        assert_eq!(f.plane_size(0, 3, 2), Some((12, 2)));
        assert_eq!(f.plane_size(1, 3, 2), None);
    }

    #[test]
    fn nv12_rounds_odd_chroma_up() {
        // 5 x 3 picture: luma 5 bytes by 3 rows = 15; chroma ceil(5/2)=3 samples of 2 bytes = 6 by ceil(3/2)=2 rows = 12.
        let f = find(FourCc::parse("NV12").unwrap()).unwrap();
        assert!(f.is_planar());
        assert_eq!(f.plane_size(0, 5, 3), Some((5, 3)));
        assert_eq!(f.plane_size(1, 5, 3), Some((6, 2)));
        assert_eq!(f.buffer_len(5, 3), Some(27));
    }

    #[test]
    fn nv16_halves_only_the_width_of_chroma() {
        // 4 x 2 picture: luma 8 bytes; chroma 2 samples of 2 bytes = 4 by 2 rows = 8; total 16.
        let f = find(FourCc::parse("NV16").unwrap()).unwrap();
        assert_eq!(f.plane_size(1, 4, 2), Some((4, 2)));
        assert_eq!(f.buffer_len(4, 2), Some(16));
    }

    #[test]
    fn packed_16_bit_and_overflow_are_handled() {
        let rgb565 = find(FourCc::parse("RG16").unwrap()).unwrap();
        assert_eq!(rgb565.buffer_len(2, 1), Some(4));
        assert_eq!(rgb565.buffer_len(usize::MAX, 2), None);
    }
}
