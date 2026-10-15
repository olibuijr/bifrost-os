//! The window's settings, changed from the Settings menu: the author written into new markup, whether a document opens
//! at the page it was last shown on, and whether a document opens as the contact sheet. They are kept as `key=value`
//! lines under the app's data directory. A missing or damaged file gives the defaults.

use std::path::{Path, PathBuf};

/// The settings the Settings menu changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// The name written as the author of new markup. Empty for none.
    pub author: String,
    /// Whether a document opens at the page it was last shown on.
    pub remember_page: bool,
    /// Whether a document opens as the contact sheet instead of one page.
    pub contact_sheet: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { author: String::new(), remember_page: true, contact_sheet: false }
    }
}

impl Prefs {
    /// Reads the settings from their text. Unknown keys and values that are not understood keep the defaults.
    pub fn parse(text: &str) -> Prefs {
        let mut prefs = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "author" => prefs.author = sanitize(value),
                "remember_page" => prefs.remember_page = flag(value).unwrap_or(prefs.remember_page),
                "contact_sheet" => prefs.contact_sheet = flag(value).unwrap_or(prefs.contact_sheet),
                _ => {}
            }
        }
        prefs
    }

    /// The settings as text, one `key=value` line each.
    pub fn to_text(&self) -> String {
        format!("author={}\nremember_page={}\ncontact_sheet={}\n", sanitize(&self.author), self.remember_page, self.contact_sheet)
    }

    /// The author name to write into markup, or None when there is none.
    pub fn author_name(&self) -> Option<&str> {
        let name = self.author.trim();
        (!name.is_empty()).then_some(name)
    }
}

/// A true or false value written as `true` or `false`; anything else is None.
fn flag(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The text with control characters (tabs and line breaks) turned into spaces and the ends trimmed, so it fits on one
/// line of the file.
pub fn sanitize(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// Where the settings are kept.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("prefs.txt")
}

/// The saved settings, or the defaults when there are none.
pub fn load(data_dir: &Path) -> Prefs {
    std::fs::read_to_string(path(data_dir)).map(|t| Prefs::parse(&t)).unwrap_or_default()
}

/// Saves the settings under `data_dir`.
pub fn save(data_dir: &Path, prefs: &Prefs) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(path(data_dir), prefs.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_remember_the_page_and_open_one_page() {
        let p = Prefs::default();
        assert!(p.remember_page && !p.contact_sheet);
        assert_eq!(p.author_name(), None);
    }

    #[test]
    fn settings_round_trip_through_their_text() {
        let p = Prefs { author: "Ana Lind".into(), remember_page: false, contact_sheet: true };
        assert_eq!(Prefs::parse(&p.to_text()), p);
        assert_eq!(p.author_name(), Some("Ana Lind"));
    }

    #[test]
    fn damaged_lines_keep_the_defaults_and_unknown_keys_are_ignored() {
        let p = Prefs::parse("garbage\nremember_page=maybe\ncontact_sheet=true\nwindow_mode=dark\n");
        assert!(p.remember_page, "a value that is not true or false keeps the default");
        assert!(p.contact_sheet);
        assert_eq!(p.author, "");
    }

    #[test]
    fn an_author_name_stays_on_one_line_and_a_blank_one_is_none() {
        // a tab inside the name becomes a space, and the ends are trimmed
        let p = Prefs::parse("author=  Ana\tLind  \n");
        assert_eq!(p.author, "Ana Lind");
        let blank = Prefs { author: "   ".into(), ..Prefs::default() };
        assert_eq!(blank.author_name(), None);
    }

    #[test]
    fn settings_are_saved_under_the_data_folder_and_loaded_back() {
        let dir = std::env::temp_dir().join(format!("viewer-prefs-{}", std::process::id()));
        assert_eq!(load(&dir), Prefs::default(), "nothing saved yet");
        let p = Prefs { author: "Ana".into(), remember_page: true, contact_sheet: true };
        save(&dir, &p).expect("saved");
        assert_eq!(load(&dir), p);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
