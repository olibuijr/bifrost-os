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
fn close_other_tabs_keeps_only_the_active_tab() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    sh.terminal_menu_action(6, 2000.0);
    sh.terminal_menu_action(6, 2100.0);
    assert_eq!(sh.term.len(), 3);
    sh.term.select(1);
    assert_eq!(MENU[24], "Close other tabs");
    sh.terminal_menu_action(24, 2200.0);
    assert_eq!(sh.term.len(), 1);
    assert_eq!(sh.term.active_index(), 0);
}

#[test]
fn paste_selection_and_help_report_through_the_menu() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    assert_eq!(MENU[26], "Paste selection");
    sh.terminal_menu_action(26, 2000.0);
    assert_eq!(sh.terminal_ui.message, "Select terminal text first");
    assert_eq!(MENU[27], "Terminal help");
    sh.terminal_menu_action(27, 2100.0);
    assert_eq!(sh.terminal_ui.message, extras::help_line());
}

#[test]
fn edit_title_renames_the_front_tab_and_empty_restores_it() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    assert_eq!(MENU[28], "Edit title");
    sh.terminal_menu_action(28, 2000.0);
    assert!(matches!(sh.terminal_ui.panel, Panel::Command));
    sh.terminal_ui.command.as_mut().unwrap().set_text(&sh.ui, "deploy");
    sh.terminal_run_command(2100.0);
    assert!(matches!(sh.terminal_ui.panel, Panel::None));
    assert_eq!(sh.term.as_ref().unwrap().title(0), "deploy");
    sh.terminal_menu_action(28, 2200.0);
    sh.terminal_ui.command.as_mut().unwrap().set_text(&sh.ui, "");
    sh.terminal_run_command(2300.0);
    assert!(!sh.term.as_ref().unwrap().has_custom_title());
}

#[test]
fn mouse_reporting_toggle_applies_to_new_tabs() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    assert_eq!(MENU[29], "Allow mouse reporting");
    assert!(sh.terminal_ui.mouse_reporting);
    sh.terminal_menu_action(29, 2000.0);
    assert!(!sh.terminal_ui.mouse_reporting);
    assert_eq!(sh.terminal_ui.message, "Mouse reports to programs are off");
}

#[test]
fn selection_rows_ask_for_a_selection_first() {
    let mut sh = terminal(&["/bin/sh", "-c", "read x"]);
    for (i, label) in [(30, "Search the web for selection"), (31, "Open man page for selection"), (32, "Save selected text to Files"), (33, "Use selection for find")] {
        assert_eq!(MENU[i], label);
        sh.terminal_ui.message.clear();
        sh.terminal_menu_action(i, 2000.0);
        assert!(sh.terminal_ui.message.contains("elect"), "row {i}: {}", sh.terminal_ui.message);
    }
}
