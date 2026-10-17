//! The face EPUB body text is set in. The Display panel's font row picks one family of BifrOSt's font library; the reader
//! measures and draws the body with that face, so the pagination holds the lines the face really fits. Chrome (the
//! bars, panels and buttons) stays in the system face.

use bifrost_app::font_descriptor::UserFonts;
use bifrost_app::kit::{Font, Ui};
use std::path::PathBuf;
use std::rc::Rc;

/// The shell's data folder: `BIFROST_SHELL_DIR`, else the shell's default root.
pub fn shell_dir() -> String {
    std::env::var("BIFROST_SHELL_DIR").unwrap_or_else(|_| bifrost_app::service_client::DEFAULT_ROOT.to_string())
}

/// The families of BifrOSt's font library, one per family and sorted by name. Empty when the library has no fonts.
pub fn families(shell_dir: &str) -> Vec<String> {
    UserFonts::choices(shell_dir).into_iter().map(|f| f.family).collect()
}

/// The font file that stands for `family` in the library.
pub fn path_of(shell_dir: &str, family: &str) -> Option<PathBuf> {
    UserFonts::choices(shell_dir).into_iter().find(|f| f.family == family).map(|f| PathBuf::from(f.path))
}

/// The bytes of the font file at `path`, read once and kept for the life of the process.
fn font_bytes(path: &PathBuf) -> Option<&'static [u8]> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static LOADED: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let mut loaded = LOADED.get_or_init(Default::default).lock().ok()?;
    if let Some(data) = loaded.get(path) {
        return Some(data);
    }
    let data: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
    loaded.insert(path.clone(), data);
    Some(data)
}

/// A UI that draws with the font file at `path`, at the point size, text scale and bold setting of `base`. None when
/// the file cannot be read or is not a font the renderer can draw. The font bytes are kept for the life of the
/// process, once per font file: later calls for the same path reuse them, so stepping through families and sizes
/// does not grow memory.
pub fn ui_for(base: &Ui, path: &PathBuf) -> Option<Ui> {
    let data = font_bytes(path)?;
    let font: Font<'static> = Font::parse(data)?;
    let mut ui = Ui::new(font, base.s);
    ui.text_scale = base.text_scale;
    ui.bold = base.bold;
    Some(ui)
}

/// The key a built face is cached under: the family and everything else that changes how the text measures.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceKey {
    pub family: String,
    pub s: f32,
    pub text_scale: f32,
    pub bold: bool,
}

impl FaceKey {
    pub fn of(family: &str, ui: &Ui) -> FaceKey {
        FaceKey { family: family.to_string(), s: ui.s, text_scale: ui.text_scale, bold: ui.bold }
    }
}

/// A built face for the reader's body text, shared between the layout and the drawing.
pub type Face = Rc<Ui>;
