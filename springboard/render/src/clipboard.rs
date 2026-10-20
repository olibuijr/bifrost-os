//! The shell-wide clipboard: one text or one image, shared by every app.
//! Text survives a restart in `/data/local/springboard/clipboard.txt`
//! (owner-only, written through a temporary file and a rename); an image
//! lives in memory only. `global()` is the instance the apps share.

use crate::gfx::Image;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The clipboard file inside the data directory.
pub const FILE: &str = "clipboard.txt";

#[derive(Debug, Default)]
pub struct Clipboard {
    text: Option<String>,
    image: Option<Image>,
    path: Option<PathBuf>,
    /// Counts the changes made through `set_text`, `set_image` and `attach`, so a reader can tell cheaply that the
    /// contents are new without comparing them.
    rev: u64,
}

impl Clipboard {
    /// A clipboard that never touches the disk.
    pub fn memory() -> Self {
        Clipboard::default()
    }

    /// Loads the text saved at `path` (none if missing) and saves there from now on.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let text = std::fs::read_to_string(&path).ok().filter(|s| !s.is_empty());
        Clipboard { text, image: None, path: Some(path), rev: 0 }
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn has_text(&self) -> bool {
        self.text.is_some()
    }

    pub fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    /// Replaces the contents with `text` (empty clears the clipboard) and persists it.
    pub fn set_text(&mut self, text: &str) {
        self.image = None;
        self.rev = self.rev.wrapping_add(1);
        self.text = (!text.is_empty()).then(|| text.to_string());
        if let Err(e) = self.save() {
            eprintln!("springboard: clipboard: {e}");
        }
    }

    /// Replaces the contents with an image; the persisted text is removed.
    pub fn set_image(&mut self, image: Image) {
        self.image = Some(image);
        self.rev = self.rev.wrapping_add(1);
        self.text = None;
        if let Err(e) = self.save() {
            eprintln!("springboard: clipboard: {e}");
        }
    }

    pub fn clear(&mut self) {
        self.set_text("");
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let Some(text) = &self.text else {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = tmp_path(path);
        write_private(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, path)
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Writes `bytes` to a new file readable by its owner only.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(any(test, feature = "testkit")))]
static GLOBAL: Mutex<Option<Clipboard>> = Mutex::new(None);

/// Tests run on their own threads; one clipboard per thread keeps them from racing on shared state.
#[cfg(any(test, feature = "testkit"))]
fn slot() -> &'static Mutex<Option<Clipboard>> {
    thread_local! {
        static SLOT: &'static Mutex<Option<Clipboard>> = Box::leak(Box::new(Mutex::new(None)));
    }
    SLOT.with(|s| *s)
}

#[cfg(not(any(test, feature = "testkit")))]
fn slot() -> &'static Mutex<Option<Clipboard>> {
    &GLOBAL
}

/// The clipboard every app shares; in memory until `attach` names its file.
pub fn global() -> MutexGuard<'static, Option<Clipboard>> {
    let mut g = slot().lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Clipboard::memory());
    }
    g
}

/// Points the shared clipboard at `path`, taking over the text saved there.
pub fn attach(path: impl Into<PathBuf>) {
    let mut g = global();
    let loaded = Clipboard::load(path);
    let image = g.as_mut().and_then(|c| c.image.take());
    // the revision only ever moves on, so a reader that saw the old contents sees the loaded ones as a change
    let rev = g.as_ref().map_or(0, |c| c.rev).wrapping_add(1);
    *g = Some(Clipboard { image: image.filter(|_| loaded.text.is_none()), rev, ..loaded });
}

/// The shared clipboard's revision: it changes whenever the text or image is replaced (`set_text`, `set_image`, `attach`).
pub fn revision() -> u64 {
    global().as_ref().map_or(0, |c| c.rev)
}

/// The shared clipboard's text, if any.
pub fn text() -> Option<String> {
    global().as_ref().and_then(|c| c.text.clone())
}

/// Puts `text` on the shared clipboard.
pub fn set_text(text: &str) {
    if let Some(c) = global().as_mut() {
        c.set_text(text);
    }
}

#[cfg(any(test, feature = "testkit"))]
pub mod tests {
    use super::*;

    /// Serialises tests that use the shared clipboard and starts them empty.
    pub fn exclusive() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *global() = Some(Clipboard::memory());
        g
    }

    #[cfg(test)]
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-clipboard")).join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn text_persists_with_newlines_and_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("a/clipboard.txt");
        let _ = std::fs::remove_file(&path);
        let mut c = Clipboard::load(&path);
        assert_eq!(c.text(), None);
        c.set_text("líne one\nline two\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "líne one\nline two\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!tmp_path(&path).exists());
        let again = Clipboard::load(&path);
        assert_eq!(again.text(), Some("líne one\nline two\n"));
        assert!(again.has_text() && again.image().is_none());
    }

    #[test]
    fn clearing_or_an_image_removes_the_file() {
        let path = scratch("b/clipboard.txt");
        let mut c = Clipboard::load(&path);
        c.set_text("x");
        assert!(path.exists());
        c.set_image(Image::new(2, 2));
        assert!(c.image().is_some() && c.text().is_none());
        assert!(!path.exists(), "an image replaces the saved text");
        c.set_text("y");
        assert!(c.image().is_none() && path.exists());
        c.clear();
        assert!(!c.has_text() && !path.exists());
        c.clear();
        assert!(!path.exists(), "clearing twice is fine");
    }

    #[test]
    fn every_change_moves_the_revision_on() {
        let _g = exclusive();
        let r0 = revision();
        set_text("a");
        let r1 = revision();
        assert_ne!(r0, r1);
        set_text("a");
        assert_ne!(r1, revision(), "setting the same text again is still a copy");
        let r2 = revision();
        let path = scratch("d/clipboard.txt");
        let _ = std::fs::remove_file(&path);
        attach(&path);
        assert_ne!(r2, revision(), "attaching to a file replaces the contents");
        *global() = Some(Clipboard::memory());
    }

    #[test]
    fn memory_clipboard_never_writes() {
        let mut c = Clipboard::memory();
        c.set_text("hello");
        assert_eq!(c.text(), Some("hello"));
        assert_eq!(c.path, None);
    }

    #[test]
    fn the_shared_clipboard_attaches_to_a_file() {
        let _g = exclusive();
        let path = scratch("c/clipboard.txt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "saved").unwrap();
        assert_eq!(text(), None);
        set_text("before");
        attach(&path);
        assert_eq!(text(), Some("saved".into()), "the file wins over the unsaved memory");
        set_text("after");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after");
        global().as_mut().unwrap().set_image(Image::new(1, 1));
        let _ = std::fs::remove_file(&path);
        attach(&path);
        assert!(global().as_ref().unwrap().image().is_some(), "an image survives attaching to an empty file");
        *global() = Some(Clipboard::memory());
    }
}
