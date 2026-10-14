//! Parity app-terminal: headless tests of the Terminal's tab order and of the links that run a command in a new tab
//! (ssh, telnet and x-man-page). Nothing here starts a shell, opens a window or reads the network.

use springboard::shell::terminal::schemes::{first_scheme_link, link_argv};
use springboard::term::tab_order::{moved_index, neighbour};

fn argv(parts: &[&str]) -> Option<Vec<String>> {
    Some(parts.iter().map(|p| p.to_string()).collect())
}

#[test]
fn a_moved_tab_lands_at_its_target_position() {
    assert_eq!(moved_index(0, 0, 3), 3);
    assert_eq!(moved_index(3, 3, 1), 1);
}

#[test]
fn the_tabs_passed_over_by_a_move_each_shift_by_one() {
    // tabs 0 1 2 3, move the tab at 0 to 2: the order becomes 1 2 0 3
    assert_eq!(moved_index(1, 0, 2), 0);
    assert_eq!(moved_index(2, 0, 2), 1);
    // tabs 0 1 2 3, move the tab at 3 to 1: the order becomes 0 3 1 2
    assert_eq!(moved_index(1, 3, 1), 2);
    assert_eq!(moved_index(2, 3, 1), 3);
}

#[test]
fn a_tab_outside_the_moved_range_stays_put() {
    assert_eq!(moved_index(4, 0, 2), 4);
    assert_eq!(moved_index(0, 2, 3), 0);
}

#[test]
fn move_left_and_right_stop_at_the_ends_of_the_bar() {
    assert_eq!(neighbour(4, 1, true), Some(2));
    assert_eq!(neighbour(4, 1, false), Some(0));
    assert_eq!(neighbour(4, 3, true), None, "the last tab cannot move right");
    assert_eq!(neighbour(4, 0, false), None, "the first tab cannot move left");
    assert_eq!(neighbour(1, 0, true), None, "a single tab has nowhere to go");
}

#[test]
fn an_ssh_link_runs_ssh_with_the_user_host_and_port() {
    assert_eq!(link_argv("ssh://example.org"), argv(&["ssh", "example.org"]));
    assert_eq!(link_argv("ssh://ops@10.0.0.2:2222"), argv(&["ssh", "-p", "2222", "ops@10.0.0.2"]));
}

#[test]
fn a_telnet_link_runs_telnet_with_the_host_and_port_and_no_user() {
    assert_eq!(link_argv("telnet://switch.lan"), argv(&["telnet", "switch.lan"]));
    assert_eq!(link_argv("telnet://switch.lan:2323"), argv(&["telnet", "switch.lan", "2323"]));
    assert_eq!(link_argv("telnet://admin@switch.lan"), None);
}

#[test]
fn an_x_man_page_link_runs_man_with_an_optional_section() {
    assert_eq!(link_argv("x-man-page://printf"), argv(&["man", "printf"]));
    assert_eq!(link_argv("x-man-page://3/printf"), argv(&["man", "3", "printf"]));
    assert_eq!(link_argv("x-man-page://3/a/b"), None, "two path parts beyond the section are not a page");
}

#[test]
fn a_link_with_an_option_or_a_shell_character_runs_nothing() {
    assert_eq!(link_argv("ssh://-oProxyCommand=x"), None);
    assert_eq!(link_argv("ssh://host;rm"), None);
    assert_eq!(link_argv("ssh://host:0"), None, "port zero is not a port");
    assert_eq!(link_argv("x-man-page://-k"), None);
}

#[test]
fn a_web_link_is_not_a_command_link() {
    assert_eq!(link_argv("https://example.org"), None);
}

#[test]
fn the_first_usable_command_link_in_a_selection_is_found() {
    assert_eq!(first_scheme_link("see ssh://a.lan. then telnet://b.lan").as_deref(), Some("ssh://a.lan"));
    assert_eq!(first_scheme_link("ssh://-x telnet://b.lan:23,").as_deref(), Some("telnet://b.lan:23"));
    assert_eq!(first_scheme_link("https://example.org only"), None);
}
