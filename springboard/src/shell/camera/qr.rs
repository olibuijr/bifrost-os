//! A pure-Rust QR code decoder for the Camera's scan overlay: model 2 codes,
//! versions 1-10, all four error-correction levels and all eight masks, numeric,
//! alphanumeric and byte segments, Reed-Solomon error correction. Codes may be
//! rotated by any angle (no perspective correction) and need no quiet zone.
//!
//! Input is an 8-bit luminance image; see `decode_luma`. The tests build their
//! own codes with a small encoder (`encoder` module, tests only).

/// Error-correction level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ecl {
    L,
    M,
    Q,
    H,
}

impl Ecl {
    fn idx(self) -> usize {
        self as usize
    }

    #[cfg(test)]
    fn format_bits(self) -> u32 {
        match self {
            Ecl::L => 1,
            Ecl::M => 0,
            Ecl::Q => 3,
            Ecl::H => 2,
        }
    }

    fn from_format_bits(b: u32) -> Ecl {
        match b & 3 {
            1 => Ecl::L,
            0 => Ecl::M,
            3 => Ecl::Q,
            _ => Ecl::H,
        }
    }
}

const MAX_VERSION: usize = 10;
const TOTAL_CW: [usize; MAX_VERSION] = [26, 44, 70, 100, 134, 172, 196, 242, 292, 346];
/// (EC codewords per block, number of blocks) per version, levels L M Q H.
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
const ALIGN: [&[usize]; MAX_VERSION] = [&[], &[6, 18], &[6, 22], &[6, 26], &[6, 30], &[6, 34], &[6, 22, 38], &[6, 24, 42], &[6, 26, 46], &[6, 28, 50]];
const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

fn dim_of(version: usize) -> usize {
    17 + 4 * version
}

// ---------------------------------------------------------------- GF(256)

struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

fn gf() -> &'static Gf {
    static GF: std::sync::OnceLock<Gf> = std::sync::OnceLock::new();
    GF.get_or_init(|| {
        let mut g = Gf { exp: [0; 512], log: [0; 256] };
        let mut x = 1u32;
        for i in 0..255 {
            g.exp[i] = x as u8;
            g.log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            g.exp[i] = g.exp[i - 255];
        }
        g
    })
}

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let g = gf();
    g.exp[g.log[a as usize] as usize + g.log[b as usize] as usize]
}

fn inv(a: u8) -> u8 {
    let g = gf();
    g.exp[255 - g.log[a as usize] as usize]
}

/// Corrects a Reed-Solomon codeword (data then `ec` check bytes) in place.
/// Returns false when it has more errors than `ec / 2` or the repair does not verify.
fn rs_correct(cw: &mut [u8], ec: usize) -> bool {
    let g = gf();
    let n = cw.len();
    let syndromes = |cw: &[u8]| -> Vec<u8> {
        (0..ec)
            .map(|j| {
                let x = g.exp[j];
                cw.iter().fold(0u8, |acc, &c| mul(acc, x) ^ c)
            })
            .collect()
    };
    let s = syndromes(cw);
    if s.iter().all(|&v| v == 0) {
        return true;
    }
    // Berlekamp-Massey: the error locator polynomial, low degree first
    let mut c = vec![1u8];
    let mut b = vec![1u8];
    let (mut l, mut m, mut bb) = (0usize, 1usize, 1u8);
    for i in 0..ec {
        let mut d = s[i];
        for k in 1..=l.min(c.len() - 1) {
            d ^= mul(c[k], s[i - k]);
        }
        if d == 0 {
            m += 1;
            continue;
        }
        let t = c.clone();
        let coef = mul(d, inv(bb));
        if c.len() < b.len() + m {
            c.resize(b.len() + m, 0);
        }
        for (k, &bk) in b.iter().enumerate() {
            c[k + m] ^= mul(coef, bk);
        }
        if 2 * l <= i {
            l = i + 1 - l;
            b = t;
            bb = d;
            m = 1;
        } else {
            m += 1;
        }
    }
    if l == 0 || l > ec / 2 {
        return false;
    }
    c.resize(l + 1, 0);
    // Chien search: position p (from the start) has locator X = alpha^(n-1-p)
    let mut positions = Vec::new();
    for p in 0..n {
        let x_inv = g.exp[(255 - (n - 1 - p) % 255) % 255];
        let mut acc = 0u8;
        for &coef in c.iter().rev() {
            acc = mul(acc, x_inv) ^ coef;
        }
        if acc == 0 {
            positions.push(p);
        }
    }
    if positions.len() != l {
        return false;
    }
    // error evaluator: omega = S(x) * locator(x) mod x^ec
    let mut omega = vec![0u8; ec];
    for (i, o) in omega.iter_mut().enumerate() {
        for (k, &ck) in c.iter().enumerate() {
            if k <= i {
                *o ^= mul(ck, s[i - k]);
            }
        }
    }
    for &p in &positions {
        let x = g.exp[(n - 1 - p) % 255];
        let x_inv = inv(x);
        let mut num = 0u8;
        for &o in omega.iter().rev() {
            num = mul(num, x_inv) ^ o;
        }
        // formal derivative of the locator: odd-degree terms only
        let mut den = 0u8;
        let mut xp = 1u8; // x_inv^(k-1)
        for (k, &ck) in c.iter().enumerate().skip(1) {
            if k % 2 == 1 {
                den ^= mul(ck, xp);
            }
            xp = mul(xp, x_inv);
        }
        if den == 0 {
            return false;
        }
        cw[p] ^= mul(x, mul(num, inv(den)));
    }
    syndromes(cw).iter().all(|&v| v == 0)
}

// ---------------------------------------------------------------- layout

/// `true` for every function-pattern module (file_managers, timing, alignment, format, version).
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
    for (i, &cy) in centres.iter().enumerate() {
        for (j, &cx) in centres.iter().enumerate() {
            let last = centres.len() - 1;
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

/// Module coordinates `(x, y)` of the data bits in reading order.
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

fn read_format(m: &[bool], size: usize) -> Option<(Ecl, u8)> {
    let mut best: Option<(u32, u32)> = None;
    for copy in format_positions(size) {
        let bits = copy.iter().enumerate().fold(0u32, |acc, (i, &(x, y))| acc | (m[y * size + x] as u32) << i);
        for data in 0..32u32 {
            let d = (format_code(data) ^ bits).count_ones();
            if d <= 3 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, data));
            }
        }
    }
    let (_, data) = best?;
    Some((Ecl::from_format_bits(data >> 3), (data & 7) as u8))
}

// ---------------------------------------------------------------- data

fn split_blocks(version: usize, ecl: Ecl) -> (usize, Vec<usize>) {
    let (ec, blocks) = EC_TABLE[version - 1][ecl.idx()];
    let data_total = TOTAL_CW[version - 1] - ec * blocks;
    let (base, extra) = (data_total / blocks, data_total % blocks);
    (ec, (0..blocks).map(|i| base + usize::from(i >= blocks - extra)).collect())
}

/// Undoes the block interleaving, repairs each block and returns the data codewords.
fn recover_data(cw: &[u8], version: usize, ecl: Ecl) -> Option<Vec<u8>> {
    let (ec, lens) = split_blocks(version, ecl);
    let data_total: usize = lens.iter().sum();
    let max_len = *lens.iter().max()?;
    let mut data: Vec<Vec<u8>> = lens.iter().map(|&l| Vec::with_capacity(l + ec)).collect();
    let mut at = 0;
    for k in 0..max_len {
        for (b, block) in data.iter_mut().enumerate() {
            if k < lens[b] {
                block.push(*cw.get(at)?);
                at += 1;
            }
        }
    }
    for _ in 0..ec {
        for block in data.iter_mut() {
            block.push(*cw.get(at)?);
            at += 1;
        }
    }
    let mut out = Vec::with_capacity(data_total);
    for (b, mut block) in data.into_iter().enumerate() {
        if !rs_correct(&mut block, ec) {
            return None;
        }
        out.extend_from_slice(&block[..lens[b]]);
    }
    Some(out)
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: usize) -> Option<u32> {
        if self.pos + n > self.data.len() * 8 {
            return None;
        }
        let mut v = 0;
        for _ in 0..n {
            v = v << 1 | ((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }
}

/// Reads the numeric, alphanumeric and byte segments of the data codewords.
fn parse_segments(data: &[u8], version: usize) -> Option<String> {
    let mut r = Bits { data, pos: 0 };
    let mut out: Vec<u8> = Vec::new();
    let wide = version >= 10;
    while let Some(mode) = r.read(4) {
        match mode {
            0 => break,
            1 => {
                let mut n = r.read(if wide { 12 } else { 10 })? as usize;
                while n > 0 {
                    let (take, bits) = match n {
                        1 => (1, 4),
                        2 => (2, 7),
                        _ => (3, 10),
                    };
                    let v = r.read(bits)?;
                    if v >= 10u32.pow(take as u32) {
                        return None;
                    }
                    out.extend(format!("{v:0width$}", width = take).bytes());
                    n -= take;
                }
            }
            2 => {
                let mut n = r.read(if wide { 11 } else { 9 })? as usize;
                while n > 0 {
                    if n >= 2 {
                        let v = r.read(11)? as usize;
                        if v >= 45 * 45 {
                            return None;
                        }
                        out.push(ALNUM[v / 45]);
                        out.push(ALNUM[v % 45]);
                        n -= 2;
                    } else {
                        let v = r.read(6)? as usize;
                        out.push(*ALNUM.get(v)?);
                        n -= 1;
                    }
                }
            }
            4 => {
                let n = r.read(if wide { 16 } else { 8 })?;
                for _ in 0..n {
                    out.push(r.read(8)? as u8);
                }
            }
            7 => {
                // ECI designator: 1, 2 or 3 bytes; the charset is ignored (UTF-8 / Latin-1 below)
                let first = r.read(8)?;
                if first & 0x80 != 0 {
                    r.read(if first & 0x40 != 0 { 16 } else { 8 })?;
                }
            }
            _ => return None,
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(String::from_utf8(out.clone()).unwrap_or_else(|_| out.iter().map(|&b| b as char).collect()))
}

/// Decodes a sampled module matrix (`true` = dark), `size` x `size`, row-major.
fn decode_matrix(m: &[bool], size: usize) -> Option<String> {
    if size < 21 || (size - 17) % 4 != 0 || m.len() < size * size {
        return None;
    }
    let version = (size - 17) / 4;
    if version > MAX_VERSION {
        return None;
    }
    let (ecl, mask) = read_format(m, size)?;
    let mut bytes = vec![0u8; TOTAL_CW[version - 1]];
    for (i, &(x, y)) in data_positions(version).iter().enumerate().take(bytes.len() * 8) {
        let bit = m[y * size + x] ^ mask_bit(mask, x, y);
        bytes[i / 8] |= (bit as u8) << (7 - i % 8);
    }
    let data = recover_data(&bytes, version, ecl)?;
    parse_segments(&data, version)
}

// ---------------------------------------------------------------- image

struct Bin {
    w: usize,
    h: usize,
    dark: Vec<bool>,
}

impl Bin {
    fn at(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.dark[y as usize * self.w + x as usize]
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }
}

fn otsu(luma: &[u8], w: usize, h: usize, invert: bool) -> Bin {
    let mut hist = [0u32; 256];
    for &p in &luma[..w * h] {
        hist[p as usize] += 1;
    }
    let total = (w * h) as f64;
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut sum_b, mut w_b, mut best, mut thr) = (0.0, 0.0, -1.0, 128u8);
    for (t, &c) in hist.iter().enumerate() {
        w_b += c as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * c as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum_all - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            thr = t as u8;
        }
    }
    Bin { w, h, dark: luma[..w * h].iter().map(|&p| (p <= thr) != invert).collect() }
}

/// Local-mean threshold (integral image) for uneven lighting.
fn adaptive(luma: &[u8], w: usize, h: usize) -> Bin {
    let mut integral = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u64;
        for x in 0..w {
            row += luma[y * w + x] as u64;
            integral[(y + 1) * (w + 1) + x + 1] = integral[y * (w + 1) + x + 1] + row;
        }
    }
    let r = (w.max(h) / 16).max(8);
    let mut dark = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = integral[y1 * (w + 1) + x1] + integral[y0 * (w + 1) + x0] - integral[y0 * (w + 1) + x1] - integral[y1 * (w + 1) + x0];
            let mean = sum as f64 / ((x1 - x0) * (y1 - y0)) as f64;
            dark[y * w + x] = (luma[y * w + x] as f64) < mean * 0.9 - 3.0;
        }
    }
    Bin { w, h, dark }
}

/// True when five run lengths are in the 1:1:3:1:1 file_manager ratio.
fn file_manager_ratio(c: [f32; 5]) -> bool {
    let total: f32 = c.iter().sum();
    if total < 7.0 {
        return false;
    }
    let ms = total / 7.0;
    let tol = ms * 0.6;
    [c[0], c[1], c[3], c[4]].iter().all(|&v| (v - ms).abs() < tol) && (c[2] - 3.0 * ms).abs() < 3.0 * tol
}

/// Runs through (x, y) along (dx, dy) (a unit step); returns the centre coordinate
/// along the axis and the pattern's total length when it is a file_manager cross-section.
fn check_line(bin: &Bin, x: i32, y: i32, dx: i32, dy: i32) -> Option<(f32, f32)> {
    if !bin.at(x, y) {
        return None;
    }
    let mut c = [0f32; 5];
    let (mut px, mut py) = (x, y);
    let mut neg_centre = 0;
    while bin.inside(px, py) && bin.at(px, py) {
        neg_centre += 1;
        px -= dx;
        py -= dy;
    }
    if !bin.inside(px, py) {
        return None;
    }
    while bin.inside(px, py) && !bin.at(px, py) {
        c[1] += 1.0;
        px -= dx;
        py -= dy;
    }
    if !bin.inside(px, py) {
        return None;
    }
    while bin.inside(px, py) && bin.at(px, py) {
        c[0] += 1.0;
        px -= dx;
        py -= dy;
    }
    let (mut px, mut py) = (x + dx, y + dy);
    let mut pos_centre = 0;
    while bin.inside(px, py) && bin.at(px, py) {
        pos_centre += 1;
        px += dx;
        py += dy;
    }
    if !bin.inside(px, py) {
        return None;
    }
    while bin.inside(px, py) && !bin.at(px, py) {
        c[3] += 1.0;
        px += dx;
        py += dy;
    }
    if !bin.inside(px, py) {
        return None;
    }
    while bin.inside(px, py) && bin.at(px, py) {
        c[4] += 1.0;
        px += dx;
        py += dy;
    }
    c[2] = (neg_centre + pos_centre) as f32;
    if !file_manager_ratio(c) {
        return None;
    }
    let axis = if dx != 0 { x } else { y };
    let start = axis - (neg_centre - 1);
    Some((start as f32 + (c[2] - 1.0) / 2.0, c.iter().sum()))
}

#[derive(Clone, Copy, Debug)]
struct FileManager {
    x: f32,
    y: f32,
    ms: f32,
    count: u32,
}

fn find_finders(bin: &Bin) -> Vec<FileManager> {
    let mut found: Vec<FileManager> = Vec::new();
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    for y in 0..bin.h {
        runs.clear();
        let mut start = 0;
        for x in 1..=bin.w {
            if x == bin.w || bin.dark[y * bin.w + x] != bin.dark[y * bin.w + start] {
                runs.push((start, x - start, bin.dark[y * bin.w + start]));
                start = x;
            }
        }
        for i in 0..runs.len().saturating_sub(4) {
            if !runs[i].2 {
                continue;
            }
            let c = [runs[i].1 as f32, runs[i + 1].1 as f32, runs[i + 2].1 as f32, runs[i + 3].1 as f32, runs[i + 4].1 as f32];
            if !file_manager_ratio(c) {
                continue;
            }
            let xc = runs[i + 2].0 as f32 + (runs[i + 2].1 as f32 - 1.0) / 2.0;
            let Some((yc, tv)) = check_line(bin, xc.round() as i32, y as i32, 0, 1) else { continue };
            let Some((xc2, th)) = check_line(bin, xc.round() as i32, yc.round() as i32, 1, 0) else { continue };
            if (tv - th).abs() > 0.4 * tv.max(th) {
                continue;
            }
            let ms = (tv + th) / 14.0;
            match found.iter_mut().find(|f| (f.x - xc2).abs() < ms * 2.0 && (f.y - yc).abs() < ms * 2.0 && (f.ms - ms).abs() < ms * 0.5) {
                Some(f) => {
                    let n = f.count as f32;
                    f.x = (f.x * n + xc2) / (n + 1.0);
                    f.y = (f.y * n + yc) / (n + 1.0);
                    f.ms = (f.ms * n + ms) / (n + 1.0);
                    f.count += 1;
                }
                None => found.push(FileManager { x: xc2, y: yc, ms, count: 1 }),
            }
        }
    }
    found.retain(|f| f.count >= 2);
    found.sort_by(|a, b| b.count.cmp(&a.count));
    found.truncate(8);
    found
}

fn dist(a: &FileManager, b: &FileManager) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Orders three file_managers as (top-left, top-right, bottom-left) for a code of any rotation.
fn order(a: FileManager, b: FileManager, c: FileManager) -> Option<(FileManager, FileManager, FileManager)> {
    let (ab, ac, bc) = (dist(&a, &b), dist(&a, &c), dist(&b, &c));
    // the corner file_manager is opposite the longest side
    let (tl, p, q) = if bc >= ab && bc >= ac {
        (a, b, c)
    } else if ac >= ab && ac >= bc {
        (b, a, c)
    } else {
        (c, a, b)
    };
    let (l1, l2) = (dist(&tl, &p), dist(&tl, &q));
    if l1 < 3.0 * tl.ms || l2 < 3.0 * tl.ms || l1.min(l2) / l1.max(l2) < 0.7 {
        return None;
    }
    let cross = (p.x - tl.x) * (q.y - tl.y) - (p.y - tl.y) * (q.x - tl.x);
    if cross.abs() < 0.5 * l1 * l2 * 0.5 {
        return None;
    }
    Some(if cross > 0.0 { (tl, p, q) } else { (tl, q, p) })
}

fn sample(bin: &Bin, tl: &FileManager, tr: &FileManager, bl: &FileManager, size: usize) -> Vec<bool> {
    let span = (size - 7) as f32;
    let (ux, uy) = ((tr.x - tl.x) / span, (tr.y - tl.y) / span);
    let (vx, vy) = ((bl.x - tl.x) / span, (bl.y - tl.y) / span);
    let mut m = vec![false; size * size];
    for j in 0..size {
        for i in 0..size {
            let (di, dj) = (i as f32 - 3.0, j as f32 - 3.0);
            let (cx, cy) = (tl.x + di * ux + dj * vx, tl.y + di * uy + dj * vy);
            let mut votes = 0;
            for (a, b) in [(0.0, 0.0), (0.3, 0.0), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3)] {
                let (sx, sy) = (cx + a * ux + b * vx, cy + a * uy + b * vy);
                if bin.at(sx.round() as i32, sy.round() as i32) {
                    votes += 1;
                }
            }
            m[j * size + i] = votes >= 3;
        }
    }
    m
}

fn decode_bin(bin: &Bin) -> Option<String> {
    let file_managers = find_finders(bin);
    if file_managers.len() < 3 {
        return None;
    }
    let n = file_managers.len();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                let (a, b, c) = (file_managers[i], file_managers[j], file_managers[k]);
                let ms = (a.ms + b.ms + c.ms) / 3.0;
                if [a.ms, b.ms, c.ms].iter().any(|m| (m - ms).abs() > ms * 0.4) {
                    continue;
                }
                let Some((tl, tr, bl)) = order(a, b, c) else { continue };
                let leg = (dist(&tl, &tr) + dist(&tl, &bl)) / 2.0;
                let dim = leg / ms + 7.0;
                let v = (((dim - 17.0) / 4.0).round() as i32).clamp(1, MAX_VERSION as i32);
                for dv in [0, 1, -1] {
                    let version = v + dv;
                    if !(1..=MAX_VERSION as i32).contains(&version) {
                        continue;
                    }
                    let size = dim_of(version as usize);
                    let m = sample(bin, &tl, &tr, &bl, size);
                    if let Some(text) = decode_matrix(&m, size) {
                        return Some(text);
                    }
                }
            }
        }
    }
    None
}

/// Finds and decodes a QR code in an 8-bit luminance image (`w` x `h`, row-major).
pub fn decode_luma(luma: &[u8], w: usize, h: usize) -> Option<String> {
    if w < 21 || h < 21 || luma.len() < w * h {
        return None;
    }
    for bin in [otsu(luma, w, h, false), adaptive(luma, w, h), otsu(luma, w, h, true)] {
        if let Some(text) = decode_bin(&bin) {
            return Some(text);
        }
    }
    None
}

/// Luminance of packed `gfx` pixels (R in the low byte), for scanning a viewfinder image.
pub fn luma_of(px: &[u32]) -> Vec<u8> {
    px.iter().map(|&p| ((p & 0xFF) * 77 + ((p >> 8) & 0xFF) * 150 + ((p >> 16) & 0xFF) * 29 >> 8) as u8).collect()
}

#[cfg(test)]
mod encoder {
    //! A minimal QR encoder (byte mode) for the tests.
    use super::*;

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

    pub fn rs_encode(data: &[u8], ec: usize) -> Vec<u8> {
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

    struct BitBuf(Vec<bool>);

    impl BitBuf {
        fn put(&mut self, v: u32, n: usize) {
            for i in (0..n).rev() {
                self.0.push(v >> i & 1 == 1);
            }
        }
    }

    pub enum Mode {
        Byte,
        Numeric,
        Alnum,
    }

    /// Returns (version, size, modules row-major).
    pub fn encode(text: &str, mode: Mode, ecl: Ecl, mask: u8) -> (usize, usize, Vec<bool>) {
        let bytes = text.as_bytes();
        let version = (1..=MAX_VERSION)
            .find(|&v| {
                let (ec, blocks) = EC_TABLE[v - 1][ecl.idx()];
                let cap = (TOTAL_CW[v - 1] - ec * blocks) * 8;
                let (count_bits, body) = match mode {
                    Mode::Byte => (if v >= 10 { 16 } else { 8 }, bytes.len() * 8),
                    Mode::Numeric => (if v >= 10 { 12 } else { 10 }, bytes.len() / 3 * 10 + [0, 4, 7][bytes.len() % 3]),
                    Mode::Alnum => (if v >= 10 { 11 } else { 9 }, bytes.len() / 2 * 11 + bytes.len() % 2 * 6),
                };
                4 + count_bits + body <= cap
            })
            .expect("text fits version 10");
        let wide = version >= 10;
        let mut buf = BitBuf(Vec::new());
        match mode {
            Mode::Byte => {
                buf.put(4, 4);
                buf.put(bytes.len() as u32, if wide { 16 } else { 8 });
                bytes.iter().for_each(|&b| buf.put(b as u32, 8));
            }
            Mode::Numeric => {
                buf.put(1, 4);
                buf.put(bytes.len() as u32, if wide { 12 } else { 10 });
                for ch in bytes.chunks(3) {
                    let v: u32 = std::str::from_utf8(ch).unwrap().parse().unwrap();
                    buf.put(v, [0, 4, 7, 10][ch.len()]);
                }
            }
            Mode::Alnum => {
                buf.put(2, 4);
                buf.put(bytes.len() as u32, if wide { 11 } else { 9 });
                let idx = |b: u8| ALNUM.iter().position(|&a| a == b).unwrap() as u32;
                for ch in bytes.chunks(2) {
                    if ch.len() == 2 {
                        buf.put(idx(ch[0]) * 45 + idx(ch[1]), 11);
                    } else {
                        buf.put(idx(ch[0]), 6);
                    }
                }
            }
        }
        let (ec, blocks) = EC_TABLE[version - 1][ecl.idx()];
        let data_cw = TOTAL_CW[version - 1] - ec * blocks;
        let cap = data_cw * 8;
        let term = (cap - buf.0.len()).min(4);
        buf.put(0, term);
        while buf.0.len() % 8 != 0 {
            buf.0.push(false);
        }
        let mut data: Vec<u8> = buf.0.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| a << 1 | b as u8)).collect();
        let mut pad = 0xEC;
        while data.len() < data_cw {
            data.push(pad);
            pad ^= 0xEC ^ 0x11;
        }
        // blocks, check bytes, interleave
        let (_, lens) = split_blocks(version, ecl);
        let mut at = 0;
        let mut dblocks = Vec::new();
        let mut eblocks = Vec::new();
        for &l in &lens {
            let d = data[at..at + l].to_vec();
            at += l;
            eblocks.push(rs_encode(&d, ec));
            dblocks.push(d);
        }
        let mut all = Vec::new();
        for k in 0..*lens.iter().max().unwrap() {
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
        let size = dim_of(version);
        let func = function_map(version);
        let mut m = vec![false; size * size];
        let set = |m: &mut Vec<bool>, x: i32, y: i32, dark: bool| {
            if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
                m[y as usize * size + x as usize] = dark;
            }
        };
        for i in 0..size as i32 {
            set(&mut m, 6, i, i % 2 == 0);
            set(&mut m, i, 6, i % 2 == 0);
        }
        for (cx, cy) in [(3, 3), (size as i32 - 4, 3), (3, size as i32 - 4)] {
            for dy in -4..=4i32 {
                for dx in -4..=4i32 {
                    let d = dx.abs().max(dy.abs());
                    set(&mut m, cx + dx, cy + dy, d != 2 && d != 4);
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
                        set(&mut m, cx as i32 + dx, cy as i32 + dy, dx.abs().max(dy.abs()) != 1);
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
            for i in 0..18 {
                let bit = bits >> i & 1 == 1;
                let (a, b) = (size as i32 - 11 + i % 3, i / 3);
                set(&mut m, a, b, bit);
                set(&mut m, b, a, bit);
            }
        }
        set(&mut m, 8, size as i32 - 8, true);
        for (i, &(x, y)) in data_positions(version).iter().enumerate() {
            let bit = i < all.len() * 8 && all[i / 8] >> (7 - i % 8) & 1 == 1;
            m[y * size + x] = bit ^ mask_bit(mask, x, y);
        }
        let fmt = format_code(ecl.format_bits() << 3 | mask as u32);
        for copy in format_positions(size) {
            for (i, &(x, y)) in copy.iter().enumerate() {
                m[y * size + x] = fmt >> i & 1 == 1;
            }
        }
        assert!(func.iter().filter(|&&f| f).count() > 0);
        (version, size, m)
    }

    /// Renders to an 8-bit image: dark 20, light 235, `scale` px per module, `quiet` modules around.
    pub fn render(m: &[bool], size: usize, scale: usize, quiet: usize) -> (Vec<u8>, usize, usize) {
        let w = (size + 2 * quiet) * scale;
        let mut img = vec![235u8; w * w];
        for y in 0..size {
            for x in 0..size {
                if m[y * size + x] {
                    for yy in 0..scale {
                        for xx in 0..scale {
                            img[((y + quiet) * scale + yy) * w + (x + quiet) * scale + xx] = 20;
                        }
                    }
                }
            }
        }
        (img, w, w)
    }
}

#[cfg(test)]
mod tests {
    use super::encoder::*;
    use super::*;

    fn roundtrip(text: &str, mode: Mode, ecl: Ecl, mask: u8, scale: usize) {
        let (_, size, m) = encode(text, mode, ecl, mask);
        let (img, w, h) = render(&m, size, scale, 4);
        assert_eq!(decode_luma(&img, w, h).as_deref(), Some(text), "{ecl:?} mask {mask} scale {scale}");
    }

    #[test]
    fn gf_arithmetic() {
        assert_eq!(mul(2, 128), 0x1D);
        assert_eq!(mul(0, 9), 0);
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1);
        }
    }

    #[test]
    fn reed_solomon_corrects_up_to_half_the_check_bytes() {
        let data: Vec<u8> = (0..30).map(|i| (i * 7 + 3) as u8).collect();
        let ec = 16;
        let mut clean = data.clone();
        clean.extend(rs_encode(&data, ec));
        assert!(rs_correct(&mut clean.clone(), ec));
        for errors in [1usize, 3, 8] {
            let mut cw = clean.clone();
            for k in 0..errors {
                cw[k * 5 + 1] ^= 0x5A + k as u8;
            }
            assert!(rs_correct(&mut cw, ec), "{errors} errors");
            assert_eq!(cw, clean);
        }
        let mut cw = clean.clone();
        for k in 0..12 {
            cw[k * 3] ^= 0xFF;
        }
        let mut copy = cw.clone();
        assert!(!(rs_correct(&mut copy, ec) && copy == clean), "too many errors are not silently 'fixed' to the original");
    }

    #[test]
    fn format_info_survives_three_bit_errors() {
        let (_, size, mut m) = encode("x", Mode::Byte, Ecl::Q, 5);
        let pos = format_positions(size);
        for &(x, y) in &pos[0][..3] {
            m[y * size + x] = !m[y * size + x];
        }
        assert_eq!(read_format(&m, size), Some((Ecl::Q, 5)));
        for &(x, y) in &pos[1][4..7] {
            m[y * size + x] = !m[y * size + x];
        }
        assert!(read_format(&m, size).is_some(), "the second copy is still intact");
    }

    #[test]
    fn matrix_round_trip_all_levels_and_masks() {
        for ecl in [Ecl::L, Ecl::M, Ecl::Q, Ecl::H] {
            for mask in 0..8 {
                let (_, size, m) = encode("https://akurai.example/qr?id=42", Mode::Byte, ecl, mask);
                assert_eq!(decode_matrix(&m, size).as_deref(), Some("https://akurai.example/qr?id=42"), "{ecl:?} {mask}");
            }
        }
    }

    #[test]
    fn decodes_a_rendered_code_in_every_mask() {
        for mask in 0..8 {
            roundtrip("Hello, Tab A8", Mode::Byte, Ecl::M, mask, 4);
        }
    }

    #[test]
    fn decodes_every_level() {
        for ecl in [Ecl::L, Ecl::M, Ecl::Q, Ecl::H] {
            roundtrip("WIFI:T:WPA;S:home;P:secret;;", Mode::Byte, ecl, 2, 5);
        }
    }

    #[test]
    fn decodes_numeric_and_alphanumeric() {
        roundtrip("0123456789012", Mode::Numeric, Ecl::L, 1, 4);
        roundtrip("HELLO WORLD 42", Mode::Alnum, Ecl::M, 3, 4);
        roundtrip("7", Mode::Numeric, Ecl::H, 0, 6);
        roundtrip("AB1", Mode::Alnum, Ecl::Q, 6, 6);
    }

    #[test]
    fn decodes_larger_versions_with_alignment_and_version_info() {
        let long: String = (0..150).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let (v, size, m) = encode(&long, Mode::Byte, Ecl::L, 4);
        assert!(v >= 7, "version {v} has version info");
        let (img, w, h) = render(&m, size, 4, 4);
        assert_eq!(decode_luma(&img, w, h).as_deref(), Some(long.as_str()));
        let mid: String = long.chars().take(60).collect();
        roundtrip(&mid, Mode::Byte, Ecl::M, 7, 3);
    }

    #[test]
    fn decodes_utf8() {
        roundtrip("Akur \u{00E1}ra\u{00F0}", Mode::Byte, Ecl::M, 2, 5);
    }

    #[test]
    fn decodes_without_a_quiet_zone_and_with_low_contrast() {
        let (_, size, m) = encode("tight", Mode::Byte, Ecl::Q, 3);
        let (mut img, w, h) = render(&m, size, 4, 1);
        for p in img.iter_mut() {
            *p = 90 + (*p as u32 * 70 / 255) as u8;
        }
        assert_eq!(decode_luma(&img, w, h).as_deref(), Some("tight"));
    }

    #[test]
    fn decodes_rotated_codes() {
        let (_, size, m) = encode("rotate me", Mode::Byte, Ecl::M, 5);
        let (img, w, h) = render(&m, size, 4, 4);
        // 90 degrees clockwise
        let mut cw = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                cw[x * h + (h - 1 - y)] = img[y * w + x];
            }
        }
        assert_eq!(decode_luma(&cw, h, w).as_deref(), Some("rotate me"));
        // 180 degrees
        let r180: Vec<u8> = img.iter().rev().copied().collect();
        assert_eq!(decode_luma(&r180, w, h).as_deref(), Some("rotate me"));
    }

    #[test]
    fn decodes_in_a_larger_noisy_scene() {
        let (_, size, m) = encode("scene", Mode::Byte, Ecl::H, 6);
        let (code, cw, ch) = render(&m, size, 3, 4);
        let (w, h) = (320, 240);
        let mut img = vec![0u8; w * h];
        for (i, p) in img.iter_mut().enumerate() {
            *p = 200 + ((i * 2654435761usize) >> 27) as u8 % 20;
        }
        let (ox, oy) = (150, 70);
        for y in 0..ch {
            for x in 0..cw {
                img[(oy + y) * w + ox + x] = code[y * cw + x].saturating_add(((x * 31 + y * 17) % 12) as u8);
            }
        }
        assert_eq!(decode_luma(&img, w, h).as_deref(), Some("scene"));
    }

    #[test]
    fn reed_solomon_repairs_damaged_modules_in_an_image() {
        let (_, size, m) = encode("damaged but readable", Mode::Byte, Ecl::H, 2);
        let (mut img, w, h) = render(&m, size, 5, 4);
        // blot out a 2x2-module patch inside the data area
        for y in 12 * 5..14 * 5 {
            for x in 14 * 5..16 * 5 {
                img[(y + 20) * w + x + 20] = 128 ^ 0xFF;
            }
        }
        assert_eq!(decode_luma(&img, w, h).as_deref(), Some("damaged but readable"));
    }

    #[test]
    fn no_code_yields_none() {
        assert_eq!(decode_luma(&vec![200u8; 100 * 100], 100, 100), None);
        let noise: Vec<u8> = (0..120 * 120).map(|i| ((i * 2654435761usize) >> 24) as u8).collect();
        assert_eq!(decode_luma(&noise, 120, 120), None);
        let gradient: Vec<u8> = (0..100 * 100).map(|i| (i % 100 * 2) as u8).collect();
        assert_eq!(decode_luma(&gradient, 100, 100), None);
        assert_eq!(decode_luma(&[0; 4], 2, 2), None);
    }

    #[test]
    fn luma_of_packed_pixels() {
        let px = [crate::gfx::pack(255, 255, 255, 255), crate::gfx::pack(0, 0, 0, 255)];
        let l = luma_of(&px);
        assert!(l[0] >= 254 && l[1] == 0);
    }
}
