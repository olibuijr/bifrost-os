//! PDF form fields (widget annotations) as a pure model: the field kinds (text, choice, button, signature), the control
//! of a button (checkbox, radio button, push button), the flags that make a field read-only, required, multiline, a
//! password, a comb or a combo box, the value and default of each field, and the set of fields in a form. The flag
//! bits are the field flag bits of the PDF format. Nothing here draws a field or reads a file; the view and the
//! document reader use this model for what a field holds.

/// The kinds of form field. The names follow the widget subtypes a PDF annotation reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subtype {
    Text,
    Choice,
    Button,
    Signature,
}

/// The control a button field is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonControl {
    CheckBox,
    Radio,
    PushButton,
}

/// Why a field refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldError {
    /// The field has the read-only flag.
    ReadOnly,
    /// The field kind does not take this kind of change, such as a state on a text field.
    WrongKind,
    /// The text has more characters than the field's length limit.
    TooLong,
    /// A choice field without the combo flag only takes one of its options.
    NotAnOption,
    /// A radio button has the no-toggle-to-off flag and cannot be turned off.
    CannotToggleOff,
    /// A field with this name is already in the form.
    Duplicate,
    /// No field in the form has this name.
    Unknown,
}

// Field flag bits, as the PDF format numbers them (bit position minus one).
const READ_ONLY: u32 = 1 << 0;
const REQUIRED: u32 = 1 << 1;
const MULTILINE: u32 = 1 << 12;
const PASSWORD: u32 = 1 << 13;
const NO_TOGGLE_TO_OFF: u32 = 1 << 14;
const RADIO: u32 = 1 << 15;
const PUSH_BUTTON: u32 = 1 << 16;
const COMBO: u32 = 1 << 17;
const COMB: u32 = 1 << 24;

/// One form field. Text and choice fields hold a string; button fields hold an on or off state.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    name: String,
    subtype: Subtype,
    control: Option<ButtonControl>,
    flags: u32,
    value: String,
    on: bool,
    default_value: String,
    default_on: bool,
    max_len: Option<usize>,
    options: Vec<String>,
}

impl Field {
    fn with_kind(name: &str, subtype: Subtype, control: Option<ButtonControl>, flags: u32, options: Vec<String>) -> Field {
        Field {
            name: name.to_string(),
            subtype,
            control,
            flags,
            value: String::new(),
            on: false,
            default_value: String::new(),
            default_on: false,
            max_len: None,
            options,
        }
    }

    /// A text field, empty and with no length limit.
    pub fn text(name: &str, flags: u32) -> Field {
        Field::with_kind(name, Subtype::Text, None, flags, Vec::new())
    }

    /// A choice field that offers `options`. Its value starts empty.
    pub fn choice(name: &str, options: &[&str], flags: u32) -> Field {
        let options = options.iter().map(|o| o.to_string()).collect();
        Field::with_kind(name, Subtype::Choice, None, flags, options)
    }

    /// A button field, off. The push-button flag makes it a push button, the radio flag a radio button, and
    /// otherwise it is a check box.
    pub fn button(name: &str, flags: u32) -> Field {
        let control = if flags & PUSH_BUTTON != 0 {
            ButtonControl::PushButton
        } else if flags & RADIO != 0 {
            ButtonControl::Radio
        } else {
            ButtonControl::CheckBox
        };
        Field::with_kind(name, Subtype::Button, Some(control), flags, Vec::new())
    }

    /// A signature field. It holds no value that this model sets.
    pub fn signature(name: &str, flags: u32) -> Field {
        Field::with_kind(name, Subtype::Signature, None, flags, Vec::new())
    }

    /// Sets the length limit of a text field, in characters. Zero means no limit, as in the format.
    pub fn limit_length(mut self, max: usize) -> Field {
        self.max_len = if max == 0 { None } else { Some(max) };
        self
    }

    /// Sets the value that `reset` restores: the text, and the on state for a button.
    pub fn with_default(mut self, value: &str, on: bool) -> Field {
        self.default_value = value.to_string();
        self.default_on = on;
        self.value = self.default_value.clone();
        self.on = on;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn subtype(&self) -> Subtype {
        self.subtype
    }

    /// The control of a button field, or `None` for any other kind.
    pub fn control(&self) -> Option<ButtonControl> {
        self.control
    }

    /// The text of a text or choice field. Empty for the other kinds.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The on state of a button field. Always false for the other kinds.
    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_len
    }

    /// The options of a choice field, in the order the form lists them.
    pub fn options(&self) -> &[String] {
        &self.options
    }

    pub fn is_read_only(&self) -> bool {
        self.flags & READ_ONLY != 0
    }

    pub fn is_required(&self) -> bool {
        self.flags & REQUIRED != 0
    }

    pub fn is_multiline(&self) -> bool {
        self.flags & MULTILINE != 0
    }

    pub fn is_password(&self) -> bool {
        self.flags & PASSWORD != 0
    }

    /// Whether the text is spread over equal cells, as in a form entry field.
    pub fn has_comb(&self) -> bool {
        self.flags & COMB != 0
    }

    /// Whether a choice field also takes text that is not one of its options.
    pub fn is_combo(&self) -> bool {
        self.flags & COMBO != 0
    }

    /// Whether a radio button that is on can be turned off by the user. It can unless the no-toggle-to-off flag is set.
    pub fn allows_toggle_to_off(&self) -> bool {
        self.flags & NO_TOGGLE_TO_OFF == 0
    }

    /// Sets the text of a text or choice field. A text field refuses a value longer than its limit, and a choice
    /// field refuses a value that is not one of its options unless it is a combo box.
    pub fn set_value(&mut self, value: &str) -> Result<(), FieldError> {
        if self.is_read_only() {
            return Err(FieldError::ReadOnly);
        }
        match self.subtype {
            Subtype::Text => {
                if let Some(max) = self.max_len {
                    if value.chars().count() > max {
                        return Err(FieldError::TooLong);
                    }
                }
            }
            Subtype::Choice => {
                if !self.is_combo() && !self.options.iter().any(|o| o == value) {
                    return Err(FieldError::NotAnOption);
                }
            }
            Subtype::Button | Subtype::Signature => return Err(FieldError::WrongKind),
        }
        self.value = value.to_string();
        Ok(())
    }

    /// Sets the on state of a check box or radio button. A push button has no state. A radio button that is on
    /// cannot be turned off when it has the no-toggle-to-off flag. Turning a radio button on does not clear the
    /// others in its group; the form does that.
    pub fn set_state(&mut self, on: bool) -> Result<(), FieldError> {
        if self.is_read_only() {
            return Err(FieldError::ReadOnly);
        }
        match self.control {
            Some(ButtonControl::CheckBox) => {}
            Some(ButtonControl::Radio) => {
                if !on && self.on && !self.allows_toggle_to_off() {
                    return Err(FieldError::CannotToggleOff);
                }
            }
            Some(ButtonControl::PushButton) | None => return Err(FieldError::WrongKind),
        }
        self.on = on;
        Ok(())
    }

    /// Whether the field holds what a required field needs: non-empty text, or a button that is on.
    /// A signature field never counts as filled here.
    pub fn is_filled(&self) -> bool {
        match self.subtype {
            Subtype::Text | Subtype::Choice => !self.value.is_empty(),
            Subtype::Button => self.on,
            Subtype::Signature => false,
        }
    }

    /// The value a submit action sends for this field. Text and choice fields always send their text, even when it
    /// is empty. A button sends `on` when it is on and nothing when it is off. A signature sends nothing.
    pub fn export_value(&self) -> Option<String> {
        match self.subtype {
            Subtype::Text | Subtype::Choice => Some(self.value.clone()),
            Subtype::Button => {
                if self.on {
                    Some("on".to_string())
                } else {
                    None
                }
            }
            Subtype::Signature => None,
        }
    }

    /// Restores the value and on state that the field was given with `with_default`.
    pub fn reset(&mut self) {
        self.value = self.default_value.clone();
        self.on = self.default_on;
    }
}

/// The fields of one form, in the order they were added. Names are unique within a form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldSet {
    fields: Vec<Field>,
}

impl FieldSet {
    pub fn new() -> FieldSet {
        FieldSet { fields: Vec::new() }
    }

    /// Adds a field at the end. Refuses a name that is already in the form.
    pub fn add(&mut self, field: Field) -> Result<(), FieldError> {
        if self.position(field.name()).is_some() {
            return Err(FieldError::Duplicate);
        }
        self.fields.push(field);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Field> {
        self.position(name).map(|i| &self.fields[i])
    }

    /// All fields, in the order they were added.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Sets the text of the field called `name`. Refuses an unknown name, and the refusals of `Field::set_value`.
    pub fn set_value(&mut self, name: &str, value: &str) -> Result<(), FieldError> {
        let i = self.position(name).ok_or(FieldError::Unknown)?;
        self.fields[i].set_value(value)
    }

    /// Sets the on state of the field called `name`. Refuses an unknown name, and the refusals of `Field::set_state`.
    pub fn set_state(&mut self, name: &str, on: bool) -> Result<(), FieldError> {
        let i = self.position(name).ok_or(FieldError::Unknown)?;
        self.fields[i].set_state(on)
    }

    /// The names of the required fields that are not filled, in form order.
    pub fn missing_required(&self) -> Vec<&str> {
        self.fields.iter().filter(|f| f.is_required() && !f.is_filled()).map(|f| f.name()).collect()
    }

    /// The name and value of each field that a submit action sends, in form order.
    pub fn export(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter_map(|f| f.export_value().map(|v| (f.name().to_string(), v)))
            .collect()
    }

    /// Restores every field to its default.
    pub fn reset_all(&mut self) {
        for field in &mut self.fields {
            field.reset();
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_field_limits_its_length_in_characters_not_bytes() {
        // "ÁÉÍÓ" is four characters but eight bytes, so it fits a limit of four.
        let mut f = Field::text("zip", 0).limit_length(4);
        assert_eq!(f.max_length(), Some(4));
        assert_eq!(f.set_value("ÁÉÍÓ"), Ok(()));
        assert_eq!(f.set_value("12345"), Err(FieldError::TooLong));
        assert_eq!(f.value(), "ÁÉÍÓ");
        // Zero means no limit.
        assert_eq!(Field::text("note", 0).limit_length(0).max_length(), None);
    }

    #[test]
    fn read_only_flag_refuses_a_value_and_a_state_change() {
        let mut text = Field::text("id", READ_ONLY);
        assert_eq!(text.set_value("x"), Err(FieldError::ReadOnly));
        assert_eq!(text.value(), "");
        let mut check = Field::button("ok", READ_ONLY);
        assert_eq!(check.set_state(true), Err(FieldError::ReadOnly));
        assert!(!check.is_on());
    }

    #[test]
    fn choice_takes_only_its_options_unless_it_is_a_combo_box() {
        let mut size = Field::choice("size", &["S", "M", "L"], 0);
        assert_eq!(size.set_value("XL"), Err(FieldError::NotAnOption));
        assert_eq!(size.set_value("M"), Ok(()));
        assert_eq!(size.value(), "M");
        let mut free = Field::choice("free", &["a"], COMBO);
        assert!(free.is_combo());
        assert_eq!(free.set_value("anything"), Ok(()));
        assert_eq!(free.value(), "anything");
    }

    #[test]
    fn radio_button_cannot_be_turned_off_when_it_has_the_no_toggle_flag() {
        // The bit values: radio is bit 16 (32768) and no-toggle-to-off is bit 15 (16384).
        assert_eq!(RADIO, 32768);
        assert_eq!(NO_TOGGLE_TO_OFF, 16384);
        let mut locked = Field::button("r1", RADIO | NO_TOGGLE_TO_OFF);
        assert_eq!(locked.control(), Some(ButtonControl::Radio));
        assert!(!locked.allows_toggle_to_off());
        assert_eq!(locked.set_state(true), Ok(()));
        assert_eq!(locked.set_state(false), Err(FieldError::CannotToggleOff));
        assert!(locked.is_on());

        let mut free = Field::button("r2", RADIO);
        assert!(free.allows_toggle_to_off());
        assert_eq!(free.set_state(true), Ok(()));
        assert_eq!(free.set_state(false), Ok(()));
        assert!(!free.is_on());
    }

    #[test]
    fn button_control_comes_from_flags_and_a_push_button_has_no_state() {
        assert_eq!(Field::button("p", PUSH_BUTTON).control(), Some(ButtonControl::PushButton));
        assert_eq!(Field::button("c", 0).control(), Some(ButtonControl::CheckBox));
        let mut push = Field::button("p", PUSH_BUTTON);
        assert_eq!(push.set_state(true), Err(FieldError::WrongKind));
        assert_eq!(Field::text("t", 0).control(), None);
        assert_eq!(Field::text("t", 0).set_state(true), Err(FieldError::WrongKind));
    }

    #[test]
    fn missing_required_lists_empty_required_fields_and_export_skips_unchecked_boxes() {
        let mut form = FieldSet::new();
        form.add(Field::text("name", REQUIRED)).unwrap();
        form.add(Field::button("agree", REQUIRED)).unwrap();
        form.add(Field::text("note", 0)).unwrap();
        assert_eq!(form.missing_required(), vec!["name", "agree"]);

        form.set_value("name", "Ola").unwrap();
        form.set_state("agree", true).unwrap();
        assert!(form.missing_required().is_empty());
        // The empty note is still sent: a text field always sends its text.
        assert_eq!(
            form.export(),
            vec![
                ("name".to_string(), "Ola".to_string()),
                ("agree".to_string(), "on".to_string()),
                ("note".to_string(), String::new()),
            ]
        );
        form.set_state("agree", false).unwrap();
        assert_eq!(form.export().len(), 2);
    }

    #[test]
    fn reset_all_restores_the_defaults_and_names_stay_unique() {
        let mut form = FieldSet::new();
        form.add(Field::text("city", 0).with_default("Reykjavik", false)).unwrap();
        form.add(Field::button("news", 0).with_default("", true)).unwrap();
        assert_eq!(form.add(Field::text("city", 0)), Err(FieldError::Duplicate));
        assert_eq!(form.len(), 2);

        form.set_value("city", "Akureyri").unwrap();
        form.set_state("news", false).unwrap();
        form.reset_all();
        assert_eq!(form.get("city").unwrap().value(), "Reykjavik");
        assert!(form.get("news").unwrap().is_on());
        assert_eq!(form.set_value("missing", "x"), Err(FieldError::Unknown));
    }
}
