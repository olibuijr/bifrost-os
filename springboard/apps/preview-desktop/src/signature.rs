//! The signature library. The Sign tool draws a signature as strokes on a page. Save Signature keeps the strokes of the
//! shown page here, and Insert Signature places a saved one on a page. A signature is kept as its strokes scaled into
//! the unit square, with the shape of its box (height over width), so it can be placed at any size. The library is one
//! line per signature under the app's data directory.

use crate::model::{decode_ink, encode_ink};
use std::path::{Path, PathBuf};

/// The most signatures the library keeps. Saving one more drops the oldest.
pub const MAX_SIGNATURES: usize = 9;

/// A saved signature: its strokes in the unit square, and the height of its box over its width.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub strokes: Vec<Vec<(f32, f32)>>,
    pub aspect: f32,
}

/// The signature for the strokes of a page, scaled into the unit square together so they keep their places to one
/// another. Strokes of fewer than two points (a dot) are left out. None when no stroke is left.
pub fn from_strokes(strokes: &[Vec<(f32, f32)>]) -> Option<Signature> {
    let strokes: Vec<&Vec<(f32, f32)>> = strokes.iter().filter(|s| s.len() >= 2).collect();
    if strokes.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in strokes.iter().flat_map(|s| s.iter()) {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    // a signature with no width (one straight vertical stroke) or no height keeps its one dimension and a unit on the other
    let w = if x1 > x0 { x1 - x0 } else { 1.0 };
    let h = if y1 > y0 { y1 - y0 } else { 1.0 };
    let strokes: Vec<Vec<(f32, f32)>> = strokes.iter().map(|s| s.iter().map(|&(x, y)| ((x - x0) / w, (y - y0) / h)).collect()).collect();
    Some(Signature { strokes, aspect: h / w })
}

/// The strokes of `sig` placed in the box at (`x`, `y`) of size `w` by `h`, in the same coordinates as the box.
pub fn place(sig: &Signature, x: f32, y: f32, w: f32, h: f32) -> Vec<Vec<(f32, f32)>> {
    sig.strokes.iter().map(|s| s.iter().map(|&(u, v)| (x + u * w, y + v * h)).collect()).collect()
}

/// The strokes of `sig` centred on (`cx`, `cy`) at `width` wide, keeping the signature's shape.
pub fn centred(sig: &Signature, cx: f32, cy: f32, width: f32) -> Vec<Vec<(f32, f32)>> {
    let height = width * sig.aspect;
    place(sig, cx - width / 2.0, cy - height / 2.0, width, height)
}

/// Adds `sig` as the newest signature, dropping the oldest when the library is full.
pub fn add(library: &mut Vec<Signature>, sig: Signature) {
    library.push(sig);
    if library.len() > MAX_SIGNATURES {
        library.remove(0);
    }
}

/// The library as text: one line per signature, the aspect, a tab, then the strokes separated by semicolons.
pub fn to_text(library: &[Signature]) -> String {
    library
        .iter()
        .map(|s| {
            let strokes: Vec<String> = s.strokes.iter().map(|st| encode_ink(st)).collect();
            format!("{}\t{}\n", s.aspect, strokes.join(";"))
        })
        .collect()
}

/// Reads the library back. A line that is not a signature (no aspect, or no stroke with two points) is skipped.
pub fn parse(text: &str) -> Vec<Signature> {
    let mut library: Vec<Signature> = text
        .lines()
        .filter_map(|line| {
            let (aspect, body) = line.split_once('\t')?;
            let aspect = aspect.trim().parse::<f32>().ok().filter(|a| a.is_finite() && *a > 0.0)?;
            let strokes: Vec<Vec<(f32, f32)>> = body.split(';').map(decode_ink).filter(|s| s.len() >= 2).collect();
            (!strokes.is_empty()).then_some(Signature { strokes, aspect })
        })
        .collect();
    let excess = library.len().saturating_sub(MAX_SIGNATURES);
    library.drain(..excess);
    library
}

/// Where the library is kept.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("signatures.txt")
}

/// The saved signatures, oldest first. None saved gives an empty list.
pub fn load(data_dir: &Path) -> Vec<Signature> {
    std::fs::read_to_string(path(data_dir)).map(|t| parse(&t)).unwrap_or_default()
}

/// Saves the library under `data_dir`. An empty library removes the file.
pub fn save(data_dir: &Path, library: &[Signature]) -> std::io::Result<()> {
    if library.is_empty() {
        return match std::fs::remove_file(path(data_dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(path(data_dir), to_text(library))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(aspect: f32) -> Signature {
        Signature { strokes: vec![vec![(0.0, 0.0), (1.0, 1.0)]], aspect }
    }

    /// Whether two stroke lists match to within rounding.
    fn close(a: &[Vec<(f32, f32)>], b: &[Vec<(f32, f32)>]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(s, t)| s.len() == t.len() && s.iter().zip(t).all(|(p, q)| (p.0 - q.0).abs() < 1e-5 && (p.1 - q.1).abs() < 1e-5))
    }

    #[test]
    fn strokes_are_scaled_together_into_the_unit_square_and_keep_their_shape() {
        // a 0.2 by 0.1 signature of two strokes starting at (0.4, 0.3)
        let s = from_strokes(&[vec![(0.4, 0.3), (0.6, 0.4)], vec![(0.5, 0.35), (0.6, 0.3)]]).expect("a signature");
        assert!(close(&s.strokes, &[vec![(0.0, 0.0), (1.0, 1.0)], vec![(0.5, 0.5), (1.0, 0.0)]]), "{:?}", s.strokes);
        assert!((s.aspect - 0.5).abs() < 1e-5);
    }

    #[test]
    fn dots_are_left_out_and_nothing_left_is_no_signature() {
        assert!(from_strokes(&[vec![(0.1, 0.1)]]).is_none(), "a dot is not a signature");
        let s = from_strokes(&[vec![(0.1, 0.1)], vec![(0.5, 0.2), (0.5, 0.6)]]).expect("the line");
        assert_eq!(s.strokes.len(), 1);
        assert!(close(&s.strokes, &[vec![(0.0, 0.0), (0.0, 1.0)]]), "a straight stroke keeps its length");
    }

    #[test]
    fn a_signature_is_centred_at_the_width_asked_for() {
        let s = Signature { strokes: vec![vec![(0.0, 0.0), (1.0, 1.0)]], aspect: 0.5 };
        assert!(close(&centred(&s, 0.5, 0.5, 0.4), &[vec![(0.3, 0.4), (0.7, 0.6)]]));
    }

    #[test]
    fn the_library_keeps_the_newest_nine() {
        let mut library = Vec::new();
        for i in 0..(MAX_SIGNATURES + 2) {
            add(&mut library, sig(i as f32 + 1.0));
        }
        assert_eq!(library.len(), MAX_SIGNATURES);
        assert_eq!(library[0].aspect, 3.0, "the two oldest are gone");
    }

    #[test]
    fn the_library_round_trips_through_its_text_and_skips_damaged_lines() {
        let library = vec![
            sig(0.5),
            Signature { strokes: vec![vec![(0.25, 0.0), (0.5, 1.0)], vec![(0.0, 0.5), (1.0, 0.5)]], aspect: 2.0 },
        ];
        let text = to_text(&library);
        assert_eq!(parse(&text), library);
        let damaged = format!("nonsense\n-1\t0,0 1,1\n0.5\t0,0\n{text}");
        assert_eq!(parse(&damaged), library);
    }

    #[test]
    fn a_library_is_saved_under_the_data_folder_and_removed_when_empty() {
        let dir = std::env::temp_dir().join(format!("viewer-signatures-{}", std::process::id()));
        assert!(load(&dir).is_empty());
        save(&dir, &[sig(0.5)]).expect("saved");
        assert_eq!(load(&dir), vec![sig(0.5)]);
        save(&dir, &[]).expect("removed");
        assert!(load(&dir).is_empty());
        save(&dir, &[]).expect("removing nothing is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
