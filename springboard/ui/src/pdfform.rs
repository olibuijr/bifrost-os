//! PDF form widgets as a pure model: the widget subtype (button, choice, text or signature), the button control type
//! (check box, radio button or push button), the text and choice options decoded from the field flags, the value and
//! default value, and the rules for changing a value: read-only fields refuse changes, text is limited by its maximum
//! length, a fixed list only accepts its listed options, and the radio buttons of one group hold at most one selection.
//!
//! The flag bits are the ones the PDF specification defines for the form field flags (bit 1 is the lowest bit). A
//! reader fills a [`Widget`] from the field dictionary; nothing here parses a file or draws a widget.

/// The kind of widget, as the field's type reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetSubtype {
    /// A check box, radio button or push button.
    Button,
    /// A list box or a pop-up menu.
    Choice,
    /// A text entry field.
    Text,
    /// A digital signature field. It holds no value that this model sets.
    Signature,
}

/// The control of a button widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlType {
    CheckBox,
    RadioButton,
    PushButton,
}

/// The state of a button widget. `Mixed` is shown when a group has no single selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    Off,
    On,
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    /// The field has the read-only flag set.
    ReadOnly,
    /// The text is longer than the field's maximum length, counted in characters.
    TooLong,
    /// A fixed list field was given a value that is not one of its options.
    NotAnOption,
    /// The operation does not apply to this kind of widget, such as checking a text field.
    WrongType,
    /// A radio group index is past the end of the group.
    BadIndex,
}

// Field flag bits (PDF form field flags, bit n is 1 << (n - 1)).
const FLAG_READ_ONLY: u32 = 1 << 0;
const FLAG_BTN_NO_TOGGLE_TO_OFF: u32 = 1 << 14;
const FLAG_BTN_RADIO: u32 = 1 << 15;
const FLAG_BTN_PUSH: u32 = 1 << 16;
const FLAG_TX_MULTILINE: u32 = 1 << 12;
const FLAG_TX_PASSWORD: u32 = 1 << 13;
const FLAG_TX_COMB: u32 = 1 << 24;
const FLAG_CH_COMBO: u32 = 1 << 17;
const FLAG_CH_EDIT: u32 = 1 << 18;

/// One form widget: its field flags, value and options.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    subtype: WidgetSubtype,
    flags: u32,
    name: String,
    max_len: Option<usize>,
    default_value: String,
    value: String,
    /// Choice options, in display order.
    options: Vec<String>,
    /// Export values of the options, one per option, when the field gives them.
    export_values: Vec<String>,
    /// Button only: the appearance state this widget shows when it is on (its export name).
    on_state: String,
    cell: CellState,
}

impl Widget {
    /// A widget of `subtype` with the given field flags and field name. It starts empty and off.
    pub fn new(subtype: WidgetSubtype, flags: u32, name: &str) -> Widget {
        Widget {
            subtype,
            flags,
            name: name.to_string(),
            max_len: None,
            default_value: String::new(),
            value: String::new(),
            options: Vec::new(),
            export_values: Vec::new(),
            on_state: "Yes".to_string(),
            cell: CellState::Off,
        }
    }

    pub fn subtype(&self) -> WidgetSubtype {
        self.subtype
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// The fully qualified field name.
    pub fn field_name(&self) -> &str {
        &self.name
    }

    pub fn is_read_only(&self) -> bool {
        self.flags & FLAG_READ_ONLY != 0
    }

    /// The control of a button widget. None for other subtypes.
    pub fn control_type(&self) -> Option<ControlType> {
        if self.subtype != WidgetSubtype::Button {
            return None;
        }
        Some(if self.flags & FLAG_BTN_PUSH != 0 {
            ControlType::PushButton
        } else if self.flags & FLAG_BTN_RADIO != 0 {
            ControlType::RadioButton
        } else {
            ControlType::CheckBox
        })
    }

    /// For a radio button: whether a selected button may be turned off by choosing it again.
    pub fn allows_toggle_to_off(&self) -> bool {
        self.flags & FLAG_BTN_NO_TOGGLE_TO_OFF == 0
    }

    /// For a text field: whether it wraps onto several lines.
    pub fn is_multiline(&self) -> bool {
        self.subtype == WidgetSubtype::Text && self.flags & FLAG_TX_MULTILINE != 0
    }

    /// For a text field: whether the text is shown as bullets.
    pub fn is_password(&self) -> bool {
        self.subtype == WidgetSubtype::Text && self.flags & FLAG_TX_PASSWORD != 0
    }

    /// For a text field: whether the field is divided into equal cells, one per character of its maximum length.
    pub fn has_comb(&self) -> bool {
        self.subtype == WidgetSubtype::Text && self.flags & FLAG_TX_COMB != 0
    }

    /// For a choice field: true for a list box, false for a pop-up menu (combo box).
    pub fn is_list_choice(&self) -> bool {
        self.subtype == WidgetSubtype::Choice && self.flags & FLAG_CH_COMBO == 0
    }

    /// For a choice field: whether the user may type a value that is not one of the options.
    pub fn is_editable_choice(&self) -> bool {
        self.subtype == WidgetSubtype::Choice && self.flags & FLAG_CH_COMBO != 0 && self.flags & FLAG_CH_EDIT != 0
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_len
    }

    pub fn set_max_length(&mut self, n: Option<usize>) {
        self.max_len = n;
    }

    /// The options of a choice field, in display order.
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Sets the options of a choice field. Export values are cleared unless `exports` has one per option.
    pub fn set_options(&mut self, options: Vec<String>, exports: Vec<String>) {
        self.options = options;
        self.export_values = if exports.len() == self.options.len() { exports } else { Vec::new() };
    }

    /// The export value of option `i`, or the option text when no export values are set.
    pub fn export_value(&self, i: usize) -> Option<&str> {
        if let Some(e) = self.export_values.get(i) {
            return Some(e.as_str());
        }
        self.options.get(i).map(|s| s.as_str())
    }

    /// The appearance name of a button's on state.
    pub fn on_state(&self) -> &str {
        &self.on_state
    }

    pub fn set_on_state(&mut self, name: &str) {
        self.on_state = name.to_string();
    }

    /// The value of a text or choice field. Empty for other subtypes.
    pub fn string_value(&self) -> &str {
        &self.value
    }

    /// The value a reset form action restores.
    pub fn default_string_value(&self) -> &str {
        &self.default_value
    }

    pub fn set_default_string_value(&mut self, v: &str) {
        self.default_value = v.to_string();
    }

    /// The state of a button widget.
    pub fn cell_state(&self) -> CellState {
        self.cell
    }

    /// Sets the text of a text field or the selection of a choice field.
    ///
    /// Read-only fields refuse the change. A text field with a maximum length refuses text that is longer, counted in
    /// characters. A list choice refuses a value that is not one of its options. An editable choice accepts any text.
    pub fn set_string_value(&mut self, v: &str) -> Result<(), FormError> {
        match self.subtype {
            WidgetSubtype::Text | WidgetSubtype::Choice => {}
            _ => return Err(FormError::WrongType),
        }
        if self.is_read_only() {
            return Err(FormError::ReadOnly);
        }
        if self.subtype == WidgetSubtype::Text {
            if let Some(max) = self.max_len {
                if v.chars().count() > max {
                    return Err(FormError::TooLong);
                }
            }
        } else if !self.is_editable_choice() && !v.is_empty() && !self.options.iter().any(|o| o == v) {
            return Err(FormError::NotAnOption);
        }
        self.value = v.to_string();
        Ok(())
    }

    /// Sets the value back to the default. Read-only fields refuse it.
    pub fn reset(&mut self) -> Result<(), FormError> {
        if self.subtype != WidgetSubtype::Text && self.subtype != WidgetSubtype::Choice {
            return Err(FormError::WrongType);
        }
        if self.is_read_only() {
            return Err(FormError::ReadOnly);
        }
        self.value = self.default_value.clone();
        Ok(())
    }

    /// Turns a check box on or off. Radio buttons change through [`select_radio`]; push buttons have no state.
    pub fn set_checked(&mut self, on: bool) -> Result<(), FormError> {
        if self.control_type() != Some(ControlType::CheckBox) {
            return Err(FormError::WrongType);
        }
        if self.is_read_only() {
            return Err(FormError::ReadOnly);
        }
        self.cell = if on { CellState::On } else { CellState::Off };
        Ok(())
    }
}

/// Chooses radio button `index` of a group. The chosen button turns on and the others turn off. Choosing the button
/// that is already on turns it off, unless its field forbids toggling to off, in which case nothing changes.
///
/// Every widget in `group` must be a radio button. Read-only buttons refuse the change.
pub fn select_radio(group: &mut [Widget], index: usize) -> Result<(), FormError> {
    if index >= group.len() {
        return Err(FormError::BadIndex);
    }
    if group.iter().any(|w| w.control_type() != Some(ControlType::RadioButton)) {
        return Err(FormError::WrongType);
    }
    if group[index].is_read_only() {
        return Err(FormError::ReadOnly);
    }
    let was_on = group[index].cell == CellState::On;
    if was_on && !group[index].allows_toggle_to_off() {
        return Ok(());
    }
    for (i, w) in group.iter_mut().enumerate() {
        w.cell = if i == index && !was_on { CellState::On } else { CellState::Off };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(flags: u32) -> Widget {
        Widget::new(WidgetSubtype::Text, flags, "name")
    }

    fn radio(flags: u32) -> Widget {
        Widget::new(WidgetSubtype::Button, flags | FLAG_BTN_RADIO, "group")
    }

    #[test]
    fn button_flags_pick_the_control() {
        assert_eq!(Widget::new(WidgetSubtype::Button, 0, "a").control_type(), Some(ControlType::CheckBox));
        assert_eq!(
            Widget::new(WidgetSubtype::Button, FLAG_BTN_RADIO, "a").control_type(),
            Some(ControlType::RadioButton)
        );
        assert_eq!(
            Widget::new(WidgetSubtype::Button, FLAG_BTN_PUSH | FLAG_BTN_RADIO, "a").control_type(),
            Some(ControlType::PushButton)
        );
        assert_eq!(text(0).control_type(), None);
    }

    #[test]
    fn text_flags_decode_multiline_password_and_comb() {
        let w = text(FLAG_TX_MULTILINE | FLAG_TX_COMB);
        assert!(w.is_multiline());
        assert!(w.has_comb());
        assert!(!w.is_password());
        assert!(text(FLAG_TX_PASSWORD).is_password());
        // The text flags mean nothing on a choice field.
        assert!(!Widget::new(WidgetSubtype::Choice, FLAG_TX_MULTILINE, "c").is_multiline());
    }

    #[test]
    fn text_respects_maximum_length_in_characters() {
        let mut w = text(0);
        w.set_max_length(Some(3));
        assert_eq!(w.set_string_value("abcd"), Err(FormError::TooLong));
        assert_eq!(w.string_value(), "");
        // Three accented characters are three characters, even though they are longer in bytes.
        assert_eq!(w.set_string_value("ÁÉÍ"), Ok(()));
        assert_eq!(w.string_value(), "ÁÉÍ");
    }

    #[test]
    fn read_only_fields_refuse_changes_and_reset() {
        let mut w = text(FLAG_READ_ONLY);
        w.set_default_string_value("fixed");
        assert_eq!(w.set_string_value("x"), Err(FormError::ReadOnly));
        assert_eq!(w.reset(), Err(FormError::ReadOnly));
        let mut open = text(0);
        open.set_default_string_value("fixed");
        open.set_string_value("changed").unwrap();
        open.reset().unwrap();
        assert_eq!(open.string_value(), "fixed");
    }

    #[test]
    fn list_choice_only_takes_its_options_and_editable_takes_any() {
        let mut list = Widget::new(WidgetSubtype::Choice, 0, "size");
        list.set_options(vec!["S".into(), "M".into(), "L".into()], Vec::new());
        assert!(list.is_list_choice());
        assert_eq!(list.set_string_value("XL"), Err(FormError::NotAnOption));
        assert_eq!(list.set_string_value("M"), Ok(()));
        assert_eq!(list.string_value(), "M");

        let mut combo = Widget::new(WidgetSubtype::Choice, FLAG_CH_COMBO | FLAG_CH_EDIT, "size");
        combo.set_options(vec!["S".into()], Vec::new());
        assert!(combo.is_editable_choice());
        assert_eq!(combo.set_string_value("XL"), Ok(()));
    }

    #[test]
    fn export_value_falls_back_to_the_option_text() {
        let mut w = Widget::new(WidgetSubtype::Choice, 0, "c");
        w.set_options(vec!["Small".into(), "Large".into()], vec!["s".into()]);
        // One export value for two options does not match, so the option text is used.
        assert_eq!(w.export_value(0), Some("Small"));
        w.set_options(vec!["Small".into(), "Large".into()], vec!["s".into(), "l".into()]);
        assert_eq!(w.export_value(1), Some("l"));
        assert_eq!(w.export_value(2), None);
    }

    #[test]
    fn radio_group_holds_one_selection() {
        let mut group = vec![radio(0), radio(0), radio(0)];
        select_radio(&mut group, 1).unwrap();
        assert_eq!(group[1].cell_state(), CellState::On);
        assert_eq!(group[0].cell_state(), CellState::Off);
        select_radio(&mut group, 2).unwrap();
        assert_eq!(group[1].cell_state(), CellState::Off);
        assert_eq!(group[2].cell_state(), CellState::On);
        assert_eq!(select_radio(&mut group, 3), Err(FormError::BadIndex));
    }

    #[test]
    fn choosing_the_selected_radio_turns_it_off_only_when_allowed() {
        let mut free = vec![radio(0), radio(0)];
        select_radio(&mut free, 0).unwrap();
        select_radio(&mut free, 0).unwrap();
        assert_eq!(free[0].cell_state(), CellState::Off);

        let mut sticky = vec![radio(FLAG_BTN_NO_TOGGLE_TO_OFF), radio(FLAG_BTN_NO_TOGGLE_TO_OFF)];
        select_radio(&mut sticky, 0).unwrap();
        select_radio(&mut sticky, 0).unwrap();
        assert_eq!(sticky[0].cell_state(), CellState::On);
    }

    #[test]
    fn check_box_toggles_and_push_button_refuses() {
        let mut cb = Widget::new(WidgetSubtype::Button, 0, "agree");
        cb.set_checked(true).unwrap();
        assert_eq!(cb.cell_state(), CellState::On);
        cb.set_checked(false).unwrap();
        assert_eq!(cb.cell_state(), CellState::Off);
        let mut push = Widget::new(WidgetSubtype::Button, FLAG_BTN_PUSH, "go");
        assert_eq!(push.set_checked(true), Err(FormError::WrongType));
        assert_eq!(text(0).set_checked(true), Err(FormError::WrongType));
    }
}
