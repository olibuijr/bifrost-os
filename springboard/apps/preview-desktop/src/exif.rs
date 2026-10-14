//! The GPS position stored in a JPEG's Exif block, for Show Location Info. Only the GPS latitude and longitude tags are
//! read; a file without them, or with a damaged block, has no position.

/// A position in decimal degrees: north and east are positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gps {
    pub lat: f64,
    pub lon: f64,
}

impl Gps {
    /// The position as text, such as `64.1466 N, 21.9426 W`.
    pub fn label(&self) -> String {
        format!(
            "{:.4} {}, {:.4} {}",
            self.lat.abs(),
            if self.lat >= 0.0 { "N" } else { "S" },
            self.lon.abs(),
            if self.lon >= 0.0 { "E" } else { "W" }
        )
    }
}

const TAG_GPS_IFD: u16 = 0x8825;
const TYPE_ASCII: u16 = 2;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;

/// A reader over the TIFF block with the file's byte order.
struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The entries of the directory at `at`: tag, type, count and the position of the 4 byte value field.
    fn entries(&self, at: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(at) else { return Vec::new() };
        (0..n as usize)
            .filter_map(|i| {
                let e = at + 2 + i * 12;
                Some((self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?, e + 8))
            })
            .collect()
    }

    /// Three rationals (degrees, minutes, seconds) at the offset in the value field `field`, as decimal degrees.
    fn degrees(&self, field: usize) -> Option<f64> {
        let at = self.u32(field)? as usize;
        let mut parts = [0.0f64; 3];
        for (i, part) in parts.iter_mut().enumerate() {
            let num = self.u32(at + i * 8)? as f64;
            let den = self.u32(at + i * 8 + 4)? as f64;
            if den == 0.0 {
                return None;
            }
            *part = num / den;
        }
        Some(parts[0] + parts[1] / 60.0 + parts[2] / 3600.0)
    }
}

/// The TIFF block of a JPEG's Exif segment.
fn exif_block(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut at = 2;
    while at + 4 <= jpeg.len() {
        if jpeg[at] != 0xff {
            return None;
        }
        let marker = jpeg[at + 1];
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        let body = jpeg.get(at + 4..at + 2 + len)?;
        if marker == 0xe1 && body.starts_with(b"Exif\0\0") {
            return Some(&body[6..]);
        }
        at += 2 + len;
    }
    None
}

/// The GPS position in the Exif block of `jpeg`, or None.
pub fn gps(jpeg: &[u8]) -> Option<Gps> {
    let data = exif_block(jpeg)?;
    let little = match data.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let tiff = Tiff { data, little };
    if tiff.u16(2)? != 42 {
        return None;
    }
    let ifd0 = tiff.u32(4)? as usize;
    let (_, kind, _, field) = tiff.entries(ifd0).into_iter().find(|e| e.0 == TAG_GPS_IFD)?;
    if kind != TYPE_LONG {
        return None;
    }
    let gps_ifd = tiff.u32(field)? as usize;
    let mut lat = (None, 'N');
    let mut lon = (None, 'E');
    for (tag, kind, _, field) in tiff.entries(gps_ifd) {
        match (tag, kind) {
            (1, TYPE_ASCII) => lat.1 = *tiff.data.get(field)? as char,
            (2, TYPE_RATIONAL) => lat.0 = tiff.degrees(field),
            (3, TYPE_ASCII) => lon.1 = *tiff.data.get(field)? as char,
            (4, TYPE_RATIONAL) => lon.0 = tiff.degrees(field),
            _ => {}
        }
    }
    let lat_deg = lat.0? * if lat.1 == 'S' { -1.0 } else { 1.0 };
    let lon_deg = lon.0? * if lon.1 == 'W' { -1.0 } else { 1.0 };
    Some(Gps { lat: lat_deg, lon: lon_deg })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(out: &mut Vec<u8>, v: u16) {
        out.extend(v.to_be_bytes());
    }

    fn put32(out: &mut Vec<u8>, v: u32) {
        out.extend(v.to_be_bytes());
    }

    fn entry(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]) {
        put16(out, tag);
        put16(out, kind);
        put32(out, count);
        out.extend(value);
    }

    fn rationals(out: &mut Vec<u8>, parts: [(u32, u32); 3]) {
        for (n, d) in parts {
            put32(out, n);
            put32(out, d);
        }
    }

    /// A big endian JPEG with only an Exif segment holding the GPS tags.
    fn jpeg(lat_ref: u8, lon_ref: u8, with_gps: bool) -> Vec<u8> {
        let mut t = b"MM\0*".to_vec();
        put32(&mut t, 8);
        // IFD0: one entry, the GPS directory at offset 26
        put16(&mut t, if with_gps { 1 } else { 0 });
        if with_gps {
            entry(&mut t, TAG_GPS_IFD, TYPE_LONG, 1, 26u32.to_be_bytes());
        }
        put32(&mut t, 0);
        if with_gps {
            put16(&mut t, 4);
            entry(&mut t, 1, TYPE_ASCII, 2, [lat_ref, 0, 0, 0]);
            entry(&mut t, 2, TYPE_RATIONAL, 3, 80u32.to_be_bytes());
            entry(&mut t, 3, TYPE_ASCII, 2, [lon_ref, 0, 0, 0]);
            entry(&mut t, 4, TYPE_RATIONAL, 3, 104u32.to_be_bytes());
            put32(&mut t, 0);
            assert_eq!(t.len(), 80);
            rationals(&mut t, [(64, 1), (8, 1), (4560, 100)]);
            rationals(&mut t, [(21, 1), (56, 1), (0, 1)]);
        }
        let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
        put16(&mut out, (2 + 6 + t.len()) as u16);
        out.extend(b"Exif\0\0");
        out.extend(t);
        out.extend([0xff, 0xd9]);
        out
    }

    #[test]
    fn the_position_is_read_with_its_hemispheres() {
        let g = gps(&jpeg(b'N', b'W', true)).expect("a position");
        assert!((g.lat - (64.0 + 8.0 / 60.0 + 45.6 / 3600.0)).abs() < 1e-9, "{}", g.lat);
        assert!((g.lon + (21.0 + 56.0 / 60.0)).abs() < 1e-9, "{}", g.lon);
        let south = gps(&jpeg(b'S', b'E', true)).expect("a position");
        assert!(south.lat < 0.0 && south.lon > 0.0);
    }

    #[test]
    fn a_file_without_gps_or_exif_has_no_position() {
        assert_eq!(gps(&jpeg(b'N', b'E', false)), None);
        assert_eq!(gps(b"not a jpeg"), None);
        assert_eq!(gps(&[0xff, 0xd8, 0xff, 0xd9]), None);
        let mut cut = jpeg(b'N', b'E', true);
        cut.truncate(40);
        assert_eq!(gps(&cut), None);
    }

    #[test]
    fn the_label_names_the_hemispheres() {
        assert_eq!(Gps { lat: 64.1466, lon: -21.9426 }.label(), "64.1466 N, 21.9426 W");
        assert_eq!(Gps { lat: -33.5, lon: 151.25 }.label(), "33.5000 S, 151.2500 E");
    }
}
