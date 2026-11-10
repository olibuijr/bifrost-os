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

/// E2E 0.10.49: an app that quit by itself or crashed keeps its host (status `Exited`) in its slot, and the state
/// report listed its controls as if it still ran. The report and the accessibility map list running apps only.
#[cfg(feature = "desktop-wayland")]
#[test]
fn an_app_that_exited_leaves_the_state_report_and_the_accessibility_map() {
    use super::{AppHost, Status};
    let mut sh = crate::shell::tests::desktop_shell();
    let cfg = bifrost_proto::Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut peers = Vec::new();
    for id in ["is.a.notes", "is.a.mail"] {
        let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
        peers.push(peer);
        let mut host = AppHost::with_channel(id, mine, None, cfg);
        host.commands = bifrost_proto::Commands(0xf);
        host.controls = vec![bifrost_proto::Node { id: 1, role: bifrost_proto::Role::Button, label: "Send".into(), value: String::new(), bounds: bifrost_proto::UiRect { x: 0., y: 0., w: 10., h: 10. }, enabled: true }];
        sh.ext.push(Some(host));
    }
    let line = |sh: &crate::shell::Shell, key: &str| sh.describe_ext().lines().find(|l| l.starts_with(key)).unwrap().to_string();
    assert_eq!(line(&sh, "apps_running="), "apps_running=is.a.notes,is.a.mail");
    assert_eq!(line(&sh, "app_controls="), "app_controls=is.a.notes:1:f,is.a.mail:1:f");
    // Mail exits (a quit of its own, or a crash): the host stays in the slot, as `pump` leaves it
    sh.ext[1].as_mut().unwrap().status = Status::Exited(139);
    assert_eq!(line(&sh, "apps_running="), "apps_running=is.a.notes");
    assert_eq!(line(&sh, "app_controls="), "app_controls=is.a.notes:1:f");
    let map = sh.ax_map(0.);
    assert!(map.under("app/is.a.notes").count() > 0, "the running app keeps its elements");
    assert_eq!(map.under("app/is.a.mail").count(), 0, "the app that exited has none");
}
