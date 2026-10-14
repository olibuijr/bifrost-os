//! Saving the files of a message. Each message's files are written into their own folder under the app's data
//! directory, so a second message with the same file name never overwrites the first.

use crate::body::Attachment;
use std::collections::HashSet;
use std::path::Path;

/// A file name that is safe to create in a folder: path separators and control characters become `_`, and leading
/// dots are dropped so a name never climbs out of the folder or hides itself.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Writes every file into `dir`, creating the folder. A name already used in this message gets its position in front
/// (`2-name`). Returns how many files were written.
pub fn save_all(dir: &Path, files: &[Attachment]) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut used = HashSet::new();
    for (index, file) in files.iter().enumerate() {
        let mut name = safe_name(&file.name);
        if !used.insert(name.clone()) {
            name = format!("{}-{name}", index + 1);
            used.insert(name.clone());
        }
        std::fs::write(dir.join(&name), &file.bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(files.len())
}

/// The media type for a file name, from its extension; `application/octet-stream` when the extension is not known.
pub fn mime_for(name: &str) -> String {
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mime = match extension.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "ics" => "text/calendar",
        "vcf" => "text/vcard",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    };
    mime.to_string()
}

/// A size for the reader: bytes, kilobytes or megabytes.
pub fn size_text(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_inside_the_folder() {
        assert_eq!(safe_name("skjal.pdf"), "skjal.pdf");
        assert_eq!(safe_name("../x"), "_x");
        assert_eq!(safe_name("a/b\\c"), "a_b_c");
        assert_eq!(safe_name("  "), "attachment");
    }

    #[test]
    fn the_media_type_follows_the_extension() {
        assert_eq!(mime_for("skjal.PDF"), "application/pdf");
        assert_eq!(mime_for("mynd.jpeg"), "image/jpeg");
        assert_eq!(mime_for("notes.txt"), "text/plain");
        assert_eq!(mime_for("old.eml"), "message/rfc822");
        assert_eq!(mime_for("noextension"), "application/octet-stream");
        assert_eq!(mime_for("weird.xyz"), "application/octet-stream");
    }

    #[test]
    fn sizes_read_in_bytes_kilobytes_and_megabytes() {
        assert_eq!(size_text(12), "12 B");
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn a_repeated_name_is_written_under_its_position() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-att-{}", std::process::id()));
        let files = vec![
            Attachment { name: "a.txt".into(), mime: "text/plain".into(), bytes: b"one".to_vec() },
            Attachment { name: "a.txt".into(), mime: "text/plain".into(), bytes: b"two".to_vec() },
        ];
        assert_eq!(save_all(&dir, &files), Ok(2));
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"one".to_vec());
        assert_eq!(std::fs::read(dir.join("2-a.txt")).unwrap(), b"two".to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
