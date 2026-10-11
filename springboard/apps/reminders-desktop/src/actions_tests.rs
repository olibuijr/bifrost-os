//! Tests for duplicate, clear completed and swipe-right completion. Child module of `app`, so it
//! can drive the private state the same way the touch and keyboard paths do.
use super::*;
use std::path::PathBuf;

fn test_ui() -> Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

fn app(name: &str) -> (Reminders, Ui, PathBuf) {
    let ui = test_ui();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/test-reminders-actions-{}-{name}/reminders.tsv",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    let app = Reminders::new(&ui, 1200, 1920, path.clone(), Wall { unix: 1_791_073_380, offset: 0, at: 0.0 });
    (app, ui, path)
}

fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

fn add(app: &mut Reminders, ui: &Ui, text: &str, t: f64) {
    app.activate(ui, Target::New, t);
    app.key(ui, Out::Text(text.into()), t + 1.0);
    app.activate(ui, Target::Save, t + 2.0);
}

/// Centre of the visible row showing `title`.
fn row_of(app: &Reminders, ui: &Ui, title: &str) -> (f32, f32, f32, f32) {
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r, Row::Item { idx, .. } if app.data.items[*idx].title == title))
        .expect("row shown");
    // rows are laid out in the list pane; window events arrive offset by the sidebar (`x0`)
    let (x, y, w, h) = app.row_rect(ui, row);
    (x + app.x0, y, w, h)
}

fn titles(app: &Reminders) -> Vec<String> {
    app.data.items.iter().map(|r| r.title.clone()).collect()
}

#[test]
fn duplicate_places_an_incomplete_copy_after_the_original() {
    let (mut app, ui, path) = app("duplicate");
    add(&mut app, &ui, "Buy milk", 0.0);
    add(&mut app, &ui, "Call mum", 10.0);
    let first = app.data.items[0].id;
    app.open_editor(&ui, Some(first), 20.0);
    app.activate(&ui, Target::Duplicate, 21.0);
    assert!(app.editor.is_none(), "editor closes after the copy");
    assert_eq!(titles(&app), ["Buy milk", "Buy milk", "Call mum"]);
    assert_ne!(app.data.items[1].id, first, "the copy has its own id");
    assert!(!app.data.items[1].completed);
    cleanup(&path);
}

#[test]
fn clear_completed_removes_the_open_lists_done_items_and_undo_restores_them() {
    let (mut app, ui, path) = app("clear-completed");
    let list = app.data.lists[0].id;
    app.view = View::List(list);
    add(&mut app, &ui, "Done one", 0.0);
    add(&mut app, &ui, "Open", 10.0);
    add(&mut app, &ui, "Done two", 20.0);
    for title in ["Done one", "Done two"] {
        let id = app.data.items.iter().find(|r| r.title == title).unwrap().id;
        let mut next = app.data.clone();
        assert!(model::toggle_complete(&mut next.items, id, app.today).is_some());
        assert!(app.commit(next));
    }
    app.activate(&ui, Target::OptClearCompleted, 30.0);
    assert_eq!(titles(&app), ["Open"]);
    assert!(app.undo.is_some(), "undo bar offered");
    app.activate(&ui, Target::Undo, 31.0);
    assert_eq!(app.data.items.len(), 3, "undo restores the removed reminders");
    cleanup(&path);
}

#[test]
fn swipe_right_past_the_threshold_completes_and_a_short_swipe_snaps_back() {
    let (mut app, ui, path) = app("swipe-right");
    add(&mut app, &ui, "Water plants", 0.0);
    add(&mut app, &ui, "Feed cat", 10.0);
    let (x, y, w, h) = row_of(&app, &ui, "Water plants");
    let (sx, sy) = (x + w / 2.0, y + h / 2.0);
    app.touch(&ui, Event::Down(sx, sy), 100.0);
    app.touch(&ui, Event::Move(sx + ui.pt(30.0), sy), 120.0);
    app.touch(&ui, Event::Move(sx + ui.pt(140.0), sy), 140.0);
    app.touch(&ui, Event::Up(sx + ui.pt(140.0), sy), 160.0);
    assert!(app.data.items.iter().find(|r| r.title == "Water plants").unwrap().completed);
    assert!(app.swipe.is_none());

    let (x, y, w, h) = row_of(&app, &ui, "Feed cat");
    let (sx, sy) = (x + w / 2.0, y + h / 2.0);
    app.touch(&ui, Event::Down(sx, sy), 200.0);
    app.touch(&ui, Event::Move(sx + ui.pt(30.0), sy), 220.0);
    app.touch(&ui, Event::Up(sx + ui.pt(30.0), sy), 240.0);
    assert!(app.swipe.is_none(), "a short swipe closes");
    assert!(!app.data.items.iter().find(|r| r.title == "Feed cat").unwrap().completed);
    cleanup(&path);
}

#[test]
fn arrow_keys_select_rows_and_enter_and_space_act_on_the_selection() {
    let (mut app, ui, path) = app("keys");
    add(&mut app, &ui, "First", 0.0);
    add(&mut app, &ui, "Second", 10.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    let shown: Vec<u64> = app
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Item { idx, .. } => Some(app.data.items[*idx].id),
            Row::Header(_) => None,
        })
        .collect();
    let (first, second) = (shown[0], shown[1]);
    app.key(&ui, Out::Named("Down"), 20.0);
    assert_eq!(app.selected, Some(first), "Down from nothing selects the first row");
    app.key(&ui, Out::Named("Down"), 21.0);
    assert_eq!(app.selected, Some(second));
    app.key(&ui, Out::Named("Down"), 22.0);
    assert_eq!(app.selected, Some(second), "stops at the last row");
    app.key(&ui, Out::Named("Up"), 23.0);
    assert_eq!(app.selected, Some(first));
    app.key(&ui, Out::Enter, 24.0);
    assert_eq!(app.editor.as_ref().and_then(|e| e.id), Some(first), "Enter opens the selected reminder");
    app.activate(&ui, Target::Cancel, 25.0);
    app.key(&ui, Out::Text(" ".into()), 26.0);
    assert!(app.data.items.iter().find(|r| r.id == first).unwrap().completed, "Space completes it");
    cleanup(&path);
}

#[test]
fn cmd_chords_start_a_new_reminder_and_search_on_the_list() {
    use bifrost_proto::{CHORD_META, CHORD_SHIFT};
    let (mut app, ui, path) = app("chords");
    app.screen = Screen::Items;
    assert!(app.chord(&ui, 49, CHORD_META, 0.0), "Cmd+N");
    assert!(app.editor.is_some(), "a new reminder opens its editor");
    app.activate(&ui, Target::Cancel, 1.0);
    assert!(app.chord(&ui, 33, CHORD_META, 2.0), "Cmd+F");
    assert!(!app.chord(&ui, 33, CHORD_META | CHORD_SHIFT, 3.0), "Cmd+Shift+F is not bound");
    assert!(!app.chord(&ui, 30, CHORD_META, 4.0), "Cmd+A is not bound here");
    cleanup(&path);
}

#[test]
fn cmd_backspace_deletes_the_selected_reminder_and_escape_cancels_the_editor() {
    use bifrost_proto::CHORD_META;
    let (mut app, ui, path) = app("delete-escape");
    add(&mut app, &ui, "Doomed", 0.0);
    add(&mut app, &ui, "Kept", 10.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    assert!(!app.chord(&ui, 14, CHORD_META, 20.0), "nothing selected, nothing deleted");
    app.key(&ui, Out::Named("Down"), 21.0);
    assert!(app.chord(&ui, 14, CHORD_META, 22.0), "Cmd+Backspace");
    assert_eq!(titles(&app), ["Kept"]);
    app.key(&ui, Out::Named("Down"), 23.0);
    app.key(&ui, Out::Enter, 24.0);
    assert!(app.editor.is_some());
    assert!(app.escape(&ui, 25.0), "Escape cancels the editor");
    assert!(app.editor.is_none());
    assert!(!app.escape(&ui, 26.0), "Escape on the list does nothing");
    cleanup(&path);
}

#[test]
fn undo_bar_names_the_change_and_lasts_at_least_five_seconds() {
    let (mut app, ui, path) = app("undo-labels");
    add(&mut app, &ui, "Milk", 0.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    app.activate(&ui, Target::SwipeFlag(0), 0.0);
    assert_eq!(app.undo.as_ref().unwrap().label, "Flagged \"Milk\"");
    app.activate(&ui, Target::SwipeFlag(0), 100.0);
    assert_eq!(app.undo.as_ref().unwrap().label, "Unflagged \"Milk\"");
    assert!(app.undo.as_ref().unwrap().until - 100.0 >= 5000.0, "the bar stays at least five seconds");
    cleanup(&path);
}

#[test]
fn new_reminders_from_a_smart_list_go_to_the_chosen_default_list() {
    let (mut app, ui, path) = app("default-list");
    app.data.lists.push(model::ListDef { id: 2, name: "Work".into(), color: 1, icon: 0 });
    app.screen = Screen::Items;
    app.view = View::Smart(Smart::All);
    app.open_editor(&ui, None, 0.0);
    assert_eq!(app.editor.as_ref().unwrap().draft.list, model::DEFAULT_LIST, "automatic is the first list");
    app.editor = None;
    app.activate(&ui, Target::OptDefaultList(Some(2)), 1.0);
    app.open_editor(&ui, None, 2.0);
    assert_eq!(app.editor.as_ref().unwrap().draft.list, 2, "the chosen list");
    app.editor = None;
    app.view = View::List(1);
    app.open_editor(&ui, None, 3.0);
    assert_eq!(app.editor.as_ref().unwrap().draft.list, 1, "a list view keeps its own list");
    cleanup(&path);
}

#[test]
fn export_list_is_a_markdown_checklist_with_subtasks_and_due_days() {
    use springboard::shell::clock::tz::days_from_civil;
    let day = days_from_civil(2026, 10, 8);
    let mut milk = model::Reminder::new(1, "  Milk   2%  ");
    milk.due = Some(day);
    milk.time = Some(9 * 60 + 5);
    let mut bread = model::Reminder::new(2, "Bread");
    bread.completed = true;
    let mut rye = model::Reminder::new(3, "Rye");
    rye.parent = Some(2);
    let mut other = model::Reminder::new(4, "Elsewhere");
    other.list = 2;
    let data = model::Data {
        lists: vec![model::ListDef { id: 1, name: "Groceries".into(), color: 0, icon: 0 }],
        items: vec![milk, bread, rye, other],
    };
    assert_eq!(
        model::export_markdown(&data, 1),
        "# Groceries\n\n- [ ] Milk 2% (2026-10-08 09:05)\n- [x] Bread\n  - [ ] Rye\n"
    );
}

