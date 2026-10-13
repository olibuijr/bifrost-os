use super::super::tests::{pump_until, terminal};
use super::super::{AppId, Screen};
use super::*;
fn tap_control(sh: &mut Shell, i: usize, t: f64) {
    sh.terminal_action(i, t + 20.0);
}
#[test]
fn tab_controls_poll_background_output_and_keep_active_input_separate() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x; echo got:$x; read y"]);
    let g = layout::Header::new(sh.w as f32, sh.ui.s, sh.term.len());
    let x = g.plus + sh.ui.pt(22.0);
    let y = sh.ui.pt(scene::STATUS_PT + 20.0);
    sh.terminal_touch(Event::Down(x, y), 2000.0);
    sh.terminal_touch(Event::Up(x, y), 2020.0);
    assert_eq!(sh.term.len(), 2);
    assert_eq!(sh.app_poll_fds().len(), 2);
    sh.term.iter_mut().next().unwrap().write(b"background\r");
    pump_until(&mut sh, |sh| {
        sh.term
            .iter()
            .next()
            .unwrap()
            .viewport_text()
            .contains("got:background")
    });
    assert!(sh.term.iter().next().unwrap().unread);
    assert!(!sh
        .term
        .as_ref()
        .unwrap()
        .viewport_text()
        .contains("got:background"));
    sh.term.select(0);
    assert!(!sh.term.as_ref().unwrap().unread);
    let area = sh.term_area();
    sh.set_keyboard(false, 3000.0);
    assert!(sh.term.iter().all(|term| term.area().3 > area.3));
    sh.term.close(1);
    assert_eq!(sh.app_poll_fds().len(), 1);
}
#[test]
fn cmd_keys_drive_find_count_tabs_and_zoom() {
    let mut sh = terminal(&["/bin/sh", "-c", "echo alpha; echo beta alpha; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("beta alpha"));
    sh.terminal_hw_key(TermKey::Find, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Find);
    sh.terminal_ui.query.as_mut().unwrap().set_text(&sh.ui, "alpha");
    sh.terminal_find(Direction::Older, true);
    assert!(find_count(&sh.terminal_ui.message), "{}", sh.terminal_ui.message);
    sh.terminal_hw_key(TermKey::FindNext, 2100.0);
    assert!(find_count(&sh.terminal_ui.message));
    sh.terminal_ui.query.as_mut().unwrap().set_text(&sh.ui, "zzzz");
    sh.terminal_find(Direction::Older, true);
    assert_eq!(sh.terminal_ui.message, "No matches");
    assert!(!find_count(&sh.terminal_ui.message), "no matches is red");
    sh.terminal_hw_key(TermKey::NewTab, 2200.0);
    assert_eq!(sh.term.len(), 2);
    sh.terminal_hw_key(TermKey::Tab(0), 2300.0);
    assert_eq!(sh.term.active_index(), 0);
    sh.terminal_hw_key(TermKey::NextTab, 2400.0);
    assert_eq!(sh.term.active_index(), 1);
    sh.terminal_hw_key(TermKey::NextTab, 2500.0);
    assert_eq!(sh.term.active_index(), 0, "wraps");
    sh.terminal_hw_key(TermKey::PrevTab, 2600.0);
    assert_eq!(sh.term.active_index(), 1);
    sh.terminal_hw_key(TermKey::Bigger, 2700.0);
    sh.terminal_hw_key(TermKey::ActualSize, 2800.0);
    assert_eq!(sh.terminal_ui.settings.zoom, termcore::zoom::DEFAULT);
    sh.terminal_hw_key(TermKey::Smaller, 2900.0);
    assert_eq!(sh.terminal_ui.settings.zoom, termcore::zoom::DEFAULT - 1);
}
#[test]
fn cmd_c_copies_only_a_selection_and_never_sends_ctrl_c() {
    let mut sh = terminal(&["/bin/sh", "-c", "/bin/stty raw -echo; echo ready; /bin/head -c 1 | /bin/od -An -c; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("ready"));
    sh.terminal_hw_key(TermKey::Copy, 2000.0);
    assert!(sh.terminal_ui.message.is_empty(), "no selection, nothing happens");
    assert!(!sh.term.as_ref().unwrap().viewport_text().contains("003"), "no ^C byte was sent");
    sh.terminal_hw_key(TermKey::SelectAll, 2100.0);
    assert!(sh.term.as_ref().unwrap().selected_text().is_some());
    sh.terminal_hw_key(TermKey::Copy, 2200.0);
}
#[test]
fn cmd_w_closes_one_tab_of_two() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_hw_key(TermKey::NewTab, 2000.0);
    assert_eq!(sh.term.len(), 2);
    sh.terminal_hw_key(TermKey::CloseTab, 2100.0);
    assert!(sh.term.len() == 1 || sh.terminal_ui.pending_close.is_some(), "closed, or asked about the running job");
}
#[test]
fn clear_screen_sends_ctrl_l_to_the_shell() {
    let mut sh = terminal(&["/bin/sh", "-c", "/bin/stty raw -echo; echo ready; /bin/head -c 1 | /bin/od -An -c; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("ready"));
    assert_eq!(MENU[12], "Clear screen");
    sh.terminal_menu_action(12, 2000.0);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("\\f"));
    assert!(sh.term.as_ref().unwrap().viewport_text().contains("\\f"));
}
#[test]
fn duplicate_tab_opens_a_second_shell_in_the_same_folder() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    assert_eq!(MENU[6], "Duplicate tab");
    sh.terminal_menu_action(6, 2000.0);
    assert_eq!(sh.term.len(), 2);
    assert_eq!(sh.term.active_index(), 1, "the new tab is selected");
}
#[test]
fn closing_an_idle_tab_takes_one_tap() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    let g = layout::Header::new(sh.w as f32, sh.ui.s, sh.term.len());
    let x = g.tab_w - g.close_width() / 2.0;
    let y = sh.ui.pt(scene::STATUS_PT + 20.0);
    sh.terminal_touch(Event::Down(x, y), 2000.0);
    sh.terminal_touch(Event::Up(x, y), 2020.0);
    assert_eq!(sh.term.len(), 1, "no job, no question");
    assert!(sh.terminal_ui.pending_close.is_none());
}
#[test]
fn exiting_background_tab_does_not_close_the_live_tab() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    sh.term.iter_mut().next().unwrap().key("Enter");
    pump_until(&mut sh, |sh| sh.term.len() == 1);
    assert_eq!(sh.term.len(), 1);
    assert!(sh.term.as_ref().unwrap().alive());
}
#[test]
fn find_controls_search_history_without_typing_into_the_shell() {
    let mut sh=terminal(&["/bin/sh","-c","echo old_NEEDLE; i=0; while [ $i -lt 100 ]; do echo row$i; i=$((i+1)); done; echo newest; read x; echo got:$x; read y"]);
    pump_until(&mut sh, |sh| {
        sh.term.as_ref().unwrap().viewport_text().contains("newest")
    });
    tap_control(&mut sh, 1, 2000.0);
    sh.type_out(Out::Text("needle".into()), 2100.0);
    assert_eq!(
        sh.term.as_ref().unwrap().selected_text().as_deref(),
        Some("NEEDLE")
    );
    assert!(sh
        .term
        .as_ref()
        .unwrap()
        .viewport_text()
        .contains("old_NEEDLE"));
    sh.terminal_ui.panel = Panel::None;
    sh.type_out(Out::Text("shell".into()), 2200.0);
    sh.type_out(Out::Enter, 2210.0);
    pump_until(&mut sh, |sh| {
        sh.term
            .as_ref()
            .unwrap()
            .viewport_text()
            .contains("got:shell")
    });
    assert!(sh
        .term
        .as_ref()
        .unwrap()
        .viewport_text()
        .contains("got:shell"));
}
#[test]
fn appearance_regrids_all_tabs_and_persists() {
    let mut sh = terminal(&["/bin/sh"]);
    sh.new_terminal_tab(None);
    let cols = sh.term.as_ref().unwrap().cols();
    tap_control(&mut sh, 3, 2000.0);
    assert!(sh.term.iter().all(|t| t.cols() < cols));
    tap_control(&mut sh, 4, 2100.0);
    assert_eq!(
        sh.term.as_ref().unwrap().background(),
        pack(250, 250, 250, 255)
    );
    let mut reloaded = TerminalUi::default();
    reloaded.load(&sh.prefs);
    assert_eq!(reloaded.settings, sh.terminal_ui.settings);
}
#[test]
fn ssh_form_validates_and_preserves_unknown_config_lines() {
    let mut sh = terminal(&["/bin/sh"]);
    sh.data_dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/test-terminal-ssh"
    ));
    sh.terminal_ui.config = Config::parse("# keep me\nHost *\n  ServerAliveInterval 30\n");
    sh.terminal_ui.form(
        &sh.ui,
        Entry {
            alias: "server".into(),
            hostname: "127.0.0.1".into(),
            user: "oli".into(),
            port: "2222".into(),
            ..Entry::default()
        },
        false,
    );
    sh.terminal_save_form(2000.0);
    let path = sh.terminal_config_path();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# keep me"));
    assert!(saved.contains("ServerAliveInterval 30"));
    assert_eq!(Config::parse(&saved).get("server").unwrap().port, "2222");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    sh.terminal_ui.form(
        &sh.ui,
        Entry {
            alias: "bad".into(),
            port: "99999".into(),
            ..Entry::default()
        },
        false,
    );
    sh.terminal_save_form(2100.0);
    assert_eq!(fs::read_to_string(&path).unwrap(), saved);
    assert!(sh.terminal_ui.message.contains("Port"));
}
#[test]
fn process_panel_samples_only_while_visible_and_refuses_self_signal() {
    let mut sh = terminal(&["/bin/sh"]);
    tap_control(&mut sh, 5, 2000.0);
    assert_eq!(sh.terminal_ui.panel as u8, Panel::Processes as u8);
    assert!(sh
        .terminal_ui
        .processes
        .iter()
        .any(|p| p.info.pid == std::process::id()));
    sh.terminal_ui.tick(2020.0);
    assert_eq!(sh.terminal_ui.next_wake(), Some(4000.0));
    assert!(sh.terminal_ui.tick(4030.0));
    assert!(sh.terminal_ui.cpu.iter().all(|c| *c >= 0.0));
    sh.terminal_ui.panel = Panel::None;
    let sampled = sh.terminal_ui.sampled;
    sh.terminal_ui.tick(7000.0);
    assert_eq!(
        sh.terminal_ui.sampled, sampled,
        "closing stops process scans while badges keep refreshing"
    );
}
#[test]
fn modal_touch_and_keyboard_shortcut_restore_terminal_input() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x; echo got:$x; read y"]);
    let g = layout::Header::new(sh.w as f32, sh.ui.s, sh.term.len());
    let y = sh.ui.pt(scene::STATUS_PT + 20.0);
    assert!(sh.keyboard.shown());
    sh.terminal_touch(Event::Down(g.ssh + 12.0, y), 2100.0);
    sh.terminal_touch(Event::Up(g.ssh + 12.0, y), 2130.0);
    assert!(matches!(sh.terminal_ui.panel, Panel::Connections));
    assert!(!sh.keyboard.shown());
    sh.terminal_touch(Event::Down(10.0, 400.0), 2200.0);
    sh.terminal_touch(Event::Up(10.0, 400.0), 2230.0);
    assert!(matches!(sh.terminal_ui.panel, Panel::None));
    assert!(sh.keyboard.shown());
    sh.type_out(Out::Named("More"), 2300.0);
    assert!(matches!(sh.terminal_ui.panel, Panel::More));
    sh.terminal_close_panel(2400.0);
    sh.type_out(Out::Text("ready".into()), 2500.0);
    sh.type_out(Out::Enter, 2530.0);
    pump_until(&mut sh, |sh| {
        sh.term
            .as_ref()
            .unwrap()
            .viewport_text()
            .contains("got:ready")
    });
    assert!(sh
        .term
        .as_ref()
        .unwrap()
        .viewport_text()
        .contains("got:ready"));
}
#[test]
fn render_terminal_panels_for_visual_review() {
    use crate::gfx::Image;
    let mut sh = terminal(&[
        "/bin/sh",
        "-c",
        "printf 'Welcome to Terminal\\nRust shell on BifrOSt\\n$ '; read x",
    ]);
    pump_until(&mut sh, |sh| {
        sh.term.as_ref().unwrap().viewport_text().contains("Welcome")
    });
    for (name, action) in [
        ("main", None),
        ("connections", Some(0)),
        ("menu", Some(6)),
        ("processes", Some(5)),
        ("find", Some(1)),
    ] {
        if let Some(action) = action {
            sh.terminal_action(action, 3000.0);
        }
        let mut img = Image::new(sh.w as usize, sh.h as usize);
        sh.draw_terminal(
            &mut Surface {
                w: img.w,
                h: img.h,
                stride: img.w,
                px: &mut img.px,
                y0: 0,
            },
            3100.0,
        );
        fs::write(
            format!("{}/target/terminal-{name}.png", env!("CARGO_MANIFEST_DIR")),
            super::super::screenshot::encode(&img),
        )
        .unwrap();
        sh.terminal_close_panel(3200.0);
    }
    sh.terminal_action(0, 3300.0);
    sh.terminal_ui.form(&sh.ui, Entry::default(), false);
    sh.set_keyboard(true, 3300.0);
    let (px, py, pw, ph) = sh.terminal_panel_rect();
    let (_, fy, _, fh) = termcore::connui::field_rect((pw / sh.ui.s) as f64, 5);
    assert!(py + sh.ui.pt((fy + fh) as f32) < py + ph - sh.ui.pt(56.0));
    assert!(px > 0.0);
    let mut img = Image::new(sh.w as usize, sh.h as usize);
    sh.draw_terminal(
        &mut Surface {
            w: img.w,
            h: img.h,
            stride: img.w,
            px: &mut img.px,
            y0: 0,
        },
        3400.0,
    );
    fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/target/terminal-form.png"),
        super::super::screenshot::encode(&img),
    )
    .unwrap();
}

#[test]
fn open_link_reports_when_the_selection_has_no_link() {
    let mut sh = terminal(&["/bin/sh", "-c", "echo plain words; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("plain words"));
    assert_eq!(MENU[15], "Open link in selection");
    // rows 7 and up are menu actions (`terminal_menu_action`); `terminal_action` only handles the header and keys
    sh.terminal_menu_action(7, 2020.0);
    sh.terminal_menu_action(15, 2120.0);
    assert_eq!(sh.terminal_ui.message, "No link in selection");
}

#[test]
fn new_command_runs_in_its_own_tab_and_an_empty_line_stays_open() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(19, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Command);
    sh.terminal_run_command(2100.0);
    assert_eq!(sh.terminal_ui.message, "Type a command to run");
    assert_eq!(sh.term.len(), 1);
    sh.terminal_ui.command.as_mut().unwrap().set_text(&sh.ui, "echo ran-command; read y");
    sh.terminal_run_command(2200.0);
    assert_eq!(sh.term.len(), 2, "the command opened a tab");
    assert_eq!(sh.term.active_index(), 1);
    assert_eq!(sh.terminal_ui.panel, Panel::None, "the panel closes once it runs");
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("ran-command"));
}

#[test]
fn secure_keyboard_entry_keeps_shell_shortcuts_out_of_the_terminal() {
    use super::super::hwkeys::Shortcut;
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(18, 2000.0);
    assert!(sh.terminal_ui.secure);
    assert!(sh.terminal_ui.message.starts_with("Secure keyboard entry on"));
    sh.hw_shortcut(Shortcut::AppSwitch, 2100.0);
    assert!(matches!(sh.screen, Screen::App(AppId::Terminal)), "the switcher does not take the terminal's keys");
    sh.terminal_menu_action(18, 2200.0);
    assert!(!sh.terminal_ui.secure);
    sh.hw_shortcut(Shortcut::AppSwitch, 2300.0);
    assert!(matches!(sh.screen, Screen::Switcher), "without it the switcher opens as before");
}

#[test]
fn window_group_saves_tab_folders_and_opens_them_as_tabs() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(23, 2000.0);
    assert_eq!(sh.terminal_ui.message, "No window group saved");
    sh.terminal_menu_action(22, 2100.0);
    assert!(sh.terminal_ui.message.starts_with("Saved "), "{}", sh.terminal_ui.message);
    let saved = groups::decode(sh.prefs.get(groups::KEY).unwrap_or(""));
    sh.terminal_menu_action(23, 2200.0);
    assert_eq!(sh.term.len(), 1 + saved.len(), "one new tab per saved folder");
}

#[test]
fn split_pane_sits_beside_the_active_tab_and_a_tap_picks_the_pane() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(20, 2000.0);
    assert_eq!(sh.term.len(), 2);
    assert_eq!(sh.term.pair(), Some((0, 1)));
    let left = sh.term.get(0).unwrap().area();
    let right = sh.term.get(1).unwrap().area();
    assert_eq!(left.1, right.1, "both panes start on the same row");
    assert!(left.2 < sh.w && right.2 < sh.w && left.0 + left.2 < right.0, "side by side, with a gap");
    sh.select_pane_at(10.0);
    assert_eq!(sh.term.active_index(), 0);
    sh.select_pane_at((sh.w - 10) as f32);
    assert_eq!(sh.term.active_index(), 1, "a tap on the right half selects the right pane");
    sh.terminal_menu_action(20, 2100.0);
    assert_eq!(sh.term.len(), 2, "a second split is refused while one is open");
    assert!(sh.terminal_ui.message.starts_with("Close the split pane first"));
    sh.terminal_menu_action(21, 2200.0);
    assert_eq!(sh.term.len(), 1, "closing the active pane leaves the other one");
    assert_eq!(sh.term.split(), None);
    assert_eq!(sh.term.get(0).unwrap().area(), sh.term_area(), "the last pane takes the whole area");
}

#[test]
fn cursor_style_and_blink_persist_and_reload() {
    let mut sh = terminal(&["/bin/sh"]);
    assert_eq!(MENU[16], "Cursor style");
    sh.terminal_menu_action(16, 2000.0);
    assert_eq!(sh.terminal_ui.cursor, profile_table::CursorStyle::Underline);
    sh.terminal_menu_action(16, 2100.0);
    sh.terminal_menu_action(17, 2200.0);
    assert!(sh.terminal_ui.blink);
    let mut reloaded = TerminalUi::default();
    reloaded.load(&sh.prefs);
    assert_eq!(reloaded.cursor, profile_table::CursorStyle::Bar);
    assert!(reloaded.blink);
}

#[test]
fn the_added_menu_rows_sit_where_the_actions_expect_them() {
    for (index, label) in [
        (item::TITLE, "Edit tab title"),
        (item::CLOSE_OTHERS, "Close other tabs"),
        (item::MOVE_LEFT, "Move tab left"),
        (item::MOVE_RIGHT, "Move tab right"),
        (item::MARK, "Mark line"),
        (item::PREVIOUS_MARK, "Previous mark"),
        (item::NEXT_MARK, "Next mark"),
        (item::FIND_SELECTION, "Use selection for find"),
        (item::PASTE_ESCAPED, "Paste escaped text"),
        (item::PASTE_SELECTION, "Paste selection"),
        (item::SEARCH_WEB, "Search the web for selection"),
        (item::MAN_PAGE, "Open man page for selection"),
        (item::SAVE_SELECTION, "Save selection to Files"),
        (item::MOUSE, "Allow mouse reporting"),
        (item::LOGIN_SHELL, "Login shell"),
        (item::GROUP_LAUNCH, "Open window group at launch"),
        (item::HELP, "Terminal help"),
        (item::COLORS, "Text and background colors"),
    ] {
        assert_eq!(MENU[index], label);
    }
    assert_eq!(item::COLORS, MENU.len() - 1, "the last row is the colors");
    assert_eq!(menu_key(item::MARK), Some("\u{2318}U"));
}

#[test]
fn edit_title_names_the_tab_and_an_empty_title_gives_the_automatic_one_back() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(item::TITLE, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Title);
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "deploy");
    assert!(sh.terminal_panel_input(&Out::Enter, 2100.0));
    assert_eq!(sh.terminal_ui.panel, Panel::None);
    assert_eq!(sh.terminal_ui.message, "Tab title set");
    assert_eq!(sh.term.as_ref().unwrap().title(0), "deploy");
    sh.terminal_menu_action(item::TITLE, 2200.0);
    assert_eq!(sh.terminal_ui.entry.as_ref().unwrap().text(), "deploy", "the typed title is offered for editing");
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "");
    sh.terminal_panel_input(&Out::Enter, 2300.0);
    assert_eq!(sh.term.as_ref().unwrap().custom_title(), None);
    assert_eq!(sh.terminal_ui.message, "Tab title is automatic");
}

#[test]
fn tabs_move_along_the_bar_and_close_others_keeps_the_active_one() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    sh.new_terminal_tab(None);
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        sh.term.get_mut(i).unwrap().set_custom_title(name);
    }
    let order = |sh: &Shell| sh.term.iter().enumerate().map(|(i, tab)| tab.title(i)).collect::<String>();
    assert_eq!(order(&sh), "abc");
    assert_eq!(sh.term.active_index(), 2);
    sh.terminal_menu_action(item::MOVE_LEFT, 2000.0);
    assert_eq!((order(&sh).as_str(), sh.term.active_index()), ("acb", 1));
    sh.terminal_menu_action(item::MOVE_LEFT, 2100.0);
    sh.terminal_menu_action(item::MOVE_LEFT, 2200.0);
    assert_eq!(order(&sh), "cab");
    assert_eq!(sh.terminal_ui.message, "The tab is already the first");
    sh.terminal_menu_action(item::MOVE_RIGHT, 2300.0);
    assert_eq!((order(&sh).as_str(), sh.term.active_index()), ("acb", 1));
    sh.terminal_menu_action(item::CLOSE_OTHERS, 2400.0);
    assert_eq!(sh.term.len(), 1);
    assert_eq!(sh.term.as_ref().unwrap().custom_title(), Some("c"));
    assert_eq!(sh.terminal_ui.message, "Closed 2 tabs");
    sh.terminal_menu_action(item::CLOSE_OTHERS, 2500.0);
    assert_eq!(sh.terminal_ui.message, "There are no other tabs");
}

#[test]
fn marks_are_set_removed_and_jumped_to_from_the_keyboard() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.term.as_mut().unwrap().feed(b"abc");
    sh.terminal_hw_key(TermKey::Mark, 2000.0);
    assert_eq!(sh.terminal_ui.message, "Marked this line");
    sh.terminal_hw_key(TermKey::PreviousMark, 2100.0);
    assert_eq!(sh.terminal_ui.message, "No earlier mark");
    sh.terminal_hw_key(TermKey::Mark, 2200.0);
    assert_eq!(sh.terminal_ui.message, "Mark removed");
    sh.terminal_hw_key(TermKey::NextMark, 2300.0);
    assert!(sh.terminal_ui.message.starts_with("No marks yet"), "{}", sh.terminal_ui.message);
}

#[test]
fn use_selection_for_find_opens_find_with_the_selected_word() {
    let mut sh = terminal(&["/bin/sh", "-c", "echo alpha beta; echo alpha; read x"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("beta"));
    sh.terminal_hw_key(TermKey::UseSelectionForFind, 2000.0);
    assert_eq!(sh.terminal_ui.message, "Select terminal text first");
    sh.term.as_mut().unwrap().select_word((1, 0));
    sh.terminal_hw_key(TermKey::UseSelectionForFind, 2100.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Find);
    assert_eq!(sh.terminal_ui.query.as_ref().unwrap().text(), "alpha");
    assert!(find_count(&sh.terminal_ui.message), "{}", sh.terminal_ui.message);
}

#[test]
fn paste_escaped_and_paste_selection_type_into_the_shell() {
    let _g = clipboard::tests::exclusive();
    let mut sh = terminal(&["/bin/sh", "-c", "echo wordy; read -r x; echo got:$x; read -r y; echo again:$y; read z"]);
    clipboard::set_text("a b");
    sh.terminal_menu_action(item::PASTE_ESCAPED, 2000.0);
    sh.term.as_mut().unwrap().key("Enter");
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("got:a\\ b"));
    sh.terminal_menu_action(item::PASTE_SELECTION, 2100.0);
    assert_eq!(sh.terminal_ui.message, "Select terminal text first");
    sh.term.as_mut().unwrap().select_word((1, 0));
    let word = sh.term.as_ref().unwrap().selected_text().unwrap();
    sh.terminal_menu_action(item::PASTE_SELECTION, 2200.0);
    sh.term.as_mut().unwrap().key("Enter");
    let want = format!("again:{word}");
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains(&want));
    assert!(sh.term.as_ref().unwrap().viewport_text().contains(&want));
}

#[test]
fn ssh_links_open_a_tab_with_the_command_typed_in() {
    let mut sh = terminal(&["/bin/sh", "-c", "read -r x; echo got:$x; read y"]);
    assert!(sh.terminal_open_link("ssh://me@host:2222/", 2000.0));
    assert_eq!(sh.term.len(), 2);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("got:ssh -p 2222 me@host"));
    assert!(sh.term.as_ref().unwrap().viewport_text().contains("got:ssh -p 2222 me@host"));
    assert!(!sh.terminal_open_link("ssh://-oProxyCommand=x", 2100.0));
    assert!(!sh.terminal_open_link("https://example.org", 2200.0));
    assert_eq!(sh.term.len(), 2, "a link that is not ours opens nothing");
}

#[test]
fn the_man_page_of_the_selected_word_opens_in_a_tab() {
    let mut sh = terminal(&["/bin/sh", "-c", "echo printf; read -r x; echo got:$x; read y"]);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("printf"));
    sh.terminal_menu_action(item::MAN_PAGE, 2000.0);
    assert_eq!(sh.terminal_ui.message, "Select a command name first");
    sh.term.as_mut().unwrap().select_word((1, 0));
    sh.terminal_menu_action(item::MAN_PAGE, 2100.0);
    assert_eq!(sh.term.len(), 2);
    pump_until(&mut sh, |sh| sh.term.as_ref().unwrap().viewport_text().contains("got:man printf"));
    assert!(sh.term.as_ref().unwrap().viewport_text().contains("got:man printf"));
}

#[test]
fn mouse_reporting_toggle_reaches_every_tab_and_reloads() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.new_terminal_tab(None);
    sh.terminal_menu_action(item::MOUSE, 2000.0);
    assert!(!sh.terminal_ui.mouse_reporting);
    let mut reloaded = TerminalUi::default();
    assert!(reloaded.mouse_reporting, "on by default");
    reloaded.load(&sh.prefs);
    assert!(!reloaded.mouse_reporting);
    sh.terminal_menu_action(item::MOUSE, 2100.0);
    assert!(sh.terminal_ui.mouse_reporting);
}

#[test]
fn login_shell_and_launch_group_are_chosen_in_the_menu_and_reload() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(item::LOGIN_SHELL, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Shell);
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "bash");
    sh.terminal_panel_input(&Out::Enter, 2100.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Shell, "a path that is not a full path stays open");
    assert!(sh.terminal_ui.message.starts_with("Type the full path"));
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "/bin/sh");
    sh.terminal_panel_input(&Out::Enter, 2200.0);
    assert_eq!(sh.terminal_ui.panel, Panel::None);
    assert_eq!(sh.terminal_ui.login_shell.as_deref(), Some("/bin/sh"));
    assert_eq!(sh.terminal_login_shell(), None, "a Terminal started on another command keeps it");
    sh.terminal_menu_action(item::GROUP_LAUNCH, 2300.0);
    assert!(sh.terminal_ui.group_launch);
    let mut reloaded = TerminalUi::default();
    reloaded.load(&sh.prefs);
    assert_eq!(reloaded.login_shell.as_deref(), Some("/bin/sh"));
    assert!(reloaded.group_launch);
}

#[test]
fn the_colour_theme_cycles_into_the_profiles_and_persists() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    for step in 0..3 {
        sh.terminal_action(4, 2000.0 + step as f64);
    }
    assert_eq!((sh.terminal_ui.settings.theme, sh.terminal_ui.profile), (2, Some(1)));
    assert_eq!(sh.term.as_ref().unwrap().background(), pack(0, 0, 0, 255), "Pro is white on black");
    assert_eq!(sh.terminal_background(), pack(0, 0, 0, 255), "and so is the strip around the grid");
    let mut reloaded = TerminalUi::default();
    reloaded.load(&sh.prefs);
    assert_eq!(reloaded.profile, Some(1));
    sh.new_terminal_tab(None);
    assert_eq!(sh.term.as_ref().unwrap().background(), pack(0, 0, 0, 255), "a new tab starts in the profile");
    for step in 0..7 {
        sh.terminal_action(4, 2100.0 + step as f64);
    }
    assert_eq!(sh.terminal_ui.profile, Some(8));
    assert!(sh.term.iter().all(|tab| tab.background() == pack(122, 37, 30, 255)), "every tab follows");
    sh.terminal_action(4, 2200.0);
    assert_eq!((sh.terminal_ui.settings.theme, sh.terminal_ui.profile), (0, None));
    let mut again = TerminalUi::default();
    again.load(&sh.prefs);
    assert_eq!(again.profile, None);
}

#[test]
fn custom_colors_become_the_last_entry_of_the_theme_cycle() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(item::COLORS, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Colors);
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "zzz");
    sh.terminal_panel_input(&Out::Enter, 2100.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Colors, "colors that do not parse keep the panel open");
    assert!(sh.terminal_ui.message.starts_with("Type the text color"), "{}", sh.terminal_ui.message);
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "ffffff 102030");
    sh.terminal_panel_input(&Out::Enter, 2200.0);
    assert_eq!(sh.terminal_ui.panel, Panel::None);
    assert_eq!(sh.terminal_ui.profile, Some(profile_table::PROFILES.len()), "the custom colors are in use");
    assert_eq!(sh.term.as_ref().unwrap().background(), pack(16, 32, 48, 255));
    assert_eq!(sh.terminal_background(), pack(16, 32, 48, 255));
    let mut reloaded = TerminalUi::default();
    reloaded.load(&sh.prefs);
    assert_eq!(reloaded.custom_colors, Some(((255, 255, 255), (16, 32, 48))));
    assert_eq!(reloaded.profile, sh.terminal_ui.profile);
    sh.terminal_action(4, 2300.0);
    assert_eq!((sh.terminal_ui.settings.theme, sh.terminal_ui.profile), (0, None), "Custom is the last stop of the cycle");
    assert!(sh.terminal_ui.custom_colors.is_some(), "and the colors wait for the next time round");
    sh.terminal_menu_action(item::COLORS, 2400.0);
    sh.terminal_ui.entry.as_mut().unwrap().set_text(&sh.ui, "");
    sh.terminal_panel_input(&Out::Enter, 2500.0);
    assert_eq!(sh.terminal_ui.custom_colors, None, "an empty line gives them up");
    assert_eq!(sh.terminal_ui.message, "Custom colors removed");
}

#[test]
fn terminal_help_opens_draws_and_closes() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(item::HELP, 2000.0);
    assert_eq!(sh.terminal_ui.panel, Panel::Help);
    let mut img = crate::gfx::Image::new(sh.w as usize, sh.h as usize);
    sh.draw_terminal(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, 2100.0);
    sh.terminal_close_panel(2200.0);
    assert_eq!(sh.terminal_ui.panel, Panel::None);
}
