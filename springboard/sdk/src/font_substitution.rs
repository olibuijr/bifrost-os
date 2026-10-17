//! Family substitution: reference family names that BifrOSt does not ship resolve to open equivalents by class.
//!
//! Documents and apps name families such as Helvetica Neue, Avenir, Futura, SF Pro Rounded, Hiragino or PingFang.
//! `substitute` maps such a name to a `FamilyClass` and an ordered list of open families that stand in for it;
//! `class_for_spec` picks a generic class from descriptor traits when the name is unknown; `with_design` applies a
//! system design (rounded, serif, monospaced) to a descriptor; `cascade_list` returns the ordered fallback families
//! for language tags. The tables are data only. No original face ships: a substitute is another design of the same
//! class, never a copy.
use bifrost_render::fontinfo::Traits;
use bifrost_render::fontobj::FontSpec;

/// The style class a family belongs to, which decides its open equivalent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyClass {
    GrotesqueSans,
    GeometricSans,
    HumanistSans,
    DidoneSerif,
    TransitionalSerif,
    Slab,
    Script,
    Monospaced,
    Rounded,
    CjkGothic,
    CjkMincho,
    CjkKai,
    CjkRounded,
}

/// A system design applied to a descriptor (`.rounded`, `.serif`, `.monospaced`; `.default` leaves it alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemDesign {
    Default,
    Rounded,
    Serif,
    Monospaced,
}

impl FamilyClass {
    /// The open families that stand in for the class, preferred first.
    pub fn equivalents(self) -> &'static [&'static str] {
        match self {
            FamilyClass::GrotesqueSans => &["Inter", "Geist", "Source Sans 3"],
            FamilyClass::GeometricSans => &["Outfit", "Poppins", "Geist", "Inter"],
            FamilyClass::HumanistSans => &["Source Sans 3", "IBM Plex Sans", "Inter"],
            FamilyClass::DidoneSerif => &["Playfair Display", "Noto Serif", "Source Serif 4"],
            FamilyClass::TransitionalSerif => &["Noto Serif", "Source Serif 4", "Libre Baskerville"],
            FamilyClass::Slab => &["Roboto Slab", "Zilla Slab", "Noto Serif"],
            FamilyClass::Script => &["Pacifico", "Caveat", "Dancing Script"],
            FamilyClass::Monospaced => &["JetBrains Mono", "Geist Mono", "Source Code Pro", "Fira Code"],
            FamilyClass::Rounded => &["Nunito", "Quicksand", "M PLUS Rounded 1c", "Inter"],
            FamilyClass::CjkGothic => &["Noto Sans CJK JP", "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Sans CJK KR"],
            FamilyClass::CjkMincho => &["Noto Serif CJK JP", "Noto Serif CJK SC", "Noto Serif CJK TC", "Noto Serif CJK KR"],
            FamilyClass::CjkKai => &["Noto Serif CJK TC", "Noto Serif CJK SC", "Noto Serif CJK JP"],
            FamilyClass::CjkRounded => &["M PLUS Rounded 1c", "Noto Sans CJK JP", "Noto Sans CJK SC"],
        }
    }
}

impl SystemDesign {
    /// The class a design maps to; None for the default design.
    pub fn class(self) -> Option<FamilyClass> {
        match self {
            SystemDesign::Default => None,
            SystemDesign::Rounded => Some(FamilyClass::Rounded),
            SystemDesign::Serif => Some(FamilyClass::TransitionalSerif),
            SystemDesign::Monospaced => Some(FamilyClass::Monospaced),
        }
    }
}

use FamilyClass::*;

/// Reference family name (lowercase) to class.
const TABLE: &[(&str, FamilyClass)] = &[
    // grotesque and neo-grotesque sans
    ("helvetica", GrotesqueSans), ("helvetica neue", GrotesqueSans), ("arial", GrotesqueSans), ("arial narrow", GrotesqueSans),
    ("arial black", GrotesqueSans), ("sf pro", GrotesqueSans), ("sf pro text", GrotesqueSans), ("sf pro display", GrotesqueSans),
    ("sf compact", GrotesqueSans), ("sf compact text", GrotesqueSans), ("sf compact display", GrotesqueSans),
    ("sf arabic", GrotesqueSans), ("sf hebrew", GrotesqueSans), ("sf armenian", GrotesqueSans), ("sf georgian", GrotesqueSans),
    ("system font", GrotesqueSans), ("akzidenz-grotesk", GrotesqueSans), ("univers", GrotesqueSans), ("roboto", GrotesqueSans),
    ("arimo", GrotesqueSans), ("liberation sans", GrotesqueSans), ("impact", GrotesqueSans), ("haettenschweiler", GrotesqueSans),
    ("din alternate", GrotesqueSans), ("din condensed", GrotesqueSans), ("avenir next condensed", GrotesqueSans),
    // geometric sans
    ("avenir", GeometricSans), ("avenir next", GeometricSans), ("futura", GeometricSans), ("gill sans", GeometricSans),
    ("century gothic", GeometricSans), ("optima", GeometricSans), ("trebuchet ms", GeometricSans), ("copperplate", GeometricSans),
    ("didot sans", GeometricSans), ("kohinoor devanagari", GeometricSans), ("kohinoor bangla", GeometricSans),
    // humanist sans
    ("lucida grande", HumanistSans), ("lucida sans", HumanistSans), ("verdana", HumanistSans), ("tahoma", HumanistSans),
    ("calibri", HumanistSans), ("segoe ui", HumanistSans), ("geneva", HumanistSans), ("myriad pro", HumanistSans),
    ("frutiger", HumanistSans), ("candara", HumanistSans), ("corbel", HumanistSans), ("noteworthy", HumanistSans),
    ("pt sans", HumanistSans), ("al bayan", HumanistSans), ("damascus", HumanistSans),
    ("geeza pro", HumanistSans), ("kefa", HumanistSans), ("mshtakan", HumanistSans), ("nadeem", HumanistSans),
    // didone and display serif
    ("didot", DidoneSerif), ("bodoni 72", DidoneSerif), ("bodoni 72 oldstyle", DidoneSerif), ("bodoni 72 smallcaps", DidoneSerif),
    ("bodoni mt", DidoneSerif), ("new york", DidoneSerif), ("playfair display", DidoneSerif),
    // transitional and old-style serif
    ("times", TransitionalSerif), ("times new roman", TransitionalSerif), ("georgia", TransitionalSerif),
    ("palatino", TransitionalSerif), ("book antiqua", TransitionalSerif), ("baskerville", TransitionalSerif),
    ("hoefler text", TransitionalSerif), ("garamond", TransitionalSerif), ("cambria", TransitionalSerif),
    ("charter", TransitionalSerif), ("cochin", TransitionalSerif), ("iowan old style", TransitionalSerif),
    ("big caslon", TransitionalSerif), ("bookman", TransitionalSerif), ("liberation serif", TransitionalSerif),
    ("tinos", TransitionalSerif), ("superclarendon", TransitionalSerif), ("sathu", TransitionalSerif),
    // slab
    ("rockwell", Slab), ("clarendon", Slab), ("american typewriter", Slab), ("museo slab", Slab),
    ("roboto slab", Slab), ("zilla slab", Slab),
    // script and handwriting
    ("snell roundhand", Script), ("zapfino", Script), ("brush script mt", Script), ("bradley hand", Script),
    ("savoye let", Script), ("chalkboard", Script), ("chalkboard se", Script), ("comic sans ms", Script),
    ("marker felt", Script), ("segoe script", Script), ("lucida handwriting", Script), ("party let", Script),
    ("papyrus", Script), ("blackmoor let", Script),
    // monospaced
    ("sf mono", Monospaced), ("menlo", Monospaced), ("monaco", Monospaced), ("courier", Monospaced),
    ("courier new", Monospaced), ("consolas", Monospaced), ("andale mono", Monospaced), ("lucida console", Monospaced),
    ("liberation mono", Monospaced), ("cousine", Monospaced), ("dejavu sans mono", Monospaced), ("osaka-mono", Monospaced),
    // rounded
    ("sf pro rounded", Rounded), ("sf compact rounded", Rounded), ("arial rounded mt bold", Rounded),
    ("varela round", Rounded), ("comfortaa", Rounded), ("avenir next rounded", Rounded),
    // CJK gothic (sans), including the decorative Korean and Chinese display faces that have no closer open class
    ("hiragino sans", CjkGothic), ("hiragino kaku gothic pro", CjkGothic), ("hiragino kaku gothic pron", CjkGothic),
    ("hiragino kaku gothic std", CjkGothic), ("hiragino kaku gothic stdn", CjkGothic), ("ヒラギノ角ゴ pro", CjkGothic),
    ("ヒラギノ角ゴ pron", CjkGothic), ("ヒラギノ角ゴ std", CjkGothic), ("ヒラギノ角ゴ stdn", CjkGothic),
    ("hiragino sans gb", CjkGothic), ("pingfang sc", CjkGothic), ("pingfang tc", CjkGothic), ("pingfang hk", CjkGothic),
    ("pingfang mo", CjkGothic), ("heiti sc", CjkGothic), ("heiti tc", CjkGothic), ("stheiti", CjkGothic),
    ("lantinghei sc", CjkGothic), ("lantinghei tc", CjkGothic), ("lantinghei", CjkGothic), ("lantingheitc-extralight", CjkGothic),
    ("yu gothic", CjkGothic), ("yu gothic medium", CjkGothic), ("yu gothic bold", CjkGothic), ("meiryo", CjkGothic),
    ("biz_udgothic_stub", CjkGothic), ("nanumgothic", CjkGothic), ("nanum gothic", CjkGothic), ("malgun gothic", CjkGothic),
    ("ms gothic", CjkGothic), ("microsoft yahei", CjkGothic), ("microsoft jhenghei", CjkGothic), ("lingwaisc-medium", CjkGothic),
    ("lingwaitc-medium", CjkGothic), ("儷黑 pro", CjkGothic), ("华文黑体", CjkGothic), ("华文细黑", CjkGothic),
    ("toppanbunkyugothicpr6n", CjkGothic), ("toppanbunkyumidashigothicstdn-extrabold", CjkGothic), ("osaka", CjkGothic),
    ("hannotate", CjkGothic), ("hanzipen", CjkGothic), ("gungseouche", CjkGothic),
    ("nanumscript", CjkGothic), ("bmdohyeon-regular", CjkGothic), ("bmhanna11yrs-regular", CjkGothic),
    ("bmhannaair-regular", CjkGothic), ("bmhannapro-regular", CjkGothic), ("bmjua-regular", CjkGothic),
    ("bmkiranghaerang-regular", CjkGothic), ("baoli", CjkGothic), ("weibeisc-bold", CjkGothic), ("weibeitc-bold", CjkGothic),
    ("xingkai", CjkGothic), ("wawasc-regular", CjkGothic), ("wawatc-regular", CjkGothic), ("yuppysc-regular", CjkGothic),
    ("yuppytc-regular", CjkGothic),
    // CJK mincho (serif)
    ("hiragino mincho pro", CjkMincho), ("hiragino mincho pron", CjkMincho), ("ヒラギノ明朝 pro", CjkMincho),
    ("ヒラギノ明朝 pron", CjkMincho), ("yumincho", CjkMincho), ("yu mincho", CjkMincho), ("ms mincho", CjkMincho),
    ("biz_udmincho-regular_stub", CjkMincho), ("toppanbunkyuminchopr6n-regular", CjkMincho),
    ("toppanbunkyumidashiminchostdn-extrabold", CjkMincho), ("songti sc", CjkMincho), ("songti tc", CjkMincho),
    ("songti", CjkMincho), ("stsong", CjkMincho), ("simsun", CjkMincho), ("simsong", CjkMincho), ("儷宋 pro", CjkMincho),
    ("lisong pro", CjkMincho), ("hycusong-caict", CjkMincho), ("nanummyeongjo", CjkMincho), ("nanum myeongjo", CjkMincho),
    ("bmyeongsung-regular", CjkMincho), ("pcmyoungjo", CjkMincho), ("mingliu", CjkMincho),
    // CJK kai and brush
    ("kaiti sc", CjkKai), ("kaiti tc", CjkKai), ("kaiti", CjkKai), ("stkaiti", CjkKai), ("biaukai", CjkKai), ("kai", CjkKai),
    ("华文仿宋", CjkKai), ("stfangsong", CjkKai), ("klee", CjkKai), ("klee one", CjkKai), ("kyokasho", CjkKai),
    ("kyokashotai", CjkKai),
    // CJK rounded
    ("hiragino maru gothic pro", CjkRounded), ("hiragino maru gothic pron", CjkRounded), ("ヒラギノ丸ゴ pron", CjkRounded),
    ("ヒラギノ丸ゴ pron w4", CjkRounded), ("ヒラギノ丸ゴ pro", CjkRounded), ("yuanti sc", CjkRounded), ("yuanti tc", CjkRounded),
    ("yuanti", CjkRounded), ("tsukushiamarugothic", CjkRounded), ("tsukushibmarugothic", CjkRounded),
];

/// The class of a reference family name (case and surrounding space ignored), None when it is not in the table.
pub fn class_of_family(family: &str) -> Option<FamilyClass> {
    let key = family.trim().to_lowercase();
    TABLE.iter().find(|(n, _)| *n == key).map(|(_, c)| *c)
}

/// Reference family name (lowercase) to the open face that stands in for it, tried before the class list. Each face
/// is packaged on Arch (`packaging/arch/runtime-packages.txt`) and is the family name fontconfig reports for it.
const FACES: &[(&str, &str)] = &[
    ("avenir", "Lato"), ("avenir next", "Lato"), ("avenir next condensed", "Lato"),
    ("arial black", "Noto Sans"), ("arial unicode ms", "Noto Sans"),
    ("andale mono", "DejaVu Sans Mono"),
    ("athelas", "Liberation Serif"),
    ("baskerville", "Libertinus Serif"), ("big caslon", "Libertinus Serif"), ("cochin", "Libertinus Serif"),
    ("charter", "Charis SIL"),
    ("american typewriter", "IBM Plex Serif"),
    // reference typefaces shipped by macOS (licensed, never shipped here); open faces from packaged families
    ("bank gothic", "Roboto Condensed"), ("bebas neue", "Roboto Condensed"), ("din alternate", "Roboto Condensed"),
    ("din condensed", "Roboto Condensed"), ("druk", "Roboto Condensed"),
    ("bodoni svtytwo", "Noto Serif Display"), ("bodoni svtytwo os", "Noto Serif Display"),
    ("bodoni svtytwo sc", "Noto Serif Display"), ("didot", "Noto Serif Display"),
    ("canela", "Noto Serif Display"), ("canela deck", "Noto Serif Display"), ("canela text", "Noto Serif Display"),
    ("bordeaux roman bold let", "Libertinus Serif"), ("bordeaux roman bold let fonts", "Libertinus Serif"),
    ("brill", "Libertinus Serif"),
    ("braganza", "TeX Gyre Chorus"),
    ("carlito", "Carlito"), ("calibri", "Carlito"),
    ("adelle sans", "IBM Plex Sans"), ("euphemia ucas", "Noto Sans"), ("euphemiacas", "Noto Sans"),
    ("coolvetica", "Montserrat"), ("futura", "Montserrat"),
    ("fakt slab stencil pro", "IBM Plex Serif"),
];

/// The open face mapped to `family` in `FACES`, if any.
pub fn face_of_family(family: &str) -> Option<&'static str> {
    let key = family.trim().to_lowercase();
    FACES.iter().find(|(n, _)| *n == key).map(|(_, f)| *f)
}

/// The open families for `family`: its own face first, then the equivalents of its class. Empty when neither is known.
fn equivalents_of(family: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = face_of_family(family).into_iter().collect();
    if let Some(c) = class_of_family(family) {
        for f in c.equivalents() {
            if !out.contains(f) {
                out.push(f);
            }
        }
    }
    out
}

/// The open families that stand in for `family`, preferred first. Empty when the name is unknown.
pub fn substitute(family: &str) -> Vec<&'static str> {
    equivalents_of(family)
}

/// The generic class for a descriptor whose family is unknown: fixed pitch is monospaced, everything else is the
/// grotesque sans that the system font belongs to.
pub fn class_for_spec(spec: &FontSpec) -> FamilyClass {
    if spec.trait_mask & Traits::MONO != 0 && spec.traits.is_mono() {
        Monospaced
    } else {
        GrotesqueSans
    }
}

/// The descriptors to try for `spec` when its own family is not installed: one per open equivalent of the family's
/// class (or of the generic class for an unknown name), in preference order. Empty when `spec` names no family.
pub fn candidates(spec: &FontSpec) -> Vec<FontSpec> {
    let Some(name) = spec.family.as_deref() else { return Vec::new() };
    let list = match equivalents_of(name) {
        v if !v.is_empty() => v,
        _ => class_for_spec(spec).equivalents().to_vec(),
    };
    list.into_iter()
        .filter(|f| !f.eq_ignore_ascii_case(name))
        .map(|f| {
            let mut s = spec.clone();
            s.family = Some(f.to_string());
            s
        })
        .collect()
}

/// `spec` with a system design applied: the family becomes the first equivalent of the design's class (the
/// resolver still tries the rest), and the monospaced design also sets the fixed-pitch trait. `Default` returns
/// `spec` unchanged.
pub fn with_design(spec: FontSpec, design: SystemDesign) -> FontSpec {
    let Some(class) = design.class() else { return spec };
    let spec = spec.family(class.equivalents()[0]);
    if design == SystemDesign::Monospaced {
        spec.mono()
    } else {
        spec
    }
}

/// The ordered fallback families for characters a face lacks, for `languages` (BCP 47 tags, most preferred first).
/// Each language contributes its script families; repeats are dropped. Unknown tags contribute nothing.
pub fn cascade_list(languages: &[&str]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for tag in languages {
        let t = tag.trim().to_lowercase().replace('_', "-");
        let lang = t.split('-').next().unwrap_or("");
        let fams: &[&'static str] = match lang {
            "ja" => &["Noto Sans CJK JP", "Noto Serif CJK JP"],
            "ko" => &["Noto Sans CJK KR", "Noto Serif CJK KR"],
            "zh" => {
                let trad = t.contains("hant") || t.ends_with("-tw") || t.ends_with("-hk") || t.ends_with("-mo");
                if trad {
                    &["Noto Sans CJK TC", "Noto Serif CJK TC"]
                } else {
                    &["Noto Sans CJK SC", "Noto Serif CJK SC"]
                }
            }
            "ar" | "fa" | "ur" => &["Noto Naskh Arabic", "Noto Sans Arabic"],
            "he" => &["Noto Sans Hebrew"],
            "hi" | "mr" | "ne" => &["Noto Sans Devanagari"],
            "th" => &["Noto Sans Thai"],
            "bn" => &["Noto Sans Bengali"],
            "ta" => &["Noto Sans Tamil"],
            "el" | "ru" | "uk" | "bg" | "sr" => &["Noto Sans"],
            _ => &[],
        };
        for f in fams {
            if !out.contains(f) {
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_families_map_to_their_class() {
        assert_eq!(class_of_family("Helvetica Neue"), Some(GrotesqueSans));
        assert_eq!(class_of_family("  AVENIR "), Some(GeometricSans));
        assert_eq!(class_of_family("Futura"), Some(GeometricSans));
        assert_eq!(class_of_family("Didot"), Some(DidoneSerif));
        assert_eq!(class_of_family("Rockwell"), Some(Slab));
        assert_eq!(class_of_family("Zapfino"), Some(Script));
        assert_eq!(class_of_family("Lucida Grande"), Some(HumanistSans));
        assert_eq!(class_of_family("SF Pro Rounded"), Some(Rounded));
        assert_eq!(class_of_family("Hiragino Sans"), Some(CjkGothic));
        assert_eq!(class_of_family("PingFang SC"), Some(CjkGothic));
        assert_eq!(class_of_family("YuMincho"), Some(CjkMincho));
        assert_eq!(class_of_family("Kaiti"), Some(CjkKai));
        assert_eq!(class_of_family("Yuanti SC"), Some(CjkRounded));
        assert_eq!(class_of_family("Nothing Like This"), None);
    }

    #[test]
    fn substitute_lists_open_equivalents_preferred_first() {
        assert_eq!(substitute("Helvetica Neue")[0], "Inter");
        assert_eq!(substitute("Futura")[0], "Montserrat");
        assert_eq!(substitute("Hiragino Mincho Pro")[0], "Noto Serif CJK JP");
        assert!(substitute("Nothing Like This").is_empty());
    }

    #[test]
    fn table_keys_are_unique_and_lowercase() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in TABLE {
            assert_eq!(*n, n.to_lowercase(), "key {n} must be lowercase");
            assert!(seen.insert(*n), "duplicate key {n}");
        }
        let mut faces = std::collections::HashSet::new();
        for (n, f) in FACES {
            assert_eq!(*n, n.to_lowercase(), "face key {n} must be lowercase");
            assert!(!f.is_empty() && faces.insert(*n), "face {n} needs one open family, once");
        }
    }

    #[test]
    fn open_faces_stand_in_for_licensed_families_first() {
        assert_eq!(substitute("Avenir Next")[0], "Lato");
        assert_eq!(substitute("  AVENIR NEXT CONDENSED ")[0], "Lato");
        assert_eq!(substitute("Charter")[0], "Charis SIL");
        assert_eq!(substitute("Andale Mono")[0], "DejaVu Sans Mono");
        assert_eq!(substitute("Arial Black")[0], "Noto Sans");
        assert_eq!(substitute("Arial Unicode MS"), vec!["Noto Sans"]);
        assert_eq!(substitute("Baskerville")[0], "Libertinus Serif");
        assert_eq!(substitute("American Typewriter")[0], "IBM Plex Serif");
        assert_eq!(substitute("Futura")[0], "Montserrat");
        let avenir = candidates(&FontSpec::new().family("Avenir"));
        assert_eq!(avenir[0].family.as_deref(), Some("Lato"));
        assert!(avenir.iter().all(|s| s.family.as_deref() != Some("Avenir")));
    }

    #[test]
    fn reference_typefaces_resolve_to_packaged_open_faces() {
        let pairs = [
            ("Bank Gothic", "Roboto Condensed"), ("Bebas Neue", "Roboto Condensed"),
            ("DIN Alternate", "Roboto Condensed"), ("DIN Condensed", "Roboto Condensed"), ("Druk", "Roboto Condensed"),
            ("Bodoni SvtyTwo", "Noto Serif Display"), ("Bodoni SvtyTwo OS", "Noto Serif Display"),
            ("Bodoni SvtyTwo SC", "Noto Serif Display"), ("Didot", "Noto Serif Display"),
            ("Canela", "Noto Serif Display"), ("Canela Deck", "Noto Serif Display"), ("Canela Text", "Noto Serif Display"),
            ("Bordeaux Roman Bold LET", "Libertinus Serif"), ("Brill", "Libertinus Serif"),
            ("Braganza", "TeX Gyre Chorus"), ("Carlito", "Carlito"), ("Calibri", "Carlito"),
            ("Adelle Sans", "IBM Plex Sans"), ("Euphemia UCAS", "Noto Sans"), ("EuphemiaCAS", "Noto Sans"),
            ("Coolvetica", "Montserrat"), ("Futura", "Montserrat"), ("Fakt Slab Stencil Pro", "IBM Plex Serif"),
        ];
        for (reference, open) in pairs {
            assert_eq!(face_of_family(reference), Some(open), "{reference}");
            assert_eq!(substitute(reference)[0], open, "{reference}");
        }
        assert_eq!(face_of_family("Blackmoor LET"), None);
    }

    #[test]
    fn unknown_family_falls_back_by_generic_class() {
        assert_eq!(class_for_spec(&FontSpec::new().family("Mystery")), GrotesqueSans);
        assert_eq!(class_for_spec(&FontSpec::new().family("Mystery").mono()), Monospaced);
        let c = candidates(&FontSpec::new().family("Mystery").mono());
        assert_eq!(c[0].family.as_deref(), Some("JetBrains Mono"));
        assert!(c.iter().all(|s| s.traits == FontSpec::new().mono().traits));
    }

    #[test]
    fn candidates_keep_the_other_attributes_and_skip_the_original() {
        let spec = FontSpec::new().family("Inter").weight(0.5).size(14.0);
        let c = candidates(&spec);
        assert!(c.iter().all(|s| s.family.as_deref() != Some("Inter")));
        assert!(c.iter().all(|s| s.weight == Some(0.5) && s.size == Some(14.0)));
        assert!(candidates(&FontSpec::new()).is_empty());
    }

    #[test]
    fn system_designs_map_to_rounded_serif_and_mono_equivalents() {
        let base = FontSpec::new().size(13.0);
        assert_eq!(with_design(base.clone(), SystemDesign::Default), base);
        assert_eq!(with_design(base.clone(), SystemDesign::Rounded).family.as_deref(), Some("Nunito"));
        assert_eq!(with_design(base.clone(), SystemDesign::Serif).family.as_deref(), Some("Noto Serif"));
        let m = with_design(base, SystemDesign::Monospaced);
        assert_eq!(m.family.as_deref(), Some("JetBrains Mono"));
        assert!(m.traits.is_mono());
        assert_eq!(m.size, Some(13.0));
    }

    #[test]
    fn cascade_list_orders_families_by_language() {
        assert_eq!(cascade_list(&["ja"]), vec!["Noto Sans CJK JP", "Noto Serif CJK JP"]);
        assert_eq!(cascade_list(&["ko"])[0], "Noto Sans CJK KR");
        assert_eq!(cascade_list(&["zh-Hans"])[0], "Noto Sans CJK SC");
        assert_eq!(cascade_list(&["zh_Hant"])[0], "Noto Sans CJK TC");
        assert_eq!(cascade_list(&["zh-TW"])[0], "Noto Sans CJK TC");
        assert_eq!(cascade_list(&["zh"])[0], "Noto Sans CJK SC");
        assert_eq!(cascade_list(&["ar"])[0], "Noto Naskh Arabic");
        assert_eq!(cascade_list(&["hi"]), vec!["Noto Sans Devanagari"]);
        assert!(cascade_list(&["xx", "en"]).is_empty());
        let both = cascade_list(&["ko", "ja", "ko"]);
        assert_eq!(both[0], "Noto Sans CJK KR");
        assert_eq!(both.iter().filter(|f| **f == "Noto Sans CJK KR").count(), 1);
        assert!(both.contains(&"Noto Sans CJK JP"));
    }
}
