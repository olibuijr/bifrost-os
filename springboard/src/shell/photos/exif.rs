//! A small EXIF reader for JPEG files: camera, date, exposure, pixel size
//! and GPS from the APP1 segment's TIFF structure. Everything is bounds
//! checked; malformed data yields `None` fields, never a panic.

/// What the info sheet shows about a picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exif {
    /// the raw maker note (Exif MakerNote, tag 0x927C), as the camera wrote it; the reference vendor's holds the Live Photo identifier
    pub maker_note: Option<Vec<u8>>,
    pub make: Option<String>,
    pub model: Option<String>,
    /// the lens model (Exif LensModel), as stored
    pub lens: Option<String>,
    /// "2026:10:04 12:30:00" as stored
    pub taken: Option<String>,
    /// 1..=8, the TIFF orientation tag
    pub orientation: Option<u16>,
    /// exposure time in seconds as numerator/denominator
    pub exposure: Option<(u32, u32)>,
    pub f_number: Option<f32>,
    pub iso: Option<u32>,
    pub focal_mm: Option<f32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// latitude, longitude in signed degrees
    pub gps: Option<(f64, f64)>,
}

impl Exif {
    /// True when nothing was found.
    pub fn is_empty(&self) -> bool {
        *self == Exif::default()
    }

    /// "Make Model" without repeating the make when the model starts with it.
    pub fn camera(&self) -> Option<String> {
        match (self.make.as_deref(), self.model.as_deref()) {
            (Some(a), Some(b)) if b.starts_with(a) => Some(b.to_string()),
            (Some(a), Some(b)) => Some(format!("{a} {b}")),
            (Some(a), None) => Some(a.to_string()),
            (None, Some(b)) => Some(b.to_string()),
            (None, None) => None,
        }
    }

    /// Label and value rows for the info sheet, skipping what is unknown.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some(t) = &self.taken {
            out.push(("Date", format_date(t)));
        }
        if let Some(c) = self.camera() {
            out.push(("Camera", c));
        }
        if let Some(l) = &self.lens {
            out.push(("Lens", l.clone()));
        }
        let mut shot = Vec::new();
        if let Some((n, d)) = self.exposure {
            shot.push(format_exposure(n, d));
        }
        if let Some(f) = self.f_number {
            shot.push(format!("f/{}", trim(f)));
        }
        if let Some(i) = self.iso {
            shot.push(format!("ISO {i}"));
        }
        if let Some(f) = self.focal_mm {
            shot.push(format!("{} mm", trim(f)));
        }
        if !shot.is_empty() {
            out.push(("Exposure", shot.join("  ")));
        }
        if let (Some(w), Some(h)) = (self.width, self.height) {
            out.push(("Size", format!("{w} x {h}")));
        }
        if let Some((lat, lon)) = self.gps {
            out.push(("Location", format_gps(lat, lon)));
        }
        out
    }
}

/// "1/250 s" or "2 s".
pub fn format_exposure(num: u32, den: u32) -> String {
    if den == 0 || num == 0 {
        return "0 s".into();
    }
    if num >= den {
        return format!("{} s", trim(num as f32 / den as f32));
    }
    // 1/x when the reciprocal is close to a whole number
    let recip = den as f32 / num as f32;
    format!("1/{} s", recip.round() as u32)
}

/// A float without trailing zeros: 2.80 -> "2.8", 4.0 -> "4".
fn trim(v: f32) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").unwrap_or(&s).to_string()
}

/// "2026:10:04 12:30:00" -> "4 October 2026, 12:30".
pub fn format_date(s: &str) -> String {
    let parse = || -> Option<String> {
        let (d, t) = s.split_once(' ')?;
        let mut dp = d.split(':');
        let (y, m, day): (i32, usize, u32) = (dp.next()?.parse().ok()?, dp.next()?.parse().ok()?, dp.next()?.parse().ok()?);
        let name = super::library::MONTHS.get(m.checked_sub(1)?)?;
        let hm: Vec<&str> = t.split(':').collect();
        Some(format!("{day} {name} {y}, {}:{}", hm.first()?, hm.get(1)?))
    };
    parse().unwrap_or_else(|| s.to_string())
}

/// "64.1466 N, 21.9426 W".
pub fn format_gps(lat: f64, lon: f64) -> String {
    format!("{:.4} {}, {:.4} {}", lat.abs(), if lat < 0.0 { 'S' } else { 'N' }, lon.abs(), if lon < 0.0 { 'W' } else { 'E' })
}

/// Reads the EXIF data of a JPEG file (the first bytes of it suffice: the
/// APP1 segment sits before the image data). Falls back to the SOF size
/// for the dimensions when EXIF lacks them.
pub fn parse(bytes: &[u8]) -> Option<Exif> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut out = Exif::default();
    let mut pos = 2;
    let mut sof = None;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            break;
        }
        let marker = bytes[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if len < 2 {
            break;
        }
        let body = bytes.get(pos + 4..(pos + 2 + len).min(bytes.len()))?;
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            read_tiff(&body[6..], &mut out);
        }
        // SOF0..SOF15 except DHT(C4), JPG(C8), DAC(CC)
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) && body.len() >= 5 {
            let h = u16::from_be_bytes([body[1], body[2]]) as u32;
            let w = u16::from_be_bytes([body[3], body[4]]) as u32;
            sof = Some((w, h));
        }
        pos += 2 + len;
    }
    if out.width.is_none() || out.height.is_none() {
        if let Some((w, h)) = sof {
            out.width = Some(w);
            out.height = Some(h);
        }
    }
    Some(out)
}

/// The eight bytes that open every PNG file.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Reads the EXIF data of a PNG from its `eXIf` chunk, which holds the TIFF data without the JPEG `Exif` header.
/// The size comes from the `IHDR` chunk when the EXIF data has no dimensions. A chunk cut short by the end of the
/// bytes ends the walk and keeps what was read before it.
pub fn parse_png(bytes: &[u8]) -> Option<Exif> {
    if bytes.get(..8)? != PNG_SIGNATURE {
        return None;
    }
    let mut out = Exif::default();
    let mut size = None;
    let mut pos = 8;
    while let Some(head) = bytes.get(pos..pos + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let Some(data) = bytes.get(pos + 8..pos + 8 + len) else { break };
        match &head[4..8] {
            b"IHDR" => size = png_size(data),
            b"eXIf" => read_tiff(data, &mut out),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    if let Some((w, h)) = size {
        out.width = out.width.or(Some(w));
        out.height = out.height.or(Some(h));
    }
    Some(out)
}

/// The width and height held in the first eight bytes of an `IHDR` chunk, both big-endian.
fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    let w = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?);
    let h = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?);
    Some((w, h))
}

/// Reads the EXIF data of a JPEG or a PNG, chosen by the file signature. Other bytes give `None`.
pub fn parse_image(bytes: &[u8]) -> Option<Exif> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        parse_png(bytes)
    } else {
        parse(bytes)
    }
}

/// The capture time ("2026:10:04 12:30:00", the camera's local wall clock) as
/// Unix seconds in a zone `offset` seconds east of UTC.
pub fn taken_unix(taken: &str, offset: i32) -> Option<i64> {
    let (d, t) = taken.trim().split_once(' ')?;
    let mut dp = d.split(':');
    let (y, m, day): (i32, u32, u32) = (dp.next()?.parse().ok()?, dp.next()?.parse().ok()?, dp.next()?.parse().ok()?);
    let mut tp = t.split(':');
    let (h, mi, s): (i64, i64, i64) = (tp.next()?.parse().ok()?, tp.next()?.parse().ok()?, tp.next().and_then(|s| s.parse().ok()).unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || mi > 59 || s > 60 || y < 1971 {
        return None;
    }
    Some(super::albums::days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + s - offset as i64)
}

/// The EXIF orientation of a JPEG (1 when absent or not a JPEG).
pub fn orientation(bytes: &[u8]) -> u16 {
    parse_image(bytes).and_then(|e| e.orientation).unwrap_or(1)
}

/// The info sheet's rows for a file: its name, the EXIF details found in
/// `head` (the start of the file) or the modification time when there is no
/// EXIF date, and the file size.
pub fn describe(name: &str, size: u64, mtime: i64, offset: i32, head: &[u8]) -> Vec<(String, String)> {
    let mut rows = vec![("Name".to_string(), name.to_string())];
    let details = parse_image(head).unwrap_or_default().rows();
    if details.iter().all(|(l, _)| *l != "Date") {
        let d = super::library::civil(mtime, offset);
        rows.push(("Date".to_string(), format!("{}, {}", super::library::day_title(&d), super::library::time_title(&d))));
    }
    rows.extend(details.into_iter().map(|(l, v)| (l.to_string(), v)));
    rows.push(("File Size".to_string(), crate::net::share::human_size(size)));
    rows
}

#[derive(Clone, Copy)]
struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    /// where the value bytes start
    at: usize,
}

impl<'a> Tiff<'a> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    fn entries(&self, ifd: usize) -> Vec<Entry> {
        let Some(n) = self.u16(ifd) else { return vec![] };
        let mut out = Vec::new();
        for i in 0..n as usize {
            let o = ifd + 2 + i * 12;
            let (Some(tag), Some(typ), Some(count)) = (self.u16(o), self.u16(o + 2), self.u32(o + 4)) else { break };
            let size = match typ {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let total = (count as usize).saturating_mul(size);
            let at = if total <= 4 { o + 8 } else { match self.u32(o + 8) { Some(v) => v as usize, None => continue } };
            if at.checked_add(total).is_none_or(|e| e > self.d.len()) {
                continue;
            }
            out.push(Entry { tag, typ, count, at });
        }
        out
    }

    /// The raw bytes of an entry of a one-byte type (undefined, as a maker note is).
    fn bytes(&self, e: Entry) -> Option<Vec<u8>> {
        Some(self.d.get(e.at..e.at.checked_add(e.count as usize)?)?.to_vec())
    }

    fn text(&self, e: Entry) -> Option<String> {
        if e.typ != 2 {
            return None;
        }
        let b = self.d.get(e.at..e.at + e.count as usize)?;
        let s = String::from_utf8_lossy(b);
        let s = s.trim_end_matches('\0').trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    /// First value of a SHORT or LONG entry.
    fn uint(&self, e: Entry) -> Option<u32> {
        match e.typ {
            3 => self.u16(e.at).map(u32::from),
            4 => self.u32(e.at),
            _ => None,
        }
    }

    /// The `i`th RATIONAL as (numerator, denominator).
    fn rational(&self, e: Entry, i: usize) -> Option<(u32, u32)> {
        if e.typ != 5 || i >= e.count as usize {
            return None;
        }
        let o = e.at + i * 8;
        Some((self.u32(o)?, self.u32(o + 4)?))
    }

    fn real(&self, e: Entry, i: usize) -> Option<f64> {
        let (n, d) = self.rational(e, i)?;
        (d != 0).then(|| n as f64 / d as f64)
    }
}

fn read_tiff(d: &[u8], out: &mut Exif) {
    let le = match d.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    let t = Tiff { d, le };
    if t.u16(2) != Some(42) {
        return;
    }
    let Some(ifd0) = t.u32(4) else { return };
    let (mut exif_ifd, mut gps_ifd) = (None, None);
    for e in t.entries(ifd0 as usize) {
        match e.tag {
            0x010F => out.make = t.text(e),
            0x0110 => out.model = t.text(e),
            0x0112 => out.orientation = t.uint(e).filter(|o| (1..=8).contains(o)).map(|o| o as u16),
            0x0132 => out.taken = out.taken.take().or_else(|| t.text(e)),
            0x8769 => exif_ifd = t.uint(e),
            0x8825 => gps_ifd = t.uint(e),
            _ => {}
        }
    }
    if let Some(ifd) = exif_ifd {
        for e in t.entries(ifd as usize) {
            match e.tag {
                0x829A => out.exposure = t.rational(e, 0).filter(|&(_, d)| d != 0),
                0x829D => out.f_number = t.real(e, 0).map(|v| v as f32),
                0x8827 => out.iso = t.uint(e),
                0x9003 => out.taken = t.text(e),
                0xA434 => out.lens = t.text(e),
                0x920A => out.focal_mm = t.real(e, 0).map(|v| v as f32),
                0xA002 => out.width = t.uint(e),
                0xA003 => out.height = t.uint(e),
                0x927C => out.maker_note = t.bytes(e),
                _ => {}
            }
        }
    }
    if let Some(ifd) = gps_ifd {
        out.gps = read_gps(&t, ifd as usize);
    }
}

fn read_gps(t: &Tiff, ifd: usize) -> Option<(f64, f64)> {
    let (mut lat_ref, mut lat, mut lon_ref, mut lon) = (None, None, None, None);
    for e in t.entries(ifd) {
        let dms = |e: Entry| Some(t.real(e, 0)? + t.real(e, 1)? / 60.0 + t.real(e, 2)? / 3600.0);
        match e.tag {
            1 => lat_ref = t.text(e),
            2 => lat = dms(e),
            3 => lon_ref = t.text(e),
            4 => lon = dms(e),
            _ => {}
        }
    }
    let (lat, lon) = (lat?, lon?);
    let lat = if lat_ref.as_deref() == Some("S") { -lat } else { lat };
    let lon = if lon_ref.as_deref() == Some("W") { -lon } else { lon };
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One IFD entry for the fixture builder.
    pub struct E {
        tag: u16,
        typ: u16,
        count: u32,
        data: Vec<u8>,
    }

    fn u16b(le: bool, v: u16) -> [u8; 2] {
        if le { v.to_le_bytes() } else { v.to_be_bytes() }
    }

    fn u32b(le: bool, v: u32) -> [u8; 4] {
        if le { v.to_le_bytes() } else { v.to_be_bytes() }
    }

    pub fn ascii(tag: u16, s: &str) -> E {
        let mut data = s.as_bytes().to_vec();
        data.push(0);
        E { tag, typ: 2, count: data.len() as u32, data }
    }

    pub fn short(le: bool, tag: u16, v: u16) -> E {
        E { tag, typ: 3, count: 1, data: u16b(le, v).to_vec() }
    }

    pub fn long(le: bool, tag: u16, v: u32) -> E {
        E { tag, typ: 4, count: 1, data: u32b(le, v).to_vec() }
    }

    pub fn rationals(le: bool, tag: u16, v: &[(u32, u32)]) -> E {
        let data = v.iter().flat_map(|&(n, d)| u32b(le, n).into_iter().chain(u32b(le, d))).collect();
        E { tag, typ: 5, count: v.len() as u32, data }
    }

    fn ifd_len(n: usize) -> usize {
        2 + 12 * n + 4
    }

    /// A TIFF block with IFD0 plus optional Exif and GPS sub-IFDs.
    pub fn tiff(le: bool, mut ifd0: Vec<E>, exif: Vec<E>, gps: Vec<E>) -> Vec<u8> {
        let n0 = ifd0.len() + usize::from(!exif.is_empty()) + usize::from(!gps.is_empty());
        let exif_off = 8 + ifd_len(n0);
        let gps_off = exif_off + if exif.is_empty() { 0 } else { ifd_len(exif.len()) };
        let data_off = gps_off + if gps.is_empty() { 0 } else { ifd_len(gps.len()) };
        if !exif.is_empty() {
            ifd0.push(long(le, 0x8769, exif_off as u32));
        }
        if !gps.is_empty() {
            ifd0.push(long(le, 0x8825, gps_off as u32));
        }
        let mut out = Vec::new();
        out.extend(if le { *b"II" } else { *b"MM" });
        out.extend(u16b(le, 42));
        out.extend(u32b(le, 8));
        let mut data: Vec<u8> = Vec::new();
        for ifd in [ifd0, exif, gps] {
            if ifd.is_empty() {
                continue;
            }
            let mut ifd = ifd;
            ifd.sort_by_key(|e| e.tag);
            out.extend(u16b(le, ifd.len() as u16));
            for e in &ifd {
                out.extend(u16b(le, e.tag));
                out.extend(u16b(le, e.typ));
                out.extend(u32b(le, e.count));
                if e.data.len() <= 4 {
                    let mut v = e.data.clone();
                    v.resize(4, 0);
                    out.extend(v);
                } else {
                    out.extend(u32b(le, (data_off + data.len()) as u32));
                    data.extend(&e.data);
                }
            }
            out.extend(u32b(le, 0));
        }
        assert_eq!(out.len(), data_off, "layout");
        out.extend(data);
        out
    }

    /// A JPEG skeleton: SOI, an APP1 with `tiff`, a SOF0 of `w` x `h`, EOI.
    pub fn jpeg(tiff: &[u8], w: u16, h: u16) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        if !tiff.is_empty() {
            out.extend([0xFF, 0xE1]);
            out.extend(((tiff.len() + 8) as u16).to_be_bytes());
            out.extend(b"Exif\0\0");
            out.extend(tiff);
        }
        out.extend([0xFF, 0xC0, 0, 11, 8]);
        out.extend(h.to_be_bytes());
        out.extend(w.to_be_bytes());
        out.extend([1, 1, 0x11, 0]);
        out.extend([0xFF, 0xD9]);
        out
    }

    fn full(le: bool) -> Vec<u8> {
        let ifd0 = vec![ascii(0x010F, "Fjord"), ascii(0x0110, "Fjord F1"), short(le, 0x0112, 6)];
        let exif = vec![
            rationals(le, 0x829A, &[(1, 250)]),
            rationals(le, 0x829D, &[(28, 10)]),
            short(le, 0x8827, 100),
            ascii(0x9003, "2026:10:04 12:30:00"),
            rationals(le, 0x920A, &[(4, 1)]),
            long(le, 0xA002, 4000),
            short(le, 0xA003, 3000),
        ];
        let gps = vec![
            ascii(1, "N"),
            rationals(le, 2, &[(64, 1), (8, 1), (48, 1)]),
            ascii(3, "W"),
            rationals(le, 4, &[(21, 1), (56, 1), (0, 1)]),
        ];
        jpeg(&tiff(le, ifd0, exif, gps), 640, 480)
    }

    #[test]
    fn reads_every_field_in_both_byte_orders() {
        for le in [true, false] {
            let e = parse(&full(le)).unwrap();
            assert_eq!(e.make.as_deref(), Some("Fjord"));
            assert_eq!(e.camera().as_deref(), Some("Fjord F1"), "make is not repeated");
            assert_eq!(e.orientation, Some(6));
            assert_eq!(e.exposure, Some((1, 250)));
            assert_eq!(e.f_number, Some(2.8));
            assert_eq!(e.iso, Some(100));
            assert_eq!(e.focal_mm, Some(4.0));
            assert_eq!(e.taken.as_deref(), Some("2026:10:04 12:30:00"));
            assert_eq!((e.width, e.height), (Some(4000), Some(3000)), "EXIF wins over the SOF size");
            let (lat, lon) = e.gps.unwrap();
            assert!((lat - (64.0 + 8.0 / 60.0 + 48.0 / 3600.0)).abs() < 1e-9);
            assert!((lon + (21.0 + 56.0 / 60.0)).abs() < 1e-9, "west is negative");
        }
    }

    #[test]
    fn rows_are_formatted_for_the_sheet() {
        let rows = parse(&full(true)).unwrap().rows();
        let get = |k: &str| rows.iter().find(|(l, _)| *l == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Date"), Some("4 October 2026, 12:30"));
        assert_eq!(get("Camera"), Some("Fjord F1"));
        assert_eq!(get("Exposure"), Some("1/250 s  f/2.8  ISO 100  4 mm"));
        assert_eq!(get("Size"), Some("4000 x 3000"));
        assert_eq!(get("Location"), Some("64.1467 N, 21.9333 W"));
        assert_eq!(format_exposure(2, 1), "2 s");
        assert_eq!(format_exposure(1, 3), "1/3 s");
        assert_eq!(format_exposure(0, 5), "0 s");
        assert_eq!(format_date("garbage"), "garbage");
        assert_eq!(format_gps(-33.5, 151.25), "33.5000 S, 151.2500 E");
    }

    #[test]
    fn falls_back_to_the_sof_size_and_tolerates_missing_exif() {
        let e = parse(&jpeg(&[], 640, 480)).unwrap();
        assert_eq!((e.width, e.height), (Some(640), Some(480)));
        assert!(e.gps.is_none() && e.camera().is_none());
        assert_eq!(e.rows(), vec![("Size", "640 x 480".to_string())]);
        assert!(Exif::default().is_empty() && Exif::default().rows().is_empty());
        assert!(!e.is_empty());
    }

    #[test]
    fn describe_adds_name_size_and_a_fallback_date() {
        let rows = describe("a.jpg", 1_500_000, 1_791_073_395, 0, &full(true));
        let labels: Vec<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Name", "Date", "Camera", "Exposure", "Size", "Location", "File Size"]);
        assert_eq!(rows[1].1, "4 October 2026, 12:30", "the EXIF date wins");
        assert_eq!(rows.last().unwrap().1, "1.4 MB");
        // a PNG has no EXIF: the file's time is shown
        let png = describe("s.png", 900, 1_791_073_395, 0, b"\x89PNG....");
        assert_eq!(png, vec![("Name".to_string(), "s.png".to_string()), ("Date".to_string(), "4 October 2026, 00:23".to_string()), ("File Size".to_string(), "900 B".to_string())]);
    }

    #[test]
    fn the_lens_model_is_read_and_listed_after_the_camera() {
        // LensModel (0xA434) lives in the Exif sub-IFD; the ASCII value is NUL terminated
        let t = tiff(true, vec![ascii(0x0110, "Canon")], vec![ascii(0xA434, "EF24mm f/1.4G")], vec![]);
        let e = parse(&jpeg(&t, 1, 1)).unwrap();
        assert_eq!(e.lens.as_deref(), Some("EF24mm f/1.4G"));
        assert_eq!(e.rows(), vec![("Camera", "Canon".to_string()), ("Lens", "EF24mm f/1.4G".to_string()), ("Size", "1 x 1".to_string())]);
    }

    #[test]
    fn dates_without_exif_date_use_ifd0_datetime() {
        let t = tiff(true, vec![ascii(0x0132, "2020:01:02 03:04:05")], vec![], vec![]);
        let e = parse(&jpeg(&t, 1, 1)).unwrap();
        assert_eq!(e.taken.as_deref(), Some("2020:01:02 03:04:05"));
        assert_eq!(e.rows()[0], ("Date", "2 January 2020, 03:04".to_string()));
    }

    #[test]
    fn rejects_non_jpeg_and_survives_corruption() {
        assert!(parse(b"\x89PNG").is_none());
        assert!(parse(&[]).is_none());
        assert!(parse(&[0xFF]).is_none());
        let good = full(true);
        // every truncation point and a bad TIFF header must not panic
        for n in 0..good.len() {
            let _ = parse(&good[..n]);
        }
        let mut bad = good.clone();
        bad[12] = b'X'; // TIFF byte-order mark broken: no EXIF, SOF still read
        let e = parse(&bad).unwrap();
        assert_eq!(e.make, None);
        let mut bad = good.clone();
        for b in bad.iter_mut().skip(20) {
            *b = 0xFF; // wild offsets and counts
        }
        let _ = parse(&bad);
        // an entry pointing outside the block is skipped
        let mut t = tiff(true, vec![ascii(0x010F, "Fjord")], vec![], vec![]);
        let at = t.len();
        t[8 + 2 + 8..8 + 2 + 12].copy_from_slice(&((at + 100) as u32).to_le_bytes());
        assert_eq!(parse(&jpeg(&t, 1, 1)).unwrap().make, None);
    }

    #[test]
    fn out_of_range_gps_and_orientation_are_dropped() {
        let t = tiff(true, vec![short(true, 0x0112, 9)], vec![], vec![ascii(1, "N"), rationals(true, 2, &[(95, 1), (0, 1), (0, 1)]), ascii(3, "E"), rationals(true, 4, &[(10, 1), (0, 1), (0, 1)])]);
        let e = parse(&jpeg(&t, 1, 1)).unwrap();
        assert_eq!((e.orientation, e.gps), (None, None));
    }
}
