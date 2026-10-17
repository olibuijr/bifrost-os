//! RAW container reader for TIFF-structured camera files (DNG and the TIFF-based vendor formats whose sensor strip is
//! stored uncompressed), as a pure model (std only). It walks the image file directories (the main one, `SubIFDs` and
//! the chained ones), picks the directory whose photometric interpretation is a colour filter array (32803), and
//! unpacks the strips into 16-bit sensor samples. Bit depths 1 to 16 are read: 8 and 16 bits as plain bytes and words in
//! the file's byte order, every other depth as a most-significant-bit-first packed stream with each row starting on a
//! byte boundary (the layout the DNG specification gives for uncompressed data).
//!
//! The DNG tags that describe the sensor are read from whichever directory carries them: black and white levels, the
//! as-shot neutral and the first colour matrix. Together with `bayer::demosaic` and `raw_color::apply_color_pipeline`
//! this closes the RAW path from file bytes to display RGB.
//!
//! Not read: compressed strips (lossless JPEG, packed vendor codecs), tiled layouts and non-2x2 filter patterns. They
//! are reported with a specific error instead of producing wrong pixels.

use crate::bayer::{demosaic, BayerError, CfaPattern};

/// Why a file was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum RawError {
    /// The bytes are not a little- or big-endian TIFF header.
    NotTiff,
    /// A directory, value or strip lies outside the file.
    Truncated,
    /// No directory holds a colour filter array image.
    NoRawImage,
    /// The sensor data uses this compression code; only 1 (none) is read.
    Compressed(u16),
    /// The sensor data is stored as tiles.
    Tiled,
    /// The bit depth is not in 1..=16, or the file has more than one sample per pixel.
    BadFormat,
    /// The colour filter array is not a 2x2 Bayer tile of red, green and blue.
    BadPattern,
}

/// A decoded sensor frame with its DNG metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct RawImage {
    pub width: usize,
    pub height: usize,
    pub bits: u32,
    pub pattern: CfaPattern,
    /// Sensor value of black. 0 when the file has none.
    pub black: f64,
    /// Sensor value of full scale. `2^bits - 1` when the file has none.
    pub white: f64,
    /// As-shot neutral (camera RGB of a neutral grey), when the file has it.
    pub neutral: Option<[f64; 3]>,
    /// ColorMatrix1 (XYZ to camera RGB, row-major), when the file has it.
    pub color_matrix: Option<[f64; 9]>,
    /// `width * height` raw samples, row-major.
    pub samples: Vec<u16>,
}

#[derive(Clone, Copy)]
struct Rd<'a> {
    d: &'a [u8],
    le: bool,
}

impl<'a> Rd<'a> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b = self.d.get(o..o.checked_add(2)?)?;
        Some(if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let b = self.d.get(o..o.checked_add(4)?)?;
        Some(if self.le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    }
}

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    off: usize,
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

fn read_entry(rd: &Rd, at: usize) -> Option<Entry> {
    let tag = rd.u16(at)?;
    let typ = rd.u16(at + 2)?;
    let count = rd.u32(at + 4)?;
    let total = type_size(typ).checked_mul(count as usize)?;
    let off = if total <= 4 { at + 8 } else { rd.u32(at + 8)? as usize };
    Some(Entry { tag, typ, count, off })
}

/// The entries of the directory at `off` and the offset of the next directory (0 for none).
fn read_ifd(rd: &Rd, off: usize) -> Option<(Vec<Entry>, usize)> {
    let n = rd.u16(off)? as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(read_entry(rd, off + 2 + 12 * i)?);
    }
    let next = rd.u32(off + 2 + 12 * n)? as usize;
    Some((out, next))
}

fn ints(rd: &Rd, e: &Entry) -> Option<Vec<u32>> {
    if e.count > (1 << 20) {
        return None;
    }
    let mut v = Vec::with_capacity(e.count as usize);
    for i in 0..e.count as usize {
        let x = match e.typ {
            1 | 7 => u32::from(*rd.d.get(e.off + i)?),
            3 => u32::from(rd.u16(e.off + 2 * i)?),
            4 | 13 => rd.u32(e.off + 4 * i)?,
            _ => return None,
        };
        v.push(x);
    }
    Some(v)
}

fn floats(rd: &Rd, e: &Entry) -> Option<Vec<f64>> {
    match e.typ {
        1 | 3 | 4 => Some(ints(rd, e)?.into_iter().map(f64::from).collect()),
        5 | 10 => {
            if e.count > (1 << 16) {
                return None;
            }
            let mut v = Vec::new();
            for i in 0..e.count as usize {
                let n = rd.u32(e.off + 8 * i)?;
                let d = rd.u32(e.off + 8 * i + 4)?;
                let (n, d) = if e.typ == 10 { (f64::from(n as i32), f64::from(d as i32)) } else { (f64::from(n), f64::from(d)) };
                v.push(if d == 0.0 { 0.0 } else { n / d });
            }
            Some(v)
        }
        11 => {
            if e.count > (1 << 16) {
                return None;
            }
            let mut v = Vec::new();
            for i in 0..e.count as usize {
                v.push(f64::from(f32::from_bits(rd.u32(e.off + 4 * i)?)));
            }
            Some(v)
        }
        _ => None,
    }
}

fn get(es: &[Entry], tag: u16) -> Option<&Entry> {
    es.iter().find(|e| e.tag == tag)
}

fn first_int(rd: &Rd, es: &[Entry], tag: u16) -> Option<u32> {
    ints(rd, get(es, tag)?)?.first().copied()
}

fn t<T>(o: Option<T>) -> Result<T, RawError> {
    o.ok_or(RawError::Truncated)
}

/// Unpacks `height` rows of `width` samples of `bits` bits from `bytes`, rows byte-aligned.
fn unpack(bytes: &[u8], width: usize, height: usize, bits: u32, le: bool) -> Option<Vec<u16>> {
    let row_bytes = (width * bits as usize).div_ceil(8);
    if bytes.len() < row_bytes.checked_mul(height)? {
        return None;
    }
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = &bytes[y * row_bytes..(y + 1) * row_bytes];
        match bits {
            8 => out.extend(row.iter().take(width).map(|&b| u16::from(b))),
            16 => {
                for x in 0..width {
                    let b = [row[2 * x], row[2 * x + 1]];
                    out.push(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) });
                }
            }
            _ => {
                let mut bit = 0usize;
                for _ in 0..width {
                    let mut v = 0u32;
                    for _ in 0..bits {
                        let byte = row[bit >> 3];
                        v = (v << 1) | u32::from((byte >> (7 - (bit & 7))) & 1);
                        bit += 1;
                    }
                    out.push(v as u16);
                }
            }
        }
    }
    Some(out)
}

/// Parses a TIFF-structured RAW file.
pub fn parse(data: &[u8]) -> Result<RawImage, RawError> {
    let le = match data.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(RawError::NotTiff),
    };
    let rd = Rd { d: data, le };
    if rd.u16(2) != Some(42) {
        return Err(RawError::NotTiff);
    }
    let first = t(rd.u32(4))? as usize;

    let mut ifds: Vec<Vec<Entry>> = Vec::new();
    let mut queue: Vec<usize> = vec![first];
    let mut i = 0;
    while i < queue.len() && ifds.len() < 16 {
        let off = queue[i];
        i += 1;
        let Some((es, next)) = read_ifd(&rd, off) else {
            if ifds.is_empty() {
                return Err(RawError::Truncated);
            }
            continue;
        };
        if let Some(sub) = get(&es, 330).and_then(|e| ints(&rd, e)) {
            for s in sub {
                if !queue.contains(&(s as usize)) {
                    queue.push(s as usize);
                }
            }
        }
        if next != 0 && !queue.contains(&next) {
            queue.push(next);
        }
        ifds.push(es);
    }

    let raw = ifds
        .iter()
        .position(|es| first_int(&rd, es, 262) == Some(32803))
        .ok_or(RawError::NoRawImage)?;
    let es = &ifds[raw];

    if get(es, 322).is_some() || get(es, 324).is_some() {
        return Err(RawError::Tiled);
    }
    let width = t(first_int(&rd, es, 256))? as usize;
    let height = t(first_int(&rd, es, 257))? as usize;
    let bits = first_int(&rd, es, 258).unwrap_or(16);
    let spp = first_int(&rd, es, 277).unwrap_or(1);
    let compression = first_int(&rd, es, 259).unwrap_or(1) as u16;
    if compression != 1 {
        return Err(RawError::Compressed(compression));
    }
    if !(1..=16).contains(&bits) || spp != 1 || width == 0 || height == 0 || width > 1 << 16 || height > 1 << 16 {
        return Err(RawError::BadFormat);
    }

    let pattern = {
        if let Some(dim) = get(es, 33421).and_then(|e| ints(&rd, e)) {
            if dim != [2, 2] {
                return Err(RawError::BadPattern);
            }
        }
        let cfa = t(get(es, 33422))?;
        let p = t(ints(&rd, cfa))?;
        match p.as_slice() {
            [0, 1, 1, 2] => CfaPattern::Rggb,
            [2, 1, 1, 0] => CfaPattern::Bggr,
            [1, 0, 2, 1] => CfaPattern::Grbg,
            [1, 2, 0, 1] => CfaPattern::Gbrg,
            _ => return Err(RawError::BadPattern),
        }
    };

    // Strips are concatenated; every strip is a whole number of byte-aligned rows.
    let row_bytes = (width * bits as usize).div_ceil(8);
    let offsets = t(ints(&rd, t(get(es, 273))?))?;
    let counts = match get(es, 279).and_then(|e| ints(&rd, e)) {
        Some(c) if c.len() == offsets.len() => c,
        _ if offsets.len() == 1 => vec![(row_bytes * height) as u32],
        _ => return Err(RawError::Truncated),
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(row_bytes * height);
    for (o, c) in offsets.iter().zip(&counts) {
        let (o, c) = (*o as usize, *c as usize);
        let end = t(o.checked_add(c))?;
        bytes.extend_from_slice(t(data.get(o..end))?);
    }
    let samples = t(unpack(&bytes, width, height, bits, le))?;

    let find = |tag: u16| ifds.iter().find_map(|es| get(es, tag));
    let black = find(50714).and_then(|e| floats(&rd, e)).and_then(|v| v.first().copied()).unwrap_or(0.0);
    let white = find(50717)
        .and_then(|e| floats(&rd, e))
        .and_then(|v| v.first().copied())
        .unwrap_or(f64::from((1u32 << bits) - 1));
    let neutral = find(50728).and_then(|e| floats(&rd, e)).and_then(|v| if v.len() == 3 { Some([v[0], v[1], v[2]]) } else { None });
    let color_matrix = find(50721).and_then(|e| floats(&rd, e)).and_then(|v| {
        if v.len() == 9 {
            let mut m = [0.0; 9];
            m.copy_from_slice(&v);
            Some(m)
        } else {
            None
        }
    });

    Ok(RawImage { width, height, bits, pattern, black, white, neutral, color_matrix, samples })
}

impl RawImage {
    /// The mosaic rescaled so black maps to 0 and white to 65535, clamped. A degenerate range (white at or below
    /// black) leaves the samples unscaled.
    pub fn normalized_mosaic(&self) -> Vec<u16> {
        let span = self.white - self.black;
        self.samples
            .iter()
            .map(|&s| {
                if span <= 0.0 {
                    s
                } else {
                    (((f64::from(s) - self.black) / span) * 65535.0).round().clamp(0.0, 65535.0) as u16
                }
            })
            .collect()
    }

    /// White balance gains (red, green, blue) from the as-shot neutral, green fixed at 1. All 1 without a neutral.
    pub fn white_balance_gains(&self) -> [f64; 3] {
        match self.neutral {
            Some(n) if n.iter().all(|v| *v > 0.0) => [n[1] / n[0], 1.0, n[1] / n[2]],
            _ => [1.0, 1.0, 1.0],
        }
    }

    /// Black-corrected, white-balanced, demosaiced interleaved 16-bit RGB in camera primaries and linear light.
    pub fn to_rgb16(&self) -> Result<Vec<u16>, BayerError> {
        demosaic(&self.normalized_mosaic(), self.width, self.height, self.pattern, self.white_balance_gains())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tag: u16, typ: u16, count: u32, val: [u8; 4]) -> Vec<u8> {
        let mut e = Vec::new();
        e.extend_from_slice(&tag.to_le_bytes());
        e.extend_from_slice(&typ.to_le_bytes());
        e.extend_from_slice(&count.to_le_bytes());
        e.extend_from_slice(&val);
        e
    }

    fn short(v: u16) -> [u8; 4] {
        let b = v.to_le_bytes();
        [b[0], b[1], 0, 0]
    }

    /// A little-endian DNG-like file with one CFA directory of inline values and `pixels` as the strip.
    fn file(width: u16, height: u16, bits: u16, compression: u16, pixels: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
        let n = 12 + extra.len();
        let data_off = (8 + 2 + n * 12 + 4) as u32;
        let mut es: Vec<Vec<u8>> = vec![
            entry(256, 3, 1, short(width)),
            entry(257, 3, 1, short(height)),
            entry(258, 3, 1, short(bits)),
            entry(259, 3, 1, short(compression)),
            entry(262, 3, 1, short(32803)),
            entry(273, 4, 1, data_off.to_le_bytes()),
            entry(277, 3, 1, short(1)),
            entry(278, 3, 1, short(height)),
            entry(279, 4, 1, (pixels.len() as u32).to_le_bytes()),
            entry(33421, 3, 2, [2, 0, 2, 0]),
            entry(33422, 7, 4, [0, 1, 1, 2]),
            entry(50717, 3, 1, short(4095)),
        ];
        es.extend(extra.iter().cloned());
        let mut f = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        f.extend_from_slice(&(es.len() as u16).to_le_bytes());
        for e in &es {
            f.extend_from_slice(e);
        }
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(pixels);
        f
    }

    #[test]
    fn reads_sixteen_bit_samples_and_pattern() {
        let vals: [u16; 8] = [1, 2, 3, 4, 5, 6, 7, 800];
        let px: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let img = parse(&file(4, 2, 16, 1, &px, &[])).unwrap();
        assert_eq!((img.width, img.height, img.bits), (4, 2, 16));
        assert_eq!(img.pattern, CfaPattern::Rggb);
        assert_eq!(img.samples, vals.to_vec());
        assert_eq!(img.white, 4095.0);
        assert_eq!(img.black, 0.0);
    }

    #[test]
    fn unpacks_twelve_bit_rows_most_significant_first() {
        // Row 0: 0xABC 0x123 -> AB C1 23; row 1: 0x456 0x789 -> 45 67 89.
        let px = [0xAB, 0xC1, 0x23, 0x45, 0x67, 0x89];
        let img = parse(&file(2, 2, 12, 1, &px, &[])).unwrap();
        assert_eq!(img.samples, vec![0xABC, 0x123, 0x456, 0x789]);
    }

    #[test]
    fn reads_black_level_and_neutral() {
        let px = vec![0u8; 16];
        let black = entry(50714, 3, 1, short(256));
        let img = parse(&file(4, 2, 16, 1, &px, &[black])).unwrap();
        assert_eq!(img.black, 256.0);
        assert_eq!(img.neutral, None);
        assert_eq!(img.white_balance_gains(), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn refuses_compressed_and_foreign_input() {
        let px = vec![0u8; 16];
        assert_eq!(parse(&file(4, 2, 16, 7, &px, &[])), Err(RawError::Compressed(7)));
        assert_eq!(parse(b"not a tiff at all"), Err(RawError::NotTiff));
        assert_eq!(parse(&[b'I', b'I', 42, 0, 8, 0, 0, 0]), Err(RawError::Truncated));
    }

    #[test]
    fn truncated_strip_is_reported() {
        let px = vec![0u8; 16];
        let mut f = file(4, 2, 16, 1, &px, &[]);
        f.truncate(f.len() - 4);
        assert_eq!(parse(&f), Err(RawError::Truncated));
    }

    #[test]
    fn normalizes_and_demosaics_to_rgb() {
        let img = RawImage {
            width: 2,
            height: 2,
            bits: 12,
            pattern: CfaPattern::Rggb,
            black: 100.0,
            white: 4095.0,
            neutral: None,
            color_matrix: None,
            samples: vec![100, 4095, 4095, 100],
        };
        assert_eq!(img.normalized_mosaic(), vec![0, 65535, 65535, 0]);
        let rgb = img.to_rgb16().unwrap();
        assert_eq!(rgb.len(), 12);
        // The top-left photosite is red and keeps its own (black) value.
        assert_eq!(rgb[0], 0);
    }

    #[test]
    fn neutral_gives_green_referenced_gains() {
        let mut img = RawImage {
            width: 2,
            height: 2,
            bits: 8,
            pattern: CfaPattern::Rggb,
            black: 0.0,
            white: 255.0,
            neutral: Some([0.5, 1.0, 0.25]),
            color_matrix: None,
            samples: vec![0; 4],
        };
        assert_eq!(img.white_balance_gains(), [2.0, 1.0, 4.0]);
        img.neutral = Some([0.0, 1.0, 1.0]);
        assert_eq!(img.white_balance_gains(), [1.0, 1.0, 1.0]);
    }
}
