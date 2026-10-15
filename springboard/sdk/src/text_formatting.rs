//! The text formatting panel of a rich text view and the coordinator that binds it to the focused view.
//!
//! This is the SDK side of the "Text formatting controller" row of the tablet parity data. Three parts:
//!
//! * [`FormatState`] reads the formatting at the selection (or the typing attributes at a caret), so a panel can show
//!   what is in effect. A mark, colour or font that the whole selection shares is reported; a mixed selection reports
//!   the mark as off.
//! * [`FormatCommand`] and [`apply`] change a [`RichTextView`]. Character commands act on the selection, or set the typing
//!   attributes when the selection is a caret. Paragraph commands (alignment, lists) act on every paragraph the selection
//!   touches. Each command is one undo step of the view.
//! * [`FormattingCoordinator`] holds the view the panel is bound to. The shell binds the panel when a text view gains focus
//!   and unbinds it when the view loses focus; a command for any other view is refused, so a panel never formats text
//!   the person is not editing.
//!
//! [`panel_items`] lists the controls of the panel in display order with their active flag. The panel is a model: the shell
//! draws the items and sends the chosen control back as a command. Pointer, touch and keyboard all end in the same commands,
//! so the panel behaves the same on a desktop and a tablet.

use crate::attributed::{Attribute, AttributeKey, AttributedString, ColorToken, ListFormat, RichText, RichTextView, SizeClass};
use bifrost_ui::paragraph::Alignment;

/// The character styles the panel toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Underline,
    Strikethrough,
}

impl Mark {
    /// Every mark, in the order the panel shows them.
    pub const ALL: [Mark; 4] = [Mark::Bold, Mark::Italic, Mark::Underline, Mark::Strikethrough];

    fn key(self) -> AttributeKey {
        match self {
            Mark::Bold => AttributeKey::Bold,
            Mark::Italic => AttributeKey::Italic,
            Mark::Underline => AttributeKey::Underline,
            Mark::Strikethrough => AttributeKey::Strikethrough,
        }
    }

    fn attribute(self, on: bool) -> Attribute {
        match self {
            Mark::Bold => Attribute::Bold(on),
            Mark::Italic => Attribute::Italic(on),
            Mark::Underline => Attribute::Underline(on),
            Mark::Strikethrough => Attribute::Strikethrough(on),
        }
    }

    /// The name of the undo step for this mark.
    fn undo_name(self) -> &'static str {
        match self {
            Mark::Bold => "Bold",
            Mark::Italic => "Italic",
            Mark::Underline => "Underline",
            Mark::Strikethrough => "Strikethrough",
        }
    }
}

/// One change the panel asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FormatCommand {
    /// Turns a mark on or off: off when every character of a non-empty selection already has it, on otherwise.
    Toggle(Mark),
    /// The font family; `None` is the system font.
    Font(Option<String>),
    /// The text style size.
    Size(SizeClass),
    /// The text colour; `None` is the default colour.
    Color(Option<ColorToken>),
    /// The alignment of every paragraph the selection touches.
    Alignment(Alignment),
    /// The list every paragraph the selection touches becomes an item of; `None` ends the list.
    List(Option<ListFormat>),
}

/// The formatting in effect at a text view's selection.
#[derive(Clone, Debug, PartialEq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub font_family: Option<String>,
    pub size: SizeClass,
    pub color: Option<ColorToken>,
    pub alignment: Alignment,
    pub list: Option<ListFormat>,
}

impl Default for FormatState {
    fn default() -> Self {
        FormatState {
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            font_family: None,
            size: SizeClass::default(),
            color: None,
            alignment: Alignment::Natural,
            list: None,
        }
    }
}

impl FormatState {
    /// Whether `mark` is in effect.
    pub fn has(&self, mark: Mark) -> bool {
        match mark {
            Mark::Bold => self.bold,
            Mark::Italic => self.italic,
            Mark::Underline => self.underline,
            Mark::Strikethrough => self.strikethrough,
        }
    }
}

/// Reads the formatting of a rich text state. A caret reads the typing attributes. A selection reads its first character
/// for values, and reads a mark as on only when every character of the selection has it.
pub fn state(text: &RichText) -> FormatState {
    let sel = text.selection.clone();
    let char_attrs = if sel.is_empty() { text.typing.clone() } else { text.content.attributes_at(sel.start) };
    // Paragraph values are read where the caret or the selection starts; a caret at the end reads the last character.
    let para_attrs = text.content.attributes_at(sel.start);
    let marked = |m: Mark| {
        if sel.is_empty() {
            text.typing.get(m.key()) == m.attribute(true)
        } else {
            text.content.all(sel.clone(), m.key(), &m.attribute(true))
        }
    };
    FormatState {
        bold: marked(Mark::Bold),
        italic: marked(Mark::Italic),
        underline: marked(Mark::Underline),
        strikethrough: marked(Mark::Strikethrough),
        font_family: char_attrs.font_family.clone(),
        size: char_attrs.size,
        color: char_attrs.color,
        alignment: para_attrs.paragraph.as_ref().map_or(Alignment::Natural, |p| p.alignment),
        list: para_attrs.text_list.as_ref().map(|l| l.format),
    }
}

/// Applies a command to a rich text view. Each call is one undo step, except that a command that changes nothing still
/// registers nothing (the view compares the content before and after).
pub fn apply(view: &mut RichTextView, cmd: &FormatCommand) {
    match cmd {
        FormatCommand::Toggle(mark) => toggle(view, *mark),
        FormatCommand::Font(family) => set_character(view, "Font", Attribute::FontFamily(family.clone())),
        FormatCommand::Size(size) => set_character(view, "Size", Attribute::Size(*size)),
        FormatCommand::Color(color) => set_character(view, "Color", Attribute::Color(*color)),
        FormatCommand::Alignment(alignment) => set_alignment(view, *alignment),
        FormatCommand::List(list) => view.set_list(*list),
    }
}

fn toggle(view: &mut RichTextView, mark: Mark) {
    view.edit(mark.undo_name(), |t| {
        if t.selection.is_empty() {
            let on = t.typing.get(mark.key()) != mark.attribute(true);
            t.typing.set(&mark.attribute(on));
        } else {
            let on = !t.content.all(t.selection.clone(), mark.key(), &mark.attribute(true));
            t.content.add_attribute(mark.attribute(on), t.selection.clone());
        }
    });
}

/// Sets a character attribute on the selection, or on the typing attributes at a caret.
fn set_character(view: &mut RichTextView, name: &str, attr: Attribute) {
    view.edit(name, |t| {
        if t.selection.is_empty() {
            t.typing.set(&attr);
        } else {
            t.content.add_attribute(attr, t.selection.clone());
        }
    });
}

/// Sets the alignment of every paragraph the selection touches, keeping the other paragraph values of each.
fn set_alignment(view: &mut RichTextView, alignment: Alignment) {
    view.edit("Alignment", |t| {
        let start = t.selection.start;
        let mut style = t.content.attributes_at(start).paragraph.unwrap_or_default();
        style.alignment = alignment;
        let from = t.content.utf16_index_from_char(t.selection.start);
        let to = t.content.utf16_index_from_char(t.selection.end);
        t.content.set_paragraph_style(from..to, Some(style));
    });
}

/// A control of the formatting panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Font,
    Size,
    Mark(Mark),
    Color,
    Alignment,
    List,
}

/// One control of the panel with its label and whether its current value is not the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelItem {
    pub control: Control,
    pub label: &'static str,
    pub active: bool,
}

/// The controls of the panel in display order, with their active flags for a state.
pub fn panel_items(state: &FormatState) -> Vec<PanelItem> {
    let mut items = vec![
        PanelItem { control: Control::Font, label: "Font", active: state.font_family.is_some() },
        PanelItem { control: Control::Size, label: "Size", active: state.size != SizeClass::Body },
    ];
    for mark in Mark::ALL {
        let label = mark.undo_name();
        items.push(PanelItem { control: Control::Mark(mark), label, active: state.has(mark) });
    }
    items.push(PanelItem { control: Control::Color, label: "Color", active: state.color.is_some() });
    items.push(PanelItem { control: Control::Alignment, label: "Alignment", active: state.alignment != Alignment::Natural });
    items.push(PanelItem { control: Control::List, label: "Lists", active: state.list.is_some() });
    items
}

/// The identity of a text view that can take the panel.
pub type ViewId = u64;

/// Binds the formatting panel to the focused text view and routes panel commands to it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormattingCoordinator {
    bound: Option<ViewId>,
}

impl FormattingCoordinator {
    pub fn new() -> Self {
        FormattingCoordinator { bound: None }
    }

    /// Binds the panel to a view that gained focus. A previous binding is replaced.
    pub fn bind(&mut self, view: ViewId) {
        self.bound = Some(view);
    }

    /// Unbinds the panel when `view` loses focus. A view that is not bound leaves the binding alone.
    pub fn unbind(&mut self, view: ViewId) {
        if self.bound == Some(view) {
            self.bound = None;
        }
    }

    /// The bound view, if any.
    pub fn bound(&self) -> Option<ViewId> {
        self.bound
    }

    /// Applies `cmd` to `text` when `view` is the bound view. Returns false, and changes nothing, otherwise.
    pub fn route(&self, view: ViewId, text: &mut RichTextView, cmd: &FormatCommand) -> bool {
        if self.bound != Some(view) {
            return false;
        }
        apply(text, cmd);
        true
    }

    /// The keyboard shortcut for strikethrough, `Cmd+Shift+X`, routed like a panel command. The other formatting shortcuts
    /// (`Cmd+B`, `Cmd+I`, `Cmd+U`) are on [`RichTextView::key_command`]. Returns true when the key was handled.
    pub fn key_command(&self, view: ViewId, text: &mut RichTextView, ch: char, shift: bool) -> bool {
        if shift && ch.eq_ignore_ascii_case(&'x') {
            return self.route(view, text, &FormatCommand::Toggle(Mark::Strikethrough));
        }
        false
    }
}

/// The plain text of a rich text view, for the panel's preview of a selection.
pub fn selected_text(text: &RichText) -> String {
    let sel = text.selection.clone();
    let plain: AttributedString = text.content.substring(sel);
    plain.string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(text: &str, selection: std::ops::Range<usize>) -> RichTextView {
        let mut v = RichTextView::new(AttributedString::plain(text));
        v.set_selection(selection);
        v
    }

    #[test]
    fn toggle_marks_the_selection_and_turns_off_when_all_have_it() {
        let mut v = view("hello world", 0..5);
        apply(&mut v, &FormatCommand::Toggle(Mark::Bold));
        assert!(v.text.content.attributes_at(0).bold);
        assert!(!v.text.content.attributes_at(6).bold);
        assert!(state(&v.text).bold);
        apply(&mut v, &FormatCommand::Toggle(Mark::Bold));
        assert!(!v.text.content.attributes_at(0).bold);
    }

    #[test]
    fn a_mixed_selection_reports_the_mark_off_and_toggles_it_on() {
        let mut v = view("hello world", 0..11);
        apply(&mut v, &FormatCommand::Toggle(Mark::Italic));
        v.set_selection(0..5);
        apply(&mut v, &FormatCommand::Toggle(Mark::Italic));
        v.set_selection(0..11);
        assert!(!state(&v.text).italic, "the selection is mixed, so the mark is off");
        apply(&mut v, &FormatCommand::Toggle(Mark::Italic));
        assert!(v.text.content.all(0..11, AttributeKey::Italic, &Attribute::Italic(true)));
    }

    #[test]
    fn a_caret_sets_the_typing_attributes_for_the_next_character() {
        let mut v = view("", 0..0);
        apply(&mut v, &FormatCommand::Toggle(Mark::Underline));
        assert!(v.text.typing.underline);
        v.insert_text("x");
        assert!(v.text.content.attributes_at(0).underline);
    }

    #[test]
    fn strikethrough_toggles_over_the_selection() {
        let mut v = view("abc", 1..2);
        apply(&mut v, &FormatCommand::Toggle(Mark::Strikethrough));
        assert!(v.text.content.attributes_at(1).strikethrough);
        assert!(!v.text.content.attributes_at(0).strikethrough);
        assert!(state(&v.text).strikethrough);
    }

    #[test]
    fn font_size_and_colour_apply_to_the_selection() {
        let mut v = view("abcd", 0..2);
        apply(&mut v, &FormatCommand::Font(Some("Serif".into())));
        apply(&mut v, &FormatCommand::Size(SizeClass::Title1));
        apply(&mut v, &FormatCommand::Color(Some(ColorToken::Red)));
        let a = v.text.content.attributes_at(1);
        assert_eq!(a.font_family.as_deref(), Some("Serif"));
        assert_eq!(a.size, SizeClass::Title1);
        assert_eq!(a.color, Some(ColorToken::Red));
        let s = state(&v.text);
        assert_eq!(s.font_family.as_deref(), Some("Serif"));
        assert_eq!(s.size, SizeClass::Title1);
        assert_eq!(s.color, Some(ColorToken::Red));
        assert_eq!(v.text.content.attributes_at(3).color, None);
    }

    #[test]
    fn font_at_a_caret_sets_the_typing_attributes() {
        let mut v = view("", 0..0);
        apply(&mut v, &FormatCommand::Font(Some("Mono".into())));
        assert_eq!(v.text.typing.font_family.as_deref(), Some("Mono"));
    }

    #[test]
    fn alignment_applies_to_every_paragraph_the_selection_touches() {
        let mut v = view("one\ntwo\nthree", 0..0);
        apply(&mut v, &FormatCommand::Alignment(Alignment::Center));
        let first = v.text.content.attributes_at(0).paragraph.unwrap_or_default();
        assert_eq!(first.alignment, Alignment::Center);
        let second = v.text.content.attributes_at(5).paragraph.unwrap_or_default();
        assert_eq!(second.alignment, Alignment::Natural, "the next paragraph keeps its alignment");
        v.set_selection(0..9);
        apply(&mut v, &FormatCommand::Alignment(Alignment::Right));
        assert_eq!(v.text.content.attributes_at(5).paragraph.unwrap_or_default().alignment, Alignment::Right);
        assert_eq!(state(&v.text).alignment, Alignment::Right);
    }

    #[test]
    fn list_formats_the_selected_paragraphs() {
        let mut v = view("one\ntwo", 0..7);
        apply(&mut v, &FormatCommand::List(Some(ListFormat::Decimal)));
        assert_eq!(v.text.content.list_markers().len(), 2);
        assert_eq!(state(&v.text).list, Some(ListFormat::Decimal));
        apply(&mut v, &FormatCommand::List(None));
        assert!(v.text.content.list_markers().is_empty());
        assert_eq!(state(&v.text).list, None);
    }

    #[test]
    fn a_formatting_change_is_one_undo_step() {
        let mut v = view("hello", 0..5);
        apply(&mut v, &FormatCommand::Toggle(Mark::Bold));
        assert!(v.text.content.attributes_at(0).bold);
        assert!(v.key_command('z', false), "undo is a view command");
        assert!(!v.text.content.attributes_at(0).bold);
    }

    #[test]
    fn coordinator_routes_only_to_the_bound_view() {
        let mut c = FormattingCoordinator::new();
        let mut v = view("abc", 0..3);
        assert!(!c.route(1, &mut v, &FormatCommand::Toggle(Mark::Bold)), "nothing is bound");
        c.bind(1);
        assert_eq!(c.bound(), Some(1));
        assert!(!c.route(2, &mut v, &FormatCommand::Toggle(Mark::Bold)));
        assert!(!v.text.content.attributes_at(0).bold);
        assert!(c.route(1, &mut v, &FormatCommand::Toggle(Mark::Bold)));
        assert!(v.text.content.attributes_at(0).bold);
    }

    #[test]
    fn unbind_only_clears_the_bound_view() {
        let mut c = FormattingCoordinator::new();
        c.bind(7);
        c.unbind(3);
        assert_eq!(c.bound(), Some(7));
        c.unbind(7);
        assert_eq!(c.bound(), None);
    }

    #[test]
    fn shift_x_toggles_strikethrough_for_the_bound_view_only() {
        let mut c = FormattingCoordinator::new();
        let mut v = view("abc", 0..3);
        c.bind(1);
        assert!(!c.key_command(1, &mut v, 'x', false), "plain x is text, not a shortcut");
        assert!(c.key_command(1, &mut v, 'X', true));
        assert!(v.text.content.attributes_at(2).strikethrough);
        assert!(!c.key_command(2, &mut v, 'x', true));
    }

    #[test]
    fn panel_items_show_the_active_controls() {
        let plain = panel_items(&FormatState::default());
        assert_eq!(plain.len(), 9);
        assert!(plain.iter().all(|i| !i.active));
        let mut s = FormatState::default();
        s.bold = true;
        s.list = Some(ListFormat::Disc);
        s.alignment = Alignment::Center;
        let active: Vec<Control> = panel_items(&s).into_iter().filter(|i| i.active).map(|i| i.control).collect();
        assert_eq!(active, vec![Control::Mark(Mark::Bold), Control::Alignment, Control::List]);
    }

    #[test]
    fn selected_text_returns_the_selection() {
        let v = view("hello world", 6..11);
        assert_eq!(selected_text(&v.text), "world");
    }
}
