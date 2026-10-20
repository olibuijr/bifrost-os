//! The window's log-position, walk, subsystem, signpost, view and find behaviour, tested on the window's plain state (a kit `Ui`
//! only where the geometry matters: `rebuild`, the menu actions and the footer are state, and the drawing reads them).

use crate::console::*;
use crate::opened::OpenedLog;
use crate::services::test_ctx;
use crate::model::{Level, LogEntry, Query};

fn msg(date: i64, subsystem: &str, message: &str) -> LogEntry {
    LogEntry { date, process: "notes".into(), pid: 1, subsystem: subsystem.into(), category: "sync".into(), level: Level::Notice, message: message.into(), invocation: String::new() }
}

fn console_with(entries: Vec<LogEntry>) -> Console {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    for e in entries {
        c.buffer.push(e);
    }
    c.rebuild();
    c
}

#[test]
fn show_messages_after_selected_keeps_only_the_newer_messages_and_toggles_back() {
    let mut c = console_with(vec![msg(10, "a.b", "one"), msg(20, "a.b", "two"), msg(30, "a.b", "three")]);
    c.selected = Some(0);
    c.toggle_after();
    assert_eq!(c.visible, vec![1, 2]);
    assert!(c.after.is_some());
    c.toggle_after();
    assert_eq!(c.visible, vec![0, 1, 2], "Show All Messages returns to the whole list");
    assert!(c.after.is_none());
}

#[test]
fn show_messages_after_with_nothing_selected_changes_nothing() {
    let mut c = console_with(vec![msg(10, "a.b", "one"), msg(20, "a.b", "two")]);
    c.toggle_after();
    assert!(c.after.is_none());
    assert_eq!(c.visible, vec![0, 1]);
}

#[test]
fn the_signposts_view_lists_only_signposts_and_totals_their_durations() {
    let mut c = console_with(vec![
        msg(10, "a.b", "shell ready"),
        msg(20, "a.b", "interval view.load id=9 40 us"),
        msg(30, "a.b", "interval view.load id=10 60 us"),
        msg(40, "a.b", "event tap id=3"),
    ]);
    c.view = View::Signposts;
    c.rebuild();
    assert_eq!(c.visible, vec![1, 2, 3]);
    let footer = c.footer_text();
    assert!(footer.contains("view.load 2x"), "{footer}");
    assert!(footer.contains("100 us total"), "{footer}");
    assert!(footer.contains("longest 60 us"), "{footer}");
}

#[test]
fn show_only_this_subsystem_adds_a_subsystem_token_to_the_search() {
    let mut c = console_with(vec![msg(10, "is.olibuijr.notes", "one"), msg(20, "is.olibuijr.books", "two")]);
    c.selected = Some(1);
    c.show_subsystem();
    assert_eq!(c.search.to_string(), "subsystem:is.olibuijr.books");
    assert_eq!(c.visible, vec![1]);
    c.selected = Some(0);
    c.show_subsystem();
    assert_eq!(c.search.to_string(), "subsystem:is.olibuijr.books subsystem:is.olibuijr.notes");
    assert!(c.visible.is_empty(), "two subsystems never match one message");
}

#[test]
fn show_only_this_subsystem_does_nothing_for_a_message_without_one() {
    let mut c = console_with(vec![msg(10, "", "claimed")]);
    c.selected = Some(0);
    c.show_subsystem();
    assert!(c.search.to_string().is_empty());
    assert_eq!(c.visible, vec![0]);
}

#[test]
fn a_saved_search_is_kept_from_the_search_field_and_chosen_from_the_sidebar() {
    let mut c = console_with(vec![
        msg(10, "a.b", "one"),
        LogEntry { level: Level::Error, ..msg(20, "a.b", "two") },
        LogEntry { level: Level::Error, ..msg(30, "a.b", "three") },
    ]);
    c.focus_search();
    assert!(c.text("type:error"));
    assert_eq!(c.visible, vec![1, 2]);
    assert!(c.can_save_search());
    assert!(!c.can_delete_search());
    assert!(c.menu(MENU_SAVE_SEARCH));
    assert_eq!(c.saved.len(), 1);
    assert!(!c.can_save_search(), "a saved search is not offered again");
    assert!(c.can_delete_search());
    // a second save of the same text adds nothing
    c.menu(MENU_SAVE_SEARCH);
    assert_eq!(c.saved.len(), 1);

    // clear the search, then choose the saved one: the live journal shows with its text
    c.search.clear();
    c.query = Query::default();
    c.rebuild();
    assert_eq!(c.visible, vec![0, 1, 2]);
    c.open_saved(0);
    assert_eq!(c.search.to_string(), "type:error");
    assert_eq!(c.visible, vec![1, 2]);
    assert!(!c.search_focused);

    assert!(c.menu(MENU_DELETE_SEARCH));
    assert!(c.saved.is_empty());
    assert!(!c.can_delete_search());
}

#[test]
fn a_blank_search_cannot_be_saved() {
    let mut c = console_with(vec![msg(10, "a.b", "one")]);
    assert!(!c.can_save_search());
    c.menu(MENU_SAVE_SEARCH);
    assert!(c.saved.is_empty());
}

#[test]
fn the_message_token_and_the_level_token_narrow_the_table() {
    let mut c = console_with(vec![
        msg(10, "a.b", "disk full"),
        LogEntry { level: Level::Error, ..msg(20, "a.b", "disk slow") },
        LogEntry { level: Level::Error, ..msg(30, "a.b", "network down") },
    ]);
    c.focus_search();
    assert!(c.text("level:error message:disk"));
    assert_eq!(c.visible, vec![1]);
}

#[test]
fn the_security_events_view_lists_every_event_and_the_search_filters_by_category() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    c.open_view(View::Security);
    assert_eq!(c.visible.len(), 5, "Info is off by default, so only the five Authorize rows show");
    c.levels.info = true;
    c.rebuild();
    assert_eq!(c.visible.len(), 13, "one row per valid (kind, phase) pair");
    assert_eq!(c.footer_text(), "13 messages");
    assert_eq!(c.status, crate::security::NOTE);
    c.focus_search();
    assert!(c.text("category:Socket"));
    assert_eq!(c.visible.len(), 6, "bind, connect and suspend-resume, each with two phases");
    assert_eq!(c.footer_text(), "6 of 13 messages");
}

fn log(name: &str, grant: u8, lines: &[&str]) -> OpenedLog {
    let entries = lines.iter().enumerate().map(|(i, l)| msg(i as i64 + 1, "", l)).collect();
    OpenedLog { name: name.into(), entries, grant: Some([grant; 16]), writable: true }
}

#[test]
fn reload_reads_the_view_again_and_keeps_the_search_and_drops_the_selection() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    c.open_view(View::Security);
    c.levels.info = true;
    c.rebuild();
    c.focus_search();
    assert!(c.text("category:Socket"));
    c.selected = Some(0);
    c.detail = Some((0, "text".into()));
    assert!(c.menu(MENU_RELOAD));
    assert_eq!(c.view, View::Security);
    assert_eq!(c.search.to_string(), "category:Socket");
    assert_eq!(c.visible.len(), 6, "the same rows come back through the same search");
    assert_eq!(c.selected, None);
    assert!(c.detail.is_none());
}

#[test]
fn hiding_the_sidebar_moves_the_table_to_the_edge_and_the_toolbar_clear_of_the_traffic_lights() {
    let ui = test_ctx().ui;
    let mut c = console_with(vec![msg(10, "a.b", "one")]);
    assert_eq!(c.geo(&ui).table_x, SIDE_PT);
    let shown = c.band(&ui).x;
    assert!(c.menu(MENU_SIDEBAR));
    assert!(!c.show_sidebar);
    assert_eq!(c.side_w(), 0.0);
    let g = c.geo(&ui);
    assert_eq!(g.table_x, 0.0);
    assert_eq!(g.cols[0].1, 0.0, "the first column starts at the window's edge");
    assert!(c.band(&ui).x >= bifrost_app::layout::traffic_lights_trailing(), "the toolbar starts past the traffic lights");
    assert!(shown > SIDE_PT);
    assert!(c.menu(MENU_SIDEBAR));
    assert_eq!(c.geo(&ui).table_x, SIDE_PT);
}

#[test]
fn the_detail_pane_can_be_hidden_and_shown_again() {
    let ui = test_ctx().ui;
    let mut c = console_with(vec![msg(10, "a.b", "one")]);
    assert!(c.geo(&ui).pane_x.is_none(), "nothing selected, no pane");
    c.selected = Some(0);
    assert!(c.geo(&ui).pane_x.is_some());
    let with_pane = c.geo(&ui).table_right;
    assert!(c.menu(MENU_DETAILS));
    let g = c.geo(&ui);
    assert!(g.pane_x.is_none());
    assert!(g.table_right > with_pane, "the table takes the pane's room");
    assert!(c.menu(MENU_DETAILS));
    assert!(c.geo(&ui).pane_x.is_some());
}

#[test]
fn the_columns_menu_adds_and_removes_columns_and_keeps_one() {
    let ui = test_ctx().ui;
    let mut c = console_with(vec![msg(10, "a.b", "one")]);
    let cols = |c: &Console| c.geo(&ui).cols.iter().map(|x| x.0).collect::<Vec<_>>();
    assert!(!cols(&c).contains(&Col::Pid), "PID starts hidden");
    c.menu(MENU_COLUMN + 3);
    assert_eq!(Col::ALL[3], Col::Pid);
    assert!(cols(&c).contains(&Col::Pid));
    c.menu(MENU_COLUMN + 1);
    assert!(!cols(&c).contains(&Col::Time));
    for i in 0..Col::ALL.len() as u32 {
        c.menu(MENU_COLUMN + i);
    }
    assert_eq!(cols(&c).len(), 1, "the last column cannot be switched off");
}

fn multi_line_console() -> Console {
    let mut c = console_with(vec![msg(10, "a.b", "one"), msg(20, "a.b", "first\nsecond\nthird"), msg(30, "a.b", "three")]);
    c.selected = Some(1);
    c
}

#[test]
fn an_expanded_message_grows_its_row_and_pushes_the_rows_below() {
    let ui = test_ctx().ui;
    let mut c = multi_line_console();
    let g = c.geo(&ui);
    assert_eq!(c.table_rows(&ui, &g).height_of(1), ROW_PT);
    assert!(c.menu(MENU_EXPAND));
    assert!(c.selected_expanded());
    let rows = c.table_rows(&ui, &g);
    assert_eq!(rows.height_of(1), ROW_PT + 2.0 * EXPAND_LINE_PT, "three lines: two more than a plain row");
    assert_eq!(rows.height_of(0), ROW_PT);
    assert_eq!(rows.top_of(2), 2.0 * ROW_PT + 2.0 * EXPAND_LINE_PT);
    assert_eq!(rows.content_height(3), 3.0 * ROW_PT + 2.0 * EXPAND_LINE_PT);
    // a tap inside the grown row selects it; a tap below it selects the row that moved down
    let top = g.table_top;
    assert_eq!(c.row_at(&ui, top + ROW_PT + 40.0), Some(1));
    assert_eq!(c.row_at(&ui, top + rows.top_of(2) + 2.0), Some(2));
    assert_eq!(c.fit_scroll(&ui), 0.0, "everything fits the window");
}

#[test]
fn collapsing_returns_the_row_and_collapse_all_clears_every_expansion() {
    let ui = test_ctx().ui;
    let mut c = multi_line_console();
    c.menu(MENU_EXPAND);
    c.menu(MENU_EXPAND);
    assert!(!c.selected_expanded() && !c.any_expanded());
    let g = c.geo(&ui);
    assert_eq!(c.table_rows(&ui, &g).height_of(1), ROW_PT);
    c.menu(MENU_EXPAND);
    c.selected = Some(2);
    c.menu(MENU_EXPAND);
    assert!(c.any_expanded());
    assert!(c.menu(MENU_COLLAPSE_ALL));
    assert!(!c.any_expanded());
}

#[test]
fn a_single_line_message_has_nothing_to_expand() {
    let ui = test_ctx().ui;
    let mut c = multi_line_console();
    c.selected = Some(0);
    c.menu(MENU_EXPAND);
    let g = c.geo(&ui);
    assert_eq!(c.table_rows(&ui, &g).height_of(0), ROW_PT, "one line fits the row");
}

#[test]
fn expansions_end_when_the_view_changes_or_the_display_is_cleared() {
    let mut c = multi_line_console();
    c.menu(MENU_EXPAND);
    c.menu(MENU_CLEAR);
    assert!(!c.any_expanded());
    c.selected = None;
    c.open_view(View::Security);
    c.selected = Some(0);
    c.menu(MENU_EXPAND);
    c.menu(MENU_RELOAD);
    assert!(!c.any_expanded());
}

fn find_console() -> Console {
    console_with(vec![msg(10, "a.b", "alpha one"), msg(20, "a.b", "beta"), msg(30, "a.b", "alpha two"), msg(40, "a.b", "gamma")])
}

#[test]
fn find_next_and_previous_step_through_matches_without_filtering_and_wrap() {
    let mut c = find_console();
    c.find_text = "alpha".into();
    assert!(c.menu(MENU_FIND_NEXT));
    assert_eq!(c.selected, Some(0));
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(2));
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(0), "wraps past the last match");
    c.menu(MENU_FIND_PREVIOUS);
    assert_eq!(c.selected, Some(2), "wraps back past the first");
    assert_eq!(c.visible, vec![0, 1, 2, 3], "the table is not filtered");
}

#[test]
fn find_with_no_find_text_uses_the_search_field_and_a_miss_keeps_the_selection() {
    let mut c = find_console();
    c.focus_search();
    c.text("beta");
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(1));
    c.find_text = "zzz".into();
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(1));
    assert_eq!(c.status, "No message matches.");
}

#[test]
fn use_selection_for_find_sets_the_find_text_from_the_selected_message() {
    let mut c = find_console();
    c.menu(MENU_USE_SELECTION);
    assert!(c.find_text.is_empty(), "nothing selected");
    c.selected = Some(2);
    c.menu(MENU_USE_SELECTION);
    assert_eq!(c.find_text, "\"alpha two\"");
    assert_eq!(c.status, "Find: \"alpha two\"");
    c.selected = Some(3);
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(2), "the only message with that text");
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(2));
}

#[test]
fn find_scrolls_the_found_row_into_view() {
    let ui = test_ctx().ui;
    let many: Vec<LogEntry> = (0..200).map(|i| msg(i, "a.b", &format!("line {i}"))).collect();
    let mut c = console_with(many);
    c.find_text = "line 150".into();
    c.menu(MENU_FIND_NEXT);
    assert_eq!(c.selected, Some(150));
    let g = c.geo(&ui);
    let max = c.fit_scroll(&ui);
    c.apply_reveal(&ui, max);
    let offset = c.scroll.offset();
    let top = 150.0 * ROW_PT;
    assert!(top >= offset && top + ROW_PT <= offset + (g.table_bottom - g.table_top) + 0.01, "row 150 is whole in view at offset {offset}");
}

#[test]
fn an_opened_log_is_shown_whole_and_listed_under_opened_files() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    assert!(c.add_opened(log("boot.logarchive", 1, &["one", "two", "three"])).is_none());
    assert_eq!(c.view, View::Opened(0));
    assert_eq!(c.visible.len(), 3);
    assert_eq!(c.footer_text(), "3 messages");
    assert_eq!(c.active_opened().map(|o| o.name.as_str()), Some("boot.logarchive"));
    assert_eq!(c.empty_message(), "No Log Entries");
}

#[test]
fn the_oldest_opened_log_goes_when_too_many_are_held() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    for i in 0..MAX_OPENED as u8 {
        assert!(c.add_opened(log(&format!("log{i}"), i, &["x"])).is_none());
    }
    assert_eq!(c.add_opened(log("extra", 99, &["y"])), Some([0; 16]), "the first one's grant comes back to be released");
    assert_eq!(c.opened.len(), MAX_OPENED);
    assert_eq!(c.opened[0].name, "log1");
    assert_eq!(c.view, View::Opened(MAX_OPENED - 1));
}

#[test]
fn open_recent_shows_the_newest_first_and_forgetting_a_log_removes_it() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    c.add_opened(log("old", 1, &["a"]));
    c.add_opened(log("new", 2, &["b", "c"]));
    c.menu(MENU_RECENT + 1);
    assert_eq!(c.view, View::Opened(0), "the second item is the older log");
    c.menu(MENU_RECENT);
    assert_eq!(c.view, View::Opened(1));
    assert_eq!(c.visible.len(), 2);
    c.view = View::Security;
    c.forget_opened([1; 16]);
    assert_eq!(c.opened.len(), 1);
    assert_eq!(c.opened[0].name, "new");
    assert_eq!(c.view, View::Security, "another view stays");
}

#[test]
fn the_loaded_source_gives_the_newest_entries_up_to_the_limit() {
    use crate::source::{Loaded, Source};
    let mut source = Loaded { entries: vec![msg(1, "", "a"), msg(2, "", "b"), msg(3, "", "c")] };
    let last_two = source.load(2).unwrap();
    assert_eq!(last_two.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
    assert_eq!(source.load(10).unwrap().len(), 3);
}

#[test]
fn save_messages_without_a_panel_writes_the_selection_or_every_shown_message() {
    let dir = std::env::temp_dir().join(format!("console-wire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = find_console();
    assert_eq!(c.messages_to_save().lines().count(), 4);
    c.selected = Some(1);
    assert_eq!(c.messages_to_save().lines().count(), 1);
    c.save_messages(&dir, 5);
    assert!(c.status.starts_with("Saved "), "{}", c.status);
    let text = std::fs::read_to_string(dir.join("Documents/Console/Console-5.log")).unwrap();
    assert!(text.contains("beta") && !text.contains("alpha"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn marked_messages_are_what_save_messages_writes_and_a_mark_toggles() {
    let mut c = console_with(vec![msg(1, "a", "alpha"), msg(2, "b", "beta"), msg(3, "c", "gamma")]);
    let seqs: Vec<u64> = c.buffer.iter().map(|(s, _)| s).collect();
    c.selected = None;
    c.toggle_mark();
    assert!(c.marked.is_empty(), "no message selected, nothing marked");
    c.selected = Some(seqs[0]);
    c.toggle_mark();
    assert!(c.selected_marked());
    c.selected = Some(seqs[2]);
    c.toggle_mark();
    let text = c.messages_to_save();
    assert!(text.contains("alpha") && text.contains("gamma") && !text.contains("beta"), "{text}");
    assert_eq!(text.lines().count(), 2);
    c.toggle_mark();
    assert!(!c.selected_marked(), "a second mark takes it out again");
    assert_eq!(c.messages_to_save().lines().count(), 1);
    c.selected = None;
    c.clear_marks();
    assert!(c.marked.is_empty());
    assert_eq!(c.messages_to_save().lines().count(), 3, "with no marks, every shown message is saved");
}

#[test]
fn every_sidebar_row_has_a_drawn_icon_and_no_platform_name() {
    use bifrost_app::controls::sidebar::SidebarItem;
    let c = console_with(Vec::new());
    let mut rows = 0;
    for item in &c.sidebar().items {
        if let SidebarItem::Row { label, symbol, .. } = item {
            rows += 1;
            let name = symbol.as_deref().unwrap_or_else(|| panic!("{label} has no icon"));
            assert!(bifrost_app::glyphs::has(name), "{label} uses {name}, which the kit does not draw");
            assert_ne!(label, "This Mac");
        }
    }
    assert!(rows >= 8, "the live row and the report rows");
    assert!(c.sidebar().items.iter().any(|i| matches!(i, SidebarItem::Row { label, .. } if label == "This Computer")));
}
