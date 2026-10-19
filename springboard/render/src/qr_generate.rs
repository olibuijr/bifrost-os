//! A QR code generator, the counterpart of `CIQRCodeGenerator`: text or bytes in, a module matrix or a
//! picture out. Model 2 codes, versions 1 to 10 (up to 271 bytes at level L), all four error-correction
//! levels, numeric, alphanumeric and byte segments (one segment per code, chosen from the data), Reed-Solomon
//! check bytes and the mask with the lowest penalty score. The decoder for the camera scan overlay lives in
//! the shell crate (`shell/camera/qr.rs`); this module shares its tables by value, not by import, because
//! the shell is not a dependency of the render crate.
//! Not built: versions 11 to 40, mixed segments, ECI and Kanji. Rounded modules
//! (`CIRoundedQRCodeGenerator`) are drawn by `QrCode::to_image` with `rounded`.
use crate::gfx::Image;

/// Error-correction level: how much of the code may be damaged and still read (about 7, 15, 25 and 30 percent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    L,
    M,
    Q,
    H,
}

impl Correction {
    fn idx(self) -> usize {
        self as usize
    }

    fn format_bits(self) -> u32 {
        match self {
            Correction::L => 1,
            Correction::M => 0,
            Correction::Q => 3,
            Correction::H => 2,
        }
    }

    /// The level named by the reference `inputCorrectionLevel` letters `L`, `M`, `Q` and `H`.
    pub fn from_letter(letter: char) -> Option<Correction> {
        match letter.to_ascii_uppercase() {
            'L' => Some(Correction::L),
            'M' => Some(Correction::M),
            'Q' => Some(Correction::Q),
            'H' => Some(Correction::H),
            _ => None,
        }
    }
}

/// The largest version this generator builds.
pub const MAX_VERSION: usize = 10;
const TOTAL_CW: [usize; MAX_VERSION] = [26, 44, 70, 100, 134, 172, 196, 242, 292, 346];
/// (check codewords per block, number of blocks) per version, levels L M Q H.
const EC_TABLE: [[(usize, usize); 4]; MAX_VERSION] = [
    [(7, 1), (10, 1), (13, 1), (17, 1)],
    [(10, 1), (16, 1), (22, 1), (28, 1)],
    [(15, 1), (26, 1), (18, 2), (22, 2)],
    [(20, 1), (18, 2), (26, 2), (16, 4)],
    [(26, 1), (24, 2), (18, 4), (22, 4)],
    [(18, 2), (16, 4), (24, 4), (28, 4)],
    [(20, 2), (18, 4), (18, 6), (26, 5)],
    [(24, 2), (22, 4), (22, 6), (26, 6)],
    [(30, 2), (22, 5), (20, 8), (24, 8)],
    [(18, 4), (26, 5), (24, 8), (28, 8)],
];
const ALIGN: [&[usize]; MAX_VERSION] =
    [&[], &[6, 18], &[6, 22], &[6, 26], &[6, 30], &[6, 34], &[6, 22, 38], &[6, 24, 42], &[6, 26, 46], &[6, 28, 50]];
const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

fn dim_of(version: usize) -> usize {
    17 + 4 * version
}

// ---------------------------------------------------------------- GF(256) and Reed-Solomon

fn tables() -> &'static ([u8; 512], [u8; 256]) {
    static T: std::sync::OnceLock<([u8; 512], [u8; 256])> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x = 1u32;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        (exp, log)
    })
}

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let (exp, log) = tables();
    exp[log[a as usize] as usize + log[b as usize] as usize]
}

/// The generator polynomial of `degree` check bytes, without its leading 1 coefficient.
fn generator(degree: usize) -> Vec<u8> {
    let mut r = vec![0u8; degree];
    r[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            r[j] = mul(r[j], root);
            if j + 1 < degree {
                r[j] ^= r[j + 1];
            }
        }
        root = mul(root, 2);
    }
    r
}

/// The `ec` Reed-Solomon check bytes of `data`.
fn rs_encode(data: &[u8], ec: usize) -> Vec<u8> {
    let g = generator(ec);
    let mut rem = vec![0u8; ec];
    for &b in data {
        let f = b ^ rem.remove(0);
        rem.push(0);
        for (r, &gi) in rem.iter_mut().zip(&g) {
            *r ^= mul(gi, f);
        }
    }
    rem
}

// ---------------------------------------------------------------- layout

fn function_map(version: usize) -> Vec<bool> {
    let size = dim_of(version);
    let mut f = vec![false; size * size];
    let mut rect = |x0: usize, y0: usize, w: usize, h: usize| {
        for y in y0..(y0 + h).min(size) {
            for x in x0..(x0 + w).min(size) {
                f[y * size + x] = true;
            }
        }
    };
    rect(0, 0, 9, 9);
    rect(size - 8, 0, 8, 9);
    rect(0, size - 8, 9, 8);
    rect(6, 0, 1, size);
    rect(0, 6, size, 1);
    let centres = ALIGN[version - 1];
    let last = centres.len().saturating_sub(1);
    for (i, &cy) in centres.iter().enumerate() {
        for (j, &cx) in centres.iter().enumerate() {
            if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                continue;
            }
            rect(cx - 2, cy - 2, 5, 5);
        }
    }
    if version >= 7 {
        rect(size - 11, 0, 3, 6);
        rect(0, size - 11, 6, 3);
    }
    f
}

/// Module coordinates `(x, y)` of the data bits in placement order.
fn data_positions(version: usize) -> Vec<(usize, usize)> {
    let size = dim_of(version);
    let func = function_map(version);
    let mut out = Vec::with_capacity(size * size);
    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as usize;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !func[y * size + x] {
                    out.push((x, y));
                }
            }
        }
        right -= 2;
    }
    out
}

fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    let (x, y) = (x as u32, y as u32);
    match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (x / 3 + y / 2) % 2 == 0,
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3) % 2 == 0,
        _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
    }
}

/// The 15 format bits for a 5-bit (level, mask) value, BCH-protected and XOR-masked.
fn format_code(data: u32) -> u32 {
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

/// The format bit positions of the two copies, bit 0 first.
fn format_positions(size: usize) -> [[(usize, usize); 15]; 2] {
    let mut a = [(0, 0); 15];
    let mut b = [(0, 0); 15];
    for i in 0..15 {
        a[i] = match i {
            0..=5 => (8, i),
            6 => (8, 7),
            7 => (8, 8),
            8 => (7, 8),
            _ => (14 - i, 8),
        };
        b[i] = if i < 8 { (size - 1 - i, 8) } else { (8, size - 15 + i) };
    }
    [a, b]
}

// ---------------------------------------------------------------- data

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Numeric,
    Alnum,
    Byte,
}

fn mode_for(data: &[u8]) -> Mode {
    if !data.is_empty() && data.iter().all(u8::is_ascii_digit) {
        Mode::Numeric
    } else if !data.is_empty() && data.iter().all(|b| ALNUM.contains(b)) {
        Mode::Alnum
    } else {
        Mode::Byte
    }
}

fn count_bits(mode: Mode, version: usize) -> usize {
    let wide = version >= 10;
    match mode {
        Mode::Numeric => if wide { 12 } else { 10 },
        Mode::Alnum => if wide { 11 } else { 9 },
        Mode::Byte => if wide { 16 } else { 8 },
    }
}

fn body_bits(mode: Mode, len: usize) -> usize {
    match mode {
        Mode::Byte => len * 8,
        Mode::Numeric => len / 3 * 10 + [0, 4, 7][len % 3],
        Mode::Alnum => len / 2 * 11 + len % 2 * 6,
    }
}

fn data_capacity_bits(version: usize, level: Correction) -> usize {
    let (ec, blocks) = EC_TABLE[version - 1][level.idx()];
    (TOTAL_CW[version - 1] - ec * blocks) * 8
}

struct BitBuf(Vec<bool>);

impl BitBuf {
    fn put(&mut self, v: u32, n: usize) {
        for i in (0..n).rev() {
            self.0.push(v >> i & 1 == 1);
        }
    }
}

/// The data codewords of `data` for `version`: mode and count header, the segment, the terminator and the pad bytes.
fn data_codewords(data: &[u8], mode: Mode, version: usize, level: Correction) -> Vec<u8> {
    let mut buf = BitBuf(Vec::new());
    buf.put(match mode { Mode::Numeric => 1, Mode::Alnum => 2, Mode::Byte => 4 }, 4);
    buf.put(data.len() as u32, count_bits(mode, version));
    match mode {
        Mode::Byte => data.iter().for_each(|&b| buf.put(b as u32, 8)),
        Mode::Numeric => {
            for chunk in data.chunks(3) {
                let v = chunk.iter().fold(0u32, |a, &d| a * 10 + (d - b'0') as u32);
                buf.put(v, [0, 4, 7, 10][chunk.len()]);
            }
        }
        Mode::Alnum => {
            let idx = |b: u8| ALNUM.iter().position(|&a| a == b).unwrap_or(0) as u32;
            for chunk in data.chunks(2) {
                if chunk.len() == 2 {
                    buf.put(idx(chunk[0]) * 45 + idx(chunk[1]), 11);
                } else {
                    buf.put(idx(chunk[0]), 6);
                }
            }
        }
    }
    let cap = data_capacity_bits(version, level);
    let term = cap.saturating_sub(buf.0.len()).min(4);
    buf.put(0, term);
    while buf.0.len() % 8 != 0 {
        buf.0.push(false);
    }
    let mut out: Vec<u8> = buf.0.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| a << 1 | b as u8)).collect();
    let mut pad = 0xEC;
    while out.len() < cap / 8 {
        out.push(pad);
        pad ^= 0xEC ^ 0x11;
    }
    out
}

/// Splits the data codewords into blocks, appends each block's check bytes and interleaves them.
fn interleave(data: &[u8], version: usize, level: Correction) -> Vec<u8> {
    let (ec, blocks) = EC_TABLE[version - 1][level.idx()];
    let data_total = TOTAL_CW[version - 1] - ec * blocks;
    let (base, extra) = (data_total / blocks, data_total % blocks);
    let lens: Vec<usize> = (0..blocks).map(|i| base + usize::from(i >= blocks - extra)).collect();
    let mut at = 0;
    let mut dblocks: Vec<Vec<u8>> = Vec::new();
    let mut eblocks: Vec<Vec<u8>> = Vec::new();
    for &l in &lens {
        let d = data[at..at + l].to_vec();
        at += l;
        eblocks.push(rs_encode(&d, ec));
        dblocks.push(d);
    }
    let mut all = Vec::with_capacity(TOTAL_CW[version - 1]);
    for k in 0..lens.iter().copied().max().unwrap_or(0) {
        for d in &dblocks {
            if k < d.len() {
                all.push(d[k]);
            }
        }
    }
    for k in 0..ec {
        for e in &eblocks {
            all.push(e[k]);
        }
    }
    all
}

/// The module matrix (row-major, true is dark) of `all` codewords under `mask`.
fn build(version: usize, level: Correction, all: &[u8], mask: u8) -> Vec<bool> {
    let size = dim_of(version);
    let mut m = vec![false; size * size];
    fn set(m: &mut [bool], size: usize, x: i32, y: i32, dark: bool) {
        if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
            m[y as usize * size + x as usize] = dark;
        }
    }
    let s = size as i32;
    for i in 0..s {
        set(&mut m, size, 6, i, i % 2 == 0);
        set(&mut m, size, i, 6, i % 2 == 0);
    }
    for (cx, cy) in [(3, 3), (s - 4, 3), (3, s - 4)] {
        for dy in -4..=4i32 {
            for dx in -4..=4i32 {
                let d = dx.abs().max(dy.abs());
                set(&mut m, size, cx + dx, cy + dy, d != 2 && d != 4);
            }
        }
    }
    let centres = ALIGN[version - 1];
    let last = centres.len().saturating_sub(1);
    for (i, &cy) in centres.iter().enumerate() {
        for (j, &cx) in centres.iter().enumerate() {
            if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                continue;
            }
            for dy in -2..=2i32 {
                for dx in -2..=2i32 {
                    set(&mut m, size, cx as i32 + dx, cy as i32 + dy, dx.abs().max(dy.abs()) != 1);
                }
            }
        }
    }
    if version >= 7 {
        let mut rem = version as u32;
        for _ in 0..12 {
            rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
        }
        let bits = (version as u32) << 12 | rem;
        for i in 0..18i32 {
            let bit = bits >> i & 1 == 1;
            let (a, b) = (s - 11 + i % 3, i / 3);
            set(&mut m, size, a, b, bit);
            set(&mut m, size, b, a, bit);
        }
    }
    set(&mut m, size, 8, s - 8, true);
    for (i, &(x, y)) in data_positions(version).iter().enumerate() {
        let bit = i < all.len() * 8 && all[i / 8] >> (7 - i % 8) & 1 == 1;
        m[y * size + x] = bit ^ mask_bit(mask, x, y);
    }
    let fmt = format_code(level.format_bits() << 3 | mask as u32);
    for copy in format_positions(size) {
        for (i, &(x, y)) in copy.iter().enumerate() {
            m[y * size + x] = fmt >> i & 1 == 1;
        }
    }
    m
}

/// The mask penalty of the specification: runs, 2x2 blocks, finder-like patterns and dark balance.
fn penalty(m: &[bool], size: usize) -> u32 {
    let mut p = 0u32;
    const A: [bool; 11] = [true, false, true, true, true, false, true, false, false, false, false];
    for line in 0..size {
        for dir in 0..2 {
            let at = |i: usize| if dir == 0 { m[line * size + i] } else { m[i * size + line] };
            let mut run = 1u32;
            for i in 1..size {
                if at(i) == at(i - 1) {
                    run += 1;
                } else {
                    if run >= 5 {
                        p += 3 + run - 5;
                    }
                    run = 1;
                }
            }
            if run >= 5 {
                p += 3 + run - 5;
            }
            for start in 0..=size - 11 {
                let forward = (0..11).all(|k| at(start + k) == A[k]);
                let backward = (0..11).all(|k| at(start + k) == A[10 - k]);
                if forward {
                    p += 40;
                }
                if backward {
                    p += 40;
                }
            }
        }
    }
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let v = m[y * size + x];
            if v == m[y * size + x + 1] && v == m[(y + 1) * size + x] && v == m[(y + 1) * size + x + 1] {
                p += 3;
            }
        }
    }
    let dark = m.iter().filter(|&&d| d).count() as u32;
    let total = (size * size) as u32;
    let off = (dark * 100 / total).abs_diff(50);
    p + off / 5 * 10
}

/// A generated code: its version, level, mask and module matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct QrCode {
    version: usize,
    level: Correction,
    mask: u8,
    size: usize,
    modules: Vec<bool>,
}

impl QrCode {
    /// Encodes `text` (its UTF-8 bytes) at `level`. Digits only use numeric mode, the alphanumeric set
    /// uses alphanumeric mode, everything else byte mode. `None` when it does not fit version 10 or is empty.
    pub fn encode(text: &str, level: Correction) -> Option<QrCode> {
        let data = text.as_bytes();
        QrCode::build_with_mode(data, mode_for(data), level)
    }

    /// Encodes raw bytes in byte mode, the way `CIQRCodeGenerator` takes its `inputMessage` data. `None`
    /// when the bytes are empty or do not fit version 10 (271 bytes at level L).
    pub fn encode_bytes(data: &[u8], level: Correction) -> Option<QrCode> {
        QrCode::build_with_mode(data, Mode::Byte, level)
    }

    fn build_with_mode(data: &[u8], mode: Mode, level: Correction) -> Option<QrCode> {
        if data.is_empty() {
            return None;
        }
        let version = (1..=MAX_VERSION)
            .find(|&v| 4 + count_bits(mode, v) + body_bits(mode, data.len()) <= data_capacity_bits(v, level))?;
        let cw = data_codewords(data, mode, version, level);
        let all = interleave(&cw, version, level);
        let size = dim_of(version);
        let mut best: Option<(u32, u8, Vec<bool>)> = None;
        for mask in 0..8u8 {
            let m = build(version, level, &all, mask);
            let score = penalty(&m, size);
            if best.as_ref().is_none_or(|(s, _, _)| score < *s) {
                best = Some((score, mask, m));
            }
        }
        let (_, mask, modules) = best?;
        Some(QrCode { version, level, mask, size, modules })
    }

    /// The version, 1 to 10.
    pub fn version(&self) -> usize {
        self.version
    }

    /// The error-correction level.
    pub fn level(&self) -> Correction {
        self.level
    }

    /// The mask pattern chosen, 0 to 7.
    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// Modules per side (21 for version 1, 4 more per version).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at column `x`, row `y` is dark. Out of range is light.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    /// The code as a picture: `scale` pixels per module, `quiet` modules of light border (the specification asks
    /// for 4), dark modules in `dark` and the rest in `light`, both packed like `gfx::Image` pixels. With `rounded`,
    /// each dark module is a disc inscribed in its cell instead of a square, as `CIRoundedQRCodeGenerator` draws
    /// them; the three finder squares stay square so scanners keep their anchor. `None` when `scale` is 0.
    pub fn to_image(&self, scale: usize, quiet: usize, dark: u32, light: u32, rounded: bool) -> Option<Image> {
        if scale == 0 {
            return None;
        }
        let side = (self.size + 2 * quiet) * scale;
        let mut img = Image::new(side, side);
        for p in img.px.iter_mut() {
            *p = light;
        }
        let in_finder = |x: usize, y: usize| {
            let n = self.size;
            (x < 8 && y < 8) || (x >= n - 8 && y < 8) || (x < 8 && y >= n - 8)
        };
        let r = scale as f32 / 2.0;
        for my in 0..self.size {
            for mx in 0..self.size {
                if !self.is_dark(mx, my) {
                    continue;
                }
                let round = rounded && !in_finder(mx, my) && scale >= 3;
                for yy in 0..scale {
                    for xx in 0..scale {
                        if round {
                            let (dx, dy) = (xx as f32 + 0.5 - r, yy as f32 + 0.5 - r);
                            if dx * dx + dy * dy > r * r {
                                continue;
                            }
                        }
                        img.px[((my + quiet) * scale + yy) * side + (mx + quiet) * scale + xx] = dark;
                    }
                }
            }
        }
        Some(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reed_solomon_matches_the_published_hello_world_vector() {
        // Version 1, level M, "HELLO WORLD" in alphanumeric mode (the worked example in the specification's tutorials).
        let cw = data_codewords(b"HELLO WORLD", Mode::Alnum, 1, Correction::M);
        assert_eq!(cw, [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17]);
        assert_eq!(rs_encode(&cw, 10), [196, 35, 39, 119, 235, 215, 231, 226, 93, 23]);
    }

    #[test]
    fn data_module_counts_match_the_specification() {
        assert_eq!(data_positions(1).len(), 208);
        assert_eq!(data_positions(2).len(), 44 * 8 + 7);
        assert_eq!(data_positions(7).len(), 196 * 8);
    }

    #[test]
    fn the_smallest_fitting_version_is_chosen() {
        let q = QrCode::encode_bytes(&[b'a'; 17], Correction::L).unwrap();
        assert_eq!((q.version(), q.size()), (1, 21));
        let q = QrCode::encode_bytes(&[b'a'; 18], Correction::L).unwrap();
        assert_eq!((q.version(), q.size()), (2, 25));
        assert!(QrCode::encode_bytes(&[b'a'; 400], Correction::L).is_none());
        assert!(QrCode::encode_bytes(&[], Correction::L).is_none());
    }

    #[test]
    fn compact_modes_fit_more_text() {
        // 41 digits fit version 1-L in numeric mode but not in byte mode.
        let digits = "01234567890123456789012345678901234567890";
        assert_eq!(QrCode::encode(digits, Correction::L).unwrap().version(), 1);
        assert!(QrCode::encode_bytes(digits.as_bytes(), Correction::L).unwrap().version() > 1);
        assert_eq!(mode_for(b"HELLO WORLD"), Mode::Alnum);
        assert_eq!(mode_for(b"hello"), Mode::Byte);
        assert_eq!(mode_for(b"123"), Mode::Numeric);
    }

    #[test]
    fn finder_patterns_timing_and_the_dark_module_are_in_place() {
        let q = QrCode::encode("https://example.org", Correction::M).unwrap();
        let n = q.size();
        for (ox, oy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
            for dy in 0..7 {
                for dx in 0..7 {
                    let ring = dx == 0 || dx == 6 || dy == 0 || dy == 6;
                    let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
                    assert_eq!(q.is_dark(ox + dx, oy + dy), ring || core, "finder at {ox},{oy} cell {dx},{dy}");
                }
            }
        }
        for i in 8..n - 8 {
            assert_eq!(q.is_dark(i, 6), i % 2 == 0);
            assert_eq!(q.is_dark(6, i), i % 2 == 0);
        }
        assert!(q.is_dark(8, n - 8));
    }

    #[test]
    fn both_format_copies_carry_the_same_valid_level_and_mask() {
        for level in [Correction::L, Correction::M, Correction::Q, Correction::H] {
            let q = QrCode::encode("BIFROST", level).unwrap();
            let mut read = Vec::new();
            for copy in format_positions(q.size()) {
                read.push(copy.iter().enumerate().fold(0u32, |acc, (i, &(x, y))| acc | (q.is_dark(x, y) as u32) << i));
            }
            assert_eq!(read[0], read[1]);
            assert_eq!(read[0], format_code(level.format_bits() << 3 | q.mask() as u32));
        }
    }

    #[test]
    fn the_chosen_mask_has_the_lowest_penalty() {
        let data = b"The quick brown fox";
        let q = QrCode::encode_bytes(data, Correction::Q).unwrap();
        let cw = data_codewords(data, Mode::Byte, q.version(), Correction::Q);
        let all = interleave(&cw, q.version(), Correction::Q);
        let chosen = penalty(&build(q.version(), Correction::Q, &all, q.mask()), q.size());
        for mask in 0..8u8 {
            assert!(chosen <= penalty(&build(q.version(), Correction::Q, &all, mask), q.size()));
        }
    }

    #[test]
    fn to_image_scales_adds_the_quiet_zone_and_rounds_the_data_modules() {
        let q = QrCode::encode("A", Correction::L).unwrap();
        let dark = 0xff00_0000;
        let light = 0xffff_ffff;
        let img = q.to_image(4, 4, dark, light, false).unwrap();
        assert_eq!((img.w, img.h), ((21 + 8) * 4, (21 + 8) * 4));
        assert_eq!(img.px[0], light);
        // The top-left finder module is dark: module (0, 0) sits at pixel (16, 16).
        assert_eq!(img.px[16 * img.w + 16], dark);
        let rounded = q.to_image(8, 0, dark, light, true).unwrap();
        let square = q.to_image(8, 0, dark, light, false).unwrap();
        let count = |i: &Image| i.px.iter().filter(|&&p| p == dark).count();
        assert!(count(&rounded) < count(&square));
        assert!(q.to_image(0, 4, dark, light, false).is_none());
    }

    #[test]
    fn level_letters_parse() {
        assert_eq!(Correction::from_letter('q'), Some(Correction::Q));
        assert_eq!(Correction::from_letter('x'), None);
    }
}
