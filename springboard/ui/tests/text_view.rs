//! Multi-line text view, TextKit 2 style layout, paragraph styles, lists, tables, attachments, selection
//! navigation and the find bar, through the public API of `bifrost_ui`.

use bifrost_ui::paragraph::{Alignment, ParagraphStyle, TabAlignment, TextTab};
use bifrost_ui::text_attachment::TextAttachment;
use bifrost_ui::text_layout_manager::{
    Direction, Granularity, ListItem, ParagraphAttrs, TextContentStorage, TextLayoutManager, TextSelection,
};
use bifrost_ui::text_table::{TextTable, TextTableBlock};
use bifrost_ui::textfind::Options;
use bifrost_ui::textlayout::TextContainer;
use bifrost_ui::textlist::{Format, List};
use bifrost_ui::textview::TextView;

/// Monospace: 10 points per character.
fn m(s: &str) -> f32 {
    s.chars().count() as f32 * 10.0
}

const LINE: f32 = 20.0;

fn laid_out(text: &str, width: f32) -> (TextContentStorage, TextLayoutManager) {
    let content = TextContentStorage::new(text);
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(width, f32::INFINITY), &m);
    (content, layout)
}

fn styled(style: ParagraphStyle) -> ParagraphAttrs {
    ParagraphAttrs { style, list: None }
}

fn list_attrs(format: Format, level: usize) -> ParagraphAttrs {
    ParagraphAttrs { style: ParagraphStyle::default(), list: Some(ListItem { list: List::new(format), level }) }
}

// --- layout invalidation -----------------------------------------------------------------------------------------

#[test]
fn an_edit_lays_out_only_the_paragraph_it_touched() {
    let mut content = TextContentStorage::new("aaa\nbbb\nccc");
    let mut layout = TextLayoutManager::new(LINE);
    let container = TextContainer::new(100.0, f32::INFINITY);
    layout.ensure_layout(&content, &container, &m);
    assert_eq!(layout.relaid_paragraphs(), &[0, 1, 2]);

    assert!(content.replace_characters(5..6, "X"));
    layout.ensure_layout(&content, &container, &m);
    assert_eq!(layout.relaid_paragraphs(), &[1]);
    assert_eq!(layout.fragments()[1].range, 4..7);

    // A new first paragraph shifts the others without laying them out again.
    assert!(content.replace_characters(0..0, "new\n"));
    layout.ensure_layout(&content, &container, &m);
    assert_eq!(layout.relaid_paragraphs(), &[0]);
    assert_eq!(layout.fragments().len(), 4);
    assert_eq!(layout.fragments()[3].range, 12..15);
}

#[test]
fn a_new_container_width_lays_out_everything_again() {
    let content = TextContentStorage::new("aaa\nbbb");
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    assert!(layout.relaid_paragraphs().is_empty());
    layout.ensure_layout(&content, &TextContainer::new(80.0, f32::INFINITY), &m);
    assert_eq!(layout.relaid_paragraphs(), &[0, 1]);
}

// --- wrapping and paragraph styles -------------------------------------------------------------------------------

#[test]
fn lines_wrap_at_the_container_width() {
    let (_, layout) = laid_out("aa bb cc dd", 50.0);
    let lines: Vec<_> = layout.fragments()[0].lines.iter().map(|l| l.range.clone()).collect();
    assert_eq!(lines, vec![0..5, 6..11]);
    assert_eq!(layout.used_height(), 2.0 * LINE);
    assert_eq!(layout.line_count(), 2);
}

#[test]
fn line_fragment_padding_narrows_the_lines() {
    let content = TextContentStorage::new("aa bb cc dd");
    let mut layout = TextLayoutManager::new(LINE);
    let mut container = TextContainer::new(60.0, f32::INFINITY);
    container.line_fragment_padding = 5.0;
    layout.ensure_layout(&content, &container, &m);
    // 50 points are free: the same wrap as a 50 point container, shifted by the padding.
    assert_eq!(layout.fragments()[0].lines[0].range, 0..5);
    assert_eq!(layout.fragments()[0].lines[0].x, 5.0);
}

#[test]
fn head_indents_apply_to_the_first_line_and_the_rest() {
    let mut content = TextContentStorage::new("aa bb cc dd");
    let style = ParagraphStyle { first_line_head_indent: 20.0, head_indent: 10.0, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..11, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(70.0, f32::INFINITY), &m);
    let lines = &layout.fragments()[0].lines;
    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0].range.clone(), lines[0].x), (0..5, 20.0));
    assert_eq!((lines[1].range.clone(), lines[1].x), (6..11, 10.0));
}

#[test]
fn tail_indent_shortens_the_line() {
    let mut content = TextContentStorage::new("aa bb cc");
    let style = ParagraphStyle { tail_indent: -30.0, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..8, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(80.0, f32::INFINITY), &m);
    // 50 points are left of 80 - 30: "aa bb" fits, "cc" wraps.
    assert_eq!(layout.fragments()[0].lines.len(), 2);
}

#[test]
fn alignment_places_the_line_in_the_container() {
    for (alignment, x) in [(Alignment::Left, 0.0), (Alignment::Center, 40.0), (Alignment::Right, 80.0)] {
        let mut content = TextContentStorage::new("aa");
        let style = ParagraphStyle { alignment, ..ParagraphStyle::default() };
        assert!(content.set_paragraph_attrs(0..2, styled(style)));
        let mut layout = TextLayoutManager::new(LINE);
        layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
        assert_eq!(layout.fragments()[0].lines[0].x, x, "{alignment:?}");
    }
}

#[test]
fn line_and_paragraph_spacing_move_the_following_text_down() {
    let mut content = TextContentStorage::new("a\nb");
    let style = ParagraphStyle { line_spacing: 4.0, paragraph_spacing: 6.0, paragraph_spacing_before: 2.0, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..3, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    let f = layout.fragments();
    assert_eq!((f[0].y, f[0].height), (0.0, 24.0));
    // 24 for the first line, 6 after the paragraph, 2 before the next one.
    assert_eq!(f[1].y, 32.0);
    assert_eq!(layout.used_height(), 32.0 + 24.0 + 6.0);
}

#[test]
fn line_height_multiple_scales_the_natural_height() {
    let mut content = TextContentStorage::new("a");
    let style = ParagraphStyle { line_height_multiple: 1.5, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..1, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    assert_eq!(layout.fragments()[0].lines[0].height, 30.0);
}

#[test]
fn clipping_keeps_a_paragraph_on_one_line() {
    let mut content = TextContentStorage::new("aa bb cc dd");
    let style = ParagraphStyle { line_break_mode: bifrost_ui::paragraph::LineBreakMode::Clip, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..11, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(50.0, f32::INFINITY), &m);
    assert_eq!(layout.fragments()[0].lines.len(), 1);
}

#[test]
fn style_survives_typing_at_the_start_of_the_paragraph() {
    let mut content = TextContentStorage::new("bb");
    let style = ParagraphStyle { alignment: Alignment::Right, ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..2, styled(style)));
    assert!(content.replace_characters(0..0, "a"));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    assert_eq!(layout.fragments()[0].lines[0].x, 70.0);
}

// --- tab stops ---------------------------------------------------------------------------------------------------

#[test]
fn a_tab_moves_to_the_next_tab_stop() {
    let mut content = TextContentStorage::new("a\tb");
    let style = ParagraphStyle { tab_stops: vec![TextTab { alignment: TabAlignment::Left, location: 40.0 }], ..ParagraphStyle::default() };
    assert!(content.set_paragraph_attrs(0..3, styled(style)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    assert_eq!(layout.x_of(&content, 1, &m), 10.0);
    assert_eq!(layout.x_of(&content, 2, &m), 40.0, "the text after the tab starts on the stop");
    assert_eq!(layout.x_of(&content, 3, &m), 50.0);
    assert_eq!(layout.fragments()[0].lines[0].width, 50.0);
}

#[test]
fn without_tab_stops_the_default_interval_applies() {
    let (content, layout) = laid_out("a\tb", 100.0);
    // The default interval is 28 points: 10 points of "a" reach the stop at 28.
    assert_eq!(layout.x_of(&content, 2, &m), 28.0);
}

// --- lists -------------------------------------------------------------------------------------------------------

#[test]
fn numbered_markers_renumber_when_items_are_inserted_and_removed() {
    let mut content = TextContentStorage::new("one\ntwo\nthree");
    assert!(content.set_paragraph_attrs(0..13, list_attrs(Format::Decimal, 0)));
    let markers = |c: &TextContentStorage| c.list_markers();
    assert_eq!(markers(&content), vec![Some("1.".to_string()), Some("2.".to_string()), Some("3.".to_string())]);

    // A new item in the middle, once it joins the list, pushes the later numbers up.
    assert!(content.replace_characters(4..4, "x\n"));
    assert!(content.set_paragraph_attrs(4..5, list_attrs(Format::Decimal, 0)));
    let m4: Vec<_> = markers(&content).into_iter().flatten().collect();
    assert_eq!(m4, vec!["1.", "2.", "3.", "4."]);

    // Removing an item closes the gap.
    assert!(content.replace_characters(4..6, ""));
    let m3: Vec<_> = markers(&content).into_iter().flatten().collect();
    assert_eq!(m3, vec!["1.", "2.", "3."]);
}

#[test]
fn bullets_do_not_count_and_a_plain_paragraph_ends_the_list() {
    let mut content = TextContentStorage::new("a\nb\nplain\nc");
    assert!(content.set_paragraph_attrs(0..3, list_attrs(Format::Disc, 0)));
    assert!(content.set_paragraph_attrs(10..11, list_attrs(Format::Disc, 0)));
    assert_eq!(
        content.list_markers(),
        vec![Some("•".to_string()), Some("•".to_string()), None, Some("•".to_string())]
    );
    let mut numbered = TextContentStorage::new("a\nplain\nb");
    assert!(numbered.set_paragraph_attrs(0..1, list_attrs(Format::Decimal, 0)));
    assert!(numbered.set_paragraph_attrs(8..9, list_attrs(Format::Decimal, 0)));
    assert_eq!(numbered.list_markers(), vec![Some("1.".to_string()), None, Some("1.".to_string())]);
}

#[test]
fn nested_lists_count_under_each_parent() {
    let mut content = TextContentStorage::new("a\nb\nc\nd");
    assert!(content.set_paragraph_attrs(0..1, list_attrs(Format::Decimal, 0)));
    assert!(content.set_paragraph_attrs(2..3, list_attrs(Format::LowercaseAlpha, 1)));
    assert!(content.set_paragraph_attrs(4..5, list_attrs(Format::LowercaseAlpha, 1)));
    assert!(content.set_paragraph_attrs(6..7, list_attrs(Format::Decimal, 0)));
    let markers: Vec<_> = content.list_markers().into_iter().flatten().collect();
    assert_eq!(markers, vec!["1.", "a.", "b.", "2."]);
}

#[test]
fn a_list_paragraph_carries_its_marker_in_the_layout() {
    let mut content = TextContentStorage::new("one\ntwo");
    assert!(content.set_paragraph_attrs(0..7, list_attrs(Format::Decimal, 0)));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    assert_eq!(layout.fragments()[1].marker.as_deref(), Some("2."));
}

// --- tables ------------------------------------------------------------------------------------------------------

#[test]
fn a_two_by_two_table_lays_out_its_cells() {
    let cells = ["aa bb cc dd", "x", "y", "z"];
    let mut table = TextTable::new(2);
    for (i, _) in cells.iter().enumerate() {
        let mut b = TextTableBlock::new(i / 2, i % 2, 0..0);
        b.padding = 5.0;
        table.blocks.push(b);
    }
    let layout = table.layout(200.0, &|i, w| {
        let content = TextContentStorage::new(cells[i]);
        let mut lm = TextLayoutManager::new(LINE);
        lm.ensure_layout(&content, &TextContainer::new(w, f32::INFINITY), &m);
        lm.used_height()
    });
    // 90 points of content width: "aa bb cc" fits (80), "dd" wraps, so the first row is two lines plus padding.
    assert_eq!(layout.row_heights, vec![2.0 * LINE + 10.0, LINE + 10.0]);
    assert_eq!(layout.cells[1].rect.x, 100.0);
    assert_eq!(layout.cells[2].rect.y, 2.0 * LINE + 10.0);
    assert_eq!(layout.height, 3.0 * LINE + 20.0);
}

// --- attachments -------------------------------------------------------------------------------------------------

#[test]
fn an_attachment_takes_its_glyph_run_size() {
    let mut content = TextContentStorage::new("ab");
    let mut att = TextAttachment::new("public.png", vec![], (30.0, 10.0));
    att.bounds = Some((30.0, 40.0));
    assert!(content.insert_attachment(1, att));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(200.0, f32::INFINITY), &m);
    let line = &layout.fragments()[0].lines[0];
    // "a" 10 + attachment 30 + "b" 10, and the line grows to the attachment's height.
    assert_eq!(line.width, 50.0);
    assert_eq!(line.height, 40.0);
    // The attachment character is three bytes: "b" starts at byte 4.
    assert_eq!(layout.x_of(&content, 4, &m), 40.0);
}

#[test]
fn a_wide_attachment_shrinks_to_the_container() {
    let mut content = TextContentStorage::new("");
    assert!(content.insert_attachment(0, TextAttachment::new("public.png", vec![], (200.0, 100.0))));
    let mut layout = TextLayoutManager::new(LINE);
    layout.ensure_layout(&content, &TextContainer::new(100.0, f32::INFINITY), &m);
    let line = &layout.fragments()[0].lines[0];
    assert_eq!((line.width, line.height), (100.0, 50.0));
}

#[test]
fn attachments_move_with_edits_before_them() {
    let mut content = TextContentStorage::new("ab");
    assert!(content.insert_attachment(1, TextAttachment::new("public.png", vec![], (5.0, 5.0))));
    assert!(content.attachment_at(1).is_some());
    assert!(content.replace_characters(0..0, "zz"));
    assert!(content.attachment_at(1).is_none());
    assert!(content.attachment_at(3).is_some());
    // Deleting the attachment character removes the attachment.
    assert!(content.replace_characters(3..6, ""));
    assert!(content.attachment_at(3).is_none());
}

// --- selection navigation ----------------------------------------------------------------------------------------

#[test]
fn selection_moves_by_character_word_line_and_paragraph() {
    let (content, layout) = laid_out("hello world\nsecond line here", 1000.0);
    let go = |from: usize, d: Direction, g: Granularity| {
        TextSelection::caret(from).moved(&layout, &content, d, g, false, &m).head
    };
    assert_eq!(go(0, Direction::Right, Granularity::Character), 1);
    assert_eq!(go(0, Direction::Right, Granularity::Word), 5);
    assert_eq!(go(5, Direction::Right, Granularity::Word), 11);
    assert_eq!(go(11, Direction::Left, Granularity::Word), 6);
    assert_eq!(go(3, Direction::Right, Granularity::Line), 11);
    assert_eq!(go(8, Direction::Left, Granularity::Line), 0);
    assert_eq!(go(0, Direction::Right, Granularity::Paragraph), 11);
    assert_eq!(go(11, Direction::Right, Granularity::Paragraph), 28);
    assert_eq!(go(20, Direction::Left, Granularity::Paragraph), 12);
    assert_eq!(go(12, Direction::Left, Granularity::Paragraph), 0);
}

#[test]
fn up_and_down_keep_the_x_position() {
    let (content, layout) = laid_out("long line here\nab\nlong line here", 1000.0);
    let s = TextSelection::caret(10);
    let down = s.moved(&layout, &content, Direction::Down, Granularity::Character, false, &m);
    assert_eq!(down.head, 17, "the short line ends before x = 100");
    let again = down.moved(&layout, &content, Direction::Down, Granularity::Character, false, &m);
    assert_eq!(again.head, 28, "the goal x of 100 comes back on the long line");
    let up = again.moved(&layout, &content, Direction::Up, Granularity::Character, false, &m);
    assert_eq!(up.head, 17);
    // At the first and last lines a move goes to the edge of the text.
    let top = TextSelection::caret(3).moved(&layout, &content, Direction::Up, Granularity::Character, false, &m);
    assert_eq!(top.head, 0);
}

#[test]
fn up_and_down_follow_wrapped_lines() {
    let (content, layout) = laid_out("aa bb cc dd", 50.0);
    let down = TextSelection::caret(1).moved(&layout, &content, Direction::Down, Granularity::Character, false, &m);
    assert_eq!(down.head, 7);
}

#[test]
fn extending_keeps_the_anchor_and_a_plain_move_collapses() {
    let (content, layout) = laid_out("hello world", 1000.0);
    let s = TextSelection::caret(2).moved(&layout, &content, Direction::Right, Granularity::Word, true, &m);
    assert_eq!((s.anchor, s.head), (2, 5));
    assert_eq!(s.range(), 2..5);
    let left = s.moved(&layout, &content, Direction::Left, Granularity::Character, false, &m);
    assert_eq!((left.anchor, left.head), (2, 2));
    let right = s.moved(&layout, &content, Direction::Right, Granularity::Character, false, &m);
    assert_eq!((right.anchor, right.head), (5, 5));
}

#[test]
fn a_point_maps_to_the_nearest_boundary() {
    let (content, layout) = laid_out("aa bb cc dd", 50.0);
    assert_eq!(layout.offset_at_point(&content, 26.0, 5.0, &m), 3);
    // The second line starts at y = 20.
    assert_eq!(layout.offset_at_point(&content, 20.0, 25.0, &m), 8);
    // Below the text: the last line.
    assert_eq!(layout.offset_at_point(&content, 1000.0, 500.0, &m), 11);
}

// --- the view: find bar, rich text, undo -------------------------------------------------------------------------

fn view(text: &str) -> TextView {
    TextView::new(text, 1000.0, LINE, 100.0)
}

#[test]
fn find_next_and_previous_wrap_around() {
    let mut v = view("one two one TWO one");
    assert!(v.show_find_bar());
    assert_eq!(v.set_find_query("one", &m), Some(0..3));
    assert_eq!(v.editor.buf.selection(), 0..3);
    assert_eq!(v.find_next(&m), Some(8..11));
    assert_eq!(v.find_next(&m), Some(16..19));
    assert_eq!(v.find_next(&m), Some(0..3), "wraps to the first match");
    assert_eq!(v.find_previous(&m), Some(16..19), "wraps to the last match");
    assert_eq!(v.find_session().position(), Some((3, 3)));
}

#[test]
fn incremental_search_starts_at_the_selection() {
    let mut v = view("one two one");
    v.place_caret(4, false);
    assert_eq!(v.set_find_query("o", &m), Some(6..7));
    assert_eq!(v.set_find_query("on", &m), Some(8..10));
    assert_eq!(v.set_find_query("zzz", &m), None);
    assert_eq!(v.find_session().count(), 0);
}

#[test]
fn find_honours_case_and_whole_word_options() {
    let mut v = view("cat concat Cat");
    v.set_find_options(Options { ignore_case: true, whole_word: true });
    v.set_find_query("cat", &m);
    assert_eq!(v.find_session().matches(), &[0..3, 11..14]);
    v.set_find_options(Options { ignore_case: false, whole_word: true });
    assert_eq!(v.find_session().matches(), &[0..3]);
    v.set_find_options(Options { ignore_case: false, whole_word: false });
    assert_eq!(v.find_session().matches(), &[0..3, 7..10]);
}

#[test]
fn replace_all_is_one_undo_step() {
    let mut v = view("cat and cat and cat");
    v.set_find_query("cat", &m);
    assert_eq!(v.replace_all("dog"), 3);
    assert_eq!(v.text(), "dog and dog and dog");
    assert!(v.editor.undo());
    assert_eq!(v.text(), "cat and cat and cat");
    assert!(v.editor.redo());
    assert_eq!(v.text(), "dog and dog and dog");
}

#[test]
fn replace_current_replaces_only_the_selected_match() {
    let mut v = view("one two one");
    v.set_find_query("one", &m);
    assert!(v.replace_current("1"));
    assert_eq!(v.text(), "1 two one");
    // The caret now sits after the replacement; it is not on a match.
    assert!(!v.replace_current("x"));
    assert_eq!(v.find_next(&m), Some(6..9));
}

#[test]
fn a_view_without_the_find_bar_refuses_to_show_it() {
    let mut v = view("text");
    v.set_uses_find_bar(false);
    assert!(!v.show_find_bar());
    assert!(!v.find_bar_visible());
    v.set_uses_find_bar(true);
    assert!(v.show_find_bar());
    v.hide_find_bar();
    assert!(!v.find_bar_visible());
}

#[test]
fn typing_is_one_undo_step() {
    let mut v = view("");
    for ch in ["x", "y", "z"] {
        assert!(v.insert(ch, &m));
    }
    assert_eq!(v.text(), "xyz");
    assert!(v.editor.undo());
    assert_eq!(v.text(), "");
}

#[test]
fn allows_undo_off_keeps_no_history() {
    let mut v = view("");
    assert!(v.allows_undo());
    v.set_allows_undo(false);
    v.insert("x", &m);
    assert_eq!(v.text(), "x");
    assert!(!v.editor.undo());
    assert_eq!(v.text(), "x");
}

#[test]
fn rich_text_is_off_by_default_and_toggles() {
    let mut v = view("text");
    assert!(!v.is_rich_text());
    v.set_rich_text(true);
    assert!(v.is_rich_text());
}

#[test]
fn the_spell_checker_hook_reports_misspelled_ranges() {
    let mut v = view("good bad good");
    assert!(v.misspelled_ranges().is_empty());
    v.set_spell_checker(Some(Box::new(|t: &str| t.match_indices("bad").map(|(i, w)| i..i + w.len()).collect::<Vec<_>>())));
    assert_eq!(v.misspelled_ranges(), vec![5..8]);
}
