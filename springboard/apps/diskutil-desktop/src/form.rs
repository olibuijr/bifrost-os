//! The text fields of a sheet. The sheet kit draws a title, a message and buttons only, so the sheets that ask for a
//! name or a size carry a `Form`: one or more single-line fields, the keyboard focus among them and the last error.
//! The window paints the fields over the sheet and sends it the keys; this module holds the editing rules.

use bifrost_app::KeyInput;

/// The longest text a field takes (characters).
pub const MAX_LEN: usize = 64;

/// One single-line field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub text: String,
    /// Shown in grey while the field is empty.
    pub hint: String,
}

impl Field {
    pub fn new(label: &str, text: &str, hint: &str) -> Field {
        Field { label: label.to_string(), text: text.to_string(), hint: hint.to_string() }
    }
}

/// What a key did to the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKey {
    /// Return: the user wants to confirm.
    Confirm,
    /// Escape: the user wants to cancel.
    Cancel,
    /// The text or the focus changed.
    Edited,
    /// The key means nothing to the form.
    Ignored,
}

/// The fields of a sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    /// The index of the field that takes typing.
    pub focus: usize,
    /// Why the last confirm was refused; cleared by the next edit.
    pub error: Option<String>,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Form {
        Form { fields, focus: 0, error: None }
    }

    /// The trimmed text of field `index` (empty when there is no such field).
    pub fn value(&self, index: usize) -> String {
        self.fields.get(index).map(|f| f.text.trim().to_string()).unwrap_or_default()
    }

    /// Applies a key: text is added at the end of the focused field, Backspace removes the last character, Tab moves to
    /// the next field, Return confirms and Escape cancels.
    pub fn key(&mut self, key: &KeyInput) -> FormKey {
        match key {
            KeyInput::Text(text) => {
                let Some(field) = self.fields.get_mut(self.focus) else { return FormKey::Ignored };
                let mut changed = false;
                for ch in text.chars().filter(|c| !c.is_control()) {
                    if field.text.chars().count() < MAX_LEN {
                        field.text.push(ch);
                        changed = true;
                    }
                }
                self.finish_edit(changed)
            }
            KeyInput::Code(8) => {
                let popped = self.fields.get_mut(self.focus).and_then(|f| f.text.pop()).is_some();
                self.finish_edit(popped)
            }
            KeyInput::Code(9) => {
                if self.fields.len() > 1 {
                    self.focus = (self.focus + 1) % self.fields.len();
                    FormKey::Edited
                } else {
                    FormKey::Ignored
                }
            }
            KeyInput::Code(13) => FormKey::Confirm,
            KeyInput::Code(1) | KeyInput::Code(27) => FormKey::Cancel,
            KeyInput::Code(_) => FormKey::Ignored,
        }
    }

    fn finish_edit(&mut self, changed: bool) -> FormKey {
        if changed {
            self.error = None;
            FormKey::Edited
        } else {
            FormKey::Ignored
        }
    }

    /// Moves the focus to field `index`, when there is one.
    pub fn set_focus(&mut self, index: usize) -> bool {
        if index < self.fields.len() && index != self.focus {
            self.focus = index;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Form {
        Form::new(vec![Field::new("Name", "", "Untitled"), Field::new("Size", "1 GB", "")])
    }

    #[test]
    fn typing_appends_and_backspace_removes() {
        let mut f = two();
        assert_eq!(f.key(&KeyInput::Text("Ab".into())), FormKey::Edited);
        assert_eq!(f.key(&KeyInput::Text("c".into())), FormKey::Edited);
        assert_eq!(f.fields[0].text, "Abc");
        assert_eq!(f.key(&KeyInput::Code(8)), FormKey::Edited);
        assert_eq!(f.fields[0].text, "Ab");
        f.fields[0].text.clear();
        assert_eq!(f.key(&KeyInput::Code(8)), FormKey::Ignored);
    }

    #[test]
    fn tab_moves_the_focus_and_typing_follows_it() {
        let mut f = two();
        assert_eq!(f.key(&KeyInput::Code(9)), FormKey::Edited);
        f.key(&KeyInput::Text("0".into()));
        assert_eq!(f.fields[1].text, "1 GB0");
        assert_eq!(f.key(&KeyInput::Code(9)), FormKey::Edited);
        assert_eq!(f.focus, 0);
        let mut one = Form::new(vec![Field::new("Name", "", "")]);
        assert_eq!(one.key(&KeyInput::Code(9)), FormKey::Ignored);
    }

    #[test]
    fn return_confirms_and_escape_cancels() {
        let mut f = two();
        assert_eq!(f.key(&KeyInput::Code(13)), FormKey::Confirm);
        assert_eq!(f.key(&KeyInput::Code(27)), FormKey::Cancel);
        assert_eq!(f.key(&KeyInput::Code(1)), FormKey::Cancel);
        assert_eq!(f.key(&KeyInput::Code(40)), FormKey::Ignored);
    }

    #[test]
    fn control_characters_are_dropped_and_the_length_is_capped() {
        let mut f = two();
        assert_eq!(f.key(&KeyInput::Text("\n\t".into())), FormKey::Ignored);
        f.key(&KeyInput::Text("x".repeat(MAX_LEN + 10)));
        assert_eq!(f.fields[0].text.chars().count(), MAX_LEN);
    }

    #[test]
    fn an_edit_clears_the_error_and_values_are_trimmed() {
        let mut f = two();
        f.error = Some("Bad".into());
        f.key(&KeyInput::Text("  hi ".into()));
        assert!(f.error.is_none());
        assert_eq!(f.value(0), "hi");
        assert_eq!(f.value(5), "");
        assert!(f.set_focus(1) && !f.set_focus(1) && !f.set_focus(9));
    }
}
