//! Text input client, shell side (`NativeTextInputContext`, `zwp_text_input_v3` semantics): what the focused app's
//! `TextState` / `CaretRect` / `InputTraits` mean for the keyboard, how the keyboard app's composition becomes
//! `SetMarkedText` / `UnmarkText`, and where a candidate popup goes. Pure functions; `apphost.rs` wires them.
//!
//! The shell is the Wayland input method of the window server (`zwp_input_method_v2`): the focused Wayland client's
//! `surrounding_text` and `content_type` arrive as an `ImBatch` at each `done`, and the keyboard's output leaves as
//! `ImOut` requests: typed text is `commit_string`, the keyboard's composition is `set_preedit_string`, a backspace
//! is `delete_surrounding_text`, and named keys go through the virtual keyboard as evdev codes. The Wayland layer
//! (`bare/wayland.rs`) owns the protocol objects; everything here is pure. Native apps keep the `ToApp` path above:
//! `preedit_string` is `Composition::Preedit` (cursor_begin/end are `sel_start`/`sel_len`), `commit_string` is a
//! `KeyInput::Text`, and `delete_surrounding_text` is `ToApp::DeleteSurrounding`.
use super::keyboard::{Composition, Out};
use bifrost_proto::{InputTraits, ReturnKey, TextContentKind, ToApp, WindowInput};

/// The keyboard app's return key label for a field: the nearest of its six (`ReturnKeyType::basic`).
pub fn return_key(t: InputTraits) -> ReturnKey {
    t.return_key.basic()
}

/// The input for a composition event from the keyboard app, for the app's key window (`AppHost::input`).
pub fn to_input(c: Composition) -> WindowInput {
    match c {
        Composition::Preedit { text, sel_start, sel_len } => WindowInput::SetMarkedText { text, sel_start, sel_len },
        Composition::Unmark { commit } => WindowInput::UnmarkText { commit },
    }
}

/// The app message for a composition event from the keyboard app, as the main window gets it.
pub fn to_app(c: Composition) -> ToApp {
    to_input(c).to_app(0)
}

/// Top-left of a popup `w` x `h` (candidate window, correction bubble) for the caret rectangle `caret` in a screen of
/// `screen_w` x `screen_h`: just below the caret, flipped above when it would not fit, and kept inside the screen
/// horizontally. All values in px.
pub fn popup_origin(caret: (f32, f32, f32, f32), w: f32, h: f32, screen_w: f32, screen_h: f32, gap: f32) -> (f32, f32) {
    let (cx, cy, _, ch) = caret;
    let below = cy + ch + gap;
    let y = if below + h <= screen_h { below } else { (cy - gap - h).max(0.0) };
    let x = cx.min(screen_w - w).max(0.0);
    (x, y)
}

/// evdev codes of the named keys the keyboard sends, as the virtual keyboard's keymap names them.
pub const KEY_ESC: u32 = 1;
pub const KEY_BACKSPACE: u32 = 14;
pub const KEY_TAB: u32 = 15;
pub const KEY_ENTER: u32 = 28;

/// One `done`-delimited batch of input-method events. The compositor applies a batch at once, so a batch carries
/// what changed since the last one: a focus change, the field's text, its content purpose.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImBatch {
    /// `activate` (`Some(true)`) or `deactivate` (`Some(false)`) in this batch
    pub focus: Option<bool>,
    /// `surrounding_text`: the text, the caret and the other end of the selection (bytes)
    pub surrounding: Option<(String, u32, u32)>,
    /// `content_type`: the content purpose (`zwp_text_input_v3` numbering)
    pub purpose: Option<u32>,
}

/// The focused Wayland client's text field as the keyboard sees it, kept across batches (the protocol sends a
/// field's state only when it changes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImField {
    /// a text field of a Wayland client is focused and the input method is active
    pub active: bool,
    pub text: String,
    pub cursor: u32,
    pub anchor: u32,
    pub purpose: u32,
    /// the marked text the keyboard set last (`set_preedit_string`); empty when there is none
    pub preedit: String,
}

/// What the input method sends to the window server: a request per entry, `Key` as a press and a release of an
/// evdev code through the virtual keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImOut {
    Commit(String),
    Preedit { text: String, begin: i32, end: i32 },
    Delete { before: u32, after: u32 },
    Key(u32),
}

impl ImField {
    /// Applies one batch and returns the focus change it carried. A deactivation forgets the field.
    pub fn apply(&mut self, b: ImBatch) -> Option<bool> {
        match b.focus {
            Some(true) => self.active = true,
            Some(false) => *self = ImField::default(),
            None => {}
        }
        if let Some((text, cursor, anchor)) = b.surrounding {
            self.text = text;
            self.cursor = cursor;
            self.anchor = anchor;
        }
        if let Some(purpose) = b.purpose {
            self.purpose = purpose;
        }
        b.focus
    }

    /// The field's traits for the keyboard: the content kind from the purpose. The protocol has no return key
    /// type, so the return key stays the default.
    pub fn traits(&self) -> InputTraits {
        InputTraits { content: content_kind(self.purpose), ..InputTraits::default() }
    }
}

/// `zwp_text_input_v3` content purpose to the keyboard's content kind. Date, time and terminal fields use the
/// default layout.
pub fn content_kind(purpose: u32) -> TextContentKind {
    match purpose {
        1 => TextContentKind::Ascii,
        2 | 3 | 9 => TextContentKind::Number,
        4 => TextContentKind::Phone,
        5 => TextContentKind::Url,
        6 => TextContentKind::Email,
        8 => TextContentKind::Password,
        _ => TextContentKind::Default,
    }
}

/// Bytes of the character before the caret in `text`; 0 at the start, or when the caret is not on a character
/// boundary of the text.
pub fn char_before(text: &str, cursor: u32) -> u32 {
    text.get(..cursor as usize).and_then(|s| s.chars().next_back()).map_or(0, |c| c.len_utf8() as u32)
}

/// The evdev code of a named key the keyboard sends. Chords (Paste, Undo, Redo) have no code here.
pub fn named_key(name: &str) -> Option<u32> {
    Some(match name {
        "Escape" => KEY_ESC,
        "Tab" => KEY_TAB,
        "Up" => 103,
        "Left" => 105,
        "Right" => 106,
        "Down" => 108,
        _ => return None,
    })
}

/// The named keys the virtual keyboard sends: evdev code, xkb key name and keysym. Matches `named_key`.
const VIRTUAL_KEYS: [(u32, &str, &str); 8] = [
    (KEY_ESC, "ESC", "Escape"),
    (KEY_BACKSPACE, "BKSP", "BackSpace"),
    (KEY_TAB, "TAB", "Tab"),
    (KEY_ENTER, "RTRN", "Return"),
    (103, "UP", "Up"),
    (105, "LEFT", "Left"),
    (106, "RGHT", "Right"),
    (108, "DOWN", "Down"),
];

/// The xkb keymap the virtual keyboard is given: the named keys only, since text goes through `commit_string`. An
/// xkb keycode is the evdev code plus 8.
pub fn virtual_keymap() -> String {
    let mut s = String::from("xkb_keymap {\nxkb_keycodes \"bifrost\" {\nminimum = 8;\nmaximum = 255;\n");
    for (code, name, _) in VIRTUAL_KEYS {
        s += &format!("<{name}> = {};\n", code + 8);
    }
    s += "};\nxkb_types \"bifrost\" { include \"complete\" };\nxkb_compatibility \"bifrost\" { include \"complete\" };\nxkb_symbols \"bifrost\" {\n";
    for (_, name, sym) in VIRTUAL_KEYS {
        s += &format!("key <{name}> {{ [ {sym} ] }};\n");
    }
    s += "};\n};\n";
    s
}

/// A backspace: delete the selection when there is one, else the character before the caret when the field's text
/// is known, else a real Backspace key (the text is not known, so the byte count would be a guess).
fn backspace(f: &ImField) -> ImOut {
    let lo = f.cursor.min(f.anchor);
    let hi = f.cursor.max(f.anchor);
    if lo != hi {
        return ImOut::Delete { before: f.cursor - lo, after: hi - f.cursor };
    }
    match char_before(&f.text, f.cursor) {
        0 => ImOut::Key(KEY_BACKSPACE),
        before => ImOut::Delete { before, after: 0 },
    }
}

/// The requests for one output of the keyboard. Hide produces none (the keyboard hides itself).
pub fn outs_for(f: &ImField, out: &Out) -> Vec<ImOut> {
    match out {
        Out::Hide => Vec::new(),
        Out::Text(s) => vec![ImOut::Commit(s.clone())],
        Out::Backspace => vec![backspace(f)],
        Out::Enter => vec![ImOut::Key(KEY_ENTER)],
        Out::Named(name) => named_key(name).map(ImOut::Key).into_iter().collect(),
    }
}

/// The requests for the keyboard's composition: the preedit is replaced, and an unmark with `commit` turns it into
/// committed text. `f` keeps the current preedit for that.
pub fn composition(f: &mut ImField, c: &Composition) -> Vec<ImOut> {
    match c {
        Composition::Preedit { text, sel_start, sel_len } => {
            f.preedit = text.clone();
            vec![ImOut::Preedit { text: text.clone(), begin: *sel_start as i32, end: sel_start.saturating_add(*sel_len) as i32 }]
        }
        Composition::Unmark { commit } => {
            let text = std::mem::take(&mut f.preedit);
            let mut out = vec![ImOut::Preedit { text: String::new(), begin: 0, end: 0 }];
            if *commit && !text.is_empty() {
                out.push(ImOut::Commit(text));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::ReturnKeyType;

    #[test]
    fn traits_choose_the_nearest_return_key() {
        let t = |r| InputTraits { return_key: r, ..InputTraits::default() };
        assert_eq!(return_key(t(ReturnKeyType::Search)), ReturnKey::Search);
        assert_eq!(return_key(t(ReturnKeyType::Continue)), ReturnKey::Next);
        assert_eq!(return_key(InputTraits::default()), ReturnKey::Return);
    }

    #[test]
    fn keyboard_composition_becomes_marked_text_messages() {
        assert_eq!(
            to_app(Composition::Preedit { text: "にほ".into(), sel_start: 0, sel_len: 6 }),
            ToApp::SetMarkedText { text: "にほ".into(), sel_start: 0, sel_len: 6 }
        );
        assert_eq!(to_app(Composition::Unmark { commit: true }), ToApp::UnmarkText { commit: true });
    }

    #[test]
    fn a_popup_sits_below_the_caret_flips_above_and_stays_on_screen() {
        let caret = (100.0, 200.0, 1.0, 20.0);
        assert_eq!(popup_origin(caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 224.0));
        // not enough room below: above the caret
        assert_eq!(popup_origin((100.0, 560.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 476.0));
        // near the right edge: pushed left
        assert_eq!(popup_origin((780.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0).0, 650.0);
    }
}

#[cfg(test)]
mod input_method_tests {
    use super::*;

    fn field(text: &str, cursor: u32, anchor: u32) -> ImField {
        ImField { active: true, text: text.into(), cursor, anchor, ..ImField::default() }
    }

    #[test]
    fn content_purpose_picks_the_keyboard_kind() {
        assert_eq!(content_kind(0), TextContentKind::Default);
        assert_eq!(content_kind(1), TextContentKind::Ascii);
        assert_eq!(content_kind(2), TextContentKind::Number);
        assert_eq!(content_kind(9), TextContentKind::Number);
        assert_eq!(content_kind(5), TextContentKind::Url);
        assert_eq!(content_kind(6), TextContentKind::Email);
        assert_eq!(content_kind(8), TextContentKind::Password);
        assert_eq!(content_kind(99), TextContentKind::Default);
        let traits = ImField { purpose: 6, ..ImField::default() }.traits();
        assert_eq!(traits.content, TextContentKind::Email);
    }

    #[test]
    fn focus_batches_raise_and_forget_the_field() {
        let mut f = ImField::default();
        let focus = f.apply(ImBatch { focus: Some(true), surrounding: Some(("ab".into(), 2, 2)), purpose: Some(8) });
        assert_eq!(focus, Some(true));
        assert!(f.active);
        assert_eq!((f.text.as_str(), f.cursor, f.purpose), ("ab", 2, 8));
        // a batch without a focus change keeps the field and reports no change
        assert_eq!(f.apply(ImBatch { surrounding: Some(("abc".into(), 3, 3)), ..ImBatch::default() }), None);
        assert_eq!(f.text, "abc");
        assert_eq!(f.apply(ImBatch { focus: Some(false), ..ImBatch::default() }), Some(false));
        assert_eq!(f, ImField::default());
    }

    #[test]
    fn typed_text_is_committed() {
        let f = field("ab", 2, 2);
        assert_eq!(outs_for(&f, &Out::Text("é".into())), vec![ImOut::Commit("é".into())]);
        assert!(outs_for(&f, &Out::Hide).is_empty());
        assert_eq!(outs_for(&f, &Out::Enter), vec![ImOut::Key(KEY_ENTER)]);
        assert_eq!(outs_for(&f, &Out::Named("Up")), vec![ImOut::Key(103)]);
        assert!(outs_for(&f, &Out::Named("Paste")).is_empty());
    }

    #[test]
    fn backspace_deletes_the_character_before_the_caret() {
        assert_eq!(char_before("aé", 3), 2);
        assert_eq!(char_before("aé", 2), 0, "a caret inside a character is not a boundary");
        assert_eq!(char_before("abc", 0), 0);
        assert_eq!(outs_for(&field("ab", 2, 2), &Out::Backspace), vec![ImOut::Delete { before: 1, after: 0 }]);
        assert_eq!(outs_for(&field("aé", 3, 3), &Out::Backspace), vec![ImOut::Delete { before: 2, after: 0 }]);
    }

    #[test]
    fn backspace_over_a_selection_deletes_the_selection() {
        // "abcd" with the selection bytes 1..3, caret at the start and at the end
        assert_eq!(outs_for(&field("abcd", 1, 3), &Out::Backspace), vec![ImOut::Delete { before: 0, after: 2 }]);
        assert_eq!(outs_for(&field("abcd", 3, 1), &Out::Backspace), vec![ImOut::Delete { before: 2, after: 0 }]);
    }

    #[test]
    fn backspace_without_known_text_is_a_key() {
        assert_eq!(outs_for(&field("", 0, 0), &Out::Backspace), vec![ImOut::Key(KEY_BACKSPACE)]);
    }

    #[test]
    fn the_virtual_keymap_names_every_key_the_shell_sends() {
        let map = virtual_keymap();
        for name in ["Escape", "Tab", "Up", "Down", "Left", "Right"] {
            let code = named_key(name).expect("a named key");
            assert!(map.contains(&format!("= {};", code + 8)), "{name} has keycode {}", code + 8);
        }
        for code in [KEY_BACKSPACE, KEY_ENTER] {
            assert!(VIRTUAL_KEYS.iter().any(|&(c, _, _)| c == code), "{code} is in the keymap");
        }
        assert!(map.contains("key <RTRN> { [ Return ] };"));
        assert!(map.starts_with("xkb_keymap {"));
    }

    #[test]
    fn composition_replaces_the_preedit_and_commits_on_unmark() {
        let mut f = field("", 0, 0);
        assert_eq!(
            composition(&mut f, &Composition::Preedit { text: "にほ".into(), sel_start: 0, sel_len: 6 }),
            vec![ImOut::Preedit { text: "にほ".into(), begin: 0, end: 6 }]
        );
        assert_eq!(f.preedit, "にほ");
        assert_eq!(
            composition(&mut f, &Composition::Unmark { commit: true }),
            vec![ImOut::Preedit { text: String::new(), begin: 0, end: 0 }, ImOut::Commit("にほ".into())]
        );
        assert!(f.preedit.is_empty());
        composition(&mut f, &Composition::Preedit { text: "x".into(), sel_start: 1, sel_len: 0 });
        assert_eq!(
            composition(&mut f, &Composition::Unmark { commit: false }),
            vec![ImOut::Preedit { text: String::new(), begin: 0, end: 0 }]
        );
    }
}
