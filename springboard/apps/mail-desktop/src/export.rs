//! Saving a message as it is (File > Save As): the raw source written as an `.eml` file with CRLF line ends.

use crate::attachments::safe_name;
use std::path::{Path, PathBuf};

/// The file name for a message: its subject (or `Message`) made safe for a folder, ending in `.eml`.
pub fn eml_name(subject: &str) -> String {
    let base = if subject.trim().is_empty() { "Message".to_string() } else { safe_name(subject) };
    let base: String = base.chars().take(80).collect();
    format!("{base}.eml")
}

/// The file name for a message exported as PDF: the same name as [`eml_name`], ending in `.pdf`.
pub fn pdf_name(subject: &str) -> String {
    let eml = eml_name(subject);
    format!("{}.pdf", eml.trim_end_matches(".eml"))
}

/// The raw text with every line ended by CRLF, as mail files are kept.
pub fn crlf(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

/// Writes `raw` into `dir` under [`eml_name`] and returns the path. A name already there gets `-2`, `-3`, ... so a saved
/// message is never overwritten.
pub fn save_eml(dir: &Path, subject: &str, raw: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = eml_name(subject);
    let stem = name.trim_end_matches(".eml");
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.eml"));
        n += 1;
    }
    std::fs::write(&path, crlf(raw)).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_subject() {
        assert_eq!(eml_name("Fundur á morgun"), "Fundur á morgun.eml");
        assert_eq!(eml_name("../etc/passwd"), "_etc_passwd.eml");
        assert_eq!(eml_name("  "), "Message.eml");
        assert_eq!(pdf_name("Fundur á morgun"), "Fundur á morgun.pdf");
        assert_eq!(pdf_name("  "), "Message.pdf");
    }

    #[test]
    fn line_ends_become_crlf_once() {
        assert_eq!(crlf("a\nb\r\nc\rd"), "a\r\nb\r\nc\r\nd");
    }

    #[test]
    fn a_second_save_keeps_the_first() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-eml-{}", std::process::id()));
        let one = save_eml(&dir, "Plan", "Subject: Plan\n\none").unwrap();
        let two = save_eml(&dir, "Plan", "Subject: Plan\n\ntwo").unwrap();
        assert_eq!(one.file_name().unwrap(), "Plan.eml");
        assert_eq!(two.file_name().unwrap(), "Plan-2.eml");
        assert_eq!(std::fs::read_to_string(&one).unwrap(), "Subject: Plan\r\n\r\none");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
