//! PDF stream filters: ASCIIHex, ASCII85, RunLength, LZW and the PNG/TIFF
//! predictors. Flate lives in `inflate`; image filters (DCT) pass through.

/// Undoes a TIFF (2) or PNG (10..=15) predictor; other values pass the data through.
pub fn predictor(data: Vec<u8>, pred: i64, colors: i64, bpc: i64, columns: i64) -> Vec<u8> {
    if pred < 2 {
        return data;
    }
    let colors = colors.max(1) as usize;
    let bpc = bpc.max(1) as usize;
    let columns = columns.max(1) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    if pred == 2 {
        // TIFF predictor, byte samples only
        if bpc != 8 {
            return data;
        }
        let mut out = data;
        for r in out.chunks_mut(row) {
            for i in colors..r.len() {
                r[i] = r[i].wrapping_add(r[i - colors]);
            }
        }
        return out;
    }
    // PNG predictors: each row starts with its filter type
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let kind = chunk[0];
        let mut cur: Vec<u8> = chunk[1..].to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let add = match kind {
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            cur[i] = cur[i].wrapping_add(add as u8);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    out
}

pub fn ascii_hex(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut hi = None;
    for &c in d {
        if c == b'>' {
            break;
        }
        let Some(v) = (c as char).to_digit(16) else { continue };
        match hi.take() {
            Some(h) => out.push((h << 4 | v) as u8),
            None => hi = Some(v),
        }
    }
    if let Some(h) = hi {
        out.push((h << 4) as u8);
    }
    out
}

pub fn ascii85(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut group = [0u32; 5];
    let mut n = 0;
    let mut i = 0;
    while i < d.len() {
        let c = d[i];
        i += 1;
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = (c - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u32, |a, &g| a.wrapping_mul(85).wrapping_add(g));
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u32, |a, &g| a.wrapping_mul(85).wrapping_add(g));
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    }
    out
}

pub fn run_length(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < d.len() {
        let n = d[i] as usize;
        i += 1;
        if n == 128 {
            break;
        }
        if n < 128 {
            let end = (i + n + 1).min(d.len());
            out.extend_from_slice(&d[i..end]);
            i = end;
        } else if let Some(&b) = d.get(i) {
            out.extend(std::iter::repeat(b).take(257 - n));
            i += 1;
        }
    }
    out
}

pub fn lzw(d: &[u8], early: bool) -> Vec<u8> {
    let mut out: Vec<u8> = vec![];
    let mut table: Vec<Vec<u8>> = vec![];
    let reset = |t: &mut Vec<Vec<u8>>| {
        t.clear();
        t.extend((0..=255u8).map(|b| vec![b]));
        t.push(vec![]); // 256 clear
        t.push(vec![]); // 257 end
    };
    reset(&mut table);
    let (mut bits, mut acc, mut width) = (0u32, 0u32, 9u32);
    let mut prev: Option<Vec<u8>> = None;
    for &byte in d {
        acc = acc << 8 | byte as u32;
        bits += 8;
        while bits >= width {
            let code = (acc >> (bits - width)) & ((1 << width) - 1);
            bits -= width;
            if code == 256 {
                reset(&mut table);
                width = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                return out;
            }
            let entry = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else if let Some(p) = &prev {
                let mut e = p.clone();
                e.push(p[0]);
                e
            } else {
                return out;
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev.take() {
                let mut e = p;
                e.push(entry[0]);
                table.push(e);
            }
            prev = Some(entry);
            let size = table.len() + usize::from(early);
            width = match size {
                0..=511 => 9,
                512..=1023 => 10,
                1024..=2047 => 11,
                _ => 12,
            };
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>"), b"Hello");
        assert_eq!(ascii85(b"87cURDZ~>"), b"Hello");
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'z', 128]), b"abczzz");
        // LZW: the PDF spec's example
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&enc, true), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn png_up_predictor() {
        assert_eq!(predictor(vec![2, 1, 2, 3, 2, 1, 1, 1], 12, 1, 8, 3), [1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn tiff_predictor_adds_the_left_neighbour() {
        assert_eq!(predictor(vec![1, 1, 1, 5, 1, 1], 2, 1, 8, 3), [1, 2, 3, 5, 6, 7]);
    }
}
