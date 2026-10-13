//! The user's font choice (`UserFonts` is the app-facing side of Settings > Fonts). Font descriptors are
//! `bifrost_render::fontobj::FontSpec`; `UserFonts::resolve` matches one against the user's font library with the
//! same `FontManager` matching the rest of the system uses.
//!
//! Settings > Fonts replaces the system and monospaced fonts with a file from BifrOSt's font library. `UserFonts`
//! reports which file an app's text uses, so apps draw with the same face the shell does.
use bifrost_render::fontobj::{FontManager, FontSpec};
use bifrost_render::fontprefs::{self, FontRole, Installed};

/// The fonts the user chose for the system and monospaced roles, resolved from the shell's data directory.
pub struct UserFonts;

impl UserFonts {
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
}
