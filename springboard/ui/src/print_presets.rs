//! Print presets (the print sheet's Presets popup): named sets of page settings that the user saves and reapplies.
//!
//! A [`Preset`] keeps paper, orientation, margins, copies, scale, two-sided and colour. It does not keep the printer, the
//! page range or the PDF path: those belong to the job, not to the setup. [`PresetStore`] holds them in a sorted-by-
//! insertion list with unique names, and reads and writes a plain text form so an app keeps it in its defaults or a file:
//!
//! ```text
//! [Duplex draft]
//! paper=A4
//! orientation=portrait
//! margins=72 72 72 72
//! copies=1
//! scaling=1
//! sides=two-sided-long-edge
//! color=monochrome
//! ```
//!
//! Reading is lenient, so a newer file still loads in an older build: unknown keys are ignored, a value that does not parse
//! keeps the default, and a section with a bad or repeated name is skipped.

use crate::print::{ColorMode, Orientation, PaperSize, PrintInfo, Sides, SCALING_RANGE};

/// The most copies a preset stores (the print dialog's limit).
const MAX_COPIES: u32 = 9999;

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub paper: PaperSize,
    pub orientation: Orientation,
    /// Top, left, bottom, right, in points.
    pub margins: [f32; 4],
    pub copies: u32,
    pub scaling: f32,
    pub sides: Sides,
    pub color: ColorMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// The name is empty, too long, or has a bracket or line break.
    BadName(String),
    /// Another preset has this name.
    Duplicate(String),
    /// No preset has this name.
    Unknown(String),
}

/// Whether `name` can name a preset: 1 to 64 characters, not blank at either end, without `[`, `]` or a line break.
pub fn valid_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && t.chars().count() <= 64 && !t.contains(['[', ']', '\n', '\r'])
}

impl Preset {
    /// The settings of `info` under `name`. Copies are kept between 1 and 9999 and the scale inside its range.
    pub fn from_info(name: &str, info: &PrintInfo) -> Result<Preset, PresetError> {
        if !valid_name(name) {
            return Err(PresetError::BadName(name.to_string()));
        }
        Ok(Preset {
            name: name.trim().to_string(),
            paper: info.paper,
            orientation: info.orientation,
            margins: info.margins,
            copies: info.copies.clamp(1, MAX_COPIES),
            scaling: info.scaling(),
            sides: info.sides,
            color: info.color,
        })
    }

    /// Writes the preset's settings into `info`. The page range, the printer and the save path are left as they were.
    pub fn apply_to(&self, info: &mut PrintInfo) {
        info.paper = self.paper;
        info.orientation = self.orientation;
        info.margins = self.margins;
        info.copies = self.copies;
        info.scaling_factor = self.scaling;
        info.sides = self.sides;
        info.color = self.color;
    }

    fn write(&self, out: &mut String) {
        let [t, l, b, r] = self.margins;
        out.push_str(&format!("[{}]\n", self.name));
        out.push_str(&format!("paper={}\n", self.paper.media()));
        out.push_str(&format!("orientation={}\n", if self.orientation == Orientation::Landscape { "landscape" } else { "portrait" }));
        out.push_str(&format!("margins={t} {l} {b} {r}\n"));
        out.push_str(&format!("copies={}\n", self.copies));
        out.push_str(&format!("scaling={}\n", self.scaling));
        out.push_str(&format!("sides={}\n", self.sides.keyword()));
        out.push_str(&format!("color={}\n", self.color.keyword()));
    }
}

/// The presets an app keeps, in the order they were added.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetStore {
    presets: Vec<Preset>,
}

fn parse_sides(v: &str) -> Option<Sides> {
    [Sides::OneSided, Sides::TwoSidedLongEdge, Sides::TwoSidedShortEdge].into_iter().find(|s| s.keyword() == v)
}

impl PresetStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.name == name.trim())
    }

    /// Adds a preset. A name that is bad or already taken is refused and the store is unchanged.
    pub fn add(&mut self, preset: Preset) -> Result<(), PresetError> {
        if !valid_name(&preset.name) {
            return Err(PresetError::BadName(preset.name));
        }
        if self.get(&preset.name).is_some() {
            return Err(PresetError::Duplicate(preset.name));
        }
        self.presets.push(preset);
        Ok(())
    }

    /// Removes and returns the preset with this name.
    pub fn remove(&mut self, name: &str) -> Result<Preset, PresetError> {
        let at = self.presets.iter().position(|p| p.name == name.trim()).ok_or_else(|| PresetError::Unknown(name.to_string()))?;
        Ok(self.presets.remove(at))
    }

    /// The text form (see the module docs). Sections are separated by a blank line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, preset) in self.presets.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            preset.write(&mut out);
        }
        out
    }

    /// Reads the text form. See the module docs for what is lenient.
    pub fn from_text(text: &str) -> PresetStore {
        let mut store = PresetStore::new();
        let mut current: Option<Preset> = None;
        let mut skipping = false;
        let finish = |store: &mut PresetStore, current: &mut Option<Preset>| {
            if let Some(preset) = current.take() {
                let _ = store.add(preset);
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                finish(&mut store, &mut current);
                skipping = !valid_name(inner) || store.get(inner).is_some();
                if !skipping {
                    let base = PrintInfo::default();
                    current = Preset::from_info(inner, &base).ok();
                }
                continue;
            }
            let (Some(preset), Some((key, value))) = (current.as_mut(), line.split_once('=')) else { continue };
            let value = value.trim();
            match key.trim() {
                "paper" => {
                    if let Some(p) = PaperSize::from_media(value) {
                        preset.paper = p;
                    }
                }
                "orientation" => match value {
                    "landscape" => preset.orientation = Orientation::Landscape,
                    "portrait" => preset.orientation = Orientation::Portrait,
                    _ => {}
                },
                "margins" => {
                    let parsed: Vec<f32> = value.split_whitespace().filter_map(|v| v.parse::<f32>().ok()).collect();
                    if let &[t, l, b, r] = parsed.as_slice() {
                        if [t, l, b, r].iter().all(|m| *m >= 0.0 && m.is_finite()) {
                            preset.margins = [t, l, b, r];
                        }
                    }
                }
                "copies" => {
                    if let Ok(n) = value.parse::<u32>() {
                        preset.copies = n.clamp(1, MAX_COPIES);
                    }
                }
                "scaling" => {
                    if let Ok(f) = value.parse::<f32>() {
                        if !f.is_nan() {
                            preset.scaling = f.clamp(SCALING_RANGE.0, SCALING_RANGE.1);
                        }
                    }
                }
                "sides" => {
                    if let Some(s) = parse_sides(value) {
                        preset.sides = s;
                    }
                }
                "color" => match value {
                    "color" => preset.color = ColorMode::Color,
                    "monochrome" => preset.color = ColorMode::Monochrome,
                    _ => {}
                },
                _ => {}
            }
        }
        finish(&mut store, &mut current);
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duplex() -> Preset {
        let info = PrintInfo {
            paper: PaperSize::Letter,
            orientation: Orientation::Landscape,
            margins: [36.0, 18.5, 36.0, 18.5],
            copies: 3,
            scaling_factor: 0.75,
            sides: Sides::TwoSidedLongEdge,
            color: ColorMode::Monochrome,
            pages: Some((2, 4)),
            ..PrintInfo::default()
        };
        Preset::from_info("Duplex draft", &info).unwrap()
    }

    #[test]
    fn a_preset_round_trips_through_its_text_form() {
        let mut store = PresetStore::new();
        store.add(duplex()).unwrap();
        store.add(Preset::from_info("Plain", &PrintInfo::default()).unwrap()).unwrap();
        let text = store.to_text();
        assert!(text.contains("[Duplex draft]\npaper=Letter\norientation=landscape\nmargins=36 18.5 36 18.5\n"));
        assert!(text.contains("sides=two-sided-long-edge\ncolor=monochrome\n"));
        assert_eq!(PresetStore::from_text(&text), store);
    }

    #[test]
    fn a_preset_keeps_the_setup_but_not_the_page_range_or_the_path() {
        let mut info = PrintInfo { pages: Some((5, 6)), saving_path: Some("/tmp/x.pdf".into()), ..PrintInfo::default() };
        duplex().apply_to(&mut info);
        assert_eq!((info.paper, info.orientation, info.copies), (PaperSize::Letter, Orientation::Landscape, 3));
        assert_eq!((info.scaling_factor, info.sides, info.color), (0.75, Sides::TwoSidedLongEdge, ColorMode::Monochrome));
        assert_eq!(info.pages, Some((5, 6)), "the range belongs to the job");
        assert!(info.saving_path.is_some());
    }

    #[test]
    fn names_must_be_unique_and_plain() {
        let mut store = PresetStore::new();
        store.add(duplex()).unwrap();
        assert_eq!(store.add(duplex()), Err(PresetError::Duplicate("Duplex draft".into())));
        assert_eq!(store.presets().len(), 1, "a refused preset changes nothing");
        assert!(Preset::from_info("  ", &PrintInfo::default()).is_err());
        assert!(Preset::from_info("a]b", &PrintInfo::default()).is_err());
        assert!(!valid_name(&"x".repeat(65)));
        assert_eq!(store.remove("Nope"), Err(PresetError::Unknown("Nope".into())));
        assert_eq!(store.remove(" Duplex draft ").unwrap().copies, 3);
        assert!(store.presets().is_empty());
    }

    #[test]
    fn reading_is_lenient_about_unknown_keys_bad_values_and_repeated_sections() {
        let text = "# saved by an older build\n\
                    [Office]\n\
                    paper=Tabloid\n\
                    margins=-1 0 0 0\n\
                    copies=100000\n\
                    scaling=nan\n\
                    colour-profile=srgb\n\
                    sides=one-sided\n\
                    \n\
                    [Office]\n\
                    copies=9\n\
                    [Second]\n\
                    orientation=landscape\n";
        let store = PresetStore::from_text(text);
        let names: Vec<&str> = store.presets().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Office", "Second"], "the repeated section is skipped");
        let office = store.get("Office").unwrap();
        assert_eq!(office.paper, PaperSize::A4, "a paper name that is not known keeps the default");
        assert_eq!(office.margins, [72.0; 4], "negative margins keep the default");
        assert_eq!((office.copies, office.scaling), (9999, 1.0));
        assert_eq!(store.get("Second").unwrap().orientation, Orientation::Landscape);
    }
}
