//! Parity sdk-tests-11: headless tests of the attributed string model and the rich text editing state through the
//! public API. Nothing here opens a display or draws a frame; the layout and drawing of rich text are not claimed.

use bifrost_app::attributed::{Attribute, AttributeKey, AttributedString, Attrs, ColorToken, RichTextView, SizeClass, Trait};

fn bold() -> Attrs {
    Attrs { bold: true, ..Default::default() }
}

fn italic() -> Attrs {
    Attrs { italic: true, ..Default::default() }
}

// --- the string model: ranges, runs and attribute edits ---

#[test]
fn a_plain_string_has_no_attributes_and_one_run() {
    let s = AttributedString::plain("hello");
    assert_eq!(s.string(), "hello");
    assert_eq!(s.len(), 5);
    assert!(!s.is_empty());
    assert_eq!(s.runs().count(), 1);
    assert_eq!(s.attributes_at(0), Attrs::default());
}

#[test]
fn an_empty_string_has_no_runs_and_no_length() {
    let s = AttributedString::new();
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert_eq!(s.runs().count(), 0);
}

#[test]
fn adding_bold_to_part_of_a_string_splits_it_into_two_runs() {
    let mut s = AttributedString::plain("hello world");
    s.add_attribute(Attribute::Bold(true), 0..5);
    assert_eq!(s.runs().count(), 2, "bold part and plain rest");
    assert_eq!(s.attributes_at(2), bold());
    assert_eq!(s.attributes_at(7), Attrs::default());
    s.add_attribute(Attribute::Italic(true), 3..8);
    assert_eq!(s.runs().count(), 4, "bold, bold italic, italic, plain");
    assert!(s.attributes_at(4).bold && s.attributes_at(4).italic);
}

#[test]
fn equal_neighbouring_runs_merge_so_equal_content_compares_equal() {
    let mut s = AttributedString::plain("hello world");
    s.add_attribute(Attribute::Bold(true), 0..5);
    s.add_attribute(Attribute::Bold(true), 5..11);
    assert_eq!(s.runs().count(), 1);
    s.remove_attribute(AttributeKey::Bold, 0..11);
    assert_eq!(s, AttributedString::plain("hello world"));
}

#[test]
fn an_empty_range_changes_nothing() {
    let mut s = AttributedString::plain("abc");
    s.add_attribute(Attribute::Bold(true), 2..2);
    assert_eq!(s, AttributedString::plain("abc"));
}

#[test]
fn a_range_past_the_end_is_clamped_to_the_string() {
    let mut s = AttributedString::plain("abc");
    s.add_attribute(Attribute::Italic(true), 1..40);
    assert!(!s.attributes_at(0).italic);
    assert!(s.attributes_at(1).italic && s.attributes_at(2).italic);
}

#[test]
fn removing_an_attribute_resets_it_to_its_default() {
    let mut s = AttributedString::with_attrs("styled", italic());
    s.remove_attribute(AttributeKey::Italic, 0..6);
    assert_eq!(s.attributes_at(3), Attrs::default());
}

#[test]
fn enumerating_an_attribute_reports_maximal_ranges_of_one_value() {
    let mut s = AttributedString::plain("abcdef");
    s.add_attribute(Attribute::Underline(true), 2..4);
    let mut seen = Vec::new();
    s.enumerate_attribute(AttributeKey::Underline, 0..6, |r, v| seen.push((r, v)));
    assert_eq!(
        seen,
        vec![(0..2, Attribute::Underline(false)), (2..4, Attribute::Underline(true)), (4..6, Attribute::Underline(false))]
    );
}

#[test]
fn enumerating_a_sub_range_clips_the_reported_ranges() {
    let mut s = AttributedString::plain("abcdef");
    s.add_attribute(Attribute::Underline(true), 2..4);
    let mut seen = Vec::new();
    s.enumerate_attribute(AttributeKey::Underline, 3..5, |r, v| seen.push((r, v)));
    assert_eq!(seen, vec![(3..4, Attribute::Underline(true)), (4..5, Attribute::Underline(false))]);
}

#[test]
fn all_is_true_only_when_every_character_of_the_range_has_the_value() {
    let mut s = AttributedString::plain("abcdef");
    s.add_attribute(Attribute::Bold(true), 0..3);
    let on = Attribute::Bold(true);
    assert!(s.all(0..3, AttributeKey::Bold, &on));
    assert!(!s.all(0..4, AttributeKey::Bold, &on), "one plain character breaks the range");
    assert!(!s.all(4..4, AttributeKey::Bold, &on), "an empty range is not all bold");
}

#[test]
fn substring_keeps_the_attributes_of_the_characters_it_copies() {
    let mut s = AttributedString::plain("one ");
    s.append("two", bold());
    let part = s.substring(2..6);
    assert_eq!(part.string(), "e tw");
    assert!(!part.attributes_at(0).bold);
    assert!(part.attributes_at(2).bold);
}

#[test]
fn append_adds_text_with_its_own_attributes() {
    let mut s = AttributedString::plain("a");
    s.append("b", italic());
    let mut tail = AttributedString::plain("c");
    tail.append("d", bold());
    s.append_attributed(&tail);
    assert_eq!(s.string(), "abcd");
    assert!(s.attributes_at(1).italic && !s.attributes_at(1).bold);
    assert!(s.attributes_at(3).bold);
}

#[test]
fn replacing_characters_takes_the_attributes_of_the_replaced_text() {
    let mut s = AttributedString::with_attrs("bold", bold());
    s.append(" plain", Attrs::default());
    s.replace_characters(1..3, "OO");
    assert_eq!(s.string(), "bOOd plain");
    assert!(s.attributes_at(1).bold);
    assert!(!s.attributes_at(8).bold);
}

#[test]
fn replacing_with_an_attributed_string_keeps_its_attributes() {
    let mut s = AttributedString::plain("xyz");
    let with = AttributedString::with_attrs("Q", bold());
    s.replace_attributed(1..2, &with);
    assert_eq!(s.string(), "xQz");
    assert!(s.attributes_at(1).bold);
    assert!(!s.attributes_at(0).bold);
}

#[test]
fn an_attribute_set_and_get_round_trip_for_each_key() {
    let mut attrs = Attrs::default();
    attrs.set(&Attribute::Kern(2.0));
    attrs.set(&Attribute::BaselineOffset(-1.5));
    attrs.set(&Attribute::Superscript(1));
    attrs.set(&Attribute::FontSize(Some(14.0)));
    attrs.set(&Attribute::Foreground(Some(0x00ff_0000)));
    assert_eq!(attrs.get(AttributeKey::Kern), Attribute::Kern(2.0));
    assert_eq!(attrs.get(AttributeKey::BaselineOffset), Attribute::BaselineOffset(-1.5));
    assert_eq!(attrs.get(AttributeKey::Superscript), Attribute::Superscript(1));
    assert_eq!(attrs.get(AttributeKey::FontSize), Attribute::FontSize(Some(14.0)));
    assert_eq!(attrs.get(AttributeKey::Foreground), Attribute::Foreground(Some(0x00ff_0000)));
    assert_eq!(attrs.get(AttributeKey::Bold), Attribute::Bold(false), "unset keys read their defaults");
}

#[test]
fn the_default_attributes_use_the_body_size_and_the_default_colour() {
    let attrs = Attrs::default();
    assert_eq!(attrs.size, SizeClass::Body);
    assert_eq!(attrs.color, None);
    assert_eq!(attrs.link, None);
    assert_eq!(attrs.font_size, None);
    assert_eq!(attrs.baseline_offset, 0.0);
}

// --- accessibility runs count UTF-16 units ---

#[test]
fn a_plain_string_is_one_accessibility_run_of_its_utf16_length() {
    let s = AttributedString::plain("abc");
    let runs = s.ax_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].utf16_start, runs[0].utf16_len), (0, 3));
}

#[test]
fn an_astral_character_takes_two_utf16_units_in_accessibility_runs() {
    let mut s = AttributedString::plain("a");
    s.append("\u{1F600}", bold());
    s.append("b", Attrs::default());
    assert_eq!(s.len(), 3, "three characters");
    let runs = s.ax_runs();
    assert_eq!(runs.len(), 3);
    assert_eq!((runs[0].utf16_start, runs[0].utf16_len), (0, 1));
    assert_eq!((runs[1].utf16_start, runs[1].utf16_len), (1, 2));
    assert!(runs[1].attrs.bold);
    assert_eq!((runs[2].utf16_start, runs[2].utf16_len), (3, 1));
}

#[test]
fn accessibility_runs_skip_nothing_and_an_empty_string_has_none() {
    assert!(AttributedString::new().ax_runs().is_empty());
}

// --- interchange: RTF and HTML round trips ---

#[test]
fn rtf_output_starts_with_the_rtf_header_and_keeps_text_and_bold() {
    let mut s = AttributedString::plain("plain bold");
    s.add_attribute(Attribute::Bold(true), 6..10);
    let rtf = s.to_rtf();
    assert!(rtf.starts_with("{\\rtf1"));
    let back = AttributedString::from_rtf(&rtf);
    assert_eq!(back.string(), "plain bold");
    assert!(back.attributes_at(7).bold);
    assert!(!back.attributes_at(0).bold);
}

#[test]
fn rtf_keeps_hyperlinks_on_the_linked_text_only() {
    let mut s = AttributedString::plain("see site now");
    s.add_attribute(Attribute::Link(Some("https://example.com/".to_string())), 4..8);
    let back = AttributedString::from_rtf(&s.to_rtf());
    assert_eq!(back.string(), "see site now");
    assert_eq!(back.attributes_at(5).link.as_deref(), Some("https://example.com/"));
    assert_eq!(back.attributes_at(0).link, None);
}

#[test]
fn rtf_keeps_braces_and_backslashes_in_the_text() {
    let s = AttributedString::plain("a {b} \\c");
    assert_eq!(AttributedString::from_rtf(&s.to_rtf()).string(), "a {b} \\c");
}

#[test]
fn html_round_trips_bold_and_italic_text() {
    let mut s = AttributedString::plain("Bold and italic");
    s.add_attribute(Attribute::Bold(true), 0..4);
    s.add_attribute(Attribute::Italic(true), 9..15);
    let back = AttributedString::from_html(&s.to_html());
    assert_eq!(back, s);
}

#[test]
fn html_escapes_angle_brackets_in_the_text() {
    let s = AttributedString::plain("1 < 2 > 0");
    let html = s.to_html();
    assert!(html.contains("&lt;"));
    assert!(!html.contains("1 < 2"), "a raw angle bracket would start a tag");
    assert_eq!(AttributedString::from_html(&html).string().trim_end(), "1 < 2 > 0");
}

#[test]
fn html_import_reads_bold_and_italic_tags() {
    let s = AttributedString::from_html("<b>Hi</b> <i>there</i>");
    assert_eq!(s.string().trim_end(), "Hi there");
    assert!(s.attributes_at(0).bold);
    assert!(s.attributes_at(3).italic);
    assert!(!s.attributes_at(0).italic);
}

// --- sizes and colour tokens ---

#[test]
fn size_classes_are_ordered_from_the_large_title_down_to_the_caption() {
    assert!(SizeClass::LargeTitle.pt() > SizeClass::Title1.pt());
    assert!(SizeClass::Title1.pt() > SizeClass::Body.pt());
    assert!(SizeClass::Body.pt() > SizeClass::Caption.pt());
}

#[test]
fn a_point_size_maps_to_the_nearest_size_class() {
    assert_eq!(SizeClass::nearest(26.0), SizeClass::LargeTitle);
    assert_eq!(SizeClass::nearest(SizeClass::Footnote.pt()), SizeClass::Footnote);
}

#[test]
fn colour_token_names_round_trip_and_unknown_names_are_refused() {
    for token in ColorToken::ALL {
        assert_eq!(ColorToken::from_name(token.name()), Some(token));
    }
    assert_eq!(ColorToken::from_name("mauve"), None);
    assert_eq!(ColorToken::Blue.name(), "blue");
}

// --- the rich text editing state ---

#[test]
fn a_new_rich_text_view_has_its_caret_at_the_end() {
    let v = RichTextView::new(AttributedString::plain("hello"));
    assert_eq!(v.text.selection, 5..5);
    assert_eq!(v.text.content.string(), "hello");
    assert!(!v.undo.can_undo());
}

#[test]
fn typing_inserts_at_the_caret_with_the_typing_attributes() {
    let mut v = RichTextView::new(AttributedString::plain("ab"));
    v.toggle(Trait::Italic);
    v.insert_text("c");
    assert_eq!(v.text.content.string(), "abc");
    assert!(v.text.content.attributes_at(2).italic);
    assert!(!v.text.content.attributes_at(0).italic);
    assert_eq!(v.text.selection, 3..3, "the caret moves past the typed text");
}

#[test]
fn a_selection_is_clamped_to_the_content() {
    let mut v = RichTextView::new(AttributedString::plain("hello"));
    v.set_selection(2..40);
    assert_eq!(v.text.selection, 2..5);
    v.set_selection(9..12);
    assert_eq!(v.text.selection, 5..5);
}

#[test]
fn command_b_toggles_bold_on_the_selection_and_undo_and_redo_reverse_it() {
    let mut v = RichTextView::new(AttributedString::plain("hello"));
    v.set_selection(0..5);
    assert!(v.key_command('b', false));
    assert!(v.text.content.attributes_at(0).bold);
    assert!(v.key_command('b', false));
    assert!(!v.text.content.attributes_at(0).bold, "a second press clears bold");
    assert!(v.key_command('z', false), "undo");
    assert!(v.text.content.attributes_at(0).bold);
    assert!(v.key_command('z', true), "redo");
    assert!(!v.text.content.attributes_at(0).bold);
}

#[test]
fn an_unknown_command_key_is_not_handled() {
    let mut v = RichTextView::new(AttributedString::plain("hello"));
    assert!(!v.key_command('q', false));
    assert_eq!(v.text.content.string(), "hello");
}

#[test]
fn undo_removes_the_typed_text_again() {
    let mut v = RichTextView::new(AttributedString::plain("ab"));
    v.insert_text("c");
    assert!(v.undo.can_undo());
    v.key_command('z', false);
    assert_eq!(v.text.content.string(), "ab");
}

#[test]
fn toggling_underline_with_a_caret_only_changes_the_typing_attributes() {
    let mut v = RichTextView::new(AttributedString::plain("ab"));
    v.toggle(Trait::Underline);
    assert!(v.text.typing.underline);
    assert!(!v.text.content.attributes_at(1).underline, "the text itself is unchanged");
    assert_eq!(v.text.content.string(), "ab");
}
