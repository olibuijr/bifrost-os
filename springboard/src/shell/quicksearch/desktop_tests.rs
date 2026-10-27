use crate::shell::hwkeys::Shortcut;
use crate::shell::keyboard::Out;
use crate::shell::{AppId, Screen};

/// Cmd+Space on the desktop opens Search over the app in front, and the keyboard types into it.
#[test]
fn cmd_space_opens_quicksearch_over_an_app_and_takes_typing() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::App(AppId::Settings);
    sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
    sh.quicksearch_tick(2000.0);
    assert!(sh.quicksearch.is_shown(), "Search opens over an app");
    sh.hw_type(Out::Text("cl".into()), 2100.0);
    assert_eq!(sh.quicksearch.query(), "cl");
}

/// E2E12 5: a shortcut run from Search answers with a banner of the app: its name over its output, "Done" when it
/// said nothing, and the problem when it failed.
#[test]
fn a_finished_shortcut_posts_its_result_as_a_notification() {
    use super::{shortcut_banner, ShortcutRun};
    assert_eq!(shortcut_banner("Current Weather", &Ok(Some(" 4° and cloudy ".into()))), ("Current Weather".into(), "4° and cloudy".into()));
    assert_eq!(shortcut_banner("New Note", &Ok(None)), ("New Note".into(), "Done".into()));
    assert_eq!(shortcut_banner("New Note", &Ok(Some("  ".into()))).1, "Done");
    let (title, body) = shortcut_banner("New Note", &Err("no reply".into()));
    assert_eq!(title, "New Note");
    assert!(body.contains("problem") && body.contains("no reply"), "{body}");
    let mut sh = crate::shell::tests::desktop_shell();
    let app = AppId::External(5);
    let before = sh.notify.store().count(app);
    assert!(!sh.post_shortcut_runs(Vec::new()), "nothing finished, nothing posted");
    let runs = vec![
        ShortcutRun { slot: 5, title: "Current Weather".into(), outcome: Ok(Some("4° and cloudy".into())) },
        ShortcutRun { slot: 5, title: "Broken".into(), outcome: Err("refused".into()) },
    ];
    assert!(sh.post_shortcut_runs(runs));
    assert_eq!(sh.notify.store().count(app), before + 2, "one notification per run, from the app");
    let group = sh.notify.store().groups().into_iter().find(|g| g.app == app).expect("the app's notifications");
    assert!(group.items.iter().any(|n| n.title == "Current Weather" && n.body == "4° and cloudy"));
    assert!(group.items.iter().any(|n| n.title == "Broken" && n.body.contains("refused")));
}

/// Cmd+Space opens Search over an app the shell does not draw (an external window in front), and a second press closes it.
#[test]
fn cmd_space_opens_and_closes_quicksearch_over_an_external_app() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::App(AppId::External(3));
    sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
    sh.quicksearch_tick(2000.0);
    assert!(sh.quicksearch.is_shown(), "Search opens over an external app");
    sh.hw_shortcut(Shortcut::QuickSearch, 3000.0);
    assert!(!sh.quicksearch.active(), "the same chord closes it");
}

/// The Cmd release that ends the chord is not Home on the desktop: Search stays open and the app stays in front.
#[test]
fn a_lone_cmd_does_not_close_search_or_go_home_on_the_desktop() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::App(AppId::Settings);
    sh.hw_shortcut(Shortcut::QuickSearch, 1000.0);
    sh.quicksearch_tick(2000.0);
    assert!(sh.quicksearch.is_shown());
    sh.hw_shortcut(Shortcut::Home, 2100.0);
    assert!(sh.quicksearch.is_shown(), "Home from a stray Cmd release does not hide Search");
    assert_eq!(sh.screen, Screen::App(AppId::Settings), "and the windows stay where they are");
}
