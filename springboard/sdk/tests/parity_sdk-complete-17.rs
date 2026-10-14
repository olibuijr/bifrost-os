//! Parity sdk-complete-17: headless tests for the app-facing font API. Family substitution maps a reference family
//! to open equivalents, the design variants change the requested family, the cascade lists script faces, and
//! `UserFonts` reads the packed system and monospaced faces. The fonts come from the packed asset directory;
//! nothing draws and no display is involved.

use bifrost_app::font_descriptor::UserFonts;
use bifrost_app::font_substitution::{candidates, cascade_list, class_of_family, substitute, with_design, FamilyClass, SystemDesign};
use bifrost_render::fontobj::FontSpec;

const ASSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets");

#[test]
fn a_reference_sans_family_substitutes_to_the_open_grotesque_faces() {
    assert_eq!(substitute("Helvetica"), vec!["Inter", "Geist", "Source Sans 3"]);
    assert_eq!(class_of_family("  Helvetica Neue "), Some(FamilyClass::GrotesqueSans), "case and spaces are ignored");
}

#[test]
fn a_reference_serif_family_substitutes_to_the_open_transitional_faces() {
    assert_eq!(substitute("Times New Roman").first().copied(), Some("Noto Serif"));
    assert_eq!(class_of_family("Times New Roman"), Some(FamilyClass::TransitionalSerif));
}

#[test]
fn a_family_the_table_does_not_know_has_no_substitute() {
    assert!(substitute("No Such Font").is_empty());
    assert_eq!(class_of_family("No Such Font"), None);
}

#[test]
fn a_reference_monospaced_family_substitutes_to_the_open_fixed_pitch_faces() {
    assert_eq!(substitute("Menlo").first().copied(), Some("JetBrains Mono"));
}

#[test]
fn candidates_are_the_open_equivalents_of_the_requested_family_in_order() {
    let spec = FontSpec::new().family("Helvetica");
    let families: Vec<Option<String>> = candidates(&spec).into_iter().map(|s| s.family).collect();
    assert_eq!(
        families,
        vec![Some("Inter".to_string()), Some("Geist".to_string()), Some("Source Sans 3".to_string())]
    );
}

#[test]
fn an_unknown_family_falls_back_to_the_generic_class_of_its_traits() {
    let sans = candidates(&FontSpec::new().family("Unknown Face"));
    assert_eq!(sans.first().and_then(|s| s.family.clone()), Some("Inter".to_string()));

    let fixed = candidates(&FontSpec::new().family("Unknown Face").mono());
    assert_eq!(fixed.first().and_then(|s| s.family.clone()), Some("JetBrains Mono".to_string()));
}

#[test]
fn a_spec_without_a_family_has_no_candidates() {
    assert!(candidates(&FontSpec::new()).is_empty());
}

#[test]
fn the_monospaced_design_requests_a_fixed_pitch_face() {
    let spec = with_design(FontSpec::new(), SystemDesign::Monospaced);
    assert_eq!(spec.family.as_deref(), Some("JetBrains Mono"));
    assert!(spec.traits.is_mono(), "the monospaced design also sets the fixed-pitch trait");
}

#[test]
fn the_rounded_and_serif_designs_request_their_first_open_equivalent() {
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Rounded).family.as_deref(), Some("Nunito"));
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Serif).family.as_deref(), Some("Noto Serif"));
}

#[test]
fn the_default_design_leaves_the_spec_unchanged() {
    let spec = FontSpec::new().family("Inter").size(13.0);
    assert_eq!(with_design(spec.clone(), SystemDesign::Default), spec);
}

#[test]
fn the_cascade_for_japanese_and_traditional_chinese_lists_each_script_once() {
    assert_eq!(
        cascade_list(&["ja", "ja-JP", "zh-Hant"]),
        vec!["Noto Sans CJK JP", "Noto Serif CJK JP", "Noto Sans CJK TC", "Noto Serif CJK TC"]
    );
}

#[test]
fn the_cascade_takes_the_primary_language_of_an_underscored_tag() {
    assert_eq!(cascade_list(&["ar_EG"]), vec!["Noto Naskh Arabic", "Noto Sans Arabic"]);
}

#[test]
fn an_unknown_language_tag_adds_no_cascade_faces() {
    assert!(cascade_list(&["xx"]).is_empty());
}

#[test]
fn the_system_font_object_is_the_packed_system_face_at_the_requested_size() {
    let o = UserFonts::system_object(ASSET_DIR, 16.0).expect("the packed system face loads");
    assert_eq!(o.postscript_name(), "Inter-Regular");
    assert_eq!(o.size(), 16.0);
}

#[test]
fn the_monospaced_font_object_is_the_packed_monospaced_face() {
    let o = UserFonts::mono_object(ASSET_DIR, 12.0).expect("the packed monospaced face loads");
    assert_eq!(o.postscript_name(), "JetBrainsMono-Regular");
}

#[test]
fn a_system_font_object_is_missing_when_the_font_directory_has_no_face() {
    assert!(UserFonts::system_object("/nonexistent-bifrost-font-dir", 12.0).is_none());
}

#[test]
fn the_system_path_is_the_packed_face_when_no_override_is_saved() {
    assert!(UserFonts::system_path(ASSET_DIR).ends_with("SFUI.ttf"));
}
