//! The input method side of text input: the registration record an input method ships with, one controller per
//! connected client input session, the composition a controller owns (the keystrokes as typed, the converted text
//! shown as marked text, its caret and the candidates), the routing of key, command and mouse events, and the server
//! that holds the connections.
//!
//! Pure model: no socket, no compositor protocol, no drawing. The shell connects clients and forwards their events;
//! the client keeps its own text and applies what [`Outcome`] reports. The candidates list is
//! [`crate::candidates::Candidates`]. Offsets are UTF-8 byte offsets in the composed text (`caret`) and in the client
//! text (`Composition::start`), as in `text.rs`.
//!
//! The conversion engine is outside this model. Without one, a text key during a composition keeps the composed text
//! equal to the original keystrokes (identity conversion); an engine replaces it with [`InputController::set_composition`].
//! With no composition, every event passes through to the client.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::candidates::{Candidates, PanelType};
use crate::keys::{Key, KeyCombo};
use crate::text::{floor_boundary, EditCommand};

/// One input mode an input method offers, such as a script or a layout. `id` is what [`State::set_mode`] takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    pub id: String,
    pub label: String,
}

/// Why a registration record is not usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    /// The controller class is missing or blank.
    NoController,
    /// No input mode is offered.
    NoModes,
    /// A mode has a blank id.
    EmptyMode,
    /// Two modes share the id.
    DuplicateMode(String),
}

/// The registration record of an input method: the controller class the server creates for each session, an
/// optional delegate class and the input modes it offers. Checked by [`Registration::validate`] before a server starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    controller: String,
    delegate: Option<String>,
    modes: Vec<Mode>,
}

impl Registration {
    pub fn new(controller: &str) -> Self {
        Registration { controller: controller.trim().to_string(), delegate: None, modes: Vec::new() }
    }

    /// Sets the delegate class. A blank name leaves the input method without a delegate.
    pub fn delegate(mut self, class: &str) -> Self {
        let class = class.trim();
        self.delegate = if class.is_empty() { None } else { Some(class.to_string()) };
        self
    }

    /// Adds an input mode. Modes keep the order they were added in; the first one is the default.
    pub fn mode(mut self, id: &str, label: &str) -> Self {
        self.modes.push(Mode { id: id.to_string(), label: label.to_string() });
        self
    }

    /// Checks the record: a controller class, at least one mode, and modes with distinct non-blank ids.
    pub fn validate(&self) -> Result<(), RegistrationError> {
        if self.controller.is_empty() {
            return Err(RegistrationError::NoController);
        }
        if self.modes.is_empty() {
            return Err(RegistrationError::NoModes);
        }
        for (i, mode) in self.modes.iter().enumerate() {
            if mode.id.trim().is_empty() {
                return Err(RegistrationError::EmptyMode);
            }
            if self.modes[..i].iter().any(|m| m.id == mode.id) {
                return Err(RegistrationError::DuplicateMode(mode.id.clone()));
            }
        }
        Ok(())
    }

    pub fn controller_class(&self) -> &str {
        &self.controller
    }

    pub fn delegate_class(&self) -> Option<&str> {
        self.delegate.as_deref()
    }

    pub fn modes(&self) -> &[Mode] {
        &self.modes
    }

    /// The first mode, which a new session starts in.
    pub fn default_mode(&self) -> Option<&Mode> {
        self.modes.first()
    }

    pub fn has_mode(&self, id: &str) -> bool {
        self.modes.iter().any(|m| m.id == id)
    }
}

/// Why a state change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// The id is not among the registered modes.
    UnknownMode(String),
}

/// The state a session reports: the active input mode and whether the session is active. A session that is not
/// active passes every event to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    mode: String,
    active: bool,
}

impl State {
    pub fn mode(&self) -> &str {
        &self.mode
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// The composition: the keystrokes as typed (`original`), the converted text shown in the client as marked text
/// (`composed`), the caret inside `composed`, and where the marked text starts in the client's text (`start`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    original: String,
    composed: String,
    caret: usize,
    start: usize,
}

impl Composition {
    /// The byte range of the marked text in the client's text.
    pub fn marked_range(&self) -> Range<usize> {
        self.start..self.start + self.composed.len()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }
}

/// A mouse phase, as the client reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Drag,
    Up,
}

/// An event the client forwards to the input method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key with the text it produced (empty when the key makes no text).
    Key { text: String, combo: KeyCombo },
    /// A command generated by an action such as a menu item or a key binding.
    Command(EditCommand),
    /// A pointer event at byte offset `at` in the client's text.
    Mouse { kind: MouseKind, at: usize },
}

/// What the client does after the input method has seen an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The input method used the event. The client does nothing more.
    Consumed,
    /// The event is the client's. The client handles it as usual.
    PassThrough,
    /// The composition was committed. The client replaces its marked text with this text. For a command or a click
    /// that ended the composition, the client then performs that command or places its caret.
    Committed(String),
    /// The composition was cancelled. The client replaces its marked text with this original text.
    Cancelled(String),
}

/// One input session: the controller a server creates for a connected client. It holds the state, the composition
/// and the candidates for that client only.
#[derive(Clone, Debug, PartialEq)]
pub struct InputController {
    client: u32,
    state: State,
    modes: Vec<String>,
    composition: Option<Composition>,
    candidates: Candidates,
}

impl InputController {
    /// A new session for `client`, starting active in the registration's default mode. `None` when the registration
    /// offers no mode.
    pub fn new(client: u32, registration: &Registration) -> Option<Self> {
        let first = registration.default_mode()?.id.clone();
        Some(InputController {
            client,
            state: State { mode: first, active: false },
            modes: registration.modes().iter().map(|m| m.id.clone()).collect(),
            composition: None,
            candidates: Candidates::new(PanelType::SingleColumnScrolling),
        })
    }

    pub fn client(&self) -> u32 {
        self.client
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Switches the input mode. Only a registered mode is accepted; the composition is committed first, because a
    /// mode change ends the conversion in progress. Returns the committed text, if there was a composition.
    pub fn set_mode(&mut self, id: &str) -> Result<Option<String>, StateError> {
        if !self.modes.iter().any(|m| m == id) {
            return Err(StateError::UnknownMode(id.to_string()));
        }
        let committed = self.commit_composition();
        self.state.mode = id.to_string();
        Ok(committed)
    }

    /// Makes the session active, so it receives events.
    pub fn activate(&mut self) {
        self.state.active = true;
    }

    /// Makes the session inactive. A pending composition is committed and its text returned.
    pub fn deactivate(&mut self) -> Option<String> {
        let committed = self.commit_composition();
        self.state.active = false;
        self.candidates.hide();
        committed
    }

    /// Sets the composition from the conversion engine: the original keystrokes, the converted text, the caret in
    /// `composed` and the client offset `start` where the marked text begins. The caret is clamped into `composed`
    /// and onto a character boundary. Starts a composition when none is open.
    pub fn set_composition(&mut self, start: usize, original: &str, composed: &str, caret: usize) {
        let caret = floor_boundary(composed, caret.min(composed.len()));
        self.composition = Some(Composition {
            original: original.to_string(),
            composed: composed.to_string(),
            caret,
            start,
        });
    }

    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// The keystrokes as typed (`originalString`).
    pub fn original_string(&self) -> Option<&str> {
        self.composition.as_ref().map(|c| c.original.as_str())
    }

    /// The converted text (`composedString`).
    pub fn composed_string(&self) -> Option<&str> {
        self.composition.as_ref().map(|c| c.composed.as_str())
    }

    pub fn candidates(&self) -> &Candidates {
        &self.candidates
    }

    /// Replaces the candidate list and shows the window. Each item is a candidate string.
    pub fn offer_candidates(&mut self, items: Vec<String>) {
        self.candidates.set_items(items);
        self.candidates.show(crate::candidates::LocationHint::Below);
    }

    /// Commits the composition: returns its converted text and clears it, with the candidates. `None` when no
    /// composition is open.
    pub fn commit_composition(&mut self) -> Option<String> {
        let c = self.composition.take()?;
        self.candidates.set_items(Vec::new());
        self.candidates.hide();
        Some(c.composed)
    }

    /// Cancels the composition: returns the original keystrokes, which replace the marked text, and clears it.
    pub fn cancel_composition(&mut self) -> Option<String> {
        let c = self.composition.take()?;
        self.candidates.set_items(Vec::new());
        self.candidates.hide();
        Some(c.original)
    }

    /// Routes one event. Inactive sessions and events with no composition open pass through, except that a
    /// composition is only started by the engine (see [`InputController::set_composition`]).
    pub fn handle(&mut self, event: &Event) -> Outcome {
        if !self.state.active || self.composition.is_none() {
            return Outcome::PassThrough;
        }
        match event {
            Event::Mouse { kind, at } => self.mouse(*kind, *at),
            Event::Command(cmd) => self.command(*cmd),
            Event::Key { text, combo } => self.key(text, combo),
        }
    }

    fn key(&mut self, text: &str, combo: &KeyCombo) -> Outcome {
        let plain = !(combo.cmd || combo.ctrl || combo.alt);
        match combo.key {
            Key::Return if plain => {
                let composed = self.commit_composition().unwrap_or_default();
                Outcome::Committed(composed)
            }
            Key::Escape if plain => Outcome::Cancelled(self.cancel_composition().unwrap_or_default()),
            Key::Char(d @ '1'..='9') if plain && self.candidates.is_visible() => {
                // a digit key is ASCII, so the offset from '1' is the candidate's index
                let index = (d as u8 - b'1') as usize;
                if index < self.candidates.items().len() {
                    let picked = self.candidates.items()[index].clone();
                    // a chosen candidate is final, so it is committed at once
                    self.composition = None;
                    self.candidates.hide();
                    Outcome::Committed(picked)
                } else {
                    Outcome::Consumed
                }
            }
            Key::Up | Key::Down if plain && self.candidates.is_visible() => {
                let len = self.candidates.items().len();
                if len > 0 {
                    let next = match (self.candidates.selected(), combo.key) {
                        (Some(i), Key::Down) => (i + 1).min(len - 1),
                        (Some(i), _) => i.saturating_sub(1),
                        (None, _) => 0,
                    };
                    self.candidates.select(next);
                }
                Outcome::Consumed
            }
            Key::Left if plain => {
                self.move_caret(false);
                Outcome::Consumed
            }
            Key::Right if plain => {
                self.move_caret(true);
                Outcome::Consumed
            }
            Key::Delete if plain => self.delete_backward(),
            _ if plain && !text.is_empty() && text.chars().all(|c| !c.is_control()) => {
                // identity conversion: typed text joins the original keystrokes and is shown as it was typed
                if let Some(c) = self.composition.as_mut() {
                    c.original.push_str(text);
                    c.composed = c.original.clone();
                    c.caret = c.composed.len();
                }
                Outcome::Consumed
            }
            _ => Outcome::PassThrough,
        }
    }

    fn command(&mut self, cmd: EditCommand) -> Outcome {
        match cmd {
            EditCommand::MoveLeft => {
                self.move_caret(false);
                Outcome::Consumed
            }
            EditCommand::MoveRight => {
                self.move_caret(true);
                Outcome::Consumed
            }
            EditCommand::DeleteBackward => self.delete_backward(),
            // any other command ends the conversion; the client inserts the committed text, then runs the command
            _ => Outcome::Committed(self.commit_composition().unwrap_or_default()),
        }
    }

    fn mouse(&mut self, kind: MouseKind, at: usize) -> Outcome {
        let (start, end) = match self.composition.as_ref() {
            Some(c) => (c.start, c.start + c.composed.len()),
            None => return Outcome::PassThrough,
        };
        let inside = at >= start && at <= end;
        match kind {
            MouseKind::Down if inside => {
                if let Some(c) = self.composition.as_mut() {
                    let rel = floor_boundary(&c.composed, at - start);
                    c.caret = rel;
                }
                Outcome::Consumed
            }
            MouseKind::Drag | MouseKind::Up if inside => Outcome::Consumed,
            MouseKind::Down => Outcome::Committed(self.commit_composition().unwrap_or_default()),
            _ => Outcome::PassThrough,
        }
    }

    /// Moves the caret one character forward or back inside the composed text.
    fn move_caret(&mut self, forward: bool) {
        if let Some(c) = self.composition.as_mut() {
            if forward {
                if let Some(ch) = c.composed[c.caret..].chars().next() {
                    c.caret += ch.len_utf8();
                }
            } else if let Some(ch) = c.composed[..c.caret].chars().next_back() {
                c.caret -= ch.len_utf8();
            }
        }
    }

    /// Removes the last character of the keystrokes. Cancels the composition when nothing is left.
    fn delete_backward(&mut self) -> Outcome {
        let empty = match self.composition.as_mut() {
            Some(c) => {
                c.original.pop();
                c.composed = c.original.clone();
                c.caret = c.composed.len();
                c.original.is_empty()
            }
            None => return Outcome::PassThrough,
        };
        if empty {
            self.composition = None;
            self.candidates.hide();
            return Outcome::Cancelled(String::new());
        }
        Outcome::Consumed
    }
}

/// Why a connection was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerError {
    /// The registration record failed [`Registration::validate`].
    Invalid(RegistrationError),
    /// The client already has a session.
    AlreadyConnected(u32),
}

/// The server of an input method: it checks the registration once, then holds one controller per client session.
#[derive(Debug)]
pub struct Server {
    registration: Registration,
    sessions: BTreeMap<u32, InputController>,
}

impl Server {
    pub fn new(registration: Registration) -> Result<Self, ServerError> {
        registration.validate().map_err(ServerError::Invalid)?;
        Ok(Server { registration, sessions: BTreeMap::new() })
    }

    pub fn registration(&self) -> &Registration {
        &self.registration
    }

    /// Opens a session for `client` and returns its controller, active and in the default mode.
    pub fn connect(&mut self, client: u32) -> Result<&mut InputController, ServerError> {
        if self.sessions.contains_key(&client) {
            return Err(ServerError::AlreadyConnected(client));
        }
        // validate() passed in new(), so a default mode exists
        let mut controller = InputController::new(client, &self.registration)
            .ok_or(ServerError::Invalid(RegistrationError::NoModes))?;
        controller.activate();
        Ok(self.sessions.entry(client).or_insert(controller))
    }

    /// Closes the session of `client`, deactivating it first. Returns the closed controller, or `None` when the
    /// client had no session. Any pending composition is dropped with the session.
    pub fn disconnect(&mut self, client: u32) -> Option<InputController> {
        let mut controller = self.sessions.remove(&client)?;
        controller.deactivate();
        Some(controller)
    }

    pub fn controller(&self, client: u32) -> Option<&InputController> {
        self.sessions.get(&client)
    }

    pub fn controller_mut(&mut self, client: u32) -> Option<&mut InputController> {
        self.sessions.get_mut(&client)
    }

    /// The connected clients, in ascending order.
    pub fn clients(&self) -> Vec<u32> {
        self.sessions.keys().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Closes every session and returns how many there were.
    pub fn shutdown(&mut self) -> usize {
        let count = self.sessions.len();
        let clients: Vec<u32> = self.clients();
        for client in clients {
            self.disconnect(client);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registration {
        Registration::new("PinyinController").delegate("PinyinDelegate").mode("zh-pinyin", "Pinyin").mode("ja-romaji", "Romaji")
    }

    fn key(c: char) -> Event {
        Event::Key { text: c.to_string(), combo: KeyCombo::new(Key::Char(c)) }
    }

    fn plain(k: Key) -> Event {
        Event::Key { text: String::new(), combo: KeyCombo::new(k) }
    }

    #[test]
    fn registration_validates_class_modes_and_ids() {
        assert_eq!(Registration::new("  ").mode("a", "A").validate(), Err(RegistrationError::NoController));
        assert_eq!(Registration::new("C").validate(), Err(RegistrationError::NoModes));
        assert_eq!(Registration::new("C").mode(" ", "x").validate(), Err(RegistrationError::EmptyMode));
        assert_eq!(
            Registration::new("C").mode("a", "A").mode("a", "again").validate(),
            Err(RegistrationError::DuplicateMode("a".to_string()))
        );
        let ok = reg();
        assert_eq!(ok.validate(), Ok(()));
        assert_eq!(ok.default_mode().map(|m| m.id.as_str()), Some("zh-pinyin"));
        assert_eq!(ok.delegate_class(), Some("PinyinDelegate"));
        assert_eq!(Registration::new("C").delegate("   ").delegate_class(), None);
    }

    #[test]
    fn server_refuses_an_invalid_record_and_a_second_connection() {
        assert_eq!(Server::new(Registration::new("C")).err(), Some(ServerError::Invalid(RegistrationError::NoModes)));
        let mut server = Server::new(reg()).unwrap();
        assert!(server.is_empty());
        server.connect(7).unwrap();
        server.connect(3).unwrap();
        assert_eq!(server.clients(), vec![3, 7]);
        assert_eq!(server.connect(7).err(), Some(ServerError::AlreadyConnected(7)));
        assert_eq!(server.len(), 2);
        assert_eq!(server.shutdown(), 2);
        assert!(server.is_empty());
    }

    #[test]
    fn inactive_or_idle_sessions_pass_every_event_through() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        // active, but no composition is open: the client types as usual
        assert_eq!(session.handle(&key('n')), Outcome::PassThrough);
        session.set_composition(0, "n", "n", 1);
        session.deactivate();
        assert_eq!(session.handle(&key('i')), Outcome::PassThrough);
        assert!(!session.state().is_active());
    }

    #[test]
    fn composition_is_committed_by_return_and_cancelled_by_escape() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        // the engine turns "ni" into the converted text "你", marked from client offset 4
        session.set_composition(4, "ni", "你", 3);
        assert_eq!(session.original_string(), Some("ni"));
        assert_eq!(session.composed_string(), Some("你"));
        assert_eq!(session.composition().unwrap().marked_range(), 4..7);
        assert_eq!(session.handle(&plain(Key::Return)), Outcome::Committed("你".to_string()));
        assert!(session.composition().is_none());

        session.set_composition(0, "hao", "好", 3);
        assert_eq!(session.handle(&plain(Key::Escape)), Outcome::Cancelled("hao".to_string()));
        assert!(session.composed_string().is_none());
    }

    #[test]
    fn typing_during_a_composition_extends_the_keystrokes() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        session.set_composition(0, "n", "n", 1);
        assert_eq!(session.handle(&key('i')), Outcome::Consumed);
        // identity conversion: the typed letters are both the keystrokes and the shown text
        assert_eq!(session.original_string(), Some("ni"));
        assert_eq!(session.composed_string(), Some("ni"));
        assert_eq!(session.composition().unwrap().caret(), 2);
        // Delete takes one keystroke back; emptying the keystrokes cancels the composition
        assert_eq!(session.handle(&plain(Key::Delete)), Outcome::Consumed);
        assert_eq!(session.handle(&plain(Key::Delete)), Outcome::Cancelled(String::new()));
        assert!(session.composition().is_none());
    }

    #[test]
    fn candidates_are_chosen_by_digit_and_moved_by_arrows() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        session.set_composition(0, "ma", "ma", 2);
        session.offer_candidates(vec!["马".into(), "吗".into(), "妈".into()]);
        assert_eq!(session.original_string(), Some("ma"));
        assert_eq!(session.candidates().items().len(), 3);
        // Down selects the first candidate, then the second; Up goes back
        assert_eq!(session.handle(&plain(Key::Down)), Outcome::Consumed);
        assert_eq!(session.handle(&plain(Key::Down)), Outcome::Consumed);
        assert_eq!(session.candidates().selected(), Some(1));
        assert_eq!(session.handle(&plain(Key::Up)), Outcome::Consumed);
        assert_eq!(session.candidates().selected(), Some(0));
        // digit 3 picks the third candidate and commits it at once
        assert_eq!(session.handle(&key('3')), Outcome::Committed("妈".to_string()));
        // a digit past the list is consumed without committing
        session.set_composition(0, "ma", "ma", 2);
        session.offer_candidates(vec!["马".into()]);
        assert_eq!(session.handle(&key('4')), Outcome::Consumed);
        assert!(session.composition().is_some());
    }

    #[test]
    fn a_click_inside_moves_the_caret_and_a_click_outside_commits() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        // marked text at client bytes 10..16 ("你好" is 6 bytes)
        session.set_composition(10, "nihao", "你好", 0);
        assert_eq!(session.handle(&Event::Mouse { kind: MouseKind::Down, at: 13 }), Outcome::Consumed);
        assert_eq!(session.composition().unwrap().caret(), 3);
        assert_eq!(session.handle(&Event::Mouse { kind: MouseKind::Drag, at: 16 }), Outcome::Consumed);
        // a click at byte 2, before the marked text, commits it and lets the client place its caret
        assert_eq!(session.handle(&Event::Mouse { kind: MouseKind::Down, at: 2 }), Outcome::Committed("你好".to_string()));
        assert!(session.composition().is_none());
    }

    #[test]
    fn commands_move_inside_the_composition_and_other_commands_commit_it() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        session.set_composition(0, "ab", "你好", 6);
        // the caret starts at the end; one step back crosses the whole three-byte character
        assert_eq!(session.handle(&Event::Command(EditCommand::MoveLeft)), Outcome::Consumed);
        assert_eq!(session.composition().unwrap().caret(), 3);
        assert_eq!(session.handle(&Event::Command(EditCommand::MoveRight)), Outcome::Consumed);
        assert_eq!(session.composition().unwrap().caret(), 6);
        assert_eq!(session.handle(&Event::Command(EditCommand::MoveDocumentEnd)), Outcome::Committed("你好".to_string()));
    }

    #[test]
    fn switching_mode_commits_and_unknown_modes_are_refused() {
        let mut server = Server::new(reg()).unwrap();
        let session = server.connect(1).unwrap();
        assert_eq!(session.state().mode(), "zh-pinyin");
        session.set_composition(0, "ka", "か", 3);
        assert_eq!(session.set_mode("ja-romaji"), Ok(Some("か".to_string())));
        assert_eq!(session.state().mode(), "ja-romaji");
        assert_eq!(session.set_mode("xx"), Err(StateError::UnknownMode("xx".to_string())));
        assert_eq!(session.state().mode(), "ja-romaji");
    }
}
