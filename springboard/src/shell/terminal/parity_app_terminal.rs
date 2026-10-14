//! Terminal parity (app-terminal): Close Other Tabs, Man Page for Selection and Edit Title, driven
//! through the More menu rows the way a tap or a menu pick drives them.
use super::super::tests::{pump_until, terminal};
use super::*;

#[test]
fn close_other_tabs_keeps_only_the_front_tab() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    sh.new_terminal_tab(None);
    assert_eq!(sh.term.len(), 3);
    sh.term.select(1);
    sh.term.as_mut().unwrap().set_custom_title(Some("keep".into()));
    sh.terminal_menu_action(24, 2000.0);
    assert_eq!(sh.term.len(), 1, "the other two tabs closed");
    assert_eq!(sh.term.as_ref().unwrap().custom_title(), Some("keep"), "the front tab is the one kept");
    assert_eq!(sh.terminal_ui.message, "Other tabs closed");
}

#[test]
fn close_other_tabs_with_one_tab_changes_nothing() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(24, 2000.0);
    assert_eq!(sh.term.len(), 1);
    assert_eq!(sh.terminal_ui.message, "Other tabs closed");
}

#[test]
fn file_menu_close_other_tabs_runs_the_same_action() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    sh.terminal_hw_key(TermKey::CloseOtherTabs, 2000.0);
    assert_eq!(sh.term.len(), 1);
    assert_eq!(sh.terminal_ui.message, "Other tabs closed");
}

#[test]
fn man_page_opens_the_selected_command_in_a_new_tab() {
    let mut sh = terminal(&["/bin/sh", "-c", "printf 'ls -la\\n'; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("ls -la"));
    sh.terminal_menu_action(26, 2000.0);
    assert_eq!(sh.term.len(), 1, "nothing is selected, so no tab opens");
    assert_eq!(sh.terminal_ui.message, "Select a command name first");
    sh.term.as_mut().unwrap().select_line(0);
    sh.terminal_menu_action(26, 2100.0);
    assert_eq!(sh.term.len(), 2, "the selection's first word opens man in a tab");
    assert_eq!(sh.terminal_ui.panel, Panel::None, "the panel closes once the tab opens");
}

#[test]
fn edit_title_names_the_front_tab_and_an_empty_name_restores_it() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(27, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Title);
    sh.terminal_ui.tab_title.as_mut().unwrap().set_text(&sh.ui, "  build  ");
    sh.terminal_save_title(2100.0);
    assert_eq!(sh.terminal_ui.panel, Panel::None);
    assert_eq!(sh.term.as_ref().unwrap().title(0), "build", "the typed name, trimmed, is the tab caption");

    sh.terminal_menu_action(27, 2200.0);
    assert_eq!(
        sh.terminal_ui.tab_title.as_ref().unwrap().text(),
        "build",
        "the panel starts with the current name"
    );
    sh.terminal_ui.tab_title.as_mut().unwrap().set_text(&sh.ui, "   ");
    sh.terminal_save_title(2300.0);
    assert!(sh.term.as_ref().unwrap().custom_title().is_none(), "a blank name restores the automatic title");
    assert_ne!(sh.term.as_ref().unwrap().title(0), "build");
}

#[test]
fn edit_title_panel_takes_enter_to_save() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(27, 2000.0);
    sh.terminal_ui.tab_title.as_mut().unwrap().set_text(&sh.ui, "logs");
    assert!(sh.terminal_panel_input(&Out::Enter, 2100.0), "the panel takes the key");
    assert_eq!(sh.term.as_ref().unwrap().custom_title(), Some("logs"));
    assert_eq!(sh.terminal_ui.panel, Panel::None);
}

#[test]
fn terminal_help_opens_the_shortcut_list_and_closes() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(28, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Help);
    assert!(sh.terminal_ui.message.is_empty());
    sh.terminal_close_panel(2100.0);
    assert_eq!(sh.terminal_ui.panel, Panel::None);
}
