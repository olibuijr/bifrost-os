use super::*;
use super::super::settings_actions::SettingsIntent as Action;
/// A fixed "now" for the row lines (seconds, UTC offset): 2026-10-22 22:13 UTC.
const W: (i64, i32) = (1_792_707_200, 0);

#[test]
fn settings_results_draw_the_symbol_their_pane_has_in_settings() {
    assert_eq!(result_symbol(&Hit::Settings("Wi-Fi")), Some(Symbol::Tile(Icon::Wifi)));
    assert_eq!(result_symbol(&Hit::Settings("About")), Some(Symbol::Glyph(Glyph::About)));
    assert_eq!(result_symbol(&Hit::Settings("Do Not Disturb")), Some(Symbol::Tile(Icon::Focus)));
    assert_eq!(pane_symbol(PageId::Wallpaper), Symbol::Tile(Icon::Wallpaper));
    assert_eq!(pane_symbol(PageId::Keyboard), Symbol::Glyph(Glyph::Keyboard));
    assert_eq!(pane_symbol(PageId::Appearance), Symbol::Tile(Icon::Appearance), "Appearance has its own tile");
    assert_eq!(pane_symbol(PageId::Root), Symbol::Tile(Icon::General), "an unnamed pane keeps the general symbol");
    // every Settings page and every Settings intent has a symbol: no blank square is left
    for page in settings::PAGES {
        assert!(result_symbol(&Hit::Settings(page)).is_some(), "{page}");
    }
    for intent in super::super::settings_intents::CATALOGUE {
        assert!(result_symbol(&Hit::SettingsIntent { id: intent.id, title: intent.title }).is_some(), "{}", intent.id);
    }
}

#[test]
fn a_pane_with_a_settings_row_has_no_second_intent_row() {
    let mut hits = BTreeMap::new();
    let rows = settings_intent_rows(&mut hits);
    assert!(!hits.contains_key("settings-intent:open-displays"), "Display & Brightness is already a Settings row");
    assert!(hits.contains_key("settings-intent:open-sound"), "a pane without a Settings row keeps its intent");
    assert_eq!(rows.len(), hits.len());
}

#[test]
fn actions_and_transport_commands_have_a_symbol_and_other_results_use_their_app() {
    let wallpaper = Hit::SettingsAction { intent: Action::SetWallpaper("dusk".into()), title: "Set Wallpaper to Dusk".into() };
    assert_eq!(result_symbol(&wallpaper), Some(Symbol::Tile(Icon::Wallpaper)));
    assert_eq!(result_symbol(&Hit::SettingsAction { intent: Action::WifiPower(true), title: "x".into() }), Some(Symbol::Tile(Icon::Wifi)));
    assert_eq!(result_symbol(&Hit::SettingsAction { intent: Action::LockScreen, title: "x".into() }), Some(Symbol::Tile(Icon::Passcode)));
    assert!(result_symbol(&Hit::App(AppId::Files)).is_none(), "an app result is drawn with the app's icon");
    assert!(result_symbol(&Hit::Terminal("x".into())).is_none());
}

#[test]
fn rows_have_a_title_and_a_second_line_saying_what_they_are() {
    let name = |_: AppId| "Files".to_string();
    assert_eq!(row_text(&Hit::App(AppId::Files), name, W), ("Files".to_string(), "Application".to_string()));
    assert_eq!(row_text(&Hit::Settings("Wi-Fi"), name, W), ("Wi-Fi".to_string(), "Settings".to_string()));
    let file = Hit::File { path: PathBuf::from("/home/u/Downloads/photo.jpg"), name: "photo.jpg".into(), modified: 0 };
    assert_eq!(row_text(&file, name, W), ("photo.jpg".to_string(), "Image \u{b7} Downloads".to_string()));
    // a file row says when the file last changed
    let dated = Hit::File { path: PathBuf::from("/home/u/Documents/e2e.txt"), name: "e2e.txt".into(), modified: W.0 - 60 };
    let (_, line) = row_text(&dated, name, W);
    assert!(line.contains(" \u{b7} Documents \u{b7} "), "kind, folder, then the date: {line}");
    assert!(line.ends_with(&crate::files::fs::date_text(W.0 - 60, W.0, 0)), "{line}");
    assert!(line.contains("Today at"), "{line}");
    let def = Hit::Definition { word: "weather".into(), dictionary: "Test".into(), preview: "n. the state of the atmosphere".into() };
    assert_eq!(row_text(&def, name, W), ("weather".to_string(), "n. the state of the atmosphere".to_string()));
    let look = Hit::Definition { word: "zyx".into(), dictionary: String::new(), preview: String::new() };
    assert_eq!(row_text(&look, name, W), ("zyx".to_string(), "Look up in Dictionary".to_string()));
    let shortcut = Hit::Shortcut { app_id: "a.b".into(), app_name: "Notes".into(), intent: "new".into(), title: "New Note".into() };
    assert_eq!(row_text(&shortcut, name, W), ("New Note".to_string(), "Shortcut \u{b7} Notes".to_string()));
    let wallpaper = Hit::SettingsAction { intent: Action::SetWallpaper("dusk".into()), title: "Set Wallpaper to Dusk".into() };
    assert_eq!(row_text(&wallpaper, name, W).1, "Use this picture as the desktop wallpaper");
    assert_eq!(row_text(&Hit::Answer("4".into()), name, W), ("= 4".to_string(), "Calculator".to_string()));
    assert_eq!(row_text(&Hit::Terminal("ls".into()), name, W), ("Search in Terminal".to_string(), "ls".to_string()));
    let indexed = Hit::Indexed { app_id: "a".into(), unique_id: "1".into(), domain: String::new(), title: "Q1 report".into(), kind: "PDF document".into(), thumb: String::new(), snippet: String::new() };
    assert_eq!(row_text(&indexed, name, W), ("Q1 report".to_string(), "PDF document".to_string()));
    let body = Hit::Indexed { app_id: "m".into(), unique_id: "2".into(), domain: String::new(), title: "Ný bókunarsíða".into(), kind: String::new(), thumb: String::new(), snippet: "\u{2026}about the glacier tour".into() };
    assert_eq!(row_text(&body, name, W), ("Ný bókunarsíða".to_string(), "\u{2026}about the glacier tour".to_string()), "a body match shows its snippet");
    let intent = Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" };
    assert_eq!(row_text(&intent, name, W), ("Open Keyboard".to_string(), "Show the keyboard settings".to_string()));
}

#[test]
fn one_chip_per_kind_of_result_and_none_for_the_best_match_or_the_actions() {
    let g = |title: &'static str, n: usize| Group { title, hits: (0..n).map(|i| Hit::City(if i == 0 { "A" } else { "B" })).collect() };
    // a Top Hit whose kind has no fixed section (the Search in Apps action here) adds no chip
    let top = Group { title: "Top Hit", hits: vec![Hit::Terminal("x".into())] };
    let groups = vec![top, g("Applications", 2), g("Settings", 1), g("Shortcuts", 3), g("Applications", 1), g("Files", 0), g("Show in Folder", 1), g("Search in Apps", 1)];
    assert_eq!(chips_of(&groups), vec!["Applications", "Settings", "Shortcuts"]);
    assert!(chips_of(&[]).is_empty());
}

/// E2E11 12: "settings" put the Settings app in the Top Hit and showed only a Mail chip; the Top Hit's kind is a chip
/// too, and that chip lists the Top Hit first.
#[test]
fn the_top_hit_gives_its_kind_a_chip_and_leads_that_chip_list() {
    let groups = vec![
        Group { title: "Top Hit", hits: vec![Hit::App(AppId::Settings)] },
        Group { title: "Mail", hits: vec![Hit::City("A")] },
        Group { title: "Search in Apps", hits: vec![Hit::Terminal("settings".into())] },
    ];
    assert_eq!(chips_of(&groups), vec!["Applications", "Mail"]);
    assert_eq!(filter_to_chip(groups.clone(), "Applications"), vec![Group { title: "Applications", hits: vec![Hit::App(AppId::Settings)] }]);
    assert_eq!(filter_to_chip(groups.clone(), "Mail"), vec![Group { title: "Mail", hits: vec![Hit::City("A")] }]);
    let more = vec![Group { title: "Top Hit", hits: vec![Hit::App(AppId::Settings)] }, Group { title: "Applications", hits: vec![Hit::App(AppId::Terminal)] }];
    assert_eq!(chips_of(&more), vec!["Applications"], "one chip per kind");
    assert_eq!(filter_to_chip(more, "Applications"), vec![Group { title: "Applications", hits: vec![Hit::App(AppId::Settings), Hit::App(AppId::Terminal)] }]);
}
