use super::ext_state_text;

#[test]
fn state_text_ends_every_line_so_the_next_section_starts_its_own() {
    let text = ext_state_text(&["is.a.calendar"], &[("is.a.calendar", 62, 0), ("is.a.browser", 15, 0xf)]);
    assert_eq!(text, "apps_running=is.a.calendar\napp_controls=is.a.calendar:62:0,is.a.browser:15:f\n");
    // the shell appends the window lines straight after: each is a line of its own
    let joined = format!("{text}window=Settings,965,306,909,846\n");
    assert!(joined.lines().any(|l| l.starts_with("window=Settings")));
    assert!(joined.lines().all(|l| l.matches('=').count() >= 1));
}

#[test]
fn state_text_with_no_apps_is_still_two_whole_lines() {
    assert_eq!(ext_state_text(&[], &[]), "apps_running=\napp_controls=\n");
}

/// E2E N2: a native app the shell launched runs while it shows a toplevel, though it has no app host.
#[test]
fn a_native_app_with_a_toplevel_is_running() {
    use super::native_running;
    let manifests = [("ai.film", "native", "ai.film"), ("is.notes", "sdk", ""), ("x.idle", "native", "x.idle"), ("x.blank", "native", "")];
    assert_eq!(native_running(&manifests, ["ai.film", "ai.film", "org.linux.app"].into_iter()), vec!["ai.film".to_string()]);
    assert!(native_running(&manifests, std::iter::empty()).is_empty(), "no toplevel, not running");
    assert!(native_running(&manifests, [""].into_iter()).is_empty(), "an app without a Wayland id never matches");
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn the_foreign_windows_line_names_each_window_once_and_the_active_one() {
    use crate::bare::wayland::ForeignWindow;
    let w = |id, app_id: &str, active| ForeignWindow { id, title: "t, with a comma".into(), app_id: app_id.into(), active, maximized: false, minimized: false };
    let windows = [w(7, "ai.film", true), w(9, "org.x", false), w(7, "ai.film", true)];
    assert_eq!(super::foreign_state_line(windows.iter()), "foreign_windows=ai.film:7:1,org.x:9:0\n");
    assert_eq!(super::foreign_state_line(std::iter::empty()), "foreign_windows=\n");
}
