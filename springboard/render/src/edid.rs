//! The colour of the connected display, read from its EDID block. The base block (128 bytes) carries the CIE xy
//! chromaticities of the red, green, blue and white points as ten-bit values: the high eight bits in bytes 27 to 34
//! and the low two bits packed into bytes 25 and 26. Byte 23 is the display gamma as `(value + 100) / 100`, or 0xFF
//! when the display uses another curve. `display_edid` reads the first connected connector under `/sys/class/drm`
//! whose EDID block is valid, and the caller falls back to sRGB when there is none.

use std::path::Path;

/// The chromaticities and gamma of a display.
#[derive(Clone, Debug, PartialEq)]
pub struct Edid {
    pub red: (f64, f64),
    pub green: (f64, f64),
    pub blue: (f64, f64),
    pub white: (f64, f64),
    /// The gamma the display declares, or `None` when it uses a curve other than a power law.
    pub gamma: Option<f64>,
}

const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Parses the base block of an EDID. `None` when the block is shorter than 128 bytes, has the wrong header, fails
/// its checksum, or declares no chromaticity.
pub fn parse(block: &[u8]) -> Option<Edid> {
    let block = block.get(..128)?;
    if block[..8] != EDID_HEADER {
        return None;
    }
    if block.iter().map(|&b| b as u32).sum::<u32>() % 256 != 0 {
        return None;
    }
    // Ten-bit coordinate: eight high bits in `msb`, two low bits at `shift` of `low`.
    let coord = |msb: usize, low: usize, shift: u32| -> f64 {
        let ten = ((block[msb] as u32) << 2) | (((block[low] >> shift) & 3) as u32);
        ten as f64 / 1024.0
    };
    let edid = Edid {
        red: (coord(27, 25, 6), coord(28, 25, 4)),
        green: (coord(29, 25, 2), coord(30, 25, 0)),
        blue: (coord(31, 26, 6), coord(32, 26, 4)),
        white: (coord(33, 26, 2), coord(34, 26, 0)),
        gamma: if block[23] == 0xFF { None } else { Some((block[23] as f64 + 100.0) / 100.0) },
    };
    let all = [edid.red, edid.green, edid.blue, edid.white];
    if all.iter().any(|&(x, y)| y <= 0.0 || x + y > 1.0 || x <= 0.0) {
        return None;
    }
    Some(edid)
}

/// The EDID blocks of connected connectors, in directory order, read from `root` (normally `/sys/class/drm`). A
/// connector counts when its `status` file reads `connected` and its `edid` file is not empty.
pub fn connected_blocks(root: &Path) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    paths
        .into_iter()
        .filter(|p| std::fs::read_to_string(p.join("status")).map(|s| s.trim() == "connected").unwrap_or(false))
        .filter_map(|p| std::fs::read(p.join("edid")).ok())
        .filter(|b| !b.is_empty())
        .collect()
}

/// The first connected display with a valid EDID, from `/sys/class/drm`.
pub fn display_edid() -> Option<Edid> {
    connected_blocks(Path::new("/sys/class/drm")).iter().find_map(|b| parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a base block with the given chromaticities (CIE xy) and a valid checksum.
    fn block_with(points: [(f64, f64); 4], gamma_byte: u8) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..8].copy_from_slice(&EDID_HEADER);
        b[23] = gamma_byte;
        let ten = |v: f64| ((v * 1024.0).round() as u16).min(1023);
        for (i, (x, y)) in points.iter().enumerate() {
            let (x, y) = (ten(*x), ten(*y));
            // Order: red, green, blue, white. Each pair's high bits are at 27 + 2i (x) and 28 + 2i (y).
            b[27 + 2 * i] = (x >> 2) as u8;
            b[28 + 2 * i] = (y >> 2) as u8;
            let low_byte = if i < 2 { 25 } else { 26 };
            let shifts = if i % 2 == 0 { (6, 4) } else { (2, 0) };
            b[low_byte] |= (((x & 3) as u8) << shifts.0) | (((y & 3) as u8) << shifts.1);
        }
        let sum: u32 = b[..127].iter().map(|&v| v as u32).sum();
        b[127] = ((256 - sum % 256) % 256) as u8;
        b
    }

    #[test]
    fn srgb_chromaticities_parse_back() {
        let b = block_with([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)], 0x78);
        let e = parse(&b).expect("valid EDID");
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(close(e.red, (0.64, 0.33)), "{:?}", e.red);
        assert!(close(e.green, (0.30, 0.60)), "{:?}", e.green);
        assert!(close(e.blue, (0.15, 0.06)), "{:?}", e.blue);
        assert!(close(e.white, (0.3127, 0.3290)), "{:?}", e.white);
        assert!((e.gamma.unwrap() - 2.2).abs() < 1e-9);
    }

    #[test]
    fn a_wide_gamut_panel_with_a_non_power_curve_has_no_gamma() {
        let b = block_with([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060), (0.3127, 0.3290)], 0xFF);
        let e = parse(&b).unwrap();
        assert!((e.red.0 - 0.680).abs() < 1e-3);
        assert_eq!(e.gamma, None);
    }

    #[test]
    fn a_bad_checksum_or_header_is_refused() {
        let mut b = block_with([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)], 0x78);
        b[127] = b[127].wrapping_add(1);
        assert!(parse(&b).is_none());
        let mut c = block_with([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)], 0x78);
        c[1] = 0;
        assert!(parse(&c).is_none());
        assert!(parse(&[0u8; 64]).is_none());
    }
}
