//! Installed input methods, served to the shell's Search field: the shell side of `bifrost_ui::imk_host`.
//!
//! An input method is a folder under `<springboard root>/input-methods/` with an `Info.plist` (`IMKControllerClass`,
//! `IMKDelegateClass`, `IMKModeDictionary`; see `bifrost_ui::imk_info`). Two BifrOSt keys pick the built-in engine that
//! converts the keystrokes (`BifrostEngine`: `kana` or `hangul`) and name the language (`BifrostLanguage`, a BCP 47
//! tag); `CFBundleIdentifier` names the bundle, the folder name does when it is missing. Each mode of the method is an
//! input source: the menu bar input menu lists the enabled ones under the keyboard layouts, and choosing one makes the
//! method the active one until a keyboard layout is chosen again (the menu or Ctrl+Space).
//!
//! While a method is active, the keys typed into the Search field go to it first (`Providers::search_key`): it shows
//! the conversion as marked text in the field and offers candidates, which `compose_window` draws. A key the method
//! does not take goes on to the field's own typing. Pure functions and a small adapter over the field; the model is
//! the host in the UI crate.
use std::path::{Path, PathBuf};

use bifrost_core::plist::{self, Plist};
use bifrost_ui::imk_client::{CharacterAttributes, TextInputClient};
use bifrost_ui::imk_host::{CandidateView, Engine, HangulEngine, Host, KanaEngine, KeyResult, PendingCommit};
use bifrost_ui::keys::{Key, KeyCombo};
use std::ops::Range;

use bifrost_proto::WindowInput;

use super::apphost::AppHost;
use super::kb_layouts::Layout;
use super::keyboard::Out;
use super::textfield::TextField;
use super::ui::Ui;
use super::{AppId, Screen, Shell};

/// The client id of the Search field in the host. An external app in slot `n` is client `n + 1`.
pub const SEARCH_CLIENT: u32 = 0;

/// The client id of the external app in `slot`.
fn app_client(slot: u8) -> u32 {
    u32::from(slot) + 1
}

/// Where the input methods are installed.
pub fn dir() -> PathBuf {
    super::apphost::root().join("input-methods")
}

/// The id of keyboard layout `index` (its place in `Layout::ALL`) as an input source.
fn layout_source(index: usize) -> String {
    format!("layout.{index}")
}

/// The string under `key` in an Info dictionary.
fn string_of(info: &Plist, key: &str) -> Option<String> {
    match info {
        Plist::Dict(entries) => entries.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            Plist::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        }),
        _ => None,
    }
}

/// The built-in engine a `BifrostEngine` value names.
fn engine_named(name: &str) -> Option<Box<dyn Engine>> {
    match name {
        "kana" => Some(Box::new(KanaEngine)),
        "hangul" => Some(Box::new(HangulEngine)),
        _ => None,
    }
}

/// The key and text a hardware key stands for in the host, or `None` for a key an input method never sees.
pub fn key_for(out: &Out) -> Option<(String, KeyCombo)> {
    let named = |k: Key| Some((String::new(), KeyCombo::new(k)));
    match out {
        Out::Text(t) if t == " " => Some((t.clone(), KeyCombo::new(Key::Space))),
        Out::Text(t) => {
            let c = t.chars().next()?;
            Some((t.clone(), KeyCombo::new(Key::Char(c.to_lowercase().next().unwrap_or(c)))))
        }
        Out::Backspace => named(Key::Delete),
        Out::Enter => named(Key::Return),
        Out::Named("Escape") => named(Key::Escape),
        Out::Named("Tab") => named(Key::Tab),
        Out::Named("Up") => named(Key::Up),
        Out::Named("Down") => named(Key::Down),
        Out::Named("Left" | "ShiftLeft") => named(Key::Left),
        Out::Named("Right" | "ShiftRight") => named(Key::Right),
        _ => None,
    }
}

/// A text field as the client of an input method. The field counts characters, the protocol bytes, so the adapter
/// converts.
struct FieldClient<'a> {
    ui: &'a Ui,
    field: &'a mut TextField,
}

impl FieldClient<'_> {
    fn byte_of(&self, chars: usize) -> usize {
        let text = self.field.text();
        text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b)
    }
}

impl TextInputClient for FieldClient<'_> {
    fn insert_text(&mut self, text: &str, _replacement: Option<Range<usize>>) {
        // a commit replaces the marked text (or the selection): the field marks it, then keeps it
        self.field.set_marked(self.ui, text, text.len());
        self.field.unmark(self.ui, true);
    }

    fn set_marked_text(&mut self, text: &str, selection: Range<usize>, _replacement: Option<Range<usize>>) {
        self.field.set_marked(self.ui, text, selection.start);
    }

    fn selected_range(&self) -> Range<usize> {
        let at = self.byte_of(self.field.caret());
        at..at
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        let (a, b) = self.field.marked_range()?;
        Some(self.byte_of(a)..self.byte_of(b))
    }

    fn attributes_for_character_index(&self, _index: usize) -> Option<CharacterAttributes> {
        None
    }
}

/// The focused text field of an external app as the client of an input method. The app owns its text: the adapter
/// sends marked text and commits as `SetMarkedText` and `UnmarkText`, and reads the caret and the marked range from the
/// text state the app last reported (the offsets are the app's window of text, not the whole document).
struct AppClient<'a> {
    host: &'a mut AppHost,
}

impl TextInputClient for AppClient<'_> {
    fn insert_text(&mut self, text: &str, _replacement: Option<Range<usize>>) {
        if text.is_empty() {
            // nothing to keep: the composition ends and its marked text goes
            self.host.input(WindowInput::UnmarkText { commit: false });
            return;
        }
        self.host.input(WindowInput::SetMarkedText { text: text.to_string(), sel_start: text.len() as u32, sel_len: 0 });
        self.host.input(WindowInput::UnmarkText { commit: true });
    }

    fn set_marked_text(&mut self, text: &str, selection: Range<usize>, _replacement: Option<Range<usize>>) {
        let sel_len = selection.end.saturating_sub(selection.start);
        self.host.input(WindowInput::SetMarkedText { text: text.to_string(), sel_start: selection.start as u32, sel_len: sel_len as u32 });
    }

    fn selected_range(&self) -> Range<usize> {
        let state = &self.host.text_state;
        let (a, b) = (state.anchor as usize, state.cursor as usize);
        a.min(b)..a.max(b)
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        self.host.text_state.marked.map(|(a, b)| a as usize..b as usize)
    }

    fn attributes_for_character_index(&self, _index: usize) -> Option<CharacterAttributes> {
        None
    }
}

/// The installed input methods and the host that runs them.
pub struct Providers {
    host: Host,
}

impl Default for Providers {
    fn default() -> Self {
        let mut host = Host::new();
        for (i, layout) in Layout::ALL.iter().enumerate() {
            let id = layout_source(i);
            host.install_layout(&id, layout.name(), &[]);
            let _ = host.enable_source(&id);
        }
        host.connect(SEARCH_CLIENT);
        Providers { host }
    }
}

impl Providers {
    /// Registers every input method found under `dir`. Returns how many were registered; a folder without a usable
    /// Info dictionary is skipped.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        folders.iter().filter(|f| self.load_bundle(f)).count()
    }

    /// Registers the input method in `folder`. False when its Info dictionary is missing, unreadable or refused.
    pub fn load_bundle(&mut self, folder: &Path) -> bool {
        let Ok(bytes) = std::fs::read(folder.join("Info.plist")) else { return false };
        let Ok(info) = plist::parse_bytes(&bytes) else { return false };
        let folder_name = folder.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let bundle = string_of(&info, "CFBundleIdentifier").unwrap_or(folder_name);
        let engine = string_of(&info, "BifrostEngine").and_then(|n| engine_named(&n));
        let language = string_of(&info, "BifrostLanguage").unwrap_or_else(|| "und".to_string());
        if self.host.register(&bundle, &info, &[language.as_str()], engine).is_err() {
            return false;
        }
        // an installed method is usable at once: its modes are enabled
        let prefix = format!("{}.", bundle.trim());
        let ids: Vec<String> =
            self.host.sources().list(true).iter().filter(|s| s.id.starts_with(&prefix)).map(|s| s.id.clone()).collect();
        for id in ids {
            let _ = self.host.enable_source(&id);
        }
        true
    }

    /// True while an input method is the selected source.
    pub fn is_method_active(&self) -> bool {
        self.host.is_method_active()
    }

    /// The enabled input modes for the input menu: (source id, label, selected).
    pub fn modes(&self) -> Vec<(String, String, bool)> {
        let current = if self.host.is_method_active() { self.host.sources().current().map(|s| s.id.clone()) } else { None };
        self.host
            .sources()
            .list(false)
            .iter()
            .filter(|s| s.kind == bifrost_ui::input_source::SourceKind::InputMode)
            .map(|s| (s.id.clone(), s.name.clone(), current.as_deref() == Some(s.id.as_str())))
            .collect()
    }

    /// True when at least one input mode is enabled (the input menu then shows even with one keyboard layout).
    pub fn has_modes(&self) -> bool {
        !self.modes().is_empty()
    }

    /// The short label for the menu bar while a method is active: the first character of the mode's name.
    pub fn label(&self) -> Option<String> {
        if !self.host.is_method_active() {
            return None;
        }
        self.host.sources().current()?.name.chars().next().map(|c| c.to_string())
    }

    /// Makes the input mode `id` the active source. The composition the previous source held comes back as texts for
    /// the clients.
    pub fn select(&mut self, id: &str) -> Vec<PendingCommit> {
        self.host.select_source(id).unwrap_or_default()
    }

    /// A keyboard layout was chosen (the menu or Ctrl+Space): no input method is active any more.
    pub fn select_layout(&mut self, layout: Layout) -> Vec<PendingCommit> {
        let Some(index) = Layout::ALL.iter().position(|l| *l == layout) else { return Vec::new() };
        if !self.host.is_method_active() {
            return Vec::new();
        }
        self.host.select_source(&layout_source(index)).unwrap_or_default()
    }

    /// Puts the compositions that a source change ended into the Search field.
    pub fn apply_commits(&mut self, commits: &[PendingCommit], ui: &Ui, field: &mut TextField) {
        for commit in commits.iter().filter(|c| c.client == SEARCH_CLIENT) {
            FieldClient { ui, field: &mut *field }.insert_text(&commit.text, None);
        }
    }

    /// A key typed into the Search field. `Consumed` means the method took it; anything else is for the field's own
    /// typing (when the method committed first, its text is in the field already).
    pub fn search_key(&mut self, out: &Out, ui: &Ui, field: &mut TextField) -> KeyResult {
        if !self.host.is_method_active() {
            return KeyResult::PassThrough;
        }
        let Some((text, combo)) = key_for(out) else { return KeyResult::PassThrough };
        let mut client = FieldClient { ui, field };
        self.host.key(SEARCH_CLIENT, &text, combo, &mut client)
    }

    /// A key typed into the focused field of the external app in `slot`. As for the Search field, `Consumed` means the
    /// method took it; otherwise the app gets the key itself.
    pub fn app_key(&mut self, slot: u8, out: &Out, app: &mut AppHost) -> KeyResult {
        if !self.host.is_method_active() {
            return KeyResult::PassThrough;
        }
        let Some((text, combo)) = key_for(out) else { return KeyResult::PassThrough };
        let client = app_client(slot);
        self.host.connect(client);
        self.host.key(client, &text, combo, &mut AppClient { host: app })
    }

    /// The candidate window of the external app in `slot`.
    pub fn app_candidate_view(&self, slot: u8) -> Option<CandidateView> {
        self.host.candidate_view(app_client(slot))
    }

    /// Puts a composition that a source change ended into the external app in `slot`.
    pub fn apply_app_commit(&mut self, text: &str, app: &mut AppHost) {
        AppClient { host: app }.insert_text(text, None);
    }

    /// The candidate window over the Search field, when a conversion offers one.
    pub fn candidate_view(&self) -> Option<CandidateView> {
        self.host.candidate_view(SEARCH_CLIENT)
    }
}

impl Shell {
    /// Puts the compositions that a source change ended into the clients that held them: the Search field and the
    /// external apps.
    pub(super) fn apply_provider_commits(&mut self, commits: Vec<PendingCommit>) {
        self.input_providers.apply_commits(&commits, &self.ui, self.quicksearch.field_mut());
        for commit in commits.iter().filter(|c| c.client != SEARCH_CLIENT) {
            let slot = (commit.client - 1) as usize;
            if let Some(Some(app)) = self.ext.get_mut(slot) {
                self.input_providers.apply_app_commit(&commit.text, app);
            }
        }
        self.dirty = true;
    }

    /// The candidate window to draw: the Search field's (no caret rectangle), else that of the external app in front
    /// with its caret rectangle in screen pixels (the app's caret in its window, below the title bar).
    pub(super) fn provider_candidates(&self) -> Option<(CandidateView, Option<(f32, f32, f32, f32)>)> {
        if let Some(view) = self.input_providers.candidate_view() {
            return Some((view, None));
        }
        let slot = self.ext_fg?;
        if !matches!(self.screen, Screen::App(AppId::External(_))) {
            return None;
        }
        let view = self.input_providers.app_candidate_view(slot)?;
        let (cx, cy, cw, ch) = self.ext.get(slot as usize)?.as_ref()?.caret_px()?;
        let win = self.desk.as_ref()?.wins.iter().find(|w| w.app == AppId::External(slot))?;
        let body = win.body(&self.desk_metrics());
        Some((view, Some((body.x as f32 + cx, body.y as f32 + cy, cw, ch))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KANA_INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.example.kana</string>
<key>BifrostEngine</key><string>kana</string>
<key>BifrostLanguage</key><string>ja</string>
<key>IMKControllerClass</key><string>KanaController</string>
<key>IMKModeDictionary</key><dict>
<key>romaji</key><dict><key>label</key><string>Romaji</string></dict>
</dict></dict></plist>"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-input-providers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hardware_keys_map_onto_host_keys() {
        let (t, c) = key_for(&Out::Text("A".into())).unwrap();
        assert_eq!((t.as_str(), c.key), ("A", Key::Char('a')));
        assert_eq!(key_for(&Out::Text(" ".into())).unwrap().1.key, Key::Space);
        assert_eq!(key_for(&Out::Backspace).unwrap().1.key, Key::Delete);
        assert_eq!(key_for(&Out::Enter).unwrap().1.key, Key::Return);
        assert_eq!(key_for(&Out::Named("Escape")).unwrap().1.key, Key::Escape);
        assert_eq!(key_for(&Out::Named("Down")).unwrap().1.key, Key::Down);
        assert_eq!(key_for(&Out::Named("ShiftLeft")).unwrap().1.key, Key::Left, "Shift with an arrow still moves the caret");
        assert_eq!(key_for(&Out::Named("ShiftRight")).unwrap().1.key, Key::Right);
        assert!(key_for(&Out::Named("Paste")).is_none());
        assert!(key_for(&Out::Text(String::new())).is_none());
        assert!(key_for(&Out::Hide).is_none());
    }

    #[test]
    fn a_folder_with_an_info_dictionary_is_registered_and_its_modes_enabled() {
        let root = scratch("load");
        let kana = root.join("Kana.imk");
        std::fs::create_dir_all(&kana).unwrap();
        std::fs::write(kana.join("Info.plist"), KANA_INFO).unwrap();
        // a folder with no Info.plist and a loose file are skipped
        std::fs::create_dir_all(root.join("Empty.imk")).unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        let mut p = Providers::default();
        assert!(!p.has_modes());
        assert_eq!(p.load_dir(&root), 1);
        assert_eq!(p.modes(), vec![("org.example.kana.romaji".to_string(), "Romaji".to_string(), false)]);
        assert!(p.has_modes() && !p.is_method_active() && p.label().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_directory_registers_nothing() {
        let mut p = Providers::default();
        assert_eq!(p.load_dir(Path::new("/nonexistent/input-methods")), 0);
    }

    #[test]
    fn choosing_a_mode_activates_the_method_and_a_layout_ends_it() {
        let root = scratch("select");
        let kana = root.join("Kana.imk");
        std::fs::create_dir_all(&kana).unwrap();
        std::fs::write(kana.join("Info.plist"), KANA_INFO).unwrap();
        let mut p = Providers::default();
        p.load_dir(&root);
        assert!(p.select("org.example.kana.romaji").is_empty());
        assert!(p.is_method_active());
        assert_eq!(p.label().as_deref(), Some("R"));
        assert!(p.modes()[0].2, "the mode shows as selected");
        assert!(p.select_layout(Layout::English).is_empty());
        assert!(!p.is_method_active());
        assert!(!p.modes()[0].2);
        // with no method active a layout choice changes nothing
        assert!(p.select_layout(Layout::Icelandic).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_source_changes_nothing() {
        let mut p = Providers::default();
        assert!(p.select("no.such.source").is_empty());
        assert!(!p.is_method_active());
    }
}
