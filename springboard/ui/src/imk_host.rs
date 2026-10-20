//! The input method host: what the shell runs so that a registered input method can serve a client text input.
//!
//! It joins the pieces of the input method model into one flow. An Info dictionary registers an input method
//! ([`Host::register`], parsed by `imk_info`), which starts a [`Server`] and installs one input source per mode
//! (`input_source`). Picking a source (the menu bar input menu, [`Host::select_source`]) makes that method the active
//! one, switches its sessions to the chosen mode and ends the compositions of the method that was active. A client
//! connects once ([`Host::connect`]) and gets one session in every method. Its key, command and pointer events go
//! through [`Host::key`], [`Host::command`] and [`Host::mouse`], which route them to the active method's controller,
//! let the conversion [`Engine`] turn the keystrokes into marked text and candidates, and apply the result to the
//! client through [`TextInputClient`]. The candidate window the controller keeps is read with
//! [`Host::candidate_view`] and placed with [`Host::candidate_origin`].
//!
//! Pure model: no socket, no drawing, no file access. Offsets are UTF-8 byte offsets in the client's text.

use std::collections::BTreeMap;

use bifrost_core::plist::Plist;

use crate::candidates::{LocationHint, PanelType};
use crate::hangul_method::HangulMethod;
use crate::imk_client::{apply, update_composition, TextInputClient};
use crate::imk_info::{registration_from_info, InfoError};
use crate::input_method::{Event, InputController, MouseKind, Outcome, Server, ServerError};
use crate::input_source::{InputSource, InputSources, SourceError, SourceKind};
use crate::kana_method::KanaMethod;
use crate::keys::{Key, KeyCombo};
use crate::text::EditCommand;

/// What an engine makes of the keystrokes typed so far: the text shown as marked text, the caret inside it (a byte
/// offset in `composed`) and the alternatives for the candidate window (empty when there are none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub composed: String,
    pub caret: usize,
    pub candidates: Vec<String>,
}

/// The conversion engine of an input method. It is stateless between calls: the host hands it the whole keystroke
/// string of the composition each time, so a backspace is a shorter string.
pub trait Engine {
    /// Converts `keystrokes` in input mode `mode`. `None` means the keystrokes are not a composition (the last key is
    /// not one this method takes), and the host ends the composition and gives the key back to the client.
    fn convert(&mut self, mode: &str, keystrokes: &str) -> Option<Conversion>;
}

/// Romaji to kana, with the hiragana and katakana forms as candidates (`kana_method`).
pub struct KanaEngine;

impl Engine for KanaEngine {
    fn convert(&mut self, _mode: &str, keystrokes: &str) -> Option<Conversion> {
        let mut method = KanaMethod::new();
        for c in keystrokes.chars() {
            if !method.push(c) {
                return None;
            }
        }
        if !method.is_composing() {
            return None;
        }
        let composed = method.marked();
        Some(Conversion { caret: composed.len(), candidates: method.candidates(), composed })
    }
}

/// Two-set Hangul: keys become jamo and join into syllables (`hangul_method`). It offers no candidates.
pub struct HangulEngine;

impl Engine for HangulEngine {
    fn convert(&mut self, _mode: &str, keystrokes: &str) -> Option<Conversion> {
        let mut method = HangulMethod::new();
        for c in keystrokes.chars() {
            if !method.push(c) {
                return None;
            }
        }
        if !method.is_composing() {
            return None;
        }
        let composed = method.marked();
        Some(Conversion { caret: composed.len(), candidates: Vec::new(), composed })
    }
}

/// Why the host refused a registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The bundle identifier is blank.
    BadBundle,
    /// A method with this bundle identifier is registered already.
    Duplicate(String),
    /// The Info dictionary does not describe a usable input method.
    Info(InfoError),
    /// The server refused the record.
    Server(ServerError),
}

/// What happened to a client's key, command or pointer event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    /// The input method used the event. The client does nothing more.
    Consumed,
    /// The event is the client's. No composition was open, or no method is active.
    PassThrough,
    /// The composition was committed into the client already. The client now handles the event itself: it types
    /// the key, runs the command or places its caret.
    CommittedThenPass,
}

/// Text to insert into a client when the active method changed or a client left: the composition it still held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCommit {
    pub client: u32,
    pub text: String,
}

/// The candidate window as the shell draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateView {
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub hint: LocationHint,
    pub opacity: f32,
    pub panel: PanelType,
}

/// One registered input method.
struct Provider {
    server: Server,
    engine: Option<Box<dyn Engine>>,
    /// (input source id, mode id) of each mode the method offers.
    sources: Vec<(String, String)>,
}

/// The registered input methods, the input sources and the connected clients.
#[derive(Default)]
pub struct Host {
    providers: BTreeMap<String, Provider>,
    sources: InputSources,
    /// The bundle of the method that is active, or `None` when a keyboard layout is selected.
    active: Option<String>,
    clients: Vec<u32>,
}

/// The editing command a name stands for, for a provider that reports its commands by name (`didCommand`). The names
/// are the snake case form of the command: `move_left`, `delete_backward`, `select_all` and so on.
pub fn command_named(name: &str) -> Option<EditCommand> {
    use EditCommand::*;
    Some(match name {
        "move_left" => MoveLeft,
        "move_right" => MoveRight,
        "move_up" => MoveUp,
        "move_down" => MoveDown,
        "move_word_left" => MoveWordLeft,
        "move_word_right" => MoveWordRight,
        "move_line_start" => MoveLineStart,
        "move_line_end" => MoveLineEnd,
        "move_document_start" => MoveDocumentStart,
        "move_document_end" => MoveDocumentEnd,
        "move_page_up" => MovePageUp,
        "move_page_down" => MovePageDown,
        "delete_backward" => DeleteBackward,
        "delete_forward" => DeleteForward,
        "delete_word_backward" => DeleteWordBackward,
        "delete_word_forward" => DeleteWordForward,
        "delete_to_line_start" => DeleteToLineStart,
        "kill_line" => KillLine,
        "yank" => Yank,
        "transpose" => Transpose,
        "open_line" => OpenLine,
        "select_all" => SelectAll,
        "undo" => Undo,
        "redo" => Redo,
        _ => return None,
    })
}

/// True when the key that ended a composition is consumed with it: Return commits and is not typed, a digit picked a
/// candidate. Any other key that commits is then the client's.
fn consumed_by_commit(combo: KeyCombo) -> bool {
    !(combo.cmd || combo.ctrl || combo.alt) && matches!(combo.key, Key::Return | Key::Char('1'..='9'))
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    /// The input sources: keyboard layouts the shell installed and the modes of the registered methods.
    pub fn sources(&self) -> &InputSources {
        &self.sources
    }

    /// Installs a keyboard layout as an input source, so the input menu lists it next to the methods. Install the
    /// layouts before registering methods: the first selectable source installed is the selected one.
    pub fn install_layout(&mut self, id: &str, name: &str, languages: &[&str]) {
        self.sources.install(InputSource::new(id, name, SourceKind::KeyboardLayout, languages));
    }

    /// Enables a source (it shows in the input menu).
    pub fn enable_source(&mut self, id: &str) -> Result<(), SourceError> {
        self.sources.enable(id)
    }

    /// The registered bundle identifiers, in order.
    pub fn bundles(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Registers an input method from its Info dictionary. Each mode becomes an input source with the id
    /// `<bundle>.<mode>`, installed but not yet enabled. Clients that are connected already get a session.
    pub fn register(
        &mut self,
        bundle: &str,
        info: &Plist,
        languages: &[&str],
        engine: Option<Box<dyn Engine>>,
    ) -> Result<(), HostError> {
        let bundle = bundle.trim();
        if bundle.is_empty() {
            return Err(HostError::BadBundle);
        }
        if self.providers.contains_key(bundle) {
            return Err(HostError::Duplicate(bundle.to_string()));
        }
        let registration = registration_from_info(info).map_err(HostError::Info)?;
        let modes: Vec<(String, String)> = registration.modes().iter().map(|m| (m.id.clone(), m.label.clone())).collect();
        let mut server = Server::new(registration).map_err(HostError::Server)?;
        let mut sources = Vec::new();
        for (mode, label) in modes {
            let id = format!("{bundle}.{mode}");
            self.sources.install(InputSource::new(&id, &label, SourceKind::InputMode, languages));
            sources.push((id, mode));
        }
        for client in &self.clients {
            let _ = server.connect(*client);
        }
        self.providers.insert(bundle.to_string(), Provider { server, engine, sources });
        Ok(())
    }

    /// The bundle and the mode of the active method, or `None` when a keyboard layout is selected.
    pub fn active(&self) -> Option<(&str, &str)> {
        let bundle = self.active.as_deref()?;
        let provider = self.providers.get(bundle)?;
        let client = *self.clients.first()?;
        let mode = provider.server.controller(client)?.state().mode();
        Some((bundle, mode))
    }

    /// True while an input method (not a plain keyboard layout) is the selected source.
    pub fn is_method_active(&self) -> bool {
        self.active.is_some()
    }

    /// Selects an input source. A method's mode makes that method active and switches its sessions to the mode; a
    /// keyboard layout makes no method active. The composition of every client of the method that stops being active
    /// (or changes mode) is committed, and the texts come back so the caller can insert them.
    pub fn select_source(&mut self, id: &str) -> Result<Vec<PendingCommit>, SourceError> {
        self.sources.select(id)?;
        let target = self.providers.iter().find_map(|(bundle, p)| {
            p.sources.iter().find(|(sid, _)| sid == id).map(|(_, mode)| (bundle.clone(), mode.clone()))
        });
        let mut commits = Vec::new();
        let previous = self.active.take();
        match target {
            Some((bundle, mode)) => {
                if let Some(prev) = previous.as_deref().filter(|p| *p != bundle) {
                    self.commit_all(prev, &mut commits);
                }
                if let Some(provider) = self.providers.get_mut(&bundle) {
                    for client in provider.server.clients() {
                        if let Some(controller) = provider.server.controller_mut(client) {
                            if let Ok(Some(text)) = controller.set_mode(&mode) {
                                commits.push(PendingCommit { client, text });
                            }
                        }
                    }
                }
                self.active = Some(bundle);
            }
            None => {
                if let Some(prev) = previous.as_deref() {
                    self.commit_all(prev, &mut commits);
                }
            }
        }
        Ok(commits)
    }

    /// Commits the composition of every client of `bundle` into `out`.
    fn commit_all(&mut self, bundle: &str, out: &mut Vec<PendingCommit>) {
        let Some(provider) = self.providers.get_mut(bundle) else { return };
        for client in provider.server.clients() {
            if let Some(controller) = provider.server.controller_mut(client) {
                if let Some(text) = controller.commit_composition() {
                    out.push(PendingCommit { client, text });
                }
            }
        }
    }

    /// A client starts a text input: it gets a session in every registered method.
    pub fn connect(&mut self, client: u32) {
        if self.clients.contains(&client) {
            return;
        }
        self.clients.push(client);
        for provider in self.providers.values_mut() {
            let _ = provider.server.connect(client);
        }
    }

    /// A client leaves. Returns the composition the active method still held for it, to be inserted.
    pub fn disconnect(&mut self, client: u32) -> Option<String> {
        self.clients.retain(|c| *c != client);
        let mut pending = None;
        for (bundle, provider) in self.providers.iter_mut() {
            if self.active.as_deref() == Some(bundle.as_str()) {
                if let Some(controller) = provider.server.controller_mut(client) {
                    pending = controller.commit_composition();
                }
            }
            provider.server.disconnect(client);
        }
        pending
    }

    fn controller(&self, client: u32) -> Option<&InputController> {
        let bundle = self.active.as_ref()?;
        self.providers.get(bundle)?.server.controller(client)
    }

    /// The converted text of the client's composition (`composedString`).
    pub fn composed_string(&self, client: u32) -> Option<String> {
        self.controller(client)?.composed_string().map(|s| s.to_string())
    }

    /// The keystrokes of the client's composition as typed (`originalString`).
    pub fn original_string(&self, client: u32) -> Option<String> {
        self.controller(client)?.original_string().map(|s| s.to_string())
    }

    /// The candidates offered for the client's composition (`candidates`).
    pub fn candidates(&self, client: u32) -> Vec<String> {
        self.controller(client).map(|c| c.candidates().items().to_vec()).unwrap_or_default()
    }

    /// The caret inside the client's composed text.
    pub fn composition_caret(&self, client: u32) -> Option<usize> {
        self.controller(client)?.composition().map(|c| c.caret())
    }

    /// True while the client has a composition open.
    pub fn is_composing(&self, client: u32) -> bool {
        self.controller(client).map(|c| c.composition().is_some()).unwrap_or(false)
    }

    /// The candidate window for the client, when it is up.
    pub fn candidate_view(&self, client: u32) -> Option<CandidateView> {
        let window = self.controller(client)?.candidates();
        if !window.is_visible() || window.items().is_empty() {
            return None;
        }
        Some(CandidateView {
            items: window.items().to_vec(),
            selected: window.selected(),
            hint: window.location_hint(),
            opacity: window.opacity(),
            panel: window.panel_type(),
        })
    }

    /// Where the candidate window of the client goes: on the preferred side of the caret rectangle, flipped when it
    /// does not fit (`candidates::place`).
    #[allow(clippy::too_many_arguments)]
    pub fn candidate_origin(
        &self,
        client: u32,
        caret: (f32, f32, f32, f32),
        w: f32,
        h: f32,
        screen_w: f32,
        screen_h: f32,
        gap: f32,
    ) -> Option<(f32, f32)> {
        Some(self.controller(client)?.candidates().position(caret, w, h, screen_w, screen_h, gap))
    }

    /// `commitComposition`: the client's composition is inserted as it is. False when none was open.
    pub fn commit(&mut self, client: u32, target: &mut dyn TextInputClient) -> bool {
        let Some(text) = self.controller_mut(client).and_then(|c| c.commit_composition()) else { return false };
        apply(&Outcome::Committed(text), target);
        true
    }

    /// The client's composition is dropped and the keystrokes as typed replace the marked text. False when none
    /// was open.
    pub fn cancel(&mut self, client: u32, target: &mut dyn TextInputClient) -> bool {
        let Some(text) = self.controller_mut(client).and_then(|c| c.cancel_composition()) else { return false };
        apply(&Outcome::Cancelled(text), target);
        true
    }

    /// A candidate was clicked: it replaces the composition. False when no composition is open or the index is out
    /// of range.
    pub fn pick_candidate(&mut self, client: u32, index: usize, target: &mut dyn TextInputClient) -> bool {
        let Some(text) = self.controller_mut(client).and_then(|c| c.pick_candidate(index)) else { return false };
        apply(&Outcome::Committed(text), target);
        true
    }

    fn controller_mut(&mut self, client: u32) -> Option<&mut InputController> {
        let bundle = self.active.as_ref()?;
        self.providers.get_mut(bundle)?.server.controller_mut(client)
    }

    /// A key from the client, with the text it makes (empty when it makes none). `target` is the client's text.
    pub fn key(&mut self, client: u32, text: &str, combo: KeyCombo, target: &mut dyn TextInputClient) -> KeyResult {
        let Some(bundle) = self.active.clone() else { return KeyResult::PassThrough };
        let Some(Provider { server, engine, .. }) = self.providers.get_mut(&bundle) else { return KeyResult::PassThrough };
        let Some(ctl) = server.controller_mut(client) else { return KeyResult::PassThrough };
        if !ctl.state().is_active() {
            return KeyResult::PassThrough;
        }
        let plain = !(combo.cmd || combo.ctrl || combo.alt);
        let composing = ctl.composition().is_some();
        let visible = ctl.candidates().is_visible();
        let typed = plain && !text.is_empty() && text.chars().all(|c| !c.is_control());
        let digit_pick = composing && visible && matches!(combo.key, Key::Char('1'..='9'));

        // Space picks the selected candidate (the first with none selected); Return picks the selected one
        if composing && plain && visible {
            let pick = if combo.key == Key::Space || text == " " {
                Some(ctl.candidates().selected().unwrap_or(0))
            } else if combo.key == Key::Return {
                ctl.candidates().selected()
            } else {
                None
            };
            if let Some(index) = pick {
                if let Some(picked) = ctl.pick_candidate(index) {
                    apply(&Outcome::Committed(picked), target);
                    return KeyResult::Consumed;
                }
            }
        }

        if let Some(eng) = engine.as_mut() {
            if typed && !digit_pick {
                let mut keystrokes = ctl.original_string().unwrap_or("").to_string();
                keystrokes.push_str(text);
                return engine_step(ctl, &mut **eng, &keystrokes, composing, true, target);
            }
            if plain && composing && combo.key == Key::Delete {
                let mut keystrokes = ctl.original_string().unwrap_or("").to_string();
                keystrokes.pop();
                if keystrokes.is_empty() {
                    ctl.cancel_composition();
                    apply(&Outcome::Cancelled(String::new()), target);
                    return KeyResult::Consumed;
                }
                return engine_step(ctl, &mut **eng, &keystrokes, true, false, target);
            }
        }

        let outcome = ctl.handle(&Event::Key { text: text.to_string(), combo });
        finish(ctl, &outcome, consumed_by_commit(combo), target)
    }

    /// An editing command from the client (a menu item or a key binding), as the provider reports it (`didCommand`).
    pub fn command(&mut self, client: u32, command: EditCommand, target: &mut dyn TextInputClient) -> KeyResult {
        let Some(ctl) = self.controller_mut(client) else { return KeyResult::PassThrough };
        let outcome = ctl.handle(&Event::Command(command));
        finish(ctl, &outcome, false, target)
    }

    /// A command by its name (see [`command_named`]). An unknown name is the client's.
    pub fn command_by_name(&mut self, client: u32, name: &str, target: &mut dyn TextInputClient) -> KeyResult {
        match command_named(name) {
            Some(command) => self.command(client, command, target),
            None => KeyResult::PassThrough,
        }
    }

    /// A pointer event at byte offset `at` in the client's text, so the input method sees a click during a
    /// composition: inside the marked text it moves the composition caret, outside it commits the composition and the
    /// client then places its own caret.
    pub fn mouse(&mut self, client: u32, kind: MouseKind, at: usize, target: &mut dyn TextInputClient) -> KeyResult {
        let Some(ctl) = self.controller_mut(client) else { return KeyResult::PassThrough };
        let outcome = ctl.handle(&Event::Mouse { kind, at });
        finish(ctl, &outcome, false, target)
    }
}

/// Runs the engine over `keystrokes`. A conversion becomes the composition and its marked text. A rejection ends the
/// composition when `give_back` is set (the key is then the client's), else it drops the composition.
fn engine_step(
    ctl: &mut InputController,
    engine: &mut dyn Engine,
    keystrokes: &str,
    composing: bool,
    give_back: bool,
    target: &mut dyn TextInputClient,
) -> KeyResult {
    let mode = ctl.state().mode().to_string();
    match engine.convert(&mode, keystrokes) {
        Some(conv) => {
            let start = match ctl.composition() {
                Some(c) => c.marked_range().start,
                None => target.selected_range().start,
            };
            ctl.set_composition(start, keystrokes, &conv.composed, conv.caret);
            if conv.candidates.is_empty() {
                ctl.clear_candidates();
            } else {
                ctl.offer_candidates(conv.candidates);
            }
            update_composition(ctl, target);
            KeyResult::Consumed
        }
        None if composing && give_back => {
            let text = ctl.commit_composition().unwrap_or_default();
            apply(&Outcome::Committed(text), target);
            KeyResult::CommittedThenPass
        }
        None if composing => {
            ctl.cancel_composition();
            apply(&Outcome::Cancelled(String::new()), target);
            KeyResult::Consumed
        }
        None => KeyResult::PassThrough,
    }
}

/// Applies what the controller reported for an event to the client.
fn finish(ctl: &InputController, outcome: &Outcome, consumes: bool, target: &mut dyn TextInputClient) -> KeyResult {
    match outcome {
        Outcome::PassThrough => KeyResult::PassThrough,
        Outcome::Consumed => {
            // the caret or the text of the composition may have moved
            update_composition(ctl, target);
            KeyResult::Consumed
        }
        Outcome::Cancelled(_) => {
            apply(outcome, target);
            KeyResult::Consumed
        }
        Outcome::Committed(_) => {
            apply(outcome, target);
            if consumes {
                KeyResult::Consumed
            } else {
                KeyResult::CommittedThenPass
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imk_client::CharacterAttributes;
    use std::ops::Range;

    #[derive(Default)]
    struct Mock {
        text: String,
        marked: Option<Range<usize>>,
    }

    impl TextInputClient for Mock {
        fn insert_text(&mut self, text: &str, replacement: Option<Range<usize>>) {
            let r = replacement.unwrap_or(self.text.len()..self.text.len());
            self.text.replace_range(r, text);
            self.marked = None;
        }
        fn set_marked_text(&mut self, text: &str, _selection: Range<usize>, replacement: Option<Range<usize>>) {
            let r = replacement.unwrap_or(self.text.len()..self.text.len());
            let start = r.start;
            self.text.replace_range(r, text);
            self.marked = Some(start..start + text.len());
        }
        fn selected_range(&self) -> Range<usize> {
            self.text.len()..self.text.len()
        }
        fn marked_range(&self) -> Option<Range<usize>> {
            self.marked.clone()
        }
        fn attributes_for_character_index(&self, _index: usize) -> Option<CharacterAttributes> {
            None
        }
    }

    fn info(modes: &[(&str, &str)]) -> Plist {
        let modes = modes
            .iter()
            .map(|(id, label)| (id.to_string(), Plist::Dict(vec![("label".to_string(), Plist::Str(label.to_string()))])))
            .collect();
        Plist::Dict(vec![
            ("IMKControllerClass".to_string(), Plist::Str("Controller".to_string())),
            ("IMKModeDictionary".to_string(), Plist::Dict(modes)),
        ])
    }

    /// A host with the Icelandic layout and a kana method, the kana mode selected, and client 1 connected.
    fn host() -> Host {
        let mut h = Host::new();
        h.install_layout("layout.is", "Icelandic", &["is"]);
        h.register("org.example.kana", &info(&[("romaji", "Romaji")]), &["ja"], Some(Box::new(KanaEngine))).unwrap();
        h.enable_source("org.example.kana.romaji").unwrap();
        h.connect(1);
        h.select_source("org.example.kana.romaji").unwrap();
        h
    }

    fn type_text(h: &mut Host, m: &mut Mock, s: &str) {
        for c in s.chars() {
            let r = h.key(1, &c.to_string(), KeyCombo::new(Key::Char(c)), m);
            assert_eq!(r, KeyResult::Consumed, "key {c}");
        }
    }

    fn press(h: &mut Host, m: &mut Mock, k: Key, text: &str) -> KeyResult {
        h.key(1, text, KeyCombo::new(k), m)
    }

    #[test]
    fn registering_installs_a_source_per_mode_and_rejects_bad_records() {
        let mut h = Host::new();
        h.install_layout("layout.is", "Icelandic", &["is"]);
        h.register("org.example.ime", &info(&[("a", "A"), ("b", "B")]), &["ja"], None).unwrap();
        assert_eq!(h.sources().list(true).len(), 3);
        assert_eq!(h.bundles(), vec!["org.example.ime".to_string()]);
        // the layout stays selected, a mode is installed but not enabled
        assert_eq!(h.sources().current().map(|s| s.id.as_str()), Some("layout.is"));
        assert_eq!(h.sources().list(false).len(), 1);
        assert_eq!(h.register("org.example.ime", &info(&[("a", "A")]), &[], None), Err(HostError::Duplicate("org.example.ime".into())));
        assert_eq!(h.register(" ", &info(&[("a", "A")]), &[], None), Err(HostError::BadBundle));
        let no_modes = Plist::Dict(vec![("IMKControllerClass".to_string(), Plist::Str("C".to_string()))]);
        assert!(matches!(h.register("org.example.other", &no_modes, &[], None), Err(HostError::Info(_))));
    }

    #[test]
    fn a_layout_passes_keys_to_the_client() {
        let mut h = Host::new();
        h.install_layout("layout.is", "Icelandic", &["is"]);
        h.connect(1);
        let mut m = Mock::default();
        assert_eq!(press(&mut h, &mut m, Key::Char('a'), "a"), KeyResult::PassThrough);
        assert!(!h.is_method_active());
    }

    #[test]
    fn typing_romaji_shows_marked_kana_and_candidates() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "k");
        assert_eq!(m.text, "k");
        assert!(h.candidate_view(1).is_none(), "no candidates until a syllable is complete");
        type_text(&mut h, &mut m, "a");
        assert_eq!(m.text, "か");
        assert_eq!(m.marked, Some(0..3));
        assert_eq!(h.composed_string(1).as_deref(), Some("か"));
        assert_eq!(h.original_string(1).as_deref(), Some("ka"));
        assert_eq!(h.candidates(1), vec!["か".to_string(), "カ".to_string()]);
        let view = h.candidate_view(1).unwrap();
        assert_eq!(view.selected, None);
        assert_eq!(view.panel, PanelType::SingleColumnScrolling);
    }

    #[test]
    fn space_picks_the_first_candidate_and_a_digit_picks_its_own() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert_eq!(press(&mut h, &mut m, Key::Space, " "), KeyResult::Consumed);
        assert_eq!((m.text.as_str(), m.marked.clone()), ("か", None));
        assert!(!h.is_composing(1));
        type_text(&mut h, &mut m, "ka");
        assert_eq!(press(&mut h, &mut m, Key::Char('2'), "2"), KeyResult::Consumed);
        assert_eq!(m.text, "かカ");
        assert_eq!(m.marked, None);
    }

    #[test]
    fn down_then_return_picks_the_selected_candidate() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        press(&mut h, &mut m, Key::Down, "");
        press(&mut h, &mut m, Key::Down, "");
        assert_eq!(h.candidate_view(1).unwrap().selected, Some(1));
        assert_eq!(press(&mut h, &mut m, Key::Return, ""), KeyResult::Consumed);
        assert_eq!(m.text, "カ");
    }

    #[test]
    fn return_without_a_selection_commits_the_composed_text_and_is_not_typed() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert_eq!(press(&mut h, &mut m, Key::Return, ""), KeyResult::Consumed);
        assert_eq!(m.text, "か");
        assert_eq!(m.marked, None);
    }

    #[test]
    fn backspace_replays_the_remaining_keystrokes_and_the_last_one_cancels() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert_eq!(press(&mut h, &mut m, Key::Delete, ""), KeyResult::Consumed);
        assert_eq!(m.text, "k");
        assert_eq!(press(&mut h, &mut m, Key::Delete, ""), KeyResult::Consumed);
        assert_eq!(m.text, "");
        assert!(!h.is_composing(1));
        // with nothing composed, backspace is the client's
        assert_eq!(press(&mut h, &mut m, Key::Delete, ""), KeyResult::PassThrough);
    }

    #[test]
    fn a_key_the_engine_refuses_commits_and_goes_back_to_the_client() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        // a digit with the candidate window closed is not romaji; "k" leaves no window, so use punctuation here
        let r = h.key(1, ".", KeyCombo::new(Key::Char('.')), &mut m);
        assert_eq!(r, KeyResult::CommittedThenPass);
        assert_eq!((m.text.as_str(), m.marked.clone()), ("か", None));
        // with no composition a refused key is plain typing
        assert_eq!(h.key(1, "1", KeyCombo::new(Key::Char('1')), &mut m), KeyResult::PassThrough);
    }

    #[test]
    fn a_shortcut_commits_the_composition_and_the_client_runs_it() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        let mut combo = KeyCombo::new(Key::Char('s'));
        combo.cmd = true;
        assert_eq!(h.key(1, "", combo, &mut m), KeyResult::CommittedThenPass);
        assert_eq!(m.text, "か");
        assert_eq!(m.marked, None);
    }

    #[test]
    fn escape_drops_the_composition() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert_eq!(press(&mut h, &mut m, Key::Escape, ""), KeyResult::Consumed);
        // the keystrokes as typed replace the marked text
        assert_eq!(m.text, "ka");
        assert_eq!(m.marked, None);
    }

    #[test]
    fn commands_move_the_composition_caret_or_commit() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert_eq!(h.composition_caret(1), Some(3));
        assert_eq!(h.command(1, EditCommand::MoveLeft, &mut m), KeyResult::Consumed);
        assert_eq!(h.composition_caret(1), Some(0));
        assert_eq!(h.command_by_name(1, "select_all", &mut m), KeyResult::CommittedThenPass);
        assert_eq!(m.text, "か");
        assert_eq!(h.command_by_name(1, "no_such_command", &mut m), KeyResult::PassThrough);
        assert_eq!(command_named("kill_line"), Some(EditCommand::KillLine));
        assert_eq!(command_named("kill"), None);
    }

    #[test]
    fn a_click_inside_the_marked_text_is_the_methods_and_outside_it_commits() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "kaki");
        assert_eq!(m.text, "かき");
        assert_eq!(h.mouse(1, MouseKind::Down, 3, &mut m), KeyResult::Consumed);
        assert_eq!(h.composition_caret(1), Some(3));
        assert_eq!(h.mouse(1, MouseKind::Down, 40, &mut m), KeyResult::CommittedThenPass);
        assert_eq!((m.text.as_str(), m.marked.clone()), ("かき", None));
        // no composition open: the click is the client's
        assert_eq!(h.mouse(1, MouseKind::Down, 0, &mut m), KeyResult::PassThrough);
    }

    #[test]
    fn a_clicked_candidate_replaces_the_composition() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert!(!h.pick_candidate(1, 5, &mut m));
        assert!(h.pick_candidate(1, 1, &mut m));
        assert_eq!(m.text, "カ");
        assert!(!h.pick_candidate(1, 0, &mut m));
    }

    #[test]
    fn commit_and_cancel_end_the_composition_on_request() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        assert!(h.commit(1, &mut m));
        assert_eq!(m.text, "か");
        assert!(!h.commit(1, &mut m));
        type_text(&mut h, &mut m, "ki");
        assert!(h.cancel(1, &mut m));
        assert_eq!(m.text, "かki");
    }

    #[test]
    fn switching_to_a_layout_commits_the_composition() {
        let mut h = host();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        let commits = h.select_source("layout.is").unwrap();
        assert_eq!(commits, vec![PendingCommit { client: 1, text: "か".to_string() }]);
        assert!(!h.is_method_active());
        assert_eq!(press(&mut h, &mut m, Key::Char('a'), "a"), KeyResult::PassThrough);
        // a source that is not enabled cannot be selected
        assert_eq!(h.select_source("nope"), Err(SourceError::Unknown("nope".into())));
    }

    #[test]
    fn switching_the_mode_commits_and_a_leaving_client_hands_back_its_composition() {
        let mut h = Host::new();
        h.install_layout("layout.is", "Icelandic", &["is"]);
        h.register("org.example.kana", &info(&[("hira", "Hiragana"), ("kata", "Katakana")]), &["ja"], Some(Box::new(KanaEngine))).unwrap();
        h.enable_source("org.example.kana.hira").unwrap();
        h.enable_source("org.example.kana.kata").unwrap();
        h.connect(1);
        h.select_source("org.example.kana.hira").unwrap();
        assert_eq!(h.active(), Some(("org.example.kana", "hira")));
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "ka");
        let commits = h.select_source("org.example.kana.kata").unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(h.active(), Some(("org.example.kana", "kata")));
        h.connect(2);
        let mut m2 = Mock::default();
        for c in "ki".chars() {
            h.key(2, &c.to_string(), KeyCombo::new(Key::Char(c)), &mut m2);
        }
        assert_eq!(h.disconnect(2), Some("き".to_string()));
        assert!(!h.is_composing(2));
    }

    #[test]
    fn a_method_registered_after_a_client_connected_serves_that_client() {
        let mut h = Host::new();
        h.install_layout("layout.is", "Icelandic", &["is"]);
        h.connect(1);
        h.register("org.example.hangul", &info(&[("two", "2-Set")]), &["ko"], Some(Box::new(HangulEngine))).unwrap();
        h.enable_source("org.example.hangul.two").unwrap();
        h.select_source("org.example.hangul.two").unwrap();
        let mut m = Mock::default();
        type_text(&mut h, &mut m, "rk");
        assert_eq!(m.text, "가");
        assert!(h.candidate_view(1).is_none());
        type_text(&mut h, &mut m, "s");
        assert_eq!(m.text, "간");
        assert_eq!(h.original_string(1).as_deref(), Some("rks"));
    }

    #[test]
    fn the_candidate_window_is_placed_beside_the_caret() {
        let mut h = host();
        let mut m = Mock::default();
        assert!(h.candidate_origin(1, (10.0, 10.0, 2.0, 16.0), 100.0, 60.0, 800.0, 600.0, 4.0).is_some());
        type_text(&mut h, &mut m, "ka");
        let (x, y) = h.candidate_origin(1, (10.0, 10.0, 2.0, 16.0), 100.0, 60.0, 800.0, 600.0, 4.0).unwrap();
        assert!(x >= 0.0 && x + 100.0 <= 800.0);
        assert!(y >= 10.0 + 16.0, "below the caret when it fits");
        let (_, flipped) = h.candidate_origin(1, (10.0, 580.0, 2.0, 16.0), 100.0, 60.0, 800.0, 600.0, 4.0).unwrap();
        assert!(flipped + 60.0 <= 580.0, "above the caret near the bottom");
    }
}
