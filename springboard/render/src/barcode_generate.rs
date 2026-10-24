//! One-dimensional barcode generators, as the reference system ships them: `CICode128BarcodeGenerator`,
//! and the generators added in OS 27, `CIEAN13BarcodeGenerator`, `CICode39BarcodeGenerator`,
//! `CICodabarBarcodeGenerator` and `CIInterleaved2of5BarcodeGenerator`.
//!
//! Measured from the reference generators (REA evidence ev_cd797970, ev_6bb789c8, ev_76cca25e,
//! ev_c86e9cb3): every one takes the message as bytes, a quiet space in modules and a bar height in
//! pixels. The quiet space is clamped to 0..=100 and rounded (7 when unset; the attribute default an
//! app sees is 10, slider 0..=20); the height is clamped to 1..=500 and rounded (32 when unset, slider
//! 1..=50). One module is one pixel.
//!
//! The symbologies follow their public standards (Code 128: ISO/IEC 15417; EAN-13: ISO/IEC 15420;
//! Code 39: ISO/IEC 16388; Codabar; Interleaved 2 of 5: ISO/IEC 16390). Decisions where the reference
//! behaviour is not measured: the quiet space is added on the left and right only; wide elements of
//! Code 39, Codabar and Interleaved 2 of 5 are three modules (a 3:1 ratio, inside every standard's
//! range) and characters are separated by one narrow gap; Code 128 picks code set C for runs of four
//! or more digits and A or B otherwise, and accepts ASCII only (no FNC4 extended bytes).
use crate::gfx::Image;

/// The quiet space used when none is given, in modules.
pub const QUIET_SPACE_UNSET: usize = 7;
/// The quiet space an app sees as the attribute default, in modules.
pub const QUIET_SPACE_DEFAULT: usize = 10;
/// The largest quiet space, in modules.
pub const QUIET_SPACE_MAX: usize = 100;
/// The bar height used when none is given, in pixels.
pub const HEIGHT_DEFAULT: usize = 32;
/// The bar height range, in pixels.
pub const HEIGHT_MIN: usize = 1;
pub const HEIGHT_MAX: usize = 500;
/// Modules in a wide element of the two-width symbologies.
pub const WIDE: usize = 3;

/// A one-dimensional symbology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbology {
    Code128,
    Ean13,
    Code39,
    Codabar,
    Interleaved2of5,
}

/// Clamps and rounds a quiet space the way the reference does. Non-finite values give the unset default.
pub fn quiet_space(v: f64) -> usize {
    if !v.is_finite() {
        return QUIET_SPACE_UNSET;
    }
    v.clamp(0.0, QUIET_SPACE_MAX as f64).round() as usize
}

/// Clamps and rounds a bar height the way the reference does. Non-finite values give the default.
pub fn bar_height(v: f64) -> usize {
    if !v.is_finite() {
        return HEIGHT_DEFAULT;
    }
    v.clamp(HEIGHT_MIN as f64, HEIGHT_MAX as f64).round() as usize
}

/// Code 128 element widths for values 0 to 105, then the stop symbol (value 106, with its final bar).
const CODE128: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213",
    "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221", "223211", "221132",
    "221231", "213212", "223112", "312131", "311222", "321122", "321221", "312212", "322112", "322211",
    "212123", "212321", "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313",
    "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121", "313121", "211331",
    "231131", "213113", "213311", "213131", "311123", "311321", "331121", "312113", "312311", "332111",
    "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214",
    "112412", "122114", "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111",
    "111242", "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311", "113141",
    "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

const START_A: u8 = 103;
const START_B: u8 = 104;
const START_C: u8 = 105;
const CODE_C: u8 = 99;
const CODE_B: u8 = 100;
const CODE_A: u8 = 101;
const STOP: u8 = 106;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Set {
    A,
    B,
    C,
}

/// The Code 128 symbol values for `msg`, start and checksum included, stop last. `None` for an empty
/// message or a byte above 127.
pub fn code128_values(msg: &[u8]) -> Option<Vec<u8>> {
    if msg.is_empty() || msg.iter().any(|&b| b > 127) {
        return None;
    }
    let digits_at = |i: usize| msg[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    // A when a control character comes before any lower-case character, else B.
    let pick_ab = |i: usize| {
        for &b in &msg[i..] {
            if b < 32 {
                return Set::A;
            }
            if b >= 96 {
                return Set::B;
            }
        }
        Set::B
    };
    let value = |set: Set, b: u8| match set {
        Set::A if b < 32 => b + 64,
        _ => b - 32,
    };
    let fits = |set: Set, b: u8| match set {
        Set::A => b < 96,
        Set::B => b >= 32,
        Set::C => false,
    };
    let run0 = digits_at(0);
    let mut set = if run0 >= 4 || (run0 == msg.len() && run0 % 2 == 0) { Set::C } else { pick_ab(0) };
    let mut out = vec![match set {
        Set::A => START_A,
        Set::B => START_B,
        Set::C => START_C,
    }];
    let mut i = 0;
    while i < msg.len() {
        if set == Set::C {
            if digits_at(i) >= 2 {
                out.push((msg[i] - b'0') * 10 + (msg[i + 1] - b'0'));
                i += 2;
            } else {
                set = pick_ab(i);
                out.push(if set == Set::A { CODE_A } else { CODE_B });
            }
            continue;
        }
        let run = digits_at(i);
        if run >= 4 && run % 2 == 0 {
            out.push(CODE_C);
            set = Set::C;
            continue;
        }
        // An odd run of four or more: its first digit goes in the current set, the rest in set C.
        let b = msg[i];
        if !fits(set, b) {
            set = if set == Set::A { Set::B } else { Set::A };
            out.push(if set == Set::A { CODE_A } else { CODE_B });
        }
        out.push(value(set, b));
        i += 1;
    }
    let sum = out.iter().enumerate().fold(0u32, |acc, (k, &v)| acc + k.max(1) as u32 * v as u32);
    out.push((sum % 103) as u8);
    out.push(STOP);
    Some(out)
}

fn push_widths(out: &mut Vec<bool>, widths: impl IntoIterator<Item = usize>) {
    let mut bar = true;
    for w in widths {
        out.extend(std::iter::repeat(bar).take(w));
        bar = !bar;
    }
}

fn code128_modules(msg: &[u8]) -> Option<Vec<bool>> {
    let mut out = Vec::new();
    for v in code128_values(msg)? {
        push_widths(&mut out, CODE128[v as usize].bytes().map(|b| (b - b'0') as usize));
    }
    Some(out)
}

/// EAN-13 left-hand odd parity (L) codes; the even codes (G) are these reversed and complemented, the
/// right-hand codes (R) these complemented.
const EAN_L: [u8; 10] = [0x0d, 0x19, 0x13, 0x3d, 0x23, 0x31, 0x2f, 0x3b, 0x37, 0x0b];
/// For each leading digit, which of the six left digits use the even (G) set, bit 5 being the first.
const EAN_PARITY: [u8; 10] = [0b000000, 0b001011, 0b001101, 0b001110, 0b010011, 0b011001, 0b011100, 0b010101, 0b010110, 0b011010];

/// The EAN-13 check digit of twelve digits.
pub fn ean13_check_digit(digits: &[u8; 12]) -> u8 {
    let sum: u32 = digits.iter().enumerate().map(|(k, &d)| d as u32 * if k % 2 == 0 { 1 } else { 3 }).sum();
    ((10 - sum % 10) % 10) as u8
}

fn ean13_modules(msg: &[u8]) -> Option<Vec<bool>> {
    if !(msg.len() == 12 || msg.len() == 13) || !msg.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut d = [0u8; 13];
    for (k, &b) in msg.iter().enumerate() {
        d[k] = b - b'0';
    }
    let mut first12 = [0u8; 12];
    first12.copy_from_slice(&d[..12]);
    let check = ean13_check_digit(&first12);
    if msg.len() == 13 && d[12] != check {
        return None;
    }
    d[12] = check;
    let bits = |code: u8, out: &mut Vec<bool>| (0..7).rev().for_each(|k| out.push(code >> k & 1 == 1));
    let mut out = Vec::with_capacity(95);
    out.extend([true, false, true]);
    for k in 0..6 {
        let l = EAN_L[d[1 + k] as usize];
        let even = EAN_PARITY[d[0] as usize] >> (5 - k) & 1 == 1;
        let code = if even { (!l & 0x7f).reverse_bits() >> 1 } else { l };
        bits(code, &mut out);
    }
    out.extend([false, true, false, true, false]);
    for k in 0..6 {
        bits(!EAN_L[d[7 + k] as usize] & 0x7f, &mut out);
    }
    out.extend([true, false, true]);
    Some(out)
}

/// Code 39: for each character, nine elements (five bars, four spaces) as narrow (false) or wide (true).
fn code39_pattern(c: u8) -> Option<[bool; 9]> {
    const GROUPS: [(&[u8; 10], usize); 4] = [(b"1234567890", 1), (b"ABCDEFGHIJ", 2), (b"KLMNOPQRST", 3), (b"UVWXYZ-. *", 0)];
    // The two wide bars (1-based) for the ten positions of a group.
    const BARS: [(usize, usize); 10] = [(1, 5), (2, 5), (1, 2), (3, 5), (1, 3), (2, 3), (4, 5), (1, 4), (2, 4), (3, 4)];
    let mut p = [false; 9];
    for (chars, space) in GROUPS {
        if let Some(k) = chars.iter().position(|&x| x == c) {
            let (a, b) = BARS[k];
            p[2 * (a - 1)] = true;
            p[2 * (b - 1)] = true;
            p[2 * space + 1] = true;
            return Some(p);
        }
    }
    // The four characters with three wide spaces and no wide bar.
    let spaces: [usize; 3] = match c {
        b'$' => [0, 1, 2],
        b'/' => [0, 1, 3],
        b'+' => [0, 2, 3],
        b'%' => [1, 2, 3],
        _ => return None,
    };
    for s in spaces {
        p[2 * s + 1] = true;
    }
    Some(p)
}

fn code39_modules(msg: &[u8]) -> Option<Vec<bool>> {
    if msg.is_empty() {
        return None;
    }
    let mut chars = vec![b'*'];
    for &b in msg {
        let c = b.to_ascii_uppercase();
        if c == b'*' {
            return None;
        }
        chars.push(c);
    }
    chars.push(b'*');
    let mut out = Vec::new();
    for (k, &c) in chars.iter().enumerate() {
        if k > 0 {
            out.push(false);
        }
        push_widths(&mut out, code39_pattern(c)?.map(|w| if w { WIDE } else { 1 }));
    }
    Some(out)
}

/// Codabar: seven elements per character, as narrow (`n`) or wide (`w`).
const CODABAR: [(u8, &str); 20] = [
    (b'0', "nnnnnww"), (b'1', "nnnnwwn"), (b'2', "nnnwnnw"), (b'3', "wwnnnnn"), (b'4', "nnwnnwn"),
    (b'5', "wnnnnwn"), (b'6', "nwnnnnw"), (b'7', "nwnnwnn"), (b'8', "nwwnnnn"), (b'9', "wnnwnnn"),
    (b'-', "nnnwwnn"), (b'$', "nnwwnnn"), (b':', "wnnnwnw"), (b'/', "wnwnnnw"), (b'.', "wnwnwnn"),
    (b'+', "nnwnwnw"), (b'A', "nnwwnwn"), (b'B', "nwnwnnw"), (b'C', "nnnwnww"), (b'D', "nnnwwwn"),
];

fn codabar_modules(msg: &[u8]) -> Option<Vec<bool>> {
    let up: Vec<u8> = msg.iter().map(u8::to_ascii_uppercase).collect();
    let is_guard = |c: u8| (b'A'..=b'D').contains(&c);
    let body: Vec<u8> = match (up.first(), up.last()) {
        (Some(&f), Some(&l)) if up.len() >= 2 && is_guard(f) && is_guard(l) => up.clone(),
        _ => {
            if up.is_empty() || up.iter().any(|&c| is_guard(c)) {
                return None;
            }
            // No start and stop given: frame the message with A.
            [b"A".as_slice(), up.as_slice(), b"A".as_slice()].concat()
        }
    };
    if body[1..body.len() - 1].iter().any(|&c| is_guard(c)) {
        return None;
    }
    let mut out = Vec::new();
    for (k, &c) in body.iter().enumerate() {
        let pat = CODABAR.iter().find(|e| e.0 == c)?.1;
        if k > 0 {
            out.push(false);
        }
        push_widths(&mut out, pat.bytes().map(|b| if b == b'w' { WIDE } else { 1 }));
    }
    Some(out)
}

/// Interleaved 2 of 5: the five elements of each digit, wide where true.
const I2OF5: [[bool; 5]; 10] = {
    const N: bool = false;
    const W: bool = true;
    [
        [N, N, W, W, N],
        [W, N, N, N, W],
        [N, W, N, N, W],
        [W, W, N, N, N],
        [N, N, W, N, W],
        [W, N, W, N, N],
        [N, W, W, N, N],
        [N, N, N, W, W],
        [W, N, N, W, N],
        [N, W, N, W, N],
    ]
};

fn i2of5_modules(msg: &[u8]) -> Option<Vec<bool>> {
    if msg.is_empty() || !msg.iter().all(u8::is_ascii_digit) {
        return None;
    }
    // An odd count of digits gets a leading zero, since digits are encoded in pairs.
    let digits: Vec<usize> = if msg.len() % 2 == 1 { std::iter::once(0).chain(msg.iter().map(|&b| (b - b'0') as usize)).collect() } else { msg.iter().map(|&b| (b - b'0') as usize).collect() };
    let width = |w: bool| if w { WIDE } else { 1 };
    let mut out = Vec::new();
    push_widths(&mut out, [1, 1, 1, 1]);
    for pair in digits.chunks(2) {
        let (bars, spaces) = (I2OF5[pair[0]], I2OF5[pair[1]]);
        push_widths(&mut out, (0..5).flat_map(|k| [width(bars[k]), width(spaces[k])]));
    }
    push_widths(&mut out, [WIDE, 1, 1]);
    Some(out)
}

/// The modules of `msg` in `symbology`, left to right, `true` for a bar. `None` when the message cannot
/// be encoded (empty, a character outside the symbology, or a wrong EAN-13 check digit).
pub fn modules(symbology: Symbology, msg: &[u8]) -> Option<Vec<bool>> {
    match symbology {
        Symbology::Code128 => code128_modules(msg),
        Symbology::Ean13 => ean13_modules(msg),
        Symbology::Code39 => code39_modules(msg),
        Symbology::Codabar => codabar_modules(msg),
        Symbology::Interleaved2of5 => i2of5_modules(msg),
    }
}

/// Draws modules one pixel wide and `height` pixels tall with `quiet` light modules on each side.
/// `dark` and `light` are packed like `Image` pixels. `None` for no modules or a zero height.
pub fn to_image(modules: &[bool], quiet: usize, height: usize, dark: u32, light: u32) -> Option<Image> {
    if modules.is_empty() || height == 0 {
        return None;
    }
    let w = modules.len() + 2 * quiet;
    let mut row = vec![light; w];
    for (k, &bar) in modules.iter().enumerate() {
        if bar {
            row[quiet + k] = dark;
        }
    }
    let mut img = Image::new(w, height);
    for y in 0..height {
        img.px[y * w..(y + 1) * w].copy_from_slice(&row);
    }
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(m: &[bool]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut k = 0;
        while k < m.len() {
            let start = k;
            while k < m.len() && m[k] == m[start] {
                k += 1;
            }
            out.push(k - start);
        }
        out
    }

    #[test]
    fn code128_table_has_eleven_modules_even_bars_and_no_duplicates() {
        for (v, p) in CODE128.iter().enumerate().take(106) {
            let w: Vec<usize> = p.bytes().map(|b| (b - b'0') as usize).collect();
            assert_eq!(w.len(), 6, "{v}");
            assert_eq!(w.iter().sum::<usize>(), 11, "{v}");
            assert_eq!((w[0] + w[2] + w[4]) % 2, 0, "bar modules are even in {v}");
        }
        assert_eq!(CODE128[106].bytes().map(|b| (b - b'0') as usize).sum::<usize>(), 13);
        let mut sorted = CODE128.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 107);
    }

    #[test]
    fn code128_values_follow_the_set_rules_and_checksum() {
        assert_eq!(code128_values(b"AB").unwrap(), [104, 33, 34, 102, 106]);
        assert_eq!(code128_values(b"123456").unwrap(), [105, 12, 34, 56, 44, 106]);
        assert_eq!(code128_values(b"\x01a").unwrap(), [103, 65, 100, 65, 48, 106]);
        assert_eq!(code128_values(b"ab12345").unwrap(), [104, 65, 66, 17, 99, 23, 45, 0, 106]);
        assert!(code128_values(b"").is_none());
        assert!(code128_values(&[200]).is_none());
    }

    #[test]
    fn code128_modules_decode_back_to_the_values() {
        let vals = code128_values(b"BifrOSt 2026").unwrap();
        let m = modules(Symbology::Code128, b"BifrOSt 2026").unwrap();
        assert_eq!(m.len(), 11 * (vals.len() - 1) + 13);
        let widths = runs(&m);
        let mut decoded = Vec::new();
        for chunk in widths.chunks(6).take(vals.len() - 1) {
            let s: String = chunk.iter().map(|w| char::from(b'0' + *w as u8)).collect();
            decoded.push(CODE128.iter().position(|p| *p == s).unwrap() as u8);
        }
        assert_eq!(decoded, vals[..vals.len() - 1]);
        assert!(m[0] && *m.last().unwrap(), "starts and ends with a bar");
    }

    #[test]
    fn ean13_matches_the_published_example() {
        let d = [4, 0, 0, 6, 3, 8, 1, 3, 3, 3, 9, 3];
        assert_eq!(ean13_check_digit(&d), 1);
        let a = modules(Symbology::Ean13, b"400638133393").unwrap();
        let b = modules(Symbology::Ean13, b"4006381333931").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 95);
        assert_eq!(&a[..3], &[true, false, true]);
        assert_eq!(&a[45..50], &[false, true, false, true, false]);
        // The first left digit (0) under parity pattern 4 (L G L L G G) is L: 0001101.
        assert_eq!(&a[3..10], &[false, false, false, true, true, false, true]);
        // The second left digit (0) is G: 0100111.
        assert_eq!(&a[10..17], &[false, true, false, false, true, true, true]);
        assert!(modules(Symbology::Ean13, b"4006381333932").is_none(), "wrong check digit");
        assert!(modules(Symbology::Ean13, b"12345").is_none());
    }

    #[test]
    fn code39_characters_have_three_wide_elements_and_follow_the_groups() {
        let all = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. *$/+%";
        let mut seen = Vec::new();
        for &c in all {
            let p = code39_pattern(c).unwrap();
            assert_eq!(p.iter().filter(|&&w| w).count(), 3, "{}", c as char);
            seen.push(p);
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), all.len());
        // '1' is wide-narrow-narrow-wide-narrow-narrow-narrow-narrow-wide.
        assert_eq!(code39_pattern(b'1').unwrap(), [true, false, false, true, false, false, false, false, true]);
        assert!(code39_pattern(b'#').is_none());
        let m = modules(Symbology::Code39, b"ab").unwrap();
        assert_eq!(m, modules(Symbology::Code39, b"AB").unwrap());
        // Four characters (two guards), each 3 wide and 6 narrow elements, with three gaps.
        assert_eq!(m.len(), 4 * (3 * WIDE + 6) + 3);
        assert!(modules(Symbology::Code39, b"A*B").is_none());
    }

    #[test]
    fn codabar_frames_with_guards_and_refuses_inner_guards() {
        let framed = modules(Symbology::Codabar, b"A123B").unwrap();
        assert!(framed[0]);
        let auto = modules(Symbology::Codabar, b"123").unwrap();
        assert_eq!(runs(&auto)[..7], runs(&modules(Symbology::Codabar, b"A1A").unwrap())[..7]);
        assert!(modules(Symbology::Codabar, b"1A2").is_none());
        assert!(modules(Symbology::Codabar, b"12x").is_none());
        let mut pats: Vec<&str> = CODABAR.iter().map(|e| e.1).collect();
        pats.sort();
        pats.dedup();
        assert_eq!(pats.len(), 20);
    }

    #[test]
    fn interleaved_2_of_5_follows_the_weights_and_pads_odd_lengths() {
        // Wide positions carry weights 1, 2, 4, 7 and 0; zero is 4 + 7.
        for (d, p) in I2OF5.iter().enumerate() {
            assert_eq!(p.iter().filter(|&&w| w).count(), 2);
            let sum: usize = p.iter().zip([1, 2, 4, 7, 0]).filter(|(w, _)| **w).map(|(_, v)| v).sum();
            assert_eq!(sum % 11, d % 11, "{d}");
        }
        assert_eq!(modules(Symbology::Interleaved2of5, b"123").unwrap(), modules(Symbology::Interleaved2of5, b"0123").unwrap());
        let m = modules(Symbology::Interleaved2of5, b"12").unwrap();
        // Start (4) + one pair (10 elements: 4 wide, 6 narrow) + stop (wide + 2).
        assert_eq!(m.len(), 4 + 4 * WIDE + 6 + WIDE + 2);
        assert!(modules(Symbology::Interleaved2of5, b"1a").is_none());
    }

    #[test]
    fn image_has_the_quiet_space_and_height_and_inputs_clamp_like_the_reference() {
        let m = modules(Symbology::Code128, b"A").unwrap();
        let img = to_image(&m, QUIET_SPACE_DEFAULT, HEIGHT_DEFAULT, 0xff00_0000, 0xffff_ffff).unwrap();
        assert_eq!((img.w, img.h), (m.len() + 20, 32));
        assert_eq!(img.px[0], 0xffff_ffff);
        assert_eq!(img.px[10], 0xff00_0000);
        assert!(to_image(&m, 0, 0, 0, 0).is_none());
        assert_eq!(quiet_space(-3.0), 0);
        assert_eq!(quiet_space(250.0), 100);
        assert_eq!(quiet_space(6.5), 7);
        assert_eq!(quiet_space(f64::NAN), QUIET_SPACE_UNSET);
        assert_eq!(bar_height(0.0), 1);
        assert_eq!(bar_height(9000.0), 500);
    }
}
