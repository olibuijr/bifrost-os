//! Tests for Read Aloud, Look Up and Translate (`narrate.rs`). Expected values are worked out by hand in the comments.

use super::narrate::*;
use super::store::Span;
use bifrost_app::controls::language::Language;
use bifrost_app::controls::speech::Utterance;

#[test]
fn sentences_end_at_stops_followed_by_space() {
    // "One. Two!  Three": the stop after "One" at index 3 and the "!" at index 8 are followed by a space
    let got = sentence_spans("One. Two!  Three", 0, 0);
    assert_eq!(
        got,
        vec![
            (Span { block: 0, offset: 0, len: 4 }, "One.".to_string()),
            (Span { block: 0, offset: 5, len: 4 }, "Two!".to_string()),
            (Span { block: 0, offset: 11, len: 5 }, "Three".to_string()),
        ]
    );
}

#[test]
fn a_decimal_point_does_not_end_a_sentence() {
    // the "." at index 1 is followed by "5", not a space
    assert_eq!(sentence_spans("3.5 ok", 2, 0), vec![(Span { block: 2, offset: 0, len: 6 }, "3.5 ok".to_string())]);
}

#[test]
fn reading_starts_at_the_given_char() {
    // "Alpha. Beta.": from char 7 is "Beta."
    assert_eq!(sentence_spans("Alpha. Beta.", 0, 7), vec![(Span { block: 0, offset: 7, len: 5 }, "Beta.".to_string())]);
}

#[test]
fn an_ellipsis_ends_a_sentence_and_blank_text_gives_none() {
    // "Wait… go": the ellipsis at index 4 is followed by a space; "go" is chars 6..8
    assert_eq!(
        sentence_spans("Wait\u{2026} go", 0, 0),
        vec![(Span { block: 0, offset: 0, len: 5 }, "Wait\u{2026}".to_string()), (Span { block: 0, offset: 6, len: 2 }, "go".to_string())]
    );
    assert!(sentence_spans("   ", 0, 0).is_empty());
    // a start past the end of the text has nothing to read
    assert!(sentence_spans("ab", 0, 9).is_empty());
}

#[test]
fn chapters_end_at_the_next_table_of_contents_start() {
    assert_eq!(chapter_end(&[0, 5, 9], 3, 20), 5);
    assert_eq!(chapter_end(&[0, 5], 6, 20), 20);
    assert_eq!(chapter_end(&[], 0, 4), 4);
}

#[test]
fn a_sentence_takes_its_reading_time_with_a_floor() {
    // default rate 0.5 reads 60 + 180 * 0.5 = 150 words a minute
    // four words: 4 * 60000 / 150 = 1600 ms
    assert_eq!(duration_ms(&Utterance::new("one two three four")), 1600.0);
    // one word at 150 wpm is 400 ms, under the 600 ms floor
    assert_eq!(duration_ms(&Utterance::new("hi")), 600.0);
    // rate 1.0 reads 240 wpm: 4 * 60000 / 240 = 1000 ms
    assert_eq!(duration_ms(&Utterance::new("one two three four").with_rate(1.0)), 1000.0);
    // rate 0.0 reads 60 wpm: 4 * 60000 / 60 = 4000 ms
    assert_eq!(duration_ms(&Utterance::new("one two three four").with_rate(0.0)), 4000.0);
}

fn two_sentences() -> Vec<(Span, String)> {
    // "one two three four" is 1600 ms; "five six" is 2 * 60000 / 150 = 800 ms (above the 600 ms floor)
    vec![(Span { block: 0, offset: 0, len: 18 }, "one two three four".to_string()), (Span { block: 0, offset: 19, len: 8 }, "five six".to_string())]
}

#[test]
fn an_empty_chapter_is_not_read() {
    assert!(Narration::start(Vec::new(), 0.0).is_none());
}

#[test]
fn the_highlight_moves_when_each_sentence_is_due() {
    let mut n = Narration::start(two_sentences(), 0.0).expect("two sentences");
    let first = Span { block: 0, offset: 0, len: 18 };
    let second = Span { block: 0, offset: 19, len: 8 };
    assert_eq!(n.current(), Some(first));
    assert_eq!(n.wake(), Some(1600.0));
    assert_eq!(n.tick(1000.0), Step::Same);
    assert_eq!(n.tick(1600.0), Step::Moved(second));
    assert_eq!(n.current(), Some(second));
    // the second ends 800 ms after the first: 1600 + 800 = 2400
    assert_eq!(n.wake(), Some(2400.0));
    assert_eq!(n.tick(2399.0), Step::Same);
    assert_eq!(n.tick(2400.0), Step::Done);
}

#[test]
fn pausing_keeps_the_time_left_on_the_sentence() {
    let mut n = Narration::start(two_sentences(), 0.0).expect("two sentences");
    assert!(n.pause(1000.0));
    // 1600 - 1000 = 600 ms left
    assert!(n.is_paused());
    assert_eq!(n.wake(), None);
    assert_eq!(n.tick(5000.0), Step::Same);
    assert_eq!(n.current(), Some(Span { block: 0, offset: 0, len: 18 }));
    assert!(n.resume(5000.0));
    // 5000 + 600 = 5600
    assert_eq!(n.wake(), Some(5600.0));
    assert_eq!(n.tick(5599.0), Step::Same);
    assert_eq!(n.tick(5600.0), Step::Moved(Span { block: 0, offset: 19, len: 8 }));
}

#[test]
fn pause_and_resume_only_work_from_the_right_state() {
    let mut n = Narration::start(two_sentences(), 0.0).expect("two sentences");
    assert!(!n.resume(0.0), "resume needs a paused reading");
    assert!(n.pause(0.0));
    assert!(!n.pause(0.0), "a paused reading cannot pause again");
}

#[test]
fn a_late_tick_catches_up_over_every_due_sentence() {
    // at 5000 ms both sentences (due at 1600 and 2400) have passed, so the chapter is done in one tick
    let mut n = Narration::start(two_sentences(), 0.0).expect("two sentences");
    assert_eq!(n.tick(5000.0), Step::Done);
}

#[test]
fn look_up_names_the_word_and_its_language() {
    assert_eq!(
        look_up_notice("cat", Some(Language::English)),
        "Look Up \u{201c}cat\u{201d} (English): no dictionary is installed on this PC, so there is no definition."
    );
    assert_eq!(
        look_up_notice("cat", None),
        "Look Up \u{201c}cat\u{201d}: no dictionary is installed on this PC, so there is no definition."
    );
}

#[test]
fn a_long_word_is_shortened_to_forty_chars() {
    let long = "a".repeat(50);
    let notice = look_up_notice(&long, None);
    assert!(notice.starts_with(&format!("Look Up \u{201c}{}\u{2026}\u{201d}", "a".repeat(40))), "{notice}");
}

#[test]
fn translate_reports_the_missing_language_pack() {
    // no packs ship, so an identified Icelandic sentence is not a supported source
    assert_eq!(translate_notice(Some(Language::Icelandic)), "Translate from Icelandic: The source language is not supported.");
    assert_eq!(translate_notice(None), "Translate: The source language could not be identified.");
}

#[test]
fn language_names_are_english() {
    assert_eq!(language_name(Language::Icelandic), "Icelandic");
    assert_eq!(language_name(Language::English), "English");
}
