//! Headless tests for the filter catalogue: the built filters by reference name, their categories and their input
//! attributes. The catalogue is data, so each test reads it through the public API and asserts concrete names and
//! values. No drawing and no display are involved.

use bifrost_render::filter_catalogue::{
    all_filter_names, attributes, default_value, filter_names, CATEGORY_BLUR, CATEGORY_DISTORTION, CATEGORY_TILE,
};

#[test]
fn headless_catalogue_holds_the_106_built_filters_with_unique_reference_names() {
    let names = all_filter_names();
    assert_eq!(names.len(), 106);
    assert!(names.iter().all(|n| n.starts_with("CI")));
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 106, "every name is listed once");
}

#[test]
fn headless_detectors_and_render_destinations_are_not_catalogued() {
    let names = all_filter_names();
    assert!(!names.contains(&"CIDetector"));
    assert!(!names.contains(&"CIRenderDestination"));
}

#[test]
fn headless_category_listing_filters_by_category_in_catalogue_order() {
    assert_eq!(
        filter_names(Some(CATEGORY_DISTORTION)),
        [
            "CITwirlDistortion",
            "CIVortexDistortion",
            "CIPinchDistortion",
            "CIBumpDistortion",
            "CIGlassDistortion",
            "CIHoleDistortion",
            "CILightTunnel",
            "CITorusLensDistortion",
            "CICircleSplashDistortion"
        ]
    );
    assert_eq!(filter_names(Some(CATEGORY_BLUR)), ["CIGaussianBlur", "CIBoxBlur", "CIMotionBlur", "CIZoomBlur", "CIDiscBlur"]);
    assert_eq!(filter_names(None).len(), 106);
    assert!(filter_names(Some("CICategoryNothing")).is_empty());
}

#[test]
fn headless_attributes_use_the_reference_keys_and_defaults() {
    assert_eq!(default_value("CIUnsharpMask", "inputRadius"), Some(2.5));
    assert_eq!(default_value("CISharpenLuminance", "inputSharpness"), Some(0.4));
    assert_eq!(default_value("CIPinchDistortion", "inputScale"), Some(0.5));
    assert_eq!(default_value("CIDotScreen", "inputWidth"), Some(6.0));
    assert_eq!(default_value("CIBloom", "inputBogus"), None);
    assert_eq!(default_value("CINope", "inputRadius"), None);
    assert!(attributes("CINope").is_none());
}

#[test]
fn headless_filter_info_carries_its_category_and_attribute_ranges() {
    let kaleidoscope = attributes("CIKaleidoscope").expect("catalogued");
    assert_eq!(kaleidoscope.name, "CIKaleidoscope");
    assert_eq!(kaleidoscope.category, CATEGORY_TILE);
    let count = kaleidoscope.attributes.iter().find(|a| a.key == "inputCount").expect("inputCount");
    assert_eq!((count.default, count.min, count.max), (6.0, 1.0, 64.0));
    let motion = attributes("CIMotionBlur").expect("catalogued");
    let angle = motion.attributes.iter().find(|a| a.key == "inputAngle").expect("inputAngle");
    assert_eq!(angle.default, 0.0);
    assert!(angle.min < 0.0 && angle.max > 0.0, "the angle range is symmetric about zero");
}

#[test]
fn headless_filters_without_inputs_list_no_attributes() {
    let affine = attributes("CIAffineTile").expect("catalogued");
    assert!(affine.attributes.is_empty());
    assert!(attributes("CISourceOverCompositing").expect("catalogued").attributes.is_empty());
}

#[test]
fn headless_every_default_lies_inside_its_range() {
    for name in all_filter_names() {
        let info = attributes(name).expect("every listed name has attributes");
        for a in info.attributes {
            assert!(a.default >= a.min && a.default <= a.max, "{} {} default {} outside {}..{}", name, a.key, a.default, a.min, a.max);
        }
    }
}
