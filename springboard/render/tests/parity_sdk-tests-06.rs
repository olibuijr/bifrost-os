//! Parity sdk-tests-06: headless tests of the colour model. ICC profile parsing, tone curves, video card gamma tables,
//! the device profile registry, RGB working-space conversions and the memory-pressure cache. Every test works on
//! memory only: no display, no device and no clock is touched.

use bifrost_render::colorspace::{Space, Transform};
use bifrost_render::icc::{Curve, DeviceClass, DeviceError, Devices, IccError, Profile, Vcgt, FALLBACK_DISPLAY_PROFILE};
use bifrost_render::purgeable::Cache;

/// Builds a profile with the given device class, colour space and tags. Each tag's data is padded to four bytes.
fn build_profile(class: &str, color_space: &str, tags: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let table_end = 132 + 12 * tags.len();
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    for (_, bytes) in tags {
        offsets.push(table_end + body.len());
        body.extend_from_slice(bytes);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    let total = table_end + body.len();
    let mut data = vec![0u8; table_end];
    data[0..4].copy_from_slice(&(total as u32).to_be_bytes());
    data[8] = 2;
    data[9] = 0x10;
    data[12..16].copy_from_slice(class.as_bytes());
    data[16..20].copy_from_slice(color_space.as_bytes());
    data[20..24].copy_from_slice(b"XYZ ");
    data[36..40].copy_from_slice(b"acsp");
    data[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
    for (i, (sig, bytes)) in tags.iter().enumerate() {
        let at = 132 + 12 * i;
        data[at..at + 4].copy_from_slice(sig.as_bytes());
        data[at + 4..at + 8].copy_from_slice(&(offsets[i] as u32).to_be_bytes());
        data[at + 8..at + 12].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
    }
    data.extend_from_slice(&body);
    data
}

/// A version 2 description tag.
fn desc_tag(text: &str) -> Vec<u8> {
    let mut v = b"desc".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
    v.extend_from_slice(text.as_bytes());
    v.push(0);
    v
}

/// A version 4 multi-localized description tag with one en-US record, the text in UTF-16BE.
fn mluc_tag(text: &str) -> Vec<u8> {
    let units: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    let mut v = b"mluc".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&12u32.to_be_bytes());
    v.extend_from_slice(b"en");
    v.extend_from_slice(b"US");
    v.extend_from_slice(&(units.len() as u32).to_be_bytes());
    v.extend_from_slice(&28u32.to_be_bytes());
    v.extend_from_slice(&units);
    v
}

/// An XYZ tag holding one colour, in signed 15.16 fixed point.
fn xyz_tag(xyz: [f64; 3]) -> Vec<u8> {
    let mut v = b"XYZ ".to_vec();
    v.extend_from_slice(&[0; 4]);
    for c in xyz {
        v.extend_from_slice(&((c * 65536.0).round() as i32).to_be_bytes());
    }
    v
}

/// A `curv` tag with no entries: the identity curve.
fn curv_identity() -> Vec<u8> {
    let mut v = b"curv".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

/// A `curv` tag with one entry: a gamma in unsigned 8.8 fixed point.
fn curv_gamma(gamma: f64) -> Vec<u8> {
    let mut v = b"curv".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
    v
}

/// The matrix and curve tags of an RGB profile with the sRGB primaries, adapted to D50, and the given tone curves.
fn rgb_profile(curves: [Vec<u8>; 3]) -> Vec<u8> {
    let [r, g, b] = curves;
    build_profile(
        "mntr",
        "RGB ",
        &[
            ("desc", mluc_tag("Panel sRGB")),
            ("rXYZ", xyz_tag([0.4361, 0.2225, 0.0139])),
            ("gXYZ", xyz_tag([0.3851, 0.7169, 0.0971])),
            ("bXYZ", xyz_tag([0.1431, 0.0606, 0.7141])),
            ("rTRC", r),
            ("gTRC", g),
            ("bTRC", b),
        ],
    )
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

// --- profile parsing (icc) ---------------------------------------------------------------------------------------

#[test]
fn a_profile_header_gives_its_class_spaces_version_and_size() {
    let data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    let profile = Profile::parse(&data).unwrap();
    assert_eq!(profile.header.device_class, "mntr");
    assert_eq!(profile.header.color_space, "RGB ");
    assert_eq!(profile.header.connection_space, "XYZ ");
    assert_eq!(profile.header.version, (2, 1));
    assert_eq!(profile.header.size as usize, data.len());
    assert_eq!(profile.tags.len(), 1);
    assert_eq!(profile.tags[0].signature, "desc");
}

#[test]
fn data_shorter_than_the_header_is_refused_as_truncated() {
    assert_eq!(Profile::parse(&[0u8; 100]), Err(IccError::Truncated));
}

#[test]
fn a_declared_size_larger_than_the_data_is_refused_as_truncated() {
    let mut data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    let too_big = data.len() as u32 + 4;
    data[0..4].copy_from_slice(&too_big.to_be_bytes());
    assert_eq!(Profile::parse(&data), Err(IccError::Truncated));
}

#[test]
fn data_without_the_acsp_magic_is_not_a_profile() {
    let mut data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    data[36..40].copy_from_slice(b"junk");
    assert_eq!(Profile::parse(&data), Err(IccError::NotAProfile));
}

#[test]
fn a_tag_that_runs_past_the_profile_is_named_in_the_error() {
    let mut data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    // The size of the first tag sits at 132 + 8; make it run far past the end.
    data[140..144].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
    assert_eq!(Profile::parse(&data), Err(IccError::TagOutOfBounds("desc".into())));
}

#[test]
fn has_tag_reports_only_the_tags_in_the_table() {
    let data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel")), ("vcgt", b"vcgt".to_vec())]);
    let profile = Profile::parse(&data).unwrap();
    assert!(profile.has_tag("desc"));
    assert!(profile.has_tag("vcgt"));
    assert!(!profile.has_tag("A2B0"));
}

#[test]
fn a_version_two_description_reads_its_ascii_text() {
    let data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    let profile = Profile::parse(&data).unwrap();
    assert_eq!(profile.description(&data).as_deref(), Some("Panel"));
}

#[test]
fn a_version_four_description_reads_utf16_text_with_non_ascii_letters() {
    let data = build_profile("mntr", "RGB ", &[("desc", mluc_tag("Skjár sRGB"))]);
    let profile = Profile::parse(&data).unwrap();
    assert_eq!(profile.description(&data).as_deref(), Some("Skjár sRGB"));
}

#[test]
fn a_profile_without_a_description_tag_has_no_description() {
    let data = build_profile("mntr", "RGB ", &[("vcgt", b"vcgt".to_vec())]);
    let profile = Profile::parse(&data).unwrap();
    assert_eq!(profile.description(&data), None);
}

#[test]
fn a_description_with_an_unknown_type_is_not_read() {
    let data = build_profile("mntr", "RGB ", &[("desc", b"xxxx\0\0\0\0\0\0\0\x01a\0".to_vec())]);
    let profile = Profile::parse(&data).unwrap();
    assert_eq!(profile.description(&data), None);
}

// --- matrix-shaper profiles and tone curves (icc, colorspace) ------------------------------------------------------

#[test]
fn the_colorants_of_a_white_rgb_profile_sum_to_the_d50_white() {
    let data = rgb_profile([curv_identity(), curv_identity(), curv_identity()]);
    let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
    let xyz = shaper.to_xyz_d50([1.0, 1.0, 1.0]);
    assert!(close(xyz, [0.9642, 1.0, 0.8249], 0.01), "got {xyz:?}");
}

#[test]
fn a_red_primary_converts_to_the_red_primary_of_the_working_space() {
    let data = rgb_profile([curv_identity(), curv_identity(), curv_identity()]);
    let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
    let out = shaper.convert([1.0, 0.0, 0.0], Space::Srgb);
    assert!(close(out, [1.0, 0.0, 0.0], 0.01), "got {out:?}");
}

#[test]
fn a_white_rgb_profile_converts_white_to_white_in_srgb() {
    let data = rgb_profile([curv_identity(), curv_identity(), curv_identity()]);
    let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
    assert!(close(shaper.convert([1.0, 1.0, 1.0], Space::Srgb), [1.0, 1.0, 1.0], 0.01));
}

#[test]
fn a_gamma_tone_curve_decodes_before_the_colorants_apply() {
    let data = rgb_profile([curv_gamma(2.2), curv_identity(), curv_identity()]);
    let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
    let x = shaper.to_xyz_d50([0.5, 0.0, 0.0])[0];
    let expected = 0.4361 * 0.5f64.powf(2.2);
    assert!((x - expected).abs() < 1e-3, "got {x}, expected {expected}");
}

#[test]
fn a_profile_outside_the_rgb_space_has_no_matrix_shaper() {
    let data = build_profile("mntr", "GRAY", &[("desc", desc_tag("Gray"))]);
    let profile = Profile::parse(&data).unwrap();
    assert!(profile.matrix_shaper(&data).is_none());
}

#[test]
fn a_matrix_shaper_needs_all_six_colorant_and_curve_tags() {
    let data = build_profile("mntr", "RGB ", &[("rXYZ", xyz_tag([0.4, 0.2, 0.0])), ("rTRC", curv_identity())]);
    let profile = Profile::parse(&data).unwrap();
    assert!(profile.matrix_shaper(&data).is_none());
}

#[test]
fn curves_evaluate_identity_gamma_table_and_parametric_forms() {
    assert_eq!(Curve::Identity.eval(0.3), 0.3);
    assert!((Curve::Gamma(2.0).eval(0.5) - 0.25).abs() < 1e-12);
    assert!((Curve::Table(vec![0.0, 1.0]).eval(0.25) - 0.25).abs() < 1e-12);
    assert!((Curve::Table(vec![0.0, 0.5, 1.0]).eval(0.75) - 0.75).abs() < 1e-12);
    assert!((Curve::Parametric { kind: 0, params: vec![2.0] }.eval(0.5) - 0.25).abs() < 1e-12);
    // Function type 3: a linear toe below d = 0.04 and a power above it.
    let toe = Curve::Parametric { kind: 3, params: vec![2.0, 1.0, 0.0, 0.5, 0.04] };
    assert!((toe.eval(0.02) - 0.01).abs() < 1e-12);
    assert!((toe.eval(0.5) - 0.25).abs() < 1e-12);
}

#[test]
fn a_curve_clamps_its_input_to_the_unit_range() {
    assert_eq!(Curve::Identity.eval(2.0), 1.0);
    assert_eq!(Curve::Identity.eval(-1.0), 0.0);
}

// --- video card gamma tables (icc, sdk colorsync) ------------------------------------------------------------------

fn vcgt_table_tag() -> Vec<u8> {
    let mut v = b"vcgt".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&2u16.to_be_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&[0, 255, 0, 128, 0, 64]);
    v
}

fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn vcgt_formula_tag(channels: [(f64, f64, f64); 3]) -> Vec<u8> {
    let mut v = b"vcgt".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&1u32.to_be_bytes());
    for (gamma, min, max) in channels {
        v.extend_from_slice(&s15(gamma));
        v.extend_from_slice(&s15(min));
        v.extend_from_slice(&s15(max));
    }
    v
}

#[test]
fn a_table_vcgt_gives_each_channel_its_own_ramp() {
    let data = build_profile("mntr", "RGB ", &[("vcgt", vcgt_table_tag())]);
    let vcgt = Profile::parse(&data).unwrap().vcgt(&data).unwrap();
    assert!(matches!(vcgt, Vcgt::Table(_)));
    assert!((vcgt.eval(0, 1.0) - 1.0).abs() < 1e-9);
    assert!((vcgt.eval(1, 1.0) - 128.0 / 255.0).abs() < 1e-9);
    assert!((vcgt.eval(2, 1.0) - 64.0 / 255.0).abs() < 1e-9);
    // A channel index past blue reads blue.
    assert_eq!(vcgt.eval(7, 1.0), vcgt.eval(2, 1.0));
}

#[test]
fn a_table_vcgt_ramp_is_sixteen_bit_and_evenly_sampled() {
    let data = build_profile("mntr", "RGB ", &[("vcgt", vcgt_table_tag())]);
    let vcgt = Profile::parse(&data).unwrap().vcgt(&data).unwrap();
    assert_eq!(vcgt.ramp(0, 3), vec![0, 32768, 65535]);
    assert_eq!(vcgt.ramp(0, 1), vec![0]);
}

#[test]
fn a_formula_vcgt_applies_gamma_and_the_output_range_per_channel() {
    let tag = vcgt_formula_tag([(1.0, 0.0, 1.0), (2.0, 0.0, 0.5), (1.0, 0.25, 0.75)]);
    let data = build_profile("mntr", "RGB ", &[("vcgt", tag)]);
    let vcgt = Profile::parse(&data).unwrap().vcgt(&data).unwrap();
    assert!(matches!(vcgt, Vcgt::Formula(_)));
    assert!((vcgt.eval(0, 0.5) - 0.5).abs() < 1e-4);
    assert!((vcgt.eval(1, 0.5) - 0.125).abs() < 1e-4);
    assert!((vcgt.eval(2, 0.0) - 0.25).abs() < 1e-4);
    assert!((vcgt.eval(2, 1.0) - 0.75).abs() < 1e-4);
}

#[test]
fn a_vcgt_with_an_unknown_type_is_not_read() {
    let mut tag = vcgt_table_tag();
    tag[8..12].copy_from_slice(&2u32.to_be_bytes());
    let data = build_profile("mntr", "RGB ", &[("vcgt", tag)]);
    assert!(Profile::parse(&data).unwrap().vcgt(&data).is_none());
}

#[test]
fn a_profile_without_a_vcgt_tag_has_no_gamma_table() {
    let data = build_profile("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
    assert!(Profile::parse(&data).unwrap().vcgt(&data).is_none());
}

// --- device profile registry (icc Devices) ---------------------------------------------------------------------------

#[test]
fn a_registered_display_uses_its_default_profile() {
    let mut devices = Devices::new();
    devices.register(DeviceClass::Display, "panel", "Panel sRGB").unwrap();
    assert_eq!(devices.profile_for(DeviceClass::Display, "panel"), Some("Panel sRGB"));
    assert_eq!(devices.display_profile("panel"), "Panel sRGB");
}

#[test]
fn an_unregistered_display_falls_back_to_the_srgb_profile() {
    let devices = Devices::new();
    assert_eq!(devices.display_profile("missing"), FALLBACK_DISPLAY_PROFILE);
    assert_eq!(FALLBACK_DISPLAY_PROFILE, "sRGB IEC61966-2.1");
    assert_eq!(devices.profile_for(DeviceClass::Display, "missing"), None);
}

#[test]
fn a_custom_profile_overrides_the_default_until_it_is_cleared() {
    let mut devices = Devices::new();
    devices.register(DeviceClass::Display, "panel", "Panel sRGB").unwrap();
    devices.set_custom(DeviceClass::Display, "panel", Some("Calibrated")).unwrap();
    assert_eq!(devices.display_profile("panel"), "Calibrated");
    devices.set_custom(DeviceClass::Display, "panel", None).unwrap();
    assert_eq!(devices.display_profile("panel"), "Panel sRGB");
}

#[test]
fn registering_again_replaces_the_default_and_keeps_the_custom_profile() {
    let mut devices = Devices::new();
    devices.register(DeviceClass::Display, "panel", "Old").unwrap();
    devices.set_custom(DeviceClass::Display, "panel", Some("Calibrated")).unwrap();
    devices.register(DeviceClass::Display, "panel", "New").unwrap();
    assert_eq!(devices.display_profile("panel"), "Calibrated");
    devices.set_custom(DeviceClass::Display, "panel", None).unwrap();
    assert_eq!(devices.display_profile("panel"), "New");
}

#[test]
fn empty_profile_names_are_refused() {
    let mut devices = Devices::new();
    assert_eq!(devices.register(DeviceClass::Printer, "p", ""), Err(DeviceError::EmptyProfile));
    devices.register(DeviceClass::Printer, "p", "Paper").unwrap();
    assert_eq!(devices.set_custom(DeviceClass::Printer, "p", Some("")), Err(DeviceError::EmptyProfile));
}

#[test]
fn a_custom_profile_needs_a_registered_device() {
    let mut devices = Devices::new();
    assert_eq!(devices.set_custom(DeviceClass::Scanner, "s", Some("Scan")), Err(DeviceError::NotRegistered));
}

#[test]
fn unregistering_a_device_reports_whether_it_was_there() {
    let mut devices = Devices::new();
    devices.register(DeviceClass::Camera, "cam", "Lens").unwrap();
    assert!(devices.unregister(DeviceClass::Camera, "cam"));
    assert!(!devices.unregister(DeviceClass::Camera, "cam"));
    assert_eq!(devices.profile_for(DeviceClass::Camera, "cam"), None);
}

#[test]
fn iterate_lists_devices_by_class_then_identifier() {
    let mut devices = Devices::new();
    devices.register(DeviceClass::Camera, "cam", "Lens").unwrap();
    devices.register(DeviceClass::Printer, "p2", "Paper B").unwrap();
    devices.register(DeviceClass::Display, "z", "Panel Z").unwrap();
    devices.register(DeviceClass::Display, "a", "Panel A").unwrap();
    let listed = devices.iterate();
    assert_eq!(
        listed,
        vec![
            (DeviceClass::Display, "a", "Panel A"),
            (DeviceClass::Display, "z", "Panel Z"),
            (DeviceClass::Printer, "p2", "Paper B"),
            (DeviceClass::Camera, "cam", "Lens"),
        ]
    );
}

// --- working spaces (colorspace) --------------------------------------------------------------------------------------

#[test]
fn the_four_working_spaces_have_their_settings_names() {
    assert_eq!(Space::ALL.len(), 4);
    assert_eq!(Space::Srgb.name(), "sRGB");
    assert_eq!(Space::DisplayP3.name(), "Display P3");
    assert_eq!(Space::AdobeRgb.name(), "Adobe RGB (1998)");
    assert_eq!(Space::RommRgb.name(), "ROMM RGB");
}

#[test]
fn the_srgb_transfer_curve_decodes_mid_grey_and_mirrors_negative_input() {
    let decoded = Space::Srgb.to_linear(0.5);
    assert!((decoded - 0.2140).abs() < 1e-3, "got {decoded}");
    assert!((Space::Srgb.to_linear(-0.5) + decoded).abs() < 1e-12);
    let round = Space::Srgb.from_linear(Space::Srgb.to_linear(0.3));
    assert!((round - 0.3).abs() < 1e-9);
}

#[test]
fn a_conversion_within_one_space_is_the_identity() {
    let t = Transform::new(Space::Srgb, Space::Srgb);
    assert!(close(t.apply([0.2, 0.5, 0.8]), [0.2, 0.5, 0.8], 1e-6));
    assert_eq!(t.apply_u8([10, 128, 250]), [10, 128, 250]);
}

#[test]
fn a_transform_remembers_its_source_and_destination() {
    let t = Transform::new(Space::Srgb, Space::AdobeRgb);
    assert_eq!(t.source(), Space::Srgb);
    assert_eq!(t.destination(), Space::AdobeRgb);
}

#[test]
fn white_stays_white_between_spaces_that_share_the_d65_white() {
    let t = Transform::new(Space::Srgb, Space::DisplayP3);
    assert!(close(t.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-3));
}

#[test]
fn an_srgb_red_is_a_less_saturated_red_in_display_p3() {
    let t = Transform::new(Space::Srgb, Space::DisplayP3);
    let out = t.apply_u8([255, 0, 0]);
    assert!((out[0] as i32 - 234).abs() <= 3, "red {out:?}");
    assert!((out[1] as i32 - 51).abs() <= 3, "green {out:?}");
    assert!((out[2] as i32 - 35).abs() <= 3, "blue {out:?}");
}

#[test]
fn a_colour_outside_the_target_gamut_is_clamped_in_eight_bits() {
    // Pure Display P3 green lies outside sRGB: its sRGB red is negative before clamping.
    let t = Transform::new(Space::DisplayP3, Space::Srgb);
    assert!(t.apply([0.0, 1.0, 0.0])[0] < 0.0);
    assert_eq!(t.apply_u8([0, 255, 0])[0], 0);
}

#[test]
fn the_rom_space_white_converts_to_white_in_srgb() {
    let t = Transform::new(Space::RommRgb, Space::Srgb);
    assert!(close(t.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 0.01));
}

// --- memory-pressure cache (purgeable) --------------------------------------------------------------------------------

#[test]
fn a_cached_value_is_returned_and_counted_at_its_cost() {
    let cache: Cache<u32, String> = Cache::new(0, 0);
    cache.set(1, "one".into(), 10);
    assert_eq!(cache.get(&1).as_deref(), Some("one"));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.total_cost(), 10);
    assert!(cache.get(&2).is_none());
}

#[test]
fn setting_a_key_again_replaces_its_value_and_cost() {
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    cache.set(7, 1, 5);
    cache.set(7, 2, 8);
    assert_eq!(cache.get(&7), Some(2));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.total_cost(), 8);
}

#[test]
fn the_count_limit_evicts_the_least_recently_used_entry() {
    let cache: Cache<u32, u32> = Cache::new(2, 0);
    cache.set(1, 10, 1);
    cache.set(2, 20, 1);
    assert_eq!(cache.get(&1), Some(10), "a hit makes key 1 the most recent");
    cache.set(3, 30, 1);
    assert!(cache.get(&2).is_none());
    assert_eq!(cache.get(&1), Some(10));
    assert_eq!(cache.get(&3), Some(30));
}

#[test]
fn the_cost_limit_evicts_the_oldest_entries_until_the_total_fits() {
    let cache: Cache<u32, u32> = Cache::new(0, 25);
    cache.set(1, 1, 10);
    cache.set(2, 2, 10);
    cache.set(3, 3, 10);
    assert!(cache.get(&1).is_none());
    assert_eq!(cache.total_cost(), 20);
    assert_eq!(cache.total_cost_limit(), 25);
}

#[test]
fn a_value_costing_more_than_the_whole_limit_is_not_kept() {
    let cache: Cache<u32, u32> = Cache::new(0, 10);
    cache.set(1, 1, 50);
    assert!(cache.is_empty());
    assert_eq!(cache.total_cost(), 0);
}

#[test]
fn lowering_a_limit_trims_the_cache_at_once() {
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    for k in 1..=3 {
        cache.set(k, k, 4);
    }
    cache.set_count_limit(1);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&3), Some(3), "the newest entry survives");
    assert_eq!(cache.count_limit(), 1);
}

#[test]
fn a_partial_purge_drops_the_oldest_half_of_the_cost() {
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    for k in 1..=4 {
        cache.set(k, k, 10);
    }
    assert_eq!(cache.purge(0.5), 20);
    assert_eq!(cache.len(), 2);
    assert!(cache.get(&1).is_none() && cache.get(&2).is_none());
    assert_eq!(cache.get(&3), Some(3));
    assert_eq!(cache.get(&4), Some(4));
}

#[test]
fn a_full_purge_empties_the_cache() {
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    cache.set(1, 1, 3);
    cache.set(2, 2, 3);
    assert_eq!(cache.purge(1.0), 6);
    assert!(cache.is_empty());
    cache.set(3, 3, 1);
    cache.remove_all();
    assert!(cache.is_empty());
}

#[test]
fn removing_a_key_frees_its_cost() {
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    cache.set(1, 1, 6);
    cache.remove(&1);
    assert_eq!(cache.total_cost(), 0);
    assert!(cache.get(&1).is_none());
}

#[test]
fn a_cloned_cache_shares_its_entries() {
    let a: Cache<u32, u32> = Cache::new(0, 0);
    let b = a.clone();
    b.set(9, 90, 1);
    assert_eq!(a.get(&9), Some(90));
}
