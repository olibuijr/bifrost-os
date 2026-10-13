//! The user's font choice (`UserFonts` is the app-facing side of Settings > Fonts). Font descriptors are
//! `bifrost_render::fontobj::FontSpec`; `UserFonts::resolve` matches one against the user's font library with the
//! same `FontManager` matching the rest of the system uses.
//!
//! Settings > Fonts replaces the system and monospaced fonts with a file from BifrOSt's font library. `UserFonts`
//! reports which file an app's text uses, so apps draw with the same face the shell does.
use bifrost_render::fontobj::{FontManager, FontObject, FontSpec};
use bifrost_render::fontprefs::{self, FontRole, Installed};

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
    /// width, slant and the other traits only rank.
    pub fn resolve(shell_dir: &str, spec: &FontSpec) -> Vec<Installed> {
        let mut m = FontManager::new();
        m.scan_dir(&fontprefs::library(shell_dir), 2);
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
