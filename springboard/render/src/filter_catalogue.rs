//! The filter catalogue: every Core Image filter BifrOSt builds, by its reference name, with its
//! categories and input attributes, as `CIFilter.filterNames(inCategory:)` and `CIFilter.attributes`
//! report them. Names are the reference `CI...` names. `CIDetector` and `CIRenderDestination` have
//! no CPU filter here: the QR decoder lives in the shell crate, so detectors stay a gap.

use std::f32::consts::PI;

pub const CATEGORY_DISTORTION: &str = "CICategoryDistortionEffect";
pub const CATEGORY_SHARPEN: &str = "CICategorySharpen";
pub const CATEGORY_STYLIZE: &str = "CICategoryStylize";
pub const CATEGORY_HALFTONE: &str = "CICategoryHalftoneEffect";
pub const CATEGORY_TILE: &str = "CICategoryTileEffect";
pub const CATEGORY_BLUR: &str = "CICategoryBlur";
pub const CATEGORY_COMPOSITE: &str = "CICategoryCompositeOperation";

/// One input attribute: its reference key, default and range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attribute {
    pub key: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

/// One catalogue entry.
#[derive(Clone, Copy, Debug)]
pub struct FilterInfo {
    pub name: &'static str,
    pub category: &'static str,
    pub attributes: &'static [Attribute],
}

const fn a(key: &'static str, default: f32, min: f32, max: f32) -> Attribute {
    Attribute { key, default, min, max }
}

const RADIUS: Attribute = a("inputRadius", 300.0, 0.0, 1000.0);
const ANGLE: Attribute = a("inputAngle", 0.0, -PI, PI);

static FILTERS: &[FilterInfo] = &[
    FilterInfo { name: "CITwirlDistortion", category: CATEGORY_DISTORTION, attributes: &[RADIUS, a("inputAngle", PI, -12.566, 12.566)] },
    FilterInfo { name: "CIVortexDistortion", category: CATEGORY_DISTORTION, attributes: &[a("inputRadius", 300.0, 0.0, 800.0), a("inputAngle", 56.5487, -94.2478, 94.2478)] },
    FilterInfo { name: "CIPinchDistortion", category: CATEGORY_DISTORTION, attributes: &[a("inputRadius", 300.0, 0.0, 2000.0), a("inputScale", 0.5, 0.0, 1.0)] },
    FilterInfo { name: "CIBumpDistortion", category: CATEGORY_DISTORTION, attributes: &[a("inputRadius", 300.0, 0.0, 600.0), a("inputScale", 0.5, -1.0, 1.0)] },
    FilterInfo { name: "CIGlassDistortion", category: CATEGORY_DISTORTION, attributes: &[a("inputScale", 200.0, 0.0, 500.0)] },
    FilterInfo { name: "CISharpenLuminance", category: CATEGORY_SHARPEN, attributes: &[a("inputSharpness", 0.4, 0.0, 2.0), a("inputRadius", 1.69, 0.0, 10.0)] },
    FilterInfo { name: "CIUnsharpMask", category: CATEGORY_SHARPEN, attributes: &[a("inputRadius", 2.5, 0.0, 100.0), a("inputIntensity", 0.5, 0.0, 1.0)] },
    FilterInfo { name: "CIBloom", category: CATEGORY_STYLIZE, attributes: &[a("inputRadius", 10.0, 0.0, 100.0), a("inputIntensity", 0.5, 0.0, 1.0)] },
    FilterInfo { name: "CIGloom", category: CATEGORY_STYLIZE, attributes: &[a("inputRadius", 10.0, 0.0, 100.0), a("inputIntensity", 0.5, 0.0, 1.0)] },
    FilterInfo { name: "CICrystallize", category: CATEGORY_STYLIZE, attributes: &[a("inputRadius", 20.0, 1.0, 100.0)] },
    FilterInfo { name: "CIPixellate", category: CATEGORY_STYLIZE, attributes: &[a("inputScale", 8.0, 1.0, 100.0)] },
    FilterInfo { name: "CIPointillize", category: CATEGORY_STYLIZE, attributes: &[a("inputRadius", 20.0, 1.0, 100.0)] },
    FilterInfo { name: "CIDotScreen", category: CATEGORY_HALFTONE, attributes: &[ANGLE, a("inputWidth", 6.0, 1.0, 50.0), a("inputSharpness", 0.7, 0.0, 1.0)] },
    FilterInfo { name: "CILineScreen", category: CATEGORY_HALFTONE, attributes: &[ANGLE, a("inputWidth", 6.0, 1.0, 50.0), a("inputSharpness", 0.7, 0.0, 1.0)] },
    FilterInfo { name: "CICircularScreen", category: CATEGORY_HALFTONE, attributes: &[a("inputWidth", 6.0, 1.0, 50.0), a("inputSharpness", 0.7, 0.0, 1.0)] },
    FilterInfo { name: "CIHatchedScreen", category: CATEGORY_HALFTONE, attributes: &[ANGLE, a("inputWidth", 6.0, 1.0, 50.0), a("inputSharpness", 0.7, 0.0, 1.0)] },
    FilterInfo { name: "CICMYKHalftone", category: CATEGORY_HALFTONE, attributes: &[ANGLE, a("inputWidth", 6.0, 1.0, 50.0), a("inputSharpness", 0.7, 0.0, 1.0), a("inputGCR", 1.0, 0.0, 1.0), a("inputUCR", 0.5, 0.0, 1.0)] },
    FilterInfo { name: "CIKaleidoscope", category: CATEGORY_TILE, attributes: &[a("inputCount", 6.0, 1.0, 64.0), ANGLE] },
    FilterInfo { name: "CIOpTile", category: CATEGORY_TILE, attributes: &[a("inputScale", 2.8, 0.1, 10.0), ANGLE, a("inputWidth", 65.0, 1.0, 1000.0)] },
    FilterInfo { name: "CIAffineTile", category: CATEGORY_TILE, attributes: &[] },
    FilterInfo { name: "CIGaussianBlur", category: CATEGORY_BLUR, attributes: &[a("inputRadius", 10.0, 0.0, 100.0)] },
    FilterInfo { name: "CIBoxBlur", category: CATEGORY_BLUR, attributes: &[a("inputRadius", 10.0, 1.0, 100.0)] },
    FilterInfo { name: "CIMotionBlur", category: CATEGORY_BLUR, attributes: &[a("inputRadius", 20.0, 0.0, 100.0), a("inputAngle", 0.0, -PI, PI)] },
    FilterInfo { name: "CIZoomBlur", category: CATEGORY_BLUR, attributes: &[a("inputAmount", 20.0, 0.0, 200.0)] },
    FilterInfo { name: "CIDiscBlur", category: CATEGORY_BLUR, attributes: &[a("inputRadius", 8.0, 0.0, 50.0)] },
    FilterInfo { name: "CISourceOverCompositing", category: CATEGORY_COMPOSITE, attributes: &[] },
    FilterInfo { name: "CIMultiplyBlendMode", category: CATEGORY_COMPOSITE, attributes: &[] },
    FilterInfo { name: "CIScreenBlendMode", category: CATEGORY_COMPOSITE, attributes: &[] },
];

/// Every catalogued filter name, in catalogue order.
pub fn all_filter_names() -> Vec<&'static str> {
    FILTERS.iter().map(|f| f.name).collect()
}

/// `CIFilter.filterNames(inCategory:)`: the filters in `category`; `None` lists them all.
pub fn filter_names(category: Option<&str>) -> Vec<&'static str> {
    FILTERS.iter().filter(|f| category.is_none_or(|c| f.category == c)).map(|f| f.name).collect()
}

/// `CIFilter.attributes` for the filter called `name`.
pub fn attributes(name: &str) -> Option<&'static FilterInfo> {
    FILTERS.iter().find(|f| f.name == name)
}

/// The default value of `key` on filter `name`.
pub fn default_value(name: &str, key: &str) -> Option<f32> {
    attributes(name)?.attributes.iter().find(|a| a.key == key).map(|a| a.default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_prefixed() {
        let mut n = all_filter_names();
        assert!(n.iter().all(|s| s.starts_with("CI")));
        n.sort();
        let len = n.len();
        n.dedup();
        assert_eq!(n.len(), len);
    }

    #[test]
    fn category_listing_filters_by_category() {
        let d = filter_names(Some(CATEGORY_DISTORTION));
        assert_eq!(d, ["CITwirlDistortion", "CIVortexDistortion", "CIPinchDistortion", "CIBumpDistortion", "CIGlassDistortion"]);
        assert_eq!(filter_names(None).len(), all_filter_names().len());
        assert!(filter_names(Some("CICategoryNothing")).is_empty());
    }

    #[test]
    fn attributes_use_the_reference_keys_and_defaults() {
        assert_eq!(default_value("CIUnsharpMask", "inputRadius"), Some(2.5));
        assert_eq!(default_value("CISharpenLuminance", "inputSharpness"), Some(0.4));
        assert_eq!(default_value("CIPinchDistortion", "inputScale"), Some(0.5));
        assert_eq!(default_value("CIBloom", "inputBogus"), None);
        assert!(attributes("CINope").is_none());
    }

    #[test]
    fn every_default_lies_in_its_range() {
        for f in FILTERS {
            for at in f.attributes {
                assert!(at.default >= at.min && at.default <= at.max, "{} {}", f.name, at.key);
            }
        }
    }
}
