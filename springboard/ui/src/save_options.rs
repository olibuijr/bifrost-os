//! Save options for an edited picture: the file format, the JPEG quality, the suffix added to the file name and
//! whether an existing file may be replaced. The image editor's save sheet reads and writes this model, and apps that
//! save pictures use the same file names and quality range.

/// The formats an edited picture can be saved as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
}

impl ImageFormat {
    /// The file name extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Tiff => "tiff",
        }
    }

    /// The name shown in the save sheet.
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG",
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::Tiff => "TIFF",
        }
    }

    /// True when the format is lossy and takes a quality setting.
    pub fn takes_quality(self) -> bool {
        matches!(self, ImageFormat::Jpeg)
    }

    /// Reads a format from a file extension, ignoring case and a leading dot. Returns `None` for other types.
    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            "tif" | "tiff" => Some(ImageFormat::Tiff),
            _ => None,
        }
    }
}

/// What the save sheet chose.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveOptions {
    pub format: ImageFormat,
    /// 0.0 (smallest file) to 1.0 (best quality); only lossy formats use it
    pub quality: f32,
    /// added to the picture's name before the extension, such as " edited"
    pub suffix: String,
    /// true when a file with the same name may be overwritten; false makes the name unique
    pub replace_existing: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        SaveOptions { format: ImageFormat::Png, quality: 0.9, suffix: " edited".into(), replace_existing: false }
    }
}

impl SaveOptions {
    /// Sets the quality, clamped to 0.0 to 1.0. A value that is not a number keeps the current quality.
    pub fn with_quality(mut self, quality: f32) -> SaveOptions {
        if quality.is_finite() {
            self.quality = quality.clamp(0.0, 1.0);
        }
        self
    }

    /// The encoder quality for a lossy format: 1 to 100, or `None` for a format without quality.
    pub fn encoder_quality(&self) -> Option<u8> {
        self.format.takes_quality().then(|| (self.quality * 100.0).round().clamp(1.0, 100.0) as u8)
    }

    /// The file name for a picture named `stem`: the stem, the suffix and the extension. Path separators are removed
    /// from both parts, so the name always stays in one folder.
    pub fn file_name(&self, stem: &str) -> String {
        self.name(stem, None)
    }

    /// The file name to write, given the names already in the folder. With `replace_existing` the plain name is
    /// used; otherwise a number is added before the extension (" 2", " 3", ...) until the name is free.
    pub fn unique_file_name(&self, stem: &str, taken: &[&str]) -> String {
        let first = self.name(stem, None);
        if self.replace_existing || !taken.contains(&first.as_str()) {
            return first;
        }
        let mut n = 2u32;
        loop {
            let candidate = self.name(stem, Some(n));
            if !taken.contains(&candidate.as_str()) {
                return candidate;
            }
            n += 1;
        }
    }

    fn name(&self, stem: &str, number: Option<u32>) -> String {
        let base = clean(stem);
        let base = base.trim();
        let base = if base.is_empty() { "Untitled" } else { base };
        let number = number.map(|n| format!(" {n}")).unwrap_or_default();
        format!("{base}{}{number}.{}", clean(&self.suffix), self.format.extension())
    }
}

fn clean(part: &str) -> String {
    part.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_save_writes_a_png_with_the_edited_suffix() {
        let options = SaveOptions::default();
        assert_eq!(options.format, ImageFormat::Png);
        assert_eq!(options.file_name("Beach"), "Beach edited.png");
        assert_eq!(options.encoder_quality(), None);
    }

    #[test]
    fn the_extensions_and_labels_of_each_format() {
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
        assert_eq!(ImageFormat::Tiff.extension(), "tiff");
        assert_eq!(ImageFormat::Jpeg.label(), "JPEG");
        assert!(ImageFormat::Jpeg.takes_quality());
        assert!(!ImageFormat::Tiff.takes_quality());
    }

    #[test]
    fn a_format_is_read_from_an_extension_in_any_case() {
        assert_eq!(ImageFormat::from_extension(".JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("tif"), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_extension("gif"), None);
    }

    #[test]
    fn quality_is_clamped_and_a_nan_keeps_the_old_value() {
        let high = SaveOptions::default().with_quality(3.0);
        assert_eq!(high.quality, 1.0);
        let low = SaveOptions::default().with_quality(-1.0);
        assert_eq!(low.quality, 0.0);
        let kept = SaveOptions::default().with_quality(0.5).with_quality(f32::NAN);
        assert_eq!(kept.quality, 0.5);
    }

    #[test]
    fn a_jpeg_quality_becomes_an_encoder_value_from_one_to_one_hundred() {
        let jpeg = SaveOptions { format: ImageFormat::Jpeg, ..SaveOptions::default() }.with_quality(0.0);
        assert_eq!(jpeg.encoder_quality(), Some(1));
        let full = SaveOptions { format: ImageFormat::Jpeg, ..SaveOptions::default() }.with_quality(1.0);
        assert_eq!(full.encoder_quality(), Some(100));
        let mid = SaveOptions { format: ImageFormat::Jpeg, ..SaveOptions::default() }.with_quality(0.5);
        assert_eq!(mid.encoder_quality(), Some(50));
    }

    #[test]
    fn path_separators_never_reach_the_file_name() {
        let options = SaveOptions { suffix: "/ x".into(), ..SaveOptions::default() };
        assert_eq!(options.file_name("../Holiday"), "..Holiday x.png");
        assert_eq!(SaveOptions::default().file_name("   "), "Untitled edited.png");
    }

    #[test]
    fn a_taken_name_gets_a_number_unless_replacing_is_allowed() {
        let taken = ["Beach edited.png", "Beach edited 2.png"];
        let options = SaveOptions::default();
        assert_eq!(options.unique_file_name("Beach", &taken), "Beach edited 3.png");
        assert_eq!(options.unique_file_name("Hills", &taken), "Hills edited.png");
        let replace = SaveOptions { replace_existing: true, ..SaveOptions::default() };
        assert_eq!(replace.unique_file_name("Beach", &taken), "Beach edited.png");
    }
}
