//! Parity sdk-tests-06: headless tests of the multi-line text view model and of handwriting line segmentation.
//! Each test drives the public API with a monospace measure (10 points per character) and asserts concrete values;
//! nothing here draws, opens a window or reads a clock.

use std::cell::RefCell;

use bifrost_ui::drawing::{Drawing, Ink, InkKind, Stroke, StrokePoint};
use bifrost_ui::handwriting::{recognize_lines, segment_lines, Recognizer};
use bifrost_ui::textview::{line_of, Highlight, TextView};
use bifrost_ui::Rect;

/// Monospace: 10 points per character.
fn m(s: &str) -> f32 {
    s.chars().count() as f32 * 10.0
}

fn texts<'a>(text: &'a str, lines: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    lines.iter().map(|r| &text[r.clone()]).collect()
}

// --- layout and geometry (textview) ----------------------------------------------------------------------------------

#[test]
fn the_text_wraps_at_word_boundaries_within_the_width() {
    let text = "alpha beta gamma";
    let view = TextView::new(text, 60.0, 20.0, 100.0);
    let lines = view.lines(&m);
    assert_eq!(texts(text, &lines).iter().map(|s| s.trim_end()).collect::<Vec<_>>(), vec!["alpha", "beta", "gamma"]);
    assert_eq!(view.content_height(&m), 60.0);
}

#[test]
fn a_newline_ends_a_line_and_an_empty_paragraph_is_an_empty_line() {
    let text = "one\n\nthree";
    let view = TextView::new(text, 200.0, 20.0, 100.0);
    let lines = view.lines(&m);
    assert_eq!(texts(text, &lines), vec!["one", "", "three"]);
}

#[test]
fn resizing_the_view_rewraps_the_text_without_any_invalidation() {
    let text = "abcdef ghij";
    let mut view = TextView::new(text, 100.0, 20.0, 100.0);
    assert_eq!(view.lines(&m).len(), 1);
    view.resize(40.0, 100.0, &m);
    assert!(view.lines(&m).len() > 1);
}

#[test]
fn a_point_inside_a_line_maps_to_the_nearest_caret_position() {
    let text = "abc\ndef";
    let mut view = TextView::new(text, 200.0, 20.0, 100.0);
    view.place_caret(5, false);
    let (x, y) = view.caret_point(&m);
    assert_eq!(view.hit_test(x + 1.0, y + 1.0, &m), 5);
    assert_eq!(view.hit_test(-50.0, 1.0, &m), 0, "left of the text maps to its start");
}

#[test]
fn a_byte_offset_belongs_to_the_visual_line_that_holds_it() {
    let text = "alpha beta gamma";
    let view = TextView::new(text, 60.0, 20.0, 100.0);
    let lines = view.lines(&m);
    assert_eq!(line_of(&lines, 0), 0);
    assert_eq!(line_of(&lines, 7), 1);
    assert_eq!(line_of(&lines, 16), 2);
}

#[test]
fn highlights_cover_the_selected_part_of_each_line_it_touches() {
    let text = "abc def";
    let mut view = TextView::new(text, 30.0, 20.0, 100.0);
    view.editor.buf.set_selection(2, 6);
    let highlights = view.highlights(&m);
    assert_eq!(
        highlights,
        vec![Highlight { line: 0, x0: 20.0, x1: 30.0 }, Highlight { line: 1, x0: 0.0, x1: 20.0 }]
    );
}

#[test]
fn an_empty_selection_has_no_highlights() {
    let view = TextView::new("abc", 200.0, 20.0, 100.0);
    assert!(view.highlights(&m).is_empty());
}

// --- caret movement (textview) ------------------------------------------------------------------------------------------

#[test]
fn vertical_movement_keeps_the_goal_column_across_a_short_line() {
    let text = "abcdef\nab\nabcdef";
    let mut view = TextView::new(text, 500.0, 20.0, 200.0);
    view.place_caret(4, false);
    assert!(view.move_vertical(true, false, &m));
    assert_eq!(view.editor.buf.caret(), 9, "the short line ends at its last character");
    assert!(view.move_vertical(true, false, &m));
    assert_eq!(view.editor.buf.caret(), 14, "the goal column of 40 points is back on the third line");
}

#[test]
fn moving_up_from_the_first_line_does_nothing() {
    let mut view = TextView::new("abc\ndef", 200.0, 20.0, 100.0);
    view.place_caret(1, false);
    assert!(!view.move_vertical(false, false, &m));
    assert_eq!(view.editor.buf.caret(), 1);
}

#[test]
fn scrolling_to_the_caret_keeps_the_last_line_in_the_viewport() {
    let text = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9";
    let mut view = TextView::new(text, 100.0, 20.0, 40.0);
    view.place_caret(text.len(), false);
    view.scroll_to_caret(&m);
    assert!(view.visible_lines(&m).contains(&9));
    view.set_scroll(0.0, &m);
    assert_eq!(view.visible_lines(&m).start, 0);
}

// --- editing through the view (textview) --------------------------------------------------------------------------------

#[test]
fn typing_inserts_at_the_caret_and_backspace_removes_the_character_before_it() {
    let mut view = TextView::new("ab", 200.0, 20.0, 100.0);
    view.place_caret(2, false);
    assert!(view.insert("c", &m));
    assert_eq!(view.text(), "abc");
    assert!(view.delete_backward(&m));
    assert_eq!(view.text(), "ab");
    assert_eq!(view.editor.buf.caret(), 2);
}

#[test]
fn backspace_in_an_empty_view_changes_nothing() {
    let mut view = TextView::new("", 200.0, 20.0, 100.0);
    assert!(!view.delete_backward(&m));
    assert_eq!(view.text(), "");
}

#[test]
fn the_find_bar_shows_and_hides_only_when_the_view_uses_it() {
    let mut view = TextView::new("abc", 200.0, 20.0, 100.0);
    assert!(view.uses_find_bar());
    assert!(!view.find_bar_visible());
    assert!(view.show_find_bar());
    assert!(view.find_bar_visible());
    view.hide_find_bar();
    assert!(!view.find_bar_visible());
    view.set_uses_find_bar(false);
    assert!(!view.show_find_bar(), "a view that uses the find panel refuses the bar");
}

#[test]
fn switching_off_the_find_bar_hides_it_at_once() {
    let mut view = TextView::new("abc", 200.0, 20.0, 100.0);
    assert!(view.show_find_bar());
    view.set_uses_find_bar(false);
    assert!(!view.find_bar_visible());
}

#[test]
fn rich_text_is_a_flag_the_view_keeps_separately_from_undo() {
    let mut view = TextView::new("abc", 200.0, 20.0, 100.0);
    assert!(!view.is_rich_text());
    view.set_rich_text(true);
    assert!(view.is_rich_text());
    assert!(view.allows_undo());
    view.set_allows_undo(false);
    assert!(view.is_rich_text(), "turning undo off does not change rich text");
}

#[test]
fn spelling_ranges_are_empty_without_a_checker_and_follow_the_hook_once_set() {
    let mut view = TextView::new("teh cat", 200.0, 20.0, 100.0);
    assert!(view.misspelled_ranges().is_empty());
    view.set_spell_checker(Some(Box::new(|t: &str| if t.starts_with("teh") { vec![0..3usize] } else { Vec::new() })));
    assert_eq!(view.misspelled_ranges(), vec![0..3]);
    view.set_spell_checker(None);
    assert!(view.misspelled_ranges().is_empty());
}

#[test]
fn no_composition_is_open_on_a_new_view() {
    let view = TextView::new("abc", 200.0, 20.0, 100.0);
    assert!(view.candidates().is_empty());
}

// --- handwriting line segmentation (handwriting) -------------------------------------------------------------------------

fn stroke(x0: f32, y0: f32, x1: f32, y1: f32) -> Stroke {
    Stroke {
        ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 },
        points: vec![StrokePoint::at(x0, y0), StrokePoint::at(x1, y1)],
    }
}

/// Reads a line as the joined x positions of its strokes, so tests can see the order it was handed in.
struct Positions {
    seen: RefCell<Vec<Vec<f32>>>,
}

impl Recognizer for Positions {
    fn recognize(&self, strokes: &[&Stroke]) -> Option<String> {
        let xs: Vec<f32> = strokes.iter().map(|s| s.points[0].x).collect();
        let text = xs.iter().map(|x| format!("{x:.0}")).collect::<Vec<_>>().join(",");
        self.seen.borrow_mut().push(xs);
        Some(text)
    }
}

/// Recognises a line with two or more strokes as "word" and refuses a single stroke.
struct TwoOrMore;

impl Recognizer for TwoOrMore {
    fn recognize(&self, strokes: &[&Stroke]) -> Option<String> {
        (strokes.len() >= 2).then(|| "word".to_string())
    }
}

#[test]
fn strokes_on_three_bands_become_three_lines_read_top_to_bottom() {
    let mut d = Drawing::new();
    d.push(stroke(10.0, 200.0, 40.0, 220.0));
    d.push(stroke(10.0, 10.0, 40.0, 30.0));
    d.push(stroke(10.0, 100.0, 40.0, 120.0));
    let lines = segment_lines(&d);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines.iter().map(|l| l.strokes.clone()).collect::<Vec<_>>(), vec![vec![1], vec![2], vec![0]]);
    assert!(lines.windows(2).all(|w| w[0].bounds.y < w[1].bounds.y));
}

#[test]
fn a_line_box_covers_all_of_its_strokes() {
    let mut d = Drawing::new();
    d.push(stroke(100.0, 10.0, 120.0, 30.0));
    d.push(stroke(10.0, 12.0, 30.0, 28.0));
    let lines = segment_lines(&d);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].bounds, Rect { x: 10.0, y: 10.0, w: 110.0, h: 20.0 });
}

#[test]
fn a_tall_stroke_joins_a_line_it_covers_by_half_of_the_shorter_height() {
    let mut d = Drawing::new();
    d.push(stroke(0.0, 0.0, 10.0, 10.0));
    d.push(stroke(20.0, 2.0, 30.0, 42.0));
    let lines = segment_lines(&d);
    assert_eq!(lines.len(), 1, "an overlap of 8 is at least 5, half of the shorter stroke");
    assert_eq!(lines[0].bounds.h, 42.0);
}

#[test]
fn a_stroke_with_no_points_does_not_make_a_line() {
    let mut d = Drawing::new();
    d.push(Stroke { ink: Ink { kind: InkKind::Pen, color: 0, width: 1.0 }, points: Vec::new() });
    assert!(segment_lines(&d).is_empty());
}

#[test]
fn the_recogniser_is_handed_the_strokes_of_a_line_from_left_to_right() {
    let mut d = Drawing::new();
    d.push(stroke(50.0, 0.0, 60.0, 10.0));
    d.push(stroke(5.0, 1.0, 15.0, 9.0));
    d.push(stroke(25.0, 2.0, 35.0, 8.0));
    let recognizer = Positions { seen: RefCell::new(Vec::new()) };
    let out = recognize_lines(&d, &recognizer);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].1, "5,25,50");
    assert_eq!(recognizer.seen.borrow().as_slice(), &[vec![5.0, 25.0, 50.0]]);
}

#[test]
fn lines_the_recogniser_refuses_are_left_out_of_the_result() {
    let mut d = Drawing::new();
    d.push(stroke(0.0, 0.0, 10.0, 10.0));
    d.push(stroke(0.0, 50.0, 10.0, 60.0));
    d.push(stroke(20.0, 52.0, 30.0, 58.0));
    let out = recognize_lines(&d, &TwoOrMore);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, Rect { x: 0.0, y: 50.0, w: 30.0, h: 10.0 });
    assert_eq!(out[0].1, "word");
}
