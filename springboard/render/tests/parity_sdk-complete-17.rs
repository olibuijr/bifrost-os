//! Parity sdk-complete-17: headless tests for the font engine (`font`, `fontobj`, `fontfallback`) and the glass
//! material recipes (`glass`). The system face is the packed variable TrueType font, read from the asset include,
//! so nothing touches a font directory, a display or a device.

use bifrost_render::font::Font;
use bifrost_render::fontfallback::split;
use bifrost_render::fontobj::{default_cascade, FontError, FontEvent, FontManager, FontObject, FontSpec, GlyphRef, Scope};
use bifrost_render::glass::{Kind, Look};

const SYSTEM_FACE: &[u8] = include_bytes!("../../assets/SFUI.ttf");

fn system_font() -> Font<'static> {
    Font::parse(SYSTEM_FACE).expect("the packed system face parses")
}

fn object(size: f32) -> FontObject {
    FontObject::from_data(SYSTEM_FACE.to_vec(), size, None).expect("the packed system face is a font object")
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// font: the parsed face, its metrics, its glyph map, its outlines and its variation axes

#[test]
fn the_system_face_has_2048_units_per_em() {
    assert_eq!(system_font().upem, 2048.0);
}

#[test]
fn line_metrics_at_the_em_size_are_the_font_table_values() {
    let m = system_font().line_metrics(2048.0);
    assert_eq!(m.ascent, 1984.0, "hhea ascent");
    assert_eq!(m.descent, 494.0, "hhea descent, reported positive");
    assert_eq!(m.leading, 0.0, "hhea line gap is zero");
    assert_eq!(m.cap_height, 1490.0, "OS/2 cap height");
    assert_eq!(m.x_height, 1118.0, "OS/2 x-height");
    assert_eq!(m.underline_position, -348.0, "post underline position");
    assert_eq!(m.underline_thickness, 140.0, "post underline thickness");
    assert_eq!(m.line_height(), 2478.0);
}

#[test]
fn line_metrics_scale_linearly_with_the_point_size() {
    let m = system_font().line_metrics(1024.0);
    assert_eq!(m.ascent, 992.0);
    assert_eq!(m.descent, 247.0);
    assert_eq!(m.cap_height, 745.0);
}

#[test]
fn a_character_maps_to_the_glyph_of_its_code_point() {
    let f = system_font();
    let a = f.glyph('A');
    assert_ne!(a, 0, "A has a glyph in the packed face");
    assert_eq!(a, f.glyph_code(0x41));
    assert_ne!(a, f.glyph('B'), "distinct letters have distinct glyphs");
}

#[test]
fn a_character_without_a_glyph_maps_to_glyph_zero() {
    let f = system_font();
    assert_eq!(f.glyph_code(0x10FFFF), 0, "no face covers the last code point");
    assert_eq!(f.glyph('\u{10FFFF}'), 0);
}

#[test]
fn glyph_names_are_only_found_in_cff_fonts() {
    assert_eq!(system_font().glyph_named("A"), None, "the packed face is TrueType, so it has no CFF names");
}

#[test]
fn the_space_glyph_has_no_outline_and_a_letter_has_one() {
    let f = system_font();
    assert!(f.glyph_segments(f.glyph(' '), 64.0, &[]).is_empty());
    let o = f.glyph_segments(f.glyph('O'), 64.0, &[]);
    assert!(!o.is_empty(), "O has contours");
    assert!(o.iter().all(|s| s.iter().all(|v| v.is_finite())), "every coordinate is finite");
}

#[test]
fn variable_axes_report_the_tag_and_the_range_of_each_axis() {
    let axes = system_font().axes().to_vec();
    assert_eq!(axes.len(), 2);
    let wght = axes.iter().find(|a| a.tag == *b"wght").expect("the face has a weight axis");
    assert_eq!((wght.min, wght.default, wght.max), (100.0, 400.0, 900.0));
}

#[test]
fn the_default_instance_advance_equals_the_static_advance() {
    let f = system_font();
    let w = f.glyph('W');
    assert_eq!(f.advance_var(w, &[(*b"wght", 400.0)]), f.advance(w));
}

#[test]
fn a_heavier_instance_widens_the_capital_w() {
    let f = system_font();
    let w = f.glyph('W');
    assert!(f.advance_var(w, &[(*b"wght", 900.0)]) > f.advance(w));
}

// fontobj: font objects, descriptors, glyph overrides, variations and metrics in points

#[test]
fn a_font_object_names_come_from_the_name_table() {
    let o = object(16.0);
    assert_eq!(o.postscript_name(), "Inter-Regular");
    assert_eq!(o.family_name(), "Inter");
    assert_eq!(o.style_name(), "Regular");
}

#[test]
fn a_font_object_reports_its_metrics_in_points_at_its_size() {
    let o = object(16.0);
    assert_eq!(o.units_per_em(), 2048);
    assert_eq!(o.ascent(), 15.5);
    assert_eq!(o.descent(), 3.859375);
    assert_eq!(o.leading(), 0.0);
    assert_eq!(o.cap_height(), 11.640625);
    assert_eq!(o.x_height(), 8.734375);
    assert_eq!(o.line_height(), 19.359375);
}

#[test]
fn a_font_object_at_a_new_size_scales_its_advances_and_keeps_its_face() {
    let a = object(16.0);
    let b = a.with_size(32.0);
    let g = a.glyph_for_char('A');
    assert_eq!(b.size(), 32.0);
    assert_eq!(b.advances(&[g])[0], 2.0 * a.advances(&[g])[0]);
    assert_eq!(a.with_size(0.0).size(), 16.0, "a non-positive size is ignored");
}

#[test]
fn a_glyph_override_replaces_the_glyph_for_that_font_object_only() {
    let plain = object(16.0);
    let overridden = plain.with_glyph_override('A', GlyphRef::Id(7));
    assert_eq!(overridden.glyph_for_char('A'), 7);
    assert_eq!(plain.glyph_for_char('A'), system_font().glyph('A'));
}

#[test]
fn glyphs_for_a_string_follow_the_order_of_its_characters() {
    let o = object(16.0);
    assert_eq!(o.glyphs_for_str("AB"), vec![o.glyph_for_char('A'), o.glyph_for_char('B')]);
}

#[test]
fn the_width_of_an_empty_string_is_zero_and_grows_with_the_size() {
    assert_eq!(object(16.0).text_width(""), 0.0);
    let small = object(16.0).text_width("Hello");
    let large = object(32.0).text_width("Hello");
    assert!(small > 0.0);
    assert!(large > small, "a larger size measures wider");
}

#[test]
fn a_variation_replaces_an_earlier_value_of_the_same_axis() {
    let o = object(16.0).with_variations(&[(*b"wght", 700.0)]).with_variations(&[(*b"wght", 300.0)]);
    assert_eq!(o.variations().len(), 1);
    assert_eq!(o.variations()[0].1, 300.0);
}

#[test]
fn a_font_object_lists_its_variation_axes_with_their_range() {
    let axes = object(16.0).variation_axes();
    assert!(axes.iter().any(|a| a.0 == *b"wght" && a.1 == 100.0 && a.2 == 400.0 && a.3 == 900.0));
}

#[test]
fn a_font_object_copies_its_tables_by_tag() {
    let o = object(16.0);
    assert!(o.available_tables().contains(b"cmap"));
    assert_eq!(o.copy_table(b"head").expect("head table").len(), 54, "the head table is 54 bytes");
    assert!(o.copy_table(b"zzzz").is_none());
}

#[test]
fn a_descriptor_fills_in_the_face_names_and_the_size() {
    let d = object(16.0).descriptor();
    assert_eq!(d.family.as_deref(), Some("Inter"));
    assert_eq!(d.postscript_name.as_deref(), Some("Inter-Regular"));
    assert_eq!(d.size, Some(16.0));
}

// fontobj: the manager registers faces, matches specs and reports changes

#[test]
fn registering_a_face_makes_its_family_available() {
    let mut m = FontManager::new();
    assert_eq!(m.register_data(SYSTEM_FACE.to_vec(), Scope::Process), Ok("Inter-Regular".to_string()));
    assert!(m.available_families().contains(&"Inter".to_string()));
}

#[test]
fn registering_the_same_face_twice_is_refused() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("first registration");
    assert_eq!(m.register_data(SYSTEM_FACE.to_vec(), Scope::Process), Err(FontError::AlreadyRegistered));
}

#[test]
fn bytes_that_are_not_a_font_are_refused() {
    let mut m = FontManager::new();
    assert_eq!(m.register_data(b"not a font".to_vec(), Scope::Process), Err(FontError::Invalid));
}

#[test]
fn a_process_face_can_be_unregistered_and_a_system_face_cannot() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    assert_eq!(m.unregister("Inter-Regular"), Ok(()));
    assert_eq!(m.unregister("Inter-Regular"), Err(FontError::NotRegistered));

    let mut system = FontManager::new();
    system.register_data(SYSTEM_FACE.to_vec(), Scope::System).expect("registered");
    assert_eq!(system.unregister("Inter-Regular"), Err(FontError::InUse));
    assert_eq!(system.faces().len(), 1, "the system face stays");
}

#[test]
fn listeners_hear_registrations_and_removals_in_order() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    m.unregister("Inter-Regular").expect("removed");
    assert_eq!(
        m.drain_events(),
        vec![FontEvent::Registered("Inter-Regular".to_string()), FontEvent::Unregistered("Inter-Regular".to_string())]
    );
    assert!(m.drain_events().is_empty(), "draining empties the queue");
}

#[test]
fn a_family_name_is_a_hard_filter_when_matching_a_spec() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    assert_eq!(m.faces_in_family("Inter").len(), 1);
    let found = FontObject::from_spec(&m, &FontSpec::new().family("Inter")).expect("the family matches");
    assert_eq!(found.postscript_name(), "Inter-Regular");
    assert_eq!(found.face_index(), Some(0));
    assert!(FontObject::from_spec(&m, &FontSpec::new().family("No Such Family")).is_none());
}

#[test]
fn a_font_object_is_found_by_postscript_name() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    assert!(FontObject::from_name(&m, "Inter-Regular", 12.0, None).is_some());
    assert!(FontObject::from_name(&m, "Missing-Face", 12.0, None).is_none());
}

// fontobj and fontfallback: the cascade for characters the face lacks

#[test]
fn text_the_face_covers_stays_on_the_face_itself() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    let o = FontObject::from_name(&m, "Inter-Regular", 16.0, None).expect("object");
    let runs = o.fallback_runs(&m, "AB", "en");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].face, None);
    assert_eq!(runs[0].text, "AB");
}

#[test]
fn fallback_runs_keep_every_character_in_order() {
    let mut m = FontManager::new();
    m.register_data(SYSTEM_FACE.to_vec(), Scope::Process).expect("registered");
    let o = FontObject::from_name(&m, "Inter-Regular", 16.0, None).expect("object");
    let text = "A\u{10FFFF}B";
    let joined: String = o.fallback_runs(&m, text, "en").into_iter().map(|r| r.text).collect();
    assert_eq!(joined, text);
}

#[test]
fn split_joins_neighbours_that_share_a_face() {
    let runs = split("abc", |c| (c == 'b').then_some(1));
    assert_eq!(runs, vec![(None, "a"), (Some(1), "b"), (None, "c")]);
    assert_eq!(split("aaa", |_| None), vec![(None, "aaa")]);
}

#[test]
fn the_japanese_cascade_starts_with_the_japanese_gothic_face_and_ends_with_colour_emoji() {
    let v = default_cascade("ja");
    assert_eq!(v.first().copied(), Some("Noto Sans CJK JP"));
    assert_eq!(v.last().copied(), Some("Noto Color Emoji"));
    let mut unique = v.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), v.len(), "no family is listed twice");
}

#[test]
fn the_arabic_cascade_uses_the_arabic_faces_for_regional_tags() {
    assert_eq!(default_cascade("ar-EG").first().copied(), Some("Noto Sans Arabic"));
}

#[test]
fn the_traditional_chinese_cascade_is_chosen_by_script_subtag() {
    assert_eq!(default_cascade("zh-Hant").first().copied(), Some("Noto Sans CJK TC"));
    assert_eq!(default_cascade("zh-CN").first().copied(), Some("Noto Sans CJK SC"));
}

// glass: the measured looks of each material kind and the filter arithmetic

#[test]
fn a_glass_look_mixed_at_the_ends_returns_the_dark_and_light_looks() {
    let (dark, light) = Kind::Content.looks();
    let at_dark = Look::mix(dark, light, 0.0);
    let at_light = Look::mix(dark, light, 1.0);
    assert_eq!(at_dark.saturation, dark.saturation);
    assert!(approx(at_light.saturation, light.saturation));
    assert!(approx(at_light.brightness, light.brightness));
    assert!(approx(at_light.lum_amount, light.lum_amount));
}

#[test]
fn a_glass_filter_keeps_every_channel_in_the_unit_range() {
    let (dark, _) = Kind::Chrome.looks();
    for rgb in [[1.0, 0.0, 0.5], [0.0, 1.0, 0.0], [0.2, 0.9, 1.0]] {
        assert!(dark.apply(rgb).iter().all(|c| (0.0..=1.0).contains(c)));
    }
}

#[test]
fn zero_saturation_turns_a_colour_into_its_luma() {
    let grey = Look { saturation: 0.0, brightness: 0.0, lum_amount: 0.0, lum_values: [0.0; 4] };
    let out = grey.apply([1.0, 0.0, 0.0]);
    assert!(approx(out[0], 0.2126), "red keeps its share of luma");
    assert!(approx(out[1], 0.2126) && approx(out[2], 0.2126), "all channels take the same value");
}

#[test]
fn brightness_shifts_every_channel_and_clamps_at_one() {
    let lift = Look { saturation: 1.0, brightness: 0.5, lum_amount: 0.0, lum_values: [0.0; 4] };
    let out = lift.apply([0.8, 0.2, 0.0]);
    assert!(approx(out[0], 1.0), "0.8 + 0.5 clamps to one");
    assert!(approx(out[1], 0.7));
    assert!(approx(out[2], 0.5));
}

#[test]
fn chrome_glass_carries_a_specular_rim_and_sidebar_glass_does_not() {
    assert_eq!(Kind::Chrome.style().rim, 1.0);
    assert_eq!(Kind::Sidebar.style().rim, 0.0);
    assert!(!Kind::Chrome.style().follows_active, "the shell's glass stays on while its window is inactive");
    assert!(Kind::Menu.style().follows_active, "menus turn opaque while their window is inactive");
}

#[test]
fn the_shell_chrome_glass_never_takes_vibrancy_for_its_content() {
    assert!(!Kind::Chrome.allows_vibrancy());
    assert_eq!(Kind::default(), Kind::Chrome);
}
