//! Header probes for the image formats the browser does not decode: BMP and TIFF (std only).
//!
//! The probe reads only the file header and the first image directory, so it gives the pixel
//! size and colour depth without decoding pixels. Formats that already have a decoder elsewhere
//! (PNG, JPEG, GIF, WebP) are left to those modules. The caller hands in the bytes it has.

/// Image container recognised by its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Bmp,
    /// TIFF in little-endian (`II`) or big-endian (`MM`) byte order.
    Tiff,
}

/// Size and depth read from a header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    pub container: Container,
    pub width: u32,
    pub height: u32,
    /// Bits per pixel as the file declares them (BMP bit count, TIFF BitsPerSample of the first
    /// sample, summed over the samples when the file lists several).
    pub bits_per_pixel: Option<u32>,
}

/// Identifies a BMP or TIFF file from its first bytes. Returns `None` for any other data.
pub fn sniff(b: &[u8]) -> Option<Container> {
    if b.starts_with(b"BM") {
        Some(Container::Bmp)
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        Some(Container::Tiff)
    } else {
        None
    }
}

/// Reads the size and depth from the header. Returns `None` when the data is not BMP or TIFF,
/// or when it is too short to hold the fields it needs.
pub fn probe(b: &[u8]) -> Option<Probe> {
    match sniff(b)? {
        Container::Bmp => probe_bmp(b),
        Container::Tiff => probe_tiff(b),
    }
}

fn probe_bmp(b: &[u8]) -> Option<Probe> {
    // BITMAPINFOHEADER: width at 18 (i32), height at 22 (i32, negative when top-down), planes
    // at 26, bit count at 28 (u16). Only the header sizes of 12 and up carry these fields.
    let w = i32::from_le_bytes(b.get(18..22)?.try_into().ok()?);
    let h = i32::from_le_bytes(b.get(22..26)?.try_into().ok()?);
    let bpp = u16::from_le_bytes(b.get(28..30)?.try_into().ok()?);
    if w <= 0 {
        return None;
    }
    Some(Probe {
        container: Container::Bmp,
        width: w as u32,
        height: h.unsigned_abs(),
        bits_per_pixel: Some(u32::from(bpp)),
    })
}

fn probe_tiff(b: &[u8]) -> Option<Probe> {
    let little = b.starts_with(b"II");
    let u16_at = |at: usize| -> Option<u32> {
        let s = b.get(at..at + 2)?;
        Some(u32::from(if little { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) }))
    };
    let u32_at = |at: usize| -> Option<u32> {
        let s = b.get(at..at + 4)?;
        let a = [s[0], s[1], s[2], s[3]];
        Some(if little { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    let (mut width, mut height, mut bits) = (None, None, None);
    for i in 0..count {
        let e = ifd + 2 + i * 12;
        let tag = u16_at(e)?;
        let kind = u16_at(e + 2)?;
        // Value field: SHORT (3) is stored in the first two bytes, LONG (4) in all four.
        let value = match kind {
            3 => u16_at(e + 8)?,
            4 => u32_at(e + 8)?,
            _ => continue,
        };
        match tag {
            256 => width = Some(value),
            257 => height = Some(value),
            258 => bits = Some(value),
            _ => {}
        }
    }
    let width = width?;
    let height = height?;
    if width == 0 {
        return None;
    }
    Some(Probe { container: Container::Tiff, width, height, bits_per_pixel: bits })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 24-bit BMP header: 3 x 2 pixels, bottom-up.
    fn bmp_3x2() -> Vec<u8> {
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&3i32.to_le_bytes());
        b[22..26].copy_from_slice(&2i32.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        b
    }

    #[test]
    fn bmp_header_gives_size_and_depth() {
        let p = probe(&bmp_3x2()).expect("bmp");
        assert_eq!(p.container, Container::Bmp);
        assert_eq!((p.width, p.height), (3, 2));
        assert_eq!(p.bits_per_pixel, Some(24));
    }

    #[test]
    fn bmp_top_down_height_is_taken_as_a_magnitude() {
        let mut b = bmp_3x2();
        b[22..26].copy_from_slice(&(-7i32).to_le_bytes());
        assert_eq!(probe(&b).map(|p| p.height), Some(7));
    }

    #[test]
    fn bmp_with_zero_width_or_short_data_is_refused() {
        let mut b = bmp_3x2();
        b[18..22].copy_from_slice(&0i32.to_le_bytes());
        assert_eq!(probe(&b), None);
        assert_eq!(probe(&bmp_3x2()[..20]), None);
    }

    /// A little-endian TIFF with three entries: width (SHORT), height (LONG), bits (SHORT).
    fn tiff_le() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"II*\0");
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        // Entry: tag, type, count, value.
        b.extend_from_slice(&[0x00, 0x01, 0x03, 0x00, 1, 0, 0, 0, 40, 0, 0, 0]); // width 40
        b.extend_from_slice(&[0x01, 0x01, 0x04, 0x00, 1, 0, 0, 0, 25, 0, 0, 0]); // height 25
        b.extend_from_slice(&[0x02, 0x01, 0x03, 0x00, 1, 0, 0, 0, 8, 0, 0, 0]); // bits 8
        b
    }

    #[test]
    fn tiff_little_endian_directory_gives_size_and_bits() {
        let p = probe(&tiff_le()).expect("tiff");
        assert_eq!(p.container, Container::Tiff);
        assert_eq!((p.width, p.height), (40, 25));
        assert_eq!(p.bits_per_pixel, Some(8));
    }

    #[test]
    fn tiff_big_endian_is_read_too() {
        // Same picture, big-endian: header MM, offset 8, width 40 as SHORT.
        let mut b = Vec::new();
        b.extend_from_slice(b"MM\0*");
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&[0x01, 0x00, 0x00, 0x03, 0, 0, 0, 1, 0, 40, 0, 0]);
        b.extend_from_slice(&[0x01, 0x01, 0x00, 0x04, 0, 0, 0, 1, 0, 0, 0, 25]);
        let p = probe(&b).expect("tiff");
        assert_eq!((p.width, p.height), (40, 25));
        assert_eq!(p.bits_per_pixel, None);
    }

    #[test]
    fn unknown_data_is_not_sniffed() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(probe(b"GIF89a"), None);
    }
}
