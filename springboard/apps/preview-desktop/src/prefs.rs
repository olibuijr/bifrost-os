//! The Viewer's settings, kept in the app's user defaults, and the signature library, kept beside the markup. The
//! settings are plain values here, so the tests read and write an in-memory store.

use crate::model::{self, Mark, MarkKind, Norm, MARK_BLUE};
use bifrost_app::defaults::UserDefaults;
use std::path::{Path, PathBuf};

const KEY_AUTHOR: &str = "viewer.annotationAuthor";
const KEY_FORGET_PAGE: &str = "viewer.forgetLastPage";
const KEY_ACTUAL_SIZE: &str = "viewer.openAtActualSize";
const KEY_HIDE_SIDEBAR: &str = "viewer.hideSidebarOnOpen";
const KEY_QUALITY: &str = "viewer.jpegQuality";

/// The JPEG qualities the export menu offers: label and quality out of 100.
pub const QUALITIES: [(&str, u8); 3] = [("High", 90), ("Medium", 75), ("Low", 50)];

/// The settings of the Viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The name written as the author of each annotation. Empty writes none.
    pub author: String,
    /// Whether a document opens at the page it was last left on.
    pub remember_page: bool,
    /// Whether a picture opens at actual size instead of fitted to the window.
    pub actual_size: bool,
    /// Whether the sidebar is shown when a document opens.
    pub sidebar_on_open: bool,
    /// The JPEG export quality, 1 to 100.
    pub jpeg_quality: u8,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { author: String::new(), remember_page: true, actual_size: false, sidebar_on_open: true, jpeg_quality: 90 }
    }
}

impl Prefs {
    /// The settings stored in `defaults`; a key that was never set keeps its default.
    pub fn load(defaults: &UserDefaults) -> Prefs {
        let mut p = Prefs::default();
        if let Some(author) = defaults.string(KEY_AUTHOR) {
            p.author = author;
        }
        p.remember_page = !defaults.bool(KEY_FORGET_PAGE);
        p.actual_size = defaults.bool(KEY_ACTUAL_SIZE);
        p.sidebar_on_open = !defaults.bool(KEY_HIDE_SIDEBAR);
        if defaults.contains(KEY_QUALITY) {
            p.jpeg_quality = defaults.int(KEY_QUALITY).clamp(1, 100) as u8;
        }
        p
    }

    /// Writes the settings to `defaults`.
    pub fn save(&self, defaults: &mut UserDefaults) {
        defaults.set_string(KEY_AUTHOR, &self.author);
        defaults.set_bool(KEY_FORGET_PAGE, !self.remember_page);
        defaults.set_bool(KEY_ACTUAL_SIZE, self.actual_size);
        defaults.set_bool(KEY_HIDE_SIDEBAR, !self.sidebar_on_open);
        defaults.set_int(KEY_QUALITY, self.jpeg_quality as i64);
    }
}

/// The most signatures the library keeps; the oldest goes first.
pub const MAX_SIGNATURES: usize = 6;

/// The file of the signature library under `data_dir`.
fn library_path(data_dir: &Path) -> PathBuf {
    data_dir.join("signatures.markup")
}

/// The saved signatures, oldest first. A missing or unreadable file gives none.
pub fn load_signatures(data_dir: &Path) -> Vec<Mark> {
    std::fs::read_to_string(library_path(data_dir)).map(|t| model::parse_sidecar(&t)).unwrap_or_default()
}

/// Saves the signature library.
pub fn save_signatures(data_dir: &Path, signatures: &[Mark]) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(library_path(data_dir), model::to_sidecar(signatures))
}

/// The bounding rectangle of `ink`.
fn bounds(ink: &[(f32, f32)]) -> Option<Norm> {
    let first = *ink.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for &(x, y) in ink {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Some(Norm { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// A signature to keep from a stroke drawn on a page whose width over height is `aspect`: the points are moved so
/// their centre is the origin and widened by the aspect, which makes them independent of the page they were drawn on.
/// None for a stroke with fewer than two points or no extent.
pub fn library_entry(ink: &[(f32, f32)], aspect: f32) -> Option<Mark> {
    let r = bounds(ink)?;
    if ink.len() < 2 || (r.w <= 0.0 && r.h <= 0.0) || aspect <= 0.0 {
        return None;
    }
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let points: Vec<(f32, f32)> = ink.iter().map(|&(x, y)| ((x - cx) * aspect, y - cy)).collect();
    let rect = bounds(&points)?;
    Some(Mark { kind: MarkKind::Ink, page: 0, rect, color: MARK_BLUE, text: String::new(), ink: points })
}

/// The stroke of library entry `entry` placed on source page `page`, centred at `centre` and `width` of the page wide
/// (`aspect` is the page's width over height).
pub fn place_signature(entry: &Mark, page: usize, centre: (f32, f32), width: f32, aspect: f32) -> Mark {
    let units = entry.rect.w.max(1e-6);
    // the entry is in height units: its width in them, against the wanted width in the same units
    let scale = width * aspect / units;
    let ink: Vec<(f32, f32)> = entry
        .ink
        .iter()
        .map(|&(x, y)| ((centre.0 + x * scale / aspect).clamp(0.0, 1.0), (centre.1 + y * scale).clamp(0.0, 1.0)))
        .collect();
    let rect = bounds(&ink).unwrap_or(Norm { x: centre.0, y: centre.1, w: 0.0, h: 0.0 });
    Mark { kind: MarkKind::Ink, page, rect, color: entry.color, text: String::new(), ink }
}

/// `library` with `entry` added at the end, the oldest dropped past the limit.
pub fn add_signature(library: &mut Vec<Mark>, entry: Mark) {
    library.push(entry);
    while library.len() > MAX_SIGNATURES {
        library.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_then_round_trip() {
        let mut d = UserDefaults::memory();
        assert_eq!(Prefs::load(&d), Prefs::default());
        let p = Prefs { author: "Sigga".into(), remember_page: false, actual_size: true, sidebar_on_open: false, jpeg_quality: 50 };
        p.save(&mut d);
        assert_eq!(Prefs::load(&d), p);
    }

    #[test]
    fn a_stored_quality_is_kept_inside_range() {
        let mut d = UserDefaults::memory();
        d.set_int(KEY_QUALITY, 900);
        assert_eq!(Prefs::load(&d).jpeg_quality, 100);
        d.set_int(KEY_QUALITY, -4);
        assert_eq!(Prefs::load(&d).jpeg_quality, 1);
    }

    #[test]
    fn a_library_entry_is_centred_and_needs_a_real_stroke() {
        assert!(library_entry(&[(0.5, 0.5)], 1.0).is_none());
        assert!(library_entry(&[(0.5, 0.5), (0.5, 0.5)], 1.0).is_none());
        let e = library_entry(&[(0.2, 0.4), (0.6, 0.6)], 2.0).expect("an entry");
        assert!((e.rect.x + 0.4).abs() < 1e-6 && (e.rect.w - 0.8).abs() < 1e-6);
        assert!((e.rect.y + 0.1).abs() < 1e-6 && (e.rect.h - 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_signature_keeps_its_shape_on_a_page_of_another_shape() {
        let e = library_entry(&[(0.2, 0.4), (0.6, 0.6)], 1.0).expect("an entry");
        // the stroke is twice as wide as tall; on a square page 0.3 wide it is 0.3 by 0.15
        let m = place_signature(&e, 3, (0.5, 0.5), 0.3, 1.0);
        assert_eq!(m.page, 3);
        assert!((m.rect.w - 0.3).abs() < 1e-5 && (m.rect.h - 0.15).abs() < 1e-5, "{:?}", m.rect);
        assert!((m.rect.x + m.rect.w / 2.0 - 0.5).abs() < 1e-5);
        // on a page twice as wide as tall the same shape is as tall, in page fractions, as it is wide
        let wide = place_signature(&e, 0, (0.5, 0.5), 0.3, 2.0);
        assert!((wide.rect.w - 0.3).abs() < 1e-5);
        assert!((wide.rect.h - 0.3).abs() < 1e-5, "{:?}", wide.rect);
    }

    #[test]
    fn the_library_keeps_the_newest_few() {
        let mut lib = Vec::new();
        for i in 0..MAX_SIGNATURES + 2 {
            let mut e = library_entry(&[(0.0, 0.0), (1.0, 1.0)], 1.0).expect("an entry");
            e.page = i;
            add_signature(&mut lib, e);
        }
        assert_eq!(lib.len(), MAX_SIGNATURES);
        assert_eq!(lib[0].page, 2);
    }

    #[test]
    fn the_library_survives_a_save() {
        let dir = std::env::temp_dir().join(format!("viewer-signatures-{}", std::process::id()));
        let e = library_entry(&[(0.1, 0.1), (0.3, 0.2), (0.5, 0.1)], 1.5).expect("an entry");
        save_signatures(&dir, std::slice::from_ref(&e)).expect("saved");
        assert_eq!(load_signatures(&dir), vec![e]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_signatures(&dir).is_empty());
    }
}
