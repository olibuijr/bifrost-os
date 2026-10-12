//! Font descriptors and the user's font choice (`FontDescriptor` mirrors the reference SDK's font descriptor type;
//! `UserFonts` is the app-facing side of Settings > Fonts).
//!
//! A descriptor is a set of mandatory attributes (family name, point size, fixed pitch). `matching` returns the
//! installed faces that satisfy them, in library order. The renderer draws whole files, so the match is on the
//! family and the pitch only: `fontprefs::installed` keeps one face per family, and the face's weight and slant
//! are not known here.
//!
//! Settings > Fonts replaces the system and monospaced fonts with a file from BifrOSt's font library. `UserFonts`
//! reports which file an app's text uses, so apps draw with the same face the shell does.
use bifrost_render::fontprefs::{self, FontRole, Installed};

/// Mandatory attributes for a font lookup. An empty descriptor matches every installed face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontDescriptor {
    family: Option<String>,
    size: Option<f32>,
    fixed_pitch: bool,
}

impl FontDescriptor {
    /// A descriptor with no mandatory attributes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requires the family `name`. An empty name clears the requirement.
    pub fn with_family(mut self, name: &str) -> Self {
        let name = name.trim();
        self.family = (!name.is_empty()).then(|| name.to_string());
        self
    }

    /// Sets the point size. A size that is not finite or not above zero clears it.
    pub fn with_size(mut self, pt: f32) -> Self {
        self.size = (pt.is_finite() && pt > 0.0).then_some(pt);
        self
    }

    /// Requires (or stops requiring) a fixed-pitch face.
    pub fn with_fixed_pitch(mut self, on: bool) -> Self {
        self.fixed_pitch = on;
        self
    }

    pub fn family(&self) -> Option<&str> {
        self.family.as_deref()
    }

    pub fn point_size(&self) -> Option<f32> {
        self.size
    }

    pub fn is_fixed_pitch(&self) -> bool {
        self.fixed_pitch
    }

    /// Whether the face `font` satisfies the family and pitch requirements. The family compares without case.
    pub fn accepts(&self, font: &Installed) -> bool {
        let family_ok = self.family.as_deref().is_none_or(|f| f.eq_ignore_ascii_case(&font.family));
        family_ok && (!self.fixed_pitch || font.mono)
    }

    /// Every face in `faces` that satisfies the descriptor, in the order given.
    pub fn matching<'a>(&self, faces: &'a [Installed]) -> Vec<&'a Installed> {
        faces.iter().filter(|f| self.accepts(f)).collect()
    }

    /// The first face in `faces` that satisfies the descriptor, if any.
    pub fn first_match<'a>(&self, faces: &'a [Installed]) -> Option<&'a Installed> {
        faces.iter().find(|f| self.accepts(f))
    }
}

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

    /// The faces a descriptor matches among the user's choices.
    pub fn resolve(shell_dir: &str, descriptor: &FontDescriptor) -> Vec<Installed> {
        let choices = Self::choices(shell_dir);
        descriptor.matching(&choices).into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, mono: bool) -> Installed {
        Installed { family: family.to_string(), path: format!("/lib/{family}.ttf"), mono, variable: false }
    }

    fn library() -> Vec<Installed> {
        vec![face("Alpha", false), face("Beta Mono", true), face("gamma", false), face("Delta Mono", true)]
    }

    #[test]
    fn empty_descriptor_matches_every_face() {
        let faces = library();
        let d = FontDescriptor::new();
        assert_eq!(d.matching(&faces).len(), 4);
        assert_eq!(d.first_match(&faces).map(|f| f.family.as_str()), Some("Alpha"));
    }

    #[test]
    fn family_match_ignores_case_and_keeps_library_order() {
        let faces = library();
        let d = FontDescriptor::new().with_family("  GAMMA ");
        assert_eq!(d.family(), Some("GAMMA"));
        let hits = d.matching(&faces);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/lib/gamma.ttf");
    }

    #[test]
    fn fixed_pitch_keeps_only_monospaced_faces_in_order() {
        let faces = library();
        let d = FontDescriptor::new().with_fixed_pitch(true);
        let names: Vec<&str> = d.matching(&faces).iter().map(|f| f.family.as_str()).collect();
        assert_eq!(names, vec!["Beta Mono", "Delta Mono"]);
        assert_eq!(d.first_match(&faces).map(|f| f.family.as_str()), Some("Beta Mono"));
    }

    #[test]
    fn family_and_pitch_must_both_hold() {
        let faces = library();
        let d = FontDescriptor::new().with_family("Alpha").with_fixed_pitch(true);
        assert!(d.matching(&faces).is_empty());
        assert!(d.first_match(&faces).is_none());
    }

    #[test]
    fn size_accepts_positive_finite_values_only() {
        assert_eq!(FontDescriptor::new().with_size(13.0).point_size(), Some(13.0));
        assert_eq!(FontDescriptor::new().with_size(0.0).point_size(), None);
        assert_eq!(FontDescriptor::new().with_size(-4.0).point_size(), None);
        assert_eq!(FontDescriptor::new().with_size(f32::NAN).point_size(), None);
        assert_eq!(FontDescriptor::new().with_size(f32::INFINITY).point_size(), None);
    }

    #[test]
    fn empty_family_clears_the_requirement() {
        let d = FontDescriptor::new().with_family("Alpha").with_family("   ");
        assert_eq!(d.family(), None);
        assert!(d.accepts(&face("Zeta", false)));
    }

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
}
