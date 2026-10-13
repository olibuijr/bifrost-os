//! The saved edit of a photo, kept as a recipe rather than a copy. The
//! original file is never written: the recipe is one line of the settings
//! store (`edit:<path>`), and the picture is decoded and adjusted with it
//! whenever it is shown, in the grid, the viewer, the slideshow or as a
//! wallpaper. An unedited photo has no recipe.

use super::edit::{Aspect, Edit, Filter};

const ASPECTS: [(Aspect, &str); 6] = [
    (Aspect::Free, "free"),
    (Aspect::Original, "original"),
    (Aspect::Square, "square"),
    (Aspect::R4x3, "4x3"),
    (Aspect::R3x2, "3x2"),
    (Aspect::R16x9, "16x9"),
];

const FILTERS: [(Filter, &str); 6] = [
    (Filter::Original, "original"),
    (Filter::Vivid, "vivid"),
    (Filter::Mono, "mono"),
    (Filter::Noir, "noir"),
    (Filter::Warm, "warm"),
    (Filter::Sepia, "sepia"),
];

/// The recipe as one settings value: `key=value` pairs separated by `;`.
pub fn encode(e: &Edit) -> String {
    let aspect = name(&ASPECTS, e.aspect);
    let filter = name(&FILTERS, e.filter);
    format!(
        "turns={};aspect={};zoom={};cx={};cy={};straighten={};filter={};brightness={};contrast={};exposure={};saturation={};warmth={};blur={};vignette={};fliph={};flipv={};auto={}",
        e.turns,
        aspect,
        e.crop_zoom,
        e.crop_center.0,
        e.crop_center.1,
        e.straighten,
        filter,
        e.brightness,
        e.contrast,
        e.exposure,
        e.saturation,
        e.warmth,
        e.blur,
        e.vignette,
        u8::from(e.flip_h),
        u8::from(e.flip_v),
        u8::from(e.auto)
    )
}

/// The edit a recipe describes. Keys it does not know are skipped; a value
/// that does not parse makes the whole recipe invalid (None), as does an
/// empty one.
pub fn decode(text: &str) -> Option<Edit> {
    let mut e = Edit::default();
    let mut any = false;
    for pair in text.split(';') {
        let (key, value) = pair.split_once('=')?;
        any = true;
        match key {
            "turns" => e.turns = value.parse::<u8>().ok()? % 4,
            "aspect" => e.aspect = lookup(&ASPECTS, value)?,
            "zoom" => e.crop_zoom = number(value)?,
            "cx" => e.crop_center.0 = number(value)?,
            "cy" => e.crop_center.1 = number(value)?,
            "straighten" => e.straighten = number(value)?,
            "filter" => e.filter = lookup(&FILTERS, value)?,
            "brightness" => e.brightness = number(value)?,
            "contrast" => e.contrast = number(value)?,
            "exposure" => e.exposure = number(value)?,
            "saturation" => e.saturation = number(value)?,
            "warmth" => e.warmth = number(value)?,
            "blur" => e.blur = number(value)?,
            "vignette" => e.vignette = number(value)?,
            "fliph" => e.flip_h = value == "1",
            "flipv" => e.flip_v = value == "1",
            "auto" => e.auto = value == "1",
            _ => {}
        }
    }
    any.then_some(e)
}

fn name<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(v, _)| *v == value).map_or("", |(_, n)| *n)
}

fn lookup<T: Copy>(table: &[(T, &'static str)], name: &str) -> Option<T> {
    table.iter().find(|(_, n)| *n == name).map(|(v, _)| *v)
}

fn number(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|x| x.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_round_trips() {
        let e = Edit {
            turns: 3,
            aspect: Aspect::R16x9,
            crop_zoom: 1.5,
            crop_center: (0.25, 0.75),
            straighten: -4.5,
            filter: Filter::Sepia,
            brightness: 0.2,
            contrast: -0.3,
            exposure: 0.1,
            saturation: -1.0,
            warmth: 0.5,
            blur: 0.4,
            vignette: 1.0,
            flip_h: true,
            flip_v: false,
            auto: true,
        };
        assert_eq!(decode(&encode(&e)), Some(e));
        assert_eq!(decode(&encode(&Edit::default())), Some(Edit::default()));
    }

    #[test]
    fn garbage_is_refused() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("turns=x"), None);
        assert_eq!(decode("aspect=wide"), None);
        assert_eq!(decode("filter=neon"), None);
        assert_eq!(decode("brightness"), None);
    }

    #[test]
    fn unknown_keys_are_skipped() {
        assert_eq!(decode("future=1;turns=2").map(|e| e.turns), Some(2));
    }
}
