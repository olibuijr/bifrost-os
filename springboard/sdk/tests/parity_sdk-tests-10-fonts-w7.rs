//! Parity sdk-tests-10: headless tests of family substitution (`bifrost_app::font_substitution`) and of the user's font
//! choice (`bifrost_app::font_descriptor`).
//!
//! Headless: substitution is a table lookup. The font choice reads a packed face from a scratch folder under the cargo
//! target directory, so the test owns every file it looks at. No display, device or system font directory is read.

use bifrost_app::font_descriptor::UserFonts;
use bifrost_app::font_substitution::{candidates, cascade_list, class_for_spec, class_of_family, substitute, with_design, FamilyClass, SystemDesign};
use bifrost_app::fontobj::FontSpec;

fn scratch(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sdk-tests-10-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fonts")).expect("scratch folder");
    dir.to_string_lossy().into_owned()
}

fn packed_face() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf")
}

// --- family substitution -----------------------------------------------------------------------------------------------

#[test]
fn headless_a_reference_family_name_maps_to_its_class_ignoring_case_and_spacing() {
    assert_eq!(class_of_family("Helvetica Neue"), Some(FamilyClass::GrotesqueSans));
    assert_eq!(class_of_family("  HELVETICA  "), Some(FamilyClass::GrotesqueSans));
    assert_eq!(class_of_family("Futura"), Some(FamilyClass::GeometricSans));
    assert_eq!(class_of_family("Georgia"), Some(FamilyClass::TransitionalSerif));
    assert_eq!(class_of_family("Menlo"), Some(FamilyClass::Monospaced));
    assert_eq!(class_of_family("Comic Nonsense Face"), None, "an unknown name has no class");
}

#[test]
fn headless_a_known_family_substitutes_the_open_equivalents_of_its_class_in_order() {
    assert_eq!(substitute("Helvetica Neue"), vec!["TeX Gyre Heros", "Inter", "Geist", "Source Sans 3"]);
    assert_eq!(substitute("Menlo")[0], "JetBrains Mono");
    assert!(substitute("Comic Nonsense Face").is_empty());
}

#[test]
fn headless_each_class_lists_at_least_two_open_families_to_fall_back_on() {
    let classes = [
        FamilyClass::GrotesqueSans,
        FamilyClass::GeometricSans,
        FamilyClass::HumanistSans,
        FamilyClass::DidoneSerif,
        FamilyClass::TransitionalSerif,
        FamilyClass::Slab,
        FamilyClass::Script,
        FamilyClass::Monospaced,
        FamilyClass::Rounded,
        FamilyClass::CjkGothic,
        FamilyClass::CjkMincho,
        FamilyClass::CjkKai,
        FamilyClass::CjkRounded,
    ];
    for class in classes {
        assert!(class.equivalents().len() >= 2, "{class:?} has fallbacks");
    }
    assert_eq!(FamilyClass::CjkKai.equivalents()[0], "Noto Serif CJK TC");
}

#[test]
fn headless_an_unknown_family_with_a_fixed_pitch_trait_falls_back_to_a_monospaced_class() {
    assert_eq!(class_for_spec(&FontSpec::new().mono()), FamilyClass::Monospaced);
    assert_eq!(class_for_spec(&FontSpec::new()), FamilyClass::GrotesqueSans);
}

#[test]
fn headless_candidates_are_the_class_equivalents_without_the_family_itself() {
    assert!(candidates(&FontSpec::new()).is_empty(), "a descriptor with no family has no candidates");
    let names: Vec<Option<String>> = candidates(&FontSpec::new().family("Inter")).into_iter().map(|c| c.family).collect();
    assert_eq!(names, vec![Some("Geist".to_string()), Some("Source Sans 3".to_string())]);
    let futura: Vec<String> = candidates(&FontSpec::new().family("Futura")).into_iter().filter_map(|c| c.family).collect();
    assert_eq!(futura, vec!["Outfit", "Poppins", "Geist", "Inter"]);
}

#[test]
fn headless_a_system_design_applies_its_class_to_the_descriptor() {
    assert_eq!(SystemDesign::Default.class(), None);
    assert_eq!(SystemDesign::Rounded.class(), Some(FamilyClass::Rounded));
    assert_eq!(SystemDesign::Serif.class(), Some(FamilyClass::TransitionalSerif));
    assert_eq!(SystemDesign::Monospaced.class(), Some(FamilyClass::Monospaced));

    assert_eq!(with_design(FontSpec::new(), SystemDesign::Default).family, None, "the default design leaves the descriptor alone");
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Rounded).family.as_deref(), Some("Nunito"));
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Serif).family.as_deref(), Some("Noto Serif"));
}

#[test]
fn headless_the_monospaced_design_also_asks_for_a_fixed_pitch_face() {
    let mono = with_design(FontSpec::new(), SystemDesign::Monospaced);
    assert_eq!(mono.family.as_deref(), Some("JetBrains Mono"));
    assert!(mono.traits.is_mono());
    assert!(!with_design(FontSpec::new(), SystemDesign::Rounded).traits.is_mono());
}

#[test]
fn headless_the_cascade_list_gives_the_script_families_for_each_language_tag_once() {
    assert_eq!(cascade_list(&["ja"]), vec!["Noto Sans CJK JP", "Noto Serif CJK JP"]);
    assert_eq!(cascade_list(&["ko_KR"]), vec!["Noto Sans CJK KR", "Noto Serif CJK KR"], "an underscore tag is read like a dash tag");
    assert_eq!(cascade_list(&["zh-Hant-TW"])[0], "Noto Sans CJK TC", "traditional Chinese by script or region");
    assert_eq!(cascade_list(&["zh-CN"])[0], "Noto Sans CJK SC");
    assert_eq!(cascade_list(&["ja", "ja-JP"]).len(), 2, "repeats are dropped");
    assert_eq!(cascade_list(&["ar"]), vec!["Noto Naskh Arabic", "Noto Sans Arabic"]);
    assert_eq!(cascade_list(&["ja", "ar"]).len(), 4, "each language adds its own families");
    assert!(cascade_list(&["xx"]).is_empty(), "an unknown language adds nothing");
}

// --- the user's font choice --------------------------------------------------------------------------------------------

#[test]
fn headless_the_system_and_monospaced_roles_draw_with_the_packed_files_until_the_user_replaces_them() {
    let shell = scratch("roles");
    assert_eq!(UserFonts::system_path(&shell), format!("{shell}/SFUI.ttf"));
    assert_eq!(UserFonts::mono_path(&shell), format!("{shell}/SFUIMono.ttf"));
    assert!(!UserFonts::is_replaced(&shell, bifrost_render::fontprefs::FontRole::System));
    assert!(!UserFonts::is_replaced(&shell, bifrost_render::fontprefs::FontRole::Mono));
}

#[test]
fn headless_the_system_object_is_none_without_the_packed_file_and_parses_it_when_present() {
    let shell = scratch("system-object");
    assert!(UserFonts::system_object(&shell, 13.0).is_none(), "no file, no font object");
    std::fs::write(std::path::Path::new(&shell).join("SFUI.ttf"), packed_face()).unwrap();
    let face = UserFonts::system_object(&shell, 13.0).expect("the packed face parses");
    assert_eq!(face.size(), 13.0);
    assert!(face.text_width("Hello") > 0.0);
    assert!(face.with_size(26.0).text_width("Hello") > face.text_width("Hello"), "a larger size is wider");
}

#[test]
fn headless_the_font_library_lists_the_faces_the_user_can_choose() {
    let shell = scratch("choices");
    assert!(UserFonts::choices(&shell).is_empty(), "an empty library offers nothing");
    std::fs::write(std::path::Path::new(&shell).join("fonts").join("SFUI.ttf"), packed_face()).unwrap();
    let choices = UserFonts::choices(&shell);
    assert_eq!(choices.len(), 1);
    assert!(!choices[0].family.is_empty());
}

#[test]
fn headless_a_library_font_object_takes_the_requested_size() {
    let shell = scratch("library-object");
    assert!(UserFonts::library_object(&shell, &FontSpec::new()).is_none());
    std::fs::write(std::path::Path::new(&shell).join("fonts").join("SFUI.ttf"), packed_face()).unwrap();
    let face = UserFonts::library_object(&shell, &FontSpec::new().size(20.0)).expect("the library face");
    assert_eq!(face.size(), 20.0);
    assert!(!face.family_name().is_empty());
}

#[test]
fn headless_resolving_an_installed_family_returns_that_face_first() {
    let shell = scratch("resolve-installed");
    std::fs::write(std::path::Path::new(&shell).join("fonts").join("SFUI.ttf"), packed_face()).unwrap();
    let own = UserFonts::choices(&shell).first().expect("library face").family.clone();
    let found = UserFonts::resolve(&shell, &FontSpec::new().family(&own));
    assert_eq!(found.first().map(|f| f.family.as_str()), Some(own.as_str()));
}

#[test]
fn headless_resolving_in_an_empty_library_finds_no_face_and_no_substitute() {
    let shell = scratch("resolve-empty");
    assert!(UserFonts::resolve(&shell, &FontSpec::new()).is_empty());
    assert!(UserFonts::resolve(&shell, &FontSpec::new().family("Alpha").mono()).is_empty());
}

#[test]
fn headless_a_missing_reference_family_resolves_to_an_installed_equivalent_of_its_class() {
    let shell = scratch("resolve-substitute");
    std::fs::write(std::path::Path::new(&shell).join("fonts").join("SFUI.ttf"), packed_face()).unwrap();
    let own = UserFonts::choices(&shell).first().expect("library face").family.clone();
    if substitute("Helvetica Neue").contains(&own.as_str()) {
        let found = UserFonts::resolve(&shell, &FontSpec::new().family("Helvetica Neue"));
        assert_eq!(found.first().map(|f| f.family.as_str()), Some(own.as_str()));
    }
    assert!(UserFonts::resolve(&shell, &FontSpec::new().family("Menlo").mono()).is_empty(), "a lone proportional face is no fixed-pitch substitute");
}

#[test]
fn headless_the_font_descriptor_delegates_design_and_cascade_to_the_substitution_tables() {
    assert!(UserFonts::with_design(FontSpec::new(), SystemDesign::Monospaced).traits.is_mono());
    assert_eq!(UserFonts::cascade_list(&["ja"])[0], "Noto Sans CJK JP");
    assert_eq!(UserFonts::cascade_list(&["ja"]), cascade_list(&["ja"]));
}
