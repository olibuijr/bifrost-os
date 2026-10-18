//! The `preview.render` generator Notes declares in its manifest (`[[preview]]`, type `net.daringfireball.markdown`): the
//! shell asks for a picture of a Markdown note (a Notes export) and Notes answers with a PNG of its first lines, set in
//! the app's own face. The shell draws the picture in the preview panel; see `springboard::shell::preview_gen`.
//!
//! Request: `{"file": "<absolute path>", "width": w, "height": h}`. Reply: `{"picture": "<absolute path to a PNG>"}`,
//! written under the app's data directory. An unreadable or non-absolute file, or a size out of range, is an error, and
//! the shell falls back to its built-in preview.

use bifrost_app::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS};
use bifrost_core::json::Json;
use springboard::gfx::{Image, Surface};
use springboard::shell::md;
use springboard::shell::ui::{Ui, Weight};
use std::io::Read;
use std::path::Path;

/// The control method the shell calls.
pub const METHOD: &str = "preview.render";
/// The largest file read for a preview; longer notes show their first part.
const MAX_BYTES: u64 = 64 * 1024;
/// The largest picture edge a request may ask for, in pixels.
const MAX_EDGE: u64 = 4096;
/// The text drawn is the first characters of the note; the rest is not laid out.
const MAX_CHARS: usize = 4000;
/// The note's text size in points, and the margin around it in points.
const TEXT_PT: f32 = 14.0;
const MARGIN_PT: f32 = 12.0;
const LINE_GAP_PT: f32 = 4.0;
/// Paper and ink, as ARGB.
const PAPER: u32 = 0xFF_FF_FF_FF;
const INK: u32 = 0xFF_1D_1D_1F;

/// The file, width and height of a request. The file must be an absolute path and the size 1 to [`MAX_EDGE`].
pub fn request(params: &Json) -> Result<(String, usize, usize), String> {
    let file = params.get("file").and_then(Json::as_str).ok_or("preview.render needs a file")?;
    if !file.starts_with('/') || file.chars().any(|c| c.is_control()) {
        return Err("the file must be an absolute path".into());
    }
    let edge = |key: &str| -> Result<usize, String> {
        match params.get(key).and_then(Json::as_u64) {
            Some(v) if (1..=MAX_EDGE).contains(&v) => Ok(v as usize),
            _ => Err(format!("{key} must be between 1 and {MAX_EDGE}")),
        }
    };
    Ok((file.to_string(), edge("width")?, edge("height")?))
}

/// The note's plain text, its first [`MAX_CHARS`] characters, from the first [`MAX_BYTES`] of the file.
fn note_text(file: &Path) -> std::io::Result<String> {
    let mut buf = Vec::new();
    std::fs::File::open(file)?.take(MAX_BYTES).read_to_end(&mut buf)?;
    let markdown = String::from_utf8_lossy(&buf);
    Ok(md::to_plain(&markdown).chars().take(MAX_CHARS).collect())
}

/// A `w` by `h` PNG of the note's text, wrapped to the width and cut at the bottom, in the app's face.
pub fn render(ui: &Ui, text: &str, w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![PAPER; w * h];
    let margin = ui.pt(MARGIN_PT) as i32;
    let max_px = (w as i32 - 2 * margin).max(1) as f32;
    let line_gap = ui.pt(LINE_GAP_PT) as i32;
    {
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let mut y = margin;
        for line in ui.wrap(text, TEXT_PT, Weight::Regular, max_px) {
            if line.is_empty() {
                y += ui.pt(TEXT_PT) as i32;
                continue;
            }
            let im: Image = ui.text(&line, TEXT_PT, Weight::Regular, INK);
            if y + im.h as i32 > h as i32 - margin {
                break;
            }
            s.blit(&im, margin, y, 255, 255);
            y += im.h as i32 + line_gap;
        }
    }
    springboard::pngenc::encode_rgb(w, h, &px)
}

/// A content key for the picture's file name: the text and the size, so an edited note gets a new picture.
fn key(text: &str, w: usize, h: usize) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes().chain(w.to_le_bytes()).chain(h.to_le_bytes()) {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Answers one `preview.render` call. `ui` is the app's own UI (its face); without it, nothing is drawn.
pub fn handle(ui: Option<&Ui>, data_dir: &Path, params: &Json) -> Result<Json, RpcError> {
    let (file, w, h) = request(params).map_err(|e| RpcError::new(INVALID_PARAMS, e))?;
    let Some(ui) = ui else {
        return Err(RpcError::new(INTERNAL_ERROR, "Notes is not ready to draw previews yet"));
    };
    let text = note_text(Path::new(&file)).map_err(|e| RpcError::new(INVALID_PARAMS, format!("{file}: {e}")))?;
    let png = render(ui, &text, w, h);
    let dir = data_dir.join("previews");
    std::fs::create_dir_all(&dir).map_err(|e| RpcError::new(INTERNAL_ERROR, e.to_string()))?;
    let path = dir.join(format!("{}.png", key(&text, w, h)));
    if !path.is_file() {
        std::fs::write(&path, &png).map_err(|e| RpcError::new(INTERNAL_ERROR, e.to_string()))?;
    }
    Ok(Json::Obj(vec![("picture".into(), Json::Str(path.to_string_lossy().into_owned()))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(file: &str, w: u64, h: u64) -> Json {
        Json::Obj(vec![("file".into(), Json::Str(file.into())), ("width".into(), Json::Num(w as f64)), ("height".into(), Json::Num(h as f64))])
    }

    #[test]
    fn a_request_needs_an_absolute_file_and_a_size_in_range() {
        assert_eq!(request(&params("/home/u/a.md", 512, 384)), Ok(("/home/u/a.md".into(), 512, 384)));
        assert!(request(&params("relative.md", 512, 384)).is_err(), "relative paths are refused");
        assert!(request(&params("/x/a\nb.md", 512, 384)).is_err(), "control characters are refused");
        assert!(request(&params("/x/a.md", 0, 384)).is_err(), "no zero-size picture");
        assert!(request(&params("/x/a.md", 512, MAX_EDGE + 1)).is_err(), "no picture past the edge limit");
        assert!(request(&Json::Obj(vec![])).is_err(), "a file is required");
    }

    #[test]
    fn a_note_is_drawn_as_a_png_of_its_size() {
        let ui = crate::test_ui();
        let png = render(&ui, "Hello\nworld", 320, 200);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the picture is a PNG");
        assert!(png.len() > 100);
        assert_ne!(render(&ui, "Hello", 320, 200), render(&ui, "", 320, 200), "text makes marks on the paper");
    }

    #[test]
    fn the_key_changes_with_the_text_and_the_size() {
        assert_eq!(key("a", 1, 2), key("a", 1, 2));
        assert_ne!(key("a", 1, 2), key("b", 1, 2));
        assert_ne!(key("a", 1, 2), key("a", 2, 1));
    }

    #[test]
    fn a_missing_note_is_an_error_and_a_note_gets_a_picture_in_the_data_dir() {
        let ui = crate::test_ui();
        let dir = std::env::temp_dir().join(format!("notes-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = handle(Some(&ui), &dir, &params("/nonexistent/note.md", 320, 200));
        assert!(missing.is_err(), "a note that is not there has no picture");
        assert!(handle(None, &dir, &params("/nonexistent/note.md", 320, 200)).is_err(), "no UI, no picture");
        let note = dir.join("note.md");
        std::fs::write(&note, "# Title\n\nSome **body** text.\n").unwrap();
        let answer = handle(Some(&ui), &dir, &params(note.to_str().unwrap(), 320, 200)).unwrap();
        let picture = answer.get("picture").and_then(Json::as_str).unwrap().to_string();
        assert!(picture.starts_with(dir.join("previews").to_str().unwrap()), "the picture goes under the data directory");
        assert!(Path::new(&picture).is_file());
        let again = handle(Some(&ui), &dir, &params(note.to_str().unwrap(), 320, 200)).unwrap();
        assert_eq!(again, answer, "the same note and size reuse the same picture");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
