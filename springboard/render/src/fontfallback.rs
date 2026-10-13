//! The process-wide fallback faces: when the UI font has no glyph for a character, `Font::render_var` and
//! `Font::measure_var` draw that run with the first of these faces that has one, so a character any installed face
//! covers never renders as `.notdef`. The shell installs the list once at start (`install_from_dir`); until then
//! the list is empty and text draws exactly as before.

use crate::font::Font;
use crate::fontobj::FontManager;
use std::sync::{Arc, RwLock};

type Faces = Arc<Vec<Font<'static>>>;

static FACES: RwLock<Option<Faces>> = RwLock::new(None);

/// The installed fallback faces, in order. Empty before `install`.
pub fn faces() -> Faces {
    FACES.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}

/// Replaces the fallback faces.
pub fn install(faces: Vec<Font<'static>>) {
    if let Ok(mut g) = FACES.write() {
        *g = Some(Arc::new(faces));
    }
}

/// Installs every face of `mgr`, in the manager's fallback order for `lang`, as the process fallback list.
/// Returns how many faces went in.
pub fn install_from_manager(mgr: &FontManager, lang: &str) -> usize {
    let faces: Vec<Font<'static>> = mgr.fallback_order(&[], lang, None).into_iter().filter_map(|i| Font::parse(mgr.static_bytes(i)?)).collect();
    let n = faces.len();
    install(faces);
    n
}

/// Registers the fonts under `dir` (the font library), then installs them as the fallback list. Returns the count.
pub fn install_from_dir(dir: &std::path::Path, lang: &str) -> usize {
    let mut m = FontManager::new();
    m.scan_dir(dir, 2);
    install_from_manager(&m, lang)
}

/// Splits `text` into runs that share a face. `pick` says, per character, which fallback face draws it (`None`
/// keeps the character on the main face). Neighbours with the same pick join one run.
pub fn split(text: &str, mut pick: impl FnMut(char) -> Option<usize>) -> Vec<(Option<usize>, &str)> {
    let mut runs: Vec<(Option<usize>, usize, usize)> = Vec::new();
    for (i, ch) in text.char_indices() {
        let f = pick(ch);
        let end = i + ch.len_utf8();
        match runs.last_mut() {
            Some(r) if r.0 == f => r.2 = end,
            _ => runs.push((f, i, end)),
        }
    }
    runs.into_iter().map(|(f, s, e)| (f, &text[s..e])).collect()
}

/// A run of text and the index (into the returned faces) of the face that draws it; `None` is the main face.
pub type Runs<'t> = Vec<(Option<usize>, &'t str)>;

/// How `text` splits between `primary` and the fallback faces, or None when `primary` draws all of it (the
/// common case, found without allocating).
pub fn runs<'t>(primary: &Font<'_>, text: &'t str) -> Option<(Faces, Runs<'t>)> {
    let faces = faces();
    if faces.is_empty() {
        return None;
    }
    let skip = |c: char| c.is_ascii() || c.is_whitespace() || c.is_control();
    if text.chars().all(|c| skip(c) || primary.glyph(c) != 0) {
        return None;
    }
    let r = split(text, |c| {
        if skip(c) || primary.glyph(c) != 0 {
            return None;
        }
        faces.iter().position(|f| f.glyph(c) != 0)
    });
    if r.iter().all(|x| x.0.is_none()) {
        return None;
    }
    Some((faces, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf() -> &'static [u8] {
        Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice())
    }

    #[test]
    fn split_merges_neighbours_and_covers_the_text() {
        let picks = |c: char| match c {
            'b' | 'c' => Some(0),
            'x' => Some(1),
            _ => None,
        };
        let r = split("abcxxa", picks);
        assert_eq!(r, vec![(None, "a"), (Some(0), "bc"), (Some(1), "xx"), (None, "a")]);
        assert!(split("", picks).is_empty());
        assert_eq!(r.iter().map(|x| x.1).collect::<String>(), "abcxxa");
    }

    #[test]
    fn no_fallback_faces_or_full_coverage_means_no_runs() {
        let data = sf();
        let f = Font::parse(data).unwrap();
        // the list may be shared with other tests in this process; whatever it holds, covered text yields None
        assert!(runs(&f, "Hello world").is_none());
        assert!(runs(&f, "").is_none());
    }

    #[test]
    fn a_character_no_face_covers_stays_on_the_main_face() {
        let data = sf();
        let f = Font::parse(data).unwrap();
        install(vec![Font::parse(data).unwrap()]);
        assert!(runs(&f, "a\u{10FFFE}b").is_none());
        install(Vec::new());
    }
}
