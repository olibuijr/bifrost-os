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
fn a_deleted_reminder_waits_in_recently_deleted_and_restores_from_there() {
    let (mut app, ui, path) = app("recently-deleted");
    add(&mut app, &ui, "Buy milk", 0.0);
    let id = app.data.items[0].id;
    assert!(app.show_item(id));
    assert!(app.delete_selected(&ui, 10.0));
    assert!(app.data.items.is_empty());
    assert_eq!(app.kept_count, 1, "the sidebar count sees the kept reminder");
    app.activate(&ui, Target::Deleted, 20.0);
    let kept: Vec<String> = app.deleted.clone().expect("the page is open").into_iter().map(|r| r.title).collect();
    assert_eq!(kept, vec!["Buy milk".to_string()]);
    // the page draws with its row and restore link
    let mut buf = vec![0u32; 1200 * 1920];
    app.draw(&mut bifrost_app::kit::Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 30.0);
    app.activate(&ui, Target::Restore(id), 40.0);
    assert_eq!(titles(&app), vec!["Buy milk".to_string()]);
    assert!(app.deleted.as_ref().expect("still on the page").is_empty());
    assert_eq!(app.kept_count, 0);
    cleanup(&path);
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
            _ => None,
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
    app.data.lists.push(model::ListDef::new(2, "Work", 1, 0));
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
        lists: vec![model::ListDef::new(1, "Groceries", 0, 0)],
        items: vec![milk, bread, rye, other],
        groups: vec![],
        templates: vec![],
        smarts: vec![],
    };
    assert_eq!(
        model::export_markdown(&data, 1),
        "# Groceries\n\n- [ ] Milk 2% (2026-10-08 09:05)\n- [x] Bread\n  - [ ] Rye\n"
    );
}

#[test]
fn first_use_alert_tip_shows_in_details_until_a_due_date_is_set() {
    let (app, ui, path) = app("alert-tip");
    let mut app = app.with_tips(bifrost_app::controls::tips::configure(&[]).unwrap());
    let has_tip = |app: &Reminders| app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Tip);
    // A new reminder without a date shows the tip under the Due row.
    app.activate(&ui, Target::New, 0.0);
    assert!(has_tip(&app));
    // Setting a date is what the tip asks for: it is dismissed and does not come back when the date is cleared.
    app.activate(&ui, Target::Due, 1.0);
    assert!(!has_tip(&app));
    app.activate(&ui, Target::Due, 2.0);
    assert!(!has_tip(&app));
    cleanup(&path);
}

#[test]
fn tapping_the_alert_tip_dismisses_it() {
    let (app, ui, path) = app("alert-tip-tap");
    let mut app = app.with_tips(bifrost_app::controls::tips::configure(&[]).unwrap());
    app.activate(&ui, Target::New, 0.0);
    app.activate(&ui, Target::Tip, 1.0);
    assert!(!app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Tip));
    cleanup(&path);
}

#[test]
fn without_a_tip_store_the_details_form_has_no_tip_row() {
    let (mut app, ui, path) = app("alert-tip-none");
    app.activate(&ui, Target::New, 0.0);
    assert!(!app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Tip));
    cleanup(&path);
}

#[test]
fn edit_undo_and_redo_step_through_saves_and_completions() {
    let (mut app, ui, path) = app("edit-undo");
    add(&mut app, &ui, "Milk", 0.0);
    assert_eq!(app.edit_titles(), ("Undo New Reminder".to_string(), "Redo".to_string()));
    let id = app.data.items[0].id;
    app.toggle_item(&ui, id);
    assert!(app.data.items[0].completed);
    assert_eq!(app.edit_titles(), ("Undo Mark as Completed".to_string(), "Redo".to_string()));
    assert!(app.step_history(&ui, false), "undo the completion");
    assert!(!app.data.items[0].completed, "undo marks it open again");
    assert_eq!(app.edit_titles().1, "Redo Mark as Completed");
    assert!(app.step_history(&ui, true), "redo the completion");
    assert!(app.data.items[0].completed, "redo completes it again");
    assert!(app.step_history(&ui, false));
    assert!(app.step_history(&ui, false), "the save is the next step back");
    assert!(titles(&app).is_empty(), "undoing the save removes the new reminder");
    assert!(!app.step_history(&ui, false), "nothing left to undo");
    assert!(app.step_history(&ui, true));
    assert_eq!(titles(&app), ["Milk"], "redo brings the reminder back");
    app.open_editor(&ui, Some(id), 50.0);
    assert!(!app.step_history(&ui, false), "no undo while an editor holds a draft");
    cleanup(&path);
}

#[test]
fn a_new_change_after_an_undo_drops_the_redo_steps() {
    let (mut app, ui, path) = app("edit-redo-drop");
    add(&mut app, &ui, "Bread", 0.0);
    let id = app.data.items[0].id;
    app.toggle_item(&ui, id);
    assert!(app.step_history(&ui, false));
    assert_eq!(app.edit_titles().1, "Redo Mark as Completed");
    app.toggle_item(&ui, id);
    assert!(app.data.items[0].completed, "the new completion stands");
    assert!(!app.history.can_redo(), "the undone step is gone");
    cleanup(&path);
}


// ---- menu commands, sections, groups, templates, import and intents --------------------------------------------

fn id_of(app: &Reminders, title: &str) -> u64 {
    app.data.items.iter().find(|r| r.title == title).unwrap().id
}

/// The rows as text: titles (subtasks indented), `[section]` headers and `<group>` headers.
fn shape(app: &Reminders) -> Vec<String> {
    app.rows
        .iter()
        .map(|r| match r {
            Row::Item { idx, depth } => format!("{}{}", if *depth == 1 { "  " } else { "" }, app.data.items[*idx].title),
            Row::Section { name, .. } => format!("[{name}]"),
            Row::Header(h) => format!("<{h}>"),
        })
        .collect()
}

fn render(app: &mut Reminders, ui: &Ui) {
    use bifrost_app::kit::Surface;
    let mut buf = vec![0u32; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, 1000.0);
}

#[test]
fn mark_due_date_as_sets_the_selected_reminder_and_undoes_in_one_step() {
    use organize::MarkDue;
    let (mut app, ui, path) = app("mark-due");
    add(&mut app, &ui, "Pay rent", 0.0);
    let id = id_of(&app, "Pay rent");
    let today = app.today;
    assert!(!app.mark_selected_due(&ui, MarkDue::Today), "nothing is selected");
    app.selected = Some(id);
    assert!(app.mark_selected_due(&ui, MarkDue::Tomorrow));
    assert_eq!(app.data.items[0].due, Some(today + 1));
    assert!(!app.mark_selected_due(&ui, MarkDue::Tomorrow), "already due that day");
    assert_eq!(app.edit_titles().0, "Undo Mark Due Date as Tomorrow");
    assert!(app.step_history(&ui, false));
    assert_eq!(app.data.items[0].due, None);
    cleanup(&path);
}

#[test]
fn mark_all_overdue_as_today_is_one_undo_step() {
    let (mut app, ui, path) = app("overdue");
    for (i, title) in ["Milk", "Bread", "Eggs"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    let today = app.today;
    let mut next = app.data.clone();
    next.items[0].due = Some(today - 3);
    next.items[1].due = Some(today - 1);
    next.items[2].due = Some(today + 2);
    assert!(app.commit(next));
    let dues = |app: &Reminders| app.data.items.iter().map(|r| r.due).collect::<Vec<_>>();
    assert!(app.has_overdue());
    assert!(app.mark_overdue_today(&ui));
    assert_eq!(dues(&app), [Some(today), Some(today), Some(today + 2)]);
    assert!(!app.has_overdue());
    assert!(!app.mark_overdue_today(&ui), "nothing left to move");
    assert!(app.step_history(&ui, false), "one step brings them all back");
    assert_eq!(dues(&app), [Some(today - 3), Some(today - 1), Some(today + 2)]);
    cleanup(&path);
}

#[test]
fn indent_outdent_and_hide_all_subtasks_work_on_the_selection() {
    let (mut app, ui, path) = app("subtasks");
    add(&mut app, &ui, "Plan trip", 0.0);
    add(&mut app, &ui, "Book hotel", 10.0);
    app.activate(&ui, Target::List(1), 20.0);
    let (plan, hotel) = (id_of(&app, "Plan trip"), id_of(&app, "Book hotel"));
    app.selected = Some(plan);
    assert!(!app.can_indent(), "the first reminder has nothing above it");
    assert!(!app.can_outdent());
    app.selected = Some(hotel);
    assert!(app.can_indent());
    assert!(app.indent_selected(&ui));
    assert_eq!(app.data.items[1].parent, Some(plan));
    assert_eq!(shape(&app), ["Plan trip", "  Book hotel"]);
    assert!(!app.can_indent(), "already a subtask");
    assert!(app.can_outdent());

    assert!(app.has_subtasks() && !app.subtasks_hidden());
    app.toggle_all_subtasks(&ui);
    assert!(app.subtasks_hidden());
    assert_eq!(shape(&app), ["Plan trip"]);
    app.toggle_all_subtasks(&ui);
    assert_eq!(shape(&app), ["Plan trip", "  Book hotel"]);

    assert!(app.outdent_selected(&ui));
    assert_eq!(app.data.items[1].parent, None);
    assert!(!app.outdent_selected(&ui), "a top-level reminder cannot outdent");
    assert!(app.step_history(&ui, false), "Undo Outdent");
    assert_eq!(app.data.items[1].parent, Some(plan));
    cleanup(&path);
}

#[test]
fn creation_dates_are_kept_and_sort_by_creation_date_puts_the_oldest_first() {
    let (mut app, ui, path) = app("created");
    for (i, title) in ["Alpha", "Beta", "Gamma"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    assert!(app.data.items.iter().all(|r| r.created.is_some()), "a new reminder is stamped");
    let mut next = app.data.clone();
    next.items[0].created = Some(300);
    next.items[1].created = Some(100);
    next.items[2].created = None;
    assert!(app.commit(next));
    app.activate(&ui, Target::List(1), 50.0);
    assert_eq!(shape(&app), ["Alpha", "Beta", "Gamma"]);
    app.set_sort(&ui, Sort::Created);
    assert_eq!(app.sort(), Sort::Created);
    assert_eq!(shape(&app), ["Gamma", "Beta", "Alpha"], "no date counts as the oldest");
    assert_eq!(Sort::Created.label(), "Creation Date");
    cleanup(&path);
}

#[test]
fn smart_list_tiles_can_be_hidden_and_a_pinned_list_gets_a_tile() {
    let (mut app, ui, path) = app("tiles");
    // five smart tiles and Recently Deleted
    assert_eq!(app.tiles().len(), 6);
    app.toggle_smart(Smart::Flagged);
    assert!(!app.smart_shown(Smart::Flagged));
    assert_eq!(app.tiles().len(), 5);
    assert!(!app.tiles().contains(&chrome::Tile::Smart(Smart::Flagged)));
    app.toggle_smart(Smart::Flagged);
    assert_eq!(app.tiles().len(), 6);

    assert!(!app.toggle_pin(), "no list is open");
    app.activate(&ui, Target::List(1), 1.0);
    assert!(app.toggle_pin());
    assert!(app.open_list_pinned());
    assert_eq!(app.tiles().last(), Some(&chrome::Tile::List(1)));
    let k = ui.pt(1.0);
    let at = |app: &Reminders, i: usize| {
        let r = app.tile_rect(&ui, i);
        app.sidebar_target(&ui, (r.x + r.w / 2.0) * k, (r.y + r.h / 2.0) * k)
    };
    assert_eq!(at(&app, 6), Some(Target::List(1)), "the pinned tile opens the list");
    assert_eq!(at(&app, 5), Some(Target::Deleted));
    assert_eq!(at(&app, 0), Some(Target::Smart(Smart::Today)));
    let tiles: Vec<_> = app.controls(&ui).into_iter().filter(|n| (100..200).contains(&n.id)).map(|n| n.label).collect();
    assert_eq!(tiles, ["Today", "Scheduled", "All", "Flagged", "Completed", "Recently Deleted", "Reminders"]);
    render(&mut app, &ui);
    assert_eq!(app.edit_titles().0, "Undo Pin List");
    assert!(app.toggle_pin());
    assert_eq!(app.tiles().len(), 6);
    cleanup(&path);
}

#[test]
fn a_tile_name_wider_than_the_tile_wraps_instead_of_being_cut() {
    use bifrost_app::kit::ui::{Weight, MAC_BODY_PT, SPACE_M};
    let (app, ui, path) = app("tile-labels");
    let room = ui.pt(app.tile_rect(&ui, 0).w) - ui.pt(2.0 * SPACE_M);
    let lines = Reminders::tile_label_lines(&ui, "Recently Deleted", room);
    assert_eq!(lines, ["Recently", "Deleted"], "two whole lines, no ellipsis");
    assert!(lines.iter().all(|l| ui.measure(l, MAC_BODY_PT, Weight::Semibold) <= room));
    assert_eq!(Reminders::tile_label_lines(&ui, "Today", room), ["Today"]);
    assert_eq!(Reminders::tile_label_lines(&ui, "Scheduled", room), ["Scheduled"]);
    cleanup(&path);
}

#[test]
fn only_eight_lists_can_be_pinned() {
    let (mut app, ui, path) = app("pin-cap");
    let mut next = app.data.clone();
    for id in 2..=10 {
        next.lists.push(model::ListDef::new(id, format!("L{id}"), 0, 0));
    }
    for l in next.lists.iter_mut().take(MAX_PINNED) {
        l.pinned = true;
    }
    assert!(app.commit(next));
    app.activate(&ui, Target::List(10), 1.0);
    assert!(!app.toggle_pin(), "a ninth pin is refused");
    assert!(app.notice.is_some());
    cleanup(&path);
}

#[test]
fn sections_group_a_list_and_a_header_tap_edits_them() {
    let (mut app, ui, path) = app("sections");
    for (i, title) in ["Milk", "Bread", "Soap"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    assert!(!app.new_section(&ui, 5.0), "needs a list on screen");
    app.activate(&ui, Target::List(1), 6.0);
    assert!(app.new_section(&ui, 7.0));
    assert_eq!(app.screen, Screen::ListEditor);
    assert_eq!(app.list_edit_title(), "New Section");
    app.key(&ui, Out::Text("Food".into()), 8.0);
    app.activate(&ui, Target::Save, 9.0);
    assert_eq!(app.screen, Screen::Items);
    assert_eq!(shape(&app), ["Milk", "Bread", "Soap", "[Food]"]);

    app.selected = Some(id_of(&app, "Bread"));
    assert!(app.can_move_to_section());
    let food = app.open_list_sections()[0].0;
    assert!(!app.move_selected_to_section(&ui, Some(food + 5)), "an unknown section");
    assert!(app.move_selected_to_section(&ui, Some(food)));
    assert_eq!(shape(&app), ["Milk", "Soap", "[Food]", "Bread"]);
    assert_eq!(app.selected_section(), Some(food));
    render(&mut app, &ui);

    // tapping the header opens the section's form; the name can change
    let row = app.rows.iter().position(|r| matches!(r, Row::Section { .. })).unwrap();
    let (x, y, w, h) = app.row_rect(&ui, row);
    assert_eq!(app.hit(&ui, x + w / 2.0, y + h / 2.0), Some(Target::Section(row)));
    app.activate(&ui, Target::Section(row), 10.0);
    assert_eq!(app.title.text(), "Food");
    assert_eq!(app.list_edit_title(), "Section");
    assert!(app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Delete));
    assert!(!app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Colors), "a section has only a name");
    render(&mut app, &ui);
    app.set_focus(Some(Field::Title), 11.0);
    for _ in 0..4 {
        app.key(&ui, Out::Backspace, 11.0);
    }
    app.key(&ui, Out::Text("Pantry".into()), 12.0);
    app.activate(&ui, Target::Save, 13.0);
    assert_eq!(shape(&app), ["Milk", "Soap", "[Pantry]", "Bread"]);

    // moving out and deleting the section keep the reminders
    assert!(app.move_selected_to_section(&ui, None));
    assert_eq!(app.selected_section(), None);
    assert_eq!(shape(&app), ["Milk", "Soap", "Bread", "[Pantry]"]);
    let row = app.rows.iter().position(|r| matches!(r, Row::Section { .. })).unwrap();
    app.activate(&ui, Target::Section(row), 14.0);
    app.activate(&ui, Target::Delete, 15.0);
    assert!(app.open_list_sections().is_empty());
    assert_eq!(shape(&app), ["Milk", "Soap", "Bread"]);
    assert!(app.step_history(&ui, false), "Undo Delete Section");
    assert_eq!(app.open_list_sections().len(), 1);
    cleanup(&path);
}

#[test]
fn dragging_a_reminder_onto_another_section_moves_it_there() {
    let (mut app, ui, path) = app("drag-section");
    for (i, title) in ["One", "Two", "Three"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    let (one, two, three) = (id_of(&app, "One"), id_of(&app, "Two"), id_of(&app, "Three"));
    let mut next = app.data.clone();
    let a = organize::add_section(&mut next, 1, "A").unwrap();
    let b = organize::add_section(&mut next, 1, "B").unwrap();
    next.items[0].section = Some(a);
    next.items[1].section = Some(b);
    assert!(app.commit(next));
    app.activate(&ui, Target::List(1), 50.0);
    assert_eq!(shape(&app), ["Three", "[A]", "One", "[B]", "Two"]);
    assert!(app.can_reorder());

    // dropped below Two: the end of section B
    let row_two = app.rows.iter().position(|r| matches!(r, Row::Item { idx, .. } if app.data.items[*idx].id == two)).unwrap();
    let rect = app.row_rect(&ui, row_two);
    app.drag = Some(Drag { id: three, y: rect.1 + rect.3 });
    app.finish_drag(&ui);
    assert_eq!(shape(&app), ["[A]", "One", "[B]", "Two", "Three"]);
    assert_eq!(app.data.items.iter().find(|r| r.id == three).unwrap().section, Some(b));

    // dropped just above B's header: the end of section A
    let header_b = app.rows.iter().position(|r| matches!(r, Row::Section { id, .. } if *id == b)).unwrap();
    let rect = app.row_rect(&ui, header_b);
    app.drag = Some(Drag { id: three, y: rect.1 + 1.0 });
    app.finish_drag(&ui);
    assert_eq!(shape(&app), ["[A]", "One", "Three", "[B]", "Two"]);

    // dropped on the first row: out of every section, to the top
    let rect = app.row_rect(&ui, 0);
    app.drag = Some(Drag { id: one, y: rect.1 });
    app.finish_drag(&ui);
    assert_eq!(app.data.items.iter().find(|r| r.id == one).unwrap().section, None);
    assert_eq!(shape(&app)[0], "One");
    cleanup(&path);
}

#[test]
fn auto_categorize_adds_grocery_sections_and_is_one_undo_step() {
    let (mut app, ui, path) = app("categorize");
    for (i, title) in ["Milk", "Bread", "Dentist"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    assert!(!app.auto_categorize(&ui), "needs a list on screen");
    app.activate(&ui, Target::List(1), 40.0);
    assert!(app.auto_categorize(&ui));
    assert_eq!(shape(&app), ["Dentist", "[Dairy and Eggs]", "Milk", "[Bakery]", "Bread"]);
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("2 reminders")));
    assert!(!app.auto_categorize(&ui), "nothing more to place");
    assert!(app.step_history(&ui, false));
    assert!(app.open_list_sections().is_empty());
    assert_eq!(shape(&app), ["Milk", "Bread", "Dentist"]);
    cleanup(&path);
}

#[test]
fn list_groups_fold_in_the_sidebar_and_deleting_one_keeps_its_lists() {
    let (mut app, ui, path) = app("groups");
    // sidebar rows are announced with ids 1000 + list index
    let has_row = |app: &Reminders| app.controls(&ui).iter().any(|n| n.id == 1000);
    app.new_group(&ui, 0.0);
    assert_eq!(app.list_edit_title(), "New Group");
    app.key(&ui, Out::Text("Home".into()), 1.0);
    app.activate(&ui, Target::Save, 2.0);
    let gid = app.data.groups[0].id;
    assert_eq!(app.data.groups[0].name, "Home");

    app.activate(&ui, Target::ListInfo(1), 3.0);
    assert!(app.form_rows().0.iter().any(|(t, _, _)| *t == Target::ListGroup), "the form offers groups once one exists");
    app.activate(&ui, Target::ListGroup, 4.0);
    app.activate(&ui, Target::PinList, 5.0);
    render(&mut app, &ui);
    app.activate(&ui, Target::Save, 6.0);
    assert_eq!(app.data.lists[0].group, Some(gid));
    assert!(app.data.lists[0].pinned);
    assert!(has_row(&app));
    render(&mut app, &ui);

    app.activate(&ui, Target::ToggleGroup(gid), 7.0);
    assert!(!has_row(&app), "a folded group hides its lists");
    app.activate(&ui, Target::ToggleGroup(gid), 8.0);
    assert!(has_row(&app));

    app.edit_group(&ui, gid, 9.0);
    assert_eq!(app.title.text(), "Home");
    app.activate(&ui, Target::Delete, 10.0);
    assert!(app.data.groups.is_empty());
    assert_eq!(app.data.lists[0].group, None);
    assert!(has_row(&app));
    cleanup(&path);
}

#[test]
fn cycling_the_group_chooser_visits_every_group_then_none() {
    let (mut app, ui, path) = app("group-cycle");
    let mut next = app.data.clone();
    next.groups = vec![model::ListGroup { id: 1, name: "A".into() }, model::ListGroup { id: 2, name: "B".into() }];
    assert!(app.commit(next));
    app.activate(&ui, Target::ListInfo(1), 0.0);
    let group = |app: &Reminders| app.list_edit.as_ref().unwrap().group;
    assert_eq!(group(&app), None);
    for expected in [Some(1), Some(2), None, Some(1)] {
        app.activate(&ui, Target::ListGroup, 1.0);
        assert_eq!(group(&app), expected);
    }
    cleanup(&path);
}

#[test]
fn a_list_saved_as_a_template_makes_new_open_lists() {
    let (mut app, ui, path) = app("templates");
    add(&mut app, &ui, "Socks", 0.0);
    add(&mut app, &ui, "Shoes", 10.0);
    let today = app.today;
    let mut next = app.data.clone();
    next.items[0].completed = true;
    next.items[1].due = Some(today + 1);
    assert!(app.commit(next));
    assert!(!app.save_as_template(), "needs a list on screen");
    app.activate(&ui, Target::List(1), 20.0);
    assert!(app.save_as_template());
    assert_eq!(app.templates().iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), ["Reminders"]);
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("template")));
    let tid = app.templates()[0].0;
    assert!(app.new_list_from_template(&ui, tid));
    assert_eq!(app.data.lists.len(), 2);
    assert_eq!(app.view, View::List(2), "the new list opens");
    let made: Vec<&Reminder> = app.data.items.iter().filter(|r| r.list == 2).collect();
    assert_eq!(made.len(), 2);
    assert!(made.iter().all(|r| !r.completed && r.due.is_none()));
    assert!(!app.new_list_from_template(&ui, 99));
    assert!(app.delete_template(tid));
    assert!(app.templates().is_empty());
    assert!(!app.delete_template(tid));
    cleanup(&path);
}

#[test]
fn import_reads_a_checklist_into_a_new_list_and_print_gives_it_back() {
    let (mut app, ui, path) = app("import");
    let text = "# Trip\n\n- [ ] Socks\n  - [x] Wool (2026-10-08 09:05)\n- [ ] Shoes\n";
    assert!(app.import_checklist(&ui, text, "ignored"));
    assert_eq!(app.data.lists.len(), 2);
    assert_eq!(app.data.lists[1].name, "Trip");
    assert_eq!(app.view, View::List(2));
    assert_eq!(shape(&app), ["Socks", "Shoes"], "a completed reminder is hidden until Show Completed");
    app.activate(&ui, Target::OptCompleted, 4.0);
    assert_eq!(shape(&app), ["Socks", "  Wool", "Shoes"]);
    let wool = app.data.items.iter().find(|r| r.title == "Wool").unwrap();
    assert!(wool.completed && wool.parent.is_some() && wool.time == Some(9 * 60 + 5));
    let (name, body) = app.printable_list().unwrap();
    assert_eq!(name, "Trip");
    assert_eq!(body, "# Trip\n\n- [ ] Socks\n  - [x] Wool (2026-10-08 09:05)\n- [ ] Shoes\n", "the printout is what was imported");
    assert!(!app.import_checklist(&ui, "nothing to read here", "x"));
    assert_eq!(app.notice.as_deref(), Some("Nothing to import"));
    app.activate(&ui, Target::Smart(Smart::All), 5.0);
    let (name, body) = app.printable_list().unwrap();
    assert_eq!(name, "All");
    assert_eq!(body, "# All\n\n- [ ] Socks\n- [x] Wool (2026-10-08 09:05)\n- [ ] Shoes\n", "a smart list prints its rows flat");
    assert!(app.can_print());
    app.activate(&ui, Target::Smart(Smart::Today), 5.0);
    assert_eq!(app.printable_list().unwrap().1, "# Today\n\n", "a smart list with no rows prints its title");
    assert_eq!(app.edit_titles().0, "Undo Import List");
    cleanup(&path);
}

#[test]
fn the_list_and_search_intents_change_what_is_shown() {
    let (mut app, ui, path) = app("intents");
    add(&mut app, &ui, "Water plants", 0.0);
    let id = app.create_list(&ui, "  Garden  ").unwrap();
    assert_eq!(app.data.lists.iter().find(|l| l.id == id).unwrap().name, "Garden");
    assert!(app.create_list(&ui, "  ").is_err());
    assert_eq!(app.open_list_named(&ui, "garden"), Ok("Garden".to_string()));
    assert_eq!(app.view, View::List(id));
    assert!(app.open_list_named(&ui, "nope").is_err());
    assert_eq!(app.open_smart_named(&ui, " flagged "), Ok(Smart::Flagged));
    assert_eq!(app.view, View::Smart(Smart::Flagged));
    assert!(app.open_smart_named(&ui, "later").is_err());
    assert_eq!(app.search_for(&ui, "plants", 5.0), Ok(1));
    assert!(app.searching);
    assert_eq!(app.search_for(&ui, "zebra", 6.0), Ok(0));
    assert!(app.search_for(&ui, "   ", 7.0).is_err());
    app.open_editor(&ui, None, 8.0);
    assert!(app.open_list_named(&ui, "Garden").is_err(), "not while a reminder is being written");
    assert!(app.search_for(&ui, "plants", 9.0).is_err());
    cleanup(&path);
}

#[test]
fn moving_a_reminder_to_another_list_drops_its_section() {
    let (mut app, ui, path) = app("section-list-move");
    add(&mut app, &ui, "Milk", 0.0);
    let mut next = app.data.clone();
    next.lists.push(model::ListDef::new(2, "Other", 1, 0));
    let s = organize::add_section(&mut next, 1, "Dairy").unwrap();
    next.items[0].section = Some(s);
    assert!(app.commit(next));
    let id = id_of(&app, "Milk");
    app.open_editor(&ui, Some(id), 10.0);
    app.activate(&ui, Target::MoveList, 11.0);
    app.activate(&ui, Target::Save, 12.0);
    let r = app.data.items.iter().find(|r| r.id == id).unwrap();
    assert_eq!((r.list, r.section), (2, None), "a section belongs to one list");
    cleanup(&path);
}

#[test]
fn quick_add_reads_the_line_as_a_reminder_in_the_default_list() {
    let (mut app, ui, path) = app("quick-add");
    app.data.lists.push(model::ListDef::new(2, "Work", 1, 0));
    app.opts.default_list = Some(2);
    let id = app.quick_add(&ui, "  fix   bug tomorrow 9am !!!  ", 0.0).unwrap();
    let r = app.data.items.iter().find(|r| r.id == id).unwrap();
    assert_eq!((r.title.as_str(), r.list, r.priority), ("fix bug", 2, 3));
    assert_eq!((r.due, r.time), (Some(app.today + 1), Some(9 * 60)));
    assert!(r.created.is_some());
    assert_eq!(app.edit_titles().0, "Undo New Reminder");
    assert!(app.quick_add(&ui, "   ", 1.0).is_err(), "nothing copied");
    assert!(app.quick_add(&ui, "tomorrow", 2.0).is_ok(), "a date alone stays the title");
    assert_eq!(app.data.items.len(), 2);
    cleanup(&path);
}

#[test]
fn a_custom_smart_list_is_made_from_its_form_shows_in_the_sidebar_and_can_be_edited_and_deleted() {
    let (mut app, ui, path) = app("smart-list");
    add(&mut app, &ui, "Milk #home", 0.0);
    add(&mut app, &ui, "Bread", 10.0);
    let mut next = app.data.clone();
    next.items[0].flagged = true;
    assert!(app.commit(next));

    assert!(app.new_smart_list(&ui, 20.0));
    assert_eq!(app.screen, Screen::ListEditor);
    assert_eq!(app.list_edit_title(), "New Smart List");
    let rows: Vec<Target> = app.form_rows().0.iter().map(|(t, _, _)| *t).collect();
    assert!(rows.contains(&Target::Colors) && !rows.contains(&Target::Icons) && !rows.contains(&Target::PinList));
    for rule in RuleRow::ALL {
        assert!(rows.contains(&Target::Rule(rule)), "{rule:?} is on the form");
    }
    assert!(!rows.contains(&Target::Delete), "nothing to delete in a new smart list");
    app.key(&ui, Out::Text("Home flags".into()), 21.0);
    app.activate(&ui, Target::Rule(RuleRow::Flagged), 22.0);
    app.activate(&ui, Target::Rule(RuleRow::Tag), 23.0);
    render(&mut app, &ui);
    app.activate(&ui, Target::Save, 24.0);

    assert_eq!(app.data.smarts.len(), 1);
    let (id, rules) = (app.data.smarts[0].id, app.data.smarts[0].rules);
    assert_eq!(app.data.smarts[0].name, "Home flags");
    assert_eq!(rules.flagged, model::FlagRule::Flagged);
    assert_eq!(rules.tag, Some(model::tag_key("home")));
    assert_eq!(app.view, View::Custom(id), "the new smart list opens");
    assert_eq!(shape(&app), ["Milk #home"]);
    render(&mut app, &ui);
    // the sidebar announces it as a row, with its count
    let row = app.controls(&ui).into_iter().find(|n| n.id == 1500).expect("a smart list row");
    assert_eq!((row.label.as_str(), row.value.as_str()), ("Home flags", "1"));

    app.activate(&ui, Target::Smart(Smart::All), 30.0);
    app.activate(&ui, Target::Custom(id), 31.0);
    assert_eq!(app.view, View::Custom(id));

    // the form opens again with the saved rules; the rules cycle and a save keeps them
    app.activate(&ui, Target::SmartInfo(id), 32.0);
    assert_eq!(app.title.text(), "Home flags");
    assert_eq!(app.list_edit_title(), "Smart List");
    assert!(app.form_rows().0.iter().any(|(t, _, _)| *t == Target::Delete));
    app.activate(&ui, Target::Rule(RuleRow::Priority), 33.0);
    app.activate(&ui, Target::Rule(RuleRow::List), 34.0);
    app.activate(&ui, Target::Rule(RuleRow::Due), 35.0);
    app.activate(&ui, Target::Save, 36.0);
    let rules = app.data.smarts[0].rules;
    assert_eq!((rules.priority, rules.list, rules.due), (1, Some(1), model::DueRule::Overdue));
    assert_eq!(shape(&app), Vec::<String>::new(), "no reminder is overdue with a priority");
    assert_eq!(app.edit_titles().0, "Undo Edit Smart List");

    app.activate(&ui, Target::SmartInfo(id), 40.0);
    app.activate(&ui, Target::Delete, 41.0);
    assert!(app.data.smarts.is_empty());
    assert_eq!(app.view, View::Smart(Smart::All), "the deleted smart list is not left open");
    assert!(app.step_history(&ui, false), "Undo Delete Smart List");
    assert_eq!(app.data.smarts.len(), 1);
    cleanup(&path);
}

#[test]
fn the_list_rule_cycles_through_the_lists_and_back_to_any() {
    let (mut app, ui, path) = app("smart-list-cycle");
    app.data.lists.push(model::ListDef::new(2, "Work", 1, 0));
    assert!(app.new_smart_list(&ui, 0.0));
    let list = |app: &Reminders| app.list_edit.as_ref().unwrap().rules.list;
    assert_eq!(list(&app), None);
    for expected in [Some(1), Some(2), None] {
        app.activate(&ui, Target::Rule(RuleRow::List), 1.0);
        assert_eq!(list(&app), expected);
    }
    let tag = |app: &Reminders| app.list_edit.as_ref().unwrap().rules.tag;
    app.activate(&ui, Target::Rule(RuleRow::Tag), 2.0);
    assert_eq!(tag(&app), None, "no reminder carries a tag yet");
    cleanup(&path);
}

#[test]
fn settings_apply_at_once_are_saved_beside_the_reminders_and_load_again() {
    let (mut app, ui, path) = app("settings");
    app.data.lists.push(model::ListDef::new(2, "Work", 1, 0));
    add(&mut app, &ui, "Pay rent", 0.0);
    let today = app.today;
    let mut next = app.data.clone();
    next.items[0].due = Some(today);
    assert!(app.commit(next));
    assert_eq!((app.all_day_min(), app.badge_count()), (9 * 60, 1), "the defaults");

    assert!(app.open_settings(10.0));
    assert!(!app.open_settings(11.0), "not while a form is open");
    assert_eq!(app.screen, Screen::ListEditor);
    assert_eq!(app.list_edit_title(), "Settings");
    let rows: Vec<Target> = app.form_rows().0.iter().map(|(t, _, _)| *t).collect();
    assert_eq!(rows, [Target::Setting(SettingRow::DefaultList), Target::Setting(SettingRow::AllDayAlert), Target::Setting(SettingRow::Badge)]);
    render(&mut app, &ui);

    let gen = app.generation();
    app.activate(&ui, Target::Setting(SettingRow::DefaultList), 12.0);
    assert_eq!((app.settings.default_list, app.opts.default_list), (Some(1), Some(1)));
    app.activate(&ui, Target::Setting(SettingRow::DefaultList), 13.0);
    assert_eq!(app.settings.default_list, Some(2));
    app.activate(&ui, Target::Setting(SettingRow::DefaultList), 14.0);
    assert_eq!(app.settings.default_list, None, "back to Automatic");
    app.activate(&ui, Target::Setting(SettingRow::DefaultList), 15.0);
    app.activate(&ui, Target::Setting(SettingRow::AllDayAlert), 16.0);
    assert_eq!(app.all_day_min(), 9 * 60 + 30);
    app.activate(&ui, Target::Setting(SettingRow::Badge), 17.0);
    assert_eq!(app.badge_count(), 0, "the badge is off");
    assert_eq!(app.due_count(), 1, "the reminders themselves are unchanged");
    assert!(app.generation() > gen, "the alert planner and the widget are told");
    render(&mut app, &ui);
    app.activate(&ui, Target::Save, 18.0);
    assert_eq!(app.screen, Screen::Items, "Done closes the form");

    // a new run reads them back from settings.tsv, and the default list is in force
    let again = Reminders::new(&ui, 1200, 1920, path.clone(), Wall { unix: 1_791_073_380, offset: 0, at: 0.0 });
    assert_eq!((again.settings.default_list, again.all_day_min(), again.settings.badge), (Some(1), 9 * 60 + 30, false));
    assert_eq!(again.opts.default_list, Some(1));
    // the View Options list chooser writes the same setting
    app.activate(&ui, Target::OptDefaultList(Some(2)), 20.0);
    assert_eq!(settings::load(&path.with_file_name("settings.tsv")).default_list, Some(2));
    // Undo steps through the reminders and leaves the settings alone
    assert!(app.step_history(&ui, false));
    assert_eq!(app.settings.default_list, Some(2));
    cleanup(&path);
}

#[test]
fn urgent_and_time_zone_are_set_in_the_details_form_and_urgent_from_the_menu() {
    let (mut app, ui, path) = app("urgent-zone");
    add(&mut app, &ui, "Milk", 0.0);
    let id = id_of(&app, "Milk");
    let has = |app: &Reminders, t: Target| app.form_rows().0.iter().any(|(row, _, _)| *row == t);
    app.open_editor(&ui, Some(id), 10.0);
    assert!(!has(&app, Target::TimeZone) && !has(&app, Target::Urgent), "both need a date");
    app.activate(&ui, Target::Due, 11.0);
    assert!(has(&app, Target::TimeZone) && has(&app, Target::Urgent));
    app.activate(&ui, Target::TimeZone, 12.0);
    app.activate(&ui, Target::TimeZone, 13.0);
    app.activate(&ui, Target::Urgent, 14.0);
    render(&mut app, &ui);
    app.activate(&ui, Target::Save, 15.0);
    let r = app.data.items.iter().find(|r| r.id == id).unwrap();
    assert_eq!((r.tz.as_deref(), r.urgent), (Some("Atlantic/Reykjavik"), true));
    app.activate(&ui, Target::Smart(Smart::All), 16.0);
    render(&mut app, &ui);

    // the zone cycles back to the device, and clearing the date clears both
    app.open_editor(&ui, Some(id), 20.0);
    for _ in 0..zones::CHOICES.len() - 1 {
        app.activate(&ui, Target::TimeZone, 21.0);
    }
    assert_eq!(app.editor.as_ref().unwrap().draft.tz, None, "after the last zone comes the device again");
    app.activate(&ui, Target::TimeZone, 22.0);
    app.activate(&ui, Target::Due, 23.0);
    let draft = &app.editor.as_ref().unwrap().draft;
    assert_eq!((draft.due, draft.tz.clone(), draft.urgent), (None, None, false));
    app.activate(&ui, Target::Cancel, 24.0);

    // Reminder > Mark as Urgent works on the selection, in one undo step each way
    app.selected = Some(id);
    assert!(app.selected_urgent());
    assert!(app.toggle_urgent_selected(&ui));
    assert!(!app.selected_urgent());
    assert_eq!(app.edit_titles().0, "Undo Remove Urgent Mark");
    assert!(app.toggle_urgent_selected(&ui));
    assert_eq!(app.edit_titles().0, "Undo Mark as Urgent");
    app.selected = None;
    assert!(!app.toggle_urgent_selected(&ui));
    cleanup(&path);
}

/// A PNG signature and header for a picture of `w` x `h`, enough for the size checks (it does not decode).
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

#[test]
fn pictures_are_attached_in_the_form_kept_on_done_and_deleted_on_cancel() {
    let (mut app, ui, path) = app("pictures");
    add(&mut app, &ui, "Plan trip", 0.0);
    let id = id_of(&app, "Plan trip");
    let dir = path.with_file_name("attachments");
    let files = |dir: &std::path::Path| std::fs::read_dir(dir).map(|d| d.flatten().count()).unwrap_or(0);
    let has = |app: &Reminders, t: Target| app.form_rows().0.iter().any(|(row, _, _)| *row == t);

    app.open_editor(&ui, Some(id), 10.0);
    assert!(has(&app, Target::AddImage) && !has(&app, Target::Thumbs) && !has(&app, Target::RemoveImage));
    assert!(!app.take_image_request());
    app.activate(&ui, Target::AddImage, 11.0);
    assert!(app.take_image_request(), "the form asks for the open panel");
    assert!(!app.take_image_request(), "once");
    assert!(app.attach_image(b"not a picture").is_err());
    assert!(app.attach_image(&png_header(5000, 5000)).is_err(), "too many pixels");
    app.attach_image(&png_header(40, 30)).unwrap();
    app.attach_image(&png_header(20, 20)).unwrap();
    assert!(has(&app, Target::Thumbs) && has(&app, Target::RemoveImage));
    assert_eq!(files(&dir), 2);
    render(&mut app, &ui);
    // Cancel throws the new pictures away
    app.activate(&ui, Target::Cancel, 12.0);
    assert_eq!(files(&dir), 0);
    assert!(app.data.items[0].attachments.is_empty());

    // Done keeps them, and they show on the list row
    app.open_editor(&ui, Some(id), 20.0);
    app.attach_image(&png_header(40, 30)).unwrap();
    app.attach_image(&png_header(20, 20)).unwrap();
    app.activate(&ui, Target::Save, 21.0);
    assert_eq!(app.data.items[0].attachments.len(), 2);
    assert_eq!(files(&dir), 2);
    app.activate(&ui, Target::Smart(Smart::All), 22.0);
    render(&mut app, &ui);

    // removing one in the form and saving deletes its file; a duplicate that shares a picture keeps it
    app.open_editor(&ui, Some(id), 30.0);
    app.activate(&ui, Target::RemoveImage, 31.0);
    app.activate(&ui, Target::Save, 32.0);
    assert_eq!(app.data.items[0].attachments.len(), 1);
    assert_eq!(files(&dir), 1);
    app.open_editor(&ui, Some(id), 40.0);
    app.activate(&ui, Target::Duplicate, 41.0);
    assert_eq!(app.data.items.len(), 2);
    app.open_editor(&ui, Some(id), 50.0);
    app.activate(&ui, Target::RemoveImage, 51.0);
    app.activate(&ui, Target::Save, 52.0);
    assert!(app.data.items[0].attachments.is_empty());
    assert_eq!(files(&dir), 1, "the copy still uses it");
    cleanup(&path);
}

#[test]
fn a_reminder_holds_four_pictures_and_the_form_must_be_open_to_add_one() {
    let (mut app, ui, path) = app("pictures-cap");
    assert!(app.attach_image(&png_header(8, 8)).is_err(), "no form is open");
    app.activate(&ui, Target::New, 0.0);
    for _ in 0..attach::MAX_PER_REMINDER {
        app.attach_image(&png_header(8, 8)).unwrap();
    }
    assert!(app.attach_image(&png_header(8, 8)).is_err());
    app.activate(&ui, Target::AddImage, 1.0);
    assert!(!app.take_image_request(), "the form does not ask for a fifth");
    assert!(app.notice.is_some());
    app.activate(&ui, Target::Cancel, 2.0);
    cleanup(&path);
}

#[test]
fn create_list_item_adds_to_the_named_list_and_add_reminder_keeps_the_default_list() {
    let (mut app, ui, path) = app("create-list-item");
    app.data.lists.push(model::ListDef::new(2, "Work", 1, 0));
    let id = app.add_reminder_to_named(&ui, "  Send invoice  ", Some("2030-01-02T09:30"), " work ").unwrap();
    let r = app.data.items.iter().find(|r| r.id == id).unwrap();
    assert_eq!((r.title.as_str(), r.list), ("Send invoice", 2));
    assert!(r.due.is_some());
    assert_eq!(r.time, Some(9 * 60 + 30));
    assert!(app.add_reminder_to_named(&ui, "Nope", None, "Garden").is_err(), "no list has that name");
    assert!(app.add_reminder_to_named(&ui, "   ", None, "Work").is_err(), "a blank title is refused");
    assert!(app.add_reminder_to_named(&ui, "Late", Some("soon"), "Work").is_err(), "a bad due date is refused");
    app.opts.default_list = Some(2);
    let plain = app.add_reminder(&ui, "Milk", None).unwrap();
    assert_eq!(app.data.items.iter().find(|r| r.id == plain).unwrap().list, 2, "add-reminder still uses the default list");
    cleanup(&path);
}

#[test]
fn add_list_while_a_reminders_details_are_open_makes_a_list() {
    // E2E W2-20: Done on the New List form saved the open reminder under the list's name
    let (mut app, ui, path) = app("addlist-over-details");
    add(&mut app, &ui, "Milk", 0.0);
    let id = app.data.items[0].id;
    app.screen = Screen::Items;
    app.rebuild(&ui);
    let row = app.rows.iter().position(|r| matches!(r, Row::Item { .. })).expect("the reminder's row");
    app.activate(&ui, Target::Row(row), 10.0);
    assert_eq!(app.editor.as_ref().and_then(|e| e.id), Some(id), "Details are open");
    let lists = app.data.lists.len();
    app.activate(&ui, Target::NewList, 11.0);
    assert!(app.editor.is_none(), "the Details form gave way to the list form");
    assert_eq!(app.list_edit_title(), "New List");
    app.key(&ui, Out::Text("E2E test list".into()), 12.0);
    app.activate(&ui, Target::Save, 13.0);
    assert_eq!(app.data.lists.len(), lists + 1);
    assert_eq!(app.data.lists.last().unwrap().name, "E2E test list");
    assert_eq!(app.data.items.iter().find(|r| r.id == id).unwrap().title, "Milk", "the reminder keeps its name");
    cleanup(&path);
}

#[test]
fn return_in_the_name_field_commits_new_reminder_and_new_list() {
    let (mut app, ui, path) = app("enter-commits");
    app.screen = Screen::Items;
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Call Anna".into()), 1.0);
    app.key(&ui, Out::Enter, 2.0);
    assert!(app.editor.is_none(), "Return is Done");
    assert_eq!(titles(&app), ["Call Anna"]);
    let lists = app.data.lists.len();
    app.activate(&ui, Target::NewList, 3.0);
    app.key(&ui, Out::Text("Errands".into()), 4.0);
    app.key(&ui, Out::Enter, 5.0);
    assert!(app.list_edit.is_none());
    assert_eq!(app.data.lists.len(), lists + 1);
    cleanup(&path);
}

#[test]
fn a_click_selects_a_row_and_a_second_click_opens_its_details() {
    let (mut app, ui, path) = app("click-selects");
    add(&mut app, &ui, "Feed cat", 0.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    let id = app.data.items[0].id;
    let (x, y, w, h) = row_of(&app, &ui, "Feed cat");
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    app.touch(&ui, Event::Down(cx, cy), 10.0);
    app.touch(&ui, Event::Up(cx, cy), 30.0);
    assert_eq!(app.selected, Some(id), "one click selects");
    assert!(app.editor.is_none(), "and does not open the Details form");
    app.touch(&ui, Event::Down(cx, cy), 900.0);
    app.touch(&ui, Event::Up(cx, cy), 920.0);
    assert_eq!(app.editor.as_ref().and_then(|e| e.id), Some(id), "a click on the selected row opens it");
    cleanup(&path);
}

#[test]
fn deleting_a_list_asks_first_and_the_more_button_and_right_click_open_its_menu() {
    let (mut app, ui, path) = app("list-menu");
    app.activate(&ui, Target::NewList, 0.0);
    app.key(&ui, Out::Text("Work".into()), 1.0);
    app.activate(&ui, Target::Save, 2.0);
    let work = app.data.lists.last().unwrap().id;
    app.activate(&ui, Target::List(work), 3.0);
    add(&mut app, &ui, "Report", 4.0);

    // Delete on the form: the first press asks, the second deletes
    app.activate(&ui, Target::ListInfo(work), 10.0);
    app.activate(&ui, Target::Delete, 11.0);
    assert!(app.data.lists.iter().any(|l| l.id == work), "the first press only asks");
    assert!(app.list_edit.as_ref().unwrap().confirm_delete);
    assert_eq!(app.notice.as_deref(), Some(delete_list_question(1).as_str()));
    app.activate(&ui, Target::Cancel, 12.0);
    app.activate(&ui, Target::ListInfo(work), 13.0);
    assert!(!app.list_edit.as_ref().unwrap().confirm_delete, "a new form asks again");
    app.activate(&ui, Target::Cancel, 14.0);

    // the toolbar's "..." is a menu, not List Info
    app.activate(&ui, Target::List(work), 20.0);
    render(&mut app, &ui);
    let more = app.controls(&ui).into_iter().find(|n| n.label == "Show List Info").expect("the ... button");
    let (mx, my) = (ui.pt(more.bounds.x + more.bounds.w / 2.0), ui.pt(more.bounds.y + more.bounds.h / 2.0));
    app.touch(&ui, Event::Down(mx, my), 21.0);
    app.touch(&ui, Event::Up(mx, my), 40.0);
    assert!(app.list_edit.is_none(), "\"...\" no longer opens List Info by itself");
    let (_, _, items) = app.take_menu_request().expect("a menu is asked for");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Show List Info", "Pin List", "New Section", "", "Delete List..."]);
    assert!(app.context_action(&ui, CTX_LIST_INFO, 41.0));
    assert_eq!(app.list_edit_title(), "List Info");
    app.activate(&ui, Target::Cancel, 42.0);

    // a right click on the list in the sidebar opens the same menu; Delete List asks before deleting
    render(&mut app, &ui);
    let index = app.data.lists.iter().position(|l| l.id == work).unwrap() as u64;
    let row = app.controls(&ui).into_iter().find(|n| n.id == 1000 + index).expect("the sidebar row");
    let (rx, ry) = (ui.pt(row.bounds.x + row.bounds.w / 2.0), ui.pt(row.bounds.y + row.bounds.h / 2.0));
    assert!(app.context_menu_at(&ui, rx, ry).is_some(), "right click on a list");
    assert!(app.context_action(&ui, CTX_DELETE_LIST, 50.0));
    assert!(app.data.lists.iter().any(|l| l.id == work), "Delete List from the menu asks first");
    assert!(app.list_edit.as_ref().is_some_and(|e| e.confirm_delete));
    app.activate(&ui, Target::Delete, 51.0);
    assert!(!app.data.lists.iter().any(|l| l.id == work), "the second press deletes the list");
    assert!(app.data.items.is_empty(), "with its reminders");
    cleanup(&path);
}

/// A list with "Open" and two completed reminders, shown.
fn list_with_completed(name: &str) -> (Reminders, Ui, PathBuf) {
    let (mut app, ui, path) = app(name);
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
    app.screen = Screen::Items;
    app.rebuild(&ui);
    (app, ui, path)
}

/// Window pixels at the middle of a published control.
fn centre_of(node: &bifrost_app::controls::Node, ui: &Ui) -> (f32, f32) {
    (ui.pt(node.bounds.x + node.bounds.w / 2.0), ui.pt(node.bounds.y + node.bounds.h / 2.0))
}

fn click(app: &mut Reminders, ui: &Ui, (x, y): (f32, f32), t: f64) {
    app.touch(ui, Event::Down(x, y), t);
    app.touch(ui, Event::Up(x, y), t + 20.0);
}

#[test]
fn a_list_with_completed_reminders_shows_the_completed_strip_and_show_toggles_them() {
    let (mut app, ui, path) = list_with_completed("completed-strip");
    assert_eq!(app.completed_in_list(), 2);
    assert_eq!(app.strip_h(), completed::STRIP_H, "the strip shows");
    let p = app.strip_parts(&ui).expect("laid out");
    assert!(p.y + p.h <= app.list_top_px(&ui) + 0.5, "the rows start under the strip");
    assert!(p.clear.0 > p.label_x && p.toggle.0 > p.clear.0 + p.clear.1, "count, Clear, then Show at the trailing edge");
    render(&mut app, &ui);
    let controls = app.controls(&ui);
    let show = controls.iter().find(|n| n.label == "Show Completed").expect("Show is published");
    assert!(!app.opts.show_completed);
    click(&mut app, &ui, centre_of(show, &ui), 100.0);
    assert!(app.opts.show_completed, "Show shows the completed reminders");
    assert!(app.controls(&ui).iter().any(|n| n.label == "Hide Completed"), "and becomes Hide");
    // a smart list has no strip
    app.activate(&ui, Target::Smart(Smart::All), 200.0);
    assert_eq!(app.strip_h(), 0.0);
    assert!(app.strip_parts(&ui).is_none());
    cleanup(&path);
}

#[test]
fn clear_on_the_completed_strip_asks_first_then_removes_the_completed_reminders() {
    let (mut app, ui, path) = list_with_completed("completed-clear");
    render(&mut app, &ui);
    let clear = app.controls(&ui).into_iter().find(|n| n.label == "Clear Completed").expect("Clear is published");
    click(&mut app, &ui, centre_of(&clear, &ui), 100.0);
    assert!(app.clear_ask, "Clear asks first");
    assert_eq!(app.data.items.len(), 3, "nothing is removed yet");
    let sheet = app.clear_sheet().expect("the alert");
    assert_eq!(sheet.title, completed::clear_question(2));
    render(&mut app, &ui);
    // the alert blocks the window: a click on the list does nothing
    let (x, y, w, h) = row_of(&app, &ui, "Open");
    click(&mut app, &ui, (x + w / 2.0, y + h / 2.0), 200.0);
    assert!(app.clear_ask && app.editor.is_none());
    // Cancel keeps them
    let (parent, bar) = app.sheet_parent(&ui);
    let layout = sheet.layout(parent, bar, 1.0);
    let button = |id: u32| layout.buttons.iter().find(|(b, _)| *b == id).map(|(_, r)| (ui.pt(r.x + r.w / 2.0), ui.pt(r.y + r.h / 2.0))).unwrap();
    click(&mut app, &ui, button(2), 300.0);
    assert!(!app.clear_ask && app.data.items.len() == 3, "Cancel keeps the completed reminders");
    // Escape is Cancel too; Return is Clear
    app.activate(&ui, Target::ClearAsk, 400.0);
    assert!(app.escape(&ui, 410.0) && !app.clear_ask);
    app.activate(&ui, Target::ClearAsk, 500.0);
    app.key(&ui, Out::Enter, 510.0);
    assert!(!app.clear_ask);
    assert_eq!(titles(&app), ["Open"], "Clear removes the list's completed reminders");
    assert!(app.undo.is_some(), "with Undo");
    assert_eq!(app.strip_h(), 0.0, "and the strip goes");
    app.activate(&ui, Target::ClearAsk, 600.0);
    assert!(!app.clear_ask, "nothing to ask about");
    cleanup(&path);
}

#[test]
fn the_i_on_the_selected_row_opens_details_in_a_popover_over_the_list() {
    let (mut app, ui, path) = app("details-popover");
    add(&mut app, &ui, "Feed cat", 0.0);
    add(&mut app, &ui, "Water plants", 1.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    let id = app.data.items.iter().find(|r| r.title == "Feed cat").unwrap().id;
    let (x, y, w, h) = row_of(&app, &ui, "Feed cat");
    click(&mut app, &ui, (x + w / 2.0, y + h / 2.0), 10.0);
    assert_eq!(app.selected, Some(id));
    render(&mut app, &ui);
    let info = app.controls(&ui).into_iter().find(|n| n.label == "Show Details").expect("the selected row has its i");
    click(&mut app, &ui, centre_of(&info, &ui), 100.0);
    assert_eq!(app.editor.as_ref().and_then(|e| e.id), Some(id), "the i opens the reminder's details");
    assert!(app.details_popover(), "in a popover, not the full pane");
    assert!(app.bar_px(&ui, chrome::B_DONE).is_none(), "the list's toolbar stays");
    let g = app.details_geometry(&ui).expect("placed");
    let k = ui.pt(1.0);
    assert!(g.panel.x >= g.bounds.x && g.panel.x + g.panel.w <= g.bounds.x + g.bounds.w + 0.01, "inside the content");
    assert!(g.panel.x >= g.anchor.x + g.anchor.w || g.panel.x + g.panel.w <= g.anchor.x, "beside the i, not over it");
    let (fx, fy, fw, _) = app.field_frame(&ui, Field::Title).expect("the title field");
    assert!(fx / k >= g.panel.x && (fx + fw) / k <= g.panel.x + g.panel.w + 0.01 && fy / k >= g.panel.y, "the form is in the popover");
    // the list stays drawn under it at its own place
    assert_eq!(row_of(&app, &ui, "Feed cat").1, y);
    render(&mut app, &ui);
    // a click outside the popover closes it and keeps the edits
    app.title.set_text(&ui, "Feed the cat");
    // the popover opens to the left of the i (no room on its right): the list's leading edge is outside it
    let (_, wy, _, wh) = row_of(&app, &ui, "Water plants");
    assert!(g.panel.x > 4.0, "room left of the popover");
    let outside = (app.x0 + ui.pt(g.panel.x / 2.0), wy + wh / 2.0);
    assert!(!g.panel.contains((outside.0 - app.x0) / k, outside.1 / k));
    click(&mut app, &ui, outside, 200.0);
    assert!(app.editor.is_none() && app.screen == Screen::Items, "the popover closed");
    assert_eq!(app.data.items.iter().find(|r| r.id == id).unwrap().title, "Feed the cat", "and kept the edit");
    cleanup(&path);
}

#[test]
fn escape_drops_the_details_edits_and_a_new_reminder_keeps_the_full_form() {
    let (mut app, ui, path) = app("details-escape");
    add(&mut app, &ui, "Feed cat", 0.0);
    app.screen = Screen::Items;
    app.rebuild(&ui);
    let row = app.rows.iter().position(|r| matches!(r, Row::Item { .. })).unwrap();
    app.activate(&ui, Target::Info(row), 10.0);
    assert!(app.details_popover());
    app.title.set_text(&ui, "Changed");
    assert!(app.escape(&ui, 20.0));
    assert!(app.editor.is_none());
    assert_eq!(app.data.items[0].title, "Feed cat", "Escape cancels");
    app.activate(&ui, Target::New, 30.0);
    assert!(app.editor.is_some() && !app.details_popover(), "a new reminder still opens the full form");
    assert!(app.details_geometry(&ui).is_none());
    app.activate(&ui, Target::Cancel, 40.0);
    cleanup(&path);
}

#[test]
fn cmd_backspace_never_deletes_a_selected_reminder_that_another_list_hides() {
    use bifrost_proto::CHORD_META;
    let (mut app, ui, path) = app("hidden-selection");
    add(&mut app, &ui, "E2E rem", 0.0);
    let id = id_of(&app, "E2E rem");
    app.selected = Some(id);
    assert_eq!(app.selected_item(), Some(id), "on screen in All");
    app.activate(&ui, Target::NewList, 10.0);
    app.key(&ui, Out::Text("E2E list".into()), 11.0);
    app.key(&ui, Out::Enter, 12.0);
    let list = app.data.lists.last().unwrap().id;
    // the view changes without the selection being cleared: the selection is not on screen, so nothing is deleted
    app.view = View::List(list);
    app.rebuild(&ui);
    assert_eq!(app.selected_item(), None);
    assert!(!app.chord(&ui, 14, CHORD_META, 20.0), "nothing on screen is selected");
    assert!(!app.delete_selected(&ui, 21.0));
    assert_eq!(titles(&app), ["E2E rem"]);
    // opening a list from the sidebar drops the selection made in another list
    app.activate(&ui, Target::Smart(Smart::All), 30.0);
    app.selected = Some(id);
    app.activate(&ui, Target::List(list), 31.0);
    assert_eq!(app.selected, None);
    // Recently Deleted hides the list: Down and Cmd+Backspace never reach the reminder under it
    app.activate(&ui, Target::Smart(Smart::All), 40.0);
    app.selected = Some(id);
    app.activate(&ui, Target::Deleted, 41.0);
    app.key(&ui, Out::Named("Down"), 42.0);
    assert!(!app.chord(&ui, 14, CHORD_META, 43.0));
    assert_eq!(titles(&app), ["E2E rem"]);
    cleanup(&path);
}

#[test]
fn recently_deleted_deletes_permanently_and_deletes_all_after_asking() {
    use bifrost_proto::CHORD_META;
    let (mut app, ui, path) = app("purge");
    for (i, title) in ["One", "Two", "Three"].iter().enumerate() {
        add(&mut app, &ui, title, i as f64 * 10.0);
    }
    for (i, title) in ["One", "Two", "Three"].iter().enumerate() {
        app.selected = Some(id_of(&app, title));
        assert!(app.delete_selected(&ui, 40.0 + i as f64));
    }
    assert_eq!(app.kept_count, 3);
    app.activate(&ui, Target::Deleted, 50.0);
    let kept: Vec<u64> = app.deleted.as_ref().unwrap().iter().map(|r| r.id).collect();
    assert_eq!(app.kept_selected(), None, "the page opens with nothing selected");
    assert!(!app.chord(&ui, 14, CHORD_META, 51.0), "nothing selected, nothing asked");

    // a click on the row selects it (the Restore link at its trailing edge restores); Cmd+Backspace asks first
    let (x, y, w, h) = app.deleted_row_rect(&ui, 0);
    assert_eq!(app.hit(&ui, x + w / 4.0, y + h / 2.0), Some(Target::KeptRow(kept[0])));
    assert_eq!(app.hit(&ui, x + w - ui.pt(SPACE_S), y + h / 2.0), Some(Target::Restore(kept[0])));
    app.activate(&ui, Target::KeptRow(kept[0]), 52.0);
    assert!(app.chord(&ui, 14, CHORD_META, 53.0));
    assert_eq!(app.purge_ask, Some(Purge::One(kept[0])));
    assert!(app.clear_sheet().is_some(), "the alert is up");
    app.key(&ui, Out::Enter, 54.0);
    assert!(app.purge_ask.is_some(), "Return does not answer a destructive button");
    assert!(app.escape(&ui, 55.0) && app.purge_ask.is_none(), "Escape keeps it");
    assert_eq!(app.kept_count, 3);
    assert!(app.chord(&ui, 14, CHORD_META, 56.0));
    app.activate(&ui, Target::PurgeConfirm, 57.0);
    assert_eq!(app.kept_count, 2);
    let left: Vec<u64> = app.deleted.as_ref().unwrap().iter().map(|r| r.id).collect();
    assert_eq!(left, kept[1..].to_vec());
    assert_eq!(app.kept_selected(), None);

    // the right click selects the row and offers Restore, Delete Permanently and Delete All
    let (x, y, w, h) = app.deleted_row_rect(&ui, 0);
    let menu = app.context_menu_at(&ui, x + app.x0 + w / 4.0, y + h / 2.0).expect("the kept row's menu");
    let rows: Vec<String> = menu.iter().filter(|i| !i.is_separator()).map(|i| i.label.clone()).collect();
    assert_eq!(rows, ["Restore", "Delete Permanently...", "Delete All..."]);
    assert_eq!(app.kept_selected(), Some(kept[1]));
    assert!(app.context_action(&ui, purge::CTX_PURGE, 60.0));
    app.activate(&ui, Target::PurgeConfirm, 61.0);
    assert_eq!(app.kept_count, 1);

    // the toolbar's "..." shows the same menu; Delete All empties the page
    app.activate(&ui, Target::ListMenu(0.0, 0.0), 70.0);
    let (_, _, items) = app.take_menu_request().expect("the page's menu");
    assert!(items.iter().any(|i| i.id == purge::CTX_PURGE_ALL && i.enabled));
    assert!(app.context_action(&ui, purge::CTX_PURGE_ALL, 71.0));
    assert_eq!(app.purge_ask, Some(Purge::All));
    app.activate(&ui, Target::PurgeConfirm, 72.0);
    assert_eq!(app.kept_count, 0);
    assert!(app.deleted.as_ref().unwrap().is_empty());
    assert!(archive::kept(&archive::path_beside(&path)).unwrap().is_empty(), "gone from the store for good");
    cleanup(&path);
}
