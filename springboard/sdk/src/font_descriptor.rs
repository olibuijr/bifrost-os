//! The user's font choice (`UserFonts` is the app-facing side of Settings > Fonts). Font descriptors are
//! `bifrost_render::fontobj::FontSpec`; `UserFonts::resolve` matches one against the user's font library with the
//! same `FontManager` matching the rest of the system uses.
//!
//! Settings > Fonts replaces the system and monospaced fonts with a file from BifrOSt's font library. `UserFonts`
//! reports which file an app's text uses, so apps draw with the same face the shell does.
use bifrost_render::fontobj::{FontManager, FontObject, FontSpec};
use bifrost_render::fontprefs::{self, FontRole, Installed};

use crate::font_substitution::{self, SystemDesign};

/// The fonts the user chose for the system and monospaced roles, resolved from the shell's data directory.
pub struct UserFonts;

impl UserFonts {
    /// The system role as a font object at `size` points: the file `system_path` names, read and parsed. None when
    /// the file is missing or is not a font.
    pub fn system_object(shell_dir: &str, size: f32) -> Option<FontObject> {
        Self::object_from_file(&Self::system_path(shell_dir), size)
    }

    /// The monospaced role as a font object at `size` points (see `system_object`).
    pub fn mono_object(shell_dir: &str, size: f32) -> Option<FontObject> {
        Self::object_from_file(&Self::mono_path(shell_dir), size)
    }

    /// The best face in the font library for `spec`, as a font object at the spec's size (12 when unset). Scans the
    /// library on each call; apps that resolve often should keep the object.
    pub fn library_object(shell_dir: &str, spec: &FontSpec) -> Option<FontObject> {
        let mut m = FontManager::new();
        m.scan_dir(&fontprefs::library(shell_dir), 2);
        FontObject::from_spec(&m, spec)
    }

    fn object_from_file(path: &str, size: f32) -> Option<FontObject> {
        FontObject::from_data(std::fs::read(path).ok()?, size, None)
    }

    /// The font file the system role draws with: the Settings override when it exists, else the packed asset.
    pub fn system_path(shell_dir: &str) -> String {
        fontprefs::font_path(shell_dir, FontRole::System)
    }

    /// The font file the monospaced role draws with.
    pub fn mono_path(shell_dir: &str) -> String {
        fontprefs::font_path(shell_dir, FontRole::Mono)
    }

    /// Whether the user replaced `role` with a font from the library (the file is not the packed asset).
    pub fn is_replaced(shell_dir: &str, role: FontRole) -> bool {
        fontprefs::font_path(shell_dir, role) != format!("{shell_dir}/{}", role.file())
    }

    /// Every face the user can choose from: BifrOSt's font library, one face per family, sorted by name.
    pub fn choices(shell_dir: &str) -> Vec<Installed> {
        fontprefs::installed(&[fontprefs::library(shell_dir)])
    }

    /// Every face in the font library that `spec` matches, best first. Names and fixed pitch must hold; weight,
    /// width, slant and the other traits only rank. When the named family is not in the library, the open
    /// equivalents of its class (`font_substitution`) are tried in preference order, so a document that names a
    /// family BifrOSt does not ship still gets a face of the same style. Only the library is read (BifrOSt's packed
    /// open faces and the fonts apps install), never any other font folder.
    pub fn resolve(shell_dir: &str, spec: &FontSpec) -> Vec<Installed> {
        let mut m = FontManager::new();
        m.scan_dir(&fontprefs::library(shell_dir), 2);
        let direct = Self::installed_for(&m, spec);
        if !direct.is_empty() {
            return direct;
        }
        font_substitution::candidates(spec).iter().map(|c| Self::installed_for(&m, c)).find(|r| !r.is_empty()).unwrap_or_default()
    }

    /// `spec` with a system design applied (`.rounded`, `.serif`, `.monospaced`): the descriptor's family becomes the
    /// design's open equivalent, and `resolve` falls through the class list when that one is not installed.
    pub fn with_design(spec: FontSpec, design: SystemDesign) -> FontSpec {
        font_substitution::with_design(spec, design)
    }

    /// The ordered fallback families for characters a face lacks, for language tags such as `ja`, `zh-Hant`, `ar`.
    pub fn cascade_list(languages: &[&str]) -> Vec<&'static str> {
        font_substitution::cascade_list(languages)
    }

    fn installed_for(m: &FontManager, spec: &FontSpec) -> Vec<Installed> {
        m.matching(spec)
            .into_iter()
            .filter_map(|i| {
                let f = &m.faces()[i];
                Some(Installed { family: f.record.family.clone(), path: f.source.path()?.to_string(), mono: f.record.traits.is_mono(), variable: f.record.variable })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreplaced_role_reports_the_packed_asset() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-descriptor-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let shell = dir.to_string_lossy().into_owned();
        assert_eq!(UserFonts::system_path(&shell), format!("{shell}/SFUI.ttf"));
        assert_eq!(UserFonts::mono_path(&shell), format!("{shell}/SFUIMono.ttf"));
        assert!(!UserFonts::is_replaced(&shell, FontRole::System));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_finds_nothing_in_an_empty_library() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-resolve-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let shell = dir.to_string_lossy().into_owned();
        assert!(UserFonts::resolve(&shell, &FontSpec::new()).is_empty());
        assert!(UserFonts::resolve(&shell, &FontSpec::new().family("Alpha").mono()).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sf_bytes() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf")
    }

    #[test]
    fn system_object_reads_the_packed_face_at_the_requested_size() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-object-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let shell = dir.to_string_lossy().into_owned();
        // no packed file in the scratch folder: nothing to return, and no panic
        assert!(UserFonts::system_object(&shell, 13.0).is_none());
        std::fs::write(dir.join("SFUI.ttf"), sf_bytes()).unwrap();
        let f = UserFonts::system_object(&shell, 13.0).expect("packed face parses");
        assert_eq!(f.size(), 13.0);
        assert!(f.text_width("Hello") > 0.0);
        assert!(f.with_size(26.0).text_width("Hello") > f.text_width("Hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_substitutes_an_installed_equivalent_for_a_missing_family() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-substitute-{}", std::process::id()));
        let lib = dir.join("fonts");
        let _ = std::fs::create_dir_all(&lib);
        let shell = dir.to_string_lossy().into_owned();
        std::fs::write(lib.join("SFUI.ttf"), sf_bytes()).unwrap();
        let own = UserFonts::choices(&shell).first().expect("library face").family.clone();
        // an installed family resolves to itself
        assert_eq!(UserFonts::resolve(&shell, &FontSpec::new().family(&own))[0].family, own);
        // a family BifrOSt does not ship, whose class lists the installed family, resolves to it
        if font_substitution::substitute("Helvetica Neue").contains(&own.as_str()) {
            assert_eq!(UserFonts::resolve(&shell, &FontSpec::new().family("Helvetica Neue"))[0].family, own);
        }
        // a mono request finds no substitute among a lone proportional face
        assert!(UserFonts::resolve(&shell, &FontSpec::new().family("Menlo").mono()).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_reads_only_the_font_library() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-library-only-{}", std::process::id()));
        let lib = dir.join("fonts");
        let beside = dir.join("other").join("fonts");
        let _ = std::fs::create_dir_all(&lib);
        let _ = std::fs::create_dir_all(&beside);
        let shell = dir.to_string_lossy().into_owned();
        std::fs::write(lib.join("SFUI.ttf"), sf_bytes()).unwrap();
        // font folders beside the library are never read, whatever their names
        std::fs::write(beside.join("SFUI.ttf"), sf_bytes()).unwrap();
        std::fs::write(dir.join("SFUI.ttf"), sf_bytes()).unwrap();
        let own = UserFonts::choices(&shell).first().expect("library face").family.clone();
        for spec in [FontSpec::new().family(&own), FontSpec::new()] {
            let hits = UserFonts::resolve(&shell, &spec);
            assert!(!hits.is_empty());
            for hit in &hits {
                assert!(std::path::Path::new(&hit.path).starts_with(&lib), "{} is outside the library", hit.path);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn design_and_cascade_helpers_delegate_to_the_substitution_tables() {
        assert!(UserFonts::with_design(FontSpec::new(), SystemDesign::Monospaced).traits.is_mono());
        assert_eq!(UserFonts::cascade_list(&["ja"])[0], "Noto Sans CJK JP");
    }

    #[test]
    fn library_object_matches_a_face_from_the_library() {
        let dir = std::env::temp_dir().join(format!("bifrost-font-library-object-{}", std::process::id()));
        let lib = dir.join("fonts");
        let _ = std::fs::create_dir_all(&lib);
        let shell = dir.to_string_lossy().into_owned();
        assert!(UserFonts::library_object(&shell, &FontSpec::new()).is_none());
        std::fs::write(lib.join("SFUI.ttf"), sf_bytes()).unwrap();
        let f = UserFonts::library_object(&shell, &FontSpec::new().size(20.0)).expect("library face");
        assert_eq!(f.size(), 20.0);
        assert!(!f.family_name().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
