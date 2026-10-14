//! One-dimensional barcode decoding from a row of brightness samples: EAN-13 and Code 39. A row is thresholded at its
//! mid level, turned into run lengths of bars and spaces, and the runs are matched against the symbology's module
//! patterns, tolerating print and camera blur. `decode_row` tries both orientations. Pure std.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Symbology {
    Ean13,
    Code39,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub symbology: Symbology,
    pub text: String,
}

/// Run lengths of a row: alternating dark and light runs, starting with the first run's colour in `starts_dark`.
pub fn runs(row: &[u8]) -> (Vec<usize>, bool) {
    if row.is_empty() {
        return (vec![], false);
    }
    let (lo, hi) = (*row.iter().min().unwrap_or(&0), *row.iter().max().unwrap_or(&255));
    if hi - lo < 40 {
        return (vec![], false);
    }
    let mid = lo as u16 + (hi - lo) as u16 / 2;
    let dark = |v: u8| (v as u16) < mid;
    let starts_dark = dark(row[0]);
    let mut out = vec![];
    let mut cur = starts_dark;
    let mut n = 0;
    for &v in row {
        if dark(v) == cur {
            n += 1;
        } else {
            out.push(n);
            cur = !cur;
            n = 1;
        }
    }
    out.push(n);
    (out, starts_dark)
}

// EAN-13 digit patterns as module widths (bar, space, bar, space) for sets A, B (parity) and C.
const L_CODE: [[u8; 4]; 10] = [
    [3, 2, 1, 1], [2, 2, 2, 1], [2, 1, 2, 2], [1, 4, 1, 1], [1, 1, 3, 2],
    [1, 2, 3, 1], [1, 1, 1, 4], [1, 3, 1, 2], [1, 2, 1, 3], [3, 1, 1, 2],
];
const FIRST_DIGIT: [&str; 10] = ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"];

fn normalize(widths: &[usize], modules: usize) -> Option<Vec<u8>> {
    let total: usize = widths.iter().sum();
    if total == 0 {
        return None;
    }
    let unit = total as f32 / modules as f32;
    Some(widths.iter().map(|w| ((*w as f32 / unit).round() as i32).clamp(1, 4) as u8).collect())
}

/// Decodes one EAN-13 digit from four runs (space-first for set A/B reading order: the runs here are
/// bar-first or space-first depending on the caller); returns the digit and whether it is a G-code (reversed).
fn ean_digit(four: &[usize]) -> Option<(u8, bool)> {
    let w = normalize(four, 7)?;
    // widths as space, bar, space, bar (the order they appear after a bar-ended guard)
    for (d, p) in L_CODE.iter().enumerate() {
        if w == p {
            return Some((d as u8, false));
        }
        let rev = [p[3], p[2], p[1], p[0]];
        if w == rev {
            return Some((d as u8, true));
        }
    }
    None
}

fn ean_checksum(d: &[u8]) -> bool {
    if d.len() != 13 {
        return false;
    }
    let sum: u32 = d[..12].iter().enumerate().map(|(i, v)| u32::from(*v) * if i % 2 == 0 { 1 } else { 3 }).sum();
    (10 - sum % 10) % 10 == u32::from(d[12])
}

/// Looks for an EAN-13 symbol in the runs, which must begin with a light (quiet zone) run.
fn ean13(r: &[usize]) -> Option<String> {
    // 3 guard + 6*4 + 5 centre + 6*4 + 3 guard = 59 runs after the quiet zone
    for start in 1..r.len().saturating_sub(58) {
        if start % 2 == 0 {
            continue; // bars sit at odd indexes (index 0 is the quiet zone)
        }
        let seg = &r[start..start + 59];
        let g = normalize(&seg[..3], 3)?;
        if g != [1, 1, 1] {
            continue;
        }
        let mut digits: Vec<u8> = vec![];
        let mut pattern = String::new();
        let mut ok = true;
        for i in 0..6 {
            match ean_digit(&seg[3 + i * 4..7 + i * 4]) {
                Some((d, g_code)) => {
                    digits.push(d);
                    pattern.push(if g_code { 'G' } else { 'L' });
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let Some(first) = FIRST_DIGIT.iter().position(|p| *p == pattern) else { continue };
        for i in 0..6 {
            match ean_digit(&seg[32 + i * 4..36 + i * 4]) {
                Some((d, false)) => digits.push(d),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut all = vec![first as u8];
        all.extend(digits);
        if ean_checksum(&all) {
            return Some(all.iter().map(|d| char::from(b'0' + d)).collect());
        }
    }
    None
}

// Code 39: each character is 9 elements (5 bars, 4 spaces); `1` marks a wide element. Index = character.
const C39: [(&str, &str); 44] = [
    ("0", "000110100"), ("1", "100100001"), ("2", "001100001"), ("3", "101100000"), ("4", "000110001"),
    ("5", "100110000"), ("6", "001110000"), ("7", "000100101"), ("8", "100100100"), ("9", "001100100"),
    ("A", "100001001"), ("B", "001001001"), ("C", "101001000"), ("D", "000011001"), ("E", "100011000"),
    ("F", "001011000"), ("G", "000001101"), ("H", "100001100"), ("I", "001001100"), ("J", "000011100"),
    ("K", "100000011"), ("L", "001000011"), ("M", "101000010"), ("N", "000010011"), ("O", "100010010"),
    ("P", "001010010"), ("Q", "000000111"), ("R", "100000110"), ("S", "001000110"), ("T", "000010110"),
    ("U", "110000001"), ("V", "011000001"), ("W", "111000000"), ("X", "010010001"), ("Y", "110010000"),
    ("Z", "011010000"), ("-", "010000101"), (".", "110000100"), (" ", "011000100"), ("$", "010101000"),
    ("/", "010100010"), ("+", "010001010"), ("%", "000101010"), ("*", "010010100"),
];

fn code39_char(nine: &[usize]) -> Option<&'static str> {
    // wide elements are about 2-3 times the narrow ones: split at the midpoint of the extremes
    let (lo, hi) = (*nine.iter().min()?, *nine.iter().max()?);
    if hi < lo * 2 {
        return None;
    }
    let cut = (lo + hi) as f32 / 2.0;
    let pat: String = nine.iter().map(|w| if *w as f32 > cut { '1' } else { '0' }).collect();
    C39.iter().find(|(_, p)| *p == pat).map(|(c, _)| *c)
}

fn code39(r: &[usize], starts_dark: bool) -> Option<String> {
    // runs start at index 0 with a bar when the row starts dark; skip a leading quiet zone
    let first_bar = if starts_dark { 0 } else { 1 };
    let mut i = first_bar;
    while i + 9 <= r.len() {
        if code39_char(&r[i..i + 9]) == Some("*") {
            let mut text = String::new();
            let mut j = i + 10; // nine elements plus the inter-character gap
            while j + 9 <= r.len() {
                match code39_char(&r[j..j + 9]) {
                    Some("*") => return (!text.is_empty()).then_some(text),
                    Some(c) => text.push_str(c),
                    None => break,
                }
                j += 10;
            }
        }
        i += 2;
    }
    None
}

/// Decodes a row of 0-255 brightness samples; also tries the row reversed.
pub fn decode_row(row: &[u8]) -> Option<Decoded> {
    for reversed in [false, true] {
        let samples: Vec<u8> = if reversed { row.iter().rev().copied().collect() } else { row.to_vec() };
        let (r, starts_dark) = runs(&samples);
        if r.len() < 20 {
            continue;
        }
        let mut with_quiet = r.clone();
        if starts_dark {
            with_quiet.insert(0, 1);
        }
        if let Some(text) = ean13(&with_quiet) {
            return Some(Decoded { symbology: Symbology::Ean13, text });
        }
        if let Some(text) = code39(&r, starts_dark) {
            return Some(Decoded { symbology: Symbology::Code39, text });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders module widths (alternating bar, space, ...) into a sample row `scale` samples per module.
    fn render(widths: &[usize], scale: usize, quiet: usize) -> Vec<u8> {
        let mut row = vec![255u8; quiet];
        for (i, w) in widths.iter().enumerate() {
            row.extend(std::iter::repeat(if i % 2 == 0 { 0u8 } else { 255u8 }).take(w * scale));
        }
        row.extend(std::iter::repeat(255u8).take(quiet));
        row
    }

    /// EAN-13 module widths for 12 data digits plus the computed check digit (bar-first).
    fn ean_widths(digits: &[u8; 12]) -> (Vec<usize>, [u8; 13]) {
        let sum: u32 = digits.iter().enumerate().map(|(i, v)| u32::from(*v) * if i % 2 == 0 { 1 } else { 3 }).sum();
        let check = ((10 - sum % 10) % 10) as u8;
        let mut all = [0u8; 13];
        all[..12].copy_from_slice(digits);
        all[12] = check;
        let parity = FIRST_DIGIT[all[0] as usize].as_bytes();
        let mut w: Vec<usize> = vec![1, 1, 1];
        for i in 0..6 {
            let d = all[1 + i] as usize;
            let p = L_CODE[d];
            // L-code is space, bar, space, bar; the symbol begins after a bar so the order is space-first
            let four: Vec<usize> = if parity[i] == b'L' { p.iter().map(|x| *x as usize).collect() } else { p.iter().rev().map(|x| *x as usize).collect() };
            w.extend(four);
        }
        w.extend([1, 1, 1, 1, 1]);
        for i in 0..6 {
            let d = all[7 + i] as usize;
            w.extend(L_CODE[d].iter().map(|x| *x as usize));
        }
        w.extend([1, 1, 1]);
        (w, all)
    }

    #[test]
    fn runs_alternate() {
        let (r, dark) = runs(&[0, 0, 255, 255, 255, 0]);
        assert_eq!(r, vec![2, 3, 1]);
        assert!(dark);
        assert_eq!(runs(&[100, 110, 105]).0, Vec::<usize>::new());
        assert!(runs(&[]).0.is_empty());
    }

    #[test]
    fn checksum() {
        assert!(ean_checksum(&[4, 0, 0, 6, 3, 8, 1, 3, 3, 3, 9, 3, 6]));
        assert!(!ean_checksum(&[4, 0, 0, 6, 3, 8, 1, 3, 3, 3, 9, 3, 5]));
        assert!(!ean_checksum(&[1, 2, 3]));
    }

    #[test]
    fn ean13_round_trip_at_scales() {
        let digits = [5, 9, 0, 1, 2, 3, 4, 1, 2, 3, 4, 5];
        let (w, all) = ean_widths(&digits);
        let want: String = all.iter().map(|d| char::from(b'0' + d)).collect();
        for scale in [2, 3, 5] {
            let row = render(&w, scale, 12 * scale);
            let d = decode_row(&row).unwrap_or_else(|| panic!("scale {scale}"));
            assert_eq!(d.symbology, Symbology::Ean13);
            assert_eq!(d.text, want);
        }
    }

    #[test]
    fn reversed_rows_decode_too() {
        let (w, all) = ean_widths(&[2, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut row = render(&w, 3, 30);
        row.reverse();
        let want: String = all.iter().map(|d| char::from(b'0' + d)).collect();
        assert_eq!(decode_row(&row).map(|d| d.text), Some(want));
    }

    #[test]
    fn code39_round_trip() {
        let mut widths: Vec<usize> = vec![];
        for ch in ["*", "A", "7", "*"] {
            let pat = C39.iter().find(|(c, _)| *c == ch).unwrap().1;
            for b in pat.chars() {
                widths.push(if b == '1' { 3 } else { 1 });
            }
            widths.push(1); // inter-character gap (a space)
        }
        widths.pop();
        let row = render(&widths, 2, 20);
        let d = decode_row(&row).unwrap();
        assert_eq!(d, Decoded { symbology: Symbology::Code39, text: "A7".to_string() });
    }

    #[test]
    fn noise_decodes_to_nothing() {
        assert_eq!(decode_row(&[128; 300]), None);
        let noise: Vec<u8> = (0..300u32).map(|i| ((i * 37 + 11) % 256) as u8).collect();
        assert_eq!(decode_row(&noise), None);
    }
}
