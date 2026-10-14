//! Tests for the Shortcuts additions of 1.9.0: per-shortcut menu keys, the Repeat with Each and Stop steps, starter
//! categories, the variable names a picker offers, and the glyph and key fields of the list file.

use super::flow::{self, Done, Host};
use super::*;

/// A host that performs every step it is given as done, and remembers their titles.
#[derive(Default)]
struct Plain {
    asked: Vec<String>,
}

impl Host for Plain {
    fn perform(&mut self, action: &Action, _input: &str) -> Result<Done, String> {
        self.asked.push(action.title().to_string());
        Ok(Done::said("done"))
    }
}

fn named(name: &str) -> Shortcut {
    Shortcut::new(name, 0, Vec::new())
}

fn texts(steps: &[flow::Step], prefix: &str) -> Vec<Result<String, String>> {
    steps.iter().filter(|s| s.line.starts_with(prefix)).map(|s| s.outcome.clone()).collect()
}

#[test]
fn repeat_with_each_runs_once_per_line_of_the_previous_output() {
    let list = vec![
        Action::Text("alpha\n\nbeta\ngamma".into()),
        Action::RepeatEach,
        Action::Text("{Item}!".into()),
        Action::EndRepeat,
    ];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(steps[1].outcome, Ok("3 lines".to_string()), "blank lines are not passes");
    assert_eq!(
        texts(&steps, "Text: {Item}"),
        vec![Ok("alpha!".to_string()), Ok("beta!".to_string()), Ok("gamma!".to_string())]
    );
    assert_eq!(steps.len(), 5, "the opener, three passes and the first text");
}

#[test]
fn repeat_with_each_on_an_empty_output_runs_nothing() {
    let list = vec![Action::RepeatEach, Action::Dnd(true), Action::EndRepeat];
    let mut host = Plain::default();
    let steps = flow::run(&list, &mut host);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].outcome, Ok("0 lines".to_string()));
    assert!(host.asked.is_empty());
}

#[test]
fn repeat_with_each_passes_at_most_the_repeat_limit() {
    let text: String = (0..30).map(|n| format!("line {n}\n")).collect();
    let list = vec![Action::Text(text), Action::RepeatEach, Action::Text("{Item}".into()), Action::EndRepeat];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(texts(&steps, "Text: {Item}").len(), MAX_REPEAT as usize);
    assert_eq!(steps[1].outcome, Ok(format!("{} lines", MAX_REPEAT)));
}

#[test]
fn the_item_variable_holds_the_line_after_the_block() {
    let list = vec![
        Action::Text("one\ntwo".into()),
        Action::RepeatEach,
        Action::EndRepeat,
        Action::Text("last {Item}".into()),
    ];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(steps.last().unwrap().outcome, Ok("last two".to_string()), "the Item variable keeps the last line");
}

#[test]
fn stop_ends_the_shortcut_and_the_rest_is_not_run() {
    let list = vec![Action::Text("a".into()), Action::Stop, Action::Text("b".into())];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[1].outcome, Err("the shortcut stopped here".to_string()));
    assert_eq!(steps[2].line, "Text: b");
    assert!(steps[2].outcome.is_err());
}

#[test]
fn a_stop_in_a_branch_not_taken_does_nothing() {
    let list = vec![
        Action::Text("no".into()),
        Action::If(Cond::Equals("yes".into())),
        Action::Stop,
        Action::EndIf,
        Action::Text("after".into()),
    ];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(steps.last().unwrap().outcome, Ok("after".to_string()));
    assert!(steps.iter().all(|s| s.line != "Stop"));
}

#[test]
fn a_stop_inside_a_repeat_stops_every_pass() {
    let list = vec![Action::Repeat(3), Action::Stop, Action::EndRepeat, Action::Text("z".into())];
    let steps = flow::run(&list, &mut Plain::default());
    assert_eq!(steps.iter().filter(|s| s.line == "Stop").count(), 1, "only the first pass reaches the Stop");
    assert!(steps.last().unwrap().outcome.is_err());
}

#[test]
fn the_new_blocks_nest_and_remove_as_one() {
    let list = vec![Action::RepeatEach, Action::Text("a".into()), Action::EndRepeat];
    assert_eq!(flow::depths(&list), vec![0, 1, 0]);
    let mut edited = vec![Action::RepeatEach, Action::Text("a".into()), Action::EndRepeat, Action::Stop];
    assert_eq!(flow::remove_step(&mut edited, 0), 3);
    assert_eq!(edited, vec![Action::Stop]);
}

#[test]
fn the_add_menu_builds_the_new_blocks_and_stop() {
    assert_eq!(KINDS[19], "Repeat with Each");
    assert_eq!(KINDS[20], "Stop");
    assert_eq!(Action::block(19), vec![Action::RepeatEach, Action::EndRepeat]);
    assert_eq!(Action::block(20), vec![Action::Stop]);
    assert!(!Action::Stop.adjustable() && !Action::RepeatEach.adjustable());
    assert_eq!(Action::Stop.value(), "");
}

#[test]
fn the_new_steps_round_trip_through_the_file() {
    let list = vec![Shortcut::new("Sweep", 2, vec![Action::RepeatEach, Action::Text("x".into()), Action::EndRepeat, Action::Stop])];
    assert_eq!(parse(&serialize(&list)), list);
}

#[test]
fn a_menu_key_round_trips_with_the_other_extra_fields() {
    let mut list = vec![named("Keyed"), named("Plain")];
    list[0].key = 7;
    let text = serialize(&list);
    assert!(text.lines().next().unwrap().ends_with("\t\t0\t0\t7"), "{text}");
    assert!(text.lines().nth(1).unwrap().ends_with("\t"), "no key and no extra fields on a plain shortcut: {text}");
    assert_eq!(parse(&text), list);
}

#[test]
fn a_menu_key_outside_one_to_nine_is_dropped() {
    let l = parse("A\t2\t\t\t\t\t12\nB\t2\t\t\t\t\t0\n");
    assert_eq!(l[0].key, 0);
    assert_eq!(l[1].key, 0);
}

#[test]
fn setting_a_key_moves_it_from_the_shortcut_that_held_it() {
    let mut list = vec![named("A"), named("B"), named("C")];
    set_key(&mut list, 0, 3).unwrap();
    set_key(&mut list, 2, 3).unwrap();
    assert_eq!(list.iter().map(|s| s.key).collect::<Vec<u8>>(), vec![0, 0, 3], "one key, one shortcut");
    set_key(&mut list, 2, 0).unwrap();
    assert_eq!(list[2].key, 0);
    assert!(set_key(&mut list, 0, MAX_KEY + 1).is_err());
    assert!(set_key(&mut list, 9, 1).is_err());
    assert_eq!(list[0].key, 0, "a refused key changes nothing");
}

#[test]
fn the_key_row_steps_through_none_then_one_to_nine() {
    assert_eq!(next_key(0), 1);
    assert_eq!(next_key(MAX_KEY - 1), MAX_KEY);
    assert_eq!(next_key(MAX_KEY), 0);
}

#[test]
fn the_first_defaults_hold_the_first_menu_keys() {
    let keys: Vec<u8> = defaults().iter().map(|s| s.key).collect();
    assert_eq!(keys, vec![1, 2, 3, 4]);
}

#[test]
fn a_copy_and_an_import_take_no_menu_key() {
    let mut list = defaults();
    let at = duplicate(&mut list, 0).unwrap();
    assert_eq!(list[at].key, 0);
    assert_eq!(list[0].key, 1, "the original keeps its key");
    let text = export(&defaults());
    let mut dst = vec![named("Other")];
    import(&mut dst, &text).unwrap();
    assert!(dst.iter().skip(1).all(|s| s.key == 0));
}

#[test]
fn the_variable_names_start_with_the_output_and_list_each_name_once() {
    let actions = vec![
        Action::SetVariable("Name".into()),
        Action::Text("x".into()),
        Action::SetVariable("Name".into()),
        Action::SetVariable("City".into()),
    ];
    assert_eq!(variable_names(&actions), vec!["Output", "Name", "City"]);
    assert_eq!(variable_names(&[]), vec!["Output"]);
}

#[test]
fn every_starter_belongs_to_a_gallery_category() {
    for s in starters() {
        assert!(STARTER_CATEGORIES.contains(&s.category), "{} has no heading", s.shortcut.name);
    }
    for c in STARTER_CATEGORIES {
        assert!(starters().iter().any(|s| s.category == c), "{c} is empty");
    }
}

#[test]
fn the_gallery_has_fourteen_starters_and_keeps_the_first_ten_in_place() {
    assert_eq!(starters().len(), 14);
    assert_eq!(starters()[7].shortcut.name, "Wind Down");
    assert_eq!(starters()[10].shortcut.name, "Sign Off");
}

#[test]
fn a_dragged_step_lands_on_the_index_it_is_dropped_at() {
    let mut a = vec![Action::Wifi(true), Action::Screenshot, Action::Timer(5), Action::Dnd(true)];
    assert_eq!(move_step(&mut a, 3, 0), Some(0));
    assert_eq!(a[0], Action::Dnd(true));
    assert_eq!(move_step(&mut a, 0, 2), Some(2));
    assert_eq!(a, vec![Action::Wifi(true), Action::Screenshot, Action::Dnd(true), Action::Timer(5)]);
    assert_eq!(move_step(&mut a, 1, 1), None, "a step dropped on itself stays");
    assert_eq!(move_step(&mut a, 1, 9), None, "a drop past the end is refused");
}

#[test]
fn block_openers_ends_and_otherwises_never_move() {
    let mut b = vec![Action::Repeat(2), Action::Dnd(true), Action::EndRepeat];
    assert_eq!(move_step(&mut b, 0, 2), None);
    assert_eq!(move_step(&mut b, 2, 0), None);
    let mut c = vec![Action::If(Cond::Empty), Action::Otherwise, Action::EndIf];
    assert_eq!(move_step(&mut c, 1, 0), None);
    assert_eq!(move_step(&mut c, 0, 1), None);
    assert_eq!(move_step(&mut b, 9, 0), None, "no step there");
}

#[test]
fn every_glyph_is_a_distinct_symbol() {
    let mut seen = std::collections::HashSet::new();
    for g in GLYPHS.iter().filter(|g| !g.is_empty()) {
        assert!(seen.insert(*g), "{g} twice");
    }
    assert_eq!(GLYPHS.len(), 21);
}
